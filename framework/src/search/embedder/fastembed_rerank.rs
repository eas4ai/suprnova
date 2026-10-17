//! Rerankers initialize lazily so bootstrap does not download every cross-encoder.

use super::{
    Precision,
    fastembed::{ModelSource, resolve_model, tokenizer_names, tokenizers},
    model_cache::{file_error, safe_path},
    profiles,
};
use crate::{
    FrameworkError,
    search::{
        SearchDocument,
        registry::{Reranker, SearchRegistry},
    },
};
use async_trait::async_trait;
use fastembed::{RerankInitOptions, RerankerModel, TextRerank, UserDefinedRerankingModel};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

/// One cross-encoder is shared behind a blocking lock; query code sees only positional scores.
#[derive(Clone)]
pub struct FastembedReranker {
    model: RerankerModel,
    cache: PathBuf,
    batch: usize,
    runtime: Arc<Mutex<Option<TextRerank>>>,
}
impl FastembedReranker {
    /// Validate a named reranker without downloading it before a query actually needs it.
    pub fn new(model: &str, cache: PathBuf, batch: usize) -> Result<Self, FrameworkError> {
        if batch == 0 {
            return Err(FrameworkError::param("SEARCH_EMBED_BATCH must be positive"));
        }
        if cache.as_os_str().is_empty() {
            return Err(FrameworkError::param("SEARCH_MODEL_CACHE is empty"));
        }
        let name = match model {
            "BAAI/bge-reranker-v2-m3" => "rozgo/bge-reranker-v2-m3",
            name => name,
        };
        let model = name
            .parse::<RerankerModel>()
            .or_else(|_| {
                TextRerank::list_supported_models()
                    .into_iter()
                    .find(|info| {
                        info.model_code
                            .rsplit('/')
                            .next()
                            .is_some_and(|short| short.eq_ignore_ascii_case(name))
                            || format!("{:?}", info.model).eq_ignore_ascii_case(name)
                    })
                    .map(|info| info.model)
                    .ok_or_else(|| format!("unknown reranker {name}"))
            })
            .map_err(|e| FrameworkError::param(format!("rerank model: {e}")))?;
        Ok(Self {
            model,
            cache,
            batch,
            runtime: Arc::new(Mutex::new(None)),
        })
    }
    fn load(&self) -> Result<TextRerank, FrameworkError> {
        let info = TextRerank::get_model_info(&self.model);
        let profile = profiles::default_profile();
        let mut names = tokenizer_names(&profile);
        names.push(info.model_file.clone());
        names.extend(info.additional_files);
        let source = ModelSource::HuggingFace {
            repository: info.model_code,
            revision: None,
        };
        let resolved = resolve_model(&source, &self.cache, Precision::Fp32, &names, None)?;
        let graph = safe_path(&resolved.root, &info.model_file)?;
        let model =
            UserDefinedRerankingModel::new(graph.clone(), tokenizers(&resolved.root, &profile)?);
        TextRerank::try_new_from_user_defined(
            model,
            RerankInitOptions::new(self.model.clone())
                .with_intra_threads(4)
                .into(),
        )
        .map_err(|e| file_error(&graph, e))
    }
    /// Register each fastembed reranker under its repository, short name and enum spelling.
    /// These share a lazy runtime, making the builder's existing lookup sufficient.
    pub fn register_supported(cache: PathBuf, batch: usize) -> Result<(), FrameworkError> {
        for info in TextRerank::list_supported_models() {
            let provider = Arc::new(Self::new(&info.model_code, cache.clone(), batch)?);
            SearchRegistry::register_reranker_default(&info.model_code, provider.clone())?;
            if let Some(short) = info.model_code.rsplit('/').next() {
                SearchRegistry::register_reranker_default(short, provider.clone())?;
            }
            SearchRegistry::register_reranker_default(
                &format!("{:?}", info.model),
                provider.clone(),
            )?;
            if info.model == RerankerModel::BGERerankerV2M3 {
                SearchRegistry::register_reranker_default("BAAI/bge-reranker-v2-m3", provider)?;
            }
        }
        Ok(())
    }
}
#[async_trait]
impl Reranker for FastembedReranker {
    async fn rerank(
        &self,
        query: &str,
        documents: &[SearchDocument],
    ) -> Result<Vec<f64>, FrameworkError> {
        if documents.is_empty() {
            return Ok(vec![]);
        }
        let query = query.to_owned();
        let documents = documents
            .iter()
            .map(|document| {
                document.embed_text.clone().unwrap_or_else(|| {
                    document
                        .attributes
                        .values()
                        .filter_map(|value| value.as_text())
                        .collect::<Vec<_>>()
                        .join(" ")
                })
            })
            .collect::<Vec<_>>();
        let provider = self.clone();
        tokio::task::spawn_blocking(move || {
            let mut runtime = provider
                .runtime
                .lock()
                .map_err(|_| FrameworkError::internal("rerank runtime lock poisoned"))?;
            if runtime.is_none() {
                *runtime = Some(provider.load()?);
            }
            let model = runtime
                .as_mut()
                .ok_or_else(|| FrameworkError::internal("rerank runtime was not initialized"))?;
            let mut rescored = model
                .rerank(query, &documents, false, Some(provider.batch))
                .map_err(|e| {
                    FrameworkError::internal(format!("rerank model {:?}: {e}", provider.model))
                })?;
            rescored.sort_by_key(|result| result.index);
            if rescored.len() != documents.len()
                || rescored
                    .iter()
                    .enumerate()
                    .any(|(index, result)| index != result.index || !result.score.is_finite())
            {
                return Err(FrameworkError::param(format!(
                    "rerank model {:?} returned invalid indices or scores",
                    provider.model
                )));
            }
            Ok(rescored
                .into_iter()
                .map(|result| f64::from(result.score))
                .collect())
        })
        .await
        .map_err(|e| FrameworkError::internal(format!("rerank task: {e}")))?
    }
}
