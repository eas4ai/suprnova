#!/usr/bin/env bash
#
# Phase 1.5 — can two workers claim the same job?
#
# N jobs, M workers, one database. Each job inserts its id into
# `bench_job_runs`, which carries a UNIQUE index on `job_id`, so a double
# claim fails at the database the moment it happens rather than being
# inferred from counts afterwards.
#
# The distinction that matters in the verdict:
#
#   duplicate  — two workers ran the same job. A claiming defect.
#   shortfall  — fewer jobs ran than were enqueued. Not a claiming defect;
#                either the run was cut short or jobs are being lost.
#
# Reporting them as one number would let the second be read as the first.
#
#   PASS — exactly COUNT distinct jobs, no duplicates.
#   FAIL — anything else.
#
# Exit codes: 0 PASS, 1 FAIL, 2 setup failure.

set -euo pipefail
# shellcheck source=lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

: "${COUNT:=1000}"      # jobs to enqueue
: "${WORKERS:=8}"       # concurrent worker containers
: "${DEADLINE:=300}"    # seconds to wait for the queue to drain

RESULTS="$(results_dir claim-exclusivity)"
echo "==> results: $RESULTS"

require_stack
stop_scaled worker

# Previous runs would be counted as this one's results. Truncating is the
# whole reason this table exists, so there is nothing here to preserve.
echo "==> clearing bench_job_runs"
bench_sql "TRUNCATE bench_job_runs;" >/dev/null

echo "==> enqueueing $COUNT jobs"
console bench:enqueue-records --count "$COUNT" >"$RESULTS/enqueue.log" 2>&1

QUEUED="$(bench_sql "SELECT COUNT(*) FROM jobs;" | tr -d '[:space:]')"
if [[ "$QUEUED" != "$COUNT" ]]; then
    echo "SETUP FAILURE: enqueued $COUNT but jobs table holds $QUEUED" | tee "$RESULTS/verdict.txt"
    cat "$RESULTS/enqueue.log"
    exit 2
fi

# Workers start only once the whole batch is queued, so every worker races
# for the same backlog. Starting them first would let the first worker
# drain the queue as it filled, which tests nothing about contention.
echo "==> starting $WORKERS workers"
STARTED_AT=$(date +%s.%N)
compose --profile worker up -d --scale worker="$WORKERS" worker >"$RESULTS/up.log" 2>&1

echo "==> waiting for the queue to drain (deadline ${DEADLINE}s)"
DRAINED=no
for _ in $(seq 1 "$DEADLINE"); do
    REMAINING="$(bench_sql "SELECT COUNT(*) FROM jobs;" | tr -d '[:space:]')"
    if [[ "$REMAINING" == "0" ]]; then
        DRAINED=yes
        break
    fi
    sleep 1
done
FINISHED_AT=$(date +%s.%N)
ELAPSED=$(awk -v a="$STARTED_AT" -v b="$FINISHED_AT" 'BEGIN{printf "%.2f", b-a}')

compose --profile worker logs worker >"$RESULTS/workers.log" 2>&1 || true

# Which worker ran what, so a duplicate is attributable rather than merely
# observed. Recorded even on a pass — it is also the evidence that the load
# genuinely spread across workers instead of one winning every race.
bench_sql "SELECT worker_id, COUNT(*) FROM bench_job_runs GROUP BY worker_id ORDER BY 2 DESC;" \
    >"$RESULTS/per-worker.txt" 2>&1 || true

FAILED_JOBS="$(bench_sql "SELECT COUNT(*) FROM failed_jobs;" | tr -d '[:space:]')"

{
    echo "experiment: 1.5-claim-exclusivity"
    echo "jobs: $COUNT"
    echo "workers: $WORKERS"
    echo "drained: $DRAINED"
    echo "drain_seconds: $ELAPSED"
    echo "failed_jobs: $FAILED_JOBS"
} | tee "$RESULTS/verdict.txt"

echo "==> per-worker distribution:"
cat "$RESULTS/per-worker.txt"

set +e
console bench:verify-records --expect "$COUNT" 2>&1 | tee "$RESULTS/verify.log"
VERIFY=${PIPESTATUS[0]}
set -e

stop_scaled worker

if [[ "$DRAINED" != "yes" ]]; then
    {
        echo "result: FAIL"
        echo "the queue did not drain inside ${DEADLINE}s; the verify result below"
        echo "describes an unfinished run and cannot be read as a claiming verdict."
    } | tee -a "$RESULTS/verdict.txt"
    exit 1
fi

if [[ "$VERIFY" -ne 0 ]]; then
    echo "result: FAIL — see verify.log" | tee -a "$RESULTS/verdict.txt"
    exit 1
fi

# A UNIQUE violation would have failed the job rather than the insert
# silently succeeding, so a non-zero failed_jobs count on an otherwise
# clean verify still means something went wrong.
if [[ "$FAILED_JOBS" != "0" ]]; then
    {
        echo "result: FAIL"
        echo "$FAILED_JOBS jobs dead-lettered. Every one of these jobs does a single"
        echo "insert, so a failure here is either a double claim caught by the UNIQUE"
        echo "index or the queue mis-settling work that succeeded."
    } | tee -a "$RESULTS/verdict.txt"
    exit 1
fi

echo "result: PASS — $COUNT jobs across $WORKERS workers, each executed exactly once" \
    | tee -a "$RESULTS/verdict.txt"
exit 0
