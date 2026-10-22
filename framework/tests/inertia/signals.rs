use http_body_util::BodyExt;
use serde_json::Value;
use suprnova::live::testing::prepare_live_router_for_test;
use suprnova::live::{LiveRegistry, LiveTopics};
use suprnova::testing::TestContainer;
use suprnova::{FrameworkError, HttpResponse, InertiaProps, InertiaResponse, Request, Router};

#[derive(InertiaProps)]
#[inertia(live(topic = "signal.render", only = ["posts"]))]
#[inertia(live(topic = "signal.render", only = ["count"]))]
struct Page {
    posts: Vec<String>,
    count: u64,
}

async fn response(request: &Request) -> Result<HttpResponse, FrameworkError> {
    InertiaResponse::live::<Page>("__tests__/TypedProps")
        .load(|| async {
            Ok(Page {
                posts: vec!["visible".to_owned()],
                count: 1,
            })
        })
        .await?
        .resolve(request)
        .await
}
async fn json(response: HttpResponse) -> Value {
    serde_json::from_slice(
        &response
            .into_hyper()
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes(),
    )
    .unwrap()
}

#[tokio::test]
#[serial_test::serial]
async fn signal_002_page_object_keeps_each_topics_cursor_on_full_and_partial_responses() {
    let _guard = TestContainer::fake();
    suprnova::Crypt::init(suprnova::EncryptionKey::generate());
    TestContainer::singleton(
        LiveRegistry::builder()
            .public_topic("signal.render")
            .unwrap()
            .build(),
    );
    let runtime = prepare_live_router_for_test(&Router::new()).unwrap();
    let topics = LiveTopics::from_runtime(&runtime);
    let full = Request::for_test_with_headers("GET", "/signal", [("X-Inertia", "true")]);
    let page = json(response(&full).await.unwrap()).await;
    assert_eq!(page["liveBindings"]["signal.render"]["cursor"], 0);
    assert_eq!(
        page["liveBindings"]["signal.render"]["only"],
        serde_json::json!(["posts", "count"])
    );
    assert_eq!(page["liveBindings"]["signal.render"]["stream"], "changes");
    assert!(page["liveBindings"]["signal.render"]["island"]["component"].is_string());
    topics.changed("signal.render", &["posts"]).await.unwrap();
    let partial = Request::for_test_with_headers(
        "GET",
        "/signal",
        [
            ("X-Inertia", "true"),
            ("X-Inertia-Partial-Component", "__tests__/TypedProps"),
            ("X-Inertia-Partial-Data", "posts"),
        ],
    );
    let page = json(response(&partial).await.unwrap()).await;
    assert_eq!(page["liveBindings"]["signal.render"]["cursor"], 1);
    assert_eq!(page["props"]["posts"], serde_json::json!(["visible"]));
    assert!(page["props"].get("count").is_none());
}

#[tokio::test]
#[serial_test::serial]
async fn signal_002_render_baseline_precedes_prop_reads_that_race_a_write() {
    let _guard = TestContainer::fake();
    suprnova::Crypt::init(suprnova::EncryptionKey::generate());
    TestContainer::singleton(
        LiveRegistry::builder()
            .public_topic("signal.render")
            .unwrap()
            .build(),
    );
    let runtime = prepare_live_router_for_test(&Router::new()).unwrap();
    let topics = LiveTopics::from_runtime(&runtime);
    let change = topics.clone();
    let request = Request::for_test_with_headers("GET", "/signal", [("X-Inertia", "true")]);
    let page = InertiaResponse::live::<Page>("__tests__/TypedProps")
        .load_with(move |response| async move {
            Ok(response.lazy("posts", move || {
                let change = change.clone();
                async move {
                    change.changed("signal.render", &["posts"]).await.unwrap();
                    Ok::<_, FrameworkError>(vec!["value"])
                }
            }))
        })
        .await
        .unwrap()
        .resolve(&request)
        .await
        .unwrap();
    assert_eq!(
        json(page).await["liveBindings"]["signal.render"]["cursor"],
        0
    );
    assert_eq!(topics.cursor("signal.render").await.unwrap(), 1);
}

#[test]
fn signal_002_binding_to_a_missing_prop_fails_to_build() {
    trybuild::TestCases::new()
        .compile_fail("tests/inertia/compile_fail/signal_002_missing_prop.rs");
}

#[test]
#[ignore = "requires Node 24.22.1 or later"]
fn signal_004_shared_client_drops_stale_cursors_and_coalesces_bounded_reloads() {
    let version = std::process::Command::new("node")
        .arg("--version")
        .output()
        .expect("requires Node 24.22.1 or later");
    let version_text = String::from_utf8_lossy(&version.stdout);
    let version_parts = version_text
        .trim()
        .trim_start_matches('v')
        .split('.')
        .map(str::parse::<u64>)
        .collect::<Result<Vec<_>, _>>();
    assert!(
        version.status.success()
            && version_parts
                .is_ok_and(|parts| parts.len() == 3 && parts.as_slice() >= [24, 22, 1].as_slice()),
        "requires Node 24.22.1 or later; found {}",
        version_text.trim()
    );
    let suite = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../suprnova-cli/tests/frontend-live-core.test.ts");
    let directory = tempfile::tempdir().expect("client page fixture directory");
    let fixture = directory.path().join("baseline-page.json");
    let page = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("loader fixture runtime")
        .block_on(loader_race_page());
    std::fs::write(&fixture, serde_json::to_vec(&page).expect("page JSON"))
        .expect("write actual baseline page");
    let output = std::process::Command::new("node")
        .arg("--test")
        .arg(suite)
        .env("SIGNAL_LIVE_BASELINE_PAGE", fixture)
        .output()
        .expect("Node 24.22.1 or later runs the kit client regression suite");
    print!("{}", String::from_utf8_lossy(&output.stdout));
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[cfg(unix)]
fn signal_fix_007_older_node_is_refused_before_the_client_suite() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("node");
    let marker = directory.path().join("suite-ran");
    std::fs::write(
        &executable,
        "#!/bin/sh\nif [ \"$1\" = --version ]; then echo v24.22.0; else : > \"$SIGNAL_NODE_MARKER\"; fi\n",
    )
    .unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "signals::signal_004_shared_client_drops_stale_cursors_and_coalesces_bounded_reloads",
            "--nocapture",
        ])
        .env("PATH", directory.path())
        .env("SIGNAL_NODE_MARKER", &marker)
        .output()
        .unwrap();
    assert!(!output.status.success(), "an older Node must be refused");
    assert!(String::from_utf8_lossy(&output.stderr).contains("requires Node 24.22.1 or later"));
    assert!(
        !marker.exists(),
        "the unsupported runtime must not run the suite"
    );
}

#[derive(Clone)]
struct SignalPrincipal(&'static str);
impl suprnova::auth::Authenticatable for SignalPrincipal {
    fn get_auth_identifier(&self) -> String {
        self.0.to_owned()
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn into_arc_any(self: std::sync::Arc<Self>) -> std::sync::Arc<dyn std::any::Any + Send + Sync> {
        self
    }
}

#[derive(InertiaProps)]
#[inertia(live(topic = "signal.auth-private", only = ["posts"]))]
#[inertia(live(topic = "signal.auth-public", only = ["posts"]))]
struct AuthorizedPage {
    posts: Vec<String>,
}

#[derive(Clone)]
struct SignalLog(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
impl std::io::Write for SignalLog {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

async fn authorized_page(principal: Option<&'static str>) -> Value {
    suprnova::auth::request_state::request_state_scope_for_test(async {
        if let Some(principal) = principal {
            suprnova::Auth::set_user(std::sync::Arc::new(SignalPrincipal(principal)));
        }
        let request =
            Request::for_test_with_headers("GET", "/signal-auth", [("X-Inertia", "true")]);
        let response = InertiaResponse::live::<AuthorizedPage>("__tests__/TypedProps")
            .load(|| async {
                Ok(AuthorizedPage {
                    posts: vec!["page still renders".to_owned()],
                })
            })
            .await?
            .resolve(&request)
            .await;
        let response = response?;
        assert_eq!(response.status_code(), 200);
        Ok::<_, FrameworkError>(json(response).await)
    })
    .await
    .unwrap()
}

#[tokio::test]
#[serial_test::serial]
async fn signal_fix_001_private_metadata_is_omitted_and_warned_once() {
    let _container = TestContainer::fake();
    suprnova::Crypt::init(suprnova::EncryptionKey::generate());
    suprnova::Gate::define::<String, String>("live:topic.signal.auth-private", |principal, _| {
        principal == "allowed"
    });
    TestContainer::singleton(
        LiveRegistry::builder()
            .topic("signal.auth-private")
            .unwrap()
            .public_topic("signal.auth-public")
            .unwrap()
            .build(),
    );
    let runtime = prepare_live_router_for_test(&Router::new()).unwrap();
    LiveTopics::from_runtime(&runtime)
        .changed("signal.auth-private", &["posts"])
        .await
        .unwrap();
    let log = SignalLog(std::sync::Arc::new(std::sync::Mutex::new(Vec::new())));
    let writer = log.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(move || writer.clone())
        .finish();
    let _subscriber = tracing::subscriber::set_default(subscriber);
    for principal in [None, Some("denied"), None] {
        let page = authorized_page(principal).await;
        assert!(
            page["liveBindings"].get("signal.auth-private").is_none(),
            "unauthorized cursor leaked: {page}"
        );
        assert_eq!(page["liveBindings"]["signal.auth-public"]["cursor"], 0);
        assert_eq!(page["props"]["posts"][0], "page still renders");
    }
    assert_eq!(
        authorized_page(Some("allowed")).await["liveBindings"]["signal.auth-private"]["cursor"],
        1
    );
    suprnova::Gate::define::<String, String>("live:topic.signal.auth-private", |_, _| false);
    assert!(
        authorized_page(Some("allowed")).await["liveBindings"]
            .get("signal.auth-private")
            .is_none()
    );
    let output = String::from_utf8(log.0.lock().unwrap().clone()).unwrap();
    let lines: Vec<_> = output
        .lines()
        .filter(|line| {
            line.contains("signal.auth-private") && line.contains("__tests__/TypedProps")
        })
        .collect();
    assert_eq!(
        lines.iter().filter(|line| line.contains("WARN")).count(),
        1,
        "{output}"
    );
    assert!(
        lines.iter().filter(|line| line.contains("DEBUG")).count() >= 3,
        "{output}"
    );
}

async fn loader_race_page() -> Value {
    let _guard = TestContainer::fake();
    suprnova::Crypt::init(suprnova::EncryptionKey::generate());
    let db = suprnova::testing::TestDatabase::sqlite_memory()
        .await
        .unwrap();
    db.execute_unprepared("CREATE TABLE signal_baseline_posts (title TEXT NOT NULL)")
        .await
        .unwrap();
    db.execute_unprepared("INSERT INTO signal_baseline_posts (title) VALUES ('old query value')")
        .await
        .unwrap();
    let hub = std::sync::Arc::new(BaselineHub::default());
    suprnova::App::bind::<dyn suprnova::broadcasting::BroadcastHub>(hub.clone());
    TestContainer::singleton(
        LiveRegistry::builder()
            .public_topic("signal.render")
            .unwrap()
            .build(),
    );
    let runtime = prepare_live_router_for_test(&Router::new()).unwrap();
    let topics = LiveTopics::from_runtime(&runtime);
    let request = Request::for_test_with_headers("GET", "/signal", [("X-Inertia", "true")]);
    let response = InertiaResponse::live::<Page>("__tests__/TypedProps")
        .load(|| async {
            let row = db
                .fetch_one("SELECT title FROM signal_baseline_posts", vec![])
                .await?;
            let title = row.try_get::<String>("", "title")?;
            let change = topics.clone();
            suprnova::DB::transaction(move |_| {
                Box::pin(async move {
                    suprnova::DB::statement(
                        "UPDATE signal_baseline_posts SET title = 'committed new value'",
                        vec![],
                    )
                    .await?;
                    change.changed("signal.render", &["posts"]).await?;
                    Ok::<(), FrameworkError>(())
                })
            })
            .await?;
            Ok(Page {
                posts: vec![title],
                count: 0,
            })
        })
        .await
        .unwrap()
        .resolve(&request)
        .await
        .unwrap();
    let page = json(response).await;
    assert_eq!(page["props"]["posts"][0], "old query value");
    assert_eq!(page["liveBindings"]["signal.render"]["cursor"], 0);
    assert_eq!(topics.cursor("signal.render").await.unwrap(), 1);
    assert_eq!(hub.reads.load(std::sync::atomic::Ordering::SeqCst), 1);
    page
}

#[tokio::test]
#[serial_test::serial]
async fn signal_fix_004_a_write_after_an_eager_query_keeps_the_old_baseline() {
    loader_race_page().await;
}

#[derive(Default)]
struct BaselineHub {
    inner: suprnova::broadcasting::InMemoryBroadcastHub,
    reads: std::sync::atomic::AtomicUsize,
}
#[suprnova::async_trait]
impl suprnova::broadcasting::BroadcastHub for BaselineHub {
    fn subscribe(
        &self,
        channel: &str,
    ) -> tokio::sync::broadcast::Receiver<suprnova::broadcasting::BroadcastEnvelope> {
        self.inner.subscribe(channel)
    }
    async fn publish(
        &self,
        envelope: suprnova::broadcasting::BroadcastEnvelope,
    ) -> Result<(), FrameworkError> {
        self.inner.publish(envelope).await
    }
    fn subscribe_live_topic(
        &self,
        topic: &str,
    ) -> Result<
        tokio::sync::broadcast::Receiver<suprnova::broadcasting::BroadcastEnvelope>,
        FrameworkError,
    > {
        self.inner.subscribe_live_topic(topic)
    }
    async fn live_topic_cursor(&self, topic: &str) -> Result<u64, FrameworkError> {
        self.inner.live_topic_cursor(topic).await
    }
    async fn live_topic_cursors(&self, topics: &[String]) -> Result<Vec<u64>, FrameworkError> {
        self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.inner.live_topic_cursors(topics).await
    }
    async fn publish_live_topic(
        &self,
        topic: &str,
        props: &[String],
        except: Option<String>,
        interval: std::time::Duration,
    ) -> Result<(), FrameworkError> {
        self.inner
            .publish_live_topic(topic, props, except, interval)
            .await
    }
}

#[derive(InertiaProps)]
#[inertia(live(topic = "signal.scoped", only = ["posts"]))]
#[inertia(live(topic = "signal.public-scoped", only = ["posts"]))]
struct ScopedPage {
    posts: Vec<String>,
}

#[tokio::test]
#[serial_test::serial]
async fn signal_fix_004_family_scope_is_required_and_authorized_before_the_loader() {
    let _guard = TestContainer::fake();
    suprnova::Crypt::init(suprnova::EncryptionKey::generate());
    suprnova::Gate::define::<String, String>("live:topic.signal.scoped", |principal, instance| {
        instance == &format!("signal.scoped.{principal}")
    });
    TestContainer::singleton(
        LiveRegistry::builder()
            .topic_family("signal.scoped")
            .unwrap()
            .public_topic_family("signal.public-scoped")
            .unwrap()
            .build(),
    );
    prepare_live_router_for_test(&Router::new()).unwrap();
    let result = InertiaResponse::live::<ScopedPage>("__tests__/TypedProps")
        .load(|| async { panic!("missing scope must fail before loading") })
        .await;
    let error = result.err().unwrap().to_string();
    assert!(
        error.contains("__tests__/TypedProps") && error.contains("signal.scoped"),
        "{error}"
    );
    for principal in [None, Some("other"), Some("owner")] {
        suprnova::auth::request_state::request_state_scope_for_test(async {
            if let Some(principal) = principal {
                suprnova::Auth::set_user(std::sync::Arc::new(SignalPrincipal(principal)));
            }
            let request = Request::for_test_with_headers("GET", "/scope", [("X-Inertia", "true")]);
            let response = InertiaResponse::live::<ScopedPage>("__tests__/TypedProps")
                .scope("signal.scoped", "owner")
                .scope("signal.public-scoped", "guest")
                .load(|| async { Ok(ScopedPage { posts: vec![] }) })
                .await
                .unwrap()
                .resolve(&request)
                .await
                .unwrap();
            let page = json(response).await;
            assert_eq!(
                page["liveBindings"].get("signal.scoped.owner").is_some(),
                principal == Some("owner")
            );
            assert_eq!(
                page["liveBindings"]["signal.public-scoped.guest"]["cursor"],
                0
            );
        })
        .await;
    }
}

#[tokio::test]
#[serial_test::serial]
async fn signal_fix_004_prebuilt_bound_props_require_a_loader() {
    async fn prebuilt(request: &Request) -> Result<HttpResponse, FrameworkError> {
        suprnova::inertia_response!(
            request,
            "__tests__/TypedProps",
            Page {
                posts: vec![],
                count: 0
            }
        )
    }
    let _container = TestContainer::fake();
    let request = Request::for_test_with_headers("GET", "/signal", [("X-Inertia", "true")]);
    assert!(
        prebuilt(&request)
            .await
            .err()
            .unwrap()
            .to_string()
            .contains("loader")
    );
}
