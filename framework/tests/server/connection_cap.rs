//! HRD-009: the server caps concurrent connections finitely by default.

use suprnova::config::ServerConfig;

/// With `SERVER_MAX_CONNECTIONS` unset the cap is the finite default, not
/// unbounded.
#[tokio::test]
async fn hardening_an_unset_connection_cap_is_finite() {
    let _env = crate::env_lock::lock_env_async().await;
    let _snapshot = crate::env_snapshot::EnvSnapshot::capture(&["SERVER_MAX_CONNECTIONS"]);
    crate::env_snapshot::set_env("SERVER_MAX_CONNECTIONS", None);

    let config = ServerConfig::from_env();
    assert_eq!(
        config.max_connections,
        Some(10_000),
        "an unset SERVER_MAX_CONNECTIONS leaves connections unbounded"
    );
}
