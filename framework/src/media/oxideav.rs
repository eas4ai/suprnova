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
//! `oxideav-io` is therefore not a dependency at all: with decode and encode
//! both on the registry, nothing was left for it to do.
//!
//! ## The sandbox, one layer up
//!
//! `oxideav-io`'s `OpenOptions::allow_codecs` is the knob its docs recommend
//! for untrusted input. Driving the registry directly gives up that knob and
//! replaces it with a stronger property: this driver only ever *asks* for one
//! of five codec ids, and which one is decided by
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
//! - **WebP and GIF** decoders declare RGBA as their only output format, so
//!   the layout is known; the driver still checks the stride and errors
//!   rather than trusting it blindly.
//! - **BMP and JPEG** have small, capability-bounded output sets, so the
//!   remaining classification is exact rather than a guess.
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
use super::sniff::{self, InputFormat};

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

    fn frame(&self) -> VideoFrame {
        VideoFrame {
            pts: Some(0),
            planes: vec![VideoPlane {
                stride: self.width as usize * 4,
                data: self.pixels.clone(),
            }],
        }
    }

    /// Rebuild from a filter's output frame.
    ///
    /// The filters that change shape (resize, crop, rotate) report the new
    /// geometry only through the frame itself. Because the layout is always
    /// RGBA, `stride / 4` and `len / stride` recover it exactly - which is
    /// what lets rotate grow the canvas without the caller predicting by how
    /// much.
    fn from_frame(frame: &VideoFrame) -> Result<Self, FrameworkError> {
        let plane = frame
            .planes
            .first()
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
            pixels: plane.data.clone(),
        })
    }
}

/// The pure-Rust image driver: OxideAV codecs, OxideAV filters, no native
/// libraries and nothing to install.
///
/// Holds one `RuntimeContext` with the five still-image codecs registered.
/// Building it is cheap but not free, and it is immutable once built, so the
/// driver is constructed once and shared.
pub struct OxideAvImageDriver {
    context: RuntimeContext,
}

impl OxideAvImageDriver {
    /// Register the five supported codecs into a fresh runtime context.
    ///
    /// Note `oxideav_bmp::register` takes the two sub-registries separately
    /// rather than the `RuntimeContext` its siblings take - an upstream
    /// inconsistency, not a mistake here.
    pub fn new() -> Self {
        let mut context = RuntimeContext::new();
        oxideav_png::register(&mut context);
        oxideav_mjpeg::register(&mut context);
        oxideav_webp::register(&mut context);
        oxideav_gif::register(&mut context);
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
        self.decode(contents, format, width, height)
    }

    fn decode(
        &self,
        contents: &[u8],
        format: InputFormat,
        width: u32,
        height: u32,
    ) -> Result<Canvas, FrameworkError> {
        if format == InputFormat::Png {
            // The crate's own all-colour-types entry point. See module docs
            // for why PNG does not go through the registry.
            let bitmap = oxideav_png::decode_png_to_rgba(contents)
                .map_err(|e| FrameworkError::param(format!("image decode failed: png: {e}")))?;
            return Canvas::packed(bitmap.width, bitmap.height, bitmap.data);
        }

        let frame = self.decode_via_registry(contents, format)?;
        let source = source_pixel_format(&frame, format, width, height)?;
        to_rgba(&frame, source, width, height)
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

    fn encode(
        &self,
        canvas: &Canvas,
        format: OutputFormat,
        quality: u8,
    ) -> Result<Vec<u8>, FrameworkError> {
        let (codec, frame, pixel_format) = match format {
            // The MJPEG encoder rejects RGBA outright, so the conversion is
            // mandatory rather than an optimisation.
            OutputFormat::Jpeg => (
                "mjpeg",
                convert_frame(&canvas.frame(), canvas, PixelFormat::Rgb24)?,
                PixelFormat::Rgb24,
            ),
            OutputFormat::Png => ("png", canvas.frame(), PixelFormat::Rgba),
            // Only the VP8L (lossless) encoder is registered; codec id
            // "webp" has a decoder but no encoder.
            OutputFormat::WebP => ("webp_vp8l", canvas.frame(), PixelFormat::Rgba),
            OutputFormat::Gif => ("gif", quantise_for_gif(canvas)?, PixelFormat::Rgba),
            OutputFormat::Bmp => ("bmp", canvas.frame(), PixelFormat::Rgba),
        };

        let mut params = CodecParameters::video(CodecId::new(codec));
        params.width = Some(canvas.width);
        params.height = Some(canvas.height);
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
        encoder
            .send_frame(&Frame::Video(frame))
            .map_err(|e| FrameworkError::internal(format!("image encode failed: {codec}: {e}")))?;
        encoder
            .flush()
            .map_err(|e| FrameworkError::internal(format!("image encode failed: {codec}: {e}")))?;
        drain(encoder.as_mut(), codec)
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
        self.encode(&canvas, target, pipeline.quality)
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

/// Determine the layout a decoder handed back.
///
/// Exact for WebP and GIF (one declared output format each) and bounded for
/// BMP and JPEG by their declared capabilities. PNG never reaches here.
fn source_pixel_format(
    frame: &VideoFrame,
    format: InputFormat,
    width: u32,
    height: u32,
) -> Result<PixelFormat, FrameworkError> {
    let plane = frame
        .planes
        .first()
        .ok_or_else(|| FrameworkError::param("image decode produced no planes"))?;
    let width_px = width as usize;
    let bytes_per_pixel = if width_px == 0 {
        0
    } else {
        plane.stride / width_px
    };

    let unsupported = |detail: &str| {
        FrameworkError::param(format!(
            "image decode produced an unsupported pixel layout ({detail}); convert the source \
             to 8-bit RGB or RGBA and retry"
        ))
    };

    match format {
        InputFormat::Png => Ok(PixelFormat::Rgba),
        InputFormat::WebP | InputFormat::Gif => {
            if frame.planes.len() == 1 && bytes_per_pixel == 4 {
                Ok(PixelFormat::Rgba)
            } else {
                Err(unsupported("expected packed RGBA"))
            }
        }
        InputFormat::Bmp => match (frame.planes.len(), bytes_per_pixel) {
            (1, 4) => Ok(PixelFormat::Rgba),
            (1, 3) => Ok(PixelFormat::Rgb24),
            _ => Err(unsupported("expected packed RGB or RGBA")),
        },
        InputFormat::Jpeg => match frame.planes.len() {
            1 => match bytes_per_pixel {
                1 => Ok(PixelFormat::Gray8),
                3 => Ok(PixelFormat::Rgb24),
                other => Err(unsupported(&format!("{other} bytes per pixel"))),
            },
            3 => yuv_layout(frame, width, height)
                .ok_or_else(|| unsupported("planar chroma geometry matches no known subsampling")),
            other => Err(unsupported(&format!("{other} planes"))),
        },
    }
}

/// Classify a three-plane frame by comparing the chroma planes' geometry to
/// the luma dimensions. Unambiguous once we already know the frame is planar.
fn yuv_layout(frame: &VideoFrame, width: u32, height: u32) -> Option<PixelFormat> {
    let chroma = frame.planes.get(1)?;
    if chroma.stride == 0 {
        return None;
    }
    let chroma_width = chroma.stride;
    let chroma_height = chroma.data.len() / chroma.stride;
    let half_width = (width as usize).div_ceil(2);
    let half_height = (height as usize).div_ceil(2);
    let full_height = height as usize;

    if chroma_width == half_width && chroma_height == half_height {
        Some(PixelFormat::Yuv420P)
    } else if chroma_width == half_width && chroma_height == full_height {
        Some(PixelFormat::Yuv422P)
    } else if chroma_width == width as usize && chroma_height == full_height {
        Some(PixelFormat::Yuv444P)
    } else {
        None
    }
}

/// Convert any decoded layout to packed RGBA with a tight stride.
fn to_rgba(
    frame: &VideoFrame,
    source: PixelFormat,
    width: u32,
    height: u32,
) -> Result<Canvas, FrameworkError> {
    let tight = width as usize * 4;
    if source == PixelFormat::Rgba {
        let plane = frame
            .planes
            .first()
            .ok_or_else(|| FrameworkError::param("image decode produced no planes"))?;
        if plane.stride == tight {
            // `Canvas::packed` is what rejects a plane shorter than the
            // declared height rather than handing it to a filter that would
            // index past the end of it.
            return Canvas::packed(width, height, plane.data.clone());
        }
    }
    let info = FrameInfo::new(source, width, height);
    let converted = pix_convert(frame, info, PixelFormat::Rgba, &ConvertOptions::default())
        .map_err(|e| FrameworkError::param(format!("image pixel conversion failed: {e}")))?;
    let pixels = pack_tight(&converted, tight, height as usize)?;
    Canvas::packed(width, height, pixels)
}

/// Strip any per-row padding a conversion left behind.
fn pack_tight(frame: &VideoFrame, tight: usize, height: usize) -> Result<Vec<u8>, FrameworkError> {
    let plane = frame
        .planes
        .first()
        .ok_or_else(|| FrameworkError::internal("pixel conversion produced no plane"))?;
    let short = || FrameworkError::internal("pixel conversion produced a short plane");
    if plane.stride == tight {
        return plane
            .data
            .get(..tight * height)
            .map(<[u8]>::to_vec)
            .ok_or_else(short);
    }
    let mut out = Vec::with_capacity(tight * height);
    for row in 0..height {
        let start = row
            .checked_mul(plane.stride)
            .ok_or_else(|| FrameworkError::internal("pixel conversion row offset overflow"))?;
        out.extend_from_slice(plane.data.get(start..start + tight).ok_or_else(short)?);
    }
    Ok(out)
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
        .apply(&canvas.frame(), params)
        .map_err(|e| FrameworkError::param(format!("image transformation failed: {e}")))?;
    Canvas::from_frame(&out)
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
    canvas: &Canvas,
    target: PixelFormat,
) -> Result<VideoFrame, FrameworkError> {
    let info = FrameInfo::new(PixelFormat::Rgba, canvas.width, canvas.height);
    pix_convert(frame, info, target, &ConvertOptions::default())
        .map_err(|e| FrameworkError::internal(format!("image pixel conversion failed: {e}")))
}

/// Reduce a full-colour frame to at most 256 colours so the GIF encoder can
/// take it.
///
/// The GIF encoder builds its own palette but refuses input with more than
/// 256 distinct colours rather than quantising, and `oxideav-pixfmt` will not
/// convert to `Pal8` without a caller-supplied palette. So the palette is
/// generated explicitly, the frame is mapped through it with Floyd-Steinberg
/// dithering, and mapped straight back to RGBA - which now holds at most 256
/// distinct colours and encodes cleanly.
fn quantise_for_gif(canvas: &Canvas) -> Result<VideoFrame, FrameworkError> {
    let frame = canvas.frame();
    let info = FrameInfo::new(PixelFormat::Rgba, canvas.width, canvas.height);
    let palette = generate_palette(&[(&frame, info)], &PaletteGenOptions::default())
        .map_err(|e| FrameworkError::internal(format!("gif palette generation failed: {e}")))?;

    let to_indexed = ConvertOptions {
        dither: Dither::FloydSteinberg,
        palette: Some(palette.clone()),
        ..Default::default()
    };
    let indexed = pix_convert(&frame, info, PixelFormat::Pal8, &to_indexed)
        .map_err(|e| FrameworkError::internal(format!("gif quantisation failed: {e}")))?;

    let indexed_info = FrameInfo::new(PixelFormat::Pal8, canvas.width, canvas.height);
    let from_indexed = ConvertOptions {
        palette: Some(palette),
        ..Default::default()
    };
    let reduced = pix_convert(&indexed, indexed_info, PixelFormat::Rgba, &from_indexed)
        .map_err(|e| FrameworkError::internal(format!("gif quantisation failed: {e}")))?;

    let tight = canvas.width as usize * 4;
    let pixels = pack_tight(&reduced, tight, canvas.height as usize)?;
    Ok(VideoFrame {
        pts: Some(0),
        planes: vec![VideoPlane {
            stride: tight,
            data: pixels,
        }],
    })
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
    for pixel in canvas.pixels.chunks_exact(4) {
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
        let err = to_rgba(&short, PixelFormat::Rgba, 4, 4).expect_err("short plane");
        assert!(err.to_string().contains("truncated"), "got: {err}");

        // The exact-length case is still accepted.
        let exact = VideoFrame {
            pts: Some(0),
            planes: vec![VideoPlane {
                stride: 4 * 4,
                data: vec![0u8; 4 * 4 * 4],
            }],
        };
        let canvas = to_rgba(&exact, PixelFormat::Rgba, 4, 4).expect("exact plane");
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
        let canvas = to_rgba(&long, PixelFormat::Rgba, 2, 2).expect("long plane");
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
        // Straight to the GIF encoder this would fail; the palette two-step
        // is what makes a photographic source encodable.
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
            .encode(&source, OutputFormat::Gif, 70)
            .expect("quantised gif");
        assert!(out.starts_with(b"GIF"), "expected a GIF file");
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
}
