//! MEM-003 on images. The image pipeline exists only with the framework's
//! `media` feature, so this test does too.

#![cfg(feature = "media")]

use oxideav_gif::{Block, DisposalMethod, Frame, GifImage, GraphicControl, Rgb, Version};
use oxideav_png::{PngEncoderOptions, PngImage, PngPixelFormat};
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

/// One decode under `budget`: the result, and the most bytes it held at once.
fn decode_under(
    driver: &OxideAvImageDriver,
    image: &[u8],
    budget: u64,
) -> (Result<(u32, u32), String>, u64) {
    let _config = ConfigGuard::set(ImageConfig {
        max_alloc_bytes: budget,
        ..ImageConfig::default()
    });
    let heap = Heap::start();
    let start = heap.live();
    let result = driver.dimensions(image).map_err(|e| e.to_string());
    let peak = (heap.peak() - start) as u64;
    drop(heap);
    (result, peak)
}

/// The configured budget holds while `image` decodes.
///
/// At the budget the header gate allows (four bytes a pixel) the decode is
/// either refused before it allocates or stays within that budget. The
/// refusal names the driver's estimate of the decode; at exactly that budget
/// the decode runs and stays within it, and one byte less is refused.
fn assert_the_budget_holds(name: &str, image: &[u8], width: u32, height: u32) {
    let driver = OxideAvImageDriver::new();
    let header_budget = u64::from(width) * u64::from(height) * 4;
    let (result, peak) = decode_under(&driver, image, header_budget);
    let message = match result {
        Ok(_) => {
            panic!("{name}: admitted at a {header_budget}-byte budget and peaked at {peak} bytes")
        }
        Err(message) => message,
    };
    assert!(
        peak < 64 * 1024,
        "{name}: the refusal itself held {peak} bytes"
    );
    let estimate: u64 = message
        .split("needs about ")
        .nth(1)
        .and_then(|rest| rest.split(' ').next())
        .and_then(|number| number.parse().ok())
        .unwrap_or_else(|| panic!("{name}: the refusal names no estimate: {message}"));

    let (result, peak) = decode_under(&driver, image, estimate);
    assert_eq!(
        result.unwrap_or_else(|e| panic!("{name}: refused at its own estimate: {e}")),
        (width, height)
    );
    assert!(
        peak <= estimate,
        "{name}: peaked at {peak} bytes over its {estimate}-byte estimate"
    );

    let (result, _) = decode_under(&driver, image, estimate - 1);
    assert!(
        result.is_err(),
        "{name}: admitted one byte under its {estimate}-byte estimate"
    );
}

/// Deterministic noise, so encoders cannot shrink it and every decode
/// buffer that scales with the compressed size is exercised.
fn noise(width: u32, height: u32, channels: usize) -> Vec<u8> {
    let mut state = 0x9E37_79B9u32;
    (0..width as usize * height as usize * channels)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state as u8
        })
        .collect()
}

fn encode_png(
    width: u32,
    height: u32,
    pixel_format: PngPixelFormat,
    channels: usize,
    interlace: bool,
) -> Vec<u8> {
    let stride = width as usize * channels;
    oxideav_png::encode_png_image_with_options(
        &PngImage {
            width,
            height,
            pixel_format,
            stride,
            data: noise(width, height, channels),
            palette: Vec::new(),
        },
        &PngEncoderOptions {
            interlace,
            ..PngEncoderOptions::default()
        },
    )
    .expect("the PNG encodes")
}

/// `image` re-encoded by the driver itself.
fn convert(image: &[u8], format: OutputFormat) -> Vec<u8> {
    let _config = ConfigGuard::set(ImageConfig::default());
    OxideAvImageDriver::new()
        .process(
            image,
            &ImagePipeline {
                format: Some(format),
                ..Default::default()
            },
        )
        .expect("the conversion runs")
}

/// Item 1: an 8-bit RGBA PNG held about four times its RGBA footprint.
#[tokio::test]
async fn mem_audit_an_rgba_png_decodes_within_the_budget() {
    let _lock = exclusive().await;
    let png = encode_png(256, 256, PngPixelFormat::Rgba, 4, false);
    assert_the_budget_holds("8-bit RGBA PNG", &png, 256, 256);
}

/// Item 1: a 16-bit PNG inflates to twice what the header gate counted.
#[tokio::test]
async fn mem_audit_a_sixteen_bit_png_decodes_within_the_budget() {
    let _lock = exclusive().await;
    let png = encode_png(256, 256, PngPixelFormat::Rgba64Le, 8, false);
    assert_the_budget_holds("16-bit RGBA PNG", &png, 256, 256);
    // Odd sides, so Adam7 leaves passes partial.
    let interlaced = encode_png(255, 201, PngPixelFormat::Rgba64Le, 8, true);
    assert_the_budget_holds("interlaced 16-bit RGBA PNG", &interlaced, 255, 201);
}

/// Item 2: every chunk costs two 24-byte entries in oxideav-png's chunk
/// lists, so a PNG of empty chunks holds several times its own size.
#[tokio::test]
async fn mem_audit_a_png_of_empty_chunks_decodes_within_the_budget() {
    let _lock = exclusive().await;
    let small = encode_png(64, 64, PngPixelFormat::Rgba, 4, false);
    let mut png = PNG_SIGNATURE.to_vec();
    let mut pos = PNG_SIGNATURE.len();
    while pos < small.len() {
        let (chunk, next) = oxideav_png::chunk::read_chunk(&small, pos).expect("a chunk");
        if chunk.is_type(b"IDAT") {
            // An unknown ancillary chunk the decoder lists and skips.
            for _ in 0..100_000 {
                oxideav_png::chunk::write_chunk(&mut png, b"zzZz", &[]);
            }
        }
        oxideav_png::chunk::write_chunk(&mut png, &chunk.chunk_type, chunk.data);
        pos = next;
    }
    assert_the_budget_holds("PNG of 100,000 empty chunks", &png, 64, 64);
}

/// Item 1: composing a GIF frame holds three screen-sized canvases.
#[tokio::test]
async fn mem_audit_a_gif_decodes_within_the_budget() {
    let _lock = exclusive().await;
    let mut frames = vec![frame(0, 0, 256, 256, 0)];
    // DRIVERS-043: the frames after the first cost nothing.
    frames.extend((0..64).map(|_| frame(0, 0, 1, 1, 1)));
    assert_the_budget_holds("animated GIF", &gif(256, 256, frames), 256, 256);

    let mut interlaced = frame(0, 0, 256, 256, 2);
    interlaced.interlaced = true;
    assert_the_budget_holds("interlaced GIF", &gif(256, 256, vec![interlaced]), 256, 256);
}

/// Item 1: JPEG, WebP and BMP, each as the driver writes it.
#[tokio::test]
async fn mem_audit_jpeg_webp_and_bmp_decode_within_the_budget() {
    let _lock = exclusive().await;
    let opaque: Vec<u8> = noise(256, 256, 3)
        .chunks(3)
        .flat_map(|rgb| [rgb[0], rgb[1], rgb[2], 255])
        .collect();
    let source = oxideav_png::encode_png_image(&PngImage {
        width: 256,
        height: 256,
        pixel_format: PngPixelFormat::Rgba,
        stride: 256 * 4,
        data: opaque,
        palette: Vec::new(),
    })
    .expect("the PNG encodes");
    for (name, format) in [
        ("JPEG", OutputFormat::Jpeg),
        ("lossy WebP", OutputFormat::WebP),
        ("lossless WebP", OutputFormat::WebPLossless),
        ("BMP", OutputFormat::Bmp),
    ] {
        assert_the_budget_holds(name, &convert(&source, format), 256, 256);
    }
}

/// Item 4: a file read stops one byte past the cap, and the buffer it reads
/// into never grows past that either. A FIFO reports a size of zero, so the
/// buffer starts empty and grows as the data arrives.
#[cfg(unix)]
#[tokio::test]
async fn mem_audit_a_file_read_never_reserves_past_the_cap() {
    use std::io::Write;

    let _lock = exclusive().await;
    // Not a power of two, so doubling growth would overshoot it.
    const CAP: u64 = 1_500_000;
    let dir = tempfile::tempdir().expect("a directory");
    let fifo = dir.path().join("endless.png");
    let made = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .expect("run mkfifo");
    assert!(made.success(), "mkfifo failed");
    let writer_path = fifo.clone();
    let writer = std::thread::spawn(move || {
        if let Ok(mut pipe) = std::fs::OpenOptions::new().write(true).open(&writer_path) {
            let chunk = vec![0u8; 64 * 1024];
            for _ in 0..256 {
                if pipe.write_all(&chunk).is_err() {
                    break;
                }
            }
        }
    });

    let _config = ConfigGuard::set(ImageConfig {
        max_alloc_bytes: CAP,
        ..ImageConfig::default()
    });
    let heap = Heap::start();
    let start = heap.live();
    let result = suprnova::Image::from_path(&fifo).to_bytes().await;
    let peak = heap.peak() - start;
    drop(heap);
    let _ = writer.join();

    assert!(result.is_err(), "the source is larger than the cap");
    assert!(
        (peak as u64) < CAP + 256 * 1024,
        "reading a source capped at {CAP} bytes held {peak}"
    );
}

/// An odd-sized 4:2:0 or 4:2:2 JPEG converts at its padded size, which the
/// estimate charges.
#[tokio::test]
async fn mem_audit_odd_sized_jpegs_decode_within_the_budget() {
    let _lock = exclusive().await;
    for (name, jpeg) in [
        (
            "4:2:0 JPEG",
            &include_bytes!("../media/fixtures/jpeg-420-333x217.jpg")[..],
        ),
        (
            "4:2:2 JPEG",
            &include_bytes!("../media/fixtures/jpeg-422-333x217.jpg")[..],
        ),
    ] {
        assert_the_budget_holds(name, jpeg, 333, 217);
    }
}
