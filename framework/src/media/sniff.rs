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

/// The next JPEG marker at or after `pos`, read the way oxideav-mjpeg's
/// `MarkerWalker::next_marker` reads it: bytes that are not 0xFF are skipped,
/// a run of 0xFF fill bytes collapses, and a stuffed 0xFF00 is no marker.
/// Returns the marker and the position just past it.
fn jpeg_next_marker(bytes: &[u8], mut pos: usize) -> Option<(u8, usize)> {
    while pos < bytes.len() {
        if bytes[pos] != 0xFF {
            pos += 1;
            continue;
        }
        while bytes.get(pos) == Some(&0xFF) {
            pos += 1;
        }
        let marker = *bytes.get(pos)?;
        pos += 1;
        if marker != 0x00 {
            return Some((marker, pos));
        }
    }
    None
}

/// The length-prefixed payload at `pos` and the position after it, read the
/// way `MarkerWalker::read_segment_payload` reads it.
fn jpeg_segment(bytes: &[u8], pos: usize) -> Option<(&[u8], usize)> {
    let length = usize::from(be_u16(bytes, pos)?);
    if length < 2 {
        return None;
    }
    let end = pos.checked_add(length)?;
    Some((bytes.get(pos + 2..end)?, end))
}

/// Read a JPEG's dimensions from the frame header its decoder uses.
fn jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let frame = match jpeg_layout(bytes)? {
        JpegLayout::Zune(zune) => zune.frame,
        JpegLayout::Lossless(frame) => frame,
    };
    Some((frame.width, frame.height))
}

/// Which decoder reads a JPEG, and what it reads before it decodes a pixel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum JpegLayout {
    /// zune-jpeg decodes every DCT coding, Huffman or arithmetic.
    Zune(ZuneJpeg),
    /// oxideav-mjpeg decodes lossless frames, which zune-jpeg reads but
    /// cannot decode. The frame is the one oxideav-mjpeg's own walk reaches.
    Lossless(JpegFrame),
}

/// Pick a JPEG's decoder and read its headers the way that decoder will.
///
/// zune-jpeg's walk decides: a lossless frame there goes to oxideav-mjpeg,
/// whose own walk must then reach a lossless frame too, so that the frame
/// costed and the frame decoded are always read by the same walk.
pub(crate) fn jpeg_layout(bytes: &[u8]) -> Option<JpegLayout> {
    let zune = jpeg_zune(bytes)?;
    if matches!(zune.frame.marker, 0xC3 | 0xCB) {
        let frame = jpeg_frame(bytes)?;
        return matches!(frame.marker, 0xC3 | 0xCB).then_some(JpegLayout::Lossless(frame));
    }
    Some(JpegLayout::Zune(zune))
}

/// The colour space zune-jpeg decodes a frame from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum JpegColour {
    Grey,
    YCbCr,
    Rgb,
    /// CMYK, YCCK, or a component count that matches no colour space.
    Other,
}

/// What zune-jpeg reads from a JPEG before it decodes a pixel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ZuneJpeg {
    pub(crate) frame: JpegFrame,
    pub(crate) colour: JpegColour,
    /// Bytes zune-jpeg copies out of APP1, APP2 and APP13 segments (Exif,
    /// XMP, ICC profile chunks, gain maps, MPF, IPTC), with the share of the
    /// lists that hold them.
    pub(crate) metadata: u64,
    /// Bytes zune-jpeg's Extended XMP reassembly reads before the first
    /// scan, at most.
    ///
    /// zune-jpeg keeps every Extended XMP segment it has met and, after
    /// each marker that follows, sorts them and compares each one with the
    /// first. Until a series completes, that re-reads every segment kept so
    /// far, so the work grows with the square of the segment count: a few
    /// megabytes of one-byte segments ask for billions of comparisons, far
    /// more than their bytes cost in [`Self::metadata`]. This counts the
    /// segments kept at every marker, as if none were ever dropped, at
    /// [`ZUNE_XMP_PASS_BYTES`] each.
    pub(crate) xmp_reads: u64,
}

/// Read a JPEG's headers the way zune-jpeg 0.5.16-rc2's
/// `decode_headers_internal` does, up to its first scan header.
///
/// zune-jpeg reads a byte at a time, takes a marker after `0xFF` (skipping
/// fill bytes and stuffed zeros), and reads every marker's segment by its
/// length, the restart markers and a second start-of-image included, where
/// oxideav-mjpeg takes those as standalone. A second frame header, a
/// hierarchical one, or the image ending before a scan are errors, and so
/// are segments that do not read. `None` wherever zune-jpeg errors.
pub(crate) fn jpeg_zune(bytes: &[u8]) -> Option<ZuneJpeg> {
    if bytes.get(..2)? != [0xFF, 0xD8] {
        return None;
    }
    let mut pos = 2;
    let mut last = 0u8;
    let mut frame: Option<JpegFrame> = None;
    let mut adobe: Option<u8> = None;
    let mut metadata = 0u64;
    let mut xmp_segments = 0u64;
    let mut xmp_reads = 0u64;
    loop {
        let mut marker = *bytes.get(pos)?;
        pos += 1;
        if (marker == 0xFF || marker == 0) && last == 0xFF {
            while marker == 0xFF || marker == 0 {
                last = marker;
                marker = *bytes.get(pos)?;
                pos += 1;
            }
        }
        if last == 0xFF {
            // End of image before a scan, or a hierarchical frame header.
            if matches!(marker, 0xD9 | 0xC5..=0xC7 | 0xCD..=0xCF) {
                return None;
            }
            let (body, next) = jpeg_segment(bytes, pos)?;
            pos = next;
            if marker == 0xE1 && tagged(body, EXTENDED_XMP) {
                xmp_segments += 1;
            }
            // zune-jpeg tries the reassembly after every marker it reads,
            // the start of scan included, over every segment it keeps.
            xmp_reads = xmp_reads.saturating_add(xmp_segments.saturating_mul(ZUNE_XMP_PASS_BYTES));
            match marker {
                0xC0..=0xC3 | 0xC9..=0xCB => {
                    if frame.is_some() {
                        return None;
                    }
                    frame = Some(zune_frame_header(marker, body)?);
                }
                0xDA => {
                    let mut frame = frame?;
                    let components = *body.first()?;
                    if components == 0 || body.len() < 1 + usize::from(components) * 2 + 3 {
                        return None;
                    }
                    frame.first_scan = components;
                    let colour = zune_colour(&frame, adobe);
                    let metadata = metadata.saturating_add(zune_metadata_after(bytes, pos));
                    return Some(ZuneJpeg {
                        frame,
                        colour,
                        metadata,
                        xmp_reads,
                    });
                }
                0xE1 | 0xE2 | 0xED => {
                    metadata = metadata.saturating_add(zune_metadata(marker, body)?);
                }
                0xEE if body.starts_with(b"Adobe") => {
                    let transform = *body.get(11)?;
                    if transform > 2 {
                        return None;
                    }
                    adobe = Some(transform);
                }
                // DRI and DNL carry exactly two bytes.
                0xDD | 0xDC if body.len() != 2 => return None,
                _ => {}
            }
        }
        last = marker;
    }
}

/// Parse and validate a frame header payload as zune-jpeg's
/// `parse_start_of_frame` does. A height of zero is kept: zune-jpeg reads
/// the height from a later DNL segment, and the header gate refuses it.
fn zune_frame_header(marker: u8, payload: &[u8]) -> Option<JpegFrame> {
    let precision = *payload.first()?;
    let precision_reads = match marker {
        0xC3 | 0xCB => (2..=16).contains(&precision),
        0xC1 | 0xC2 => precision == 8 || precision == 12,
        _ => precision == 8,
    };
    let width = be_u16(payload, 3)?;
    let components = *payload.get(5)?;
    if !precision_reads
        || width == 0
        || components == 0
        || components > 4
        || payload.len() != 6 + 3 * usize::from(components)
    {
        return None;
    }
    let mut sampling = [(0u8, 0u8); 4];
    let mut ids = [0u8; 4];
    for index in 0..usize::from(components) {
        let at = 6 + 3 * index;
        let factors = (payload[at + 1] >> 4, payload[at + 1] & 0x0F);
        // Sampling factors 1, 2 or 4; quantization tables 0 to 3.
        let valid = |factor: u8| matches!(factor, 1 | 2 | 4);
        if !valid(factors.0) || !valid(factors.1) || payload[at + 2] >= 4 {
            return None;
        }
        ids[index] = payload[at];
        sampling[index] = factors;
    }
    Some(JpegFrame {
        marker,
        precision,
        width: u32::from(width),
        height: u32::from(be_u16(payload, 1)?),
        components,
        sampling,
        ids,
        first_scan: 0,
    })
}

/// zune-jpeg's `resolve_input_colorspace`: an Adobe APP14 transform first,
/// then the component count, then components named `R`, `G`, `B`.
fn zune_colour(frame: &JpegFrame, adobe: Option<u8>) -> JpegColour {
    match (adobe, frame.components) {
        (Some(0), 3) => JpegColour::Rgb,
        // YCCK with three components reads as YCbCr.
        (Some(1 | 2), 3) => JpegColour::YCbCr,
        (Some(_), _) => JpegColour::Other,
        (None, 1) => JpegColour::Grey,
        (None, 3) if frame.ids[..3] == *b"RGB" => JpegColour::Rgb,
        (None, 3) => JpegColour::YCbCr,
        (None, _) => JpegColour::Other,
    }
}

/// The lists zune-jpeg pushes metadata onto, each entry's size: an ICC
/// chunk, a gain map, an extended XMP segment. They grow by doubling, so an
/// entry can hold twice its size.
const ZUNE_ICC_CHUNK: u64 = 32;
const ZUNE_GAIN_MAP: u64 = 24;
const ZUNE_EXTENDED_XMP: u64 = 56;

/// The four entries each list reserves on its first push.
pub(crate) const ZUNE_METADATA_LISTS: u64 =
    4 * (ZUNE_ICC_CHUNK + ZUNE_GAIN_MAP + ZUNE_EXTENDED_XMP);

/// What one reassembly pass reads of each Extended XMP segment zune-jpeg
/// keeps: the segment's list entry, and its 32-byte GUID, compared with the
/// first segment's.
pub(crate) const ZUNE_XMP_PASS_BYTES: u64 = ZUNE_EXTENDED_XMP + 32;

/// The namespace that opens an Extended XMP segment.
const EXTENDED_XMP: &[u8] = b"http://ns.adobe.com/xmp/extension/\0";

/// Whether a segment `body` opens with `prefix` and holds more after it, the
/// test zune-jpeg classifies its APP segments by.
fn tagged(body: &[u8], prefix: &[u8]) -> bool {
    body.len() > prefix.len() && body.starts_with(prefix)
}

/// What zune-jpeg copies out of one APP1, APP2 or APP13 segment, as its
/// `parse_app1`, `parse_app2` and `parse_app13` classify it. `None` where
/// zune-jpeg errors on the segment.
fn zune_metadata(marker: u8, body: &[u8]) -> Option<u64> {
    const EXIF: &[u8] = b"Exif\0\0";
    const XMP: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";
    const ICC: &[u8] = b"ICC_PROFILE\0";
    const GAIN_MAP: &[u8] = b"urn:iso:std:iso:ts:21496:-1\0";
    const MPF: &[u8] = b"MPF\0";
    const IPTC: &[u8] = b"Photoshop 3.0\0";
    let length = body.len() as u64;
    let after = |prefix: &[u8]| length - prefix.len() as u64;
    let tagged = |prefix: &[u8]| tagged(body, prefix);
    Some(match marker {
        0xE1 if tagged(EXIF) => after(EXIF),
        0xE1 if tagged(XMP) => after(XMP),
        0xE1 if tagged(EXTENDED_XMP) => {
            // A 40-byte header (GUID, total size, offset), then the data,
            // which is copied again when every segment is in.
            let data = after(EXTENDED_XMP).checked_sub(40)?;
            32 + 2 * data + 2 * ZUNE_EXTENDED_XMP
        }
        0xE2 if body.len() > ICC.len() + 2 && body.starts_with(ICC) => {
            let (sequence, count) = (body[ICC.len()], body[ICC.len() + 1]);
            if count == 0 || sequence == 0 || sequence > count {
                return None;
            }
            after(ICC) - 2 + 2 * ZUNE_ICC_CHUNK
        }
        0xE2 if tagged(GAIN_MAP) => match after(GAIN_MAP) {
            4 => 2 * ZUNE_GAIN_MAP,
            rest if rest > 4 => rest + 2 * ZUNE_GAIN_MAP,
            _ => 0,
        },
        0xE2 if tagged(MPF) => after(MPF),
        0xED if tagged(IPTC) => after(IPTC),
        _ => 0,
    })
}

/// What zune-jpeg can copy out of metadata segments after the first scan
/// header: a progressive or multi-scan image reads segments between its
/// scans. Every `0xFF 0xE1`, `0xFF 0xE2` or `0xFF 0xED` from `pos` on is
/// counted as a segment, aligned or not, so the count is never short of
/// what the decoder reads.
fn zune_metadata_after(bytes: &[u8], pos: usize) -> u64 {
    let mut total = 0u64;
    for at in pos..bytes.len().saturating_sub(3) {
        if bytes[at] != 0xFF || !matches!(bytes[at + 1], 0xE1 | 0xE2 | 0xED) {
            continue;
        }
        let Some(length) = be_u16(bytes, at + 2) else {
            continue;
        };
        let end = (at + 2 + usize::from(length)).min(bytes.len());
        if let Some(body) = bytes.get(at + 4..end) {
            total = total.saturating_add(zune_metadata(bytes[at + 1], body).unwrap_or(0));
        }
    }
    total
}

/// What a JPEG's frame header and first scan header declare: everything that
/// sizes the decoder's buffers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct JpegFrame {
    /// The Start-Of-Frame marker, which names the coding process.
    pub(crate) marker: u8,
    /// Bits per sample.
    pub(crate) precision: u8,
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// Components the frame declares.
    pub(crate) components: u8,
    /// (horizontal, vertical) sampling factors of the first four components.
    pub(crate) sampling: [(u8, u8); 4],
    /// Identifiers of the first four components. `R`, `G`, `B` mark samples
    /// stored as RGB rather than YCbCr.
    pub(crate) ids: [u8; 4],
    /// Components in the first scan. A first scan with every component is
    /// what lets a sequential decode skip the coefficient buffers.
    pub(crate) first_scan: u8,
}

/// Read the frame header and first scan header oxideav-mjpeg will decode.
///
/// The walk is the decoder's own, marker for marker: the same marker search,
/// the same markers taken as standalone (SOI, RSTn), the same segments read
/// by their length, and the same segments skipped when their length does not
/// read. A walk that differs would measure a frame the decoder never decodes,
/// so where the decoder errors (a second frame header, a hierarchical one, a
/// scan before the frame, the image ending first) this refuses too. The
/// frame header is validated as the decoder validates it.
pub(crate) fn jpeg_frame(bytes: &[u8]) -> Option<JpegFrame> {
    let mut frame: Option<JpegFrame> = None;
    let mut pos = 2;
    loop {
        let (marker, after) = jpeg_next_marker(bytes, pos)?;
        pos = after;
        match marker {
            // End of image before a scan, or a hierarchical frame header.
            0xD9 | 0xC5..=0xC7 | 0xCD..=0xCF => return None,
            // RST0-7 (0xD0 to 0xD7) and SOI (0xD8) carry no payload.
            0xD0..=0xD8 => {}
            // The frame headers the decoder takes: SOF0-3, SOF9-11.
            0xC0..=0xC3 | 0xC9..=0xCB => {
                if frame.is_some() {
                    return None;
                }
                let (payload, next) = jpeg_segment(bytes, pos)?;
                frame = Some(jpeg_frame_header(marker, payload)?);
                pos = next;
            }
            // The first scan header: the decoder decodes from here on.
            0xDA => {
                let mut frame = frame?;
                let (payload, _) = jpeg_segment(bytes, pos)?;
                let components = *payload.first()?;
                if payload.len() < 1 + usize::from(components) * 2 + 3 {
                    return None;
                }
                frame.first_scan = components;
                return Some(frame);
            }
            // Segments the decoder reads, and fails on when they do not read:
            // DHT, DAC, DQT, DNL, DRI, APPn, COM.
            0xC4 | 0xCC | 0xDB..=0xDD | 0xE0..=0xEF | 0xFE => {
                let (_, next) = jpeg_segment(bytes, pos)?;
                pos = next;
            }
            // Any other marker: the decoder skips its segment when the
            // length reads, and otherwise carries on from just past it.
            _ => {
                if let Some((_, next)) = jpeg_segment(bytes, pos) {
                    pos = next;
                }
            }
        }
    }
}

/// Parse and validate a frame header payload as oxideav-mjpeg's `parse_sof`
/// and `validate_sof` do: one to four components, sampling factors 1 to 4,
/// quantization tables 0 to 3.
fn jpeg_frame_header(marker: u8, payload: &[u8]) -> Option<JpegFrame> {
    let components = *payload.get(5)?;
    if !(1..=4).contains(&components) || payload.len() < 6 + 3 * usize::from(components) {
        return None;
    }
    let mut sampling = [(0u8, 0u8); 4];
    let mut ids = [0u8; 4];
    for index in 0..usize::from(components) {
        // Each component: identifier, sampling factors, quantization table.
        let at = 6 + 3 * index;
        let factors = (payload[at + 1] >> 4, payload[at + 1] & 0x0F);
        if !(1..=4).contains(&factors.0) || !(1..=4).contains(&factors.1) || payload[at + 2] >= 4 {
            return None;
        }
        ids[index] = payload[at];
        sampling[index] = factors;
    }
    Some(JpegFrame {
        marker,
        precision: payload[0],
        width: u32::from(be_u16(payload, 3)?),
        height: u32::from(be_u16(payload, 1)?),
        components,
        sampling,
        ids,
        first_scan: 0,
    })
}

/// The logical screen descriptor follows the 6-byte version signature.
fn gif_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    Some((u32::from(le_u16(bytes, 6)?), u32::from(le_u16(bytes, 8)?)))
}

/// A GIF's first image: where it sits on the logical screen, and where its
/// color table and LZW data are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GifFrame {
    pub(crate) left: u16,
    pub(crate) top: u16,
    pub(crate) width: u16,
    pub(crate) height: u16,
    /// Rows are stored in the four-pass interlaced order.
    pub(crate) interlaced: bool,
    /// The color table the frame uses, as (offset, entries): its own local
    /// table, or else the global one. `None` when there is neither.
    pub(crate) palette: Option<(usize, usize)>,
    /// The transparent index of the Graphic Control Extension that applies
    /// to the frame, if it sets one.
    pub(crate) transparent: Option<u8>,
    /// Offset of the LZW minimum code size; the data sub-blocks follow it.
    pub(crate) data: usize,
}

impl GifFrame {
    /// True when the frame lies inside a `width x height` logical screen.
    pub(crate) fn fits(self, width: u32, height: u32) -> bool {
        u32::from(self.left) + u32::from(self.width) <= width
            && u32::from(self.top) + u32::from(self.height) <= height
    }
}

/// Find a GIF's first image by walking its block structure, decoding nothing.
///
/// The gate measures the logical screen, but a frame is decoded at the size
/// its own Image Descriptor declares, so the first descriptor has to be read
/// as well. Extensions before it are stepped over by the GIF grammar: each
/// fixed-size first block at its required size (Graphic Control 4, Plain
/// Text 12, Application 11), then data sub-blocks. A Graphic Control
/// Extension applies to the next graphic block, so a Plain Text block in
/// between takes it. `None` when the stream ends, reaches its trailer, or
/// breaks that grammar before the first image.
pub(crate) fn gif_first_frame(bytes: &[u8]) -> Option<GifFrame> {
    // The signature and Logical Screen Descriptor take 13 bytes. A Global
    // Color Table of 3 * 2^(n + 1) bytes follows when the top bit of the
    // descriptor's packed field is set.
    let screen_flags = *bytes.get(10)?;
    let mut pos = 13usize;
    let mut palette = None;
    if screen_flags & 0x80 != 0 {
        let entries = 2usize << (screen_flags & 0x07);
        bytes.get(pos..pos + 3 * entries)?;
        palette = Some((pos, entries));
        pos += 3 * entries;
    }
    let mut transparent = None;
    loop {
        match *bytes.get(pos)? {
            // Image Descriptor: left, top, width, height, packed fields. A
            // Local Color Table follows when the packed field's top bit is
            // set, then the LZW minimum code size and the data sub-blocks.
            0x2C => {
                let flags = *bytes.get(pos + 9)?;
                let mut data = pos + 10;
                if flags & 0x80 != 0 {
                    let entries = 2usize << (flags & 0x07);
                    bytes.get(data..data + 3 * entries)?;
                    palette = Some((data, entries));
                    data += 3 * entries;
                }
                bytes.get(data)?;
                return Some(GifFrame {
                    left: le_u16(bytes, pos + 1)?,
                    top: le_u16(bytes, pos + 3)?,
                    width: le_u16(bytes, pos + 5)?,
                    height: le_u16(bytes, pos + 7)?,
                    interlaced: flags & 0x40 != 0,
                    palette,
                    transparent,
                    data,
                });
            }
            // Extension introducer, then a label byte.
            0x21 => {
                let label = *bytes.get(pos + 1)?;
                pos += 2;
                match label {
                    // Graphic Control: one 4-byte block (flags, delay,
                    // transparent index), then the terminator.
                    0xF9 => {
                        if *bytes.get(pos)? != 4 || *bytes.get(pos + 5)? != 0 {
                            return None;
                        }
                        transparent = (bytes[pos + 1] & 0x01 != 0).then_some(bytes[pos + 4]);
                        pos += 6;
                    }
                    // Plain Text opens with a 12-byte block and Application
                    // with an 11-byte one; data sub-blocks follow both.
                    0x01 | 0xFF => {
                        let fixed = if label == 0x01 { 12 } else { 11 };
                        if *bytes.get(pos)? != fixed {
                            return None;
                        }
                        pos = skip_sub_blocks(bytes, pos)?;
                        if label == 0x01 {
                            transparent = None;
                        }
                    }
                    // Comment: data sub-blocks only.
                    0xFE => pos = skip_sub_blocks(bytes, pos)?,
                    _ => return None,
                }
            }
            // The trailer, or a byte that introduces no block at all.
            _ => return None,
        }
    }
}

/// Skip a run of GIF data sub-blocks and the zero-length block that ends it,
/// returning the position after the terminator.
fn skip_sub_blocks(bytes: &[u8], mut pos: usize) -> Option<usize> {
    loop {
        let len = usize::from(*bytes.get(pos)?);
        pos += 1;
        if len == 0 {
            return Some(pos);
        }
        bytes.get(pos..pos + len)?;
        pos += len;
    }
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

/// What a BMP's DIB header declares beyond its size: the fields that pick the
/// decoder's buffers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BmpLayout {
    pub(crate) bits_per_pixel: u16,
    /// `biCompression`: 1 and 2 are the run-length encodings.
    pub(crate) compression: u32,
    /// Palette entries the header declares (`biClrUsed`, or the full table
    /// for its bit depth), before the decoder checks them against the data.
    pub(crate) palette_entries: u64,
    /// Bytes per palette entry on disk: 3 for BITMAPCOREHEADER, else 4.
    pub(crate) palette_entry_bytes: u64,
}

/// Read the DIB header fields [`BmpLayout`] names. The legacy 12-byte
/// BITMAPCOREHEADER has no compression or palette-count fields.
pub(crate) fn bmp_layout(bytes: &[u8]) -> Option<BmpLayout> {
    let dib_size = le_u32(bytes, 14)?;
    let (bits_per_pixel, compression, colors_used, palette_entry_bytes) = if dib_size == 12 {
        (le_u16(bytes, 24)?, 0, 0, 3)
    } else {
        (
            le_u16(bytes, 28)?,
            le_u32(bytes, 30)?,
            le_u32(bytes, 46)?,
            4,
        )
    };
    let palette_entries = match bits_per_pixel {
        1 | 4 | 8 if colors_used == 0 => 1u64 << bits_per_pixel,
        1 | 4 | 8 => u64::from(colors_used),
        _ => 0,
    };
    Some(BmpLayout {
        bits_per_pixel,
        compression,
        palette_entries,
        palette_entry_bytes,
    })
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
pub(crate) fn vp8_dimensions(bytes: &[u8], data: usize) -> Option<(u32, u32)> {
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

    /// A GIF89a head: a 16x16 logical screen with a four-entry Global Color
    /// Table.
    fn gif_head() -> Vec<u8> {
        let mut gif = Vec::from(*b"GIF89a");
        gif.extend_from_slice(&16u16.to_le_bytes());
        gif.extend_from_slice(&16u16.to_le_bytes());
        // Global Color Table present, 2^(1 + 1) = 4 entries.
        gif.extend_from_slice(&[0x81, 0, 0]);
        gif.extend_from_slice(&[0u8; 12]);
        gif
    }

    /// An Image Descriptor for `width x height` at `left, top`, with no
    /// Local Color Table, followed by an LZW minimum code size and three
    /// bytes of data in one sub-block.
    fn image_descriptor(left: u16, top: u16, width: u16, height: u16) -> Vec<u8> {
        let mut block = vec![0x2C];
        for value in [left, top, width, height] {
            block.extend_from_slice(&value.to_le_bytes());
        }
        block.push(0);
        block.extend_from_slice(&[2, 3, 0xAA, 0xBB, 0xCC, 0]);
        block
    }

    #[test]
    fn the_first_gif_frame_is_found_behind_every_extension_kind() {
        let mut gif = gif_head();
        // Graphic Control.
        gif.extend_from_slice(&[0x21, 0xF9, 4, 0, 10, 0, 0, 0]);
        // Comment, two sub-blocks.
        gif.extend_from_slice(&[0x21, 0xFE, 3, b'a', b'b', b'c', 1, b'd', 0]);
        // Application: an 11-byte block, then one data sub-block.
        gif.extend_from_slice(&[0x21, 0xFF, 11]);
        gif.extend_from_slice(b"NETSCAPE2.0");
        gif.extend_from_slice(&[3, 1, 0, 0, 0]);
        // Plain Text: a 12-byte block, then one data sub-block.
        gif.extend_from_slice(&[0x21, 0x01, 12]);
        gif.extend_from_slice(&[0u8; 12]);
        gif.extend_from_slice(&[2, b'h', b'i', 0]);
        let descriptor_at = gif.len();
        gif.extend_from_slice(&image_descriptor(2, 3, 8, 9));
        // A second, larger frame the walk must never report.
        gif.extend_from_slice(&image_descriptor(0, 0, 900, 900));

        let frame = gif_first_frame(&gif).expect("the first frame");
        assert_eq!(
            frame,
            GifFrame {
                left: 2,
                top: 3,
                width: 8,
                height: 9,
                interlaced: false,
                // The global table, right after the screen descriptor.
                palette: Some((13, 4)),
                transparent: None,
                data: descriptor_at + 10,
            }
        );
        assert!(frame.fits(16, 16));
        assert!(!frame.fits(9, 16), "2 + 8 overruns a 9-wide screen");
        assert!(!frame.fits(16, 11), "3 + 9 overruns an 11-high screen");
    }

    #[test]
    fn the_first_gif_frame_reports_its_palette_transparency_and_interlacing() {
        let mut gif = gif_head();
        // Graphic Control with the transparency flag set, index 1.
        gif.extend_from_slice(&[0x21, 0xF9, 4, 0x01, 10, 0, 1, 0]);
        let descriptor_at = gif.len();
        let mut descriptor = vec![0x2C];
        for value in [0u16, 0, 4, 4] {
            descriptor.extend_from_slice(&value.to_le_bytes());
        }
        // Local Color Table of 2^(0 + 1) = 2 entries, interlaced.
        descriptor.push(0x80 | 0x40);
        descriptor.extend_from_slice(&[0u8; 6]);
        descriptor.extend_from_slice(&[2, 2, 0xAA, 0xBB, 0]);
        gif.extend_from_slice(&descriptor);
        let frame = gif_first_frame(&gif).expect("the first frame");
        assert!(frame.interlaced);
        assert_eq!(frame.palette, Some((descriptor_at + 10, 2)));
        assert_eq!(frame.transparent, Some(1));
        assert_eq!(frame.data, descriptor_at + 16);

        // A Plain Text block between them takes the Graphic Control.
        let mut plain_text = gif_head();
        plain_text.extend_from_slice(&[0x21, 0xF9, 4, 0x01, 10, 0, 1, 0]);
        plain_text.extend_from_slice(&[0x21, 0x01, 12]);
        plain_text.extend_from_slice(&[0u8; 12]);
        plain_text.extend_from_slice(&[0]);
        plain_text.extend_from_slice(&image_descriptor(0, 0, 4, 4));
        assert_eq!(
            gif_first_frame(&plain_text).map(|frame| frame.transparent),
            Some(None)
        );

        // A local table that runs past the end of the stream.
        gif.truncate(descriptor_at + 13);
        assert_eq!(gif_first_frame(&gif), None);
    }

    #[test]
    fn a_gif_without_a_global_color_table_starts_its_blocks_at_byte_13() {
        let mut gif = Vec::from(*b"GIF87a");
        gif.extend_from_slice(&[4, 0, 2, 0, 0, 0, 0]);
        gif.extend_from_slice(&image_descriptor(0, 0, 4, 2));
        assert_eq!(
            gif_first_frame(&gif).map(|frame| (frame.width, frame.height)),
            Some((4, 2))
        );
    }

    #[test]
    fn a_gif_walk_refuses_what_the_decoder_refuses() {
        let with = |blocks: &[u8]| {
            let mut gif = gif_head();
            gif.extend_from_slice(blocks);
            gif.extend_from_slice(&image_descriptor(0, 0, 4, 4));
            gif
        };
        // The trailer before any image.
        assert_eq!(gif_first_frame(&with(&[0x3B])), None);
        // A byte that is no block introducer.
        assert_eq!(gif_first_frame(&with(&[0x00])), None);
        // An unknown extension label.
        assert_eq!(gif_first_frame(&with(&[0x21, 0x42, 0])), None);
        // A Graphic Control block of the wrong size, or without its
        // terminator.
        assert_eq!(
            gif_first_frame(&with(&[0x21, 0xF9, 5, 0, 0, 0, 0, 0, 0])),
            None
        );
        assert_eq!(
            gif_first_frame(&with(&[0x21, 0xF9, 4, 0, 0, 0, 0, 7])),
            None
        );
        // Application and Plain Text blocks of the wrong size.
        assert_eq!(gif_first_frame(&with(&[0x21, 0xFF, 10])), None);
        assert_eq!(gif_first_frame(&with(&[0x21, 0x01, 11])), None);
        // A sub-block that runs past the end of the data.
        let mut truncated = gif_head();
        truncated.extend_from_slice(&[0x21, 0xFE, 200, 1, 2, 3]);
        assert_eq!(gif_first_frame(&truncated), None);
        // A descriptor cut short.
        let mut short = gif_head();
        short.extend_from_slice(&[0x2C, 0, 0, 0, 0, 4]);
        assert_eq!(gif_first_frame(&short), None);
        // A Global Color Table longer than the data.
        assert_eq!(gif_first_frame(b"GIF89a\x10\x00\x10\x00\x87\x00\x00"), None);
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

    /// A one-component frame header for `width x height` behind `marker`.
    fn grey_frame_header(marker: u8, width: u16, height: u16) -> Vec<u8> {
        let mut segment = vec![0xFF, marker, 0x00, 0x0B, 8];
        segment.extend_from_slice(&height.to_be_bytes());
        segment.extend_from_slice(&width.to_be_bytes());
        segment.extend_from_slice(&[1, 1, 0x11, 0]);
        segment
    }

    /// A scan header naming `components` components.
    fn scan_header(components: u8) -> Vec<u8> {
        let mut segment = vec![0xFF, 0xDA, 0x00, 6 + 2 * components, components];
        for id in 1..=components {
            segment.extend_from_slice(&[id, 0x00]);
        }
        segment.extend_from_slice(&[0x00, 0x3F, 0x00]);
        segment
    }

    #[test]
    fn jpeg_walks_segments_to_the_first_start_of_frame() {
        // SOI, an APP0 segment of length 4, then SOF0 declaring 2x3.
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00];
        jpeg.extend_from_slice(&grey_frame_header(0xC0, 2, 3));
        jpeg.extend_from_slice(&scan_header(1));
        assert_eq!(
            header_dimensions(InputFormat::Jpeg, &jpeg).expect("dims"),
            (2, 3)
        );
    }

    #[test]
    fn jpeg_does_not_mistake_a_huffman_table_for_a_frame_header() {
        // DHT (0xC4) sits inside the 0xC0..=0xCF range but is not an SOF.
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xC4, 0x00, 0x04, 0x00, 0x00];
        jpeg.extend_from_slice(&grey_frame_header(0xC2, 9, 5));
        jpeg.extend_from_slice(&scan_header(1));
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

    /// SOI, an APP0 segment, then a progressive frame header (SOF2) for a
    /// 4:2:0 YCbCr 640x480 image, a DHT segment, and a scan header naming
    /// `scan_components` components.
    fn progressive_jpeg(scan_components: u8) -> Vec<u8> {
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00];
        jpeg.extend_from_slice(&[0xFF, 0xC2, 0x00, 0x11, 8]);
        jpeg.extend_from_slice(&480u16.to_be_bytes());
        jpeg.extend_from_slice(&640u16.to_be_bytes());
        jpeg.extend_from_slice(&[3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
        jpeg.extend_from_slice(&[0xFF, 0xC4, 0x00, 0x04, 0x00, 0x00]);
        jpeg.extend_from_slice(&scan_header(scan_components));
        jpeg
    }

    #[test]
    fn a_jpeg_frame_reads_its_coding_components_and_first_scan() {
        let frame = jpeg_frame(&progressive_jpeg(3)).expect("a frame header");
        assert_eq!(frame.marker, 0xC2);
        assert_eq!(frame.precision, 8);
        assert_eq!((frame.width, frame.height), (640, 480));
        assert_eq!(frame.components, 3);
        assert_eq!(frame.ids[..3], [1, 2, 3]);
        assert_eq!(frame.sampling[..3], [(2, 2), (1, 1), (1, 1)]);
        assert_eq!(frame.first_scan, 3);

        assert_eq!(
            jpeg_frame(&progressive_jpeg(1)).map(|frame| frame.first_scan),
            Some(1)
        );

        // No scan header: the decoder has nothing to decode and errors.
        let mut no_scan = progressive_jpeg(3);
        no_scan.truncate(no_scan.len() - 14);
        assert_eq!(jpeg_frame(&no_scan), None);

        // A frame header cut off inside its component list.
        let mut cut = progressive_jpeg(3);
        cut.truncate(8 + 15);
        assert_eq!(jpeg_frame(&cut), None);
    }

    #[test]
    fn a_jpeg_walk_follows_the_decoders_marker_rules() {
        let with = |prefix: &[u8]| {
            let mut jpeg = vec![0xFF, 0xD8];
            jpeg.extend_from_slice(prefix);
            jpeg.extend_from_slice(&grey_frame_header(0xC0, 7, 5));
            jpeg.extend_from_slice(&scan_header(1));
            jpeg
        };
        let size = |jpeg: &[u8]| jpeg_frame(jpeg).map(|frame| (frame.width, frame.height));

        // Bytes that are not 0xFF, a fill run, and a stuffed 0xFF00 are all
        // stepped over on the way to the next marker.
        assert_eq!(size(&with(&[0x12, 0x34, 0xFF, 0xFF, 0x00])), Some((7, 5)));
        // RSTn and a second SOI carry no payload.
        assert_eq!(size(&with(&[0xFF, 0xD3, 0xFF, 0xD8])), Some((7, 5)));
        // TEM (0xFF01) has no payload in the standard, but the decoder reads
        // a length after it: a frame header inside that length is skipped.
        let mut tem = vec![0xFF, 0x01, 0x00, 0x0F];
        tem.extend_from_slice(&grey_frame_header(0xC0, 900, 900));
        assert_eq!(size(&with(&tem)), Some((7, 5)));
        // A marker the decoder skips, whose length does not read, is passed
        // over in place.
        assert_eq!(size(&with(&[0xFF, 0x02, 0x00, 0x01])), Some((7, 5)));

        // Where the decoder errors, the walk refuses: a second frame header,
        // a hierarchical one, a sampling factor of zero, the image ending.
        let mut two = with(&[]);
        let scan_at = two.len() - scan_header(1).len();
        two.splice(scan_at..scan_at, grey_frame_header(0xC2, 9, 9));
        assert_eq!(jpeg_frame(&two), None);
        let mut hierarchical = vec![0xFF, 0xD8];
        hierarchical.extend_from_slice(&grey_frame_header(0xC5, 7, 5));
        hierarchical.extend_from_slice(&scan_header(1));
        assert_eq!(jpeg_frame(&hierarchical), None);
        let mut zero = with(&[]);
        zero[2 + 11] = 0x01;
        assert_eq!(jpeg_frame(&zero), None);
        assert_eq!(jpeg_frame(&with(&[0xFF, 0xD9])), None);
    }

    /// A segment: marker, length, body.
    fn segment(marker: u8, body: &[u8]) -> Vec<u8> {
        let mut out = vec![0xFF, marker];
        out.extend_from_slice(&((body.len() + 2) as u16).to_be_bytes());
        out.extend_from_slice(body);
        out
    }

    /// A three-component frame header with the given component names and
    /// sampling factors.
    fn colour_frame_header(marker: u8, ids: [u8; 3], factors: [u8; 3]) -> Vec<u8> {
        let mut body = vec![8, 0, 16, 0, 32, 3];
        for (id, factor) in ids.iter().zip(factors) {
            body.extend_from_slice(&[*id, factor, 0]);
        }
        segment(marker, &body)
    }

    #[test]
    fn the_zune_walk_reads_every_marker_by_its_length() {
        // A restart marker among the headers is read with a length, as
        // zune-jpeg reads it: the 8x8 frame header inside that length is
        // skipped, and the 32x16 one after it is the frame. oxideav-mjpeg's
        // walk takes the restart marker as standalone, so it meets two frame
        // headers and refuses.
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xD0, 0x00, 0x0F];
        jpeg.extend_from_slice(&grey_frame_header(0xC0, 8, 8)[..13]);
        jpeg.extend_from_slice(&grey_frame_header(0xC0, 32, 16));
        jpeg.extend_from_slice(&scan_header(1));
        let zune = jpeg_zune(&jpeg).expect("zune-jpeg reads it");
        assert_eq!((zune.frame.width, zune.frame.height), (32, 16));
        assert_eq!(jpeg_frame(&jpeg), None);
        assert_eq!(
            header_dimensions(InputFormat::Jpeg, &jpeg).expect("dims"),
            (32, 16)
        );
    }

    #[test]
    fn the_zune_walk_refuses_where_zune_jpeg_errors() {
        let with = |prefix: &[u8], frame: Vec<u8>| {
            let mut jpeg = vec![0xFF, 0xD8];
            jpeg.extend_from_slice(prefix);
            jpeg.extend_from_slice(&frame);
            jpeg.extend_from_slice(&scan_header(1));
            jpeg
        };
        let grey = || grey_frame_header(0xC0, 7, 5);
        assert!(jpeg_zune(&with(&[], grey())).is_some());
        // A second frame header.
        let mut two = grey();
        two.extend_from_slice(&grey_frame_header(0xC2, 9, 9));
        assert_eq!(jpeg_zune(&with(&[], two)), None);
        // A hierarchical frame header.
        assert_eq!(jpeg_zune(&with(&[], grey_frame_header(0xC5, 7, 5))), None);
        // A sampling factor that is not 1, 2 or 4.
        let mut three = grey();
        three[11] = 0x31;
        assert_eq!(jpeg_zune(&with(&[], three)), None);
        // The image ending before a scan.
        assert_eq!(jpeg_zune(&with(&[0xFF, 0xD9], grey())), None);
        // An ICC chunk numbered 0, and an Adobe segment of an unknown
        // transform.
        let icc = segment(0xE2, b"ICC_PROFILE\0\x00\x01data");
        assert_eq!(jpeg_zune(&with(&icc, grey())), None);
        let adobe = segment(0xEE, b"Adobe\0\x64\0\0\0\0\x07");
        assert_eq!(jpeg_zune(&with(&adobe, grey())), None);
    }

    #[test]
    fn the_zune_walk_reads_the_colour_space_zune_jpeg_decodes_from() {
        let colour = |prefix: &[u8], frame: Vec<u8>| {
            let mut jpeg = vec![0xFF, 0xD8];
            jpeg.extend_from_slice(prefix);
            jpeg.extend_from_slice(&frame);
            jpeg.extend_from_slice(&scan_header(3));
            jpeg_zune(&jpeg).expect("zune-jpeg reads it").colour
        };
        let ycbcr = || colour_frame_header(0xC0, [1, 2, 3], [0x22, 0x11, 0x11]);
        assert_eq!(colour(&[], ycbcr()), JpegColour::YCbCr);
        assert_eq!(
            colour(&[], colour_frame_header(0xC0, *b"RGB", [0x11; 3])),
            JpegColour::Rgb
        );
        let adobe = |transform: u8| {
            segment(
                0xEE,
                &[b'A', b'd', b'o', b'b', b'e', 0, 100, 0, 0, 0, 0, transform],
            )
        };
        assert_eq!(colour(&adobe(0), ycbcr()), JpegColour::Rgb);
        assert_eq!(colour(&adobe(1), ycbcr()), JpegColour::YCbCr);
        let mut grey = vec![0xFF, 0xD8];
        grey.extend_from_slice(&grey_frame_header(0xC0, 7, 5));
        grey.extend_from_slice(&scan_header(1));
        assert_eq!(jpeg_zune(&grey).map(|z| z.colour), Some(JpegColour::Grey));
    }

    /// zune-jpeg re-reads every Extended XMP segment it keeps after each
    /// marker, the segment's own and the start of scan included, so three
    /// segments then a frame header and a scan header cost 1 + 2 + 3 + 3 + 3
    /// segment reads. Segments of other kinds cost nothing.
    #[test]
    fn the_zune_walk_counts_the_extended_xmp_reassembly_reads() {
        let mut extended = EXTENDED_XMP.to_vec();
        extended.extend_from_slice(&[b'0'; 32]);
        extended.extend_from_slice(&1u32.to_be_bytes());
        extended.extend_from_slice(&1u32.to_be_bytes());
        extended.push(0);
        let mut jpeg = vec![0xFF, 0xD8];
        jpeg.extend_from_slice(&segment(0xE1, b"Exif\0\0\x07"));
        for _ in 0..3 {
            jpeg.extend_from_slice(&segment(0xE1, &extended));
        }
        jpeg.extend_from_slice(&grey_frame_header(0xC0, 7, 5));
        jpeg.extend_from_slice(&scan_header(1));
        let zune = jpeg_zune(&jpeg).expect("zune-jpeg reads it");
        assert_eq!(zune.xmp_reads, 12 * ZUNE_XMP_PASS_BYTES);

        let mut plain = vec![0xFF, 0xD8];
        plain.extend_from_slice(&segment(0xE1, b"Exif\0\0\x07"));
        plain.extend_from_slice(&grey_frame_header(0xC0, 7, 5));
        plain.extend_from_slice(&scan_header(1));
        assert_eq!(jpeg_zune(&plain).map(|z| z.xmp_reads), Some(0));
    }

    #[test]
    fn the_zune_walk_counts_the_metadata_zune_jpeg_copies() {
        let mut exif = b"Exif\0\0".to_vec();
        exif.extend_from_slice(&[7; 100]);
        let mut icc = b"ICC_PROFILE\0\x01\x01".to_vec();
        icc.extend_from_slice(&[7; 50]);
        let mut iptc = b"Photoshop 3.0\0".to_vec();
        iptc.extend_from_slice(&[7; 30]);
        let mut jpeg = vec![0xFF, 0xD8];
        jpeg.extend_from_slice(&segment(0xE1, &exif));
        jpeg.extend_from_slice(&segment(0xE2, &icc));
        jpeg.extend_from_slice(&segment(0xED, &iptc));
        // An APP1 segment zune-jpeg does not recognise is not copied.
        jpeg.extend_from_slice(&segment(0xE1, &[7; 40]));
        jpeg.extend_from_slice(&grey_frame_header(0xC2, 7, 5));
        jpeg.extend_from_slice(&scan_header(1));
        // A progressive image reads segments between its scans too.
        jpeg.extend_from_slice(&[0x12, 0x34]);
        jpeg.extend_from_slice(&segment(0xE2, &icc));
        let zune = jpeg_zune(&jpeg).expect("zune-jpeg reads it");
        let chunk = 50 + 2 * ZUNE_ICC_CHUNK;
        assert_eq!(zune.metadata, 100 + chunk + 30 + chunk);
    }

    #[test]
    fn a_bmp_layout_reads_both_header_generations() {
        // BITMAPINFOHEADER (40): 8 bits a pixel, RLE8, 16 palette entries.
        let mut bmp = Vec::from(*b"BM");
        bmp.extend_from_slice(&[0u8; 12]);
        bmp.extend_from_slice(&40u32.to_le_bytes());
        bmp.extend_from_slice(&4i32.to_le_bytes());
        bmp.extend_from_slice(&2i32.to_le_bytes());
        bmp.extend_from_slice(&1u16.to_le_bytes());
        bmp.extend_from_slice(&8u16.to_le_bytes());
        bmp.extend_from_slice(&1u32.to_le_bytes());
        bmp.extend_from_slice(&[0u8; 12]);
        bmp.extend_from_slice(&16u32.to_le_bytes());
        bmp.extend_from_slice(&[0u8; 4]);
        assert_eq!(
            bmp_layout(&bmp),
            Some(BmpLayout {
                bits_per_pixel: 8,
                compression: 1,
                palette_entries: 16,
                palette_entry_bytes: 4,
            })
        );

        // No colors-used count means the whole table for the bit depth.
        bmp[46..50].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(
            bmp_layout(&bmp).map(|layout| layout.palette_entries),
            Some(256)
        );

        // BITMAPCOREHEADER (12): 4 bits a pixel, three-byte palette entries.
        let mut core = Vec::from(*b"BM");
        core.extend_from_slice(&[0u8; 12]);
        core.extend_from_slice(&12u32.to_le_bytes());
        core.extend_from_slice(&7u16.to_le_bytes());
        core.extend_from_slice(&3u16.to_le_bytes());
        core.extend_from_slice(&1u16.to_le_bytes());
        core.extend_from_slice(&4u16.to_le_bytes());
        assert_eq!(
            bmp_layout(&core),
            Some(BmpLayout {
                bits_per_pixel: 4,
                compression: 0,
                palette_entries: 16,
                palette_entry_bytes: 3,
            })
        );
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
