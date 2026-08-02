//! First-class localization: Fluent message catalogs, per-request locale
//! detection, translated validation messages, and ICU-backed formatting.
//!
//! See `manual/localization.md`. Feature-gated (`localization`,
//! default-on); with the feature off, validation renders its embedded
//! English fallbacks and nothing here compiles.

mod config;
mod locale;

pub use config::{Detect, LocalizationConfig};
pub use locale::{Locale, negotiate};
