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
//! The built-in driver's encoders write pixels only, and leave room in the
//! one buffer they fill, so the kept items are added in place. ImageMagick
//! writes whatever it read and adds more (`date:` text chunks, its own
//! orientation chunk, the comment), so the `magick` driver strips its output
//! in place and then adds the same items. Neither copies the encoded file
//! again (MEM-003). One set of rules serves both, so the drivers cannot
//! drift apart on what survives.
//!
//! Every reader here takes untrusted bytes: it never indexes past them,
//! allocates nothing to walk them, never inflates a compressed chunk it does
//! not need, and reads a profile's 128-byte header before it decides
//! whether the rest is worth holding.

use std::ops::Range;

use crate::error::FrameworkError;

use super::color::Color;
use super::driver::OutputFormat;
use super::orientation::{Orientation, orientation_only_exif};
use super::sniff::InputFormat;

/// What an inflate allocates besides its output: compcol's 32 KiB window,
/// its Huffman tables, and a 64 KiB scratch. The same figure the PNG decode
/// estimate uses.
pub(crate) const INFLATE_WORK: u64 = 128 * 1024;

/// The most profile bytes one JPEG `APP2` segment holds: 65,535 less the
/// length, the `ICC_PROFILE\0` tag and the two sequence bytes.
const JPEG_ICC_CHUNK: usize = 65_519;

/// The prefix of an APP2 segment that carries an ICC profile chunk.
const ICC_PROFILE: &[u8] = b"ICC_PROFILE\0";

/// The prefix of an APP1 segment that carries EXIF.
const EXIF_PREFIX: &[u8] = b"Exif\0\0";

/// The length of an ICC profile's header, which says what the rest is.
const ICC_HEADER: usize = 128;

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

/// What an ICC profile's header says: its colour space and its size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ProfileHeader {
    pub(crate) class: ColourClass,
    /// The profile's length in bytes, as its first four bytes give it.
    pub(crate) size: u64,
}

/// Read a profile header: 128 bytes, the `acsp` signature, a size no
/// shorter than the header itself.
fn read_header(bytes: &[u8]) -> Option<ProfileHeader> {
    let header = bytes.get(..ICC_HEADER)?;
    if header.get(36..40)? != b"acsp" {
        return None;
    }
    let size = u64::from(be_u32(header, 0)?);
    if size < ICC_HEADER as u64 {
        return None;
    }
    let class = match header.get(16..20)? {
        b"RGB " => ColourClass::Rgb,
        b"GRAY" => ColourClass::Gray,
        _ => ColourClass::Other,
    };
    Some(ProfileHeader { class, size })
}

/// The colour space of `profile` when it is an ICC profile: its header
/// reads, and the size the header gives is its own length.
///
/// The size check is what keeps a short valid header in front of a long run
/// of filler from passing for a profile, the shape a decompression bomb
/// takes inside a PNG `iCCP`.
pub(crate) fn icc_class(profile: &[u8]) -> Option<ColourClass> {
    let header = read_header(profile)?;
    (header.size == profile.len() as u64).then_some(header.class)
}

fn be_u16(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn be_u32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn le_u32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

/// The error for encoded output whose structure does not read.
fn unreadable(format: &str) -> FrameworkError {
    FrameworkError::internal(format!(
        "image encode produced a {format} whose structure could not be read to set its metadata"
    ))
}

// ───────────────────────── JPEG ─────────────────────────

/// One marker segment of a JPEG: the marker, where the segment starts (its
/// `0xFF`) and ends, and its payload after the two length bytes.
struct JpegSegment {
    marker: u8,
    start: usize,
    end: usize,
    body: Range<usize>,
}

/// One unit of a JPEG, in file order after its start-of-image marker.
enum JpegItem {
    /// A marker segment with a length, a start-of-scan header included.
    Segment(JpegSegment),
    /// A marker with no length (RSTn, a second SOI, TEM), or the
    /// entropy-coded data of a scan.
    Other(Range<usize>),
    /// The end-of-image marker.
    End(Range<usize>),
}

/// Where a JPEG walk stands.
struct JpegCursor {
    pos: usize,
    /// Inside entropy-coded data, after a start-of-scan header.
    in_scan: bool,
}

impl JpegCursor {
    /// The cursor after a JPEG's start-of-image marker, or `None` when the
    /// bytes do not open with one.
    fn open(bytes: &[u8]) -> Option<Self> {
        (bytes.get(..2)? == [0xFF, 0xD8]).then_some(Self {
            pos: 2,
            in_scan: false,
        })
    }

    /// The next unit, or `None` where the structure does not read.
    ///
    /// Scans are passed over to the next marker that is neither a stuffed
    /// `0xFF00` nor a restart marker, the rule JPEG decoders read by, so the
    /// walk sees the segments a progressive JPEG carries between its scans.
    /// Allocates nothing.
    fn next(&mut self, bytes: &[u8]) -> Option<JpegItem> {
        if self.in_scan {
            let start = self.pos;
            let mut at = start;
            loop {
                if *bytes.get(at)? == 0xFF {
                    let mut marker = at + 1;
                    while bytes.get(marker) == Some(&0xFF) {
                        marker += 1;
                    }
                    let value = *bytes.get(marker)?;
                    if value == 0x00 || (0xD0..=0xD7).contains(&value) {
                        at = marker + 1;
                        continue;
                    }
                    break;
                }
                at += 1;
            }
            self.in_scan = false;
            self.pos = at;
            if at > start {
                return Some(JpegItem::Other(start..at));
            }
        }
        let start = self.pos;
        if *bytes.get(start)? != 0xFF {
            return None;
        }
        let mut pos = start;
        while bytes.get(pos) == Some(&0xFF) {
            pos += 1;
        }
        let marker = *bytes.get(pos)?;
        pos += 1;
        self.pos = pos;
        match marker {
            0xD9 => return Some(JpegItem::End(start..pos)),
            0x01 | 0xD0..=0xD8 => return Some(JpegItem::Other(start..pos)),
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
        self.pos = end;
        self.in_scan = marker == 0xDA;
        Some(JpegItem::Segment(JpegSegment {
            marker,
            start,
            end,
            body: pos + 2..end,
        }))
    }
}

/// Visit the marker segments of a JPEG up to its first start-of-scan, and
/// return the offset of that scan's marker. `None` when the header does not
/// read.
fn jpeg_walk(bytes: &[u8], mut visit: impl FnMut(&JpegSegment)) -> Option<usize> {
    let mut cursor = JpegCursor::open(bytes)?;
    loop {
        match cursor.next(bytes)? {
            JpegItem::Segment(segment) if segment.marker == 0xDA => return Some(segment.start),
            JpegItem::Segment(segment) => visit(&segment),
            JpegItem::Other(_) => {}
            JpegItem::End(_) => return None,
        }
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
        let Some(body) = bytes.get(segment.body.clone()) else {
            corrupt = true;
            return;
        };
        if corrupt || segment.marker != 0xE2 || !body.starts_with(ICC_PROFILE) {
            return;
        }
        let (Some(&sequence), Some(&total), Some(data)) =
            (body.get(12), body.get(13), body.get(14..))
        else {
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
        *slot = Some(data);
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

/// The header of a JPEG's ICC profile: from its first chunk when that chunk
/// holds the whole header, as every writer's does, else from the joined
/// profile.
fn jpeg_icc_header(bytes: &[u8]) -> Option<ProfileHeader> {
    let mut first = None;
    jpeg_walk(bytes, |segment| {
        let body = bytes.get(segment.body.clone()).unwrap_or_default();
        if first.is_none()
            && segment.marker == 0xE2
            && body.starts_with(ICC_PROFILE)
            && body.get(12) == Some(&1)
        {
            first = body.get(14..);
        }
    })?;
    match first.and_then(read_header) {
        Some(header) => Some(header),
        None => read_header(&jpeg_icc(bytes)?),
    }
}

/// The TIFF bytes of the last Exif APP1 segment anywhere in a JPEG, the one
/// zune-jpeg keeps: it reads the segments a progressive JPEG carries between
/// its scans as well as those before the first. A file that ends early
/// keeps the last one read before it.
fn jpeg_exif(bytes: &[u8]) -> Option<&[u8]> {
    let mut cursor = JpegCursor::open(bytes)?;
    let mut exif = None;
    while let Some(item) = cursor.next(bytes) {
        match item {
            JpegItem::Segment(segment) if segment.marker == 0xE1 => {
                let body = bytes.get(segment.body)?;
                if body.len() > EXIF_PREFIX.len() && body.starts_with(EXIF_PREFIX) {
                    exif = body.get(EXIF_PREFIX.len()..);
                }
            }
            JpegItem::End(_) => break,
            JpegItem::Segment(_) | JpegItem::Other(_) => {}
        }
    }
    exif
}

// ───────────────────────── PNG ─────────────────────────

/// One chunk of a PNG: its type, the whole chunk (length to CRC) and its
/// data.
struct PngChunk {
    kind: [u8; 4],
    whole: Range<usize>,
    data: Range<usize>,
}

/// The PNG chunk at `pos`. CRCs are not checked: a chunk is copied whole,
/// CRC included, or dropped.
fn png_chunk_at(bytes: &[u8], pos: usize) -> Option<PngChunk> {
    let length = usize::try_from(be_u32(bytes, pos)?).ok()?;
    let kind: [u8; 4] = bytes.get(pos + 4..pos + 8)?.try_into().ok()?;
    let data = pos + 8..pos.checked_add(8)?.checked_add(length)?;
    let end = data.end.checked_add(4)?;
    (end <= bytes.len()).then_some(PngChunk {
        kind,
        whole: pos..end,
        data,
    })
}

const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

/// Visit the chunks of a PNG, signature to IEND. `None` when the chunk list
/// does not read. Allocates nothing.
fn png_walk(bytes: &[u8], mut visit: impl FnMut(&PngChunk)) -> Option<()> {
    if bytes.get(..8)? != PNG_SIGNATURE {
        return None;
    }
    let mut pos = 8;
    while pos < bytes.len() {
        let chunk = png_chunk_at(bytes, pos)?;
        visit(&chunk);
        pos = chunk.whole.end;
        if &chunk.kind == b"IEND" {
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

/// The defined length of a PNG colour chunk, which IMG-002 keeps with the
/// profile: `cHRM`, `gAMA`, `sRGB` and `cICP`.
fn png_colour_length(kind: &[u8; 4]) -> Option<usize> {
    match kind {
        b"cHRM" => Some(32),
        b"gAMA" | b"cICP" => Some(4),
        b"sRGB" => Some(1),
        _ => None,
    }
}

/// A PNG's colour chunks, each as its type and data, in file order. A chunk
/// of another length than its type defines is malformed and not kept, so
/// what is copied is never more than 41 bytes a chunk.
pub(crate) fn png_colour_chunks(bytes: &[u8]) -> Vec<([u8; 4], Vec<u8>)> {
    let mut found = Vec::new();
    png_walk(bytes, |chunk| {
        let Some(length) = png_colour_length(&chunk.kind) else {
            return;
        };
        if chunk.data.len() == length
            && found.len() < 4
            && let Some(data) = bytes.get(chunk.data.clone())
        {
            found.push((chunk.kind, data.to_vec()));
        }
    });
    found
}

/// The compressed profile in a PNG's `iCCP` chunk data: past the 1-79 byte
/// name, its NUL and the compression method, which must be zlib.
fn iccp_stream(data: &[u8]) -> Option<&[u8]> {
    let name_end = data.iter().take(80).position(|&byte| byte == 0)?;
    (data.get(name_end + 1) == Some(&0)).then(|| data.get(name_end + 2..))?
}

// ───────────────────────── inflating ─────────────────────────

/// The first `length` bytes a zlib stream inflates to, or fewer when it
/// ends sooner. Inflates no further.
fn inflate_prefix(data: &[u8], length: usize) -> Option<Vec<u8>> {
    use compcol::{Algorithm, Decoder, Status};

    let mut decoder = compcol::zlib::Zlib::decoder();
    let mut out = vec![0u8; length];
    let (mut consumed, mut written) = (0, 0);
    while written < length && consumed < data.len() {
        let (progress, status) = decoder
            .decode(data.get(consumed..)?, out.get_mut(written..)?)
            .ok()?;
        consumed += progress.consumed;
        written += progress.written;
        if status == Status::StreamEnd || (progress.consumed == 0 && progress.written == 0) {
            out.truncate(written);
            return Some(out);
        }
    }
    while written < length {
        let (progress, status) = decoder.finish(out.get_mut(written..)?).ok()?;
        written += progress.written;
        if status == Status::StreamEnd || progress.written == 0 {
            break;
        }
    }
    out.truncate(written);
    Some(out)
}

/// What inflating a zlib stream against a size found.
#[derive(Debug, PartialEq, Eq)]
enum Inflated {
    /// It inflates to exactly the size.
    Exactly,
    /// It inflates to more or fewer bytes, or does not inflate.
    Otherwise,
}

/// Where an inflate's next output goes: the rest of the sink, one byte past
/// it to tell a longer stream, or the scratch.
fn inflate_target<'b>(
    sink: &'b mut Option<&mut [u8]>,
    scratch: &'b mut [u8],
    overflow: &'b mut [u8; 1],
    written: u64,
) -> &'b mut [u8] {
    match sink.as_deref_mut() {
        Some(out) => match usize::try_from(written)
            .ok()
            .and_then(|at| out.get_mut(at..))
        {
            Some(rest) if !rest.is_empty() => rest,
            _ => overflow,
        },
        None => scratch,
    }
}

/// Inflate a zlib stream, keeping its output in `sink` when there is one
/// (the caller sized it to `size`), else writing it through a 64 KiB scratch
/// that is overwritten. Stops one byte past `size`, so a longer stream costs
/// no more than its size to tell.
///
/// The loop is `compcol::vec::decompress_to_vec_capped`'s: decode while
/// input remains, then finish.
fn inflate_to(data: &[u8], size: u64, mut sink: Option<&mut [u8]>) -> Inflated {
    use compcol::{Algorithm, Decoder, Status};

    let mut decoder =
        compcol::limit::LimitedDecoder::new(compcol::zlib::Zlib::decoder(), size.saturating_add(1));
    let mut scratch = if sink.is_some() {
        Vec::new()
    } else {
        vec![0u8; 64 * 1024]
    };
    let mut overflow = [0u8; 1];
    let (mut consumed, mut written) = (0usize, 0u64);
    let ended = |written: u64| {
        if written == size {
            Inflated::Exactly
        } else {
            Inflated::Otherwise
        }
    };
    while consumed < data.len() {
        let Some(input) = data.get(consumed..) else {
            return Inflated::Otherwise;
        };
        let out = inflate_target(&mut sink, &mut scratch, &mut overflow, written);
        let Ok((progress, status)) = decoder.decode(input, out) else {
            return Inflated::Otherwise;
        };
        consumed += progress.consumed;
        written += progress.written as u64;
        if written > size {
            return Inflated::Otherwise;
        }
        match status {
            Status::StreamEnd => return ended(written),
            Status::InputEmpty => break,
            Status::OutputFull if progress.consumed == 0 && progress.written == 0 => break,
            Status::OutputFull => {}
        }
    }
    loop {
        let out = inflate_target(&mut sink, &mut scratch, &mut overflow, written);
        let Ok((progress, status)) = decoder.finish(out) else {
            return Inflated::Otherwise;
        };
        written += progress.written as u64;
        if written > size {
            return Inflated::Otherwise;
        }
        if status == Status::StreamEnd {
            return ended(written);
        }
        if progress.written == 0 {
            return Inflated::Otherwise;
        }
    }
}

// ───────────────────────── WebP, BMP, GIF ─────────────────────────

/// One top-level chunk of a RIFF WebP: its fourcc, the whole chunk
/// (padding included) and its payload.
struct RiffChunk {
    fourcc: [u8; 4],
    whole: Range<usize>,
    payload: Range<usize>,
}

/// The RIFF chunk at `pos`.
fn riff_chunk_at(bytes: &[u8], pos: usize) -> Option<RiffChunk> {
    let fourcc: [u8; 4] = bytes.get(pos..pos + 4)?.try_into().ok()?;
    let size = usize::try_from(le_u32(bytes, pos + 4)?).ok()?;
    let payload = pos + 8..(pos + 8).checked_add(size)?;
    let end = payload.end.checked_add(size & 1)?.min(bytes.len());
    (payload.end <= bytes.len()).then_some(RiffChunk {
        fourcc,
        whole: pos..end,
        payload,
    })
}

/// Visit the top-level chunks of a RIFF WebP after its 12-byte header, at
/// most 4096 of them, the bound the header gate refuses past. Allocates
/// nothing.
fn riff_walk(bytes: &[u8], mut visit: impl FnMut(&RiffChunk)) -> Option<()> {
    if bytes.get(..4)? != b"RIFF" || bytes.get(8..12)? != b"WEBP" {
        return None;
    }
    let mut pos = 12;
    for _ in 0..4096 {
        if pos + 8 > bytes.len() {
            break;
        }
        let chunk = riff_chunk_at(bytes, pos)?;
        visit(&chunk);
        if chunk.whole.end <= pos {
            break;
        }
        pos = chunk.whole.end;
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

/// Where a V5 BMP's embedded profile sits: `bV5CSType` is
/// `PROFILE_EMBEDDED` (`MBED`), and the offset is from the start of the info
/// header.
fn bmp_icc_range(bytes: &[u8]) -> Option<Range<usize>> {
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
    /// application extension), and the whole block.
    Extension {
        label: u8,
        first: Range<usize>,
        whole: Range<usize>,
    },
    /// An image: its local colour table (empty when it has none) and the
    /// whole block.
    Image {
        table: Range<usize>,
        whole: Range<usize>,
    },
    /// The trailer, which ends the file.
    Trailer(usize),
}

/// The length of a colour table a GIF's packed field declares.
fn gif_table_len(packed: u8) -> usize {
    if packed & 0x80 != 0 {
        3usize << ((packed & 0x07) + 1)
    } else {
        0
    }
}

/// The position after a run of GIF sub-blocks starting at `pos`.
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

/// A GIF's global colour table, and the position of its first block.
fn gif_header(bytes: &[u8]) -> Option<Range<usize>> {
    if !matches!(bytes.get(..6)?, b"GIF87a" | b"GIF89a") {
        return None;
    }
    let global = 13..13 + gif_table_len(*bytes.get(10)?);
    (global.end <= bytes.len()).then_some(global)
}

/// The GIF block at `pos`.
fn gif_block_at(bytes: &[u8], pos: usize) -> Option<GifBlock> {
    let block = match *bytes.get(pos)? {
        0x3B => GifBlock::Trailer(pos),
        0x21 => {
            let label = *bytes.get(pos + 1)?;
            let size = usize::from(*bytes.get(pos + 2)?);
            let first = pos + 3..pos + 3 + size;
            let end = gif_sub_blocks_end(bytes, pos + 2)?;
            GifBlock::Extension {
                label,
                first,
                whole: pos..end,
            }
        }
        0x2C => {
            let table = pos + 10..pos + 10 + gif_table_len(*bytes.get(pos + 9)?);
            // The LZW minimum code size, then the data sub-blocks.
            let end = gif_sub_blocks_end(bytes, table.end + 1)?;
            GifBlock::Image {
                table,
                whole: pos..end,
            }
        }
        _ => return None,
    };
    let end = match &block {
        GifBlock::Extension { whole, .. } | GifBlock::Image { whole, .. } => whole.end,
        GifBlock::Trailer(at) => at + 1,
    };
    (end <= bytes.len()).then_some(block)
}

/// Visit a GIF's blocks, trailer included. Returns the global colour table.
/// Allocates nothing.
fn gif_walk(bytes: &[u8], mut visit: impl FnMut(&GifBlock)) -> Option<Range<usize>> {
    let global = gif_header(bytes)?;
    let mut pos = global.end;
    loop {
        let block = gif_block_at(bytes, pos)?;
        visit(&block);
        pos = match block {
            GifBlock::Extension { whole, .. } | GifBlock::Image { whole, .. } => whole.end,
            GifBlock::Trailer(_) => return Some(global),
        };
    }
}

/// The application identifier and code of the ICC profile extension.
const GIF_ICC_APPLICATION: &[u8] = b"ICCRGBG1012";
/// Application extensions that describe playback, which a GIF keeps.
const GIF_LOOP_APPLICATIONS: [&[u8]; 2] = [b"NETSCAPE2.0", b"ANIMEXTS1.0"];

/// Where a GIF's ICC profile sub-blocks run.
fn gif_icc_range(bytes: &[u8]) -> Option<Range<usize>> {
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
    found
}

/// Join GIF sub-blocks, up to `limit` bytes.
fn gif_join(bytes: &[u8], blocks: Range<usize>, limit: usize) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut pos = blocks.start;
    while pos < blocks.end && out.len() < limit {
        let size = usize::from(*bytes.get(pos)?);
        if size == 0 {
            break;
        }
        let data = bytes.get(pos + 1..pos + 1 + size)?;
        let take = data.len().min(limit - out.len());
        out.extend_from_slice(&data[..take]);
        pos += 1 + size;
    }
    Some(out)
}

// ───────────────────────── profiles in a file ─────────────────────────

/// Where a file's ICC profile sits.
enum ProfileSite<'a> {
    /// APP2 chunks of a JPEG, joined in sequence order.
    Jpeg,
    /// A PNG `iCCP`: its zlib stream, and the chunk's data as it stands.
    Png { stream: &'a [u8], chunk: &'a [u8] },
    /// Plain bytes: a WebP `ICCP`, a BMP's embedded profile.
    Plain(&'a [u8]),
    /// GIF sub-blocks after the ICC application extension.
    Gif(Range<usize>),
}

/// An ICC profile found in a file, of which only the header has been read.
pub(crate) struct FoundProfile<'a> {
    pub(crate) header: ProfileHeader,
    site: ProfileSite<'a>,
    bytes: &'a [u8],
}

/// Find the ICC profile in a file of `format` and read its header, and
/// nothing more: a PNG's profile is inflated only as far as its 128 header
/// bytes.
pub(crate) fn find_profile(format: InputFormat, bytes: &[u8]) -> Option<FoundProfile<'_>> {
    let (site, header) = match format {
        InputFormat::Jpeg => (ProfileSite::Jpeg, jpeg_icc_header(bytes)?),
        InputFormat::Png => {
            let chunk = png_chunk(bytes, b"iCCP")?;
            let stream = iccp_stream(chunk)?;
            let header = read_header(&inflate_prefix(stream, ICC_HEADER)?)?;
            (ProfileSite::Png { stream, chunk }, header)
        }
        InputFormat::WebP => {
            let profile = webp_chunk(bytes, b"ICCP")?;
            (ProfileSite::Plain(profile), read_header(profile)?)
        }
        InputFormat::Bmp => {
            let profile = bytes.get(bmp_icc_range(bytes)?)?;
            (ProfileSite::Plain(profile), read_header(profile)?)
        }
        InputFormat::Gif => {
            let blocks = gif_icc_range(bytes)?;
            let header = read_header(&gif_join(bytes, blocks.clone(), ICC_HEADER)?)?;
            (ProfileSite::Gif(blocks), header)
        }
    };
    Some(FoundProfile {
        header,
        site,
        bytes,
    })
}

/// [`find_profile`] for encoded output of `format`.
pub(crate) fn find_output_profile(format: OutputFormat, bytes: &[u8]) -> Option<FoundProfile<'_>> {
    find_profile(input_of(format), bytes)
}

impl FoundProfile<'_> {
    /// The whole profile, or `None` when it is not as long as its header
    /// says. A PNG's is inflated once, into a buffer of exactly that size,
    /// and no further than one byte past it; the caller has already charged
    /// that size to the budget.
    pub(crate) fn read(&self) -> Option<Vec<u8>> {
        let profile = match &self.site {
            ProfileSite::Jpeg => jpeg_icc(self.bytes)?,
            ProfileSite::Png { stream, .. } => {
                let mut out = vec![0u8; usize::try_from(self.header.size).ok()?];
                match inflate_to(stream, self.header.size, Some(&mut out)) {
                    Inflated::Exactly => out,
                    Inflated::Otherwise => return None,
                }
            }
            ProfileSite::Plain(profile) => profile.to_vec(),
            ProfileSite::Gif(blocks) => gif_join(self.bytes, blocks.clone(), usize::MAX)?,
        };
        (profile.len() as u64 == self.header.size).then_some(profile)
    }

    /// Whether the profile is as long as its header says, read without
    /// keeping it: a PNG's is inflated through a scratch buffer.
    pub(crate) fn is_whole(&self) -> bool {
        match &self.site {
            ProfileSite::Png { stream, .. } => {
                matches!(
                    inflate_to(stream, self.header.size, None),
                    Inflated::Exactly
                )
            }
            ProfileSite::Plain(profile) => profile.len() as u64 == self.header.size,
            ProfileSite::Jpeg | ProfileSite::Gif(_) => self.read().is_some(),
        }
    }

    /// A PNG source's `iCCP` chunk data as it stands, to carry into PNG
    /// output without inflating and compressing it again.
    pub(crate) fn png_chunk(&self) -> Option<&[u8]> {
        match &self.site {
            ProfileSite::Png { chunk, .. } => Some(chunk),
            _ => None,
        }
    }
}

// ───────────────────────── orientation ─────────────────────────

/// The EXIF orientation a source carries, without inflating anything: a
/// JPEG's last Exif APP1 segment, a PNG's uncompressed `eXIf` chunk, or a
/// WebP's `EXIF` chunk. BMP and GIF hold no EXIF.
///
/// Both drivers read the orientation here, so they turn a file alike. A PNG
/// that keeps its EXIF in a compressed text chunk, or ImageMagick's own
/// `orNT` chunk, has no orientation to either.
pub(crate) fn source_orientation(format: InputFormat, bytes: &[u8]) -> Option<Orientation> {
    let tiff = match format {
        InputFormat::Jpeg => jpeg_exif(bytes)?,
        InputFormat::Png => png_chunk(bytes, b"eXIf")?,
        InputFormat::WebP => webp_exif(bytes)?,
        InputFormat::Gif | InputFormat::Bmp => return None,
    };
    Orientation::from_tiff(tiff)
}

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

/// The EXIF orientation in encoded output, as an encoder wrote it.
pub(crate) fn output_orientation(format: OutputFormat, bytes: &[u8]) -> Option<Orientation> {
    source_orientation(input_of(format), bytes)
}

/// The orientation ImageMagick holds for an image, as it writes it into a
/// PNG: its own one-byte `orNT` chunk, or the `eXIf` chunk, whose tag it
/// writes alike. Only `orNT` is always there: a TIFF's tag lives in the
/// TIFF's own directory, so ImageMagick has no EXIF to write it in.
pub(crate) fn magick_png_orientation(bytes: &[u8]) -> Option<Orientation> {
    match png_chunk(bytes, b"orNT") {
        Some(&[tag]) => Orientation::from_tag(tag),
        _ => output_orientation(OutputFormat::Png, bytes),
    }
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
                    && matches!(
                        segment.marker,
                        0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF
                    )
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

/// Convert a GIF's colour tables, global and local, from `profile` to sRGB
/// in place: every pixel of a GIF is a table entry, so this converts the
/// image exactly, with no decode.
pub(crate) fn gif_to_srgb(gif: &mut [u8], profile: &[u8]) -> Result<(), FrameworkError> {
    let Some(conversion) = SrgbConversion::from_profile(profile) else {
        return Ok(());
    };
    let mut tables = Vec::new();
    let global = gif_walk(gif, |block| {
        if let GifBlock::Image { table, .. } = block
            && !table.is_empty()
        {
            tables.push(table.clone());
        }
    })
    .ok_or_else(|| unreadable("GIF"))?;
    tables.push(global);
    for table in tables {
        let colours = gif.get_mut(table).ok_or_else(|| unreadable("GIF"))?;
        conversion.convert_rgb(colours)?;
    }
    Ok(())
}

// ───────────────────────── adding to output ─────────────────────────

/// A profile to add to output.
pub(crate) enum IccData<'a> {
    /// The profile's bytes.
    Profile(&'a [u8]),
    /// A PNG source's `iCCP` chunk data, for PNG output.
    PngChunk(&'a [u8]),
}

/// What processed output carries besides its pixels, decided by the driver.
#[derive(Default)]
pub(crate) struct Kept<'a> {
    /// The ICC profile to add, already known to describe the output's
    /// pixels.
    pub(crate) icc: Option<IccData<'a>>,
    /// The orientation to keep as an Orientation-only EXIF, when
    /// orientation was not applied.
    pub(crate) orientation: Option<Orientation>,
    /// A PNG source's colour chunks to carry into PNG output.
    pub(crate) png_colour: &'a [([u8; 4], Vec<u8>)],
}

/// The bytes [`add`] inserts into output of one format: `front` after its
/// header, `back` at its end.
pub(crate) struct Additions {
    format: OutputFormat,
    front: Vec<u8>,
    back: Vec<u8>,
    profile: bool,
}

impl Additions {
    /// How many bytes adding these can grow the output by, for the encoder
    /// to leave room for: with it, adding them moves bytes within the one
    /// buffer and allocates nothing.
    pub(crate) fn len(&self) -> usize {
        // A WebP that has no extended header gains one, 18 bytes.
        let header = if matches!(self.format, OutputFormat::WebP | OutputFormat::WebPLossless) {
            18
        } else {
            0
        };
        self.front.len() + self.back.len() + header
    }
}

/// A JPEG segment: marker, length, parts.
fn push_jpeg_segment(out: &mut Vec<u8>, marker: u8, parts: &[&[u8]]) {
    let length: usize = 2 + parts.iter().map(|part| part.len()).sum::<usize>();
    out.extend_from_slice(&[0xFF, marker]);
    out.extend_from_slice(&(length as u16).to_be_bytes());
    for part in parts {
        out.extend_from_slice(part);
    }
}

/// A RIFF chunk: fourcc, size, payload, padding.
fn push_riff_chunk(out: &mut Vec<u8>, fourcc: &[u8; 4], data: &[u8]) -> Result<(), FrameworkError> {
    let size = u32::try_from(data.len())
        .map_err(|_| FrameworkError::internal("image metadata chunk over 4 GiB"))?;
    out.extend_from_slice(fourcc);
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(data);
    if data.len() % 2 == 1 {
        out.push(0);
    }
    Ok(())
}

impl Kept<'_> {
    /// Build the bytes adding these to output of `format` inserts. The
    /// driver sizes the encoder's buffer with [`Additions::len`] before it
    /// encodes.
    pub(crate) fn prepare(&self, format: OutputFormat) -> Result<Additions, FrameworkError> {
        let mut front = Vec::new();
        let mut back = Vec::new();
        let exif = self.orientation.map(orientation_only_exif);
        match format {
            OutputFormat::Jpeg => {
                if let Some(exif) = &exif {
                    push_jpeg_segment(&mut front, 0xE1, &[EXIF_PREFIX, exif]);
                }
                if let Some(icc) = &self.icc {
                    let profile = match icc {
                        IccData::Profile(profile) => *profile,
                        IccData::PngChunk(_) => {
                            return Err(FrameworkError::internal(
                                "image: a PNG iCCP chunk cannot go into a JPEG as it stands",
                            ));
                        }
                    };
                    let chunks = profile.chunks(JPEG_ICC_CHUNK);
                    let total = u8::try_from(chunks.len()).map_err(|_| {
                        FrameworkError::param(format!(
                            "image ICC profile is {} bytes, more than a JPEG can hold (255 \
                             segments of {JPEG_ICC_CHUNK} bytes); convert the output to PNG or \
                             WebP to keep it",
                            profile.len()
                        ))
                    })?;
                    for (index, chunk) in chunks.enumerate() {
                        push_jpeg_segment(
                            &mut front,
                            0xE2,
                            &[ICC_PROFILE, &[index as u8 + 1, total], chunk],
                        );
                    }
                }
            }
            OutputFormat::Png => {
                for (kind, data) in self.png_colour {
                    oxideav_png::chunk::write_chunk(&mut front, kind, data);
                }
                match &self.icc {
                    Some(IccData::PngChunk(chunk)) => {
                        oxideav_png::chunk::write_chunk(&mut front, b"iCCP", chunk);
                    }
                    Some(IccData::Profile(profile)) => {
                        let compressed = compcol::vec::compress_to_vec::<compcol::zlib::Zlib>(
                            profile,
                        )
                        .map_err(|e| {
                            FrameworkError::internal(format!(
                                "image ICC profile compress failed: {e}"
                            ))
                        })?;
                        let data = [&b"ICC profile\0\0"[..], &compressed].concat();
                        oxideav_png::chunk::write_chunk(&mut front, b"iCCP", &data);
                    }
                    None => {}
                }
                if let Some(exif) = &exif {
                    oxideav_png::chunk::write_chunk(&mut front, b"eXIf", exif);
                }
            }
            OutputFormat::WebP | OutputFormat::WebPLossless => {
                match &self.icc {
                    Some(IccData::Profile(profile)) => {
                        push_riff_chunk(&mut front, b"ICCP", profile)?;
                    }
                    Some(IccData::PngChunk(_)) => {
                        return Err(FrameworkError::internal(
                            "image: a PNG iCCP chunk cannot go into a WebP as it stands",
                        ));
                    }
                    None => {}
                }
                if let Some(exif) = &exif {
                    push_riff_chunk(&mut back, b"EXIF", exif)?;
                }
            }
            // A GIF holds no profile or EXIF here; a BMP's encoder embeds
            // the profile itself.
            OutputFormat::Gif | OutputFormat::Bmp => {}
        }
        Ok(Additions {
            format,
            front,
            back,
            profile: self.icc.is_some(),
        })
    }
}

/// Insert `bytes` at `at`, moving the rest of the buffer along within it.
fn insert(buffer: &mut Vec<u8>, at: usize, bytes: &[u8]) -> Result<(), FrameworkError> {
    if at > buffer.len() {
        return Err(FrameworkError::internal(
            "image metadata insert past the end of the output",
        ));
    }
    buffer.splice(at..at, bytes.iter().copied());
    Ok(())
}

/// Add the prepared metadata to encoded output, in place.
pub(crate) fn add(output: &mut Vec<u8>, additions: &Additions) -> Result<(), FrameworkError> {
    if additions.front.is_empty() && additions.back.is_empty() {
        return Ok(());
    }
    match additions.format {
        OutputFormat::Jpeg => {
            // After the JFIF segment when the encoder wrote one first, so
            // it stays the first, else after the start-of-image marker.
            let mut cursor = JpegCursor::open(output).ok_or_else(|| unreadable("JPEG"))?;
            let at = match cursor.next(output) {
                Some(JpegItem::Segment(segment))
                    if segment.marker == 0xE0
                        && output
                            .get(segment.body.clone())
                            .is_some_and(|body| body.starts_with(b"JFIF\0")) =>
                {
                    segment.end
                }
                _ => 2,
            };
            insert(output, at, &additions.front)
        }
        OutputFormat::Png => {
            // After IHDR, which must be the first chunk.
            let ihdr = png_chunk_at(output, PNG_SIGNATURE.len())
                .filter(|chunk| &chunk.kind == b"IHDR")
                .ok_or_else(|| unreadable("PNG"))?;
            if output.get(..8) != Some(PNG_SIGNATURE) {
                return Err(unreadable("PNG"));
            }
            insert(output, ihdr.whole.end, &additions.front)
        }
        OutputFormat::WebP | OutputFormat::WebPLossless => add_to_webp(output, additions),
        OutputFormat::Gif | OutputFormat::Bmp => Ok(()),
    }
}

/// The canvas size and alpha of a WebP's bitstream, for a `VP8X` header
/// built around a simple-format file.
fn webp_bitstream_shape(bytes: &[u8], chunk: &RiffChunk) -> Option<(u32, u32, bool)> {
    let data = bytes.get(chunk.payload.clone())?;
    let (width, height, alpha) = match &chunk.fourcc {
        b"VP8L" => {
            if *data.first()? != 0x2F {
                return None;
            }
            let bits = le_u32(data, 1)?;
            (
                (bits & 0x3FFF) + 1,
                ((bits >> 14) & 0x3FFF) + 1,
                bits & (1 << 28) != 0,
            )
        }
        b"VP8 " => {
            let (width, height) = super::sniff::vp8_dimensions(data, 0)?;
            (width, height, false)
        }
        _ => return None,
    };
    (width > 0 && height > 0).then_some((width, height, alpha))
}

/// The `VP8X` flags for an ICC profile, alpha, EXIF and XMP.
const WEBP_ICC: u8 = 0x20;
const WEBP_ALPHA: u8 = 0x10;
const WEBP_EXIF: u8 = 0x08;
const WEBP_XMP: u8 = 0x04;

/// Set a WebP's RIFF size from its length.
fn set_riff_size(webp: &mut [u8]) -> Result<(), FrameworkError> {
    let size = u32::try_from(webp.len().saturating_sub(8))
        .map_err(|_| FrameworkError::internal("image encode produced a WebP over 4 GiB"))?;
    webp.get_mut(4..8)
        .ok_or_else(|| unreadable("WebP"))?
        .copy_from_slice(&size.to_le_bytes());
    Ok(())
}

fn add_to_webp(webp: &mut Vec<u8>, additions: &Additions) -> Result<(), FrameworkError> {
    let first = riff_chunk_at(webp, 12)
        .filter(|_| webp.get(..4) == Some(b"RIFF") && webp.get(8..12) == Some(b"WEBP"))
        .ok_or_else(|| unreadable("WebP"))?;
    let exif = !additions.back.is_empty();
    if &first.fourcc == b"VP8X" {
        let flags = webp
            .get_mut(first.payload.start)
            .ok_or_else(|| unreadable("WebP"))?;
        if additions.profile {
            *flags |= WEBP_ICC;
        }
        if exif {
            *flags |= WEBP_EXIF;
        }
        insert(webp, first.whole.end, &additions.front)?;
    } else {
        let (width, height, alpha) =
            webp_bitstream_shape(webp, &first).ok_or_else(|| unreadable("WebP"))?;
        let mut header = [0u8; 10];
        header[0] = if alpha { WEBP_ALPHA } else { 0 }
            | if additions.profile { WEBP_ICC } else { 0 }
            | if exif { WEBP_EXIF } else { 0 };
        header[4..7].copy_from_slice(&(width - 1).to_le_bytes()[..3]);
        header[7..10].copy_from_slice(&(height - 1).to_le_bytes()[..3]);
        let mut front = Vec::with_capacity(18 + additions.front.len());
        push_riff_chunk(&mut front, b"VP8X", &header)?;
        front.extend_from_slice(&additions.front);
        insert(webp, 12, &front)?;
    }
    webp.extend_from_slice(&additions.back);
    set_riff_size(webp)
}

// ───────────────────────── stripping output ─────────────────────────

/// Keep, of a JPEG's segments, the ones that describe how to decode it: the
/// frame, scans, tables and restart interval, JFIF's `APP0` and Adobe's
/// `APP14` (its colour transform), and the ICC profile when `keep_icc`.
/// Every other `APPn` and every comment is metadata and goes.
fn keeps_jpeg_segment(marker: u8, body: &[u8], keep_icc: bool) -> bool {
    match marker {
        0xE0 => body.starts_with(b"JFIF\0"),
        0xE2 => keep_icc && body.starts_with(ICC_PROFILE),
        0xEE => body.starts_with(b"Adobe"),
        0xE1..=0xEF | 0xFE => false,
        _ => true,
    }
}

/// The PNG chunks IMG-002 strips: EXIF, text, the time, ImageMagick's own
/// orientation chunk and its older EXIF chunk names, and the colour chunks,
/// which come back from the source; the profile too unless it is kept.
fn drops_png_chunk(kind: &[u8; 4], keep_icc: bool) -> bool {
    matches!(
        kind,
        b"eXIf" | b"exIf" | b"zxIf" | b"tEXt" | b"zTXt" | b"iTXt" | b"tIME" | b"orNT"
    ) || (kind == b"iCCP" && !keep_icc)
        || png_colour_length(kind).is_some()
}

/// Whether a GIF block describes how to draw the image, and stays: images,
/// graphic control, plain text, the looping application extensions, the
/// trailer. Comments and every other application extension (the ICC
/// profile, XMP) are metadata.
fn keeps_gif_block(bytes: &[u8], block: &GifBlock) -> bool {
    match block {
        GifBlock::Image { .. } | GifBlock::Trailer(_) => true,
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

/// Move the bytes of `range` down to `write` and return where writing goes
/// on. `write` is never past `range.start`, so nothing yet unread is
/// overwritten.
fn keep(buffer: &mut [u8], range: Range<usize>, write: usize) -> usize {
    let length = range.len();
    if range.start != write {
        buffer.copy_within(range, write);
    }
    write + length
}

/// Strip everything IMG-002 drops from ImageMagick's output, in place: the
/// buffer only shrinks, and nothing is copied out of it. `keep_icc` keeps
/// the ICC profile as ImageMagick wrote it.
pub(crate) fn strip(
    output: &mut Vec<u8>,
    format: OutputFormat,
    keep_icc: bool,
) -> Result<(), FrameworkError> {
    let end = match format {
        OutputFormat::Jpeg => strip_jpeg(output, keep_icc)?,
        OutputFormat::Png => strip_png(output, keep_icc)?,
        OutputFormat::WebP | OutputFormat::WebPLossless => strip_webp(output, keep_icc)?,
        OutputFormat::Gif => strip_gif(output)?,
        OutputFormat::Bmp => {
            if !keep_icc {
                strip_bmp_profile(output)?;
            }
            output.len()
        }
    };
    output.truncate(end);
    if matches!(format, OutputFormat::WebP | OutputFormat::WebPLossless) {
        set_riff_size(output)?;
    }
    Ok(())
}

/// Through every scan to the end-of-image marker; what follows it goes.
fn strip_jpeg(output: &mut [u8], keep_icc: bool) -> Result<usize, FrameworkError> {
    let mut cursor = JpegCursor::open(output).ok_or_else(|| unreadable("JPEG"))?;
    let mut write = 2;
    loop {
        let item = cursor.next(output).ok_or_else(|| unreadable("JPEG"))?;
        match item {
            JpegItem::Segment(segment) => {
                let body = output
                    .get(segment.body.clone())
                    .ok_or_else(|| unreadable("JPEG"))?;
                if keeps_jpeg_segment(segment.marker, body, keep_icc) {
                    write = keep(output, segment.start..segment.end, write);
                }
            }
            JpegItem::Other(range) => write = keep(output, range, write),
            JpegItem::End(range) => return Ok(keep(output, range, write)),
        }
    }
}

/// To IEND; what follows it goes.
fn strip_png(output: &mut [u8], keep_icc: bool) -> Result<usize, FrameworkError> {
    if output.get(..8) != Some(PNG_SIGNATURE) {
        return Err(unreadable("PNG"));
    }
    let (mut read, mut write) = (8, 8);
    while read < output.len() {
        let chunk = png_chunk_at(output, read).ok_or_else(|| unreadable("PNG"))?;
        read = chunk.whole.end;
        if !drops_png_chunk(&chunk.kind, keep_icc) {
            write = keep(output, chunk.whole, write);
        }
        if &chunk.kind == b"IEND" {
            break;
        }
    }
    Ok(write)
}

/// Keeps the image chunks and, when `keep_icc`, the profile; the extended
/// header's flags then say what is left. The RIFF size is set by the caller
/// once the buffer is cut to length.
fn strip_webp(output: &mut [u8], keep_icc: bool) -> Result<usize, FrameworkError> {
    if output.get(..4) != Some(b"RIFF") || output.get(8..12) != Some(b"WEBP") {
        return Err(unreadable("WebP"));
    }
    let (mut read, mut write) = (12, 12);
    let mut header = None;
    let mut profile = false;
    for _ in 0..4096 {
        if read + 8 > output.len() {
            break;
        }
        let chunk = riff_chunk_at(output, read).ok_or_else(|| unreadable("WebP"))?;
        read = chunk.whole.end;
        let kept = match &chunk.fourcc {
            b"VP8X" | b"ANIM" | b"ANMF" | b"ALPH" | b"VP8 " | b"VP8L" => true,
            b"ICCP" => keep_icc,
            _ => false,
        };
        if kept {
            if &chunk.fourcc == b"VP8X" {
                header = Some(write + 8);
            }
            profile |= &chunk.fourcc == b"ICCP";
            write = keep(output, chunk.whole, write);
        }
    }
    if let Some(flags) = header {
        let flags = output.get_mut(flags).ok_or_else(|| unreadable("WebP"))?;
        *flags &= !(WEBP_ICC | WEBP_EXIF | WEBP_XMP);
        if profile {
            *flags |= WEBP_ICC;
        }
    }
    Ok(write)
}

/// To the trailer; what follows it goes.
fn strip_gif(output: &mut [u8]) -> Result<usize, FrameworkError> {
    let global = gif_header(output).ok_or_else(|| unreadable("GIF"))?;
    let (mut read, mut write) = (global.end, global.end);
    loop {
        let block = gif_block_at(output, read).ok_or_else(|| unreadable("GIF"))?;
        let keeps = keeps_gif_block(output, &block);
        let range = match block {
            GifBlock::Extension { whole, .. } | GifBlock::Image { whole, .. } => whole,
            GifBlock::Trailer(at) => return Ok(keep(output, at..at + 1, write)),
        };
        read = range.end;
        if keeps {
            write = keep(output, range, write);
        }
    }
}

/// Remove a BMP's embedded profile: the header says sRGB, and the profile
/// bytes, which encoders write last, are cut off or blanked.
fn strip_bmp_profile(output: &mut Vec<u8>) -> Result<(), FrameworkError> {
    const LCS_SRGB: u32 = 0x7352_4742;
    let Some(range) = bmp_icc_range(output) else {
        return Ok(());
    };
    output
        .get_mut(14 + 56..14 + 60)
        .ok_or_else(|| unreadable("BMP"))?
        .copy_from_slice(&LCS_SRGB.to_le_bytes());
    output
        .get_mut(14 + 112..14 + 120)
        .ok_or_else(|| unreadable("BMP"))?
        .fill(0);
    if range.end == output.len() {
        output.truncate(range.start);
        let size = u32::try_from(output.len())
            .map_err(|_| FrameworkError::internal("image encode produced a BMP over 4 GiB"))?;
        output
            .get_mut(2..6)
            .ok_or_else(|| unreadable("BMP"))?
            .copy_from_slice(&size.to_le_bytes());
    } else if let Some(profile) = output.get_mut(range) {
        profile.fill(0);
    }
    Ok(())
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
    fn img_002_a_profile_is_only_as_long_as_its_header_says() {
        let mut longer = profile(b"RGB ");
        longer.extend_from_slice(&[0; 4]);
        assert_eq!(icc_class(&longer), None, "longer than its header says");
        let mut shorter = profile(b"RGB ");
        shorter[..4].copy_from_slice(&200u32.to_be_bytes());
        assert_eq!(icc_class(&shorter), None, "shorter than its header says");
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
    fn img_002_an_inflate_stops_one_byte_past_the_size_it_checks() {
        let data = vec![7u8; 100_000];
        let compressed = compcol::vec::compress_to_vec::<compcol::zlib::Zlib>(&data).unwrap();
        assert_eq!(inflate_to(&compressed, 100_000, None), Inflated::Exactly);
        assert_eq!(inflate_to(&compressed, 99_999, None), Inflated::Otherwise);
        assert_eq!(inflate_to(&compressed, 100_001, None), Inflated::Otherwise);
        let mut out = vec![0u8; 100_000];
        assert_eq!(
            inflate_to(&compressed, 100_000, Some(&mut out)),
            Inflated::Exactly
        );
        assert_eq!(out, data);
        assert_eq!(inflate_prefix(&compressed, 128), Some(vec![7u8; 128]));
        assert_eq!(inflate_to(b"not zlib", 8, None), Inflated::Otherwise);
    }

    #[test]
    fn img_001_a_tag_between_scans_is_the_last_one() {
        let exif = |tag: u8| {
            let mut out = Vec::new();
            let tiff = orientation_only_exif(Orientation::from_tag(tag).unwrap());
            push_jpeg_segment(&mut out, 0xE1, &[EXIF_PREFIX, &tiff]);
            out
        };
        let mut jpeg = vec![0xFF, 0xD8];
        jpeg.extend_from_slice(&exif(3));
        // A scan header, entropy data with a stuffed byte and a restart
        // marker, a tag between scans, a second scan, the end.
        jpeg.extend_from_slice(&[
            0xFF, 0xDA, 0x00, 0x02, 0x12, 0xFF, 0x00, 0x34, 0xFF, 0xD0, 0x56,
        ]);
        jpeg.extend_from_slice(&exif(6));
        jpeg.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x02, 0x78, 0xFF, 0xD9]);
        jpeg.extend_from_slice(&exif(8));
        let tiff = jpeg_exif(&jpeg).unwrap();
        assert_eq!(
            Orientation::from_tiff(tiff),
            Orientation::from_tag(6),
            "after EOI is not read"
        );
    }

    #[test]
    fn img_002_short_or_broken_output_is_an_error_not_a_panic() {
        let kept = Kept {
            orientation: Orientation::from_tag(6),
            ..Kept::default()
        };
        for format in [
            OutputFormat::Jpeg,
            OutputFormat::Png,
            OutputFormat::WebP,
            OutputFormat::Gif,
            OutputFormat::Bmp,
        ] {
            for bytes in [
                &b""[..],
                b"\x89PNG\r",
                b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR",
                b"\xff\xd8\xff",
                b"\xff\xd8\xff\xe0\xff\xff",
                b"RIFF\0\0\0\0WEBPVP8X",
                b"RIFF\0\0\0\0WEBPVP8L\x05\0\0\0\x2f",
                b"GIF89a",
                b"BM",
            ] {
                let result = std::panic::catch_unwind(|| {
                    let mut stripped = bytes.to_vec();
                    let _ = strip(&mut stripped, format, false);
                    let mut added = bytes.to_vec();
                    if let Ok(additions) = kept.prepare(format) {
                        let _ = add(&mut added, &additions);
                    }
                    let mut gif = bytes.to_vec();
                    let _ = gif_to_srgb(&mut gif, b"");
                    let _ = find_profile(input_of(format), bytes).map(|found| found.is_whole());
                });
                assert!(result.is_ok(), "{format:?} panicked on {bytes:?}");
            }
        }
    }

    #[test]
    fn img_002_a_colour_chunk_is_kept_only_at_its_defined_length() {
        let mut png = PNG_SIGNATURE.to_vec();
        oxideav_png::chunk::write_chunk(
            &mut png,
            b"IHDR",
            &[0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0],
        );
        oxideav_png::chunk::write_chunk(&mut png, b"gAMA", &vec![0u8; 1 << 20]);
        oxideav_png::chunk::write_chunk(&mut png, b"sRGB", &[0]);
        oxideav_png::chunk::write_chunk(&mut png, b"IEND", &[]);
        assert_eq!(png_colour_chunks(&png), vec![(*b"sRGB", vec![0])]);
    }

    /// The metadata goes into the buffer the encoder left room in: the
    /// buffer is neither reallocated nor copied (MEM-003).
    #[test]
    fn img_002_adding_metadata_moves_bytes_within_the_reserved_buffer() {
        let mut profile = profile(b"RGB ");
        profile.resize(4096, 0);
        profile[..4].copy_from_slice(&4096u32.to_be_bytes());
        // A simple lossless WebP, 1x1, and a JPEG and PNG header in front
        // of some body bytes.
        let mut webp = b"RIFF\0\0\0\0WEBPVP8L\x05\0\0\0\x2f\0\0\0\0\0".to_vec();
        set_riff_size(&mut webp).unwrap();
        let mut jpeg = vec![0xFF, 0xD8];
        push_jpeg_segment(&mut jpeg, 0xE0, &[b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0"]);
        jpeg.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x02, 0x00, 0xFF, 0xD9]);
        let mut png = PNG_SIGNATURE.to_vec();
        oxideav_png::chunk::write_chunk(
            &mut png,
            b"IHDR",
            &[0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0],
        );
        oxideav_png::chunk::write_chunk(&mut png, b"IEND", &[]);
        for (format, encoded) in [
            (OutputFormat::WebPLossless, webp),
            (OutputFormat::Jpeg, jpeg),
            (OutputFormat::Png, png),
        ] {
            let kept = Kept {
                icc: Some(IccData::Profile(&profile)),
                orientation: Orientation::from_tag(6),
                png_colour: &[],
            };
            let additions = kept.prepare(format).unwrap();
            let mut output = Vec::with_capacity(encoded.len() + additions.len());
            output.extend_from_slice(&encoded);
            let (start, capacity) = (output.as_ptr(), output.capacity());
            add(&mut output, &additions).unwrap();
            assert_eq!(output.as_ptr(), start, "{format:?}: the buffer moved");
            assert_eq!(output.capacity(), capacity, "{format:?}: the buffer grew");
            let found = find_output_profile(format, &output).expect("the profile was added");
            assert_eq!(found.read().as_deref(), Some(&profile[..]), "{format:?}");
            assert_eq!(
                output_orientation(format, &output),
                Orientation::from_tag(6),
                "{format:?}"
            );
        }
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
