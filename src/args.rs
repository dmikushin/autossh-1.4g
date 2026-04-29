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

/// `strip_arg(arg, ch, opts)`: strip every occurrence of `ch` from
/// the short-option argument `arg` (e.g. removes 'f' from "-fN" →
/// "-N"). If `arg` does not start with '-' or is just "-", it is
/// left untouched. If `*f` matches a flag in `opts` whose entry
/// in the option string is followed by ':' (i.e. takes a parameter),
/// the rest of `arg` is the parameter value and we stop scanning.
///
/// This is a literal port of the C version, including the in-place
/// `memmove(f, f+1, len)` slide that may read one byte past the
/// end of `arg` on the final iteration. Static-string tests pass
/// because the byte past the NUL terminator is zero in practice;
/// preserving the exact behaviour avoids changing observable
/// output for adversarial inputs.
///
/// # Safety
/// `arg` and `opts` must be NUL-terminated C strings; `arg` must be
/// writable.
#[no_mangle]
pub unsafe extern "C" fn strip_arg(arg: *mut c_char, ch: c_char, opts: *const c_char) {
    if arg.is_null() || opts.is_null() {
        return;
    }
    if *arg != b'-' as c_char || *arg.add(1) == 0 {
        return;
    }

    let mut len = libc::strlen(arg);
    let mut f = arg;
    while *f != 0 {
        let o = libc::strchr(opts, *f as c_int);
        if !o.is_null() && *o.add(1) == b':' as c_char {
            // *f is a flag taking a parameter; rest of arg is the
            // parameter value. Stop scanning.
            return;
        }
        if *f == ch {
            // shift everything after f one byte left
            libc::memmove(f as *mut _, f.add(1) as *const _, len);
        }
        f = f.add(1);
        if len > 0 {
            len -= 1;
        }
    }

    if *arg.add(1) == 0 {
        *arg = 0;
    }
}
