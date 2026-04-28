/*
 * Tests for ssh_watch() — the main monitoring loop.
 *
 * This is the most important place to test because the recent
 * watchdog/Ctrl+C fixes live here. We mock poll, waitpid, time,
 * alarm, kill, sleep, read so the loop is fully deterministic.
 *
 * Strategy: stage a minimal sequence of mock results that drives
 * the loop into the branch we care about, then assert on:
 *   - return value (P_RESTART/EXITERR/EXITOK)
 *   - whether ssh_kill was called (mock_kill_call_count)
 *   - alarm value (clamping by max_session)
 */

#include <stdio.h>
#include <signal.h>
#include <poll.h>
#include <unistd.h>
#include <sys/wait.h>

#include "../framework.h"
#include "../mocks/mocks.h"

extern int     ssh_watch(int sock);
extern int     cchild;
extern int     ssh_stderr_fd;
extern int     start_count;
extern double  gate_time;
extern double  max_session;
extern int     poll_time;
extern int     first_poll_time;
extern double  max_lifetime;
extern time_t  start_time;
extern time_t  pid_start_time;
extern time_t  pipe_lost_time;
extern time_t  last_stderr_time;
extern char   *writep;
extern volatile sig_atomic_t exit_signalled;
extern volatile sig_atomic_t restart_ssh;
extern volatile sig_atomic_t dolongjmp;
extern volatile sig_atomic_t port_fwd_failed;

#define P_CONTINUE  0
#define P_RESTART   1
#define P_EXITOK    2
#define P_EXITERR   3

#define CHILD_PID 9999
#define FAKE_FD   42

static void setup(void)
{
    mocks_reset();
    cchild           = CHILD_PID;
    ssh_stderr_fd    = FAKE_FD;
    start_count      = 2;          /* past first start; bypass gate logic */
    gate_time        = 30;
    max_session      = 0;
    poll_time        = 600;
    first_poll_time  = 600;
    max_lifetime     = 0;
    start_time       = mock_current_time;
    pid_start_time   = mock_current_time;
    pipe_lost_time   = 0;
    last_stderr_time = mock_current_time;
    writep           = NULL;        /* -M 0 mode */
    exit_signalled   = 0;
    restart_ssh      = 0;
    dolongjmp        = 0;
    port_fwd_failed  = 0;
    mock_write_fd_filter = STDERR_FILENO;
}

static void enq_poll(short revents, int ret)
{
    mock_poll_queue[mock_poll_qlen].revents = revents;
    mock_poll_queue[mock_poll_qlen].ret     = ret;
    mock_poll_qlen++;
}

static void enq_waitpid(pid_t ret, int status)
{
    mock_waitpid_queue[mock_waitpid_qlen].ret    = ret;
    mock_waitpid_queue[mock_waitpid_qlen].status = status;
    mock_waitpid_qlen++;
}

static void enq_read(const char *data, int len)
{
    mock_read_queue[mock_read_qlen].data = data;
    mock_read_queue[mock_read_qlen].len  = len;
    mock_read_qlen++;
}

/* ---- exit_signalled short-circuits the loop ---- */
TEST(exit_signalled_returns_exiterr)
{
    setup();
    /*
     * waitpid first call: child still alive (so we get past P_CONTINUE
     * check), then exit_signalled check kicks in.
     */
    enq_waitpid(0, 0);
    /* enough subsequent waitpids for ssh_kill */
    enq_waitpid(CHILD_PID, 0);
    exit_signalled = 1;
    int rc = ssh_watch(-1);
    ASSERT_EQ(rc, P_EXITERR);
    /* ssh_kill must have sent SIGTERM */
    ASSERT_TRUE(mock_kill_call_count >= 1);
    ASSERT_EQ(mock_kill_calls[0].sig, SIGTERM);
}

/* ---- restart_ssh forces P_RESTART ---- */
TEST(restart_ssh_flag_returns_restart)
{
    setup();
    restart_ssh = 1;
    /* ssh_kill expects waitpids */
    enq_waitpid(CHILD_PID, 0);
    int rc = ssh_watch(-1);
    ASSERT_EQ(rc, P_RESTART);
}

/* ---- WNOHANG sees dead child → returns based on exit ---- */
TEST(reaped_child_with_status_zero_returns_exitok)
{
    setup();
    /* waitpid(WNOHANG) returns child with exit(0) */
    enq_waitpid(CHILD_PID, (0 & 0xff) << 8);
    mock_current_time += 100;  /* past gate */
    int rc = ssh_watch(-1);
    ASSERT_EQ(rc, P_EXITOK);
}

TEST(reaped_child_with_status_255_returns_restart)
{
    setup();
    enq_waitpid(CHILD_PID, (255 & 0xff) << 8);
    mock_current_time += 100;
    int rc = ssh_watch(-1);
    ASSERT_EQ(rc, P_RESTART);
}

/* ---- secs_left clamping by max_session ---- */
TEST(alarm_clamped_by_max_session)
{
    setup();
    poll_time = 600;
    first_poll_time = 600;
    max_session = 5;
    /* child alive on first WNOHANG */
    enq_waitpid(0, 0);
    /*
     * poll returns POLLIN with no data so check_ssh_stderr() finds
     * nothing → loop continues. We then return on second WNOHANG
     * via reaped child.
     */
    enq_poll(POLLIN, 1);
    /* subsequent waitpid: reap with exit 0 to break the loop */
    enq_waitpid(CHILD_PID, 0);
    mock_current_time += 100;

    ssh_watch(-1);

    /*
     * The very first alarm() call after entering the loop must be
     * clamped to max_session (5), not poll_time (600).
     */
    ASSERT_EQ(mock_alarm_last_value, 5);
}

TEST(alarm_uses_first_poll_time_when_no_max_session)
{
    setup();
    poll_time = 600;
    first_poll_time = 30;
    max_session = 0;
    enq_waitpid(0, 0);
    enq_poll(0, 0);  /* timeout */
    /*
     * On poll timeout SIGALRM would fire normally; in our mock
     * we don't actually deliver SIGALRM, so the next iteration
     * just calls waitpid again — give it a way out.
     */
    enq_waitpid(CHILD_PID, 0);
    mock_current_time += 100;

    ssh_watch(-1);
    ASSERT_EQ(mock_alarm_last_value, 30);
}

/* ---- POLLHUP closes pipe and sets pipe_lost_time ---- */
TEST(pollhup_closes_pipe_and_records_loss)
{
    setup();
    enq_waitpid(0, 0);
    enq_poll(POLLHUP, 1);
    /* Next iteration: child reaped */
    enq_waitpid(CHILD_PID, (0 & 0xff) << 8);
    mock_current_time += 100;

    ssh_watch(-1);
    ASSERT_EQ(ssh_stderr_fd, -1);
    ASSERT_NE(pipe_lost_time, 0);
}

/* ---- port forwarding failure path ---- */
TEST(port_fwd_failed_pattern_triggers_restart)
{
    setup();
    enq_waitpid(0, 0);
    enq_poll(POLLIN, 1);
    enq_read("remote port forwarding failed\n", 30);
    /* ssh_kill waitpid */
    enq_waitpid(CHILD_PID, 0);

    int rc = ssh_watch(-1);
    ASSERT_EQ(rc, P_RESTART);
    ASSERT_EQ(port_fwd_failed, 1);
    /* ssh_kill must have been called; sleep PORT_FWD_FAIL_DELAY=5 */
    ASSERT_TRUE(mock_kill_call_count >= 1);
    ASSERT_TRUE(mock_sleep_total_secs >= 5);
}

TEST_SUITE_BEGIN("ssh_watch")
    RUN_TEST(exit_signalled_returns_exiterr);
    RUN_TEST(restart_ssh_flag_returns_restart);
    RUN_TEST(reaped_child_with_status_zero_returns_exitok);
    RUN_TEST(reaped_child_with_status_255_returns_restart);
    RUN_TEST(alarm_clamped_by_max_session);
    RUN_TEST(alarm_uses_first_poll_time_when_no_max_session);
    RUN_TEST(pollhup_closes_pipe_and_records_loss);
    RUN_TEST(port_fwd_failed_pattern_triggers_restart);
TEST_SUITE_END
