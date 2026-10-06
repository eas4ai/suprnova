//! What an image carries besides its pixels, read from a source and written
//! into processed output by both drivers.
//!
//! Processed output keeps exactly two things (IMG-002): the source's ICC
//! profile, so its colours read the same, and, when orientation was not
//! applied, an EXIF block holding the `Orientation` tag alone, so a viewer
//! can still turn the image. Everything else is dropped: EXIF (the GPS
//! position with it), XMP, IPTC, comments and text chunks, and the date
//! chunks an encoder adds by itself.
//!
//! The built-in driver's encoders write pixels only, so its output just has
//! the two kept items added. ImageMagick writes whatever it read and adds
//! more (`date:` text chunks, its own orientation chunk, the comment), so the
//! `magick` driver passes its output through the same rewrite with
//! everything else stripped. One rewrite serves both, so the drivers cannot
//! drift apart on what survives.
//!
//! Every reader here takes untrusted bytes: it never indexes past them, never
//! inflates a compressed chunk it does not need, and bounds the one it does
//! (a PNG `iCCP`) by the decode budget.

use crate::error::FrameworkError;

use super::color::Color;
use super::driver::OutputFormat;
use super::orientation::{Orientation, orientation_only_exif};
use super::sniff::InputFormat;

/// What the decode of a compressed profile allocates besides its output:
/// compcol's 32 KiB window, its Huffman tables, and the 64 KiB scratch the
/// counting pass writes through. The same figure the PNG decode estimate
/// uses for an inflate.
const INFLATE_WORK: u64 = 128 * 1024;

/// The largest ICC profile a JPEG can hold: 255 APP2 segments of 65,519
/// bytes each.
const JPEG_ICC_CHUNK: usize = 65_519;

/// The prefix of an APP2 segment that carries an ICC profile chunk.
const ICC_PROFILE: &[u8] = b"ICC_PROFILE\0";

/// The prefix of an APP1 segment that carries EXIF.
const EXIF_PREFIX: &[u8] = b"Exif\0\0";

/// The colour space an ICC profile describes, from its header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ColourClass {
    /// `RGB `: the class of RGB, YCbCr and palette pixels.
    Rgb,
    /// `GRAY`.
    Gray,
    /// CMYK, Lab, or any other space.
    Other,
}

/// The colour space of a structurally valid ICC profile, or `None` when the
/// bytes are not one: shorter than the 128-byte header, or without the
/// `acsp` signature.
pub(crate) fn icc_class(profile: &[u8]) -> Option<ColourClass> {
    if profile.len() < 128 || profile.get(36..40)? != b"acsp" {
        return None;
    }
    Some(match profile.get(16..20)? {
        b"RGB " => ColourClass::Rgb,
        b"GRAY" => ColourClass::Gray,
        _ => ColourClass::Other,
    })
}

// ───────────────────────── container walks ─────────────────────────

fn be_u16(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn be_u32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn le_u32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

/// One marker segment of a JPEG before its first scan.
struct JpegSegment {
    marker: u8,
    /// Where the segment starts (its `0xFF`) and ends, in the file.
    start: usize,
    end: usize,
    /// The payload after the two length bytes.
    body: std::ops::Range<usize>,
}

/// Visit the marker segments of a JPEG up to its first start-of-scan, and
/// return the offset of that scan's marker. `None` when the header does not
/// read. Allocates nothing, so a file of a million segments costs no memory
/// to walk.
///
/// Fill bytes before a marker are skipped. Standalone markers (SOI, RSTn,
/// TEM) carry no length and are passed over.
fn jpeg_walk(bytes: &[u8], mut visit: impl FnMut(&JpegSegment)) -> Option<usize> {
    if bytes.get(..2)? != [0xFF, 0xD8] {
        return None;
    }
    let mut pos = 2;
    loop {
        let start = pos;
        if *bytes.get(pos)? != 0xFF {
            return None;
        }
        while bytes.get(pos) == Some(&0xFF) {
            pos += 1;
        }
        let marker = *bytes.get(pos)?;
        pos += 1;
        match marker {
            0xDA => return Some(start),
            0xD9 => return None,
            0x01 | 0xD0..=0xD8 => continue,
            _ => {}
        }
        let length = usize::from(be_u16(bytes, pos)?);
        if length < 2 {
            return None;
        }
        let end = pos.checked_add(length)?;
        if end > bytes.len() {
            return None;
        }
        visit(&JpegSegment {
            marker,
            start,
            end,
            body: pos + 2..end,
        });
        pos = end;
    }
}

/// Reassemble a JPEG's ICC profile from its APP2 chunks, as zune-jpeg does:
/// every chunk names the same count, each sequence number from 1 to that
/// count appears once, and the chunks join in sequence order. Anything else
/// is a corrupt profile and reads as none.
fn jpeg_icc(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut chunks: [Option<&[u8]>; 256] = [None; 256];
    let mut count: Option<u8> = None;
    let mut corrupt = false;
    jpeg_walk(bytes, |segment| {
        let body = &bytes[segment.body.clone()];
        if corrupt || segment.marker != 0xE2 || !body.starts_with(ICC_PROFILE) {
            return;
        }
        let (Some(&sequence), Some(&total)) = (body.get(12), body.get(13)) else {
            corrupt = true;
            return;
        };
        let slot = &mut chunks[usize::from(sequence)];
        if total == 0
            || sequence == 0
            || sequence > total
            || *count.get_or_insert(total) != total
            || slot.is_some()
        {
            corrupt = true;
            return;
        }
        *slot = Some(&body[14..]);
    })?;
    let total = usize::from(count?);
    if corrupt {
        return None;
    }
    let parts = chunks.get(1..=total)?;
    let length = parts
        .iter()
        .try_fold(0usize, |sum, part| part.map(|data| sum + data.len()))?;
    let mut profile = Vec::with_capacity(length);
    for part in parts.iter().flatten() {
        profile.extend_from_slice(part);
    }
    Some(profile)
}

/// The TIFF bytes of the last Exif APP1 segment before the first scan: the
/// one zune-jpeg keeps.
fn jpeg_exif(bytes: &[u8]) -> Option<&[u8]> {
    let mut exif = None;
    jpeg_walk(bytes, |segment| {
        let body = &bytes[segment.body.clone()];
        if segment.marker == 0xE1 && body.len() > EXIF_PREFIX.len() && body.starts_with(EXIF_PREFIX)
        {
            exif = Some(&body[EXIF_PREFIX.len()..]);
        }
    })?;
    exif
}

/// One chunk of a PNG: its type, where the whole chunk sits (length to CRC)
/// and where its data sits.
struct PngChunk {
    kind: [u8; 4],
    whole: std::ops::Range<usize>,
    data: std::ops::Range<usize>,
}

/// Visit the chunks of a PNG, signature to IEND. CRCs are not checked: a
/// chunk is either copied whole, CRC included, or dropped. `None` when the
/// chunk list does not read. Allocates nothing.
fn png_walk(bytes: &[u8], mut visit: impl FnMut(&PngChunk)) -> Option<()> {
    if bytes.get(..8)? != b"\x89PNG\r\n\x1a\n" {
        return None;
    }
    let mut pos = 8;
    while pos < bytes.len() {
        let length = usize::try_from(be_u32(bytes, pos)?).ok()?;
        let kind: [u8; 4] = bytes.get(pos + 4..pos + 8)?.try_into().ok()?;
        let data = pos + 8..pos.checked_add(8)?.checked_add(length)?;
        let end = data.end.checked_add(4)?;
        if end > bytes.len() {
            return None;
        }
        visit(&PngChunk {
            kind,
            whole: pos..end,
            data,
        });
        pos = end;
        if &kind == b"IEND" {
            break;
        }
    }
    Some(())
}

/// The data of a PNG's first chunk of `kind`.
fn png_chunk<'a>(bytes: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    let mut found = None;
    png_walk(bytes, |chunk| {
        if found.is_none() && &chunk.kind == kind {
            found = Some(chunk.data.clone());
        }
    })?;
    bytes.get(found?)
}

/// The colour chunks a PNG keeps with its profile (IMG-002): `cHRM`,
/// `gAMA`, `sRGB` and `cICP`, each as its type and data, in file order.
pub(crate) fn png_colour_chunks(bytes: &[u8]) -> Vec<([u8; 4], Vec<u8>)> {
    let mut found = Vec::new();
    png_walk(bytes, |chunk| {
        if is_png_colour_chunk(&chunk.kind) && found.len() < 4 {
            found.push((chunk.kind, bytes[chunk.data.clone()].to_vec()));
        }
    });
    found
}

fn is_png_colour_chunk(kind: &[u8; 4]) -> bool {
    matches!(kind, b"cHRM" | b"gAMA" | b"sRGB" | b"cICP")
}

/// A PNG's `iCCP` profile, inflated within `limit` bytes.
///
/// The profile is compressed, so the inflate is what IMG-002 bounds: a few
/// kilobytes can expand to gigabytes. A first pass counts the inflated size
/// without keeping it, refusing past `limit`; the second inflates into a
/// buffer of exactly that size.
fn png_icc(bytes: &[u8], limit: u64) -> Result<Option<Vec<u8>>, FrameworkError> {
    let Some(data) = png_chunk(bytes, b"iCCP") else {
        return Ok(None);
    };
    // A 1-79 byte name, a NUL, the compression method (0, zlib), the data.
    let Some(name_end) = data.iter().take(80).position(|&byte| byte == 0) else {
        return Ok(None);
    };
    if data.get(name_end + 1) != Some(&0) {
        return Ok(None);
    }
    let compressed = &data[name_end + 2..];
    match inflate_bounded(compressed, limit) {
        Ok(profile) => Ok(Some(profile)),
        Err(compcol::Error::OutputLimitExceeded) => Err(FrameworkError::param(format!(
            "image exceeds configured decode limits: its PNG ICC profile inflates past the \
             {limit} bytes the IMAGE_MAX_ALLOC_BYTES limit leaves for it"
        ))),
        // A profile that does not inflate is a corrupt profile, which reads
        // as none, the way a decoder ignores it.
        Err(_) => Ok(None),
    }
}

/// Inflate a zlib stream into a buffer of exactly its inflated size,
/// failing with `OutputLimitExceeded` past `limit` bytes before any of it is
/// kept.
fn inflate_bounded(data: &[u8], limit: u64) -> Result<Vec<u8>, compcol::Error> {
    use compcol::{Algorithm, Decoder, Status};

    let run = |mut sink: Option<&mut Vec<u8>>| -> Result<u64, compcol::Error> {
        let mut decoder =
            compcol::limit::LimitedDecoder::new(compcol::zlib::Zlib::decoder(), limit);
        let mut scratch = vec![0u8; 64 * 1024];
        let mut total = 0u64;
        let mut consumed = 0;
        let mut keep = |written: usize, scratch: &[u8], total: &mut u64| {
            *total += written as u64;
            if let Some(out) = sink.as_deref_mut() {
                out.extend_from_slice(&scratch[..written]);
            }
        };
        while consumed < data.len() {
            let (progress, status) = decoder.decode(&data[consumed..], &mut scratch)?;
            consumed += progress.consumed;
            keep(progress.written, &scratch, &mut total);
            match status {
                Status::StreamEnd => return Ok(total),
                Status::InputEmpty => break,
                Status::OutputFull => {
                    if progress.consumed == 0 && progress.written == 0 {
                        break;
                    }
                }
            }
        }
        loop {
            let (progress, status) = decoder.finish(&mut scratch)?;
            keep(progress.written, &scratch, &mut total);
            if status == Status::StreamEnd {
                return Ok(total);
            }
            if progress.written == 0 {
                return Err(compcol::Error::Corrupt);
            }
        }
    };
    let length = run(None)?;
    let mut profile = Vec::with_capacity(usize::try_from(length).unwrap_or(0));
    run(Some(&mut profile))?;
    Ok(profile)
}

/// One top-level chunk of a RIFF WebP: its fourcc, the whole chunk
/// (padding included) and its payload.
struct RiffChunk {
    fourcc: [u8; 4],
    whole: std::ops::Range<usize>,
    payload: std::ops::Range<usize>,
}

/// Visit the top-level chunks of a RIFF WebP after its 12-byte header.
/// Stops at 4096 chunks, the bound the header gate refuses past. Allocates
/// nothing.
fn riff_walk(bytes: &[u8], mut visit: impl FnMut(&RiffChunk)) -> Option<()> {
    if bytes.get(..4)? != b"RIFF" || bytes.get(8..12)? != b"WEBP" {
        return None;
    }
    let mut pos = 12;
    let mut visited = 0;
    while pos + 8 <= bytes.len() && visited < 4096 {
        let fourcc: [u8; 4] = bytes.get(pos..pos + 4)?.try_into().ok()?;
        let size = usize::try_from(le_u32(bytes, pos + 4)?).ok()?;
        let payload = pos + 8..(pos + 8).checked_add(size)?.min(bytes.len());
        let end = payload.end.saturating_add(size & 1).min(bytes.len());
        visit(&RiffChunk {
            fourcc,
            whole: pos..end,
            payload,
        });
        visited += 1;
        if end <= pos {
            break;
        }
        pos = end;
    }
    Some(())
}

/// The payload of a WebP's first top-level chunk named `fourcc`.
fn webp_chunk<'a>(bytes: &'a [u8], fourcc: &[u8; 4]) -> Option<&'a [u8]> {
    let mut found = None;
    riff_walk(bytes, |chunk| {
        if found.is_none() && &chunk.fourcc == fourcc {
            found = Some(chunk.payload.clone());
        }
    })?;
    bytes.get(found?)
}

/// A WebP's EXIF as TIFF bytes, its `Exif\0\0` prefix removed when a writer
/// added one.
fn webp_exif(bytes: &[u8]) -> Option<&[u8]> {
    let exif = webp_chunk(bytes, b"EXIF")?;
    Some(exif.strip_prefix(EXIF_PREFIX).unwrap_or(exif))
}

/// The profile a V5 BMP embeds, and where it sits: `bV5CSType` is
/// `PROFILE_EMBEDDED` (`MBED`), and the offset is from the start of the info
/// header.
fn bmp_icc_range(bytes: &[u8]) -> Option<std::ops::Range<usize>> {
    const PROFILE_EMBEDDED: u32 = 0x4D42_4544;
    if bytes.get(..2)? != b"BM"
        || le_u32(bytes, 14)? < 124
        || le_u32(bytes, 14 + 56)? != PROFILE_EMBEDDED
    {
        return None;
    }
    let offset = 14usize.checked_add(usize::try_from(le_u32(bytes, 14 + 112)?).ok()?)?;
    let size = usize::try_from(le_u32(bytes, 14 + 116)?).ok()?;
    let range = offset..offset.checked_add(size)?;
    (size > 0 && range.end <= bytes.len()).then_some(range)
}

/// One block of a GIF after its header and global colour table.
enum GifBlock {
    /// An extension: its label, the first sub-block's data (which names an
    /// application extension), and the whole block's range.
    Extension {
        label: u8,
        first: std::ops::Range<usize>,
        whole: std::ops::Range<usize>,
    },
    /// An image: the range of its local colour table (empty when it has
    /// none), and the whole block's range.
    Image {
        table: std::ops::Range<usize>,
        whole: std::ops::Range<usize>,
    },
}

/// Skip a run of GIF sub-blocks starting at `pos`; the position after its
/// terminator.
fn gif_sub_blocks_end(bytes: &[u8], mut pos: usize) -> Option<usize> {
    loop {
        let size = usize::from(*bytes.get(pos)?);
        pos += 1;
        if size == 0 {
            return Some(pos);
        }
        pos = pos.checked_add(size)?;
    }
}

/// Visit a GIF's blocks up to its trailer. Returns the global colour
/// table's range and the trailer's offset; `None` when the block structure
/// does not read. Allocates nothing.
fn gif_walk(
    bytes: &[u8],
    mut visit: impl FnMut(&GifBlock),
) -> Option<(std::ops::Range<usize>, usize)> {
    if !matches!(bytes.get(..6)?, b"GIF87a" | b"GIF89a") {
        return None;
    }
    let table_len = |packed: u8| {
        if packed & 0x80 != 0 {
            3usize << ((packed & 0x07) + 1)
        } else {
            0
        }
    };
    let global = 13..13 + table_len(*bytes.get(10)?);
    let mut pos = global.end;
    loop {
        let start = pos;
        match *bytes.get(pos)? {
            0x3B => return Some((global, pos)),
            0x21 => {
                let label = *bytes.get(pos + 1)?;
                let size = usize::from(*bytes.get(pos + 2)?);
                let first = pos + 3..pos + 3 + size;
                let end = gif_sub_blocks_end(bytes, pos + 2)?;
                visit(&GifBlock::Extension {
                    label,
                    first,
                    whole: start..end,
                });
                pos = end;
            }
            0x2C => {
                let table = pos + 10..pos + 10 + table_len(*bytes.get(pos + 9)?);
                // The LZW minimum code size, then the data sub-blocks.
                let end = gif_sub_blocks_end(bytes, table.end + 1)?;
                visit(&GifBlock::Image {
                    table,
                    whole: start..end,
                });
                pos = end;
            }
            _ => return None,
        }
        if pos > bytes.len() {
            return None;
        }
    }
}

/// The application identifier and code of the ICC profile extension.
const GIF_ICC_APPLICATION: &[u8] = b"ICCRGBG1012";
/// Application extensions that describe playback, which a GIF keeps.
const GIF_LOOP_APPLICATIONS: [&[u8]; 2] = [b"NETSCAPE2.0", b"ANIMEXTS1.0"];

/// A GIF's ICC profile: the sub-blocks after an `ICCRGBG1012` application
/// extension's identifier, joined.
fn gif_icc(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut found = None;
    gif_walk(bytes, |block| {
        if let GifBlock::Extension {
            label: 0xFF,
            first,
            whole,
        } = block
            && found.is_none()
            && bytes.get(first.clone()) == Some(GIF_ICC_APPLICATION)
        {
            found = Some(first.end..whole.end);
        }
    })?;
    let sub_blocks = found?;
    // Sub-blocks: a length byte, then that many bytes, to a zero length.
    let mut length = 0usize;
    let mut pos = sub_blocks.start;
    while pos < sub_blocks.end {
        let size = usize::from(*bytes.get(pos)?);
        if size == 0 {
            break;
        }
        length += size;
        pos += 1 + size;
    }
    let mut profile = Vec::with_capacity(length);
    let mut pos = sub_blocks.start;
    while pos < sub_blocks.end {
        let size = usize::from(*bytes.get(pos)?);
        if size == 0 {
            break;
        }
        profile.extend_from_slice(bytes.get(pos + 1..pos + 1 + size)?);
        pos += 1 + size;
    }
    Some(profile)
}

// ───────────────────────── reading a source ─────────────────────────

/// The EXIF orientation a source carries, without inflating anything: a
/// JPEG's last Exif APP1 segment, a PNG's uncompressed `eXIf` chunk, or a
/// WebP's `EXIF` chunk. BMP and GIF hold no EXIF.
pub(crate) fn source_orientation(format: InputFormat, bytes: &[u8]) -> Option<Orientation> {
    let tiff = match format {
        InputFormat::Jpeg => jpeg_exif(bytes)?,
        InputFormat::Png => png_chunk(bytes, b"eXIf")?,
        InputFormat::WebP => webp_exif(bytes)?,
        InputFormat::Gif | InputFormat::Bmp => return None,
    };
    Orientation::from_tiff(tiff)
}

/// The ICC profile a source carries, in any of the five formats, or `None`
/// when it has none or a corrupt one. Only a PNG's profile is compressed;
/// its inflate is refused past `inflate_limit` bytes.
pub(crate) fn source_icc(
    format: InputFormat,
    bytes: &[u8],
    inflate_limit: u64,
) -> Result<Option<Vec<u8>>, FrameworkError> {
    let profile = match format {
        InputFormat::Jpeg => jpeg_icc(bytes),
        InputFormat::Png => png_icc(bytes, inflate_limit)?,
        InputFormat::WebP => webp_chunk(bytes, b"ICCP").map(<[u8]>::to_vec),
        InputFormat::Bmp => bmp_icc_range(bytes).map(|range| bytes[range].to_vec()),
        InputFormat::Gif => gif_icc(bytes),
    };
    Ok(profile.filter(|profile| icc_class(profile).is_some()))
}

/// What an inflate of a profile may take: the budget, less what the
/// pipeline already holds and the inflate's own work.
pub(crate) fn inflate_limit(max_alloc_bytes: u64, held: u64) -> u64 {
    max_alloc_bytes
        .saturating_sub(held)
        .saturating_sub(INFLATE_WORK)
}

// ───────────────────────── reading processed output ─────────────────────────

/// The input format an output format is read back as.
fn input_of(format: OutputFormat) -> InputFormat {
    match format {
        OutputFormat::Jpeg => InputFormat::Jpeg,
        OutputFormat::Png => InputFormat::Png,
        OutputFormat::WebP | OutputFormat::WebPLossless => InputFormat::WebP,
        OutputFormat::Gif => InputFormat::Gif,
        OutputFormat::Bmp => InputFormat::Bmp,
    }
}

/// The ICC profile in encoded output, as an encoder wrote it.
pub(crate) fn output_icc(
    format: OutputFormat,
    bytes: &[u8],
    inflate_limit: u64,
) -> Result<Option<Vec<u8>>, FrameworkError> {
    source_icc(input_of(format), bytes, inflate_limit)
}

/// The EXIF orientation in encoded output, as an encoder wrote it.
pub(crate) fn output_orientation(format: OutputFormat, bytes: &[u8]) -> Option<Orientation> {
    source_orientation(input_of(format), bytes)
}

/// The colour class of the pixels encoded output stores: grey for a
/// one-component JPEG and a greyscale PNG, other for a four-component
/// (CMYK) JPEG, RGB for everything else (WebP, GIF and BMP palettes, and
/// BMP direct colour, are always RGB).
pub(crate) fn pixel_class(format: OutputFormat, bytes: &[u8]) -> ColourClass {
    match format {
        OutputFormat::Jpeg => {
            let mut components = None;
            jpeg_walk(bytes, |segment| {
                if components.is_none()
                    && matches!(segment.marker, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF)
                {
                    components = bytes.get(segment.body.start + 5).copied();
                }
            });
            match components {
                Some(1) => ColourClass::Gray,
                Some(4) => ColourClass::Other,
                _ => ColourClass::Rgb,
            }
        }
        OutputFormat::Png => {
            match png_chunk(bytes, b"IHDR").and_then(|ihdr| ihdr.get(9).copied()) {
                Some(0 | 4) => ColourClass::Gray,
                _ => ColourClass::Rgb,
            }
        }
        OutputFormat::WebP | OutputFormat::WebPLossless | OutputFormat::Gif | OutputFormat::Bmp => {
            ColourClass::Rgb
        }
    }
}

// ───────────────────────── colour conversion ─────────────────────────

/// Converts pixels from a profile's space to sRGB, for output that cannot
/// carry the profile: GIF, and pixels the profile no longer describes.
pub(crate) enum SrgbConversion {
    /// An RGB profile, through moxcms.
    Rgb(std::sync::Arc<moxcms::Transform8BitExecutor>),
    /// A grey profile, as a table from each 8-bit level to its sRGB level.
    /// Applied to each of R, G and B, so grey pixels stay exact and colour
    /// a step added (a coloured rotation background) keeps its hue.
    Table(Box<[u8; 256]>),
}

impl SrgbConversion {
    /// The conversion from `profile`, or `None` when moxcms cannot read the
    /// profile or the space has no conversion to sRGB here (CMYK, Lab).
    pub(crate) fn from_profile(profile: &[u8]) -> Option<Self> {
        use moxcms::{ColorProfile, Layout, TransformOptions};

        let source = ColorProfile::new_from_slice(profile).ok()?;
        let srgb = ColorProfile::new_srgb();
        match icc_class(profile)? {
            ColourClass::Rgb => source
                .create_transform_8bit(Layout::Rgb, &srgb, Layout::Rgb, TransformOptions::default())
                .ok()
                .map(Self::Rgb),
            ColourClass::Gray => {
                let transform = source
                    .create_transform_8bit(
                        Layout::Gray,
                        &srgb,
                        Layout::Rgb,
                        TransformOptions::default(),
                    )
                    .ok()?;
                let levels: Vec<u8> = (0..=255).collect();
                let mut rgb = vec![0u8; 256 * 3];
                transform.transform(&levels, &mut rgb).ok()?;
                let mut table = Box::new([0u8; 256]);
                for (level, out) in table.iter_mut().enumerate() {
                    *out = rgb[level * 3];
                }
                Some(Self::Table(table))
            }
            ColourClass::Other => None,
        }
    }

    /// Convert packed RGB triples in place: a palette, or the colour part
    /// of RGBA through [`Self::convert_rgba`].
    pub(crate) fn convert_rgb(&self, rgb: &mut [u8]) -> Result<(), FrameworkError> {
        match self {
            Self::Rgb(transform) => {
                // moxcms writes into a separate buffer; a small one, reused,
                // keeps the conversion from copying the whole image.
                let mut out = [0u8; 3 * 1024];
                for chunk in rgb.chunks_mut(3 * 1024) {
                    let target = &mut out[..chunk.len()];
                    transform.transform(chunk, target).map_err(|e| {
                        FrameworkError::internal(format!(
                            "image colour conversion to sRGB failed: {e}"
                        ))
                    })?;
                    chunk.copy_from_slice(target);
                }
            }
            Self::Table(table) => {
                for value in rgb.iter_mut() {
                    *value = table[usize::from(*value)];
                }
            }
        }
        Ok(())
    }

    /// Convert packed RGBA in place, leaving alpha as it is.
    pub(crate) fn convert_rgba(&self, rgba: &mut [u8]) -> Result<(), FrameworkError> {
        let mut rgb = [0u8; 3 * 1024];
        for chunk in rgba.chunks_mut(4 * 1024) {
            let pixels = chunk.len() / 4;
            for (index, pixel) in chunk.as_chunks::<4>().0.iter().enumerate() {
                rgb[index * 3..index * 3 + 3].copy_from_slice(&pixel[..3]);
            }
            self.convert_rgb(&mut rgb[..pixels * 3])?;
            for (index, pixel) in chunk.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                pixel[..3].copy_from_slice(&rgb[index * 3..index * 3 + 3]);
            }
        }
        Ok(())
    }
}

// ───────────────────────── writing processed output ─────────────────────────

/// What processed output carries, decided by the driver.
#[derive(Default)]
pub(crate) struct OutputMetadata<'a> {
    /// The ICC profile to carry, already known to match the output's pixels.
    /// For GIF output, which holds no profile here, the profile the palette
    /// is converted from to sRGB instead.
    pub(crate) icc: Option<&'a [u8]>,
    /// The orientation to keep as an Orientation-only EXIF, when orientation
    /// was not applied.
    pub(crate) orientation: Option<Orientation>,
    /// A PNG source's colour chunks to carry into PNG output.
    pub(crate) png_colour: &'a [([u8; 4], Vec<u8>)],
    /// Whether the encoder may have written metadata that has to go:
    /// ImageMagick's output, as opposed to the built-in encoders', which
    /// write pixels only.
    pub(crate) strip: bool,
}

impl OutputMetadata<'_> {
    fn adds_nothing(&self) -> bool {
        self.icc.is_none() && self.orientation.is_none() && self.png_colour.is_empty()
    }
}

/// Rewrite encoded output so it carries what `metadata` says and nothing
/// else. Output that needs no change is returned as it is, without a copy.
pub(crate) fn rewrite(
    output: Vec<u8>,
    format: OutputFormat,
    metadata: &OutputMetadata<'_>,
) -> Result<Vec<u8>, FrameworkError> {
    if !metadata.strip && metadata.adds_nothing() {
        return Ok(output);
    }
    match format {
        OutputFormat::Jpeg => rewrite_jpeg(output, metadata),
        OutputFormat::Png => rewrite_png(output, metadata),
        OutputFormat::WebP | OutputFormat::WebPLossless => rewrite_webp(output, metadata),
        OutputFormat::Gif => rewrite_gif(output, metadata),
        OutputFormat::Bmp => rewrite_bmp(output, metadata),
    }
}

fn unreadable(format: &str) -> FrameworkError {
    FrameworkError::internal(format!(
        "image encode produced a {format} whose structure could not be read to set its metadata"
    ))
}

/// A JPEG segment with `marker` and `body`.
fn push_jpeg_segment(out: &mut Vec<u8>, marker: u8, parts: &[&[u8]]) {
    let length: usize = 2 + parts.iter().map(|part| part.len()).sum::<usize>();
    out.extend_from_slice(&[0xFF, marker]);
    out.extend_from_slice(&(length as u16).to_be_bytes());
    for part in parts {
        out.extend_from_slice(part);
    }
}

/// Keep, of a JPEG's header segments, the ones that describe how to decode
/// it: the frame, tables and restart interval, JFIF's `APP0` and Adobe's
/// `APP14` (its colour transform). Every other `APPn` and every comment is
/// metadata and goes.
fn keeps_jpeg_segment(marker: u8, body: &[u8]) -> bool {
    match marker {
        0xE0 => body.starts_with(b"JFIF\0"),
        0xEE => body.starts_with(b"Adobe"),
        0xE1..=0xEF | 0xFE => false,
        _ => true,
    }
}

fn rewrite_jpeg(output: Vec<u8>, metadata: &OutputMetadata<'_>) -> Result<Vec<u8>, FrameworkError> {
    let icc_chunks = match metadata.icc {
        Some(profile) => {
            let chunks: Vec<&[u8]> = profile.chunks(JPEG_ICC_CHUNK).collect();
            if chunks.len() > 255 {
                return Err(FrameworkError::param(format!(
                    "image ICC profile is {} bytes, more than a JPEG can hold (255 segments of \
                     {JPEG_ICC_CHUNK} bytes); convert the output to PNG or WebP to keep it",
                    profile.len()
                )));
            }
            chunks
        }
        None => Vec::new(),
    };
    let mut out = Vec::with_capacity(output.len() + metadata.icc.map_or(0, <[u8]>::len) + 64);
    out.extend_from_slice(&[0xFF, 0xD8]);
    let mut inserted = false;
    let insert = |out: &mut Vec<u8>| {
        if let Some(orientation) = metadata.orientation {
            push_jpeg_segment(
                out,
                0xE1,
                &[EXIF_PREFIX, &orientation_only_exif(orientation)],
            );
        }
        let total = icc_chunks.len() as u8;
        for (index, chunk) in icc_chunks.iter().enumerate() {
            push_jpeg_segment(out, 0xE2, &[ICC_PROFILE, &[index as u8 + 1, total], chunk]);
        }
    };
    let scan = jpeg_walk(&output, |segment| {
        let body = &output[segment.body.clone()];
        if !keeps_jpeg_segment(segment.marker, body) {
            return;
        }
        if !inserted && segment.marker != 0xE0 {
            insert(&mut out);
            inserted = true;
        }
        out.extend_from_slice(&output[segment.start..segment.end]);
    })
    .ok_or_else(|| unreadable("JPEG"))?;
    if !inserted {
        insert(&mut out);
    }
    out.extend_from_slice(&output[scan..]);
    Ok(out)
}

/// A PNG chunk: length, type, data, CRC.
fn push_png_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    oxideav_png::chunk::write_chunk(out, kind, data);
}

/// The chunks an encoder writes that IMG-002 strips: EXIF, text, the
/// time, ImageMagick's own orientation chunk and its older EXIF chunk
/// names; and the colour chunks, which are rewritten from the source.
fn drops_png_chunk(kind: &[u8; 4]) -> bool {
    matches!(
        kind,
        b"eXIf" | b"exIf" | b"zxIf" | b"tEXt" | b"zTXt" | b"iTXt" | b"tIME" | b"orNT" | b"iCCP"
    ) || is_png_colour_chunk(kind)
}

fn rewrite_png(output: Vec<u8>, metadata: &OutputMetadata<'_>) -> Result<Vec<u8>, FrameworkError> {
    let iccp = match metadata.icc {
        Some(profile) => {
            let mut data = b"ICC profile\0\0".to_vec();
            let compressed = compcol::vec::compress_to_vec::<compcol::zlib::Zlib>(profile)
                .map_err(|e| {
                    FrameworkError::internal(format!("image ICC profile compress failed: {e}"))
                })?;
            data.extend_from_slice(&compressed);
            Some(data)
        }
        None => None,
    };
    let mut out = Vec::with_capacity(output.len() + iccp.as_ref().map_or(0, Vec::len) + 128);
    out.extend_from_slice(&output[..8]);
    png_walk(&output, |chunk| {
        if drops_png_chunk(&chunk.kind) {
            return;
        }
        out.extend_from_slice(&output[chunk.whole.clone()]);
        if &chunk.kind == b"IHDR" {
            for (kind, data) in metadata.png_colour {
                push_png_chunk(&mut out, kind, data);
            }
            if let Some(data) = &iccp {
                push_png_chunk(&mut out, b"iCCP", data);
            }
            if let Some(orientation) = metadata.orientation {
                push_png_chunk(&mut out, b"eXIf", &orientation_only_exif(orientation));
            }
        }
    })
    .ok_or_else(|| unreadable("PNG"))?;
    Ok(out)
}

/// The canvas size and alpha of a WebP's bitstream, for a `VP8X` header
/// built around a simple-format file.
fn webp_bitstream_shape(
    bytes: &[u8],
    fourcc: &[u8; 4],
    payload: &std::ops::Range<usize>,
) -> Option<(u32, u32, bool)> {
    let data = bytes.get(payload.clone())?;
    match fourcc {
        b"VP8L" => {
            if *data.first()? != 0x2F {
                return None;
            }
            let bits = u32::from_le_bytes(data.get(1..5)?.try_into().ok()?);
            Some((
                (bits & 0x3FFF) + 1,
                ((bits >> 14) & 0x3FFF) + 1,
                bits & (1 << 28) != 0,
            ))
        }
        b"VP8 " => {
            let (width, height) = super::sniff::vp8_dimensions(data, 0)?;
            Some((width, height, false))
        }
        _ => None,
    }
}

fn rewrite_webp(output: Vec<u8>, metadata: &OutputMetadata<'_>) -> Result<Vec<u8>, FrameworkError> {
    const ICC: u8 = 0x20;
    const ALPHA: u8 = 0x10;
    const EXIF: u8 = 0x08;
    const XMP: u8 = 0x04;
    // The extended header, if the file has one, and the first bitstream.
    let mut extended = None;
    let mut bitstream = None;
    riff_walk(&output, |chunk| {
        if &chunk.fourcc == b"VP8X" && extended.is_none() {
            extended = Some(chunk.payload.clone());
        } else if matches!(&chunk.fourcc, b"VP8L" | b"VP8 ") && bitstream.is_none() {
            bitstream = Some((chunk.fourcc, chunk.payload.clone()));
        }
    })
    .ok_or_else(|| unreadable("WebP"))?;
    let exif = metadata.orientation.map(orientation_only_exif);
    if extended.is_none() && metadata.icc.is_none() && exif.is_none() {
        // A simple-format file holds no metadata chunk to strip.
        return Ok(output);
    }
    let (mut flags, canvas) = match extended {
        Some(payload) => {
            let data = output.get(payload).ok_or_else(|| unreadable("WebP"))?;
            let canvas: [u8; 6] = data
                .get(4..10)
                .and_then(|canvas| canvas.try_into().ok())
                .ok_or_else(|| unreadable("WebP"))?;
            (data[0], canvas)
        }
        None => {
            let (fourcc, payload) = bitstream.ok_or_else(|| unreadable("WebP"))?;
            let (width, height, alpha) = webp_bitstream_shape(&output, &fourcc, &payload)
                .filter(|(width, height, _)| *width > 0 && *height > 0)
                .ok_or_else(|| unreadable("WebP"))?;
            let mut canvas = [0u8; 6];
            canvas[..3].copy_from_slice(&(width - 1).to_le_bytes()[..3]);
            canvas[3..].copy_from_slice(&(height - 1).to_le_bytes()[..3]);
            (if alpha { ALPHA } else { 0 }, canvas)
        }
    };
    flags &= !(ICC | EXIF | XMP);
    if metadata.icc.is_some() {
        flags |= ICC;
    }
    if exif.is_some() {
        flags |= EXIF;
    }
    let mut out = Vec::with_capacity(output.len() + metadata.icc.map_or(0, <[u8]>::len) + 64);
    out.extend_from_slice(b"RIFF\0\0\0\0WEBP");
    let push = |out: &mut Vec<u8>, fourcc: &[u8; 4], data: &[u8]| {
        out.extend_from_slice(fourcc);
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(data);
        if data.len() % 2 == 1 {
            out.push(0);
        }
    };
    let mut header = [0u8; 10];
    header[0] = flags;
    header[4..].copy_from_slice(&canvas);
    push(&mut out, b"VP8X", &header);
    if let Some(profile) = metadata.icc {
        push(&mut out, b"ICCP", profile);
    }
    // The image itself, in file order: animation, alpha, bitstream, frames.
    // ICCP, EXIF, XMP and unknown chunks are what goes.
    riff_walk(&output, |chunk| {
        if matches!(
            &chunk.fourcc,
            b"ANIM" | b"ANMF" | b"ALPH" | b"VP8 " | b"VP8L"
        ) {
            out.extend_from_slice(&output[chunk.whole.clone()]);
        }
    })
    .ok_or_else(|| unreadable("WebP"))?;
    if let Some(exif) = &exif {
        push(&mut out, b"EXIF", exif);
    }
    let riff = u32::try_from(out.len() - 8)
        .map_err(|_| FrameworkError::internal("image encode produced a WebP over 4 GiB"))?;
    out[4..8].copy_from_slice(&riff.to_le_bytes());
    Ok(out)
}

/// Whether a GIF block describes how to draw the image, and stays: images,
/// graphic control (transparency, delay), plain text, and the looping
/// application extensions. Comments and every other application extension
/// (the ICC profile, XMP) are metadata.
fn keeps_gif_block(bytes: &[u8], block: &GifBlock) -> bool {
    match block {
        GifBlock::Image { .. } => true,
        GifBlock::Extension {
            label: 0xF9 | 0x01, ..
        } => true,
        GifBlock::Extension {
            label: 0xFF, first, ..
        } => bytes
            .get(first.clone())
            .is_some_and(|name| GIF_LOOP_APPLICATIONS.contains(&name)),
        GifBlock::Extension { .. } => false,
    }
}

fn rewrite_gif(output: Vec<u8>, metadata: &OutputMetadata<'_>) -> Result<Vec<u8>, FrameworkError> {
    // The profile the palette is in: the one the driver names, else one an
    // encoder wrote into the file.
    let written = if metadata.icc.is_none() {
        gif_icc(&output)
    } else {
        None
    };
    let conversion = metadata
        .icc
        .or(written.as_deref())
        .and_then(SrgbConversion::from_profile);
    let mut clean = true;
    gif_walk(&output, |block| clean &= keeps_gif_block(&output, block))
        .ok_or_else(|| unreadable("GIF"))?;
    if conversion.is_none() && clean {
        return Ok(output);
    }
    let mut out = Vec::with_capacity(output.len());
    let mut converted: Result<(), FrameworkError> = Ok(());
    let (global, trailer) = gif_walk(&output, |block| {
        if out.is_empty() {
            // The header, screen descriptor and global table, before the
            // first block.
            let first = match block {
                GifBlock::Image { whole, .. } | GifBlock::Extension { whole, .. } => whole.start,
            };
            out.extend_from_slice(&output[..first]);
        }
        if !keeps_gif_block(&output, block) {
            return;
        }
        match block {
            GifBlock::Image { table, whole } => {
                let start = out.len();
                out.extend_from_slice(&output[whole.clone()]);
                if let Some(conversion) = &conversion {
                    let local =
                        start + (table.start - whole.start)..start + (table.end - whole.start);
                    if converted.is_ok()
                        && let Err(error) = conversion.convert_rgb(&mut out[local])
                    {
                        converted = Err(error);
                    }
                }
            }
            GifBlock::Extension { whole, .. } => out.extend_from_slice(&output[whole.clone()]),
        }
    })
    .ok_or_else(|| unreadable("GIF"))?;
    converted?;
    if out.is_empty() {
        out.extend_from_slice(&output[..trailer]);
    }
    if let Some(conversion) = &conversion {
        conversion.convert_rgb(&mut out[global])?;
    }
    out.extend_from_slice(&output[trailer..]);
    Ok(out)
}

/// BMP holds no EXIF or text, only an embedded profile. One the driver did
/// not keep is removed: the header says sRGB, and the profile bytes, which
/// encoders write last, are cut off.
fn rewrite_bmp(
    mut output: Vec<u8>,
    metadata: &OutputMetadata<'_>,
) -> Result<Vec<u8>, FrameworkError> {
    const LCS_SRGB: u32 = 0x7352_4742;
    if metadata.icc.is_some() {
        return Ok(output);
    }
    let Some(range) = bmp_icc_range(&output) else {
        return Ok(output);
    };
    output[14 + 56..14 + 60].copy_from_slice(&LCS_SRGB.to_le_bytes());
    output[14 + 112..14 + 120].fill(0);
    if range.end == output.len() {
        output.truncate(range.start);
        let size = u32::try_from(output.len())
            .map_err(|_| FrameworkError::internal("image encode produced a BMP over 4 GiB"))?;
        output[2..6].copy_from_slice(&size.to_le_bytes());
    } else {
        output[range].fill(0);
    }
    Ok(output)
}

/// The colour a rotation's exposed corners take when the caller named none:
/// white where the output cannot hold transparency, transparent where it
/// can (IMG-006).
pub(crate) fn default_background(format: OutputFormat) -> Color {
    match format {
        OutputFormat::Jpeg | OutputFormat::Gif => Color::WHITE,
        OutputFormat::Png | OutputFormat::WebP | OutputFormat::WebPLossless | OutputFormat::Bmp => {
            Color::TRANSPARENT
        }
    }
}

/// Whether the output format drops alpha, so the pipeline flattens onto a
/// background before encoding.
pub(crate) fn flattens(format: OutputFormat) -> bool {
    matches!(format, OutputFormat::Jpeg | OutputFormat::Gif)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(class: &[u8; 4]) -> Vec<u8> {
        let mut profile = vec![0u8; 132];
        profile[..4].copy_from_slice(&132u32.to_be_bytes());
        profile[16..20].copy_from_slice(class);
        profile[36..40].copy_from_slice(b"acsp");
        profile
    }

    #[test]
    fn img_002_a_profile_names_its_colour_class() {
        assert_eq!(icc_class(&profile(b"RGB ")), Some(ColourClass::Rgb));
        assert_eq!(icc_class(&profile(b"GRAY")), Some(ColourClass::Gray));
        assert_eq!(icc_class(&profile(b"CMYK")), Some(ColourClass::Other));
        assert_eq!(icc_class(&[0u8; 64]), None);
        let mut unsigned = profile(b"RGB ");
        unsigned[36..40].copy_from_slice(b"xxxx");
        assert_eq!(icc_class(&unsigned), None);
    }

    #[test]
    fn img_002_jpeg_icc_chunks_join_in_sequence_order_or_not_at_all() {
        let segment = |sequence: u8, total: u8, data: &[u8]| {
            let mut out = Vec::new();
            push_jpeg_segment(&mut out, 0xE2, &[ICC_PROFILE, &[sequence, total], data]);
            out
        };
        let jpeg = |segments: &[Vec<u8>]| {
            let mut out = vec![0xFF, 0xD8];
            for segment in segments {
                out.extend_from_slice(segment);
            }
            out.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x02]);
            out
        };
        let ordered = jpeg(&[segment(2, 2, b"cd"), segment(1, 2, b"ab")]);
        assert_eq!(jpeg_icc(&ordered).as_deref(), Some(&b"abcd"[..]));
        let duplicate = jpeg(&[segment(1, 1, b"ab"), segment(1, 1, b"ab")]);
        assert_eq!(jpeg_icc(&duplicate), None);
        let missing = jpeg(&[segment(1, 2, b"ab")]);
        assert_eq!(jpeg_icc(&missing), None);
    }

    #[test]
    fn img_002_a_compressed_profile_is_refused_past_its_limit_before_it_is_kept() {
        let data = vec![7u8; 100_000];
        let compressed = compcol::vec::compress_to_vec::<compcol::zlib::Zlib>(&data).unwrap();
        assert_eq!(inflate_bounded(&compressed, 100_000).unwrap(), data);
        assert!(matches!(
            inflate_bounded(&compressed, 99_999),
            Err(compcol::Error::OutputLimitExceeded)
        ));
    }

    #[test]
    fn img_006_corners_default_to_what_the_format_can_hold() {
        assert_eq!(default_background(OutputFormat::Jpeg), Color::WHITE);
        assert_eq!(default_background(OutputFormat::Gif), Color::WHITE);
        for format in [
            OutputFormat::Png,
            OutputFormat::WebP,
            OutputFormat::WebPLossless,
            OutputFormat::Bmp,
        ] {
            assert_eq!(default_background(format), Color::TRANSPARENT, "{format:?}");
        }
    }
}
