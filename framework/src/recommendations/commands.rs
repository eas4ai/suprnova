//! Command inventory makes batch generation and application retention available to every app.
use super::{GenerateRun, GenerationOutcome, Recommend};
use crate::{FrameworkError, console};

#[suprnova_macros::command(
    name = "recommendations:generate",
    description = "Generate a recommendation snapshot [--surface s] [--user id]"
)]
async fn generate(args: Vec<String>) -> Result<(), FrameworkError> {
    let options = options(&args, &["--surface", "--user"])?;
    let run = GenerateRun {
        surface: options.get("--surface").cloned(),
        user: options.get("--user").cloned(),
    };
    match Recommend::generate(run).await? {
        GenerationOutcome::Locked => {
            console::line("recommendations:generate - lock held by another run")
        }
        GenerationOutcome::Completed(run) => console::line(super::storage::encode(&run)?),
    }
    Ok(())
}
#[suprnova_macros::command(
    name = "recommendations:status",
    description = "Show the latest recommendation runs"
)]
async fn status(args: Vec<String>) -> Result<(), FrameworkError> {
    options(&args, &[])?;
    for run in Recommend::status(10).await? {
        console::line(super::storage::encode(&run)?);
    }
    Ok(())
}
#[suprnova_macros::command(
    name = "recommendations:prune",
    description = "Apply application retention --older-than <days>"
)]
async fn prune(args: Vec<String>) -> Result<(), FrameworkError> {
    let options = options(&args, &["--older-than"])?;
    let days = options
        .get("--older-than")
        .ok_or_else(|| FrameworkError::param("recommendations:prune requires --older-than <days>"))?
        .parse::<u32>()
        .map_err(|_| FrameworkError::param("--older-than must be a positive number of days"))?;
    console::line(format!(
        "recommendations:prune - removed {} events",
        Recommend::prune(days).await?
    ));
    Ok(())
}
#[suprnova_macros::command(
    name = "recommendations:graph-rebuild",
    description = "Rebuild the recommendation graph from the event stream"
)]
async fn graph_rebuild(args: Vec<String>) -> Result<(), FrameworkError> {
    options(&args, &[])?;
    match Recommend::rebuild_graph().await? {
        None => console::line("recommendations:graph-rebuild - lock held by another rebuild"),
        Some(built) => console::line(super::storage::encode(&built)?),
    }
    Ok(())
}
fn options(
    args: &[String],
    allowed: &[&str],
) -> Result<std::collections::BTreeMap<String, String>, FrameworkError> {
    let mut options = std::collections::BTreeMap::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let (name, value) = match arg.split_once('=') {
            Some(pair) => pair,
            None => (
                arg.as_str(),
                args.next()
                    .ok_or_else(|| FrameworkError::param(format!("{arg} requires a value")))?
                    .as_str(),
            ),
        };
        if !allowed.contains(&name) {
            return Err(FrameworkError::param(format!(
                "unknown recommendation option: {name}"
            )));
        }
        if value.trim().is_empty() || value.starts_with("--") {
            return Err(FrameworkError::param(format!("{name} requires a value")));
        }
        if options.insert(name.into(), value.into()).is_some() {
            return Err(FrameworkError::param(format!(
                "{name} supplied more than once"
            )));
        }
    }
    Ok(options)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_both_option_forms_and_rejects_malformed_commands() {
        let args = ["--user", "alice", "--surface=home"].map(String::from);
        let parsed = options(&args, &["--user", "--surface"]).unwrap();
        assert_eq!(parsed["--user"], "alice");
        assert_eq!(parsed["--surface"], "home");
        for args in [
            vec!["--user"],
            vec!["--user="],
            vec!["--user=a", "--user=b"],
            vec!["--unknown=x"],
        ] {
            assert!(
                options(
                    &args.into_iter().map(String::from).collect::<Vec<_>>(),
                    &["--user"]
                )
                .is_err()
            );
        }
    }
}
