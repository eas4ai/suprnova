#!/usr/bin/env bash
# Run the framework's MySQL-only attribute-write regression against a
# disposable, loopback-only MariaDB instance.

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

echo
echo "check-mysql: OK"
