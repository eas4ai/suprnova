//! Regression tests for the 2026-09-13 adversarial audit, one per agreed
//! requirement in `docs/spec/render-cache.md` (CACHE-001 to CACHE-010).
//!
//! Each test is the audit's own probe with its assertion inverted to the
//! agreed behavior, so each fails on the tree the audit examined and passes
//! once its fix lands. The mechanism under `.cairn/mechanisms/cache-*` runs
//! exactly one of these by name; the test name is the contract there, so a
//! rename here is a mechanism change.
//!
//! Every test is `#[serial_test::serial]` and plain `#[tokio::test]`
//! (current-thread) for the reasons `middleware.rs` gives: the installed
//! runtime and the global middleware registry are process-global.
#![cfg(feature = "testing")]

use crate::render_cache_middleware_support;
use render_cache_middleware_support::{
    SECURITY_HEADERS, boot_with_render_cache, counting_route, dispatch_get,
};
use suprnova::StatusCode;

/// CACHE-003: a response carrying a security header is either replayed
/// with that header byte for byte or never stored. The audit (ASTRA-11)
/// served an HTML attachment from storage without its `Content-Disposition`,
/// so bytes that downloaded on the render rendered inline on the hit.
///
/// The property asserted is the requirement's own: the second response
/// carries every one of the six headers with the first response's exact
/// value, whether it came from storage or from a fresh render. Which of
/// the two happened is reported, not asserted, because either satisfies
/// the requirement.
#[tokio::test]
#[serial_test::serial]
async fn security_headers_replay_or_decline() {
    let harness = boot_with_render_cache().await;

    let first = dispatch_get(&harness, "/security-headers", &[]).await;
    assert_eq!(first.status, StatusCode::OK);
    assert_eq!(
        counting_route::renders(),
        1,
        "the cold request rendered once"
    );
    for (name, value) in SECURITY_HEADERS {
        assert_eq!(
            first.header(name),
            Some(*value),
            "the render carried {name} as the handler set it"
        );
    }

    let second = dispatch_get(&harness, "/security-headers", &[]).await;
    assert_eq!(second.status, StatusCode::OK);
    assert_eq!(
        second.body, first.body,
        "the same representation was served both times"
    );
    for (name, value) in SECURITY_HEADERS {
        assert_eq!(
            second.header(name),
            Some(*value),
            "{name} survived the second request byte for byte (served {})",
            if counting_route::renders() == 1 {
                "from storage"
            } else {
                "by a fresh render"
            }
        );
    }
}
/// CACHE-004: a `Content-Security-Policy` that carries a nonce source is
/// never replayed from a complete entry. The audit (ASTRA-12) served the
/// same `script-src 'nonce-...'` header and the same body nonce on a cache
/// hit, so a public page's per-response secret became predictable for the
/// life of the entry.
///
/// The property asserted is the requirement's own: two requests never
/// share a nonce, in the header or in the body. The decision recorded for
/// this requirement declines storage rather than re-noncing, so the second
/// request is a fresh render; that is reported in the failure message, not
/// asserted, because re-noncing would satisfy the requirement too.
#[tokio::test]
#[serial_test::serial]
async fn csp_nonce_is_never_replayed() {
    let harness = boot_with_render_cache().await;

    let first = dispatch_get(&harness, "/csp-nonce", &[]).await;
    assert_eq!(first.status, StatusCode::OK);
    let first_csp = first
        .header("content-security-policy")
        .expect("the render declares its nonce in the CSP")
        .to_owned();
    assert!(
        first_csp.contains("'nonce-"),
        "precondition: the route mints a nonce source, saw {first_csp}"
    );

    let second = dispatch_get(&harness, "/csp-nonce", &[]).await;
    assert_eq!(second.status, StatusCode::OK);
    let second_csp = second
        .header("content-security-policy")
        .expect("the second response declares its own nonce")
        .to_owned();
    let served = if counting_route::renders() == 1 {
        "from storage"
    } else {
        "by a fresh render"
    };
    assert_ne!(
        first_csp, second_csp,
        "the CSP nonce was reused across requests (second served {served})"
    );
    assert_ne!(
        first.body, second.body,
        "the body nonce was reused across requests (second served {served})"
    );
}
