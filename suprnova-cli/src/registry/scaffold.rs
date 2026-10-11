//! The tree `suprnova live:registry new` scaffolds (REG-018, REG-025): the
//! example component, written into `components/counter/`, and the files the
//! preview application adds to the application `suprnova new` generates.
//!
//! Every value substituted here is a namespace that passed REG-004's closed
//! character set, or a name derived from one, so no template needs escaping.

use crate::templates::render_placeholders;

/// The example component's directory under `components/`.
pub const EXAMPLE: &str = "counter";

/// The example component's files, by manifest name, in the manifest's order,
/// followed by `manifest.json` itself.
pub fn example_component(namespace: &str) -> Vec<(&'static str, String)> {
    let values = [("{namespace}", namespace)];
    [
        (
            "counter.html",
            include_str!("../templates/files/library/counter/counter.html.tpl"),
        ),
        (
            "counter.css",
            include_str!("../templates/files/library/counter/counter.css.tpl"),
        ),
        (
            "counter.js",
            include_str!("../templates/files/library/counter/counter.js.tpl"),
        ),
        (
            "counter.rs",
            include_str!("../templates/files/library/counter/counter.rs.tpl"),
        ),
        (
            "manifest.json",
            include_str!("../templates/files/library/counter/manifest.json.tpl"),
        ),
    ]
    .into_iter()
    .map(|(name, template)| (name, render_placeholders(template, &values)))
    .collect()
}

/// `library.json` as `live:registry new` writes it: the keys of REG-025 in
/// the order the specification lists them, with a title and description.
pub fn library_json(
    namespace: &str,
    source: &str,
    version: &str,
    framework: &str,
    public_key: &str,
) -> String {
    library_json_text(&LibraryText {
        namespace,
        source,
        version,
        framework,
        public_key,
        previous_keys: &[],
        title: Some(namespace),
        description: Some("Suprnova Live components."),
    })
}

/// One `previousKeys` entry (REG-033): the former key, the fingerprint of
/// the key it hands over to, and its signature over the handover statement.
pub struct PreviousKeyText<'a> {
    /// `publicKey`: the former key, `ed25519:<base64>`.
    pub public_key: &'a str,
    /// `next`: the new key's fingerprint.
    pub next: &'a str,
    /// `signature`: base64 of the former key's signature.
    pub signature: &'a str,
}

/// Every value a `library.json` holds, as text.
pub struct LibraryText<'a> {
    /// `namespace`.
    pub namespace: &'a str,
    /// `source`.
    pub source: &'a str,
    /// `version`.
    pub version: &'a str,
    /// `framework`.
    pub framework: &'a str,
    /// `publicKey`.
    pub public_key: &'a str,
    /// `previousKeys`, written only when not empty.
    pub previous_keys: &'a [PreviousKeyText<'a>],
    /// `title`, when given.
    pub title: Option<&'a str>,
    /// `description`, when given.
    pub description: Option<&'a str>,
}

/// Writes `library.json` with its keys in the order REG-025 and REG-033
/// list them, two-space indented, each value a JSON string, so every
/// library's file reads the same way.
pub fn library_json_text(library: &LibraryText<'_>) -> String {
    let quoted =
        |value: &str| serde_json::to_string(value).unwrap_or_else(|_| String::from("\"\""));
    let mut lines = vec![
        format!("  \"namespace\": {}", quoted(library.namespace)),
        format!("  \"source\": {}", quoted(library.source)),
        format!("  \"version\": {}", quoted(library.version)),
        format!("  \"framework\": {}", quoted(library.framework)),
        format!("  \"publicKey\": {}", quoted(library.public_key)),
    ];
    if !library.previous_keys.is_empty() {
        let entries: Vec<String> = library
            .previous_keys
            .iter()
            .map(|entry| {
                format!(
                    "    {{\n      \"publicKey\": {},\n      \"next\": {},\n      \"signature\": {}\n    }}",
                    quoted(entry.public_key),
                    quoted(entry.next),
                    quoted(entry.signature)
                )
            })
            .collect();
        lines.push(format!(
            "  \"previousKeys\": [\n{}\n  ]",
            entries.join(",\n")
        ));
    }
    if let Some(title) = library.title {
        lines.push(format!("  \"title\": {}", quoted(title)));
    }
    if let Some(description) = library.description {
        lines.push(format!("  \"description\": {}", quoted(description)));
    }
    format!("{{\n{}\n}}\n", lines.join(",\n"))
}

/// The preview's `src/live/mod.rs`: the registry builder in the scaffold's
/// form, and the Live routes with an optional sign-in.
pub fn preview_live_module(namespace: &str, namespace_module: &str) -> String {
    render_placeholders(
        include_str!("../templates/files/library/preview/live_mod.rs.tpl"),
        &[
            ("{namespace}", namespace),
            ("{namespace_module}", namespace_module),
        ],
    )
}

/// The preview's `src/live/<namespace_module>/mod.rs`: each component's
/// Rust file, included by `#[path]` from where it sits.
pub fn preview_namespace_module(namespace: &str) -> String {
    render_placeholders(
        include_str!("../templates/files/library/preview/namespace_mod.rs.tpl"),
        &[("{namespace}", namespace)],
    )
}

/// The preview's `src/preview.rs`: the asset route and one page per
/// component.
pub fn preview_pages(namespace: &str, namespace_module: &str) -> String {
    render_placeholders(
        include_str!("../templates/files/library/preview/preview.rs.tpl"),
        &[
            ("{namespace}", namespace),
            ("{namespace_module}", namespace_module),
        ],
    )
}

/// The preview's `askama.toml`: its own `templates/` and the library's
/// `../components/` as the two template roots.
pub fn preview_askama_toml(namespace: &str) -> String {
    render_placeholders(
        include_str!("../templates/files/library/preview/askama.toml.tpl"),
        &[("{namespace}", namespace)],
    )
}

/// The preview's `templates/<namespace>-ui/<directory>/<file>`: one line
/// that includes the library's view from where it sits, under the name an
/// application installs it at.
pub fn preview_view_stub(namespace: &str, directory: &str, file: &str) -> String {
    render_placeholders(
        include_str!("../templates/files/library/preview/view_stub.html.tpl"),
        &[
            ("{namespace}", namespace),
            ("{directory}", directory),
            ("{file}", file),
        ],
    )
}

/// The preview's `templates/_preview/page.html`.
pub fn preview_page_view(namespace: &str) -> String {
    render_placeholders(
        include_str!("../templates/files/library/preview/page.html.tpl"),
        &[("{namespace}", namespace)],
    )
}

/// The preview's `tests/counter.rs`, which starts the preview's own server
/// and reads the counter's page.
pub fn preview_counter_test(namespace: &str, package: &str) -> String {
    render_placeholders(
        include_str!("../templates/files/library/preview/counter_test.rs.tpl"),
        &[("{namespace}", namespace), ("{package}", package)],
    )
}
