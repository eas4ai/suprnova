//! The vector operations an engine without a native recommend or discover runs: a fold
//! of the stored vectors into one query, and a ranking of the target's neighbourhood by
//! context pairs. Both read the generation's own vectors, never a copy (REC-006).
use super::{EngineResult, GenerationId, SearchEngine, Total, VectorDiscover, VectorRecommend};
use crate::{FrameworkError, search::registry::IndexDescriptor};
use std::collections::HashSet;

fn empty() -> EngineResult {
    EngineResult {
        candidates: Vec::new(),
        total: Total::Estimated(0),
    }
}

fn mean(index: &IndexDescriptor, vectors: &[&Vec<f32>]) -> Result<Vec<f64>, FrameworkError> {
    let dimensions = vectors.first().map_or(0, |vector| vector.len());
    let mut sum = vec![0.0; dimensions];
    for vector in vectors {
        if vector.len() != dimensions {
            return Err(FrameworkError::param(format!(
                "search index {}: stored vectors have different dimensions",
                index.name
            )));
        }
        for (slot, value) in sum.iter_mut().zip(vector.iter()) {
            *slot += f64::from(*value);
        }
    }
    let count = vectors.len().max(1) as f64;
    Ok(sum.into_iter().map(|value| value / count).collect())
}

fn cosine(a: &[f32], b: &[f32]) -> f64 {
    let (mut dot, mut left, mut right) = (0.0, 0.0, 0.0);
    for (x, y) in a.iter().zip(b) {
        let (x, y) = (f64::from(*x), f64::from(*y));
        dot += x * y;
        left += x * x;
        right += y * y;
    }
    if left == 0.0 || right == 0.0 {
        0.0
    } else {
        dot / (left.sqrt() * right.sqrt())
    }
}

/// Qdrant's average-vector strategy: the positives' mean, pushed away from the
/// negatives' mean by the same distance it lies from it.
pub(super) async fn recommend<E: SearchEngine + ?Sized>(
    engine: &E,
    index: &IndexDescriptor,
    generation: &GenerationId,
    request: &VectorRecommend,
) -> Result<EngineResult, FrameworkError> {
    if request.depth == 0 || request.positive.is_empty() {
        return Ok(empty());
    }
    let inputs: Vec<String> = request
        .positive
        .iter()
        .chain(&request.negative)
        .cloned()
        .collect();
    let vectors = engine.stored_vectors(index, generation, &inputs).await?;
    let positive: Vec<&Vec<f32>> = request
        .positive
        .iter()
        .filter_map(|key| vectors.get(key))
        .collect();
    if positive.is_empty() {
        return Ok(empty());
    }
    let negative: Vec<&Vec<f32>> = request
        .negative
        .iter()
        .filter_map(|key| vectors.get(key))
        .collect();
    let toward = mean(index, &positive)?;
    let query: Vec<f64> = if negative.is_empty() {
        toward
    } else {
        let away = mean(index, &negative)?;
        if away.len() != toward.len() {
            return Err(FrameworkError::param(format!(
                "search index {}: stored vectors have different dimensions",
                index.name
            )));
        }
        toward.iter().zip(&away).map(|(t, a)| t + t - a).collect()
    };
    if query.iter().all(|value| *value == 0.0) {
        return Ok(empty());
    }
    let query: Vec<f32> = query.into_iter().map(|value| value as f32).collect();
    let excluded: HashSet<&str> = inputs.iter().map(String::as_str).collect();
    let mut result = engine
        .semantic(
            index,
            generation,
            &query,
            request.filter.as_ref(),
            request.depth.saturating_add(excluded.len()),
        )
        .await?;
    result
        .candidates
        .retain(|candidate| !excluded.contains(candidate.document.key.as_str()));
    result.candidates.truncate(request.depth);
    result.total = Total::Estimated(result.candidates.len() as u64);
    Ok(result)
}

/// Rank the target's semantic neighbourhood: first by how many context pairs a document
/// lies nearer the positive than the negative side of, then by nearness to the target,
/// so a score is the rank plus a fraction below one.
pub(super) async fn discover<E: SearchEngine + ?Sized>(
    engine: &E,
    index: &IndexDescriptor,
    generation: &GenerationId,
    request: &VectorDiscover,
) -> Result<EngineResult, FrameworkError> {
    if request.depth == 0 {
        return Ok(empty());
    }
    let mut inputs = vec![request.target.clone()];
    for (positive, negative) in &request.context {
        inputs.push(positive.clone());
        inputs.push(negative.clone());
    }
    let vectors = engine.stored_vectors(index, generation, &inputs).await?;
    let Some(target) = vectors.get(&request.target).cloned() else {
        return Ok(empty());
    };
    let pairs: Vec<(&Vec<f32>, &Vec<f32>)> = request
        .context
        .iter()
        .filter_map(|(positive, negative)| Some((vectors.get(positive)?, vectors.get(negative)?)))
        .collect();
    let excluded: HashSet<&str> = inputs.iter().map(String::as_str).collect();
    // A neighbourhood four times the depth leaves the context room to reorder it.
    let pool = request
        .depth
        .saturating_mul(4)
        .saturating_add(excluded.len());
    let mut result = engine
        .semantic(index, generation, &target, request.filter.as_ref(), pool)
        .await?;
    result
        .candidates
        .retain(|candidate| !excluded.contains(candidate.document.key.as_str()));
    let keys: Vec<String> = result
        .candidates
        .iter()
        .map(|candidate| candidate.document.key.clone())
        .collect();
    let stored = engine.stored_vectors(index, generation, &keys).await?;
    for candidate in &mut result.candidates {
        let Some(vector) = stored.get(&candidate.document.key) else {
            candidate.score = f64::MIN;
            continue;
        };
        let rank = pairs
            .iter()
            .filter(|(positive, negative)| cosine(vector, positive) > cosine(vector, negative))
            .count() as f64;
        let nearness = (cosine(vector, &target) + 1.0) / 2.0 * 0.999;
        candidate.score = rank + nearness;
    }
    result
        .candidates
        .retain(|candidate| candidate.score > f64::MIN);
    result.candidates.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.document.key.cmp(&b.document.key))
    });
    result.candidates.truncate(request.depth);
    result.total = Total::Estimated(result.candidates.len() as u64);
    Ok(result)
}
