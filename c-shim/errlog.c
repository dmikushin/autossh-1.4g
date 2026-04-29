/*
 *	autossh — C shim: variadic logging + sigjmp_buf storage.
 *
 *	Everything else is in Rust (src/*.rs). This file is compiled
 *	into the autossh staticlib via build.rs (cc crate) so cargo
 *	can produce a self-contained binary; tests link against it
 *	the same way.
 *
 *	The two reasons this still exists in C:
 *
 *	- errlog/xerrlog/doerrlog are C-variadic. Stable Rust can
 *	  call C variadic functions but cannot *define* them
 *	  (rust-lang/rust#44930, still nightly-gated as of 1.95).
 *	- jumpbuf is sigjmp_buf-typed; libc::sigjmp_buf is not in
 *	  the Rust libc crate, so the storage lives in C and Rust
 *	  references it via an opaque extern type.
 *
 *	From the example of rstunnel.
 *	Copyright (c) Carson Harding, 2002-2018. All rights reserved.
 */

#include <stdarg.h>
#include <stdio.h>
#include <syslog.h>
#include <setjmp.h>
#include <unistd.h>

#ifdef HAVE___PROGNAME
extern char *__progname;
#else
char *__progname = "autossh";
#endif

#define L_FILELOG	0x01
#define L_SYSLOG	0x02

/* Globals provided by Rust (src/globals.rs). */
extern int	logtype;
extern int	loglevel;
extern FILE	*flog;

/* Forward declarations to Rust-provided helpers. */
extern char *timestr(void);
extern void  ssh_kill(void);
extern void  unlink_pid_file(void);

/* sigjmp_buf storage. Rust's signals.rs declares an opaque JmpBuf
 * extern; this is the actual instance the linker resolves. */
sigjmp_buf jumpbuf;

/* Log to file and/or syslog. */
void
doerrlog(int level, char *fmt, va_list ap)
{
	FILE	*fl;

	fl = flog;

	if (loglevel >= level) {
		if (logtype & L_SYSLOG) {
			vsyslog(level, fmt, ap);
		} else if (!fl) {
			fl = stderr;
		}
		if ((logtype & L_FILELOG) && fl) {
			fprintf(fl, "%s %s[%d]: ", timestr(),
			    __progname, (int)getpid());
			vfprintf(fl, fmt, ap);
			fprintf(fl, "\n");
			fflush(fl);
		}
	}
	return;
}

void
errlog(int level, char *fmt, ...)
{
	va_list	ap;
	va_start(ap, fmt);
	doerrlog(level, fmt, ap);
	va_end(ap);
}

void
xerrlog(int level, char *fmt, ...)
{
	va_list	ap;
	va_start(ap, fmt);
	doerrlog(level, fmt, ap);
	va_end(ap);

	ssh_kill();
	unlink_pid_file();
	_exit(1);
}
