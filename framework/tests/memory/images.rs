//! MEM-003 on images. The image pipeline exists only with the framework's
//! `media` feature, so this test does too.

#![cfg(feature = "media")]

use suprnova::media::{
    ImageDriver, ImagePipeline, OutputFormat, OxideAvImageDriver, Transformation,
};

use crate::support::{Heap, exclusive};

const RED_PNG_1X1: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53,
    0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x78, 0xDA, 0x63, 0xF8, 0xCF, 0xC0, 0x00,
    0x00, 0x03, 0x01, 0x01, 0x00, 0xF7, 0x03, 0x41, 0x43, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E,
    0x44, 0xAE, 0x42, 0x60, 0x82,
];

/// MEM-003: an image pipeline moves its planes from step to step and into
/// the encoder rather than copying each one.
#[tokio::test]
async fn mem_audit_an_image_pipeline_moves_its_planes() {
    let _lock = exclusive().await;
    let driver = OxideAvImageDriver::new();
    let bmp = driver
        .process(
            RED_PNG_1X1,
            &ImagePipeline {
                transformations: vec![Transformation::Resize {
                    width: 1024,
                    height: 1024,
                }],
                format: Some(OutputFormat::Bmp),
                ..Default::default()
            },
        )
        .expect("a 1024 by 1024 bitmap");
    let steps = ImagePipeline {
        transformations: vec![
            Transformation::FlipVertically,
            Transformation::FlipHorizontally,
            Transformation::Grayscale,
        ],
        format: Some(OutputFormat::Bmp),
        ..Default::default()
    };
    driver.process(&bmp, &steps).expect("a warm-up");

    const PLANE: u64 = 1024 * 1024 * 4;
    let heap = Heap::start();
    let before = heap.bytes();
    driver.process(&bmp, &steps).expect("the pipeline");
    let used = heap.bytes() - before;
    assert!(
        used < 11 * PLANE,
        "three steps over a 4 MiB plane allocated {used} bytes"
    );
}
