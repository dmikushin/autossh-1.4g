#!/bin/sh
#
# mock_ssh.sh — a configurable stand-in for the real `ssh` binary.
# Used by integration tests via AUTOSSH_PATH=$(pwd)/mock_ssh.sh.
#
# Behaviour selected by MOCK_SSH_MODE:
#   exit-fast-N        : exit code N immediately
#   sleep-then-exit    : sleep $MOCK_SSH_SLEEP, exit $MOCK_SSH_CODE
#   print-pf-fail      : echo "remote port forwarding failed" >&2; exit 255
#   silent-hang        : ignore stderr, sleep forever (until killed)
#   chatty-hang        : print one line to stderr, then sleep forever
#   slow-on-sigterm    : trap SIGTERM, ignore it; sleep forever
#   sigint-immune      : trap SIGINT, ignore it; sleep forever
#
# When invoked autossh passes a long argv. We don't care about the
# args; just record them to MOCK_SSH_LOG (if set) for assertions.

if [ -n "$MOCK_SSH_LOG" ]; then
    echo "[$$] $*" >> "$MOCK_SSH_LOG"
fi

case "$MOCK_SSH_MODE" in
    exit-fast-*)
        exit "${MOCK_SSH_MODE#exit-fast-}"
        ;;
    sleep-then-exit)
        sleep "${MOCK_SSH_SLEEP:-1}"
        exit "${MOCK_SSH_CODE:-0}"
        ;;
    print-pf-fail)
        echo "Warning: remote port forwarding failed for listen port 22322" >&2
        sleep "${MOCK_SSH_SLEEP:-1}"
        exit 255
        ;;
    silent-hang)
        # exec sleep so the shell process IS the sleep — SIGKILL kills it
        # in one shot rather than orphaning a child sleep when bash dies.
        exec sleep 99999
        ;;
    chatty-hang)
        echo "debug1: Connecting to host" >&2
        exec sleep 99999
        ;;
    slow-on-sigterm)
        # Keep bash so we can trap SIGTERM; SIGKILL still works.
        trap '' TERM
        while :; do sleep 60; done
        ;;
    sigint-immune)
        trap '' INT
        while :; do sleep 60; done
        ;;
    *)
        # default: behave like a happy ssh that exits 0
        exit 0
        ;;
esac
