//! Pivot timestamps that `attach` writes, read back through the pivot model,
//! on real Postgres, MySQL and MariaDB.
//!
//! `attach` stamps `created_at` / `updated_at` itself rather than through the
//! pivot model. These tests pin that the stamp reads back as the same moment
//! through each timestamp cast a pivot model can declare: the default text
//! cast over a `VARCHAR` column, `AsNativeDateTime` over a zone-aware native
//! column and `AsNaiveDateTime` over a native column without a zone.
//!
//! ```text
//! PG_TEST_URL=postgres://... cargo test -p suprnova --test eloquent -- \
//!   --ignored --test-threads=1 pivot_timestamps_engines::postgres_
//! MYSQL_TEST_URL=mysql://... cargo test -p suprnova --test eloquent -- \
//!   --ignored --test-threads=1 pivot_timestamps_engines::mysql_
//! ```

use chrono::{DateTime, TimeZone, Utc};
use suprnova::database::{DatabaseConfig, DbConnection};
use suprnova::testing::{TestClock, TestContainer, TestContainerGuard};
use suprnova::{Model, attrs, model};

#[model(table = "ptz_users", relations = {
    text_roles: BelongsToMany<PtzRole, PtzTextPivot> { with_timestamps },
    native_roles: BelongsToMany<PtzRole, PtzNativePivot> { with_timestamps },
    naive_roles: BelongsToMany<PtzRole, PtzNaivePivot> { with_timestamps },
})]
pub struct PtzUser {
    pub id: i64,
    pub name: String,
}

#[model(table = "ptz_roles")]
pub struct PtzRole {
    pub id: i64,
    pub name: String,
}

#[model(table = "ptz_text_pivot", primary_key = "id")]
pub struct PtzTextPivot {
    pub id: i64,
    pub ptz_user_id: i64,
    pub ptz_role_id: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[model(
    table = "ptz_native_pivot",
    primary_key = "id",
    casts = {
        created_at = suprnova::AsNativeDateTime,
        updated_at = suprnova::AsNativeDateTime,
    },
)]
pub struct PtzNativePivot {
    pub id: i64,
    pub ptz_user_id: i64,
    pub ptz_role_id: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[model(
    table = "ptz_naive_pivot",
    primary_key = "id",
    casts = {
        created_at = suprnova::AsNaiveDateTime,
        updated_at = suprnova::AsNaiveDateTime,
    },
)]
pub struct PtzNaivePivot {
    pub id: i64,
    pub ptz_user_id: i64,
    pub ptz_role_id: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

async fn live_pivot_timestamps(env: &str) {
    use sea_orm::ConnectionTrait;

    let url = std::env::var(env).expect("explicit disposable database URL required");
    let guard: TestContainerGuard = TestContainer::fake();
    let config = DatabaseConfig::builder()
        .url(url)
        .max_connections(1)
        .min_connections(1)
        .logging(false)
        .build();
    let database = DbConnection::connect(&config)
        .await
        .expect("connect test database");
    TestContainer::singleton(database.clone());

    let postgres = database.inner().get_database_backend() == sea_orm::DatabaseBackend::Postgres;
    let id_column = if postgres {
        "id BIGSERIAL PRIMARY KEY"
    } else {
        "id BIGINT AUTO_INCREMENT PRIMARY KEY"
    };
    let (zoned, naive) = if postgres {
        ("TIMESTAMPTZ", "TIMESTAMP")
    } else {
        ("TIMESTAMP NULL", "DATETIME")
    };
    let pivot = |table: &str, stamp: &str| {
        format!(
            "CREATE TEMPORARY TABLE {table} ({id_column}, ptz_user_id BIGINT NOT NULL, \
             ptz_role_id BIGINT NOT NULL, created_at {stamp}, updated_at {stamp})"
        )
    };
    for sql in [
        format!("CREATE TEMPORARY TABLE ptz_users ({id_column}, name VARCHAR(255) NOT NULL)"),
        format!("CREATE TEMPORARY TABLE ptz_roles ({id_column}, name VARCHAR(255) NOT NULL)"),
        pivot("ptz_text_pivot", "VARCHAR(255) NOT NULL"),
        pivot("ptz_native_pivot", zoned),
        pivot("ptz_naive_pivot", naive),
    ] {
        database
            .inner()
            .execute_unprepared(&sql)
            .await
            .expect("create isolated temporary table");
    }

    let moment = Utc.with_ymd_and_hms(2031, 3, 14, 9, 26, 53).unwrap();
    let _clock = TestClock::travel_to(moment);
    let user = PtzUser::create(attrs! { name: "Ada" }).await.unwrap();
    let role = PtzRole::create(attrs! { name: "editor" }).await.unwrap();
    user.text_roles()
        .attach(role.id)
        .await
        .expect("attach over text");
    user.native_roles()
        .attach(role.id)
        .await
        .expect("attach over a zone-aware column");
    user.naive_roles()
        .attach(role.id)
        .await
        .expect("attach over a column without a zone");

    let text = PtzTextPivot::query().first().await.unwrap().unwrap();
    assert_eq!((text.created_at, text.updated_at), (moment, moment));
    let native = PtzNativePivot::query().first().await.unwrap().unwrap();
    assert_eq!((native.created_at, native.updated_at), (moment, moment));
    let naive = PtzNaivePivot::query().first().await.unwrap().unwrap();
    assert_eq!((naive.created_at, naive.updated_at), (moment, moment));

    drop(guard);
    database.inner().clone().close().await.unwrap();
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL at PG_TEST_URL"]
async fn postgres_attached_pivot_timestamps_read_back_through_each_cast() {
    live_pivot_timestamps("PG_TEST_URL").await;
}

#[tokio::test]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_attached_pivot_timestamps_read_back_through_each_cast() {
    live_pivot_timestamps("MYSQL_TEST_URL").await;
}
