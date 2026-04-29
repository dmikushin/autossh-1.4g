//! Self-contained extraction of the LD_PRELOAD watchdog library.
//!
//! `ssh-stuck-detector` is built as a sibling cdylib (see
//! `ssh-stuck-detector/`). At build time, the resulting .so is
//! embedded into the autossh binary via `include_bytes!`. At
//! runtime, on the first SSH child fork, we extract it to a
//! per-process tempfile (`/tmp/autossh-stuck-detector-<pid>.so`)
//! and stash the path. ssh_run then sets LD_PRELOAD to that path
//! in the child env before execvp.
//!
//! The tempfile is overwritten on each invocation (one per parent
//! pid) and removed via atexit through unlink_pid_file's
//! companion. autossh's process lifetime is much longer than any
//! transient ssh child, so reusing the path is safe.

use libc::{c_char, c_int, mode_t};
use std::ffi::CString;
use std::sync::OnceLock;

/// The .so bytes, baked in at compile time. Built by cargo's
/// workspace member `ssh-stuck-detector`.
const SO_BYTES: &[u8] = include_bytes!(
    concat!(env!("CARGO_MANIFEST_DIR"),
        "/target/release/libssh_stuck_detector.so"));

/// Path the .so was extracted to, set on first call to `path()`.
static EXTRACTED: OnceLock<CString> = OnceLock::new();

/// Extract (idempotent) and return a NUL-terminated path to the
/// .so. Empty CString on failure.
pub fn path() -> &'static CString {
    EXTRACTED.get_or_init(extract_once)
}

fn extract_once() -> CString {
    let pid = unsafe { libc::getpid() };
    let path = format!("/tmp/autossh-stuck-detector-{}.so", pid);
    let cpath = match CString::new(path.clone()) {
        Ok(c) => c,
        Err(_) => return CString::new("").unwrap(),
    };

    unsafe {
        // O_WRONLY|O_CREAT|O_TRUNC, mode 0700.
        let fd = libc::open(
            cpath.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC,
            0o700 as mode_t as c_int,
        );
        if fd < 0 {
            return CString::new("").unwrap();
        }
        let mut written: usize = 0;
        while written < SO_BYTES.len() {
            let n = libc::write(
                fd,
                SO_BYTES.as_ptr().add(written) as *const _,
                SO_BYTES.len() - written,
            );
            if n <= 0 {
                libc::close(fd);
                return CString::new("").unwrap();
            }
            written += n as usize;
        }
        libc::close(fd);
    }

    cpath
}

/// Prepend or append `LD_PRELOAD=<path>` to the current process's
/// LD_PRELOAD env so subsequent execvp inherits it. Idempotent.
///
/// We use putenv (not setenv) because putenv lets us hand a
/// stable static buffer; the kernel keeps a pointer into it
/// across exec.
pub fn install_for_child() {
    let so = path();
    if so.as_bytes().is_empty() {
        return;
    }
    // Read current LD_PRELOAD, build new value, putenv.
    let new_env = unsafe {
        let cur = libc::getenv(c"LD_PRELOAD".as_ptr());
        if cur.is_null() || *cur == 0 {
            format!("LD_PRELOAD={}", so.to_string_lossy())
        } else {
            let cur_s = std::ffi::CStr::from_ptr(cur).to_string_lossy();
            // Avoid double-insertion if we're somehow re-entered.
            if cur_s.contains("autossh-stuck-detector") {
                return;
            }
            format!("LD_PRELOAD={}:{}", so.to_string_lossy(), cur_s)
        }
    };
    // putenv requires a leaked CString — the env keeps the pointer.
    if let Ok(cs) = CString::new(new_env) {
        let leaked = cs.into_raw();
        unsafe { libc::putenv(leaked); }
    }
}

/// Remove the extracted .so. Called from atexit so the tempfile
/// doesn't accumulate across autossh restarts.
pub fn cleanup() {
    if let Some(c) = EXTRACTED.get() {
        if !c.as_bytes().is_empty() {
            unsafe { libc::unlink(c.as_ptr() as *const c_char); }
        }
    }
}
