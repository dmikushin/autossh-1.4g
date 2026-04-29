//! SSH child reaper.
//!
//! `ssh_wait(options)`: waits on cchild (WNOHANG or blocking) and
//! interprets the exit status, returning P_CONTINUE/P_RESTART/
//! P_EXITOK/P_EXITERR per the long-standing autossh policy:
//!
//!   - waitpid returned 0 / not yet exited → P_CONTINUE
//!   - WIFSIGNALED → restart
//!   - WIFEXITED 0 → exit ok
//!   - WIFEXITED 255 → restart (auth/connection drop), unless on the
//!                     first try within gate_time (then EXITERR)
//!   - WIFEXITED 1/2 on first try with gate_time != 0 → EXITERR
//!   - WIFEXITED 1/2 thereafter or with gate_time=0 → restart
//!   - any other exit code → EXITERR
//!
//! Test coverage: tests/unit/test_ssh_wait.c (11 cases covering
//! every branch).

use libc::{c_char, c_double, c_int, time_t};

use crate::errlog;
use crate::log::cstr_or;

const P_CONTINUE: c_int = 0;
const P_RESTART:  c_int = 1;
const P_EXITOK:   c_int = 2;
const P_EXITERR:  c_int = 3;

extern "C" {
    static mut cchild: c_int;
    static mut start_count: c_int;
    static mut gate_time: c_double;
    static mut start_time: time_t;
    static __progname: *const c_char;
}

/// `ssh_wait(options)`: see module docs.
#[no_mangle]
pub unsafe extern "C" fn ssh_wait(options: c_int) -> c_int {
    let mut status: c_int = 0;
    let r = libc::waitpid(cchild, &mut status as *mut c_int, options);

    if r <= 0 {
        return P_CONTINUE;
    }

    // Drain any remaining stderr before analyzing exit.
    crate::stderr_drain::check_ssh_stderr();

    if libc::WIFSIGNALED(status) {
        let sig = libc::WTERMSIG(status);
        errlog!(libc::LOG_INFO, "ssh exited on signal {}, restarting ssh", sig);
        return P_RESTART;
    }

    if libc::WIFEXITED(status) {
        let evalue = libc::WEXITSTATUS(status);
        let progname = cstr_or(__progname, "autossh");

        if start_count == 1 && gate_time != 0.0 {
            let mut now: time_t = 0;
            libc::time(&raw mut now);
            if libc::difftime(now, start_time) <= gate_time {
                errlog!(libc::LOG_ERR,
                    "ssh exited prematurely with status {}; {} exiting",
                    evalue, progname);
                return P_EXITERR;
            }
        }

        match evalue {
            255 => {
                errlog!(libc::LOG_INFO,
                    "ssh exited with error status {}; restarting ssh", evalue);
                P_RESTART
            }
            0 => {
                errlog!(libc::LOG_INFO,
                    "ssh exited with status {}; {} exiting", evalue, progname);
                P_EXITOK
            }
            1 | 2 => {
                if start_count > 1 || gate_time == 0.0 {
                    errlog!(libc::LOG_INFO,
                        "ssh exited with error status {}; restarting ssh", evalue);
                    P_RESTART
                } else {
                    errlog!(libc::LOG_INFO,
                        "ssh exited with status {}; {} exiting", evalue, progname);
                    P_EXITERR
                }
            }
            _ => {
                errlog!(libc::LOG_INFO,
                    "ssh exited with status {}; {} exiting", evalue, progname);
                P_EXITERR
            }
        }
    } else {
        // Stopped/continued etc. — extremely rare, treat as continue.
        P_CONTINUE
    }
}
