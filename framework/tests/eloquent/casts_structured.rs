//! Phase 10A T7b - Structured + enum casts + `with_casts` runtime
//! override.
//!
//! Same model-hoisting convention as T7a: each test's model lives at
//! module scope so the `#[model]` macro's inner module (which only
//! sees the test file's top-level `use` items) resolves the cast type
//! names correctly. The 5 structured casts round-trip Vec / HashMap /
//! Collection / serde_json::Value / IndexMap shapes; `AsEnum` rides on
//! `strum::EnumString` + `strum::AsRefStr` for FromStr / AsRef<str>
//! cleanly without a custom impl. The final two tests exercise the
//! `Builder<M>::with_casts(...)` runtime override path: one with
//! well-formed data (proves the pipeline runs end-to-end), one with
//! malformed data (proves the cast actually fires - without it the
//! query succeeds; with it the cast errors).
//!
//! T7c finishes the cast surface with encrypted + hashed casts.

use chrono::Utc;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use suprnova::testing::TestDatabase;
use suprnova::{
    AsArray, AsArrayObject, AsBool, AsCollection, AsDate, AsDateTime, AsEnum, AsInt, AsJson,
    AsObject, AsOptionalArray, AsOptionalArrayObject, AsOptionalCollection, AsOptionalJson,
    AsOptionalObject, Cast, Collection, Model, attrs, model,
};

// ---- Test fixtures hoisted to module scope ------------------------------

#[derive(
    Serialize, Deserialize, Clone, Debug, PartialEq, Default, strum::EnumString, strum::AsRefStr,
)]
pub enum Role {
    #[default]
    Admin,
    Member,
    Guest,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Prefs {
    pub theme: String,
    pub notifications: bool,
}

// ---- Models -------------------------------------------------------------

#[model(
    table = "t7b_arr",
    timestamps = false,
    fillable = ["tags"],
    casts = { tags = AsArray<String> }
)]
pub struct ArrModel {
    pub id: i64,
    pub tags: Vec<String>,
}

#[model(
    table = "t7b_obj",
    timestamps = false,
    fillable = ["prefs"],
    casts = { prefs = AsObject<Prefs> }
)]
pub struct ObjModel {
    pub id: i64,
    pub prefs: Prefs,
}

#[model(
    table = "t7b_col",
    timestamps = false,
    fillable = ["items"],
    casts = { items = AsCollection<String> }
)]
pub struct ColModel {
    pub id: i64,
    pub items: Collection<String>,
}

#[model(
    table = "t7b_json",
    timestamps = false,
    fillable = ["payload"],
    casts = { payload = AsJson<serde_json::Value> }
)]
pub struct JsonModel {
    pub id: i64,
    pub payload: serde_json::Value,
}

#[model(
    table = "t7b_ao",
    timestamps = false,
    fillable = ["labels"],
    casts = { labels = AsArrayObject<String> }
)]
pub struct AoModel {
    pub id: i64,
    pub labels: IndexMap<String, String>,
}

// #133: the nullable siblings of the JSON casts, over nullable columns.
#[model(
    table = "t133_meta",
    timestamps = false,
    fillable = ["metadata"],
    casts = { metadata = AsOptionalJson<serde_json::Value> }
)]
pub struct OptionalJsonModel {
    pub id: i64,
    pub metadata: Option<serde_json::Value>,
}

#[model(
    table = "t133_structured",
    timestamps = false,
    fillable = ["tags", "prefs", "items", "labels"],
    casts = {
        tags = AsOptionalArray<String>,
        prefs = AsOptionalObject<Prefs>,
        items = AsOptionalCollection<String>,
        labels = AsOptionalArrayObject<String>
    }
)]
pub struct OptionalStructuredModel {
    pub id: i64,
    pub tags: Option<Vec<String>>,
    pub prefs: Option<Prefs>,
    pub items: Option<Collection<String>>,
    pub labels: Option<IndexMap<String, String>>,
}

#[model(
    table = "t7b_enum",
    timestamps = false,
    fillable = ["role"],
    casts = { role = AsEnum<Role> }
)]
pub struct EnumModel {
    pub id: i64,
    pub role: Role,
}

#[model(table = "t7b_wc1", timestamps = false, fillable = ["ts"])]
pub struct WcSanityModel {
    pub id: i64,
    pub ts: String,
}

#[model(table = "t7b_wc2", timestamps = false, fillable = ["s"])]
pub struct WcParseFailModel {
    pub id: i64,
    pub s: String,
}

// Override-semantic fixture. Static `AsBool` (i64 ↔ bool) on `flag`,
// so the storage shape (`i64`) differs from the runtime shape (`bool`).
// The override-semantic test below replaces the static cast with a
// runtime `AsInt<i64>` cast and asserts the runtime cast saw the
// storage shape - proving runtime casts bypass static casts entirely,
// not stack on top of them.
#[model(
    table = "t7b_override",
    timestamps = false,
    fillable = ["flag"],
    casts = { flag = AsBool }
)]
pub struct OverrideModel {
    pub id: i64,
    pub flag: bool,
}

// ---- Tests --------------------------------------------------------------

#[tokio::test]
async fn as_array_round_trips_vec() {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    db.execute_unprepared(
        "CREATE TABLE t7b_arr (id INTEGER PRIMARY KEY AUTOINCREMENT, tags TEXT NOT NULL)",
    )
    .await
    .unwrap();
    let made = ArrModel::create(attrs! { tags: ["rust", "web"] })
        .await
        .unwrap();
    let read = ArrModel::find(made.id).await.unwrap().unwrap();
    assert_eq!(read.tags, vec!["rust".to_string(), "web".to_string()]);
}

#[tokio::test]
async fn as_object_round_trips_custom_struct() {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    db.execute_unprepared(
        "CREATE TABLE t7b_obj (id INTEGER PRIMARY KEY AUTOINCREMENT, prefs TEXT NOT NULL)",
    )
    .await
    .unwrap();
    let made = ObjModel::create(
        attrs! { prefs: serde_json::json!({ "theme": "dark", "notifications": true }) },
    )
    .await
    .unwrap();
    let read = ObjModel::find(made.id).await.unwrap().unwrap();
    assert_eq!(read.prefs.theme, "dark");
    assert!(read.prefs.notifications);
}

#[tokio::test]
async fn as_collection_wraps_vec_in_collection() {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    db.execute_unprepared(
        "CREATE TABLE t7b_col (id INTEGER PRIMARY KEY AUTOINCREMENT, items TEXT NOT NULL)",
    )
    .await
    .unwrap();
    let made = ColModel::create(attrs! { items: ["a", "b", "c"] })
        .await
        .unwrap();
    let read = ColModel::find(made.id).await.unwrap().unwrap();
    assert_eq!(read.items.len(), 3);
    // Deref to slice works - Collection<T>::Deref::Target = [T].
    assert_eq!(&read.items[0], "a");
}

#[tokio::test]
async fn as_json_preserves_raw_value() {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    db.execute_unprepared(
        "CREATE TABLE t7b_json (id INTEGER PRIMARY KEY AUTOINCREMENT, payload TEXT NOT NULL)",
    )
    .await
    .unwrap();
    let made = JsonModel::create(
        attrs! { payload: serde_json::json!({ "count": 42, "nested": { "ok": true } }) },
    )
    .await
    .unwrap();
    let read = JsonModel::find(made.id).await.unwrap().unwrap();
    assert_eq!(read.payload["count"], 42);
    assert_eq!(read.payload["nested"]["ok"], true);
}

#[tokio::test]
async fn as_array_object_preserves_string_keys() {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    db.execute_unprepared(
        "CREATE TABLE t7b_ao (id INTEGER PRIMARY KEY AUTOINCREMENT, labels TEXT NOT NULL)",
    )
    .await
    .unwrap();
    let made =
        AoModel::create(attrs! { labels: serde_json::json!({ "color": "blue", "size": "large" }) })
            .await
            .unwrap();
    let read = AoModel::find(made.id).await.unwrap().unwrap();
    assert_eq!(read.labels.get("color"), Some(&"blue".to_string()));
    assert_eq!(read.labels.get("size"), Some(&"large".to_string()));
}

#[tokio::test]
async fn as_enum_round_trips() {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    db.execute_unprepared(
        "CREATE TABLE t7b_enum (id INTEGER PRIMARY KEY AUTOINCREMENT, role TEXT NOT NULL)",
    )
    .await
    .unwrap();
    let made = EnumModel::create(attrs! { role: "Admin" }).await.unwrap();
    let read = EnumModel::find(made.id).await.unwrap().unwrap();
    assert_eq!(read.role, Role::Admin);
}

#[tokio::test]
async fn with_casts_sanity_succeeds_against_well_formed_data() {
    // Smoke check: with a well-formed timestamp, the runtime cast
    // pipeline fires and the query succeeds. The unambiguous proof
    // that the pipeline actually ran is in the next test (which uses
    // a malformed value to force the cast to error).
    let db = TestDatabase::sqlite_memory().await.unwrap();
    db.execute_unprepared(
        "CREATE TABLE t7b_wc1 (id INTEGER PRIMARY KEY AUTOINCREMENT, ts TEXT NOT NULL)",
    )
    .await
    .unwrap();
    let when = Utc::now();
    WcSanityModel::create(attrs! { ts: when.to_rfc3339() })
        .await
        .unwrap();

    let rows = WcSanityModel::query()
        .with_casts(suprnova::casts! { ts = AsDateTime })
        .get()
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
}

#[tokio::test]
async fn with_casts_bypasses_static_casts_entirely() {
    // Override semantics: when `with_casts(...)` is set, the static
    // cast pipeline is bypassed *entirely*, not stacked on top. To
    // prove this we use a model with `static AsBool` on a `bool` field,
    // then call `with_casts` setting an unrelated column. If the static
    // pipeline were still running, the bool field would land in `M` as
    // `true`/`false`. With the bypass semantic it lands in raw storage
    // shape (`i64`), which fails to deserialize into `M.flag: bool` -
    // proving the static cast did NOT run.
    let db = TestDatabase::sqlite_memory().await.unwrap();
    db.execute_unprepared(
        "CREATE TABLE t7b_override (id INTEGER PRIMARY KEY AUTOINCREMENT, flag INTEGER NOT NULL)",
    )
    .await
    .unwrap();
    let made = OverrideModel::create(attrs! { flag: true }).await.unwrap();

    // Baseline: without runtime casts, static AsBool fires and the
    // value round-trips correctly.
    let plain = OverrideModel::find(made.id).await.unwrap().unwrap();
    assert!(
        plain.flag,
        "static cast should round-trip without with_casts"
    );

    // With a runtime cast set on an unrelated column (`id` here), the
    // static cast pipeline is bypassed for ALL columns. The `flag`
    // column comes back as raw i64 (storage shape), which fails to
    // deserialize into the user's `bool` field - surfacing as a
    // FrameworkError.
    let result = OverrideModel::query()
        .with_casts(suprnova::casts! { id = AsInt<i64> })
        .get()
        .await;
    assert!(
        result.is_err(),
        "expected runtime override to bypass static AsBool and fail bool/i64 deserialization"
    );
}

#[tokio::test]
async fn with_casts_pipeline_actually_runs_proven_by_parse_failure() {
    // Stored data: a non-date string. Without a runtime cast, the
    // query succeeds (the model's String field accepts anything). With
    // an AsDate runtime cast, the pipeline tries to parse "not-a-date"
    // as a NaiveDate at decode time and the parse fails - surfacing as
    // a FrameworkError. This unambiguously proves the cast actually ran.
    let db = TestDatabase::sqlite_memory().await.unwrap();
    db.execute_unprepared(
        "CREATE TABLE t7b_wc2 (id INTEGER PRIMARY KEY AUTOINCREMENT, s TEXT NOT NULL)",
    )
    .await
    .unwrap();
    WcParseFailModel::create(attrs! { s: "not-a-date" })
        .await
        .unwrap();

    // Baseline: without the cast, the query succeeds.
    let plain = WcParseFailModel::query().get().await.unwrap();
    assert_eq!(plain.len(), 1);
    assert_eq!(plain[0].s, "not-a-date");

    // With the runtime cast: pipeline fires, parse fails, query errors.
    let result = WcParseFailModel::query()
        .with_casts(suprnova::casts! { s = AsDate })
        .get()
        .await;
    assert!(
        result.is_err(),
        "expected runtime AsDate cast to error on \"not-a-date\""
    );
    let msg = format!("{}", result.unwrap_err()).to_lowercase();
    assert!(
        msg.contains("date") || msg.contains("parse"),
        "expected date/parse mention in error, got: {msg}"
    );
}

// ---- #133: nullable JSON casts --------------------------------------------

async fn optional_json_table() -> TestDatabase {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    db.execute_unprepared(
        "CREATE TABLE t133_meta (id INTEGER PRIMARY KEY AUTOINCREMENT, metadata TEXT NULL)",
    )
    .await
    .unwrap();
    db
}

/// `1` when the row's `metadata` is SQL `NULL`. The text `null` would read
/// as `0`, which is the defect `AsJson<Option<T>>` has.
async fn metadata_is_sql_null(db: &TestDatabase, id: i64) -> i64 {
    db.fetch_one(
        "SELECT metadata IS NULL AS is_null FROM t133_meta WHERE id = ?",
        vec![id.into()],
    )
    .await
    .unwrap()
    .try_get("", "is_null")
    .unwrap()
}

#[tokio::test]
async fn as_optional_json_round_trips_some_and_stores_none_as_sql_null() {
    let db = optional_json_table().await;
    let value = serde_json::json!({ "count": 42, "nested": { "ok": true } });

    let made = OptionalJsonModel::create(attrs! { metadata: value.clone() })
        .await
        .unwrap();
    let read = OptionalJsonModel::find(made.id).await.unwrap().unwrap();
    assert_eq!(read.metadata, Some(value.clone()));
    assert_eq!(metadata_is_sql_null(&db, made.id).await, 0);

    // `update` decodes the JSON `null` into `None`.
    let updated = read
        .update(attrs! { metadata: serde_json::Value::Null })
        .await
        .unwrap();
    assert_eq!(updated.metadata, None);
    assert_eq!(
        OptionalJsonModel::find(made.id)
            .await
            .unwrap()
            .unwrap()
            .metadata,
        None
    );
    assert_eq!(
        metadata_is_sql_null(&db, made.id).await,
        1,
        "None must store SQL NULL, not the text `null`"
    );

    // `save` writes the field through the same cast.
    let mut restored = OptionalJsonModel::find(made.id).await.unwrap().unwrap();
    restored.metadata = Some(value.clone());
    restored.save().await.unwrap();
    assert_eq!(
        OptionalJsonModel::find(made.id)
            .await
            .unwrap()
            .unwrap()
            .metadata,
        Some(value)
    );
    let mut cleared = OptionalJsonModel::find(made.id).await.unwrap().unwrap();
    cleared.metadata = None;
    cleared.save().await.unwrap();
    assert_eq!(metadata_is_sql_null(&db, made.id).await, 1);
}

#[tokio::test]
async fn as_optional_json_reads_a_pre_existing_null_row_as_none() {
    let db = optional_json_table().await;
    db.execute_unprepared("INSERT INTO t133_meta (id, metadata) VALUES (7, NULL)")
        .await
        .unwrap();

    let read = OptionalJsonModel::find(7_i64).await.unwrap().unwrap();
    assert_eq!(read.metadata, None);
}

#[tokio::test]
async fn as_optional_json_returns_the_as_json_error_on_malformed_json() {
    let db = optional_json_table().await;
    db.execute_unprepared("INSERT INTO t133_meta (id, metadata) VALUES (9, '{not json')")
        .await
        .unwrap();

    let cast_error = <AsJson<serde_json::Value> as Cast>::from_storage(&"{not json".to_string())
        .expect_err("AsJson rejects malformed JSON")
        .to_string();
    let error = OptionalJsonModel::find(9_i64)
        .await
        .expect_err("a malformed non-null row is a cast error, not a value")
        .to_string();
    assert!(
        error.contains(&cast_error),
        "the optional cast reports AsJson's error ({cast_error}); got: {error}"
    );
}

#[tokio::test]
async fn the_other_optional_structured_casts_round_trip_some_and_none() {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    db.execute_unprepared(
        "CREATE TABLE t133_structured (id INTEGER PRIMARY KEY AUTOINCREMENT, \
         tags TEXT NULL, prefs TEXT NULL, items TEXT NULL, labels TEXT NULL)",
    )
    .await
    .unwrap();

    let full = OptionalStructuredModel::create(attrs! {
        tags: ["rust", "web"],
        prefs: serde_json::json!({ "theme": "dark", "notifications": true }),
        items: ["a", "b"],
        labels: serde_json::json!({ "color": "blue", "size": "large" }),
    })
    .await
    .unwrap();
    let read = OptionalStructuredModel::find(full.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(read.tags, Some(vec!["rust".to_string(), "web".to_string()]));
    assert_eq!(
        read.prefs,
        Some(Prefs {
            theme: "dark".into(),
            notifications: true
        })
    );
    let items = read.items.expect("items present");
    assert_eq!(items.len(), 2);
    assert_eq!(&items[0], "a");
    let labels = read.labels.expect("labels present");
    assert_eq!(
        labels.keys().collect::<Vec<_>>(),
        vec!["color", "size"],
        "AsOptionalArrayObject keeps key order"
    );

    let empty = OptionalStructuredModel::create(attrs! {
        tags: serde_json::Value::Null,
        prefs: serde_json::Value::Null,
        items: serde_json::Value::Null,
        labels: serde_json::Value::Null,
    })
    .await
    .unwrap();
    let read = OptionalStructuredModel::find(empty.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(read.tags, None);
    assert_eq!(read.prefs, None);
    assert!(read.items.is_none());
    assert_eq!(read.labels, None);

    let nulls: i64 = db
        .fetch_one(
            "SELECT (tags IS NULL) + (prefs IS NULL) + (items IS NULL) + (labels IS NULL) AS n \
             FROM t133_structured WHERE id = ?",
            vec![empty.id.into()],
        )
        .await
        .unwrap()
        .try_get("", "n")
        .unwrap();
    assert_eq!(nulls, 4, "every None is stored as SQL NULL");
}

/// The same round trip on Postgres, whose `TEXT` column and parameter
/// types are stricter than SQLite's.
///
/// ```text
/// PG_TEST_URL=postgres://... cargo test -p suprnova --test eloquent -- \
///   --ignored --test-threads=1 casts_structured::postgres_
/// ```
#[tokio::test]
#[serial_test::serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_as_optional_json_round_trips_some_and_none() {
    use sea_orm::{ConnectOptions, ConnectionTrait, Database, Statement};
    use suprnova::DbConnection;
    use suprnova::testing::TestContainer;

    let url = std::env::var("PG_TEST_URL").expect("set PG_TEST_URL to a disposable Postgres");
    let mut options = ConnectOptions::new(url);
    options
        .max_connections(2)
        .min_connections(0)
        .connect_timeout(std::time::Duration::from_secs(2))
        .acquire_timeout(std::time::Duration::from_secs(2));
    let conn = Database::connect(options)
        .await
        .expect("Postgres test database must be reachable");
    let backend = conn.get_database_backend();
    for sql in [
        "DROP TABLE IF EXISTS t133_meta",
        "CREATE TABLE t133_meta (id BIGSERIAL PRIMARY KEY, metadata TEXT NULL)",
        "INSERT INTO t133_meta (metadata) VALUES (NULL)",
    ] {
        conn.execute_raw(Statement::from_string(backend, sql.to_owned()))
            .await
            .unwrap();
    }
    let _guard = TestContainer::fake();
    TestContainer::singleton(DbConnection::from_raw(conn.clone()));

    let pre_existing = OptionalJsonModel::query().first().await.unwrap().unwrap();
    assert_eq!(pre_existing.metadata, None);

    let value = serde_json::json!({ "count": 42 });
    let made = OptionalJsonModel::create(attrs! { metadata: value.clone() })
        .await
        .unwrap();
    assert_eq!(
        OptionalJsonModel::find(made.id)
            .await
            .unwrap()
            .unwrap()
            .metadata,
        Some(value)
    );
    let cleared = made
        .update(attrs! { metadata: serde_json::Value::Null })
        .await
        .unwrap();
    assert_eq!(cleared.metadata, None);
    let row = conn
        .query_one_raw(Statement::from_sql_and_values(
            backend,
            "SELECT metadata IS NULL AS is_null FROM t133_meta WHERE id = $1",
            [cleared.id.into()],
        ))
        .await
        .unwrap()
        .unwrap();
    let is_null: bool = row.try_get("", "is_null").unwrap();
    assert!(is_null, "None must store SQL NULL on Postgres");

    conn.execute_raw(Statement::from_string(
        backend,
        "DROP TABLE t133_meta".to_owned(),
    ))
    .await
    .unwrap();
}
