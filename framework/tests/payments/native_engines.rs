//! The payments mirror tables on real Postgres, MySQL and MariaDB.
//!
//! The framework's payments migration creates native `timestamp with time
//! zone` columns (`TIMESTAMP` on MySQL). The mirror models must read and
//! write those columns as native date-times: a text-stored timestamp is
//! refused by Postgres on bind and by every engine's driver on decode, which
//! turned every webhook delivery into a 500 and every mirror read into an
//! error. The SQLite suite cannot see that, because SQLite stores both shapes
//! as text.
//!
//! Each test runs the shipped migration, drives the webhook route with the
//! mock provider, and creates, reads and updates a row of every mirror table
//! through the model API.
//!
//! ```text
//! PG_TEST_URL=postgres://... cargo test -p suprnova --test payments -- \
//!   --ignored --test-threads=1 native_engines::postgres_
//! MYSQL_TEST_URL=mysql://... cargo test -p suprnova --test payments -- \
//!   --ignored --test-threads=1 native_engines::mysql_
//! ```

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use chrono::{DateTime, TimeZone, Utc};
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use sea_orm::ConnectionTrait;
use sea_orm_migration::{MigrationTrait, SchemaManager};
use serde_json::json;
use suprnova::payments::entities::{
    customer::Customer, payment_method::PaymentMethod, subscription::Subscription,
    subscription_item::SubscriptionItem, transaction::Transaction, webhook_event::WebhookEvent,
};
use suprnova::payments::migrations::CreatePaymentsTables;
use suprnova::payments::{
    MockPaymentProvider, PaymentProvider, PaymentProviderRegistry, SubscribeRequest,
    Subscription as SubscriptionApi, webhook_routes,
};
use suprnova::testing::{TestClock, TestContainer, TestContainerGuard};
use suprnova::{
    DatabaseConfig, DbConnection, MiddlewareRegistry, Model, Router, attrs, handle_request,
};

/// The clock the test runs at. Whole seconds, so a MySQL `TIMESTAMP` of
/// precision 0 holds each moment exactly.
fn at(second: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2031, 3, 14, 9, 26, second).unwrap()
}

async fn connect_live(env: &str) -> (TestContainerGuard, DbConnection) {
    let url = std::env::var(env).expect("explicit disposable database URL required");
    let guard = TestContainer::fake();
    let config = DatabaseConfig::builder()
        .url(url)
        .max_connections(4)
        .min_connections(1)
        .logging(false)
        .build();
    let database = DbConnection::connect(&config)
        .await
        .expect("connect test database");
    TestContainer::singleton(database.clone());
    (guard, database)
}

/// Drop whatever an earlier run left, children first, and run the shipped
/// migration so the columns are exactly the ones an application gets.
async fn fresh_payments_schema(database: &DbConnection) {
    for table in [
        "payments_subscription_items",
        "payments_subscriptions",
        "payments_transactions",
        "payments_payment_methods",
        "payments_customers",
        "payments_webhook_events",
    ] {
        database
            .inner()
            .execute_unprepared(&format!("DROP TABLE IF EXISTS {table}"))
            .await
            .expect("drop a leftover payments table");
    }
    CreatePaymentsTables
        .up(&SchemaManager::new(database.inner()))
        .await
        .expect("run the payments migration");
}

async fn spawn_server(router: Router) -> SocketAddr {
    let router = Arc::new(router);
    let middleware = Arc::new(MiddlewareRegistry::new());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral listener");
    let addr = listener.local_addr().expect("local_addr");
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let router = router.clone();
            let middleware = middleware.clone();
            tokio::spawn(async move {
                let svc = service_fn(move |req: hyper::Request<Incoming>| {
                    let router = router.clone();
                    let middleware = middleware.clone();
                    async move { Ok::<_, Infallible>(handle_request(router, middleware, req).await) }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), svc)
                    .await;
            });
        }
    });
    addr
}

async fn post(addr: SocketAddr, path: &str, body: serde_json::Value) -> (u16, String) {
    let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let (mut sender, conn) =
        hyper::client::conn::http1::handshake::<_, Full<Bytes>>(TokioIo::new(stream))
            .await
            .unwrap();
    tokio::spawn(async move {
        let _ = conn.await;
    });
    let body = Bytes::from(body.to_string());
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
        .expect("webhook timeout")
        .expect("webhook send");
    let status = response.status().as_u16();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// The webhook route persists the audit row and hydrates the mirrors.
async fn webhooks_persist_and_hydrate(database: &DbConnection, provider_name: &'static str) {
    let mock = Arc::new(MockPaymentProvider::new());
    let provider: Arc<dyn PaymentProvider> = mock.clone();
    PaymentProviderRegistry::bind(provider_name, provider);
    let addr = spawn_server(webhook_routes(Arc::new(database.inner().clone()))).await;
    let path = format!("/webhooks/payments/{provider_name}");

    let sub = mock
        .subscribe(SubscribeRequest {
            customer_ref: "cus_engines".into(),
            price_refs: vec!["price_basic".into(), "price_seats".into()],
            trial_days: None,
            idempotency_key: None,
            metadata: None,
        })
        .await
        .expect("mock subscribe");
    let created = json!({
        "id": "evt_engines_sub_created",
        "type": "subscription.created",
        "data": { "object": { "id": sub.provider_subscription_id, "customer": "cus_engines" } }
    });
    let (status, body) = post(addr, &path, created.clone()).await;
    assert_eq!(status, 200, "subscription.created must persist: {body}");

    let audit = WebhookEvent::query()
        .filter("provider_event_id", "evt_engines_sub_created")
        .first()
        .await
        .expect("read the audit row")
        .expect("the audit row exists");
    assert_eq!(audit.received_at, at(0));
    assert_eq!(audit.processed_at, Some(at(0)));
    assert_eq!(audit.process_error, None);

    let (status, body) = post(addr, &path, created).await;
    assert_eq!(status, 200, "a replay is acknowledged: {body}");
    assert_eq!(
        WebhookEvent::query()
            .count()
            .await
            .expect("count audit rows"),
        1,
        "a replay writes no second audit row"
    );

    let mirror = Subscription::query()
        .filter(
            "provider_subscription_id",
            sub.provider_subscription_id.as_str(),
        )
        .first()
        .await
        .expect("read the subscription mirror")
        .expect("the subscription mirror exists");
    assert_eq!(mirror.status, "active");
    assert_eq!(mirror.canceled_at, None);
    assert_eq!(mirror.created_at, at(0));
    let items = SubscriptionItem::query()
        .filter("subscription_id", mirror.id)
        .get()
        .await
        .expect("read the item mirrors");
    assert_eq!(items.len(), 2);

    mock.cancel(&sub.provider_subscription_id, false)
        .await
        .expect("mock cancel");
    let (status, body) = post(
        addr,
        &path,
        json!({
            "id": "evt_engines_sub_canceled",
            "type": "subscription.canceled",
            "data": { "object": { "id": sub.provider_subscription_id, "customer": "cus_engines" } }
        }),
    )
    .await;
    assert_eq!(status, 200, "subscription.canceled must persist: {body}");
    let canceled = Subscription::find(mirror.id)
        .await
        .expect("reread the subscription mirror")
        .expect("the subscription mirror still exists");
    assert_eq!(canceled.canceled_at, Some(at(0)));

    let (status, body) = post(
        addr,
        &path,
        json!({
            "id": "evt_engines_paid",
            "type": "payment.succeeded",
            "data": { "object": {
                "id": "txn_engines",
                "customer": "cus_engines",
                "amount": 4999,
                "currency": "USD",
                "paid_at": "2026-05-22T12:00:00+00:00"
            } }
        }),
    )
    .await;
    assert_eq!(status, 200, "payment.succeeded must persist: {body}");
    let paid = Transaction::query()
        .filter("provider_transaction_id", "txn_engines")
        .first()
        .await
        .expect("read the transaction mirror")
        .expect("the transaction mirror exists");
    assert_eq!(
        paid.paid_at,
        Some(Utc.with_ymd_and_hms(2026, 5, 22, 12, 0, 0).unwrap())
    );
    assert_eq!(paid.amount_total_minor, 4999);
}

/// Every mirror model creates, reads and updates its row.
async fn mirrors_create_read_update() {
    let clock = TestClock::travel_to(at(10));

    let customer = Customer::create(attrs! {
        provider: "mock",
        provider_customer_id: "cus_model",
        user_id: "user-1",
        email: "first@example.com",
        provider_metadata: json!({ "tier": "gold" }),
    })
    .await
    .expect("create a customer mirror");
    assert_eq!(customer.created_at, at(10));
    clock.set(at(11));
    let customer = customer
        .update(attrs! { email: "second@example.com" })
        .await
        .expect("update the customer mirror");
    let reread = Customer::find(customer.id)
        .await
        .expect("read the customer mirror")
        .expect("the customer mirror exists");
    assert_eq!(reread.email, "second@example.com");
    assert_eq!(reread.created_at, at(10));
    assert_eq!(reread.updated_at, at(11));
    assert_eq!(reread.provider_metadata, json!({ "tier": "gold" }));

    let method = PaymentMethod::create(attrs! {
        provider: "mock",
        provider_payment_method_id: "pm_model",
        provider_customer_id: "cus_model",
        method_type: "card",
        method_details: json!({ "brand": "visa", "last4": "4242" }),
        is_default: true,
        provider_metadata: json!({}),
    })
    .await
    .expect("create a payment method mirror");
    clock.set(at(12));
    let method = method
        .update(attrs! { is_default: false })
        .await
        .expect("update the payment method mirror");
    let method = PaymentMethod::find(method.id)
        .await
        .expect("read the payment method mirror")
        .expect("the payment method mirror exists");
    assert!(!method.is_default);
    assert_eq!(method.updated_at, at(12));

    let subscription = Subscription::create(attrs! {
        provider: "mock",
        provider_subscription_id: "sub_model",
        provider_customer_id: "cus_model",
        status: "active",
        current_period_start: at(0).to_rfc3339(),
        current_period_end: at(30).to_rfc3339(),
        cancel_at_period_end: false,
        provider_metadata: json!({}),
    })
    .await
    .expect("create a subscription mirror");
    let subscription = subscription
        .update(attrs! { canceled_at: at(20).to_rfc3339() })
        .await
        .expect("update the subscription mirror");
    let subscription = Subscription::find(subscription.id)
        .await
        .expect("read the subscription mirror")
        .expect("the subscription mirror exists");
    assert_eq!(subscription.current_period_start, at(0));
    assert_eq!(subscription.current_period_end, at(30));
    assert_eq!(subscription.canceled_at, Some(at(20)));
    let window = Subscription::query()
        .where_between(
            "current_period_end",
            at(29).to_rfc3339()..=at(31).to_rfc3339(),
        )
        .count()
        .await
        .expect("compare a native column with a moment");
    assert_eq!(window, 1);

    let item = SubscriptionItem::create(attrs! {
        subscription_id: subscription.id,
        provider_item_id: "si_model",
        provider_price_id: "price_model",
        quantity: 2,
        unit_amount_minor: 1500,
        unit_currency: "USD",
        provider_metadata: json!({}),
    })
    .await
    .expect("create a subscription item mirror");
    let item = item
        .update(attrs! { quantity: 3 })
        .await
        .expect("update the subscription item mirror");
    let item = SubscriptionItem::find(item.id)
        .await
        .expect("read the subscription item mirror")
        .expect("the subscription item mirror exists");
    assert_eq!(item.quantity, 3);
    assert_eq!(item.created_at, at(12));

    let transaction = Transaction::create(attrs! {
        provider: "mock",
        provider_transaction_id: "txn_model",
        provider_customer_id: "cus_model",
        amount_total_minor: 1000,
        amount_tax_minor: 0,
        currency: "USD",
        status: "pending",
        provider_metadata: json!({}),
    })
    .await
    .expect("create a transaction mirror");
    assert_eq!(transaction.paid_at, None);
    let transaction = transaction
        .update(attrs! { status: "succeeded", paid_at: at(40).to_rfc3339() })
        .await
        .expect("update the transaction mirror");
    let transaction = Transaction::find(transaction.id)
        .await
        .expect("read the transaction mirror")
        .expect("the transaction mirror exists");
    assert_eq!(transaction.status, "succeeded");
    assert_eq!(transaction.paid_at, Some(at(40)));

    let event = WebhookEvent::create(attrs! {
        provider: "mock",
        provider_event_id: "evt_model",
        provider_event_type: "mock.event",
        payload: json!({ "id": "evt_model" }),
        received_at: at(50).to_rfc3339(),
    })
    .await
    .expect("create a webhook event row");
    let event = event
        .update(attrs! { processed_at: at(51).to_rfc3339() })
        .await
        .expect("update the webhook event row");
    let event = WebhookEvent::find(event.id)
        .await
        .expect("read the webhook event row")
        .expect("the webhook event row exists");
    assert_eq!(event.received_at, at(50));
    assert_eq!(event.processed_at, Some(at(51)));
}

async fn live_payments(env: &str, provider_name: &'static str) {
    let (guard, database) = connect_live(env).await;
    fresh_payments_schema(&database).await;
    {
        let _clock = TestClock::travel_to(at(0));
        webhooks_persist_and_hydrate(&database, provider_name).await;
    }
    mirrors_create_read_update().await;
    drop(guard);
    database.inner().clone().close().await.unwrap();
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL at PG_TEST_URL"]
async fn postgres_payments_mirrors_read_and_write_native_timestamps() {
    live_payments("PG_TEST_URL", "mock-engines-postgres").await;
}

#[tokio::test]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_payments_mirrors_read_and_write_native_timestamps() {
    live_payments("MYSQL_TEST_URL", "mock-engines-mysql").await;
}
