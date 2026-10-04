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
