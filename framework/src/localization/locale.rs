//! Locale identity and negotiation.

use crate::error::FrameworkError;
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
        s.parse::<LanguageIdentifier>()
            .map(Self)
            .map_err(|e| FrameworkError::internal(format!("invalid locale '{s}': {e}")))
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
/// Parses accept-language header and finds the first available locale that
/// matches a requested locale (either exact match or language-only match).
pub fn negotiate(accept_language: &str, available: &[Locale]) -> Option<Locale> {
    let requested: Vec<LanguageIdentifier> = accept_language
        .split(',')
        .filter_map(|part| {
            part.split(';').next()?.trim().parse::<LanguageIdentifier>().ok()
        })
        .collect();

    for req in &requested {
        for av in available {
            // Exact match first
            if av.0 == *req {
                return Some(av.clone());
            }
            // Language-only match (e.g., "en-US" request matches "en" available)
            if av.0.language == req.language {
                return Some(av.clone());
            }
        }
    }

    None
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
}
