//! Integration tests for the webhook ingress route - idempotency layer.
//!
//! Tests:
//!   1. `duplicate_webhook_deduped_and_returns_ok` - posting the same webhook
//!      twice persists exactly one row and both calls return 200.
//!   2. `webhook_for_unknown_provider_returns_404` - a request for an
//!      unregistered provider name returns 404.
//!   3. `blank_event_ids_are_rejected_without_persisting_state` - malformed
//!      identifiers never enter the shared idempotency namespace.
//!
//! These tests use a real TCP server (`spawn_server`) and a raw hyper HTTP
//! client (`send_webhook`) to drive the handler end-to-end.  No tower or
//! axum test helpers are used - the framework is hyper-native.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use sea_orm::EntityTrait;
use sea_orm_migration::MigratorTrait;
use serde_json::json;
use suprnova::handle_request;
use suprnova::payments::{
    Checkout, CreateCustomerRequest, CustomerRef, CustomerStore, MockPaymentProvider, PaymentError,
    PaymentProvider, PaymentProviderRegistry, PaymentResult, SessionPayload, StartSessionRequest,
    SubscribeRequest, Subscription, SubscriptionResult, UpdateCustomerRequest,
    UpdateSubscriptionRequest, WebhookContext, WebhookEvent, WebhookHandler,
    entities::{transaction, webhook_event},
    webhook_routes,
};
use suprnova::testing::TestDatabase;
use suprnova::{MiddlewareRegistry, Router};

// ── migrator ──────────────────────────────────────────────────────────────────

struct PaymentsTestMigrator;

#[async_trait::async_trait]
impl MigratorTrait for PaymentsTestMigrator {
    fn migrations() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
        suprnova::payments::migrations::migrations()
    }
}

// ── test server helpers ───────────────────────────────────────────────────────

/// Stand up a fresh TCP server on an ephemeral port that dispatches requests
/// through `router`.  Shuts down after `accepts` connections.
async fn spawn_server(router: Router, accepts: usize) -> SocketAddr {
    let router = Arc::new(router);
    let middleware = Arc::new(MiddlewareRegistry::new());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral listener");
    let addr = listener.local_addr().expect("local_addr");

    tokio::spawn(async move {
        for _ in 0..accepts {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let io = TokioIo::new(stream);
            let router = router.clone();
            let middleware = middleware.clone();
            tokio::spawn(async move {
                let svc = service_fn(move |req: hyper::Request<Incoming>| {
                    let router = router.clone();
                    let middleware = middleware.clone();
                    async move { Ok::<_, Infallible>(handle_request(router, middleware, req).await) }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(io, svc)
                    .await;
            });
        }
    });

    addr
}

/// POST `body` to `path` on `addr`.  Returns `(status_code, response_body)`.
async fn send_webhook(
    addr: SocketAddr,
    path: &str,
    body: Bytes,
) -> (hyper::http::StatusCode, Bytes) {
    let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let io = TokioIo::new(stream);
    let (mut sender, conn) = hyper::client::conn::http1::handshake::<_, Full<Bytes>>(io)
        .await
        .unwrap();
    tokio::spawn(async move {
        let _ = conn.await;
    });

    let content_len = body.len().to_string();
    let req = hyper::Request::builder()
        .method("POST")
        .uri(path)
        .header("Host", "localhost")
        .header("Content-Type", "application/json")
        .header("Content-Length", content_len)
        .body(Full::new(body))
        .unwrap();

    let resp = tokio::time::timeout(Duration::from_secs(5), sender.send_request(req))
        .await
        .expect("send_webhook timeout")
        .expect("hyper send_request");
    let (parts, resp_body) = resp.into_parts();
    let collected = resp_body.collect().await.unwrap().to_bytes();
    (parts.status, collected)
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// Register the mock provider under a test-scoped name and return the concrete
/// Arc so the caller can pre-populate provider-side state (e.g. subscriptions).
///
/// Each test uses a unique name so parallel runs don't stomp on each other
/// inside the process-global registry.
fn register_mock(name: &'static str) -> Arc<MockPaymentProvider> {
    let mock = Arc::new(MockPaymentProvider::new());
    let as_trait: Arc<dyn PaymentProvider> = mock.clone();
    PaymentProviderRegistry::bind(name, as_trait);
    mock
}

struct BlankEventIdProvider;

impl PaymentProvider for BlankEventIdProvider {
    fn name(&self) -> &'static str {
        "blank-event-id-probe"
    }
}

#[async_trait::async_trait]
impl WebhookHandler for BlankEventIdProvider {
    fn verify(&self, _ctx: &WebhookContext<'_>) -> PaymentResult<()> {
        Ok(())
    }

    fn parse_event(&self, body: &[u8]) -> PaymentResult<WebhookEvent> {
        let raw_payload: serde_json::Value = serde_json::from_slice(body)
            .map_err(|error| PaymentError::Validation(format!("invalid probe body: {error}")))?;
        Ok(WebhookEvent {
            provider: self.name().into(),
            provider_event_id: String::new(),
            provider_event_type: "probe.event".into(),
            neutral: None,
            raw_payload,
        })
    }
}

#[async_trait::async_trait]
impl Checkout for BlankEventIdProvider {
    async fn start_session(&self, _req: StartSessionRequest) -> PaymentResult<SessionPayload> {
        Err(PaymentError::NotSupported("probe".into()))
    }
}

#[async_trait::async_trait]
impl Subscription for BlankEventIdProvider {
    async fn subscribe(&self, _req: SubscribeRequest) -> PaymentResult<SubscriptionResult> {
        Err(PaymentError::NotSupported("probe".into()))
    }

    async fn update(&self, _req: UpdateSubscriptionRequest) -> PaymentResult<SubscriptionResult> {
        Err(PaymentError::NotSupported("probe".into()))
    }

    async fn cancel(&self, _id: &str, _at_period_end: bool) -> PaymentResult<SubscriptionResult> {
        Err(PaymentError::NotSupported("probe".into()))
    }

    async fn get(&self, _id: &str) -> PaymentResult<SubscriptionResult> {
        Err(PaymentError::NotFound("probe".into()))
    }
}

#[async_trait::async_trait]
impl CustomerStore for BlankEventIdProvider {
    async fn create_customer(&self, _req: CreateCustomerRequest) -> PaymentResult<CustomerRef> {
        Err(PaymentError::NotSupported("probe".into()))
    }

    async fn update_customer(&self, _req: UpdateCustomerRequest) -> PaymentResult<CustomerRef> {
        Err(PaymentError::NotSupported("probe".into()))
    }

    async fn get_customer(&self, _id: &str) -> PaymentResult<CustomerRef> {
        Err(PaymentError::NotFound("probe".into()))
    }

    async fn delete_customer(&self, _id: &str) -> PaymentResult<()> {
        Err(PaymentError::NotFound("probe".into()))
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

/// Posting the same webhook payload twice:
///   - First POST: 200 "ok" - event inserted and processed.
///   - Second POST: 200 "duplicate" - idempotency check fires, no second row.
///   - DB: exactly one row in `payments_webhook_events` with matching fields.
#[tokio::test]
async fn duplicate_webhook_deduped_and_returns_ok() {
    let provider_name: &'static str = "mock-idem-dedup";
    let mock = register_mock(provider_name);

    let db = TestDatabase::fresh::<PaymentsTestMigrator>()
        .await
        .expect("TestDatabase::fresh");
    let conn = Arc::new(db.conn().clone());

    // Pre-populate a subscription in the mock so the webhook's
    // Subscription::get(id) call returns canonical state instead of NotFound -
    // we're testing idempotency, but the full hydration path now runs in a
    // transaction and a missing subscription would fail with 503.
    let sub = mock
        .subscribe(SubscribeRequest {
            customer_ref: "cus_idem_test".into(),
            price_refs: vec!["price_idem".into()],
            trial_days: None,
            idempotency_key: None,
            metadata: None,
        })
        .await
        .expect("mock subscribe");

    let router = webhook_routes(conn.clone());
    // 4 accepts: 2 for the two POSTs + 2 headroom for connection keep-alive
    let addr = spawn_server(router, 4).await;

    let webhook_body = Bytes::from(
        json!({
            "id": "evt_unique_abc123",
            "type": "subscription.created",
            "data": { "object": {
                "id": sub.provider_subscription_id,
                "customer": sub.provider_customer_id
            }}
        })
        .to_string(),
    );
    let path = format!("/webhooks/payments/{provider_name}");

    // First request - fresh event.
    let (status1, body1) = send_webhook(addr, &path, webhook_body.clone()).await;
    assert_eq!(
        status1.as_u16(),
        200,
        "first POST must return 200, got body: {}",
        String::from_utf8_lossy(&body1)
    );
    assert_eq!(body1.as_ref(), b"ok", "first POST must return body 'ok'");

    // Second request - duplicate.
    let (status2, body2) = send_webhook(addr, &path, webhook_body.clone()).await;
    assert_eq!(
        status2.as_u16(),
        200,
        "duplicate POST must return 200, got body: {}",
        String::from_utf8_lossy(&body2)
    );
    assert_eq!(
        body2.as_ref(),
        b"duplicate",
        "duplicate POST must return body 'duplicate'"
    );

    // DB: exactly one row.
    let rows = webhook_event::Entity::find()
        .all(&*conn)
        .await
        .expect("db query");
    assert_eq!(
        rows.len(),
        1,
        "duplicate webhook must not create a second DB row; found {} rows",
        rows.len()
    );
    assert_eq!(rows[0].provider, "mock");
    assert_eq!(rows[0].provider_event_id, "evt_unique_abc123");
    assert_eq!(rows[0].provider_event_type, "subscription.created");
    assert!(
        rows[0].processed_at.is_some(),
        "processed_at must be set after successful processing"
    );
}

#[tokio::test]
async fn blank_event_ids_are_rejected_without_persisting_state() {
    let provider_name: &'static str = "mock-idem-blank-event-id";
    let provider: Arc<dyn PaymentProvider> = Arc::new(BlankEventIdProvider);
    PaymentProviderRegistry::bind(provider_name, provider);

    let db = TestDatabase::fresh::<PaymentsTestMigrator>()
        .await
        .expect("TestDatabase::fresh");
    let conn = Arc::new(db.conn().clone());
    let router = webhook_routes(conn.clone());
    let addr = spawn_server(router, 2).await;
    let path = format!("/webhooks/payments/{provider_name}");

    let first = Bytes::from(
        json!({
            "id": "",
            "type": "payment.succeeded",
            "data": { "object": {
                "id": "txn_blank_event_a",
                "customer": "cus_blank_event_a",
                "amount": 100,
                "currency": "USD"
            }}
        })
        .to_string(),
    );
    let second = Bytes::from(
        json!({
            "id": "",
            "type": "payment.succeeded",
            "data": { "object": {
                "id": "txn_blank_event_b",
                "customer": "cus_blank_event_b",
                "amount": 200,
                "currency": "EUR"
            }}
        })
        .to_string(),
    );

    let (first_status, _) = send_webhook(addr, &path, first).await;
    let (second_status, _) = send_webhook(addr, &path, second).await;
    assert_eq!(first_status.as_u16(), 400);
    assert_eq!(second_status.as_u16(), 400);
    assert!(
        webhook_event::Entity::find()
            .all(&*conn)
            .await
            .expect("audit query")
            .is_empty(),
        "invalid event identifiers must not enter the audit table"
    );
    assert!(
        transaction::Entity::find()
            .all(&*conn)
            .await
            .expect("transaction query")
            .is_empty(),
        "invalid event identifiers must not hydrate transaction mirrors"
    );
}

/// A POST to `/webhooks/payments/unregistered-xyz` returns 404.
#[tokio::test]
async fn webhook_for_unknown_provider_returns_404() {
    let db = TestDatabase::fresh::<PaymentsTestMigrator>()
        .await
        .expect("TestDatabase::fresh");
    let conn = Arc::new(db.conn().clone());

    let router = webhook_routes(conn.clone());
    let addr = spawn_server(router, 2).await;

    let body = Bytes::from(b"{\"id\":\"evt_1\",\"type\":\"test\"}".to_vec());
    let (status, resp_body) = send_webhook(addr, "/webhooks/payments/unregistered-xyz", body).await;
    assert_eq!(
        status.as_u16(),
        404,
        "unknown provider must return 404, got body: {}",
        String::from_utf8_lossy(&resp_body)
    );
}
