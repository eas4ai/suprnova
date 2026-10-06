//! LDB-010: the framework's shipped migrations upgrade an existing
//! Suprnova application in place. On a database holding the layouts an
//! earlier release created, with rows, `migrate` moves every queued,
//! delayed, reserved and failed job, batch, session, notification, flag,
//! role, permission and assignment into the layouts of LDB-001, and each
//! keeps working.
//!
//! The earlier tables are built here with the statements the earlier
//! migrations ran: the dogfood application's `m_2026_08_01_queue_tables`
//! for `jobs` and `failed_jobs`, the batch repository's documented schema,
//! the earlier scaffold's sessions migration, and the earlier framework
//! migrations for `notifications`, `features` and the RBAC tables.

use std::time::Duration;

use chrono::Utc;
use sea_orm::{ConnectionTrait, DatabaseConnection};
use sea_orm_migration::prelude::{Alias, ColumnDef, Expr, Index, Table};
use sea_orm_migration::sea_query::{Query, SimpleExpr};
use suprnova::{
    Auth, BatchRepository, DatabaseBatchRepository, DatabaseFailedJobStore, DatabaseQueueDriver,
    FailedJobStore, HttpResponse, Queue, QueueDriver, Request, ReservationToken, Router,
};
use uuid::Uuid;

use crate::browser::{Browser, SESSION_COOKIE};
use crate::failed_jobs::envelope;
use crate::on_every_engine;
use crate::support::{self, Engine};

/// The moment the earlier rows were written.
fn now() -> i64 {
    Utc::now().timestamp()
}

fn col(name: &str) -> ColumnDef {
    ColumnDef::new(Alias::new(name))
}

async fn create(
    conn: &DatabaseConnection,
    table: sea_orm_migration::sea_query::TableCreateStatement,
) {
    conn.execute(&table).await.expect("create an earlier table");
}

async fn insert(conn: &DatabaseConnection, table: &str, columns: &[&str], values: Vec<SimpleExpr>) {
    let mut statement = Query::insert();
    statement
        .into_table(Alias::new(table))
        .columns(columns.iter().map(|c| Alias::new(*c)))
        .values(values)
        .expect("an insert");
    conn.execute(&statement)
        .await
        .unwrap_or_else(|e| panic!("insert into the earlier {table}: {e}"));
}

fn v<T: Into<sea_orm::Value>>(value: T) -> SimpleExpr {
    Expr::value(value.into())
}

/// The ids of the earlier rows the tests follow.
struct Earlier {
    queued: Uuid,
    delayed: Uuid,
    reserved: Uuid,
    reserved_token: Uuid,
    reserved_until: i64,
    delayed_until: i64,
    failed: [Uuid; 2],
    batch: String,
    batch_failed_job: Uuid,
    session: String,
    session_csrf: String,
    notifications: [String; 2],
}

/// Every table in the layout an earlier release created, with rows.
async fn earlier_database(engine: Engine) -> (support::Db, Earlier) {
    let db = support::empty(engine).await;
    let conn = &db.conn;
    let t0 = now();

    // jobs and failed_jobs: the dogfood application's `m_2026_08_01_queue_tables`.
    create(
        conn,
        Table::create()
            .table(Alias::new("jobs"))
            .col(col("id").string().not_null().primary_key())
            .col(col("job_name").string().not_null())
            .col(col("queue").string().null())
            .col(col("envelope_json").text().not_null())
            .col(col("available_at").big_integer().not_null())
            .col(col("reserved_until").big_integer().null())
            .col(col("reserved_token").string().null())
            .col(col("attempts").integer().not_null().default(0))
            .col(col("created_at").big_integer().not_null())
            .to_owned(),
    )
    .await;
    create(
        conn,
        Table::create()
            .table(Alias::new("failed_jobs"))
            .col(col("id").string().not_null().primary_key())
            .col(col("connection").string().not_null())
            .col(col("job_name").string().not_null())
            .col(col("queue").string().not_null())
            .col(col("envelope_json").text().not_null())
            .col(col("exception").text().not_null())
            .col(col("failed_at").big_integer().not_null())
            .to_owned(),
    )
    .await;
    let earlier = Earlier {
        queued: Uuid::new_v4(),
        delayed: Uuid::new_v4(),
        reserved: Uuid::new_v4(),
        reserved_token: Uuid::new_v4(),
        reserved_until: t0 + 300,
        delayed_until: t0 + 200,
        failed: [Uuid::new_v4(), Uuid::new_v4()],
        batch: Uuid::new_v4().to_string(),
        batch_failed_job: Uuid::new_v4(),
        session: format!("{:0<40}", "earliersession"),
        session_csrf: "earlier-csrf-token-0123456789".to_owned(),
        notifications: [Uuid::new_v4().to_string(), Uuid::new_v4().to_string()],
    };
    for (id, queue, available_at, reservation) in [
        (earlier.queued, None, t0 - 10, None),
        (earlier.delayed, Some("mail"), earlier.delayed_until, None),
        (
            earlier.reserved,
            Some("default"),
            t0 - 20,
            Some((earlier.reserved_until, earlier.reserved_token)),
        ),
    ] {
        let mut env = envelope("Ldb.Upgrade", serde_json::json!({ "job": id.to_string() }));
        env.id = id;
        env.attempts = u32::from(reservation.is_some());
        env.queue = queue.map(str::to_owned);
        insert(
            conn,
            "jobs",
            &[
                "id",
                "job_name",
                "queue",
                "envelope_json",
                "available_at",
                "reserved_until",
                "reserved_token",
                "attempts",
                "created_at",
            ],
            vec![
                v(id.to_string()),
                v("Ldb.Upgrade"),
                v(queue.map(str::to_owned)),
                v(env.to_json().expect("envelope JSON")),
                v(available_at),
                v(reservation.map(|(until, _)| until)),
                v(reservation.map(|(_, token)| token.to_string())),
                v(i32::from(reservation.is_some())),
                v(t0 - 30),
            ],
        )
        .await;
    }
    for id in earlier.failed {
        let mut env = envelope(
            "Ldb.Refund",
            serde_json::json!({ "refund": id.to_string() }),
        );
        env.id = id;
        insert(
            conn,
            "failed_jobs",
            &[
                "id",
                "connection",
                "job_name",
                "queue",
                "envelope_json",
                "exception",
                "failed_at",
            ],
            vec![
                v(id.to_string()),
                v("database"),
                v("Ldb.Refund"),
                v("default"),
                v(env.to_json().expect("envelope JSON")),
                v("boom"),
                v(t0 - 60),
            ],
        )
        .await;
    }

    // job_batches and job_batch_settlements: the batch repository's schema.
    create(
        conn,
        Table::create()
            .table(Alias::new("job_batches"))
            .col(col("id").string().not_null().primary_key())
            .col(col("name").string().not_null())
            .col(col("total_jobs").integer().not_null())
            .col(col("options_json").text().not_null())
            .col(col("created_at").big_integer().not_null())
            .col(col("cancelled_at").big_integer().null())
            .col(col("finished_at").big_integer().null())
            .to_owned(),
    )
    .await;
    create(
        conn,
        Table::create()
            .table(Alias::new("job_batch_settlements"))
            .col(col("batch_id").string().not_null())
            .col(col("job_id").string().not_null())
            .col(col("failed").integer().not_null())
            .col(col("settled_at").big_integer().not_null())
            .primary_key(
                Index::create()
                    .col(Alias::new("batch_id"))
                    .col(Alias::new("job_id")),
            )
            .to_owned(),
    )
    .await;
    insert(
        conn,
        "job_batches",
        &["id", "name", "total_jobs", "options_json", "created_at"],
        vec![
            v(earlier.batch.clone()),
            v("imports"),
            v(3),
            v(serde_json::to_string(&suprnova::BatchOptions::default()).expect("options")),
            v(t0 - 100),
        ],
    )
    .await;
    for (job, failed) in [(Uuid::new_v4(), 0), (earlier.batch_failed_job, 1)] {
        insert(
            conn,
            "job_batch_settlements",
            &["batch_id", "job_id", "failed", "settled_at"],
            vec![
                v(earlier.batch.clone()),
                v(job.to_string()),
                v(failed),
                v(t0 - 50),
            ],
        )
        .await;
    }

    // sessions: the earlier scaffold's migration, and a signed-in session
    // as the earlier driver stored it: the data as JSON, the CSRF token in a
    // column of its own, the last activity as a date-time.
    create(
        conn,
        Table::create()
            .table(Alias::new("sessions"))
            .col(col("id").string().not_null().primary_key())
            .col(col("user_id").string().null())
            .col(col("payload").text().not_null())
            .col(col("csrf_token").string().not_null())
            .col(col("last_activity").date_time().not_null())
            .to_owned(),
    )
    .await;
    let mut session =
        suprnova::session::SessionData::new(earlier.session.clone(), earlier.session_csrf.clone());
    session.set_auth_guard_for_test("web", "1", None);
    session.user_id = Some("1".to_owned());
    insert(
        conn,
        "sessions",
        &["id", "user_id", "payload", "csrf_token", "last_activity"],
        vec![
            v(earlier.session.clone()),
            v("1"),
            v(serde_json::to_string(&session.data).expect("session data")),
            v(earlier.session_csrf.clone()),
            v(Utc::now().naive_utc() - chrono::Duration::minutes(5)),
        ],
    )
    .await;

    // notifications: the earlier `CreateNotificationsTable`.
    create(
        conn,
        Table::create()
            .table(Alias::new("notifications"))
            .col(col("id").char_len(36).not_null().primary_key())
            .col(col("type").string_len(255).not_null())
            .col(col("notifiable_type").string_len(255).not_null())
            .col(col("notifiable_id").string_len(64).not_null())
            .col(col("data").text().not_null())
            .col(col("read_at").date_time().null())
            .col(col("created_at").date_time().not_null())
            .col(col("updated_at").date_time().not_null())
            .to_owned(),
    )
    .await;
    let at = chrono::Timelike::with_nanosecond(
        &(Utc::now() - chrono::Duration::hours(1)).naive_utc(),
        0,
    )
    .expect("a whole second");
    for (index, id) in earlier.notifications.iter().enumerate() {
        insert(
            conn,
            "notifications",
            &[
                "id",
                "type",
                "notifiable_type",
                "notifiable_id",
                "data",
                "read_at",
                "created_at",
                "updated_at",
            ],
            vec![
                v(id.clone()),
                v("Shipped"),
                v("App\\Models\\User"),
                v("1"),
                v(serde_json::json!({ "order": index }).to_string()),
                v((index == 1).then_some(at)),
                v(at),
                v(at),
            ],
        )
        .await;
    }

    // features: the earlier `CreateFeaturesTable`.
    let zoned = |name: &str| {
        let mut column = col(name);
        if engine == Engine::Mysql {
            column.date_time();
        } else {
            column.timestamp_with_time_zone();
        }
        column
            .not_null()
            .default(Expr::current_timestamp())
            .to_owned()
    };
    create(
        conn,
        Table::create()
            .table(Alias::new("features"))
            .col(
                col("id")
                    .big_integer()
                    .not_null()
                    .auto_increment()
                    .primary_key(),
            )
            .col(col("name").string_len(255).not_null())
            .col(col("scope_key").string_len(255).not_null().default(""))
            .col(col("enabled").boolean().not_null())
            .col(col("description").text().null())
            .col(col("updated_by").string_len(255).null())
            .col(zoned("created_at"))
            .col(zoned("updated_at"))
            .to_owned(),
    )
    .await;
    for (name, scope, enabled, description) in [
        ("beta", "", true, Some("The beta")),
        ("beta", "user:2", false, None),
        ("ops-tools", "team:staff", true, None),
    ] {
        insert(
            conn,
            "features",
            &["name", "scope_key", "enabled", "description"],
            vec![v(name), v(scope), v(enabled), v(description)],
        )
        .await;
    }

    // roles, permissions and the assignments: the earlier `CreateRbacTables`.
    for table in ["roles", "permissions"] {
        create(
            conn,
            Table::create()
                .table(Alias::new(table))
                .col(
                    col("id")
                        .big_integer()
                        .not_null()
                        .auto_increment()
                        .primary_key(),
                )
                .col(col("name").string_len(255).not_null())
                .col(col("display_name").string_len(255).null())
                .col(col("guard_name").string_len(255).not_null().default("web"))
                .col(
                    col("created_at")
                        .timestamp_with_time_zone()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .col(
                    col("updated_at")
                        .timestamp_with_time_zone()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .to_owned(),
        )
        .await;
    }
    for (table, owner) in [
        ("model_roles", "role_id"),
        ("model_permissions", "permission_id"),
    ] {
        create(
            conn,
            Table::create()
                .table(Alias::new(table))
                .col(
                    col("id")
                        .big_integer()
                        .not_null()
                        .auto_increment()
                        .primary_key(),
                )
                .col(col("model_type").string_len(255).not_null())
                .col(col("model_id").string_len(255).not_null())
                .col(col(owner).big_integer().not_null())
                .to_owned(),
        )
        .await;
    }
    create(
        conn,
        Table::create()
            .table(Alias::new("role_permissions"))
            .col(
                col("id")
                    .big_integer()
                    .not_null()
                    .auto_increment()
                    .primary_key(),
            )
            .col(col("role_id").big_integer().not_null())
            .col(col("permission_id").big_integer().not_null())
            .to_owned(),
    )
    .await;
    insert(
        conn,
        "roles",
        &["name", "display_name"],
        vec![v("writer"), v("Writer")],
    )
    .await;
    for name in ["edit articles", "delete articles"] {
        insert(conn, "permissions", &["name"], vec![v(name)]).await;
    }
    insert(
        conn,
        "role_permissions",
        &["role_id", "permission_id"],
        vec![v(1i64), v(1i64)],
    )
    .await;
    insert(
        conn,
        "model_roles",
        &["model_type", "model_id", "role_id"],
        vec![v("App\\Models\\User"), v("1"), v(1i64)],
    )
    .await;
    insert(
        conn,
        "model_permissions",
        &["model_type", "model_id", "permission_id"],
        vec![v("App\\Models\\User"), v("2"), v(2i64)],
    )
    .await;

    (db, earlier)
}

/// The earlier database, upgraded by the framework's and a fresh
/// scaffold's migrations.
async fn upgraded(engine: Engine) -> (support::Db, Earlier) {
    let (db, earlier) = earlier_database(engine).await;
    crate::scaffold::migrate(&db.conn)
        .await
        .unwrap_or_else(|e| panic!("{engine:?}: migrate over the earlier layouts: {e}"));
    (db, earlier)
}

/// Pop and ack every job that is due, as a worker would; the envelope ids,
/// in the order they ran.
async fn drain(driver: &DatabaseQueueDriver) -> Vec<Uuid> {
    let mut ran = Vec::new();
    while let Some(reservation) = driver.pop(Duration::from_secs(30)).await.expect("pop") {
        driver.ack(&reservation.token).await.expect("ack");
        ran.push(reservation.envelope.id);
    }
    ran
}

/// A job queued, delayed or reserved before the upgrade runs exactly once
/// after it: the queued one now, the reserved one by its holder's token
/// or, once its reservation lapses, by the next worker; the delayed one
/// when it is due.
async fn jobs_run_once(engine: Engine) {
    let (db, earlier) = upgraded(engine).await;
    let _bound = support::bind(&db.conn);
    let driver = DatabaseQueueDriver::new(db.conn.clone(), "jobs".to_owned()).expect("driver");
    let mut ran = Vec::new();

    // Now: the queued job, and not the delayed or the reserved one.
    let now_ran = drain(&driver).await;
    assert_eq!(now_ran, [earlier.queued], "{engine:?}: what ran at once");
    ran.extend(now_ran);

    // The reservation kept its token and deadline.
    let reservations = support::rows(
        &db.conn,
        "SELECT token, reserved_until FROM suprnova_jobs_reservations",
    )
    .await;
    assert_eq!(reservations.len(), 1, "{engine:?}");
    assert_eq!(
        support::text(&reservations[0], "token"),
        earlier.reserved_token.to_string()
    );
    assert_eq!(
        support::int(&reservations[0], "reserved_until"),
        earlier.reserved_until
    );

    // Past the delay and the reservation's deadline: each runs once more.
    {
        let _clock =
            suprnova::testing::TestClock::travel_to(Utc::now() + chrono::Duration::seconds(400));
        let later = drain(&driver).await;
        let mut later_sorted = later.clone();
        later_sorted.sort();
        let mut expected = vec![earlier.delayed, earlier.reserved];
        expected.sort();
        assert_eq!(later_sorted, expected, "{engine:?}: what ran once due");
        ran.extend(later);
    }
    {
        let _clock =
            suprnova::testing::TestClock::travel_to(Utc::now() + chrono::Duration::days(30));
        let never = drain(&driver).await;
        assert!(never.is_empty(), "{engine:?}: a job ran twice: {never:?}");
    }
    assert_eq!(ran.len(), 3);
    assert_eq!(support::count(&db.conn, "jobs", "").await, 0);
}

on_every_engine!(jobs_run_once =>
    ldb_010_queued_delayed_and_reserved_jobs_run_once_sqlite,
    ldb_010_queued_delayed_and_reserved_jobs_run_once_postgres,
    ldb_010_queued_delayed_and_reserved_jobs_run_once_mysql);

/// The worker that held the reservation before the upgrade settles it by
/// its token, and nobody else runs it.
async fn the_holder_settles_its_reservation(engine: Engine) {
    let (db, earlier) = upgraded(engine).await;
    let _bound = support::bind(&db.conn);
    let driver = DatabaseQueueDriver::new(db.conn.clone(), "jobs".to_owned()).expect("driver");
    driver
        .ack(&ReservationToken(earlier.reserved_token))
        .await
        .expect("the holder acks by its token");
    let _clock =
        suprnova::testing::TestClock::travel_to(Utc::now() + chrono::Duration::seconds(400));
    let later = drain(&driver).await;
    assert!(
        !later.contains(&earlier.reserved),
        "{engine:?}: the settled reservation ran again"
    );
}

on_every_engine!(the_holder_settles_its_reservation =>
    ldb_010_a_reservation_settles_by_its_earlier_token_sqlite,
    ldb_010_a_reservation_settles_by_its_earlier_token_postgres,
    ldb_010_a_reservation_settles_by_its_earlier_token_mysql);

/// Each failed job lists, retries and forgets by its earlier id.
async fn failed_jobs_keep_their_ids(engine: Engine) {
    let (db, earlier) = upgraded(engine).await;
    let _bound = support::bind(&db.conn);
    let store = std::sync::Arc::new(
        DatabaseFailedJobStore::new(db.conn.clone(), "failed_jobs".to_owned()).expect("store"),
    );
    store
        .check()
        .await
        .expect("the worker's check accepts the upgraded table");
    let mut listed = store.ids().await.expect("list");
    listed.sort();
    let mut expected = earlier.failed.to_vec();
    expected.sort();
    assert_eq!(listed, expected, "{engine:?}: queue:failed");

    let driver = std::sync::Arc::new(
        DatabaseQueueDriver::new(db.conn.clone(), "jobs".to_owned()).expect("driver"),
    );
    Queue::set_driver(driver.clone());
    Queue::set_failed_store(store.clone());
    assert!(
        Queue::retry_failed(earlier.failed[0]).await.expect("retry"),
        "{engine:?}: queue:retry by the earlier id"
    );
    assert!(
        drain(&driver).await.contains(&earlier.failed[0]),
        "{engine:?}: the retried job was not queued"
    );
    assert!(
        store.forget(earlier.failed[1]).await.expect("forget"),
        "{engine:?}: queue:forget"
    );
    assert_eq!(store.count().await.expect("count"), 0);
}

on_every_engine!(failed_jobs_keep_their_ids =>
    ldb_010_failed_jobs_list_retry_and_forget_by_their_ids_sqlite,
    ldb_010_failed_jobs_list_retry_and_forget_by_their_ids_postgres,
    ldb_010_failed_jobs_list_retry_and_forget_by_their_ids_mysql);

/// A batch reports the total, pending and failed counts it had.
async fn batches_keep_their_counts(engine: Engine) {
    let (db, earlier) = upgraded(engine).await;
    let batch = DatabaseBatchRepository::new(db.conn.clone())
        .find(&earlier.batch)
        .await
        .expect("find")
        .expect("the batch");
    assert_eq!(
        (batch.total_jobs, batch.pending_jobs, batch.failed_jobs),
        (3, 1, 1),
        "{engine:?}"
    );
    assert_eq!(batch.failed_job_ids, [earlier.batch_failed_job]);
    assert_eq!(batch.name, "imports");
}

on_every_engine!(batches_keep_their_counts =>
    ldb_010_batches_keep_their_counts_sqlite,
    ldb_010_batches_keep_their_counts_postgres,
    ldb_010_batches_keep_their_counts_mysql);

/// A session issued before the upgrade still signs its user in and keeps
/// its CSRF token.
async fn sessions_stay_signed_in(engine: Engine) {
    let (db, earlier) = upgraded(engine).await;
    let _bound = support::bind(&db.conn);
    let _ = suprnova::crypto::_test_install_key(suprnova::EncryptionKey::generate());
    let router: Router = Router::new()
        .get("/whoami", |_request: Request| async {
            Ok(HttpResponse::text(
                Auth::id().unwrap_or_else(|| "guest".to_owned()),
            ))
        })
        .get("/csrf", |_request: Request| async {
            Ok(HttpResponse::text(
                suprnova::get_csrf_token().unwrap_or_default(),
            ))
        })
        .into();
    let mut browser = Browser::serve(router).await;
    let cookie = suprnova::Cookie::encrypted(SESSION_COOKIE, &earlier.session).expect("cookie");
    browser.set_cookie(SESSION_COOKIE, cookie.value());
    let (_, user) = browser.get("/whoami", &[]).await;
    assert_eq!(user, "1", "{engine:?}: the session signed its user out");
    let (_, csrf) = browser.get("/csrf", &[]).await;
    assert_eq!(csrf, earlier.session_csrf, "{engine:?}: the CSRF token");
}

on_every_engine!(sessions_stay_signed_in =>
    ldb_010_sessions_stay_signed_in_with_their_csrf_token_sqlite,
    ldb_010_sessions_stay_signed_in_with_their_csrf_token_postgres,
    ldb_010_sessions_stay_signed_in_with_their_csrf_token_mysql);

/// `notifications`, `features`, `roles` and `permissions` take the layouts
/// of LDB-001, and every notification, flag, role, permission and
/// assignment survives.
async fn reshaped_tables_keep_their_rows(engine: Engine) {
    let laravel = crate::tables::laravel_shapes(engine).await;
    let (db, earlier) = upgraded(engine).await;
    let _bound = support::bind(&db.conn);
    for table in [
        "jobs",
        "job_batches",
        "failed_jobs",
        "sessions",
        "notifications",
        "features",
        "roles",
        "permissions",
        "model_has_roles",
        "model_has_permissions",
        "role_has_permissions",
    ] {
        let ours = crate::catalog::shape(&db.conn, table).await;
        crate::tables::assert_same_layout(engine, table, &laravel[table], &ours);
    }
    for gone in ["model_roles", "model_permissions", "role_permissions"] {
        assert!(
            !crate::catalog::tables(&db.conn)
                .await
                .iter()
                .any(|t| t == gone),
            "{engine:?}: the earlier {gone} is still there"
        );
    }

    // Notifications.
    let inbox = suprnova::notifications::all_for(&db.conn, "App\\Models\\User", "1")
        .await
        .expect("the inbox");
    let mut ids: Vec<String> = inbox.iter().map(|n| n.id.clone()).collect();
    ids.sort();
    let mut expected = earlier.notifications.to_vec();
    expected.sort();
    assert_eq!(ids, expected, "{engine:?}");
    let read = inbox
        .iter()
        .find(|n| n.id == earlier.notifications[1])
        .expect("the read one");
    assert!(read.read_at.is_some());
    assert_eq!(read.data, serde_json::json!({ "order": 1 }));

    // Flags.
    let evaluator = std::sync::Arc::new(
        suprnova::features::DatabaseEvaluator::new()
            .await
            .expect("the evaluator"),
    );
    use suprnova::features::Evaluator;
    let (global, user_2) = featureflag::evaluator::with_default(evaluator.clone(), || {
        let user_2 = featureflag::context! { user_id = 2i64 };
        (
            evaluator.is_enabled("beta", &suprnova::features::Context::root()),
            evaluator.is_enabled("beta", &user_2),
        )
    });
    assert_eq!((global, user_2), (Some(true), Some(false)), "{engine:?}");
    let mut stored: Vec<(String, String)> =
        support::rows(&db.conn, "SELECT scope, value FROM features")
            .await
            .iter()
            .map(|row| (support::text(row, "scope"), support::text(row, "value")))
            .collect();
    stored.sort();
    assert_eq!(
        stored,
        [
            ("App\\Models\\User|2".to_owned(), "false".to_owned()),
            ("__laravel_null".to_owned(), "true".to_owned()),
            ("team:staff".to_owned(), "true".to_owned()),
        ],
        "{engine:?}"
    );
    let described = support::rows(
        &db.conn,
        "SELECT description FROM suprnova_feature_details WHERE name = 'beta'",
    )
    .await;
    assert_eq!(support::text(&described[0], "description"), "The beta");

    // Roles, permissions and assignments.
    assert!(
        suprnova::rbac::has_role_for_model("App\\Models\\User", "1", "writer")
            .await
            .expect("check"),
        "{engine:?}"
    );
    assert!(
        suprnova::rbac::has_permission_for_model("App\\Models\\User", "1", "edit articles")
            .await
            .expect("check"),
        "{engine:?}: through the role"
    );
    assert!(
        suprnova::rbac::has_permission_for_model("App\\Models\\User", "2", "delete articles")
            .await
            .expect("check"),
        "{engine:?}: direct"
    );
    let detail = support::rows(
        &db.conn,
        "SELECT display_name FROM suprnova_role_details WHERE role_id = 1",
    )
    .await;
    assert_eq!(support::text(&detail[0], "display_name"), "Writer");
    // A role created after the upgrade takes the next id.
    let next = suprnova::rbac::create_role("editor")
        .await
        .expect("create a role");
    assert_eq!(next, 2, "{engine:?}");
}

on_every_engine!(reshaped_tables_keep_their_rows =>
    ldb_010_notifications_flags_roles_and_permissions_move_sqlite,
    ldb_010_notifications_flags_roles_and_permissions_move_postgres,
    ldb_010_notifications_flags_roles_and_permissions_move_mysql);

/// An upgrade that stopped after copying the earlier `jobs` aside, before
/// or after dropping it, resumes on the next `migrate` and moves every job
/// once.
async fn an_interrupted_upgrade_resumes(engine: Engine) {
    for dropped in [false, true] {
        let (db, earlier) = earlier_database(engine).await;
        db.conn
            .execute_unprepared("CREATE TABLE suprnova_earlier_jobs AS SELECT * FROM jobs")
            .await
            .expect("copy the earlier jobs aside");
        if dropped {
            db.conn
                .execute_unprepared("DROP TABLE jobs")
                .await
                .expect("drop the earlier jobs");
        }
        crate::scaffold::migrate(&db.conn)
            .await
            .unwrap_or_else(|e| panic!("{engine:?}: the resumed migrate: {e}"));
        let _bound = support::bind(&db.conn);
        assert_eq!(
            support::count(&db.conn, "jobs", "").await,
            3,
            "{engine:?} (dropped: {dropped}): every job moved once"
        );
        assert!(
            !crate::catalog::tables(&db.conn)
                .await
                .iter()
                .any(|t| t == "suprnova_earlier_jobs"),
            "{engine:?}: the copy is gone once empty"
        );
        let driver = DatabaseQueueDriver::new(db.conn.clone(), "jobs".to_owned()).expect("driver");
        assert_eq!(
            drain(&driver).await,
            [earlier.queued],
            "{engine:?} (dropped: {dropped})"
        );
    }
}

on_every_engine!(an_interrupted_upgrade_resumes =>
    ldb_010_an_interrupted_upgrade_resumes_sqlite,
    ldb_010_an_interrupted_upgrade_resumes_postgres,
    ldb_010_an_interrupted_upgrade_resumes_mysql);

/// The migration the manual gives for an earlier scaffold's `users`.
#[path = "users_upgrade.rs"]
mod users_upgrade;

struct UsersMigrator;

impl sea_orm_migration::MigratorTrait for UsersMigrator {
    fn migrations() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
        vec![Box::new(users_upgrade::Migration)]
    }
}

/// The manual's migration moves an earlier scaffold's `users` into the
/// skeleton's layout with every row, and the scaffold's `User` then
/// creates the next one.
async fn the_manuals_users_migration_works(engine: Engine) {
    let laravel = crate::tables::laravel_shapes(engine).await;
    let db = support::empty(engine).await;
    let conn = &db.conn;
    // The earlier scaffold's `create_users_table`.
    create(
        conn,
        Table::create()
            .table(Alias::new("users"))
            .col(
                col("id")
                    .big_integer()
                    .not_null()
                    .auto_increment()
                    .primary_key(),
            )
            .col(col("name").string().not_null())
            .col(col("email").string().not_null().unique_key())
            .col(col("password").string().not_null())
            .col(col("remember_token").string().null())
            .col(col("email_verified_at").date_time().null())
            .col(
                col("created_at")
                    .date_time()
                    .not_null()
                    .default(Expr::current_timestamp()),
            )
            .col(
                col("updated_at")
                    .date_time()
                    .not_null()
                    .default(Expr::current_timestamp()),
            )
            .to_owned(),
    )
    .await;
    for (name, email, token) in [
        ("Taylor", "taylor@example.com", Some("remember-me")),
        ("Abigail", "abigail@example.com", None),
    ] {
        insert(
            conn,
            "users",
            &["name", "email", "password", "remember_token"],
            vec![v(name), v(email), v("$2b$12$hash"), v(token)],
        )
        .await;
    }
    <UsersMigrator as sea_orm_migration::MigratorTrait>::up(conn, None)
        .await
        .unwrap_or_else(|e| panic!("{engine:?}: the manual's users migration: {e}"));
    let ours = crate::catalog::shape(conn, "users").await;
    crate::tables::assert_same_layout(engine, "users", &laravel["users"], &ours);
    let rows = support::rows(
        conn,
        "SELECT id, email, remember_token FROM users ORDER BY id",
    )
    .await;
    assert_eq!(rows.len(), 2, "{engine:?}");
    assert_eq!(support::int(&rows[0], "id"), 1);
    assert_eq!(support::text(&rows[0], "remember_token"), "remember-me");
    assert_eq!(support::text(&rows[1], "email"), "abigail@example.com");
    let _bound = support::bind(conn);
    let created = crate::scaffold::user::User::create("Nuno", "nuno@example.com", "secret")
        .await
        .expect("create a user");
    assert_eq!(
        created.id, 3,
        "{engine:?}: the next id follows the kept ones"
    );
}

on_every_engine!(the_manuals_users_migration_works =>
    ldb_010_the_manuals_users_migration_moves_an_earlier_scaffold_sqlite,
    ldb_010_the_manuals_users_migration_moves_an_earlier_scaffold_postgres,
    ldb_010_the_manuals_users_migration_moves_an_earlier_scaffold_mysql);

/// Assignments an earlier release stored under its default discriminator,
/// the model's Rust type path, still apply after the upgrade, now that the
/// default follows the model's `morph_type`. A new grant does not duplicate
/// them, and a removal takes them away.
async fn earlier_default_discriminator_still_applies(engine: Engine) {
    use suprnova::{HasRoles, Model};
    let (db, _) = earlier_database(engine).await;
    let earlier_default = std::any::type_name::<crate::models::LdbUser>();
    for table in ["model_roles", "model_permissions"] {
        db.conn
            .execute_unprepared(&format!(
                "UPDATE {table} SET model_type = '{earlier_default}'"
            ))
            .await
            .expect("store the earlier default discriminator");
    }
    crate::scaffold::migrate(&db.conn)
        .await
        .unwrap_or_else(|e| panic!("{engine:?}: migrate over the earlier layouts: {e}"));
    let _bound = support::bind(&db.conn);
    db.conn
        .execute_unprepared(
            "INSERT INTO users (id, name, email, password) VALUES \
             (1, 'Earlier', '1@example.com', 'x'), (2, 'Earlier', '2@example.com', 'x')",
        )
        .await
        .expect("the two users");
    let first = crate::models::LdbUser::find(1u64)
        .await
        .expect("find")
        .expect("user 1");
    let second = crate::models::LdbUser::find(2u64)
        .await
        .expect("find")
        .expect("user 2");
    let user = |id: u64| if id == 1 { &first } else { &second };
    assert!(
        user(1).has_role("writer").await.expect("check"),
        "{engine:?}: the role an earlier release assigned is gone"
    );
    assert!(
        user(1)
            .has_permission_to("edit articles")
            .await
            .expect("check"),
        "{engine:?}: the permission through that role is gone"
    );
    assert!(
        user(2)
            .has_permission_to("delete articles")
            .await
            .expect("check"),
        "{engine:?}: the direct permission an earlier release gave is gone"
    );
    user(1).assign_role("writer").await.expect("assign again");
    assert_eq!(
        support::count(&db.conn, "model_has_roles", "").await,
        1,
        "{engine:?}: a grant the model already held was written twice"
    );
    user(1)
        .remove_role("writer")
        .await
        .expect("remove the role");
    assert!(
        !user(1).has_role("writer").await.expect("check"),
        "{engine:?}: the earlier assignment survived its removal"
    );
    user(2)
        .remove_permission_to("delete articles")
        .await
        .expect("remove the permission");
    assert!(
        !user(2)
            .has_permission_to("delete articles")
            .await
            .expect("check"),
        "{engine:?}: the earlier grant survived its removal"
    );
}

on_every_engine!(earlier_default_discriminator_still_applies =>
    ldb_010_assignments_under_the_earlier_default_discriminator_still_apply_sqlite,
    ldb_010_assignments_under_the_earlier_default_discriminator_still_apply_postgres,
    ldb_010_assignments_under_the_earlier_default_discriminator_still_apply_mysql);

/// An upgrade that stopped after it created the new `features`,
/// `notifications` and `roles` tables and before their indexes resumes on
/// the next `migrate` and completes them: every index of the package's or
/// Laravel's layout, and the uniqueness the index enforces.
async fn a_resume_completes_the_indexes(engine: Engine) {
    use suprnova::schema::Schema;
    let laravel = crate::tables::laravel_shapes(engine).await;
    let (db, _) = earlier_database(engine).await;
    let manager = sea_orm_migration::SchemaManager::new(&db.conn);
    for table in ["features", "notifications", "roles"] {
        db.conn
            .execute_unprepared(&format!(
                "CREATE TABLE suprnova_earlier_{table} AS SELECT * FROM {table}"
            ))
            .await
            .expect("set the earlier rows aside");
        db.conn
            .execute_unprepared(&format!("DROP TABLE {table}"))
            .await
            .expect("drop the earlier table");
    }
    // The new tables as the stopped upgrade left them: created, without the
    // indexes the next statements would have added.
    Schema::create(&manager, "features", |t| {
        t.unsigned_id();
        t.string("name");
        t.string("scope");
        t.text("value");
        t.date_time("created_at").precision(0).nullable();
        t.date_time("updated_at").precision(0).nullable();
    })
    .await
    .expect("features without its index");
    Schema::create(&manager, "notifications", |t| {
        t.uuid("id").primary();
        t.string("type");
        t.string("notifiable_type");
        t.unsigned_big_integer("notifiable_id");
        t.text("data");
        t.date_time("read_at").precision(0).nullable();
        t.date_time("created_at").precision(0).nullable();
        t.date_time("updated_at").precision(0).nullable();
    })
    .await
    .expect("notifications without its index");
    Schema::create(&manager, "roles", |t| {
        t.unsigned_id();
        t.string("name");
        t.string("guard_name");
        t.date_time("created_at").precision(0).nullable();
        t.date_time("updated_at").precision(0).nullable();
    })
    .await
    .expect("roles without its index");

    crate::scaffold::migrate(&db.conn)
        .await
        .unwrap_or_else(|e| panic!("{engine:?}: the resumed migrate: {e}"));
    for table in ["features", "notifications", "roles"] {
        let ours = crate::catalog::shape(&db.conn, table).await;
        crate::tables::assert_same_layout(engine, table, &laravel[table], &ours);
    }
    for (table, rows) in [("features", 3), ("notifications", 2), ("roles", 1)] {
        assert_eq!(
            support::count(&db.conn, table, "").await,
            rows,
            "{engine:?}: {table} lost or doubled a row"
        );
    }
    let duplicate = db
        .conn
        .execute_unprepared(
            "INSERT INTO features (name, scope, value) VALUES ('beta', '__laravel_null', 'true')",
        )
        .await;
    assert!(
        duplicate.is_err(),
        "{engine:?}: features took a second row for one flag and scope"
    );
}

on_every_engine!(a_resume_completes_the_indexes =>
    ldb_010_a_resume_between_a_table_and_its_index_completes_the_schema_sqlite,
    ldb_010_a_resume_between_a_table_and_its_index_completes_the_schema_postgres,
    ldb_010_a_resume_between_a_table_and_its_index_completes_the_schema_mysql);

/// Sets an environment variable for as long as it lives.
struct EnvVar(&'static str);

impl EnvVar {
    fn set(name: &'static str, value: &str) -> Self {
        // SAFETY: the tests are serial within their process, and nothing
        // else reads the environment while a migration runs.
        unsafe { std::env::set_var(name, value) };
        Self(name)
    }
}

impl Drop for EnvVar {
    fn drop(&mut self) {
        // SAFETY: as in `set`.
        unsafe { std::env::remove_var(self.0) };
    }
}

/// Notifications and assignments whose recipient key is a UUID, migrated
/// first under the default `int` settings, which refuse them, and then
/// under the `uuid` settings each refusal advises, end in the `uuid`
/// layout with every row.
async fn the_advised_key_setting_upgrades(engine: Engine) {
    let laravel = crate::tables::laravel_shapes(engine).await;
    let (db, _) = earlier_database(engine).await;
    let recipient = Uuid::new_v4().to_string();
    for (table, column) in [
        ("notifications", "notifiable_id"),
        ("model_roles", "model_id"),
        ("model_permissions", "model_id"),
    ] {
        db.conn
            .execute_unprepared(&format!("UPDATE {table} SET {column} = '{recipient}'"))
            .await
            .expect("a UUID recipient");
    }
    let refused = crate::scaffold::migrate(&db.conn)
        .await
        .expect_err("NOTIFICATIONS_MORPH_KEY=int took a UUID recipient");
    assert!(
        refused.to_string().contains("NOTIFICATIONS_MORPH_KEY"),
        "{engine:?}: {refused}"
    );
    let _notifications = EnvVar::set("NOTIFICATIONS_MORPH_KEY", "uuid");
    let refused = crate::scaffold::migrate(&db.conn)
        .await
        .expect_err("RBAC_MODEL_KEY=int took a UUID model id");
    assert!(
        refused.to_string().contains("RBAC_MODEL_KEY"),
        "{engine:?}: {refused}"
    );
    let _rbac = EnvVar::set("RBAC_MODEL_KEY", "uuid");
    crate::scaffold::migrate(&db.conn)
        .await
        .unwrap_or_else(|e| panic!("{engine:?}: migrate under the advised settings: {e}"));

    for (table, column) in [
        ("notifications", "notifiable_id"),
        ("model_has_roles", "model_id"),
        ("model_has_permissions", "model_id"),
    ] {
        let mut expected = laravel[table].clone();
        expected
            .columns
            .iter_mut()
            .find(|c| c.name == column)
            .expect("the key column")
            .kind = crate::tables::laravel_key_kind(engine, "uuid").to_owned();
        let ours = crate::catalog::shape(&db.conn, table).await;
        crate::tables::assert_same_layout(engine, table, &expected, &ours);
    }
    for (table, rows) in [
        ("notifications", 2),
        ("model_has_roles", 1),
        ("model_has_permissions", 1),
    ] {
        assert_eq!(
            support::count(&db.conn, table, "").await,
            rows,
            "{engine:?}: {table} lost a row"
        );
    }
    let _bound = support::bind(&db.conn);
    assert!(
        suprnova::rbac::has_role_for_model("App\\Models\\User", &recipient, "writer")
            .await
            .expect("check"),
        "{engine:?}: the UUID model lost its role"
    );
}

on_every_engine!(the_advised_key_setting_upgrades =>
    ldb_010_a_wrong_key_setting_then_the_advised_one_upgrades_every_row_sqlite,
    ldb_010_a_wrong_key_setting_then_the_advised_one_upgrades_every_row_postgres,
    ldb_010_a_wrong_key_setting_then_the_advised_one_upgrades_every_row_mysql);
