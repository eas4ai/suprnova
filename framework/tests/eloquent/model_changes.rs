//! PAR-004: a model reports what its last save changed, as Laravel's does.
//!
//! After a save that changes something, `was_changed` / `was_changed_any` /
//! `get_changes` describe that save, and keep describing it until a later
//! save changes something. While the save's `updated` and `saved` observers
//! run, `get_original` (cast) and `get_raw_original` (stored) return the
//! values loaded before the save; once the save returns they return the
//! saved values. An insert reports no changes, and has no original until
//! it returns. Laravel's
//! `HasAttributes::wasChanged`, `getChanges`, `getOriginal`,
//! `getRawOriginal`, `syncChanges` and `syncOriginal`, as `Model::save`,
//! `performUpdate` and `finishSave` call them.
//!
//! `is_admin` carries the `AsBool` cast, so its stored value (`0` / `1`)
//! differs from its cast value (`false` / `true`): that is what tells
//! `get_raw_original` and `get_original` apart.
//!
//! The observer models are dedicated to this file: observer
//! registration is process-global, and every record it makes carries the
//! row's email, so each test reads only the records of its own row.
//!
//! The `postgres_` and `mysql_` variants run the observer scenario against
//! a disposable server. They are ignored by default:
//!
//! ```text
//! PG_TEST_URL=postgres://... cargo nextest run -p suprnova --test eloquent \
//!   --run-ignored only -E 'test(/^model_changes::/)'
//! ```

use std::sync::Mutex;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};
use suprnova::database::{DatabaseConfig, DbConnection};
use suprnova::eloquent::observers::Observer;
use suprnova::testing::{TestClock, TestContainer, TestContainerGuard, TestDatabase};
use suprnova::{AsBool, AsEncrypted, Attrs, DB, FrameworkError, Model, attrs, model};

// ---- Models -------------------------------------------------------------

/// The model the observer scenarios save. No timestamps, so a save changes
/// exactly the columns the test changes.
#[model(
    table = "par_change_users",
    timestamps = false,
    fillable = ["name", "email", "is_admin"],
    casts = { is_admin = AsBool }
)]
pub struct ChangeUser {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub is_admin: bool,
}

/// Same shape, no observer: the caller-side scenarios.
#[model(
    table = "par_plain_users",
    timestamps = false,
    fillable = ["name", "email", "is_admin"],
    casts = { is_admin = AsBool }
)]
pub struct PlainUser {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub is_admin: bool,
}

/// A model with timestamps: a save also changes `updated_at`.
#[model(table = "par_change_posts", fillable = ["title", "body"])]
pub struct ChangePost {
    pub id: i64,
    pub title: String,
    pub body: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A model with an encrypted column. `AsEncrypted` writes a new ciphertext
/// on every save, so the stored value of `secret` differs after any save;
/// only a change to the secret itself is a change.
#[model(
    table = "par_secret_users",
    timestamps = false,
    fillable = ["secret", "is_admin"],
    casts = { secret = AsEncrypted, is_admin = AsBool }
)]
pub struct SecretUser {
    pub id: i64,
    pub secret: String,
    pub is_admin: bool,
}

/// The observer scenario's model for the live-engine variants. Separate
/// from `ChangeUser` so its observer never sees a SQLite test's rows.
#[model(
    table = "par_live_change_users",
    timestamps = false,
    fillable = ["name", "email", "is_admin"],
    casts = { is_admin = AsBool }
)]
pub struct LiveChangeUser {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub is_admin: bool,
}

// ---- What an observer saw -----------------------------------------------

/// Everything an observer reads off the model it receives, captured so the
/// test can assert on it after the save returns.
#[derive(Debug, Clone)]
struct Seen {
    event: &'static str,
    email: String,
    is_admin_changed: bool,
    name_changed: bool,
    name_or_email_changed: bool,
    name_or_is_admin_changed: bool,
    anything_changed: bool,
    changes: Vec<(String, Value)>,
    raw_original_is_admin: Option<Value>,
    original_is_admin: Option<Value>,
}

fn sorted_changes(changes: &Attrs) -> Vec<(String, Value)> {
    let mut pairs: Vec<(String, Value)> = changes
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect();
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    pairs
}

fn see<M>(event: &'static str, email: &str, current: &M) -> Result<Seen, FrameworkError>
where
    M: Model + From<<M::Entity as sea_orm::EntityTrait>::Model>,
    <M::Entity as sea_orm::EntityTrait>::Model: From<M>
        + sea_orm::IntoActiveModel<<M::Entity as sea_orm::EntityTrait>::ActiveModel>
        + serde::Serialize
        + Send
        + Sync,
    <M::Entity as sea_orm::EntityTrait>::ActiveModel: Send,
    <<M::Entity as sea_orm::EntityTrait>::PrimaryKey as sea_orm::PrimaryKeyTrait>::ValueType:
        Send + Into<sea_orm::Value>,
{
    Ok(Seen {
        event,
        email: email.to_string(),
        is_admin_changed: current.was_changed("is_admin"),
        name_changed: current.was_changed("name"),
        name_or_email_changed: current.was_changed_any(&["name", "email"]),
        name_or_is_admin_changed: current.was_changed_any(&["name", "is_admin"]),
        anything_changed: current.was_changed_any(&[]),
        changes: sorted_changes(&current.get_changes()),
        raw_original_is_admin: current.get_raw_original("is_admin"),
        original_is_admin: current.get_original("is_admin")?,
    })
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

fn seen_for(event: &str, email: &str) -> Vec<Seen> {
    SEEN.lock()
        .unwrap()
        .iter()
        .filter(|s| s.event == event && s.email == email)
        .cloned()
        .collect()
}

#[derive(Clone)]
struct AuditObserver;

#[async_trait]
impl Observer<ChangeUser> for AuditObserver {
    async fn created(&self, model: &ChangeUser) -> Result<(), FrameworkError> {
        let seen = see("created", &model.email, model)?;
        SEEN.lock().unwrap().push(seen);
        Ok(())
    }

    async fn updated(
        &self,
        _previous: &ChangeUser,
        current: &ChangeUser,
    ) -> Result<(), FrameworkError> {
        let seen = see("updated", &current.email, current)?;
        SEEN.lock().unwrap().push(seen);
        Ok(())
    }

    async fn saved(&self, model: &ChangeUser) -> Result<(), FrameworkError> {
        let seen = see("saved", &model.email, model)?;
        SEEN.lock().unwrap().push(seen);
        Ok(())
    }
}

#[derive(Clone)]
struct LiveAuditObserver;

#[async_trait]
impl Observer<LiveChangeUser> for LiveAuditObserver {
    async fn updated(
        &self,
        _previous: &LiveChangeUser,
        current: &LiveChangeUser,
    ) -> Result<(), FrameworkError> {
        let seen = see("updated", &current.email, current)?;
        SEEN.lock().unwrap().push(seen);
        Ok(())
    }
}

/// What an observer of a save that flipped `is_admin` from `false` to
/// `true`, and nothing else, must see.
fn assert_saw_only_the_flip(seen: &Seen) {
    assert!(seen.is_admin_changed, "was_changed(is_admin): {seen:?}");
    assert!(!seen.name_changed, "was_changed(name): {seen:?}");
    assert!(
        !seen.name_or_email_changed,
        "was_changed_any([name, email]) names nothing the save changed: {seen:?}"
    );
    assert!(seen.name_or_is_admin_changed, "{seen:?}");
    assert!(seen.anything_changed, "{seen:?}");
    assert_eq!(
        seen.changes,
        vec![("is_admin".to_string(), json!(1))],
        "get_changes holds exactly the flipped column, as stored"
    );
    assert_eq!(
        seen.raw_original_is_admin,
        Some(json!(0)),
        "get_raw_original is the stored value loaded before the save"
    );
    assert_eq!(
        seen.original_is_admin,
        Some(json!(false)),
        "get_original is the loaded value through the cast"
    );
}

/// The observer records of one save: every `updated` record, and the last
/// `saved` record (the first one belongs to the row's insert).
fn assert_observers_saw_only_the_flip(email: &str) {
    let updated = seen_for("updated", email);
    assert!(!updated.is_empty(), "the updated observer ran");
    for record in &updated {
        assert_saw_only_the_flip(record);
    }
    let saved = seen_for("saved", email);
    let last_saved = saved.last().expect("the saved observer ran");
    assert_saw_only_the_flip(last_saved);
}

// ---- Fixtures -----------------------------------------------------------

async fn sqlite() -> TestDatabase {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    for table in ["par_change_users", "par_plain_users"] {
        db.execute_unprepared(&format!(
            "CREATE TABLE {table} (\
                id INTEGER PRIMARY KEY AUTOINCREMENT, \
                name TEXT NOT NULL, \
                email TEXT NOT NULL UNIQUE, \
                is_admin INTEGER NOT NULL)"
        ))
        .await
        .unwrap();
    }
    db.execute_unprepared(
        "CREATE TABLE par_change_posts (\
            id INTEGER PRIMARY KEY AUTOINCREMENT, \
            title TEXT NOT NULL, \
            body TEXT NOT NULL, \
            created_at TEXT NOT NULL, \
            updated_at TEXT NOT NULL)",
    )
    .await
    .unwrap();
    db
}

async fn plain(name: &str, email: &str) -> PlainUser {
    let created = PlainUser::create(attrs! { name: name, email: email, is_admin: false })
        .await
        .unwrap();
    PlainUser::find_or_fail(created.id).await.unwrap()
}

// ---- Inside the `updated` and `saved` observers -------------------------

#[tokio::test]
async fn observers_see_what_save_changed_and_the_loaded_value() {
    let _db = sqlite().await;
    ChangeUser::observe(AuditObserver).await;
    let created = ChangeUser::create(attrs! {
        name: "Ada",
        email: "observer-save@example.com",
        is_admin: false,
    })
    .await
    .unwrap();

    let mut user = ChangeUser::find_or_fail(created.id).await.unwrap();
    user.is_admin = true;
    user.save().await.unwrap();

    assert_observers_saw_only_the_flip("observer-save@example.com");
}

#[tokio::test]
async fn observers_see_what_update_changed() {
    let _db = sqlite().await;
    ChangeUser::observe(AuditObserver).await;
    let created = ChangeUser::create(attrs! {
        name: "Grace",
        email: "observer-update@example.com",
        is_admin: false,
    })
    .await
    .unwrap();

    let user = ChangeUser::find_or_fail(created.id).await.unwrap();
    user.update(attrs! { is_admin: true }).await.unwrap();

    assert_observers_saw_only_the_flip("observer-update@example.com");
}

#[tokio::test]
async fn an_insert_has_no_original_while_its_observers_run() {
    let _db = sqlite().await;
    ChangeUser::observe(AuditObserver).await;
    let email = "observer-insert@example.com";
    let created = ChangeUser::create(attrs! { name: "Ada", email: email, is_admin: true })
        .await
        .unwrap();

    for event in ["created", "saved"] {
        let records = seen_for(event, email);
        assert!(!records.is_empty(), "the {event} observer ran");
        for record in &records {
            assert_eq!(
                record.raw_original_is_admin, None,
                "{event}: nothing was loaded before the insert: {record:?}"
            );
            assert_eq!(record.original_is_admin, None, "{event}: {record:?}");
            assert!(!record.anything_changed, "{event}: {record:?}");
        }
    }
    assert_eq!(
        created.get_raw_original("is_admin"),
        Some(json!(1)),
        "once the insert returns, the original is the inserted row"
    );
}

// ---- On the caller's model ----------------------------------------------

#[tokio::test]
async fn save_leaves_the_change_record_on_the_saved_model() {
    let _db = sqlite().await;
    let mut user = plain("Ada", "save@example.com").await;
    user.is_admin = true;
    user.save().await.unwrap();

    assert!(user.was_changed("is_admin"));
    assert!(!user.was_changed("name"));
    assert!(!user.was_changed("email"));
    assert!(!user.was_changed_any(&["name", "email"]));
    assert!(user.was_changed_any(&["email", "is_admin"]));
    assert!(user.was_changed_any(&[]));
    assert_eq!(
        sorted_changes(&user.get_changes()),
        vec![("is_admin".to_string(), json!(1))]
    );
}

#[tokio::test]
async fn after_the_save_returns_the_original_is_the_saved_value() {
    let _db = sqlite().await;
    let mut user = plain("Ada", "synced@example.com").await;
    user.is_admin = true;
    user.save().await.unwrap();

    assert_eq!(
        user.get_original("is_admin").unwrap(),
        Some(json!(true)),
        "once the save returns, the original is what it saved"
    );
    assert_eq!(user.get_raw_original("is_admin"), Some(json!(1)));
    assert_eq!(user.get_original("name").unwrap(), Some(json!("Ada")));
}

#[tokio::test]
async fn update_returns_a_model_that_reports_the_change() {
    let _db = sqlite().await;
    let user = plain("Ada", "update@example.com").await;
    let updated = user.update(attrs! { name: "Ada Lovelace" }).await.unwrap();

    assert!(updated.was_changed("name"));
    assert!(!updated.was_changed("is_admin"));
    assert_eq!(
        sorted_changes(&updated.get_changes()),
        vec![("name".to_string(), json!("Ada Lovelace"))]
    );
    assert_eq!(
        updated.get_raw_original("name"),
        Some(json!("Ada Lovelace"))
    );
    assert_eq!(
        updated.get_original("name").unwrap(),
        Some(json!("Ada Lovelace"))
    );
}

#[tokio::test]
async fn a_later_save_replaces_the_record_of_the_earlier_one() {
    let _db = sqlite().await;
    let mut user = plain("Ada", "first@example.com").await;
    user.name = "Ada Lovelace".into();
    user.save().await.unwrap();
    assert!(user.was_changed("name"));

    user.email = "second@example.com".into();
    user.save().await.unwrap();

    assert!(user.was_changed("email"));
    assert!(
        !user.was_changed("name"),
        "the second save did not change `name`; the first save's record is gone"
    );
    assert_eq!(
        sorted_changes(&user.get_changes()),
        vec![("email".to_string(), json!("second@example.com"))]
    );
    assert_eq!(
        user.get_original("email").unwrap(),
        Some(json!("second@example.com"))
    );
    assert_eq!(
        user.get_original("name").unwrap(),
        Some(json!("Ada Lovelace"))
    );
}

#[tokio::test]
async fn a_save_that_changes_nothing_keeps_the_previous_changes() {
    let _db = sqlite().await;
    let mut user = plain("Ada", "noop@example.com").await;
    user.name = "Ada Lovelace".into();
    user.save().await.unwrap();
    assert!(user.was_changed("name"));

    user.save().await.unwrap();

    assert!(
        user.was_changed("name"),
        "a save with nothing dirty leaves the previous save's changes in place"
    );
    assert_eq!(
        sorted_changes(&user.get_changes()),
        vec![("name".to_string(), json!("Ada Lovelace"))]
    );
    assert_eq!(
        user.get_original("name").unwrap(),
        Some(json!("Ada Lovelace"))
    );
}

#[tokio::test]
async fn a_loaded_model_reports_no_change_and_its_loaded_values() {
    let _db = sqlite().await;
    let mut user = plain("Ada", "loaded@example.com").await;

    assert!(!user.was_changed_any(&[]));
    assert!(user.get_changes().is_empty());
    assert_eq!(user.get_original("name").unwrap(), Some(json!("Ada")));

    // A change in memory is not a save: the original stays as loaded.
    user.name = "Someone Else".into();
    user.is_admin = true;
    assert!(!user.was_changed("name"));
    assert_eq!(user.get_original("name").unwrap(), Some(json!("Ada")));
    assert_eq!(user.get_raw_original("is_admin"), Some(json!(0)));
}

#[tokio::test]
async fn a_created_model_reports_no_change() {
    let _db = sqlite().await;
    let user = PlainUser::create(attrs! {
        name: "Ada",
        email: "created@example.com",
        is_admin: true,
    })
    .await
    .unwrap();

    assert!(!user.was_changed_any(&[]));
    assert!(!user.was_changed("is_admin"));
    assert!(user.get_changes().is_empty());
    assert_eq!(user.get_raw_original("is_admin"), Some(json!(1)));
    assert_eq!(user.get_original("is_admin").unwrap(), Some(json!(true)));
}

#[tokio::test]
async fn an_unknown_attribute_reports_nothing() {
    let _db = sqlite().await;
    let mut user = plain("Ada", "unknown@example.com").await;
    user.is_admin = true;
    user.save().await.unwrap();

    assert!(!user.was_changed("no_such_column"));
    assert!(!user.was_changed_any(&["no_such_column"]));
    assert_eq!(user.get_raw_original("no_such_column"), None);
    assert_eq!(user.get_original("no_such_column").unwrap(), None);
}

#[tokio::test]
async fn a_save_with_timestamps_also_reports_updated_at() {
    let _db = sqlite().await;
    let clock = TestClock::freeze();
    let created = ChangePost::create(attrs! { title: "Draft", body: "Text" })
        .await
        .unwrap();
    let mut post = ChangePost::find_or_fail(created.id).await.unwrap();
    clock.advance(Duration::seconds(5));
    post.title = "Final".into();
    post.save().await.unwrap();

    let keys: Vec<String> = sorted_changes(&post.get_changes())
        .into_iter()
        .map(|(k, _)| k)
        .collect();
    assert_eq!(keys, vec!["title".to_string(), "updated_at".to_string()]);
    assert!(post.was_changed("updated_at"));
    assert!(!post.was_changed("created_at"));
}

#[tokio::test]
async fn an_encrypted_column_is_changed_only_when_its_value_changes() {
    suprnova::testing::install_test_encryption_key();
    let db = sqlite().await;
    db.execute_unprepared(
        "CREATE TABLE par_secret_users (\
            id INTEGER PRIMARY KEY AUTOINCREMENT, \
            secret TEXT NOT NULL, \
            is_admin INTEGER NOT NULL)",
    )
    .await
    .unwrap();
    let created = SecretUser::create(attrs! { secret: "first secret", is_admin: false })
        .await
        .unwrap();
    let mut user = SecretUser::find_or_fail(created.id).await.unwrap();

    user.is_admin = true;
    user.save().await.unwrap();
    assert!(
        !user.was_changed("secret"),
        "the save stored a new ciphertext of the same secret: {:?}",
        sorted_changes(&user.get_changes())
    );
    assert_eq!(
        sorted_changes(&user.get_changes()),
        vec![("is_admin".to_string(), json!(1))]
    );

    user.secret = "second secret".into();
    user.save().await.unwrap();
    assert!(user.was_changed("secret"), "a new secret is a change");
    assert!(!user.was_changed("is_admin"));
    let keys: Vec<String> = sorted_changes(&user.get_changes())
        .into_iter()
        .map(|(k, _)| k)
        .collect();
    assert_eq!(keys, vec!["secret".to_string()]);
}

#[tokio::test]
async fn a_model_not_read_from_the_database_reports_every_column_as_changed() {
    let _db = sqlite().await;
    let loaded = plain("Ada", "unread@example.com").await;

    // Built in memory, never read: there is no loaded row to compare with.
    let user = PlainUser {
        id: loaded.id,
        name: "Ada".into(),
        email: "unread@example.com".into(),
        is_admin: true,
        ..Default::default()
    };
    assert_eq!(user.get_raw_original("is_admin"), None);

    user.save().await.unwrap();

    for column in ["name", "email", "is_admin"] {
        assert!(user.was_changed(column), "{column} counts as changed");
    }
    assert_eq!(
        user.get_raw_original("is_admin"),
        Some(json!(1)),
        "after the save the original is the saved row"
    );
}

#[tokio::test]
async fn save_with_tx_and_update_with_tx_record_the_change() {
    let _db = sqlite().await;
    let mut first = plain("Ada", "tx-save@example.com").await;
    let second = plain("Grace", "tx-update@example.com").await;

    let tx = DB::begin_transaction().await.unwrap();
    first.is_admin = true;
    first.save_with_tx(&tx).await.unwrap();
    let second = second
        .update_with_tx(&tx, attrs! { name: "Grace Hopper" })
        .await
        .unwrap();
    tx.commit().await.unwrap();

    assert_eq!(
        sorted_changes(&first.get_changes()),
        vec![("is_admin".to_string(), json!(1))]
    );
    assert_eq!(first.get_raw_original("is_admin"), Some(json!(1)));
    assert_eq!(
        sorted_changes(&second.get_changes()),
        vec![("name".to_string(), json!("Grace Hopper"))]
    );
    assert_eq!(
        second.get_original("name").unwrap(),
        Some(json!("Grace Hopper"))
    );
}

#[tokio::test]
async fn a_failed_save_keeps_the_record_of_the_last_successful_one() {
    let _db = sqlite().await;
    let _taken = plain("Grace", "taken@example.com").await;
    let mut user = plain("Ada", "mine@example.com").await;
    user.name = "Ada Lovelace".into();
    user.save().await.unwrap();

    // `email` is UNIQUE: this save fails in the database.
    user.email = "taken@example.com".into();
    user.save()
        .await
        .expect_err("the unique constraint refuses the write");

    assert!(user.was_changed("name"), "the earlier save's record stands");
    assert!(!user.was_changed("email"));
    assert_eq!(
        user.get_original("email").unwrap(),
        Some(json!("mine@example.com"))
    );
}

#[tokio::test]
async fn a_clone_keeps_the_record_and_a_replica_does_not() {
    let _db = sqlite().await;
    let mut user = plain("Ada", "clone@example.com").await;
    user.is_admin = true;
    user.save().await.unwrap();

    let copy = user.clone();
    assert!(copy.was_changed("is_admin"));
    assert_eq!(copy.get_raw_original("is_admin"), Some(json!(1)));

    let replica = user.replicate().await.unwrap();
    assert!(!replica.was_changed_any(&[]));
    assert_eq!(replica.get_raw_original("is_admin"), None);
}

// ---- Live engines -------------------------------------------------------

/// One connection, so the temporary table below is visible to every
/// statement the scenario runs.
async fn connect_live(env: &str) -> (TestContainerGuard, DbConnection) {
    let url = std::env::var(env).expect("explicit disposable database URL required");
    let guard = TestContainer::fake();
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
    (guard, database)
}

async fn live_save_reports_its_changes(env: &str) {
    use sea_orm::ConnectionTrait;

    let (guard, database) = connect_live(env).await;
    let id_column = match database.inner().get_database_backend() {
        sea_orm::DatabaseBackend::Postgres => "id BIGSERIAL PRIMARY KEY",
        _ => "id BIGINT AUTO_INCREMENT PRIMARY KEY",
    };
    database
        .inner()
        .execute_unprepared(&format!(
            "CREATE TEMPORARY TABLE par_live_change_users ({id_column}, \
             name VARCHAR(255) NOT NULL, email VARCHAR(255) NOT NULL, \
             is_admin BIGINT NOT NULL)"
        ))
        .await
        .expect("create isolated temporary table");
    LiveChangeUser::observe(LiveAuditObserver).await;
    let email = format!("{}-live@example.com", env.to_lowercase());

    let created =
        LiveChangeUser::create(attrs! { name: "Ada", email: email.clone(), is_admin: false })
            .await
            .unwrap();
    let mut user = LiveChangeUser::find_or_fail(created.id).await.unwrap();
    user.is_admin = true;
    user.save().await.unwrap();

    let seen = seen_for("updated", &email);
    assert!(!seen.is_empty(), "the updated observer ran");
    for record in &seen {
        assert_saw_only_the_flip(record);
    }
    assert_eq!(
        sorted_changes(&user.get_changes()),
        vec![("is_admin".to_string(), json!(1))]
    );
    assert_eq!(user.get_raw_original("is_admin"), Some(json!(1)));

    // Nothing dirty: the flip stays the last change.
    user.save().await.unwrap();
    assert!(user.was_changed("is_admin"));

    let user = user.update(attrs! { name: "Ada Lovelace" }).await.unwrap();
    assert_eq!(
        sorted_changes(&user.get_changes()),
        vec![("name".to_string(), json!("Ada Lovelace"))]
    );
    assert_eq!(
        user.get_original("name").unwrap(),
        Some(json!("Ada Lovelace"))
    );

    drop(guard);
    database.inner().clone().close().await.unwrap();
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL at PG_TEST_URL"]
async fn postgres_save_reports_its_changes() {
    live_save_reports_its_changes("PG_TEST_URL").await;
}

#[tokio::test]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_save_reports_its_changes() {
    live_save_reports_its_changes("MYSQL_TEST_URL").await;
}
