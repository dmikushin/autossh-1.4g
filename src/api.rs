//! Embeddable libautossh C ABI.
//!
//! The library API intentionally keeps autossh's signal handling and
//! SIGCHLD ownership out of the host process. A session forks a dedicated
//! supervisor child; that child runs the existing autossh main loop and owns
//! the OpenSSH process it supervises.

use libc::{c_char, c_double, c_int, pid_t};
use std::ffi::{CStr, CString};
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::ptr;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::{Duration, Instant};

const AUTOSSH_SESSION_MONITOR_DISABLED: u32 = 1u32 << 0;

const AUTOSSH_STATE_INIT: c_int = 0;
const AUTOSSH_STATE_STARTING: c_int = 1;
const AUTOSSH_STATE_RUNNING: c_int = 2;
const AUTOSSH_STATE_EXITED: c_int = 3;
const AUTOSSH_STATE_STOPPED: c_int = 4;
const AUTOSSH_STATE_ERROR: c_int = 5;

const DEFAULT_BIND_HOST: &str = "127.0.0.1";
const DEFAULT_TARGET_PORT: u16 = 22;
const DEFAULT_START_TIMEOUT_MS: c_int = 15000;
const ENV_AUTOSSH_PATH: &[u8] = b"AUTOSSH_PATH\0";
const ENV_AUTOSSH_MHOST: &[u8] = b"AUTOSSH_MHOST\0";
const ENV_AUTOSSH_POLL: &[u8] = b"AUTOSSH_POLL\0";
const ENV_AUTOSSH_FIRST_POLL: &[u8] = b"AUTOSSH_FIRST_POLL\0";
const ENV_AUTOSSH_GATETIME: &[u8] = b"AUTOSSH_GATETIME\0";
const ENV_AUTOSSH_MAXSTART: &[u8] = b"AUTOSSH_MAXSTART\0";
const ENV_AUTOSSH_MAX_SESSION: &[u8] = b"AUTOSSH_MAX_SESSION\0";

#[repr(C)]
pub struct autossh_session_config {
    pub struct_size: usize,

    pub ssh_path: *const c_char,
    pub ssh_destination: *const c_char,
    pub target_host: *const c_char,
    pub target_port: u16,

    pub bind_host: *const c_char,
    pub local_port: u16,
    pub monitor_host: *const c_char,
    pub monitor_port: u16,
    pub flags: u32,

    pub poll_time: c_int,
    pub first_poll_time: c_int,
    pub gate_time: c_double,
    pub max_start: c_int,
    pub max_session: c_double,
    pub start_timeout_ms: c_int,

    pub extra_ssh_args: *const *const c_char,
    pub extra_ssh_args_len: usize,
}

struct SessionConfig {
    ssh_path: Option<CString>,
    ssh_destination: CString,
    target_host: CString,
    target_port: u16,
    bind_host: CString,
    local_port: u16,
    monitor_host: Option<CString>,
    monitor_port: u16,
    monitor_disabled: bool,
    poll_time: c_int,
    first_poll_time: c_int,
    gate_time: c_double,
    max_start: c_int,
    max_session: c_double,
    start_timeout_ms: c_int,
    extra_ssh_args: Vec<CString>,
}

#[repr(C)]
pub struct autossh_session {
    config: SessionConfig,
    /// The supervisor's pid, or 0. Atomic, and handed over by `swap`, because
    /// `autossh_session_stop` and `autossh_session_state` are called from
    /// different threads of the embedding process (ssh2ng under Bun: a poll
    /// on one thread, a close on another) and both reap the child. Stop's
    /// loop re-read the field while state's poll had already reaped and
    /// zeroed it, and after its five-second grace it ran
    /// `kill(session.pid, SIGKILL)` with pid 0 — which is the caller's whole
    /// process group. That took a test runner and its shell down with it.
    /// Every kill in this file now goes through `kill_pid`, which refuses a
    /// pid that is not a single positive process.
    pid: AtomicI32,
    state: c_int,
    last_error: CString,
}

/// `kill(2)` restricted to one live, positive pid. `pid == 0` is the caller's
/// process group, `-1` is every process the caller may signal, and a negative
/// pid is a group: none of those is ever what a supervisor teardown means.
fn kill_pid(pid: pid_t, signal: c_int) -> bool {
    if pid <= 0 {
        return false;
    }
    unsafe { libc::kill(pid, signal) == 0 }
}

#[no_mangle]
pub unsafe extern "C" fn autossh_session_config_init(config: *mut autossh_session_config) {
    if config.is_null() {
        return;
    }
    ptr::write_bytes(config, 0, 1);
    (*config).struct_size = std::mem::size_of::<autossh_session_config>();
    (*config).target_port = DEFAULT_TARGET_PORT;
    (*config).start_timeout_ms = DEFAULT_START_TIMEOUT_MS;
    (*config).max_start = -1;
}

unsafe fn optional_cstring(ptr: *const c_char, field: &str) -> Result<Option<CString>, CString> {
    if ptr.is_null() {
        return Ok(None);
    }
    let bytes = CStr::from_ptr(ptr).to_bytes();
    CString::new(bytes)
        .map(Some)
        .map_err(|_| CString::new(format!("{field} contains an interior NUL")).unwrap())
}

unsafe fn required_cstring(ptr: *const c_char, field: &str) -> Result<CString, CString> {
    match optional_cstring(ptr, field)? {
        Some(s) if !s.as_bytes().is_empty() => Ok(s),
        _ => Err(CString::new(format!("{field} is required")).unwrap()),
    }
}

fn string_from_cstring(s: &CString) -> Result<String, CString> {
    s.to_str()
        .map(|s| s.to_owned())
        .map_err(|_| CString::new("configuration contains non-UTF-8 text").unwrap())
}

fn set_error(session: &mut autossh_session, message: impl Into<String>) -> c_int {
    session.state = AUTOSSH_STATE_ERROR;
    session.last_error = CString::new(message.into())
        .unwrap_or_else(|_| CString::new("error message contains an interior NUL").unwrap());
    -1
}

unsafe fn parse_config(config: *const autossh_session_config) -> Result<SessionConfig, CString> {
    if config.is_null() {
        return Err(CString::new("config is NULL").unwrap());
    }
    let c = &*config;
    if c.struct_size < std::mem::size_of::<autossh_session_config>() {
        return Err(CString::new("config struct_size is too small").unwrap());
    }

    let ssh_destination = required_cstring(c.ssh_destination, "ssh_destination")?;
    let target_host = required_cstring(c.target_host, "target_host")?;
    let bind_host = optional_cstring(c.bind_host, "bind_host")?
        .unwrap_or_else(|| CString::new(DEFAULT_BIND_HOST).unwrap());
    let monitor_host = optional_cstring(c.monitor_host, "monitor_host")?;
    let ssh_path = optional_cstring(c.ssh_path, "ssh_path")?;

    let mut extra_ssh_args = Vec::new();
    if c.extra_ssh_args_len > 0 {
        if c.extra_ssh_args.is_null() {
            return Err(CString::new("extra_ssh_args is NULL").unwrap());
        }
        for i in 0..c.extra_ssh_args_len {
            let arg = *c.extra_ssh_args.add(i);
            extra_ssh_args.push(required_cstring(arg, "extra_ssh_args item")?);
        }
    }

    Ok(SessionConfig {
        ssh_path,
        ssh_destination,
        target_host,
        target_port: if c.target_port == 0 {
            DEFAULT_TARGET_PORT
        } else {
            c.target_port
        },
        bind_host,
        local_port: c.local_port,
        monitor_host,
        monitor_port: c.monitor_port,
        monitor_disabled: (c.flags & AUTOSSH_SESSION_MONITOR_DISABLED) != 0,
        poll_time: c.poll_time,
        first_poll_time: c.first_poll_time,
        gate_time: c.gate_time,
        max_start: c.max_start,
        max_session: c.max_session,
        start_timeout_ms: if c.start_timeout_ms <= 0 {
            DEFAULT_START_TIMEOUT_MS
        } else {
            c.start_timeout_ms
        },
        extra_ssh_args,
    })
}

fn bind_addr(host: &str, port: u16) -> Result<SocketAddr, String> {
    (host, port)
        .to_socket_addrs()
        .map_err(|e| format!("resolve {host}:{port}: {e}"))?
        .next()
        .ok_or_else(|| format!("resolve {host}:{port}: no addresses"))
}

fn reserve_port(host: &str) -> Result<u16, String> {
    let listener =
        TcpListener::bind(bind_addr(host, 0)?).map_err(|e| format!("bind {host}:0: {e}"))?;
    listener
        .local_addr()
        .map(|a| a.port())
        .map_err(|e| format!("read local address: {e}"))
}

fn reserve_monitor_port(host: &str) -> Result<u16, String> {
    for _ in 0..64 {
        let first =
            TcpListener::bind(bind_addr(host, 0)?).map_err(|e| format!("bind {host}:0: {e}"))?;
        let port = first
            .local_addr()
            .map_err(|e| format!("read local address: {e}"))?
            .port();
        if port == u16::MAX {
            continue;
        }
        if let Ok(second) = TcpListener::bind(bind_addr(host, port + 1)?) {
            drop(second);
            drop(first);
            return Ok(port);
        }
    }
    Err("could not reserve adjacent monitor ports".to_string())
}

fn child_exited(pid: pid_t) -> Option<c_int> {
    let mut status: c_int = 0;
    let rc = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
    if rc == pid {
        Some(status)
    } else {
        None
    }
}

fn wait_for_listener(session: &autossh_session) -> Result<(), String> {
    let host = string_from_cstring(&session.config.bind_host)
        .map_err(|e| e.to_string_lossy().into_owned())?;
    let addr = bind_addr(&host, session.config.local_port)?;
    let deadline = Instant::now() + Duration::from_millis(session.config.start_timeout_ms as u64);

    while Instant::now() < deadline {
        if let Some(status) = child_exited(session.pid.load(Ordering::SeqCst)) {
            return Err(format!(
                "autossh supervisor exited early with status {status}"
            ));
        }
        if TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    Err(format!(
        "timed out waiting for local tunnel listener on {}:{}",
        host, session.config.local_port
    ))
}

fn push_arg(args: &mut Vec<CString>, value: impl AsRef<str>) {
    args.push(CString::new(value.as_ref()).unwrap());
}

fn build_child_argv(config: &SessionConfig) -> Result<Vec<CString>, String> {
    let bind_host =
        string_from_cstring(&config.bind_host).map_err(|e| e.to_string_lossy().into_owned())?;
    let target_host =
        string_from_cstring(&config.target_host).map_err(|e| e.to_string_lossy().into_owned())?;
    let ssh_destination = string_from_cstring(&config.ssh_destination)
        .map_err(|e| e.to_string_lossy().into_owned())?;

    let mut argv = Vec::new();
    push_arg(&mut argv, "autossh");
    push_arg(&mut argv, "-M");
    if config.monitor_disabled {
        push_arg(&mut argv, "0");
    } else {
        push_arg(&mut argv, config.monitor_port.to_string());
    }
    push_arg(&mut argv, "-N");
    push_arg(&mut argv, "-o");
    push_arg(&mut argv, "ExitOnForwardFailure=yes");
    push_arg(&mut argv, "-L");
    push_arg(
        &mut argv,
        format!(
            "{}:{}:{}:{}",
            bind_host, config.local_port, target_host, config.target_port
        ),
    );
    for arg in &config.extra_ssh_args {
        argv.push(arg.clone());
    }
    push_arg(&mut argv, ssh_destination);
    Ok(argv)
}

unsafe fn child_main(session: &autossh_session) -> ! {
    if let Some(path) = &session.config.ssh_path {
        libc::setenv(ENV_AUTOSSH_PATH.as_ptr() as *const c_char, path.as_ptr(), 1);
    }
    if let Some(host) = &session.config.monitor_host {
        libc::setenv(
            ENV_AUTOSSH_MHOST.as_ptr() as *const c_char,
            host.as_ptr(),
            1,
        );
    }
    if session.config.poll_time > 0 {
        let s = CString::new(session.config.poll_time.to_string()).unwrap();
        libc::setenv(ENV_AUTOSSH_POLL.as_ptr() as *const c_char, s.as_ptr(), 1);
    }
    if session.config.first_poll_time > 0 {
        let s = CString::new(session.config.first_poll_time.to_string()).unwrap();
        libc::setenv(
            ENV_AUTOSSH_FIRST_POLL.as_ptr() as *const c_char,
            s.as_ptr(),
            1,
        );
    }
    if session.config.gate_time >= 0.0 {
        let s = CString::new(format!("{}", session.config.gate_time)).unwrap();
        libc::setenv(
            ENV_AUTOSSH_GATETIME.as_ptr() as *const c_char,
            s.as_ptr(),
            1,
        );
    }
    if session.config.max_start != 0 {
        let s = CString::new(session.config.max_start.to_string()).unwrap();
        libc::setenv(
            ENV_AUTOSSH_MAXSTART.as_ptr() as *const c_char,
            s.as_ptr(),
            1,
        );
    }
    if session.config.max_session > 0.0 {
        let s = CString::new(format!("{}", session.config.max_session)).unwrap();
        libc::setenv(
            ENV_AUTOSSH_MAX_SESSION.as_ptr() as *const c_char,
            s.as_ptr(),
            1,
        );
    }

    let argv = match build_child_argv(&session.config) {
        Ok(argv) => argv,
        Err(_) => libc::_exit(2),
    };
    let mut argv_ptrs: Vec<*mut c_char> = argv.iter().map(|s| s.as_ptr() as *mut c_char).collect();
    argv_ptrs.push(ptr::null_mut());
    let argc = (argv_ptrs.len() - 1) as c_int;

    let rc = crate::main_logic::autossh_main(argc, argv_ptrs.as_mut_ptr());
    libc::_exit(rc);
}

#[no_mangle]
pub unsafe extern "C" fn autossh_session_new(
    config: *const autossh_session_config,
) -> *mut autossh_session {
    let config = match parse_config(config) {
        Ok(config) => config,
        Err(_) => return ptr::null_mut(),
    };
    Box::into_raw(Box::new(autossh_session {
        config,
        pid: AtomicI32::new(0),
        state: AUTOSSH_STATE_INIT,
        last_error: CString::new("").unwrap(),
    }))
}

#[no_mangle]
pub unsafe extern "C" fn autossh_session_start(session: *mut autossh_session) -> c_int {
    if session.is_null() {
        return -1;
    }
    let session = &mut *session;
    if session.pid.load(Ordering::SeqCst) > 0 {
        return set_error(session, "session is already started");
    }

    let bind_host = match string_from_cstring(&session.config.bind_host) {
        Ok(host) => host,
        Err(e) => return set_error(session, e.to_string_lossy()),
    };
    if session.config.local_port == 0 {
        match reserve_port(&bind_host) {
            Ok(port) => session.config.local_port = port,
            Err(e) => return set_error(session, e),
        }
    }
    if !session.config.monitor_disabled && session.config.monitor_port == 0 {
        match reserve_monitor_port(&bind_host) {
            Ok(port) => session.config.monitor_port = port,
            Err(e) => return set_error(session, e),
        }
    }

    session.state = AUTOSSH_STATE_STARTING;
    let pid = libc::fork();
    if pid < 0 {
        return set_error(session, "fork failed");
    }
    if pid == 0 {
        child_main(session);
    }

    session.pid.store(pid, Ordering::SeqCst);
    match wait_for_listener(session) {
        Ok(()) => {
            session.state = AUTOSSH_STATE_RUNNING;
            session.last_error = CString::new("").unwrap();
            0
        }
        Err(e) => {
            let _ = autossh_session_stop(session);
            set_error(session, e)
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn autossh_session_stop(session: *mut autossh_session) -> c_int {
    if session.is_null() {
        return -1;
    }
    let session = &mut *session;
    // Take the pid: whoever swaps it out owns the teardown, and a second
    // caller — or a state poll racing this one — finds 0 and does nothing.
    let pid = session.pid.swap(0, Ordering::SeqCst);
    if pid <= 0 {
        session.state = AUTOSSH_STATE_STOPPED;
        return 0;
    }

    kill_pid(pid, libc::SIGTERM);
    for _ in 0..50 {
        let mut status: c_int = 0;
        let rc = libc::waitpid(pid, &mut status, libc::WNOHANG);
        if rc == pid || (rc == -1 && *libc::__errno_location() == libc::ECHILD) {
            // Reaped here, or already reaped by someone else (ECHILD): either
            // way the process is gone and there is nothing left to kill.
            session.state = AUTOSSH_STATE_STOPPED;
            return 0;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    kill_pid(pid, libc::SIGKILL);
    let mut status: c_int = 0;
    libc::waitpid(pid, &mut status, 0);
    session.state = AUTOSSH_STATE_STOPPED;
    0
}

#[no_mangle]
pub unsafe extern "C" fn autossh_session_free(session: *mut autossh_session) {
    if session.is_null() {
        return;
    }
    let _ = autossh_session_stop(session);
    drop(Box::from_raw(session));
}

#[no_mangle]
pub unsafe extern "C" fn autossh_session_state(session: *mut autossh_session) -> c_int {
    if session.is_null() {
        return AUTOSSH_STATE_ERROR;
    }
    let session = &mut *session;
    let pid = session.pid.load(Ordering::SeqCst);
    if pid > 0 && child_exited(pid).is_some() {
        // Only clear the pid we reaped; a concurrent stop may have taken it.
        let _ = session
            .pid
            .compare_exchange(pid, 0, Ordering::SeqCst, Ordering::SeqCst);
        session.state = AUTOSSH_STATE_EXITED;
    }
    session.state
}

#[no_mangle]
pub unsafe extern "C" fn autossh_session_pid(session: *mut autossh_session) -> pid_t {
    if session.is_null() {
        return 0;
    }
    (*session).pid.load(Ordering::SeqCst)
}

#[no_mangle]
pub unsafe extern "C" fn autossh_session_local_port(session: *mut autossh_session) -> u16 {
    if session.is_null() {
        return 0;
    }
    (*session).config.local_port
}

#[no_mangle]
pub unsafe extern "C" fn autossh_session_bind_host(session: *mut autossh_session) -> *const c_char {
    if session.is_null() {
        return ptr::null();
    }
    (*session).config.bind_host.as_ptr()
}

#[no_mangle]
pub unsafe extern "C" fn autossh_session_last_error(
    session: *mut autossh_session,
) -> *const c_char {
    if session.is_null() {
        return ptr::null();
    }
    (*session).last_error.as_ptr()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cstr(s: &str) -> CString {
        CString::new(s).unwrap()
    }

    /// The signal that took a test runner down: kill with pid 0 is the
    /// caller's own process group. `kill_pid` must refuse it, and every
    /// other non-positive pid, without touching the kernel.
    #[test]
    fn kill_pid_refuses_groups_and_zero() {
        assert!(!kill_pid(0, libc::SIGKILL));
        assert!(!kill_pid(-1, libc::SIGKILL));
        assert!(!kill_pid(-(std::process::id() as pid_t), libc::SIGKILL));
        // Signal 0 is a liveness probe: a positive live pid is accepted.
        assert!(kill_pid(std::process::id() as pid_t, 0));
    }

    /// Stop and state, racing on one session whose child is already gone:
    /// exactly one of them may own the pid, and neither may end up sending
    /// anything to pid 0. The child is a real forked process that exits at
    /// once, so waitpid sees it reaped from one side or the other.
    #[test]
    fn concurrent_stop_and_state_never_kill_pid_zero() {
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0);
        if pid == 0 {
            unsafe { libc::_exit(0) };
        }
        let session = Box::into_raw(Box::new(autossh_session {
            config: SessionConfig {
                ssh_path: Some(cstr("/bin/true")),
                ssh_destination: cstr("nowhere"),
                target_host: cstr("127.0.0.1"),
                target_port: 1,
                bind_host: cstr("127.0.0.1"),
                local_port: 1,
                monitor_host: None,
                monitor_port: 0,
                monitor_disabled: true,
                poll_time: 0,
                first_poll_time: 0,
                gate_time: 0.0,
                max_start: 0,
                max_session: 0.0,
                start_timeout_ms: 0,
                extra_ssh_args: Vec::new(),
            },
            pid: AtomicI32::new(pid),
            state: AUTOSSH_STATE_RUNNING,
            last_error: CString::new("").unwrap(),
        }));
        let addr = session as usize;
        let poller = std::thread::spawn(move || {
            for _ in 0..200 {
                unsafe { autossh_session_state(addr as *mut autossh_session) };
            }
        });
        let stopper = std::thread::spawn(move || unsafe {
            autossh_session_stop(addr as *mut autossh_session)
        });
        assert_eq!(stopper.join().unwrap(), 0);
        poller.join().unwrap();
        // We are still here — the process group was not signalled — and the
        // pid is cleared exactly once.
        assert_eq!(unsafe { autossh_session_pid(session) }, 0);
        unsafe { autossh_session_free(session) };
    }

    #[test]
    fn config_init_sets_abi_defaults() {
        let mut cfg = autossh_session_config {
            struct_size: 123,
            ssh_path: std::ptr::dangling(),
            ssh_destination: std::ptr::dangling(),
            target_host: std::ptr::dangling(),
            target_port: 1234,
            bind_host: std::ptr::dangling(),
            local_port: 1234,
            monitor_host: std::ptr::dangling(),
            monitor_port: 1234,
            flags: 1234,
            poll_time: 1234,
            first_poll_time: 1234,
            gate_time: 1234.0,
            max_start: 1234,
            max_session: 1234.0,
            start_timeout_ms: 1234,
            extra_ssh_args: std::ptr::dangling(),
            extra_ssh_args_len: 1234,
        };

        unsafe { autossh_session_config_init(&mut cfg) };

        assert_eq!(
            cfg.struct_size,
            std::mem::size_of::<autossh_session_config>()
        );
        assert!(cfg.ssh_path.is_null());
        assert!(cfg.ssh_destination.is_null());
        assert!(cfg.target_host.is_null());
        assert_eq!(cfg.target_port, DEFAULT_TARGET_PORT);
        assert!(cfg.bind_host.is_null());
        assert_eq!(cfg.local_port, 0);
        assert!(cfg.monitor_host.is_null());
        assert_eq!(cfg.monitor_port, 0);
        assert_eq!(cfg.flags, 0);
        assert_eq!(cfg.max_start, -1);
        assert_eq!(cfg.start_timeout_ms, DEFAULT_START_TIMEOUT_MS);
    }

    #[test]
    fn parse_config_requires_destination_and_target() {
        let target = cstr("127.0.0.1");
        let mut cfg = autossh_session_config {
            struct_size: std::mem::size_of::<autossh_session_config>(),
            ssh_path: ptr::null(),
            ssh_destination: ptr::null(),
            target_host: target.as_ptr(),
            target_port: 22,
            bind_host: ptr::null(),
            local_port: 0,
            monitor_host: ptr::null(),
            monitor_port: 0,
            flags: 0,
            poll_time: 0,
            first_poll_time: 0,
            gate_time: 0.0,
            max_start: -1,
            max_session: 0.0,
            start_timeout_ms: 0,
            extra_ssh_args: ptr::null(),
            extra_ssh_args_len: 0,
        };
        assert!(unsafe { parse_config(&cfg) }.is_err());

        let destination = cstr("jump");
        cfg.ssh_destination = destination.as_ptr();
        cfg.target_host = ptr::null();
        assert!(unsafe { parse_config(&cfg) }.is_err());
    }

    #[test]
    fn parse_config_applies_defaults_and_extra_args() {
        let destination = cstr("jump");
        let target = cstr("final");
        let monitor_host = cstr("172.17.0.1");
        let extra = [cstr("-F"), cstr("/tmp/ssh_config")];
        let extra_ptrs = [extra[0].as_ptr(), extra[1].as_ptr()];
        let cfg = autossh_session_config {
            struct_size: std::mem::size_of::<autossh_session_config>(),
            ssh_path: ptr::null(),
            ssh_destination: destination.as_ptr(),
            target_host: target.as_ptr(),
            target_port: 0,
            bind_host: ptr::null(),
            local_port: 0,
            monitor_host: monitor_host.as_ptr(),
            monitor_port: 0,
            flags: AUTOSSH_SESSION_MONITOR_DISABLED,
            poll_time: 0,
            first_poll_time: 0,
            gate_time: 0.0,
            max_start: -1,
            max_session: 0.0,
            start_timeout_ms: 0,
            extra_ssh_args: extra_ptrs.as_ptr(),
            extra_ssh_args_len: extra_ptrs.len(),
        };

        let parsed = unsafe { parse_config(&cfg) }.unwrap();

        assert_eq!(parsed.target_port, DEFAULT_TARGET_PORT);
        assert_eq!(parsed.bind_host.to_str().unwrap(), DEFAULT_BIND_HOST);
        assert_eq!(parsed.start_timeout_ms, DEFAULT_START_TIMEOUT_MS);
        assert_eq!(parsed.monitor_host.unwrap().to_str().unwrap(), "172.17.0.1");
        assert!(parsed.monitor_disabled);
        assert_eq!(parsed.extra_ssh_args.len(), 2);
    }

    #[test]
    fn build_child_argv_uses_local_forward_contract() {
        let config = SessionConfig {
            ssh_path: None,
            ssh_destination: cstr("jump"),
            target_host: cstr("final.internal"),
            target_port: 22,
            bind_host: cstr("127.0.0.1"),
            local_port: 43000,
            monitor_host: Some(cstr("172.17.0.1")),
            monitor_port: 0,
            monitor_disabled: true,
            poll_time: 0,
            first_poll_time: 0,
            gate_time: 0.0,
            max_start: -1,
            max_session: 0.0,
            start_timeout_ms: 1,
            extra_ssh_args: vec![cstr("-F"), cstr("/tmp/ssh_config")],
        };

        let argv = build_child_argv(&config).unwrap();
        let argv: Vec<&str> = argv.iter().map(|s| s.to_str().unwrap()).collect();

        assert_eq!(
            argv,
            vec![
                "autossh",
                "-M",
                "0",
                "-N",
                "-o",
                "ExitOnForwardFailure=yes",
                "-L",
                "127.0.0.1:43000:final.internal:22",
                "-F",
                "/tmp/ssh_config",
                "jump"
            ]
        );
    }
}
