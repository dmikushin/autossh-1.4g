// build.rs — compile the small C shim (variadic logging + sigjmp_buf
// storage) into the staticlib. The result is a single libautossh.a
// containing both Rust and C symbols, so cargo's bin target links it
// transparently and unit tests can link it in place of the old
// autossh.o.

fn main() {
    cc::Build::new()
        .file("c-shim/errlog.c")
        .flag_if_supported("-Wno-unused-parameter")
        .flag_if_supported("-Wno-unused-result")
        .compile("autossh_cshim");

    println!("cargo:rerun-if-changed=c-shim/errlog.c");
}
