/*
 * Tests for add_arg() — argv builder used to assemble the ssh
 * command line. Pure logic; no syscalls. Smoke test for the
 * test infrastructure.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "../framework.h"

/* Symbols from autossh.c we test. */
extern int    newac;
extern char **newav;
extern void   add_arg(char *s);

static void reset_av(void)
{
    /* Don't free; tests are forked per RUN_TEST so leaks die with child. */
    newac = 0;
    newav = NULL;
}

TEST(empty_string_ignored)
{
    reset_av();
    add_arg("");
    ASSERT_EQ(newac, 0);
    ASSERT_NULL(newav);
}

TEST(first_call_allocates_and_terminates)
{
    reset_av();
    add_arg("ssh");
    ASSERT_EQ(newac, 1);
    ASSERT_NOT_NULL(newav);
    ASSERT_STR_EQ(newav[0], "ssh");
    ASSERT_NULL(newav[1]);  /* NULL-terminated for execvp */
}

TEST(multiple_args_in_order)
{
    reset_av();
    add_arg("ssh");
    add_arg("-N");
    add_arg("user@host");
    ASSERT_EQ(newac, 3);
    ASSERT_STR_EQ(newav[0], "ssh");
    ASSERT_STR_EQ(newav[1], "-N");
    ASSERT_STR_EQ(newav[2], "user@host");
    ASSERT_NULL(newav[3]);
}

TEST(realloc_grows_past_initial_capacity)
{
    /*
     * START_AV_SZ in autossh.c is 16. Adding more than 16 args
     * must trigger realloc and preserve all entries.
     */
    reset_av();
    char buf[8];
    int i;
    for (i = 0; i < 25; i++) {
        snprintf(buf, sizeof(buf), "arg%d", i);
        add_arg(buf);
    }
    ASSERT_EQ(newac, 25);
    for (i = 0; i < 25; i++) {
        snprintf(buf, sizeof(buf), "arg%d", i);
        ASSERT_STR_EQ(newav[i], buf);
    }
    ASSERT_NULL(newav[25]);
}

TEST(arg_is_copied_not_referenced)
{
    /*
     * add_arg() must malloc+copy: caller's buffer may be on the
     * stack and disappear before exec.
     */
    reset_av();
    char local[16];
    snprintf(local, sizeof(local), "transient");
    add_arg(local);
    snprintf(local, sizeof(local), "OVERWRITTEN");
    ASSERT_STR_EQ(newav[0], "transient");
}

TEST_SUITE_BEGIN("add_arg")
    RUN_TEST(empty_string_ignored);
    RUN_TEST(first_call_allocates_and_terminates);
    RUN_TEST(multiple_args_in_order);
    RUN_TEST(realloc_grows_past_initial_capacity);
    RUN_TEST(arg_is_copied_not_referenced);
TEST_SUITE_END
