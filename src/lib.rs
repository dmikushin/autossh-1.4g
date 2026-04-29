//! autossh — Rust port of the autossh.c monitor.
//!
//! Every business decision lives in this crate. The two-file
//! `c-shim/` directory holds the minimal C plumbing that stable
//! Rust cannot express today:
//!   - errlog/xerrlog/doerrlog: C-variadic logging entry points.
//!   - jumpbuf: sigjmp_buf storage referenced by sig_catch via an
//!     opaque extern type.
//!
//! cargo's bin target (src/main.rs) is the entrypoint. build.rs
//! compiles c-shim/errlog.c via the cc crate and links it into
//! both the bin and the staticlib.

#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]

pub mod args;
pub mod conn;
pub mod env;
pub mod globals;
pub mod grace;
pub mod kill;
pub mod lifetime;
pub mod log;
pub mod main_logic;
pub mod run;
pub mod signals;
pub mod stderr_drain;
pub mod util;
pub mod wait;
pub mod watch;
