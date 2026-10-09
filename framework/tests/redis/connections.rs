//! PAR-031: named connections, opened on their first command, opened again
//! after they are lost, forgotten by `purge`.

use crate::support::{
    client_id, client_open, connection, database, kill_client, unique, url, wire_key,
};
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
        .env_remove("REDIS_HOST")
        .env_remove("REDIS_PORT")
        .env_remove("REDIS_PASSWORD")
        .env_remove("REDIS_USERNAME")
        .env_remove("REDIS_DB")
        .env_remove("REDIS_CACHE_DB")
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
        .arg(wire_key(&key))
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
    run_child(
        "connections::child_reads_the_default_connections_address",
        None,
        &[("REDIS_PASSWORD", ""), ("REDIS_USERNAME", "")],
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

/// A standalone server proves host, port and authentication without inherited settings.
struct ConfigServer {
    port: u16,
    child: std::process::Child,
    _dir: tempfile::TempDir,
}

impl Drop for ConfigServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn config_server() -> ConfigServer {
    let dir = tempfile::tempdir().unwrap();
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let child = Command::new("redis-server")
        .args([
            "--bind",
            "127.0.0.1",
            "--port",
            &port.to_string(),
            "--requirepass",
            "test:@/#password",
            "--save",
            "",
            "--appendonly",
            "no",
        ])
        .current_dir(dir.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let server = ConfigServer {
        port,
        child,
        _dir: dir,
    };
    let start = std::time::Instant::now();
    while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    server
}

#[test]
#[ignore = "needs Redis: runs redis-server"]
#[serial]
fn environment_tuple_and_cache_database_reach_the_configured_server() {
    let server = config_server();
    run_child(
        "connections::child_checks_environment_connections",
        None,
        &[
            ("REDIS_HOST", "localhost"),
            ("REDIS_PORT", &server.port.to_string()),
            ("REDIS_PASSWORD", "test:@/#password"),
            ("REDIS_DB", "6"),
            ("REDIS_CACHE_DB", "7"),
            ("SUPRNOVA_EXPECT_DEFAULT_DB", "6"),
            ("SUPRNOVA_EXPECT_CACHE_DB", "7"),
            ("SUPRNOVA_EXPECT_PORT", &server.port.to_string()),
        ],
    );
}

#[test]
#[ignore = "needs Redis: set REDIS_TEST_URL"]
#[serial]
fn redis_url_takes_precedence_and_cache_uses_its_own_database() {
    let parsed = url::Url::parse(&url()).unwrap();
    run_child(
        "connections::child_checks_environment_connections",
        Some(&url()),
        &[
            ("REDIS_HOST", "unreachable.invalid"),
            ("REDIS_PORT", "1"),
            ("REDIS_PASSWORD", "wrong"),
            ("REDIS_DB", "14"),
            ("REDIS_CACHE_DB", "7"),
            ("SUPRNOVA_EXPECT_DEFAULT_DB", &database().to_string()),
            ("SUPRNOVA_EXPECT_CACHE_DB", "7"),
            (
                "SUPRNOVA_EXPECT_PORT",
                &parsed.port().unwrap_or(6379).to_string(),
            ),
        ],
    );
    run_child(
        "connections::child_checks_environment_connections",
        Some(&url()),
        &[
            ("SUPRNOVA_EXPECT_DEFAULT_DB", &database().to_string()),
            ("SUPRNOVA_EXPECT_CACHE_DB", "1"),
            (
                "SUPRNOVA_EXPECT_PORT",
                &parsed.port().unwrap_or(6379).to_string(),
            ),
        ],
    );
}

#[tokio::test]
#[ignore = "child process for environment configuration"]
async fn child_checks_environment_connections() {
    if !is_child() {
        return;
    }
    let port = std::env::var("SUPRNOVA_EXPECT_PORT").unwrap();
    for (name, expected) in [
        ("default", "SUPRNOVA_EXPECT_DEFAULT_DB"),
        ("cache", "SUPRNOVA_EXPECT_CACHE_DB"),
    ] {
        let connection = Redis::connection(name).unwrap();
        let info = client_info(&connection).await;
        let db = std::env::var(expected).unwrap();
        assert!(info.contains(&format!(" db={db} ")), "{info}");
        assert!(info.contains(&format!(":{port} ")), "{info}");
        let key = unique("env-key");
        connection.set(&key, "configured").await.unwrap();
        assert_eq!(
            connection.get(&key).await.unwrap().as_deref(),
            Some("configured")
        );
        connection.del(&[key.as_str()]).await.unwrap();
    }
}

#[test]
#[ignore = "needs Redis: set REDIS_TEST_URL"]
#[serial]
fn explicit_empty_and_app_name_prefixes_are_used_on_the_wire() {
    for prefix in ["shop-", "shop[*?]\\-", ""] {
        run_child(
            "connections::child_checks_key_prefix",
            Some(&url()),
            &[("REDIS_PREFIX", prefix), ("APP_NAME", "My Shop")],
        );
    }
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "connections::child_checks_key_prefix",
            "--ignored",
            "--test-threads=1",
        ])
        .env(CHILD, "1")
        .env("REDIS_URL", url())
        .env("APP_NAME", "My Shop")
        .env_remove("REDIS_PREFIX")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[tokio::test]
#[ignore = "child process for key prefixes"]
async fn child_checks_key_prefix() {
    if !is_child() {
        return;
    }
    let redis = Redis::connection("default").unwrap();
    let key = unique("prefix-key");
    let prefix = std::env::var("REDIS_PREFIX").unwrap_or_else(|_| "my-shop-database-".to_owned());
    let wire = format!("{prefix}{key}");
    redis.set(&key, "typed").await.unwrap();
    assert_eq!(
        redis.command("GET", &[wire.as_str()]).await.unwrap(),
        RedisValue::Bytes(b"typed".to_vec())
    );
    if !prefix.is_empty() {
        assert_eq!(
            redis.command("GET", &[key.as_str()]).await.unwrap(),
            RedisValue::Nil
        );
        redis.command("SET", &[key.as_str(), "raw"]).await.unwrap();
        assert_eq!(
            redis.execute_raw(&["GET", key.as_str()]).await.unwrap(),
            RedisValue::Bytes(b"raw".to_vec())
        );
        assert_eq!(redis.get(&key).await.unwrap().as_deref(), Some("typed"));
        redis.command("DEL", &[key.as_str()]).await.unwrap();
    }
    assert_eq!(redis.scan(&key).await.unwrap(), vec![wire.clone()]);
    redis
        .eval(
            "return redis.call('set', KEYS[1], ARGV[1])",
            &[key.as_str()],
            &["script"],
        )
        .await
        .unwrap();
    assert_eq!(
        redis.command("GET", &[wire.as_str()]).await.unwrap(),
        RedisValue::Bytes(b"script".to_vec())
    );
    redis
        .pipeline(|pipe| {
            pipe.set(&key, "pipeline");
        })
        .await
        .unwrap();
    assert_eq!(
        redis.command("GET", &[wire.as_str()]).await.unwrap(),
        RedisValue::Bytes(b"pipeline".to_vec())
    );
    redis.del(&[key.as_str()]).await.unwrap();
}
