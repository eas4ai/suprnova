//! PAR-015: the page is one HTML document that requests nothing and runs
//! no JavaScript. It renders when the app has no frontend build or Vite
//! manifest, and when Inertia is what failed. Every value it shows is
//! HTML-escaped, and the response carries `Cache-Control: no-store` and a
//! `Content-Security-Policy` that allows no script.

use serial_test::serial;

use suprnova::http::text;
use suprnova::testing::TestContainer;
use suprnova::{
    App, FrameworkError, HttpResponse, InertiaConfig, InertiaErrorPageMiddleware, InertiaResponse,
    MiddlewareRegistry, Request, Response, Router,
};

use super::{
    BROWSER, BROWSER_ACCEPT, INVOICE_ERROR, Reply, SessionScope, assert_debug_page,
    assert_page_shows, chain, debug_mode, exchange, get, ledger_routes, post_invoice, request,
};

/// Markup in an error message, a header and a panic message. Shown
/// unescaped, each would run or load something.
const MESSAGE_MARKUP: &str = "<script>alert(1)</script>";
const HEADER_MARKUP: &str = "<img src=x onerror=alert(2)>";
const PANIC_MARKUP: &str = "<script>alert(3)</script>";

/// What the app's shared Inertia prop fails with, failing every Inertia
/// render: the handler's page and the app's Inertia error page alike.
const SHARED_PROP_ERROR: &str = "loading the ledger totals for the shared props failed";

/// Where `InertiaConfig::default` looks for the Vite manifest, relative
/// to the test's working directory (the `framework` package).
const DEFAULT_MANIFEST: &str = "public/assets/.vite/manifest.json";

fn markup_routes() -> Router {
    Router::new()
        .get("/markup", |_req: Request| async {
            let response: Response =
                Err(chain(MESSAGE_MARKUP, "the memo was rejected", "memo too long").into());
            response
        })
        .get("/markup-panic", |_req: Request| async {
            text(reject_memo())
        })
        .into()
}

/// Panics with markup in its message.
fn reject_memo() -> String {
    panic!("{PANIC_MARKUP}");
}

/// `/posts` fails behind the app's Inertia error page.
fn inertia_error_page_routes() -> Router {
    Router::new()
        .get("/posts", post_invoice)
        .middleware(InertiaErrorPageMiddleware::new("Error"))
        .into()
}

/// `/dashboard` renders an Inertia page, behind the app's Inertia error
/// page.
fn dashboard_routes() -> Router {
    Router::new()
        .get("/dashboard", |req: Request| async move {
            InertiaResponse::new("Ledger/Dashboard")
                .resolve(&req)
                .await
                .map_err(HttpResponse::from)
        })
        .middleware(InertiaErrorPageMiddleware::new("Error"))
        .into()
}

/// The names of every attribute of every start tag in `html`.
///
/// Values may be quoted with `"` or `'` and hold spaces or `>`; the
/// scanner skips them whole so text inside a value is never read as an
/// attribute. Comments and the doctype are skipped.
fn attribute_names(html: &str) -> Vec<String> {
    let bytes = html.as_bytes();
    let mut names = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] != b'<' || !bytes.get(at + 1).is_some_and(u8::is_ascii_alphabetic) {
            at += 1;
            continue;
        }
        // The tag name.
        at += 1;
        while at < bytes.len()
            && !bytes[at].is_ascii_whitespace()
            && !matches!(bytes[at], b'>' | b'/')
        {
            at += 1;
        }
        // Its attributes, up to the `>` that ends the tag.
        loop {
            while at < bytes.len() && (bytes[at].is_ascii_whitespace() || bytes[at] == b'/') {
                at += 1;
            }
            if at >= bytes.len() || bytes[at] == b'>' {
                break;
            }
            let start = at;
            while at < bytes.len()
                && !bytes[at].is_ascii_whitespace()
                && !matches!(bytes[at], b'=' | b'>' | b'/')
            {
                at += 1;
            }
            names.push(html[start..at].to_ascii_lowercase());
            while at < bytes.len() && bytes[at].is_ascii_whitespace() {
                at += 1;
            }
            if bytes.get(at) == Some(&b'=') {
                at += 1;
                while at < bytes.len() && bytes[at].is_ascii_whitespace() {
                    at += 1;
                }
                match bytes.get(at) {
                    Some(&quote @ (b'"' | b'\'')) => {
                        at += 1;
                        while at < bytes.len() && bytes[at] != quote {
                            at += 1;
                        }
                        at += 1;
                    }
                    _ => {
                        while at < bytes.len()
                            && !bytes[at].is_ascii_whitespace()
                            && bytes[at] != b'>'
                        {
                            at += 1;
                        }
                    }
                }
            }
        }
    }
    names
}

/// Fail if the page could fetch anything or run any script.
fn assert_requests_nothing(reply: &Reply) {
    let lower = reply.body.to_ascii_lowercase();
    for needle in [
        "<script",
        "<link",
        "<img",
        "<iframe",
        "<frame",
        "<object",
        "<embed",
        "@import",
        "url(",
        "javascript:",
    ] {
        assert!(
            !lower.contains(needle),
            "the page must request nothing and run no script, and it holds {needle:?}; page:\n{}",
            reply.body
        );
    }
    for name in attribute_names(&reply.body) {
        assert!(
            !name.starts_with("on"),
            "the page must run no JavaScript, and a tag carries the event handler \
             attribute {name:?}; page:\n{}",
            reply.body
        );
        assert!(
            !matches!(name.as_str(), "src" | "srcset" | "poster" | "background"),
            "the page must request nothing, and a tag carries {name:?}; page:\n{}",
            reply.body
        );
    }
}

/// The source list a `Content-Security-Policy` applies to scripts:
/// `script-src` when the policy has one, `default-src` otherwise.
fn script_sources(policy: &str) -> Option<String> {
    let directives: Vec<(String, String)> = policy
        .split(';')
        .map(str::trim)
        .filter(|directive| !directive.is_empty())
        .map(|directive| {
            let (name, sources) = directive
                .split_once(char::is_whitespace)
                .unwrap_or((directive, ""));
            (
                name.to_ascii_lowercase(),
                sources.trim().to_ascii_lowercase(),
            )
        })
        .collect();
    let find = |wanted: &str| {
        directives
            .iter()
            .find(|(name, _)| name == wanted)
            .map(|(_, sources)| sources.clone())
    };
    find("script-src").or_else(|| find("default-src"))
}

#[tokio::test]
#[serial]
async fn the_page_requests_nothing_and_holds_no_script() {
    let _debug = debug_mode(true, &[]).await;

    for path in ["/invoice", "/ledger-index"] {
        let reply = get(ledger_routes(), path, BROWSER).await;

        assert_debug_page(&reply, 500);
        assert_requests_nothing(&reply);
    }
}

#[tokio::test]
#[serial]
async fn markup_in_an_error_message_a_header_or_a_panic_message_is_escaped() {
    let _debug = debug_mode(true, &[]).await;

    let error = get(
        markup_routes(),
        "/markup?view=%3Cb%3Ebold%3C%2Fb%3E",
        &[("Accept", BROWSER_ACCEPT), ("X-Trace-Note", HEADER_MARKUP)],
    )
    .await;
    assert_debug_page(&error, 500);
    for markup in ["<script", "<img", "<b>"] {
        assert!(
            !error.body.contains(markup),
            "{markup:?} reached the page unescaped; page:\n{}",
            error.body
        );
    }
    assert_page_shows(&error, &[MESSAGE_MARKUP, HEADER_MARKUP]);

    let panic = get(markup_routes(), "/markup-panic", BROWSER).await;
    assert_debug_page(&panic, 500);
    assert!(
        !panic.body.contains("<script"),
        "the panic message reached the page unescaped; page:\n{}",
        panic.body
    );
    assert_page_shows(&panic, &[PANIC_MARKUP]);
}

#[tokio::test]
#[serial]
async fn the_page_renders_for_an_app_with_no_frontend_build_or_vite_manifest() {
    // `APP_ENV=production` makes the Inertia layer production-shaped, so
    // it reads the Vite manifest; none was ever built in this checkout.
    let _debug = debug_mode(true, &[("APP_ENV", "production")]).await;
    let config = InertiaConfig::default();
    assert!(
        !config.development && config.vite_manifest().is_none(),
        "the Inertia layer must be production-shaped with no Vite manifest at {DEFAULT_MANIFEST}"
    );

    let reply = exchange(
        inertia_error_page_routes(),
        MiddlewareRegistry::new().append(SessionScope),
        request("GET", "/posts", BROWSER, ""),
    )
    .await;

    assert_debug_page(&reply, 500);
    assert_requests_nothing(&reply);
    assert_page_shows(&reply, &[INVOICE_ERROR]);
}

#[tokio::test]
#[serial]
async fn the_page_renders_when_the_inertia_render_itself_fails() {
    let _debug = debug_mode(true, &[]).await;

    let reply = TestContainer::scope(async {
        App::inertia_share_lazy("ledger_totals", || async {
            Err::<serde_json::Value, _>(FrameworkError::internal(SHARED_PROP_ERROR))
        });
        exchange(
            dashboard_routes(),
            MiddlewareRegistry::new().append(SessionScope),
            request("GET", "/dashboard", BROWSER, ""),
        )
        .await
    })
    .await;

    assert_debug_page(&reply, 500);
    assert_requests_nothing(&reply);
    assert_page_shows(&reply, &[SHARED_PROP_ERROR]);
}

#[tokio::test]
#[serial]
async fn the_page_carries_cache_control_no_store_and_a_csp_that_forbids_script() {
    let _debug = debug_mode(true, &[]).await;

    for path in ["/invoice", "/ledger-index"] {
        let reply = get(ledger_routes(), path, BROWSER).await;

        assert_debug_page(&reply, 500);
        assert!(
            reply.header_values("cache-control").any(|value| {
                value
                    .split(',')
                    .any(|directive| directive.trim().eq_ignore_ascii_case("no-store"))
            }),
            "the page must carry Cache-Control: no-store; headers:\n{}",
            reply.wire_headers()
        );
        assert!(
            reply
                .header_values("content-security-policy")
                .any(|policy| script_sources(policy).as_deref() == Some("'none'")),
            "the page must carry a Content-Security-Policy whose script-src, or default-src \
             when it has none, is 'none'; headers:\n{}",
            reply.wire_headers()
        );
    }
}
