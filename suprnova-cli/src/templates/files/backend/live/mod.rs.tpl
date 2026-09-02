//! Live components registered with the application.
//!
//! `registry()` builds the immutable registry of every Live component. Bind it
//! during bootstrap so the runtime, the routes, and the `suprnova live:*`
//! commands all see the same components:
//!
//! ```text
//! suprnova::App::singleton(crate::live::registry().expect("Live registry"));
//! ```

use suprnova::live::{LiveRegistry, RegistryError};

pub mod {snake};

/// Builds the registry of every Live component in this application.
pub fn registry() -> Result<LiveRegistry, RegistryError> {
    let registry = LiveRegistry::builder()
        .register::<{snake}::{pascal}>()?
        .build();
    Ok(registry)
}
