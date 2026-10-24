//! Prove the real dogfood bootstrap installs debug diagnostics on an application page.

#[path = "../../framework/tests/support/env_lock.rs"]
mod env_lock;
#[path = "../../framework/tests/support/env_snapshot.rs"]
mod env_snapshot;
mod live_support;
mod test_environment;

async fn dogfood(debug: bool) {
    let _lock = env_lock::lock_env_async().await;
    let _env = env_snapshot::EnvSnapshot::capture(&[
        "APP_DEBUG",
        "DEBUGBAR",
        "APP_ENV",
        "INERTIA_DEVTOOLS_ENABLED",
    ]);
    env_snapshot::set_env("APP_DEBUG", Some(if debug { "true" } else { "false" }));
    env_snapshot::set_env("APP_ENV", Some("testing"));
    env_snapshot::set_env("DEBUGBAR", None);
    env_snapshot::set_env("INERTIA_DEVTOOLS_ENABLED", Some("false"));
    let app = live_support::setup_app(2).await;
    let request = live_support::request(&app, hyper::Method::GET, "/live/public", None, false)
        .header("accept", "text/html")
        .body(live_support::empty())
        .expect("browser request");
    let reply = live_support::send(app.addr, request).await;
    assert_eq!(reply.status, hyper::StatusCode::OK);
    assert!(reply.text().contains("Public counter"));
    assert_eq!(reply.text().contains("id=\"suprnova-debugbar\""), debug);
    assert_eq!(reply.header("x-debugbar-id").is_some(), debug);
    if debug {
        let id = reply.header("x-debugbar-id").expect("record id");
        let record = live_support::get(&app, &format!("/_debugbar/{id}"), None).await;
        assert_eq!(record.status, hyper::StatusCode::OK);
        let record: serde_json::Value = serde_json::from_str(&record.text()).expect("record JSON");
        assert!(
            !record["panels"]["live"]
                .as_array()
                .expect("Live entries")
                .is_empty()
        );
    }
}

#[tokio::test]
async fn dogfood_debugbar_is_present_with_debug_on() {
    dogfood(true).await;
}

#[tokio::test]
async fn dogfood_debugbar_is_absent_with_debug_off() {
    dogfood(false).await;
}
