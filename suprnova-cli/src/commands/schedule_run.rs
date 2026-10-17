//! schedule:run command - Run all due scheduled tasks once

use crate::commands::interpret_cargo_status;
use crate::ui;

pub fn run() {
    if let Err(e) = run_inner() {
        ui::error(&e);
        std::process::exit(1);
    }
}

fn run_inner() -> Result<(), String> {
    ui::info("Running due scheduled tasks...");
    ui::br();

    let status = crate::commands::cargo_run(&["schedule:run"]).status();

    interpret_cargo_status(status, "schedule:run", false)
}
