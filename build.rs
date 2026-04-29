// build.rs — compile the tiny C shim. The shim is one file
// (c-shim/jumpbuf.c, ~12 lines) holding the sigjmp_buf storage
// referenced by Rust's signal handler — libc 0.2 doesn't expose
// sigjmp_buf so we keep the typed instance on the C side and
// reference it via an opaque extern type.

fn main() {
    cc::Build::new()
        .file("c-shim/jumpbuf.c")
        .flag_if_supported("-Wno-unused-parameter")
        .compile("autossh_cshim");

    println!("cargo:rerun-if-changed=c-shim/jumpbuf.c");
}

