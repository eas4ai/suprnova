#![cfg(all(feature = "media", unix))]
//! HRD-002: the `magick` driver names the coder it hands ImageMagick.
//!
//! None of these tests runs ImageMagick. The driver's binary is a shell
//! script that records each invocation, its arguments and the bytes on its
//! stdin, and answers each probe the way ImageMagick would. So a test sees
//! whether a process started at all, and what it was told to read.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use serial_test::serial;
use suprnova::{
    ImageConfig, ImageDriver, ImagePipeline, MagickCliDriver, OutputFormat, OxideAvImageDriver,
};

use crate::image_processing::{ConfigGuard, RED_PNG_1X1};

/// An MVG document: ImageMagick reads it as drawing commands, and this one
/// asks for a URL.
const MVG: &[u8] = include_bytes!("fixtures/magick-script.mvg");

/// An MSL document: ImageMagick's scripting language reads and writes images
/// by name.
const MSL: &[u8] = include_bytes!("fixtures/magick-script.msl");

/// Input that names ImageMagick's `url:` pseudo-coder.
const URL_PSEUDO_CODER: &[u8] = include_bytes!("fixtures/magick-url-pseudo-coder.txt");

/// A 1x1 binary PPM: a real raster format, and one the sniffer does not
/// name, which is the input the autodetection opt-in exists for.
const PPM_1X1: &[u8] = b"P6\n1 1\n255\n\xff\x00\x00";

/// The message every refusal of unnamed bytes carries.
const NOT_NAMED: &str = "not one Suprnova names";

/// A stand-in `magick` binary that records every invocation.
struct Recorder {
    binary: PathBuf,
    argv: PathBuf,
    stdin: PathBuf,
}

impl Recorder {
    /// Write the stand-in into a scratch directory of its own.
    fn new(name: &str) -> Self {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("hardening-magick-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create the scratch directory");
        let argv = dir.join("argv.log");
        let stdin = dir.join("stdin.log");
        let png = dir.join("answer.png");
        std::fs::write(&png, RED_PNG_1X1).expect("write the PNG the stand-in answers with");
        let binary = dir.join("magick-recording");
        // The last argument says what ImageMagick would write: a pixel
        // enumeration for the colour probe, a PNG for a pipeline, and a
        // size for `identify`.
        std::fs::write(
            &binary,
            format!(
                "#!/bin/sh\n\
                 printf '%s\\n' \"$@\" >> '{argv}'\n\
                 printf -- '--end--\\n' >> '{argv}'\n\
                 cat >> '{stdin}'\n\
                 for last; do :; done\n\
                 case \"$last\" in\n\
                 txt:-) printf '0,0: (255,0,0)  #FF0000  srgb(255,0,0)\\n' ;;\n\
                 png:-) cat '{png}' ;;\n\
                 *) printf '1 1' ;;\n\
                 esac\n",
                argv = argv.display(),
                stdin = stdin.display(),
                png = png.display(),
            ),
        )
        .expect("write the stand-in binary");
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755))
            .expect("make the stand-in executable");
        Self {
            binary,
            argv,
            stdin,
        }
    }

    fn driver(&self) -> MagickCliDriver {
        MagickCliDriver::new(self.binary.to_string_lossy())
    }

    /// The arguments of every process started so far, one list a process.
    fn runs(&self) -> Vec<Vec<String>> {
        let Ok(log) = std::fs::read_to_string(&self.argv) else {
            return Vec::new();
        };
        log.split("--end--\n")
            .filter(|run| !run.is_empty())
            .map(|run| run.lines().map(str::to_string).collect())
            .collect()
    }

    /// Every byte the processes started so far read on stdin.
    fn stdin(&self) -> Vec<u8> {
        std::fs::read(&self.stdin).unwrap_or_default()
    }

    /// Forget the invocations so far.
    fn clear(&self) {
        let _ = std::fs::remove_file(&self.argv);
        let _ = std::fs::remove_file(&self.stdin);
    }
}

fn to_png() -> ImagePipeline {
    ImagePipeline {
        format: Some(OutputFormat::Png),
        ..ImagePipeline::default()
    }
}

/// Assert that each entry point of the driver refuses `bytes` as unnamed.
fn assert_refused(driver: &MagickCliDriver, what: &str, bytes: &[u8]) {
    let refusals = [
        ("process", driver.process(bytes, &to_png()).map(|_| ())),
        ("dimensions", driver.dimensions(bytes).map(|_| ())),
        ("dominant_color", driver.dominant_color(bytes).map(|_| ())),
    ];
    for (entry, result) in refusals {
        let err = result.expect_err(&format!("{entry} must refuse {what}"));
        assert!(
            err.to_string().contains(NOT_NAMED),
            "{entry} refused {what} for another reason: {err}"
        );
    }
}

/// An argument that hands ImageMagick bare stdin: `-`, or `-` with a frame
/// selector.
fn is_bare_stdin(arg: &str) -> bool {
    arg == "-" || arg.starts_with("-[")
}

#[test]
#[serial]
fn hardening_unrecognised_bytes_never_reach_imagemagick() {
    let _config = ConfigGuard::set(ImageConfig::default());
    let recorder = Recorder::new("unrecognised");
    let driver = recorder.driver();
    for (what, bytes) in [
        ("an MVG document", MVG),
        ("an MSL document", MSL),
        ("a url: pseudo-coder input", URL_PSEUDO_CODER),
        ("a PPM, which the sniffer does not name", PPM_1X1),
    ] {
        assert_refused(&driver, what, bytes);
        assert!(
            recorder.runs().is_empty(),
            "a process started for {what}: {:?}",
            recorder.runs()
        );
        assert!(recorder.stdin().is_empty(), "{what} reached a process");
    }
}

#[test]
#[serial]
fn hardening_recognised_formats_are_coder_qualified() {
    let _config = ConfigGuard::set(ImageConfig::default());
    let recorder = Recorder::new("recognised");
    let driver = recorder.driver();
    let encode = |format| {
        OxideAvImageDriver::new()
            .process(
                RED_PNG_1X1,
                &ImagePipeline {
                    format: Some(format),
                    ..ImagePipeline::default()
                },
            )
            .expect("the built-in driver writes the fixture")
    };
    let cases: Vec<(&str, Vec<u8>, &str)> = vec![
        ("PNG", RED_PNG_1X1.to_vec(), "png"),
        ("JPEG", encode(OutputFormat::Jpeg), "jpeg"),
        ("WebP", encode(OutputFormat::WebP), "webp"),
        ("GIF", encode(OutputFormat::Gif), "gif"),
        ("BMP", encode(OutputFormat::Bmp), "bmp"),
        (
            "HEIC",
            b"\x00\x00\x00\x18ftypheic\x00\x00\x00\x00mif1heic".to_vec(),
            "heic",
        ),
        (
            "HEIF",
            b"\x00\x00\x00\x18ftypmif1\x00\x00\x00\x00mif1heic".to_vec(),
            "heic",
        ),
        (
            "AVIF",
            b"\x00\x00\x00\x1cftypavif\x00\x00\x00\x00avifmif1miaf".to_vec(),
            "avif",
        ),
        (
            "little-endian TIFF",
            b"II*\x00\x08\x00\x00\x00".to_vec(),
            "tiff",
        ),
        (
            "big-endian TIFF",
            b"MM\x00*\x00\x00\x00\x08".to_vec(),
            "tiff",
        ),
        (
            "BigTIFF",
            b"II+\x00\x08\x00\x00\x00\x10\x00\x00\x00\x00\x00\x00\x00".to_vec(),
            "tiff",
        ),
        (
            "PSD",
            b"8BPS\x00\x01\x00\x00\x00\x00\x00\x00".to_vec(),
            "psd",
        ),
        (
            "PSB",
            b"8BPS\x00\x02\x00\x00\x00\x00\x00\x00".to_vec(),
            "psb",
        ),
    ];
    for (what, bytes, coder) in cases {
        recorder.clear();
        driver
            .dimensions(&bytes)
            .unwrap_or_else(|e| panic!("{what}: dimensions: {e}"));
        driver
            .dominant_color(&bytes)
            .unwrap_or_else(|e| panic!("{what}: dominant_color: {e}"));
        driver
            .process(&bytes, &to_png())
            .unwrap_or_else(|e| panic!("{what}: process: {e}"));
        let runs = recorder.runs();
        assert!(runs.len() >= 3, "{what}: every entry point ran: {runs:?}");
        let input = format!("{coder}:-[0]");
        for run in &runs {
            assert!(
                run.contains(&input),
                "{what} is read as `{input}` in every run: {run:?}"
            );
            assert!(
                !run.iter().any(|arg| is_bare_stdin(arg)),
                "{what} reached ImageMagick on bare stdin: {run:?}"
            );
        }
        assert_eq!(
            recorder.stdin(),
            bytes.repeat(runs.len()),
            "{what}: every run read the input"
        );
    }
}

#[test]
#[serial]
fn hardening_autodetection_is_an_explicit_opt_in() {
    let recorder = Recorder::new("opt-in");
    let driver = recorder.driver();

    // Off by default.
    assert!(!ImageConfig::default().magick_autodetect);
    {
        let _config = ConfigGuard::set(ImageConfig::default());
        assert_refused(&driver, "a PPM", PPM_1X1);
        assert!(recorder.runs().is_empty(), "{:?}", recorder.runs());
    }

    // Opted into by name, the bytes reach ImageMagick on bare stdin.
    {
        let _config = ConfigGuard::set(ImageConfig::default().magick_autodetect(true));
        assert_eq!(driver.dimensions(PPM_1X1).expect("dimensions"), (1, 1));
        driver
            .process(PPM_1X1, &to_png())
            .expect("the pipeline runs");
        let runs = recorder.runs();
        assert!(runs.len() >= 2, "{runs:?}");
        for run in &runs {
            assert!(
                run.iter().any(|arg| arg == "-[0]"),
                "the opt-in hands ImageMagick bare stdin: {run:?}"
            );
        }
        assert_eq!(recorder.stdin(), PPM_1X1.repeat(runs.len()));

        // A named format keeps its coder under the opt-in.
        recorder.clear();
        driver.dimensions(RED_PNG_1X1).expect("dimensions");
        for run in recorder.runs() {
            assert!(run.contains(&"png:-[0]".to_string()), "{run:?}");
        }
    }

    // `IMAGE_MAGICK_AUTODETECT=true` opts in; unset, the bytes are refused.
    for (value, expect) in [(Some("true"), "bare"), (None, "refused")] {
        let output = {
            let _env = crate::env_lock::lock_env();
            let mut child = crate::own_process::child_command(
                "magick_coder::hardening_autodetection_is_an_explicit_opt_in_child",
            );
            match value {
                Some(value) => child.env("IMAGE_MAGICK_AUTODETECT", value),
                None => child.env_remove("IMAGE_MAGICK_AUTODETECT"),
            };
            child
                .env("HARDENING_MAGICK_EXPECT", expect)
                .output()
                .expect("the child runs")
        };
        crate::own_process::assert_child_passed(&output);
    }
}

/// Runs only in the child process the opt-in test starts: it reads the
/// image configuration from the environment it was started with.
#[test]
fn hardening_autodetection_is_an_explicit_opt_in_child() {
    if !crate::own_process::is_child() {
        return;
    }
    let expect = std::env::var("HARDENING_MAGICK_EXPECT").expect("the parent says what to expect");
    let recorder = Recorder::new(&format!("env-{expect}"));
    let driver = recorder.driver();
    match expect.as_str() {
        "bare" => {
            assert!(suprnova::media::config().magick_autodetect);
            assert_eq!(driver.dimensions(PPM_1X1).expect("dimensions"), (1, 1));
            let runs = recorder.runs();
            assert!(!runs.is_empty());
            for run in &runs {
                assert!(run.iter().any(|arg| arg == "-[0]"), "{run:?}");
            }
        }
        "refused" => {
            assert!(!suprnova::media::config().magick_autodetect);
            assert_refused(&driver, "a PPM", PPM_1X1);
            assert!(recorder.runs().is_empty(), "{:?}", recorder.runs());
        }
        other => panic!("unknown expectation {other}"),
    }
}
