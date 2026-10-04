//! The single-use ceremony store works on the `auth_ceremony_tokens`
//! table its documented migration creates, on every engine.
//!
//! The migration (the dogfood app's, included below) creates `expires_at`
//! and `created_at` with `.timestamp()`: `TIMESTAMP` on MySQL and MariaDB.
//! `ceremony::consume` once read whole rows into an entity with
//! `NaiveDateTime` fields, which the MySQL driver decodes only from
//! `DATETIME`, so every ceremony failed to complete there.
//!
//! ```bash
//! MYSQL_TEST_URL=mysql://... cargo test -p suprnova --test magnetar_integration -- --ignored ceremony_store::mysql_
//! PG_TEST_URL=postgres://... cargo test -p suprnova --test magnetar_integration -- --ignored ceremony_store::postgres_
//! ```

#[path = "../../../app/src/migrations/m20251209_000000_create_auth_ceremony_tokens_table.rs"]
mod ceremony_migration;

use sea_orm::ConnectionTrait;
use sea_orm_migration::{MigrationTrait, SchemaManager};

use suprnova::database::{DatabaseConfig, DbConnection};
use suprnova::magnetar_integration::ceremony;
use suprnova::testing::TestContainer;

async fn issue_consume_and_prune(url: &str) {
    let config = DatabaseConfig::builder()
        .url(url)
        .max_connections(1)
        .min_connections(1)
        .logging(false)
        .build();
    let database = DbConnection::connect(&config)
        .await
        .expect("connect test database");
    database
        .inner()
        .execute_unprepared("DROP TABLE IF EXISTS auth_ceremony_tokens")
        .await
        .expect("drop auth_ceremony_tokens");
    ceremony_migration::Migration
        .up(&SchemaManager::new(database.inner()))
        .await
        .expect("ceremony migration");
    let _container = TestContainer::fake();
    TestContainer::singleton(database.clone());

    let payload = serde_json::json!({ "challenge": "abc" });
    ceremony::issue("ceremony-1", ceremony::kind::OAUTH, &payload, 10)
        .await
        .expect("issue a ceremony");
    let consumed: Option<serde_json::Value> =
        ceremony::consume("ceremony-1", ceremony::kind::OAUTH)
            .await
            .expect("consume the ceremony");
    assert_eq!(consumed, Some(payload.clone()));
    let replayed: Option<serde_json::Value> =
        ceremony::consume("ceremony-1", ceremony::kind::OAUTH)
            .await
            .expect("consume it again");
    assert_eq!(replayed, None, "a ceremony completes once");

    // An expiry past 2038-01-19, the end of MySQL's `TIMESTAMP` range,
    // still issues a usable ceremony.
    ceremony::issue(
        "ceremony-2",
        ceremony::kind::OAUTH,
        &payload,
        20 * 365 * 24 * 60,
    )
    .await
    .expect("issue a ceremony that outlives 2038");
    let long_lived: Option<serde_json::Value> =
        ceremony::consume("ceremony-2", ceremony::kind::OAUTH)
            .await
            .expect("consume the long-lived ceremony");
    assert_eq!(long_lived, Some(payload.clone()));

    ceremony::issue("ceremony-3", ceremony::kind::OAUTH, &payload, -5)
        .await
        .expect("issue an expired ceremony");
    assert_eq!(ceremony::prune_expired().await.expect("prune"), 1);

    database
        .inner()
        .execute_unprepared("DROP TABLE auth_ceremony_tokens")
        .await
        .expect("drop auth_ceremony_tokens");
    database.inner().clone().close().await.unwrap();
}

#[tokio::test]
async fn sqlite_ceremony_store_issues_consumes_and_prunes() {
    issue_consume_and_prune("sqlite::memory:").await;
}

#[tokio::test]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_ceremony_store_issues_consumes_and_prunes() {
    let url = std::env::var("MYSQL_TEST_URL").expect("set MYSQL_TEST_URL");
    issue_consume_and_prune(&url).await;
}

#[tokio::test]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_ceremony_store_issues_consumes_and_prunes() {
    let url = std::env::var("PG_TEST_URL").expect("set PG_TEST_URL");
    issue_consume_and_prune(&url).await;
}
