//! Signal handling.
//!
//! `sig_catch()` is the single signal handler installed for SIGINT,
//! SIGTERM, SIGHUP, SIGUSR1, SIGUSR2, SIGCHLD and SIGALRM. Behaviour:
//!
//!   - SIGUSR1            → set restart_ssh = 1
//!   - SIGINT/SIGTERM 1×  → set exit_signalled = 1
//!   - SIGINT/SIGTERM 2×  → call _exit(1) immediately (Ctrl+C bail
//!                          fix). The first invocation set the flag;
//!                          on the second we don't even bother with
//!                          ordered shutdown — the user is impatient.
//!   - any signal: if `dolongjmp` was set (by ssh_watch right before
//!     poll/pause), siglongjmp(jumpbuf, sig) — control unwinds all
//!     the way back to the for-loop's sigsetjmp.
//!
//! All flags and `jumpbuf` live in autossh.c (`volatile sig_atomic_t`
//! and `sigjmp_buf` respectively); we access them via `extern static
//! mut`.
//!
//! Test coverage: tests/unit/test_sig_catch.c (10 cases, including
//! both legs of the double-SIGINT fix and the dolongjmp behaviour).

use libc::c_int;

/// Storage for `sigjmp_buf jumpbuf` referenced by sig_catch.
///
/// libc 0.2 doesn't expose `sigjmp_buf`. Glibc x86_64 sigjmp_buf is
/// ~200 bytes (jmp_buf 144 + mask_was_saved + sigset_t 128). Other
/// glibc targets reach ~570 bytes (aarch64). 1024 bytes aligned to
/// 16 is generous on every platform we care about and matches the
/// alignment requirement of the underlying struct __jmp_buf_tag.
///
/// The instance is zero-initialised; sigsetjmp populates it on first
/// call. Since signals are blocked around the sigsetjmp/longjmp
/// sequences in ssh_watch, no external code observes the storage
/// before it's been written, so zero-init is harmless.
#[repr(C, align(16))]
pub struct JmpBuf {
    _data: [u8; 1024],
}

#[no_mangle]
pub static mut jumpbuf: JmpBuf = JmpBuf { _data: [0; 1024] };

extern "C" {
    static mut exit_signalled: c_int; // volatile sig_atomic_t in C
    static mut restart_ssh: c_int;
    static mut dolongjmp: c_int;

    fn siglongjmp(env: *mut JmpBuf, val: c_int) -> !;
}

/// Signal handler. Async-signal-safe: only sets atomic-ish ints,
/// optionally calls _exit or siglongjmp — no allocations, no Drop,
/// no Rust runtime hooks.
///
/// # Safety
/// Invoked by the kernel from signal-delivery context. Must not be
/// called from regular Rust code other than tests.
#[no_mangle]
pub unsafe extern "C" fn sig_catch(sig: c_int) {
    if sig == libc::SIGUSR1 {
        restart_ssh = 1;
    } else if sig == libc::SIGTERM || sig == libc::SIGINT {
        if exit_signalled != 0 {
            // Second termination signal during shutdown — bail.
            libc::_exit(1);
        }
        exit_signalled = 1;
    }

    if dolongjmp != 0 {
        dolongjmp = 0;
        siglongjmp(&raw mut jumpbuf, sig);
    }
}

/// Install sig_catch on SIGTERM/SIGINT only, with empty sa_mask.
/// Called once at the start of ssh_run; survives across iterations
/// thanks to unset_sig_handlers leaving SIGTERM/SIGINT alone.
#[no_mangle]
pub unsafe extern "C" fn set_exit_sig_handler() {
    let mut act: libc::sigaction = std::mem::zeroed();
    act.sa_sigaction = sig_catch as *const () as usize;
    libc::sigemptyset(&mut act.sa_mask);
    act.sa_flags = 0;

    libc::sigaction(libc::SIGTERM, &act, std::ptr::null_mut());
    libc::sigaction(libc::SIGINT, &act, std::ptr::null_mut());
}

/// Install sig_catch on the full set of signals tracked during the
/// ssh_watch loop. The mask blocks every other signal sig_catch
/// listens for, so handler invocation is serialized.
#[no_mangle]
pub unsafe extern "C" fn set_sig_handlers() {
    let mut act: libc::sigaction = std::mem::zeroed();
    act.sa_sigaction = sig_catch as *const () as usize;
    act.sa_flags = 0;

    libc::sigemptyset(&mut act.sa_mask);
    libc::sigaddset(&mut act.sa_mask, libc::SIGTERM);
    libc::sigaddset(&mut act.sa_mask, libc::SIGINT);
    libc::sigaddset(&mut act.sa_mask, libc::SIGHUP);
    libc::sigaddset(&mut act.sa_mask, libc::SIGUSR1);
    libc::sigaddset(&mut act.sa_mask, libc::SIGUSR2);
    libc::sigaddset(&mut act.sa_mask, libc::SIGCHLD);
    libc::sigaddset(&mut act.sa_mask, libc::SIGALRM);
    libc::sigaddset(&mut act.sa_mask, libc::SIGPIPE);

    libc::sigaction(libc::SIGTERM, &act, std::ptr::null_mut());
    libc::sigaction(libc::SIGINT, &act, std::ptr::null_mut());
    libc::sigaction(libc::SIGHUP, &act, std::ptr::null_mut());
    libc::sigaction(libc::SIGUSR1, &act, std::ptr::null_mut());
    libc::sigaction(libc::SIGUSR2, &act, std::ptr::null_mut());
    libc::sigaction(libc::SIGCHLD, &act, std::ptr::null_mut());

    // SIGALRM uses SA_RESTART so blocking syscalls retry rather
    // than returning EINTR (the longjmp via sig_catch unwinds them).
    act.sa_flags |= libc::SA_RESTART;
    libc::sigaction(libc::SIGALRM, &act, std::ptr::null_mut());

    // SIGPIPE: ignore (a broken pipe shouldn't kill autossh).
    act.sa_sigaction = libc::SIG_IGN;
    act.sa_flags = 0;
    libc::sigaction(libc::SIGPIPE, &act, std::ptr::null_mut());
}

/// Restore default disposition for the signals set_sig_handlers
/// installed, EXCEPT SIGTERM/SIGINT — those persist across
/// iterations of the ssh_run loop so a Ctrl+C between forks still
/// works.
#[no_mangle]
pub unsafe extern "C" fn unset_sig_handlers() {
    let mut act: libc::sigaction = std::mem::zeroed();
    act.sa_sigaction = libc::SIG_DFL;
    libc::sigemptyset(&mut act.sa_mask);
    act.sa_flags = 0;

    libc::sigaction(libc::SIGHUP, &act, std::ptr::null_mut());
    libc::sigaction(libc::SIGUSR1, &act, std::ptr::null_mut());
    libc::sigaction(libc::SIGUSR2, &act, std::ptr::null_mut());
    libc::sigaction(libc::SIGCHLD, &act, std::ptr::null_mut());
    libc::sigaction(libc::SIGALRM, &act, std::ptr::null_mut());
    libc::sigaction(libc::SIGPIPE, &act, std::ptr::null_mut());
}
