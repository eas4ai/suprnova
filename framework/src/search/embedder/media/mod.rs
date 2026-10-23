//! Processor tensors are available without loading ONNX Runtime.

mod audio;
mod image;
mod video;

pub use audio::{AudioFeatures, preprocess_search_audio};
pub use image::{ImageFeatures, preprocess_search_image};
pub use video::{preprocess_search_video, preprocess_search_video_with_binary};

use crate::FrameworkError;

const MAX_BYTES: usize = 32 * 1024 * 1024;

fn bounded(bytes: &[u8]) -> Result<(), FrameworkError> {
    if bytes.is_empty() || bytes.len() > MAX_BYTES {
        return Err(FrameworkError::param(
            "search media requires 1 to 33554432 bytes",
        ));
    }
    Ok(())
}

fn error(message: impl std::fmt::Display) -> FrameworkError {
    FrameworkError::param(format!("search media: {message}"))
}

#[cfg(feature = "search-fastembed")]
pub(super) mod runtime;
