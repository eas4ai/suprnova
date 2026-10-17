//! Typed configuration keeps recording and generation on the same scoring contract.
use super::event::EventScoring;
use crate::{Config, FrameworkError};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, future::Future};

/// Bounds keep the starter usable while the graph engine's data accumulates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RecommendConfig {
    /// RECOMMEND_ENABLED opts validated search analytics into the event stream.
    pub enabled: bool,
    /// RECOMMEND_HISTORY caps each actor's newest positive and negative facts.
    pub history: usize,
    /// RECOMMEND_PEERS caps peers ordered by their most recent shared fact.
    pub peers: usize,
    /// RECOMMEND_MIN_PEERS is how many distinct peers a collaborative candidate needs, so
    /// no suggestion's peer count can point at one person's views or purchases.
    pub min_peers: usize,
    /// RECOMMEND_POPULARITY_WINDOW counts UTC days of positive activity.
    pub popularity_window: u32,
    /// RECOMMEND_LOCK_TTL_SECS bounds the renewable generation lease.
    pub lock_ttl_secs: u64,
    /// RECOMMEND_EVENT_TYPES is a JSON object of application scoring overrides.
    pub event_types: BTreeMap<String, EventScoring>,
}
impl Default for RecommendConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            history: 500,
            peers: 200,
            min_peers: 3,
            popularity_window: 30,
            lock_ttl_secs: 3600,
            event_types: BTreeMap::new(),
        }
    }
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct RawConfig {
    enabled: Option<String>,
    history: Option<String>,
    peers: Option<String>,
    min_peers: Option<String>,
    popularity_window: Option<String>,
    lock_ttl_secs: Option<String>,
    event_types: Option<String>,
}
tokio::task_local! { static SCOPED: RecommendConfig; }
impl RecommendConfig {
    /// Config resolution respects registered configuration as well as environment prefixes.
    pub fn current() -> Result<Self, FrameworkError> {
        let config = match SCOPED
            .try_with(Clone::clone)
            .ok()
            .or_else(Config::get::<Self>)
        {
            Some(config) => config,
            None => return Self::from_env(),
        };
        config.validate()?;
        Ok(config)
    }
    /// Invalid environment values identify the variable instead of selecting a fallback.
    pub fn from_env() -> Result<Self, FrameworkError> {
        let raw = Config::resolve_prefixed::<RawConfig>("RECOMMEND_")
            .map_err(|e| FrameworkError::param(format!("RECOMMEND_ configuration: {e}")))?;
        Self::from_raw(raw)
    }
    fn from_raw(raw: RawConfig) -> Result<Self, FrameworkError> {
        fn parse<T: std::str::FromStr>(
            value: Option<String>,
            default: T,
            name: &str,
        ) -> Result<T, FrameworkError> {
            value.map_or(Ok(default), |v| {
                v.parse()
                    .map_err(|_| FrameworkError::param(format!("{name}: invalid value")))
            })
        }
        let default = Self::default();
        let config = Self {
            enabled: parse(raw.enabled, default.enabled, "RECOMMEND_ENABLED")?,
            history: parse(raw.history, default.history, "RECOMMEND_HISTORY")?,
            peers: parse(raw.peers, default.peers, "RECOMMEND_PEERS")?,
            min_peers: parse(raw.min_peers, default.min_peers, "RECOMMEND_MIN_PEERS")?,
            popularity_window: parse(
                raw.popularity_window,
                default.popularity_window,
                "RECOMMEND_POPULARITY_WINDOW",
            )?,
            lock_ttl_secs: parse(
                raw.lock_ttl_secs,
                default.lock_ttl_secs,
                "RECOMMEND_LOCK_TTL_SECS",
            )?,
            event_types: raw
                .event_types
                .map(|value| {
                    serde_json::from_str(&value)
                        .map_err(|e| FrameworkError::param(format!("RECOMMEND_EVENT_TYPES: {e}")))
                })
                .transpose()?
                .unwrap_or_default(),
        };
        config.validate()?;
        Ok(config)
    }
    /// Zero bounds would either hide all data or make a renewable lock unsafe.
    pub fn validate(&self) -> Result<(), FrameworkError> {
        for (name, value) in [
            ("RECOMMEND_HISTORY", self.history as u64),
            ("RECOMMEND_PEERS", self.peers as u64),
            ("RECOMMEND_MIN_PEERS", self.min_peers as u64),
            (
                "RECOMMEND_POPULARITY_WINDOW",
                u64::from(self.popularity_window),
            ),
            ("RECOMMEND_LOCK_TTL_SECS", self.lock_ttl_secs),
        ] {
            if value == 0 || value > i64::MAX as u64 {
                return Err(FrameworkError::param(format!(
                    "{name}: expected a positive signed integer"
                )));
            }
        }
        if std::time::Instant::now()
            .checked_add(std::time::Duration::from_secs(self.lock_ttl_secs))
            .is_none()
        {
            return Err(FrameworkError::param(
                "RECOMMEND_LOCK_TTL_SECS exceeds the platform clock range",
            ));
        }
        if crate::clock::now()
            .checked_sub_signed(chrono::Duration::days(i64::from(self.popularity_window)))
            .is_none()
        {
            return Err(FrameworkError::param(
                "RECOMMEND_POPULARITY_WINDOW exceeds the supported calendar range",
            ));
        }
        for (name, scoring) in &self.event_types {
            super::event::validate_text(name, 128, "event type override")?;
            scoring.validate()?;
        }
        Ok(())
    }
    /// Task-local overrides let parallel tests and background jobs isolate their configuration.
    pub async fn scope<F: Future>(config: Self, future: F) -> Result<F::Output, FrameworkError> {
        config.validate()?;
        Ok(SCOPED.scope(config, future).await)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_environment_values_name_the_variable() {
        for (raw, name) in [
            (
                RawConfig {
                    history: Some("zero".into()),
                    ..Default::default()
                },
                "RECOMMEND_HISTORY",
            ),
            (
                RawConfig {
                    peers: Some("0".into()),
                    ..Default::default()
                },
                "RECOMMEND_PEERS",
            ),
            (
                RawConfig {
                    event_types: Some("[]".into()),
                    ..Default::default()
                },
                "RECOMMEND_EVENT_TYPES",
            ),
        ] {
            assert!(
                RecommendConfig::from_raw(raw)
                    .unwrap_err()
                    .to_string()
                    .contains(name)
            );
        }
        assert_eq!(
            RecommendConfig::from_raw(RawConfig::default()).unwrap(),
            RecommendConfig::default()
        );
    }
}
