//! `registries-scan`: the Rust, view and script scans refuse every item of
//! the bypass corpus under `tests/fixtures/registry/bypass/` with the check,
//! file and line named, and accept every shipped component (REG-016,
//! REG-022, REG-030, REG-031, REG-032).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use suprnova_cli::registry::Capability;
use suprnova_cli::registry::address::LibraryAddress;
use suprnova_cli::registry::project::ProjectFile;
use suprnova_cli::registry::scan::allowlist::{self, Admission, AllowedItem, Allowlist};
use suprnova_cli::registry::scan::{
    ComponentFiles, ScanReport, scan_component, scan_component_with_manifest,
};
use suprnova_cli::registry::signing::{self, SecretKey};
use suprnova_cli::registry::statement::{Digest, Statement};

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/registry/bypass")
}

/// Reads one corpus component's files by manifest name.
fn files_of(component: &str) -> Vec<(String, Vec<u8>)> {
    let dir = corpus().join(component);
    let mut files: Vec<(String, Vec<u8>)> = fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("{}: {error}", dir.display()))
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file() && path.file_name().is_some_and(|name| name != "manifest.json")
        })
        .map(|path| {
            let name = path
                .file_name()
                .expect("name")
                .to_string_lossy()
                .into_owned();
            (name, fs::read(&path).expect("read"))
        })
        .collect();
    files.sort();
    files
}

/// An allowlist with no items: nothing from `suprnova::` is admitted, so a
/// refusal of a `std` path cannot come from a missing allowlist entry.
fn empty_allowlist() -> Allowlist {
    Allowlist::from_items(std::collections::BTreeMap::<String, AllowedItem>::new())
}

/// REG-030: `std::fs` under any alias is refused, and the refusal names the
/// check, the file and the line.
#[test]
fn reg_030_std_fs_under_an_alias_is_refused_naming_the_file_and_line() {
    let files = files_of("rust-std-fs-alias");
    let component = ComponentFiles {
        namespace: "evil",
        directory: "rust-std-fs-alias",
        files: &files,
        dependency_modules: &[],
        importable_views: &[],
        importable_scripts: &[],
    };
    let report = scan_component(&component, &empty_allowlist()).expect("the scan runs");
    assert!(!report.accepted(), "the component was admitted: {report:?}");
    let finding = report
        .findings
        .iter()
        .find(|finding| finding.file == "widget.rs" && finding.message.contains("std::fs"))
        .unwrap_or_else(|| {
            panic!(
                "no finding names std::fs in widget.rs: {:?}",
                report.findings
            )
        });
    assert!(finding.check.starts_with("rust-"), "{finding}");
    assert!(finding.message.contains("std::fs"), "{finding}");
    assert_eq!(finding.line, Some(3), "{finding}");
}

/// The workspace root, two levels above this crate's manifest.
fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the CLI sits in the workspace")
        .to_path_buf()
}

/// REG-030: the embedded allowlist is exactly what the generator produces
/// from the current feature map, so an API change cannot leave a stale
/// capability behind.
#[test]
fn reg_030_the_allowlist_matches_a_fresh_generation_from_the_feature_map() {
    let output = std::process::Command::new("python3")
        .arg(workspace().join("feature-map/tools/registry_allowlist.py"))
        .arg("--stdout")
        .output()
        .expect("python3 runs the generator");
    assert!(
        output.status.success(),
        "the generator failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let fresh = String::from_utf8(output.stdout).expect("the generator writes UTF-8");
    assert!(
        fresh == allowlist::embedded_text(),
        "suprnova-cli/src/registry/scan/allowlist.jsonl is out of date; run feature-map/tools/registry_allowlist.py"
    );
}

fn capability_of(list: &Allowlist, path: &str) -> Option<Capability> {
    match list.admit(path) {
        Some(Admission::Item { item, .. }) | Some(Admission::Prefix { item, .. }) => {
            assert!(!item.hidden, "{path} is hidden");
            item.capability
        }
        other => panic!("{path} is not admitted as an item: {other:?}"),
    }
}

/// REG-006, REG-030: the embedded allowlist carries a capability for each
/// effect, re-exported crates included, refuses hidden re-exports and
/// leaves out what it cannot decide.
#[test]
fn reg_030_the_embedded_allowlist_carries_each_items_capability() {
    let list = allowlist::embedded().expect("the embedded allowlist parses");
    assert!(list.len() > 1000, "only {} items", list.len());
    assert_eq!(
        capability_of(list, "suprnova::Storage"),
        Some(Capability::Files)
    );
    assert_eq!(
        capability_of(list, "suprnova::filesystem::Storage"),
        Some(Capability::Files)
    );
    assert_eq!(
        capability_of(list, "suprnova::tokio::fs::read_to_string"),
        Some(Capability::Files)
    );
    assert_eq!(
        capability_of(list, "suprnova::opendal::Operator"),
        Some(Capability::Files)
    );
    assert_eq!(capability_of(list, "suprnova::serde::Serialize"), None);
    assert_eq!(
        capability_of(list, "suprnova::csrf_token"),
        Some(Capability::Session)
    );
    assert_eq!(capability_of(list, "suprnova::route"), None);
    assert_eq!(capability_of(list, "suprnova::url::to"), None);
    match list.admit("suprnova::inventory::submit") {
        Some(Admission::Prefix { item, .. }) => assert!(item.hidden),
        other => panic!("inventory is not a hidden re-export: {other:?}"),
    }
    assert!(list.admit("suprnova::tokio::runtime::Runtime").is_none());
    assert!(matches!(
        list.admit("suprnova::PasswordReset"),
        Some(Admission::Refused { .. })
    ));
    assert!(list.admit("suprnova::NoSuchItem").is_none());
    // Sibling-crate re-exports count through their `suprnova::` paths: the
    // chart renderer is admitted, and so is the TrustedHtml type, whose
    // constructors the Rust scan refuses on its own.
    assert_eq!(
        capability_of(list, "suprnova::live::charts::render_chart"),
        None
    );
    assert_eq!(capability_of(list, "suprnova::view::TrustedHtml"), None);
    assert_eq!(
        capability_of(list, "suprnova::view::filters::live_key"),
        None
    );
    assert_eq!(
        capability_of(list, "suprnova::live::UploadScan::Disabled"),
        None
    );
    assert!(matches!(
        list.admit("suprnova::live::LiveRegistry"),
        Some(Admission::Item { .. })
    ));
}

/// The accepted fixtures: components the scans must admit.
fn accepted() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/registry/accepted")
}

/// A fixture manifest's string list under `key`.
fn manifest_list(dir: &Path, key: &str) -> Vec<String> {
    let manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(dir.join("manifest.json"))
            .unwrap_or_else(|error| panic!("{}: {error}", dir.display())),
    )
    .expect("the fixture manifest is JSON");
    manifest[key]
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .map(|entry| entry.as_str().expect("a string entry").to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// A fixture component: its files by manifest name, its namespace and its
/// manifest's `register`.
type Fixture = (Vec<(String, Vec<u8>)>, String, Vec<String>);

/// Reads a fixture component's files, its namespace and `register`.
fn fixture(dir: &Path) -> Fixture {
    let manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(dir.join("manifest.json"))
            .unwrap_or_else(|error| panic!("{}: {error}", dir.display())),
    )
    .expect("the fixture manifest is JSON");
    let name = manifest["name"].as_str().expect("the manifest has a name");
    let namespace = name.split('.').next().expect("a namespace").to_string();
    let register = manifest["register"]
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .map(|entry| entry.as_str().expect("a register entry").to_string())
                .collect()
        })
        .unwrap_or_default();
    let mut files = Vec::new();
    for entry in manifest["files"]
        .as_array()
        .expect("the manifest names files")
    {
        let file = entry.as_str().expect("a file name");
        files.push((
            file.to_string(),
            fs::read(dir.join(file)).expect("a named file"),
        ));
    }
    (files, namespace, register)
}

fn scan_fixture(dir: &Path) -> ScanReport {
    let (files, namespace, register) = fixture(dir);
    let directory = dir
        .file_name()
        .expect("a directory name")
        .to_string_lossy()
        .into_owned();
    let component = ComponentFiles {
        namespace: &namespace,
        directory: &directory,
        files: &files,
        dependency_modules: &[],
        importable_views: &[],
        importable_scripts: &[],
    };
    let elements = manifest_list(dir, "elements");
    scan_component_with_manifest(
        &component,
        &register,
        &elements,
        allowlist::embedded().expect("the embedded allowlist parses"),
    )
    .expect("the scan runs")
}

/// Each `refused: <check>` marker in a fixture: the check, the file and the
/// line it sits on.
fn markers(dir: &Path) -> Vec<(String, String, u32)> {
    let (files, _, _) = fixture(dir);
    let mut found = Vec::new();
    for (name, bytes) in files {
        let text = String::from_utf8_lossy(&bytes);
        for (index, line) in text.lines().enumerate() {
            if let Some(rest) = line.split("refused: ").nth(1) {
                let check: String = rest
                    .chars()
                    .take_while(|c| c.is_ascii_lowercase() || *c == '-')
                    .collect();
                found.push((
                    check,
                    name.clone(),
                    u32::try_from(index + 1).expect("a line number"),
                ));
            }
        }
    }
    found
}

/// REG-032: a static import resolves only to the component's own scripts
/// or to a script a dependency's manifest names; a file in a dependency's
/// directory that its manifest does not name, and a shipped script the
/// component does not depend on, are refused, though each is a relative
/// path.
#[test]
fn reg_032_a_static_import_may_name_only_the_scripts_its_dependencies_carry() {
    let scan = |script: &str| {
        let files = vec![
            ("widget.html".to_string(), b"<div>x</div>".to_vec()),
            ("widget.js".to_string(), script.as_bytes().to_vec()),
        ];
        let importable_scripts = vec!["other-ui/thing/thing.js".to_string()];
        let component = ComponentFiles {
            namespace: "acme",
            directory: "widget",
            files: &files,
            dependency_modules: &[],
            importable_views: &[
                "suprnova-ui/combobox/combobox.html".to_string(),
                "other-ui/thing/thing.html".to_string(),
            ],
            importable_scripts: &importable_scripts,
        };
        scan_component(&component, allowlist::embedded().expect("allowlist")).expect("scan")
    };
    let named = scan("import \"../../other-ui/thing/thing.js\";\n");
    assert!(named.accepted(), "{:?}", named.findings);
    let unnamed = scan("import \"../../other-ui/thing/extra.js\";\n");
    assert!(
        unnamed.findings.iter().any(|f| f.check == "script-import"),
        "{:?}",
        unnamed.findings
    );
    let shipped = scan("import \"../../suprnova-ui/combobox/combobox.js\";\n");
    assert!(
        shipped.findings.iter().any(|f| f.check == "script-import"),
        "{:?}",
        shipped.findings
    );
}

/// REG-032: a static import is a relative path (`./` or `../`) into the
/// component or a component it depends on. An absolute path breaks under a
/// path prefix (PFX-006), so it is refused even when it names the
/// component's own script, and so is a URL; the refusal says what the
/// specifier must be.
#[test]
fn reg_032_a_static_import_is_a_relative_path_into_the_component_or_a_dependency() {
    let scan = |script: &str| {
        let files = vec![
            ("x.html".to_string(), b"<div>x</div>".to_vec()),
            ("x.js".to_string(), b"export const value = 1;\n".to_vec()),
            ("main.js".to_string(), script.as_bytes().to_vec()),
        ];
        let importable_scripts = vec!["acme-ui/y/y.js".to_string()];
        let component = ComponentFiles {
            namespace: "acme",
            directory: "x",
            files: &files,
            dependency_modules: &[],
            importable_views: &[],
            importable_scripts: &importable_scripts,
        };
        scan_component(&component, allowlist::embedded().expect("allowlist")).expect("scan")
    };
    let own = scan("import \"./x.js\";\n");
    assert!(own.accepted(), "{:?}", own.findings);
    let dependency = scan("import \"../y/y.js\";\n");
    assert!(dependency.accepted(), "{:?}", dependency.findings);
    for specifier in [
        "/acme-ui/x/x.js",
        "/acme-ui/y/y.js",
        "https://cdn.example/x.js",
        "//cdn.example/x.js",
    ] {
        let report = scan(&format!("import \"{specifier}\";\n"));
        let refusal = report
            .findings
            .iter()
            .find(|finding| finding.check == "script-import")
            .unwrap_or_else(|| panic!("`{specifier}` was admitted: {:?}", report.findings));
        assert_eq!(refusal.file, "main.js", "{specifier}");
        assert_eq!(refusal.line, Some(1), "{specifier}");
        assert!(
            refusal
                .message
                .contains("must be a relative path (`./` or `../`)"),
            "`{specifier}`: {}",
            refusal.message
        );
    }
}

/// REG-031: a view calls nothing the scan cannot show carries no
/// capability, so a call into a dependency's Rust, whose functions this
/// scan does not read, is refused.
#[test]
fn reg_031_a_view_may_not_call_a_dependencys_function() {
    let files = vec![(
        "widget.html".to_string(),
        b"<p>{{ crate::live::evil::helper::token() }}</p>\n".to_vec(),
    )];
    let dependency_modules = vec!["evil::helper".to_string()];
    let component = ComponentFiles {
        namespace: "evil",
        directory: "widget",
        files: &files,
        dependency_modules: &dependency_modules,
        importable_views: &[],
        importable_scripts: &[],
    };
    let report =
        scan_component(&component, allowlist::embedded().expect("allowlist")).expect("scan");
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.check == "view-call" && finding.line == Some(1)),
        "{:?}",
        report.findings
    );
}

/// REG-006: every public API that reaches one of the nine effects carries
/// its capability, including methods that live on an otherwise effect-free
/// type (a redirect that writes the session, a response that opens a file),
/// and an API that rewrites the application for every request is refused
/// like a container binding.
#[test]
fn reg_006_effectful_methods_on_effect_free_types_carry_their_capability() {
    let list = allowlist::embedded().expect("the embedded allowlist parses");
    let carries = [
        ("suprnova::Redirect::set_intended_url", Capability::Session),
        ("suprnova::Redirect::back", Capability::Session),
        ("suprnova::Redirect::with", Capability::Session),
        (
            "suprnova::HttpResponse::without_cookie",
            Capability::Session,
        ),
        ("suprnova::url::previous", Capability::Session),
        ("suprnova::live::LiveDocument::mount", Capability::Session),
        ("suprnova::HttpResponse::download", Capability::Files),
        ("suprnova::HttpResponse::file", Capability::Files),
        (
            "suprnova::Router::try_live_ui_assets_for",
            Capability::Files,
        ),
        ("suprnova::hash", Capability::Environment),
        ("suprnova::url::signed_route", Capability::Environment),
        ("suprnova::InertiaConfig::new", Capability::Environment),
        ("suprnova::InertiaConfig", Capability::Environment),
        ("suprnova::Password::uncompromised", Capability::Network),
        (
            "suprnova::live::verify_ledger_backend",
            Capability::Database,
        ),
    ];
    let mut wrong = Vec::new();
    for (path, capability) in carries {
        let found = match list.admit(path) {
            Some(Admission::Item { item, .. }) | Some(Admission::Prefix { item, .. }) => {
                item.capability
            }
            _ => match list.member(
                path.rsplit_once("::").expect("a member path").0,
                path.rsplit_once("::").expect("a member path").1,
            ) {
                Some(Admission::Item { item, .. }) | Some(Admission::Prefix { item, .. }) => {
                    item.capability
                }
                _ => None,
            },
        };
        if found != Some(capability) {
            wrong.push(format!("{path}: {found:?}, not {capability:?}"));
        }
    }
    for path in [
        "suprnova::register_global_middleware",
        "suprnova::prepend_global_middleware",
        "suprnova::register_middleware_group",
        "suprnova::register_terminable",
        "suprnova::data::registry::register",
        "suprnova::InertiaRegistry::share_value",
    ] {
        if !matches!(list.admit(path), Some(Admission::Refused { .. })) {
            wrong.push(format!("{path} is admitted"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// REG-022, REG-030, REG-031, REG-032: every component of the bypass corpus
/// is refused, and each refusal its fixture marks is reported with that
/// check, file and line.
#[test]
fn reg_022_every_bypass_in_the_corpus_is_refused_with_its_check_file_and_line() {
    let mut dirs: Vec<PathBuf> = fs::read_dir(corpus())
        .expect("the corpus exists")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    dirs.sort();
    assert!(
        dirs.len() >= 60,
        "the corpus holds only {} components",
        dirs.len()
    );
    let mut failures = Vec::new();
    for dir in &dirs {
        let name = dir
            .file_name()
            .expect("a name")
            .to_string_lossy()
            .into_owned();
        let report = scan_fixture(dir);
        if report.accepted() {
            failures.push(format!("{name}: admitted"));
            continue;
        }
        let expected = markers(dir);
        if expected.is_empty() {
            failures.push(format!("{name}: the fixture marks no refusal"));
        }
        for (check, file, line) in expected {
            let hit = report.findings.iter().any(|finding| {
                finding.check == check && finding.file == file && finding.line == Some(line)
            });
            if !hit {
                failures.push(format!(
                    "{name}: no `{check}` finding at {file}:{line}; got {}",
                    report
                        .findings
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(" | ")
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// REG-006, REG-030: well-formed components are admitted, and each reports
/// exactly the capabilities its Rust reaches.
#[test]
fn reg_030_accepted_components_pass_and_report_their_capabilities() {
    let cases: &[(&str, &[Capability], &[&str])] = &[
        ("counter", &[], &["counter::Counter"]),
        ("tally", &[], &["tally::Tally"]),
        ("notes", &[Capability::Files], &["notes::Notes"]),
        (
            "inventory",
            &[Capability::Database],
            &["inventory::Inventory"],
        ),
        ("disclosure", &[], &[]),
        (
            "feed",
            &[Capability::Files, Capability::Network],
            &["feed::Feed"],
        ),
    ];
    for (name, capabilities, defined) in cases {
        let report = scan_fixture(&accepted().join(name));
        assert!(
            report.accepted(),
            "{name} was refused: {}",
            report
                .findings
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" | ")
        );
        let expected: std::collections::BTreeSet<Capability> =
            capabilities.iter().copied().collect();
        assert_eq!(report.capabilities, expected, "{name}");
        assert_eq!(report.defined_components, *defined, "{name}");
    }
}

/// The shipped library, embedded in the CLI from the Live crate.
fn shipped() -> PathBuf {
    workspace().join("crates/suprnova-live/components")
}

/// REG-016: every shipped component's views, stylesheets and scripts pass
/// the view and script scans, as a third-party library's would.
#[test]
fn reg_016_every_shipped_component_passes_the_view_and_script_scans() {
    let mut dirs: Vec<PathBuf> = fs::read_dir(shipped())
        .expect("the shipped library exists")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.join("manifest.json").is_file())
        .collect();
    dirs.sort();
    assert!(dirs.len() >= 50, "only {} shipped components", dirs.len());
    let mut importable = Vec::new();
    for dir in &dirs {
        let (files, _, _) = fixture(dir);
        let name = dir
            .file_name()
            .expect("a name")
            .to_string_lossy()
            .into_owned();
        for (file, _) in &files {
            if file.ends_with(".html") {
                importable.push(format!("suprnova-ui/{name}/{file}"));
            }
        }
    }
    let mut failures = Vec::new();
    for dir in &dirs {
        let (files, namespace, _) = fixture(dir);
        assert_eq!(namespace, "suprnova");
        assert!(
            files.iter().all(|(file, _)| !file.ends_with(".rs")),
            "a shipped component carries Rust"
        );
        let directory = dir
            .file_name()
            .expect("a name")
            .to_string_lossy()
            .into_owned();
        let component = ComponentFiles {
            namespace: &namespace,
            directory: &directory,
            files: &files,
            dependency_modules: &[],
            importable_views: &importable,
            importable_scripts: &[],
        };
        let elements = manifest_list(dir, "elements");
        let report = scan_component_with_manifest(
            &component,
            &[],
            &elements,
            allowlist::embedded().expect("the allowlist parses"),
        )
        .expect("the scan runs");
        for finding in &report.findings {
            failures.push(format!("{directory}: {finding}"));
        }
        assert!(
            report.capabilities.is_empty(),
            "{directory} reports {:?}",
            report.capabilities
        );
    }
    assert!(
        failures.is_empty(),
        "{} findings:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// REG-031: the framework filters the view scan admits are exactly the ones
/// `suprnova::view::filters` re-exports.
#[test]
fn reg_031_the_admitted_framework_filters_match_the_framework() {
    let source =
        fs::read_to_string(workspace().join("framework/src/view/mod.rs")).expect("the view module");
    let start = source.find("pub mod filters {").expect("a filters module");
    let end = start + source[start..].find('}').expect("its end");
    let mut exported: Vec<&str> = source[start..end]
        .lines()
        .filter_map(|line| {
            line.trim()
                .strip_prefix("pub use suprnova_live::view::filters::")
        })
        .filter_map(|rest| rest.strip_suffix(';'))
        .collect();
    exported.sort_unstable();
    let mut admitted = suprnova_cli::registry::scan::view::FRAMEWORK_FILTERS.to_vec();
    admitted.sort_unstable();
    assert_eq!(admitted, exported);
}

/// REG-022: a file built to make a parser recurse past the scan's stack is
/// refused with a finding, and the scan returns instead of crashing.
#[test]
fn reg_022_inputs_built_to_exhaust_a_parsers_stack_are_refused() {
    let cases: Vec<(&str, String, &str)> = vec![
        (
            "deep.rs",
            format!(
                "fn f() {{ let x = {}1{}; }}",
                "(".repeat(5000),
                ")".repeat(5000)
            ),
            "rust-limit",
        ),
        (
            "unary.rs",
            format!("fn f() {{ let x = {}true; }}", "!".repeat(200_000)),
            "rust-limit",
        ),
        (
            "assign.rs",
            format!("fn f() {{ {}1; }}", "x = ".repeat(100_000)),
            "rust-limit",
        ),
        (
            "closures.rs",
            format!("fn f() {{ let x = {}1; }}", "|| ".repeat(100_000)),
            "rust-limit",
        ),
        (
            "generic.rs",
            format!(
                "type T = {}u8{};",
                "Vec<".repeat(100_000),
                ">".repeat(100_000)
            ),
            "rust-limit",
        ),
        (
            "ifelse.rs",
            format!("fn f() {{ {}{{}} }}", "if x {} else ".repeat(100_000)),
            "rust-limit",
        ),
        (
            "deep.js",
            format!("const x = {}1{};", "(".repeat(5000), ")".repeat(5000)),
            "script-limit",
        ),
        (
            "unary.js",
            format!("const x = {}1;", "!".repeat(200_000)),
            "script-limit",
        ),
        (
            "ternary.js",
            format!("const x = {}1;", "a ? b : ".repeat(100_000)),
            "script-limit",
        ),
        (
            "arrows.js",
            format!("const x = {}1;", "a => ".repeat(100_000)),
            "script-limit",
        ),
        (
            "new.js",
            format!("const x = {}X;", "new ".repeat(100_000)),
            "script-limit",
        ),
        (
            "ifelse.js",
            format!("{}{{}}", "if (x) {} else ".repeat(100_000)),
            "script-limit",
        ),
        (
            "expr.html",
            format!("{{{{ {}1{} }}}}", "(".repeat(5000), ")".repeat(5000)),
            "view-parse",
        ),
        (
            "char-literal.rs",
            format!(
                "fn f() {{ let _c = '\"'; let x = {}1; }}",
                "(".repeat(1_000_000)
            ),
            "rust-limit",
        ),
        (
            "raw-string.rs",
            format!(
                "fn f() {{ let _s = r#\"a\"b\"#; let x = {}1; }}",
                "(".repeat(1_000_000)
            ),
            "rust-limit",
        ),
        (
            "nested-comment.rs",
            format!(
                "fn f() {{ /* /* */ \" */ let x = {}1; }}",
                "(".repeat(1_000_000)
            ),
            "rust-limit",
        ),
        (
            "template.js",
            format!(
                "const t = `${{1}}\"`; const x = {}1;",
                "(".repeat(1_000_000)
            ),
            "script-limit",
        ),
        (
            "regex.js",
            format!("const r = /\"/; const x = {}1;", "(".repeat(1_000_000)),
            "script-limit",
        ),
        (
            "separator.js",
            format!("// a comment\u{2028}const x = {}1;", "(".repeat(1_000_000)),
            "script-limit",
        ),
        (
            "deep.css",
            format!(
                ".a {{ b: {}1{}; }}",
                "(".repeat(100_000),
                ")".repeat(100_000)
            ),
            "view-css",
        ),
    ];
    for (name, text, check) in cases {
        let files = vec![(name.to_string(), text.into_bytes())];
        let component = ComponentFiles {
            namespace: "evil",
            directory: "widget",
            files: &files,
            dependency_modules: &[],
            importable_views: &[],
            importable_scripts: &[],
        };
        let report = scan_component(
            &component,
            allowlist::embedded().expect("the allowlist parses"),
        )
        .expect("the scan returns");
        assert!(
            report
                .findings
                .iter()
                .any(|finding| finding.check == check && finding.file == name),
            "{name}: {:?}",
            report.findings
        );
    }
}

/// REG-022: a file of a type no scan reads is refused, so nothing the scan
/// did not read is installed.
#[test]
fn reg_022_a_file_no_scan_reads_is_refused() {
    let files = vec![
        ("widget.html".to_string(), b"<p>x</p>".to_vec()),
        ("widget.wasm".to_string(), vec![0, 97, 115, 109]),
    ];
    let component = ComponentFiles {
        namespace: "evil",
        directory: "widget",
        files: &files,
        dependency_modules: &[],
        importable_views: &[],
        importable_scripts: &[],
    };
    let report = scan_component(
        &component,
        allowlist::embedded().expect("the allowlist parses"),
    )
    .expect("the scan runs");
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.check == "scan-file" && finding.file == "widget.wasm"),
        "{:?}",
        report.findings
    );
}

/// REG-030: a component may name the modules of the components it depends
/// on, listed relative to `crate::live`, and nothing else under it.
#[test]
fn reg_030_dependency_modules_are_admitted_and_other_components_are_not() {
    let rust = b"//! Uses a dependency.\n\
use crate::live::acme::counter::Counter;\n\
use crate::live::acme::secret::Key;\n"
        .to_vec();
    let files = vec![("widget.rs".to_string(), rust)];
    let dependencies = vec!["acme::counter".to_string()];
    let component = ComponentFiles {
        namespace: "acme",
        directory: "widget",
        files: &files,
        dependency_modules: &dependencies,
        importable_views: &[],
        importable_scripts: &[],
    };
    let report = scan_component(
        &component,
        allowlist::embedded().expect("the allowlist parses"),
    )
    .expect("the scan runs");
    let lines: Vec<Option<u32>> = report.findings.iter().map(|finding| finding.line).collect();
    assert_eq!(lines, vec![Some(3)], "{:?}", report.findings);
    assert!(
        report.findings[0]
            .message
            .contains("crate::live::acme::secret")
    );
}

/// REG-016: every shipped component passes the view and script scans
/// (REG-031, REG-032) a third-party component must pass, with the embedded
/// allowlist, its views free to import any shipped view. The shipped
/// library is exempt from the signature and the hash, never from the scan.
#[test]
fn reg_016_every_shipped_component_passes_the_scan() {
    use suprnova_cli::registry::fetch::COMPONENTS;
    use suprnova_cli::registry::library::{FileKind, parse_shipped_manifest};
    use suprnova_cli::registry::scan::allowlist;

    let allowlist = allowlist::embedded().expect("the embedded allowlist");
    let shipped_views: Vec<String> = COMPONENTS
        .iter()
        .flat_map(|component| {
            component
                .files
                .iter()
                .filter(|(name, _)| FileKind::of(name) == Some(FileKind::View))
                .map(|(name, _)| format!("suprnova-ui/{}/{name}", component.directory))
        })
        .collect();
    assert!(
        COMPONENTS.len() > 50,
        "only {} components",
        COMPONENTS.len()
    );
    let mut refused = Vec::new();
    for component in COMPONENTS {
        let manifest = parse_shipped_manifest(component.manifest.as_bytes(), component.directory)
            .unwrap_or_else(|error| panic!("{}: {error}", component.directory));
        let files: Vec<(String, Vec<u8>)> = manifest
            .files
            .iter()
            .map(|name| {
                let bytes = component
                    .files
                    .iter()
                    .find(|(file, _)| file == name)
                    .map(|(_, bytes)| bytes.as_bytes().to_vec())
                    .unwrap_or_else(|| panic!("{} names {name}", component.directory));
                (name.clone(), bytes)
            })
            .collect();
        let report = scan_component(
            &ComponentFiles {
                namespace: "suprnova",
                directory: component.directory,
                files: &files,
                dependency_modules: &[],
                importable_views: &shipped_views,
                importable_scripts: &[],
            },
            allowlist,
        )
        .unwrap_or_else(|error| panic!("{}: the scan did not run: {error}", component.directory));
        refused.extend(
            report
                .findings
                .iter()
                .map(|finding| format!("{}: {finding}", component.directory)),
        );
    }
    assert!(
        refused.is_empty(),
        "shipped components fail the scan:\n{}",
        refused.join("\n")
    );
}

/// REG-018, REG-022: the example component `live:registry new` scaffolds
/// passes every scan, `live:check`'s view checks included, against the
/// contract read from its Rust.
#[test]
fn reg_022_the_scaffolded_example_component_passes_the_scan_and_the_view_checks() {
    for namespace in ["acme", "acme-ui-kit"] {
        let rendered = suprnova_cli::registry::scaffold::example_component(namespace);
        let manifest: serde_json::Value = serde_json::from_str(
            &rendered
                .iter()
                .find(|(name, _)| *name == "manifest.json")
                .expect("a manifest")
                .1,
        )
        .expect("the manifest is JSON");
        let files: Vec<(String, Vec<u8>)> = rendered
            .iter()
            .filter(|(name, _)| *name != "manifest.json")
            .map(|(name, text)| ((*name).to_string(), text.clone().into_bytes()))
            .collect();
        let strings = |key: &str| -> Vec<String> {
            manifest[key]
                .as_array()
                .map(|entries| {
                    entries
                        .iter()
                        .filter_map(|entry| entry.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default()
        };
        let register = strings("register");
        let elements = strings("elements");
        let component = ComponentFiles {
            namespace,
            directory: "counter",
            files: &files,
            dependency_modules: &[],
            importable_views: &[],
            importable_scripts: &[],
        };
        let report = scan_component_with_manifest(
            &component,
            &register,
            &elements,
            allowlist::embedded().expect("the allowlist parses"),
        )
        .expect("the scan runs");
        assert!(
            report.accepted(),
            "{namespace}: {}",
            report
                .findings
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" | ")
        );
        assert_eq!(
            report.defined_components,
            vec!["counter::Counter".to_string()]
        );
    }
}

/// One allowlist line, as the generator writes it.
fn allowlist_line(
    path: &str,
    kind: &str,
    capability: Option<&str>,
    returns: Option<&str>,
) -> String {
    let mut line = serde_json::json!({
        "path": path,
        "kind": kind,
        "capability": capability,
        "hidden": false,
        "prefix": false,
        "refused": false,
        "aliases": [],
        "implements": [],
    });
    if let Some(returns) = returns {
        line["returns"] = serde_json::Value::String(returns.to_string());
    }
    line.to_string()
}

/// REG-030: with the return types the feature map records, a method chain
/// on a value a Suprnova function returns is classified by the method it
/// reaches on that type, with that method's capability, and a method the
/// type does not have is still refused.
#[test]
fn reg_030_a_chain_on_a_suprnova_return_value_is_typed_from_the_recorded_return_types() {
    let lines = [
        allowlist_line("suprnova::live::LiveComponent", "proc macro", None, None),
        allowlist_line("suprnova::live::live", "proc macro", None, None),
        allowlist_line("suprnova::live::UploadPolicy", "struct", None, None),
        allowlist_line(
            "suprnova::live::UploadPolicy::builder",
            "fn",
            None,
            Some("suprnova::live::UploadPolicyBuilder"),
        ),
        allowlist_line("suprnova::live::UploadPolicyBuilder", "struct", None, None),
        allowlist_line(
            "suprnova::live::UploadPolicyBuilder::maximum_files",
            "fn",
            None,
            Some("suprnova::live::UploadPolicyBuilder"),
        ),
        allowlist_line(
            "suprnova::live::UploadPolicyBuilder::build",
            "fn",
            None,
            Some("suprnova::live::UploadPolicy"),
        ),
        allowlist_line("suprnova::Storage", "struct", Some("files"), None),
        allowlist_line(
            "suprnova::Storage::disk",
            "fn",
            Some("files"),
            Some("core::result::Result<suprnova::Disk,suprnova::FrameworkError>"),
        ),
        allowlist_line("suprnova::Disk", "struct", Some("files"), None),
        allowlist_line("suprnova::Disk::delete", "fn", Some("files"), Some("()")),
    ];
    let list = Allowlist::parse(&lines.join("\n")).expect("the test allowlist parses");
    let scan = |body: &str| {
        let rust = format!(
            "//! A chain.\nuse suprnova::live::UploadPolicy;\n\n/// The chain.\npub fn chain() {{\n    {body}\n}}\n"
        );
        let files = vec![("widget.rs".to_string(), rust.into_bytes())];
        let component = ComponentFiles {
            namespace: "acme",
            directory: "widget",
            files: &files,
            dependency_modules: &[],
            importable_views: &[],
            importable_scripts: &[],
        };
        scan_component(&component, &list).expect("the scan runs")
    };
    let builder = scan("let _policy = UploadPolicy::builder().maximum_files(1).build();");
    assert!(builder.accepted(), "{:?}", builder.findings);
    assert!(builder.capabilities.is_empty());
    let disk = scan("let _ = suprnova::Storage::disk(\"public\").map(|disk| disk.delete(\"x\"));");
    assert!(disk.accepted(), "{:?}", disk.findings);
    assert_eq!(
        disk.capabilities,
        std::iter::once(Capability::Files).collect()
    );
    let unknown = scan("let _ = UploadPolicy::builder().evil();");
    assert!(
        unknown
            .findings
            .iter()
            .any(|finding| finding.check == "rust-method" && finding.message.contains("evil")),
        "{:?}",
        unknown.findings
    );
}

/// The feature map extractor's own tests: the return types the chain
/// typing above reads.
#[test]
fn reg_030_the_feature_map_extractor_records_return_types() {
    let output = std::process::Command::new("python3")
        .arg("-B")
        .arg(workspace().join("feature-map/tools/test_rust_api.py"))
        .output()
        .expect("python3 runs the extractor tests");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Scans a component whose only file is the view `html`, against the
/// embedded allowlist.
fn scan_view(html: &str) -> ScanReport {
    let files = vec![("widget.html".to_string(), html.as_bytes().to_vec())];
    let component = ComponentFiles {
        namespace: "acme",
        directory: "widget",
        files: &files,
        dependency_modules: &[],
        importable_views: &[],
        importable_scripts: &[],
    };
    scan_component(&component, allowlist::embedded().expect("allowlist")).expect("scan")
}

/// REG-031, PFX-011: `suprnova::url::root()` is Suprnova API that carries no
/// capability.
#[test]
fn reg_031_url_root_is_admitted_and_carries_no_capability() {
    let list = allowlist::embedded().expect("the embedded allowlist parses");
    assert_eq!(capability_of(list, "suprnova::url::root"), None);
}

/// REG-031: a URL attribute may hold `suprnova::url::root()` followed
/// directly by constant text that starts with exactly one `/`.
#[test]
fn reg_031_url_root_before_a_rooted_constant_path_is_admitted() {
    for view in [
        r#"<a href="{{ suprnova::url::root() }}/x">x</a>"#,
        r#"<a href="{{ suprnova::url::root() }}/">home</a>"#,
        r#"<img src="{{ suprnova::url::root() }}/suprnova-ui/x/x.png" alt="">"#,
        r#"<a href="{{ suprnova::url::root() }}/posts/{{ id }}">post</a>"#,
        r#"<a href="{{ suprnova::url::root() }}/search?q=a#top">search</a>"#,
        r#"{% let home = suprnova::url::root() %}<a href="{{ home }}/x">x</a>"#,
    ] {
        let report = scan_view(&format!("{view}\n"));
        assert!(report.accepted(), "{view}: {:?}", report.findings);
    }
}

/// REG-031: anything else after `suprnova::url::root()` is refused: a second
/// slash or a backslash, in any spelling a browser reads, no slash, a value,
/// or nothing. A template local keeps what it was bound to, and a macro
/// argument cannot carry the root into a URL attribute, so neither writes
/// it there without the rooted constant.
#[test]
fn reg_031_url_root_before_anything_but_a_rooted_constant_is_refused() {
    for view in [
        r#"<a href="{{ suprnova::url::root() }}//evil.example/x">x</a>"#,
        r#"<a href="{{ suprnova::url::root() }}/\evil.example">x</a>"#,
        r#"<a href="{{ suprnova::url::root() }}/&#47;evil.example">x</a>"#,
        r#"<a href="{{ suprnova::url::root() }}/&#9;/evil.example">x</a>"#,
        r#"<a href="{{ suprnova::url::root() }}/&#x5c;evil.example">x</a>"#,
        r#"<a href="{{ suprnova::url::root() }}evil.example">x</a>"#,
        r#"<a href="{{ suprnova::url::root() }}{{ value }}">x</a>"#,
        r#"<a href="{{ suprnova::url::root() }}">x</a>"#,
        r#"<a href="https://evil.example{{ suprnova::url::root() }}/x">x</a>"#,
        r#"<a href="/{{ suprnova::url::root() }}/x">x</a>"#,
        r#"{% let home = suprnova::url::root() %}<a href="{{ home }}">x</a>"#,
        r#"{% let home = suprnova::url::root() %}<a href="{{ home }}//evil.example/x">x</a>"#,
        r#"{% if let home = suprnova::url::root() %}<a href="{{ home }}">x</a>{% endif %}"#,
        r#"{% match suprnova::url::root() %}{% when home %}<a href="{{ home }}">x</a>{% endmatch %}"#,
        r#"{% for part in suprnova::url::root() %}<a href="{{ part }}/x">x</a>{% endfor %}"#,
        r#"{% macro link(href) %}<a href="{{ href }}">x</a>{% endmacro %}{% call link(suprnova::url::root()) %}{% endcall %}"#,
        r#"{% macro link(href) %}<a href="{{ href }}">x</a>{% endmacro %}{% let home = suprnova::url::root() %}{% call link(home) %}{% endcall %}"#,
        r#"{% macro link(href = suprnova::url::root()) %}<a href="{{ href }}">x</a>{% endmacro %}{% call link() %}{% endcall %}"#,
    ] {
        let report = scan_view(&format!("{view}\n"));
        assert!(
            report
                .findings
                .iter()
                .any(|finding| finding.check == "view-url" && finding.line == Some(1)),
            "{view} was not refused: {:?}",
            report.findings
        );
    }
}

/// The key the vendored library below signs with.
const LIBRARY_SEED: [u8; 32] = [7; 32];

/// A project to install into, holding a signed library under
/// `vendor/acme-ui` whose one component, `widget`, has the view `view`, with
/// the library's key pinned (REG-024). Returns the project and the
/// component's manifest.
fn project_with_widget(view: &str) -> (tempfile::TempDir, PathBuf) {
    let project = tempfile::tempdir().expect("tempdir");
    let root = project.path();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"demo-app\"\nversion = \"0.1.0\"\n\n[dependencies]\nsuprnova = { git = \"https://github.com/eas4ai/suprnova.git\", tag = \"v3.2.1\" }\n",
    )
    .expect("manifest");
    let library = root.join("vendor/acme-ui");
    let component = library.join("components/widget");
    fs::create_dir_all(&component).expect("vendor dir");
    let secret = SecretKey::from_bytes(LIBRARY_SEED);
    let library_json = serde_json::to_vec_pretty(&serde_json::json!({
        "namespace": "acme",
        "source": "github.com/acme/acme-ui",
        "version": "1.0.0",
        "framework": ">=3.0.0",
        "publicKey": secret.public_key().encode(),
    }))
    .expect("library.json");
    let manifest = serde_json::to_vec_pretty(&serde_json::json!({
        "name": "acme.widget",
        "root": "acme-ui/widget",
        "files": ["widget.html", "widget.css"],
    }))
    .expect("manifest.json");
    let css = b".acme-widget { display: block; }\n";
    fs::write(library.join("library.json"), &library_json).expect("library.json");
    fs::write(component.join("manifest.json"), &manifest).expect("manifest.json");
    fs::write(component.join("widget.html"), view).expect("view");
    fs::write(component.join("widget.css"), css).expect("stylesheet");
    let statement = Statement {
        library: "github.com/acme/acme-ui".to_owned(),
        version: semver::Version::parse("1.0.0").expect("semver"),
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

    let canonical = fs::canonicalize(&library).expect("canonical");
    let mut file = ProjectFile::load(root).expect("load the project file");
    file.set_library(
        &LibraryAddress(canonical.to_str().expect("utf-8").to_owned()),
        None,
        Some(&secret.public_key()),
    )
    .expect("pin the library's key");
    file.save().expect("save the project file");
    (project, component.join("manifest.json"))
}

/// `suprnova live:add --manifest <manifest> --yes` in `root`: the install
/// path, which scans the component with its manifest's context
/// (`scan_component_in`) before it writes a file.
fn live_add(root: &Path, manifest: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_suprnova"))
        .args(["live:add", "--manifest"])
        .arg(manifest)
        .arg("--yes")
        .current_dir(root)
        .output()
        .expect("run suprnova live:add")
}

fn printed(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// REG-031 through the install path: `live:add` refuses a component whose
/// view breaks a view rule, naming the check, and writes nothing; a clean
/// view, `url::root()` before a rooted constant path included, installs.
#[test]
fn reg_031_live_add_refuses_a_view_that_breaks_a_rule_and_installs_a_clean_one() {
    for (view, check) in [
        ("<div onclick=\"steal()\">Widget</div>\n", "onclick"),
        ("<a href=\"javascript:steal()\">Widget</a>\n", "javascript:"),
        ("<script>steal()</script>\n", "script"),
        (
            "<a href=\"{{ suprnova::url::root() }}//evil.example/x\">x</a>\n",
            "view-url",
        ),
    ] {
        let (project, manifest) = project_with_widget(view);
        let output = live_add(project.path(), &manifest);
        assert!(
            !output.status.success(),
            "{view:?} installed: {}",
            printed(&output)
        );
        assert!(
            printed(&output).contains("widget.html:1: [view-") && printed(&output).contains(check),
            "{view:?}: the refusal is not the view finding naming {check}: {}",
            printed(&output)
        );
        assert!(
            !project.path().join("templates/acme-ui").exists(),
            "{view:?}: a refused component wrote files"
        );
    }
    for view in [
        "<div class=\"acme-widget\">Widget</div>\n",
        "<a href=\"{{ suprnova::url::root() }}/x\">x</a>\n",
    ] {
        let (project, manifest) = project_with_widget(view);
        let output = live_add(project.path(), &manifest);
        assert!(
            output.status.success(),
            "{view:?} was refused: {}",
            printed(&output)
        );
        assert_eq!(
            fs::read_to_string(project.path().join("templates/acme-ui/widget/widget.html"))
                .expect("the installed view"),
            view
        );
    }
}
