use std::path::Path;

use crate::commands::interpret_cargo_status;
use crate::ui;

/// `suprnova schema:dump`: the app binary's `schema:dump`, which owns the
/// database connection; with `--prune` it also edits `src/migrations`.
pub fn run(
    path: Option<String>,
    prune: bool,
    database: Option<String>,
    without_migration_data: bool,
) {
    if let Err(e) = run_inner(
        path.as_deref(),
        prune,
        database.as_deref(),
        without_migration_data,
    ) {
        ui::error(&e);
        std::process::exit(1);
    }
}

fn run_inner(
    path: Option<&str>,
    prune: bool,
    database: Option<&str>,
    without_migration_data: bool,
) -> Result<(), String> {
    if prune && !Path::new("src/migrations").exists() {
        return Err("--prune needs the migrations at src/migrations".to_string());
    }

    ui::info("Dumping the database schema...");

    let mut args = vec!["schema:dump"];
    if let Some(path) = path {
        args.extend(["--path", path]);
    }
    if prune {
        args.push("--prune");
    }
    if let Some(database) = database {
        args.extend(["--database", database]);
    }
    if without_migration_data {
        args.push("--without-migration-data");
    }
    let status = crate::commands::cargo_run(&args).status();

    interpret_cargo_status(status, "schema:dump", false)
}
