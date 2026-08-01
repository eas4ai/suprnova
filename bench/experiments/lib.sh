# Shared plumbing for the Phase 1 experiment drivers.
#
# Sourced, never executed. Everything here is about talking to the one
# compose project the harness owns — the project name is repeated in every
# invocation on purpose, because a compose command that picks up the
# ambient project would be operating on the host's real deployments.

# shellcheck shell=bash

BENCH_PROJECT="suprnova-bench"
BENCH_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BENCH_COMPOSE_FILE="$BENCH_ROOT/compose/suprnova.yml"

compose() {
    docker compose -p "$BENCH_PROJECT" -f "$BENCH_COMPOSE_FILE" "$@"
}

# One-shot console command against the same image, database and network as
# the system under test. `-T` because these run unattended.
console() {
    compose --profile tools run --rm -T console "$@"
}

# Raw SQL against the benchmark database. Used where the question is about
# the queue's own bookkeeping — `attempts`, `reserved_until` — which is
# state the framework deliberately does not expose through a command.
bench_sql() {
    compose exec -T db psql -U bench -d bench -tAc "$1"
}

# Every experiment writes into its own timestamped directory so a re-run
# never overwrites the evidence for the previous one.
results_dir() {
    local name="$1"
    local dir="${RESULTS_DIR:-$BENCH_ROOT/results/$(date -u +%Y%m%dT%H%M%SZ)-$name}"
    mkdir -p "$dir"
    printf '%s' "$dir"
}

# Fail loudly and early rather than producing a verdict from a stack that
# was never up. A benchmark that reports FAIL because nothing was listening
# is worse than one that refuses to run.
#
# The schema check is not belt-and-braces: only `app serve` runs
# migrations, and `console` does not. Without `sut` having booted at least
# once, every queue experiment would fail on a missing table and read as a
# damning result rather than an un-run one.
require_stack() {
    local up_hint="  docker compose -p $BENCH_PROJECT -f $BENCH_COMPOSE_FILE up -d"

    if ! compose ps --status running --services 2>/dev/null | grep -qx "db"; then
        echo "SETUP FAILURE: the bench database is not running." >&2
        echo "  bring the stack up first:" >&2
        echo "$up_hint" >&2
        exit 2
    fi

    local missing
    missing="$(bench_sql "SELECT string_agg(t, ', ') FROM unnest(
        ARRAY['jobs','failed_jobs','bench_job_runs','bench_scheduler_ticks']
    ) AS t WHERE to_regclass('public.' || t) IS NULL;" | tr -d '[:space:]')"
    if [[ -n "$missing" ]]; then
        echo "SETUP FAILURE: migrations have not run — missing tables: $missing" >&2
        echo "  only \`app serve\` migrates, so the sut has to boot at least once:" >&2
        echo "$up_hint" >&2
        exit 2
    fi
}

# Tear down only what this experiment scaled up, leaving db and sut alone.
stop_scaled() {
    local service="$1"
    compose --profile worker --profile scheduler rm -sf "$service" >/dev/null 2>&1 || true
}
