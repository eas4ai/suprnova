//! The `Role` and `Permission` models read and write the tables the RBAC
//! migration creates, on every engine.
//!
//! `CreateRbacTables` creates `created_at` and `updated_at` as `timestamp
//! with time zone` on Postgres and `TIMESTAMP` on MySQL and MariaDB. The
//! models stored them through the text cast `AsDateTime`, so on those
//! engines reading a role failed to decode and creating one through the
//! model sent text the column refused.
//!
//! ```bash
//! MYSQL_TEST_URL=mysql://... cargo test -p suprnova --test rbac -- --ignored shipped_tables::mysql_
//! PG_TEST_URL=postgres://... cargo test -p suprnova --test rbac -- --ignored shipped_tables::postgres_
//! ```

use sea_orm::ConnectionTrait;
use sea_orm_migration::{MigrationTrait, SchemaManager};

use suprnova::attrs;
use suprnova::database::{DatabaseConfig, DbConnection};
use suprnova::eloquent::Model;
use suprnova::rbac::entity::{Permission, Role};
use suprnova::rbac::migrations::CreateRbacTables;
use suprnova::rbac::{create_role, give_permission_to_role};
use suprnova::testing::TestContainer;

const TABLES: [&str; 5] = [
    "model_permissions",
    "model_roles",
    "role_permissions",
    "permissions",
    "roles",
];

async fn drop_tables(database: &DbConnection) {
    for table in TABLES {
        database
            .inner()
            .execute_unprepared(&format!("DROP TABLE IF EXISTS {table}"))
            .await
            .expect("drop rbac table");
    }
}

async fn roles_and_permissions_round_trip(url: &str) {
    let config = DatabaseConfig::builder()
        .url(url)
        .max_connections(1)
        .min_connections(1)
        .logging(false)
        .build();
    let database = DbConnection::connect(&config)
        .await
        .expect("connect test database");
    drop_tables(&database).await;
    CreateRbacTables
        .up(&SchemaManager::new(database.inner()))
        .await
        .expect("rbac migration");
    let _container = TestContainer::fake();
    TestContainer::singleton(database.clone());

    let before = chrono::Utc::now() - chrono::Duration::seconds(5);
    create_role("editor").await.expect("create a role");
    give_permission_to_role("editor", "articles.publish")
        .await
        .expect("grant a permission");

    let role = Role::query().filter("name", "editor").first().await;
    let permission = Permission::query()
        .filter("name", "articles.publish")
        .first()
        .await;
    let created = Permission::create(attrs! {
        name: "articles.delete".to_string(),
        guard_name: "web".to_string(),
    })
    .await;
    // One assertion over every call, so a failure shows each of them.
    assert_eq!(
        format!(
            "{:?} {:?} {:?}",
            role.as_ref()
                .map(|row| row.as_ref().map(|role| role.name.clone())),
            permission
                .as_ref()
                .map(|row| row.as_ref().map(|permission| permission.name.clone())),
            created.as_ref().map(|permission| permission.name.clone()),
        ),
        r#"Ok(Some("editor")) Ok(Some("articles.publish")) Ok("articles.delete")"#,
        "read a role, read a permission, create a permission"
    );

    let role = role.expect("role").expect("role row");
    assert!(
        role.created_at > before && role.updated_at >= role.created_at,
        "the stored times read back as UTC times: {:?} {:?}",
        role.created_at,
        role.updated_at
    );
    let created = created.expect("created permission");
    let reread = Permission::query()
        .filter("name", "articles.delete")
        .first()
        .await
        .expect("read the created permission")
        .expect("permission row");
    // Within a second: a MySQL `TIMESTAMP` keeps whole seconds.
    assert!(
        (reread.created_at - created.created_at).num_seconds().abs() <= 1,
        "the model's own timestamp round-trips: wrote {:?}, read {:?}",
        created.created_at,
        reread.created_at
    );

    drop_tables(&database).await;
    database.inner().clone().close().await.unwrap();
}

#[tokio::test]
async fn sqlite_rbac_models_read_and_write_the_shipped_tables() {
    roles_and_permissions_round_trip("sqlite::memory:").await;
}

#[tokio::test]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_rbac_models_read_and_write_the_shipped_tables() {
    let url = std::env::var("MYSQL_TEST_URL").expect("set MYSQL_TEST_URL");
    roles_and_permissions_round_trip(&url).await;
}

#[tokio::test]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_rbac_models_read_and_write_the_shipped_tables() {
    let url = std::env::var("PG_TEST_URL").expect("set PG_TEST_URL");
    roles_and_permissions_round_trip(&url).await;
}
