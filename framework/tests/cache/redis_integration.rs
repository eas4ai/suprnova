//! Live-Redis integration tests for the cache module.
//!
//! These tests connect to a running Redis at `CACHE_REDIS_TEST_URL`
//! (default `redis://127.0.0.1:6379`). They are `#[ignore]`d so the
//! default `cargo test` run does not require a Redis. Run them with:
//!
//! ```sh
//! cargo test -p suprnova --test cache_redis_integration -- --ignored
//! ```
//!
//! Each test scopes itself to a unique prefix so concurrent runs and
//! prior failed runs do not see each other's keys.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use suprnova::cache::store::CacheStore;
use suprnova::cache::{CacheConfig, RedisCache};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

fn redis_url() -> String {
    std::env::var("CACHE_REDIS_TEST_URL")
        .or_else(|_| std::env::var("REDIS_URL"))
        .unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string())
}

async fn fresh_store(prefix: &str) -> Arc<dyn CacheStore> {
    store_at(&redis_url(), format!("{}{}:", prefix, uuid::Uuid::new_v4())).await
}

async fn store_at(url: &str, prefix: String) -> Arc<dyn CacheStore> {
    let cfg = CacheConfig {
        driver: suprnova::cache::CacheDriver::Redis,
        url: url.to_string(),
        prefix,
        default_ttl: 0,
    };
    let cache = RedisCache::connect(&cfg)
        .await
        .expect("connect to test Redis (set CACHE_REDIS_TEST_URL if not on localhost)");
    Arc::new(cache)
}

#[derive(Debug, PartialEq, Eq)]
enum AddBoundary {
    SplitCleanupBlocked,
    AtomicScriptExecuted,
}

async fn read_resp_command<R>(reader: &mut R) -> std::io::Result<Option<(Vec<u8>, Vec<Vec<u8>>)>>
where
    R: tokio::io::AsyncBufRead + Unpin,
{
    let mut raw = Vec::new();
    let mut line = Vec::new();
    if reader.read_until(b'\n', &mut line).await? == 0 {
        return Ok(None);
    }
    raw.extend_from_slice(&line);
    let count = std::str::from_utf8(&line)
        .ok()
        .and_then(|value| value.strip_prefix('*'))
        .map(str::trim)
        .and_then(|value| value.parse::<usize>().ok())
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "RESP array"))?;

    let mut args = Vec::with_capacity(count);
    for _ in 0..count {
        line.clear();
        reader.read_until(b'\n', &mut line).await?;
        raw.extend_from_slice(&line);
        let len = std::str::from_utf8(&line)
            .ok()
            .and_then(|value| value.strip_prefix('$'))
            .map(str::trim)
            .and_then(|value| value.parse::<usize>().ok())
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "RESP bulk"))?;
        let mut data = vec![0; len + 2];
        reader.read_exact(&mut data).await?;
        raw.extend_from_slice(&data);
        args.push(data[..len].to_vec());
    }
    Ok(Some((raw, args)))
}

async fn start_add_race_proxy(
    upstream_url: &str,
    value_key: String,
    aux_key: String,
) -> (
    String,
    tokio::sync::mpsc::UnboundedReceiver<AddBoundary>,
    tokio::sync::oneshot::Sender<()>,
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
) {
    let mut proxy_url = url::Url::parse(upstream_url).expect("valid Redis test URL");
    assert_eq!(
        proxy_url.scheme(),
        "redis",
        "controlled cache race test requires a plaintext redis:// endpoint"
    );
    let upstream_host = proxy_url.host_str().expect("Redis host").to_string();
    let upstream_port = proxy_url.port().unwrap_or(6379);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind Redis race proxy");
    let proxy_port = listener.local_addr().expect("proxy address").port();
    proxy_url
        .set_host(Some("127.0.0.1"))
        .expect("set proxy host");
    proxy_url
        .set_port(Some(proxy_port))
        .expect("set proxy port");

    let (boundary_tx, boundary_rx) = tokio::sync::mpsc::unbounded_channel();
    let (release_cleanup_tx, mut release_cleanup_rx) = tokio::sync::oneshot::channel();
    let (release_script_tx, release_script_rx) = tokio::sync::oneshot::channel();
    let hold_script_response = Arc::new(AtomicBool::new(false));
    let task = tokio::spawn(async move {
        let (client, _) = listener.accept().await.expect("accept Redis client");
        let upstream = tokio::net::TcpStream::connect((upstream_host.as_str(), upstream_port))
            .await
            .expect("connect race proxy upstream");
        let (client_read, mut client_write) = client.into_split();
        let (upstream_read, mut upstream_write) = upstream.into_split();
        let response_boundary_tx = boundary_tx.clone();
        let response_hold = Arc::clone(&hold_script_response);
        let response_pump = tokio::spawn(async move {
            let mut upstream_read = BufReader::new(upstream_read);
            let mut release_script_rx = Some(release_script_rx);
            let mut buffer = [0_u8; 8 * 1024];
            loop {
                let read = upstream_read
                    .read(&mut buffer)
                    .await
                    .expect("read Redis response");
                if read == 0 {
                    break;
                }
                if response_hold.swap(false, Ordering::SeqCst) {
                    response_boundary_tx
                        .send(AddBoundary::AtomicScriptExecuted)
                        .expect("report executed atomic add script");
                    if let Some(release) = release_script_rx.take() {
                        let _ = release.await;
                    }
                }
                client_write
                    .write_all(&buffer[..read])
                    .await
                    .expect("forward Redis response");
            }
        });
        let mut reader = BufReader::new(client_read);
        let mut successful_add_seen = false;

        while let Some((raw, args)) = read_resp_command(&mut reader)
            .await
            .expect("read Redis command")
        {
            let command = args
                .first()
                .map(|arg| String::from_utf8_lossy(arg).to_ascii_uppercase())
                .unwrap_or_default();
            if command == "SET"
                && args.get(1).is_some_and(|arg| arg == value_key.as_bytes())
                && args.iter().any(|arg| arg.eq_ignore_ascii_case(b"NX"))
            {
                successful_add_seen = true;
            }
            if successful_add_seen
                && command == "DEL"
                && args.get(1).is_some_and(|arg| arg == aux_key.as_bytes())
            {
                boundary_tx
                    .send(AddBoundary::SplitCleanupBlocked)
                    .expect("report split cleanup");
                let _ = (&mut release_cleanup_rx).await;
            } else if command == "EVAL"
                && args.get(1).is_some_and(|script| {
                    String::from_utf8_lossy(script).contains("suprnova_cache_add_raw_v1")
                })
            {
                // Hold the response, not the request: observing the upstream
                // response proves Redis has executed the script while the
                // add_raw caller is still unable to issue follow-up cleanup.
                successful_add_seen = true;
                hold_script_response.store(true, Ordering::SeqCst);
            }
            upstream_write
                .write_all(&raw)
                .await
                .expect("forward Redis command");
        }
        response_pump.abort();
    });

    (
        proxy_url.to_string(),
        boundary_rx,
        release_cleanup_tx,
        release_script_tx,
        task,
    )
}

#[tokio::test]
#[ignore = "requires Redis at CACHE_REDIS_TEST_URL or default localhost"]
async fn redis_put_with_subsecond_ttl_expires_correctly() {
    let s = fresh_store("sub-ttl").await;
    s.put_raw("k", "{\"v\":1}", Some(Duration::from_millis(80)))
        .await
        .unwrap();
    assert!(
        s.has("k").await.unwrap(),
        "value present immediately after put"
    );
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(
        !s.has("k").await.unwrap(),
        "sub-second TTL must be honoured (PX, not EX rounded to 0)"
    );
}

#[tokio::test]
#[ignore = "requires Redis at CACHE_REDIS_TEST_URL or default localhost"]
async fn redis_lock_subsecond_ttl_expires_and_releases() {
    let s = fresh_store("sub-lock").await;
    let alice = s
        .acquire_lock("printer", Duration::from_millis(50))
        .await
        .unwrap();
    assert!(alice.is_some(), "first acquire wins");

    let bob = s
        .acquire_lock("printer", Duration::from_millis(50))
        .await
        .unwrap();
    assert!(bob.is_none(), "contention");

    tokio::time::sleep(Duration::from_millis(120)).await;
    let carol = s
        .acquire_lock("printer", Duration::from_secs(5))
        .await
        .unwrap();
    assert!(
        carol.is_some(),
        "sub-second lock TTL must expire - EX-as-secs would have errored or rounded to 0"
    );
}

#[tokio::test]
#[ignore = "requires Redis at CACHE_REDIS_TEST_URL or default localhost"]
async fn redis_lock_refresh_subsecond_ttl_extends() {
    let s = fresh_store("sub-refresh").await;
    let alice = s
        .acquire_lock("k", Duration::from_millis(200))
        .await
        .unwrap()
        .unwrap();

    tokio::time::sleep(Duration::from_millis(100)).await;

    let refreshed = s
        .refresh_lock("k", &alice, Duration::from_millis(300))
        .await
        .unwrap();
    assert!(refreshed, "refresh succeeds with valid token");

    tokio::time::sleep(Duration::from_millis(150)).await;

    let bob = s
        .acquire_lock("k", Duration::from_millis(50))
        .await
        .unwrap();
    assert!(
        bob.is_none(),
        "PEXPIRE extended the lock - EXPIRE with sub-second TTL would have deleted the key"
    );

    s.release_lock("k", &alice).await.unwrap();
}

#[tokio::test]
#[ignore = "requires Redis at CACHE_REDIS_TEST_URL or default localhost"]
async fn redis_touch_subsecond_ttl_extends() {
    let s = fresh_store("sub-touch").await;
    s.put_raw("k", "v", Some(Duration::from_millis(80)))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(40)).await;

    let touched = s.touch("k", Duration::from_millis(300)).await.unwrap();
    assert!(touched, "touch returns true on extant key");

    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        s.has("k").await.unwrap(),
        "PEXPIRE extended; EXPIRE 0 would have deleted the key"
    );
}

#[tokio::test]
#[ignore = "requires Redis at CACHE_REDIS_TEST_URL or default localhost"]
async fn redis_flush_uses_scan_and_clears_the_keyspace() {
    let s = fresh_store("scan-flush").await;
    for i in 0..50 {
        s.put_raw(&format!("k:{i}"), &format!("v{i}"), None)
            .await
            .unwrap();
    }
    for i in 0..50 {
        assert!(s.has(&format!("k:{i}")).await.unwrap());
    }
    s.flush().await.unwrap();
    for i in 0..50 {
        assert!(
            !s.has(&format!("k:{i}")).await.unwrap(),
            "flush via SCAN must remove every prefixed key"
        );
    }
}

#[tokio::test]
#[ignore = "requires Redis at CACHE_REDIS_TEST_URL or default localhost"]
async fn redis_tagged_writes_can_be_flushed_by_tag() {
    let s = fresh_store("redis-tags-1").await;
    s.tagged_put_raw(&["users"], "u:1", "{\"id\":1}", None)
        .await
        .unwrap();
    s.tagged_put_raw(&["users", "active"], "u:2", "{\"id\":2}", None)
        .await
        .unwrap();
    s.tagged_put_raw(&["posts"], "p:1", "{\"id\":1}", None)
        .await
        .unwrap();

    s.flush_tags(&["users"]).await.unwrap();

    assert!(!s.has("u:1").await.unwrap());
    assert!(!s.has("u:2").await.unwrap());
    assert!(s.has("p:1").await.unwrap(), "different tag untouched");
}

#[tokio::test]
#[ignore = "requires Redis at CACHE_REDIS_TEST_URL or default localhost"]
async fn redis_untagged_overwrite_after_tagged_survives_flush() {
    let s = fresh_store("redis-tags-2").await;
    s.tagged_put_raw(&["users"], "u:1", "v1", None)
        .await
        .unwrap();
    s.put_raw("u:1", "v2", None).await.unwrap();

    s.flush_tags(&["users"]).await.unwrap();

    assert!(
        s.has("u:1").await.unwrap(),
        "untagged overwrite cleared the tag aux set - flush_tags must skip it"
    );
    let got: Option<String> = s.get_raw("u:1").await.unwrap();
    assert_eq!(got.as_deref(), Some("v2"));
}

#[tokio::test]
#[ignore = "requires Redis at CACHE_REDIS_TEST_URL or default localhost"]
async fn redis_retagging_drops_old_membership() {
    let s = fresh_store("redis-tags-3").await;
    s.tagged_put_raw(&["a"], "k", "v1", None).await.unwrap();
    s.tagged_put_raw(&["b"], "k", "v2", None).await.unwrap();

    s.flush_tags(&["a"]).await.unwrap();
    assert!(
        s.has("k").await.unwrap(),
        "k re-tagged to b - flushing a must not delete it"
    );

    s.flush_tags(&["b"]).await.unwrap();
    assert!(!s.has("k").await.unwrap(), "flushing current tag deletes");
}

#[tokio::test]
#[ignore = "requires Redis at CACHE_REDIS_TEST_URL or default localhost"]
async fn redis_tagged_subsecond_ttl_expires() {
    let s = fresh_store("redis-tags-sub").await;
    s.tagged_put_raw(&["t"], "k", "v", Some(Duration::from_millis(80)))
        .await
        .unwrap();
    assert!(s.has("k").await.unwrap());
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(
        !s.has("k").await.unwrap(),
        "tagged_put_raw must use PX for sub-second TTL"
    );
}

#[tokio::test]
#[ignore = "requires Redis at CACHE_REDIS_TEST_URL or default localhost"]
async fn redis_add_with_subsecond_ttl_expires() {
    let s = fresh_store("redis-add-sub").await;
    let ok = s
        .add_raw("k", "v", Some(Duration::from_millis(80)))
        .await
        .unwrap();
    assert!(ok);

    // Contention with another add - must fail until the TTL expires.
    let busy = s
        .add_raw("k", "v2", Some(Duration::from_secs(5)))
        .await
        .unwrap();
    assert!(!busy, "contention while value is live");

    tokio::time::sleep(Duration::from_millis(150)).await;

    let free = s
        .add_raw("k", "v3", Some(Duration::from_secs(5)))
        .await
        .unwrap();
    assert!(free, "add succeeds after sub-second TTL expires");
    let v: Option<String> = s.get_raw("k").await.unwrap();
    assert_eq!(v.as_deref(), Some("v3"));
}

#[tokio::test]
#[ignore = "requires Redis at CACHE_REDIS_TEST_URL or default localhost"]
async fn redis_add_cleanup_cannot_detach_a_concurrent_tagged_overwrite() {
    let upstream_url = redis_url();
    let prefix = format!("redis-add-tag-race-{}:", uuid::Uuid::new_v4());
    let value_key = format!("{prefix}k");
    let aux_key = format!("{prefix}\0key_tags:{value_key}");
    let (proxy_url, mut boundary_rx, release_cleanup, release_script_response, proxy_task) =
        start_add_race_proxy(&upstream_url, value_key, aux_key).await;
    let add_store = store_at(&proxy_url, prefix.clone()).await;
    let direct_store = store_at(&upstream_url, prefix).await;

    let mut add_task = tokio::spawn({
        let add_store = Arc::clone(&add_store);
        async move { add_store.add_raw("k", "added", None).await }
    });
    let boundary = tokio::time::timeout(Duration::from_secs(2), boundary_rx.recv())
        .await
        .expect("add operation reaches the proxy boundary")
        .expect("proxy reports add boundary");

    let (added, post_script_cleanup) = match boundary {
        AddBoundary::SplitCleanupBlocked => {
            direct_store
                .tagged_put_raw(&["fresh-tag"], "k", "tagged-newer", None)
                .await
                .expect("install newer tagged value while stale cleanup is blocked");
            release_cleanup
                .send(())
                .expect("release delayed auxiliary cleanup");
            (
                add_task.await.expect("add task").expect("add succeeds"),
                false,
            )
        }
        AddBoundary::AtomicScriptExecuted => {
            assert!(
                !add_task.is_finished(),
                "the proxy must hold the executed script response at the caller boundary"
            );
            direct_store
                .tagged_put_raw(&["fresh-tag"], "k", "tagged-newer", None)
                .await
                .expect("overwrite while add_raw is waiting for its script response");
            release_script_response
                .send(())
                .expect("release executed script response");

            tokio::time::timeout(Duration::from_secs(2), async {
                tokio::select! {
                    result = &mut add_task => (
                        result.expect("add task").expect("add succeeds"),
                        false,
                    ),
                    next = boundary_rx.recv() => {
                        assert_eq!(
                            next,
                            Some(AddBoundary::SplitCleanupBlocked),
                            "only a post-script auxiliary cleanup may follow the EVAL response"
                        );
                        release_cleanup
                            .send(())
                            .expect("release post-script auxiliary cleanup");
                        (
                            add_task.await.expect("add task").expect("add succeeds"),
                            true,
                        )
                    }
                }
            })
            .await
            .expect("add completes or exposes a post-script cleanup")
        }
    };
    assert!(added, "the initial conditional add must win");
    assert!(
        !post_script_cleanup,
        "add_raw must not issue auxiliary cleanup after its atomic script returns"
    );

    direct_store
        .flush_tags(&["fresh-tag"])
        .await
        .expect("flush current tag");
    let remaining = direct_store.get_raw("k").await.expect("read cache value");
    direct_store.flush().await.expect("clean test prefix");
    drop(add_store);
    let mut proxy_task = proxy_task;
    if tokio::time::timeout(Duration::from_secs(1), &mut proxy_task)
        .await
        .is_err()
    {
        proxy_task.abort();
        let _ = proxy_task.await;
    }

    assert_eq!(
        remaining, None,
        "a delayed add cleanup must not detach the newer tagged value from its tag"
    );
}

/// A regular `Cache::forget("lock:foo")` MUST NOT release a held
/// distributed lock for `foo`. Pre-isolation, the lock value lived at
/// `<prefix>lock:foo` and was indistinguishable from a user-side
/// `forget("lock:foo")` (which also produced `<prefix>lock:foo`).
#[tokio::test]
#[ignore = "requires Redis at CACHE_REDIS_TEST_URL or default localhost"]
async fn redis_forget_with_lock_prefixed_key_does_not_release_held_lock() {
    let s = fresh_store("redis-lock-iso-1").await;
    let token = s
        .acquire_lock("printer", Duration::from_secs(30))
        .await
        .unwrap()
        .expect("lock acquired");

    // User-side `forget("lock:printer")` must NOT touch the lock's
    // internal slot.
    let _ = s.forget("lock:printer").await.unwrap();

    assert!(
        s.acquire_lock("printer", Duration::from_secs(30))
            .await
            .unwrap()
            .is_none(),
        "lock keyspace must be isolated from user `forget(\"lock:...\")`"
    );
    assert!(s.release_lock("printer", &token).await.unwrap());
}

/// A user-side `put("lock:foo", ...)` MUST NOT overwrite a held
/// distributed lock for `foo`.
#[tokio::test]
#[ignore = "requires Redis at CACHE_REDIS_TEST_URL or default localhost"]
async fn redis_put_with_lock_prefixed_key_does_not_overwrite_held_lock() {
    let s = fresh_store("redis-lock-iso-2").await;
    let token = s
        .acquire_lock("job", Duration::from_secs(30))
        .await
        .unwrap()
        .expect("lock acquired");

    s.put_raw("lock:job", "hijacked-token", Some(Duration::from_secs(30)))
        .await
        .unwrap();

    assert!(
        s.acquire_lock("job", Duration::from_secs(30))
            .await
            .unwrap()
            .is_none(),
        "lock keyspace must be isolated from user `put(\"lock:...\")`"
    );
    assert!(s.release_lock("job", &token).await.unwrap());
}

#[tokio::test]
#[ignore = "requires Redis at CACHE_REDIS_TEST_URL or default localhost"]
async fn redis_sentinel_prefixed_user_key_is_disjoint_from_lock_storage() {
    let s = fresh_store("redis-lock-iso-sentinel").await;
    let user_key = "\0lock:job";
    s.put_raw(user_key, "user-value", None).await.unwrap();

    let token = s
        .acquire_lock("job", Duration::from_secs(30))
        .await
        .unwrap()
        .expect("user data must not block lock acquisition");
    assert_eq!(
        s.get_raw(user_key).await.unwrap().as_deref(),
        Some("user-value")
    );

    assert!(s.forget(user_key).await.unwrap());
    assert!(
        s.acquire_lock("job", Duration::from_secs(30))
            .await
            .unwrap()
            .is_none(),
        "user deletion must not change lock ownership"
    );
    assert!(s.release_lock("job", &token).await.unwrap());
}

/// A `Cache::forget("tag:users")` MUST NOT clobber the tag forward
/// index for `users`. Pre-isolation, the forward index lived at
/// `<prefix>tag:users` and could be deleted by a user-side
/// `forget("tag:users")`, breaking subsequent `flush_tags(["users"])`.
#[tokio::test]
#[ignore = "requires Redis at CACHE_REDIS_TEST_URL or default localhost"]
async fn redis_forget_with_tag_prefixed_key_does_not_clobber_tag_index() {
    let s = fresh_store("redis-tag-iso").await;
    s.tagged_put_raw(&["users"], "u:1", "{\"id\":1}", None)
        .await
        .unwrap();

    // User-side forget against the same prefix we used to store the
    // forward index - must miss because the internal index lives in
    // a NUL-byte-prefixed slot the user cannot reach.
    let _ = s.forget("tag:users").await.unwrap();

    s.flush_tags(&["users"]).await.unwrap();
    assert!(
        !s.has("u:1").await.unwrap(),
        "flush_tags must still find and delete tagged keys"
    );
}

#[tokio::test]
#[ignore = "requires Redis at CACHE_REDIS_TEST_URL or default localhost"]
async fn redis_sentinel_prefixed_user_key_is_disjoint_from_tag_index() {
    let s = fresh_store("redis-tag-iso-sentinel").await;
    s.tagged_put_raw(&["users"], "u:1", "{\"id\":1}", None)
        .await
        .unwrap();

    s.put_raw("\0tag:users", "user-value", None).await.unwrap();
    s.flush_tags(&["users"]).await.unwrap();

    assert!(!s.has("u:1").await.unwrap());
    assert_eq!(
        s.get_raw("\0tag:users").await.unwrap().as_deref(),
        Some("user-value")
    );
}

/// A tag holding more members than one `SSCAN` round returns must still
/// flush completely.
///
/// `flush_tags` used to call `SMEMBERS`, which materialises the whole
/// forward index at once - unbounded in Redis and again in this process.
/// It now scans in batches, and a batching bug is invisible on the small
/// tags every other test here uses: the first round would flush and the
/// loop would stop, leaving the rest cached forever. 600 keys against a
/// 256-hint batch guarantees several rounds.
#[tokio::test]
#[ignore = "requires a running Redis"]
async fn redis_flush_tags_spans_multiple_scan_rounds() {
    let s = fresh_store("redis-tags-batched").await;

    const N: usize = 600;
    for i in 0..N {
        s.tagged_put_raw(&["bulk"], &format!("b:{i}"), "v", None)
            .await
            .unwrap();
    }
    // Sanity: the writes landed, so a later "all gone" is meaningful.
    assert!(s.has("b:0").await.unwrap());
    assert!(s.has(&format!("b:{}", N - 1)).await.unwrap());

    s.flush_tags(&["bulk"]).await.unwrap();

    let mut survivors = Vec::new();
    for i in 0..N {
        let key = format!("b:{i}");
        if s.has(&key).await.unwrap() {
            survivors.push(key);
        }
    }
    assert!(
        survivors.is_empty(),
        "{} of {N} tagged keys survived the flush; the scan loop stopped early. \
         First few: {:?}",
        survivors.len(),
        &survivors[..survivors.len().min(5)]
    );
}
