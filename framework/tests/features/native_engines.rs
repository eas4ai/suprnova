//! The `features` table on real Postgres, MySQL and MariaDB.
//!
//! `CreateFeaturesTable` creates native `timestamp with time zone` columns
//! (`TIMESTAMP` on MySQL). The feature model must read and write them as
//! native date-times: before, `set_flag` and the admin upsert bound text,
//! which Postgres refuses for such a column, and every read failed to decode
//! as soon as a row existed. The SQLite suite cannot see that.
//!
//! ```text
//! PG_TEST_URL=postgres://... cargo test -p suprnova --test features -- \
//!   --ignored --test-threads=1 native_engines::postgres_
//! MYSQL_TEST_URL=mysql://... cargo test -p suprnova --test features -- \
//!   --ignored --test-threads=1 native_engines::mysql_
//! ```

use chrono::{DateTime, TimeZone, Utc};
use sea_orm::ConnectionTrait;
use sea_orm_migration::{MigrationTrait, SchemaManager};
use suprnova::features::entity::Feature;
use suprnova::features::migrations::CreateFeaturesTable;
use suprnova::features::{Context, DatabaseEvaluator, Evaluator, admin};
use suprnova::testing::{TestClock, TestContainer, TestContainerGuard};
use suprnova::{DatabaseConfig, DbConnection, Model, attrs};

/// Whole seconds, so a MySQL `TIMESTAMP` of precision 0 holds each moment
/// exactly.
fn at(second: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2031, 3, 14, 9, 26, second).unwrap()
}

async fn connect_live(env: &str) -> (TestContainerGuard, DbConnection) {
    let url = std::env::var(env).expect("explicit disposable database URL required");
    let guard = TestContainer::fake();
    let config = DatabaseConfig::builder()
        .url(url)
        .max_connections(2)
        .min_connections(1)
        .logging(false)
        .build();
    let database = DbConnection::connect(&config)
        .await
        .expect("connect test database");
    TestContainer::singleton(database.clone());
    (guard, database)
}

async fn live_features(env: &str) {
    let (guard, database) = connect_live(env).await;
    database
        .inner()
        .execute_unprepared("DROP TABLE IF EXISTS features")
        .await
        .expect("drop a leftover features table");
    CreateFeaturesTable
        .up(&SchemaManager::new(database.inner()))
        .await
        .expect("run the features migration");
    let clock = TestClock::travel_to(at(0));

    let evaluator = DatabaseEvaluator::new()
        .await
        .expect("boot the evaluator over an empty table");
    evaluator
        .set_flag("checkout.v2", "", true)
        .await
        .expect("set_flag inserts the row");
    clock.set(at(1));
    evaluator
        .set_flag("checkout.v2", "", false)
        .await
        .expect("set_flag updates the row");
    evaluator
        .reload()
        .await
        .expect("reload decodes the populated table");
    assert_eq!(
        evaluator.is_enabled("checkout.v2", &Context::root()),
        Some(false)
    );
    DatabaseEvaluator::new()
        .await
        .expect("a fresh evaluator boots over the populated table");

    clock.set(at(2));
    let row = admin::upsert(
        "reports",
        "user:42",
        true,
        Some("beta reports".into()),
        Some("admin-1".into()),
    )
    .await
    .expect("admin upsert");
    assert_eq!(row.created_at, at(2));
    assert_eq!(row.updated_at, at(2));
    let listed = admin::list().await.expect("admin list");
    assert_eq!(listed.len(), 2);
    let flag = listed
        .iter()
        .find(|row| row.name == "checkout.v2")
        .expect("the set_flag row is listed");
    assert_eq!(flag.created_at, at(0));
    assert_eq!(flag.updated_at, at(1));

    clock.set(at(3));
    let made = Feature::create(attrs! {
        name: "search",
        scope_key: "",
        enabled: true,
        description: "full-text search",
    })
    .await
    .expect("create a flag through the model");
    assert_eq!(made.created_at, at(3));
    clock.set(at(4));
    let made = made
        .update(attrs! { enabled: false })
        .await
        .expect("update a flag through the model");
    let reread = Feature::find(made.id)
        .await
        .expect("read the flag through the model")
        .expect("the flag exists");
    assert!(!reread.enabled);
    assert_eq!(reread.created_at, at(3));
    assert_eq!(reread.updated_at, at(4));

    drop(clock);
    drop(guard);
    database.inner().clone().close().await.unwrap();
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL at PG_TEST_URL"]
async fn postgres_features_read_and_write_native_timestamps() {
    live_features("PG_TEST_URL").await;
}

#[tokio::test]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_features_read_and_write_native_timestamps() {
    live_features("MYSQL_TEST_URL").await;
}
