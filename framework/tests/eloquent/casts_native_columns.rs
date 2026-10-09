//! Casts over native JSON and boolean columns on real Postgres, MySQL and
//! MariaDB.
//!
//! The text structured casts (`AsJson` and its siblings) bind and read text:
//! Postgres refuses a text parameter for a `jsonb` or `json` column, and the
//! MySQL driver refuses to read MySQL's `JSON` type as text. `AsNativeJson`
//! and `AsOptionalNativeJson` store JSON, for those columns.
//!
//! The file also covers `AsEnumCollection` on a native JSON column.
//!
//! `AsBool` stores a 64-bit integer: it serves an integer column (`BIGINT` on
//! Postgres) and Postgres refuses it for a `BOOLEAN` one, so a native boolean
//! column is a plain `bool` field with no cast.
//!
//! ```text
//! PG_TEST_URL=postgres://... cargo test -p suprnova --test eloquent -- \
//!   --ignored --test-threads=1 casts_native_columns::postgres_
//! MYSQL_TEST_URL=mysql://... cargo test -p suprnova --test eloquent -- \
//!   --ignored --test-threads=1 casts_native_columns::mysql_
//! ```

use serde::{Deserialize, Serialize};
use serde_json::json;
use std::str::FromStr;
use suprnova::database::{DatabaseConfig, DbConnection};
use suprnova::testing::{TestContainer, TestContainerGuard};
use suprnova::{AsBool, AsEnumCollection, AsNativeJson, AsOptionalNativeJson, Model, attrs, model};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NcPrefs {
    pub theme: String,
    pub size: i64,
}

#[model(
    table = "nc_documents",
    timestamps = false,
    casts = {
        prefs = AsNativeJson<NcPrefs>,
        tags = AsOptionalNativeJson<Vec<String>>,
        archived = AsBool,
    },
)]
pub struct NcDocument {
    pub id: i64,
    pub prefs: NcPrefs,
    pub tags: Option<Vec<String>>,
    pub published: bool,
    pub archived: bool,
}

/// Storage strings are `"draft"` and `"live"`, the way Laravel writes a backed enum.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum NcStatus {
    Draft,
    Live,
}

impl FromStr for NcStatus {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "draft" => Ok(Self::Draft),
            "live" => Ok(Self::Live),
            other => Err(format!("unknown status `{other}`")),
        }
    }
}

impl AsRef<str> for NcStatus {
    fn as_ref(&self) -> &str {
        match self {
            Self::Draft => "draft",
            Self::Live => "live",
        }
    }
}

#[model(
    table = "nc_enum_lists",
    timestamps = false,
    casts = {
        statuses = AsEnumCollection<NcStatus>,
    },
)]
pub struct NcEnumList {
    pub id: i64,
    pub statuses: Vec<NcStatus>,
}

async fn live_native_columns(env: &str) {
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
    let table = if postgres {
        "CREATE TEMPORARY TABLE nc_documents (id BIGSERIAL PRIMARY KEY, \
         prefs JSONB NOT NULL, tags JSON NULL, published BOOLEAN NOT NULL, \
         archived BIGINT NOT NULL)"
    } else {
        "CREATE TEMPORARY TABLE nc_documents (id BIGINT AUTO_INCREMENT PRIMARY KEY, \
         prefs JSON NOT NULL, tags JSON NULL, published BOOLEAN NOT NULL, \
         archived BIGINT NOT NULL)"
    };
    database
        .inner()
        .execute_unprepared(table)
        .await
        .expect("create isolated temporary table");

    let made = NcDocument::create(attrs! {
        prefs: json!({ "theme": "dark", "size": 12 }),
        tags: json!(["a", "b"]),
        published: true,
        archived: false,
    })
    .await
    .expect("create through native JSON and boolean columns");
    let read = NcDocument::find(made.id)
        .await
        .expect("read native JSON and boolean columns")
        .expect("the row exists");
    assert_eq!(
        read.prefs,
        NcPrefs {
            theme: "dark".into(),
            size: 12
        }
    );
    assert_eq!(read.tags, Some(vec!["a".to_owned(), "b".to_owned()]));
    assert!(read.published);
    assert!(!read.archived);

    let updated = read
        .update(attrs! {
            prefs: json!({ "theme": "light", "size": 14 }),
            tags: serde_json::Value::Null,
            archived: true,
        })
        .await
        .expect("update native JSON and boolean columns");
    assert_eq!(updated.tags, None);
    let reread = NcDocument::find(updated.id).await.unwrap().unwrap();
    assert_eq!(reread.prefs.theme, "light");
    assert_eq!(reread.tags, None);
    assert!(reread.archived);

    let changed = NcDocument::query()
        .filter("id", reread.id)
        .update_all(attrs! { prefs: json!({ "theme": "blue", "size": 16 }) })
        .await
        .expect("a mass update binds a native JSON column");
    assert_eq!(changed, 1);
    assert_eq!(
        NcDocument::find(reread.id)
            .await
            .unwrap()
            .unwrap()
            .prefs
            .theme,
        "blue"
    );

    drop(guard);
    database.inner().clone().close().await.unwrap();
}

async fn live_enum_collection(env: &str) {
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
    let table = if postgres {
        "CREATE TEMPORARY TABLE nc_enum_lists (id BIGSERIAL PRIMARY KEY, statuses JSONB NOT NULL)"
    } else {
        "CREATE TEMPORARY TABLE nc_enum_lists (id BIGINT AUTO_INCREMENT PRIMARY KEY, statuses JSON NOT NULL)"
    };
    database
        .inner()
        .execute_unprepared(table)
        .await
        .expect("create isolated temporary table");

    // Laravel writes the list as a plain JSON array of storage strings.
    let insert = if postgres {
        "INSERT INTO nc_enum_lists (id, statuses) VALUES (1, '[\"draft\",\"live\"]'::jsonb)"
    } else {
        "INSERT INTO nc_enum_lists (id, statuses) VALUES (1, '[\"draft\",\"live\"]')"
    };
    database
        .inner()
        .execute_unprepared(insert)
        .await
        .expect("insert a Laravel-written enum list");

    let read = NcEnumList::find(1)
        .await
        .expect("read the enum collection")
        .expect("the row exists");
    assert_eq!(read.statuses, vec![NcStatus::Draft, NcStatus::Live]);

    let mut list = read;
    list.statuses = vec![NcStatus::Live, NcStatus::Draft, NcStatus::Live];
    list.save()
        .await
        .expect("write the enum collection through a native JSON column");
    let reread = NcEnumList::find(1)
        .await
        .expect("read the saved enum collection")
        .expect("the row exists");
    assert_eq!(
        reread.statuses,
        vec![NcStatus::Live, NcStatus::Draft, NcStatus::Live]
    );

    let changed = NcEnumList::query()
        .filter("id", 1)
        .update_all(attrs! { statuses: json!(["draft"]) })
        .await
        .expect("a mass update binds an enum collection to a native JSON column");
    assert_eq!(changed, 1);
    let last = NcEnumList::find(1)
        .await
        .expect("read the mass-updated enum collection")
        .expect("the row exists");
    assert_eq!(last.statuses, vec![NcStatus::Draft]);
    assert_eq!(last.get_raw_original("statuses"), Some(json!(["draft"])));

    drop(guard);
    database.inner().clone().close().await.unwrap();
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL at PG_TEST_URL"]
async fn postgres_casts_read_and_write_native_json_and_boolean_columns() {
    live_native_columns("PG_TEST_URL").await;
}

#[tokio::test]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_casts_read_and_write_native_json_and_boolean_columns() {
    live_native_columns("MYSQL_TEST_URL").await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL at PG_TEST_URL"]
async fn postgres_enum_collection_reads_and_writes_a_native_json_column() {
    live_enum_collection("PG_TEST_URL").await;
}

#[tokio::test]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_enum_collection_reads_and_writes_a_native_json_column() {
    live_enum_collection("MYSQL_TEST_URL").await;
}
