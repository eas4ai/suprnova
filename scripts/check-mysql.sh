#!/usr/bin/env bash
# Run the framework's MySQL-only regressions against a disposable,
# loopback-only MariaDB instance.

set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

: "${SUPRNOVA_GATE_RUN_ID:?SUPRNOVA_GATE_RUN_ID must be set by gate-runner.py}"

if ! docker info >/dev/null 2>&1; then
    echo "check-mysql: the Docker daemon must be reachable." >&2
    exit 1
fi

CONTAINER="suprnova-gate-mysql-${SUPRNOVA_GATE_RUN_ID}-$$"
MYSQL_PASSWORD="gate-$(head -c 12 /dev/urandom | od -An -tx1 | tr -d ' \n')"

cleanup() {
    docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
}
trap cleanup EXIT INT TERM

echo "starting disposable MariaDB (${CONTAINER})..."
docker run -d --rm --name "$CONTAINER" \
    --label "suprnova-gate-run=${SUPRNOVA_GATE_RUN_ID}" \
    -e MARIADB_ROOT_PASSWORD="$MYSQL_PASSWORD" \
    -e MARIADB_DATABASE=suprnova_test \
    -p 127.0.0.1::3306 \
    mariadb:11-jammy >/dev/null

HOST_PORT="$(docker port "$CONTAINER" 3306/tcp | head -1 | sed 's/.*://')"
if [[ -z "$HOST_PORT" ]]; then
    echo "check-mysql: could not determine the mapped host port." >&2
    exit 1
fi
echo "    mapped to 127.0.0.1:${HOST_PORT}"

for _ in $(seq 1 90); do
    if docker exec "$CONTAINER" mariadb-admin ping \
        --host=127.0.0.1 \
        --user=root \
        --password="$MYSQL_PASSWORD" \
        --silent >/dev/null 2>&1; then
        ready=1
        break
    fi
    sleep 1
done
if [[ "${ready:-0}" -ne 1 ]]; then
    echo "check-mysql: MariaDB never became ready. Container log:" >&2
    docker logs "$CONTAINER" >&2 || true
    exit 1
fi

export MYSQL_TEST_URL="mysql://root:${MYSQL_PASSWORD}@127.0.0.1:${HOST_PORT}/suprnova_test"

echo
echo "==> cargo test -p suprnova --test eloquent_mass_write_mysql -- --ignored"
cargo test -p suprnova --test eloquent_mass_write_mysql -- --ignored --test-threads=1

# `render_cache_ledger` is a mixed file: SQLite tests run unconditionally,
# and Postgres-tagged and MySQL-tagged `#[ignore]`d tests share it. Select
# the MySQL-tagged ones by name so this run never trips a Postgres-only
# test for want of a Postgres.
#
# `--ignored live_mysql` exits 0 when the filter matches nothing, so a
# renamed test would silently stop testing MySQL here while the gate stayed
# green. Assert on the output, not just the exit code, the way the workflow
# regression step below already does.
echo
echo "==> cargo test -p suprnova --test render_cache_ledger -- --ignored live_mysql"
render_cache_mysql_out="$(cargo test -p suprnova --test render_cache_ledger -- --ignored --test-threads=1 live_mysql 2>&1)"
echo "$render_cache_mysql_out"
for render_cache_mysql_test in \
    live_mysql_generation_ledger_advances_and_reads \
    live_mysql_concurrent_advances_in_opposite_order_do_not_deadlock \
    live_mysql_a_write_committed_during_a_cached_render_is_never_published_as_current; do
    if ! grep -qE "^test ${render_cache_mysql_test} \.\.\. ok" <<<"$render_cache_mysql_out"; then
        echo "check-mysql: ${render_cache_mysql_test} did not report ok (filter may have matched nothing)" >&2
        exit 1
    fi
done

cargo test -p suprnova --test queue_after_commit savepoint_aliases_mysql_rows_and_jobs_agree -- --ignored --exact

echo
echo "==> cargo test -p suprnova --lib workflow::tests::test_mysql_"
workflow_out="$(cargo test -p suprnova --lib \
    workflow::tests::test_mysql_ \
    -- --ignored --test-threads=1 --nocapture 2>&1)"
echo "$workflow_out"

if [[ "$workflow_out" == *"skipping:"* ]]; then
    echo "check-mysql: workflow regression test skipped despite MYSQL_TEST_URL" >&2
    exit 1
fi
if [[ "$workflow_out" != *"2 passed; 0 failed"* ]]; then
    echo "check-mysql: workflow regression tests did not execute exactly twice" >&2
    exit 1
fi

echo
echo "check-mysql: OK"
