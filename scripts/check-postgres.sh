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

if ! docker info >/dev/null 2>&1; then
    echo "check-postgres: the Docker daemon must be reachable." >&2
    echo "    These tests need a real Postgres; there is no SQLite fallback" >&2
    echo "    that would prove anything (that is the bug they guard)." >&2
    exit 1
fi

CONTAINER="suprnova-gate-pg-${SUPRNOVA_GATE_RUN_ID}-$$"
PG_PASSWORD="gate-$(head -c 12 /dev/urandom | od -An -tx1 | tr -d ' \n')"

cleanup() {
    docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
}
trap cleanup EXIT INT TERM

echo "starting disposable Postgres (${CONTAINER})..."
docker run -d --rm --name "$CONTAINER" \
    --label "suprnova-gate-run=${SUPRNOVA_GATE_RUN_ID}" \
    -e POSTGRES_PASSWORD="$PG_PASSWORD" \
    -e POSTGRES_DB=suprnova_test \
    -p 127.0.0.1::5432 \
    postgres:17-alpine >/dev/null

# `docker port` reports the host side Docker picked, e.g. "127.0.0.1:49154".
HOST_PORT="$(docker port "$CONTAINER" 5432/tcp | head -1 | sed 's/.*://')"
if [[ -z "$HOST_PORT" ]]; then
    echo "check-postgres: could not determine the mapped host port." >&2
    exit 1
fi
echo "    mapped to 127.0.0.1:${HOST_PORT}"

# Wait for readiness. `pg_isready` runs inside the container, so this is
# the server's own view rather than a TCP connect that succeeds before
# Postgres finishes its first-boot initdb.
for _ in $(seq 1 60); do
    if docker exec "$CONTAINER" pg_isready -U postgres -d suprnova_test >/dev/null 2>&1; then
        ready=1
        break
    fi
    sleep 1
done
if [[ "${ready:-0}" -ne 1 ]]; then
    echo "check-postgres: Postgres never became ready. Container log:" >&2
    docker logs "$CONTAINER" >&2 || true
    exit 1
fi

export PG_TEST_URL="postgres://postgres:${PG_PASSWORD}@127.0.0.1:${HOST_PORT}/suprnova_test"

# Serial, always. These tests share a database and several of them DROP and
# recreate the same table names; in parallel they clobber each other and
# fail in ways that look like product bugs.
#
# Each file is named explicitly rather than swept with a glob so that a new
# Postgres test file has to be added here consciously - a glob would let one
# land, never run, and look covered.
PG_TESTS=(
    rbac_postgres
    queue_database_postgres
    queue_worker_postgres
    notification_database_postgres
    eloquent_aggregate_postgres
    eloquent_mass_write_postgres
)

# Wave 6 owns this target and releases in 1.3.4. The 1.3.3 gate knows the
# future target name but must not require code that does not ship in this
# release. Once the file lands, it is appended automatically; after the
# workspace advances past 1.3.3, its absence is a gate error rather than a
# silent skip.
PIVOT_FILTER_TEST="framework/tests/eloquent_relations_pivot_filters_postgres.rs"
if [[ -f "$PIVOT_FILTER_TEST" ]]; then
    PG_TESTS+=(eloquent_relations_pivot_filters_postgres)
elif ! grep -q '^version = "1\.3\.3"$' Cargo.toml; then
    echo "check-postgres: missing required target $PIVOT_FILTER_TEST" >&2
    exit 1
fi

for t in "${PG_TESTS[@]}"; do
    echo
    # The list is shared by every branch this checkout's scripts/ is copied
    # into, so a file that only exists on a newer branch is skipped loudly
    # rather than failing a release gate for a test it cannot contain. An
    # existing file is never skipped, so the "add it consciously" rule holds.
    if [ ! -f "framework/tests/${t}.rs" ]; then
        echo "==> skip ${t}: framework/tests/${t}.rs does not exist on this branch"
        continue
    fi
    echo "==> cargo test -p suprnova --test ${t} -- --ignored"
    cargo test -p suprnova --test "$t" -- --ignored --test-threads=1
done

# `pagination` is a mixed file: its live tests cover Postgres AND MySQL, so
# it cannot be run with a bare `--ignored` here - the MySQL case would fail
# for want of a MySQL. Select the Postgres one by name.
echo
echo "==> cargo test -p suprnova --test pagination -- --ignored live_postgres"
cargo test -p suprnova --test pagination -- --ignored --test-threads=1 live_postgres

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
