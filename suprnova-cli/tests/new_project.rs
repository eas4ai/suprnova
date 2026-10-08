//! `suprnova new` refuses a project path that already names anything on disk.

use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_suprnova");

#[cfg(unix)]
#[test]
fn a_dangling_symlink_at_the_project_path_is_refused() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let target = tmp.path().join("elsewhere");
    std::os::unix::fs::symlink(&target, tmp.path().join("myapp")).expect("dangling symlink");
    let output = Command::new(BIN)
        .args([
            "new",
            "myapp",
            "--no-interaction",
            "--no-git",
            "--frontend",
            "svelte",
        ])
        .current_dir(tmp.path())
        .output()
        .expect("spawn");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output.status.success(), "{text}");
    assert!(text.contains("already exists"), "{text}");
    assert!(
        !target.exists(),
        "nothing was created at the link target: {}",
        target.display()
    );
}
