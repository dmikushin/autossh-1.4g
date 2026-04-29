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

int	logtype  = L_SYSLOG;	/* default log to syslog */
int	loglevel = LOG_INFO;	/* default loglevel */
int	syslog_perror;		/* use PERROR option? */
FILE	*flog;			/* log file */

char	*writep;		/* write port as string */
char	readp[16];		/* read port as string */
char	*echop;			/* echo port as string */
char	*mhost = "127.0.0.1";	/* host in port forwards */
char	*env_port;		/* port spec'd in environment */
char	*echo_message = "";	/* message to append to echo string */
char	*pid_file_name;		/* path to pid file */
int	pid_file_created;	/* we have created pid file */
time_t	pid_start_time;		/* time autossh process started */
int	poll_time = POLL_TIME;	/* default connection poll time */
int	first_poll_time = POLL_TIME; /* initial connection poll time */
double	gate_time = GATE_TIME;	/* time to "make it out of the gate" */
int	max_start = MAX_START;  /* how many times to run (default no limit) */
double 	max_lifetime = MAX_LIFETIME; /* how long can the process/daemon live */
double	max_session = MAX_SESSION; /* max time per ssh child session */
int	net_timeout = TIMEO_NET; /* timeout on network data */
char	*ssh_path = SSH_PATH;	/* default path to ssh */
int	start_count;		/* # of times exec()d ssh */
time_t	start_time;		/* time we exec()d ssh */

#if defined(__CYGWIN__)
int	ntservice;		/* set some stuff for running as nt service */
#endif

/* newac, newav and add_arg() — moved to src/args.rs (Phase 1 port) */
extern int	newac;
extern char  **newav;
extern void	add_arg(char *s);
#define START_AV_SZ	16

int	cchild;			/* current child */
int	ssh_stderr_fd = -1;	/* read end of SSH stderr pipe */
time_t	pipe_lost_time;		/* when stderr pipe was lost (0 = pipe alive) */
time_t	last_stderr_time;	/* last time we read data from SSH stderr */
volatile sig_atomic_t	port_fwd_failed; /* SSH reported port forwarding failure */

volatile sig_atomic_t   exit_signalled;  /* signalled outside of monitor loop */
volatile sig_atomic_t	restart_ssh;	/* signalled to restart ssh child */
volatile sig_atomic_t	dolongjmp;
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

void
usage(int code)
{
	fprintf(code ? stderr : stdout,
	    "usage: %s [-V] [-M monitor_port[:echo_port]] [-f] [SSH_OPTIONS]\n", 
	    __progname);
	if (code) {
		fprintf(stderr, "\n");
		fprintf(stderr, 
		    "    -M specifies monitor port. May be overridden by"
		    " environment\n"
		    "       variable AUTOSSH_PORT. 0 turns monitoring"
		    " loop off.\n"
		    "       Alternatively, a port for an echo service on"
		    " the remote\n"
		    "       machine may be specified. (Normally port 7.)\n");
		fprintf(stderr, 
		    "    -f run in background (autossh handles this, and"
		    " does not\n"
		    "       pass it to ssh.)\n");
		fprintf(stderr, 
		    "    -V print autossh version and exit.\n");
		fprintf(stderr, "\n");
		fprintf(stderr, "Environment variables are:\n");
		fprintf(stderr, 
		    "    AUTOSSH_GATETIME    "
		    "- how long must an ssh session be established\n"
		    "                        "
		    "  before we decide it really was established\n"
		    "                        "
		    "  (in seconds). Default is %d seconds; use of -f\n"
		    "                        "
		    "  flag sets this to 0.\n", GATE_TIME);
		fprintf(stderr, 
		    "    AUTOSSH_LOGFILE     "
		    "- file to log to (default is to use the syslog\n"
		    "                        "
		    "  facility)\n");
		fprintf(stderr, 
		    "    AUTOSSH_LOGLEVEL    "
		    "- level of log verbosity\n");
		fprintf(stderr, 
		    "    AUTOSSH_MAXLIFETIME "
		    "- set the maximum time to live (seconds)\n");
		fprintf(stderr,
		    "    AUTOSSH_MAXSTART    "
		    "- max times to restart (default is no limit)\n");
		fprintf(stderr,
		    "    AUTOSSH_MAX_SESSION "
		    "- max seconds SSH may be silent on stderr (or stuck\n"
		    "                        "
		    "  after stderr pipe loss) before being considered\n"
		    "                        "
		    "  stuck and killed. 0 = no watchdog (default).\n"
		    "                        "
		    "  Recommended with -M 0.\n");
		fprintf(stderr, 
		    "    AUTOSSH_MESSAGE     "
		    "- message to append to echo string (max 64 bytes)\n");
#if defined(__CYGWIN__)
		fprintf(stderr, 
		    "    AUTOSSH_NTSERVICE   "
		    "- tweak some things for running under cygrunsrv\n");
#endif
		fprintf(stderr, 
		    "    AUTOSSH_PATH        "
		    "- path to ssh if not default\n");
		fprintf(stderr, 
		    "    AUTOSSH_PIDFILE     "
		    "- write pid to this file\n");
		fprintf(stderr, 
		    "    AUTOSSH_POLL        "
		    "- how often to check the connection (seconds)\n");
		fprintf(stderr, 
		    "    AUTOSSH_FIRST_POLL  "
		    "- time before first connection check (seconds)\n");
		fprintf(stderr, 
		    "    AUTOSSH_PORT        "
		    "- port to use for monitor connection\n");
		fprintf(stderr, 
		    "    AUTOSSH_DEBUG       "
		    "- turn logging to maximum verbosity and log to\n"
		    "                        "
		    "  stderr\n");
		fprintf(stderr, "\n");
	}
	exit(code);
}

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
 * Run ssh
 */
int
ssh_run(int sock, char **av) 
{
	int retval;
	struct	timeval tv;

	/* 
	 * There are much better things. and we all wait
	 * for solaris to get /dev/random.
	 */
	gettimeofday(&tv, NULL);
	srandom(getpid() ^ tv.tv_usec ^ tv.tv_sec);

	set_exit_sig_handler();
	
	while (max_start < 0 || start_count < max_start) {
		if (exceeded_lifetime())
			return P_EXITOK;
		restart_ssh = 0;
		start_count++;
		grace_time(start_time);
		if (exit_signalled) {
			errlog(LOG_ERR, "signalled to exit");
			return P_EXITERR;
		}
		time(&start_time);
		port_fwd_failed = 0;
		pipe_lost_time = 0;
		time(&last_stderr_time);
		if (max_start < 0)
			errlog(LOG_INFO, "starting ssh (count %d)",
			   start_count);
		else
			errlog(LOG_INFO, "starting ssh (count %d of %d)",
			   start_count, max_start);

		/*
		 * Create a pipe to capture SSH stderr so we can
		 * detect errors like "remote port forwarding failed"
		 * without waiting for the next poll interval.
		 */
		{
			int stderr_pipe[2];
			if (pipe(stderr_pipe) < 0) {
				errlog(LOG_ERR, "pipe: %s", strerror(errno));
				stderr_pipe[0] = stderr_pipe[1] = -1;
			}
			cchild = fork();
			switch (cchild) {
			case 0:
				/* child: redirect stderr to pipe */
				if (stderr_pipe[1] >= 0) {
					close(stderr_pipe[0]);
					dup2(stderr_pipe[1], STDERR_FILENO);
					close(stderr_pipe[1]);
				}
				errlog(LOG_DEBUG, "child of %d execing %s",
				    getppid(), av[0]);
				execvp(av[0], av);
				errlog(LOG_ERR, "%s: %s", av[0], strerror(errno));
				 /* else can loop restarting! */
				kill(getppid(), SIGTERM);
				_exit(1);
				break;
			case -1:
				cchild = 0;
				if (stderr_pipe[0] >= 0) close(stderr_pipe[0]);
				if (stderr_pipe[1] >= 0) close(stderr_pipe[1]);
				xerrlog(LOG_ERR, "fork: %s", strerror(errno));
				break;
			default:
				/* parent: keep read end, close write end */
				if (stderr_pipe[1] >= 0)
					close(stderr_pipe[1]);
				ssh_stderr_fd = stderr_pipe[0];
				if (ssh_stderr_fd >= 0) {
					/* set non-blocking */
					int flags = fcntl(ssh_stderr_fd, F_GETFL, 0);
					if (flags >= 0)
						fcntl(ssh_stderr_fd, F_SETFL, flags | O_NONBLOCK);
				}
				errlog(LOG_INFO, "ssh child pid is %d", (int)cchild);
				set_sig_handlers();
				retval = ssh_watch(sock);
				dolongjmp = 0;
				clear_alarm_timer();
				unset_sig_handlers();
				if (ssh_stderr_fd >= 0) {
					close(ssh_stderr_fd);
					ssh_stderr_fd = -1;
				}
				if (retval == P_EXITOK || retval == P_EXITERR)
					return retval;
				break;
			}
		}
	}

	errlog(LOG_INFO, "max start count reached; exiting");

	return P_EXITOK;
}

/*
 * check_ssh_stderr() — moved to src/stderr_drain.rs (Phase 4 port).
 */

/*
 * Periodically test network connection. On signals, determine what
 * happened or what to do with child. Return as necessary for exit
 * or restart of child.
 */
int
ssh_watch(int sock)
{
	int	r;
	int	val;
	static	int	secs_left;
	int	my_poll_time = first_poll_time;
	time_t	now;
	double	secs_to_shutdown;

#if defined(HAVE_SETPROCTITLE)
	setproctitle("parent of %d (%d)", 
	    (int)cchild, start_count);
#endif

	/*
	 * KNOWN ISSUE: SIGCHLD race between ssh_wait(WNOHANG) and
	 * dolongjmp=1. If the child exits in that window, sig_catch
	 * runs with dolongjmp==0 and returns without longjmp; the
	 * signal is lost and poll()/pause() then blocks for up to
	 * secs_left seconds before noticing the dead child. See
	 * KNOWN_ISSUES.md for analysis and a proposed ppoll/sigsuspend
	 * fix. Workaround: set AUTOSSH_POLL to a small value.
	 */

	for (;;) {
		if (restart_ssh) {
			errlog(LOG_INFO, "signalled to kill and restart ssh");
			ssh_kill();
			return P_RESTART;
		}
		if ((val = sigsetjmp(jumpbuf, 1)) == 0) {

			errlog(LOG_DEBUG, "check on child %d", cchild);

			/* poll for expired child */
			r = ssh_wait(WNOHANG);
			if (r != P_CONTINUE) {
				errlog(LOG_DEBUG, 
				    "expired child, returning %d", r);
				return r;
			}

			secs_left = clear_alarm_timer();
			if (secs_left == 0)
				secs_left = my_poll_time;

			my_poll_time = poll_time;

			if (max_lifetime != 0) {
				time(&now);
				secs_to_shutdown = max_lifetime - difftime(now,pid_start_time);
				if (secs_to_shutdown < poll_time)
					secs_left = secs_to_shutdown;
			}

			/*
			 * If a stuck-SSH watchdog is configured, ensure the
			 * alarm wakes us at least that often, so the SIGALRM
			 * handler can check stderr silence in time.
			 */
			if (max_session > 0 && secs_left > max_session)
				secs_left = max_session;

			errlog(LOG_DEBUG, 
			    "set alarm for %d secs", secs_left);

			dolongjmp = 1;
			alarm(secs_left);

			/* In case we were signalled while setting
			   all this up */
			if (exit_signalled) {
				errlog(LOG_INFO, "signalled to exit");
				ssh_kill();
				return P_EXITERR;
			}

			/*
			 * Instead of pause() which only waits for
			 * signals, use poll() on the SSH stderr pipe
			 * so we can detect errors like "remote port
			 * forwarding failed" immediately, rather than
			 * waiting for the next poll_time interval.
			 * If no stderr pipe, fall back to pause().
			 */
			if (ssh_stderr_fd >= 0) {
				struct pollfd spfd;
				spfd.fd = ssh_stderr_fd;
				spfd.events = POLLIN;
				/* poll with long timeout; SIGALRM
				 * will interrupt with EINTR */
				poll(&spfd, 1, secs_left * 1000);
				if (spfd.revents & POLLIN) {
					if (check_ssh_stderr()) {
						ssh_kill();
						errlog(LOG_INFO,
						    "waiting %d seconds "
						    "for port to be released",
						    PORT_FWD_FAIL_DELAY);
						sleep(PORT_FWD_FAIL_DELAY);
						return P_RESTART;
					}
				}
				/* If pipe closed (SSH exited/crashed)
				 * or errored without readable data,
				 * loop back to ssh_wait(WNOHANG) to
				 * detect the dead child. Start the
				 * watchdog timer if max_session is set. */
				if ((spfd.revents & (POLLHUP | POLLERR))
				    && !(spfd.revents & POLLIN)) {
					errlog(LOG_INFO,
					    "SSH stderr pipe closed");
					close(ssh_stderr_fd);
					ssh_stderr_fd = -1;
					if (pipe_lost_time == 0)
						time(&pipe_lost_time);
				}
			} else {
				pause();
			}

		} else {

			switch(val) {
			case SIGINT:
			case SIGTERM:
			case SIGQUIT:
			case SIGABRT:
				errlog(LOG_INFO, 
				    "received signal to exit (%d)", val);
				ssh_kill();
				return P_EXITERR;
				break;
			case SIGALRM:
				r = exceeded_lifetime();
				errlog(LOG_DEBUG,
				    "received SIGALRM (end-of-life %d)", r);

				/* exit if user-configured lifetime exceeded */
				if (r) {
					ssh_kill();
					return P_EXITOK;
				}

				/*
				 * Watchdog for stuck SSH, primarily for
				 * -M 0 mode where conn_test is unavailable.
				 * Two triggers, both gated by max_session > 0:
				 *   (1) stderr pipe lost and silent for too long;
				 *   (2) stderr pipe alive but ssh produced no
				 *       output for too long (e.g., stuck in
				 *       initial connect()).
				 */
				if (max_session > 0) {
					time(&now);
					if (pipe_lost_time != 0 &&
					    difftime(now, pipe_lost_time) >=
					    max_session) {
						errlog(LOG_WARNING,
						    "ssh child stuck for "
						    "%.0f secs after stderr "
						    "pipe lost; restarting",
						    difftime(now,
						        pipe_lost_time));
						ssh_kill();
						return P_RESTART;
					}
					if (last_stderr_time != 0 &&
					    difftime(now, last_stderr_time) >=
					    max_session) {
						errlog(LOG_WARNING,
						    "ssh child silent on "
						    "stderr for %.0f secs; "
						    "considered stuck, "
						    "restarting",
						    difftime(now,
						        last_stderr_time));
						ssh_kill();
						return P_RESTART;
					}
				}

				/* Check stderr for errors before
				 * running the slower conn_test */
				if (check_ssh_stderr()) {
					ssh_kill();
					errlog(LOG_INFO,
					    "waiting %d seconds "
					    "for port to be released",
					    PORT_FWD_FAIL_DELAY);
					sleep(PORT_FWD_FAIL_DELAY);
					return P_RESTART;
				}

				if (writep && sock != -1 &&
				    !conn_test(sock, mhost, writep)) {
					errlog(LOG_INFO,
					    "port down, restarting ssh");
					ssh_kill();
					return P_RESTART;
				}
#ifdef TOUCH_PIDFILE
				/*
				 * utimes() with a NULL time argument sets
				 * file access and modification times to
				 * the current time
				 */
				if (pid_file_name && 
				    utimes(pid_file_name, NULL) != 0) {
					errlog(LOG_ERR,
					    "could not touch pid file: %s",
					    strerror(errno));
				}
#endif
				break;
			default:
				break;
			}
		}
	}
}

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

void
set_exit_sig_handler()
{
	struct	sigaction act;

	memset(&act, 0, sizeof(act));
	act.sa_handler = sig_catch;
	sigemptyset(&act.sa_mask);
	act.sa_flags = 0;

	sigaction(SIGTERM, &act, NULL);
	sigaction(SIGINT,  &act, NULL);
}

void
set_sig_handlers(void)
{
	struct		sigaction act;
	sigset_t	blockmask;

	memset(&act, 0, sizeof(act));
	act.sa_handler = sig_catch;
	act.sa_flags = 0;

	/* create mask to ensure sig_catch() processes one signal at-a-time */
	sigemptyset(&blockmask);
	sigaddset(&blockmask, SIGTERM);
	sigaddset(&blockmask, SIGINT);
	sigaddset(&blockmask, SIGHUP);
	sigaddset(&blockmask, SIGUSR1);
	sigaddset(&blockmask, SIGUSR2);
	sigaddset(&blockmask, SIGCHLD);
	sigaddset(&blockmask, SIGALRM);
	sigaddset(&blockmask, SIGPIPE);
	act.sa_mask = blockmask;

	sigaction(SIGTERM, &act, NULL);
	sigaction(SIGINT,  &act, NULL);
	sigaction(SIGHUP,  &act, NULL);
	sigaction(SIGUSR1, &act, NULL);
	sigaction(SIGUSR2, &act, NULL);
	sigaction(SIGCHLD, &act, NULL);

	act.sa_flags |= SA_RESTART;
	sigaction(SIGALRM, &act, NULL);

	act.sa_handler = SIG_IGN;
	act.sa_flags = 0;
	sigaction(SIGPIPE, &act, NULL);
}

void
unset_sig_handlers(void)
{
	struct	sigaction act;

	memset(&act, 0, sizeof(act));
	act.sa_handler = SIG_DFL;
	sigemptyset(&act.sa_mask);
	act.sa_flags = 0;
	
	/* We don't reset SIGTERM, as we want the
	   handler to persist in the run_ssh() loop */
	/* This seems a little hidden down in here... */
	/* sigaction(SIGTERM, &act, NULL); */
	/* sigaction(SIGINT,  &act, NULL); */
	sigaction(SIGHUP,  &act, NULL);
	sigaction(SIGUSR1, &act, NULL);
	sigaction(SIGUSR2, &act, NULL);
	sigaction(SIGCHLD, &act, NULL);
	sigaction(SIGALRM, &act, NULL);
	sigaction(SIGPIPE, &act, NULL);
}

/*
 * sig_catch() — moved to src/signals.rs (Phase 3 port).
 * Globals it touches (exit_signalled, restart_ssh, dolongjmp, jumpbuf)
 * still live below; Rust accesses them via extern static mut.
 */

/*
 * Test the connection monitor loop can pass traffic, and that
 * we get back what we send. This needs the most testing.
 */
int
conn_test(int sock, char *host, char *write_port)
{
	int	rval;			/* default return value (failure) */
	int	tries;			/* message attempts */
	int	send_error;		/* did it go/come ok? */
	struct	pollfd	pfd[2];		/* poll fds */
	int	ntopoll;		/* # fds to poll */
	int	rd, wd;			/* read and write descriptors */
	long	id;			/* for a random number */

	struct	utsname uts;
	char	wbuf[64+sizeof(uts.nodename)+MAX_MESSAGE];
	char	rbuf[sizeof(wbuf)];

	wd = -1;			/* default desc. values */
	rd = -1;
	rval = 0;			/* default return value : no success */
	tries = 0;			/* number of attempts */

	uts.nodename[0] = '\0';
	(void)uname(&uts);
	id = random();

	if (dolongjmp != 0)
		errlog(LOG_ERR, "conn_test(): error: dolongjmp != 0");

	/* set up write connection */
	if ((wd = conn_remote(host, write_port)) == -1)
		return 0;

	pfd[1].fd = wd;
	pfd[1].events = POLLOUT;

	while (tries++ < MAX_CONN_TRIES) {

		if (tries >= MAX_CONN_TRIES) {
			errlog(LOG_DEBUG, 
			    "tried connection %d times and failed",
			    tries);
			break;				/* give up */
		} 

		/* close read socket if we're coming around again */
		if (sock != NO_RD_SOCK && rd != -1) {
			shutdown(rd, SHUT_RDWR);
			close(rd);
			rd = -1;
		}

		/* 
		 * Some data to send: something that is identifiable 
		 * as coming from ourselves. Any user can still trash 
		 * our listening port. We'd really like to be able to 
		 * connect and accept connections from certain pids 
		 * (ourself, our children).
		 */
		if (snprintf(wbuf, sizeof(wbuf), 
		    "%s %s %d %ld %s\r\n", uts.nodename, __progname, 
		    (int)getpid(), id, echo_message) >= sizeof(wbuf))
			xerrlog(LOG_ERR, "conn_test: buffer overflow");
		memset(rbuf, '\0', sizeof(rbuf));

		if (sock != NO_RD_SOCK) {
			/* 
			 * If doing loop of connections, then accept() the read
			 * connection and use both read and write fds for
			 * poll(). Replace poll fd with accepted connection fd.
			 */
			rd = conn_poll_for_accept(sock, pfd);
			if (rd < 0)
				break;			/* give up */
			pfd[0].fd = rd;
			pfd[0].events = POLLIN;
			ntopoll = 2;
		} else {
			/* 
			 * For talking to echo service, shift over and
			 * just use the one descriptor for both read and
			 * write.
			 */
			pfd[0].fd = wd;
			pfd[0].events = POLLIN|POLLOUT;
			ntopoll = 1;
		}

		send_error = conn_send_and_receive(rbuf, wbuf, 
				 	strlen(wbuf), pfd, ntopoll);
		if (send_error == 0) {
			/* we try again if received does not match sent */
			if (strcmp(rbuf, wbuf) == 0) {
				errlog(LOG_DEBUG, "connection ok");
				rval = 1;		/* success */
				break;			/* out of here */
			} else {
				errlog(LOG_DEBUG, 
				    "not what I sent: \"%s\" : \"%s\"",
				     wbuf, rbuf);
				/* loop again */
			}
		} else if (send_error == 1) {
			errlog(LOG_DEBUG, 
			    "timeout on io poll, looping to accept again");
		} else {
			errlog(LOG_DEBUG, "error on poll: %s",
			    strerror(errno));
			break;		/* hard error, we're out of here */
		}
	}

	shutdown(wd, SHUT_RDWR);
	close(wd); 
	if (sock != NO_RD_SOCK) {
		shutdown(rd, SHUT_RDWR);
		close(rd);
	}

	return rval;
}

/*
 * poll for accept(), return file descriptor for accepted connection,
 * or -1 for error.
 */
int
conn_poll_for_accept(int sock, struct pollfd *pfd)
{
	int	rd;			/* new descriptor on accept */
	int	timeo_polla;		/* for accept() */
	struct sockaddr cliaddr;
	socklen_t	len;		/* listen socket info */

	rd = 0;
	timeo_polla  = net_timeout;	/* timeout value for accept() */
	len = sizeof(struct sockaddr);

	/* 
	 * first we're going to poll for accept()
	 */
	pfd[0].fd = sock;
	pfd[0].events = POLLIN;

	for (;;) {
		switch(poll(pfd, 1, timeo_polla)) {
		case 0:
			errlog(LOG_INFO, 
			    "timeout polling to accept read connection");
			return -1;
		case -1:
			errlog(LOG_ERR, 
			    "error polling to accept read connection: %s",
			    strerror(errno));
			return -1;
		default:
			break;
		}

		if (pfd[0].revents & POLLIN) {
			rd = accept(sock, &cliaddr, &len);
			if (rd == -1) {
				errlog(LOG_ERR, 
				    "error accepting read connection: %s",
				    strerror(errno));
				return -1;
			}
			break;
		}
		break;
	}

	return rd;
}

/* 
 * Send from wp and receive into rp.
 * 	1  = try again
 * 	0  = ok
 * 	-1 = error 
 */
int
conn_send_and_receive(char *rp, char *wp, size_t len, 
    struct pollfd *pfd, int ntopoll)
{
	ssize_t nwrite, nread;
	size_t  rleft, wleft;
	int	timeo_pollio;
	int	ird, iwr;
	int	loops = 0;

	timeo_pollio = net_timeout;	/* timeout value for net io */
	rleft = wleft = len;

	/* 
	 * If two fds, one is to read, one is to write,
	 * else read and write on the same fd.
	 */
	if (ntopoll == 2) {
		ird = 0;
		iwr = 1;
	} else {
		iwr = ird = 0;
	}


	/*
	 * Now, send and receive. When we're doing the loop thing, we stop
	 * polling for write() once we've sent the whole message.
	 */
	while (rleft > 0) {

		switch(poll(pfd, ntopoll, timeo_pollio)) {
		case 0:
			return 1;
			break;
		case -1:
			return -1;
			break;
		default:
			break;
		}

		if (wleft && pfd[iwr].revents & POLLOUT) {
			while (wleft > 0) {
				nwrite = write(pfd[iwr].fd, wp, wleft);
				if (nwrite == 0) {
					wleft = 0; /* EOF */
					break;
				} else if (nwrite == -1) {
				    if (errno == EINTR || errno == EAGAIN)
					break;
			            else
					return -1;
				}
				wleft -= nwrite;
				wp    += nwrite;
			}
			/* if complete, turn off polling for write */
			if (wleft == 0) {
				ntopoll = 1;
				/* 
				 * if we are reading and writing to the 
				 * same fd then we must clear the write bit 
				 * so that poll doesn't loop tight.
				 */
				if (iwr == ird)
				    pfd[ird].events = POLLIN;
			}
		}

		if (pfd[ird].revents & POLLIN || pfd[ird].revents & POLLHUP) {
			while (rleft > 0) {
				nread = read(pfd[ird].fd, rp, rleft);
				if (nread == 0) {
					rleft = 0; /* EOF */
					break;
				} else if (nread == -1) {
				    if (errno == EINTR || errno == EAGAIN)
					break;
			            else
					return -1;
				}
				rleft -= nread;
				rp    += nread;
			}
		}

		/* 
		 * we can run into situations where the data gets black-holed
		 * and poll() can't tell. And then we loop fast and
		 * things go nuts. So if we do that, give up after a while.
		 */
		if (loops++ > 5) {
			sleep(1);
			if (loops > 10) {
				errlog(LOG_INFO, 
				    "too many loops without data");
				return -1;
			}
		}
	}

	return 0;
}

#ifndef HAVE_ADDRINFO

/*
 * Convert names to addresses, setup for connection.
 */
void
conn_addr(char *host, char *port, struct sockaddr_in *resp)
{
	struct hostent *h;

	if ((h = gethostbyname(host)) == NULL)
		xerrlog(LOG_ERR, "%s: %s", host, hstrerror(h_errno));

	resp->sin_family = h->h_addrtype;
	resp->sin_port = htons(atoi(port));
	resp->sin_addr = *((struct in_addr *) h->h_addr_list[0]);

	return;
}

/*
 * Open connection we're writing to.
 */
int
conn_remote(char *host, char *port)
{
	int	sock;
	static struct sockaddr_in res = {AF_UNSPEC};

	/* Cache the address info */
	if (res.sin_family == AF_UNSPEC)
		conn_addr(host, port, &res);

	if ((sock = socket(res.sin_family, SOCK_STREAM, 0)) == -1)
		xerrlog(LOG_ERR, "socket: %s", strerror(errno));

	if (connect(sock, (struct sockaddr *) &res, sizeof(res)) == -1) {
		errlog(LOG_INFO, "%s:%s: %s", host, port, strerror(errno));
		close(sock);
		return -1;
	}

	return sock;
}

/*
 * Returns a socket listening on a local port, bound to specified source
 * address. Errors in binding to the local listening port are fatal.
 */
int
conn_listen(char *host,  char *port)
{
	int sock;
	struct sockaddr_in res;
	int on = 1;

	/* 
	 * Unlike conn_remote, we don't need to cache the 
	 * info; we're only calling once at start. All errors
	 * here are fatal.
	 */
	conn_addr(host, port, &res);

	if ((sock = socket(res.sin_family, SOCK_STREAM, 0)) == -1)
		xerrlog(LOG_ERR, "socket: %s", strerror(errno));

	if (setsockopt(sock, SOL_SOCKET, SO_REUSEADDR,
	    (char *) &on, sizeof on) != 0) {
		xerrlog(LOG_ERR, "setsockopt: %s", strerror(errno));
	}

	if (bind(sock, (struct sockaddr *)&res, sizeof(res)) == -1)
		xerrlog(LOG_ERR, "bind on %s:%s: %s", 
		    host, port, strerror(errno));

	if (listen(sock, 1) < 0)
		xerrlog(LOG_ERR, "listen: %s", strerror(errno));

	return sock;
}

#else /* HAVE_ADDRINFO */

/*
 * Convert names to addresses, setup for connection.
 */
void
conn_addr(char *host,  char *port, struct addrinfo **resp)
{
	int family = AF_INET;
	struct addrinfo hints;
	int error;

	memset(&hints, 0, sizeof(struct addrinfo));
	hints.ai_family = family;
	hints.ai_socktype = SOCK_STREAM;
	hints.ai_protocol = IPPROTO_TCP;

	/* Allow nodename to be null */
	hints.ai_flags |= AI_PASSIVE;

	/*
	 * In the case of binding to a wildcard address
	 * default to binding to an ipv4 address.
	 */
	if (host == NULL && hints.ai_family == AF_UNSPEC)
		hints.ai_family = AF_INET;

	if ((error = getaddrinfo(host, port, &hints, resp)))
                xerrlog(LOG_ERR, "%s", gai_strerror(error));

	return;
}

/*
 * Open connection we're writing to.
 */
int
conn_remote(char *host, char *port)
{
	int	sock;
	static  struct addrinfo *res;

	/* Cache the address info */
	if (!res)
		conn_addr(host, port, &res);

	if ((sock = socket(res->ai_family, res->ai_socktype, 
	    res->ai_protocol)) == -1)
		xerrlog(LOG_ERR, "socket: %s", strerror(errno));

	if (connect(sock, res->ai_addr, res->ai_addrlen) == -1) {
		errlog(LOG_INFO, "%s:%s: %s", host, port, strerror(errno));
		close(sock);
		return -1;
	}

	return sock;
}

/*
 * Returns a socket listening on a local port, bound to specified source
 * address. Errors in binding to the local listening port are fatal.
 */
int
conn_listen(char *host,  char *port)
{
	int sock;
	struct addrinfo *res;
	int on = 1;

	/* 
	 * Unlike conn_remote, we don't need to cache the 
	 * info; we're only calling once at start. All errors
	 * here are fatal.
	 */
	conn_addr(host, port, &res);

	if ((sock = socket(res->ai_family, res->ai_socktype,
	    res->ai_protocol)) == -1)
		xerrlog(LOG_ERR, "socket: %s", strerror(errno));

	if (setsockopt(sock, SOL_SOCKET, SO_REUSEADDR,
	    (char *) &on, sizeof on) != 0) {
		xerrlog(LOG_ERR, "setsockopt: %s", strerror(errno));
	}

	if (bind(sock, (struct sockaddr *)res->ai_addr,
	    res->ai_addrlen) == -1)
		xerrlog(LOG_ERR, "bind on %s:%s: %s", 
		    host, port, strerror(errno));

	if (listen(sock, 1) < 0)
		xerrlog(LOG_ERR, "listen: %s", strerror(errno));

	freeaddrinfo(res);

	return sock;
}
#endif /* ! HAVE_ADDRINFO */

/*
 * On OpenBSD _exit() calls atexit() registered functions.
 * Solaris has a function, _exithandle(), you can call
 * before _exit().
 */
void
unlink_pid_file(void)
{
	if (pid_file_created)
		(void)unlink(pid_file_name);
	pid_file_created = 0;
}

/*
 * Nicely formatted time string for logging
 */
char *
timestr(void)
{
	static	char timestr[32];
	time_t  now;
	struct	tm *tm;

	(void)time(&now);
	tm = localtime(&now);
	(void)strftime(timestr, sizeof(timestr), 
	    "%Y/%m/%d %H:%M:%S", tm);

	return timestr;
}

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
