//! LDB-001: the tables the framework and the scaffold use under Laravel's
//! names take Laravel's layouts, and work on a database Laravel created.
//! LDB-011: their migrations leave a table Laravel created as Laravel left
//! it, and the tables they create use `DATETIME`, never `TIMESTAMP`, on
//! MySQL.

use std::collections::BTreeMap;
use std::time::Duration;

use sea_orm::{ConnectionTrait, DbBackend};
use sea_orm_migration::SchemaManager;
use suprnova::session::migrations::{SessionUserKey, create_sessions_table};
use suprnova::{
    Batch, BatchOptions, BatchRepository, DatabaseBatchRepository, DatabaseQueueDriver, QueueDriver,
};

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
fn laravel_key_kind(engine: Engine, key: &str) -> &'static str {
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
    let mut session = suprnova::session::SessionData::new(
        format!("{:0<40}", "ldbsession"),
        "csrf-token".into(),
    );
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
