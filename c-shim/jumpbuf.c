/*
 *	autossh — sigjmp_buf storage.
 *
 *	The libc 0.2 crate doesn't expose sigjmp_buf or sigsetjmp/
 *	siglongjmp on Linux, so this single 200-byte typed slot
 *	lives in C and Rust references it through the opaque JmpBuf
 *	type defined in src/signals.rs. That's the only reason this
 *	file still exists; everything else is in src/*.rs.
 */

#include <setjmp.h>

sigjmp_buf jumpbuf;
