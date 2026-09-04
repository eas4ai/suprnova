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
//!
//! This module is `pub` only so the `_for_test` report builders below are
//! reachable from integration tests outside this crate, letting a test
//! assert on exactly what gets printed without capturing process stdout;
//! it registers no command API of its own; `build_epoch_advance_command`,
//! `run_epoch_advance_command`, `build_inspect_command`, and
//! `run_inspect_command` stay private, reachable only through the
//! `inventory::submit!` blocks below.
//!
//! Fix round 1 (R95): `run_inspect_command` used to map every failure -
//! including a malformed key - to a printed message and `Ok(())`,
//! indistinguishable from success to a caller inspecting the exit code.
//! It now propagates like `run_epoch_advance_command` always did (F9).
//! Both commands also now print the current authority epoch alongside
//! whatever else they report, so an operator can tell whether an
//! inspected entry's own epoch (`EntryInspection::epoch`) is still live or
//! stale (F10).

use std::future::Future;
use std::pin::Pin;

use crate::console::CommandEntry;
use crate::error::FrameworkError;

use super::RenderCache;

const EPOCH_ADVANCE_COMMAND_NAME: &str = "render-cache:epoch-advance";
const INSPECT_COMMAND_NAME: &str = "render-cache:inspect";

/// The current authority epoch, for display only. `unwrap_or_default`
/// (reads as epoch `0`) rather than propagating a second failure: this is
/// called only after the primary operation (`advance_epoch` or `inspect`)
/// has already succeeded against an installed runtime, so a failure here
/// would be surprising, not a normal case worth failing the whole command
/// over - the primary result is still worth printing and returning `Ok`
/// for.
async fn current_epoch_for_display() -> u64 {
    RenderCache::store_inspection()
        .await
        .map(|store| store.epoch)
        .unwrap_or_default()
}

/// Builds exactly the text `render-cache:epoch-advance` prints, so a test
/// can assert on it without capturing process stdout.
async fn epoch_advance_report() -> Result<String, FrameworkError> {
    RenderCache::advance_epoch()
        .await
        .map_err(|_| FrameworkError::internal("render-cache:epoch-advance failed"))?;
    let epoch = current_epoch_for_display().await;
    Ok(format!("epoch advanced to {epoch}"))
}

/// Test-only: exposes [`epoch_advance_report`] to integration tests
/// outside this crate. Not part of the public console command API.
#[doc(hidden)]
pub async fn epoch_advance_report_for_test() -> Result<String, FrameworkError> {
    epoch_advance_report().await
}

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
        println!("{}", epoch_advance_report().await?);
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

/// Builds exactly the text `render-cache:inspect` prints, so a test can
/// assert on it without capturing process stdout.
///
/// # Errors
///
/// Propagates [`RenderCache::inspect`]'s error (an unparseable key text,
/// or no runtime installed) rather than swallowing it - fix round 1, R95/F9:
/// this used to return `Ok(())` on every failure, indistinguishable from
/// success to a caller checking the exit code. A well-formed key that
/// simply names no stored entry is not a failure, and still resolves
/// `Ok("no entry ...")`.
async fn inspect_report(key: &str) -> Result<String, FrameworkError> {
    let inspection = RenderCache::inspect(key).await.map_err(|_| {
        FrameworkError::internal("render-cache:inspect failed: invalid key or no runtime installed")
    })?;
    let epoch = current_epoch_for_display().await;
    Ok(match inspection {
        Some(inspection) => format!("{inspection:?} (current epoch: {epoch})"),
        None => format!("no entry (current epoch: {epoch})"),
    })
}

/// Test-only: exposes [`inspect_report`] to integration tests outside this
/// crate. Not part of the public console command API.
#[doc(hidden)]
pub async fn inspect_report_for_test(key: &str) -> Result<String, FrameworkError> {
    inspect_report(key).await
}

fn run_inspect_command(
    matches: &clap::ArgMatches,
) -> Pin<Box<dyn Future<Output = Result<(), FrameworkError>> + Send>> {
    let key = matches
        .get_one::<String>("key")
        .cloned()
        .unwrap_or_default();
    Box::pin(async move {
        println!("{}", inspect_report(&key).await?);
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
