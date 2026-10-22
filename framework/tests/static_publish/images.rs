#![cfg(feature = "media")]
//! PUB-007 image preparation against a memory-backed publication source.

use sha2::{Digest, Sha256};
use suprnova::media::{Image, OutputFormat, PngEffort};
use suprnova::static_publish::images::{ImageOptions, prepare};

#[suprnova::inertia_root(path = "inertia/publication-images.html")]
struct ImageDocument;

#[derive(suprnova::InertiaProps)]
#[inertia(cacheable)]
struct ImageProps {
    #[inertia(public)]
    image: String,
}

#[tokio::test]
#[serial_test::serial]
async fn pub_007_runtime_publishes_transcodes_and_hydration_uses_stripped_fallback() {
    let _container = suprnova::testing::TestContainer::fake();
    let source = source().await;
    let router: suprnova::Router = suprnova::Router::new()
        .get("/images-page", |request: suprnova::Request| async move {
            suprnova::InertiaResponse::new("Images")
                .with_config(
                    suprnova::InertiaConfig::new()
                        .development(true)
                        .version("images-v1")
                        .ssr_disabled()
                        .vite_dev_server("http://localhost:5173")
                        .root_template(
                            suprnova::inertia::InertiaRootTemplate::of::<ImageDocument>(),
                        ),
                )
                .try_with("image", "/images/photo.png")
                .map_err(suprnova::HttpResponse::from)?
                .resolve(&request)
                .await
                .map_err(suprnova::HttpResponse::from)
        })
        .into();
    let router = router
        .public_page::<ImageProps>("/images-page", "Images")
        .expect("public image page");
    let disk = opendal::Operator::new(opendal::services::Memory::default()).expect("target disk");
    let publisher = suprnova::static_publish::Publisher::from_operator(
        disk.clone(),
        "images-v1",
        suprnova::static_publish::PublishConfig::default(),
    )
    .expect("publisher");
    let runtime =
        suprnova::static_publish::StaticPublication::install(&router, publisher).expect("runtime");
    runtime.assets(source).expect("public-root source");
    runtime.publish().await.expect("image publication");
    let manifest: suprnova::static_publish::Manifest = serde_json::from_slice(
        &disk
            .read("_pub/manifest.json")
            .await
            .expect("manifest")
            .to_bytes(),
    )
    .expect("manifest JSON");
    let objects = &manifest.versions[&manifest.live].objects;
    let fallback = objects
        .iter()
        .find(|object| object.path.ends_with(".png"))
        .expect("stripped original object");
    let webp = objects
        .iter()
        .find(|object| object.path.ends_with(".webp"))
        .expect("transcoded WebP object");
    let page = objects
        .iter()
        .find(|object| object.path.ends_with(".html"))
        .expect("HTML object");
    let html = disk
        .read(&page.path)
        .await
        .expect("published HTML")
        .to_bytes();
    let html = String::from_utf8_lossy(&html);
    assert!(html.contains(&format!("/{}", webp.path)));
    assert!(html.contains(&format!("/{}", fallback.path)));
    assert!(
        !html.contains("/images/photo.png"),
        "hydration data and DOM must both use published metadata-stripped fallback"
    );
    let json = objects
        .iter()
        .find(|object| object.path.ends_with(".json"))
        .expect("Inertia JSON object");
    let json: serde_json::Value = serde_json::from_slice(
        &disk
            .read(&json.path)
            .await
            .expect("published JSON")
            .to_bytes(),
    )
    .expect("page JSON");
    assert_eq!(json["props"]["image"], format!("/{}", fallback.path));
    for image in [fallback, webp] {
        assert_eq!(
            image.headers["cache-control"],
            "public, max-age=31536000, immutable"
        );
        let bytes = disk
            .read(&image.path)
            .await
            .expect("published image")
            .to_bytes();
        assert!(
            !bytes
                .windows(b"private location".len())
                .any(|part| part == b"private location")
        );
    }
}

fn png() -> Vec<u8> {
    let mut png = oxideav_png::encode_plane(
        8,
        8,
        oxideav_png::PngPixelFormat::Rgba,
        32,
        &[180, 60, 20, 255].repeat(64),
        None,
        &oxideav_png::EncodeOptions::default(),
    )
    .expect("PNG fixture");
    let mut chunk = Vec::new();
    oxideav_png::chunk::write_chunk(
        &mut chunk,
        b"tEXt",
        b"Comment\0private location and caption",
    );
    png.splice(33..33, chunk);
    png
}

async fn source() -> opendal::Operator {
    let source =
        opendal::Operator::new(opendal::services::Memory::default()).expect("memory operator");
    source
        .write("images/photo.png", png())
        .await
        .expect("source image");
    source
}

#[tokio::test]
async fn pub_007_transcodes_real_images_rewrites_picture_and_preserves_stripped_fallback() {
    let source = source().await;
    let html = br#"<!doctype html><script>const sample = '<img src="/missing.png">';</script><!-- <img src="/missing.png"> --><IMG alt="a &amp; b" src='/images/photo.png'><picture><source media="(min-width: 600px)" srcset="/images/photo.png 2x"><img srcset="/images/photo.png 1x, /images/photo.png 2x" src="/images/photo.png"></picture>"#;
    let prepared = prepare(html, &source, &ImageOptions::default())
        .await
        .expect("prepare images");
    let html = String::from_utf8(prepared.html.to_vec()).expect("HTML");
    assert!(html.contains("const sample = '<img src=\"/missing.png\">';"));
    assert!(html.contains("<!-- <img src=\"/missing.png\"> -->"));
    assert!(html.contains("type=\"image/webp\""));
    assert!(html.contains("alt=\"a &amp; b\""));
    assert!(html.contains(" 1x, /static/images/"));
    assert_eq!(
        html.matches("<picture>").count(),
        2,
        "existing picture is not nested"
    );
    assert_eq!(prepared.assets.len(), 2, "deduplicates repeated references");
    for asset in &prepared.assets {
        assert!(html.contains(&format!("/{}", asset.path)));
        assert!(
            asset
                .path
                .contains(&hex::encode(Sha256::digest(&asset.bytes)))
        );
        assert!(
            !asset
                .bytes
                .windows(b"private location".len())
                .any(|part| part == b"private location")
        );
    }
    let fallback = prepared
        .assets
        .iter()
        .find(|asset| asset.content_type == "image/png")
        .expect("original PNG fallback");
    assert_eq!(
        prepared.replacements["/images/photo.png"],
        format!("/{}", fallback.path)
    );
    let mut pos = 8;
    loop {
        let (chunk, next) =
            oxideav_png::chunk::read_chunk(&fallback.bytes, pos).expect("PNG chunk");
        assert!(!matches!(
            &chunk.chunk_type,
            b"tEXt" | b"zTXt" | b"iTXt" | b"eXIf" | b"tIME"
        ));
        if chunk.is_type(b"IEND") {
            break;
        }
        pos = next;
    }
    assert!(
        prepared
            .assets
            .iter()
            .any(|asset| asset.content_type == "image/webp")
    );
    assert!(
        !prepared
            .assets
            .iter()
            .any(|asset| asset.content_type == "image/avif")
    );
}

#[tokio::test]
async fn pub_007_quality_floor_and_configured_encode_are_enforced() {
    let source = source().await;
    let mut options = ImageOptions {
        quality: 84,
        ..ImageOptions::default()
    };
    let error = prepare(b"<img src='/images/photo.png'>", &source, &options)
        .await
        .expect_err("reject quality below 85");
    assert!(error.to_string().contains("85..=100"));
    options.quality = 92;
    let prepared = prepare(b"<img src='/images/photo.png'>", &source, &options)
        .await
        .expect("quality 92");
    let expected = Image::from_bytes(png())
        .to_format(OutputFormat::WebP)
        .quality(92)
        .to_bytes()
        .await
        .expect("media encoding at requested quality");
    assert_eq!(
        prepared
            .assets
            .iter()
            .find(|asset| asset.content_type == "image/webp")
            .expect("WebP")
            .bytes
            .as_ref(),
        expected
    );
}

#[tokio::test]
async fn pub_007_kept_png_uses_maximum_lossless_recompression() {
    let source = source().await;
    let prepared = prepare(
        b"<img data-static-keep-png src='/images/photo.png'>",
        &source,
        &ImageOptions::default(),
    )
    .await
    .expect("kept PNG");
    assert_eq!(prepared.assets.len(), 1);
    assert_eq!(prepared.assets[0].content_type, "image/png");
    let expected = Image::from_bytes(png())
        .to_png()
        .recompress_png(PngEffort::Maximum)
        .to_bytes()
        .await
        .expect("maximum oxipng with Zopfli");
    assert_eq!(prepared.assets[0].bytes.as_ref(), expected);
    assert!(!String::from_utf8_lossy(&prepared.html).contains("image/webp"));
    let mut options = ImageOptions::default();
    options.keep_png.insert("images/photo.png".into());
    let marked = prepare(b"<img src='/images/photo.png'>", &source, &options)
        .await
        .expect("application marked PNG");
    assert_eq!(marked.assets[0].bytes, prepared.assets[0].bytes);
}

#[tokio::test]
async fn pub_007_manifest_icons_keep_png_and_relative_references_resolve() {
    let source = source().await;
    source.write("site.webmanifest", br#"{"name":"site","icons":[{"src":"images/photo.png","sizes":"8x8","type":"image/png"}]}"#.to_vec()).await.expect("manifest source");
    let options = ImageOptions {
        document_path: "/nested/page".into(),
        ..ImageOptions::default()
    };
    let prepared = prepare(
        b"<link rel='manifest' href='../site.webmanifest'><img src='../images/photo.png'>",
        &source,
        &options,
    )
    .await
    .expect("manifest and relative image");
    let manifest = prepared
        .assets
        .iter()
        .find(|asset| asset.content_type == "application/manifest+json")
        .expect("rewritten manifest");
    let value: serde_json::Value = serde_json::from_slice(&manifest.bytes).expect("manifest JSON");
    let icon = value["icons"][0]["src"].as_str().expect("icon URL");
    assert!(icon.ends_with(".png"));
    let icon = prepared
        .assets
        .iter()
        .find(|asset| format!("/{}", asset.path) == icon)
        .expect("published manifest icon");
    let expected = Image::from_bytes(png())
        .to_png()
        .recompress_png(PngEffort::Maximum)
        .to_bytes()
        .await
        .expect("maximum icon compression");
    assert_eq!(icon.bytes.as_ref(), expected);
    assert!(
        prepared
            .assets
            .iter()
            .any(|asset| asset.content_type == "image/webp")
    );
}

#[tokio::test]
async fn pub_007_remote_images_remain_external_and_missing_local_images_fail() {
    let source = source().await;
    let html = b"<img src='https://example.org/photo.png'><img src='data:image/png;base64,AAAA'>";
    let prepared = prepare(html, &source, &ImageOptions::default())
        .await
        .expect("external images");
    assert_eq!(prepared.html.as_ref(), html);
    assert!(prepared.assets.is_empty());
    let error = prepare(
        b"<img src='/missing.png'>",
        &source,
        &ImageOptions::default(),
    )
    .await
    .expect_err("missing source refuses publication");
    assert!(error.to_string().contains("missing.png"));
    assert!(
        prepare(
            b"<img src='/%2e%2e%2fprivate.png'>",
            &source,
            &ImageOptions::default()
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn pub_007_avif_requires_magick_configuration() {
    let source = source().await;
    let options = ImageOptions {
        avif: true,
        ..ImageOptions::default()
    };
    if suprnova::media::default_driver().expect("driver").name() == "magick" {
        return;
    }
    let error = prepare(b"<img src='/images/photo.png'>", &source, &options)
        .await
        .expect_err("AVIF requires configured Magick");
    assert!(error.to_string().contains("IMAGE_DRIVER=magick"));
}

#[tokio::test]
async fn pub_007_avif_magick_publishes_enabled_alternative_and_strips_metadata() {
    const CHILD: &str = "SUPRNOVA_TEST_PUBLICATION_AVIF_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args([
                "pub_007_avif_magick_publishes_enabled_alternative_and_strips_metadata",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env("IMAGE_DRIVER", "magick")
            .output()
            .expect("isolated Magick publication test");
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let source = source().await;
    let options = ImageOptions {
        avif: true,
        ..ImageOptions::default()
    };
    let prepared = prepare(b"<img src='/images/photo.png'>", &source, &options)
        .await
        .expect("configured Magick AVIF encode");
    let html = String::from_utf8_lossy(&prepared.html);
    assert!(html.contains("type=\"image/avif\""));
    assert!(html.contains("type=\"image/webp\""));
    let avif = prepared
        .assets
        .iter()
        .find(|asset| asset.content_type == "image/avif")
        .expect("AVIF object");
    assert_eq!(
        infer::get(&avif.bytes).expect("AVIF magic").mime_type(),
        "image/avif"
    );
    assert!(
        !avif
            .bytes
            .windows(b"private location".len())
            .any(|part| part == b"private location")
    );
}

#[tokio::test]
async fn pub_007_fix_k_video_posters_and_image_preloads_are_prepared() {
    let source = source().await;
    let prepared=prepare(br#"<video poster="/images/photo.png"></video><link rel="preload" as="image" href="/images/photo.png">"#,&source,&ImageOptions::default()).await.unwrap();
    let html = String::from_utf8(prepared.html.to_vec()).unwrap();
    assert!(
        !html.contains("/images/photo.png"),
        "every raster reference is rewritten: {html}"
    );
    assert!(html.contains(".webp"));
    assert_eq!(
        prepared.assets.len(),
        2,
        "WebP and stripped fallback are both retained"
    );
    for asset in &prepared.assets {
        assert!(
            !asset
                .bytes
                .windows(b"private location".len())
                .any(|v| v == b"private location")
        );
    }
}
