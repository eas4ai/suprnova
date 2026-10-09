#![cfg(feature = "media")]
//! The Laravel infrastructure gaps of the image pipeline: sides clamped to
//! at least 1 before a driver sees them, the one-side resizes that keep the
//! other side (PAR-155), and custom transformations with per-call settings
//! (PAR-156).
//!
//! The ImageMagick cases are plain tests that return early, saying so on
//! stderr, when the host's `magick` binary does not run, so the mechanism
//! runs them wherever ImageMagick 7 is installed.
//!
//! Every test is `#[serial]`: some install a process-global `ImageConfig`
//! override, and a sibling running beside one would decode under it.

use std::sync::{Arc, Mutex};

use serial_test::serial;
use suprnova::{
    CustomTransformation, FrameworkError, Image, ImageConfig, ImageDriver, ImageDriverKind,
    ImagePipeline, ImagePixels, MagickCliDriver, OutputFormat, Transformation,
    register_transformation, register_transformation_with,
};

use crate::image_processing::ConfigGuard;

// ───────────────────────── fixtures ─────────────────────────

/// One colour, `width x height`, as a PNG.
fn flat_png(width: u32, height: u32) -> Vec<u8> {
    let rgba = [200u8, 60, 40, 255].repeat((width * height) as usize);
    oxideav_png::encode_plane(
        width,
        height,
        oxideav_png::PngPixelFormat::Rgba,
        width as usize * 4,
        &rgba,
        None,
        &oxideav_png::EncodeOptions::default(),
    )
    .expect("the fixture PNG encodes")
}

/// Whether the host's ImageMagick binary runs. The ImageMagick cases
/// return early without it, and say so.
fn magick_runs(test: &str) -> bool {
    let runs = std::process::Command::new(MagickCliDriver::from_env().binary())
        .arg("-version")
        .output()
        .is_ok_and(|output| output.status.success());
    if !runs {
        eprintln!("{test}: skipped, the host has no ImageMagick binary that runs");
    }
    runs
}

async fn dimensions_of(image: Image) -> (u32, u32) {
    image.dimensions().await.expect("the pipeline runs")
}

// ───────────────────────── PAR-155 ─────────────────────────

/// What the recording driver received, one pipeline per run.
static RECORDED: Mutex<Vec<Vec<Transformation>>> = Mutex::new(Vec::new());

/// A driver that records the steps it is handed and returns the input.
struct RecordingDriver;

impl ImageDriver for RecordingDriver {
    fn process(
        &self,
        contents: &[u8],
        pipeline: &ImagePipeline,
    ) -> Result<Vec<u8>, FrameworkError> {
        RECORDED
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(pipeline.transformations.clone());
        Ok(contents.to_vec())
    }

    fn dimensions(&self, _contents: &[u8]) -> Result<(u32, u32), FrameworkError> {
        Ok((1, 1))
    }

    fn dominant_color(&self, _contents: &[u8]) -> Result<String, FrameworkError> {
        Ok("#000000".to_owned())
    }

    fn name(&self) -> &'static str {
        "recording"
    }
}

/// Runs alone in a child process (see `own_process`): it installs the
/// process default driver, which the first image of a process fixes.
#[test]
fn every_side_reaches_a_driver_as_at_least_one() {
    crate::own_process::run_alone(
        "laravel_infra_gaps::every_side_reaches_a_driver_as_at_least_one_child",
    );
}

#[tokio::test]
async fn every_side_reaches_a_driver_as_at_least_one_child() {
    if !crate::own_process::is_child() {
        return;
    }
    suprnova::media::set_default_driver(Box::new(RecordingDriver))
        .expect("the child process is the first to install a driver");
    let png = flat_png(4, 2);
    let images = [
        Image::from_bytes(png.clone()).resize(0, 0),
        Image::from_bytes(png.clone()).cover(0, 0),
        Image::from_bytes(png.clone()).resize_width_only(0),
        Image::from_bytes(png.clone()).resize_height_only(0),
        Image::from_bytes(png.clone()).resize_width(0),
        Image::from_bytes(png.clone()).resize_height(0),
        Image::from_bytes(png.clone()).resize(3, 0).cover(0, 5),
    ];
    for image in images {
        image
            .to_bytes()
            .await
            .expect("the recording driver answers");
    }

    let recorded = RECORDED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    assert_eq!(
        recorded,
        vec![
            vec![Transformation::Resize {
                width: 1,
                height: 1
            }],
            vec![Transformation::Cover {
                width: 1,
                height: 1
            }],
            vec![Transformation::ResizeWidthOnly(1)],
            vec![Transformation::ResizeHeightOnly(1)],
            vec![Transformation::ResizeWidth(1)],
            vec![Transformation::ResizeHeight(1)],
            vec![
                Transformation::Resize {
                    width: 3,
                    height: 1
                },
                Transformation::Cover {
                    width: 1,
                    height: 5
                },
            ],
        ]
    );
}

#[tokio::test]
#[serial]
async fn a_one_side_resize_keeps_the_other_side_on_oxideav() {
    let png = flat_png(40, 20);
    let oxideav = |image: Image| image.using(ImageDriverKind::OxideAv);

    assert_eq!(
        dimensions_of(oxideav(
            Image::from_bytes(png.clone()).resize_width_only(10)
        ))
        .await,
        (10, 20)
    );
    assert_eq!(
        dimensions_of(oxideav(
            Image::from_bytes(png.clone()).resize_height_only(5)
        ))
        .await,
        (40, 5)
    );
    // The other side is the current one at that point of the pipeline.
    assert_eq!(
        dimensions_of(oxideav(
            Image::from_bytes(png.clone())
                .resize(30, 12)
                .resize_width_only(10)
        ))
        .await,
        (10, 12)
    );
    // `resize_width` keeps its meaning: it derives the height.
    assert_eq!(
        dimensions_of(oxideav(Image::from_bytes(png).resize_width(10))).await,
        (10, 5)
    );
}

#[tokio::test]
#[serial]
async fn a_one_side_resize_keeps_the_other_side_on_magick() {
    if !magick_runs("a_one_side_resize_keeps_the_other_side_on_magick") {
        return;
    }
    let png = flat_png(40, 20);
    let magick = |image: Image| image.using(ImageDriverKind::Magick);

    assert_eq!(
        dimensions_of(magick(Image::from_bytes(png.clone()).resize_width_only(10))).await,
        (10, 20)
    );
    assert_eq!(
        dimensions_of(magick(Image::from_bytes(png.clone()).resize_height_only(5))).await,
        (40, 5)
    );
    assert_eq!(
        dimensions_of(magick(
            Image::from_bytes(png.clone())
                .resize(30, 12)
                .resize_width_only(10)
        ))
        .await,
        (10, 12)
    );
    assert_eq!(
        dimensions_of(magick(
            Image::from_bytes(png.clone())
                .resize(30, 12)
                .resize_height_only(4)
        ))
        .await,
        (30, 4)
    );
    // A zero side reaches ImageMagick as 1, not as a `0x0` geometry.
    assert_eq!(
        dimensions_of(magick(Image::from_bytes(png.clone()).resize(0, 0))).await,
        (1, 1)
    );
    assert_eq!(
        dimensions_of(magick(Image::from_bytes(png).cover(0, 0))).await,
        (1, 1)
    );
}

#[tokio::test]
#[serial]
async fn a_one_side_resize_past_the_decode_limits_is_refused() {
    let png = flat_png(40, 20);
    let _config = ConfigGuard::set({
        let mut config = ImageConfig::default();
        config.max_dimension = 64;
        config
    });

    let error = Image::from_bytes(png.clone())
        .using(ImageDriverKind::OxideAv)
        .resize_width_only(100)
        .to_bytes()
        .await
        .expect_err("a 100 px side is over the 64 px limit");
    assert!(error.to_string().contains("limit"), "got: {error}");

    if magick_runs("a_one_side_resize_past_the_decode_limits_is_refused (magick half)") {
        Image::from_bytes(png)
            .using(ImageDriverKind::Magick)
            .resize_height_only(100)
            .to_bytes()
            .await
            .expect_err("ImageMagick refuses a 100 px side over the 64 px limit");
    }
}

// ───────────────────────── PAR-156 ─────────────────────────

/// The falsifier's settings.
struct Pixelate {
    size: u32,
}

/// Register `pixelate`, recording each size it sees into the returned list.
fn register_pixelate() -> Arc<Mutex<Vec<u32>>> {
    let seen: Arc<Mutex<Vec<u32>>> = Arc::default();
    let record = Arc::clone(&seen);
    register_transformation_with(
        "pixelate",
        move |pixels: ImagePixels, settings: &Pixelate| {
            record
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(settings.size);
            Ok(pixels)
        },
    );
    seen
}

#[tokio::test]
#[serial]
async fn a_transformation_with_settings_receives_each_images_own_settings() {
    let seen = register_pixelate();
    let png = flat_png(8, 8);

    let small = Image::from_bytes(png.clone())
        .using(ImageDriverKind::OxideAv)
        .transform_with("pixelate", Pixelate { size: 4 });
    let large = Image::from_bytes(png)
        .using(ImageDriverKind::OxideAv)
        .transform_with("pixelate", Pixelate { size: 8 });
    small.to_bytes().await.expect("the first image runs");
    large.to_bytes().await.expect("the second image runs");

    assert_eq!(*seen.lock().unwrap(), vec![4, 8]);
}

#[tokio::test]
#[serial]
async fn a_transformation_with_settings_receives_them_on_magick() {
    if !magick_runs("a_transformation_with_settings_receives_them_on_magick") {
        return;
    }
    let seen = register_pixelate();
    let png = flat_png(8, 8);

    for size in [4, 8] {
        Image::from_bytes(png.clone())
            .using(ImageDriverKind::Magick)
            .transform_with("pixelate", Pixelate { size })
            .to_png()
            .to_bytes()
            .await
            .expect("the image runs under ImageMagick");
    }

    assert_eq!(*seen.lock().unwrap(), vec![4, 8]);
}

#[tokio::test]
#[serial]
async fn settings_of_another_type_or_none_fail_naming_the_transformation() {
    register_pixelate();
    register_transformation("pixels-only-gap", Ok);
    let png = flat_png(4, 4);

    let wrong_type = Image::from_bytes(png.clone())
        .using(ImageDriverKind::OxideAv)
        .transform_with("pixelate", 4u32)
        .to_bytes()
        .await
        .expect_err("a u32 is not the registered Pixelate");
    assert!(wrong_type.to_string().contains("pixelate"), "{wrong_type}");

    let no_settings = Image::from_bytes(png.clone())
        .using(ImageDriverKind::OxideAv)
        .transform(Transformation::custom("pixelate"))
        .to_bytes()
        .await
        .expect_err("pixelate needs its settings");
    assert!(
        no_settings.to_string().contains("pixelate"),
        "{no_settings}"
    );

    let unwanted = Image::from_bytes(png)
        .using(ImageDriverKind::OxideAv)
        .transform_with("pixels-only-gap", Pixelate { size: 2 })
        .to_bytes()
        .await
        .expect_err("a transformation of the pixels only takes no settings");
    assert!(
        unwanted.to_string().contains("pixels-only-gap"),
        "{unwanted}"
    );
}

/// Settings that report whether they are still alive.
struct Tracked {
    _alive: Arc<()>,
}

#[tokio::test]
#[serial]
async fn the_settings_are_released_when_the_image_is_dropped() {
    register_transformation_with("tracked-gap", |pixels: ImagePixels, _: &Tracked| Ok(pixels));
    let alive = Arc::new(());
    let watch = Arc::downgrade(&alive);

    let image = Image::from_bytes(flat_png(4, 4))
        .using(ImageDriverKind::OxideAv)
        .transform_with("tracked-gap", Tracked { _alive: alive });
    let clone = image.clone();
    clone.to_bytes().await.expect("the clone runs");
    assert!(
        watch.upgrade().is_some(),
        "the image still holds its settings"
    );

    drop(image);
    assert!(
        watch.upgrade().is_none(),
        "the settings are released with the last image that recorded them"
    );
}

#[tokio::test]
#[serial]
async fn register_transformation_and_custom_keep_their_meaning() {
    fn assert_copy<T: Copy>() {}
    assert_copy::<Transformation>();
    assert_copy::<CustomTransformation>();

    register_transformation("invert-gap", |mut pixels: ImagePixels| {
        for byte in pixels.pixels_mut() {
            *byte = !*byte;
        }
        Ok(pixels)
    });
    let bytes = Image::from_bytes(flat_png(2, 2))
        .using(ImageDriverKind::OxideAv)
        .transform(Transformation::custom("invert-gap"))
        .to_format(OutputFormat::Png)
        .to_bytes()
        .await
        .expect("a transformation of the pixels only still runs");
    assert!(bytes.starts_with(b"\x89PNG"));
    assert_eq!(
        CustomTransformation::new("invert-gap")
            .apply(ImagePixels::new(1, 1, vec![0, 1, 2, 3]).expect("pixels"))
            .expect("apply runs a transformation of the pixels only")
            .pixels(),
        [255, 254, 253, 252]
    );
}
