//! Vite inventory dependencies pass through the same raster boundary as HTML references.
use suprnova::static_publish::{PublishConfig, Publisher, StaticPublication};
use suprnova::{InertiaConfig, InertiaProps, InertiaResponse, Request, Router};
#[suprnova::inertia_root(path = "static_publish/vite-raster.html")]
struct ViteRoot;
#[derive(InertiaProps)]
#[inertia(cacheable)]
struct Props {
    #[inertia(public)]
    title: String,
}
const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0xd, 0xa, 0x1a, 0xa, 0x0, 0x0, 0x0, 0xd, 0x49, 0x48, 0x44, 0x52, 0x0,
    0x0, 0x0, 0x1, 0x0, 0x0, 0x0, 0x1, 0x8, 0x6, 0x0, 0x0, 0x0, 0x1f, 0x15, 0xc4, 0x89, 0x0, 0x0,
    0x0, 0x18, 0x74, 0x45, 0x58, 0x74, 0x43, 0x6f, 0x6d, 0x6d, 0x65, 0x6e, 0x74, 0x0, 0x70, 0x72,
    0x69, 0x76, 0x61, 0x74, 0x65, 0x20, 0x6d, 0x65, 0x74, 0x61, 0x64, 0x61, 0x74, 0x61, 0x9e, 0xcc,
    0x9c, 0xe6, 0x0, 0x0, 0x0, 0xd, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x10, 0x50, 0x30,
    0xf8, 0xf, 0x0, 0x2, 0x4, 0x1, 0x60, 0x8d, 0xbc, 0xbb, 0x71, 0x0, 0x0, 0x0, 0x0, 0x49, 0x45,
    0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];
async fn inventory() -> (std::sync::Arc<StaticPublication>, opendal::Operator) {
    let source = opendal::Operator::new(opendal::services::Memory::default()).unwrap();
    source.write("assets/bg.png", PNG).await.unwrap();
    source
        .write("assets/entry-AbC12345.js", b"export {};".as_slice())
        .await
        .unwrap();
    source
        .write(
            "assets/.vite/manifest.json",
            br#"{"entry":{"file":"entry-AbC12345.js","assets":["bg.png"]}}"#.as_slice(),
        )
        .await
        .unwrap();
    let router: Router = Router::new()
        .get("/vite-inventory", |req: Request| async move {
            InertiaResponse::new("Public")
                .with_config(
                    InertiaConfig::new()
                        .version("v1")
                        .development(true)
                        .ssr_disabled()
                        .root_template(suprnova::InertiaRootTemplate::of::<ViteRoot>()),
                )
                .cacheable(Props {
                    title: "Public".into(),
                })
                .map_err(suprnova::HttpResponse::from)?
                .resolve(&req)
                .await
                .map_err(suprnova::HttpResponse::from)
        })
        .into();
    let router = router
        .public_page::<Props>("/vite-inventory", "Public")
        .unwrap();
    let disk = opendal::Operator::new(opendal::services::Memory::default()).unwrap();
    let runtime = StaticPublication::install(
        &router,
        Publisher::from_operator(disk.clone(), "v1", PublishConfig::default()).unwrap(),
    )
    .unwrap();
    runtime.assets(source).unwrap();
    (runtime, disk)
}
#[cfg(feature = "media")]
#[tokio::test]
#[serial_test::serial]
async fn pub_007_round2_6_vite_only_raster_is_stripped_at_its_key() {
    let _container = suprnova::testing::TestContainer::fake();
    let (runtime, disk) = inventory().await;
    runtime.publish().await.unwrap();
    let manifest: suprnova::static_publish::Manifest =
        serde_json::from_slice(&disk.read("_pub/manifest.json").await.unwrap().to_bytes()).unwrap();
    let object = manifest.versions[&manifest.live]
        .objects
        .iter()
        .find(|o| o.url == "/assets/bg.png")
        .unwrap();
    let bytes = disk.read(&object.path).await.unwrap().to_bytes();
    assert_ne!(
        bytes.as_ref(),
        PNG,
        "Vite-only rasters cannot bypass metadata stripping"
    );
    assert!(
        !bytes
            .windows(b"private metadata".len())
            .any(|v| v == b"private metadata")
    );
    assert_eq!(
        image::load_from_memory(&bytes).unwrap().to_rgba8(),
        image::load_from_memory(PNG).unwrap().to_rgba8()
    );
}
#[cfg(not(feature = "media"))]
#[tokio::test]
#[serial_test::serial]
async fn pub_007_round2_6_without_media_refuses_vite_only_rasters() {
    let _container = suprnova::testing::TestContainer::fake();
    let (runtime, _) = inventory().await;
    let cause = runtime.publish().await.unwrap_err().to_string();
    assert!(
        cause.contains("/vite-inventory") && cause.contains("media"),
        "{cause}"
    );
}

#[suprnova::inertia_root(path = "static_publish/metadata-links.html")]
struct MetadataRoot;

#[tokio::test]
async fn pub_002_metadata_links_are_not_read_or_rewritten_as_assets() {
    let _container = suprnova::testing::TestContainer::fake();
    let source = opendal::Operator::new(opendal::services::Memory::default()).unwrap();
    // These are the only source bytes. Reading either route as a file would fail.
    source
        .write("theme.css", b"body{}".as_slice())
        .await
        .unwrap();
    let router: Router = Router::new()
        .get("/metadata", |request: Request| async move {
            InertiaResponse::new("Public")
                .with_config(
                    InertiaConfig::new()
                        .version("v1")
                        .development(true)
                        .ssr_disabled()
                        .vite_dev_server("http://localhost:5173")
                        .root_template(suprnova::InertiaRootTemplate::of::<MetadataRoot>()),
                )
                .cacheable(Props {
                    title: "Article".into(),
                })
                .unwrap()
                .resolve(&request)
                .await
                .map_err(Into::into)
        })
        .into();
    let router = router.public_page::<Props>("/metadata", "Public").unwrap();
    let disk = opendal::Operator::new(opendal::services::Memory::default()).unwrap();
    let runtime = StaticPublication::install(
        &router,
        Publisher::from_operator(disk.clone(), "v1", PublishConfig::default()).unwrap(),
    )
    .unwrap();
    runtime.assets(source).unwrap();
    runtime.publish().await.unwrap();
    let manifest: suprnova::static_publish::Manifest =
        serde_json::from_slice(&disk.read("_pub/manifest.json").await.unwrap().to_bytes()).unwrap();
    let objects = &manifest.versions[&manifest.live].objects;
    let page = objects
        .iter()
        .find(|object| object.path.ends_with(".html"))
        .unwrap();
    let bytes = disk.read(&page.path).await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&bytes);
    assert!(
        html.contains(r#"<link rel="canonical" href="/article">"#),
        "{html}"
    );
    assert!(
        html.contains(r#"<link rel="alternate" hreflang="fr" href="/fr/article">"#),
        "{html}"
    );
    let style = objects
        .iter()
        .find(|object| object.url == "/theme.css")
        .unwrap();
    assert_ne!(style.path, "theme.css");
    assert!(html.contains(&format!("/{}", style.path)), "{html}");
    assert!(objects.iter().all(|object| !object.url.contains("article")));
}
