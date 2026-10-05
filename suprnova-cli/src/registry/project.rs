//! The project's side of an install: `suprnova.toml` as the provenance
//! record and the key pins (REG-001, REG-013, REG-024), the per-directory
//! install record that detects edits (REG-028), and the lock and journal
//! that make an install atomic (REG-029).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::address::{ComponentAddress, LibraryAddress};
use super::signing::{PublicKey, Signature};
use super::statement::Digest;
use super::{Capability, RegistryError, Result};

/// The project file, all lowercase (REG-001).
pub const PROJECT_FILE: &str = "suprnova.toml";

/// The name the CLI used to read and no longer does.
pub const LEGACY_PROJECT_FILE: &str = "Suprnova.toml";

/// The install record beside a component's files (REG-028).
pub const INSTALL_RECORD: &str = ".suprnova-installed.json";

/// Refuses a project that holds `Suprnova.toml` and no `suprnova.toml`, by
/// its exact directory entries, naming the rename (REG-001).
pub fn refuse_legacy_project_file(root: &Path) -> Result<()> {
    let entries = std::fs::read_dir(root)
        .map_err(|error| RegistryError::Io(format!("cannot read {}: {error}", root.display())))?;
    let mut legacy = false;
    let mut current = false;
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name == LEGACY_PROJECT_FILE {
            legacy = true;
        } else if name == PROJECT_FILE {
            current = true;
        }
    }
    if legacy && !current {
        return Err(RegistryError::Invalid(format!(
            "the project file is `{PROJECT_FILE}`, all lowercase; rename `{LEGACY_PROJECT_FILE}` to `{PROJECT_FILE}`"
        )));
    }
    Ok(())
}

/// One `[live.components."<address>"]` table (REG-013): exactly what
/// arrived.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentRecord {
    /// The library's `source`.
    pub source: String,
    /// The library version installed.
    pub version: semver::Version,
    /// The commit the tag resolved to.
    pub commit: String,
    /// The digest of `library.json` as it arrived.
    pub library_json: Digest,
    /// The digest of `manifest.json` as it arrived.
    pub manifest: Digest,
    /// The digest of each named file as it arrived, by file name.
    pub files: BTreeMap<String, Digest>,
    /// The verification hash (REG-023).
    pub hash: Digest,
    /// The signature (REG-024).
    pub signature: Signature,
    /// Each dependency with the version it resolved to (REG-010).
    pub dependencies: BTreeMap<ComponentAddress, semver::Version>,
    /// Each registered type's full path, as the registration line wrote it
    /// (REG-005).
    pub registered: Vec<String>,
    /// Each capability the scan found, with its approval (REG-006).
    pub capabilities: BTreeMap<Capability, Approval>,
    /// Files the install kept as edited (REG-028).
    pub kept: Vec<String>,
}

/// How a capability came to be approved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Approval {
    /// The developer confirmed it on a terminal.
    Terminal,
    /// `--allow <capability>` named it.
    Flag,
}

/// The project's `suprnova.toml`, read whole and edited in place with
/// every byte it does not own kept (REG-001).
#[derive(Debug)]
pub struct ProjectFile {
    root: PathBuf,
}

impl ProjectFile {
    /// Loads the project file, or an empty one when none exists; refuses a
    /// project that holds only the legacy name.
    pub fn load(root: &Path) -> Result<Self> {
        refuse_legacy_project_file(root)?;
        Err(RegistryError::NotBuilt("the project file"))
    }

    /// The project root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Every installed component's record.
    pub fn components(&self) -> Result<BTreeMap<ComponentAddress, ComponentRecord>> {
        Err(RegistryError::NotBuilt("the project file"))
    }

    /// The key pinned for a library (REG-024), if any.
    pub fn pinned_key(&self, library: &LibraryAddress) -> Result<Option<PublicKey>> {
        let _ = library;
        Err(RegistryError::NotBuilt("the project file"))
    }

    /// The library an installed namespace belongs to (REG-011), if any.
    pub fn namespace_owner(&self, namespace: &str) -> Result<Option<LibraryAddress>> {
        let _ = namespace;
        Err(RegistryError::NotBuilt("the project file"))
    }

    /// Writes a component's record and, when given, a key pin, keeping
    /// every other byte of the file (REG-001, REG-013, REG-024).
    pub fn record(
        &mut self,
        address: &ComponentAddress,
        record: &ComponentRecord,
        pin: Option<(&LibraryAddress, &PublicKey)>,
    ) -> Result<()> {
        let _ = (address, record, pin);
        Err(RegistryError::NotBuilt("the project file"))
    }
}

/// The per-directory record of the upstream bytes last written at each
/// path, keyed by the path from the project root (REG-028).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InstallRecord {
    /// Each installed path's digest.
    pub digests: BTreeMap<PathBuf, Digest>,
}

impl InstallRecord {
    /// Reads a component directory's record, reading today's bare-name form
    /// as files of that directory (REG-028).
    pub fn load(project_root: &Path, component_dir: &Path) -> Result<Self> {
        let _ = (project_root, component_dir);
        Err(RegistryError::NotBuilt("the install record"))
    }

    /// Writes the record in the new form.
    pub fn save(&self, project_root: &Path, component_dir: &Path) -> Result<()> {
        let _ = (project_root, component_dir);
        Err(RegistryError::NotBuilt("the install record"))
    }
}

/// The exclusive project lock `live:add` holds from before it reads the
/// records until it finishes (REG-029); `serve` waits on it.
#[derive(Debug)]
pub struct ProjectLock {
    path: PathBuf,
}

impl ProjectLock {
    /// Takes the lock, refusing when another `live:add` holds it.
    pub fn acquire(project_root: &Path) -> Result<Self> {
        let _ = project_root;
        Err(RegistryError::NotBuilt("the project lock"))
    }

    /// Whether another process holds the lock.
    pub fn is_held(project_root: &Path) -> bool {
        let _ = project_root;
        false
    }

    /// The lock file's path.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// The journal written before the first write: every path the install will
/// change or create, with the prior bytes of each it changes (REG-029).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Journal {
    /// Paths that will be created.
    pub created: Vec<PathBuf>,
    /// Paths that will change, with their prior bytes.
    pub changed: BTreeMap<PathBuf, Vec<u8>>,
}

impl Journal {
    /// Writes the journal into the project.
    pub fn write(&self, project_root: &Path) -> Result<()> {
        let _ = project_root;
        Err(RegistryError::NotBuilt("the journal"))
    }

    /// Restores every file the journal names and removes the journal;
    /// `live:add` and `serve` call this when they find one with no lock held.
    pub fn restore(project_root: &Path) -> Result<bool> {
        let _ = project_root;
        Err(RegistryError::NotBuilt("the journal"))
    }
}

#[cfg(test)]
mod tests {
    use super::refuse_legacy_project_file;

    #[test]
    fn a_project_holding_only_the_legacy_name_is_refused_naming_the_rename() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("Suprnova.toml"), "[serve]\n").expect("write");
        let error = refuse_legacy_project_file(dir.path()).expect_err("refused");
        assert!(
            error
                .to_string()
                .contains("rename `Suprnova.toml` to `suprnova.toml`"),
            "{error}"
        );
        std::fs::write(dir.path().join("suprnova.toml"), "").expect("write");
        refuse_legacy_project_file(dir.path()).expect("both present is allowed");
    }
}
