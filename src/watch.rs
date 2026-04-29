//! Main monitoring loop.
//!
//! `ssh_watch(sock)`: per-iteration:
//!   1. block all signals sig_catch handles.
//!   2. ssh_wait(WNOHANG) — race-free because signals are blocked.
//!   3. arm SIGALRM via alarm(secs_left), set dolongjmp=1.
//!   4. atomic unblock + wait via ppoll() / sigsuspend() with the
//!      pre-block sigmask. This is the fix for the SIGCHLD race
//!      previously documented in KNOWN_ISSUES.md: the signal can
//!      now ONLY arrive while we are blocked in ppoll/sigsuspend,
//!      so sig_catch's siglongjmp is guaranteed to land back at
//!      our sigsetjmp.
//!   5. dispatch on val: SIGINT/TERM/QUIT/ABRT → exit, SIGALRM →
//!      watchdog/conn_test, else loop.
//!
//! All return paths restore the caller's signal mask via
//! sigprocmask(SIG_SETMASK, &savedmask, ...).
//!
//! Drop-free invariant: this function uses sigsetjmp/siglongjmp.
//! Rust types with Drop in scope would leak when longjmp'd over —
//! so the body deliberately uses only POD: c_int, time_t, c_double,
//! pollfd. No Vec, no String, no Box, no file handles.
//!
//! errlog! macros allocate (format → String). To stay safe we
//! clear `dolongjmp = 0` BEFORE every errlog! inside the longjmp-
//! armed window. After ppoll returns we clear it unconditionally.
//!
//! Test coverage: tests/unit/test_ssh_watch.c (11 cases including
//! the SIGALRM-watchdog branches).

use libc::{
    c_char, c_double, c_int, c_uint, pollfd, sigset_t, time_t, timespec,
    POLLERR, POLLHUP, POLLIN, SIGABRT, SIGALRM, SIGCHLD, SIGHUP, SIGINT,
    SIGQUIT, SIGTERM, SIGUSR1, SIGUSR2, WNOHANG,
};
use std::ptr;

use crate::errlog;
use crate::signals::JmpBuf;

const P_CONTINUE: c_int = 0;
const P_RESTART:  c_int = 1;
const P_EXITOK:   c_int = 2;
const P_EXITERR:  c_int = 3;

const PORT_FWD_FAIL_DELAY: c_uint = 5;

extern "C" {
    static mut ssh_stderr_fd: c_int;
    static mut pid_start_time: time_t;
    static mut start_time: time_t;
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
    fn conn_test(sock: c_int, host: *const c_char, port: *const c_char) -> c_int;
}

/// `static int secs_left` from the C original (function-scope static).
static mut SECS_LEFT: c_int = 0;

/// Build the signal set sig_catch handles, used for sigprocmask.
unsafe fn build_blockmask() -> sigset_t {
    let mut s: sigset_t = std::mem::zeroed();
    libc::sigemptyset(&mut s);
    libc::sigaddset(&mut s, SIGCHLD);
    libc::sigaddset(&mut s, SIGALRM);
    libc::sigaddset(&mut s, SIGINT);
    libc::sigaddset(&mut s, SIGTERM);
    libc::sigaddset(&mut s, SIGHUP);
    libc::sigaddset(&mut s, SIGUSR1);
    libc::sigaddset(&mut s, SIGUSR2);
    s
}

/// Restore `mask` and return `rc`. Single restore-then-return
/// helper so every exit path of ssh_watch leaves the caller's
/// signal mask intact.
#[inline(always)]
unsafe fn ret(savedmask: &sigset_t, rc: c_int) -> c_int {
    libc::sigprocmask(libc::SIG_SETMASK, savedmask as *const _, ptr::null_mut());
    rc
}

#[no_mangle]
pub unsafe extern "C" fn ssh_watch(sock: c_int) -> c_int {
    let mut my_poll_time = first_poll_time;

    // Block sig_catch's signals; remember the caller's mask so the
    // wait calls can atomically unblock-and-wait, and so we can
    // restore on return.
    let blockmask = build_blockmask();
    let mut savedmask: sigset_t = std::mem::zeroed();
    libc::sigprocmask(libc::SIG_BLOCK, &blockmask, &mut savedmask);

    loop {
        // Re-block at top of every iteration: the CATCH branch may
        // have unblocked signals so ssh_kill could be interrupted,
        // and a default-case fall-through brings us here without a
        // re-block.
        libc::sigprocmask(libc::SIG_BLOCK, &blockmask, ptr::null_mut());

        if restart_ssh != 0 {
            errlog!(libc::LOG_INFO, "signalled to kill and restart ssh");
            crate::kill::ssh_kill();
            return ret(&savedmask, P_RESTART);
        }

        let val = __sigsetjmp(&raw mut jumpbuf, 1);
        if val == 0 {
            // ----- TRY branch (signals blocked) -----
            let r = crate::wait::ssh_wait(WNOHANG);
            if r != P_CONTINUE {
                return ret(&savedmask, r);
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

            if max_session > 0.0 && (SECS_LEFT as c_double) > max_session {
                SECS_LEFT = max_session as c_int;
            }

            dolongjmp = 1;
            libc::alarm(SECS_LEFT as c_uint);

            // Drain any stale exit_signalled from earlier (e.g. set
            // before set_sig_handlers but after set_exit_sig_handler).
            if exit_signalled != 0 {
                dolongjmp = 0;
                errlog!(libc::LOG_INFO, "signalled to exit");
                crate::kill::ssh_kill();
                return ret(&savedmask, P_EXITERR);
            }

            // Atomic unblock+wait: ppoll for the stderr-fd case,
            // sigsuspend otherwise. Either delivers the pending or
            // newly-arriving signal under savedmask, sig_catch fires,
            // siglongjmp returns to sigsetjmp above with val != 0
            // and the original (blocked) mask restored.
            if ssh_stderr_fd >= 0 {
                let mut spfd = pollfd {
                    fd: ssh_stderr_fd,
                    events: POLLIN,
                    revents: 0,
                };
                // ppoll's timespec is its OWN timeout. We rely on
                // alarm()+SIGALRM for the watchdog dispatch, so
                // don't time out here — pass NULL and let SIGALRM
                // (or any other signal) wake us.
                libc::ppoll(&mut spfd, 1, ptr::null(), &savedmask);
                dolongjmp = 0;

                if (spfd.revents & POLLIN) != 0 {
                    if crate::stderr_drain::check_ssh_stderr() != 0 {
                        crate::kill::ssh_kill();
                        errlog!(libc::LOG_INFO,
                            "waiting {} seconds for port to be released",
                            PORT_FWD_FAIL_DELAY);
                        libc::sleep(PORT_FWD_FAIL_DELAY);
                        return ret(&savedmask, P_RESTART);
                    }
                }
                if (spfd.revents & (POLLHUP | POLLERR)) != 0
                    && (spfd.revents & POLLIN) == 0
                {
                    errlog!(libc::LOG_INFO, "SSH stderr pipe closed");
                    libc::close(ssh_stderr_fd);
                    ssh_stderr_fd = -1;
                    if pipe_lost_time == 0 {
                        libc::time(&raw mut pipe_lost_time);
                    }
                }
            } else {
                libc::sigsuspend(&savedmask);
                dolongjmp = 0;
            }
        } else {
            // ----- CATCH branch (sig_catch already cleared dolongjmp) -----
            //
            // sigsetjmp(jumpbuf, 1) restored our blocked mask. We
            // unblock here so ssh_kill / errlog can be interrupted
            // by a second termination signal — that's what powers
            // the double-Ctrl+C force-exit.
            libc::sigprocmask(libc::SIG_SETMASK, &savedmask, ptr::null_mut());
            match val {
                v if v == SIGINT
                    || v == SIGTERM
                    || v == SIGQUIT
                    || v == SIGABRT =>
                {
                    errlog!(libc::LOG_INFO,
                        "received signal to exit ({})", val);
                    crate::kill::ssh_kill();
                    return ret(&savedmask, P_EXITERR);
                }
                v if v == SIGALRM => {
                    let r = crate::lifetime::exceeded_lifetime();
                    if r != 0 {
                        crate::kill::ssh_kill();
                        return ret(&savedmask, P_EXITOK);
                    }

                    if max_session > 0.0 {
                        let mut now: time_t = 0;
                        libc::time(&raw mut now);
                        // Pipe-lost watchdog: SSH stderr fd was
                        // closed; child is dying or already dead.
                        if pipe_lost_time != 0
                            && libc::difftime(now, pipe_lost_time)
                                >= max_session
                        {
                            errlog!(libc::LOG_WARNING,
                                "ssh child stuck for {:.0} secs after stderr pipe lost; restarting",
                                libc::difftime(now, pipe_lost_time));
                            crate::kill::ssh_kill();
                            return ret(&savedmask, P_RESTART);
                        }
                        // Initial-connect watchdog: ssh has *never*
                        // produced any stderr output (last_stderr_time
                        // is the 0 sentinel set by ssh_run, cleared
                        // by check_ssh_stderr on the first read).
                        // Once the child says ANYTHING — even just
                        // "Pseudo-terminal will not be allocated"
                        // — the connection is considered established
                        // and silence is normal (`ssh -N` is mute by
                        // design). This prevents the watchdog from
                        // killing healthy long-running sessions.
                        if last_stderr_time == 0
                            && libc::difftime(now, start_time)
                                >= max_session
                        {
                            errlog!(libc::LOG_WARNING,
                                "ssh produced no stderr in {:.0} secs since start; assuming stuck in connect, restarting",
                                libc::difftime(now, start_time));
                            crate::kill::ssh_kill();
                            return ret(&savedmask, P_RESTART);
                        }
                    }

                    if crate::stderr_drain::check_ssh_stderr() != 0 {
                        crate::kill::ssh_kill();
                        errlog!(libc::LOG_INFO,
                            "waiting {} seconds for port to be released",
                            PORT_FWD_FAIL_DELAY);
                        libc::sleep(PORT_FWD_FAIL_DELAY);
                        return ret(&savedmask, P_RESTART);
                    }

                    if !writep.is_null() && sock != -1
                        && conn_test(sock, mhost, writep) == 0
                    {
                        errlog!(libc::LOG_INFO, "port down, restarting ssh");
                        crate::kill::ssh_kill();
                        return ret(&savedmask, P_RESTART);
                    }
                }
                _ => {}
            }
        }
    }
}

// Suppress: timespec is currently unused but kept on the import list
// for future explicit-timeout work.
#[allow(dead_code)]
fn _unused() {
    let _ = std::mem::size_of::<timespec>();
}
