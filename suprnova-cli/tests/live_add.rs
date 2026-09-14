//! `live:add` installs a library component from its manifest into one
//! directory under `templates/`, never overwrites an edited file, and accepts a
//! third-party manifest in the same format (Cairn UI-017).

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_suprnova");

fn project() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().expect("tempdir");
    fs::write(
        tmp.path().join("Cargo.toml"),
        "[package]\nname = \"demo-app\"\nversion = \"0.1.0\"\n",
    )
    .expect("manifest");
    tmp
}

fn add(root: &Path, args: &[&str]) -> Output {
    Command::new(BIN)
        .arg("live:add")
        .args(args)
        .current_dir(root)
        .output()
        .expect("spawn")
}

fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn a_shipped_component_installs_as_one_directory_and_an_edit_survives_a_second_run() {
    let project = project();
    let root = project.path();

    let first = add(root, &["field"]);
    assert!(first.status.success(), "{}", combined(&first));
    let directory = root.join("templates/suprnova-ui/field");
    for file in ["manifest.json", "field.html", "field.css"] {
        assert!(directory.join(file).is_file(), "{file} installed");
    }
    let view = directory.join("field.html");
    let shipped = fs::read_to_string(&view).expect("view");
    assert!(shipped.contains("{% macro field("));
    let manifest: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(directory.join("manifest.json")).expect("manifest"),
    )
    .expect("manifest json");
    assert_eq!(manifest["name"], "suprnova.field");
    assert_eq!(manifest["root"], "suprnova-ui/field");

    let edited = format!("{shipped}\n{{# edited by the application #}}\n");
    fs::write(&view, &edited).expect("edit the view");
    let second = add(root, &["field"]);
    assert!(second.status.success(), "{}", combined(&second));
    assert_eq!(
        fs::read_to_string(&view).expect("view"),
        edited,
        "the edited view survives a second install"
    );
    let report = combined(&second);
    assert!(report.contains("kept, edited locally"), "{report}");
    assert!(report.contains("unchanged"), "{report}");

    let forced = add(root, &["field", "--force"]);
    assert!(forced.status.success(), "{}", combined(&forced));
    assert_eq!(
        fs::read_to_string(&view).expect("view"),
        shipped,
        "--force restores the shipped view"
    );
}

#[test]
fn a_dry_run_writes_nothing_and_an_unknown_name_is_refused() {
    let project = project();
    let root = project.path();
    let dry = add(root, &["field", "--dry-run"]);
    assert!(dry.status.success(), "{}", combined(&dry));
    assert!(!root.join("templates").exists(), "dry run wrote nothing");
    let unknown = add(root, &["carousel"]);
    assert!(!unknown.status.success());
    assert!(combined(&unknown).contains("not a shipped library component"));
}

#[test]
fn a_third_party_manifest_installs_under_its_own_root_and_may_not_claim_the_library_root() {
    let project = project();
    let root = project.path();
    let vendor = project.path().join("vendor/acme-widget");
    fs::create_dir_all(&vendor).expect("vendor dir");
    fs::write(
        vendor.join("manifest.json"),
        "{\n  \"name\": \"acme.widget\",\n  \"version\": 1,\n  \"root\": \"acme-ui/widget\",\n  \"files\": [\"widget.html\", \"widget.css\"],\n  \"elements\": []\n}\n",
    )
    .expect("manifest");
    fs::write(
        vendor.join("widget.html"),
        "{% macro widget(name) %}<div class=\"acme-widget\">{{ name }}</div>{% endmacro %}\n",
    )
    .expect("view");
    fs::write(
        vendor.join("widget.css"),
        ".acme-widget { display: block; }\n",
    )
    .expect("css");

    let installed = add(
        root,
        &[
            "--manifest",
            vendor.join("manifest.json").to_str().expect("utf-8"),
        ],
    );
    assert!(installed.status.success(), "{}", combined(&installed));
    let directory = root.join("templates/acme-ui/widget");
    for file in ["manifest.json", "widget.html", "widget.css"] {
        assert!(directory.join(file).is_file(), "{file} installed");
    }

    fs::write(
        vendor.join("manifest.json"),
        "{\n  \"name\": \"acme.widget\",\n  \"version\": 1,\n  \"root\": \"suprnova-ui/widget\",\n  \"files\": [\"widget.html\"],\n  \"elements\": []\n}\n",
    )
    .expect("manifest");
    let refused = add(
        root,
        &[
            "--manifest",
            vendor.join("manifest.json").to_str().expect("utf-8"),
        ],
    );
    assert!(!refused.status.success());
    assert!(combined(&refused).contains("reserved for the shipped library"));
    assert!(!root.join("templates/suprnova-ui").exists());
}
