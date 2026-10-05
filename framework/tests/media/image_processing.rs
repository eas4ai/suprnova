#![cfg(feature = "media")]
//! The Image subsystem: lazy pipeline, OxideAV driver, decode limits, and the
//! processed-output metadata contract.
//!
//! Every test here is `#[serial]`, including the ones that never touch the
//! config. The limit tests install a process-global `ImageConfig` override,
//! and `#[serial]` only orders a test against other `#[serial]` tests - so a
//! non-serial sibling would still run concurrently and decode under the
//! tightened cap. Serialising the whole file is what keeps that from being an
//! order-dependent flake.

use suprnova::{Image, ImageConfig, OutputFormat};

/// A 1x1 red PNG, byte-literal fixture (verified: `file` reports
/// `PNG image data, 1 x 1, 8-bit/color RGB, non-interlaced`, and the
/// subsystem decodes it to `(1, 1, [255, 0, 0, 255])`).
pub(crate) const RED_PNG_1X1: &[u8] = &[
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

/// Installs an `ImageConfig` override and clears it unconditionally on drop.
///
/// The override is process-global. Clearing it on the happy path only means a
/// single failed assertion leaks a tightened cap into every test that runs
/// after it, turning one red test into a cascade that hides its own cause.
pub(crate) struct ConfigGuard;

impl ConfigGuard {
    pub(crate) fn set(config: ImageConfig) -> Self {
        suprnova::media::set_config_for_tests(Some(config));
        Self
    }
}

impl Drop for ConfigGuard {
    fn drop(&mut self) {
        suprnova::media::set_config_for_tests(None);
    }
}

/// Photo-sized, with both sides odd, so the lossy WebP path has to extend
/// the image to the even size its 4:2:0 conversion needs.
const PHOTO_WIDTH: u32 = 97;
const PHOTO_HEIGHT: u32 = 65;

/// Advance a xorshift32 generator and return noise in `-10..=10`.
///
/// A fixed seed rather than a random source, so every run encodes the same
/// bytes and a size comparison cannot flake.
fn noise(state: &mut u32) -> i32 {
    *state ^= *state << 13;
    *state ^= *state >> 17;
    *state ^= *state << 5;
    (*state % 21) as i32 - 10
}

/// A photo-like, fully opaque image as packed RGBA: colour gradients under
/// per-pixel noise.
///
/// Flat colour flatters every encoder. Gradients with noise are the content
/// on which lossy and lossless WebP differ the way they do on a photograph.
fn photo_rgba(width: u32, height: u32) -> Vec<u8> {
    let mut state = 0x2545_F491_u32;
    let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height {
        for x in 0..width {
            let red = x * 255 / width.saturating_sub(1).max(1);
            let green = y * 255 / height.saturating_sub(1).max(1);
            let blue = 255 - (x + y) * 255 / (width + height).saturating_sub(2).max(1);
            for channel in [red, green, blue] {
                let value = channel as i32 + noise(&mut state);
                rgba.push(value.clamp(0, 255) as u8);
            }
            rgba.push(u8::MAX);
        }
    }
    rgba
}

/// Wrap packed RGBA in a 32-bit, bottom-up, uncompressed BMP.
///
/// BMP is the fixture format because it stores pixels as they are: the test
/// sets exact values, alpha included, with no encoder in between.
fn rgba_bmp(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    // BITMAPFILEHEADER (14 bytes) plus BITMAPINFOHEADER (40 bytes).
    const PIXEL_OFFSET: u32 = 14 + 40;
    let image_size = width * height * 4;
    let mut bmp = Vec::with_capacity((PIXEL_OFFSET + image_size) as usize);
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&(PIXEL_OFFSET + image_size).to_le_bytes());
    bmp.extend_from_slice(&[0; 4]); // reserved
    bmp.extend_from_slice(&PIXEL_OFFSET.to_le_bytes());
    bmp.extend_from_slice(&40u32.to_le_bytes()); // info header size
    bmp.extend_from_slice(&(width as i32).to_le_bytes());
    bmp.extend_from_slice(&(height as i32).to_le_bytes()); // positive: bottom-up
    bmp.extend_from_slice(&1u16.to_le_bytes()); // colour planes
    bmp.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
    bmp.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB, uncompressed
    bmp.extend_from_slice(&image_size.to_le_bytes());
    bmp.extend_from_slice(&2835i32.to_le_bytes()); // 72 dpi across
    bmp.extend_from_slice(&2835i32.to_le_bytes()); // 72 dpi down
    bmp.extend_from_slice(&0u32.to_le_bytes()); // palette size
    bmp.extend_from_slice(&0u32.to_le_bytes()); // important colours
    for row in rgba.chunks_exact(width as usize * 4).rev() {
        for pixel in row.as_chunks::<4>().0 {
            bmp.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
        }
    }
    bmp
}

/// Read a 32-bit uncompressed BMP back as `(width, height, packed RGBA)`.
///
/// The subsystem has no pixel accessor, so a test sees decoded pixels by
/// converting an image to BMP through the pipeline and reading this.
fn bmp_rgba(bmp: &[u8]) -> (u32, u32, Vec<u8>) {
    let le_u32 = |at: usize| u32::from_le_bytes(bmp[at..at + 4].try_into().expect("4 bytes"));
    let le_u16 = |at: usize| u16::from_le_bytes(bmp[at..at + 2].try_into().expect("2 bytes"));
    assert!(bmp.starts_with(b"BM"), "expected a BMP file");
    assert_eq!(le_u16(28), 32, "expected 32 bits per pixel");
    assert_eq!(le_u32(30), 0, "expected uncompressed BI_RGB");
    let offset = le_u32(10) as usize;
    let width = (le_u32(18) as i32).unsigned_abs();
    let stored_height = le_u32(22) as i32;
    let height = stored_height.unsigned_abs();
    let row_bytes = width as usize * 4;
    let mut rgba = Vec::with_capacity(row_bytes * height as usize);
    for y in 0..height as usize {
        // A positive stored height means the rows run bottom-up.
        let stored_row = if stored_height > 0 {
            height as usize - 1 - y
        } else {
            y
        };
        let start = offset + stored_row * row_bytes;
        for pixel in bmp[start..start + row_bytes].as_chunks::<4>().0 {
            rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
        }
    }
    (width, height, rgba)
}

/// The FourCC of every top-level chunk of a RIFF/WEBP file, in order.
///
/// The chunk list is what tells the encodings apart: a simple lossy file is
/// one `VP8 ` chunk (the space is part of the FourCC), a simple lossless
/// file one `VP8L` chunk, and a lossless file with alpha a `VP8X` header
/// chunk followed by `VP8L`. The ImageMagick driver tests use it too.
pub(crate) fn webp_chunks(webp: &[u8]) -> Vec<String> {
    assert!(
        webp.starts_with(b"RIFF") && webp.get(8..12) == Some(&b"WEBP"[..]),
        "expected a RIFF/WEBP file, got {:02x?}",
        &webp[..webp.len().min(12)]
    );
    let mut chunks = Vec::new();
    let mut at = 12;
    while at + 8 <= webp.len() {
        chunks.push(String::from_utf8_lossy(&webp[at..at + 4]).into_owned());
        let size = u32::from_le_bytes(webp[at + 4..at + 8].try_into().expect("4 bytes")) as usize;
        // Chunk payloads are padded to an even length.
        at += 8 + size + (size & 1);
    }
    chunks
}

/// The packed RGBA of a 32-bit BMP the driver wrote. The ImageMagick
/// driver tests use it too.
pub(crate) fn bmp_rgba_pixels(bmp: &[u8]) -> Vec<u8> {
    bmp_rgba(bmp).2
}

/// Run `source` through the pipeline to `format` at `quality`.
async fn encode(source: &[u8], format: OutputFormat, quality: u8) -> Vec<u8> {
    Image::from_bytes(source.to_vec())
        .to_format(format)
        .quality(quality)
        .to_bytes()
        .await
        .unwrap_or_else(|e| panic!("{format:?} at quality {quality} must encode: {e}"))
}

/// Decode any supported image to `(width, height, packed RGBA)`.
pub(crate) async fn decoded_rgba(image: &[u8]) -> (u32, u32, Vec<u8>) {
    let bmp = Image::from_bytes(image.to_vec())
        .to_format(OutputFormat::Bmp)
        .to_bytes()
        .await
        .expect("the image must decode and re-encode as BMP");
    bmp_rgba(&bmp)
}

/// The photo-like fixture as a BMP, ready for the pipeline. The ImageMagick
/// driver tests use it too.
pub(crate) fn photo_bmp() -> Vec<u8> {
    rgba_bmp(
        PHOTO_WIDTH,
        PHOTO_HEIGHT,
        &photo_rgba(PHOTO_WIDTH, PHOTO_HEIGHT),
    )
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
        (OutputFormat::WebPLossless, "image/webp"),
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
    let _config = ConfigGuard::set(ImageConfig {
        max_dimension: 2,
        ..ImageConfig::default()
    });
    let err = Image::from_bytes(src)
        .to_bytes()
        .await
        .expect_err("limit hit");
    assert!(err.to_string().contains("limit"), "got: {err}");
}

#[tokio::test]
#[serial_test::serial]
async fn decode_limits_reject_oversized_allocation() {
    let src = red_png_4x2().await;
    let _config = ConfigGuard::set(ImageConfig {
        // 4 x 2 x 4 bytes = 32; cap one byte under it.
        max_alloc_bytes: 31,
        ..ImageConfig::default()
    });
    let err = Image::from_bytes(src)
        .to_bytes()
        .await
        .expect_err("limit hit");
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

#[cfg(feature = "testing")]
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
    let _config = ConfigGuard::set(ImageConfig {
        max_alloc_bytes: 8,
        ..ImageConfig::default()
    });
    let chunks: Vec<std::io::Result<bytes::Bytes>> = RED_PNG_1X1
        .chunks(16)
        .map(|chunk| Ok(bytes::Bytes::copy_from_slice(chunk)))
        .collect();
    let err = Image::from_stream(stream::iter(chunks))
        .await
        .expect_err("the stream exceeds the cap");
    assert!(err.to_string().contains("limit"), "got: {err}");
}

#[tokio::test]
#[serial_test::serial]
async fn webp_quality_sets_the_size_and_lossy_undercuts_lossless() {
    let photo = photo_bmp();
    let low = encode(&photo, OutputFormat::WebP, 50).await;
    let high = encode(&photo, OutputFormat::WebP, 90).await;
    let lossless = encode(&photo, OutputFormat::WebPLossless, 90).await;
    assert!(
        low.len() < high.len(),
        "quality 50 ({} bytes) must be smaller than quality 90 ({} bytes)",
        low.len(),
        high.len()
    );
    assert!(
        high.len() < lossless.len(),
        "lossy quality 90 ({} bytes) must be smaller than lossless ({} bytes)",
        high.len(),
        lossless.len()
    );
}

#[tokio::test]
#[serial_test::serial]
async fn opaque_webp_is_lossy_and_webp_lossless_is_not() {
    let photo = photo_bmp();
    assert_eq!(
        webp_chunks(&encode(&photo, OutputFormat::WebP, 70).await),
        ["VP8 "],
        "an opaque image must be written as one lossy VP8 chunk"
    );
    assert_eq!(
        webp_chunks(&encode(&photo, OutputFormat::WebPLossless, 70).await),
        ["VP8L"],
        "WebPLossless must be written as one lossless VP8L chunk"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn webp_with_one_transparent_pixel_stays_lossless_and_keeps_it() {
    let mut rgba = photo_rgba(PHOTO_WIDTH, PHOTO_HEIGHT);
    let (x, y) = (10, 7);
    let alpha = (y * PHOTO_WIDTH as usize + x) * 4 + 3;
    rgba[alpha] = 0;
    let source = rgba_bmp(PHOTO_WIDTH, PHOTO_HEIGHT, &rgba);

    let webp = encode(&source, OutputFormat::WebP, 50).await;
    assert_eq!(
        webp_chunks(&webp),
        ["VP8X", "VP8L"],
        "lossy VP8 has no alpha, so the image must be lossless: VP8L behind \
         the VP8X header that declares its alpha"
    );

    let (width, height, decoded) = decoded_rgba(&webp).await;
    assert_eq!((width, height), (PHOTO_WIDTH, PHOTO_HEIGHT));
    assert_eq!(
        decoded[alpha], 0,
        "the transparent pixel must stay transparent"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn lossy_webp_decodes_at_the_source_dimensions() {
    let webp = encode(&photo_bmp(), OutputFormat::WebP, 70).await;
    let dimensions = Image::from_bytes(webp)
        .dimensions()
        .await
        .expect("lossy WebP must decode");
    assert_eq!(dimensions, (PHOTO_WIDTH, PHOTO_HEIGHT));
}

#[tokio::test]
#[serial_test::serial]
async fn webp_lossless_decodes_to_the_source_pixels() {
    let rgba = photo_rgba(PHOTO_WIDTH, PHOTO_HEIGHT);
    let source = rgba_bmp(PHOTO_WIDTH, PHOTO_HEIGHT, &rgba);
    let webp = encode(&source, OutputFormat::WebPLossless, 70).await;

    let (width, height, decoded) = decoded_rgba(&webp).await;
    assert_eq!((width, height), (PHOTO_WIDTH, PHOTO_HEIGHT));
    assert!(decoded == rgba, "lossless WebP must reproduce every pixel");
}
