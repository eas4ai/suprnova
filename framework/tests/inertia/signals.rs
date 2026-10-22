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
#[ignore = "requires Bun"]
fn signal_004_shared_client_drops_stale_cursors_and_coalesces_bounded_reloads() {
    let available = std::process::Command::new("bun")
        .arg("--version")
        .output()
        .expect("requires Bun on PATH to run the client core regression suite");
    assert!(
        available.status.success(),
        "requires Bun: bun --version failed"
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
    let retirement = directory.path().join("retired-instance.json");
    let retired = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .start_paused(true)
        .build()
        .expect("retirement fixture runtime")
        .block_on(retired_instance_fixture());
    std::fs::write(&retirement, serde_json::to_vec(&retired).unwrap()).unwrap();
    let output = std::process::Command::new("bun")
        .arg("test")
        .arg(suite)
        .env("SIGNAL_LIVE_BASELINE_PAGE", fixture)
        .env("SIGNAL_LIVE_RETIRED_PAGE", retirement)
        .output()
        .expect("requires Bun on PATH to run the client core regression suite");
    print!("{}", String::from_utf8_lossy(&output.stdout));
    eprint!("{}", String::from_utf8_lossy(&output.stderr));
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn signal_fix_007_missing_bun_is_refused_before_the_client_suite() {
    let directory = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "signals::signal_004_shared_client_drops_stale_cursors_and_coalesces_bounded_reloads",
            "--nocapture",
        ])
        .env("PATH", directory.path())
        .output()
        .unwrap();
    assert!(!output.status.success(), "missing Bun must be refused");
    assert!(String::from_utf8_lossy(&output.stderr).contains("requires Bun on PATH"));
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
    async fn live_topic_positions(
        &self,
        topics: &[String],
    ) -> Result<Vec<suprnova::LiveTopicPosition>, FrameworkError> {
        self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.inner.live_topic_positions(topics).await
    }
    async fn publish_live_topic(
        &self,
        topic: &str,
        props: &[String],
        except: Option<suprnova::broadcasting::LiveTopicExclusion>,
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

#[test]
fn signal_fix_008_live_binding_public_types_are_exported_from_the_root() {
    fn bindings<P: suprnova::InertiaProps>() -> &'static [suprnova::LiveBinding] {
        P::live_bindings()
    }
    let _: Option<suprnova::InertiaLiveResponse<Page>> = None;
    let _: Option<suprnova::LiveTopicExclusion> = None;
    let _: Option<suprnova::LiveTopicSender> = None;
    let _: Option<suprnova::LiveTopicPosition> = None;
    let _: Option<suprnova::LiveTopics> = None;
    let _: Option<suprnova::LiveTopicSignal> = None;
    assert_eq!(bindings::<Page>()[0].topic, "signal.render");
}

#[tokio::test]
#[serial_test::serial]
async fn signal_fix2_002_denied_family_instances_share_one_warning() {
    let _guard = TestContainer::fake();
    suprnova::Crypt::init(suprnova::EncryptionKey::generate());
    suprnova::Gate::define::<String, String>("live:topic.signal.scoped", |_, _| false);
    TestContainer::singleton(
        LiveRegistry::builder()
            .topic_family("signal.scoped")
            .unwrap()
            .public_topic_family("signal.public-scoped")
            .unwrap()
            .build(),
    );
    prepare_live_router_for_test(&Router::new()).unwrap();
    let log = SignalLog(std::sync::Arc::new(std::sync::Mutex::new(Vec::new())));
    let writer = log.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(move || writer.clone())
        .finish();
    let _subscriber = tracing::subscriber::set_default(subscriber);
    for key in ["first", "second"] {
        InertiaResponse::live::<ScopedPage>("DeniedInstances")
            .scope("signal.scoped", key)
            .scope("signal.public-scoped", key)
            .load(|| async { Ok(ScopedPage { posts: vec![] }) })
            .await
            .unwrap();
    }
    assert_eq!(
        suprnova::live::testing::live_binding_warning_entries("DeniedInstances"),
        1
    );
    let output = String::from_utf8(log.0.lock().unwrap().clone()).unwrap();
    assert_eq!(
        output
            .lines()
            .filter(|line| line.contains("WARN") && line.contains("DeniedInstances"))
            .count(),
        1,
        "{output}"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn signal_fix2_003_static_unknown_and_duplicate_scopes_fail_before_loading() {
    let _guard = TestContainer::fake();
    suprnova::Crypt::init(suprnova::EncryptionKey::generate());
    TestContainer::singleton(
        LiveRegistry::builder()
            .public_topic("signal.render")
            .unwrap()
            .build(),
    );
    prepare_live_router_for_test(&Router::new()).unwrap();
    for scopes in [
        vec!["signal.render"],
        vec!["unknown"],
        vec!["signal.render", "signal.render"],
    ] {
        let mut builder = InertiaResponse::live::<Page>("ScopeValidation");
        for scope in &scopes {
            builder = builder.scope(*scope, "key");
        }
        let result = builder
            .load(|| async { panic!("invalid scope ran the loader") })
            .await;
        let error = result.err().expect("invalid scope accepted").to_string();
        assert!(
            error.contains("ScopeValidation") && error.contains(scopes[0]),
            "{error}"
        );
    }
}

#[test]
fn signal_fix2_005_manual_controller_imports_the_macro_it_calls() {
    let manual = include_str!("../../../manual/frontend-pages.md");
    let opening = manual
        .split("```rust\n")
        .nth(1)
        .unwrap()
        .split("```")
        .next()
        .unwrap();
    compile_manual_example(opening);
    // Application queries and models supply the context these chapter excerpts name.
    let context = r#"
use suprnova::*;
use suprnova::live::{LiveRegistry, LiveTopics};
#[derive(InertiaProps)] pub struct PostSummary { title: String }
#[derive(InertiaProps)] pub struct NoteSummary { title: String }
type Stats = u64;
struct Note { user_id: i64 }
struct Post;
#[derive(InertiaProps)]
#[inertia(live(topic = "notes", only = ["stats"]))]
struct NotesDashboardProps { stats: Stats }
async fn recent_posts() -> Result<Vec<PostSummary>, FrameworkError> { Ok(vec![]) }
async fn count_posts() -> Result<usize, FrameworkError> { Ok(0) }
async fn note_stats(_: i64) -> Result<Stats, FrameworkError> { Ok(0) }
async fn recent_notes(_: i64) -> Result<Vec<NoteSummary>, FrameworkError> { Ok(vec![]) }
async fn import_posts() -> Result<(), FrameworkError> { Ok(()) }
async fn rebuild_post_count() -> Result<(), FrameworkError> { Ok(()) }
async fn restore_posts_from_snapshot() -> Result<(), FrameworkError> { Ok(()) }
"#;
    for section in [
        manual
            .split("## Live topic bindings")
            .nth(1)
            .unwrap()
            .split("## Shared props")
            .next()
            .unwrap(),
        include_str!("../../../manual/live.md")
            .split("## Topic signals")
            .nth(1)
            .unwrap()
            .split("## Assets")
            .next()
            .unwrap(),
        include_str!("../../../manual/starter-kits.md")
            .split("### Live note updates")
            .nth(1)
            .unwrap()
            .split("### Why Suprnova diverges")
            .next()
            .unwrap(),
    ] {
        for block in section.split("```rust\n").skip(1) {
            let example = block.split("```").next().unwrap();
            compile_manual_example(&format!(
                "{context}\nasync fn example(req: Request, user_id: i64) -> Result<(), Box<dyn std::error::Error>> {{\n{example};\nOk(())\n}}"
            ));
        }
    }
}

fn compile_manual_example(example: &str) {
    let directory = tempfile::tempdir().unwrap();
    let pages = directory.path().join("frontend/src/pages");
    std::fs::create_dir_all(&pages).unwrap();
    std::fs::write(pages.join("Home.svelte"), "<h1>Home</h1>").unwrap();
    let source = directory.path().join("controller.rs");
    std::fs::write(&source, example).unwrap();
    let deps = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .to_owned();
    let mut command = std::process::Command::new("rustc");
    command
        .args(["--edition=2024", "--crate-type=lib", "--emit=metadata"])
        .arg(&source)
        .arg("-o")
        .arg(directory.path().join("controller.rmeta"))
        .arg("-L")
        .arg(format!("dependency={}", deps.display()))
        .env("CARGO_MANIFEST_DIR", directory.path());
    for name in ["suprnova"] {
        let library = std::fs::read_dir(&deps)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(&format!("lib{name}-"))
                    && path.extension().is_some_and(|ext| ext == "rlib")
            })
            .max_by_key(|path| path.metadata().unwrap().modified().unwrap())
            .unwrap();
        command
            .arg("--extern")
            .arg(format!("{name}={}", library.display()));
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "manual example does not compile: {}\n{example}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[derive(InertiaProps)]
#[inertia(live(topic = "notes", only = ["stats", "recent_notes"]))]
struct RetiredPage {
    stats: u64,
    recent_notes: Vec<String>,
}

async fn retired_instance_fixture() -> Value {
    use suprnova::broadcasting::BroadcastHub;
    let _guard = TestContainer::fake();
    suprnova::Crypt::init(suprnova::EncryptionKey::generate());
    let hub = std::sync::Arc::new(suprnova::InMemoryBroadcastHub::new());
    suprnova::App::bind::<dyn BroadcastHub>(hub.clone());
    TestContainer::singleton(
        suprnova::live::LiveConfig::builder()
            .topic_instance_retention_secs(2)
            .build()
            .unwrap(),
    );
    TestContainer::singleton(
        LiveRegistry::builder()
            .public_topic_family("notes")
            .unwrap()
            .build(),
    );
    let runtime = prepare_live_router_for_test(&Router::new()).unwrap();
    for _ in 0..10 {
        hub.publish_live_topic(
            "notes.42",
            &["stats".into()],
            None,
            std::time::Duration::from_millis(1),
        )
        .await
        .unwrap();
        tokio::time::advance(std::time::Duration::from_millis(2)).await;
    }
    async fn page() -> Value {
        let request = Request::for_test_with_headers("GET", "/retired", [("X-Inertia", "true")]);
        json(
            InertiaResponse::live::<RetiredPage>("__tests__/TypedProps")
                .scope("notes", "42")
                .load(|| async {
                    Ok(RetiredPage {
                        stats: 0,
                        recent_notes: vec![],
                    })
                })
                .await
                .unwrap()
                .resolve(&request)
                .await
                .unwrap(),
        )
        .await
    }
    let old = page().await;
    assert_eq!(old["liveBindings"]["notes.42"]["cursor"], 10);
    tokio::time::advance(std::time::Duration::from_secs(3)).await;
    assert_eq!(hub.prune_live_topics().unwrap(), 1);
    let reset = page().await;
    assert_eq!(reset["liveBindings"]["notes.42"]["cursor"], 0);
    assert!(
        reset["liveBindings"]["notes.42"]["generation"]
            .as_u64()
            .unwrap()
            > old["liveBindings"]["notes.42"]["generation"]
                .as_u64()
                .unwrap()
    );
    let mut receiver = hub.subscribe_live_topic("notes.42").unwrap();
    LiveTopics::from_runtime(&runtime)
        .changed("notes.42", &["stats"])
        .await
        .unwrap();
    let signal = receiver.recv().await.unwrap().data;
    assert_eq!(signal["cursor"], 1);
    assert_eq!(
        signal["generation"],
        reset["liveBindings"]["notes.42"]["generation"]
    );
    let current = page().await;
    serde_json::json!({ "oldPage": old, "resetPage": reset, "currentPage": current, "signal": signal })
}

#[tokio::test(start_paused = true)]
#[serial_test::serial]
async fn signal_fix2_001_retired_family_renders_zero_before_its_first_new_signal() {
    retired_instance_fixture().await;
}

#[tokio::test(start_paused = true)]
#[serial_test::serial]
async fn signal_fix3_001_retirement_changes_the_rendered_generation_and_signal() {
    retired_instance_fixture().await;
}
