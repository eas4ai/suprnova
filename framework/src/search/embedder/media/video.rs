use super::{ImageFeatures, bounded, error, preprocess_search_image};
use crate::{FrameworkError, Process};
use std::{io::Read, time::Duration};

/// Isolated frame files let ffmpeg decode seekable containers without exposing upload paths.
pub async fn preprocess_search_video(bytes: &[u8]) -> Result<Vec<ImageFeatures>, FrameworkError> {
    let binary = std::env::var("SEARCH_FFMPEG_BINARY").unwrap_or_else(|_| "ffmpeg".into());
    preprocess_search_video_with_binary(bytes, &binary).await
}

/// Explicit binary selection keeps deployments and missing-binary tests independent of process globals.
pub async fn preprocess_search_video_with_binary(
    bytes: &[u8],
    binary: &str,
) -> Result<Vec<ImageFeatures>, FrameworkError> {
    bounded(bytes)?;
    let directory = tempfile::tempdir().map_err(error)?;
    let input = directory.path().join("input");
    tokio::fs::write(&input, bytes).await.map_err(error)?;
    let pattern = directory.path().join("%02d.png");
    let probe = Process::command([
        binary,
        "-nostdin",
        "-hide_banner",
        "-format_whitelist",
        "matroska,webm,mov,avi,mpegts",
        "-protocol_whitelist",
        "file",
        "-i",
        &input.to_string_lossy(),
    ])
    .timeout(Duration::from_secs(10))
    .run()
    .await
    .map_err(|e| error(format!("SEARCH_FFMPEG_BINARY: {e}")))?;
    let seconds = duration_seconds(probe.error_output())?;
    let count = seconds.floor().max(1.0) as usize;
    let filter = if count > 32 {
        let indices = (0..32)
            .map(|i| format!("eq(n\\,{})", i * (count - 1) / 31))
            .collect::<Vec<_>>()
            .join("+");
        format!("fps=1:round=up,select='{indices}'")
    } else {
        "fps=1:round=up".into()
    };
    let frame_limit = count.min(32).to_string();
    let result = Process::command([
        binary,
        "-nostdin",
        "-loglevel",
        "error",
        "-max_alloc",
        "67108864",
        "-format_whitelist",
        "matroska,webm,mov,avi,mpegts",
        "-protocol_whitelist",
        "file",
        "-i",
        &input.to_string_lossy(),
        "-vf",
        &filter,
        "-frames:v",
        &frame_limit,
        "-threads",
        "1",
        "-f",
        "image2",
        &pattern.to_string_lossy(),
    ])
    .timeout(Duration::from_secs(30))
    .run()
    .await
    .map_err(|e| error(format!("SEARCH_FFMPEG_BINARY: {e}")))?;
    if !result.successful() {
        return Err(error(format!(
            "SEARCH_FFMPEG_BINARY: {}",
            result.error_output()
        )));
    }
    tokio::task::spawn_blocking(move || decode_frames(directory))
        .await
        .map_err(error)?
}

fn decode_frames(directory: tempfile::TempDir) -> Result<Vec<ImageFeatures>, FrameworkError> {
    let mut frames = Vec::new();
    for index in 1..=32 {
        let path = directory.path().join(format!("{index:02}.png"));
        if !path.exists() {
            break;
        }
        let mut data = Vec::new();
        std::fs::File::open(path)
            .map_err(error)?
            .take(super::MAX_BYTES as u64 + 1)
            .read_to_end(&mut data)
            .map_err(error)?;
        frames.push(preprocess_search_image(&data, 140)?);
    }
    if frames.is_empty() {
        return Err(error("video has no frames at 1 fps"));
    }
    Ok(frames)
}

fn duration_seconds(output: &str) -> Result<f64, FrameworkError> {
    let text = output
        .split("Duration: ")
        .nth(1)
        .and_then(|value| value.split(',').next())
        .ok_or_else(|| error("SEARCH_FFMPEG_BINARY: video duration missing"))?;
    let values = text
        .split(':')
        .map(str::parse::<f64>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(error)?;
    if values.len() != 3 || values.iter().any(|v| !v.is_finite() || *v < 0.0) {
        return Err(error("invalid video duration"));
    }
    let duration = values[0] * 3600.0 + values[1] * 60.0 + values[2];
    if duration <= 0.0 || duration > 86400.0 {
        return Err(error(
            "video duration must be positive and at most 24 hours",
        ));
    }
    Ok(duration)
}
