#![cfg(feature = "media")]
//! Live-binary integration tests for the `magick` image driver.
//!
//! These shell out to a real ImageMagick 7 on the host, so they are
//! `#[ignore]`d and the unattended gate never runs them. Run with:
//!
//! ```sh
//! cargo test -p suprnova --features media --test image_magick_driver -- --ignored
//! ```
//!
//! The argument-construction contract is covered by pure unit tests inside
//! `framework/src/media/magick.rs`; what only a real binary can prove is that
//! those arguments mean what the mapping claims they mean. Tests that need a
//! format-specific delegate (HEIC) skip-detect rather than fail, because a
//! delegate is a host build option, not a Suprnova defect.

use std::process::Command;

use suprnova::{Image, ImageDriver, ImagePipeline, MagickCliDriver, OutputFormat, Transformation};

use crate::image_processing::{photo_bmp, webp_chunks};

/// 1x1 red PNG, the same verified fixture the pure-Rust tests use.
const RED_PNG_1X1: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53,
    0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x78, 0xDA, 0x63, 0xF8, 0xCF, 0xC0, 0x00,
    0x00, 0x03, 0x01, 0x01, 0x00, 0xF7, 0x03, 0x41, 0x43, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E,
    0x44, 0xAE, 0x42, 0x60, 0x82,
];

fn driver() -> MagickCliDriver {
    MagickCliDriver::from_env()
}

fn pipeline(steps: Vec<Transformation>, format: OutputFormat) -> ImagePipeline {
    ImagePipeline {
        transformations: steps,
        format: Some(format),
        ..ImagePipeline::default()
    }
}

/// True when the host's ImageMagick lists a read delegate for `format`.
fn supports_format(format: &str) -> bool {
    let Ok(output) = Command::new(MagickCliDriver::from_env().binary())
        .args(["-list", "format"])
        .output()
    else {
        return false;
    };
    String::from_utf8_lossy(&output.stdout).lines().any(|line| {
        let mut fields = line.split_whitespace();
        let name = fields.next().unwrap_or_default().trim_end_matches('*');
        // Columns are: NAME MODULE MODE DESCRIPTION; mode "rw+"/"r--" etc.
        name.eq_ignore_ascii_case(format) && fields.nth(1).is_some_and(|mode| mode.starts_with('r'))
    })
}

#[test]
#[ignore = "requires a host ImageMagick 7 binary"]
fn resize_round_trips_through_the_real_binary() {
    let out = driver()
        .process(
            RED_PNG_1X1,
            &pipeline(
                vec![Transformation::Resize {
                    width: 12,
                    height: 6,
                }],
                OutputFormat::Png,
            ),
        )
        .expect("magick must resize the fixture");
    assert!(out.starts_with(b"\x89PNG"), "expected a PNG back");

    let (width, height) = driver().dimensions(&out).expect("dimensions");
    assert_eq!(
        (width, height),
        (12, 6),
        "the `!` geometry suffix must force exact dimensions"
    );
}

#[test]
#[ignore = "requires a host ImageMagick 7 binary"]
fn scale_never_enlarges_through_the_real_binary() {
    // The whole point of the `>` suffix: asking for a bigger box is a no-op.
    let enlarged = driver()
        .process(
            RED_PNG_1X1,
            &pipeline(
                vec![Transformation::Scale {
                    width: 100,
                    height: 100,
                }],
                OutputFormat::Png,
            ),
        )
        .expect("magick must handle the scale");
    assert_eq!(
        driver().dimensions(&enlarged).expect("dimensions"),
        (1, 1),
        "scale must never enlarge"
    );
}

#[test]
#[ignore = "requires a host ImageMagick 7 binary"]
fn cover_fills_the_box_exactly() {
    let source = driver()
        .process(
            RED_PNG_1X1,
            &pipeline(
                vec![Transformation::Resize {
                    width: 40,
                    height: 10,
                }],
                OutputFormat::Png,
            ),
        )
        .expect("build a wide source");

    let covered = driver()
        .process(
            &source,
            &pipeline(
                vec![Transformation::Cover {
                    width: 20,
                    height: 20,
                }],
                OutputFormat::Png,
            ),
        )
        .expect("cover");
    assert_eq!(
        driver().dimensions(&covered).expect("dimensions"),
        (20, 20),
        "the ^ resize plus -extent must land exactly on the target box"
    );
}

#[test]
#[ignore = "requires a host ImageMagick 7 binary"]
fn every_output_format_encodes_through_the_real_binary() {
    for (format, magic) in [
        (OutputFormat::Png, &b"\x89PNG"[..]),
        (OutputFormat::Jpeg, &[0xFF, 0xD8, 0xFF][..]),
        (OutputFormat::Gif, &b"GIF"[..]),
        (OutputFormat::Bmp, &b"BM"[..]),
        (OutputFormat::WebP, &b"RIFF"[..]),
    ] {
        if !supports_format(format.extension()) {
            eprintln!("skipping {format:?}: no host delegate");
            continue;
        }
        let out = driver()
            .process(RED_PNG_1X1, &pipeline(Vec::new(), format))
            .unwrap_or_else(|e| panic!("{format:?} must encode: {e}"));
        assert!(
            out.starts_with(magic),
            "{format:?} produced the wrong magic bytes: {:02x?}",
            &out[..out.len().min(8)]
        );
    }
}

#[test]
#[ignore = "requires a host ImageMagick 7 binary"]
fn webp_quality_reaches_the_lossy_encoder_through_the_real_binary() {
    if !supports_format("webp") {
        eprintln!("skipping: this host's ImageMagick has no WebP delegate");
        return;
    }
    let source = photo_bmp();
    let encode = |quality: u8| {
        driver()
            .process(
                &source,
                &ImagePipeline {
                    format: Some(OutputFormat::WebP),
                    quality,
                    ..ImagePipeline::default()
                },
            )
            .unwrap_or_else(|e| panic!("magick must write WebP at quality {quality}: {e}"))
    };
    let (low, high) = (encode(50), encode(90));
    assert!(
        low.len() < high.len(),
        "quality 50 ({} bytes) must be smaller than quality 90 ({} bytes)",
        low.len(),
        high.len()
    );
    let chunks = webp_chunks(&high);
    assert!(
        chunks.iter().any(|chunk| chunk == "VP8 "),
        "an opaque image must carry a lossy VP8 bitstream, got {chunks:?}"
    );
}

#[test]
#[ignore = "requires a host ImageMagick 7 binary"]
fn webp_lossless_reaches_the_lossless_encoder_through_the_real_binary() {
    if !supports_format("webp") {
        eprintln!("skipping: this host's ImageMagick has no WebP delegate");
        return;
    }
    let out = driver()
        .process(
            &photo_bmp(),
            &pipeline(Vec::new(), OutputFormat::WebPLossless),
        )
        .expect("magick must write lossless WebP");
    let chunks = webp_chunks(&out);
    assert!(
        chunks.iter().any(|chunk| chunk == "VP8L"),
        "the lossless define must select a VP8L bitstream, got {chunks:?}"
    );
    assert!(
        !chunks.iter().any(|chunk| chunk == "VP8 "),
        "a lossless file must carry no lossy bitstream, got {chunks:?}"
    );
}

#[test]
#[ignore = "requires a host ImageMagick 7 binary"]
fn dominant_color_reads_back_the_source_colour() {
    let color = driver()
        .dominant_color(RED_PNG_1X1)
        .expect("dominant colour");
    assert_eq!(color, "#ff0000");
}

#[test]
#[ignore = "requires a host ImageMagick 7 binary"]
fn a_missing_binary_fails_with_an_actionable_message() {
    let absent = MagickCliDriver::new("suprnova-definitely-not-installed");
    let err = absent
        .process(RED_PNG_1X1, &pipeline(Vec::new(), OutputFormat::Png))
        .expect_err("the binary does not exist");
    let message = err.to_string();
    assert!(message.contains("IMAGE_MAGICK_BINARY"), "got: {message}");
}

#[test]
#[ignore = "requires a host ImageMagick 7 binary with the libheif delegate"]
fn heic_decodes_when_the_host_carries_the_delegate() {
    if !supports_format("heic") {
        eprintln!(
            "skipping: this host's ImageMagick has no HEIC read delegate. \
             That is a host build option, not a Suprnova defect."
        );
        return;
    }

    // Build the HEIC fixture with the same binary under test, so the test
    // carries no binary blob for a format the framework cannot itself write.
    let heic = Command::new(MagickCliDriver::from_env().binary())
        .args(["-size", "8x4", "xc:red", "heic:-"])
        .output()
        .expect("magick must run");
    assert!(
        heic.status.success() && !heic.stdout.is_empty(),
        "could not build a HEIC fixture: {}",
        String::from_utf8_lossy(&heic.stderr)
    );

    // This is the case the whole driver exists for: the pure-Rust driver
    // refuses HEIC by design, and IMAGE_DRIVER=magick reads it.
    let out = driver()
        .process(
            &heic.stdout,
            &pipeline(
                vec![Transformation::Resize {
                    width: 4,
                    height: 2,
                }],
                OutputFormat::Png,
            ),
        )
        .expect("magick must ingest HEIC when the delegate is present");
    assert!(out.starts_with(b"\x89PNG"));

    // And the result is readable by the pure-Rust side: HEIC in, a format
    // Suprnova fully supports out.
    let (width, height) = suprnova::OxideAvImageDriver::new()
        .dimensions(&out)
        .expect("the converted PNG must decode in the pure-Rust driver");
    assert_eq!((width, height), (4, 2));
}

#[test]
#[ignore = "requires a host ImageMagick 7 binary"]
fn hardening_each_named_coder_reads_its_format_through_the_real_binary() {
    // The driver hands ImageMagick `<coder>:-` for every format the sniffer
    // names, so a coder name ImageMagick does not know would refuse a file it
    // reads. Each fixture is written by the binary under test; `tiff64` is
    // its name for BigTIFF.
    for (writer, sniffed) in [
        ("heic", "heic"),
        ("avif", "avif"),
        ("tiff", "tiff"),
        ("tiff64", "tiff"),
        ("psd", "psd"),
        ("psb", "psb"),
    ] {
        if !supports_format(writer) {
            eprintln!("skipping {writer}: no host delegate");
            continue;
        }
        let made = Command::new(driver().binary())
            .args(["-size", "8x4", "xc:red", &format!("{writer}:-")])
            .output()
            .expect("magick must run");
        assert!(
            made.status.success() && !made.stdout.is_empty(),
            "could not write a {writer} fixture: {}",
            String::from_utf8_lossy(&made.stderr)
        );
        assert_eq!(
            driver()
                .dimensions(&made.stdout)
                .unwrap_or_else(|e| panic!("{writer} read as {sniffed}: {e}")),
            (8, 4),
            "{writer}"
        );
        let png = driver()
            .process(&made.stdout, &pipeline(Vec::new(), OutputFormat::Png))
            .unwrap_or_else(|e| panic!("{writer} read as {sniffed}: {e}"));
        assert!(png.starts_with(b"\x89PNG"), "{writer}");
    }
}

#[test]
#[serial_test::serial]
#[ignore = "requires a host ImageMagick 7 binary"]
fn hardening_an_mvg_document_is_refused_before_the_real_binary_runs() {
    // Under the default configuration the bytes never reach ImageMagick, so
    // the refusal is the framework's, whatever the host's policy.xml allows.
    let _config = crate::image_processing::ConfigGuard::set(suprnova::ImageConfig::default());
    let mvg = include_bytes!("fixtures/magick-script.mvg");
    let err = driver()
        .process(mvg, &pipeline(Vec::new(), OutputFormat::Png))
        .expect_err("an MVG document is refused");
    assert!(
        err.to_string().contains("not one Suprnova names"),
        "got: {err}"
    );
}

#[test]
#[serial_test::serial]
#[ignore = "requires a host ImageMagick 7 binary"]
fn hardening_autodetected_input_meets_the_framework_policy_in_the_real_binary() {
    // Under the opt-in, ImageMagick reads PostScript and PDF by their
    // magic and hands them to Ghostscript. The framework's policy denies
    // both coders, so ImageMagick refuses them by policy before it starts
    // anything, on a host whose own policy is silent on them.
    let _config = crate::image_processing::ConfigGuard::set(
        suprnova::ImageConfig::default().magick_autodetect(true),
    );
    let documents: [(&str, &[u8]); 2] = [
        (
            "an EPS document",
            b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 1 1\nshowpage\n",
        ),
        (
            "a PDF document",
            b"%PDF-1.4\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n\
              2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj\n\
              3 0 obj<</Type/Page/MediaBox[0 0 1 1]/Parent 2 0 R>>endobj\n\
              trailer<</Root 1 0 R>>\n%%EOF\n",
        ),
    ];
    for (what, bytes) in documents {
        let err = driver()
            .process(bytes, &pipeline(Vec::new(), OutputFormat::Png))
            .expect_err(&format!("{what} is refused"));
        assert!(
            err.to_string().contains("security policy"),
            "{what} is refused by the policy: {err}"
        );
    }
}

#[test]
#[serial_test::serial]
#[ignore = "requires a host ImageMagick 7 binary"]
fn hardening_the_installed_policy_in_effect_passes_the_check() {
    // The driver reads `magick -list policy` before its first run, then
    // runs an MVG probe that the installed ImageMagick's policy must
    // refuse. It refuses every run unless both pass, with or without the
    // autodetection opt-in, so a run that succeeds is the listing passing
    // and the probe refused.
    let _config = crate::image_processing::ConfigGuard::set(
        suprnova::ImageConfig::default().magick_autodetect(true),
    );
    assert_eq!(
        driver()
            .dimensions(RED_PNG_1X1)
            .expect("the policy in effect on this host passes the framework's check"),
        (1, 1)
    );
}

/// The host grants of MVG and EPS the grant test starts its children
/// under: on the operator's `MAGICK_CONFIGURE_PATH`, in
/// `$XDG_CONFIG_HOME/ImageMagick`, the same marked stealth, and in the
/// account's own `~/.config/ImageMagick`.
#[cfg(unix)]
const HOST_GRANTS: [&str; 4] = ["operator", "user", "stealth", "home"];

/// The real binary the grant test's wrapper runs.
#[cfg(unix)]
fn driver_binary() -> String {
    driver().binary().to_string()
}

/// Where the grant test keeps its policies and what its children record.
#[cfg(unix)]
fn host_grant_scratch() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("hardening-magick-host-grant")
}

#[cfg(unix)]
#[test]
#[ignore = "requires a host ImageMagick 7 binary"]
fn hardening_a_host_grant_is_outranked_in_the_real_binary() {
    let scratch = host_grant_scratch();
    let _ = std::fs::remove_dir_all(&scratch);
    // MVG is the coder the driver's probe reads; EPS is one ImageMagick
    // chooses from the bytes on a bare `-`, so an autodetected run meets it.
    let grant = "<policymap>\n  <policy domain=\"coder\" rights=\"read|write\" pattern=\"{MVG,EPS}\" />\n</policymap>\n";
    // ImageMagick applies a stealth entry and leaves it out of
    // `magick -list policy`.
    let stealth = grant.replace(" />", " stealth=\"true\" />");
    let operator = scratch.join("operator");
    let user = scratch.join("user").join("ImageMagick");
    let hidden = scratch.join("stealth").join("ImageMagick");
    let home = scratch.join("home").join(".config").join("ImageMagick");
    for (dir, policy) in [
        (&operator, grant),
        (&user, grant),
        (&hidden, stealth.as_str()),
        (&home, grant),
    ] {
        std::fs::create_dir_all(dir).expect("create the policy directory");
        std::fs::write(dir.join("policy.xml"), policy).expect("write the grant");
    }
    for case in HOST_GRANTS {
        let output = {
            let _env = crate::env_lock::lock_env();
            let mut child = crate::own_process::child_command(
                "image_magick_driver::hardening_a_host_grant_is_outranked_in_the_real_binary_child",
            );
            child.env("HARDENING_MAGICK_GRANT", case);
            match case {
                // On the operator's configure path, between two reads of
                // the framework's directory.
                "operator" => child.env("MAGICK_CONFIGURE_PATH", &operator),
                // In the user's ImageMagick directory, which ImageMagick
                // reads after its own configuration directory and before
                // `$HOME/.config/ImageMagick`.
                "user" => child.env("XDG_CONFIG_HOME", scratch.join("user")),
                // The same, marked stealth, so the listing does not show it.
                "stealth" => child.env("XDG_CONFIG_HOME", scratch.join("stealth")),
                // In the account's own `~/.config/ImageMagick`, which the
                // driver's `HOME` takes the place of.
                _ => child.env("HOME", scratch.join("home")),
            };
            child.output().expect("the child runs")
        };
        crate::own_process::assert_child_passed(&output);
    }
}

/// Runs only in the child process the grant test starts, under a host
/// policy that grants the MVG and EPS coders, with autodetection on. The
/// driver's policy is the last ImageMagick reads, so it outranks the grant:
/// a named run proceeds, which under the opt-in means the listing passed
/// and the MVG probe was refused, and an EPS document on a bare `-` is
/// refused by ImageMagick's security policy. ImageMagick 7.1.2 does not
/// choose MVG from the bytes of a script on a bare `-`, so the test also
/// runs the real binary on an MVG script under the environment the driver
/// built, and sees its policy refuse it.
#[cfg(unix)]
#[test]
#[ignore = "requires a host ImageMagick 7 binary"]
fn hardening_a_host_grant_is_outranked_in_the_real_binary_child() {
    use std::os::unix::fs::PermissionsExt;
    if !crate::own_process::is_child() {
        return;
    }
    let case = std::env::var("HARDENING_MAGICK_GRANT").expect("the parent names the case");
    let scratch = host_grant_scratch();
    let recorded = scratch.join(format!("{case}-recorded"));
    std::fs::create_dir_all(&recorded).expect("create the record directory");
    let listing = recorded.join("listing");
    let home = recorded.join("home");
    let configure_path = recorded.join("configure-path");
    // A wrapper around the real binary that keeps what `-list policy`
    // printed, and the `HOME` and `MAGICK_CONFIGURE_PATH` it ran under, so
    // the test reads the order ImageMagick reads its policies in under the
    // environment the driver builds.
    let wrapper = recorded.join("magick-recording");
    std::fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\n\
             if [ \"$1\" = -list ] && [ \"$2\" = policy ]; then\n\
             printf '%s' \"$HOME\" > '{home}'\n\
             printf '%s' \"$MAGICK_CONFIGURE_PATH\" > '{configure_path}'\n\
             '{binary}' \"$@\" > '{listing}' || exit $?\n\
             exec cat '{listing}'\n\
             fi\n\
             exec '{binary}' \"$@\"\n",
            home = home.display(),
            configure_path = configure_path.display(),
            listing = listing.display(),
            binary = driver_binary(),
        ),
    )
    .expect("write the wrapper");
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755))
        .expect("make the wrapper executable");
    let driver = MagickCliDriver::new(wrapper.to_string_lossy());

    let _config = crate::image_processing::ConfigGuard::set(
        suprnova::ImageConfig::default().magick_autodetect(true),
    );
    assert_eq!(
        driver
            .dimensions(RED_PNG_1X1)
            .unwrap_or_else(|e| panic!("{case}: the grant is outranked and the run proceeds: {e}")),
        (1, 1)
    );
    let eps = b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 1 1\nshowpage\n";
    let err = driver
        .process(eps, &pipeline(Vec::new(), OutputFormat::Png))
        .expect_err("an EPS document on a bare `-` is refused")
        .to_string();
    assert!(
        err.contains("security policy"),
        "{case}: ImageMagick's security policy refuses the EPS document: {err}"
    );

    // The environment the driver built: ImageMagick refuses an MVG script
    // under it, whatever the host grants.
    let home = std::fs::read_to_string(&home).expect("the listing ran");
    let configure_path = std::fs::read_to_string(&configure_path).expect("the listing ran");
    let probe = {
        let mut child = Command::new(driver_binary())
            .args(["mvg:-", "info:"])
            .env("HOME", &home)
            .env("MAGICK_CONFIGURE_PATH", &configure_path)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("magick must run");
        std::io::Write::write_all(
            &mut child.stdin.take().expect("stdin"),
            include_bytes!("fixtures/magick-script.mvg"),
        )
        .expect("write the script");
        child.wait_with_output().expect("magick must finish")
    };
    let stderr = String::from_utf8_lossy(&probe.stderr);
    assert!(
        !probe.status.success()
            && stderr.contains("not authorized by the security policy")
            && stderr.contains("`MVG'"),
        "{case}: ImageMagick refuses MVG under the driver's environment: {stderr}"
    );

    // The last file ImageMagick read before its built-in policy is the
    // framework's, under the HOME the driver gave it.
    let framework = format!("{home}/.config/ImageMagick/policy.xml");
    let listing = std::fs::read_to_string(&listing).expect("read the listing");
    let paths: Vec<&str> = listing
        .lines()
        .filter_map(|line| line.strip_prefix("Path: "))
        .collect();
    assert_eq!(
        paths.last().copied(),
        Some("[built-in]"),
        "{case}: {paths:?}"
    );
    assert_eq!(
        paths.iter().rev().nth(1).copied(),
        Some(framework.as_str()),
        "{case}: the framework's file is the last ImageMagick reads: {paths:?}"
    );
    let named = |dir: std::path::PathBuf| {
        let file = dir.join("policy.xml");
        paths.contains(&file.to_string_lossy().as_ref())
    };
    match case.as_str() {
        "operator" => assert!(
            named(scratch.join("operator")),
            "{case}: the operator's file is read: {paths:?}"
        ),
        "user" => assert!(
            named(scratch.join("user").join("ImageMagick")),
            "{case}: the user's file is read: {paths:?}"
        ),
        // The stealth file holds no entry the listing shows, so it has no
        // `Path:` line of its own.
        "stealth" => {}
        "home" => assert!(
            !named(scratch.join("home").join(".config").join("ImageMagick")),
            "{case}: the account's own file is not read: {paths:?}"
        ),
        other => panic!("unknown case {other}"),
    }
}

/// Runs alone in a child process (see `own_process`). The facade's default
/// driver is installed once per process, by whichever test first reaches
/// the facade, and most tests in this binary go through the facade with the
/// default driver. Run alongside them (`--include-ignored`), this test
/// could not install the magick driver.
#[test]
#[ignore = "requires a host ImageMagick 7 binary"]
fn the_image_facade_drives_the_magick_driver() {
    crate::own_process::run_alone(
        "image_magick_driver::the_image_facade_drives_the_magick_driver_child",
    );
}

#[tokio::test]
async fn the_image_facade_drives_the_magick_driver_child() {
    if !crate::own_process::is_child() {
        return;
    }
    // Installed explicitly rather than through IMAGE_DRIVER, so the test does
    // not depend on process-global env ordering.
    suprnova::media::set_default_driver(Box::new(MagickCliDriver::from_env()))
        .expect("the child process must be the first to install a driver, or the test exercises OxideAV while claiming magick");

    let bytes = Image::from_bytes(RED_PNG_1X1)
        .resize(9, 3)
        .to_format(OutputFormat::Png)
        .to_bytes()
        .await
        .expect("pipeline");
    assert!(bytes.starts_with(b"\x89PNG"));
}

#[test]
#[ignore = "requires a host ImageMagick 7 binary"]
fn an_animation_reports_the_dimensions_of_its_first_frame() {
    // ImageMagick writes the fixture, because it does not read the GIFs
    // OxideAV's encoder writes, and this test is about the probe.
    let made = Command::new(driver().binary())
        .args([
            "-size", "100x100", "xc:red", "-size", "100x100", "xc:blue", "-loop", "0", "gif:-",
        ])
        .output()
        .expect("magick must run");
    assert!(made.status.success(), "magick must write the animation");
    let animation = made.stdout;

    // `-format` writes no separator between frames, so probing every frame
    // of a two-frame 100x100 GIF prints `100 100100 100`, which reads as a
    // height of 100100.
    assert_eq!(
        driver().dimensions(&animation).expect("dimensions"),
        (100, 100)
    );

    // The facade probes the processed output, and a GIF processed without a
    // conversion keeps every frame.
    let processed = driver()
        .process(&animation, &ImagePipeline::default())
        .expect("magick must re-encode the animation");
    assert_eq!(
        driver().dimensions(&processed).expect("dimensions"),
        (100, 100)
    );

    // A first frame smaller than its logical screen: the built-in driver
    // composes it onto the screen, so the answer is the screen, 100x80, not
    // the frame's own 40x30.
    let made = Command::new(driver().binary())
        .args([
            "-size",
            "40x30",
            "xc:red",
            "-set",
            "page",
            "100x80+10+10",
            "-size",
            "100x80",
            "xc:blue",
            "-set",
            "page",
            "100x80+0+0",
            "-loop",
            "0",
            "gif:-",
        ])
        .output()
        .expect("magick must run");
    assert!(made.status.success(), "magick must write the animation");
    assert_eq!(
        driver().dimensions(&made.stdout).expect("dimensions"),
        (100, 80)
    );
}

/// Run `magick identify -format <format> gif:-` over `gif` and return what it
/// printed, one entry per frame.
fn identify_frames(gif: &[u8], format: &str) -> Vec<String> {
    use std::io::Write;
    let mut child = Command::new(driver().binary())
        .args(["identify", "-format", format, "gif:-"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("magick must run");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(gif)
        .expect("write the GIF");
    let out = child.wait_with_output().expect("identify output");
    assert!(out.status.success(), "identify must read the GIF");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::to_string)
        .collect()
}

#[test]
#[ignore = "requires a host ImageMagick 7 binary"]
fn an_animation_is_processed_as_its_first_frame() {
    // The built-in driver decodes the first frame composed onto the logical
    // screen and nothing after it; the manual says the pipeline only uses
    // the first frame. So this driver must too, or a resize scales each
    // frame on its own and the output keeps every frame.
    let made = Command::new(driver().binary())
        .args([
            "-size",
            "40x30",
            "xc:red",
            "-set",
            "page",
            "100x80+10+10",
            "-size",
            "100x80",
            "xc:blue",
            "-set",
            "page",
            "100x80+0+0",
            "-loop",
            "0",
            "gif:-",
        ])
        .output()
        .expect("magick must run");
    assert!(made.status.success(), "magick must write the animation");

    let processed = driver()
        .process(
            &made.stdout,
            &pipeline(
                vec![Transformation::Resize {
                    width: 50,
                    height: 40,
                }],
                OutputFormat::Gif,
            ),
        )
        .expect("magick must process the animation");
    let frames = identify_frames(&processed, "%w %h %W %H %[pixel:p{15,12}]\n");
    assert_eq!(
        frames.len(),
        1,
        "only the first frame is processed: {frames:?}"
    );
    let first = &frames[0];
    assert!(
        first.starts_with("50 40 50 40 "),
        "the first frame, composed onto the 100x80 screen, then resized: {first}"
    );
    assert!(
        first.contains("red") || first.contains("255,0,0"),
        "the red first frame sits where it was placed: {first}"
    );
}

#[test]
#[ignore = "requires a host ImageMagick 7 binary"]
fn the_default_drivers_gif_output_reads_back_through_imagemagick() {
    use std::io::Write;

    // The built-in driver's GIF output, read by ImageMagick and by the
    // built-in driver itself, gives the same pixels.
    let oxideav = suprnova::OxideAvImageDriver::new();
    let gif = oxideav
        .process(&photo_bmp(), &pipeline(Vec::new(), OutputFormat::Gif))
        .expect("the built-in driver writes a GIF");
    let mut child = Command::new(driver().binary())
        .args(["gif:-", "-depth", "8", "rgba:-"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("magick must run");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(&gif)
        .expect("write the GIF");
    let out = child.wait_with_output().expect("magick output");
    assert!(
        out.status.success(),
        "ImageMagick must read the GIF: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let bmp = oxideav
        .process(&gif, &pipeline(Vec::new(), OutputFormat::Bmp))
        .expect("the built-in driver reads its own GIF");
    let ours = crate::image_processing::bmp_rgba_pixels(&bmp);
    assert_eq!(out.stdout.len(), ours.len(), "both read a 97x65 image");
    assert!(out.stdout == ours, "ImageMagick reads the same pixels");
}
