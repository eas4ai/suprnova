//! Integration tests for the `features` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

pub mod features;
pub mod migration;
#[cfg(feature = "testing")]
pub mod shipped_table;
