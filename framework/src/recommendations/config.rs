//! Typed configuration keeps recording and generation on the same scoring contract.
use super::event::EventScoring;
use crate::{Config, FrameworkError};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
};

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
    /// RECOMMEND_ENGINE picks the engine: `starter`, or `graph`, which falls back to the
    /// starter's rows when its confidence is low.
    pub engine: String,
    /// RECOMMEND_GRAPH_STORE picks where the graph lives: `database`, the application's
    /// own database, or `memgraph` (feature `graph-memgraph`).
    pub graph_store: String,
    /// RECOMMEND_GRAPH_URL is the Memgraph server's Bolt address. Its user and password
    /// come from RECOMMEND_GRAPH_USERNAME and RECOMMEND_GRAPH_PASSWORD, read only when the
    /// store connects, so they never appear in this value.
    pub graph_url: Option<String>,
    /// RECOMMEND_GRAPH_DEPTH is the most hops a graph path takes, 1 to 4.
    pub graph_depth: usize,
    /// RECOMMEND_GRAPH_FANOUT is the most edges followed out of one node per hop.
    pub graph_fanout: usize,
    /// RECOMMEND_GRAPH_MAX_NODES is the most nodes one walk reaches.
    pub graph_max_nodes: usize,
    /// RECOMMEND_PUBLIC_EVENT_TYPES lists the event types other viewers may see, comma
    /// separated. Only these become per-actor graph facts and social signals; every other
    /// type enters the graph only as an aggregate of at least RECOMMEND_MIN_PEERS actors.
    pub public_event_types: BTreeSet<String>,
    /// RECOMMEND_SOCIAL_MIN_ACTORS is how many followed actors must agree on a target
    /// before it gains social proximity.
    pub social_min_actors: usize,
    /// RECOMMEND_ACTOR_INDEX names the search index whose documents are users, keyed by
    /// user id, so a follow of one of its documents names that user.
    pub actor_index: Option<String>,
    /// RECOMMEND_GRAPH_SURFACES is a JSON object of per-surface weights, confidence
    /// thresholds and diversity caps; a surface it does not name uses the defaults.
    pub graph_surfaces: BTreeMap<String, GraphSurface>,
}

/// How the graph engine ranks one surface. Values are basis points, 0 to 10000.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GraphSurface {
    /// Weight of paths from the viewer's positive targets through the graph.
    pub affinity: u32,
    /// Weight of nearness in the search collection's vectors.
    pub semantic: u32,
    /// Weight of followed actors' public engagement.
    pub social: u32,
    /// Weight of the target's recency.
    pub freshness: u32,
    /// Weight of the viewer's negatives near the target, subtracted.
    pub feedback: u32,
    /// Weight of the target's hide and report events against its positives, subtracted.
    pub trust: u32,
    /// Below this share of a list filled with graph or social evidence, the surface
    /// answers with the starter's rows instead.
    pub threshold: u32,
    /// The most results in one list that share one facet value.
    pub per_value: usize,
    /// The most results in one list reached through one intermediate node.
    pub per_node: usize,
    /// The most results in one list from the most popular tenth of the catalog.
    pub per_band: usize,
}

impl Default for GraphSurface {
    fn default() -> Self {
        Self {
            affinity: 4_000,
            semantic: 2_500,
            social: 1_500,
            freshness: 1_000,
            feedback: 1_000,
            trust: 2_000,
            threshold: 5_000,
            per_value: 3,
            per_node: 3,
            per_band: 3,
        }
    }
}

impl GraphSurface {
    fn validate(&self, surface: &str) -> Result<(), FrameworkError> {
        for (name, value) in [
            ("affinity", self.affinity),
            ("semantic", self.semantic),
            ("social", self.social),
            ("freshness", self.freshness),
            ("feedback", self.feedback),
            ("trust", self.trust),
            ("threshold", self.threshold),
        ] {
            if value > 10_000 {
                return Err(FrameworkError::param(format!(
                    "RECOMMEND_GRAPH_SURFACES {surface}.{name}: expected 0 to 10000 basis points"
                )));
            }
        }
        for (name, value) in [
            ("per_value", self.per_value),
            ("per_node", self.per_node),
            ("per_band", self.per_band),
        ] {
            if value == 0 {
                return Err(FrameworkError::param(format!(
                    "RECOMMEND_GRAPH_SURFACES {surface}.{name}: expected a positive cap"
                )));
            }
        }
        Ok(())
    }
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
            engine: "starter".into(),
            graph_store: "database".into(),
            graph_url: None,
            graph_depth: 2,
            graph_fanout: 50,
            graph_max_nodes: 2_000,
            public_event_types: BTreeSet::new(),
            social_min_actors: 3,
            actor_index: None,
            graph_surfaces: BTreeMap::new(),
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
    engine: Option<String>,
    graph_store: Option<String>,
    graph_url: Option<String>,
    graph_depth: Option<String>,
    graph_fanout: Option<String>,
    graph_max_nodes: Option<String>,
    public_event_types: Option<String>,
    social_min_actors: Option<String>,
    actor_index: Option<String>,
    graph_surfaces: Option<String>,
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
            engine: raw.engine.unwrap_or(default.engine),
            graph_store: raw.graph_store.unwrap_or(default.graph_store),
            graph_url: raw.graph_url.filter(|url| !url.is_empty()),
            graph_depth: parse(
                raw.graph_depth,
                default.graph_depth,
                "RECOMMEND_GRAPH_DEPTH",
            )?,
            graph_fanout: parse(
                raw.graph_fanout,
                default.graph_fanout,
                "RECOMMEND_GRAPH_FANOUT",
            )?,
            graph_max_nodes: parse(
                raw.graph_max_nodes,
                default.graph_max_nodes,
                "RECOMMEND_GRAPH_MAX_NODES",
            )?,
            public_event_types: raw
                .public_event_types
                .map(|list| {
                    list.split(',')
                        .map(str::trim)
                        .filter(|name| !name.is_empty())
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            social_min_actors: parse(
                raw.social_min_actors,
                default.social_min_actors,
                "RECOMMEND_SOCIAL_MIN_ACTORS",
            )?,
            actor_index: raw.actor_index.filter(|index| !index.is_empty()),
            graph_surfaces: raw
                .graph_surfaces
                .map(|value| {
                    serde_json::from_str(&value).map_err(|e| {
                        FrameworkError::param(format!("RECOMMEND_GRAPH_SURFACES: {e}"))
                    })
                })
                .transpose()?
                .unwrap_or_default(),
        };
        config.validate()?;
        Ok(config)
    }

    /// The ranking settings of `surface`, or the defaults when no entry names it.
    pub fn graph_surface(&self, surface: &str) -> GraphSurface {
        self.graph_surfaces
            .get(surface)
            .cloned()
            .unwrap_or_default()
    }
    /// Zero bounds would either hide all data or make a renewable lock unsafe.
    pub fn validate(&self) -> Result<(), FrameworkError> {
        for (name, value) in [
            ("RECOMMEND_HISTORY", self.history as u64),
            ("RECOMMEND_PEERS", self.peers as u64),
            ("RECOMMEND_MIN_PEERS", self.min_peers as u64),
            ("RECOMMEND_GRAPH_FANOUT", self.graph_fanout as u64),
            ("RECOMMEND_GRAPH_MAX_NODES", self.graph_max_nodes as u64),
            ("RECOMMEND_SOCIAL_MIN_ACTORS", self.social_min_actors as u64),
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
        if !matches!(self.engine.as_str(), "starter" | "graph") {
            return Err(FrameworkError::param(
                "RECOMMEND_ENGINE: expected starter or graph",
            ));
        }
        if !matches!(self.graph_store.as_str(), "database" | "memgraph") {
            return Err(FrameworkError::param(
                "RECOMMEND_GRAPH_STORE: expected database or memgraph",
            ));
        }
        if !(1..=4).contains(&self.graph_depth) {
            return Err(FrameworkError::param(
                "RECOMMEND_GRAPH_DEPTH: expected 1 to 4 hops",
            ));
        }
        if let Some(url) = &self.graph_url
            && (!(url.starts_with("bolt://") || url.starts_with("bolt+s://")) || url.contains('@'))
        {
            return Err(FrameworkError::param(
                "RECOMMEND_GRAPH_URL: expected a bolt:// or bolt+s:// address without credentials",
            ));
        }
        for name in &self.public_event_types {
            super::event::validate_text(name, 128, "RECOMMEND_PUBLIC_EVENT_TYPES entry")?;
        }
        for (surface, settings) in &self.graph_surfaces {
            super::event::validate_text(surface, 128, "RECOMMEND_GRAPH_SURFACES surface")?;
            settings.validate(surface)?;
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
