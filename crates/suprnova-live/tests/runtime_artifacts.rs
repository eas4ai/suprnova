//! The embedded browser artifacts are exactly the reviewed production build.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use sha2::{Digest as _, Sha256};
use suprnova_live::artifacts::{
    ArtifactErrorKind, ArtifactRole, BROWSER_RUNTIME_VERSION, PreloadRelation,
    RUNTIME_CONTRACT_VERSION, RuntimeArtifactManifest, ScriptKind, runtime_artifacts,
};
use suprnova_live::{SUPPORTED_PROTOCOL_VERSIONS, SUPPORTED_SNAPSHOT_VERSIONS};

const TRACKED_MANIFEST: &[u8] = include_bytes!("../browser/dist/suprnova-live.assets.json");

fn hex(digest: &[u8]) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
fn embedded_artifacts_match_the_tracked_manifest_byte_for_byte() {
    let manifest = runtime_artifacts().expect("embedded artifacts validate");
    assert_eq!(manifest.manifest_bytes(), TRACKED_MANIFEST);
    assert_eq!(manifest.schema_version(), 2);
    assert_eq!(manifest.engine_version(), BROWSER_RUNTIME_VERSION);
    assert_eq!(
        manifest.runtime_contract_version(),
        RUNTIME_CONTRACT_VERSION
    );
    assert_eq!(manifest.protocol_versions(), SUPPORTED_PROTOCOL_VERSIONS);
    assert_eq!(manifest.snapshot_versions(), SUPPORTED_SNAPSHOT_VERSIONS);
    assert_eq!(manifest.built_at(), "1970-01-01T00:00:00.000Z");
    assert_eq!(manifest.artifacts().len(), ArtifactRole::ALL.len());

    let parsed: serde_json::Value =
        serde_json::from_slice(TRACKED_MANIFEST).expect("manifest is JSON");
    for role in ArtifactRole::ALL {
        let artifact = manifest.artifact(role);
        assert_eq!(artifact.role(), role);
        let recorded = parsed["assets"]
            .as_array()
            .expect("assets")
            .iter()
            .find(|entry| entry["role"] == role.as_str())
            .unwrap_or_else(|| panic!("manifest records {}", role.as_str()));
        assert_eq!(artifact.file(), recorded["file"].as_str().expect("file"));
        assert_eq!(
            artifact.bytes().len() as u64,
            recorded["bytes"].as_u64().expect("bytes")
        );
        let digest = Sha256::digest(artifact.bytes());
        assert_eq!(hex(&digest), recorded["sha256"].as_str().expect("sha256"));
        assert_eq!(artifact.sha256_hex(), hex(&digest));
        assert_eq!(artifact.sri(), format!("sha256-{}", BASE64.encode(digest)));
        assert_eq!(artifact.sri(), recorded["sri"].as_str().expect("sri"));
        assert_eq!(artifact.content_type(), "text/javascript; charset=utf-8");
        assert_eq!(
            artifact.cache_control(),
            "public, max-age=31536000, immutable"
        );
        assert_eq!(artifact.capability(), role.capability());
        assert_eq!(artifact.capability_version(), 1);
        assert_eq!(artifact.compatible_core(), ">=0.1.0 <0.2.0");
        assert_eq!(artifact.script_kind(), role.script_kind());
        assert_eq!(artifact.preload_relation(), role.preload_relation());
        assert!(!artifact.bytes().is_empty());
        assert!(!artifact.bytes().contains(&0));
    }
}

#[test]
fn roles_are_a_closed_typed_set() {
    assert_eq!(ArtifactRole::ALL.len(), 8);
    assert_eq!(ArtifactRole::CoreEsm.file(), "suprnova-live.esm.js");
    assert_eq!(ArtifactRole::CoreClassic.file(), "suprnova-live.classic.js");
    assert_eq!(
        ArtifactRole::UploadsEsm.file(),
        "suprnova-live.uploads.esm.js"
    );
    assert_eq!(
        ArtifactRole::AsyncClassic.script_kind(),
        ScriptKind::Classic
    );
    assert_eq!(ArtifactRole::StimulusEsm.script_kind(), ScriptKind::Module);
    assert_eq!(
        ArtifactRole::StimulusEsm.preload_relation(),
        PreloadRelation::ModulePreload
    );
    assert_eq!(
        ArtifactRole::UploadsClassic.preload_relation(),
        PreloadRelation::Preload
    );
    assert_eq!(ArtifactRole::AsyncEsm.capability(), "async@1");
    assert_eq!(ArtifactRole::CoreClassic.capability(), "core@1");
    for role in ArtifactRole::ALL {
        assert_eq!(ArtifactRole::parse(role.as_str()), Some(role));
        assert!(role.file().ends_with(".js"));
        assert!(!role.file().contains('/'));
    }
    assert_eq!(ArtifactRole::parse("core"), None);
    assert_eq!(ArtifactRole::parse("../suprnova-live.esm.js"), None);
}

#[test]
fn the_asset_identity_is_stable_bounded_and_manifest_derived() {
    let manifest = runtime_artifacts().expect("embedded artifacts validate");
    let identity = manifest.asset_identity();
    assert_eq!(
        identity,
        runtime_artifacts().expect("same").asset_identity()
    );
    assert!(identity.starts_with("suprnova-live-0.1.0-"));
    assert!(identity.len() <= 128);
    assert!(
        identity
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
    );
    let digest = hex(&Sha256::digest(TRACKED_MANIFEST));
    assert!(identity.ends_with(&digest[..16]));
}

fn leaked(bytes: Vec<u8>) -> &'static [u8] {
    Box::leak(bytes.into_boxed_slice())
}

fn embedded_files() -> Vec<(&'static str, &'static [u8])> {
    let manifest = runtime_artifacts().expect("embedded artifacts validate");
    ArtifactRole::ALL
        .into_iter()
        .map(|role| {
            let artifact = manifest.artifact(role);
            (artifact.file(), artifact.bytes())
        })
        .collect()
}

#[test]
fn a_manifest_that_disagrees_with_the_bytes_is_rejected() {
    let files = embedded_files();
    let mut tampered: serde_json::Value =
        serde_json::from_slice(TRACKED_MANIFEST).expect("manifest is JSON");
    tampered["assets"][0]["sha256"] = serde_json::Value::String("0".repeat(64));
    let error = RuntimeArtifactManifest::validate(
        leaked(serde_json::to_vec(&tampered).expect("encode")),
        leaked_files(&files),
    )
    .expect_err("digest drift fails closed");
    assert_eq!(error.kind(), ArtifactErrorKind::IntegrityMismatch);

    let mut tampered: serde_json::Value =
        serde_json::from_slice(TRACKED_MANIFEST).expect("manifest is JSON");
    tampered["assets"][0]["bytes"] = serde_json::Value::from(1_u64);
    let error = RuntimeArtifactManifest::validate(
        leaked(serde_json::to_vec(&tampered).expect("encode")),
        leaked_files(&files),
    )
    .expect_err("length drift fails closed");
    assert_eq!(error.kind(), ArtifactErrorKind::IntegrityMismatch);

    let mut tampered: serde_json::Value =
        serde_json::from_slice(TRACKED_MANIFEST).expect("manifest is JSON");
    tampered["assets"][0]["capability"] = serde_json::Value::String("core@2".to_owned());
    let error = RuntimeArtifactManifest::validate(
        leaked(serde_json::to_vec(&tampered).expect("encode")),
        leaked_files(&files),
    )
    .expect_err("capability drift fails closed");
    assert_eq!(error.kind(), ArtifactErrorKind::MetadataMismatch);

    let mut tampered: serde_json::Value =
        serde_json::from_slice(TRACKED_MANIFEST).expect("manifest is JSON");
    tampered["assets"][0]["file"] = serde_json::Value::String("../evil.js".to_owned());
    let error = RuntimeArtifactManifest::validate(
        leaked(serde_json::to_vec(&tampered).expect("encode")),
        leaked_files(&files),
    )
    .expect_err("path drift fails closed");
    assert_eq!(error.kind(), ArtifactErrorKind::MetadataMismatch);

    let mut tampered: serde_json::Value =
        serde_json::from_slice(TRACKED_MANIFEST).expect("manifest is JSON");
    tampered["engine_version"] = serde_json::Value::String("0.2.0".to_owned());
    let error = RuntimeArtifactManifest::validate(
        leaked(serde_json::to_vec(&tampered).expect("encode")),
        leaked_files(&files),
    )
    .expect_err("version drift fails closed");
    assert_eq!(error.kind(), ArtifactErrorKind::ManifestInvalid);

    let mut tampered: serde_json::Value =
        serde_json::from_slice(TRACKED_MANIFEST).expect("manifest is JSON");
    tampered["assets"].as_array_mut().expect("assets").pop();
    let error = RuntimeArtifactManifest::validate(
        leaked(serde_json::to_vec(&tampered).expect("encode")),
        leaked_files(&files),
    )
    .expect_err("a missing role fails closed");
    assert_eq!(error.kind(), ArtifactErrorKind::ManifestInvalid);

    let mut missing = files.clone();
    missing.retain(|(file, _)| *file != "suprnova-live.async.esm.js");
    let error = RuntimeArtifactManifest::validate(TRACKED_MANIFEST, leaked_files(&missing))
        .expect_err("absent bytes fail closed");
    assert_eq!(error.kind(), ArtifactErrorKind::RoleMissing);
    assert_eq!(error.to_string(), "browser_artifact_role_missing");

    assert!(RuntimeArtifactManifest::validate(b"not json", leaked_files(&files)).is_err());
}

fn leaked_files(
    files: &[(&'static str, &'static [u8])],
) -> &'static [(&'static str, &'static [u8])] {
    Box::leak(files.to_vec().into_boxed_slice())
}
