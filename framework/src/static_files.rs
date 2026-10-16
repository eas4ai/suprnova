//! Framework-native static file fallback serving.
//!
//! [`StaticFiles`] is intended for `fallback!` registration: normal routes
//! win first, and safe `GET` / `HEAD` misses can be resolved from a configured
//! public directory without handing the request to application code.
//!
//! A request path is resolved through held directory handles, never checked
//! and then opened again by name (HRD-005): a symbolic link is followed only
//! while it stays inside the root, and a directory swapped for a link while
//! the request is in flight fails the request instead of redirecting it. The
//! length and the content type a response declares come from the handle its
//! body is read from.

use crate::app::paths::public_path;
use crate::http::file_response::{file_body, mime_from_extension, with_charset};
use crate::http::{HttpResponse, Request, Response};
use bytes::Bytes;
use hyper::Method;
use std::fs::File;
use std::future::Future;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

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

        let root = self.root.clone();
        let Ok(Some(OpenedFile {
            file,
            length: content_length,
            content_type,
        })) = tokio::task::spawn_blocking(move || open_for_serving(&root, &relative_path)).await
        else {
            return not_found();
        };
        let file = tokio::fs::File::from_std(file);

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

/// A file opened for serving, with the length and the content type its
/// response declares, both taken from the handle the body is read from.
struct OpenedFile {
    file: File,
    length: u64,
    content_type: String,
}

/// Open the regular file at `relative` under `root` for serving. `None`
/// when there is nothing there to serve, or it cannot be opened or read.
fn open_for_serving(root: &Path, relative: &Path) -> Option<OpenedFile> {
    let (mut file, length, name) = open_beneath(root, relative)?;
    let content_type = content_type_for(&name, &mut file).ok()?;
    Some(OpenedFile {
        file,
        length,
        content_type,
    })
}

/// Open the regular file at `relative` under `root` through held directory
/// handles, and return it with its length and the name it has in its
/// directory.
///
/// The root is resolved once, so a root that is itself a symbolic link
/// still serves. Below it, the walk follows a symbolic link only while the
/// link stays inside the root, and opens every directory and the file with
/// `O_NOFOLLOW`, so a directory swapped for a link after the walk passed
/// it fails the open instead of redirecting it. The length comes from the
/// opened handle, and the name is the one the walk reached, so a link
/// inside the root takes the type of the file it leads to.
#[cfg(unix)]
fn open_beneath(root: &Path, relative: &Path) -> Option<(File, u64, PathBuf)> {
    use crate::confined_walk::{ConfinedRoot, Last, open_file_in};

    let relative = relative.to_str()?;
    let entry = ConfinedRoot::new(root)
        .ok()?
        .resolve(relative, Last::Follow, false)
        .ok()?;
    let (file, metadata) = open_file_in(&entry.dir, &entry.name, relative).ok()??;
    Some((file, metadata.len(), PathBuf::from(entry.name)))
}

/// Where `openat` and `O_NOFOLLOW` are unavailable there is no way to keep
/// a request inside the root once a directory on its path can be swapped
/// for a symbolic link, so nothing is served: every request is a 404
/// rather than a check that a rename can defeat (HRD-005). The framework
/// builds on Unix alone today (its process and signal handling are Unix
/// calls), so this arm is reached by no supported build.
#[cfg(not(unix))]
fn open_beneath(_root: &Path, _relative: &Path) -> Option<(File, u64, PathBuf)> {
    None
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

/// The `Content-Type` of the file `name`, open as `file`: the type its
/// extension implies, or else the type its first bytes show. Those bytes
/// are read from the handle the body is sent from, which is then rewound
/// to its start, so the type always describes the file the response sends.
fn content_type_for(name: &Path, file: &mut File) -> std::io::Result<String> {
    let mime = match mime_from_extension(name) {
        Some(mime) => mime,
        None => {
            let mut prefix = Vec::with_capacity(SNIFF_PREFIX_BYTES);
            Read::by_ref(file)
                .take(SNIFF_PREFIX_BYTES as u64)
                .read_to_end(&mut prefix)?;
            file.seek(SeekFrom::Start(0))?;
            mime_from_content(&prefix).unwrap_or("application/octet-stream")
        }
    };

    Ok(with_charset(mime))
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
