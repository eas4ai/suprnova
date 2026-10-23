//! schedule:list command - Display all registered scheduled tasks

use crate::commands::interpret_cargo_status;
use crate::ui;

pub fn run(timezone: Option<&str>) {
    if let Err(e) = run_inner(timezone) {
        ui::error(&e);
        std::process::exit(1);
    }
}

/// The arguments the application binary receives, command name first.
///
/// The zone goes over as one `--timezone=<zone>` argument and is not
/// checked here: the application owns the zone database the listing is
/// read in, so it is the one that can say a name is unknown.
fn app_args(timezone: Option<&str>) -> Vec<String> {
    let mut args = vec!["schedule:list".to_owned()];
    if let Some(timezone) = timezone {
        args.push(format!("--timezone={timezone}"));
    }
    args
}

fn run_inner(timezone: Option<&str>) -> Result<(), String> {
    let args = app_args(timezone);
    let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
    let status = crate::commands::cargo_run(&borrowed).status();

    interpret_cargo_status(status, "schedule:list", false)
}

#[cfg(test)]
mod tests {
    use super::app_args;

    #[test]
    fn the_timezone_is_forwarded_to_the_application() {
        assert_eq!(
            app_args(Some("Asia/Tokyo")),
            ["schedule:list", "--timezone=Asia/Tokyo"]
        );
    }

    #[test]
    fn without_a_timezone_the_application_gets_the_command_alone() {
        assert_eq!(app_args(None), ["schedule:list"]);
    }
}
