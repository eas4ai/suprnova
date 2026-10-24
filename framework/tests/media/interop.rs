#![cfg(feature = "media")]
//! Images other software writes, in shapes the driver's own encoders do not
//! produce: odd-sized subsampled JPEGs, and GIFs from ImageMagick and Pillow.
//! The fixtures are small files checked in under `fixtures/`.
//!
//! `#[serial]` for the same reason as `image_processing`: other tests in
//! this binary install a process-global `ImageConfig` override.

use suprnova::media::ImageDriver;

use crate::image_processing::decoded_rgba;

/// The RGBA of pixel `(x, y)` in a `width`-wide packed buffer.
fn pixel(rgba: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let at = (y as usize * width as usize + x as usize) * 4;
    [rgba[at], rgba[at + 1], rgba[at + 2], rgba[at + 3]]
}

/// True when every channel of `got` is within `tolerance` of `want`.
fn close(got: [u8; 4], want: [u8; 4], tolerance: u8) -> bool {
    got.iter()
        .zip(want)
        .all(|(&g, w)| g.abs_diff(w) <= tolerance)
}

/// Odd sides are ordinary (crops, resized exports), and 4:2:0 and 4:2:2
/// chroma does not divide them evenly. Each fixture is 333x217, its left
/// half `#c03020` and its right half `#2060c0`, written by ImageMagick at
/// quality 95; lossy coding moves a channel by a few levels.
#[tokio::test]
#[serial_test::serial]
async fn odd_sized_subsampled_jpegs_decode_at_their_size() {
    for (name, jpeg) in [
        (
            "4:2:0",
            &include_bytes!("fixtures/jpeg-420-333x217.jpg")[..],
        ),
        (
            "4:2:2",
            &include_bytes!("fixtures/jpeg-422-333x217.jpg")[..],
        ),
    ] {
        let (width, height, rgba) = decoded_rgba(jpeg).await;
        assert_eq!((width, height), (333, 217), "{name}");
        let red = [0xC0, 0x30, 0x20, 0xFF];
        let blue = [0x20, 0x60, 0xC0, 0xFF];
        for (x, y, want) in [
            (0, 0, red),
            (100, 108, red),
            (240, 108, blue),
            (332, 0, blue),
            (332, 216, blue),
            (0, 216, red),
        ] {
            let got = pixel(&rgba, width, x, y);
            assert!(
                close(got, want, 12),
                "{name} pixel ({x}, {y}) is {got:?}, expected about {want:?}"
            );
        }
    }
}

/// One GIF fixture: its bytes, ImageMagick's RGBA of its first frame
/// composed onto the screen (`magick X.gif -coalesce -delete 1--1 rgba:`),
/// and the rectangle that first frame covers. ImageMagick paints the rest of
/// the screen with the GIF's background color; the driver leaves it
/// transparent, as browsers do.
struct GifFixture {
    name: &'static str,
    gif: &'static [u8],
    reference: &'static [u8],
    covered: (u32, u32, u32, u32),
}

const GIF_FIXTURES: [GifFixture; 8] = [
    GifFixture {
        name: "ImageMagick, 200 colors",
        gif: include_bytes!("fixtures/gif-im-static.gif"),
        reference: include_bytes!("fixtures/gif-im-static.rgba"),
        covered: (0, 0, 48, 32),
    },
    GifFixture {
        name: "ImageMagick, animated, first frame at an offset",
        gif: include_bytes!("fixtures/gif-im-animated.gif"),
        reference: include_bytes!("fixtures/gif-im-animated.rgba"),
        covered: (8, 6, 20, 12),
    },
    GifFixture {
        name: "ImageMagick, interlaced",
        gif: include_bytes!("fixtures/gif-im-interlaced.gif"),
        reference: include_bytes!("fixtures/gif-im-interlaced.rgba"),
        covered: (0, 0, 48, 32),
    },
    GifFixture {
        name: "Pillow, 256 colors",
        gif: include_bytes!("fixtures/gif-pil-static.gif"),
        reference: include_bytes!("fixtures/gif-pil-static.rgba"),
        covered: (0, 0, 48, 32),
    },
    GifFixture {
        name: "Pillow, animated",
        gif: include_bytes!("fixtures/gif-pil-animated.gif"),
        reference: include_bytes!("fixtures/gif-pil-animated.rgba"),
        covered: (0, 0, 48, 32),
    },
    GifFixture {
        name: "Pillow, interlaced",
        gif: include_bytes!("fixtures/gif-pil-interlaced.gif"),
        reference: include_bytes!("fixtures/gif-pil-interlaced.rgba"),
        covered: (0, 0, 48, 32),
    },
    GifFixture {
        name: "Pillow, first frame with a local color table",
        gif: include_bytes!("fixtures/gif-pil-local-palette.gif"),
        reference: include_bytes!("fixtures/gif-pil-local-palette.rgba"),
        covered: (0, 0, 48, 32),
    },
    GifFixture {
        name: "Pillow, with a transparent index",
        gif: include_bytes!("fixtures/gif-pil-transparent.gif"),
        reference: include_bytes!("fixtures/gif-pil-transparent.rgba"),
        covered: (0, 0, 48, 32),
    },
];

/// GIFs written by ImageMagick and Pillow decode to the pixels ImageMagick
/// decodes: static and animated, interlaced, a first frame at an offset or
/// with its own color table, and a transparent index. Every fixture uses
/// LZW codes wider than its minimum code size, which is where an LZW
/// decoder that changes code width one code early goes wrong.
#[tokio::test]
#[serial_test::serial]
async fn gifs_from_imagemagick_and_pillow_decode_to_their_pixels() {
    for fixture in &GIF_FIXTURES {
        let name = fixture.name;
        let (width, height, rgba) = decoded_rgba(fixture.gif).await;
        assert_eq!((width, height), (48, 32), "{name}");
        let (left, top, frame_width, frame_height) = fixture.covered;
        for y in 0..height {
            for x in 0..width {
                let got = pixel(&rgba, width, x, y);
                let inside = (left..left + frame_width).contains(&x)
                    && (top..top + frame_height).contains(&y);
                let want = pixel(fixture.reference, width, x, y);
                if !inside || want[3] == 0 {
                    assert_eq!(
                        got[3], 0,
                        "{name} pixel ({x}, {y}) is {got:?}, expected transparent"
                    );
                } else {
                    assert_eq!(got, want, "{name} pixel ({x}, {y})");
                }
            }
        }
    }
}

/// Writes variable-width LZW codes the way a GIF decoder reads them, least
/// significant bit first, tracking the decoder's dictionary so each code is
/// written at the width the decoder reads it with.
struct LzwCodes {
    bytes: Vec<u8>,
    pending: u64,
    pending_bits: u32,
    width: u32,
    next: u16,
    defined: bool,
}

impl LzwCodes {
    /// Codes for a two-color table: clear code 4, first entry 6, 3 bits.
    fn new() -> Self {
        Self {
            bytes: Vec::new(),
            pending: 0,
            pending_bits: 0,
            width: 3,
            next: 6,
            defined: false,
        }
    }

    fn put(&mut self, code: u16) {
        self.pending |= u64::from(code) << self.pending_bits;
        self.pending_bits += self.width;
        while self.pending_bits >= 8 {
            self.bytes.push(self.pending as u8);
            self.pending >>= 8;
            self.pending_bits -= 8;
        }
        if code == 4 {
            self.width = 3;
            self.next = 6;
            self.defined = false;
            return;
        }
        // Every code after the first defines the next entry until the
        // dictionary is full, and the width grows when it fills.
        if self.defined && self.next < 4096 {
            self.next += 1;
            if u32::from(self.next) == 1 << self.width && self.width < 12 {
                self.width += 1;
            }
        }
        self.defined = true;
    }

    /// The codes as GIF data sub-blocks, with the terminator.
    fn sub_blocks(mut self) -> Vec<u8> {
        if self.pending_bits > 0 {
            self.bytes.push(self.pending as u8);
        }
        let mut out = Vec::new();
        for block in self.bytes.chunks(255) {
            out.push(block.len() as u8);
            out.extend_from_slice(block);
        }
        out.push(0);
        out
    }
}

/// A 1x1 GIF whose single pixel is the first LZW code, followed by about a
/// megabyte of valid codes that each expand to some 4000 pixels: a decoder
/// that reads every code to the end of the data walks billions of dictionary
/// steps for pixels past the frame.
fn gif_with_a_long_tail() -> Vec<u8> {
    let mut codes = LzwCodes::new();
    codes.put(4);
    codes.put(0);
    // Each code one past the dictionary defines a longer run of zeros.
    while codes.next < 4096 {
        let next = codes.next;
        codes.put(next);
    }
    // The dictionary is full: repeat its longest entry.
    for _ in 0..700_000 {
        codes.put(4095);
    }
    codes.put(5);

    let mut gif = Vec::from(*b"GIF89a");
    gif.extend_from_slice(&[1, 0, 1, 0, 0x80, 0, 0]);
    gif.extend_from_slice(&[0x12, 0x34, 0x56, 0xFF, 0xFF, 0xFF]);
    gif.extend_from_slice(&[0x2C, 0, 0, 0, 0, 1, 0, 1, 0, 0, 2]);
    gif.extend_from_slice(&codes.sub_blocks());
    gif.push(0x3B);
    gif
}

/// Decoding stops when the frame is complete; the codes after it are never
/// read.
#[tokio::test]
#[serial_test::serial]
async fn a_gif_frame_stops_decoding_when_it_is_complete() {
    let gif = gif_with_a_long_tail();
    assert!(gif.len() > 1_000_000, "the tail is {} bytes", gif.len());
    let started = std::time::Instant::now();
    let (width, height, rgba) = decoded_rgba(&gif).await;
    assert_eq!((width, height), (1, 1));
    assert_eq!(pixel(&rgba, 1, 0, 0), [0x12, 0x34, 0x56, 0xFF]);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(10),
        "decoding a 1x1 frame took {:?}",
        started.elapsed()
    );
}

/// The JPEG fixtures under `fixtures/jpeg/`, read at run time so a new one
/// is picked up without listing it. `prefix` selects a set.
fn jpeg_fixtures(prefix: &str) -> Vec<(String, Vec<u8>)> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/media/fixtures/jpeg");
    let mut found: Vec<(String, Vec<u8>)> = std::fs::read_dir(&dir)
        .expect("the JPEG fixture directory exists")
        .map(|entry| entry.expect("a directory entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "jpg"))
        .filter_map(|path| {
            let name = path.file_stem()?.to_string_lossy().into_owned();
            name.starts_with(prefix)
                .then(|| (name, std::fs::read(&path).expect("the fixture reads")))
        })
        .collect();
    found.sort();
    found
}

/// RGBA decoded by the built-in driver, without the facade.
fn driver_rgba(jpeg: &[u8]) -> Result<Vec<u8>, String> {
    let bmp = suprnova::OxideAvImageDriver::new()
        .process(
            jpeg,
            &suprnova::media::ImagePipeline {
                format: Some(suprnova::media::OutputFormat::Bmp),
                ..Default::default()
            },
        )
        .map_err(|e| e.to_string())?;
    Ok(crate::image_processing::bmp_rgba_pixels(&bmp))
}

/// Every coding the driver reads decodes to the pixels libjpeg-turbo's
/// `djpeg` produces: baseline, progressive and arithmetic coding, every
/// sampling the standard allows at 8 bits, greyscale, RGB-coded, one
/// component a scan, and lossless. Each fixture is 35x21 (both sides odd),
/// written by `cjpeg` at quality 85, next to `djpeg`'s RGB of it. Decoders
/// may round the inverse DCT and upsampling differently by a few levels;
/// lossless files must match exactly.
#[tokio::test]
#[serial_test::serial]
async fn jpegs_decode_to_libjpeg_turbos_pixels_in_every_coding() {
    let fixtures = jpeg_fixtures("photo-");
    assert_eq!(fixtures.len(), 20, "the accuracy fixtures");
    let mut failures = Vec::new();
    for (name, jpeg) in fixtures {
        let reference = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(format!("tests/media/fixtures/jpeg/{name}.rgb")),
        )
        .expect("each fixture has a reference");
        let rgba = match driver_rgba(&jpeg) {
            Ok(rgba) => rgba,
            Err(e) => {
                failures.push(format!("{name}: {e}"));
                continue;
            }
        };
        if rgba.len() / 4 != reference.len() / 3 {
            failures.push(format!(
                "{name}: {} pixels, expected {}",
                rgba.len() / 4,
                reference.len() / 3
            ));
            continue;
        }
        let (mut worst, mut total) = (0u8, 0u64);
        for (got, want) in rgba.chunks(4).zip(reference.chunks(3)) {
            for channel in 0..3 {
                let difference = got[channel].abs_diff(want[channel]);
                worst = worst.max(difference);
                total += u64::from(difference);
            }
        }
        let mean = total as f64 / reference.len() as f64;
        let lossless = name.contains("lossless");
        if (lossless && worst != 0) || worst > 8 || mean > 1.0 {
            failures.push(format!(
                "{name}: off by up to {worst}, {mean:.2} on average"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "JPEGs that decode wrong:\n{}",
        failures.join("\n")
    );
}

/// A progressive JPEG whose only scan is the DC scan, every block's DC
/// difference zero: a mid-grey image of `width x height` in a few hundred
/// KiB, sampled 4:4:4.
fn dc_only_progressive_jpeg(width: u16, height: u16) -> Vec<u8> {
    fn segment(jpeg: &mut Vec<u8>, marker: u8, body: &[u8]) {
        jpeg.extend_from_slice(&[0xFF, marker]);
        jpeg.extend_from_slice(&((body.len() + 2) as u16).to_be_bytes());
        jpeg.extend_from_slice(body);
    }
    let mut jpeg = vec![0xFF, 0xD8];
    let mut quantization = vec![0x00];
    quantization.extend_from_slice(&[1; 64]);
    segment(&mut jpeg, 0xDB, &quantization);
    let mut frame = vec![8];
    frame.extend_from_slice(&height.to_be_bytes());
    frame.extend_from_slice(&width.to_be_bytes());
    frame.extend_from_slice(&[3, 1, 0x11, 0, 2, 0x11, 0, 3, 0x11, 0]);
    segment(&mut jpeg, 0xC2, &frame);
    // One DC code, one bit long, for difference category 0.
    let mut huffman = vec![0x00, 1];
    huffman.extend_from_slice(&[0; 15]);
    huffman.push(0);
    segment(&mut jpeg, 0xC4, &huffman);
    segment(&mut jpeg, 0xDA, &[3, 1, 0x00, 2, 0x00, 3, 0x00, 0, 0, 0]);
    let blocks = usize::from(width).div_ceil(8) * usize::from(height).div_ceil(8) * 3;
    jpeg.resize(jpeg.len() + blocks / 8, 0);
    if blocks % 8 != 0 {
        jpeg.push((1u8 << (8 - blocks % 8)) - 1);
    }
    jpeg.extend_from_slice(&[0xFF, 0xD9]);
    jpeg
}

/// A 48-megapixel photo, 8000x6000 in progressive 4:4:4 (the costliest
/// layout to decode), decodes under the default IMAGE_MAX_ALLOC_BYTES.
#[tokio::test]
#[serial_test::serial]
async fn a_48_megapixel_progressive_jpeg_decodes_at_the_default_budget() {
    suprnova::media::set_config_for_tests(None);
    let jpeg = dc_only_progressive_jpeg(8000, 6000);
    let png = suprnova::OxideAvImageDriver::new()
        .process(
            &jpeg,
            &suprnova::media::ImagePipeline {
                transformations: vec![suprnova::media::Transformation::Resize {
                    width: 80,
                    height: 60,
                }],
                format: Some(suprnova::media::OutputFormat::Bmp),
                ..Default::default()
            },
        )
        .unwrap_or_else(|e| panic!("a 48-megapixel progressive JPEG must decode: {e}"));
    let rgba = crate::image_processing::bmp_rgba_pixels(&png);
    assert_eq!(rgba.len(), 80 * 60 * 4);
    assert!(
        rgba.chunks(4)
            .all(|px| close([px[0], px[1], px[2], px[3]], [128, 128, 128, 255], 1)),
        "the image is mid-grey"
    );
}
