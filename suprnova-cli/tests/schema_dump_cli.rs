//! `suprnova schema:dump` runs the app binary's `schema:dump`, and the
//! migrate commands pass `--schema-path` on (PAR-038 to PAR-040). A fake
//! `cargo` first on PATH writes the arguments it was given.

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

use tempfile::{TempDir, tempdir};

const BIN: &str = env!("CARGO_BIN_EXE_suprnova");

/// A project with a migrations directory and a `cargo` that records its
/// arguments, one per line.
fn project() -> TempDir {
    let dir = tempdir().expect("create tempdir");
    fs::create_dir_all(dir.path().join("src/migrations")).expect("mkdir src/migrations");
    let bin = dir.path().join("fakebin");
    fs::create_dir_all(&bin).expect("mkdir fakebin");
    let shim = bin.join("cargo");
    fs::write(
        &shim,
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$CARGO_ARGS_LOG\"\nexit 0\n",
    )
    .expect("write cargo shim");
    fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).expect("chmod shim");
    dir
}

/// The arguments `cargo` received after `--` when the CLI ran `args`.
fn app_arguments(dir: &TempDir, args: &[&str]) -> Vec<String> {
    let log = dir.path().join("cargo-args.log");
    let path = format!(
        "{}:{}",
        dir.path().join("fakebin").display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let out = Command::new(BIN)
        .args(args)
        .env("PATH", path)
        .env("APP_ENV", "local")
        .env("CARGO_ARGS_LOG", &log)
        .current_dir(dir.path())
        .output()
        .expect("spawn suprnova");
    assert!(
        out.status.success(),
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let logged = fs::read_to_string(&log).expect("cargo ran");
    let all: Vec<String> = logged.lines().map(str::to_owned).collect();
    let after = all
        .iter()
        .position(|a| a == "--")
        .expect("a `--` before the app's arguments");
    all[after + 1..].to_vec()
}

#[test]
fn schema_dump_runs_the_app_binarys_command() {
    let dir = project();
    assert_eq!(app_arguments(&dir, &["schema:dump"]), ["schema:dump"]);
    assert_eq!(
        app_arguments(&dir, &["schema:dump", "--prune", "--path", "db/schema.sql"]),
        ["schema:dump", "--path", "db/schema.sql", "--prune"]
    );
}

#[test]
fn migrate_passes_the_schema_path_on() {
    let dir = project();
    assert_eq!(
        app_arguments(&dir, &["migrate", "--schema-path", "db/schema.sql"]),
        ["migrate", "--schema-path", "db/schema.sql"]
    );
    assert_eq!(app_arguments(&dir, &["migrate"]), ["migrate"]);
}

#[test]
fn migrate_fresh_passes_the_schema_path_on() {
    let dir = project();
    assert_eq!(
        app_arguments(&dir, &["migrate:fresh", "--schema-path", "db/schema.sql"]),
        ["migrate:fresh", "--schema-path", "db/schema.sql"]
    );
}

/// The scaffolded images auto-migrate, so they carry the schema dumps and
/// a client that can load one.
#[test]
fn the_docker_images_carry_the_dumps_and_a_client() {
    for (name, template) in [
        (
            "Dockerfile.tpl",
            include_str!("../src/templates/files/docker/Dockerfile.tpl"),
        ),
        (
            "Dockerfile.api.tpl",
            include_str!("../src/templates/files/docker/Dockerfile.api.tpl"),
        ),
    ] {
        let runtime = template
            .split("AS runtime")
            .nth(1)
            .unwrap_or_else(|| panic!("{name} has a runtime stage"));
        assert!(
            runtime.contains("./database/schema"),
            "{name}'s runtime stage copies database/schema"
        );
        assert!(
            runtime.contains("ARG DB_CLIENT=postgresql-client") && runtime.contains("${DB_CLIENT}"),
            "{name}'s runtime stage installs the database client"
        );
    }
}
