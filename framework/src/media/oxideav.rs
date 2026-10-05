//! The built-in, pure-Rust image driver, backed by the OxideAV codec family.
//!
//! # Architecture, and why it is not the obvious one
//!
//! OxideAV ships an `oxideav-io` facade with `open_rgba`/`save` entry points
//! that look like exactly what this driver wants. They are not usable here:
//! that facade routes through the *container* registry, and of the five
//! formats this driver supports only PNG, JPEG, and BMP register a demuxer -
//! GIF and WebP register a file-extension hint and nothing else, so the
//! facade cannot see them at all. Its save path is narrower still.
//!
//! So this driver drives the **codec registry directly**. For a still image
//! that works cleanly: the whole file is one `Packet` in, one `VideoFrame`
//! out, and on the encode side the packet an encoder emits *is* the complete
//! file - which is precisely why those codecs never needed a muxer.
//!
//! Decoding takes the registry only for lossless JPEG. PNG, GIF, WebP and
//! BMP have entry points of their own that return RGBA, and those skip the
//! registry's two copies of the input (the packet, and the decoder's clone
//! of it) and the conversion afterwards. Every other JPEG is decoded by
//! zune-jpeg (see `decode_jpeg`): oxideav-mjpeg refuses any frame of more
//! than 64 Mi samples, about 22 megapixels in colour, and decodes
//! arithmetic-coded colour JPEGs to the wrong pixels. Every format's decode
//! is costed before it runs; see the `peak` module.
//!
//! `oxideav-io` is therefore not a dependency at all: with decode and encode
//! on the registry and the codecs' own entry points, nothing was left for it
//! to do.
//!
//! ## The sandbox, one layer up
//!
//! `oxideav-io`'s `OpenOptions::allow_codecs` is the knob its docs recommend
//! for untrusted input. Driving the registry directly gives up that knob and
//! replaces it with a stronger property: this driver only ever uses one of
//! five codecs, and which one is decided by
//! [`sniff::detect`](super::sniff::detect) from the input's own magic bytes.
//! Input that is not one of those five never reaches a codec at all. Same
//! guarantee, enforced before the registry rather than inside it.
//!
//! ## Pixel formats
//!
//! Decoders do not all hand back the same layout, and the `Decoder` trait has
//! no `output_params()` to ask. Guessing from plane geometry is not safe: a
//! palette PNG and an 8-bit greyscale PNG both decode to one plane at one
//! byte per pixel, and reading the former as the latter renders palette
//! *indices* as grey levels - a silently wrong image, the worst possible
//! failure. So:
//!
//! - **PNG** goes through `oxideav_png::decode_png_to_rgba`, the crate's own
//!   entry point that resolves every colour type and bit depth (palette via
//!   `PLTE`/`tRNS`, 16-bit, grey+alpha) to RGBA. No inference at all.
//! - **GIF** is decoded by the framework itself (see the `gif` module): the
//!   first frame only, written straight onto the screen as RGBA, stopping
//!   the moment the frame is complete.
//! - **WebP** and **BMP** go through `decode_webp_image` and `decode_bmp`,
//!   which return one packed RGBA buffer; `Canvas::packed` checks its length.
//! - **JPEG** goes through zune-jpeg, which writes RGBA from YCbCr and grey
//!   and RGB from RGB-coded files, into one buffer the driver allocates. A
//!   lossless JPEG goes through oxideav-mjpeg, whose lossless output is one
//!   packed grey or RGB plane, so the classification is exact rather than a
//!   guess.
//!
//! Everything is normalised to packed RGBA before the first filter runs, so
//! the transformation pipeline only ever deals with one layout.

use oxideav_core::{
    CodecId, CodecParameters, DecoderLimits, Encoder, Frame, Packet, PixelFormat, RuntimeContext,
    TimeBase, VideoFrame, VideoPlane,
};
use oxideav_image_filter::{
    Blur, Crop, Flip, Flop, Grayscale, ImageFilter, Interpolation, Resize, Rotate, Sharpen,
    VideoStreamParams,
};
use oxideav_pixfmt::{
    ConvertOptions, Dither, FrameInfo, PaletteGenOptions, convert as pix_convert, generate_palette,
};

use crate::error::FrameworkError;

use super::ImageConfig;
use super::driver::{ImageDriver, ImagePipeline, OutputFormat, Transformation};
use super::sniff::{self, InputFormat, JpegColour, JpegLayout, ZuneJpeg};

mod gif;
mod peak;
mod webp;

use peak::{Layout, PngLayout};

/// A decoded image in packed RGBA8888, tight stride.
///
/// Every stage of the pipeline sees this one layout, so filters never have to
/// negotiate a format and the encode step always starts from a known base.
#[derive(Debug)]
struct Canvas {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl Canvas {
    /// Build a canvas from packed RGBA, enforcing the type's invariant:
    /// `pixels` is exactly `width * height * 4` bytes.
    ///
    /// Every decode path funnels through here, because the invariant is load
    /// bearing rather than cosmetic. A decoder that returns fewer pixels than
    /// its own header declared (a truncated or lying bitstream) would
    /// otherwise be handed to a filter that indexes by the declared height -
    /// upstream's resize copies rows without a length guard and panics. The
    /// driver contract is no panics on hostile input, so a short buffer is
    /// rejected here as caller input, not discovered later as a fault.
    fn packed(width: u32, height: u32, mut pixels: Vec<u8>) -> Result<Self, FrameworkError> {
        let needed = (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixel_count| pixel_count.checked_mul(4))
            .ok_or_else(|| {
                FrameworkError::param("image dimensions overflow the addressable pixel buffer")
            })?;
        if pixels.len() < needed {
            return Err(FrameworkError::param(format!(
                "image decode produced {} bytes for a declared {width}x{height} image, which \
                 needs {needed}; the bitstream is truncated or its header is inconsistent",
                pixels.len()
            )));
        }
        // A decoder is free to over-allocate its final row; trim so the
        // invariant holds exactly.
        pixels.truncate(needed);
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    fn stream_params(&self) -> VideoStreamParams {
        VideoStreamParams {
            format: PixelFormat::Rgba,
            width: self.width,
            height: self.height,
        }
    }

    /// The canvas as a one-plane RGBA frame. The pixels move into the
    /// frame: a step that turns a canvas into a frame no longer needs the
    /// canvas, and copying them doubled every step's peak.
    fn into_frame(self) -> VideoFrame {
        VideoFrame {
            pts: Some(0),
            planes: vec![VideoPlane {
                stride: self.width as usize * 4,
                data: self.pixels,
            }],
        }
    }

    /// True when every pixel is fully opaque.
    ///
    /// The canvas is always RGBA, and a source format with no alpha channel
    /// decodes with every alpha byte at 255, so such an image is opaque by
    /// construction. One pass over the alpha bytes, reading the pixels in
    /// place and stopping at the first one that is not fully opaque.
    fn is_opaque(&self) -> bool {
        self.pixels
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| pixel[3] == u8::MAX)
    }

    /// Rebuild from a filter's output frame.
    ///
    /// The filters that change shape (resize, crop, rotate) report the new
    /// geometry only through the frame itself. Because the layout is always
    /// RGBA, `stride / 4` and `len / stride` recover it exactly - which is
    /// what lets rotate grow the canvas without the caller predicting by how
    /// much.
    fn from_frame(frame: VideoFrame) -> Result<Self, FrameworkError> {
        let plane = frame
            .planes
            .into_iter()
            .next()
            .ok_or_else(|| FrameworkError::internal("image filter returned no plane"))?;
        if plane.stride == 0 || plane.stride % 4 != 0 {
            return Err(FrameworkError::internal(format!(
                "image filter returned an unexpected RGBA stride of {}",
                plane.stride
            )));
        }
        let width = (plane.stride / 4) as u32;
        let height = (plane.data.len() / plane.stride) as u32;
        if width == 0 || height == 0 {
            return Err(FrameworkError::internal(
                "image filter returned an empty frame",
            ));
        }
        Ok(Self {
            width,
            height,
            pixels: plane.data,
        })
    }
}

/// The pure-Rust image driver: OxideAV codecs, OxideAV filters, no native
/// libraries and nothing to install.
///
/// Holds one `RuntimeContext` with the PNG, JPEG, WebP and BMP codecs
/// registered; GIF is read by the framework and written by
/// `oxideav_gif::encode_rgba8`, outside the registry.
/// Building it is cheap but not free, and it is immutable once built, so the
/// driver is constructed once and shared.
pub struct OxideAvImageDriver {
    context: RuntimeContext,
}

impl OxideAvImageDriver {
    /// Register the supported registry codecs into a fresh runtime context.
    ///
    /// Note `oxideav_bmp::register` takes the two sub-registries separately
    /// rather than the `RuntimeContext` its siblings take - an upstream
    /// inconsistency, not a mistake here.
    pub fn new() -> Self {
        let mut context = RuntimeContext::new();
        oxideav_png::register(&mut context);
        oxideav_mjpeg::register(&mut context);
        oxideav_webp::register(&mut context);
        oxideav_bmp::register(&mut context.codecs, &mut context.containers);
        Self { context }
    }

    /// Run the shared guard, then decode to RGBA.
    fn load(&self, contents: &[u8], config: &ImageConfig) -> Result<Canvas, FrameworkError> {
        if sniff::looks_like_heif(contents) {
            // Deliberately specific rather than falling through to the
            // generic unsupported-format error: iOS clients send HEIC
            // constantly, and "here is why, and here are your two ways
            // forward" is a far more useful answer than a shrug.
            return Err(FrameworkError::param(
                "HEIC is not supported by the oxideav image driver (patent-encumbered; see the \
                 images chapter for why). Convert to JPEG, PNG, or WebP before upload, or set \
                 IMAGE_DRIVER=magick on a host whose ImageMagick has the libheif delegate.",
            ));
        }
        let format = sniff::guard(contents, config)?.ok_or_else(|| {
            FrameworkError::param(
                "image format is not supported: expected PNG, JPEG, WebP, GIF, or BMP",
            )
        })?;
        let (width, height) = sniff::header_dimensions(format, contents)?;
        // The header gate counted the output at four bytes a pixel; the
        // decoders allocate more than that on the way. Refuse what the
        // decode itself would take past the budget. See `peak`.
        let layout = peak::layout(format, contents, width, height)?;
        let needed = peak::estimate(&layout, contents.len() as u64, width, height)?;
        if needed > config.max_alloc_bytes {
            return Err(FrameworkError::param(format!(
                "image exceeds configured decode limits: decoding this {width}x{height} {} \
                 needs about {needed} bytes, over the IMAGE_MAX_ALLOC_BYTES limit of {}",
                format.mime_type(),
                config.max_alloc_bytes
            )));
        }
        // zune-jpeg has no option to skip Extended XMP, and its reassembly
        // costs time with the square of the segment count rather than memory
        // with their bytes, so the estimate above cannot see it. The budget
        // bounds the bytes those passes read instead.
        if let Layout::Jpeg(JpegLayout::Zune(zune)) = &layout
            && zune.xmp_reads > config.max_alloc_bytes
        {
            return Err(FrameworkError::param(format!(
                "image exceeds configured decode limits: reassembling the Extended XMP \
                 segments in this JPEG's headers reads about {} bytes, over the \
                 IMAGE_MAX_ALLOC_BYTES limit of {}",
                zune.xmp_reads, config.max_alloc_bytes
            )));
        }
        self.decode(contents, &layout, width, height, config)
    }

    fn decode(
        &self,
        contents: &[u8],
        layout: &Layout,
        width: u32,
        height: u32,
        config: &ImageConfig,
    ) -> Result<Canvas, FrameworkError> {
        match layout {
            Layout::Png(png) => {
                // The crate's own all-colour-types entry point. See module
                // docs for why PNG does not go through the registry.
                check_png_inflate(contents, png)?;
                let bitmap = oxideav_png::decode_png_to_rgba(contents).map_err(png_error)?;
                Canvas::packed(bitmap.width, bitmap.height, bitmap.data)
            }
            Layout::Gif(first) => gif::decode_first_frame(contents, first, width, height),
            Layout::WebP(_) => {
                let image = oxideav_webp::decode_webp_image(contents).map_err(|e| {
                    FrameworkError::param(format!("image decode failed: image/webp: {e}"))
                })?;
                Canvas::packed(image.width, image.height, image.rgba)
            }
            Layout::Bmp(_) => decode_bmp(contents),
            Layout::Jpeg(JpegLayout::Zune(zune)) => {
                decode_jpeg(contents, zune, config.max_dimension)
            }
            Layout::Jpeg(JpegLayout::Lossless(_)) => {
                let frame = self.decode_via_registry(contents, InputFormat::Jpeg)?;
                let source = jpeg_pixel_format(&frame, width)?;
                to_rgba(frame, source, width, height)
            }
        }
    }

    fn decode_via_registry(
        &self,
        contents: &[u8],
        format: InputFormat,
    ) -> Result<VideoFrame, FrameworkError> {
        let mut params = CodecParameters::video(CodecId::new(format.codec_id()));
        // Inert against the published codecs, which do not read these caps -
        // the framework's own header gate above is what actually enforces
        // them. Set anyway so the day upstream wires `DecoderLimits` up, the
        // second layer is already in place.
        params.limits = decoder_limits(&super::config());

        let mut decoder = self.context.codecs.first_decoder(&params).map_err(|e| {
            FrameworkError::internal(format!(
                "image decode failed: no {} decoder registered: {e}",
                format.codec_id()
            ))
        })?;
        decoder
            .send_packet(&Packet::new(0, TimeBase::new(1, 1), contents.to_vec()))
            .map_err(|e| {
                FrameworkError::param(format!("image decode failed: {}: {e}", format.mime_type()))
            })?;
        match decoder.receive_frame() {
            Ok(Frame::Video(frame)) => Ok(frame),
            Ok(_) => Err(FrameworkError::param(format!(
                "image decode failed: {} produced a non-video frame",
                format.mime_type()
            ))),
            Err(e) => Err(FrameworkError::param(format!(
                "image decode failed: {}: {e}",
                format.mime_type()
            ))),
        }
    }

    fn transform(
        &self,
        mut canvas: Canvas,
        pipeline: &ImagePipeline,
        config: &ImageConfig,
    ) -> Result<Canvas, FrameworkError> {
        for step in &pipeline.transformations {
            canvas = apply(canvas, *step, config)?;
        }
        Ok(canvas)
    }

    /// Encode `canvas`, which the encoder consumes: its pixels move into
    /// the frame the encoder takes.
    fn encode(
        &self,
        canvas: Canvas,
        format: OutputFormat,
        quality: u8,
    ) -> Result<Vec<u8>, FrameworkError> {
        // Every `WebP` canvas `webp_is_lossy` turns down goes to the
        // lossless arm below, rather than into a file that drops the
        // transparency or an encoder that refuses the size.
        if format == OutputFormat::WebP
            && webp_is_lossy(canvas.width, canvas.height, canvas.is_opaque())
        {
            return encode_lossy_webp(canvas, quality);
        }

        let (width, height) = (canvas.width, canvas.height);
        let (codec, frame, pixel_format) = match format {
            // The MJPEG encoder rejects RGBA outright, so the conversion is
            // mandatory rather than an optimisation.
            OutputFormat::Jpeg => (
                "mjpeg",
                convert_frame(&canvas.into_frame(), width, height, PixelFormat::Rgb24)?,
                PixelFormat::Rgb24,
            ),
            OutputFormat::Png => ("png", canvas.into_frame(), PixelFormat::Rgba),
            // The VP8L (lossless) encoder is the only WebP encoder in the
            // registry; codec id "webp" has a decoder but no encoder. `WebP`
            // reaches this arm only when `webp_is_lossy` says no.
            OutputFormat::WebP | OutputFormat::WebPLossless => {
                ("webp_vp8l", canvas.into_frame(), PixelFormat::Rgba)
            }
            OutputFormat::Gif => return encode_gif(canvas),
            OutputFormat::Bmp => ("bmp", canvas.into_frame(), PixelFormat::Rgba),
        };

        let mut params = CodecParameters::video(CodecId::new(codec));
        params.width = Some(width);
        params.height = Some(height);
        params.pixel_format = Some(pixel_format);
        // Only JPEG has a quality knob that does anything here. Passing the
        // option to PNG is not merely useless, it is fatal: that encoder
        // rejects unknown options outright.
        if matches!(format, OutputFormat::Jpeg) {
            params.options = params.options.set("quality", quality.to_string());
        }

        let mut encoder = self.context.codecs.first_encoder(&params).map_err(|e| {
            FrameworkError::internal(format!("image encode failed: no {codec} encoder: {e}"))
        })?;
        encode_frame(encoder.as_mut(), frame, codec)
    }
}

impl Default for OxideAvImageDriver {
    fn default() -> Self {
        Self::new()
    }
}

impl ImageDriver for OxideAvImageDriver {
    fn process(
        &self,
        contents: &[u8],
        pipeline: &ImagePipeline,
    ) -> Result<Vec<u8>, FrameworkError> {
        let config = super::config();
        let canvas = self.load(contents, &config)?;
        let source_format = sniff::detect(contents);
        let canvas = self.transform(canvas, pipeline, &config)?;
        let target = pipeline
            .format
            .or_else(|| source_format.and_then(output_for_input))
            // Only reachable if a format was recognised on the way in and has
            // no encoder counterpart, which cannot happen for these five.
            .unwrap_or(OutputFormat::Png);
        self.encode(canvas, target, pipeline.quality)
    }

    fn dimensions(&self, contents: &[u8]) -> Result<(u32, u32), FrameworkError> {
        let config = super::config();
        let canvas = self.load(contents, &config)?;
        Ok((canvas.width, canvas.height))
    }

    fn dominant_color(&self, contents: &[u8]) -> Result<String, FrameworkError> {
        let config = super::config();
        let canvas = self.load(contents, &config)?;
        Ok(average_color(&canvas))
    }

    fn name(&self) -> &'static str {
        "oxideav"
    }
}

// ───────────────────────── decode helpers ─────────────────────────

fn png_error(error: oxideav_png::PngError) -> FrameworkError {
    FrameworkError::param(format!("image decode failed: png: {error}"))
}

/// Refuse PNG pixel data that inflates past the size its header declares.
///
/// The header gate measures the IHDR dimensions, and `oxideav-png` then
/// inflates every IDAT byte with an inflater that has no output limit, and
/// only afterwards compares the result with those dimensions. A few
/// kilobytes of zlib can expand to gigabytes, so a file that declares one
/// pixel could make the decoder allocate that much before refusing it.
/// Inflating once here, capped at the exact length the header implies, stops
/// at that bound instead. A valid file never reaches the cap: the decoder
/// rejects any other length.
///
/// The chunk walk (`read_chunk`) and the inflater (`compcol`'s zlib) are the
/// ones `oxideav-png` uses, so both passes see the same bytes. This pass
/// keeps none of what it inflates, so it costs one scratch buffer rather
/// than a second copy of the pixel data.
fn check_png_inflate(contents: &[u8], png: &PngLayout) -> Result<(), FrameworkError> {
    let ihdr = &png.ihdr;
    let declared = png_inflated_len(ihdr).ok_or_else(|| {
        FrameworkError::param(format!(
            "image decode failed: png: colour type {} at bit depth {} with interlace method {} \
             is not a PNG pixel format, or is too large to decode",
            ihdr.colour_type, ihdr.bit_depth, ihdr.interlace
        ))
    })?;
    let mut idat = Vec::with_capacity(usize::try_from(png.idat_len).unwrap_or(0));
    peak::for_each_png_chunk(contents, |chunk| {
        if chunk.is_type(b"IDAT") {
            idat.extend_from_slice(chunk.data);
        }
        Ok(())
    })?;
    match inflate_within(&idat, declared) {
        Ok(()) => Ok(()),
        Err(compcol::Error::OutputLimitExceeded) => Err(FrameworkError::param(format!(
            "image is malformed: its PNG pixel data inflates past the {declared} bytes its \
             {}x{} header allows",
            ihdr.width, ihdr.height
        ))),
        Err(e) => Err(FrameworkError::param(format!(
            "image decode failed: png: the pixel data does not inflate: {e}"
        ))),
    }
}

/// Inflate a zlib stream without keeping its output, failing with
/// `OutputLimitExceeded` once it produces more than `limit` bytes.
///
/// The loop is `compcol::vec::decompress_to_vec_capped`'s, including its
/// guard against a decoder that stops making progress, with the output
/// written into one scratch buffer and dropped.
fn inflate_within(data: &[u8], limit: u64) -> Result<(), compcol::Error> {
    use compcol::{Algorithm, Decoder, Status};

    let mut decoder = compcol::limit::LimitedDecoder::new(compcol::zlib::Zlib::decoder(), limit);
    let mut scratch = vec![0u8; 64 * 1024];
    let mut consumed = 0;
    while consumed < data.len() {
        let (progress, status) = decoder.decode(&data[consumed..], &mut scratch)?;
        consumed += progress.consumed;
        match status {
            Status::StreamEnd => return Ok(()),
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
        if status == Status::StreamEnd {
            return Ok(());
        }
        if progress.written == 0 {
            return Err(compcol::Error::Corrupt);
        }
    }
}

/// Adam7's seven passes, as (first row, first column, row step, column step).
const ADAM7_PASSES: [(u64, u64, u64, u64); 7] = [
    (0, 0, 8, 8),
    (0, 4, 8, 8),
    (4, 0, 8, 4),
    (0, 2, 4, 4),
    (2, 0, 4, 2),
    (0, 1, 2, 2),
    (1, 0, 2, 1),
];

/// The exact length a PNG's pixel data inflates to, from its header alone.
///
/// Every scanline is one filter byte plus its packed samples, and an
/// interlaced image is seven smaller images, one per Adam7 pass. `None` for
/// a colour type, bit depth, or interlace method PNG does not define, which
/// the decoder refuses as well, and for a length too large for a `u64`.
fn png_inflated_len(ihdr: &oxideav_png::Ihdr) -> Option<u64> {
    let channels: u64 = match (ihdr.colour_type, ihdr.bit_depth) {
        (0, 1 | 2 | 4 | 8 | 16) | (3, 1 | 2 | 4 | 8) => 1,
        (4, 8 | 16) => 2,
        (2, 8 | 16) => 3,
        (6, 8 | 16) => 4,
        _ => return None,
    };
    let bits_per_pixel = channels * u64::from(ihdr.bit_depth);
    let scanlines = |width: u64, height: u64| -> Option<u64> {
        if width == 0 || height == 0 {
            return Some(0);
        }
        let row = width.checked_mul(bits_per_pixel)?.div_ceil(8);
        row.checked_add(1)?.checked_mul(height)
    };
    let (width, height) = (u64::from(ihdr.width), u64::from(ihdr.height));
    match ihdr.interlace {
        0 => scanlines(width, height),
        1 => ADAM7_PASSES
            .iter()
            .try_fold(0u64, |total, &(row, column, row_step, column_step)| {
                let pass_width = width.saturating_sub(column).div_ceil(column_step);
                let pass_height = height.saturating_sub(row).div_ceil(row_step);
                total.checked_add(scanlines(pass_width, pass_height)?)
            }),
        _ => None,
    }
}

fn decoder_limits(config: &ImageConfig) -> DecoderLimits {
    let max_pixels =
        u64::from(config.max_dimension).saturating_mul(u64::from(config.max_dimension));
    DecoderLimits::default()
        .with_max_pixels_per_frame(max_pixels)
        .with_max_alloc_bytes_per_frame(config.max_alloc_bytes)
}

/// Which `OutputFormat` re-encodes an input format unchanged.
fn output_for_input(format: InputFormat) -> Option<OutputFormat> {
    Some(match format {
        InputFormat::Png => OutputFormat::Png,
        InputFormat::Jpeg => OutputFormat::Jpeg,
        InputFormat::WebP => OutputFormat::WebP,
        InputFormat::Gif => OutputFormat::Gif,
        InputFormat::Bmp => OutputFormat::Bmp,
    })
}

/// Determine the layout oxideav-mjpeg handed back for a lossless JPEG.
///
/// Its lossless decoder takes only unsubsampled frames and returns one
/// packed plane, grey or RGB, so the classification is exact rather than a
/// guess. JPEG is the one format decoded through the codec registry, and
/// only when it is lossless; see `decode_jpeg` for the rest.
fn jpeg_pixel_format(frame: &VideoFrame, width: u32) -> Result<PixelFormat, FrameworkError> {
    let unsupported = |detail: &str| {
        FrameworkError::param(format!(
            "image decode produced an unsupported pixel layout ({detail}); convert the source \
             to 8-bit RGB or RGBA and retry"
        ))
    };
    let [plane] = frame.planes.as_slice() else {
        return Err(unsupported(&format!("{} planes", frame.planes.len())));
    };
    match plane.stride.checked_div(width as usize).unwrap_or(0) {
        1 => Ok(PixelFormat::Gray8),
        3 => Ok(PixelFormat::Rgb24),
        other => Err(unsupported(&format!("{other} bytes per pixel"))),
    }
}

/// Convert a decoded layout to packed RGBA with a tight stride.
///
/// Takes the frame by value so its planes can move into the canvas: the
/// source planes are dropped as soon as the conversion is done, and a plane
/// that is already tight RGBA is never copied.
fn to_rgba(
    frame: VideoFrame,
    source: PixelFormat,
    width: u32,
    height: u32,
) -> Result<Canvas, FrameworkError> {
    let converted = if source == PixelFormat::Rgba {
        frame
    } else {
        let info = FrameInfo::new(source, width, height);
        pix_convert(&frame, info, PixelFormat::Rgba, &ConvertOptions::default())
            .map_err(|e| FrameworkError::param(format!("image pixel conversion failed: {e}")))?
    };
    // `Canvas::packed` is what rejects a plane shorter than the declared
    // height rather than handing it to a filter that would index past the
    // end of it.
    let pixels = pack_tight(converted, width as usize * 4, height as usize)?;
    Canvas::packed(width, height, pixels)
}

/// Strip any per-row padding a conversion left behind. A plane that is
/// already tight moves out of the frame, trimmed to `height` rows; one that
/// is short is returned as it is, for `Canvas::packed` to refuse.
fn pack_tight(frame: VideoFrame, tight: usize, height: usize) -> Result<Vec<u8>, FrameworkError> {
    let plane = frame
        .planes
        .into_iter()
        .next()
        .ok_or_else(|| FrameworkError::internal("pixel conversion produced no plane"))?;
    if plane.stride == tight {
        let mut data = plane.data;
        data.truncate(tight.saturating_mul(height));
        return Ok(data);
    }
    let short = || FrameworkError::internal("pixel conversion produced a short plane");
    let mut out = Vec::with_capacity(tight * height);
    for row in 0..height {
        let start = row
            .checked_mul(plane.stride)
            .ok_or_else(|| FrameworkError::internal("pixel conversion row offset overflow"))?;
        out.extend_from_slice(plane.data.get(start..start + tight).ok_or_else(short)?);
    }
    Ok(out)
}

/// Decode a JPEG through zune-jpeg into one RGBA buffer.
///
/// zune-jpeg converts YCbCr and greyscale to RGBA itself. It has no RGB to
/// RGBA mapping, so an RGB-coded JPEG decodes as RGB into the front of the
/// same buffer and is spread to RGBA in place, back to front. The colour
/// space comes from the header walk the estimate used, and the decoder must
/// agree with it before anything is allocated. Its size limits are the
/// framework's: the decode estimate decides what is admitted, not the
/// crate's own defaults.
fn decode_jpeg(
    contents: &[u8],
    zune: &ZuneJpeg,
    max_dimension: u32,
) -> Result<Canvas, FrameworkError> {
    use zune_jpeg::zune_core::bytestream::ZCursor;
    use zune_jpeg::zune_core::colorspace::ColorSpace;
    use zune_jpeg::zune_core::options::DecoderOptions;

    let failed = |e: &dyn std::fmt::Display| {
        FrameworkError::param(format!("image decode failed: image/jpeg: {e}"))
    };
    let (input, output, channels) = match zune.colour {
        JpegColour::Rgb => (ColorSpace::RGB, ColorSpace::RGB, 3),
        JpegColour::YCbCr => (ColorSpace::YCbCr, ColorSpace::RGBA, 4),
        JpegColour::Grey => (ColorSpace::Luma, ColorSpace::RGBA, 4),
        JpegColour::Other => return Err(failed(&"the colour space is not supported")),
    };
    let limit = max_dimension as usize;
    let options = DecoderOptions::default()
        .set_max_width(limit)
        .set_max_height(limit)
        .jpeg_set_out_colorspace(output);
    let mut decoder = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(contents), options);
    decoder.decode_headers().map_err(|e| failed(&e))?;
    if decoder.input_colorspace() != Some(input) {
        return Err(failed(&"the decoder read a different colour space"));
    }
    let dimensions = decoder.dimensions().map(|(w, h)| (w as u32, h as u32));
    if dimensions != Some((zune.frame.width, zune.frame.height)) {
        return Err(failed(&"the decoder read a different frame"));
    }
    let (width, height) = (zune.frame.width as usize, zune.frame.height as usize);
    let pixels = width * height;
    let mut rgba = vec![0u8; pixels * 4];
    let decoded = rgba
        .get_mut(..pixels * channels)
        .ok_or_else(|| failed(&"the output buffer is short"))?;
    decoder.decode_into(decoded).map_err(|e| failed(&e))?;
    if channels == 3 {
        for pixel in (0..pixels).rev() {
            rgba.copy_within(pixel * 3..pixel * 3 + 3, pixel * 4);
            rgba[pixel * 4 + 3] = u8::MAX;
        }
    }
    Canvas::packed(zune.frame.width, zune.frame.height, rgba)
}

/// Decode a BMP through the crate's own entry point, which always returns
/// one tight RGBA plane.
fn decode_bmp(contents: &[u8]) -> Result<Canvas, FrameworkError> {
    let image = oxideav_bmp::decode_bmp(contents)
        .map_err(|e| FrameworkError::param(format!("image decode failed: image/bmp: {e}")))?;
    let plane = image
        .planes
        .into_iter()
        .next()
        .ok_or_else(|| FrameworkError::param("image decode produced no planes"))?;
    if image.pixel_format != oxideav_bmp::image::BmpPixelFormat::Rgba
        || plane.stride != image.width as usize * 4
    {
        return Err(FrameworkError::param(
            "image decode produced an unsupported pixel layout (expected packed RGBA)",
        ));
    }
    Canvas::packed(image.width, image.height, plane.data)
}

// ───────────────────────── transformation helpers ─────────────────────────

/// Round a scale factor onto a pixel count, never yielding zero.
fn scaled(value: u32, factor: f64) -> u32 {
    let scaled = (f64::from(value) * factor).round();
    if scaled < 1.0 {
        1
    } else if scaled > f64::from(u32::MAX) {
        u32::MAX
    } else {
        scaled as u32
    }
}

fn apply(
    canvas: Canvas,
    step: Transformation,
    config: &ImageConfig,
) -> Result<Canvas, FrameworkError> {
    let (w, h) = (canvas.width, canvas.height);
    match step {
        Transformation::Resize { width, height } => resize(canvas, width, height, config),
        Transformation::ResizeWidth(width) => {
            let factor = f64::from(width) / f64::from(w);
            resize(canvas, width, scaled(h, factor), config)
        }
        Transformation::ResizeHeight(height) => {
            let factor = f64::from(height) / f64::from(h);
            resize(canvas, scaled(w, factor), height, config)
        }
        Transformation::Scale { width, height } => {
            let factor = fit_factor(w, h, width, height).min(1.0);
            resize(canvas, scaled(w, factor), scaled(h, factor), config)
        }
        Transformation::ScaleWidth(width) => {
            let factor = (f64::from(width) / f64::from(w)).min(1.0);
            resize(canvas, scaled(w, factor), scaled(h, factor), config)
        }
        Transformation::ScaleHeight(height) => {
            let factor = (f64::from(height) / f64::from(h)).min(1.0);
            resize(canvas, scaled(w, factor), scaled(h, factor), config)
        }
        Transformation::Contain { width, height } => {
            let factor = fit_factor(w, h, width, height);
            resize(canvas, scaled(w, factor), scaled(h, factor), config)
        }
        Transformation::Cover { width, height } => cover(canvas, width, height, config),
        Transformation::Crop {
            width,
            height,
            x,
            y,
        } => crop(canvas, x, y, width, height),
        Transformation::Rotate(degrees) => rotate(canvas, degrees, config),
        Transformation::FlipVertically => filter(canvas, &Flip::new()),
        Transformation::FlipHorizontally => filter(canvas, &Flop::new()),
        Transformation::Blur(amount) => match blur_radius(amount) {
            Some(radius) => filter(canvas, &Blur::new(radius).with_sigma(radius as f32 / 2.0)),
            None => Ok(canvas),
        },
        Transformation::Sharpen(amount) => match sharpen_strength(amount) {
            Some(strength) => filter(canvas, &Sharpen::new(1, 0.5).with_amount(strength)),
            None => Ok(canvas),
        },
        Transformation::Grayscale => filter(
            canvas,
            // Stay in RGBA rather than collapsing to Gray8: every later stage
            // assumes one layout, and the encoders take RGBA.
            &Grayscale::new()
                .with_preserve_alpha(true)
                .with_output_gray8(false),
        ),
    }
}

/// The factor that fits `w x h` inside `target_w x target_h`.
fn fit_factor(w: u32, h: u32, target_w: u32, target_h: u32) -> f64 {
    let by_width = f64::from(target_w) / f64::from(w);
    let by_height = f64::from(target_h) / f64::from(h);
    by_width.min(by_height)
}

/// Map a `0..=100` blur strength onto a Gaussian radius.
///
/// `0` means "do nothing" and skips the filter entirely; `1` is the smallest
/// visible blur and `100` maps to a radius of 15, past which a separable
/// Gaussian on a web-sized image is indistinguishable mush.
fn blur_radius(amount: u32) -> Option<u32> {
    let amount = amount.min(100);
    if amount == 0 {
        return None;
    }
    let radius = ((f64::from(amount) / 100.0) * 15.0).ceil() as u32;
    Some(radius.max(1))
}

/// Map a `0..=100` sharpen strength onto an unsharp-mask amount.
///
/// `0` skips the filter. `50` maps to `1.0`, the classic unsharp amount the
/// filter documents, so the scale has the conventional setting in the middle
/// and `100` at twice that.
fn sharpen_strength(amount: u32) -> Option<f32> {
    let amount = amount.min(100);
    if amount == 0 {
        return None;
    }
    Some(amount as f32 / 50.0)
}

fn filter(canvas: Canvas, image_filter: &dyn ImageFilter) -> Result<Canvas, FrameworkError> {
    let params = canvas.stream_params();
    let out = image_filter
        .apply(&canvas.into_frame(), params)
        .map_err(|e| FrameworkError::param(format!("image transformation failed: {e}")))?;
    Canvas::from_frame(out)
}

/// Resize, re-applying the decode caps to the *target*.
///
/// An oversized target is the same denial-of-service as an oversized source -
/// `.resize(50_000, 50_000)` allocates 10 GB whether the pixels came from a
/// user or from a mistyped constant.
fn resize(
    canvas: Canvas,
    width: u32,
    height: u32,
    config: &ImageConfig,
) -> Result<Canvas, FrameworkError> {
    let width = width.max(1);
    let height = height.max(1);
    sniff::enforce_limits(width, height, config)?;
    if width == canvas.width && height == canvas.height {
        return Ok(canvas);
    }
    filter(
        canvas,
        // Bilinear is the only smooth kernel the filter crate ships; its
        // `Interpolation` enum has no Lanczos/Area/Bicubic despite what the
        // README advertises. It is the crate's documented default for
        // natural images.
        &Resize::new(width, height).with_interpolation(Interpolation::Bilinear),
    )
}

/// Rotate, re-applying the decode caps to the *grown* canvas.
///
/// Rotation is the other shape-changing transformation, and the only one whose
/// output is larger than anything the caller named: a 45-degree turn grows
/// each side by up to sqrt(2), so an image sitting exactly on
/// `IMAGE_MAX_DIMENSION` would land 1.41x over it. Predicting the extent and
/// checking it first keeps the cap meaningful for the same reason `resize`
/// checks its target.
fn rotate(canvas: Canvas, degrees: f32, config: &ImageConfig) -> Result<Canvas, FrameworkError> {
    let (width, height) = rotated_extent(canvas.width, canvas.height, degrees);
    sniff::enforce_limits(width, height, config)?;
    filter(canvas, &Rotate::new(degrees))
}

/// The bounding box of `width x height` rotated by `degrees`, matching the
/// filter's own forward-transformed extent (exact for quarter turns, which it
/// fast-paths without resampling).
fn rotated_extent(width: u32, height: u32, degrees: f32) -> (u32, u32) {
    let normalised = degrees.rem_euclid(360.0);
    if (normalised % 90.0).abs() < f32::EPSILON {
        // Quarter turns swap the axes or leave them alone; no growth.
        return if (normalised - 90.0).abs() < f32::EPSILON
            || (normalised - 270.0).abs() < f32::EPSILON
        {
            (height, width)
        } else {
            (width, height)
        };
    }
    let radians = f64::from(normalised).to_radians();
    let (sin, cos) = (radians.sin().abs(), radians.cos().abs());
    let w = f64::from(width);
    let h = f64::from(height);
    let grown = |value: f64| -> u32 {
        let ceiled = value.ceil();
        if ceiled < 1.0 {
            1
        } else if ceiled > f64::from(u32::MAX) {
            u32::MAX
        } else {
            ceiled as u32
        }
    };
    (grown(w * cos + h * sin), grown(w * sin + h * cos))
}

fn crop(canvas: Canvas, x: u32, y: u32, width: u32, height: u32) -> Result<Canvas, FrameworkError> {
    if width == 0 || height == 0 {
        return Err(FrameworkError::param(
            "image crop width and height must both be greater than zero",
        ));
    }
    let exceeds_width = x.checked_add(width).is_none_or(|edge| edge > canvas.width);
    let exceeds_height = y
        .checked_add(height)
        .is_none_or(|edge| edge > canvas.height);
    if exceeds_width || exceeds_height {
        return Err(FrameworkError::param(format!(
            "image crop {width}x{height}+{x}+{y} falls outside the {}x{} image",
            canvas.width, canvas.height
        )));
    }
    filter(canvas, &Crop::new(x, y, width, height))
}

/// Aspect-fill then centre-crop, Laravel's `cover`.
fn cover(
    canvas: Canvas,
    width: u32,
    height: u32,
    config: &ImageConfig,
) -> Result<Canvas, FrameworkError> {
    let width = width.max(1);
    let height = height.max(1);
    let by_width = f64::from(width) / f64::from(canvas.width);
    let by_height = f64::from(height) / f64::from(canvas.height);
    let factor = by_width.max(by_height);
    // Round up so the intermediate never falls a pixel short of the crop.
    let filled_w = scaled(canvas.width, factor).max(width);
    let filled_h = scaled(canvas.height, factor).max(height);
    let filled = resize(canvas, filled_w, filled_h, config)?;
    let x = (filled.width.saturating_sub(width)) / 2;
    let y = (filled.height.saturating_sub(height)) / 2;
    crop(filled, x, y, width, height)
}

// ───────────────────────── encode helpers ─────────────────────────

fn convert_frame(
    frame: &VideoFrame,
    width: u32,
    height: u32,
    target: PixelFormat,
) -> Result<VideoFrame, FrameworkError> {
    let info = FrameInfo::new(PixelFormat::Rgba, width, height);
    pix_convert(frame, info, target, &ConvertOptions::default())
        .map_err(|e| FrameworkError::internal(format!("image pixel conversion failed: {e}")))
}

/// The largest width or height a VP8 frame can have.
///
/// The VP8 key frame header stores each dimension in 14 bits (RFC 6386,
/// section 9.1), and the upstream lossy encoder refuses anything larger.
/// VP8L stores `size - 1` in its 14 bits, so lossless WebP reaches one
/// pixel further, to 16384.
const VP8_MAX_DIMENSION: u32 = 16_383;

/// Whether [`OutputFormat::WebP`] takes the lossy encoder for a canvas of
/// this size and opacity.
///
/// Lossy needs both an opaque canvas, because the lossy encoder has no
/// alpha channel, and sides a VP8 frame can have. Anything else is written
/// lossless, which keeps the transparency and takes the larger size.
fn webp_is_lossy(width: u32, height: u32, opaque: bool) -> bool {
    opaque && width <= VP8_MAX_DIMENSION && height <= VP8_MAX_DIMENSION
}

/// Encode an opaque canvas as lossy WebP: a simple container around one
/// `VP8 ` bitstream, at `quality` on the `0..=100` WebP scale.
///
/// Upstream reserves the `webp_vp8` codec id but registers no factory under
/// it, so the encoder is built directly instead of looked up in the
/// registry. It is built before the pixels are converted, so a parameter
/// the factory refuses costs no conversion.
fn encode_lossy_webp(canvas: Canvas, quality: u8) -> Result<Vec<u8>, FrameworkError> {
    let codec = oxideav_webp::CODEC_ID_VP8;
    let mut params = CodecParameters::video(CodecId::new(codec));
    params.width = Some(canvas.width);
    params.height = Some(canvas.height);
    params.pixel_format = Some(PixelFormat::Yuv420P);
    let mut encoder =
        oxideav_webp::encoder_vp8::make_encoder_with_quality(&params, f32::from(quality))
            .map_err(|e| FrameworkError::internal(format!("image encode failed: {codec}: {e}")))?;
    let frame = yuv420_frame(canvas)?;
    encode_frame(encoder.as_mut(), frame, codec)
}

/// Convert the canvas to the planar 4:2:0 layout the VP8 encoder takes.
///
/// The converter's default colour space, BT.601 limited range, is the one
/// WebP decoders assume, so no option is set.
///
/// The converter refuses odd dimensions, because one chroma sample covers a
/// 2x2 block. An odd right or bottom edge is therefore extended by one
/// column or row that repeats the edge, which is also how the encoder pads
/// a partial macroblock. The encoder is still told the true size, so it
/// reads no padded luma, and each edge chroma sample is the average of the
/// real pixels it covers.
fn yuv420_frame(canvas: Canvas) -> Result<VideoFrame, FrameworkError> {
    let even = |side: u32| {
        side.checked_next_multiple_of(2).ok_or_else(|| {
            FrameworkError::internal("image dimensions overflow the addressable pixel buffer")
        })
    };
    let (width, height) = (even(canvas.width)?, even(canvas.height)?);
    let frame = if width == canvas.width && height == canvas.height {
        canvas.into_frame()
    } else {
        edge_extended_frame(&canvas, width, height)?
    };
    let info = FrameInfo::new(PixelFormat::Rgba, width, height);
    pix_convert(
        &frame,
        info,
        PixelFormat::Yuv420P,
        &ConvertOptions::default(),
    )
    .map_err(|e| FrameworkError::internal(format!("image pixel conversion failed: {e}")))
}

/// Copy the canvas into an RGBA frame of `width x height`, filling the
/// extra columns with each row's last pixel and the extra rows with the
/// last row.
fn edge_extended_frame(
    canvas: &Canvas,
    width: u32,
    height: u32,
) -> Result<VideoFrame, FrameworkError> {
    let row_bytes = canvas.width as usize * 4;
    let stride = width as usize * 4;
    let last_row = canvas.height.saturating_sub(1) as usize;
    let mut data = Vec::with_capacity(stride * height as usize);
    for y in 0..height as usize {
        let start = y.min(last_row) * row_bytes;
        let row = canvas
            .pixels
            .get(start..start + row_bytes)
            .ok_or_else(|| FrameworkError::internal("image canvas is shorter than its size"))?;
        data.extend_from_slice(row);
        if let Some(edge) = row.last_chunk::<4>() {
            for _ in canvas.width..width {
                data.extend_from_slice(edge);
            }
        }
    }
    Ok(VideoFrame {
        pts: Some(0),
        planes: vec![VideoPlane { stride, data }],
    })
}

/// Write a canvas as a single-frame GIF.
///
/// The canvas is reduced to at most 256 colours first, with Floyd-Steinberg
/// dithering, so `oxideav_gif::encode_rgba8` takes its colours as they are;
/// that encoder would otherwise quantise with a plain median cut. The
/// reduction maps every pixel to an opaque palette colour, so the GIF has no
/// transparent index.
fn encode_gif(canvas: Canvas) -> Result<Vec<u8>, FrameworkError> {
    let (width, height) = (canvas.width, canvas.height);
    let frame = quantise_for_gif(canvas)?;
    let rgba = frame
        .planes
        .into_iter()
        .next()
        .ok_or_else(|| FrameworkError::internal("gif quantisation produced no plane"))?
        .data;
    oxideav_gif::encode_rgba8(width, height, &rgba, &oxideav_gif::EncodeOptions::default())
        .map_err(|e| FrameworkError::internal(format!("image encode failed: gif: {e}")))
}

/// Reduce a full-colour frame to at most 256 colours for the GIF encoder.
///
/// `oxideav-pixfmt` will not convert to `Pal8` without a caller-supplied
/// palette, so the palette is generated explicitly, the frame is mapped
/// through it with Floyd-Steinberg dithering, and mapped straight back to
/// RGBA - which now holds at most 256 distinct colours.
fn quantise_for_gif(canvas: Canvas) -> Result<VideoFrame, FrameworkError> {
    let (width, height) = (canvas.width, canvas.height);
    let frame = canvas.into_frame();
    let info = FrameInfo::new(PixelFormat::Rgba, width, height);
    let palette = generate_palette(&[(&frame, info)], &PaletteGenOptions::default())
        .map_err(|e| FrameworkError::internal(format!("gif palette generation failed: {e}")))?;

    let to_indexed = ConvertOptions {
        dither: Dither::FloydSteinberg,
        palette: Some(palette.clone()),
        ..Default::default()
    };
    let indexed = pix_convert(&frame, info, PixelFormat::Pal8, &to_indexed)
        .map_err(|e| FrameworkError::internal(format!("gif quantisation failed: {e}")))?;

    let indexed_info = FrameInfo::new(PixelFormat::Pal8, width, height);
    let from_indexed = ConvertOptions {
        palette: Some(palette),
        ..Default::default()
    };
    let reduced = pix_convert(&indexed, indexed_info, PixelFormat::Rgba, &from_indexed)
        .map_err(|e| FrameworkError::internal(format!("gif quantisation failed: {e}")))?;

    let tight = width as usize * 4;
    let pixels = pack_tight(reduced, tight, height as usize)?;
    Ok(VideoFrame {
        pts: Some(0),
        planes: vec![VideoPlane {
            stride: tight,
            data: pixels,
        }],
    })
}

/// Feed a still image's one frame to `encoder` and collect the file it
/// emits.
fn encode_frame(
    encoder: &mut dyn Encoder,
    frame: VideoFrame,
    codec: &str,
) -> Result<Vec<u8>, FrameworkError> {
    encoder
        .send_frame(&Frame::Video(frame))
        .map_err(|e| FrameworkError::internal(format!("image encode failed: {codec}: {e}")))?;
    encoder
        .flush()
        .map_err(|e| FrameworkError::internal(format!("image encode failed: {codec}: {e}")))?;
    drain(encoder, codec)
}

/// Collect the encoded file out of an encoder.
///
/// Every still-image encoder here emits the complete file as a single packet.
/// The drain has to tolerate one upstream wart: `oxideav-gif` signals "no
/// more packets" with `Error::InvalidData` instead of `NeedMore`/`Eof`, so a
/// naive loop reads a successful encode as a failure. Errors are therefore
/// only fatal before the first packet arrives - after that they mean the
/// stream is drained.
fn drain(encoder: &mut dyn Encoder, codec: &str) -> Result<Vec<u8>, FrameworkError> {
    let mut out = Vec::new();
    loop {
        match encoder.receive_packet() {
            Ok(packet) => out.extend_from_slice(&packet.data),
            Err(oxideav_core::Error::NeedMore) | Err(oxideav_core::Error::Eof) => break,
            Err(e) => {
                if out.is_empty() {
                    return Err(FrameworkError::internal(format!(
                        "image encode failed: {codec}: {e}"
                    )));
                }
                break;
            }
        }
    }
    if out.is_empty() {
        return Err(FrameworkError::internal(format!(
            "image encode failed: {codec} produced no output"
        )));
    }
    Ok(out)
}

/// The mean colour of the canvas as `#rrggbb`.
///
/// This is the coverage-weighted average an "area" downscale to 1x1 would
/// produce, computed directly because the filter crate ships no area kernel.
/// Alpha is dropped, matching Laravel.
fn average_color(canvas: &Canvas) -> String {
    let mut totals = [0u64; 3];
    let mut count = 0u64;
    for pixel in canvas.pixels.as_chunks::<4>().0 {
        totals[0] += u64::from(pixel[0]);
        totals[1] += u64::from(pixel[1]);
        totals[2] += u64::from(pixel[2]);
        count += 1;
    }
    if count == 0 {
        return "#000000".to_string();
    }
    let channel = |total: u64| -> u8 {
        // Round to nearest rather than truncating, so a uniform image round
        // trips to exactly its own colour.
        (((total * 2) + count) / (count * 2)).min(255) as u8
    };
    format!(
        "#{:02x}{:02x}{:02x}",
        channel(totals[0]),
        channel(totals[1]),
        channel(totals[2])
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED_PNG_1X1: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90,
        0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x78, 0xDA, 0x63, 0xF8,
        0xCF, 0xC0, 0x00, 0x00, 0x03, 0x01, 0x01, 0x00, 0xF7, 0x03, 0x41, 0x43, 0x00, 0x00, 0x00,
        0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    fn canvas(width: u32, height: u32, rgba: [u8; 4]) -> Canvas {
        Canvas {
            width,
            height,
            pixels: rgba.repeat((width * height) as usize),
        }
    }

    fn ihdr(
        width: u32,
        height: u32,
        colour_type: u8,
        bit_depth: u8,
        interlace: u8,
    ) -> oxideav_png::Ihdr {
        oxideav_png::Ihdr {
            width,
            height,
            bit_depth,
            colour_type,
            compression: 0,
            filter: 0,
            interlace,
        }
    }

    /// A PNG with this header and `raw` as its inflated pixel data, and a
    /// sixteen-entry palette when the header names colour type 3.
    fn png_with_pixel_data(header: oxideav_png::Ihdr, raw: &[u8]) -> Vec<u8> {
        let idat = compcol::vec::compress_to_vec::<compcol::zlib::Zlib>(raw).expect("zlib");
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        oxideav_png::chunk::write_chunk(&mut png, b"IHDR", &header.to_bytes());
        if header.colour_type == 3 {
            oxideav_png::chunk::write_chunk(&mut png, b"PLTE", &[0u8; 48]);
        }
        oxideav_png::chunk::write_chunk(&mut png, b"IDAT", &idat);
        oxideav_png::chunk::write_chunk(&mut png, b"IEND", &[]);
        png
    }

    /// The chunk layout of a PNG, as `load` reads it.
    fn png_layout(png: &[u8]) -> PngLayout {
        let (width, height) =
            sniff::header_dimensions(InputFormat::Png, png).expect("PNG dimensions");
        match peak::layout(InputFormat::Png, png, width, height).expect("a PNG layout") {
            Layout::Png(layout) => layout,
            _ => panic!("a PNG reads as a PNG layout"),
        }
    }

    #[test]
    fn png_inflated_length_follows_the_header() {
        // 8-bit RGBA: one filter byte and four bytes a pixel, per row.
        assert_eq!(png_inflated_len(&ihdr(1, 1, 6, 8, 0)), Some(5));
        assert_eq!(png_inflated_len(&ihdr(10, 3, 6, 8, 0)), Some(3 * 41));
        // 16-bit RGB: six bytes a pixel.
        assert_eq!(png_inflated_len(&ihdr(2, 2, 2, 16, 0)), Some(2 * 13));
        // 1-bit grey packs eight pixels a byte, rounded up per row.
        assert_eq!(png_inflated_len(&ihdr(13, 5, 0, 1, 0)), Some(5 * 3));
        // Adam7 on 3x3 at 1 bit: passes 2 and 3 are empty, pass 6 has two
        // rows, and every other pass one row of one byte.
        assert_eq!(png_inflated_len(&ihdr(3, 3, 0, 1, 1)), Some(12));
        // Combinations PNG does not define.
        assert_eq!(png_inflated_len(&ihdr(1, 1, 2, 4, 0)), None);
        assert_eq!(png_inflated_len(&ihdr(1, 1, 3, 16, 0)), None);
        assert_eq!(png_inflated_len(&ihdr(1, 1, 5, 8, 0)), None);
        assert_eq!(png_inflated_len(&ihdr(1, 1, 6, 8, 2)), None);
        // The largest header PNG can declare has no length a `u64` can hold:
        // an answer of `None`, not an overflow.
        assert_eq!(png_inflated_len(&ihdr(u32::MAX, u32::MAX, 6, 16, 0)), None);
        assert_eq!(png_inflated_len(&ihdr(u32::MAX, u32::MAX, 6, 16, 1)), None);
        assert!(png_inflated_len(&ihdr(u32::MAX, 1, 6, 16, 1)).is_some());
    }

    #[test]
    fn png_pixel_data_of_exactly_the_declared_length_decodes() {
        // The decoder agrees on the length the header implies: interlaced and
        // sub-byte images, where a wrong pass or row size would show, decode
        // at exactly that length.
        let driver = OxideAvImageDriver::new();
        for header in [
            ihdr(3, 3, 0, 1, 1),
            ihdr(13, 5, 3, 4, 0),
            ihdr(9, 9, 3, 2, 1),
        ] {
            let len = png_inflated_len(&header).expect("a defined format") as usize;
            let png = png_with_pixel_data(header, &vec![0u8; len]);
            assert_eq!(
                driver
                    .dimensions(&png)
                    .expect("pixel data of the declared length"),
                (header.width, header.height)
            );
        }
    }

    #[test]
    fn png_pixel_data_one_byte_past_the_declared_length_is_refused() {
        let header = ihdr(3, 3, 0, 1, 1);
        let len = png_inflated_len(&header).expect("a defined format") as usize;
        let png = png_with_pixel_data(header, &vec![0u8; len + 1]);
        let err = check_png_inflate(&png, &png_layout(&png)).expect_err("one byte too many");
        assert!(
            err.to_string().contains("inflates past the 12 bytes"),
            "got: {err}"
        );
    }

    #[test]
    fn png_pixel_data_that_is_not_zlib_is_refused_before_decoding() {
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        oxideav_png::chunk::write_chunk(&mut png, b"IHDR", &ihdr(1, 1, 6, 8, 0).to_bytes());
        oxideav_png::chunk::write_chunk(&mut png, b"IDAT", b"not a zlib stream");
        oxideav_png::chunk::write_chunk(&mut png, b"IEND", &[]);
        let err = check_png_inflate(&png, &png_layout(&png)).expect_err("not zlib");
        assert!(err.to_string().contains("does not inflate"), "got: {err}");
    }

    #[test]
    fn heic_input_gets_its_own_named_error() {
        let driver = OxideAvImageDriver::new();
        let heic = b"\x00\x00\x00\x18ftypheic\x00\x00\x00\x00mif1heic";
        let err = driver
            .process(heic, &ImagePipeline::default())
            .expect_err("HEIC must be refused");
        let message = err.to_string();
        assert!(message.contains("HEIC is not supported"), "got: {message}");
        assert!(
            message.contains("images chapter"),
            "the error must point at the rationale: {message}"
        );
    }

    #[test]
    fn unknown_format_is_a_param_error() {
        let driver = OxideAvImageDriver::new();
        let err = driver
            .process(&[0u8; 64], &ImagePipeline::default())
            .expect_err("not an image");
        assert!(err.to_string().contains("not supported"), "got: {err}");
    }

    #[test]
    fn empty_input_is_a_param_error() {
        let driver = OxideAvImageDriver::new();
        let err = driver
            .process(b"", &ImagePipeline::default())
            .expect_err("empty");
        assert!(err.to_string().contains("empty"), "got: {err}");
    }

    #[test]
    fn crop_beyond_the_source_bounds_errs() {
        let err = crop(canvas(4, 2, [255, 0, 0, 255]), 2, 0, 4, 2).expect_err("out of bounds");
        assert!(err.to_string().contains("falls outside"), "got: {err}");

        let zero = crop(canvas(4, 2, [255, 0, 0, 255]), 0, 0, 0, 2).expect_err("zero width");
        assert!(
            zero.to_string().contains("greater than zero"),
            "got: {zero}"
        );
    }

    #[test]
    fn crop_inside_the_bounds_succeeds() {
        let out = crop(canvas(4, 2, [255, 0, 0, 255]), 1, 0, 2, 2).expect("in bounds");
        assert_eq!((out.width, out.height), (2, 2));
    }

    #[test]
    fn a_plane_shorter_than_its_declared_height_errors_rather_than_panicking() {
        // A decoder handing back fewer rows than the header promised is what a
        // truncated or lying bitstream produces. Upstream's resize copies rows
        // by the declared height without a length guard, so letting this
        // through is a panic, not a wrong image.
        let short = VideoFrame {
            pts: Some(0),
            planes: vec![VideoPlane {
                stride: 4 * 4,
                // Four rows are declared below; only two are present.
                data: vec![0u8; 4 * 4 * 2],
            }],
        };
        let err = to_rgba(short, PixelFormat::Rgba, 4, 4).expect_err("short plane");
        assert!(err.to_string().contains("truncated"), "got: {err}");

        // The exact-length case is still accepted.
        let exact = VideoFrame {
            pts: Some(0),
            planes: vec![VideoPlane {
                stride: 4 * 4,
                data: vec![0u8; 4 * 4 * 4],
            }],
        };
        let canvas = to_rgba(exact, PixelFormat::Rgba, 4, 4).expect("exact plane");
        assert_eq!((canvas.width, canvas.height), (4, 4));
        assert_eq!(canvas.pixels.len(), 4 * 4 * 4);
    }

    #[test]
    fn an_over_long_plane_is_trimmed_to_the_declared_size() {
        let long = VideoFrame {
            pts: Some(0),
            planes: vec![VideoPlane {
                stride: 2 * 4,
                data: vec![7u8; 2 * 4 * 2 + 64],
            }],
        };
        let canvas = to_rgba(long, PixelFormat::Rgba, 2, 2).expect("long plane");
        assert_eq!(
            canvas.pixels.len(),
            2 * 2 * 4,
            "the canvas invariant is exact, not at-least"
        );
    }

    #[test]
    fn rotation_growth_is_predicted_and_capped() {
        // A 45-degree turn grows each side by about sqrt(2), so an image at
        // the cap lands over it. Quarter turns only swap the axes.
        assert_eq!(rotated_extent(100, 50, 90.0), (50, 100));
        assert_eq!(rotated_extent(100, 50, 180.0), (100, 50));
        assert_eq!(rotated_extent(100, 50, 270.0), (50, 100));
        assert_eq!(rotated_extent(100, 50, 0.0), (100, 50));
        let (w, h) = rotated_extent(100, 100, 45.0);
        assert!(
            (140..=142).contains(&w) && (140..=142).contains(&h),
            "45 degrees grows a square by sqrt(2), got {w}x{h}"
        );

        let config = ImageConfig {
            max_dimension: 8,
            ..ImageConfig::default()
        };
        // 8x8 is at the cap; rotating it 45 degrees would need ~12 per side.
        let err = apply(
            canvas(8, 8, [1, 2, 3, 255]),
            Transformation::Rotate(45.0),
            &config,
        )
        .expect_err("the grown canvas exceeds the cap");
        assert!(err.to_string().contains("limit"), "got: {err}");

        // A quarter turn of the same image does not grow, so it is allowed.
        apply(
            canvas(8, 8, [1, 2, 3, 255]),
            Transformation::Rotate(90.0),
            &config,
        )
        .expect("a quarter turn stays within the cap");
    }

    #[test]
    fn blur_amount_maps_across_its_whole_range() {
        assert_eq!(blur_radius(0), None, "zero is an explicit no-op");
        assert_eq!(blur_radius(1), Some(1), "the smallest visible blur");
        assert_eq!(blur_radius(100), Some(15), "the documented maximum");
        // Out-of-range input clamps rather than scaling past the maximum.
        assert_eq!(blur_radius(u32::MAX), Some(15));
        // Monotonic across the range, never zero.
        let mut previous = 0;
        for amount in 1..=100 {
            let radius = blur_radius(amount).expect("non-zero amount blurs");
            assert!(radius >= previous && radius >= 1, "amount {amount}");
            previous = radius;
        }
    }

    #[test]
    fn sharpen_amount_maps_across_its_whole_range() {
        assert_eq!(sharpen_strength(0), None, "zero is an explicit no-op");
        assert_eq!(
            sharpen_strength(50),
            Some(1.0),
            "the classic unsharp amount sits mid-scale"
        );
        assert_eq!(sharpen_strength(100), Some(2.0));
        assert_eq!(sharpen_strength(u32::MAX), Some(2.0), "clamped, not scaled");
    }

    #[test]
    fn native_sharpen_filter_runs_and_preserves_geometry() {
        // The upstream crate ships `Sharpen`, so there is no hand-rolled
        // unsharp fallback to test - this pins that the native path is the
        // one wired up and that it keeps the canvas shape.
        let out = apply(
            canvas(4, 2, [10, 120, 250, 255]),
            Transformation::Sharpen(50),
            &ImageConfig::default(),
        )
        .expect("sharpen");
        assert_eq!((out.width, out.height), (4, 2));
    }

    #[test]
    fn quality_is_only_handed_to_the_encoder_that_accepts_it() {
        // The PNG encoder rejects a `quality` option outright, so a pipeline
        // carrying one must still encode. This is the regression guard for
        // that upstream wart.
        let driver = OxideAvImageDriver::new();
        let pipeline = ImagePipeline {
            format: Some(OutputFormat::Png),
            quality: 55,
            ..ImagePipeline::default()
        };
        let out = driver.process(RED_PNG_1X1, &pipeline).expect("png encodes");
        assert!(out.starts_with(b"\x89PNG"), "expected a PNG file");
    }

    #[test]
    fn every_output_format_encodes() {
        let driver = OxideAvImageDriver::new();
        for (format, magic) in [
            (OutputFormat::Png, &b"\x89PNG"[..]),
            (OutputFormat::Jpeg, &[0xFF, 0xD8, 0xFF][..]),
            (OutputFormat::WebP, &b"RIFF"[..]),
            (OutputFormat::WebPLossless, &b"RIFF"[..]),
            (OutputFormat::Gif, &b"GIF"[..]),
            (OutputFormat::Bmp, &b"BM"[..]),
        ] {
            let pipeline = ImagePipeline {
                transformations: vec![Transformation::Resize {
                    width: 4,
                    height: 2,
                }],
                format: Some(format),
                ..ImagePipeline::default()
            };
            let out = driver
                .process(RED_PNG_1X1, &pipeline)
                .unwrap_or_else(|e| panic!("{format:?} must encode: {e}"));
            assert!(
                out.starts_with(magic),
                "{format:?} produced the wrong magic bytes: {:02x?}",
                &out[..out.len().min(8)]
            );
        }
    }

    #[test]
    fn gif_encodes_an_image_with_more_than_256_colours() {
        // The palette two-step dithers a photographic source down to 256
        // colours before the encoder sees it.
        let mut pixels = Vec::new();
        for i in 0..300u32 {
            pixels.extend_from_slice(&[
                (i % 256) as u8,
                ((i / 2) % 256) as u8,
                ((i / 3) % 256) as u8,
                255,
            ]);
        }
        let source = Canvas {
            width: 300,
            height: 1,
            pixels,
        };
        let driver = OxideAvImageDriver::new();
        let out = driver
            .encode(source, OutputFormat::Gif, 70)
            .expect("quantised gif");
        assert!(out.starts_with(b"GIF"), "expected a GIF file");
    }

    #[test]
    fn a_gif_round_trips_its_colours() {
        // Under 256 colours, GIF keeps every one exactly.
        let source = Canvas {
            width: 3,
            height: 1,
            pixels: vec![200, 10, 10, 255, 10, 10, 200, 255, 30, 200, 30, 255],
        };
        let driver = OxideAvImageDriver::new();
        let gif = driver
            .encode(source, OutputFormat::Gif, 70)
            .expect("an encodable canvas");
        let decoded = driver
            .load(&gif, &ImageConfig::default())
            .expect("our own GIF decodes");
        assert_eq!((decoded.width, decoded.height), (3, 1));
        assert_eq!(
            decoded.pixels,
            [200, 10, 10, 255, 10, 10, 200, 255, 30, 200, 30, 255]
        );
    }

    #[test]
    fn opacity_is_read_from_every_alpha_byte() {
        assert!(canvas(3, 2, [10, 20, 30, 255]).is_opaque());

        let mut one_translucent = canvas(3, 2, [10, 20, 30, 255]);
        one_translucent.pixels[4 * 5 + 3] = 254;
        assert!(
            !one_translucent.is_opaque(),
            "the last pixel alone must be enough to count"
        );
    }

    #[test]
    fn edge_extension_repeats_the_last_column_and_row() {
        let source = Canvas {
            width: 3,
            height: 1,
            pixels: vec![1, 1, 1, 255, 2, 2, 2, 255, 3, 3, 3, 255],
        };
        let frame = edge_extended_frame(&source, 4, 2).expect("extend");
        let row: [u8; 16] = [1, 1, 1, 255, 2, 2, 2, 255, 3, 3, 3, 255, 3, 3, 3, 255];
        assert_eq!(frame.planes[0].stride, 16);
        assert_eq!(frame.planes[0].data, [row, row].concat());
    }

    #[test]
    fn an_odd_sized_opaque_canvas_encodes_as_lossy_webp() {
        // The 4:2:0 converter only takes even sizes; the edge extension is
        // what lets a 5x3 image reach the lossy encoder at all.
        let driver = OxideAvImageDriver::new();
        let out = driver
            .encode(canvas(5, 3, [200, 120, 40, 255]), OutputFormat::WebP, 70)
            .expect("lossy webp");
        assert!(out.starts_with(b"RIFF"), "expected a WebP file");
        assert_eq!(&out[12..16], b"VP8 ", "an opaque canvas must encode lossy");
        assert_eq!(driver.dimensions(&out).expect("decodes"), (5, 3));
    }

    #[test]
    fn webp_falls_back_to_lossless_past_the_largest_vp8_frame() {
        // The decision is tested on the numbers alone, so no canvas of this
        // size is ever allocated.
        assert!(
            webp_is_lossy(16_383, 16_383, true),
            "the largest VP8 frame must stay lossy"
        );
        assert!(
            !webp_is_lossy(16_384, 1, true),
            "one column wider than a VP8 frame must be lossless"
        );
        assert!(
            !webp_is_lossy(1, 16_384, true),
            "one row taller than a VP8 frame must be lossless"
        );
        assert!(
            !webp_is_lossy(1, 1, false),
            "transparency must be lossless at any size"
        );
    }

    #[test]
    fn a_canvas_with_transparency_keeps_webp_lossless() {
        let driver = OxideAvImageDriver::new();
        let mut source = canvas(4, 2, [10, 20, 30, 255]);
        source.pixels[3] = 0;
        let out = driver
            .encode(source, OutputFormat::WebP, 70)
            .expect("lossless webp");
        // VP8L with alpha is written in the extended layout: a VP8X header
        // chunk first, then the VP8L bitstream.
        assert_eq!(&out[12..16], b"VP8X");
        assert!(out.windows(4).any(|chunk| chunk == b"VP8L"));
        assert!(!out.windows(4).any(|chunk| chunk == b"VP8 "));

        let lossless = driver
            .encode(
                canvas(4, 2, [10, 20, 30, 255]),
                OutputFormat::WebPLossless,
                70,
            )
            .expect("lossless webp");
        assert_eq!(
            &lossless[12..16],
            b"VP8L",
            "WebPLossless stays lossless on an opaque canvas"
        );
    }

    #[test]
    fn average_color_rounds_a_uniform_image_to_its_own_colour() {
        assert_eq!(average_color(&canvas(4, 2, [255, 0, 0, 255])), "#ff0000");
        assert_eq!(average_color(&canvas(2, 2, [18, 52, 86, 255])), "#123456");
        // Alpha is dropped, not blended into the result.
        assert_eq!(average_color(&canvas(2, 2, [255, 0, 0, 0])), "#ff0000");
    }

    #[test]
    fn average_color_of_an_empty_canvas_is_black_not_a_panic() {
        let empty = Canvas {
            width: 0,
            height: 0,
            pixels: Vec::new(),
        };
        assert_eq!(average_color(&empty), "#000000");
    }

    #[test]
    fn scale_never_enlarges_but_contain_may() {
        let config = ImageConfig::default();
        let scaled_up = apply(
            canvas(4, 2, [1, 2, 3, 255]),
            Transformation::Scale {
                width: 100,
                height: 100,
            },
            &config,
        )
        .expect("scale");
        assert_eq!((scaled_up.width, scaled_up.height), (4, 2));

        let contained = apply(
            canvas(4, 2, [1, 2, 3, 255]),
            Transformation::Contain {
                width: 100,
                height: 100,
            },
            &config,
        )
        .expect("contain");
        assert_eq!(
            (contained.width, contained.height),
            (100, 50),
            "contain fits the box and may enlarge"
        );
    }

    #[test]
    fn cover_fills_the_box_exactly() {
        let out = apply(
            canvas(8, 2, [1, 2, 3, 255]),
            Transformation::Cover {
                width: 4,
                height: 4,
            },
            &ImageConfig::default(),
        )
        .expect("cover");
        assert_eq!((out.width, out.height), (4, 4));
    }

    #[test]
    fn resize_targets_are_capped_by_the_configured_limits() {
        let config = ImageConfig {
            max_dimension: 16,
            ..ImageConfig::default()
        };
        let err = apply(
            canvas(4, 2, [1, 2, 3, 255]),
            Transformation::Resize {
                width: 4_000,
                height: 4_000,
            },
            &config,
        )
        .expect_err("oversized target");
        assert!(err.to_string().contains("limit"), "got: {err}");
    }

    /// MEM-003: a filter step hands the canvas's plane to the filter and
    /// adopts the filter's output, copying neither.
    #[test]
    fn mem_audit_a_filter_step_moves_planes() {
        use std::sync::Mutex;
        struct Probe {
            input: Mutex<usize>,
            output: Mutex<usize>,
        }
        impl ImageFilter for Probe {
            fn apply(
                &self,
                input: &VideoFrame,
                _params: VideoStreamParams,
            ) -> Result<VideoFrame, oxideav_core::Error> {
                *self.input.lock().unwrap() = input.planes[0].data.as_ptr() as usize;
                let data = input.planes[0].data.clone();
                *self.output.lock().unwrap() = data.as_ptr() as usize;
                Ok(VideoFrame {
                    pts: Some(0),
                    planes: vec![VideoPlane {
                        stride: input.planes[0].stride,
                        data,
                    }],
                })
            }
        }
        let source = canvas(64, 64, [1, 2, 3, 255]);
        let source_ptr = source.pixels.as_ptr() as usize;
        let probe = Probe {
            input: Mutex::new(0),
            output: Mutex::new(0),
        };
        let result = filter(source, &probe).unwrap();
        assert_eq!(
            *probe.input.lock().unwrap(),
            source_ptr,
            "the filter saw a copy"
        );
        assert_eq!(
            result.pixels.as_ptr() as usize,
            *probe.output.lock().unwrap(),
            "the canvas copied the filter's output"
        );
    }
}
