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
async fn pub_006_unpublished_path_renders_once_and_next_visit_reads_storage() {
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
