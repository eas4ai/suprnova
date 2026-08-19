//! Share/prop ergonomics (wave 4, T25): dot-key nesting, `App::inertia_shared`
//! / `App::flush_inertia_shared` read-back and flush, the component-aware
//! `InertiaSharedData::share(&req, component)`, and `InertiaResponse::always_with`.
//!
//! `framework/tests/inertia.rs` covers the base shared-data precedence
//! (static registry < trait provider < builder) and lazy/once shares. This
//! file covers what T25 adds on top of that, driving `InertiaResponse::resolve`
//! through the same in-test `InertiaRequestExt` mock `inertia.rs` and
//! `inertia_prop_composition.rs` use — `hyper::body::Incoming` cannot be
//! constructed outside hyper's connection machinery.
//!
//! Tests that touch the shared registry use `TestContainer::fake()` for
//! per-test isolation, matching `inertia.rs`'s tier-1 shared-data tests.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use suprnova::{App, FrameworkError, InertiaRequestExt, InertiaResponse, InertiaSharedData, Prop};

/// Minimal `InertiaRequestExt` impl, mirroring `inertia_prop_composition.rs`.
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

// ---- dot-key nesting: InertiaResponse::with ----

#[tokio::test]
async fn with_dotted_key_nests_into_an_object() {
    let req = MockReq::new("/").inertia();
    let resp = InertiaResponse::new("Home")
        .with("user.name", "Todd")
        .resolve(&req)
        .await
        .unwrap();
    let page = page_of(resp).await;
    assert_eq!(page["props"]["user"]["name"], "Todd");
    assert!(
        page["props"].get("user.name").is_none(),
        "the literal dotted key must not survive"
    );
}

#[tokio::test]
async fn with_dotted_keys_sharing_a_prefix_accumulate_into_one_object() {
    let req = MockReq::new("/").inertia();
    let resp = InertiaResponse::new("Home")
        .with("user.name", "Todd")
        .with("user.age", 30)
        .resolve(&req)
        .await
        .unwrap();
    let page = page_of(resp).await;
    assert_eq!(page["props"]["user"]["name"], "Todd");
    assert_eq!(page["props"]["user"]["age"], 30);
}

#[tokio::test]
async fn dotted_key_overwrites_an_earlier_plain_scalar_at_the_same_segment() {
    let req = MockReq::new("/").inertia();
    let resp = InertiaResponse::new("Home")
        .with("user", "scalar")
        .with("user.name", "Todd")
        .resolve(&req)
        .await
        .unwrap();
    let page = page_of(resp).await;
    // `Arr::set` semantics: the scalar is silently replaced, not an error.
    assert_eq!(page["props"]["user"], serde_json::json!({ "name": "Todd" }));
}

#[tokio::test]
async fn a_later_plain_key_overwrites_an_earlier_dotted_object() {
    let req = MockReq::new("/").inertia();
    let resp = InertiaResponse::new("Home")
        .with("user.name", "Todd")
        .with("user", "scalar")
        .resolve(&req)
        .await
        .unwrap();
    let page = page_of(resp).await;
    assert_eq!(page["props"]["user"], serde_json::json!("scalar"));
}

// ---- dot-key nesting: shared registry ----

#[tokio::test]
async fn static_share_dotted_keys_nest_on_the_wire() {
    let _guard = suprnova::testing::TestContainer::fake();
    App::inertia_share("user.name", "Todd");
    App::inertia_share("user.locale", "es");

    let req = MockReq::new("/").inertia();
    let resp = InertiaResponse::new("Home").resolve(&req).await.unwrap();
    let page = page_of(resp).await;
    assert_eq!(page["props"]["user"]["name"], "Todd");
    assert_eq!(page["props"]["user"]["locale"], "es");
}

#[tokio::test]
async fn dotted_share_keys_advertise_their_root_segment_in_shared_props() {
    // `sharedProps` has to name the same top-level keys the client can
    // find in `props`. The Inertia client filters the list with a flat
    // `key in current.props` lookup and then spreads the survivors into
    // the intermediate page it renders during an instant swap
    // (`inertia-3.6.1/packages/core/src/router.ts:624-633`), so a raw
    // `"user.name"` entry never matches and `user` vanishes from that
    // frame entirely — a layout reading `props.user.name` throws.
    // Laravel has the same top-level shape because `Inertia::share`
    // runs `Arr::set` at share time
    // (`inertia-laravel-2.0.25/src/ResponseFactory.php:94`).
    let _guard = suprnova::testing::TestContainer::fake();
    App::inertia_share("user.name", "Todd");
    App::inertia_share("user.locale", "es");
    App::inertia_share("appName", "Suprnova");

    let req = MockReq::new("/").inertia();
    let resp = InertiaResponse::new("Home").resolve(&req).await.unwrap();
    let page = page_of(resp).await;

    let names: Vec<&str> = page["sharedProps"]
        .as_array()
        .expect("sharedProps should be an array")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    // Two dotted shares under one root collapse to a single entry.
    assert_eq!(names, vec!["user", "appName"]);
    assert!(page["props"]["user"]["name"] == "Todd");
}

#[tokio::test]
async fn static_and_builder_dotted_keys_nest_into_the_same_object() {
    let _guard = suprnova::testing::TestContainer::fake();
    App::inertia_share("user.name", "Todd");

    let req = MockReq::new("/").inertia();
    let resp = InertiaResponse::new("Home")
        .with("user.age", 30)
        .resolve(&req)
        .await
        .unwrap();
    let page = page_of(resp).await;
    assert_eq!(page["props"]["user"]["name"], "Todd");
    assert_eq!(page["props"]["user"]["age"], 30);
}

// ---- App::inertia_shared / App::flush_inertia_shared ----

#[tokio::test]
async fn inertia_shared_reads_back_an_eager_value() {
    let _guard = suprnova::testing::TestContainer::fake();
    App::inertia_share("appName", "Suprnova");
    assert_eq!(
        App::inertia_shared("appName"),
        Some(serde_json::json!("Suprnova"))
    );
}

#[tokio::test]
async fn inertia_shared_reads_back_a_dotted_value_by_its_full_key_or_its_parent() {
    let _guard = suprnova::testing::TestContainer::fake();
    App::inertia_share("user.name", "Todd");
    App::inertia_share("user.age", 30);

    assert_eq!(
        App::inertia_shared("user.name"),
        Some(serde_json::json!("Todd"))
    );
    assert_eq!(
        App::inertia_shared("user"),
        Some(serde_json::json!({ "name": "Todd", "age": 30 }))
    );
}

#[tokio::test]
async fn inertia_shared_returns_none_for_a_lazy_share() {
    let _guard = suprnova::testing::TestContainer::fake();
    App::inertia_share_lazy("locale", || async {
        Ok::<_, FrameworkError>("es".to_string())
    });
    assert_eq!(App::inertia_shared("locale"), None);
}

#[tokio::test]
async fn inertia_shared_returns_none_for_an_unregistered_key() {
    let _guard = suprnova::testing::TestContainer::fake();
    assert_eq!(App::inertia_shared("nope"), None);
}

#[tokio::test]
async fn flush_inertia_shared_clears_the_static_registry() {
    let _guard = suprnova::testing::TestContainer::fake();
    App::inertia_share("appName", "Suprnova");
    assert!(App::inertia_shared("appName").is_some());

    App::flush_inertia_shared();
    assert_eq!(App::inertia_shared("appName"), None);

    let req = MockReq::new("/").inertia();
    let resp = InertiaResponse::new("Home").resolve(&req).await.unwrap();
    let page = page_of(resp).await;
    assert!(page["props"].get("appName").is_none());
}

#[tokio::test]
async fn flush_inertia_shared_does_not_touch_the_trait_provider() {
    let _guard = suprnova::testing::TestContainer::fake();

    struct Provider;
    #[async_trait::async_trait]
    impl InertiaSharedData for Provider {
        async fn share(
            &self,
            _req: &dyn InertiaRequestExt,
            _component: &str,
        ) -> Result<indexmap::IndexMap<String, Prop>, FrameworkError> {
            let mut m = indexmap::IndexMap::new();
            m.insert("auth".to_string(), Prop::eager(serde_json::json!("alice")));
            Ok(m)
        }
    }
    App::register_inertia_shared(Arc::new(Provider));
    App::inertia_share("appName", "Suprnova");

    App::flush_inertia_shared();

    let req = MockReq::new("/").inertia();
    let resp = InertiaResponse::new("Home").resolve(&req).await.unwrap();
    let page = page_of(resp).await;
    assert!(
        page["props"].get("appName").is_none(),
        "the static share must be flushed"
    );
    assert_eq!(
        page["props"]["auth"], "alice",
        "the trait provider must survive the flush"
    );
}

// ---- component-aware shared provider ----

#[tokio::test]
async fn shared_provider_receives_the_component_name() {
    let _guard = suprnova::testing::TestContainer::fake();

    struct PerPageProvider;
    #[async_trait::async_trait]
    impl InertiaSharedData for PerPageProvider {
        async fn share(
            &self,
            _req: &dyn InertiaRequestExt,
            component: &str,
        ) -> Result<indexmap::IndexMap<String, Prop>, FrameworkError> {
            let mut m = indexmap::IndexMap::new();
            if component == "Dashboard" {
                m.insert(
                    "widgets".to_string(),
                    Prop::eager(serde_json::json!(["sales", "traffic"])),
                );
            }
            Ok(m)
        }
    }
    App::register_inertia_shared(Arc::new(PerPageProvider));

    let req = MockReq::new("/").inertia();
    let dashboard = page_of(
        InertiaResponse::new("Dashboard")
            .resolve(&req)
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(
        dashboard["props"]["widgets"],
        serde_json::json!(["sales", "traffic"])
    );

    let settings = page_of(
        InertiaResponse::new("Settings")
            .resolve(&req)
            .await
            .unwrap(),
    )
    .await;
    assert!(settings["props"].get("widgets").is_none());
}

// ---- InertiaResponse::always_with ----

#[tokio::test]
async fn always_with_resolver_ships_on_a_partial_reload_that_excludes_the_key() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();

    let req = MockReq::new("/")
        .inertia()
        .header("X-Inertia-Partial-Component", "Home")
        .header("X-Inertia-Partial-Data", "other_key");
    let resp = InertiaResponse::new("Home")
        .with("other_key", "present")
        .always_with("plan", move || {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok::<_, FrameworkError>("pro")
            }
        })
        .resolve(&req)
        .await
        .unwrap();
    let page = page_of(resp).await;
    assert_eq!(page["props"]["plan"], "pro");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
