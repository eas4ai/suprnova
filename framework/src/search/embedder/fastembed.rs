//! Verified artifacts and blocking inference keep ONNX off Tokio's worker threads.

/// Cross-encoder registration uses the core registry and builder contract.
pub use super::fastembed_rerank::FastembedReranker;
/// Manifests are exposed so operators can inspect and verify provenance independently of inference.
pub use super::model_cache::{ManifestFile, ModelManifest};
use super::model_cache::{file_error, hex, safe_path};
use super::{
    EmbedInput, Embedder, Embedding, ModelProfile, Pooling, Precision, SparseEmbedding, profiles,
    validate_embedding,
};
use crate::{FrameworkError, config::Config};
use async_trait::async_trait;
use fastembed::{
    Bgem3Embedding, Bgem3Model, EmbeddingModel, InitOptionsUserDefined, OutputKey,
    QuantizationMode, SparseModel, SparseTextEmbedding, TextEmbedding, TokenizerFiles,
    UserDefinedEmbeddingModel, UserDefinedSparseModel,
};
use hf_hub::{Repo, RepoType, api::sync::ApiBuilder};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

/// Model identities distinguish hub revisions from local directories and fastembed's named exports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelSource {
    /// A named export carries fastembed's graph, pooling and quantization metadata.
    Named(EmbeddingModel),
    /// A moving hub reference is resolved to a commit SHA on its first successful load.
    HuggingFace {
        /// The repository supplies all artifacts from one revision.
        repository: String,
        /// An explicit revision permits reproducible selection before the first cache fill.
        revision: Option<String>,
    },
    /// Local artifacts are copied into a verified cache before the runtime consumes them.
    Path(PathBuf),
}
impl ModelSource {
    /// Invalid identities fail before downloading or opening a model graph.
    pub fn parse(id: &str) -> Result<Self, FrameworkError> {
        if let Some(id) = id.strip_prefix("hf:") {
            let (repository, revision) = id
                .rsplit_once('@')
                .map_or((id, None), |(repo, rev)| (repo, Some(rev.to_owned())));
            if repository.split('/').count() != 2
                || repository.split('/').any(|part| {
                    part.is_empty()
                        || part == "."
                        || part == ".."
                        || !part
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
                })
                || revision.as_ref().is_some_and(|rev| {
                    rev.is_empty()
                        || !rev.chars().all(|c| {
                            c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/')
                        })
                        || Path::new(rev)
                            .components()
                            .any(|part| !matches!(part, std::path::Component::Normal(_)))
                })
            {
                return Err(FrameworkError::param(
                    "SEARCH_EMBED_MODEL requires hf:owner/repository[@revision]",
                ));
            }
            return Ok(Self::HuggingFace {
                repository: repository.into(),
                revision,
            });
        }
        if let Some(path) = id.strip_prefix("path:") {
            if path.trim().is_empty() {
                return Err(FrameworkError::param("SEARCH_EMBED_MODEL path is empty"));
            }
            return Ok(Self::Path(path.into()));
        }
        let model = id
            .parse::<EmbeddingModel>()
            .or_else(|_| {
                TextEmbedding::list_supported_models()
                    .into_iter()
                    .find(|info| info.model_code.eq_ignore_ascii_case(id))
                    .map(|info| info.model)
                    .ok_or_else(|| format!("unknown named model {id}"))
            })
            .map_err(|e| FrameworkError::param(format!("SEARCH_EMBED_MODEL: {e}")))?;
        Ok(Self::Named(model))
    }
}
/// Precision selects concrete text graphs; fp16 exports are deliberately excluded.
pub fn precision_files(precision: Precision) -> (&'static str, Option<&'static str>) {
    match precision {
        Precision::Fp32 => ("onnx/model.onnx", Some("onnx/model.onnx_data")),
        Precision::Int8 => ("onnx/model_quantized.onnx", None),
        Precision::Q4 => ("onnx/model_q4.onnx", None),
    }
}
/// Local inference configuration is immutable once the shared model is installed.
#[derive(Clone, Debug)]
pub struct FastembedConfig {
    /// An unset model selects the shipped EmbeddingGemma 2 text export.
    pub model: String,
    /// Graph variants are distinct cache identities and generation provenance.
    pub precision: Precision,
    /// Each model owns a verified directory under this deployment-owned root.
    pub cache: PathBuf,
    /// Inference batches bound temporary tensor memory.
    pub batch: usize,
    /// A configured sparse head supplies lexical vectors independently of dense inference.
    pub sparse_model: Option<String>,
    /// An explicit profile lets user-defined exports declare their full inference contract.
    pub profile: Option<ModelProfile>,
    /// Supported Matryoshka dimensions may reduce storage after native inference.
    pub dimensions: Option<usize>,
}
impl Default for FastembedConfig {
    fn default() -> Self {
        Self {
            model: "hf:onnx-community/embeddinggemma-2-ONNX".into(),
            precision: Precision::Fp32,
            cache: "storage/search/models".into(),
            batch: 64,
            sparse_model: None,
            profile: None,
            dimensions: None,
        }
    }
}
impl FastembedConfig {
    /// Resolve environment or registered configuration once at bootstrap, never per request.
    pub fn from_env() -> Result<Self, FrameworkError> {
        if let Some(config) = Config::get::<Self>() {
            return Ok(config);
        }
        let variable = |name: &str| std::env::var(name).ok();
        let default = Self::default();
        let precision = match variable("SEARCH_EMBED_PRECISION")
            .as_deref()
            .unwrap_or("fp32")
        {
            "fp32" => Precision::Fp32,
            "int8" => Precision::Int8,
            "q4" => Precision::Q4,
            _ => {
                return Err(FrameworkError::param(
                    "SEARCH_EMBED_PRECISION must be fp32, int8 or q4",
                ));
            }
        };
        Ok(Self {
            model: variable("SEARCH_EMBED_MODEL").unwrap_or(default.model),
            cache: variable("SEARCH_MODEL_CACHE")
                .map(PathBuf::from)
                .unwrap_or(default.cache),
            precision,
            batch: super::http::parse(variable("SEARCH_EMBED_BATCH"), 64, "SEARCH_EMBED_BATCH")?,
            sparse_model: variable("SEARCH_EMBED_SPARSE_MODEL"),
            profile: None,
            dimensions: variable("SEARCH_EMBED_DIMENSIONS")
                .map(|v| super::http::parse(Some(v), 0, "SEARCH_EMBED_DIMENSIONS"))
                .transpose()?,
        })
    }
}
enum SparseRuntime {
    Text(SparseTextEmbedding),
    Bgem3(Bgem3Embedding),
}
impl SparseRuntime {
    fn embed(
        &mut self,
        texts: &[String],
        batch: usize,
    ) -> Result<Vec<fastembed::SparseEmbedding>, FrameworkError> {
        match self {
            Self::Text(model) => model
                .embed(texts, Some(batch))
                .map_err(|e| FrameworkError::internal(format!("SEARCH_EMBED_SPARSE_MODEL: {e}"))),
            // The joint BGE-M3 export owns its learned sparse head and external tensors.
            Self::Bgem3(model) => model
                .embed(texts, Some(1))
                .map(|output| output.sparse)
                .map_err(|e| {
                    FrameworkError::internal(format!("SEARCH_EMBED_SPARSE_MODEL bge-m3: {e}"))
                }),
        }
    }
}
struct LocalModels {
    dense: TextEmbedding,
    sparse: Option<SparseRuntime>,
}
/// Clones share one initialized runtime, serialized on blocking threads rather than async workers.
#[derive(Clone)]
pub struct FastembedEmbedder {
    models: Arc<Mutex<LocalModels>>,
    profile: ModelProfile,
    native_profile: ModelProfile,
    batch: usize,
    dimensions: Option<usize>,
    cached_bytes: u64,
}
impl FastembedEmbedder {
    /// Boot resolves model files and initializes ONNX Runtime on the blocking pool.
    pub async fn from_env() -> Result<Self, FrameworkError> {
        Self::new(FastembedConfig::from_env()?).await
    }
    /// Explicit configuration supports isolated deployments and hermetic configuration tests.
    pub async fn new(config: FastembedConfig) -> Result<Self, FrameworkError> {
        tokio::task::spawn_blocking(move || Self::load(config))
            .await
            .map_err(|e| {
                FrameworkError::internal(format!("SEARCH_EMBED_MODEL initialization task: {e}"))
            })?
    }
    fn load(config: FastembedConfig) -> Result<Self, FrameworkError> {
        if config.batch == 0 {
            return Err(FrameworkError::param("SEARCH_EMBED_BATCH must be positive"));
        }
        if config.cache.as_os_str().is_empty() {
            return Err(FrameworkError::param("SEARCH_MODEL_CACHE is empty"));
        }
        let mut source = ModelSource::parse(&config.model)?;
        if let ModelSource::Named(model) = &source
            && config.precision != Precision::Fp32
        {
            let suffix = match config.precision {
                Precision::Int8 => "Q",
                Precision::Q4 => "Q4",
                Precision::Fp32 => "",
            };
            if !format!("{model:?}").ends_with(suffix) {
                source = ModelSource::Named(
                    format!(
                        "{}{suffix}",
                        format!("{model:?}")
                            .trim_end_matches("Q4")
                            .trim_end_matches('Q')
                    )
                    .parse()
                    .map_err(|_| {
                        FrameworkError::param(format!(
                            "SEARCH_EMBED_PRECISION {suffix} is unavailable for {model:?}"
                        ))
                    })?,
                );
            }
        }
        let mut profile = match &config.profile {
            Some(profile) => profile.clone(),
            None => source_profile(&source, config.dimensions)?,
        };
        let mut extras = Vec::new();
        let quantization = if let ModelSource::Named(model) = &source {
            let info =
                TextEmbedding::get_model_info(model).map_err(|e| model_error(&config.model, e))?;
            profile.files.graph = info.model_file.clone();
            extras = info.additional_files.clone();
            profile.files.external_data = extras.first().cloned();
            if profile.output_key.is_none() {
                profile.output_key = info.output_key.as_ref().and_then(|key| match key {
                    OutputKey::ByName(name) => Some((*name).into()),
                    _ => None,
                });
            }
            let quantization = TextEmbedding::get_quantization_mode(model);
            profile.precision = if format!("{model:?}").ends_with("Q4") {
                Precision::Q4
            } else if quantization != QuantizationMode::None {
                Precision::Int8
            } else {
                Precision::Fp32
            };
            quantization
        } else {
            profile.precision = config.precision;
            let (graph, external) = precision_files(config.precision);
            // Explicit profile filenames permit exports with nonstandard graph layouts.
            if config.profile.is_none() || profile.files.graph == "onnx/model.onnx" {
                profile.files.graph = graph.into();
                profile.files.external_data = external.map(Into::into);
            }
            match config.precision {
                Precision::Int8 => QuantizationMode::Dynamic,
                Precision::Fp32 | Precision::Q4 => QuantizationMode::None,
            }
        };
        profile.validate()?;
        if profile.inputs.image || profile.inputs.audio || profile.inputs.video {
            return Err(FrameworkError::param(
                "SEARCH_EMBED_MODEL fastembed text profiles cannot enable image, audio or video input",
            ));
        }
        if let Some(dimensions) = config.dimensions
            && dimensions != profile.dimensions
            && !profile.truncation_dims.contains(&dimensions)
        {
            return Err(FrameworkError::param(format!(
                "SEARCH_EMBED_DIMENSIONS unsupported {dimensions} for {}",
                profile.id
            )));
        }
        let mut names = tokenizer_names(&profile);
        names.push(profile.files.graph.clone());
        names.extend(extras.clone());
        let optional = if extras.is_empty() {
            profile
                .files
                .external_data
                .clone()
                .or_else(|| Some(format!("{}_data", profile.files.graph)))
        } else {
            None
        };
        let resolved = resolve_model(
            &source,
            &config.cache,
            profile.precision,
            &names,
            optional.as_deref(),
        )?;
        profile.revision = Some(resolved.manifest.revision.clone());
        profile.files.manifest_sha256 = Some(resolved.manifest.verify(&resolved.root)?);
        for (name, file) in &resolved.manifest.files {
            if let Some(expected) = profile.files.sha256.get(name)
                && expected != &file.sha256
            {
                return Err(FrameworkError::param(format!(
                    "model file {name} differs from profile sha256"
                )));
            }
            profile
                .files
                .sha256
                .insert(name.clone(), file.sha256.clone());
        }
        if let Some(name) = optional.filter(|name| resolved.manifest.files.contains_key(name)) {
            profile.files.external_data = Some(name);
        }
        if profile
            .files
            .external_data
            .as_ref()
            .is_some_and(|name| !resolved.manifest.files.contains_key(name))
        {
            profile.files.external_data = None;
        }
        let text_graph = format!("{}.text-only.onnx", profile.files.graph);
        if resolved.manifest.generated_files.contains_key(&text_graph)
            && resolved.manifest.files.contains_key(&text_graph)
        {
            profile.files.graph = text_graph;
        }
        let pooling = runtime_pooling(profile.pooling);
        let mut model = UserDefinedEmbeddingModel::new(
            read_file(&resolved.root, &profile.files.graph)?,
            tokenizers(&resolved.root, &profile)?,
        )
        .with_quantization(quantization);
        if let Some(pooling) = pooling {
            model = model.with_pooling(pooling);
        }
        let external = extras
            .into_iter()
            .chain(profile.files.external_data.clone())
            .collect::<std::collections::BTreeSet<_>>();
        for name in external {
            let parent = Path::new(&profile.files.graph)
                .parent()
                .unwrap_or(Path::new(""));
            let relative = Path::new(&name)
                .strip_prefix(parent)
                .unwrap_or(Path::new(&name));
            model = model.with_external_initializer(
                relative.to_string_lossy().into_owned(),
                read_file(&resolved.root, &name)?,
            );
        }
        let dense = TextEmbedding::try_new_from_user_defined(
            model,
            InitOptionsUserDefined::new()
                .with_max_length(profile.max_tokens)
                .with_intra_threads(4),
        )
        .map_err(|e| model_error(&config.model, e))?;
        let mut cached_bytes = resolved.manifest.files.values().map(|file| file.size).sum();
        let sparse = if let Some(name) = config.sparse_model {
            let (model, sparse_cache) = load_sparse(&name, &config.cache)?;
            cached_bytes += sparse_cache
                .manifest
                .files
                .values()
                .map(|file| file.size)
                .sum::<u64>();
            profile.files.sha256.insert(
                format!("sparse/{name}/manifest.json"),
                sparse_cache.manifest.verify(&sparse_cache.root)?,
            );
            Some(model)
        } else {
            None
        };
        Ok(Self {
            models: Arc::new(Mutex::new(LocalModels { dense, sparse })),
            profile: profile.with_dimensions(config.dimensions)?,
            native_profile: profile,
            // Dynamic graphs choose their range per inference call. Singleton batches
            // keep document and query vectors in the same space regardless of input order.
            batch: if quantization == QuantizationMode::Dynamic {
                1
            } else {
                config.batch
            },
            dimensions: config.dimensions,
            cached_bytes,
        })
    }
    /// The manifest's byte totals let operators plan disk capacity without reading model weights.
    pub fn cached_bytes(&self) -> u64 {
        self.cached_bytes
    }
    /// Prepared text permits qualification against exact model instructions and custom pipelines.
    /// Ordinary callers should use Embedder so prefixes and templates are always applied.
    pub async fn embed_prepared(&self, texts: &[String]) -> Result<Vec<Embedding>, FrameworkError> {
        if texts.is_empty() {
            return Ok(vec![]);
        }
        let texts = texts.to_vec();
        let models = Arc::clone(&self.models);
        let profile = self.native_profile.clone();
        let batch_size = self.batch;
        let dimensions = self.dimensions;
        tokio::task::spawn_blocking(move || {
            let mut models = models.lock().map_err(|_| {
                FrameworkError::internal("SEARCH_EMBED_MODEL inference lock poisoned")
            })?;
            let mut result = Vec::with_capacity(texts.len());
            for batch in texts.chunks(batch_size) {
                let output = models
                    .dense
                    .transform(batch, Some(batch.len()))
                    .map_err(|e| model_error(&profile.id, e))?;
                let vectors = pool_output(output, &profile)?;
                if vectors.len() != batch.len() {
                    return Err(model_error(
                        &profile.id,
                        "inference returned the wrong batch length",
                    ));
                }
                for vector in vectors {
                    result.push(validate_embedding(vector, &profile, dimensions)?);
                }
            }
            Ok(result)
        })
        .await
        .map_err(|e| model_error(&self.profile.id, e))?
    }
}
#[async_trait]
impl Embedder for FastembedEmbedder {
    fn profile(&self) -> &ModelProfile {
        &self.profile
    }
    async fn embed_documents(
        &self,
        inputs: &[EmbedInput],
    ) -> Result<Vec<Embedding>, FrameworkError> {
        let texts = inputs
            .iter()
            .map(|input| self.profile.document_input(input))
            .collect::<Result<Vec<_>, _>>()?;
        self.embed_prepared(&texts).await
    }
    async fn embed_query(&self, text: &str) -> Result<Embedding, FrameworkError> {
        self.embed_prepared(&[self.profile.query_input(text)?])
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| model_error(&self.profile.id, "no query vector"))
    }
    async fn embed_sparse(&self, texts: &[String]) -> Result<Vec<SparseEmbedding>, FrameworkError> {
        let texts = texts.to_vec();
        let models = Arc::clone(&self.models);
        let batch_size = self.batch;
        tokio::task::spawn_blocking(move || {
            let mut models = models.lock().map_err(|_| {
                FrameworkError::internal("SEARCH_EMBED_SPARSE_MODEL inference lock poisoned")
            })?;
            let sparse = models.sparse.as_mut().ok_or_else(|| {
                FrameworkError::param("SEARCH_EMBED_SPARSE_MODEL is not configured")
            })?;
            let mut result = Vec::with_capacity(texts.len());
            for batch in texts.chunks(batch_size) {
                let vectors = sparse.embed(batch, batch.len()).map_err(|e| {
                    FrameworkError::internal(format!("SEARCH_EMBED_SPARSE_MODEL: {e}"))
                })?;
                if vectors.len() != batch.len() {
                    return Err(FrameworkError::internal(
                        "SEARCH_EMBED_SPARSE_MODEL returned invalid batch length",
                    ));
                }
                for vector in vectors {
                    if vector.indices.len() != vector.values.len() {
                        return Err(FrameworkError::param(
                            "SEARCH_EMBED_SPARSE_MODEL returned mismatched indices and weights",
                        ));
                    }
                    let mut pairs = vector
                        .indices
                        .into_iter()
                        .zip(vector.values)
                        .map(|(index, value)| {
                            u32::try_from(index)
                                .map(|index| (index, value))
                                .map_err(|_| {
                                    FrameworkError::param(
                                        "SEARCH_EMBED_SPARSE_MODEL index exceeds u32",
                                    )
                                })
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    pairs.sort_by_key(|pair| pair.0);
                    let (indices, values) = pairs.into_iter().unzip();
                    let vector = SparseEmbedding { indices, values };
                    vector.validate()?;
                    result.push(vector);
                }
            }
            Ok(result)
        })
        .await
        .map_err(|e| FrameworkError::internal(format!("SEARCH_EMBED_SPARSE_MODEL task: {e}")))?
    }
}
fn model_error(id: &str, error: impl std::fmt::Display) -> FrameworkError {
    FrameworkError::internal(format!("SEARCH_EMBED_MODEL {id}: {error}"))
}
fn runtime_pooling(pooling: Pooling) -> Option<fastembed::Pooling> {
    match pooling {
        Pooling::Mean => Some(fastembed::Pooling::Mean),
        Pooling::Cls => Some(fastembed::Pooling::Cls),
        Pooling::LastToken => None,
    }
}
pub(super) fn tokenizer_names(profile: &ModelProfile) -> Vec<String> {
    vec![
        profile.files.tokenizer.clone(),
        profile.files.config.clone(),
        profile.files.tokenizer_config.clone(),
        profile.files.special_tokens_map.clone(),
    ]
}
fn read_file(root: &Path, name: &str) -> Result<Vec<u8>, FrameworkError> {
    let path = safe_path(root, name)?;
    fs::read(&path).map_err(|e| file_error(&path, e))
}
pub(super) fn tokenizers(
    root: &Path,
    profile: &ModelProfile,
) -> Result<TokenizerFiles, FrameworkError> {
    Ok(TokenizerFiles {
        tokenizer_file: read_file(root, &profile.files.tokenizer)?,
        config_file: read_file(root, &profile.files.config)?,
        special_tokens_map_file: read_file(root, &profile.files.special_tokens_map)?,
        tokenizer_config_file: read_file(root, &profile.files.tokenizer_config)?,
    })
}
fn source_profile(
    source: &ModelSource,
    dimensions: Option<usize>,
) -> Result<ModelProfile, FrameworkError> {
    let name = match source {
        ModelSource::Named(model) => {
            let info = TextEmbedding::get_model_info(model)
                .map_err(|e| model_error(&model.to_string(), e))?;
            let short = match format!("{model:?}")
                .trim_end_matches("Q4")
                .trim_end_matches('Q')
            {
                "AllMiniLML6V2" => "all-MiniLM-L6-v2",
                "AllMiniLML12V2" => "all-MiniLM-L12-v2",
                "BGEBaseENV15" => "bge-base-en-v1.5",
                "BGELargeENV15" => "bge-large-en-v1.5",
                "BGESmallENV15" => "bge-small-en-v1.5",
                "NomicEmbedTextV15" => "nomic-embed-text-v1.5",
                "MxbaiEmbedLargeV1" => "mxbai-embed-large-v1",
                "GTEBaseENV15" => "gte-base-en-v1.5",
                "GTELargeENV15" => "gte-large-en-v1.5",
                "EmbeddingGemma300M" => "embeddinggemma-300m",
                _ => &info.model_code,
            };
            let mut profile = profiles::profile(short).unwrap_or_else(|_| {
                profiles::text_profile(
                    &info.model_code,
                    info.dim,
                    fastembed::TextInitOptions::new(model.clone()).max_length,
                    match TextEmbedding::get_default_pooling_method(model) {
                        Some(fastembed::Pooling::Mean) => Pooling::Mean,
                        _ => Pooling::Cls,
                    },
                )
            });
            profile.id = info.model_code.clone();
            return Ok(profile);
        }
        ModelSource::HuggingFace { repository, .. } => repository.clone(),
        ModelSource::Path(path) => format!("path:{}", path.display()),
    };
    profiles::profile(&name).or_else(|_| {
        let dimensions=dimensions.ok_or_else(||FrameworkError::param(format!("SEARCH_EMBED_MODEL {name} needs an explicit ModelProfile or SEARCH_EMBED_DIMENSIONS")))?;
        Ok(profiles::text_profile(&name,dimensions,8192,Pooling::Mean))
    })
}
fn pool_output(
    output: fastembed::EmbeddingOutput,
    profile: &ModelProfile,
) -> Result<Vec<Vec<f32>>, FrameworkError> {
    let mut result = vec![];
    for batch in output.into_raw() {
        let index = if let Some(key) = &profile.output_key {
            batch.outputs.iter().position(|(name, _)| name == key)
        } else if batch.outputs.len() == 1 {
            Some(0)
        } else {
            ["text_embeds", "last_hidden_state", "sentence_embedding"]
                .iter()
                .find_map(|key| batch.outputs.iter().position(|(name, _)| name == key))
        }
        .ok_or_else(|| {
            model_error(
                &profile.id,
                format!("output key {:?} is absent", profile.output_key),
            )
        })?;
        let key = OutputKey::ByOrder(index);
        if profile.pooling != Pooling::LastToken {
            let pooled = batch
                .select_and_pool_output(&&key, runtime_pooling(profile.pooling))
                .map_err(|e| model_error(&profile.id, e))?;
            result.extend(pooled.rows().into_iter().map(|row| row.to_vec()));
        } else {
            let tensor = batch
                .select_output(&&key)
                .map_err(|e| model_error(&profile.id, e))?;
            if tensor.ndim() == 2 {
                for row in 0..tensor.shape()[0] {
                    result.push(
                        (0..tensor.shape()[1])
                            .map(|dim| tensor[&[row, dim][..]])
                            .collect(),
                    );
                }
            } else if tensor.ndim() == 3
                && tensor.shape()[0] == batch.attention_mask_array.nrows()
                && tensor.shape()[1] == batch.attention_mask_array.ncols()
            {
                for (row, mask) in batch.attention_mask_array.rows().into_iter().enumerate() {
                    let last = mask.iter().rposition(|value| *value != 0).ok_or_else(|| {
                        model_error(
                            &profile.id,
                            "last-token pooling has an empty attention mask",
                        )
                    })?;
                    result.push(
                        (0..tensor.shape()[2])
                            .map(|dim| tensor[&[row, last, dim][..]])
                            .collect(),
                    );
                }
            } else {
                return Err(model_error(
                    &profile.id,
                    "last-token pooling requires a batch-token-dimension tensor",
                ));
            }
        }
    }
    Ok(result)
}

pub(super) struct ResolvedModel {
    pub(super) root: PathBuf,
    pub(super) manifest: ModelManifest,
}
pub(super) fn resolve_model(
    source: &ModelSource,
    cache: &Path,
    precision: Precision,
    names: &[String],
    optional: Option<&str>,
) -> Result<ResolvedModel, FrameworkError> {
    let (repository, revision, local) = match source {
        ModelSource::Named(model) => {
            let info = TextEmbedding::get_model_info(model)
                .map_err(|e| model_error(&model.to_string(), e))?;
            (info.model_code.clone(), None, None)
        }
        ModelSource::HuggingFace {
            repository,
            revision,
        } => (repository.clone(), revision.clone(), None),
        ModelSource::Path(path) => (format!("path:{}", path.display()), None, Some(path)),
    };
    let identity = format!(
        "{repository}@{}:{precision:?}:{}:{}",
        revision.as_deref().unwrap_or("main"),
        names.join(","),
        optional.unwrap_or("")
    );
    let sanitized = repository
        .chars()
        .take(100)
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
    let root = cache.join(format!(
        "{sanitized}-{}",
        &hex(&Sha256::digest(identity.as_bytes()))[..16]
    ));
    fs::create_dir_all(&root).map_err(|e| file_error(&root, e))?;
    let lock_path = root.join(".lock");
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .map_err(|e| file_error(&lock_path, e))?;
    lock.lock().map_err(|e| file_error(&lock_path, e))?;
    let manifest_path = root.join("manifest.json");
    let mut manifest = if manifest_path.exists() {
        let manifest = ModelManifest::read(&root)?;
        if manifest.repository != repository
            || names.iter().any(|name| !manifest.files.contains_key(name))
        {
            return Err(file_error(
                &manifest_path,
                "model identity or required artifact differs",
            ));
        }
        manifest.verify(&root)?;
        manifest
    } else {
        let mut files = names.to_vec();
        let mut generated_files = std::collections::BTreeMap::new();
        let resolved_revision;
        if let Some(local) = local {
            if let Some(name) = optional
                && safe_path(local, name)?.exists()
            {
                files.push(name.into());
            }
            for name in &files {
                copy_file(&safe_path(local, name)?, &safe_path(&root, name)?)?;
            }
            resolved_revision = "local".into();
        } else {
            let api = ApiBuilder::from_cache(hf_hub::Cache::new(root.join(".hub")))
                .with_token(None)
                .with_progress(false)
                .with_retries(3)
                .build()
                .map_err(|e| model_error(&repository, e))?;
            let requested = api.repo(Repo::with_revision(
                repository.clone(),
                RepoType::Model,
                revision.clone().unwrap_or_else(|| "main".into()),
            ));
            let info = requested.info().map_err(|e| model_error(&repository, e))?;
            resolved_revision = info.sha;
            if resolved_revision.len() != 40
                || !resolved_revision
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit())
            {
                return Err(model_error(&repository, "hub did not answer a commit SHA"));
            }
            if let Some(name) = optional
                && info.siblings.iter().any(|file| file.rfilename == name)
            {
                files.push(name.into());
            }
            let pinned = api.repo(Repo::with_revision(
                repository.clone(),
                RepoType::Model,
                resolved_revision.clone(),
            ));
            for name in &files {
                if name == "special_tokens_map.json"
                    && !info.siblings.iter().any(|file| file.rfilename == *name)
                {
                    derive_special_tokens(&root)?;
                    generated_files.insert(
                        name.clone(),
                        "tokenizer_config.json token metadata, token-map-v1".into(),
                    );
                    continue;
                }
                let downloaded = pinned.get(name).map_err(|e| {
                    FrameworkError::internal(format!("model file {repository}/{name}: {e}"))
                })?;
                copy_file(&downloaded, &safe_path(&root, name)?)?;
            }
        }
        files.sort();
        files.dedup();
        ModelManifest::record_with_generated(
            &root,
            &repository,
            &resolved_revision,
            &files,
            generated_files,
        )?
    };
    manifest.verify(&root)?;
    if repository.eq_ignore_ascii_case("onnx-community/embeddinggemma-2-ONNX") {
        let graph = names
            .iter()
            .find(|name| name.ends_with(".onnx"))
            .ok_or_else(|| model_error(&repository, "text graph file is missing"))?;
        let text_graph = format!("{graph}.text-only.onnx");
        if !manifest.generated_files.contains_key(&text_graph) {
            let bytes = super::text_graph::embeddinggemma_text_graph(&read_file(&root, graph)?)?;
            let output = safe_path(&root, &text_graph)?;
            fs::write(&output, bytes).map_err(|e| file_error(&output, e))?;
            let mut generated = manifest.generated_files.clone();
            generated.insert(
                text_graph.clone(),
                format!("{graph} with empty media features, text-graph-v1"),
            );
            let mut files = manifest.files.keys().cloned().collect::<Vec<_>>();
            files.push(text_graph);
            manifest = ModelManifest::record_with_generated(
                &root,
                &repository,
                &manifest.revision,
                &files,
                generated,
            )?;
        }
    }
    manifest.verify(&root)?;
    Ok(ResolvedModel { root, manifest })
}
fn derive_special_tokens(root: &Path) -> Result<(), FrameworkError> {
    let input = root.join("tokenizer_config.json");
    let value: serde_json::Value =
        serde_json::from_slice(&fs::read(&input).map_err(|e| file_error(&input, e))?)
            .map_err(|e| file_error(&input, e))?;
    let metadata = value
        .as_object()
        .ok_or_else(|| file_error(&input, "token metadata must be an object"))?;
    let mut tokens = std::collections::BTreeMap::new();
    for (name, value) in metadata {
        if name.ends_with("_token") {
            if let Some(content) = value
                .as_str()
                .or_else(|| value.get("content").and_then(serde_json::Value::as_str))
            {
                tokens.insert(name.clone(), content.to_owned());
            }
        } else if name.ends_with("_special_tokens")
            && let Some(values) = value.as_array()
        {
            for (index, token) in values.iter().enumerate() {
                if let Some(content) = token.as_str() {
                    tokens.insert(format!("{name}_{index}"), content.to_owned());
                }
            }
        }
    }
    if tokens.is_empty() {
        return Err(file_error(
            &input,
            "missing special_tokens_map.json cannot be derived without token metadata",
        ));
    }
    let output = root.join("special_tokens_map.json");
    fs::write(
        &output,
        serde_json::to_vec_pretty(&tokens).map_err(|e| file_error(&output, e))?,
    )
    .map_err(|e| file_error(&output, e))
}

fn copy_file(from: &Path, to: &Path) -> Result<(), FrameworkError> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).map_err(|e| file_error(parent, e))?;
    }
    fs::copy(from, to).map_err(|e| file_error(from, e))?;
    Ok(())
}
fn load_sparse(name: &str, cache: &Path) -> Result<(SparseRuntime, ResolvedModel), FrameworkError> {
    let name = match name {
        "prithivida/Splade_PP_en_v1" => "Qdrant/Splade_PP_en_v1",
        "opensearch-project/opensearch-neural-sparse-encoding-doc-v3-gte" => {
            "Qdrant/opensearch-neural-sparse-encoding-doc-v3-gte"
        }
        name => name,
    };
    let model = name
        .parse::<SparseModel>()
        .map_err(|e| FrameworkError::param(format!("SEARCH_EMBED_SPARSE_MODEL: {e}")))?;
    let profile = profiles::default_profile();
    if model == SparseModel::BGEM3 {
        // The sparse byte loader cannot attach external tensors. Fastembed's joint
        // BGE-M3 export has a path loader and the same learned sparse head, in int8.
        let info = Bgem3Embedding::get_model_info(&Bgem3Model::BGEM3Q);
        let mut names = tokenizer_names(&profile);
        names.push(info.model_file.clone());
        names.extend(info.additional_files);
        let source = ModelSource::HuggingFace {
            repository: info.model_code,
            revision: None,
        };
        let resolved = resolve_model(&source, cache, Precision::Int8, &names, None)?;
        let graph = safe_path(&resolved.root, &info.model_file)?;
        let runtime = Bgem3Embedding::try_new_from_path(
            graph.clone(),
            tokenizers(&resolved.root, &profile)?,
            InitOptionsUserDefined::new()
                .with_max_length(8192)
                .with_intra_threads(4),
        )
        .map_err(|e| file_error(&graph, e))?;
        return Ok((SparseRuntime::Bgem3(runtime), resolved));
    }
    let info = SparseTextEmbedding::get_model_info(&model);
    let mut names = tokenizer_names(&profile);
    names.push(info.model_file.clone());
    names.extend(info.additional_files);
    let source = ModelSource::HuggingFace {
        repository: info.model_code,
        revision: None,
    };
    let resolved = resolve_model(&source, cache, Precision::Fp32, &names, None)?;
    let mut definition = UserDefinedSparseModel::new(
        read_file(&resolved.root, &info.model_file)?,
        tokenizers(&resolved.root, &profile)?,
        model,
    );
    if names.iter().any(|name| name == "idf.json") {
        definition = definition.with_idf_file(read_file(&resolved.root, "idf.json")?);
    }
    let runtime = SparseTextEmbedding::try_new_from_user_defined(
        definition,
        InitOptionsUserDefined::new()
            .with_max_length(512)
            .with_intra_threads(4),
    )
    .map_err(|e| FrameworkError::internal(format!("SEARCH_EMBED_SPARSE_MODEL {name}: {e}")))?;
    Ok((SparseRuntime::Text(runtime), resolved))
}
