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

    fn errlog(level: c_int, fmt: *const c_char, ...);
    fn xerrlog(level: c_int, fmt: *const c_char, ...);

    // glibc has srandom() (stdlib.h) but the Rust libc crate does
    // not expose it on Linux at our pinned version. Declare it
    // directly — links to libc.so's symbol.
    fn srandom(seed: libc::c_uint);
}

/// `ssh_run(sock, av)`: enter the start/restart loop. Returns
/// P_EXITOK on graceful end (max_start reached, lifetime exceeded,
/// or child exit-success), P_EXITERR on signal-driven termination.
#[no_mangle]
pub unsafe extern "C" fn ssh_run(sock: c_int, av: *mut *mut c_char) -> c_int {
    // Seed RNG (not strictly needed for the port loop, but matches
    // C's behaviour for downstream callers of random()).
    let mut tv: libc::timeval = std::mem::zeroed();
    libc::gettimeofday(&mut tv, std::ptr::null_mut());
    let pid = libc::getpid() as libc::c_uint;
    srandom(
        (pid ^ tv.tv_usec as libc::c_uint ^ tv.tv_sec as libc::c_uint)
            as libc::c_uint,
    );

    crate::signals::set_exit_sig_handler();

    while max_start < 0 || start_count < max_start {
        if crate::lifetime::exceeded_lifetime() != 0 {
            return P_EXITOK;
        }
        restart_ssh = 0;
        start_count += 1;
        crate::grace::grace_time(start_time);
        if exit_signalled != 0 {
            errlog(libc::LOG_ERR, c"signalled to exit".as_ptr());
            return P_EXITERR;
        }
        libc::time(&raw mut start_time);
        port_fwd_failed = 0;
        pipe_lost_time = 0;
        libc::time(&raw mut last_stderr_time);

        if max_start < 0 {
            errlog(libc::LOG_INFO,
                c"starting ssh (count %d)".as_ptr(),
                start_count);
        } else {
            errlog(libc::LOG_INFO,
                c"starting ssh (count %d of %d)".as_ptr(),
                start_count, max_start);
        }

        // Pipe for capturing SSH stderr.
        let mut stderr_pipe: [c_int; 2] = [-1, -1];
        if libc::pipe(stderr_pipe.as_mut_ptr()) < 0 {
            errlog(libc::LOG_ERR,
                c"pipe: %s".as_ptr(),
                libc::strerror(*libc::__errno_location()));
            stderr_pipe[0] = -1;
            stderr_pipe[1] = -1;
        }

        cchild = libc::fork();
        match cchild {
            0 => {
                // CHILD: redirect stderr → pipe write end, exec ssh.
                if stderr_pipe[1] >= 0 {
                    libc::close(stderr_pipe[0]);
                    libc::dup2(stderr_pipe[1], STDERR_FILENO);
                    libc::close(stderr_pipe[1]);
                }
                let av0 = *av;
                errlog(libc::LOG_DEBUG,
                    c"child of %d execing %s".as_ptr(),
                    libc::getppid() as c_int, av0);
                libc::execvp(av0, av as *const *const c_char);
                // execvp failed: log, signal parent, exit.
                errlog(libc::LOG_ERR, c"%s: %s".as_ptr(), av0,
                    libc::strerror(*libc::__errno_location()));
                libc::kill(libc::getppid(), SIGTERM);
                libc::_exit(1);
            }
            -1 => {
                // fork failed: cleanup, abort.
                cchild = 0;
                if stderr_pipe[0] >= 0 { libc::close(stderr_pipe[0]); }
                if stderr_pipe[1] >= 0 { libc::close(stderr_pipe[1]); }
                xerrlog(libc::LOG_ERR,
                    c"fork: %s".as_ptr(),
                    libc::strerror(*libc::__errno_location()));
                // xerrlog never returns; unreachable.
            }
            _ => {
                // PARENT: keep read end of pipe, install signal
                // handlers, run ssh_watch, tear down.
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
                errlog(libc::LOG_INFO,
                    c"ssh child pid is %d".as_ptr(), cchild);

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
                // P_RESTART or P_CONTINUE: loop.
            }
        }
    }

    errlog(libc::LOG_INFO,
        c"max start count reached; exiting".as_ptr());
    P_EXITOK
}
