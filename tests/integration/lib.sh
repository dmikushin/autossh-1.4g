#!/bin/sh
#
# Common helpers for integration tests. Each test sources this.
# Provides: setup_test, run_autossh_async, kill_autossh, assert_*.
#
# We intentionally do NOT use `set -e` here. POSIX's set -e
# semantics interact awkwardly with the `if "$@"; then` dispatch
# in run_it_test: simple commands inside the called function may
# or may not trigger an exit depending on subtle context rules,
# and behaviour differs across shells. Each test function instead
# uses explicit `|| return 1` and `return 1` after error checks.

# Resolve to absolute paths relative to this file's directory.
INT_DIR="$(cd "$(dirname "$0")" && pwd)"
TOPDIR="$(cd "$INT_DIR/../.." && pwd)"
AUTOSSH_BIN="$TOPDIR/autossh"
MOCK_SSH="$INT_DIR/mock_ssh.sh"

# Per-test scratch dir
setup_test() {
    TEST_DIR=$(mktemp -d -t "autossh-it.XXXXXX")
    AUTOSSH_LOG="$TEST_DIR/autossh.log"
    MOCK_LOG="$TEST_DIR/mock.log"
    PIDFILE="$TEST_DIR/auto.pid"
    : > "$AUTOSSH_LOG"
    : > "$MOCK_LOG"

    # Common environment for autossh
    export AUTOSSH_PATH="$MOCK_SSH"
    export AUTOSSH_LOGFILE="$AUTOSSH_LOG"
    export MOCK_SSH_LOG="$MOCK_LOG"
}

teardown_test() {
    if [ -n "$AUTO_PID" ] && kill -0 "$AUTO_PID" 2>/dev/null; then
        kill -KILL "$AUTO_PID" 2>/dev/null || true
        wait "$AUTO_PID" 2>/dev/null || true
    fi
    # Clean up any orphaned mock_ssh.sh / sleep processes spawned via
    # this MOCK_SSH_LOG. ssh_kill() may abandon stuck children; we don't
    # want them piling up between tests.
    if [ -n "$MOCK_LOG" ] && [ -s "$MOCK_LOG" ]; then
        # awk over the log to extract our mock pids ("[$$] ..." prefix)
        awk -F'[][]' '/^\[/ {print $2}' "$MOCK_LOG" 2>/dev/null \
            | while read -r pid; do
                kill -KILL "$pid" 2>/dev/null || true
              done
    fi
    if [ -n "$TEST_DIR" ] && [ -d "$TEST_DIR" ]; then
        rm -rf "$TEST_DIR"
    fi
    AUTO_PID=
    TEST_DIR=
}

# Run autossh in the background, store PID in $AUTO_PID.
# Args after "--" are passed to autossh.
run_autossh_async() {
    "$AUTOSSH_BIN" "$@" </dev/null >>"$AUTOSSH_LOG" 2>&1 &
    AUTO_PID=$!
}

# Wait up to N seconds for autossh to exit; return its status, or
# 124 if it never did. Polls at 50ms granularity so sub-second
# exits are accurately measurable.
wait_autossh_with_timeout() {
    timeout="$1"
    waited_ms=0
    timeout_ms=$((timeout * 1000))
    while [ $waited_ms -lt $timeout_ms ]; do
        if ! kill -0 "$AUTO_PID" 2>/dev/null; then
            wait "$AUTO_PID" 2>/dev/null
            return $?
        fi
        sleep 0.05
        waited_ms=$((waited_ms + 50))
    done
    return 124
}

assert_log_contains() {
    if ! grep -qE "$1" "$AUTOSSH_LOG"; then
        echo "  FAIL: log does not match /$1/"
        echo "  --- log ---"
        cat "$AUTOSSH_LOG"
        echo "  -----------"
        return 1
    fi
}

assert_log_not_contains() {
    if grep -qE "$1" "$AUTOSSH_LOG"; then
        echo "  FAIL: log unexpectedly matches /$1/"
        cat "$AUTOSSH_LOG"
        return 1
    fi
}

assert_count_in_log() {
    pattern="$1"
    expected="$2"
    actual=$(grep -cE "$pattern" "$AUTOSSH_LOG" || true)
    if [ "$actual" -ne "$expected" ]; then
        echo "  FAIL: pattern /$pattern/ matched $actual times, expected $expected"
        cat "$AUTOSSH_LOG"
        return 1
    fi
}

# Run a test function and report.
run_it_test() {
    name="$1"
    shift
    setup_test
    if "$@"; then
        echo "  PASS $name"
        teardown_test
        return 0
    else
        echo "  FAIL $name"
        teardown_test
        return 1
    fi
}
