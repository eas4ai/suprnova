//! PAR-117: `url::current` without the query, `url::previous_path`,
//! `url::secure_with`, and `Redirect::guest` storing the request's own URL
//! only for a page request.
//!
//! A test that reads `APP_URL` registers its own `AppConfig`, which is
//! process-wide, so it runs alone in a child process: the `#[test]` of the
//! plain name starts the `_child` test, which does the work.

use serial_test::serial;
use suprnova::session::{new_session_slot_for_test, session, session_mut, session_scope_for_test};
use suprnova::{AppConfig, Config, Redirect, Request, url};

const MODULE: &str = "laravel_http_gaps::urls::";

fn run_alone(child: &str) {
    crate::own_process::run_alone(&format!("{MODULE}{child}"));
}

fn install_app_url(app_url: &str) {
    Config::register(AppConfig::builder().url(app_url).debug(false).build());
}

// ---- url::current and url::full -------------------------------------------

#[test]
fn current_is_the_app_url_origin_root_and_path_without_the_query() {
    run_alone("current_is_the_app_url_origin_root_and_path_without_the_query_child");
}

#[test]
fn current_is_the_app_url_origin_root_and_path_without_the_query_child() {
    if !crate::own_process::is_child() {
        return;
    }
    install_app_url("https://example.com");
    let request = Request::for_test("GET", "/invoices?page=2");
    assert_eq!(url::current(&request), "https://example.com/invoices");
    assert_eq!(url::full(&request), "https://example.com/invoices?page=2");

    // No query: the two agree.
    let bare = Request::for_test("GET", "/invoices");
    assert_eq!(url::current(&bare), "https://example.com/invoices");
    assert_eq!(url::full(&bare), "https://example.com/invoices");

    // The origin is APP_URL's, whatever Host the client sends.
    let forged =
        Request::for_test_with_headers("GET", "/invoices?page=2", [("host", "evil.example")]);
    assert_eq!(url::current(&forged), "https://example.com/invoices");

    // A request-target naming another host stays a path on the origin.
    let network_path = Request::for_test("GET", "//evil.example/x?y=1");
    assert_eq!(
        url::current(&network_path),
        "https://example.com//evil.example/x"
    );
}

#[test]
fn current_carries_the_public_root() {
    run_alone("current_carries_the_public_root_child");
}

#[test]
fn current_carries_the_public_root_child() {
    if !crate::own_process::is_child() {
        return;
    }
    install_app_url("https://example.com/billing/");
    let request = Request::for_test("GET", "/invoices?page=2");
    assert_eq!(
        url::current(&request),
        "https://example.com/billing/invoices"
    );
    assert_eq!(
        url::full(&request),
        "https://example.com/billing/invoices?page=2"
    );
}

#[test]
fn refresh_for_keeps_the_query() {
    let request = Request::for_test("GET", "/invoices?page=2");
    let location = into_location(Redirect::refresh_for(&request));
    assert_eq!(location, "/invoices?page=2");
}

// ---- url::previous_path ---------------------------------------------------

#[test]
fn previous_path_strips_the_query_root_and_trailing_slash() {
    run_alone("previous_path_strips_the_query_root_and_trailing_slash_child");
}

#[tokio::test]
async fn previous_path_strips_the_query_root_and_trailing_slash_child() {
    if !crate::own_process::is_child() {
        return;
    }
    install_app_url("https://example.com/billing");
    assert_eq!(url::root(), "/billing");

    for (previous, expected) in [
        ("/billing/invoices/?page=2", "/invoices"),
        ("/billing/invoices#top", "/invoices"),
        ("/billing/", "/"),
        ("/billing?tab=1", "/"),
        ("/billing", "/"),
        // Not under the root: the path is kept.
        ("/other/x/", "/other/x"),
        // `/billingx` shares the bytes of the root but not a segment.
        ("/billingx/y", "/billingx/y"),
    ] {
        let slot = new_session_slot_for_test();
        let got = session_scope_for_test(slot, async {
            session_mut(|s| s.set_previous_url(previous));
            url::previous_path("/")
        })
        .await;
        assert_eq!(got, expected, "previous URL {previous}");
    }
}

#[test]
fn previous_path_without_a_previous_url_takes_the_fallbacks_path() {
    run_alone("previous_path_without_a_previous_url_takes_the_fallbacks_path_child");
}

#[tokio::test]
async fn previous_path_without_a_previous_url_takes_the_fallbacks_path_child() {
    if !crate::own_process::is_child() {
        return;
    }
    install_app_url("https://example.com/billing");
    // No session at all, and a session with nothing recorded.
    assert_eq!(url::previous_path("/"), "/");
    let slot = new_session_slot_for_test();
    session_scope_for_test(slot, async {
        assert_eq!(url::previous_path("/"), "/");
        assert_eq!(url::previous_path("/billing/home/?x=1"), "/home");
        assert_eq!(
            url::previous_path("https://example.com/billing/dash/"),
            "/dash"
        );
        assert_eq!(url::previous_path("https://example.com"), "/");
        assert_eq!(url::previous_path("home"), "/home");
        assert_eq!(url::previous_path(""), "/");
    })
    .await;
}

// ---- url::secure_with -----------------------------------------------------

#[test]
fn secure_with_appends_encoded_segments_and_upgrades_to_https() {
    run_alone("secure_with_appends_encoded_segments_and_upgrades_to_https_child");
}

#[test]
fn secure_with_appends_encoded_segments_and_upgrades_to_https_child() {
    if !crate::own_process::is_child() {
        return;
    }
    install_app_url("http://example.com");
    assert_eq!(
        url::secure_with("users", &["a b", "7"]),
        "https://example.com/users/a%20b/7"
    );
    assert_eq!(
        url::secure_with("/users/", &["a/b", "?#"]),
        "https://example.com/users/a%2Fb/%3F%23",
        "a slash, `?` and `#` stay inside one segment"
    );
    assert_eq!(
        url::secure_with("/users?tab=1#top", &["7"]),
        "https://example.com/users/7?tab=1#top",
        "the segments go before the query and the fragment"
    );
    assert_eq!(
        url::secure_with("/users", &[]),
        url::secure("/users"),
        "no segments is `secure`"
    );
    assert_eq!(
        url::secure_with("http://cdn.example/files", &["a b"]),
        "https://cdn.example/files/a%20b"
    );
}

#[test]
fn secure_with_keeps_the_app_url_root() {
    run_alone("secure_with_keeps_the_app_url_root_child");
}

#[test]
fn secure_with_keeps_the_app_url_root_child() {
    if !crate::own_process::is_child() {
        return;
    }
    install_app_url("https://example.com/billing");
    assert_eq!(
        url::secure_with("users", &["a b", "7"]),
        "https://example.com/billing/users/a%20b/7"
    );
}

// ---- Redirect::guest ------------------------------------------------------

fn into_location(redirect: Redirect) -> String {
    let response: suprnova::Response = redirect.into();
    let response = response.unwrap_or_else(|_| panic!("the redirect converts to Ok"));
    response
        .header_value("Location")
        .expect("a Location header")
        .to_string()
}

/// Run `Redirect::guest` for `request` in a session whose previous URL is
/// `previous`, and return the intended URL it left.
async fn intended_after_guest(request: Request, previous: Option<&str>) -> Option<String> {
    let slot = new_session_slot_for_test();
    session_scope_for_test(slot, async {
        if let Some(previous) = previous {
            session_mut(|s| s.set_previous_url(previous));
        }
        let location = into_location(Redirect::guest(&request, "/login"));
        assert_eq!(location, "/login");
        session().and_then(|s| s.get::<String>("url.intended"))
    })
    .await
}

#[tokio::test]
#[serial]
async fn guest_stores_a_plain_gets_own_path_and_query() {
    let request = Request::for_test("GET", "/invoices?page=2");
    assert_eq!(
        intended_after_guest(request, Some("/dashboard"))
            .await
            .as_deref(),
        Some("/invoices?page=2")
    );
    // An HTML navigation, and a GET whose Referer names another page.
    let html = Request::for_test_with_headers(
        "GET",
        "/reports",
        [
            ("accept", "text/html,application/xhtml+xml"),
            ("referer", "https://example.com/elsewhere"),
        ],
    );
    assert_eq!(
        intended_after_guest(html, None).await.as_deref(),
        Some("/reports")
    );
}

#[tokio::test]
#[serial]
async fn guest_stores_the_previous_url_for_a_post() {
    let request = Request::for_test_with_headers(
        "POST",
        "/invoices?draft=1",
        [("referer", "https://example.com/from-referer")],
    );
    assert_eq!(
        intended_after_guest(request, Some("/dashboard?tab=1"))
            .await
            .as_deref(),
        Some("/dashboard?tab=1")
    );
    let delete = Request::for_test("DELETE", "/invoices/7");
    assert_eq!(
        intended_after_guest(delete, Some("/invoices/7"))
            .await
            .as_deref(),
        Some("/invoices/7")
    );
}

#[tokio::test]
#[serial]
async fn guest_stores_the_previous_url_for_a_get_that_expects_json() {
    let wants_json =
        Request::for_test_with_headers("GET", "/api/invoices", [("accept", "application/json")]);
    assert_eq!(
        intended_after_guest(wants_json, Some("/dashboard"))
            .await
            .as_deref(),
        Some("/dashboard")
    );
    let ajax = Request::for_test_with_headers(
        "GET",
        "/api/invoices",
        [("x-requested-with", "XMLHttpRequest"), ("accept", "*/*")],
    );
    assert_eq!(
        intended_after_guest(ajax, Some("/dashboard"))
            .await
            .as_deref(),
        Some("/dashboard")
    );
}

#[tokio::test]
#[serial]
async fn guest_never_stores_the_referer() {
    // A POST with no previous URL recorded stores nothing, and removes a
    // stale intended URL, rather than reading the Referer.
    let request = Request::for_test_with_headers(
        "POST",
        "/invoices",
        [("referer", "https://evil.example/phish")],
    );
    let slot = new_session_slot_for_test();
    let intended = session_scope_for_test(slot, async {
        session_mut(|s| s.put("url.intended", "/stale"));
        let location = into_location(Redirect::guest(&request, "/login"));
        assert_eq!(location, "/login");
        session().and_then(|s| s.get::<String>("url.intended"))
    })
    .await;
    assert_eq!(intended, None);

    let same_site_referer =
        Request::for_test_with_headers("POST", "/invoices", [("referer", "/from-referer")]);
    assert_eq!(intended_after_guest(same_site_referer, None).await, None);
}

#[tokio::test]
#[serial]
async fn guest_keeps_the_same_site_check_for_both_sources() {
    // A previous URL a browser would read as another host, written raw as
    // an earlier release could have, is not stored.
    let request = Request::for_test("POST", "/invoices");
    let slot = new_session_slot_for_test();
    let intended = session_scope_for_test(slot, async {
        session_mut(|s| s.put("_previous.url", "//evil.example/x"));
        let _ = into_location(Redirect::guest(&request, "/login"));
        session().and_then(|s| s.get::<String>("url.intended"))
    })
    .await;
    assert_eq!(intended, None);

    // A GET whose own target names another host is not stored either.
    let network_path = Request::for_test("GET", "//evil.example/x");
    assert_eq!(
        intended_after_guest(network_path, Some("/dashboard")).await,
        None
    );
}
