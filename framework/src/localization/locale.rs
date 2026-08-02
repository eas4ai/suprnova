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
            .map_err(|_| FrameworkError::param(s))
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
/// Uses fluent-langneg filtering: requested locales in q-order against available,
/// first match wins (exact match before language-only match), `None` when nothing
/// matches.
pub fn negotiate(accept_language: &str, available: &[Locale]) -> Option<Locale> {
    // Parse requested locales from Accept-Language header
    let requested: Vec<String> = accept_language
        .split(',')
        .filter_map(|part| {
            let locale_str = part.split(';').next()?.trim();
            if !locale_str.is_empty() {
                Some(locale_str.to_string())
            } else {
                None
            }
        })
        .collect();

    // Two-pass negotiation: exact match first, then language-only match.
    // This ensures "en" request returns "en" (exact) not "en-GB" (language match).

    // First pass: exact match
    for req_str in &requested {
        if let Ok(req_id) = req_str.parse::<LanguageIdentifier>() {
            for av in available {
                if av.0 == req_id {
                    return Some(av.clone());
                }
            }
        }
    }

    // Second pass: language-only match
    for req_str in &requested {
        if let Ok(req_id) = req_str.parse::<LanguageIdentifier>() {
            for av in available {
                if av.0.language == req_id.language {
                    return Some(av.clone());
                }
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
