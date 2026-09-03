//! Route and group RenderCache policy, deterministic patches, and the
//! concrete-response eligibility decision.

use std::collections::BTreeSet;

use super::{RenderCacheError, RenderCacheErrorKind};

/// Upper bound on any freshness interval: 31 days in milliseconds.
pub const MAX_INTERVAL_MS: u64 = 31 * 24 * 60 * 60 * 1000;
/// Upper bound on declared query parameter names per route.
pub const MAX_DECLARED_QUERY: usize = 32;

/// A dimension along which a representation varies beyond route, query,
/// media type, and build. A route declares the dimensions it needs; each
/// declared dimension joins the cache key.
///
/// This is a minimal placeholder pending the full variance contract added in
/// a later task; it carries only the dimension this task's policy exercises.
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub enum VarianceDimension {
    /// The negotiated locale.
    Locale,
}

/// How a representation may be shared. Order is widest to narrowest sharing.
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub enum RepresentationClass {
    /// One representation for every request that matches the public dimensions.
    PublicShared,
    /// A shared shell with request-specific stitched segments (assembled later).
    PublicShellStitched,
    /// One representation per private key material set.
    PrivateCached,
    /// Never stored.
    Uncacheable,
}

impl RepresentationClass {
    /// Returns the narrower of two classes; sharing only ever reduces.
    #[must_use]
    pub fn narrowest(self, other: Self) -> Self {
        self.max(other)
    }
}

/// Fresh, stale-servable, and stale-on-error intervals in milliseconds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FreshnessPolicy {
    fresh_ms: u64,
    stale_servable_ms: u64,
    stale_on_error_ms: u64,
}

impl FreshnessPolicy {
    /// Builds bounded intervals; each is at most [`MAX_INTERVAL_MS`].
    pub fn new(
        fresh_ms: u64,
        stale_servable_ms: u64,
        stale_on_error_ms: u64,
    ) -> Result<Self, RenderCacheError> {
        if [fresh_ms, stale_servable_ms, stale_on_error_ms]
            .iter()
            .any(|value| *value > MAX_INTERVAL_MS)
        {
            return Err(RenderCacheError::new(RenderCacheErrorKind::PolicyInvalid));
        }
        Ok(Self {
            fresh_ms,
            stale_servable_ms,
            stale_on_error_ms,
        })
    }

    /// Milliseconds a representation is fresh after publication.
    #[must_use]
    pub const fn fresh_ms(&self) -> u64 {
        self.fresh_ms
    }

    /// Milliseconds after freshness during which stale service is permitted.
    #[must_use]
    pub const fn stale_servable_ms(&self) -> u64 {
        self.stale_servable_ms
    }

    /// Milliseconds after freshness during which stale-on-error is permitted.
    #[must_use]
    pub const fn stale_on_error_ms(&self) -> u64 {
        self.stale_on_error_ms
    }
}

/// Which storage layers a policy populates.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StorageLayers {
    l0: bool,
    l1: bool,
}

impl StorageLayers {
    /// In-process only.
    #[must_use]
    pub const fn l0_only() -> Self {
        Self {
            l0: true,
            l1: false,
        }
    }

    /// In-process and the configured L1 provider.
    #[must_use]
    pub const fn l0_and_l1() -> Self {
        Self { l0: true, l1: true }
    }

    /// Whether L0 participates.
    #[must_use]
    pub const fn l0(&self) -> bool {
        self.l0
    }

    /// Whether L1 participates.
    #[must_use]
    pub const fn l1(&self) -> bool {
        self.l1
    }
}

/// How currentness is proved on a hit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoherenceMode {
    /// Reread the generation authority on every hit.
    Authority,
    /// Trust a local validation lease of at most this many milliseconds.
    Lease {
        /// Maximum milliseconds a validation lease may be trusted before a
        /// fresh authority read is required.
        max_age_ms: u64,
    },
}

/// External shared-cache directive policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SharedCachePolicy {
    /// `Cache-Control: private`; shared caches never store the response.
    Private,
    /// Bounded `s-maxage`; shared caches may store for this many seconds.
    SMaxAge {
        /// Bounded `s-maxage` value in seconds.
        seconds: u32,
    },
}

/// Behavior when a store, ledger, or coordinator provider fails.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailurePolicy {
    /// Serve the route normally without caching.
    Open,
    /// Refuse the request with a closed 503.
    Closed,
}

/// What an undeclared query parameter does to lookup.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueryUnknown {
    /// Bypass the cache for the request.
    Bypass,
}

/// Declared query semantics: only declared names join the key.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QueryPolicy {
    declared: BTreeSet<String>,
    unknown: QueryUnknown,
}

impl QueryPolicy {
    /// Declares the query names that distinguish representations.
    pub fn declared<I, S>(names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            declared: names.into_iter().map(Into::into).collect(),
            unknown: QueryUnknown::Bypass,
        }
    }

    /// No query parameter joins the key; any present one bypasses.
    #[must_use]
    pub fn none() -> Self {
        Self::declared(Vec::<String>::new())
    }

    /// The declared names.
    #[must_use]
    pub fn declared_names(&self) -> &BTreeSet<String> {
        &self.declared
    }

    /// Undeclared parameter behavior.
    #[must_use]
    pub const fn unknown(&self) -> QueryUnknown {
        self.unknown
    }
}

/// The effective RenderCache policy of one route.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderCachePolicy {
    class: RepresentationClass,
    freshness: FreshnessPolicy,
    layers: StorageLayers,
    coherence: CoherenceMode,
    shared: SharedCachePolicy,
    failure: FailurePolicy,
    query: QueryPolicy,
    vary: BTreeSet<VarianceDimension>,
}

impl RenderCachePolicy {
    /// Starts a policy for one class with conservative defaults: fresh 0,
    /// L0 only, authority coherence, private shared policy, fail open, no
    /// query parameters, no extra variance.
    #[must_use]
    pub fn builder(class: RepresentationClass) -> RenderCachePolicyBuilder {
        RenderCachePolicyBuilder {
            policy: Self {
                class,
                freshness: FreshnessPolicy {
                    fresh_ms: 0,
                    stale_servable_ms: 0,
                    stale_on_error_ms: 0,
                },
                layers: StorageLayers::l0_only(),
                coherence: CoherenceMode::Authority,
                shared: SharedCachePolicy::Private,
                failure: FailurePolicy::Open,
                query: QueryPolicy::none(),
                vary: BTreeSet::new(),
            },
        }
    }

    /// The declared class before any observed downgrade.
    #[must_use]
    pub const fn class(&self) -> RepresentationClass {
        self.class
    }

    /// Freshness intervals.
    #[must_use]
    pub const fn freshness(&self) -> FreshnessPolicy {
        self.freshness
    }

    /// Storage layers.
    #[must_use]
    pub const fn layers(&self) -> StorageLayers {
        self.layers
    }

    /// Coherence mode.
    #[must_use]
    pub const fn coherence(&self) -> CoherenceMode {
        self.coherence
    }

    /// Shared-cache directive policy.
    #[must_use]
    pub const fn shared(&self) -> SharedCachePolicy {
        self.shared
    }

    /// Provider failure policy.
    #[must_use]
    pub const fn failure(&self) -> FailurePolicy {
        self.failure
    }

    /// Query semantics.
    #[must_use]
    pub fn query(&self) -> &QueryPolicy {
        &self.query
    }

    /// Declared variance dimensions beyond route, query, media, and build.
    #[must_use]
    pub fn vary(&self) -> &BTreeSet<VarianceDimension> {
        &self.vary
    }

    /// Applies a route patch to a group policy. Every field the patch names
    /// replaces the group's; a class may only narrow. Deterministic.
    pub fn apply(&self, patch: &PolicyPatch) -> Result<Self, RenderCacheError> {
        let mut next = self.clone();
        if let Some(class) = patch.class {
            if class < self.class {
                return Err(RenderCacheError::new(RenderCacheErrorKind::PolicyInvalid));
            }
            next.class = class;
        }
        if let Some(freshness) = patch.freshness {
            next.freshness = freshness;
        }
        if let Some(layers) = patch.layers {
            next.layers = layers;
        }
        if let Some(coherence) = patch.coherence {
            next.coherence = coherence;
        }
        if let Some(shared) = patch.shared {
            next.shared = shared;
        }
        if let Some(failure) = patch.failure {
            next.failure = failure;
        }
        if let Some(query) = &patch.query {
            next.query = query.clone();
        }
        if let Some(vary) = &patch.vary {
            next.vary = vary.clone();
        }
        next.validate()?;
        Ok(next)
    }

    /// Validates the bounds `build` and `apply` both must enforce: declared
    /// query names stay within [`MAX_DECLARED_QUERY`] and a lease's maximum
    /// age stays within [`MAX_INTERVAL_MS`].
    fn validate(&self) -> Result<(), RenderCacheError> {
        if self.query.declared.len() > MAX_DECLARED_QUERY {
            return Err(RenderCacheError::new(RenderCacheErrorKind::PolicyInvalid));
        }
        if let CoherenceMode::Lease { max_age_ms } = self.coherence
            && max_age_ms > MAX_INTERVAL_MS
        {
            return Err(RenderCacheError::new(RenderCacheErrorKind::PolicyInvalid));
        }
        Ok(())
    }

    /// Decides whether a concrete response may be stored, and in which class.
    /// The decision only preserves or narrows the declared class.
    #[must_use]
    pub fn eligibility(&self, signals: &ResponseSignals) -> Eligibility {
        if self.class == RepresentationClass::Uncacheable {
            return Eligibility::Decline(DeclineReason::PolicyUncacheable);
        }
        if signals.method != "GET" && signals.method != "HEAD" {
            return Eligibility::Decline(DeclineReason::Method);
        }
        if signals.status != 200 {
            return Eligibility::Decline(DeclineReason::Status);
        }
        if signals.streaming {
            return Eligibility::Decline(DeclineReason::Streaming);
        }
        if signals.sets_cookie {
            return Eligibility::Decline(DeclineReason::SetsCookie);
        }
        if signals
            .header_names
            .iter()
            .any(|name| UNSAFE_RESPONSE_HEADERS.contains(&name.to_ascii_lowercase().as_str()))
        {
            return Eligibility::Decline(DeclineReason::UnsafeHeader);
        }
        let class = if signals.private_observed {
            self.class.narrowest(RepresentationClass::PrivateCached)
        } else {
            self.class
        };
        Eligibility::Store(class)
    }
}

/// Response headers that never enter a stored representation and whose
/// presence declines storage: hop-by-hop, connection-scoped, and per-request
/// tracing headers.
pub const UNSAFE_RESPONSE_HEADERS: [&str; 9] = [
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "set-cookie",
];

/// Builder for [`RenderCachePolicy`].
pub struct RenderCachePolicyBuilder {
    policy: RenderCachePolicy,
}

impl RenderCachePolicyBuilder {
    /// Sets freshness intervals.
    #[must_use]
    pub fn freshness(mut self, freshness: FreshnessPolicy) -> Self {
        self.policy.freshness = freshness;
        self
    }

    /// Sets storage layers.
    #[must_use]
    pub fn layers(mut self, layers: StorageLayers) -> Self {
        self.policy.layers = layers;
        self
    }

    /// Sets the coherence mode.
    #[must_use]
    pub fn coherence(mut self, coherence: CoherenceMode) -> Self {
        self.policy.coherence = coherence;
        self
    }

    /// Sets the shared-cache policy.
    #[must_use]
    pub fn shared(mut self, shared: SharedCachePolicy) -> Self {
        self.policy.shared = shared;
        self
    }

    /// Sets the failure policy.
    #[must_use]
    pub fn failure(mut self, failure: FailurePolicy) -> Self {
        self.policy.failure = failure;
        self
    }

    /// Sets the query semantics.
    #[must_use]
    pub fn query(mut self, query: QueryPolicy) -> Self {
        self.policy.query = query;
        self
    }

    /// Adds one variance dimension.
    #[must_use]
    pub fn vary(mut self, dimension: VarianceDimension) -> Self {
        self.policy.vary.insert(dimension);
        self
    }

    /// Validates bounds and returns the policy.
    pub fn build(self) -> Result<RenderCachePolicy, RenderCacheError> {
        self.policy.validate()?;
        Ok(self.policy)
    }
}

/// A route-level override of a group policy; unnamed fields inherit.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PolicyPatch {
    class: Option<RepresentationClass>,
    freshness: Option<FreshnessPolicy>,
    layers: Option<StorageLayers>,
    coherence: Option<CoherenceMode>,
    shared: Option<SharedCachePolicy>,
    failure: Option<FailurePolicy>,
    query: Option<QueryPolicy>,
    vary: Option<BTreeSet<VarianceDimension>>,
}

impl PolicyPatch {
    /// Narrows the class.
    #[must_use]
    pub fn class(mut self, class: RepresentationClass) -> Self {
        self.class = Some(class);
        self
    }

    /// Replaces freshness.
    #[must_use]
    pub fn freshness(mut self, freshness: FreshnessPolicy) -> Self {
        self.freshness = Some(freshness);
        self
    }

    /// Replaces layers.
    #[must_use]
    pub fn layers(mut self, layers: StorageLayers) -> Self {
        self.layers = Some(layers);
        self
    }

    /// Replaces the coherence mode.
    #[must_use]
    pub fn coherence(mut self, coherence: CoherenceMode) -> Self {
        self.coherence = Some(coherence);
        self
    }

    /// Replaces the shared-cache policy.
    #[must_use]
    pub fn shared(mut self, shared: SharedCachePolicy) -> Self {
        self.shared = Some(shared);
        self
    }

    /// Replaces the failure policy.
    #[must_use]
    pub fn failure(mut self, failure: FailurePolicy) -> Self {
        self.failure = Some(failure);
        self
    }

    /// Replaces query semantics.
    #[must_use]
    pub fn query(mut self, query: QueryPolicy) -> Self {
        self.query = Some(query);
        self
    }

    /// Replaces the variance set.
    #[must_use]
    pub fn vary(mut self, vary: BTreeSet<VarianceDimension>) -> Self {
        self.vary = Some(vary);
        self
    }
}

/// Safety signals observed on one concrete response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResponseSignals {
    /// Request method, upper case.
    pub method: String,
    /// Response status.
    pub status: u16,
    /// Whether the body streams.
    pub streaming: bool,
    /// Whether the response sets a cookie.
    pub sets_cookie: bool,
    /// The content type, if any.
    pub content_type: Option<String>,
    /// Lower-case response header names.
    pub header_names: Vec<String>,
    /// Whether rendering observed principal, session, authorization, or
    /// secret context.
    pub private_observed: bool,
}

/// The storage decision for one concrete response.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Eligibility {
    /// Store under this class.
    Store(RepresentationClass),
    /// Serve normally, store nothing, poison nothing.
    Decline(DeclineReason),
}

/// Why a response was not stored.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeclineReason {
    /// The route policy is uncacheable.
    PolicyUncacheable,
    /// Not GET or HEAD.
    Method,
    /// Not a 200 canonical representation.
    Status,
    /// The body streams.
    Streaming,
    /// The response sets a cookie.
    SetsCookie,
    /// A hop-by-hop, connection, or per-request header is present.
    UnsafeHeader,
}
