//! Cache manifests pin artifact content before a model loader can consume it.

use crate::FrameworkError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{Read, Write},
    path::{Component, Path},
    time::{SystemTime, UNIX_EPOCH},
};

/// Each artifact is verified independently so an error names the changed file.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManifestFile {
    /// SHA256 detects changes even when the file's length is unchanged.
    pub sha256: String,
    /// Sizes make download and disk requirements inspectable without loading tensors.
    pub size: u64,
}
/// A persisted manifest binds a resolved revision to the files inference actually uses.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelManifest {
    /// The original repository or local model identity remains readable to operators.
    pub repository: String,
    /// Remote models use the hub's commit SHA; local models use their artifact digest.
    pub revision: String,
    /// UTC Unix seconds record when the artifact set was first pinned.
    pub recorded_at: u64,
    /// Paths are relative to this model's cache directory, never external destinations.
    pub files: BTreeMap<String, ManifestFile>,
    /// Some exports put special-token metadata in tokenizer_config rather than a separate map.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub generated_files: BTreeMap<String, String>,
}
impl ModelManifest {
    /// Record only a complete file set and atomically publish it after hashing succeeds.
    pub fn record(
        root: &Path,
        repository: &str,
        revision: &str,
        names: &[String],
    ) -> Result<Self, FrameworkError> {
        Self::record_with_generated(root, repository, revision, names, BTreeMap::new())
    }
    pub(super) fn record_with_generated(
        root: &Path,
        repository: &str,
        revision: &str,
        names: &[String],
        generated_files: BTreeMap<String, String>,
    ) -> Result<Self, FrameworkError> {
        let mut files = BTreeMap::new();
        for name in names {
            files.insert(name.clone(), file_digest(&safe_path(root, name)?)?);
        }
        if files.is_empty() {
            return Err(FrameworkError::param("manifest.json requires model files"));
        }
        let revision = if revision == "local" {
            format!(
                "local:{}",
                hex(&Sha256::digest(serde_json::to_vec(&files).map_err(
                    |e| FrameworkError::internal(format!("manifest.json: {e}"))
                )?))
            )
        } else {
            revision.into()
        };
        let manifest = Self {
            repository: repository.into(),
            revision,
            recorded_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|e| FrameworkError::internal(format!("manifest.json time: {e}")))?
                .as_secs(),
            files,
            generated_files,
        };
        let bytes = serde_json::to_vec_pretty(&manifest)
            .map_err(|e| FrameworkError::internal(format!("manifest.json: {e}")))?;
        let mut temporary =
            tempfile::NamedTempFile::new_in(root).map_err(|e| file_error(root, e))?;
        temporary
            .write_all(&bytes)
            .map_err(|e| file_error(root, e))?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|e| file_error(root, e))?;
        temporary
            .persist(root.join("manifest.json"))
            .map_err(|e| file_error(root, e))?;
        Ok(manifest)
    }
    /// Cached models are usable offline, but every file must still match its pinned content.
    pub fn read(root: &Path) -> Result<Self, FrameworkError> {
        let path = root.join("manifest.json");
        let bytes = fs::read(&path).map_err(|e| file_error(&path, e))?;
        serde_json::from_slice(&bytes).map_err(|e| file_error(&path, e))
    }
    /// Refuse corrupt artifacts before loading and return the digest of the persisted manifest.
    pub fn verify(&self, root: &Path) -> Result<String, FrameworkError> {
        if self.files.is_empty() || self.repository.is_empty() || self.revision.is_empty() {
            return Err(FrameworkError::param(
                "manifest.json incomplete model provenance",
            ));
        }
        for name in self.generated_files.keys() {
            if !self.files.contains_key(name) {
                return Err(FrameworkError::param(format!(
                    "model generated file {name} is missing its sha256 entry in manifest.json"
                )));
            }
        }
        for (name, expected) in &self.files {
            let path = safe_path(root, name)?;
            let actual = file_digest(&path)?;
            if actual != *expected {
                return Err(FrameworkError::param(format!(
                    "model file {} sha256 or size differs from manifest.json",
                    path.display()
                )));
            }
        }
        Ok(file_digest(&root.join("manifest.json"))?.sha256)
    }
}
pub(super) fn safe_path(root: &Path, name: &str) -> Result<std::path::PathBuf, FrameworkError> {
    if name.is_empty()
        || Path::new(name)
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(FrameworkError::param(format!(
            "model file {name} must be a relative artifact path"
        )));
    }
    Ok(root.join(name))
}
pub(super) fn file_error(path: &Path, error: impl std::fmt::Display) -> FrameworkError {
    FrameworkError::internal(format!("model file {}: {error}", path.display()))
}
fn file_digest(path: &Path) -> Result<ManifestFile, FrameworkError> {
    let mut file = File::open(path).map_err(|e| file_error(path, e))?;
    let mut hash = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|e| file_error(path, e))?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
        size += read as u64;
    }
    Ok(ManifestFile {
        sha256: hex(&hash.finalize()),
        size,
    })
}
pub(super) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
