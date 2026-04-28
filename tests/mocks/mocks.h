/*
 * Mock state and helpers for autossh unit tests.
 *
 * These are linked in via gcc -Wl,--wrap=<sym>.  Each __wrap_X
 * intercepts the libc symbol; tests configure mock state before
 * calling autossh code.
 */

#ifndef AUTOSSH_MOCKS_H
#define AUTOSSH_MOCKS_H

#include <signal.h>
#include <sys/types.h>
#include <time.h>

/* ---- kill ---------------------------------------------------- */
#define MOCK_KILL_MAX_CALLS 32
struct mock_kill_call {
    pid_t pid;
    int   sig;
};
extern struct mock_kill_call mock_kill_calls[MOCK_KILL_MAX_CALLS];
extern int                   mock_kill_call_count;
extern int                   mock_kill_return;       /* default 0 */
extern int                   mock_kill_errno;        /* set if return -1 */

/* ---- waitpid ------------------------------------------------- */
/*
 * Programmable queue of (return_value, status, errno) tuples.
 * waitpid() consumes one per call; if queue empty, returns -1/ECHILD.
 */
#define MOCK_WAITPID_MAX 32
struct mock_waitpid_result {
    pid_t ret;
    int   status;
    int   err;
};
extern struct mock_waitpid_result mock_waitpid_queue[MOCK_WAITPID_MAX];
extern int                        mock_waitpid_qlen;
extern int                        mock_waitpid_qpos;
extern int                        mock_waitpid_call_count;
/* Last seen pid/options for assertions. */
extern pid_t                      mock_waitpid_last_pid;
extern int                        mock_waitpid_last_options;

/* ---- sleep --------------------------------------------------- */
extern unsigned int mock_sleep_total_secs;
extern int          mock_sleep_call_count;
/* If non-zero, every sleep() advances mock_current_time by N. */
extern unsigned int mock_sleep_advances_time;

/* ---- alarm --------------------------------------------------- */
extern unsigned int mock_alarm_last_value;     /* most recently set */
extern unsigned int mock_alarm_pending;        /* simulated remaining */
extern int          mock_alarm_call_count;

/* ---- time ---------------------------------------------------- */
extern time_t mock_current_time;
extern int    mock_time_call_count;

/* ---- _exit --------------------------------------------------- */
/*
 * __wrap__exit longjumps back to mock_exit_jmp if mock_exit_trap
 * is non-zero; otherwise calls __real__exit. Tests that expect
 * a code path to call _exit() set up the trap with MOCK_EXPECT_EXIT.
 */
#include <setjmp.h>
extern int      mock_exit_trap;
extern int      mock_exit_status;
extern sigjmp_buf mock_exit_jmp;

#define MOCK_EXPECT_EXIT(block) do {                                    \
    mock_exit_trap = 1;                                                 \
    if (sigsetjmp(mock_exit_jmp, 1) == 0) {                             \
        block;                                                          \
        mock_exit_trap = 0;                                             \
        fprintf(stderr, "  FAIL %s:%d: expected _exit() but block returned\n", \
                __FILE__, __LINE__);                                    \
        exit(1);                                                        \
    }                                                                   \
    mock_exit_trap = 0;                                                 \
} while (0)

/* ---- read ---------------------------------------------------- */
/*
 * Programmable queue of read() responses. Each entry supplies
 * either a buffer to return (n bytes) or an error.
 */
#define MOCK_READ_MAX 16
struct mock_read_chunk {
    const char *data;        /* NULL means error */
    int         len;         /* bytes to return; -1 for error */
    int         err;         /* errno when len < 0 */
};
extern struct mock_read_chunk mock_read_queue[MOCK_READ_MAX];
extern int                    mock_read_qlen;
extern int                    mock_read_qpos;
extern int                    mock_read_call_count;
/* If non-zero, only intercept reads for this fd; pass through others. */
extern int                    mock_read_fd_filter;

/* ---- write --------------------------------------------------- */
/* Captures everything written (concatenated) up to MOCK_WRITE_BUF_SZ. */
#define MOCK_WRITE_BUF_SZ 4096
extern char         mock_write_buf[MOCK_WRITE_BUF_SZ];
extern unsigned int mock_write_used;
extern int          mock_write_call_count;
/* If non-zero, only intercept writes for this fd; pass through others. */
extern int          mock_write_fd_filter;

/* ---- poll ---------------------------------------------------- */
/*
 * Programmable queue of poll() results. Each entry sets a single
 * fd's revents and a return value (0=timeout, >0=count, <0=err).
 * For multi-fd polls (conn_test) we record only entry index 0.
 *
 * raise_sig: if non-zero, before returning the mock invokes
 * sig_catch(raise_sig). With dolongjmp=1 (as set by ssh_watch)
 * this triggers siglongjmp back to the caller's jumpbuf — the
 * canonical way to simulate a kernel-delivered signal during
 * a blocking syscall in unit tests.
 */
#define MOCK_POLL_MAX 16
struct mock_poll_result {
    short revents;     /* applied to pollfd[0] */
    int   ret;         /* return value of poll() */
    int   err;         /* errno when ret < 0 */
    int   advance_time; /* if > 0, mock_current_time += this */
    int   raise_sig;   /* if > 0, call sig_catch(raise_sig) */
};
extern struct mock_poll_result mock_poll_queue[MOCK_POLL_MAX];
extern int                     mock_poll_qlen;
extern int                     mock_poll_qpos;
extern int                     mock_poll_call_count;
extern int                     mock_poll_last_timeout_ms;

/* ---- getenv -------------------------------------------------- */
#define MOCK_GETENV_MAX 32
struct mock_env_entry {
    const char *name;
    const char *value;       /* NULL means "not set" */
};
extern struct mock_env_entry mock_env[MOCK_GETENV_MAX];
extern int                   mock_env_count;
void mock_setenv(const char *name, const char *value);
void mock_clearenv(void);

/* ---- master reset ------------------------------------------- */
/*
 * Restore all mock state to defaults. Call at the start of each
 * test (or rely on fork-isolation in framework.h, but explicit is
 * still good).
 */
void mocks_reset(void);

#endif /* AUTOSSH_MOCKS_H */
