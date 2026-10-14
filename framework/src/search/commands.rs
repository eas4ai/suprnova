//! Console commands resolve declared indexes before invoking the generation coordinator.

use super::{generations, registry::SearchRegistry};
use crate::FrameworkError;
use suprnova_macros::command;

fn parse_args(
    command: &str,
    args: &[String],
    required: bool,
    model: bool,
    allow_prune: bool,
) -> Result<(Option<&'static str>, bool), FrameworkError> {
    let kind = if model { "model or index" } else { "index" };
    let mut name = None;
    let mut prune = false;
    for argument in args {
        if allow_prune && argument == "--prune" {
            if prune {
                return Err(FrameworkError::bad_request(format!(
                    "{command}: --prune supplied twice"
                )));
            }
            prune = true;
        } else if argument.starts_with('-') {
            return Err(FrameworkError::bad_request(format!(
                "{command}: unknown option '{argument}'"
            )));
        } else if argument.trim().is_empty() || name.replace(argument.as_str()).is_some() {
            return Err(FrameworkError::bad_request(format!(
                "{command}: requires one {kind} name"
            )));
        }
    }
    let Some(name) = name else {
        if required {
            return Err(FrameworkError::bad_request(format!(
                "{command}: requires a {kind} name"
            )));
        }
        return Ok((None, prune));
    };
    let entry = if model {
        SearchRegistry::by_model_or_index(name)
    } else {
        SearchRegistry::by_index(name)
    }
    .map_err(|error| FrameworkError::bad_request(format!("{command}: {error}")))?;
    Ok((Some(entry.index), prune))
}

#[command(
    name = "search:import",
    description = "Import a searchable model or index"
)]
async fn import(args: Vec<String>) -> Result<(), FrameworkError> {
    let (index, _) = parse_args("search:import", &args, true, true, false)?;
    if let Some(index) = index {
        generations::import_index(index).await?;
    }
    Ok(())
}

#[command(
    name = "search:flush",
    description = "Remove all documents from a model or index"
)]
async fn flush(args: Vec<String>) -> Result<(), FrameworkError> {
    let (index, _) = parse_args("search:flush", &args, true, true, false)?;
    if let Some(index) = index {
        generations::flush_index(index).await?;
    }
    Ok(())
}

#[command(name = "search:index", description = "Create a declared search index")]
async fn create(args: Vec<String>) -> Result<(), FrameworkError> {
    let (index, _) = parse_args("search:index", &args, true, false, false)?;
    if let Some(index) = index {
        generations::create_index(index).await?;
    }
    Ok(())
}

#[command(
    name = "search:delete-index",
    description = "Delete a declared search index"
)]
async fn delete(args: Vec<String>) -> Result<(), FrameworkError> {
    let (index, _) = parse_args("search:delete-index", &args, true, false, false)?;
    if let Some(index) = index {
        generations::delete_index(index).await?;
    }
    Ok(())
}

#[command(
    name = "search:sync-index-settings",
    description = "Apply code-declared search settings"
)]
async fn sync(args: Vec<String>) -> Result<(), FrameworkError> {
    let (index, _) = parse_args("search:sync-index-settings", &args, false, false, false)?;
    let indexes = match index {
        Some(index) => vec![index],
        None => SearchRegistry::indexes()?,
    };
    for index in indexes {
        generations::sync_settings(index).await?;
    }
    Ok(())
}

#[command(
    name = "search:reindex",
    description = "Build and publish a complete search generation"
)]
async fn reindex(args: Vec<String>) -> Result<(), FrameworkError> {
    let (index, _) = parse_args("search:reindex", &args, true, true, false)?;
    if let Some(index) = index {
        generations::reindex(index).await?;
    }
    Ok(())
}

#[command(
    name = "search:status",
    description = "Report search generations and optionally prune tombstones"
)]
async fn status(args: Vec<String>) -> Result<(), FrameworkError> {
    let (index, prune) = parse_args("search:status", &args, false, false, true)?;
    let indexes = match index {
        Some(index) => vec![index],
        None => SearchRegistry::indexes()?,
    };
    if indexes.is_empty() && prune {
        generations::prune_tombstones().await?;
    }
    for index in indexes {
        generations::status(index, prune).await?;
    }
    Ok(())
}
