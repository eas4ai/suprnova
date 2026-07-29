//! schedule:list command - Display all registered scheduled tasks

use crate::commands::interpret_cargo_status;
use crate::ui;

pub fn run() {
    if let Err(e) = run_inner() {
        ui::error(&e);
        std::process::exit(1);
    }
}

fn run_inner() -> Result<(), String> {
    let status = crate::commands::cargo_run(&["schedule:list"]).status();

    interpret_cargo_status(status, "schedule:list", false)
}
