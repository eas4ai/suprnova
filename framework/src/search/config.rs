//! Strict environment parsing keeps a mistyped search option from becoming a default.

use crate::{FrameworkError, config::Config};
use serde::{Deserialize, Serialize};

/// Built-in driver names are typed so configuration cannot silently select a fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SearchDriver {
    /// Application databases provide native full-text ranking by default.
    #[default]
    Database,
    /// Qdrant combines payload search and vector retrieval.
    Qdrant,
    /// Collection is an in-process engine for deterministic tests.
    Collection,
    /// Null accepts writes and intentionally returns no hits.
    Null,
}

impl SearchDriver {
    /// Registry keys match SEARCH_DRIVER values exactly.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Database => "database",
            Self::Qdrant => "qdrant",
            Self::Collection => "collection",
            Self::Null => "null",
        }
    }
}

/// Built-in embedding provider names leave the no-embedding choice explicit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SearchEmbedder {
    /// Lexical indexes do not require an embedding provider.
    #[default]
    None,
    /// An OpenAI-compatible HTTP service supplies vectors.
    Http,
    /// A local ONNX provider supplies vectors on blocking threads.
    Fastembed,
}

impl SearchEmbedder {
    /// Provider registry keys match SEARCH_EMBEDDER values.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Http => "http",
            Self::Fastembed => "fastembed",
        }
    }
}

/// Search configuration includes the controls shared by indexing and retrieval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SearchConfig {
    /// SEARCH_DRIVER selects the default retrieval engine.
    pub driver: SearchDriver,
    /// SEARCH_EMBEDDER selects the default semantic provider.
    pub embedder: SearchEmbedder,
    /// SEARCH_QUEUE defers indexing through the queue when true.
    pub queue: bool,
    /// SEARCH_QUEUE_CONNECTION chooses a registered queue connection.
    pub queue_connection: Option<String>,
    /// SEARCH_QUEUE_NAME chooses the queue search jobs use.
    pub queue_name: Option<String>,
    /// SEARCH_CHUNK bounds memory in bulk indexing operations.
    pub chunk: usize,
    /// SEARCH_EXPANSION_TERMS bounds alternatives per query word.
    pub expansion_terms: usize,
    /// SEARCH_HITS_CAP bounds accessible ranked results.
    pub hits_cap: usize,
    /// SEARCH_FUSION_DEPTH bounds each hybrid retrieval leg.
    pub fusion_depth: usize,
    /// SEARCH_BM25_AVERAGE_LENGTH is the fixed BM25 document length in tokens (default 256).
    pub bm25_average_length: usize,
    /// SEARCH_TOMBSTONE_HOURS retains removals against delayed retries.
    pub tombstone_hours: u64,
    /// SEARCH_ANALYTICS opts into retaining search activity.
    pub analytics: bool,
    /// SEARCH_ANALYTICS_RETENTION_DAYS bounds activity retention.
    pub analytics_retention_days: u64,
    /// SEARCH_ANALYTICS_VIEWER opts into retaining authenticated viewer ids.
    pub analytics_viewer: bool,
    /// SEARCH_MODEL_CACHE names the directory for verified embedding model files.
    pub model_cache: String,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            driver: SearchDriver::Database,
            embedder: SearchEmbedder::None,
            queue: true,
            queue_connection: None,
            queue_name: None,
            chunk: 500,
            expansion_terms: 64,
            hits_cap: 1000,
            fusion_depth: 200,
            bm25_average_length: 256,
            tombstone_hours: 24,
            analytics: false,
            analytics_retention_days: 90,
            analytics_viewer: false,
            model_cache: "storage/search/models".into(),
        }
    }
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawSearchConfig {
    driver: Option<String>,
    embedder: Option<String>,
    queue: Option<String>,
    queue_connection: Option<String>,
    queue_name: Option<String>,
    chunk: Option<String>,
    expansion_terms: Option<String>,
    hits_cap: Option<String>,
    fusion_depth: Option<String>,
    bm25_average_length: Option<String>,
    tombstone_hours: Option<String>,
    analytics: Option<String>,
    analytics_retention_days: Option<String>,
    analytics_viewer: Option<String>,
    model_cache: Option<String>,
}

tokio::task_local! { static SCOPED_CONFIG: SearchConfig; }

impl SearchConfig {
    /// Resolve SEARCH_ variables through Config and report the exact failing variable.
    pub fn from_env() -> Result<Self, FrameworkError> {
        let raw = Config::resolve_prefixed::<RawSearchConfig>("SEARCH_")
            .map_err(|error| FrameworkError::param(format!("SEARCH_ configuration: {error}")))?;
        Self::from_raw(raw)
    }

    /// A registered configuration permits deterministic bootstrap and test overrides.
    pub fn current() -> Result<Self, FrameworkError> {
        if let Ok(config) = SCOPED_CONFIG.try_with(Clone::clone) {
            return Ok(config);
        }
        let config = match Config::get::<Self>() {
            Some(config) => config,
            None => return Self::from_env(),
        };
        config.validate()?;
        Ok(config)
    }

    /// Isolate a configuration override for one async operation and its awaited work.
    pub async fn scope<F: std::future::Future>(
        config: Self,
        future: F,
    ) -> Result<F::Output, FrameworkError> {
        config.validate()?;
        Ok(SCOPED_CONFIG.scope(config, future).await)
    }

    /// Refuse zero limits and empty destinations before an indexing operation begins.
    pub fn validate(&self) -> Result<(), FrameworkError> {
        for (name, value) in [
            ("SEARCH_CHUNK", self.chunk),
            ("SEARCH_EXPANSION_TERMS", self.expansion_terms),
            ("SEARCH_HITS_CAP", self.hits_cap),
            ("SEARCH_FUSION_DEPTH", self.fusion_depth),
            ("SEARCH_BM25_AVERAGE_LENGTH", self.bm25_average_length),
        ] {
            if value == 0 {
                return Err(FrameworkError::param(name));
            }
        }
        if self.tombstone_hours == 0 {
            return Err(FrameworkError::param("SEARCH_TOMBSTONE_HOURS"));
        }
        if self.analytics_retention_days == 0 {
            return Err(FrameworkError::param("SEARCH_ANALYTICS_RETENTION_DAYS"));
        }
        if self.model_cache.trim().is_empty() {
            return Err(FrameworkError::param("SEARCH_MODEL_CACHE"));
        }
        if self
            .queue_connection
            .as_ref()
            .is_some_and(|v| v.trim().is_empty())
        {
            return Err(FrameworkError::param("SEARCH_QUEUE_CONNECTION"));
        }
        if self
            .queue_name
            .as_ref()
            .is_some_and(|v| v.trim().is_empty())
        {
            return Err(FrameworkError::param("SEARCH_QUEUE_NAME"));
        }
        Ok(())
    }

    fn from_raw(raw: RawSearchConfig) -> Result<Self, FrameworkError> {
        fn parse<T: std::str::FromStr>(
            value: Option<String>,
            default: T,
            name: &str,
        ) -> Result<T, FrameworkError> {
            match value {
                Some(value) => value.parse().map_err(|_| FrameworkError::param(name)),
                None => Ok(default),
            }
        }
        let default = Self::default();
        let driver = match raw.driver.as_deref().unwrap_or("database") {
            "database" => SearchDriver::Database,
            "qdrant" => SearchDriver::Qdrant,
            "collection" => SearchDriver::Collection,
            "null" => SearchDriver::Null,
            _ => return Err(FrameworkError::param("SEARCH_DRIVER")),
        };
        let embedder = match raw.embedder.as_deref().unwrap_or("none") {
            "none" => SearchEmbedder::None,
            "http" => SearchEmbedder::Http,
            "fastembed" => SearchEmbedder::Fastembed,
            _ => return Err(FrameworkError::param("SEARCH_EMBEDDER")),
        };
        let config = Self {
            driver,
            embedder,
            queue: parse(raw.queue, default.queue, "SEARCH_QUEUE")?,
            queue_connection: raw.queue_connection,
            queue_name: raw.queue_name,
            chunk: parse(raw.chunk, default.chunk, "SEARCH_CHUNK")?,
            expansion_terms: parse(
                raw.expansion_terms,
                default.expansion_terms,
                "SEARCH_EXPANSION_TERMS",
            )?,
            hits_cap: parse(raw.hits_cap, default.hits_cap, "SEARCH_HITS_CAP")?,
            fusion_depth: parse(
                raw.fusion_depth,
                default.fusion_depth,
                "SEARCH_FUSION_DEPTH",
            )?,
            bm25_average_length: parse(
                raw.bm25_average_length,
                default.bm25_average_length,
                "SEARCH_BM25_AVERAGE_LENGTH",
            )?,
            tombstone_hours: parse(
                raw.tombstone_hours,
                default.tombstone_hours,
                "SEARCH_TOMBSTONE_HOURS",
            )?,
            analytics: parse(raw.analytics, default.analytics, "SEARCH_ANALYTICS")?,
            analytics_retention_days: parse(
                raw.analytics_retention_days,
                default.analytics_retention_days,
                "SEARCH_ANALYTICS_RETENTION_DAYS",
            )?,
            analytics_viewer: parse(
                raw.analytics_viewer,
                default.analytics_viewer,
                "SEARCH_ANALYTICS_VIEWER",
            )?,
            model_cache: raw.model_cache.unwrap_or(default.model_cache),
        };
        config.validate()?;
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_environment_values_name_the_variable() {
        let raw = RawSearchConfig {
            chunk: Some("potato".into()),
            ..Default::default()
        };
        assert!(
            SearchConfig::from_raw(raw)
                .unwrap_err()
                .to_string()
                .contains("SEARCH_CHUNK")
        );
        let raw = RawSearchConfig {
            driver: Some("typo".into()),
            ..Default::default()
        };
        assert!(
            SearchConfig::from_raw(raw)
                .unwrap_err()
                .to_string()
                .contains("SEARCH_DRIVER")
        );
        let raw = RawSearchConfig {
            analytics: Some("yes".into()),
            ..Default::default()
        };
        assert!(
            SearchConfig::from_raw(raw)
                .unwrap_err()
                .to_string()
                .contains("SEARCH_ANALYTICS")
        );
    }

    #[test]
    fn bm25_average_length_requires_positive_tokens() {
        assert_eq!(SearchConfig::default().bm25_average_length, 256);
        for value in ["0", "-1", "potato", "NaN", "1.5"] {
            let raw = RawSearchConfig {
                bm25_average_length: Some(value.into()),
                ..Default::default()
            };
            assert!(
                SearchConfig::from_raw(raw)
                    .unwrap_err()
                    .to_string()
                    .contains("SEARCH_BM25_AVERAGE_LENGTH")
            );
        }
        let raw = RawSearchConfig {
            bm25_average_length: Some("128".into()),
            ..Default::default()
        };
        assert_eq!(
            SearchConfig::from_raw(raw).unwrap().bm25_average_length,
            128
        );
    }

    #[test]
    fn unset_values_use_documented_defaults() {
        assert_eq!(
            SearchConfig::from_raw(RawSearchConfig::default()).unwrap(),
            SearchConfig::default()
        );
    }
}
