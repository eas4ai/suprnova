//! Register model metadata before loading weights or resolving artifacts.

use super::{EmbedInput, Embedder, Embedding, Metered, ModelProfile, SparseEmbedding};
use crate::FrameworkError;
use async_trait::async_trait;
use std::{future::Future, pin::Pin, sync::Arc};
use tokio::sync::OnceCell;

type LoadFuture = Pin<Box<dyn Future<Output = Result<Arc<dyn Embedder>, FrameworkError>> + Send>>;
type Loader = Arc<dyn Fn() -> LoadFuture + Send + Sync>;

/// A shipped profile lets bootstrap register a model without downloading its weights.
pub struct LazyEmbedder {
    profile: ModelProfile,
    load: Loader,
    embedder: OnceCell<Arc<dyn Embedder>>,
}

impl LazyEmbedder {
    /// Validate metadata now and defer loading until inference or generation pinning needs it.
    pub fn new<F, Fut>(profile: ModelProfile, load: F) -> Result<Self, FrameworkError>
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Arc<dyn Embedder>, FrameworkError>> + Send + 'static,
    {
        profile.validate()?;
        Ok(Self {
            profile,
            load: Arc::new(move || Box::pin(load())),
            embedder: OnceCell::new(),
        })
    }

    /// Inspect whether loading succeeded without starting a download.
    pub fn loaded(&self) -> Option<&Arc<dyn Embedder>> {
        self.embedder.get()
    }

    async fn get(&self) -> Result<&Arc<dyn Embedder>, FrameworkError> {
        self.embedder
            .get_or_try_init(|| async {
                let embedder = (self.load)().await?;
                let profile = embedder.profile();
                if profile.id != self.profile.id || profile.dimensions != self.profile.dimensions {
                    return Err(FrameworkError::param(format!(
                        "lazy embedding model '{}' ({} dimensions) loaded '{}' ({} dimensions): id and dimensions must match the shipped profile",
                        self.profile.id, self.profile.dimensions, profile.id, profile.dimensions
                    )));
                }
                profile.validate()?;
                Ok(embedder)
            })
            .await
    }
}

#[async_trait]
impl Embedder for LazyEmbedder {
    fn profile(&self) -> &ModelProfile {
        self.loaded()
            .map(|embedder| embedder.profile())
            .unwrap_or(&self.profile)
    }

    async fn resolve_profile(&self) -> Result<ModelProfile, FrameworkError> {
        Ok(self.get().await?.profile().clone())
    }

    async fn embed_documents(
        &self,
        inputs: &[EmbedInput],
    ) -> Result<Vec<Embedding>, FrameworkError> {
        self.get().await?.embed_documents(inputs).await
    }

    async fn embed_query(&self, text: &str) -> Result<Embedding, FrameworkError> {
        self.get().await?.embed_query(text).await
    }

    async fn embed_sparse_query(&self, text: &str) -> Result<SparseEmbedding, FrameworkError> {
        self.get().await?.embed_sparse_query(text).await
    }

    async fn embed_sparse(&self, texts: &[String]) -> Result<Vec<SparseEmbedding>, FrameworkError> {
        self.get().await?.embed_sparse(texts).await
    }

    async fn embed_documents_metered(
        &self,
        inputs: &[EmbedInput],
    ) -> Result<Metered<Vec<Embedding>>, FrameworkError> {
        self.get().await?.embed_documents_metered(inputs).await
    }

    async fn embed_query_metered(&self, text: &str) -> Result<Metered<Embedding>, FrameworkError> {
        self.get().await?.embed_query_metered(text).await
    }
}
