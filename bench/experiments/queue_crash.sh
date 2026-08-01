#!/usr/bin/env bash
#
# Phase 1.3 — what happens to a job whose worker dies mid-execution?
#
# Not a panic: the framework's panic boundary catches those and settles
# them as ordinary failures, which already works. This is abrupt death —
# `abort()`, and by extension OOM kills, segfaults and `docker kill` —
# where the process vanishes without settling anything.
#
# The reclaim path is `pop`'s "reserved_until has passed" predicate. The
# question is whether the job's attempt count survives that path, because
# the count is what eventually dead-letters a job.
#
#   PASS — `attempts` advances on each reclaim, so a job that kills its
#          worker is dead-lettered after max_tries and stops.
#   FAIL — `attempts` stays flat. The job is immortal: it kills each worker
#          that claims it, is reclaimed unchanged, and kills the next one,
#          for as long as the deployment keeps restarting workers.
#
# Exit codes: 0 PASS, 1 FAIL, 2 setup failure.

set -euo pipefail
# shellcheck source=lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

: "${VIS:=10}"        # visibility timeout — how soon a lost job is reclaimable
: "${ROUNDS:=3}"      # how many workers to feed it
: "${DEADLINE:=60}"   # per-round wait for the worker to die

RESULTS="$(results_dir queue-crash)"
echo "==> results: $RESULTS"

require_stack
stop_scaled worker

echo "==> clearing jobs and failed_jobs"
bench_sql "TRUNCATE jobs;" >/dev/null
bench_sql "TRUNCATE failed_jobs;" >/dev/null

echo "==> enqueueing one BenchAbort job"
console bench:enqueue-abort --marker "phase-1.3" >"$RESULTS/enqueue.log" 2>&1

JOB_ID="$(bench_sql "SELECT id FROM jobs LIMIT 1;" | tr -d '[:space:]')"
if [[ -z "$JOB_ID" ]]; then
    echo "SETUP FAILURE: nothing landed in the jobs table" | tee "$RESULTS/verdict.txt"
    cat "$RESULTS/enqueue.log"
    exit 2
fi
echo "==> job id: $JOB_ID"

printf 'round\tworker_exit\tattempts\tstill_queued\tfailed_jobs\n' >"$RESULTS/rounds.tsv"

for round in $(seq 1 "$ROUNDS"); do
    echo "==> round $round/$ROUNDS — starting a worker (visibility timeout ${VIS}s)"
    # `run` rather than `up`, because this needs a command override: the
    # service's default visibility timeout is 60s, which would make each
    # round wait a minute for the reclaim to become possible.
    CID="$(compose --profile worker run -d worker app queue:work --visibility-timeout "$VIS" 2>>"$RESULTS/run.log")"
    CID="$(printf '%s' "$CID" | tr -d '[:space:]')"
    if [[ -z "$CID" ]]; then
        echo "SETUP FAILURE: worker container did not start in round $round" \
            | tee "$RESULTS/verdict.txt"
        cat "$RESULTS/run.log"
        exit 2
    fi

    WORKER_EXIT="timeout"
    for _ in $(seq 1 "$DEADLINE"); do
        if [[ "$(docker inspect -f '{{.State.Running}}' "$CID" 2>/dev/null)" != "true" ]]; then
            WORKER_EXIT="$(docker inspect -f '{{.State.ExitCode}}' "$CID")"
            break
        fi
        sleep 1
    done

    docker logs "$CID" >"$RESULTS/worker-$round.log" 2>&1 || true
    docker rm -f "$CID" >/dev/null 2>&1 || true

    if [[ "$WORKER_EXIT" == "timeout" ]]; then
        echo "    worker survived ${DEADLINE}s — the job did not kill it"
    else
        # 134 is 128+6: SIGABRT, which is what abort() raises. Anything else
        # means the process died of something other than the job.
        echo "    worker exited $WORKER_EXIT"
    fi

    # Wait out the reservation so the row is claimable again and its
    # bookkeeping has settled before we read it.
    sleep "$((VIS + 2))"

    ATTEMPTS="$(bench_sql "SELECT attempts FROM jobs WHERE id = '$JOB_ID';" | tr -d '[:space:]')"
    STILL_QUEUED=yes
    if [[ -z "$ATTEMPTS" ]]; then
        STILL_QUEUED=no
        ATTEMPTS="-"
    fi
    FAILED="$(bench_sql "SELECT COUNT(*) FROM failed_jobs;" | tr -d '[:space:]')"

    printf '%s\t%s\t%s\t%s\t%s\n' \
        "$round" "$WORKER_EXIT" "$ATTEMPTS" "$STILL_QUEUED" "$FAILED" >>"$RESULTS/rounds.tsv"
    echo "    attempts=$ATTEMPTS still_queued=$STILL_QUEUED failed_jobs=$FAILED"

    if [[ "$STILL_QUEUED" == "no" ]]; then
        echo "    job left the queue after $round round(s)"
        break
    fi
done

stop_scaled worker

FINAL_ATTEMPTS="$(bench_sql "SELECT attempts FROM jobs WHERE id = '$JOB_ID';" | tr -d '[:space:]')"
FINAL_FAILED="$(bench_sql "SELECT COUNT(*) FROM failed_jobs;" | tr -d '[:space:]')"

{
    echo "experiment: 1.3-queue-crash"
    echo "visibility_timeout_s: $VIS"
    echo "rounds: $ROUNDS"
    echo "final_attempts: ${FINAL_ATTEMPTS:--}"
    echo "final_failed_jobs: $FINAL_FAILED"
} | tee "$RESULTS/verdict.txt"

echo "==> per-round:"
cat "$RESULTS/rounds.tsv"

# The job leaving the queue is only a pass if it left by dead-lettering.
# Vanishing without a failed_jobs row would be worse than looping: the work
# would be silently lost.
if [[ -z "$FINAL_ATTEMPTS" ]]; then
    if [[ "$FINAL_FAILED" != "0" ]]; then
        echo "result: PASS — the job dead-lettered after repeated worker loss" \
            | tee -a "$RESULTS/verdict.txt"
        exit 0
    fi
    {
        echo "result: FAIL"
        echo "the job left the queue without landing in failed_jobs — work lost silently."
    } | tee -a "$RESULTS/verdict.txt"
    exit 1
fi

if [[ "$FINAL_ATTEMPTS" == "0" ]]; then
    {
        echo "result: FAIL"
        echo "After $ROUNDS workers were killed by this job, its attempt count is still 0."
        echo "Reclaim after a lost reservation preserves attempts, so a job that crashes"
        echo "its worker is never dead-lettered: it kills each worker that claims it, is"
        echo "reclaimed unchanged, and kills the next one. A deployment that restarts"
        echo "workers automatically will do this forever."
    } | tee -a "$RESULTS/verdict.txt"
    exit 1
fi

{
    echo "result: FAIL"
    echo "attempts reached $FINAL_ATTEMPTS after $ROUNDS rounds but the job is still queued;"
    echo "it advances, so it will eventually dead-letter, but it had not by the end of"
    echo "this run. Re-run with a larger ROUNDS to find where it stops."
} | tee -a "$RESULTS/verdict.txt"
exit 1
