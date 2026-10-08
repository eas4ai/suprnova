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

/// RDOC-006: the violating example for the `inertia-root-template`
/// mechanism's fail receipt. The scaffold ships a root template under
/// `templates/` and no `frontend/index.html`, which the server never
/// serves.
#[test]
fn rdoc_the_scaffold_ships_a_root_template_in_place_of_frontend_index_html() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let output = Command::new(BIN)
        .args([
            "new",
            "rdocapp",
            "--no-interaction",
            "--no-git",
            "--frontend",
            "svelte",
        ])
        .current_dir(tmp.path())
        .output()
        .expect("spawn");
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let project = tmp.path().join("rdocapp");
    assert!(
        !project.join("frontend/index.html").exists(),
        "the scaffold still writes frontend/index.html, a shell the server never serves"
    );
    assert!(
        project.join("templates").join("inertia").is_dir()
            || project.join("templates").join("root.html").exists(),
        "the scaffold ships no root template under templates/"
    );
}
