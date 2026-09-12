#!/usr/bin/env bash
# Run the framework's MySQL-only regressions against both standing engines:
# the machine's MariaDB service and the persistent gate-owned MySQL 8.4.

set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

: "${SUPRNOVA_GATE_RUN_ID:?SUPRNOVA_GATE_RUN_ID must be set by gate-runner.py}"

MARIADB_HOST=127.0.0.1
MARIADB_PORT=3306
MYSQL_PROPER_HOST=127.0.0.1
MYSQL_PROPER_PORT=3317

# Standing services (ruling, Shawn 2026-09-12): the gate provisions nothing.
# The mysql protocol is tested against BOTH engines it can point at - the
# host's standing MariaDB and the persistent gate-owned MySQL 8.4 container
# from scripts/setup-gate-databases.sh. Each engine pass works in its own
# pid-suffixed throwaway database and drops it on exit, so parallel gate
# runs cannot collide and nothing outside suprnova_test% is reachable with
# the gate user's grants.

mariadb_admin() {
    mariadb -h "$MARIADB_HOST" -P "$MARIADB_PORT" -u suprnova_gate -psuprnova-gate -e "$1" 2>/dev/null
}
mysql_proper_admin() {
    mysql -h "$MYSQL_PROPER_HOST" -P "$MYSQL_PROPER_PORT" -u root -psuprnova-gate -e "$1" 2>/dev/null
}

if ! mariadb_admin 'select 1' >/dev/null; then
    echo "check-mysql: cannot reach the standing MariaDB at ${MARIADB_HOST}:${MARIADB_PORT} as suprnova_gate." >&2
    echo "    Start the service and create the gate user:" >&2
    echo "    sudo mariadb < scripts/setup/gate-mariadb-user.sql" >&2
    exit 1
fi
if ! mysql_proper_admin 'select 1' >/dev/null; then
    echo "check-mysql: cannot reach the gate MySQL 8.4 at ${MYSQL_PROPER_HOST}:${MYSQL_PROPER_PORT}." >&2
    echo "    Run scripts/setup-gate-databases.sh to create and start it." >&2
    exit 1
fi

MARIADB_DB="suprnova_test_$$_md"
MYSQL_PROPER_DB="suprnova_test_$$_my"
cleanup() {
    mariadb_admin "DROP DATABASE IF EXISTS ${MARIADB_DB}" || true
    mysql_proper_admin "DROP DATABASE IF EXISTS ${MYSQL_PROPER_DB}" || true
}
trap cleanup EXIT INT TERM

run_mysql_suite() {
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
}

mariadb_admin "DROP DATABASE IF EXISTS ${MARIADB_DB}"
mariadb_admin "CREATE DATABASE ${MARIADB_DB}"
echo
echo "=== engine pass 1/2: MariaDB (standing service, ${MARIADB_HOST}:${MARIADB_PORT}, db ${MARIADB_DB}) ==="
export MYSQL_TEST_URL="mysql://suprnova_gate:suprnova-gate@${MARIADB_HOST}:${MARIADB_PORT}/${MARIADB_DB}"
run_mysql_suite

mysql_proper_admin "DROP DATABASE IF EXISTS ${MYSQL_PROPER_DB}"
mysql_proper_admin "CREATE DATABASE ${MYSQL_PROPER_DB}"
echo
echo "=== engine pass 2/2: MySQL 8.4 (gate container, ${MYSQL_PROPER_HOST}:${MYSQL_PROPER_PORT}, db ${MYSQL_PROPER_DB}) ==="
export MYSQL_TEST_URL="mysql://root:suprnova-gate@${MYSQL_PROPER_HOST}:${MYSQL_PROPER_PORT}/${MYSQL_PROPER_DB}"
run_mysql_suite
