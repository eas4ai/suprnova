//! `live:add` installs a library component from its manifest into one
//! directory under `templates/`, never overwrites an edited file, replaces an
//! unedited one when the shipped file changes, and accepts a third-party
//! component in the same manifest format, from a library tree whose files are
//! regular files in their directories (Cairn UI-017, UI-022, UI-023).
//!
//! A third-party install runs the scan (REG-022). Where a test reaches the
//! scan it drives the install through the registry's own API with a stand-in
//! for the scan until the scanners land; where a refusal comes first it
//! drives the binary.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use suprnova_cli::registry::RegistryError;
use suprnova_cli::registry::address::{self, LibraryAddress};
use suprnova_cli::registry::fetch::SourceFetcher;
use suprnova_cli::registry::install::{self, RegistrationEdits};
use suprnova_cli::registry::plan::{self, FileOutcome, Options, Scanner};
use suprnova_cli::registry::project::{ProjectFile, ProjectLock};
use suprnova_cli::registry::scan::allowlist::Allowlist;
use suprnova_cli::registry::scan::{self, ComponentFiles, ScanReport};
use suprnova_cli::registry::signing::{self, SecretKey};
use suprnova_cli::registry::statement::{Digest, Statement};

const BIN: &str = env!("CARGO_BIN_EXE_suprnova");
const SEED: [u8; 32] = [5; 32];

fn project() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().expect("tempdir");
    fs::write(
        tmp.path().join("Cargo.toml"),
        "[package]\nname = \"demo-app\"\nversion = \"0.1.0\"\n\n[dependencies]\nsuprnova = { git = \"https://github.com/eas4ai/suprnova.git\", tag = \"v3.2.1\" }\n",
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

/// The path-keyed install record entry for one installed file (REG-028).
fn record_entry(path: &str, bytes: &[u8]) -> (String, serde_json::Value) {
    (
        path.to_owned(),
        serde_json::Value::String(Digest::of(bytes).to_string()),
    )
}

/// Writes a signed library tree with one component, `widget`, under
/// `<project>/vendor/acme-ui`, and returns its manifest's path.
fn vendor_library(root: &Path, namespace: &str, version: &str, view: &str) -> PathBuf {
    let library_root = root.join("vendor/acme-ui");
    let component = library_root.join("components/widget");
    fs::create_dir_all(&component).expect("vendor dir");
    let secret = SecretKey::from_bytes(SEED);
    let library_json = serde_json::to_vec_pretty(&serde_json::json!({
        "namespace": namespace,
        "source": "github.com/acme/acme-ui",
        "version": version,
        "framework": ">=3.0.0",
        "publicKey": secret.public_key().encode(),
    }))
    .expect("library.json");
    let manifest = serde_json::to_vec_pretty(&serde_json::json!({
        "name": format!("{namespace}.widget"),
        "root": format!("{namespace}-ui/widget"),
        "files": ["widget.html", "widget.css"],
    }))
    .expect("manifest");
    let css = b".acme-widget { display: block; }\n";
    fs::write(library_root.join("library.json"), &library_json).expect("library.json");
    fs::write(component.join("manifest.json"), &manifest).expect("manifest");
    fs::write(component.join("widget.html"), view).expect("view");
    fs::write(component.join("widget.css"), css).expect("css");
    let statement = Statement {
        library: "github.com/acme/acme-ui".to_owned(),
        version: semver::Version::parse(version).expect("semver"),
        component: "widget".to_owned(),
        library_json: Digest::of(&library_json),
        manifest: Digest::of(&manifest),
        files: BTreeMap::from([
            ("widget.css".to_owned(), Digest::of(css)),
            ("widget.html".to_owned(), Digest::of(view.as_bytes())),
        ]),
    };
    let signature = signing::sign(&secret, &statement.verification_hash()).expect("sign");
    fs::write(component.join("manifest.sig"), signature.encode()).expect("signature");
    component.join("manifest.json")
}

fn project_with_vendor(namespace: &str) -> tempfile::TempDir {
    let project = project();
    vendor_library(project.path(), namespace, "1.0.0", "<div>Widget</div>\n");
    project
}

/// Pins the vendored library's key by hand, as a developer may (REG-024).
fn pin_vendor(root: &Path) {
    let library = fs::canonicalize(root.join("vendor/acme-ui")).expect("canonical");
    let mut project = ProjectFile::load(root).expect("load");
    project
        .set_library(
            &LibraryAddress(library.to_str().expect("utf-8").to_owned()),
            None,
            Some(&SecretKey::from_bytes(SEED).public_key()),
        )
        .expect("pin");
    project.save().expect("save");
}

/// The real scan with an empty allowlist; until the scanners land the scan
/// is a placeholder, and these clean components stand for themselves.
struct Clean;

impl Scanner for Clean {
    fn scan(&self, component: &ComponentFiles<'_>) -> Result<ScanReport, RegistryError> {
        match scan::scan_component(component, &Allowlist::default()) {
            Err(RegistryError::NotBuilt(_)) => Ok(ScanReport::default()),
            other => other,
        }
    }
}

fn no_registration(
    _: &Path,
    _: &str,
    _: &[String],
    _: &[String],
    _: &[String],
) -> Result<RegistrationEdits, RegistryError> {
    Ok(RegistrationEdits::Write(Vec::new()))
}

/// `live:add --manifest <manifest> --yes` through the registry, with the
/// stand-in scan.
fn add_manifest(
    root: &Path,
    manifest: &Path,
) -> Result<Vec<(PathBuf, FileOutcome)>, RegistryError> {
    let options = Options {
        yes: true,
        ..Options::default()
    };
    let lock = ProjectLock::acquire(root)?;
    let mut project = ProjectFile::load(root)?;
    let source = address::parse(manifest.to_str().expect("utf-8"))?;
    let plan = plan::resolve_with(
        &source,
        &options,
        &SourceFetcher::default(),
        &project,
        &Clean,
    )?;
    let decisions = plan::decisions_from_flags(&plan, &options, &project)?;
    let outcomes = install::apply_with(
        &plan,
        &mut project,
        &lock,
        &options,
        &decisions,
        &no_registration,
    )?;
    lock.release()?;
    Ok(outcomes)
}

fn outcome(outcomes: &[(PathBuf, FileOutcome)], path: &str) -> FileOutcome {
    outcomes
        .iter()
        .find(|(candidate, _)| candidate == Path::new(path))
        .map(|(_, outcome)| *outcome)
        .unwrap_or_else(|| panic!("{path} is not in {outcomes:?}"))
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
    let manifest = vendor_library(
        root,
        "acme",
        "1.0.0",
        "<div class=\"acme-widget\">Widget</div>\n",
    );
    pin_vendor(root);
    add_manifest(root, &manifest).expect("installs");
    let directory = root.join("templates/acme-ui/widget");
    for file in ["manifest.json", "widget.html", "widget.css"] {
        assert!(directory.join(file).is_file(), "{file} installed");
    }

    let reserved = project_with_vendor("suprnova");
    let refused = add(
        reserved.path(),
        &[
            "--manifest",
            "vendor/acme-ui/components/widget/manifest.json",
            "--yes",
        ],
    );
    assert!(!refused.status.success());
    assert!(
        combined(&refused).contains("reserved for the shipped library"),
        "{}",
        combined(&refused)
    );
    assert!(!reserved.path().join("templates/suprnova-ui").exists());
}

#[test]
fn ui_022_an_unedited_older_install_is_replaced_and_an_edited_one_is_kept() {
    let project = project();
    let root = project.path();
    let older = "<div>Widget</div>\n";
    let newer = "<div class=\"acme-widget\">Widget</div>\n";
    let manifest = vendor_library(root, "acme", "1.0.0", older);
    pin_vendor(root);
    add_manifest(root, &manifest).expect("installs");
    let view = root.join("templates/acme-ui/widget/widget.html");
    assert_eq!(fs::read_to_string(&view).expect("view"), older);

    vendor_library(root, "acme", "1.1.0", newer);
    let upgraded = add_manifest(root, &manifest).expect("upgrades");
    assert_eq!(
        fs::read_to_string(&view).expect("view"),
        newer,
        "an unedited file follows the library"
    );
    assert_eq!(
        outcome(&upgraded, "templates/acme-ui/widget/widget.html"),
        FileOutcome::Replaced
    );

    let edited = format!("{newer}<!-- edited by the application -->\n");
    fs::write(&view, &edited).expect("edit the view");
    vendor_library(root, "acme", "1.2.0", older);
    let kept = add_manifest(root, &manifest).expect("keeps the edit");
    assert_eq!(
        fs::read_to_string(&view).expect("view"),
        edited,
        "the edit survives"
    );
    assert_eq!(
        outcome(&kept, "templates/acme-ui/widget/widget.html"),
        FileOutcome::Kept
    );

    // The edit stays known across runs: a second run still keeps it.
    vendor_library(root, "acme", "1.3.0", newer);
    let again = add_manifest(root, &manifest).expect("keeps it again");
    assert_eq!(
        outcome(&again, "templates/acme-ui/widget/widget.html"),
        FileOutcome::Kept
    );
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
    let (key, value) = record_entry("templates/suprnova-ui/field/field.html", older.as_bytes());
    record.insert(key, value);
    fs::write(&record_path, serde_json::to_vec(&record).expect("encode")).expect("record");

    let upgraded = add(root, &["field"]);
    assert!(upgraded.status.success(), "{}", combined(&upgraded));
    assert_eq!(fs::read_to_string(&view).expect("view"), shipped);
    assert!(
        !combined(&upgraded).contains("edited locally"),
        "{}",
        combined(&upgraded)
    );
}

/// The chart stylesheet `live:add chart` installed in 2.1.0 and 3.0.0, before
/// the chart took its colors from the tokens (DATA-007): it has no rule for
/// the classes the rendered SVG now carries, so every mark paints the text
/// color.
const CHART_CSS_BEFORE_DATA_007: &str = "@layer suprnova-ui {
  .sn-chart {
    display: grid;
    gap: var(--sn-space-2);
    margin: 0;
  }

  .sn-chart-title {
    font-weight: var(--sn-font-weight-semibold);
  }

  .sn-chart-marks {
    inline-size: 100%;
    overflow-x: auto;
  }

  .sn-chart-marks svg {
    display: block;
    max-inline-size: 100%;
    block-size: auto;
  }

  .sn-chart-summary {
    margin: 0;
    color: var(--sn-color-text-muted);
    font-size: var(--sn-font-size-sm);
  }
}
";

#[test]
fn data_007_a_chart_vendored_before_the_token_classes_takes_them_from_live_add() {
    let project = project();
    let root = project.path();
    let first = add(root, &["chart"]);
    assert!(first.status.success(), "{}", combined(&first));
    let directory = root.join("templates/suprnova-ui/chart");
    let stylesheet = directory.join("chart.css");

    // The install an earlier release left: the older stylesheet, which its
    // record vouches for because the application never edited it.
    fs::write(&stylesheet, CHART_CSS_BEFORE_DATA_007).expect("older stylesheet");
    let record_path = directory.join(".suprnova-installed.json");
    let mut record: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(&fs::read_to_string(&record_path).expect("install record"))
            .expect("record json");
    let (key, value) = record_entry(
        "templates/suprnova-ui/chart/chart.css",
        CHART_CSS_BEFORE_DATA_007.as_bytes(),
    );
    record.insert(key, value);
    fs::write(&record_path, serde_json::to_vec(&record).expect("encode")).expect("record");

    let upgraded = add(root, &["chart"]);
    let report = combined(&upgraded);
    assert!(upgraded.status.success(), "{report}");
    assert!(
        report.contains("chart.css") && report.contains("replaced"),
        "{report}"
    );
    let css = fs::read_to_string(&stylesheet).expect("stylesheet");
    let mut classes = vec![
        "sn-chart-text".to_owned(),
        "sn-chart-axis-text".to_owned(),
        "sn-chart-axis".to_owned(),
        "sn-chart-grid".to_owned(),
    ];
    classes.extend((1..=6).map(|n| format!("sn-chart-series-{n}")));
    for class in classes {
        assert!(
            css.contains(&format!(".{class} {{")),
            "the re-vendored chart.css has no .{class} rule:\n{css}"
        );
    }
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
    assert!(
        combined(&second).contains("no install record"),
        "{}",
        combined(&second)
    );
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

#[test]
fn ui_022_a_run_that_stops_partway_writes_nothing_and_the_next_run_installs() {
    let project = project();
    let root = project.path();
    let manifest = vendor_library(root, "acme", "1.0.0", "<div>Widget</div>\n");
    pin_vendor(root);
    // The stylesheet's destination is a directory, so the run stops at it
    // before it writes anything, the manifest and the view included.
    let target = root.join("templates/acme-ui/widget");
    fs::create_dir_all(target.join("widget.css")).expect("blocking directory");
    assert!(add_manifest(root, &manifest).is_err());
    assert!(!target.join("widget.html").exists());
    assert!(!target.join("manifest.json").exists());
    fs::remove_dir(target.join("widget.css")).expect("unblock");

    add_manifest(root, &manifest).expect("the next run installs");
    assert_eq!(
        fs::read_to_string(target.join("widget.html")).expect("view"),
        "<div>Widget</div>\n"
    );
}

#[test]
fn ui_023_a_manifest_named_by_a_relative_path_is_read_from_the_working_directory() {
    // A relative `--manifest` path is a path source from the working
    // directory. The library's reserved namespace is refused only once its
    // library.json was found and read there.
    let project = project_with_vendor("sn");
    let output = add(
        project.path(),
        &[
            "--manifest",
            "vendor/acme-ui/components/widget/manifest.json",
            "--yes",
        ],
    );
    assert!(!output.status.success());
    assert!(
        combined(&output).contains("the namespace `sn` is reserved"),
        "{}",
        combined(&output)
    );
}

#[cfg(unix)]
#[test]
fn ui_023_a_third_party_file_that_is_a_symbolic_link_is_refused() {
    let project = project();
    let root = project.path();
    let manifest = vendor_library(root, "acme", "1.0.0", "<div>Widget</div>\n");
    let secret = root.join("secret.key");
    fs::write(&secret, "not a template\n").expect("secret");
    let stylesheet = manifest.with_file_name("widget.css");
    fs::remove_file(&stylesheet).expect("drop the stylesheet");
    std::os::unix::fs::symlink(&secret, &stylesheet).expect("link outside the directory");

    let manifest_arg = manifest.to_str().expect("utf-8");
    let refused = add(root, &["--manifest", manifest_arg, "--yes"]);
    assert!(!refused.status.success(), "{}", combined(&refused));
    assert!(
        combined(&refused).contains("symbolic link"),
        "{}",
        combined(&refused)
    );
    assert!(
        !root.join("templates/acme-ui").exists(),
        "nothing installed"
    );

    // A link that stays inside the directory is refused the same way.
    fs::remove_file(&stylesheet).expect("drop the link");
    let real = manifest.with_file_name("real.css");
    fs::write(&real, ".acme-widget {}\n").expect("real");
    std::os::unix::fs::symlink(&real, &stylesheet).expect("link inside the directory");
    let refused = add(root, &["--manifest", manifest_arg, "--yes"]);
    assert!(!refused.status.success(), "{}", combined(&refused));
    assert!(
        combined(&refused).contains("symbolic link"),
        "{}",
        combined(&refused)
    );
    assert!(
        !root.join("templates/acme-ui").exists(),
        "nothing installed"
    );
}
