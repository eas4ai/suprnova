//! Errors raised while composing and dispatching plugins.

use core::fmt;

/// Result type used by the plugin SDK.
pub type PluginResult<T> = core::result::Result<T, PluginError>;

/// Plugin-specific failures with enough context for host diagnostics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PluginError {
    /// A plugin or route violates a composition invariant.
    InvalidComposition {
        /// Plugin associated with the defect, when known.
        plugin: String,
        /// Human-readable defect.
        message: String,
    },
    /// No registered route matched the request.
    RouteNotFound {
        /// Request path that was not found.
        path: String,
    },
    /// A plugin rejected or could not process the request.
    Request {
        /// Plugin that returned the error.
        plugin: String,
        /// Human-readable detail.
        message: String,
    },
    /// A lifecycle callback panicked after commit; delivery may be retried.
    LifecyclePanic {
        /// Plugin that panicked.
        plugin: String,
        /// Hook index in the plugin's hook list.
        hook: usize,
    },
    /// A lower-level Magnetar operation failed.
    Foundation(crate::Error),
}

impl fmt::Display for PluginError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidComposition { plugin, message } => {
                write!(
                    formatter,
                    "invalid plugin composition for {plugin}: {message}"
                )
            }
            Self::RouteNotFound { path } => write!(formatter, "plugin route not found: {path}"),
            Self::Request { plugin, message } => {
                write!(formatter, "plugin {plugin} request failed: {message}")
            }
            Self::LifecyclePanic { plugin, hook } => {
                write!(formatter, "plugin {plugin} lifecycle hook {hook} panicked")
            }
            Self::Foundation(error) => write!(formatter, "plugin foundation error: {error}"),
        }
    }
}

impl std::error::Error for PluginError {}

impl From<crate::Error> for PluginError {
    fn from(error: crate::Error) -> Self {
        Self::Foundation(error)
    }
}
