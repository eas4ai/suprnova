//! BM25 supplies document term saturation; Qdrant supplies collection IDF at query time.
use super::{
    EmbedInput, Embedder, Embedding, InputKinds, ModelFiles, ModelProfile, Pooling, Precision,
    SparseEmbedding,
};
use crate::{FrameworkError, search::expansion::tokenize};
use async_trait::async_trait;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) struct Bm25Embedder {
    profile: ModelProfile,
    average_length: f64,
    stop_words: BTreeSet<String>,
}

impl Bm25Embedder {
    pub(crate) fn new(
        average_length: usize,
        stop_words: BTreeSet<String>,
    ) -> Result<Self, FrameworkError> {
        if average_length == 0 {
            return Err(FrameworkError::param("SEARCH_BM25_AVERAGE_LENGTH"));
        }
        Ok(Self {
            profile: ModelProfile {
                id: "bm25".into(),
                revision: None,
                dimensions: 1,
                max_tokens: usize::MAX,
                query_prefix: String::new(),
                document_template: "{text}".into(),
                pooling: Pooling::Mean,
                normalize: false,
                truncation_dims: vec![],
                inputs: InputKinds::default(),
                precision: Precision::Fp32,
                output_key: None,
                files: ModelFiles::default(),
            },
            average_length: average_length as f64,
            stop_words: stop_words.iter().flat_map(|word| tokenize(word)).collect(),
        })
    }

    fn encode(&self, text: &str, query: bool) -> SparseEmbedding {
        let mut counts = BTreeMap::<u32, usize>::new();
        let mut length = 0usize;
        for token in tokenize(text)
            .into_iter()
            .filter(|token| !self.stop_words.contains(token))
        {
            // Storage contract: 32-bit FNV-1a of normalized token UTF-8 bytes,
            // offset basis 2166136261, prime 16777619, wrapping after every byte.
            // Colliding tokens share a dimension and their frequencies merge.
            let hash = token.bytes().fold(2_166_136_261u32, |hash, byte| {
                (hash ^ u32::from(byte)).wrapping_mul(16_777_619)
            });
            *counts.entry(hash).or_default() += 1;
            length += 1;
        }
        let norm = 1.2 * (1.0 - 0.75 + 0.75 * length as f64 / self.average_length);
        let values = counts
            .values()
            .map(|tf| {
                if query {
                    1.0
                } else {
                    let tf = *tf as f64;
                    (tf * (1.2 + 1.0) / (tf + norm)) as f32
                }
            })
            .collect();
        SparseEmbedding {
            indices: counts.into_keys().collect(),
            values,
        }
    }
}

#[async_trait]
impl Embedder for Bm25Embedder {
    fn profile(&self) -> &ModelProfile {
        &self.profile
    }

    async fn embed_documents(
        &self,
        _inputs: &[EmbedInput],
    ) -> Result<Vec<Embedding>, FrameworkError> {
        Err(FrameworkError::param(
            "bm25 supports sparse embeddings only",
        ))
    }

    async fn embed_query(&self, _text: &str) -> Result<Embedding, FrameworkError> {
        Err(FrameworkError::param(
            "bm25 supports sparse embeddings only",
        ))
    }

    async fn embed_sparse(&self, texts: &[String]) -> Result<Vec<SparseEmbedding>, FrameworkError> {
        Ok(texts.iter().map(|text| self.encode(text, false)).collect())
    }

    async fn embed_sparse_query(&self, text: &str) -> Result<SparseEmbedding, FrameworkError> {
        Ok(self.encode(text, true))
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn encoder() -> Bm25Embedder {
        Bm25Embedder::new(256, BTreeSet::new()).unwrap()
    }

    #[test]
    fn tokenizer_and_stop_words_match_framework_tokens() {
        let provider =
            Bm25Embedder::new(256, BTreeSet::from(["the".into(), "and".into()])).unwrap();
        assert_eq!(
            provider.encode("THE café, and APPLE!", false),
            encoder().encode("cafe apple", false)
        );
        assert!(provider.encode("the AND", false).indices.is_empty());
    }

    #[test]
    fn fnv1a_token_hash_is_a_storage_contract() {
        assert_eq!(encoder().encode("apple", false).indices, [280_767_167]);
        // These distinct tokens collide under 32-bit FNV-1a and share one dimension.
        assert_eq!(encoder().encode("costarring liquid", true).values, [1.0]);
        assert_eq!(
            encoder().encode("costarring liquid", false),
            encoder().encode("liquid liquid", false)
        );
    }

    #[test]
    fn document_saturation_uses_fixed_average_length() {
        for (tf, expected) in [(1, 1.0), (2, 1.375), (5, 1.774_193_5)] {
            let text = std::iter::repeat_n("apple", tf)
                .chain(std::iter::repeat_n("filler", 256 - tf))
                .collect::<Vec<_>>()
                .join(" ");
            let vector = encoder().encode(&text, false);
            let position = vector
                .indices
                .iter()
                .position(|id| *id == 280_767_167)
                .unwrap();
            assert!((vector.values[position] - expected).abs() < 1e-6);
        }
        let short = encoder().encode("apple", false).values[0];
        let other_average = Bm25Embedder::new(1, BTreeSet::new())
            .unwrap()
            .encode("apple", false)
            .values[0];
        assert!(short > other_average);
        assert_eq!(other_average, 1.0);
        assert!(Bm25Embedder::new(0, BTreeSet::new()).is_err());
    }

    #[tokio::test]
    async fn queries_use_one_per_distinct_token_and_documents_are_deterministic() {
        let provider = encoder();
        let vector = provider
            .embed_sparse_query("apple pie apple")
            .await
            .unwrap();
        assert_eq!(vector.indices, [280_767_167, 1_782_644_405]);
        assert_eq!(vector.values, [1.0, 1.0]);
        let texts = ["fresh baked apple pie apple".into(), "".into()];
        let vectors = provider.embed_sparse(&texts).await.unwrap();
        assert_eq!(vectors, provider.embed_sparse(&texts).await.unwrap());
        assert_eq!(
            vectors[0].indices,
            [280_767_167, 1_782_644_405, 2_905_130_287, 3_076_581_868]
        );
        assert_eq!(
            vectors[0].values,
            [1.898_533_6, 1.669_730_2, 1.669_730_2, 1.669_730_2]
        );
        assert!(vectors[1].indices.is_empty());
        assert!(vectors[1].values.is_empty());
        for vector in vectors {
            vector.validate().unwrap();
        }
        assert!(
            provider
                .embed_sparse_query("")
                .await
                .unwrap()
                .indices
                .is_empty()
        );
        assert!(provider.embed_documents(&[]).await.is_err());
        assert!(provider.embed_query("apple").await.is_err());
    }
}
