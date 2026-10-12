//! Vite asset tags for any page, Laravel's `Vite` facade.
//!
//! The Inertia shell and [`Vite::tags`] write their `<script>` and `<link>`
//! tags through one function, so a page outside Inertia (an Askama view, an
//! error page, a mail preview) loads the same build the Inertia shell does,
//! and a later change to the tags (a CSP nonce, an integrity attribute)
//! reaches both.
//!
//! Which files the tags name depends on the mode and the hot file:
//!
//! - In production, the build manifest's files. The hot file is never
//!   read, so a stale `public/hot` left in a deploy cannot point visitors
//!   at a dev server.
//! - In development, the Vite dev server while the hot file exists (its
//!   content is the server's URL), the manifest's files while it does not
//!   and a manifest exists, and the configured dev server when there is
//!   neither. `suprnova serve` writes the hot file only for a frontend
//!   that declares `@inertiajs/vite`, so a frontend without it keeps the
//!   dev server until a build writes a manifest.

use std::borrow::Cow;
use std::path::PathBuf;

use super::config::{Frontend, InertiaConfig};
use super::manifest::ViteManifest;
use super::response::escape_html_attr;
use crate::FrameworkError;

/// Laravel's `Vite` facade: the asset tags of the installed Inertia
/// configuration, for any page that loads the frontend build.
///
/// Every call reads the configuration [`crate::Inertia::install`] retained,
/// or the defaults when nothing was installed, so the tags match the ones
/// the Inertia shell writes.
///
/// ```rust,no_run
/// use suprnova::{FrameworkError, Vite};
///
/// fn head() -> Result<String, FrameworkError> {
///     // The tags of the configured entry points.
///     let all = Vite::to_html()?;
///     // The tags of entry points you name.
///     let admin = Vite::tags(["src/admin.ts", "src/admin.css"])?;
///     Ok(format!("{all}{admin}"))
/// }
/// ```
pub struct Vite;

impl Vite {
    /// The tags of `entry_points`: see [`InertiaConfig::vite_tags`].
    ///
    /// # Errors
    ///
    /// Returns an error naming the manifest's path when the tags need the
    /// manifest and it does not exist or cannot be parsed, and an error
    /// naming the entry when the manifest lacks one.
    pub fn tags<I, S>(entry_points: I) -> Result<String, FrameworkError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        installed().vite_tags(entry_points)
    }

    /// The tags of the configured entry points: the configuration's
    /// `entry_point` and those [`InertiaConfig::entry_points`] added, the
    /// same tags the Inertia shell writes. Laravel's `Vite::toHtml`.
    ///
    /// # Errors
    ///
    /// As [`Self::tags`]. The Inertia shell answers a missing manifest
    /// with its own fallback instead.
    pub fn to_html() -> Result<String, FrameworkError> {
        let config = installed();
        config.vite_tags(config.configured_entry_points())
    }

    /// The hot file's path: see [`InertiaConfig::hot_file`]. Laravel's
    /// `Vite::hotFile`.
    pub fn hot_file() -> PathBuf {
        installed().hot_file().to_path_buf()
    }

    /// Whether the Vite dev server runs, which the hot file's existence
    /// says. Laravel's `Vite::isRunningHot`.
    ///
    /// Always `false` in production, where the tags never point at the dev
    /// server and the hot file is never looked at.
    pub fn is_running_hot() -> bool {
        is_running_hot(&installed())
    }

    /// The dev server's URL: the hot file's trimmed content, the configured
    /// dev server URL when the file is empty, and `None` without a hot file.
    /// Laravel's `Vite::devServerUrl`.
    ///
    /// Always `None` in production, where the hot file is never read.
    pub fn dev_server_url() -> Option<String> {
        dev_server_url(&installed())
    }
}

/// The configuration the facade reads.
fn installed() -> InertiaConfig {
    crate::App::inertia_registry()
        .installed_config()
        .unwrap_or_default()
}

/// Whether `config` runs hot: development with the hot file present.
pub(crate) fn is_running_hot(config: &InertiaConfig) -> bool {
    config.development && config.hot_file().is_file()
}

/// The dev server's URL from the hot file, in development only.
pub(crate) fn dev_server_url(config: &InertiaConfig) -> Option<String> {
    if !config.development {
        return None;
    }
    config.hot_file_url()
}

/// Why tags could not be rendered, kept apart from [`FrameworkError`] so
/// the Inertia shell, which answers each case with a fallback, builds no
/// error on its hot path.
pub(crate) enum TagError {
    /// Production found no manifest at the configured path.
    MissingManifest,
    /// The manifest could not be read or parsed.
    Load(FrameworkError),
    /// The manifest lacks this entry point.
    MissingEntry(String),
}

impl TagError {
    pub(crate) fn into_framework_error(self, config: &InertiaConfig) -> FrameworkError {
        match self {
            Self::MissingManifest => match ViteManifest::load(&config.manifest_path) {
                Err(error) => error,
                Ok(_) => FrameworkError::internal(format!(
                    "Vite manifest not found at {} when it was first read; it exists now, \
                     and is read on the next start",
                    config.manifest_path.display()
                )),
            },
            Self::Load(error) => error,
            Self::MissingEntry(entry) => FrameworkError::internal(format!(
                "Unable to locate file in Vite manifest: {entry} (manifest {})",
                config.manifest_path.display()
            )),
        }
    }
}

/// What the tags name.
enum Source<'a> {
    /// The Vite dev server at this URL.
    DevServer(Cow<'a, str>),
    /// The build manifest's files.
    Manifest(Cow<'a, ViteManifest>),
}

/// Choose the source by the rules in the module documentation.
fn source(config: &InertiaConfig) -> Result<Source<'_>, TagError> {
    if !config.development {
        // The cached manifest: a production server reads its build once.
        return config
            .vite_manifest()
            .map(|manifest| Source::Manifest(Cow::Borrowed(manifest)))
            .ok_or(TagError::MissingManifest);
    }
    if let Some(url) = config.hot_file_url() {
        return Ok(Source::DevServer(Cow::Owned(url)));
    }
    if config.manifest_path.is_file() {
        // Read on every call: in development a build can write, rewrite or
        // remove the manifest while the server runs.
        return ViteManifest::load(&config.manifest_path)
            .map(|manifest| Source::Manifest(Cow::Owned(manifest)))
            .map_err(TagError::Load);
    }
    Ok(Source::DevServer(Cow::Borrowed(&config.vite_dev_server)))
}

/// The tags of `entries` under `config`.
pub(crate) fn tags(config: &InertiaConfig, entries: &[&str]) -> Result<String, TagError> {
    match source(config)? {
        Source::DevServer(server) => Ok(dev_server_tags(config, &server, entries)),
        Source::Manifest(manifest) => manifest_tags(config, &manifest, entries),
    }
}

/// The tags the Inertia shell writes for the configured entry points.
///
/// Where [`tags`] fails, the shell keeps its own answer: the configured
/// dev server in development, and in production the legacy
/// `{assets_base_url}/main.js` and `main.css` that applications built
/// before the manifest layer still serve.
pub(crate) fn shell_tags(config: &InertiaConfig) -> String {
    let entries = config.configured_entry_points();
    match tags(config, &entries) {
        Ok(tags) => tags,
        Err(error) if config.development => {
            if let TagError::Load(error) = &error {
                tracing::warn!(
                    path = %config.manifest_path.display(),
                    %error,
                    "the Vite manifest cannot be read; the page loads from the dev server"
                );
            }
            dev_server_tags(config, &config.vite_dev_server, &entries)
        }
        // The manifest's absence was logged when it was first read.
        Err(_) => legacy_tags(config),
    }
}

/// Dev-server tags: the React refresh preamble for a React frontend, each
/// stylesheet entry, the Vite client, then each script entry.
pub(crate) fn dev_server_tags(config: &InertiaConfig, server: &str, entries: &[&str]) -> String {
    let server = server.trim_end_matches('/');
    let url = |path: &str| escape_html_attr(&format!("{server}/{path}"));
    let mut out = String::new();
    // React requires the `@react-refresh` preamble before any module
    // loads; Svelte and Vue have HMR built into their Vite plugins.
    if config.frontend == Frontend::React {
        // `serde_json::to_string` writes a double-quoted JSON literal with
        // every `\`, quote and control byte escaped; the quotes are swapped
        // for the single quotes the import uses, and `<` is escaped so the
        // URL, which can come from the hot file, cannot end the element.
        let js_server = serde_json::to_string(server).unwrap_or_else(|_| "\"\"".to_string());
        let js_server = js_server
            .trim_matches('"')
            .replace('\'', "\\'")
            .replace('<', "\\u003c");
        out.push_str(&format!(
            "<script type=\"module\">\n\
             import RefreshRuntime from '{js_server}/@react-refresh'\n\
             RefreshRuntime.injectIntoGlobalHook(window)\n\
             window.$RefreshReg$ = () => {{}}\n\
             window.$RefreshSig$ = () => (type) => type\n\
             window.__vite_plugin_react_preamble_installed__ = true\n\
             </script>\n"
        ));
    }
    for entry in entries.iter().filter(|entry| is_css_path(entry)) {
        out.push_str(&stylesheet_tag(&url(entry)));
    }
    out.push_str(&script_tag(&url("@vite/client")));
    for entry in entries.iter().filter(|entry| !is_css_path(entry)) {
        out.push_str(&script_tag(&url(entry)));
    }
    out
}

/// Manifest tags for every entry, deduplicated: the stylesheets, then the
/// scripts, then a `modulepreload` for each imported chunk that is not
/// itself a script entry.
fn manifest_tags(
    config: &InertiaConfig,
    manifest: &ViteManifest,
    entries: &[&str],
) -> Result<String, TagError> {
    let mut stylesheets: Vec<String> = Vec::new();
    let mut scripts: Vec<String> = Vec::new();
    let mut preloads: Vec<String> = Vec::new();
    for entry in entries {
        let assets = manifest
            .resolve_entry(entry)
            .ok_or_else(|| TagError::MissingEntry((*entry).to_string()))?;
        for file in assets.js {
            if is_css_path(&file) {
                push_unique(&mut stylesheets, file);
            } else {
                push_unique(&mut scripts, file);
            }
        }
        for css in assets.css {
            push_unique(&mut stylesheets, css);
        }
        for chunk in assets.preload {
            push_unique(&mut preloads, chunk);
        }
    }
    preloads.retain(|chunk| !scripts.contains(chunk));

    let base = asset_base(&config.assets_base_url);
    let url = |file: &str| escape_html_attr(&format!("{base}/{file}"));
    let mut out = String::new();
    for css in &stylesheets {
        out.push_str(&stylesheet_tag(&url(css)));
    }
    for js in &scripts {
        out.push_str(&script_tag(&url(js)));
    }
    for chunk in &preloads {
        out.push_str(&format!(
            "<link rel=\"modulepreload\" href=\"{}\">\n",
            url(chunk)
        ));
    }
    Ok(out)
}

/// The production shell's answer to a missing manifest or entry.
fn legacy_tags(config: &InertiaConfig) -> String {
    let base = asset_base(&config.assets_base_url);
    let url = |file: &str| escape_html_attr(&format!("{base}/{file}"));
    format!(
        "{}{}",
        script_tag(&url("main.js")),
        stylesheet_tag(&url("main.css"))
    )
}

fn script_tag(src: &str) -> String {
    format!("<script type=\"module\" src=\"{src}\"></script>\n")
}

fn stylesheet_tag(href: &str) -> String {
    format!("<link rel=\"stylesheet\" href=\"{href}\">\n")
}

fn push_unique(list: &mut Vec<String>, item: String) {
    if !list.contains(&item) {
        list.push(item);
    }
}

/// Whether `path` names a stylesheet Vite builds, by the extensions
/// Laravel's `Vite::isCssPath` checks, a query string allowed after it.
fn is_css_path(path: &str) -> bool {
    let path = path.split('?').next().unwrap_or(path);
    path.rsplit_once('.').is_some_and(|(_, extension)| {
        matches!(
            extension,
            "css" | "less" | "sass" | "scss" | "styl" | "stylus" | "pcss" | "postcss"
        )
    })
}

/// The base the tags name their files under (PFX-005).
///
/// A root-relative `assets_base_url`, one that starts with a single `/`
/// such as the default `/assets`, is served by the application and gets
/// the public root in front. An absolute or network-path base (a CDN) is
/// another host's path and is left as it is.
///
/// The base is classified before its trailing slashes are removed, so a
/// base of `/`, which serves the assets at the root itself, is root-relative
/// too and gives the root alone.
fn asset_base(assets_base_url: &str) -> Cow<'_, str> {
    let trimmed = assets_base_url.trim_end_matches('/');
    if assets_base_url.starts_with('/') && !assets_base_url.starts_with("//") {
        Cow::Owned(crate::routing::root::prefixed(trimmed))
    } else {
        Cow::Borrowed(trimmed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn css_paths_follow_laravels_extensions() {
        for path in ["a.css", "src/a.scss", "a.pcss?inline", "a.postcss"] {
            assert!(is_css_path(path), "{path}");
        }
        for path in ["a.ts", "a.css.ts", "src/main.tsx", "noextension"] {
            assert!(!is_css_path(path), "{path}");
        }
    }

    #[test]
    fn dev_head_includes_react_preamble_for_react_only() {
        for (frontend, preamble) in [
            (Frontend::React, true),
            (Frontend::Svelte, false),
            (Frontend::Vue, false),
        ] {
            let cfg = InertiaConfig::new().frontend(frontend);
            let head = dev_server_tags(&cfg, &cfg.vite_dev_server, &[cfg.entry_point.as_str()]);
            assert_eq!(head.contains("@react-refresh"), preamble, "{frontend:?}");
            assert_eq!(
                head.contains("__vite_plugin_react_preamble_installed__"),
                preamble,
                "{frontend:?}"
            );
        }
    }

    #[test]
    fn a_dev_server_url_cannot_end_the_react_preamble() {
        let cfg = InertiaConfig::new().frontend(Frontend::React);
        let head = dev_server_tags(
            &cfg,
            "http://x/</script><script>alert(1)//",
            &["src/main.tsx"],
        );
        assert_eq!(head.matches("</script>").count(), 3, "{head}");
        assert!(!head.contains("<script>alert"), "{head}");
    }

    #[test]
    fn dev_head_loads_correct_entry_point_per_frontend() {
        for (frontend, entry) in [
            (Frontend::Svelte, "src/main.ts\""),
            (Frontend::React, "src/main.tsx\""),
            (Frontend::Vue, "src/main.ts\""),
        ] {
            let cfg = InertiaConfig::new().frontend(frontend);
            let head = dev_server_tags(&cfg, &cfg.vite_dev_server, &[cfg.entry_point.as_str()]);
            assert!(head.contains(entry), "{frontend:?}: {head}");
        }
    }
}
