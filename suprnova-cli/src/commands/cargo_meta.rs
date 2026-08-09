//! Helpers for reading the consuming project's `Cargo.toml`.

use std::fs;
use std::path::Path;

/// Parse a Cargo.toml document into a `toml::Table`.
///
/// Cargo.toml is always a TOML *document* (with `[package]`, `[dependencies]`,
/// etc.), not a single TOML *value*. Parsing into `toml::Value` fails on the
/// first `[section]` header with "expected nothing"; `toml::Table` is the
/// correct shape.
pub fn parse_cargo_toml(content: &str) -> Result<toml::Table, toml::de::Error> {
    content.parse::<toml::Table>()
}

/// Extract `[package].name` from already-loaded Cargo.toml content.
pub fn package_name_from_content(content: &str) -> Option<String> {
    let table = parse_cargo_toml(content).ok()?;
    let name = table.get("package")?.get("name")?.as_str()?;
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

/// Read `Cargo.toml` from `path` and return `[package].name`.
pub fn package_name_from_path(path: &Path) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    package_name_from_content(&content)
}

/// The `path` of the `[[bin]]` target whose name matches the package.
///
/// Both scaffolds declare two binaries — the server (named for the
/// package) and `console` — so "the first bin" is not good enough.
pub fn server_bin_path(content: &str) -> Option<String> {
    let table = parse_cargo_toml(content).ok()?;
    let package_name = table.get("package")?.get("name")?.as_str()?;
    let bins = table.get("bin")?.as_array()?;
    bins.iter()
        .find(|bin| bin.get("name").and_then(|n| n.as_str()) == Some(package_name))
        .and_then(|bin| bin.get("path"))
        .and_then(|p| p.as_str())
        .map(str::to_string)
}

/// Which scaffold shape a project was generated from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectKind {
    /// `suprnova new` — server at `cmd/main.rs`, with a `frontend/` that
    /// Vite builds into `public/assets`.
    FullStack,
    /// `suprnova new --api` — server at `src/main.rs`, no `frontend/`
    /// and no `cmd/` at all.
    Api,
}

/// Detect the scaffold shape from a project's `Cargo.toml`.
///
/// The server binary's path is the discriminator, and it is the same fact
/// the Dockerfile needs anyway: `cmd/main.rs` means there is a `cmd/` to
/// copy and a frontend to build, `src/main.rs` means neither exists.
///
/// Defaults to [`ProjectKind::FullStack`] when the manifest cannot be
/// read or declares no matching bin, because that is what every previous
/// version emitted unconditionally — an unparseable manifest should not
/// silently switch an existing project to a different Dockerfile.
pub fn detect_project_kind(content: &str) -> ProjectKind {
    match server_bin_path(content).as_deref() {
        Some(path) if path.starts_with("src/") => ProjectKind::Api,
        _ => ProjectKind::FullStack,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCAFFOLD_CARGO_TOML: &str = r#"[package]
name = "nebula"
version = "0.1.0"
edition = "2024"
rust-version = "1.91.1"
description = "A starter kit for Suprnova"
authors = ["eas4ai <shawn.payments@gmail.com>"]

[[bin]]
name = "nebula"
path = "cmd/main.rs"

[[bin]]
name = "console"
path = "src/bin/console.rs"

[dependencies]
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v0.6.0" }
tokio = { version = "1", features = ["full"] }
"#;

    #[test]
    fn parses_scaffold_shaped_cargo_toml_as_document_not_single_value() {
        let table = parse_cargo_toml(SCAFFOLD_CARGO_TOML).expect(
            "scaffold-shaped Cargo.toml must parse as a TOML document; \
             regression guard against re-introducing `toml::Value` here",
        );
        assert!(table.contains_key("package"));
        assert!(table.contains_key("dependencies"));
    }

    #[test]
    fn extracts_package_name_from_scaffold() {
        assert_eq!(
            package_name_from_content(SCAFFOLD_CARGO_TOML).as_deref(),
            Some("nebula")
        );
    }

    #[test]
    fn returns_none_for_invalid_toml() {
        assert_eq!(package_name_from_content("this is not toml ===="), None);
    }

    #[test]
    fn returns_none_when_package_table_absent() {
        assert_eq!(
            package_name_from_content("[workspace]\nmembers = []\n"),
            None
        );
    }

    #[test]
    fn returns_none_when_package_name_empty() {
        assert_eq!(package_name_from_content("[package]\nname = \"\"\n"), None);
    }
}

#[cfg(test)]
mod project_kind_tests {
    //! `docker:init` emitted the full-stack Dockerfile for every project
    //! shape, so an API project's build died on `COPY frontend/package.json`.
    //! These pin the discriminator.

    use super::*;

    const API_MANIFEST: &str = r#"
[package]
name = "nebula"
version = "0.1.0"

[[bin]]
name = "nebula"
path = "src/main.rs"

[[bin]]
name = "console"
path = "src/bin/console.rs"
"#;

    const FULLSTACK_MANIFEST: &str = r#"
[package]
name = "nebula"
version = "0.1.0"

[[bin]]
name = "nebula"
path = "cmd/main.rs"

[[bin]]
name = "console"
path = "src/bin/console.rs"
"#;

    #[test]
    fn the_server_bin_is_found_by_name_not_by_position() {
        // Both scaffolds declare two bins, and `console` sorts first in
        // neither — but relying on order would be luck rather than logic.
        assert_eq!(
            server_bin_path(API_MANIFEST).as_deref(),
            Some("src/main.rs")
        );
        assert_eq!(
            server_bin_path(FULLSTACK_MANIFEST).as_deref(),
            Some("cmd/main.rs")
        );
    }

    #[test]
    fn the_api_scaffold_is_detected_from_its_manifest() {
        assert_eq!(detect_project_kind(API_MANIFEST), ProjectKind::Api);
    }

    #[test]
    fn the_full_stack_scaffold_is_detected_from_its_manifest() {
        assert_eq!(
            detect_project_kind(FULLSTACK_MANIFEST),
            ProjectKind::FullStack
        );
    }

    /// An unreadable or unexpected manifest must keep emitting what every
    /// previous version emitted. Guessing "API" on a parse failure would
    /// hand an existing full-stack project a Dockerfile that silently
    /// drops its frontend.
    #[test]
    fn an_unparseable_manifest_falls_back_to_the_previous_behaviour() {
        for manifest in ["", "not toml at all {{{", "[package]\nname = \"x\"\n"] {
            assert_eq!(
                detect_project_kind(manifest),
                ProjectKind::FullStack,
                "manifest {manifest:?} must not switch an existing project's shape"
            );
        }
    }

    /// A `console`-only manifest has no server bin matching the package
    /// name, so there is nothing to discriminate on.
    #[test]
    fn a_manifest_without_a_matching_bin_falls_back() {
        let manifest = r#"
[package]
name = "nebula"

[[bin]]
name = "console"
path = "src/bin/console.rs"
"#;
        assert_eq!(server_bin_path(manifest), None);
        assert_eq!(detect_project_kind(manifest), ProjectKind::FullStack);
    }
}
