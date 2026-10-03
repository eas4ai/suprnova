use std::path::Path;

use crate::commands::interpret_cargo_status;
use crate::ui;

pub fn run(schema_path: Option<String>) {
    if let Err(e) = run_inner(schema_path.as_deref()) {
        ui::error(&e);
        std::process::exit(1);
    }
}

fn run_inner(schema_path: Option<&str>) -> Result<(), String> {
    if !Path::new("src/migrations").exists() {
        ui::hint("Run 'suprnova make:migration <name>' to create your first migration.");
        return Err("No migrations directory found at src/migrations".to_string());
    }

    ui::info("Running migrations...");

    let mut args = vec!["migrate"];
    if let Some(path) = schema_path {
        args.extend(["--schema-path", path]);
    }
    let status = crate::commands::cargo_run(&args).status();

    interpret_cargo_status(status, "migrate", false)
}
