/*
 * Tests for sig_catch() — the signal handler.
 *
 * Three behaviours:
 *   - SIGUSR1 sets restart_ssh
 *   - SIGINT/SIGTERM the first time sets exit_signalled
 *   - SIGINT/SIGTERM the second time (when exit_signalled already
 *     set) calls _exit(1) — the double-Ctrl+C force-exit fix
 *   - dolongjmp=1 causes siglongjmp(jumpbuf, sig); dolongjmp=0
 *     does not
 *
 * We exercise the longjmp behaviour by calling sigsetjmp(jumpbuf)
 * ourselves before invoking sig_catch.
 */

#include <stdio.h>
#include <signal.h>
#include <setjmp.h>

#include "../framework.h"
#include "../mocks/mocks.h"

extern void sig_catch(int sig);
extern volatile sig_atomic_t exit_signalled;
extern volatile sig_atomic_t restart_ssh;
extern volatile sig_atomic_t dolongjmp;
extern sigjmp_buf jumpbuf;

static void reset_sig_state(void)
{
    mocks_reset();
    exit_signalled = 0;
    restart_ssh    = 0;
    dolongjmp      = 0;
}

TEST(sigusr1_sets_restart_flag)
{
    reset_sig_state();
    sig_catch(SIGUSR1);
    ASSERT_EQ(restart_ssh, 1);
    ASSERT_EQ(exit_signalled, 0);
}

TEST(sigint_first_sets_exit_flag)
{
    reset_sig_state();
    sig_catch(SIGINT);
    ASSERT_EQ(exit_signalled, 1);
    ASSERT_EQ(restart_ssh, 0);
}

TEST(sigterm_first_sets_exit_flag)
{
    reset_sig_state();
    sig_catch(SIGTERM);
    ASSERT_EQ(exit_signalled, 1);
}

TEST(double_sigint_forces_exit)
{
    reset_sig_state();
    sig_catch(SIGINT);
    ASSERT_EQ(exit_signalled, 1);
    /* Second SIGINT must call _exit(1). Caught via mock_exit trap. */
    MOCK_EXPECT_EXIT({
        sig_catch(SIGINT);
    });
    ASSERT_EQ(mock_exit_status, 1);
}

TEST(double_sigterm_forces_exit)
{
    reset_sig_state();
    sig_catch(SIGTERM);
    MOCK_EXPECT_EXIT({
        sig_catch(SIGTERM);
    });
    ASSERT_EQ(mock_exit_status, 1);
}

TEST(sigint_then_sigterm_forces_exit)
{
    /* Mixed: first SIGINT, then SIGTERM. Second one bails. */
    reset_sig_state();
    sig_catch(SIGINT);
    MOCK_EXPECT_EXIT({
        sig_catch(SIGTERM);
    });
}

TEST(dolongjmp_zero_no_jump)
{
    reset_sig_state();
    /* dolongjmp=0 → handler returns normally, we get past the call */
    sig_catch(SIGINT);
    ASSERT_EQ(dolongjmp, 0);
    /* If we got here, no longjmp happened. Good. */
}

TEST(dolongjmp_one_does_longjmp)
{
    reset_sig_state();
    int val = sigsetjmp(jumpbuf, 1);
    if (val == 0) {
        dolongjmp = 1;
        sig_catch(SIGINT);
        /* siglongjmp should have happened — we shouldn't reach here */
        ASSERT_TRUE(0 && "expected siglongjmp but returned");
    }
    /* longjmp returned: val == sig */
    ASSERT_EQ(val, SIGINT);
    /* sig_catch must have cleared dolongjmp before jumping */
    ASSERT_EQ(dolongjmp, 0);
    ASSERT_EQ(exit_signalled, 1);
}

TEST(sigusr1_with_dolongjmp_jumps_too)
{
    reset_sig_state();
    int val = sigsetjmp(jumpbuf, 1);
    if (val == 0) {
        dolongjmp = 1;
        sig_catch(SIGUSR1);
        ASSERT_TRUE(0 && "expected siglongjmp");
    }
    ASSERT_EQ(val, SIGUSR1);
    ASSERT_EQ(restart_ssh, 1);
}

TEST(unhandled_signal_no_flags_changed)
{
    /*
     * SIGCHLD is in the sigaction set but sig_catch only sets flags
     * for USR1/INT/TERM. Other signals just optionally longjmp.
     */
    reset_sig_state();
    sig_catch(SIGCHLD);
    ASSERT_EQ(restart_ssh, 0);
    ASSERT_EQ(exit_signalled, 0);
}

TEST_SUITE_BEGIN("sig_catch")
    RUN_TEST(sigusr1_sets_restart_flag);
    RUN_TEST(sigint_first_sets_exit_flag);
    RUN_TEST(sigterm_first_sets_exit_flag);
    RUN_TEST(double_sigint_forces_exit);
    RUN_TEST(double_sigterm_forces_exit);
    RUN_TEST(sigint_then_sigterm_forces_exit);
    RUN_TEST(dolongjmp_zero_no_jump);
    RUN_TEST(dolongjmp_one_does_longjmp);
    RUN_TEST(sigusr1_with_dolongjmp_jumps_too);
    RUN_TEST(unhandled_signal_no_flags_changed);
TEST_SUITE_END
