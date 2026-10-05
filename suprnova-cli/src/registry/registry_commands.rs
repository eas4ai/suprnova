//! Authoring a library: `live:registry new`, `check` and `sign` (REG-018,
//! REG-019, REG-020).

use std::path::Path;

use super::{RegistryError, Result};

/// The environment variable that names the author's private key file.
pub const KEY_ENV: &str = "SUPRNOVA_LIBRARY_KEY";

/// Scaffolds a library tree with one example component and a `preview/`
/// application, makes the key pair, and says where the private key is.
pub fn new(namespace: &str, directory: &Path) -> Result<()> {
    let _ = (namespace, directory);
    Err(RegistryError::NotBuilt("live:registry new"))
}

/// Checks every component as `live:add` would for reasons the library
/// alone decides, and lists each component's capabilities.
pub fn check(library_root: &Path) -> Result<()> {
    let _ = library_root;
    Err(RegistryError::NotBuilt("live:registry check"))
}

/// Runs every check but the signature check, then signs every component,
/// all or nothing.
pub fn sign(library_root: &Path) -> Result<()> {
    let _ = library_root;
    Err(RegistryError::NotBuilt("live:registry sign"))
}
