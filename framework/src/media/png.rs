//! Lossless compression for images that must remain PNG.

use crate::error::FrameworkError;

use super::sniff::{self, InputFormat};

/// Choose compression work without reducing PNG quality.
///
/// Neither preset changes pixels, including RGB values behind transparency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PngEffort {
    /// Use oxipng's default level 2 for quick page budget audits.
    Fast,
    /// Use level 6 with Zopfli for PNGs retained during static publication.
    /// This takes more CPU time to reduce transfer size.
    Maximum,
}

/// Reduce PNG transfer size without a lossy encode or a quality setting.
///
/// Text, time and location metadata are removed. Colour chunks, the ICC
/// profile, transparency and indexed palettes are kept. The returned file
/// is no larger than the input, even when compression cannot improve it.
/// The size fallback uses stripped bytes so private metadata cannot return.
///
/// This CPU work is synchronous. Async callers can use
/// [`Image::recompress_png`](super::Image::recompress_png) to run it on a
/// blocking worker. Non-PNG and malformed input return an error.
pub fn recompress_png(bytes: &[u8], effort: PngEffort) -> Result<Vec<u8>, FrameworkError> {
    recompress(bytes, effort, false)
}

/// Driver output already holds only the opt-out's Orientation-only EXIF.
/// Keep it so recompression does not undo that viewer correction.
pub(super) fn recompress_processed_png(
    bytes: &[u8],
    effort: PngEffort,
) -> Result<Vec<u8>, FrameworkError> {
    recompress(bytes, effort, true)
}

fn recompress(
    bytes: &[u8],
    effort: PngEffort,
    keep_orientation: bool,
) -> Result<Vec<u8>, FrameworkError> {
    let config = super::config();
    super::check_source_size(bytes.len() as u64, config.max_alloc_bytes, "PNG source")?;
    let format = sniff::detect(bytes);
    if format != Some(InputFormat::Png) {
        let name = format.map_or_else(
            || infer::get(bytes).map_or("unrecognised format", |kind| kind.mime_type()),
            InputFormat::mime_type,
        );
        return Err(FrameworkError::param(format!(
            "PNG recompression requires image/png; received {name}"
        )));
    }
    sniff::guard(bytes, &config)?;
    let stripped = strip_metadata(bytes, keep_orientation)?;
    let mut options = match effort {
        PngEffort::Fast => oxipng::Options::default(),
        PngEffort::Maximum => {
            let mut options = oxipng::Options::max_compression();
            options.deflater = oxipng::Deflater::Zopfli(oxipng::ZopfliOptions::default());
            options
        }
    };
    // Strip before optimization. oxipng's Safe list drops gAMA and cHRM,
    // and enabling its stripping can replace an ICC profile with sRGB.
    options.strip = oxipng::StripChunks::None;
    options.optimize_alpha = false;
    options.scale_16 = false;
    options.max_decompressed_size =
        Some(usize::try_from(config.max_alloc_bytes).unwrap_or(usize::MAX));
    if stripped.get(25) == Some(&3) {
        // Keep indexed output and the original palette entries and order.
        options.palette_reduction = false;
        options.color_type_reduction = false;
        options.grayscale_reduction = false;
    }
    let optimized = oxipng::optimize_from_memory(&stripped, &options)
        .map_err(|e| FrameworkError::param(format!("PNG recompression failed: {e}")))?;
    Ok(if optimized.len() < stripped.len() {
        optimized
    } else {
        stripped
    })
}

fn strip_metadata(bytes: &[u8], keep_orientation: bool) -> Result<Vec<u8>, FrameworkError> {
    let (first, mut pos) = oxideav_png::chunk::read_chunk(bytes, 8)
        .map_err(|e| FrameworkError::param(format!("PNG recompression failed: {e}")))?;
    if !first.is_type(b"IHDR") {
        return Err(FrameworkError::param("PNG must start with its IHDR chunk"));
    }
    let header = oxideav_png::Ihdr::parse(first.data)
        .map_err(|e| FrameworkError::param(format!("PNG recompression failed: {e}")))?;
    let canvas = (header.width, header.height);
    let mut output = bytes[..pos].to_vec();
    loop {
        let (chunk, next) = oxideav_png::chunk::read_chunk(bytes, pos)
            .map_err(|e| FrameworkError::param(format!("PNG recompression failed: {e}")))?;
        if chunk.is_type(b"IHDR") {
            return Err(FrameworkError::param("PNG has a duplicate IHDR chunk"));
        }
        validate_animation_chunk(&chunk, canvas)?;
        let kind = &chunk.chunk_type;
        let display = matches!(
            kind,
            b"tRNS"
                | b"iCCP"
                | b"sRGB"
                | b"cICP"
                | b"gAMA"
                | b"cHRM"
                | b"pHYs"
                | b"bKGD"
                | b"sBIT"
                | b"mDCV"
                | b"cLLI"
                | b"acTL"
                | b"fcTL"
                | b"fdAT"
        );
        if kind[0].is_ascii_uppercase() || display || (keep_orientation && kind == b"eXIf") {
            output.extend_from_slice(&bytes[pos..next]);
        }
        if chunk.is_type(b"IEND") {
            return Ok(output);
        }
        pos = next;
    }
}

// oxipng assumes valid frame controls and does not apply its input size cap
// when decoding later frames. Validate before it reads or allocates them.
fn validate_animation_chunk(
    chunk: &oxideav_png::chunk::ChunkRef<'_>,
    canvas: (u32, u32),
) -> Result<(), FrameworkError> {
    if chunk.is_type(b"fdAT") && chunk.data.len() < 4 {
        return Err(FrameworkError::param(
            "PNG fdAT frame data lacks its four-byte sequence number",
        ));
    }
    let checked = match &chunk.chunk_type {
        b"acTL" => oxideav_png::apng::Actl::parse(chunk.data).map(|_| ()),
        b"fcTL" => oxideav_png::apng::Fctl::parse(chunk.data)
            .and_then(|frame| frame.validate_within_canvas(canvas.0, canvas.1)),
        _ => Ok(()),
    };
    checked.map_err(|e| FrameworkError::param(format!("PNG recompression failed: {e}")))
}
