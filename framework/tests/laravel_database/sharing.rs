//! LDB-009: a Laravel 13 application keeps working on the database while a
//! Suprnova application with `LARAVEL_SHARED_DATABASE` on writes to it.
//!
//! No Laravel application runs here. Each row Suprnova writes is decoded
//! the way Laravel 13's or the package's reader of that table decodes it,
//! reproduced below from the source the fixtures were generated with
//! (laravel/framework v13.34.0, laravel/pennant v1.26.0); password hashes
//! go through the host `php`.

use std::sync::Arc;
use std::time::Duration;

use sea_orm::ConnectionTrait;
use serde::{Deserialize, Serialize};
use suprnova::notifications::Channel;
use suprnova::queue::worker::{WorkerConfig, run_worker};
use suprnova::session::SessionStore;
use suprnova::{
    Batch, BatchOptions, BatchRepository, DatabaseBatchRepository, DatabaseFailedJobStore,
    DatabaseQueueDriver, FailedJobStore, FrameworkError, Job, Queue, async_trait,
};
use tokio_util::sync::CancellationToken;

use crate::failed_jobs::envelope;
use crate::on_every_engine;
use crate::support::{self, Engine, Shared};

/// A job queued with default settings.
#[derive(Serialize, Deserialize, Debug, Clone)]
struct ShipOrder {
    order: u64,
}

#[async_trait]
impl Job for ShipOrder {
    fn job_name() -> &'static str {
        "Ldb.ShipOrder"
    }

    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
}

// ---- Laravel's readers -------------------------------------------------------

/// A value of PHP's `serialize` format, as far as `unserialize` reads the
/// rows these tests check: null, booleans, integers, floats, strings and
/// arrays. Anything else is refused, as Laravel's reader would fail on it.
#[derive(Debug, Clone, PartialEq)]
enum Php {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(Vec<u8>),
    Array(Vec<(Php, Php)>),
}

/// PHP's `unserialize` over the subset [`Php`] holds; `None` where PHP
/// returns `false`.
fn php_unserialize(input: &[u8]) -> Option<Php> {
    fn number(input: &[u8], end: u8) -> Option<(&str, &[u8])> {
        let at = input.iter().position(|b| *b == end)?;
        Some((std::str::from_utf8(&input[..at]).ok()?, &input[at + 1..]))
    }
    fn value(input: &[u8]) -> Option<(Php, &[u8])> {
        match input {
            [b'N', b';', rest @ ..] => Some((Php::Null, rest)),
            [b'b', b':', rest @ ..] => {
                let (n, rest) = number(rest, b';')?;
                Some((Php::Bool(n.parse::<u8>().ok()? != 0), rest))
            }
            [b'i', b':', rest @ ..] => {
                let (n, rest) = number(rest, b';')?;
                Some((Php::Int(n.parse().ok()?), rest))
            }
            [b'd', b':', rest @ ..] => {
                let (n, rest) = number(rest, b';')?;
                Some((Php::Float(n.parse().ok()?), rest))
            }
            [b's', b':', rest @ ..] => {
                let (len, rest) = number(rest, b':')?;
                let len: usize = len.parse().ok()?;
                let rest = rest.strip_prefix(b"\"")?;
                let bytes = rest.get(..len)?.to_vec();
                let rest = rest.get(len..)?.strip_prefix(b"\";")?;
                Some((Php::Str(bytes), rest))
            }
            [b'a', b':', rest @ ..] => {
                let (count, rest) = number(rest, b':')?;
                let count: usize = count.parse().ok()?;
                let mut rest = rest.strip_prefix(b"{")?;
                let mut entries = Vec::with_capacity(count);
                for _ in 0..count {
                    let (key, after) = value(rest)?;
                    if !matches!(key, Php::Int(_) | Php::Str(_)) {
                        return None;
                    }
                    let (item, after) = value(after)?;
                    entries.push((key, item));
                    rest = after;
                }
                Some((Php::Array(entries), rest.strip_prefix(b"}")?))
            }
            _ => None,
        }
    }
    let (parsed, rest) = value(input)?;
    rest.is_empty().then_some(parsed)
}

fn base64(text: &str) -> Option<Vec<u8>> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.decode(text).ok()
}

/// `DatabaseBatchRepository::toBatch` (Illuminate/Bus/DatabaseBatchRepository.php
/// 348-383): `failed_job_ids` through `json_decode`, `options` through
/// `unserialize`, base64-decoded first on Postgres when it holds no `:` or
/// `;`, and handed to `Batch`'s `array $options`; the counts as `(int)`
/// and the times through `CarbonImmutable::createFromTimestamp`.
fn laravel_reads_batch(engine: Engine, row: &serde_json::Value) -> Result<(), String> {
    for column in ["total_jobs", "pending_jobs", "failed_jobs", "created_at"] {
        if !row[column].is_i64() && !row[column].is_u64() {
            return Err(format!("{column} is not an integer: {}", row[column]));
        }
    }
    for column in ["cancelled_at", "finished_at"] {
        if !row[column].is_null() && !row[column].is_i64() {
            return Err(format!("{column} is not a timestamp: {}", row[column]));
        }
    }
    let ids: serde_json::Value = serde_json::from_str(&support::text(row, "failed_job_ids"))
        .map_err(|e| format!("failed_job_ids: {e}"))?;
    if !ids.is_array() {
        return Err(format!("failed_job_ids is not a list: {ids}"));
    }
    let options = support::text(row, "options");
    let serialized = if engine == Engine::Postgres && !options.contains([':', ';']) {
        base64(&options).ok_or("options is not base64")?
    } else {
        options.into_bytes()
    };
    match php_unserialize(&serialized) {
        Some(Php::Array(_)) => Ok(()),
        other => Err(format!("options unserializes to {other:?}, not an array")),
    }
}

/// `DatabaseJob` over a `jobs` row (Illuminate/Queue/Jobs/Job.php 86-89,
/// 284-287, 366-369; JobName.php 27-34): `payload()` is
/// `json_decode($raw, true)`, `uuid()` its `uuid`, the name its
/// `displayName`; `attempts` is `(int)`, `reserved_at` and `available_at`
/// are epoch seconds.
fn laravel_reads_job(row: &serde_json::Value) -> Result<(String, String), String> {
    let payload: serde_json::Value = serde_json::from_str(&support::text(row, "payload"))
        .map_err(|e| format!("payload: {e}"))?;
    let uuid = payload["uuid"].as_str().ok_or("payload has no uuid")?;
    uuid::Uuid::parse_str(uuid).map_err(|e| format!("uuid {uuid}: {e}"))?;
    let name = payload["displayName"]
        .as_str()
        .filter(|name| !name.is_empty())
        .ok_or("payload has no displayName")?;
    for column in ["attempts", "available_at", "created_at"] {
        if !row[column].is_i64() {
            return Err(format!("{column} is not an integer: {}", row[column]));
        }
    }
    if !row["reserved_at"].is_null() && !row["reserved_at"].is_i64() {
        return Err(format!("reserved_at: {}", row["reserved_at"]));
    }
    Ok((uuid.to_owned(), name.to_owned()))
}

/// `queue:failed` over `DatabaseUuidFailedJobProvider::all()`
/// (Queue/Failed/DatabaseUuidFailedJobProvider.php, ListFailedCommand.php):
/// the row's `uuid` is its id, `payload` goes through `json_decode`, and
/// `failed_at` is a date-time.
fn laravel_reads_failed_job(row: &serde_json::Value) -> Result<(), String> {
    uuid::Uuid::parse_str(&support::text(row, "uuid")).map_err(|e| format!("uuid: {e}"))?;
    let payload: serde_json::Value = serde_json::from_str(&support::text(row, "payload"))
        .map_err(|e| format!("payload: {e}"))?;
    if payload["uuid"].as_str() != Some(support::text(row, "uuid").as_str()) {
        return Err("payload uuid is not the row's".to_owned());
    }
    date_time(&support::text(row, "failed_at")).map(|_| ())
}

/// `DatabaseSessionHandler::read` and `Store::readFromHandler` with the
/// skeleton's `json` serialization (Session/DatabaseSessionHandler.php
/// 115-132, Session/Store.php 126-141): the payload base64-decodes to a
/// JSON object, and `last_activity` is epoch seconds. Returns the session's
/// `_token`.
fn laravel_reads_session(row: &serde_json::Value) -> Result<String, String> {
    if !row["last_activity"].is_i64() {
        return Err(format!("last_activity: {}", row["last_activity"]));
    }
    let decoded = base64(&support::text(row, "payload")).ok_or("payload is not base64")?;
    let data: serde_json::Value =
        serde_json::from_slice(&decoded).map_err(|e| format!("payload JSON: {e}"))?;
    let data = data.as_object().ok_or("the payload is not an array")?;
    Ok(data
        .get("_token")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned())
}

/// `DatabaseNotification` (Notifications/DatabaseNotification.php 47-49):
/// a string key, `data` cast `array` through `json_decode`, `read_at` and
/// the timestamps date-times.
fn laravel_reads_notification(row: &serde_json::Value) -> Result<(), String> {
    uuid::Uuid::parse_str(&support::text(row, "id")).map_err(|e| format!("id: {e}"))?;
    let data: serde_json::Value =
        serde_json::from_str(&support::text(row, "data")).map_err(|e| format!("data: {e}"))?;
    if !data.is_object() {
        return Err(format!("data is not an array: {data}"));
    }
    for column in ["created_at", "updated_at"] {
        date_time(&support::text(row, column))?;
    }
    if !row["read_at"].is_null() {
        date_time(&support::text(row, "read_at"))?;
    }
    Ok(())
}

/// Pennant's `DatabaseDriver::get` (pennant/src/Drivers/DatabaseDriver.php
/// 209): `json_decode($record->value, JSON_THROW_ON_ERROR)`, for a scope
/// `FeatureManager::serializeScope` gives: `__laravel_null` or
/// `{class}|{key}`.
fn pennant_reads_flag(row: &serde_json::Value) -> Result<serde_json::Value, String> {
    let scope = support::text(row, "scope");
    let model_scope = scope
        .split_once('|')
        .is_some_and(|(class, key)| !class.is_empty() && key.parse::<u64>().is_ok());
    if scope != "__laravel_null" && !model_scope {
        return Err(format!("scope {scope} is not one Pennant serializes"));
    }
    serde_json::from_str(&support::text(row, "value")).map_err(|e| format!("value: {e}"))
}

/// A date-time Eloquent's `asDateTime` reads: `Y-m-d H:i:s`.
fn date_time(text: &str) -> Result<chrono::NaiveDateTime, String> {
    chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S")
        .map_err(|e| format!("{text} is not a date-time: {e}"))
}

// ---- The tests ---------------------------------------------------------------

/// Every row Suprnova writes into `jobs`, `job_batches`, `failed_jobs`,
/// `sessions`, `notifications` and `features` decodes through Laravel's
/// reader, and a password Suprnova sets passes PHP's check.
async fn rows_suprnova_writes_decode_in_laravel(engine: Engine) {
    let _shared = Shared::on();
    let (db, _) = support::laravel(engine).await;
    crate::scaffold::migrate(&db.conn).await.expect("migrate");
    let _bound = support::bind(&db.conn);

    // jobs: a job queued with default settings.
    let laravel_jobs: Vec<String> = support::rows(&db.conn, "SELECT id FROM jobs")
        .await
        .iter()
        .map(|row| support::text(row, "id"))
        .collect();
    Queue::set_driver(Arc::new(
        DatabaseQueueDriver::new(db.conn.clone(), "jobs".to_owned()).expect("the jobs driver"),
    ));
    Queue::push(ShipOrder { order: 7 })
        .await
        .expect("queue a job");
    let ours: Vec<serde_json::Value> = support::rows(&db.conn, "SELECT * FROM jobs")
        .await
        .into_iter()
        .filter(|row| !laravel_jobs.contains(&support::text(row, "id")))
        .collect();
    assert_eq!(ours.len(), 1, "{engine:?}");
    let (_, name) = laravel_reads_job(&ours[0]).unwrap_or_else(|e| panic!("{engine:?} jobs: {e}"));
    assert_eq!(name, "Ldb.ShipOrder");
    for row in support::rows(&db.conn, "SELECT * FROM jobs").await {
        laravel_reads_job(&row).unwrap_or_else(|e| panic!("{engine:?}: Laravel's own row: {e}"));
    }

    // job_batches: a batch with a settled failure.
    let batches = DatabaseBatchRepository::new(db.conn.clone());
    let batch = Batch {
        id: uuid::Uuid::new_v4().to_string(),
        name: "shipping".into(),
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
        .expect("record a failure");
    for row in support::rows(&db.conn, "SELECT * FROM job_batches").await {
        laravel_reads_batch(engine, &row).unwrap_or_else(|e| {
            panic!("{engine:?} job_batches {}: {e}", support::text(&row, "id"))
        });
    }
    let ours = support::rows(
        &db.conn,
        &format!("SELECT * FROM job_batches WHERE id = '{}'", batch.id),
    )
    .await;
    assert_eq!(
        (
            support::int(&ours[0], "pending_jobs"),
            support::int(&ours[0], "failed_jobs")
        ),
        (1, 1),
        "{engine:?}: Laravel reads the counts Suprnova keeps"
    );

    // failed_jobs: a dead letter.
    let failed = DatabaseFailedJobStore::new(db.conn.clone(), "failed_jobs".to_owned())
        .expect("the failed-jobs store");
    failed
        .log(
            "database",
            "default",
            &envelope("Ldb.Refund", serde_json::json!({})),
            "boom",
        )
        .await
        .expect("dead-letter a job");
    for row in support::rows(&db.conn, "SELECT * FROM failed_jobs").await {
        laravel_reads_failed_job(&row).unwrap_or_else(|e| panic!("{engine:?} failed_jobs: {e}"));
    }

    // sessions: a signed-in session with data and a CSRF token.
    let driver = suprnova::session::driver::DatabaseSessionDriver::new(Duration::from_secs(7200));
    let mut session = suprnova::session::SessionData::new(
        format!("{:0<40}", "ldbsharing"),
        "suprnova-csrf-token".into(),
    );
    session.user_id = Some("1".into());
    session
        .data
        .insert("cart".into(), serde_json::json!({ "items": [1, 2] }));
    driver.write(&session).await.expect("write a session");
    for row in support::rows(&db.conn, "SELECT * FROM sessions").await {
        let token =
            laravel_reads_session(&row).unwrap_or_else(|e| panic!("{engine:?} sessions: {e}"));
        if support::text(&row, "id") == session.id {
            assert_eq!(token, "suprnova-csrf-token", "{engine:?}: Laravel's _token");
            assert_eq!(
                support::int(&row, "user_id"),
                1,
                "{engine:?}: sessions.user_id"
            );
        }
    }

    // notifications: one delivered through the database channel.
    suprnova::DatabaseChannel::new(db.conn.clone(), "App\\Models\\User")
        .deliver("2", &support::Shipped)
        .await
        .expect("deliver a notification");
    for row in support::rows(&db.conn, "SELECT * FROM notifications").await {
        laravel_reads_notification(&row)
            .unwrap_or_else(|e| panic!("{engine:?} notifications: {e}"));
    }

    // features: a global flag and a user's.
    let evaluator = suprnova::features::DatabaseEvaluator::new()
        .await
        .expect("the evaluator");
    evaluator
        .set_flag("checkout-v2", "", true)
        .await
        .expect("a global flag");
    evaluator
        .set_flag("checkout-v2", "user:2", false)
        .await
        .expect("a user's flag");
    for row in support::rows(&db.conn, "SELECT * FROM features").await {
        pennant_reads_flag(&row).unwrap_or_else(|e| panic!("{engine:?} features: {e}"));
    }

    // users: a password the scaffold sets passes PHP's check.
    let created =
        crate::scaffold::user::User::create("Nuno", "nuno@example.com", "set-by-suprnova")
            .await
            .expect("create a user");
    let (algo, ok) = support::php_password(&created.password, "set-by-suprnova");
    assert_eq!(
        (algo.as_str(), ok),
        ("bcrypt", true),
        "{engine:?}: {}",
        created.password
    );
}

on_every_engine!(rows_suprnova_writes_decode_in_laravel =>
    ldb_009_rows_suprnova_writes_decode_through_laravels_readers_sqlite,
    ldb_009_rows_suprnova_writes_decode_through_laravels_readers_postgres,
    ldb_009_rows_suprnova_writes_decode_through_laravels_readers_mysql);

/// The queue a job queued with default settings lands on: one Laravel's
/// default worker does not read with the setting on, `default` with it off.
async fn the_default_queue_follows_the_setting(engine: Engine) {
    let (db, _) = support::laravel(engine).await;
    let _bound = support::bind(&db.conn);
    Queue::set_driver(Arc::new(
        DatabaseQueueDriver::new(db.conn.clone(), "jobs".to_owned()).expect("the jobs driver"),
    ));
    let queue_of_last = || async {
        let rows = support::rows(&db.conn, "SELECT queue FROM jobs ORDER BY id DESC").await;
        support::text(&rows[0], "queue")
    };

    suprnova::LaravelDatabase::share(false);
    Queue::push(ShipOrder { order: 1 }).await.expect("queue");
    assert_eq!(
        queue_of_last().await,
        "default",
        "{engine:?}: with the setting off"
    );

    let shared = Shared::on();
    Queue::push(ShipOrder { order: 2 }).await.expect("queue");
    let queue = queue_of_last().await;
    assert_ne!(
        queue, "default",
        "{engine:?}: Laravel's default worker would run it"
    );
    assert_eq!(queue, suprnova::LaravelDatabase::default_queue());
    drop(shared);
}

on_every_engine!(the_default_queue_follows_the_setting =>
    ldb_009_the_default_queue_follows_the_setting_sqlite,
    ldb_009_the_default_queue_follows_the_setting_postgres,
    ldb_009_the_default_queue_follows_the_setting_mysql);

/// A Suprnova worker with default settings runs Suprnova's job and leaves
/// every job a Laravel fixture row queued as it was.
async fn a_worker_leaves_laravels_jobs(engine: Engine) {
    let _shared = Shared::on();
    let (db, _) = support::laravel(engine).await;
    crate::scaffold::migrate(&db.conn).await.expect("migrate");
    let _bound = support::bind(&db.conn);
    let laravel_jobs = support::rows(
        &db.conn,
        "SELECT id, queue, attempts, reserved_at, available_at FROM jobs ORDER BY id",
    )
    .await;
    assert!(!laravel_jobs.is_empty());
    // Every Laravel job is due, so only the worker's own filter keeps it off.
    db.conn
        .execute_unprepared("UPDATE jobs SET available_at = 0, reserved_at = NULL")
        .await
        .expect("make Laravel's jobs due");
    let laravel_jobs = support::rows(
        &db.conn,
        "SELECT id, queue, attempts, reserved_at, available_at FROM jobs ORDER BY id",
    )
    .await;

    suprnova::queue::worker::register_job::<ShipOrder>();
    Queue::set_failed_store(Arc::new(
        DatabaseFailedJobStore::new(db.conn.clone(), "failed_jobs".to_owned()).expect("store"),
    ));
    let driver = Arc::new(
        DatabaseQueueDriver::new(db.conn.clone(), "jobs".to_owned()).expect("the jobs driver"),
    );
    Queue::set_driver(driver.clone());
    Queue::push(ShipOrder { order: 3 }).await.expect("queue");
    let config = WorkerConfig {
        visibility_timeout: Duration::from_secs(30),
        poll_interval: Duration::from_millis(10),
        max_jobs: Some(1),
        queues: Vec::new(),
    };
    tokio::time::timeout(
        Duration::from_secs(60),
        run_worker(driver, config, CancellationToken::new()),
    )
    .await
    .expect("the worker ran one job in time")
    .expect("the worker");
    let after = support::rows(
        &db.conn,
        "SELECT id, queue, attempts, reserved_at, available_at FROM jobs ORDER BY id",
    )
    .await;
    assert_eq!(
        after, laravel_jobs,
        "{engine:?}: the worker reserved a Laravel job"
    );
    assert_eq!(
        support::count(&db.conn, "failed_jobs", "queue <> 'failing'").await,
        0
    );
}

on_every_engine!(a_worker_leaves_laravels_jobs =>
    ldb_009_a_suprnova_worker_does_not_reserve_laravels_jobs_sqlite,
    ldb_009_a_suprnova_worker_does_not_reserve_laravels_jobs_postgres,
    ldb_009_a_suprnova_worker_does_not_reserve_laravels_jobs_mysql);

/// Session garbage collection deletes Suprnova's expired session and no
/// session a Laravel fixture row holds, however old.
async fn session_gc_leaves_laravels_sessions(engine: Engine) {
    let (db, fixture) = support::laravel(engine).await;
    let _bound = support::bind(&db.conn);
    let driver = suprnova::session::driver::DatabaseSessionDriver::new(Duration::from_secs(60));
    {
        let _clock = suprnova::testing::TestClock::travel_to(
            chrono::Utc::now() - chrono::Duration::hours(5),
        );
        let session = suprnova::session::SessionData::new(
            "ldbexpired000000000000000000000000000000".into(),
            "csrf".into(),
        );
        driver.write(&session).await.expect("write an old session");
    }
    let collected = driver.gc().await.expect("gc");
    assert_eq!(collected, 1, "{engine:?}: Suprnova's expired session");
    for id in [&fixture.sessions.user, &fixture.sessions.guest] {
        assert_eq!(
            support::count(&db.conn, "sessions", &format!("id = '{id}'")).await,
            1,
            "{engine:?}: gc deleted Laravel's session {id}"
        );
    }
}

on_every_engine!(session_gc_leaves_laravels_sessions =>
    ldb_009_session_gc_leaves_laravels_sessions_sqlite,
    ldb_009_session_gc_leaves_laravels_sessions_postgres,
    ldb_009_session_gc_leaves_laravels_sessions_mysql);

/// The reproduced `unserialize` reads what Laravel and Suprnova store in
/// `job_batches.options`, and refuses what PHP refuses.
#[test]
fn ldb_009_the_reproduced_unserialize_reads_php_serialize() {
    assert_eq!(php_unserialize(b"a:0:{}"), Some(Php::Array(Vec::new())));
    assert_eq!(
        php_unserialize(br#"a:1:{s:8:"suprnova";s:2:"{}";}"#),
        Some(Php::Array(vec![(
            Php::Str(b"suprnova".to_vec()),
            Php::Str(b"{}".to_vec())
        )]))
    );
    assert_eq!(
        php_unserialize(b"a:2:{i:0;b:1;i:1;N;}"),
        Some(Php::Array(vec![
            (Php::Int(0), Php::Bool(true)),
            (Php::Int(1), Php::Null)
        ]))
    );
    assert_eq!(php_unserialize(b"d:1.5;"), Some(Php::Float(1.5)));
    assert_eq!(php_unserialize(br#"s:3:"ab";"#), None, "a wrong length");
    assert_eq!(php_unserialize(b"{\"json\":true}"), None);
}
