//! Storage reads are bounded independently of decoder limits.

use super::{AttributeValue, MediaPart, SearchDocument, settings::MediaSource};
use crate::FrameworkError;

/// Range reads prevent a changed object from bypassing its earlier size check.
pub async fn load_search_media(
    document: &SearchDocument,
    sources: &[MediaSource],
    limit: usize,
) -> Result<Vec<MediaPart>, FrameworkError> {
    if limit == 0 || limit > 32 * 1024 * 1024 {
        return Err(FrameworkError::param(
            "SEARCH_MEDIA_MAX_BYTES requires 1 to 33554432",
        ));
    }
    let mut parts = Vec::with_capacity(sources.len());
    for source in sources {
        let fail = |message: &str| {
            FrameworkError::param(format!(
                "search index '{}' media field '{}': {message}",
                document.index, source.field
            ))
        };
        let value = document
            .attributes
            .get(&source.field)
            .ok_or_else(|| fail("missing storage path"))?;
        if matches!(value, AttributeValue::Null) {
            continue;
        }
        let path = value
            .as_text()
            .filter(|path| !path.is_empty())
            .ok_or_else(|| fail("requires a storage path string"))?;
        let bytes = read_part(source, path, limit)
            .await
            .map_err(|e| fail(&e.to_string()))?;
        parts.push(MediaPart {
            media_type: media_type(source, path),
            data: bytes,
        });
    }
    Ok(parts)
}

#[cfg(feature = "filesystem")]
async fn read_part(
    source: &MediaSource,
    path: &str,
    limit: usize,
) -> Result<Vec<u8>, FrameworkError> {
    let disk = crate::Storage::disk(&source.disk)?;
    let fail = |error: opendal::Error| FrameworkError::param(error.to_string());
    let metadata = disk.stat(path).await.map_err(fail)?;
    if metadata.content_length() > limit as u64 {
        return Err(FrameworkError::param("exceeds SEARCH_MEDIA_MAX_BYTES"));
    }
    let bytes = disk
        .read_with(path)
        .range(0..metadata.content_length().min(limit as u64 + 1))
        .await
        .map_err(fail)?
        .to_vec();
    if bytes.is_empty() || bytes.len() > limit {
        return Err(FrameworkError::param(
            "empty or exceeds SEARCH_MEDIA_MAX_BYTES",
        ));
    }
    Ok(bytes)
}

#[cfg(not(feature = "filesystem"))]
async fn read_part(
    _source: &MediaSource,
    _path: &str,
    _limit: usize,
) -> Result<Vec<u8>, FrameworkError> {
    Err(FrameworkError::param(
        "search media storage requires the filesystem cargo feature",
    ))
}

fn media_type(source: &MediaSource, path: &str) -> String {
    let extension = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    let subtype = match (source.kind, extension.as_str()) {
        (super::settings::MediaKind::Image, "jpg" | "jpeg") => "jpeg",
        (super::settings::MediaKind::Image, "webp") => "webp",
        (super::settings::MediaKind::Image, _) => "png",
        (super::settings::MediaKind::Audio, _) => "wav",
        (super::settings::MediaKind::Video, "mp4") => "mp4",
        (super::settings::MediaKind::Video, "webm") => "webm",
        (super::settings::MediaKind::Video, _) => "x-matroska",
    };
    format!("{}/{subtype}", source.kind.name())
}
