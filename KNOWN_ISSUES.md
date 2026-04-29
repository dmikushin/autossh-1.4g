# Known issues

## SIGCHLD race in `ssh_watch()` between `ssh_wait(WNOHANG)` and `dolongjmp=1`

**Severity**: Medium. Manifests in tests; rarely hits in production with
real `ssh` because real ssh does not exit instantaneously after fork.

**Location**: `autossh.c:898-948` (the body of the `for(;;)` loop in
`ssh_watch`, specifically the window between the call to
`ssh_wait(WNOHANG)` and the assignment `dolongjmp = 1`).

**Symptom**: If the SSH child exits in this brief window, the kernel
delivers SIGCHLD, `sig_catch()` runs with `dolongjmp == 0`, and the
handler returns without `siglongjmp`-ing. The signal is "consumed".
Subsequently `poll()` is called and blocks until the next `SIGALRM`
(after `secs_left` seconds) or another signal arrives. With default
`poll_time = 600`, the parent can hang for up to 10 minutes between a
quick child exit and noticing it.

**Repro**: a mock-`ssh` that exits 255 in ≤ 100 ms causes the parent to
hang ~poll_time seconds per restart. See the comment block in
`tests/integration/run.sh` near `test_restart_on_exit_255` for the
removed test that exposed this.

**Workaround**: set `AUTOSSH_POLL` to a small value (e.g. `5`). This
caps the per-iteration hang.

**Proper fix sketch**:

The race-free pattern is to block all relevant signals (`SIGCHLD`,
`SIGALRM`, `SIGINT`, `SIGTERM`, `SIGUSR1`) during the critical section
between `ssh_wait(WNOHANG)` and the wait point, then use `ppoll(2)`
or `sigsuspend(2)` to atomically unblock-and-wait. Pseudocode:

```c
sigset_t blockmask, savedmask;
/* fill blockmask with the signals above */
sigprocmask(SIG_BLOCK, &blockmask, &savedmask);

for (;;) {
    if ((val = sigsetjmp(jumpbuf, 1)) == 0) {
        r = ssh_wait(WNOHANG);          /* signals blocked here */
        if (r != P_CONTINUE) {
            sigprocmask(SIG_SETMASK, &savedmask, NULL);
            return r;
        }
        /* ... set up alarm, dolongjmp=1 ... */
        if (ssh_stderr_fd >= 0) {
            ppoll(&spfd, 1, NULL, &savedmask);  /* atomic */
        } else {
            sigsuspend(&savedmask);
        }
    } else {
        /* siglongjmp restored mask to whatever was saved at sigsetjmp */
        switch(val) { /* ... */ }
    }
}
```

This is a non-trivial change to the signal-handling layer of `ssh_watch`
and is intentionally out of scope for the testing/fix work in commits
8986ca6 / a35c30a / 75cc0f8. Filed as a known issue so it isn't lost.

## C variadic logging shim — RESOLVED

The C variadic functions (errlog, xerrlog, doerrlog) have been
removed and replaced with Rust `errlog!`/`xerrlog!` macros backed
by non-variadic `errlog_str(level, *const c_char)` /
`xerrlog_str(level, *const c_char) -> !` in `src/log.rs`. The
macros use Rust's native `format!` and convert to a `CString`
before calling the shim, so all formatting happens in Rust.

The only remaining C is a 12-line file `c-shim/jumpbuf.c`
containing the typed `sigjmp_buf jumpbuf;` storage —
`libc::sigjmp_buf` isn't exposed by the libc 0.2 crate so the
typed instance still lives on the C side; Rust references it
via the opaque `JmpBuf` extern type.
