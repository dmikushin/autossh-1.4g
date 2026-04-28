/*
 * Tests for strip_arg() — strips a flag character from a combined
 * short-option argument (e.g. removes 'f' from "-fN" → "-N").
 *
 * The third argument is the option string used to detect flags
 * that take a parameter (matching getopt's "x:" suffix).
 */

#include <stdio.h>
#include <string.h>

#include "../framework.h"

extern void strip_arg(char *arg, char ch, char *opts);

/*
 * autossh's real OPTION_STRING — used so test cases reflect actual
 * stripping behaviour for known flags.
 */
#define OPT_STR "M:V1246ab:c:e:fgi:kl:m:no:p:qstvw:xyACD:E:F:GI:MJKL:NO:PQ:R:S:TW:XYB:"

TEST(plain_f_alone)
{
    char arg[] = "-f";
    strip_arg(arg, 'f', OPT_STR);
    ASSERT_STR_EQ(arg, "");  /* "-" alone → truncated to empty */
}

TEST(f_at_start_of_combined)
{
    char arg[] = "-fN";
    strip_arg(arg, 'f', OPT_STR);
    ASSERT_STR_EQ(arg, "-N");
}

TEST(f_at_end_of_combined)
{
    char arg[] = "-Nf";
    strip_arg(arg, 'f', OPT_STR);
    ASSERT_STR_EQ(arg, "-N");
}

TEST(f_in_middle)
{
    char arg[] = "-NfT";
    strip_arg(arg, 'f', OPT_STR);
    ASSERT_STR_EQ(arg, "-NT");
}

TEST(parameter_taking_flag_stops_strip)
{
    /*
     * "-Mport"  — M takes a parameter (M: in OPT_STR), so anything
     * after it is the value, not flags. strip_arg must leave it.
     */
    char arg[] = "-Mfoo";
    strip_arg(arg, 'f', OPT_STR);
    ASSERT_STR_EQ(arg, "-Mfoo");
}

TEST(double_dash_untouched)
{
    /*
     * "--" is the GNU end-of-options marker; arg[1]=='-' so the
     * leading guard `arg[0]=='-' && arg[1]!='\0'` is true, but
     * the loop runs over '-' '-' and won't strip 'f' (no f present).
     */
    char arg[] = "--";
    strip_arg(arg, 'f', OPT_STR);
    ASSERT_STR_EQ(arg, "--");
}

TEST(non_option_arg_untouched)
{
    /* arg[0] != '-' → guard fails, no modification */
    char arg[] = "user@host";
    strip_arg(arg, 'f', OPT_STR);
    ASSERT_STR_EQ(arg, "user@host");
}

TEST(empty_string)
{
    char arg[] = "";
    strip_arg(arg, 'f', OPT_STR);
    ASSERT_STR_EQ(arg, "");
}

TEST(single_dash_alone)
{
    /* "-" — arg[1]=='\0', guard fails, untouched */
    char arg[] = "-";
    strip_arg(arg, 'f', OPT_STR);
    ASSERT_STR_EQ(arg, "-");
}

TEST(multiple_f_all_stripped)
{
    char arg[] = "-fNf";
    strip_arg(arg, 'f', OPT_STR);
    ASSERT_STR_EQ(arg, "-N");
}

TEST(strip_other_char)
{
    char arg[] = "-Nq";
    strip_arg(arg, 'q', OPT_STR);
    ASSERT_STR_EQ(arg, "-N");
}

TEST_SUITE_BEGIN("strip_arg")
    RUN_TEST(plain_f_alone);
    RUN_TEST(f_at_start_of_combined);
    RUN_TEST(f_at_end_of_combined);
    RUN_TEST(f_in_middle);
    RUN_TEST(parameter_taking_flag_stops_strip);
    RUN_TEST(double_dash_untouched);
    RUN_TEST(non_option_arg_untouched);
    RUN_TEST(empty_string);
    RUN_TEST(single_dash_alone);
    RUN_TEST(multiple_f_all_stripped);
    RUN_TEST(strip_other_char);
TEST_SUITE_END
