#![deny(unsafe_code)]
#![deny(missing_docs)]

//! Standalone foundations for the Magnetar service.
//!
//! This crate intentionally exposes only the module boundaries needed by the
//! foundation plan. Domain behavior is added by later plan tasks.

/// Abuse-prevention and policy boundaries.
pub mod abuse;
/// Shared primary-authentication and factor-gate boundaries.
pub mod auth;
/// Cryptographic boundaries.
pub mod crypto;
/// Transactional migration bindings for the default application schema.
#[cfg(feature = "migration")]
pub mod default_migration;
/// Default application-owned SeaORM auth schema and SQL stores.
pub mod default_schema;
/// Errors returned by Magnetar operations.
pub mod error;
/// Atomic first-email-proof persistence boundary.
pub mod first_email_proof;

/// The third-party refresh lease broker and M2M token cache.
#[cfg(feature = "oauth")]
pub mod broker;
/// The typed outbound-mail catalog.
pub mod mail;
/// OAuth protocol engine: RFC wire types and declarative request shapes.
#[cfg(feature = "oauth")]
pub mod oauth;
/// WebAuthn passkey ceremonies and credential management.
#[cfg(feature = "passkey")]
pub mod passkey;
/// Password-domain services: hashing policy and lockout.
pub mod password;
/// TOTP two-factor authentication and the factor-gate wiring.
#[cfg(feature = "two-factor")]
pub mod two_factor;

/// Source-shape-aware migration planning and execution primitives.
#[cfg(feature = "migration")]
pub mod migration;

/// Shared schema boundaries.
pub mod schema;

/// Optional persistence and abuse-limiting drivers.
pub mod drivers;
/// Persistence boundaries.
pub mod storage;

/// Generic, framework-neutral plugin SDK.
pub mod plugin;
/// First-party feature-mirrored plugins.
pub mod plugins;
/// Session-management boundaries.
pub mod sessions;

/// The structured error type used by this crate.
pub use error::Error;

/// The result type used by this crate.
pub use error::Result;
