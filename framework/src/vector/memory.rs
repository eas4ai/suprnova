//! Phase 9A - in-process [`VectorDriver`] backed by a `HashMap`.
//!
//! Used by unit tests and the `dev` profile when an external
//! vector backend isn't worth spinning up. Cosine similarity is
//! the only ranking - drivers that natively offer dot-product /
//! Euclidean shapes (Qdrant / Pinecone) expose those in their
//! own configuration, not here.

use super::driver::{VectorDriver, VectorItem, VectorMatch};
use crate::FrameworkError;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::RwLock;

/// In-process vector driver. Cheap to construct, hermetic per
/// instance (no shared state between two `MemoryVectorDriver::new()`
/// calls).
#[derive(Default)]
pub struct MemoryVectorDriver {
    // Map: store name -> point id -> VectorItem.
    stores: RwLock<HashMap<String, HashMap<String, VectorItem>>>,
}

impl MemoryVectorDriver {
    /// Construct an empty in-memory vector store. Equivalent to
    /// [`MemoryVectorDriver::default`].
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl VectorDriver for MemoryVectorDriver {
    async fn upsert(&self, store: &str, items: Vec<VectorItem>) -> Result<(), FrameworkError> {
        let mut guard = self
            .stores
            .write()
            .map_err(|_| FrameworkError::internal("memory vector lock poisoned"))?;
        let entry = guard.entry(store.to_string()).or_default();
        for item in items {
            entry.insert(item.id.clone(), item);
        }
        Ok(())
    }

    async fn similar(
        &self,
        store: &str,
        query: Vec<f32>,
        k: usize,
    ) -> Result<Vec<VectorMatch>, FrameworkError> {
        let guard = self
            .stores
            .read()
            .map_err(|_| FrameworkError::internal("memory vector lock poisoned"))?;
        let bucket = match guard.get(store) {
            Some(b) => b,
            None => return Ok(Vec::new()),
        };
        let q_norm = norm(&query);
        if q_norm == 0.0 {
            return Err(FrameworkError::param(
                "vector::similar query is zero-vector",
            ));
        }
        // Rank borrowed items and copy out only the `k` that are returned:
        // cloning every item's id and metadata to keep three of them was
        // the whole store's size again on every search.
        let mut scored: Vec<(f32, &VectorItem)> = bucket
            .values()
            .filter(|item| item.embedding.len() == query.len())
            .map(|item| (cosine(&query, &item.embedding, q_norm), item))
            .collect();
        // A NaN score, from a NaN in a stored embedding, ranks below every
        // number. `partial_cmp` called it equal to everything, which is not
        // an order, and the standard sort may panic on a comparator that
        // is not one.
        let rank = |score: f32| {
            if score.is_nan() {
                f32::NEG_INFINITY
            } else {
                score
            }
        };
        let best_first =
            |a: &(f32, &VectorItem), b: &(f32, &VectorItem)| rank(b.0).total_cmp(&rank(a.0));
        if k == 0 {
            return Ok(Vec::new());
        }
        if k < scored.len() {
            scored.select_nth_unstable_by(k - 1, best_first);
            scored.truncate(k);
        }
        scored.sort_by(best_first);
        Ok(scored
            .into_iter()
            .map(|(score, item)| VectorMatch {
                id: item.id.clone(),
                score,
                metadata: item.metadata.clone(),
            })
            .collect())
    }

    async fn delete(&self, store: &str, ids: Vec<String>) -> Result<(), FrameworkError> {
        let mut guard = self
            .stores
            .write()
            .map_err(|_| FrameworkError::internal("memory vector lock poisoned"))?;
        if let Some(bucket) = guard.get_mut(store) {
            for id in ids {
                bucket.remove(&id);
            }
        }
        Ok(())
    }

    async fn count(&self, store: &str) -> Result<usize, FrameworkError> {
        let guard = self
            .stores
            .read()
            .map_err(|_| FrameworkError::internal("memory vector lock poisoned"))?;
        Ok(guard.get(store).map(|b| b.len()).unwrap_or(0))
    }
}

fn norm(v: &[f32]) -> f32 {
    v.iter().map(|x| x * x).sum::<f32>().sqrt()
}

fn cosine(query: &[f32], item: &[f32], q_norm: f32) -> f32 {
    let dot: f32 = query.iter().zip(item.iter()).map(|(a, b)| a * b).sum();
    let item_norm = norm(item);
    if item_norm == 0.0 {
        return 0.0;
    }
    dot / (q_norm * item_norm)
}

#[cfg(test)]
mod mem_audit {
    use super::*;
    use crate::vector::VectorItem;

    /// MEM-002: a top-3 search over 1,000 items holds three results' room.
    #[tokio::test]
    async fn mem_audit_a_search_holds_only_its_k_results() {
        let driver = MemoryVectorDriver::new();
        driver
            .upsert(
                "s",
                (0..1000)
                    .map(|i| {
                        VectorItem::new(
                            format!("id-{i}"),
                            vec![1.0, i as f32],
                            serde_json::json!({ "i": i }),
                        )
                    })
                    .collect(),
            )
            .await
            .unwrap();
        let hits = driver.similar("s", vec![1.0, 0.0], 3).await.unwrap();
        assert_eq!(hits.len(), 3);
        assert!(
            hits.capacity() <= 3,
            "the result kept room for {}",
            hits.capacity()
        );
        assert_eq!(hits[0].id, "id-0");
        assert!(hits.windows(2).all(|w| w[0].score >= w[1].score));
    }

    /// MEM-006: a NaN score does not panic the search.
    #[tokio::test]
    async fn mem_audit_a_nan_score_does_not_panic() {
        let driver = MemoryVectorDriver::new();
        let mut items: Vec<VectorItem> = (0..64)
            .map(|i| {
                VectorItem::new(
                    format!("id-{i}"),
                    vec![1.0, i as f32],
                    serde_json::json!({}),
                )
            })
            .collect();
        items.push(VectorItem::new(
            "nan",
            vec![f32::NAN, 1.0],
            serde_json::json!({}),
        ));
        driver.upsert("s", items).await.unwrap();
        let hits = driver.similar("s", vec![1.0, 0.0], 10).await.unwrap();
        assert_eq!(hits.len(), 10);
    }
}
