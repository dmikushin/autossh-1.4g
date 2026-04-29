//! All process-wide global state.
//!
//! These were scattered across autossh.c at file scope; the port
//! consolidates them here so that as functions move to Rust we can
//! eventually drop autossh.c entirely.
//!
//! Each is exported `#[no_mangle] pub static mut <name>` to preserve
//! the C ABI symbol used by:
//!   - the unit-test suite (extern declarations in tests/unit/)
//!   - the integration build (linked with autossh.o + libautossh.a)
//!   - autossh.c (still references some until its final removal)

use libc::{c_char, c_double, c_int, time_t, FILE, LOG_INFO};

// Defaults from autossh.c #defines.
const POLL_TIME_DEFAULT:    c_int    = 600;
const GATE_TIME_DEFAULT:    c_double = 30.0;
const MAX_START_DEFAULT:    c_int    = -1;
const MAX_LIFETIME_DEFAULT: c_double = 0.0;
const MAX_SESSION_DEFAULT:  c_double = 0.0;
const TIMEO_NET_DEFAULT:    c_int    = 15000;
const L_SYSLOG:             c_int    = 0x02;

// String defaults — must be allocated as [u8; N] so .as_ptr() is
// const-callable; cast to *mut c_char at use sites.
static MHOST_BUF:        [u8; 10] = *b"127.0.0.1\0";
static EMPTY_BUF:        [u8;  1] = *b"\0";
// SSH_PATH is set by configure; matches autossh.c's compile-time -DSSH_PATH=...
// We default to /usr/bin/ssh; main() will allow AUTOSSH_PATH to override.
static SSH_PATH_BUF:     [u8; 13] = *b"/usr/bin/ssh\0";

// ---- log state ----
#[no_mangle] pub static mut logtype:        c_int            = L_SYSLOG;
#[no_mangle] pub static mut loglevel:       c_int            = LOG_INFO;
#[no_mangle] pub static mut syslog_perror:  c_int            = 0;
#[no_mangle] pub static mut flog:           *mut FILE        = std::ptr::null_mut();

// ---- ssh ports + hosts ----
#[no_mangle] pub static mut writep:         *mut c_char      = std::ptr::null_mut();
#[no_mangle] pub static mut readp:          [c_char; 16]     = [0; 16];
#[no_mangle] pub static mut echop:          *mut c_char      = std::ptr::null_mut();
#[no_mangle] pub static mut mhost:          *mut c_char      = MHOST_BUF.as_ptr() as *mut c_char;
#[no_mangle] pub static mut env_port:       *mut c_char      = std::ptr::null_mut();
#[no_mangle] pub static mut echo_message:   *mut c_char      = EMPTY_BUF.as_ptr() as *mut c_char;

// ---- pid file ----
#[no_mangle] pub static mut pid_file_name:    *mut c_char    = std::ptr::null_mut();
#[no_mangle] pub static mut pid_file_created: c_int          = 0;
#[no_mangle] pub static mut pid_start_time:   time_t         = 0;

// ---- poll/gate/lifetime config ----
#[no_mangle] pub static mut poll_time:        c_int          = POLL_TIME_DEFAULT;
#[no_mangle] pub static mut first_poll_time:  c_int          = POLL_TIME_DEFAULT;
#[no_mangle] pub static mut gate_time:        c_double       = GATE_TIME_DEFAULT;
#[no_mangle] pub static mut max_start:        c_int          = MAX_START_DEFAULT;
#[no_mangle] pub static mut max_lifetime:     c_double       = MAX_LIFETIME_DEFAULT;
#[no_mangle] pub static mut max_session:      c_double       = MAX_SESSION_DEFAULT;
#[no_mangle] pub static mut net_timeout:      c_int          = TIMEO_NET_DEFAULT;
#[no_mangle] pub static mut ssh_path:         *mut c_char    = SSH_PATH_BUF.as_ptr() as *mut c_char;

#[no_mangle] pub static mut start_count:      c_int          = 0;
#[no_mangle] pub static mut start_time:       time_t         = 0;

// ---- ssh child + monitor state ----
#[no_mangle] pub static mut cchild:           c_int          = 0;
#[no_mangle] pub static mut ssh_stderr_fd:    c_int          = -1;
#[no_mangle] pub static mut pipe_lost_time:   time_t         = 0;
#[no_mangle] pub static mut last_stderr_time: time_t         = 0;

// ---- volatile sig_atomic_t flags (writable from sig_catch) ----
#[no_mangle] pub static mut port_fwd_failed: c_int = 0;
#[no_mangle] pub static mut exit_signalled:  c_int = 0;
#[no_mangle] pub static mut restart_ssh:     c_int = 0;
#[no_mangle] pub static mut dolongjmp:       c_int = 0;
