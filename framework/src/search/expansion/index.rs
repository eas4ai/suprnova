//! Snapshot-local deletion postings avoid a vocabulary scan for each typo.

use std::{
    collections::{BTreeMap, HashMap, HashSet, btree_map},
    fmt,
    ops::{Deref, DerefMut},
    sync::{Arc, OnceLock},
};

use serde::{Deserialize, Serialize};

use super::MAX_CORRECTION_SCALARS;

/// A frequency map whose lazy correction index is invalidated by map mutations.
/// Collect word-frequency pairs directly, or convert a `BTreeMap` with `into()`.
/// Clones share an initialized index until either map changes. Serde keeps the
/// same map representation and does not serialize the cache.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct VocabularyWords {
    words: BTreeMap<String, u64>,
    #[serde(skip)]
    index: OnceLock<Arc<DeletionIndex>>,
}

impl fmt::Debug for VocabularyWords {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.words.fmt(formatter)
    }
}

impl PartialEq for VocabularyWords {
    fn eq(&self, other: &Self) -> bool {
        self.words == other.words
    }
}
impl Eq for VocabularyWords {}

impl From<BTreeMap<String, u64>> for VocabularyWords {
    fn from(words: BTreeMap<String, u64>) -> Self {
        Self {
            words,
            index: OnceLock::new(),
        }
    }
}

impl FromIterator<(String, u64)> for VocabularyWords {
    fn from_iter<T: IntoIterator<Item = (String, u64)>>(iter: T) -> Self {
        BTreeMap::from_iter(iter).into()
    }
}

impl Deref for VocabularyWords {
    type Target = BTreeMap<String, u64>;
    fn deref(&self) -> &Self::Target {
        &self.words
    }
}

impl DerefMut for VocabularyWords {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.index.take();
        &mut self.words
    }
}

impl<'a> IntoIterator for &'a VocabularyWords {
    type Item = (&'a String, &'a u64);
    type IntoIter = btree_map::Iter<'a, String, u64>;
    fn into_iter(self) -> Self::IntoIter {
        self.words.iter()
    }
}

impl VocabularyWords {
    pub(super) fn candidates(&self, word: &str, budget: usize) -> Vec<&str> {
        let index = self
            .index
            .get_or_init(|| Arc::new(DeletionIndex::build(&self.words)));
        let length = word.chars().count();
        let mut ids = HashSet::new();
        for hash in deletion_hashes(word) {
            let mut posting = index.heads.get(&hash).copied();
            while let Some(at) = posting {
                let entry = &index.postings[at];
                ids.insert(entry.word);
                posting = entry.next;
            }
        }
        // The old scan accepts a 65/66-scalar candidate for a 64-scalar query.
        // Preserve that boundary without indexing oversized words' deletions.
        if length + budget > MAX_CORRECTION_SCALARS {
            ids.extend(&index.boundary);
        }
        ids.into_iter()
            .filter_map(|id| {
                let (candidate, candidate_length) = &index.words[id];
                (length.abs_diff(*candidate_length) <= budget).then_some(candidate.as_str())
            })
            .collect()
    }
}

struct Posting {
    word: usize,
    next: Option<usize>,
}

struct DeletionIndex {
    words: Vec<(String, usize)>,
    heads: HashMap<u64, usize>,
    postings: Vec<Posting>,
    boundary: Vec<usize>,
}

impl DeletionIndex {
    fn build(frequencies: &BTreeMap<String, u64>) -> Self {
        let words: Vec<_> = frequencies
            .iter()
            .filter(|(_, frequency)| **frequency > 0)
            .filter_map(|(word, _)| {
                let length = word.chars().take(MAX_CORRECTION_SCALARS + 3).count();
                (length <= MAX_CORRECTION_SCALARS + 2).then(|| (word.clone(), length))
            })
            .collect();
        let capacity = words
            .iter()
            .filter(|(_, n)| *n <= MAX_CORRECTION_SCALARS)
            .map(|(_, n)| 1 + n + n * n.saturating_sub(1) / 2)
            .sum();
        let mut index = Self {
            words,
            heads: HashMap::with_capacity(capacity),
            postings: Vec::with_capacity(capacity),
            boundary: Vec::new(),
        };
        for (id, (word, length)) in index.words.iter().enumerate() {
            if *length > MAX_CORRECTION_SCALARS {
                index.boundary.push(id);
                continue;
            }
            for hash in deletion_hashes(word) {
                let next = index.heads.insert(hash, index.postings.len());
                index.postings.push(Posting { word: id, next });
            }
        }
        index
    }
}

// A polynomial fingerprint lets us join scalar slices in constant time per
// deletion, without allocating every variant's string. Hash collisions only add
// candidates: the caller always confirms the full Damerau-Levenshtein distance.
fn deletion_hashes(word: &str) -> Vec<u64> {
    const BASE: u64 = 1_000_003;
    let mut prefix = [0u64; MAX_CORRECTION_SCALARS + 1];
    let mut powers = [1u64; MAX_CORRECTION_SCALARS + 1];
    let mut length = 0;
    for scalar in word.chars() {
        prefix[length + 1] = prefix[length]
            .wrapping_mul(BASE)
            .wrapping_add(u64::from(scalar) + 1);
        powers[length + 1] = powers[length].wrapping_mul(BASE);
        length += 1;
    }
    let slice = |start: usize, end: usize| {
        prefix[end].wrapping_sub(prefix[start].wrapping_mul(powers[end - start]))
    };
    let mut hashes = Vec::with_capacity(1 + length + length * length.saturating_sub(1) / 2);
    hashes.push(prefix[length]);
    for first in 0..length {
        hashes.push(
            prefix[first]
                .wrapping_mul(powers[length - first - 1])
                .wrapping_add(slice(first + 1, length)),
        );
        for second in first + 1..length {
            let left = prefix[first]
                .wrapping_mul(powers[second - first - 1])
                .wrapping_add(slice(first + 1, second));
            hashes.push(
                left.wrapping_mul(powers[length - second - 1])
                    .wrapping_add(slice(second + 1, length)),
            );
        }
    }
    hashes.sort_unstable();
    hashes.dedup();
    hashes
}
