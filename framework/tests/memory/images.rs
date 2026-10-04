//! MEM-003 on images. The image pipeline exists only with the framework's
//! `media` feature, so this test does too.

#![cfg(feature = "media")]

use oxideav_gif::{Block, DisposalMethod, Frame, GifImage, GraphicControl, Rgb, Version};
use oxideav_png::{PngImage, PngPixelFormat};
use suprnova::ImageConfig;
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

/// Installs an `ImageConfig` override and clears it on drop, so a failed
/// assertion cannot leak a tightened budget into the next test.
struct ConfigGuard;

impl ConfigGuard {
    fn set(config: ImageConfig) -> Self {
        suprnova::media::set_config_for_tests(Some(config));
        Self
    }
}

impl Drop for ConfigGuard {
    fn drop(&mut self) {
        suprnova::media::set_config_for_tests(None);
    }
}

const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

/// A PNG whose header declares 1x1 RGBA but whose IDAT is the pixel data of
/// a `width x height` RGBA image, `height * (1 + width * 4)` bytes inflated.
fn png_declaring_one_pixel(width: u32, height: u32) -> Vec<u8> {
    let stride = width as usize * 4;
    let large = oxideav_png::encode_png_image(&PngImage {
        width,
        height,
        pixel_format: PngPixelFormat::Rgba,
        stride,
        data: vec![0u8; stride * height as usize],
        palette: Vec::new(),
    })
    .expect("the large image encodes");
    let mut idat = Vec::new();
    let mut pos = PNG_SIGNATURE.len();
    while pos < large.len() {
        let (chunk, next) =
            oxideav_png::chunk::read_chunk(&large, pos).expect("a well-formed chunk");
        if chunk.is_type(b"IDAT") {
            idat.extend_from_slice(chunk.data);
        }
        pos = next;
    }

    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&1u32.to_be_bytes());
    ihdr.extend_from_slice(&1u32.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    let mut png = PNG_SIGNATURE.to_vec();
    oxideav_png::chunk::write_chunk(&mut png, b"IHDR", &ihdr);
    oxideav_png::chunk::write_chunk(&mut png, b"IDAT", &idat);
    oxideav_png::chunk::write_chunk(&mut png, b"IEND", &[]);
    png
}

/// A four-colour GIF89a on a `width x height` logical screen.
fn gif(width: u16, height: u16, frames: Vec<Frame>) -> Vec<u8> {
    oxideav_gif::encode(&GifImage {
        version: Version::Gif89a,
        screen_width: width,
        screen_height: height,
        color_resolution: 7,
        global_palette_sorted: false,
        background_index: 0,
        pixel_aspect_ratio: 0,
        global_palette: Some(vec![
            Rgb::new(255, 0, 0),
            Rgb::new(0, 255, 0),
            Rgb::new(0, 0, 255),
            Rgb::new(0, 0, 0),
        ]),
        blocks: frames.into_iter().map(Block::Image).collect(),
    })
    .expect("the GIF encodes")
}

/// A solid frame of palette entry `index`, kept on the canvas after it shows.
fn frame(left: u16, top: u16, width: u16, height: u16, index: u8) -> Frame {
    Frame {
        left,
        top,
        width,
        height,
        local_palette: None,
        palette_sorted: false,
        interlaced: false,
        indices: vec![index; usize::from(width) * usize::from(height)],
        graphic_control: Some(GraphicControl {
            disposal: DisposalMethod::Keep,
            user_input: false,
            transparent_index: None,
            delay_centis: 10,
        }),
    }
}

/// DRIVERS-042: a PNG that declares one pixel is refused before its pixel
/// data is inflated, rather than after a decoder has inflated all of it.
#[tokio::test]
async fn mem_audit_a_png_that_inflates_past_its_header_is_refused_before_inflating() {
    let _lock = exclusive().await;
    let driver = OxideAvImageDriver::new();
    let png = png_declaring_one_pixel(2048, 512);
    let inflated = 512 * (1 + 2048 * 4);

    let heap = Heap::start();
    let start = heap.live();
    let result = driver.dimensions(&png);
    let peak = heap.peak() - start;
    drop(heap);

    assert!(
        peak < inflated / 4,
        "refusing the PNG peaked at {peak} bytes; its pixel data inflates to {inflated}"
    );
    let err = result.expect_err("pixel data past the declared size must be refused");
    assert!(err.to_string().contains("inflates past"), "got: {err}");
}

/// DRIVERS-043: an animation costs the canvas of its first frame, which is
/// the only one the pipeline uses, not a canvas for every frame.
#[tokio::test]
async fn mem_audit_an_animated_gif_costs_one_canvas_not_one_per_frame() {
    let _lock = exclusive().await;
    const SIDE: u16 = 256;
    let canvas = usize::from(SIDE) * usize::from(SIDE) * 4;
    let mut frames = vec![frame(0, 0, SIDE, SIDE, 0)];
    frames.extend((0..64).map(|_| frame(0, 0, 1, 1, 1)));
    let animation = gif(SIDE, SIDE, frames);
    // Room for eight canvases; all 65 frames as canvases need 65.
    let budget = 8 * canvas;
    let _config = ConfigGuard::set(ImageConfig {
        max_alloc_bytes: budget as u64,
        ..ImageConfig::default()
    });
    let driver = OxideAvImageDriver::new();

    let heap = Heap::start();
    let start = heap.live();
    let dimensions = driver.dimensions(&animation);
    let peak = heap.peak() - start;
    drop(heap);

    assert!(
        peak < budget,
        "decoding the first of 65 frames peaked at {peak} bytes against a {budget}-byte budget"
    );
    assert_eq!(
        dimensions.expect("the first frame decodes"),
        (u32::from(SIDE), u32::from(SIDE))
    );
}

/// DRIVERS-043: a first frame larger than its logical screen is refused
/// before its pixels are decoded. The gate measured only the screen.
#[tokio::test]
async fn mem_audit_a_gif_frame_larger_than_its_screen_is_refused_before_decoding() {
    let _lock = exclusive().await;
    let malformed = gif(16, 16, vec![frame(0, 0, 2048, 2048, 0)]);
    let driver = OxideAvImageDriver::new();

    let heap = Heap::start();
    let start = heap.live();
    let result = driver.dimensions(&malformed);
    let peak = heap.peak() - start;
    drop(heap);

    assert!(
        peak < 1024 * 1024,
        "refusing the frame peaked at {peak} bytes; decoding it takes 4 MiB"
    );
    assert!(
        result.is_err(),
        "a frame larger than its screen must be refused"
    );
}
