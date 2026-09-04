//! Hidden operator console commands for RenderCache: an emergency epoch
//! advance and body-free entry inspection by key.
//!
//! Registered the same way `crate::live::tooling` registers its own
//! hidden command - a `build_command` function, a `run_command` function,
//! and an `inventory::submit!` block binding them to a
//! [`crate::console::CommandEntry`] - rather than a call into
//! `console/mod.rs`, which has no `register` function to call. Both
//! commands print bounded, human-readable text; neither ever prints a
//! stored body or a raw (non-digested) identity.

use std::future::Future;
use std::pin::Pin;

use crate::console::CommandEntry;
use crate::error::FrameworkError;

use super::RenderCache;

const EPOCH_ADVANCE_COMMAND_NAME: &str = "render-cache:epoch-advance";
const INSPECT_COMMAND_NAME: &str = "render-cache:inspect";

fn build_epoch_advance_command() -> clap::Command {
    clap::Command::new(EPOCH_ADVANCE_COMMAND_NAME)
        .hide(true)
        .about(
            "Advances the RenderCache authority epoch, making every stored entry \
             unreachable at its next freshness check (emergency invalidation)",
        )
}

fn run_epoch_advance_command(
    _matches: &clap::ArgMatches,
) -> Pin<Box<dyn Future<Output = Result<(), FrameworkError>> + Send>> {
    Box::pin(async move {
        RenderCache::advance_epoch()
            .await
            .map_err(|_| FrameworkError::internal("render-cache:epoch-advance failed"))?;
        println!("epoch advanced");
        Ok(())
    })
}

inventory::submit! {
    CommandEntry {
        name: EPOCH_ADVANCE_COMMAND_NAME,
        description: "Advance the RenderCache authority epoch (emergency invalidation)",
        clap_builder: build_epoch_advance_command,
        handler: run_epoch_advance_command,
    }
}

fn build_inspect_command() -> clap::Command {
    clap::Command::new(INSPECT_COMMAND_NAME)
        .hide(true)
        .about("Body-free inspection of one RenderCache entry by its rk1. key")
        .arg(
            clap::Arg::new("key")
                .required(true)
                .help("The entry's rk1. lookup key, as printed by application logging"),
        )
}

fn run_inspect_command(
    matches: &clap::ArgMatches,
) -> Pin<Box<dyn Future<Output = Result<(), FrameworkError>> + Send>> {
    let key = matches
        .get_one::<String>("key")
        .cloned()
        .unwrap_or_default();
    Box::pin(async move {
        match RenderCache::inspect(&key).await {
            Ok(Some(inspection)) => println!("{inspection:?}"),
            Ok(None) => println!("no entry"),
            Err(_) => println!("render-cache:inspect failed: invalid key or no runtime installed"),
        }
        Ok(())
    })
}

inventory::submit! {
    CommandEntry {
        name: INSPECT_COMMAND_NAME,
        description: "Inspect one RenderCache entry by key (metadata only, never a body)",
        clap_builder: build_inspect_command,
        handler: run_inspect_command,
    }
}
