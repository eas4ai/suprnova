//! Integration tests for the composition of `?include=` (DTO lazy resolution)
//! and `X-Inertia-Partial-Data` (Inertia partial-reload filtering).
//!
//! Gate order (spec): include-set + allowlist enforcement runs FIRST (Stage 1);
//! `X-Inertia-Partial-Data` is applied AFTER as the final "only" filter
//! (Stage 2). A disallowed `?include=` field must return 400 even when
//! `X-Inertia-Partial-Data` would have filtered the field out anyway.

use std::collections::HashMap;
use std::sync::Arc;

use suprnova::data::{REQUEST_INCLUDE_SET, RequestIncludeSet, registry};
use suprnova::inertia::Prop;
use suprnova::{InertiaRequestExt, InertiaResponse};

// ---------------------------------------------------------------------------
// Test request fixture — mirrors the MockReq used in framework/tests/inertia.rs
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Body reader helper — equivalent to the one in tests/inertia.rs
// ---------------------------------------------------------------------------

async fn body_to_string(
    body: http_body_util::combinators::BoxBody<bytes::Bytes, std::convert::Infallible>,
) -> String {
    use http_body_util::BodyExt;
    let bytes = body.collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

// ---------------------------------------------------------------------------
// Test A: partial-data filters AFTER ?include= resolves the lazy field.
//
// ?include=albums (via task-local) → resolver would run.
// X-Inertia-Partial-Data: name → only "name" passes the partial-data gate.
// Result: "name" present, "albums" absent (partial-data pre-gates it out
// before the include-set check even runs).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn partial_data_filters_after_include_resolves() {
    registry::register("_test_ArtistDto_t6a", &["albums"]);

    let set = Arc::new(RequestIncludeSet {
        include: vec!["albums".into()],
        ..Default::default()
    });

    let req = MockReq::new("/artist/1")
        .inertia()
        .header("X-Inertia-Partial-Component", "Artist/Show")
        .header("X-Inertia-Partial-Data", "name");

    let resp = REQUEST_INCLUDE_SET
        .scope(
            set,
            InertiaResponse::new("Artist/Show")
                .with("name", "Beethoven")
                .prop_lazy_with_owner(
                    "_test_ArtistDto_t6a",
                    "albums",
                    Prop::lazy(|| async { serde_json::json!(["Symphony 9"]) }),
                )
                .resolve(&req),
        )
        .await
        .unwrap();

    let body = body_to_string(resp.into_hyper().into_body()).await;
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();

    // "name" is in X-Inertia-Partial-Data → included.
    assert!(
        page["props"]["name"].is_string(),
        "expected 'name' prop to be present, got: {:?}",
        page["props"]
    );
    assert_eq!(page["props"]["name"], "Beethoven");

    // "albums" is NOT in X-Inertia-Partial-Data → partial-data gate excludes it
    // before the include-set resolver even runs. Use contains_key to distinguish
    // "absent key" from "key present with null value".
    assert!(
        !page["props"].as_object().unwrap().contains_key("albums"),
        "expected 'albums' prop to be absent (partial-data filtered), got: {:?}",
        page["props"]
    );
}

// ---------------------------------------------------------------------------
// Test B: no partial-data header → full include-resolved set returned.
//
// ?include=albums (via task-local), no X-Inertia-Partial-Data.
// Both "name" (eager) and "albums" (lazy-owned, in include set) appear.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn no_partial_data_returns_full_include_resolved_set() {
    registry::register("_test_ArtistDto_t6b", &["albums"]);

    let set = Arc::new(RequestIncludeSet {
        include: vec!["albums".into()],
        ..Default::default()
    });

    // No X-Inertia-Partial-Data header — full set returned.
    let req = MockReq::new("/artist/1").inertia();

    let resp = REQUEST_INCLUDE_SET
        .scope(
            set,
            InertiaResponse::new("Artist/Show")
                .with("name", "Beethoven")
                .prop_lazy_with_owner(
                    "_test_ArtistDto_t6b",
                    "albums",
                    Prop::lazy(|| async { serde_json::json!(["Symphony 9"]) }),
                )
                .resolve(&req),
        )
        .await
        .unwrap();

    let body = body_to_string(resp.into_hyper().into_body()).await;
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();

    // Both props present — no partial-data filter active.
    assert_eq!(
        page["props"]["name"], "Beethoven",
        "expected 'name' prop, got: {:?}",
        page["props"]
    );
    assert_eq!(
        page["props"]["albums"],
        serde_json::json!(["Symphony 9"]),
        "expected 'albums' prop resolved from include set, got: {:?}",
        page["props"]
    );
}

// ---------------------------------------------------------------------------
// Test C: both ?include=albums AND X-Inertia-Partial-Data: albums.
//
// The two filters agree on "albums" → it is present.
// "name" is an eager prop but excluded by partial-data.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn partial_data_and_include_both_request_same_key() {
    registry::register("_test_ArtistDto_t6c", &["albums"]);

    let set = Arc::new(RequestIncludeSet {
        include: vec!["albums".into()],
        ..Default::default()
    });

    let req = MockReq::new("/artist/1")
        .inertia()
        .header("X-Inertia-Partial-Component", "Artist/Show")
        .header("X-Inertia-Partial-Data", "albums");

    let resp = REQUEST_INCLUDE_SET
        .scope(
            set,
            InertiaResponse::new("Artist/Show")
                .with("name", "Beethoven")
                .prop_lazy_with_owner(
                    "_test_ArtistDto_t6c",
                    "albums",
                    Prop::lazy(|| async { serde_json::json!(["Symphony 9"]) }),
                )
                .resolve(&req),
        )
        .await
        .unwrap();

    let body = body_to_string(resp.into_hyper().into_body()).await;
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();

    // "albums" passes both gates — present.
    assert_eq!(
        page["props"]["albums"],
        serde_json::json!(["Symphony 9"]),
        "expected 'albums' prop when both partial-data and include agree, got: {:?}",
        page["props"]
    );

    // "name" is NOT in X-Inertia-Partial-Data → excluded.
    assert!(
        !page["props"].as_object().unwrap().contains_key("name"),
        "expected 'name' prop to be absent (partial-data restricts to 'albums'), got: {:?}",
        page["props"]
    );
}

// ---------------------------------------------------------------------------
// Test D: disallowed ?include= returns 400 even when X-Inertia-Partial-Data
//         would have filtered the field out anyway.
//
// Security contract: the include-set + allowlist gate (Stage 1) MUST fire
// before the partial-data filter (Stage 2) can swallow the error.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn disallowed_include_returns_400_even_when_partial_data_narrower() {
    // `lyrics` is NOT on the allowlist — only `albums` is allowed.
    registry::register("_test_ArtistDto_t6d", &["albums"]);

    let set = Arc::new(RequestIncludeSet {
        include: vec!["lyrics".into()], // attacker requests a disallowed field
        ..Default::default()
    });

    // Partial-data only asks for `name`. Without the correct gate order, the
    // partial-data filter would silently skip `lyrics` before
    // `resolve_with_owner` can raise the 400 — masking the security error.
    let req = MockReq::new("/artist/1")
        .inertia()
        .header("X-Inertia-Partial-Component", "Artist/Show")
        .header("X-Inertia-Partial-Data", "name");

    let result = REQUEST_INCLUDE_SET
        .scope(
            set,
            InertiaResponse::new("Artist/Show")
                .with("name", "Beethoven")
                .prop_lazy_with_owner(
                    "_test_ArtistDto_t6d",
                    "lyrics",
                    Prop::lazy(|| async { serde_json::json!("la la") }),
                )
                .resolve(&req),
        )
        .await;

    // Spec requires 400 on disallowed include — must fire even when
    // partial-data would have filtered the field out before resolution.
    match result {
        Ok(_) => panic!("expected Err(400 disallowed include), got Ok response"),
        Err(err) => assert_eq!(
            err.status_code(),
            400,
            "expected HTTP 400 for disallowed include, got {}",
            err.status_code()
        ),
    }
}

// ---------------------------------------------------------------------------
// Test E: a DEFERRED owner-tagged field honours the same `?include=` +
//         allowlist gate as a plain lazy one.
//
// `#[data(lazy(deferred))]` emits `PropEntry::DeferredOwned`, whose `Prop`
// carries `Visibility::Deferred` — so `Prop::is_lazy()` is false for it and
// the owner-tagged fast path in `resolve_props` used to skip it entirely.
// The field then resolved off the ordinary prop path, with no include-set
// check anywhere in it: the DTO's opt-in field shipped to any client that
// sent the deferred follow-up.
// ---------------------------------------------------------------------------

use suprnova::inertia::PropEntry;

fn deferred_owned(owner: &'static str, field: &'static str, value: serde_json::Value) -> PropEntry {
    PropEntry::DeferredOwned {
        owner,
        field,
        prop: Prop::lazy(move || {
            let value = value.clone();
            async move { value }
        })
        .defer(),
    }
}

#[tokio::test]
async fn a_deferred_owner_tagged_field_stays_out_when_include_does_not_name_it() {
    registry::register("_test_ArtistDto_t6e", &["albums"]);

    // `?include=` is empty: the client never opted into `albums`.
    let set = Arc::new(RequestIncludeSet::default());

    // The deferred follow-up: a matched partial reload naming the key.
    let req = MockReq::new("/artist/1")
        .inertia()
        .header("X-Inertia-Partial-Component", "Artist/Show")
        .header("X-Inertia-Partial-Data", "albums");

    let resp = REQUEST_INCLUDE_SET
        .scope(
            set,
            InertiaResponse::from_data_props(
                "Artist/Show",
                vec![
                    (
                        "name".to_string(),
                        PropEntry::Eager(serde_json::json!("Beethoven")),
                    ),
                    (
                        "albums".to_string(),
                        deferred_owned(
                            "_test_ArtistDto_t6e",
                            "albums",
                            serde_json::json!(["Symphony 9"]),
                        ),
                    ),
                ],
            )
            .resolve(&req),
        )
        .await
        .unwrap();

    let body = body_to_string(resp.into_hyper().into_body()).await;
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert!(
        !page["props"].as_object().unwrap().contains_key("albums"),
        "a deferred owner-tagged field must honour ?include=, got: {page}"
    );
}

#[tokio::test]
async fn a_deferred_owner_tagged_field_is_not_announced_when_include_does_not_name_it() {
    registry::register("_test_ArtistDto_t6f", &["albums"]);

    // Initial visit, empty `?include=`. Announcing the key would send the
    // client after a field this request never opted into.
    let set = Arc::new(RequestIncludeSet::default());
    let req = MockReq::new("/artist/1").inertia();

    let resp = REQUEST_INCLUDE_SET
        .scope(
            set,
            InertiaResponse::from_data_props(
                "Artist/Show",
                vec![(
                    "albums".to_string(),
                    deferred_owned(
                        "_test_ArtistDto_t6f",
                        "albums",
                        serde_json::json!(["Symphony 9"]),
                    ),
                )],
            )
            .resolve(&req),
        )
        .await
        .unwrap();

    let body = body_to_string(resp.into_hyper().into_body()).await;
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert!(
        !page.as_object().unwrap().contains_key("deferredProps"),
        "a field outside ?include= must not be announced, got: {page}"
    );
}

#[tokio::test]
async fn a_deferred_owner_tagged_field_resolves_when_include_names_it() {
    registry::register("_test_ArtistDto_t6g", &["albums"]);

    let set = Arc::new(RequestIncludeSet {
        include: vec!["albums".into()],
        ..Default::default()
    });

    // Visit 1: announced, not resolved.
    let req = MockReq::new("/artist/1").inertia();
    let resp = REQUEST_INCLUDE_SET
        .scope(
            Arc::clone(&set),
            InertiaResponse::from_data_props(
                "Artist/Show",
                vec![(
                    "albums".to_string(),
                    deferred_owned(
                        "_test_ArtistDto_t6g",
                        "albums",
                        serde_json::json!(["Symphony 9"]),
                    ),
                )],
            )
            .resolve(&req),
        )
        .await
        .unwrap();
    let body = body_to_string(resp.into_hyper().into_body()).await;
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        page["deferredProps"]["default"],
        serde_json::json!(["albums"])
    );
    assert!(!page["props"].as_object().unwrap().contains_key("albums"));

    // Visit 2: the deferred follow-up. Both gates agree, so it ships.
    let req = MockReq::new("/artist/1")
        .inertia()
        .header("X-Inertia-Partial-Component", "Artist/Show")
        .header("X-Inertia-Partial-Data", "albums");
    let resp = REQUEST_INCLUDE_SET
        .scope(
            set,
            InertiaResponse::from_data_props(
                "Artist/Show",
                vec![(
                    "albums".to_string(),
                    deferred_owned(
                        "_test_ArtistDto_t6g",
                        "albums",
                        serde_json::json!(["Symphony 9"]),
                    ),
                )],
            )
            .resolve(&req),
        )
        .await
        .unwrap();
    let body = body_to_string(resp.into_hyper().into_body()).await;
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(page["props"]["albums"], serde_json::json!(["Symphony 9"]));
}

#[tokio::test]
async fn a_deferred_owner_tagged_field_with_a_disallowed_include_returns_400() {
    // `lyrics` is not on the allowlist. The 400 must fire on the initial
    // visit too — the flag-free path raises it regardless of what
    // `X-Inertia-Partial-Data` says (Test D), and a flag on the prop
    // cannot be what buys an attacker silence.
    registry::register("_test_ArtistDto_t6h", &["albums"]);

    let set = Arc::new(RequestIncludeSet {
        include: vec!["lyrics".into()],
        ..Default::default()
    });

    let req = MockReq::new("/artist/1").inertia();
    let result = REQUEST_INCLUDE_SET
        .scope(
            set,
            InertiaResponse::from_data_props(
                "Artist/Show",
                vec![(
                    "lyrics".to_string(),
                    deferred_owned("_test_ArtistDto_t6h", "lyrics", serde_json::json!("la la")),
                )],
            )
            .resolve(&req),
        )
        .await;

    match result {
        Ok(_) => panic!("expected Err(400 disallowed include), got Ok response"),
        Err(err) => assert_eq!(
            err.status_code(),
            400,
            "expected HTTP 400 for a disallowed include on a deferred field, got {}",
            err.status_code()
        ),
    }
}
