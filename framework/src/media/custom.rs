//! Custom transformations: a pixel step an application registers by name,
//! applied by both built-in drivers at its place in the pipeline.
//!
//! Laravel's `transformUsing` lets an application add a transformation to a
//! driver. Here a custom transformation is a function over decoded pixels
//! rather than driver code, for two reasons. It works the same under both
//! drivers, so switching `IMAGE_DRIVER` does not drop it. And it never
//! contributes ImageMagick arguments: under `magick` the driver hands the
//! function the pixels between two ImageMagick runs over stdin and stdout,
//! so the module's promise that no caller text reaches an argument position
//! still holds.

use std::any::Any;
use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, OnceLock, RwLock};

use crate::error::FrameworkError;

/// The decoded pixels a custom transformation receives and returns: packed
/// 8-bit RGBA, rows top to bottom, no padding.
///
/// `pixels().len()` is always `width * height * 4`; [`ImagePixels::new`]
/// refuses anything else, so a transformation can index rows without
/// checking.
#[derive(Clone, PartialEq, Eq)]
pub struct ImagePixels {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl fmt::Debug for ImagePixels {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The pixels themselves would print megabytes.
        f.debug_struct("ImagePixels")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

impl ImagePixels {
    /// Wrap packed RGBA of `width x height`.
    ///
    /// Errors when a side is zero or `pixels` is not exactly
    /// `width * height * 4` bytes. A transformation that changes the size
    /// builds its result with this, so the driver never receives a buffer
    /// shorter than the size it claims.
    pub fn new(width: u32, height: u32, pixels: Vec<u8>) -> Result<Self, FrameworkError> {
        if width == 0 || height == 0 {
            return Err(FrameworkError::param(
                "image pixels must have a width and a height greater than zero",
            ));
        }
        let needed = (width as usize)
            .checked_mul(height as usize)
            .and_then(|count| count.checked_mul(4))
            .ok_or_else(|| {
                FrameworkError::param("image dimensions overflow the addressable pixel buffer")
            })?;
        if pixels.len() != needed {
            return Err(FrameworkError::param(format!(
                "image pixels for {width}x{height} must be {needed} bytes of RGBA, got {}",
                pixels.len()
            )));
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    /// Width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The packed RGBA bytes.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// The packed RGBA bytes, to change in place. The size cannot change
    /// through this; build a new [`ImagePixels`] to resize.
    pub fn pixels_mut(&mut self) -> &mut [u8] {
        &mut self.pixels
    }

    /// Take the bytes out, to build a result of another size.
    pub fn into_pixels(self) -> Vec<u8> {
        self.pixels
    }
}

/// The settings of one step, kept with the name of their type for the
/// errors that name it.
#[derive(Clone)]
struct StepSettings {
    type_name: &'static str,
    value: Arc<dyn Any + Send + Sync>,
}

/// The settings of the custom steps of one pipeline, recorded with
/// [`Image::transform_with`](super::Image::transform_with).
///
/// The image holds them rather than the step, so a
/// [`Transformation`](super::Transformation) and a [`CustomTransformation`]
/// stay `Copy`: a step carries a key into this table. Clones of the image
/// share the values, and the values are released when the last image or
/// pipeline holding them is dropped. Two tables are equal when they hold
/// the very same values.
#[derive(Clone, Default)]
pub struct TransformationSettings {
    entries: Vec<StepSettings>,
}

impl TransformationSettings {
    /// Keep `settings` and return the key a step finds them by.
    pub(crate) fn push<S: Any + Send + Sync>(&mut self, settings: S) -> usize {
        self.entries.push(StepSettings {
            type_name: std::any::type_name::<S>(),
            value: Arc::new(settings),
        });
        self.entries.len() - 1
    }

    /// How many steps' settings the table holds.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the table holds no settings: no step of the pipeline was
    /// recorded with [`Image::transform_with`](super::Image::transform_with).
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl fmt::Debug for TransformationSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The values are opaque; their types say what the steps carry.
        f.debug_list()
            .entries(self.entries.iter().map(|entry| entry.type_name))
            .finish()
    }
}

impl PartialEq for TransformationSettings {
    fn eq(&self, other: &Self) -> bool {
        self.entries.len() == other.entries.len()
            && self
                .entries
                .iter()
                .zip(&other.entries)
                .all(|(left, right)| Arc::ptr_eq(&left.value, &right.value))
    }
}

/// The function behind a registered custom transformation: the pixels and,
/// for a step recorded with settings, those settings. Each registration
/// wraps the application's function in the check of what the step carries.
type CustomFunction =
    dyn Fn(ImagePixels, Option<&StepSettings>) -> Result<ImagePixels, FrameworkError> + Send + Sync;

/// A custom transformation, by the name it was registered under.
///
/// A copyable handle rather than the function itself, so a
/// [`Transformation`](super::Transformation) stays plain data that an
/// [`Image`](super::Image) can record, clone and compare. The function is
/// looked up when the pipeline runs. A step recorded with
/// [`Image::transform_with`](super::Image::transform_with) also carries the
/// key of its settings in the pipeline's [`TransformationSettings`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CustomTransformation {
    name: &'static str,
    settings: Option<usize>,
}

impl CustomTransformation {
    /// A handle for the transformation registered as `name`.
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            settings: None,
        }
    }

    /// A handle for `name` whose settings are the entry `key` of the
    /// pipeline's table.
    pub(crate) const fn with_settings(name: &'static str, key: usize) -> Self {
        Self {
            name,
            settings: Some(key),
        }
    }

    /// The name it is registered under.
    pub const fn name(self) -> &'static str {
        self.name
    }

    /// Run the registered function on `pixels`.
    ///
    /// For a step without settings. A step recorded with
    /// [`Image::transform_with`](super::Image::transform_with) finds its
    /// settings in the pipeline, so it runs through
    /// [`apply_with`](Self::apply_with) and errors here. Errors, naming
    /// the transformation, when nothing is registered under its name.
    pub fn apply(self, pixels: ImagePixels) -> Result<ImagePixels, FrameworkError> {
        if self.settings.is_some() {
            return Err(FrameworkError::param(format!(
                "image transformation `{}` was recorded with settings, which the pipeline \
                 holds; run it with CustomTransformation::apply_with and the pipeline's settings",
                self.name
            )));
        }
        self.run(pixels, None)
    }

    /// Run the registered function on `pixels`, with the settings this
    /// step was recorded with, from `settings`, the
    /// [`settings`](super::ImagePipeline::settings) of the pipeline the
    /// step belongs to.
    ///
    /// Both built-in drivers call this at the transformation's place in
    /// the pipeline; a custom [`ImageDriver`](super::ImageDriver) can too.
    /// Errors, naming the transformation, when nothing is registered under
    /// its name, when the step's settings are not in `settings`, and when
    /// they are not of the type the function was registered for.
    pub fn apply_with(
        self,
        pixels: ImagePixels,
        settings: &TransformationSettings,
    ) -> Result<ImagePixels, FrameworkError> {
        let step = match self.settings {
            Some(key) => Some(settings.entries.get(key).ok_or_else(|| {
                FrameworkError::param(format!(
                    "image transformation `{}` has no settings in this pipeline; a step \
                     recorded with Image::transform_with runs in the pipeline it was recorded in",
                    self.name
                ))
            })?),
            None => None,
        };
        self.run(pixels, step)
    }

    fn run(
        self,
        pixels: ImagePixels,
        settings: Option<&StepSettings>,
    ) -> Result<ImagePixels, FrameworkError> {
        let function = registered(self.name).ok_or_else(|| {
            FrameworkError::param(format!(
                "image transformation `{}` is not registered; register it with \
                 suprnova::media::register_transformation before the first image uses it",
                self.name
            ))
        })?;
        function(pixels, settings)
    }
}

fn registry() -> &'static RwLock<HashMap<&'static str, Arc<CustomFunction>>> {
    static REGISTRY: OnceLock<RwLock<HashMap<&'static str, Arc<CustomFunction>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| RwLock::new(HashMap::new()))
}

fn registered(name: &str) -> Option<Arc<CustomFunction>> {
    let map = registry()
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    map.get(name).cloned()
}

fn register(name: &'static str, function: Arc<CustomFunction>) {
    let mut map = registry()
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    map.insert(name, function);
}

/// Register a custom transformation under `name`, for
/// [`Image::transform`](super::Image::transform) to use as
/// `Transformation::custom(name)`.
///
/// Laravel's `transformUsing`, without a driver: the function receives the
/// decoded pixels as they stand at its place in the pipeline and returns the
/// pixels the pipeline continues with, under either driver. Registering a
/// name again replaces its function, as Laravel's array of handlers does.
/// Call it during bootstrap; an image that names a transformation nothing is
/// registered under fails with an error naming it. A step recorded with
/// settings fails too: this function takes none. Use
/// [`register_transformation_with`] for a transformation that does.
pub fn register_transformation<F>(name: &'static str, transformation: F)
where
    F: Fn(ImagePixels) -> Result<ImagePixels, FrameworkError> + Send + Sync + 'static,
{
    register(
        name,
        Arc::new(
            move |pixels, settings: Option<&StepSettings>| match settings {
                None => transformation(pixels),
                Some(given) => Err(FrameworkError::param(format!(
                    "image transformation `{name}` takes no settings, but its step carries a \
                 `{}`; record it with Image::transform, or register it with \
                 register_transformation_with",
                    given.type_name
                ))),
            },
        ),
    );
}

/// Register a custom transformation under `name` whose function receives
/// per-call settings of type `S` beside the pixels, for
/// [`Image::transform_with`](super::Image::transform_with)`(name, settings)`.
///
/// Laravel's custom transformation is an object with its own fields, which
/// the handler receives; here the fields are `S`, so one registration
/// serves a pixelate of 4 on one image and of 8 on the next. The image, not
/// the step, holds the settings. A step whose settings are not an `S`, or
/// a step recorded without settings, fails the image with an error naming
/// the transformation. Registering a name again replaces its function. The
/// settings reach only this function; under the `magick` driver they never
/// become an ImageMagick argument.
pub fn register_transformation_with<S, F>(name: &'static str, transformation: F)
where
    S: Any + Send + Sync,
    F: Fn(ImagePixels, &S) -> Result<ImagePixels, FrameworkError> + Send + Sync + 'static,
{
    let expected = std::any::type_name::<S>();
    register(
        name,
        Arc::new(move |pixels, settings: Option<&StepSettings>| {
            let Some(given) = settings else {
                return Err(FrameworkError::param(format!(
                    "image transformation `{name}` takes settings of type `{expected}`; \
                     record it with Image::transform_with"
                )));
            };
            match given.value.downcast_ref::<S>() {
                Some(settings) => transformation(pixels, settings),
                None => Err(FrameworkError::param(format!(
                    "image transformation `{name}` takes settings of type `{expected}`, but \
                     its step carries a `{}`",
                    given.type_name
                ))),
            }
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn img_004_pixels_refuse_a_buffer_that_does_not_match_their_size() {
        assert!(ImagePixels::new(2, 2, vec![0; 16]).is_ok());
        assert!(ImagePixels::new(2, 2, vec![0; 15]).is_err());
        assert!(ImagePixels::new(0, 2, Vec::new()).is_err());
    }

    #[test]
    fn img_004_an_unregistered_transformation_fails_naming_it() {
        let err = CustomTransformation::new("img-004-never-registered")
            .apply(ImagePixels::new(1, 1, vec![0; 4]).unwrap())
            .expect_err("nothing is registered under this name");
        assert!(
            err.to_string().contains("img-004-never-registered"),
            "got: {err}"
        );
    }

    #[test]
    fn img_004_a_registered_transformation_receives_the_pixels() {
        register_transformation("img-004-invert", |mut pixels: ImagePixels| {
            for byte in pixels.pixels_mut() {
                *byte = !*byte;
            }
            Ok(pixels)
        });
        let out = CustomTransformation::new("img-004-invert")
            .apply(ImagePixels::new(1, 1, vec![0, 1, 2, 3]).unwrap())
            .unwrap();
        assert_eq!(out.pixels(), [255, 254, 253, 252]);
    }
}
