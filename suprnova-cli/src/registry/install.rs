//! The write phase (REG-005, REG-013, REG-028, REG-029): under the project
//! lock, journal first, then every file, module declaration and
//! registration, then the records; any failure restores every prior byte.

use std::path::{Path, PathBuf};

use super::plan::{FileOutcome, Options, Plan};
use super::project::ProjectFile;
use super::{RegistryError, Result};

/// Applies a confirmed plan to the project. Returns each path with what
/// happened to it.
pub fn apply(
    plan: &Plan,
    project: &mut ProjectFile,
    options: &Options,
) -> Result<Vec<(PathBuf, FileOutcome)>> {
    let _ = (plan, project, options);
    Err(RegistryError::NotBuilt("installing"))
}

/// Adds `pub mod` declarations and `.register::<T>()` calls to the
/// application's `src/live/mod.rs` and namespace module, by parsing them
/// with `syn` (REG-005). Returns the new source of each file, or the lines
/// to add when the builder form is not the scaffold's.
pub fn registration_edits(
    project_root: &Path,
    namespace_module: &str,
    modules: &[String],
    register: &[String],
    unregister: &[String],
) -> Result<RegistrationEdits> {
    let _ = (
        project_root,
        namespace_module,
        modules,
        register,
        unregister,
    );
    Err(RegistryError::NotBuilt("registration edits"))
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
