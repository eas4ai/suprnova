//! The library tree, `library.json` and a component's `manifest.json`
//! (REG-002, REG-003, REG-004, REG-025, REG-033).

use super::signing::{KeyHandover, PublicKey};
use super::{RegistryError, Result};

/// The largest JSON document a library may carry (REG-002, REG-009).
pub const MAX_JSON_BYTES: usize = 1024 * 1024;

/// The largest file a manifest may name (REG-003).
pub const MAX_FILE_BYTES: usize = 1024 * 1024;

/// Namespaces only the shipped library may use (REG-004).
pub const RESERVED_NAMESPACES: [&str; 3] = ["suprnova", "sn", "live"];

/// `library.json`: one object with exactly these keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryJson {
    /// The library's namespace (REG-004).
    pub namespace: String,
    /// The canonical address the library is published at (REG-008).
    pub source: String,
    /// The library's version, equal to the release tag's (REG-026).
    pub version: semver::Version,
    /// The framework versions the library admits (REG-007).
    pub framework: semver::VersionReq,
    /// The signing key (REG-024).
    pub public_key: PublicKey,
    /// Former keys, each vouching for the next (REG-033).
    pub previous_keys: Vec<KeyHandover>,
    /// A title for people.
    pub title: Option<String>,
    /// A description for people.
    pub description: Option<String>,
}

/// Where a named file lands, decided by its extension (REG-003).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    /// `.html`, an Askama view under `templates/<namespace>-ui/<directory>/`.
    View,
    /// `.css`, under the same directory, served by the asset route.
    Stylesheet,
    /// `.js`, under the same directory, served by the asset route.
    Script,
    /// `.rs`, under `src/live/<namespace_module>/`.
    Rust,
}

impl FileKind {
    /// The kind a file name's extension gives it, or none for any other
    /// extension.
    pub fn of(file_name: &str) -> Option<Self> {
        let (_, extension) = file_name.rsplit_once('.')?;
        match extension {
            "html" => Some(FileKind::View),
            "css" => Some(FileKind::Stylesheet),
            "js" => Some(FileKind::Script),
            "rs" => Some(FileKind::Rust),
            _ => None,
        }
    }
}

/// A component's `manifest.json`: one object with exactly these keys, no
/// version and no crates (REG-002).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentManifest {
    /// `<namespace>.<directory>`.
    pub name: String,
    /// `<namespace>-ui/<directory>` when given.
    pub root: Option<String>,
    /// A title for people.
    pub title: Option<String>,
    /// A description for people.
    pub description: Option<String>,
    /// The files the component carries, by name (REG-003).
    pub files: Vec<String>,
    /// The custom element tags its script defines, each `<namespace>-`
    /// prefixed (REG-004).
    pub elements: Vec<String>,
    /// Each Live component its Rust defines, as `<module>::<Type>` (REG-005).
    pub register: Vec<String>,
    /// Components it needs first: a bare shipped name, `./<component>` of
    /// the same library, or a full address (REG-010).
    pub dependencies: Vec<String>,
}

/// Parses `library.json` (REG-025).
pub fn parse_library_json(bytes: &[u8]) -> Result<LibraryJson> {
    let _ = bytes;
    Err(RegistryError::NotBuilt("library.json parsing"))
}

/// Parses a component's `manifest.json` for the directory it sits in and
/// the library's namespace (REG-002, REG-003, REG-004).
pub fn parse_manifest(bytes: &[u8], directory: &str, namespace: &str) -> Result<ComponentManifest> {
    let _ = (bytes, directory, namespace);
    Err(RegistryError::NotBuilt("manifest parsing"))
}

/// Reads one JSON object strictly: at most [`MAX_JSON_BYTES`], exactly one
/// object, no duplicate key at any depth (REG-009).
pub fn strict_json_object(bytes: &[u8]) -> Result<serde_json::Map<String, serde_json::Value>> {
    let _ = bytes;
    Err(RegistryError::NotBuilt("strict JSON reading"))
}

/// Whether a namespace is one segment of lowercase letters, digits and
/// hyphens, starting with a letter, of at most 32 bytes, whose module form
/// is not a Rust keyword (REG-004).
pub fn valid_namespace(namespace: &str) -> bool {
    let module = namespace_module(namespace);
    !namespace.is_empty()
        && namespace.len() <= 32
        && namespace.as_bytes()[0].is_ascii_lowercase()
        && namespace
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !is_rust_keyword(&module)
}

/// The namespace with each hyphen written as an underscore: the module
/// under `src/live/` (REG-003).
pub fn namespace_module(namespace: &str) -> String {
    namespace.replace('-', "_")
}

/// Whether a component directory name is 1 to 64 bytes of lowercase
/// letters, digits and hyphens, neither starting nor ending with a hyphen
/// (REG-025).
pub fn valid_directory_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('-')
        && !name.ends_with('-')
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

/// Whether an identifier is a Rust keyword, `mod` or `lib` (REG-003).
pub fn is_rust_keyword(identifier: &str) -> bool {
    const KEYWORDS: [&str; 56] = [
        "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn",
        "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref",
        "return", "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe",
        "use", "where", "while", "async", "await", "dyn", "abstract", "become", "box", "do",
        "final", "macro", "override", "priv", "typeof", "unsized", "virtual", "yield", "try",
        "gen", "union", "lib", "main", "build",
    ];
    KEYWORDS.contains(&identifier)
}

#[cfg(test)]
mod tests {
    use super::{
        FileKind, is_rust_keyword, namespace_module, valid_directory_name, valid_namespace,
    };

    #[test]
    fn namespaces_and_directory_names_are_closed() {
        for ok in ["acme", "acme-ui", "a1"] {
            assert!(valid_namespace(ok), "{ok}");
            assert!(valid_directory_name(ok), "{ok}");
        }
        for hostile in ["", "1acme", "Acme", "self", "a_b", "a.b", &"a".repeat(33)] {
            assert!(!valid_namespace(hostile), "{hostile}");
        }
        for hostile in ["", "-a", "a-", "A", &"a".repeat(65)] {
            assert!(!valid_directory_name(hostile), "{hostile}");
        }
        assert_eq!(namespace_module("acme-ui"), "acme_ui");
        assert!(is_rust_keyword("mod") && is_rust_keyword("lib"));
    }

    #[test]
    fn file_kinds_follow_extensions() {
        assert_eq!(FileKind::of("widget.html"), Some(FileKind::View));
        assert_eq!(FileKind::of("widget.css"), Some(FileKind::Stylesheet));
        assert_eq!(FileKind::of("widget.js"), Some(FileKind::Script));
        assert_eq!(FileKind::of("widget.rs"), Some(FileKind::Rust));
        assert_eq!(FileKind::of("widget.wasm"), None);
        assert_eq!(FileKind::of("widget"), None);
    }
}
