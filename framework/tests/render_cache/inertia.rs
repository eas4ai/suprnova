//! Protocol representations remain separate through the production cache middleware.
use crate::render_cache_middleware_support::{boot_with_inertia_routes, dispatch_get};
use std::sync::atomic::{AtomicUsize, Ordering};
use suprnova::render_cache::{FreshnessPolicy, RenderCachePolicy, RepresentationClass};
use suprnova::{InertiaResponse, Request, Response, Router};
static RENDERS: AtomicUsize = AtomicUsize::new(0);
async fn page(req: Request) -> Response {
    RENDERS.fetch_add(1, Ordering::SeqCst);
    InertiaResponse::new("Cache/Page")
        .with("a", 1)
        .with("b", 2)
        .resolve(&req)
        .await
        .map_err(Into::into)
}
async fn boot() -> std::sync::Arc<crate::render_cache_middleware_support::Harness> {
    RENDERS.store(0, Ordering::SeqCst);
    let router: Router = Router::new().get("/inertia-cache", page).into();
    let policy = RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .freshness(FreshnessPolicy::new(60_000, 0, 0).unwrap())
        .vary(suprnova::render_cache::VarianceDimension::Locale)
        .build()
        .unwrap();
    boot_with_inertia_routes(router.try_render_cache("/inertia-cache", policy).unwrap()).await
}
#[tokio::test]
#[serial_test::serial]
async fn cache_011_document_json_and_partial_sets_have_distinct_entries() {
    let h = boot().await;
    let json = [("X-Inertia", "true"), ("X-Inertia-Version", "cache-v1")];
    let doc = dispatch_get(&h, "/inertia-cache", &[]).await;
    assert!(String::from_utf8_lossy(&doc.body).contains("<!DOCTYPE html>"));
    let full = dispatch_get(&h, "/inertia-cache", &json).await;
    let full_page: serde_json::Value = serde_json::from_slice(&full.body).unwrap();
    assert_eq!(full_page["props"]["a"], 1);
    for (only, except, expected) in [("a", "", "a"), ("b", "", "b"), ("", "a", "b")] {
        let mut headers = json.to_vec();
        headers.extend([
            ("X-Inertia-Partial-Component", "Cache/Page"),
            ("X-Inertia-Partial-Data", only),
            ("X-Inertia-Partial-Except", except),
        ]);
        let first = dispatch_get(&h, "/inertia-cache", &headers).await;
        let page: serde_json::Value = serde_json::from_slice(&first.body).unwrap();
        assert!(page["props"].get(expected).is_some());
        assert!(
            page["props"]
                .get(if expected == "a" { "b" } else { "a" })
                .is_none()
        );
        let before = RENDERS.load(Ordering::SeqCst);
        let hit = dispatch_get(&h, "/inertia-cache", &headers).await;
        assert_eq!(hit.body, first.body);
        assert_eq!(hit.header("x-inertia"), Some("true"));
        assert_eq!(RENDERS.load(Ordering::SeqCst), before, "partial published");
    }
    let renders = RENDERS.load(Ordering::SeqCst);
    assert_eq!(dispatch_get(&h, "/inertia-cache", &[]).await.body, doc.body);
    assert_eq!(
        dispatch_get(&h, "/inertia-cache", &json).await.body,
        full.body
    );
    assert_eq!(
        RENDERS.load(Ordering::SeqCst),
        renders,
        "full representations published"
    );
}
#[tokio::test]
#[serial_test::serial]
async fn cache_011_stale_version_answers_before_lookup_even_when_installed_later() {
    let h = boot().await;
    let headers = [("X-Inertia", "1"), ("X-Inertia-Version", "cache-v1")];
    assert_eq!(
        dispatch_get(&h, "/inertia-cache", &headers)
            .await
            .status
            .as_u16(),
        200
    );
    suprnova::Inertia::version("cache-v2");
    let before = RENDERS.load(Ordering::SeqCst);
    let stale = dispatch_get(&h, "/inertia-cache", &headers).await;
    assert_eq!(stale.status.as_u16(), 409);
    assert_eq!(stale.header("x-inertia-version"), Some("cache-v2"));
    assert!(stale.header("age").is_none());
    assert_eq!(RENDERS.load(Ordering::SeqCst), before);
}

#[tokio::test]
#[serial_test::serial]
async fn cache_011_non_text_inertia_header_never_uses_the_document_key() {
    let h = boot().await;
    let document = dispatch_get(&h, "/inertia-cache", &[]).await;
    let json = crate::render_cache_middleware_support::dispatch_inertia_bytes(
        &h,
        "/inertia-cache",
        b"\xff",
    )
    .await;
    assert_eq!(json.status.as_u16(), 200);
    assert_eq!(json.header("x-inertia"), Some("true"));
    let page: serde_json::Value = serde_json::from_slice(&json.body).unwrap();
    assert_eq!(page["props"]["a"], 1);
    assert_eq!(
        dispatch_get(&h, "/inertia-cache", &[]).await.body,
        document.body
    );
    assert_eq!(RENDERS.load(Ordering::SeqCst), 2);
}

#[tokio::test]
#[serial_test::serial]
async fn cache_011_document_partial_headers_have_their_own_key_in_both_warming_orders() {
    for partial_first in [true, false] {
        let h = boot().await;
        let partial = [
            ("X-Inertia-Partial-Component", "Cache/Page"),
            ("X-Inertia-Partial-Data", "a"),
        ];
        let (first, second) = if partial_first {
            (partial.as_slice(), &[][..])
        } else {
            (&[][..], partial.as_slice())
        };
        let a = dispatch_get(&h, "/inertia-cache", first).await;
        let b = dispatch_get(&h, "/inertia-cache", second).await;
        assert_ne!(a.body, b.body);
        assert_eq!(RENDERS.load(Ordering::SeqCst), 2);
        assert_eq!(dispatch_get(&h, "/inertia-cache", first).await.body, a.body);
        assert_eq!(
            dispatch_get(&h, "/inertia-cache", second).await.body,
            b.body
        );
        assert_eq!(RENDERS.load(Ordering::SeqCst), 2);
    }
}
