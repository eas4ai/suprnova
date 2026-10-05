//! The Rust scan (REG-030): `syn` parses each `.rs` file; every path is
//! resolved through its `use`, alias and rename, then classified against
//! the allowlist, the component's own items, its dependencies' modules and
//! the admitted part of `std`. Anything else, and every construct that
//! could run or read at build time, is refused.

use super::allowlist::Allowlist;
use super::{ComponentFiles, ScanReport};
use crate::registry::{RegistryError, Result};

/// Scans the component's Rust files.
pub fn scan(component: &ComponentFiles<'_>, allowlist: &Allowlist) -> Result<ScanReport> {
    let _ = (component, allowlist);
    Err(RegistryError::NotBuilt("the Rust scan"))
}
