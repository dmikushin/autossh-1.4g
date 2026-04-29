//! Pure-Rust logging.
//!
//! Replaces the C variadic trio (errlog/xerrlog/doerrlog) with a
//! non-variadic implementation:
//!
//!   - `errlog_str(level, &CStr)` — write the message via syslog
//!     and/or stderr/file depending on globals::logtype.
//!   - `xerrlog_str(level, &CStr) -> !` — like errlog_str, then
//!     ssh_kill + unlink_pid_file + _exit(1).
//!
//! The `errlog!()` / `xerrlog!()` macros do the formatting in Rust
//! (`format!`) and feed a `CString` to the non-variadic shim.
//! Allocation per log line is fine — logging is rare.
//!
//! The C-side `errlog`/`xerrlog`/`doerrlog` symbols are also kept
//! as ABI shims for any remaining FFI callers and for the existing
//! unit-test helpers; both implementations route to errlog_str.

use libc::{c_char, c_int, FILE};
use std::ffi::CStr;

const L_FILELOG: c_int = 0x01;
const L_SYSLOG:  c_int = 0x02;

extern "C" {
    static mut logtype: c_int;
    static mut loglevel: c_int;
    static mut flog: *mut FILE;
    static __progname: *const c_char;
    static stderr: *mut FILE;
}

/// Write `msg` to all configured log sinks at `level`.
/// Non-variadic; the caller is expected to have already done any
/// printf-style substitution.
#[no_mangle]
pub unsafe extern "C" fn errlog_str(level: c_int, msg: *const c_char) {
    if level > loglevel || msg.is_null() {
        return;
    }

    if (logtype & L_SYSLOG) != 0 {
        // syslog is variadic but stable to call; %s + msg is the
        // canonical "pre-formatted" idiom.
        libc::syslog(level, c"%s".as_ptr(), msg);
    }

    let mut fl = flog;
    if (logtype & L_SYSLOG) == 0 && fl.is_null() {
        fl = stderr;
    }
    if (logtype & L_FILELOG) != 0 && !fl.is_null() {
        let progname = if __progname.is_null() {
            c"autossh".as_ptr()
        } else {
            __progname
        };
        let ts = crate::util::timestr() as *const c_char;
        // Use libc::fprintf with %s for the pre-formatted msg —
        // we only need the variadic CALL, not a definition.
        libc::fprintf(fl, c"%s %s[%d]: %s\n".as_ptr(),
            ts, progname, libc::getpid() as c_int, msg);
        libc::fflush(fl);
    }
}

/// Write `msg`, then orderly-exit: kill the SSH child, unlink pid
/// file, _exit(1). Diverging.
#[no_mangle]
pub unsafe extern "C" fn xerrlog_str(level: c_int, msg: *const c_char) -> ! {
    errlog_str(level, msg);
    crate::kill::ssh_kill();
    crate::util::unlink_pid_file();
    libc::_exit(1);
}

/// Helper used by the macros: format args into a CString, call
/// errlog_str. Hidden from doc.
#[doc(hidden)]
pub fn _log_with(level: c_int, args: std::fmt::Arguments<'_>) {
    let s = std::fmt::format(args);
    if let Ok(cs) = std::ffi::CString::new(s) {
        unsafe { errlog_str(level, cs.as_ptr()); }
    }
}

#[doc(hidden)]
pub fn _xlog_with(level: c_int, args: std::fmt::Arguments<'_>) -> ! {
    let s = std::fmt::format(args);
    let cs = std::ffi::CString::new(s)
        .unwrap_or_else(|_| std::ffi::CString::new("autossh: log formatting failed").unwrap());
    unsafe { xerrlog_str(level, cs.as_ptr()) }
}

/// `errlog!(LOG_INFO, "fmt {}", arg)` — Rust-side format, no
/// variadic FFI.
#[macro_export]
macro_rules! errlog {
    ($level:expr, $($arg:tt)*) => {
        $crate::log::_log_with($level, format_args!($($arg)*))
    };
}

/// `xerrlog!(LOG_ERR, "fmt {}", arg)` — diverging.
#[macro_export]
macro_rules! xerrlog {
    ($level:expr, $($arg:tt)*) => {
        $crate::log::_xlog_with($level, format_args!($($arg)*))
    };
}

/// Render a C string pointer (possibly NULL) to a Rust `&str` via
/// CStr; used inside macro call sites so we can put C-provided
/// `*const c_char` into Rust format args.
///
/// # Safety
/// `p` must be NUL-terminated or NULL.
pub unsafe fn cstr_or(p: *const c_char, fallback: &'static str) -> std::borrow::Cow<'static, str> {
    if p.is_null() {
        std::borrow::Cow::Borrowed(fallback)
    } else {
        match CStr::from_ptr(p).to_str() {
            Ok(s) => std::borrow::Cow::Owned(s.to_string()),
            Err(_) => std::borrow::Cow::Borrowed("<non-utf8>"),
        }
    }
}
