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

mod driver;
mod magick;
mod oxideav;
mod sniff;

use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};

use bytes::Bytes;

use crate::config::env_optional;
use crate::error::FrameworkError;
use crate::http::HttpResponse;

pub use driver::{DEFAULT_IMAGE_QUALITY, ImageDriver, ImagePipeline, OutputFormat, Transformation};
pub use magick::MagickCliDriver;
pub use oxideav::OxideAvImageDriver;

/// Default cap on either pixel dimension of a decoded image.
///
/// 16384 is comfortably past any camera or scanner output while still
/// bounding the decoded buffer to something a server can survive.
pub const DEFAULT_IMAGE_MAX_DIMENSION: u32 = 16_384;

/// Default cap on the decoded RGBA footprint of a single image, in bytes.
pub const DEFAULT_IMAGE_MAX_ALLOC_BYTES: u64 = 256 * 1024 * 1024;

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

/// Decode limits for the image subsystem.
///
/// # Environment variables
///
/// - `IMAGE_MAX_DIMENSION` - cap on width and height in pixels
///   (default 16384).
/// - `IMAGE_MAX_ALLOC_BYTES` - cap on the decoded RGBA footprint
///   (default 256 MiB).
///
/// Out-of-range values clamp with a warning rather than failing boot: a
/// misconfigured limit should be loud, not fatal, and a limit of zero would
/// reject every image in the application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageConfig {
    /// Maximum width or height, in pixels, of a decoded image.
    pub max_dimension: u32,
    /// Maximum decoded RGBA footprint, in bytes.
    pub max_alloc_bytes: u64,
    /// Wall-clock seconds an ImageMagick invocation may run for.
    ///
    /// Only the `magick` driver reads it. Without a bound, a delegate that
    /// stalls holds a blocking worker for the life of the process.
    pub magick_timeout_secs: u32,
}

impl Default for ImageConfig {
    fn default() -> Self {
        Self {
            max_dimension: DEFAULT_IMAGE_MAX_DIMENSION,
            max_alloc_bytes: DEFAULT_IMAGE_MAX_ALLOC_BYTES,
            magick_timeout_secs: DEFAULT_IMAGE_MAGICK_TIMEOUT_SECS,
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

        Self {
            max_dimension,
            max_alloc_bytes,
            magick_timeout_secs,
        }
    }
}

static CONFIG: OnceLock<ImageConfig> = OnceLock::new();
static CONFIG_OVERRIDE: RwLock<Option<ImageConfig>> = RwLock::new(None);
static DEFAULT_DRIVER: OnceLock<Box<dyn ImageDriver>> = OnceLock::new();

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
}

impl Image {
    fn new(source: Source) -> Self {
        Self {
            source,
            pipeline: ImagePipeline::default(),
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
        use futures_util::TryStreamExt;

        let cap = config().max_alloc_bytes;
        let mut collected: Vec<u8> = Vec::new();
        let mut stream = std::pin::pin!(stream);
        while let Some(chunk) = stream
            .try_next()
            .await
            .map_err(|e| FrameworkError::internal(format!("image stream read failed: {e}")))?
        {
            if collected.len() as u64 + chunk.len() as u64 > cap {
                return Err(FrameworkError::param(format!(
                    "image exceeds configured decode limits: stream is larger than the \
                     IMAGE_MAX_ALLOC_BYTES limit of {cap}"
                )));
            }
            collected.extend_from_slice(&chunk);
        }
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
    pub fn rotate(self, angle: f32) -> Self {
        self.push(Transformation::Rotate(angle))
    }

    /// Mirror top-to-bottom (Laravel's `flip`).
    pub fn flip_vertically(self) -> Self {
        self.push(Transformation::FlipVertically)
    }

    /// Mirror left-to-right (Laravel's `flop`).
    pub fn flip_horizontally(self) -> Self {
        self.push(Transformation::FlipHorizontally)
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
        let driver = default_driver()?;
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

    /// Run the pipeline and write the result to a storage disk.
    #[cfg(feature = "filesystem")]
    pub async fn store(self, disk: &str, path: &str) -> Result<(), FrameworkError> {
        use crate::DiskExt;
        let bytes = self.to_bytes().await?;
        crate::Storage::disk(disk)?.put(path, bytes).await
    }

    /// Dimensions of the **processed** image, as Laravel reports them.
    pub async fn dimensions(self) -> Result<(u32, u32), FrameworkError> {
        self.blocking(|driver, contents, pipeline| {
            let processed = driver.process(contents, pipeline)?;
            driver.dimensions(&processed)
        })
        .await
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
fn check_source_size(len: usize, cap: u64, source: &str) -> Result<(), FrameworkError> {
    if len as u64 > cap {
        return Err(FrameworkError::param(format!(
            "image exceeds configured decode limits: {source} is {len} bytes, over the \
             IMAGE_MAX_ALLOC_BYTES limit of {cap}"
        )));
    }
    Ok(())
}

async fn read_source(source: Source) -> Result<Vec<u8>, FrameworkError> {
    let cap = config().max_alloc_bytes;
    match source {
        Source::Bytes(bytes) => {
            check_source_size(bytes.len(), cap, "the image")?;
            Ok(bytes.to_vec())
        }
        Source::Path(path) => {
            // Ask the filesystem how big it is before reading it, so an
            // oversized file is refused rather than read and then rejected.
            if let Ok(metadata) = tokio::fs::metadata(&path).await {
                check_source_size(metadata.len() as usize, cap, "the file")?;
            }
            let bytes = tokio::fs::read(&path).await.map_err(|e| {
                FrameworkError::internal(format!(
                    "image source read failed for {}: {e}",
                    path.display()
                ))
            })?;
            check_source_size(bytes.len(), cap, "the file")?;
            Ok(bytes)
        }
        #[cfg(feature = "filesystem")]
        Source::Disk { disk, path } => {
            use crate::DiskExt;
            let handle = crate::Storage::disk(&disk)?;
            if let Ok(size) = handle.size(&path).await {
                check_source_size(size as usize, cap, "the stored file")?;
            }
            let bytes = handle.get(&path).await?;
            check_source_size(bytes.len(), cap, "the stored file")?;
            Ok(bytes)
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
                Transformation::Rotate(90.0),
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
    fn source_size_is_capped_before_the_bytes_are_kept() {
        assert!(check_source_size(10, 10, "the file").is_ok());
        let err = check_source_size(11, 10, "the file").expect_err("over the cap");
        assert!(err.to_string().contains("limit"), "got: {err}");
    }
}
