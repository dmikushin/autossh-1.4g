use std::ffi::CString;
use std::os::unix::ffi::OsStringExt;
use std::ptr;

fn main() {
    // Convert OsString args to CString. Anything containing an
    // interior NUL is a hard error — argv can't carry NULs anyway.
    let owned: Vec<CString> = std::env::args_os()
        .map(|os| {
            CString::new(os.into_vec())
                .expect("argv element contains an interior NUL byte")
        })
        .collect();

    // Build a NULL-terminated *mut c_char array. We collect into a
    // Vec so the storage lives until the process exits; we then
    // leak it because autossh_main may save pointers into newav
    // (consumed via execvp later) and never gives them back.
    let mut argv_ptrs: Vec<*mut libc::c_char> =
        owned.iter().map(|c| c.as_ptr() as *mut libc::c_char).collect();
    argv_ptrs.push(ptr::null_mut());

    let argc = (argv_ptrs.len() - 1) as libc::c_int;
    let argv_box = argv_ptrs.into_boxed_slice();
    let argv_ptr = Box::leak(argv_box).as_mut_ptr();
    std::mem::forget(owned);

    let rc = unsafe { autossh::main_logic::autossh_main(argc, argv_ptr) };
    std::process::exit(rc);
}
