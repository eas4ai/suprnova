use suprnova::static_publish::{PublishConfig, Publisher, StaticPublication};
use suprnova::{InertiaConfig, InertiaProps, InertiaResponse, Request, Router};

#[suprnova::inertia_root(path = "static_publish/document-css.html")]
struct CssDocument;
#[derive(InertiaProps)]
#[inertia(cacheable)]
struct Props {
    #[inertia(public)]
    title: String,
}

async fn document(
    kind: &'static str,
    css: String,
) -> (std::sync::Arc<StaticPublication>, Router, opendal::Operator) {
    let content = css.clone();
    let router: Router = Router::new()
        .get("/css-page", move |request: Request| {
            let css = content.clone();
            async move {
                let response = InertiaResponse::new("Css")
                    .with_config(
                        InertiaConfig::new()
                            .development(true)
                            .version("v1")
                            .ssr_disabled()
                            .vite_dev_server("http://localhost:5173")
                            .root_template(suprnova::InertiaRootTemplate::of::<CssDocument>()),
                    )
                    .with_view_data(
                        kind,
                        if kind == "stylesheet" {
                            "/theme.css".to_string()
                        } else {
                            css
                        },
                    )
                    .cacheable(Props {
                        title: "CSS page".into(),
                    })
                    .unwrap();
                response.resolve(&request).await.map_err(Into::into)
            }
        })
        .into();
    let router = router.public_page::<Props>("/css-page", "Css").unwrap();
    let disk = opendal::Operator::new(opendal::services::Memory::default()).unwrap();
    let runtime = StaticPublication::install(
        &router,
        Publisher::from_operator(disk.clone(), "v1", PublishConfig::default()).unwrap(),
    )
    .unwrap();
    if kind == "stylesheet" {
        let source = opendal::Operator::new(opendal::services::Memory::default()).unwrap();
        source.write("theme.css", css).await.unwrap();
        runtime.assets(source).unwrap();
    }
    (runtime, router, disk)
}

#[tokio::test]
async fn pub_007_review_document_css_rasters_refuse_and_origin_keeps_serving() {
    if crate::own_process_async::delegate(
        module_path!(),
        "pub_007_review_document_css_rasters_refuse_and_origin_keeps_serving",
    )
    .await
    {
        return;
    }
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    let _container = suprnova::testing::TestContainer::fake();
    let jpeg = STANDARD.encode(include_bytes!(
        "../media/fixtures/jpeg/photo-base-444-35x21.jpg"
    ));
    for (kind, css, element) in [
        (
            "attribute",
            format!("background-image:url(data:image/jpeg;base64,{jpeg})"),
            "<div>",
        ),
        (
            "style",
            "body { background-image:url(/images/a.png) }".to_string(),
            "<style>",
        ),
        (
            "style",
            format!("body {{ background-image:image-set(\"data:image/jpeg;base64,{jpeg}\" 1x) }}"),
            "<style>",
        ),
        (
            "attribute",
            r#"background-image:u\72l('http://localhost/images/a.png')"#.to_string(),
            "<div>",
        ),
    ] {
        let (runtime, router, disk) = document(kind, css).await;
        let cause = runtime
            .publish()
            .await
            .expect_err("document CSS raster refuses publication")
            .to_string();
        for expected in [
            "/css-page",
            element,
            "<img>",
            "<picture>",
            "stylesheet file",
        ] {
            assert!(cause.contains(expected), "missing {expected}: {cause}");
        }
        assert!(!disk.exists("_pub/manifest.json").await.unwrap());
        let response =
            suprnova::testing::TestClient::new(router, suprnova::MiddlewareRegistry::new())
                .get("/css-page")
                .send()
                .await;
        response.assert_ok();
        assert!(response.body_text().contains("background-image"));
    }
}

#[tokio::test]
async fn pub_007_review_linked_stylesheet_keeps_inline_png_bytes() {
    if crate::own_process_async::delegate(
        module_path!(),
        "pub_007_review_linked_stylesheet_keeps_inline_png_bytes",
    )
    .await
    {
        return;
    }
    let _container = suprnova::testing::TestContainer::fake();
    let css = "body { background-image:url(data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a5XcAAAAASUVORK5CYII=) }";
    let (runtime, _, disk) = document("stylesheet", css.to_string()).await;
    runtime.publish().await.unwrap();
    let manifest: suprnova::static_publish::Manifest =
        serde_json::from_slice(&disk.read("_pub/manifest.json").await.unwrap().to_bytes()).unwrap();
    let asset = manifest.versions[&manifest.live]
        .objects
        .iter()
        .find(|object| object.url == "/theme.css")
        .unwrap();
    assert_eq!(
        disk.read(&asset.path).await.unwrap().to_bytes().as_ref(),
        css.as_bytes()
    );
}

#[tokio::test]
async fn pub_007_review_non_raster_document_css_is_allowed() {
    if crate::own_process_async::delegate(
        module_path!(),
        "pub_007_review_non_raster_document_css_is_allowed",
    )
    .await
    {
        return;
    }
    let _container = suprnova::testing::TestContainer::fake();
    let css = r#"/* url(/images/a.png) */ body::before {content:'url(/images/a.png)'} body {background:url('https://outside.example/a.png');mask:url('data:image/svg+xml,%3Csvg/%3E')}"#;
    let (runtime, _, _) = document("style", css.to_string()).await;
    runtime.publish().await.unwrap();
}
