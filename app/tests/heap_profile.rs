//! MEM-007: heap profiling an administrator switches on with a feature.
//!
//! With `--features heap-profiling` the framework installs dhat's
//! allocator, and a command that runs and exits writes its profile to the
//! file `SUPRNOVA_HEAP_PROFILE` names. The workspace offers the
//! `profiling` Cargo profile for samply or perf; the scaffolds' copy is
//! checked in `suprnova-cli`.

/// The workspace's `profiling` profile is the release profile with debug
/// symbols.
#[test]
fn mem_audit_the_workspace_offers_a_profiling_profile() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../Cargo.toml");
    let manifest = std::fs::read_to_string(path).expect("the workspace manifest");
    let manifest: toml::Table = toml::from_str(&manifest).expect("the workspace manifest parses");
    let profile = manifest
        .get("profile")
        .and_then(|p| p.get("profiling"))
        .expect("the workspace has no profiling profile");
    assert_eq!(
        profile.get("inherits").and_then(toml::Value::as_str),
        Some("release")
    );
    assert_eq!(
        profile.get("debug").and_then(toml::Value::as_bool),
        Some(true)
    );
}

/// A console command that runs and exits writes a DHAT profile with the
/// totals: bytes allocated, the heap at its peak and the heap at the end.
#[cfg(feature = "heap-profiling")]
#[test]
fn mem_audit_a_command_writes_a_heap_profile() {
    let tmp = tempfile::TempDir::new().expect("tmpdir");
    let profile = tmp.path().join("heap.json");
    let run = |bin: &str, args: &[&str]| {
        std::process::Command::new(bin)
            .args(args)
            .env(
                "DATABASE_URL",
                format!("sqlite://{}?mode=rwc", tmp.path().join("heap.db").display()),
            )
            .env("APP_ENV", "testing")
            .env("LOG_LEVEL", "warn")
            .env("SUPRNOVA_HEAP_PROFILE", &profile)
            // A `.env` in the app directory would otherwise override
            // every variable set here.
            .current_dir(tmp.path())
            .output()
            .expect("spawn the binary")
    };
    // The bootstrap reads the `features` table, so the database is
    // migrated first. That run writes a profile of its own, which is
    // removed so the file read below can only be the command's.
    let migrated = run(env!("CARGO_BIN_EXE_app"), &["migrate"]);
    assert!(
        migrated.status.success(),
        "`app migrate` failed:\n{}",
        String::from_utf8_lossy(&migrated.stderr)
    );
    std::fs::remove_file(&profile).expect("`app migrate` wrote a heap profile too");

    let output = run(env!("CARGO_BIN_EXE_console"), &["greet"]);
    assert!(
        output.status.success(),
        "`console greet` failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json = std::fs::read_to_string(&profile).expect("the command wrote no heap profile");
    let json: serde_json::Value = serde_json::from_str(&json).expect("the profile is JSON");
    assert_eq!(json["dhatFileVersion"], 2);
    assert_eq!(json["mode"], "rust-heap");
    let points = json["pps"]
        .as_array()
        .expect("the profile has program points");
    let sum = |field: &str| -> u64 {
        points
            .iter()
            .map(|point| point[field].as_u64().expect("a byte total"))
            .sum()
    };
    assert!(sum("tb") > 0, "the profile counts no bytes allocated");
    assert!(sum("gb") > 0, "the profile has no peak heap");
    assert!(
        points.iter().all(|point| point["eb"].is_u64()),
        "the profile has no heap-at-end totals"
    );
}
