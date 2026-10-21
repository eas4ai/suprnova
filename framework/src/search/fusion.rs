//! Rank fusion compares engine positions instead of incomparable native scores.

use std::collections::{BTreeMap, BTreeSet};

/// RRF uses k = 60 to avoid letting one first-ranked candidate dominate agreement.
/// Repeated keys in a single leg contribute once; zero or invalid weights contribute nothing.
pub fn weighted_rrf(lists: &[(f64, Vec<String>)]) -> Vec<(String, f64)> {
    let mut scores = BTreeMap::<String, f64>::new();
    for (weight, keys) in lists {
        if !weight.is_finite() || *weight <= 0.0 {
            continue;
        }
        let mut seen = BTreeSet::new();
        for (position, key) in keys.iter().enumerate() {
            if seen.insert(key) {
                *scores.entry(key.clone()).or_default() += weight / (61.0 + position as f64);
            }
        }
    }
    let mut ranked: Vec<_> = scores.into_iter().collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    ranked
}

/// Relative scores stay within a result because native engine scales have no shared meaning.
pub fn normalize(scores: &[(String, f64)]) -> Vec<(String, f64)> {
    let maximum = scores
        .iter()
        .map(|(_, score)| *score)
        .filter(|score| score.is_finite())
        .fold(0.0_f64, f64::max);
    scores
        .iter()
        .map(|(key, score)| {
            (
                key.clone(),
                if maximum > 0.0 && score.is_finite() {
                    (score / maximum).clamp(0.0, 1.0)
                } else {
                    0.0
                },
            )
        })
        .collect()
}

/// An index-qualified key keeps identical primary keys from different indexes distinct.
#[derive(Clone, Debug, PartialEq)]
pub struct FederatedRank {
    /// Index identity is retained for hydration and attribution.
    pub index: String,
    /// Primary keys are meaningful only within their index.
    pub key: String,
    /// This RRF score can be normalized within the merged response.
    pub score: f64,
}

/// Each index contributes its own RRF rank; equal scores honor requested index order.
pub fn federated_rrf(indexes: &[(String, Vec<String>)]) -> Vec<FederatedRank> {
    let mut ranked = Vec::new();
    for (index_order, (index, keys)) in indexes.iter().enumerate() {
        let mut seen = BTreeSet::new();
        for (position, key) in keys.iter().enumerate() {
            if seen.insert(key) {
                ranked.push((
                    index_order,
                    FederatedRank {
                        index: index.clone(),
                        key: key.clone(),
                        score: 1.0 / (61.0 + position as f64),
                    },
                ));
            }
        }
    }
    ranked.sort_by(|a, b| {
        b.1.score
            .total_cmp(&a.1.score)
            .then_with(|| a.0.cmp(&b.0))
            .then_with(|| a.1.key.cmp(&b.1.key))
    });
    ranked.into_iter().map(|(_, hit)| hit).collect()
}

/// Slicing saturates large offsets and limits while retaining the caller's ranked order.
pub fn paginate<T>(ranked: &[T], offset: usize, limit: usize) -> &[T] {
    let start = offset.min(ranked.len());
    &ranked[start..start.saturating_add(limit).min(ranked.len())]
}
