#!/usr/bin/env bash
# Builds the Laravel 13 fixtures the laravel_database tests load:
# laravel-13-sqlite.json, laravel-13-mysql.json and laravel-13-pgsql.json.
#
# Each one holds the schema Laravel's own migrations create on that engine
# (the Laravel 13 skeleton, laravel/pennant, spatie/laravel-permission, the
# notifications table, and the overlay's posts, comments and images tables)
# and the rows Laravel itself writes into it: users with $2y$ hashes,
# polymorphic rows, notifications, queued, batched and failed jobs, sessions,
# Pennant flags, spatie roles and permissions. The tests never run this
# script and never download anything; they load the committed JSON files.
#
# Tools: php 8.4 with pdo_sqlite, pdo_mysql and pdo_pgsql (on PATH, or at
# /home/linuxbrew/.linuxbrew/bin/php), composer, the mariadb or mysql client,
# psql, and network access for the composer install. The package versions
# come from app/composer.lock, the skeleton version from app/composer.json.
#
# Environment:
#   LDB_MYSQL_HOST, LDB_MYSQL_PORT, LDB_MYSQL_USER (default 127.0.0.1, 3317,
#   root) and LDB_MYSQL_PASSWORD (required); LDB_PG_HOST, LDB_PG_PORT,
#   LDB_PG_USER (default 127.0.0.1, 5432, suprnova_gate) and LDB_PG_PASSWORD
#   (required). The user needs the right to create and drop databases.
#
# It creates throwaway databases ldb_fixture_<pid> and ldb_fixture_<pid>_replay
# on MySQL and Postgres and drops them on exit. Work happens in a temporary
# directory under ${TMPDIR}, also removed on exit.
#
# Usage: generate.sh [sqlite] [mysql] [pgsql]   (no argument builds all three)
# After building each file it replays the file into an empty database and
# checks it with verify.php.
set -euo pipefail

FIXTURES="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ENGINES=("$@")
[ "${#ENGINES[@]}" -gt 0 ] || ENGINES=(sqlite mysql pgsql)
for engine in "${ENGINES[@]}"; do
    case "$engine" in
        sqlite|mysql|pgsql) ;;
        *) echo "generate.sh: unknown engine '$engine' (expected sqlite, mysql or pgsql)" >&2; exit 2 ;;
    esac
done

PHP="$(command -v php || true)"
[ -n "$PHP" ] || PHP=/home/linuxbrew/.linuxbrew/bin/php
[ -x "$PHP" ] || { echo "generate.sh: php 8.4 is not on PATH" >&2; exit 1; }
"$PHP" -r 'exit(PHP_MAJOR_VERSION === 8 && PHP_MINOR_VERSION === 4 ? 0 : 1);' \
    || { echo "generate.sh: php 8.4 is required, $("$PHP" -r 'echo PHP_VERSION;') found" >&2; exit 1; }
COMPOSER="$(command -v composer || true)"
[ -n "$COMPOSER" ] || { echo "generate.sh: composer is not on PATH" >&2; exit 1; }
MYSQL_CLIENT="$(command -v mariadb || command -v mysql || true)"

needs_mysql=false
needs_pg=false
for engine in "${ENGINES[@]}"; do
    [ "$engine" = mysql ] && needs_mysql=true
    [ "$engine" = pgsql ] && needs_pg=true
done

LDB_MYSQL_HOST="${LDB_MYSQL_HOST:-127.0.0.1}"
LDB_MYSQL_PORT="${LDB_MYSQL_PORT:-3317}"
LDB_MYSQL_USER="${LDB_MYSQL_USER:-root}"
LDB_PG_HOST="${LDB_PG_HOST:-127.0.0.1}"
LDB_PG_PORT="${LDB_PG_PORT:-5432}"
LDB_PG_USER="${LDB_PG_USER:-suprnova_gate}"
if $needs_mysql; then
    : "${LDB_MYSQL_PASSWORD:?set LDB_MYSQL_PASSWORD to build the MySQL fixture}"
    [ -n "$MYSQL_CLIENT" ] || { echo "generate.sh: the mariadb or mysql client is not on PATH" >&2; exit 1; }
fi
if $needs_pg; then
    : "${LDB_PG_PASSWORD:?set LDB_PG_PASSWORD to build the Postgres fixture}"
    command -v psql >/dev/null || { echo "generate.sh: psql is not on PATH" >&2; exit 1; }
fi

DB="ldb_fixture_$$"
REPLAY="${DB}_replay"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/ldb-fixtures.XXXXXX")"

# The MariaDB client warns that it skips server certificate checks when the
# password comes from MYSQL_PWD; that line is dropped, every other one kept.
my() {
    MYSQL_PWD="$LDB_MYSQL_PASSWORD" "$MYSQL_CLIENT" -h "$LDB_MYSQL_HOST" -P "$LDB_MYSQL_PORT" \
        -u "$LDB_MYSQL_USER" -e "$1" 2> >(grep -v 'ssl-verify-server-cert' >&2)
}
pg() {
    PGPASSWORD="$LDB_PG_PASSWORD" PGOPTIONS='-c client_min_messages=warning' psql -h "$LDB_PG_HOST" \
        -p "$LDB_PG_PORT" -U "$LDB_PG_USER" -d postgres -v ON_ERROR_STOP=1 -qAtc "$1"
}

cleanup() {
    if $needs_mysql; then
        my "DROP DATABASE IF EXISTS ${DB}" >/dev/null 2>&1 || true
        my "DROP DATABASE IF EXISTS ${REPLAY}" >/dev/null 2>&1 || true
    fi
    if $needs_pg; then
        pg "DROP DATABASE IF EXISTS ${DB}" >/dev/null 2>&1 || true
        pg "DROP DATABASE IF EXISTS ${REPLAY}" >/dev/null 2>&1 || true
    fi
    rm -rf "$WORK"
}
trap cleanup EXIT INT TERM

# The Laravel app: the pinned skeleton, the committed lock file, the overlay.
SKELETON="$("$PHP" -r '$c = json_decode(file_get_contents($argv[1]), true); echo str_replace(" ", "=", $c["extra"]["suprnova-fixture"]["skeleton"]);' "$FIXTURES/app/composer.json")"
APP="$WORK/app"
"$COMPOSER" create-project "$SKELETON" "$APP" --no-install --no-scripts --no-interaction --quiet
cp -R "$FIXTURES/app/." "$APP/"
(
    cd "$APP"
    "$COMPOSER" install --no-dev --no-scripts --no-interaction --quiet
    "$PHP" artisan package:discover --quiet
    "$PHP" artisan vendor:publish --provider='Laravel\Pennant\PennantServiceProvider' --quiet
    "$PHP" artisan vendor:publish --provider='Spatie\Permission\PermissionServiceProvider' --quiet
    "$PHP" artisan make:notifications-table --quiet
)

# Writes the app's .env for one engine and database name.
write_env() {
    local engine="$1" database="$2"
    {
        echo "APP_NAME=Laravel"
        echo "APP_ENV=local"
        echo "APP_KEY="
        echo "APP_DEBUG=true"
        echo "APP_URL=http://localhost"
        echo "BCRYPT_ROUNDS=12"
        echo "LOG_CHANNEL=single"
        echo "QUEUE_CONNECTION=database"
        echo "SESSION_DRIVER=database"
        echo "CACHE_STORE=database"
        echo "PENNANT_STORE=database"
        echo "DB_CONNECTION=${engine}"
        case "$engine" in
            sqlite)
                echo "DB_DATABASE=${database}"
                ;;
            mysql)
                echo "DB_HOST=${LDB_MYSQL_HOST}"
                echo "DB_PORT=${LDB_MYSQL_PORT}"
                echo "DB_DATABASE=${database}"
                echo "DB_USERNAME=${LDB_MYSQL_USER}"
                echo "DB_PASSWORD=${LDB_MYSQL_PASSWORD}"
                ;;
            pgsql)
                echo "DB_HOST=${LDB_PG_HOST}"
                echo "DB_PORT=${LDB_PG_PORT}"
                echo "DB_DATABASE=${database}"
                echo "DB_USERNAME=${LDB_PG_USER}"
                echo "DB_PASSWORD=${LDB_PG_PASSWORD}"
                ;;
        esac
    } > "$APP/.env"
    (cd "$APP" && "$PHP" artisan key:generate --force --quiet)
}

for engine in "${ENGINES[@]}"; do
    out="$FIXTURES/laravel-13-${engine}.json"
    case "$engine" in
        sqlite)
            built="$WORK/built.sqlite"
            replay="$WORK/replay.sqlite"
            rm -f "$built" "$replay"
            touch "$built" "$replay"
            write_env sqlite "$built"
            built_dsn="sqlite:${built}"
            replay_dsn="sqlite:${replay}"
            verify_user=""
            verify_password=""
            ;;
        mysql)
            my "DROP DATABASE IF EXISTS ${DB}"
            my "DROP DATABASE IF EXISTS ${REPLAY}"
            my "CREATE DATABASE ${DB}"
            my "CREATE DATABASE ${REPLAY}"
            write_env mysql "$DB"
            built_dsn="mysql:host=${LDB_MYSQL_HOST};port=${LDB_MYSQL_PORT};dbname=${DB};charset=utf8mb4"
            replay_dsn="mysql:host=${LDB_MYSQL_HOST};port=${LDB_MYSQL_PORT};dbname=${REPLAY};charset=utf8mb4"
            verify_user="$LDB_MYSQL_USER"
            verify_password="$LDB_MYSQL_PASSWORD"
            ;;
        pgsql)
            pg "DROP DATABASE IF EXISTS ${DB}" >/dev/null
            pg "DROP DATABASE IF EXISTS ${REPLAY}" >/dev/null
            pg "CREATE DATABASE ${DB}" >/dev/null
            pg "CREATE DATABASE ${REPLAY}" >/dev/null
            write_env pgsql "$DB"
            built_dsn="pgsql:host=${LDB_PG_HOST};port=${LDB_PG_PORT};dbname=${DB}"
            replay_dsn="pgsql:host=${LDB_PG_HOST};port=${LDB_PG_PORT};dbname=${REPLAY}"
            verify_user="$LDB_PG_USER"
            verify_password="$LDB_PG_PASSWORD"
            ;;
    esac
    (cd "$APP" && "$PHP" artisan fixtures:build "$engine" "$out")
    LDB_VERIFY_BUILT_DSN="$built_dsn" LDB_VERIFY_REPLAY_DSN="$replay_dsn" \
        LDB_VERIFY_USER="$verify_user" LDB_VERIFY_PASSWORD="$verify_password" \
        "$PHP" "$FIXTURES/verify.php" "$out"
done
