//! schedule:work command - Run the scheduler daemon

use crate::commands::interpret_cargo_status;
use crate::ui;

pub fn run() {
    if let Err(e) = run_inner() {
        ui::error(&e);
        std::process::exit(1);
    }
}

fn run_inner() -> Result<(), String> {
    ui::info("Starting scheduler daemon...");
    ui::hint("Press Ctrl+C to stop");
    ui::br();

    let status = crate::commands::cargo_run(&["schedule:work"]).status();

    interpret_cargo_status(status, "schedule:work", true)?;

    ui::br();
    ui::info("Scheduler daemon stopped.");
    Ok(())
}
