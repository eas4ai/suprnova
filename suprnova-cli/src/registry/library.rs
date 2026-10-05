//! The library tree, `library.json` and a component's `manifest.json`
//! (REG-002, REG-003, REG-004, REG-025, REG-033).
//!
//! Both documents are read with one strict reader: one JSON object, no
//! duplicate key at any depth, at most 1 MiB. A duplicate key is refused
//! rather than resolved because two readers that pick different copies
//! would see two different components under one signature. Every value
//! the statement (REG-023) or a destination path is built from is checked
//! against a closed character set.

use std::collections::BTreeSet;
use std::fmt;

use serde::Deserialize as _;
use serde::de::{self, MapAccess, SeqAccess, Visitor};

use super::address::{SHIPPED_LIBRARY, parse_dependency, parse_published_address};
use super::signing::{Fingerprint, KeyHandover, PublicKey, Signature};
use super::{RegistryError, Result};

/// The largest JSON document a library may carry (REG-002, REG-009).
pub const MAX_JSON_BYTES: usize = 1024 * 1024;

/// The largest file a manifest may name (REG-003).
pub const MAX_FILE_BYTES: usize = 1024 * 1024;

/// Namespaces only the shipped library may use (REG-004).
pub const RESERVED_NAMESPACES: [&str; 3] = ["suprnova", "sn", "live"];

/// The element tag prefix the shipped library keeps (UI-018, REG-004).
pub const SHIPPED_ELEMENT_PREFIX: &str = "sn-";

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

impl ComponentManifest {
    /// The modules its Rust files declare: each `.rs` file's stem.
    pub fn rust_modules(&self) -> Vec<String> {
        self.files
            .iter()
            .filter(|file| FileKind::of(file) == Some(FileKind::Rust))
            .filter_map(|file| file.strip_suffix(".rs"))
            .map(str::to_owned)
            .collect()
    }
}

const LIBRARY_KEYS: [&str; 8] = [
    "namespace",
    "source",
    "version",
    "framework",
    "publicKey",
    "previousKeys",
    "title",
    "description",
];

const HANDOVER_KEYS: [&str; 3] = ["publicKey", "next", "signature"];

const MANIFEST_KEYS: [&str; 8] = [
    "name",
    "files",
    "root",
    "title",
    "description",
    "elements",
    "register",
    "dependencies",
];

/// Parses `library.json` (REG-025, REG-033).
pub fn parse_library_json(bytes: &[u8]) -> Result<LibraryJson> {
    let object = strict_json_object(bytes)?;
    refuse_unknown_keys(&object, &LIBRARY_KEYS, "library.json")?;
    let namespace = required_string(&object, "namespace", "library.json")?;
    if !valid_namespace(&namespace) {
        return Err(RegistryError::Invalid(format!(
            "library.json namespace `{namespace}` must be one segment of lowercase letters, digits and hyphens, starting with a letter, of at most 32 bytes, whose module form is not a Rust keyword"
        )));
    }
    if RESERVED_NAMESPACES.contains(&namespace.as_str()) {
        return Err(RegistryError::Invalid(format!(
            "the namespace `{namespace}` is reserved for the shipped library"
        )));
    }
    let source = required_string(&object, "source", "library.json")?;
    parse_published_address(&source)?;
    let version_text = required_string(&object, "version", "library.json")?;
    let version = semver::Version::parse(&version_text).map_err(|error| {
        RegistryError::Invalid(format!(
            "library.json version `{version_text}` is not semver: {error}"
        ))
    })?;
    let framework_text = required_string(&object, "framework", "library.json")?;
    let framework = semver::VersionReq::parse(&framework_text).map_err(|error| {
        RegistryError::Invalid(format!(
            "library.json framework `{framework_text}` is not a semver requirement: {error}"
        ))
    })?;
    let public_key = PublicKey::parse(&required_string(&object, "publicKey", "library.json")?)?;
    let previous_keys = match object.get("previousKeys") {
        None => Vec::new(),
        Some(serde_json::Value::Array(entries)) => entries
            .iter()
            .map(parse_handover)
            .collect::<Result<Vec<_>>>()?,
        Some(_) => {
            return Err(RegistryError::Invalid(
                "library.json previousKeys is not a list".to_owned(),
            ));
        }
    };
    Ok(LibraryJson {
        namespace,
        source,
        version,
        framework,
        public_key,
        previous_keys,
        title: optional_text(&object, "title", "library.json")?,
        description: optional_text(&object, "description", "library.json")?,
    })
}

fn parse_handover(value: &serde_json::Value) -> Result<KeyHandover> {
    let object = value.as_object().ok_or_else(|| {
        RegistryError::Invalid("library.json previousKeys entries are objects".to_owned())
    })?;
    refuse_unknown_keys(object, &HANDOVER_KEYS, "a previousKeys entry")?;
    let from = PublicKey::parse(&required_string(
        object,
        "publicKey",
        "a previousKeys entry",
    )?)?;
    let next = required_string(object, "next", "a previousKeys entry")?;
    let to = Fingerprint::parse(&next).ok_or_else(|| {
        RegistryError::Invalid(format!(
            "previousKeys next `{next}` is not a key fingerprint (`sha256:` and 64 lowercase hex characters)"
        ))
    })?;
    let signature = Signature::parse_strict(&required_string(
        object,
        "signature",
        "a previousKeys entry",
    )?)?;
    Ok(KeyHandover {
        from,
        to,
        signature,
    })
}

/// Parses a component's `manifest.json` for the directory it sits in and
/// the library's namespace (REG-002, REG-003, REG-004). The namespace
/// `suprnova` is the shipped library's, whose element tags keep `sn-`.
pub fn parse_manifest(bytes: &[u8], directory: &str, namespace: &str) -> Result<ComponentManifest> {
    let object = strict_json_object(bytes)?;
    manifest_from_object(object, directory, namespace)
}

/// Parses a shipped component's manifest by the same rules as a
/// third-party one. The shipped manifests still carry the integer
/// `version` REG-002 removes; until the shipped tree drops it, that one
/// key is set aside here, and every other key is held to the third-party
/// format.
pub fn parse_shipped_manifest(bytes: &[u8], directory: &str) -> Result<ComponentManifest> {
    let mut object = strict_json_object(bytes)?;
    if object.get("version").is_some_and(serde_json::Value::is_u64) {
        object.remove("version");
    }
    manifest_from_object(object, directory, SHIPPED_LIBRARY)
}

fn manifest_from_object(
    object: serde_json::Map<String, serde_json::Value>,
    directory: &str,
    namespace: &str,
) -> Result<ComponentManifest> {
    let label = format!("components/{directory}/manifest.json");
    refuse_unknown_keys(&object, &MANIFEST_KEYS, &label)?;
    if !valid_directory_name(directory) {
        return Err(RegistryError::Invalid(format!(
            "component directory `{directory}` must be 1 to 64 lowercase letters, digits and hyphens, neither starting nor ending with a hyphen"
        )));
    }
    let name = required_string(&object, "name", &label)?;
    if name != format!("{namespace}.{directory}") {
        return Err(RegistryError::Invalid(format!(
            "{label} name `{name}` must be `{namespace}.{directory}`"
        )));
    }
    let root = optional_text(&object, "root", &label)?;
    if let Some(root) = &root
        && *root != format!("{namespace}-ui/{directory}")
    {
        return Err(RegistryError::Invalid(format!(
            "{label} root `{root}` must be `{namespace}-ui/{directory}`"
        )));
    }
    let files = string_list(&object, "files", &label)?;
    if files.is_empty() {
        return Err(RegistryError::Invalid(format!("{label} names no files")));
    }
    let mut seen = BTreeSet::new();
    for file in &files {
        validate_file_name(file)
            .map_err(|reason| RegistryError::Invalid(format!("{label} file `{file}`: {reason}")))?;
        if !seen.insert(file.as_str()) {
            return Err(RegistryError::Invalid(format!(
                "{label} names `{file}` twice"
            )));
        }
    }
    if !files
        .iter()
        .any(|file| FileKind::of(file) == Some(FileKind::View))
    {
        return Err(RegistryError::Invalid(format!(
            "{label} names no `.html` view"
        )));
    }
    let prefix = if namespace == SHIPPED_LIBRARY {
        SHIPPED_ELEMENT_PREFIX.to_owned()
    } else {
        format!("{namespace}-")
    };
    let elements = string_list(&object, "elements", &label)?;
    for element in &elements {
        let rest = element.strip_prefix(&prefix).unwrap_or_default();
        let valid = !rest.is_empty()
            && !element.ends_with('-')
            && element
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
        if !valid {
            return Err(RegistryError::Invalid(format!(
                "{label} element `{element}` must start with `{prefix}` and be lowercase letters, digits and hyphens"
            )));
        }
    }
    let modules: BTreeSet<String> = files
        .iter()
        .filter(|file| FileKind::of(file) == Some(FileKind::Rust))
        .filter_map(|file| file.strip_suffix(".rs"))
        .map(str::to_owned)
        .collect();
    let register = string_list(&object, "register", &label)?;
    let mut registered = BTreeSet::new();
    for entry in &register {
        let valid = entry.split_once("::").is_some_and(|(module, type_name)| {
            modules.contains(module) && valid_type_name(type_name)
        });
        if !valid {
            return Err(RegistryError::Invalid(format!(
                "{label} register `{entry}` must be `<module>::<Type>`, where `<module>` is one of its Rust files"
            )));
        }
        if !registered.insert(entry.as_str()) {
            return Err(RegistryError::Invalid(format!(
                "{label} registers `{entry}` twice"
            )));
        }
    }
    let dependencies = string_list(&object, "dependencies", &label)?;
    let mut depended = BTreeSet::new();
    for dependency in &dependencies {
        parse_dependency(dependency)?;
        if !depended.insert(dependency.as_str()) {
            return Err(RegistryError::Invalid(format!(
                "{label} names the dependency `{dependency}` twice"
            )));
        }
    }
    Ok(ComponentManifest {
        name,
        root,
        title: optional_text(&object, "title", &label)?,
        description: optional_text(&object, "description", &label)?,
        files,
        elements,
        register,
        dependencies,
    })
}

/// Checks one file name a manifest names (REG-003): a single name in the
/// component's directory whose extension gives its kind, from the closed
/// set its kind allows. Returns why it is refused.
pub fn validate_file_name(file: &str) -> std::result::Result<FileKind, String> {
    let kind = FileKind::of(file)
        .ok_or_else(|| "the extension must be .html, .css, .js or .rs".to_owned())?;
    if file.contains("..") || file.starts_with('.') || file.contains('/') || file.contains('\\') {
        return Err("a file name holds no `..`, leading dot or separator".to_owned());
    }
    let (stem, _) = file
        .rsplit_once('.')
        .ok_or_else(|| "a file name has a stem".to_owned())?;
    match kind {
        FileKind::View => {
            let valid = !stem.is_empty()
                && file.len() <= 128
                && file.bytes().all(|byte| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || matches!(byte, b'-' | b'.')
                });
            if !valid {
                return Err(
                    "a view name is lowercase letters, digits, hyphens and dots, at most 128 bytes"
                        .to_owned(),
                );
            }
        }
        FileKind::Stylesheet | FileKind::Script => {
            if !valid_directory_name(stem) {
                return Err("a stylesheet or script stem is 1 to 64 lowercase letters, digits and hyphens, neither starting nor ending with a hyphen, as the asset route serves".to_owned());
            }
        }
        FileKind::Rust => {
            let mut bytes = stem.bytes();
            let valid = bytes
                .next()
                .is_some_and(|first| first.is_ascii_lowercase() || first == b'_')
                && bytes
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
                && stem != "_"
                && !is_rust_keyword(stem);
            if !valid {
                return Err("a Rust file name is a lowercase Rust identifier that is not a keyword, `mod` or `lib`".to_owned());
            }
        }
    }
    Ok(kind)
}

fn valid_type_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        && name != "_"
        && !is_rust_keyword(name)
}

fn refuse_unknown_keys(
    object: &serde_json::Map<String, serde_json::Value>,
    allowed: &[&str],
    label: &str,
) -> Result<()> {
    for key in object.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(RegistryError::Invalid(format!(
                "{label} holds the key `{key}`, which is not one of {}",
                allowed.join(", ")
            )));
        }
    }
    Ok(())
}

fn required_string(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    label: &str,
) -> Result<String> {
    object
        .get(key)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| RegistryError::Invalid(format!("{label} has no `{key}` string")))
}

/// An optional string for people: no control characters, so printing it
/// cannot drive the terminal.
fn optional_text(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    label: &str,
) -> Result<Option<String>> {
    match object.get(key) {
        None => Ok(None),
        Some(serde_json::Value::String(text)) if !text.chars().any(char::is_control) => {
            Ok(Some(text.clone()))
        }
        Some(_) => Err(RegistryError::Invalid(format!(
            "{label} `{key}` must be a string with no control characters"
        ))),
    }
}

fn string_list(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    label: &str,
) -> Result<Vec<String>> {
    match object.get(key) {
        None => Ok(Vec::new()),
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_str().map(str::to_owned).ok_or_else(|| {
                    RegistryError::Invalid(format!("{label} `{key}` entries must be strings"))
                })
            })
            .collect(),
        Some(_) => Err(RegistryError::Invalid(format!(
            "{label} `{key}` is not a list"
        ))),
    }
}

/// Reads one JSON object strictly: at most [`MAX_JSON_BYTES`], exactly one
/// object, no duplicate key at any depth (REG-009).
pub fn strict_json_object(bytes: &[u8]) -> Result<serde_json::Map<String, serde_json::Value>> {
    if bytes.len() > MAX_JSON_BYTES {
        return Err(RegistryError::Invalid(format!(
            "a JSON document of {} bytes is over the {} byte limit",
            bytes.len(),
            MAX_JSON_BYTES
        )));
    }
    match strict_json(bytes)? {
        serde_json::Value::Object(object) => Ok(object),
        _ => Err(RegistryError::Invalid(
            "the JSON document is not one object".to_owned(),
        )),
    }
}

/// Reads one JSON value, refusing a duplicate key at any depth and any
/// bytes after the value. Callers cap the size first.
pub fn strict_json(bytes: &[u8]) -> Result<serde_json::Value> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value = StrictValue::deserialize(&mut deserializer)
        .map_err(|error| RegistryError::Invalid(format!("the JSON document is refused: {error}")))?
        .0;
    deserializer.end().map_err(|error| {
        RegistryError::Invalid(format!("the JSON document is not one value: {error}"))
    })?;
    Ok(value)
}

/// A JSON value read with every object's keys checked for duplicates.
struct StrictValue(serde_json::Value);

impl<'de> serde::Deserialize<'de> for StrictValue {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        deserializer.deserialize_any(StrictVisitor).map(StrictValue)
    }
}

struct StrictVisitor;

impl<'de> Visitor<'de> for StrictVisitor {
    type Value = serde_json::Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value")
    }

    fn visit_bool<E>(self, value: bool) -> std::result::Result<Self::Value, E> {
        Ok(serde_json::Value::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> std::result::Result<Self::Value, E> {
        Ok(serde_json::Value::from(value))
    }

    fn visit_u64<E>(self, value: u64) -> std::result::Result<Self::Value, E> {
        Ok(serde_json::Value::from(value))
    }

    fn visit_f64<E: de::Error>(self, value: f64) -> std::result::Result<Self::Value, E> {
        serde_json::Number::from_f64(value)
            .map(serde_json::Value::Number)
            .ok_or_else(|| E::custom("a number that is not finite"))
    }

    fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E> {
        Ok(serde_json::Value::String(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> std::result::Result<Self::Value, E> {
        Ok(serde_json::Value::String(value))
    }

    fn visit_unit<E>(self) -> std::result::Result<Self::Value, E> {
        Ok(serde_json::Value::Null)
    }

    fn visit_none<E>(self) -> std::result::Result<Self::Value, E> {
        Ok(serde_json::Value::Null)
    }

    fn visit_seq<A: SeqAccess<'de>>(
        self,
        mut seq: A,
    ) -> std::result::Result<Self::Value, A::Error> {
        let mut items = Vec::new();
        while let Some(StrictValue(item)) = seq.next_element()? {
            items.push(item);
        }
        Ok(serde_json::Value::Array(items))
    }

    fn visit_map<A: MapAccess<'de>>(
        self,
        mut map: A,
    ) -> std::result::Result<Self::Value, A::Error> {
        let mut object = serde_json::Map::new();
        while let Some(key) = map.next_key::<String>()? {
            if object.contains_key(&key) {
                return Err(de::Error::custom(format!("the key `{key}` appears twice")));
            }
            let StrictValue(value) = map.next_value()?;
            object.insert(key, value);
        }
        Ok(serde_json::Value::Object(object))
    }
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
        FileKind, is_rust_keyword, namespace_module, strict_json, strict_json_object,
        valid_directory_name, valid_namespace, validate_file_name,
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

    #[test]
    fn file_names_are_closed_by_kind() {
        for ok in [
            "widget.html",
            "widget.part.html",
            "widget.css",
            "a1-b.js",
            "date_picker.rs",
        ] {
            assert!(validate_file_name(ok).is_ok(), "{ok}");
        }
        for hostile in [
            "mod.rs",
            "lib.rs",
            "Widget.rs",
            "Widget.css",
            "-x.js",
            "x-.css",
            "../x.html",
            ".x.html",
            "a/b.html",
            "a..html",
            "x.wasm",
            "_.rs",
            "1x.rs",
        ] {
            assert!(validate_file_name(hostile).is_err(), "{hostile}");
        }
        let long_stem = format!("{}.css", "a".repeat(65));
        assert!(validate_file_name(&long_stem).is_err());
    }

    #[test]
    fn the_strict_reader_refuses_duplicates_at_any_depth_and_trailing_values() {
        assert!(strict_json_object(br#"{"a":1,"a":2}"#).is_err());
        assert!(strict_json_object(br#"{"a":{"b":1,"b":1}}"#).is_err());
        assert!(strict_json_object(br#"{"a":[{"b":1,"b":1}]}"#).is_err());
        assert!(strict_json_object(br#"{"a":1}{"a":1}"#).is_err());
        assert!(strict_json_object(br#"[1]"#).is_err());
        assert!(strict_json_object(br#"{"a":1} "#).is_ok());
        assert!(strict_json(br#"[{"a":1}]"#).is_ok());
        let big = format!("{{\"a\":\"{}\"}}", "x".repeat(super::MAX_JSON_BYTES));
        assert!(strict_json_object(big.as_bytes()).is_err());
    }
}
