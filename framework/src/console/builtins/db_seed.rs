//! Runs the root seeder or a named class on the requested database.
//! Production needs `--force`. Nested calls share one invocation's once tracking.
//! Targeted progress keeps the console's existing two-column layout.

use std::time::Instant;

use crate::console::output;
use crate::error::FrameworkError;
use crate::seed;
use suprnova_macros::command;

#[command(
    name = "db:seed",
    description = "Run the root seeder, or one via --class=<Name>"
)]
async fn db_seed(args: Vec<String>) -> Result<(), FrameworkError> {
    let options = parse_seed_args(&args)?;
    if crate::Config::is_production() && !options.force {
        return Err(FrameworkError::bad_request(
            "db:seed refuses to run in production without --force",
        ));
    }
    let run = seed::with_invocation(run_seed(options.class));
    match options.database {
        Some(name) => crate::DB::with_default_connection(name, run).await,
        None => run.await,
    }
}

async fn run_seed(class: Option<String>) -> Result<(), FrameworkError> {
    // Only a bare run has "nothing to run". A named class goes to
    // `run_one`, which owns the not-found error: an empty registry has no
    // seeder of that name either, and reporting success for work that was
    // never found is the wrong exit. The count is read fallibly, so a
    // registry that cannot be read fails the command instead of looking
    // empty.
    if class.is_none() && seed::try_count()? == 0 {
        // Two channels by design: the standard error so the user
        // actually sees feedback in the absence of a configured tracing
        // subscriber; tracing::warn so observability tools still
        // pick it up in production.
        crate::console::error_line("db:seed: no seeders registered - nothing to run");
        tracing::warn!("db:seed: no seeders registered - nothing to run");
        return Ok(());
    }

    match class {
        Some(name) => {
            // Laravel resolves the seeder before it reports progress
            // (`SeedCommand.php:71` runs before the `:79-83` RUNNING
            // line) - an unknown class fails before any progress line
            // prints. Match that here: bail out through `run_one` (the
            // single owner of the not-found error text) before printing
            // RUNNING, rather than duplicating the message.
            if !seed::is_registered(&name) {
                return seed::run_one(&name).await;
            }
            // The root controls its own child progress. A named command
            // keeps the existing console layout for its outer progress.
            crate::console::line(output::two_column_detail(&name, "RUNNING"));
            let started = Instant::now();
            let result = seed::run_one(&name).await;
            // DONE only on success. A failure is reported once, by the
            // dispatcher, on stderr - printing it here as well would
            // double up.
            if result.is_ok() {
                let elapsed = started.elapsed().as_millis();
                crate::console::line(output::two_column_detail(
                    &name,
                    &format!("{elapsed} ms DONE"),
                ));
                crate::console::line("");
            }
            result
        }
        None => seed::run_root().await,
    }
}

#[derive(Debug, Default)]
struct SeedArgs {
    class: Option<String>,
    database: Option<String>,
    force: bool,
}

fn parse_seed_args(args: &[String]) -> Result<SeedArgs, FrameworkError> {
    let mut options = SeedArgs::default();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--force" {
            options.force = true;
            continue;
        }
        let (flag, value) = if let Some(value) = arg.strip_prefix("--class=") {
            ("--class", value)
        } else if let Some(value) = arg.strip_prefix("--database=") {
            ("--database", value)
        } else if arg == "--class" || arg == "--database" {
            let value = iter.next().ok_or_else(|| missing_value(arg))?;
            (arg.as_str(), value.as_str())
        } else if arg.starts_with('-') {
            return Err(FrameworkError::bad_request(format!(
                "db:seed: unknown option `{arg}`"
            )));
        } else {
            ("--class", arg.as_str())
        };
        if value.is_empty() || value.starts_with('-') {
            return Err(missing_value(flag));
        }
        let target = if flag == "--class" {
            &mut options.class
        } else {
            &mut options.database
        };
        if target.replace(value.to_owned()).is_some() {
            return Err(FrameworkError::bad_request(format!(
                "db:seed: {flag} supplied twice"
            )));
        }
    }
    Ok(options)
}

fn missing_value(flag: &str) -> FrameworkError {
    let kind = if flag == "--class" {
        "seeder"
    } else {
        "connection"
    };
    FrameworkError::bad_request(format!(
        "db:seed {flag} requires a {kind} name, found a flag or missing value"
    ))
}

#[cfg(test)]
fn parse_class_arg(args: &[String]) -> Result<Option<String>, FrameworkError> {
    parse_seed_args(args).map(|args| args.class)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn no_args_returns_none() {
        assert_eq!(parse_class_arg(&[]).unwrap(), None);
    }

    #[test]
    fn equals_form_parses_name() {
        assert_eq!(
            parse_class_arg(&s(&["--class=UserSeeder"])).unwrap(),
            Some("UserSeeder".to_string())
        );
    }

    #[test]
    fn space_form_parses_name() {
        assert_eq!(
            parse_class_arg(&s(&["--class", "UserSeeder"])).unwrap(),
            Some("UserSeeder".to_string())
        );
    }

    #[test]
    fn bare_positional_parses_name() {
        assert_eq!(
            parse_class_arg(&s(&["UserSeeder"])).unwrap(),
            Some("UserSeeder".to_string())
        );
    }

    #[test]
    fn empty_equals_rejected() {
        let err = parse_class_arg(&s(&["--class="])).unwrap_err();
        assert!(format!("{err}").contains("requires a seeder name"));
    }

    #[test]
    fn missing_value_after_space_rejected() {
        let err = parse_class_arg(&s(&["--class"])).unwrap_err();
        assert!(format!("{err}").contains("requires a seeder name"));
    }

    #[test]
    fn flag_after_class_rejected() {
        let err = parse_class_arg(&s(&["--class", "--force"])).unwrap_err();
        assert!(format!("{err}").contains("found a flag"));
    }
}
