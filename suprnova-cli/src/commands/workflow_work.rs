//! workflow:work command - Run the workflow worker daemon

use crate::commands::interpret_cargo_status;
use crate::ui;

pub fn run() {
    if let Err(e) = run_inner() {
        ui::error(&e);
        std::process::exit(1);
    }
}

fn run_inner() -> Result<(), String> {
    ui::info("Starting workflow worker...");
    ui::hint("Press Ctrl+C to stop");
    ui::br();

    let status = crate::commands::cargo_run(&["workflow:work"]).status();

    interpret_cargo_status(status, "workflow:work", true)?;

    ui::br();
    ui::info("Workflow worker stopped.");
    Ok(())
}
