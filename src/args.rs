//! Argument-list builder used to assemble the ssh command line.
//!
//! Mirrors C's:
//!     int      newac;
//!     char   **newav;
//!     void     add_arg(char *s);
//!
//! `newav` is a malloc'd, NULL-terminated array of malloc'd C strings,
//! so that ssh_run can hand it to execvp without conversion. Storage
//! grows by doubling, starting at START_AV_SZ.
//!
//! Globals are intentionally exported `static mut` with C names: the
//! existing unit-test suite (tests/unit/test_add_arg.c) reads them
//! directly via `extern int newac; extern char **newav;`.

use libc::{c_char, c_int, calloc, malloc, realloc};
use std::ptr;

const START_AV_SZ: usize = 16;

/// Capacity of `newav`, in pointer slots. Persists across add_arg
/// calls; grows by doubling. C had this as a function-static.
static mut NEWAV_CAP: usize = START_AV_SZ;

#[no_mangle]
pub static mut newac: c_int = 0;

#[no_mangle]
pub static mut newav: *mut *mut c_char = ptr::null_mut();

/// Diagnostic + abort, matching xerrlog(LOG_ERR, ...) followed by
/// `_exit(1)` semantics in the C original. The C code uses xerrlog
/// to log via syslog/file before exiting; for the rare malloc-failure
/// paths we simplify to a stderr write + exit(1) so we don't have to
/// thread the format-string arguments through FFI variadics.
unsafe fn alloc_fail(what: &str) -> ! {
    let errno = *libc::__errno_location();
    let msg = libc::strerror(errno);
    let s = if msg.is_null() {
        "<unknown>".to_string()
    } else {
        std::ffi::CStr::from_ptr(msg).to_string_lossy().into_owned()
    };
    eprintln!("autossh: {}: {}", what, s);
    libc::_exit(1);
}

/// `add_arg(s)`: append a copy of `s` to `newav`, NULL-terminating
/// the array and growing storage on demand. Empty strings are
/// silently ignored (matches C behavior).
///
/// # Safety
/// `s` must be a NUL-terminated C string. After return, `newav` may
/// point to new storage. Concurrent access from another thread is
/// undefined — autossh is single-threaded.
#[no_mangle]
pub unsafe extern "C" fn add_arg(s: *const c_char) {
    if s.is_null() {
        return;
    }
    let len = libc::strlen(s);
    if len == 0 {
        return;
    }

    if newav.is_null() {
        newav = calloc(START_AV_SZ, std::mem::size_of::<*mut c_char>())
            as *mut *mut c_char;
        if newav.is_null() {
            alloc_fail("malloc");
        }
        NEWAV_CAP = START_AV_SZ;
    } else if (newac as usize) >= NEWAV_CAP - 1 {
        let new_cap = NEWAV_CAP * 2;
        let new_ptr = realloc(
            newav as *mut libc::c_void,
            new_cap * std::mem::size_of::<*mut c_char>(),
        ) as *mut *mut c_char;
        if new_ptr.is_null() {
            alloc_fail("realloc");
        }
        newav = new_ptr;
        NEWAV_CAP = new_cap;
    }

    let p = malloc(len + 1) as *mut c_char;
    if p.is_null() {
        alloc_fail("malloc");
    }
    libc::memmove(p as *mut _, s as *const _, len);
    *p.add(len) = 0;

    *newav.add(newac as usize) = p;
    newac += 1;
    *newav.add(newac as usize) = ptr::null_mut();
}
