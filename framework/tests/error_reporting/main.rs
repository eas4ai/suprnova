//! Error reporting contract, transport and runtime coverage.

mod drivers;

mod manual;
mod paths;

mod filtering;

mod event;
mod serialization;

mod delivery;
mod logs_traces;
#[cfg(feature = "reporting-sentry")]
mod rustrak;

mod hook;

#[cfg(feature = "reporting-sentry")]
mod privacy;
