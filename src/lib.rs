//! autossh — port of autossh.c to Rust.
//!
//! This is the staticlib crate: every function and global tested from
//! the existing C unit-test suite is re-exported with C ABI under the
//! same name as the legacy autossh.c symbol. Tests link against
//! libautossh.a in place of autossh.o.
//!
//! As each function is moved to Rust the corresponding C definition
//! is deleted and a `#[no_mangle] pub extern "C"` definition lands
//! here. The unit test suite (linked via -Wl,--wrap=<libc syscalls>)
//! validates each step.

#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]

pub mod args;
pub mod env;
pub mod kill;
pub mod lifetime;
pub mod signals;
pub mod stderr_drain;
