//! Integration tests for the `features` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

pub mod features;
pub mod migration;
#[cfg(feature = "testing")]
#[path = "../support/mysql_time_columns.rs"]
mod mysql_time_columns;
// Its tests move the framework clock with `TestClock`, which exists with
// the `testing` feature only.
#[cfg(feature = "testing")]
pub mod native_engines;
