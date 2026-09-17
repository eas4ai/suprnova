//! `live:add` installs a library component from its manifest into one
//! directory under `templates/`, never overwrites an edited file, replaces an
//! unedited one when the shipped file changes, and accepts a third-party
//! manifest in the same format whose files are regular files in its directory
//! (Cairn UI-017, UI-022, UI-023).

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use sha2::Digest as _;

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

fn vendor_widget(root: &Path, view: &str) -> std::path::PathBuf {
    let vendor = root.join("vendor/acme-widget");
    fs::create_dir_all(&vendor).expect("vendor dir");
    fs::write(
        vendor.join("manifest.json"),
        "{\n  \"name\": \"acme.widget\",\n  \"version\": 1,\n  \"root\": \"acme-ui/widget\",\n  \"files\": [\"widget.html\", \"widget.css\"],\n  \"elements\": []\n}\n",
    )
    .expect("manifest");
    fs::write(vendor.join("widget.html"), view).expect("view");
    fs::write(vendor.join("widget.css"), ".acme-widget { display: block; }\n").expect("css");
    vendor.join("manifest.json")
}

fn add_manifest(root: &Path, manifest: &Path) -> Output {
    add(root, &["--manifest", manifest.to_str().expect("utf-8")])
}

#[test]
fn ui_022_an_unedited_older_install_is_replaced_and_an_edited_one_is_kept() {
    let project = project();
    let root = project.path();
    let older = "{% macro widget(name) %}<div>{{ name }}</div>{% endmacro %}\n";
    let newer = "{% macro widget(name) %}<div class=\"acme-widget\">{{ name }}</div>{% endmacro %}\n";
    let manifest = vendor_widget(root, older);
    let installed = add_manifest(root, &manifest);
    assert!(installed.status.success(), "{}", combined(&installed));
    let view = root.join("templates/acme-ui/widget/widget.html");
    assert_eq!(fs::read_to_string(&view).expect("view"), older);

    fs::write(root.join("vendor/acme-widget/widget.html"), newer).expect("newer view");
    let upgraded = add_manifest(root, &manifest);
    assert!(upgraded.status.success(), "{}", combined(&upgraded));
    assert_eq!(
        fs::read_to_string(&view).expect("view"),
        newer,
        "an unedited file follows the shipped file"
    );
    let report = combined(&upgraded);
    assert!(report.contains("replaced"), "{report}");
    assert!(!report.contains("edited locally"), "{report}");

    let edited = format!("{newer}{{# edited by the application #}}\n");
    fs::write(&view, &edited).expect("edit the view");
    fs::write(root.join("vendor/acme-widget/widget.html"), older).expect("another shipped view");
    let kept = add_manifest(root, &manifest);
    assert!(kept.status.success(), "{}", combined(&kept));
    assert_eq!(fs::read_to_string(&view).expect("view"), edited, "the edit survives");
    let report = combined(&kept);
    assert!(report.contains("kept, edited locally"), "{report}");

    // The edit stays known across runs: a second run still keeps it.
    let again = add_manifest(root, &manifest);
    assert!(combined(&again).contains("kept, edited locally"), "{}", combined(&again));
}

#[test]
fn ui_022_a_shipped_file_installed_from_an_older_release_is_replaced() {
    let project = project();
    let root = project.path();
    let first = add(root, &["field"]);
    assert!(first.status.success(), "{}", combined(&first));
    let directory = root.join("templates/suprnova-ui/field");
    let view = directory.join("field.html");
    let shipped = fs::read_to_string(&view).expect("view");

    // An install from an older release: other bytes, which its record vouches for.
    let older = "{% macro field(name, label) %}<div>{{ caller() }}</div>{% endmacro %}\n";
    fs::write(&view, older).expect("older view");
    let record_path = directory.join(".suprnova-installed.json");
    let mut record: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(&fs::read_to_string(&record_path).expect("install record"))
            .expect("record json");
    let digest: String = sha2::Sha256::digest(older.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    record.insert("field.html".to_owned(), serde_json::Value::String(digest));
    fs::write(&record_path, serde_json::to_vec(&record).expect("encode")).expect("record");

    let upgraded = add(root, &["field"]);
    assert!(upgraded.status.success(), "{}", combined(&upgraded));
    assert_eq!(fs::read_to_string(&view).expect("view"), shipped);
    assert!(!combined(&upgraded).contains("edited locally"), "{}", combined(&upgraded));
}

#[test]
fn ui_022_a_differing_file_no_record_vouches_for_is_kept_with_the_reason() {
    let project = project();
    let root = project.path();
    let first = add(root, &["field"]);
    assert!(first.status.success(), "{}", combined(&first));
    let directory = root.join("templates/suprnova-ui/field");
    fs::remove_file(directory.join(".suprnova-installed.json")).expect("drop the record");
    let view = directory.join("field.html");
    fs::write(&view, "{# a copy from before install records #}\n").expect("hand copy");

    let second = add(root, &["field"]);
    assert!(second.status.success(), "{}", combined(&second));
    assert_eq!(
        fs::read_to_string(&view).expect("view"),
        "{# a copy from before install records #}\n"
    );
    assert!(combined(&second).contains("no install record"), "{}", combined(&second));
}

#[test]
fn ui_022_a_damaged_install_record_is_refused_and_named() {
    let project = project();
    let root = project.path();
    let first = add(root, &["field"]);
    assert!(first.status.success(), "{}", combined(&first));
    let directory = root.join("templates/suprnova-ui/field");
    let view = directory.join("field.html");
    fs::write(&view, "{# edited #}\n").expect("edit the view");
    fs::write(directory.join(".suprnova-installed.json"), "<<<<<<< ours\n").expect("damage");

    let refused = add(root, &["field"]);
    assert!(!refused.status.success(), "{}", combined(&refused));
    let report = combined(&refused);
    assert!(report.contains(".suprnova-installed.json"), "{report}");
    assert_eq!(fs::read_to_string(&view).expect("view"), "{# edited #}\n");
}

#[cfg(unix)]
#[test]
fn ui_023_a_third_party_file_that_is_a_symbolic_link_is_refused() {
    let project = project();
    let root = project.path();
    let manifest = vendor_widget(root, "{% macro widget(name) %}{{ name }}{% endmacro %}\n");
    let secret = root.join("secret.key");
    fs::write(&secret, "not a template\n").expect("secret");
    let script = root.join("vendor/acme-widget/widget.css");
    fs::remove_file(&script).expect("drop the stylesheet");
    std::os::unix::fs::symlink(&secret, &script).expect("link outside the directory");

    let refused = add_manifest(root, &manifest);
    assert!(!refused.status.success(), "{}", combined(&refused));
    assert!(combined(&refused).contains("symbolic link"), "{}", combined(&refused));
    assert!(!root.join("templates/acme-ui").exists(), "nothing installed");

    // A link that stays inside the directory is refused the same way.
    fs::remove_file(&script).expect("drop the link");
    fs::write(root.join("vendor/acme-widget/real.css"), ".acme-widget {}\n").expect("real");
    std::os::unix::fs::symlink(root.join("vendor/acme-widget/real.css"), &script)
        .expect("link inside the directory");
    let refused = add_manifest(root, &manifest);
    assert!(!refused.status.success(), "{}", combined(&refused));
    assert!(!root.join("templates/acme-ui").exists(), "nothing installed");
}
