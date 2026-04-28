/*
 * Tests for get_env_args() — populates global config from
 * AUTOSSH_* environment variables.
 *
 * We mock getenv() to inject specific values without touching
 * the real environment. xerrlog() (called on invalid input) is
 * trapped via _exit mock.
 */

#include <stdio.h>
#include <string.h>
#include <syslog.h>

#include "../framework.h"
#include "../mocks/mocks.h"

extern void   get_env_args(void);
extern char  *ssh_path;
extern int    poll_time;
extern int    first_poll_time;
extern double gate_time;
extern int    max_start;
extern double max_lifetime;
extern double max_session;
extern char  *echo_message;
extern char  *env_port;
extern char  *pid_file_name;
extern char  *mhost;
extern int    loglevel;

/*
 * Reset all globals get_env_args writes to known defaults so each
 * test starts clean. (Globals persist between tests when not
 * forked, but RUN_TEST() forks anyway — this is belt + suspenders.)
 */
static void reset_globals(void)
{
    ssh_path        = (char *)"/usr/bin/ssh";
    poll_time       = 600;
    first_poll_time = 600;
    gate_time       = 30;
    max_start       = -1;
    max_lifetime    = 0;
    max_session     = 0;
    echo_message    = (char *)"";
    env_port        = NULL;
    pid_file_name   = NULL;
    /* mhost is read elsewhere (in main, not get_env_args) */
    loglevel        = LOG_INFO;
    mock_clearenv();
}

TEST(autossh_path_overrides_default)
{
    reset_globals();
    mock_setenv("AUTOSSH_PATH", "/opt/bin/ssh");
    get_env_args();
    ASSERT_STR_EQ(ssh_path, "/opt/bin/ssh");
}

TEST(autossh_loglevel_valid)
{
    reset_globals();
    mock_setenv("AUTOSSH_LOGLEVEL", "7");  /* LOG_DEBUG */
    get_env_args();
    ASSERT_EQ(loglevel, 7);
}

TEST(autossh_loglevel_invalid_calls_exit)
{
    reset_globals();
    mock_setenv("AUTOSSH_LOGLEVEL", "abc");
    MOCK_EXPECT_EXIT({
        get_env_args();
    });
    ASSERT_EQ(mock_exit_status, 1);  /* xerrlog → _exit(1) */
}

TEST(autossh_poll_valid)
{
    reset_globals();
    mock_setenv("AUTOSSH_POLL", "120");
    get_env_args();
    ASSERT_EQ(poll_time, 120);
    /*
     * AUTOSSH_FIRST_POLL not set, so first_poll_time should
     * inherit poll_time per the env-arg parser.
     */
    ASSERT_EQ(first_poll_time, 120);
}

TEST(autossh_poll_zero_invalid)
{
    reset_globals();
    mock_setenv("AUTOSSH_POLL", "0");
    MOCK_EXPECT_EXIT({
        get_env_args();
    });
}

TEST(autossh_first_poll_explicit)
{
    reset_globals();
    mock_setenv("AUTOSSH_POLL", "600");
    mock_setenv("AUTOSSH_FIRST_POLL", "30");
    get_env_args();
    ASSERT_EQ(poll_time, 600);
    ASSERT_EQ(first_poll_time, 30);
}

TEST(autossh_gatetime)
{
    reset_globals();
    mock_setenv("AUTOSSH_GATETIME", "0");
    get_env_args();
    ASSERT_EQ((int)gate_time, 0);
}

TEST(autossh_maxstart)
{
    reset_globals();
    mock_setenv("AUTOSSH_MAXSTART", "5");
    get_env_args();
    ASSERT_EQ(max_start, 5);
}

TEST(autossh_maxstart_minus_one_means_unlimited)
{
    reset_globals();
    mock_setenv("AUTOSSH_MAXSTART", "-1");
    get_env_args();
    ASSERT_EQ(max_start, -1);
}

TEST(autossh_maxstart_invalid)
{
    reset_globals();
    mock_setenv("AUTOSSH_MAXSTART", "-5");  /* < -1 rejected */
    MOCK_EXPECT_EXIT({
        get_env_args();
    });
}

TEST(autossh_max_session)
{
    reset_globals();
    mock_setenv("AUTOSSH_MAX_SESSION", "60");
    get_env_args();
    ASSERT_EQ((int)max_session, 60);
}

TEST(autossh_port_overrides)
{
    reset_globals();
    mock_setenv("AUTOSSH_PORT", "20000");
    get_env_args();
    ASSERT_NOT_NULL(env_port);
    ASSERT_STR_EQ(env_port, "20000");
}

TEST(autossh_port_empty_string_ignored)
{
    reset_globals();
    mock_setenv("AUTOSSH_PORT", "");
    get_env_args();
    /* per the source: empty string does not assign env_port */
    ASSERT_NULL(env_port);
}

TEST(autossh_pidfile)
{
    reset_globals();
    mock_setenv("AUTOSSH_PIDFILE", "/run/autossh.pid");
    get_env_args();
    ASSERT_STR_EQ(pid_file_name, "/run/autossh.pid");
}

TEST(autossh_message)
{
    reset_globals();
    mock_setenv("AUTOSSH_MESSAGE", "hello-from-host");
    get_env_args();
    ASSERT_STR_EQ(echo_message, "hello-from-host");
}

TEST(autossh_maxlifetime)
{
    reset_globals();
    mock_setenv("AUTOSSH_MAXLIFETIME", "3600");
    get_env_args();
    ASSERT_EQ((int)max_lifetime, 3600);
}

TEST(autossh_maxlifetime_caps_poll_time)
{
    /*
     * If poll_time > max_lifetime the env handler clamps poll_time
     * down to avoid waiting past lifetime expiry.
     */
    reset_globals();
    mock_setenv("AUTOSSH_POLL", "600");
    mock_setenv("AUTOSSH_MAXLIFETIME", "60");
    get_env_args();
    ASSERT_EQ((int)max_lifetime, 60);
    ASSERT_EQ(poll_time, 60);
}

TEST(no_env_vars_leaves_defaults)
{
    reset_globals();
    get_env_args();
    ASSERT_STR_EQ(ssh_path, "/usr/bin/ssh");
    ASSERT_EQ(poll_time, 600);
    ASSERT_EQ((int)gate_time, 30);
    ASSERT_EQ(max_start, -1);
    ASSERT_EQ((int)max_lifetime, 0);
    ASSERT_EQ((int)max_session, 0);
}

TEST_SUITE_BEGIN("get_env_args")
    RUN_TEST(autossh_path_overrides_default);
    RUN_TEST(autossh_loglevel_valid);
    RUN_TEST(autossh_loglevel_invalid_calls_exit);
    RUN_TEST(autossh_poll_valid);
    RUN_TEST(autossh_poll_zero_invalid);
    RUN_TEST(autossh_first_poll_explicit);
    RUN_TEST(autossh_gatetime);
    RUN_TEST(autossh_maxstart);
    RUN_TEST(autossh_maxstart_minus_one_means_unlimited);
    RUN_TEST(autossh_maxstart_invalid);
    RUN_TEST(autossh_max_session);
    RUN_TEST(autossh_port_overrides);
    RUN_TEST(autossh_port_empty_string_ignored);
    RUN_TEST(autossh_pidfile);
    RUN_TEST(autossh_message);
    RUN_TEST(autossh_maxlifetime);
    RUN_TEST(autossh_maxlifetime_caps_poll_time);
    RUN_TEST(no_env_vars_leaves_defaults);
TEST_SUITE_END
