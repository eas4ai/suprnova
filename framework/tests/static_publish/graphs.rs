//! A published document must retain the complete relative asset graph it uses.
use super::*;

#[tokio::test]
#[serial_test::serial]
async fn pub_002_round2_3_mixed_entrypoints_resolve_mutable_dependencies() {
    let _container = suprnova::testing::TestContainer::fake();
    let router = asset_router();
    let (p, disk) = publisher("v1");
    let mut rendered = page("mixed graph");
    rendered.html =
        Bytes::from_static(br#"<script type="module" src="/bundle-AbC12345.js"></script>"#);
    rendered.assets.insert(
        "bundle-AbC12345.js".into(),
        (
            Bytes::from_static(b"new URL('./bg.png', import.meta.url)"),
            "application/javascript".into(),
        ),
    );
    rendered.assets.insert(
        "bg.png".into(),
        (Bytes::from_static(b"pixels"), "image/png".into()),
    );
    p.publish(vec![rendered]).await.unwrap();
    assert!(
        disk.exists("bundle-AbC12345.js").await.unwrap(),
        "fingerprinted names remain available"
    );
    let manifest = p.manifest().await.unwrap();
    let html = manifest.versions[&manifest.live]
        .objects
        .iter()
        .find(|object| object.url == "/about")
        .unwrap();
    let html = String::from_utf8(disk.read(&html.path).await.unwrap().to_bytes().to_vec()).unwrap();
    let script = html
        .split("src=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    let entry = url::Url::parse(&format!("https://public.test{script}")).unwrap();
    let dependency = entry.join("./bg.png").unwrap();
    suprnova::static_publish::StaticPublication::install(
        &router,
        Publisher::from_operator(disk, "v1", PublishConfig::default()).unwrap(),
    )
    .unwrap();
    let client = suprnova::testing::TestClient::new(router, suprnova::MiddlewareRegistry::new());
    let response = client.get(dependency.path()).send().await;
    response.assert_ok();
    assert_eq!(response.body_text(), "pixels");
}
#[tokio::test]
#[serial_test::serial]
async fn pub_008_round2_3_partial_pages_keep_their_old_asset_graph_until_replaced() {
    let _container = suprnova::testing::TestContainer::fake();
    #[derive(suprnova::InertiaProps)]
    #[inertia(cacheable)]
    struct Props {
        #[inertia(public)]
        title: String,
    }
    let router: suprnova::Router = suprnova::Router::new()
        .get("/a", |_| async {
            Ok(suprnova::HttpResponse::text("unexpected"))
        })
        .get("/b", |_| async {
            Ok(suprnova::HttpResponse::text("unexpected"))
        })
        .into();
    let router = router
        .public_page::<Props>("/a", "About")
        .unwrap()
        .public_page::<Props>("/b", "About")
        .unwrap();
    let disk = memory();
    let config = PublishConfig {
        grace_period: Duration::ZERO,
        ..Default::default()
    };
    let p = Publisher::from_operator(disk.clone(), "v1", config.clone()).unwrap();
    let make = |path: &str, css: &'static [u8]| {
        let mut page = page_at(path, "graph");
        page.html = Bytes::from_static(br#"<link rel="stylesheet" href="/theme.css">"#);
        page.assets.insert(
            "theme.css".into(),
            (Bytes::from_static(css), "text/css".into()),
        );
        page.assets.insert(
            "bg.png".into(),
            (Bytes::from_static(b"pixels"), "image/png".into()),
        );
        page
    };
    p.publish(vec![
        make("/a", b"body{background:url(./bg.png)}"),
        make("/b", b"body{background:url(./bg.png)}"),
    ])
    .await
    .unwrap();
    let old = p.manifest().await.unwrap();
    let old_paths: Vec<_> = old.versions[&old.live]
        .objects
        .iter()
        .filter(|object| object.asset)
        .map(|object| object.path.clone())
        .collect();
    p.publish_partial(vec![make(
        "/a",
        b"body{color:red;background:url(./bg.png)}",
    )])
    .await
    .unwrap();
    suprnova::static_publish::StaticPublication::install(
        &router,
        Publisher::from_operator(disk.clone(), "v1", config).unwrap(),
    )
    .unwrap();
    let client = suprnova::testing::TestClient::new(router, suprnova::MiddlewareRegistry::new());
    client.get("/b").send().await.assert_ok();
    for path in &old_paths {
        client.get(format!("/{path}")).send().await.assert_ok();
    }
    p.publish(vec![
        make("/a", b"body{color:red;background:url(./bg.png)}"),
        make("/b", b"body{color:red;background:url(./bg.png)}"),
    ])
    .await
    .unwrap();
    for path in old_paths {
        assert!(
            !disk.exists(&path).await.unwrap(),
            "a full replacement releases the old graph: {path}"
        );
    }
}
