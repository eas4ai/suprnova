//! Laravel-shaped image processing.
//!
//! ```rust,no_run
//! use suprnova::media::Image;
//! use suprnova::OutputFormat;
//! # async fn ex() -> Result<(), suprnova::FrameworkError> {
//! let thumbnail = Image::from_path("photo.jpg")
//!     .cover(320, 320)
//!     .to_format(OutputFormat::WebP)
//!     .to_bytes()
//!     .await?;
//! # let _ = thumbnail;
//! # Ok(())
//! # }
//! ```
//!
//! # The pipeline is lazy
//!
//! Constructing an [`Image`] reads nothing and decodes nothing. Operations
//! record themselves, and the source is only opened when a terminal runs -
//! the same design as Laravel's `ImageManager`, where the contents are a
//! closure evaluated at processing time. That is what makes it safe to build
//! an `Image` in a route, pass it around, clone it, and pay for the pixels
//! exactly once, at the end, on a blocking thread.
//!
//! Two constructors have to be eager, and say so:
//! [`Image::from_upload`] (an upload's temp file does not outlive the
//! request) and [`Image::from_stream`] (a stream can only be consumed once).
//!
//! # Two drivers, like Laravel
//!
//! Laravel picks between GD and Imagick with `IMAGE_DRIVER`. Suprnova does
//! the same with a different pair:
//!
//! - `oxideav` (default) - the pure-Rust [`OxideAvImageDriver`]. Nothing to
//!   install, no native library, no patent exposure. Reads and writes PNG,
//!   JPEG, WebP, GIF, and BMP.
//! - `magick` - the opt-in [`MagickCliDriver`], which runs a host-installed
//!   ImageMagick 7 binary. Wider input support (whatever the host's
//!   delegates provide, including HEIC), at the cost of a host dependency.
//!
//! Anything else is an [`ImageDriver`] implementation and
//! [`set_default_driver`].
//!
//! # Limits
//!
//! Decoding is where hostile input does damage, so the framework caps it:
//! `IMAGE_MAX_DIMENSION` and `IMAGE_MAX_ALLOC_BYTES` are checked against the
//! input's own declared header dimensions *before* anything allocates. See
//! [`ImageConfig`] and the `sniff` module for why the framework does this
//! itself rather than delegating to a codec.
//!
//! Decoders allocate more than the RGBA they return, so the default driver
//! also estimates, from the headers, how many bytes the whole decode will
//! allocate, and refuses the image when that is over `IMAGE_MAX_ALLOC_BYTES`.
//! It bounds what the headers cannot show as well: PNG pixel data that
//! inflates past what its header declares is refused at that point, and only
//! the first frame of a GIF is decoded. Path and disk sources are read no
//! further than the same cap.
//!
//! # Orientation and metadata
//!
//! Both drivers apply the source's EXIF orientation as they decode, as
//! Laravel's do, unless `IMAGE_AUTO_ORIENT=false`. Processed output carries
//! the source's ICC profile and nothing else of its metadata: no EXIF (so no
//! GPS position), XMP, IPTC, comments or text chunks. See the `metadata`
//! module.

mod color;
mod custom;
mod driver;
mod magick;
mod metadata;
mod orientation;
mod oxideav;
mod sniff;

use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};

use bytes::Bytes;

use crate::config::env_optional;
use crate::error::FrameworkError;
use crate::http::HttpResponse;

pub use color::Color;
pub use custom::{CustomTransformation, ImagePixels, register_transformation};
pub use driver::{DEFAULT_IMAGE_QUALITY, ImageDriver, ImagePipeline, OutputFormat, Transformation};
pub use magick::MagickCliDriver;
pub use oxideav::OxideAvImageDriver;

/// Default cap on either pixel dimension of a decoded image.
///
/// 16384 is comfortably past any camera or scanner output while still
/// bounding the decoded buffer to something a server can survive.
pub const DEFAULT_IMAGE_MAX_DIMENSION: u32 = 16_384;

/// Default cap on the bytes decoding a single image may allocate.
///
/// 1 GiB admits a 48-megapixel photo (8000x6000) in every 8-bit format
/// the built-in driver reads, a progressive 4:4:4 JPEG and a PNG of
/// incompressible RGBA included: decoders hold several times the RGBA they
/// return. 16-bit PNG holds more and tops out lower, and the built-in JPEG
/// decoder has a smaller frame limit of its own; the images chapter lists
/// both.
pub const DEFAULT_IMAGE_MAX_ALLOC_BYTES: u64 = 1024 * 1024 * 1024;

/// Default wall-clock ceiling on one ImageMagick invocation, in seconds.
///
/// Conservative on purpose: a legitimate resize of a web-sized image finishes
/// in well under a second, so 30 leaves enormous headroom while still bounding
/// a delegate that has gone away.
pub const DEFAULT_IMAGE_MAGICK_TIMEOUT_SECS: u32 = 30;

/// Which built-in driver `IMAGE_DRIVER` selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImageDriverKind {
    /// Pure Rust, no host dependency. The default.
    #[default]
    OxideAv,
    /// Shells out to a host-installed ImageMagick 7 binary.
    Magick,
}

impl ImageDriverKind {
    /// Parse an `IMAGE_DRIVER` value. Case-insensitive, whitespace-trimmed.
    ///
    /// An unknown name is an error rather than a silent fallback: quietly
    /// running the pure-Rust driver when an operator asked for ImageMagick
    /// would turn a typo into "why does HEIC upload fail in production".
    pub fn parse(value: &str) -> Result<Self, FrameworkError> {
        match value.trim().to_ascii_lowercase().as_str() {
            "oxideav" => Ok(Self::OxideAv),
            "magick" | "imagemagick" => Ok(Self::Magick),
            other => Err(FrameworkError::internal(format!(
                "IMAGE_DRIVER: unknown driver `{other}` (expected `oxideav` or `magick`)"
            ))),
        }
    }

    /// Read `IMAGE_DRIVER`, defaulting to [`ImageDriverKind::OxideAv`].
    pub fn from_env() -> Result<Self, FrameworkError> {
        match env_optional::<String>("IMAGE_DRIVER") {
            Some(raw) => Self::parse(&raw),
            None => Ok(Self::default()),
        }
    }
}

/// Decode limits and decode behaviour for the image subsystem.
///
/// # Environment variables
///
/// - `IMAGE_MAX_DIMENSION` - cap on width and height in pixels
///   (default 16384).
/// - `IMAGE_MAX_ALLOC_BYTES` - cap on the bytes one decode may allocate
///   (default 1 GiB).
/// - `IMAGE_MAGICK_TIMEOUT_SECS` - wall-clock cap on one ImageMagick run
///   (default 30).
/// - `IMAGE_AUTO_ORIENT` - apply the EXIF orientation on decode
///   (default `true`).
///
/// Out-of-range values clamp with a warning rather than failing boot: a
/// misconfigured limit should be loud, not fatal, and a limit of zero would
/// reject every image in the application.
///
/// `#[non_exhaustive]` so a field can be added without breaking callers:
/// start from [`ImageConfig::default`] or [`ImageConfig::from_env`] and set
/// the fields to change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ImageConfig {
    /// Maximum width or height, in pixels, of a decoded image.
    pub max_dimension: u32,
    /// Most bytes decoding one image may allocate.
    ///
    /// Every driver checks the decoded RGBA footprint (four bytes a pixel)
    /// against it before decoding, and so does every resize target. The
    /// built-in driver also estimates the whole decode per format, since its
    /// decoders hold more than the RGBA they return, and refuses an image
    /// whose estimate is over it. The `magick` driver hands it to
    /// ImageMagick's own memory limits. Source files are capped at it too.
    ///
    /// The built-in driver also caps at it the bytes its JPEG decoder reads
    /// while reassembling Extended XMP metadata, which grow with the square
    /// of the segment count rather than with their size.
    pub max_alloc_bytes: u64,
    /// Wall-clock seconds an ImageMagick invocation may run for.
    ///
    /// Only the `magick` driver reads it. Without a bound, a delegate that
    /// stalls holds a blocking worker for the life of the process.
    pub magick_timeout_secs: u32,
    /// Whether decoding applies the source's EXIF orientation.
    ///
    /// On by default, as in Laravel: a phone photo comes out upright. Off
    /// keeps the pixels as the sensor wrote them, and the output keeps the
    /// `Orientation` tag so a viewer can still turn them; the pipeline's
    /// [`Image::orient`] applies it at a chosen place instead.
    pub auto_orient: bool,
}

impl Default for ImageConfig {
    fn default() -> Self {
        Self {
            max_dimension: DEFAULT_IMAGE_MAX_DIMENSION,
            max_alloc_bytes: DEFAULT_IMAGE_MAX_ALLOC_BYTES,
            magick_timeout_secs: DEFAULT_IMAGE_MAGICK_TIMEOUT_SECS,
            auto_orient: true,
        }
    }
}

/// Read a raw `IMAGE_AUTO_ORIENT`: `false`, `0`, `no` or `off` turn
/// orientation on decode off, `true`, `1`, `yes` or `on` (or no value) keep
/// it. Anything else keeps the default with a warning, as the other image
/// settings clamp rather than fail boot.
fn parse_auto_orient(raw: Option<&str>) -> bool {
    let Some(raw) = raw else {
        return true;
    };
    match raw.trim().to_ascii_lowercase().as_str() {
        "false" | "0" | "no" | "off" => false,
        "true" | "1" | "yes" | "on" => true,
        _ => {
            tracing::warn!(
                env = "IMAGE_AUTO_ORIENT",
                value = raw,
                "IMAGE_AUTO_ORIENT is not true or false; orienting on decode, the default"
            );
            true
        }
    }
}

/// Clamp a raw `IMAGE_MAX_DIMENSION`, warning when it moves.
///
/// Pulled out of [`ImageConfig::from_env`] so the clamp can be tested
/// directly: the env-var path is process-global and would have to be
/// serialised against every other test to exercise the same three lines.
fn clamp_max_dimension(raw: u32) -> u32 {
    if raw == 0 {
        tracing::warn!(
            env = "IMAGE_MAX_DIMENSION",
            value = raw,
            clamped_to = 1u32,
            "IMAGE_MAX_DIMENSION of 0 would reject every image; clamping"
        );
        return 1;
    }
    raw
}

/// Clamp a raw `IMAGE_MAX_ALLOC_BYTES`. The floor is one RGBA pixel.
fn clamp_max_alloc_bytes(raw: u64) -> u64 {
    if raw < 4 {
        tracing::warn!(
            env = "IMAGE_MAX_ALLOC_BYTES",
            value = raw,
            clamped_to = 4u64,
            "IMAGE_MAX_ALLOC_BYTES below one RGBA pixel; clamping"
        );
        return 4;
    }
    raw
}

/// Clamp a raw `IMAGE_MAGICK_TIMEOUT_SECS`. Zero would mean "no time at all".
fn clamp_magick_timeout_secs(raw: u32) -> u32 {
    if raw == 0 {
        tracing::warn!(
            env = "IMAGE_MAGICK_TIMEOUT_SECS",
            value = raw,
            clamped_to = 1u32,
            "IMAGE_MAGICK_TIMEOUT_SECS of 0 would fail every invocation; clamping"
        );
        return 1;
    }
    raw
}

impl ImageConfig {
    /// Build the config from the process environment, clamping nonsense.
    pub fn from_env() -> Self {
        let defaults = Self::default();

        let max_dimension = env_optional::<u32>("IMAGE_MAX_DIMENSION")
            .map(clamp_max_dimension)
            .unwrap_or(defaults.max_dimension);

        let max_alloc_bytes = env_optional::<u64>("IMAGE_MAX_ALLOC_BYTES")
            .map(clamp_max_alloc_bytes)
            .unwrap_or(defaults.max_alloc_bytes);

        let magick_timeout_secs = env_optional::<u32>("IMAGE_MAGICK_TIMEOUT_SECS")
            .map(clamp_magick_timeout_secs)
            .unwrap_or(defaults.magick_timeout_secs);

        let auto_orient = parse_auto_orient(std::env::var("IMAGE_AUTO_ORIENT").ok().as_deref());

        Self {
            max_dimension,
            max_alloc_bytes,
            magick_timeout_secs,
            auto_orient,
        }
    }
}

static CONFIG: OnceLock<ImageConfig> = OnceLock::new();
static CONFIG_OVERRIDE: RwLock<Option<ImageConfig>> = RwLock::new(None);
static DEFAULT_DRIVER: OnceLock<Box<dyn ImageDriver>> = OnceLock::new();
static OXIDEAV_DRIVER: OnceLock<OxideAvImageDriver> = OnceLock::new();
static MAGICK_DRIVER: OnceLock<MagickCliDriver> = OnceLock::new();

/// The active decode limits.
///
/// Resolved from the environment on first use and cached. Infallible by
/// design - limits clamp rather than fail, so a driver deep in a blocking
/// thread never has to decide what to do about an unreadable config.
pub fn config() -> ImageConfig {
    if let Ok(guard) = CONFIG_OVERRIDE.read()
        && let Some(config) = *guard
    {
        return config;
    }
    *CONFIG.get_or_init(ImageConfig::from_env)
}

/// Override the active [`ImageConfig`], or restore the environment-derived
/// one with `None`.
///
/// Exists so tests can exercise the limit paths without an env-var dance
/// (the limits are read on every decode, and a `OnceLock` cannot be reset).
/// Not part of the supported surface.
#[doc(hidden)]
pub fn set_config_for_tests(config: Option<ImageConfig>) {
    if let Ok(mut guard) = CONFIG_OVERRIDE.write() {
        *guard = config;
    }
}

/// Resolve the active image driver, building it on first use.
///
/// Configuration errors propagate rather than falling back, so an
/// `IMAGE_DRIVER` typo surfaces as an error naming the valid values.
pub fn default_driver() -> Result<&'static dyn ImageDriver, FrameworkError> {
    if let Some(driver) = DEFAULT_DRIVER.get() {
        return Ok(driver.as_ref());
    }
    let driver: Box<dyn ImageDriver> = match ImageDriverKind::from_env()? {
        ImageDriverKind::OxideAv => Box::new(OxideAvImageDriver::new()),
        ImageDriverKind::Magick => Box::new(MagickCliDriver::from_env()),
    };
    // Race-safe: if another thread won, discard ours and use the winner -
    // both were built from the same environment.
    let _ = DEFAULT_DRIVER.set(driver);
    Ok(DEFAULT_DRIVER
        .get()
        .expect("DEFAULT_DRIVER initialised above")
        .as_ref())
}

/// The built-in driver of `kind`, for an image that chose its driver with
/// [`Image::using`].
///
/// Each is built once, from the environment, beside the default driver
/// rather than in place of it: one image's choice never changes the driver
/// another image or the process default runs.
fn driver_of(kind: ImageDriverKind) -> &'static dyn ImageDriver {
    match kind {
        ImageDriverKind::OxideAv => OXIDEAV_DRIVER.get_or_init(OxideAvImageDriver::new),
        ImageDriverKind::Magick => MAGICK_DRIVER.get_or_init(MagickCliDriver::from_env),
    }
}

/// Install a custom image driver.
///
/// This is the supported escape hatch for formats the framework does not
/// ship - wrap libvips, an external binary, a cloud service, anything that
/// can implement [`ImageDriver`]. The app owns whatever that decoder drags
/// in, including its licensing.
///
/// Returns an error if a driver is already active: the driver does not flip
/// mid-process, so call this during bootstrap, before the first image.
pub fn set_default_driver(driver: Box<dyn ImageDriver>) -> Result<(), FrameworkError> {
    DEFAULT_DRIVER.set(driver).map_err(|_| {
        FrameworkError::internal(
            "image: default driver already initialised; cannot override after first use",
        )
    })
}

/// Where an [`Image`]'s bytes come from. Resolved at terminal time.
#[derive(Debug, Clone)]
enum Source {
    Bytes(Bytes),
    Path(PathBuf),
    #[cfg(feature = "filesystem")]
    Disk {
        disk: String,
        path: String,
    },
}

/// A lazily-evaluated image pipeline.
///
/// Build it with a constructor, chain operations, finish with a terminal.
/// Nothing is read, decoded, or encoded until the terminal runs, and the
/// whole pixel pipeline runs on a blocking thread so it never stalls the
/// async runtime.
///
/// Cloning is cheap and copies only the recorded instructions - a clone
/// re-runs the pipeline from its source rather than sharing a result.
///
/// Deliberately not serialisable: Laravel throws on `__serialize` for the
/// same reason, since the useful thing to persist is the path or the disk
/// key, not the pixels.
#[derive(Debug, Clone)]
pub struct Image {
    source: Source,
    pipeline: ImagePipeline,
    /// The built-in driver this image chose, over the process default.
    driver: Option<ImageDriverKind>,
}

impl Image {
    fn new(source: Source) -> Self {
        Self {
            source,
            pipeline: ImagePipeline::default(),
            driver: None,
        }
    }

    fn push(mut self, step: Transformation) -> Self {
        self.pipeline.transformations.push(step);
        self
    }

    // ───────────────────────── construction ─────────────────────────

    /// Start from bytes already in memory.
    pub fn from_bytes(bytes: impl Into<Bytes>) -> Self {
        Self::new(Source::Bytes(bytes.into()))
    }

    /// Start from a filesystem path. The file is read at terminal time.
    pub fn from_path(path: impl Into<PathBuf>) -> Self {
        Self::new(Source::Path(path.into()))
    }

    /// Start from a file on a [`Storage`](crate::Storage) disk, read at
    /// terminal time.
    #[cfg(feature = "filesystem")]
    pub fn from_disk(disk: &str, path: &str) -> Self {
        Self::new(Source::Disk {
            disk: disk.to_string(),
            path: path.to_string(),
        })
    }

    /// Start from an uploaded file.
    ///
    /// **Eager**: the upload's bytes are read now, not at terminal time,
    /// because a disk-backed upload's temp file is deleted when the request
    /// ends and an `Image` routinely outlives that.
    pub async fn from_upload<V>(
        file: &crate::http::upload::UploadedFile<V>,
    ) -> Result<Self, FrameworkError>
    where
        V: crate::http::upload::validators::UploadValidator,
    {
        Ok(Self::from_bytes(file.bytes().await?))
    }

    /// Start from a byte stream.
    ///
    /// **Eager**: a stream can only be consumed once, so it is drained now.
    /// The running total is checked against `IMAGE_MAX_ALLOC_BYTES` *while*
    /// collecting, so an endless stream is cut off rather than being
    /// discovered after it has already filled memory.
    pub async fn from_stream<S>(stream: S) -> Result<Self, FrameworkError>
    where
        S: futures_util::Stream<Item = std::io::Result<Bytes>> + Send,
    {
        let collected = collect_capped(stream, config().max_alloc_bytes, 0, "stream").await?;
        Ok(Self::from_bytes(collected))
    }

    // ───────────────────────── operations ─────────────────────────

    /// Force exact dimensions, ignoring the source aspect ratio.
    pub fn resize(self, width: u32, height: u32) -> Self {
        self.push(Transformation::Resize { width, height })
    }

    /// Resize to a width, deriving the height from the aspect ratio.
    pub fn resize_width(self, width: u32) -> Self {
        self.push(Transformation::ResizeWidth(width))
    }

    /// Resize to a height, deriving the width from the aspect ratio.
    pub fn resize_height(self, height: u32) -> Self {
        self.push(Transformation::ResizeHeight(height))
    }

    /// Fit inside a box, preserving aspect ratio. Never enlarges.
    pub fn scale(self, width: u32, height: u32) -> Self {
        self.push(Transformation::Scale { width, height })
    }

    /// Scale down to at most a width. Never enlarges.
    pub fn scale_width(self, width: u32) -> Self {
        self.push(Transformation::ScaleWidth(width))
    }

    /// Scale down to at most a height. Never enlarges.
    pub fn scale_height(self, height: u32) -> Self {
        self.push(Transformation::ScaleHeight(height))
    }

    /// Cut a rectangle out of the image.
    pub fn crop(self, width: u32, height: u32, x: u32, y: u32) -> Self {
        self.push(Transformation::Crop {
            width,
            height,
            x,
            y,
        })
    }

    /// Fill the target box exactly, cropping the overflow from the centre.
    pub fn cover(self, width: u32, height: u32) -> Self {
        self.push(Transformation::Cover { width, height })
    }

    /// Fit inside the target box, preserving aspect ratio. No padding.
    pub fn contain(self, width: u32, height: u32) -> Self {
        self.push(Transformation::Contain { width, height })
    }

    /// Rotate clockwise by an arbitrary angle, growing the canvas to fit.
    ///
    /// The corners the turn exposes are white when the output is JPEG or
    /// GIF, which cannot hold transparency, and transparent when it is PNG,
    /// WebP or BMP. [`Image::rotate_with_background`] names the colour.
    pub fn rotate(self, angle: f32) -> Self {
        self.push(Transformation::Rotate {
            degrees: angle,
            background: None,
        })
    }

    /// Rotate clockwise, filling the exposed corners with `background`
    /// (Laravel's `rotate($angle, $background)`).
    ///
    /// JPEG and GIF output also flattens any transparency onto this colour.
    pub fn rotate_with_background(self, angle: f32, background: Color) -> Self {
        self.push(Transformation::Rotate {
            degrees: angle,
            background: Some(background),
        })
    }

    /// Mirror top-to-bottom (Laravel's `flip`).
    pub fn flip_vertically(self) -> Self {
        self.push(Transformation::FlipVertically)
    }

    /// Mirror left-to-right (Laravel's `flop`).
    pub fn flip_horizontally(self) -> Self {
        self.push(Transformation::FlipHorizontally)
    }

    /// Mirror top-to-bottom: Laravel's name for
    /// [`Image::flip_vertically`].
    pub fn flip(self) -> Self {
        self.flip_vertically()
    }

    /// Mirror left-to-right: Laravel's name for
    /// [`Image::flip_horizontally`].
    pub fn flop(self) -> Self {
        self.flip_horizontally()
    }

    /// Apply the source's EXIF orientation at this point in the pipeline.
    ///
    /// Decoding already orients by default, so this does nothing then.
    /// With `IMAGE_AUTO_ORIENT=false` the pixels arrive as the sensor wrote
    /// them, and this is where they turn upright.
    pub fn orient(self) -> Self {
        self.push(Transformation::Orient)
    }

    /// Add any transformation to the pipeline: a built-in one, or a custom
    /// one registered with [`register_transformation`], as
    /// `Transformation::custom(name)` (Laravel's `transform`).
    pub fn transform(self, transformation: Transformation) -> Self {
        self.push(transformation)
    }

    /// Gaussian blur. `amount` clamps to `0..=100`; `0` is a no-op.
    pub fn blur(self, amount: u32) -> Self {
        self.push(Transformation::Blur(amount.min(100)))
    }

    /// Unsharp-mask sharpen. `amount` clamps to `0..=100`; `0` is a no-op.
    pub fn sharpen(self, amount: u32) -> Self {
        self.push(Transformation::Sharpen(amount.min(100)))
    }

    /// Desaturate to grey. Spelled the Laravel way.
    pub fn grayscale(self) -> Self {
        self.push(Transformation::Grayscale)
    }

    /// Encode to a specific format. Without this the source format is kept.
    pub fn to_format(mut self, format: OutputFormat) -> Self {
        self.pipeline.format = Some(format);
        self
    }

    /// Set the encode quality. Clamped to `1..=100`; defaults to 70.
    ///
    /// Only the lossy encoders read it - see [`ImagePipeline::quality`].
    pub fn quality(mut self, quality: u8) -> Self {
        self.pipeline.quality = quality.clamp(1, 100);
        self
    }

    /// Encode to `format` at `quality`, in one call (Laravel's `optimize`).
    pub fn optimize(self, format: OutputFormat, quality: u8) -> Self {
        self.to_format(format).quality(quality)
    }

    /// Encode to WebP: [`OutputFormat::WebP`].
    pub fn to_webp(self) -> Self {
        self.to_format(OutputFormat::WebP)
    }

    /// Encode to JPEG: [`OutputFormat::Jpeg`].
    pub fn to_jpg(self) -> Self {
        self.to_format(OutputFormat::Jpeg)
    }

    /// Encode to JPEG: [`OutputFormat::Jpeg`], under Laravel's other name.
    pub fn to_jpeg(self) -> Self {
        self.to_format(OutputFormat::Jpeg)
    }

    /// Encode to PNG: [`OutputFormat::Png`].
    pub fn to_png(self) -> Self {
        self.to_format(OutputFormat::Png)
    }

    /// Encode to GIF: [`OutputFormat::Gif`].
    pub fn to_gif(self) -> Self {
        self.to_format(OutputFormat::Gif)
    }

    /// Encode to BMP: [`OutputFormat::Bmp`].
    pub fn to_bmp(self) -> Self {
        self.to_format(OutputFormat::Bmp)
    }

    /// Process this image with the built-in driver `driver`, whatever the
    /// process default is (Laravel's `using`).
    ///
    /// The choice belongs to this image and its clones only. HEIC uploads
    /// can go to `magick` while every other image stays on `oxideav`.
    pub fn using(mut self, driver: ImageDriverKind) -> Self {
        self.driver = Some(driver);
        self
    }

    // ───────────────────────── terminals ─────────────────────────

    /// Read the source, then run the pipeline on a blocking thread.
    ///
    /// Source I/O happens here, in async context, *before* the blocking hop,
    /// so a slow disk never occupies a blocking worker.
    async fn blocking<T, F>(self, op: F) -> Result<T, FrameworkError>
    where
        F: FnOnce(&'static dyn ImageDriver, &[u8], &ImagePipeline) -> Result<T, FrameworkError>
            + Send
            + 'static,
        T: Send + 'static,
    {
        let driver = match self.driver {
            Some(kind) => driver_of(kind),
            None => default_driver()?,
        };
        let contents = read_source(self.source).await?;
        let pipeline = self.pipeline;
        tokio::task::spawn_blocking(move || op(driver, &contents, &pipeline))
            .await
            .map_err(|e| FrameworkError::internal(format!("image driver panicked: {e}")))?
    }

    /// Run the pipeline and return the encoded bytes.
    pub async fn to_bytes(self) -> Result<Vec<u8>, FrameworkError> {
        self.blocking(|driver, contents, pipeline| driver.process(contents, pipeline))
            .await
    }

    /// Run the pipeline and return it as an HTTP response with the right
    /// `Content-Type`.
    pub async fn to_response(self) -> Result<HttpResponse, FrameworkError> {
        let (bytes, mime) = self
            .blocking(|driver, contents, pipeline| {
                let mime = resolve_mime(contents, pipeline)?;
                Ok((driver.process(contents, pipeline)?, mime))
            })
            .await?;
        Ok(HttpResponse::bytes_body(bytes, mime))
    }

    /// Run the pipeline and write the result to a filesystem path.
    pub async fn save(self, path: &Path) -> Result<(), FrameworkError> {
        let bytes = self.to_bytes().await?;
        tokio::fs::write(path, bytes).await.map_err(|e| {
            FrameworkError::internal(format!("image save failed for {}: {e}", path.display()))
        })
    }

    /// Run the pipeline and return the encoded bytes as standard base64,
    /// padded (Laravel's `toBase64`).
    pub async fn to_base64(self) -> Result<String, FrameworkError> {
        use base64::Engine as _;
        let bytes = self.to_bytes().await?;
        Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
    }

    /// Run the pipeline and return a `data:` URI of the result: its
    /// [`mime_type`](Image::mime_type) and its base64 bytes (Laravel's
    /// `toDataUri`).
    pub async fn to_data_uri(self) -> Result<String, FrameworkError> {
        use base64::Engine as _;
        let (bytes, mime) = self
            .blocking(|driver, contents, pipeline| {
                let mime = resolve_mime(contents, pipeline)?;
                Ok((driver.process(contents, pipeline)?, mime))
            })
            .await?;
        let payload = base64::engine::general_purpose::STANDARD.encode(bytes);
        Ok(format!("data:{mime};base64,{payload}"))
    }

    /// Run the pipeline and store the result on a disk, in `directory`,
    /// under a generated name: 40 random letters and digits and the output
    /// format's extension. Returns the stored path (Laravel's `store`).
    ///
    /// `disk` names a [`Storage`](crate::Storage) disk; `None` is the
    /// application's default disk. A disk with a public base URL is what
    /// makes the file public: Suprnova sets visibility per disk, not per
    /// file, so there is no `store_publicly`.
    #[cfg(feature = "filesystem")]
    pub async fn store(
        self,
        directory: &str,
        disk: Option<&str>,
    ) -> Result<String, FrameworkError> {
        let (bytes, format) = self.processed_with_format().await?;
        let name = format!("{}.{}", generated_name(), format.extension());
        put_on_disk(disk, &stored_path(directory, &name), bytes).await
    }

    /// Run the pipeline and store the result on a disk as
    /// `directory/name`. Returns the stored path (Laravel's `storeAs`).
    ///
    /// `disk` is as for [`Image::store`].
    #[cfg(feature = "filesystem")]
    pub async fn store_as(
        self,
        directory: &str,
        name: &str,
        disk: Option<&str>,
    ) -> Result<String, FrameworkError> {
        let bytes = self.to_bytes().await?;
        put_on_disk(disk, &stored_path(directory, name), bytes).await
    }

    /// Run the pipeline and return the encoded bytes with the format they
    /// are in.
    #[cfg(feature = "filesystem")]
    async fn processed_with_format(self) -> Result<(Vec<u8>, OutputFormat), FrameworkError> {
        self.blocking(|driver, contents, pipeline| {
            let format = resolve_format(contents, pipeline)?;
            Ok((driver.process(contents, pipeline)?, format))
        })
        .await
    }

    /// Dimensions of the **processed** image, as Laravel reports them.
    pub async fn dimensions(self) -> Result<(u32, u32), FrameworkError> {
        self.blocking(|driver, contents, pipeline| {
            let processed = driver.process(contents, pipeline)?;
            driver.dimensions(&processed)
        })
        .await
    }

    /// Width of the **processed** image: the first of
    /// [`dimensions`](Image::dimensions).
    pub async fn width(self) -> Result<u32, FrameworkError> {
        Ok(self.dimensions().await?.0)
    }

    /// Height of the **processed** image: the second of
    /// [`dimensions`](Image::dimensions).
    pub async fn height(self) -> Result<u32, FrameworkError> {
        Ok(self.dimensions().await?.1)
    }

    /// `Content-Type` of the **processed** image.
    ///
    /// The pipeline still runs: reporting a type for an image that cannot
    /// actually be produced would be a lie a caller only discovers later.
    pub async fn mime_type(self) -> Result<String, FrameworkError> {
        self.blocking(|driver, contents, pipeline| {
            let mime = resolve_mime(contents, pipeline)?;
            driver.process(contents, pipeline)?;
            Ok(mime)
        })
        .await
    }

    /// Average colour of the **processed** image, as `#rrggbb`.
    pub async fn dominant_color(self) -> Result<String, FrameworkError> {
        self.blocking(|driver, contents, pipeline| {
            let processed = driver.process(contents, pipeline)?;
            driver.dominant_color(&processed)
        })
        .await
    }
}

/// The format a pipeline will produce: its target format, or the source's
/// own format when the pipeline does not convert.
#[cfg(feature = "filesystem")]
fn resolve_format(
    contents: &[u8],
    pipeline: &ImagePipeline,
) -> Result<OutputFormat, FrameworkError> {
    if let Some(format) = pipeline.format {
        return Ok(format);
    }
    sniff::detect(contents)
        .map(|format| match format {
            sniff::InputFormat::Png => OutputFormat::Png,
            sniff::InputFormat::Jpeg => OutputFormat::Jpeg,
            sniff::InputFormat::WebP => OutputFormat::WebP,
            sniff::InputFormat::Gif => OutputFormat::Gif,
            sniff::InputFormat::Bmp => OutputFormat::Bmp,
        })
        .ok_or_else(|| {
            FrameworkError::param(
                "image format is not recognised, so its file extension cannot be chosen; \
                 call to_format() to choose one",
            )
        })
}

/// A name for a stored image: 40 random letters and digits, Laravel's
/// `Str::random(40)`. 62^40 names make two images in one directory meeting
/// on one as likely as guessing a 238-bit key.
#[cfg(feature = "filesystem")]
fn generated_name() -> String {
    use rand::RngExt;
    let mut rng = rand::rng();
    (0..40)
        .map(|_| char::from(rng.sample(rand::distr::Alphanumeric)))
        .collect()
}

/// `directory/name`, each trimmed of the slashes at its ends, as Laravel
/// trims the joined path: an empty directory stores at the disk's root, and
/// `avatars/` and `/avatars` name the same directory.
#[cfg(feature = "filesystem")]
fn stored_path(directory: &str, name: &str) -> String {
    let (directory, name) = (directory.trim_matches('/'), name.trim_matches('/'));
    if directory.is_empty() {
        name.to_string()
    } else {
        format!("{directory}/{name}")
    }
}

/// Write `bytes` to `path` on `disk`, or on the default disk, and return
/// the path.
#[cfg(feature = "filesystem")]
async fn put_on_disk(
    disk: Option<&str>,
    path: &str,
    bytes: Vec<u8>,
) -> Result<String, FrameworkError> {
    use crate::DiskExt;
    let handle = match disk {
        Some(name) => crate::Storage::disk(name)?,
        None => crate::Storage::default_disk()?,
    };
    handle.put(path, bytes).await?;
    Ok(path.to_string())
}

/// The `Content-Type` a pipeline will produce: its target format, or the
/// source's own format when the pipeline does not convert.
fn resolve_mime(contents: &[u8], pipeline: &ImagePipeline) -> Result<String, FrameworkError> {
    if let Some(format) = pipeline.format {
        return Ok(format.mime_type().to_string());
    }
    sniff::detect(contents)
        .map(|format| format.mime_type().to_string())
        .ok_or_else(|| {
            FrameworkError::param(
                "image format is not recognised, so its media type cannot be reported; \
                 call to_format() to choose one",
            )
        })
}

/// Refuse a source whose raw bytes already exceed the allocation budget.
///
/// The header gate bounds what a *decode* costs, but it cannot help once the
/// encoded file is already resident: a 4 GiB PNG on a storage disk is 4 GiB of
/// RAM before a single header is parsed. `from_stream` has always counted as
/// it collected; this gives the path and disk sources the same ceiling instead
/// of leaving them as the one uncapped way in.
fn check_source_size(len: u64, cap: u64, source: &str) -> Result<(), FrameworkError> {
    if len > cap {
        return Err(FrameworkError::param(format!(
            "image exceeds configured decode limits: {source} is {len} bytes, over the \
             IMAGE_MAX_ALLOC_BYTES limit of {cap}"
        )));
    }
    Ok(())
}

/// Append `data` to `buffer`, growing it by doubling but never to a capacity
/// past `limit`.
///
/// `Vec`'s own growth doubles past what is needed, so a buffer filled to just
/// over half the cap would reserve close to twice the cap. Callers check the
/// length against `limit` first, so the clamp is always enough room.
fn extend_within(buffer: &mut Vec<u8>, data: &[u8], limit: usize) {
    let needed = buffer.len().saturating_add(data.len());
    if needed > buffer.capacity() {
        let grown = buffer
            .capacity()
            .saturating_mul(2)
            .max(needed)
            .min(limit.max(needed));
        buffer.reserve_exact(grown - buffer.len());
    }
    buffer.extend_from_slice(data);
}

/// Collect a byte stream, refusing it once it passes `cap` bytes.
///
/// `capacity_hint` is a size reported before the read, already checked
/// against the cap; zero means unknown.
async fn collect_capped<S>(
    stream: S,
    cap: u64,
    capacity_hint: u64,
    source: &str,
) -> Result<Vec<u8>, FrameworkError>
where
    S: futures_util::Stream<Item = std::io::Result<Bytes>>,
{
    use futures_util::TryStreamExt;

    let limit = usize::try_from(cap).unwrap_or(usize::MAX);
    let mut collected = Vec::with_capacity(usize::try_from(capacity_hint.min(cap)).unwrap_or(0));
    let mut stream = std::pin::pin!(stream);
    while let Some(chunk) = stream
        .try_next()
        .await
        .map_err(|e| FrameworkError::internal(format!("image {source} read failed: {e}")))?
    {
        if collected.len() as u64 + chunk.len() as u64 > cap {
            return Err(FrameworkError::param(format!(
                "image exceeds configured decode limits: {source} is larger than the \
                 IMAGE_MAX_ALLOC_BYTES limit of {cap}"
            )));
        }
        extend_within(&mut collected, &chunk, limit);
    }
    Ok(collected)
}

/// Read a file, reading no more than one byte past `cap`.
///
/// The size the filesystem reports refuses an oversized file before any of
/// it is read, but it is not the size of what a read returns: a FIFO or a
/// device reports zero, and a file can grow between the check and the read.
/// So the read itself stops one byte past the cap, and that byte is what
/// tells an oversized source from one that fits exactly. The buffer grows
/// within the same bound, never to twice it.
fn read_file_capped(path: &Path, cap: u64) -> Result<Vec<u8>, FrameworkError> {
    use std::io::Read;

    let read_failed = |e: std::io::Error| {
        FrameworkError::internal(format!(
            "image source read failed for {}: {e}",
            path.display()
        ))
    };
    let file = std::fs::File::open(path).map_err(read_failed)?;
    let reported = match file.metadata() {
        Ok(metadata) => {
            check_source_size(metadata.len(), cap, "the file")?;
            metadata.len()
        }
        Err(_) => 0,
    };
    let limit = cap.saturating_add(1);
    let limit_len = usize::try_from(limit).unwrap_or(usize::MAX);
    let mut bytes = Vec::with_capacity(usize::try_from(reported.min(limit)).unwrap_or(0));
    let mut reader = file.take(limit);
    let mut chunk = [0u8; 16 * 1024];
    loop {
        let read = match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => read,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(read_failed(e)),
        };
        extend_within(&mut bytes, &chunk[..read], limit_len);
    }
    check_source_size(bytes.len() as u64, cap, "the file")?;
    Ok(bytes)
}

/// The source's bytes, for the driver to read. Bytes the caller handed
/// [`Image::from_bytes`] are passed on as they are, never copied: a driver
/// only reads its input (MEM-003).
async fn read_source(source: Source) -> Result<Bytes, FrameworkError> {
    let cap = config().max_alloc_bytes;
    match source {
        Source::Bytes(bytes) => {
            check_source_size(bytes.len() as u64, cap, "the image")?;
            Ok(bytes)
        }
        Source::Path(path) => tokio::task::spawn_blocking(move || read_file_capped(&path, cap))
            .await
            .map_err(|e| FrameworkError::internal(format!("image source read panicked: {e}")))?
            .map(Bytes::from),
        #[cfg(feature = "filesystem")]
        Source::Disk { disk, path } => {
            use crate::DiskExt;
            let handle = crate::Storage::disk(&disk)?;
            // A reported size over the cap is refused without reading. The
            // report is only a hint: OpenDAL reports zero when a service
            // omits the length, and an object can change after the stat. So
            // the read streams to the end of the object and stops at the cap,
            // whatever the stat said.
            let reported = match handle.size(&path).await {
                Ok(size) => {
                    check_source_size(size, cap, "the stored file")?;
                    size
                }
                Err(_) => 0,
            };
            let storage_error =
                |e: opendal::Error| FrameworkError::internal(format!("storage read({path}): {e}"));
            let stream = handle
                .reader(&path)
                .await
                .map_err(storage_error)?
                .into_bytes_stream(..)
                .await
                .map_err(storage_error)?;
            collect_capped(stream, cap, reported, "stored file")
                .await
                .map(Bytes::from)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn driver_names_parse_case_insensitively() {
        assert_eq!(
            ImageDriverKind::parse("oxideav").expect("oxideav"),
            ImageDriverKind::OxideAv
        );
        assert_eq!(
            ImageDriverKind::parse("  MAGICK ").expect("magick"),
            ImageDriverKind::Magick
        );
        assert_eq!(
            ImageDriverKind::parse("ImageMagick").expect("alias"),
            ImageDriverKind::Magick
        );
    }

    #[test]
    fn an_unknown_driver_name_names_the_valid_ones() {
        let err = ImageDriverKind::parse("gd").expect_err("gd is not a driver");
        let message = err.to_string();
        assert!(message.contains("oxideav"), "got: {message}");
        assert!(message.contains("magick"), "got: {message}");
    }

    #[test]
    fn quality_clamps_into_the_valid_range() {
        assert_eq!(Image::from_bytes(vec![]).quality(0).pipeline.quality, 1);
        assert_eq!(Image::from_bytes(vec![]).quality(255).pipeline.quality, 100);
        assert_eq!(Image::from_bytes(vec![]).quality(55).pipeline.quality, 55);
        assert_eq!(
            Image::from_bytes(vec![]).pipeline.quality,
            DEFAULT_IMAGE_QUALITY,
            "the default matches Laravel's"
        );
    }

    #[test]
    fn blur_and_sharpen_amounts_clamp_at_the_recording_layer() {
        let image = Image::from_bytes(vec![]).blur(500).sharpen(500);
        assert_eq!(
            image.pipeline.transformations,
            vec![Transformation::Blur(100), Transformation::Sharpen(100)]
        );
    }

    #[test]
    fn operations_record_in_order_without_touching_the_source() {
        let image = Image::from_bytes(vec![1, 2, 3])
            .resize(10, 10)
            .grayscale()
            .rotate(90.0);
        assert_eq!(
            image.pipeline.transformations,
            vec![
                Transformation::Resize {
                    width: 10,
                    height: 10
                },
                Transformation::Grayscale,
                Transformation::Rotate {
                    degrees: 90.0,
                    background: None
                },
            ]
        );
        assert!(
            image.pipeline.format.is_none(),
            "no conversion was asked for"
        );
    }

    #[test]
    fn mime_falls_back_to_the_source_format_when_no_conversion_is_requested() {
        let png = b"\x89PNG\r\n\x1a\n";
        let pipeline = ImagePipeline::default();
        assert_eq!(resolve_mime(png, &pipeline).expect("png mime"), "image/png");

        let converting = ImagePipeline {
            format: Some(OutputFormat::WebP),
            ..ImagePipeline::default()
        };
        assert_eq!(
            resolve_mime(png, &converting).expect("webp mime"),
            "image/webp"
        );
    }

    #[test]
    fn mime_of_an_unrecognised_source_is_an_error_not_a_guess() {
        let pipeline = ImagePipeline::default();
        assert!(resolve_mime(&[0u8; 32], &pipeline).is_err());
    }

    #[test]
    fn config_defaults_are_the_documented_ones() {
        let defaults = ImageConfig::default();
        assert_eq!(defaults.max_dimension, DEFAULT_IMAGE_MAX_DIMENSION);
        assert_eq!(defaults.max_alloc_bytes, DEFAULT_IMAGE_MAX_ALLOC_BYTES);
        assert_eq!(
            defaults.magick_timeout_secs,
            DEFAULT_IMAGE_MAGICK_TIMEOUT_SECS
        );
    }

    /// The `ImageConfig` environment docs name the default budget the
    /// constant holds. They still said 256 MiB after the default rose to
    /// 1 GiB, so an operator sizing a host read the wrong limit.
    #[test]
    fn the_config_docs_name_the_default_alloc_budget() {
        let source = include_str!("mod.rs");
        let entry = source
            .split("/// - `IMAGE_MAX_ALLOC_BYTES` - ")
            .nth(1)
            .and_then(|rest| rest.split(").").next())
            .expect("the ImageConfig docs describe IMAGE_MAX_ALLOC_BYTES");
        let entry = entry.split_whitespace().filter(|word| *word != "///");
        let entry = entry.collect::<Vec<_>>().join(" ");
        let gib = DEFAULT_IMAGE_MAX_ALLOC_BYTES / (1024 * 1024 * 1024);
        assert!(
            entry.ends_with(&format!("(default {gib} GiB")),
            "the docs say: {entry}"
        );
    }

    #[test]
    fn out_of_range_limits_clamp_to_a_usable_floor() {
        // The clamps are pure functions precisely so they can be exercised
        // here rather than through a process-global env var.
        assert_eq!(clamp_max_dimension(0), 1, "0 would reject every image");
        assert_eq!(clamp_max_dimension(1), 1);
        assert_eq!(clamp_max_dimension(4096), 4096, "valid values pass through");
        assert_eq!(clamp_max_dimension(u32::MAX), u32::MAX);

        for raw in 0..4u64 {
            assert_eq!(
                clamp_max_alloc_bytes(raw),
                4,
                "{raw} is below one RGBA pixel"
            );
        }
        assert_eq!(clamp_max_alloc_bytes(4), 4);
        assert_eq!(clamp_max_alloc_bytes(1_048_576), 1_048_576);

        assert_eq!(
            clamp_magick_timeout_secs(0),
            1,
            "0 seconds would fail every invocation"
        );
        assert_eq!(clamp_magick_timeout_secs(30), 30);
    }

    #[test]
    fn img_001_image_auto_orient_reads_like_the_other_flags() {
        assert!(parse_auto_orient(None), "on by default");
        for off in ["false", "0", "no", "off", " FALSE "] {
            assert!(!parse_auto_orient(Some(off)), "{off:?} turns it off");
        }
        for on in ["true", "1", "yes", "on"] {
            assert!(parse_auto_orient(Some(on)), "{on:?} keeps it on");
        }
        assert!(
            parse_auto_orient(Some("maybe")),
            "an unknown value keeps the default"
        );
        assert!(ImageConfig::default().auto_orient);
    }

    #[test]
    fn img_007_shortcuts_record_what_to_format_and_quality_record() {
        let image = || Image::from_bytes(vec![]);
        for (shortcut, format) in [
            (image().to_webp(), OutputFormat::WebP),
            (image().to_jpg(), OutputFormat::Jpeg),
            (image().to_jpeg(), OutputFormat::Jpeg),
            (image().to_png(), OutputFormat::Png),
            (image().to_gif(), OutputFormat::Gif),
            (image().to_bmp(), OutputFormat::Bmp),
        ] {
            assert_eq!(shortcut.pipeline, image().to_format(format).pipeline);
        }
        assert_eq!(
            image().optimize(OutputFormat::WebP, 55).pipeline,
            image().to_format(OutputFormat::WebP).quality(55).pipeline
        );
        assert_eq!(
            image().flip().pipeline,
            image().flip_vertically().pipeline,
            "flip is Laravel's top-to-bottom mirror"
        );
        assert_eq!(
            image().flop().pipeline,
            image().flip_horizontally().pipeline,
            "flop is Laravel's left-to-right mirror"
        );
    }

    #[test]
    fn img_003_a_driver_choice_belongs_to_the_image_that_made_it() {
        let image = Image::from_bytes(vec![]);
        let chose = image.clone().using(ImageDriverKind::Magick);
        assert_eq!(chose.driver, Some(ImageDriverKind::Magick));
        assert_eq!(
            chose.clone().resize(1, 1).driver,
            Some(ImageDriverKind::Magick)
        );
        assert_eq!(image.driver, None, "the original keeps the process default");
        assert_eq!(driver_of(ImageDriverKind::OxideAv).name(), "oxideav");
        assert_eq!(driver_of(ImageDriverKind::Magick).name(), "magick");
    }

    #[cfg(feature = "filesystem")]
    #[test]
    fn img_005_a_stored_path_joins_directory_and_name() {
        assert_eq!(stored_path("avatars", "me.png"), "avatars/me.png");
        assert_eq!(stored_path("/avatars/", "/me.png"), "avatars/me.png");
        assert_eq!(stored_path("", "me.png"), "me.png");
        let name = generated_name();
        assert_eq!(name.len(), 40);
        assert!(
            name.bytes().all(|byte| byte.is_ascii_alphanumeric()),
            "{name}"
        );
        assert_ne!(name, generated_name());
    }

    #[test]
    fn source_size_is_capped_before_the_bytes_are_kept() {
        assert!(check_source_size(10, 10, "the file").is_ok());
        let err = check_source_size(11, 10, "the file").expect_err("over the cap");
        assert!(err.to_string().contains("limit"), "got: {err}");
    }
}
