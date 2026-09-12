#!/usr/bin/env bash
# Run the tracked Magnetar T2 live-database profile against standing services.
#
# Ruling (Shawn, 2026-09-12): the gate provisions nothing. Postgres and
# MariaDB are the machine's standing services; MySQL 8.4 is the persistent
# gate-owned container from scripts/setup-gate-databases.sh. The mysql
# protocol profile runs twice - once per engine - because the framework
# claims both and the engines genuinely diverge (collations, RETURNING,
# JSON, auth). Each pass works in pid-suffixed throwaway databases dropped
# on exit, so parallel gate runs cannot collide.

set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

: "${SUPRNOVA_GATE_RUN_ID:?SUPRNOVA_GATE_RUN_ID must be set by gate-runner.py}"

PG_HOST=127.0.0.1
PG_PORT=5432
MARIADB_HOST=127.0.0.1
MARIADB_PORT=3306
MYSQL_PROPER_HOST=127.0.0.1
MYSQL_PROPER_PORT=3317

psql_gate() {
    PGPASSWORD=suprnova-gate psql -h "$PG_HOST" -p "$PG_PORT" -U suprnova_gate -d postgres -v ON_ERROR_STOP=1 -qAtc "$1"
}
mariadb_admin() {
    mariadb -h "$MARIADB_HOST" -P "$MARIADB_PORT" -u suprnova_gate -psuprnova-gate -e "$1" 2>/dev/null
}
mysql_proper_admin() {
    mysql -h "$MYSQL_PROPER_HOST" -P "$MYSQL_PROPER_PORT" -u root -psuprnova-gate -e "$1" 2>/dev/null
}

if ! psql_gate 'select 1' >/dev/null 2>&1; then
    echo "check-magnetar-live: cannot reach standing Postgres as suprnova_gate." >&2
    echo "    sudo -u postgres psql -f scripts/setup/gate-postgres-role.sql" >&2
    exit 1
fi
if ! mariadb_admin 'select 1' >/dev/null; then
    echo "check-magnetar-live: cannot reach standing MariaDB as suprnova_gate." >&2
    echo "    sudo mariadb < scripts/setup/gate-mariadb-user.sql" >&2
    exit 1
fi
if ! mysql_proper_admin 'select 1' >/dev/null; then
    echo "check-magnetar-live: gate MySQL 8.4 is not answering." >&2
    echo "    Run scripts/setup-gate-databases.sh to create and start it." >&2
    exit 1
fi

PG_DB="magnetar_test_$$"
MARIADB_DB="magnetar_test_$$_md"
MYSQL_PROPER_DB="magnetar_test_$$_my"

cleanup() {
    psql_gate "DROP DATABASE IF EXISTS ${PG_DB}" >/dev/null 2>&1 || true
    mariadb_admin "DROP DATABASE IF EXISTS ${MARIADB_DB}" || true
    mysql_proper_admin "DROP DATABASE IF EXISTS ${MYSQL_PROPER_DB}" || true
}
trap cleanup EXIT INT TERM

psql_gate "DROP DATABASE IF EXISTS ${PG_DB}" >/dev/null
psql_gate "CREATE DATABASE ${PG_DB}" >/dev/null
export MAGNETAR_POSTGRES_TEST_URL="postgres://suprnova_gate:suprnova-gate@${PG_HOST}:${PG_PORT}/${PG_DB}"

mariadb_admin "DROP DATABASE IF EXISTS ${MARIADB_DB}"
mariadb_admin "CREATE DATABASE ${MARIADB_DB}"
echo "=== engine pass 1/2: MariaDB (standing service, db ${MARIADB_DB}) ==="
export MAGNETAR_MYSQL_TEST_URL="mysql://suprnova_gate:suprnova-gate@${MARIADB_HOST}:${MARIADB_PORT}/${MARIADB_DB}"
crates/suprnova-magnetar/scripts/gate.sh --live

mysql_proper_admin "DROP DATABASE IF EXISTS ${MYSQL_PROPER_DB}"
mysql_proper_admin "CREATE DATABASE ${MYSQL_PROPER_DB}"
echo "=== engine pass 2/2: MySQL 8.4 (gate container, db ${MYSQL_PROPER_DB}) ==="
export MAGNETAR_MYSQL_TEST_URL="mysql://root:suprnova-gate@${MYSQL_PROPER_HOST}:${MYSQL_PROPER_PORT}/${MYSQL_PROPER_DB}"
crates/suprnova-magnetar/scripts/gate.sh --live
