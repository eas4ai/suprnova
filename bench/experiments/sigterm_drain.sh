#!/usr/bin/env bash
#
# Phase 1.4 — does SIGTERM drain the queue worker, or kill it?
#
# Claim under test: the daemons in `app/mod.rs` select only on
# `tokio::signal::ctrl_c()`. SIGINT is what Ctrl-C sends; SIGTERM is what
# `docker stop`, Coolify, systemd, and Kubernetes send. If nothing catches
# SIGTERM, the process dies on the default disposition and the graceful
# drain never runs at all.
#
# Run inside Docker rather than against a local `cargo run`, because
# `docker stop` IS the production stop path. A shell sending `kill -TERM`
# would be a reconstruction of it; this is the thing itself.
#
#   PASS — the in-flight job runs to completion and the container exits 0
#          inside the grace window.
#   FAIL — the container dies on the default SIGTERM disposition (143) with
#          the job unfinished, or has to be SIGKILLed (137).
#
# Exit codes: 0 PASS, 1 FAIL, 2 setup failure.

set -euo pipefail
# shellcheck source=lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

: "${SLEEP_JOB_SECS:=20}"   # how long the in-flight job occupies the worker
: "${SIGNAL_AFTER:=5}"      # let the worker claim the job first
: "${GRACE:=40}"            # docker stop's window before it escalates to KILL

RESULTS="$(results_dir sigterm)"
echo "==> results: $RESULTS"

require_stack

# A worker left over from an earlier run would claim the job before this
# one starts, and the experiment would then measure a container we are not
# signalling.
stop_scaled worker

echo "==> enqueueing a ${SLEEP_JOB_SECS}s job"
console bench:enqueue-sleep --seconds "$SLEEP_JOB_SECS" >"$RESULTS/enqueue.log" 2>&1

echo "==> starting one queue:work container"
# Visibility timeout comfortably longer than the job, so a reclaim by
# another worker cannot be mistaken for the original finishing.
compose --profile worker up -d --scale worker=1 worker >"$RESULTS/up.log" 2>&1
WORKER="$(compose --profile worker ps -q worker | head -1)"
if [[ -z "$WORKER" ]]; then
    echo "SETUP FAILURE: no worker container started" | tee "$RESULTS/verdict.txt"
    cat "$RESULTS/up.log"
    exit 2
fi
echo "==> worker container: ${WORKER:0:12}"

# The signal has to land while the job is genuinely in flight; sending it
# before the worker has claimed anything would prove nothing.
sleep "$SIGNAL_AFTER"
if [[ "$(docker inspect -f '{{.State.Running}}' "$WORKER")" != "true" ]]; then
    echo "SETUP FAILURE: worker exited before we could signal it" | tee "$RESULTS/verdict.txt"
    docker logs "$WORKER" >"$RESULTS/worker.log" 2>&1 || true
    cat "$RESULTS/worker.log"
    exit 2
fi

echo "==> docker stop (SIGTERM, ${GRACE}s grace)"
SENT_AT=$(date +%s.%N)
docker stop -t "$GRACE" "$WORKER" >/dev/null
EXITED_AT=$(date +%s.%N)
LATENCY=$(awk -v a="$SENT_AT" -v b="$EXITED_AT" 'BEGIN{printf "%.2f", b-a}')

EXIT_CODE="$(docker inspect -f '{{.State.ExitCode}}' "$WORKER")"
docker logs "$WORKER" >"$RESULTS/worker.log" 2>&1 || true

# The log line is the only direct evidence the job ran to completion; exit
# code alone cannot distinguish "drained" from "exited before claiming".
JOB_FINISHED=no
if grep -q "bench sleep job finished" "$RESULTS/worker.log"; then
    JOB_FINISHED=yes
fi

# 143 is 128+15: the process died on the default SIGTERM disposition, i.e.
# nothing was listening. 137 is 128+9: it ignored SIGTERM until docker
# escalated. Both are failures, and they fail differently.
case "$EXIT_CODE" in
    0)   DISPOSITION="handler ran and the process exited cleanly" ;;
    143) DISPOSITION="default SIGTERM disposition — no handler installed" ;;
    137) DISPOSITION="SIGKILL after the grace window — SIGTERM was ignored" ;;
    *)   DISPOSITION="unexpected exit code" ;;
esac

{
    echo "experiment: 1.4-sigterm-drain"
    echo "job_seconds: $SLEEP_JOB_SECS"
    echo "signal_after_s: $SIGNAL_AFTER"
    echo "grace_s: $GRACE"
    echo "exit_code: $EXIT_CODE"
    echo "exit_latency_s: $LATENCY"
    echo "job_finished: $JOB_FINISHED"
    echo "disposition: $DISPOSITION"
} | tee "$RESULTS/verdict.txt"

stop_scaled worker

if [[ "$EXIT_CODE" -eq 0 && "$JOB_FINISHED" == "yes" ]]; then
    echo "result: PASS — job completed, exited 0 after ${LATENCY}s" | tee -a "$RESULTS/verdict.txt"
    exit 0
fi

{
    echo "result: FAIL"
    echo "SIGTERM did not drain the worker. Every containerised deployment"
    echo "stops with SIGTERM, so the graceful drain never runs there — the"
    echo "in-flight job stays reserved until its visibility timeout expires,"
    echo "and the work is redone by whoever claims it next."
} | tee -a "$RESULTS/verdict.txt"
exit 1
