//! PAR-031: named connections, opened on their first command, opened again
//! after they are lost, forgotten by `purge`.

use crate::support::{client_id, client_open, connection, database, kill_client, unique, url};
use serial_test::serial;
use std::process::{Command, Stdio};
use suprnova::{Redis, RedisValue};

const CHILD: &str = "SUPRNOVA_REDIS_CHILD";

fn is_child() -> bool {
    std::env::var_os(CHILD).is_some()
}

/// Run the test `name` of this binary alone in a child process with
/// `REDIS_URL` set to `redis_url`, or unset, and assert it passed.
fn run_child(name: &str, redis_url: Option<&str>, extra: &[(&str, &str)]) {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            name,
            "--nocapture",
            "--ignored",
            "--test-threads=1",
        ])
        .env(CHILD, "1")
        .env_remove("REDIS_URL")
        .env_remove("REDIS_COMMAND_RETRIES")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(redis_url) = redis_url {
        command.env("REDIS_URL", redis_url);
    }
    for (key, value) in extra {
        command.env(key, value);
    }
    let output = command.output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains("1 passed"),
        "the child failed: {stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The text of a `CLIENT INFO` reply.
async fn client_info(connection: &suprnova::RedisConnection) -> String {
    match connection.command("CLIENT", &["INFO"]).await.unwrap() {
        RedisValue::Bytes(bytes) => String::from_utf8(bytes).unwrap(),
        RedisValue::Status(text) => text,
        other => panic!("CLIENT INFO replied {other:?}"),
    }
}

/// The test URL with its database index replaced by `db`.
fn url_with_database(db: i64) -> String {
    let base = url();
    let host = base
        .trim_start_matches("redis://")
        .split('/')
        .next()
        .unwrap();
    format!("redis://{host}/{db}")
}

#[tokio::test]
#[ignore = "needs Redis: set REDIS_TEST_URL"]
#[serial]
async fn a_defined_connection_reaches_its_urls_database() {
    let redis = connection("defined");
    let key = unique("defined-key");
    redis.set(&key, "here").await.unwrap();

    let client = suprnova::redis::Client::open(url()).unwrap();
    let mut direct = client.get_multiplexed_async_connection().await.unwrap();
    let value: Option<String> = suprnova::redis::cmd("GET")
        .arg(&key)
        .query_async(&mut direct)
        .await
        .unwrap();
    assert_eq!(value.as_deref(), Some("here"));
    assert!(
        client_info(&redis)
            .await
            .contains(&format!(" db={} ", database()))
    );
    redis.del(&[key.as_str()]).await.unwrap();
}

#[tokio::test]
#[ignore = "needs Redis: set REDIS_TEST_URL"]
#[serial]
async fn define_replaces_a_connection_of_that_name() {
    let name = unique("replaced");
    let other = if database() == 15 { 14 } else { database() + 1 };
    Redis::define(&name, &url()).unwrap();
    let first = Redis::connection(&name).unwrap();
    assert!(
        client_info(&first)
            .await
            .contains(&format!(" db={} ", database()))
    );

    Redis::define(&name, &url_with_database(other)).unwrap();
    let second = Redis::connection(&name).unwrap();
    assert!(
        client_info(&second)
            .await
            .contains(&format!(" db={other} ")),
        "the second define replaces the first"
    );
}

#[tokio::test]
#[ignore = "needs Redis: set REDIS_TEST_URL"]
#[serial]
async fn the_default_connection_reaches_redis_url() {
    let key = unique("default-key");
    run_child(
        "connections::child_writes_through_the_default_connection",
        Some(&url()),
        &[("SUPRNOVA_REDIS_KEY", &key)],
    );
    let redis = connection("default-reader");
    assert_eq!(
        redis.get(&key).await.unwrap().as_deref(),
        Some("from the child")
    );
    redis.del(&[key.as_str()]).await.unwrap();
}

#[tokio::test]
#[ignore = "needs Redis: set REDIS_TEST_URL"]
#[serial]
async fn child_writes_through_the_default_connection() {
    if !is_child() {
        return;
    }
    let key = std::env::var("SUPRNOVA_REDIS_KEY").unwrap();
    let redis = Redis::connection("default").unwrap();
    redis.set(&key, "from the child").await.unwrap();
}

#[tokio::test]
#[ignore = "needs Redis: set REDIS_TEST_URL"]
#[serial]
async fn without_redis_url_the_default_connection_reaches_the_local_server() {
    run_child(
        "connections::child_reads_the_default_connections_address",
        None,
        &[],
    );
}

#[tokio::test]
#[ignore = "needs Redis: set REDIS_TEST_URL"]
#[serial]
async fn child_reads_the_default_connections_address() {
    if !is_child() {
        return;
    }
    let redis = Redis::connection("default").unwrap();
    let info = client_info(&redis).await;
    assert!(info.contains(" db=0 "), "{info}");
    assert!(info.contains("laddr=127.0.0.1:6379 "), "{info}");
}

#[tokio::test]
#[ignore = "needs Redis: set REDIS_TEST_URL"]
#[serial]
async fn a_redis_url_that_is_not_a_redis_url_is_an_error_naming_it() {
    run_child(
        "connections::child_resolves_a_bad_redis_url",
        Some("http://x"),
        &[],
    );
}

#[tokio::test]
#[ignore = "needs Redis: set REDIS_TEST_URL"]
#[serial]
async fn child_resolves_a_bad_redis_url() {
    if !is_child() {
        return;
    }
    let error = Redis::connection("default").expect_err("an http URL is no Redis URL");
    assert!(error.to_string().contains("REDIS_URL"), "{error}");
}

#[tokio::test]
#[ignore = "needs Redis: set REDIS_TEST_URL"]
#[serial]
async fn resolving_a_connection_sends_nothing_until_its_first_command() {
    let name = unique("nowhere");
    Redis::define(&name, "redis://127.0.0.1:1/0").unwrap();
    let redis = Redis::connection(&name).expect("resolving opens no connection");
    assert!(
        redis.get("anything").await.is_err(),
        "the first command opens the connection, and nothing listens"
    );
}

#[tokio::test]
#[ignore = "needs Redis: set REDIS_TEST_URL"]
#[serial]
async fn a_dropped_connection_opens_again() {
    let redis = connection("dropped");
    let key = unique("dropped-key");
    redis.set(&key, "kept").await.unwrap();
    let before = client_id(&redis).await;

    kill_client(before).await;

    assert_eq!(redis.get(&key).await.unwrap().as_deref(), Some("kept"));
    redis.set(&key, "written after").await.unwrap();
    assert_eq!(
        redis.get(&key).await.unwrap().as_deref(),
        Some("written after")
    );
    assert_ne!(
        client_id(&redis).await,
        before,
        "a new client replaced the dropped one"
    );
    redis.del(&[key.as_str()]).await.unwrap();
}

#[tokio::test]
#[ignore = "needs Redis: set REDIS_TEST_URL"]
#[serial]
async fn purge_forgets_a_connection_and_it_closes_when_no_handle_holds_it() {
    let name = unique("purged");
    Redis::define(&name, &url()).unwrap();
    let redis = Redis::connection(&name).unwrap();
    let id = client_id(&redis).await;
    assert!(Redis::connections().contains(&name));

    drop(redis);
    Redis::purge(&name);

    assert!(
        !Redis::connections().contains(&name),
        "a purged name is not listed"
    );
    assert!(!client_open(id).await, "the purged connection closed");
    let again = Redis::connection(&name).expect("the definition outlives the purge");
    assert_ne!(
        client_id(&again).await,
        id,
        "resolving again opens a new connection"
    );
}

#[tokio::test]
#[ignore = "needs Redis: set REDIS_TEST_URL"]
#[serial]
async fn an_unknown_name_is_an_error_naming_it() {
    let name = unique("never-defined");
    let error = Redis::connection(&name).expect_err("no connection has this name");
    assert!(error.to_string().contains(&name), "{error}");
}

#[tokio::test]
#[ignore = "needs Redis: set REDIS_TEST_URL"]
#[serial]
async fn a_url_that_is_not_a_redis_url_is_an_error_naming_the_connection() {
    let name = unique("http");
    let error = Redis::define(&name, "http://x").expect_err("http is no Redis scheme");
    assert!(error.to_string().contains(&name), "{error}");
}

#[test]
#[ignore = "needs Redis: set REDIS_TEST_URL"]
#[serial]
fn a_connection_opens_again_on_the_next_runtime() {
    let redis = connection("two-runtimes");
    let key = unique("two-runtimes-key");
    let runtime = || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    };

    runtime().block_on(async {
        redis.del(&[key.as_str()]).await.unwrap();
    });
    // The first runtime, and the connection's task on it, are gone; the
    // next command opens the connection on this one, even a write that is
    // never sent twice.
    runtime().block_on(async {
        assert_eq!(redis.incr(&key, 1).await.unwrap(), 1);
        redis.del(&[key.as_str()]).await.unwrap();
    });
}

#[tokio::test]
#[ignore = "needs Redis: set REDIS_TEST_URL"]
#[serial]
async fn define_client_gives_a_connection_a_client_built_elsewhere() {
    let name = unique("own-client");
    let client = suprnova::redis::Client::open(url()).unwrap();
    Redis::define_client(&name, client);
    let redis = Redis::connection(&name).unwrap();
    let key = unique("own-client-key");
    redis.set(&key, "through the given client").await.unwrap();
    assert_eq!(
        redis.get(&key).await.unwrap().as_deref(),
        Some("through the given client")
    );
    assert!(
        client_info(&redis)
            .await
            .contains(&format!(" db={} ", database()))
    );
    redis.del(&[key.as_str()]).await.unwrap();
}

#[tokio::test]
#[ignore = "needs Redis: set REDIS_TEST_URL"]
#[serial]
async fn a_resolved_name_stays_listed_when_it_is_defined_again() {
    let name = unique("redefined");
    Redis::define(&name, &url()).unwrap();
    let first = Redis::connection(&name).unwrap();
    first.command("PING", &[] as &[&str]).await.unwrap();

    Redis::define(&name, &url()).unwrap();
    assert!(
        Redis::connections().contains(&name),
        "the name was resolved and never purged"
    );
}
