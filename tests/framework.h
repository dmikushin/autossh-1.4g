/*
 * Minimal in-house C unit test framework for autossh.
 *
 * Usage:
 *
 *     #include "framework.h"
 *
 *     TEST(my_test_name) {
 *         ASSERT_EQ(2 + 2, 4);
 *         ASSERT_STR_EQ(s, "hello");
 *     }
 *
 *     int main(void) {
 *         RUN_TEST(my_test_name);
 *         return TESTS_FAILED ? 1 : 0;
 *     }
 *
 * Each TEST() defines a static function. RUN_TEST() invokes it inside
 * a fork() so that an exit/abort/segfault in one test doesn't kill
 * the entire suite. The parent reports pass/fail and continues.
 */

#ifndef AUTOSSH_TEST_FRAMEWORK_H
#define AUTOSSH_TEST_FRAMEWORK_H

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <sys/wait.h>

#ifdef __cplusplus
extern "C" {
#endif

extern int TESTS_PASSED;
extern int TESTS_FAILED;

/* current test name + first-failure location (per-fork) */
extern const char *_current_test_name;

#define TEST(name) static void test_##name(void)

#define ASSERT_TRUE(expr) do {                                          \
    if (!(expr)) {                                                      \
        fprintf(stderr, "  FAIL %s:%d: ASSERT_TRUE(%s)\n",              \
                __FILE__, __LINE__, #expr);                             \
        exit(1);                                                        \
    }                                                                   \
} while (0)

#define ASSERT_FALSE(expr) ASSERT_TRUE(!(expr))

#define ASSERT_EQ(actual, expected) do {                                \
    long long _a = (long long)(actual);                                 \
    long long _e = (long long)(expected);                               \
    if (_a != _e) {                                                     \
        fprintf(stderr, "  FAIL %s:%d: ASSERT_EQ(%s, %s):"              \
                " got %lld, expected %lld\n",                           \
                __FILE__, __LINE__, #actual, #expected, _a, _e);        \
        exit(1);                                                        \
    }                                                                   \
} while (0)

#define ASSERT_NE(actual, unexpected) do {                              \
    long long _a = (long long)(actual);                                 \
    long long _u = (long long)(unexpected);                             \
    if (_a == _u) {                                                     \
        fprintf(stderr, "  FAIL %s:%d: ASSERT_NE(%s, %s):"              \
                " both are %lld\n",                                     \
                __FILE__, __LINE__, #actual, #unexpected, _a);          \
        exit(1);                                                        \
    }                                                                   \
} while (0)

#define ASSERT_STR_EQ(actual, expected) do {                            \
    const char *_a = (actual);                                          \
    const char *_e = (expected);                                        \
    if (_a == NULL || _e == NULL || strcmp(_a, _e) != 0) {              \
        fprintf(stderr, "  FAIL %s:%d: ASSERT_STR_EQ(%s, %s):"          \
                " got \"%s\", expected \"%s\"\n",                       \
                __FILE__, __LINE__, #actual, #expected,                 \
                _a ? _a : "(null)", _e ? _e : "(null)");                \
        exit(1);                                                        \
    }                                                                   \
} while (0)

#define ASSERT_NOT_NULL(ptr) ASSERT_TRUE((ptr) != NULL)
#define ASSERT_NULL(ptr)     ASSERT_TRUE((ptr) == NULL)

/*
 * Run a single test in a forked child. The child does setup → body
 * → exit(0). On any assertion failure or unexpected death, exit
 * status is non-zero and the parent records the failure.
 *
 * Note: we call __real_waitpid because the test binary is linked
 * with -Wl,--wrap=waitpid (so autossh's internal waitpid is mocked).
 * The framework's own fork/wait must bypass that wrap.
 *
 * forking also gives us isolation of global state in autossh.c
 * (newav, cchild, exit_signalled, …) so tests don't bleed into each
 * other.
 */
extern pid_t __real_waitpid(pid_t pid, int *wstatus, int options);

#define RUN_TEST(name) do {                                             \
    fflush(stdout);                                                     \
    fflush(stderr);                                                     \
    pid_t _pid = fork();                                                \
    if (_pid == 0) {                                                    \
        _current_test_name = #name;                                     \
        test_##name();                                                  \
        exit(0);                                                        \
    } else if (_pid > 0) {                                              \
        int _st = 0;                                                    \
        __real_waitpid(_pid, &_st, 0);                                  \
        if (WIFEXITED(_st) && WEXITSTATUS(_st) == 0) {                  \
            printf("  PASS %s\n", #name);                               \
            TESTS_PASSED++;                                             \
        } else {                                                        \
            printf("  FAIL %s (status=%d, signal=%d)\n", #name,         \
                   WIFEXITED(_st) ? WEXITSTATUS(_st) : -1,              \
                   WIFSIGNALED(_st) ? WTERMSIG(_st) : 0);               \
            TESTS_FAILED++;                                             \
        }                                                               \
    } else {                                                            \
        perror("fork");                                                 \
        TESTS_FAILED++;                                                 \
    }                                                                   \
} while (0)

#define TEST_SUITE_BEGIN(suite_name)                                    \
    int TESTS_PASSED = 0;                                               \
    int TESTS_FAILED = 0;                                               \
    const char *_current_test_name = NULL;                              \
    int main(void) {                                                    \
        printf("== %s ==\n", suite_name);

#define TEST_SUITE_END                                                  \
        printf("Result: %d passed, %d failed\n",                        \
               TESTS_PASSED, TESTS_FAILED);                             \
        return TESTS_FAILED ? 1 : 0;                                    \
    }

#ifdef __cplusplus
}
#endif

#endif /* AUTOSSH_TEST_FRAMEWORK_H */
