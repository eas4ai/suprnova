//! The write phase (REG-005, REG-013, REG-028, REG-029): under the project
//! lock, journal first, then every file, module declaration and
//! registration, then the records; any failure restores every prior byte.
//!
//! The caller holds the [`ProjectLock`](super::project::ProjectLock) from
//! before it loaded the project's records until this returns. Every path
//! the install will touch is known before the first write and goes into
//! the journal with its prior bytes, so a failure at any later step, or a
//! process killed partway, is put back by the journal: this process
//! restores it on an error, and the next `live:add` or `serve` restores it
//! after a kill. Nothing here starts a process (REG-015).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use super::address::ComponentAddress;
use super::fetch::shipped_version;
use super::library::{FileKind, namespace_module};
use super::plan::{
    Decisions, FileOutcome, Options, Plan, component_record, decisions_from_flags, outcome_for,
};
use super::project::{
    INSTALL_RECORD, InstallRecord, Journal, PROJECT_FILE, ProjectFile, ProjectLock, ShippedRecord,
};
use super::statement::Digest;
use super::{RegistryError, Result};
use crate::secure_fs;

/// Applies a confirmed plan to the project. Returns each path with what
/// happened to it. A capability neither `--allow` nor an earlier install
/// approved is refused, since no approval can be recorded for it; a caller
/// that asks on a terminal uses [`apply_with`].
pub fn apply(
    plan: &Plan,
    project: &mut ProjectFile,
    lock: &ProjectLock,
    options: &Options,
) -> Result<Vec<(PathBuf, FileOutcome)>> {
    let decisions = decisions_from_flags(plan, options, project)?;
    apply_with(
        plan,
        project,
        lock,
        options,
        &decisions,
        &registration_edits,
    )
}

/// The registration writer's signature: [`registration_edits`] in an
/// install, a stand-in where a test needs one.
pub type Registrar<'a> =
    dyn Fn(&Path, &str, &[String], &[String], &[String]) -> Result<RegistrationEdits> + 'a;

/// One namespace's module declarations and registrations, over every
/// component of the plan in it.
#[derive(Default)]
struct NamespaceEdits {
    modules: Vec<String>,
    register: Vec<String>,
    unregister: Vec<String>,
}

/// Applies a confirmed plan with the developer's decisions and a
/// registration writer.
pub fn apply_with(
    plan: &Plan,
    project: &mut ProjectFile,
    lock: &ProjectLock,
    options: &Options,
    decisions: &Decisions,
    registrar: &Registrar<'_>,
) -> Result<Vec<(PathBuf, FileOutcome)>> {
    let root = project.root().to_path_buf();
    let mut writes: Vec<(PathBuf, &[u8])> = Vec::new();
    let mut outcomes = Vec::new();
    let mut install_records = Vec::with_capacity(plan.components.len());
    let mut kept_names: BTreeMap<ComponentAddress, Vec<String>> = BTreeMap::new();
    let mut removals: Vec<PathBuf> = Vec::new();
    let mut undeclare: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for component in &plan.components {
        let directory = component.view_directory();
        let mut record = InstallRecord::load(&root, &directory)?;
        let manifest_path = component.manifest_destination();
        let manifest_outcome = outcome_for(
            &root,
            &manifest_path,
            &component.manifest_bytes,
            &record,
            options.force,
        )?;
        let entries = std::iter::once((
            "manifest.json",
            &manifest_path,
            component.manifest_bytes.as_slice(),
            manifest_outcome,
        ))
        .chain(component.files.iter().map(|file| {
            (
                file.name.as_str(),
                &file.destination,
                file.bytes.as_slice(),
                file.outcome,
            )
        }));
        for (name, path, bytes, outcome) in entries {
            match outcome {
                FileOutcome::New | FileOutcome::Replaced => {
                    writes.push((path.clone(), bytes));
                    record.digests.insert(path.clone(), Digest::of(bytes));
                }
                FileOutcome::Unchanged => {
                    record.digests.insert(path.clone(), Digest::of(bytes));
                }
                // A kept file keeps its previous digest, so the next install
                // still sees the edit (REG-028).
                FileOutcome::Kept | FileOutcome::ChangedSinceRecord => {
                    kept_names
                        .entry(component.address.clone())
                        .or_default()
                        .push(name.to_owned());
                }
                // Only a file the library dropped is removed.
                FileOutcome::Removed => {}
            }
            outcomes.push((path.clone(), outcome));
        }
        // A file the library dropped is removed when the application never
        // edited it, its digest and module going with it; an edited one is
        // kept, recorded as kept, and keeps its digest (REG-028).
        for dropped in &component.dropped {
            match dropped.outcome {
                FileOutcome::Removed => {
                    removals.push(dropped.destination.clone());
                    record.digests.remove(&dropped.destination);
                    if dropped.kind == FileKind::Rust
                        && let Some(module) = dropped.name.strip_suffix(".rs")
                    {
                        undeclare
                            .entry(namespace_module(&component.library.namespace))
                            .or_default()
                            .push(module.to_owned());
                    }
                }
                _ => kept_names
                    .entry(component.address.clone())
                    .or_default()
                    .push(dropped.name.clone()),
            }
            outcomes.push((dropped.destination.clone(), dropped.outcome));
        }
        install_records.push((directory, record));
    }

    let mut namespaces: BTreeMap<String, NamespaceEdits> = BTreeMap::new();
    for component in &plan.components {
        if component.module_declarations.is_empty()
            && component.registrations.is_empty()
            && component.unregistrations.is_empty()
        {
            continue;
        }
        let edits = namespaces
            .entry(namespace_module(&component.library.namespace))
            .or_default();
        edits.modules.extend(component.manifest.rust_modules());
        edits
            .register
            .extend(component.registrations.iter().cloned());
        edits
            .unregister
            .extend(component.unregistrations.iter().cloned());
    }
    // Asked once before anything is written, so a builder in a form the
    // writer does not recognize writes nothing at all (REG-005).
    let mut registration_paths: BTreeSet<PathBuf> = BTreeSet::new();
    for (module, edits) in &namespaces {
        registration_paths.insert(PathBuf::from("src/live/mod.rs"));
        registration_paths.insert(PathBuf::from("src/live").join(module).join("mod.rs"));
        match registrar(
            &root,
            module,
            &edits.modules,
            &edits.register,
            &edits.unregister,
        )? {
            RegistrationEdits::Write(files) => {
                for (path, _) in files {
                    registration_paths.insert(relative_to(&root, &path));
                }
            }
            RegistrationEdits::Report(lines) => {
                return Err(RegistryError::Invalid(format!(
                    "src/live/mod.rs does not hold the registry builder in the form live:add edits, so nothing was written. Add these lines yourself, then run live:add again:\n{}",
                    lines.join("\n")
                )));
            }
        }
    }

    // A module whose file is removed loses its declaration; the module file
    // is read now so one this cannot edit refuses before anything is
    // written.
    for module in undeclare.keys() {
        let path = PathBuf::from("src/live").join(module).join("mod.rs");
        if let Some(source) = read_optional(&root, &path)?
            && syn::parse_file(&source).is_err()
        {
            return Err(RegistryError::Invalid(format!(
                "{} does not parse, so live:add cannot remove the declarations of the modules this update drops ({}); nothing was written",
                path.display(),
                undeclare[module].join(", ")
            )));
        }
        registration_paths.insert(path);
    }

    let mut touched: BTreeSet<PathBuf> = writes.iter().map(|(path, _)| path.clone()).collect();
    touched.extend(removals.iter().cloned());
    for (directory, _) in &install_records {
        touched.insert(directory.join(INSTALL_RECORD));
    }
    touched.insert(PathBuf::from(PROJECT_FILE));
    touched.extend(registration_paths.iter().cloned());
    let mut journal = journal_for(&root, &touched)?;
    lock.begin_journal(&mut journal)?;

    let prepared = Prepared {
        writes,
        removals,
        install_records,
        namespaces,
        undeclare,
        registration_paths,
        kept_names,
    };
    let result = write_all(plan, project, decisions, registrar, &root, &prepared);
    match result {
        Ok(()) => {
            Journal::remove(&root)?;
            Ok(outcomes)
        }
        // The journal is removed only once everything it names is back; a
        // rollback that could not put a path back keeps it, so the next
        // `live:add` or `serve` tries again, and says which paths.
        Err(error) => match journal.restore_into(&root) {
            Ok(()) => {
                Journal::remove(&root)?;
                Err(error)
            }
            Err(restore) => Err(RegistryError::Io(format!(
                "{error}; the rollback could not put everything back: {restore}. {} is kept and names every file to put back; the next live:add or suprnova serve restores from it",
                super::project::JOURNAL_FILE
            ))),
        },
    }
}

fn read_optional(root: &Path, path: &Path) -> Result<Option<String>> {
    secure_fs::ensure_contained(root, path).map_err(RegistryError::Io)?;
    match std::fs::read_to_string(root.join(path)) {
        Ok(source) => Ok(Some(source)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(RegistryError::Io(format!(
            "cannot read {}: {error}",
            path.display()
        ))),
    }
}

/// `source` with the bodiless `mod <name>;` declarations of `modules`
/// removed, each with the whole lines it spans.
fn without_modules(source: &str, modules: &[String]) -> Result<String> {
    use syn::spanned::Spanned as _;
    let file = syn::parse_file(source).map_err(|error| {
        RegistryError::Invalid(format!("a namespace module does not parse: {error}"))
    })?;
    let mut ranges: Vec<std::ops::Range<usize>> = file
        .items
        .iter()
        .filter(|item| {
            matches!(item, syn::Item::Mod(module)
                if module.content.is_none() && modules.iter().any(|name| module.ident == name))
        })
        .map(|item| {
            let range = item.span().byte_range();
            let start = source[..range.start].rfind('\n').map_or(0, |at| at + 1);
            let end = source[range.end..]
                .find('\n')
                .map_or(source.len(), |at| range.end + at + 1);
            start..end
        })
        .collect();
    ranges.sort_by_key(|range| std::cmp::Reverse(range.start));
    let mut edited = source.to_owned();
    for range in ranges {
        edited.replace_range(range, "");
    }
    Ok(edited)
}

fn relative_to(root: &Path, path: &Path) -> PathBuf {
    path.strip_prefix(root).unwrap_or(path).to_path_buf()
}

/// Every path the install touches, with the prior bytes of each that
/// exists, and every file and directory it creates, directories first.
fn journal_for(root: &Path, touched: &BTreeSet<PathBuf>) -> Result<Journal> {
    let mut journal = Journal::default();
    let mut created_directories: BTreeSet<PathBuf> = BTreeSet::new();
    for path in touched {
        secure_fs::ensure_contained(root, path).map_err(RegistryError::Io)?;
        let full = root.join(path);
        match std::fs::read(&full) {
            Ok(bytes) => {
                journal.changed.insert(path.clone(), bytes);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let mut missing = Vec::new();
                let mut parent = path.parent();
                while let Some(directory) = parent {
                    if directory.as_os_str().is_empty() || root.join(directory).exists() {
                        break;
                    }
                    missing.push(directory.to_path_buf());
                    parent = directory.parent();
                }
                for directory in missing.into_iter().rev() {
                    if created_directories.insert(directory.clone()) {
                        journal.created_directories.push(directory);
                    }
                }
                journal.created.push(path.clone());
            }
            Err(error) => {
                return Err(RegistryError::Io(format!(
                    "cannot read {}: {error}",
                    path.display()
                )));
            }
        }
    }
    Ok(journal)
}

/// Everything decided before the first write.
struct Prepared<'a> {
    writes: Vec<(PathBuf, &'a [u8])>,
    removals: Vec<PathBuf>,
    install_records: Vec<(PathBuf, InstallRecord)>,
    namespaces: BTreeMap<String, NamespaceEdits>,
    undeclare: BTreeMap<String, Vec<String>>,
    registration_paths: BTreeSet<PathBuf>,
    kept_names: BTreeMap<ComponentAddress, Vec<String>>,
}

fn write_all(
    plan: &Plan,
    project: &mut ProjectFile,
    decisions: &Decisions,
    registrar: &Registrar<'_>,
    root: &Path,
    prepared: &Prepared<'_>,
) -> Result<()> {
    let writes = &prepared.writes;
    for (path, _) in writes {
        if let Some(parent) = path.parent() {
            create_directories(root, parent)?;
        }
    }
    for (path, bytes) in writes {
        secure_fs::write_atomic_under(root, path, bytes).map_err(RegistryError::Io)?;
    }
    for path in &prepared.removals {
        secure_fs::ensure_contained(root, path).map_err(RegistryError::Io)?;
        match std::fs::remove_file(root.join(path)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(RegistryError::Io(format!(
                    "cannot remove {}: {error}",
                    path.display()
                )));
            }
        }
    }
    for (module, edits) in &prepared.namespaces {
        match registrar(
            root,
            module,
            &edits.modules,
            &edits.register,
            &edits.unregister,
        )? {
            RegistrationEdits::Write(files) => {
                for (path, source) in files {
                    let relative = relative_to(root, &path);
                    if !prepared.registration_paths.contains(&relative) {
                        return Err(RegistryError::Invalid(format!(
                            "the registration writer named {}, which the journal does not hold",
                            relative.display()
                        )));
                    }
                    if let Some(parent) = relative.parent() {
                        create_directories(root, parent)?;
                    }
                    secure_fs::write_atomic_under(root, &relative, source.as_bytes())
                        .map_err(RegistryError::Io)?;
                }
            }
            RegistrationEdits::Report(lines) => {
                return Err(RegistryError::Invalid(format!(
                    "src/live/mod.rs changed form during the install; add these lines yourself:\n{}",
                    lines.join("\n")
                )));
            }
        }
    }
    for (module, modules) in &prepared.undeclare {
        let path = PathBuf::from("src/live").join(module).join("mod.rs");
        if let Some(source) = read_optional(root, &path)? {
            let edited = without_modules(&source, modules)?;
            if edited != source {
                secure_fs::write_atomic_under(root, &path, edited.as_bytes())
                    .map_err(RegistryError::Io)?;
            }
        }
    }
    for (directory, record) in &prepared.install_records {
        create_directories(root, directory)?;
        record.save(root, directory)?;
    }
    for component in &plan.components {
        if component.is_shipped() {
            project.set_shipped(
                &component.address,
                &ShippedRecord {
                    version: shipped_version(),
                    manifest: Digest::of(&component.manifest_bytes),
                    files: component
                        .files
                        .iter()
                        .map(|file| (file.name.clone(), file.digest.clone()))
                        .collect(),
                },
            )?;
            continue;
        }
        let kept = prepared
            .kept_names
            .get(&component.address)
            .cloned()
            .unwrap_or_default();
        let record = component_record(plan, component, decisions, kept)?;
        project.set_component(&component.address, &record)?;
        let pin = plan
            .pins
            .iter()
            .find(|pin| pin.library == component.address.library)
            .map(|pin| &pin.key);
        project.set_library(
            &component.address.library,
            Some(&component.library.namespace),
            pin,
        )?;
    }
    project.save()
}

/// Creates a directory under the project and every missing parent, refusing
/// a path that leaves the project or passes through a link.
fn create_directories(root: &Path, directory: &Path) -> Result<()> {
    if directory.as_os_str().is_empty() {
        return Ok(());
    }
    secure_fs::ensure_contained(root, directory).map_err(RegistryError::Io)?;
    std::fs::create_dir_all(root.join(directory)).map_err(|error| {
        RegistryError::Io(format!("cannot create {}: {error}", directory.display()))
    })
}

/// Adds `pub mod` declarations and `.register::<T>()` calls to the
/// application's `src/live/mod.rs` and namespace module, by parsing them
/// with `syn` (REG-005). Returns the new source of each file, or the lines
/// to add when the builder form is not the scaffold's.
///
/// `modules` are the module names to declare in
/// `src/live/<namespace_module>/mod.rs`. Each entry of `register` and
/// `unregister` is a manifest entry, `<module>::<Type>`, or the full path
/// `crate::live::<namespace_module>::<module>::<Type>` it stands for; the
/// registration line always writes the full path. Every path in
/// [`RegistrationEdits::Write`] is relative to `project_root`, and only a
/// file whose source changes is listed, so a second install of the same
/// component writes nothing. A name that is not a plain identifier, or a
/// path outside the namespace module, is refused before anything is read.
pub fn registration_edits(
    project_root: &Path,
    namespace_module: &str,
    modules: &[String],
    register: &[String],
    unregister: &[String],
) -> Result<RegistrationEdits> {
    super::registration::edits(
        project_root,
        namespace_module,
        modules,
        register,
        unregister,
    )
}

/// The outcome of planning registrations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistrationEdits {
    /// Each file to rewrite with its new source.
    Write(Vec<(PathBuf, String)>),
    /// The builder is not in the scaffold's form: the lines the developer
    /// adds by hand; nothing is written for the whole install.
    Report(Vec<String>),
}
