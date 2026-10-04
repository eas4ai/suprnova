#![cfg(feature = "media")]
//! Live-binary integration tests for the `magick` image driver.
//!
//! These shell out to a real ImageMagick 7 on the host, so they are
//! `#[ignore]`d and the unattended gate never runs them. Run with:
//!
//! ```sh
//! cargo test -p suprnova --features media --test image_magick_driver -- --ignored
//! ```
//!
//! The argument-construction contract is covered by pure unit tests inside
//! `framework/src/media/magick.rs`; what only a real binary can prove is that
//! those arguments mean what the mapping claims they mean. Tests that need a
//! format-specific delegate (HEIC) skip-detect rather than fail, because a
//! delegate is a host build option, not a Suprnova defect.

use std::process::Command;

use suprnova::{Image, ImageDriver, ImagePipeline, MagickCliDriver, OutputFormat, Transformation};

use crate::image_processing::{photo_bmp, webp_chunks};

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
fn webp_quality_reaches_the_lossy_encoder_through_the_real_binary() {
    if !supports_format("webp") {
        eprintln!("skipping: this host's ImageMagick has no WebP delegate");
        return;
    }
    let source = photo_bmp();
    let encode = |quality: u8| {
        driver()
            .process(
                &source,
                &ImagePipeline {
                    format: Some(OutputFormat::WebP),
                    quality,
                    ..ImagePipeline::default()
                },
            )
            .unwrap_or_else(|e| panic!("magick must write WebP at quality {quality}: {e}"))
    };
    let (low, high) = (encode(50), encode(90));
    assert!(
        low.len() < high.len(),
        "quality 50 ({} bytes) must be smaller than quality 90 ({} bytes)",
        low.len(),
        high.len()
    );
    let chunks = webp_chunks(&high);
    assert!(
        chunks.iter().any(|chunk| chunk == "VP8 "),
        "an opaque image must carry a lossy VP8 bitstream, got {chunks:?}"
    );
}

#[test]
#[ignore = "requires a host ImageMagick 7 binary"]
fn webp_lossless_reaches_the_lossless_encoder_through_the_real_binary() {
    if !supports_format("webp") {
        eprintln!("skipping: this host's ImageMagick has no WebP delegate");
        return;
    }
    let out = driver()
        .process(
            &photo_bmp(),
            &pipeline(Vec::new(), OutputFormat::WebPLossless),
        )
        .expect("magick must write lossless WebP");
    let chunks = webp_chunks(&out);
    assert!(
        chunks.iter().any(|chunk| chunk == "VP8L"),
        "the lossless define must select a VP8L bitstream, got {chunks:?}"
    );
    assert!(
        !chunks.iter().any(|chunk| chunk == "VP8 "),
        "a lossless file must carry no lossy bitstream, got {chunks:?}"
    );
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
    suprnova::media::set_default_driver(Box::new(MagickCliDriver::from_env()))
        .expect("this binary must be the first to install a driver, or the test exercises OxideAV while claiming magick");

    let bytes = Image::from_bytes(RED_PNG_1X1)
        .resize(9, 3)
        .to_format(OutputFormat::Png)
        .to_bytes()
        .await
        .expect("pipeline");
    assert!(bytes.starts_with(b"\x89PNG"));
}

#[test]
#[ignore = "requires a host ImageMagick 7 binary"]
fn an_animation_reports_the_dimensions_of_its_first_frame() {
    // ImageMagick writes the fixture, because it does not read the GIFs
    // OxideAV's encoder writes, and this test is about the probe.
    let made = Command::new(driver().binary())
        .args([
            "-size", "100x100", "xc:red", "-size", "100x100", "xc:blue", "-loop", "0", "gif:-",
        ])
        .output()
        .expect("magick must run");
    assert!(made.status.success(), "magick must write the animation");
    let animation = made.stdout;

    // `-format` writes no separator between frames, so probing every frame
    // of a two-frame 100x100 GIF prints `100 100100 100`, which reads as a
    // height of 100100.
    assert_eq!(
        driver().dimensions(&animation).expect("dimensions"),
        (100, 100)
    );

    // The facade probes the processed output, and a GIF processed without a
    // conversion keeps every frame.
    let processed = driver()
        .process(&animation, &ImagePipeline::default())
        .expect("magick must re-encode the animation");
    assert_eq!(
        driver().dimensions(&processed).expect("dimensions"),
        (100, 100)
    );

    // A first frame smaller than its logical screen: the built-in driver
    // composes it onto the screen, so the answer is the screen, 100x80, not
    // the frame's own 40x30.
    let made = Command::new(driver().binary())
        .args([
            "-size",
            "40x30",
            "xc:red",
            "-set",
            "page",
            "100x80+10+10",
            "-size",
            "100x80",
            "xc:blue",
            "-set",
            "page",
            "100x80+0+0",
            "-loop",
            "0",
            "gif:-",
        ])
        .output()
        .expect("magick must run");
    assert!(made.status.success(), "magick must write the animation");
    assert_eq!(
        driver().dimensions(&made.stdout).expect("dimensions"),
        (100, 80)
    );
}

/// Run `magick identify -format <format> gif:-` over `gif` and return what it
/// printed, one entry per frame.
fn identify_frames(gif: &[u8], format: &str) -> Vec<String> {
    use std::io::Write;
    let mut child = Command::new(driver().binary())
        .args(["identify", "-format", format, "gif:-"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("magick must run");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(gif)
        .expect("write the GIF");
    let out = child.wait_with_output().expect("identify output");
    assert!(out.status.success(), "identify must read the GIF");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::to_string)
        .collect()
}

#[test]
#[ignore = "requires a host ImageMagick 7 binary"]
fn an_animation_is_processed_as_its_first_frame() {
    // The built-in driver decodes the first frame composed onto the logical
    // screen and nothing after it; the manual says the pipeline only uses
    // the first frame. So this driver must too, or a resize scales each
    // frame on its own and the output keeps every frame.
    let made = Command::new(driver().binary())
        .args([
            "-size",
            "40x30",
            "xc:red",
            "-set",
            "page",
            "100x80+10+10",
            "-size",
            "100x80",
            "xc:blue",
            "-set",
            "page",
            "100x80+0+0",
            "-loop",
            "0",
            "gif:-",
        ])
        .output()
        .expect("magick must run");
    assert!(made.status.success(), "magick must write the animation");

    let processed = driver()
        .process(
            &made.stdout,
            &pipeline(
                vec![Transformation::Resize {
                    width: 50,
                    height: 40,
                }],
                OutputFormat::Gif,
            ),
        )
        .expect("magick must process the animation");
    let frames = identify_frames(&processed, "%w %h %W %H %[pixel:p{15,12}]\n");
    assert_eq!(
        frames.len(),
        1,
        "only the first frame is processed: {frames:?}"
    );
    let first = &frames[0];
    assert!(
        first.starts_with("50 40 50 40 "),
        "the first frame, composed onto the 100x80 screen, then resized: {first}"
    );
    assert!(
        first.contains("red") || first.contains("255,0,0"),
        "the red first frame sits where it was placed: {first}"
    );
}
