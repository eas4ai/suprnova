//! The app binary's `schema:dump`, `migrate`, `migrate:fresh` and `serve`
//! with a schema dump (PAR-038 to PAR-040), run as the real process. The
//! framework's tests drive `SchemaDump` directly; these prove the
//! subcommands reach it, and that `serve` stops on a failed load even in
//! best-effort mode.

#![cfg(unix)]

use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

const APP_BIN: &str = env!("CARGO_BIN_EXE_app");

/// The app binary in `dir` against the SQLite file `app.sqlite` there.
/// The directory is the base path, so `database/schema` and
/// `src/migrations` are its own; a `.env` elsewhere is never read.
fn app(dir: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new(APP_BIN);
    cmd.args(args)
        .env(
            "DATABASE_URL",
            format!("sqlite://{}?mode=rwc", dir.join("app.sqlite").display()),
        )
        .env("APP_ENV", "testing")
        .env("APP_DEBUG", "false")
        .env("LOG_LEVEL", "warn")
        .env_remove("SUPRNOVA_AUTO_MIGRATE_BEST_EFFORT")
        .current_dir(dir);
    cmd
}

fn run(dir: &Path, args: &[&str]) -> String {
    let out = app(dir, args)
        .output()
        .unwrap_or_else(|e| panic!("spawn {args:?}: {e}"));
    let text = combined(&out);
    assert!(out.status.success(), "{args:?} failed:\n{text}");
    text
}

fn combined(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// The ledger versions a dump records.
fn ledger(dump: &str) -> Vec<String> {
    dump.lines()
        .filter(|line| line.starts_with("INSERT"))
        .filter_map(|line| {
            line.split_once("VALUES ('")?
                .1
                .split_once('\'')
                .map(|(v, _)| v.to_owned())
        })
        .collect()
}

#[test]
fn the_app_binary_dumps_loads_and_prunes() {
    let first = tempfile::tempdir().expect("a temporary directory");
    run(first.path(), &["migrate"]);
    let dumped = run(first.path(), &["schema:dump"]);
    let dump_path = first.path().join("database/schema/sqlite-schema.sql");
    assert!(dumped.contains("Database schema dumped"), "{dumped}");
    let dump = std::fs::read_to_string(&dump_path).expect("the default dump");
    let versions = ledger(&dump);
    assert!(
        !versions.is_empty(),
        "the dump records the app's migrations"
    );

    // An empty database finds the dump at its default path and loads it.
    let second = tempfile::tempdir().expect("a temporary directory");
    std::fs::create_dir_all(second.path().join("database/schema")).expect("mkdir");
    std::fs::copy(
        &dump_path,
        second.path().join("database/schema/sqlite-schema.sql"),
    )
    .expect("copy");
    let migrated = run(second.path(), &["migrate"]);
    assert!(migrated.contains("Loaded the schema dump"), "{migrated}");
    let fresh = run(
        second.path(),
        &[
            "migrate:fresh",
            "--schema-path",
            &dump_path.display().to_string(),
        ],
    );
    assert!(fresh.contains("Loaded the schema dump"), "{fresh}");

    // --prune edits the migrations under the app's own base path.
    let migrations = first.path().join("src/migrations");
    std::fs::create_dir_all(&migrations).expect("mkdir");
    let mut module = String::from("pub use sea_orm_migration::prelude::*;\n\n");
    for version in &versions {
        std::fs::write(migrations.join(format!("{version}.rs")), "// a migration\n")
            .expect("write");
        module.push_str(&format!("mod {version};\n"));
    }
    module.push_str("\npub struct Migrator;\n\n#[async_trait::async_trait]\nimpl MigratorTrait for Migrator {\n    fn migrations() -> Vec<Box<dyn MigrationTrait>> {\n        vec![\n");
    for version in &versions {
        module.push_str(&format!("            Box::new({version}::Migration),\n"));
    }
    module.push_str("        ]\n    }\n}\n");
    std::fs::write(migrations.join("mod.rs"), module).expect("write mod.rs");
    let pruned = run(first.path(), &["schema:dump", "--prune"]);
    assert!(
        pruned.contains(&format!("Pruned {} migration(s)", versions.len())),
        "{pruned}"
    );
    let module = std::fs::read_to_string(migrations.join("mod.rs")).expect("read mod.rs");
    for version in &versions {
        assert!(
            !migrations.join(format!("{version}.rs")).exists(),
            "{version}"
        );
        assert!(
            module.contains(&format!("PrunedMigration::new(\"{version}\")")),
            "{module}"
        );
    }
}

#[test]
fn a_broken_dump_stops_serve_even_in_best_effort_mode() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    std::fs::create_dir_all(dir.path().join("database/schema")).expect("mkdir");
    std::fs::write(
        dir.path().join("database/schema/sqlite-schema.sql"),
        "THIS IS NOT SQL;\n",
    )
    .expect("write a broken dump");
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .expect("a free port")
        .local_addr()
        .expect("its address")
        .port();
    let mut child = app(dir.path(), &["serve"])
        .env("SUPRNOVA_AUTO_MIGRATE_BEST_EFFORT", "true")
        .env("SERVER_HOST", "127.0.0.1")
        .env("SERVER_PORT", port.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn serve");
    let deadline = Instant::now() + Duration::from_secs(60);
    let status = loop {
        if let Some(status) = child.try_wait().expect("poll serve") {
            break status;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("serve booted past a schema dump that failed to load");
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let mut stderr = String::new();
    std::io::Read::read_to_string(&mut child.stderr.take().expect("piped stderr"), &mut stderr)
        .expect("read stderr");
    // Without the load failing closed, best-effort mode logs the failure
    // and goes on to boot, which then dies for want of tables.
    assert!(
        !stderr.contains("best-effort mode"),
        "a failed load is not a best-effort migration failure:\n{stderr}"
    );
    assert!(stderr.contains("schema dump"), "{stderr}");
    assert!(!status.success(), "serve exits with an error");
}
