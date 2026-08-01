#!/usr/bin/env bash
#
# Phase 1.4 - does SIGTERM drain the queue worker, or kill it?
#
# Claim under test: the daemons in `app/mod.rs` select only on
# `tokio::signal::ctrl_c()`. SIGINT is what Ctrl-C sends; SIGTERM is what
# `docker stop`, Coolify, systemd, and Kubernetes send. If nothing catches
# SIGTERM, the process dies on the default disposition and the graceful
# drain never runs at all.
#
#   PASS - the in-flight job completes or is cleanly nacked, and the
#          process exits 0 within the grace window.
#   FAIL - immediate termination (exit 143 = 128+SIGTERM), job left
#          reserved until its visibility timeout expires.
#
# Requires a job in the dogfood app that sleeps long enough to still be
# running when the signal lands. See bench/README.md.

set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
RESULTS="${RESULTS_DIR:-$REPO_ROOT/bench/results/$(date -u +%Y%m%dT%H%M%SZ)-sigterm}"
mkdir -p "$RESULTS"

: "${SLEEP_JOB_SECS:=20}"   # how long the in-flight job runs
: "${SIGNAL_AFTER:=3}"      # let the worker pick the job up first
: "${GRACE:=30}"            # how long we wait before calling it hung

echo "==> results: $RESULTS"

echo "==> enqueueing a ${SLEEP_JOB_SECS}s job"
cargo run --release -p app --bin console -- bench:sleep-job \
    --seconds "$SLEEP_JOB_SECS" >"$RESULTS/enqueue.log" 2>&1

echo "==> starting queue:work"
cargo run --release -p app --bin app -- queue:work >"$RESULTS/worker.log" 2>&1 &
WORKER=$!

# The signal has to land while the job is genuinely in flight; sending it
# before the worker has claimed anything would prove nothing.
sleep "$SIGNAL_AFTER"
if ! kill -0 "$WORKER" 2>/dev/null; then
    echo "SETUP FAILURE: worker exited before we could signal it" | tee "$RESULTS/verdict.txt"
    cat "$RESULTS/worker.log"
    exit 2
fi

echo "==> sending SIGTERM to $WORKER"
SENT_AT=$(date +%s.%N)
kill -TERM "$WORKER"

EXIT_CODE=""
for _ in $(seq 1 "$((GRACE * 10))"); do
    if ! kill -0 "$WORKER" 2>/dev/null; then
        wait "$WORKER" 2>/dev/null && EXIT_CODE=0 || EXIT_CODE=$?
        break
    fi
    sleep 0.1
done
EXITED_AT=$(date +%s.%N)
LATENCY=$(awk -v a="$SENT_AT" -v b="$EXITED_AT" 'BEGIN{printf "%.2f", b-a}')

if [[ -z "$EXIT_CODE" ]]; then
    kill -KILL "$WORKER" 2>/dev/null || true
    {
        echo "result: FAIL"
        echo "reason: still running ${GRACE}s after SIGTERM; killed"
        echo "exit_latency_s: >${GRACE}"
    } | tee "$RESULTS/verdict.txt"
    exit 1
fi

# 143 is 128+15: the shell's way of saying the process died on the default
# SIGTERM disposition, i.e. nothing was listening for it.
DRAINED="unknown"
if [[ "$EXIT_CODE" -eq 0 ]]; then
    DRAINED="yes"
elif [[ "$EXIT_CODE" -eq 143 ]]; then
    DRAINED="no - default SIGTERM disposition, no handler installed"
fi

{
    echo "experiment: 1.4-sigterm-drain"
    echo "job_seconds: $SLEEP_JOB_SECS"
    echo "signal_after_s: $SIGNAL_AFTER"
    echo "exit_code: $EXIT_CODE"
    echo "exit_latency_s: $LATENCY"
    echo "handler_installed: $DRAINED"
} | tee "$RESULTS/verdict.txt"

# A worker that drains waits for the job; one that dies on the default
# disposition exits almost instantly. The latency is the tell, and the
# exit code corroborates it.
if [[ "$EXIT_CODE" -eq 0 ]]; then
    echo "result: PASS - exited 0 after ${LATENCY}s" | tee -a "$RESULTS/verdict.txt"
    exit 0
fi

{
    echo "result: FAIL"
    echo "SIGTERM was not handled. Every containerised deployment stops with"
    echo "SIGTERM, so the graceful drain never runs there - the in-flight job"
    echo "stays reserved until its visibility timeout expires."
} | tee -a "$RESULTS/verdict.txt"
exit 1
