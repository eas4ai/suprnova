//! Profiles keep model preparation and vector validation at the embedding boundary.

use crate::FrameworkError;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub(crate) mod bm25;
/// A deterministic provider makes semantic pipeline tests independent of inference services.
pub mod fake;
/// ONNX inference and pinned artifacts remain optional for lexical and HTTP deployments.
#[cfg(feature = "search-fastembed")]
pub mod fastembed;
#[cfg(feature = "search-fastembed")]
mod fastembed_rerank;
/// Deployment-owned OpenAI-compatible inference keeps credentials off request-controlled URLs.
pub mod http;
/// Media tensors remain usable without the inference runtime.
#[cfg(feature = "search-media")]
pub mod media;
#[cfg(feature = "search-fastembed")]
mod model_cache;
/// Named profiles encode each model's preparation and inference conventions.
pub mod profiles;
#[cfg(feature = "search-fastembed")]
mod text_graph;

/// Pooling is explicit because token outputs alone do not identify a sentence embedding.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pooling {
    /// Average non-padding token states for models trained with mean pooling.
    Mean,
    /// Use the classification token for models trained with CLS pooling.
    Cls,
    /// Decoder embedding models may train on the last non-padding token.
    LastToken,
}

/// Graph precision belongs in the profile so cache identity cannot hide a quantization change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Precision {
    /// The shipped default retains full floating-point weights.
    Fp32,
    /// Eight-bit graphs reduce memory when a model supplies them.
    Int8,
    /// Four-bit graphs are distinct artifacts with their own hashes.
    Q4,
}

/// Capabilities reject media input before a provider silently discards it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputKinds {
    /// Text support is required for lexical-to-semantic search queries.
    pub text: bool,
    /// Image support permits image media parts.
    pub image: bool,
    /// Audio support permits audio media parts.
    pub audio: bool,
    /// Video support permits video media parts.
    pub video: bool,
}

impl Default for InputKinds {
    fn default() -> Self {
        Self {
            text: true,
            image: false,
            audio: false,
            video: false,
        }
    }
}

/// Artifact names and digests allow providers to pin user-defined ONNX models reproducibly.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelFiles {
    /// The graph filename is model-specific rather than assumed by the provider.
    pub graph: String,
    /// External tensor data is present only for graphs that need it.
    pub external_data: Option<String>,
    /// Tokenizer identity controls context truncation and token IDs.
    pub tokenizer: String,
    /// Architecture metadata is required by user-defined models.
    pub config: String,
    /// Tokenizer options must come from the same pinned model revision.
    pub tokenizer_config: String,
    /// Special token mappings prevent hidden input preparation differences.
    pub special_tokens_map: String,
    /// Providers populate sha256 hex digests after resolving a revision.
    pub sha256: BTreeMap<String, String>,
    /// The persisted manifest digest binds the artifact set to generation provenance.
    #[serde(default)]
    pub manifest_sha256: Option<String>,
}

impl Default for ModelFiles {
    fn default() -> Self {
        Self {
            graph: "onnx/model.onnx".into(),
            external_data: None,
            tokenizer: "tokenizer.json".into(),
            config: "config.json".into(),
            tokenizer_config: "tokenizer_config.json".into(),
            special_tokens_map: "special_tokens_map.json".into(),
            sha256: BTreeMap::new(),
            manifest_sha256: None,
        }
    }
}

/// Preparation is data so HTTP and local providers use identical model conventions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelProfile {
    /// A repository or named model identifies the intended embedding space.
    pub id: String,
    /// Providers pin this revision on first resolution and persist it with generations.
    pub revision: Option<String>,
    /// Dimensions identify stored vectors; providers retain a native profile before truncation.
    pub dimensions: usize,
    /// Providers truncate with their actual tokenizer, counting prefixes and special tokens.
    pub max_tokens: usize,
    /// Retrieval query instructions are owned by the framework, never application callers.
    pub query_prefix: String,
    /// The only placeholders are `{title}` and `{text}`; absent titles become `none`.
    pub document_template: String,
    /// Sentence pooling is part of model compatibility.
    pub pooling: Pooling,
    /// Cosine-ready models require L2 normalization at the boundary.
    pub normalize: bool,
    /// Only dimensions trained with Matryoshka may truncate the native vector.
    pub truncation_dims: Vec<usize>,
    /// Media capabilities are explicit for multimodal providers.
    pub inputs: InputKinds,
    /// Precision selects a concrete artifact variant.
    pub precision: Precision,
    /// Multi-output graphs may require a named tensor instead of the first output.
    pub output_key: Option<String>,
    /// A manifest retains all artifacts needed for reproducible loading.
    pub files: ModelFiles,
}

/// Generations can retain inference provenance without depending on a provider's cache layout.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelProvenance {
    /// The model identifies the intended embedding space.
    pub id: String,
    /// A resolved revision prevents a moving repository reference from changing an index silently.
    pub revision: Option<String>,
    /// A manifest digest identifies the exact cached artifact set.
    pub manifest_sha256: Option<String>,
    /// Individual hashes make artifact identity inspectable across providers.
    pub files: BTreeMap<String, String>,
}

impl ModelProfile {
    /// Return validated provenance for storage alongside a generation's settings.
    pub fn provenance(&self) -> Result<ModelProvenance, FrameworkError> {
        self.validate()?;
        Ok(ModelProvenance {
            id: self.id.clone(),
            revision: self.revision.clone(),
            manifest_sha256: self.files.manifest_sha256.clone(),
            files: self.files.sha256.clone(),
        })
    }

    /// Expose stored dimensions after a provider-wide Matryoshka choice so core validation agrees.
    pub fn with_dimensions(&self, dimensions: Option<usize>) -> Result<Self, FrameworkError> {
        self.validate()?;
        let target = dimensions.unwrap_or(self.dimensions);
        if target != self.dimensions && !self.truncation_dims.contains(&target) {
            return Err(FrameworkError::param(format!(
                "SEARCH_EMBED_DIMENSIONS unsupported {target} for {}",
                self.id
            )));
        }
        let mut profile = self.clone();
        profile.dimensions = target;
        profile.normalize |= target != self.dimensions;
        profile
            .truncation_dims
            .retain(|dimension| *dimension < target);
        Ok(profile)
    }

    /// Reject malformed profiles before inference or generation creation.
    pub fn validate(&self) -> Result<(), FrameworkError> {
        if self.id.trim().is_empty() || self.dimensions == 0 || self.max_tokens == 0 {
            return Err(FrameworkError::param(format!(
                "embedding model {} requires an id, dimensions and max_tokens",
                self.id
            )));
        }
        if !self.inputs.text {
            return Err(FrameworkError::param(format!(
                "embedding model {} must accept text",
                self.id
            )));
        }
        if self
            .truncation_dims
            .iter()
            .any(|dim| *dim == 0 || *dim >= self.dimensions)
        {
            return Err(FrameworkError::param(format!(
                "embedding model {} truncation dimensions",
                self.id
            )));
        }
        let template = self
            .document_template
            .replace("{title}", "")
            .replace("{text}", "");
        if !self.document_template.contains("{text}") || template.contains(['{', '}']) {
            return Err(FrameworkError::param(format!(
                "embedding model {} document_template",
                self.id
            )));
        }
        Ok(())
    }

    /// Prefix application is centralized so providers do not drift between indexing and queries.
    pub fn query_input(&self, text: &str) -> Result<String, FrameworkError> {
        self.validate()?;
        Ok(format!("{}{text}", self.query_prefix))
    }

    /// Substitute once so braces inside user text never become template instructions.
    pub fn document_input(&self, input: &EmbedInput) -> Result<String, FrameworkError> {
        self.validate()?;
        for media in &input.media {
            let kind = media
                .media_type
                .split_once('/')
                .filter(|(_, subtype)| !subtype.is_empty())
                .map(|(kind, _)| kind.to_ascii_lowercase())
                .unwrap_or_default();
            let accepted = match kind.as_str() {
                "image" => self.inputs.image,
                "audio" => self.inputs.audio,
                "video" => self.inputs.video,
                _ => false,
            };
            if !accepted {
                return Err(FrameworkError::param(format!(
                    "embedding model {} does not accept {}",
                    self.id, media.media_type
                )));
            }
        }
        let template = input
            .document_template
            .as_deref()
            .filter(|template| !template.is_empty())
            .unwrap_or(&self.document_template);
        let remaining_placeholders = template.replace("{title}", "").replace("{text}", "");
        if !template.contains("{text}") || remaining_placeholders.contains(['{', '}']) {
            return Err(FrameworkError::param(format!(
                "embedding model {} embed_template",
                self.id
            )));
        }
        let mut output = String::new();
        let mut remaining = template;
        while let Some(at) = remaining.find('{') {
            output.push_str(&remaining[..at]);
            remaining = &remaining[at..];
            if let Some(rest) = remaining.strip_prefix("{title}") {
                output.push_str(
                    input
                        .title
                        .as_deref()
                        .filter(|title| !title.is_empty())
                        .unwrap_or("none"),
                );
                remaining = rest;
            } else if let Some(rest) = remaining.strip_prefix("{text}") {
                output.push_str(&input.text);
                remaining = rest;
            } else {
                return Err(FrameworkError::param(format!(
                    "embedding model {} document_template",
                    self.id
                )));
            }
        }
        output.push_str(remaining);
        Ok(output)
    }
}

/// Media remains typed and separate from text so providers can preserve input ordering.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaPart {
    /// MIME types determine which profile input capability is required.
    pub media_type: String,
    /// Owned bytes avoid assuming a filesystem or network fetch inside inference.
    pub data: Vec<u8>,
}

/// A title is distinct from the body because retrieval models may train on title templates.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbedInput {
    /// The declared embed source is joined into this text before model preparation.
    pub text: String,
    /// Missing titles deliberately render as `none` in profiles that use titles.
    pub title: Option<String>,
    /// Media-capable providers receive the same typed input contract.
    pub media: Vec<MediaPart>,
    /// An index-time override is carried with its input so its digest matches prepared text.
    #[serde(default)]
    pub document_template: Option<String>,
}

/// Validated vectors are separate from provider response envelopes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Embedding {
    /// Dense values have the requested supported dimensions and finite components.
    pub dense: Vec<f32>,
}

/// Sparse token weights permit providers such as BGE-M3 to serve lexical-vector retrieval.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SparseEmbedding {
    /// Sorted unique indices keep sparse dot products deterministic.
    pub indices: Vec<u32>,
    /// Weights correspond one-to-one to indices and must be finite.
    pub values: Vec<f32>,
}

impl SparseEmbedding {
    /// Reject ambiguous sparse representations before an engine stores them.
    pub fn validate(&self) -> Result<(), FrameworkError> {
        if self.indices.len() != self.values.len()
            || self.values.iter().any(|value| !value.is_finite())
            || self.indices.windows(2).any(|pair| pair[0] >= pair[1])
        {
            return Err(FrameworkError::param("sparse embedding indices and values"));
        }
        Ok(())
    }
}

/// Validate native or already-truncated output, then normalize after supported truncation.
pub fn validate_embedding(
    mut vector: Vec<f32>,
    profile: &ModelProfile,
    dimensions: Option<usize>,
) -> Result<Embedding, FrameworkError> {
    profile.validate()?;
    let target = dimensions.unwrap_or(profile.dimensions);
    if target != profile.dimensions && !profile.truncation_dims.contains(&target) {
        return Err(FrameworkError::param(format!(
            "embedding model {} unsupported dimensions {target}",
            profile.id
        )));
    }
    if (vector.len() != profile.dimensions && vector.len() != target)
        || vector.iter().any(|value| !value.is_finite())
    {
        return Err(FrameworkError::param(format!(
            "embedding model {} expected {target} dimensions of finite values",
            profile.id
        )));
    }
    vector.truncate(target);
    if profile.normalize || target != profile.dimensions {
        let norm = vector
            .iter()
            .map(|value| f64::from(*value).powi(2))
            .sum::<f64>()
            .sqrt();
        if norm == 0.0 || !norm.is_finite() {
            return Err(FrameworkError::param(format!(
                "embedding model {} zero vector",
                profile.id
            )));
        }
        for value in &mut vector {
            *value = (f64::from(*value) / norm) as f32;
        }
    }
    Ok(Embedding { dense: vector })
}

/// Providers remain object-safe so an index can choose its own inference implementation.
#[async_trait]
pub trait Embedder: Send + Sync {
    /// The profile identifies the embedding space stored in an index generation.
    fn profile(&self) -> &ModelProfile;
    /// Providers apply the profile template and tokenize within the context limit.
    async fn embed_documents(
        &self,
        inputs: &[EmbedInput],
    ) -> Result<Vec<Embedding>, FrameworkError>;
    /// Providers apply the profile query prefix before tokenization and inference.
    async fn embed_query(&self, text: &str) -> Result<Embedding, FrameworkError>;
    /// Query-specific sparse weights default to the provider's existing sparse head.
    async fn embed_sparse_query(&self, text: &str) -> Result<SparseEmbedding, FrameworkError> {
        let mut vectors = self.embed_sparse(&[text.to_owned()]).await?;
        if vectors.len() != 1 {
            return Err(FrameworkError::param(format!(
                "sparse model {}: expected one query embedding",
                self.profile().id
            )));
        }
        let vector = vectors.remove(0);
        vector.validate()?;
        Ok(vector)
    }
    /// Optional sparse output fails explicitly when the provider has no sparse head.
    async fn embed_sparse(
        &self,
        _texts: &[String],
    ) -> Result<Vec<SparseEmbedding>, FrameworkError> {
        Err(FrameworkError::param(format!(
            "embedding model {} does not support sparse embeddings",
            self.profile().id
        )))
    }
}

/// Texts counted against `SEARCH_EMBED_BATCH` are this many tokens long.
const BATCH_TEXT_TOKENS: u128 = 512;

/// Local inference batches over texts of the given token lengths, as input indexes.
///
/// Attention keeps a score for every pair of tokens and pads a batch to its longest
/// text, so a batch costs its size times the square of that length: one long document
/// among short ones once pushed a single call past 24 GB. Texts are taken shortest
/// first, and a batch closes at `limit` texts or when the next text would cost more
/// than `limit` texts of `BATCH_TEXT_TOKENS` tokens. A text longer than that runs alone.
#[cfg_attr(not(feature = "search-fastembed"), allow(dead_code))]
pub(crate) fn token_batches(lengths: &[usize], limit: usize) -> Vec<Vec<usize>> {
    let limit = limit.max(1);
    let budget = limit as u128 * BATCH_TEXT_TOKENS * BATCH_TEXT_TOKENS;
    let mut order: Vec<usize> = (0..lengths.len()).collect();
    order.sort_by_key(|&index| (lengths[index], index));
    let mut batches = Vec::new();
    let mut current: Vec<usize> = Vec::new();
    for index in order {
        // Ascending order makes this text the batch's longest.
        let longest = lengths[index] as u128;
        let cost = (current.len() as u128 + 1) * longest * longest;
        if !current.is_empty() && (current.len() >= limit || cost > budget) {
            batches.push(std::mem::take(&mut current));
        }
        current.push(index);
    }
    if !current.is_empty() {
        batches.push(current);
    }
    batches
}

#[cfg(test)]
mod bm25_tests {
    use crate::search::SearchRegistry;

    #[tokio::test]
    async fn bm25_is_available_without_an_optional_feature() {
        let provider = SearchRegistry::embedder_named("test", "bm25").unwrap();
        let vectors = provider.embed_sparse(&["apple".into()]).await.unwrap();
        assert_eq!(vectors[0].indices, [280_767_167]);
    }
}

#[cfg(test)]
mod token_batch_tests {
    use super::token_batches;

    fn flat(batches: &[Vec<usize>]) -> Vec<usize> {
        let mut all: Vec<usize> = batches.iter().flatten().copied().collect();
        all.sort_unstable();
        all
    }

    #[test]
    fn a_long_text_runs_apart_from_short_ones() {
        let mut lengths = vec![40; 63];
        lengths.insert(10, 2_500);
        let batches = token_batches(&lengths, 64);
        assert_eq!(flat(&batches), (0..64).collect::<Vec<_>>());
        let long = batches.iter().find(|batch| batch.contains(&10)).unwrap();
        assert_eq!(long, &vec![10]);
        assert_eq!(batches.len(), 2);
    }

    #[test]
    fn every_batch_stays_within_the_count_and_the_attention_budget() {
        let lengths: Vec<usize> = (0..500).map(|i| 20 + (i * 37) % 1_500).collect();
        let batches = token_batches(&lengths, 16);
        assert_eq!(flat(&batches), (0..500).collect::<Vec<_>>());
        for batch in &batches {
            assert!(batch.len() <= 16);
            let longest = batch.iter().map(|&i| lengths[i]).max().unwrap() as u128;
            assert!(
                batch.len() == 1 || batch.len() as u128 * longest * longest <= 16 * 512 * 512,
                "batch of {} up to {longest} tokens",
                batch.len()
            );
        }
    }

    #[test]
    fn short_texts_fill_the_whole_count() {
        let batches = token_batches(&[30; 130], 64);
        assert_eq!(
            batches.iter().map(Vec::len).collect::<Vec<_>>(),
            [64, 64, 2]
        );
    }

    #[test]
    fn a_count_of_one_keeps_every_text_alone() {
        let batches = token_batches(&[5, 900, 5], 1);
        assert_eq!(batches.len(), 3);
        assert!(batches.iter().all(|batch| batch.len() == 1));
    }

    #[test]
    fn no_texts_make_no_batches() {
        assert!(token_batches(&[], 64).is_empty());
    }
}
