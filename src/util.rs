//! Utility leaf functions: usage(), unlink_pid_file(), timestr().

use libc::{c_char, c_int, time_t};
use std::ffi::CStr;
use std::ptr;

extern "C" {
    static mut pid_file_created: c_int;
    static mut pid_file_name: *mut c_char;
    static __progname: *const c_char;
}

/// Print usage and exit. `code != 0` includes the env-var section
/// and routes to stderr; `code == 0` writes the short form to
/// stdout and exits 0.
#[no_mangle]
pub unsafe extern "C" fn usage(code: c_int) -> ! {
    let progname = if __progname.is_null() {
        "autossh".to_string()
    } else {
        CStr::from_ptr(__progname).to_string_lossy().into_owned()
    };
    let stream = if code != 0 { "stderr" } else { "stdout" };
    let _ = stream; // logical clarification only

    let header = format!(
        "usage: {} [-V] [-M monitor_port[:echo_port]] [-f] [SSH_OPTIONS]\n",
        progname,
    );
    if code != 0 {
        eprint!("{}", header);
        eprintln!();
        eprintln!(
            "    -M specifies monitor port. May be overridden by environment\n\
             \x20      variable AUTOSSH_PORT. 0 turns monitoring loop off.\n\
             \x20      Alternatively, a port for an echo service on the remote\n\
             \x20      machine may be specified. (Normally port 7.)"
        );
        eprintln!(
            "    -f run in background (autossh handles this, and does not\n\
             \x20      pass it to ssh.)"
        );
        eprintln!("    -V print autossh version and exit.");
        eprintln!();
        eprintln!("Environment variables are:");
        eprintln!(
            "    AUTOSSH_GATETIME    - how long must an ssh session be established\n\
             \x20                       before we decide it really was established\n\
             \x20                       (in seconds). Default is 30 seconds; use of -f\n\
             \x20                       flag sets this to 0."
        );
        eprintln!(
            "    AUTOSSH_LOGFILE     - file to log to (default is to use the syslog\n\
             \x20                       facility)"
        );
        eprintln!("    AUTOSSH_LOGLEVEL    - level of log verbosity");
        eprintln!("    AUTOSSH_MAXLIFETIME - set the maximum time to live (seconds)");
        eprintln!("    AUTOSSH_MAXSTART    - max times to restart (default is no limit)");
        eprintln!(
            "    AUTOSSH_MAX_SESSION - max seconds SSH may be silent on stderr (or stuck\n\
             \x20                       after stderr pipe loss) before being considered\n\
             \x20                       stuck and killed. 0 = no watchdog (default).\n\
             \x20                       Recommended with -M 0."
        );
        eprintln!(
            "    AUTOSSH_MESSAGE     - message to append to echo string (max 64 bytes)"
        );
        eprintln!("    AUTOSSH_PATH        - path to ssh if not default");
        eprintln!("    AUTOSSH_PIDFILE     - write pid to this file");
        eprintln!("    AUTOSSH_POLL        - how often to check the connection (seconds)");
        eprintln!("    AUTOSSH_FIRST_POLL  - time before first connection check (seconds)");
        eprintln!("    AUTOSSH_PORT        - port to use for monitor connection");
        eprintln!(
            "    AUTOSSH_DEBUG       - turn logging to maximum verbosity and log to\n\
             \x20                       stderr"
        );
        eprintln!();
    } else {
        print!("{}", header);
    }
    libc::exit(code);
}

/// `atexit`-registered cleanup: remove the pid file we created.
/// Becomes a no-op if pid_file_created is 0.
#[no_mangle]
pub unsafe extern "C" fn unlink_pid_file() {
    if pid_file_created != 0 && !pid_file_name.is_null() {
        libc::unlink(pid_file_name);
    }
    pid_file_created = 0;
}

/// Return a pointer to a static buffer with the current local time
/// formatted "YYYY/MM/DD HH:MM:SS". The buffer is reused on each
/// call (matches C's static-buffer return).
#[no_mangle]
pub unsafe extern "C" fn timestr() -> *mut c_char {
    const BUF_SZ: usize = 32;
    static mut BUF: [c_char; BUF_SZ] = [0; BUF_SZ];

    let mut now: time_t = 0;
    libc::time(&raw mut now);
    let tm = libc::localtime(&now);
    if tm.is_null() {
        BUF[0] = 0;
        return (&raw mut BUF) as *mut c_char;
    }
    let fmt = c"%Y/%m/%d %H:%M:%S".as_ptr();
    libc::strftime(
        (&raw mut BUF) as *mut c_char,
        BUF_SZ,
        fmt,
        tm,
    );
    (&raw mut BUF) as *mut c_char
}

// Suppress dead-code on `ptr` import if unused.
#[allow(dead_code)]
fn _unused() {
    let _ = ptr::null::<c_int>();
}
