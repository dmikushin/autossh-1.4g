//! SSH stderr drain + fatal-pattern detection.
//!
//! `check_ssh_stderr()` reads all available bytes from the SSH child's
//! stderr pipe (non-blocking), forwards them to our own stderr so the
//! user keeps seeing SSH messages, and updates `last_stderr_time` —
//! the timestamp the SIGALRM watchdog uses to detect stuck children.
//!
//! Returns 1 if the data contains "remote port forwarding failed",
//! a known fatal SSH error that warrants an immediate kill+restart;
//! returns 0 otherwise (or if there is no stderr fd to read).
//!
//! Test coverage: tests/unit/test_check_ssh_stderr.c (8 cases).

use libc::{c_char, c_int, c_void, size_t, ssize_t, time_t};

const STDERR_BUF_SZ: usize = 4096;

extern "C" {
    static mut ssh_stderr_fd: c_int;
    static mut last_stderr_time: time_t;
    static mut port_fwd_failed: c_int; // volatile sig_atomic_t
    fn errlog(level: c_int, fmt: *const c_char, ...);
}

/// Drain the SSH stderr pipe; forward to our stderr; flag fatal
/// patterns. Returns 1 if a known-fatal SSH error was seen, 0
/// otherwise.
#[no_mangle]
pub unsafe extern "C" fn check_ssh_stderr() -> c_int {
    if ssh_stderr_fd < 0 {
        return 0;
    }

    let mut buf = [0u8; STDERR_BUF_SZ];
    loop {
        let n: ssize_t = libc::read(
            ssh_stderr_fd,
            buf.as_mut_ptr() as *mut c_void,
            (STDERR_BUF_SZ - 1) as size_t,
        );
        if n <= 0 {
            // EAGAIN, EOF, or error — stop reading without touching
            // last_stderr_time (matches C semantics).
            return 0;
        }

        // NUL-terminate for strstr.
        buf[n as usize] = 0;
        libc::time(&raw mut last_stderr_time);

        // Forward to our own stderr (best-effort — ignore write
        // errors, matching the C original).
        let _ = libc::write(libc::STDERR_FILENO, buf.as_ptr() as *const c_void,
                            n as size_t);

        // Look for the known-fatal pattern.
        let needle = c"remote port forwarding failed".as_ptr();
        let found = libc::strstr(buf.as_ptr() as *const c_char, needle);
        if !found.is_null() {
            errlog(
                libc::LOG_ERR,
                c"detected SSH error: remote port forwarding failed; will kill and restart ssh".as_ptr(),
            );
            port_fwd_failed = 1;
            return 1;
        }
    }
}
