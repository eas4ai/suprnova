//! `live:assets` publishes the helper's reviewed bytes atomically and refuses drift.

mod live_support;

use std::fs;
use std::io::Cursor;
use std::path::Path;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use live_support::{
    ASSET_FILES, IDENTITY, asset, asset_body, asset_stream, begin, combined, end_ok, envelope,
    fake_project, run_cli, sha256_hex,
};
use suprnova_cli::commands::live_tool::{MAX_ASSETS, Operation, consume};

fn published(out: &Path) -> Vec<(String, Vec<u8>)> {
    let dir = out.join(IDENTITY);
    let mut entries: Vec<_> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .map(|entry| {
            let entry = entry.expect("entry");
            (
                entry.file_name().to_string_lossy().into_owned(),
                fs::read(entry.path()).expect("file"),
            )
        })
        .collect();
    entries.sort();
    entries
}

fn expected() -> Vec<(String, Vec<u8>)> {
    let mut files: Vec<_> = ASSET_FILES
        .iter()
        .map(|(_, file, bytes)| ((*file).to_owned(), bytes.to_vec()))
        .collect();
    files.sort();
    files
}

#[test]
fn publication_is_exact_atomic_and_idempotent() {
    let out = "out-exact";
    let first = run_cli(&["live:assets", "--out", out], &asset_stream(), 0);
    let text = combined(&first);
    assert_eq!(first.status.code(), Some(0), "{text}");
    assert!(text.contains(IDENTITY), "{text}");
    let root = fake_project().join(out);
    assert_eq!(published(&root), expected());
    let leftovers: Vec<_> = fs::read_dir(&root)
        .expect("out")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .filter(|name| name != IDENTITY)
        .collect();
    assert!(leftovers.is_empty(), "no staging leftovers: {leftovers:?}");

    let second = run_cli(&["live:assets", "--out", out], &asset_stream(), 0);
    let text = combined(&second);
    assert_eq!(second.status.code(), Some(0), "{text}");
    assert!(text.contains("up to date"), "{text}");
    assert_eq!(published(&root), expected());
}

#[test]
fn drift_is_refused_unless_replacement_is_requested() {
    let out = "out-drift";
    assert!(
        run_cli(&["live:assets", "--out", out], &asset_stream(), 0)
            .status
            .success()
    );
    let root = fake_project().join(out);
    let tampered = root.join(IDENTITY).join("suprnova-live.esm.js");
    fs::write(&tampered, b"tampered\n").expect("tamper");

    let refused = run_cli(&["live:assets", "--out", out], &asset_stream(), 0);
    let text = combined(&refused);
    assert_eq!(refused.status.code(), Some(1), "{text}");
    assert!(text.contains("--replace"), "{text}");
    assert_eq!(fs::read(&tampered).expect("read"), b"tampered\n");

    let replaced = run_cli(
        &["live:assets", "--out", out, "--replace"],
        &asset_stream(),
        0,
    );
    let text = combined(&replaced);
    assert_eq!(replaced.status.code(), Some(0), "{text}");
    assert_eq!(published(&root), expected());
    let leftovers: Vec<_> = fs::read_dir(&root)
        .expect("out")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .filter(|name| name != IDENTITY)
        .collect();
    assert!(
        leftovers.is_empty(),
        "replacement leaves nothing behind: {leftovers:?}"
    );
}

#[test]
fn a_digest_mismatch_writes_nothing() {
    let out = "out-digest";
    let mut stream = begin("assets");
    let body = asset_body(
        "artifact",
        "suprnova-live.esm.js",
        "text/javascript; charset=utf-8",
        b"real\n",
    )
    .replace(&sha256_hex(b"real\n"), &sha256_hex(b"fake\n"));
    stream.push_str(&envelope(1, "assets", &body));
    stream.push_str(&end_ok(2, "assets"));
    let output = run_cli(&["live:assets", "--out", out], &stream, 0);
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(1), "{text}");
    assert!(text.to_lowercase().contains("digest"), "{text}");
    assert!(!fake_project().join(out).exists());
}

#[test]
fn unsafe_output_locations_are_refused() {
    for out in ["../escape", "/tmp/suprnova-live-absolute"] {
        let output = run_cli(&["live:assets", "--out", out], &asset_stream(), 0);
        assert_eq!(
            output.status.code(),
            Some(1),
            "{out}: {}",
            combined(&output)
        );
    }
    #[cfg(unix)]
    {
        let outside = tempfile::tempdir().expect("outside");
        let link = fake_project().join("out-link");
        let _ = fs::remove_file(&link);
        std::os::unix::fs::symlink(outside.path(), &link).expect("symlink");
        let output = run_cli(&["live:assets", "--out", "out-link"], &asset_stream(), 0);
        let text = combined(&output);
        assert_eq!(output.status.code(), Some(1), "{text}");
        assert!(text.contains("symlink"), "{text}");
        assert!(
            fs::read_dir(outside.path())
                .expect("outside")
                .next()
                .is_none()
        );
    }
}

#[test]
fn asset_envelopes_are_validated_on_the_transport() {
    let ok = consume(Cursor::new(asset_stream().as_bytes()), Operation::Assets).expect("valid");
    assert_eq!(ok.assets_out.len(), 3);
    assert_eq!(ok.assets_out[1].file, "suprnova-live.esm.js");
    assert_eq!(ok.assets_out[1].bytes, b"export const live = 1;\n");

    let cases: Vec<(String, &str)> = vec![
        (
            format!(
                "{}{}{}",
                begin("assets"),
                asset(1, "artifact", "../evil.js", "text/javascript", b"x"),
                end_ok(2, "assets")
            ),
            "file name",
        ),
        (
            format!(
                "{}{}{}",
                begin("assets"),
                asset(1, "artifact", ".hidden.js", "text/javascript", b"x"),
                end_ok(2, "assets")
            ),
            "file name",
        ),
        (
            format!(
                "{}{}{}",
                begin("assets"),
                envelope(
                    1,
                    "assets",
                    &asset_body("artifact", "a.js", "text/javascript", b"abc")
                        .replace("\"bytes\":3", "\"bytes\":4")
                ),
                end_ok(2, "assets")
            ),
            "length",
        ),
        (
            format!(
                "{}{}{}",
                begin("assets"),
                envelope(
                    1,
                    "assets",
                    &asset_body("artifact", "a.js", "text/javascript", b"abc")
                        .replace(&BASE64.encode(b"abc"), "not*base64")
                ),
                end_ok(2, "assets")
            ),
            "base64",
        ),
        (
            format!(
                "{}{}{}{}",
                begin("assets"),
                asset(1, "artifact", "a.js", "text/javascript", b"x"),
                asset(2, "artifact", "a.js", "text/javascript", b"x"),
                end_ok(3, "assets")
            ),
            "duplicate",
        ),
        (
            {
                let mut stream = begin("assets");
                for index in 0..=MAX_ASSETS as u32 {
                    stream.push_str(&asset(
                        index + 1,
                        "artifact",
                        &format!("a{index}.js"),
                        "text/javascript",
                        b"x",
                    ));
                }
                stream.push_str(&end_ok(MAX_ASSETS as u32 + 2, "assets"));
                stream
            },
            "asset",
        ),
    ];
    for (stream, expected) in cases {
        let error = consume(Cursor::new(stream.as_bytes()), Operation::Assets)
            .expect_err(&format!("must fail closed: {expected}"));
        assert!(
            error.to_string().to_lowercase().contains(expected),
            "{error} mentions {expected}"
        );
    }
}
