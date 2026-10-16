//! Integration tests for the `Str` helpers and `Lang`'s percentage and
//! abbreviation (PAR-035 to PAR-037).

pub mod helpers;
/// Tests observe the agreed infrastructure gaps.
pub mod laravel_infra_gaps;
#[cfg(feature = "localization")]
pub mod numbers;
pub mod plural;
