//! What oxideav-webp allocates to decode a WebP, read from the file before
//! it decodes.
//!
//! Most of a WebP decode scales with the image's declared size, which the
//! header gate has already read. A lossless (`VP8L`) bitstream adds one cost
//! no header declares: its prefix-code tables. The bitstream codes its pixels
//! with groups of five prefix codes, and the decoder builds every group
//! before it decodes a pixel. How many groups there are is written inside
//! the compressed data: an entropy image, itself compressed, names a group
//! for each block of the image, and the decoder reads as many groups as the
//! largest name plus one, up to 65,536. Each group keeps a byte per symbol of
//! its five alphabets, and an 11-bit color cache widens the first alphabet to
//! 2,328 symbols. A 4x4 image can make the decoder build 250 MB of tables
//! from 160 KiB of input. A lossless-coded alpha plane (`ALPH`) is the same
//! bitstream without its header.
//!
//! The walk here reads each bitstream the way the decoder does, with the
//! decoder's own bit reader and prefix-code reader, up to the last group:
//! the transforms and their sub-images, the color cache, the entropy image,
//! and every group. It stops before the image's own pixels. It keeps no
//! pixels: it decodes a sub-image's pixels only far enough to know where the
//! next field starts, and the entropy image's only far enough to know the
//! largest group it names. It holds one group at a time, so what it
//! allocates does not grow with anything the file declares: at most about
//! 17 KiB for a group, and the container's chunk list, which the header
//! gate has already bounded. Its work is one pass over the bits it reads,
//! one step a sub-image pixel, and building each group's tables: the same
//! steps the decode takes before it decodes a pixel.
//!
//! The chunks are picked by the decoder's own container parser, so the walk
//! measures the bitstream the decoder decodes: the first top-level `VP8L`
//! chunk, or failing that the first `VP8 `, with the first `ALPH` beside
//! either. A bitstream the walk cannot read is refused, because the decoder
//! fails at the same field.

use std::fmt::Display;

use oxideav_webp::alph::{AlphCompression, AlphFiltering, AlphHeader};
use oxideav_webp::container::{self, fourcc};
use oxideav_webp::meta_prefix::{ImageRole, MetaPrefixCodes, MetaPrefixHeader, PrefixCodeGroup};
use oxideav_webp::vp8l_chunk;
use oxideav_webp::vp8l_decode::{GreenSymbol, distance_code_to_pixel_distance, read_lz77_value};
use oxideav_webp::vp8l_prefix::PrefixCode;
use oxideav_webp::vp8l_stream::{BitReader, TransformType};
use oxideav_webp::vp8l_transform::color_indexing_width_bits;

use crate::error::FrameworkError;
use crate::media::sniff;

/// The decoder reserves a lossless image's pixels up front only up to this
/// many; past it the buffer grows by doubling.
const EAGER_PIXELS: u64 = 1 << 22;

/// One group of five prefix codes, as the decoder stores it inline in its
/// list of groups or in the box of a single-group image.
const GROUP: u64 = std::mem::size_of::<PrefixCodeGroup>() as u64;

/// One row of a prefix code's canonical decoding table: a `u8`, a `u32` and
/// two `usize`, padded to 24 bytes.
const LENGTH_ROW: u64 = 24;

/// A prefix code's map from code length to table row: sixteen bytes.
const LENGTH_TO_ROW: u64 = 16;

/// The 8-bit lookup table a prefix code builds when it has at least 32
/// symbols: 256 `u32` entries.
const LOOKUP: u64 = 256 * 4;

/// The fewest symbols a prefix code builds its lookup table for.
const LOOKUP_SYMBOLS: usize = 32;

/// What `decode_webp_image` will decode, and what each part allocates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WebpPlan {
    /// The first top-level `VP8L` chunk, and an alpha plane beside it when
    /// there is an `ALPH` chunk.
    Lossless {
        width: u32,
        height: u32,
        image: Lossless,
        alpha: Option<u64>,
    },
    /// No `VP8L` chunk: the first top-level `VP8 ` chunk, and the most
    /// bytes its alpha plane holds while it decodes, if there is one.
    Lossy { alpha: Option<u64> },
    /// Neither bitstream at the top level (an animation keeps its frames
    /// inside `ANMF` chunks): the decoder fails before it allocates.
    Neither,
}

/// What decoding one `VP8L` bitstream allocates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Lossless {
    /// The most bytes alive at once while it decodes.
    pub(super) peak: u64,
    /// What the decoded image holds once the decode returns.
    pub(super) image: u64,
}

fn failed(error: impl Display) -> FrameworkError {
    FrameworkError::param(format!("image decode failed: image/webp: {error}"))
}

/// Read what decoding `contents` allocates, before it decodes.
pub(super) fn plan(contents: &[u8]) -> Result<WebpPlan, FrameworkError> {
    let container = container::parse(contents).map_err(failed)?;
    let alpha = container
        .first_chunk_with_fourcc(fourcc::ALPH)
        .map(|chunk| chunk.payload(contents));
    if let Some(chunk) = vp8l_chunk::extract_lossless(contents, &container).map_err(failed)? {
        let (width, height) = (chunk.width(), chunk.height());
        let image = lossless(
            BitReader::new_after_image_header(chunk.bitstream()),
            width,
            height,
        )?;
        let alpha = alpha
            .map(|payload| alpha_plane(payload, width, height))
            .transpose()?;
        return Ok(WebpPlan::Lossless {
            width,
            height,
            image,
            alpha,
        });
    }
    let Some(vp8) = container.first_chunk_with_fourcc(fourcc::VP8) else {
        return Ok(WebpPlan::Neither);
    };
    // The alpha plane takes the lossy frame's size. A frame whose header
    // does not read fails to decode before its alpha plane is reached.
    let alpha = match (alpha, sniff::vp8_dimensions(vp8.payload(contents), 0)) {
        (Some(payload), Some((width, height))) => Some(alpha_plane(payload, width, height)?),
        _ => None,
    };
    Ok(WebpPlan::Lossy { alpha })
}

/// The most bytes `alph::decode_alpha` holds for a `width x height` plane.
///
/// The plane is decompressed into one buffer and, unless it is unfiltered,
/// inverse-filtered into a second. A lossless plane is a headerless `VP8L`
/// image whose green channel is collected into the first buffer while the
/// decoded image is still alive.
fn alpha_plane(payload: &[u8], width: u32, height: u32) -> Result<u64, FrameworkError> {
    let header = AlphHeader::parse(payload).map_err(failed)?;
    let plane = u64::from(width) * u64::from(height);
    let filtering = if header.filtering == AlphFiltering::None {
        plane
    } else {
        plane.saturating_mul(2)
    };
    let decompressing = match header.compression {
        AlphCompression::None => plane,
        AlphCompression::Lossless => {
            let bitstream = payload.get(1..).unwrap_or_default();
            let image = lossless(BitReader::new(bitstream), width, height)?;
            image.peak.max(image.image.saturating_add(plane))
        }
        AlphCompression::Reserved(method) => {
            return Err(failed(format_args!(
                "alpha compression method {method} is reserved"
            )));
        }
    };
    Ok(decompressing.max(filtering))
}

/// Bytes the decoder's pixel buffer holds for `pixels` pixels: reserved
/// exactly up to [`EAGER_PIXELS`], grown by doubling past it.
fn pixel_buffer(pixels: u64) -> u64 {
    let capacity = if pixels <= EAGER_PIXELS {
        pixels
    } else {
        pixels.checked_next_power_of_two().unwrap_or(u64::MAX)
    };
    capacity.saturating_mul(4)
}

/// What one prefix code keeps on the heap, as `PrefixCode::from_code_lengths`
/// builds it: a byte a symbol of its alphabet, and for a code of more than
/// one symbol, its table rows, the symbols sorted by code, the row map and,
/// from 32 symbols, the lookup table.
fn code_bytes(code: &PrefixCode) -> u64 {
    let lengths = code.code_lengths();
    let alphabet = lengths.len() as u64;
    if code.single_symbol().is_some() {
        return alphabet + 2;
    }
    let mut lengths_used = [false; 16];
    let mut symbols = 0usize;
    for &length in lengths.iter().filter(|length| **length != 0) {
        symbols += 1;
        if let Some(used) = lengths_used.get_mut(usize::from(length)) {
            *used = true;
        }
    }
    // The rows are pushed one at a time: capacity 4, then 8, then 16.
    let rows = match lengths_used.iter().filter(|used| **used).count() {
        0..=4 => 4,
        5..=8 => 8,
        _ => 16,
    };
    let lookup = if symbols >= LOOKUP_SYMBOLS { LOOKUP } else { 0 };
    alphabet + rows * LENGTH_ROW + symbols as u64 * 2 + LENGTH_TO_ROW + lookup
}

/// What a group's five codes keep on the heap, beside the group itself.
fn group_bytes(group: &PrefixCodeGroup) -> u64 {
    [
        &group.green,
        &group.red,
        &group.blue,
        &group.alpha,
        &group.distance,
    ]
    .into_iter()
    .map(code_bytes)
    .sum()
}

/// What decoding one `width x height` `VP8L` image allocates.
///
/// Mirrors `vp8l_transform::decode_lossless`: each transform's sub-image is
/// decoded and copied, and the copies are kept until the decode ends. The
/// image itself is decoded with one group, or with as many as its entropy
/// image names, beside the entropy image's group numbers, the color cache
/// and the pixel buffer. A color-indexing transform then expands the packed
/// pixels into a new full-width buffer beside them.
fn lossless(
    mut reader: BitReader<'_>,
    width: u32,
    height: u32,
) -> Result<Lossless, FrameworkError> {
    let reader = &mut reader;
    let mut kept = 0u64;
    let mut peak = 0u64;
    let mut seen = [false; 4];
    let mut packed_width = width;
    let mut indexed = false;
    while reader.read_bit().map_err(failed)? {
        let kind = TransformType::from_bits(reader.read_bits(2).map_err(failed)?);
        if let Some(seen) = seen.get_mut(kind as usize) {
            if *seen {
                return Err(failed("a transform appears more than once"));
            }
            *seen = true;
        }
        let (sub_width, sub_height) = match kind {
            TransformType::Predictor | TransformType::Color => {
                let block = 1u32 << (reader.read_bits(3).map_err(failed)? + 2);
                (packed_width.div_ceil(block), height.div_ceil(block))
            }
            TransformType::SubtractGreen => continue,
            TransformType::ColorIndexing => (reader.read_bits(8).map_err(failed)? + 1, 1),
        };
        let sub = sub_image(reader, sub_width, sub_height)?;
        let copy = sub.pixels.saturating_mul(4);
        peak = peak.max(kept.saturating_add(sub.decoding.max(sub.buffer.saturating_add(copy))));
        kept = kept.saturating_add(copy);
        if kind == TransformType::ColorIndexing {
            indexed = true;
            let bundled = color_indexing_width_bits(sub_width as usize);
            packed_width = packed_width.div_ceil(1 << bundled);
        }
    }

    let header =
        MetaPrefixHeader::read(reader, ImageRole::Argb, packed_width, height).map_err(failed)?;
    let cache_size = header.color_cache.size();
    let cache = cache_size as u64 * 4;
    let buffer = pixel_buffer(u64::from(packed_width) * u64::from(height));
    let tables = match &header.codes {
        MetaPrefixCodes::Single { group } => GROUP + group_bytes(group),
        MetaPrefixCodes::EntropyImagePending {
            image_width,
            image_height,
            ..
        } => {
            let entropy = sub_image(reader, *image_width, *image_height)?;
            // The decoded entropy image, then the group numbers collected
            // out of it while it is still alive.
            let index = entropy.pixels.saturating_mul(2);
            peak = peak.max(
                kept.saturating_add(entropy.decoding.max(entropy.buffer.saturating_add(index))),
            );
            let groups = u64::from(entropy.largest_group) + 1;
            let mut tables = index.saturating_add(groups.saturating_mul(GROUP));
            for _ in 0..groups {
                let group = PrefixCodeGroup::read(reader, cache_size).map_err(failed)?;
                tables = tables.saturating_add(group_bytes(&group));
            }
            tables
        }
    };
    peak = peak.max(
        kept.saturating_add(tables)
            .saturating_add(cache)
            .saturating_add(buffer),
    );

    let full = (u64::from(width) * u64::from(height)).saturating_mul(4);
    let image = if indexed {
        peak = peak.max(kept.saturating_add(buffer).saturating_add(full));
        full
    } else {
        buffer
    };
    Ok(Lossless { peak, image })
}

/// A sub-image read past: what its decode allocates, and the largest group
/// number its pixels name, when it is an entropy image.
struct SubImage {
    /// Bytes alive while it decodes: its group, color cache and pixels.
    decoding: u64,
    /// Bytes of its pixel buffer.
    buffer: u64,
    /// How many pixels it has.
    pixels: u64,
    /// The largest `(red << 8) | green` among its pixels.
    largest_group: u32,
}

/// Read an entropy-coded sub-image as `vp8l_decode::decode_entropy_coded_image`
/// does, without keeping its pixels.
fn sub_image(
    reader: &mut BitReader<'_>,
    width: u32,
    height: u32,
) -> Result<SubImage, FrameworkError> {
    if width == 0 || height == 0 {
        return Err(failed("a sub-image is empty"));
    }
    let header =
        MetaPrefixHeader::read(reader, ImageRole::EntropyCoded, width, height).map_err(failed)?;
    let MetaPrefixCodes::Single { group } = &header.codes else {
        return Err(failed("a sub-image names an entropy image"));
    };
    let cache_size = header.color_cache.size();
    let pixels = u64::from(width) * u64::from(height);
    let largest_group = skip_pixels(reader, group, cache_size, width, pixels)?;
    let buffer = pixel_buffer(pixels);
    Ok(SubImage {
        decoding: (GROUP + group_bytes(group))
            .saturating_add(cache_size as u64 * 4)
            .saturating_add(buffer),
        buffer,
        pixels,
        largest_group,
    })
}

/// Read `pixels` pixels coded with `group`, as `vp8l_decode::decode_image`
/// does, and return the largest group number they name. Only literal pixels
/// can raise it: a backward reference copies earlier pixels, and a color
/// cache hit returns an earlier pixel or zero.
fn skip_pixels(
    reader: &mut BitReader<'_>,
    group: &PrefixCodeGroup,
    cache_size: usize,
    width: u32,
    pixels: u64,
) -> Result<u32, FrameworkError> {
    let alphabet = PrefixCodeGroup::green_alphabet_size(cache_size);
    let mut position = 0u64;
    let mut largest = 0u32;
    while position < pixels {
        let symbol = group.green.read_symbol(reader).map_err(failed)?;
        match GreenSymbol::classify(usize::from(symbol), alphabet).map_err(failed)? {
            GreenSymbol::Literal { green } => {
                let red = group.red.read_symbol(reader).map_err(failed)? as u8;
                group.blue.read_symbol(reader).map_err(failed)?;
                group.alpha.read_symbol(reader).map_err(failed)?;
                largest = largest.max((u32::from(red) << 8) | u32::from(green));
                position += 1;
            }
            GreenSymbol::LengthPrefix { prefix_code } => {
                let length = read_lz77_value(reader, prefix_code).map_err(failed)?;
                let distance_prefix = group.distance.read_symbol(reader).map_err(failed)?;
                let distance_code =
                    read_lz77_value(reader, u32::from(distance_prefix)).map_err(failed)?;
                let distance = distance_code_to_pixel_distance(distance_code, width);
                if distance as u64 > position {
                    return Err(failed(format_args!(
                        "a backward reference of {distance} pixels underflows at pixel {position}"
                    )));
                }
                let end = position + u64::from(length);
                if end > pixels {
                    return Err(failed(format_args!(
                        "a backward reference of {length} pixels at pixel {position} overruns \
                         the {pixels}-pixel image"
                    )));
                }
                position = end;
            }
            GreenSymbol::ColorCache { index } => {
                if index >= cache_size {
                    return Err(failed(format_args!(
                        "color cache index {index} is past a {cache_size}-entry cache"
                    )));
                }
                position += 1;
            }
        }
    }
    Ok(largest)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bits packed least-significant first, the order VP8L reads them in.
    #[derive(Default)]
    struct Bits {
        bytes: Vec<u8>,
        used: usize,
    }

    impl Bits {
        fn put(&mut self, value: u32, count: usize) {
            for bit in 0..count {
                if self.used.is_multiple_of(8) {
                    self.bytes.push(0);
                }
                if (value >> bit) & 1 == 1
                    && let Some(byte) = self.bytes.last_mut()
                {
                    *byte |= 1 << (self.used % 8);
                }
                self.used += 1;
            }
        }

        /// A simple prefix code with one symbol.
        fn single_symbol(&mut self, symbol: u32) {
            self.put(1, 1);
            self.put(0, 1);
            if symbol < 2 {
                self.put(0, 1);
                self.put(symbol, 1);
            } else {
                self.put(1, 1);
                self.put(symbol, 8);
            }
        }

        /// A `VP8L` chunk payload's signature and image header.
        fn header(&mut self, width: u32, height: u32) {
            self.put(0x2F, 8);
            self.put(width - 1, 14);
            self.put(height - 1, 14);
            self.put(0, 4);
        }
    }

    fn webp(payload: &[u8]) -> Vec<u8> {
        let mut body = b"WEBPVP8L".to_vec();
        body.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        body.extend_from_slice(payload);
        if payload.len() % 2 == 1 {
            body.push(0);
        }
        let mut file = b"RIFF".to_vec();
        file.extend_from_slice(&(body.len() as u32).to_le_bytes());
        file.extend_from_slice(&body);
        file
    }

    /// A 4x4 image with no transform and no color cache, whose 1x1 entropy
    /// image names group `groups - 1`, followed by that many groups of
    /// one-symbol codes.
    fn grouped(groups: u32) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.header(4, 4);
        bits.put(0, 1);
        bits.put(0, 1);
        bits.put(1, 1);
        bits.put(0, 3);
        bits.put(0, 1);
        let last = groups - 1;
        for symbol in [last & 0xFF, last >> 8, 0, 0, 0] {
            bits.single_symbol(symbol);
        }
        for _ in 0..groups * 5 {
            bits.single_symbol(0);
        }
        bits.bytes
    }

    fn lossless_peak(payload: &[u8]) -> u64 {
        match plan(&webp(payload)).expect("the bitstream reads") {
            WebpPlan::Lossless { image, .. } => image.peak,
            other => panic!("expected the lossless path, got {other:?}"),
        }
    }

    #[test]
    fn every_group_the_entropy_image_names_is_charged() {
        let one = lossless_peak(&grouped(1));
        let many = lossless_peak(&grouped(300));
        // Each group is stored inline in the decoder's list, and each of its
        // five one-symbol codes keeps a byte per symbol of its alphabet and
        // the symbol itself.
        let per_group = GROUP + (280 + 2) + 3 * (256 + 2) + (40 + 2);
        assert_eq!(many - one, 299 * per_group);
    }

    #[test]
    fn a_bitstream_that_ends_inside_its_groups_is_refused() {
        let mut payload = grouped(300);
        payload.truncate(payload.len() / 2);
        let error = plan(&webp(&payload)).expect_err("the groups run past the data");
        assert!(
            error
                .to_string()
                .contains("image decode failed: image/webp")
        );
    }

    #[test]
    fn a_repeated_transform_is_refused() {
        let mut bits = Bits::default();
        bits.header(4, 4);
        for _ in 0..2 {
            // A subtract-green transform, which carries no data.
            bits.put(1, 1);
            bits.put(2, 2);
        }
        bits.put(0, 1);
        let error = plan(&webp(&bits.bytes)).expect_err("a transform appears twice");
        assert!(error.to_string().contains("more than once"), "{error}");
    }

    #[test]
    fn a_color_indexed_image_is_charged_for_its_expansion() {
        // Two colors bundle eight pixels a byte: the 256x256 image is coded
        // 32 pixels wide, then expanded into a full-width buffer beside it.
        let mut bits = Bits::default();
        bits.header(256, 256);
        bits.put(1, 1);
        bits.put(3, 2);
        bits.put(1, 8);
        // The 2x1 color table: no cache, one-symbol codes.
        bits.put(0, 1);
        for _ in 0..5 {
            bits.single_symbol(0);
        }
        bits.put(0, 1);
        // The packed image: no cache, no meta prefix, one-symbol codes.
        bits.put(0, 1);
        bits.put(0, 1);
        for _ in 0..5 {
            bits.single_symbol(0);
        }
        let WebpPlan::Lossless { image, .. } = plan(&webp(&bits.bytes)).expect("it reads") else {
            panic!("expected the lossless path");
        };
        let table = 2 * 4;
        assert_eq!(image.image, 256 * 256 * 4);
        assert_eq!(image.peak, table + 32 * 256 * 4 + 256 * 256 * 4);
    }

    #[test]
    fn a_reserved_alpha_compression_is_refused() {
        let error = alpha_plane(&[0b11], 4, 4).expect_err("method 3 is reserved");
        assert!(error.to_string().contains("reserved"), "{error}");
    }
}
