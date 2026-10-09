//! db:seed and model:prune - commands the project's console binary owns.
//!
//! The console binary is the one that links the project's seeders and
//! models, so each of these builds the argument list and hands it over.

use crate::commands::interpret_cargo_status;
use crate::ui;

/// One console command, with the arguments the operator typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsoleCommand {
    /// `db:seed [--class=<Name>]`
    Seed { class: Option<String> },
    /// `model:prune [--model=<Name>] [--pretend]`
    Prune {
        model: Option<String>,
        pretend: bool,
    },
}

impl ConsoleCommand {
    /// The arguments the console binary receives, command name first.
    ///
    /// A value goes over in one argument with its option, `--class=<Name>`,
    /// so the console cannot read a value that starts with a hyphen as an
    /// option of its own. No value is checked here: the console knows the
    /// seeders and the models, and it is the one that can say a name is
    /// unknown.
    fn console_args(&self) -> Vec<String> {
        match self {
            Self::Seed { class: None } => vec!["db:seed".into()],
            Self::Seed { class: Some(class) } => {
                vec!["db:seed".into(), format!("--class={class}")]
            }
            Self::Prune { model, pretend } => {
                let mut args = vec!["model:prune".to_owned()];
                if let Some(model) = model {
                    args.push(format!("--model={model}"));
                }
                if *pretend {
                    args.push("--pretend".into());
                }
                args
            }
        }
    }
}

pub fn run(command: ConsoleCommand) {
    if let Err(e) = run_inner(&command) {
        ui::error(&e);
        std::process::exit(1);
    }
}

fn run_inner(command: &ConsoleCommand) -> Result<(), String> {
    let args = command.console_args();
    let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
    let status = crate::commands::cargo_run_console(&borrowed).status();

    interpret_cargo_status(status, &args[0], false)
}

#[cfg(test)]
mod tests {
    use super::ConsoleCommand;

    #[test]
    fn db_seed_forwards_the_seeder_as_the_class_option() {
        assert_eq!(
            ConsoleCommand::Seed { class: None }.console_args(),
            ["db:seed"]
        );
        assert_eq!(
            ConsoleCommand::Seed {
                class: Some("UserSeeder".into())
            }
            .console_args(),
            ["db:seed", "--class=UserSeeder"]
        );
    }

    #[test]
    fn model_prune_forwards_the_options_that_were_given() {
        assert_eq!(
            ConsoleCommand::Prune {
                model: None,
                pretend: false
            }
            .console_args(),
            ["model:prune"]
        );
        assert_eq!(
            ConsoleCommand::Prune {
                model: Some("User".into()),
                pretend: true
            }
            .console_args(),
            ["model:prune", "--model=User", "--pretend"]
        );
        assert_eq!(
            ConsoleCommand::Prune {
                model: None,
                pretend: true
            }
            .console_args(),
            ["model:prune", "--pretend"]
        );
    }
}
