//! Serves the stylesheet and script a vendored library component keeps
//! beside its view (UI-017, REG-017).
//!
//! `live:add` installs a component as one directory under its library's
//! template root, `templates/<namespace>-ui/<component>/`, holding its view,
//! its stylesheet and its JavaScript; the shipped library's root is
//! `templates/suprnova-ui/`. The view is compiled in; the other two must
//! reach the browser, so each library's route serves exactly those two file
//! kinds from that directory at `/<namespace>-ui/<component>/<file>`.
//! Nothing else under the template root is reachable: a name outside the
//! closed character set, a nested path, a view, a manifest, or a file
//! reached through a symbolic link below the root is a closed 404.

use std::path::{Path, PathBuf};

use bytes::Bytes;
use hyper::Method;
use sha2::{Digest as _, Sha256};

use crate::{FrameworkError, HttpResponse, Request, Response};

/// The URL prefix the shipped library's component assets are served under;
/// a third-party library's is `/<namespace>-ui`.
pub const LIVE_UI_ASSET_PATH_PREFIX: &str = "/suprnova-ui";
pub(crate) const LIVE_UI_ASSET_ROUTE: &str = "/suprnova-ui/{component}/{file}";
/// The template root `live:add` installs the shipped library's components
/// under; a third-party library's is `<namespace>-ui`.
pub const LIVE_UI_TEMPLATE_ROOT: &str = "suprnova-ui";
const MAX_ASSET_BYTES: u64 = 1024 * 1024;
const CACHE_CONTROL: &str = "public, max-age=0, must-revalidate";
/// The shipped library's namespace: its route and template root are the
/// constants above, and only `Router::try_live_ui_assets` installs them.
const SHIPPED_NAMESPACE: &str = "suprnova";
/// REG-004: the namespaces only the shipped library may hold.
const RESERVED_NAMESPACES: [&str; 3] = [SHIPPED_NAMESPACE, "sn", "live"];
/// The longest namespace REG-004 admits, in bytes.
const MAX_NAMESPACE_BYTES: usize = 32;
/// Rust keywords plus `lib`, `main` and `build`: `live:add` writes a
/// library's Rust under `src/live/<namespace_module>/`, so no library holds
/// a namespace whose module form is one of these (REG-003, REG-004). The
/// list is the CLI's, so the router admits exactly the namespaces
/// `live:add` can install.
const MODULE_KEYWORDS: [&str; 56] = [
    "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn", "for",
    "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return",
    "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where",
    "while", "async", "await", "dyn", "abstract", "become", "box", "do", "final", "macro",
    "override", "priv", "typeof", "unsized", "virtual", "yield", "try", "gen", "union", "lib",
    "main", "build",
];

/// A third-party library's namespace that passed REG-004's rule, with the
/// route and template root REG-017 derives from it.
#[derive(Debug)]
pub(crate) struct LibraryNamespace {
    route: String,
    template_root: String,
}

impl LibraryNamespace {
    /// Checks `namespace` before it reaches a route pattern or a path. The
    /// closed character set is what keeps route syntax (`{`, `:`, `*`) and
    /// path syntax (`/`, `..`) out of both.
    ///
    /// The shipped namespace is refused here because
    /// `Router::try_live_ui_assets` is its one call: two spellings of one
    /// route would only collide at startup. `sn` and `live` are reserved for
    /// the shipped library and no library holds them, so nothing is ever
    /// installed under their roots.
    pub(crate) fn parse(namespace: &str) -> Result<Self, FrameworkError> {
        if !valid_namespace(namespace) {
            return Err(FrameworkError::internal(format!(
                "{namespace:?} is not a Live library namespace: a namespace is 1 to {MAX_NAMESPACE_BYTES} bytes of lowercase letters, digits and hyphens, starts with a letter, and its module form (hyphens as underscores) is not a Rust keyword"
            )));
        }
        if namespace == SHIPPED_NAMESPACE {
            return Err(FrameworkError::internal(format!(
                "the shipped namespace {SHIPPED_NAMESPACE:?} is served by Router::try_live_ui_assets(); call that instead of try_live_ui_assets_for({SHIPPED_NAMESPACE:?})"
            )));
        }
        if RESERVED_NAMESPACES.contains(&namespace) {
            return Err(FrameworkError::internal(format!(
                "the namespace {namespace:?} is reserved for the shipped library, so no library is installed under it; the shipped components are served by Router::try_live_ui_assets()"
            )));
        }
        Ok(Self {
            route: route_for(namespace),
            template_root: template_root_for(namespace),
        })
    }

    /// The route pattern, `/<namespace>-ui/{component}/{file}`.
    pub(crate) fn route(&self) -> &str {
        &self.route
    }

    /// The directory under `templates/`, `<namespace>-ui`.
    pub(crate) fn template_root(&self) -> &str {
        &self.template_root
    }
}

/// REG-004's namespace rule, the same one `live:add` applies.
fn valid_namespace(namespace: &str) -> bool {
    let module = namespace.replace('-', "_");
    namespace.len() <= MAX_NAMESPACE_BYTES
        && namespace
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_lowercase)
        && namespace
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !MODULE_KEYWORDS.contains(&module.as_str())
}

fn route_for(namespace: &str) -> String {
    format!("/{namespace}-ui/{{component}}/{{file}}")
}

fn template_root_for(namespace: &str) -> String {
    format!("{namespace}-ui")
}

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
        // A file is served only where it sits. Resolving both the root and
        // the file refuses a component directory or file that is a symbolic
        // link below the root, which could otherwise put a view, the
        // install record, `suprnova.toml` or any file on the host behind an
        // asset name. The root itself may be a link, as a deployment's
        // templates directory often is.
        let (Ok(root), Ok(path)) = (
            tokio::fs::canonicalize(&self.root).await,
            tokio::fs::canonicalize(self.root.join(component).join(file)).await,
        ) else {
            return Ok(closed(404));
        };
        if path != root.join(component).join(file) {
            return Ok(closed(404));
        }
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
    use super::{
        LIVE_UI_ASSET_PATH_PREFIX, LIVE_UI_ASSET_ROUTE, LIVE_UI_TEMPLATE_ROOT, LibraryNamespace,
        SHIPPED_NAMESPACE, asset_name, component_name, route_for, template_root_for,
        valid_namespace,
    };

    #[test]
    fn the_shipped_route_is_the_suprnova_namespace_case() {
        assert_eq!(route_for(SHIPPED_NAMESPACE), LIVE_UI_ASSET_ROUTE);
        assert_eq!(template_root_for(SHIPPED_NAMESPACE), LIVE_UI_TEMPLATE_ROOT);
        assert_eq!(
            format!("/{}", template_root_for(SHIPPED_NAMESPACE)),
            LIVE_UI_ASSET_PATH_PREFIX
        );
        let acme = LibraryNamespace::parse("acme").expect("a valid namespace");
        assert_eq!(acme.route(), "/acme-ui/{component}/{file}");
        assert_eq!(acme.template_root(), "acme-ui");
    }

    #[test]
    fn namespaces_follow_the_cli_rule() {
        for ok in ["acme", "acme-ui", "a1", "a", "acme-"] {
            assert!(valid_namespace(ok), "{ok}");
        }
        for hostile in [
            "",
            "1acme",
            "-acme",
            "Acme",
            "self",
            "a_b",
            "a.b",
            "a/b",
            "{a}",
            &"a".repeat(33),
        ] {
            assert!(!valid_namespace(hostile), "{hostile}");
        }
        for reserved in ["suprnova", "sn", "live"] {
            assert!(valid_namespace(reserved), "{reserved}");
            assert!(LibraryNamespace::parse(reserved).is_err(), "{reserved}");
        }
    }

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
