//! Shared cross-language fixture catalog and deterministic manifest hashing.

use std::error::Error;
use std::fmt;
use std::fs;
use std::path::PathBuf;

use sha2::{Digest as _, Sha256};

/// Ordered Live v1 fixture files consumed by both Rust and TypeScript.
pub const FIXTURE_FILES_V1: &[&str] = &[
    "canonical-success.json",
    "canonical-failure.json",
    "snapshot-success.json",
    "snapshot-failure.json",
    "protocol-success.json",
    "protocol-failure.json",
    "response-ordering.json",
    "compatibility.json",
];

/// Closed fixture-catalog failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConformanceErrorKind {
    /// A reviewed fixture file could not be read.
    FixtureUnavailable,
}

/// Redacted conformance-catalog error.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct ConformanceError {
    kind: ConformanceErrorKind,
}

impl ConformanceError {
    /// Returns the closed failure reason.
    #[must_use]
    pub const fn kind(self) -> ConformanceErrorKind {
        self.kind
    }
}

impl fmt::Display for ConformanceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("fixture_unavailable")
    }
}

impl fmt::Debug for ConformanceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl Error for ConformanceError {}

/// Returns the repository fixture directory independent of process CWD.
#[must_use]
pub fn fixture_directory_v1() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/v1")
}

/// Hashes ordered file names and exact bytes for Rust/TypeScript parity.
pub fn fixture_manifest_sha256() -> Result<String, ConformanceError> {
    let directory = fixture_directory_v1();
    let mut hash = Sha256::new();
    for name in FIXTURE_FILES_V1 {
        let bytes = fs::read(directory.join(name)).map_err(|_| ConformanceError {
            kind: ConformanceErrorKind::FixtureUnavailable,
        })?;
        hash.update(name.as_bytes());
        hash.update([0]);
        hash.update(bytes);
        hash.update([0]);
    }
    Ok(hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

/// Reads the reviewed manifest digest stored beside the fixture corpus.
pub fn expected_fixture_manifest_sha256() -> Result<String, ConformanceError> {
    fs::read_to_string(fixture_directory_v1().join("manifest.sha256"))
        .map(|value| value.trim().to_owned())
        .map_err(|_| ConformanceError {
            kind: ConformanceErrorKind::FixtureUnavailable,
        })
}
