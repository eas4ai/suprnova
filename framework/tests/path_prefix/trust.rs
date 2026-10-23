//! PFX-001: `X-Forwarded-Prefix` counts only from a trusted proxy, and only
//! when it passes the value rule.

use suprnova::{HttpResponse, MiddlewareRegistry, Request, Router};

use crate::support::{self, PREFIX};

/// A route that answers with the root and with what `route()` builds for
/// itself, the two places a prefix would reach a URL.
fn probe_router() -> Router {
    Router::new()
        .get("/probe", |_request: Request| async {
            let route = suprnova::route("pfx.probe", &[]).expect("the probe route is named");
            Ok(HttpResponse::text(format!(
                "{}|{route}",
                suprnova::url::root()
            )))
        })
        .name("pfx.probe")
}

async fn probe(app_url: &str, trusted: bool, headers: &[(&str, &[u8])]) -> String {
    if trusted {
        support::install(app_url);
    } else {
        support::install_untrusted(app_url);
    }
    let address = support::serve(probe_router(), MiddlewareRegistry::new()).await;
    let reply = support::send(address, "GET", "/probe", headers, b"").await;
    assert_eq!(reply.status, 200, "{}", reply.body);
    reply.body
}

#[tokio::test]
async fn pfx_001_a_valid_prefix_from_a_trusted_proxy_becomes_the_root() {
    if crate::own_process_async::delegate(
        module_path!(),
        "pfx_001_a_valid_prefix_from_a_trusted_proxy_becomes_the_root",
    )
    .await
    {
        return;
    }
    let body = probe(
        "http://localhost",
        true,
        &[("x-forwarded-prefix", PREFIX.as_bytes())],
    )
    .await;
    assert_eq!(body, "/billing|/billing/probe");
}

#[tokio::test]
async fn pfx_001_a_prefix_from_an_untrusted_peer_reaches_no_url() {
    if crate::own_process_async::delegate(
        module_path!(),
        "pfx_001_a_prefix_from_an_untrusted_peer_reaches_no_url",
    )
    .await
    {
        return;
    }
    let body = probe(
        "http://localhost",
        false,
        &[("x-forwarded-prefix", PREFIX.as_bytes())],
    )
    .await;
    assert_eq!(body, "|/probe");
}

#[tokio::test]
async fn pfx_001_a_value_that_breaks_the_rule_reaches_no_url() {
    if crate::own_process_async::delegate(
        module_path!(),
        "pfx_001_a_value_that_breaks_the_rule_reaches_no_url",
    )
    .await
    {
        return;
    }
    support::install("http://localhost");
    let address = support::serve(probe_router(), MiddlewareRegistry::new()).await;
    let refused: [&[u8]; 9] = [
        b"%2e%2e",
        b"/%2e%2e",
        "/caf\u{e9}".as_bytes(),
        b"/a/../b",
        b"/a/./b",
        b"/a//b",
        b"/billing/",
        b"billing",
        b"/a,/b",
    ];
    for value in refused {
        let reply = support::send(
            address,
            "GET",
            "/probe",
            &[("x-forwarded-prefix", value)],
            b"",
        )
        .await;
        assert_eq!(
            reply.body,
            "|/probe",
            "{:?} reached a URL",
            String::from_utf8_lossy(value)
        );
    }
}

#[tokio::test]
async fn pfx_001_a_prefix_sent_on_two_lines_is_ignored_whole() {
    if crate::own_process_async::delegate(
        module_path!(),
        "pfx_001_a_prefix_sent_on_two_lines_is_ignored_whole",
    )
    .await
    {
        return;
    }
    let body = probe(
        "http://localhost",
        true,
        &[
            ("x-forwarded-prefix", b"/evil"),
            ("x-forwarded-prefix", PREFIX.as_bytes()),
        ],
    )
    .await;
    assert_eq!(body, "|/probe");
}

#[tokio::test]
async fn pfx_001_a_trusted_slash_makes_the_root_the_host_root() {
    if crate::own_process_async::delegate(
        module_path!(),
        "pfx_001_a_trusted_slash_makes_the_root_the_host_root",
    )
    .await
    {
        return;
    }
    let body = probe(
        "https://example.org/app",
        true,
        &[("x-forwarded-prefix", b"/")],
    )
    .await;
    assert_eq!(body, "|/probe", "a trusted `/` must drop the APP_URL path");
}

#[tokio::test]
async fn pfx_001_a_trusted_empty_value_is_ignored() {
    if crate::own_process_async::delegate(
        module_path!(),
        "pfx_001_a_trusted_empty_value_is_ignored",
    )
    .await
    {
        return;
    }
    let body = probe(
        "https://example.org/app",
        true,
        &[("x-forwarded-prefix", b"")],
    )
    .await;
    assert_eq!(body, "/app|/app/probe", "an empty value must leave APP_URL");
}
