//! Helpers the kit test binaries share: scaffolding a kit with the CLI,
//! running a command to completion with its output in a log, copying a
//! tree, and pointing a scaffold at the in-tree framework.
//!
//! `kit_browser.rs` and `agents_browser/kits.rs` include this with
//! `#[path]`; each uses only some of it.
#![allow(dead_code)]

use std::fs;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use tokio::process::Command as AsyncCommand;

/// The CLI binary under test.
pub const CLI: &str = env!("CARGO_BIN_EXE_suprnova");

/// The three Inertia kits.
pub const KITS: [&str; 3] = ["svelte", "react", "vue"];

/// The file at `path`, or the reason it could not be read.
pub fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|error| format!("read {}: {error}", path.display()))
}

/// Scaffold a project of `kit` named `name` inside `parent` with the CLI,
/// and return the project's directory.
pub fn scaffold(parent: &Path, name: &str, kit: &str) -> Result<PathBuf, String> {
    let output = Command::new(CLI)
        .args([
            "new",
            name,
            "--no-interaction",
            "--no-git",
            "--frontend",
            kit,
        ])
        .current_dir(parent)
        .output()
        .map_err(|error| format!("run `suprnova new`: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "`suprnova new --frontend {kit}` failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        ));
    }
    Ok(parent.join(name))
}

/// Run `command` within `timeout`, its stdout and stderr together in a
/// durable `log`, including on timeout. A timed-out command's whole process
/// group is killed: Cargo, npm and npx start children.
pub fn run(command: &mut AsyncCommand, log: &Path, timeout: Duration) -> Result<(), String> {
    let description = format!("{command:?}");
    eprintln!("kit test: {description}; log {}", log.display());
    let file = fs::File::create(log).map_err(|error| error.to_string())?;
    command.stdout(file.try_clone().map_err(|error| error.to_string())?);
    command.stderr(file);
    let runtime = tokio::runtime::Runtime::new().map_err(|error| error.to_string())?;
    #[cfg(unix)]
    command.as_std_mut().process_group(0);
    command.kill_on_drop(true);
    runtime.block_on(async {
        let mut child = command
            .spawn()
            .map_err(|error| format!("{description}: {error}"))?;
        #[cfg(unix)]
        let process_group = child.id();
        match tokio::time::timeout(timeout, child.wait()).await {
            Ok(Ok(status)) if status.success() => {
                eprintln!("kit test: {description}: {status}\n{}", read(log));
                Ok(())
            }
            result => {
                #[cfg(unix)]
                if let Some(pid) = process_group {
                    let _ = nix::sys::signal::killpg(
                        nix::unistd::Pid::from_raw(pid as i32),
                        nix::sys::signal::Signal::SIGKILL,
                    );
                }
                // Reap a timed-out child before reading the final log.
                let _ = child.kill().await;
                let _ = child.wait().await;
                Err(format!("{description}: {result:?}\n{}", read(log)))
            }
        }
    })
}

/// Use the in-tree framework and detach the scaffold from any workspace,
/// as `live_generated_app.rs` does.
pub fn patch_local_suprnova(project: &Path) -> Result<(), String> {
    let framework = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or("workspace root")?
        .join("framework");
    let manifest = project.join("Cargo.toml");
    let original = fs::read_to_string(&manifest).map_err(|error| error.to_string())?;
    let mut replaced = false;
    let mut rewritten = String::new();
    for line in original.lines() {
        if line.trim_start().starts_with("suprnova = ") {
            rewritten.push_str(&format!("suprnova = {{ path = {:?} }}\n", framework));
            replaced = true;
        } else {
            rewritten.push_str(line);
            rewritten.push('\n');
        }
    }
    if !replaced {
        return Err("scaffold has no suprnova dependency".into());
    }
    if !original.lines().any(|line| line.trim() == "[workspace]") {
        rewritten.push_str("\n[workspace]\n");
    }
    fs::write(manifest, rewritten).map_err(|error| error.to_string())
}

/// Copy the tree at `source` into `destination`.
pub fn copy_tree(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination).map_err(|error| error.to_string())?;
    for entry in fs::read_dir(source).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let target = destination.join(entry.file_name());
        if entry
            .file_type()
            .map_err(|error| error.to_string())?
            .is_dir()
        {
            copy_tree(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target).map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}
