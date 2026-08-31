#!/usr/bin/env bash
# Provision isolated live databases for the tracked Magnetar T2 profile.

set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

: "${SUPRNOVA_GATE_RUN_ID:?SUPRNOVA_GATE_RUN_ID must be set by gate-runner.py}"

if ! docker info >/dev/null 2>&1; then
    printf 'check-magnetar-live: the Docker daemon must be reachable.\n' >&2
    exit 1
fi

POSTGRES_CONTAINER="suprnova-gate-magnetar-pg-${SUPRNOVA_GATE_RUN_ID}-$$"
MYSQL_CONTAINER="suprnova-gate-magnetar-mysql-${SUPRNOVA_GATE_RUN_ID}-$$"
POSTGRES_PASSWORD="gate-$(head -c 12 /dev/urandom | od -An -tx1 | tr -d ' \n')"
MYSQL_PASSWORD="gate-$(head -c 12 /dev/urandom | od -An -tx1 | tr -d ' \n')"

cleanup() {
    docker rm -f "$POSTGRES_CONTAINER" "$MYSQL_CONTAINER" >/dev/null 2>&1 || true
}
trap cleanup EXIT INT TERM

docker run -d --rm --name "$POSTGRES_CONTAINER" \
    --label "suprnova-gate-run=${SUPRNOVA_GATE_RUN_ID}" \
    -e POSTGRES_PASSWORD="$POSTGRES_PASSWORD" \
    -e POSTGRES_DB=magnetar_test \
    -p 127.0.0.1::5432 \
    postgres:17-alpine >/dev/null

docker run -d --rm --name "$MYSQL_CONTAINER" \
    --label "suprnova-gate-run=${SUPRNOVA_GATE_RUN_ID}" \
    -e MYSQL_ROOT_PASSWORD="$MYSQL_PASSWORD" \
    -e MYSQL_DATABASE=magnetar_test \
    -e MYSQL_ROOT_HOST=% \
    -p 127.0.0.1::3306 \
    mysql:8.4 >/dev/null

POSTGRES_PORT="$(docker port "$POSTGRES_CONTAINER" 5432/tcp | sed -n '1s/.*://p')"
MYSQL_PORT="$(docker port "$MYSQL_CONTAINER" 3306/tcp | sed -n '1s/.*://p')"
if [[ -z "$POSTGRES_PORT" || -z "$MYSQL_PORT" ]]; then
    printf 'check-magnetar-live: could not determine mapped database ports.\n' >&2
    exit 1
fi

postgres_ready=0
for _ in $(seq 1 60); do
    if docker exec "$POSTGRES_CONTAINER" pg_isready -U postgres -d magnetar_test >/dev/null 2>&1; then
        postgres_ready=1
        break
    fi
    sleep 1
done
if [[ $postgres_ready -ne 1 ]]; then
    docker logs "$POSTGRES_CONTAINER" >&2 || true
    printf 'check-magnetar-live: Postgres never became ready.\n' >&2
    exit 1
fi

mysql_ready=0
for _ in $(seq 1 90); do
    if docker exec "$MYSQL_CONTAINER" mysqladmin ping \
        --host=127.0.0.1 --user=root --password="$MYSQL_PASSWORD" \
        --silent >/dev/null 2>&1; then
        mysql_ready=1
        break
    fi
    sleep 1
done
if [[ $mysql_ready -ne 1 ]]; then
    docker logs "$MYSQL_CONTAINER" >&2 || true
    printf 'check-magnetar-live: MySQL never became ready.\n' >&2
    exit 1
fi

export MAGNETAR_POSTGRES_TEST_URL="postgres://postgres:${POSTGRES_PASSWORD}@127.0.0.1:${POSTGRES_PORT}/magnetar_test"
export MAGNETAR_MYSQL_TEST_URL="mysql://root:${MYSQL_PASSWORD}@127.0.0.1:${MYSQL_PORT}/magnetar_test"

crates/suprnova-magnetar/scripts/gate.sh --live
