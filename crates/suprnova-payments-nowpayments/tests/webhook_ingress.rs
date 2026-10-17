//! Real framework ingress: authentication, audit deduplication and DB recovery.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use hmac::{Hmac, KeyInit, Mac};
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use sea_orm::{ConnectionTrait, DbBackend, EntityTrait, Statement};
use sea_orm_migration::MigratorTrait;
use serde_json::{Value, json};
use sha2::Sha512;
use suprnova::payments::{
    PaymentProviderRegistry,
    entities::{transaction, webhook_event},
    webhook_routes,
};
use suprnova::testing::TestDatabase;
use suprnova::{MiddlewareRegistry, handle_request};
use suprnova_payments_nowpayments::{NowPaymentsEnvironment, NowPaymentsProvider};

struct Migrator;
#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
        suprnova::payments::migrations::migrations()
    }
}

const SECRET: &str = "ingress-test-secret";

fn body(status: &str) -> Value {
    json!({"invoice_id":"4522625843", "order_id":"order-1001", "payment_id":"5745459419", "payment_status":status, "price_amount":"12.34", "price_currency":"usd"})
}

fn signature(value: &Value) -> String {
    // These flat fixtures contain only strings. Explicit sorted key insertion
    // keeps this independent of the adapter's recursive canonicalizer.
    let object = value.as_object().unwrap();
    let mut keys: Vec<_> = object.keys().collect();
    keys.sort();
    let mut fields = Vec::new();
    for key in keys {
        fields.push(format!(
            "{}:{}",
            serde_json::to_string(key).unwrap(),
            serde_json::to_string(&object[key]).unwrap()
        ));
    }
    let canonical = format!("{{{}}}", fields.join(","));
    let mut mac = Hmac::<Sha512>::new_from_slice(SECRET.as_bytes()).unwrap();
    mac.update(canonical.as_bytes());
    mac.finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[tokio::test]
async fn real_ingress_rejects_forgery_deduplicates_reordered_events_and_recovers_failure() {
    let db = TestDatabase::fresh::<Migrator>().await.unwrap();
    let conn = Arc::new(db.conn().clone());
    let provider = NowPaymentsProvider::new(
        "test-key",
        SECRET,
        "https://merchant.example/webhooks/payments/nowpayments",
        NowPaymentsEnvironment::Sandbox,
    )
    .unwrap();
    PaymentProviderRegistry::bind("nowpayments", Arc::new(provider));
    let router = Arc::new(webhook_routes(conn.clone()));
    let middleware = Arc::new(MiddlewareRegistry::new());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "http://{}/webhooks/payments/nowpayments",
        listener.local_addr().unwrap()
    );
    let server = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let router = router.clone();
            let middleware = middleware.clone();
            tokio::spawn(async move {
                let service = service_fn(move |request| {
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
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let first = body("finished");
    let forged = client
        .post(&url)
        .header("x-nowpayments-sig", "0".repeat(128))
        .json(&first)
        .send()
        .await
        .unwrap();
    assert_eq!(forged.status().as_u16(), 401);
    assert!(
        webhook_event::Entity::find()
            .all(&*conn)
            .await
            .unwrap()
            .is_empty()
    );

    // Fail after receipt persistence, when the route marks hydration complete.
    // The failed receipt remains retryable; no fake customer is created.
    conn.execute_raw(Statement::from_string(DbBackend::Sqlite,
        "CREATE TRIGGER fail_nowpayments_processed BEFORE UPDATE OF processed_at ON payments_webhook_events WHEN NEW.processed_at IS NOT NULL BEGIN SELECT RAISE(ABORT, 'injected payment receipt failure'); END".to_owned())).await.unwrap();
    let failed = client
        .post(&url)
        .header("x-nowpayments-sig", signature(&first))
        .json(&first)
        .send()
        .await
        .unwrap();
    assert_eq!(failed.status().as_u16(), 503);
    let rows = webhook_event::Entity::find().all(&*conn).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].processed_at.is_none());
    assert!(rows[0].process_error.is_some());
    conn.execute_raw(Statement::from_string(
        DbBackend::Sqlite,
        "DROP TRIGGER fail_nowpayments_processed".to_owned(),
    ))
    .await
    .unwrap();

    for (status, response_body) in [
        ("finished", "ok"),
        ("finished", "duplicate"),
        ("waiting", "ok"),
        ("partially_paid", "ok"),
        ("refunded", "ok"),
        ("finished", "duplicate"),
    ] {
        let value = body(status);
        let response = client
            .post(&url)
            .header("x-nowpayments-sig", signature(&value))
            .json(&value)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 200, "{status}");
        assert_eq!(response.text().await.unwrap(), response_body, "{status}");
    }
    let rows = webhook_event::Entity::find().all(&*conn).await.unwrap();
    assert_eq!(rows.len(), 4);
    assert!(
        rows.iter()
            .all(|row| row.processed_at.is_some() && row.process_error.is_none())
    );
    assert_eq!(
        rows.iter()
            .filter(|row| row.provider_event_type == "payment.finished")
            .count(),
        1
    );
    assert!(
        transaction::Entity::find()
            .all(&*conn)
            .await
            .unwrap()
            .is_empty(),
        "customerless invoices must not fabricate transaction mirrors"
    );
    server.abort();
}
