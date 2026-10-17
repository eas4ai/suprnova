//! `suprnova db:sync` reports filesystem / subprocess / runtime / database
//! failures as clean user-facing errors (`ui::error` + exit code 1) instead
//! of aborting with a Rust panic backtrace.
//!
//! Print a message via `ui::error` and `std::process::exit(1)` is the
//! contract for every failure path in db_sync.rs.
//!
//! Teeth: against the original code the blocked-entities-dir path executed
//! `fs::create_dir_all(...).expect("Failed to create entities directory")`,
//! so the process aborted with `thread 'main' panicked at ...` and a
//! backtrace. The assertions below require a non-zero exit, a human-readable
//! message, AND the absence of `"panicked"` - the last is what proves the
//! panic became a user-facing error.

use std::fs;
use std::process::{Command, Output};

use sea_orm::{ConnectionTrait, Database};
use tempfile::tempdir;

const BIN: &str = env!("CARGO_BIN_EXE_suprnova");

/// Combined stdout + stderr, since the `ui` helpers may write to either stream.
fn combined(out: &Output) -> String {
    let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
    s.push_str(&String::from_utf8_lossy(&out.stderr));
    s
}

#[test]
fn db_sync_outside_a_project_exits_with_clean_error_not_panic() {
    let dir = tempdir().expect("create tempdir");
    let out = Command::new(BIN)
        .arg("db:sync")
        .current_dir(dir.path())
        .output()
        .expect("spawn suprnova binary");

    let text = combined(&out);
    assert_eq!(
        out.status.code(),
        Some(1),
        "db:sync outside a project must exit 1; output: {text}"
    );
    assert!(
        text.contains("Not in a Suprnova project"),
        "must print a user-facing message; got: {text}"
    );
    assert!(!text.contains("panicked"), "must NOT panic; got: {text}");
}

#[test]
fn db_sync_reports_clean_error_when_entities_dir_cannot_be_created() {
    let dir = tempdir().expect("create tempdir");
    let root = dir.path();

    // A sqlite database with one user table, so schema discovery finds work to
    // do and proceeds to the entity-file generation step (an empty DB would
    // short-circuit at "No tables found").
    let db_path = root.join("schema.db");
    let rt = tokio::runtime::Runtime::new().expect("build tokio runtime");
    rt.block_on(async {
        let db = Database::connect(format!("sqlite://{}?mode=rwc", db_path.display()))
            .await
            .expect("connect to sqlite");
        db.execute_unprepared("CREATE TABLE widgets (id INTEGER PRIMARY KEY, name TEXT)")
            .await
            .expect("create widgets table");
    });

    // Stage `src/models` as a FILE: it exists (so we are "in a project" and the
    // `src/models` create is skipped), but creating `src/models/entities/` then
    // fails because its parent is a regular file. The blocked-entities-dir
    // failure must surface as a clean error, not a panic backtrace.
    fs::create_dir_all(root.join("src")).expect("mkdir src");
    fs::write(root.join("src").join("models"), "not a directory").expect("stage src/models file");

    let out = Command::new(BIN)
        .arg("db:sync")
        .env("DATABASE_URL", format!("sqlite://{}", db_path.display()))
        .current_dir(root)
        .output()
        .expect("spawn suprnova binary");

    let text = combined(&out);
    assert_eq!(
        out.status.code(),
        Some(1),
        "a blocked entities directory must exit 1; output: {text}"
    );
    assert!(
        text.contains("Failed to create entities directory"),
        "must print a user-facing filesystem error; got: {text}"
    );
    assert!(
        !text.contains("panicked"),
        "the filesystem failure must NOT panic; got: {text}"
    );
}

#[test]
fn db_sync_missing_database_url_exits_clean() {
    let dir = tempdir().expect("create tempdir");
    let root = dir.path();

    // Make this a "Suprnova project" (src/models present) so we sail past the
    // project-detection guard and hit `env::var("DATABASE_URL")`.  We isolate
    // DATABASE_URL away - both as a process env var and by writing an empty
    // .env so dotenvy can't backfill it from anywhere.
    fs::create_dir_all(root.join("src/models")).expect("mkdir src/models");
    fs::write(root.join(".env"), "").expect("write empty .env");

    let out = Command::new(BIN)
        .arg("db:sync")
        .arg("--skip-migrations")
        .env_remove("DATABASE_URL")
        .current_dir(root)
        .output()
        .expect("spawn suprnova binary");

    let text = combined(&out);
    assert_eq!(
        out.status.code(),
        Some(1),
        "missing DATABASE_URL must exit 1; output: {text}"
    );
    assert!(
        text.contains("DATABASE_URL not set"),
        "must print a user-facing DATABASE_URL error; got: {text}"
    );
    assert!(!text.contains("panicked"), "must NOT panic; got: {text}");
}

#[test]
fn db_sync_unreachable_database_exits_clean() {
    let dir = tempdir().expect("create tempdir");
    let root = dir.path();

    // "In a Suprnova project" so we get past the directory guard, no
    // migrations directory so `--skip-migrations` isn't even needed but we
    // pass it anyway for determinism.
    fs::create_dir_all(root.join("src/models")).expect("mkdir src/models");

    // Point DATABASE_URL at a sqlite file in a directory that does NOT exist -
    // `Database::connect` then fails on the open call, and we need that to
    // surface as a clean error.
    let unreachable = root.join("does-not-exist-dir").join("nope.db");

    let out = Command::new(BIN)
        .arg("db:sync")
        .arg("--skip-migrations")
        .env(
            "DATABASE_URL",
            format!("sqlite://{}", unreachable.display()),
        )
        .current_dir(root)
        .output()
        .expect("spawn suprnova binary");

    let text = combined(&out);
    assert_eq!(
        out.status.code(),
        Some(1),
        "unreachable database must exit 1; output: {text}"
    );
    assert!(
        text.contains("Failed to connect to database"),
        "must print a user-facing connect error; got: {text}"
    );
    assert!(!text.contains("panicked"), "must NOT panic; got: {text}");
}

#[test]
fn db_sync_unreadable_models_mod_exits_clean() {
    // Cover the previously-silent fs::read_to_string fallback in
    // update_models_mod: a permission-blocked `src/models/mod.rs` used to be
    // swallowed by `.unwrap_or_default()`, then overwritten on the next
    // `fs::write` - silently destroying the user's customizations.  Now it
    // must surface as a clean error.
    use std::os::unix::fs::PermissionsExt;

    let dir = tempdir().expect("create tempdir");
    let root = dir.path();
    let db_path = root.join("schema.db");

    let rt = tokio::runtime::Runtime::new().expect("build tokio runtime");
    rt.block_on(async {
        let db = Database::connect(format!("sqlite://{}?mode=rwc", db_path.display()))
            .await
            .expect("connect to sqlite");
        db.execute_unprepared("CREATE TABLE widgets (id INTEGER PRIMARY KEY, name TEXT)")
            .await
            .expect("create widgets table");
    });

    fs::create_dir_all(root.join("src/models/entities")).expect("mkdir entities");
    let mod_path = root.join("src/models/mod.rs");
    fs::write(&mod_path, "//! Application models\n\npub mod custom;\n").expect("seed mod.rs");

    // Strip read permission so fs::read_to_string fails. If we're root the
    // OS ignores this (root bypasses DAC) and the test is a no-op assertion
    // on a clean run - skip cleanly in that case to keep CI portable.
    let mut perms = fs::metadata(&mod_path).expect("metadata").permissions();
    perms.set_mode(0o000);
    fs::set_permissions(&mod_path, perms).expect("set perms");
    let still_readable = fs::read_to_string(&mod_path).is_ok();
    if still_readable {
        // running as root or on a filesystem that doesn't honor the mode bits
        return;
    }

    let out = Command::new(BIN)
        .arg("db:sync")
        .arg("--skip-migrations")
        .env("DATABASE_URL", format!("sqlite://{}", db_path.display()))
        .current_dir(root)
        .output()
        .expect("spawn suprnova binary");

    // Restore perms so tempdir cleanup can drop the file.
    let mut restore = fs::metadata(&mod_path).expect("metadata").permissions();
    restore.set_mode(0o644);
    let _ = fs::set_permissions(&mod_path, restore);

    let text = combined(&out);
    assert_eq!(
        out.status.code(),
        Some(1),
        "unreadable mod.rs must exit 1; output: {text}"
    );
    assert!(
        text.contains("Failed to read existing models/mod.rs"),
        "must surface the read failure; got: {text}"
    );
    assert!(!text.contains("panicked"), "must NOT panic; got: {text}");
}

#[test]
fn db_sync_rejects_mysql_instead_of_running_postgres_sql() {
    // db_sync.rs:78 classified every non-sqlite URL as Postgres, so a
    // mysql:// URL issued Postgres information_schema statements against
    // MySQL. Failing clearly beats failing confusingly.
    let dir = tempdir().expect("create tempdir");
    let root = dir.path();
    fs::create_dir_all(root.join("src/models/entities")).expect("create project layout");
    fs::write(root.join("Cargo.toml"), "[package]\nname = \"probe\"\n").expect("write manifest");

    let out = Command::new(BIN)
        .arg("db:sync")
        .env("DATABASE_URL", "mysql://user:pass@127.0.0.1:3306/probe")
        .current_dir(root)
        .output()
        .expect("spawn suprnova binary");

    let text = combined(&out);
    assert_eq!(
        out.status.code(),
        Some(1),
        "mysql:// must exit 1 rather than run Postgres SQL; output: {text}"
    );
    assert!(
        text.contains("MySQL") || text.contains("mysql"),
        "error must name the unsupported backend; got: {text}"
    );
    assert!(!text.contains("panicked"), "must NOT panic; got: {text}");
}

/// P4-06: every current scaffold registers its migrator in the default
/// application binary - there is no `src/bin/migrate.rs` anymore. A
/// project in the current layout must still run its migrations before
/// discovery instead of warning and introspecting a stale database.
///
/// Teeth: with the obsolete marker check in place, this fixture warns
/// "Migration binary not found, skipping migrations" and exits 0 after
/// discovering the stale schema. The assertions below require the
/// migrate invocation to be attempted (and, in this minimal fixture
/// whose package has no runnable binary, to fail loudly rather than
/// silently sync stale models).
#[test]
fn db_sync_runs_migrations_for_current_layout_without_legacy_migrate_binary() {
    let dir = tempdir().expect("create tempdir");
    let root = dir.path();
    // Current scaffold layout: migrations live in `src/migrations`, the
    // migrator is registered in the default application binary, and no
    // legacy `src/bin/migrate.rs` exists.
    fs::create_dir_all(root.join("src/models/entities")).expect("create project layout");
    fs::create_dir_all(root.join("src/migrations")).expect("create migrations dir");
    fs::write(root.join("Cargo.toml"), "[package]\nname = \"probe\"\n").expect("write manifest");

    let out = Command::new(BIN)
        .arg("db:sync")
        .env(
            "DATABASE_URL",
            format!("sqlite://{}/schema.db?mode=rwc", root.display()),
        )
        .current_dir(root)
        .output()
        .expect("spawn suprnova binary");

    let text = combined(&out);
    assert!(
        !text.contains("skipping migrations"),
        "current-layout projects must not skip migrations; got: {text}"
    );
    assert_eq!(
        out.status.code(),
        Some(1),
        "the migrate invocation must be attempted (and fail loudly in this \
         binary-less fixture) instead of syncing stale models; output: {text}"
    );
    assert!(
        text.contains("Migration failed") || text.contains("Failed to execute"),
        "the failure must name the migrate step; got: {text}"
    );
    assert!(!text.contains("panicked"), "must NOT panic; got: {text}");
}

/// The schema is untrusted input. A table name is used both as a path
/// component (`src/models/entities/<name>.rs`) and inside generated Rust, so
/// `db:sync` must refuse names that escape the entity directory - while still
/// syncing the well-named tables sitting next to them.
///
/// Teeth: with the `SafeName`/`contained_path` guards removed, the hostile
/// table below writes `<root>/src/pwned.rs` (two levels up from
/// `src/models/entities/`) and the hostile column lands `pub struct Evil` in
/// the generated entity. Both assertions below fail in that state.
#[test]
fn db_sync_refuses_hostile_schema_names_but_syncs_the_rest() {
    let dir = tempdir().expect("create tempdir");
    let root = dir.path();
    let db_path = root.join("schema.db");

    let rt = tokio::runtime::Runtime::new().expect("build tokio runtime");
    rt.block_on(async {
        let db = Database::connect(format!("sqlite://{}?mode=rwc", db_path.display()))
            .await
            .expect("connect to sqlite");
        // A perfectly ordinary table that must still be generated.
        db.execute_unprepared("CREATE TABLE widgets (id INTEGER PRIMARY KEY, name TEXT)")
            .await
            .expect("create widgets table");
        // Path traversal: relative to src/models/entities/, this is
        // <root>/src/pwned.rs.
        db.execute_unprepared(r#"CREATE TABLE "../../pwned" (id INTEGER PRIMARY KEY)"#)
            .await
            .expect("create traversal table");
        // Source injection through a column name.
        db.execute_unprepared(
            r#"CREATE TABLE gadgets (id INTEGER PRIMARY KEY, "x TEXT, } pub struct Evil { pub y" TEXT)"#,
        )
        .await
        .expect("create injected-column table");
        // A name whose *derived* struct name is the reserved `Self`.
        db.execute_unprepared("CREATE TABLE selfs (id INTEGER PRIMARY KEY)")
            .await
            .expect("create selfs table");
    });

    fs::create_dir_all(root.join("src/models/entities")).expect("create project layout");

    let out = Command::new(BIN)
        .arg("db:sync")
        .arg("--skip-migrations")
        .env("DATABASE_URL", format!("sqlite://{}", db_path.display()))
        .current_dir(root)
        .output()
        .expect("spawn suprnova binary");

    let text = combined(&out);
    assert_eq!(
        out.status.code(),
        Some(0),
        "the well-named table must still sync; output: {text}"
    );

    // Nothing escaped the entity directory.
    assert!(
        !root.join("src/pwned.rs").exists(),
        "a traversing table name wrote outside src/models/entities: {text}"
    );
    let stray: Vec<_> = fs::read_dir(root.join("src"))
        .expect("read src")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n != "models")
        .collect();
    assert!(stray.is_empty(), "unexpected files under src/: {stray:?}");

    // The good table generated normally...
    let entity = fs::read_to_string(root.join("src/models/entities/widgets.rs"))
        .expect("widgets entity must exist");
    assert!(
        entity.contains("pub name"),
        "column discovery must still work; entity was:\n{entity}"
    );
    assert!(
        entity.contains(r#"table_name = "widgets""#),
        "the table_name attribute must be present; entity was:\n{entity}"
    );

    // ...and the hostile ones were skipped, loudly.
    assert!(
        !root.join("src/models/entities/gadgets.rs").exists(),
        "a table with an unusable column must be skipped whole: {text}"
    );
    assert!(
        !root.join("src/models/entities/selfs.rs").exists(),
        "a table whose derived struct name is `Self` must be skipped: {text}"
    );
    assert!(
        text.contains("Skipping table"),
        "the user must be told what was skipped; got: {text}"
    );
    assert!(!text.contains("panicked"), "must NOT panic; got: {text}");

    // No generated file anywhere contains the injected item.
    for name in [
        "entities/mod.rs",
        "mod.rs",
        "widgets.rs",
        "entities/widgets.rs",
    ] {
        if let Ok(content) = fs::read_to_string(root.join("src/models").join(name)) {
            assert!(
                !content.contains("pub struct Evil"),
                "injected source reached src/models/{name}:\n{content}"
            );
        }
    }
}

/// Every table being unusable is a failure, not a silent success - the user
/// asked for models and got none.
#[test]
fn db_sync_errors_when_every_table_is_rejected() {
    let dir = tempdir().expect("create tempdir");
    let root = dir.path();
    let db_path = root.join("schema.db");

    let rt = tokio::runtime::Runtime::new().expect("build tokio runtime");
    rt.block_on(async {
        let db = Database::connect(format!("sqlite://{}?mode=rwc", db_path.display()))
            .await
            .expect("connect to sqlite");
        db.execute_unprepared(r#"CREATE TABLE "../../pwned" (id INTEGER PRIMARY KEY)"#)
            .await
            .expect("create traversal table");
    });

    fs::create_dir_all(root.join("src/models/entities")).expect("create project layout");

    let out = Command::new(BIN)
        .arg("db:sync")
        .arg("--skip-migrations")
        .env("DATABASE_URL", format!("sqlite://{}", db_path.display()))
        .current_dir(root)
        .output()
        .expect("spawn suprnova binary");

    let text = combined(&out);
    assert_eq!(
        out.status.code(),
        Some(1),
        "an all-rejected schema must exit 1; output: {text}"
    );
    assert!(
        !root.join("src/pwned.rs").exists(),
        "nothing may be written outside the entity directory: {text}"
    );
    assert!(!text.contains("panicked"), "must NOT panic; got: {text}");
}

/// A symlink standing where an entity file goes must not redirect the write.
///
/// Teeth: with `fs::write` in place of the O_EXCL-plus-rename writer, the
/// victim file below ends up holding generated Rust.
#[cfg(unix)]
#[test]
fn db_sync_does_not_write_through_a_planted_symlink() {
    let dir = tempdir().expect("create tempdir");
    let root = dir.path();
    let db_path = root.join("schema.db");

    let rt = tokio::runtime::Runtime::new().expect("build tokio runtime");
    rt.block_on(async {
        let db = Database::connect(format!("sqlite://{}?mode=rwc", db_path.display()))
            .await
            .expect("connect to sqlite");
        db.execute_unprepared("CREATE TABLE widgets (id INTEGER PRIMARY KEY, name TEXT)")
            .await
            .expect("create widgets table");
    });

    fs::create_dir_all(root.join("src/models/entities")).expect("create project layout");
    let victim = root.join("precious.txt");
    fs::write(&victim, "DO NOT OVERWRITE").expect("seed victim");
    std::os::unix::fs::symlink(&victim, root.join("src/models/entities/widgets.rs"))
        .expect("plant symlink");

    let out = Command::new(BIN)
        .arg("db:sync")
        .arg("--skip-migrations")
        .env("DATABASE_URL", format!("sqlite://{}", db_path.display()))
        .current_dir(root)
        .output()
        .expect("spawn suprnova binary");

    let text = combined(&out);
    assert_eq!(
        fs::read_to_string(&victim).expect("victim readable"),
        "DO NOT OVERWRITE",
        "db:sync wrote through the symlink; output: {text}"
    );
    assert!(!text.contains("panicked"), "must NOT panic; got: {text}");
}

/// Requires a live Postgres. Run with:
///   PG_TEST_URL=postgres://user:pass@127.0.0.1:5432/probe \
///     cargo test -p suprnova-cli --test db_sync_cli -- --ignored postgres
///
/// Guards the identifier-vs-literal quoting bug: db_sync.rs emitted
/// `c.table_name = "users"`, which Postgres reads as a column reference,
/// so column discovery errored on every Postgres project.
#[test]
#[ignore = "requires a live Postgres; set PG_TEST_URL"]
fn db_sync_discovers_columns_against_real_postgres() {
    let url = match std::env::var("PG_TEST_URL") {
        Ok(u) => u,
        Err(_) => panic!("set PG_TEST_URL to run this test"),
    };

    let rt = tokio::runtime::Runtime::new().expect("build runtime");
    rt.block_on(async {
        let db = Database::connect(&url).await.expect("connect to postgres");
        db.execute_unprepared("DROP TABLE IF EXISTS probe_widgets")
            .await
            .expect("drop probe table");
        db.execute_unprepared(
            "CREATE TABLE probe_widgets (id BIGSERIAL PRIMARY KEY, label TEXT NOT NULL)",
        )
        .await
        .expect("create probe table");
    });

    let dir = tempdir().expect("create tempdir");
    let root = dir.path();
    fs::create_dir_all(root.join("src/models/entities")).expect("create project layout");
    fs::write(root.join("Cargo.toml"), "[package]\nname = \"probe\"\n").expect("write manifest");

    let out = Command::new(BIN)
        .arg("db:sync")
        .env("DATABASE_URL", &url)
        .current_dir(root)
        .output()
        .expect("spawn suprnova binary");

    let text = combined(&out);
    assert_eq!(
        out.status.code(),
        Some(0),
        "db:sync must succeed against Postgres; output: {text}"
    );

    let entity = fs::read_to_string(root.join("src/models/entities/probe_widgets.rs"))
        .expect("db:sync must have written an entity for probe_widgets");
    assert!(
        entity.contains("pub label"),
        "column discovery must find `label` - an empty column list is the \
         signature of the identifier-quoting bug; entity was:\n{entity}"
    );
}
