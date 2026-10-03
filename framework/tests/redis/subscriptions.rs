//! PAR-034: subscriptions and blocking commands on connections of their
//! own.

use crate::support::{connection, unique};
use serial_test::serial;
use std::time::{Duration, Instant};
use suprnova::{RedisSide, RedisValue};

const WAIT: Duration = Duration::from_secs(3);

#[tokio::test]
#[ignore = "needs Redis: set REDIS_TEST_URL"]
#[serial]
async fn subscribe_yields_each_messages_channel_and_payload() {
    let redis = connection("subscribe");
    let channel = unique("channel");
    let mut subscription = redis.subscribe(&[channel.as_str()]).await.unwrap();

    assert_eq!(redis.publish(&channel, "hello").await.unwrap(), 1);
    assert_eq!(redis.publish(&channel, "again").await.unwrap(), 1);

    let message = tokio::time::timeout(WAIT, subscription.next()).await.unwrap().unwrap();
    assert_eq!(message.channel, channel);
    assert_eq!(message.pattern, None);
    assert_eq!(message.payload, b"hello");
    assert_eq!(message.payload_str(), Some("hello"));
    let message = tokio::time::timeout(WAIT, subscription.next()).await.unwrap().unwrap();
    assert_eq!(message.payload, b"again");
}

#[tokio::test]
#[ignore = "needs Redis: set REDIS_TEST_URL"]
#[serial]
async fn psubscribe_yields_the_pattern_a_message_matched() {
    let redis = connection("psubscribe");
    let prefix = unique("events");
    let pattern = format!("{prefix}:*");
    let mut subscription = redis.psubscribe(&[pattern.as_str()]).await.unwrap();

    let channel = format!("{prefix}:created");
    assert_eq!(redis.publish(&channel, "payload").await.unwrap(), 1);

    let message = tokio::time::timeout(WAIT, subscription.next()).await.unwrap().unwrap();
    assert_eq!(message.channel, channel);
    assert_eq!(message.pattern.as_deref(), Some(pattern.as_str()));
    assert_eq!(message.payload, b"payload");
}

#[tokio::test]
#[ignore = "needs Redis: set REDIS_TEST_URL"]
#[serial]
async fn the_connection_runs_commands_while_a_subscription_is_open() {
    let redis = connection("subscribed");
    let channel = unique("channel");
    let key = unique("key");
    let _subscription = redis.subscribe(&[channel.as_str()]).await.unwrap();
    let ran = tokio::time::timeout(Duration::from_secs(1), async {
        redis.set(&key, "1").await.unwrap();
        redis.get(&key).await.unwrap()
    })
    .await
    .expect("commands do not wait behind the subscription");
    assert_eq!(ran.as_deref(), Some("1"));
    redis.del(&[key.as_str()]).await.unwrap();
}

#[tokio::test]
#[ignore = "needs Redis: set REDIS_TEST_URL"]
#[serial]
async fn a_dropped_subscription_unsubscribes() {
    let redis = connection("dropped-subscription");
    let channel = unique("channel");
    let subscription = redis.subscribe(&[channel.as_str()]).await.unwrap();
    assert_eq!(subscribers(&redis, &channel).await, 1);

    drop(subscription);

    let start = Instant::now();
    while subscribers(&redis, &channel).await != 0 {
        assert!(start.elapsed() < WAIT, "the subscription's connection never closed");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn subscribers(redis: &suprnova::RedisConnection, channel: &str) -> i64 {
    match redis.command("PUBSUB", &["NUMSUB", channel]).await.unwrap() {
        RedisValue::Array(reply) => match reply.as_slice() {
            [_, RedisValue::Int(count)] => *count,
            other => panic!("PUBSUB NUMSUB replied {other:?}"),
        },
        other => panic!("PUBSUB NUMSUB replied {other:?}"),
    }
}

#[tokio::test]
#[ignore = "needs Redis: set REDIS_TEST_URL"]
#[serial]
async fn a_blpop_that_waits_delays_no_other_command_and_waits_its_whole_timeout() {
    let redis = connection("blocking");
    let list = unique("empty-list");
    let key = unique("key");
    redis.set(&key, "answer").await.unwrap();

    let waiting = redis.clone();
    let waited_list = list.clone();
    let blpop = tokio::spawn(async move {
        let start = Instant::now();
        let popped = waiting.blpop(&[waited_list.as_str()], Duration::from_secs(2)).await;
        (popped, start.elapsed())
    });
    tokio::time::sleep(Duration::from_millis(200)).await;

    let start = Instant::now();
    assert_eq!(redis.get(&key).await.unwrap().as_deref(), Some("answer"));
    assert!(
        start.elapsed() < Duration::from_millis(500),
        "the GET waited {:?} behind the BLPOP",
        start.elapsed()
    );

    let (popped, waited) = blpop.await.unwrap();
    assert_eq!(popped.unwrap(), None, "nothing was pushed");
    assert!(
        waited >= Duration::from_millis(1900),
        "the BLPOP returned after {waited:?}, before its two-second timeout"
    );
    redis.del(&[key.as_str()]).await.unwrap();
}

#[tokio::test]
#[ignore = "needs Redis: set REDIS_TEST_URL"]
#[serial]
async fn a_blpop_returns_an_element_pushed_while_it_waits() {
    let redis = connection("blocking-push");
    let list = unique("list");
    let waiting = redis.clone();
    let waited_list = list.clone();
    let blpop = tokio::spawn(async move {
        waiting.blpop(&[waited_list.as_str()], Duration::from_secs(5)).await
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    redis.rpush(&list, &["pushed"]).await.unwrap();

    let popped = tokio::time::timeout(WAIT, blpop).await.unwrap().unwrap().unwrap();
    assert_eq!(popped, Some((list.clone(), "pushed".to_owned())));
}

#[tokio::test]
#[ignore = "needs Redis: set REDIS_TEST_URL"]
#[serial]
async fn blocking_commands_return_what_is_there() {
    let redis = connection("blocking-ready");
    let (list, other, zset) = (unique("list"), unique("other"), unique("zset"));
    let second = Duration::from_secs(1);
    redis.rpush(&list, &["a", "b", "c", "d"]).await.unwrap();

    assert_eq!(
        redis.brpop(&[list.as_str()], second).await.unwrap(),
        Some((list.clone(), "d".to_owned()))
    );
    assert_eq!(
        redis
            .blmove(&list, &other, RedisSide::Left, RedisSide::Right, second)
            .await
            .unwrap()
            .as_deref(),
        Some("a")
    );
    assert_eq!(
        redis.brpoplpush(&list, &other, second).await.unwrap().as_deref(),
        Some("c")
    );
    assert_eq!(redis.lrange(&other, 0, -1).await.unwrap(), vec!["c", "a"]);

    redis.zadd(&zset, "low", 1.0).await.unwrap();
    redis.zadd(&zset, "high", 9.0).await.unwrap();
    assert_eq!(
        redis.bzpopmin(&[zset.as_str()], second).await.unwrap(),
        Some((zset.clone(), "low".to_owned(), 1.0))
    );
    assert_eq!(
        redis.bzpopmax(&[zset.as_str()], second).await.unwrap(),
        Some((zset.clone(), "high".to_owned(), 9.0))
    );
    redis.del(&[list.as_str(), other.as_str()]).await.unwrap();
}
