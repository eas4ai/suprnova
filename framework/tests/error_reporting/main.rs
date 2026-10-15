//! Error reporting contract, transport and runtime coverage.

mod drivers;
mod filtering;
mod hook;
mod paths;
mod event;
mod serialization;
mod delivery;
#[cfg(feature = "reporting-sentry")]
mod privacy;
