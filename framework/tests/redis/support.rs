//! Helpers: the test server's URL, names no other test uses, and a proxy
//! that shows when a client waits for a reply.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;
use suprnova::{Redis, RedisConnection, RedisValue};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// The URL of the test database, from `REDIS_TEST_URL`.
pub fn url() -> String {
    std::env::var("REDIS_TEST_URL")
        .expect("REDIS_TEST_URL must name a Redis database the tests may write to")
}

/// The test database's index, from the URL's path; 0 when it has none.
pub fn database() -> i64 {
    url()
        .rsplit_once('/')
        .and_then(|(_, db)| db.parse().ok())
        .unwrap_or(0)
}

/// A name no other test in this run uses, for keys and connections.
pub fn unique(label: &str) -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!(
        "suprnova-redis-test:{}:{}:{label}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

/// A connection to the test database under a name of its own.
pub fn connection(label: &str) -> RedisConnection {
    let name = unique(label);
    Redis::define(&name, &url()).expect("the test URL is a Redis URL");
    Redis::connection(&name).expect("a defined connection resolves")
}

/// The server's id for the connection's current client.
pub async fn client_id(connection: &RedisConnection) -> i64 {
    match connection.command("CLIENT", &["ID"]).await.unwrap() {
        RedisValue::Int(id) => id,
        other => panic!("CLIENT ID replied {other:?}"),
    }
}

/// Close the client with `id` from the server's side.
pub async fn kill_client(id: i64) {
    let killer = connection("killer");
    killer
        .command("CLIENT", &["KILL", "ID", &id.to_string()])
        .await
        .expect("CLIENT KILL");
}

/// Whether the server still has a client with `id`, waiting up to two
/// seconds for it to go.
pub async fn client_open(id: i64) -> bool {
    let watcher = connection("watcher");
    for _ in 0..40 {
        let listed = watcher
            .command("CLIENT", &["LIST", "ID", &id.to_string()])
            .await
            .unwrap();
        let text = match listed {
            RedisValue::Bytes(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
            RedisValue::Status(text) => text,
            other => panic!("CLIENT LIST replied {other:?}"),
        };
        if text.trim().is_empty() {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    true
}

/// A TCP proxy in front of the test server. Once the client has sent the
/// first marker, the proxy holds the server's replies back until every
/// marker has arrived, or for one second. `held_until_all_sent` then says
/// whether the client sent every marker without waiting for a reply.
pub struct Proxy {
    pub url: String,
    held_until_all_sent: Arc<AtomicBool>,
}

impl Proxy {
    pub fn held_until_all_sent(&self) -> bool {
        self.held_until_all_sent.load(Ordering::SeqCst)
    }
}

pub async fn proxy(markers: Vec<String>) -> Proxy {
    let target = url()
        .trim_start_matches("redis://")
        .split('/')
        .next()
        .unwrap()
        .to_owned();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let held = Arc::new(AtomicBool::new(false));
    let flag = held.clone();
    tokio::spawn(async move {
        loop {
            let Ok((client, _)) = listener.accept().await else {
                return;
            };
            let server = TcpStream::connect(&target).await.unwrap();
            tokio::spawn(relay(client, server, markers.clone(), flag.clone()));
        }
    });
    Proxy {
        url: format!("redis://{address}/{}", database()),
        held_until_all_sent: held,
    }
}

async fn relay(client: TcpStream, server: TcpStream, markers: Vec<String>, held: Arc<AtomicBool>) {
    let (mut client_read, mut client_write) = client.into_split();
    let (mut server_read, mut server_write) = server.into_split();
    let sent = Arc::new(tokio::sync::Mutex::new(Vec::<u8>::new()));
    let seen = sent.clone();
    let upstream = tokio::spawn(async move {
        let mut buffer = [0u8; 8192];
        loop {
            let Ok(n) = client_read.read(&mut buffer).await else {
                return;
            };
            if n == 0 {
                return;
            }
            seen.lock().await.extend_from_slice(&buffer[..n]);
            if server_write.write_all(&buffer[..n]).await.is_err() {
                return;
            }
        }
    });
    let contains = |haystack: &[u8], needle: &str| {
        haystack
            .windows(needle.len())
            .any(|window| window == needle.as_bytes())
    };
    // The first reply after the first marker decides: the client either
    // sent every marker before it, or waited for it.
    let mut decided = false;
    let mut buffer = [0u8; 8192];
    loop {
        let Ok(n) = server_read.read(&mut buffer).await else {
            break;
        };
        if n == 0 {
            break;
        }
        if !decided && contains(&sent.lock().await, &markers[0]) {
            decided = true;
            let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
            loop {
                let all = {
                    let bytes = sent.lock().await;
                    markers.iter().all(|marker| contains(&bytes, marker))
                };
                if all {
                    held.store(true, Ordering::SeqCst);
                    break;
                }
                if tokio::time::Instant::now() > deadline {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
        if client_write.write_all(&buffer[..n]).await.is_err() {
            break;
        }
    }
    upstream.abort();
}
