//! PAR-188: `SESSION_DRIVER=mongodb` selects a `SessionStore` over a
//! `sessions` collection: read and write with the user id, the guards and
//! the last activity, the destroys, the two-factor migration, and `gc`.
//!
//! The unignored tests read the documents the store sends (rendered without
//! a server), the selection by the environment (in a child process) and
//! the errors of a server that does not answer. The `mongodb_` tests run
//! each falsifier against the server at `MONGODB_TEST_URL`.

use std::time::Duration;

use chrono::{DateTime, Utc};
use suprnova::bson::{Bson, Document, doc};
use suprnova::session::SessionDriver;
use suprnova::testing::TestClock;
use suprnova::{
    Mongo, MongoSessionDriver, SessionConfig, SessionData, SessionMiddleware,
    SessionMigrationError, SessionStore, generate_session_id,
};

use crate::store_support::{drop_collections, run_alone_with_drivers, server, unique, unreachable};
use crate::support::UNREACHABLE_URI;

fn is_child() -> bool {
    crate::own_process::is_child()
}

fn moment() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-10-10T12:00:00.250Z")
        .expect("valid time")
        .with_timezone(&Utc)
}

fn bson_time(at: DateTime<Utc>) -> suprnova::bson::DateTime {
    suprnova::bson::DateTime::from_millis(at.timestamp_millis())
}

const LIFETIME: Duration = Duration::from_secs(2 * 60 * 60);

fn signed_in(user: Option<&str>) -> SessionData {
    let mut session = SessionData::new(generate_session_id(), "csrf-token".to_owned());
    session.user_id = user.map(str::to_owned);
    session.put("cart", vec![1, 2, 3]);
    session
}

// --- Selection by the environment ---------------------------------------------

#[test]
fn session_driver_parses_database_and_mongodb() {
    assert_eq!(SessionDriver::default(), SessionDriver::Database);
    assert_eq!(
        SessionDriver::parse("database").unwrap(),
        SessionDriver::Database
    );
    assert_eq!(
        SessionDriver::parse(" MongoDB ").unwrap(),
        SessionDriver::MongoDb
    );
    let error = SessionDriver::parse("redis").expect_err("not a session driver");
    let message = error.to_string();
    assert!(message.contains("redis"), "{message}");
    assert!(
        message.contains("database") && message.contains("mongodb"),
        "{message}"
    );
}

#[test]
fn session_driver_mongodb_makes_the_middleware_store_sessions_in_mongodb() {
    run_alone_with_drivers(
        "session::session_driver_mongodb_makes_the_middleware_store_sessions_in_mongodb_child",
        &[
            ("SESSION_DRIVER", "mongodb"),
            ("MONGODB_URI", UNREACHABLE_URI),
            ("MONGODB_DATABASE", "suprnova_sessions"),
        ],
    );
}

#[tokio::test]
async fn session_driver_mongodb_makes_the_middleware_store_sessions_in_mongodb_child() {
    if !is_child() {
        return;
    }
    let config = SessionConfig::from_env();
    assert_eq!(config.driver, SessionDriver::MongoDb);
    Mongo::bootstrap()
        .await
        .expect("the boot registers the connection");
    let store = SessionMiddleware::new(config).store();
    let error = store
        .read(&generate_session_id())
        .await
        .expect_err("no server answers");
    assert!(error.to_string().contains("MongoDB"), "{error}");
}

#[test]
fn without_session_driver_the_database_driver_is_used() {
    run_alone_with_drivers(
        "session::without_session_driver_the_database_driver_is_used_child",
        &[],
    );
}

#[test]
fn without_session_driver_the_database_driver_is_used_child() {
    if !is_child() {
        return;
    }
    assert_eq!(SessionConfig::from_env().driver, SessionDriver::Database);
}

#[test]
fn the_boot_refuses_a_session_driver_it_does_not_know() {
    run_alone_with_drivers(
        "session::the_boot_refuses_a_session_driver_it_does_not_know_child",
        &[("SESSION_DRIVER", "mongo")],
    );
}

#[test]
fn the_boot_refuses_a_session_driver_it_does_not_know_child() {
    if !is_child() {
        return;
    }
    // From the infallible SessionConfig::from_env, the typo would be the
    // database driver; Config::init stops the boot instead.
    let root = tempfile::tempdir().expect("an empty project root");
    let error = suprnova::Config::init(root.path()).expect_err("SESSION_DRIVER=mongo is a typo");
    let message = error.to_string();
    assert!(message.contains("SESSION_DRIVER"), "{message}");
    assert!(message.contains("mongodb"), "the accepted names: {message}");
}

#[test]
fn the_mongodb_driver_before_the_boot_names_mongodb_uri() {
    run_alone_with_drivers(
        "session::the_mongodb_driver_before_the_boot_names_mongodb_uri_child",
        &[("SESSION_DRIVER", "mongodb")],
    );
}

#[tokio::test]
async fn the_mongodb_driver_before_the_boot_names_mongodb_uri_child() {
    if !is_child() {
        return;
    }
    // The middleware is built in the application's bootstrap, before the
    // boot registers the connection: the store finds it on each call.
    let store = SessionMiddleware::new(SessionConfig::from_env()).store();
    let error = store
        .read(&generate_session_id())
        .await
        .expect_err("no connection registered");
    assert!(error.to_string().contains("MONGODB_URI"), "{error}");
}

// --- The documents the store sends ---------------------------------------------

#[tokio::test]
async fn a_write_stores_the_payload_user_guards_and_last_activity() {
    let driver = MongoSessionDriver::with_connection(LIFETIME, &unreachable().await, "sessions")
        .expect("valid collection");
    let now = moment();
    let mut session = signed_in(Some("7"));
    session.set_auth_guard_for_test("admin", "9", None);

    let fresh = driver.rendered_write(&session, now).expect("rendered");
    assert_eq!(
        fresh.get_document("filter").unwrap(),
        &doc! { "session_id": session.id.as_str() }
    );
    assert!(
        fresh.get_bool("upsert").unwrap(),
        "a new session is created"
    );
    let set = fresh
        .get_document("update")
        .unwrap()
        .get_document("$set")
        .unwrap();
    assert_eq!(set.get_str("user_id").unwrap(), "7");
    assert_eq!(
        set.get_array("guards").unwrap(),
        &vec![Bson::Document(doc! { "guard": "admin", "user_id": "9" })]
    );
    assert_eq!(*set.get_datetime("last_activity").unwrap(), bson_time(now));
    assert!(!set.get_str("payload").unwrap().is_empty());

    session.loaded_from_store = true;
    let loaded = driver.rendered_write(&session, now).expect("rendered");
    assert!(
        !loaded.get_bool("upsert").unwrap(),
        "a loaded session is never recreated after a revocation"
    );

    let guest = driver
        .rendered_write(&signed_in(None), now)
        .expect("rendered");
    let set = guest
        .get_document("update")
        .unwrap()
        .get_document("$set")
        .unwrap();
    assert_eq!(set.get("user_id"), Some(&Bson::Null));
    assert_eq!(set.get_array("guards").unwrap(), &Vec::<Bson>::new());
}

#[tokio::test]
async fn the_two_factor_migration_renames_the_session_in_one_update() {
    let driver = MongoSessionDriver::with_connection(LIFETIME, &unreachable().await, "sessions")
        .expect("valid collection");
    let now = moment();
    let session = signed_in(Some("7"));
    let rendered = driver
        .rendered_migration("old-id", &session, now)
        .expect("rendered");
    assert_eq!(
        rendered.get_document("filter").unwrap(),
        &doc! { "session_id": "old-id" }
    );
    let set = rendered
        .get_document("update")
        .unwrap()
        .get_document("$set")
        .unwrap();
    assert_eq!(set.get_str("session_id").unwrap(), session.id);
    assert_eq!(set.get_str("user_id").unwrap(), "7");
}

#[tokio::test]
async fn gc_removes_only_sessions_idle_past_the_lifetime() {
    let driver = MongoSessionDriver::with_connection(LIFETIME, &unreachable().await, "sessions")
        .expect("valid collection");
    let now = moment();
    assert_eq!(
        driver.rendered_gc(now),
        doc! { "filter": {
            "session_id": { "$exists": true },
            "last_activity": { "$lt": bson_time(now - chrono::Duration::hours(2)) },
        } }
    );
}

#[tokio::test]
async fn a_server_that_does_not_answer_fails_the_store_as_mongodb() {
    let driver = MongoSessionDriver::with_connection(LIFETIME, &unreachable().await, "sessions")
        .expect("valid collection");
    let error = driver
        .write(&signed_in(Some("7")))
        .await
        .expect_err("no server");
    assert!(error.to_string().contains("MongoDB"), "{error}");
    let error = driver.gc().await.expect_err("no server");
    assert!(error.to_string().contains("MongoDB"), "{error}");
    match driver
        .migrate_two_factor_session("old-id", &signed_in(Some("7")))
        .await
    {
        Err(SessionMigrationError::RolledBack(error)) => {
            assert!(error.to_string().contains("MongoDB"), "{error}")
        }
        other => panic!("no server took the update, so nothing changed: {other:?}"),
    }
    assert!(MongoSessionDriver::with_connection(LIFETIME, &unreachable().await, "").is_err());
}

// --- Against a server --------------------------------------------------------------

async fn driver_on_server(
    lifetime: Duration,
) -> (MongoSessionDriver, suprnova::MongoConnection, String) {
    let connection = server().await;
    let name = unique("par188_sessions");
    let driver =
        MongoSessionDriver::with_connection(lifetime, &connection, &name).expect("valid name");
    (driver, connection, name)
}

#[tokio::test]
#[ignore = "needs MONGODB_TEST_URL"]
async fn mongodb_a_written_session_reads_back_with_its_payload() {
    let (driver, connection, name) = driver_on_server(LIFETIME).await;
    let mut session = signed_in(Some("7"));
    session.set_auth_guard_for_test("admin", "9", None);
    driver.write(&session).await.expect("write");

    let read = driver.read(&session.id).await.unwrap().expect("stored");
    assert_eq!(read.id, session.id);
    assert_eq!(read.user_id.as_deref(), Some("7"));
    assert_eq!(read.csrf_token, "csrf-token");
    assert_eq!(read.get::<Vec<i32>>("cart"), Some(vec![1, 2, 3]));
    assert!(read.is_signed_in_as("admin", "9"));
    assert!(read.loaded_from_store);
    assert!(driver.read(&generate_session_id()).await.unwrap().is_none());

    // An update keeps one document per session.
    let mut changed = read.clone();
    changed.put("cart", vec![4]);
    driver.write(&changed).await.expect("update");
    assert_eq!(
        driver
            .read(&session.id)
            .await
            .unwrap()
            .unwrap()
            .get::<Vec<i32>>("cart"),
        Some(vec![4])
    );
    assert_eq!(
        connection
            .collection::<Document>(&name)
            .count_documents(doc! { "session_id": session.id.as_str() })
            .await
            .unwrap(),
        1
    );

    driver.destroy(&session.id).await.expect("destroy");
    assert!(driver.read(&session.id).await.unwrap().is_none());
    // A loaded session is not recreated by a write after its destroy.
    driver.write(&changed).await.expect("the write declines");
    assert!(driver.read(&session.id).await.unwrap().is_none());
    drop_collections(&connection, &[&name]).await;
}

#[tokio::test]
#[ignore = "needs MONGODB_TEST_URL"]
async fn mongodb_destroy_for_user_removes_that_users_sessions_and_no_other() {
    let (driver, connection, name) = driver_on_server(LIFETIME).await;
    let mine = [signed_in(Some("7")), signed_in(Some("7"))];
    let theirs = signed_in(Some("8"));
    let mut admin_seven = signed_in(None);
    admin_seven.set_auth_guard_for_test("admin", "7", None);
    for session in mine.iter().chain([&theirs, &admin_seven]) {
        driver.write(session).await.expect("write");
    }

    assert_eq!(driver.destroy_for_user("7").await.unwrap(), 2);
    for session in &mine {
        assert!(driver.read(&session.id).await.unwrap().is_none());
    }
    assert!(
        driver.read(&theirs.id).await.unwrap().is_some(),
        "user 8 stays"
    );
    assert!(
        driver.read(&admin_seven.id).await.unwrap().is_some(),
        "admin 7 is another user"
    );

    let destroyed = driver
        .destroy_guard_sessions("admin", "7")
        .await
        .expect("admin sessions");
    assert_eq!(destroyed.count, 1);
    assert_eq!(destroyed.ids, vec![admin_seven.id.clone()]);

    let current = signed_in(Some("8"));
    let other_device = signed_in(Some("8"));
    driver.write(&current).await.unwrap();
    driver.write(&other_device).await.unwrap();
    let guard = suprnova::Auth::default_guard_name();
    let destroyed = driver
        .destroy_other_guard_sessions(&guard, "8", &current.id)
        .await
        .expect("other devices");
    assert_eq!(destroyed.count, 2, "theirs and the other device");
    assert!(destroyed.ids.contains(&other_device.id));
    assert!(
        driver.read(&current.id).await.unwrap().is_some(),
        "the current one stays"
    );
    drop_collections(&connection, &[&name]).await;
}

#[tokio::test]
#[ignore = "needs MONGODB_TEST_URL"]
async fn mongodb_gc_removes_idle_sessions_and_keeps_active_ones() {
    let (driver, connection, name) = driver_on_server(Duration::from_secs(60)).await;
    let clock = TestClock::freeze();
    let idle = signed_in(Some("7"));
    driver.write(&idle).await.unwrap();
    clock.advance(chrono::Duration::seconds(50));
    let active = signed_in(Some("8"));
    driver.write(&active).await.unwrap();
    clock.advance(chrono::Duration::seconds(20));

    assert!(
        driver.read(&idle.id).await.unwrap().is_none(),
        "idle past the lifetime"
    );
    driver.write(&idle).await.unwrap();
    clock.advance(chrono::Duration::seconds(61));
    driver.write(&active).await.unwrap();
    assert_eq!(driver.gc().await.unwrap(), 1, "the idle one goes");
    assert!(
        driver.read(&active.id).await.unwrap().is_some(),
        "the active one stays"
    );
    drop_collections(&connection, &[&name]).await;
}

#[tokio::test]
#[ignore = "needs MONGODB_TEST_URL"]
async fn mongodb_migrate_two_factor_session_keeps_the_pending_factor() {
    let (driver, connection, name) = driver_on_server(LIFETIME).await;
    let mut pending = signed_in(None);
    pending.put("_two_factor_pending_user_id", "7");
    driver.write(&pending).await.unwrap();

    let mut promoted = pending.clone();
    promoted.rotate_id(generate_session_id());
    promoted.put("factor", "totp");
    driver
        .migrate_two_factor_session(&pending.id, &promoted)
        .await
        .expect("migrated");
    assert!(
        driver.read(&pending.id).await.unwrap().is_none(),
        "the old id is gone"
    );
    let read = driver
        .read(&promoted.id)
        .await
        .unwrap()
        .expect("the new id");
    assert_eq!(
        read.get::<String>("_two_factor_pending_user_id").as_deref(),
        Some("7")
    );
    assert_eq!(read.get::<String>("factor").as_deref(), Some("totp"));

    // A missing old session creates nothing.
    let mut stray = signed_in(Some("7"));
    stray.rotate_id(generate_session_id());
    match driver.migrate_two_factor_session("absent", &stray).await {
        Err(SessionMigrationError::RolledBack(_)) => {}
        other => panic!("a missing old session is rolled back: {other:?}"),
    }
    assert!(driver.read(&stray.id).await.unwrap().is_none());

    // A new id that already exists leaves both sessions as they were.
    let taken = signed_in(Some("8"));
    driver.write(&taken).await.unwrap();
    let mut onto_taken = taken.clone();
    onto_taken.user_id = Some("7".to_owned());
    match driver
        .migrate_two_factor_session(&promoted.id, &onto_taken)
        .await
    {
        Err(SessionMigrationError::RolledBack(_)) => {}
        other => panic!("the server refused the duplicate id: {other:?}"),
    }
    assert!(driver.read(&promoted.id).await.unwrap().is_some());
    assert_eq!(
        driver
            .read(&taken.id)
            .await
            .unwrap()
            .unwrap()
            .user_id
            .as_deref(),
        Some("8")
    );
    drop_collections(&connection, &[&name]).await;
}
