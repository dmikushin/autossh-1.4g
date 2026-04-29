//! Environment-variable parsing.
//!
//! Mirrors C's `get_env_args(void)` — reads AUTOSSH_* variables and
//! mutates the global config. All globals it writes still live in
//! autossh.c (declared `extern static mut` here); we just orchestrate.
//!
//! For invalid input, the C version calls xerrlog(LOG_ERR, …) which
//! does its own log + ssh_kill + _exit(1). We invoke xerrlog via FFI
//! using Rust's stable C-variadic call support.
//!
//! Test coverage: tests/unit/test_get_env_args.c (18 cases including
//! every invalid-input path).

use libc::{c_char, c_double, c_int, c_long, c_void, time_t, FILE};
use std::ptr;

extern "C" {
    static mut ssh_path: *mut c_char;
    static mut syslog_perror: c_int;
    static mut logtype: c_int;
    static mut flog: *mut FILE;
    static mut loglevel: c_int;
    static mut poll_time: c_int;
    static mut first_poll_time: c_int;
    static mut gate_time: c_double;
    static mut max_start: c_int;
    static mut echo_message: *mut c_char;
    static mut env_port: *mut c_char;
    static mut max_lifetime: c_double;
    static mut pid_start_time: time_t;
    static mut max_session: c_double;
    static mut pid_file_name: *mut c_char;

    fn xerrlog(level: c_int, fmt: *const c_char, ...);
    fn errlog(level: c_int, fmt: *const c_char, ...);
}

const MAX_MESSAGE: usize = 64;
const POLL_TIME_DEFAULT: c_int = 600;
const HAVE_LOG_PERROR_LOG_PERROR: c_int = libc::LOG_PERROR;

/// Equivalent to C's `strtoul(s, &endptr, 0)` returning value + endptr.
unsafe fn parse_ul(s: *const c_char) -> (libc::c_ulong, *mut c_char) {
    let mut end: *mut c_char = ptr::null_mut();
    let v = libc::strtoul(s, &mut end as *mut *mut c_char, 0);
    (v, end)
}

unsafe fn parse_l(s: *const c_char) -> (c_long, *mut c_char) {
    let mut end: *mut c_char = ptr::null_mut();
    let v = libc::strtol(s, &mut end as *mut *mut c_char, 0);
    (v, end)
}

/// Returns true iff `s` is non-NULL and points to an empty C string.
unsafe fn is_empty_cstr(s: *const c_char) -> bool {
    !s.is_null() && *s == 0
}

/// `get_env_args()`: read AUTOSSH_* env vars and mutate globals.
#[no_mangle]
pub unsafe extern "C" fn get_env_args() {
    // AUTOSSH_PATH overrides the default ssh path.
    let s = libc::getenv(c"AUTOSSH_PATH".as_ptr());
    if !s.is_null() {
        ssh_path = s;
    }

    // AUTOSSH_DEBUG: enables LOG_DEBUG. Linux glibc has LOG_PERROR.
    let s = libc::getenv(c"AUTOSSH_DEBUG".as_ptr());
    if !s.is_null() {
        syslog_perror = HAVE_LOG_PERROR_LOG_PERROR;
        loglevel = libc::LOG_DEBUG;
    } else {
        // AUTOSSH_LOGLEVEL: integer in [LOG_EMERG..LOG_DEBUG].
        let s = libc::getenv(c"AUTOSSH_LOGLEVEL".as_ptr());
        if !s.is_null() {
            let (v, t) = parse_l(s);
            if is_empty_cstr(s) || (!t.is_null() && *t != 0)
                || v < libc::LOG_EMERG as c_long
                || v > libc::LOG_DEBUG as c_long
            {
                xerrlog(libc::LOG_ERR,
                    c"invalid log level \"%s\"".as_ptr(), s);
            }
            loglevel = v as c_int;
        }
    }

    // AUTOSSH_POLL: positive integer; 0 invalid; non-numeric invalid.
    let s = libc::getenv(c"AUTOSSH_POLL".as_ptr());
    if !s.is_null() {
        let (v, t) = parse_ul(s);
        if is_empty_cstr(s) || v == 0 || (!t.is_null() && *t != 0) {
            xerrlog(libc::LOG_ERR,
                c"invalid poll time \"%s\"".as_ptr(), s);
        }
        poll_time = v as c_int;
        if poll_time <= 0 {
            poll_time = POLL_TIME_DEFAULT;
        }
    }

    // AUTOSSH_FIRST_POLL: same rules as POLL; falls back to poll_time.
    let s = libc::getenv(c"AUTOSSH_FIRST_POLL".as_ptr());
    if !s.is_null() {
        let (v, t) = parse_ul(s);
        if is_empty_cstr(s) || v == 0 || (!t.is_null() && *t != 0) {
            xerrlog(libc::LOG_ERR,
                c"invalid first poll time \"%s\"".as_ptr(), s);
        }
        first_poll_time = v as c_int;
        if first_poll_time <= 0 {
            first_poll_time = POLL_TIME_DEFAULT;
        }
    } else {
        first_poll_time = poll_time;
    }

    // AUTOSSH_GATETIME: non-negative double.
    let s = libc::getenv(c"AUTOSSH_GATETIME".as_ptr());
    if !s.is_null() {
        let (v, t) = parse_l(s);
        if is_empty_cstr(s) || (v as c_double) < 0.0
            || (!t.is_null() && *t != 0)
        {
            xerrlog(libc::LOG_ERR,
                c"invalid gate time \"%s\"".as_ptr(), s);
        }
        gate_time = v as c_double;
    }

    // AUTOSSH_MAXSTART: integer >= -1.
    let s = libc::getenv(c"AUTOSSH_MAXSTART".as_ptr());
    if !s.is_null() {
        let (v, t) = parse_l(s);
        if is_empty_cstr(s) || v < -1 || (!t.is_null() && *t != 0) {
            xerrlog(libc::LOG_ERR,
                c"invalid max start number \"%s\"".as_ptr(), s);
        }
        max_start = v as c_int;
    }

    // AUTOSSH_MESSAGE: non-empty string up to MAX_MESSAGE bytes.
    let s = libc::getenv(c"AUTOSSH_MESSAGE".as_ptr());
    if !s.is_null() {
        if *s != 0 {
            echo_message = s;
        }
        if libc::strlen(echo_message) > MAX_MESSAGE {
            xerrlog(libc::LOG_ERR,
                c"echo message may only be %d bytes long".as_ptr(),
                MAX_MESSAGE as c_int);
        }
    }

    // AUTOSSH_PORT: non-empty string. Empty string ignored (does not
    // assign env_port).
    let s = libc::getenv(c"AUTOSSH_PORT".as_ptr());
    if !s.is_null() && *s != 0 {
        env_port = s;
    }

    // AUTOSSH_MAXLIFETIME: non-negative double; clamps poll times
    // when set and seeds pid_start_time.
    let s = libc::getenv(c"AUTOSSH_MAXLIFETIME".as_ptr());
    if !s.is_null() {
        let (v, t) = parse_ul(s);
        if is_empty_cstr(s) || (!t.is_null() && *t != 0) {
            xerrlog(libc::LOG_ERR,
                c"invalid max lifetime \"%s\"".as_ptr(), s);
        }
        max_lifetime = v as c_double;
        if max_lifetime <= 0.0 {
            max_lifetime = 0.0;
        } else {
            if (poll_time as c_double) > max_lifetime {
                errlog(libc::LOG_INFO,
                    c"poll time is greater than lifetime, dropping poll time to %.0f".as_ptr(),
                    max_lifetime);
                poll_time = max_lifetime as c_int;
            }
            if (first_poll_time as c_double) > max_lifetime {
                errlog(libc::LOG_INFO,
                    c"first poll time is greater than lifetime, dropping first poll time to %.0f".as_ptr(),
                    max_lifetime);
                first_poll_time = max_lifetime as c_int;
            }
            libc::time(&raw mut pid_start_time);
        }
    }

    // AUTOSSH_MAX_SESSION: non-negative double (stderr-silence
    // watchdog threshold).
    let s = libc::getenv(c"AUTOSSH_MAX_SESSION".as_ptr());
    if !s.is_null() {
        let (v, t) = parse_ul(s);
        if is_empty_cstr(s) || (!t.is_null() && *t != 0) {
            xerrlog(libc::LOG_ERR,
                c"invalid max session time \"%s\"".as_ptr(), s);
        }
        max_session = v as c_double;
    }

    // AUTOSSH_PIDFILE: non-empty path.
    let s = libc::getenv(c"AUTOSSH_PIDFILE".as_ptr());
    if !s.is_null() && *s != 0 {
        pid_file_name = s;
    }

    // AUTOSSH_LOGFILE: route logs to file. Looked up after the
    // (cygwin-only) NTSERVICE handling so a service may override.
    let s = libc::getenv(c"AUTOSSH_LOGFILE".as_ptr());
    if !s.is_null() {
        let mode = c"a".as_ptr();
        let f = libc::fopen(s, mode);
        if f.is_null() {
            xerrlog(libc::LOG_ERR,
                c"%s: %s".as_ptr(), s,
                libc::strerror(*libc::__errno_location()));
        }
        flog = f;
        logtype = L_FILELOG;
    }

    // Suppress "unused": referenced via *mut c_void in inline
    // computations above.
    let _ = ptr::null::<c_void>;
}

/// L_FILELOG bit (matches autossh.c). Kept private here.
const L_FILELOG: c_int = 0x01;
