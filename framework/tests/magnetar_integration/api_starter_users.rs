//! The `--api` starter's `app_users` table reads and writes through its own
//! `User` model and through Magnetar, which shares the table, on every
//! engine.
//!
//! The starter's migration and model come from the CLI's templates, so the
//! test cannot drift from what `suprnova new --api` writes. Both the model
//! and Magnetar's `app_users` entity hold the time columns as
//! `DateTime<Utc>`, which Postgres decodes only from `timestamp with time
//! zone`. The migration created `.timestamp()`, `timestamp` there, so on
//! Postgres neither could read a user; it now creates `timestamptz` on
//! Postgres and DATETIME on MySQL and MariaDB.
//!
//! ```bash
//! MYSQL_TEST_URL=mysql://... cargo test -p suprnova --test magnetar_integration -- --ignored api_starter_users::mysql_
//! PG_TEST_URL=postgres://... cargo test -p suprnova --test magnetar_integration -- --ignored api_starter_users::postgres_
//! ```

// `#[rustfmt::skip]`: the templates are scaffold output, not workspace
// source, so `cargo fmt` must not rewrite them.
#[rustfmt::skip]
#[path = "../../../suprnova-cli/src/templates/files/api/src/migrations/create_users_table.rs.tpl"]
mod api_users_migration;
#[rustfmt::skip]
#[path = "../../../suprnova-cli/src/templates/files/api/src/models/user.rs.tpl"]
mod api_user;

use sea_orm::{ConnectionTrait, DatabaseBackend, EntityTrait, Statement};
use sea_orm_migration::{MigrationTrait, SchemaManager};

use suprnova::database::{DatabaseConfig, DbConnection};
use suprnova::testing::TestContainer;

/// The type the engine reports for `app_users.created_at`.
async fn created_at_type(database: &DbConnection) -> String {
    let backend = database.inner().get_database_backend();
    let sql = match backend {
        DatabaseBackend::MySql => {
            "SELECT DATA_TYPE AS kind FROM information_schema.COLUMNS \
             WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = 'app_users' \
             AND COLUMN_NAME = 'created_at'"
        }
        _ => {
            "SELECT data_type AS kind FROM information_schema.columns \
             WHERE table_name = 'app_users' AND column_name = 'created_at'"
        }
    };
    let row = database
        .inner()
        .query_one_raw(Statement::from_string(backend, sql))
        .await
        .expect("read column type")
        .expect("created_at exists");
    let kind: String = row.try_get("", "kind").expect("column type");
    kind.to_ascii_lowercase()
}

/// `expected_type` is what the engine reports for the time columns, or
/// `None` on SQLite, which has no such catalog.
async fn create_read_and_share(url: &str, expected_type: Option<&str>) {
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
        .execute_unprepared("DROP TABLE IF EXISTS app_users")
        .await
        .expect("drop app_users");
    api_users_migration::Migration
        .up(&SchemaManager::new(database.inner()))
        .await
        .expect("api starter users migration");
    if let Some(expected) = expected_type {
        assert_eq!(created_at_type(&database).await, expected);
    }
    let _container = TestContainer::fake();
    TestContainer::singleton(database.clone());

    let created = api_user::User::create("api@example.test").await;
    let by_email = api_user::Model::find_by_email("api@example.test").await;
    let all = api_user::Model::all().await;
    let shared = magnetar::default_schema::users::Entity::find()
        .all(database.inner())
        .await;
    // One assertion over every call, so a failure shows each of them.
    assert_eq!(
        format!(
            "{:?} {:?} {:?} {:?}",
            created.as_ref().map(|user| user.email.clone()),
            by_email
                .as_ref()
                .map(|user| user.as_ref().map(|user| user.email.clone())),
            all.as_ref().map(Vec::len),
            shared.as_ref().map(Vec::len),
        ),
        r#"Ok("api@example.test") Ok(Some("api@example.test")) Ok(1) Ok(1)"#,
        "create, find by email, list, Magnetar's read of app_users"
    );
    let id = created.expect("created user").id;
    let found = api_user::Model::find_by_id(id)
        .await
        .expect("find by id")
        .expect("the user");
    let age = chrono::Utc::now() - found.created_at;
    assert!(
        age.num_seconds().abs() <= 5,
        "created_at reads back as the UTC time written: {:?}",
        found.created_at
    );

    database
        .inner()
        .execute_unprepared("DROP TABLE app_users")
        .await
        .expect("drop app_users");
    database.inner().clone().close().await.unwrap();
}

#[tokio::test]
async fn sqlite_api_starter_users_are_shared_with_magnetar() {
    create_read_and_share("sqlite::memory:", None).await;
}

#[tokio::test]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_api_starter_users_are_shared_with_magnetar() {
    let url = std::env::var("MYSQL_TEST_URL").expect("set MYSQL_TEST_URL");
    create_read_and_share(&url, Some("datetime")).await;
}

#[tokio::test]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_api_starter_users_are_shared_with_magnetar() {
    let url = std::env::var("PG_TEST_URL").expect("set PG_TEST_URL");
    create_read_and_share(&url, Some("timestamp with time zone")).await;
}
