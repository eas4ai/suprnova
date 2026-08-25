//! Integration tests for `Redirect` and `RedirectRouteBuilder` query
//! string encoding (codex review finding 15).
//!
//! Verifies that special characters in redirect query keys and values
//! are properly percent-encoded via `url::form_urlencoded` rather than
//! concatenated raw - which previously produced malformed redirects and
//! parameter injection.

use suprnova::Cookie;
use suprnova::Redirect;
use suprnova::routing::register_route_name;

/// Extract the `Location` header from a `Response` produced by
/// converting a `Redirect` (or `RedirectRouteBuilder`) into a
/// `suprnova::Response`. Panics if the conversion failed or no
/// `Location` header was set.
fn location_of(resp: suprnova::Response) -> String {
    let http = match resp {
        Ok(r) => r,
        Err(_) => panic!("redirect conversion produced Err"),
    };
    let hyper = http.into_hyper();
    hyper
        .headers()
        .get("Location")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .expect("Location header missing")
}

#[tokio::test]
async fn redirect_encodes_space_and_ampersand_in_value() {
    let resp: suprnova::Response = Redirect::to("/dashboard")
        .query("message", "hello world & friends")
        .into();
    let loc = location_of(resp);

    // `form_urlencoded` encodes spaces as `+` and `&` as `%26`.
    assert_eq!(
        loc, "/dashboard?message=hello+world+%26+friends",
        "got {loc}"
    );
}

#[tokio::test]
async fn redirect_encodes_equals_in_value() {
    let resp: suprnova::Response = Redirect::to("/x").query("token", "a=b=c").into();
    let loc = location_of(resp);

    // `=` inside the value must be encoded so the receiver doesn't
    // mistake it for a key/value separator.
    assert_eq!(loc, "/x?token=a%3Db%3Dc", "got {loc}");
}

#[tokio::test]
async fn redirect_encodes_unicode_value() {
    let resp: suprnova::Response = Redirect::to("/u").query("name", "héllo").into();
    let loc = location_of(resp);

    // Multi-byte UTF-8 encodes per RFC 3986 - the character itself
    // must not appear raw in the Location header.
    assert!(
        !loc.contains('é'),
        "unicode must be percent-encoded, got {loc}"
    );
    assert!(
        loc.contains("h%C3%A9llo"),
        "expected percent-encoded UTF-8, got {loc}"
    );
}

#[tokio::test]
async fn redirect_encodes_special_chars_in_key() {
    let resp: suprnova::Response = Redirect::to("/x").query("key&with=special", "v").into();
    let loc = location_of(resp);

    // Both `&` and `=` are URL-significant; both must be encoded
    // inside the key.
    assert!(loc.contains("key%26with%3Dspecial=v"), "got {loc}");
}

#[tokio::test]
async fn redirect_preserves_existing_query_string() {
    let resp: suprnova::Response = Redirect::to("/search?q=foo").query("page", "2").into();
    let loc = location_of(resp);

    // The new pair appends with `&`, the existing `q=foo` survives.
    assert_eq!(loc, "/search?q=foo&page=2", "got {loc}");
}

#[tokio::test]
async fn redirect_preserves_fragment_outside_query() {
    let resp: suprnova::Response = Redirect::to("/page#section").query("p", "1").into();
    let loc = location_of(resp);

    // The fragment must land at the end, outside the query string -
    // otherwise the encoder would percent-encode the `#`.
    assert_eq!(loc, "/page?p=1#section", "got {loc}");
}

#[tokio::test]
async fn redirect_no_query_params_returns_location_unchanged() {
    let resp: suprnova::Response = Redirect::to("/clean").into();
    let loc = location_of(resp);

    assert_eq!(
        loc, "/clean",
        "no query params must not append `?`, got {loc}"
    );
}

#[tokio::test]
async fn redirect_multiple_query_params_joined_with_ampersand() {
    let resp: suprnova::Response = Redirect::to("/x")
        .query("a", "1")
        .query("b", "2")
        .query("c", "3")
        .into();
    let loc = location_of(resp);

    assert_eq!(loc, "/x?a=1&b=2&c=3", "got {loc}");
}

#[tokio::test]
async fn redirect_route_encodes_query_params() {
    // `RedirectRouteBuilder` has its own `build_url`; this proves it
    // uses the same encoder as `Redirect::to`.
    register_route_name("_test_redirect_encoding_target", "/test/encoded/target");

    let resp: suprnova::Response = Redirect::route("_test_redirect_encoding_target")
        .query("q", "hello world&friends")
        .into();
    let loc = location_of(resp);

    assert_eq!(
        loc, "/test/encoded/target?q=hello+world%26friends",
        "RedirectRouteBuilder must encode query params, got {loc}"
    );
}

#[tokio::test]
async fn redirect_route_appends_to_existing_query_in_path() {
    // If a registered route somehow already carries a `?` in its
    // template path, the helper must append with `&`, not `?`.
    register_route_name("_test_redirect_with_existing_query", "/test/path?seed=42");

    let resp: suprnova::Response = Redirect::route("_test_redirect_with_existing_query")
        .query("page", "2")
        .into();
    let loc = location_of(resp);

    assert_eq!(loc, "/test/path?seed=42&page=2", "got {loc}");
}

#[tokio::test]
async fn redirect_route_unknown_name_returns_sanitised_500() {
    // A `Redirect::route(name)` whose name has no matching registry
    // entry surfaces a `RouteUrlError::NameNotFound` from `build_url`.
    // The conversion must route through the canonical
    // `FrameworkError -> HttpResponse` converter so the body matches
    // the framework's 5xx sanitisation policy: a Laravel-shaped JSON
    // body whose `message` field is the generic sanitised string, NOT
    // the raw `RouteUrlError` display (which would leak routing-table
    // detail to production clients). The dev-only `debug_message`
    // field carries the raw detail when APP_DEBUG=true and is
    // explicitly documented as "MUST NOT be parsed by production
    // clients" (see `From<FrameworkError> for HttpResponse`), so the
    // assertion here pins the `message` policy only.
    let resp: suprnova::Response =
        Redirect::route("_definitely_not_a_registered_route_name").into();

    // Short-circuit path returns Err for a 5xx - both arms carry an
    // HttpResponse; unwrap_or_else collapses them in the runtime.
    let http = match resp {
        Ok(r) => r,
        Err(r) => r,
    };
    let hyper = http.into_hyper();
    assert_eq!(
        hyper.status().as_u16(),
        500,
        "build_url failure must surface as a 500"
    );

    // The pre-fix path returned `HttpResponse::text(e.to_string())`
    // (`text/plain`); after the fix the canonical converter emits
    // `application/json`. Drain the body and assert the sanitised
    // Laravel shape.
    let bytes = http_body_util::BodyExt::collect(hyper.into_body())
        .await
        .expect("body collect")
        .to_bytes();
    let body: serde_json::Value =
        serde_json::from_slice(&bytes).expect("response must be JSON, not a text/plain dump");
    assert_eq!(
        body.get("message").and_then(|v| v.as_str()),
        Some("Internal Server Error"),
        "5xx body's `message` field must be the sanitised Laravel-shaped string, got {body}"
    );
    // The body must also carry the canonical `request_id` field that
    // the FrameworkError converter injects - its presence (even when
    // null outside a request scope) is what distinguishes the
    // converter path from the old raw `HttpResponse::text` path.
    assert!(
        body.get("request_id").is_some(),
        "5xx body must include `request_id` (proves the FrameworkError converter ran), got {body}"
    );
}

// ---- cookie removal on redirects (Laravel 13.25 #61115) ------------------

/// Collect every `Set-Cookie` off a `Response` produced by converting a
/// redirect builder.
fn set_cookies_of(resp: suprnova::Response) -> Vec<String> {
    let http = match resp {
        Ok(r) => r,
        Err(_) => panic!("redirect conversion produced Err"),
    };
    http.into_hyper()
        .headers()
        .get_all("Set-Cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .collect()
}

#[tokio::test]
async fn redirect_without_cookies_expires_each_name_on_the_302() {
    let resp: suprnova::Response = Redirect::to("/login")
        .without_cookies(["session", "remember"])
        .into();
    let cookies = set_cookies_of(resp);

    assert_eq!(cookies.len(), 2, "got: {cookies:?}");
    assert!(
        cookies.iter().all(|c| c.contains("Max-Age=0")),
        "{cookies:?}"
    );
    assert!(
        cookies.iter().any(|c| c.starts_with("session=")),
        "{cookies:?}"
    );
    assert!(
        cookies.iter().any(|c| c.starts_with("remember=")),
        "{cookies:?}"
    );
}

#[tokio::test]
async fn redirect_without_cookie_composes_with_with_cookies() {
    let resp: suprnova::Response = Redirect::to("/home")
        .with_cookies([Cookie::new("welcome", "yes")])
        .without_cookie("session")
        .into();
    let cookies = set_cookies_of(resp);

    assert_eq!(cookies.len(), 2, "got: {cookies:?}");
    assert!(
        cookies.iter().any(|c| c.starts_with("welcome=yes")),
        "{cookies:?}"
    );
    assert!(
        cookies
            .iter()
            .any(|c| c.starts_with("session=") && c.contains("Max-Age=0")),
        "{cookies:?}"
    );
}

#[tokio::test]
async fn redirect_route_without_cookies_expires_each_name() {
    register_route_name("_test_redirect_without_cookies", "/test/logged-out");

    let resp: suprnova::Response = Redirect::route("_test_redirect_without_cookies")
        .without_cookies(["session", "csrf"])
        .into();
    let cookies = set_cookies_of(resp);

    assert_eq!(cookies.len(), 2, "got: {cookies:?}");
    assert!(
        cookies.iter().all(|c| c.contains("Max-Age=0")),
        "{cookies:?}"
    );
}

#[tokio::test]
async fn redirect_route_without_cookie_takes_a_single_name() {
    register_route_name("_test_redirect_without_cookie_single", "/test/bye");

    let resp: suprnova::Response = Redirect::route("_test_redirect_without_cookie_single")
        .without_cookie("session")
        .into();
    let cookies = set_cookies_of(resp);

    assert_eq!(cookies.len(), 1);
    assert!(cookies[0].starts_with("session="), "got: {}", cookies[0]);
    assert!(cookies[0].contains("Max-Age=0"), "got: {}", cookies[0]);
}
