//! Publication writes complete versions before changing the live manifest.

pub(crate) mod assets;
pub(crate) mod batch;
pub mod cdn;
mod console;
#[cfg(feature = "media")]
pub mod images;
mod lease;
mod manifest;
mod publisher;
mod runtime;
mod work;

pub use manifest::{Manifest, ObjectEntry, VersionEntry};
pub use publisher::{
    PageObject, PruneOutcome, PublishConfig, PublishOutcome, PublishStatus, Publisher,
};
pub use runtime::StaticPublication;

pub(crate) use runtime::{committed_generations, spawn_after_bind};

use crate::FrameworkError;

fn error(message: impl std::fmt::Display) -> FrameworkError {
    FrameworkError::internal(format!("static publication: {message}"))
}

fn safe_path(path: &str) -> Result<String, FrameworkError> {
    if !path.starts_with('/')
        || path.starts_with("//")
        || path.contains(['\\', '?', '#', '%'])
        || path.chars().any(|c| c.is_control() || c.is_whitespace())
        || path.split('/').any(|part| matches!(part, "." | ".."))
    {
        return Err(error(
            "expected a local absolute path without query or dot segments",
        ));
    }
    let path = path.trim_end_matches('/');
    Ok(if path.is_empty() { "/" } else { path }.to_owned())
}

fn stem(path: &str) -> &str {
    if path == "/" {
        "index"
    } else {
        path.trim_matches('/')
    }
}
