//! Live Redis integration test for the queue driver. Requires a Redis
//! daemon on `redis://127.0.0.1:6379`.
//!
//! Run with `cargo test -p suprnova --test queue_redis -- --ignored`.

use chrono::Utc;
use std::time::Duration;
use suprnova::queue::driver::{QueueDriver, Settled};
use suprnova::queue::redis::RedisQueueDriver;
use suprnova::queue::{BackoffSchedule, CURRENT_SCHEMA_VERSION, Envelope};
use uuid::Uuid;

/// Where to reach Redis. Matches `cache_redis_integration`'s resolution so
/// one env var points every Redis-backed suite at the same instance -
/// including a throwaway one, which is the only way to run these without
/// writing into whatever Redis happens to be on the default port.
fn redis_url() -> String {
    std::env::var("QUEUE_REDIS_TEST_URL")
        .or_else(|_| std::env::var("REDIS_URL"))
        .unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string())
}

async fn redis_connection() -> redis::aio::ConnectionManager {
    let client = redis::Client::open(redis_url()).expect("valid Redis URL");
    redis::aio::ConnectionManager::new(client)
        .await
        .expect("connect direct Redis client")
}

async fn only_stream_entry_id(stream: &str) -> String {
    let mut conn = redis_connection().await;
    let reply: redis::streams::StreamRangeReply = redis::cmd("XRANGE")
        .arg(stream)
        .arg("-")
        .arg("+")
        .query_async(&mut conn)
        .await
        .expect("read stream entry");
    assert_eq!(reply.ids.len(), 1, "test stream must contain one entry");
    reply.ids[0].id.clone()
}

fn env(name: &str) -> Envelope {
    let now = Utc::now();
    Envelope {
        schema_version: CURRENT_SCHEMA_VERSION,
        id: Uuid::new_v4(),
        job_name: name.into(),
        queue: None,
        payload: serde_json::json!({}),
        dispatched_at: now,
        available_at: now,
        attempts: 0,
        max_tries: 3,
        backoff: BackoffSchedule::default(),
        timeout_secs: None,
        fail_on_timeout: false,
        idempotency_key: None,
        unique_lock_owner: None,
        debounce_id: None,
        debounce_owner: None,
        batch_id: None,
        chain_remaining: Vec::new(),
    }
}

#[ignore = "requires a real Redis"]
#[tokio::test]
async fn redis_driver_push_pop_ack_round_trip() {
    let stream = format!("test-{}", uuid::Uuid::new_v4());
    let d = RedisQueueDriver::connect(&redis_url(), &stream, "g1", "c1", Duration::from_secs(60))
        .await
        .unwrap();

    d.push(env("R")).await.unwrap();

    let r1 = d.pop(Duration::from_secs(60)).await.unwrap().unwrap();
    assert_eq!(r1.envelope.job_name, "R");
    d.ack(&r1.token).await.unwrap();

    let none = d.pop(Duration::from_millis(50)).await.unwrap();
    assert!(none.is_none());
}

#[ignore = "requires a real Redis"]
#[tokio::test]
async fn redis_driver_ack_removes_the_exact_pending_entry() {
    let stream = format!("test-{}", uuid::Uuid::new_v4());
    let d = RedisQueueDriver::connect(
        &redis_url(),
        &stream,
        "g-ack-fence",
        "c-ack-fence",
        Duration::from_secs(60),
    )
    .await
    .unwrap();

    d.push(env("ack-fence")).await.unwrap();
    let reservation = d.pop(Duration::from_secs(5)).await.unwrap().unwrap();
    assert_eq!(d.reserved_size().await.unwrap(), 1);

    d.ack(&reservation.token).await.unwrap();

    assert_eq!(
        d.reserved_size().await.unwrap(),
        0,
        "ack must execute XACK rather than only queueing local bookkeeping"
    );
    d.clear().await.unwrap();
}

#[ignore = "requires a real Redis"]
#[tokio::test]
async fn redis_driver_concurrent_pops_claim_one_distinct_entry_each() {
    let stream = format!("test-{}", Uuid::new_v4());
    let d = RedisQueueDriver::connect(
        &redis_url(),
        &stream,
        "g-concurrent-pop",
        "c-concurrent-pop",
        Duration::from_secs(60),
    )
    .await
    .unwrap();
    d.push(env("concurrent-a")).await.unwrap();
    d.push(env("concurrent-b")).await.unwrap();

    let (first, second) =
        tokio::join!(d.pop(Duration::from_secs(5)), d.pop(Duration::from_secs(5)));
    let first = first.unwrap().expect("first concurrent delivery");
    let second = second.unwrap().expect("second concurrent delivery");

    assert_ne!(first.envelope.id, second.envelope.id);
    assert_ne!(first.token, second.token);
    assert_eq!(d.reserved_size().await.unwrap(), 2);
    d.ack(&first.token).await.unwrap();
    d.ack(&second.token).await.unwrap();
    assert_eq!(d.reserved_size().await.unwrap(), 0);
    d.clear().await.unwrap();
}

#[ignore = "requires a real Redis"]
#[tokio::test]
async fn redis_driver_settle_publishes_follow_up_and_acks_atomically() {
    let stream = format!("test-{}", uuid::Uuid::new_v4());
    let d = RedisQueueDriver::connect(
        &redis_url(),
        &stream,
        "g-settle-fence",
        "c-settle-fence",
        Duration::from_secs(60),
    )
    .await
    .unwrap();

    d.push(env("original")).await.unwrap();
    let reservation = d.pop(Duration::from_secs(5)).await.unwrap().unwrap();
    let result = d
        .settle(&reservation.token, &[env("follow-up")])
        .await
        .unwrap();

    assert_eq!(result, Settled::Atomically);
    assert_eq!(d.reserved_size().await.unwrap(), 0);
    assert_eq!(d.delayed_size().await.unwrap(), 1);

    let follow_up = d
        .pop(Duration::from_secs(5))
        .await
        .unwrap()
        .expect("the staged follow-up must be promoted by pop");
    assert_eq!(follow_up.envelope.job_name, "follow-up");
    d.ack(&follow_up.token).await.unwrap();
    d.clear().await.unwrap();
}

#[ignore = "requires a real Redis"]
#[tokio::test]
async fn redis_driver_settle_preserves_duplicate_follow_ups() {
    let stream = format!("test-{}", uuid::Uuid::new_v4());
    let d = RedisQueueDriver::connect(
        &redis_url(),
        &stream,
        "g-settle-duplicates",
        "c-settle-duplicates",
        Duration::from_secs(60),
    )
    .await
    .unwrap();

    d.push(env("original")).await.unwrap();
    let reservation = d.pop(Duration::from_secs(5)).await.unwrap().unwrap();
    let duplicate = env("duplicate-follow-up");
    let duplicate_id = duplicate.id;

    let result = d
        .settle(&reservation.token, &[duplicate.clone(), duplicate])
        .await
        .unwrap();

    assert_eq!(result, Settled::Atomically);
    assert_eq!(
        d.delayed_size().await.unwrap(),
        2,
        "ZSET member identity must not collapse equal envelope payloads"
    );

    let first = d.pop(Duration::from_secs(5)).await.unwrap().unwrap();
    let second = d.pop(Duration::from_secs(5)).await.unwrap().unwrap();
    assert_eq!(first.envelope.id, duplicate_id);
    assert_eq!(second.envelope.id, duplicate_id);
    assert_ne!(first.token, second.token);
    d.ack(&first.token).await.unwrap();
    d.ack(&second.token).await.unwrap();
    d.clear().await.unwrap();
}

#[ignore = "requires a real Redis"]
#[tokio::test]
async fn redis_driver_stale_settlement_cannot_publish_follow_ups() {
    let stream = format!("test-{}", uuid::Uuid::new_v4());
    let group = "g-stale-fence";
    let d = RedisQueueDriver::connect(
        &redis_url(),
        &stream,
        group,
        "consumer-old",
        Duration::from_secs(60),
    )
    .await
    .unwrap();

    d.push(env("original")).await.unwrap();
    let reservation = d.pop(Duration::from_secs(5)).await.unwrap().unwrap();
    let entry_id = only_stream_entry_id(&stream).await;

    let mut conn = redis_connection().await;
    // Claim to the same consumer name so only Redis's delivery generation
    // changes; an owner-only fence would incorrectly let the old token settle.
    let _: redis::Value = redis::cmd("XCLAIM")
        .arg(&stream)
        .arg(group)
        .arg("consumer-old")
        .arg(0)
        .arg(&entry_id)
        .query_async(&mut conn)
        .await
        .expect("advance the pending entry's delivery generation");

    let result = d
        .settle(&reservation.token, &[env("forbidden-follow-up")])
        .await
        .unwrap();

    assert_eq!(result, Settled::Stale);
    assert_eq!(d.delayed_size().await.unwrap(), 0);
    assert_eq!(d.reserved_size().await.unwrap(), 1);
    let length: i64 = redis::cmd("XLEN")
        .arg(&stream)
        .query_async(&mut conn)
        .await
        .unwrap();
    assert_eq!(length, 1, "stale settlement must not XADD a successor");
    d.clear().await.unwrap();
}

#[ignore = "requires a real Redis"]
#[tokio::test]
async fn redis_driver_reclaims_its_own_expired_delivery_with_a_new_generation() {
    let stream = format!("test-{}", Uuid::new_v4());
    let d = RedisQueueDriver::connect(
        &redis_url(),
        &stream,
        "g-self-reclaim",
        "consumer-self",
        Duration::from_millis(50),
    )
    .await
    .unwrap();

    d.push(env("self-reclaim")).await.unwrap();
    let first = d.pop(Duration::from_secs(5)).await.unwrap().unwrap();
    tokio::time::sleep(Duration::from_millis(75)).await;
    let reclaimed = d.pop(Duration::from_secs(5)).await.unwrap().unwrap();

    assert_eq!(reclaimed.envelope.id, first.envelope.id);
    assert_eq!(reclaimed.envelope.attempts, first.envelope.attempts + 1);
    assert_ne!(reclaimed.token, first.token);
    d.ack(&first.token).await.unwrap();
    assert_eq!(d.reserved_size().await.unwrap(), 1);
    d.ack(&reclaimed.token).await.unwrap();
    assert_eq!(d.reserved_size().await.unwrap(), 0);
    d.clear().await.unwrap();
}

#[ignore = "requires a real Redis"]
#[tokio::test]
async fn redis_driver_reclaims_another_consumers_expired_delivery_to_itself() {
    let stream = format!("test-{}", Uuid::new_v4());
    let group = "g-cross-reclaim";
    let first_driver = RedisQueueDriver::connect(
        &redis_url(),
        &stream,
        group,
        "consumer-a",
        Duration::from_millis(50),
    )
    .await
    .unwrap();

    first_driver.push(env("cross-reclaim")).await.unwrap();
    let first = first_driver
        .pop(Duration::from_secs(5))
        .await
        .unwrap()
        .unwrap();
    tokio::time::sleep(Duration::from_millis(75)).await;
    let second_driver = RedisQueueDriver::connect(
        &redis_url(),
        &stream,
        group,
        "consumer-b",
        Duration::from_millis(50),
    )
    .await
    .unwrap();
    let reclaimed = second_driver
        .pop(Duration::from_secs(5))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(reclaimed.envelope.id, first.envelope.id);
    assert_eq!(reclaimed.envelope.attempts, first.envelope.attempts + 1);
    first_driver.ack(&first.token).await.unwrap();
    assert_eq!(second_driver.reserved_size().await.unwrap(), 1);
    second_driver.ack(&reclaimed.token).await.unwrap();
    assert_eq!(second_driver.reserved_size().await.unwrap(), 0);
    second_driver.clear().await.unwrap();
}

#[ignore = "requires a real Redis"]
#[tokio::test]
async fn redis_driver_does_not_immediately_replay_a_fresh_claim() {
    let stream = format!("test-{}", Uuid::new_v4());
    let group = "g-claim-without-pel-replay";
    let first_driver = RedisQueueDriver::connect(
        &redis_url(),
        &stream,
        group,
        "consumer-a",
        Duration::from_millis(50),
    )
    .await
    .unwrap();
    first_driver.push(env("claim-once")).await.unwrap();
    let original = first_driver
        .pop(Duration::from_secs(5))
        .await
        .unwrap()
        .unwrap();

    tokio::time::sleep(Duration::from_millis(75)).await;
    let second_driver = RedisQueueDriver::connect(
        &redis_url(),
        &stream,
        group,
        "consumer-b",
        Duration::from_millis(50),
    )
    .await
    .unwrap();
    let reclaimed = second_driver
        .pop(Duration::from_secs(5))
        .await
        .unwrap()
        .expect("the expired delivery should be claimed exactly once");
    assert_eq!(reclaimed.envelope.id, original.envelope.id);

    let immediate = second_driver.pop(Duration::from_secs(5)).await.unwrap();
    assert!(
        immediate.is_none(),
        "a fresh claim must not be replayed from this consumer's PEL before visibility expiry"
    );

    second_driver.ack(&reclaimed.token).await.unwrap();
    second_driver.clear().await.unwrap();
}

#[ignore = "requires a real Redis"]
#[tokio::test]
async fn redis_driver_one_pop_registers_exactly_one_pel_delivery() {
    let stream = format!("test-{}", Uuid::new_v4());
    let group = "g-single-delivery";
    let driver = RedisQueueDriver::connect(
        &redis_url(),
        &stream,
        group,
        "consumer-single-delivery",
        Duration::from_secs(60),
    )
    .await
    .unwrap();
    driver.push(env("first")).await.unwrap();
    driver.push(env("second")).await.unwrap();

    let first = driver
        .pop(Duration::from_secs(5))
        .await
        .unwrap()
        .expect("first delivery");
    let mut conn = redis_connection().await;
    let pending: redis::streams::StreamPendingReply = redis::cmd("XPENDING")
        .arg(&stream)
        .arg(group)
        .query_async(&mut conn)
        .await
        .unwrap();
    assert_eq!(
        pending.count(),
        1,
        "one pop must not leave an unregistered SeaStreamer delivery in the PEL"
    );
    assert_eq!(driver.reserved_jobs(None).await.unwrap().len(), 1);

    driver.ack(&first.token).await.unwrap();
    let second = driver
        .pop(Duration::from_secs(5))
        .await
        .unwrap()
        .expect("second delivery");
    assert_ne!(first.envelope.id, second.envelope.id);
    driver.ack(&second.token).await.unwrap();
    driver.clear().await.unwrap();
}

#[ignore = "requires a real Redis"]
#[tokio::test]
async fn redis_driver_reclaim_cursor_advances_past_an_empty_scan_page() {
    let stream = format!("test-{}", Uuid::new_v4());
    let group = "g-reclaim-cursor";
    let mut conn = redis_connection().await;
    let mut entry_ids = Vec::new();
    for index in 0..12 {
        let payload = env(&format!("cursor-{index}")).to_json().unwrap();
        let entry_id: String = redis::cmd("XADD")
            .arg(&stream)
            .arg("*")
            .arg("msg")
            .arg(payload)
            .query_async(&mut conn)
            .await
            .unwrap();
        entry_ids.push(entry_id);
    }
    let _: () = redis::cmd("XGROUP")
        .arg("CREATE")
        .arg(&stream)
        .arg(group)
        .arg("0")
        .query_async(&mut conn)
        .await
        .unwrap();
    let _: redis::Value = redis::cmd("XREADGROUP")
        .arg("GROUP")
        .arg(group)
        .arg("consumer-a")
        .arg("COUNT")
        .arg(12)
        .arg("STREAMS")
        .arg(&stream)
        .arg(">")
        .query_async(&mut conn)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(75)).await;
    let _: redis::Value = redis::cmd("XCLAIM")
        .arg(&stream)
        .arg(group)
        .arg("consumer-a")
        .arg(0)
        .arg(&entry_ids[..10])
        .query_async(&mut conn)
        .await
        .unwrap();

    let driver = RedisQueueDriver::connect(
        &redis_url(),
        &stream,
        group,
        "consumer-b",
        Duration::from_millis(50),
    )
    .await
    .unwrap();
    assert!(
        driver
            .pop(Duration::from_millis(25))
            .await
            .unwrap()
            .is_none(),
        "the first scan page contains only freshly touched entries"
    );
    let reclaimed = driver
        .pop(Duration::from_secs(5))
        .await
        .unwrap()
        .expect("the persisted cursor must reach a later eligible entry");
    assert!(
        reclaimed.envelope.job_name == "cursor-10" || reclaimed.envelope.job_name == "cursor-11"
    );
    driver.ack(&reclaimed.token).await.unwrap();
    driver.clear().await.unwrap();
}

#[ignore = "requires a real Redis"]
#[tokio::test]
async fn redis_driver_clear_epoch_fences_an_identical_recreated_delivery() {
    let stream = format!("test-{}", Uuid::new_v4());
    let group = "g-clear-epoch";
    let consumer = "same-consumer";
    let old_driver = RedisQueueDriver::connect(
        &redis_url(),
        &stream,
        group,
        consumer,
        Duration::from_secs(60),
    )
    .await
    .unwrap();
    let mut conn = redis_connection().await;
    let old = env("before-clear");
    let old_json = old.to_json().unwrap();
    let old_id: String = redis::cmd("XADD")
        .arg(&stream)
        .arg("1-0")
        .arg("msg")
        .arg(old_json)
        .query_async(&mut conn)
        .await
        .unwrap();
    assert_eq!(old_id, "1-0");
    let old_reservation = old_driver
        .pop(Duration::from_secs(5))
        .await
        .unwrap()
        .unwrap();

    let clearing_driver = RedisQueueDriver::connect(
        &redis_url(),
        &stream,
        group,
        "clearer",
        Duration::from_secs(60),
    )
    .await
    .unwrap();
    clearing_driver.clear().await.unwrap();

    let replacement = env("after-clear");
    let replacement_json = replacement.to_json().unwrap();
    let replacement_id: String = redis::cmd("XADD")
        .arg(&stream)
        .arg("1-0")
        .arg("msg")
        .arg(replacement_json)
        .query_async(&mut conn)
        .await
        .unwrap();
    assert_eq!(replacement_id, "1-0");
    let replacement_driver = RedisQueueDriver::connect(
        &redis_url(),
        &stream,
        group,
        consumer,
        Duration::from_secs(60),
    )
    .await
    .unwrap();
    let replacement_reservation = replacement_driver
        .pop(Duration::from_secs(5))
        .await
        .unwrap()
        .unwrap();

    let stale = old_driver
        .settle(&old_reservation.token, &[env("forbidden-after-clear")])
        .await
        .unwrap();
    assert_eq!(stale, Settled::Stale);
    assert_eq!(replacement_driver.reserved_size().await.unwrap(), 1);
    assert_eq!(replacement_driver.delayed_size().await.unwrap(), 0);
    replacement_driver
        .ack(&replacement_reservation.token)
        .await
        .unwrap();
    replacement_driver.clear().await.unwrap();
}

#[ignore = "requires a real Redis"]
#[tokio::test]
async fn redis_driver_nack_with_delay_redelivers_with_bumped_attempts() {
    let stream = format!("test-{}", uuid::Uuid::new_v4());
    let d = RedisQueueDriver::connect(&redis_url(), &stream, "g2", "c2", Duration::from_secs(60))
        .await
        .unwrap();

    d.push(env("R")).await.unwrap();

    let r1 = d.pop(Duration::from_secs(60)).await.unwrap().unwrap();
    assert_eq!(r1.envelope.attempts, 0);

    d.nack(&r1.token, Duration::from_millis(0)).await.unwrap();

    let r2 = d.pop(Duration::from_secs(60)).await.unwrap().unwrap();
    assert_eq!(
        r2.envelope.attempts, 1,
        "nack must bump attempts per trait contract"
    );
}

#[ignore = "requires a real Redis"]
#[tokio::test]
async fn redis_driver_release_redelivers_without_bumping_attempts() {
    let stream = format!("test-{}", Uuid::new_v4());
    let d = RedisQueueDriver::connect(
        &redis_url(),
        &stream,
        "g-release",
        "c-release",
        Duration::from_secs(60),
    )
    .await
    .unwrap();
    let mut original = env("release");
    original.attempts = 2;
    let original_id = original.id;
    d.push(original).await.unwrap();

    let first = d.pop(Duration::from_secs(5)).await.unwrap().unwrap();
    d.release(&first.token, &first.envelope, Duration::ZERO)
        .await
        .unwrap();
    let redelivered = d.pop(Duration::from_secs(5)).await.unwrap().unwrap();

    assert_eq!(redelivered.envelope.id, original_id);
    assert_eq!(redelivered.envelope.attempts, 2);
    assert_ne!(redelivered.token, first.token);
    d.ack(&first.token).await.unwrap();
    assert_eq!(d.reserved_size().await.unwrap(), 1);
    d.ack(&redelivered.token).await.unwrap();
    assert_eq!(d.reserved_size().await.unwrap(), 0);
    d.clear().await.unwrap();
}

/// `Queue::later` / `push` with a future `available_at` MUST not be visible
/// to pop until the delay elapses. Without the ZSET fix, the envelope went
/// straight onto the stream and was popped immediately.
#[ignore = "requires a real Redis"]
#[tokio::test]
async fn redis_driver_push_with_future_available_at_defers_until_due() {
    let stream = format!("test-{}", uuid::Uuid::new_v4());
    let d = RedisQueueDriver::connect(&redis_url(), &stream, "g3", "c3", Duration::from_secs(60))
        .await
        .unwrap();

    // Start near a whole-second boundary so a sub-second deadline remains in
    // the same unix second. Flooring its ZSET score would make it immediately
    // eligible; the driver must round a genuinely future deadline up instead.
    let now_ms = Utc::now().timestamp_millis();
    let until_next_second_ms = 1_000 - now_ms.rem_euclid(1_000);
    tokio::time::sleep(Duration::from_millis(until_next_second_ms as u64 + 20)).await;
    let mut e = env("delayed");
    e.available_at = Utc::now() + chrono::Duration::milliseconds(700);
    d.push(e).await.unwrap();

    // Immediate pop must NOT see the envelope.
    let now_view = d.pop(Duration::from_millis(150)).await.unwrap();
    assert!(
        now_view.is_none(),
        "delayed envelope leaked into the stream before its available_at"
    );

    // Wait past the deadline; pop must promote and deliver.
    tokio::time::sleep(Duration::from_millis(1_200)).await;
    let later_view = d.pop(Duration::from_secs(5)).await.unwrap();
    let r = later_view.expect("delayed envelope must be visible after the deadline");
    assert_eq!(r.envelope.job_name, "delayed");
    d.ack(&r.token).await.unwrap();
}

#[ignore = "requires a real Redis"]
#[tokio::test]
async fn redis_driver_promotes_legacy_unprefixed_delayed_members() {
    let stream = format!("test-{}", uuid::Uuid::new_v4());
    let d = RedisQueueDriver::connect(
        &redis_url(),
        &stream,
        "g-legacy-delayed",
        "c-legacy-delayed",
        Duration::from_secs(60),
    )
    .await
    .unwrap();
    let delayed_key = format!("{stream}:delayed");
    let legacy = env("legacy-delayed");
    let legacy_json = legacy.to_json().unwrap();
    let mut conn = redis_connection().await;
    let _: i64 = redis::cmd("ZADD")
        .arg(&delayed_key)
        .arg(Utc::now().timestamp())
        .arg(legacy_json)
        .query_async(&mut conn)
        .await
        .unwrap();

    let reservation = d
        .pop(Duration::from_secs(5))
        .await
        .unwrap()
        .expect("legacy delayed member should be promoted");
    assert_eq!(reservation.envelope.id, legacy.id);
    d.ack(&reservation.token).await.unwrap();
    d.clear().await.unwrap();
}

/// `nack` with a non-zero `requeue_delay` MUST also route via the ZSET; an
/// immediately-following pop must not see the redelivered envelope until the
/// delay elapses.
#[ignore = "requires a real Redis"]
#[tokio::test]
async fn redis_driver_nack_with_delay_defers_redelivery() {
    let stream = format!("test-{}", uuid::Uuid::new_v4());
    let d = RedisQueueDriver::connect(&redis_url(), &stream, "g4", "c4", Duration::from_secs(60))
        .await
        .unwrap();

    d.push(env("retry")).await.unwrap();
    let r1 = d.pop(Duration::from_secs(60)).await.unwrap().unwrap();

    d.nack(&r1.token, Duration::from_millis(1_500))
        .await
        .unwrap();

    // Immediate pop sees nothing (envelope is parked in the ZSET).
    let now_view = d.pop(Duration::from_millis(150)).await.unwrap();
    assert!(
        now_view.is_none(),
        "nack(delay=1.5s) re-delivered immediately"
    );

    tokio::time::sleep(Duration::from_millis(2_000)).await;
    let r2 = d
        .pop(Duration::from_secs(5))
        .await
        .unwrap()
        .expect("retry must surface after its delay");
    assert_eq!(r2.envelope.job_name, "retry");
    assert_eq!(r2.envelope.attempts, 1);
}

/// Lights up the M40 fix. With no overrides, the trait defaults for
/// `size`/`pending_size`/`reserved_size`/`delayed_size`/`clear` returned
/// `Err("does not implement")` - admin dashboards inspecting a Redis
/// queue got no number back. The overrides round-trip:
///   - push 2 immediate + 1 delayed → size = 3, delayed = 1
///   - pop one → reserved = 1, pending shrinks accordingly
///   - clear → everything = 0
#[ignore = "requires a real Redis"]
#[tokio::test]
async fn redis_driver_size_introspection_round_trip() {
    let stream = format!("test-{}", uuid::Uuid::new_v4());
    let d = RedisQueueDriver::connect(
        &redis_url(),
        &stream,
        "g-size",
        "c-size",
        Duration::from_secs(60),
    )
    .await
    .unwrap();

    // Pre-pop the empty stream so the consumer group exists for XPENDING.
    let _ = d.pop(Duration::from_millis(50)).await.unwrap();

    // Empty state.
    assert_eq!(d.size().await.unwrap(), 0);
    assert_eq!(d.pending_size().await.unwrap(), 0);
    assert_eq!(d.delayed_size().await.unwrap(), 0);
    assert_eq!(d.reserved_size().await.unwrap(), 0);

    // Push 2 immediate + 1 delayed.
    d.push(env("s1")).await.unwrap();
    d.push(env("s2")).await.unwrap();
    let mut delayed = env("s3-late");
    delayed.available_at = Utc::now() + chrono::Duration::milliseconds(3_000);
    d.push(delayed).await.unwrap();

    assert_eq!(
        d.size().await.unwrap(),
        3,
        "size = XLEN(stream) + ZCARD(delayed) = 2 + 1"
    );
    assert_eq!(
        d.delayed_size().await.unwrap(),
        1,
        "one envelope parked on the delayed ZSET"
    );
    assert_eq!(
        d.reserved_size().await.unwrap(),
        0,
        "direct new-delivery reads must not reserve work before pop"
    );
    let r1 = d.pop(Duration::from_secs(5)).await.unwrap().unwrap();
    assert_eq!(
        d.reserved_size().await.unwrap(),
        1,
        "COUNT 1 must reserve exactly the returned delivery"
    );

    d.ack(&r1.token).await.unwrap();
    assert_eq!(
        d.reserved_size().await.unwrap(),
        0,
        "ack must remove the only PEL entry"
    );

    // clear() returns an approximate count and drains everything.
    let cleared = d.clear().await.unwrap();
    assert!(cleared >= 1, "clear must report dropped envelopes");
    assert_eq!(
        d.delayed_size().await.unwrap(),
        0,
        "delayed key must be empty after clear"
    );
}

/// Queue inspection (#60966): `pending_jobs` / `delayed_jobs` /
/// `reserved_jobs` listings on the live Redis driver.
///
/// New deliveries use direct `XREADGROUP COUNT 1 ... >` calls, so there is no
/// background read-ahead task: a freshly pushed entry remains visible in
/// `pending_jobs` until a consumer explicitly pops it.
///
/// The test also guards the older inspection regression: `ack` only `XACK`s
/// an entry (this driver never `XDEL`/`XTRIM`s
/// the stream), so the old whole-stream-scan implementation - which
/// excluded only this process's in-memory `pending` map - reported every
/// acked job as pending forever. The cursor-based scan must not, because
/// an acked entry's id sits at or below the group's `last-delivered-id`
/// regardless of ack state.
#[ignore = "requires a real Redis"]
#[tokio::test]
async fn redis_driver_inspection_listings_round_trip() {
    let stream = format!("test-{}", uuid::Uuid::new_v4());
    let d = RedisQueueDriver::connect(
        &redis_url(),
        &stream,
        "g-inspect",
        "c-inspect",
        Duration::from_secs(60),
    )
    .await
    .unwrap();

    // Pre-pop the empty stream so the consumer group exists.
    let _ = d.pop(Duration::from_millis(50)).await.unwrap();

    d.push(env("pending-one")).await.unwrap();
    let mut delayed = env("delayed-one");
    delayed.available_at = Utc::now() + chrono::Duration::milliseconds(3_000);
    d.push(delayed).await.unwrap();

    let delayed_list = d.delayed_jobs(None).await.unwrap();
    assert_eq!(delayed_list.len(), 1, "one envelope parked on the ZSET");
    assert_eq!(delayed_list[0].name, "delayed-one");

    // The delayed envelope never reaches the stream until its deadline, so
    // it must not appear in the stream-backed pending listing.
    let pending_list = d.pending_jobs(None).await.unwrap();
    assert!(
        pending_list.iter().any(|j| j.name == "pending-one"),
        "a freshly pushed envelope must remain pending until pop"
    );
    assert!(
        !pending_list.iter().any(|j| j.name == "delayed-one"),
        "the delayed envelope must not show as pending"
    );

    // Reserve the pushed envelope.
    let r1 = d.pop(Duration::from_secs(5)).await.unwrap().unwrap();
    assert_eq!(r1.envelope.job_name, "pending-one");

    let reserved_list = d.reserved_jobs(None).await.unwrap();
    assert!(
        reserved_list.iter().any(|j| j.name == "pending-one"),
        "the popped-but-unacked envelope must appear in reserved_jobs"
    );
    let pending_after_pop = d.pending_jobs(None).await.unwrap();
    assert!(
        !pending_after_pop.iter().any(|j| j.name == "pending-one"),
        "a reserved envelope must not also show as pending"
    );

    d.ack(&r1.token).await.unwrap();

    let pending_after_ack = d.pending_jobs(None).await.unwrap();
    assert!(
        !pending_after_ack.iter().any(|j| j.name == "pending-one"),
        "an acked job must never reappear as pending - the regression this \
         task's cursor-based scan fixes"
    );
    let reserved_after_ack = d.reserved_jobs(None).await.unwrap();
    assert!(
        !reserved_after_ack.iter().any(|j| j.name == "pending-one"),
        "an acked job must no longer be reserved either"
    );
}
