//! PostgreSQL regressions for Eloquent soft-delete placeholders and timestamp casts.
//!
//! Run explicitly against a disposable database:
//!
//! ```text
//! PG_TEST_URL=postgres://... \
//!   cargo test -p suprnova --test eloquent_soft_delete_postgres -- \
//!   --ignored --test-threads=1
//! ```

use std::time::Duration;

use chrono::{DateTime, FixedOffset, Utc};
use sea_orm::{ConnectOptions, ConnectionTrait, Database, Statement};
use serial_test::serial;
use suprnova::testing::TestContainer;
use suprnova::{Cast, DbConnection, FrameworkError, Model, Touchable, attrs, model};

pub struct NativeUtcDateTime;

impl Cast for NativeUtcDateTime {
    type Runtime = DateTime<Utc>;
    type Storage = DateTime<FixedOffset>;

    fn to_storage(value: &Self::Runtime) -> Result<Self::Storage, FrameworkError> {
        Ok(value.fixed_offset())
    }

    fn from_storage(stored: &Self::Storage) -> Result<Self::Runtime, FrameworkError> {
        Ok(stored.with_timezone(&Utc))
    }
}

pub struct OptionalNativeUtcDateTime;

impl Cast for OptionalNativeUtcDateTime {
    type Runtime = Option<DateTime<Utc>>;
    type Storage = Option<DateTime<FixedOffset>>;

    fn to_storage(value: &Self::Runtime) -> Result<Self::Storage, FrameworkError> {
        Ok(value.as_ref().map(DateTime::fixed_offset))
    }

    fn from_storage(stored: &Self::Storage) -> Result<Self::Runtime, FrameworkError> {
        Ok(stored.as_ref().map(|value| value.with_timezone(&Utc)))
    }
}

#[model(
    table = "suprnova_pg_soft_delete_text",
    soft_deletes,
    timestamps = false,
    fillable = ["name"]
)]
pub struct PgTextSoftDelete {
    pub id: i64,
    pub name: String,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[model(
    table = "suprnova_pg_native_timestamps",
    soft_deletes,
    fillable = ["name"],
    casts = {
        created_at = NativeUtcDateTime,
        updated_at = NativeUtcDateTime,
        deleted_at = OptionalNativeUtcDateTime
    }
)]
pub struct PgNativeTimestamps {
    pub id: i64,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

async fn connect_postgres() -> sea_orm::DatabaseConnection {
    let url = std::env::var("PG_TEST_URL").expect("set PG_TEST_URL to a disposable Postgres");
    let mut options = ConnectOptions::new(url);
    options
        .max_connections(2)
        .min_connections(0)
        .connect_timeout(Duration::from_secs(2))
        .acquire_timeout(Duration::from_secs(2));
    Database::connect(options)
        .await
        .expect("Postgres test database must be reachable")
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_soft_delete_and_native_timestamp_writes_use_declared_storage() {
    let conn = connect_postgres().await;
    let backend = conn.get_database_backend();

    for sql in [
        "DROP TABLE IF EXISTS suprnova_pg_native_timestamps",
        "DROP TABLE IF EXISTS suprnova_pg_soft_delete_text",
        "CREATE TABLE suprnova_pg_soft_delete_text (\
            id BIGSERIAL PRIMARY KEY, \
            name TEXT NOT NULL, \
            deleted_at TEXT NULL\
         )",
        "CREATE TABLE suprnova_pg_native_timestamps (\
            id BIGSERIAL PRIMARY KEY, \
            name TEXT NOT NULL, \
            created_at TIMESTAMPTZ NOT NULL, \
            updated_at TIMESTAMPTZ NOT NULL, \
            deleted_at TIMESTAMPTZ NULL\
         )",
    ] {
        conn.execute_raw(Statement::from_string(backend, sql.to_owned()))
            .await
            .unwrap();
    }

    let _guard = TestContainer::fake();
    TestContainer::singleton(DbConnection::from_raw(conn.clone()));

    let text = PgTextSoftDelete::create(attrs! { name: "text-backed" })
        .await
        .unwrap();
    let text_id = text.id;
    text.delete().await.unwrap();
    let text = PgTextSoftDelete::with_trashed()
        .filter("id", text_id)
        .first()
        .await
        .unwrap()
        .unwrap();
    assert!(text.deleted_at.is_some());
    text.restore().await.unwrap();
    assert!(
        PgTextSoftDelete::find(text_id)
            .await
            .unwrap()
            .unwrap()
            .deleted_at
            .is_none()
    );

    let native = PgNativeTimestamps::create(attrs! { name: "native" })
        .await
        .unwrap();
    let native_id = native.id;
    let native = native.update(attrs! { name: "updated" }).await.unwrap();
    let mut native_for_save = native.clone();
    native_for_save.name = "saved".to_owned();
    native_for_save.save().await.unwrap();
    native_for_save.touch().await.unwrap();
    native_for_save.delete().await.unwrap();

    let row = conn
        .query_one_raw(Statement::from_sql_and_values(
            backend,
            "SELECT created_at, updated_at, deleted_at \
             FROM suprnova_pg_native_timestamps WHERE id = $1",
            [native_id.into()],
        ))
        .await
        .unwrap()
        .unwrap();
    let _: DateTime<FixedOffset> = row.try_get("", "created_at").unwrap();
    let _: DateTime<FixedOffset> = row.try_get("", "updated_at").unwrap();
    let deleted_at: Option<DateTime<FixedOffset>> = row.try_get("", "deleted_at").unwrap();
    assert!(deleted_at.is_some());

    let native = PgNativeTimestamps::with_trashed()
        .filter("id", native_id)
        .first()
        .await
        .unwrap()
        .unwrap();
    native.restore().await.unwrap();
    assert!(
        PgNativeTimestamps::find(native_id)
            .await
            .unwrap()
            .unwrap()
            .deleted_at
            .is_none()
    );

    for sql in [
        "DROP TABLE suprnova_pg_native_timestamps",
        "DROP TABLE suprnova_pg_soft_delete_text",
    ] {
        conn.execute_raw(Statement::from_string(backend, sql.to_owned()))
            .await
            .unwrap();
    }
}
