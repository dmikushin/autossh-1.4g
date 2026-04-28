/*
 * Tests for ssh_kill() — the aggressive process killer.
 *
 * Verifies the recently-fixed behavior:
 *   - SIGTERM_GRACE = 2 seconds (not 10)
 *   - SIGKILL_WAIT  = 2 seconds
 *   - clean reap on first WNOHANG → no SIGKILL
 *   - escalation to SIGKILL when child ignores SIGTERM
 *   - abandon (cchild=0) when even SIGKILL doesn't reap
 *   - ECHILD short-circuits cleanly
 *   - stderr fd is closed
 */

#include <stdio.h>
#include <signal.h>
#include <unistd.h>
#include <errno.h>

#include "../framework.h"
#include "../mocks/mocks.h"

extern void   ssh_kill(void);
extern int    cchild;
extern int    ssh_stderr_fd;

#define CHILD_PID 12345

static void setup(int with_stderr)
{
    mocks_reset();
    cchild = CHILD_PID;
    ssh_stderr_fd = with_stderr ? 7 : -1;
}

static void enqueue_waitpid(pid_t ret, int status, int err)
{
    mock_waitpid_queue[mock_waitpid_qlen].ret    = ret;
    mock_waitpid_queue[mock_waitpid_qlen].status = status;
    mock_waitpid_queue[mock_waitpid_qlen].err    = err;
    mock_waitpid_qlen++;
}

TEST(no_child_is_noop)
{
    mocks_reset();
    cchild = 0;
    ssh_stderr_fd = -1;
    ssh_kill();
    ASSERT_EQ(mock_kill_call_count, 0);
    ASSERT_EQ(mock_waitpid_call_count, 0);
}

TEST(stderr_fd_is_closed)
{
    setup(1);
    /* Child reaps immediately so we exit fast. */
    enqueue_waitpid(CHILD_PID, 0, 0);
    ssh_kill();
    ASSERT_EQ(ssh_stderr_fd, -1);
}

TEST(fast_reap_no_sigkill)
{
    setup(0);
    /* First waitpid returns the pid → immediate exit, no SIGKILL */
    enqueue_waitpid(CHILD_PID, 0, 0);
    ssh_kill();

    /* Exactly one kill: SIGTERM */
    ASSERT_EQ(mock_kill_call_count, 1);
    ASSERT_EQ(mock_kill_calls[0].pid, CHILD_PID);
    ASSERT_EQ(mock_kill_calls[0].sig, SIGTERM);
    ASSERT_EQ(cchild, 0);
}

TEST(escalates_to_sigkill_when_term_ignored)
{
    setup(0);
    /*
     * SIGTERM_GRACE iterations of WNOHANG returning 0 (still alive),
     * then SIGKILL is sent and child finally reaps.
     * SIGTERM_GRACE = 2, so 2 zeros, then SIGKILL, then reap.
     */
    enqueue_waitpid(0, 0, 0);
    enqueue_waitpid(0, 0, 0);
    enqueue_waitpid(CHILD_PID, 0, 0);
    ssh_kill();

    /* Two kill() calls: SIGTERM then SIGKILL */
    ASSERT_EQ(mock_kill_call_count, 2);
    ASSERT_EQ(mock_kill_calls[0].sig, SIGTERM);
    ASSERT_EQ(mock_kill_calls[1].sig, SIGKILL);
    ASSERT_EQ(cchild, 0);
}

TEST(abandons_after_sigkill_doesnt_reap)
{
    setup(0);
    /*
     * Stuck-in-D child: every waitpid returns 0.
     * After SIGTERM_GRACE + SIGKILL_WAIT iterations (2+2=4) of "still
     * alive", ssh_kill abandons cchild=0 without an extra kill.
     */
    int i;
    for (i = 0; i < 4; i++)
        enqueue_waitpid(0, 0, 0);
    ssh_kill();

    ASSERT_EQ(mock_kill_call_count, 2);  /* TERM + KILL */
    ASSERT_EQ(cchild, 0);                 /* abandoned */
}

TEST(echild_short_circuits)
{
    setup(0);
    /* Already-reaped child: waitpid returns -1 with ECHILD */
    enqueue_waitpid(-1, 0, ECHILD);
    ssh_kill();

    ASSERT_EQ(mock_kill_call_count, 1);  /* only SIGTERM */
    ASSERT_EQ(cchild, 0);
}

TEST(kill_completes_within_4_seconds_for_stuck_child)
{
    /*
     * Even worst-case (child stays alive through both SIGTERM and
     * SIGKILL waits) ssh_kill must not sleep more than 4 seconds
     * total. This is the contract the recent fix established
     * (was 20s before).
     *
     * The hardcoded "4" is intentional: a regression that bumped
     * SIGTERM_GRACE/SIGKILL_WAIT back up would fail this assertion
     * with mock_sleep_total_secs of 6, 12, 20 — independent of
     * the constants' current values.
     */
    setup(0);
    int i;
    for (i = 0; i < 4; i++)
        enqueue_waitpid(0, 0, 0);
    ssh_kill();

    ASSERT_TRUE(mock_sleep_total_secs <= 4);
}

TEST(unexpected_waitpid_error_aborts_cleanly)
{
    setup(0);
    /* EINVAL or other non-EINTR/non-ECHILD error → bail out, cchild=0 */
    enqueue_waitpid(-1, 0, EINVAL);
    ssh_kill();
    /* cchild should be 0 (zeroed on the error-bailout path). */
    ASSERT_EQ(cchild, 0);
}

TEST_SUITE_BEGIN("ssh_kill")
    RUN_TEST(no_child_is_noop);
    RUN_TEST(stderr_fd_is_closed);
    RUN_TEST(fast_reap_no_sigkill);
    RUN_TEST(escalates_to_sigkill_when_term_ignored);
    RUN_TEST(abandons_after_sigkill_doesnt_reap);
    RUN_TEST(echild_short_circuits);
    RUN_TEST(kill_completes_within_4_seconds_for_stuck_child);
    RUN_TEST(unexpected_waitpid_error_aborts_cleanly);
TEST_SUITE_END
