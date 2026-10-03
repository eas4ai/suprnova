//! PAR-032: typed commands, any command, the client, retried reads, and
//! the command events.

use crate::support::{client_id, connection, kill_client, unique};
use serial_test::serial;
use std::collections::{BTreeMap, BTreeSet};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use suprnova::{Redis, RedisCommandExecuted, RedisCommandFailed, RedisValue};

#[tokio::test]
#[ignore = "needs Redis: set REDIS_TEST_URL"]
#[serial]
async fn typed_commands_return_the_servers_replies() {
    let redis = connection("typed");
    let k = |label: &str| unique(label);
    let (string, expiring, counter, hash, list, set, zset, missing) = (
        k("string"),
        k("expiring"),
        k("counter"),
        k("hash"),
        k("list"),
        k("set"),
        k("zset"),
        k("missing"),
    );

    redis.set(&string, "value").await.unwrap();
    assert_eq!(redis.get(&string).await.unwrap().as_deref(), Some("value"));
    assert_eq!(redis.get(&missing).await.unwrap(), None);
    redis.set_ex(&expiring, "soon", 100).await.unwrap();
    assert!((1..=100).contains(&redis.ttl(&expiring).await.unwrap()));
    assert!(redis.exists(&string).await.unwrap());
    assert!(!redis.exists(&missing).await.unwrap());
    assert_eq!(redis.ttl(&missing).await.unwrap(), -2);
    assert!(redis.expire(&string, 100).await.unwrap());
    assert!(!redis.expire(&missing, 100).await.unwrap());
    assert_eq!(
        redis
            .mget(&[string.as_str(), missing.as_str()])
            .await
            .unwrap(),
        vec![Some("value".to_owned()), None]
    );

    assert_eq!(redis.incr(&counter, 5).await.unwrap(), 5);
    assert_eq!(redis.decr(&counter, 2).await.unwrap(), 3);

    assert_eq!(redis.hset(&hash, "field", "1").await.unwrap(), 1);
    assert_eq!(
        redis.hget(&hash, "field").await.unwrap().as_deref(),
        Some("1")
    );
    assert_eq!(redis.hget(&hash, "other").await.unwrap(), None);
    assert_eq!(
        redis.hgetall(&hash).await.unwrap(),
        BTreeMap::from([("field".to_owned(), "1".to_owned())])
    );
    assert_eq!(redis.hdel(&hash, &["field"]).await.unwrap(), 1);

    assert_eq!(redis.lpush(&list, &["a", "b"]).await.unwrap(), 2);
    assert_eq!(redis.rpush(&list, &["c"]).await.unwrap(), 3);
    assert_eq!(
        redis.lrange(&list, 0, -1).await.unwrap(),
        vec!["b", "a", "c"]
    );
    assert_eq!(redis.lpop(&list).await.unwrap().as_deref(), Some("b"));
    assert_eq!(redis.rpop(&list).await.unwrap().as_deref(), Some("c"));
    assert_eq!(redis.rpop(&missing).await.unwrap(), None);

    assert_eq!(redis.sadd(&set, &["x", "y"]).await.unwrap(), 2);
    assert_eq!(redis.srem(&set, &["x"]).await.unwrap(), 1);
    assert_eq!(
        redis.smembers(&set).await.unwrap(),
        BTreeSet::from(["y".to_owned()])
    );

    assert_eq!(redis.zadd(&zset, "first", 1.0).await.unwrap(), 1);
    assert_eq!(redis.zadd(&zset, "second", 2.0).await.unwrap(), 1);
    assert_eq!(
        redis.zrange(&zset, 0, -1).await.unwrap(),
        vec!["first", "second"]
    );
    assert_eq!(
        redis
            .zrangebyscore(&zset, 1.5, f64::INFINITY)
            .await
            .unwrap(),
        vec!["second"]
    );

    assert_eq!(
        redis.publish(&unique("channel"), "nobody").await.unwrap(),
        0
    );
    assert_eq!(
        redis
            .eval(
                "return {KEYS[1], ARGV[1]}",
                &[string.as_str()],
                &["argument"]
            )
            .await
            .unwrap(),
        RedisValue::Array(vec![
            RedisValue::Bytes(string.clone().into_bytes()),
            RedisValue::Bytes(b"argument".to_vec()),
        ])
    );

    let prefix = unique("scan");
    for n in 0..25 {
        redis.set(&format!("{prefix}:{n}"), "x").await.unwrap();
    }
    let found: BTreeSet<String> = redis
        .scan(&format!("{prefix}:*"))
        .await
        .unwrap()
        .into_iter()
        .collect();
    assert_eq!(found.len(), 25, "scan walks every cursor");
    let scanned: Vec<&str> = found.iter().map(String::as_str).collect();
    assert_eq!(redis.del(&scanned).await.unwrap(), 25);

    assert_eq!(
        redis
            .del(&[
                string.as_str(),
                expiring.as_str(),
                counter.as_str(),
                list.as_str(),
                set.as_str(),
                zset.as_str()
            ])
            .await
            .unwrap(),
        6
    );
}

#[tokio::test]
#[ignore = "needs Redis: set REDIS_TEST_URL"]
#[serial]
async fn command_runs_any_command_and_returns_the_reply() {
    let redis = connection("command");
    let list = unique("list");
    redis.rpush(&list, &["one", "two"]).await.unwrap();
    assert_eq!(
        redis
            .command("LRANGE", &[list.as_str(), "0", "-1"])
            .await
            .unwrap(),
        RedisValue::Array(vec![
            RedisValue::Bytes(b"one".to_vec()),
            RedisValue::Bytes(b"two".to_vec()),
        ])
    );
    assert_eq!(
        redis.command("llen", &[list.as_str()]).await.unwrap(),
        RedisValue::Int(2),
        "a lower-case name works too"
    );
    assert_eq!(
        redis.command("SET", &[list.as_str(), "x"]).await.unwrap(),
        RedisValue::Status("OK".into()),
        "SET replaces a key of any type"
    );
    assert_eq!(
        redis.command("GET", &[list.as_str()]).await.unwrap(),
        RedisValue::Bytes(b"x".to_vec())
    );
    assert_eq!(
        redis.command("PING", &[] as &[&str]).await.unwrap(),
        RedisValue::Status("PONG".into())
    );
    redis.del(&[list.as_str()]).await.unwrap();
}

#[tokio::test]
#[ignore = "needs Redis: set REDIS_TEST_URL"]
#[serial]
async fn client_is_a_client_of_the_same_server_and_database() {
    let redis = connection("client");
    let key = unique("client-key");
    redis.set(&key, "shared").await.unwrap();
    let mut client = redis.client().unwrap();
    let value: Option<String> = suprnova::redis::cmd("GET")
        .arg(&key)
        .query_async(&mut client)
        .await
        .unwrap();
    assert_eq!(value.as_deref(), Some("shared"));
    redis.del(&[key.as_str()]).await.unwrap();
}

#[tokio::test]
#[ignore = "needs Redis: set REDIS_TEST_URL"]
#[serial]
async fn a_read_is_sent_again_after_a_lost_connection() {
    let redis = connection("retried");
    let key = unique("retried-key");
    redis.set(&key, "still here").await.unwrap();

    kill_client(client_id(&redis).await).await;
    assert_eq!(
        redis.get(&key).await.unwrap().as_deref(),
        Some("still here")
    );

    kill_client(client_id(&redis).await).await;
    assert_eq!(
        redis.command("GET", &[key.as_str()]).await.unwrap(),
        RedisValue::Bytes(b"still here".to_vec()),
        "GET given to command is one of the retryable commands"
    );

    kill_client(client_id(&redis).await).await;
    assert_eq!(
        redis.lrange(&unique("empty"), 0, -1).await.unwrap(),
        Vec::<String>::new()
    );
    redis.del(&[key.as_str()]).await.unwrap();
}

#[tokio::test]
#[ignore = "needs Redis: set REDIS_TEST_URL"]
#[serial]
async fn a_write_is_not_applied_twice_after_a_lost_connection() {
    let redis = connection("write");
    let counter = unique("counter");
    kill_client(client_id(&redis).await).await;
    let _ = redis.incr(&counter, 1).await;
    let value = redis.get(&counter).await.unwrap();
    assert!(
        value.is_none() || value.as_deref() == Some("1"),
        "the INCR was applied at most once, got {value:?}"
    );
    redis.del(&[counter.as_str()]).await.unwrap();
}

type Seen<T> = Arc<Mutex<Vec<T>>>;

fn record_executed() -> Seen<RedisCommandExecuted> {
    let seen: Seen<RedisCommandExecuted> = Arc::default();
    let sink = seen.clone();
    Redis::listen(move |event: &RedisCommandExecuted| sink.lock().unwrap().push(event.clone()));
    seen
}

fn record_failed() -> Seen<RedisCommandFailed> {
    let seen: Seen<RedisCommandFailed> = Arc::default();
    let sink = seen.clone();
    Redis::listen_for_failures(move |event: &RedisCommandFailed| {
        sink.lock().unwrap().push(event.clone())
    });
    seen
}

#[tokio::test]
#[ignore = "needs Redis: set REDIS_TEST_URL"]
#[serial]
async fn while_events_are_enabled_each_command_is_reported() {
    let redis = connection("events");
    let key = unique("event-key");
    let executed = record_executed();
    Redis::enable_events();

    redis.set(&key, "value").await.unwrap();
    redis.command("STRLEN", &[key.as_str()]).await.unwrap();

    Redis::disable_events();
    redis.get(&key).await.unwrap();

    let mine: Vec<RedisCommandExecuted> = executed
        .lock()
        .unwrap()
        .iter()
        .filter(|event| event.connection == redis.name())
        .cloned()
        .collect();
    assert_eq!(
        mine.len(),
        2,
        "SET and STRLEN, and not the GET after disable: {mine:?}"
    );
    assert_eq!(mine[0].command, "SET");
    assert_eq!(mine[0].arguments, vec![key.clone(), "value".to_owned()]);
    assert_eq!(mine[1].command, "STRLEN");
    assert_eq!(mine[1].arguments, vec![key.clone()]);
    redis.del(&[key.as_str()]).await.unwrap();
}

#[tokio::test]
#[ignore = "needs Redis: set REDIS_TEST_URL"]
#[serial]
async fn a_failed_command_is_reported_to_the_failure_listeners() {
    let redis = connection("failures");
    let key = unique("not-a-number");
    redis.set(&key, "abc").await.unwrap();
    let failed = record_failed();
    Redis::enable_events();

    let error = redis.incr(&key, 1).await.unwrap_err();

    Redis::disable_events();
    let mine: Vec<RedisCommandFailed> = failed
        .lock()
        .unwrap()
        .iter()
        .filter(|event| event.connection == redis.name())
        .cloned()
        .collect();
    assert_eq!(mine.len(), 1, "{mine:?}");
    assert_eq!(mine[0].command, "INCRBY");
    assert_eq!(mine[0].arguments, vec![key.clone(), "1".to_owned()]);
    assert!(
        mine[0].error.contains("not an integer"),
        "{}",
        mine[0].error
    );
    assert!(error.to_string().contains("not an integer"), "{error}");
    redis.del(&[key.as_str()]).await.unwrap();
}

#[tokio::test]
#[ignore = "needs Redis: set REDIS_TEST_URL"]
#[serial]
async fn events_are_off_until_enabled() {
    let mut command = Command::new(std::env::current_exe().unwrap());
    let output = command
        .args([
            "--exact",
            "commands::child_runs_a_command_with_events_never_enabled",
            "--nocapture",
            "--ignored",
            "--test-threads=1",
        ])
        .env("SUPRNOVA_REDIS_CHILD", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains("1 passed"),
        "the child failed: {stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
#[ignore = "needs Redis: set REDIS_TEST_URL"]
#[serial]
async fn child_runs_a_command_with_events_never_enabled() {
    if std::env::var_os("SUPRNOVA_REDIS_CHILD").is_none() {
        return;
    }
    let executed = record_executed();
    let redis = connection("quiet");
    redis.command("PING", &[] as &[&str]).await.unwrap();
    assert!(
        executed.lock().unwrap().is_empty(),
        "no event before enable_events"
    );
}
