//! Validation and exact serving of production browser artifacts.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Component, Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use tokio::fs;

const MANIFEST_FILE: &str = "suprnova-live.assets.json";
const EXPECTED_ROLES: [&str; 8] = [
    "core-classic",
    "core-esm",
    "stimulus-classic",
    "stimulus-esm",
    "uploads-classic",
    "uploads-esm",
    "async-classic",
    "async-esm",
];

#[derive(Debug, Deserialize, Serialize)]
struct Manifest {
    schema_version: u64,
    engine_version: String,
    runtime_contract_version: u64,
    protocol_versions: Vec<u64>,
    snapshot_versions: Vec<u64>,
    built_at: String,
    assets: Vec<ManifestAsset>,
    provenance: serde_json::Value,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct ManifestAsset {
    file: String,
    role: String,
    bytes: u64,
    sha256: String,
    sri: String,
    capability: String,
    capability_version: u64,
    compatible_core: String,
    content_type: String,
    script_kind: String,
    preload_rel: String,
    cache_control: String,
}

#[derive(Clone)]
pub(super) struct Artifact {
    pub(super) bytes: Vec<u8>,
    pub(super) content_type: String,
    pub(super) cache_control: String,
}

#[derive(Clone)]
pub(super) struct ValidatedArtifacts {
    manifest: Vec<u8>,
    assets: BTreeMap<String, Artifact>,
}

impl ValidatedArtifacts {
    pub(super) async fn load(root: &Path) -> Result<Self, String> {
        let root = root
            .canonicalize()
            .map_err(|error| format!("artifact root: {error}"))?;
        let manifest_path = root.join(MANIFEST_FILE);
        let manifest_bytes = fs::read(&manifest_path)
            .await
            .map_err(|error| format!("asset manifest: {error}"))?;
        let manifest: Manifest = serde_json::from_slice(&manifest_bytes)
            .map_err(|error| format!("asset manifest JSON: {error}"))?;
        if manifest.schema_version != 2 || manifest.assets.len() != EXPECTED_ROLES.len() {
            return Err("asset manifest has an unsupported schema or asset count".to_owned());
        }
        let roles = manifest
            .assets
            .iter()
            .map(|asset| asset.role.as_str())
            .collect::<BTreeSet<_>>();
        if roles != EXPECTED_ROLES.into_iter().collect() {
            return Err("asset manifest does not contain the exact production roles".to_owned());
        }

        let mut assets = BTreeMap::new();
        for selected in &manifest.assets {
            validate_file_name(&selected.file)?;
            if selected.content_type != "text/javascript; charset=utf-8"
                || selected.cache_control != "public, max-age=31536000, immutable"
                || !matches!(selected.script_kind.as_str(), "classic" | "module")
            {
                return Err(format!("invalid production metadata for {}", selected.file));
            }
            let path = root.join(&selected.file);
            let canonical = path
                .canonicalize()
                .map_err(|error| format!("artifact {}: {error}", selected.file))?;
            if canonical.parent() != Some(root.as_path()) {
                return Err("asset escaped the validated artifact root".to_owned());
            }
            let bytes = fs::read(&canonical)
                .await
                .map_err(|error| format!("artifact {}: {error}", selected.file))?;
            let digest = Sha256::digest(&bytes);
            let mut sha256 = String::with_capacity(64);
            for byte in digest {
                write!(&mut sha256, "{byte:02x}").expect("formatting into String cannot fail");
            }
            let sri = format!("sha256-{}", BASE64.encode(digest));
            if bytes.len() as u64 != selected.bytes
                || sha256 != selected.sha256
                || sri != selected.sri
            {
                return Err(format!("artifact integrity mismatch for {}", selected.file));
            }
            if assets
                .insert(
                    selected.file.clone(),
                    Artifact {
                        bytes,
                        content_type: selected.content_type.clone(),
                        cache_control: selected.cache_control.clone(),
                    },
                )
                .is_some()
            {
                return Err("duplicate manifest asset".to_owned());
            }
        }
        let manifest = serde_json::to_vec_pretty(&manifest)
            .map_err(|error| format!("asset manifest encoding: {error}"))?;
        Ok(Self { manifest, assets })
    }

    pub(super) fn manifest(&self) -> &[u8] {
        &self.manifest
    }

    pub(super) fn asset(&self, path: &str) -> Option<&Artifact> {
        path.strip_prefix('/')
            .and_then(|file| self.assets.get(file))
    }
}

fn validate_file_name(file: &str) -> Result<(), String> {
    let path = PathBuf::from(file);
    let mut components = path.components();
    if file.is_empty()
        || file.len() > 255
        || !matches!(components.next(), Some(Component::Normal(_)))
        || components.next().is_some()
        || !file.ends_with(".js")
    {
        return Err("manifest asset file is not a single JavaScript path segment".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_file_name;

    #[test]
    fn manifest_file_names_are_single_closed_segments() {
        assert!(validate_file_name("suprnova-live.esm.js").is_ok());
        for hostile in [
            "",
            "../runtime.js",
            "/runtime.js",
            "src/runtime.js",
            "runtime.ts",
        ] {
            assert!(validate_file_name(hostile).is_err(), "{hostile}");
        }
    }
}
