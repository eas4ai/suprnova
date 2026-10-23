//! LDB-001: the tables the framework and the scaffold use under Laravel's
//! names take Laravel's layouts, and work on a database Laravel created.
//! LDB-011: their migrations leave a table Laravel created as Laravel left
//! it, and the tables they create use `DATETIME`, never `TIMESTAMP`, on
//! MySQL.

use std::any::Any;
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use sea_orm::{ConnectionTrait, DbBackend};
use sea_orm_migration::{MigrationTrait as _, SchemaManager};
use suprnova::eloquent::{HasUniqueId, UniqueIdKind};
use suprnova::session::migrations::{CreateSessionsTable, SessionUserKey, create_sessions_table};
use suprnova::{
    Auth, AuthConfig, AuthManager, Authenticatable, Batch, BatchOptions, BatchRepository,
    DatabaseBatchRepository, DatabaseQueueDriver, FrameworkError, HttpResponse, Model, QueueDriver,
    Request, Router, UserProvider, attrs, model,
};

use crate::browser::{self, Browser};
use crate::catalog::{self, Shape};
use crate::failed_jobs::envelope;
use crate::on_every_engine;
use crate::support::{self, Engine};

/// Every table LDB-001 names that the framework or the scaffold creates,
/// with spatie's assignment tables, which the RBAC writes.
const LAYOUT_TABLES: [&str; 12] = [
    "failed_jobs",
    "jobs",
    "job_batches",
    "sessions",
    "notifications",
    "users",
    "features",
    "roles",
    "permissions",
    "model_has_roles",
    "model_has_permissions",
    "role_has_permissions",
];

/// Whether two column types are the same column, allowing the one
/// difference LDB-001 makes: `DATETIME` where Laravel says `TIMESTAMP` on
/// MySQL.
fn same_kind(engine: Engine, laravel: &str, ours: &str) -> bool {
    laravel == ours || (engine == Engine::Mysql && laravel == "timestamp" && ours == "datetime")
}

/// Assert `ours` is `laravel` up to LDB-001's one exception.
pub(crate) fn assert_same_layout(engine: Engine, table: &str, laravel: &Shape, ours: &Shape) {
    let names = |shape: &Shape| {
        shape
            .columns
            .iter()
            .map(|c| c.name.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        names(ours),
        names(laravel),
        "{engine:?} {table}: the columns differ"
    );
    for (theirs, mine) in laravel.columns.iter().zip(&ours.columns) {
        assert!(
            same_kind(engine, &theirs.kind, &mine.kind),
            "{engine:?} {table}.{}: Laravel's type is {}, ours is {}",
            theirs.name,
            theirs.kind,
            mine.kind
        );
        assert_eq!(
            theirs.nullable, mine.nullable,
            "{engine:?} {table}.{}: nullability differs",
            theirs.name
        );
    }
    assert_eq!(
        ours.primary, laravel.primary,
        "{engine:?} {table}: primary key"
    );
    assert_eq!(ours.indexes, laravel.indexes, "{engine:?} {table}: indexes");
    assert_eq!(
        ours.foreign_keys, laravel.foreign_keys,
        "{engine:?} {table}: foreign keys"
    );
}

pub(crate) async fn laravel_shapes(engine: Engine) -> BTreeMap<&'static str, Shape> {
    let (db, _) = support::laravel_schema(engine).await;
    let mut shapes = BTreeMap::new();
    for table in LAYOUT_TABLES {
        shapes.insert(table, catalog::shape(&db.conn, table).await);
    }
    shapes
}

/// On an empty database, every table the framework's and the scaffold's
/// migrations create under a name in LDB-001 has Laravel's layout.
async fn created_tables_take_laravels_layout(engine: Engine) {
    let laravel = laravel_shapes(engine).await;
    let db = support::empty(engine).await;
    crate::scaffold::migrate(&db.conn)
        .await
        .expect("the framework's and the scaffold's migrations run");
    for table in LAYOUT_TABLES {
        let ours = catalog::shape(&db.conn, table).await;
        assert_same_layout(engine, table, &laravel[table], &ours);
    }
}

on_every_engine!(created_tables_take_laravels_layout =>
    ldb_001_tables_suprnova_creates_take_laravels_layout_sqlite,
    ldb_001_tables_suprnova_creates_take_laravels_layout_postgres,
    ldb_001_tables_suprnova_creates_take_laravels_layout_mysql);

/// The type Laravel's grammar gives a `uuid` or `ulid` column on `engine`
/// (`typeUuid`, `typeChar`), normalized as the catalog normalizes.
pub(crate) fn laravel_key_kind(engine: Engine, key: &str) -> &'static str {
    match (engine, key) {
        (Engine::Sqlite, _) => "text",
        (Engine::Postgres, "uuid") => "uuid",
        (Engine::Postgres, _) => "character(26)",
        (Engine::Mysql, "uuid") => "char(36)",
        (Engine::Mysql, _) => "char(26)",
    }
}

/// `notifications.notifiable_id` follows `NOTIFICATIONS_MORPH_KEY` as
/// Laravel's `morphs`, `uuidMorphs` and `ulidMorphs` do, and
/// `sessions.user_id` follows the user model's key as `foreignId`,
/// `foreignUuid` and `foreignUlid` do. Everything else in the two tables
/// stays Laravel's.
async fn keyed_columns_follow_the_key_type(engine: Engine) {
    let laravel = laravel_shapes(engine).await;
    for (morph_key, session_key) in [
        ("uuid", SessionUserKey::Uuid),
        ("ulid", SessionUserKey::Ulid),
        ("int", SessionUserKey::Integer),
    ] {
        let db = support::empty(engine).await;
        // SAFETY: the test is serial within its process, and nothing else
        // reads the environment while the migration runs.
        unsafe { std::env::set_var("NOTIFICATIONS_MORPH_KEY", morph_key) };
        let migrated = sea_orm_migration::MigrationTrait::up(
            &suprnova::notifications::migrations::CreateNotificationsTable,
            &SchemaManager::new(&db.conn),
        )
        .await;
        // SAFETY: as above.
        unsafe { std::env::remove_var("NOTIFICATIONS_MORPH_KEY") };
        migrated.expect("the notifications migration runs");
        create_sessions_table(&SchemaManager::new(&db.conn), "sessions", session_key)
            .await
            .expect("the sessions table");

        for (table, column) in [("notifications", "notifiable_id"), ("sessions", "user_id")] {
            let mut expected = laravel[table].clone();
            if morph_key != "int" {
                let kind = laravel_key_kind(engine, morph_key).to_owned();
                expected
                    .columns
                    .iter_mut()
                    .find(|c| c.name == column)
                    .expect("the keyed column")
                    .kind = kind;
            }
            let ours = catalog::shape(&db.conn, table).await;
            assert_same_layout(engine, &format!("{table} ({morph_key})"), &expected, &ours);
        }
    }
}

on_every_engine!(keyed_columns_follow_the_key_type =>
    ldb_001_notifiable_id_and_user_id_follow_the_key_type_sqlite,
    ldb_001_notifiable_id_and_user_id_follow_the_key_type_postgres,
    ldb_001_notifiable_id_and_user_id_follow_the_key_type_mysql);

/// A user model keyed by an auto-incrementing integer, Laravel's default.
#[model(table = "ldb_int_users", fillable = ["name"])]
pub struct LdbIntUser {
    pub id: u64,
    pub name: String,
}

/// A user model with a UUID key (`HasUuids`).
#[model(
    table = "ldb_uuid_users",
    primary_key = "id",
    key_type = "String",
    auto_increment = false,
    unique_id = "uuid",
    fillable = ["name"]
)]
pub struct LdbUuidUser {
    pub id: String,
    pub name: String,
}

/// A user model with a ULID key (`HasUlids`).
#[model(
    table = "ldb_ulid_users",
    primary_key = "id",
    key_type = "String",
    auto_increment = false,
    unique_id = "ulid",
    fillable = ["name"]
)]
pub struct LdbUlidUser {
    pub id: String,
    pub name: String,
}

macro_rules! authenticatable {
    ($($model:ty),*) => {$(
        impl Authenticatable for $model {
            fn get_auth_identifier(&self) -> String {
                self.id.to_string()
            }

            fn as_any(&self) -> &dyn Any {
                self
            }

            fn into_arc_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
                self
            }
        }
    )*};
}

authenticatable!(LdbIntUser, LdbUuidUser, LdbUlidUser);

/// The `sessions.user_id` a `unique_id` key takes, as the application
/// picks it from its user model's key: `foreignUuid` for a UUID,
/// `foreignUlid` for a ULID.
fn session_key<M: HasUniqueId>() -> SessionUserKey {
    match M::UNIQUE_ID_KIND {
        UniqueIdKind::UuidV7 | UniqueIdKind::UuidV4 => SessionUserKey::Uuid,
        UniqueIdKind::Ulid => SessionUserKey::Ulid,
    }
}

type Found = Result<Option<Arc<dyn Authenticatable>>, FrameworkError>;

/// The `users` provider of the default guard: one of the models above,
/// looked up by its key.
struct ModelUsers(fn(String) -> Pin<Box<dyn Future<Output = Found> + Send>>);

#[async_trait::async_trait]
impl UserProvider for ModelUsers {
    async fn retrieve_by_id(&self, id: &str) -> Found {
        (self.0)(id.to_owned()).await
    }

    async fn retrieve_by_credentials(&self, _credentials: &serde_json::Value) -> Found {
        Ok(None)
    }

    async fn validate_credentials(
        &self,
        _user: &dyn Authenticatable,
        _credentials: &serde_json::Value,
    ) -> Result<bool, FrameworkError> {
        Ok(false)
    }
}

fn found<M: Authenticatable + 'static>(user: Option<M>) -> Found {
    Ok(user.map(|user| Arc::new(user) as Arc<dyn Authenticatable>))
}

/// `/login` signs in, through the default guard, the user the `x-user`
/// header names; `/whoami` answers who the session says is signed in.
fn sign_in_router() -> Router {
    Router::new()
        .get("/login", |request: Request| async move {
            match Auth::login_using_id(&browser::header(&request, "x-user"), false).await {
                Ok(Some(user)) => Ok(HttpResponse::text(user.get_auth_identifier())),
                Ok(None) => Ok(HttpResponse::text("no such user").status(404)),
                Err(error) => {
                    Err(HttpResponse::text(error.to_string()).status(error.status_code()))
                }
            }
        })
        .get("/whoami", |_request: Request| async {
            Ok(HttpResponse::text(
                Auth::id().unwrap_or_else(|| "guest".to_owned()),
            ))
        })
        .into()
}

/// One user model's half of [`sessions_follow_the_user_model`]: its
/// `users` table, the shipped sessions migration for its key, and a sign-in
/// of a user the model created, through the session driver.
async fn sign_in_with(
    engine: Engine,
    laravel_sessions: &Shape,
    users_key: &str,
    key: SessionUserKey,
    lookup: fn(String) -> Pin<Box<dyn Future<Output = Found> + Send>>,
    create: impl AsyncFnOnce() -> String,
) {
    let db = support::empty(engine).await;
    db.conn
        .execute_unprepared(&format!(
            "CREATE TABLE ldb_{}_users (id {users_key}, name VARCHAR(255) NOT NULL)",
            match key {
                SessionUserKey::Integer => "int",
                SessionUserKey::Uuid => "uuid",
                SessionUserKey::Ulid => "ulid",
            }
        ))
        .await
        .expect("the users table");
    CreateSessionsTable::new(key)
        .up(&SchemaManager::new(&db.conn))
        .await
        .expect("the shipped sessions migration");

    let mut expected = laravel_sessions.clone();
    let kind = match key {
        SessionUserKey::Integer => None,
        SessionUserKey::Uuid => Some(laravel_key_kind(engine, "uuid")),
        SessionUserKey::Ulid => Some(laravel_key_kind(engine, "ulid")),
    };
    if let Some(kind) = kind {
        expected
            .columns
            .iter_mut()
            .find(|c| c.name == "user_id")
            .expect("user_id")
            .kind = kind.to_owned();
    }
    let ours = catalog::shape(&db.conn, "sessions").await;
    assert_same_layout(
        engine,
        &format!("sessions ({key:?} user)"),
        &expected,
        &ours,
    );

    let _bound = support::bind(&db.conn);
    let _ = suprnova::crypto::_test_install_key(suprnova::EncryptionKey::generate());
    suprnova::testing::TestContainer::singleton(AuthManager::new(AuthConfig::default()));
    Auth::register_provider("users", Arc::new(ModelUsers(lookup)))
        .expect("register the users provider");
    let id = create().await;

    let mut browser = Browser::serve(sign_in_router()).await;
    assert_eq!(
        browser.get("/login", &[("x-user", id.as_str())]).await,
        (200, id.clone()),
        "{engine:?} {key:?}: the sign-in failed"
    );
    assert_eq!(
        browser.get("/whoami", &[]).await,
        (200, id.clone()),
        "{engine:?} {key:?}: the session does not sign its user in"
    );
    let stored = support::rows(&db.conn, "SELECT user_id FROM sessions").await;
    assert_eq!(stored.len(), 1, "{engine:?} {key:?}: one session");
    assert_eq!(
        support::text(&stored[0], "user_id"),
        id,
        "{engine:?} {key:?}: sessions.user_id does not hold the user's key"
    );
    // Revoking the user's sessions finds the row by its `user_id`.
    suprnova::session::destroy_all_for_user(&id)
        .await
        .expect("revoke the user's sessions");
    assert_eq!(
        browser.get("/whoami", &[]).await,
        (200, "guest".to_owned()),
        "{engine:?} {key:?}: the revoked session still signs its user in"
    );
}

/// `sessions.user_id` with an integer-keyed and a `unique_id`-keyed user
/// model: the shipped migration, given the key the model has, creates the
/// column Laravel's `foreignId`, `foreignUuid` or `foreignUlid` creates, and
/// a user the model created signs in, is stored in that column and is
/// signed out by a revocation of their sessions.
async fn sessions_follow_the_user_model(engine: Engine) {
    let laravel = laravel_shapes(engine).await;
    let sessions = &laravel["sessions"];
    let integer_key = match engine {
        Engine::Sqlite => "INTEGER PRIMARY KEY AUTOINCREMENT",
        Engine::Postgres => "BIGSERIAL PRIMARY KEY",
        Engine::Mysql => "BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY",
    };
    sign_in_with(
        engine,
        sessions,
        integer_key,
        SessionUserKey::Integer,
        |id| {
            Box::pin(async move {
                match id.parse::<u64>() {
                    Ok(key) => found(LdbIntUser::find(key).await?),
                    Err(_) => Ok(None),
                }
            })
        },
        async || {
            let user = LdbIntUser::create(attrs! { name: "Taylor" })
                .await
                .expect("create an integer-keyed user");
            user.id.to_string()
        },
    )
    .await;
    sign_in_with(
        engine,
        sessions,
        "CHAR(36) PRIMARY KEY",
        session_key::<LdbUuidUser>(),
        |id| Box::pin(async move { found(LdbUuidUser::find(id).await?) }),
        async || {
            LdbUuidUser::create(attrs! { name: "Ada" })
                .await
                .expect("create a UUID-keyed user")
                .id
        },
    )
    .await;
    sign_in_with(
        engine,
        sessions,
        "CHAR(26) PRIMARY KEY",
        session_key::<LdbUlidUser>(),
        |id| Box::pin(async move { found(LdbUlidUser::find(id).await?) }),
        async || {
            LdbUlidUser::create(attrs! { name: "Grace" })
                .await
                .expect("create a ULID-keyed user")
                .id
        },
    )
    .await;
}

on_every_engine!(sessions_follow_the_user_model =>
    ldb_001_sessions_user_id_follows_an_integer_and_a_unique_id_keyed_user_model_sqlite,
    ldb_001_sessions_user_id_follows_an_integer_and_a_unique_id_keyed_user_model_postgres,
    ldb_001_sessions_user_id_follows_an_integer_and_a_unique_id_keyed_user_model_mysql);

/// On the tables Laravel created, with the rows Laravel wrote, the jobs
/// driver, the batch repository, the session driver and the database
/// notification channel all work.
async fn stores_work_on_laravels_tables(engine: Engine) {
    let (db, fixture) = support::laravel(engine).await;
    // The framework's migrations add only tables of its own names here.
    crate::scaffold::migrate(&db.conn)
        .await
        .expect("migrate on Laravel's database");
    let _bound = support::bind(&db.conn);

    // jobs: push, pop, ack, and Laravel's own queued rows left alone.
    let laravel_jobs = support::count(&db.conn, "jobs", "").await;
    let jobs = DatabaseQueueDriver::new(db.conn.clone(), "jobs".to_owned()).unwrap();
    jobs.push(envelope("Ldb.Ship", serde_json::json!({ "order": 7 })))
        .await
        .expect("push onto Laravel's jobs table");
    let popped = jobs
        .pop(Duration::from_secs(30))
        .await
        .expect("pop")
        .expect("the job just pushed, not one Laravel queued");
    assert_eq!(popped.envelope.job_name, "Ldb.Ship");
    jobs.ack(&popped.token).await.expect("ack");
    assert!(jobs.pop(Duration::from_secs(30)).await.unwrap().is_none());
    assert_eq!(
        support::count(&db.conn, "jobs", "").await,
        laravel_jobs,
        "{engine:?}: Laravel's queued jobs were touched"
    );

    // job_batches: store, settle, read back.
    let batches = DatabaseBatchRepository::new(db.conn.clone());
    let batch = Batch {
        id: uuid::Uuid::new_v4().to_string(),
        name: "ldb".into(),
        total_jobs: 2,
        pending_jobs: 2,
        failed_jobs: 0,
        failed_job_ids: Vec::new(),
        options: BatchOptions::default(),
        created_at: chrono::Utc::now(),
        cancelled_at: None,
        finished_at: None,
    };
    batches.store(batch.clone()).await.expect("store a batch");
    batches
        .record_failed_job(&batch.id, uuid::Uuid::new_v4())
        .await
        .expect("settle a job");
    let read = batches.find(&batch.id).await.unwrap().expect("the batch");
    assert_eq!((read.pending_jobs, read.failed_jobs), (1, 1));
    let laravel_batch = support::rows(&db.conn, "SELECT id FROM job_batches").await;
    assert_eq!(laravel_batch.len(), 2, "Laravel's batch and ours");

    // sessions: write, read, and Laravel's session rows untouched.
    use suprnova::session::SessionStore;
    let driver = suprnova::session::driver::DatabaseSessionDriver::new(Duration::from_secs(7200));
    let mut session =
        suprnova::session::SessionData::new(format!("{:0<40}", "ldbsession"), "csrf-token".into());
    session.user_id = Some("1".into());
    session
        .data
        .insert("cart".into(), serde_json::json!(["book"]));
    driver.write(&session).await.expect("write a session");
    let read = driver
        .read(&session.id)
        .await
        .unwrap()
        .expect("the session");
    assert_eq!(read.csrf_token, "csrf-token");
    assert_eq!(read.user_id.as_deref(), Some("1"));
    assert_eq!(read.data["cart"], serde_json::json!(["book"]));
    assert!(
        driver
            .read(&fixture.sessions.guest)
            .await
            .unwrap()
            .is_none(),
        "Laravel's idle session is past this driver's lifetime"
    );

    // notifications: deliver one, read Laravel's and ours.
    use suprnova::notifications::{Channel, all_for};
    let channel = suprnova::DatabaseChannel::new(db.conn.clone(), "App\\Models\\User");
    channel
        .deliver("1", &crate::support::Shipped)
        .await
        .expect("deliver into Laravel's notifications table");
    let inbox = all_for(&db.conn, "App\\Models\\User", "1")
        .await
        .expect("read the inbox");
    assert_eq!(
        inbox.len(),
        3,
        "{engine:?}: Laravel's two notifications and ours"
    );
    assert!(
        inbox
            .iter()
            .any(|n| n.type_name == "App\\Notifications\\InvoicePaid")
    );
    assert!(inbox.iter().any(|n| n.type_name == "Shipped"));
}

on_every_engine!(stores_work_on_laravels_tables =>
    ldb_001_queue_session_and_notification_stores_work_on_laravels_tables_sqlite,
    ldb_001_queue_session_and_notification_stores_work_on_laravels_tables_postgres,
    ldb_001_queue_session_and_notification_stores_work_on_laravels_tables_mysql);

/// After the framework's and a fresh scaffold's migrations run on the
/// Laravel 13 schema and rows, `migrate` has succeeded and every table
/// Laravel or a package created is as it was: columns, types,
/// nullability, defaults, keys, indexes and rows. A second `migrate`
/// changes nothing either.
async fn migrations_leave_laravels_tables_alone(engine: Engine) {
    let (db, _) = support::laravel(engine).await;
    let laravel_tables = catalog::tables(&db.conn).await;
    let mut before = BTreeMap::new();
    for table in &laravel_tables {
        before.insert(
            table.clone(),
            (
                catalog::shape(&db.conn, table).await,
                support::count(&db.conn, table, "").await,
            ),
        );
    }
    crate::scaffold::migrate(&db.conn)
        .await
        .expect("migrate succeeds on Laravel's database");
    crate::scaffold::migrate(&db.conn)
        .await
        .expect("a second migrate succeeds");
    for table in &laravel_tables {
        let after = (
            catalog::shape(&db.conn, table).await,
            support::count(&db.conn, table, "").await,
        );
        assert_eq!(
            after, before[table],
            "{engine:?}: the migrations changed Laravel's {table}"
        );
    }
}

on_every_engine!(migrations_leave_laravels_tables_alone =>
    ldb_011_migrations_leave_laravels_tables_as_laravel_left_them_sqlite,
    ldb_011_migrations_leave_laravels_tables_as_laravel_left_them_postgres,
    ldb_011_migrations_leave_laravels_tables_as_laravel_left_them_mysql);

/// On MySQL, a table Suprnova creates under a name in LDB-001 holds no
/// `TIMESTAMP` column.
#[tokio::test]
#[serial_test::serial]
#[ignore = "requires a throwaway MySQL at MYSQL_TEST_URL"]
async fn ldb_011_tables_suprnova_creates_have_no_timestamp_column_mysql() {
    let db = support::empty(Engine::Mysql).await;
    crate::scaffold::migrate(&db.conn).await.expect("migrate");
    assert_eq!(db.conn.get_database_backend(), DbBackend::MySql);
    for table in LAYOUT_TABLES {
        let shape = catalog::shape(&db.conn, table).await;
        for column in &shape.columns {
            assert_ne!(
                column.kind, "timestamp",
                "{table}.{} is TIMESTAMP, which refuses any time after 2038-01-19",
                column.name
            );
        }
    }
}

/// Rolling every migration back leaves each table Laravel or a package
/// created, with its rows: the scaffold's `users` and `sessions` migrations
/// skipped those tables on the way up, so their `down` must not drop them.
async fn rollback_leaves_laravels_tables(engine: Engine) {
    let (db, _) = support::laravel(engine).await;
    let laravel_tables = catalog::tables(&db.conn).await;
    let mut before = BTreeMap::new();
    for table in &laravel_tables {
        before.insert(
            table.clone(),
            (
                catalog::shape(&db.conn, table).await,
                support::count(&db.conn, table, "").await,
            ),
        );
    }
    crate::scaffold::migrate(&db.conn)
        .await
        .expect("migrate on Laravel's database");
    <crate::scaffold::Migrator as sea_orm_migration::MigratorTrait>::down(&db.conn, None)
        .await
        .expect("roll every migration back");
    let left = catalog::tables(&db.conn).await;
    for table in &laravel_tables {
        assert!(
            left.contains(table),
            "{engine:?}: the rollback dropped Laravel's {table}"
        );
        let after = (
            catalog::shape(&db.conn, table).await,
            support::count(&db.conn, table, "").await,
        );
        assert_eq!(
            after, before[table],
            "{engine:?}: the rollback changed Laravel's {table}"
        );
    }
}

on_every_engine!(rollback_leaves_laravels_tables =>
    ldb_011_a_rollback_leaves_laravels_tables_and_rows_sqlite,
    ldb_011_a_rollback_leaves_laravels_tables_and_rows_postgres,
    ldb_011_a_rollback_leaves_laravels_tables_and_rows_mysql);
