//! End-to-end tests for the Inertia response pipeline.
//!
//! Drives `InertiaResponse::resolve` directly through an in-test
//! `InertiaRequestExt` mock so the full filtering + page-object
//! materialization path is covered without booting a real server.
//! `hyper::body::Incoming` cannot be constructed outside hyper's
//! connection machinery, which is why these tests go through the
//! trait rather than `suprnova::Request` directly.
//!
//! Tier 1 shared-data tests use `TestContainer::fake()` for per-test
//! isolation - the container's Inertia registry is scoped to the
//! guard's lifetime, so tests run in parallel without seeing each
//! other's registrations.

use std::collections::HashMap;
use suprnova::testing::{AssertableInertia, ReloadRequest};
use suprnova::{
    Frontend, InertiaConfig, InertiaRequestExt, InertiaResponse, MANIFEST_VERSION_FALLBACK,
    VersionResolver,
};

/// Minimal `InertiaRequestExt` impl for tests.
struct MockReq {
    path: String,
    query: Option<String>,
    headers: HashMap<String, String>,
}

impl MockReq {
    fn new(path: &str) -> Self {
        Self {
            path: path.to_string(),
            query: None,
            headers: HashMap::new(),
        }
    }

    /// Attach a query string (no leading `?`), so `path_and_query()`
    /// returns what a real `Request` returns for `/users?page=2`.
    fn query(mut self, query: &str) -> Self {
        self.query = Some(query.to_string());
        self
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
    fn path_and_query(&self) -> String {
        match &self.query {
            Some(q) => format!("{}?{}", self.path, q),
            None => self.path.clone(),
        }
    }
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(|s| s.as_str())
    }
}

#[tokio::test]
async fn initial_html_visit_returns_shell_with_embedded_page_object() {
    let req = MockReq::new("/home"); // no X-Inertia header → HTML response
    let resp = InertiaResponse::new("Home")
        .with("title", "Welcome")
        .with("count", 42u32)
        .resolve(&req)
        .await
        .unwrap();

    // `AssertableInertia::from_response` reads the page object out of the
    // HTML shell's embedded <script data-page="app"> the same way it
    // would read an X-Inertia JSON body.
    AssertableInertia::from_response(&resp)
        .component("Home")
        .url("/home")
        .where_("title", "Welcome")
        .where_("count", 42);

    let hyper_resp = resp.into_hyper();
    assert_eq!(hyper_resp.status(), 200);

    let content_type = hyper_resp.headers().get("Content-Type").unwrap();
    assert!(content_type.to_str().unwrap().starts_with("text/html"));

    let vary = hyper_resp.headers().get("Vary").unwrap();
    assert_eq!(vary, "X-Inertia");

    let body = body_to_string(hyper_resp.into_body());
    assert!(body.contains("<!DOCTYPE html>"));
    assert!(body.contains("<title>Suprnova</title>"));
    // Inertia 3 contract: the initial page lives in a sibling
    // <script type="application/json" data-page="app"> alongside an empty
    // <div id="app"></div> mount node - read by getInitialPageFromDOM.
    assert!(body.contains(r#"<script type="application/json" data-page="app">"#));
    assert!(body.contains(r#"<div id="app"></div>"#));
    assert!(body.contains(r#"<div id="app"></div>"#));
}

#[tokio::test]
async fn inertia_xhr_visit_returns_json_page_object() {
    let req = MockReq::new("/users").inertia();
    let resp = InertiaResponse::new("Users")
        .with("users", serde_json::json!([{"id": 1, "name": "Alice"}]))
        .resolve(&req)
        .await
        .unwrap();

    // No manifest is configured in this test, so the version resolves to
    // the documented fallback, not a hardcoded "1.0".
    AssertableInertia::from_response(&resp)
        .component("Users")
        .url("/users")
        .version(MANIFEST_VERSION_FALLBACK)
        .has("users")
        .has("errors")
        .missing("nonexistent")
        .where_("users.0.name", "Alice")
        .count("users", 1);

    let hyper_resp = resp.into_hyper();
    assert_eq!(hyper_resp.status(), 200);

    let content_type = hyper_resp.headers().get("Content-Type").unwrap();
    assert!(
        content_type
            .to_str()
            .unwrap()
            .starts_with("application/json")
    );

    assert_eq!(hyper_resp.headers().get("X-Inertia").unwrap(), "true");
    assert_eq!(hyper_resp.headers().get("Vary").unwrap(), "X-Inertia");
}

#[tokio::test]
async fn partial_reload_with_only_filters_props_correctly() {
    let req = MockReq::new("/users")
        .inertia()
        .header("X-Inertia-Partial-Component", "Users")
        .header("X-Inertia-Partial-Data", "users");

    let resp = InertiaResponse::new("Users")
        .with("auth", serde_json::json!({"id": 1}))
        .with("users", serde_json::json!([]))
        .with("categories", serde_json::json!([]))
        .resolve(&req)
        .await
        .unwrap();

    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();

    let props = page["props"].as_object().unwrap();
    assert!(props.contains_key("users"));
    assert!(!props.contains_key("auth"));
    assert!(!props.contains_key("categories"));
    assert!(props.contains_key("errors")); // always present
}

#[tokio::test]
async fn partial_reload_with_except_excludes_listed_props() {
    let req = MockReq::new("/users")
        .inertia()
        .header("X-Inertia-Partial-Component", "Users")
        .header("X-Inertia-Partial-Except", "auth");

    let resp = InertiaResponse::new("Users")
        .with("auth", serde_json::json!({"id": 1}))
        .with("users", serde_json::json!([]))
        .resolve(&req)
        .await
        .unwrap();

    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();

    let props = page["props"].as_object().unwrap();
    assert!(props.contains_key("users"));
    assert!(!props.contains_key("auth"));
}

#[tokio::test]
async fn partial_reload_except_takes_precedence_over_only() {
    let req = MockReq::new("/users")
        .inertia()
        .header("X-Inertia-Partial-Component", "Users")
        .header("X-Inertia-Partial-Data", "users,auth")
        .header("X-Inertia-Partial-Except", "auth");

    let resp = InertiaResponse::new("Users")
        .with("auth", serde_json::json!({"id": 1}))
        .with("users", serde_json::json!([]))
        .resolve(&req)
        .await
        .unwrap();

    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();

    let props = page["props"].as_object().unwrap();
    assert!(props.contains_key("users"));
    assert!(!props.contains_key("auth"));
}

#[tokio::test]
async fn partial_reload_for_different_component_returns_all_props() {
    // Component mismatch: client says it's on "Posts", server is rendering "Users".
    // The filter is inactive - all props returned.
    let req = MockReq::new("/users")
        .inertia()
        .header("X-Inertia-Partial-Component", "Posts")
        .header("X-Inertia-Partial-Data", "users");

    let resp = InertiaResponse::new("Users")
        .with("auth", serde_json::json!({"id": 1}))
        .with("users", serde_json::json!([]))
        .resolve(&req)
        .await
        .unwrap();

    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();

    let props = page["props"].as_object().unwrap();
    assert!(props.contains_key("users"));
    assert!(props.contains_key("auth"));
}

#[tokio::test]
async fn always_props_bypass_partial_reload_filter() {
    let req = MockReq::new("/users")
        .inertia()
        .header("X-Inertia-Partial-Component", "Users")
        .header("X-Inertia-Partial-Data", "users");

    let resp = InertiaResponse::new("Users")
        .with("users", serde_json::json!([]))
        .always("flash", serde_json::json!({"msg": "saved"}))
        .resolve(&req)
        .await
        .unwrap();

    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();

    let props = page["props"].as_object().unwrap();
    assert!(props.contains_key("users"));
    // `flash` is Always - appears despite not being in partial-data.
    assert!(props.contains_key("flash"));
}

// ---- T26: partial reload dot-notation only/except ------------------------

#[tokio::test]
async fn partial_data_dot_notation_returns_only_the_nested_key() {
    let req = MockReq::new("/users")
        .inertia()
        .header("X-Inertia-Partial-Component", "Users")
        .header("X-Inertia-Partial-Data", "user.name");

    let resp = InertiaResponse::new("Users")
        .with(
            "user",
            serde_json::json!({"name": "Alice", "email": "alice@example.com"}),
        )
        .resolve(&req)
        .await
        .unwrap();

    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert_eq!(page["props"]["user"], serde_json::json!({"name": "Alice"}));
}

#[tokio::test]
async fn partial_except_dot_notation_prunes_a_nested_key_without_only() {
    // No X-Inertia-Partial-Data - this is what `router.reload({ except:
    // [...] })` sends: everything except one nested field, no whitelist.
    let req = MockReq::new("/users")
        .inertia()
        .header("X-Inertia-Partial-Component", "Users")
        .header("X-Inertia-Partial-Except", "user.email");

    let resp = InertiaResponse::new("Users")
        .with(
            "user",
            serde_json::json!({"name": "Alice", "email": "alice@example.com"}),
        )
        .resolve(&req)
        .await
        .unwrap();

    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert_eq!(page["props"]["user"], serde_json::json!({"name": "Alice"}));
}

#[tokio::test]
async fn partial_except_dot_notation_wins_over_only_on_the_same_path() {
    let req = MockReq::new("/users")
        .inertia()
        .header("X-Inertia-Partial-Component", "Users")
        .header("X-Inertia-Partial-Data", "user.email")
        .header("X-Inertia-Partial-Except", "user.email");

    let resp = InertiaResponse::new("Users")
        .with(
            "user",
            serde_json::json!({"name": "Alice", "email": "alice@example.com"}),
        )
        .resolve(&req)
        .await
        .unwrap();

    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();

    // "user" still participates (only named it), but the one path both
    // headers agree on is gone - except wins, leaving an empty object
    // rather than dropping "user" from props altogether.
    let props = page["props"].as_object().unwrap();
    assert!(props.contains_key("user"));
    assert_eq!(page["props"]["user"], serde_json::json!({}));
}

#[tokio::test]
async fn partial_data_unknown_nested_path_yields_nothing_for_that_key_without_dropping_siblings() {
    let req = MockReq::new("/users")
        .inertia()
        .header("X-Inertia-Partial-Component", "Users")
        .header("X-Inertia-Partial-Data", "user.name,user.bogus,user.email");

    let resp = InertiaResponse::new("Users")
        .with(
            "user",
            serde_json::json!({"name": "Alice", "email": "alice@example.com"}),
        )
        .resolve(&req)
        .await
        .unwrap();

    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();

    // "bogus" doesn't exist on `user` - it contributes nothing, but its
    // siblings in the same request ("name", "email") still land.
    assert_eq!(
        page["props"]["user"],
        serde_json::json!({"name": "Alice", "email": "alice@example.com"})
    );
}

#[tokio::test]
async fn partial_data_dotted_path_through_a_scalar_drops_silently() {
    let req = MockReq::new("/settings")
        .inertia()
        .header("X-Inertia-Partial-Component", "Settings")
        .header("X-Inertia-Partial-Data", "config.theme,config.level.nested");

    let resp = InertiaResponse::new("Settings")
        .with("config", serde_json::json!({"theme": "dark", "level": 3}))
        .resolve(&req)
        .await
        .unwrap();

    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();

    // "level" is a scalar, so a path that drills through it
    // ("level.nested") drops silently; the sibling "theme" path still
    // comes through.
    assert_eq!(
        page["props"]["config"],
        serde_json::json!({"theme": "dark"})
    );
}

#[tokio::test]
async fn partial_data_bare_key_wins_over_a_narrower_dotted_entry() {
    let req = MockReq::new("/users")
        .inertia()
        .header("X-Inertia-Partial-Component", "Users")
        .header("X-Inertia-Partial-Data", "user,user.name");

    let resp = InertiaResponse::new("Users")
        .with(
            "user",
            serde_json::json!({"name": "Alice", "email": "alice@example.com"}),
        )
        .resolve(&req)
        .await
        .unwrap();

    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert_eq!(
        page["props"]["user"],
        serde_json::json!({"name": "Alice", "email": "alice@example.com"})
    );
}

#[tokio::test]
async fn always_prop_ignores_dotted_only_and_ships_whole_value() {
    let req = MockReq::new("/users")
        .inertia()
        .header("X-Inertia-Partial-Component", "Users")
        .header("X-Inertia-Partial-Data", "user.name");

    let resp = InertiaResponse::new("Users")
        .always(
            "user",
            serde_json::json!({"name": "Alice", "email": "alice@example.com"}),
        )
        .resolve(&req)
        .await
        .unwrap();

    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();

    // Always bypasses partial-reload filtering entirely, dot notation
    // included - Laravel's `resolveAlways` re-injects the raw, unfiltered
    // value (`inertia-laravel-2.0.25/src/Response.php:406-416`).
    assert_eq!(
        page["props"]["user"],
        serde_json::json!({"name": "Alice", "email": "alice@example.com"})
    );
}

#[tokio::test]
async fn optional_prop_dot_only_resolves_and_narrows() {
    let _guard = suprnova::testing::TestContainer::fake();
    let req = MockReq::new("/team")
        .inertia()
        .header("X-Inertia-Partial-Component", "Team")
        .header("X-Inertia-Partial-Data", "permissions.read");

    let resp = InertiaResponse::new("Team")
        .optional("permissions", || async {
            Ok::<_, suprnova::FrameworkError>(serde_json::json!({"read": true, "write": false}))
        })
        .resolve(&req)
        .await
        .unwrap();

    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert_eq!(
        page["props"]["permissions"],
        serde_json::json!({"read": true})
    );
}

#[tokio::test]
async fn defer_prop_dot_only_on_the_followup_resolves_and_narrows() {
    let _guard = suprnova::testing::TestContainer::fake();
    let req = MockReq::new("/chat")
        .inertia()
        .header("X-Inertia-Partial-Component", "Chat")
        .header("X-Inertia-Partial-Data", "thread.title");

    let resp = InertiaResponse::new("Chat")
        .defer("thread", || async {
            Ok::<_, suprnova::FrameworkError>(serde_json::json!({
                "title": "Hello",
                "messages": [{"id": 1}],
            }))
        })
        .resolve(&req)
        .await
        .unwrap();

    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert_eq!(
        page["props"]["thread"],
        serde_json::json!({"title": "Hello"})
    );
    assert!(!page.as_object().unwrap().contains_key("deferredProps"));
}

#[tokio::test]
async fn merge_prop_dot_only_narrows_the_value_but_merge_metadata_keeps_the_bare_key() {
    let req = MockReq::new("/feed")
        .inertia()
        .header("X-Inertia-Partial-Component", "Feed")
        .header("X-Inertia-Partial-Data", "feed.items");

    let resp = InertiaResponse::new("Feed")
        .merge_with(
            "feed",
            serde_json::json!({"items": [{"id": 1}], "meta": {"total": 1}}),
            suprnova::MergeStrategy::Append { match_on: None },
        )
        .resolve(&req)
        .await
        .unwrap();

    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert_eq!(
        page["props"]["feed"],
        serde_json::json!({"items": [{"id": 1}]})
    );
    assert_eq!(page["mergeProps"], serde_json::json!(["feed"]));
}

#[tokio::test]
async fn html_shell_uses_per_response_title_override() {
    let req = MockReq::new("/home");
    let resp = InertiaResponse::new("Home")
        .title("My Custom Page")
        .resolve(&req)
        .await
        .unwrap();

    let body = body_to_string(resp.into_hyper().into_body());
    assert!(body.contains("<title>My Custom Page</title>"));
    assert!(!body.contains("<title>Suprnova</title>"));
}

#[tokio::test]
async fn html_shell_uses_config_default_title_when_no_override() {
    let cfg = InertiaConfig::new().default_title("Acme App");
    let req = MockReq::new("/home");
    let resp = InertiaResponse::new("Home")
        .with_config(cfg)
        .resolve(&req)
        .await
        .unwrap();

    let body = body_to_string(resp.into_hyper().into_body());
    assert!(body.contains("<title>Acme App</title>"));
}

/// With no locale scope open, the document declares the configured
/// default - `en` in a test process that sets no `APP_LOCALE`. This is
/// also the whole behaviour when the `localization` feature is off, which
/// is why the test is not gated on it.
#[tokio::test]
async fn html_shell_declares_english_outside_any_locale_scope() {
    let req = MockReq::new("/home");
    let resp = InertiaResponse::new("Home").resolve(&req).await.unwrap();

    let body = body_to_string(resp.into_hyper().into_body());
    assert!(
        body.contains("<html lang=\"en\">"),
        "the shell must declare a language; got:\n{body}"
    );
}

/// A reader who switched to Japanese must not be handed a document that
/// declares itself English: a screen reader picks its voice from this
/// attribute and a search engine takes it as the language signal.
#[cfg(feature = "localization")]
#[tokio::test]
async fn html_shell_declares_the_active_locale() {
    let req = MockReq::new("/home");
    let body = suprnova::scope_locale(suprnova::Locale::parse("ja").unwrap(), async {
        let resp = InertiaResponse::new("Home").resolve(&req).await.unwrap();
        body_to_string(resp.into_hyper().into_body())
    })
    .await;

    assert!(
        body.contains("<html lang=\"ja\">"),
        "the shell must follow the locale in effect for the request; got:\n{body}"
    );
    assert!(
        !body.contains("<html lang=\"en\">"),
        "and it must not still claim English; got:\n{body}"
    );
}

/// The attribute carries the BCP 47 form the `Locale` type renders, region
/// subtag and script included. Lowercasing it (`pt-br`) would still parse,
/// but `zh-Hans` and `zh-hant` are the pair that stops matching CSS
/// `:lang()` selectors and font stacks written the conventional way.
#[cfg(feature = "localization")]
#[tokio::test]
async fn html_shell_keeps_the_bcp47_casing_of_the_locale() {
    let req = MockReq::new("/home");
    for locale in ["pt-BR", "zh-Hans"] {
        let body = suprnova::scope_locale(suprnova::Locale::parse(locale).unwrap(), async {
            let resp = InertiaResponse::new("Home").resolve(&req).await.unwrap();
            body_to_string(resp.into_hyper().into_body())
        })
        .await;
        assert!(
            body.contains(&format!("<html lang=\"{locale}\">")),
            "expected lang=\"{locale}\"; got:\n{body}"
        );
    }
}

#[tokio::test]
async fn html_shell_for_react_includes_refresh_preamble() {
    let cfg = InertiaConfig::new().frontend(Frontend::React);
    let req = MockReq::new("/home");
    let resp = InertiaResponse::new("Home")
        .with_config(cfg)
        .resolve(&req)
        .await
        .unwrap();

    let body = body_to_string(resp.into_hyper().into_body());
    assert!(body.contains("@react-refresh"));
    assert!(body.contains("__vite_plugin_react_preamble_installed__"));
    assert!(body.contains("src/main.tsx"));
}

#[tokio::test]
async fn html_shell_for_svelte_omits_react_preamble() {
    let cfg = InertiaConfig::new().frontend(Frontend::Svelte);
    let req = MockReq::new("/home");
    let resp = InertiaResponse::new("Home")
        .with_config(cfg)
        .resolve(&req)
        .await
        .unwrap();

    let body = body_to_string(resp.into_hyper().into_body());
    assert!(!body.contains("@react-refresh"));
    assert!(body.contains("src/main.ts"));
}

#[tokio::test]
async fn html_shell_for_vue_omits_react_preamble() {
    let cfg = InertiaConfig::new().frontend(Frontend::Vue);
    let req = MockReq::new("/home");
    let resp = InertiaResponse::new("Home")
        .with_config(cfg)
        .resolve(&req)
        .await
        .unwrap();

    let body = body_to_string(resp.into_hyper().into_body());
    assert!(!body.contains("@react-refresh"));
    assert!(body.contains("src/main.ts"));
}

#[tokio::test]
async fn production_html_shell_falls_back_to_legacy_paths_when_manifest_missing() {
    // When no manifest.json exists on disk, the framework falls back to the
    // pre-manifest hardcoded `/{assets_base_url}/main.{js,css}` shape so apps
    // produced before D20-B keep booting. A tracing::warn! fires once on
    // first read inside `InertiaConfig::vite_manifest` (not asserted here -
    // requires tracing capture).
    let cfg = InertiaConfig::new()
        .production()
        // Point at a path guaranteed not to exist.
        .manifest_path("/definitely/not/a/real/manifest.json");
    let req = MockReq::new("/home");
    let resp = InertiaResponse::new("Home")
        .with_config(cfg)
        .resolve(&req)
        .await
        .unwrap();

    let body = body_to_string(resp.into_hyper().into_body());
    assert!(body.contains("/assets/main.js"));
    assert!(body.contains("/assets/main.css"));
    // Dev-only Vite scripts should NOT appear in production
    assert!(!body.contains("/@vite/client"));
    assert!(!body.contains("@react-refresh"));
}

#[tokio::test]
async fn production_html_shell_reads_vite_manifest_for_hashed_assets() {
    // D20-B regression: with a real manifest pointing entry `src/main.ts`
    // at hashed output, the prod shell emits the hashed filenames + CSS +
    // modulepreload chunks instead of the legacy `/assets/main.js` path.
    let dir = std::env::temp_dir();
    let manifest_path = dir.join(format!(
        "test-inertia-manifest-{}.json",
        uuid::Uuid::new_v4()
    ));
    let manifest = r#"{
        "src/main.ts": {
            "file": "main-Q9zSqcUL.js",
            "name": "main",
            "src": "src/main.ts",
            "isEntry": true,
            "css": ["main-3R4lN-AT.css"],
            "imports": ["_runtime-DTQbz0Cz.js"]
        },
        "_runtime-DTQbz0Cz.js": {
            "file": "runtime-DTQbz0Cz.js"
        }
    }"#;
    std::fs::write(&manifest_path, manifest).unwrap();

    let cfg = InertiaConfig::new()
        .production()
        .manifest_path(&manifest_path);

    let req = MockReq::new("/home");
    let resp = InertiaResponse::new("Home")
        .with_config(cfg)
        .resolve(&req)
        .await
        .unwrap();

    let body = body_to_string(resp.into_hyper().into_body());
    std::fs::remove_file(&manifest_path).ok();

    // Hashed entry file present
    assert!(
        body.contains("/assets/main-Q9zSqcUL.js"),
        "body should contain hashed entry; got: {body}"
    );
    // Hashed CSS file present
    assert!(
        body.contains("/assets/main-3R4lN-AT.css"),
        "body should contain hashed CSS; got: {body}"
    );
    // Module preload for the imported runtime chunk
    assert!(
        body.contains("modulepreload"),
        "body should contain modulepreload tag"
    );
    assert!(
        body.contains("/assets/runtime-DTQbz0Cz.js"),
        "body should contain preloaded chunk; got: {body}"
    );
    // Legacy hardcoded paths should NOT appear
    assert!(!body.contains("/assets/main.js"));
    assert!(!body.contains("/assets/main.css"));
}

#[tokio::test]
async fn production_html_shell_respects_custom_assets_base_url() {
    // assets_base_url defaults to /assets; users can override (e.g. when
    // serving from /build or a CDN).
    let dir = std::env::temp_dir();
    let manifest_path = dir.join(format!(
        "test-inertia-manifest-{}.json",
        uuid::Uuid::new_v4()
    ));
    let manifest = r#"{
        "src/main.ts": {
            "file": "main-AAA.js",
            "isEntry": true,
            "css": []
        }
    }"#;
    std::fs::write(&manifest_path, manifest).unwrap();

    let cfg = InertiaConfig::new()
        .production()
        .manifest_path(&manifest_path)
        .assets_base_url("/build");

    let req = MockReq::new("/home");
    let resp = InertiaResponse::new("Home")
        .with_config(cfg)
        .resolve(&req)
        .await
        .unwrap();

    let body = body_to_string(resp.into_hyper().into_body());
    std::fs::remove_file(&manifest_path).ok();

    assert!(
        body.contains("/build/main-AAA.js"),
        "custom base URL should prefix asset path; got: {body}"
    );
    assert!(!body.contains("/assets/main"));
}

#[tokio::test]
async fn version_in_page_object_matches_configured_version() {
    let cfg = InertiaConfig::new().version("abc123");
    let req = MockReq::new("/").inertia();
    let resp = InertiaResponse::new("Home")
        .with_config(cfg)
        .resolve(&req)
        .await
        .unwrap();

    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(page["version"], "abc123");
}

#[tokio::test]
async fn errors_prop_is_always_an_empty_object_when_unset() {
    let req = MockReq::new("/").inertia();
    let resp = InertiaResponse::new("Home").resolve(&req).await.unwrap();

    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    let errors = &page["props"]["errors"];
    assert!(errors.is_object());
    assert!(errors.as_object().unwrap().is_empty());
}

#[tokio::test]
async fn props_serialize_in_insertion_order_via_indexmap() {
    // serde_json's preserve_order feature + IndexMap should produce stable,
    // insertion-ordered output. The "errors" key is inserted first by the
    // resolver, then each user-added prop in order.
    let req = MockReq::new("/").inertia();
    let resp = InertiaResponse::new("Test")
        .with("zebra", 1)
        .with("apple", 2)
        .with("mango", 3)
        .resolve(&req)
        .await
        .unwrap();

    let body = body_to_string(resp.into_hyper().into_body());

    let zebra_pos = body.find("zebra").unwrap();
    let apple_pos = body.find("apple").unwrap();
    let mango_pos = body.find("mango").unwrap();
    assert!(zebra_pos < apple_pos);
    assert!(apple_pos < mango_pos);
}

#[tokio::test]
async fn version_conflict_response_carries_x_inertia_location() {
    let resp = InertiaResponse::version_conflict("/new-location");
    let hyper_resp = resp.into_hyper();
    assert_eq!(hyper_resp.status(), 409);
    assert_eq!(
        hyper_resp.headers().get("X-Inertia-Location").unwrap(),
        "/new-location"
    );
}

#[tokio::test]
async fn page_object_url_reflects_request_path() {
    let req = MockReq::new("/users/42/edit").inertia();
    let resp = InertiaResponse::new("Users/Edit")
        .resolve(&req)
        .await
        .unwrap();

    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(page["url"], "/users/42/edit");
}

#[tokio::test]
async fn xhr_response_omits_html_shell_entirely() {
    let req = MockReq::new("/home").inertia();
    let resp = InertiaResponse::new("Home")
        .with("data", "value")
        .resolve(&req)
        .await
        .unwrap();

    let body = body_to_string(resp.into_hyper().into_body());
    // JSON output should NOT contain any of the HTML shell markers.
    assert!(!body.contains("<!DOCTYPE html>"));
    assert!(!body.contains("<html"));
    assert!(!body.contains("data-page="));
}

#[tokio::test]
async fn resolvers_run_concurrently_not_serially() {
    // Three Lazy resolvers each sleep 80ms. Serial would be ~240ms, parallel
    // should be ~80ms. Allow generous headroom on the upper bound to avoid
    // flakiness on a loaded CI runner while still catching serialization.
    let req = MockReq::new("/").inertia();
    let start = std::time::Instant::now();
    let _ = InertiaResponse::new("Home")
        .lazy("a", || async {
            tokio::time::sleep(std::time::Duration::from_millis(80)).await;
            Ok::<_, suprnova::FrameworkError>(serde_json::json!("a"))
        })
        .lazy("b", || async {
            tokio::time::sleep(std::time::Duration::from_millis(80)).await;
            Ok::<_, suprnova::FrameworkError>(serde_json::json!("b"))
        })
        .lazy("c", || async {
            tokio::time::sleep(std::time::Duration::from_millis(80)).await;
            Ok::<_, suprnova::FrameworkError>(serde_json::json!("c"))
        })
        .resolve(&req)
        .await
        .unwrap();
    let elapsed = start.elapsed();

    assert!(
        elapsed < std::time::Duration::from_millis(200),
        "parallel resolution should complete in ~80ms, took {:?}",
        elapsed
    );
    assert!(
        elapsed >= std::time::Duration::from_millis(80),
        "should still take at least one resolver's duration, took {:?}",
        elapsed
    );
}

#[tokio::test]
async fn html_shell_csrf_meta_tag_present_even_when_session_unset() {
    // No session => csrf_token() returns None => empty content. The
    // tag still needs to render so the frontend can read it.
    let req = MockReq::new("/");
    let resp = InertiaResponse::new("Home").resolve(&req).await.unwrap();

    let body = body_to_string(resp.into_hyper().into_body());
    assert!(body.contains(r#"<meta name="csrf-token" content="""#));
}

// ---- Tier 1: shared data, Lazy/Optional, version middleware ----

#[tokio::test]
async fn static_share_appears_in_every_inertia_response() {
    let _guard = suprnova::testing::TestContainer::fake();

    suprnova::App::inertia_share("appName", "Suprnova");

    let req = MockReq::new("/").inertia();
    let resp = InertiaResponse::new("Home").resolve(&req).await.unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert_eq!(page["props"]["appName"], "Suprnova");
}

#[tokio::test]
async fn user_props_override_static_shared_data() {
    let _guard = suprnova::testing::TestContainer::fake();

    suprnova::App::inertia_share("title", "Shared Title");

    let req = MockReq::new("/").inertia();
    let resp = InertiaResponse::new("Home")
        .with("title", "Page Title")
        .resolve(&req)
        .await
        .unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();

    // Per the precedence chain (static → trait → user), user wins on dups.
    assert_eq!(page["props"]["title"], "Page Title");
}

#[tokio::test]
async fn shared_props_field_lists_registry_keys() {
    let _guard = suprnova::testing::TestContainer::fake();

    suprnova::App::inertia_share("appName", "Suprnova");
    suprnova::App::inertia_share("apiHost", "api.example.com");

    let req = MockReq::new("/").inertia();
    let resp = InertiaResponse::new("Home")
        .with("page", "home")
        .resolve(&req)
        .await
        .unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();

    let shared = page["sharedProps"]
        .as_array()
        .expect("sharedProps should be an array");
    let names: Vec<&str> = shared.iter().filter_map(|v| v.as_str()).collect();
    assert!(names.contains(&"appName"));
    assert!(names.contains(&"apiHost"));
    // User-only `page` key must NOT be advertised as shared.
    assert!(!names.contains(&"page"));
}

#[tokio::test]
async fn shared_props_field_omitted_when_registry_empty() {
    let _guard = suprnova::testing::TestContainer::fake();

    let req = MockReq::new("/").inertia();
    let resp = InertiaResponse::new("Home")
        .with("page", "home")
        .resolve(&req)
        .await
        .unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        !page.as_object().unwrap().contains_key("sharedProps"),
        "sharedProps must be omitted when no shared registry entries exist"
    );
}

#[tokio::test]
async fn shared_props_includes_key_even_when_user_overrides() {
    // Per the Inertia v3 client contract, sharedProps is just a key
    // list - the client reads values from `props`. Overriding a
    // shared key with `.with()` doesn't remove the key from
    // sharedProps; the override wins in `props` and that's what the
    // client sees. Verifies the override-still-in-sharedProps contract.
    let _guard = suprnova::testing::TestContainer::fake();

    suprnova::App::inertia_share("title", "Shared Title");

    let req = MockReq::new("/").inertia();
    let resp = InertiaResponse::new("Home")
        .with("title", "Page Title")
        .resolve(&req)
        .await
        .unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert_eq!(page["props"]["title"], "Page Title");
    let shared = page["sharedProps"].as_array().unwrap();
    let names: Vec<&str> = shared.iter().filter_map(|v| v.as_str()).collect();
    assert!(
        names.contains(&"title"),
        "shared key should remain in sharedProps even when overridden"
    );
}

#[tokio::test]
async fn lazy_shared_resolves_only_when_partial_includes_key() {
    let _guard = suprnova::testing::TestContainer::fake();

    let call_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = call_count.clone();

    // Unique key so we don't collide with other concurrent tests that
    // might have left state in the static registry despite SHARED_LOCK
    // (e.g. across cargo's parallel test binaries - unlikely but cheap
    // to guard against).
    let key = "expensive_lazy_test";

    suprnova::App::inertia_share_lazy(key, move || {
        let c = counter.clone();
        async move {
            c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok::<_, suprnova::FrameworkError>(serde_json::json!({"computed": true}))
        }
    });

    // Standard visit - resolver should run (Lazy is included on standard visits).
    let req = MockReq::new("/").inertia();
    let _ = InertiaResponse::new("Home").resolve(&req).await.unwrap();
    let after_step_1 = call_count.load(std::sync::atomic::Ordering::SeqCst);
    assert_eq!(after_step_1, 1, "standard visit should resolve lazy once");

    // Partial reload excluding the key - resolver should NOT run.
    let req = MockReq::new("/")
        .inertia()
        .header("X-Inertia-Partial-Component", "Home")
        .header("X-Inertia-Partial-Data", "other_key");
    let _ = InertiaResponse::new("Home").resolve(&req).await.unwrap();
    let after_step_2 = call_count.load(std::sync::atomic::Ordering::SeqCst);
    assert_eq!(
        after_step_2, 1,
        "partial reload excluding the key must not invoke the resolver"
    );

    // Partial reload that explicitly requests the key - resolver runs.
    let req = MockReq::new("/")
        .inertia()
        .header("X-Inertia-Partial-Component", "Home")
        .header("X-Inertia-Partial-Data", key);
    let _ = InertiaResponse::new("Home").resolve(&req).await.unwrap();
    let after_step_3 = call_count.load(std::sync::atomic::Ordering::SeqCst);
    assert_eq!(
        after_step_3, 2,
        "explicit partial-data request must invoke the resolver"
    );
}

#[tokio::test]
async fn trait_provider_runs_with_request_context() {
    let _guard = suprnova::testing::TestContainer::fake();

    struct AuthProvider;
    #[async_trait::async_trait]
    impl suprnova::inertia::InertiaSharedData for AuthProvider {
        async fn share(
            &self,
            req: &dyn suprnova::InertiaRequestExt,
            _component: &str,
        ) -> Result<indexmap::IndexMap<String, suprnova::Prop>, suprnova::FrameworkError> {
            let mut m = indexmap::IndexMap::new();
            // Per-request data: read a header to derive the prop.
            let auth_header = req.header("X-Auth-User").unwrap_or("anonymous");
            m.insert(
                "auth".to_string(),
                suprnova::Prop::eager(serde_json::json!({ "user": auth_header })),
            );
            Ok(m)
        }
    }

    suprnova::App::register_inertia_shared(std::sync::Arc::new(AuthProvider));

    let req = MockReq::new("/").inertia().header("X-Auth-User", "alice");
    let resp = InertiaResponse::new("Home").resolve(&req).await.unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(page["props"]["auth"]["user"], "alice");

    // Different request → different per-request data.
    let req2 = MockReq::new("/").inertia().header("X-Auth-User", "bob");
    let resp2 = InertiaResponse::new("Home").resolve(&req2).await.unwrap();
    let body2 = body_to_string(resp2.into_hyper().into_body());
    let page2: serde_json::Value = serde_json::from_str(&body2).unwrap();
    assert_eq!(page2["props"]["auth"]["user"], "bob");
}

#[tokio::test]
async fn trait_share_overrides_static_share_but_user_overrides_both() {
    let _guard = suprnova::testing::TestContainer::fake();

    suprnova::App::inertia_share("layer", "static");

    struct Trait;
    #[async_trait::async_trait]
    impl suprnova::inertia::InertiaSharedData for Trait {
        async fn share(
            &self,
            _req: &dyn suprnova::InertiaRequestExt,
            _component: &str,
        ) -> Result<indexmap::IndexMap<String, suprnova::Prop>, suprnova::FrameworkError> {
            let mut m = indexmap::IndexMap::new();
            m.insert(
                "layer".to_string(),
                suprnova::Prop::eager(serde_json::Value::String("trait".into())),
            );
            Ok(m)
        }
    }
    suprnova::App::register_inertia_shared(std::sync::Arc::new(Trait));

    // No user override - trait wins over static.
    let req = MockReq::new("/").inertia();
    let resp = InertiaResponse::new("Home").resolve(&req).await.unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(page["props"]["layer"], "trait");

    // User override - user wins.
    let resp = InertiaResponse::new("Home")
        .with("layer", "user")
        .resolve(&req)
        .await
        .unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(page["props"]["layer"], "user");
}

#[tokio::test]
async fn lazy_user_prop_resolves_only_when_requested_in_partial_reload() {
    // Acquire SHARED_LOCK even though this test doesn't touch the static
    // registry: it calls `resolve()`, which reads the global registry,
    // so if another test in this binary has shared data registered we
    // don't want it to leak into ours.
    let _guard = suprnova::testing::TestContainer::fake();
    let req = MockReq::new("/posts")
        .inertia()
        .header("X-Inertia-Partial-Component", "Posts")
        .header("X-Inertia-Partial-Data", "users");

    let call_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = call_count.clone();

    let resp = InertiaResponse::new("Posts")
        .with("users", serde_json::json!([]))
        .lazy("posts", move || {
            let c = counter.clone();
            async move {
                c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok::<_, suprnova::FrameworkError>(serde_json::json!([{"id": 1}]))
            }
        })
        .resolve(&req)
        .await
        .unwrap();

    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();

    // `posts` not in partial-data → resolver not invoked, key absent.
    assert_eq!(call_count.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert!(!page["props"].as_object().unwrap().contains_key("posts"));
    assert!(page["props"].as_object().unwrap().contains_key("users"));
}

#[tokio::test]
async fn optional_prop_excluded_on_standard_visit() {
    let _guard = suprnova::testing::TestContainer::fake();
    let req = MockReq::new("/").inertia();

    let call_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = call_count.clone();

    let resp = InertiaResponse::new("Home")
        .optional("permissions", move || {
            let c = counter.clone();
            async move {
                c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok::<_, suprnova::FrameworkError>(serde_json::json!(["read", "write"]))
            }
        })
        .resolve(&req)
        .await
        .unwrap();

    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();

    // Standard visit → optional NOT included AND NOT resolved.
    assert_eq!(call_count.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert!(
        !page["props"]
            .as_object()
            .unwrap()
            .contains_key("permissions")
    );
}

#[tokio::test]
async fn optional_prop_included_when_explicitly_requested() {
    let _guard = suprnova::testing::TestContainer::fake();
    let req = MockReq::new("/")
        .inertia()
        .header("X-Inertia-Partial-Component", "Home")
        .header("X-Inertia-Partial-Data", "permissions");

    let resp = InertiaResponse::new("Home")
        .optional("permissions", || async {
            Ok::<_, suprnova::FrameworkError>(serde_json::json!(["read"]))
        })
        .resolve(&req)
        .await
        .unwrap();

    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(page["props"]["permissions"], serde_json::json!(["read"]));
}

#[tokio::test]
async fn lazy_resolver_error_propagates_as_framework_error() {
    let _guard = suprnova::testing::TestContainer::fake();
    let req = MockReq::new("/").inertia();

    let result = InertiaResponse::new("Home")
        .lazy("boom", || async {
            Err::<serde_json::Value, _>(suprnova::FrameworkError::internal("kaboom"))
        })
        .resolve(&req)
        .await;

    match result {
        Err(e) => assert!(e.to_string().contains("kaboom")),
        Ok(_) => panic!("expected resolver error to propagate"),
    }
}

// ---- Tier 2: flash, deferred, merge, once ----

#[tokio::test]
async fn flash_via_response_builder_emits_top_level_field() {
    let req = MockReq::new("/").inertia();
    let resp = InertiaResponse::new("Home")
        .flash("toast", serde_json::json!({"msg": "saved"}))
        .resolve(&req)
        .await
        .unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(page["flash"]["toast"]["msg"], "saved");
    // Not under props.
    assert!(!page["props"].as_object().unwrap().contains_key("flash"));
}

#[tokio::test]
async fn flash_field_absent_when_no_data() {
    let req = MockReq::new("/").inertia();
    let resp = InertiaResponse::new("Home").resolve(&req).await.unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(!page.as_object().unwrap().contains_key("flash"));
}

#[tokio::test]
async fn defer_on_initial_visit_is_in_deferred_props_not_props() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let call_count = Arc::new(AtomicUsize::new(0));

    let req = MockReq::new("/").inertia();
    let counter = call_count.clone();
    let resp = InertiaResponse::new("Users")
        .defer("permissions", move || {
            let c = counter.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok::<_, suprnova::FrameworkError>(serde_json::json!(["read", "write"]))
            }
        })
        .resolve(&req)
        .await
        .unwrap();

    // Resolver not called on the initial visit; the key is reported under
    // deferredProps, not props.
    assert_eq!(call_count.load(Ordering::SeqCst), 0);

    let counter_for_reload = call_count.clone();
    let assertable =
        AssertableInertia::from_response(&resp).with_reload(move |reload: ReloadRequest| {
            let counter = counter_for_reload.clone();
            async move {
                let mut req = MockReq::new("/").inertia();
                for (name, value) in reload.headers() {
                    req = req.header(&name, &value);
                }
                let resp = InertiaResponse::new("Users")
                    .defer("permissions", move || {
                        let c = counter.clone();
                        async move {
                            c.fetch_add(1, Ordering::SeqCst);
                            Ok::<_, suprnova::FrameworkError>(serde_json::json!(["read", "write"]))
                        }
                    })
                    .resolve(&req)
                    .await
                    .unwrap();
                AssertableInertia::from_response(&resp)
            }
        });

    assertable.missing("permissions");

    let reloaded = assertable.load_deferred_props().await;

    assert_eq!(
        call_count.load(Ordering::SeqCst),
        1,
        "resolver runs exactly once, on the reload"
    );
    reloaded
        .component("Users")
        .has("permissions")
        .where_("permissions", serde_json::json!(["read", "write"]));
}

#[tokio::test]
async fn defer_partial_reload_invokes_resolver_and_lands_in_props() {
    let req = MockReq::new("/")
        .inertia()
        .header("X-Inertia-Partial-Component", "Users")
        .header("X-Inertia-Partial-Data", "permissions");

    let call_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = call_count.clone();

    let resp = InertiaResponse::new("Users")
        .defer("permissions", move || {
            let c = counter.clone();
            async move {
                c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok::<_, suprnova::FrameworkError>(serde_json::json!(["read"]))
            }
        })
        .resolve(&req)
        .await
        .unwrap();

    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert_eq!(call_count.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(page["props"]["permissions"], serde_json::json!(["read"]));
    // No deferredProps emitted for the resolved key.
    assert!(!page.as_object().unwrap().contains_key("deferredProps"));
}

#[tokio::test]
async fn defer_grouping_buckets_keys() {
    let req = MockReq::new("/").inertia();

    let resp = InertiaResponse::new("Posts")
        .defer_with(
            "teams",
            suprnova::DeferOptions::new().group("attributes"),
            || async { Ok::<_, suprnova::FrameworkError>(serde_json::json!([])) },
        )
        .defer_with(
            "projects",
            suprnova::DeferOptions::new().group("attributes"),
            || async { Ok::<_, suprnova::FrameworkError>(serde_json::json!([])) },
        )
        .defer("permissions", || async {
            Ok::<_, suprnova::FrameworkError>(serde_json::json!([]))
        })
        .resolve(&req)
        .await
        .unwrap();

    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();

    let deferred = page["deferredProps"].as_object().unwrap();
    assert_eq!(
        deferred["attributes"].as_array().unwrap(),
        &vec![serde_json::json!("teams"), serde_json::json!("projects")]
    );
    assert_eq!(
        deferred["default"].as_array().unwrap(),
        &vec![serde_json::json!("permissions")]
    );
}

#[tokio::test]
async fn defer_rescue_catches_resolver_error() {
    let req = MockReq::new("/")
        .inertia()
        .header("X-Inertia-Partial-Component", "Users")
        .header("X-Inertia-Partial-Data", "permissions");

    let resp = InertiaResponse::new("Users")
        .defer_with(
            "permissions",
            suprnova::DeferOptions::new().rescue(),
            || async { Err::<serde_json::Value, _>(suprnova::FrameworkError::internal("boom")) },
        )
        .resolve(&req)
        .await
        .unwrap();

    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();

    // Prop omitted from props
    assert!(
        !page["props"]
            .as_object()
            .unwrap()
            .contains_key("permissions")
    );
    // But listed in rescuedProps
    assert_eq!(page["rescuedProps"], serde_json::json!(["permissions"]));
}

/// The ErrorOccurred event must be dispatched on the rescue path so observability
/// listeners (Sentry, Pagerduty, custom shippers) see the rescued resolver error.
/// The dispatch is **spawned**, not awaited - mirroring the http/response.rs
/// pattern and the documented `events::builtins::ErrorOccurred` best-effort
/// contract - so the Inertia partial-response collector never blocks on listener
/// execution.
#[tokio::test]
async fn defer_rescue_dispatches_error_occurred_event() {
    let _guard = suprnova::EventFacade::fake();
    let req = MockReq::new("/")
        .inertia()
        .header("X-Inertia-Partial-Component", "Users")
        .header("X-Inertia-Partial-Data", "permissions");

    let resp = InertiaResponse::new("Users")
        .defer_with(
            "permissions",
            suprnova::DeferOptions::new().rescue(),
            || async {
                Err::<serde_json::Value, _>(suprnova::FrameworkError::internal("rescued failure"))
            },
        )
        .resolve(&req)
        .await
        .unwrap();

    // The response itself still resolves cleanly with the rescued marker -
    // proving the inline path returned promptly and did not hang on the
    // spawned event dispatch.
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(page["rescuedProps"], serde_json::json!(["permissions"]));

    // The spawn happens on the current Tokio runtime - yield + a short
    // sleep so the spawned dispatcher task lands its record. Mirrors the
    // pattern in tests/events.rs::server_error_dispatches_error_occurred.
    tokio::task::yield_now().await;
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;

    suprnova::events::testing::assert_dispatched::<suprnova::ErrorOccurred>(|e| {
        e.status_code == 500 && e.error_message.contains("rescued failure")
    });
}

#[tokio::test]
async fn defer_without_rescue_propagates_error() {
    let req = MockReq::new("/")
        .inertia()
        .header("X-Inertia-Partial-Component", "Users")
        .header("X-Inertia-Partial-Data", "permissions");

    let result = InertiaResponse::new("Users")
        .defer("permissions", || async {
            Err::<serde_json::Value, _>(suprnova::FrameworkError::internal("kaboom"))
        })
        .resolve(&req)
        .await;

    match result {
        Err(e) => assert!(e.to_string().contains("kaboom")),
        Ok(_) => panic!("expected error to propagate"),
    }
}

#[tokio::test]
async fn merge_emits_merge_props_and_includes_value() {
    let req = MockReq::new("/").inertia();
    let resp = InertiaResponse::new("Tags")
        .merge("tags", serde_json::json!(["rust", "web"]))
        .resolve(&req)
        .await
        .unwrap();

    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(page["props"]["tags"], serde_json::json!(["rust", "web"]));
    assert_eq!(page["mergeProps"], serde_json::json!(["tags"]));
}

#[tokio::test]
async fn merge_prepend_emits_prepend_props() {
    let req = MockReq::new("/").inertia();
    let resp = InertiaResponse::new("Feed")
        .merge_prepend("notifications", serde_json::json!([]))
        .resolve(&req)
        .await
        .unwrap();

    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(page["prependProps"], serde_json::json!(["notifications"]));
    assert!(!page.as_object().unwrap().contains_key("mergeProps"));
}

#[tokio::test]
async fn deep_merge_emits_deep_merge_props() {
    let req = MockReq::new("/").inertia();
    let resp = InertiaResponse::new("Chat")
        .deep_merge("chat", serde_json::json!({"messages": []}))
        .resolve(&req)
        .await
        .unwrap();

    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(page["deepMergeProps"], serde_json::json!(["chat"]));
}

#[tokio::test]
async fn merge_with_match_on_emits_dotted_path_in_match_props_on() {
    let req = MockReq::new("/").inertia();
    let resp = InertiaResponse::new("Posts")
        .merge_with(
            "posts",
            serde_json::json!([{"id": 1}]),
            suprnova::MergeStrategy::Append {
                match_on: Some(vec!["id".into()]),
            },
        )
        .resolve(&req)
        .await
        .unwrap();

    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(page["mergeProps"], serde_json::json!(["posts"]));
    assert_eq!(page["matchPropsOn"], serde_json::json!(["posts.id"]));
}

#[tokio::test]
async fn once_first_visit_resolves_and_emits_metadata() {
    let req = MockReq::new("/").inertia();
    let call_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = call_count.clone();

    let resp = InertiaResponse::new("Billing")
        .once("plans", move || {
            let c = counter.clone();
            async move {
                c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok::<_, suprnova::FrameworkError>(serde_json::json!([{"id": 1, "name": "Basic"}]))
            }
        })
        .resolve(&req)
        .await
        .unwrap();

    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert_eq!(call_count.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(page["props"]["plans"].is_array());
    let once = page["onceProps"].as_object().unwrap();
    let entry = once["plans"].as_object().unwrap();
    assert_eq!(entry["prop"], "plans");
    assert!(entry["expiresAt"].is_null());
}

#[tokio::test]
async fn once_second_visit_skips_resolver_via_except_header() {
    let req = MockReq::new("/")
        .inertia()
        .header("X-Inertia-Except-Once-Props", "plans");

    let call_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = call_count.clone();

    let resp = InertiaResponse::new("Billing")
        .once("plans", move || {
            let c = counter.clone();
            async move {
                c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok::<_, suprnova::FrameworkError>(serde_json::json!([]))
            }
        })
        .resolve(&req)
        .await
        .unwrap();

    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();

    // Resolver skipped - client claims to have it cached.
    assert_eq!(call_count.load(std::sync::atomic::Ordering::SeqCst), 0);
    // Value NOT in props.
    assert!(!page["props"].as_object().unwrap().contains_key("plans"));
    // But metadata still emitted.
    assert!(page["onceProps"]["plans"].is_object());
}

#[tokio::test]
async fn once_with_fresh_ignores_except_header() {
    let req = MockReq::new("/")
        .inertia()
        .header("X-Inertia-Except-Once-Props", "plans");

    let call_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = call_count.clone();

    let resp = InertiaResponse::new("Billing")
        .once_with("plans", suprnova::OnceOptions::new().fresh(), move || {
            let c = counter.clone();
            async move {
                c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok::<_, suprnova::FrameworkError>(serde_json::json!([{"id": 99}]))
            }
        })
        .resolve(&req)
        .await
        .unwrap();

    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();

    // fresh() forces resolver to run despite the except-header.
    assert_eq!(call_count.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(page["props"]["plans"], serde_json::json!([{"id": 99}]));
}

#[tokio::test]
async fn once_with_as_key_uses_custom_cache_key() {
    let req = MockReq::new("/").inertia();
    let resp = InertiaResponse::new("Team")
        .once_with(
            "memberRoles",
            suprnova::OnceOptions::new().as_key("roles"),
            || async { Ok::<_, suprnova::FrameworkError>(serde_json::json!(["admin", "member"])) },
        )
        .resolve(&req)
        .await
        .unwrap();

    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();

    // Prop name is "memberRoles", cache key is "roles".
    assert!(page["props"]["memberRoles"].is_array());
    let once = page["onceProps"].as_object().unwrap();
    assert!(once.contains_key("roles"));
    let entry = once["roles"].as_object().unwrap();
    assert_eq!(entry["prop"], "memberRoles");
}

#[tokio::test]
async fn once_with_until_emits_expires_at() {
    let req = MockReq::new("/").inertia();
    let resp = InertiaResponse::new("Dashboard")
        .once_with(
            "rates",
            suprnova::OnceOptions::new().until(1_700_000_000_000),
            || async { Ok::<_, suprnova::FrameworkError>(serde_json::json!({})) },
        )
        .resolve(&req)
        .await
        .unwrap();

    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();

    let entry = page["onceProps"]["rates"].as_object().unwrap();
    assert_eq!(entry["expiresAt"], serde_json::json!(1_700_000_000_000_i64));
}

#[tokio::test]
async fn once_with_expired_until_forces_resolver_despite_client_cache_header() {
    // D20-C regression - ChatGPT MODULE_REVIEW_NOTES ## inertia HIGH #2.
    //
    // The client sends `X-Inertia-Except-Once-Props: rates` claiming it
    // has `rates` cached. Without server-side expiry enforcement the
    // resolver would silently skip and the client would keep its stale
    // value forever. With D20-C the server checks `expires_at` against
    // wall-clock time and forces the resolver to run when expired.
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let resolver_ran = Arc::new(AtomicBool::new(false));
    let flag = resolver_ran.clone();

    // Expiry in the past: epoch + 1ms (Jan 1 1970 + 1ms). Now() is
    // long past this, so the cache is server-expired.
    let past_expires_ms: i64 = 1;

    let req = MockReq::new("/")
        .inertia()
        .header("X-Inertia-Except-Once-Props", "rates");

    let resp = InertiaResponse::new("Dashboard")
        .once_with(
            "rates",
            suprnova::OnceOptions::new().until(past_expires_ms),
            move || {
                let flag = flag.clone();
                async move {
                    flag.store(true, Ordering::SeqCst);
                    Ok::<_, suprnova::FrameworkError>(serde_json::json!([1, 2, 3]))
                }
            },
        )
        .resolve(&req)
        .await
        .unwrap();

    // Resolver MUST have run because the server-side expiry has passed.
    assert!(
        resolver_ran.load(Ordering::SeqCst),
        "expired once-prop resolver must run despite client cache header"
    );

    // And the freshly-resolved value must be on the page under `props.rates`.
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(page["props"]["rates"], serde_json::json!([1, 2, 3]));
}

#[tokio::test]
async fn dev_head_html_escapes_vite_dev_server_and_entry_point() {
    // D20-G regression - ChatGPT MODULE_REVIEW_NOTES ## inertia LOW
    // #1. Dev-server URLs are normally trusted config values, but a
    // misconfigured env / dotfile shouldn't be able to inject markup
    // into the dev HTML shell.
    let cfg = InertiaConfig::new()
        // Inject an attribute-breaking sequence via vite_dev_server.
        .vite_dev_server("http://evil.test\"><script>alert(1)</script>")
        .entry_point("src/main.ts\"><img src=x>");

    let req = MockReq::new("/");
    let resp = InertiaResponse::new("Home")
        .with_config(cfg)
        .resolve(&req)
        .await
        .unwrap();

    let body = body_to_string(resp.into_hyper().into_body());
    // The raw injection sequence must NOT appear unescaped.
    assert!(
        !body.contains("\"><script>alert(1)</script>"),
        "dev shell must HTML-attr-escape vite_dev_server; body contains raw \
         attribute-breaking sequence"
    );
    assert!(
        !body.contains("\"><img src=x>"),
        "dev shell must HTML-attr-escape entry_point; body contains raw \
         attribute-breaking sequence"
    );
    // Escaped form (the &quot; sequence) must appear.
    assert!(
        body.contains("&quot;"),
        "dev shell must emit &quot; for embedded double quote in config"
    );
}

#[tokio::test]
async fn lazy_resolver_fanout_is_bounded_by_max_concurrent_resolvers() {
    // D20-E regression - ChatGPT MODULE_REVIEW_NOTES ## inertia MEDIUM
    // #4. Without a cap a page with many lazy props would fire all
    // resolvers in parallel via `try_join_all`. Now the response
    // pipeline routes them through `stream.buffered(N)` so at most N
    // resolvers run concurrently.
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let in_flight = Arc::new(AtomicUsize::new(0));
    let high_water = Arc::new(AtomicUsize::new(0));

    let cfg = InertiaConfig::new().max_concurrent_resolvers(3);
    let req = MockReq::new("/").inertia();

    let mut resp = InertiaResponse::new("Dashboard").with_config(cfg);
    for i in 0..20 {
        let flight = in_flight.clone();
        let high = high_water.clone();
        let key = format!("k{i}");
        resp = resp.lazy(key, move || {
            let flight = flight.clone();
            let high = high.clone();
            async move {
                let now = flight.fetch_add(1, Ordering::SeqCst) + 1;
                // Track the highest observed concurrent count.
                high.fetch_max(now, Ordering::SeqCst);
                // Hold the resolver "in flight" briefly so multiple
                // resolvers contend.
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                flight.fetch_sub(1, Ordering::SeqCst);
                Ok::<_, suprnova::FrameworkError>(serde_json::json!(i))
            }
        });
    }

    let _resp = resp.resolve(&req).await.unwrap();

    let peak = high_water.load(Ordering::SeqCst);
    assert!(
        peak <= 3,
        "max concurrent resolvers should not exceed cap of 3; observed {peak}"
    );
    assert!(
        peak >= 2,
        "with 20 resolvers and a 3-cap we should observe at least some concurrency; \
         observed {peak} - buffered(N) may have degenerated to serial"
    );
}

#[tokio::test]
async fn ssr_response_body_cap_falls_back_to_csr_when_exceeded() {
    // D20-D regression - ChatGPT MODULE_REVIEW_NOTES ## inertia MEDIUM
    // #3. The SSR client now reads through http_body_util::Limited,
    // capping the worker's response body at `max_response_bytes`. When
    // exceeded, render() either falls back to CSR (default) or
    // surfaces a 500 (throw_on_error=true).
    //
    // We spin a tiny TCP listener that responds with an oversized
    // body for one POST, then verify the framework falls back to
    // CSR cleanly.
    use std::net::SocketAddr;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let local: SocketAddr = listener.local_addr().unwrap();

    // Server task: accept one connection, drain request, write an
    // oversized response.
    let server = tokio::spawn(async move {
        if let Ok((mut sock, _)) = listener.accept().await {
            // Drain request just enough to unblock the client.
            let mut buf = [0u8; 4096];
            let _ = sock.read(&mut buf).await;
            // Write a 1 MiB response - well above our 64 KiB cap.
            let body = vec![b'A'; 1_000_000];
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                 Content-Length: {}\r\n\r\n",
                body.len()
            );
            let _ = sock.write_all(header.as_bytes()).await;
            let _ = sock.write_all(&body).await;
        }
    });

    let cfg = InertiaConfig::new()
        .ssr(format!("http://{local}"))
        .ssr_max_response_bytes(64 * 1024)
        // Don't throw - exercise the fallback path explicitly.
        .ssr_throw_on_error(false);

    let req = MockReq::new("/dashboard");
    let resp = InertiaResponse::new("Dashboard")
        .with_config(cfg)
        .resolve(&req)
        .await
        .unwrap();

    server.await.ok();

    // CSR fallback - no `data-server-rendered="true"` attribute.
    let body = body_to_string(resp.into_hyper().into_body());
    assert!(
        !body.contains("data-server-rendered=\"true\""),
        "oversized SSR response should NOT inject server-rendered marker; \
         body must show CSR fallback. body excerpt: {}",
        &body[..body.len().min(300)]
    );
}

#[tokio::test]
async fn once_with_future_until_honours_client_cache_header() {
    // Inverse of the D20-C regression: when expiry is in the future
    // the client's cache header is honoured (no resolver call, only
    // metadata emitted). This preserves the optimisation when no
    // expiry is breached.
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let resolver_ran = Arc::new(AtomicBool::new(false));
    let flag = resolver_ran.clone();

    // Far-future expiry: year 2099.
    let future_expires_ms: i64 = 4_070_908_800_000;

    let req = MockReq::new("/")
        .inertia()
        .header("X-Inertia-Except-Once-Props", "rates");

    let _resp = InertiaResponse::new("Dashboard")
        .once_with(
            "rates",
            suprnova::OnceOptions::new().until(future_expires_ms),
            move || {
                let flag = flag.clone();
                async move {
                    flag.store(true, Ordering::SeqCst);
                    Ok::<_, suprnova::FrameworkError>(serde_json::json!([1, 2, 3]))
                }
            },
        )
        .resolve(&req)
        .await
        .unwrap();

    assert!(
        !resolver_ran.load(Ordering::SeqCst),
        "unexpired once-prop resolver must NOT run when client claims cache"
    );
}

#[tokio::test]
async fn app_flash_persists_to_response_via_task_local() {
    let _guard = suprnova::testing::TestContainer::fake();
    let req = MockReq::new("/").inertia();

    // Set up a fresh flash scope using the same pattern the server uses.
    let bag = suprnova::inertia::flash_new_bag_for_test();
    suprnova::inertia::flash_scope_for_test(bag, async move {
        suprnova::App::flash("toast", serde_json::json!({"msg": "via App::flash"}));

        let resp = InertiaResponse::new("Home").resolve(&req).await.unwrap();
        let body = body_to_string(resp.into_hyper().into_body());
        let page: serde_json::Value = serde_json::from_str(&body).unwrap();

        assert_eq!(page["flash"]["toast"]["msg"], "via App::flash");
    })
    .await;
}

#[tokio::test]
async fn share_once_via_app_registers_once_prop() {
    let _guard = suprnova::testing::TestContainer::fake();

    suprnova::App::inertia_share_once("countries", || async {
        Ok::<_, suprnova::FrameworkError>(serde_json::json!(["US", "CA"]))
    });

    let req = MockReq::new("/").inertia();
    let resp = InertiaResponse::new("Home").resolve(&req).await.unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert_eq!(page["props"]["countries"], serde_json::json!(["US", "CA"]));
    assert!(page["onceProps"]["countries"].is_object());
}

// ---- Tier 3: history encryption, location, 303 middleware ----

#[tokio::test]
async fn encrypt_history_per_response_emits_flag() {
    let req = MockReq::new("/").inertia();
    let resp = InertiaResponse::new("Home")
        .encrypt_history(true)
        .resolve(&req)
        .await
        .unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(page["encryptHistory"], true);
}

#[tokio::test]
async fn encrypt_history_omitted_when_false_or_unset() {
    let req = MockReq::new("/").inertia();
    let resp = InertiaResponse::new("Home").resolve(&req).await.unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(!page.as_object().unwrap().contains_key("encryptHistory"));

    let req = MockReq::new("/").inertia();
    let resp = InertiaResponse::new("Home")
        .encrypt_history(false)
        .resolve(&req)
        .await
        .unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(!page.as_object().unwrap().contains_key("encryptHistory"));
}

#[tokio::test]
async fn clear_history_emits_flag_only_when_set() {
    let req = MockReq::new("/").inertia();
    let resp = InertiaResponse::new("Home")
        .clear_history()
        .resolve(&req)
        .await
        .unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(page["clearHistory"], true);

    let req = MockReq::new("/").inertia();
    let resp = InertiaResponse::new("Home").resolve(&req).await.unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(!page.as_object().unwrap().contains_key("clearHistory"));
}

#[tokio::test]
async fn encrypt_history_per_response_overrides_config_default() {
    let cfg = suprnova::InertiaConfig::new().encrypt_history(true);
    let req = MockReq::new("/").inertia();
    let resp = InertiaResponse::new("Home")
        .with_config(cfg)
        .encrypt_history(false)
        .resolve(&req)
        .await
        .unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    // Per-response false beats config-default true.
    assert!(!page.as_object().unwrap().contains_key("encryptHistory"));
}

#[tokio::test]
async fn encrypt_history_config_default_applies_when_no_override() {
    let cfg = suprnova::InertiaConfig::new().encrypt_history(true);
    let req = MockReq::new("/").inertia();
    let resp = InertiaResponse::new("Home")
        .with_config(cfg)
        .resolve(&req)
        .await
        .unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(page["encryptHistory"], true);
}

#[tokio::test]
async fn inertia_location_returns_409_with_x_inertia_location() {
    let resp = InertiaResponse::location("https://example.com/external");
    let hyper_resp = resp.into_hyper();
    assert_eq!(hyper_resp.status(), 409);
    assert_eq!(
        hyper_resp.headers().get("X-Inertia-Location").unwrap(),
        "https://example.com/external"
    );
}

// ---- Tier 3.1: fragment preservation ----
//
// `preserveFragment` is a page-object flag set on the *destination*
// response of a redirect - the client (which knows its own URL hash)
// carries the fragment over to the new URL when this flag is true.
// `InertiaResponse::redirect(url)` is the X-Inertia-Redirect mechanism
// for soft Inertia redirects whose target URL may carry a `#fragment`.

#[tokio::test]
async fn preserve_fragment_true_emits_flag_in_page_object() {
    let req = MockReq::new("/article/new").inertia();
    let resp = InertiaResponse::new("Article/Show")
        .preserve_fragment(true)
        .resolve(&req)
        .await
        .unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(page["preserveFragment"], true);
}

#[tokio::test]
async fn preserve_fragment_default_does_not_emit_flag() {
    let req = MockReq::new("/article").inertia();
    let resp = InertiaResponse::new("Article/Show")
        .resolve(&req)
        .await
        .unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(!page.as_object().unwrap().contains_key("preserveFragment"));
}

#[tokio::test]
async fn preserve_fragment_false_does_not_emit_flag() {
    let req = MockReq::new("/article").inertia();
    let resp = InertiaResponse::new("Article/Show")
        .preserve_fragment(false)
        .resolve(&req)
        .await
        .unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(!page.as_object().unwrap().contains_key("preserveFragment"));
}

#[tokio::test]
async fn inertia_redirect_returns_409_with_x_inertia_redirect() {
    let resp = InertiaResponse::redirect("/article/new#section");
    let hyper_resp = resp.into_hyper();
    assert_eq!(hyper_resp.status(), 409);
    assert_eq!(
        hyper_resp.headers().get("X-Inertia-Redirect").unwrap(),
        "/article/new#section"
    );
    // X-Inertia-Redirect is distinct from X-Inertia-Location - only
    // one of the two should be present, per the protocol.
    assert!(hyper_resp.headers().get("X-Inertia-Location").is_none());
}

#[tokio::test]
async fn inertia_redirect_distinct_from_location() {
    // Sanity check: redirect() and location() produce different shapes.
    let redirect = InertiaResponse::redirect("/foo").into_hyper();
    let location = InertiaResponse::location("/foo").into_hyper();

    assert!(redirect.headers().get("X-Inertia-Redirect").is_some());
    assert!(redirect.headers().get("X-Inertia-Location").is_none());

    assert!(location.headers().get("X-Inertia-Redirect").is_none());
    assert!(location.headers().get("X-Inertia-Location").is_some());
}

#[tokio::test]
async fn preserve_fragment_flows_through_html_shell_data_page() {
    // Initial (non-XHR) visit returns the HTML shell with the page object
    // embedded as the textContent of the Inertia 3 sibling
    // `<script type="application/json" data-page="app">` tag. Verify
    // `preserveFragment: true` survives that path the same as the XHR path.
    let req = MockReq::new("/article/new"); // no X-Inertia → HTML response
    let resp = InertiaResponse::new("Article/Show")
        .preserve_fragment(true)
        .resolve(&req)
        .await
        .unwrap();
    let body = body_to_string(resp.into_hyper().into_body());

    // The script tag's textContent is raw JSON (no HTML attribute encoding),
    // so `"preserveFragment":true` appears verbatim.
    assert!(
        body.contains(r#""preserveFragment":true"#),
        "expected raw preserveFragment:true in <script data-page>; body was:\n{}",
        body
    );
}

#[tokio::test]
async fn preserve_fragment_survives_partial_reload_filter() {
    // `preserveFragment` is a top-level page-object flag, not a prop, so
    // partial-reload filtering (which only filters `props`) must not
    // affect it. Drive a partial reload with `X-Inertia-Partial-Component`
    // + `X-Inertia-Partial-Data` and verify the flag still emits.
    let req = MockReq::new("/article")
        .inertia()
        .header("X-Inertia-Partial-Component", "Article/Show")
        .header("X-Inertia-Partial-Data", "title");
    let resp = InertiaResponse::new("Article/Show")
        .preserve_fragment(true)
        .with("title", "Welcome")
        .with("body", "long content")
        .resolve(&req)
        .await
        .unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    // Partial filter limited props to `title` only.
    assert_eq!(page["props"]["title"], "Welcome");
    assert!(page["props"].as_object().unwrap().get("body").is_none());
    // …but the top-level flag is unaffected.
    assert_eq!(page["preserveFragment"], true);
}

// ---- Tier 4: SSR ----
//
// These tests spawn a tiny localhost HTTP server that mimics the
// `@inertiajs/{...}/server` SSR worker - accepts `POST /render` with
// the page object, returns `{head, body}`. We then resolve an Inertia
// response with SSR enabled pointed at this worker and inspect the
// generated HTML shell.

mod ssr_tests {
    use super::*;
    use http_body_util::Full;
    use hyper::body::Bytes;
    use hyper::server::conn::http1;
    use hyper::service::service_fn;
    use hyper_util::rt::TokioIo;
    use std::convert::Infallible;
    use std::net::SocketAddr;
    use suprnova::{InertiaConfig, InertiaResponse};

    /// Spawn a one-shot SSR worker bound to 127.0.0.1:0. The worker
    /// returns a `body` matching the real Inertia 3 contract - what
    /// `@inertiajs/core::buildSSRBody` produces: a `<script
    /// type="application/json" data-page="app">` element holding the
    /// page JSON, followed by `<div data-server-rendered="true"
    /// id="app">…prerendered…</div>`. The framework injects this raw
    /// (no wrapping div of its own). Returns the listening address.
    async fn spawn_mock_ssr() -> SocketAddr {
        spawn_mock_ssr_with_head(&[
            "<title>SSR Title</title>",
            "<meta name=\"ssr\" content=\"yes\">",
        ])
        .await
    }

    /// [`spawn_mock_ssr`] with the worker's `head` array chosen by the
    /// caller - what the page's own `Head` component rendered, which is
    /// the thing the framework has to reconcile its default `<title>`
    /// against.
    async fn spawn_mock_ssr_with_head(head: &[&str]) -> SocketAddr {
        let body = serde_json::json!({
            "head": head,
            "body": "<script type=\"application/json\" data-page=\"app\">{\"component\":\"Home\"}</script><div data-server-rendered=\"true\" id=\"app\"><main id=\"ssr\">SSR rendered content</main></div>",
        });
        let payload = Bytes::from(serde_json::to_vec(&body).unwrap());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let (stream, _) = match listener.accept().await {
                    Ok(p) => p,
                    Err(_) => break,
                };
                let payload = payload.clone();
                tokio::spawn(async move {
                    let io = TokioIo::new(stream);
                    let svc = service_fn(move |_req: hyper::Request<hyper::body::Incoming>| {
                        let payload = payload.clone();
                        async move {
                            Ok::<_, Infallible>(
                                hyper::Response::builder()
                                    .status(200)
                                    .header("content-type", "application/json")
                                    .body(Full::new(payload))
                                    .unwrap(),
                            )
                        }
                    });
                    let _ = http1::Builder::new().serve_connection(io, svc).await;
                });
            }
        });
        addr
    }

    #[tokio::test]
    async fn ssr_disabled_by_default_produces_empty_mount() {
        let req = MockReq::new("/"); // non-XHR initial visit
        let resp = InertiaResponse::new("Home")
            .with("title", "Hi")
            .resolve(&req)
            .await
            .unwrap();
        let body = body_to_string(resp.into_hyper().into_body());
        assert!(!body.contains("data-server-rendered"));
        assert!(body.contains("<div id=\"app\""));
    }

    #[tokio::test]
    async fn ssr_enabled_injects_head_and_body_with_data_attr() {
        let addr = spawn_mock_ssr().await;
        let cfg = InertiaConfig::new().ssr(format!("http://{}", addr));
        let req = MockReq::new("/");
        let resp = InertiaResponse::new("Home")
            .with_config(cfg)
            .with("title", "Hi")
            .resolve(&req)
            .await
            .unwrap();
        let body = body_to_string(resp.into_hyper().into_body());

        assert!(
            body.contains("data-server-rendered=\"true\""),
            "expected data-server-rendered on mount; body:\n{}",
            body
        );
        assert!(body.contains("<title>SSR Title</title>"));
        assert!(body.contains("<meta name=\"ssr\" content=\"yes\">"));
        assert!(body.contains("<main id=\"ssr\">SSR rendered content</main>"));
        // Inject the worker's body verbatim; don't double-wrap. Two
        // `<div ... id="app">` would produce duplicate IDs and break
        // `document.getElementById('app')` in the client hydration.
        assert_eq!(
            body.matches("id=\"app\"").count(),
            1,
            "framework must not wrap the SSR body in a second mount div; body:\n{}",
            body
        );
    }

    /// A page that renders its own `<title>` through Inertia's `Head`
    /// component sends it back in the SSR head. The framework's default
    /// title must stand down: emitted as well it would be the *first*
    /// `<title>` in the document, and first is the one browsers, crawlers
    /// and the pre-hydration tab read.
    #[tokio::test]
    async fn an_ssr_head_that_carries_a_title_replaces_the_frameworks_default() {
        let addr = spawn_mock_ssr().await;
        let cfg = InertiaConfig::new().ssr(format!("http://{}", addr));
        let req = MockReq::new("/");
        let resp = InertiaResponse::new("Home")
            .with_config(cfg)
            .resolve(&req)
            .await
            .unwrap();
        let body = body_to_string(resp.into_hyper().into_body());

        assert_eq!(
            body.matches("<title").count(),
            1,
            "a document must carry exactly one title; body:\n{body}"
        );
        assert!(body.contains("<title>SSR Title</title>"));
        assert!(
            !body.contains("<title>Suprnova</title>"),
            "the page's own head wins over the config default; body:\n{body}"
        );
    }

    /// The mirror: a worker whose head carries no title leaves the
    /// framework's default in place, exactly as before.
    #[tokio::test]
    async fn an_ssr_head_without_a_title_keeps_the_frameworks_default() {
        let addr = spawn_mock_ssr_with_head(&["<meta name=\"ssr\" content=\"yes\">"]).await;
        let cfg = InertiaConfig::new()
            .ssr(format!("http://{}", addr))
            .default_title("Acme App");
        let req = MockReq::new("/");
        let resp = InertiaResponse::new("Home")
            .with_config(cfg)
            .resolve(&req)
            .await
            .unwrap();
        let body = body_to_string(resp.into_hyper().into_body());

        assert_eq!(
            body.matches("<title").count(),
            1,
            "the default is still the document's only title; body:\n{body}"
        );
        assert!(body.contains("<title>Acme App</title>"));
    }

    /// The title rule matches the element, not the string: a custom
    /// element whose name merely starts with `title` must not be mistaken
    /// for one, or the document ends up with no title at all.
    #[tokio::test]
    async fn a_head_element_that_merely_starts_with_title_is_not_a_title() {
        let addr = spawn_mock_ssr_with_head(&["<title-bar data-x=\"1\"></title-bar>"]).await;
        let cfg = InertiaConfig::new()
            .ssr(format!("http://{}", addr))
            .default_title("Acme App");
        let req = MockReq::new("/");
        let resp = InertiaResponse::new("Home")
            .with_config(cfg)
            .resolve(&req)
            .await
            .unwrap();
        let body = body_to_string(resp.into_hyper().into_body());

        assert!(
            body.contains("<title>Acme App</title>"),
            "`<title-bar>` is not a title; body:\n{body}"
        );
    }

    /// The SSR path shares the shell with the non-SSR path, so it must
    /// declare the active locale too.
    #[cfg(feature = "localization")]
    #[tokio::test]
    async fn the_ssr_shell_declares_the_active_locale() {
        let addr = spawn_mock_ssr().await;
        let cfg = InertiaConfig::new().ssr(format!("http://{}", addr));
        let req = MockReq::new("/");
        let body = suprnova::scope_locale(suprnova::Locale::parse("ja").unwrap(), async {
            let resp = InertiaResponse::new("Home")
                .with_config(cfg)
                .resolve(&req)
                .await
                .unwrap();
            body_to_string(resp.into_hyper().into_body())
        })
        .await;

        assert!(body.contains("<html lang=\"ja\">"), "got:\n{body}");
    }

    #[tokio::test]
    async fn ssr_worker_unreachable_falls_back_to_csr() {
        // Point at a port nothing is listening on. Default
        // throw_on_error=false → falls back silently.
        let cfg = InertiaConfig::new().ssr("http://127.0.0.1:1");
        let req = MockReq::new("/");
        let resp = InertiaResponse::new("Home")
            .with_config(cfg)
            .resolve(&req)
            .await
            .unwrap();
        let body = body_to_string(resp.into_hyper().into_body());
        assert!(!body.contains("data-server-rendered"));
    }

    #[tokio::test]
    async fn ssr_throw_on_error_propagates_error() {
        let cfg = InertiaConfig::new()
            .ssr("http://127.0.0.1:1")
            .ssr_throw_on_error(true);
        let req = MockReq::new("/");
        let result = InertiaResponse::new("Home")
            .with_config(cfg)
            .resolve(&req)
            .await;
        assert!(
            result.is_err(),
            "throw_on_error=true must propagate worker failure"
        );
    }

    #[tokio::test]
    async fn ssr_excluded_path_skips_worker() {
        // Even with a working worker, excluded paths render CSR.
        let addr = spawn_mock_ssr().await;
        let cfg = InertiaConfig::new()
            .ssr(format!("http://{}", addr))
            .ssr_exclude("/admin/**");
        let req = MockReq::new("/admin/users");
        let resp = InertiaResponse::new("Admin/Users")
            .with_config(cfg)
            .resolve(&req)
            .await
            .unwrap();
        let body = body_to_string(resp.into_hyper().into_body());
        assert!(!body.contains("data-server-rendered"));
    }

    #[tokio::test]
    async fn ssr_xhr_request_does_not_invoke_worker() {
        // For Inertia XHRs we return JSON, not HTML - SSR is irrelevant.
        // We bind a worker that would PANIC if called; if SSR is invoked
        // erroneously, the request stalls or errors.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handler_called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let called = handler_called.clone();
        tokio::spawn(async move {
            if let Ok((stream, _)) = listener.accept().await {
                called.store(true, std::sync::atomic::Ordering::SeqCst);
                drop(stream);
            }
        });
        let cfg = InertiaConfig::new().ssr(format!("http://{}", addr));
        let req = MockReq::new("/").inertia();
        let resp = InertiaResponse::new("Home")
            .with_config(cfg)
            .resolve(&req)
            .await
            .unwrap();
        let _ = body_to_string(resp.into_hyper().into_body());
        // Slack for spurious accept races: we only assert the SSR
        // handler was NOT triggered.
        assert!(
            !handler_called.load(std::sync::atomic::Ordering::SeqCst),
            "XHR responses must not contact the SSR worker"
        );
    }
}

// ---- Infinite scroll: Inertia::scroll() + scrollProps + merge intent ----
//
// A scroll prop always carries merge metadata - unlike a plain merge
// prop it needs no explicit `.merge()` - defaulting to append and
// switching to prepend only when the client sends
// `X-Inertia-Infinite-Scroll-Merge-Intent: prepend`. `reset` reads
// `X-Inertia-Reset` independently of that header. Both match Laravel's
// `ScrollProp::configureMergeIntent` + `Response::resolveScrollProps`
// (`inertia-laravel-2.0.25/src/ScrollProp.php:72-79`,
// `src/Response.php:700-716`).

use suprnova::{Prop, ScrollMetadata};

#[tokio::test]
async fn scroll_fresh_visit_emits_reset_false_and_append_merge_metadata() {
    // No merge-intent header, no reset header: Laravel still emits the
    // append merge instruction on this very response - it isn't held
    // back for a follow-up fetch - and `reset` is false because the
    // client never asked to start over.
    let req = MockReq::new("/users").inertia();
    let resp = InertiaResponse::new("Users/Index")
        .scroll(
            "users",
            ScrollMetadata::new("page").current(1).next(2),
            serde_json::json!([{"id": 1, "name": "Alice"}]),
        )
        .resolve(&req)
        .await
        .unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();

    // Value is in props.
    assert_eq!(page["props"]["users"][0]["name"], "Alice");
    let scroll = &page["scrollProps"]["users"];
    assert_eq!(scroll["pageName"], "page");
    assert_eq!(scroll["currentPage"], 1);
    assert_eq!(scroll["nextPage"], 2);
    assert_eq!(scroll["previousPage"], serde_json::Value::Null);
    assert_eq!(
        scroll["reset"], false,
        "no X-Inertia-Reset means reset stays false"
    );
    let merge: Vec<&str> = page["mergeProps"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(
        merge.contains(&"users"),
        "a fresh visit still carries the append merge instruction; got {page}"
    );
}

#[tokio::test]
async fn scroll_append_intent_emits_merge_props_no_reset() {
    let req = MockReq::new("/users")
        .inertia()
        .header("X-Inertia-Infinite-Scroll-Merge-Intent", "append");
    let resp = InertiaResponse::new("Users/Index")
        .scroll(
            "users",
            ScrollMetadata::new("page").current(2).next(3).previous(1),
            serde_json::json!([{"id": 21}]),
        )
        .resolve(&req)
        .await
        .unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();

    let scroll = &page["scrollProps"]["users"];
    assert_eq!(scroll["reset"], false, "append fetch must not reset");
    let merge: Vec<&str> = page["mergeProps"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(merge.contains(&"users"));
}

#[tokio::test]
async fn scroll_prepend_intent_emits_prepend_props_no_reset() {
    let req = MockReq::new("/users")
        .inertia()
        .header("X-Inertia-Infinite-Scroll-Merge-Intent", "prepend");
    let resp = InertiaResponse::new("Users/Index")
        .scroll(
            "users",
            ScrollMetadata::new("page").current(0).previous(-1).next(1),
            serde_json::json!([{"id": 0}]),
        )
        .resolve(&req)
        .await
        .unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert_eq!(page["scrollProps"]["users"]["reset"], false);
    let prepend: Vec<&str> = page["prependProps"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(prepend.contains(&"users"));
}

#[tokio::test]
async fn scroll_unknown_intent_falls_back_to_append_default() {
    // Only "append" / "prepend" are meaningful intent values, so the
    // header parser collapses anything else to `None` - the same state
    // as no header at all, which now means "append", not "reset".
    let req = MockReq::new("/users")
        .inertia()
        .header("X-Inertia-Infinite-Scroll-Merge-Intent", "garbage");
    let resp = InertiaResponse::new("Users/Index")
        .scroll(
            "users",
            ScrollMetadata::new("page").current(1).next(2),
            serde_json::json!([]),
        )
        .resolve(&req)
        .await
        .unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(page["scrollProps"]["users"]["reset"], false);
    let merge: Vec<&str> = page["mergeProps"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(
        merge.contains(&"users"),
        "an unrecognized intent still defaults to append"
    );
}

#[tokio::test]
async fn scroll_with_async_resolver_runs_closure() {
    let req = MockReq::new("/users").inertia();
    let call_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = call_count.clone();
    let resp = InertiaResponse::new("Users/Index")
        .scroll_with("users", ScrollMetadata::new("page").current(1), move || {
            let c = counter.clone();
            async move {
                c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok::<_, suprnova::FrameworkError>(serde_json::json!([{"id": 1}]))
            }
        })
        .resolve(&req)
        .await
        .unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert_eq!(call_count.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(page["props"]["users"][0]["id"], 1);
    assert_eq!(page["scrollProps"]["users"]["currentPage"], 1);
}

#[tokio::test]
async fn scroll_props_field_omitted_when_no_scroll_props() {
    let req = MockReq::new("/").inertia();
    let resp = InertiaResponse::new("Home")
        .with("title", "x")
        .resolve(&req)
        .await
        .unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(!page.as_object().unwrap().contains_key("scrollProps"));
}

#[tokio::test]
async fn scroll_reset_header_sets_reset_true_and_excludes_merge_metadata() {
    // Laravel-identical `reset` semantics: it comes from `X-Inertia-Reset`
    // alone, not from the merge-intent header - so a client that sends
    // BOTH still gets `reset: true` and no merge instruction for the key.
    let req = MockReq::new("/users")
        .inertia()
        .header("X-Inertia-Partial-Component", "Users/Index")
        .header("X-Inertia-Partial-Data", "users")
        .header("X-Inertia-Reset", "users")
        .header("X-Inertia-Infinite-Scroll-Merge-Intent", "append");
    let resp = InertiaResponse::new("Users/Index")
        .scroll(
            "users",
            ScrollMetadata::new("page").current(2).next(3),
            serde_json::json!([{"id": 21}]),
        )
        .resolve(&req)
        .await
        .unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();

    // Value still resolves normally.
    assert_eq!(page["props"]["users"][0]["id"], 21);
    assert_eq!(page["scrollProps"]["users"]["reset"], true);
    let obj = page.as_object().unwrap();
    let merge_props = obj
        .get("mergeProps")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let names: Vec<&str> = merge_props.iter().filter_map(|v| v.as_str()).collect();
    assert!(
        !names.contains(&"users"),
        "a reset key must not appear in mergeProps; got {page}"
    );
}

#[tokio::test]
async fn scroll_reset_header_excludes_from_prepend_props_too() {
    // Same exclusion, prepend direction - reset wins regardless of which
    // way the intent header points.
    let req = MockReq::new("/users")
        .inertia()
        .header("X-Inertia-Partial-Component", "Users/Index")
        .header("X-Inertia-Partial-Data", "users")
        .header("X-Inertia-Reset", "users")
        .header("X-Inertia-Infinite-Scroll-Merge-Intent", "prepend");
    let resp = InertiaResponse::new("Users/Index")
        .scroll(
            "users",
            ScrollMetadata::new("page").current(0),
            serde_json::json!([{"id": 0}]),
        )
        .resolve(&req)
        .await
        .unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert_eq!(page["scrollProps"]["users"]["reset"], true);
    let obj = page.as_object().unwrap();
    let prepend_props = obj
        .get("prependProps")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let names: Vec<&str> = prepend_props.iter().filter_map(|v| v.as_str()).collect();
    assert!(!names.contains(&"users"));
}

#[tokio::test]
async fn scroll_metadata_handles_string_cursor() {
    // Cursor pagination uses string identifiers, not numbers.
    let req = MockReq::new("/posts").inertia();
    let resp = InertiaResponse::new("Posts/Index")
        .scroll(
            "posts",
            ScrollMetadata::new("cursor")
                .current("c-100")
                .next("c-200")
                .previous("c-50"),
            serde_json::json!([]),
        )
        .resolve(&req)
        .await
        .unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    let scroll = &page["scrollProps"]["posts"];
    assert_eq!(scroll["pageName"], "cursor");
    assert_eq!(scroll["currentPage"], "c-100");
    assert_eq!(scroll["nextPage"], "c-200");
}

#[tokio::test]
async fn scroll_wrapped_targets_merge_metadata_at_nested_path() {
    let req = MockReq::new("/feed").inertia();
    let resp = InertiaResponse::new("Feed/Index")
        .scroll_wrapped(
            "posts",
            "data",
            ScrollMetadata::new("page").current(2).next(3),
            serde_json::json!({ "data": [{"id": 1}], "meta": { "total": 1 } }),
        )
        .resolve(&req)
        .await
        .unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert_eq!(page["props"]["posts"]["data"][0]["id"], 1);
    let merge: Vec<&str> = page["mergeProps"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(
        merge.contains(&"posts.data"),
        "wrapped scroll must target the nested path, not the bare key; got {page}"
    );
    assert!(!merge.contains(&"posts"));
    assert_eq!(page["scrollProps"]["posts"]["reset"], false);
}

#[tokio::test]
async fn scroll_with_wrapped_resolver_runs_closure_and_wraps_the_prepend_path() {
    let req = MockReq::new("/feed")
        .inertia()
        .header("X-Inertia-Infinite-Scroll-Merge-Intent", "prepend");
    let call_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = call_count.clone();
    let resp = InertiaResponse::new("Feed/Index")
        .scroll_with_wrapped(
            "posts",
            "data",
            ScrollMetadata::new("page").current(1),
            move || {
                let c = counter.clone();
                async move {
                    c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    Ok::<_, suprnova::FrameworkError>(serde_json::json!({ "data": [{"id": 9}] }))
                }
            },
        )
        .resolve(&req)
        .await
        .unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert_eq!(call_count.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(page["props"]["posts"]["data"][0]["id"], 9);
    let prepend: Vec<&str> = page["prependProps"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(prepend.contains(&"posts.data"));
}

#[tokio::test]
async fn scroll_match_on_emits_match_props_on_keyed_to_the_bare_prop_name() {
    // An unwrapped scroll prop merges at its own key, so its `match_on`
    // fields key off that same bare name - Laravel's
    // `resolveMergeMatchingKeys` folds a `ScrollProp`'s `matchesOn()` in
    // exactly like any other `Mergeable`, no scroll exclusion
    // (`Response.php:558,641-652`).
    let req = MockReq::new("/users").inertia();
    let resp = InertiaResponse::new("Users/Index")
        .prop(
            "users",
            Prop::eager(serde_json::json!([{"id": 1, "name": "Alice"}]))
                .scroll(ScrollMetadata::new("page").current(1).next(2))
                .match_on(["id"]),
        )
        .resolve(&req)
        .await
        .unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert_eq!(
        page["matchPropsOn"],
        serde_json::json!(["users.id"]),
        "an unwrapped scroll prop's match_on field must key off the bare prop name; got {page}"
    );
}

#[tokio::test]
async fn scroll_wrapped_match_on_emits_match_props_on_keyed_to_the_wrap_path() {
    // A wrapped scroll prop merges at `key.wrap_key`, not `key` - so its
    // `match_on` field must key off that same nested path, or the
    // client's prefix-matching `mergeOrMatchItems` can never find it
    // (`inertia-3.6.1/packages/core/src/response.ts:524-546`).
    let req = MockReq::new("/feed").inertia();
    let resp = InertiaResponse::new("Feed/Index")
        .prop(
            "posts",
            Prop::eager(serde_json::json!({ "data": [{"id": 1}], "meta": { "total": 1 } }))
                .scroll(ScrollMetadata::new("page").current(2).next(3))
                .scroll_wrap("data")
                .match_on("id"),
        )
        .resolve(&req)
        .await
        .unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert_eq!(
        page["matchPropsOn"],
        serde_json::json!(["posts.data.id"]),
        "a wrapped scroll prop's match_on field must key off key.wrap_key, not the bare key; got {page}"
    );
}

#[tokio::test]
async fn scroll_always_prop_outside_only_list_emits_no_merge_metadata() {
    // Regression for a review finding (I3): `should_include` is
    // unconditionally true for an Always prop, so gating the scroll
    // block on it (rather than on `passes_lists`, the same gate the
    // once/merge blocks use) let an Always+scroll prop outside a
    // partial reload's `only` list emit a merge instruction for a
    // value that already shipped whole - the client would then append
    // the same rows on top of themselves. Laravel narrows both
    // `resolveMergeProps` and `resolveScrollProps` by `only`/`except`
    // (`Response.php:553-560`, `:700-716`), independent of `Always`
    // bypassing the value filter. Mirrors the plain-merge-prop rule
    // already pinned by
    // `an_always_merge_prop_keeps_its_value_but_drops_merge_metadata_when_filtered_out`
    // in `inertia_prop_composition.rs`.
    let req = MockReq::new("/users")
        .inertia()
        .header("X-Inertia-Partial-Component", "Users/Index")
        .header("X-Inertia-Partial-Data", "other");
    let resp = InertiaResponse::new("Users/Index")
        .prop(
            "users",
            Prop::eager(serde_json::json!([{"id": 1}]))
                .scroll(ScrollMetadata::new("page").current(1).next(2))
                .always(),
        )
        .with("other", 1)
        .resolve(&req)
        .await
        .unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();

    // The value still ships - Always bypasses the value filter.
    assert_eq!(page["props"]["users"][0]["id"], 1);
    let obj = page.as_object().unwrap();
    let merge_props = obj
        .get("mergeProps")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let names: Vec<&str> = merge_props.iter().filter_map(|v| v.as_str()).collect();
    assert!(
        !names.contains(&"users"),
        "an Always scroll prop outside the requested set must not emit a merge instruction; got {page}"
    );
    assert!(
        !obj.contains_key("scrollProps")
            || !obj["scrollProps"]
                .as_object()
                .unwrap()
                .contains_key("users"),
        "an Always scroll prop outside the requested set must not emit a scrollProps entry either; got {page}"
    );
}

#[tokio::test]
async fn scroll_deep_merge_emits_deep_merge_props_not_merge_props() {
    // I4: Laravel's `ScrollProp` constructor already sets `merge = true`
    // (`ScrollProp.php:60`), so a caller's own `->merge()`/`->prepend()`
    // has nothing left to change - but `->deepMerge()` routes the prop
    // through `resolveDeepMergeProps` instead, a completely separate
    // list ahead of the append/prepend computation
    // (`Response.php:590,610`). `.deep_merge()` on a scroll prop must
    // land in `deepMergeProps`, not `mergeProps`.
    let req = MockReq::new("/users").inertia();
    let resp = InertiaResponse::new("Users/Index")
        .prop(
            "users",
            Prop::eager(serde_json::json!({ "a": 1 }))
                .scroll(ScrollMetadata::new("page").current(1))
                .deep_merge(),
        )
        .resolve(&req)
        .await
        .unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();

    let deep: Vec<&str> = page["deepMergeProps"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(
        deep.contains(&"users"),
        "a deep-merge scroll prop must land in deepMergeProps; got {page}"
    );
    let obj = page.as_object().unwrap();
    let merge_props = obj
        .get("mergeProps")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    assert!(
        !merge_props
            .iter()
            .filter_map(|v| v.as_str())
            .any(|s| s == "users"),
        "a deep-merge scroll prop must not also appear in mergeProps; got {page}"
    );
}

#[tokio::test]
async fn scroll_wrapped_reset_excludes_the_wrapped_path_from_merge_props() {
    // Coverage gap (m8): the reset-exclusion tests above only cover the
    // unwrapped case (`scroll_reset_header_sets_reset_true_and_excludes_merge_metadata`).
    // A wrapped scroll prop must exclude its wrapped path
    // (`posts.data`), not the bare key, since the bare key is never
    // what would have been pushed anyway.
    let req = MockReq::new("/feed")
        .inertia()
        .header("X-Inertia-Partial-Component", "Feed/Index")
        .header("X-Inertia-Partial-Data", "posts")
        .header("X-Inertia-Reset", "posts");
    let resp = InertiaResponse::new("Feed/Index")
        .scroll_wrapped(
            "posts",
            "data",
            ScrollMetadata::new("page").current(2).next(3),
            serde_json::json!({ "data": [{"id": 1}], "meta": { "total": 1 } }),
        )
        .resolve(&req)
        .await
        .unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert_eq!(page["scrollProps"]["posts"]["reset"], true);
    let obj = page.as_object().unwrap();
    let merge_props = obj
        .get("mergeProps")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let names: Vec<&str> = merge_props.iter().filter_map(|v| v.as_str()).collect();
    assert!(
        !names.contains(&"posts.data"),
        "a reset key's wrapped merge path must be excluded; got {page}"
    );
    assert!(!names.contains(&"posts"));
}

// ---- Purpose: prefetch header ----

#[tokio::test]
async fn is_prefetch_detects_purpose_header() {
    let req = MockReq::new("/").inertia().header("Purpose", "prefetch");
    assert!(req.is_prefetch());
    assert!(req.is_inertia(), "prefetch is independent of is_inertia");
}

#[tokio::test]
async fn is_prefetch_case_insensitive() {
    let req = MockReq::new("/").header("Purpose", "Prefetch");
    assert!(req.is_prefetch());
    let req = MockReq::new("/").header("Purpose", "PREFETCH");
    assert!(req.is_prefetch());
}

#[tokio::test]
async fn is_prefetch_false_when_header_missing_or_other_value() {
    let req = MockReq::new("/");
    assert!(!req.is_prefetch());
    let req = MockReq::new("/").header("Purpose", "navigation");
    assert!(!req.is_prefetch());
    let req = MockReq::new("/").header("Purpose", "");
    assert!(!req.is_prefetch());
}

// ---- X-Inertia-Error-Bag header ----

#[tokio::test]
async fn errors_default_is_flat_empty_object() {
    let req = MockReq::new("/").inertia();
    let resp = InertiaResponse::new("Home").resolve(&req).await.unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    // No error bag → flat `errors: {}` shape.
    assert!(page["props"]["errors"].is_object());
    assert!(page["props"]["errors"].as_object().unwrap().is_empty());
}

#[tokio::test]
async fn errors_scoped_under_named_bag_when_header_set() {
    let req = MockReq::new("/")
        .inertia()
        .header("X-Inertia-Error-Bag", "registration");
    let resp = InertiaResponse::new("Home").resolve(&req).await.unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    // Errors are now `errors: { registration: {} }`.
    let errors = page["props"]["errors"].as_object().unwrap();
    assert!(errors.contains_key("registration"));
    assert!(errors["registration"].is_object());
}

#[tokio::test]
async fn error_bag_wraps_handler_injected_errors() {
    // Regression test: previously the bag scoping was done at the
    // start of resolve_props with an empty object, then user props
    // could overwrite it - silently losing the bag wrapping. The fix
    // moves scoping to after all props resolve.
    let req = MockReq::new("/")
        .inertia()
        .header("X-Inertia-Error-Bag", "checkout");
    let resp = InertiaResponse::new("Home")
        .with(
            "errors",
            serde_json::json!({"email": "must be valid", "card": "expired"}),
        )
        .resolve(&req)
        .await
        .unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    let errors = page["props"]["errors"].as_object().unwrap();
    assert!(
        errors.contains_key("checkout"),
        "handler-injected errors must be wrapped under bag, got: {:?}",
        errors
    );
    assert_eq!(errors["checkout"]["email"], "must be valid");
    assert_eq!(errors["checkout"]["card"], "expired");
}

// ---- `errors` under only/except ----
//
// Laravel shares `errors` as `Inertia::always(...)`
// (`inertia-laravel-2.0.25/src/Middleware.php:61`), and `resolveAlways`
// re-injects an `AlwaysProp`'s raw value after the only/except rebuild
// rather than narrowing it (`Response.php:406-416`). So the bag is
// exempt from partial-reload filtering on both counts: it ships on a
// partial that never names it, and it ships whole when one does.
//
// Suprnova seeds the session-flashed bag before the resolve loop, so
// that path was already exempt. A handler-supplied `.with("errors", …)`
// prop went through the ordinary gates instead, which made the same page
// ship a whole bag or a sliced one depending only on where the errors
// came from.

#[tokio::test]
async fn a_handler_supplied_errors_bag_is_not_narrowed_by_a_dotted_only_entry() {
    let req = MockReq::new("/")
        .inertia()
        .header("X-Inertia-Partial-Component", "Home")
        .header("X-Inertia-Partial-Data", "errors.email");
    let resp = InertiaResponse::new("Home")
        .with(
            "errors",
            serde_json::json!({"email": "Invalid", "card": "Expired"}),
        )
        .resolve(&req)
        .await
        .unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    let errors = page["props"]["errors"].as_object().unwrap();

    assert_eq!(errors["email"], "Invalid");
    assert_eq!(
        errors["card"], "Expired",
        "the errors bag is an always prop and is never narrowed; got {errors:?}"
    );
}

#[tokio::test]
async fn a_handler_supplied_errors_bag_ships_on_a_partial_that_never_names_it() {
    let req = MockReq::new("/")
        .inertia()
        .header("X-Inertia-Partial-Component", "Home")
        .header("X-Inertia-Partial-Data", "users");
    let resp = InertiaResponse::new("Home")
        .with("users", serde_json::json!([]))
        .with("errors", serde_json::json!({"email": "Invalid"}))
        .resolve(&req)
        .await
        .unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    let errors = page["props"]["errors"].as_object().unwrap();

    assert_eq!(
        errors["email"], "Invalid",
        "the errors bag survives a partial that does not name it; got {errors:?}"
    );
}

#[tokio::test]
async fn an_except_entry_cannot_drop_the_errors_bag() {
    // `Arr::forget` removes it and `resolveAlways` puts it straight back
    // (`Response.php:292-294`, `:406-416`), so `except=errors` is inert.
    // The seeded bag is already unreachable by `except`; the handler bag
    // has to behave the same way.
    let req = MockReq::new("/")
        .inertia()
        .header("X-Inertia-Partial-Component", "Home")
        .header("X-Inertia-Partial-Except", "errors");
    let resp = InertiaResponse::new("Home")
        .with("errors", serde_json::json!({"email": "Invalid"}))
        .resolve(&req)
        .await
        .unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert_eq!(page["props"]["errors"]["email"], "Invalid", "got {page}");
}

#[tokio::test]
async fn an_explicit_optional_flag_on_the_errors_key_still_wins() {
    // The exemption is a default, not a law. `Inertia::optional(...)`
    // under the `errors` key replaces the middleware's `AlwaysProp` in
    // Laravel's merged bag and behaves optionally; the same flag has to
    // mean the same thing here.
    let req = MockReq::new("/")
        .inertia()
        .header("X-Inertia-Partial-Component", "Home")
        .header("X-Inertia-Partial-Data", "users");
    let resp = InertiaResponse::new("Home")
        .with("users", serde_json::json!([]))
        .prop(
            "errors",
            suprnova::Prop::eager(serde_json::json!({"email": "Invalid"})).optional(),
        )
        .resolve(&req)
        .await
        .unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert!(
        page["props"]["errors"].as_object().unwrap().is_empty(),
        "an optional errors prop is withheld unless named; got {page}"
    );
}

#[tokio::test]
async fn a_dotted_only_entry_does_not_narrow_the_session_seeded_errors_bag_either() {
    // The other half of the pair: the seeded path, pinned so the two
    // cannot drift apart again.
    use suprnova::Redirect;
    use suprnova::session::{new_session_slot_for_test, session_mut, session_scope_for_test};

    let slot = new_session_slot_for_test();
    session_scope_for_test(slot, async {
        let _: suprnova::Response = Redirect::to("/login")
            .with_errors([("email", "Invalid"), ("card", "Expired")])
            .into();
        session_mut(|s| s.age_flash_data());

        let req = MockReq::new("/")
            .inertia()
            .header("X-Inertia-Partial-Component", "Login")
            .header("X-Inertia-Partial-Data", "errors.email");
        let resp = InertiaResponse::new("Login").resolve(&req).await.unwrap();
        let body = body_to_string(resp.into_hyper().into_body());
        let page: serde_json::Value = serde_json::from_str(&body).unwrap();
        let errors = page["props"]["errors"].as_object().unwrap();

        assert_eq!(errors["email"], "Invalid");
        assert_eq!(errors["card"], "Expired", "got {errors:?}");
    })
    .await;
}

#[tokio::test]
async fn empty_error_bag_header_treated_as_unset() {
    let req = MockReq::new("/")
        .inertia()
        .header("X-Inertia-Error-Bag", "  ");
    let resp = InertiaResponse::new("Home").resolve(&req).await.unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    // Whitespace-only / empty bag should fall back to flat shape.
    let errors = page["props"]["errors"].as_object().unwrap();
    assert!(errors.is_empty(), "expected flat errors, got {:?}", errors);
}

#[tokio::test]
async fn flashed_default_bag_errors_seed_flat_not_bag_keyed() {
    use suprnova::Redirect;
    use suprnova::session::{new_session_slot_for_test, session_mut, session_scope_for_test};

    let slot = new_session_slot_for_test();
    session_scope_for_test(slot, async {
        // A previous request flashed validation errors under the default bag.
        let _: suprnova::Response = Redirect::to("/login")
            .with_errors([("email", "Invalid")])
            .into();
        // SessionMiddleware ages new -> old at the start of the next request.
        session_mut(|s| s.age_flash_data());

        // The receiving page (no X-Inertia-Error-Bag header) must surface
        // errors FLAT (`errors.email`), not nested under the bag name
        // (`errors.default.email`) - otherwise the Inertia client's
        // `$errors.email` binding comes up undefined.
        let req = MockReq::new("/").inertia();
        let resp = InertiaResponse::new("Login").resolve(&req).await.unwrap();
        let body = body_to_string(resp.into_hyper().into_body());
        let page: serde_json::Value = serde_json::from_str(&body).unwrap();
        let errors = page["props"]["errors"].as_object().unwrap();
        assert_eq!(
            errors["email"], "Invalid",
            "default-bag errors must be flat, and a field's value is its first message \
             (Inertia's `ErrorValue` is `string`); got {errors:?}"
        );
        assert!(
            !errors.contains_key("default"),
            "default bag must be flattened, not nested under \"default\"; got {errors:?}"
        );
    })
    .await;
}

// ---- X-Inertia-Reset header ----
//
// When the client sends X-Inertia-Reset with merge-prop key names, the
// server resolves those props normally but suppresses the merge
// metadata so the client treats the response as a fresh replacement
// (not an append).

#[tokio::test]
async fn x_inertia_reset_strips_merge_metadata() {
    let req = MockReq::new("/posts")
        .inertia()
        .header("X-Inertia-Partial-Component", "Posts/Index")
        .header("X-Inertia-Partial-Data", "posts")
        .header("X-Inertia-Reset", "posts");
    let resp = InertiaResponse::new("Posts/Index")
        .merge("posts", serde_json::json!([{"id": 1}]))
        .resolve(&req)
        .await
        .unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();

    // Value is present
    assert_eq!(page["props"]["posts"], serde_json::json!([{"id": 1}]));
    // …but merge metadata is suppressed because client asked for reset.
    let obj = page.as_object().unwrap();
    let merge_props = obj
        .get("mergeProps")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let names: Vec<&str> = merge_props.iter().filter_map(|v| v.as_str()).collect();
    assert!(
        !names.contains(&"posts"),
        "reset key must NOT appear in mergeProps"
    );
}

#[tokio::test]
async fn x_inertia_reset_does_not_affect_non_reset_merges() {
    let req = MockReq::new("/posts")
        .inertia()
        .header("X-Inertia-Partial-Component", "Posts/Index")
        .header("X-Inertia-Partial-Data", "posts,comments")
        .header("X-Inertia-Reset", "comments");
    let resp = InertiaResponse::new("Posts/Index")
        .merge("posts", serde_json::json!([{"id": 1}]))
        .merge("comments", serde_json::json!([{"id": 2}]))
        .resolve(&req)
        .await
        .unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();

    let merge_props: Vec<&str> = page["mergeProps"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(merge_props.contains(&"posts"));
    assert!(!merge_props.contains(&"comments"));
}

#[tokio::test]
async fn x_inertia_reset_empty_header_is_noop() {
    let req = MockReq::new("/posts")
        .inertia()
        .header("X-Inertia-Partial-Component", "Posts/Index")
        .header("X-Inertia-Partial-Data", "posts")
        .header("X-Inertia-Reset", "");
    let resp = InertiaResponse::new("Posts/Index")
        .merge("posts", serde_json::json!([{"id": 1}]))
        .resolve(&req)
        .await
        .unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();

    let merge_props: Vec<&str> = page["mergeProps"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(merge_props.contains(&"posts"));
}

// ---- Cross-redirect carry: Redirect::preserve_fragment() round-trip ----
//
// These tests drive the full chain: a `Redirect::preserve_fragment()`
// chainable flashes `_inertia.preserve_fragment` to the session, and
// the next request's `InertiaResponse::resolve()` consumes the flag
// and emits `preserveFragment: true`. Each test scopes the session
// via `session_scope_for_test` (mirroring what `SessionMiddleware`
// does at runtime) so the `task_local!` slot is bound.

#[tokio::test]
async fn redirect_preserve_fragment_flashes_session_flag() {
    use suprnova::Redirect;
    use suprnova::session::{new_session_slot_for_test, session_scope_for_test};

    let slot = new_session_slot_for_test();
    session_scope_for_test(slot.clone(), async {
        let _: suprnova::Response = Redirect::to("/article/new").preserve_fragment().into();
    })
    .await;

    // The chainable should have set a *new* flash entry (before aging).
    let s = slot.lock().unwrap();
    let session = s.as_ref().expect("session present");
    assert!(
        session.has("_flash.new._inertia.preserve_fragment"),
        "expected new-flash entry after Redirect::preserve_fragment() conversion"
    );
}

#[tokio::test]
async fn redirect_route_preserve_fragment_flashes_session_flag() {
    // The `RedirectRouteBuilder::From<...>` impl has a separate code
    // path (it can short-circuit on missing route). Ensure
    // `.preserve_fragment()` flashes on the happy path too - they
    // share a helper but the helper must actually be called from both.
    use suprnova::Redirect;
    use suprnova::routing::register_route_name;
    use suprnova::session::{new_session_slot_for_test, session_scope_for_test};

    // Register a route with a unique name so this test doesn't collide
    // with other tests touching the process-global route registry.
    register_route_name(
        "_test_redirect_preserve_fragment_target",
        "/test/article/new",
    );

    let slot = new_session_slot_for_test();
    session_scope_for_test(slot.clone(), async {
        let resp: suprnova::Response = Redirect::route("_test_redirect_preserve_fragment_target")
            .preserve_fragment()
            .into();
        assert!(resp.is_ok(), "route should resolve");
    })
    .await;

    let s = slot.lock().unwrap();
    let session = s.as_ref().expect("session present");
    assert!(
        session.has("_flash.new._inertia.preserve_fragment"),
        "RedirectRouteBuilder::preserve_fragment must flash the same key as Redirect::preserve_fragment"
    );
}

#[tokio::test]
async fn redirect_route_missing_does_not_flash() {
    // When the route doesn't exist, From<RedirectRouteBuilder> returns
    // a 500 Err. Skipping the flash is intentional - otherwise a stray
    // `_inertia.preserve_fragment` would attach to whatever page the
    // user navigates to next.
    use suprnova::Redirect;
    use suprnova::session::{new_session_slot_for_test, session_scope_for_test};

    let slot = new_session_slot_for_test();
    session_scope_for_test(slot.clone(), async {
        let resp: suprnova::Response = Redirect::route("_test_nonexistent_route_xyz")
            .preserve_fragment()
            .into();
        assert!(resp.is_err(), "missing route should yield Err");
    })
    .await;

    let s = slot.lock().unwrap();
    let session = s.as_ref().expect("session present");
    assert!(
        !session.has("_flash.new._inertia.preserve_fragment"),
        "missing-route 500 must NOT flash a stray preserve-fragment"
    );
}

#[tokio::test]
async fn inertia_resolve_picks_up_flashed_preserve_fragment() {
    use suprnova::session::{new_session_slot_for_test, session_scope_for_test};

    let slot = new_session_slot_for_test();
    // Pre-populate as if a previous request flashed it and the session
    // middleware aged it (moving `_flash.new.*` → `_flash.old.*`).
    {
        let mut g = slot.lock().unwrap();
        let s = g.as_mut().unwrap();
        s.put("_flash.old._inertia.preserve_fragment", true);
    }
    let req = MockReq::new("/article/new").inertia();
    let page: serde_json::Value = session_scope_for_test(slot.clone(), async move {
        let resp = InertiaResponse::new("Article/Show")
            .resolve(&req)
            .await
            .unwrap();
        let body = body_to_string(resp.into_hyper().into_body());
        serde_json::from_str(&body).unwrap()
    })
    .await;
    assert_eq!(page["preserveFragment"], true);
}

#[tokio::test]
async fn per_response_false_defeats_flashed_true() {
    // Advisor's critical negative test: explicit `preserve_fragment(false)`
    // on the destination must override a flashed `true` from a redirect.
    use suprnova::session::{new_session_slot_for_test, session_scope_for_test};

    let slot = new_session_slot_for_test();
    {
        let mut g = slot.lock().unwrap();
        let s = g.as_mut().unwrap();
        s.put("_flash.old._inertia.preserve_fragment", true);
    }
    let req = MockReq::new("/article").inertia();
    let page: serde_json::Value = session_scope_for_test(slot.clone(), async move {
        let resp = InertiaResponse::new("Article/Show")
            .preserve_fragment(false)
            .resolve(&req)
            .await
            .unwrap();
        let body = body_to_string(resp.into_hyper().into_body());
        serde_json::from_str(&body).unwrap()
    })
    .await;
    assert!(
        !page.as_object().unwrap().contains_key("preserveFragment"),
        "preserve_fragment(false) must defeat a flashed true"
    );
}

#[tokio::test]
async fn flashed_preserve_fragment_is_one_shot() {
    // After one Inertia response consumes the flashed flag, the next
    // response in the same session must NOT see it again.
    use suprnova::session::{new_session_slot_for_test, session_scope_for_test};

    let slot = new_session_slot_for_test();
    {
        let mut g = slot.lock().unwrap();
        let s = g.as_mut().unwrap();
        s.put("_flash.old._inertia.preserve_fragment", true);
    }

    // First resolve consumes the flash.
    let req1 = MockReq::new("/article").inertia();
    let page1: serde_json::Value = session_scope_for_test(slot.clone(), async move {
        let resp = InertiaResponse::new("Article/Show")
            .resolve(&req1)
            .await
            .unwrap();
        let body = body_to_string(resp.into_hyper().into_body());
        serde_json::from_str(&body).unwrap()
    })
    .await;
    assert_eq!(page1["preserveFragment"], true);

    // Second resolve sees nothing (same session, but flash was drained).
    let req2 = MockReq::new("/article").inertia();
    let page2: serde_json::Value = session_scope_for_test(slot.clone(), async move {
        let resp = InertiaResponse::new("Article/Show")
            .resolve(&req2)
            .await
            .unwrap();
        let body = body_to_string(resp.into_hyper().into_body());
        serde_json::from_str(&body).unwrap()
    })
    .await;
    assert!(
        !page2.as_object().unwrap().contains_key("preserveFragment"),
        "second resolve must not see a re-emitted preserveFragment"
    );
}

#[tokio::test]
async fn no_session_scope_silently_drops_preserve_fragment_flash() {
    // Defensive: Redirect::preserve_fragment() outside a session scope
    // is a documented no-op. It must not panic. The destination
    // response (also outside session scope) sees no flag.
    use suprnova::Redirect;

    let _: suprnova::Response = Redirect::to("/x").preserve_fragment().into();
    let req = MockReq::new("/x").inertia();
    let resp = InertiaResponse::new("X").resolve(&req).await.unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(!page.as_object().unwrap().contains_key("preserveFragment"));
}

#[tokio::test]
async fn three_browser_history_flags_combine_without_coupling() {
    // encryptHistory, clearHistory, preserveFragment are independent
    // top-level fields. Setting all three should emit all three with
    // value `true` and not interfere with each other.
    let req = MockReq::new("/secure").inertia();
    let resp = InertiaResponse::new("Secure/Page")
        .encrypt_history(true)
        .clear_history()
        .preserve_fragment(true)
        .resolve(&req)
        .await
        .unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(page["encryptHistory"], true);
    assert_eq!(page["clearHistory"], true);
    assert_eq!(page["preserveFragment"], true);
}

// ---- version-mismatch middleware ----
//
// These tests drive the middleware directly via the Middleware trait
// rather than booting a Server. They construct a `Next` closure that
// either captures whether it was called (proceed) or returns a sentinel
// response (so the test can tell pass-through from short-circuit).

mod version_mw {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use suprnova::{HttpResponse, InertiaResponse, InertiaVersionMiddleware, Middleware};

    /// Build a `Next` that records whether it was invoked and returns a
    /// trivial 200 response when called.
    fn passthrough_next() -> (Arc<AtomicBool>, suprnova::Next) {
        let flag = Arc::new(AtomicBool::new(false));
        let f = flag.clone();
        let next: suprnova::Next = Arc::new(move |_req| {
            let f = f.clone();
            Box::pin(async move {
                f.store(true, Ordering::SeqCst);
                Ok(HttpResponse::text("through"))
            })
        });
        (flag, next)
    }

    // Test note: full Request construction requires `hyper::body::Incoming`
    // which can't be built outside hyper. The middleware tests therefore
    // live in this submodule with a dedicated runner that exercises the
    // middleware's logic against the actual `Request` type through a
    // minimal hyper service setup. We use `hyper::Request::builder()`
    //   + `http_body_util::Empty` as the body, then convert via
    // `Request::new` after collecting a wrapped Incoming.
    //
    // Since hyper doesn't expose a way to construct Incoming, we
    // instead test the middleware behavior end-to-end by binding a
    // tokio TCP listener on `127.0.0.1:0` and sending real HTTP
    // requests through a hyper client.
    //
    // That setup is heavier than the direct invocation pattern used for
    // the rest of these integration tests, so we use a separate test
    // fixture below.
    use http_body_util::{BodyExt, Empty};
    use hyper::body::Bytes;
    use hyper::server::conn::http1;
    use hyper::service::service_fn;
    use hyper_util::rt::TokioIo;
    use std::convert::Infallible;
    use std::net::SocketAddr;

    /// Boot a one-shot HTTP server that wraps the given middleware around
    /// a fixed "fallthrough" handler, send an HTTP request to it, return
    /// the response.
    async fn drive(
        mw: InertiaVersionMiddleware,
        req: hyper::Request<Empty<Bytes>>,
    ) -> hyper::Response<Bytes> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();

        let mw = Arc::new(mw);

        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let io = TokioIo::new(stream);
            let mw = mw.clone();
            let service = service_fn(move |hyper_req: hyper::Request<hyper::body::Incoming>| {
                let mw = mw.clone();
                async move {
                    let req = suprnova::Request::new(hyper_req);
                    let (_flag, next) = passthrough_next();
                    let response = mw.handle(req, next).await;
                    let http = response.unwrap_or_else(|e| e);
                    Ok::<_, Infallible>(http.into_hyper())
                }
            });
            http1::Builder::new()
                .serve_connection(io, service)
                .await
                .ok();
        });

        // Build the request via hyper client.
        let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let io = TokioIo::new(stream);
        let (mut sender, conn) = hyper::client::conn::http1::handshake::<_, Empty<Bytes>>(io)
            .await
            .unwrap();
        tokio::spawn(async move {
            let _ = conn.await;
        });

        let req = req;
        let resp = sender.send_request(req).await.unwrap();
        let (parts, body) = resp.into_parts();
        let collected = body.collect().await.unwrap();
        hyper::Response::from_parts(parts, collected.to_bytes())
    }

    fn request(
        method: &str,
        version_header: Option<&str>,
        inertia: bool,
    ) -> hyper::Request<Empty<Bytes>> {
        request_with_uri(method, version_header, inertia, "http://localhost/users")
    }

    fn request_with_uri(
        method: &str,
        version_header: Option<&str>,
        inertia: bool,
        uri: &str,
    ) -> hyper::Request<Empty<Bytes>> {
        let mut b = hyper::Request::builder().method(method).uri(uri);
        if inertia {
            b = b.header("X-Inertia", "true");
        }
        if let Some(v) = version_header {
            b = b.header("X-Inertia-Version", v);
        }
        b.body(Empty::<Bytes>::new()).unwrap()
    }

    // Sentinel: when the middleware proceeds, the handler returns "through".
    // When it short-circuits with 409, the body is empty.
    fn _sentinel() -> &'static str {
        "through"
    }

    #[tokio::test]
    async fn matching_version_passes_through() {
        let mw = InertiaVersionMiddleware::new("v1");
        let resp = drive(mw, request("GET", Some("v1"), true)).await;
        assert_eq!(resp.status(), 200);
        assert_eq!(resp.body().as_ref(), b"through");
    }

    #[tokio::test]
    async fn mismatched_version_on_inertia_get_returns_409_with_location() {
        let mw = InertiaVersionMiddleware::new("v2");
        let resp = drive(mw, request("GET", Some("v1"), true)).await;
        assert_eq!(resp.status(), 409);
        let location = resp
            .headers()
            .get("X-Inertia-Location")
            .expect("X-Inertia-Location header");
        assert_eq!(location, "/users");
    }

    #[tokio::test]
    async fn mismatched_version_on_inertia_post_passes_through() {
        // Per spec, only GET mismatches trigger 409 - other methods rely on
        // their post-action GET redirect to surface the mismatch.
        let mw = InertiaVersionMiddleware::new("v2");
        let resp = drive(mw, request("POST", Some("v1"), true)).await;
        assert_eq!(resp.status(), 200);
        assert_eq!(resp.body().as_ref(), b"through");
    }

    #[tokio::test]
    async fn non_inertia_request_passes_through_even_with_version_mismatch() {
        let mw = InertiaVersionMiddleware::new("v2");
        let resp = drive(mw, request("GET", Some("v1"), false)).await;
        assert_eq!(resp.status(), 200);
        assert_eq!(resp.body().as_ref(), b"through");
    }

    #[tokio::test]
    async fn missing_version_header_on_inertia_get_is_treated_as_mismatch() {
        // Per spec, the client should always send X-Inertia-Version with
        // an Inertia request. A missing header is effectively an empty
        // version string, which doesn't match a configured non-empty one.
        let mw = InertiaVersionMiddleware::new("v1");
        let resp = drive(mw, request("GET", None, true)).await;
        assert_eq!(resp.status(), 409);
    }

    #[tokio::test]
    async fn missing_version_header_matches_empty_configured_version() {
        // Reverse case: server has empty version (default unset), client
        // sends no header. They match.
        let mw = InertiaVersionMiddleware::new("");
        let resp = drive(mw, request("GET", None, true)).await;
        assert_eq!(resp.status(), 200);
    }

    #[tokio::test]
    async fn mismatched_version_preserves_query_string_in_location() {
        // Asset-version 409 must redirect back to the SAME URL - the
        // query string carries pagination cursors, search terms, and
        // form-submitted GET params. Dropping it on every mismatch
        // silently kicks users to page 1 / empty search after every
        // deploy.
        let mw = InertiaVersionMiddleware::new("v2");
        let resp = drive(
            mw,
            request_with_uri(
                "GET",
                Some("v1"),
                true,
                "http://localhost/users?page=3&q=alice",
            ),
        )
        .await;
        assert_eq!(resp.status(), 409);
        let location = resp
            .headers()
            .get("X-Inertia-Location")
            .expect("X-Inertia-Location header");
        assert_eq!(location, "/users?page=3&q=alice");
    }

    /// Boot a one-shot HTTP server that resolves an `InertiaResponse`
    /// against a REAL `crate::http::Request` for the given URI, and
    /// return the page object's `url` field. Mirrors `drive`'s server
    /// plumbing - a real `hyper::body::Incoming` can only be constructed
    /// through hyper's own connection machinery, so this is the only way
    /// to exercise `InertiaRequestExt::path_and_query`'s `Request` impl
    /// (rather than `MockReq`'s hand-written stand-in) from a test.
    async fn resolve_page_url(uri: &str) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let io = TokioIo::new(stream);
            let service = service_fn(
                move |hyper_req: hyper::Request<hyper::body::Incoming>| async move {
                    let req = suprnova::Request::new(hyper_req);
                    let resp = InertiaResponse::new("Users").resolve(&req).await.unwrap();
                    Ok::<_, Infallible>(resp.into_hyper())
                },
            );
            http1::Builder::new()
                .serve_connection(io, service)
                .await
                .ok();
        });

        let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let io = TokioIo::new(stream);
        let (mut sender, conn) = hyper::client::conn::http1::handshake::<_, Empty<Bytes>>(io)
            .await
            .unwrap();
        tokio::spawn(async move {
            let _ = conn.await;
        });

        let req = hyper::Request::builder()
            .method("GET")
            .uri(uri)
            .header("X-Inertia", "true")
            .body(Empty::<Bytes>::new())
            .unwrap();
        let resp = sender.send_request(req).await.unwrap();
        let (_, body) = resp.into_parts();
        let bytes = body.collect().await.unwrap().to_bytes();
        let page: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        page["url"]
            .as_str()
            .expect("page.url is a string")
            .to_string()
    }

    #[tokio::test]
    async fn page_url_and_the_version_bounce_url_agree_on_a_real_request() {
        // A 409 version bounce and the page object it bounces to must name
        // the same URL, query string included; otherwise a stale-asset
        // reload lands on page 1 while the page object still says page 3.
        // Both derive from `InertiaRequestExt::path_and_query`, and this
        // drives each through a real request for the same URI so any drift
        // between them fails here, not in a browser.
        let uri = "http://localhost/users?page=3&q=alice";

        // (a) the version-mismatch bounce.
        let mw = InertiaVersionMiddleware::new("v2");
        let resp = drive(mw, request_with_uri("GET", Some("v1"), true, uri)).await;
        assert_eq!(resp.status(), 409);
        let location = resp
            .headers()
            .get("X-Inertia-Location")
            .expect("X-Inertia-Location header")
            .to_str()
            .unwrap()
            .to_string();
        assert_eq!(location, "/users?page=3&q=alice");

        // (b) the Inertia page object's `url`, for the same URI.
        let page_url = resolve_page_url(uri).await;
        assert_eq!(page_url, "/users?page=3&q=alice");

        // (c) byte-for-byte agreement.
        assert_eq!(location, page_url);
    }
}

// ---- Cross-redirect flash persistence: App::flash + Redirect ----
//
// These exercise the full chain so that `App::flash` survives an
// outgoing redirect: the redirect conversion transfers the task-local
// bag into the session, the receiving request's session ages it into
// `_flash.old.*`, and `InertiaResponse::resolve` merges it into the
// page object's top-level `flash` field. Without the session bridge
// the flash would only appear on the *current* response - the
// finding's "silently no-ops outside the task-local request bag"
// premise.

#[tokio::test]
async fn app_flash_survives_redirect_into_session() {
    use suprnova::Redirect;
    use suprnova::session::{new_session_slot_for_test, session_scope_for_test};

    let slot = new_session_slot_for_test();
    let bag = suprnova::inertia::flash_new_bag_for_test();
    session_scope_for_test(slot.clone(), async {
        suprnova::inertia::flash_scope_for_test(bag, async {
            suprnova::App::flash("status", serde_json::json!("Saved!"));
            // Convert a Redirect → Response. The conversion should
            // bridge the task-local bag into the session as
            // `_flash.new.*`.
            let _: suprnova::Response = Redirect::to("/dashboard").into();
        })
        .await;
    })
    .await;

    let s = slot.lock().unwrap();
    let session = s.as_ref().expect("session present");
    assert!(
        session.has("_flash.new.status"),
        "App::flash should be bridged into session as _flash.new.status"
    );
}

#[tokio::test]
async fn app_flash_redirect_destination_sees_flash_in_page() {
    // Full round-trip: request A flashes via App::flash + Redirect,
    // session middleware ages on request B, request B's
    // InertiaResponse surfaces the value under page.flash.
    use suprnova::Redirect;
    use suprnova::session::{new_session_slot_for_test, session_scope_for_test};

    let slot = new_session_slot_for_test();

    // Request A: flash + redirect.
    let bag = suprnova::inertia::flash_new_bag_for_test();
    session_scope_for_test(slot.clone(), async {
        suprnova::inertia::flash_scope_for_test(bag, async {
            suprnova::App::flash("status", serde_json::json!("Saved!"));
            let _: suprnova::Response = Redirect::to("/dashboard").into();
        })
        .await;
    })
    .await;

    // Session middleware on the receiving request ages flash.
    {
        let mut g = slot.lock().unwrap();
        let s = g.as_mut().unwrap();
        s.age_flash_data();
    }

    // Request B: Inertia response surfaces the value under page.flash.
    let req = MockReq::new("/dashboard").inertia();
    let bag_b = suprnova::inertia::flash_new_bag_for_test();
    let page: serde_json::Value = session_scope_for_test(slot.clone(), async move {
        suprnova::inertia::flash_scope_for_test(bag_b, async move {
            let resp = InertiaResponse::new("Dashboard")
                .resolve(&req)
                .await
                .unwrap();
            let body = body_to_string(resp.into_hyper().into_body());
            serde_json::from_str(&body).unwrap()
        })
        .await
    })
    .await;

    assert_eq!(page["flash"]["status"], serde_json::json!("Saved!"));
}

#[tokio::test]
async fn app_flash_no_redirect_does_not_leak_to_next_request() {
    // Non-redirect path: App::flash should appear on THIS response
    // (existing semantics) and must NOT persist into the session,
    // otherwise the next request sees a stale flash. This is the
    // anti-test for the "write to both at flash time" foot-gun.
    use suprnova::session::{new_session_slot_for_test, session_scope_for_test};

    let slot = new_session_slot_for_test();

    // Request A: flash + non-redirect Inertia response.
    let req_a = MockReq::new("/").inertia();
    let bag_a = suprnova::inertia::flash_new_bag_for_test();
    let page_a: serde_json::Value = session_scope_for_test(slot.clone(), async {
        suprnova::inertia::flash_scope_for_test(bag_a, async move {
            suprnova::App::flash("toast", serde_json::json!("hi"));
            let resp = InertiaResponse::new("Home").resolve(&req_a).await.unwrap();
            let body = body_to_string(resp.into_hyper().into_body());
            serde_json::from_str(&body).unwrap()
        })
        .await
    })
    .await;
    assert_eq!(page_a["flash"]["toast"], serde_json::json!("hi"));

    // Session middleware on the next request ages flash. Nothing
    // should have been written so age is a no-op.
    {
        let mut g = slot.lock().unwrap();
        let s = g.as_mut().unwrap();
        s.age_flash_data();
        assert!(
            !s.has("_flash.old.toast"),
            "App::flash without redirect must not bleed into session - \
             otherwise it would appear on every subsequent unrelated request"
        );
    }

    // Request B: fresh task-local bag, no flash should appear.
    let req_b = MockReq::new("/").inertia();
    let bag_b = suprnova::inertia::flash_new_bag_for_test();
    let page_b: serde_json::Value = session_scope_for_test(slot.clone(), async move {
        suprnova::inertia::flash_scope_for_test(bag_b, async move {
            let resp = InertiaResponse::new("Home").resolve(&req_b).await.unwrap();
            let body = body_to_string(resp.into_hyper().into_body());
            serde_json::from_str(&body).unwrap()
        })
        .await
    })
    .await;
    assert!(
        !page_b.as_object().unwrap().contains_key("flash"),
        "follow-up request must not see the non-redirected flash"
    );
}

#[tokio::test]
async fn redirect_with_flash_surfaces_in_destination_page() {
    // `Redirect::with(...)` already wrote to the session in
    // `drain_flash`. The complementary half - `InertiaResponse::resolve`
    // reading session `_flash.old.*` - must now surface it.
    use suprnova::Redirect;
    use suprnova::session::{new_session_slot_for_test, session_scope_for_test};

    let slot = new_session_slot_for_test();

    session_scope_for_test(slot.clone(), async {
        let _: suprnova::Response = Redirect::to("/dashboard")
            .with("status", serde_json::json!("Updated."))
            .into();
    })
    .await;

    // Age - emulating the receiving request's SessionMiddleware.
    {
        let mut g = slot.lock().unwrap();
        let s = g.as_mut().unwrap();
        s.age_flash_data();
    }

    let req = MockReq::new("/dashboard").inertia();
    let bag = suprnova::inertia::flash_new_bag_for_test();
    let page: serde_json::Value = session_scope_for_test(slot.clone(), async move {
        suprnova::inertia::flash_scope_for_test(bag, async move {
            let resp = InertiaResponse::new("Dashboard")
                .resolve(&req)
                .await
                .unwrap();
            let body = body_to_string(resp.into_hyper().into_body());
            serde_json::from_str(&body).unwrap()
        })
        .await
    })
    .await;

    assert_eq!(page["flash"]["status"], serde_json::json!("Updated."));
}

#[tokio::test]
async fn session_flash_filters_internal_keys() {
    // `_old_input` (form repopulation) and `_inertia.*` (protocol
    // flags) ride in the same session flash queue but must NOT leak
    // into page.flash, which is user-facing.
    use suprnova::session::{new_session_slot_for_test, session_scope_for_test};

    let slot = new_session_slot_for_test();
    {
        let mut g = slot.lock().unwrap();
        let s = g.as_mut().unwrap();
        s.put("_flash.old.status", serde_json::json!("ok"));
        s.put("_flash.old._old_input", serde_json::json!({"email": "x"}));
        s.put("_flash.old._inertia.preserve_fragment", true);
    }

    let req = MockReq::new("/dashboard").inertia();
    let bag = suprnova::inertia::flash_new_bag_for_test();
    let page: serde_json::Value = session_scope_for_test(slot.clone(), async move {
        suprnova::inertia::flash_scope_for_test(bag, async move {
            let resp = InertiaResponse::new("Dashboard")
                .resolve(&req)
                .await
                .unwrap();
            let body = body_to_string(resp.into_hyper().into_body());
            serde_json::from_str(&body).unwrap()
        })
        .await
    })
    .await;

    let flash = page["flash"].as_object().expect("flash object");
    assert_eq!(flash.get("status"), Some(&serde_json::json!("ok")));
    assert!(
        !flash.contains_key("_old_input"),
        "internal _old_input key must not leak into page.flash"
    );
    assert!(
        !flash.contains_key("_inertia.preserve_fragment"),
        "internal _inertia.* keys must not leak into page.flash"
    );
}

#[tokio::test]
async fn same_request_flash_wins_over_session_flash_on_collision() {
    // Precedence contract: a destination controller can override
    // an inherited flash value by re-flashing the same key. Order is
    // session-old < task-local < builder.
    use suprnova::session::{new_session_slot_for_test, session_scope_for_test};

    let slot = new_session_slot_for_test();
    {
        let mut g = slot.lock().unwrap();
        let s = g.as_mut().unwrap();
        s.put(
            "_flash.old.status",
            serde_json::json!("from previous request"),
        );
    }

    let req = MockReq::new("/dashboard").inertia();
    let bag = suprnova::inertia::flash_new_bag_for_test();
    let page: serde_json::Value = session_scope_for_test(slot.clone(), async move {
        suprnova::inertia::flash_scope_for_test(bag, async move {
            // Same-key flash on the destination handler - should win.
            let resp = InertiaResponse::new("Dashboard")
                .flash("status", serde_json::json!("overridden"))
                .resolve(&req)
                .await
                .unwrap();
            let body = body_to_string(resp.into_hyper().into_body());
            serde_json::from_str(&body).unwrap()
        })
        .await
    })
    .await;

    assert_eq!(page["flash"]["status"], serde_json::json!("overridden"));
}

#[tokio::test]
async fn app_flash_without_session_scope_stays_one_shot() {
    // No session scope around the Redirect conversion → bridge is a
    // no-op (documented). The flash appears on the same-request
    // response only; subsequent requests see nothing. This is the
    // documented degraded-mode behaviour for routes outside session
    // middleware.
    use suprnova::Redirect;

    let bag = suprnova::inertia::flash_new_bag_for_test();
    suprnova::inertia::flash_scope_for_test(bag, async {
        suprnova::App::flash("status", serde_json::json!("ephemeral"));
        let _: suprnova::Response = Redirect::to("/x").into();
        // The bag was drained on the redirect conversion.
    })
    .await;

    // A fresh request after the redirect - no session anywhere - must
    // not see the value.
    let req = MockReq::new("/x").inertia();
    let bag2 = suprnova::inertia::flash_new_bag_for_test();
    let page: serde_json::Value = suprnova::inertia::flash_scope_for_test(bag2, async move {
        let resp = InertiaResponse::new("X").resolve(&req).await.unwrap();
        let body = body_to_string(resp.into_hyper().into_body());
        serde_json::from_str(&body).unwrap()
    })
    .await;
    assert!(
        !page.as_object().unwrap().contains_key("flash"),
        "without a session scope, App::flash can't cross a redirect - \
         destination must not see it"
    );
}

// ---- page.url keeps the query string (Laravel Response::getUrl) ----

#[tokio::test]
async fn page_url_keeps_the_query_string() {
    // Without the query string the client's history entry, back/forward
    // navigation, and `router.reload()` all replay `/users` instead of
    // `/users?page=2&sort=name` - pagination and filters silently reset.
    let req = MockReq::new("/users").query("page=2&sort=name").inertia();
    let resp = InertiaResponse::new("Users")
        .with("users", serde_json::json!([]))
        .resolve(&req)
        .await
        .unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(page["url"], "/users?page=2&sort=name");
}

#[tokio::test]
async fn page_url_without_a_query_string_is_just_the_path() {
    let req = MockReq::new("/users").inertia();
    let resp = InertiaResponse::new("Users").resolve(&req).await.unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(page["url"], "/users");
}

#[tokio::test]
async fn url_resolver_overrides_the_default() {
    // Laravel's `Inertia::resolveUrlUsing`. Typical use: strip a locale
    // prefix, or canonicalise a proxy-rewritten path.
    let cfg =
        InertiaConfig::new().url_resolver(|req| format!("/canonical{}", req.path_and_query()));
    let req = MockReq::new("/users").query("page=2").inertia();
    let resp = InertiaResponse::new("Users")
        .with_config(cfg)
        .resolve(&req)
        .await
        .unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(page["url"], "/canonical/users?page=2");
}

#[test]
fn mock_req_path_and_query_matches_hyper_uris_derivation() {
    // Sanity check on the test double, not a pin on production behaviour:
    // confirms `MockReq::path_and_query()` builds the same string
    // `hyper::Uri::path_and_query()` would for the same URI, so the
    // MockReq-based tests above are exercising a faithful stand-in.
    // The real pin - that `InertiaVersionMiddleware`'s `X-Inertia-Location`
    // and `InertiaResponse::resolve`'s `page.url` agree byte-for-byte
    // through a REAL `crate::http::Request` - lives in
    // `version_mw::page_url_and_the_version_bounce_url_agree_on_a_real_request`,
    // since both now derive their string through the single
    // `InertiaRequestExt::path_and_query` implementation on `Request`
    // (version_middleware.rs, prop.rs).
    let uri: hyper::Uri = "http://localhost/users?page=2&sort=name".parse().unwrap();
    let from_uri = uri
        .path_and_query()
        .map(|pq| pq.as_str().to_string())
        .expect("uri has a path");
    let mock = MockReq::new(uri.path()).query(uri.query().unwrap());
    assert_eq!(mock.path_and_query(), from_uri);
    assert_eq!(from_uri, "/users?page=2&sort=name");
}

// ---- location: 409 for Inertia, 302 for everyone else ----

#[tokio::test]
async fn location_for_returns_409_on_an_inertia_request() {
    let req = MockReq::new("/billing").inertia();
    let resp = InertiaResponse::location_for(&req, "https://billing.example/checkout");
    let hyper_resp = resp.into_hyper();
    assert_eq!(hyper_resp.status(), 409);
    assert_eq!(
        hyper_resp.headers().get("X-Inertia-Location").unwrap(),
        "https://billing.example/checkout"
    );
    assert!(hyper_resp.headers().get("Location").is_none());
}

#[tokio::test]
async fn location_for_returns_302_on_a_plain_browser_request() {
    // A hard navigation into an OAuth / SSO bounce has no X-Inertia
    // header. A bare 409 with no Location header is a dead end for it -
    // the browser has nowhere to go.
    let req = MockReq::new("/billing");
    let resp = InertiaResponse::location_for(&req, "https://billing.example/checkout");
    let hyper_resp = resp.into_hyper();
    assert_eq!(hyper_resp.status(), 302);
    assert_eq!(
        hyper_resp.headers().get("Location").unwrap(),
        "https://billing.example/checkout"
    );
    assert!(hyper_resp.headers().get("X-Inertia-Location").is_none());
}

#[tokio::test]
async fn location_keeps_its_always_409_shape() {
    // The pre-existing surface is unchanged: `location(url)` is the
    // "I already know this is an Inertia request" form.
    let resp = InertiaResponse::location("https://example.com/external");
    let hyper_resp = resp.into_hyper();
    assert_eq!(hyper_resp.status(), 409);
    assert!(hyper_resp.headers().get("X-Inertia-Location").is_some());
}

// ---- clear_history survives a redirect ----

#[tokio::test]
async fn clear_history_flashed_by_a_previous_request_reaches_the_next_page() {
    // The logout flow: the handler calls `App::clear_history()` and
    // redirects. The redirect's own response is discarded by the browser;
    // the LOGIN page is the one that renders, and it is the page that has
    // to carry `clearHistory: true` - otherwise the previous session's
    // encrypted history entries stay decryptable.
    use suprnova::session::{new_session_slot_for_test, session_scope_for_test};

    let slot = new_session_slot_for_test();
    {
        let mut g = slot.lock().unwrap();
        let s = g.as_mut().unwrap();
        // As if the logout request flashed it and the session middleware
        // aged it into `_flash.old.*`.
        s.put("_flash.old._inertia.clear_history", true);
    }
    let req = MockReq::new("/login").inertia();
    let page: serde_json::Value = session_scope_for_test(slot.clone(), async move {
        let resp = InertiaResponse::new("Auth/Login")
            .resolve(&req)
            .await
            .unwrap();
        let body = body_to_string(resp.into_hyper().into_body());
        serde_json::from_str(&body).unwrap()
    })
    .await;
    assert_eq!(page["clearHistory"], true);
}

#[tokio::test]
async fn flashed_clear_history_is_one_shot() {
    // It must survive exactly one hop. A flag that stuck around would
    // rotate the history key on every navigation and defeat the point of
    // encrypting it at all.
    use suprnova::session::{new_session_slot_for_test, session_scope_for_test};

    let slot = new_session_slot_for_test();
    {
        let mut g = slot.lock().unwrap();
        g.as_mut()
            .unwrap()
            .put("_flash.old._inertia.clear_history", true);
    }

    let req1 = MockReq::new("/login").inertia();
    let page1: serde_json::Value = session_scope_for_test(slot.clone(), async move {
        let resp = InertiaResponse::new("Auth/Login")
            .resolve(&req1)
            .await
            .unwrap();
        serde_json::from_str(&body_to_string(resp.into_hyper().into_body())).unwrap()
    })
    .await;
    assert_eq!(page1["clearHistory"], true);

    let req2 = MockReq::new("/dashboard").inertia();
    let page2: serde_json::Value = session_scope_for_test(slot.clone(), async move {
        let resp = InertiaResponse::new("Dashboard")
            .resolve(&req2)
            .await
            .unwrap();
        serde_json::from_str(&body_to_string(resp.into_hyper().into_body())).unwrap()
    })
    .await;
    assert!(
        !page2.as_object().unwrap().contains_key("clearHistory"),
        "the flag must not survive a second hop"
    );
}

#[tokio::test]
async fn app_clear_history_flashes_into_the_session() {
    use suprnova::App;
    use suprnova::session::{new_session_slot_for_test, session_scope_for_test};

    let slot = new_session_slot_for_test();
    session_scope_for_test(slot.clone(), async {
        App::clear_history();
    })
    .await;

    let g = slot.lock().unwrap();
    let s = g.as_ref().unwrap();
    assert!(
        s.has("_flash.new._inertia.clear_history"),
        "App::clear_history must flash for the NEXT request"
    );
}

#[tokio::test]
async fn per_response_clear_history_still_works_without_a_session() {
    // The existing surface is unchanged and does not depend on a session.
    let req = MockReq::new("/settings").inertia();
    let resp = InertiaResponse::new("Settings")
        .clear_history()
        .resolve(&req)
        .await
        .unwrap();
    let page: serde_json::Value =
        serde_json::from_str(&body_to_string(resp.into_hyper().into_body())).unwrap();
    assert_eq!(page["clearHistory"], true);
}

#[tokio::test]
async fn clear_history_flashed_after_a_session_flush_survives_the_flush() {
    // Documents the required ordering: `Auth::logout_and_invalidate()`
    // rotates the session id and flushes the whole session -
    // `session_mut(|s| s.flush())` below stands in for that call, since
    // exercising the real `Auth::logout_and_invalidate` needs a user
    // provider and a database, which this test harness doesn't have.
    // `App::clear_history()` must run AFTER the flush: its flag lives in
    // the very session the flush just cleared, so flashing before the
    // flush would erase the flag before it ever reaches the next page.
    use suprnova::App;
    use suprnova::session::{new_session_slot_for_test, session_mut, session_scope_for_test};

    let slot = new_session_slot_for_test();
    session_scope_for_test(slot.clone(), async {
        session_mut(|s| s.flush());
        App::clear_history();
        // What `SessionMiddleware` does at the end of a request: move
        // `_flash.new.*` into `_flash.old.*` for the next request to read.
        session_mut(|s| s.age_flash_data());
    })
    .await;

    let req = MockReq::new("/login").inertia();
    let page: serde_json::Value = session_scope_for_test(slot.clone(), async move {
        let resp = InertiaResponse::new("Auth/Login")
            .resolve(&req)
            .await
            .unwrap();
        serde_json::from_str(&body_to_string(resp.into_hyper().into_body())).unwrap()
    })
    .await;
    assert_eq!(
        page["clearHistory"], true,
        "clear_history() called after flush() must still reach the next page"
    );
}

#[tokio::test]
async fn clear_history_outside_a_session_scope_is_a_harmless_no_op() {
    // Unlike its neighbour `App::flash`, which warns and drops on a
    // serialise failure, `App::clear_history` used to fail silently
    // outside a session scope. It now logs a warning too (see the
    // no-session arm's `tracing::warn!`), but the call must still not
    // panic - a caller with no `SessionMiddleware` in the stack should
    // not crash the request.
    suprnova::App::clear_history();
}

// ---- once props are skipped only on a non-partial Inertia visit ----

#[tokio::test]
async fn a_once_prop_resolves_on_an_explicit_partial_reload() {
    // `router.reload({ only: ['stats'] })` is the user explicitly asking
    // for this prop. Honouring the client's "I have it cached" claim
    // there returns literally nothing for the key the client just asked
    // for. Laravel gates the skip on `isInertia && !isPartial`
    // (Response.php:307).
    let req = MockReq::new("/dashboard")
        .inertia()
        .header("X-Inertia-Partial-Component", "Dashboard")
        .header("X-Inertia-Partial-Data", "stats")
        .header("X-Inertia-Except-Once-Props", "stats");
    let resp = InertiaResponse::new("Dashboard")
        .once("stats", || async {
            Ok::<_, suprnova::FrameworkError>(serde_json::json!({ "visits": 7 }))
        })
        .resolve(&req)
        .await
        .unwrap();
    let page: serde_json::Value =
        serde_json::from_str(&body_to_string(resp.into_hyper().into_body())).unwrap();
    assert_eq!(
        page["props"]["stats"]["visits"], 7,
        "an explicitly requested once prop must be resolved, got {page}"
    );
}

#[tokio::test]
async fn a_once_prop_is_still_skipped_on_a_full_inertia_visit() {
    // Regression guard for the gate: the caching behaviour itself is
    // unchanged for a normal (non-partial) visit.
    let req = MockReq::new("/dashboard")
        .inertia()
        .header("X-Inertia-Except-Once-Props", "stats");
    let resp = InertiaResponse::new("Dashboard")
        .once("stats", || async {
            Ok::<_, suprnova::FrameworkError>(serde_json::json!({ "visits": 7 }))
        })
        .resolve(&req)
        .await
        .unwrap();
    let page: serde_json::Value =
        serde_json::from_str(&body_to_string(resp.into_hyper().into_body())).unwrap();
    assert!(
        page["props"].as_object().unwrap().get("stats").is_none(),
        "a cached once prop is not re-sent on a full visit, got {page}"
    );
    assert!(
        page["onceProps"].is_object(),
        "the once metadata still confirms the cache key, got {page}"
    );
}

#[tokio::test]
async fn a_once_prop_is_skipped_on_a_partial_reload_that_does_not_name_it() {
    // An explicit partial reload for a DIFFERENT prop must not resolve -
    // or even invoke - a once prop the client didn't ask for and already
    // claims to have cached.
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let ran = Arc::new(AtomicBool::new(false));
    let ran_for_resolver = ran.clone();

    let req = MockReq::new("/dashboard")
        .inertia()
        .header("X-Inertia-Partial-Component", "Dashboard")
        .header("X-Inertia-Partial-Data", "other")
        .header("X-Inertia-Except-Once-Props", "stats");
    let resp = InertiaResponse::new("Dashboard")
        .with("other", serde_json::json!({ "ok": true }))
        .once("stats", move || {
            let ran_for_resolver = ran_for_resolver.clone();
            async move {
                ran_for_resolver.store(true, Ordering::SeqCst);
                Ok::<_, suprnova::FrameworkError>(serde_json::json!({ "visits": 7 }))
            }
        })
        .resolve(&req)
        .await
        .unwrap();
    let page: serde_json::Value =
        serde_json::from_str(&body_to_string(resp.into_hyper().into_body())).unwrap();
    assert!(
        !ran.load(Ordering::SeqCst),
        "the once resolver must not run when the partial reload didn't ask for it"
    );
    assert!(
        page["props"].as_object().unwrap().get("stats").is_none(),
        "stats must not be included, got {page}"
    );
}

#[tokio::test]
async fn a_once_prop_ignores_the_except_header_on_a_non_inertia_visit() {
    // A hard navigation renders the whole page from scratch; the client
    // has no cache to honour. Laravel's gate starts with `!isInertia`.
    let req = MockReq::new("/dashboard").header("X-Inertia-Except-Once-Props", "stats");
    let resp = InertiaResponse::new("Dashboard")
        .once("stats", || async {
            Ok::<_, suprnova::FrameworkError>(serde_json::json!({ "visits": 7 }))
        })
        .resolve(&req)
        .await
        .unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    assert!(
        body.contains(r#""visits":7"#),
        "the HTML shell must carry the resolved value, got {body}"
    );
}

// ---- Inertia::install retains its config as the response default ----
//
// `install` used to read `development`, `manifest_path` and `version` off
// the config and drop it, so every `InertiaResponse::new` started from a
// fresh `InertiaConfig::default()`. A React app rendered the Svelte entry
// point unless SUPRNOVA_FRONTEND happened to be set, the page object's
// version came from a different config than the version middleware's
// resolver, and SSR "enabled on the config" never reached a response.
//
// Every test here takes a `TestContainer::fake()` guard: `install` writes
// to the active container's Inertia registry, and without the guard that
// write would land on the global registry and change what every other
// test in this binary renders.

#[tokio::test]
async fn installed_config_frontend_reaches_the_html_shell() {
    let _guard = suprnova::testing::TestContainer::fake();
    suprnova::Inertia::install(
        &InertiaConfig::new()
            .frontend(Frontend::React)
            .version("v-installed")
            .development(true),
    )
    .expect("dev-mode install must not require a Vite manifest");

    let req = MockReq::new("/home"); // no X-Inertia header → HTML shell
    let resp = InertiaResponse::new("Home").resolve(&req).await.unwrap();
    let body = body_to_string(resp.into_hyper().into_body());

    assert!(
        body.contains("src/main.tsx"),
        "the installed config says React, so the shell must load React's \
         entry point, not Svelte's src/main.ts:\n{body}"
    );
    assert!(
        body.contains("@react-refresh"),
        "React needs its refresh preamble before any module loads:\n{body}"
    );
    assert!(
        body.contains("__vite_plugin_react_preamble_installed__"),
        "the React preamble must be the complete one:\n{body}"
    );
}

#[tokio::test]
async fn installed_config_version_reaches_the_page_object() {
    // The version middleware bounces a client whose X-Inertia-Version
    // doesn't match the INSTALLED config. If the page object advertises a
    // version from a different config, the client stores that one, sends
    // it back, and is bounced forever.
    let _guard = suprnova::testing::TestContainer::fake();
    suprnova::Inertia::install(
        &InertiaConfig::new()
            .version("v-installed")
            .development(true),
    )
    .expect("dev-mode install must not require a Vite manifest");

    let req = MockReq::new("/").inertia();
    let resp = InertiaResponse::new("Home").resolve(&req).await.unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        page["version"], "v-installed",
        "the page object must advertise the version Inertia::install was \
         given, not InertiaConfig::default()'s \"1.0\""
    );
}

#[tokio::test]
async fn per_response_config_still_beats_the_installed_config() {
    let _guard = suprnova::testing::TestContainer::fake();
    suprnova::Inertia::install(
        &InertiaConfig::new()
            .version("v-installed")
            .development(true),
    )
    .expect("dev-mode install must not require a Vite manifest");

    let req = MockReq::new("/").inertia();
    let resp = InertiaResponse::new("Home")
        .with_config(InertiaConfig::new().version("per-response"))
        .resolve(&req)
        .await
        .unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        page["version"], "per-response",
        "with_config is a per-response override and must still win over \
         the installed default"
    );
}

#[tokio::test]
async fn without_an_install_the_response_uses_the_default_config() {
    // The regression guard for every app and every test that never calls
    // Inertia::install: behaviour must be byte-for-byte what it was.
    let _guard = suprnova::testing::TestContainer::fake();

    let req = MockReq::new("/").inertia();
    let resp = InertiaResponse::new("Home").resolve(&req).await.unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        page["version"], "1.0",
        "with nothing installed the response must fall back to \
         InertiaConfig::default()"
    );
}

#[tokio::test]
async fn a_failed_install_retains_nothing() {
    // The failure mode. Inertia::install fails closed in production when
    // no Vite manifest exists (CFG-01). It must not half-install: no
    // middleware AND no retained config, so responses keep rendering from
    // the default rather than from a config the operator was just told
    // was unusable.
    let _guard = suprnova::testing::TestContainer::fake();
    let err = suprnova::Inertia::install(
        &InertiaConfig::new()
            .version("v-never-installed")
            .production()
            .manifest_path("this/path/does/not/exist/manifest.json"),
    )
    .expect_err("production install without a manifest must fail closed");
    assert!(
        format!("{err}").contains("Vite manifest"),
        "sanity check: this must be the fail-closed error, got: {err}"
    );

    let req = MockReq::new("/").inertia();
    let resp = InertiaResponse::new("Home").resolve(&req).await.unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        page["version"], "1.0",
        "a failed install must retain no config"
    );
}

#[tokio::test]
async fn a_second_install_replaces_the_first() {
    // `Inertia::install` is legitimately callable more than once (tests,
    // and apps that re-bootstrap), and the retained config is a plain
    // `RwLock<Option<_>>` rather than a `OnceLock` precisely so that a
    // later call wins rather than erroring or being silently ignored.
    let _guard = suprnova::testing::TestContainer::fake();
    suprnova::Inertia::install(&InertiaConfig::new().version("a").development(true))
        .expect("dev-mode install must not require a Vite manifest");
    suprnova::Inertia::install(&InertiaConfig::new().version("b").development(true))
        .expect("dev-mode install must not require a Vite manifest");

    let req = MockReq::new("/").inertia();
    let resp = InertiaResponse::new("Home").resolve(&req).await.unwrap();
    let body = body_to_string(resp.into_hyper().into_body());
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        page["version"], "b",
        "a second install must replace the first, not merge with or defer \
         to it"
    );
}

// ---- helpers ----

fn body_to_string(
    body: http_body_util::combinators::BoxBody<bytes::Bytes, std::convert::Infallible>,
) -> String {
    use http_body_util::BodyExt;
    let bytes = futures_lite_block_on(async move { body.collect().await.unwrap().to_bytes() });
    String::from_utf8(bytes.to_vec()).unwrap()
}

/// Minimal block-on for collecting the response body in sync tests, without
/// pulling in the full tokio runtime (these tests don't otherwise need one).
fn futures_lite_block_on<F: std::future::Future>(fut: F) -> F::Output {
    use std::pin::pin;
    use std::task::{Context, Poll, Waker};
    let mut fut = pin!(fut);
    let waker = Waker::noop();
    let mut ctx = Context::from_waker(waker);
    loop {
        match fut.as_mut().poll(&mut ctx) {
            Poll::Ready(v) => return v,
            Poll::Pending => std::thread::yield_now(),
        }
    }
}

// ---- asset version from the Vite manifest (inertia-laravel I1.4) ---------

/// Write a throwaway manifest and return its path.
fn write_temp_manifest(body: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "test-inertia-version-manifest-{}.json",
        uuid::Uuid::new_v4()
    ));
    std::fs::write(&path, body).unwrap();
    path
}

#[test]
fn manifest_version_is_a_stable_32_char_hex_string() {
    let path = write_temp_manifest(r#"{"src/main.ts":{"file":"main-AAA.js","isEntry":true}}"#);
    let resolver = VersionResolver::from_manifest(&path);

    let first = resolver.resolve();
    let second = resolver.resolve();
    std::fs::remove_file(&path).ok();

    assert_eq!(
        first, second,
        "the same bytes must hash to the same version"
    );
    assert_eq!(first.len(), 32, "got: {first}");
    assert!(
        first.chars().all(|c| c.is_ascii_hexdigit()),
        "version must be hex, got: {first}"
    );
}

#[test]
fn manifest_version_changes_when_the_manifest_changes() {
    let path = write_temp_manifest(r#"{"src/main.ts":{"file":"main-AAA.js","isEntry":true}}"#);
    let resolver = VersionResolver::from_manifest(&path);
    let before = resolver.resolve();

    std::fs::write(
        &path,
        r#"{"src/main.ts":{"file":"main-BBB.js","isEntry":true}}"#,
    )
    .unwrap();
    let after = resolver.resolve();
    std::fs::remove_file(&path).ok();

    assert_ne!(
        before, after,
        "a rebuilt bundle must produce a new version so clients bounce"
    );
}

#[test]
fn manifest_version_falls_back_when_the_file_is_missing() {
    let resolver = VersionResolver::from_manifest("/definitely/not/a/real/manifest.json");
    assert_eq!(
        resolver.resolve(),
        "1.0",
        "a missing manifest must not error - dev has no build"
    );
}

#[test]
fn default_config_resolves_its_version_from_the_configured_manifest() {
    // The default resolver follows `manifest_path`, so pointing the
    // config at a manifest is all an app has to do.
    let path = write_temp_manifest(r#"{"src/main.ts":{"file":"main-CCC.js","isEntry":true}}"#);
    let cfg = InertiaConfig::new().manifest_path(&path);
    let resolved = cfg.version.resolve();
    let expected = VersionResolver::from_manifest(&path).resolve();
    std::fs::remove_file(&path).ok();

    assert_eq!(resolved, expected);
    assert_ne!(
        resolved, "1.0",
        "a present manifest must not use the fallback"
    );
}

#[test]
fn an_explicit_version_survives_a_later_manifest_path_call() {
    // `.version(...)` is a deliberate statement; re-pointing the
    // manifest must not silently overrule it.
    let path = write_temp_manifest(r#"{"src/main.ts":{"file":"main-DDD.js","isEntry":true}}"#);
    let cfg = InertiaConfig::new().version("pinned").manifest_path(&path);
    let resolved = cfg.version.resolve();
    std::fs::remove_file(&path).ok();

    assert_eq!(resolved, "pinned");
}

#[tokio::test]
async fn page_object_carries_the_manifest_derived_version() {
    let path = write_temp_manifest(r#"{"src/main.ts":{"file":"main-EEE.js","isEntry":true}}"#);
    let expected = VersionResolver::from_manifest(&path).resolve();

    let cfg = InertiaConfig::new().manifest_path(&path);
    let req = MockReq::new("/").inertia();
    let resp = InertiaResponse::new("Home")
        .with_config(cfg)
        .resolve(&req)
        .await
        .unwrap();

    let body = body_to_string(resp.into_hyper().into_body());
    std::fs::remove_file(&path).ok();
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(page["version"], expected);
}
