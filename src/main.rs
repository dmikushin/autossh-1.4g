//! autossh-rs binary entrypoint.
//!
//! In Phase 0 this is a stub. The production `autossh` binary is
//! still built from autossh.c. As functions are ported to Rust, the
//! C `main()` will progressively delegate to Rust functions, and in
//! the final phase the C file is dropped and this main becomes the
//! sole entrypoint.

fn main() {
    // Phase 0 placeholder. The real entrypoint is autossh.c::main
    // until the port reaches Phase 6.
    std::process::exit(0);
}
