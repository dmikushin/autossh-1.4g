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

/// Opaque handle to C's `sigjmp_buf jumpbuf;`. The libc crate doesn't
/// expose `sigjmp_buf` (which on glibc is a typedef for a struct
/// array), so we use the canonical C-FFI pattern of an extern type
/// referenced only by pointer. Storage is owned by autossh.c.
#[repr(C)]
pub struct JmpBuf {
    _private: [u8; 0],
}

extern "C" {
    static mut exit_signalled: c_int; // volatile sig_atomic_t in C
    static mut restart_ssh: c_int;
    static mut dolongjmp: c_int;
    static mut jumpbuf: JmpBuf;

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
