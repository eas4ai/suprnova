//! #132: `SESSION_TABLE` names the session table. `SessionConfig::from_env`
//! is infallible and runs per request, so `Config::init` is where a bad
//! name has to stop the app: at boot, with the variable named.

use suprnova::Config;
use suprnova::config::__reset_loaded_keys_for_tests;
use suprnova::session::SessionConfig;

use crate::env_snapshot::{EnvSnapshot, set_env};

/// Boots `Config::init` from an empty directory, so only the process
/// environment is in play.
fn boot() -> Result<suprnova::config::Environment, suprnova::FrameworkError> {
    let root = tempfile::tempdir().expect("tempdir");
    Config::init(root.path())
}

#[test]
fn an_invalid_session_table_fails_boot_and_names_the_variable() {
    let _env = crate::env_lock::lock_env();
    __reset_loaded_keys_for_tests();
    let _snap = EnvSnapshot::capture(&["SESSION_TABLE"]);
    set_env("SESSION_TABLE", Some("bad name"));

    let error = boot().expect_err("a table name with a space must fail boot");

    let message = error.to_string();
    assert!(message.contains("SESSION_TABLE"), "{message}");
    assert!(message.contains("bad name"), "{message}");
}

#[test]
fn an_unset_session_table_boots_with_the_default_table() {
    let _env = crate::env_lock::lock_env();
    __reset_loaded_keys_for_tests();
    let _snap = EnvSnapshot::capture(&["SESSION_TABLE"]);
    set_env("SESSION_TABLE", None);

    boot().expect("an unset SESSION_TABLE boots");

    assert_eq!(SessionConfig::from_env().table_name, "sessions");
}

#[test]
fn a_valid_session_table_boots_and_reaches_the_session_config() {
    let _env = crate::env_lock::lock_env();
    __reset_loaded_keys_for_tests();
    let _snap = EnvSnapshot::capture(&["SESSION_TABLE"]);
    set_env("SESSION_TABLE", Some("app_sessions"));

    boot().expect("a plain identifier boots");

    assert_eq!(SessionConfig::from_env().table_name, "app_sessions");
}

/// A boot that fails registers nothing. `Config::init` used to register
/// `AppConfig` before it checked the rest, so a caller that handled the
/// error kept a half-applied configuration.
#[test]
fn a_failed_boot_leaves_the_registered_configuration_alone() {
    let _env = crate::env_lock::lock_env();
    __reset_loaded_keys_for_tests();
    let _snap = EnvSnapshot::capture(&["APP_NAME", "SESSION_TABLE"]);
    suprnova::config::repository::register(
        suprnova::AppConfig::builder()
            .name("before-the-boot")
            .build(),
    );
    set_env("APP_NAME", Some("from-the-failed-boot"));
    set_env("SESSION_TABLE", Some("bad name"));

    boot().expect_err("a bad session table fails the boot");

    let app = suprnova::Config::get::<suprnova::AppConfig>().expect("an AppConfig is registered");
    assert_eq!(app.name, "before-the-boot");
}
