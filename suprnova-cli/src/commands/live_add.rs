//! `suprnova live:add` - install a Live component library component into the
//! application from its JSON manifest (Cairn UI-017).
//!
//! A component is one directory holding its view, its stylesheet when it has
//! one, its JavaScript when it has one, and the manifest that names them. The
//! shipped library is embedded in this binary; `--manifest` installs a
//! third-party component in the same format from a directory on disk. Every
//! file lands under `templates/<root>/`; a file the application has edited is
//! kept, and `--force` is the only way past that.

use std::fs;
use std::path::{Path, PathBuf};

use crate::secure_fs;
use crate::ui;

/// The template root the shipped library installs under; a third-party
/// manifest may not claim it.
pub const RESERVED_ROOT: &str = "suprnova-ui";

/// One component of the shipped library, embedded at build time.
pub struct EmbeddedComponent {
    /// The component directory name, which is also the `live:add` argument.
    pub directory: &'static str,
    /// The manifest's bytes.
    pub manifest: &'static str,
    /// Every file beside the manifest, by name.
    pub files: &'static [(&'static str, &'static str)],
}

include!(concat!(env!("OUT_DIR"), "/live_components.rs"));

/// The parsed manifest of a component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    /// The component name, `suprnova.<directory>` for the shipped library.
    pub name: String,
    /// The manifest version.
    pub version: u64,
    /// The install root under `templates/`, for example `suprnova-ui/field`.
    pub root: String,
    /// The files the component needs, by name.
    pub files: Vec<String>,
    /// The custom element tags the component's JavaScript defines.
    pub elements: Vec<String>,
}

impl Manifest {
    /// Parses a manifest, refusing anything that could escape its root.
    pub fn parse(source: &str) -> Result<Self, String> {
        let value: serde_json::Value = serde_json::from_str(source)
            .map_err(|error| format!("manifest is not JSON: {error}"))?;
        let object = value.as_object().ok_or("manifest is not a JSON object")?;
        let name = object
            .get("name")
            .and_then(|value| value.as_str())
            .filter(|name| !name.is_empty())
            .ok_or("manifest has no name")?
            .to_owned();
        let version = object
            .get("version")
            .and_then(|value| value.as_u64())
            .ok_or("manifest has no integer version")?;
        let root = object
            .get("root")
            .and_then(|value| value.as_str())
            .ok_or("manifest has no root")?
            .to_owned();
        if !valid_root(&root) {
            return Err(format!(
                "manifest root `{root}` is not a closed relative path"
            ));
        }
        let files = string_list(object.get("files"), "files")?;
        if files.is_empty() {
            return Err("manifest names no files".to_owned());
        }
        for file in &files {
            if !valid_file_name(file) {
                return Err(format!(
                    "manifest file `{file}` is not a single closed file name"
                ));
            }
        }
        let elements = string_list(object.get("elements"), "elements")?;
        Ok(Self {
            name,
            version,
            root,
            files,
            elements,
        })
    }
}

fn string_list(value: Option<&serde_json::Value>, field: &str) -> Result<Vec<String>, String> {
    match value {
        None => Ok(Vec::new()),
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| format!("manifest {field} entries must be strings"))
            })
            .collect(),
        Some(_) => Err(format!("manifest {field} is not a list")),
    }
}

fn valid_root(root: &str) -> bool {
    !root.is_empty()
        && root.len() <= 128
        && !root.starts_with('/')
        && !root.ends_with('/')
        && root.split('/').all(|segment| {
            !segment.is_empty()
                && segment
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        })
}

fn valid_file_name(file: &str) -> bool {
    !file.is_empty()
        && file.len() <= 128
        && !file.contains('/')
        && !file.contains('\\')
        && !file.starts_with('.')
        && file.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'.')
        })
}

/// A component ready to install: its manifest and the bytes of each file.
struct Source {
    manifest: Manifest,
    manifest_source: String,
    files: Vec<(String, String)>,
}

/// What one file's installation did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The file did not exist and was written.
    Written,
    /// The file existed with the same bytes.
    Unchanged,
    /// The file existed with different bytes and was left alone.
    Kept,
    /// The file existed with different bytes and `--force` replaced it.
    Replaced,
}

/// Runs `live:add`.
pub fn run(name: Option<String>, manifest: Option<PathBuf>, force: bool, dry_run: bool) {
    if let Err(error) = run_inner(name.as_deref(), manifest.as_deref(), force, dry_run) {
        ui::error(&error);
        std::process::exit(1);
    }
}

fn run_inner(
    name: Option<&str>,
    manifest: Option<&Path>,
    force: bool,
    dry_run: bool,
) -> Result<(), String> {
    if !Path::new("Cargo.toml").exists() {
        ui::hint("Make sure you're in a Suprnova project root directory.");
        return Err(
            "No Cargo.toml found in the current directory (project root expected)".to_owned(),
        );
    }
    let source = match (name, manifest) {
        (Some(name), None) => embedded(name)?,
        (None, Some(manifest)) => third_party(manifest)?,
        (Some(_), Some(_)) => {
            return Err("pass a library component name or --manifest, not both".to_owned());
        }
        (None, None) => {
            ui::hint(&format!(
                "Shipped components: {}",
                COMPONENTS
                    .iter()
                    .map(|component| component.directory)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
            return Err("pass a library component name or --manifest <path>".to_owned());
        }
    };
    let outcomes = install(&source, Path::new("."), force, dry_run)?;
    let action = if dry_run {
        "would install"
    } else {
        "installed"
    };
    ui::success(&format!(
        "{action} {} under templates/{}",
        source.manifest.name, source.manifest.root
    ));
    for (file, outcome) in outcomes {
        let word = match outcome {
            Outcome::Written => "written",
            Outcome::Unchanged => "unchanged",
            Outcome::Kept => "kept, edited locally (pass --force to replace)",
            Outcome::Replaced => "replaced",
        };
        ui::label_value(&file, word);
    }
    Ok(())
}

fn embedded(name: &str) -> Result<Source, String> {
    let directory = name.strip_prefix("suprnova.").unwrap_or(name);
    let component = COMPONENTS
        .iter()
        .find(|component| component.directory == directory)
        .ok_or_else(|| {
            ui::hint(&format!(
                "Shipped components: {}",
                COMPONENTS
                    .iter()
                    .map(|component| component.directory)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
            format!("`{name}` is not a shipped library component")
        })?;
    let manifest = Manifest::parse(component.manifest)?;
    let mut files = Vec::new();
    for file in &manifest.files {
        let (_, content) = component
            .files
            .iter()
            .find(|(candidate, _)| candidate == file)
            .ok_or_else(|| format!("shipped component {directory} lacks {file}"))?;
        files.push((file.clone(), (*content).to_owned()));
    }
    Ok(Source {
        manifest,
        manifest_source: component.manifest.to_owned(),
        files,
    })
}

fn third_party(path: &Path) -> Result<Source, String> {
    let manifest_source = fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let manifest = Manifest::parse(&manifest_source)?;
    if manifest.root == RESERVED_ROOT || manifest.root.starts_with(&format!("{RESERVED_ROOT}/")) {
        return Err(format!(
            "the `{RESERVED_ROOT}/` template root is reserved for the shipped library; a third-party component installs under its own root"
        ));
    }
    if manifest.name.starts_with("suprnova.") {
        return Err(
            "the `suprnova.` component namespace is reserved for the shipped library".to_owned(),
        );
    }
    let directory = path.parent().unwrap_or(Path::new("."));
    let mut files = Vec::new();
    for file in &manifest.files {
        let file_path = directory.join(file);
        let content = fs::read_to_string(&file_path)
            .map_err(|error| format!("cannot read {}: {error}", file_path.display()))?;
        files.push((file.clone(), content));
    }
    Ok(Source {
        manifest,
        manifest_source,
        files,
    })
}

fn install(
    source: &Source,
    project: &Path,
    force: bool,
    dry_run: bool,
) -> Result<Vec<(String, Outcome)>, String> {
    let target = project.join("templates").join(&source.manifest.root);
    secure_fs::ensure_contained(project, &target)?;
    let mut planned: Vec<(String, String)> = Vec::with_capacity(source.files.len() + 1);
    planned.push(("manifest.json".to_owned(), source.manifest_source.clone()));
    planned.extend(source.files.iter().cloned());
    let mut outcomes = Vec::with_capacity(planned.len());
    for (file, content) in &planned {
        let path = target.join(file);
        secure_fs::ensure_contained(project, &path)?;
        let outcome = match fs::read_to_string(&path) {
            Ok(existing) if existing == *content => Outcome::Unchanged,
            Ok(_) if !force => Outcome::Kept,
            Ok(_) => Outcome::Replaced,
            Err(_) => Outcome::Written,
        };
        if !dry_run && matches!(outcome, Outcome::Written | Outcome::Replaced) {
            fs::create_dir_all(&target)
                .map_err(|error| format!("cannot create {}: {error}", target.display()))?;
            secure_fs::write_atomic(&path, content.as_bytes())?;
        }
        outcomes.push((
            format!("templates/{}/{file}", source.manifest.root),
            outcome,
        ));
    }
    Ok(outcomes)
}

#[cfg(test)]
mod tests {
    use super::{COMPONENTS, Manifest, valid_file_name, valid_root};

    #[test]
    fn every_shipped_component_has_a_parseable_manifest_that_names_its_files() {
        assert!(!COMPONENTS.is_empty());
        for component in COMPONENTS {
            let manifest = Manifest::parse(component.manifest).expect(component.directory);
            assert_eq!(manifest.name, format!("suprnova.{}", component.directory));
            assert_eq!(
                manifest.root,
                format!("suprnova-ui/{}", component.directory)
            );
            for file in &manifest.files {
                assert!(
                    component.files.iter().any(|(name, _)| name == file),
                    "{} names {file} but does not ship it",
                    component.directory
                );
            }
        }
    }

    #[test]
    fn roots_and_file_names_are_closed() {
        assert!(valid_root("suprnova-ui/field"));
        assert!(valid_root("acme-ui/widget"));
        for hostile in ["", "/x", "x/", "../x", "a//b", "A/b", "x/..", "sup rnova"] {
            assert!(!valid_root(hostile), "{hostile}");
        }
        assert!(valid_file_name("field.html"));
        for hostile in ["", "a/b", "../x", ".hidden", "Field.html", "x\\y"] {
            assert!(!valid_file_name(hostile), "{hostile}");
        }
    }
}
