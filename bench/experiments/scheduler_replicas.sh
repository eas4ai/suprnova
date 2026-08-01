#!/usr/bin/env bash
#
# Phase 1.2 — does a scheduled task fire once, or once per replica?
#
# Laravel's answer is `onOneServer()`, backed by a cache lock. The question
# here is what happens by default: run N `schedule:work` processes against
# one database, the way any horizontally-scaled deployment does, and count
# the executions of a task registered to run every minute.
#
#   PASS — one row per tick_minute. The replicas coordinate.
#   FAIL — N rows per tick_minute. Every scheduled task runs once per
#          replica, so a nightly billing job bills every customer N times.
#
# Boundary note: the first and last minutes of the window can legitimately
# hold fewer rows than the replica count, because replicas start and stop
# within them. That can only manufacture a false PASS on those minutes,
# never a false FAIL — the verdict below is driven by minutes with *more*
# than one row, and the per-minute breakdown is printed so a boundary
# minute is visible rather than merely averaged away.
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

bench_sql "SELECT tick_minute, COUNT(*) AS runs, COUNT(DISTINCT instance_id) AS instances \
           FROM bench_scheduler_ticks GROUP BY tick_minute ORDER BY tick_minute;" \
    >"$RESULTS/per-minute.txt" 2>&1 || true

{
    echo "experiment: 1.2-scheduler-replicas"
    echo "replicas: $REPLICAS"
    echo "window_minutes: $MINUTES"
} | tee "$RESULTS/verdict.txt"

echo "==> per-minute (tick_minute | runs | distinct instances):"
cat "$RESULTS/per-minute.txt"

set +e
console bench:verify-ticks 2>&1 | tee "$RESULTS/verify.log"
VERIFY=${PIPESTATUS[0]}
set -e

stop_scaled scheduler

if [[ "$VERIFY" -ne 0 ]]; then
    {
        echo "result: FAIL — see verify.log"
        echo "A task registered once fired more than once per minute. Nothing in the"
        echo "default configuration stops a second replica from running the same due"
        echo "task, so every scheduled side effect is multiplied by the replica count."
    } | tee -a "$RESULTS/verdict.txt"
    exit 1
fi

echo "result: PASS — each tick fired exactly once across $REPLICAS replicas" \
    | tee -a "$RESULTS/verdict.txt"
exit 0
