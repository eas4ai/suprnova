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
    // Two `../` from `acme-ui/x/` reach the components' root and come back
    // in: under a prefix the browser resolves it inside the prefix.
    let root = scan("import \"../../acme-ui/x/x.js\";\n");
    assert!(root.accepted(), "{:?}", root.findings);
    // A third leaves the root: `/prefix/acme-ui/x/main.js` resolves it to
    // `/acme-ui/x/x.js`, outside the prefix.
    for specifier in [
        "/acme-ui/x/x.js",
        "/acme-ui/y/y.js",
        "https://cdn.example/x.js",
        "//cdn.example/x.js",
        "../../../acme-ui/x/x.js",
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

/// Scans `script` as the only script of a component, under `widget.js`.
fn scan_widget_script(script: &str) -> ScanReport {
    let files = vec![
        ("widget.html".to_string(), b"<div>x</div>".to_vec()),
        ("widget.js".to_string(), script.as_bytes().to_vec()),
    ];
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

/// Each of `lines` of `widget.js` that a scan of `script` does not refuse
/// with `check`, described with every finding the scan made.
fn missing_refusals(script: &str, check: &str, lines: &[u32]) -> Vec<String> {
    let report = scan_widget_script(script);
    lines
        .iter()
        .filter(|line| {
            !report.findings.iter().any(|finding| {
                finding.check == check
                    && finding.file == "widget.js"
                    && finding.line == Some(**line)
            })
        })
        .map(|line| {
            format!(
                "{script:?}: no `{check}` at widget.js:{line}; got [{}]",
                report
                    .findings
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(" | ")
            )
        })
        .collect()
}

/// REG-032: a built-in prototype is refused however the script reaches it:
/// under a computed key that traces to `prototype` (a string, a template
/// literal, a concatenation), through a name that holds it, from
/// `getPrototypeOf`, or handed to a function that changes it. Each case
/// names the lines refused with `script-prototype`; a destructured name
/// the scan cannot follow is stopped where the prototype enters it.
#[test]
fn reg_032_a_prototype_reached_by_a_constant_key_an_alias_or_a_call_is_refused() {
    let cases: &[(&str, &[u32])] = &[
        ("Array[\"prototype\"].polluted = 1;\n", &[1]),
        ("Array[`prototype`].polluted = 1;\n", &[1]),
        (
            "const key = \"proto\" + \"type\";\nArray[key].polluted = 1;\n",
            &[2],
        ),
        ("const p = Array.prototype;\np.polluted = 1;\n", &[1, 2]),
        ("let p;\np = Array.prototype;\np.polluted = 1;\n", &[2, 3]),
        ("(0, Array.prototype).polluted = 1;\n", &[1]),
        (
            "const slice = Array.prototype.slice;\nslice.call = () => 1;\n",
            &[2],
        ),
        ("Object.getPrototypeOf([]).polluted = 1;\n", &[1]),
        ("Reflect.getPrototypeOf([]).x = 1;\n", &[1]),
        (
            "Object.defineProperty(Array.prototype, \"x\", { value: 1 });\n",
            &[1],
        ),
        ("Object.assign(Array.prototype, { x: 1 });\n", &[1]),
        ("Object.setPrototypeOf(Array.prototype, null);\n", &[1]),
        ("Object.freeze(Array.prototype);\n", &[1]),
        ("Reflect.set(Array.prototype, \"x\", 1);\n", &[1]),
        (
            "Reflect.defineProperty(Array.prototype, \"x\", { value: 1 });\n",
            &[1],
        ),
        (
            "function pollute(p) {\n  p.x = 1;\n}\npollute(Array.prototype);\n",
            &[4],
        ),
        ("const [p] = [Array.prototype];\np.x = 1;\n", &[1]),
    ];
    let failures: Vec<String> = cases
        .iter()
        .flat_map(|(script, lines)| missing_refusals(script, "script-prototype", lines))
        .collect();
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// REG-032: a prototype may not be kept or passed on, so it is refused in
/// every position where the script uses it as a value.
#[test]
fn reg_032_a_prototype_used_as_a_value_is_refused_in_every_position() {
    let cases: &[(&str, u32)] = &[
        ("export const p = Array.prototype;\n", 1),
        ("let p;\np = Array.prototype;\n", 2),
        ("const xs = [];\nxs.push(Array.prototype);\n", 2),
        ("const xs = [];\nxs.push(...Array.prototype);\n", 2),
        ("export const s = new Set(Array.prototype);\n", 1),
        ("export function f() {\n  return Array.prototype;\n}\n", 2),
        ("export function* g() {\n  yield Array.prototype;\n}\n", 2),
        (
            "export async function a() {\n  await Array.prototype;\n}\n",
            2,
        ),
        ("export const a = () => Array.prototype;\n", 1),
        ("export const xs = [Array.prototype];\n", 1),
        ("export const xs = [...Array.prototype];\n", 1),
        ("export const o = { p: Array.prototype };\n", 1),
        ("export const o = { ...Array.prototype };\n", 1),
        (
            "export function f(p = Array.prototype) {\n  return p;\n}\n",
            1,
        ),
        ("export const t = `${Array.prototype}`;\n", 1),
        ("export const c = (x) => (x ? Array.prototype : null);\n", 1),
        ("export const l = (x) => x ?? Array.prototype;\n", 1),
        ("export const s = (0, Array.prototype);\n", 1),
        ("export const p = Array?.prototype;\n", 1),
        ("export class A {\n  p = Array.prototype;\n}\n", 2),
        ("export default Array.prototype;\n", 1),
        ("const p = Array.prototype;\nexport const q = p;\n", 2),
    ];
    let failures: Vec<String> = cases
        .iter()
        .flat_map(|(script, line)| missing_refusals(script, "script-prototype", &[*line]))
        .collect();
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// REG-032: every way a script writes a member of a prototype changes the
/// prototype, so each is refused, not only a plain assignment: a compound
/// or logical assignment, `++` and `--`, a destructuring target, a `for`
/// loop's target, and `delete`.
#[test]
fn reg_032_every_write_to_a_prototype_member_is_refused() {
    let cases: &[(&str, u32)] = &[
        ("Array.prototype.polluted ??= 1;\n", 1),
        ("Array.prototype.polluted ||= 1;\n", 1),
        ("Array.prototype.count += 1;\n", 1),
        ("Array.prototype.count++;\n", 1),
        ("--Array.prototype.count;\n", 1),
        ("[Array.prototype.polluted] = [1];\n", 1),
        ("({ a: Array.prototype.polluted } = { a: 1 });\n", 1),
        ("for (Array.prototype.polluted of [1]) {\n}\n", 1),
        ("for (Array.prototype.polluted in { a: 1 }) {\n}\n", 1),
        (
            "const slice = Array.prototype.slice;\nslice.count += 1;\n",
            2,
        ),
        ("delete Array.prototype.map;\n", 1),
        ("delete Array[\"prototype\"].map;\n", 1),
        ("const p = Array.prototype;\ndelete p.map;\n", 2),
    ];
    let failures: Vec<String> = cases
        .iter()
        .flat_map(|(script, line)| missing_refusals(script, "script-prototype", &[*line]))
        .collect();
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// REG-032: a method called on a prototype itself runs with the prototype
/// as `this`, and `Array.prototype` is an array, so `push`, `fill` and
/// `splice` change it. Each such call is refused, a tagged template's
/// included. A method borrowed with `call` runs on the value it is given
/// and stays admitted.
#[test]
fn reg_032_a_method_called_on_a_prototype_itself_is_refused() {
    let cases: &[(&str, u32)] = &[
        ("Array.prototype.push(1);\n", 1),
        ("Array.prototype.fill(0, 0, 1);\n", 1),
        ("Array[\"prototype\"].splice(0, 0, 1);\n", 1),
        ("Array.prototype?.push(1);\n", 1),
        ("(0, Array.prototype).push(1);\n", 1),
        ("Array.prototype.push`polluted`;\n", 1),
    ];
    let failures: Vec<String> = cases
        .iter()
        .flat_map(|(script, line)| missing_refusals(script, "script-prototype", &[*line]))
        .collect();
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
    let borrowed =
        scan_widget_script("export const add = (xs) => Array.prototype.push.call(xs, 1);\n");
    assert!(borrowed.accepted(), "{:?}", borrowed.findings);
}

/// REG-032: destructuring `prototype` out of an object puts the prototype
/// in a name the scan does not follow, so it is refused in a declaration,
/// a parameter, a `catch` clause, a `for` loop and a destructuring
/// assignment, by a static or computed key, shorthand or nested.
#[test]
fn reg_032_destructuring_a_prototype_out_of_an_object_is_refused() {
    let cases: &[(&str, u32)] = &[
        ("const { prototype: p } = Array;\np.polluted = 1;\n", 1),
        ("const { prototype } = Array;\nprototype.polluted = 1;\n", 1),
        ("const { [\"proto\" + \"type\"]: p } = Array;\n", 1),
        ("const { a: { prototype: p } } = { a: Array };\n", 1),
        (
            "function pollute({ prototype }) {\n  prototype.polluted = 1;\n}\npollute(Array);\n",
            1,
        ),
        ("for (const { prototype } of [Array]) {\n}\n", 1),
        ("try {\n} catch ({ prototype }) {\n}\n", 2),
        ("let p;\n({ prototype: p } = Array);\n", 2),
        ("let prototype;\n({ prototype } = Array);\n", 2),
        ("let p;\n({ [`prototype`]: p } = Array);\n", 2),
        ("let p;\n({ a: { prototype: p } } = { a: Array });\n", 2),
        ("let p;\n[{ prototype: p }] = [Array];\n", 2),
    ];
    let failures: Vec<String> = cases
        .iter()
        .flat_map(|(script, line)| missing_refusals(script, "script-prototype", &[*line]))
        .collect();
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// REG-032: a destructuring assignment's defaults and keys are expressions
/// the script evaluates, so the scan walks them at every depth as it walks
/// any value: a prototype there is refused, and so is `eval`, and a nested
/// key gets the property checks a top-level key gets.
#[test]
fn reg_032_every_key_and_default_of_a_destructuring_assignment_is_checked() {
    let cases: &[(&str, &str, u32)] = &[
        (
            "let p;\n[p = Array.prototype] = [];\np.polluted = 1;\n",
            "script-prototype",
            2,
        ),
        (
            "let p;\n({ p = Array.prototype } = {});\n",
            "script-prototype",
            2,
        ),
        (
            "let p;\n({ a: p = Array.prototype } = {});\n",
            "script-prototype",
            2,
        ),
        (
            "let q;\n({ a: { [Array.prototype]: q } } = { a: {} });\n",
            "script-prototype",
            2,
        ),
        ("let a;\n[a = eval(\"1\")] = [];\n", "script-eval", 2),
        ("let a;\n({ a = eval(\"1\") } = {});\n", "script-eval", 2),
        (
            "let a;\n({ x: [a = eval(\"1\")] } = { x: [] });\n",
            "script-eval",
            2,
        ),
        (
            "let q;\n({ a: { [eval(\"k\")]: q } } = { a: {} });\n",
            "script-eval",
            2,
        ),
        (
            "let c;\n({ a: { constructor: c } } = { a: [] });\n",
            "script-eval",
            2,
        ),
        (
            "let constructor;\n({ constructor } = []);\n",
            "script-eval",
            2,
        ),
    ];
    let failures: Vec<String> = cases
        .iter()
        .flat_map(|(script, check, line)| missing_refusals(script, check, &[*line]))
        .collect();
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// REG-032: a shorthand target in a destructuring assignment (`({ u } =
/// o)`) receives a value the scan does not follow, like any other target:
/// the binding it writes is no longer traced to its earlier constant, and
/// a name the script does not declare is a global write, checked as one.
#[test]
fn reg_032_a_shorthand_destructuring_target_is_a_target_like_any_other() {
    let cases: &[(&str, &str, u32)] = &[
        (
            "let u = \"/ok\";\n({ u } = { u: \"https://evil.example/x\" });\nconst img = new Image();\nimg.src = u;\n",
            "script-url",
            4,
        ),
        (
            "let u = \"/ok\";\n({ a: { u } } = { a: { u: \"https://evil.example/x\" } });\nconst img = new Image();\nimg.src = u;\n",
            "script-url",
            4,
        ),
        (
            "let u = \"/ok\";\n({ u = \"/fine\" } = { u: \"https://evil.example/x\" });\nconst img = new Image();\nimg.src = u;\n",
            "script-url",
            4,
        ),
        (
            "let u = \"/ok\";\nfor ({ u } of [{ u: \"https://evil.example/x\" }]) {\n}\nconst img = new Image();\nimg.src = u;\n",
            "script-url",
            5,
        ),
        (
            "let f = () => 1;\n({ f } = { f: \"alert(1)\" });\nsetTimeout(f, 1);\n",
            "script-timer",
            3,
        ),
        (
            "({ location } = { location: \"javascript:alert(1)\" });\n",
            "script-url",
            1,
        ),
        (
            "({ onerror } = { onerror: () => 1 });\n",
            "script-global",
            1,
        ),
    ];
    let failures: Vec<String> = cases
        .iter()
        .flat_map(|(script, check, line)| missing_refusals(script, check, &[*line]))
        .collect();
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
    let declared = scan_widget_script("let a = 0;\n({ a } = { a: 1 });\nexport const b = a;\n");
    assert!(declared.accepted(), "{:?}", declared.findings);
}

/// REG-032: a value the scan follows from elsewhere (an initializer, an
/// assignment, a call's argument) names the binding each of its names had
/// where it was written, not the binding the same name has where the value
/// is used, so a shadowing name cannot stand in for the real value.
#[test]
fn reg_032_a_traced_value_resolves_each_name_where_it_is_written() {
    let cases: &[(&str, &str, u32)] = &[
        (
            "const y = \"https://evil.example/x\";\nconst x = y;\nexport function show() {\n  const y = \"/ok\";\n  const img = new Image();\n  img.src = x;\n}\n",
            "script-url",
            6,
        ),
        (
            "let x = \"/ok\";\nexport function set() {\n  const y = \"https://evil.example/x\";\n  x = y;\n}\nconst y = \"/fine\";\nexport function show() {\n  const img = new Image();\n  img.src = x;\n}\n",
            "script-url",
            9,
        ),
        (
            "function load(u) {\n  const img = new Image();\n  img.src = u;\n}\nconst v = \"/ok\";\nexport function go() {\n  const v = \"https://evil.example/x\";\n  load(v);\n}\n",
            "script-url",
            3,
        ),
        (
            "const k = \"constructor\";\nconst n = k;\nexport function read(o) {\n  const k = 1;\n  return o[n];\n}\n",
            "script-eval",
            5,
        ),
        (
            "const g = \"alert(1)\";\nconst f = g;\nexport function later() {\n  function g() {}\n  setTimeout(f, 1);\n}\n",
            "script-timer",
            5,
        ),
    ];
    let failures: Vec<String> = cases
        .iter()
        .flat_map(|(script, check, line)| missing_refusals(script, check, &[*line]))
        .collect();
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
    let shadowed = scan_widget_script(
        "const y = \"/ok\";\nconst x = y;\nexport function show() {\n  const y = \"https://evil.example/x\";\n  const img = new Image();\n  img.src = x;\n}\n",
    );
    assert!(shadowed.accepted(), "{:?}", shadowed.findings);
}

/// REG-032: reading a prototype's member is admitted, so the usual ways of
/// borrowing a built-in method stay open. A member read from a prototype is
/// a value, but not the prototype itself, so a script may keep one in a
/// name (`const has = Object.prototype.hasOwnProperty`); exporting it is
/// the built-in rule's to refuse, because it would leave the file the scan
/// reads. `hasOwnProperty` itself is not an admitted method, so calling it
/// is refused by the call rule, but not as a prototype.
#[test]
fn reg_032_a_prototype_read_through_a_member_is_admitted() {
    let admitted = [
        "export const copy = (xs) => Array.prototype.slice.call(xs);\n",
        "export const kind = (x) => Object.prototype.toString.call(x);\n",
        "export const isList = (x) => Array.isArray(x);\n",
        "export const isArray = (x) => x instanceof Array;\n",
        "export class A {\n  static of() {}\n}\n",
        "export const count = (o) => Object.keys(o).length;\n",
        "const has = Object.prototype.hasOwnProperty;\n",
        "export const same = (x) => x === Array.prototype;\n",
        "export const kindOf = typeof Array.prototype;\n",
    ];
    for script in admitted {
        let report = scan_widget_script(script);
        assert!(
            report.accepted(),
            "{script:?} was refused: {}",
            report
                .findings
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" | ")
        );
    }
    let borrowed = scan_widget_script(
        "export const has = (o, k) => Object.prototype.hasOwnProperty.call(o, k);\n",
    );
    assert!(
        borrowed
            .findings
            .iter()
            .all(|finding| finding.check != "script-prototype"),
        "{:?}",
        borrowed.findings
    );
}

/// Every `(script, check, line)` case whose scan does not refuse that line
/// of `widget.js` with that check.
fn missing_cases(cases: &[(&str, &str, u32)]) -> Vec<String> {
    cases
        .iter()
        .flat_map(|(script, check, line)| missing_refusals(script, check, &[*line]))
        .collect()
}

/// REG-032: a write to a member of a built-in changes it for every script
/// on the page, so it is refused however the script names the built-in (by
/// its name or through the global object) and however it writes (`=`, a
/// compound or logical assignment, `++`, a destructuring or `for` loop
/// target, `delete`). A built-in function counts: `Object.keys.call` is a
/// member of the built-in `Object.keys`.
#[test]
fn reg_032_a_write_to_a_member_of_a_built_in_is_refused_in_every_form() {
    let cases: &[(&str, &str, u32)] = &[
        ("Object.keys = () => [];\n", "script-builtin", 1),
        ("JSON.parse = () => null;\n", "script-builtin", 1),
        ("Math.random = () => 0;\n", "script-builtin", 1),
        ("globalThis.Object.keys = () => [];\n", "script-builtin", 1),
        ("window.JSON.parse = () => null;\n", "script-builtin", 1),
        ("self[\"Math\"].random = () => 0;\n", "script-builtin", 1),
        ("console.log = () => {};\n", "script-builtin", 1),
        ("customElements.define = () => {};\n", "script-builtin", 1),
        ("Object.keys.call = () => [];\n", "script-builtin", 1),
        ("Object.keys ??= () => [];\n", "script-builtin", 1),
        ("Math.random ||= () => 0;\n", "script-builtin", 1),
        ("JSON.parse += \"\";\n", "script-builtin", 1),
        ("Math.random++;\n", "script-builtin", 1),
        ("delete JSON.parse;\n", "script-builtin", 1),
        ("[Math.random] = [() => 0];\n", "script-builtin", 1),
        (
            "({ a: JSON.parse } = { a: () => null });\n",
            "script-builtin",
            1,
        ),
        ("for (Math.random of [() => 0]) {\n}\n", "script-builtin", 1),
    ];
    let failures = missing_cases(cases);
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// REG-032: a built-in, or a member a built-in hands over, kept in a name is
/// followed to where a member of it is written: a variable's initializer
/// and assignments, a parameter's default, and the arguments every call of
/// its function passes, through as many functions as hand it on. A member
/// read from a prototype stays the prototype rule's.
#[test]
fn reg_032_a_built_in_held_in_a_name_is_refused_where_a_member_is_written() {
    let cases: &[(&str, &str, u32)] = &[
        (
            "const k = Object;\nk.keys = () => [];\n",
            "script-builtin",
            2,
        ),
        (
            "let k;\nk = JSON;\nk.parse = () => null;\n",
            "script-builtin",
            3,
        ),
        (
            "const k = Object.keys;\nk.call = () => [];\n",
            "script-builtin",
            2,
        ),
        (
            "export function f(c) {\n  const k = c ? Object : JSON;\n  k.keys = () => [];\n}\n",
            "script-builtin",
            3,
        ),
        (
            "function f(p) {\n  p.call = () => [];\n}\nf(Object.keys);\n",
            "script-builtin",
            2,
        ),
        (
            "function f(p = Object.keys) {\n  p.call = () => [];\n}\nf();\n",
            "script-builtin",
            2,
        ),
        (
            "const f = (p) => {\n  p.parse = () => null;\n};\nf(JSON);\n",
            "script-builtin",
            2,
        ),
        (
            "function f(p) {\n  g(p);\n}\nfunction g(q) {\n  q.random = () => 0;\n}\nf(Math);\n",
            "script-builtin",
            5,
        ),
        (
            "function f(p) {\n  p = Math;\n  p.random = () => 0;\n}\nf({});\n",
            "script-builtin",
            3,
        ),
        (
            "export function f() {\n  const k = Object;\n  setTimeout(() => {\n    k.keys = () => [];\n  }, 0);\n}\n",
            "script-builtin",
            4,
        ),
        (
            "function f(p) {\n  p.call = () => 1;\n}\nf(Array.prototype.slice);\n",
            "script-prototype",
            2,
        ),
        (
            "const f = (p) => {\n  delete p.call;\n};\nf(Array.prototype.slice);\n",
            "script-prototype",
            2,
        ),
    ];
    let failures = missing_cases(cases);
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// REG-032: a method every value inherits is a built-in function, so a
/// member written on one read from any value the script did not make
/// (`[].slice`, an element's `addEventListener`, `super.focus`) is refused.
/// `document`, `location` and `history` are the page, whose members a
/// component may change, but a method of theirs is a built-in function, so
/// assigning one is refused however the page object is reached.
#[test]
fn reg_032_a_built_in_method_reached_through_any_value_or_the_page_is_refused() {
    let cases: &[(&str, &str, u32)] = &[
        ("[].slice.call = () => 1;\n", "script-builtin", 1),
        ("({}).toString.call = () => \"\";\n", "script-builtin", 1),
        (
            "const s = \"\".trim;\ns.call = () => \"\";\n",
            "script-builtin",
            2,
        ),
        (
            "export function f(el) {\n  el.addEventListener.call = () => {};\n}\n",
            "script-builtin",
            2,
        ),
        (
            "export class A extends HTMLElement {\n  m() {\n    super.focus.call = () => {};\n  }\n}\n",
            "script-builtin",
            3,
        ),
        (
            "export class A extends HTMLElement {\n  m() {\n    this.querySelector.call = () => null;\n  }\n}\n",
            "script-builtin",
            3,
        ),
        (
            "document.createElement = () => null;\n",
            "script-builtin",
            1,
        ),
        ("location.assign = () => {};\n", "script-builtin", 1),
        ("history.pushState = () => {};\n", "script-builtin", 1),
        (
            "const d = document;\nd.querySelector = () => null;\n",
            "script-builtin",
            2,
        ),
        (
            "window.document.getElementById = () => null;\n",
            "script-builtin",
            1,
        ),
        (
            "let d;\nd ||= document;\nd.querySelector = () => null;\n",
            "script-builtin",
            3,
        ),
        (
            "function f(d) {\n  d.createElement = () => null;\n}\nf(document);\n",
            "script-builtin",
            2,
        ),
        (
            "export class A extends HTMLElement {\n  m() {\n    this.ownerDocument.createElement = () => null;\n  }\n}\n",
            "script-builtin",
            3,
        ),
        (
            "document.location.assign = () => {};\n",
            "script-builtin",
            1,
        ),
        (
            "export function clear(form) {\n  form.search.value = \"\";\n}\n",
            "script-builtin",
            2,
        ),
    ];
    let failures = missing_cases(cases);
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Each script of `scripts` that a scan refuses, described with every
/// finding the scan made.
fn refused_scripts(scripts: &[&str]) -> Vec<String> {
    scripts
        .iter()
        .filter_map(|script| {
            let report = scan_widget_script(script);
            (!report.accepted()).then(|| {
                format!(
                    "{script:?} was refused: {}",
                    report
                        .findings
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(" | ")
                )
            })
        })
        .collect()
}

/// REG-032: a member read off a literal is the built-in method the literal
/// inherits, whatever its name, so the scan needs no list of names for it:
/// `(0).toPrecision` is `Number.prototype.toPrecision`, `"".anchor` and
/// `[].copyWithin` are the string's and the array's, and an operator's
/// result (`-1`, `"a" + 1`, `typeof x`) is a primitive like a literal. A
/// write to any member of one is refused, and so is passing one where the
/// scan stops following it, read off the literal or off a constant that
/// holds it. A class that extends a built-in inherits its statics the same
/// way, so a static the class does not declare is the built-in's. What a
/// literal holds itself (a string's or an array's `length` and indices, a
/// regular expression's `lastIndex`, `source` and flags) and a class's own
/// statics stay admitted, and so does a parameter that is a string in one
/// call and an object in another: the scan reads no `typeof` test, so it
/// follows no literal into a parameter or a variable.
#[test]
fn reg_032_a_member_read_off_a_literal_is_the_built_in_method_it_inherits() {
    let cases: &[(&str, &str, u32)] = &[
        ("(0).toPrecision.call = () => \"\";\n", "script-builtin", 1),
        ("(0).toPrecision.label = \"x\";\n", "script-builtin", 1),
        ("(1.5).toExponential.label = \"x\";\n", "script-builtin", 1),
        ("\"\".anchor.label = \"x\";\n", "script-builtin", 1),
        ("`${1}`.substr.label = \"x\";\n", "script-builtin", 1),
        ("true.x.label = 1;\n", "script-builtin", 1),
        ("(1n).x.label = 1;\n", "script-builtin", 1),
        ("/x/.compile.label = 1;\n", "script-builtin", 1),
        ("[].copyWithin.label = 1;\n", "script-builtin", 1),
        ("[1, 2].toSpliced.label = 1;\n", "script-builtin", 1),
        ("(0)[\"toPrecision\"].label = 1;\n", "script-builtin", 1),
        ("(0).toPrecision.call.label = 1;\n", "script-builtin", 1),
        ("(-1).toPrecision.label = 1;\n", "script-builtin", 1),
        ("(\"a\" + 1).anchor.label = 1;\n", "script-builtin", 1),
        ("(typeof 0).big.label = 1;\n", "script-builtin", 1),
        ("delete \"\".anchor.label;\n", "script-builtin", 1),
        (
            "const n = 0;\nn.toPrecision.label = 1;\n",
            "script-builtin",
            2,
        ),
        (
            "const re = /x/;\nre.compile.label = 1;\n",
            "script-builtin",
            2,
        ),
        ("export const p = (0).toPrecision;\n", "script-builtin", 1),
        (
            "const n = 0;\nexport const p = n.toPrecision;\n",
            "script-builtin",
            2,
        ),
        (
            "Promise.resolve([].copyWithin).then((m) => {\n  m.label = 1;\n});\n",
            "script-builtin",
            1,
        ),
        (
            "(class extends Array {}).of.label = 1;\n",
            "script-builtin",
            1,
        ),
        (
            "class A extends Promise {}\nA.withResolvers.label = 1;\n",
            "script-builtin",
            2,
        ),
        (
            "class A extends Map {}\nclass B extends A {}\nB.groupBy.label = 1;\n",
            "script-builtin",
            3,
        ),
        (
            "class A extends Array {\n  of = 1;\n}\nA.of.label = 1;\n",
            "script-builtin",
            4,
        ),
        (
            "const A = class extends Array {};\nexport const of = A.of;\n",
            "script-builtin",
            2,
        ),
    ];
    let mut failures = missing_cases(cases);
    failures.extend(refused_scripts(&[
        "export const n = (s) => Math.max(\"abc\".length, [1, 2].length, s);\n",
        "export const re = [/x/g.source, /x/g.flags, /x/g.lastIndex, \"ab\"[1]];\n",
        "const label = \"x\";\nexport const size = [label.length, label[0]];\n",
        "const cells = [];\nexport const fill = (text) => {\n  cells[0].textContent = text;\n};\n",
        "export class A extends HTMLElement {\n  static config = {};\n}\nA.config.open = true;\n",
        "export const trimmed = (s) => \" a \".trim() + `${s}`.toUpperCase();\n",
        "function labelOf(option) {\n  return typeof option === \"string\" ? option : option.label;\n}\nexport const labels = [labelOf(\"x\"), labelOf({ label: \"y\" })];\n",
        "function highlight(target) {\n  const el = typeof target === \"string\" ? document.querySelector(target) : target;\n  el.style.outline = \"1px solid\";\n}\nhighlight(\"#x\");\n",
    ]));
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// REG-032: `getRootNode()` returns the document for a node in it, as
/// `ownerDocument` does, whatever node it is called on, and `Object(value)`
/// returns the value itself, so the result of either is the page to the
/// rule that refuses assigning a method of the page: assigning `open` on
/// `document.getRootNode()` is refused as `document.open = f` is.
#[test]
fn reg_032_the_root_node_and_the_object_of_the_page_are_the_page() {
    let cases: &[(&str, &str, u32)] = &[
        (
            "document.getRootNode().open = () => null;\n",
            "script-builtin",
            1,
        ),
        (
            "export class A extends HTMLElement {\n  connectedCallback() {\n    this.getRootNode().close = () => {};\n  }\n}\n",
            "script-builtin",
            3,
        ),
        (
            "export function f(el) {\n  el.getRootNode().append = () => {};\n}\n",
            "script-builtin",
            2,
        ),
        (
            "const root = document.body.getRootNode();\nroot.open = () => null;\n",
            "script-builtin",
            2,
        ),
        (
            "document.body.getRootNode.call(document).open = () => null;\n",
            "script-builtin",
            1,
        ),
        (
            "export class A extends HTMLElement {\n  connectedCallback() {\n    const root = this.getRootNode.bind(this);\n    root().open = () => null;\n  }\n}\n",
            "script-builtin",
            4,
        ),
        (
            "document[\"getRootNode\"]().contains = () => true;\n",
            "script-builtin",
            1,
        ),
        ("Object(document).open = () => null;\n", "script-builtin", 1),
        ("Object(location).reload = () => {};\n", "script-builtin", 1),
    ];
    let mut failures = missing_cases(cases);
    failures.extend(refused_scripts(&[
        "export class A extends HTMLElement {\n  connectedCallback() {\n    this.getRootNode().title = \"x\";\n    this.getRootNode().body.hidden = false;\n  }\n}\n",
        "const o = {};\nObject(o).open = true;\n",
    ]));
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// REG-032: other scripts reach a built-in function's code through its
/// `call`, `apply` and `bind` (`Array.prototype.slice.call(list)`), and a
/// value the script did not make may hold any built-in method under any
/// name: a number from `Math.random()` inherits `toPrecision`, a string
/// from an input's `value` inherits `anchor`, an element inherits
/// `requestFullscreen`. Writing one of those three names is refused on
/// every value but one the script made: an object, array, function or
/// class literal, a function or class it declares, a new instance of a
/// standard constructor, or a name that holds only those. A `this` in a
/// class is its instance, and its `call` stays its own.
#[test]
fn reg_032_call_apply_or_bind_written_on_a_value_the_script_did_not_make_is_refused() {
    let cases: &[(&str, &str, u32)] = &[
        (
            "Math.random().toPrecision.call = () => \"\";\n",
            "script-builtin",
            1,
        ),
        ("(-1).toPrecision.call = () => \"\";\n", "script-builtin", 1),
        (
            "export function f(input) {\n  input.value.anchor.apply = () => \"\";\n}\n",
            "script-builtin",
            2,
        ),
        (
            "export function f(el) {\n  el.requestFullscreen.bind = () => null;\n}\n",
            "script-builtin",
            2,
        ),
        (
            "export class A extends HTMLElement {\n  connectedCallback() {\n    this.requestFullscreen.call = () => null;\n  }\n}\n",
            "script-builtin",
            3,
        ),
        (
            "const s = new Set();\ns.union.call = () => s;\n",
            "script-builtin",
            2,
        ),
        (
            "export function f(p) {\n  p.call = () => null;\n}\n",
            "script-builtin",
            2,
        ),
        (
            "let o = {};\nexport function f(p) {\n  o = p;\n  o.call = () => null;\n}\n",
            "script-builtin",
            4,
        ),
    ];
    let mut failures = missing_cases(cases);
    failures.extend(refused_scripts(&[
        "const o = {};\no.call = 1;\n",
        "const xs = [];\nxs.apply = true;\n",
        "function f() {}\nf.bind = null;\n",
        "const m = new Map();\nm.call = 1;\n",
        "let o;\no = { call: 0 };\no.call = 1;\n",
        "function f(p) {\n  p.call = 1;\n}\nf({});\n",
        "export class A extends HTMLElement {\n  connectedCallback() {\n    this.call = null;\n  }\n}\n",
    ]));
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// REG-032: the scan follows a built-in only through names: a variable, and
/// a parameter of a function the script calls by name. Anywhere else it
/// would leave for code the scan cannot follow (a destructured name, an
/// array or object, a member, another script through an export, a function
/// it does not trace, a return or a browser API that hands it back), so it
/// is refused there, which covers `Object.defineProperty` and `Reflect.set`
/// targeting one. A built-in may still be called, read from, compared, and
/// given to a browser API that only calls it back (`map(Number)`).
#[test]
fn reg_032_a_built_in_is_refused_where_it_leaves_the_names_the_scan_follows() {
    let cases: &[(&str, &str, u32)] = &[
        (
            "const { keys } = Object;\nkeys.call = () => [];\n",
            "script-builtin",
            1,
        ),
        (
            "const [s] = [Array.prototype.slice];\ns.call = () => 1;\n",
            "script-builtin",
            1,
        ),
        (
            "let k;\n({ k } = { k: Object.keys });\n",
            "script-builtin",
            2,
        ),
        (
            "Object.defineProperty(Object, \"keys\", { value: () => [] });\n",
            "script-builtin",
            1,
        ),
        (
            "Reflect.set(Math, \"random\", () => 0);\n",
            "script-builtin",
            1,
        ),
        ("Object.freeze(Math);\n", "script-builtin", 1),
        (
            "export const values = Object.values(Math);\n",
            "script-builtin",
            1,
        ),
        ("const xs = [];\nxs.push(Math);\n", "script-builtin", 2),
        ("Array.prototype.push.call(Math, 1);\n", "script-builtin", 1),
        ("export const m = Math;\n", "script-builtin", 1),
        (
            "export const has = Object.prototype.hasOwnProperty;\n",
            "script-builtin",
            1,
        ),
        ("const m = JSON;\nexport { m };\n", "script-builtin", 1),
        ("export default Object.keys;\n", "script-builtin", 1),
        ("export const get = () => Math;\n", "script-builtin", 1),
        (
            "export function get() {\n  return JSON;\n}\n",
            "script-builtin",
            2,
        ),
        (
            "export const o = { random: Math.random };\n",
            "script-builtin",
            1,
        ),
        ("export const o = { Math };\n", "script-builtin", 1),
        ("const o = {};\no.m = Math;\n", "script-builtin", 2),
        (
            "Promise.resolve(Math).then((m) => {\n  m.random = () => 0;\n});\n",
            "script-builtin",
            1,
        ),
        (
            "setTimeout((m) => {\n  m.random = () => 0;\n}, 0, Math);\n",
            "script-builtin",
            3,
        ),
        ("export class A {\n  m = Math;\n}\n", "script-builtin", 2),
        (
            "export function* g() {\n  yield Math;\n}\n",
            "script-builtin",
            2,
        ),
        (
            "function f({ random }) {\n  random.call = () => 0;\n}\nf(Math);\n",
            "script-builtin",
            4,
        ),
        (
            "function f(p) {\n  arguments[0].random = () => 0;\n}\nf(Math);\n",
            "script-builtin",
            4,
        ),
        ("let k;\nk ??= Object;\n", "script-builtin", 2),
        ("throw Math;\n", "script-builtin", 1),
        ("const xs = [];\nxs.push((0, Math));\n", "script-builtin", 2),
        (
            "const xs = [];\nlet k;\nxs.push(k = Math);\n",
            "script-builtin",
            3,
        ),
        (
            "import { f } from \"./dep.js\";\nf(Math);\n",
            "script-builtin",
            2,
        ),
        (
            "function run(o) {\n  o.forEach(Math);\n}\nrun({ forEach(p) {\n  p.random = () => 0;\n} });\n",
            "script-builtin",
            2,
        ),
    ];
    let failures = missing_cases(cases);
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// REG-032: an expression that may yield the global object is the global
/// object to every rule about its members: a sequence's last expression, a
/// conditional's branch and a logical expression's side. Each read below
/// is refused as `window.localStorage` is, naming the global it reads, not
/// only the global object it passes through.
#[test]
fn reg_032_the_global_object_reached_through_an_expression_is_the_global_object() {
    let cases: &[(&str, u32)] = &[
        ("export const s = window.localStorage;\n", 1),
        ("export const s = (0, window).localStorage;\n", 1),
        (
            "export function f(flag) {\n  return (flag ? window : self).localStorage;\n}\n",
            2,
        ),
        (
            "export function f(w) {\n  return (w || globalThis).localStorage;\n}\n",
            2,
        ),
        ("export const s = (0, self)[\"local\" + \"Storage\"];\n", 1),
        ("(0, globalThis).fetch = () => null;\n", 1),
    ];
    let mut failures = Vec::new();
    for (script, line) in cases {
        let report = scan_widget_script(script);
        let named = report.findings.iter().any(|finding| {
            finding.check == "script-global"
                && finding.line == Some(*line)
                && (finding.message.contains("`localStorage`")
                    || finding.message.contains("`fetch`"))
        });
        if !named {
            failures.push(format!(
                "{script:?}: no `script-global` naming the global at widget.js:{line}; got [{}]",
                report
                    .findings
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(" | ")
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// REG-032: writing a name on the global object makes it a global of the
/// script's own, which the script may then read back, only when the name
/// cannot be one of the browser's own properties there. A browser global
/// whose write throws, as `localStorage`'s does, would otherwise be read
/// for real after a `try`. Each read below is refused as the plain read is,
/// however the name was written; the write itself is unchanged, and a
/// global the script names with a capital stays its own.
#[test]
fn reg_032_writing_a_browser_global_does_not_unlock_reading_it() {
    let cases: &[(&str, &str, u32)] = &[
        (
            "try {\n  window.localStorage = 1;\n} catch {}\nexport const s = window.localStorage;\n",
            "localStorage",
            4,
        ),
        (
            "try {\n  globalThis.indexedDB = 1;\n} catch {}\nexport const s = self.indexedDB;\n",
            "indexedDB",
            4,
        ),
        (
            "try {\n  self[\"session\" + \"Storage\"] = 1;\n} catch {}\nexport const s = globalThis.sessionStorage;\n",
            "sessionStorage",
            4,
        ),
        (
            "Object.defineProperty(window, \"caches\", { value: 1 });\nexport const s = window.caches;\n",
            "caches",
            2,
        ),
        (
            "window.acmeState = {};\nexport const s = window.acmeState;\n",
            "acmeState",
            2,
        ),
    ];
    let mut failures = Vec::new();
    for (script, name, line) in cases {
        let report = scan_widget_script(script);
        let refused = report.findings.iter().any(|finding| {
            finding.check == "script-global"
                && finding.line == Some(*line)
                && finding.message.contains(&format!("`{name}`"))
        });
        if !refused {
            failures.push(format!(
                "{script:?}: the read of `{name}` at widget.js:{line} is not refused; got [{}]",
                report
                    .findings
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(" | ")
            ));
        }
    }
    let own = [
        "window.AcmeX = {};\nexport const x = (0, window).AcmeX;\n",
        "globalThis[\"Acme\" + \"Y\"] = {};\nexport const y = window.AcmeY;\n",
    ];
    for script in own {
        let report = scan_widget_script(script);
        if !report.accepted() {
            failures.push(format!(
                "{script:?} was refused: {}",
                report
                    .findings
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(" | ")
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// The admitted fixtures, under `accepted/`, that pin what the built-in
/// rule leaves open (REG-032): writes to the script's own objects and
/// functions, and the writes an ordinary component makes on the page, its
/// elements and the values it builds.
const BUILT_IN_RULE_ADMITS: &[&str] = &["own-members", "ordinary-writes"];

/// REG-032: writing a member of an object or function the script made stays
/// admitted, its own object, a parameter given one and its own function
/// included, and so do calling, reading, comparing and feature-testing a
/// built-in, handing a built-in function to a browser API that calls it
/// back, a constant such as `Number.MAX_SAFE_INTEGER` used as a value, and
/// the page's own properties (`document.title`, `location.hash`), and what an
/// ordinary component writes on its elements (`textContent`, `value`,
/// `this.state.open`).
#[test]
fn reg_032_a_scripts_own_members_and_uses_of_built_ins_stay_admitted() {
    let admitted = [
        "const o = {};\no.keys = () => [];\nexport { o };\n",
        "function f(p) {\n  p.keys = () => [];\n}\nf({});\n",
        "function mine() {}\nmine.cache = new Map();\n",
        "export const count = (o) => Object.keys(o).length;\n",
        "const keys = Object.keys;\nexport const count = (o) => keys(o).length;\n",
        "const k = Object;\nexport const n = (o) => k.keys(o).length;\n",
        "export const clean = (xs) => xs.map(Number).filter(Boolean);\n",
        "export const run = (xs) => xs.forEach(console.log);\n",
        "export const big = Math.max(1, Number.MAX_SAFE_INTEGER);\n",
        "export const label = (n) => `${Math.round(n)} of ${Number.MAX_SAFE_INTEGER}`;\n",
        "export const same = (x) => x === Object || typeof JSON === \"object\";\n",
        "if (Element.prototype.checkVisibility) {\n  document.title = \"x\";\n}\n",
        "location.hash = \"#a\";\n",
        "export const api = { open() {} };\napi.open.label = \"x\";\n",
        "export class A extends HTMLElement {\n  error = null;\n  connectedCallback() {\n    this.error = this.querySelector(\".e\");\n    if (this.error) this.error.hidden = true;\n  }\n}\n",
        "globalThis.AcmeLib = { version: 1 };\nglobalThis.AcmeLib.version = 2;\n",
        "export const failure = (result) => ({ error: result.error, next: result.next });\n",
        "export const take = (el) => [el.focus, el.values];\n",
        "export function clear(form) {\n  form.querySelector(\"[name=search]\").value = \"\";\n}\n",
        "function cache() {}\ncache.size = 0;\n",
    ];
    let mut failures = Vec::new();
    for script in admitted {
        let report = scan_widget_script(script);
        if !report.accepted() {
            failures.push(format!(
                "{script:?} was refused: {}",
                report
                    .findings
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(" | ")
            ));
        }
    }
    for name in BUILT_IN_RULE_ADMITS {
        let fixture = scan_fixture(&accepted().join(name));
        if !fixture.accepted() {
            failures.push(format!(
                "accepted/{name} was refused: {}",
                fixture
                    .findings
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(" | ")
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
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
