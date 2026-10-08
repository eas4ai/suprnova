//! PAR-049: the `Inertia` facade's runtime surface - shared data in every
//! form `ResponseFactory::share` takes, `get_shared`, the component
//! transformer and the `ensure_pages_exist` check.
//!
//! Laravel's references are `ResponseFactory::share`, `getShared`,
//! `transformComponentUsing`, `render` and `findComponentOrFail` in
//! inertia-laravel 3.5.1.

use std::sync::Arc;

use serde_json::json;
use suprnova::testing::TestContainer;
use suprnova::{App, Inertia, InertiaConfig, InertiaResponse, InertiaSharedData, Prop};

use crate::protocol_harness::{MockReq, page_of};

async fn render(component: &str) -> serde_json::Value {
    let response = InertiaResponse::new(component)
        .resolve(&MockReq::new("/").inertia())
        .await
        .unwrap();
    page_of(response)
}

#[tokio::test]
async fn inp_a_dotted_share_nests_when_it_is_shared() {
    // `Arr::set` at share time: a later plain share of the parent replaces
    // the whole object, so a child shared in between does not survive it.
    let _guard = TestContainer::fake();
    App::inertia_share("user", json!({"name": "A"}));
    App::inertia_share("user.age", 3);
    App::inertia_share("user", json!({"name": "B"}));

    let page = render("Home").await;
    assert_eq!(page["props"]["user"], json!({"name": "B"}), "{page}");
}

#[tokio::test]
async fn inp_inertia_share_reads_back_nested_and_with_a_default() {
    let _guard = TestContainer::fake();
    Inertia::share("a.b", 1).unwrap();

    assert_eq!(Inertia::get_shared_all(), json!({"a": {"b": 1}}));
    assert_eq!(Inertia::get_shared("a", json!(null)), json!({"b": 1}));
    assert_eq!(Inertia::get_shared("a.b", json!(null)), json!(1));
    assert_eq!(Inertia::get_shared("missing", 7), json!(7));
    assert_eq!(render("Home").await["props"]["a"], json!({"b": 1}));
}

#[derive(suprnova::Data, validator::Validate)]
struct SiteMeta {
    site: String,
    build: u32,
}

#[tokio::test]
async fn inp_inertia_share_takes_a_map_and_a_data_object() {
    let _guard = TestContainer::fake();
    Inertia::share_many([("locale", json!("en")), ("menu.open", json!(true))]).unwrap();
    Inertia::share_data(SiteMeta {
        site: "Suprnova".to_string(),
        build: 7,
    })
    .unwrap();

    let props = render("Home").await["props"].clone();
    assert_eq!(props["locale"], "en");
    assert_eq!(props["menu"], json!({"open": true}));
    assert_eq!(props["site"], "Suprnova");
    assert_eq!(props["build"], 7);
}

struct Tenant;

#[suprnova::async_trait]
impl InertiaSharedData for Tenant {
    async fn share(
        &self,
        _req: &dyn suprnova::InertiaRequestExt,
        component: &str,
    ) -> Result<suprnova::indexmap::IndexMap<String, Prop>, suprnova::FrameworkError> {
        let mut out = suprnova::indexmap::IndexMap::new();
        out.insert("tenant".to_string(), Prop::eager(json!(component)));
        Ok(out)
    }
}

#[tokio::test]
async fn inp_inertia_share_takes_a_provider() {
    let _guard = TestContainer::fake();
    Inertia::share_provider(Arc::new(Tenant));
    assert_eq!(render("Billing").await["props"]["tenant"], "Billing");
}

#[tokio::test]
async fn inp_a_component_transformer_renames_the_component_before_render() {
    let _guard = TestContainer::fake();
    Inertia::transform_component_using(|component| {
        component
            .strip_prefix("Old/")
            .map(|rest| format!("New/{rest}"))
    });

    assert_eq!(render("Old/Users").await["component"], "New/Users");
    // `None` keeps the name it was given.
    assert_eq!(render("Users").await["component"], "Users");
}

/// A pages directory of its own, removed when the test ends.
struct PagesDir(std::path::PathBuf);

impl PagesDir {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("suprnova-inp-pages-{name}-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("Admin")).unwrap();
        std::fs::write(dir.join("Present.svelte"), "").unwrap();
        std::fs::write(dir.join("Admin").join("Users.vue"), "").unwrap();
        Self(dir)
    }
}

impl Drop for PagesDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn inp_ensure_pages_exist_makes_a_missing_page_an_error() {
    let pages = PagesDir::new("missing");
    let config = InertiaConfig::new()
        .ensure_pages_exist(true)
        .pages_dir(&pages.0);
    let req = MockReq::new("/").inertia();

    let Err(error) = InertiaResponse::new("Missing")
        .with_config(config.clone())
        .resolve(&req)
        .await
    else {
        panic!("a component with no page file must not render");
    };
    let message = error.to_string();
    assert!(message.contains("Missing"), "{message}");
    assert!(
        message.contains(&pages.0.display().to_string()),
        "{message}"
    );

    for present in ["Present", "Admin/Users"] {
        InertiaResponse::new(present)
            .with_config(config.clone())
            .resolve(&req)
            .await
            .unwrap_or_else(|e| panic!("{present} has a page file: {e}"));
    }
    // A name that climbs out of the directory is never looked up.
    assert!(
        InertiaResponse::new("../missing-pages/Present")
            .with_config(config.clone())
            .resolve(&req)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn inp_ensure_pages_exist_is_off_by_default() {
    let req = MockReq::new("/").inertia();
    assert!(
        InertiaResponse::new("NoSuchPage")
            .with_config(InertiaConfig::new())
            .resolve(&req)
            .await
            .is_ok()
    );
}
