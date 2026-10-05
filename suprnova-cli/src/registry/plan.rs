//! Resolving a source into a plan the developer confirms (REG-006, REG-007,
//! REG-010, REG-011, REG-012, REG-014): every component, its files and
//! their outcomes, every module and registration, every capability, and
//! the key to pin. Nothing is written until the plan is confirmed.

use std::collections::BTreeSet;
use std::path::PathBuf;

use super::address::{ComponentAddress, LibraryAddress, Source};
use super::fetch::{Commit, Fetcher};
use super::library::{ComponentManifest, FileKind, LibraryJson};
use super::project::ProjectFile;
use super::scan::ScanReport;
use super::signing::{Fingerprint, PublicKey};
use super::statement::Digest;
use super::{Capability, RegistryError, Result};

/// What installing a file would do (REG-012, REG-028).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileOutcome {
    /// The path does not exist and is written.
    New,
    /// The path holds the same bytes.
    Unchanged,
    /// The path holds unedited bytes and is replaced.
    Replaced,
    /// The path holds edited bytes and is kept; `--force` replaces it.
    Kept,
    /// The path holds bytes no record vouches for and is kept.
    ChangedSinceRecord,
}

/// One file of the plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedFile {
    /// The manifest name.
    pub name: String,
    /// Where it lands, from the project root.
    pub destination: PathBuf,
    /// Its kind.
    pub kind: FileKind,
    /// What happens to it.
    pub outcome: FileOutcome,
    /// The digest of the bytes that arrived.
    pub digest: Digest,
    /// The bytes that arrived.
    pub bytes: Vec<u8>,
}

/// One component of the plan, dependencies first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedComponent {
    /// The component's canonical address.
    pub address: ComponentAddress,
    /// The library as it arrived.
    pub library: LibraryJson,
    /// The library's version.
    pub version: semver::Version,
    /// The commit every file came from.
    pub commit: Commit,
    /// The manifest as it arrived.
    pub manifest: ComponentManifest,
    /// The bytes of `library.json` and `manifest.json` as they arrived.
    pub library_json_bytes: Vec<u8>,
    /// The manifest bytes as they arrived.
    pub manifest_bytes: Vec<u8>,
    /// The signature file as it arrived, absent for the shipped library.
    pub signature: Option<String>,
    /// The verification hash.
    pub hash: Digest,
    /// Every file.
    pub files: Vec<PlannedFile>,
    /// What the scan found.
    pub scan: ScanReport,
    /// Module declarations to add, as `(file, line)`.
    pub module_declarations: Vec<(PathBuf, String)>,
    /// Registrations to add, as full paths.
    pub registrations: Vec<String>,
    /// Registrations to remove, as full paths.
    pub unregistrations: Vec<String>,
}

/// A key the plan would pin or re-pin (REG-024, REG-033).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyPin {
    /// The library.
    pub library: LibraryAddress,
    /// The key to pin.
    pub key: PublicKey,
    /// The fingerprint of the key it replaces, when it does.
    pub replaces: Option<Fingerprint>,
}

/// What the developer asked for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Options {
    /// Replace edited files and accept downgrades and moved tags.
    pub force: bool,
    /// Report and write nothing.
    pub dry_run: bool,
    /// Confirm the plan without a terminal; never approves a capability or
    /// pins a key.
    pub yes: bool,
    /// Capabilities approved on the command line.
    pub allow: BTreeSet<Capability>,
}

/// The whole plan, dependencies first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// Every component, in install order.
    pub components: Vec<PlannedComponent>,
    /// The framework version the application depends on, as checked (REG-007).
    pub framework_version: semver::Version,
    /// Keys the plan pins.
    pub pins: Vec<KeyPin>,
    /// The router call to report when a namespace's first component installs
    /// (REG-017).
    pub router_calls: Vec<String>,
}

impl Plan {
    /// Every capability the plan needs approved, over all its components.
    pub fn capabilities(&self) -> BTreeSet<Capability> {
        self.components
            .iter()
            .flat_map(|component| component.scan.capabilities.iter().copied())
            .collect()
    }
}

/// Resolves a source into a plan: fetches, verifies and scans every
/// component, dependencies included, and decides each file's outcome.
pub fn resolve(
    source: &Source,
    options: &Options,
    fetcher: &dyn Fetcher,
    project: &ProjectFile,
) -> Result<Plan> {
    let _ = (source, options, fetcher, project);
    Err(RegistryError::NotBuilt("plan resolution"))
}

/// Renders the plan as `live:add` reports it before asking (REG-012).
pub fn render(plan: &Plan) -> String {
    let _ = plan;
    String::new()
}
