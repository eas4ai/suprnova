//! The RBAC `roles` and `permissions` models on real Postgres, MySQL and
//! MariaDB.
//!
//! `CreateRbacTables` creates native `timestamp with time zone` columns
//! (`TIMESTAMP` on MySQL). The helpers in `suprnova::rbac` write through raw
//! SQL and leave the timestamps to the column defaults, so they never touched
//! the columns; the public `Role` and `Permission` models did, and stored text,
//! which Postgres refuses on bind and every driver refuses on decode.
//!
//! ```text
//! PG_TEST_URL=postgres://... cargo test -p suprnova --test rbac -- \
//!   --ignored --test-threads=1 native_engines::postgres_
//! MYSQL_TEST_URL=mysql://... cargo test -p suprnova --test rbac -- \
//!   --ignored --test-threads=1 native_engines::mysql_
//! ```

use chrono::{DateTime, TimeZone, Utc};
use suprnova::rbac::entity::{Permission, Role};
use suprnova::rbac::{create_permission, create_role};
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

async fn live_rbac_models(env: &str) {
    let (guard, database) = connect_live(env).await;
    super::postgres::fresh_rbac_schema(&database).await;

    // A row the helper wrote, with the column defaults' timestamps, reads
    // back through the model.
    let helper_role = create_role("editor").await.expect("create_role");
    let editor = Role::find(helper_role)
        .await
        .expect("read a helper-created role through the model")
        .expect("the role exists");
    assert_eq!(editor.name, "editor");
    let helper_permission = create_permission("articles.publish")
        .await
        .expect("create_permission");
    assert!(
        Permission::find(helper_permission)
            .await
            .expect("read a helper-created permission through the model")
            .is_some()
    );

    let clock = TestClock::travel_to(at(0));
    let auditor = Role::create(attrs! { name: "auditor", guard_name: "web" })
        .await
        .expect("create a role through the model");
    assert_eq!(auditor.created_at, at(0));
    clock.set(at(1));
    let auditor = auditor
        .update(attrs! { display_name: "Auditor" })
        .await
        .expect("update a role through the model");
    let auditor = Role::find(auditor.id)
        .await
        .expect("reread the role")
        .expect("the role exists");
    assert_eq!(auditor.display_name.as_deref(), Some("Auditor"));
    assert_eq!(auditor.created_at, at(0));
    assert_eq!(auditor.updated_at, at(1));

    clock.set(at(2));
    let delete = Permission::create(attrs! { name: "articles.delete", guard_name: "web" })
        .await
        .expect("create a permission through the model");
    clock.set(at(3));
    let delete = delete
        .update(attrs! { display_name: "Delete articles" })
        .await
        .expect("update a permission through the model");
    let delete = Permission::find(delete.id)
        .await
        .expect("reread the permission")
        .expect("the permission exists");
    assert_eq!(delete.created_at, at(2));
    assert_eq!(delete.updated_at, at(3));
    assert_eq!(Role::query().get().await.expect("list every role").len(), 2);

    drop(clock);
    drop(guard);
    database.inner().clone().close().await.unwrap();
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL at PG_TEST_URL"]
async fn postgres_rbac_models_read_and_write_native_timestamps() {
    live_rbac_models("PG_TEST_URL").await;
}

#[tokio::test]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_rbac_models_read_and_write_native_timestamps() {
    live_rbac_models("MYSQL_TEST_URL").await;
}

/// IDENTITY-021 on a real engine: every create and grant converges when
/// another connection inserts the same row between the helper's check and
/// its insert (see `concurrent_grants`).
async fn live_grants_converge(env: &str, probe: super::concurrent_grants::LockProbe) {
    let url = std::env::var(env).expect("explicit disposable database URL required");
    super::concurrent_grants::race_against_a_held_row(&url, probe, || async {
        let (guard, database) = connect_live(env).await;
        super::postgres::fresh_rbac_schema(&database).await;
        (guard, database)
    })
    .await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL at PG_TEST_URL"]
async fn postgres_every_grant_converges_when_a_concurrent_request_inserts_first() {
    live_grants_converge("PG_TEST_URL", super::concurrent_grants::LockProbe::Postgres).await;
}

#[tokio::test]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_every_grant_converges_when_a_concurrent_request_inserts_first() {
    live_grants_converge("MYSQL_TEST_URL", super::concurrent_grants::LockProbe::MySql).await;
}
