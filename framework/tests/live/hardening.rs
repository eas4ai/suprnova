//! Regression tests for the 2026-09-13 adversarial audit, one per agreed
//! requirement in `docs/spec/live.md` (LIVE-016 to LIVE-020).
//!
//! Each test is the audit's own probe with its assertion inverted to the
//! agreed behavior, so each fails on the tree the audit examined and passes
//! once its fix lands. The mechanism under `.cairn/mechanisms/live-*` runs
//! exactly one of these by name; the test name is the contract there.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use crate::live_async_support;
use bytes::Bytes;
use futures_util::future::join_all;
use hyper::Method;
use live_async_support::*;
use suprnova::live::{
    CanonicalValue, LiveEventTarget, LiveRegistry, LiveStreams, RegistryErrorKind,
};
use suprnova::session::{SessionData, SessionStore};
use suprnova::testing::TestContainer;
use suprnova::{Gate, LiveComponent, live};

/// LIVE-016: authorization is re-evaluated before each asynchronous
/// delivery, and a membership whose authorization no longer holds is
/// retired. The audit (ASTRA-01) redefined a stream's Gate to deny, saw a
/// new subscription refused with 403, and still received an event
/// published afterwards on the existing stream.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revoked_gate_ends_delivery() {
    let (router, _runtime) = router_and_runtime();
    let server = spawn_server(router).await;
    let alice = Identity::alice();
    let issued = issue(
        server.port,
        &alice,
        orders_issue_body("sse", "doc-instance-0001"),
    )
    .await;
    let credential = issued.credential.clone().expect("an SSE credential");
    let mut stream = SseClient::open(server.port, &alice, &credential, 1, &[]).await;
    assert_eq!(stream.status.as_u16(), 200);
    let ack = subscribe(
        server.port,
        &alice,
        &credential,
        &issued,
        "nonce-subscribe-0001",
        1,
    )
    .await;
    assert_eq!(ack.status.as_u16(), 200);

    Gate::define::<String, String>("live:tests.async-orders.stream.orders", |_, _| false);
    let denied = post_control(
        server.port,
        &alice,
        SUBSCRIPTION_PATH,
        None,
        orders_issue_body("sse", "doc-instance-0002"),
    )
    .await;
    assert_eq!(denied.status.as_u16(), 403, "a new subscription is refused");

    LiveStreams::resolve()
        .expect("the Live streams facade resolves")
        .event::<OrdersUpdated>(
            "orders",
            LiveEventTarget::Island,
            CanonicalValue::String("post-revocation".into()),
        )
        .await
        .expect("publishing to a topic with no authorized member is not an error");

    // Whatever the stream still carries (a heartbeat, an unsubscribe
    // control, or nothing before it ends), the revoked marker never arrives.
    let leaked = tokio::time::timeout(Duration::from_secs(3), async {
        while let Some(data) = stream.next_data().await {
            if data.to_string().contains("post-revocation") {
                return true;
            }
        }
        false
    })
    .await
    .unwrap_or(false);
    assert!(
        !leaked,
        "an event published after the Gate denied reached the old stream"
    );
}

/// Reads the stream for up to three seconds and reports whether `marker`
/// arrived in any data record.
async fn stream_carries(stream: &mut SseClient, marker: &str) -> bool {
    tokio::time::timeout(Duration::from_secs(3), async {
        while let Some(data) = stream.next_data().await {
            if data.to_string().contains(marker) {
                return true;
            }
        }
        false
    })
    .await
    .unwrap_or(false)
}

/// LIVE-019: a membership does not outlive the session that opened it on
/// the node that destroys the session. The stream is issued under the real
/// session middleware, the browser logs out through
/// `Auth::logout_and_invalidate`, and an event published afterwards must
/// never reach the old stream (the open clause of LIVE-016).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revoked_session_ends_delivery() {
    let (router, _runtime) = router_and_runtime();
    let store = Arc::new(MemorySessionStore::default());
    let server = spawn_server_with_sessions(router, Arc::clone(&store)).await;
    let bootstrap = send(
        server.port,
        &Identity::alice(),
        Method::GET,
        "/session/touch",
        &[],
        Bytes::new(),
    )
    .await;
    assert_eq!(bootstrap.status.as_u16(), 200);
    let cookie = session_cookie(&bootstrap).expect("the session middleware set its cookie");
    let alice = Identity::alice().with_cookie(&cookie);
    let issued = issue(
        server.port,
        &alice,
        orders_issue_body("sse", "doc-instance-0001"),
    )
    .await;
    let credential = issued.credential.clone().expect("an SSE credential");
    let mut stream = SseClient::open(server.port, &alice, &credential, 1, &[]).await;
    assert_eq!(stream.status.as_u16(), 200);
    let ack = subscribe(
        server.port,
        &alice,
        &credential,
        &issued,
        "nonce-subscribe-0001",
        1,
    )
    .await;
    assert_eq!(ack.status.as_u16(), 200);

    let logout = send(
        server.port,
        &alice,
        Method::POST,
        "/session/logout",
        &[],
        Bytes::new(),
    )
    .await;
    assert_eq!(
        logout.status.as_u16(),
        200,
        "logout failed: {}",
        String::from_utf8_lossy(&logout.body)
    );

    LiveStreams::resolve()
        .expect("the Live streams facade resolves")
        .event::<OrdersUpdated>(
            "orders",
            LiveEventTarget::Island,
            CanonicalValue::String("post-logout".into()),
        )
        .await
        .expect("publishing to a topic with no live member is not an error");

    assert!(
        !stream_carries(&mut stream, "post-logout").await,
        "an event published after the session was destroyed reached the old stream"
    );
}

/// LIVE-021: the logout the scaffold ships, plain `Auth::logout`, keeps the
/// session row and id and only clears the signed-in user; the memberships
/// that session opened for that user must still end with it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plain_logout_ends_delivery() {
    let (router, _runtime) = router_and_runtime();
    let store = Arc::new(MemorySessionStore::default());
    let server = spawn_server_with_sessions(router, Arc::clone(&store)).await;
    let bootstrap = send(
        server.port,
        &Identity::alice(),
        Method::GET,
        "/session/touch",
        &[],
        Bytes::new(),
    )
    .await;
    assert_eq!(bootstrap.status.as_u16(), 200);
    let cookie = session_cookie(&bootstrap).expect("the session middleware set its cookie");
    let alice = Identity::alice().with_cookie(&cookie);
    let issued = issue(
        server.port,
        &alice,
        orders_issue_body("sse", "doc-instance-0001"),
    )
    .await;
    let credential = issued.credential.clone().expect("an SSE credential");
    let mut stream = SseClient::open(server.port, &alice, &credential, 1, &[]).await;
    assert_eq!(stream.status.as_u16(), 200);
    let ack = subscribe(
        server.port,
        &alice,
        &credential,
        &issued,
        "nonce-subscribe-0001",
        1,
    )
    .await;
    assert_eq!(ack.status.as_u16(), 200);

    let logout = send(
        server.port,
        &alice,
        Method::POST,
        "/session/logout-plain",
        &[],
        Bytes::new(),
    )
    .await;
    assert_eq!(
        logout.status.as_u16(),
        200,
        "logout failed: {}",
        String::from_utf8_lossy(&logout.body)
    );

    LiveStreams::resolve()
        .expect("the Live streams facade resolves")
        .event::<OrdersUpdated>(
            "orders",
            LiveEventTarget::Island,
            CanonicalValue::String("post-plain-logout".into()),
        )
        .await
        .expect("publishing to a topic with no live member is not an error");

    assert!(
        !stream_carries(&mut stream, "post-plain-logout").await,
        "an event published after a plain logout reached the old stream"
    );
}

/// LIVE-020: a session destroyed behind the runtime, as another node's
/// logout does, stops delivery within the re-verification interval. The
/// session row is removed from the shared store directly, the clock passes
/// ten seconds, and a publish must not reach the membership.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stale_store_session_ends_delivery() {
    let (router, _runtime, clock) = router_and_runtime_with_clock();
    let server = spawn_server(router).await;
    let session_id = "livesessionstale00000000000000000000000a".to_owned();
    assert_eq!(session_id.len(), 40, "a store-shaped session id");
    let store = Arc::new(MemorySessionStore::default());
    store.seed(SessionData::new(
        session_id.clone(),
        "csrf-token".to_owned(),
    ));
    let shared: Arc<dyn SessionStore> = Arc::clone(&store) as Arc<dyn SessionStore>;
    TestContainer::scope(async move {
        TestContainer::bind::<dyn SessionStore>(shared);
        let alice = Identity::alice().with_session(&session_id);
        let issued = issue(
            server.port,
            &alice,
            orders_issue_body("sse", "doc-instance-0001"),
        )
        .await;
        let credential = issued.credential.clone().expect("an SSE credential");
        let mut stream = SseClient::open(server.port, &alice, &credential, 1, &[]).await;
        assert_eq!(stream.status.as_u16(), 200);
        let ack = subscribe(
            server.port,
            &alice,
            &credential,
            &issued,
            "nonce-subscribe-0001",
            1,
        )
        .await;
        assert_eq!(ack.status.as_u16(), 200);
        let streams = LiveStreams::resolve().expect("the Live streams facade resolves");

        streams
            .event::<OrdersUpdated>(
                "orders",
                LiveEventTarget::Island,
                CanonicalValue::String("before-removal".into()),
            )
            .await
            .expect("publish to a live member");
        assert!(
            stream_carries(&mut stream, "before-removal").await,
            "a member whose session the store holds receives events"
        );

        store.remove(&session_id);
        clock.advance_ms(10_001);
        streams
            .event::<OrdersUpdated>(
                "orders",
                LiveEventTarget::Island,
                CanonicalValue::String("post-removal".into()),
            )
            .await
            .expect("publishing to a topic with no live member is not an error");
        assert!(
            !stream_carries(&mut stream, "post-removal").await,
            "an event published after the store dropped the session reached the stream"
        );
    })
    .await;
}

#[derive(LiveComponent)]
#[live(
    name = "tests.hardening-required",
    view = "live/tests/hardening-required.html"
)]
pub struct RequiredTransactionComponent {
    #[model]
    note: String,
}

#[live]
impl RequiredTransactionComponent {
    #[mount]
    pub fn mount() -> Self {
        Self {
            note: String::new(),
        }
    }

    #[action(transaction = "required")]
    pub fn save(&mut self) {}
}

#[derive(LiveComponent)]
#[live(
    name = "tests.hardening-plain",
    view = "live/tests/hardening-plain.html"
)]
pub struct PlainTransactionComponent {
    #[model]
    note: String,
}

#[live]
impl PlainTransactionComponent {
    #[mount]
    pub fn mount() -> Self {
        Self {
            note: String::new(),
        }
    }

    #[action]
    pub fn save(&mut self) {}
}

/// LIVE-017, under the 2026-09-13 decision: until the framework runs a
/// Required action inside one ambient transaction its writes join, the
/// registry refuses a component whose action declares the policy. The
/// audit (ASTRA-05) found the transaction port a documented no-op, so the
/// policy promised atomicity it never provided.
#[tokio::test]
async fn required_transaction_is_refused_until_real() {
    let refused = LiveRegistry::builder()
        .register::<RequiredTransactionComponent>()
        .expect_err("a Required-transaction action is refused at registration");
    assert_eq!(
        refused.kind(),
        RegistryErrorKind::RequiredTransactionUnsupported
    );

    LiveRegistry::builder()
        .register::<PlainTransactionComponent>()
        .expect("an action without the policy registers as before");
}

/// LIVE-018: a subscription slot is reserved under the per-scope limit
/// before the authorizer is awaited, and released on every error path. The
/// audit (ASTRA-07) reasoned from source that 513 concurrent issuances
/// against a delayed authorizer would all pass the count check before any
/// inserted; this test holds every request at the authorizer and releases
/// them together.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn issuance_cap_holds_under_concurrency() {
    const BURST: usize = 513;
    const LIMIT: usize = 512;
    let (router, _runtime) = router_and_runtime();
    let server = spawn_server(router).await;
    let gate_calls = Arc::new(AtomicUsize::new(0));
    let gate_calls_in_gate = Arc::clone(&gate_calls);
    let (release_tx, release_rx) = tokio::sync::watch::channel(false);
    Gate::define_async::<String, String, _, _>(
        "live:tests.async-orders.stream.orders",
        move |_, _| {
            let gate_calls = Arc::clone(&gate_calls_in_gate);
            let mut release = release_rx.clone();
            async move {
                gate_calls.fetch_add(1, Ordering::SeqCst);
                if !*release.borrow() {
                    let _ = release.changed().await;
                }
                true
            }
        },
    );
    let port = server.port;
    let requests = tokio::spawn(async move {
        join_all((0..BURST).map(|_| async move {
            let identity = Identity::alice();
            post_control(
                port,
                &identity,
                SUBSCRIPTION_PATH,
                None,
                orders_issue_body("sse", "doc-instance-limit"),
            )
            .await
        }))
        .await
    });
    // With slots reserved before authorization, at most LIMIT requests ever
    // reach the authorizer; the rest are refused first.
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if gate_calls.load(Ordering::SeqCst) >= LIMIT {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the admitted requests reached authorization");
    release_tx.send(true).expect("release the authorizer");
    let replies = tokio::time::timeout(Duration::from_secs(30), requests)
        .await
        .expect("the burst completed")
        .expect("the burst task did not panic");
    let successes = replies
        .iter()
        .filter(|reply| reply.status.as_u16() == 201)
        .count();
    let limited = replies
        .iter()
        .filter(|reply| reply.status.as_u16() == 409)
        .count();
    // The cap is the contract: the limit refuses exactly the overflow, and no
    // more than LIMIT requests are ever admitted to authorization, whatever
    // else those admitted requests then meet.
    let mut histogram = std::collections::BTreeMap::new();
    for reply in &replies {
        let key = if reply.status.as_u16() == 201 {
            "201".to_owned()
        } else {
            format!("{} {}", reply.status.as_u16(), reply.json())
        };
        *histogram.entry(key).or_insert(0_usize) += 1;
    }
    let admitted = replies.len() - limited;
    assert!(
        admitted <= LIMIT,
        "the per-scope limit admitted {admitted} concurrent issuances (limit {LIMIT}); \
         statuses seen: {histogram:?}"
    );
    assert!(
        successes <= LIMIT,
        "more issuances succeeded than the limit allows: {successes}"
    );
    assert_eq!(
        limited,
        BURST - LIMIT,
        "exactly the overflow was refused by the limit; statuses seen: {histogram:?}"
    );
}
