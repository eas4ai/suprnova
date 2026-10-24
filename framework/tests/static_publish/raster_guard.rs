//! Publication fails closed when raster preparation is unavailable.
use suprnova::static_publish::{PublishConfig, Publisher, StaticPublication};
use suprnova::{InertiaConfig, InertiaProps, InertiaResponse, Request, Router, Storage};
#[suprnova::inertia_root(path = "static_publish/rasters.html")]
struct RasterRoot;
#[derive(InertiaProps)]
#[inertia(cacheable)]
struct Props {
    #[inertia(public)]
    title: String,
}
#[tokio::test]
async fn pub_007_fix_k_without_media_refuses_rasters_with_route_and_reason() {
    let _container = suprnova::testing::TestContainer::fake();
    let router: Router = Router::new()
        .get("/raster-guard", |request: Request| async move {
            InertiaResponse::new("Public")
                .with_config(
                    InertiaConfig::new()
                        .version("v1")
                        .development(true)
                        .ssr_disabled()
                        .root_template(suprnova::InertiaRootTemplate::of::<RasterRoot>()),
                )
                .cacheable(Props {
                    title: "Public".into(),
                })
                .unwrap()
                .resolve(&request)
                .await
                .map_err(Into::into)
        })
        .into();
    let router = router
        .public_page::<Props>("/raster-guard", "Public")
        .unwrap();
    Storage::register_memory("no-media-pages");
    Storage::register_memory("no-media-assets");
    let source = Storage::disk("no-media-assets").unwrap();
    source
        .write("images/photo.png", b"unprepared raster".as_slice())
        .await
        .unwrap();
    let runtime = StaticPublication::install(
        &router,
        Publisher::new("no-media-pages", "v1", PublishConfig::default()).unwrap(),
    )
    .unwrap();
    runtime.assets(source).unwrap();
    let result = runtime.render("/raster-guard").await;
    assert!(result.is_err(), "raster publication requires media");
    let cause = result.unwrap_err().to_string();
    assert!(
        cause.contains("/raster-guard") && cause.contains("media"),
        "{cause}"
    );
}
