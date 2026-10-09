//! PAR-033: pipelines send everything before reading; transactions are
//! all or none when Redis rejects a queued command.

use crate::support::{connection, proxy, unique};
use serial_test::serial;
use suprnova::{Redis, RedisValue};

#[tokio::test]
#[ignore = "needs Redis: set REDIS_TEST_URL"]
#[serial]
async fn a_pipeline_sends_every_command_before_it_reads_a_reply() {
    let (first, second, third) = (unique("first"), unique("second"), unique("third"));
    let proxy = proxy(vec![first.clone(), second.clone(), third.clone()]).await;
    let name = unique("through-proxy");
    Redis::define(&name, &proxy.url).unwrap();
    let redis = Redis::connection(&name).unwrap();
    redis.command("PING", &[] as &[&str]).await.unwrap();

    let replies = redis
        .pipeline(|pipe| {
            pipe.set(&first, "1");
            pipe.set(&second, "2");
            pipe.get(&third);
        })
        .await
        .unwrap();

    assert!(
        proxy.held_until_all_sent(),
        "the three commands were sent before the first reply was read"
    );
    assert_eq!(
        replies,
        vec![
            RedisValue::Status("OK".into()),
            RedisValue::Status("OK".into()),
            RedisValue::Nil,
        ]
    );
    redis.del(&[first.as_str(), second.as_str()]).await.unwrap();
}

#[tokio::test]
#[ignore = "needs Redis: set REDIS_TEST_URL"]
#[serial]
async fn a_pipeline_returns_its_replies_in_order() {
    let redis = connection("pipeline-order");
    let key = unique("counter");
    let replies = redis
        .pipeline(|pipe| {
            pipe.set(&key, "1");
            pipe.incr(&key, 1);
            pipe.command("INCRBY", &[key.as_str(), "10"]);
            pipe.get(&key);
        })
        .await
        .unwrap();
    assert_eq!(
        replies,
        vec![
            RedisValue::Status("OK".into()),
            RedisValue::Int(2),
            RedisValue::Int(12),
            RedisValue::Bytes(b"12".to_vec()),
        ]
    );
    redis.del(&[key.as_str()]).await.unwrap();
}

#[tokio::test]
#[ignore = "needs Redis: set REDIS_TEST_URL"]
#[serial]
async fn a_transaction_applies_none_when_redis_rejects_a_queued_command() {
    let redis = connection("transaction-abort");
    let (before, after) = (unique("before"), unique("after"));
    let result = redis
        .transaction(|pipe| {
            pipe.set(&before, "1");
            pipe.command("NOSUCHCOMMAND", &[] as &[&str]);
            pipe.set(&after, "2");
        })
        .await;
    assert!(result.is_err(), "EXEC is aborted: {result:?}");
    assert_eq!(redis.get(&before).await.unwrap(), None);
    assert_eq!(redis.get(&after).await.unwrap(), None);
}

#[tokio::test]
#[ignore = "needs Redis: set REDIS_TEST_URL"]
#[serial]
async fn a_transaction_returns_its_replies_in_order() {
    let redis = connection("transaction-order");
    let key = unique("counter");
    let replies = redis
        .transaction(|pipe| {
            pipe.set(&key, "5");
            pipe.incr(&key, 1);
            pipe.get(&key);
        })
        .await
        .unwrap();
    assert_eq!(
        replies,
        vec![
            RedisValue::Status("OK".into()),
            RedisValue::Int(6),
            RedisValue::Bytes(b"6".to_vec()),
        ]
    );
    redis.del(&[key.as_str()]).await.unwrap();
}
