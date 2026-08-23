//! Magic-byte detection, header-only dimension parsing, and the decode-limit
//! gate - the framework's DoS boundary for image input.
//!
//! # Why the framework parses headers itself
//!
//! This is the permanent design, not a stopgap. `oxideav-core` ships a
//! `DecoderLimits` struct, but no still-image codec in the published set
//! reads it, and the `oxideav-io` facade has no seam to pass one through.
//! More fundamentally: apps can install their own [`ImageDriver`] (and the
//! built-in `magick` driver shells out to a binary the framework does not
//! control), so a limit enforced inside any one codec would not be a
//! framework guarantee at all. The framework owns its DoS boundary by
//! reading the declared dimensions out of the input's own header - a few
//! dozen bytes, no allocation - and refusing oversized input *before* a
//! decoder is even constructed. A 1 GiB declared frame in a 4 KiB file dies
//! here.
//!
//! Every magic-byte decision in the subsystem lives in this module so the
//! HEIC check, the format allowlist, and the header parsers cannot drift
//! apart across the two built-in drivers.
//!
//! [`ImageDriver`]: super::ImageDriver

use crate::error::FrameworkError;

use super::ImageConfig;

/// A format the framework can recognise from magic bytes and measure from
/// its header.
///
/// This is deliberately the *framework's* allowlist, not any backend's
/// capability list: the OxideAV driver only ever asks its codec registry for
/// one of these five codec ids, chosen here. That gives the same property
/// `oxideav-io`'s `OpenOptions::allow_codecs` sandbox provides - no codec
/// outside a known-good set ever sees a byte - enforced one layer up, where
/// it also covers the drivers that do not use a codec registry at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InputFormat {
    /// PNG (and the first frame of an APNG).
    Png,
    /// JPEG, any of the baseline/progressive/lossless flavours.
    Jpeg,
    /// WebP: lossy (`VP8 `), lossless (`VP8L`), or extended (`VP8X`).
    WebP,
    /// GIF 87a or 89a.
    Gif,
    /// Windows bitmap.
    Bmp,
}

impl InputFormat {
    /// The OxideAV codec id that decodes this format.
    pub(crate) fn codec_id(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "mjpeg",
            Self::WebP => "webp",
            Self::Gif => "gif",
            Self::Bmp => "bmp",
        }
    }

    /// The `Content-Type` this format is served under.
    pub(crate) fn mime_type(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::WebP => "image/webp",
            Self::Gif => "image/gif",
            Self::Bmp => "image/bmp",
        }
    }

    /// The ImageMagick coder name for this format.
    ///
    /// Used to write `png:-` rather than a bare `-`, which pins the decoder
    /// instead of letting ImageMagick pick one from the input's own bytes.
    pub(crate) fn magick_coder(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpeg",
            Self::WebP => "webp",
            Self::Gif => "gif",
            Self::Bmp => "bmp",
        }
    }
}

/// Recognise one of the five supported formats from its leading bytes.
///
/// Returns `None` for anything else, including HEIC - callers that need to
/// distinguish "not an image we support" from "specifically HEIC" call
/// [`looks_like_heif`] first.
pub(crate) fn detect(bytes: &[u8]) -> Option<InputFormat> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some(InputFormat::Png);
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some(InputFormat::Jpeg);
    }
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Some(InputFormat::WebP);
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some(InputFormat::Gif);
    }
    if bytes.starts_with(b"BM") {
        return Some(InputFormat::Bmp);
    }
    None
}

/// True when the input is an ISO-BMFF HEIF/HEIC still image.
///
/// Checked before the format allowlist so HEIC gets the named, actionable
/// error the manual explains rather than a generic "unsupported format" -
/// HEIC uploads arrive from iOS clients constantly, and "we deliberately do
/// not ship this, here is what to do" is a far better answer than a shrug.
///
/// Matches the `ftyp` box brand, not the file extension: bytes 4..8 are the
/// box type and 8..12 the major brand. `mif1` is included because iOS writes
/// it for single-image HEIC files.
pub(crate) fn looks_like_heif(bytes: &[u8]) -> bool {
    if bytes.len() < 12 || &bytes[4..8] != b"ftyp" {
        return false;
    }
    matches!(
        &bytes[8..12],
        b"heic" | b"heix" | b"heim" | b"heis" | b"hevc" | b"hevx" | b"mif1" | b"msf1"
    )
}

/// Read `width x height` straight out of the format's header.
///
/// No decode, no allocation proportional to the image - just the handful of
/// bytes the container declares its dimensions in. This is what makes the
/// limit check meaningful: it happens before anything sizes a buffer.
pub(crate) fn header_dimensions(
    format: InputFormat,
    bytes: &[u8],
) -> Result<(u32, u32), FrameworkError> {
    let dims = match format {
        InputFormat::Png => png_dimensions(bytes),
        InputFormat::Jpeg => jpeg_dimensions(bytes),
        InputFormat::WebP => webp_dimensions(bytes),
        InputFormat::Gif => gif_dimensions(bytes),
        InputFormat::Bmp => bmp_dimensions(bytes),
    };
    dims.ok_or_else(|| {
        FrameworkError::param(format!(
            "image header is malformed: could not read {} dimensions",
            format.mime_type()
        ))
    })
}

/// Refuse input whose declared size exceeds the configured caps.
///
/// The allocation estimate is `width * height * 4` in `u64` - the decoded
/// RGBA footprint. `u64` because the whole point is that `u32 * u32`
/// overflows exactly where an attacker wants it to.
pub(crate) fn enforce_limits(
    width: u32,
    height: u32,
    config: &ImageConfig,
) -> Result<(), FrameworkError> {
    if width == 0 || height == 0 {
        return Err(FrameworkError::param(
            "image declares a zero width or height",
        ));
    }
    if width > config.max_dimension || height > config.max_dimension {
        return Err(FrameworkError::param(format!(
            "image exceeds configured decode limits: {width}x{height} exceeds the \
             IMAGE_MAX_DIMENSION limit of {}",
            config.max_dimension
        )));
    }
    let estimated = u64::from(width)
        .saturating_mul(u64::from(height))
        .saturating_mul(4);
    if estimated > config.max_alloc_bytes {
        return Err(FrameworkError::param(format!(
            "image exceeds configured decode limits: decoding {width}x{height} needs about \
             {estimated} bytes, over the IMAGE_MAX_ALLOC_BYTES limit of {}",
            config.max_alloc_bytes
        )));
    }
    Ok(())
}

/// The shared front door every built-in driver runs before decoding.
///
/// Rejects empty input, then - for the five formats the framework can
/// measure - reads the header and applies the caps. Returns `Ok(None)` when
/// the bytes are not one of those five, which is not itself an error: the
/// OxideAV driver treats it as unsupported, while the `magick` driver
/// proceeds (delegating breadth is its entire purpose) under ImageMagick's
/// own resource limits instead.
pub(crate) fn guard(
    bytes: &[u8],
    config: &ImageConfig,
) -> Result<Option<InputFormat>, FrameworkError> {
    if bytes.is_empty() {
        return Err(FrameworkError::param("image input is empty"));
    }
    let Some(format) = detect(bytes) else {
        return Ok(None);
    };
    let (width, height) = header_dimensions(format, bytes)?;
    enforce_limits(width, height, config)?;
    Ok(Some(format))
}

// ───────────────────────── per-format header parsers ─────────────────────────

fn be_u32(bytes: &[u8], at: usize) -> Option<u32> {
    let slice = bytes.get(at..at + 4)?;
    Some(u32::from_be_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn be_u16(bytes: &[u8], at: usize) -> Option<u16> {
    let slice = bytes.get(at..at + 2)?;
    Some(u16::from_be_bytes([slice[0], slice[1]]))
}

fn le_u16(bytes: &[u8], at: usize) -> Option<u16> {
    let slice = bytes.get(at..at + 2)?;
    Some(u16::from_le_bytes([slice[0], slice[1]]))
}

fn le_u32(bytes: &[u8], at: usize) -> Option<u32> {
    let slice = bytes.get(at..at + 4)?;
    Some(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

/// IHDR is mandated to be the first chunk, so its payload sits at a fixed
/// offset: 8-byte signature, 4-byte length, 4-byte type, then the dimensions.
fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.get(12..16)? != b"IHDR" {
        return None;
    }
    Some((be_u32(bytes, 16)?, be_u32(bytes, 20)?))
}

/// Walk the marker segments to the first Start-Of-Frame, which is where JPEG
/// declares its size. The loop is bounded so a file made entirely of
/// well-formed empty segments cannot spin.
fn jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let mut pos = 2usize; // past the SOI
    for _ in 0..1024 {
        // Segments are 0xFF-prefixed; fill bytes are legal padding.
        while bytes.get(pos) == Some(&0xFF) && bytes.get(pos + 1) == Some(&0xFF) {
            pos += 1;
        }
        if *bytes.get(pos)? != 0xFF {
            return None;
        }
        let marker = *bytes.get(pos + 1)?;
        match marker {
            // Standalone markers: no payload length follows.
            0x01 | 0xD0..=0xD7 => {
                pos += 2;
                continue;
            }
            // End of image, or the start of entropy-coded scan data. Either
            // way there is no SOF ahead of us any more.
            0xD9 | 0xDA => return None,
            // SOF0..SOF15, minus the three markers that share the range but
            // are not frame headers: DHT (C4), JPG (C8), DAC (CC).
            0xC0..=0xCF if marker != 0xC4 && marker != 0xC8 && marker != 0xCC => {
                let height = be_u16(bytes, pos + 5)?;
                let width = be_u16(bytes, pos + 7)?;
                return Some((u32::from(width), u32::from(height)));
            }
            _ => {
                let length = usize::from(be_u16(bytes, pos + 2)?);
                if length < 2 {
                    return None;
                }
                pos = pos.checked_add(2)?.checked_add(length)?;
            }
        }
    }
    None
}

/// The logical screen descriptor follows the 6-byte version signature.
fn gif_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    Some((u32::from(le_u16(bytes, 6)?), u32::from(le_u16(bytes, 8)?)))
}

/// BMP carries its size in the DIB header, whose layout depends on its own
/// declared length. The legacy 12-byte BITMAPCOREHEADER uses `u16` fields;
/// every later version uses `i32`, where a negative height means the rows are
/// stored top-down - the magnitude is still the pixel height.
fn bmp_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let dib_size = le_u32(bytes, 14)?;
    if dib_size == 12 {
        return Some((u32::from(le_u16(bytes, 18)?), u32::from(le_u16(bytes, 20)?)));
    }
    let width = le_u32(bytes, 18)? as i32;
    let height = le_u32(bytes, 22)? as i32;
    Some((width.unsigned_abs(), height.unsigned_abs()))
}

/// WebP wraps one of three bitstream chunks; each declares its size in its own
/// way.
///
/// # The extended (`VP8X`) form needs more than its canvas
///
/// A `VP8X` header declares a canvas size, but that canvas is **advisory**:
/// `oxideav-webp` sizes the decode from the inner `VP8 `/`VP8L` bitstream
/// header, and its container layer explicitly leaves cross-checking the two to
/// the caller. So a file can declare a 1x1 canvas in front of a
/// 16384x16384 lossless bitstream, sail through a canvas-only gate at four
/// bytes of budget, and then decode a gigabyte.
///
/// The gate therefore caps on the **larger** of the canvas and every bitstream
/// extent in the file, which is the only figure that bounds what a decoder can
/// actually be made to allocate.
fn webp_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let fourcc = bytes.get(12..16)?;
    let data = 20usize; // RIFF(4) size(4) WEBP(4) fourcc(4) chunksize(4)
    match fourcc {
        b"VP8 " => vp8_dimensions(bytes, data),
        b"VP8L" => vp8l_dimensions(bytes, data),
        b"VP8X" => {
            let (canvas_width, canvas_height) = vp8x_canvas(bytes, data)?;
            match largest_bitstream_extent(bytes) {
                Some((width, height)) => Some((canvas_width.max(width), canvas_height.max(height))),
                // No readable bitstream chunk: the canvas is all we have, and
                // a decode will fail on its own terms.
                None => Some((canvas_width, canvas_height)),
            }
        }
        _ => None,
    }
}

/// Lossy `VP8 `: 3-byte frame tag, 3-byte sync code, two 14-bit dimensions.
fn vp8_dimensions(bytes: &[u8], data: usize) -> Option<(u32, u32)> {
    if bytes.get(data + 3..data + 6)? != [0x9D, 0x01, 0x2A] {
        return None;
    }
    let width = le_u16(bytes, data + 6)? & 0x3FFF;
    let height = le_u16(bytes, data + 8)? & 0x3FFF;
    Some((u32::from(width), u32::from(height)))
}

/// Lossless `VP8L`: signature byte, then 14 bits of width-1 and height-1.
fn vp8l_dimensions(bytes: &[u8], data: usize) -> Option<(u32, u32)> {
    if *bytes.get(data)? != 0x2F {
        return None;
    }
    let bits = le_u32(bytes, data + 1)?;
    let width = (bits & 0x3FFF) + 1;
    let height = ((bits >> 14) & 0x3FFF) + 1;
    Some((width, height))
}

/// `VP8X` canvas: two 24-bit little-endian values, each stored minus one,
/// after the 4-byte feature flags.
fn vp8x_canvas(bytes: &[u8], data: usize) -> Option<(u32, u32)> {
    let w = bytes.get(data + 4..data + 7)?;
    let h = bytes.get(data + 7..data + 10)?;
    Some((
        u32::from_le_bytes([w[0], w[1], w[2], 0]) + 1,
        u32::from_le_bytes([h[0], h[1], h[2], 0]) + 1,
    ))
}

/// The largest `VP8 `/`VP8L` extent anywhere in the RIFF chunk list.
fn largest_bitstream_extent(bytes: &[u8]) -> Option<(u32, u32)> {
    let mut largest: Option<(u32, u32)> = None;
    walk_riff_chunks(bytes, 12, 0, &mut largest);
    largest
}

/// Maximum RIFF chunks visited per level, and how deep `ANMF` nesting is
/// followed. Both are bounds against a file built to make the walk itself the
/// denial of service.
const MAX_RIFF_CHUNKS: usize = 64;
const MAX_RIFF_DEPTH: u32 = 2;

/// Walk the chunk list from `pos`, widening `largest` with every bitstream
/// header found. Animated frames (`ANMF`) carry their own sub-chunks after a
/// 16-byte frame header, so those are descended into as well.
fn walk_riff_chunks(bytes: &[u8], mut pos: usize, depth: u32, largest: &mut Option<(u32, u32)>) {
    if depth > MAX_RIFF_DEPTH {
        return;
    }
    for _ in 0..MAX_RIFF_CHUNKS {
        let Some(fourcc) = bytes.get(pos..pos + 4) else {
            return;
        };
        let Some(size) = le_u32(bytes, pos + 4) else {
            return;
        };
        let payload = pos + 8;
        match fourcc {
            b"VP8 " => widen(largest, vp8_dimensions(bytes, payload)),
            b"VP8L" => widen(largest, vp8l_dimensions(bytes, payload)),
            b"ANMF" => walk_riff_chunks(bytes, payload + 16, depth + 1, largest),
            _ => {}
        }
        // Chunk payloads are padded to an even length.
        let size = size as usize;
        let Some(next) = size
            .checked_add(size & 1)
            .and_then(|padded| padded.checked_add(8))
            .and_then(|advance| pos.checked_add(advance))
        else {
            return;
        };
        if next <= pos || next >= bytes.len() {
            return;
        }
        pos = next;
    }
}

fn widen(largest: &mut Option<(u32, u32)>, found: Option<(u32, u32)>) {
    let Some((width, height)) = found else {
        return;
    };
    *largest = Some(match *largest {
        Some((w, h)) => (w.max(width), h.max(height)),
        None => (width, height),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1x1 red PNG, the same verified fixture the integration tests use.
    const RED_PNG_1X1: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90,
        0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x78, 0xDA, 0x63, 0xF8,
        0xCF, 0xC0, 0x00, 0x00, 0x03, 0x01, 0x01, 0x00, 0xF7, 0x03, 0x41, 0x43, 0x00, 0x00, 0x00,
        0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    #[test]
    fn detects_the_five_supported_formats() {
        assert_eq!(detect(RED_PNG_1X1), Some(InputFormat::Png));
        assert_eq!(detect(&[0xFF, 0xD8, 0xFF, 0xE0]), Some(InputFormat::Jpeg));
        assert_eq!(detect(b"GIF89a\x04\x00\x02\x00"), Some(InputFormat::Gif));
        assert_eq!(detect(b"BM\x00\x00\x00\x00"), Some(InputFormat::Bmp));
        let mut webp = Vec::from(*b"RIFF\x00\x00\x00\x00WEBP");
        webp.extend_from_slice(b"VP8L");
        assert_eq!(detect(&webp), Some(InputFormat::WebP));
    }

    #[test]
    fn unknown_bytes_detect_as_nothing() {
        assert_eq!(detect(&[0u8; 64]), None);
        assert_eq!(detect(b""), None);
        // RIFF that is not WEBP (a WAV) must not be claimed.
        assert_eq!(detect(b"RIFF\x00\x00\x00\x00WAVEfmt "), None);
    }

    #[test]
    fn heif_brands_are_recognised_but_never_claimed_as_supported() {
        let heic = b"\x00\x00\x00\x18ftypheic\x00\x00\x00\x00";
        assert!(looks_like_heif(heic));
        assert_eq!(detect(heic), None, "HEIC must not enter the allowlist");
        assert!(looks_like_heif(b"\x00\x00\x00\x18ftypmif1\x00\x00\x00\x00"));
        assert!(!looks_like_heif(RED_PNG_1X1));
        // An ISO-BMFF file that is not HEIF (an MP4) must not be claimed.
        assert!(!looks_like_heif(
            b"\x00\x00\x00\x18ftypisom\x00\x00\x00\x00"
        ));
    }

    #[test]
    fn png_dimensions_come_from_ihdr() {
        assert_eq!(
            header_dimensions(InputFormat::Png, RED_PNG_1X1).expect("dims"),
            (1, 1)
        );
    }

    #[test]
    fn gif_dimensions_come_from_the_logical_screen_descriptor() {
        // 87a header declaring 4x2.
        let gif = b"GIF87a\x04\x00\x02\x00\x00\x00\x00";
        assert_eq!(
            header_dimensions(InputFormat::Gif, gif).expect("dims"),
            (4, 2)
        );
    }

    #[test]
    fn bmp_reads_both_header_generations_and_top_down_rows() {
        // BITMAPINFOHEADER (40), 4x2.
        let mut bmp = Vec::from(*b"BM");
        bmp.extend_from_slice(&[0u8; 12]);
        bmp.extend_from_slice(&40u32.to_le_bytes());
        bmp.extend_from_slice(&4i32.to_le_bytes());
        bmp.extend_from_slice(&2i32.to_le_bytes());
        assert_eq!(
            header_dimensions(InputFormat::Bmp, &bmp).expect("dims"),
            (4, 2)
        );

        // A negative height means top-down storage, not a negative size.
        let mut top_down = bmp.clone();
        top_down[22..26].copy_from_slice(&(-2i32).to_le_bytes());
        assert_eq!(
            header_dimensions(InputFormat::Bmp, &top_down).expect("dims"),
            (4, 2)
        );

        // BITMAPCOREHEADER (12) uses u16 fields at different offsets.
        let mut core = Vec::from(*b"BM");
        core.extend_from_slice(&[0u8; 12]);
        core.extend_from_slice(&12u32.to_le_bytes());
        core.extend_from_slice(&7u16.to_le_bytes());
        core.extend_from_slice(&3u16.to_le_bytes());
        assert_eq!(
            header_dimensions(InputFormat::Bmp, &core).expect("dims"),
            (7, 3)
        );
    }

    #[test]
    fn truncated_and_zeroed_headers_never_panic_and_never_pass_the_gate() {
        let config = ImageConfig::default();
        for format in [
            InputFormat::Png,
            InputFormat::Jpeg,
            InputFormat::WebP,
            InputFormat::Gif,
            InputFormat::Bmp,
        ] {
            for len in 0..64usize {
                let truncated = vec![0u8; len];
                // A short or zeroed buffer must never panic. Most parsers
                // run out of bytes and error; a few (GIF, BMP) have their
                // size fields inside the bytes we do have and legitimately
                // read 0x0 out of them. Either way nothing reaches a decoder:
                // zero dimensions are refused by the limit gate.
                match header_dimensions(format, &truncated) {
                    Err(_) => {}
                    Ok((width, height)) => {
                        assert_eq!(
                            (width, height),
                            (0, 0),
                            "{format:?} read real dimensions out of a zeroed buffer"
                        );
                        assert!(
                            enforce_limits(width, height, &config).is_err(),
                            "zero dimensions must be refused by the gate"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn jpeg_walks_segments_to_the_first_start_of_frame() {
        // SOI, an APP0 segment of length 4, then SOF0 declaring 2x3.
        let jpeg = [
            0xFF, 0xD8, // SOI
            0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00, // APP0, length 4
            0xFF, 0xC0, 0x00, 0x11, 0x08, // SOF0, length 17, precision 8
            0x00, 0x03, // height 3
            0x00, 0x02, // width 2
        ];
        assert_eq!(
            header_dimensions(InputFormat::Jpeg, &jpeg).expect("dims"),
            (2, 3)
        );
    }

    #[test]
    fn jpeg_does_not_mistake_a_huffman_table_for_a_frame_header() {
        // DHT (0xC4) sits inside the 0xC0..=0xCF range but is not an SOF.
        let jpeg = [
            0xFF, 0xD8, // SOI
            0xFF, 0xC4, 0x00, 0x04, 0x00, 0x00, // DHT, length 4
            0xFF, 0xC2, 0x00, 0x11, 0x08, // SOF2 (progressive), length 17
            0x00, 0x05, // height 5
            0x00, 0x09, // width 9
        ];
        assert_eq!(
            header_dimensions(InputFormat::Jpeg, &jpeg).expect("dims"),
            (9, 5)
        );
    }

    #[test]
    fn jpeg_scan_start_without_a_frame_header_is_an_error() {
        let jpeg = [0xFF, 0xD8, 0xFF, 0xDA, 0x00, 0x02];
        assert!(header_dimensions(InputFormat::Jpeg, &jpeg).is_err());
    }

    #[test]
    fn webp_reads_all_three_bitstream_chunks() {
        // VP8L: 14-bit width-1 and height-1 packed little-endian.
        let mut lossless = Vec::from(*b"RIFF\x00\x00\x00\x00WEBPVP8L\x00\x00\x00\x00");
        lossless.push(0x2F);
        let bits: u32 = (3 - 1) | ((2 - 1) << 14);
        lossless.extend_from_slice(&bits.to_le_bytes());
        assert_eq!(
            header_dimensions(InputFormat::WebP, &lossless).expect("dims"),
            (3, 2)
        );

        // VP8X: two 24-bit canvas dimensions, each stored minus one.
        let mut extended = Vec::from(*b"RIFF\x00\x00\x00\x00WEBPVP8X\x00\x00\x00\x00");
        extended.extend_from_slice(&[0u8; 4]); // feature flags
        extended.extend_from_slice(&[9, 0, 0]); // width - 1
        extended.extend_from_slice(&[4, 0, 0]); // height - 1
        assert_eq!(
            header_dimensions(InputFormat::WebP, &extended).expect("dims"),
            (10, 5)
        );

        // VP8 lossy: sync code then two 14-bit dimensions.
        let mut lossy = Vec::from(*b"RIFF\x00\x00\x00\x00WEBPVP8 \x00\x00\x00\x00");
        lossy.extend_from_slice(&[0, 0, 0]); // frame tag
        lossy.extend_from_slice(&[0x9D, 0x01, 0x2A]); // sync code
        lossy.extend_from_slice(&6u16.to_le_bytes());
        lossy.extend_from_slice(&8u16.to_le_bytes());
        assert_eq!(
            header_dimensions(InputFormat::WebP, &lossy).expect("dims"),
            (6, 8)
        );
    }

    /// Build an extended WebP: a `VP8X` canvas header followed by a `VP8L`
    /// bitstream of independent dimensions.
    fn vp8x_over_vp8l(canvas: (u32, u32), bitstream: (u32, u32)) -> Vec<u8> {
        let mut file = Vec::from(*b"RIFF\x00\x00\x00\x00WEBP");
        // VP8X chunk: fourcc, size 10, flags(4) + width-1(3) + height-1(3).
        file.extend_from_slice(b"VP8X");
        file.extend_from_slice(&10u32.to_le_bytes());
        file.extend_from_slice(&[0u8; 4]);
        file.extend_from_slice(&(canvas.0 - 1).to_le_bytes()[..3]);
        file.extend_from_slice(&(canvas.1 - 1).to_le_bytes()[..3]);
        // VP8L chunk: fourcc, size 5, signature + packed 14-bit dimensions.
        file.extend_from_slice(b"VP8L");
        file.extend_from_slice(&5u32.to_le_bytes());
        file.push(0x2F);
        let bits: u32 = (bitstream.0 - 1) | ((bitstream.1 - 1) << 14);
        file.extend_from_slice(&bits.to_le_bytes());
        file
    }

    #[test]
    fn a_small_vp8x_canvas_cannot_hide_a_large_bitstream() {
        // The attack: declare a 1x1 canvas so a canvas-only gate budgets four
        // bytes, then hand the decoder a 16384x16384 lossless bitstream.
        // oxideav-webp sizes its decode from the inner chunk, so the gate has
        // to cap on the larger of the two.
        let file = vp8x_over_vp8l((1, 1), (16_384, 16_384));
        assert_eq!(
            header_dimensions(InputFormat::WebP, &file).expect("dims"),
            (16_384, 16_384),
            "the bitstream extent must win over a smaller canvas"
        );

        let config = ImageConfig {
            max_dimension: 4096,
            ..ImageConfig::default()
        };
        let err = guard(&file, &config).expect_err("the gate must refuse it");
        assert!(err.to_string().contains("limit"), "got: {err}");
    }

    #[test]
    fn a_large_vp8x_canvas_still_wins_over_a_small_bitstream() {
        // The mirror case: a huge canvas around a tiny bitstream must not be
        // shrunk by the new maximum.
        let file = vp8x_over_vp8l((8_000, 6_000), (2, 2));
        assert_eq!(
            header_dimensions(InputFormat::WebP, &file).expect("dims"),
            (8_000, 6_000)
        );
    }

    #[test]
    fn vp8x_falls_back_to_the_canvas_when_no_bitstream_is_readable() {
        let mut file = Vec::from(*b"RIFF\x00\x00\x00\x00WEBPVP8X");
        file.extend_from_slice(&10u32.to_le_bytes());
        file.extend_from_slice(&[0u8; 4]);
        file.extend_from_slice(&[9, 0, 0]);
        file.extend_from_slice(&[4, 0, 0]);
        assert_eq!(
            header_dimensions(InputFormat::WebP, &file).expect("dims"),
            (10, 5)
        );
    }

    #[test]
    fn the_riff_walk_terminates_on_hostile_chunk_sizes() {
        // A zero-size chunk advances by the 8-byte header, so the walk still
        // progresses; a size that overflows the buffer ends it. Neither may
        // spin or panic.
        let mut zero_sized = Vec::from(*b"RIFF\x00\x00\x00\x00WEBPVP8X");
        zero_sized.extend_from_slice(&10u32.to_le_bytes());
        zero_sized.extend_from_slice(&[0u8; 4]);
        zero_sized.extend_from_slice(&[0, 0, 0]);
        zero_sized.extend_from_slice(&[0, 0, 0]);
        for _ in 0..8 {
            zero_sized.extend_from_slice(b"JUNK");
            zero_sized.extend_from_slice(&0u32.to_le_bytes());
        }
        let _ = header_dimensions(InputFormat::WebP, &zero_sized);

        let mut huge = Vec::from(*b"RIFF\x00\x00\x00\x00WEBPVP8X");
        huge.extend_from_slice(&u32::MAX.to_le_bytes());
        huge.extend_from_slice(&[0u8; 4]);
        huge.extend_from_slice(&[9, 0, 0]);
        huge.extend_from_slice(&[4, 0, 0]);
        assert_eq!(
            header_dimensions(InputFormat::WebP, &huge).expect("dims"),
            (10, 5),
            "an overflowing chunk size ends the walk, leaving the canvas"
        );
    }

    #[test]
    fn limits_reject_oversized_dimensions_and_allocations() {
        let config = ImageConfig {
            max_dimension: 100,
            max_alloc_bytes: 1_000_000,
            ..ImageConfig::default()
        };
        assert!(enforce_limits(100, 100, &config).is_ok());

        let too_wide = enforce_limits(101, 10, &config).expect_err("width cap");
        assert!(too_wide.to_string().contains("limit"));

        let too_tall = enforce_limits(10, 101, &config).expect_err("height cap");
        assert!(too_tall.to_string().contains("limit"));

        // Within the dimension cap but over the byte budget: 100*100*4 = 40_000.
        let tight = ImageConfig {
            max_dimension: 100,
            max_alloc_bytes: 39_999,
            ..ImageConfig::default()
        };
        let too_big = enforce_limits(100, 100, &tight).expect_err("alloc cap");
        assert!(too_big.to_string().contains("limit"));
    }

    #[test]
    fn limits_do_not_overflow_on_adversarial_dimensions() {
        let config = ImageConfig {
            max_dimension: u32::MAX,
            max_alloc_bytes: u64::MAX,
            ..ImageConfig::default()
        };
        // u32::MAX * u32::MAX * 4 overflows u64 arithmetic done naively; the
        // saturating path must still produce a decision, not a panic.
        assert!(enforce_limits(u32::MAX, u32::MAX, &config).is_ok());

        let capped = ImageConfig {
            max_dimension: u32::MAX,
            max_alloc_bytes: 1024,
            ..ImageConfig::default()
        };
        assert!(enforce_limits(u32::MAX, u32::MAX, &capped).is_err());
    }

    #[test]
    fn zero_dimensions_are_rejected() {
        let config = ImageConfig::default();
        assert!(enforce_limits(0, 10, &config).is_err());
        assert!(enforce_limits(10, 0, &config).is_err());
    }

    #[test]
    fn guard_rejects_empty_input_and_passes_unknown_formats_through() {
        let config = ImageConfig::default();
        assert!(guard(b"", &config).is_err());
        assert_eq!(guard(&[0u8; 64], &config).expect("unknown ok"), None);
        assert_eq!(
            guard(RED_PNG_1X1, &config).expect("png ok"),
            Some(InputFormat::Png)
        );
    }

    #[test]
    fn guard_applies_the_caps_to_recognised_input() {
        let config = ImageConfig {
            max_dimension: 0,
            ..ImageConfig::default()
        };
        let err = guard(RED_PNG_1X1, &config).expect_err("1x1 exceeds a zero cap");
        assert!(err.to_string().contains("limit"));
    }
}
