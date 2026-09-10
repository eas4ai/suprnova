//! Route and group RenderCache policy, deterministic patches, and the
//! concrete-response eligibility decision.

use std::collections::BTreeSet;

use super::variance::VarianceDimension;
use super::{RenderCacheError, RenderCacheErrorKind};

/// Upper bound on any freshness interval: 31 days in milliseconds.
pub const MAX_INTERVAL_MS: u64 = 31 * 24 * 60 * 60 * 1000;
/// Upper bound on declared query parameter names per route.
pub const MAX_DECLARED_QUERY: usize = 32;

/// How a representation may be shared. Order is widest to narrowest sharing.
///
/// Serialized as a `snake_case` tag (`"public_shared"`, and so on): the
/// stored entry header is JSON, and every public JSON field and tag in this
/// crate is `snake_case` by convention.
#[derive(
    Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
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

    /// Milliseconds after publication beyond which a representation under
    /// this policy is `Dead` in every band - the single source of truth
    /// [`super::coherence::evaluate_freshness`] and any retention-based
    /// cleanup (a file-backed L1's sweep, for one) must agree on.
    ///
    /// Class-aware (fix round 2, R99): `PrivateCached` never gets a stale
    /// grace period at all - [`super::coherence::evaluate_freshness`] puts
    /// it at `Dead` the instant it stops being fresh, per spec 16's private
    /// entries having bounded retention and eviction independent of public
    /// entries - so for that class this is `fresh_ms` alone. For every
    /// other class this is **not** `fresh_ms + stale_on_error_ms`: the
    /// stale-servable and stale-on-error windows are both measured from the
    /// same point - the end of the fresh interval, not cumulatively - and
    /// this constructor does not require `stale_on_error_ms >=
    /// stale_servable_ms`. A policy may legally declare a wider
    /// stale-servable window than its stale-on-error window (serve stale
    /// content longer than it tolerates a failed rebuild), in which case
    /// the stale-servable window alone determines when the representation
    /// is truly dead. The edge for every non-`PrivateCached` class is
    /// therefore `fresh_ms + max(stale_servable_ms, stale_on_error_ms)`.
    #[must_use]
    pub const fn dead_after_ms(&self, class: RepresentationClass) -> u64 {
        if matches!(class, RepresentationClass::PrivateCached) {
            return self.fresh_ms;
        }
        let widest_stale = if self.stale_servable_ms > self.stale_on_error_ms {
            self.stale_servable_ms
        } else {
            self.stale_on_error_ms
        };
        self.fresh_ms.saturating_add(widest_stale)
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

/// Upper bound on entries in one [`NegotiatedPolicy`]'s declared closed set.
pub const MAX_NEGOTIATED_VALUES: usize = 16;
/// Upper bound on one declared closed-set value's length in bytes.
pub const MAX_NEGOTIATED_VALUE_BYTES: usize = 128;
/// Upper bound on header entries [`NegotiatedPolicy::negotiate`] considers;
/// further comma-separated entries are ignored, bounding a hostile header's
/// parsing cost.
pub const MAX_NEGOTIATION_ENTRIES: usize = 64;

/// The closed set a route negotiates `Media` or `Encoding` variance
/// against, and the default value served when a request negotiates nothing
/// that matches it. See [`RenderCachePolicyBuilder::vary_media`] and
/// [`RenderCachePolicyBuilder::vary_encoding`].
///
/// # Negotiation rule
///
/// [`Self::negotiate`] is `q`-weighted (RFC 9110 quality values): the
/// accepted-set member with the highest quality wins, and when two
/// candidates tie on quality, the one listed first in the header wins. A
/// header token is matched case-insensitively against the declared
/// (lower-case) set; a wildcard (`*/*`, `type/*`, or a bare `*`) is compared
/// as a literal token like any other rather than expanded against the set,
/// so it practically never matches a real declared value. A `q=0`,
/// out-of-range (outside `0.0..=1.0`), or unparsable quality excludes that
/// entry rather than defaulting it to `1.0`. An absent header, a header
/// naming nothing in the declared set, or a header this parser cannot make
/// sense of resolves to [`Self::default_value`] - never panics, and never
/// treats a header value as anything other than data to compare.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NegotiatedPolicy {
    accepted: BTreeSet<String>,
    default: String,
}

impl NegotiatedPolicy {
    /// Declares the closed accepted set and its default, which must itself
    /// be a member of the set. Bounded: at most [`MAX_NEGOTIATED_VALUES`]
    /// entries, each non-empty, at most [`MAX_NEGOTIATED_VALUE_BYTES`]
    /// bytes, ASCII with no control bytes, and lower case - the canonical
    /// form a request's header is matched against (see [`Self::negotiate`]).
    pub fn declared<I, S>(accepted: I, default: S) -> Result<Self, RenderCacheError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let invalid = || RenderCacheError::new(RenderCacheErrorKind::PolicyInvalid);
        let accepted: BTreeSet<String> = accepted.into_iter().map(Into::into).collect();
        let default = default.into();
        if accepted.is_empty() || accepted.len() > MAX_NEGOTIATED_VALUES {
            return Err(invalid());
        }
        if accepted.iter().any(|value| {
            value.is_empty()
                || value.len() > MAX_NEGOTIATED_VALUE_BYTES
                || value
                    .bytes()
                    .any(|b| !b.is_ascii() || b.is_ascii_control() || b.is_ascii_uppercase())
        }) {
            return Err(invalid());
        }
        if !accepted.contains(&default) {
            return Err(invalid());
        }
        Ok(Self { accepted, default })
    }

    /// The declared closed set.
    #[must_use]
    pub fn accepted(&self) -> &BTreeSet<String> {
        &self.accepted
    }

    /// The declared default, served when negotiation matches nothing in the
    /// closed set.
    #[must_use]
    pub fn default_value(&self) -> &str {
        &self.default
    }

    /// Negotiates a raw `Accept`- or `Accept-Encoding`-shaped header value
    /// against the declared closed set. See the type's own doc for the
    /// exact rule. Never panics, and never echoes the header value back
    /// into its result unless it is exactly one of the declared members.
    #[must_use]
    pub fn negotiate(&self, header: Option<&str>) -> String {
        let Some(header) = header else {
            return self.default.clone();
        };
        let mut best: Option<(f32, &str)> = None;
        for entry in header.split(',').take(MAX_NEGOTIATION_ENTRIES) {
            let mut parts = entry.split(';');
            let Some(token) = parts.next() else {
                continue;
            };
            let token = token.trim();
            if token.is_empty() {
                continue;
            }
            let quality = parts
                .filter_map(|param| {
                    let param = param.trim();
                    param
                        .strip_prefix("q=")
                        .or_else(|| param.strip_prefix("Q="))
                })
                .next()
                .map_or(1.0, |value| value.trim().parse::<f32>().unwrap_or(0.0));
            if !quality.is_finite() || quality <= 0.0 || quality > 1.0 {
                continue;
            }
            let Some(matched) = self
                .accepted
                .iter()
                .find(|candidate| candidate.eq_ignore_ascii_case(token))
            else {
                continue;
            };
            let better = match best {
                None => true,
                Some((best_quality, _)) => quality > best_quality,
            };
            if better {
                best = Some((quality, matched.as_str()));
            }
        }
        best.map_or_else(|| self.default.clone(), |(_, value)| value.to_owned())
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
    media: Option<NegotiatedPolicy>,
    encoding: Option<NegotiatedPolicy>,
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
                media: None,
                encoding: None,
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

    /// The declared `Media` closed set and default, when the route declared
    /// `Media` variance via [`RenderCachePolicyBuilder::vary_media`].
    #[must_use]
    pub fn media(&self) -> Option<&NegotiatedPolicy> {
        self.media.as_ref()
    }

    /// The declared `Encoding` closed set and default, when the route
    /// declared `Encoding` variance via
    /// [`RenderCachePolicyBuilder::vary_encoding`].
    #[must_use]
    pub fn encoding(&self) -> Option<&NegotiatedPolicy> {
        self.encoding.as_ref()
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
            // A patch's `vary` fully replaces the declared set (like every
            // other field here), so a patch that narrows Media or Encoding
            // out of it must not leave the old closed set behind - doing so
            // would fail `validate`'s consistency check below for a patch
            // that only meant to drop the dimension, not replace its set.
            if !next.vary.contains(&VarianceDimension::Media) {
                next.media = None;
            }
            if !next.vary.contains(&VarianceDimension::Encoding) {
                next.encoding = None;
            }
        }
        if let Some(media) = &patch.media {
            next.media = Some(media.clone());
        }
        if let Some(encoding) = &patch.encoding {
            next.encoding = Some(encoding.clone());
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
        // Fix round 5 rejected `FeatureVersion`, `ConfigVersion`, and
        // `Application` here on the grounds that no host had a producer for
        // them. Fix round 6 moved that rejection to the host's own
        // `variance_descriptor` (see its doc): whether a producer exists is
        // a fact about one host's implementation, not about this
        // host-neutral crate's own extension point, and refusing it here
        // made the engine learn about the host to justify the refusal.
        //
        // Fix round 5's `PrivateCached`-with-empty-variance rule is fixed in
        // place rather than moved: checking mere non-emptiness let a
        // `PrivateCached` policy that declares only, say, `Media` still
        // build - `Media` never resolves to `DimensionValue::Private`
        // regardless of host, so every visitor would still share the one
        // "private" entry. Only `Principal` and `Tenant` are documented as
        // opaque private material (see their own variants' doc on
        // [`VarianceDimension`]); that is intrinsic to the dimension's own
        // definition, not a fact any one host could differ on, so the check
        // stays here.
        if self.class == RepresentationClass::PrivateCached
            && !self.vary.iter().any(|dimension| {
                matches!(
                    dimension,
                    VarianceDimension::Principal | VarianceDimension::Tenant
                )
            })
        {
            return Err(RenderCacheError::new(RenderCacheErrorKind::PolicyInvalid));
        }
        // A Composite entry is assembled per request from typed slot
        // outcomes; the bytes a downstream shared cache would see are never
        // what the server actually cached, so a shell-stitched class may
        // never declare `s-maxage` (see `http::cache_control_value`'s doc).
        if self.class == RepresentationClass::PublicShellStitched
            && matches!(self.shared, SharedCachePolicy::SMaxAge { .. })
        {
            return Err(RenderCacheError::new(RenderCacheErrorKind::PolicyInvalid));
        }
        // `Media` and `Encoding` negotiate against a closed declared set
        // (`NegotiatedPolicy`), not a bare presence flag: a policy that
        // varies one of them with no declared set would have nothing to
        // negotiate against, and a policy that carries a declared set for a
        // dimension it does not vary is dead data a caller could mistake
        // for effective. `RenderCachePolicyBuilder::vary_media` and
        // `::vary_encoding` always set both together, so this only ever
        // rejects a caller that bypassed them - a bare `.vary(Media)` with
        // no matching `.vary_media(..)` call, or a `PolicyPatch` whose
        // `vary` and `media`/`encoding` fields disagree.
        if self.vary.contains(&VarianceDimension::Media) != self.media.is_some() {
            return Err(RenderCacheError::new(RenderCacheErrorKind::PolicyInvalid));
        }
        if self.vary.contains(&VarianceDimension::Encoding) != self.encoding.is_some() {
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
    ///
    /// `Media` and `Encoding` also need their closed accepted set and
    /// default declared - use [`Self::vary_media`] or
    /// [`Self::vary_encoding`] for those two instead of this method, or
    /// `build` rejects the policy: there is nothing here for them to
    /// negotiate against.
    #[must_use]
    pub fn vary(mut self, dimension: VarianceDimension) -> Self {
        self.policy.vary.insert(dimension);
        self
    }

    /// Declares `Media` variance together with the closed set it negotiates
    /// against and the default it falls back to. The route's stored
    /// representations partition by the value [`NegotiatedPolicy::negotiate`]
    /// resolves from the request's `Accept` header.
    #[must_use]
    pub fn vary_media(mut self, media: NegotiatedPolicy) -> Self {
        self.policy.vary.insert(VarianceDimension::Media);
        self.policy.media = Some(media);
        self
    }

    /// Declares `Encoding` variance together with the closed set it
    /// negotiates against and the default it falls back to. The route's
    /// stored representations partition by the value
    /// [`NegotiatedPolicy::negotiate`] resolves from the request's
    /// `Accept-Encoding` header.
    #[must_use]
    pub fn vary_encoding(mut self, encoding: NegotiatedPolicy) -> Self {
        self.policy.vary.insert(VarianceDimension::Encoding);
        self.policy.encoding = Some(encoding);
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
    media: Option<NegotiatedPolicy>,
    encoding: Option<NegotiatedPolicy>,
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

    /// Replaces the variance set. When the replacement drops `Media` or
    /// `Encoding`, `RenderCachePolicy::apply` also drops the matching
    /// declared closed set rather than leaving it behind as dead data; pass
    /// [`Self::media`] or [`Self::encoding`] alongside this to declare or
    /// replace a closed set for a dimension the replacement adds.
    #[must_use]
    pub fn vary(mut self, vary: BTreeSet<VarianceDimension>) -> Self {
        self.vary = Some(vary);
        self
    }

    /// Declares or replaces the `Media` closed set and default. Does not by
    /// itself add `Media` to the vary set - pair with [`Self::vary`] when
    /// the base policy does not already declare it.
    #[must_use]
    pub fn media(mut self, media: NegotiatedPolicy) -> Self {
        self.media = Some(media);
        self
    }

    /// Declares or replaces the `Encoding` closed set and default. Does not
    /// by itself add `Encoding` to the vary set - pair with [`Self::vary`]
    /// when the base policy does not already declare it.
    #[must_use]
    pub fn encoding(mut self, encoding: NegotiatedPolicy) -> Self {
        self.encoding = Some(encoding);
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
