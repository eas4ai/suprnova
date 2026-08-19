//! Prop composition: the orthogonal flags on `Prop` and how each
//! combination reaches the wire.
//!
//! `framework/tests/inertia.rs` covers each flag on its own through the
//! `InertiaResponse` shortcuts (`.defer`, `.merge`, `.once`, `.scroll`).
//! This file covers what those shortcuts cannot spell: two or more flags
//! on one prop, attached with `InertiaResponse::prop`.
//!
//! Like `inertia.rs`, these drive `InertiaResponse::resolve` through an
//! in-test `InertiaRequestExt` mock - `hyper::body::Incoming` cannot be
//! constructed outside hyper's connection machinery.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::json;
use suprnova::{FrameworkError, InertiaRequestExt, InertiaResponse, Prop, ScrollMetadata};

/// Minimal `InertiaRequestExt` impl, mirroring `framework/tests/inertia.rs`.
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

/// A resolver that always fails, so the rescue path has something to
/// catch. Written as a named function with an explicit return type so
/// the `PropFuture` coercion has an unambiguous target - the same shape
/// `prop.rs`'s own `failing_resolver` test helper uses.
fn failing_resolver() -> suprnova::PropResolver {
    Arc::new(|| Box::pin(async { Err(FrameworkError::internal("feed exploded")) }))
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

// ---- the headline: defer().merge() ----

#[tokio::test]
async fn defer_then_merge_announces_on_visit_one_and_merges_on_the_follow_up() {
    // Visit 1 - a standard Inertia visit. The prop is deferred, so the
    // resolver must not run and the key is announced under
    // `deferredProps`. The merge metadata rides along because Laravel
    // computes it from the unfiltered prop bag (`Response.php:553-560`)
    // and the client ignores it on a non-partial visit
    // (`inertia-3.6.1/packages/core/src/response.ts:348-350`).
    let calls = Arc::new(AtomicUsize::new(0));

    let resp = InertiaResponse::new("Feed/Index")
        .prop(
            "posts",
            counted(calls.clone(), json!([{ "id": 2 }]))
                .defer()
                .merge()
                .match_on("id"),
        )
        .resolve(&MockReq::new("/feed").inertia())
        .await
        .expect("initial visit resolves");
    let page = page_of(resp).await;

    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "a deferred resolver must not run on the initial visit"
    );
    assert!(
        !page["props"].as_object().unwrap().contains_key("posts"),
        "the deferred value must not be in props on visit 1; got {page}"
    );
    assert_eq!(page["deferredProps"]["default"], json!(["posts"]));
    assert_eq!(
        names(&page, "mergeProps"),
        vec!["posts".to_string()],
        "merge metadata is computed from the unfiltered prop bag, so it \
         rides the initial visit too; got {page}"
    );
    assert_eq!(names(&page, "matchPropsOn"), vec!["posts.id".to_string()]);

    // Visit 2 - the follow-up partial reload the client issues for the
    // deferred group. Now the resolver runs, the value lands in props,
    // the merge instruction is repeated, and nothing is announced as
    // still-deferred.
    let calls = Arc::new(AtomicUsize::new(0));
    let req = MockReq::new("/feed")
        .inertia()
        .header("X-Inertia-Partial-Component", "Feed/Index")
        .header("X-Inertia-Partial-Data", "posts");

    let resp = InertiaResponse::new("Feed/Index")
        .prop(
            "posts",
            counted(calls.clone(), json!([{ "id": 2 }]))
                .defer()
                .merge()
                .match_on("id"),
        )
        .resolve(&req)
        .await
        .expect("follow-up partial resolves");
    let page = page_of(resp).await;

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(page["props"]["posts"], json!([{ "id": 2 }]));
    assert_eq!(names(&page, "mergeProps"), vec!["posts".to_string()]);
    assert_eq!(names(&page, "matchPropsOn"), vec!["posts.id".to_string()]);
    assert!(
        !page.as_object().unwrap().contains_key("deferredProps"),
        "a resolved deferred prop must not be announced again; got {page}"
    );
}

#[tokio::test]
async fn defer_then_deep_merge_emits_deep_merge_props_on_the_follow_up() {
    let req = MockReq::new("/chat")
        .inertia()
        .header("X-Inertia-Partial-Component", "Chat/Show")
        .header("X-Inertia-Partial-Data", "thread");

    let resp = InertiaResponse::new("Chat/Show")
        .prop(
            "thread",
            Prop::lazy(|| async { json!({ "messages": [{ "id": 7 }] }) })
                .defer()
                .deep_merge(),
        )
        .resolve(&req)
        .await
        .unwrap();
    let page = page_of(resp).await;

    assert_eq!(page["props"]["thread"]["messages"][0]["id"], 7);
    assert_eq!(names(&page, "deepMergeProps"), vec!["thread".to_string()]);
    assert!(names(&page, "mergeProps").is_empty());
}

#[tokio::test]
async fn defer_then_prepend_emits_prepend_props() {
    let resp = InertiaResponse::new("Feed/Index")
        .prop(
            "alerts",
            Prop::lazy(|| async { json!([]) }).defer().prepend(),
        )
        .resolve(&MockReq::new("/feed").inertia())
        .await
        .unwrap();
    let page = page_of(resp).await;

    assert_eq!(names(&page, "prependProps"), vec!["alerts".to_string()]);
    assert!(names(&page, "mergeProps").is_empty());
    assert_eq!(page["deferredProps"]["default"], json!(["alerts"]));
}

#[tokio::test]
async fn defer_merge_drops_its_merge_metadata_when_the_client_asks_for_a_reset() {
    let req = MockReq::new("/feed")
        .inertia()
        .header("X-Inertia-Partial-Component", "Feed/Index")
        .header("X-Inertia-Partial-Data", "posts")
        .header("X-Inertia-Reset", "posts");

    let resp = InertiaResponse::new("Feed/Index")
        .prop(
            "posts",
            Prop::lazy(|| async { json!([{ "id": 1 }]) })
                .defer()
                .merge(),
        )
        .resolve(&req)
        .await
        .unwrap();
    let page = page_of(resp).await;

    assert_eq!(page["props"]["posts"], json!([{ "id": 1 }]));
    assert!(
        !names(&page, "mergeProps").contains(&"posts".to_string()),
        "a reset key must ship its value as a replacement, not an append; got {page}"
    );
}

#[tokio::test]
async fn defer_rescue_still_rescues_when_composed_with_merge() {
    let req = MockReq::new("/feed")
        .inertia()
        .header("X-Inertia-Partial-Component", "Feed/Index")
        .header("X-Inertia-Partial-Data", "posts");

    let resp = InertiaResponse::new("Feed/Index")
        .prop(
            "posts",
            Prop::from_resolver(failing_resolver())
                .defer()
                .rescue()
                .merge(),
        )
        .resolve(&req)
        .await
        .expect("rescue must convert the resolver error into a rescued key");
    let page = page_of(resp).await;

    assert!(!page["props"].as_object().unwrap().contains_key("posts"));
    assert_eq!(page["rescuedProps"], json!(["posts"]));
    // The merge instruction is metadata, gated by the only/except lists
    // alone - a rescued value never arrives, but the client still needs to
    // know how to fold in the one it eventually fetches.
    assert_eq!(names(&page, "mergeProps"), vec!["posts".to_string()]);
}

// ---- merge().once() ----

#[tokio::test]
async fn merge_then_once_emits_both_merge_and_once_metadata() {
    let calls = Arc::new(AtomicUsize::new(0));

    let resp = InertiaResponse::new("Billing/Index")
        .prop(
            "plans",
            counted(calls.clone(), json!([{ "id": 1 }])).merge().once(),
        )
        .resolve(&MockReq::new("/billing").inertia())
        .await
        .unwrap();
    let page = page_of(resp).await;

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(page["props"]["plans"], json!([{ "id": 1 }]));
    assert_eq!(names(&page, "mergeProps"), vec!["plans".to_string()]);
    assert_eq!(page["onceProps"]["plans"]["prop"], "plans");
    assert!(page["onceProps"]["plans"]["expiresAt"].is_null());
}

#[tokio::test]
async fn merge_once_skips_the_resolver_when_the_client_holds_the_cache_key() {
    let calls = Arc::new(AtomicUsize::new(0));
    let req = MockReq::new("/billing")
        .inertia()
        .header("X-Inertia-Except-Once-Props", "plans");

    let resp = InertiaResponse::new("Billing/Index")
        .prop(
            "plans",
            counted(calls.clone(), json!([{ "id": 1 }])).merge().once(),
        )
        .resolve(&req)
        .await
        .unwrap();
    let page = page_of(resp).await;

    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "the client claims the cache; the resolver must not run"
    );
    assert!(!page["props"].as_object().unwrap().contains_key("plans"));
    // Both metadata blocks still ship: the client needs `onceProps` to
    // restore the value and `mergeProps` to know how to fold it.
    assert_eq!(names(&page, "mergeProps"), vec!["plans".to_string()]);
    assert_eq!(page["onceProps"]["plans"]["prop"], "plans");
}

#[tokio::test]
async fn merge_once_with_a_custom_cache_key_and_expiry() {
    let resp = InertiaResponse::new("Billing/Index")
        .prop(
            "memberRoles",
            Prop::lazy(|| async { json!(["admin"]) })
                .merge()
                .once()
                .as_key("roles")
                .until(4_070_908_800_000),
        )
        .resolve(&MockReq::new("/billing").inertia())
        .await
        .unwrap();
    let page = page_of(resp).await;

    let once = page["onceProps"].as_object().unwrap();
    assert!(once.contains_key("roles"), "as_key renames the cache key");
    assert_eq!(once["roles"]["prop"], "memberRoles");
    assert_eq!(once["roles"]["expiresAt"], json!(4_070_908_800_000_i64));
}

#[tokio::test]
async fn once_fresh_beats_the_client_cache_claim_on_a_composed_prop() {
    let calls = Arc::new(AtomicUsize::new(0));
    let req = MockReq::new("/billing")
        .inertia()
        .header("X-Inertia-Except-Once-Props", "plans");

    let resp = InertiaResponse::new("Billing/Index")
        .prop(
            "plans",
            counted(calls.clone(), json!([{ "id": 9 }]))
                .merge()
                .once()
                .fresh(),
        )
        .resolve(&req)
        .await
        .unwrap();
    let page = page_of(resp).await;

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(page["props"]["plans"], json!([{ "id": 9 }]));
}

// ---- optional().once() ----

#[tokio::test]
async fn optional_then_once_stays_out_of_the_initial_visit_but_advertises_its_cache_key() {
    let calls = Arc::new(AtomicUsize::new(0));

    let resp = InertiaResponse::new("Team/Show")
        .prop(
            "permissions",
            counted(calls.clone(), json!(["read"])).optional().once(),
        )
        .resolve(&MockReq::new("/team").inertia())
        .await
        .unwrap();
    let page = page_of(resp).await;

    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "an optional prop is never resolved on a standard visit"
    );
    assert!(
        !page["props"]
            .as_object()
            .unwrap()
            .contains_key("permissions")
    );
    // The client tolerates an `onceProps` entry whose value is missing -
    // it skips such entries when building `X-Inertia-Except-Once-Props`
    // (`inertia-3.6.1/packages/core/src/request.ts:179-186`).
    assert_eq!(page["onceProps"]["permissions"]["prop"], "permissions");
}

#[tokio::test]
async fn optional_once_resolves_on_the_partial_that_asks_for_it() {
    let calls = Arc::new(AtomicUsize::new(0));
    let req = MockReq::new("/team")
        .inertia()
        .header("X-Inertia-Partial-Component", "Team/Show")
        .header("X-Inertia-Partial-Data", "permissions");

    let resp = InertiaResponse::new("Team/Show")
        .prop(
            "permissions",
            counted(calls.clone(), json!(["read"])).optional().once(),
        )
        .resolve(&req)
        .await
        .unwrap();
    let page = page_of(resp).await;

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(page["props"]["permissions"], json!(["read"]));
    assert_eq!(page["onceProps"]["permissions"]["prop"], "permissions");
}

// ---- defer().once() ----

#[tokio::test]
async fn defer_once_stops_announcing_the_key_after_the_client_has_it() {
    // First visit: nothing cached, so the key is announced.
    let resp = InertiaResponse::new("Dashboard")
        .prop(
            "rates",
            Prop::lazy(|| async { json!({ "usd": 1 }) }).defer().once(),
        )
        .resolve(&MockReq::new("/dash").inertia())
        .await
        .unwrap();
    let page = page_of(resp).await;
    assert_eq!(page["deferredProps"]["default"], json!(["rates"]));
    assert_eq!(page["onceProps"]["rates"]["prop"], "rates");

    // Second visit: the client says it already holds `rates`. Announcing
    // it again would make the client refetch on every navigation and
    // `once` would buy nothing (`Response.php:653-673`).
    let req = MockReq::new("/dash")
        .inertia()
        .header("X-Inertia-Except-Once-Props", "rates");
    let resp = InertiaResponse::new("Dashboard")
        .prop(
            "rates",
            Prop::lazy(|| async { json!({ "usd": 1 }) }).defer().once(),
        )
        .resolve(&req)
        .await
        .unwrap();
    let page = page_of(resp).await;

    assert!(
        !page.as_object().unwrap().contains_key("deferredProps"),
        "a cached deferred prop must not be announced again; got {page}"
    );
    assert_eq!(page["onceProps"]["rates"]["prop"], "rates");
}

#[tokio::test]
async fn defer_once_announces_again_once_the_server_side_expiry_has_passed() {
    // Epoch + 1ms is long past, so the server refuses to honour the
    // client's cache claim (Domain 20 audit D20-C).
    let req = MockReq::new("/dash")
        .inertia()
        .header("X-Inertia-Except-Once-Props", "rates");
    let resp = InertiaResponse::new("Dashboard")
        .prop(
            "rates",
            Prop::lazy(|| async { json!({ "usd": 1 }) })
                .defer()
                .once()
                .until(1),
        )
        .resolve(&req)
        .await
        .unwrap();
    let page = page_of(resp).await;

    assert_eq!(page["deferredProps"]["default"], json!(["rates"]));
}

// ---- mutually exclusive visibility ----

#[tokio::test]
async fn always_after_optional_wins() {
    let req = MockReq::new("/team")
        .inertia()
        .header("X-Inertia-Partial-Component", "Team/Show")
        .header("X-Inertia-Partial-Data", "other");

    let resp = InertiaResponse::new("Team/Show")
        .prop(
            "flags",
            Prop::eager(json!({ "beta": true })).optional().always(),
        )
        .with("other", 1)
        .resolve(&req)
        .await
        .unwrap();
    let page = page_of(resp).await;

    assert_eq!(
        page["props"]["flags"],
        json!({ "beta": true }),
        "always() must erase the earlier optional(); got {page}"
    );
}

#[tokio::test]
async fn optional_after_always_wins_too() {
    let resp = InertiaResponse::new("Team/Show")
        .prop("flags", Prop::eager(json!(1)).always().optional())
        .resolve(&MockReq::new("/team").inertia())
        .await
        .unwrap();
    let page = page_of(resp).await;

    assert!(
        !page["props"].as_object().unwrap().contains_key("flags"),
        "optional() must erase the earlier always(); got {page}"
    );
}

#[tokio::test]
async fn defer_after_always_wins_and_announces() {
    let resp = InertiaResponse::new("Team/Show")
        .prop("flags", Prop::lazy(|| async { json!(1) }).always().defer())
        .resolve(&MockReq::new("/team").inertia())
        .await
        .unwrap();
    let page = page_of(resp).await;

    assert!(!page["props"].as_object().unwrap().contains_key("flags"));
    assert_eq!(page["deferredProps"]["default"], json!(["flags"]));
}

// ---- flags that are stored but ignored ----

#[tokio::test]
async fn group_and_rescue_are_ignored_on_a_prop_that_is_not_deferred() {
    let resp = InertiaResponse::new("Home")
        .prop(
            "stats",
            Prop::lazy(|| async { json!({ "hits": 3 }) })
                .group("analytics")
                .rescue(),
        )
        .resolve(&MockReq::new("/").inertia())
        .await
        .unwrap();
    let page = page_of(resp).await;

    assert_eq!(page["props"]["stats"], json!({ "hits": 3 }));
    assert!(
        !page.as_object().unwrap().contains_key("deferredProps"),
        "group() alone must not defer a prop; got {page}"
    );
}

#[tokio::test]
async fn scroll_wrap_is_ignored_on_a_prop_that_is_not_scroll() {
    // `Prop::scroll_wrap`'s own doc comment claims this is inert without
    // `.scroll(...)` set - same "stored but ignored" shape as
    // `group()`/`rescue()` on a non-deferred prop above. Nothing enforced
    // it at the wire level until this test.
    let resp = InertiaResponse::new("Feed/Index")
        .prop(
            "posts",
            Prop::eager(json!({ "data": [{ "id": 1 }] })).scroll_wrap("data"),
        )
        .resolve(&MockReq::new("/feed").inertia())
        .await
        .unwrap();
    let page = page_of(resp).await;

    assert_eq!(page["props"]["posts"], json!({ "data": [{ "id": 1 }] }));
    assert!(
        !page.as_object().unwrap().contains_key("scrollProps"),
        "scroll_wrap() without scroll() must not produce a scrollProps entry; got {page}"
    );
    assert!(
        names(&page, "mergeProps").is_empty(),
        "scroll_wrap() without scroll() must not produce merge metadata; got {page}"
    );
}

#[tokio::test]
async fn a_scroll_prop_ignores_an_explicit_merge_flag_and_uses_the_intent_header() {
    let req = MockReq::new("/users")
        .inertia()
        .header("X-Inertia-Infinite-Scroll-Merge-Intent", "prepend");

    let resp = InertiaResponse::new("Users/Index")
        .prop(
            "users",
            Prop::eager(json!([{ "id": 1 }]))
                .scroll(ScrollMetadata::new("page").current(2).previous(1))
                .merge(),
        )
        .resolve(&req)
        .await
        .unwrap();
    let page = page_of(resp).await;

    assert_eq!(
        names(&page, "prependProps"),
        vec!["users".to_string()],
        "the intent header owns the direction on a scroll prop; got {page}"
    );
    assert!(
        names(&page, "mergeProps").is_empty(),
        "the explicit merge flag must be ignored on a scroll prop; got {page}"
    );
    assert_eq!(page["scrollProps"]["users"]["reset"], false);
}

#[tokio::test]
async fn an_absent_prop_emits_neither_a_value_nor_metadata_whatever_its_flags() {
    let resp = InertiaResponse::new("Album/Show")
        .prop("songs", Prop::absent().defer().merge().once())
        .resolve(&MockReq::new("/albums/1").inertia())
        .await
        .unwrap();
    let page = page_of(resp).await;

    let obj = page.as_object().unwrap();
    assert!(!page["props"].as_object().unwrap().contains_key("songs"));
    assert!(!obj.contains_key("deferredProps"), "got {page}");
    assert!(!obj.contains_key("mergeProps"), "got {page}");
    assert!(!obj.contains_key("onceProps"), "got {page}");
}

#[tokio::test]
async fn an_always_merge_prop_keeps_its_value_but_drops_merge_metadata_when_filtered_out() {
    // Laravel's metadata gate is `only`/`except` alone and knows nothing
    // about `AlwaysProp` (`Response.php:553-560`), so an always prop that
    // is outside the requested set still ships its value and loses its
    // merge instruction. Odd, but wire-identical.
    let req = MockReq::new("/feed")
        .inertia()
        .header("X-Inertia-Partial-Component", "Feed/Index")
        .header("X-Inertia-Partial-Data", "other");

    let resp = InertiaResponse::new("Feed/Index")
        .prop("banner", Prop::eager(json!(["a"])).always().merge())
        .with("other", 1)
        .resolve(&req)
        .await
        .unwrap();
    let page = page_of(resp).await;

    assert_eq!(page["props"]["banner"], json!(["a"]));
    assert!(
        names(&page, "mergeProps").is_empty(),
        "merge metadata follows only/except, not always-ness; got {page}"
    );
}

#[tokio::test]
async fn repeated_match_on_accumulates_into_match_props_on() {
    let resp = InertiaResponse::new("Feed/Index")
        .prop(
            "posts",
            Prop::eager(json!([]))
                .merge()
                .match_on("id")
                .match_on("slug"),
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
