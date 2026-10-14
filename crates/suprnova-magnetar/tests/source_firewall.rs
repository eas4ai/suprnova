use std::fs;
use std::path::{Path, PathBuf};

fn walk(root: &Path, paths: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        paths.push(path.clone());
        if path.is_dir() {
            walk(&path, paths);
        }
    }
}

#[test]
fn gpl_broker_is_not_in_the_source_tree() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let forbidden = [
        "reference/oauth2-broker-0.1.3",
        "src/oauth2_broker.rs",
        "src/oauth2-broker.rs",
    ];
    for relative in forbidden {
        assert!(
            !root.join(relative).exists(),
            "GPL source must not be vendored: {relative}"
        );
    }

    let mut paths = Vec::new();
    walk(root, &mut paths);
    for path in paths {
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_ascii_lowercase();
        assert!(
            !relative.contains("oauth2-broker") && !relative.contains("oauth2_broker"),
            "GPL broker path must not enter the crate tree: {relative}"
        );
    }
}
