//! PAR-071: recording is gated by `InertiaConfig::devtools`, skips the
//! `except` paths, and never changes the response it records.

use std::collections::HashMap;

use suprnova::http::text;
use suprnova::{
    DevToolsConfig, HttpResponse, Inertia, InertiaResponse, MiddlewareRegistry, Request, Router,
};

use super::{app_env, client, devtools, entry_ids, inertia};
use crate::protocol_harness::{Client, serve};

fn router() -> Router {
    Router::new()
        .get("/page", |req: Request| async move {
            InertiaResponse::new("Home")
                .with("greeting", "hi")
                .resolve(&req)
                .await
                .map_err(HttpResponse::from)
        })
        .get("/_suprnova/health", |_req: Request| async { text("ok") })
        .get("/admin/report", |_req: Request| async { text("report") })
        .into()
}

#[tokio::test]
async fn indt_enabled_unset_records_in_the_local_environment_only() {
    let dir = tempfile::tempdir().unwrap();
    let unset = || DevToolsConfig::new().storage_path(dir.path());

    let local = app_env("local").await;
    assert_eq!(unset().enabled, None, "INERTIA_DEVTOOLS_ENABLED is unset");
    let response = client(router(), unset()).get("/page").send().await;
    assert!(
        response.header("x-inertia-devtools-id").is_some(),
        "local records"
    );
    assert_eq!(entry_ids(dir.path()).len(), 1);
    drop(local);

    let _production = app_env("production").await;
    let response = client(router(), unset()).get("/page").send().await;
    response.assert_ok();
    assert_eq!(
        response.header("x-inertia-devtools-id"),
        None,
        "production does not"
    );
    assert_eq!(entry_ids(dir.path()).len(), 1, "no entry was added");
}

#[tokio::test]
async fn indt_enabled_decides_outright_whatever_the_environment() {
    let dir = tempfile::tempdir().unwrap();

    let local = app_env("local").await;
    let off = DevToolsConfig::new()
        .enabled(false)
        .storage_path(dir.path());
    let response = client(router(), off).get("/page").send().await;
    response.assert_ok();
    assert_eq!(response.header("x-inertia-devtools-id"), None);
    assert_eq!(response.header("x-inertia-devtools-parent-out"), None);
    assert!(
        !response.body_text().contains("data-inertia-devtools-id"),
        "no tag with DevTools off"
    );
    assert!(
        entry_ids(dir.path()).is_empty(),
        "enabled(false) records nothing in local"
    );
    drop(local);

    let _production = app_env("production").await;
    let response = client(router(), devtools(dir.path()))
        .get("/page")
        .send()
        .await;
    assert!(response.header("x-inertia-devtools-id").is_some());
    assert_eq!(
        entry_ids(dir.path()).len(),
        1,
        "enabled(true) records in production"
    );
}

#[tokio::test]
async fn indt_the_devtools_and_framework_paths_are_never_recorded() {
    let dir = tempfile::tempdir().unwrap();
    let client = client(router(), devtools(dir.path()));

    let health = client.get("/_suprnova/health").send().await;
    health.assert_ok().assert_see("ok");
    assert_eq!(health.header("x-inertia-devtools-id"), None);

    let entries = client.get("/_inertia/devtools/entries").send().await;
    assert_eq!(entries.header("x-inertia-devtools-id"), None);
    assert!(
        entry_ids(dir.path()).is_empty(),
        "neither path left an entry"
    );

    client.get("/page").send().await.assert_ok();
    assert_eq!(entry_ids(dir.path()).len(), 1, "a page is recorded");
}

#[tokio::test]
async fn indt_a_path_under_an_except_pattern_of_its_own_is_not_recorded() {
    let dir = tempfile::tempdir().unwrap();
    let client = client(router(), devtools(dir.path()).except(["/admin/*"]));

    let report = client.get("/admin/report").send().await;
    report.assert_ok().assert_see("report");
    assert_eq!(report.header("x-inertia-devtools-id"), None);
    assert!(entry_ids(dir.path()).is_empty());

    client.get("/page").send().await.assert_ok();
    assert_eq!(entry_ids(dir.path()).len(), 1);
}

/// The headers of a reply that say something about the response itself:
/// not the date, not the request id, not DevTools' own, and not the
/// length, which the id tag of a first visit changes and the body
/// comparison covers.
fn comparable(headers: &HashMap<String, Vec<String>>) -> Vec<(String, Vec<String>)> {
    let mut kept: Vec<(String, Vec<String>)> = headers
        .iter()
        .filter(|(name, _)| {
            !matches!(name.as_str(), "date" | "x-request-id" | "content-length")
                && !name.starts_with("x-inertia-devtools-")
        })
        .map(|(name, values)| (name.clone(), values.clone()))
        .collect();
    kept.sort();
    kept
}

/// `body` without the DevTools id tag a first visit's document carries.
fn without_id_tag(body: &str) -> String {
    match body.find("<script data-inertia-devtools-id") {
        Some(start) => {
            let end = body[start..]
                .find("</script>")
                .map_or(body.len(), |end| start + end + 9);
            format!("{}{}", &body[..start], &body[end..])
        }
        None => body.to_string(),
    }
}

#[tokio::test]
async fn indt_a_storage_path_that_cannot_be_written_leaves_the_response_as_it_was() {
    let dir = tempfile::tempdir().unwrap();
    // A path below a file: no directory can be made there.
    let blocker = dir.path().join("not-a-directory");
    std::fs::write(&blocker, "file").unwrap();
    let unwritable = blocker.join("devtools");

    let plain = serve(
        router(),
        MiddlewareRegistry::new().append(Inertia::middleware(&inertia(
            DevToolsConfig::new().enabled(false),
        ))),
    )
    .await;
    let failing = serve(
        router(),
        MiddlewareRegistry::new().append(Inertia::middleware(&inertia(devtools(&unwritable)))),
    )
    .await;

    for headers in [
        vec![("Accept", "application/json")],
        vec![("X-Inertia", "true")],
    ] {
        let expected = Client::new(plain).send("GET", "/page", &headers).await;
        let got = Client::new(failing).send("GET", "/page", &headers).await;
        assert_eq!(got.status, expected.status);
        assert!(
            got.header("x-inertia-devtools-id").is_some(),
            "the headers still go out"
        );
        assert_eq!(comparable(&got.headers), comparable(&expected.headers));
        assert_eq!(
            without_id_tag(&got.body),
            expected.body,
            "the body is untouched but for the id tag of a first visit"
        );
    }
    assert!(!unwritable.exists());
}

#[tokio::test]
#[tracing_test::traced_test]
async fn indt_a_storage_failure_is_logged_once_and_pauses_recording() {
    let dir = tempfile::tempdir().unwrap();
    let blocker = dir.path().join("not-a-directory");
    std::fs::write(&blocker, "file").unwrap();
    let client = client(router(), devtools(&blocker.join("devtools")));

    for _ in 0..3 {
        client.get("/page").send().await.assert_ok();
    }
    logs_assert(|lines: &[&str]| {
        let warnings = lines
            .iter()
            .filter(|line| line.contains("WARN") && line.contains("failed to persist entry"))
            .count();
        if warnings == 1 {
            Ok(())
        } else {
            Err(format!("expected one warning, got {warnings}: {lines:#?}"))
        }
    });
}

#[tokio::test]
async fn indt_the_settings_read_the_inertia_devtools_variables() {
    use crate::env_snapshot::{EnvSnapshot, set_env};
    let _env = app_env("production").await;
    let _variables = EnvSnapshot::capture(&[
        "INERTIA_DEVTOOLS_TTL_HOURS",
        "INERTIA_DEVTOOLS_PRUNE_INTERVAL_SECONDS",
        "INERTIA_DEVTOOLS_LIMIT",
        "INERTIA_DEVTOOLS_GATE",
    ]);
    set_env("INERTIA_DEVTOOLS_ENABLED", Some("true"));
    set_env("INERTIA_DEVTOOLS_TTL_HOURS", Some("12"));
    set_env("INERTIA_DEVTOOLS_PRUNE_INTERVAL_SECONDS", Some("60"));
    set_env("INERTIA_DEVTOOLS_LIMIT", Some("7"));
    set_env("INERTIA_DEVTOOLS_GATE", Some("viewInertiaDevtools"));

    let config = DevToolsConfig::new();
    assert_eq!(config.enabled, Some(true));
    assert!(config.is_enabled(), "true records in production");
    assert_eq!(config.ttl_hours, 12);
    assert_eq!(config.prune_interval_secs, 60);
    assert_eq!(config.limit, 7);
    assert_eq!(config.gate.as_deref(), Some("viewInertiaDevtools"));

    for (raw, expected) in [
        ("false", Some(false)),
        ("0", Some(false)),
        ("1", Some(true)),
        ("", None),
    ] {
        set_env("INERTIA_DEVTOOLS_ENABLED", Some(raw));
        assert_eq!(
            DevToolsConfig::new().enabled,
            expected,
            "INERTIA_DEVTOOLS_ENABLED={raw:?}"
        );
    }
    assert_eq!(
        DevToolsConfig::new().enabled(true).enabled,
        Some(true),
        "the builder wins over the variable"
    );
}
