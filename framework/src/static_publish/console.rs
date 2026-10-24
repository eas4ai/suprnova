//! Console commands use the installed application service, just like boot.

use super::{StaticPublication, error};
use crate::{FrameworkError, console::CommandEntry};
use std::{future::Future, pin::Pin};

fn service() -> Result<std::sync::Arc<StaticPublication>, FrameworkError> {
    StaticPublication::current()?
        .ok_or_else(|| error("no static publication configured in application bootstrap"))
}
fn publish_command() -> clap::Command {
    clap::Command::new("static:publish").about("Publish marked public Inertia pages")
}
fn status_command() -> clap::Command {
    clap::Command::new("static:status").about("Show live version, last outcome and pending work")
}
fn prune_command() -> clap::Command {
    clap::Command::new("static:prune")
        .about("Remove publication objects outside their grace period")
}
fn publish(
    _: &clap::ArgMatches,
) -> Pin<Box<dyn Future<Output = Result<(), FrameworkError>> + Send>> {
    Box::pin(async {
        crate::console::line(
            serde_json::to_string_pretty(&service()?.publish().await?).map_err(error)?,
        );
        Ok(())
    })
}
fn status(
    _: &clap::ArgMatches,
) -> Pin<Box<dyn Future<Output = Result<(), FrameworkError>> + Send>> {
    Box::pin(async {
        crate::console::line(
            serde_json::to_string_pretty(&service()?.status().await).map_err(error)?,
        );
        Ok(())
    })
}
fn prune(_: &clap::ArgMatches) -> Pin<Box<dyn Future<Output = Result<(), FrameworkError>> + Send>> {
    Box::pin(async {
        crate::console::line(
            serde_json::to_string_pretty(&service()?.prune().await?).map_err(error)?,
        );
        Ok(())
    })
}
inventory::submit! { CommandEntry { name:"static:publish", description:"Publish public pages", clap_builder:publish_command, handler:publish } }
inventory::submit! { CommandEntry { name:"static:status", description:"Show publication status", clap_builder:status_command, handler:status } }
inventory::submit! { CommandEntry { name:"static:prune", description:"Prune expired publication objects", clap_builder:prune_command, handler:prune } }
