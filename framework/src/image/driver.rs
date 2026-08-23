//! The image driver boundary.
//!
//! Everything in this module is backend-agnostic on purpose: `&[u8]` goes in,
//! `Vec<u8>` comes out, and no codec type ever crosses the line. That is the
//! whole justification for having a trait here rather than calling a codec
//! directly - this subsystem already swapped its backend once during design
//! (from the `image` crate to OxideAV), and the next swap, or an app that
//! needs a format the framework deliberately does not ship, costs one `impl`
//! rather than a rewrite.
//!
//! [`Transformation`] deliberately mirrors Laravel's transformation objects
//! rather than any backend's filter names, so a custom driver reads the same
//! instruction set the manual documents.

use crate::error::FrameworkError;

/// Default encode quality, matching Laravel's `Image::quality()` default.
///
/// Only the lossy encoders read it. See [`ImagePipeline::quality`] for which
/// formats honour the knob and which ignore it.
pub const DEFAULT_IMAGE_QUALITY: u8 = 70;

/// The container an [`Image`](super::Image) pipeline encodes to.
///
/// Deliberately five variants, not six: AVIF is absent rather than present
/// and always failing, because a variant that never works is a partial
/// scaffold. It becomes an additive change the day the in-house AV1 encoder
/// publishes - see the images chapter of the manual.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OutputFormat {
    /// JPEG. Lossy; honours [`ImagePipeline::quality`].
    Jpeg,
    /// PNG. Lossless; ignores quality, as in Laravel's encoder table.
    Png,
    /// WebP. Encoded losslessly (VP8L), so quality is a no-op today.
    WebP,
    /// GIF. Palette-quantised to at most 256 colours before encoding.
    Gif,
    /// Windows bitmap. Lossless; ignores quality.
    Bmp,
}

impl OutputFormat {
    /// The `Content-Type` this format is served under.
    ///
    /// Used by [`Image::to_response`](super::Image::to_response) and
    /// [`Image::mime_type`](super::Image::mime_type), so the value a handler
    /// sends and the value a caller reads back can never drift apart.
    pub fn mime_type(self) -> &'static str {
        match self {
            Self::Jpeg => "image/jpeg",
            Self::Png => "image/png",
            Self::WebP => "image/webp",
            Self::Gif => "image/gif",
            Self::Bmp => "image/bmp",
        }
    }

    /// The conventional file extension, without a leading dot.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Png => "png",
            Self::WebP => "webp",
            Self::Gif => "gif",
            Self::Bmp => "bmp",
        }
    }
}

/// One step of an image pipeline.
///
/// Recorded, not executed: an [`Image`](super::Image) accumulates these and
/// the driver replays them at terminal time. Keeping them as plain data is
/// what lets the pipeline stay lazy and stay cloneable.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Transformation {
    /// Force exact dimensions, ignoring the source aspect ratio.
    Resize {
        /// Target width in pixels.
        width: u32,
        /// Target height in pixels.
        height: u32,
    },
    /// Resize to a width, deriving the height from the source aspect ratio.
    ResizeWidth(u32),
    /// Resize to a height, deriving the width from the source aspect ratio.
    ResizeHeight(u32),
    /// Fit inside a box, preserving aspect ratio and never enlarging.
    Scale {
        /// Bounding width in pixels.
        width: u32,
        /// Bounding height in pixels.
        height: u32,
    },
    /// Scale to at most a width, preserving aspect ratio, never enlarging.
    ScaleWidth(u32),
    /// Scale to at most a height, preserving aspect ratio, never enlarging.
    ScaleHeight(u32),
    /// Cut a rectangle out of the source.
    Crop {
        /// Rectangle width in pixels.
        width: u32,
        /// Rectangle height in pixels.
        height: u32,
        /// Left edge, in pixels from the source's left.
        x: u32,
        /// Top edge, in pixels from the source's top.
        y: u32,
    },
    /// Fill the target box, cropping the overflow from the centre.
    Cover {
        /// Target width in pixels.
        width: u32,
        /// Target height in pixels.
        height: u32,
    },
    /// Fit inside the target box, preserving aspect ratio. No padding.
    Contain {
        /// Bounding width in pixels.
        width: u32,
        /// Bounding height in pixels.
        height: u32,
    },
    /// Rotate clockwise by an arbitrary angle, growing the canvas to fit.
    Rotate(f32),
    /// Mirror top-to-bottom (Laravel's `flip`).
    FlipVertically,
    /// Mirror left-to-right (Laravel's `flop`).
    FlipHorizontally,
    /// Gaussian blur, strength `0..=100`.
    Blur(u32),
    /// Unsharp-mask sharpen, strength `0..=100`.
    Sharpen(u32),
    /// Desaturate to grey while staying in a colour layout.
    Grayscale,
}

/// A complete recorded pipeline: what to do, what to encode to, how hard.
#[derive(Debug, Clone, PartialEq)]
pub struct ImagePipeline {
    /// Steps to replay, in the order the caller chained them.
    pub transformations: Vec<Transformation>,
    /// Target format. `None` re-encodes to the format the source was in,
    /// matching Laravel's "keep the format unless asked" behaviour.
    pub format: Option<OutputFormat>,
    /// Encode quality, always in `1..=100`.
    ///
    /// Honoured by JPEG. Ignored by PNG, GIF, and BMP - the same encoder
    /// table Laravel documents - and currently a no-op for WebP, which the
    /// built-in driver encodes losslessly.
    pub quality: u8,
}

impl Default for ImagePipeline {
    fn default() -> Self {
        Self {
            transformations: Vec::new(),
            format: None,
            quality: DEFAULT_IMAGE_QUALITY,
        }
    }
}

/// A backend that can decode, transform, and re-encode image bytes.
///
/// # Contract
///
/// A conforming driver **enforces the configured
/// [`ImageConfig`](super::ImageConfig) limits before allocating for a
/// decode** - it reads [`image::config()`](super::config) (or its own
/// equivalent) and refuses input whose declared dimensions exceed
/// [`max_dimension`](super::ImageConfig::max_dimension) or whose decoded size
/// would exceed [`max_alloc_bytes`](super::ImageConfig::max_alloc_bytes).
/// The framework cannot enforce this on a driver's behalf, because the
/// framework never sees the decoded buffer; a driver that skips the check
/// hands an attacker a decompression bomb. Both built-in drivers parse the
/// input's header and check before a single pixel is allocated.
///
/// Implementations must not panic on hostile input - return an error. The
/// terminal wrapper turns a panicking driver into
/// [`FrameworkError::internal`], but that is a net for genuine bugs, not a
/// substitute for validation.
pub trait ImageDriver: Send + Sync + 'static {
    /// Decode `contents`, replay `pipeline`, and encode the result.
    ///
    /// Returns the complete encoded file, ready to write or serve.
    fn process(&self, contents: &[u8], pipeline: &ImagePipeline)
    -> Result<Vec<u8>, FrameworkError>;

    /// Report the pixel dimensions of the image in `contents`.
    ///
    /// Callers hand this the *processed* bytes, so the answer reflects the
    /// finished image the way Laravel's `dimensions()` does.
    fn dimensions(&self, contents: &[u8]) -> Result<(u32, u32), FrameworkError>;

    /// Report the average colour of the image in `contents` as `#rrggbb`.
    ///
    /// Alpha is dropped, matching Laravel's `dominantColor()`.
    fn dominant_color(&self, contents: &[u8]) -> Result<String, FrameworkError>;

    /// Short driver name, for diagnostics and `IMAGE_DRIVER` round-tripping.
    fn name(&self) -> &'static str;
}
