//! queue:failed, queue:retry, queue:forget, queue:flush and
//! queue:prune-failed - the failed-job commands.
//!
//! The application binary owns the failed-job store and the queue a retry
//! pushes to, so each of these builds the argument list and hands it over.

use crate::commands::interpret_cargo_status;
use crate::ui;

/// One failed-job command, with the arguments the operator typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailedJobs {
    /// `queue:failed`
    List,
    /// `queue:retry <id>...` or `queue:retry all`
    Retry(Vec<String>),
    /// `queue:forget <id>`
    Forget(String),
    /// `queue:flush [--hours N]`
    Flush(Option<u64>),
    /// `queue:prune-failed [--hours N]`
    Prune(u64),
}

impl FailedJobs {
    /// The arguments the application binary receives, command name first.
    fn app_args(&self) -> Vec<String> {
        match self {
            Self::List => vec!["queue:failed".into()],
            Self::Retry(ids) => std::iter::once("queue:retry".to_owned())
                .chain(ids.iter().cloned())
                .collect(),
            Self::Forget(id) => vec!["queue:forget".into(), id.clone()],
            Self::Flush(None) => vec!["queue:flush".into()],
            Self::Flush(Some(hours)) => {
                vec!["queue:flush".into(), "--hours".into(), hours.to_string()]
            }
            Self::Prune(hours) => vec![
                "queue:prune-failed".into(),
                "--hours".into(),
                hours.to_string(),
            ],
        }
    }
}

pub fn run(command: FailedJobs) {
    if let Err(e) = run_inner(&command) {
        ui::error(&e);
        std::process::exit(1);
    }
}

fn run_inner(command: &FailedJobs) -> Result<(), String> {
    let args = command.app_args();
    let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
    let status = crate::commands::cargo_run(&borrowed).status();

    interpret_cargo_status(status, &args[0], false)
}

#[cfg(test)]
mod tests {
    use super::FailedJobs;

    #[test]
    fn each_command_forwards_its_own_name_and_arguments() {
        assert_eq!(FailedJobs::List.app_args(), ["queue:failed"]);
        assert_eq!(
            FailedJobs::Retry(vec!["a".into(), "b".into()]).app_args(),
            ["queue:retry", "a", "b"]
        );
        assert_eq!(
            FailedJobs::Retry(vec!["all".into()]).app_args(),
            ["queue:retry", "all"]
        );
        assert_eq!(
            FailedJobs::Forget("a".into()).app_args(),
            ["queue:forget", "a"]
        );
        assert_eq!(FailedJobs::Flush(None).app_args(), ["queue:flush"]);
        assert_eq!(
            FailedJobs::Flush(Some(48)).app_args(),
            ["queue:flush", "--hours", "48"]
        );
        assert_eq!(
            FailedJobs::Prune(24).app_args(),
            ["queue:prune-failed", "--hours", "24"]
        );
    }
}
