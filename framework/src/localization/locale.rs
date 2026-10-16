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

    /// The hard-coded `en` locale - the last-resort default `Lang` falls
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
/// The requested languages are ranked by their q-values, highest first,
/// with equal weights keeping the header's order; a language with `q=0` is
/// refused and never chosen. The ranked list is matched against `available`
/// with fluent-langneg filtering: the first requested language with a match
/// wins, and `None` means nothing matched. A segment that is not a language
/// tag is skipped, so a malformed header can only fail to match.
pub fn negotiate(accept_language: &str, available: &[Locale]) -> Option<Locale> {
    let requested = ranked_languages(accept_language);

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

/// The language tags of an `Accept-Language` header, most preferred first.
///
/// fluent-langneg's own parser drops everything after `;`, so it keeps the
/// header's order and ignores the weights the client actually sent. Weights
/// are compared in thousandths, the precision RFC 9110 allows, so `0.001`
/// still outranks `0`. A weight that does not parse counts as `0`: the
/// client did not say it accepts the language.
fn ranked_languages(header: &str) -> Vec<fluent_langneg::LanguageIdentifier> {
    let mut weighted: Vec<(u16, fluent_langneg::LanguageIdentifier)> = header
        .split(',')
        .filter_map(|segment| {
            let mut parts = segment.split(';');
            let tag = parts.next()?.trim();
            let weight = parts
                .filter_map(|parameter| {
                    let (name, value) = parameter.split_once('=')?;
                    name.trim()
                        .eq_ignore_ascii_case("q")
                        .then(|| thousandths(value.trim()))
                })
                .next()
                .unwrap_or(1000);
            if weight == 0 {
                return None;
            }
            tag.parse().ok().map(|language| (weight, language))
        })
        .collect();
    // A stable sort keeps the header's order among equal weights.
    weighted.sort_by_key(|(weight, _)| std::cmp::Reverse(*weight));
    weighted.into_iter().map(|(_, language)| language).collect()
}

/// An RFC 9110 qvalue (`0` to `1`, at most three decimals) in thousandths,
/// or `0` when it is not one.
fn thousandths(raw: &str) -> u16 {
    let (whole, fraction) = raw.split_once('.').unwrap_or((raw, ""));
    if fraction.len() > 3 || !fraction.bytes().all(|b| b.is_ascii_digit()) {
        return 0;
    }
    let digits = format!("{fraction:0<3}");
    match (whole, digits.parse::<u16>()) {
        ("0", Ok(fraction)) => fraction,
        ("1", Ok(0)) => 1000,
        _ => 0,
    }
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
    fn qvalues_parse_to_thousandths() {
        assert_eq!(thousandths("1"), 1000);
        assert_eq!(thousandths("1.000"), 1000);
        assert_eq!(thousandths("0.5"), 500);
        assert_eq!(thousandths("0.001"), 1);
        assert_eq!(thousandths("0"), 0);
        assert_eq!(thousandths("1.5"), 0);
        assert_eq!(thousandths("0.0001"), 0);
        assert_eq!(thousandths("abc"), 0);
        assert_eq!(thousandths(""), 0);
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
