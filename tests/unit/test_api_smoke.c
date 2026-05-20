/*
 * Smoke tests for the public libautossh C API.
 */

#include <stdint.h>

#include "autossh.h"
#include "../framework.h"

TEST(config_init_sets_defaults)
{
    autossh_session_config cfg;
    autossh_session_config_init(&cfg);

    ASSERT_EQ(cfg.struct_size, sizeof(cfg));
    ASSERT_EQ(cfg.target_port, 22);
    ASSERT_EQ(cfg.start_timeout_ms, 15000);
    ASSERT_EQ(cfg.max_start, -1);
}

TEST(new_rejects_missing_destination)
{
    autossh_session_config cfg;
    autossh_session_config_init(&cfg);
    cfg.target_host = "127.0.0.1";

    ASSERT_NULL(autossh_session_new(&cfg));
}

TEST(new_state_and_accessors)
{
    autossh_session_config cfg;
    autossh_session_config_init(&cfg);
    cfg.ssh_destination = "jump-alias";
    cfg.target_host = "127.0.0.1";
    cfg.target_port = 2222;
    cfg.bind_host = "127.0.0.1";
    cfg.local_port = 43210;
    cfg.flags = AUTOSSH_SESSION_MONITOR_DISABLED;

    autossh_session *session = autossh_session_new(&cfg);
    ASSERT_NOT_NULL(session);
    ASSERT_EQ(autossh_session_state(session), AUTOSSH_STATE_INIT);
    ASSERT_EQ(autossh_session_pid(session), 0);
    ASSERT_EQ(autossh_session_local_port(session), 43210);
    ASSERT_STR_EQ(autossh_session_bind_host(session), "127.0.0.1");
    ASSERT_STR_EQ(autossh_session_last_error(session), "");

    ASSERT_EQ(autossh_session_stop(session), 0);
    ASSERT_EQ(autossh_session_state(session), AUTOSSH_STATE_STOPPED);
    autossh_session_free(session);
}

TEST_SUITE_BEGIN("libautossh_api")
    RUN_TEST(config_init_sets_defaults);
    RUN_TEST(new_rejects_missing_destination);
    RUN_TEST(new_state_and_accessors);
TEST_SUITE_END
