//! Locale identity and negotiation.

use crate::error::FrameworkError;
use fluent_langneg::{NegotiationStrategy, negotiate_languages};
use std::fmt;
use std::str::FromStr;
use unic_langid::LanguageIdentifier;

/// A BCP-47 language identifier (`en`, `en-US`, `pt-BR`).
///
/// Newtype over `unic_langid::LanguageIdentifier` so the dependency
/// never leaks into public signatures.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Locale(LanguageIdentifier);

impl Locale {
    /// Parse a locale, failing loudly on malformed input.
    pub fn parse(s: &str) -> Result<Self, FrameworkError> {
        s.parse::<LanguageIdentifier>().map(Self).map_err(|e| {
            FrameworkError::param(format!(
                "locale `{s}` is not a valid BCP-47 language identifier: {e}"
            ))
        })
    }

    /// The full identifier as text (`pt-BR`).
    pub fn as_str(&self) -> String {
        self.0.to_string()
    }

    /// The primary language subtag only (`pt` for `pt-BR`).
    pub fn language(&self) -> String {
        self.0.language.to_string()
    }

    pub(crate) fn as_langid(&self) -> &LanguageIdentifier {
        &self.0
    }

    /// The hard-coded `en` locale — the last-resort default `Lang` falls
    /// back to when no bootstrap config, task-local, or global override
    /// applies (or when a malformed env value would otherwise make
    /// `LocalizationConfig::from_env` fail). Built via the `langid!`
    /// macro, which const-validates the identifier at compile time, so
    /// this constructor has no runtime failure path and never needs
    /// `.unwrap()`/`.expect()`.
    pub(crate) fn fallback_en() -> Self {
        Self(unic_langid::langid!("en"))
    }
}

impl FromStr for Locale {
    type Err = FrameworkError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl fmt::Display for Locale {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Negotiate the best available locale for an `Accept-Language` header.
///
/// Uses fluent-langneg filtering: requested locales in q-order against available,
/// first match wins, `None` when nothing matches. Handles q-values and malformed
/// segments transparently.
pub fn negotiate(accept_language: &str, available: &[Locale]) -> Option<Locale> {
    // fluent_langneg's Accept-Language parser handles q-values and malformed segments
    let requested = fluent_langneg::accepted_languages::parse(accept_language);

    // Convert available locales to fluent_langneg's LanguageIdentifier for matching
    let avail: Vec<fluent_langneg::LanguageIdentifier> = available
        .iter()
        .filter_map(|l| l.as_str().parse().ok())
        .collect();

    // Use fluent_langneg's Filtering strategy to negotiate
    let matched = negotiate_languages(&requested, &avail, None, NegotiationStrategy::Filtering);

    // Find the best matched locale from our available list
    let best = matched.first()?.to_string();
    available.iter().find(|l| l.as_str() == best).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_prints_bcp47() {
        let l = Locale::parse("pt-BR").unwrap();
        assert_eq!(l.as_str(), "pt-BR");
        assert_eq!(l.language(), "pt");
        assert!(Locale::parse("not a locale!").is_err());
    }

    #[test]
    fn negotiates_accept_language_with_q_values() {
        let available = vec![Locale::parse("en").unwrap(), Locale::parse("es").unwrap()];
        let got = negotiate("fr-CH, es;q=0.8, en;q=0.5", &available).unwrap();
        assert_eq!(got.as_str(), "es");
        assert!(negotiate("zh, ja;q=0.9", &available).is_none());
    }

    #[test]
    fn exact_match_beats_language_match_regardless_of_order() {
        let en_gb = Locale::parse("en-GB").unwrap();
        let en = Locale::parse("en").unwrap();

        // Request "en" should match exact "en", not "en-GB" even though it's first
        let available = vec![en_gb.clone(), en.clone()];
        let got = negotiate("en", &available).unwrap();
        assert_eq!(got.as_str(), "en");

        // Request "en" should also match exact "en" even when it's second in the list
        let available = vec![en.clone(), en_gb.clone()];
        let got = negotiate("en", &available).unwrap();
        assert_eq!(got.as_str(), "en");
    }
}
