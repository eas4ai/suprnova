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

/// Ordered Live v2 fixture files consumed by both Rust and TypeScript.
pub const FIXTURE_FILES_V2: &[&str] = &[
    "protocol-success.json",
    "protocol-failure.json",
    "compatibility.json",
];

/// Closed fixture versions shared by every conformance harness.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FixtureVersion {
    /// Stable protocol and snapshot corpus.
    V1,
    /// Lifecycle, child-delivery, URL, and rolling-version corpus.
    V2,
}

impl FixtureVersion {
    /// Returns the numeric fixture version.
    #[must_use]
    pub const fn get(self) -> u16 {
        match self {
            Self::V1 => 1,
            Self::V2 => 2,
        }
    }

    /// Returns the exact ordered files covered by this version's manifest.
    #[must_use]
    pub const fn files(self) -> &'static [&'static str] {
        match self {
            Self::V1 => FIXTURE_FILES_V1,
            Self::V2 => FIXTURE_FILES_V2,
        }
    }
}

/// Complete ordered fixture-version catalog.
pub const FIXTURE_VERSIONS: &[FixtureVersion] = &[FixtureVersion::V1, FixtureVersion::V2];

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
    fixture_directory(FixtureVersion::V1)
}

/// Returns the repository v2 fixture directory independent of process CWD.
#[must_use]
pub fn fixture_directory_v2() -> PathBuf {
    fixture_directory(FixtureVersion::V2)
}

/// Returns one versioned repository fixture directory independent of process CWD.
#[must_use]
pub fn fixture_directory(version: FixtureVersion) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("fixtures/v{}", version.get()))
}

/// Hashes ordered file names and exact bytes for Rust/TypeScript parity.
pub fn fixture_manifest_sha256() -> Result<String, ConformanceError> {
    fixture_manifest_sha256_for(&fixture_directory_v1(), FIXTURE_FILES_V1)
}

/// Hashes the ordered v2 file names and exact bytes.
pub fn fixture_manifest_sha256_v2() -> Result<String, ConformanceError> {
    fixture_manifest_sha256_for(&fixture_directory_v2(), FIXTURE_FILES_V2)
}

/// Hashes the exact reviewed file catalog for one fixture version.
pub fn fixture_manifest_sha256_version(
    version: FixtureVersion,
) -> Result<String, ConformanceError> {
    fixture_manifest_sha256_for(&fixture_directory(version), version.files())
}

fn fixture_manifest_sha256_for(
    directory: &std::path::Path,
    files: &[&str],
) -> Result<String, ConformanceError> {
    let mut hash = Sha256::new();
    for name in files {
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
    expected_fixture_manifest_sha256_for(&fixture_directory_v1())
}

/// Reads the reviewed v2 manifest digest stored beside the fixture corpus.
pub fn expected_fixture_manifest_sha256_v2() -> Result<String, ConformanceError> {
    expected_fixture_manifest_sha256_for(&fixture_directory_v2())
}

/// Reads the reviewed manifest digest for one fixture version.
pub fn expected_fixture_manifest_sha256_version(
    version: FixtureVersion,
) -> Result<String, ConformanceError> {
    expected_fixture_manifest_sha256_for(&fixture_directory(version))
}

fn expected_fixture_manifest_sha256_for(
    directory: &std::path::Path,
) -> Result<String, ConformanceError> {
    fs::read_to_string(directory.join("manifest.sha256"))
        .map(|value| value.trim().to_owned())
        .map_err(|_| ConformanceError {
            kind: ConformanceErrorKind::FixtureUnavailable,
        })
}
