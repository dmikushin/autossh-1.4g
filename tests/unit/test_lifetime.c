/*
 * Tests for exceeded_lifetime() and clear_alarm_timer().
 *
 * exceeded_lifetime() compares (mock) current time against
 * pid_start_time + max_lifetime. Both globals live in autossh.c.
 *
 * clear_alarm_timer() is a thin wrapper around alarm(0); we verify
 * it returns the previously pending value and zeroes the alarm.
 */

#include <stdio.h>
#include <time.h>

#include "../framework.h"
#include "../mocks/mocks.h"

extern int    exceeded_lifetime(void);
extern unsigned int clear_alarm_timer(void);
extern double max_lifetime;
extern time_t pid_start_time;

/* ---- exceeded_lifetime ---- */

TEST(lifetime_zero_never_expires)
{
    mocks_reset();
    max_lifetime   = 0;
    pid_start_time = mock_current_time;
    mock_current_time += 1000000;  /* far future */
    ASSERT_EQ(exceeded_lifetime(), 0);
}

TEST(lifetime_within_limit)
{
    mocks_reset();
    max_lifetime    = 100;
    pid_start_time  = mock_current_time;
    mock_current_time += 50;   /* halfway through */
    ASSERT_EQ(exceeded_lifetime(), 0);
}

TEST(lifetime_exactly_at_limit)
{
    mocks_reset();
    max_lifetime    = 100;
    pid_start_time  = mock_current_time;
    mock_current_time += 100;  /* exactly at the limit */
    ASSERT_EQ(exceeded_lifetime(), 1);
}

TEST(lifetime_exceeded)
{
    mocks_reset();
    max_lifetime    = 60;
    pid_start_time  = mock_current_time;
    mock_current_time += 600;
    ASSERT_EQ(exceeded_lifetime(), 1);
}

/* ---- clear_alarm_timer ---- */

TEST(clear_returns_pending_value)
{
    mocks_reset();
    /* Simulate that an alarm was previously set for 42s. */
    mock_alarm_pending = 42;
    unsigned int got = clear_alarm_timer();
    ASSERT_EQ(got, 42);
    /* clear_alarm_timer() called alarm(0): mock should record that. */
    ASSERT_EQ(mock_alarm_last_value, 0);
}

TEST(clear_when_no_alarm_returns_zero)
{
    mocks_reset();
    mock_alarm_pending = 0;
    ASSERT_EQ(clear_alarm_timer(), 0);
    ASSERT_EQ(mock_alarm_last_value, 0);
}

TEST_SUITE_BEGIN("exceeded_lifetime + clear_alarm_timer")
    RUN_TEST(lifetime_zero_never_expires);
    RUN_TEST(lifetime_within_limit);
    RUN_TEST(lifetime_exactly_at_limit);
    RUN_TEST(lifetime_exceeded);
    RUN_TEST(clear_returns_pending_value);
    RUN_TEST(clear_when_no_alarm_returns_zero);
TEST_SUITE_END
