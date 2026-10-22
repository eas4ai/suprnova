#![cfg(feature = "media")]
//! Default lossy quality and the setter's bounds.

use suprnova::{DEFAULT_IMAGE_QUALITY, Image, ImageDriverKind, ImagePipeline, OutputFormat};

use crate::image_processing::{photo_bmp, webp_chunks};

#[test]
fn img_009_default_quality_is_85() {
    assert_eq!(DEFAULT_IMAGE_QUALITY, 85);
    assert_eq!(ImagePipeline::default().quality, 85);
}

async fn assert_quality(driver: ImageDriverKind) {
    let source = photo_bmp();
    for format in [OutputFormat::Jpeg, OutputFormat::WebP] {
        let pipeline = Image::from_bytes(source.clone())
            .using(driver)
            .to_format(format);
        let default = pipeline.clone().to_bytes().await.expect("default encode");
        let explicit = pipeline
            .clone()
            .quality(85)
            .to_bytes()
            .await
            .expect("85 encode");
        let former = pipeline
            .clone()
            .quality(70)
            .to_bytes()
            .await
            .expect("70 encode");
        assert!(default == explicit, "{driver:?} {format:?} defaults to 85");
        assert!(default != former, "{driver:?} {format:?} differs at 70");
        if format == OutputFormat::WebP {
            assert!(webp_chunks(&default).iter().any(|chunk| chunk == "VP8 "));
        }
        for (input, bound) in [(0, 1), (255, 100)] {
            let clamped = pipeline
                .clone()
                .quality(input)
                .to_bytes()
                .await
                .expect("clamped encode");
            let explicit = pipeline
                .clone()
                .quality(bound)
                .to_bytes()
                .await
                .expect("bound encode");
            assert!(
                clamped == explicit,
                "{driver:?} {format:?} clamps {input} to {bound}"
            );
        }
    }
}

#[tokio::test]
#[serial_test::serial]
async fn img_009_oxideav_defaults_to_85_and_clamps_quality() {
    assert_quality(ImageDriverKind::OxideAv).await;
}

// The driver's policy check is cached per process. A real binary needs its
// own process so a stand-in in another test cannot supply that check. The
// case returns early, and says so, on a host without ImageMagick.
#[cfg(unix)]
#[test]
fn img_009_magick_defaults_to_85_and_clamps_quality() {
    if !crate::laravel_infra_gaps::magick_runs("img_009_magick_defaults_to_85_and_clamps_quality") {
        return;
    }
    crate::own_process::run_alone(
        "quality::img_009_magick_defaults_to_85_and_clamps_quality_child",
    );
}

#[cfg(unix)]
#[tokio::test]
async fn img_009_magick_defaults_to_85_and_clamps_quality_child() {
    if !crate::own_process::is_child() {
        return;
    }
    assert_quality(ImageDriverKind::Magick).await;
}
