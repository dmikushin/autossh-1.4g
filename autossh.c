/*
 *	Start an ssh session (or tunnel) and monitor it.
 *	If it fails or blocks, restart it.
 *
 * 	From the example of rstunnel.
 *
 * Copyright (c) Carson Harding, 2002-2018.
 * All rights reserved.
 *
 * Redistribution and use in source and binary forms, with or without
 * modification, are freely permitted.
 *
 * THIS SOFTWARE IS PROVIDED ``AS IS'' AND ANY EXPRESS OR IMPLIED WARRANTIES, 
 * INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY 
 * AND FITNESS FOR A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL 
 * THE AUTHOR OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, 
 * SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO,
 * PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS;
 * OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, 
 * WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR 
 * OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF 
 * ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
 *
 * $Id: autossh.c,v 1.91 2019/01/05 01:23:39 harding Exp $
 *
 */

#include "config.h"

#include <sys/types.h>
#include <sys/time.h>

#ifndef HAVE_SOCKLEN_T
typedef int32_t socklen_t;
#endif

#include <sys/socket.h>
#include <sys/utsname.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <fcntl.h>
#include <netdb.h>
#include <stdio.h>
#include <string.h>
#include <stdlib.h>
#include <unistd.h>
#include <signal.h>
#include <limits.h>
#include <sys/wait.h>
#include <setjmp.h>
#include <stdarg.h>
#include <syslog.h>
#include <time.h>
#include <errno.h>

#ifndef HAVE_POLL
#  ifdef HAVE_SELECT
#    include "fakepoll.h"
#  else
#    error "System lacks both select() and poll()!"
#  endif
#else
#  include <poll.h>
#endif

#ifndef __attribute__
#  if __GNUC__ < 2 || (__GNUC__ == 2 && __GNUC_MINOR__ < 8) || __STRICT_ANSI__
#    define __attribute__(x)
#  endif
#endif

#ifndef _PATH_DEVNULL
#  define _PATH_DEVNULL "/dev/null"
#endif

#ifndef HAVE_DAEMON
#  include "daemon.h"
#endif

#ifdef HAVE___PROGNAME
extern char *__progname;
#else
char *__progname;
#endif

const char *rcsid = "$Id: autossh.c,v 1.91 2019/01/05 01:23:39 harding Exp $";

#ifndef SSH_PATH
#  define SSH_PATH "/usr/bin/ssh"
#endif

#define POLL_TIME	600	/* 10 minutes default */
#define GATE_TIME	30	/* 30 seconds default */
#define MAX_LIFETIME	0	/* default max lifetime of forever */
#define TIMEO_NET	15000	/* poll on accept() and io (msecs) */
#define MAX_CONN_TRIES	3	/* how many attempts */
#define MAX_START	(-1)	/* max # of runs; <0 == forever */
#define MAX_MESSAGE	64	/* max length of message we can add */
#define SIGTERM_GRACE	2	/* seconds to wait for SIGTERM before SIGKILL */
#define SIGKILL_WAIT	2	/* seconds to wait for SIGKILL before abandoning */
#define STDERR_BUF_SZ	4096	/* buffer for reading SSH stderr */
#define PORT_FWD_FAIL_DELAY 5	/* seconds to wait before restart on port fwd failure */
#define MAX_SESSION	0	/* default max stderr silence (0 = no watchdog) */

#define P_CONTINUE	0	/* continue monitoring */
#define P_RESTART	1	/* restart ssh process */
#define P_EXITOK	2	/* exit ok */
#define P_EXITERR	3	/* exit with error */

#define L_FILELOG 	0x01	/* log to file   */
#define L_SYSLOG  	0x02	/* log to syslog */

#define NO_RD_SOCK	-2	/* magic flag for echo: no read socket */

#define N_FAST_TRIES    5       /* try this many times fast before slowing */

#define	OPTION_STRING "M:V1246ab:c:e:fgi:kl:m:no:p:qstvw:xyACD:E:F:GI:MJKL:NO:PQ:R:S:TW:XYB:"

/*
 * All file-scope globals were moved to src/globals.rs (Phase 6.4).
 * The C code below references them via these extern declarations.
 */
extern int	logtype;
extern int	loglevel;
extern int	syslog_perror;
extern FILE	*flog;

extern char	*writep;
extern char	readp[16];
extern char	*echop;
extern char	*mhost;
extern char	*env_port;
extern char	*echo_message;
extern char	*pid_file_name;
extern int	pid_file_created;
extern time_t	pid_start_time;
extern int	poll_time;
extern int	first_poll_time;
extern double	gate_time;
extern int	max_start;
extern double 	max_lifetime;
extern double	max_session;
extern int	net_timeout;
extern char	*ssh_path;
extern int	start_count;
extern time_t	start_time;

#if defined(__CYGWIN__)
int	ntservice;		/* set some stuff for running as nt service */
#endif

/* newac, newav and add_arg() — moved to src/args.rs (Phase 1 port) */
extern int	newac;
extern char  **newav;
extern void	add_arg(char *s);
#define START_AV_SZ	16

extern int	cchild;
extern int	ssh_stderr_fd;
extern time_t	pipe_lost_time;
extern time_t	last_stderr_time;
extern volatile sig_atomic_t	port_fwd_failed;

extern volatile sig_atomic_t   exit_signalled;
extern volatile sig_atomic_t	restart_ssh;
extern volatile sig_atomic_t	dolongjmp;
sigjmp_buf jumpbuf;

void	usage(int code) __attribute__ ((__noreturn__));
void	get_env_args(void);
void	add_arg(char *s);
void	strip_arg(char *arg, char ch, char *opts);
int	ssh_run(int sock, char **argv);
int	ssh_watch(int sock);
int	ssh_wait(int options);
void	ssh_kill(void);
int	conn_test(int sock, char *host, char *write_port);
int	conn_poll_for_accept(int sock, struct pollfd *pfd);
int	conn_send_and_receive(char *rp, char *wp, size_t len, 
	    struct pollfd *pfd, int ntopoll);
#ifndef HAVE_ADDRINFO
void	conn_addr(char *host, char *port, struct sockaddr_in *resp);
#else
void	conn_addr(char *host,  char *port, struct addrinfo **resp);
#endif
int	conn_listen(char *host,  char *port);
int	conn_remote(char *host,  char *port);
void	grace_time(time_t last_start);
int	check_ssh_stderr(void);
void	unlink_pid_file(void);
void	errlog(int level, char *fmt, ...)
	    __attribute__ ((__format__ (__printf__, 2, 3)));
void	xerrlog(int level, char *fmt, ...)
	    __attribute__ ((__format__ (__printf__, 2, 3)));
void	doerrlog(int level, char *fmt, va_list ap);
char	*timestr(void);
void	set_exit_sig_handler(void);
void    set_sig_handlers(void);
void    unset_sig_handlers(void);
void    sig_catch(int sig);
int	exceeded_lifetime(void);
unsigned int	clear_alarm_timer(void);

/*
 * usage / unlink_pid_file / timestr — moved to src/util.rs (Phase 6 port).
 */

#ifndef UNIT_TEST_NO_MAIN
int
main(int argc, char **argv)
{
	int	i;
	int	n;
	int	ch;
	char	*s;
	int	wp, rp, ep = 0;
	char	wmbuf[256], rmbuf[256];
	FILE	*pid_file;

	int     retval = 0;
	int	sock = -1;
	int	done_fwds = 0;
	int	runasdaemon = 0;
	int	sawargstop = 0;
#if defined(__CYGWIN__)
	int	sawoptionn = 0;
#endif

#ifndef HAVE___PROGNAME
	__progname = "autossh";
#endif	

	/* 
	 * set up options from environment
	 */
	get_env_args();

	/*
	 * We accept all ssh args, and quietly pass them on
	 * to ssh when we call it.
	 */
	while ((ch = getopt(argc, argv, OPTION_STRING)) != -1) {
		switch(ch) {
		case 'M':
			if (!env_port)
				writep = optarg;
			break;
		case 'V':
			fprintf(stdout, "%s %s\n", __progname, VER);
			exit(0);
			break;
		case 'f':
			runasdaemon = 1;
			break;
#if defined(__CYGWIN__)
		case 'N':
			sawoptionn = 1;
			break;
#endif
		case '?':
			usage(1);
			break;
		default:
			/* other options get passed to ssh */
			break;
		}
	}

	/* if we got it from the environment */
	if (env_port)
		writep = env_port;

	/*
	 * We must at least have a monitor port and a remote host.
	 */
	if (!writep || argc == optind)
		usage(1);

	if (logtype & L_SYSLOG)
		openlog(__progname, LOG_PID|syslog_perror, LOG_USER);

	/*
	 * Check for echo port
	 */
	if ((s = strchr(writep, ':')) != NULL) {
		*s = '\0';
		echop = s + 1;
		ep = strtoul(echop, &s, 0);
		if (*echop == '\0' || *s != '\0' || ep == 0)
			xerrlog(LOG_ERR, "invalid echo port  \"%s\"", echop);
	}

	if ((s = getenv("AUTOSSH_MHOST")) != NULL)
		if (*s != '\0')
			mhost = s;

	/* 
	 * Check, and get the read port (write port + 1);
	 * then construct port-forwarding arguments for ssh.
	 */
	wp = strtoul(writep, &s, 0);
	if (*writep == '\0' || *s != '\0')
		xerrlog(LOG_ERR, "invalid port \"%s\"", writep);
	if (wp == 0) {
		errlog(LOG_INFO, "port set to 0, monitoring disabled");
		writep = NULL;
	}
	else if (wp > 65534 || wp < 0)
		xerrlog(LOG_ERR, "monitor port (%d) out of range", wp);
	else {
		rp = wp+1;
		/* all this for solaris; we could use asprintf() */
		(void)snprintf(readp, sizeof(readp), "%d", rp);

		/* port-forward arg strings */
		n = snprintf(wmbuf, sizeof(wmbuf), "%d:%s:%d", wp, mhost, 
		        echop ? ep : wp);
		if (n > sizeof(wmbuf))
			xerrlog(LOG_ERR, 
			    "overflow building forwarding string");
		if (!echop) {
			n = snprintf(rmbuf, sizeof(rmbuf), "%d:%s:%d", 
			        wp, mhost, rp);
			if (n > sizeof(rmbuf))
				xerrlog(LOG_ERR, 
				    "overflow building forwarding string");
		}
	}

	/* 
	 * Adjust timeouts if necessary: net_timeout is first
	 * the timeout for accept and then for io, so if the 
	 * poll_time is set less than 2 timeouts, the timeouts need 
	 * to be adjusted to be at least 1/2. Perhaps there should be
	 * be some padding here as well....
	 */
	if ((poll_time * 1000) / 2 < net_timeout) {
		net_timeout = (poll_time * 1000) / 2;
		errlog(LOG_INFO,
		    "short poll time: adjusting net timeouts to %d",
		    net_timeout);
	}

	/*
	 * Build a new arg list, skipping -f, -M and inserting 
	 * port forwards.
	 */
	add_arg(ssh_path);

#if defined(__CYGWIN__)
	if (ntservice && !sawoptionn)
		add_arg("-N");
#endif

	for (i = 1; i < argc; i++) {
		/* 
		 * We step past the first '--', taking it as ours
		 * (autossh's). Any further ones we pass to ssh.
		 */
		if (argv[i][0] == '-' && argv[i][1] == '-') {
			if (!sawargstop) {
				sawargstop = 1;
				continue;
			}
		}
 		if (wp && env_port && !done_fwds) {
			add_arg("-L");
			add_arg(wmbuf);
			if (!echop) {
				add_arg("-R");
				add_arg(rmbuf);
			}
			done_fwds = 1;
		} else if (!sawargstop && argv[i][0] == '-' && argv[i][1] == 'M') {
			if (argv[i][2] == '\0')
				i++;
			if (wp && !done_fwds) {
				add_arg("-L");
				add_arg(wmbuf);
				if (!echop) {
					add_arg("-R");
					add_arg(rmbuf);
				}
				done_fwds = 1;
			}
			continue;
		}
		/* look for -f in option args and strip out */
		strip_arg(argv[i], 'f', OPTION_STRING);
		add_arg(argv[i]);
	}

	if (runasdaemon) {
		if (daemon(0, 0) == -1) {
			xerrlog(LOG_ERR, "run as daemon failed: %s", 
			    strerror(errno));
		}
		/* 
		 * If running as daemon, the user likely wants it
		 * to just run and not fail early (perhaps machines
		 * are coming up, etc.)
		 */ 
		gate_time = 0;
	}

	/* 
	 * Only if we're doing the network monitor thing.
	 * Socket once opened stays open for listening for 
	 * the duration of the program.
	 */
	if (writep) {
		if (!echop) {
			sock = conn_listen(mhost, readp);
			/* set close-on-exec */
			(void)fcntl(sock, F_SETFD, FD_CLOEXEC);
		} else
			sock = NO_RD_SOCK;
	}

	if (pid_file_name) {
		pid_file = fopen(pid_file_name, "w");
		if (!pid_file) {
			xerrlog(LOG_ERR, "cannot open pid file \"%s\": %s",
			    pid_file_name, strerror(errno));
		}
		pid_file_created = 1;
		atexit(unlink_pid_file);
		if (fprintf(pid_file, "%d\n", (int)getpid()) == 0)
			xerrlog(LOG_ERR, "write failed to pid file \"%s\": %s",
			    pid_file_name, strerror(errno));
		fflush(pid_file);
		fclose(pid_file);
	}

	retval = ssh_run(sock, newav);

	if (sock >= 0) {
		shutdown(sock, SHUT_RDWR);
		close(sock);
	}

	if (logtype & L_SYSLOG)
		closelog();

	if (retval == P_EXITERR)
		exit(1);
	exit(0);
}
#endif /* UNIT_TEST_NO_MAIN */

/*
 * add_arg() — moved to src/args.rs (Phase 1 port).
 */

/*
 * strip_arg() — moved to src/args.rs (Phase 1 port).
 */

/*
 * get_env_args() — moved to src/env.rs (Phase 2 port).
 * Note: the Cygwin __CYGWIN__ branch (AUTOSSH_NTSERVICE) was not
 * ported; current target is Linux. Re-add to env.rs if Cygwin
 * support is restored.
 */

/*
 * ssh_run() — moved to src/run.rs (Phase 6 port).
 */

/*
 * check_ssh_stderr() — moved to src/stderr_drain.rs (Phase 4 port).
 */

/*
 * ssh_watch() — moved to src/watch.rs (Phase 5 port).
 */

/*
 * clear_alarm_timer() and exceeded_lifetime() — moved to
 * src/lifetime.rs (Phase 1 port).
 */

/*
 * ssh_wait() — moved to src/wait.rs (Phase 4 port).
 */

/*
 * ssh_kill() — moved to src/kill.rs (Phase 4 port).
 */

/*
 * grace_time() — moved to src/grace.rs (Phase 4 port).
 */

/*
 * set_exit_sig_handler / set_sig_handlers / unset_sig_handlers
 * — moved to src/signals.rs (Phase 6 port).
 */

/*
 * sig_catch() — moved to src/signals.rs (Phase 3 port).
 * Globals it touches (exit_signalled, restart_ssh, dolongjmp, jumpbuf)
 * still live below; Rust accesses them via extern static mut.
 */

/*
 * conn_test, conn_poll_for_accept, conn_send_and_receive,
 * conn_addr, conn_remote, conn_listen — moved to src/conn.rs
 * (Phase 6 port). Only the HAVE_ADDRINFO branch is supported.
 */


/*
 * Log errors.
 */	
void
errlog(int level, char *fmt, ...)
{
	va_list	ap;

	va_start(ap, fmt);
	doerrlog(level, fmt, ap);
	va_end(ap);
}

/*
 * Log and then exit with error status.
 */
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

/*
 * Log to file and/or syslog as directed. We want different
 * behaviour before syslog has been called and set up; and
 * different behaviour before we fork for ssh: errors before
 * that point result in exit.
 */
void
doerrlog(int level, char *fmt, va_list ap)
{
	FILE	*fl;
#ifndef HAVE_VSYSLOG
	char	logbuf[1024];
#endif

	fl = flog;	/* only set per-call */

	if (loglevel >= level) {
		if (logtype & L_SYSLOG) {
#ifndef HAVE_VSYSLOG
			(void)vsnprintf(logbuf, sizeof(logbuf), fmt, ap);
			syslog(level, logbuf);
#else
			vsyslog(level, fmt, ap);
#endif
		} else if (!fl) {
			/* 
			 * if we're not using syslog, and we
			 * don't have a log file, then use
			 * stderr.
			 */
			fl = stderr;
		}
		if ((logtype & L_FILELOG) && fl) {
			fprintf(fl, 
			    "%s %s[%d]: ", timestr(),
			    __progname, (int)getpid());
			vfprintf(fl, fmt, ap);
			fprintf(fl, "\n");
			fflush(fl);
		}
	}
	return;
}

/* END */
