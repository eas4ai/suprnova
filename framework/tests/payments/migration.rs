//! Phase 12 T5 - payments DB mirror migration integration test.
//!
//! Boots a fresh in-memory SQLite via `TestDatabase::fresh::<PaymentsTestMigrator>()`
//! and confirms all six payments tables exist and are queryable.

use sea_orm::ConnectionTrait;
use sea_orm_migration::MigratorTrait;
use suprnova::payments::migrations::migrations as payments_migrations;
use suprnova::testing::TestDatabase;

struct PaymentsTestMigrator;

#[async_trait::async_trait]
impl MigratorTrait for PaymentsTestMigrator {
    fn migrations() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
        payments_migrations()
    }
}

#[tokio::test]
async fn payments_migration_up_creates_all_six_tables() {
    let db = TestDatabase::fresh::<PaymentsTestMigrator>().await.unwrap();
    let conn = db.conn();

    for table in [
        "payments_customers",
        "payments_payment_methods",
        "payments_subscriptions",
        "payments_subscription_items",
        "payments_transactions",
        "payments_webhook_events",
    ] {
        let stmt = sea_orm::Statement::from_string(
            conn.get_database_backend(),
            format!("SELECT COUNT(*) FROM {table}"),
        );
        let res = conn.query_one_raw(stmt).await;
        assert!(res.is_ok(), "table {table} should exist and be queryable");
    }
}

/// Every index the payments migration creates, by table.
const PAYMENTS_INDEXES: [(&str, &str); 15] = [
    (
        "payments_customers",
        "uniq_payments_customers_provider_customer_id",
    ),
    ("payments_customers", "idx_payments_customers_user_id"),
    (
        "payments_payment_methods",
        "uniq_payments_payment_methods_provider_pm_id",
    ),
    (
        "payments_payment_methods",
        "idx_payments_payment_methods_customer_id",
    ),
    (
        "payments_subscriptions",
        "uniq_payments_subscriptions_provider_sub_id",
    ),
    (
        "payments_subscriptions",
        "idx_payments_subscriptions_customer_id",
    ),
    (
        "payments_subscriptions",
        "idx_payments_subscriptions_status",
    ),
    (
        "payments_subscription_items",
        "idx_payments_subscription_items_sub_id",
    ),
    (
        "payments_subscription_items",
        "uniq_payments_subscription_items_provider_item",
    ),
    (
        "payments_transactions",
        "uniq_payments_transactions_provider_tx_id",
    ),
    (
        "payments_transactions",
        "idx_payments_transactions_customer_id",
    ),
    (
        "payments_transactions",
        "idx_payments_transactions_subscription_id",
    ),
    ("payments_transactions", "idx_payments_transactions_status"),
    (
        "payments_webhook_events",
        "uniq_payments_webhook_events_provider_event_id",
    ),
    (
        "payments_webhook_events",
        "idx_payments_webhook_events_event_type",
    ),
];

/// An app whose database already holds the payments tables and their
/// indexes (created by an earlier run, or by hand) runs `up` again. It
/// must succeed and leave every index in place. Before the index guard
/// the second run failed on the first existing index, on every backend.
#[tokio::test]
async fn payments_migration_up_over_existing_tables_and_indexes_succeeds() {
    use sea_orm_migration::{MigrationTrait, SchemaManager};
    use suprnova::payments::migrations::CreatePaymentsTables;

    let db = sea_orm::Database::connect("sqlite::memory:").await.unwrap();
    let manager = SchemaManager::new(&db);
    CreatePaymentsTables
        .up(&manager)
        .await
        .expect("create the tables and their indexes");

    CreatePaymentsTables
        .up(&manager)
        .await
        .expect("up over existing tables and indexes");

    for (table, index) in PAYMENTS_INDEXES {
        assert!(
            manager.has_index(table, index).await.unwrap(),
            "{table}.{index} is still there"
        );
    }
}
