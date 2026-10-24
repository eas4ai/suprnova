//! Raw chunk cursors, task-local model routing and declared defaults.

use sea_orm::ConnectionTrait;
use std::sync::{Arc, Mutex};
use suprnova::testing::TestDatabase;
use suprnova::{Cast, ConnectionRegistry, DB, DbConnection, FrameworkError, Model, attrs, model};

/// Translate integer keys to prove the chunk cursor reads storage values.
pub struct ShiftKey<const SHIFT: i64>;
impl<const SHIFT: i64> Cast for ShiftKey<SHIFT> {
    type Runtime = i64;
    type Storage = i64;
    fn to_storage(value: &i64) -> Result<i64, FrameworkError> {
        value
            .checked_sub(SHIFT)
            .ok_or_else(|| FrameworkError::internal("key overflow"))
    }
    fn from_storage(value: &i64) -> Result<i64, FrameworkError> {
        value
            .checked_add(SHIFT)
            .ok_or_else(|| FrameworkError::internal("key overflow"))
    }
}

#[model(table = "delta_keys", casts = { id = ShiftKey<100> })]
/// Expose shifted keys so skipped rows show an incorrect cursor.
pub struct ForwardKey {
    /// Identify stored rows while exercising model construction.
    pub id: i64,
    /// Supply the caller attribute independently of declared defaults.
    pub title: String,
}
#[model(table = "delta_keys", casts = { id = ShiftKey<-100> })]
/// Expose shifted keys so repeated rows show an incorrect cursor.
pub struct BackwardKey {
    /// Identify stored rows while exercising model construction.
    pub id: i64,
    /// Supply the caller attribute independently of declared defaults.
    pub title: String,
}

#[model(table = "delta_posts", fillable = ["title", "status", "votes"], defaults(status = "draft", votes = 0))]
/// Separate model defaults from database defaults in partial creates.
pub struct DefaultPost {
    /// Identify stored rows while exercising model construction.
    pub id: i64,
    /// Supply the caller attribute independently of declared defaults.
    pub title: String,
    /// Read the declared default and explicit caller override.
    pub status: String,
    /// Verify zero is kept as a declared default.
    pub votes: i64,
}

#[model(table = "delta_posts", fillable = ["title", "status", "votes"])]
/// Preserve construction and inserts for models without defaults.
pub struct PlainPost {
    /// Identify stored rows while exercising model construction.
    pub id: i64,
    /// Supply the caller attribute independently of declared defaults.
    pub title: String,
    /// Read the declared default and explicit caller override.
    pub status: String,
    /// Verify zero is kept as a declared default.
    pub votes: i64,
}

#[model(table = "delta_posts", fillable = ["title"], defaults(status = "draft", votes = 0))]
/// Verify trusted defaults survive the caller mass-assignment filter.
pub struct GuardedDefaultPost {
    /// Identify stored rows while exercising model construction.
    pub id: i64,
    /// Supply the caller attribute independently of declared defaults.
    pub title: String,
    /// Read the declared default and explicit caller override.
    pub status: String,
    /// Verify zero is kept as a declared default.
    pub votes: i64,
}

async fn posts() -> TestDatabase {
    let db = TestDatabase::sqlite_memory().await.expect("database");
    db.execute_unprepared("CREATE TABLE delta_posts (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL, status TEXT NOT NULL DEFAULT 'database', votes INTEGER NOT NULL DEFAULT 7)").await.expect("table");
    db
}

#[test]
fn new_reads_all_declared_defaults_and_plain_construction_stays_unchanged() {
    let post = DefaultPost::new();
    assert_eq!(post.status, "draft");
    assert_eq!(post.field_value("votes"), Some(serde_json::json!(0)));
    assert_eq!(post.title, "");
    assert_eq!(post.id, 0);
    let plain = PlainPost::new();
    assert_eq!(plain.status, "");
    assert_eq!(plain.votes, 0);
}

#[tokio::test]
async fn partial_create_saves_defaults_and_explicit_attributes_win() {
    let _db = posts().await;
    let made = DefaultPost::create(attrs! { title: "First" })
        .await
        .expect("partial create");
    let read = DefaultPost::find(made.id)
        .await
        .expect("read")
        .expect("row");
    assert_eq!(read.status, "draft");
    assert_eq!(read.votes, 0);
    let live = DefaultPost::create(attrs! { title: "Second", status: "live", votes: 12 })
        .await
        .expect("explicit values");
    assert_eq!(live.status, "live");
    assert_eq!(live.votes, 12);
    let plain = PlainPost::create(attrs! { title: "Plain" })
        .await
        .expect("no declared defaults");
    assert_eq!(plain.status, "database");
    assert_eq!(plain.votes, 7);
    let guarded = GuardedDefaultPost::create(attrs! { title: "Trusted defaults" })
        .await
        .expect("guarded defaults");
    assert_eq!(guarded.status, "draft");
    assert_eq!(guarded.votes, 0);
}

#[tokio::test]
async fn transactional_create_and_unsaved_attributes_use_the_same_defaults() {
    let _db = posts().await;
    let tx = DB::begin_transaction().await.expect("transaction");
    let made = DefaultPost::create_with_tx(&tx, attrs! { title: "Tx", status: "live" })
        .await
        .expect("create");
    assert_eq!(made.status, "live");
    assert_eq!(made.votes, 0);
    tx.commit().await.expect("commit");
    let unsaved =
        <DefaultPost as suprnova::FirstOrCreate>::from_attrs_unsaved(attrs! { title: "Unsaved" })
            .expect("unsaved");
    assert_eq!(unsaved.status, "draft");
    assert_eq!(unsaved.votes, 0);
    assert_eq!(unsaved.title, "Unsaved");
}

#[tokio::test]
async fn invalid_supplied_attribute_errors_without_saving_a_default_in_its_place() {
    let _db = posts().await;
    assert!(
        DefaultPost::create(attrs! { title: "Invalid", votes: "not an integer" })
            .await
            .is_err()
    );
    assert_eq!(DefaultPost::query().count().await.expect("count"), 0);
}

#[tokio::test]
async fn cast_keys_chunk_once_each_using_the_stored_cursor_in_both_directions() {
    let db = TestDatabase::sqlite_memory().await.expect("database");
    db.execute_unprepared("CREATE TABLE delta_keys (id INTEGER PRIMARY KEY, title TEXT NOT NULL)")
        .await
        .expect("table");
    db.execute_unprepared(
        "INSERT INTO delta_keys VALUES (1, 'a'), (2, 'b'), (3, 'c'), (4, 'd'), (5, 'e')",
    )
    .await
    .expect("seed");
    let seen = Arc::new(Mutex::new(Vec::new()));
    let captured = seen.clone();
    ForwardKey::query()
        .chunk_by_id(2, move |batch| {
            let seen = captured.clone();
            async move {
                seen.lock()
                    .expect("seen")
                    .extend(batch.iter().map(|row| row.id));
                Ok(())
            }
        })
        .await
        .expect("forward cast");
    assert_eq!(*seen.lock().expect("seen"), [101, 102, 103, 104, 105]);
    seen.lock().expect("seen").clear();
    let captured = seen.clone();
    BackwardKey::query()
        .chunk_by_id(2, move |batch| {
            let seen = captured.clone();
            async move {
                let mut seen = seen.lock().expect("seen");
                for row in batch.iter() {
                    assert!(
                        !seen.contains(&row.id),
                        "stored cursor must never repeat a row"
                    );
                    seen.push(row.id);
                }
                Ok(())
            }
        })
        .await
        .expect("backward cast");
    assert_eq!(*seen.lock().expect("seen"), [-99, -98, -97, -96, -95]);
    assert!(
        ForwardKey::query()
            .chunk_by_id(0, |_| async { Ok(()) })
            .await
            .is_err()
    );
    let failure = ForwardKey::query()
        .chunk_by_id(2, |_| async {
            Err(FrameworkError::internal("callback failed"))
        })
        .await
        .expect_err("callback error");
    assert_eq!(failure.message(), "callback failed");
    assert!(
        ForwardKey::query()
            .filter("id", 999)
            .chunk_by_id(2, |_| async { panic!("empty chunk must not call back") })
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn scoped_default_routes_model_reads_and_creates_without_leaking_to_concurrent_task() {
    let _primary = posts().await;
    PlainPost::create(attrs! { title: "primary" })
        .await
        .expect("primary seed");
    let reporting = DbConnection::from_raw(
        sea_orm::Database::connect("sqlite::memory:")
            .await
            .expect("reporting"),
    );
    reporting.inner().execute_unprepared("CREATE TABLE delta_posts (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL, status TEXT NOT NULL DEFAULT 'reporting', votes INTEGER NOT NULL DEFAULT 42)").await.expect("table");
    reporting
        .inner()
        .execute_unprepared("INSERT INTO delta_posts (title) VALUES ('reporting')")
        .await
        .expect("seed");
    let name = format!("model_reporting_{}", uuid::Uuid::new_v4());
    ConnectionRegistry::register_existing(&name, reporting)
        .await
        .expect("register");
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let other_barrier = barrier.clone();
    let other = tokio::spawn(async move {
        other_barrier.wait().await;
        assert_eq!(
            PlainPost::query()
                .first()
                .await
                .expect("primary")
                .expect("row")
                .title,
            "primary"
        );
        other_barrier.wait().await;
    });
    DB::with_default_connection(&name, async {
        barrier.wait().await;
        assert_eq!(
            PlainPost::query()
                .first()
                .await?
                .expect("reporting row")
                .title,
            "reporting"
        );
        let made = PlainPost::create(attrs! { title: "scoped" }).await?;
        assert_eq!(made.status, "reporting");
        assert_eq!(made.votes, 42);
        assert_eq!(PlainPost::on_write_connection().count().await?, 1);
        barrier.wait().await;
        Ok(())
    })
    .await
    .expect("scope");
    other.await.expect("other task");
    assert_eq!(
        PlainPost::query()
            .count()
            .await
            .expect("primary after scope"),
        1
    );
}

/// The relation operations of the delta (PAR-092), from lane d2.
pub mod relations;
