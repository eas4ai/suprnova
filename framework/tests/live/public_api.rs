//! The public `live` and `view` documentation never shows an internal crate
//! path, and no public signature an application reaches through `suprnova`,
//! its own or one of an engine item it re-exports, names a type of an
//! internal crate that an application cannot name through `suprnova`. The
//! checks read rustdoc's JSON output, never the HTML renderer, which climbs
//! past 64 GB of memory on this crate, and write only inside the checkout's
//! configured target directory.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

/// Crate names a reader of the public documentation must never see: the
/// application reaches these crates only through `suprnova`.
const FORBIDDEN: [&str; 3] = ["suprnova_live", "suprnova_live_macros", "askama_parser"];

/// Crates an application reaches only through `suprnova`, so a type of one
/// that a public signature names must be nameable through `suprnova`: the
/// crates the documentation never shows, and Magnetar, which an application
/// consumes through `suprnova::magnetar_integration` alone.
const REACHED_THROUGH_SUPRNOVA: [&str; 4] = [
    "suprnova_live",
    "suprnova_live_macros",
    "askama_parser",
    "magnetar",
];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("framework lives under workspace root")
        .to_path_buf()
}

/// The directory the JSON builds use, inside the checkout's configured
/// target directory, and each workspace library's package name by its crate
/// name.
///
/// The JSON renderer needs `RUSTC_BOOTSTRAP`, and build scripts such as
/// `proc-macro2`'s rerun when it changes, so sharing the test build's
/// directory would rebuild every dependency here and again in the next
/// ordinary build. A directory of its own stays warm.
fn workspace_metadata(root: &Path) -> (PathBuf, BTreeMap<String, String>) {
    let output = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(root)
        .output()
        .expect("run cargo metadata");
    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: Value =
        serde_json::from_slice(&output.stdout).expect("cargo metadata prints JSON");
    let target = metadata["target_directory"]
        .as_str()
        .expect("cargo metadata names the target directory");
    let mut packages = BTreeMap::new();
    for package in metadata["packages"].as_array().expect("packages") {
        let name = package["name"].as_str().expect("a package name");
        for target in package["targets"].as_array().expect("targets") {
            let library = target["kind"].as_array().is_some_and(|kinds| {
                kinds
                    .iter()
                    .any(|kind| matches!(kind.as_str(), Some("lib" | "proc-macro")))
            });
            if library {
                let crate_name = target["name"].as_str().expect("a target name");
                packages.insert(crate_name.replace('-', "_"), name.to_owned());
            }
        }
    }
    (
        PathBuf::from(target).join("live-public-api-rustdoc"),
        packages,
    )
}

/// Builds one workspace library's rustdoc JSON and reads it back.
fn rustdoc_json(root: &Path, target: &Path, package: &str, crate_name: &str) -> Value {
    build_rustdoc_json(root, target, package, crate_name, &[], &[])
}

/// The features `suprnova`'s own build enables on the workspace library
/// `package`, comma separated.
///
/// Built alone, a library takes its own default features, and Magnetar's
/// are not what `suprnova` asks of it: the JSON would document items the
/// framework never links, or miss items it does.
fn features_as_suprnova_builds(root: &Path, package: &str) -> String {
    let output = Command::new("cargo")
        .args([
            "tree",
            "-p",
            "suprnova",
            "-i",
            package,
            "-e",
            "normal,build",
        ])
        .args(["--depth", "0", "--prefix", "none", "--format", "{f}"])
        .current_dir(root)
        .output()
        .expect("run cargo tree");
    assert!(
        output.status.success(),
        "cargo tree failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("cargo tree prints UTF-8")
        .trim()
        .to_owned()
}

/// Builds the rustdoc JSON of a workspace library `suprnova` depends on,
/// with the features `suprnova` enables on it, and reads it back.
///
/// Lints are capped at warnings: under `suprnova`'s feature set a doc link
/// of that library may point at an item of a feature left off, which is the
/// library's own concern, and its JSON is read here for its signatures.
fn dependency_json(root: &Path, target: &Path, package: &str, crate_name: &str) -> Value {
    let features = features_as_suprnova_builds(root, package);
    build_rustdoc_json(
        root,
        target,
        package,
        crate_name,
        &["--no-default-features", "--features", &features],
        &["--cap-lints", "warn"],
    )
}

/// Builds one workspace library's rustdoc JSON with the given cargo and
/// rustdoc arguments, and reads it back.
fn build_rustdoc_json(
    root: &Path,
    target: &Path,
    package: &str,
    crate_name: &str,
    cargo_args: &[&str],
    rustdoc_args: &[&str],
) -> Value {
    let mut command = Command::new("cargo");
    command
        .args(["rustdoc", "-p", package, "--lib"])
        .args(cargo_args)
        .arg("--target-dir")
        .arg(target)
        .arg("--")
        .args(["-Z", "unstable-options", "--output-format", "json"])
        .args(rustdoc_args)
        .current_dir(root)
        .env("RUSTC_BOOTSTRAP", "1")
        .env("CARGO_INCREMENTAL", "0");
    if std::env::var_os("CARGO_BUILD_JOBS").is_none() {
        // The rest of the suite is running beside this build.
        command.env("CARGO_BUILD_JOBS", "1");
    }
    let status = command.status().expect("run cargo rustdoc");
    assert!(
        status.success(),
        "the rustdoc JSON build of {package} failed"
    );
    let path = target.join("doc").join(format!("{crate_name}.json"));
    let bytes = fs::read(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|error| panic!("{} is not JSON: {error}", path.display()))
}

/// Whether a fenced code block's info string makes it a Rust block, whose
/// hidden lines rustdoc leaves off the page.
fn is_rust_block(info: &str) -> bool {
    info.split(|c: char| c == ',' || c.is_whitespace())
        .filter(|token| !token.is_empty())
        .all(|token| {
            matches!(
                token,
                "rust"
                    | "ignore"
                    | "should_panic"
                    | "no_run"
                    | "compile_fail"
                    | "test_harness"
                    | "standalone_crate"
            ) || token.starts_with("edition")
                || token.starts_with("ignore-")
        })
}

/// A doc comment as rustdoc renders it: a Rust code block's hidden lines
/// (`# ` and a bare `#`) never reach the page, so a doctest's
/// `# use suprnova_live as suprnova;` shows nothing to a reader.
fn rendered_docs(docs: &str) -> String {
    let mut rendered = String::with_capacity(docs.len());
    // The open fence: its character, its length and whether it is Rust.
    let mut open: Option<(char, usize, bool)> = None;
    for line in docs.lines() {
        let trimmed = line.trim_start();
        let fence = ['`', '~'].into_iter().find_map(|mark| {
            let length = trimmed.chars().take_while(|c| *c == mark).count();
            (length >= 3).then_some((mark, length))
        });
        match (open, fence) {
            (None, Some((mark, length))) => {
                open = Some((mark, length, is_rust_block(&trimmed[length..])));
            }
            (Some((mark, length, _)), Some((closing, closing_length)))
                if closing == mark
                    && closing_length >= length
                    && trimmed[closing_length..].trim().is_empty() =>
            {
                open = None;
            }
            (Some((_, _, true)), _) if trimmed == "#" || trimmed.starts_with("# ") => continue,
            _ => {}
        }
        rendered.push_str(line);
        rendered.push('\n');
    }
    rendered
}

/// The forbidden crate names a reader would see in `text`.
/// `__suprnova_live` is the `LiveComponent` derive's helper attribute, which
/// the `#[live]` macro writes, not a crate path, so it is read past.
fn exposed(text: &str) -> Vec<&'static str> {
    let visible = text.replace("__suprnova_live", "");
    FORBIDDEN
        .into_iter()
        .filter(|name| visible.contains(name))
        .collect()
}

fn ids(value: &Value) -> impl Iterator<Item = u64> + '_ {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_u64)
}

/// The items a page shows as part of an item: a module's items, a type's
/// fields, variants and impls, and an impl's or a trait's items. A trait's
/// implementors are listed by their headers alone, so they are not.
fn shown_with(inner: &Value, found: &mut Vec<u64>) {
    match inner {
        Value::Object(object) => {
            for (key, value) in object {
                match key.as_str() {
                    "items" | "impls" | "fields" | "variants" | "tuple" => {
                        found.extend(ids(value));
                    }
                    _ => {}
                }
                shown_with(value, found);
            }
        }
        Value::Array(values) => {
            for value in values {
                shown_with(value, found);
            }
        }
        _ => {}
    }
}

/// One crate's rustdoc JSON.
struct Crate<'a> {
    name: &'a str,
    json: &'a Value,
}

impl<'a> Crate<'a> {
    fn item(&self, id: u64) -> Option<&'a Value> {
        self.json["index"].get(id.to_string())
    }

    fn path(&self, id: u64) -> Option<Vec<&'a str>> {
        let summary = self.json["paths"].get(id.to_string())?;
        summary["path"]
            .as_array()
            .map(|segments| segments.iter().filter_map(Value::as_str).collect())
    }

    fn crate_of(&self, id: u64) -> Option<&'a str> {
        let crate_id = self.json["paths"].get(id.to_string())?["crate_id"].as_u64()?;
        if crate_id == 0 {
            return Some(self.name);
        }
        self.json["external_crates"][crate_id.to_string()]["name"].as_str()
    }

    fn root(&self) -> u64 {
        self.json["root"].as_u64().expect("the rustdoc root id")
    }

    fn module_items(&self, id: u64) -> Vec<u64> {
        self.item(id)
            .map(|item| ids(&item["inner"]["module"]["items"]).collect())
            .unwrap_or_default()
    }

    fn named(&self, module: u64, name: &str) -> Option<u64> {
        self.module_items(module).into_iter().find_map(|id| {
            let item = self.item(id)?;
            if let Some(re_export) = item["inner"].get("use") {
                if re_export["name"].as_str() == Some(name) {
                    return re_export["id"]
                        .as_u64()
                        .filter(|id| self.item(*id).is_some());
                }
                return None;
            }
            (item["name"].as_str() == Some(name)).then_some(id)
        })
    }

    /// The item at `path` in this crate: through rustdoc's path table, or
    /// by walking its modules from the root, which finds a derive macro the
    /// path table names under another id.
    fn resolve(&self, path: &[&str]) -> Option<u64> {
        let by_table = self.json["paths"]
            .as_object()?
            .iter()
            .filter(|(_, summary)| {
                summary["crate_id"].as_u64() == Some(0)
                    && summary["path"].as_array().is_some_and(|segments| {
                        segments
                            .iter()
                            .map(Value::as_str)
                            .eq(path.iter().copied().map(Some))
                    })
            })
            .filter_map(|(id, _)| id.parse::<u64>().ok())
            .find(|id| self.item(*id).is_some());
        if by_table.is_some() {
            return by_table;
        }
        let (crate_name, segments) = path.split_first()?;
        if *crate_name != self.name {
            return None;
        }
        segments
            .iter()
            .try_fold(self.root(), |module, segment| self.named(module, segment))
    }

    /// The ids rustdoc's path table records at `path` in this crate, with
    /// their kinds, whether or not the crate exports them.
    fn path_entries(&self, path: &[&str]) -> Vec<(u64, &'a str)> {
        let Some(table) = self.json["paths"].as_object() else {
            return Vec::new();
        };
        table
            .iter()
            .filter(|(_, summary)| {
                summary["crate_id"].as_u64() == Some(0)
                    && summary["path"].as_array().is_some_and(|segments| {
                        segments
                            .iter()
                            .map(Value::as_str)
                            .eq(path.iter().copied().map(Some))
                    })
            })
            .filter_map(|(id, summary)| Some((id.parse().ok()?, summary["kind"].as_str()?)))
            .collect()
    }

    /// The ids an application reaches from this crate's root: the items of
    /// its public modules, and what they re-export, globs included.
    fn exported(&self) -> BTreeSet<u64> {
        let mut exported = BTreeSet::new();
        let mut pending = vec![self.root()];
        while let Some(id) = pending.pop() {
            if !exported.insert(id) {
                continue;
            }
            let Some(item) = self.item(id) else { continue };
            match item["inner"].get("use") {
                Some(re_export) => {
                    if let Some(target) = re_export["id"].as_u64() {
                        if re_export["is_glob"].as_bool() == Some(true) {
                            pending.extend(self.module_items(target));
                        } else {
                            pending.push(target);
                        }
                    }
                }
                None => pending.extend(self.module_items(id)),
            }
        }
        exported
    }
}

/// What a reader of the documentation would see that names a forbidden
/// crate, starting at `start` in `docs`. A re-export from another workspace
/// crate is followed into that crate's JSON, as rustdoc's HTML inlines it.
fn check(
    docs: &Crate<'_>,
    start: u64,
    others: &BTreeMap<String, Value>,
    seen: &mut BTreeSet<(String, u64)>,
    exposures: &mut Vec<String>,
) {
    let mut pending = vec![start];
    while let Some(id) = pending.pop() {
        if !seen.insert((docs.name.to_owned(), id)) {
            continue;
        }
        let Some(item) = docs.item(id) else { continue };
        if item["crate_id"].as_u64() != Some(0) {
            continue;
        }
        let name = item["name"].as_str().unwrap_or_default();
        if name == "__private" {
            continue;
        }
        let local = docs.name == "suprnova";
        let mut shown = vec![
            ("name", name.to_owned()),
            (
                "docs",
                rendered_docs(item["docs"].as_str().unwrap_or_default()),
            ),
        ];
        if local && let Some(path) = docs.path(id) {
            shown.push(("path", path.join("::")));
        }
        if let Some(re_export) = item["inner"].get("use") {
            // The name the item is shown under.
            shown.push((
                "re-exported name",
                re_export["name"].as_str().unwrap_or_default().to_owned(),
            ));
            let target = re_export["id"].as_u64();
            let inlined = !serde_json::to_string(&item["attrs"])
                .expect("attributes serialize")
                .contains("no_inline");
            // An unresolved or `no_inline` re-export prints its source path.
            if target.is_none() || !inlined {
                shown.push((
                    "re-export",
                    re_export["source"].as_str().unwrap_or_default().to_owned(),
                ));
            }
            if let Some(target) = target {
                if docs
                    .item(target)
                    .is_some_and(|item| item["crate_id"].as_u64() == Some(0))
                {
                    pending.push(target);
                } else if inlined
                    && let Some(owner) = docs.crate_of(target)
                    && let Some(json) = others.get(owner)
                {
                    let other = Crate { name: owner, json };
                    let path = docs.path(target).unwrap_or_default();
                    match other.resolve(&path) {
                        Some(found) => check(&other, found, others, seen, exposures),
                        None => exposures.push(format!(
                            "the re-export of {} names nothing in {owner}'s rustdoc JSON, so its documentation went unread",
                            path.join("::")
                        )),
                    }
                }
            }
        } else {
            let mut nested = Vec::new();
            shown_with(&item["inner"], &mut nested);
            pending.extend(nested);
        }
        for (what, text) in shown {
            for forbidden in exposed(&text) {
                exposures.push(format!(
                    "{} item `{name}` ({}) shows {forbidden} in its {what}",
                    docs.name,
                    item["span"]["filename"].as_str().unwrap_or("no source"),
                ));
            }
        }
    }
}

#[test]
fn rendered_public_docs_do_not_expose_internal_crate_paths() {
    let root = workspace_root();
    let (target, packages) = workspace_metadata(&root);
    let framework = rustdoc_json(&root, &target, "suprnova", "suprnova");
    let docs = Crate {
        name: "suprnova",
        json: &framework,
    };
    let modules: Vec<u64> = docs
        .module_items(docs.root())
        .into_iter()
        .filter(|id| {
            docs.item(*id).is_some_and(|item| {
                item["inner"].get("module").is_some()
                    && matches!(item["name"].as_str(), Some("live" | "view"))
            })
        })
        .collect();
    assert_eq!(modules.len(), 2, "the crate root holds `live` and `view`");

    // The workspace crates whose items `live` and `view` re-export, whose
    // documentation rustdoc's HTML inlines there. A third-party crate's
    // documentation is not this repository's to keep clean.
    let mut re_exported = BTreeSet::new();
    let mut pending = modules.clone();
    while let Some(id) = pending.pop() {
        for child in docs.module_items(id) {
            let Some(item) = docs.item(child) else {
                continue;
            };
            if item["inner"].get("module").is_some() {
                pending.push(child);
            } else if let Some(target) = item["inner"]["use"]["id"].as_u64()
                && let Some(owner) = docs.crate_of(target)
                && owner != "suprnova"
                && packages.contains_key(owner)
            {
                re_exported.insert(owner.to_owned());
            }
        }
    }
    assert!(
        re_exported.contains("suprnova_live"),
        "`live` re-exports the Live engine's types: {re_exported:?}"
    );
    let others: BTreeMap<String, Value> = re_exported
        .iter()
        .map(|crate_name| {
            let json = dependency_json(&root, &target, &packages[crate_name], crate_name);
            (crate_name.clone(), json)
        })
        .collect();

    let mut seen = BTreeSet::new();
    let mut exposures = Vec::new();
    for module in modules {
        check(&docs, module, &others, &mut seen, &mut exposures);
    }
    assert!(
        seen.len() > 1000,
        "the walk read only {} items, so it missed the documentation",
        seen.len()
    );
    assert!(
        exposures.is_empty(),
        "public documentation exposed internal paths:\n{}",
        exposures.join("\n")
    );
}

/// The types the public signatures an application reaches name, and what
/// those crates re-export from other crates.
#[derive(Default)]
struct Signatures {
    /// Each named type of a crate an application reaches only through
    /// `suprnova`, by its canonical path, with the public items whose
    /// signatures name it.
    named: BTreeMap<Vec<String>, BTreeSet<String>>,
    /// The canonical paths of the other crates' items re-exported.
    re_exported: BTreeSet<Vec<String>>,
    /// The canonical paths of the other crates' modules re-exported whole,
    /// through a module re-export or a glob.
    re_exported_modules: Vec<Vec<String>>,
    /// The re-exported items of a crate an application reaches only through
    /// `suprnova` whose own signatures an application reaches too: every one
    /// but a macro, which has none.
    to_follow: BTreeSet<Vec<String>>,
}

impl Signatures {
    /// Whether an application reaches the item at `path` through the crate:
    /// the crate re-exports it, or a module it sits under.
    fn reaches(&self, path: &[String]) -> bool {
        self.re_exported.contains(path)
            || self
                .re_exported_modules
                .iter()
                .any(|module| path.len() > module.len() && path.starts_with(module))
    }
}

/// The ids of every type `value` names: a resolved path, or a trait in a
/// bound, a `dyn` type or an `impl` header.
fn named_types(value: &Value, found: &mut Vec<u64>) {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                let names_a_type = matches!(key.as_str(), "resolved_path" | "trait")
                    && value.get("path").is_some();
                if names_a_type && let Some(id) = value["id"].as_u64() {
                    found.push(id);
                }
                named_types(value, found);
            }
        }
        Value::Array(values) => {
            for value in values {
                named_types(value, found);
            }
        }
        _ => {}
    }
}

/// Walks every item an application reaches from `starts` in `docs`:
/// modules, re-exports, items and their fields, variants, methods and the
/// impls written in that crate. A blanket impl and an auto-trait impl
/// rustdoc adds are not, as no one wrote their signatures there.
fn walk(docs: &Crate<'_>, starts: Vec<(u64, String)>, found: &mut Signatures) {
    let mut seen = BTreeSet::new();
    let mut pending = starts;
    while let Some((id, owner)) = pending.pop() {
        if !seen.insert(id) {
            continue;
        }
        let Some(item) = docs.item(id) else { continue };
        if item["crate_id"].as_u64() != Some(0) {
            continue;
        }
        let inner = &item["inner"];
        if let Some(re_export) = inner.get("use") {
            let Some(target) = re_export["id"].as_u64() else {
                continue;
            };
            let glob = re_export["is_glob"].as_bool() == Some(true);
            if docs
                .item(target)
                .is_some_and(|item| item["crate_id"].as_u64() == Some(0))
            {
                if glob {
                    let items = docs.module_items(target);
                    pending.extend(items.into_iter().map(|child| (child, owner.clone())));
                } else {
                    pending.push((target, owner));
                }
            } else if let Some(path) = docs.path(target) {
                let path: Vec<String> = path.into_iter().map(str::to_owned).collect();
                let kind = docs.json["paths"][target.to_string()]["kind"].as_str();
                if glob || kind == Some("module") {
                    found.re_exported_modules.push(path.clone());
                }
                let macro_kind = matches!(kind, Some("macro" | "proc_attribute" | "proc_derive"));
                if !macro_kind
                    && path
                        .first()
                        .is_some_and(|name| REACHED_THROUGH_SUPRNOVA.contains(&name.as_str()))
                {
                    found.to_follow.insert(path.clone());
                }
                found.re_exported.insert(path);
            }
            continue;
        }
        if inner.get("impl").is_some_and(|implementation| {
            !implementation["blanket_impl"].is_null()
                || implementation["is_synthetic"].as_bool() == Some(true)
        }) {
            continue;
        }
        // An impl is named after the type it is for; a method or a field
        // after its owner, as neither has a path of its own.
        let name = match docs.path(id) {
            Some(path) => path.join("::"),
            None if inner.get("impl").is_some() => owner,
            None => format!("{owner}::{}", item["name"].as_str().unwrap_or_default()),
        };
        let mut types = Vec::new();
        named_types(inner, &mut types);
        for named in types {
            let reached_only_through_suprnova = docs
                .crate_of(named)
                .is_some_and(|owner| REACHED_THROUGH_SUPRNOVA.contains(&owner));
            if reached_only_through_suprnova && let Some(path) = docs.path(named) {
                let path = path.into_iter().map(str::to_owned).collect();
                found.named.entry(path).or_default().insert(name.clone());
            }
        }
        let mut nested = Vec::new();
        shown_with(inner, &mut nested);
        pending.extend(nested.into_iter().map(|child| (child, name.clone())));
    }
}

/// An application names a type through the crate root (the house rule), so
/// a public signature that names a type of an internal crate the crate
/// does not re-export hands the application a value whose type it cannot
/// write down, and the rendered signature links into the internal crate.
/// The compiler warns about a private type of this crate in a public
/// signature, never about a public type of a dependency the application
/// cannot reach, so this reads the signatures.
///
/// The walk is transitive: an engine item `suprnova` re-exports is the
/// application's too, so its own public signatures (its fields, its
/// methods, its trait items and the impls its crate writes) are read in
/// that crate's rustdoc JSON, and so on for every engine item they lead to.
#[test]
fn public_signatures_name_only_types_an_application_can_reach() {
    let root = workspace_root();
    let (target, packages) = workspace_metadata(&root);
    let framework = rustdoc_json(&root, &target, "suprnova", "suprnova");
    let docs = Crate {
        name: "suprnova",
        json: &framework,
    };
    let mut signatures = Signatures::default();
    walk(
        &docs,
        vec![(docs.root(), "suprnova".to_owned())],
        &mut signatures,
    );

    // Follow every re-exported engine item into its own crate's JSON,
    // until the items it leads to are all read.
    let mut engines: BTreeMap<String, Value> = BTreeMap::new();
    let mut followed = BTreeSet::new();
    let mut unread = Vec::new();
    loop {
        let next: Vec<Vec<String>> = signatures
            .to_follow
            .difference(&followed)
            .cloned()
            .collect();
        if next.is_empty() {
            break;
        }
        for path in next {
            followed.insert(path.clone());
            let owner = path[0].clone();
            let Some(package) = packages.get(&owner) else {
                unread.push(format!(
                    "{} is re-exported from {owner}, which is not a workspace library, so its signatures went unread",
                    path.join("::")
                ));
                continue;
            };
            let json = engines
                .entry(owner.clone())
                .or_insert_with(|| dependency_json(&root, &target, package, &owner));
            let engine = Crate { name: &owner, json };
            let segments: Vec<&str> = path.iter().map(String::as_str).collect();
            match engine.resolve(&segments) {
                Some(id) => walk(&engine, vec![(id, path.join("::"))], &mut signatures),
                None => unread.push(format!(
                    "the re-export of {} names nothing in {owner}'s rustdoc JSON, so its signatures went unread",
                    path.join("::")
                )),
            }
        }
    }
    assert!(
        followed.len() > 100,
        "the walk followed only {} re-exported engine items, so it missed the re-exports",
        followed.len()
    );

    let exported: BTreeMap<&str, BTreeSet<u64>> = engines
        .iter()
        .map(|(name, json)| (name.as_str(), Crate { name, json }.exported()))
        .collect();
    let mut checked = 0;
    let mut unreachable = Vec::new();
    for (path, users) in &signatures.named {
        let owner = path[0].as_str();
        // A trait its own crate keeps private, the supertrait that seals a
        // trait, is one no one outside may name or implement, by design.
        // A trait named only by the framework's own Live components is the
        // component macro's contract: a shipped component implements it
        // through the macro exactly as an application's component does,
        // through `live::__private`, which no application names, so that
        // impl is the macro's and not surface an application can use.
        if let (Some(json), Some(exported)) = (engines.get(owner), exported.get(owner)) {
            let engine = Crate { name: owner, json };
            let segments: Vec<&str> = path.iter().map(String::as_str).collect();
            let entries = engine.path_entries(&segments);
            let all_traits =
                !entries.is_empty() && entries.iter().all(|(_, kind)| *kind == "trait");
            let sealed = all_traits && entries.iter().all(|(id, _)| !exported.contains(id));
            let macro_contract = all_traits
                && users
                    .iter()
                    .all(|user| user.starts_with("suprnova::live::components::"));
            if sealed || macro_contract {
                continue;
            }
        }
        checked += 1;
        if !signatures.reaches(path) {
            let users: Vec<&str> = users.iter().map(String::as_str).collect();
            unreachable.push(format!(
                "{} is named by {}",
                path.join("::"),
                users.join(", ")
            ));
        }
    }
    assert!(
        unread.is_empty() && unreachable.is_empty(),
        "public signatures name types an application cannot name through `suprnova` ({} of {checked}):\n{}",
        unreachable.len(),
        unread
            .iter()
            .chain(&unreachable)
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join("\n")
    );
    // `suprnova`'s own signatures name fewer than 200 engine types and the
    // engine items they lead to name the rest, so a floor above that shows
    // the walk followed them.
    assert!(
        checked > 400,
        "the walk found only {checked} engine types in public signatures, so it missed the signatures or the engine items they lead to"
    );
}
