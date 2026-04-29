# Known issues

(no open issues — both items in this file have been resolved.
Kept as a historical record.)

## SIGCHLD race in `ssh_watch()` between `ssh_wait(WNOHANG)` and `dolongjmp=1` — RESOLVED

**Original symptom**: if the SSH child exited in the brief window
between `ssh_wait(WNOHANG)` returning P_CONTINUE and the parent
setting `dolongjmp = 1`, the kernel-delivered SIGCHLD ran sig_catch
with `dolongjmp == 0`, the handler returned without `siglongjmp`,
and the signal was "consumed". Subsequently `poll()` would block
until the next SIGALRM (default `poll_time = 600` → 10 minutes per
silent restart).

**Fix** (in `src/watch.rs`):

1. `sigprocmask(SIG_BLOCK, &blockmask, &savedmask)` at function
   entry blocks SIGCHLD/SIGALRM/SIGINT/SIGTERM/SIGHUP/SIGUSR1/USR2.
2. The race window between `ssh_wait(WNOHANG)` and the wait point
   is now signal-free; signals queue rather than fire.
3. The wait itself uses `ppoll()` (with `savedmask` as the wait-time
   sigmask) for the stderr-fd path, or `sigsuspend(&savedmask)` for
   the pause-equivalent. Both atomically unblock-and-wait so the
   queued signal is delivered the instant we begin waiting,
   sig_catch can `siglongjmp` reliably.
4. Each loop iteration re-applies SIG_BLOCK at the top (the CATCH
   branch unblocks so ssh_kill / errlog can be interrupted by a
   second termination signal — that powers the double-Ctrl+C
   force-exit).
5. Every `return` path restores the caller's mask via a small
   `ret(savedmask, rc)` helper.

**Verification**: a tight `MOCK_SSH_MODE=exit-fast-255` loop with
`AUTOSSH_MAXSTART=3` that previously hung up to 1800s now completes
in ~15ms (3 starts, default poll_time=600).

## C variadic logging shim — RESOLVED

The C variadic functions (errlog, xerrlog, doerrlog) have been
removed and replaced with Rust `errlog!`/`xerrlog!` macros backed
by non-variadic `errlog_str(level, *const c_char)` /
`xerrlog_str(level, *const c_char) -> !` in `src/log.rs`. The
macros use Rust's native `format!` and convert to a `CString`
before calling the shim, so all formatting happens in Rust.

The only remaining C is a 13-line file `c-shim/jumpbuf.c`
containing the typed `sigjmp_buf jumpbuf;` storage —
`libc::sigjmp_buf` isn't exposed by the libc 0.2 crate so the
typed instance still lives on the C side; Rust references it
via the opaque `JmpBuf` extern type.

