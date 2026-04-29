//! Aggressive process killer for the SSH child.
//!
//! Behavior (preserves the recent fix in commit 8986ca6):
//!   1. close ssh_stderr_fd if open.
//!   2. send SIGTERM, then up to SIGTERM_GRACE seconds of WNOHANG
//!      polling with sleep(1) between attempts.
//!   3. if still alive: send SIGKILL, wait up to SIGKILL_WAIT.
//!   4. if STILL alive (D-state child): give up, set cchild = 0,
//!      log an abandon message. Total worst case: ~4 seconds.
//!
//! Test coverage: tests/unit/test_ssh_kill.c (8 cases including
//! the "completes within 4 seconds" contract).

use libc::c_int;

use crate::errlog;
use crate::log::cstr_or;

const SIGTERM_GRACE: c_int = 2;
const SIGKILL_WAIT:  c_int = 2;

extern "C" {
    static mut cchild: c_int;
    static mut ssh_stderr_fd: c_int;
}

/// `ssh_kill()`: kill the SSH child process aggressively.
#[no_mangle]
pub unsafe extern "C" fn ssh_kill() {
    if ssh_stderr_fd >= 0 {
        libc::close(ssh_stderr_fd);
        ssh_stderr_fd = -1;
    }

    if cchild == 0 {
        return;
    }

    libc::kill(cchild, libc::SIGTERM);

    let mut status: c_int = 0;
    if reap_with_grace(SIGTERM_GRACE, &mut status, false) {
        return;
    }

    // SIGTERM didn't work, escalate to SIGKILL.
    errlog!(libc::LOG_WARNING,
        "ssh child {} did not exit after {} seconds, sending SIGKILL",
        cchild, SIGTERM_GRACE);
    libc::kill(cchild, libc::SIGKILL);

    if reap_with_grace(SIGKILL_WAIT, &mut status, true) {
        return;
    }

    errlog!(libc::LOG_ERR,
        "ssh child {} not dead after SIGKILL + {} seconds (likely in uninterruptible state); abandoning",
        cchild, SIGKILL_WAIT);
    cchild = 0;
}

/// Helper: WNOHANG-poll up to `seconds`, sleeping 1s between attempts.
/// Returns true if `cchild` was reaped (or already gone) and we
/// should stop. `after_sigkill` chooses the error message variant.
unsafe fn reap_with_grace(
    seconds: c_int,
    status: &mut c_int,
    after_sigkill: bool,
) -> bool {
    let mut waited: c_int = 0;
    while waited < seconds {
        let errno_ptr = libc::__errno_location();
        *errno_ptr = 0;
        let w = libc::waitpid(cchild, status as *mut c_int, libc::WNOHANG);
        if w > 0 {
            cchild = 0;
            return true;
        }
        if w < 0 {
            let e = *errno_ptr;
            if e == libc::ECHILD {
                cchild = 0;
                return true;
            }
            if e != libc::EINTR {
                let err_msg = cstr_or(libc::strerror(e), "?");
                if after_sigkill {
                    errlog!(libc::LOG_ERR, "waitpid after SIGKILL: {}", err_msg);
                } else {
                    errlog!(libc::LOG_ERR, "waitpid: {}", err_msg);
                }
                cchild = 0;
                return true;
            }
        }
        libc::sleep(1);
        waited += 1;
    }
    false
}
