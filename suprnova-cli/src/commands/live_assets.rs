//! `suprnova live:assets` - publish the reviewed Live runtime artifacts into
//! a directory the application or a CDN serves.
//!
//! The application helper exports the exact reviewed bytes; this command
//! verifies their digests on the transport, stages a complete
//! `<out>/<identity>/` directory, and renames it into place. An existing
//! identical publication is left untouched; one whose bytes differ is refused
//! unless `--replace` is given.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::commands::live_check::{explain_helper_failure, require_project};
use crate::commands::live_tool::{self, Operation, Outcome, PublishedAsset};
use crate::secure_fs;
use crate::ui;

pub fn run(out: PathBuf, replace: bool, timeout_secs: u64) {
    if let Err(e) = run_inner(&out, replace, timeout_secs) {
        ui::error(&e);
        std::process::exit(1);
    }
}

fn identity_ok(identity: &str) -> bool {
    let mut bytes = identity.bytes();
    !identity.is_empty()
        && identity.len() <= 128
        && bytes
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && bytes
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
}

/// Compare an existing publication with the exported assets: `Ok(true)` when
/// the file set and every byte match, `Ok(false)` on any difference.
fn publication_matches(target: &Path, assets: &[PublishedAsset]) -> Result<bool, String> {
    let mut on_disk = BTreeMap::new();
    for entry in
        fs::read_dir(target).map_err(|e| format!("Failed to read {}: {e}", target.display()))?
    {
        let entry = entry.map_err(|e| format!("Failed to read {}: {e}", target.display()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let metadata = fs::symlink_metadata(entry.path())
            .map_err(|e| format!("Failed to inspect {}: {e}", entry.path().display()))?;
        if !metadata.is_file() {
            return Ok(false);
        }
        let bytes = fs::read(entry.path())
            .map_err(|e| format!("Failed to read {}: {e}", entry.path().display()))?;
        on_disk.insert(name, bytes);
    }
    if on_disk.len() != assets.len() {
        return Ok(false);
    }
    Ok(assets.iter().all(|asset| {
        on_disk
            .get(&asset.file)
            .is_some_and(|bytes| *bytes == asset.bytes)
    }))
}

fn run_inner(out: &Path, replace: bool, timeout_secs: u64) -> Result<(), String> {
    require_project()?;
    secure_fs::ensure_contained(Path::new("."), out)?;
    ui::hint("Building and running the application's Live tooling helper...");
    let session = live_tool::run(Operation::Assets, &[], Duration::from_secs(timeout_secs))
        .map_err(|e| e.to_string())?;
    if session.outcome == Outcome::Failed {
        return Err(explain_helper_failure(
            session.error.as_deref().unwrap_or("unknown failure"),
        ));
    }
    let identity = session.assets.as_deref().ok_or_else(|| {
        "The application's reviewed artifacts did not validate; nothing was published".to_string()
    })?;
    if !identity_ok(identity) {
        return Err("The application helper reported an unsafe asset identity".to_string());
    }
    if session.assets_out.is_empty() {
        return Err("The application helper exported no assets".to_string());
    }
    let target = out.join(identity);
    secure_fs::ensure_contained(Path::new("."), &target)?;
    match fs::symlink_metadata(&target) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(format!(
                "{} is a symlink; refusing to publish through it",
                target.display()
            ));
        }
        Ok(metadata) if metadata.is_dir() => {
            if publication_matches(&target, &session.assets_out)? {
                ui::success(&format!(
                    "Live assets are up to date at {}",
                    target.display()
                ));
                return Ok(());
            }
            if !replace {
                ui::hint(
                    "Pass --replace to replace the existing publication with the reviewed bytes.",
                );
                return Err(format!(
                    "{} exists and differs from the reviewed artifacts; pass --replace to replace it",
                    target.display()
                ));
            }
        }
        Ok(_) => {
            return Err(format!(
                "{} exists and is not a directory",
                target.display()
            ));
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("Failed to inspect {}: {e}", target.display())),
    }
    let pid = std::process::id();
    let staging = out.join(format!(".{identity}.{pid}.staging"));
    let result = publish(out, &staging, &target, &session.assets_out, replace, pid);
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result?;
    let total: usize = session
        .assets_out
        .iter()
        .map(|asset| asset.bytes.len())
        .sum();
    ui::success(&format!(
        "Published {} files ({total} bytes) to {}",
        session.assets_out.len(),
        target.display()
    ));
    ui::hint(
        "Serve this directory with long immutable caching; the framework's bootstrap references",
    );
    ui::hint(&format!(
        "/__live/v1/assets/{identity}/<file> for this exact build."
    ));
    Ok(())
}

fn publish(
    out: &Path,
    staging: &Path,
    target: &Path,
    assets: &[PublishedAsset],
    replace: bool,
    pid: u32,
) -> Result<(), String> {
    secure_fs::ensure_contained(Path::new("."), staging)?;
    let _ = fs::remove_dir_all(staging);
    fs::create_dir_all(staging)
        .map_err(|e| format!("Failed to create {}: {e}", staging.display()))?;
    for asset in assets {
        secure_fs::write_atomic(&staging.join(&asset.file), &asset.bytes)?;
    }
    if replace && target.exists() {
        let retired = out.join(format!(
            ".{}.{pid}.replaced",
            target
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("publication")
        ));
        let _ = fs::remove_dir_all(&retired);
        fs::rename(target, &retired)
            .map_err(|e| format!("Failed to retire {}: {e}", target.display()))?;
        if let Err(e) = fs::rename(staging, target) {
            let _ = fs::rename(&retired, target);
            return Err(format!(
                "Failed to move the new publication into place: {e}"
            ));
        }
        let _ = fs::remove_dir_all(&retired);
        return Ok(());
    }
    fs::rename(staging, target).map_err(|e| {
        format!(
            "Failed to move {} into place at {}: {e}",
            staging.display(),
            target.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identities_are_bounded_single_components() {
        assert!(identity_ok("suprnova-live-0.1.0-0123456789abcdef"));
        assert!(!identity_ok(""));
        assert!(!identity_ok(".hidden"));
        assert!(!identity_ok("a/b"));
        assert!(!identity_ok(&"a".repeat(129)));
    }
}
