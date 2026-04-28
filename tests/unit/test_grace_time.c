/*
 * Tests for grace_time() — backoff between SSH restarts.
 *
 * Behaviour:
 *   - tries counter is static; resets when last_start was long ago
 *   - first N_FAST_TRIES (=5) restarts in quick succession: no sleep
 *   - beyond that: quadratic-ish sleep capped by poll_time
 *   - port_fwd_failed flag forces a fixed PORT_FWD_FAIL_DELAY=5s
 *     sleep and clears itself
 */

#include <stdio.h>
#include <signal.h>
#include <time.h>

#include "../framework.h"
#include "../mocks/mocks.h"

extern void grace_time(time_t last_start);
extern int  poll_time;
extern volatile sig_atomic_t port_fwd_failed;

static void setup(void)
{
    mocks_reset();
    poll_time = 600;
    port_fwd_failed = 0;
    /*
     * grace_time uses a static `tries` counter that persists across
     * calls. Each TEST is forked so we get a fresh static — no
     * cross-test pollution.
     */
}

TEST(first_call_no_sleep)
{
    setup();
    /* last_start = "long ago" → tries reset → no sleep */
    grace_time(mock_current_time - 1000);
    ASSERT_EQ(mock_sleep_total_secs, 0);
}

TEST(rapid_restarts_below_threshold_no_sleep)
{
    /*
     * Five rapid restarts (within min_time): tries goes 1..5.
     * That's NOT > N_FAST_TRIES (5), so no sleep yet.
     */
    setup();
    int i;
    for (i = 0; i < 5; i++) {
        grace_time(mock_current_time);  /* same time = "rapid" */
    }
    /* Excluding any port_fwd_failed sleep */
    ASSERT_EQ(mock_sleep_total_secs, 0);
}

TEST(rapid_restarts_past_threshold_sleeps)
{
    /*
     * Drive tries beyond N_FAST_TRIES. The 7th rapid restart
     * (tries=7) should sleep something.
     */
    setup();
    int i;
    for (i = 0; i < 7; i++)
        grace_time(mock_current_time);
    ASSERT_TRUE(mock_sleep_total_secs > 0);
}

TEST(slow_restarts_reset_tries)
{
    /*
     * After a long-ago last_start, tries resets to 0 → no sleep
     * even after many prior rapid calls.
     */
    setup();
    int i;
    for (i = 0; i < 10; i++)
        grace_time(mock_current_time);
    /* Now last_start is "old" — should reset */
    unsigned int before = mock_sleep_total_secs;
    grace_time(mock_current_time - 10000);
    /* No additional sleep on this call (tries reset to 0) */
    ASSERT_EQ(mock_sleep_total_secs, before);
}

TEST(port_fwd_failed_forces_delay_and_clears)
{
    setup();
    port_fwd_failed = 1;
    grace_time(mock_current_time - 10000);  /* tries=0 path */
    /* PORT_FWD_FAIL_DELAY = 5 */
    ASSERT_EQ(mock_sleep_total_secs, 5);
    ASSERT_EQ(port_fwd_failed, 0);   /* must self-clear */
}

TEST(port_fwd_failed_combines_with_backoff)
{
    setup();
    int i;
    for (i = 0; i < 7; i++)
        grace_time(mock_current_time);
    unsigned int after_backoff = mock_sleep_total_secs;
    port_fwd_failed = 1;
    grace_time(mock_current_time);
    /* Last call adds backoff (some) + 5s for port-fwd flag */
    ASSERT_TRUE(mock_sleep_total_secs >= after_backoff + 5);
    ASSERT_EQ(port_fwd_failed, 0);
}

TEST_SUITE_BEGIN("grace_time")
    RUN_TEST(first_call_no_sleep);
    RUN_TEST(rapid_restarts_below_threshold_no_sleep);
    RUN_TEST(rapid_restarts_past_threshold_sleeps);
    RUN_TEST(slow_restarts_reset_tries);
    RUN_TEST(port_fwd_failed_forces_delay_and_clears);
    RUN_TEST(port_fwd_failed_combines_with_backoff);
TEST_SUITE_END
