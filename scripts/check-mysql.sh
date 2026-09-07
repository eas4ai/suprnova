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
echo "==> cargo test -p suprnova --test eloquent -- --ignored mass_write_mysql::"
cargo test -p suprnova --test eloquent -- --ignored --test-threads=1 mass_write_mysql::

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
echo "==> cargo test -p suprnova --test render_cache -- --ignored ledger::live_mysql"
render_cache_mysql_out="$(cargo test -p suprnova --test render_cache -- --ignored --test-threads=1 ledger::live_mysql 2>&1)"
echo "$render_cache_mysql_out"
for render_cache_mysql_test in \
    live_mysql_generation_ledger_advances_and_reads \
    live_mysql_concurrent_advances_in_opposite_order_do_not_deadlock \
    live_mysql_a_write_committed_during_a_cached_render_is_never_published_as_current; do
    if ! grep -qE "^test ledger::${render_cache_mysql_test} \.\.\. ok" <<<"$render_cache_mysql_out"; then
        echo "check-mysql: ${render_cache_mysql_test} did not report ok (filter may have matched nothing)" >&2
        exit 1
    fi
done

# `tiers/sql` is mixed the same way, with one more dialect in it: SQLite tier
# tests run unconditionally beside Postgres- and MySQL-tagged `#[ignore]`d
# ones. Select the MySQL-tagged ones by name, and assert on the output for the
# same reason as above. The submodule is part of the filter (the tier proofs
# are split by backend), so a renamed or moved submodule stops this step
# rather than quietly selecting nothing.
echo
echo "==> cargo test -p suprnova --test render_cache -- --ignored tiers::sql::live_mysql"
if ! tiers_mysql_out="$(cargo test -p suprnova --test render_cache -- --ignored --test-threads=1 tiers::sql::live_mysql 2>&1)"; then
    echo "$tiers_mysql_out"
    exit 1
fi
echo "$tiers_mysql_out"
for tiers_mysql_test in \
    live_mysql_record_creation_and_cas_conflict \
    live_mysql_publish_fencing_and_sweep \
    live_mysql_lease_takeover_and_fencing; do
    if ! grep -qE "^test tiers::sql::${tiers_mysql_test} \.\.\. ok" <<<"$tiers_mysql_out"; then
        echo "check-mysql: ${tiers_mysql_test} did not report ok (filter may have matched nothing)" >&2
        exit 1
    fi
done
# The exact number of `tiers::sql::live_mysql` tests. Update it when one is added
# or removed.
if ! grep -qE "^test result: ok\. 3 passed" <<<"$tiers_mysql_out"; then
    echo "check-mysql: the tiers summary line does not report exactly 3 passed" >&2
    exit 1
fi

# `store_conformance` is mixed the same way once more, and for a different
# reason: one `RenderStore` conformance suite runs over every provider, so
# the file and SQLite variants run unconditionally beside Postgres-, MySQL-,
# and Redis-tagged `#[ignore]`d ones in the same submodule. Select the
# MySQL-tagged one by name, and assert on the output for the same reason as
# above.
echo
echo "==> cargo test -p suprnova --test render_cache -- --ignored store_conformance::live_mysql_"
if ! store_mysql_out="$(cargo test -p suprnova --test render_cache -- --ignored --test-threads=1 store_conformance::live_mysql_ 2>&1)"; then
    echo "$store_mysql_out"
    exit 1
fi
echo "$store_mysql_out"
if ! grep -qE "^test store_conformance::live_mysql_render_store_conforms \.\.\. ok" <<<"$store_mysql_out"; then
    echo "check-mysql: the RenderStore conformance suite did not report ok (filter may have matched nothing)" >&2
    exit 1
fi
# The exact number of `store_conformance::live_mysql_` tests. Update it when
# one is added or removed.
if ! grep -qE "^test result: ok\. 1 passed" <<<"$store_mysql_out"; then
    echo "check-mysql: the store conformance summary line does not report exactly 1 passed" >&2
    exit 1
fi

# The SQL record store's guarded upsert - the statement that must refuse a
# lower fence and accept a higher one - is proved by an in-source unit test,
# so it needs `--lib` rather than the integration binary. Same silent-pass
# hazard, same output assertion.
echo
echo "==> cargo test -p suprnova --lib -- --ignored live_mysql"
if ! sql_store_mysql_out="$(cargo test -p suprnova --lib -- --ignored --test-threads=1 live_mysql 2>&1)"; then
    echo "$sql_store_mysql_out"
    exit 1
fi
echo "$sql_store_mysql_out"
if ! grep -qE "^test render_cache::providers::sql_store::tests::live_mysql_the_guarded_upsert_refuses_a_lower_fence_and_takes_a_higher_one \.\.\. ok" <<<"$sql_store_mysql_out"; then
    echo "check-mysql: the SQL store guarded-upsert test did not report ok (filter may have matched nothing)" >&2
    exit 1
fi
# The bare `live_mysql` filter selects exactly one in-source test. Update it
# when another is added.
if ! grep -qE "^test result: ok\. 1 passed" <<<"$sql_store_mysql_out"; then
    echo "check-mysql: the --lib summary line does not report exactly 1 passed" >&2
    exit 1
fi

cargo test -p suprnova --test queue after_commit::savepoint_aliases_mysql_rows_and_jobs_agree -- --ignored --exact

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
