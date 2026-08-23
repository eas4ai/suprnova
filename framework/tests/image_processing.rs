#![cfg(feature = "images")]
//! The Image subsystem: lazy pipeline, OxideAV driver, decode limits, and the
//! processed-output metadata contract.
//!
//! Every test here is `#[serial]`, including the ones that never touch the
//! config. The limit tests install a process-global `ImageConfig` override,
//! and `#[serial]` only orders a test against other `#[serial]` tests - so a
//! non-serial sibling would still run concurrently and decode under the
//! tightened cap. Serialising the whole file is what keeps that from being an
//! order-dependent flake.

// `Image` is imported from the subsystem module, not the crate root:
// `suprnova::Image` is already the upload-validator marker used as
// `UploadedFile<(Image, MaxSize<N>)>`. Every other name in the subsystem
// is flat at the crate root.
use suprnova::OutputFormat;
use suprnova::image::{Image, ImageConfig};

/// A 1x1 red PNG, byte-literal fixture (verified: `file` reports
/// `PNG image data, 1 x 1, 8-bit/color RGB, non-interlaced`, and the
/// subsystem decodes it to `(1, 1, [255, 0, 0, 255])`).
const RED_PNG_1X1: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53,
    0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x78, 0xDA, 0x63, 0xF8, 0xCF, 0xC0, 0x00,
    0x00, 0x03, 0x01, 0x01, 0x00, 0xF7, 0x03, 0x41, 0x43, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E,
    0x44, 0xAE, 0x42, 0x60, 0x82,
];

/// Build a larger fixture through the subsystem itself: upscale the 1x1 red
/// PNG to 4x2. The subsystem is its own fixture factory once decode+encode
/// round-trips - and the first test proves that round-trip.
async fn red_png_4x2() -> Vec<u8> {
    Image::from_bytes(RED_PNG_1X1)
        .resize(4, 2)
        .to_format(OutputFormat::Png)
        .to_bytes()
        .await
        .expect("fixture build")
}

#[tokio::test]
#[serial_test::serial]
async fn png_roundtrip_resize_reports_processed_dimensions() {
    let img = Image::from_bytes(RED_PNG_1X1)
        .resize(4, 2)
        .to_format(OutputFormat::Png);
    assert_eq!(img.clone().dimensions().await.expect("dims"), (4, 2));
    assert_eq!(img.mime_type().await.expect("mime"), "image/png");
}

#[tokio::test]
#[serial_test::serial]
async fn convert_to_each_supported_format() {
    let src = red_png_4x2().await;
    for (format, mime) in [
        (OutputFormat::Jpeg, "image/jpeg"),
        (OutputFormat::WebP, "image/webp"),
        (OutputFormat::Gif, "image/gif"),
        (OutputFormat::Bmp, "image/bmp"),
    ] {
        let img = Image::from_bytes(src.clone()).to_format(format);
        assert_eq!(
            img.mime_type().await.expect("mime"),
            mime,
            "conversion to {mime} must produce that mime"
        );
    }
}

#[tokio::test]
#[serial_test::serial]
async fn scale_never_enlarges() {
    let src = red_png_4x2().await;
    let img = Image::from_bytes(src).scale(100, 100);
    assert_eq!(
        img.dimensions().await.expect("dims"),
        (4, 2),
        "scale is scale-DOWN, per Laravel"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn rotate_arbitrary_angle_grows_the_canvas() {
    let src = red_png_4x2().await;
    let (w, h) = Image::from_bytes(src)
        .rotate(45.0)
        .dimensions()
        .await
        .expect("dims");
    assert!(
        w > 4 && h > 2,
        "45-degree rotation must grow the canvas, got {w}x{h}"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn dominant_color_of_a_red_image_is_red() {
    let src = red_png_4x2().await;
    let color = Image::from_bytes(src)
        .dominant_color()
        .await
        .expect("color");
    assert_eq!(color, "#ff0000");
}

#[tokio::test]
#[serial_test::serial]
async fn garbage_input_is_a_param_error_not_a_panic() {
    let err = Image::from_bytes(vec![0u8; 64])
        .to_bytes()
        .await
        .expect_err("not an image");
    assert!(
        err.to_string().contains("image"),
        "error names the boundary: {err}"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn zero_byte_input_is_rejected() {
    let err = Image::from_bytes(Vec::new())
        .to_bytes()
        .await
        .expect_err("empty input");
    assert!(
        err.to_string().contains("image"),
        "error names the boundary: {err}"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn decode_limits_reject_oversized_dimensions() {
    // Build the fixture under the default limits, then tighten the cap
    // below its width so the decode is refused before any allocation.
    let src = red_png_4x2().await;
    suprnova::image::set_config_for_tests(Some(ImageConfig {
        max_dimension: 2,
        ..ImageConfig::default()
    }));
    let err = Image::from_bytes(src)
        .to_bytes()
        .await
        .expect_err("limit hit");
    suprnova::image::set_config_for_tests(None);
    assert!(err.to_string().contains("limit"), "got: {err}");
}

#[tokio::test]
#[serial_test::serial]
async fn decode_limits_reject_oversized_allocation() {
    let src = red_png_4x2().await;
    suprnova::image::set_config_for_tests(Some(ImageConfig {
        // 4 x 2 x 4 bytes = 32; cap one byte under it.
        max_alloc_bytes: 31,
        ..ImageConfig::default()
    }));
    let err = Image::from_bytes(src)
        .to_bytes()
        .await
        .expect_err("limit hit");
    suprnova::image::set_config_for_tests(None);
    assert!(err.to_string().contains("limit"), "got: {err}");
}

#[tokio::test]
#[serial_test::serial]
async fn to_response_carries_the_processed_mime() {
    let src = red_png_4x2().await;
    let resp = Image::from_bytes(src)
        .to_format(OutputFormat::WebP)
        .to_response()
        .await
        .expect("response");
    assert_eq!(resp.status_code(), 200);
    assert_eq!(resp.header_value("Content-Type"), Some("image/webp"));
    assert!(!resp.body().is_empty());
}

#[tokio::test]
#[serial_test::serial]
async fn storage_roundtrip() {
    let _guard = suprnova::Storage::fake();
    suprnova::Storage::register_memory("images");
    let disk = suprnova::Storage::disk("images").expect("disk");
    use suprnova::DiskExt;
    disk.put("in.png", red_png_4x2().await).await.expect("seed");

    Image::from_disk("images", "in.png")
        .resize(2, 1)
        .store("images", "out.png")
        .await
        .expect("store");
    let out = disk.get("out.png").await.expect("read back");
    let (w, h) = Image::from_bytes(out).dimensions().await.expect("decode");
    assert_eq!((w, h), (2, 1));
}

#[tokio::test]
#[serial_test::serial]
async fn from_upload_reads_the_bytes_eagerly() {
    // The canonical use: an avatar arrives as an upload and is resized on
    // the way to storage. `from_upload` has to be eager, because the upload's
    // backing temp file does not outlive the request.
    let upload: suprnova::UploadedFile = suprnova::UploadedFile::from_memory(
        bytes::Bytes::from_static(RED_PNG_1X1),
        Some("avatar.png".to_string()),
        Some("image/png".to_string()),
        Some("png"),
    );

    let image = Image::from_upload(&upload).await.expect("read the upload");
    let (w, h) = image.resize(8, 4).dimensions().await.expect("dims");
    assert_eq!((w, h), (8, 4));
}

#[tokio::test]
#[serial_test::serial]
async fn from_stream_is_capped_while_it_collects() {
    use futures_util::stream;

    let chunks: Vec<std::io::Result<bytes::Bytes>> = RED_PNG_1X1
        .chunks(16)
        .map(|chunk| Ok(bytes::Bytes::copy_from_slice(chunk)))
        .collect();
    let image = Image::from_stream(stream::iter(chunks))
        .await
        .expect("collect the stream");
    assert_eq!(image.dimensions().await.expect("dims"), (1, 1));

    // With the cap below the payload, collection stops rather than
    // discovering the problem after memory is already spent.
    suprnova::image::set_config_for_tests(Some(ImageConfig {
        max_alloc_bytes: 8,
        ..ImageConfig::default()
    }));
    let chunks: Vec<std::io::Result<bytes::Bytes>> = RED_PNG_1X1
        .chunks(16)
        .map(|chunk| Ok(bytes::Bytes::copy_from_slice(chunk)))
        .collect();
    let err = Image::from_stream(stream::iter(chunks))
        .await
        .expect_err("the stream exceeds the cap");
    suprnova::image::set_config_for_tests(None);
    assert!(err.to_string().contains("limit"), "got: {err}");
}
