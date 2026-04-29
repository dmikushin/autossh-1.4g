//! Restart backoff calculator.
//!
//! `grace_time(last_start)`: between consecutive ssh forks, sleeps
//! progressively longer if the SSH child has been failing fast.
//! Also enforces a fixed delay if the previous attempt reported
//! "remote port forwarding failed" (so the remote port has time to
//! be released).
//!
//! Test coverage: tests/unit/test_grace_time.c (6 cases).

use libc::{c_double, c_int, c_uint, time_t};

use crate::errlog;

const N_FAST_TRIES: c_int = 5;
const PORT_FWD_FAIL_DELAY: c_uint = 5;

extern "C" {
    static mut poll_time: c_int;
    static mut port_fwd_failed: c_int; // volatile sig_atomic_t
}

/// Replaces C's `static int tries;` inside grace_time.
static mut TRIES: c_int = 0;

#[no_mangle]
pub unsafe extern "C" fn grace_time(last_start: time_t) {
    let mut min_time: c_double = (poll_time / 10) as c_double;
    if min_time < 10.0 {
        min_time = 10.0;
    }

    let mut now: time_t = 0;
    libc::time(&raw mut now);

    if libc::difftime(now, last_start) >= min_time {
        TRIES = 0;
    } else {
        TRIES += 1;
    }

    if TRIES > N_FAST_TRIES {
        let t = (TRIES - N_FAST_TRIES) as c_double;
        // Match the original: integer truncation in the
        // multiplication, then clamp by poll_time.
        let n = ((poll_time as c_double / 100.0) * (t * (t / 3.0))) as c_int;
        let interval = if n > poll_time { poll_time } else { n };
        if interval > 0 {
            libc::sleep(interval as c_uint);
        }
    }

    if port_fwd_failed != 0 {
        errlog!(libc::LOG_INFO,
            "port forwarding failed on previous attempt, enforcing minimum {} second delay",
            PORT_FWD_FAIL_DELAY);
        libc::sleep(PORT_FWD_FAIL_DELAY);
        port_fwd_failed = 0;
    }
}
