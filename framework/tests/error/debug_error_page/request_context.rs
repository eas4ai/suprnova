//! PAR-014: the page shows the request's method, path, query, headers,
//! matched route pattern and request id. It redacts credentials, cookies,
//! secret-named headers and query parameters, and URL passwords in the
//! error chain. It never shows the request body, environment variables or
//! configuration values.

use serial_test::serial;

use suprnova::{MiddlewareRegistry, Request, Response, Router};

use super::{
    BROWSER_ACCEPT, INVOICE_ERROR, assert_debug_page, assert_page_shows, chain, debug_mode,
    exchange, get, post_invoice, request,
};

/// The route pattern `/ledger/accounts/42` matches.
const ACCOUNT_ROUTE: &str = "/ledger/accounts/{id}";

/// The request id the client sends. `RequestIdMiddleware` keeps a
/// well-formed inbound id.
const REQUEST_ID: &str = "rid-debug-page-7f3a";

/// A header and a query parameter that hold nothing secret, so the page
/// shows them as they are.
const VISIBLE_HEADER: (&str, &str) = ("X-Trace-Note", "visible-trace-note");
const VISIBLE_QUERY_VALUE: &str = "visible-query-value";

/// Every secret this file sends carries this marker, so one search finds
/// any of them on the page.
const SECRET_MARKER: &str = "s3cr3t";

fn account_routes() -> Router {
    Router::new()
        .get(ACCOUNT_ROUTE, post_invoice)
        .post("/ledger/entries", record_entry)
        .get("/ledger/replica", |_req: Request| async {
            let response: Response = Err(chain(
                "connecting to postgres://app:hunter2@db/app failed",
                "the replica refused the session",
                "the server at postgres://ledger:swordfish@replica:5432/app closed the connection",
            )
            .into());
            response
        })
        .into()
}

/// Reads the request body, then fails.
async fn record_entry(req: Request) -> Response {
    let _entry: serde_json::Value = req.json().await?;
    Err(chain(
        INVOICE_ERROR,
        "the entry was rejected",
        "the ledger is read-only",
    )
    .into())
}

/// Where `text` sits on the page, raw or with character references
/// decoded, with some context on each side; `None` when it is not there.
fn find_on_page(page: &str, decoded: &str, text: &str) -> Option<String> {
    context_of(page, text).or_else(|| context_of(decoded, text))
}

fn context_of(haystack: &str, needle: &str) -> Option<String> {
    let at = haystack.find(needle)?;
    let mut start = at.saturating_sub(80);
    while !haystack.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = (at + needle.len() + 80).min(haystack.len());
    while !haystack.is_char_boundary(end) {
        end += 1;
    }
    Some(haystack[start..end].to_string())
}

#[tokio::test]
#[serial]
async fn the_page_shows_the_method_path_query_headers_route_pattern_and_request_id() {
    let _debug = debug_mode(true, &[]).await;

    let reply = get(
        account_routes(),
        &format!("/ledger/accounts/42?view={VISIBLE_QUERY_VALUE}"),
        &[
            ("Accept", BROWSER_ACCEPT),
            ("X-Request-Id", REQUEST_ID),
            VISIBLE_HEADER,
        ],
    )
    .await;

    assert_debug_page(&reply, 500);
    assert_eq!(
        reply.header("x-request-id"),
        Some(REQUEST_ID),
        "the response echoes the request id the client sent"
    );
    assert_page_shows(
        &reply,
        &[
            "GET",
            "/ledger/accounts/42",
            VISIBLE_QUERY_VALUE,
            VISIBLE_HEADER.1,
            ACCOUNT_ROUTE,
            REQUEST_ID,
        ],
    );
    assert!(
        reply.text().to_ascii_lowercase().contains("x-trace-note"),
        "the page must name the request's headers; page:\n{}",
        reply.body
    );
}

#[tokio::test]
#[serial]
async fn a_request_carrying_credentials_yields_a_page_without_their_values() {
    let _debug = debug_mode(true, &[]).await;

    let target = format!(
        "/ledger/accounts/42?token={SECRET_MARKER}-query-token\
         &api_KEY={SECRET_MARKER}-query-key\
         &Signature={SECRET_MARKER}-query-signature\
         &client_secret={SECRET_MARKER}-query-secret\
         &PassWord={SECRET_MARKER}-query-password\
         &view={VISIBLE_QUERY_VALUE}"
    );
    let bearer = format!("Bearer {SECRET_MARKER}-bearer");
    let proxy = format!("Basic {SECRET_MARKER}-proxy");
    let cookie =
        format!("suprnova_session={SECRET_MARKER}-session-cookie; theme=cookie-theme-value");
    let set_cookie = format!("remember={SECRET_MARKER}-set-cookie");
    let api_key = format!("{SECRET_MARKER}-api-key");
    let csrf = format!("{SECRET_MARKER}-csrf-token");
    let client_secret = format!("{SECRET_MARKER}-client-secret");
    let password = format!("{SECRET_MARKER}-password-header");
    let signature = format!("{SECRET_MARKER}-signature-header");

    let reply = get(
        account_routes(),
        &target,
        &[
            ("Accept", BROWSER_ACCEPT),
            ("Authorization", bearer.as_str()),
            ("Proxy-Authorization", proxy.as_str()),
            ("Cookie", cookie.as_str()),
            ("Set-Cookie", set_cookie.as_str()),
            ("X-Api-Key", api_key.as_str()),
            ("X-CSRF-TOKEN", csrf.as_str()),
            ("X-Client-Secret", client_secret.as_str()),
            ("X-PASSWORD-Reset", password.as_str()),
            ("X-Webhook-Signature", signature.as_str()),
            VISIBLE_HEADER,
        ],
    )
    .await;

    assert_debug_page(&reply, 500);
    let decoded = reply.text();
    for secret in [SECRET_MARKER, "cookie-theme-value"] {
        let found = find_on_page(&reply.body, &decoded, secret);
        assert!(
            found.is_none(),
            "the page shows {secret:?}, a credential's value: {found:?}"
        );
        assert!(
            !reply.wire_headers().contains(secret),
            "{secret:?} reached the response headers:\n{}",
            reply.wire_headers()
        );
    }
    // Redaction hides values, not the request: what is not secret stays.
    assert_page_shows(&reply, &[VISIBLE_HEADER.1, VISIBLE_QUERY_VALUE]);
}

#[tokio::test]
#[serial]
async fn a_url_password_in_the_error_chain_is_redacted() {
    let _debug = debug_mode(true, &[]).await;

    let reply = get(
        account_routes(),
        "/ledger/replica",
        &[("Accept", BROWSER_ACCEPT)],
    )
    .await;

    assert_debug_page(&reply, 500);
    let decoded = reply.text();
    for password in ["hunter2", "swordfish"] {
        let found = find_on_page(&reply.body, &decoded, password);
        assert!(
            found.is_none(),
            "the page shows the URL password {password:?}: {found:?}"
        );
    }
    // The rest of the chain is still shown.
    assert_page_shows(&reply, &["the replica refused the session", "db/app"]);
}

#[tokio::test]
#[serial]
async fn the_page_shows_no_request_body_environment_variable_or_configuration_value() {
    const ENV_CANARY: &str = "env-canary-7d1c";
    const APP_NAME_CANARY: &str = "config-canary-app-name";
    const APP_URL_CANARY: &str = "config-canary-url";
    const BODY_CANARY: &str = "body-canary-memo";
    let _debug = debug_mode(
        true,
        &[
            ("SUPRNOVA_DEBUG_PAGE_CANARY", ENV_CANARY),
            ("APP_NAME", APP_NAME_CANARY),
            ("APP_URL", "http://config-canary-url.test"),
        ],
    )
    .await;

    let body = format!("{{\"memo\":\"{BODY_CANARY}\"}}");
    let reply = exchange(
        account_routes(),
        MiddlewareRegistry::new(),
        request(
            "POST",
            "/ledger/entries",
            &[
                ("Accept", BROWSER_ACCEPT),
                ("Content-Type", "application/json"),
            ],
            &body,
        ),
    )
    .await;

    assert_debug_page(&reply, 500);
    assert_page_shows(&reply, &[INVOICE_ERROR]);
    let decoded = reply.text();
    for (what, canary) in [
        ("the request body", BODY_CANARY),
        ("an environment variable", ENV_CANARY),
        ("the configured APP_NAME", APP_NAME_CANARY),
        ("the configured APP_URL", APP_URL_CANARY),
    ] {
        let found = find_on_page(&reply.body, &decoded, canary);
        assert!(found.is_none(), "the page shows {what}: {found:?}");
    }
}
