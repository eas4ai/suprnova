#!/usr/bin/env bash
#
# Seed one benchmark database. Run once per stack, same arguments both
# times — the data is generated from pure functions of the row index, so
# the two databases come out identical by construction rather than by
# later assertion.
#
#   bench/seed/seed.sh                 # full scale, ~400M rows
#   USERS=1000 bench/seed/seed.sh      # 1/1000 smoke test, seconds
#
# Scale is expressed as a user count; everything else derives from it, so
# the shape of the dataset is identical at any size. Smoke-test a schema
# change at USERS=1000 before committing a machine to the full load.
#
# What the smoke test cannot cover is anything that only breaks at scale.
# The 1/1000 run executes every statement, but its values sit four orders
# of magnitude below the int4 ceiling that the full run's hash arithmetic
# runs up against — see the overflow note at the top of load.sql, which is
# exactly this failure and cost a full load to find. Green at USERS=1000
# means the SQL is well-formed, not that it survives 200M rows.
#
# Measured, on the benchmark host (EPYC 4545P, NVMe, synchronous_commit
# off): the full 400M-row seed took 2636s end to end, 2528s of it in the
# load. Read that load figure as an upper bound rather than the expected
# one — that run carried the five bench indexes throughout, because
# TRUNCATE preserves them and an earlier smoke test had created them (see
# the note at the top of load.sql). load.sql drops them now, so a clean
# run should be faster in the load and slower in the index build.
#
# The index build at full scale is therefore still untimed: on that run it
# reported 4s, which was five `already exists, skipping` notices rather
# than any work.

set -euo pipefail
SEED_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

: "${PGHOST:=127.0.0.1}"
: "${PGPORT:=5433}"
: "${PGUSER:=bench}"
: "${PGDATABASE:=bench}"
# `: "${VAR:=default}"` assigns but does not export, and psql reads these
# from the environment — without this line the defaults above are decorative
# and psql silently falls back to the OS user, which inside the Postgres
# container is root. That failure reads as `role "root" does not exist`,
# which points at the database rather than at this script.
export PGHOST PGPORT PGUSER PGDATABASE

: "${USERS:=1000000}"
: "${POSTS_PER_USER:=50}"
: "${COMMENTS_PER_POST:=4}"
: "${TAGS_PER_POST:=3}"
: "${TAGS:=1000}"

# Must verify against the framework's own hasher — a hash pasted from
# elsewhere leaves every seeded account unable to log in, which surfaces
# as a warmup failure at the far end of a multi-hour load. Generate it
# from the app and pass it in.
: "${PASSWORD_HASH:?set PASSWORD_HASH to a hash produced by the framework hasher}"

POSTS=$(( USERS * POSTS_PER_USER ))
COMMENTS=$(( POSTS * COMMENTS_PER_POST ))
TAGGABLES=$(( POSTS * TAGS_PER_POST ))

psql() { command psql -v ON_ERROR_STOP=1 -q "$@"; }

echo "==> target ${PGUSER}@${PGHOST}:${PGPORT}/${PGDATABASE}"
printf '    users %s  posts %s  comments %s  taggables %s  tags %s\n' \
    "$USERS" "$POSTS" "$COMMENTS" "$TAGGABLES" "$TAGS"

started=$(date +%s)

psql -f "$SEED_DIR/load.sql" \
    -v users="$USERS" \
    -v posts="$POSTS" \
    -v comments="$COMMENTS" \
    -v taggables="$TAGGABLES" \
    -v tags="$TAGS" \
    -v posts_per_user="$POSTS_PER_USER" \
    -v comments_per_post="$COMMENTS_PER_POST" \
    -v tags_per_post="$TAGS_PER_POST" \
    -v password_hash="$PASSWORD_HASH"

loaded=$(date +%s)
echo "==> loaded in $(( loaded - started ))s"

psql -f "$SEED_DIR/indexes.sql"
echo "==> indexed in $(( $(date +%s) - loaded ))s"

# ---------------------------------------------------------------------
# Verification. A seeder that reports success without checking is how a
# benchmark ends up measuring a table that is not the size it claims.
# ---------------------------------------------------------------------

echo
echo "==> row counts"
fail=0
check_count() {
    local table="$1" want="$2"
    local got
    got="$(psql -Atc "SELECT count(*) FROM ${table}")"
    if [[ "$got" == "$want" ]]; then
        printf '    %-12s %14s  ok\n' "$table" "$got"
    else
        printf '    %-12s %14s  MISMATCH (wanted %s)\n' "$table" "$got" "$want"
        fail=1
    fi
}
check_count users "$USERS"
check_count profiles "$USERS"
check_count role_user "$USERS"
check_count posts "$POSTS"
check_count comments "$COMMENTS"
check_count tags "$TAGS"
check_count roles 5

# taggables is the one table allowed to come in short: three hash-drawn
# tags per post occasionally collide on the unique index and are dropped.
# Report the shortfall rather than asserting an exact figure, so a large
# unexpected loss is still visible.
tg="$(psql -Atc 'SELECT count(*) FROM taggables')"
dropped=$(( TAGGABLES - tg ))
# Basis points via integer math rather than bc — this runs inside the
# Postgres image, which has psql and bash but no arbitrary-precision
# calculator, and a seeder that needs extra packages installed is a
# seeder that does not run.
bp=0
(( TAGGABLES > 0 )) && bp=$(( dropped * 10000 / TAGGABLES ))
printf '    %-12s %14s  (%s dropped to unique conflicts, %d.%02d%%)\n' \
    taggables "$tg" "$dropped" "$(( bp / 100 ))" "$(( bp % 100 ))"

echo
echo "==> table sizes"
psql -Atc "
SELECT rpad(relname, 12) || lpad(pg_size_pretty(pg_total_relation_size(relid)), 10)
FROM pg_stat_user_tables
WHERE relname IN ('users','posts','comments','profiles','tags','taggables','roles','role_user')
ORDER BY pg_total_relation_size(relid) DESC" | sed 's/^/    /'

# ---------------------------------------------------------------------
# Plan checks. The index set is only worth anything if the planner picks
# it. A sequential scan here means a tier would have been benchmarking a
# missing index instead of the framework.
# ---------------------------------------------------------------------

echo
echo "==> plan checks"
plan_check() {
    local label="$1" query="$2"
    local plan
    plan="$(psql -Atc "EXPLAIN (FORMAT TEXT) ${query}")"
    if grep -qi "Seq Scan" <<<"$plan"; then
        printf '    %-28s SEQ SCAN — index not used\n' "$label"
        sed 's/^/        /' <<<"$plan"
        fail=1
    else
        printf '    %-28s ok\n' "$label"
    fi
}
plan_check "posts by author" \
    "SELECT * FROM posts WHERE author_id = 42"
plan_check "public posts ordered" \
    "SELECT * FROM posts WHERE is_public = true ORDER BY id LIMIT 20"
plan_check "recent feed" \
    "SELECT * FROM posts ORDER BY created_at DESC, id DESC LIMIT 20"
plan_check "tags of a post" \
    "SELECT * FROM taggables WHERE taggable_id = 42 AND taggable_type = 'post'"
plan_check "comments of a post" \
    "SELECT * FROM comments WHERE commentable_id = 42 AND commentable_type = 'post'"
plan_check "user by email" \
    "SELECT * FROM users WHERE email = 'user42@bench.local'"

echo
if (( fail )); then
    echo "SEED FAILED — see mismatches above"
    exit 1
fi
echo "==> seed complete in $(( $(date +%s) - started ))s"
