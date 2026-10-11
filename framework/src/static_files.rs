//! Framework-native static file fallback serving.
//!
//! [`StaticFiles`] is intended for `fallback!` registration: normal routes
//! win first, and safe `GET` / `HEAD` misses can be resolved from a configured
//! public directory without handing the request to application code.

use crate::app::paths::public_path;
use crate::http::file_response::{file_body, mime_from_extension, with_charset};
use crate::http::{HttpResponse, Request, Response};
use bytes::Bytes;
use hyper::Method;
use std::future::Future;
use std::path::{Component, Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use tokio::io::AsyncReadExt;

const SNIFF_PREFIX_BYTES: usize = 8 * 1024;

/// Static file fallback handler rooted at a configured directory.
#[derive(Clone, Debug)]
pub struct StaticFiles {
    root: PathBuf,
    cache_control: Option<String>,
}

impl StaticFiles {
    /// Serve files from Suprnova's configured public directory.
    pub fn public() -> Self {
        Self::from_dir(public_path(""))
    }

    /// Serve files from `root`.
    pub fn from_dir(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            cache_control: None,
        }
    }

    /// Attach a `Cache-Control` header to successful static file responses.
    pub fn cache_control(mut self, value: impl Into<String>) -> Self {
        self.cache_control = Some(value.into());
        self
    }

    /// Build a cloneable fallback handler suitable for `fallback!(...)`.
    pub fn handler(
        self,
    ) -> impl Fn(Request) -> Pin<Box<dyn Future<Output = Response> + Send>> + Clone + Send + Sync + 'static
    {
        let files = Arc::new(self);
        move |request| {
            let files = files.clone();
            Box::pin(async move { files.serve(request).await })
        }
    }

    async fn serve(self: Arc<Self>, request: Request) -> Response {
        let is_head = request.method() == Method::HEAD;
        if request.method() != Method::GET && !is_head {
            return not_found();
        }

        let Some(relative_path) = safe_relative_path(request.path()) else {
            return not_found();
        };

        let Ok(root) = tokio::fs::canonicalize(&self.root).await else {
            return not_found();
        };

        let candidate = root.join(relative_path);
        let Ok(file_path) = tokio::fs::canonicalize(candidate).await else {
            return not_found();
        };

        if !file_path.starts_with(&root) {
            return not_found();
        }

        let Ok(file) = tokio::fs::File::open(&file_path).await else {
            return not_found();
        };

        let Ok(metadata) = file.metadata().await else {
            return not_found();
        };
        if !metadata.is_file() {
            return not_found();
        }

        let content_type = content_type_for(&file_path).await;
        let content_length = metadata.len();

        let mut response = if is_head {
            // HEAD carries no body, so the stat-derived length is reported
            // directly; there is no body for it to disagree with.
            drop(file);
            HttpResponse::bytes_body(Bytes::new(), content_type)
                .header("Content-Length", content_length.to_string())
        } else {
            // Buffered at or below 1 MiB, streamed above it; see
            // `file_body` for how each keeps `Content-Length` honest.
            let Ok(response) = file_body(file, content_length, content_type).await else {
                return not_found();
            };
            response
        };

        if let Some(value) = &self.cache_control {
            response = response.header("Cache-Control", value.clone());
        }

        Ok(response)
    }
}

fn not_found() -> Response {
    Ok(HttpResponse::text(crate::http::NOT_FOUND_BODY).status(404))
}

fn safe_relative_path(path: &str) -> Option<PathBuf> {
    let decoded = decode_url_path(path)?;
    let relative = decoded.strip_prefix('/')?;
    if relative.is_empty()
        || relative.starts_with('/')
        || relative.contains('\0')
        || relative.contains('\\')
        || has_windows_drive_prefix(relative)
        || has_dot_component(relative)
    {
        return None;
    }

    let path = Path::new(relative);
    if path.is_absolute() {
        return None;
    }

    let mut safe = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                if starts_with_dot(part) {
                    return None;
                }
                safe.push(part);
            }
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => {
                return None;
            }
        }
    }

    if safe.as_os_str().is_empty() {
        None
    } else {
        Some(safe)
    }
}

/// Dotfiles (`.env`, `.git/`, `.htpasswd`, …) routinely hold secrets and
/// repository internals, so any path segment whose name begins with the dot
/// character is refused. This is broader than the `.`/`..` traversal check:
/// it hides every hidden file and directory, not just the relative cursors.
fn starts_with_dot(segment: &std::ffi::OsStr) -> bool {
    segment.as_encoded_bytes().first() == Some(&b'.')
}

fn decode_url_path(path: &str) -> Option<String> {
    let encoded = path
        .replace('+', "%2B")
        .replace('&', "%26")
        .replace('=', "%3D");
    let query = format!("path={encoded}");
    url::form_urlencoded::parse(query.as_bytes())
        .next()
        .map(|(_, value)| value.into_owned())
}

fn has_windows_drive_prefix(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn has_dot_component(path: &str) -> bool {
    path.split('/')
        .any(|segment| segment == "." || segment == "..")
}

async fn content_type_for(path: &Path) -> String {
    let mime = match mime_from_extension(path) {
        Some(mime) => mime,
        None => {
            let prefix = sniff_prefix(path).await.unwrap_or_default();
            mime_from_content(&prefix).unwrap_or("application/octet-stream")
        }
    };

    with_charset(mime)
}

fn mime_from_content(bytes: &[u8]) -> Option<&'static str> {
    if let Some(kind) = infer::get(bytes) {
        return Some(kind.mime_type());
    }

    let text = std::str::from_utf8(bytes).ok()?;
    let trimmed = text.trim_start_matches('\u{feff}').trim_start();
    let prefix = trimmed
        .chars()
        .take(64)
        .collect::<String>()
        .to_ascii_lowercase();

    if prefix.starts_with("<svg") {
        return Some("image/svg+xml");
    }
    if prefix.starts_with("<?xml") {
        return Some("application/xml");
    }
    if (trimmed.starts_with('{') || trimmed.starts_with('['))
        && serde_json::from_str::<serde_json::Value>(trimmed).is_ok()
    {
        return Some("application/json");
    }

    Some("text/plain")
}

async fn sniff_prefix(path: &Path) -> Option<Vec<u8>> {
    let mut file = tokio::fs::File::open(path).await.ok()?;
    let mut prefix = vec![0; SNIFF_PREFIX_BYTES];
    let read = file.read(&mut prefix).await.ok()?;
    prefix.truncate(read);
    Some(prefix)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn safe_relative_path_rejects_dotfiles() {
        assert!(safe_relative_path("/.env").is_none());
        assert!(safe_relative_path("/.git/config").is_none());
        assert!(safe_relative_path("/.htpasswd").is_none());
        assert!(safe_relative_path("/assets/.secret").is_none());
        // Percent-encoded leading dot is rejected too.
        assert!(safe_relative_path("/%2eenv").is_none());

        // Ordinary assets and dotted file names that do not start a segment
        // with a dot remain reachable.
        assert_eq!(
            safe_relative_path("/assets/app.css"),
            Some(PathBuf::from("assets/app.css"))
        );
        assert_eq!(
            safe_relative_path("/app.min.js"),
            Some(PathBuf::from("app.min.js"))
        );
    }
}
