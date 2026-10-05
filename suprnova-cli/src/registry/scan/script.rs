//! The script scan (REG-032): each `.js` file parsed as a JavaScript
//! module; every call must resolve to a function the script defines or a
//! standard browser API, and every URL must stay on the application's
//! origin.

use super::{ComponentFiles, ScanReport};
use crate::registry::{RegistryError, Result};

/// Scans the component's scripts.
pub fn scan(component: &ComponentFiles<'_>) -> Result<ScanReport> {
    let _ = component;
    Err(RegistryError::NotBuilt("the script scan"))
}
