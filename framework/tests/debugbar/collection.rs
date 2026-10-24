use super::support::{Harness, debug_mode, page};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use suprnova::{
    App, Cache, DB, Event, EventFacade, FrameworkError, HttpResponse, InMemoryCache, Job, Listener,
    Mail, Middleware, MiddlewareRegistry, Next, Notifiable, Notification, Notify, Queue, Request,
    Response, Router, async_trait,
};

#[derive(Debug, Clone)]
struct Activity {
    answer: u32,
}
impl Event for Activity {
    fn event_name() -> &'static str {
        "debugbar.Activity"
    }
}
struct ObserveActivity;
#[async_trait]
impl Listener<Activity> for ObserveActivity {
    async fn handle(&self, event: &Activity) -> Result<(), FrameworkError> {
        assert_eq!(event.answer, 42);
        Ok(())
    }
}
#[derive(Serialize, Deserialize)]
struct ExampleJob {
    number: u32,
}
#[async_trait]
impl Job for ExampleJob {
    fn job_name() -> &'static str {
        "debugbar.ExampleJob"
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
}
#[derive(Serialize, Deserialize)]
struct ExampleNotification;
impl Notification for ExampleNotification {
    fn notification_name() -> &'static str {
        "debugbar.ExampleNotification"
    }
    fn channels(&self) -> Vec<&'static str> {
        vec!["mail"]
    }
    fn data(&self) -> serde_json::Value {
        json!({"number":42})
    }
}
struct Recipient;
impl Notifiable for Recipient {
    fn route_for(&self, channel: &str) -> Option<String> {
        (channel == "mail").then(|| "reader@example.test".into())
    }
}

async fn activity(request: Request) -> Response {
    DB::select("SELECT ? AS answer", [42_i32.into()]).await?;
    let tx = DB::begin_transaction().await?;
    tx.commit().await?;
    let tx = DB::begin_transaction().await?;
    tx.rollback().await?;
    // Closure finalization runs on an owned task, which must retain the collector.
    DB::transaction(|_tx| Box::pin(async { Ok::<_, FrameworkError>(()) })).await?;
    Cache::put("answer", &42, None).await?;
    let _: Option<u32> = Cache::get("answer").await?;
    let _: Option<u32> = Cache::get("absent").await?;
    EventFacade::dispatch(Activity { answer: 42 }).await?;
    Mail::raw("fixture message", |mail| {
        mail.to("reader@example.test").subject("Debug bar mail")
    })
    .await?;
    Notify::send(&Recipient, &ExampleNotification).await?;
    Queue::push(ExampleJob { number: 42 }).await?;
    page(request).await
}

#[tokio::test]
async fn dbg_001_collects_queries_transactions_cache_events_mail_notifications_and_jobs() {
    let _mode = debug_mode(true, &[]).await;
    let _database = suprnova::testing::TestDatabase::sqlite_memory()
        .await
        .expect("SQLite fixture");
    App::bind::<dyn suprnova::CacheStore>(Arc::new(InMemoryCache::new()));
    let _mail = Mail::fake();
    let _queue = Queue::fake();
    let _notifications = Notify::fake();
    EventFacade::listen::<Activity, _>(Arc::new(ObserveActivity)).await;
    let app = Harness::new(Router::new().get("/activity", activity).into());
    let reply = app
        .get(
            "/activity?visible=yes",
            &[
                ("accept", "text/html"),
                ("x-request-id", "debugbar-request-42"),
            ],
        )
        .await;
    assert_eq!(reply.status, 200);
    let record = app.record(&reply).await;
    let panels = &record["panels"];
    for panel in [
        "queries",
        "transactions",
        "cache",
        "events",
        "mail",
        "notifications",
        "queue",
    ] {
        assert!(
            !panels[panel].as_array().expect("panel entries").is_empty(),
            "missing {panel}"
        );
    }
    let queries = panels["queries"].to_string();
    for field in [
        "SELECT ? AS answer",
        "bindings",
        "42",
        "time",
        "connection_name",
    ] {
        assert!(queries.contains(field), "query lacks {field}");
    }
    let transactions = panels["transactions"].to_string();
    for name in [
        "TransactionBeginning",
        "TransactionCommitted",
        "TransactionRolledBack",
    ] {
        assert!(transactions.contains(name));
    }
    assert!(panels["events"].to_string().contains("ObserveActivity"));
    assert!(panels["events"].to_string().contains("answer: 42"));
    assert!(panels["cache"].to_string().contains("hit"));
    assert!(panels["cache"].to_string().contains("miss"));
    assert!(panels["cache"].to_string().contains("write"));
    assert_eq!(record["request"]["request_id"], "\"debugbar-request-42\"");
    assert_eq!(record["request"]["route_pattern"], "\"/activity\"");
    assert_eq!(record["status"], 200);
    assert!(record["peak_rss_growth_bytes"].is_u64());
    assert!(
        record["timings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|span| span["name"] == "handler")
    );
}

#[tokio::test]
async fn dbg_001_observes_bootstrap_cache_handles_in_each_request() {
    let _mode = debug_mode(true, &[]).await;
    App::bind::<dyn suprnova::CacheStore>(Arc::new(InMemoryCache::new()));
    Cache::register_store("debugbar-bootstrap", Arc::new(InMemoryCache::new())).unwrap();
    let stores = Arc::new([
        Cache::store().expect("default store before the request"),
        Cache::store_named("debugbar-bootstrap").expect("named store before the request"),
    ]);
    for store in stores.iter() {
        store
            .put_raw("bootstrap-only", "outside request", None)
            .await
            .unwrap();
    }
    let app = Harness::new(
        Router::new()
            .get("/{entry}", move |request: Request| {
                let stores = Arc::clone(&stores);
                async move {
                    let key = request.param("entry").expect("cache key");
                    for store in stores.iter() {
                        assert_eq!(store.get_raw(key).await?, None);
                        store.put_raw(key, "42", None).await?;
                        assert_eq!(store.get_raw(key).await?.as_deref(), Some("42"));
                    }
                    page(request).await
                }
            })
            .into(),
    );
    for key in ["first", "second"] {
        let reply = app.get(&format!("/{key}"), &[]).await;
        assert_eq!(reply.status, 200);
        let record = app.record(&reply).await;
        let entries = record["panels"]["cache"].as_array().expect("cache panel");
        assert_eq!(entries.len(), 6);
        for operation in ["miss", "write", "hit"] {
            assert_eq!(
                entries
                    .iter()
                    .filter(|entry| entry["name"] == operation)
                    .count(),
                2
            );
        }
        for entry in entries {
            assert!(entry["value"].as_str().unwrap().contains(key));
        }
    }
}

#[tokio::test]
async fn dbg_001_redacts_credentials_secret_parameters_and_reflected_values() {
    let _mode = debug_mode(true, &[]).await;
    let app = Harness::new(
        Router::new()
            .get("/account/{password}", |_: Request| async {
                Err(HttpResponse::from(FrameworkError::internal(
                    "request failed password=fixture-parameter",
                )))
            })
            .into(),
    );
    let reply = app
        .get(
            "/account/fixture-path?api%5Ftoken=fixture-query&visible=readable",
            &[
                ("accept", "application/json"),
                ("authorization", "Bearer fixture-credential"),
                ("cookie", "sid=fixture-cookie"),
                ("x-api-key", "fixture-key"),
                (
                    "referer",
                    "http://local/?secret=fixture-referrer&ok=readable",
                ),
            ],
        )
        .await;
    assert_eq!(reply.status, 500);
    let record = app.record(&reply).await;
    let shown = record.to_string();
    for secret in [
        "fixture-path",
        "fixture-query",
        "fixture-credential",
        "fixture-cookie",
        "fixture-key",
        "fixture-referrer",
        "fixture-parameter",
    ] {
        assert!(!shown.contains(secret), "leaked a credential fixture");
    }
    assert!(shown.contains("[redacted]"));
    assert!(shown.contains("readable"));
    assert!(record["error"].is_string());
}

struct SlowMiddleware;
#[async_trait]
impl Middleware for SlowMiddleware {
    async fn handle(&self, request: Request, next: Next) -> Response {
        tokio::time::sleep(Duration::from_millis(50)).await;
        next(request).await
    }
}

#[tokio::test]
async fn dbg_001_times_each_middleware_and_handler_separately() {
    let _mode = debug_mode(true, &[]).await;
    let app = Harness::with_middleware(
        Router::new()
            .get("/", |_: Request| async {
                tokio::time::sleep(Duration::from_millis(70)).await;
                Ok(HttpResponse::html("<body>timed</body>"))
            })
            .into(),
        MiddlewareRegistry::new().append(SlowMiddleware),
    );
    let reply = app.get("/", &[]).await;
    let record = app.record(&reply).await;
    let spans = record["timings"].as_array().unwrap();
    let middleware = spans
        .iter()
        .find(|span| span["name"].as_str().unwrap().ends_with("SlowMiddleware"))
        .expect("named middleware");
    assert!(middleware["duration_ms"].as_f64().unwrap() >= 50.0);
    let handler = spans.iter().find(|span| span["name"] == "handler").unwrap();
    assert!(handler["duration_ms"].as_f64().unwrap() >= 70.0);
    assert!(
        middleware["inclusive_ms"].as_f64().unwrap()
            >= middleware["duration_ms"].as_f64().unwrap() + 70.0
    );
}

#[tokio::test]
async fn dbg_001_keeps_panic_error_report_and_unwound_spans() {
    let _mode = debug_mode(true, &[]).await;
    let app = Harness::new(
        Router::new()
            .get("/panic", |_: Request| async {
                panic!("debugbar fixture panic")
            })
            .into(),
    );
    let reply = app.get("/panic", &[("accept", "application/json")]).await;
    assert_eq!(reply.status, 500);
    let record = app.record(&reply).await;
    assert!(
        record["error"]
            .as_str()
            .unwrap()
            .contains("debugbar fixture panic")
    );
    assert!(
        record["timings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|span| span["name"] == "handler")
    );
}

#[tokio::test]
async fn dbg_001_preserves_duplicate_query_parameters_and_header_values() {
    let _mode = debug_mode(true, &[]).await;
    let app = Harness::new(Router::new().get("/", page).into());
    let reply = app
        .get(
            "/?filter=one&filter=two",
            &[("x-note", "one"), ("x-note", "two")],
        )
        .await;
    let record = app.record(&reply).await;
    for field in ["query", "headers"] {
        let shown = record["request"][field].to_string();
        assert!(shown.contains("one"));
        assert!(shown.contains("two"));
    }
}

#[tokio::test]
async fn dbg_001_redacts_secret_named_sql_and_its_positional_bindings_in_both_panels() {
    let _mode = debug_mode(true, &[]).await;
    let _database = suprnova::testing::TestDatabase::sqlite_memory()
        .await
        .expect("SQLite fixture");
    let app = Harness::new(
        Router::new()
            .get("/", |request: Request| async {
                DB::select("SELECT ? AS api_key", ["fixture-query-credential".into()]).await?;
                page(request).await
            })
            .into(),
    );
    let reply = app.get("/", &[]).await;
    let record = app.record(&reply).await;
    for panel in ["queries", "events"] {
        assert!(!record["panels"][panel].as_array().unwrap().is_empty());
        assert!(
            !record["panels"][panel]
                .to_string()
                .contains("fixture-query-credential")
        );
    }
}
