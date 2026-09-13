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
    Post, SECURITY_HEADERS, boot_with_render_cache, counting_route, dispatch_get,
};
use suprnova::{ConnectionTrait, DB, Model, StatusCode, attrs};

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
/// CACHE-001: a handler's `Cache-Control: no-store` is a storage veto, and
/// the route policy never replaces it. The audit (ASTRA-02) rendered a
/// public cached route once and replayed it with
/// `public, max-age=60, s-maxage=60`, so a handler's own "do not store"
/// was both ignored and rewritten.
#[tokio::test]
#[serial_test::serial]
async fn no_store_is_a_storage_veto() {
    let harness = boot_with_render_cache().await;

    let first = dispatch_get(&harness, "/no-store", &[]).await;
    assert_eq!(first.status, StatusCode::OK);
    assert_eq!(
        first.header("cache-control"),
        Some("no-store"),
        "the render keeps the handler's own directive"
    );

    let second = dispatch_get(&harness, "/no-store", &[]).await;
    assert_eq!(second.status, StatusCode::OK);
    assert_eq!(
        counting_route::renders(),
        2,
        "a no-store response was stored and replayed"
    );
    assert_eq!(
        second.header("cache-control"),
        Some("no-store"),
        "the second render keeps the handler's own directive too"
    );
    assert_ne!(
        first.body, second.body,
        "each request rendered its own body"
    );
}
/// CACHE-002: a handler's `Vary` must agree with the declared key
/// dimensions, or the response is not stored. The audit (ASTRA-09) served
/// the `vanilla` body to a `chocolate` request from storage, with the
/// `Vary: X-Flavor` the handler declared dropped from the hit.
#[tokio::test]
#[serial_test::serial]
async fn vary_must_match_declared_dimensions() {
    let harness = boot_with_render_cache().await;

    let vanilla = dispatch_get(&harness, "/vary-undeclared", &[("x-flavor", "vanilla")]).await;
    assert_eq!(vanilla.status, StatusCode::OK);
    assert_eq!(&vanilla.body[..], b"vanilla");
    assert_eq!(vanilla.header("vary"), Some("X-Flavor"));

    let chocolate = dispatch_get(&harness, "/vary-undeclared", &[("x-flavor", "chocolate")]).await;
    assert_eq!(chocolate.status, StatusCode::OK);
    assert_eq!(
        &chocolate.body[..],
        b"chocolate",
        "one variant's body was served to another"
    );
    assert_eq!(
        chocolate.header("vary"),
        Some("X-Flavor"),
        "the handler's Vary contract reached the second response"
    );
    assert_eq!(
        counting_route::renders(),
        2,
        "an undeclared Vary field must not be stored under a key that omits it"
    );
}

/// CACHE-009: a data write and its generation advancement commit together
/// on the autocommit path, so a failure of either leaves neither. The
/// audit (ASTRA-10) removed the generation log table, issued a raw
/// `UPDATE`, and saw the row committed while the advancement failed: the
/// API returned an error, the data was durable, and the cached page kept
/// serving the pre-write body under the old generation.
///
/// After the fix the two share one transaction: the failed advancement
/// rolls the row write back, so the database and the cache agree.
#[tokio::test]
#[serial_test::serial]
async fn write_and_generation_commit_together() {
    let harness = boot_with_render_cache().await;
    let post = Post::create(attrs! { title: "before" })
        .await
        .expect("seed the row the route renders");
    let path = format!("/write-atomicity/{}", post.id);

    let first = dispatch_get(&harness, &path, &[]).await;
    assert_eq!(first.status, StatusCode::OK);
    assert_eq!(&first.body[..], b"before");

    // Through the raw connection, not `DB::statement`: the facade's own
    // write hook would try to advance generations for the drop itself.
    DB::connection()
        .expect("the harness connected the primary database")
        .inner()
        .execute_unprepared("DROP TABLE suprnova_render_generation_log")
        .await
        .expect("remove the generation log so advancement fails");
    let write = DB::statement(
        &format!("UPDATE posts SET title = 'after' WHERE id = {}", post.id),
        Vec::new(),
    )
    .await;
    assert!(
        write.is_err(),
        "a write whose advancement cannot be recorded reports failure"
    );

    let direct = Post::find(post.id)
        .await
        .expect("read the row back")
        .expect("the row still exists");
    assert_eq!(
        direct.title, "before",
        "the row write rolled back with its failed advancement"
    );
    let second = dispatch_get(&harness, &path, &[]).await;
    assert_eq!(
        &second.body[..],
        b"before",
        "the cache and the database agree after the failed write"
    );
}
