#!/usr/bin/env bash
# Run the RenderCache Tier 2 regressions against a real Redis.
#
# The Redis provider's guarantees - one accepted publication per fence, an
# evicted entry answering as a miss, a tampered hash decoding as a miss -
# live in Lua scripts and in Redis's own expiry. Neither has a fake, so the
# tests that prove them are `#[ignore]`d and nothing would run them. This
# script is what runs them.
#
# Usage:
#   scripts/check-redis.sh
#
# The container is disposable and removed on exit, success or failure.
#
# ## Port safety
#
# The host port is assigned by Docker (`-p 127.0.0.1::6379`) and read back,
# rather than pinned. Two reasons: a pinned 6379 collides with whatever the
# developer already runs, and - more importantly - a wrong guess would point
# these tests at somebody's real instance. They write and delete keys.
# Letting Docker choose makes that impossible by construction. The bind is
# loopback-only, matching the scaffold's compose templates.

set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

: "${SUPRNOVA_GATE_RUN_ID:?SUPRNOVA_GATE_RUN_ID must be set by gate-runner.py}"

if ! docker info >/dev/null 2>&1; then
    echo "check-redis: the Docker daemon must be reachable." >&2
    echo "    These tests need a real Redis; there is no in-memory fallback" >&2
    echo "    that would prove anything about the scripts or expiry." >&2
    exit 1
fi

CONTAINER="suprnova-gate-redis-${SUPRNOVA_GATE_RUN_ID}-$$"

cleanup() {
    docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
}
trap cleanup EXIT INT TERM

echo "starting disposable Redis (${CONTAINER})..."
docker run -d --rm --name "$CONTAINER" \
    --label "suprnova-gate-run=${SUPRNOVA_GATE_RUN_ID}" \
    -p 127.0.0.1::6379 \
    redis:7-alpine >/dev/null

# `docker port` reports the host side Docker picked, e.g. "127.0.0.1:49154".
HOST_PORT="$(docker port "$CONTAINER" 6379/tcp | head -1 | sed 's/.*://')"
if [[ -z "$HOST_PORT" ]]; then
    echo "check-redis: could not determine the mapped host port." >&2
    exit 1
fi
echo "    mapped to 127.0.0.1:${HOST_PORT}"

# Wait for readiness, bounded at 60 attempts a second apart. `redis-cli ping`
# runs inside the container, so PONG is the server's own answer rather than a
# TCP connect that succeeds before Redis finishes loading and starts
# accepting commands.
for _ in $(seq 1 60); do
    if [[ "$(docker exec "$CONTAINER" redis-cli ping 2>/dev/null)" == *PONG* ]]; then
        ready=1
        break
    fi
    sleep 1
done
if [[ "${ready:-0}" -ne 1 ]]; then
    echo "check-redis: Redis never became ready. Container log:" >&2
    docker logs "$CONTAINER" >&2 || true
    exit 1
fi

# The mapped port is the only value that reaches the tests, and it reaches
# them as this URL and nothing else.
export REDIS_TEST_URL="redis://127.0.0.1:${HOST_PORT}/"

# Serial, always. These tests share one instance and several of them inspect
# key counts and run bounded scans; in parallel they see each other's keys
# and fail in ways that look like product bugs.
#
# `tiers` is a mixed submodule: SQLite tier tests run unconditionally beside
# Postgres-, MySQL-, and Redis-tagged `#[ignore]`d ones. Select the
# Redis-tagged ones by name so this run never trips a Postgres-only test for
# want of a Postgres.
#
# `--ignored tiers::live_redis` exits 0 when the filter matches nothing, so a
# renamed test would silently stop testing Redis here while the gate stayed
# green. Assert on the output, not just the exit code: name representative
# tests, and refuse a summary line that reports nothing ran.
echo
echo "==> cargo test -p suprnova --test render_cache -- --ignored tiers::live_redis"
if ! redis_out="$(cargo test -p suprnova --test render_cache -- --ignored --test-threads=1 tiers::live_redis 2>&1)"; then
    echo "$redis_out"
    exit 1
fi
echo "$redis_out"
for redis_test in \
    live_redis_publish_fences_and_eviction_is_a_miss \
    live_redis_two_coordinators_lead_once_and_bypass_once \
    live_redis_a_lease_is_taken_over_by_store_time_and_the_former_leader_is_fenced \
    live_redis_the_redis_profile_publishes_to_the_redis_l1_and_serves_from_it; do
    if ! grep -qE "^test tiers::${redis_test} \.\.\. ok" <<<"$redis_out"; then
        echo "check-redis: ${redis_test} did not report ok (filter may have matched nothing)" >&2
        exit 1
    fi
done
# The exact number of `tiers::live_redis` tests, pinned rather than "more
# than none" so a test that quietly stops being selected - renamed out of
# the prefix, or its `#[ignore]` dropped - fails here instead of shrinking
# the run in silence. Update this number when a live_redis test is added or
# removed.
if ! grep -qE "^test result: ok\. 16 passed" <<<"$redis_out"; then
    echo "check-redis: the summary line does not report exactly 16 passed" >&2
    exit 1
fi

echo
echo "check-redis: OK"
