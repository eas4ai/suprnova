//! Merge-prop refinements on top of T32's `Prop` composition: nested
//! merge paths, multi-field `match_on` in one call, and a resolver-backed
//! merge value.
//!
//! `framework/tests/inertia.rs` covers the root-level merge / prepend /
//! deep-merge / single-field `match_on` cases through the
//! `InertiaResponse` shortcuts. `framework/tests/inertia_prop_composition.rs`
//! covers flag composition in general. This file covers what neither
//! exercises: `Prop::merge_with_path`, `Prop::match_on` taking several
//! fields at once, and `InertiaResponse::merge_lazy` / `merge_lazy_with`.
//!
//! Like those files, these drive `InertiaResponse::resolve` through an
//! in-test `InertiaRequestExt` mock — `hyper::body::Incoming` cannot be
//! constructed outside hyper's connection machinery.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::json;
use suprnova::{FrameworkError, InertiaRequestExt, InertiaResponse, MergeStrategy, Prop};

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
async fn page_of(resp: suprnova::HttpResponse) -> serde_json::Value {
    use http_body_util::BodyExt;
    let bytes = resp
        .into_hyper()
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    serde_json::from_slice(&bytes).expect("an Inertia visit returns a JSON page object")
}

/// A counting resolver, so a test can prove a resolver did or did not run.
fn counted(counter: Arc<AtomicUsize>, value: serde_json::Value) -> Prop {
    Prop::lazy(move || {
        let counter = counter.clone();
        let value = value.clone();
        async move {
            counter.fetch_add(1, Ordering::SeqCst);
            value
        }
    })
}

fn names(page: &serde_json::Value, field: &str) -> Vec<String> {
    page.get(field)
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default()
}

// ---- merge_with_path: nested merge targets ----

#[tokio::test]
async fn merge_with_path_emits_the_nested_key_in_merge_props() {
    let resp = InertiaResponse::new("Feed/Index")
        .prop(
            "posts",
            Prop::eager(json!({ "data": [{ "id": 1 }], "meta": { "total": 1 } }))
                .merge()
                .merge_with_path("data"),
        )
        .resolve(&MockReq::new("/feed").inertia())
        .await
        .unwrap();
    let page = page_of(resp).await;

    assert_eq!(names(&page, "mergeProps"), vec!["posts.data".to_string()]);
    assert!(
        names(&page, "mergeProps").iter().all(|p| p != "posts"),
        "a prop merging at a path must not also merge its whole value; got {page}"
    );
    assert_eq!(
        page["props"]["posts"]["data"],
        json!([{ "id": 1 }]),
        "the value itself is unaffected by the path — only the merge instruction narrows"
    );
}

#[tokio::test]
async fn merge_with_path_on_a_prepend_prop_emits_prepend_props_not_merge_props() {
    let resp = InertiaResponse::new("Feed/Index")
        .prop(
            "alerts",
            Prop::eager(json!({ "items": [] }))
                .prepend()
                .merge_with_path("items"),
        )
        .resolve(&MockReq::new("/feed").inertia())
        .await
        .unwrap();
    let page = page_of(resp).await;

    assert_eq!(
        names(&page, "prependProps"),
        vec!["alerts.items".to_string()]
    );
    assert!(names(&page, "mergeProps").is_empty());
}

#[tokio::test]
async fn merge_with_path_accumulates_multiple_paths_in_call_order() {
    let resp = InertiaResponse::new("Dashboard")
        .prop(
            "widgets",
            Prop::eager(json!({ "charts": [], "tables": [] }))
                .merge()
                .merge_with_path("charts")
                .merge_with_path("tables"),
        )
        .resolve(&MockReq::new("/dash").inertia())
        .await
        .unwrap();
    let page = page_of(resp).await;

    assert_eq!(
        names(&page, "mergeProps"),
        vec!["widgets.charts".to_string(), "widgets.tables".to_string()]
    );
}

#[tokio::test]
async fn merge_with_path_alone_without_a_merge_flag_is_ignored() {
    // `merge_with_path` is stored unconditionally and read only when
    // `merge_mode()` is set — the same "documented as ignored" shape
    // T32 established for `group()`/`rescue()` on a non-deferred prop.
    let resp = InertiaResponse::new("Feed/Index")
        .prop(
            "posts",
            Prop::eager(json!({ "data": [] })).merge_with_path("data"),
        )
        .resolve(&MockReq::new("/feed").inertia())
        .await
        .unwrap();
    let page = page_of(resp).await;

    assert_eq!(page["props"]["posts"], json!({ "data": [] }));
    assert!(
        !page.as_object().unwrap().contains_key("mergeProps"),
        "no merge flag was set, so merge_with_path must have no effect; got {page}"
    );
}

#[tokio::test]
async fn merge_with_path_is_ignored_on_a_deep_merge_prop() {
    // Deep merge already recurses into every nested field on its own —
    // Laravel excludes deep-merge props from the root/path partition
    // entirely (`Response.php:590`) and always emits the bare key.
    let resp = InertiaResponse::new("Chat/Show")
        .prop(
            "thread",
            Prop::eager(json!({ "messages": [] }))
                .deep_merge()
                .merge_with_path("messages"),
        )
        .resolve(&MockReq::new("/chat").inertia())
        .await
        .unwrap();
    let page = page_of(resp).await;

    assert_eq!(names(&page, "deepMergeProps"), vec!["thread".to_string()]);
    assert!(names(&page, "mergeProps").is_empty());
}

#[tokio::test]
async fn merge_with_path_composes_with_a_nested_match_on_field() {
    let resp = InertiaResponse::new("Feed/Index")
        .prop(
            "posts",
            Prop::eager(json!({ "data": [{ "id": 1 }] }))
                .merge()
                .merge_with_path("data")
                .match_on("data.id"),
        )
        .resolve(&MockReq::new("/feed").inertia())
        .await
        .unwrap();
    let page = page_of(resp).await;

    assert_eq!(names(&page, "mergeProps"), vec!["posts.data".to_string()]);
    assert_eq!(
        names(&page, "matchPropsOn"),
        vec!["posts.data.id".to_string()]
    );
}

// ---- match_on: several fields in one call ----

#[tokio::test]
async fn match_on_accepts_an_array_of_fields_in_one_call() {
    let resp = InertiaResponse::new("Feed/Index")
        .prop(
            "posts",
            Prop::eager(json!([])).merge().match_on(["id", "slug"]),
        )
        .resolve(&MockReq::new("/feed").inertia())
        .await
        .unwrap();
    let page = page_of(resp).await;

    assert_eq!(
        names(&page, "matchPropsOn"),
        vec!["posts.id".to_string(), "posts.slug".to_string()]
    );
}

#[tokio::test]
async fn match_on_array_call_and_chained_single_calls_accumulate_together() {
    let resp = InertiaResponse::new("Feed/Index")
        .prop(
            "posts",
            Prop::eager(json!([]))
                .merge()
                .match_on("id")
                .match_on(["slug", "uuid"]),
        )
        .resolve(&MockReq::new("/feed").inertia())
        .await
        .unwrap();
    let page = page_of(resp).await;

    assert_eq!(
        names(&page, "matchPropsOn"),
        vec![
            "posts.id".to_string(),
            "posts.slug".to_string(),
            "posts.uuid".to_string(),
        ]
    );
}

// ---- merge_lazy / merge_lazy_with: resolver-backed merge values ----

#[tokio::test]
async fn merge_lazy_resolves_the_value_and_emits_merge_props() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();

    let resp = InertiaResponse::new("Feed/Index")
        .merge_lazy("posts", move || {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok::<_, FrameworkError>(json!([{ "id": 9 }]))
            }
        })
        .resolve(&MockReq::new("/feed").inertia())
        .await
        .unwrap();
    let page = page_of(resp).await;

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(page["props"]["posts"], json!([{ "id": 9 }]));
    assert_eq!(names(&page, "mergeProps"), vec!["posts".to_string()]);
}

#[tokio::test]
async fn merge_lazy_with_applies_an_explicit_strategy_and_match_on() {
    let resp = InertiaResponse::new("Feed/Index")
        .merge_lazy_with(
            "posts",
            MergeStrategy::Prepend {
                match_on: Some("id".into()),
            },
            || async { Ok::<_, FrameworkError>(json!([{ "id": 3 }])) },
        )
        .resolve(&MockReq::new("/feed").inertia())
        .await
        .unwrap();
    let page = page_of(resp).await;

    assert_eq!(page["props"]["posts"], json!([{ "id": 3 }]));
    assert_eq!(names(&page, "prependProps"), vec!["posts".to_string()]);
    assert_eq!(names(&page, "matchPropsOn"), vec!["posts.id".to_string()]);
}

// ---- composition: defer + merge_with_path across two visits ----

#[tokio::test]
async fn defer_then_merge_with_path_announces_on_visit_one_and_merges_nested_on_the_follow_up() {
    // Visit 1: a standard visit. The resolver must not run; the key is
    // announced under deferredProps. The merge instruction still rides
    // along at its nested path — T32's metadata gate reads only
    // only/except, never whether the value resolved.
    let calls = Arc::new(AtomicUsize::new(0));
    let resp = InertiaResponse::new("Feed/Index")
        .prop(
            "posts",
            counted(calls.clone(), json!({ "data": [{ "id": 2 }] }))
                .defer()
                .merge()
                .merge_with_path("data")
                .match_on("data.id"),
        )
        .resolve(&MockReq::new("/feed").inertia())
        .await
        .unwrap();
    let page = page_of(resp).await;

    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(!page["props"].as_object().unwrap().contains_key("posts"));
    assert_eq!(page["deferredProps"]["default"], json!(["posts"]));
    assert_eq!(names(&page, "mergeProps"), vec!["posts.data".to_string()]);
    assert_eq!(
        names(&page, "matchPropsOn"),
        vec!["posts.data.id".to_string()]
    );

    // Visit 2: the follow-up partial. The resolver runs, the value lands
    // at props.posts, and the same nested merge instruction repeats.
    let calls = Arc::new(AtomicUsize::new(0));
    let req = MockReq::new("/feed")
        .inertia()
        .header("X-Inertia-Partial-Component", "Feed/Index")
        .header("X-Inertia-Partial-Data", "posts");

    let resp = InertiaResponse::new("Feed/Index")
        .prop(
            "posts",
            counted(calls.clone(), json!({ "data": [{ "id": 2 }] }))
                .defer()
                .merge()
                .merge_with_path("data")
                .match_on("data.id"),
        )
        .resolve(&req)
        .await
        .unwrap();
    let page = page_of(resp).await;

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(page["props"]["posts"]["data"], json!([{ "id": 2 }]));
    assert_eq!(names(&page, "mergeProps"), vec!["posts.data".to_string()]);
    assert!(!page.as_object().unwrap().contains_key("deferredProps"));
}
