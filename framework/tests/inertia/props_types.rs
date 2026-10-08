//! PAR-051 and PAR-052: prop providers, `sharedProps`, and the merge,
//! once and scroll prop types, measured against inertia-laravel 3.5.1.
//!
//! Every test drives `InertiaResponse::resolve` through an in-test
//! `InertiaRequestExt` mock, the way the other files in this binary do,
//! and reads the page object the client would receive.

use std::collections::HashMap;

use serde_json::{Value, json};
use suprnova::{InertiaRequestExt, InertiaResponse, Prop};

/// Minimal `InertiaRequestExt` impl, mirroring the other Inertia test files.
struct MockReq {
    path: String,
    headers: HashMap<String, String>,
}

impl MockReq {
    fn new(path: &str) -> Self {
        Self {
            path: path.to_string(),
            headers: HashMap::new(),
        }
    }

    fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.insert(name.to_string(), value.to_string());
        self
    }

    fn inertia(self) -> Self {
        self.header("X-Inertia", "true")
    }
}

impl InertiaRequestExt for MockReq {
    fn path(&self) -> &str {
        &self.path
    }
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(|s| s.as_str())
    }
}

/// Resolve a response and parse the JSON page object out of it.
async fn page_of(response: InertiaResponse, req: &MockReq) -> Value {
    use http_body_util::BodyExt;
    let resp = response.resolve(req).await.expect("the response resolves");
    let bytes = resp
        .into_hyper()
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    serde_json::from_slice(&bytes).expect("an Inertia visit returns a JSON page object")
}

/// The string entries of one page-object list, empty when it is absent.
fn names(page: &Value, field: &str) -> Vec<String> {
    page.get(field)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

// ---- PAR-052: merge at nested paths, `match_on` ----

#[tokio::test]
async fn inp_append_at_two_paths_with_match_on_prefixes_each_path() {
    // Laravel's `append(['a.items', 'b'], 'id')`: both paths merge, and
    // each one gets its own `{path}.{match_on}` dedupe field.
    let response = InertiaResponse::new("Feed").prop(
        "feed",
        Prop::eager(json!({ "a": { "items": [] }, "b": [] })).append_at(["a.items", "b"], "id"),
    );
    let page = page_of(response, &MockReq::new("/").inertia()).await;

    assert_eq!(names(&page, "mergeProps"), ["feed.a.items", "feed.b"]);
    assert_eq!(
        names(&page, "matchPropsOn"),
        ["feed.a.items.id", "feed.b.id"]
    );
    assert!(names(&page, "prependProps").is_empty(), "{page}");
}

#[tokio::test]
async fn inp_append_at_one_path_without_match_on_adds_no_match_field() {
    let response = InertiaResponse::new("Feed").prop(
        "posts",
        Prop::eager(json!({ "data": [] })).append_at("data", None),
    );
    let page = page_of(response, &MockReq::new("/").inertia()).await;

    assert_eq!(names(&page, "mergeProps"), ["posts.data"]);
    assert!(
        !page.as_object().unwrap().contains_key("matchPropsOn"),
        "no match field was named; got {page}"
    );
}

#[tokio::test]
async fn inp_prepend_at_with_match_on_emits_prepend_props() {
    let response = InertiaResponse::new("Chat").prop(
        "thread",
        Prop::eager(json!({ "messages": [] })).prepend_at("messages", "uuid"),
    );
    let page = page_of(response, &MockReq::new("/").inertia()).await;

    assert_eq!(names(&page, "prependProps"), ["thread.messages"]);
    assert_eq!(names(&page, "matchPropsOn"), ["thread.messages.uuid"]);
    assert!(names(&page, "mergeProps").is_empty(), "{page}");
}

#[tokio::test]
async fn inp_append_and_prepend_paths_mix_on_one_prop() {
    // Laravel keeps two path lists, so one prop can append at one path
    // and prepend at another.
    let response = InertiaResponse::new("Dashboard").prop(
        "activity",
        Prop::eager(json!({ "older": [], "newer": [] }))
            .append_at("older", None)
            .prepend_at("newer", "id"),
    );
    let page = page_of(response, &MockReq::new("/").inertia()).await;

    assert_eq!(names(&page, "mergeProps"), ["activity.older"]);
    assert_eq!(names(&page, "prependProps"), ["activity.newer"]);
    assert_eq!(names(&page, "matchPropsOn"), ["activity.newer.id"]);
}

#[tokio::test]
async fn inp_match_on_replaces_the_list_on_each_call() {
    // Laravel's `matchOn` sets the list (`Arr::wrap`); a second call
    // replaces the first rather than adding to it.
    let response = InertiaResponse::new("Feed").prop(
        "posts",
        Prop::eager(json!([])).merge().match_on("x").match_on("y"),
    );
    let page = page_of(response, &MockReq::new("/").inertia()).await;

    assert_eq!(names(&page, "matchPropsOn"), ["posts.y"]);
}

// ---- PAR-052: once options ----

/// A fixed render moment, so an expiry computed from "now" is exact.
fn render_moment() -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp(1_800_000_000, 250_000_000).expect("a valid timestamp")
}

/// A once prop's resolver that counts its runs.
fn counted_rates(
    calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
) -> impl Fn() -> std::future::Ready<Result<Value, suprnova::FrameworkError>> + Send + Sync + 'static
{
    move || {
        calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        std::future::ready(Ok(json!({ "usd": 1 })))
    }
}

#[tokio::test]
async fn inp_once_until_seconds_emits_now_plus_the_seconds_in_milliseconds() {
    // Laravel's `until(60)`: `expiresAt` is (now + 60 s) * 1000, the
    // seconds counted from the render.
    let _clock = suprnova::testing::TestClock::travel_to(render_moment());
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let response = InertiaResponse::new("Dashboard").once_with(
        "rates",
        suprnova::OnceOptions::new().until(60),
        counted_rates(calls),
    );
    let page = page_of(response, &MockReq::new("/").inertia()).await;

    assert_eq!(
        page["onceProps"]["rates"]["expiresAt"],
        json!((1_800_000_000_i64 + 60) * 1000)
    );
}

#[tokio::test]
async fn inp_once_until_a_date_emits_that_moment_in_milliseconds() {
    let _clock = suprnova::testing::TestClock::travel_to(render_moment());
    let at = chrono::DateTime::from_timestamp(1_800_003_600, 0).expect("a valid timestamp");
    let response =
        InertiaResponse::new("Dashboard").prop("rates", Prop::eager(json!({})).once().until(at));
    let page = page_of(response, &MockReq::new("/").inertia()).await;

    assert_eq!(
        page["onceProps"]["rates"]["expiresAt"],
        json!(1_800_003_600_000_i64)
    );
}

#[tokio::test]
async fn inp_once_until_a_duration_emits_now_plus_the_duration() {
    let _clock = suprnova::testing::TestClock::travel_to(render_moment());
    let response = InertiaResponse::new("Dashboard")
        .prop(
            "rates",
            Prop::eager(json!({}))
                .once()
                .until(std::time::Duration::from_secs(90)),
        )
        .prop(
            "plans",
            Prop::eager(json!({}))
                .once()
                .until(chrono::TimeDelta::minutes(2)),
        );
    let page = page_of(response, &MockReq::new("/").inertia()).await;

    assert_eq!(
        page["onceProps"]["rates"]["expiresAt"],
        json!((1_800_000_000_i64 + 90) * 1000)
    );
    assert_eq!(
        page["onceProps"]["plans"]["expiresAt"],
        json!((1_800_000_000_i64 + 120) * 1000)
    );
}

#[tokio::test]
async fn inp_once_until_a_past_date_refuses_the_client_cache_claim() {
    // The server-side refusal stays: a client claiming a value whose
    // deadline has passed gets a fresh one.
    let _clock = suprnova::testing::TestClock::travel_to(render_moment());
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let past = chrono::DateTime::from_timestamp(1_700_000_000, 0).expect("a valid timestamp");
    let response = InertiaResponse::new("Dashboard").once_with(
        "rates",
        suprnova::OnceOptions::new().until(past),
        counted_rates(calls.clone()),
    );
    let req = MockReq::new("/")
        .inertia()
        .header("X-Inertia-Except-Once-Props", "rates");
    let page = page_of(response, &req).await;

    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(page["props"]["rates"], json!({ "usd": 1 }));
}

#[tokio::test]
async fn inp_once_fresh_takes_a_bool() {
    let claim = MockReq::new("/")
        .inertia()
        .header("X-Inertia-Except-Once-Props", "rates");

    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let response = InertiaResponse::new("Dashboard").once_with(
        "rates",
        suprnova::OnceOptions::new().fresh(true),
        counted_rates(calls.clone()),
    );
    page_of(response, &claim).await;
    assert_eq!(
        calls.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "fresh(true) resolves despite the client's claim"
    );

    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let response = InertiaResponse::new("Dashboard").prop(
        "rates",
        Prop::lazy(|| async { json!({ "usd": 1 }) })
            .once()
            .fresh(true)
            .fresh(false),
    );
    let page = page_of(response, &claim).await;
    assert!(
        !page["props"].as_object().unwrap().contains_key("rates"),
        "fresh(false) honours the client's claim; got {page}"
    );
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[tokio::test]
async fn inp_once_false_turns_the_once_flag_off() {
    // Laravel's `once(false)`: the prop is an ordinary prop again, with
    // no `onceProps` entry and no regard for the client's claim.
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let response = InertiaResponse::new("Dashboard").once_with(
        "rates",
        suprnova::OnceOptions::new().once(false),
        counted_rates(calls.clone()),
    );
    let req = MockReq::new("/")
        .inertia()
        .header("X-Inertia-Except-Once-Props", "rates");
    let page = page_of(response, &req).await;

    assert!(
        !page.as_object().unwrap().contains_key("onceProps"),
        "{page}"
    );
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[tokio::test]
async fn inp_once_with_sets_the_flag_key_and_expiry_in_one_call() {
    // Laravel's `once(true, 'plans', 60)` on any prop.
    let _clock = suprnova::testing::TestClock::travel_to(render_moment());
    let response = InertiaResponse::new("Billing").prop(
        "planCatalog",
        Prop::eager(json!([])).once_with(
            suprnova::OnceOptions::new()
                .once(true)
                .as_key("plans")
                .until(60),
        ),
    );
    let page = page_of(response, &MockReq::new("/").inertia()).await;

    assert_eq!(
        page["onceProps"]["plans"],
        json!({ "prop": "planCatalog", "expiresAt": (1_800_000_000_i64 + 60) * 1000 })
    );
}

/// A cache key named by an enum, Laravel's `as(Plan::Pro)`.
enum PlanKey {
    Pro,
}

impl std::fmt::Display for PlanKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlanKey::Pro => f.write_str("plan-pro"),
        }
    }
}

#[tokio::test]
async fn inp_once_as_key_takes_an_enum() {
    let response = InertiaResponse::new("Billing")
        .prop("plan", Prop::eager(json!({})).once().as_key(PlanKey::Pro))
        .once_with(
            "other",
            suprnova::OnceOptions::new().as_key(PlanKey::Pro),
            || async { Ok::<_, suprnova::FrameworkError>(json!(1)) },
        );
    let page = page_of(response, &MockReq::new("/").inertia()).await;

    assert_eq!(page["onceProps"]["plan-pro"]["prop"], "other");
}
