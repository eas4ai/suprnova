//! Server-driven interactive components for ordinary Suprnova applications.
//!
//! Live keeps routes and initial HTML server-rendered while allowing an island
//! to synchronize typed state and invoke registered Rust actions through the
//! shipped browser runtime.

mod config;
mod registry;

/// Focused assertion helpers for application component tests.
pub mod testing;

/// Hidden generated-code support. Application code must not use this module.
#[doc(hidden)]
pub mod __private;

pub use config::{LiveConfig, LiveConfigBuilder, LiveConfigError, LiveConfigErrorKind};
pub use registry::{
    ComponentContract, LiveRegistry, LiveRegistryBuilder, RegistryError, RegistryErrorKind,
};
pub use suprnova_macros::{LiveComponent, live};

/// Closed semantic results and metadata returned by registered Live actions.
pub mod action {
    pub use suprnova_live::action::{
        ActionOutcome, ActionResult, AuthorizedAction, FlashIntent, OutcomeError, OutcomeErrorKind,
        OutcomeMetadata, RouteIntent, UrlIntent,
    };
}

/// Versioned browser event and effect contracts declared by Live components.
pub mod metadata {
    pub use suprnova_live::metadata::{EffectPayloadMetadata, EventPayloadMetadata};
}

/// Stable redacted failure and browser-recovery contracts.
pub mod error {
    pub use suprnova_live::error::{
        ErrorCategory, LiveError, RecoveryInstruction, SafeDiagnosticCode,
    };
}

/// Bounded localizable validation contracts used by Live components.
pub mod validation {
    pub use suprnova_live::validation::{
        BagPolicy, ErrorBag, ValidationIssue, ValidationMessageId, ValidationStatus,
    };
}

pub use action::{ActionOutcome, ActionResult, AuthorizedAction};
pub use error::{ErrorCategory, LiveError, RecoveryInstruction, SafeDiagnosticCode};
pub use metadata::{EffectPayloadMetadata, EventPayloadMetadata};
pub use validation::{ErrorBag, ValidationIssue, ValidationMessageId, ValidationStatus};
