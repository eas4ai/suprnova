//! Foundation backend reachability checks.
//!
//! These tests intentionally do not skip when a backend is unavailable. The CI gate
//! runs them with all backend features enabled and canonical test URLs exported.

#[cfg(any(feature = "seaorm-postgres", feature = "seaorm-mysql"))]
use std::env;

fn required_backend_url(name: &str, value: Option<String>) -> Result<String, String> {
    match value {
        Some(url) if !url.trim().is_empty() => Ok(url),
        _ => Err(format!(
            "{name} must be set to a reachable CI backend; refusing to skip the foundation gate"
        )),
    }
}

#[cfg(any(feature = "seaorm-postgres", feature = "seaorm-mysql"))]
fn configured_backend_url(name: &str) -> String {
    required_backend_url(name, env::var(name).ok()).unwrap_or_else(|message| panic!("{message}"))
}

#[test]
fn missing_postgres_configuration_is_rejected() {
    let error = required_backend_url("MAGNETAR_POSTGRES_TEST_URL", None)
        .expect_err("missing Postgres configuration must be an error");
    assert!(error.contains("MAGNETAR_POSTGRES_TEST_URL"));
    assert!(error.contains("refusing to skip"));
}

#[test]
fn missing_mysql_configuration_is_rejected() {
    let error = required_backend_url("MAGNETAR_MYSQL_TEST_URL", None)
        .expect_err("missing MySQL configuration must be an error");
    assert!(error.contains("MAGNETAR_MYSQL_TEST_URL"));
    assert!(error.contains("refusing to skip"));
}

#[test]
fn blank_backend_configuration_is_rejected() {
    for name in ["MAGNETAR_POSTGRES_TEST_URL", "MAGNETAR_MYSQL_TEST_URL"] {
        let error = required_backend_url(name, Some("  \t".to_owned()))
            .expect_err("blank backend configuration must be an error");
        assert!(error.contains(name));
        assert!(error.contains("refusing to skip"));
    }
}

#[test]
fn configured_backend_url_is_accepted() {
    for (name, url) in [
        (
            "MAGNETAR_POSTGRES_TEST_URL",
            "postgres://postgres:postgres@127.0.0.1:5432/magnetar",
        ),
        (
            "MAGNETAR_MYSQL_TEST_URL",
            "mysql://root:root@127.0.0.1:3306/magnetar",
        ),
    ] {
        assert_eq!(
            required_backend_url(name, Some(url.to_owned())),
            Ok(url.to_owned())
        );
    }
}

#[cfg(feature = "seaorm-postgres")]
#[tokio::test]
async fn postgres_backend_is_reachable() {
    use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};

    let url = configured_backend_url("MAGNETAR_POSTGRES_TEST_URL");
    let database = Database::connect(&url)
        .await
        .unwrap_or_else(|error| panic!("Postgres foundation backend cannot connect: {error}"));
    database.execute_raw(Statement::from_string(DbBackend::Postgres, "SELECT 1"))
        .await
        .unwrap_or_else(|error| panic!("Postgres foundation backend rejected SELECT 1: {error}"));
}

#[cfg(feature = "seaorm-mysql")]
#[tokio::test]
async fn mysql_backend_is_reachable() {
    use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};

    let url = configured_backend_url("MAGNETAR_MYSQL_TEST_URL");
    let database = Database::connect(&url)
        .await
        .unwrap_or_else(|error| panic!("MySQL foundation backend cannot connect: {error}"));
    database.execute_raw(Statement::from_string(DbBackend::MySql, "SELECT 1"))
        .await
        .unwrap_or_else(|error| panic!("MySQL foundation backend rejected SELECT 1: {error}"));
}
