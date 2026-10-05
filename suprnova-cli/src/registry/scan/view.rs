//! The view scan (REG-031): Askama's parser for the template expressions,
//! a markup walk for the elements and attributes, and `cssparser` for the
//! stylesheets, `style` elements and `style` attributes.

use super::allowlist::Allowlist;
use super::{ComponentFiles, ScanReport};
use crate::registry::{RegistryError, Result};

/// Scans the component's views and stylesheets.
pub fn scan(component: &ComponentFiles<'_>, allowlist: &Allowlist) -> Result<ScanReport> {
    let _ = (component, allowlist);
    Err(RegistryError::NotBuilt("the view scan"))
}
