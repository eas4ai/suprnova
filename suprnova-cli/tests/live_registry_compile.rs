//! `registries-compile`: a library the SDK scaffolds installs into a
//! scaffolded application, compiles and renders through its Live route, and
//! the manual's commands produce a signed, tagged library (REG-005, REG-018,
//! REG-021). Each test scaffolds and compiles a project, so the suite is
//! ignored by default and the gate runs it with `-- --ignored`.

use std::path::Path;
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_suprnova");

fn run(cwd: &Path, args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .current_dir(cwd)
        .env_remove("SUPRNOVA_LIBRARY_KEY")
        .output()
        .expect("run suprnova")
}

fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// REG-018: `live:registry new <namespace>` scaffolds the library tree with
/// one example component and a preview application, writes the public key
/// into `library.json`, and keeps the private key outside the project.
#[test]
#[ignore = "scaffolds a project; the gate runs it with -- --ignored"]
fn reg_018_registry_new_scaffolds_the_tree_with_an_example_and_a_key_outside_the_project() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().join("home");
    std::fs::create_dir_all(&home).expect("home");
    let output = Command::new(BIN)
        .args(["live:registry", "new", "acme"])
        .current_dir(tmp.path())
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env_remove("SUPRNOVA_LIBRARY_KEY")
        .output()
        .expect("run live:registry new");
    let text = combined(&output);
    assert!(output.status.success(), "live:registry new failed:\n{text}");
    let library = tmp.path().join("acme");
    assert!(
        library.join("library.json").is_file(),
        "no library.json:\n{text}"
    );
    assert!(
        library.join("components").is_dir(),
        "no components/:\n{text}"
    );
    assert!(
        library.join("preview/Cargo.toml").is_file(),
        "no preview application:\n{text}"
    );
    let secrets: Vec<_> = walkdir::WalkDir::new(&library)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry.file_name().to_string_lossy().contains("key") && entry.file_type().is_file()
        })
        .filter(|entry| {
            std::fs::read(entry.path())
                .is_ok_and(|bytes| bytes.len() >= 32 && !entry.path().ends_with("library.json"))
        })
        .collect();
    assert!(
        secrets.is_empty(),
        "a key file sits inside the project: {secrets:?}"
    );
    let check = run(&library, &["live:registry", "check"]);
    assert!(
        check.status.success(),
        "the scaffold fails its own check:\n{}",
        combined(&check)
    );
}
