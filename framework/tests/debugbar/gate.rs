use super::env_snapshot::set_env;
use super::support::{Harness, debug_mode, page};
use suprnova::{DebugBar, HttpResponse, Request, Router};

#[tokio::test]
async fn dbg_003_debug_off_passes_through_and_exposes_no_route_or_record() {
    let _mode = debug_mode(false, &[]).await;
    let app = Harness::new(
        Router::new()
            .get("/", |_: Request| async {
                Ok(HttpResponse::html("<body>unchanged</body>")
                    .header("x-app", "kept")
                    .header("cache-control", "public"))
            })
            .into(),
    );
    let reply = app.get("/", &[("accept", "text/html")]).await;
    assert_eq!(reply.body, "<body>unchanged</body>");
    assert_eq!(reply.header("x-app"), Some("kept"));
    assert_eq!(reply.header("cache-control"), Some("public"));
    assert_eq!(reply.header("x-debugbar-id"), None);
    let lookup = app.get("/_debugbar/absent", &[]).await;
    assert_eq!(lookup.status, 404);
    assert_eq!(lookup.header("x-debugbar-id"), None);
}

#[tokio::test]
async fn dbg_003_debugbar_false_disables_collection_even_with_debug_on() {
    let _mode = debug_mode(true, &[("DEBUGBAR", "false")]).await;
    let app = Harness::new(Router::new().get("/", page).into());
    let reply = app.get("/", &[("accept", "text/html")]).await;
    assert_eq!(reply.header("x-debugbar-id"), None);
    assert!(!reply.body.contains("suprnova-debugbar"));
    assert_eq!(app.get("/_debugbar/absent", &[]).await.status, 404);
}

#[tokio::test]
async fn dbg_003_closing_the_gate_erases_previous_records_and_cannot_be_overridden() {
    let _mode = debug_mode(true, &[]).await;
    let app = Harness::new(Router::new().get("/", page).into());
    let initial = app.get("/", &[]).await;
    set_env("APP_DEBUG", Some("false"));
    set_env("DEBUGBAR", Some("true"));
    let off = app.get("/", &[("accept", "text/html")]).await;
    assert_eq!(off.header("x-debugbar-id"), None);
    set_env("APP_DEBUG", Some("true"));
    assert_eq!(
        app.get(&format!("/_debugbar/{}", initial.id()), &[])
            .await
            .status,
        404
    );
}

#[tokio::test]
async fn dbg_003_invalid_retention_fails_configuration_and_default_gate_is_on() {
    let _mode = debug_mode(true, &[]).await;
    for invalid in ["0", "-1", "abc", ""] {
        set_env("DEBUGBAR_KEEP", Some(invalid));
        assert!(DebugBar::middleware().is_err());
    }
    set_env("DEBUGBAR_KEEP", None);
    let app = Harness::new(Router::new().get("/", page).into());
    assert!(
        app.get("/", &[("accept", "text/html")])
            .await
            .body
            .contains("suprnova-debugbar")
    );
}

#[tokio::test]
async fn dbg_003_closed_gate_starts_no_collector_and_keeps_no_record() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static FORMATTED: AtomicUsize = AtomicUsize::new(0);
    #[derive(Clone)]
    struct Probe;
    impl std::fmt::Debug for Probe {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            FORMATTED.fetch_add(1, Ordering::SeqCst);
            formatter.write_str("Probe")
        }
    }
    impl suprnova::Event for Probe {
        fn event_name() -> &'static str {
            "debugbar.Probe"
        }
    }
    let _mode = debug_mode(false, &[]).await;
    let middleware = DebugBar::middleware().expect("debug bar");
    let app = Harness {
        router: Arc::new(
            Router::new()
                .get("/", |request: Request| async {
                    suprnova::EventFacade::dispatch(Probe).await?;
                    page(request).await
                })
                .into(),
        ),
        registry: Arc::new(suprnova::MiddlewareRegistry::new().append(middleware.clone())),
    };
    app.get("/", &[]).await;
    assert_eq!(FORMATTED.load(Ordering::SeqCst), 0);
    assert_eq!(middleware.record_count_for_test().expect("record count"), 0);
    set_env("APP_DEBUG", Some("true"));
    app.get("/", &[]).await;
    assert!(FORMATTED.load(Ordering::SeqCst) > 0);
    assert_eq!(middleware.record_count_for_test().expect("record count"), 1);
    set_env("DEBUGBAR", Some("false"));
    app.get("/", &[]).await;
    assert_eq!(middleware.record_count_for_test().expect("record count"), 0);
}
