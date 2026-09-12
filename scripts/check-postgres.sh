#!/usr/bin/env bash
# CI-01 - run the framework's Postgres-only tests against a real Postgres.
#
# The audit's DATA-01 (raw SQL built with `?` placeholders, which Postgres
# rejects outright) shipped because the whole suite runs on SQLite. The
# fix landed with tests, but those tests are `#[ignore]`d and nothing ran
# them, so the bug class could regress silently the same way it arrived.
# This script is what runs them.
#
# Usage:
#   scripts/check-postgres.sh
#
# The container is disposable and removed on exit, success or failure.
#
# ## Port safety
#
# The host port is assigned by Docker (`-p 127.0.0.1::5432`) and read back,
# rather than pinned. Two reasons: a pinned port collides with whatever the
# developer already runs, and - more importantly - a wrong guess would
# point these tests at somebody's real database. They are destructive:
# `DROP TABLE`, `CREATE TABLE`, bulk inserts. Letting Docker choose makes
# that impossible by construction. The bind is loopback-only, matching the
# scaffold's compose templates.

set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

: "${SUPRNOVA_GATE_RUN_ID:?SUPRNOVA_GATE_RUN_ID must be set by gate-runner.py}"

# Standing service (ruling, Shawn 2026-09-12): the gate provisions nothing.
# The host's Postgres at 127.0.0.1:5432 answers, with the scoped gate role
# from scripts/setup-gate-databases.sh; each run works in its own throwaway
# database (pid-suffixed, so parallel gate runs cannot collide) and drops it
# on exit. The role owns only what it creates.
PG_HOST=127.0.0.1
PG_PORT=5432
PG_USER=suprnova_gate
PG_PASSWORD=suprnova-gate
GATE_DB="suprnova_test_$$"

psql_gate() {
    PGPASSWORD="$PG_PASSWORD" psql -h "$PG_HOST" -p "$PG_PORT" -U "$PG_USER" -d postgres -v ON_ERROR_STOP=1 -qAtc "$1"
}

if ! psql_gate 'select 1' >/dev/null 2>&1; then
    echo "check-postgres: cannot reach Postgres at ${PG_HOST}:${PG_PORT} as ${PG_USER}." >&2
    echo "    The gate uses the machine's standing Postgres and never provisions" >&2
    echo "    one mid-run. Start the service and create the gate role:" >&2
    echo "    sudo -u postgres psql -f scripts/setup/gate-postgres-role.sql" >&2
    exit 1
fi

cleanup() {
    psql_gate "DROP DATABASE IF EXISTS ${GATE_DB}" >/dev/null 2>&1 || true
}
trap cleanup EXIT INT TERM

psql_gate "DROP DATABASE IF EXISTS ${GATE_DB}" >/dev/null
psql_gate "CREATE DATABASE ${GATE_DB}" >/dev/null
echo "using standing Postgres at ${PG_HOST}:${PG_PORT}, database ${GATE_DB}"

export PG_TEST_URL="postgres://${PG_USER}:${PG_PASSWORD}@${PG_HOST}:${PG_PORT}/${GATE_DB}"

# Serial, always. These tests share a database and several of them DROP and
# recreate the same table names; in parallel they clobber each other and
# fail in ways that look like product bugs.
#
# Each submodule is named explicitly rather than swept with a glob so that a new
# Postgres submodule has to be added here consciously - a glob would let one
# land, never run, and look covered.
PG_TESTS=(
    rbac/postgres
    queue/database_postgres
    queue/worker_postgres
    notifications/database_postgres
    eloquent/aggregate_postgres
    eloquent/mass_write_postgres
)

# Wave 6 owns this target and releases in 1.3.4. The 1.3.3 gate knows the
# future target name but must not require code that does not ship in this
# release. Once the submodule lands, it is appended automatically; after the
# workspace advances past 1.3.3, its absence is a gate error rather than a
# silent skip.
PIVOT_FILTER_TEST="framework/tests/eloquent/relations_pivot_filters_postgres.rs"
if [[ -f "$PIVOT_FILTER_TEST" ]]; then
    PG_TESTS+=(eloquent/relations_pivot_filters_postgres)
elif ! grep -q '^version = "1\.3\.3"$' Cargo.toml; then
    echo "check-postgres: missing required target $PIVOT_FILTER_TEST" >&2
    exit 1
fi

for t in "${PG_TESTS[@]}"; do
    module="${t%%/*}"
    stem="${t#*/}"
    echo
    # Each entry is module/stem: the module binary under framework/tests/<module>/
    # and the former file, now a submodule, selected by its path prefix. The
    # existence check keeps the "add it consciously" rule from before the fold.
    if [ ! -f "framework/tests/${module}/${stem}.rs" ]; then
        echo "==> skip ${t}: framework/tests/${module}/${stem}.rs does not exist on this branch"
        continue
    fi
    echo "==> cargo test -p suprnova --test ${module} -- --ignored ${stem}::"
    cargo test -p suprnova --test "$module" -- --ignored --test-threads=1 "${stem}::"
done

# `pagination` is a mixed submodule: its live tests cover Postgres AND MySQL, so
# it cannot be run with a bare `--ignored` here - the MySQL case would fail
# for want of a MySQL. Select the Postgres one by name.
echo
echo "==> cargo test -p suprnova --test pagination -- --ignored pagination::live_postgres"
cargo test -p suprnova --test pagination -- --ignored --test-threads=1 pagination::live_postgres

# `render_cache_ledger` is the same shape: SQLite tests run unconditionally,
# and Postgres-tagged and MySQL-tagged `#[ignore]`d tests share the submodule.
# Select the Postgres-tagged ones by name for the same reason as above.
#
# `--ignored live_postgres` exits 0 when the filter matches nothing, so a
# renamed test would silently stop testing Postgres here while the gate
# stayed green. Assert on the output, not just the exit code, the way the
# workflow lease-reclaim step below already does.
echo
echo "==> cargo test -p suprnova --test render_cache -- --ignored ledger::live_postgres"
render_cache_pg_out="$(cargo test -p suprnova --test render_cache -- --ignored --test-threads=1 ledger::live_postgres 2>&1)"
echo "$render_cache_pg_out"
for render_cache_pg_test in \
    live_postgres_generation_ledger_advances_and_reads \
    live_postgres_concurrent_advances_in_opposite_order_do_not_deadlock \
    live_postgres_a_write_committed_during_a_cached_render_is_never_published_as_current; do
    if ! grep -qE "^test ledger::${render_cache_pg_test} \.\.\. ok" <<<"$render_cache_pg_out"; then
        echo "check-postgres: ${render_cache_pg_test} did not report ok (filter may have matched nothing)" >&2
        exit 1
    fi
done

# `render_cache/tiers/sql` is mixed the same way, with one more dialect in
# it: SQLite tier tests run unconditionally beside Postgres- and MySQL-tagged
# `#[ignore]`d ones. Select the Postgres-tagged ones by name, and assert on
# the output for the same reason as above. The submodule is part of the
# filter (the tier proofs are split by backend), so a renamed or moved
# submodule stops this step rather than quietly selecting nothing.
echo
echo "==> cargo test -p suprnova --test render_cache -- --ignored tiers::sql::live_postgres"
if ! tiers_pg_out="$(cargo test -p suprnova --test render_cache -- --ignored --test-threads=1 tiers::sql::live_postgres 2>&1)"; then
    echo "$tiers_pg_out"
    exit 1
fi
echo "$tiers_pg_out"
for tiers_pg_test in \
    live_postgres_record_creation_and_cas_conflict \
    live_postgres_publish_fencing_and_sweep \
    live_postgres_lease_takeover_and_fencing; do
    if ! grep -qE "^test tiers::sql::${tiers_pg_test} \.\.\. ok" <<<"$tiers_pg_out"; then
        echo "check-postgres: ${tiers_pg_test} did not report ok (filter may have matched nothing)" >&2
        exit 1
    fi
done
# The exact number of `tiers::sql::live_postgres` tests. Update it when one is
# added or removed.
if ! grep -qE "^test result: ok\. 3 passed" <<<"$tiers_pg_out"; then
    echo "check-postgres: the tiers summary line does not report exactly 3 passed" >&2
    exit 1
fi

# `store_conformance` is mixed the same way once more, and for a different
# reason: one `RenderStore` conformance suite runs over every provider, so
# the file and SQLite variants run unconditionally beside Postgres-, MySQL-,
# and Redis-tagged `#[ignore]`d ones in the same submodule. Select the
# Postgres-tagged one by name, and assert on the output for the same reason
# as above.
echo
echo "==> cargo test -p suprnova --test render_cache -- --ignored store_conformance::live_postgres_"
if ! store_pg_out="$(cargo test -p suprnova --test render_cache -- --ignored --test-threads=1 store_conformance::live_postgres_ 2>&1)"; then
    echo "$store_pg_out"
    exit 1
fi
echo "$store_pg_out"
if ! grep -qE "^test store_conformance::live_postgres_render_store_conforms \.\.\. ok" <<<"$store_pg_out"; then
    echo "check-postgres: the RenderStore conformance suite did not report ok (filter may have matched nothing)" >&2
    exit 1
fi
# The exact number of `store_conformance::live_postgres_` tests. Update it
# when one is added or removed.
if ! grep -qE "^test result: ok\. 1 passed" <<<"$store_pg_out"; then
    echo "check-postgres: the store conformance summary line does not report exactly 1 passed" >&2
    exit 1
fi

# The SQL record store's guarded upsert - the statement that must refuse a
# lower fence and accept a higher one - is proved by an in-source unit test,
# so it needs `--lib` rather than the integration binary. Same silent-pass
# hazard, same output assertion.
echo
echo "==> cargo test -p suprnova --lib -- --ignored live_postgres"
if ! sql_store_pg_out="$(cargo test -p suprnova --lib -- --ignored --test-threads=1 live_postgres 2>&1)"; then
    echo "$sql_store_pg_out"
    exit 1
fi
echo "$sql_store_pg_out"
if ! grep -qE "^test render_cache::providers::sql_store::tests::live_postgres_the_guarded_upsert_refuses_a_lower_fence_and_takes_a_higher_one \.\.\. ok" <<<"$sql_store_pg_out"; then
    echo "check-postgres: the SQL store guarded-upsert test did not report ok (filter may have matched nothing)" >&2
    exit 1
fi
# The bare `live_postgres` filter selects exactly one in-source test. Update
# it when another is added.
if ! grep -qE "^test result: ok\. 1 passed" <<<"$sql_store_pg_out"; then
    echo "check-postgres: the --lib summary line does not report exactly 1 passed" >&2
    exit 1
fi

# Savepoint aliases must select the same row and deferred-effect boundary.
cargo test -p suprnova --test queue after_commit::savepoint_aliases_postgres_rows_and_jobs_agree -- --ignored --exact

# The workflow lease-reclaim tests are in-source unit tests, and they are
# gated TWICE: `#[ignore]` keeps them out of the normal run, and even when
# un-ignored they return early unless `DATABASE_URL` names a Postgres. Both
# gates were always closed, so they reported green without executing a line.
# They cover both sides of crash recovery: reclaim work that remains below
# its attempt budget, and terminalize exhausted work without running it.
#
# Run each one with the disposable database and both gates opened, then
# assert it actually ran: a silent skip here would restore exactly the hole
# this step exists to close.
WORKFLOW_TESTS=(
    test_claim_reclaims_expired_running_row
    test_expired_running_workflow_at_attempt_budget_is_failed_not_reclaimed
    test_postgres_reclaim_rejects_stale_step_completion
)

for workflow_test in "${WORKFLOW_TESTS[@]}"; do
    echo
    echo "==> cargo test -p suprnova --lib workflow::tests::${workflow_test}"
    workflow_out="$(DATABASE_URL="$PG_TEST_URL" cargo test -p suprnova --lib \
        "workflow::tests::${workflow_test}" \
        -- --ignored --test-threads=1 --nocapture 2>&1)"
    echo "$workflow_out"

    if grep -q "skipping:" <<<"$workflow_out"; then
        echo >&2
        echo "check-postgres: ${workflow_test} SKIPPED itself despite a" >&2
        echo "    Postgres DATABASE_URL being set. That is the silent-pass bug" >&2
        echo "    this step exists to prevent - fix the gate, not this check." >&2
        exit 1
    fi
    if ! grep -qE "^test .*${workflow_test} \.\.\. ok" <<<"$workflow_out"; then
        echo >&2
        echo "check-postgres: ${workflow_test} did not report ok." >&2
        exit 1
    fi
done

echo
echo "check-postgres: OK"
