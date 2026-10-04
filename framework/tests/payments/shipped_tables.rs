//! Webhooks hydrate the payment mirror tables the payments migration
//! creates, on every engine.
//!
//! The migration creates every time column - `created_at`, `updated_at`,
//! the subscription period, `canceled_at`, `paid_at`, `received_at`,
//! `processed_at` - as `timestamp with time zone` on Postgres and
//! `TIMESTAMP` on MySQL and MariaDB. The payment models stored them
//! through the text cast `AsDateTime`, so on those engines the first
//! webhook write failed and no mirror row could be read back.
//!
//! ```bash
//! MYSQL_TEST_URL=mysql://... cargo test -p suprnova --test payments -- --ignored shipped_tables::mysql_
//! PG_TEST_URL=postgres://... cargo test -p suprnova --test payments -- --ignored shipped_tables::postgres_
//! ```

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use sea_orm::{ColumnTrait, ConnectionTrait, Database, EntityTrait, QueryFilter};
use sea_orm_migration::SchemaManager;
use serde_json::json;

use suprnova::payments::entities::{subscription, subscription_item, transaction, webhook_event};
use suprnova::payments::{
    MockPaymentProvider, PaymentProvider, PaymentProviderRegistry, SubscribeRequest, Subscription,
    webhook_routes,
};
use suprnova::{MiddlewareRegistry, Router, handle_request};

const TABLES: [&str; 6] = [
    "payments_subscription_items",
    "payments_subscriptions",
    "payments_transactions",
    "payments_payment_methods",
    "payments_customers",
    "payments_webhook_events",
];

async fn serve(router: Router, accepts: usize) -> SocketAddr {
    let router = Arc::new(router);
    let middleware = Arc::new(MiddlewareRegistry::new());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        for _ in 0..accepts {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let router = router.clone();
            let middleware = middleware.clone();
            tokio::spawn(async move {
                let service = service_fn(move |request: hyper::Request<Incoming>| {
                    let router = router.clone();
                    let middleware = middleware.clone();
                    async move { Ok::<_, Infallible>(handle_request(router, middleware, request).await) }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .await;
            });
        }
    });
    addr
}

/// POST `event` to the webhook route and return `(status, body)`.
async fn post(addr: SocketAddr, path: &str, event: serde_json::Value) -> (u16, String) {
    let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let (mut sender, conn) =
        hyper::client::conn::http1::handshake::<_, Full<Bytes>>(TokioIo::new(stream))
            .await
            .unwrap();
    tokio::spawn(async move {
        let _ = conn.await;
    });
    let body = Bytes::from(event.to_string());
    let request = hyper::Request::builder()
        .method("POST")
        .uri(path)
        .header("Host", "localhost")
        .header("Content-Type", "application/json")
        .header("Content-Length", body.len().to_string())
        .body(Full::new(body))
        .unwrap();
    let response = tokio::time::timeout(Duration::from_secs(10), sender.send_request(request))
        .await
        .expect("webhook timed out")
        .expect("send webhook");
    let status = response.status().as_u16();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&body).into_owned())
}

async fn hydrate_subscription_and_transaction(url: &str, provider_name: &'static str) {
    let database = Database::connect(url).await.expect("connect test database");
    for table in TABLES {
        database
            .execute_unprepared(&format!("DROP TABLE IF EXISTS {table}"))
            .await
            .expect("drop payments table");
    }
    let manager = SchemaManager::new(&database);
    for migration in suprnova::payments::migrations::migrations() {
        migration.up(&manager).await.expect("payments migration");
    }

    let mock = Arc::new(MockPaymentProvider::new());
    let provider: Arc<dyn PaymentProvider> = mock.clone();
    PaymentProviderRegistry::bind(provider_name, provider);
    let sub = mock
        .subscribe(SubscribeRequest {
            customer_ref: "cus_engines".into(),
            price_refs: vec!["price_basic".into()],
            trial_days: None,
            idempotency_key: None,
            metadata: None,
        })
        .await
        .expect("mock subscribe");
    let sub_id = sub.provider_subscription_id.clone();
    let customer_id = sub.provider_customer_id.clone();

    let conn = Arc::new(database.clone());
    let addr = serve(webhook_routes(conn.clone()), 3).await;
    let path = format!("/webhooks/payments/{provider_name}");

    let created = post(
        addr,
        &path,
        json!({
            "id": "evt_engines_sub_created",
            "type": "subscription.created",
            "data": { "object": { "id": sub_id, "customer": customer_id } }
        }),
    )
    .await;
    mock.cancel(&sub_id, false).await.expect("mock cancel");
    let canceled = post(
        addr,
        &path,
        json!({
            "id": "evt_engines_sub_canceled",
            "type": "subscription.canceled",
            "data": { "object": { "id": sub_id, "customer": customer_id } }
        }),
    )
    .await;
    let paid = post(
        addr,
        &path,
        json!({
            "id": "evt_engines_paid",
            "type": "payment.succeeded",
            "data": {
                "object": {
                    "id": "txn_engines",
                    "customer": "cus_engines",
                    "amount": 4999,
                    "currency": "USD",
                    "paid_at": "2026-05-22T12:00:00+00:00"
                }
            }
        }),
    )
    .await;
    // One assertion over every delivery, so a failure shows each of them.
    assert_eq!(
        [created.0, canceled.0, paid.0],
        [200, 200, 200],
        "subscription.created {created:?}, subscription.canceled {canceled:?}, \
         payment.succeeded {paid:?}"
    );

    let mirror = subscription::Entity::find()
        .filter(subscription::Column::ProviderSubscriptionId.eq(sub_id.clone()))
        .one(&database)
        .await
        .expect("read the subscription mirror")
        .expect("subscription mirror row");
    assert_eq!(mirror.status, "canceled");
    assert!(mirror.canceled_at.is_some(), "canceled_at is stored");
    assert!(mirror.current_period_end > mirror.current_period_start);
    let items = subscription_item::Entity::find()
        .filter(subscription_item::Column::SubscriptionId.eq(mirror.id))
        .all(&database)
        .await
        .expect("read the subscription items");
    assert_eq!(items.len(), 1);

    let payment = transaction::Entity::find()
        .filter(transaction::Column::ProviderTransactionId.eq("txn_engines"))
        .one(&database)
        .await
        .expect("read the transaction mirror")
        .expect("transaction mirror row");
    let paid_at = format!("{:?}", payment.paid_at);
    assert!(
        paid_at.contains("2026-05-22T12:00:00"),
        "paid_at reads back as the UTC time the payload carried: {paid_at}"
    );

    let audits = webhook_event::Entity::find()
        .all(&database)
        .await
        .expect("read the webhook audit rows");
    assert_eq!(audits.len(), 3);
    assert!(
        audits.iter().all(|audit| audit.processed_at.is_some()),
        "every delivery is marked processed"
    );

    for table in TABLES {
        database
            .execute_unprepared(&format!("DROP TABLE {table}"))
            .await
            .expect("drop payments table");
    }
    database.close().await.unwrap();
}

#[tokio::test]
async fn sqlite_webhooks_hydrate_the_shipped_payment_tables() {
    hydrate_subscription_and_transaction("sqlite::memory:", "mock-shipped-tables-sqlite").await;
}

#[tokio::test]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_webhooks_hydrate_the_shipped_payment_tables() {
    let url = std::env::var("MYSQL_TEST_URL").expect("set MYSQL_TEST_URL");
    hydrate_subscription_and_transaction(&url, "mock-shipped-tables-mysql").await;
}

#[tokio::test]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_webhooks_hydrate_the_shipped_payment_tables() {
    let url = std::env::var("PG_TEST_URL").expect("set PG_TEST_URL");
    hydrate_subscription_and_transaction(&url, "mock-shipped-tables-postgres").await;
}
