#![cfg(feature = "media")]
//! Live-binary integration tests for the `magick` image driver.
//!
//! These shell out to a real ImageMagick 7 on the host, so they are
//! `#[ignore]`d and the unattended gate never runs them. Run with:
//!
//! ```sh
//! cargo test -p suprnova --features images --test image_magick_driver -- --ignored
//! ```
//!
//! The argument-construction contract is covered by pure unit tests inside
//! `framework/src/image/magick.rs`; what only a real binary can prove is that
//! those arguments mean what the mapping claims they mean. Tests that need a
//! format-specific delegate (HEIC) skip-detect rather than fail, because a
//! delegate is a host build option, not a Suprnova defect.

use std::process::Command;

use suprnova::{Image, ImageDriver, ImagePipeline, MagickCliDriver, OutputFormat, Transformation};

/// 1x1 red PNG, the same verified fixture the pure-Rust tests use.
const RED_PNG_1X1: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53,
    0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x78, 0xDA, 0x63, 0xF8, 0xCF, 0xC0, 0x00,
    0x00, 0x03, 0x01, 0x01, 0x00, 0xF7, 0x03, 0x41, 0x43, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E,
    0x44, 0xAE, 0x42, 0x60, 0x82,
];

fn driver() -> MagickCliDriver {
    MagickCliDriver::from_env()
}

fn pipeline(steps: Vec<Transformation>, format: OutputFormat) -> ImagePipeline {
    ImagePipeline {
        transformations: steps,
        format: Some(format),
        ..ImagePipeline::default()
    }
}

/// True when the host's ImageMagick lists a read delegate for `format`.
fn supports_format(format: &str) -> bool {
    let Ok(output) = Command::new(MagickCliDriver::from_env().binary())
        .args(["-list", "format"])
        .output()
    else {
        return false;
    };
    String::from_utf8_lossy(&output.stdout).lines().any(|line| {
        let mut fields = line.split_whitespace();
        let name = fields.next().unwrap_or_default().trim_end_matches('*');
        // Columns are: NAME MODULE MODE DESCRIPTION; mode "rw+"/"r--" etc.
        name.eq_ignore_ascii_case(format) && fields.nth(1).is_some_and(|mode| mode.starts_with('r'))
    })
}

#[test]
#[ignore = "requires a host ImageMagick 7 binary"]
fn resize_round_trips_through_the_real_binary() {
    let out = driver()
        .process(
            RED_PNG_1X1,
            &pipeline(
                vec![Transformation::Resize {
                    width: 12,
                    height: 6,
                }],
                OutputFormat::Png,
            ),
        )
        .expect("magick must resize the fixture");
    assert!(out.starts_with(b"\x89PNG"), "expected a PNG back");

    let (width, height) = driver().dimensions(&out).expect("dimensions");
    assert_eq!(
        (width, height),
        (12, 6),
        "the `!` geometry suffix must force exact dimensions"
    );
}

#[test]
#[ignore = "requires a host ImageMagick 7 binary"]
fn scale_never_enlarges_through_the_real_binary() {
    // The whole point of the `>` suffix: asking for a bigger box is a no-op.
    let enlarged = driver()
        .process(
            RED_PNG_1X1,
            &pipeline(
                vec![Transformation::Scale {
                    width: 100,
                    height: 100,
                }],
                OutputFormat::Png,
            ),
        )
        .expect("magick must handle the scale");
    assert_eq!(
        driver().dimensions(&enlarged).expect("dimensions"),
        (1, 1),
        "scale must never enlarge"
    );
}

#[test]
#[ignore = "requires a host ImageMagick 7 binary"]
fn cover_fills_the_box_exactly() {
    let source = driver()
        .process(
            RED_PNG_1X1,
            &pipeline(
                vec![Transformation::Resize {
                    width: 40,
                    height: 10,
                }],
                OutputFormat::Png,
            ),
        )
        .expect("build a wide source");

    let covered = driver()
        .process(
            &source,
            &pipeline(
                vec![Transformation::Cover {
                    width: 20,
                    height: 20,
                }],
                OutputFormat::Png,
            ),
        )
        .expect("cover");
    assert_eq!(
        driver().dimensions(&covered).expect("dimensions"),
        (20, 20),
        "the ^ resize plus -extent must land exactly on the target box"
    );
}

#[test]
#[ignore = "requires a host ImageMagick 7 binary"]
fn every_output_format_encodes_through_the_real_binary() {
    for (format, magic) in [
        (OutputFormat::Png, &b"\x89PNG"[..]),
        (OutputFormat::Jpeg, &[0xFF, 0xD8, 0xFF][..]),
        (OutputFormat::Gif, &b"GIF"[..]),
        (OutputFormat::Bmp, &b"BM"[..]),
        (OutputFormat::WebP, &b"RIFF"[..]),
    ] {
        if !supports_format(format.extension()) {
            eprintln!("skipping {format:?}: no host delegate");
            continue;
        }
        let out = driver()
            .process(RED_PNG_1X1, &pipeline(Vec::new(), format))
            .unwrap_or_else(|e| panic!("{format:?} must encode: {e}"));
        assert!(
            out.starts_with(magic),
            "{format:?} produced the wrong magic bytes: {:02x?}",
            &out[..out.len().min(8)]
        );
    }
}

#[test]
#[ignore = "requires a host ImageMagick 7 binary"]
fn dominant_color_reads_back_the_source_colour() {
    let color = driver()
        .dominant_color(RED_PNG_1X1)
        .expect("dominant colour");
    assert_eq!(color, "#ff0000");
}

#[test]
#[ignore = "requires a host ImageMagick 7 binary"]
fn a_missing_binary_fails_with_an_actionable_message() {
    let absent = MagickCliDriver::new("suprnova-definitely-not-installed");
    let err = absent
        .process(RED_PNG_1X1, &pipeline(Vec::new(), OutputFormat::Png))
        .expect_err("the binary does not exist");
    let message = err.to_string();
    assert!(message.contains("IMAGE_MAGICK_BINARY"), "got: {message}");
}

#[test]
#[ignore = "requires a host ImageMagick 7 binary with the libheif delegate"]
fn heic_decodes_when_the_host_carries_the_delegate() {
    if !supports_format("heic") {
        eprintln!(
            "skipping: this host's ImageMagick has no HEIC read delegate. \
             That is a host build option, not a Suprnova defect."
        );
        return;
    }

    // Build the HEIC fixture with the same binary under test, so the test
    // carries no binary blob for a format the framework cannot itself write.
    let heic = Command::new(MagickCliDriver::from_env().binary())
        .args(["-size", "8x4", "xc:red", "heic:-"])
        .output()
        .expect("magick must run");
    assert!(
        heic.status.success() && !heic.stdout.is_empty(),
        "could not build a HEIC fixture: {}",
        String::from_utf8_lossy(&heic.stderr)
    );

    // This is the case the whole driver exists for: the pure-Rust driver
    // refuses HEIC by design, and IMAGE_DRIVER=magick reads it.
    let out = driver()
        .process(
            &heic.stdout,
            &pipeline(
                vec![Transformation::Resize {
                    width: 4,
                    height: 2,
                }],
                OutputFormat::Png,
            ),
        )
        .expect("magick must ingest HEIC when the delegate is present");
    assert!(out.starts_with(b"\x89PNG"));

    // And the result is readable by the pure-Rust side: HEIC in, a format
    // Suprnova fully supports out.
    let (width, height) = suprnova::OxideAvImageDriver::new()
        .dimensions(&out)
        .expect("the converted PNG must decode in the pure-Rust driver");
    assert_eq!((width, height), (4, 2));
}

#[tokio::test]
#[ignore = "requires a host ImageMagick 7 binary"]
async fn the_image_facade_drives_the_magick_driver() {
    // Installed explicitly rather than through IMAGE_DRIVER, so the test does
    // not depend on process-global env ordering.
    let _ = suprnova::media::set_default_driver(Box::new(MagickCliDriver::from_env()));

    let bytes = Image::from_bytes(RED_PNG_1X1)
        .resize(9, 3)
        .to_format(OutputFormat::Png)
        .to_bytes()
        .await
        .expect("pipeline");
    assert!(bytes.starts_with(b"\x89PNG"));
}
