//! `suprnova live:add` - install a Live component into the application from
//! the shipped library or a third-party library (UI-017, REG-001 to
//! REG-029).
//!
//! A source is a shipped name, `[<host>/]<owner>/<library>/<component>`, an
//! `https://` URL of a component directory, or a path to one on disk
//! (`--manifest <file>` is a spelling of the last). Under the project lock,
//! `live:add` restores any journal an interrupted install left, resolves
//! the whole plan (fetching at a tag, verifying each signature and scanning
//! every file as data, dependencies included), shows it, takes the
//! developer's decisions, and only then writes, journal first. It starts no
//! process (REG-015): nothing a component carries runs.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::registry::fetch::SourceFetcher;
use crate::registry::plan::{self, Options, TerminalPrompter};
use crate::registry::project::{self, Journal, ProjectFile, ProjectLock};
use crate::registry::{Capability, address, install};
use crate::ui;

// The shipped library lives with the registry, which reads it; it is
// named here as well because `live:add` is where it was always found.
pub use crate::registry::fetch::COMPONENTS;

/// What `live:add` was asked to do, as the command line gave it.
#[derive(Debug, Clone, Default)]
pub struct Request {
    /// The source: a shipped name, an address, a URL or a path.
    pub name: Option<String>,
    /// A path to a component's `manifest.json`, a spelling of a path source.
    pub manifest: Option<PathBuf>,
    /// Replace edited files and accept downgrades and moved tags.
    pub force: bool,
    /// Report the plan and write nothing.
    pub dry_run: bool,
    /// Confirm the plan without a terminal.
    pub yes: bool,
    /// Capabilities approved on the command line.
    pub allow: Vec<String>,
}

/// Runs `live:add`.
pub fn run(request: Request) {
    if let Err(error) = run_inner(&request) {
        ui::error(&error);
        std::process::exit(1);
    }
}

fn shipped_names() -> String {
    COMPONENTS
        .iter()
        .map(|component| component.directory)
        .collect::<Vec<_>>()
        .join(", ")
}

/// The source text a request names: the positional source, or the
/// `--manifest` path with `./` prepended when it has no prefix.
fn source_spec(request: &Request) -> Result<String, String> {
    match (&request.name, &request.manifest) {
        (Some(name), None) => Ok(name.clone()),
        (None, Some(manifest)) => {
            let text = manifest
                .to_str()
                .ok_or_else(|| format!("{} is not a UTF-8 path", manifest.display()))?;
            if text.starts_with("./")
                || text.starts_with("../")
                || text.starts_with('/')
                || manifest.is_absolute()
            {
                Ok(text.to_owned())
            } else {
                Ok(format!("./{text}"))
            }
        }
        (Some(_), Some(_)) => Err("pass a component source or --manifest, not both".to_owned()),
        (None, None) => {
            ui::hint(&format!("Shipped components: {}", shipped_names()));
            Err("pass a component: a shipped name, <owner>/<library>/<component>, an https:// URL, a ./path, or --manifest <path>".to_owned())
        }
    }
}

fn run_inner(request: &Request) -> Result<(), String> {
    if !Path::new("Cargo.toml").exists() {
        ui::hint("Make sure you're in a Suprnova project root directory.");
        return Err(
            "No Cargo.toml found in the current directory (project root expected)".to_owned(),
        );
    }
    let root = std::env::current_dir()
        .map_err(|error| format!("cannot read the working directory: {error}"))?;
    project::refuse_legacy_project_file(&root).map_err(|error| error.to_string())?;
    let spec = source_spec(request)?;
    let source = address::parse(&spec).map_err(|error| error.to_string())?;
    let mut allow = BTreeSet::new();
    for name in &request.allow {
        let capability = Capability::parse(name).ok_or_else(|| {
            format!(
                "`{name}` is not a capability; the capabilities are {}",
                Capability::ALL.map(Capability::name).join(", ")
            )
        })?;
        allow.insert(capability);
    }
    let options = Options {
        force: request.force,
        dry_run: request.dry_run,
        yes: request.yes,
        allow,
    };
    let fetcher = SourceFetcher::default();

    if options.dry_run {
        // A dry run writes nothing, the lock and the journal included
        // (REG-014), so it asks whether an install holds the lock instead of
        // taking it, and leaves a journal for an install to restore.
        if ProjectLock::is_held(&root) {
            return Err("another live:add is installing into this project; run the dry run when it finishes".to_owned());
        }
        if Journal::exists(&root) {
            return Err(format!(
                "an interrupted live:add left {}; run live:add without --dry-run, or suprnova serve, to restore it first",
                project::JOURNAL_FILE
            ));
        }
        let project = ProjectFile::load(&root).map_err(|error| error.to_string())?;
        let plan = plan::resolve(&source, &options, &fetcher, &project)
            .map_err(|error| error.to_string())?;
        print!("{}", plan::render_with(&plan, &options, &project));
        ui::info("dry run: nothing was written");
        return Ok(());
    }

    let lock = ProjectLock::acquire(&root).map_err(|error| error.to_string())?;
    if Journal::restore(&root).map_err(|error| error.to_string())? {
        ui::warning(&format!(
            "an interrupted live:add left {}; every file it named was restored first",
            project::JOURNAL_FILE
        ));
    }
    let mut project = ProjectFile::load(&root).map_err(|error| error.to_string())?;
    let plan =
        plan::resolve(&source, &options, &fetcher, &project).map_err(|error| error.to_string())?;
    print!("{}", plan::render_with(&plan, &options, &project));
    let decisions = plan::confirm(&plan, &options, &project, &mut TerminalPrompter)
        .map_err(|error| error.to_string())?;
    let outcomes = install::apply_with(
        &plan,
        &mut project,
        &options,
        &decisions,
        &install::registration_edits,
    )
    .map_err(|error| error.to_string())?;
    lock.release().map_err(|error| error.to_string())?;
    for component in &plan.components {
        ui::success(&format!(
            "installed {} under {}",
            component.address,
            component.view_directory().display()
        ));
    }
    for (path, outcome) in outcomes {
        ui::label_value(&path.display().to_string(), outcome.describe());
    }
    for call in &plan.router_calls {
        ui::hint(&format!(
            "Serve its stylesheets and scripts: call `router.{call}` when you build the router."
        ));
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::{COMPONENTS, Request, source_spec};
    use crate::registry::library::{FileKind, parse_shipped_manifest};

    #[test]
    fn every_shipped_component_has_a_parseable_manifest_that_names_its_files() {
        assert!(!COMPONENTS.is_empty());
        for component in COMPONENTS {
            let manifest =
                parse_shipped_manifest(component.manifest.as_bytes(), component.directory)
                    .expect(component.directory);
            assert_eq!(manifest.name, format!("suprnova.{}", component.directory));
            for file in &manifest.files {
                assert!(
                    component.files.iter().any(|(name, _)| name == file),
                    "{} names {file} but does not ship it",
                    component.directory
                );
                assert_ne!(FileKind::of(file), Some(FileKind::Rust));
            }
        }
    }

    #[test]
    fn a_manifest_path_without_a_prefix_is_a_path_source() {
        let request = Request {
            manifest: Some("components/widget/manifest.json".into()),
            ..Request::default()
        };
        assert_eq!(
            source_spec(&request).expect("spec"),
            "./components/widget/manifest.json"
        );
        let both = Request {
            name: Some("field".to_owned()),
            manifest: Some("x".into()),
            ..Request::default()
        };
        assert!(source_spec(&both).is_err());
    }
}
