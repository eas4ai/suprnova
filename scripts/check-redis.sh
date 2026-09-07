#!/usr/bin/env bash
# Run every Redis-only suite in this repository against a real Redis:
# integration suites, in-source unit suites, and the one behind a non-default
# feature.
#
# The Redis provider's guarantees - one accepted publication per fence, an
# evicted entry answering as a miss, a tampered hash decoding as a miss -
# live in Lua scripts and in Redis's own expiry. Neither has a fake, so the
# tests that prove them are `#[ignore]`d and nothing would run them. The same
# is true of the older Redis suites this repository already carried: the
# queue driver's streams and reclamation, the queue's own in-source fenced
# settlement and epoch revalidation proofs, the idempotency lease's takeover,
# the cache store's TTLs, tag indexes, and connection retry, and the
# broadcasting fanout hub, which needs `--features broadcasting-fanout` and
# therefore a second build. This script is what runs all of them, in one
# container, in one order.
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
# `tiers/redis` holds the Redis-tagged `#[ignore]`d tests plus two that need
# no Redis at all; its siblings hold the database tier's. Select this
# submodule by name so the run never trips a Postgres-only test for want of a
# Postgres, and so a submodule that is renamed or moved stops the step rather
# than quietly selecting nothing.
#
# `--ignored tiers::redis::live_redis` exits 0 when the filter matches
# nothing, so a renamed test would silently stop testing Redis here while the
# gate stayed green. Assert on the output, not just the exit code: name
# representative tests, and refuse a summary line that reports nothing ran.
echo
echo "==> cargo test -p suprnova --test render_cache -- --ignored tiers::redis::live_redis"
if ! redis_out="$(cargo test -p suprnova --test render_cache -- --ignored --test-threads=1 tiers::redis::live_redis 2>&1)"; then
    echo "$redis_out"
    exit 1
fi
echo "$redis_out"
for redis_test in \
    live_redis_publish_fences_and_eviction_is_a_miss \
    live_redis_two_coordinators_lead_once_and_bypass_once \
    live_redis_a_lease_is_taken_over_by_store_time_and_the_former_leader_is_fenced \
    live_redis_the_redis_profile_publishes_to_the_redis_l1_and_serves_from_it; do
    if ! grep -qE "^test tiers::redis::${redis_test} \.\.\. ok" <<<"$redis_out"; then
        echo "check-redis: ${redis_test} did not report ok (filter may have matched nothing)" >&2
        exit 1
    fi
done
# The exact number of `tiers::redis::live_redis` tests, pinned rather than "more
# than none" so a test that quietly stops being selected - renamed out of
# the prefix, or its `#[ignore]` dropped - fails here instead of shrinking
# the run in silence. Update this number when a live_redis test is added or
# removed.
if ! grep -qE "^test result: ok\. 16 passed" <<<"$redis_out"; then
    echo "check-redis: the summary line does not report exactly 16 passed" >&2
    exit 1
fi

# --- The other Redis suites, which no gate step has ever run ---------------
#
# Everything above is the RenderCache tier. What follows is older: the queue
# driver, the idempotency lease, and the cache store each carry `#[ignore]`d
# Redis suites that have always been gated behind a URL nobody set, so
# nothing ran them. They reach the same disposable instance this run already
# started, through the variable each one resolves:
#
#   QUEUE_REDIS_TEST_URL  framework/tests/queue/redis.rs, and the in-source
#                         suite in framework/src/queue/redis.rs, whose
#                         `configured_redis_test_url()` reads this name and
#                         then REDIS_URL
#   REDIS_URL             framework/tests/queue/reclaim_attempts.rs, which
#                         reads that name and no other
#   CACHE_REDIS_TEST_URL  framework/tests/cache/redis_integration.rs,
#                         framework/tests/cache/redis_retry.rs, and the one
#                         Redis test in framework/tests/idempotency
#   REDIS_BROADCAST_URL   framework/tests/broadcasting/fanout.rs, exported at
#                         its own block below because that suite needs a
#                         feature build
#
# The order below is deliberate, and the last block is the reason: the cache
# retry suite issues `CLIENT KILL TYPE normal`, which disconnects every other
# client on the instance - including any still-open connection manager a
# previous suite left behind. It therefore runs after every other suite has
# finished with the container, and the container is removed on exit
# immediately afterwards.
export QUEUE_REDIS_TEST_URL="$REDIS_TEST_URL"
export CACHE_REDIS_TEST_URL="$REDIS_TEST_URL"
export REDIS_URL="$REDIS_TEST_URL"

# The queue's Redis driver: streams, the pending-entries list, reclamation,
# and the delayed set. Serial for the same reason as everything else here.
echo
echo "==> cargo test -p suprnova --test queue -- --ignored redis::"
if ! queue_redis_out="$(cargo test -p suprnova --test queue -- --ignored --test-threads=1 redis:: 2>&1)"; then
    echo "$queue_redis_out"
    exit 1
fi
echo "$queue_redis_out"
for queue_redis_test in \
    redis_driver_push_pop_ack_round_trip \
    redis_driver_concurrent_pops_claim_one_distinct_entry_each \
    redis_driver_reclaims_another_consumers_expired_delivery_to_itself \
    redis_driver_clear_epoch_fences_an_identical_recreated_delivery; do
    if ! grep -qE "^test redis::${queue_redis_test} \.\.\. ok" <<<"$queue_redis_out"; then
        echo "check-redis: ${queue_redis_test} did not report ok (filter may have matched nothing)" >&2
        exit 1
    fi
done
# The exact number of ignored tests in framework/tests/queue/redis.rs, pinned
# for the reason the tier count above is. Update it when one is added or
# removed.
if ! grep -qE "^test result: ok\. 19 passed" <<<"$queue_redis_out"; then
    echo "check-redis: the queue Redis summary line does not report exactly 19 passed" >&2
    exit 1
fi

# The queue's attempt accounting across a lost worker, which is a separate
# submodule and reads `REDIS_URL` rather than `QUEUE_REDIS_TEST_URL`.
echo
echo "==> cargo test -p suprnova --test queue -- --ignored reclaim_attempts::redis_"
if ! reclaim_out="$(cargo test -p suprnova --test queue -- --ignored --test-threads=1 reclaim_attempts::redis_ 2>&1)"; then
    echo "$reclaim_out"
    exit 1
fi
echo "$reclaim_out"
# These two sit one module deeper than the filter, in
# `reclaim_attempts::redis_driver`; the guard names the path the harness
# prints, not the filter.
for reclaim_test in \
    redis_reclaim_after_worker_loss_consumes_an_attempt \
    redis_first_delivery_does_not_consume_an_attempt; do
    if ! grep -qE "^test reclaim_attempts::redis_driver::${reclaim_test} \.\.\. ok" <<<"$reclaim_out"; then
        echo "check-redis: ${reclaim_test} did not report ok (filter may have matched nothing)" >&2
        exit 1
    fi
done
if ! grep -qE "^test result: ok\. 2 passed" <<<"$reclaim_out"; then
    echo "check-redis: the reclaim-attempts summary line does not report exactly 2 passed" >&2
    exit 1
fi

# The queue driver's own in-source proofs: fenced settlement (the receipt
# replay, the live-PEL refusal, the stale prior-epoch receipt, the XACK
# rollback, the preserved delayed member, the oversized batch) and epoch
# revalidation. They are unit tests inside framework/src/queue/redis.rs, so
# they need `--lib` rather than an integration binary - the same shape
# check-postgres.sh uses for the SQL store's guarded upsert - and they resolve
# their URL through `configured_redis_test_url()`, which reads
# QUEUE_REDIS_TEST_URL and then REDIS_URL, both exported above.
echo
echo "==> cargo test -p suprnova --lib -- --ignored queue::redis::tests"
if ! queue_lib_out="$(cargo test -p suprnova --lib -- --ignored --test-threads=1 queue::redis::tests 2>&1)"; then
    echo "$queue_lib_out"
    exit 1
fi
echo "$queue_lib_out"
for queue_lib_test in \
    fenced_settlement_receipt_replays_a_lost_success_without_duplicates \
    epoch_revalidation_reloads_the_recreated_stream_payload \
    fenced_settlement_backend_rejects_oversized_batch_before_receipt; do
    if ! grep -qE "^test queue::redis::tests::${queue_lib_test} \.\.\. ok" <<<"$queue_lib_out"; then
        echo "check-redis: ${queue_lib_test} did not report ok (filter may have matched nothing)" >&2
        exit 1
    fi
done
# The exact number of `#[ignore]`d tests in framework/src/queue/redis.rs.
# Update it when one is added or removed.
if ! grep -qE "^test result: ok\. 7 passed" <<<"$queue_lib_out"; then
    echo "check-redis: the in-source queue summary line does not report exactly 7 passed" >&2
    exit 1
fi

# The idempotency lease against a real Redis: a body still running when its
# lease is taken over must report `Unfenced`, which a memory cache cannot
# show.
echo
echo "==> cargo test -p suprnova --test idempotency -- --ignored idempotency::redis_"
if ! idempotency_out="$(cargo test -p suprnova --test idempotency -- --ignored --test-threads=1 idempotency::redis_ 2>&1)"; then
    echo "$idempotency_out"
    exit 1
fi
echo "$idempotency_out"
if ! grep -qE "^test idempotency::redis_synchronously_blocked_body_reports_unfenced_after_takeover \.\.\. ok" <<<"$idempotency_out"; then
    echo "check-redis: the idempotency takeover test did not report ok (filter may have matched nothing)" >&2
    exit 1
fi
if ! grep -qE "^test result: ok\. 1 passed" <<<"$idempotency_out"; then
    echo "check-redis: the idempotency summary line does not report exactly 1 passed" >&2
    exit 1
fi

# The cache store's Redis integration: sub-second TTLs, tag indexes, the
# SCAN-based flush, and the lock keyspace - all things a fake cannot prove.
echo
echo "==> cargo test -p suprnova --test cache -- --ignored redis_integration::"
if ! cache_redis_out="$(cargo test -p suprnova --test cache -- --ignored --test-threads=1 redis_integration:: 2>&1)"; then
    echo "$cache_redis_out"
    exit 1
fi
echo "$cache_redis_out"
for cache_redis_test in \
    redis_put_with_subsecond_ttl_expires_correctly \
    redis_flush_uses_scan_and_clears_the_keyspace \
    redis_tagged_writes_can_be_flushed_by_tag \
    redis_flush_tags_spans_multiple_scan_rounds; do
    if ! grep -qE "^test redis_integration::${cache_redis_test} \.\.\. ok" <<<"$cache_redis_out"; then
        echo "check-redis: ${cache_redis_test} did not report ok (filter may have matched nothing)" >&2
        exit 1
    fi
done
# The exact number of ignored tests in
# framework/tests/cache/redis_integration.rs. Update it when one is added or
# removed.
if ! grep -qE "^test result: ok\. 17 passed" <<<"$cache_redis_out"; then
    echo "check-redis: the cache Redis summary line does not report exactly 17 passed" >&2
    exit 1
fi

# The broadcasting fanout hub against a real Redis stream: two hubs over one
# stream exchanging envelopes, and the two write-failure reports. This suite
# is behind `--features broadcasting-fanout`, which is not in the default
# feature set, so this block builds a second feature configuration - it is the
# slowest block here on a cold target and the only one that compiles anything
# the rest of the gate did not.
#
# The export is unconditional and derived from this run's own container port,
# and that is what makes the block mean something: each of these three tests
# reads REDIS_BROADCAST_URL and `return`s silently when it is unset or empty
# (framework/tests/broadcasting/fanout.rs:519 and its twins), and a test that
# returns early still prints `... ok`. A vacuous pass is therefore
# indistinguishable from a real one in the output, so the named guards and the
# count pin below cannot catch it; only the export can, and it cannot be unset
# here because it is assigned from HOST_PORT above.
export REDIS_BROADCAST_URL="$REDIS_TEST_URL"
echo
echo "==> cargo test -p suprnova --features broadcasting-fanout --test broadcasting -- --ignored fanout::redis_"
if ! fanout_out="$(cargo test -p suprnova --features broadcasting-fanout --test broadcasting -- --ignored --test-threads=1 fanout::redis_ 2>&1)"; then
    echo "$fanout_out"
    exit 1
fi
echo "$fanout_out"
for fanout_test in \
    redis_backend_cross_hub_fanout \
    redis_publish_reports_stream_write_failure \
    redis_presence_reports_stream_write_failure; do
    if ! grep -qE "^test fanout::${fanout_test} \.\.\. ok" <<<"$fanout_out"; then
        echo "check-redis: ${fanout_test} did not report ok (filter may have matched nothing)" >&2
        exit 1
    fi
done
# The exact number of `#[ignore]`d tests in
# framework/tests/broadcasting/fanout.rs. Update it when one is added or
# removed.
if ! grep -qE "^test result: ok\. 3 passed" <<<"$fanout_out"; then
    echo "check-redis: the fanout summary line does not report exactly 3 passed" >&2
    exit 1
fi

# LAST, and it must be: this suite kills every other client on the instance
# (see the ordering note above). It also skips itself with a printed line
# rather than failing when `CACHE_REDIS_TEST_URL` is unset, so the skip is
# refused explicitly here - a silent skip would restore exactly the hole this
# step exists to close.
#
# `--nocapture` is what makes that refusal able to fire: the skip is an
# `eprintln!` inside a test that then passes, and libtest captures a passing
# test's output unless it is told not to, so without this the marker never
# reaches the variable the check below greps. check-postgres.sh's workflow
# block passes it for the same reason.
echo
echo "==> cargo test -p suprnova --test cache -- --ignored redis_retry::"
if ! cache_retry_out="$(cargo test -p suprnova --test cache -- --ignored --test-threads=1 --nocapture redis_retry:: 2>&1)"; then
    echo "$cache_retry_out"
    exit 1
fi
echo "$cache_retry_out"
if grep -q "skipping:" <<<"$cache_retry_out"; then
    echo "check-redis: the cache retry suite SKIPPED itself despite" >&2
    echo "    CACHE_REDIS_TEST_URL being set. That is the silent-pass bug this" >&2
    echo "    step exists to prevent - fix the gate, not this check." >&2
    exit 1
fi
if ! grep -qE "^test redis_retry::redis_get_survives_a_killed_connection \.\.\. ok" <<<"$cache_retry_out"; then
    echo "check-redis: the cache retry test did not report ok (filter may have matched nothing)" >&2
    exit 1
fi
if ! grep -qE "^test result: ok\. 1 passed" <<<"$cache_retry_out"; then
    echo "check-redis: the cache retry summary line does not report exactly 1 passed" >&2
    exit 1
fi

echo
echo "check-redis: OK"
