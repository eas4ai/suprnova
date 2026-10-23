//! A component's verification statement and hash (REG-023).
//!
//! The statement is one JSON object with no whitespace and its keys in a
//! fixed order, so it has one spelling; the hash is the sha256 of its UTF-8
//! bytes. It covers the component exactly as it arrived, never the files on
//! disk.

use std::collections::BTreeMap;
use std::fmt;

use sha2::{Digest as _, Sha256};

/// `sha256:` and the lowercase hex digest of some bytes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Digest(String);

impl Digest {
    /// The digest of `bytes`.
    pub fn of(bytes: &[u8]) -> Self {
        let hex: String = Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Digest(format!("sha256:{hex}"))
    }

    /// Reads a digest written as `sha256:` and 64 lowercase hex characters.
    pub fn parse(text: &str) -> Option<Self> {
        let hex = text.strip_prefix("sha256:")?;
        (hex.len() == 64
            && hex
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f')))
        .then(|| Digest(text.to_owned()))
    }

    /// The digest as written.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The format the statement names.
pub const FORMAT: &str = "suprnova-component/1";

/// What a signature covers: the library, the version, the component
/// directory, and the digest of every byte that arrived.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Statement {
    /// The library's `source` (REG-025).
    pub library: String,
    /// The library's version.
    pub version: semver::Version,
    /// The component directory name.
    pub component: String,
    /// The digest of `library.json`.
    pub library_json: Digest,
    /// The digest of `manifest.json`.
    pub manifest: Digest,
    /// The digest of each named file, by file name.
    pub files: BTreeMap<String, Digest>,
}

impl Statement {
    /// The statement's one spelling: `{"format":...,"library":...,
    /// "version":...,"component":...,"libraryJson":...,"manifest":...,
    /// "files":{...}}`, file names in byte order, no whitespace.
    pub fn canonical_json(&self) -> String {
        fn quoted(value: &str) -> String {
            serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_owned())
        }
        let mut out = String::new();
        out.push_str("{\"format\":");
        out.push_str(&quoted(FORMAT));
        out.push_str(",\"library\":");
        out.push_str(&quoted(&self.library));
        out.push_str(",\"version\":");
        out.push_str(&quoted(&self.version.to_string()));
        out.push_str(",\"component\":");
        out.push_str(&quoted(&self.component));
        out.push_str(",\"libraryJson\":");
        out.push_str(&quoted(self.library_json.as_str()));
        out.push_str(",\"manifest\":");
        out.push_str(&quoted(self.manifest.as_str()));
        out.push_str(",\"files\":{");
        for (index, (name, digest)) in self.files.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            out.push_str(&quoted(name));
            out.push(':');
            out.push_str(&quoted(digest.as_str()));
        }
        out.push_str("}}");
        out
    }

    /// The verification hash: the digest of the canonical statement.
    pub fn verification_hash(&self) -> Digest {
        Digest::of(self.canonical_json().as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{Digest, Statement};

    fn statement() -> Statement {
        Statement {
            library: "github.com/acme/acme-ui".to_owned(),
            version: semver::Version::new(1, 2, 0),
            component: "date-picker".to_owned(),
            library_json: Digest::of(b"library"),
            manifest: Digest::of(b"manifest"),
            files: BTreeMap::from([
                ("date-picker.html".to_owned(), Digest::of(b"view")),
                ("date-picker.css".to_owned(), Digest::of(b"css")),
            ]),
        }
    }

    #[test]
    fn the_statement_has_one_spelling_with_keys_in_order_and_files_by_byte_order() {
        let json = statement().canonical_json();
        assert!(json.starts_with("{\"format\":\"suprnova-component/1\",\"library\":\"github.com/acme/acme-ui\",\"version\":\"1.2.0\",\"component\":\"date-picker\",\"libraryJson\":\"sha256:"));
        let files_at = json.find("\"files\":{").expect("files");
        let css_at = json.find("\"date-picker.css\"").expect("css");
        let html_at = json.find("\"date-picker.html\"").expect("html");
        assert!(files_at < css_at && css_at < html_at, "{json}");
        assert!(!json.contains(' ') && !json.contains('\n'));
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("the statement is JSON");
        assert_eq!(
            parsed["files"].as_object().map(|files| files.len()),
            Some(2)
        );
    }

    #[test]
    fn the_hash_is_a_sha256_digest_that_changes_with_one_byte() {
        let a = statement();
        let hash = a.verification_hash();
        assert_eq!(Digest::parse(hash.as_str()), Some(hash.clone()));
        let mut b = a.clone();
        b.files
            .insert("date-picker.css".to_owned(), Digest::of(b"css2"));
        assert_ne!(b.verification_hash(), hash);
        assert_eq!(a.verification_hash(), hash);
    }

    #[test]
    fn a_digest_is_sha256_and_lowercase_hex() {
        let digest = Digest::of(b"");
        assert_eq!(
            digest.as_str(),
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert!(Digest::parse("sha256:E3B0").is_none());
        assert!(Digest::parse("md5:00").is_none());
    }
}
