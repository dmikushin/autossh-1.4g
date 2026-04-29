/*
 *	autossh — C shim: variadic logging + sigjmp_buf storage.
 *
 *	Everything else has been ported to Rust (src/*.rs). The
 *	functions kept here exist because:
 *
 *	- errlog/xerrlog/doerrlog are C-variadic. Stable Rust can
 *	  call C variadic functions but cannot define them.
 *	- jumpbuf is sigjmp_buf-typed; libc::sigjmp_buf is not in
 *	  the Rust libc crate, so the storage lives in C and Rust
 *	  references it via an opaque extern type.
 *	- main() is a one-line wrapper around Rust's autossh_main().
 *
 *	From the example of rstunnel.
 *	Copyright (c) Carson Harding, 2002-2018. All rights reserved.
 *	$Id: autossh.c,v 1.91 2019/01/05 01:23:39 harding Exp $
 */

#include "config.h"

#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
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
extern int   autossh_main(int argc, char **argv);

/* sigjmp_buf storage. Rust's signals.rs declares an opaque JmpBuf
 * extern; this is the actual instance the linker resolves. */
sigjmp_buf jumpbuf;

/* Log to file and/or syslog. */
void
doerrlog(int level, char *fmt, va_list ap)
{
	FILE	*fl;
#ifndef HAVE_VSYSLOG
	char	logbuf[1024];
#endif

	fl = flog;

	if (loglevel >= level) {
		if (logtype & L_SYSLOG) {
#ifndef HAVE_VSYSLOG
			(void)vsnprintf(logbuf, sizeof(logbuf), fmt, ap);
			syslog(level, logbuf);
#else
			vsyslog(level, fmt, ap);
#endif
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

#ifndef UNIT_TEST_NO_MAIN
int
main(int argc, char **argv)
{
	/* Body lives in src/main_logic.rs. */
	return autossh_main(argc, argv);
}
#endif
