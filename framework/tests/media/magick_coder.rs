#![cfg(all(feature = "media", unix))]
//! HRD-002: the `magick` driver names the coder it hands ImageMagick.
//! HRD-010: every run reads the framework's ImageMagick policy, read last,
//! and the policy in effect is checked before the first run. A failed check
//! refuses every run, with or without autodetection.
//!
//! None of these tests runs ImageMagick. The driver's binary is a shell
//! script that records each invocation, its arguments, the bytes on its
//! stdin, its `HOME` and `MAGICK_CONFIGURE_PATH`, and the `policy.xml` in
//! the first directory of that path, and answers each probe the way
//! ImageMagick would. It answers `-list policy` with a listing the test
//! chose, with the framework's file under its own `HOME`, and records that
//! apart from the runs on an image. So a test sees whether a process started
//! at all, what it was told to read, which policy it would have read, and
//! how often the driver asked for the policy in effect.

use std::os::unix::fs::{MetadataExt, PermissionsExt};
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
    configure: PathBuf,
    policy: PathBuf,
    /// The `HOME` of every process, the checks of the policy in effect
    /// included.
    home: PathBuf,
    /// The scratch directory, which holds the answers to the driver's
    /// checks of the policy in effect.
    dir: PathBuf,
    /// The `MAGICK_CONFIGURE_PATH` of each `-list policy` call.
    listings: PathBuf,
    /// While this file exists, `-list policy` fails.
    listing_fails: PathBuf,
    /// The `MAGICK_CONFIGURE_PATH` of each MVG probe.
    probes: PathBuf,
}

/// What the stand-in records for a process started without
/// `MAGICK_CONFIGURE_PATH`, or without `HOME`.
const UNSET: &str = "(unset)";

/// What the stand-in records for a process whose first configure
/// directory holds no `policy.xml`.
const MISSING: &str = "(missing)";

/// The coders HRD-010 names, each of which the framework's policy denies.
const DENIED_CODERS: [&str; 19] = [
    "URL",
    "HTTP",
    "HTTPS",
    "FTP",
    "MVG",
    "MSL",
    "TEXT",
    "LABEL",
    "CAPTION",
    "PANGO",
    "SCREENSHOT",
    "X",
    "EPHEMERAL",
    "PS",
    "PS2",
    "PS3",
    "EPS",
    "PDF",
    "XPS",
];

impl Recorder {
    /// Write the stand-in into a scratch directory of its own.
    fn new(name: &str) -> Self {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("hardening-magick-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create the scratch directory");
        let argv = dir.join("argv.log");
        let stdin = dir.join("stdin.log");
        let configure = dir.join("configure.log");
        let policy = dir.join("policy.log");
        let home = dir.join("home.log");
        let listings = dir.join("listings.log");
        let listing_fails = dir.join("listing-fails");
        let probes = dir.join("probes.log");
        let answer = crate::magick_policy::answer_policy_checks(&dir);
        let png = dir.join("answer.png");
        std::fs::write(&png, RED_PNG_1X1).expect("write the PNG the stand-in answers with");
        let binary = dir.join("magick-recording");
        // Every process logs its `HOME` first, the checks of the policy in
        // effect included. `-list policy` and the MVG probe (`mvg:-`) are
        // those checks: each is logged apart from the runs on an image and
        // answered from the files the test chose, and the listing fails
        // while the flag file exists. For a run, the policy is the one
        // ImageMagick would read first: `policy.xml` in the first directory
        // of the configure path, which ends at the first `:`. The last
        // argument says what ImageMagick would write: a pixel enumeration
        // for the colour probe, a PNG for a pipeline, and a size for
        // `identify`.
        std::fs::write(
            &binary,
            format!(
                "#!/bin/sh\n\
                 if [ \"${{HOME+set}}\" = set ]; then\n\
                 printf '%s\\n' \"$HOME\" >> '{home}'\n\
                 else\n\
                 printf '{UNSET}\\n' >> '{home}'\n\
                 fi\n\
                 if [ \"$1\" = -list ] && [ \"$2\" = policy ]; then\n\
                 printf '%s\\n' \"$MAGICK_CONFIGURE_PATH\" >> '{listings}'\n\
                 if [ -e '{listing_fails}' ]; then\n\
                 printf 'magick: unable to list the policy\\n' >&2\n\
                 exit 1\n\
                 fi\n\
                 fi\n\
                 for arg; do\n\
                 if [ \"$arg\" = mvg:- ]; then\n\
                 printf '%s\\n' \"$MAGICK_CONFIGURE_PATH\" >> '{probes}'\n\
                 if [ -e '{probe_hangs}' ]; then sleep 30; fi\n\
                 fi\n\
                 done\n\
                 {answer}\
                 printf '%s\\n' \"$@\" >> '{argv}'\n\
                 printf -- '--end--\\n' >> '{argv}'\n\
                 if [ \"${{MAGICK_CONFIGURE_PATH+set}}\" = set ]; then\n\
                 printf '%s\\n' \"$MAGICK_CONFIGURE_PATH\" >> '{configure}'\n\
                 else\n\
                 printf '{UNSET}\\n' >> '{configure}'\n\
                 fi\n\
                 first=\"${{MAGICK_CONFIGURE_PATH%%:*}}\"\n\
                 if [ -n \"$first\" ] && [ -f \"$first/policy.xml\" ]; then\n\
                 cat \"$first/policy.xml\" >> '{policy}'\n\
                 else\n\
                 printf '{MISSING}' >> '{policy}'\n\
                 fi\n\
                 printf -- '--end--\\n' >> '{policy}'\n\
                 cat >> '{stdin}'\n\
                 for last; do :; done\n\
                 case \"$last\" in\n\
                 txt:-) printf '0,0: (255,0,0)  #FF0000  srgb(255,0,0)\\n' ;;\n\
                 png:-) cat '{png}' ;;\n\
                 *) printf '1 1' ;;\n\
                 esac\n",
                argv = argv.display(),
                stdin = stdin.display(),
                configure = configure.display(),
                policy = policy.display(),
                home = home.display(),
                listings = listings.display(),
                listing_fails = listing_fails.display(),
                probes = probes.display(),
                probe_hangs = dir.join("probe-hangs").display(),
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
            configure,
            policy,
            home,
            dir,
            listings,
            listing_fails,
            probes,
        }
    }

    /// Answer `-list policy` with `listing` from now on. The stand-in puts
    /// its own `HOME` in place of [`crate::magick_policy::POLICY_HOME`].
    fn answer_listing(&self, listing: &str) {
        std::fs::write(self.dir.join(crate::magick_policy::LISTING), listing)
            .expect("write the listing");
    }

    /// Answer the MVG probe from now on by writing `stdout` and `stderr`
    /// and exiting with `exit`.
    fn answer_probe(&self, stdout: &str, stderr: &str, exit: u8) {
        for (file, bytes) in [
            (crate::magick_policy::PROBE_STDOUT, stdout.to_string()),
            (crate::magick_policy::PROBE_STDERR, stderr.to_string()),
            (crate::magick_policy::PROBE_EXIT, exit.to_string()),
        ] {
            std::fs::write(self.dir.join(file), bytes).expect("write the probe's answer");
        }
    }

    /// Make the MVG probe hang from now on, past any deadline a test sets.
    fn hang_probe(&self) {
        std::fs::write(self.dir.join("probe-hangs"), b"").expect("write the flag");
    }

    /// The `MAGICK_CONFIGURE_PATH` of each MVG probe so far.
    fn probes(&self) -> Vec<String> {
        std::fs::read_to_string(&self.probes)
            .map(|log| log.lines().map(str::to_string).collect())
            .unwrap_or_default()
    }

    /// Make `-list policy` fail from now on.
    fn fail_listing(&self) {
        std::fs::write(&self.listing_fails, b"").expect("write the flag");
    }

    /// The `MAGICK_CONFIGURE_PATH` of each `-list policy` call so far.
    fn listings(&self) -> Vec<String> {
        std::fs::read_to_string(&self.listings)
            .map(|log| log.lines().map(str::to_string).collect())
            .unwrap_or_default()
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

    /// The lines of a log of an environment variable, one a process:
    /// `None` where the process started without it.
    fn variable_log(path: &Path) -> Vec<Option<String>> {
        let Ok(log) = std::fs::read_to_string(path) else {
            return Vec::new();
        };
        log.lines()
            .map(|line| (line != UNSET).then(|| line.to_string()))
            .collect()
    }

    /// The `MAGICK_CONFIGURE_PATH` of every run on an image so far, `None`
    /// where it was unset.
    fn configure_paths(&self) -> Vec<Option<String>> {
        Self::variable_log(&self.configure)
    }

    /// The `HOME` of every process started so far, the checks of the
    /// policy in effect included, `None` where it was unset.
    fn homes(&self) -> Vec<Option<String>> {
        Self::variable_log(&self.home)
    }

    /// The `policy.xml` each process started so far would have read first,
    /// `None` where its first configure directory held none.
    fn policies(&self) -> Vec<Option<String>> {
        let Ok(log) = std::fs::read_to_string(&self.policy) else {
            return Vec::new();
        };
        let mut runs: Vec<&str> = log.split("--end--\n").collect();
        // The log ends with a separator, so the last piece is empty.
        runs.pop();
        runs.into_iter()
            .map(|policy| (policy != MISSING).then(|| policy.to_string()))
            .collect()
    }

    /// Forget the invocations so far.
    fn clear(&self) {
        let _ = std::fs::remove_file(&self.argv);
        let _ = std::fs::remove_file(&self.stdin);
        let _ = std::fs::remove_file(&self.configure);
        let _ = std::fs::remove_file(&self.policy);
        let _ = std::fs::remove_file(&self.home);
    }
}

/// The directory that holds the framework's `policy.xml` inside the policy
/// directory, which ImageMagick reads as `$HOME/.config/ImageMagick`.
const POLICY_CONFIG_DIR: &str = ".config/ImageMagick";

/// Assert that every process started so far read the framework's policy,
/// and return the policy directory they read it from. Every process, the
/// checks of the policy in effect included, starts with `HOME` naming that
/// directory, so the framework's file is the last ImageMagick reads; and
/// every run on an image starts with the first directory of its configure
/// path at `<that directory>/.config/ImageMagick`, whose `policy.xml` is
/// the framework's byte for byte.
fn assert_every_run_read_the_policy(recorder: &Recorder) -> PathBuf {
    let runs = recorder.runs();
    let paths = recorder.configure_paths();
    let policies = recorder.policies();
    let homes = recorder.homes();
    assert!(!runs.is_empty(), "no process started");
    assert_eq!(paths.len(), runs.len(), "one configure path a process");
    assert_eq!(policies.len(), runs.len(), "one policy a process");
    assert!(
        homes.len() >= runs.len(),
        "one HOME a process, the checks included: {homes:?}"
    );
    let roots: Vec<PathBuf> = paths
        .iter()
        .zip(&policies)
        .zip(&runs)
        .map(|((path, policy), run)| {
            let path = path
                .as_deref()
                .unwrap_or_else(|| panic!("MAGICK_CONFIGURE_PATH was unset for {run:?}"));
            assert_eq!(
                policy.as_deref(),
                Some(MagickCliDriver::POLICY),
                "the first directory of `{path}` did not hold the framework's policy for {run:?}"
            );
            let first = Path::new(path.split(':').next().unwrap_or_default());
            assert!(
                first.ends_with(POLICY_CONFIG_DIR),
                "the first directory of `{path}` is `{POLICY_CONFIG_DIR}` in the policy \
                 directory for {run:?}"
            );
            first
                .parent()
                .and_then(Path::parent)
                .expect("the policy directory holds `.config`")
                .to_path_buf()
        })
        .collect();
    let root = roots[0].clone();
    assert!(
        roots.iter().all(|other| *other == root),
        "one policy directory a process: {roots:?}"
    );
    for home in &homes {
        assert_eq!(
            home.as_deref().map(Path::new),
            Some(root.as_path()),
            "every process starts with HOME naming the policy directory: {homes:?}"
        );
    }
    root
}

/// A scratch directory of the test's own, empty.
fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("hardening-magick-{name}"));
    if let Ok(meta) = std::fs::metadata(&dir) {
        // A read-only directory a previous run left behind cannot be
        // emptied until it is writable again.
        if meta.is_dir() {
            let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755));
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create the scratch directory");
    dir
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

/// Assert that `root`, `root/.config` and `root/.config/ImageMagick` are
/// each a directory, not a link, open to their owner alone.
fn assert_closed_directories(root: &Path, what: &str) {
    for dir in [
        root.to_path_buf(),
        root.join(".config"),
        root.join(POLICY_CONFIG_DIR),
    ] {
        let meta = std::fs::symlink_metadata(&dir)
            .unwrap_or_else(|e| panic!("{what}: {} exists: {e}", dir.display()));
        assert!(
            meta.is_dir(),
            "{what}: {} is a directory, not a link",
            dir.display()
        );
        assert_eq!(
            meta.mode() & 0o777,
            0o700,
            "{what}: {} is open to its owner alone",
            dir.display()
        );
    }
}

/// The names in `dir`, sorted.
fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .map(|entry| {
            entry
                .expect("read an entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

#[test]
#[serial]
fn hardening_every_run_reads_the_framework_policy() {
    let recorder = Recorder::new("policy-every-run");
    let driver = recorder.driver();
    {
        let _config = ConfigGuard::set(ImageConfig::default());
        driver.dimensions(RED_PNG_1X1).expect("dimensions");
        driver.dominant_color(RED_PNG_1X1).expect("dominant_color");
        driver
            .process(RED_PNG_1X1, &to_png())
            .expect("the pipeline runs");
    }
    // The opt-in hands ImageMagick bare stdin, and the policy still holds.
    {
        let _config = ConfigGuard::set(ImageConfig::default().magick_autodetect(true));
        driver.dimensions(PPM_1X1).expect("dimensions");
        driver
            .process(PPM_1X1, &to_png())
            .expect("the pipeline runs");
    }
    let root = assert_every_run_read_the_policy(&recorder);
    let runs = recorder.runs();
    assert!(runs.len() >= 5, "every entry point ran: {runs:?}");
    assert!(
        recorder.homes().len() >= runs.len() + 2,
        "the listing and the probe start with the same HOME as the runs"
    );

    // One directory a process, under the system's temporary directory,
    // open to its owner alone, holding the policy byte for byte where
    // ImageMagick reads it under HOME, and nothing else.
    assert!(
        root.starts_with(std::env::temp_dir()),
        "{} is under the temporary directory",
        root.display()
    );
    let name = root.file_name().expect("a name").to_string_lossy();
    assert!(
        name.starts_with(&format!("suprnova-magick-policy-{}-", std::process::id())),
        "the directory names the framework and the process: {name}"
    );
    assert_closed_directories(&root, "a new policy directory");
    assert_eq!(names_in(&root), [".config"]);
    assert_eq!(names_in(&root.join(".config")), ["ImageMagick"]);
    assert_eq!(names_in(&root.join(POLICY_CONFIG_DIR)), ["policy.xml"]);
    assert_eq!(
        std::fs::read(root.join(POLICY_CONFIG_DIR).join("policy.xml"))
            .expect("the policy is on disk"),
        MagickCliDriver::POLICY.as_bytes()
    );
}

#[test]
#[serial]
fn hardening_a_changed_policy_is_written_again_before_the_run() {
    let _config = ConfigGuard::set(ImageConfig::default());
    let recorder = Recorder::new("policy-rewritten");
    let driver = recorder.driver();
    driver.dimensions(RED_PNG_1X1).expect("dimensions");
    let root = assert_every_run_read_the_policy(&recorder);
    let file = root.join(POLICY_CONFIG_DIR).join("policy.xml");
    let policy = MagickCliDriver::POLICY.as_bytes();

    let elsewhere = scratch("policy-elsewhere").join("policy.xml");
    std::fs::write(&elsewhere, policy).expect("write a copy elsewhere");
    let changes: [(&str, &dyn Fn()); 5] = [
        ("a grant in place of the policy", &|| {
            std::fs::write(
                &file,
                "<policymap>\n  <policy domain=\"coder\" rights=\"read|write\" pattern=\"*\" />\n</policymap>\n",
            )
            .expect("change the policy");
        }),
        ("the policy cut short by one byte", &|| {
            std::fs::write(&file, &policy[..policy.len() - 1]).expect("truncate the policy");
        }),
        ("the policy with an entry after it", &|| {
            let mut longer = policy.to_vec();
            longer.extend_from_slice(
                b"<policymap><policy domain=\"coder\" rights=\"all\" pattern=\"MVG\" /></policymap>\n",
            );
            std::fs::write(&file, longer).expect("lengthen the policy");
        }),
        ("no policy", &|| {
            std::fs::remove_file(&file).expect("remove the policy");
        }),
        ("a link to a copy of the policy", &|| {
            std::fs::remove_file(&file).expect("remove the policy");
            std::os::unix::fs::symlink(&elsewhere, &file).expect("link to the copy");
        }),
    ];
    for (what, change) in changes {
        change();
        recorder.clear();
        driver.dimensions(RED_PNG_1X1).expect("dimensions");
        assert_eq!(
            assert_every_run_read_the_policy(&recorder),
            root,
            "{what}: the run read the framework's policy from the same directory"
        );
        let meta = std::fs::symlink_metadata(&file).expect("the policy is on disk");
        assert!(meta.is_file(), "{what}: the policy is a file of its own");
        assert_eq!(
            std::fs::read(&file).expect("read the policy"),
            policy,
            "{what}"
        );
    }
    assert_eq!(
        std::fs::read(&elsewhere).expect("read the copy"),
        policy,
        "writing over the link left its target alone"
    );

    // A run after `change` makes a new policy directory, closed like the
    // first, rather than trust the one it had.
    let replaced = |previous: &Path, what: &str| -> PathBuf {
        recorder.clear();
        driver.dimensions(RED_PNG_1X1).expect("dimensions");
        let made = assert_every_run_read_the_policy(&recorder);
        assert_ne!(made, previous, "{what}: a new directory");
        assert_closed_directories(&made, what);
        made
    };

    // A directory removed under the driver, as a temporary-file cleaner
    // removes one, is made again.
    std::fs::remove_dir_all(&root).expect("remove the directory");
    let mut current = replaced(&root, "a removed directory");

    // A directory opened to other users, who could then replace the
    // policy, is not trusted: the next run makes a new one. The same for
    // either directory inside it.
    for (inner, what) in [
        ("", "the policy directory opened up"),
        (".config", "its `.config` opened up"),
        (POLICY_CONFIG_DIR, "its `.config/ImageMagick` opened up"),
    ] {
        std::fs::set_permissions(current.join(inner), std::fs::Permissions::from_mode(0o777))
            .unwrap_or_else(|e| panic!("{what}: open the directory up: {e}"));
        let next = replaced(&current, what);
        std::fs::remove_dir_all(&current).expect("remove the opened directory");
        current = next;
    }

    // A `.config` replaced by a link is not trusted either, even to a
    // closed copy that holds the policy.
    let copy = scratch("policy-config-copy").join(".config");
    let copy_dir = copy.join("ImageMagick");
    std::fs::create_dir_all(&copy_dir).expect("make the copy");
    std::fs::write(copy_dir.join("policy.xml"), policy).expect("copy the policy");
    for dir in [&copy, &copy_dir] {
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
            .expect("close the copy");
    }
    std::fs::remove_dir_all(current.join(".config")).expect("remove `.config`");
    std::os::unix::fs::symlink(&copy, current.join(".config")).expect("link to the copy");
    let next = replaced(&current, "its `.config` replaced by a link");
    std::fs::remove_dir_all(&current).expect("remove the linked directory");
    assert_closed_directories(&next, "the last policy directory");
    assert_eq!(
        std::fs::read(copy_dir.join("policy.xml")).expect("read the copy"),
        policy,
        "the link's target is left alone"
    );
}

/// The temporary directories the refusal test starts its children with:
/// each name, and how the parent makes it.
const UNWRITABLE: [&str; 3] = ["read-only", "not-a-directory", "path:separator"];

#[test]
#[serial]
fn hardening_a_policy_that_cannot_be_written_refuses_the_run() {
    let root = scratch("policy-unwritable");
    for case in UNWRITABLE {
        let tmpdir = root.join(case);
        match case {
            "read-only" => {
                std::fs::create_dir(&tmpdir).expect("create the directory");
                std::fs::set_permissions(&tmpdir, std::fs::Permissions::from_mode(0o555))
                    .expect("make it read-only");
                // Permissions bind no one running as root.
                if std::fs::write(tmpdir.join("probe"), b"").is_ok() {
                    eprintln!("skipping the read-only case: this user writes past permissions");
                    continue;
                }
            }
            "not-a-directory" => std::fs::write(&tmpdir, b"a file").expect("create the file"),
            // ImageMagick splits its configure path at `:`, so a directory
            // whose path holds one would read as two.
            _ => std::fs::create_dir(&tmpdir).expect("create the directory"),
        }
        let output = {
            let _env = crate::env_lock::lock_env();
            crate::own_process::child_command(
                "magick_coder::hardening_a_policy_that_cannot_be_written_refuses_the_run_child",
            )
            .env("TMPDIR", &tmpdir)
            .env("HARDENING_MAGICK_TMPDIR_CASE", case)
            .output()
            .expect("the child runs")
        };
        crate::own_process::assert_child_passed(&output);
    }
    let _ = std::fs::set_permissions(
        root.join("read-only"),
        std::fs::Permissions::from_mode(0o755),
    );
}

/// Runs only in the child process the refusal test starts, with `TMPDIR`
/// naming a place the driver cannot make its policy directory in.
#[test]
fn hardening_a_policy_that_cannot_be_written_refuses_the_run_child() {
    if !crate::own_process::is_child() {
        return;
    }
    let case = std::env::var("HARDENING_MAGICK_TMPDIR_CASE").expect("the parent names the case");
    let tmpdir = std::env::var("TMPDIR").expect("the parent sets TMPDIR");
    let recorder = Recorder::new(&format!("policy-refused-{}", case.replace(':', "-")));
    let driver = recorder.driver();
    // A policy the driver cannot put in place refuses every run, with and
    // without the opt-in.
    let modes: [(&str, ImageConfig, &[&[u8]]); 2] = [
        (
            "without autodetection",
            ImageConfig::default(),
            &[RED_PNG_1X1],
        ),
        (
            "with autodetection",
            ImageConfig::default().magick_autodetect(true),
            &[RED_PNG_1X1, PPM_1X1],
        ),
    ];
    for (mode, config, inputs) in modes {
        let _config = ConfigGuard::set(config);
        for &input in inputs {
            let refusals = [
                ("process", driver.process(input, &to_png()).map(|_| ())),
                ("dimensions", driver.dimensions(input).map(|_| ())),
                ("dominant_color", driver.dominant_color(input).map(|_| ())),
            ];
            for (entry, result) in refusals {
                let err = result
                    .expect_err(&format!("{case}, {mode}: {entry} refuses the run"))
                    .to_string();
                assert!(
                    err.contains(&tmpdir),
                    "{case}, {mode}: {entry} names the path: {err}"
                );
                assert!(
                    err.contains("ImageMagick was not started"),
                    "{case}, {mode}: {entry} says what it refused: {err}"
                );
                assert!(
                    !err.contains("<policy"),
                    "{case}, {mode}: {entry} never carries the policy: {err}"
                );
            }
        }
    }
    assert!(
        recorder.runs().is_empty(),
        "{case}: a process started: {:?}",
        recorder.runs()
    );
    assert!(
        recorder.stdin().is_empty(),
        "{case}: input reached a process"
    );
}

#[test]
#[serial]
fn hardening_the_operators_configure_path_follows_the_frameworks() {
    for operator in [
        Some("/srv/operator-one:/srv/operator-two"),
        Some("/srv/operator"),
        Some(""),
        None,
    ] {
        // The account's own ImageMagick directory: `$XDG_CONFIG_HOME/ImageMagick`
        // when that is set, else `$HOME/.config/ImageMagick`.
        for xdg in [None, Some("/srv/xdg-config")] {
            let output = {
                let _env = crate::env_lock::lock_env();
                let mut child = crate::own_process::child_command(
                    "magick_coder::hardening_the_operators_configure_path_follows_the_frameworks_child",
                );
                match operator {
                    Some(value) => child.env("MAGICK_CONFIGURE_PATH", value),
                    None => child.env_remove("MAGICK_CONFIGURE_PATH"),
                };
                match xdg {
                    Some(value) => child.env("XDG_CONFIG_HOME", value),
                    None => child.env_remove("XDG_CONFIG_HOME"),
                };
                child.output().expect("the child runs")
            };
            crate::own_process::assert_child_passed(&output);
        }
    }
}

/// The account's own ImageMagick configuration directory, as ImageMagick
/// and the driver name it: `$XDG_CONFIG_HOME/ImageMagick` when that is set,
/// else `$HOME/.config/ImageMagick`; `None` with neither.
fn account_imagemagick_dir() -> Option<String> {
    let xdg = std::env::var_os("XDG_CONFIG_HOME").filter(|value| !value.is_empty());
    let dir = match xdg {
        Some(xdg) => std::path::PathBuf::from(xdg).join("ImageMagick"),
        None => std::path::PathBuf::from(std::env::var_os("HOME")?)
            .join(".config")
            .join("ImageMagick"),
    };
    Some(dir.to_string_lossy().into_owned())
}

/// Runs only in the child process the configure-path test starts, with the
/// operator's `MAGICK_CONFIGURE_PATH` set or unset.
#[test]
fn hardening_the_operators_configure_path_follows_the_frameworks_child() {
    if !crate::own_process::is_child() {
        return;
    }
    let operator = std::env::var("MAGICK_CONFIGURE_PATH").ok();
    let account = account_imagemagick_dir().expect("the test process has a HOME");
    let _config = ConfigGuard::set(ImageConfig::default());
    let recorder = Recorder::new(&format!(
        "policy-operator-{}-{}",
        operator
            .as_deref()
            .map_or("unset", |value| if value.is_empty() {
                "empty"
            } else if value.contains(':') {
                "two"
            } else {
                "one"
            }),
        if std::env::var_os("XDG_CONFIG_HOME").is_some() {
            "xdg"
        } else {
            "home"
        }
    ));
    recorder
        .driver()
        .dimensions(RED_PNG_1X1)
        .expect("dimensions");
    let root = assert_every_run_read_the_policy(&recorder);
    let framework = root.join(POLICY_CONFIG_DIR);
    let framework = framework.to_string_lossy();
    let expected = match operator.as_deref() {
        Some(value) if !value.is_empty() => format!("{framework}:{value}:{account}:{framework}"),
        _ => format!("{framework}:{account}:{framework}"),
    };
    assert_eq!(
        recorder.configure_paths(),
        [Some(expected.clone())],
        "the directory that holds the framework's file first and last, the operator's value \
         unchanged and then the account's own ImageMagick directory between"
    );
    assert_eq!(
        recorder.listings(),
        std::slice::from_ref(&expected),
        "the policy in effect is read under the same configure path"
    );
    assert_eq!(
        recorder.probes(),
        [expected],
        "the MVG probe runs under the same configure path"
    );
    assert_eq!(
        recorder.homes().len(),
        3,
        "the listing, the probe and the run each start with HOME naming the policy directory"
    );
}

#[test]
fn hardening_the_policy_denies_every_coder_hrd_010_names() {
    // Each entry is one line: `<policy domain="..." rights="..." pattern="..." />`.
    let entries: Vec<(&str, &str, &str)> = MagickCliDriver::POLICY
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("<policy "))
        .map(|line| {
            let parts: Vec<&str> = line.split('"').collect();
            assert_eq!(
                parts[..],
                [
                    "<policy domain=",
                    parts[1],
                    " rights=",
                    parts[3],
                    " pattern=",
                    parts[5],
                    " />"
                ],
                "an entry of the expected shape: {line}"
            );
            (parts[1], parts[3], parts[5])
        })
        .collect();
    assert_eq!(
        entries.len(),
        MagickCliDriver::POLICY.matches("<policy ").count(),
        "every entry is on a line of its own"
    );
    for coder in DENIED_CODERS {
        assert!(
            entries.contains(&("coder", "none", coder)),
            "the policy denies the {coder} coder"
        );
    }
    assert!(
        entries.contains(&("delegate", "none", "*")),
        "every delegate"
    );
    assert!(
        entries.contains(&("path", "none", "@*")),
        "indirect path reads"
    );
    assert_eq!(
        entries.len(),
        DENIED_CODERS.len() + 2,
        "nothing beyond what HRD-010 names: {entries:?}"
    );
    // Every entry denies; the policy grants nothing.
    assert!(
        entries.iter().all(|(_, rights, _)| *rights == "none"),
        "{entries:?}"
    );
    assert!(MagickCliDriver::POLICY.starts_with("<?xml"));
    assert!(MagickCliDriver::POLICY.trim_end().ends_with("</policymap>"));
}

/// Where the listings below say the framework's policy came from: the
/// driver's `HOME`, which the stand-in fills in.
const FRAMEWORK_POLICY: &str = crate::magick_policy::FRAMEWORK_POLICY;

/// A listing of the framework's policy followed by `file`, a host policy
/// ImageMagick read after it, holding `entries`.
fn listing_with(file: &str, entries: &[crate::magick_policy::Entry<'_>]) -> String {
    crate::magick_policy::listing(&[
        (FRAMEWORK_POLICY, &crate::magick_policy::framework_entries()),
        (file, entries),
    ])
}

/// A listing of `file`, a host policy holding `entries`, between two reads
/// of the framework's policy: the shape ImageMagick lists under the driver,
/// which names the directory that holds the framework's file first on
/// `MAGICK_CONFIGURE_PATH` and `HOME` last.
fn listing_around(file: &str, entries: &[crate::magick_policy::Entry<'_>]) -> String {
    let framework = crate::magick_policy::framework_entries();
    crate::magick_policy::listing(&[
        (FRAMEWORK_POLICY, &framework),
        (file, entries),
        (FRAMEWORK_POLICY, &framework),
    ])
}

/// Assert that every entry point of `driver` refuses the run in both
/// modes: with autodetection on, for a named input and an unnamed one, and
/// with it off, for a named input. Each error says the driver refused to
/// run ImageMagick, names each of `named`, and carries none of `absent` and
/// never a policy's bytes.
fn assert_refused_in_both_modes(
    driver: &MagickCliDriver,
    name: &str,
    named: &[&str],
    absent: &[&str],
) {
    let modes: [(&str, ImageConfig, &[&[u8]]); 2] = [
        (
            "with autodetection",
            ImageConfig::default().magick_autodetect(true),
            &[RED_PNG_1X1, PPM_1X1],
        ),
        (
            "without autodetection",
            ImageConfig::default(),
            &[RED_PNG_1X1],
        ),
    ];
    for (mode, config, inputs) in modes {
        let _config = ConfigGuard::set(config);
        for &input in inputs {
            let refusals = [
                ("process", driver.process(input, &to_png()).map(|_| ())),
                ("dimensions", driver.dimensions(input).map(|_| ())),
                ("dominant_color", driver.dominant_color(input).map(|_| ())),
            ];
            for (entry, result) in refusals {
                let err = result
                    .expect_err(&format!("{name}, {mode}: {entry} refuses the run"))
                    .to_string();
                assert!(
                    err.contains("refused to run ImageMagick"),
                    "{name}, {mode}: {entry}: {err}"
                );
                for part in named {
                    assert!(
                        err.contains(part),
                        "{name}, {mode}: {entry} names {part}: {err}"
                    );
                }
                for part in absent.iter().chain(&["<policy"]) {
                    assert!(
                        !err.contains(part),
                        "{name}, {mode}: {entry} never carries {part}: {err}"
                    );
                }
            }
        }
    }
}

#[test]
#[serial]
fn hardening_a_denying_listing_lets_runs_proceed_and_is_asked_for_once() {
    // Under the opt-in a failed check refuses the run, so a run that
    // proceeds here is the listing passing the check.
    let _config = ConfigGuard::set(ImageConfig::default().magick_autodetect(true));
    // A web-safe host policy read between the framework's: it denies every
    // coder, grants four back, denies every delegate again, grants reads
    // under one directory, and limits a resource. None of that reopens what
    // the framework denies.
    let web_safe = listing_around(
        "/etc/ImageMagick-7/policy.xml",
        &[
            ("Coder", "None", "*"),
            ("Coder", "Read Write", "{GIF,JPEG,PNG,WEBP}"),
            // Patterns that read like a denied coder and match none.
            ("Coder", "Read Write", "[!M]VG"),
            ("Coder", "Read", "P?D"),
            ("Delegate", "None", "*"),
            ("Path", "Read", "/repository/*"),
        ],
    )
    .replace(
        "Path: /etc/ImageMagick-7/policy.xml\n",
        "Path: /etc/ImageMagick-7/policy.xml\n  Policy: Resource\n    name: memory\n    value: 256MiB\n",
    );
    for (name, listing) in [
        ("listing-denies", None),
        ("listing-web-safe", Some(web_safe)),
    ] {
        let recorder = Recorder::new(name);
        if let Some(listing) = &listing {
            recorder.answer_listing(listing);
        }
        let driver = recorder.driver();
        driver.dimensions(RED_PNG_1X1).expect("dimensions");
        driver.dominant_color(RED_PNG_1X1).expect("dominant_color");
        driver
            .process(RED_PNG_1X1, &to_png())
            .expect("the pipeline runs");
        driver
            .process(RED_PNG_1X1, &to_png())
            .expect("the pipeline runs again");
        let runs = recorder.runs();
        assert!(runs.len() >= 4, "{name}: every entry point ran: {runs:?}");
        let listings = recorder.listings();
        assert_eq!(
            listings.len(),
            1,
            "{name}: the policy in effect is asked for once a process: {listings:?}"
        );
        assert!(
            recorder
                .configure_paths()
                .iter()
                .all(|path| path.as_deref() == Some(listings[0].as_str())),
            "{name}: the listing ran under the runs' configure path"
        );
        assert_eq!(
            recorder.probes(),
            listings,
            "{name}: one MVG probe a process, under the same configure path"
        );
    }
}

#[test]
#[serial]
fn hardening_a_grant_read_before_the_frameworks_policy_is_outranked() {
    // The framework's file is the last ImageMagick reads, so a grant in any
    // file before it, on the operator's configure path, in ImageMagick's
    // own directory or in `$XDG_CONFIG_HOME/ImageMagick`, is outranked by
    // the framework's denial: the check passes and the runs proceed.
    let framework = crate::magick_policy::framework_entries();
    let grants: &[crate::magick_policy::Entry<'_>] = &[
        ("Coder", "Read Write", "MVG"),
        ("Coder", "Read", "*"),
        ("Delegate", "Execute", "gs"),
        ("Path", "Read", "@*"),
    ];
    let listing = crate::magick_policy::listing(&[
        (FRAMEWORK_POLICY, &framework),
        ("/srv/operator/policy.xml", grants),
        (FRAMEWORK_POLICY, &framework),
        ("/etc/ImageMagick-7/policy.xml", grants),
        ("/srv/xdg/ImageMagick/policy.xml", grants),
        (FRAMEWORK_POLICY, &framework),
    ]);
    for (name, config) in [
        (
            "listing-grants-before-with-autodetection",
            ImageConfig::default().magick_autodetect(true),
        ),
        (
            "listing-grants-before-without-autodetection",
            ImageConfig::default(),
        ),
    ] {
        let _config = ConfigGuard::set(config);
        let recorder = Recorder::new(name);
        recorder.answer_listing(&listing);
        let driver = recorder.driver();
        driver
            .dimensions(RED_PNG_1X1)
            .unwrap_or_else(|e| panic!("{name}: the run proceeds: {e}"));
        driver
            .process(RED_PNG_1X1, &to_png())
            .unwrap_or_else(|e| panic!("{name}: the pipeline runs: {e}"));
        assert!(recorder.runs().len() >= 2, "{name}: {:?}", recorder.runs());
        assert_eq!(recorder.listings().len(), 1, "{name}: one listing");
        assert_eq!(
            recorder.probes().len(),
            1,
            "{name}: the listing passed, so the probe ran"
        );
    }
}

#[test]
#[serial]
fn hardening_a_policy_file_read_after_the_frameworks_refuses_every_run() {
    // A binary that reads a file after `$HOME/.config/ImageMagick` lets an
    // entry there outrank the framework's. The file below only denies, so
    // the rule for each denial passes; the order alone refuses the run, in
    // both modes, whichever mode asks first.
    let user = "/home/x/.config/ImageMagick/policy.xml";
    let listing = listing_with(user, &[("Coder", "None", "MVG")]);
    let named = [
        user,
        "after the framework's policy",
        "applies the last entry that matches",
    ];
    for (name, first) in [
        ("listing-file-after-asked-with-autodetection", true),
        ("listing-file-after-asked-without-autodetection", false),
    ] {
        let recorder = Recorder::new(name);
        recorder.answer_listing(&listing);
        let driver = recorder.driver();
        {
            let _config = ConfigGuard::set(ImageConfig::default().magick_autodetect(first));
            let err = driver
                .dimensions(RED_PNG_1X1)
                .expect_err(&format!("{name}: the first run is refused"))
                .to_string();
            assert!(err.contains("refused to run ImageMagick"), "{name}: {err}");
            for part in named {
                assert!(err.contains(part), "{name}: names {part}: {err}");
            }
        }
        assert_refused_in_both_modes(&driver, name, &named, &[]);
        assert!(
            recorder.runs().is_empty(),
            "{name}: a process ran on an image: {:?}",
            recorder.runs()
        );
        assert!(
            recorder.stdin().is_empty(),
            "{name}: input reached a process"
        );
        assert_eq!(
            recorder.listings().len(),
            1,
            "{name}: the verdict is kept for the process"
        );
        assert!(
            recorder.probes().is_empty(),
            "{name}: a refused listing needs no probe"
        );
    }
}

#[test]
#[serial]
fn hardening_a_listing_that_reopens_a_denial_refuses_every_run() {
    let host = "/etc/ImageMagick-7/policy.xml";
    let user = "/home/app/.config/ImageMagick/policy.xml";
    let framework = crate::magick_policy::framework_entries();
    let without_xps: Vec<_> = framework
        .iter()
        .copied()
        .filter(|(_, _, pattern)| *pattern != "XPS")
        .collect();
    let lowercase_mvg: Vec<_> = framework
        .iter()
        .map(|&(domain, rights, pattern)| {
            (
                domain,
                rights,
                if pattern == "MVG" { "mvg" } else { pattern },
            )
        })
        .collect();
    let cases: [(&str, String, &[&str]); 10] = [
        (
            "listing-grants-mvg",
            listing_with(host, &[("Coder", "Read Write", "MVG")]),
            &["the MVG coder", "`MVG`", "Read Write", host],
        ),
        (
            "listing-grants-every-coder",
            listing_with(user, &[("Coder", "Read", "*")]),
            &["the URL coder", "`*`", user],
        ),
        // ImageMagick 7.1.2 matches with case, but the check does not rely
        // on it: a grant that matches ignoring case reopens the coder.
        (
            "listing-grants-ignoring-case",
            listing_with(host, &[("Coder", "Read Write", "mvg")]),
            &["the MVG coder", "`mvg`", host],
        ),
        // Each glob form ImageMagick reads: one character, a set, and a
        // list of alternatives.
        (
            "listing-grants-one-character",
            listing_with(host, &[("Coder", "Read", "M?G")]),
            &["the MVG coder", "`M?G`"],
        ),
        (
            "listing-grants-a-set",
            listing_with(host, &[("Coder", "Read", "[L-N]VG")]),
            &["the MVG coder", "`[L-N]VG`"],
        ),
        (
            "listing-grants-alternatives",
            listing_with(host, &[("Coder", "Write", "{GIF,MSL}")]),
            &["the MSL coder", "`{GIF,MSL}`", "Write"],
        ),
        (
            "listing-grants-a-delegate",
            listing_with(host, &[("Delegate", "Execute", "gs")]),
            &["`delegate`", "`gs`", "Execute", host],
        ),
        (
            "listing-grants-an-indirect-read",
            listing_with(host, &[("Path", "Read", "@/srv/*")]),
            &["`path`", "`@/srv/*`", host],
        ),
        // A coder no entry names is open: ImageMagick did not read the
        // framework's policy, or read another one.
        (
            "listing-misses-a-coder",
            crate::magick_policy::listing(&[(FRAMEWORK_POLICY, &without_xps)]),
            &["does not deny the XPS coder"],
        ),
        // And a denial that matches only ignoring case does not close it.
        (
            "listing-denies-only-ignoring-case",
            crate::magick_policy::listing(&[(FRAMEWORK_POLICY, &lowercase_mvg)]),
            &["does not deny the MVG coder"],
        ),
    ];
    for (name, listing, named) in cases {
        let recorder = Recorder::new(name);
        recorder.answer_listing(&listing);
        // The verdict is made by the first run, with autodetection on, and
        // kept: the runs without it are refused by the same verdict.
        assert_refused_in_both_modes(&recorder.driver(), name, named, &[]);
        assert!(
            recorder.runs().is_empty(),
            "{name}: a process ran on an image: {:?}",
            recorder.runs()
        );
        assert!(
            recorder.stdin().is_empty(),
            "{name}: input reached a process"
        );
        assert_eq!(
            recorder.listings().len(),
            1,
            "{name}: the verdict is kept for the process"
        );
        assert!(
            recorder.probes().is_empty(),
            "{name}: a refused listing needs no probe"
        );
    }
}

#[test]
#[serial]
fn hardening_a_listing_that_cannot_be_read_refuses() {
    let unknown_right =
        crate::magick_policy::framework_listing().replacen("rights: None", "rights: Everything", 1);
    for (name, listing) in [
        (
            "listing-garbage",
            "this is not a policy listing\n".to_string(),
        ),
        (
            "listing-without-entries",
            format!("\nPath: {FRAMEWORK_POLICY}\n\nPath: [built-in]\n"),
        ),
        ("listing-unknown-right", unknown_right),
    ] {
        let recorder = Recorder::new(name);
        recorder.answer_listing(&listing);
        let driver = recorder.driver();
        for autodetect in [true, false] {
            let _config = ConfigGuard::set(ImageConfig::default().magick_autodetect(autodetect));
            for _ in 0..2 {
                let err = driver
                    .dimensions(RED_PNG_1X1)
                    .expect_err(&format!("{name}: an unreadable listing refuses the run"))
                    .to_string();
                assert!(err.contains("refused to run ImageMagick"), "{name}: {err}");
                assert!(err.contains("cannot be verified"), "{name}: {err}");
                assert!(
                    !err.contains("not a policy listing"),
                    "{name}: the error never repeats the listing: {err}"
                );
            }
        }
        assert!(recorder.runs().is_empty(), "{name}: {:?}", recorder.runs());
        assert_eq!(
            recorder.listings().len(),
            1,
            "{name}: an unreadable listing is a verdict, kept for the process"
        );
    }

    // A listing that could not be had is refused too, in both modes, and
    // asked for again at the next run: the fault may pass.
    let recorder = Recorder::new("listing-fails");
    recorder.fail_listing();
    let driver = recorder.driver();
    for autodetect in [true, false] {
        let _config = ConfigGuard::set(ImageConfig::default().magick_autodetect(autodetect));
        for _ in 0..2 {
            let err = driver
                .dimensions(RED_PNG_1X1)
                .expect_err("a failed listing refuses the run")
                .to_string();
            assert!(err.contains("refused to run ImageMagick"), "{err}");
            assert!(err.contains("-list policy` failed"), "{err}");
            assert!(err.contains("cannot be verified"), "{err}");
            assert!(err.contains("unable to list the policy"), "{err}");
        }
    }
    assert!(recorder.runs().is_empty(), "{:?}", recorder.runs());
    assert_eq!(
        recorder.listings().len(),
        4,
        "asked again at every run, in both modes"
    );
}

#[test]
#[serial]
fn hardening_the_mvg_probe_must_meet_the_policy_refusal() {
    // ImageMagick's refusal, the stand-in's default: runs proceed, and the
    // probe is asked for once a process.
    {
        let _config = ConfigGuard::set(ImageConfig::default().magick_autodetect(true));
        let recorder = Recorder::new("probe-refused");
        let driver = recorder.driver();
        driver.dimensions(RED_PNG_1X1).expect("dimensions");
        driver
            .process(PPM_1X1, &to_png())
            .expect("the pipeline runs");
        driver.dominant_color(RED_PNG_1X1).expect("dominant_color");
        assert!(recorder.runs().len() >= 3, "{:?}", recorder.runs());
        assert_eq!(recorder.probes().len(), 1, "one probe a process");
    }

    // A probe ImageMagick reads, as under a stealth grant of MVG, or fails
    // for another reason, or refuses for another coder, cannot show that
    // the policy in effect denies MVG.
    let other_coder = crate::magick_policy::MVG_REFUSAL.replace("`MVG'", "`PS'");
    let cases: [(&str, &str, &str, u8, &str); 3] = [
        (
            "probe-read",
            "1",
            "",
            0,
            "ImageMagick read an MVG probe script",
        ),
        (
            "probe-fails-otherwise",
            "",
            "magick: unable to open image `-': No such file or directory\n",
            1,
            "for a reason other than its security policy",
        ),
        (
            "probe-refused-for-another-coder",
            "",
            &other_coder,
            1,
            "for a reason other than its security policy",
        ),
    ];
    for (name, stdout, stderr, exit, outcome) in cases {
        let recorder = Recorder::new(name);
        recorder.answer_probe(stdout, stderr, exit);
        assert_refused_in_both_modes(
            &recorder.driver(),
            name,
            &["cannot be shown to deny the MVG coder", "stealth", outcome],
            &["unable to open image", "IsCoderAuthorized"],
        );
        assert!(
            recorder.runs().is_empty(),
            "{name}: a process ran on an image: {:?}",
            recorder.runs()
        );
        assert!(
            recorder.stdin().is_empty(),
            "{name}: input reached a process"
        );
        assert_eq!(
            recorder.probes().len(),
            1,
            "{name}: the verdict is kept for the process"
        );
    }
}

#[test]
#[serial]
fn hardening_a_probe_that_cannot_be_had_refuses_and_is_asked_again() {
    // The probe outlives the deadline, so the driver kills it: there is no
    // exit status to judge, which refuses the run in both modes but is not
    // kept.
    let recorder = Recorder::new("probe-hangs");
    recorder.hang_probe();
    let driver = recorder.driver();
    for (autodetect, runs) in [(true, 2), (false, 1)] {
        let mut config = ImageConfig::default().magick_autodetect(autodetect);
        config.magick_timeout_secs = 1;
        let _config = ConfigGuard::set(config);
        for _ in 0..runs {
            let err = driver
                .dimensions(RED_PNG_1X1)
                .expect_err("a probe past the deadline refuses the run")
                .to_string();
            assert!(err.contains("refused to run ImageMagick"), "{err}");
            assert!(err.contains("MVG probe"), "{err}");
            assert!(err.contains("could not be run"), "{err}");
            assert!(err.contains("timed out"), "{err}");
        }
    }
    assert!(recorder.runs().is_empty(), "{:?}", recorder.runs());
    assert_eq!(
        recorder.probes().len(),
        3,
        "asked again at every run, in both modes"
    );
}
