//! The feature-flag store reads and writes the `features` table its own
//! migration creates, on every engine.
//!
//! `CreateFeaturesTable` creates `created_at` and `updated_at` as
//! `timestamp with time zone` on Postgres and `TIMESTAMP` on MySQL and
//! MariaDB. The `Feature` model stored them through the text cast
//! `AsDateTime`, so on those engines every write sent text the column
//! refused and every read asked for text the driver would not decode.
//!
//! ```bash
//! MYSQL_TEST_URL=mysql://... cargo test -p suprnova --test features -- --ignored shipped_table::mysql_
//! PG_TEST_URL=postgres://... cargo test -p suprnova --test features -- --ignored shipped_table::postgres_
//! ```

use sea_orm::ConnectionTrait;
use sea_orm_migration::{MigrationTrait, SchemaManager};

use suprnova::database::{DatabaseConfig, DbConnection};
use suprnova::features::DatabaseEvaluator;
use suprnova::features::admin;
use suprnova::features::migrations::CreateFeaturesTable;
use suprnova::testing::TestContainer;

async fn upsert_read_and_reload(url: &str) {
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
        .execute_unprepared("DROP TABLE IF EXISTS features")
        .await
        .expect("drop features");
    CreateFeaturesTable
        .up(&SchemaManager::new(database.inner()))
        .await
        .expect("features migration");
    let _container = TestContainer::fake();
    TestContainer::singleton(database.clone());

    let before = chrono::Utc::now() - chrono::Duration::seconds(5);
    let created = admin::upsert("checkout.v2", "", true, Some("new checkout".into()), None).await;
    let updated = admin::upsert("checkout.v2", "", false, None, Some("7".into())).await;
    let listed = admin::list().await;
    let evaluator = DatabaseEvaluator::new().await;
    // One assertion over every call, so a failure shows each of them.
    assert_eq!(
        format!(
            "{:?} {:?} {:?} {:?}",
            created.as_ref().map(|row| row.enabled),
            updated.as_ref().map(|row| row.enabled),
            listed.as_ref().map(Vec::len),
            evaluator.as_ref().map(|_| "loaded"),
        ),
        r#"Ok(true) Ok(false) Ok(1) Ok("loaded")"#,
        "upsert, upsert again, list, evaluator load"
    );

    let row = admin::get("checkout.v2", "")
        .await
        .expect("get")
        .expect("row");
    assert!(
        row.created_at > before && row.updated_at >= row.created_at,
        "the stored times read back as the UTC times written: {row:?}"
    );

    let evaluator = evaluator.expect("evaluator");
    evaluator
        .set_flag("search.v3", "", true)
        .await
        .expect("set a flag");
    evaluator.reload().await.expect("reload");
    assert_eq!(admin::list().await.expect("list").len(), 2);

    database
        .inner()
        .execute_unprepared("DROP TABLE features")
        .await
        .expect("drop features");
    database.inner().clone().close().await.unwrap();
}

#[tokio::test]
async fn sqlite_feature_store_reads_and_writes_its_shipped_table() {
    upsert_read_and_reload("sqlite::memory:").await;
}

#[tokio::test]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_feature_store_reads_and_writes_its_shipped_table() {
    let url = std::env::var("MYSQL_TEST_URL").expect("set MYSQL_TEST_URL");
    upsert_read_and_reload(&url).await;
}

#[tokio::test]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_feature_store_reads_and_writes_its_shipped_table() {
    let url = std::env::var("PG_TEST_URL").expect("set PG_TEST_URL");
    upsert_read_and_reload(&url).await;
}
