/*
 * Tests for check_ssh_stderr() — drains the SSH stderr pipe,
 * forwards bytes to our own stderr, watches for known fatal
 * patterns, and updates last_stderr_time.
 */

#include <stdio.h>
#include <string.h>
#include <signal.h>
#include <unistd.h>

#include "../framework.h"
#include "../mocks/mocks.h"

extern int    check_ssh_stderr(void);
extern int    ssh_stderr_fd;
extern volatile sig_atomic_t port_fwd_failed;
extern time_t last_stderr_time;

#define FAKE_FD 42

static void setup(void)
{
    mocks_reset();
    ssh_stderr_fd    = FAKE_FD;
    port_fwd_failed  = 0;
    last_stderr_time = 0;
    mock_current_time = 1000;
    mock_write_fd_filter = STDERR_FILENO;  /* only capture forwarded stderr */
}

static void enqueue_read(const char *data, int len)
{
    mock_read_queue[mock_read_qlen].data = data;
    mock_read_queue[mock_read_qlen].len  = len;
    mock_read_queue[mock_read_qlen].err  = 0;
    mock_read_qlen++;
}

TEST(no_fd_returns_zero)
{
    mocks_reset();
    ssh_stderr_fd = -1;
    ASSERT_EQ(check_ssh_stderr(), 0);
    ASSERT_EQ(mock_read_call_count, 0);
}

TEST(benign_data_no_pattern)
{
    setup();
    const char *msg = "Authenticated to host.\n";
    enqueue_read(msg, (int)strlen(msg));
    /* Next read returns EAGAIN to terminate the loop */
    int rc = check_ssh_stderr();
    ASSERT_EQ(rc, 0);
    ASSERT_EQ(port_fwd_failed, 0);
    /* data forwarded to our stderr */
    ASSERT_TRUE(strstr(mock_write_buf, "Authenticated to host") != NULL);
}

TEST(port_forwarding_failed_detected)
{
    setup();
    const char *msg = "Warning: remote port forwarding failed for listen port 22\n";
    enqueue_read(msg, (int)strlen(msg));
    int rc = check_ssh_stderr();
    ASSERT_EQ(rc, 1);
    ASSERT_EQ(port_fwd_failed, 1);
}

TEST(updates_last_stderr_time)
{
    setup();
    last_stderr_time = 0;
    mock_current_time = 12345;
    enqueue_read("hello", 5);
    check_ssh_stderr();
    ASSERT_EQ(last_stderr_time, 12345);
}

TEST(eagain_does_not_update_time)
{
    setup();
    last_stderr_time = 500;
    mock_current_time = 12345;
    /* No reads enqueued → mock returns -1/EAGAIN immediately */
    check_ssh_stderr();
    ASSERT_EQ(last_stderr_time, 500);  /* untouched */
}

TEST(eof_returns_zero)
{
    setup();
    /* len=0 means EOF on the pipe */
    enqueue_read("", 0);
    int rc = check_ssh_stderr();
    ASSERT_EQ(rc, 0);
    ASSERT_EQ(port_fwd_failed, 0);
}

TEST(multiple_chunks_drained)
{
    setup();
    const char *m1 = "first chunk\n";
    const char *m2 = "second chunk\n";
    enqueue_read(m1, (int)strlen(m1));
    enqueue_read(m2, (int)strlen(m2));
    int rc = check_ssh_stderr();
    ASSERT_EQ(rc, 0);
    ASSERT_TRUE(strstr(mock_write_buf, "first chunk")  != NULL);
    ASSERT_TRUE(strstr(mock_write_buf, "second chunk") != NULL);
}

TEST(pattern_in_first_chunk_returns_immediately)
{
    setup();
    const char *m1 = "remote port forwarding failed\n";
    const char *m2 = "would-not-be-read\n";
    enqueue_read(m1, (int)strlen(m1));
    enqueue_read(m2, (int)strlen(m2));
    int rc = check_ssh_stderr();
    ASSERT_EQ(rc, 1);
    ASSERT_EQ(port_fwd_failed, 1);
    /* Second chunk must NOT have been consumed. */
    ASSERT_EQ(mock_read_qpos, 1);
}

TEST_SUITE_BEGIN("check_ssh_stderr")
    RUN_TEST(no_fd_returns_zero);
    RUN_TEST(benign_data_no_pattern);
    RUN_TEST(port_forwarding_failed_detected);
    RUN_TEST(updates_last_stderr_time);
    RUN_TEST(eagain_does_not_update_time);
    RUN_TEST(eof_returns_zero);
    RUN_TEST(multiple_chunks_drained);
    RUN_TEST(pattern_in_first_chunk_returns_immediately);
TEST_SUITE_END
