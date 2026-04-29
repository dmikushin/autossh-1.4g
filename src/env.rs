//! Environment-variable parsing.
//!
//! Mirrors C's `get_env_args(void)`. Test coverage:
//! tests/unit/test_get_env_args.c (18 cases including every
//! invalid-input path, which exits via xerrlog!).

use libc::{c_char, c_double, c_int, c_long, time_t, FILE};
use std::ptr;

use crate::log::cstr_or;
use crate::{errlog, xerrlog};

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
}

const MAX_MESSAGE: usize = 64;
const POLL_TIME_DEFAULT: c_int = 600;
const HAVE_LOG_PERROR_LOG_PERROR: c_int = libc::LOG_PERROR;

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

unsafe fn is_empty_cstr(s: *const c_char) -> bool {
    !s.is_null() && *s == 0
}

#[no_mangle]
pub unsafe extern "C" fn get_env_args() {
    let s = libc::getenv(c"AUTOSSH_PATH".as_ptr());
    if !s.is_null() {
        ssh_path = s;
    }

    let s = libc::getenv(c"AUTOSSH_DEBUG".as_ptr());
    if !s.is_null() {
        syslog_perror = HAVE_LOG_PERROR_LOG_PERROR;
        loglevel = libc::LOG_DEBUG;
    } else {
        let s = libc::getenv(c"AUTOSSH_LOGLEVEL".as_ptr());
        if !s.is_null() {
            let (v, t) = parse_l(s);
            if is_empty_cstr(s) || (!t.is_null() && *t != 0)
                || v < libc::LOG_EMERG as c_long
                || v > libc::LOG_DEBUG as c_long
            {
                xerrlog!(libc::LOG_ERR, "invalid log level \"{}\"", cstr_or(s, ""));
            }
            loglevel = v as c_int;
        }
    }

    let s = libc::getenv(c"AUTOSSH_POLL".as_ptr());
    if !s.is_null() {
        let (v, t) = parse_ul(s);
        if is_empty_cstr(s) || v == 0 || (!t.is_null() && *t != 0) {
            xerrlog!(libc::LOG_ERR, "invalid poll time \"{}\"", cstr_or(s, ""));
        }
        poll_time = v as c_int;
        if poll_time <= 0 {
            poll_time = POLL_TIME_DEFAULT;
        }
    }

    let s = libc::getenv(c"AUTOSSH_FIRST_POLL".as_ptr());
    if !s.is_null() {
        let (v, t) = parse_ul(s);
        if is_empty_cstr(s) || v == 0 || (!t.is_null() && *t != 0) {
            xerrlog!(libc::LOG_ERR, "invalid first poll time \"{}\"", cstr_or(s, ""));
        }
        first_poll_time = v as c_int;
        if first_poll_time <= 0 {
            first_poll_time = POLL_TIME_DEFAULT;
        }
    } else {
        first_poll_time = poll_time;
    }

    let s = libc::getenv(c"AUTOSSH_GATETIME".as_ptr());
    if !s.is_null() {
        let (v, t) = parse_l(s);
        if is_empty_cstr(s) || (v as c_double) < 0.0
            || (!t.is_null() && *t != 0)
        {
            xerrlog!(libc::LOG_ERR, "invalid gate time \"{}\"", cstr_or(s, ""));
        }
        gate_time = v as c_double;
    }

    let s = libc::getenv(c"AUTOSSH_MAXSTART".as_ptr());
    if !s.is_null() {
        let (v, t) = parse_l(s);
        if is_empty_cstr(s) || v < -1 || (!t.is_null() && *t != 0) {
            xerrlog!(libc::LOG_ERR, "invalid max start number \"{}\"", cstr_or(s, ""));
        }
        max_start = v as c_int;
    }

    let s = libc::getenv(c"AUTOSSH_MESSAGE".as_ptr());
    if !s.is_null() {
        if *s != 0 {
            echo_message = s;
        }
        if libc::strlen(echo_message) > MAX_MESSAGE {
            xerrlog!(libc::LOG_ERR,
                "echo message may only be {} bytes long", MAX_MESSAGE);
        }
    }

    let s = libc::getenv(c"AUTOSSH_PORT".as_ptr());
    if !s.is_null() && *s != 0 {
        env_port = s;
    }

    let s = libc::getenv(c"AUTOSSH_MAXLIFETIME".as_ptr());
    if !s.is_null() {
        let (v, t) = parse_ul(s);
        if is_empty_cstr(s) || (!t.is_null() && *t != 0) {
            xerrlog!(libc::LOG_ERR, "invalid max lifetime \"{}\"", cstr_or(s, ""));
        }
        max_lifetime = v as c_double;
        if max_lifetime <= 0.0 {
            max_lifetime = 0.0;
        } else {
            if (poll_time as c_double) > max_lifetime {
                errlog!(libc::LOG_INFO,
                    "poll time is greater than lifetime, dropping poll time to {:.0}",
                    max_lifetime);
                poll_time = max_lifetime as c_int;
            }
            if (first_poll_time as c_double) > max_lifetime {
                errlog!(libc::LOG_INFO,
                    "first poll time is greater than lifetime, dropping first poll time to {:.0}",
                    max_lifetime);
                first_poll_time = max_lifetime as c_int;
            }
            libc::time(&raw mut pid_start_time);
        }
    }

    let s = libc::getenv(c"AUTOSSH_MAX_SESSION".as_ptr());
    if !s.is_null() {
        let (v, t) = parse_ul(s);
        if is_empty_cstr(s) || (!t.is_null() && *t != 0) {
            xerrlog!(libc::LOG_ERR, "invalid max session time \"{}\"", cstr_or(s, ""));
        }
        max_session = v as c_double;
    }

    let s = libc::getenv(c"AUTOSSH_PIDFILE".as_ptr());
    if !s.is_null() && *s != 0 {
        pid_file_name = s;
    }

    let s = libc::getenv(c"AUTOSSH_LOGFILE".as_ptr());
    if !s.is_null() {
        let mode = c"a".as_ptr();
        let f = libc::fopen(s, mode);
        if f.is_null() {
            let path = cstr_or(s, "");
            let err = cstr_or(libc::strerror(*libc::__errno_location()), "?");
            xerrlog!(libc::LOG_ERR, "{}: {}", path, err);
        }
        flog = f;
        logtype = L_FILELOG;
    }
}

const L_FILELOG: c_int = 0x01;
