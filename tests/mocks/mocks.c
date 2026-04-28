/*
 * Mock implementations for autossh unit tests.
 * Linked against test binaries with -Wl,--wrap=<sym>.
 */

#define _GNU_SOURCE
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <signal.h>
#include <time.h>
#include <poll.h>

#include "mocks.h"

/* Real symbols supplied by ld --wrap. */
extern pid_t __real_waitpid(pid_t, int *, int);
extern int   __real_kill(pid_t, int);
extern unsigned int __real_sleep(unsigned int);
extern unsigned int __real_alarm(unsigned int);
extern time_t __real_time(time_t *);
extern void   __real__exit(int) __attribute__((noreturn));
extern ssize_t __real_read(int, void *, size_t);
extern ssize_t __real_write(int, const void *, size_t);
extern char  *__real_getenv(const char *);
extern int    __real_poll(struct pollfd *, nfds_t, int);

/* ---- kill ---------------------------------------------------- */
struct mock_kill_call mock_kill_calls[MOCK_KILL_MAX_CALLS];
int                   mock_kill_call_count = 0;
int                   mock_kill_return     = 0;
int                   mock_kill_errno      = 0;

int __wrap_kill(pid_t pid, int sig)
{
    if (mock_kill_call_count < MOCK_KILL_MAX_CALLS) {
        mock_kill_calls[mock_kill_call_count].pid = pid;
        mock_kill_calls[mock_kill_call_count].sig = sig;
    }
    mock_kill_call_count++;
    if (mock_kill_return < 0)
        errno = mock_kill_errno;
    return mock_kill_return;
}

/* ---- waitpid ------------------------------------------------- */
struct mock_waitpid_result mock_waitpid_queue[MOCK_WAITPID_MAX];
int                        mock_waitpid_qlen        = 0;
int                        mock_waitpid_qpos        = 0;
int                        mock_waitpid_call_count  = 0;
pid_t                      mock_waitpid_last_pid    = 0;
int                        mock_waitpid_last_options = 0;

pid_t __wrap_waitpid(pid_t pid, int *status, int options)
{
    mock_waitpid_call_count++;
    mock_waitpid_last_pid     = pid;
    mock_waitpid_last_options = options;

    if (mock_waitpid_qpos < mock_waitpid_qlen) {
        struct mock_waitpid_result r =
            mock_waitpid_queue[mock_waitpid_qpos++];
        if (status)
            *status = r.status;
        if (r.ret < 0)
            errno = r.err;
        return r.ret;
    }
    /* exhausted queue: act as ECHILD */
    if (status)
        *status = 0;
    errno = ECHILD;
    return -1;
}

/* ---- sleep --------------------------------------------------- */
unsigned int mock_sleep_total_secs       = 0;
int          mock_sleep_call_count       = 0;
unsigned int mock_sleep_advances_time    = 0;

unsigned int __wrap_sleep(unsigned int s)
{
    mock_sleep_call_count++;
    mock_sleep_total_secs += s;
    if (mock_sleep_advances_time)
        mock_current_time += s;
    return 0;  /* always "completed" */
}

/* ---- alarm --------------------------------------------------- */
unsigned int mock_alarm_last_value = 0;
unsigned int mock_alarm_pending    = 0;
int          mock_alarm_call_count = 0;

unsigned int __wrap_alarm(unsigned int s)
{
    unsigned int prev = mock_alarm_pending;
    mock_alarm_call_count++;
    mock_alarm_last_value = s;
    mock_alarm_pending    = s;
    return prev;
}

/* ---- time ---------------------------------------------------- */
time_t mock_current_time     = 1000000000;  /* arbitrary stable epoch */
int    mock_time_call_count  = 0;

time_t __wrap_time(time_t *t)
{
    mock_time_call_count++;
    if (t)
        *t = mock_current_time;
    return mock_current_time;
}

/* ---- _exit --------------------------------------------------- */
int        mock_exit_trap   = 0;
int        mock_exit_status = 0;
sigjmp_buf mock_exit_jmp;

void __wrap__exit(int status)
{
    if (mock_exit_trap) {
        mock_exit_status = status;
        siglongjmp(mock_exit_jmp, 1);
    }
    __real__exit(status);
}

/* ---- read ---------------------------------------------------- */
struct mock_read_chunk mock_read_queue[MOCK_READ_MAX];
int                    mock_read_qlen        = 0;
int                    mock_read_qpos        = 0;
int                    mock_read_call_count  = 0;
int                    mock_read_fd_filter   = 0;  /* 0 = intercept all */

ssize_t __wrap_read(int fd, void *buf, size_t count)
{
    if (mock_read_fd_filter && fd != mock_read_fd_filter)
        return __real_read(fd, buf, count);

    mock_read_call_count++;
    if (mock_read_qpos < mock_read_qlen) {
        struct mock_read_chunk c = mock_read_queue[mock_read_qpos++];
        if (c.len < 0) {
            errno = c.err;
            return -1;
        }
        size_t n = (size_t)c.len > count ? count : (size_t)c.len;
        if (c.data)
            memcpy(buf, c.data, n);
        return (ssize_t)n;
    }
    /* exhausted: simulate EAGAIN to avoid blocking forever */
    errno = EAGAIN;
    return -1;
}

/* ---- write --------------------------------------------------- */
char         mock_write_buf[MOCK_WRITE_BUF_SZ];
unsigned int mock_write_used        = 0;
int          mock_write_call_count  = 0;
int          mock_write_fd_filter   = 0;  /* 0 = intercept all */

ssize_t __wrap_write(int fd, const void *buf, size_t count)
{
    if (mock_write_fd_filter && fd != mock_write_fd_filter)
        return __real_write(fd, buf, count);

    mock_write_call_count++;
    size_t avail = MOCK_WRITE_BUF_SZ - 1 - mock_write_used;
    size_t n = count > avail ? avail : count;
    if (n > 0) {
        memcpy(mock_write_buf + mock_write_used, buf, n);
        mock_write_used += (unsigned int)n;
        mock_write_buf[mock_write_used] = '\0';
    }
    return (ssize_t)count;
}

/* ---- getenv -------------------------------------------------- */
struct mock_env_entry mock_env[MOCK_GETENV_MAX];
int                   mock_env_count = 0;void mock_setenv(const char *name, const char *value)
{
    int i;
    for (i = 0; i < mock_env_count; i++) {
        if (strcmp(mock_env[i].name, name) == 0) {
            mock_env[i].value = value;
            return;
        }
    }
    if (mock_env_count < MOCK_GETENV_MAX) {
        mock_env[mock_env_count].name  = name;
        mock_env[mock_env_count].value = value;
        mock_env_count++;
    }
}

void mock_clearenv(void)
{
    mock_env_count = 0;
}

char *__wrap_getenv(const char *name)
{
    int i;
    for (i = 0; i < mock_env_count; i++) {
        if (strcmp(mock_env[i].name, name) == 0)
            return (char *)mock_env[i].value;  /* may be NULL */
    }
    /* fall through to real env so unrelated vars (PATH, HOME …) still work */
    return __real_getenv(name);
}

/* ---- poll ---------------------------------------------------- */
struct mock_poll_result mock_poll_queue[MOCK_POLL_MAX];
int                     mock_poll_qlen        = 0;
int                     mock_poll_qpos        = 0;
int                     mock_poll_call_count  = 0;
int                     mock_poll_last_timeout_ms = 0;

/* For raise_sig: invoke autossh's signal handler directly. */
extern void sig_catch(int sig);

int __wrap_poll(struct pollfd *fds, nfds_t nfds, int timeout)
{
    mock_poll_call_count++;
    mock_poll_last_timeout_ms = timeout;

    if (mock_poll_qpos < mock_poll_qlen) {
        struct mock_poll_result r = mock_poll_queue[mock_poll_qpos++];
        if (nfds > 0 && fds)
            fds[0].revents = r.revents;
        if (r.advance_time > 0)
            mock_current_time += r.advance_time;
        if (r.raise_sig > 0) {
            /*
             * Simulate signal delivery during the syscall. If
             * dolongjmp is set, sig_catch will siglongjmp out and
             * this function never returns.
             */
            sig_catch(r.raise_sig);
        }
        if (r.ret < 0)
            errno = r.err;
        return r.ret;
    }
    /* exhausted: simulate timeout */
    if (nfds > 0 && fds)
        fds[0].revents = 0;
    return 0;
}

/* ---- master reset ------------------------------------------- */
void mocks_reset(void)
{
    memset(mock_kill_calls, 0, sizeof(mock_kill_calls));
    mock_kill_call_count = 0;
    mock_kill_return     = 0;
    mock_kill_errno      = 0;

    memset(mock_waitpid_queue, 0, sizeof(mock_waitpid_queue));
    mock_waitpid_qlen        = 0;
    mock_waitpid_qpos        = 0;
    mock_waitpid_call_count  = 0;
    mock_waitpid_last_pid    = 0;
    mock_waitpid_last_options = 0;

    mock_sleep_total_secs    = 0;
    mock_sleep_call_count    = 0;
    mock_sleep_advances_time = 0;

    mock_alarm_last_value = 0;
    mock_alarm_pending    = 0;
    mock_alarm_call_count = 0;

    mock_current_time    = 1000000000;
    mock_time_call_count = 0;

    mock_exit_trap   = 0;
    mock_exit_status = 0;

    memset(mock_read_queue, 0, sizeof(mock_read_queue));
    mock_read_qlen       = 0;
    mock_read_qpos       = 0;
    mock_read_call_count = 0;
    mock_read_fd_filter  = 0;

    memset(mock_write_buf, 0, sizeof(mock_write_buf));
    mock_write_used        = 0;
    mock_write_call_count  = 0;
    mock_write_fd_filter   = 0;

    memset(mock_poll_queue, 0, sizeof(mock_poll_queue));
    mock_poll_qlen        = 0;
    mock_poll_qpos        = 0;
    mock_poll_call_count  = 0;
    mock_poll_last_timeout_ms = 0;

    mock_env_count = 0;
}
