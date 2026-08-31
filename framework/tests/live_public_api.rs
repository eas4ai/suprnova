use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("framework lives under workspace root")
        .to_path_buf()
}

fn html_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory).expect("read rustdoc directory") {
            let path = entry.expect("read rustdoc entry").path();
            if path.is_dir() {
                pending.push(path);
            } else if path
                .extension()
                .is_some_and(|extension| extension == "html")
            {
                files.push(path);
            }
        }
    }
    files
}

#[test]
fn rendered_public_docs_do_not_expose_internal_crate_paths() {
    let root = workspace_root();
    let target = root.join("target/live-public-api-rustdoc");
    let docs = target.join("doc/suprnova");
    if docs.exists() {
        fs::remove_dir_all(&docs).expect("remove stale isolated framework rustdoc");
    }
    let status = Command::new("cargo")
        .args(["doc", "-p", "suprnova", "--no-deps", "--target-dir"])
        .arg(&target)
        .current_dir(&root)
        .env("CARGO_BUILD_JOBS", "1")
        .env("CARGO_INCREMENTAL", "0")
        .status()
        .expect("build isolated framework rustdoc");
    assert!(status.success(), "isolated framework rustdoc failed");

    for module in ["live", "view"] {
        let module_docs = docs.join(module);
        for path in html_files(&module_docs) {
            if path
                .strip_prefix(&module_docs)
                .expect("module document path")
                .components()
                .any(|component| component.as_os_str() == "__private")
            {
                continue;
            }
            let html = fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
            for forbidden in ["suprnova_live", "suprnova_live_macros", "askama_parser"] {
                assert!(
                    !html.contains(forbidden),
                    "public {module} documentation exposed internal path {forbidden} in {}",
                    path.display()
                );
            }
        }
    }
}
