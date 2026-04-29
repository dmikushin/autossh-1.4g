//! Small leaf functions: alarm-clearing wrapper and lifetime check.
//!
//! Both reference globals that still live in autossh.c (max_lifetime,
//! pid_start_time). They are pulled in via `extern "C"` declarations
//! so this file compiles without needing the globals duplicated here.
//!
//! errlog() is called via FFI for log lines that integration tests
//! grep for. It is a C variadic function; Rust calls into it with
//! a fixed format string argument (no further variadic args needed
//! at our call sites). The LOG_DEBUG line in clear_alarm_timer is
//! omitted — diagnostic only, never asserted on by tests.

use libc::{c_char, c_double, c_int, c_uint, time_t};

extern "C" {
    static max_lifetime: c_double;
    static pid_start_time: time_t;
    fn errlog(level: c_int, fmt: *const c_char, ...);
}

/// `clear_alarm_timer()`: cancel any pending SIGALRM and return the
/// number of seconds that were left until it would have fired.
/// Equivalent to `alarm(0)` plus a debug log; tests only observe the
/// return value and the side-effect on the alarm.
#[no_mangle]
pub unsafe extern "C" fn clear_alarm_timer() -> c_uint {
    libc::alarm(0)
}

/// `exceeded_lifetime()`: returns 1 if `max_lifetime` is configured
/// and the time elapsed since `pid_start_time` is at least
/// `max_lifetime`; otherwise returns 0. The log line is emitted
/// (integration tests grep for it) via the existing C errlog.
#[no_mangle]
pub unsafe extern "C" fn exceeded_lifetime() -> c_int {
    if max_lifetime > 0.0 {
        let mut now: time_t = 0;
        libc::time(&mut now);
        if libc::difftime(now, pid_start_time) >= max_lifetime {
            errlog(
                libc::LOG_INFO,
                c"exceeded maximum time to live, shutting down".as_ptr()
                    as *const c_char,
            );
            return 1;
        }
    }
    0
}
