//! How many bytes the built-in driver's decoders allocate for one image,
//! estimated from its headers before any decoder runs.
//!
//! The header gate counts the declared pixels at four bytes each, but the
//! decoders allocate more than the RGBA they return. oxideav-png keeps its
//! inflated data, the unfiltered rows and a copy of them while it builds its
//! output; GIF composition holds three screen-sized canvases; the JPEG
//! decoder keeps four bytes of coefficients for every sample of a progressive
//! image. `IMAGE_MAX_ALLOC_BYTES` is the most one decode may allocate, so the
//! driver refuses an image whose estimate is larger.
//!
//! # How the numbers are derived
//!
//! Each estimate follows its decoder's code path at the pinned version and
//! adds up the buffers that are alive at the same time, taking the largest
//! such set. The comments name each buffer. Capacities count, not lengths: a
//! `Vec` that grows by doubling can hold up to twice what it needs, and the
//! capacity is what the allocator hands out.
//!
//! The estimates are upper bounds. `framework/tests/memory/images.rs`
//! measures each format with dhat and fails when a decode allocates more
//! than its estimate, so a decoder upgrade that allocates more shows up
//! there first.
//!
//! The input itself is not counted: it is already in memory, under the
//! separate source-size check. Copies of it are counted.
//!
//! One allocation is not covered: the prefix-code tables of a lossless WebP
//! (and of a lossless-coded alpha plane). How many there are comes from the
//! compressed data, not from a header, so only the decode itself can count
//! them.

use crate::error::FrameworkError;

use super::{png_error, png_inflated_len};
use crate::media::sniff::{self, BmpLayout, GifFrame, InputFormat, JpegFrame, WebpLayout};

/// Small allocations no estimate tracks one by one: frame and plane headers,
/// per-component vectors, Huffman and quantization tables, error strings.
/// dhat measures a few KiB of them on every format.
const FIXED: u64 = 64 * 1024;

/// compcol's inflate: the 64 KiB output scratch both PNG passes write
/// through, plus the decoder's 32 KiB window and per-block Huffman tables.
/// dhat measures 110 KB for the framework's own pass over a small stream.
const INFLATE_WORK: u64 = 128 * 1024;

/// One `ChunkRef` in oxideav-png's chunk list: a 4-byte type and a slice.
const PNG_CHUNK_REF: u64 = 24;

/// The LZW dictionary of oxideav-gif (4096 `u16` parents and 4096 `u8`
/// suffixes), its 4096-byte scratch string, and two color tables of at most
/// 256 RGB entries.
const GIF_TABLES: u64 = 4096 * 2 + 4096 + 4096 + 2 * 256 * 3;

/// oxideav-vp8, per 16x16 macroblock: 800 bytes of coefficients (25 blocks of
/// sixteen `i16`), 384 bytes of padded Y, U and V planes, and the 21-byte
/// prediction modes, rounded up.
const VP8_PER_MACROBLOCK: u64 = 800 + 384 + 24;

/// oxideav-webp's lossless color cache: at most 2^11 `u32` entries.
const VP8L_COLOR_CACHE: u64 = (1 << 11) * 4;

/// oxideav-webp reserves a lossless image's pixels up front only to this
/// many; a larger image grows its buffer by doubling.
const VP8L_EAGER_PIXELS: u64 = 1 << 22;

/// oxideav-webp's chunk list: 24 bytes a top-level chunk, and the gate
/// refuses a file with more than 4096 of them.
const WEBP_CHUNK_LIST: u64 = 24 * 4096;

/// oxideav-bmp's lookup table for 16-bit pixels: 65,536 RGBA entries.
const BMP_LUT_16: u64 = 65_536 * 4;

/// What [`estimate`] and the decode read from an image's headers, walked once.
pub(super) enum Layout {
    Png(PngLayout),
    Gif(GifFrame),
    Jpeg(JpegFrame),
    WebP(WebpLayout),
    Bmp(BmpLayout),
}

/// What the PNG estimate and the inflate pre-pass read from the chunk list.
pub(super) struct PngLayout {
    pub(super) ihdr: oxideav_png::Ihdr,
    /// Total IDAT payload, which oxideav-png copies into one buffer.
    pub(super) idat_len: u64,
    /// Chunks in the file, each one a `ChunkRef` in oxideav-png's list.
    pub(super) chunks: u64,
}

/// Walk a PNG's chunks the way oxideav-png does: `read_chunk` from the
/// signature to IEND, with the same CRC checks.
pub(super) fn for_each_png_chunk<'a>(
    contents: &'a [u8],
    mut visit: impl FnMut(&oxideav_png::chunk::ChunkRef<'a>) -> Result<(), FrameworkError>,
) -> Result<(), FrameworkError> {
    let mut pos = 8;
    loop {
        let (chunk, next) = oxideav_png::chunk::read_chunk(contents, pos).map_err(png_error)?;
        visit(&chunk)?;
        pos = next;
        if chunk.is_type(b"IEND") {
            return Ok(());
        }
        if pos >= contents.len() {
            return Err(FrameworkError::param(
                "image decode failed: png: the stream ends before its IEND chunk",
            ));
        }
    }
}

/// Read the headers of an image the gate has already measured.
///
/// A GIF whose first frame does not fit its logical screen is refused here:
/// the decoder sizes that frame's pixels from its own descriptor, and only
/// composing it onto the screen would reject it, after the allocation.
pub(super) fn layout(
    format: InputFormat,
    contents: &[u8],
    width: u32,
    height: u32,
) -> Result<Layout, FrameworkError> {
    let malformed = || {
        FrameworkError::param(format!(
            "image header is malformed: could not read the {} layout",
            format.mime_type()
        ))
    };
    Ok(match format {
        InputFormat::Png => {
            let mut ihdr = None;
            let mut idat_len = 0u64;
            let mut chunks = 0u64;
            for_each_png_chunk(contents, |chunk| {
                chunks += 1;
                if chunk.is_type(b"IHDR") && ihdr.is_none() {
                    ihdr = Some(oxideav_png::Ihdr::parse(chunk.data).map_err(png_error)?);
                } else if chunk.is_type(b"IDAT") {
                    idat_len += chunk.data.len() as u64;
                }
                Ok(())
            })?;
            let ihdr = ihdr
                .ok_or_else(|| FrameworkError::param("image decode failed: png: no IHDR chunk"))?;
            Layout::Png(PngLayout {
                ihdr,
                idat_len,
                chunks,
            })
        }
        InputFormat::Gif => {
            let first = sniff::gif_first_frame(contents).ok_or_else(|| {
                FrameworkError::param(
                    "image decode failed: image/gif: the stream ends or breaks its block \
                     structure before the first frame",
                )
            })?;
            if !first.fits(width, height) {
                return Err(FrameworkError::param(format!(
                    "image is malformed: its first GIF frame ({}x{} at {},{}) does not fit its \
                     {width}x{height} logical screen",
                    first.width, first.height, first.left, first.top
                )));
            }
            Layout::Gif(first)
        }
        InputFormat::Jpeg => Layout::Jpeg(sniff::jpeg_frame(contents).ok_or_else(malformed)?),
        InputFormat::WebP => Layout::WebP(sniff::webp_layout(contents)),
        InputFormat::Bmp => Layout::Bmp(sniff::bmp_layout(contents).ok_or_else(malformed)?),
    })
}

/// The most bytes decoding this image allocates, beyond the input.
///
/// Errors for a JPEG coding the driver cannot use, before it is decoded.
pub(super) fn estimate(
    layout: &Layout,
    input_len: u64,
    width: u32,
    height: u32,
) -> Result<u64, FrameworkError> {
    let (width, height) = (u64::from(width), u64::from(height));
    let peak = match layout {
        Layout::Png(png) => png_peak(png)?,
        Layout::Gif(first) => gif_peak(width, height, first),
        Layout::Jpeg(frame) => jpeg_peak(frame, input_len)?,
        Layout::WebP(webp) => webp_peak(width, height, *webp),
        Layout::Bmp(bmp) => bmp_peak(width, height, *bmp, input_len),
    };
    Ok(peak.saturating_add(FIXED))
}

/// `a * b`, saturating: a saturated estimate is refused, never wrapped.
fn mul(a: u64, b: u64) -> u64 {
    a.saturating_mul(b)
}

/// `a + b`, saturating.
fn add(a: u64, b: u64) -> u64 {
    a.saturating_add(b)
}

/// Capacity of a `Vec` built by `push`: oxideav-png's chunk list starts at
/// four entries and doubles.
fn pushed_capacity(len: u64) -> u64 {
    len.max(4).checked_next_power_of_two().unwrap_or(u64::MAX)
}

/// oxideav-png's `decode_png_to_rgba`.
///
/// While the pixels decode, these are alive together:
///
/// - two chunk lists, one built by `decode_png_to_rgba` and one by the
///   `decode_png` it calls, at 24 bytes a chunk, doubling;
/// - the concatenated IDAT data, sized exactly;
/// - the inflated data. compcol starts that buffer at twice the IDAT length
///   and doubles it from there, so it holds `2 * idat` when that already
///   fits the inflated length, and less than twice the inflated length when
///   it grows;
///
/// and, in turn, the inflate's own work, the unfiltered rows with a copy of
/// them, and the unfiltered image beside the built one. Then the chunk lists
/// and the inflated data go, and the built image converts to RGBA. The
/// framework's own inflate pass runs before all of it, holding a copy of the
/// IDAT data and the inflate's work.
fn png_peak(png: &PngLayout) -> Result<u64, FrameworkError> {
    let ihdr = &png.ihdr;
    let inflated = png_inflated_len(ihdr).ok_or_else(|| {
        FrameworkError::param(format!(
            "image decode failed: png: colour type {} at bit depth {} with interlace method {} \
             is not a PNG pixel format, or is too large to decode",
            ihdr.colour_type, ihdr.bit_depth, ihdr.interlace
        ))
    })?;
    let (width, height) = (u64::from(ihdr.width), u64::from(ihdr.height));
    let pixels = mul(width, height);
    let channels: u64 = match ihdr.colour_type {
        2 => 3,
        4 => 2,
        6 => 4,
        _ => 1,
    };
    let bits = channels * u64::from(ihdr.bit_depth);
    let sub_byte = ihdr.bit_depth < 8;
    // Bytes per pixel of the unfiltered image: sub-byte samples widen to one
    // byte each.
    let pixel_bytes = if sub_byte { 1 } else { bits / 8 };
    let unfiltered = mul(pixels, pixel_bytes);
    // `build_png_image` converts to little-endian at the same size, except
    // 16-bit grey with alpha, which it widens to four channels.
    let built = if ihdr.colour_type == 4 && ihdr.bit_depth == 16 {
        mul(pixels, 8)
    } else {
        unfiltered
    };
    let row = |pass_width: u64| mul(pass_width, bits).div_ceil(8);
    // `reconstruct_filtered` allocates the rows plus one zero row; the
    // expand step copies them (bit depth 8 and up) or widens them to a byte a
    // pixel (sub-byte).
    let unfilter = |pass_width: u64, pass_height: u64| {
        let raw = mul(row(pass_width), pass_height);
        let expanded = if sub_byte {
            mul(pass_width, pass_height)
        } else {
            raw
        };
        add(add(raw, row(pass_width)), expanded)
    };
    let unfiltering = if ihdr.interlace == 0 {
        unfilter(width, height)
    } else {
        // Adam7 scatters each pass into a full-size canvas.
        let largest_pass = super::ADAM7_PASSES
            .iter()
            .map(|&(first_row, first_column, row_step, column_step)| {
                unfilter(
                    width.saturating_sub(first_column).div_ceil(column_step),
                    height.saturating_sub(first_row).div_ceil(row_step),
                )
            })
            .max()
            .unwrap_or(0);
        add(unfiltered, largest_pass)
    };
    let chunk_list = mul(PNG_CHUNK_REF, pushed_capacity(png.chunks));
    let idat = png.idat_len;
    let inflate_buffer = if mul(idat, 2) >= inflated {
        mul(idat, 2)
    } else {
        mul(inflated, 2)
    };
    let held = add(add(mul(chunk_list, 2), idat), inflate_buffer);
    let decoding = add(
        held,
        INFLATE_WORK.max(unfiltering).max(add(unfiltered, built)),
    );
    let to_rgba = add(add(chunk_list, built), mul(pixels, 4));
    let pre_pass = add(idat, INFLATE_WORK);
    Ok(decoding.max(to_rgba).max(pre_pass))
}

/// oxideav-gif's `decode_first_frame`, then `compose`.
///
/// Decoding gathers the frame's LZW data into one buffer that grows a
/// sub-block at a time (doubling), and decompresses into an index buffer
/// reserved at `min(frame pixels, data * 4096)`; an interlaced frame is
/// reordered into a second index buffer. Composing that one frame holds a
/// screen-sized canvas, the snapshot taken before the frame renders, and the
/// clone it returns, beside the frame's indices.
fn gif_peak(screen_width: u64, screen_height: u64, first: &GifFrame) -> u64 {
    let screen = mul(mul(screen_width, screen_height), 4);
    let frame_pixels = mul(u64::from(first.width), u64::from(first.height));
    let data = first.data_len as u64;
    let gathered = mul(data, 2).max(8);
    let indices = if mul(data, 4096) >= frame_pixels {
        frame_pixels
    } else {
        mul(frame_pixels, 2)
    };
    let reordered = if first.interlaced { frame_pixels } else { 0 };
    let decoding = add(add(gathered, indices), reordered);
    let kept = if first.interlaced {
        frame_pixels
    } else {
        indices
    };
    let composing = add(kept, mul(screen, 3));
    add(decoding.max(composing), GIF_TABLES)
}

/// oxideav-mjpeg through the codec registry, then the conversion to RGBA.
///
/// The driver hands the decoder a copy of the input, and the decoder keeps a
/// clone of that packet until it decodes. Each component decodes into a
/// sample buffer padded to whole MCUs. A sequential frame whose first scan
/// carries every component goes straight to the output planes; any other
/// (progressive, arithmetic, or one component a scan) first fills a
/// coefficient buffer of four bytes a padded sample and renders from it at
/// the end. Lossless frames keep a `u32` a sample. Converting YCbCr planes
/// to RGBA copies them (`gather_tight`), builds packed RGB, then RGBA; grey
/// and packed RGB convert directly. An odd-sized subsampled image converts
/// at its padded size; see `to_rgba`.
fn jpeg_peak(frame: &JpegFrame, input_len: u64) -> Result<u64, FrameworkError> {
    let unsupported = |what: &str| {
        Err(FrameworkError::param(format!(
            "image format is not supported: {what} JPEG. The oxideav driver reads 8-bit \
             greyscale and colour JPEGs; convert the image, or set IMAGE_DRIVER=magick"
        )))
    };
    match frame.marker {
        0xC5..=0xC7 | 0xCD..=0xCF => return unsupported("hierarchical"),
        _ => {}
    }
    if frame.precision != 8 {
        return unsupported(&format!("{}-bit", frame.precision));
    }
    let components = frame.components;
    if components != 1 && components != 3 {
        return unsupported(&format!("{components}-component"));
    }
    let used = &frame.sampling[..usize::from(components)];
    let max_h = u64::from(used.iter().map(|f| f.0).max().unwrap_or(1).max(1));
    let max_v = u64::from(used.iter().map(|f| f.1).max().unwrap_or(1).max(1));
    let (width, height) = (u64::from(frame.width), u64::from(frame.height));
    let mcus_x = width.div_ceil(8 * max_h);
    let mcus_y = height.div_ceil(8 * max_v);
    let padded: u64 = used
        .iter()
        .map(|&(h, v)| mul(mul(mcus_x, 8 * u64::from(h)), mul(mcus_y, 8 * u64::from(v))))
        .fold(0, add);
    let pixels = mul(width, height);
    // The output planes are never larger than the padded sample buffers.
    let planes = padded;
    let decoding = match frame.marker {
        0xC3 | 0xCB => add(
            mul(mul(pixels, 4), u64::from(components)),
            mul(pixels, u64::from(components)),
        ),
        0xC0 | 0xC1 if frame.first_scan == components => add(padded, planes),
        _ => add(add(mul(padded, 4), padded), planes),
    };
    // Samples stored as RGB (components named R, G, B at full resolution)
    // decode to packed RGB, which converts straight to RGBA.
    let packed_rgb = components == 3
        && frame.ids[..3] == *b"RGB"
        && used.iter().all(|&factors| factors == (1, 1));
    let converting = if components == 1 || packed_rgb {
        add(planes, mul(pixels, 4))
    } else {
        // An odd side under 4:2:0 or 4:2:2 converts at the size the chroma
        // covers: the driver first copies the luma plane at that size beside
        // the planes, then converts the padded frame.
        let min_h = u64::from(used.iter().map(|f| f.0).min().unwrap_or(1).max(1));
        let min_v = u64::from(used.iter().map(|f| f.1).min().unwrap_or(1).max(1));
        let padded_pixels = mul(
            width.next_multiple_of(max_h / min_h),
            height.next_multiple_of(max_v / min_v),
        );
        if padded_pixels == pixels {
            add(mul(planes, 2), mul(pixels, 7))
        } else {
            let padded_planes = add(planes, padded_pixels - pixels);
            add(planes, padded_pixels).max(add(mul(padded_planes, 2), mul(padded_pixels, 7)))
        }
    };
    let sending = mul(input_len, 2);
    Ok(sending.max(add(input_len, decoding)).max(converting))
}

/// oxideav-webp's `decode_webp_image`, which returns RGBA directly.
///
/// Lossless: the ARGB image (reserved exactly up to 2^22 pixels, doubling
/// past that), the RGBA conversion beside it, and up to three sub-images at
/// a quarter of each dimension (predictor, color transform, entropy image),
/// plus the color cache. A color-indexing transform expands into a new
/// full-size buffer while the packed one is alive, which the same total
/// covers. Lossy: oxideav-vp8 holds every macroblock's coefficients, the
/// padded planes and the cropped copy together; the RGBA conversion follows,
/// and an alpha plane decodes beside the finished RGBA.
fn webp_peak(width: u64, height: u64, webp: WebpLayout) -> u64 {
    let pixels = mul(width, height);
    let rgba = mul(pixels, 4);
    let lossless_core = || {
        let argb = if pixels <= VP8L_EAGER_PIXELS {
            pixels
        } else {
            pixels.checked_next_power_of_two().unwrap_or(u64::MAX)
        };
        let sub_image = mul(mul(width.div_ceil(4), height.div_ceil(4)), 4);
        add(add(mul(argb, 4), mul(sub_image, 3)), VP8L_COLOR_CACHE)
    };
    let lossless = if webp.lossless {
        add(lossless_core(), rgba)
    } else {
        0
    };
    let lossy = if webp.lossy {
        let macroblocks = mul(width.div_ceil(16), height.div_ceil(16));
        let cropped = add(pixels, mul(mul(width.div_ceil(2), height.div_ceil(2)), 2));
        let decoding = add(mul(macroblocks, VP8_PER_MACROBLOCK), cropped);
        let converting = add(cropped, rgba);
        let alpha = if webp.alpha {
            add(add(rgba, lossless_core()), pixels)
        } else {
            0
        };
        decoding.max(converting).max(alpha)
    } else {
        0
    };
    add(lossless.max(lossy), WEBP_CHUNK_LIST)
}

/// oxideav-bmp's `decode_bmp`, which writes one RGBA plane.
///
/// Run-length encoded bitmaps decode into one RGBA vector a row and are then
/// concatenated, so both copies are alive. 16-bit pixels may go through a
/// lookup table. The palette holds four bytes an entry, as many as the
/// header declares and the space before the pixel data allows.
fn bmp_peak(width: u64, height: u64, bmp: BmpLayout, input_len: u64) -> u64 {
    let rgba = mul(mul(width, height), 4);
    let run_length = if matches!(bmp.compression, 1 | 2) {
        // The rows, and a 24-byte vector header each.
        add(rgba, mul(height, 24))
    } else {
        0
    };
    let lookup = if bmp.bits_per_pixel == 16 {
        BMP_LUT_16
    } else {
        0
    };
    let palette = mul(
        bmp.palette_entries
            .min(input_len / bmp.palette_entry_bytes.max(1)),
        4,
    );
    add(add(add(rgba, run_length), lookup), palette)
}
