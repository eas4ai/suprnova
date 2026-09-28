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
        // WebP reports its own error, because "I could not finish looking"
        // has to be distinguishable from "this header is malformed" - the
        // first one has to fail closed.
        InputFormat::WebP => return webp_dimensions(bytes),
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

/// WebP declares its size in up to three different places, and the gate has to
/// account for all of them.
///
/// # Why a canvas is not enough, and why the first chunk is not enough
///
/// The extended (`VP8X`) form declares a canvas, but that canvas is
/// **advisory**: `oxideav-webp` sizes the decode from the inner `VP8 `/`VP8L`
/// bitstream header and its container layer explicitly leaves cross-checking
/// the two to the caller. A 1x1 canvas in front of a 16384x16384 lossless
/// bitstream would otherwise pass at four bytes of budget and decode a
/// gigabyte.
///
/// Reading only the *first* chunk is no better. Upstream's
/// `decode_webp_image` tries `extract_lossless` first, and that searches for a
/// `VP8L` chunk **anywhere** in the container, whatever the shape. So a
/// simple-lossy file whose first chunk is a 16x16 `VP8 ` and whose second is a
/// 16384x16384 `VP8L` decodes at the larger size - upstream prefers the
/// trailing `VP8L` over the leading `VP8 `.
///
/// So: walk every container, cap on the maximum over the canvas and every
/// bitstream extent at every level, and **fail closed** when the walk cannot
/// finish. A gate that cannot see the whole file must not report a number.
fn webp_dimensions(bytes: &[u8]) -> Result<(u32, u32), FrameworkError> {
    // The canvas only exists in the extended form, and only ever raises the
    // figure - it never licenses a smaller one. Read it within the VP8X
    // chunk's own declared payload, the same bound the walk applies: a
    // zero-length VP8X would otherwise have its "canvas" read out of the chunk
    // that follows, and since the canvas only ever raises the figure that
    // shows up as a false refusal of a file upstream decodes fine.
    let canvas = match bytes.get(12..16) {
        Some(b"VP8X") => le_u32(bytes, 16).and_then(|size| {
            let payload = 20usize;
            let end = payload.saturating_add(size as usize).min(bytes.len());
            vp8x_canvas(bytes.get(..end).unwrap_or(bytes), payload)
        }),
        _ => None,
    };

    let mut walk = Walk::default();
    walk_riff_chunks(bytes, 12, 0, &mut walk);

    if walk.gave_up {
        // Refusing here is the whole point: "I stopped early" and "there was
        // nothing to find" must never produce the same answer, because a file
        // can be built to make the first look like the second.
        // Deliberately does NOT say "configured": no environment variable
        // governs this bound, and an operator who reads "configured" will
        // raise IMAGE_MAX_ALLOC_BYTES, see no change, and be stuck.
        return Err(FrameworkError::param(format!(
            "image is too structurally complex to inspect: this WebP nests deeper or carries \
             more than {MAX_RIFF_CHUNKS} container chunks per level, so its true decoded size \
             cannot be bounded and it is refused. This is a fixed safety bound, not a \
             configurable limit - see the images chapter."
        )));
    }

    match (walk.largest, canvas) {
        (Some((width, height)), Some((canvas_width, canvas_height))) => {
            Ok((width.max(canvas_width), height.max(canvas_height)))
        }
        (Some(extent), None) => Ok(extent),
        // No bitstream chunk anywhere. Upstream cannot decode this either, so
        // refusing loses nothing and closes the hole where a bare `VP8X`
        // canvas stood in for a bitstream the walk never reached.
        (None, _) => Err(FrameworkError::param(
            "image header is malformed: this WebP carries no readable VP8 or VP8L bitstream",
        )),
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

/// How many RIFF chunks the walk will visit per level, and how far it follows
/// `ANMF` nesting.
///
/// Generous enough for a real animation - upstream's own parser has no chunk
/// cap at all, so anything short of this is ordinary content - while still
/// bounding a file built to make the walk itself the denial of service.
/// Raising it is not what makes the gate safe; failing closed past it is.
const MAX_RIFF_CHUNKS: usize = 4096;
const MAX_RIFF_DEPTH: u32 = 2;

/// What a walk of the chunk list found, and whether it got to the end.
///
/// `gave_up` is deliberately a field rather than an `Option` sentinel: the
/// previous version returned `Option<(u32, u32)>`, which made "no bitstream
/// present" and "I stopped looking" the same value, and that conflation was
/// the bypass. Keeping the two apart in the type is what stops it coming back.
#[derive(Default)]
struct Walk {
    /// Largest bitstream extent seen so far, if any.
    largest: Option<(u32, u32)>,
    /// True when the walk stopped at one of its own bounds rather than at the
    /// end of the data, so nothing can be concluded about what lies beyond.
    gave_up: bool,
}

impl Walk {
    fn widen(&mut self, found: Option<(u32, u32)>) {
        let Some((width, height)) = found else {
            return;
        };
        self.largest = Some(match self.largest {
            Some((w, h)) => (w.max(width), h.max(height)),
            None => (width, height),
        });
    }
}

/// Walk the chunk list from `pos`, widening `walk` with every bitstream header
/// found at any position.
///
/// Animated frames (`ANMF`) carry their own sub-chunks after a 16-byte frame
/// header, so those are descended into, bounded to the frame's own payload.
fn walk_riff_chunks(bytes: &[u8], mut pos: usize, depth: u32, walk: &mut Walk) {
    if depth > MAX_RIFF_DEPTH {
        // Content below this point is unread, so the result is inconclusive.
        walk.gave_up = true;
        return;
    }
    for visited in 0.. {
        if visited >= MAX_RIFF_CHUNKS {
            walk.gave_up = true;
            return;
        }
        // Out of bytes for a chunk header: this is the end of the data, which
        // is a complete walk rather than an abandoned one.
        let Some(fourcc) = bytes.get(pos..pos + 4) else {
            return;
        };
        let Some(size) = le_u32(bytes, pos + 4) else {
            return;
        };
        let payload = pos + 8;
        // Read each header within its own declared payload, exactly as
        // upstream's container parser slices it. Without this bound a
        // zero-length chunk's "header" would be read out of whatever follows
        // it, measuring something no decoder would ever see.
        let chunk_end = payload.saturating_add(size as usize).min(bytes.len());
        let chunk = bytes.get(..chunk_end).unwrap_or(bytes);
        match fourcc {
            b"VP8 " => walk.widen(vp8_dimensions(chunk, payload)),
            b"VP8L" => walk.widen(vp8l_dimensions(chunk, payload)),
            // Bound the descent to this frame's payload too, so a sub-walk
            // cannot run on into its siblings and spend their budget.
            b"ANMF" => walk_riff_chunks(chunk, payload + 16, depth + 1, walk),
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
    fn webp_reads_both_bitstream_chunk_kinds() {
        // A simple-lossless container: the VP8L bitstream is the whole story.
        let lossless = webp(&[chunk(b"VP8L", &vp8l_payload(3, 2))]);
        assert_eq!(
            header_dimensions(InputFormat::WebP, &lossless).expect("dims"),
            (3, 2)
        );

        // A simple-lossy container: the VP8 frame header carries it.
        let lossy = webp(&[chunk(b"VP8 ", &vp8_payload(6, 8))]);
        assert_eq!(
            header_dimensions(InputFormat::WebP, &lossy).expect("dims"),
            (6, 8)
        );

        // An extended container with a matching canvas and bitstream.
        let extended = webp(&[
            chunk(b"VP8X", &vp8x_payload(10, 5)),
            chunk(b"VP8L", &vp8l_payload(10, 5)),
        ]);
        assert_eq!(
            header_dimensions(InputFormat::WebP, &extended).expect("dims"),
            (10, 5)
        );
    }

    /// A RIFF chunk: fourcc, little-endian size, payload, even-length padding.
    fn chunk(fourcc: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::from(&fourcc[..]);
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(payload);
        if payload.len() % 2 == 1 {
            out.push(0);
        }
        out
    }

    /// A `VP8L` payload declaring `width x height`.
    fn vp8l_payload(width: u32, height: u32) -> Vec<u8> {
        let mut payload = vec![0x2Fu8];
        let bits: u32 = (width - 1) | ((height - 1) << 14);
        payload.extend_from_slice(&bits.to_le_bytes());
        payload
    }

    /// A `VP8 ` payload declaring `width x height`.
    fn vp8_payload(width: u16, height: u16) -> Vec<u8> {
        let mut payload = vec![0u8, 0, 0, 0x9D, 0x01, 0x2A];
        payload.extend_from_slice(&(width & 0x3FFF).to_le_bytes());
        payload.extend_from_slice(&(height & 0x3FFF).to_le_bytes());
        payload
    }

    /// A `VP8X` payload declaring a canvas of `width x height`.
    fn vp8x_payload(width: u32, height: u32) -> Vec<u8> {
        let mut payload = vec![0u8; 4]; // feature flags
        payload.extend_from_slice(&(width - 1).to_le_bytes()[..3]);
        payload.extend_from_slice(&(height - 1).to_le_bytes()[..3]);
        payload
    }

    /// Wrap chunks in a RIFF/WEBP container.
    fn webp(chunks: &[Vec<u8>]) -> Vec<u8> {
        let body: Vec<u8> = chunks.concat();
        let mut file = Vec::from(*b"RIFF");
        file.extend_from_slice(&((body.len() + 4) as u32).to_le_bytes());
        file.extend_from_slice(b"WEBP");
        file.extend_from_slice(&body);
        file
    }

    fn tight_config() -> ImageConfig {
        ImageConfig {
            max_dimension: 4096,
            ..ImageConfig::default()
        }
    }

    #[test]
    fn a_small_vp8x_canvas_cannot_hide_a_large_bitstream() {
        // Bypass shape: declare a 1x1 canvas so a canvas-only gate budgets
        // four bytes, then hand the decoder a 16384x16384 lossless bitstream.
        let file = webp(&[
            chunk(b"VP8X", &vp8x_payload(1, 1)),
            chunk(b"VP8L", &vp8l_payload(16_384, 16_384)),
        ]);
        assert_eq!(
            header_dimensions(InputFormat::WebP, &file).expect("dims"),
            (16_384, 16_384),
            "the bitstream extent must win over a smaller canvas"
        );
        let err = guard(&file, &tight_config()).expect_err("the gate must refuse it");
        assert!(err.to_string().contains("limit"), "got: {err}");
    }

    #[test]
    fn a_trailing_vp8l_behind_a_small_leading_vp8_is_measured() {
        // BYPASS A, the one the walk used to miss entirely: a simple-lossy
        // container whose FIRST chunk is a small `VP8 ` and whose second is a
        // huge `VP8L`. Upstream's decode tries extract_lossless first, and
        // that searches for VP8L anywhere in the container - so this file
        // really does decode at the larger size. Dispatching on the first
        // chunk alone reported 16x16 and let it through.
        let file = webp(&[
            chunk(b"VP8 ", &vp8_payload(16, 16)),
            chunk(b"VP8L", &vp8l_payload(16_384, 16_384)),
        ]);
        assert_eq!(
            header_dimensions(InputFormat::WebP, &file).expect("dims"),
            (16_384, 16_384),
            "a trailing VP8L must be seen even behind a leading VP8"
        );
        let err = guard(&file, &tight_config()).expect_err("the gate must refuse it");
        assert!(err.to_string().contains("limit"), "got: {err}");
    }

    #[test]
    fn filler_chunks_cannot_push_a_bitstream_past_the_walk() {
        // BYPASS B: filler chunks ahead of the real bitstream used to exhaust
        // the walk's own cap, after which it fell back to the canvas and
        // reported 1x1. The reviewer's exact repro was 63 fillers; the walk is
        // wider now, so that shape is measured correctly...
        let mut chunks = vec![chunk(b"VP8X", &vp8x_payload(1, 1))];
        for _ in 0..63 {
            chunks.push(chunk(b"JUNK", &[]));
        }
        chunks.push(chunk(b"VP8L", &vp8l_payload(16_384, 16_384)));
        let file = webp(&chunks);
        assert_eq!(
            header_dimensions(InputFormat::WebP, &file).expect("dims"),
            (16_384, 16_384)
        );
        assert!(guard(&file, &tight_config()).is_err());

        // ...and past the cap the answer is a refusal, not a fallback. This is
        // the property that matters: a wider cap alone would still be
        // bypassable at cap+1.
        let mut chunks = vec![chunk(b"VP8X", &vp8x_payload(1, 1))];
        for _ in 0..MAX_RIFF_CHUNKS {
            chunks.push(chunk(b"JUNK", &[]));
        }
        chunks.push(chunk(b"VP8L", &vp8l_payload(16_384, 16_384)));
        let file = webp(&chunks);
        let err = header_dimensions(InputFormat::WebP, &file)
            .expect_err("an unfinishable walk must refuse, never fall back");
        assert!(
            err.to_string().contains("structurally complex"),
            "got: {err}"
        );
        assert!(
            !err.to_string().contains("configured"),
            "no env var governs this bound, so the message must not imply one: {err}"
        );
        assert!(guard(&file, &ImageConfig::default()).is_err());
    }

    #[test]
    fn an_animation_with_more_frames_than_the_cap_is_refused() {
        // The same fail-closed rule for ANMF: measuring only the first N
        // frames of an animation whose later frames are larger would be the
        // bypass wearing a different hat.
        let frame = |width: u32, height: u32| {
            let mut payload = vec![0u8; 16]; // ANMF frame header
            payload.extend_from_slice(&chunk(b"VP8L", &vp8l_payload(width, height)));
            chunk(b"ANMF", &payload)
        };
        let mut chunks = vec![chunk(b"VP8X", &vp8x_payload(4, 4))];
        for _ in 0..MAX_RIFF_CHUNKS {
            chunks.push(frame(4, 4));
        }
        chunks.push(frame(16_384, 16_384));
        let file = webp(&chunks);
        let err = header_dimensions(InputFormat::WebP, &file)
            .expect_err("more frames than the cap must refuse");
        assert!(
            err.to_string().contains("structurally complex"),
            "got: {err}"
        );

        // A modest animation is still measured, and sees inside its frames.
        let small = webp(&[
            chunk(b"VP8X", &vp8x_payload(4, 4)),
            frame(4, 4),
            frame(64, 32),
        ]);
        assert_eq!(
            header_dimensions(InputFormat::WebP, &small).expect("dims"),
            (64, 32),
            "the largest frame's bitstream sets the figure"
        );
    }

    #[test]
    fn a_zero_length_vp8x_is_not_spuriously_refused() {
        // The canvas read used absolute offsets, so a zero-length VP8X read
        // six bytes of the FOLLOWING chunk as its canvas and reported a
        // nonsense extent - refusing a file upstream decodes fine at 4x4.
        // Canvas only participates via `.max()`, so this could never
        // under-measure; it was a false refusal, not a bypass.
        let file = webp(&[chunk(b"VP8X", &[]), chunk(b"VP8L", &vp8l_payload(4, 4))]);
        assert_eq!(
            header_dimensions(InputFormat::WebP, &file).expect("must not be refused"),
            (4, 4),
            "the bitstream is the only real measurement here"
        );
        assert!(guard(&file, &ImageConfig::default()).is_ok());
    }

    #[test]
    fn a_header_is_never_read_out_of_the_chunk_that_follows_it() {
        // A zero-length VP8L whose "payload" would be the next chunk's bytes.
        // Upstream slices by the declared size and finds nothing decodable, so
        // measuring those trailing bytes would report a size no decoder ever
        // produces - and with nothing else found, the file is refused.
        let file = webp(&[
            chunk(b"VP8L", &[]),
            chunk(b"JUNK", &vp8l_payload(16_384, 16_384)),
        ]);
        assert!(
            header_dimensions(InputFormat::WebP, &file).is_err(),
            "a zero-length chunk must not borrow the next chunk's bytes"
        );
    }

    #[test]
    fn a_large_vp8x_canvas_still_wins_over_a_small_bitstream() {
        // The mirror case: a huge canvas around a tiny bitstream must not be
        // shrunk by taking the maximum.
        let file = webp(&[
            chunk(b"VP8X", &vp8x_payload(8_000, 6_000)),
            chunk(b"VP8L", &vp8l_payload(2, 2)),
        ]);
        assert_eq!(
            header_dimensions(InputFormat::WebP, &file).expect("dims"),
            (8_000, 6_000)
        );
    }

    #[test]
    fn a_webp_with_no_bitstream_is_refused_rather_than_measured() {
        // A bare VP8X used to report its canvas. Upstream cannot decode this
        // either, so refusing loses nothing and removes the resting place the
        // exhausted walk used to fall back to.
        let file = webp(&[chunk(b"VP8X", &vp8x_payload(10, 5))]);
        assert!(header_dimensions(InputFormat::WebP, &file).is_err());
        assert!(guard(&file, &ImageConfig::default()).is_err());
    }

    #[test]
    fn the_riff_walk_terminates_on_hostile_chunk_sizes() {
        // A chunk size that runs past the buffer ends the walk at the data,
        // not at a self-imposed bound - so it is a complete walk with nothing
        // found, which is a refusal rather than a hang.
        let mut huge = Vec::from(*b"RIFF\x00\x00\x00\x00WEBPVP8X");
        huge.extend_from_slice(&u32::MAX.to_le_bytes());
        huge.extend_from_slice(&vp8x_payload(10, 5));
        assert!(header_dimensions(InputFormat::WebP, &huge).is_err());

        // A long run of zero-sized chunks advances by the 8-byte header each
        // time, so the walk progresses and terminates.
        let mut chunks = vec![chunk(b"VP8L", &vp8l_payload(4, 4))];
        for _ in 0..32 {
            chunks.push(chunk(b"JUNK", &[]));
        }
        assert_eq!(
            header_dimensions(InputFormat::WebP, &webp(&chunks)).expect("dims"),
            (4, 4)
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
