//! Reviewed production browser artifacts embedded with their manifest.
//!
//! The browser package's deterministic build output is tracked under
//! `browser/dist/` and embedded here so a host can serve the exact reviewed
//! bytes without a JavaScript build step. The manifest is validated against
//! the embedded bytes on first use: any drift in digest, length, file name,
//! role, capability, or version fails closed before a single byte is served.

use std::fmt;
use std::path::{Component, Path};
use std::sync::OnceLock;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::Deserialize;
use sha2::{Digest as _, Sha256};

use crate::{SUPPORTED_PROTOCOL_VERSIONS, SUPPORTED_SNAPSHOT_VERSIONS};

/// Version line of the browser runtime, carried by the manifest and by the
/// runtime's public `version` export. It is independent of the crate version.
pub const BROWSER_RUNTIME_VERSION: &str = "0.1.0";

/// Runtime configuration contract understood by the embedded browser runtime.
pub const RUNTIME_CONTRACT_VERSION: u16 = 1;

/// Manifest schema version this engine understands.
pub const MANIFEST_SCHEMA_VERSION: u16 = 2;

/// Fixed timestamp the reproducible build records.
pub const REPRODUCIBLE_BUILD_TIMESTAMP: &str = "1970-01-01T00:00:00.000Z";

/// Exact media type of every JavaScript artifact.
pub const ARTIFACT_CONTENT_TYPE: &str = "text/javascript; charset=utf-8";

/// Cache policy the manifest records for immutable, identity-addressed artifacts.
pub const ARTIFACT_CACHE_CONTROL: &str = "public, max-age=31536000, immutable";

/// File name of the asset manifest.
pub const MANIFEST_FILE: &str = "suprnova-live.assets.json";

const COMPATIBLE_CORE: &str = ">=0.1.0 <0.2.0";
const CAPABILITY_VERSION: u16 = 1;
const MAX_MANIFEST_BYTES: usize = 64 * 1024;
const MAX_FILE_NAME_BYTES: usize = 255;
const IDENTITY_DIGEST_CHARS: usize = 16;

const MANIFEST_BYTES: &[u8] = include_bytes!("../browser/dist/suprnova-live.assets.json");

const EMBEDDED_FILES: &[(&str, &[u8])] = &[
    (
        "suprnova-live.classic.js",
        include_bytes!("../browser/dist/suprnova-live.classic.js"),
    ),
    (
        "suprnova-live.esm.js",
        include_bytes!("../browser/dist/suprnova-live.esm.js"),
    ),
    (
        "suprnova-live.stimulus.classic.js",
        include_bytes!("../browser/dist/suprnova-live.stimulus.classic.js"),
    ),
    (
        "suprnova-live.stimulus.esm.js",
        include_bytes!("../browser/dist/suprnova-live.stimulus.esm.js"),
    ),
    (
        "suprnova-live.uploads.classic.js",
        include_bytes!("../browser/dist/suprnova-live.uploads.classic.js"),
    ),
    (
        "suprnova-live.uploads.esm.js",
        include_bytes!("../browser/dist/suprnova-live.uploads.esm.js"),
    ),
    (
        "suprnova-live.async.classic.js",
        include_bytes!("../browser/dist/suprnova-live.async.classic.js"),
    ),
    (
        "suprnova-live.async.esm.js",
        include_bytes!("../browser/dist/suprnova-live.async.esm.js"),
    ),
];

/// Closed set of production artifact roles.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord)]
pub enum ArtifactRole {
    /// Universal core runtime as an ES module.
    CoreEsm,
    /// Universal core runtime as a classic script.
    CoreClassic,
    /// Optional Stimulus bridge as an ES module.
    StimulusEsm,
    /// Optional Stimulus bridge as a classic script.
    StimulusClassic,
    /// Optional upload feature as an ES module.
    UploadsEsm,
    /// Optional upload feature as a classic script.
    UploadsClassic,
    /// Optional asynchronous-update feature as an ES module.
    AsyncEsm,
    /// Optional asynchronous-update feature as a classic script.
    AsyncClassic,
}

impl ArtifactRole {
    /// Every role, in the order the manifest is validated and served.
    pub const ALL: [Self; 8] = [
        Self::CoreClassic,
        Self::CoreEsm,
        Self::StimulusClassic,
        Self::StimulusEsm,
        Self::UploadsClassic,
        Self::UploadsEsm,
        Self::AsyncClassic,
        Self::AsyncEsm,
    ];

    /// Returns the manifest role name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CoreEsm => "core-esm",
            Self::CoreClassic => "core-classic",
            Self::StimulusEsm => "stimulus-esm",
            Self::StimulusClassic => "stimulus-classic",
            Self::UploadsEsm => "uploads-esm",
            Self::UploadsClassic => "uploads-classic",
            Self::AsyncEsm => "async-esm",
            Self::AsyncClassic => "async-classic",
        }
    }

    /// Parses an exact manifest role name.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|role| role.as_str() == value)
    }

    /// Returns the single-segment artifact file name.
    #[must_use]
    pub const fn file(self) -> &'static str {
        match self {
            Self::CoreEsm => "suprnova-live.esm.js",
            Self::CoreClassic => "suprnova-live.classic.js",
            Self::StimulusEsm => "suprnova-live.stimulus.esm.js",
            Self::StimulusClassic => "suprnova-live.stimulus.classic.js",
            Self::UploadsEsm => "suprnova-live.uploads.esm.js",
            Self::UploadsClassic => "suprnova-live.uploads.classic.js",
            Self::AsyncEsm => "suprnova-live.async.esm.js",
            Self::AsyncClassic => "suprnova-live.async.classic.js",
        }
    }

    /// Returns the versioned capability the artifact provides.
    #[must_use]
    pub const fn capability(self) -> &'static str {
        match self {
            Self::CoreEsm | Self::CoreClassic => "core@1",
            Self::StimulusEsm | Self::StimulusClassic => "stimulus@1",
            Self::UploadsEsm | Self::UploadsClassic => "uploads@1",
            Self::AsyncEsm | Self::AsyncClassic => "async@1",
        }
    }

    /// Returns how a browser must execute the artifact.
    #[must_use]
    pub const fn script_kind(self) -> ScriptKind {
        match self {
            Self::CoreEsm | Self::StimulusEsm | Self::UploadsEsm | Self::AsyncEsm => {
                ScriptKind::Module
            }
            Self::CoreClassic
            | Self::StimulusClassic
            | Self::UploadsClassic
            | Self::AsyncClassic => ScriptKind::Classic,
        }
    }

    /// Returns the preload relationship recorded for the artifact.
    #[must_use]
    pub const fn preload_relation(self) -> PreloadRelation {
        match self.script_kind() {
            ScriptKind::Module => PreloadRelation::ModulePreload,
            ScriptKind::Classic => PreloadRelation::Preload,
        }
    }

    /// Returns whether the role is the universal core rather than an optional feature.
    #[must_use]
    pub const fn is_core(self) -> bool {
        matches!(self, Self::CoreEsm | Self::CoreClassic)
    }

    /// Returns the role's position in [`Self::ALL`], which is also its artifact index.
    const fn index(self) -> usize {
        match self {
            Self::CoreClassic => 0,
            Self::CoreEsm => 1,
            Self::StimulusClassic => 2,
            Self::StimulusEsm => 3,
            Self::UploadsClassic => 4,
            Self::UploadsEsm => 5,
            Self::AsyncClassic => 6,
            Self::AsyncEsm => 7,
        }
    }
}

/// How a browser executes an artifact.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ScriptKind {
    /// `<script type="module">`.
    Module,
    /// Classic `<script>`.
    Classic,
}

impl ScriptKind {
    /// Returns the manifest `script_kind` value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Module => "module",
            Self::Classic => "classic",
        }
    }
}

/// Preload relationship a document uses to fetch an artifact early.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PreloadRelation {
    /// `<link rel="modulepreload">`.
    ModulePreload,
    /// `<link rel="preload" as="script">`.
    Preload,
}

impl PreloadRelation {
    /// Returns the manifest `preload_rel` value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ModulePreload => "modulepreload",
            Self::Preload => "preload",
        }
    }
}

/// Closed reasons the embedded artifacts cannot be served.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArtifactErrorKind {
    /// The manifest is not the exact schema, version set, or role set this engine expects.
    ManifestInvalid,
    /// A recorded role has no embedded bytes.
    RoleMissing,
    /// A recorded file name, capability, kind, or policy disagrees with the role contract.
    MetadataMismatch,
    /// A recorded length, digest, or integrity value disagrees with the embedded bytes.
    IntegrityMismatch,
}

/// Failure to validate the embedded artifacts against their manifest.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArtifactError {
    kind: ArtifactErrorKind,
}

impl ArtifactError {
    const fn new(kind: ArtifactErrorKind) -> Self {
        Self { kind }
    }

    /// Returns the closed failure class.
    #[must_use]
    pub const fn kind(self) -> ArtifactErrorKind {
        self.kind
    }
}

impl fmt::Display for ArtifactError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            ArtifactErrorKind::ManifestInvalid => "browser_artifact_manifest_invalid",
            ArtifactErrorKind::RoleMissing => "browser_artifact_role_missing",
            ArtifactErrorKind::MetadataMismatch => "browser_artifact_metadata_mismatch",
            ArtifactErrorKind::IntegrityMismatch => "browser_artifact_integrity_mismatch",
        })
    }
}

impl std::error::Error for ArtifactError {}

/// One validated production artifact: exact bytes plus the manifest facts about them.
#[derive(Clone, Debug)]
pub struct RuntimeArtifact {
    role: ArtifactRole,
    bytes: &'static [u8],
    sha256_hex: String,
    sri: String,
}

impl RuntimeArtifact {
    /// Returns the artifact role.
    #[must_use]
    pub const fn role(&self) -> ArtifactRole {
        self.role
    }

    /// Returns the single-segment file name.
    #[must_use]
    pub const fn file(&self) -> &'static str {
        self.role.file()
    }

    /// Returns the exact reviewed bytes.
    #[must_use]
    pub const fn bytes(&self) -> &'static [u8] {
        self.bytes
    }

    /// Returns the lower-case hexadecimal SHA-256 digest of the bytes.
    #[must_use]
    pub fn sha256_hex(&self) -> &str {
        &self.sha256_hex
    }

    /// Returns the Subresource Integrity value of the bytes.
    #[must_use]
    pub fn sri(&self) -> &str {
        &self.sri
    }

    /// Returns the exact media type to serve.
    #[must_use]
    pub const fn content_type(&self) -> &'static str {
        ARTIFACT_CONTENT_TYPE
    }

    /// Returns the immutable cache policy to serve.
    #[must_use]
    pub const fn cache_control(&self) -> &'static str {
        ARTIFACT_CACHE_CONTROL
    }

    /// Returns the versioned capability the artifact provides.
    #[must_use]
    pub const fn capability(&self) -> &'static str {
        self.role.capability()
    }

    /// Returns the capability version.
    #[must_use]
    pub const fn capability_version(&self) -> u16 {
        CAPABILITY_VERSION
    }

    /// Returns the core version range the artifact is compatible with.
    #[must_use]
    pub const fn compatible_core(&self) -> &'static str {
        COMPATIBLE_CORE
    }

    /// Returns how a browser must execute the artifact.
    #[must_use]
    pub const fn script_kind(&self) -> ScriptKind {
        self.role.script_kind()
    }

    /// Returns the preload relationship recorded for the artifact.
    #[must_use]
    pub const fn preload_relation(&self) -> PreloadRelation {
        self.role.preload_relation()
    }
}

/// The validated manifest plus every artifact it records.
#[derive(Clone, Debug)]
pub struct RuntimeArtifactManifest {
    bytes: &'static [u8],
    engine_version: String,
    protocol_versions: Vec<u16>,
    snapshot_versions: Vec<u16>,
    built_at: String,
    artifacts: Box<[RuntimeArtifact; 8]>,
    identity: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireManifest {
    schema_version: u16,
    engine_version: String,
    runtime_contract_version: u16,
    protocol_versions: Vec<u16>,
    snapshot_versions: Vec<u16>,
    built_at: String,
    assets: Vec<WireAsset>,
    provenance: WireProvenance,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireAsset {
    file: String,
    role: String,
    bytes: u64,
    sha256: String,
    sri: String,
    capability: String,
    capability_version: u16,
    compatible_core: String,
    content_type: String,
    script_kind: String,
    preload_rel: String,
    cache_control: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireProvenance {
    idiomorph: WireIdiomorph,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireIdiomorph {
    name: String,
    version: String,
    license: String,
    bundled: bool,
}

impl RuntimeArtifactManifest {
    /// Validates `manifest` against `files`, the exact bytes available for each recorded file.
    ///
    /// Every role must be recorded exactly once with its contracted file name,
    /// capability, execution kind, preload relationship, media type, and cache
    /// policy, and the bytes for each file must match the recorded length,
    /// digest, and integrity value. Anything else fails closed.
    pub fn validate(
        manifest: &'static [u8],
        files: &'static [(&'static str, &'static [u8])],
    ) -> Result<Self, ArtifactError> {
        if manifest.len() > MAX_MANIFEST_BYTES {
            return Err(ArtifactError::new(ArtifactErrorKind::ManifestInvalid));
        }
        let wire: WireManifest = serde_json::from_slice(manifest)
            .map_err(|_| ArtifactError::new(ArtifactErrorKind::ManifestInvalid))?;
        if wire.schema_version != MANIFEST_SCHEMA_VERSION
            || wire.engine_version != BROWSER_RUNTIME_VERSION
            || wire.runtime_contract_version != RUNTIME_CONTRACT_VERSION
            || wire.protocol_versions != SUPPORTED_PROTOCOL_VERSIONS
            || wire.snapshot_versions != SUPPORTED_SNAPSHOT_VERSIONS
            || wire.built_at != REPRODUCIBLE_BUILD_TIMESTAMP
            || wire.assets.len() != ArtifactRole::ALL.len()
            || wire.provenance.idiomorph.name != "idiomorph"
            || wire.provenance.idiomorph.version != "0.7.4"
            || wire.provenance.idiomorph.license != "0BSD"
            || !wire.provenance.idiomorph.bundled
        {
            return Err(ArtifactError::new(ArtifactErrorKind::ManifestInvalid));
        }
        let mut artifacts = Vec::with_capacity(ArtifactRole::ALL.len());
        for role in ArtifactRole::ALL {
            let mut recorded = wire
                .assets
                .iter()
                .filter(|asset| asset.role == role.as_str());
            let asset = recorded
                .next()
                .ok_or(ArtifactError::new(ArtifactErrorKind::ManifestInvalid))?;
            if recorded.next().is_some() {
                return Err(ArtifactError::new(ArtifactErrorKind::ManifestInvalid));
            }
            if !single_segment_javascript(&asset.file)
                || asset.file != role.file()
                || asset.capability != role.capability()
                || asset.capability_version != CAPABILITY_VERSION
                || asset.compatible_core != COMPATIBLE_CORE
                || asset.content_type != ARTIFACT_CONTENT_TYPE
                || asset.script_kind != role.script_kind().as_str()
                || asset.preload_rel != role.preload_relation().as_str()
                || asset.cache_control != ARTIFACT_CACHE_CONTROL
            {
                return Err(ArtifactError::new(ArtifactErrorKind::MetadataMismatch));
            }
            let bytes = files
                .iter()
                .find(|(file, _)| *file == role.file())
                .map(|(_, bytes)| *bytes)
                .ok_or(ArtifactError::new(ArtifactErrorKind::RoleMissing))?;
            let digest = Sha256::digest(bytes);
            let sha256_hex = hex(&digest);
            let sri = format!("sha256-{}", BASE64.encode(digest));
            if bytes.len() as u64 != asset.bytes
                || bytes.is_empty()
                || asset.sha256 != sha256_hex
                || asset.sri != sri
            {
                return Err(ArtifactError::new(ArtifactErrorKind::IntegrityMismatch));
            }
            artifacts.push(RuntimeArtifact {
                role,
                bytes,
                sha256_hex,
                sri,
            });
        }
        let artifacts: Box<[RuntimeArtifact; 8]> = artifacts
            .into_boxed_slice()
            .try_into()
            .map_err(|_| ArtifactError::new(ArtifactErrorKind::ManifestInvalid))?;
        let manifest_digest = hex(&Sha256::digest(manifest));
        let identity = format!(
            "suprnova-live-{}-{}",
            wire.engine_version,
            &manifest_digest[..IDENTITY_DIGEST_CHARS]
        );
        Ok(Self {
            bytes: manifest,
            engine_version: wire.engine_version,
            protocol_versions: wire.protocol_versions,
            snapshot_versions: wire.snapshot_versions,
            built_at: wire.built_at,
            artifacts,
            identity,
        })
    }

    /// Returns the exact manifest bytes.
    #[must_use]
    pub const fn manifest_bytes(&self) -> &'static [u8] {
        self.bytes
    }

    /// Returns the manifest schema version.
    #[must_use]
    pub const fn schema_version(&self) -> u16 {
        MANIFEST_SCHEMA_VERSION
    }

    /// Returns the browser runtime version the manifest records.
    #[must_use]
    pub fn engine_version(&self) -> &str {
        &self.engine_version
    }

    /// Returns the runtime configuration contract version.
    #[must_use]
    pub const fn runtime_contract_version(&self) -> u16 {
        RUNTIME_CONTRACT_VERSION
    }

    /// Returns the wire protocol versions the runtime speaks.
    #[must_use]
    pub fn protocol_versions(&self) -> &[u16] {
        &self.protocol_versions
    }

    /// Returns the snapshot versions the runtime understands.
    #[must_use]
    pub fn snapshot_versions(&self) -> &[u16] {
        &self.snapshot_versions
    }

    /// Returns the reproducible build timestamp.
    #[must_use]
    pub fn built_at(&self) -> &str {
        &self.built_at
    }

    /// Returns every artifact in [`ArtifactRole::ALL`] order.
    #[must_use]
    pub fn artifacts(&self) -> &[RuntimeArtifact] {
        &self.artifacts[..]
    }

    /// Returns the artifact serving `role`.
    ///
    /// Validation stores the artifacts in [`ArtifactRole::ALL`] order, so the
    /// lookup is an index and cannot fail.
    #[must_use]
    pub fn artifact(&self, role: ArtifactRole) -> &RuntimeArtifact {
        &self.artifacts[role.index()]
    }

    /// Returns the artifact recorded under the exact single-segment `file` name.
    #[must_use]
    pub fn artifact_by_file(&self, file: &str) -> Option<&RuntimeArtifact> {
        self.artifacts
            .iter()
            .find(|artifact| artifact.file() == file)
    }

    /// Returns the deterministic identity of this artifact set.
    ///
    /// The identity binds the runtime version and the manifest digest, so a
    /// host can address immutable URLs by it and a document can name it in
    /// its inert configuration.
    #[must_use]
    pub fn asset_identity(&self) -> &str {
        &self.identity
    }
}

/// Returns the embedded artifacts after validating them against the embedded manifest.
///
/// Validation runs once per process; a failure is remembered and returned on
/// every call so no host serves partially validated bytes.
pub fn runtime_artifacts() -> Result<&'static RuntimeArtifactManifest, ArtifactError> {
    static VALIDATED: OnceLock<Result<RuntimeArtifactManifest, ArtifactError>> = OnceLock::new();
    VALIDATED
        .get_or_init(|| RuntimeArtifactManifest::validate(MANIFEST_BYTES, EMBEDDED_FILES))
        .as_ref()
        .map_err(|error| *error)
}

fn single_segment_javascript(file: &str) -> bool {
    let path = Path::new(file);
    let mut components = path.components();
    !file.is_empty()
        && file.len() <= MAX_FILE_NAME_BYTES
        && file.is_ascii()
        && !file.bytes().any(|byte| byte.is_ascii_control())
        && matches!(components.next(), Some(Component::Normal(_)))
        && components.next().is_none()
        && file.ends_with(".js")
}

fn hex(digest: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut text = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(&mut text, "{byte:02x}").expect("formatting into a String cannot fail");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::single_segment_javascript;

    #[test]
    fn artifact_file_names_are_single_closed_segments() {
        assert!(single_segment_javascript("suprnova-live.esm.js"));
        for hostile in [
            "",
            "../runtime.js",
            "/runtime.js",
            "src/runtime.js",
            "runtime.ts",
            "run\ntime.js",
        ] {
            assert!(!single_segment_javascript(hostile), "{hostile}");
        }
    }
}
