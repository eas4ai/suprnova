//! HRD-009: the server caps concurrent connections finitely by default.

use suprnova::config::ServerConfig;
use suprnova::config::providers::{
    DEFAULT_MAX_CONNECTIONS, DEFAULT_MAX_CONNECTIONS_ON_MISCONFIGURATION,
};

/// Sets `SERVER_MAX_CONNECTIONS` to `value`, or removes it, and returns the
/// cap `ServerConfig::from_env` reads. The caller holds the env lock and a
/// snapshot of the variable.
fn cap_from_env(value: Option<&str>) -> Option<usize> {
    crate::env_snapshot::set_env("SERVER_MAX_CONNECTIONS", value);
    ServerConfig::from_env().max_connections
}

/// With `SERVER_MAX_CONNECTIONS` unset the cap is the finite default, not
/// unbounded.
#[tokio::test]
async fn hardening_an_unset_connection_cap_is_finite() {
    let _env = crate::env_lock::lock_env_async().await;
    let _snapshot = crate::env_snapshot::EnvSnapshot::capture(&["SERVER_MAX_CONNECTIONS"]);
    crate::env_snapshot::set_env("SERVER_MAX_CONNECTIONS", None);

    let config = ServerConfig::from_env();
    assert_eq!(DEFAULT_MAX_CONNECTIONS, 10_000);
    assert_eq!(
        config.max_connections,
        Some(DEFAULT_MAX_CONNECTIONS),
        "an unset SERVER_MAX_CONNECTIONS leaves connections unbounded"
    );
}

/// A blank value is unset, so it gets the finite default too. A `.env`
/// line such as `SERVER_MAX_CONNECTIONS=` sets the variable to an empty
/// string, and a trailing space is invisible in the file.
#[tokio::test]
async fn hardening_a_blank_connection_cap_is_finite() {
    let _env = crate::env_lock::lock_env_async().await;
    let _snapshot = crate::env_snapshot::EnvSnapshot::capture(&["SERVER_MAX_CONNECTIONS"]);

    assert_eq!(cap_from_env(Some("")), Some(DEFAULT_MAX_CONNECTIONS));
    assert_eq!(cap_from_env(Some("   ")), Some(DEFAULT_MAX_CONNECTIONS));
}

/// Zero would admit no connection at all, so it is a misconfigured cap and
/// gets the misconfiguration fallback, never "no cap".
#[tokio::test]
async fn hardening_a_zero_connection_cap_falls_back_to_a_finite_cap() {
    let _env = crate::env_lock::lock_env_async().await;
    let _snapshot = crate::env_snapshot::EnvSnapshot::capture(&["SERVER_MAX_CONNECTIONS"]);

    assert_eq!(
        cap_from_env(Some("0")),
        Some(DEFAULT_MAX_CONNECTIONS_ON_MISCONFIGURATION)
    );
}

/// A value that does not parse as a count is a misconfigured cap and gets
/// the misconfiguration fallback, never "no cap".
#[tokio::test]
async fn hardening_an_unparseable_connection_cap_falls_back_to_a_finite_cap() {
    let _env = crate::env_lock::lock_env_async().await;
    let _snapshot = crate::env_snapshot::EnvSnapshot::capture(&["SERVER_MAX_CONNECTIONS"]);

    assert_eq!(
        cap_from_env(Some("abc")),
        Some(DEFAULT_MAX_CONNECTIONS_ON_MISCONFIGURATION)
    );
    assert_eq!(
        cap_from_env(Some("-5")),
        Some(DEFAULT_MAX_CONNECTIONS_ON_MISCONFIGURATION)
    );
}

/// An explicit positive value is the cap, below or above the default.
#[tokio::test]
async fn hardening_an_explicit_connection_cap_is_used() {
    let _env = crate::env_lock::lock_env_async().await;
    let _snapshot = crate::env_snapshot::EnvSnapshot::capture(&["SERVER_MAX_CONNECTIONS"]);

    assert_eq!(cap_from_env(Some("250")), Some(250));
    assert_eq!(cap_from_env(Some(" 250 ")), Some(250));
    assert_eq!(cap_from_env(Some("50000")), Some(50_000));
}

/// `Config::init` builds the server configuration through `try_from_env`,
/// so the boot path gets the same finite default.
#[tokio::test]
async fn hardening_the_boot_path_caps_an_unset_connection_count() {
    let _env = crate::env_lock::lock_env_async().await;
    let _snapshot = crate::env_snapshot::EnvSnapshot::capture(&["SERVER_MAX_CONNECTIONS"]);
    crate::env_snapshot::set_env("SERVER_MAX_CONNECTIONS", None);

    let config = ServerConfig::try_from_env()
        .expect("the server configuration reads from a clean test environment");
    assert_eq!(config.max_connections, Some(DEFAULT_MAX_CONNECTIONS));
}

/// The builder starts from the environment, so with nothing set its cap is
/// the finite default; `max_connections` replaces it.
#[tokio::test]
async fn hardening_the_builder_default_connection_cap_is_finite() {
    let _env = crate::env_lock::lock_env_async().await;
    let _snapshot = crate::env_snapshot::EnvSnapshot::capture(&["SERVER_MAX_CONNECTIONS"]);
    crate::env_snapshot::set_env("SERVER_MAX_CONNECTIONS", None);

    assert_eq!(
        ServerConfig::builder().build().max_connections,
        Some(DEFAULT_MAX_CONNECTIONS)
    );
    assert_eq!(
        ServerConfig::builder()
            .max_connections(250)
            .build()
            .max_connections,
        Some(250)
    );
}

/// No cap is reachable only by asking for it by name, and the request wins
/// over a cap the environment sets. A later `max_connections` call replaces
/// the opt-out, as the last call to any builder method does.
#[tokio::test]
async fn hardening_the_builder_opt_out_removes_the_connection_cap() {
    let _env = crate::env_lock::lock_env_async().await;
    let _snapshot = crate::env_snapshot::EnvSnapshot::capture(&["SERVER_MAX_CONNECTIONS"]);

    crate::env_snapshot::set_env("SERVER_MAX_CONNECTIONS", None);
    assert_eq!(
        ServerConfig::builder()
            .unbounded_connections()
            .build()
            .max_connections,
        None
    );

    crate::env_snapshot::set_env("SERVER_MAX_CONNECTIONS", Some("250"));
    assert_eq!(
        ServerConfig::builder()
            .unbounded_connections()
            .build()
            .max_connections,
        None
    );

    assert_eq!(
        ServerConfig::builder()
            .unbounded_connections()
            .max_connections(64)
            .build()
            .max_connections,
        Some(64)
    );
}

/// A builder cap of zero means no cap, as on the server itself; it never
/// builds a server that accepts nothing.
#[tokio::test]
async fn hardening_a_zero_builder_cap_means_no_cap() {
    let _env = crate::env_lock::lock_env_async().await;
    let _snapshot = crate::env_snapshot::EnvSnapshot::capture(&["SERVER_MAX_CONNECTIONS"]);
    crate::env_snapshot::set_env("SERVER_MAX_CONNECTIONS", None);

    let config = ServerConfig::builder().max_connections(0).build();
    assert_eq!(config.max_connections, None);
}
