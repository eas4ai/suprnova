//! `AssertableInertia` unit tests. Everything here builds an
//! `HttpResponse` (or a `TestResponse`) by hand rather than driving a
//! real request — the type under test is a pure JSON-object parser
//! plus a set of dot-path assertions, so there's nothing a socket would
//! add. `framework/tests/inertia.rs` is the proof-of-use site: three of
//! its existing tests are rewritten against this type as part of the
//! same task.
//!
//! The `reload_*` tests fake a "server" inline — a closure that
//! filters a canned page object's `props` by the `ReloadRequest`'s
//! `only`/`except` lists — so they prove the replay plumbing (request
//! shape in, chained `AssertableInertia` out, reloader propagation)
//! without needing `PartialFilter`/`InertiaResponse` at all; those are
//! already covered end-to-end by `framework/tests/inertia.rs`.

use serde_json::json;

use suprnova::testing::{AssertableInertia, ReloadRequest, TestResponse};
use suprnova::{HttpResponse, MANIFEST_VERSION_FALLBACK};

fn json_page_response() -> HttpResponse {
    let page = json!({
        "component": "Users/Index",
        "props": {
            "users": [{"id": 1, "name": "Ada"}, {"id": 2, "name": "Grace"}],
            "errors": {},
        },
        "url": "/users",
        "version": MANIFEST_VERSION_FALLBACK,
        "flash": {"toast": {"message": "Saved!"}},
        "deferredProps": {"default": ["permissions"]},
    });
    HttpResponse::json(page).header("X-Inertia", "true")
}

fn html_shell_response(page: &serde_json::Value) -> HttpResponse {
    let script = serde_json::to_string(page).unwrap().replace('/', "\\/");
    let html = format!(
        "<!DOCTYPE html><html><head></head><body>\
         <script type=\"application/json\" data-page=\"app\">{script}</script>\
         <div id=\"app\"></div></body></html>"
    );
    HttpResponse::html(html)
}

// ── `from_response` — both shapes ────────────────────────────────────

#[test]
fn from_response_parses_json_page_object_and_chains_every_assertion() {
    let response = json_page_response();

    AssertableInertia::from_response(&response)
        .component("Users/Index")
        .url("/users")
        .version(MANIFEST_VERSION_FALLBACK)
        .has("users")
        .has("users.0.name")
        .missing("admin_only")
        .where_("users.0.name", "Ada")
        .where_("users.1.id", 2)
        .count("users", 2)
        .has_flash("toast.message", Some(json!("Saved!")))
        .has_flash("toast", None::<serde_json::Value>);
}

#[test]
fn from_response_extracts_the_page_object_from_the_html_shell() {
    let page = json!({
        "component": "Home",
        "props": {"greeting": "hi"},
        "url": "/",
        "version": "abc123",
    });
    let response = html_shell_response(&page);

    AssertableInertia::from_response(&response)
        .component("Home")
        .url("/")
        .version("abc123")
        .where_("greeting", "hi");
}

#[test]
fn prop_returns_the_value_at_a_dot_path_or_null_when_absent() {
    let response = json_page_response();
    let assertable = AssertableInertia::from_response(&response);

    assert_eq!(assertable.prop("users.0.id"), json!(1));
    assert_eq!(assertable.prop("nope"), serde_json::Value::Null);
}

#[test]
fn where_reads_the_collapsed_first_message_errors_shape_from_task_23() {
    // `errors.<field>` is a plain string by default since T23, not an
    // array — the shape a validation-redirect page actually renders.
    let page = json!({
        "component": "Register",
        "props": {"errors": {"email": "The email field is required."}},
        "url": "/register",
        "version": MANIFEST_VERSION_FALLBACK,
    });
    let response = HttpResponse::json(page).header("X-Inertia", "true");

    AssertableInertia::from_response(&response)
        .where_("errors.email", "The email field is required.");
}

#[test]
#[should_panic(expected = "no Inertia page object")]
fn from_response_panics_when_neither_shape_is_present() {
    let response = HttpResponse::text("plain text, not Inertia");
    AssertableInertia::from_response(&response);
}

#[test]
#[should_panic(expected = "not valid JSON")]
fn from_response_panics_when_the_x_inertia_body_is_not_json() {
    let response = HttpResponse::text("not json").header("X-Inertia", "true");
    AssertableInertia::from_response(&response);
}

#[test]
#[should_panic(expected = "missing required key")]
fn from_response_panics_when_a_required_key_is_missing() {
    let page = json!({"component": "Home", "props": {}, "url": "/"}); // no version
    let response = HttpResponse::json(page).header("X-Inertia", "true");
    AssertableInertia::from_response(&response);
}

#[test]
#[should_panic(expected = "element, but its content is not valid JSON")]
fn from_response_reports_the_real_parse_error_when_the_html_shells_script_is_malformed() {
    // The <script data-page="app"> element IS present here - only its
    // content is broken. The panic must say so, not claim the element
    // is missing (that would send a reader looking in the wrong place
    // entirely for a page that has a data-page element with bad JSON
    // inside it).
    let html = "<!DOCTYPE html><html><head></head><body>\
        <script type=\"application/json\" data-page=\"app\">{not valid json at all</script>\
        <div id=\"app\"></div></body></html>";
    let response = HttpResponse::html(html);
    AssertableInertia::from_response(&response);
}

// ── page-level assertions — failure modes ────────────────────────────

#[test]
#[should_panic(expected = "AssertableInertia::component")]
fn component_panics_on_mismatch() {
    AssertableInertia::from_response(&json_page_response()).component("Wrong");
}

#[test]
#[should_panic(expected = "AssertableInertia::url")]
fn url_panics_on_mismatch() {
    AssertableInertia::from_response(&json_page_response()).url("/wrong");
}

#[test]
#[should_panic(expected = "AssertableInertia::version")]
fn version_panics_on_mismatch() {
    AssertableInertia::from_response(&json_page_response()).version("nope");
}

#[test]
#[should_panic(expected = "AssertableInertia::has(")]
fn has_panics_when_the_prop_is_absent() {
    AssertableInertia::from_response(&json_page_response()).has("nope");
}

#[test]
#[should_panic(expected = "AssertableInertia::missing(")]
fn missing_panics_when_the_prop_is_present() {
    AssertableInertia::from_response(&json_page_response()).missing("users");
}

#[test]
#[should_panic(expected = "AssertableInertia::where_")]
fn where_panics_on_a_value_mismatch() {
    AssertableInertia::from_response(&json_page_response()).where_("users.0.name", "Wrong");
}

#[test]
#[should_panic(expected = "AssertableInertia::count")]
fn count_panics_on_a_length_mismatch() {
    AssertableInertia::from_response(&json_page_response()).count("users", 5);
}

#[test]
// Distinct from `count_panics_on_a_length_mismatch`: there the path
// resolves to a real (wrong-length) array, so the message echoes it
// verbatim. Here the path resolves to a present value that isn't an
// array at all, which is a different branch in `count`'s match and
// prints the actual value rather than the "<missing or not an array>"
// placeholder a truly-missing path would print.
#[should_panic(expected = "Received: \"not-an-array\"")]
fn count_panics_with_the_actual_value_when_the_path_resolves_to_a_non_array() {
    let page = json!({
        "component": "Users/Index",
        "props": {"users": "not-an-array"},
        "url": "/users",
        "version": MANIFEST_VERSION_FALLBACK,
    });
    let response = HttpResponse::json(page).header("X-Inertia", "true");
    AssertableInertia::from_response(&response).count("users", 1);
}

#[test]
#[should_panic(expected = "AssertableInertia::has_flash")]
fn has_flash_panics_when_the_key_is_absent() {
    AssertableInertia::from_response(&json_page_response())
        .has_flash("nope", None::<serde_json::Value>);
}

#[test]
#[should_panic(expected = "AssertableInertia::has_flash")]
fn has_flash_panics_on_a_value_mismatch() {
    AssertableInertia::from_response(&json_page_response())
        .has_flash("toast.message", Some(json!("Wrong")));
}

// ── `TestResponse::assert_inertia` ───────────────────────────────────

#[test]
fn test_response_assert_inertia_parses_the_json_body() {
    let page = json!({
        "component": "Users/Index",
        "props": {"users": []},
        "url": "/users",
        "version": MANIFEST_VERSION_FALLBACK,
    });
    let response = TestResponse::new(
        200,
        vec![
            ("content-type".to_string(), "application/json".to_string()),
            ("x-inertia".to_string(), "true".to_string()),
        ],
        page.to_string(),
    );

    response
        .assert_inertia()
        .component("Users/Index")
        .url("/users");
}

#[test]
#[should_panic(expected = "assert_inertia")]
fn test_response_assert_inertia_panics_without_the_x_inertia_header() {
    let response = TestResponse::new(200, Vec::<(String, String)>::new(), "{}");
    response.assert_inertia();
}

#[test]
// Distinct from the header-absent case above: here the header IS
// present, just not "true" — a client that sent `X-Inertia: false` (or
// any other stray value) must be rejected the same way as one that
// sent no header at all, and the message should echo the value it saw.
#[should_panic(expected = "got X-Inertia = Some(\"false\")")]
fn test_response_assert_inertia_panics_when_the_header_is_present_but_not_true() {
    let response = TestResponse::new(
        200,
        vec![("x-inertia".to_string(), "false".to_string())],
        "{}",
    );
    response.assert_inertia();
}

// ── `reload_only` / `reload_except` / `load_deferred_props` ─────────

fn full_users_page() -> serde_json::Value {
    json!({
        "component": "Users/Index",
        "props": {
            "users": [{"id": 1, "name": "Ada"}],
            "stats": {"total": 1},
        },
        "url": "/users",
        "version": MANIFEST_VERSION_FALLBACK,
        "deferredProps": {"default": ["stats"]},
    })
}

/// Fakes the server side of a partial reload: filters the canned page's
/// `props` by the `ReloadRequest`'s `only`/`except` lists. Proves the
/// replay plumbing works without pulling in `PartialFilter` — the real
/// filtering semantics are `framework/src/inertia/prop.rs`'s job and
/// are already covered by `framework/tests/inertia.rs`.
fn filtered_response(reload: &ReloadRequest) -> HttpResponse {
    let page = full_users_page();
    let mut props = page["props"].as_object().unwrap().clone();
    if let Some(only) = &reload.only {
        props.retain(|k, _| only.iter().any(|o| o == k));
    }
    if let Some(except) = &reload.except {
        props.retain(|k, _| !except.iter().any(|e| e == k));
    }
    let mut out = page;
    out["props"] = serde_json::Value::Object(props);
    out.as_object_mut().unwrap().remove("deferredProps");
    HttpResponse::json(out).header("X-Inertia", "true")
}

/// Same fake server as `filtered_response`, but reconstructs the
/// only/except lists from `ReloadRequest::headers()` instead of reading
/// `reload.only`/`reload.except` off the struct directly - proving the
/// `X-Inertia-Partial-Except` header `.headers()` emits round-trips
/// correctly, not just the field it was built from. `filtered_response`
/// (used by the `only` tests) already covers `X-Inertia-Partial-Data`
/// the same way `reload_request_headers_include_partial_component_only_when_only_or_except_is_set`
/// covers it as a raw unit test - this is the `except` counterpart for
/// the integration path.
fn filtered_response_from_headers(reload: &ReloadRequest) -> HttpResponse {
    let headers = reload.headers();
    let csv = |name: &str| -> Option<Vec<String>> {
        headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.split(',').map(str::to_string).collect())
    };
    let only = csv("X-Inertia-Partial-Data");
    let except = csv("X-Inertia-Partial-Except");

    let page = full_users_page();
    let mut props = page["props"].as_object().unwrap().clone();
    if let Some(only) = &only {
        props.retain(|k, _| only.iter().any(|o| o == k));
    }
    if let Some(except) = &except {
        props.retain(|k, _| !except.iter().any(|e| e == k));
    }
    let mut out = page;
    out["props"] = serde_json::Value::Object(props);
    out.as_object_mut().unwrap().remove("deferredProps");
    HttpResponse::json(out).header("X-Inertia", "true")
}

#[tokio::test]
async fn reload_only_replays_a_partial_reload_and_asserts_the_requested_keys_are_present() {
    let response = HttpResponse::json(full_users_page()).header("X-Inertia", "true");
    let assertable = AssertableInertia::from_response(&response).with_reload(|reload| async move {
        AssertableInertia::from_response(&filtered_response(&reload))
    });

    let reloaded = assertable.reload_only(["users"]).await;
    reloaded.has("users").missing("stats");
}

#[tokio::test]
async fn reload_except_replays_a_partial_reload_and_asserts_the_excluded_keys_are_absent() {
    let response = HttpResponse::json(full_users_page()).header("X-Inertia", "true");
    // Routed through `ReloadRequest::headers()` rather than reading
    // `reload.except` off the struct directly, so this test also proves
    // the `X-Inertia-Partial-Except` header `.headers()` builds is what
    // actually drives the exclusion, not just the field behind it.
    let assertable = AssertableInertia::from_response(&response).with_reload(|reload| async move {
        AssertableInertia::from_response(&filtered_response_from_headers(&reload))
    });

    let reloaded = assertable.reload_except(["stats"]).await;
    reloaded.has("users").missing("stats");
}

#[tokio::test]
async fn load_deferred_props_replays_every_deferred_group_in_one_reload() {
    let response = HttpResponse::json(full_users_page()).header("X-Inertia", "true");
    let assertable = AssertableInertia::from_response(&response).with_reload(|reload| async move {
        assert_eq!(reload.only.as_deref(), Some(&["stats".to_string()][..]));
        AssertableInertia::from_response(&filtered_response(&reload))
    });

    let reloaded = assertable.load_deferred_props().await;
    reloaded.has("stats");
}

#[tokio::test]
async fn a_reloaded_assertable_carries_the_same_reloader_forward() {
    let response = HttpResponse::json(full_users_page()).header("X-Inertia", "true");
    let assertable = AssertableInertia::from_response(&response).with_reload(|reload| async move {
        AssertableInertia::from_response(&filtered_response(&reload))
    });

    let first = assertable.reload_only(["users"]).await;
    // `first` never had `.with_reload(...)` called on it directly — this
    // only works if the reloader was carried forward from `assertable`.
    let second = first.reload_only(["users"]).await;
    second.has("users");
}

#[tokio::test]
#[should_panic(expected = "no reloader attached")]
async fn reload_only_panics_without_a_reloader_attached() {
    let response = HttpResponse::json(full_users_page()).header("X-Inertia", "true");
    AssertableInertia::from_response(&response)
        .reload_only(["users"])
        .await;
}

// ── the internal consistency guard `reload_only`/`reload_except` run on
// the replayed result actually fires, not just documents intent ──────
//
// Each fake reloader below deliberately misbehaves (the way a buggy
// `with_reload` harness — or a server bug the harness faithfully
// reports — would): it returns a page that doesn't match what a
// correct replay of the same request would produce. If the
// `reloaded.component(...)`/`.url(...)`/`.version(...)`/`.has(...)`/
// `.missing(...)` re-assertions inside `reload_only`/`reload_except`
// were ever deleted, every one of these tests would stop panicking and
// fail — that's what makes them prove the guard is load-bearing rather
// than merely present.

#[tokio::test]
#[should_panic(expected = "AssertableInertia::component")]
async fn reload_only_panics_when_the_replayed_response_is_for_a_different_component() {
    let response = HttpResponse::json(full_users_page()).header("X-Inertia", "true");
    let assertable =
        AssertableInertia::from_response(&response).with_reload(|_reload| async move {
            // Misbehaving reloader: claims success but lands on a different
            // component, as a harness that followed a redirect to the wrong
            // route might.
            let wrong_component = json!({
                "component": "Wrong/Component",
                "props": {"users": [{"id": 1, "name": "Ada"}]},
                "url": "/users",
                "version": MANIFEST_VERSION_FALLBACK,
            });
            AssertableInertia::from_response(
                &HttpResponse::json(wrong_component).header("X-Inertia", "true"),
            )
        });

    assertable.reload_only(["users"]).await;
}

#[tokio::test]
#[should_panic(expected = "AssertableInertia::has(")]
async fn reload_only_panics_when_the_replayed_response_omits_a_requested_key() {
    let response = HttpResponse::json(full_users_page()).header("X-Inertia", "true");
    let assertable =
        AssertableInertia::from_response(&response).with_reload(|_reload| async move {
            // Misbehaving reloader: the reply doesn't actually carry the key
            // that was requested via `only`.
            let missing_users = json!({
                "component": "Users/Index",
                "props": {"stats": {"total": 1}},
                "url": "/users",
                "version": MANIFEST_VERSION_FALLBACK,
            });
            AssertableInertia::from_response(
                &HttpResponse::json(missing_users).header("X-Inertia", "true"),
            )
        });

    assertable.reload_only(["users"]).await;
}

#[tokio::test]
#[should_panic(expected = "AssertableInertia::component")]
async fn reload_except_panics_when_the_replayed_response_is_for_a_different_component() {
    let response = HttpResponse::json(full_users_page()).header("X-Inertia", "true");
    let assertable =
        AssertableInertia::from_response(&response).with_reload(|_reload| async move {
            let wrong_component = json!({
                "component": "Wrong/Component",
                "props": {"users": [{"id": 1, "name": "Ada"}]},
                "url": "/users",
                "version": MANIFEST_VERSION_FALLBACK,
            });
            AssertableInertia::from_response(
                &HttpResponse::json(wrong_component).header("X-Inertia", "true"),
            )
        });

    assertable.reload_except(["stats"]).await;
}

#[tokio::test]
#[should_panic(expected = "AssertableInertia::missing(")]
async fn reload_except_panics_when_the_replayed_response_still_contains_an_excluded_key() {
    let response = HttpResponse::json(full_users_page()).header("X-Inertia", "true");
    let assertable =
        AssertableInertia::from_response(&response).with_reload(|_reload| async move {
            // Misbehaving reloader: ignores the exclusion and echoes the
            // full, unfiltered page back — "stats" is still there despite
            // being named in `except`.
            AssertableInertia::from_response(
                &HttpResponse::json(full_users_page()).header("X-Inertia", "true"),
            )
        });

    assertable.reload_except(["stats"]).await;
}

#[test]
fn reload_request_headers_include_partial_component_only_when_only_or_except_is_set() {
    let plain = ReloadRequest {
        url: "/users".to_string(),
        component: "Users/Index".to_string(),
        version: "v1".to_string(),
        only: None,
        except: None,
    };
    let headers = plain.headers();
    assert!(headers.contains(&("X-Inertia".to_string(), "true".to_string())));
    assert!(headers.contains(&("X-Inertia-Version".to_string(), "v1".to_string())));
    assert!(
        !headers
            .iter()
            .any(|(k, _)| k == "X-Inertia-Partial-Component")
    );

    let only = ReloadRequest {
        only: Some(vec!["users".to_string(), "stats".to_string()]),
        ..plain.clone()
    };
    let headers = only.headers();
    assert!(headers.contains(&(
        "X-Inertia-Partial-Component".to_string(),
        "Users/Index".to_string()
    )));
    assert!(headers.contains(&(
        "X-Inertia-Partial-Data".to_string(),
        "users,stats".to_string()
    )));
}

#[test]
fn reload_request_headers_include_partial_except_and_omit_partial_data_when_only_is_unset() {
    let request = ReloadRequest {
        url: "/users".to_string(),
        component: "Users/Index".to_string(),
        version: "v1".to_string(),
        only: None,
        except: Some(vec!["stats".to_string(), "extra".to_string()]),
    };
    let headers = request.headers();
    assert!(headers.contains(&(
        "X-Inertia-Partial-Component".to_string(),
        "Users/Index".to_string()
    )));
    assert!(headers.contains(&(
        "X-Inertia-Partial-Except".to_string(),
        "stats,extra".to_string()
    )));
    // The `only` and `except` branches build independent header entries
    // - setting one must not also emit the other's header.
    assert!(!headers.iter().any(|(k, _)| k == "X-Inertia-Partial-Data"));
}
