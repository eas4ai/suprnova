#!/usr/bin/env bash
# CI-01 — run the framework's Postgres-only tests against a real Postgres.
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
# developer already runs, and — more importantly — a wrong guess would
# point these tests at somebody's real database. They are destructive:
# `DROP TABLE`, `CREATE TABLE`, bulk inserts. Letting Docker choose makes
# that impossible by construction. The bind is loopback-only, matching the
# scaffold's compose templates.

set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

if ! docker info >/dev/null 2>&1; then
    echo "check-postgres: the Docker daemon must be reachable." >&2
    echo "    These tests need a real Postgres; there is no SQLite fallback" >&2
    echo "    that would prove anything (that is the bug they guard)." >&2
    exit 1
fi

CONTAINER="suprnova-gate-pg-$$"
PG_PASSWORD="gate-$(head -c 12 /dev/urandom | od -An -tx1 | tr -d ' \n')"

cleanup() {
    docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
}
trap cleanup EXIT INT TERM

echo "starting disposable Postgres (${CONTAINER})..."
docker run -d --rm --name "$CONTAINER" \
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
# Postgres test file has to be added here consciously — a glob would let one
# land, never run, and look covered.
PG_TESTS=(
    rbac_postgres
    queue_database_postgres
    queue_worker_postgres
    notification_database_postgres
    eloquent_aggregate_postgres
)

for t in "${PG_TESTS[@]}"; do
    echo
    echo "==> cargo test -p suprnova --test ${t} -- --ignored"
    cargo test -p suprnova --test "$t" -- --ignored --test-threads=1
done

# `pagination` is a mixed file: its live tests cover Postgres AND MySQL, so
# it cannot be run with a bare `--ignored` here — the MySQL case would fail
# for want of a MySQL. Select the Postgres one by name.
echo
echo "==> cargo test -p suprnova --test pagination -- --ignored live_postgres"
cargo test -p suprnova --test pagination -- --ignored --test-threads=1 live_postgres

# The workflow lease-reclaim test is an in-source unit test, and it is
# gated TWICE: `#[ignore]` keeps it out of the normal run, and even when
# un-ignored it returns early unless `DATABASE_URL` names a Postgres. Both
# gates were always closed, so it reported green without executing a line.
# It covers "a worker died holding the lock, another must reclaim it",
# which is not a guarantee worth leaving unproven.
#
# Run it with the disposable database and both gates opened, then assert it
# actually ran: a silent skip here would restore exactly the hole this step
# exists to close.
echo
echo "==> cargo test -p suprnova --lib workflow::tests::test_claim_reclaims_expired_running_row"
workflow_out="$(DATABASE_URL="$PG_TEST_URL" cargo test -p suprnova --lib \
    workflow::tests::test_claim_reclaims_expired_running_row \
    -- --ignored --test-threads=1 --nocapture 2>&1)"
echo "$workflow_out"

if grep -q "skipping:" <<<"$workflow_out"; then
    echo >&2
    echo "check-postgres: the workflow reclaim test SKIPPED itself despite a" >&2
    echo "    Postgres DATABASE_URL being set. That is the silent-pass bug" >&2
    echo "    this step exists to prevent — fix the gate, not this check." >&2
    exit 1
fi
if ! grep -qE "^test .*test_claim_reclaims_expired_running_row \.\.\. ok" <<<"$workflow_out"; then
    echo >&2
    echo "check-postgres: the workflow reclaim test did not report ok." >&2
    exit 1
fi

echo
echo "check-postgres: OK"
