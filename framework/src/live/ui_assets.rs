//! Serves the stylesheet and script a vendored library component keeps
//! beside its view (UI-017).
//!
//! `live:add` installs a component as one directory under the reserved
//! template root, `templates/suprnova-ui/<component>/`, holding its view,
//! its stylesheet and its JavaScript. The view is compiled in; the other
//! two must reach the browser, so this route serves exactly those two file
//! kinds from that directory at `/suprnova-ui/<component>/<file>`. Nothing
//! else under the template root is reachable: a name outside the closed
//! character set, a nested path, a view, or a manifest is a closed 404.

use std::path::{Path, PathBuf};

use bytes::Bytes;
use hyper::Method;
use sha2::{Digest as _, Sha256};

use crate::{HttpResponse, Request, Response};

/// The route the vendored component assets are served on.
pub const LIVE_UI_ASSET_PATH_PREFIX: &str = "/suprnova-ui";
pub(crate) const LIVE_UI_ASSET_ROUTE: &str = "/suprnova-ui/{component}/{file}";
/// The template root `live:add` installs library components under.
pub const LIVE_UI_TEMPLATE_ROOT: &str = "suprnova-ui";
const MAX_ASSET_BYTES: u64 = 1024 * 1024;
const CACHE_CONTROL: &str = "public, max-age=0, must-revalidate";

/// The directory the vendored components live in for this process.
#[derive(Clone, Debug)]
pub(crate) struct LiveUiAssets {
    root: PathBuf,
}

impl LiveUiAssets {
    pub(crate) fn from_root(root: PathBuf) -> Self {
        Self { root }
    }

    pub(crate) async fn serve(&self, request: Request) -> Response {
        let is_head = request.method() == Method::HEAD;
        if request.method() != Method::GET && !is_head {
            return Ok(closed(405).header("Allow", "GET, HEAD"));
        }
        if request.query().is_some() {
            return Ok(closed(404));
        }
        let (Ok(component), Ok(file)) = (request.param("component"), request.param("file")) else {
            return Ok(closed(404));
        };
        if !component_name(component) || !asset_name(file) {
            return Ok(closed(404));
        }
        let content_type = if Path::new(file).extension().is_some_and(|ext| ext == "css") {
            "text/css; charset=utf-8"
        } else {
            "text/javascript; charset=utf-8"
        };
        let path = self.root.join(component).join(file);
        let Ok(metadata) = tokio::fs::metadata(&path).await else {
            return Ok(closed(404));
        };
        if !metadata.is_file() || metadata.len() > MAX_ASSET_BYTES {
            return Ok(closed(404));
        }
        let Ok(bytes) = tokio::fs::read(&path).await else {
            return Ok(closed(404));
        };
        let etag = format!("\"{}\"", hex(&Sha256::digest(&bytes)));
        if request.header("if-none-match").is_some_and(|header| {
            header
                .split(',')
                .any(|tag| tag.trim() == etag || tag.trim() == "*")
        }) {
            return Ok(HttpResponse::new()
                .status(304)
                .header("ETag", etag)
                .header("Cache-Control", CACHE_CONTROL)
                .header("X-Content-Type-Options", "nosniff"));
        }
        let length = bytes.len();
        let body = if is_head {
            Bytes::new()
        } else {
            Bytes::from(bytes)
        };
        Ok(HttpResponse::bytes_body(body, content_type)
            .header("Content-Length", length.to_string())
            .header("Cache-Control", CACHE_CONTROL)
            .header("ETag", etag)
            .header("X-Content-Type-Options", "nosniff"))
    }
}

/// One closed segment of lowercase letters, digits and hyphens.
fn component_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !value.starts_with('-')
        && !value.ends_with('-')
}

/// A component name followed by `.css` or `.js`, nothing else.
fn asset_name(value: &str) -> bool {
    let Some((stem, extension)) = value.rsplit_once('.') else {
        return false;
    };
    component_name(stem) && matches!(extension, "css" | "js")
}

fn closed(status: u16) -> HttpResponse {
    HttpResponse::new()
        .status(status)
        .header("Cache-Control", "no-store")
        .header("X-Content-Type-Options", "nosniff")
}

fn hex(digest: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut text = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(&mut text, "{byte:02x}").expect("formatting into a String cannot fail");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::{asset_name, component_name};

    #[test]
    fn names_are_closed_lowercase_segments() {
        assert!(component_name("password-input"));
        assert!(asset_name("password-input.js"));
        assert!(asset_name("field.css"));
        for hostile in [
            "",
            "-x",
            "x-",
            "Field",
            "a/b",
            "..",
            "field.html",
            "manifest.json",
            "x.CSS",
        ] {
            assert!(
                !component_name(hostile) || !asset_name(hostile),
                "{hostile}"
            );
        }
        assert!(!asset_name("field.html"));
        assert!(!asset_name("manifest.json"));
        assert!(!component_name(".."));
    }
}
