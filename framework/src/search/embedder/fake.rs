//! Stable hashing exercises semantic plumbing without a model download or network service.

use super::{EmbedInput, Embedder, Embedding, ModelProfile, Pooling, validate_embedding};
use crate::FrameworkError;
use async_trait::async_trait;
use sha2::{Digest, Sha256};

/// Equal prepared texts produce equal normalized vectors in every process.
///
/// ```
/// use suprnova::search::embedder::{Embedder, fake::FakeEmbedder};
/// # async fn example() -> Result<(), suprnova::FrameworkError> {
/// let embedder = FakeEmbedder::new(16)?;
/// let vector = embedder.embed_query("rust search").await?;
/// assert_eq!(vector.dense.len(), 16);
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct FakeEmbedder {
    profile: ModelProfile,
}

impl FakeEmbedder {
    /// Dimensions are configurable so validation tests can use small, readable vectors.
    pub fn new(dimensions: usize) -> Result<Self, FrameworkError> {
        let mut profile =
            super::profiles::text_profile("fake-hash", dimensions, 8192, Pooling::Mean);
        profile.revision = Some("sha256-v1".into());
        profile.validate()?;
        Ok(Self { profile })
    }

    /// A custom profile exposes prefix and document preparation to deterministic tests.
    pub fn with_profile(profile: ModelProfile) -> Result<Self, FrameworkError> {
        profile.validate()?;
        Ok(Self { profile })
    }

    fn embed(&self, text: &str) -> Result<Embedding, FrameworkError> {
        // The fake tokenizer defines one Unicode scalar as one token.
        let text: String = text.chars().take(self.profile.max_tokens).collect();
        let mut dense = Vec::new();
        dense
            .try_reserve_exact(self.profile.dimensions)
            .map_err(|error| {
                FrameworkError::param(format!(
                    "fake embedding dimensions {}: {error}",
                    self.profile.dimensions
                ))
            })?;
        for block in 0..self.profile.dimensions.div_ceil(8) {
            let mut hash = Sha256::new();
            hash.update(text.as_bytes());
            hash.update((block as u64).to_le_bytes());
            let bytes = hash.finalize();
            for chunk in bytes.as_chunks::<4>().0 {
                let raw = u32::from_le_bytes(*chunk);
                if dense.len() < self.profile.dimensions {
                    dense.push((f64::from(raw) / f64::from(u32::MAX) * 2.0 - 1.0) as f32);
                }
            }
        }
        dense.truncate(self.profile.dimensions);
        validate_embedding(dense, &self.profile, None)
    }
}

#[async_trait]
impl Embedder for FakeEmbedder {
    fn profile(&self) -> &ModelProfile {
        &self.profile
    }
    async fn embed_documents(
        &self,
        inputs: &[EmbedInput],
    ) -> Result<Vec<Embedding>, FrameworkError> {
        inputs
            .iter()
            .map(|input| {
                self.profile
                    .document_input(input)
                    .and_then(|text| self.embed(&text))
            })
            .collect()
    }
    async fn embed_query(&self, text: &str) -> Result<Embedding, FrameworkError> {
        self.embed(&self.profile.query_input(text)?)
    }
}
