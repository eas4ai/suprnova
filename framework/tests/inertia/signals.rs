use http_body_util::BodyExt;
use serde_json::Value;
use suprnova::inertia::InertiaProps as BindingMetadata;
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
    let props = Page {
        posts: vec!["visible".to_owned()],
        count: 1,
    };
    suprnova::inertia_response!(request, "__tests__/TypedProps", props)
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
    let page = InertiaResponse::new("__tests__/TypedProps")
        .with_live_bindings(Page::live_bindings())
        .lazy("posts", move || {
            let change = change.clone();
            async move {
                change.changed("signal.render", &["posts"]).await.unwrap();
                Ok::<_, FrameworkError>(vec!["value"])
            }
        })
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
#[ignore = "requires Node 24"]
fn signal_004_shared_client_drops_stale_cursors_and_coalesces_bounded_reloads() {
    let suite = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../suprnova-cli/tests/frontend-live-core.test.ts");
    let output = std::process::Command::new("node")
        .arg("--test")
        .arg(suite)
        .output()
        .expect("Node 24 runs the kit client regression suite");
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
