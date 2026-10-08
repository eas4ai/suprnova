//! The public `live` and `view` documentation never shows an internal crate
//! path. The check reads rustdoc's JSON output, never the HTML renderer,
//! which climbs past 64 GB of memory on this crate, and writes only inside
//! the checkout's configured target directory.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

/// Crate names a reader of the public documentation must never see: the
/// application reaches these crates only through `suprnova`.
const FORBIDDEN: [&str; 3] = ["suprnova_live", "suprnova_live_macros", "askama_parser"];

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
    let mut command = Command::new("cargo");
    command
        .args(["rustdoc", "-p", package, "--lib", "--target-dir"])
        .arg(target)
        .arg("--")
        .args(["-Z", "unstable-options", "--output-format", "json"])
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
            let json = rustdoc_json(&root, &target, &packages[crate_name], crate_name);
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
