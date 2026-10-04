#![cfg(feature = "media")]
//! Images other software writes, in shapes the driver's own encoders do not
//! produce: odd-sized subsampled JPEGs, and GIFs from ImageMagick and Pillow.
//! The fixtures are small files checked in under `fixtures/`.
//!
//! `#[serial]` for the same reason as `image_processing`: other tests in
//! this binary install a process-global `ImageConfig` override.

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
