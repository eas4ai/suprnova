//! PAR-182: `MongoConfig` reads `MONGODB_URI` and `MONGODB_DATABASE`, its
//! builder takes the URI, the database, the pool options and named
//! connections, and every bad value is an error that names its source,
//! never a panic and never an echo of the password.
//!
//! The tests that read the environment run alone in a child process
//! started with the variables they need (see `support::run_alone_with`).

use std::time::Duration;

use suprnova::{DEFAULT_MONGO_CONNECTION, FrameworkError, MongoConfig, MongoConnectionConfig};

use crate::support::{PASSWORD, run_alone_with};

fn is_child() -> bool {
    crate::own_process::is_child()
}

fn message(error: &FrameworkError) -> String {
    error.to_string()
}

// --- From the environment ----------------------------------------------------

#[test]
fn from_env_reads_mongodb_uri_and_mongodb_database() {
    run_alone_with(
        "configuration::from_env_reads_mongodb_uri_and_mongodb_database_child",
        &[
            ("MONGODB_URI", "mongodb://db.internal:27017/?maxPoolSize=20"),
            ("MONGODB_DATABASE", "shop"),
        ],
    );
}

#[test]
fn from_env_reads_mongodb_uri_and_mongodb_database_child() {
    if !is_child() {
        return;
    }
    let config = MongoConfig::from_env().expect("both variables are valid");
    assert_eq!(
        config.default.uri,
        "mongodb://db.internal:27017/?maxPoolSize=20"
    );
    assert_eq!(config.default.database, "shop");
    assert!(config.connections.is_empty());
    assert_eq!(
        config
            .connection(DEFAULT_MONGO_CONNECTION)
            .map(|c| c.database.as_str()),
        Some("shop"),
        "the default connection answers by its name"
    );

    // The builder starts from the same variables and overrides what it sets.
    let built = MongoConfig::builder()
        .max_pool_size(7)
        .build()
        .expect("the builder falls back to the environment");
    assert_eq!(built.default.uri, config.default.uri);
    assert_eq!(built.default.database, "shop");
    assert_eq!(built.default.max_pool_size, Some(7));
}

#[test]
fn from_env_takes_the_database_from_the_uri_when_mongodb_database_is_unset() {
    run_alone_with(
        "configuration::from_env_takes_the_database_from_the_uri_when_mongodb_database_is_unset_child",
        &[("MONGODB_URI", "mongodb://127.0.0.1:27017/inventory")],
    );
}

#[test]
fn from_env_takes_the_database_from_the_uri_when_mongodb_database_is_unset_child() {
    if !is_child() {
        return;
    }
    let config = MongoConfig::from_env().expect("the URI names a database");
    assert_eq!(config.default.database, "inventory");
}

#[test]
fn mongodb_database_wins_over_the_database_in_the_uri() {
    run_alone_with(
        "configuration::mongodb_database_wins_over_the_database_in_the_uri_child",
        &[
            ("MONGODB_URI", "mongodb://127.0.0.1:27017/inventory"),
            ("MONGODB_DATABASE", "shop"),
        ],
    );
}

#[test]
fn mongodb_database_wins_over_the_database_in_the_uri_child() {
    if !is_child() {
        return;
    }
    let config = MongoConfig::from_env().expect("both variables are valid");
    assert_eq!(config.default.database, "shop");
}

#[test]
fn from_env_without_mongodb_uri_is_an_error_naming_it() {
    let child = "configuration::from_env_without_mongodb_uri_is_an_error_naming_it_child";
    // Unset, and set to blanks, which a `.env` line `MONGODB_URI=` gives.
    run_alone_with(child, &[("MONGODB_DATABASE", "shop")]);
    run_alone_with(
        child,
        &[("MONGODB_URI", "  "), ("MONGODB_DATABASE", "shop")],
    );
}

#[test]
fn from_env_without_mongodb_uri_is_an_error_naming_it_child() {
    if !is_child() {
        return;
    }
    let error = MongoConfig::from_env().expect_err("there is no URI to connect to");
    assert!(
        message(&error).contains("MONGODB_URI"),
        "the error names the variable: {error}"
    );
}

#[test]
fn a_malformed_mongodb_uri_is_an_error_naming_it() {
    run_alone_with(
        "configuration::a_malformed_mongodb_uri_is_an_error_naming_it_child",
        &[
            ("MONGODB_URI", &format!("postgres://app:{PASSWORD}@db/shop")),
            ("MONGODB_DATABASE", "shop"),
        ],
    );
}

#[test]
fn a_malformed_mongodb_uri_is_an_error_naming_it_child() {
    if !is_child() {
        return;
    }
    let error = MongoConfig::from_env().expect_err("the URI is not a MongoDB URI");
    let text = message(&error);
    assert!(
        text.contains("MONGODB_URI"),
        "the error names the variable: {text}"
    );
    assert!(
        !text.contains(PASSWORD),
        "the error never shows the password: {text}"
    );
}

#[test]
fn a_missing_database_is_an_error_naming_mongodb_database() {
    run_alone_with(
        "configuration::a_missing_database_is_an_error_naming_mongodb_database_child",
        &[("MONGODB_URI", "mongodb://127.0.0.1:27017")],
    );
}

#[test]
fn a_missing_database_is_an_error_naming_mongodb_database_child() {
    if !is_child() {
        return;
    }
    let error = MongoConfig::from_env().expect_err("neither variable names a database");
    assert!(
        message(&error).contains("MONGODB_DATABASE"),
        "the error names the variable: {error}"
    );
}

#[test]
fn an_invalid_mongodb_database_is_an_error_naming_it() {
    run_alone_with(
        "configuration::an_invalid_mongodb_database_is_an_error_naming_it_child",
        &[
            ("MONGODB_URI", "mongodb://127.0.0.1:27017"),
            ("MONGODB_DATABASE", "my.shop"),
        ],
    );
}

#[test]
fn an_invalid_mongodb_database_is_an_error_naming_it_child() {
    if !is_child() {
        return;
    }
    let error = MongoConfig::from_env().expect_err("a dot is not allowed in a database name");
    assert!(
        message(&error).contains("MONGODB_DATABASE"),
        "the error names the variable: {error}"
    );
}

// --- The builder -------------------------------------------------------------

#[test]
fn the_builder_sets_the_uri_database_pool_options_and_named_connections() {
    let reporting = MongoConnectionConfig {
        max_pool_size: Some(4),
        ..MongoConnectionConfig::new("mongodb://reports.internal:27017", "reports")
    };
    let config = MongoConfig::builder()
        .uri("mongodb://db.internal:27017")
        .database("shop")
        .max_pool_size(50)
        .min_pool_size(5)
        .max_idle_time(Duration::from_secs(60))
        .max_connecting(4)
        .connect_timeout(Duration::from_secs(3))
        .server_selection_timeout(Duration::from_secs(5))
        .connection("reporting", reporting)
        .build()
        .expect("every value is valid");

    let default = &config.default;
    assert_eq!(default.uri, "mongodb://db.internal:27017");
    assert_eq!(default.database, "shop");
    assert_eq!(default.max_pool_size, Some(50));
    assert_eq!(default.min_pool_size, Some(5));
    assert_eq!(default.max_idle_time, Some(Duration::from_secs(60)));
    assert_eq!(default.max_connecting, Some(4));
    assert_eq!(default.connect_timeout, Some(Duration::from_secs(3)));
    assert_eq!(
        default.server_selection_timeout,
        Some(Duration::from_secs(5))
    );

    let named = config
        .connection("reporting")
        .expect("the named connection");
    assert_eq!(named.uri, "mongodb://reports.internal:27017");
    assert_eq!(named.database, "reports");
    assert_eq!(named.max_pool_size, Some(4));
    assert!(config.connection("missing").is_none());
    config.validate().expect("a built configuration is valid");
}

#[test]
fn the_builder_refuses_a_malformed_uri_naming_the_connection() {
    for uri in [
        "localhost:27017",
        "http://db.internal:27017",
        "mongodb://db.internal:notaport",
        "mongodb+srv://cluster.example.com:27017",
    ] {
        let error = MongoConfig::builder()
            .uri(uri)
            .database("shop")
            .build()
            .expect_err(uri);
        assert!(
            message(&error).contains(DEFAULT_MONGO_CONNECTION),
            "{uri}: the error names the connection: {error}"
        );
    }

    let error = MongoConfig::builder()
        .uri("mongodb://db.internal:27017")
        .database("shop")
        .connection(
            "reporting",
            MongoConnectionConfig::new("mysql://db.internal/reports", "reports"),
        )
        .build()
        .expect_err("the named connection's URI is not a MongoDB URI");
    assert!(
        message(&error).contains("'reporting'"),
        "the error names the connection: {error}"
    );
}

#[test]
fn the_builder_refuses_invalid_database_names() {
    for database in [
        "",
        "my.shop",
        "a/b",
        "with space",
        "dollar$",
        &"a".repeat(64),
    ] {
        let result = MongoConfig::builder()
            .uri("mongodb://db.internal:27017")
            .database(database)
            .build();
        assert!(result.is_err(), "{database:?} is not a database name");
    }
    MongoConfig::builder()
        .uri("mongodb://db.internal:27017")
        .database("a".repeat(63))
        .build()
        .expect("63 bytes is the longest name MongoDB takes");
}

#[test]
fn the_builder_refuses_pool_options_the_driver_cannot_use() {
    let base = || {
        MongoConfig::builder()
            .uri("mongodb://db.internal:27017")
            .database("shop")
    };
    for (what, built) in [
        ("max_pool_size", base().max_pool_size(0).build()),
        ("max_connecting", base().max_connecting(0).build()),
        (
            "min_pool_size",
            base().min_pool_size(20).max_pool_size(10).build(),
        ),
    ] {
        let error = built.expect_err(what);
        assert!(
            message(&error).contains(what),
            "the error names {what}: {error}"
        );
    }
}

#[test]
fn the_builder_refuses_connection_names_it_cannot_tell_apart() {
    let base = || {
        MongoConfig::builder()
            .uri("mongodb://db.internal:27017")
            .database("shop")
    };
    let other = || MongoConnectionConfig::new("mongodb://db.internal:27017", "other");
    base()
        .connection("", other())
        .build()
        .expect_err("an empty name");
    let error = base()
        .connection(DEFAULT_MONGO_CONNECTION, other())
        .build()
        .expect_err("the default connection's name");
    assert!(message(&error).contains(DEFAULT_MONGO_CONNECTION));
}

#[test]
fn debug_output_never_shows_the_password() {
    let uri = format!("mongodb://app:{PASSWORD}@db.internal:27017/?authSource=admin");
    let config = MongoConfig::builder()
        .uri(&uri)
        .database("shop")
        .connection("reporting", MongoConnectionConfig::new(&uri, "reports"))
        .build()
        .expect("valid");
    let shown = format!("{config:?}");
    assert!(!shown.contains(PASSWORD), "{shown}");
    assert!(
        shown.contains("db.internal:27017"),
        "the host stays visible: {shown}"
    );
}

#[test]
fn a_uri_error_never_quotes_a_secret_from_the_query_string() {
    // The driver quotes a malformed `authMechanismProperties` entry in its
    // message; the entry can be a token.
    let uri = format!("mongodb://db.internal:27017/?authMechanismProperties={PASSWORD}");
    let error = MongoConfig::builder()
        .uri(&uri)
        .database("shop")
        .build()
        .expect_err("the entry is not a key:value pair");
    let text = message(&error);
    assert!(
        text.contains("authMechanismProperties"),
        "the reason stays: {text}"
    );
    assert!(!text.contains(PASSWORD), "never the secret: {text}");
}
