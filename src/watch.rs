//! Main monitoring loop.
//!
//! `ssh_watch(sock)`: per-iteration:
//!   1. arm SIGALRM via alarm(secs_left), set dolongjmp=1
//!   2. block in poll()/pause() until either:
//!        - SIGCHLD wakes us → loop, ssh_wait(WNOHANG) reaps
//!        - poll() returns events → handle stderr / pipe-loss
//!        - signal arrives → sig_catch siglongjmp's back to the
//!          sigsetjmp at the top of the loop, val != 0
//!   3. dispatch on val: SIGINT/TERM/QUIT/ABRT → exit, SIGALRM →
//!      watchdog/conn_test, else loop
//!
//! Critical for safety: this function uses sigsetjmp/siglongjmp.
//! Rust types with Drop in scope would leak when longjmp'd over —
//! so the body deliberately uses only POD: c_int, time_t, c_double,
//! pollfd. No Vec, no String, no Box, no file handles.
//!
//! Test coverage: tests/unit/test_ssh_watch.c (11 cases including
//! the SIGALRM-watchdog branches added in the recent fix).

use libc::{
    c_char, c_double, c_int, c_uint, pollfd, time_t, POLLERR, POLLHUP, POLLIN,
    SIGABRT, SIGALRM, SIGINT, SIGQUIT, SIGTERM, WNOHANG,
};

use crate::signals::JmpBuf;

const P_CONTINUE: c_int = 0;
const P_RESTART:  c_int = 1;
const P_EXITOK:   c_int = 2;
const P_EXITERR:  c_int = 3;

const PORT_FWD_FAIL_DELAY: c_uint = 5;

extern "C" {
    static mut ssh_stderr_fd: c_int;
    static mut pid_start_time: time_t;
    static mut pipe_lost_time: time_t;
    static mut last_stderr_time: time_t;
    static mut max_session: c_double;
    static mut max_lifetime: c_double;
    static mut poll_time: c_int;
    static mut first_poll_time: c_int;
    static mut writep: *mut c_char;
    static mut mhost: *mut c_char;

    static mut exit_signalled: c_int;
    static mut restart_ssh: c_int;
    static mut dolongjmp: c_int;
    static mut jumpbuf: JmpBuf;

    fn __sigsetjmp(env: *mut JmpBuf, savemask: c_int) -> c_int;
    fn errlog(level: c_int, fmt: *const c_char, ...);

    // Still C-resident.
    fn conn_test(sock: c_int, host: *const c_char, port: *const c_char) -> c_int;
}

/// `static int secs_left` from the C original (function-scope static).
static mut SECS_LEFT: c_int = 0;

#[no_mangle]
pub unsafe extern "C" fn ssh_watch(sock: c_int) -> c_int {
    let mut my_poll_time = first_poll_time;

    loop {
        if restart_ssh != 0 {
            errlog(libc::LOG_INFO,
                c"signalled to kill and restart ssh".as_ptr());
            crate::kill::ssh_kill();
            return P_RESTART;
        }

        let val = __sigsetjmp(&raw mut jumpbuf, 1);
        if val == 0 {
            // ----- TRY branch (no signal yet) -----
            let r = crate::wait::ssh_wait(WNOHANG);
            if r != P_CONTINUE {
                return r;
            }

            SECS_LEFT = crate::lifetime::clear_alarm_timer() as c_int;
            if SECS_LEFT == 0 {
                SECS_LEFT = my_poll_time;
            }
            my_poll_time = poll_time;

            if max_lifetime != 0.0 {
                let mut now: time_t = 0;
                libc::time(&raw mut now);
                let secs_to_shutdown =
                    max_lifetime - libc::difftime(now, pid_start_time);
                if secs_to_shutdown < poll_time as c_double {
                    SECS_LEFT = secs_to_shutdown as c_int;
                }
            }

            // Watchdog clamp: ensure SIGALRM fires often enough that
            // the SIGALRM handler can detect stuck SSH within
            // max_session seconds.
            if max_session > 0.0 && (SECS_LEFT as c_double) > max_session {
                SECS_LEFT = max_session as c_int;
            }

            dolongjmp = 1;
            libc::alarm(SECS_LEFT as c_uint);

            // Race: signal could have arrived while setting up.
            if exit_signalled != 0 {
                errlog(libc::LOG_INFO,
                    c"signalled to exit".as_ptr());
                crate::kill::ssh_kill();
                return P_EXITERR;
            }

            if ssh_stderr_fd >= 0 {
                let mut spfd = pollfd {
                    fd: ssh_stderr_fd,
                    events: POLLIN,
                    revents: 0,
                };
                libc::poll(&mut spfd, 1, SECS_LEFT * 1000);

                if (spfd.revents & POLLIN) != 0 {
                    if crate::stderr_drain::check_ssh_stderr() != 0 {
                        crate::kill::ssh_kill();
                        errlog(libc::LOG_INFO,
                            c"waiting %d seconds for port to be released".as_ptr(),
                            PORT_FWD_FAIL_DELAY as c_int);
                        libc::sleep(PORT_FWD_FAIL_DELAY);
                        return P_RESTART;
                    }
                }
                if (spfd.revents & (POLLHUP | POLLERR)) != 0
                    && (spfd.revents & POLLIN) == 0
                {
                    errlog(libc::LOG_INFO,
                        c"SSH stderr pipe closed".as_ptr());
                    libc::close(ssh_stderr_fd);
                    ssh_stderr_fd = -1;
                    if pipe_lost_time == 0 {
                        libc::time(&raw mut pipe_lost_time);
                    }
                }
            } else {
                libc::pause();
            }
        } else {
            // ----- CATCH branch (sig_catch siglongjmp'd here) -----
            match val {
                v if v == SIGINT
                    || v == SIGTERM
                    || v == SIGQUIT
                    || v == SIGABRT =>
                {
                    errlog(libc::LOG_INFO,
                        c"received signal to exit (%d)".as_ptr(), val);
                    crate::kill::ssh_kill();
                    return P_EXITERR;
                }
                v if v == SIGALRM => {
                    let r = crate::lifetime::exceeded_lifetime();
                    if r != 0 {
                        crate::kill::ssh_kill();
                        return P_EXITOK;
                    }

                    if max_session > 0.0 {
                        let mut now: time_t = 0;
                        libc::time(&raw mut now);
                        if pipe_lost_time != 0
                            && libc::difftime(now, pipe_lost_time)
                                >= max_session
                        {
                            errlog(libc::LOG_WARNING,
                                c"ssh child stuck for %.0f secs after stderr pipe lost; restarting".as_ptr(),
                                libc::difftime(now, pipe_lost_time));
                            crate::kill::ssh_kill();
                            return P_RESTART;
                        }
                        if last_stderr_time != 0
                            && libc::difftime(now, last_stderr_time)
                                >= max_session
                        {
                            errlog(libc::LOG_WARNING,
                                c"ssh child silent on stderr for %.0f secs; considered stuck, restarting".as_ptr(),
                                libc::difftime(now, last_stderr_time));
                            crate::kill::ssh_kill();
                            return P_RESTART;
                        }
                    }

                    if crate::stderr_drain::check_ssh_stderr() != 0 {
                        crate::kill::ssh_kill();
                        errlog(libc::LOG_INFO,
                            c"waiting %d seconds for port to be released".as_ptr(),
                            PORT_FWD_FAIL_DELAY as c_int);
                        libc::sleep(PORT_FWD_FAIL_DELAY);
                        return P_RESTART;
                    }

                    if !writep.is_null() && sock != -1
                        && conn_test(sock, mhost, writep) == 0
                    {
                        errlog(libc::LOG_INFO,
                            c"port down, restarting ssh".as_ptr());
                        crate::kill::ssh_kill();
                        return P_RESTART;
                    }
                }
                _ => {}
            }
        }
    }
}

// (no module-level helpers; ssh_watch is the only entrypoint)
