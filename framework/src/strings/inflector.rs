//! Plural and singular forms by doctrine/inflector's rules, the engine
//! behind Laravel's `Pluralizer`.

use super::inflector_rules::{ENGLISH, FRENCH, NORWEGIAN_BOKMAL, PORTUGUESE, SPANISH, TURKISH};
use regex::Regex;
use std::collections::HashMap;
use std::sync::OnceLock;

/// One direction of a language's rules, as doctrine lists them.
pub(super) struct RuleList {
    /// A pattern, its replacement, and a suffix the word must not end
    /// with (empty for none); the first pattern that applies wins.
    pub(super) transformations: &'static [(&'static str, &'static str, &'static str)],
    /// Words the direction leaves alone.
    pub(super) uninflected: &'static [&'static str],
}

/// A language's rules.
pub(super) struct LanguageRules {
    pub(super) singular: RuleList,
    pub(super) plural: RuleList,
    /// Singular and plural pairs no pattern covers.
    pub(super) irregular: &'static [(&'static str, &'static str)],
}

/// One direction, compiled.
struct Compiled {
    uninflected: Option<Regex>,
    irregular: HashMap<String, &'static str>,
    transformations: Vec<(Regex, &'static str, String)>,
}

/// A language's two directions, compiled.
struct Language {
    singular: Compiled,
    plural: Compiled,
}

impl Compiled {
    fn new(list: &RuleList, irregular: impl Iterator<Item = (&'static str, &'static str)>) -> Self {
        // A rule that does not compile is left out; a unit test proves
        // every rule compiles, so none is.
        let uninflected = (!list.uninflected.is_empty())
            .then(|| Regex::new(&format!("(?i)^(?:{})$", list.uninflected.join("|"))).ok())
            .flatten();
        Self {
            uninflected,
            irregular: irregular
                .map(|(from, to)| (from.to_lowercase(), to))
                .collect(),
            transformations: list
                .transformations
                .iter()
                .filter_map(|(pattern, replacement, unless)| {
                    Regex::new(pattern)
                        .ok()
                        .map(|regex| (regex, *replacement, unless.to_lowercase()))
                })
                .collect(),
        }
    }

    /// doctrine's `RulesetInflector::inflect`: a word the rules leave
    /// alone, else an irregular form, else the first pattern that matches.
    fn inflect(&self, word: &str) -> String {
        if word.is_empty() {
            return String::new();
        }
        if self
            .uninflected
            .as_ref()
            .is_some_and(|regex| regex.is_match(word))
        {
            return word.to_owned();
        }
        let lower = word.to_lowercase();
        if let Some(to) = self.irregular.get(&lower) {
            // The irregular form keeps a capital first letter.
            return if lower.chars().next() != word.chars().next() {
                upper_first(to)
            } else {
                (*to).to_owned()
            };
        }
        for (regex, replacement, unless) in &self.transformations {
            if regex.is_match(word) && (unless.is_empty() || !lower.ends_with(unless.as_str())) {
                return regex.replace_all(word, *replacement).into_owned();
            }
        }
        word.to_owned()
    }
}

impl Language {
    fn new(rules: &LanguageRules) -> Self {
        Self {
            singular: Compiled::new(
                &rules.singular,
                rules
                    .irregular
                    .iter()
                    .map(|(singular, plural)| (*plural, *singular)),
            ),
            plural: Compiled::new(&rules.plural, rules.irregular.iter().copied()),
        }
    }
}

/// The languages with rules of their own; any other uses English.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Tongue {
    English,
    French,
    NorwegianBokmal,
    Portuguese,
    Spanish,
    Turkish,
}

impl Tongue {
    /// The rules for a language subtag such as `fr` or `pt`.
    pub(super) fn for_language(language: &str) -> Self {
        match language.to_ascii_lowercase().as_str() {
            "fr" => Tongue::French,
            "nb" | "no" => Tongue::NorwegianBokmal,
            "pt" => Tongue::Portuguese,
            "es" => Tongue::Spanish,
            "tr" => Tongue::Turkish,
            _ => Tongue::English,
        }
    }

    fn compiled(self) -> &'static Language {
        static LANGUAGES: [OnceLock<Language>; 6] = [const { OnceLock::new() }; 6];
        let (slot, rules) = match self {
            Tongue::English => (0, &ENGLISH),
            Tongue::French => (1, &FRENCH),
            Tongue::NorwegianBokmal => (2, &NORWEGIAN_BOKMAL),
            Tongue::Portuguese => (3, &PORTUGUESE),
            Tongue::Spanish => (4, &SPANISH),
            Tongue::Turkish => (5, &TURKISH),
        };
        LANGUAGES[slot].get_or_init(|| Language::new(rules))
    }

    pub(super) fn pluralize(self, word: &str) -> String {
        self.compiled().plural.inflect(word)
    }

    pub(super) fn singularize(self, word: &str) -> String {
        self.compiled().singular.inflect(word)
    }
}

/// `word` with its first character in upper case.
pub(super) fn upper_first(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_regatta_exception_holds() {
        assert_eq!(Tongue::English.singularize("bacteria"), "bacterium");
        assert_eq!(Tongue::English.singularize("regatta"), "regatta");
    }

    #[test]
    fn every_rule_compiles() {
        for rules in [
            &ENGLISH,
            &FRENCH,
            &NORWEGIAN_BOKMAL,
            &PORTUGUESE,
            &SPANISH,
            &TURKISH,
        ] {
            for list in [&rules.singular, &rules.plural] {
                for (pattern, _, _) in list.transformations {
                    assert!(Regex::new(pattern).is_ok(), "{pattern}");
                }
                if !list.uninflected.is_empty() {
                    let joined = format!("(?i)^(?:{})$", list.uninflected.join("|"));
                    assert!(Regex::new(&joined).is_ok(), "{joined}");
                }
            }
        }
    }
}
