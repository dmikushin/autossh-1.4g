/*
 * Tests for ssh_wait() — interprets the child's exit status and
 * decides whether to RESTART, EXITOK, EXITERR, or CONTINUE.
 *
 * The function calls waitpid (mocked), check_ssh_stderr (we
 * disable it via ssh_stderr_fd=-1), and time().
 */

#include <stdio.h>
#include <signal.h>
#include <sys/wait.h>

#include "../framework.h"
#include "../mocks/mocks.h"

extern int    ssh_wait(int options);
extern int    cchild;
extern int    ssh_stderr_fd;
extern int    start_count;
extern double gate_time;
extern time_t start_time;

/*
 * P_* values from autossh.c. Re-declare locally to avoid leaking
 * the autossh internal header.
 */
#define P_CONTINUE  0
#define P_RESTART   1
#define P_EXITOK    2
#define P_EXITERR   3

#define CHILD 1234

static void setup(int sc, double gt)
{
    mocks_reset();
    cchild = CHILD;
    ssh_stderr_fd = -1;          /* skip check_ssh_stderr */
    start_count = sc;
    gate_time = gt;
    start_time = mock_current_time;
}

static void enq(pid_t ret, int status)
{
    mock_waitpid_queue[mock_waitpid_qlen].ret    = ret;
    mock_waitpid_queue[mock_waitpid_qlen].status = status;
    mock_waitpid_queue[mock_waitpid_qlen].err    = 0;
    mock_waitpid_qlen++;
}

/* helpers for status word — these match the Linux layout */
static int exited(int code)   { return (code & 0xff) << 8; }
static int signaled(int sig)  { return sig & 0x7f; }

TEST(wnohang_no_child_yet)
{
    setup(1, 30);
    enq(0, 0);  /* still alive */
    ASSERT_EQ(ssh_wait(WNOHANG), P_CONTINUE);
}

TEST(exit_zero_means_ok)
{
    setup(2, 30);
    mock_current_time += 100;  /* past gate_time */
    enq(CHILD, exited(0));
    ASSERT_EQ(ssh_wait(0), P_EXITOK);
}

TEST(exit_255_restarts)
{
    setup(2, 30);
    mock_current_time += 100;
    enq(CHILD, exited(255));
    ASSERT_EQ(ssh_wait(0), P_RESTART);
}

TEST(exit_255_premature_first_try_exits_with_error)
{
    setup(1, 30);
    /* start_count=1, gate_time=30, but only 5s elapsed → premature */
    mock_current_time += 5;
    enq(CHILD, exited(255));
    ASSERT_EQ(ssh_wait(0), P_EXITERR);
}

TEST(exit_1_first_try_after_gatetime_is_error)
{
    /*
     * status==1 with start_count==1 and gate_time != 0 falls through
     * to default → P_EXITERR (interpreted as "user setup error").
     */
    setup(1, 30);
    mock_current_time += 60;  /* past gate_time, NOT premature */
    enq(CHILD, exited(1));
    ASSERT_EQ(ssh_wait(0), P_EXITERR);
}

TEST(exit_1_with_gatetime_zero_restarts)
{
    setup(1, 0);
    mock_current_time += 1;
    enq(CHILD, exited(1));
    ASSERT_EQ(ssh_wait(0), P_RESTART);
}

TEST(exit_1_after_first_start_restarts)
{
    setup(2, 30);
    mock_current_time += 100;
    enq(CHILD, exited(1));
    ASSERT_EQ(ssh_wait(0), P_RESTART);
}

TEST(exit_2_treated_like_exit_1)
{
    setup(2, 30);
    mock_current_time += 100;
    enq(CHILD, exited(2));
    ASSERT_EQ(ssh_wait(0), P_RESTART);
}

TEST(exit_other_means_remote_command_error)
{
    setup(2, 30);
    mock_current_time += 100;
    enq(CHILD, exited(42));
    ASSERT_EQ(ssh_wait(0), P_EXITERR);
}

TEST(killed_by_signal_restarts)
{
    setup(2, 30);
    mock_current_time += 100;
    /* WIFSIGNALED true, term sig SIGTERM */
    enq(CHILD, signaled(SIGTERM));
    ASSERT_EQ(ssh_wait(0), P_RESTART);
}

TEST(exit_255_at_gatetime_zero_first_try_still_premature_check_skipped)
{
    /*
     * gate_time==0 disables the premature-exit guard, so even on
     * the first try a 255 exit just restarts.
     */
    setup(1, 0);
    mock_current_time += 1;
    enq(CHILD, exited(255));
    ASSERT_EQ(ssh_wait(0), P_RESTART);
}

TEST_SUITE_BEGIN("ssh_wait")
    RUN_TEST(wnohang_no_child_yet);
    RUN_TEST(exit_zero_means_ok);
    RUN_TEST(exit_255_restarts);
    RUN_TEST(exit_255_premature_first_try_exits_with_error);
    RUN_TEST(exit_1_first_try_after_gatetime_is_error);
    RUN_TEST(exit_1_with_gatetime_zero_restarts);
    RUN_TEST(exit_1_after_first_start_restarts);
    RUN_TEST(exit_2_treated_like_exit_1);
    RUN_TEST(exit_other_means_remote_command_error);
    RUN_TEST(killed_by_signal_restarts);
    RUN_TEST(exit_255_at_gatetime_zero_first_try_still_premature_check_skipped);
TEST_SUITE_END
