//! Small leaf functions: alarm-clearing wrapper and lifetime check.

use libc::{c_double, c_int, c_uint, time_t};

use crate::errlog;

extern "C" {
    static max_lifetime: c_double;
    static pid_start_time: time_t;
}

/// `clear_alarm_timer()`: cancel any pending SIGALRM and return the
/// number of seconds that were left until it would have fired.
#[no_mangle]
pub unsafe extern "C" fn clear_alarm_timer() -> c_uint {
    libc::alarm(0)
}

/// `exceeded_lifetime()`: returns 1 if `max_lifetime` is configured
/// and the time elapsed since `pid_start_time` is at least
/// `max_lifetime`; otherwise returns 0.
#[no_mangle]
pub unsafe extern "C" fn exceeded_lifetime() -> c_int {
    if max_lifetime > 0.0 {
        let mut now: time_t = 0;
        libc::time(&mut now);
        if libc::difftime(now, pid_start_time) >= max_lifetime {
            errlog!(libc::LOG_INFO,
                "exceeded maximum time to live, shutting down");
            return 1;
        }
    }
    0
}
