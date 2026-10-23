use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use suprnova::static_publish::{PublishConfig, Publisher, StaticPublication};
use suprnova::testing::{TestClient, TestContainer};
use suprnova::{
    InertiaConfig, InertiaProps, InertiaResponse, MiddlewareRegistry, Request, Router, Storage,
};

#[derive(InertiaProps)]
#[inertia(cacheable)]
struct PublicProps {
    #[inertia(public)]
    title: String,
}

#[tokio::test]
async fn pub_006_fix_g_first_response_matches_stored() {
    let _container = TestContainer::fake();
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let router: Router = Router::new()
        .get("/articles/{slug}", move |request: Request| {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                InertiaResponse::new("Article")
                    .with_config(
                        InertiaConfig::new()
                            .version("fallback-v1")
                            .development(true)
                            .ssr_disabled(),
                    )
                    .cacheable(PublicProps {
                        title: "Public article".into(),
                    })
                    .unwrap()
                    .resolve(&request)
                    .await
                    .map_err(Into::into)
            }
        })
        .into();
    let router = router
        .public_page::<PublicProps>("/articles/{slug}", "Article")
        .unwrap();
    Storage::register_memory("fallback-publication");
    let runtime = StaticPublication::install(
        &router,
        Publisher::new(
            "fallback-publication",
            "fallback-v1",
            PublishConfig::default(),
        )
        .unwrap(),
    )
    .unwrap();
    let client = TestClient::new(router, MiddlewareRegistry::new());
    let first = client.get("/articles/first").send().await;
    first.assert_ok();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(!runtime.status().await.live_version.is_empty());
    let second = client.get("/articles/first").send().await;
    second.assert_ok();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(first.body_text(), second.body_text());
    for name in ["Cache-Control", "ETag", "Content-Length"] {
        assert_eq!(
            first.header(name),
            second.header(name),
            "first fallback {name} matches storage"
        );
    }
    assert!(first.header("ETag").is_some());
    let json = client.get("/articles/first/index.json").send().await;
    json.assert_ok();
    assert_eq!(json.header("X-Inertia"), Some("true"));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(json.body_text().contains("Public article"));
}

#[tokio::test]
async fn pub_006_origin_keeps_rendering_without_publication() {
    let _container = TestContainer::fake();
    let router = Router::new().get("/origin", |request: Request| async move {
        InertiaResponse::new("Origin")
            .with("message", "origin works")
            .resolve(&request)
            .await
            .map_err(Into::into)
    });
    let client = TestClient::new(router, MiddlewareRegistry::new());
    client
        .get("/origin")
        .send()
        .await
        .assert_ok()
        .assert_see("origin works");
}

#[tokio::test]
#[serial_test::serial]
async fn pub_006_fix_i_request_directives_are_case_insensitive() {
    let _container = TestContainer::fake();
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let router: Router = Router::new()
        .get("/public", move |request: Request| {
            let count = count.clone();
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                InertiaResponse::new("Public")
                    .with_config(
                        InertiaConfig::new()
                            .version("v1")
                            .development(true)
                            .ssr_disabled(),
                    )
                    .cacheable(PublicProps {
                        title: "Public".into(),
                    })
                    .unwrap()
                    .resolve(&request)
                    .await
                    .map_err(Into::into)
            }
        })
        .into();
    let router = router
        .public_page::<PublicProps>("/public", "Public")
        .unwrap();
    Storage::register_memory("request-directives");
    let runtime = StaticPublication::install(
        &router,
        Publisher::new("request-directives", "v1", PublishConfig::default()).unwrap(),
    )
    .unwrap();
    runtime.publish().await.unwrap();
    let client = TestClient::new(router, MiddlewareRegistry::new());
    for directive in ["No-Store", "PUBLIC, NO-CACHE", "max-age=0, nO-sToRe"] {
        let before = calls.load(Ordering::SeqCst);
        client
            .get("/public")
            .header("Cache-Control", directive)
            .send()
            .await
            .assert_ok();
        assert_eq!(
            calls.load(Ordering::SeqCst),
            before + 1,
            "{directive} bypasses storage"
        );
    }
}
