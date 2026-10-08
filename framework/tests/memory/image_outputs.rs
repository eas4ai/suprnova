//! MEM-003 on image output: every image the framework wrote before
//! application-port-third-round, other than the ones that commitment
//! changes on purpose, is still the same bytes.
//!
//! `fixtures/image-outputs-c04577b96.txt` holds the SHA-256 of each output
//! the code at c04577b96 wrote for the cases below, made by running the
//! same case builder against that commit (with its `Rotate(degrees)`). The
//! sources carry no EXIF orientation, profile or other metadata, no
//! pipeline turns by an angle that exposes corners, and no output is a JPEG
//! or a GIF of transparent pixels, so no output here is one IMG-001,
//! IMG-002 or IMG-006 changes or a JPEG the built-in driver now writes as
//! JFIF YCbCr. The cases run the built-in driver only: what ImageMagick
//! wrote at that commit carried the time it was written.

#![cfg(feature = "media")]

use sha2::{Digest, Sha256};
use suprnova::media::{
    ImageDriver, ImagePipeline, OutputFormat, OxideAvImageDriver, Transformation,
};

use crate::support::exclusive;

/// A clockwise turn by `degrees`, onto the default background.
fn rotate(degrees: f32) -> Transformation {
    Transformation::Rotate {
        degrees,
        background: None,
    }
}

/// The side lengths of the generated sources: odd, so no resize, crop or
/// turn lands on a power of two by accident.
const GOLDEN_WIDTH: u32 = 37;
const GOLDEN_HEIGHT: u32 = 23;

/// Packed RGBA in which neighbouring pixels differ in every channel, with
/// alpha varying over the image when `alpha` is set.
fn golden_rgba(alpha: bool) -> Vec<u8> {
    let mut out = Vec::new();
    for y in 0..GOLDEN_HEIGHT {
        for x in 0..GOLDEN_WIDTH {
            out.extend_from_slice(&[
                (x * 255 / (GOLDEN_WIDTH - 1)) as u8,
                (y * 255 / (GOLDEN_HEIGHT - 1)) as u8,
                ((x * 13 + y * 29) % 256) as u8,
                if alpha {
                    ((x + y) * 11 % 256) as u8
                } else {
                    255
                },
            ]);
        }
    }
    out
}

fn golden_png(format: oxideav_png::PngPixelFormat, channels: usize, data: Vec<u8>) -> Vec<u8> {
    oxideav_png::encode_plane(
        GOLDEN_WIDTH,
        GOLDEN_HEIGHT,
        format,
        GOLDEN_WIDTH as usize * channels,
        &data,
        None,
        &oxideav_png::EncodeOptions::default(),
    )
    .expect("the golden PNG encodes")
}

/// Every source, by name, and whether it is opaque. None carries an EXIF
/// orientation, a profile or any other metadata, so no output of theirs is
/// one IMG-001 or IMG-002 changes.
fn golden_sources() -> Vec<(&'static str, Vec<u8>, bool)> {
    let opaque = golden_rgba(false);
    let rgb: Vec<u8> = opaque
        .chunks(4)
        .flat_map(|pixel| [pixel[0], pixel[1], pixel[2]])
        .collect();
    let grey: Vec<u8> = opaque.chunks(4).map(|pixel| pixel[2]).collect();
    let grey_alpha: Vec<u8> = golden_rgba(true)
        .chunks(4)
        .flat_map(|pixel| [pixel[0], pixel[3]])
        .collect();
    let bmp = oxideav_bmp::encode(
        &oxideav_bmp::BmpImage::new(
            GOLDEN_WIDTH,
            GOLDEN_HEIGHT,
            oxideav_bmp::PixelFormat::Rgba,
            vec![oxideav_bmp::Plane::new(
                GOLDEN_WIDTH as usize * 4,
                opaque.clone(),
            )],
        )
        .expect("the golden BMP image"),
        &oxideav_bmp::EncodeOptions::default(),
    )
    .expect("the golden BMP encodes");
    use oxideav_png::PngPixelFormat as P;
    vec![
        ("png-rgba", golden_png(P::Rgba, 4, golden_rgba(true)), false),
        ("png-rgba-opaque", golden_png(P::Rgba, 4, opaque), true),
        ("png-rgb", golden_png(P::Rgb24, 3, rgb), true),
        ("png-grey", golden_png(P::Gray8, 1, grey), true),
        ("png-grey-alpha", golden_png(P::Ya8, 2, grey_alpha), false),
        ("bmp", bmp, true),
        (
            "jpeg-base-420",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/media/fixtures/jpeg/photo-base-420-35x21.jpg"
            ))
            .to_vec(),
            true,
        ),
        (
            "jpeg-prog-444",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/media/fixtures/jpeg/photo-prog-444-35x21.jpg"
            ))
            .to_vec(),
            true,
        ),
        (
            "jpeg-arith-420",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/media/fixtures/jpeg/photo-arith-420-35x21.jpg"
            ))
            .to_vec(),
            true,
        ),
        (
            "jpeg-lossless-rgb",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/media/fixtures/jpeg/photo-lossless-rgb-35x21.jpg"
            ))
            .to_vec(),
            true,
        ),
        (
            "jpeg-base-grey",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/media/fixtures/jpeg/photo-base-grey-35x21.jpg"
            ))
            .to_vec(),
            true,
        ),
        (
            "webp-lossless",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/media/fixtures/webp-libwebp-color-cache-128x86.webp"
            ))
            .to_vec(),
            true,
        ),
        (
            "webp-lossy-alpha",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/media/fixtures/webp-libwebp-lossy-alpha-128x86.webp"
            ))
            .to_vec(),
            false,
        ),
        (
            "gif-static",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/media/fixtures/gif-pil-static.gif"
            ))
            .to_vec(),
            true,
        ),
        (
            "gif-animated",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/media/fixtures/gif-im-animated.gif"
            ))
            .to_vec(),
            // Its first frame covers part of the screen; the rest is
            // transparent.
            false,
        ),
        (
            "gif-transparent",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/media/fixtures/gif-pil-transparent.gif"
            ))
            .to_vec(),
            false,
        ),
    ]
}

/// Every pipeline, by name. A turn is by a right angle only: a turn by
/// another angle exposes corners, whose fill IMG-006 changes.
fn golden_steps() -> Vec<(&'static str, Vec<Transformation>)> {
    use Transformation as T;
    vec![
        ("none", vec![]),
        (
            "resize",
            vec![T::Resize {
                width: 17,
                height: 11,
            }],
        ),
        ("resize-width", vec![T::ResizeWidth(20)]),
        ("resize-height", vec![T::ResizeHeight(9)]),
        (
            "scale",
            vec![T::Scale {
                width: 30,
                height: 30,
            }],
        ),
        (
            "cover",
            vec![T::Cover {
                width: 12,
                height: 12,
            }],
        ),
        (
            "contain",
            vec![T::Contain {
                width: 20,
                height: 20,
            }],
        ),
        (
            "crop",
            vec![T::Crop {
                width: 10,
                height: 8,
                x: 3,
                y: 2,
            }],
        ),
        ("rotate-90", vec![rotate(90.0)]),
        ("rotate-180", vec![rotate(180.0)]),
        ("rotate-270", vec![rotate(270.0)]),
        ("flip-vertically", vec![T::FlipVertically]),
        ("flip-horizontally", vec![T::FlipHorizontally]),
        ("blur", vec![T::Blur(30)]),
        ("sharpen", vec![T::Sharpen(60)]),
        ("grayscale", vec![T::Grayscale]),
        (
            "chain",
            vec![
                T::Resize {
                    width: 20,
                    height: 14,
                },
                T::FlipHorizontally,
                rotate(90.0),
                T::Sharpen(40),
                T::Grayscale,
            ],
        ),
    ]
}

/// Every output MEM-003 holds to its bytes before the commitment, by
/// label: each pipeline to PNG, the two that shrink the image to every
/// other format the built-in driver writes but JPEG (its JPEGs are JFIF YCbCr now, which
/// MEM-003 excepts), GIF from opaque sources only (IMG-006 flattens
/// transparency in GIF now), and each source's size and average colour.
/// The two larger WebP sources take only the pipelines that shrink them
/// first: a WebP encode is slow in a debug build.
fn mem_003_outputs() -> Vec<(String, Vec<u8>)> {
    let driver = OxideAvImageDriver::new();
    let mut out = Vec::new();
    for (source, bytes, opaque) in golden_sources() {
        for (name, steps) in golden_steps() {
            if source.starts_with("webp") && !matches!(name, "resize" | "cover" | "chain") {
                continue;
            }
            let mut targets = vec![("png", OutputFormat::Png)];
            if matches!(name, "resize" | "chain") {
                targets.push(("webp", OutputFormat::WebP));
                targets.push(("webp-lossless", OutputFormat::WebPLossless));
                targets.push(("bmp", OutputFormat::Bmp));
                if opaque {
                    targets.push(("gif", OutputFormat::Gif));
                }
            }
            for (target, format) in targets {
                let pipeline = ImagePipeline {
                    transformations: steps.clone(),
                    format: Some(format),
                    ..ImagePipeline::default()
                };
                let label = format!("{source}/{name}/{target}");
                let output = driver
                    .process(&bytes, &pipeline)
                    .unwrap_or_else(|e| format!("error: {e}").into_bytes());
                out.push((label, output));
            }
        }
        let (width, height) = driver.dimensions(&bytes).expect("the source decodes");
        out.push((
            format!("{source}/dimensions"),
            format!("{width}x{height}").into_bytes(),
        ));
        out.push((
            format!("{source}/dominant-color"),
            driver
                .dominant_color(&bytes)
                .expect("the source decodes")
                .into_bytes(),
        ));
    }
    out
}

/// Whether `bytes` is a lossless WebP: a RIFF/WEBP file whose image is a
/// `VP8L` bitstream and carries no lossy `VP8 ` one.
fn is_lossless_webp(bytes: &[u8]) -> bool {
    if !bytes.starts_with(b"RIFF") || bytes.get(8..12) != Some(&b"WEBP"[..]) {
        return false;
    }
    let (mut lossless, mut lossy) = (false, false);
    let mut at = 12;
    while let Some(head) = bytes.get(at..at + 8) {
        lossless |= &head[..4] == b"VP8L";
        lossy |= &head[..4] == b"VP8 ";
        let size = u32::from_le_bytes(head[4..].try_into().expect("four bytes")) as usize;
        at += 8 + size + (size & 1);
    }
    lossless && !lossy
}

/// MEM-003: the built-in driver's output, size and average colour for
/// every case are byte for byte what the code before the commitment
/// produced.
///
/// Lossless WebP is MEM-003's exception: the eas4ai/oxideav-webp encoder
/// chooses its transforms in one pass, and writes the simple lossless
/// layout where oxideav-webp 0.2.3 put a `VP8X` header in front of an
/// image with alpha. `fixtures/image-outputs-lossless-webp-eas4ai-65a9c8a.txt`
/// holds those outputs as the fork at 65a9c8a writes them, and only a
/// lossless WebP may take its digest from there.
#[tokio::test]
async fn mem_audit_image_output_is_byte_for_byte_what_it_was() {
    let _lock = exclusive().await;
    let digests = |file: &'static str| -> std::collections::BTreeMap<&'static str, &'static str> {
        file.lines()
            .filter_map(|line| line.split_once(' '))
            .collect()
    };
    let golden = digests(include_str!("fixtures/image-outputs-c04577b96.txt"));
    let lossless = digests(include_str!(
        "fixtures/image-outputs-lossless-webp-eas4ai-65a9c8a.txt"
    ));
    assert!(
        lossless.keys().all(|label| golden.contains_key(label)),
        "every lossless WebP digest replaces one from c04577b96"
    );
    let mut differ = Vec::new();
    let mut seen = 0;
    for (label, bytes) in mem_003_outputs() {
        let digest = hex::encode(Sha256::digest(&bytes));
        let expected = is_lossless_webp(&bytes)
            .then(|| lossless.get(label.as_str()))
            .flatten()
            .or_else(|| golden.get(label.as_str()));
        match expected {
            Some(&before) if before == digest => {}
            Some(_) => differ.push(label),
            None => differ.push(format!("{label} (no digest from c04577b96)")),
        }
        seen += 1;
    }
    assert_eq!(seen, golden.len(), "every recorded case runs");
    assert!(
        differ.is_empty(),
        "{} of {seen} outputs differ from the code before the commitment: {differ:#?}",
        differ.len()
    );
}
