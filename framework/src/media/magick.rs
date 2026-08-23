//! The opt-in ImageMagick driver: `IMAGE_DRIVER=magick`.
//!
//! Laravel's image surface has always been two drivers - GD by default,
//! Imagick when the host provides it. This is the same shape. The framework
//! ships no codec here, links nothing, and compiles nothing native: it runs
//! a binary the host operator installed, and whatever formats that binary's
//! delegates support come along for free. HEIC is the motivating case.
//!
//! # Execution safety
//!
//! Arguments are **always** a fixed array handed straight to
//! [`std::process::Command`]. There is no shell, no `sh -c`, and no string
//! interpolation into a command line anywhere in this module. That matters
//! because the input to an image pipeline is attacker-controlled by
//! definition - it arrives as an upload. Every numeric argument is formatted
//! from an already-validated `u32`/`f32`/`u8` field of a
//! [`Transformation`], never from caller-supplied text, so there is no
//! argument position an attacker can reach. The image bytes themselves go
//! over stdin and come back over stdout; no temp file is written, so there
//! is no path to traverse and nothing to clean up.
//!
//! # Two-tier limits
//!
//! Decode limits are enforced twice, because this driver's whole purpose is
//! reading formats the framework cannot parse:
//!
//! 1. **Framework tier.** For the five formats
//!    [`sniff`](super::sniff) recognises, the header is parsed and the caps
//!    applied before the process is even spawned - identical to the
//!    pure-Rust driver.
//! 2. **ImageMagick tier.** For everything else (HEIC, AVIF, TIFF, PSD,
//!    whatever the host's delegates add) a pre-parse is impossible, so every
//!    invocation carries ImageMagick's own resource limits derived from the
//!    same [`ImageConfig`]. `-limit disk 0` is deliberate: without it, IM
//!    spills the pixel cache to disk and the memory cap stops being a cap.

use std::io::Write;
use std::process::{Command, Stdio};

use crate::config::env_optional;
use crate::error::FrameworkError;

use super::ImageConfig;
use super::driver::{ImageDriver, ImagePipeline, OutputFormat, Transformation};
use super::sniff;

/// Default binary name. ImageMagick 7 only.
///
/// Not `convert`: that is the ImageMagick 6 name, and IM6's argument
/// handling differs enough that silently accepting it would produce subtly
/// wrong output rather than an honest failure.
const DEFAULT_BINARY: &str = "magick";

/// An [`ImageDriver`] that shells out to a host-installed ImageMagick 7.
///
/// Selected with `IMAGE_DRIVER=magick`. The binary name comes from
/// `IMAGE_MAGICK_BINARY` and defaults to `magick`; an absent or
/// non-executing binary is an error at first use, naming the env var.
pub struct MagickCliDriver {
    binary: String,
}

impl MagickCliDriver {
    /// Build a driver around a specific binary name or path.
    pub fn new(binary: impl Into<String>) -> Self {
        Self {
            binary: binary.into(),
        }
    }

    /// Build a driver from `IMAGE_MAGICK_BINARY`, defaulting to `magick`.
    pub fn from_env() -> Self {
        Self::new(
            env_optional::<String>("IMAGE_MAGICK_BINARY")
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_BINARY.to_string()),
        )
    }

    /// The binary this driver invokes.
    pub fn binary(&self) -> &str {
        &self.binary
    }

    /// Apply the framework-tier limits to input we can parse ourselves.
    ///
    /// Returns the recognised format, or `None` when the bytes are something
    /// only ImageMagick can read - which is not an error here, unlike in the
    /// pure-Rust driver.
    fn guard(
        contents: &[u8],
        config: &ImageConfig,
    ) -> Result<Option<sniff::InputFormat>, FrameworkError> {
        sniff::guard(contents, config)
    }

    /// Spawn the binary, stream `input` in, and collect stdout.
    fn run(&self, args: &[String], input: &[u8]) -> Result<Vec<u8>, FrameworkError> {
        let mut child = Command::new(&self.binary)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    FrameworkError::internal(format!(
                        "image: the `{}` binary was not found. IMAGE_DRIVER=magick requires \
                         ImageMagick 7 installed on the host; install it, or set \
                         IMAGE_MAGICK_BINARY to its path, or use the default IMAGE_DRIVER=oxideav.",
                        self.binary
                    ))
                } else {
                    FrameworkError::internal(format!("image: could not run `{}`: {e}", self.binary))
                }
            })?;

        // Write stdin from a worker thread: writing the whole input before
        // reading stdout deadlocks as soon as the output outgrows the pipe
        // buffer, which for an image is immediately.
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| FrameworkError::internal("image: could not open the magick stdin"))?;
        let payload = input.to_vec();
        let writer = std::thread::spawn(move || {
            // A broken pipe here just means the child rejected the input and
            // exited early; its stderr is the useful diagnostic, not this.
            let _ = stdin.write_all(&payload);
        });

        let output = child.wait_with_output().map_err(|e| {
            FrameworkError::internal(format!("image: `{}` failed: {e}", self.binary))
        })?;
        let _ = writer.join();

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let detail = match sanitise_stderr(&stderr) {
                Some(detail) => detail,
                None => format!("exited with {}", output.status),
            };
            // A failure here is usually a bad or unsupported input, so this
            // is caller-facing (4xx) rather than an internal fault.
            return Err(FrameworkError::param(format!(
                "image processing failed in ImageMagick: {detail}"
            )));
        }
        if output.stdout.is_empty() {
            return Err(FrameworkError::param(
                "image processing failed in ImageMagick: it produced no output",
            ));
        }
        Ok(output.stdout)
    }
}

impl ImageDriver for MagickCliDriver {
    fn process(
        &self,
        contents: &[u8],
        pipeline: &ImagePipeline,
    ) -> Result<Vec<u8>, FrameworkError> {
        let config = super::config();
        let detected = Self::guard(contents, &config)?;
        let target = match pipeline.format.or_else(|| detected.map(same_format)) {
            Some(format) => format,
            None => {
                return Err(FrameworkError::param(
                    "image format is not one Suprnova can name, so the output format is \
                     ambiguous; call to_format() to choose one",
                ));
            }
        };
        self.run(&process_args(pipeline, &config, detected, target), contents)
    }

    fn dimensions(&self, contents: &[u8]) -> Result<(u32, u32), FrameworkError> {
        let config = super::config();
        let detected = Self::guard(contents, &config)?;
        let raw = self.run(&dimensions_args(&config, detected), contents)?;
        parse_dimensions(&String::from_utf8_lossy(&raw))
    }

    fn dominant_color(&self, contents: &[u8]) -> Result<String, FrameworkError> {
        let config = super::config();
        let detected = Self::guard(contents, &config)?;
        let raw = self.run(&dominant_color_args(&config, detected), contents)?;
        parse_hex_pixel(&String::from_utf8_lossy(&raw))
    }

    fn name(&self) -> &'static str {
        "magick"
    }
}

/// The `OutputFormat` that re-encodes a recognised input unchanged.
fn same_format(format: sniff::InputFormat) -> OutputFormat {
    match format {
        sniff::InputFormat::Png => OutputFormat::Png,
        sniff::InputFormat::Jpeg => OutputFormat::Jpeg,
        sniff::InputFormat::WebP => OutputFormat::WebP,
        sniff::InputFormat::Gif => OutputFormat::Gif,
        sniff::InputFormat::Bmp => OutputFormat::Bmp,
    }
}

/// ImageMagick's own resource caps, derived from [`ImageConfig`].
///
/// These are settings, so they precede the input on the command line.
fn limit_args(config: &ImageConfig) -> Vec<String> {
    let dimension = config.max_dimension.to_string();
    let bytes = config.max_alloc_bytes.to_string();
    let seconds = config.magick_timeout_secs.to_string();
    vec![
        // Wall-clock bound. Without it a delegate that stalls - a malformed
        // HEIC that sends libheif into a long loop, a network-backed coder -
        // holds a `spawn_blocking` worker for the life of the process. The
        // pipe plumbing alone cannot rescue that: the child is simply never
        // going to write, so nothing here would ever return.
        "-limit".into(),
        "time".into(),
        seconds,
        "-limit".into(),
        "width".into(),
        dimension.clone(),
        "-limit".into(),
        "height".into(),
        dimension,
        "-limit".into(),
        "area".into(),
        bytes.clone(),
        "-limit".into(),
        "memory".into(),
        bytes.clone(),
        "-limit".into(),
        "map".into(),
        bytes,
        // Without this, ImageMagick spills the pixel cache to disk when the
        // memory cap is hit and carries on - which would make every cap
        // above advisory rather than enforced.
        "-limit".into(),
        "disk".into(),
        "0".into(),
    ]
}

/// The IM7 operator sequence for one recorded transformation.
///
/// Geometry suffixes carry the semantics: `!` forces exact dimensions,
/// `>` shrinks only, `^` fills the box, and a bare `WxH` fits inside it.
fn transformation_args(step: Transformation) -> Vec<String> {
    let arg = |value: &str| value.to_string();
    match step {
        Transformation::Resize { width, height } => {
            vec![arg("-resize"), format!("{width}x{height}!")]
        }
        Transformation::ResizeWidth(width) => vec![arg("-resize"), format!("{width}x")],
        Transformation::ResizeHeight(height) => vec![arg("-resize"), format!("x{height}")],
        Transformation::Scale { width, height } => {
            vec![arg("-resize"), format!("{width}x{height}>")]
        }
        Transformation::ScaleWidth(width) => vec![arg("-resize"), format!("{width}x>")],
        Transformation::ScaleHeight(height) => vec![arg("-resize"), format!("x{height}>")],
        Transformation::Contain { width, height } => {
            vec![arg("-resize"), format!("{width}x{height}")]
        }
        Transformation::Cover { width, height } => vec![
            arg("-resize"),
            format!("{width}x{height}^"),
            arg("-gravity"),
            arg("center"),
            arg("-extent"),
            format!("{width}x{height}"),
            // Reset the virtual canvas the crop leaves behind, or the offset
            // is baked into the output file's geometry.
            arg("+repage"),
        ],
        Transformation::Crop {
            width,
            height,
            x,
            y,
        } => vec![
            arg("-crop"),
            format!("{width}x{height}+{x}+{y}"),
            arg("+repage"),
        ],
        Transformation::Rotate(degrees) => vec![
            arg("-background"),
            arg("none"),
            arg("-rotate"),
            format!("{degrees}"),
            arg("+repage"),
        ],
        Transformation::FlipVertically => vec![arg("-flip")],
        Transformation::FlipHorizontally => vec![arg("-flop")],
        Transformation::Blur(amount) => match blur_sigma(amount) {
            Some(sigma) => vec![arg("-blur"), format!("0x{sigma}")],
            None => Vec::new(),
        },
        Transformation::Sharpen(amount) => match sharpen_amount(amount) {
            Some(strength) => vec![arg("-unsharp"), format!("0x1+{strength}+0")],
            None => Vec::new(),
        },
        Transformation::Grayscale => vec![arg("-colorspace"), arg("Gray")],
    }
}

/// Blur strength `0..=100` to a Gaussian sigma.
///
/// Matches the pure-Rust driver's radius mapping (`amount/100 * 15`, rounded
/// up, sigma = radius/2) so switching drivers does not visibly change the
/// strength of the same pipeline.
fn blur_sigma(amount: u32) -> Option<f32> {
    let amount = amount.min(100);
    if amount == 0 {
        return None;
    }
    let radius = ((f64::from(amount) / 100.0) * 15.0).ceil().max(1.0);
    Some((radius / 2.0) as f32)
}

/// Sharpen strength `0..=100` to an unsharp amount, matching the pure-Rust
/// driver's scale where 50 is the classic `1.0`.
fn sharpen_amount(amount: u32) -> Option<f32> {
    let amount = amount.min(100);
    if amount == 0 {
        return None;
    }
    Some(amount as f32 / 50.0)
}

/// How stdin is named on the command line.
///
/// A bare `-` lets ImageMagick choose the decoder from the bytes it is handed,
/// which is the ImageTragick shape: a file whose magic says MVG or MSL is read
/// as a *script*, no matter what the application believed it was accepting,
/// with the host's `policy.xml` as the only thing standing in the way. When
/// the framework's own sniffer has already identified the format, name the
/// coder - `png:-` decodes as PNG or fails, and cannot be talked into
/// anything else.
///
/// The unrecognised path keeps the bare `-`, because reading formats the
/// framework cannot name is the entire reason this driver exists. That path
/// is the one an operator's `policy.xml` still has to cover.
fn input_spec(detected: Option<sniff::InputFormat>) -> String {
    match detected {
        Some(format) => format!("{}:-", format.magick_coder()),
        None => "-".to_string(),
    }
}

/// Full argv (after the binary) for a process run.
fn process_args(
    pipeline: &ImagePipeline,
    config: &ImageConfig,
    detected: Option<sniff::InputFormat>,
    target: OutputFormat,
) -> Vec<String> {
    let mut args = limit_args(config);
    args.push(input_spec(detected));
    for step in &pipeline.transformations {
        args.extend(transformation_args(*step));
    }
    args.push("-quality".into());
    args.push(pipeline.quality.to_string());
    // `format:-` forces the output encoder and writes to stdout.
    args.push(format!("{}:-", target.extension()));
    args
}

/// Full argv for a dimensions probe.
fn dimensions_args(config: &ImageConfig, detected: Option<sniff::InputFormat>) -> Vec<String> {
    let mut args = vec!["identify".to_string()];
    args.extend(limit_args(config));
    args.push("-format".into());
    args.push("%w %h".into());
    args.push(input_spec(detected));
    args
}

/// Full argv for an average-colour probe.
///
/// Alpha is switched off *before* the downscale so a transparent image's
/// colour is not blended toward the background, matching the pure-Rust
/// driver and Laravel, both of which drop alpha rather than weighting by it.
fn dominant_color_args(config: &ImageConfig, detected: Option<sniff::InputFormat>) -> Vec<String> {
    let mut args = limit_args(config);
    args.push(input_spec(detected));
    args.push("-alpha".into());
    args.push("off".into());
    args.push("-resize".into());
    args.push("1x1!".into());
    args.push("-depth".into());
    args.push("8".into());
    // The `txt:` encoder's pixel enumeration is stable across IM versions
    // and always includes a `#RRGGBB` token.
    args.push("txt:-".into());
    args
}

/// Reduce ImageMagick's stderr to the part a caller should see.
///
/// IM appends its own build detail to every message - the source file and line
/// that raised it (`... @ error/constitute.c/ReadImage/741`), and for a policy
/// rejection the path of the `policy.xml` that did it. That is host
/// configuration leaking into a 4xx body, so keep the first line and cut at
/// the `@ error/` marker.
fn sanitise_stderr(stderr: &str) -> Option<String> {
    let first = stderr.lines().find(|line| !line.trim().is_empty())?;
    let trimmed = first.split(" @ error/").next().unwrap_or(first).trim();
    let trimmed = trimmed
        .split(" @ warning/")
        .next()
        .unwrap_or(trimmed)
        .trim();
    if trimmed.is_empty() {
        return None;
    }
    // A defensive ceiling: a coder is free to emit a very long single line.
    const MAX: usize = 200;
    if trimmed.len() <= MAX {
        return Some(trimmed.to_string());
    }
    let mut cut = MAX;
    while cut > 0 && !trimmed.is_char_boundary(cut) {
        cut -= 1;
    }
    Some(format!("{}...", &trimmed[..cut]))
}

fn parse_dimensions(raw: &str) -> Result<(u32, u32), FrameworkError> {
    let malformed = || {
        FrameworkError::internal(format!(
            "image: could not read dimensions from ImageMagick output {raw:?}"
        ))
    };
    let mut parts = raw.split_whitespace();
    let width = parts.next().ok_or_else(malformed)?;
    let height = parts.next().ok_or_else(malformed)?;
    Ok((
        width.parse().map_err(|_| malformed())?,
        height.parse().map_err(|_| malformed())?,
    ))
}

/// Pull the `#RRGGBB` token out of a `txt:` pixel enumeration.
///
/// At depth 8 an opaque pixel renders as `#RRGGBB` and one with an alpha
/// channel as `#RRGGBBAA`; taking the first six digits drops alpha either
/// way.
fn parse_hex_pixel(raw: &str) -> Result<String, FrameworkError> {
    for token in raw.split_whitespace() {
        let Some(hex) = token.strip_prefix('#') else {
            continue;
        };
        if hex.len() >= 6 && hex.as_bytes()[..6].iter().all(u8::is_ascii_hexdigit) {
            return Ok(format!("#{}", hex[..6].to_ascii_lowercase()));
        }
    }
    Err(FrameworkError::internal(format!(
        "image: could not read a pixel colour from ImageMagick output {raw:?}"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> ImageConfig {
        ImageConfig {
            max_dimension: 4096,
            max_alloc_bytes: 1024,
            magick_timeout_secs: 30,
        }
    }

    fn limits() -> Vec<String> {
        vec![
            "-limit", "time", "30", "-limit", "width", "4096", "-limit", "height", "4096",
            "-limit", "area", "1024", "-limit", "memory", "1024", "-limit", "map", "1024",
            "-limit", "disk", "0",
        ]
        .into_iter()
        .map(String::from)
        .collect()
    }

    #[test]
    fn limits_are_derived_from_the_image_config() {
        assert_eq!(limit_args(&config()), limits());
    }

    #[test]
    fn the_pixel_cache_cannot_escape_to_disk() {
        let args = limit_args(&config());
        let disk = args.iter().position(|a| a == "disk").expect("disk limit");
        assert_eq!(
            args[disk + 1],
            "0",
            "a non-zero disk limit lets IM spill past the memory cap"
        );
    }

    #[test]
    fn a_full_pipeline_builds_the_exact_expected_argv() {
        let pipeline = ImagePipeline {
            transformations: vec![
                Transformation::Resize {
                    width: 800,
                    height: 600,
                },
                Transformation::Grayscale,
            ],
            format: Some(OutputFormat::WebP),
            quality: 65,
        };
        let mut expected = limits();
        expected.extend(
            [
                // The input coder is pinned, not sniffed by ImageMagick.
                "png:-",
                "-resize",
                "800x600!",
                "-colorspace",
                "Gray",
                "-quality",
                "65",
                "webp:-",
            ]
            .into_iter()
            .map(String::from),
        );
        assert_eq!(
            process_args(
                &pipeline,
                &config(),
                Some(sniff::InputFormat::Png),
                OutputFormat::WebP
            ),
            expected
        );
    }

    #[test]
    fn geometry_suffixes_carry_the_resize_semantics() {
        // `!` forces exact dimensions, ignoring aspect ratio.
        assert_eq!(
            transformation_args(Transformation::Resize {
                width: 10,
                height: 20
            }),
            vec!["-resize", "10x20!"]
        );
        // `>` shrinks only - this is what makes `scale` never enlarge.
        assert_eq!(
            transformation_args(Transformation::Scale {
                width: 10,
                height: 20
            }),
            vec!["-resize", "10x20>"]
        );
        assert_eq!(
            transformation_args(Transformation::ScaleWidth(10)),
            vec!["-resize", "10x>"]
        );
        assert_eq!(
            transformation_args(Transformation::ScaleHeight(20)),
            vec!["-resize", "x20>"]
        );
        // A bare geometry fits inside the box, preserving aspect ratio.
        assert_eq!(
            transformation_args(Transformation::Contain {
                width: 10,
                height: 20
            }),
            vec!["-resize", "10x20"]
        );
        // A single dimension lets IM derive the other.
        assert_eq!(
            transformation_args(Transformation::ResizeWidth(10)),
            vec!["-resize", "10x"]
        );
        assert_eq!(
            transformation_args(Transformation::ResizeHeight(20)),
            vec!["-resize", "x20"]
        );
    }

    #[test]
    fn cover_fills_then_crops_from_the_centre() {
        assert_eq!(
            transformation_args(Transformation::Cover {
                width: 64,
                height: 64
            }),
            vec![
                "-resize", "64x64^", "-gravity", "center", "-extent", "64x64", "+repage"
            ]
        );
    }

    #[test]
    fn crop_and_rotate_reset_the_virtual_canvas() {
        let crop = transformation_args(Transformation::Crop {
            width: 4,
            height: 3,
            x: 2,
            y: 1,
        });
        assert_eq!(crop, vec!["-crop", "4x3+2+1", "+repage"]);

        let rotate = transformation_args(Transformation::Rotate(45.0));
        assert_eq!(
            rotate,
            vec!["-background", "none", "-rotate", "45", "+repage"],
            "a transparent background keeps rotation from painting corners black"
        );
    }

    #[test]
    fn flips_map_to_their_imagemagick_names() {
        assert_eq!(
            transformation_args(Transformation::FlipVertically),
            vec!["-flip"]
        );
        assert_eq!(
            transformation_args(Transformation::FlipHorizontally),
            vec!["-flop"]
        );
    }

    #[test]
    fn zero_strength_blur_and_sharpen_emit_no_arguments() {
        assert!(transformation_args(Transformation::Blur(0)).is_empty());
        assert!(transformation_args(Transformation::Sharpen(0)).is_empty());
    }

    #[test]
    fn blur_and_sharpen_match_the_pure_rust_driver_scale() {
        assert_eq!(blur_sigma(0), None);
        assert_eq!(blur_sigma(100), Some(7.5));
        assert_eq!(sharpen_amount(50), Some(1.0));
        assert_eq!(sharpen_amount(100), Some(2.0));
        assert_eq!(
            transformation_args(Transformation::Blur(100)),
            vec!["-blur", "0x7.5"]
        );
        assert_eq!(
            transformation_args(Transformation::Sharpen(50)),
            vec!["-unsharp", "0x1+1+0"]
        );
    }

    #[test]
    fn every_argument_is_its_own_array_element() {
        // The whole safety argument rests on this: nothing that reaches
        // Command::args may contain a shell metacharacter boundary, because
        // each element is passed as one argv entry and never parsed.
        let pipeline = ImagePipeline {
            transformations: vec![
                Transformation::Crop {
                    width: 1,
                    height: 2,
                    x: 3,
                    y: 4,
                },
                Transformation::Rotate(-33.5),
            ],
            format: Some(OutputFormat::Jpeg),
            quality: 70,
        };
        for arg in process_args(
            &pipeline,
            &config(),
            Some(sniff::InputFormat::Jpeg),
            OutputFormat::Jpeg,
        ) {
            assert!(
                !arg.contains(';')
                    && !arg.contains('|')
                    && !arg.contains('&')
                    && !arg.contains(' '),
                "argument {arg:?} would be ambiguous if it ever reached a shell"
            );
        }
    }

    #[test]
    fn dimensions_probe_uses_the_identify_subcommand() {
        let args = dimensions_args(&config(), Some(sniff::InputFormat::Gif));
        assert_eq!(args[0], "identify");
        assert_eq!(args[args.len() - 3..], ["-format", "%w %h", "gif:-"]);
        assert!(args.contains(&"-limit".to_string()));
    }

    #[test]
    fn dominant_color_probe_drops_alpha_before_downscaling() {
        let args = dominant_color_args(&config(), Some(sniff::InputFormat::Bmp));
        let alpha = args.iter().position(|a| a == "-alpha").expect("-alpha");
        let resize = args.iter().position(|a| a == "-resize").expect("-resize");
        assert!(
            alpha < resize,
            "alpha must be off before the 1x1 downscale or it weights the average"
        );
        assert_eq!(args[args.len() - 1], "txt:-");
    }

    #[test]
    fn dimensions_parse_from_identify_output() {
        assert_eq!(parse_dimensions("640 480").expect("dims"), (640, 480));
        assert_eq!(parse_dimensions(" 12 34 \n").expect("dims"), (12, 34));
        assert!(parse_dimensions("").is_err());
        assert!(parse_dimensions("640").is_err());
        assert!(parse_dimensions("wide tall").is_err());
    }

    #[test]
    fn hex_pixel_parses_from_a_txt_enumeration() {
        let opaque = "# ImageMagick pixel enumeration: 1,1,255,srgb\n\
                      0,0: (255,0,0)  #FF0000  srgb(255,0,0)\n";
        assert_eq!(parse_hex_pixel(opaque).expect("colour"), "#ff0000");

        // With an alpha channel IM writes eight digits; alpha is dropped.
        let with_alpha = "0,0: (18,52,86,128)  #12345680  srgba(18,52,86,0.5)\n";
        assert_eq!(parse_hex_pixel(with_alpha).expect("colour"), "#123456");

        assert!(parse_hex_pixel("no pixels here").is_err());
        assert!(parse_hex_pixel("#xyz").is_err());
    }

    #[test]
    fn a_missing_binary_names_the_env_var_and_the_alternative() {
        let driver = MagickCliDriver::new("suprnova-no-such-binary-exists");
        let err = driver
            .run(&["-".to_string()], b"irrelevant")
            .expect_err("binary is absent");
        let message = err.to_string();
        assert!(message.contains("IMAGE_MAGICK_BINARY"), "got: {message}");
        assert!(message.contains("oxideav"), "got: {message}");
    }

    #[test]
    fn a_recognised_input_pins_the_decoder_instead_of_letting_im_choose() {
        // The ImageTragick shape: a bare `-` lets ImageMagick pick the coder
        // from the bytes, so a file whose magic says MVG or MSL is executed as
        // a script no matter what the app thought it accepted. Naming the
        // coder makes the decode fail instead of becoming something else.
        for (format, expected) in [
            (sniff::InputFormat::Png, "png:-"),
            (sniff::InputFormat::Jpeg, "jpeg:-"),
            (sniff::InputFormat::WebP, "webp:-"),
            (sniff::InputFormat::Gif, "gif:-"),
            (sniff::InputFormat::Bmp, "bmp:-"),
        ] {
            assert_eq!(input_spec(Some(format)), expected);
        }
    }

    #[test]
    fn an_unrecognised_input_keeps_the_bare_stdin_marker() {
        // Reading formats the framework cannot name is this driver's entire
        // purpose, so that path cannot pin a coder; it is the one the host's
        // policy.xml still has to cover.
        assert_eq!(input_spec(None), "-");
    }

    #[test]
    fn every_probe_pins_the_coder_when_the_format_is_known() {
        let dims = dimensions_args(&config(), Some(sniff::InputFormat::Png));
        assert_eq!(dims[dims.len() - 1], "png:-");
        let colour = dominant_color_args(&config(), Some(sniff::InputFormat::Jpeg));
        assert!(colour.contains(&"jpeg:-".to_string()));
    }

    #[test]
    fn stderr_is_reduced_to_the_part_a_caller_should_see() {
        // IM appends the source file and line that raised the error, and for a
        // policy rejection the path of the policy.xml. Neither belongs in a
        // 4xx body.
        let coder = "magick: no decode delegate for this image format `HEIC' \
                     @ error/constitute.c/ReadImage/741";
        assert_eq!(
            sanitise_stderr(coder).expect("a message"),
            "magick: no decode delegate for this image format `HEIC'"
        );

        let policy = "magick: attempt to perform an operation not allowed by the security \
                      policy `MVG' @ error/policy.c/IsRightsAuthorized/574";
        let cleaned = sanitise_stderr(policy).expect("a message");
        assert!(!cleaned.contains("policy.c"), "got: {cleaned}");
        assert!(!cleaned.contains('@'), "got: {cleaned}");

        // Only the first line survives a multi-line spew.
        let multi = "first problem @ error/a.c/B/1\nsecond problem @ error/c.c/D/2\n";
        assert_eq!(sanitise_stderr(multi).expect("a message"), "first problem");

        // Blank stderr yields nothing, so the caller falls back to the status.
        assert_eq!(sanitise_stderr("   \n  \n"), None);
        assert_eq!(sanitise_stderr(""), None);

        // A pathological single line is truncated on a char boundary.
        let long = format!("{} @ error/x.c/Y/1", "é".repeat(400));
        let cut = sanitise_stderr(&long).expect("a message");
        assert!(cut.len() <= 204, "length {}", cut.len());
        assert!(cut.ends_with("..."));
    }

    #[test]
    fn the_invocation_is_bounded_in_wall_clock_time() {
        // Without this a stalled delegate holds a blocking worker forever.
        let args = limit_args(&config());
        let time = args.iter().position(|a| a == "time").expect("time limit");
        assert_eq!(args[time - 1], "-limit");
        assert_eq!(args[time + 1], "30");
    }

    #[test]
    fn the_binary_name_comes_from_the_environment_with_a_v7_default() {
        assert_eq!(MagickCliDriver::new("magick").binary(), "magick");
        assert_eq!(DEFAULT_BINARY, "magick", "IM6's `convert` is not accepted");
    }
}
