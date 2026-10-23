//! Result envelopes keep native relevance and public document projections distinct.

use super::document::AttributeValue;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Totals expose whether a semantic retrieval window bounds the count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Total {
    /// Lexical engines count the entire filtered candidate set.
    Exact(u64),
    /// A semantic top-k can supply only an estimated total.
    Estimated(u64),
}

impl Total {
    /// Extract the count while leaving its accuracy flag in the envelope.
    pub fn value(self) -> u64 {
        match self {
            Self::Exact(n) | Self::Estimated(n) => n,
        }
    }
}

/// One facet value retains its type and count for endpoint consumers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FacetValue {
    /// The declared attribute value avoids ambiguous numeric labels.
    pub value: AttributeValue,
    /// Counts are over the complete filtered lexical candidate set.
    pub count: u64,
}

/// Named distributions preserve each engine's exact facet counting universe.
pub type FacetDistribution = BTreeMap<String, Vec<FacetValue>>;

/// Typed extrema keep datetime stats separate from numeric stats.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FacetRange {
    /// The smallest matching non-null value.
    pub min: AttributeValue,
    /// The largest matching non-null value.
    pub max: AttributeValue,
}

/// A facet with no non-null values is absent rather than inventing extrema.
pub type FacetStats = BTreeMap<String, FacetRange>;

/// Hydrated hits carry both native scores and presentation metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hit<M> {
    /// Eloquent hydration applies the model's ordinary database scopes.
    pub model: M,
    /// Keys retain engine ordering when hydration returns a different order.
    pub key: String,
    /// Native relevance is retained even when hybrid fusion changes rank.
    pub engine_score: f64,
    /// Relative relevance is max-normalized within this result to [0, 1].
    pub normalized_score: f64,
    /// Engine identity makes native ranking differences visible.
    pub engine: String,
    /// Escaped and highlighted fields are safe for HTML insertion.
    pub formatted: BTreeMap<String, String>,
    /// Distance in meters is present for a distance sort.
    pub geo_distance: Option<f64>,
    /// Additional ranked members belong to the same requested group.
    pub group_members: Vec<Hit<M>>,
}

/// Raw projections contain only declared attributes, never a hydrated model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawHit {
    /// The engine document is the projection allowed by the index schema.
    pub attributes: BTreeMap<String, AttributeValue>,
}

/// Pagination metadata describes a slice of one deterministically ranked list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchPagination {
    /// The requested page length is independent of the total counting universe.
    pub limit: usize,
    /// Offset counts ranked results after distinct and grouping.
    pub offset: usize,
    /// Page numbers are one-based for UI use.
    pub page: usize,
    /// The ranked list contains another result beyond this slice.
    pub has_more: bool,
}

/// A search response carries models, exact facets and the engine's counting accuracy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResults<M> {
    /// A server-session-bound list identity permits validated click reporting when analytics is enabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub impression: Option<String>,
    /// Hits remain in the final retrieval order after hydration.
    pub hits: Vec<Hit<M>>,
    /// A semantic leg marks its bounded count as estimated.
    pub total: Total,
    /// Facet values count the entire filtered lexical candidate set.
    pub facets: FacetDistribution,
    /// Extrema come from the same candidate set as facets.
    pub facet_stats: FacetStats,
    /// Elapsed wall time includes expansion, retrieval and hydration.
    pub processing_time_ms: u64,
    /// The engine name permits diagnostics without exposing backend details.
    pub engine: String,
    /// Original query text allows clients to associate a response with a request.
    pub query: String,
    /// Slice metadata supports both full and simple pagination.
    pub pagination: SearchPagination,
}

/// Federated hits retain index identity when model types differ across indexes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FederatedHit {
    /// Index names disambiguate the same key in different tables.
    pub index: String,
    /// Model keys remain strings throughout the engine contract.
    pub key: String,
    /// Only declared attributes may cross a heterogeneous search boundary.
    pub attributes: BTreeMap<String, AttributeValue>,
    /// RRF relevance describes rank across indexes without mixing native scores.
    pub score: f64,
    /// The producing engine remains visible in a federated response.
    pub engine: String,
    /// Formatted fields contain escaped source text and trusted highlight tags.
    pub formatted: BTreeMap<String, String>,
}

/// Federation shares pagination while preserving per-index facet labels.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FederatedResults {
    /// Hits are ordered by RRF and input index order for equal ranks.
    pub hits: Vec<FederatedHit>,
    /// Summed totals preserve an estimate when any retrieval leg is semantic.
    pub total: Total,
    /// Facets are namespaced by index to avoid combining different schemas.
    pub facets: BTreeMap<String, FacetDistribution>,
    /// Extrema are namespaced for the same reason as facets.
    pub facet_stats: BTreeMap<String, FacetStats>,
    /// Elapsed wall time includes every index search.
    pub processing_time_ms: u64,
    /// Query echo associates this response with the caller's input.
    pub query: String,
    /// Pagination slices the combined list after fusion.
    pub pagination: SearchPagination,
}
