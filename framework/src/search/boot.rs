//! Provider boot pins deployment configuration before indexing and query work begins.
use super::{
    SearchConfig, SearchEmbedder, SearchRegistry,
    embedder::{Embedder, http::HttpEmbedder},
};
use crate::FrameworkError;
use std::sync::Arc;

static BOOT: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Construct the chosen provider once, preserving a provider installed by application bootstrap.
/// With no embedder, the core's lexical-only behavior remains authoritative.
pub async fn boot() -> Result<(), FrameworkError> {
    let config = SearchConfig::current()?;
    let _guard = BOOT.lock().await;
    #[cfg(feature = "search-fastembed")]
    {
        let (cache, batch) = if let Some(local) =
            crate::config::Config::get::<super::embedder::fastembed::FastembedConfig>()
        {
            (local.cache, local.batch)
        } else {
            (
                config.model_cache.clone().into(),
                super::embedder::http::parse(
                    std::env::var("SEARCH_EMBED_BATCH").ok(),
                    64,
                    "SEARCH_EMBED_BATCH",
                )?,
            )
        };
        super::embedder::fastembed::FastembedReranker::register_supported(cache, batch)?;
    }
    if config.embedder == SearchEmbedder::None
        || SearchRegistry::embedder_named("bootstrap", config.embedder.as_str()).is_ok()
    {
        return Ok(());
    }
    let embedder: Arc<dyn Embedder> = match config.embedder {
        SearchEmbedder::None => return Ok(()),
        SearchEmbedder::Http => Arc::new(HttpEmbedder::from_env().await?),
        SearchEmbedder::Fastembed => {
            #[cfg(feature = "search-fastembed")]
            {
                Arc::new(super::embedder::fastembed::FastembedEmbedder::from_env().await?)
            }
            #[cfg(not(feature = "search-fastembed"))]
            {
                return Err(FrameworkError::param(
                    "SEARCH_EMBEDDER=fastembed requires the search-fastembed cargo feature",
                ));
            }
        }
    };
    SearchRegistry::register_embedder(config.embedder.as_str(), embedder)
}
