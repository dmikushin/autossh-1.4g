//! `ssh_run`: outer loop that forks/execs the SSH child and feeds
//! ssh_watch.
//!
//! Wiring:
//!   - install set_exit_sig_handler once (covers SIGTERM/SIGINT
//!     across iterations).
//!   - per iteration: pipe + fork + dup2 stderr + execvp in the
//!     child; in the parent, set_sig_handlers + ssh_watch + tear
//!     down handlers.
//!   - retry until max_start (if positive) or forever.
//!
//! Tested via integration: tests/integration/run.sh
//! (test_watchdog_silence, test_port_fwd_fail, test_max_lifetime,
//! …) which exercise the full fork+exec path against mock_ssh.sh.

use libc::{
    c_char, c_int, fcntl, F_GETFL, F_SETFL, O_NONBLOCK, STDERR_FILENO,
    SIGTERM, time_t,
};

use crate::log::cstr_or;
use crate::{errlog, xerrlog};

const P_EXITOK:  c_int = 2;
const P_EXITERR: c_int = 3;

extern "C" {
    static mut cchild: c_int;
    static mut ssh_stderr_fd: c_int;
    static mut start_count: c_int;
    static mut start_time: time_t;
    static mut max_start: c_int;
    static mut last_stderr_time: time_t;
    static mut pipe_lost_time: time_t;
    static mut port_fwd_failed: c_int;
    static mut exit_signalled: c_int;
    static mut restart_ssh: c_int;
    static mut dolongjmp: c_int;

    fn srandom(seed: libc::c_uint);
}

/// `ssh_run(sock, av)`: enter the start/restart loop.
#[no_mangle]
pub unsafe extern "C" fn ssh_run(sock: c_int, av: *mut *mut c_char) -> c_int {
    let mut tv: libc::timeval = std::mem::zeroed();
    libc::gettimeofday(&mut tv, std::ptr::null_mut());
    let pid = libc::getpid() as libc::c_uint;
    srandom(
        (pid ^ tv.tv_usec as libc::c_uint ^ tv.tv_sec as libc::c_uint)
            as libc::c_uint,
    );

    crate::signals::set_exit_sig_handler();

    // Stage the LD_PRELOAD watchdog (extract once, install in env
    // so the SSH child inherits it on execvp).
    crate::stuck_detector::install_for_child();

    while max_start < 0 || start_count < max_start {
        if crate::lifetime::exceeded_lifetime() != 0 {
            return P_EXITOK;
        }
        restart_ssh = 0;
        start_count += 1;
        crate::grace::grace_time(start_time);
        if exit_signalled != 0 {
            errlog!(libc::LOG_ERR, "signalled to exit");
            return P_EXITERR;
        }
        libc::time(&raw mut start_time);
        port_fwd_failed = 0;
        pipe_lost_time = 0;
        // Sentinel: 0 means "ssh has not produced any stderr output
        // yet". check_ssh_stderr will write the real time on the
        // first successful read; until then, the silence watchdog
        // in ssh_watch treats this child as still in initial
        // connect.
        last_stderr_time = 0;

        if max_start < 0 {
            errlog!(libc::LOG_INFO, "starting ssh (count {})", start_count);
        } else {
            errlog!(libc::LOG_INFO,
                "starting ssh (count {} of {})", start_count, max_start);
        }

        // Pipe for capturing SSH stderr.
        let mut stderr_pipe: [c_int; 2] = [-1, -1];
        if libc::pipe(stderr_pipe.as_mut_ptr()) < 0 {
            let err = cstr_or(libc::strerror(*libc::__errno_location()), "?");
            errlog!(libc::LOG_ERR, "pipe: {}", err);
            stderr_pipe[0] = -1;
            stderr_pipe[1] = -1;
        }

        cchild = libc::fork();
        match cchild {
            0 => {
                // CHILD: must avoid Rust allocation between fork and
                // execvp; the libc errlog format-string FFI is the
                // safer choice here.
                if stderr_pipe[1] >= 0 {
                    libc::close(stderr_pipe[0]);
                    libc::dup2(stderr_pipe[1], STDERR_FILENO);
                    libc::close(stderr_pipe[1]);
                }
                let av0 = *av;
                libc::execvp(av0, av as *const *const c_char);
                // execvp failed. Allocation is risky now but we're
                // about to _exit anyway; fall back to a fixed
                // pre-formatted message.
                let _ = libc::write(
                    libc::STDERR_FILENO,
                    c"autossh: execvp failed\n".as_ptr() as *const _,
                    23,
                );
                libc::kill(libc::getppid(), SIGTERM);
                libc::_exit(1);
            }
            -1 => {
                cchild = 0;
                if stderr_pipe[0] >= 0 { libc::close(stderr_pipe[0]); }
                if stderr_pipe[1] >= 0 { libc::close(stderr_pipe[1]); }
                let err = cstr_or(libc::strerror(*libc::__errno_location()), "?");
                xerrlog!(libc::LOG_ERR, "fork: {}", err);
            }
            _ => {
                if stderr_pipe[1] >= 0 {
                    libc::close(stderr_pipe[1]);
                }
                ssh_stderr_fd = stderr_pipe[0];
                if ssh_stderr_fd >= 0 {
                    let flags = fcntl(ssh_stderr_fd, F_GETFL, 0);
                    if flags >= 0 {
                        fcntl(ssh_stderr_fd, F_SETFL, flags | O_NONBLOCK);
                    }
                }
                errlog!(libc::LOG_INFO, "ssh child pid is {}", cchild);

                crate::signals::set_sig_handlers();
                let retval = crate::watch::ssh_watch(sock);
                dolongjmp = 0;
                crate::lifetime::clear_alarm_timer();
                crate::signals::unset_sig_handlers();
                if ssh_stderr_fd >= 0 {
                    libc::close(ssh_stderr_fd);
                    ssh_stderr_fd = -1;
                }
                if retval == P_EXITOK || retval == P_EXITERR {
                    return retval;
                }
            }
        }
    }

    errlog!(libc::LOG_INFO, "max start count reached; exiting");
    P_EXITOK
}
