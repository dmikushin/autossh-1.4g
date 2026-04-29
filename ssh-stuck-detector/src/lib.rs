//! ssh-stuck-detector — LD_PRELOAD library that watches a handful
//! of network syscalls in the SSH child for "stuck" conditions
//! (proxy hang, DNS hang, kex hang) that ssh's own
//! ServerAliveInterval can't catch because they happen *before*
//! the protocol layer is even up.
//!
//! Usage:
//!     LD_PRELOAD=/path/to/libssh_stuck_detector.so \
//!         SSH_STUCK_THRESHOLD=30 ssh user@host
//!
//! On first interception the library spawns a watchdog thread.
//! Every second it scans the per-syscall trackers; when a tracked
//! call has been in flight longer than SSH_STUCK_THRESHOLD seconds
//! it writes a one-line diagnostic to stderr:
//!     ssh-stuck-detector: STUCK in connect() for 35 secs
//! autossh's existing stderr-drain detects this pattern and
//! restarts the child.
//!
//! Hooked: connect(2), getaddrinfo(3), recv(2), recvfrom(2),
//! recvmsg(2). poll/select are NOT hooked: they're allowed to
//! block by design.

use libc::{c_char, c_int, c_void, sockaddr, socklen_t, ssize_t, size_t};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicPtr, Ordering};
use std::sync::Once;

const DEFAULT_THRESHOLD_SECS: i64 = 30;

/// Per-syscall tracker. `start_sec == 0` means idle.
struct Tracker {
    name: &'static str,
    start_sec: AtomicI64,
    last_warn_sec: AtomicI64,
}

impl Tracker {
    const fn new(name: &'static str) -> Self {
        Tracker {
            name,
            start_sec: AtomicI64::new(0),
            last_warn_sec: AtomicI64::new(0),
        }
    }

    /// Start tracking. Returns the previous start_sec (0 if idle)
    /// so reentrant calls don't clobber each other.
    fn enter(&self) -> i64 {
        let prev = self.start_sec.load(Ordering::Relaxed);
        if prev == 0 {
            self.start_sec.store(now_secs(), Ordering::Relaxed);
        }
        prev
    }

    fn leave(&self, prev: i64) {
        // Only clear if we were the entry that started tracking.
        if prev == 0 {
            self.start_sec.store(0, Ordering::Relaxed);
            self.last_warn_sec.store(0, Ordering::Relaxed);
        }
    }
}

static CONNECT_T:     Tracker = Tracker::new("connect");
static GETADDRINFO_T: Tracker = Tracker::new("getaddrinfo");
static RECV_T:        Tracker = Tracker::new("recv");
static RECVFROM_T:    Tracker = Tracker::new("recvfrom");
static RECVMSG_T:     Tracker = Tracker::new("recvmsg");

const TRACKERS: &[&Tracker] = &[
    &CONNECT_T, &GETADDRINFO_T, &RECV_T, &RECVFROM_T, &RECVMSG_T,
];

static MONITOR_STARTED: Once = Once::new();
static THRESHOLD_SECS: AtomicI64 = AtomicI64::new(DEFAULT_THRESHOLD_SECS);
static IN_INTERCEPTOR: AtomicBool = AtomicBool::new(false);

fn now_secs() -> i64 {
    let mut ts: libc::timespec = unsafe { std::mem::zeroed() };
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts); }
    ts.tv_sec as i64
}

/// Read SSH_STUCK_THRESHOLD env, fall back to default. Called once.
unsafe fn read_threshold() {
    let key = c"SSH_STUCK_THRESHOLD".as_ptr();
    let raw = libc::getenv(key);
    if raw.is_null() {
        return;
    }
    let mut end: *mut c_char = std::ptr::null_mut();
    let v = libc::strtol(raw, &mut end, 10);
    if v > 0 && !end.is_null() && *end == 0 {
        THRESHOLD_SECS.store(v as i64, Ordering::Relaxed);
    }
}

/// Spawned monitor thread: scans trackers, writes warnings,
/// eventually self-terminates the process if we're truly stuck.
extern "C" fn monitor_main(_: *mut c_void) -> *mut c_void {
    loop {
        unsafe { libc::sleep(1); }
        let now = now_secs();
        let threshold = THRESHOLD_SECS.load(Ordering::Relaxed);
        for t in TRACKERS {
            let start = t.start_sec.load(Ordering::Relaxed);
            if start == 0 { continue; }
            let elapsed = now - start;
            if elapsed < threshold { continue; }
            // Rate-limit: warn at most once per `threshold` secs
            // for the same in-flight call.
            let last = t.last_warn_sec.load(Ordering::Relaxed);
            if now - last >= threshold {
                t.last_warn_sec.store(now, Ordering::Relaxed);
                warn_stderr(t.name, elapsed);
            }
            // After 2× threshold of being stuck, abandon: write a
            // final line and _exit. The host process dies; if it's
            // an SSH ProxyCommand the parent ssh sees the proxy
            // gone and exits; autossh then catches SIGCHLD and
            // restarts. Stderr might be /dev/null'd by ssh — the
            // _exit is what actually unblocks the chain.
            if elapsed >= threshold * 2 {
                let msg = b"ssh-stuck-detector: aborting stuck process\n";
                unsafe {
                    libc::write(libc::STDERR_FILENO,
                        msg.as_ptr() as *const c_void, msg.len());
                    libc::_exit(1);
                }
            }
        }
    }
}

/// Write a fixed-format line to stderr without allocating.
/// Format: "ssh-stuck-detector: STUCK in <name>() for <secs> secs\n"
fn warn_stderr(name: &str, secs: i64) {
    let mut buf = [0u8; 128];
    let mut n: usize = 0;
    let prefix = b"ssh-stuck-detector: STUCK in ";
    for &b in prefix { if n < buf.len() { buf[n] = b; n += 1; } }
    for &b in name.as_bytes() { if n < buf.len() { buf[n] = b; n += 1; } }
    let mid = b"() for ";
    for &b in mid { if n < buf.len() { buf[n] = b; n += 1; } }
    // itoa on `secs`
    let mut s = secs;
    if s < 0 { s = 0; }
    let mut digits = [0u8; 20];
    let mut dn = 0;
    if s == 0 {
        digits[0] = b'0';
        dn = 1;
    } else {
        while s > 0 {
            digits[dn] = b'0' + (s % 10) as u8;
            s /= 10;
            dn += 1;
        }
    }
    for i in (0..dn).rev() {
        if n < buf.len() { buf[n] = digits[i]; n += 1; }
    }
    let suffix = b" secs\n";
    for &b in suffix { if n < buf.len() { buf[n] = b; n += 1; } }
    unsafe {
        libc::write(libc::STDERR_FILENO, buf.as_ptr() as *const c_void, n);
    }
}

unsafe fn ensure_monitor() {
    MONITOR_STARTED.call_once(|| {
        read_threshold();
        let mut tid: libc::pthread_t = std::mem::zeroed();
        // Detached thread, default attrs.
        let _ = libc::pthread_create(
            &mut tid,
            std::ptr::null(),
            monitor_main,
            std::ptr::null_mut(),
        );
        let _ = libc::pthread_detach(tid);
    });
}

/// dlsym RTLD_NEXT for the named libc function; cache statically.
unsafe fn next_fn(name: &[u8]) -> *mut c_void {
    libc::dlsym(libc::RTLD_NEXT, name.as_ptr() as *const c_char)
}

// ---------- connect ----------
type ConnectFn = unsafe extern "C" fn(c_int, *const sockaddr, socklen_t) -> c_int;
static REAL_CONNECT: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

#[no_mangle]
pub unsafe extern "C" fn connect(fd: c_int, addr: *const sockaddr, len: socklen_t) -> c_int {
    let mut p = REAL_CONNECT.load(Ordering::Relaxed);
    if p.is_null() {
        p = next_fn(b"connect\0");
        REAL_CONNECT.store(p, Ordering::Relaxed);
    }
    let real: ConnectFn = std::mem::transmute(p);
    ensure_monitor();
    let prev = CONNECT_T.enter();
    let r = real(fd, addr, len);
    CONNECT_T.leave(prev);
    r
}

// ---------- getaddrinfo ----------
type GaiFn = unsafe extern "C" fn(
    *const c_char, *const c_char,
    *const libc::addrinfo, *mut *mut libc::addrinfo,
) -> c_int;
static REAL_GAI: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

#[no_mangle]
pub unsafe extern "C" fn getaddrinfo(
    node: *const c_char, service: *const c_char,
    hints: *const libc::addrinfo, res: *mut *mut libc::addrinfo,
) -> c_int {
    let mut p = REAL_GAI.load(Ordering::Relaxed);
    if p.is_null() {
        p = next_fn(b"getaddrinfo\0");
        REAL_GAI.store(p, Ordering::Relaxed);
    }
    let real: GaiFn = std::mem::transmute(p);
    ensure_monitor();
    let prev = GETADDRINFO_T.enter();
    let r = real(node, service, hints, res);
    GETADDRINFO_T.leave(prev);
    r
}

// ---------- recv / recvfrom / recvmsg ----------
type RecvFn = unsafe extern "C" fn(c_int, *mut c_void, size_t, c_int) -> ssize_t;
static REAL_RECV: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

#[no_mangle]
pub unsafe extern "C" fn recv(fd: c_int, buf: *mut c_void, n: size_t, flags: c_int) -> ssize_t {
    let mut p = REAL_RECV.load(Ordering::Relaxed);
    if p.is_null() {
        p = next_fn(b"recv\0");
        REAL_RECV.store(p, Ordering::Relaxed);
    }
    let real: RecvFn = std::mem::transmute(p);
    ensure_monitor();
    let prev = RECV_T.enter();
    let r = real(fd, buf, n, flags);
    RECV_T.leave(prev);
    r
}

type RecvfromFn = unsafe extern "C" fn(
    c_int, *mut c_void, size_t, c_int, *mut sockaddr, *mut socklen_t,
) -> ssize_t;
static REAL_RECVFROM: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

#[no_mangle]
pub unsafe extern "C" fn recvfrom(
    fd: c_int, buf: *mut c_void, n: size_t, flags: c_int,
    addr: *mut sockaddr, addrlen: *mut socklen_t,
) -> ssize_t {
    let mut p = REAL_RECVFROM.load(Ordering::Relaxed);
    if p.is_null() {
        p = next_fn(b"recvfrom\0");
        REAL_RECVFROM.store(p, Ordering::Relaxed);
    }
    let real: RecvfromFn = std::mem::transmute(p);
    ensure_monitor();
    let prev = RECVFROM_T.enter();
    let r = real(fd, buf, n, flags, addr, addrlen);
    RECVFROM_T.leave(prev);
    r
}

type RecvmsgFn = unsafe extern "C" fn(c_int, *mut libc::msghdr, c_int) -> ssize_t;
static REAL_RECVMSG: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

#[no_mangle]
pub unsafe extern "C" fn recvmsg(fd: c_int, msg: *mut libc::msghdr, flags: c_int) -> ssize_t {
    let mut p = REAL_RECVMSG.load(Ordering::Relaxed);
    if p.is_null() {
        p = next_fn(b"recvmsg\0");
        REAL_RECVMSG.store(p, Ordering::Relaxed);
    }
    let real: RecvmsgFn = std::mem::transmute(p);
    ensure_monitor();
    let prev = RECVMSG_T.enter();
    let r = real(fd, msg, flags);
    RECVMSG_T.leave(prev);
    r
}

// Suppress dead-code warning on the IN_INTERCEPTOR flag — kept as
// a hook for future re-entrancy guards if profiling shows spin.
#[allow(dead_code)]
fn _unused() { let _ = IN_INTERCEPTOR.load(Ordering::Relaxed); }
