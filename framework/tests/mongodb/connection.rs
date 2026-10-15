//! PAR-182: the boot registers the connection when `MONGODB_URI` is set,
//! the `Mongo` facade answers the connection, the database, a typed
//! collection and a ping, and every failure is a `FrameworkError`: a
//! missing or malformed URI names `MONGODB_URI`, and a server that cannot
//! be reached fails the first call that needs it.
//!
//! The facade's connections live in the process container, so each test
//! that boots them runs alone in a child process (see
//! `support::run_alone_with`). The `mongodb_` tests need a server at
//! `MONGODB_TEST_URL` and are ignored without one.

use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use serde::{Deserialize, Serialize};
use suprnova::bson::{Document, doc};
use suprnova::{
    Config, DEFAULT_MONGO_CONNECTION, FrameworkError, Mongo, MongoConfig, MongoConnection,
    MongoConnectionConfig,
};

use crate::support::{PASSWORD, UNREACHABLE_URI, database_of, run_alone_with, test_url};

fn is_child() -> bool {
    crate::own_process::is_child()
}

fn names(error: &FrameworkError, what: &str) -> bool {
    error.to_string().contains(what)
}

// --- Without a URI, with a malformed one -------------------------------------

#[test]
fn without_mongodb_uri_every_facade_call_is_an_error_naming_it() {
    run_alone_with(
        "connection::without_mongodb_uri_every_facade_call_is_an_error_naming_it_child",
        &[],
    );
}

#[tokio::test]
async fn without_mongodb_uri_every_facade_call_is_an_error_naming_it_child() {
    if !is_child() {
        return;
    }
    // The boot has nothing to register and is not an error.
    Mongo::bootstrap()
        .await
        .expect("no URI, nothing to register");

    let error = Mongo::connection().expect_err("no connection");
    assert!(names(&error, "MONGODB_URI"), "{error}");
    let error = Mongo::connection_named(DEFAULT_MONGO_CONNECTION).expect_err("no connection");
    assert!(names(&error, "MONGODB_URI"), "{error}");
    let error = Mongo::database().expect_err("no database");
    assert!(names(&error, "MONGODB_URI"), "{error}");
    let error = Mongo::collection::<Document>("users").expect_err("no collection");
    assert!(names(&error, "MONGODB_URI"), "{error}");
    let error = Mongo::ping().await.expect_err("nothing to ping");
    assert!(names(&error, "MONGODB_URI"), "{error}");
}

#[test]
fn with_a_malformed_mongodb_uri_the_boot_and_the_facade_fail_naming_it() {
    run_alone_with(
        "connection::with_a_malformed_mongodb_uri_the_boot_and_the_facade_fail_naming_it_child",
        &[
            (
                "MONGODB_URI",
                &format!("http://app:{PASSWORD}@db.internal/shop"),
            ),
            ("MONGODB_DATABASE", "shop"),
        ],
    );
}

#[tokio::test]
async fn with_a_malformed_mongodb_uri_the_boot_and_the_facade_fail_naming_it_child() {
    if !is_child() {
        return;
    }
    let error = Mongo::bootstrap()
        .await
        .expect_err("the boot refuses the URI");
    assert!(names(&error, "MONGODB_URI"), "{error}");
    assert!(!names(&error, PASSWORD), "never the password: {error}");

    let error = Mongo::connection().expect_err("nothing was registered");
    assert!(names(&error, "MONGODB_URI"), "{error}");
    assert!(!names(&error, PASSWORD), "never the password: {error}");
}

// --- The boot registers the connection ---------------------------------------

#[test]
fn the_boot_registers_the_connection_when_mongodb_uri_is_set() {
    run_alone_with(
        "connection::the_boot_registers_the_connection_when_mongodb_uri_is_set_child",
        &[
            ("MONGODB_URI", UNREACHABLE_URI),
            ("MONGODB_DATABASE", "suprnova_boot"),
        ],
    );
}

#[tokio::test]
async fn the_boot_registers_the_connection_when_mongodb_uri_is_set_child() {
    if !is_child() {
        return;
    }
    // No server listens: the driver connects lazily, so the boot does not
    // need one.
    Mongo::bootstrap()
        .await
        .expect("the boot registers the connection");

    let connection = Mongo::connection().expect("registered at boot");
    assert_eq!(connection.name(), DEFAULT_MONGO_CONNECTION);
    assert_eq!(connection.database().name(), "suprnova_boot");
    assert_eq!(Mongo::database().expect("database").name(), "suprnova_boot");

    let users = Mongo::collection::<Document>("users").expect("collection");
    assert_eq!(users.name(), "users");
    assert_eq!(users.namespace().db, "suprnova_boot");

    let by_name = Mongo::connection_named(DEFAULT_MONGO_CONNECTION).expect("the default by name");
    assert_eq!(by_name.database().name(), "suprnova_boot");

    let error = Mongo::connection_named("reporting").expect_err("no such connection");
    assert!(names(&error, "'reporting'"), "{error}");
}

#[test]
fn a_connection_failure_surfaces_from_the_first_call_that_needs_the_server() {
    run_alone_with(
        "connection::a_connection_failure_surfaces_from_the_first_call_that_needs_the_server_child",
        &[
            ("MONGODB_URI", UNREACHABLE_URI),
            ("MONGODB_DATABASE", "suprnova_boot"),
        ],
    );
}

#[tokio::test]
async fn a_connection_failure_surfaces_from_the_first_call_that_needs_the_server_child() {
    if !is_child() {
        return;
    }
    Mongo::bootstrap().await.expect("the boot needs no server");
    let error = Mongo::ping().await.expect_err("nothing listens on port 1");
    assert!(
        names(&error, DEFAULT_MONGO_CONNECTION),
        "the error names the connection: {error}"
    );
    assert!(
        error.external_source().is_some(),
        "the driver's error stays reachable as the source: {error:?}"
    );
}

#[test]
fn a_driver_error_converts_into_a_framework_error_with_its_source() {
    run_alone_with(
        "connection::a_driver_error_converts_into_a_framework_error_with_its_source_child",
        &[
            ("MONGODB_URI", UNREACHABLE_URI),
            ("MONGODB_DATABASE", "suprnova_boot"),
        ],
    );
}

/// A handler's shape: the driver's own operation, propagated with `?`.
async fn count_users() -> Result<u64, FrameworkError> {
    let users = Mongo::collection::<Document>("users")?;
    Ok(users.count_documents(doc! {}).await?)
}

#[tokio::test]
async fn a_driver_error_converts_into_a_framework_error_with_its_source_child() {
    if !is_child() {
        return;
    }
    Mongo::bootstrap().await.expect("the boot needs no server");
    let error = count_users().await.expect_err("nothing listens on port 1");
    assert!(
        names(&error, "MongoDB"),
        "the error says where it came from: {error}"
    );
    assert!(
        error
            .external_source()
            .and_then(|source| source.downcast_ref::<suprnova::mongodb::driver::error::Error>())
            .is_some(),
        "the driver's error is the source: {error:?}"
    );
}

#[test]
fn the_boot_keeps_a_connection_the_application_registered() {
    run_alone_with(
        "connection::the_boot_keeps_a_connection_the_application_registered_child",
        &[
            ("MONGODB_URI", UNREACHABLE_URI),
            ("MONGODB_DATABASE", "from_the_environment"),
        ],
    );
}

#[tokio::test]
async fn the_boot_keeps_a_connection_the_application_registered_child() {
    if !is_child() {
        return;
    }
    let config = MongoConfig::builder()
        .uri(UNREACHABLE_URI)
        .database("from_the_bootstrap")
        .build()
        .expect("valid");
    Mongo::init_with(config)
        .await
        .expect("registered by the application");
    Mongo::bootstrap().await.expect("the boot");
    assert_eq!(
        Mongo::database().expect("database").name(),
        "from_the_bootstrap"
    );
}

#[test]
fn the_boot_registers_a_registered_mongo_config_without_mongodb_uri() {
    run_alone_with(
        "connection::the_boot_registers_a_registered_mongo_config_without_mongodb_uri_child",
        &[],
    );
}

#[tokio::test]
async fn the_boot_registers_a_registered_mongo_config_without_mongodb_uri_child() {
    if !is_child() {
        return;
    }
    Config::register(
        MongoConfig::builder()
            .uri(UNREACHABLE_URI)
            .database("from_config")
            .build()
            .expect("valid"),
    );
    Mongo::bootstrap()
        .await
        .expect("the boot reads the registered config");
    assert_eq!(Mongo::database().expect("database").name(), "from_config");

    // `init` reads the same registered config.
    Mongo::init().await.expect("init");
    assert_eq!(Mongo::database().expect("database").name(), "from_config");
}

#[test]
fn init_with_registers_named_connections() {
    crate::own_process::run_alone("connection::init_with_registers_named_connections_child");
}

#[tokio::test]
async fn init_with_registers_named_connections_child() {
    if !is_child() {
        return;
    }
    let config = MongoConfig::builder()
        .uri(UNREACHABLE_URI)
        .database("main")
        .connection(
            "reporting",
            MongoConnectionConfig::new(UNREACHABLE_URI, "reports"),
        )
        .build()
        .expect("valid");
    Mongo::init_with(config).await.expect("registered");

    assert_eq!(Mongo::database().expect("default").name(), "main");
    let reporting = Mongo::connection_named("reporting").expect("named");
    assert_eq!(reporting.name(), "reporting");
    assert_eq!(reporting.database().name(), "reports");
    assert_eq!(
        reporting.collection::<Document>("events").namespace().db,
        "reports"
    );
}

// --- No Tokio runtime ----------------------------------------------------------

/// Poll `future` once with a waker that does nothing, outside any runtime.
fn poll_once<F: Future>(future: F) -> Poll<F::Output> {
    let mut future = pin!(future);
    future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
}

#[test]
fn connecting_outside_a_tokio_runtime_is_an_error_not_a_panic() {
    let config = MongoConnectionConfig::new(UNREACHABLE_URI, "shop");
    let Poll::Ready(result) = poll_once(MongoConnection::connect("plain", &config)) else {
        panic!("the connect answers before it awaits anything");
    };
    let error = result.expect_err("the driver needs a Tokio runtime");
    assert!(names(&error, "Tokio"), "{error}");
}

// --- Against a server ----------------------------------------------------------

#[test]
#[ignore = "needs MONGODB_TEST_URL"]
fn mongodb_ping_answers_and_the_database_is_the_configured_one() {
    let url = test_url();
    let database = database_of(&url);
    run_alone_with(
        "connection::mongodb_ping_answers_and_the_database_is_the_configured_one_child",
        &[("MONGODB_URI", &url), ("MONGODB_DATABASE", &database)],
    );
}

#[tokio::test]
#[ignore = "needs MONGODB_TEST_URL"]
async fn mongodb_ping_answers_and_the_database_is_the_configured_one_child() {
    if !is_child() {
        return;
    }
    Mongo::bootstrap()
        .await
        .expect("the boot registers the connection");
    Mongo::ping().await.expect("the server answers ping");
    let expected = std::env::var("MONGODB_DATABASE").expect("the parent set it");
    assert_eq!(Mongo::database().expect("database").name(), expected);
    Mongo::connection()
        .expect("registered")
        .ping()
        .await
        .expect("the connection answers ping");
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Note {
    title: String,
    pages: i32,
}

#[test]
#[ignore = "needs MONGODB_TEST_URL"]
fn mongodb_a_typed_collection_round_trips_a_document() {
    let url = test_url();
    let database = database_of(&url);
    run_alone_with(
        "connection::mongodb_a_typed_collection_round_trips_a_document_child",
        &[("MONGODB_URI", &url), ("MONGODB_DATABASE", &database)],
    );
}

#[tokio::test]
#[ignore = "needs MONGODB_TEST_URL"]
async fn mongodb_a_typed_collection_round_trips_a_document_child() {
    if !is_child() {
        return;
    }
    Mongo::bootstrap()
        .await
        .expect("the boot registers the connection");
    let notes = Mongo::collection::<Note>("suprnova_par182_notes").expect("collection");
    notes.drop().await.expect("start from an empty collection");

    let note = Note {
        title: "first".to_owned(),
        pages: 3,
    };
    notes.insert_one(&note).await.expect("insert");
    let found = notes
        .find_one(doc! { "title": "first" })
        .await
        .expect("find");
    assert_eq!(found, Some(note));

    notes.drop().await.expect("clean up");
}
