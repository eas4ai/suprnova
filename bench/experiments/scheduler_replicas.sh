#!/usr/bin/env bash
#
# Phase 1.2 — does a scheduled task fire once, or once per replica?
#
# Run N `schedule:work` processes against one database, the way any
# horizontally-scaled deployment does, and count the executions of a task
# registered to run every minute.
#
# Two arms in one window, because single-server execution is opt-in and
# measuring only one of them would mislead:
#
#   bench:tick-plain       no coordination requested — expected to record
#                          one row per replica per minute. Not a
#                          regression; it is the documented default, and
#                          it is the evidence the replicas were genuinely
#                          live and contending.
#   bench:tick-one-server  .on_one_server() — must record exactly one row
#                          per minute however many replicas are running.
#
#   PASS — the elected arm fired once per tick AND the control arm fired
#          on every replica. Both halves are required: a clean elected arm
#          proves nothing if nothing was contending it.
#   FAIL — the elected arm fired more than once on any tick, or the
#          control arm shows the replicas were never really racing.
#
# Boundary note: the first and last minutes of the window can legitimately
# hold fewer rows than the replica count, because replicas start and stop
# within them. For the elected arm that is harmless — a boundary can only
# hide an execution, never invent one, and the verdict is driven by
# minutes with *more* than one row. For the control arm it matters, so
# `bench:verify-ticks` looks only at interior minutes when deciding
# whether the replicas were genuinely contending, and marks boundary
# minutes in its output rather than averaging them away.
#
# Exit codes: 0 PASS, 1 FAIL, 2 setup failure.

set -euo pipefail
# shellcheck source=lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

: "${REPLICAS:=3}"     # concurrent schedule:work containers
: "${MINUTES:=4}"      # observation window

RESULTS="$(results_dir scheduler-replicas)"
echo "==> results: $RESULTS"

require_stack
stop_scaled scheduler

echo "==> clearing bench_scheduler_ticks"
bench_sql "TRUNCATE bench_scheduler_ticks;" >/dev/null

# Started in one command so the replicas race from the same instant. A
# staggered start would let each land in a different minute, and the
# collision this experiment looks for would never have the chance to occur.
echo "==> starting $REPLICAS schedule:work replicas"
compose --profile scheduler up -d --scale scheduler="$REPLICAS" scheduler \
    >"$RESULTS/up.log" 2>&1

RUNNING="$(compose --profile scheduler ps -q scheduler | wc -l | tr -d '[:space:]')"
if [[ "$RUNNING" != "$REPLICAS" ]]; then
    echo "SETUP FAILURE: asked for $REPLICAS replicas, $RUNNING are running" \
        | tee "$RESULTS/verdict.txt"
    cat "$RESULTS/up.log"
    stop_scaled scheduler
    exit 2
fi

echo "==> observing for ${MINUTES} minutes"
for m in $(seq 1 "$MINUTES"); do
    sleep 60
    TICKS="$(bench_sql "SELECT COUNT(*) FROM bench_scheduler_ticks;" | tr -d '[:space:]')"
    echo "    minute $m/$MINUTES — $TICKS ticks recorded"
done

compose --profile scheduler logs scheduler >"$RESULTS/schedulers.log" 2>&1 || true

bench_sql "SELECT task_name, tick_minute, COUNT(*) AS runs, \
                  COUNT(DISTINCT instance_id) AS instances \
           FROM bench_scheduler_ticks \
           GROUP BY task_name, tick_minute ORDER BY task_name, tick_minute;" \
    >"$RESULTS/per-minute.txt" 2>&1 || true

{
    echo "experiment: 1.2-scheduler-replicas"
    echo "replicas: $REPLICAS"
    echo "window_minutes: $MINUTES"
} | tee "$RESULTS/verdict.txt"

echo "==> per-minute (task | tick_minute | runs | distinct instances):"
cat "$RESULTS/per-minute.txt"

set +e
console bench:verify-ticks --replicas "$REPLICAS" 2>&1 | tee "$RESULTS/verify.log"
VERIFY=${PIPESTATUS[0]}
set -e

stop_scaled scheduler

if [[ "$VERIFY" -ne 0 ]]; then
    {
        echo "result: FAIL — see verify.log"
        echo "Either the elected arm fired more than once on some tick, or the control"
        echo "arm shows the replicas were never genuinely contending — in which case a"
        echo "single execution of the elected arm proves nothing."
    } | tee -a "$RESULTS/verdict.txt"
    exit 1
fi

echo "result: PASS — on_one_server() fired once per tick across $REPLICAS replicas, \
with the uncoordinated control arm firing on all of them" \
    | tee -a "$RESULTS/verdict.txt"
exit 0
