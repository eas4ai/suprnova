//! Bounded lexical expansion keeps every engine's relevance model intact.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;

use super::{query::QueryText, settings::QuerySettings};
use crate::FrameworkError;

/// Document frequencies select useful corrections without scanning application rows.
///
/// ```
/// use suprnova::search::expansion::Vocabulary;
/// let vocabulary = Vocabulary {
///     words: [("house".to_owned(), 3)].into_iter().collect(),
/// };
/// assert_eq!(vocabulary.within("hosue", 1), vec![("house".to_owned(), 3)]);
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Vocabulary {
    /// Only words with a positive document frequency belong to the live vocabulary.
    pub words: BTreeMap<String, u64>,
}

const MAX_CORRECTION_SCALARS: usize = 64;
const MAX_CORRECTION_TOKENS: usize = 32;
const MAX_QUERY_BYTES: usize = 65_536;
const MAX_QUERY_TOKENS: usize = 1_024;

impl Vocabulary {
    /// Damerau distance recognizes adjacent transpositions as one typing error.
    /// Correction is limited to 64 Unicode scalars per query token. Longer tokens
    /// retain exact matches only. Candidate lengths must fit the edit budget before
    /// distance is computed, bounding each expansion comparison to at most 64 by 66.
    pub fn within(&self, word: &str, budget: usize) -> Vec<(String, u64)> {
        let length = word.chars().take(MAX_CORRECTION_SCALARS + 1).count();
        let mut candidates: Vec<_> = self
            .words
            .iter()
            .filter(|(candidate, df)| {
                if **df == 0 {
                    return false;
                }
                if word == candidate.as_str() {
                    return true;
                }
                if length > MAX_CORRECTION_SCALARS || budget == 0 {
                    return false;
                }
                let candidate_length = candidate
                    .chars()
                    .take(length.saturating_add(budget).saturating_add(1))
                    .count();
                length.abs_diff(candidate_length) <= budget
                    && strsim::damerau_levenshtein(word, candidate) <= budget
            })
            .map(|(candidate, df)| (candidate.clone(), *df))
            .collect();
        candidates.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        candidates
    }
}

/// An engine renders alternatives as one OR group, retaining AND between groups.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpandedQuery {
    /// Each group represents one original word or a multi-word synonym source.
    pub terms: Vec<ExpandedTerm>,
    /// These token sequences require an ordered, adjacent phrase match.
    pub phrases: Vec<Vec<String>>,
    /// Exclusions are exact tokens and are never typo-expanded.
    pub negatives: Vec<String>,
    /// Engines omit correction alternatives on these attributes, retaining originals and synonyms.
    #[serde(default)]
    pub typo_disabled_attributes: BTreeSet<String>,
}

/// Keeping the original separately lets renderers apply prefix matching precisely.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpandedTerm {
    /// The user's normalized word, or normalized multi-word synonym source.
    pub original: String,
    /// Normalized exact alternatives; strings containing spaces are phrase alternatives.
    pub alternatives: Vec<String>,
    /// Prefix matching applies to the original, never to corrected alternatives.
    pub prefix: bool,
    /// Correction provenance lets engines honor attribute-specific typo exclusions.
    #[serde(default)]
    pub typo_alternatives: Vec<String>,
}

/// Shared tokenization prevents correction and highlighting from disagreeing with collection.
pub fn tokenize(text: &str) -> Vec<String> {
    text.unicode_words()
        .flat_map(|word| {
            normalize_word(word)
                .unicode_words()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect()
}

pub(crate) fn normalize_word(word: &str) -> String {
    deunicode::deunicode(&word.to_lowercase())
        .trim()
        .to_lowercase()
}

/// Expansion is capped including the original so a query cannot grow without bound.
/// Multi-word synonyms occupy one OR group: their tokens must match as a phrase.
/// Phrase alternatives rank by their least-frequent token because the word vocabulary
/// has no phrase frequencies; the original always retains one slot within the cap.
/// Typo thresholds count original Unicode scalars before transliteration expands them.
/// Only the first 32 query tokens receive correction, and tokens longer than 64
/// scalars skip correction. Queries exceeding 65536 bytes or 1024 source tokens
/// return a parameter error, including programmatically constructed query text.
pub fn expand(
    query: &QueryText,
    settings: &QuerySettings,
    vocabulary: &Vocabulary,
    cap: usize,
) -> Result<ExpandedQuery, FrameworkError> {
    if cap == 0 {
        return Err(FrameworkError::param(
            "SEARCH_EXPANSION_TERMS must be positive",
        ));
    }
    let tokens = query
        .words
        .iter()
        .chain(query.phrases.iter().flatten())
        .chain(&query.negatives);
    let (bytes, count) = tokens.fold((0usize, 0usize), |(bytes, count), token| {
        (bytes.saturating_add(token.len()), count.saturating_add(1))
    });
    if query.original.len() > MAX_QUERY_BYTES || bytes > MAX_QUERY_BYTES || count > MAX_QUERY_TOKENS
    {
        return Err(FrameworkError::param(
            "search query exceeds 65536 bytes or 1024 tokens",
        ));
    }
    let stop: BTreeSet<_> = settings
        .stop_words
        .iter()
        .map(|word| normalize_word(word))
        .collect();
    let (words, source_lengths): (Vec<_>, Vec<_>) = query
        .words
        .iter()
        .flat_map(|word| word.unicode_words())
        .flat_map(|source| {
            let length = source.chars().count();
            tokenize(source).into_iter().map(move |word| (word, length))
        })
        .filter(|(word, _)| !stop.contains(word))
        .unzip();
    let synonyms: Vec<_> = settings
        .synonyms
        .iter()
        .map(|synonym| {
            (
                synonym
                    .terms
                    .iter()
                    .map(|term| tokenize(term))
                    .collect::<Vec<_>>(),
                synonym.one_way,
            )
        })
        .collect();
    let mut terms = Vec::new();
    let mut at = 0;
    while at < words.len() {
        let width = synonym_width(&words[at..], &synonyms);
        let original = words[at..at + width].join(" ");
        let mut alternatives = synonym_targets(&original, &synonyms, vocabulary);
        let budget = correction_budget(&original, settings, width, source_lengths[at]);
        let corrections = if budget > 0
            && at < MAX_CORRECTION_TOKENS
            && source_lengths[at] <= MAX_CORRECTION_SCALARS
            && original.chars().take(MAX_CORRECTION_SCALARS + 1).count() <= MAX_CORRECTION_SCALARS
        {
            vocabulary.within(&original, budget)
        } else {
            Vec::new()
        };
        let correction_only: BTreeSet<_> = corrections
            .iter()
            .filter(|(word, _)| word != &original && !alternatives.contains_key(word))
            .map(|(word, _)| word.clone())
            .collect();
        alternatives.extend(corrections);
        let alternatives = rank_alternatives(&original, alternatives, cap);
        let typo_alternatives = alternatives
            .iter()
            .filter(|word| correction_only.contains(*word))
            .cloned()
            .collect();
        terms.push(ExpandedTerm {
            alternatives,
            typo_alternatives,
            original,
            prefix: width == 1 && at + width == words.len(),
        });
        at += width;
    }
    let phrases = expand_phrases(query, &stop, &synonyms, vocabulary, cap, &mut terms);
    Ok(ExpandedQuery {
        terms,
        phrases,
        negatives: query
            .negatives
            .iter()
            .flat_map(|word| tokenize(word))
            .collect(),
        typo_disabled_attributes: settings.typo_tolerance.disable_on_attributes.clone(),
    })
}

fn synonym_width(words: &[String], synonyms: &[(Vec<Vec<String>>, bool)]) -> usize {
    synonyms
        .iter()
        .flat_map(|(variants, one_way)| {
            variants
                .iter()
                .take(if *one_way { 1 } else { variants.len() })
        })
        .filter(|variant| !variant.is_empty() && words.starts_with(variant))
        .map(Vec::len)
        .max()
        .unwrap_or(1)
        .max(1)
}

fn synonym_targets(
    original: &str,
    synonyms: &[(Vec<Vec<String>>, bool)],
    vocabulary: &Vocabulary,
) -> BTreeMap<String, u64> {
    let mut targets = BTreeMap::new();
    for (variants, one_way) in synonyms {
        let sources = &variants[..if *one_way {
            variants.len().min(1)
        } else {
            variants.len()
        }];
        if sources.iter().any(|variant| variant.join(" ") == original) {
            for target in variants.iter().filter(|target| !target.is_empty()) {
                let frequency = target
                    .iter()
                    .filter_map(|word| vocabulary.words.get(word))
                    .copied()
                    .min()
                    .unwrap_or(0);
                targets.insert(target.join(" "), frequency);
            }
        }
    }
    targets
}

fn correction_budget(
    original: &str,
    settings: &QuerySettings,
    width: usize,
    source_length: usize,
) -> usize {
    let tolerance = &settings.typo_tolerance;
    let disabled = tolerance
        .disable_on_words
        .iter()
        .any(|word| normalize_word(word) == original);
    let numeric = original.parse::<f64>().is_ok() || original.chars().all(char::is_numeric);
    if !tolerance.enabled || width > 1 || disabled || numeric {
        return 0;
    }
    if source_length >= tolerance.two_edit_min_length {
        2
    } else if source_length >= tolerance.one_edit_min_length {
        1
    } else {
        0
    }
}

fn rank_alternatives(
    original: &str,
    mut alternatives: BTreeMap<String, u64>,
    cap: usize,
) -> Vec<String> {
    alternatives.remove(original);
    let mut ranked: Vec<_> = alternatives.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let mut alternatives = vec![original.to_owned()];
    alternatives.extend(ranked.into_iter().take(cap - 1).map(|(word, _)| word));
    alternatives
}

fn expand_phrases(
    query: &QueryText,
    stop: &BTreeSet<String>,
    synonyms: &[(Vec<Vec<String>>, bool)],
    vocabulary: &Vocabulary,
    cap: usize,
    terms: &mut Vec<ExpandedTerm>,
) -> Vec<Vec<String>> {
    let mut phrases = Vec::new();
    for source in &query.phrases {
        let phrase: Vec<_> = source
            .iter()
            .flat_map(|word| tokenize(word))
            .filter(|word| !stop.contains(word))
            .collect();
        if phrase.is_empty() {
            continue;
        }
        let original = phrase.join(" ");
        let alternatives = synonym_targets(&original, synonyms, vocabulary);
        if alternatives.keys().any(|term| term != &original) {
            terms.push(ExpandedTerm {
                alternatives: rank_alternatives(&original, alternatives, cap),
                original,
                prefix: false,
                typo_alternatives: Vec::new(),
            });
        } else {
            phrases.push(phrase);
        }
    }
    phrases
}
