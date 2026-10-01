//! The supported way for a test to prove that the RenderCache served a
//! route instead of rendering it.
//!
//! An identical body or an `Age` header both have an innocent explanation,
//! so neither proves a hit. The store does: the key a route derives, the
//! entry each tier holds under that key, and what the next request does
//! once the first tier is empty.
//! [`RenderCacheProbe`](crate::render_cache::testing::RenderCacheProbe)
//! reads all three for one route, and
//! [`StatementCounter`](crate::database::testing::StatementCounter) counts
//! what a request cost the database, so a test can hold a hit to none.
//!
//! Compiled only when the `testing` feature is on (a default feature), so a
//! production build that turns the feature off carries none of it. No read
//! panics: each returns a [`FrameworkError`] that names the route pattern
//! and what went wrong, and never a login, a parameter value, or a key.
//!
//! # Example
//!
//! ```rust,no_run
//! use suprnova::render_cache::testing::RenderCacheProbe;
//!
//! # async fn example() -> Result<(), suprnova::FrameworkError> {
//! // After `RenderCache::install` and one request to `/posts/1`, on a route
//! // whose policy stores in L0 and L1:
//! let probe = RenderCacheProbe::route("/posts/{id}").params(&[("id", "1")]);
//! assert!(probe.l0().await?.is_some(), "the request published the page");
//!
//! RenderCacheProbe::clear_l0()?;
//! assert!(probe.l0().await?.is_none(), "memory is empty");
//! assert!(probe.l1().await?.is_some(), "the second tier still holds the page");
//! # Ok(())
//! # }
//! ```
//!
//! `policy_table` and the `_for_test` functions in this module are seams
//! for the framework's own test suite. They are hidden from the
//! documentation, panic on a broken setup, and are not part of the
//! supported surface.

use std::collections::BTreeMap;
use std::sync::Arc;

use suprnova_live::render_cache::RenderCacheError;

use super::middleware::{FixedFacts, RenderCacheRuntime, SEEDED_EPOCH, build_key_input};
use super::registry::RenderCachePolicyTable;
use super::{EntryInspection, RenderCache, VarianceDimension};
use crate::FrameworkError;

/// What a probe reports when this process has no RenderCache runtime.
const NO_RUNTIME: &str =
    "no RenderCache runtime is installed; call RenderCache::install before probing";

/// What a probe reports for a route that no policy covers.
const NO_POLICY: &str = "no RenderCache policy covers the route or a group that encloses it";

/// What a probe reports for a route that varies on `Host` when it was given
/// no host: a request always names one, and the key holds it.
const NO_HOST: &str = "the route varies on Host, so the probe needs the request's host; call \
     RenderCacheProbe::host";

/// What [`RenderCacheProbe::l1`] reports when the runtime has no second tier.
const NO_L1: &str = "no L1 tier is configured on the installed runtime";

/// One route's view of the installed RenderCache: the key the route derives
/// and the entry each tier holds under that key.
///
/// Built for one route pattern, then narrowed to the request a test
/// dispatched. The key is built by the function the RenderCache middleware
/// builds its own with, so an entry the probe finds is the entry the request
/// is served from. What the probe has to be told depends on the dimensions
/// the route's policy varies on:
///
/// | The policy varies on | Tell the probe | Left unsaid |
/// |---|---|---|
/// | `Principal` | [`Self::login`] | an anonymous request |
/// | `Tenant` | [`Self::tenant`] | a request with no tenant |
/// | `Locale` | [`Self::locale`] | the process's current locale |
/// | `Host` | [`Self::host`] | an error: the request always names a host |
/// | `Media`, `Encoding` | nothing | the policy's declared default, as a request that negotiates nothing gets |
///
/// The route parameters come from [`Self::params`] and the epoch from
/// [`Self::at_epoch`]. The request is taken to carry no query string. A
/// dimension the policy does not declare has no effect on the key, whatever
/// the probe is told. A route that declares a dimension this host has no
/// producer for gets a probe error, as it gets no key from the middleware.
///
/// # Example
///
/// ```rust,no_run
/// use suprnova::render_cache::testing::RenderCacheProbe;
///
/// # async fn example() -> Result<(), suprnova::FrameworkError> {
/// let probe = RenderCacheProbe::route("/teams/{team}/dashboard")
///     .params(&[("team", "7")])
///     .login("42");
/// let entry = probe.l0().await?.expect("the first request published the page");
/// assert_eq!(entry.status, 200);
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct RenderCacheProbe {
    pattern: String,
    params: Vec<(String, String)>,
    login: Option<String>,
    tenant: Option<String>,
    locale: Option<String>,
    host: Option<String>,
    epoch: Option<u64>,
}

impl RenderCacheProbe {
    /// A probe for the route registered under `pattern`, written as the
    /// router holds it (`/posts/{id}`, not `/posts/1`), with no parameters,
    /// no login, no tenant, no host, the process's current locale, and the
    /// seeded epoch.
    ///
    /// # Example
    ///
    /// ```rust
    /// use suprnova::render_cache::testing::RenderCacheProbe;
    ///
    /// let probe = RenderCacheProbe::route("/posts/{id}");
    /// ```
    #[must_use]
    pub fn route(pattern: &str) -> Self {
        Self {
            pattern: pattern.to_owned(),
            params: Vec::new(),
            login: None,
            tenant: None,
            locale: None,
            host: None,
            epoch: None,
        }
    }

    /// The route parameters of the request, as name and value pairs. Replaces
    /// any set before. The key does not depend on their order.
    ///
    /// # Example
    ///
    /// ```rust
    /// use suprnova::render_cache::testing::RenderCacheProbe;
    ///
    /// let probe = RenderCacheProbe::route("/teams/{team}/posts/{id}")
    ///     .params(&[("team", "7"), ("id", "1")]);
    /// ```
    #[must_use]
    pub fn params(mut self, params: &[(&str, &str)]) -> Self {
        self.params = params
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        self
    }

    /// The signed-in user the request was made as, by the identifier
    /// [`Authenticatable::get_auth_identifier`](crate::auth::Authenticatable::get_auth_identifier)
    /// returns for that user.
    ///
    /// It changes the key only on a route that varies on
    /// [`VarianceDimension::Principal`].
    /// Without it the probe derives the key an anonymous request derives.
    ///
    /// # Example
    ///
    /// ```rust
    /// use suprnova::render_cache::testing::RenderCacheProbe;
    ///
    /// let probe = RenderCacheProbe::route("/account").login("42");
    /// ```
    #[must_use]
    pub fn login(mut self, id: &str) -> Self {
        self.login = Some(id.to_owned());
        self
    }

    /// The tenant the request resolved to, by the identifier the tenant
    /// middleware set on it.
    ///
    /// It changes the key only on a route that varies on
    /// [`VarianceDimension::Tenant`]. Without it the probe derives the key
    /// of a request that has no tenant.
    ///
    /// # Example
    ///
    /// ```rust
    /// use suprnova::render_cache::testing::RenderCacheProbe;
    ///
    /// let probe = RenderCacheProbe::route("/dashboard").tenant("acme");
    /// ```
    #[must_use]
    pub fn tenant(mut self, id: &str) -> Self {
        self.tenant = Some(id.to_owned());
        self
    }

    /// The locale the request was rendered in, written as
    /// `Lang::locale().as_str()` writes it (`fr`, `pt-BR`).
    ///
    /// It changes the key only on a route that varies on
    /// [`VarianceDimension::Locale`]. Without it the probe uses the
    /// process's current locale, the one a request that negotiates no locale
    /// of its own is rendered in.
    ///
    /// # Example
    ///
    /// ```rust
    /// use suprnova::render_cache::testing::RenderCacheProbe;
    ///
    /// let probe = RenderCacheProbe::route("/posts/{id}").locale("fr");
    /// ```
    #[must_use]
    pub fn locale(mut self, locale: &str) -> Self {
        self.locale = Some(locale.to_owned());
        self
    }

    /// The host the request was made to, written as `Request::http_host`
    /// reports it: the host, and the port when it is not the scheme's
    /// default (`example.test`, `example.test:8080`).
    ///
    /// It changes the key only on a route that varies on
    /// [`VarianceDimension::Host`], and such a route needs it: without a
    /// host the probe returns an error that names the dimension.
    ///
    /// # Example
    ///
    /// ```rust
    /// use suprnova::render_cache::testing::RenderCacheProbe;
    ///
    /// let probe = RenderCacheProbe::route("/").host("shop.example.test");
    /// ```
    #[must_use]
    pub fn host(mut self, host: &str) -> Self {
        self.host = Some(host.to_owned());
        self
    }

    /// The authority epoch to derive the key under.
    ///
    /// Every key includes the epoch it was derived under. Without this the
    /// probe uses epoch 1, the value the RenderCache migration seeds. A test
    /// that has advanced the epoch, with [`RenderCache::advance_epoch`] or on
    /// another node through the shared ledger, names the new epoch here, or
    /// it looks up a key that nothing was published under.
    ///
    /// # Example
    ///
    /// ```rust
    /// use suprnova::render_cache::testing::RenderCacheProbe;
    ///
    /// let probe = RenderCacheProbe::route("/posts/{id}")
    ///     .params(&[("id", "1")])
    ///     .at_epoch(2);
    /// ```
    #[must_use]
    pub fn at_epoch(mut self, epoch: u64) -> Self {
        self.epoch = Some(epoch);
        self
    }

    /// The key text the RenderCache middleware derives for this request: the
    /// text [`RenderCache::inspect`] and the `render-cache:inspect` console
    /// command take.
    ///
    /// The probe has to be told the login, tenant, locale and host of the
    /// dimensions the route varies on: see the table on
    /// [`RenderCacheProbe`].
    /// A key derived without what the route varies on is a key nothing was
    /// stored under, so a probe of it reads `None` whether or not the page
    /// was cached.
    ///
    /// # Errors
    ///
    /// Returns a [`FrameworkError`] when no RenderCache runtime is
    /// installed, when no RenderCache policy covers the route, when the
    /// route varies on `Host` and the probe has none, or when the key cannot
    /// be derived from these parameters, for example because a parameter
    /// name is empty or a value is longer than a key allows.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use suprnova::render_cache::RenderCache;
    /// use suprnova::render_cache::testing::RenderCacheProbe;
    ///
    /// # async fn example() -> Result<(), suprnova::FrameworkError> {
    /// let key = RenderCacheProbe::route("/posts/{id}")
    ///     .params(&[("id", "1")])
    ///     .key()?;
    /// let entry = RenderCache::inspect(&key).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn key(&self) -> Result<String, FrameworkError> {
        let runtime = self.runtime()?;
        Ok(self.derive(&runtime)?.to_base64url())
    }

    /// The entry the first tier, this process's in-memory L0, holds under
    /// [`Self::key`], or `None` when it holds none.
    ///
    /// The inspection carries no body: the entry's class, kind, status, body
    /// size, epoch, publication time, dependency count and stitch slots.
    ///
    /// The read is not a use: it leaves L0's eviction order as it was, so a
    /// test of eviction can probe between requests without changing which
    /// entry the next publication evicts.
    ///
    /// # Errors
    ///
    /// Returns a [`FrameworkError`] for every reason [`Self::key`] does, and
    /// when the entry held there does not decode.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use suprnova::render_cache::EntryKind;
    /// use suprnova::render_cache::testing::RenderCacheProbe;
    ///
    /// # async fn example() -> Result<(), suprnova::FrameworkError> {
    /// let entry = RenderCacheProbe::route("/posts/{id}")
    ///     .params(&[("id", "1")])
    ///     .l0()
    ///     .await?
    ///     .expect("the first request published the page");
    /// assert_eq!(entry.kind, EntryKind::Complete);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn l0(&self) -> Result<Option<EntryInspection>, FrameworkError> {
        let runtime = self.runtime()?;
        let key = self.derive(&runtime)?;
        runtime
            .l0
            .peek(&key)
            .map(|stored| self.inspect_stored(&runtime, &stored.bytes, "L0"))
            .transpose()
    }

    /// The entry the second tier, the configured L1 store, holds under
    /// [`Self::key`], or `None` when it holds none.
    ///
    /// Only a route whose policy stores in L1 publishes there. Read it after
    /// [`Self::clear_l0`] to prove that the next request can only be served
    /// from L1.
    ///
    /// # Errors
    ///
    /// Returns a [`FrameworkError`] for every reason [`Self::key`] does, when
    /// the installed runtime has no L1 tier configured, when the L1 read
    /// fails, and when the entry stored there does not decode.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use suprnova::render_cache::testing::RenderCacheProbe;
    ///
    /// # async fn example() -> Result<(), suprnova::FrameworkError> {
    /// let stored = RenderCacheProbe::route("/posts/{id}")
    ///     .params(&[("id", "1")])
    ///     .l1()
    ///     .await?;
    /// assert!(stored.is_some(), "the first request published to L1");
    /// # Ok(())
    /// # }
    /// ```
    pub async fn l1(&self) -> Result<Option<EntryInspection>, FrameworkError> {
        use suprnova_live::render_cache::store::RenderStore as _;

        let runtime = self.runtime()?;
        let Some(l1) = runtime.l1.as_ref() else {
            return Err(self.error(NO_L1));
        };
        let key = self.derive(&runtime)?;
        let stored = l1
            .get(&key)
            .await
            .map_err(|error| self.failure("the L1 read failed", error))?;
        stored
            .map(|stored| self.inspect_stored(&runtime, &stored.bytes, "L1"))
            .transpose()
    }

    /// Empties the first tier, L0, and leaves the L1 tier, the authority
    /// epoch, and the rebuild coordinator as they are.
    ///
    /// This is how a test proves that a request was served from L1 and not
    /// from memory. [`RenderCache::advance_epoch`] empties L0 too, but it
    /// also changes the epoch every key is derived under, so the next request
    /// misses in both tiers.
    ///
    /// # Errors
    ///
    /// Returns a [`FrameworkError`] when no RenderCache runtime is installed.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use suprnova::render_cache::testing::RenderCacheProbe;
    ///
    /// # async fn example() -> Result<(), suprnova::FrameworkError> {
    /// RenderCacheProbe::clear_l0()?;
    /// let probe = RenderCacheProbe::route("/posts/{id}").params(&[("id", "1")]);
    /// assert!(probe.l0().await?.is_none());
    /// # Ok(())
    /// # }
    /// ```
    pub fn clear_l0() -> Result<(), FrameworkError> {
        let message = format!("RenderCacheProbe::clear_l0: {NO_RUNTIME}");
        let runtime = RenderCache::runtime().ok_or_else(|| FrameworkError::internal(message))?;
        runtime.l0.clear();
        Ok(())
    }

    /// The installed runtime, or the error that says none is installed.
    fn runtime(&self) -> Result<Arc<RenderCacheRuntime>, FrameworkError> {
        RenderCache::runtime().ok_or_else(|| self.error(NO_RUNTIME))
    }

    /// The key the middleware derives for this request under `runtime`.
    fn derive(
        &self,
        runtime: &RenderCacheRuntime,
    ) -> Result<suprnova_live::render_cache::key::RenderKey, FrameworkError> {
        use suprnova_live::render_cache::key::RenderKey;

        let Some(policy) = runtime.table.effective_policy(&self.pattern) else {
            return Err(self.error(NO_POLICY));
        };
        if self.host.is_none() && policy.vary().contains(&VarianceDimension::Host) {
            return Err(self.error(NO_HOST));
        }
        let params: BTreeMap<String, String> = self.params.iter().cloned().collect();
        let facts = FixedFacts {
            locale: self.locale.clone(),
            principal: self.login.as_deref(),
            tenant: self.tenant.as_deref(),
            host: self.host.as_deref(),
        };
        let input = build_key_input(
            runtime,
            &facts,
            &self.pattern,
            &policy,
            self.epoch.unwrap_or(SEEDED_EPOCH),
            params,
            Vec::new(),
        )
        .map_err(|error| self.failure("the key input cannot be built for this route", error))?;
        RenderKey::derive(&input, &runtime.keys).map_err(|error| {
            self.failure(
                "the key cannot be derived from these parameters and login",
                error,
            )
        })
    }

    /// The body-free inspection of `bytes`, read out of `tier`.
    fn inspect_stored(
        &self,
        runtime: &RenderCacheRuntime,
        bytes: &bytes::Bytes,
        tier: &str,
    ) -> Result<EntryInspection, FrameworkError> {
        suprnova_live::render_cache::inspect(bytes, &runtime.limits).map_err(|error| {
            self.failure(
                &format!("the entry stored in {tier} does not decode"),
                error,
            )
        })
    }

    /// An error that names the route pattern and `what` went wrong. The
    /// pattern is the application's own route declaration, so it is the one
    /// part of the request that is safe to repeat: the login, the parameter
    /// values, and the key never appear.
    fn error(&self, what: &str) -> FrameworkError {
        let pattern = &self.pattern;
        FrameworkError::internal(format!("RenderCacheProbe for route `{pattern}`: {what}"))
    }

    /// [`Self::error`] with the engine's closed error token appended, which
    /// names the violated contract and carries no request material.
    fn failure(&self, what: &str, error: RenderCacheError) -> FrameworkError {
        self.error(&format!("{what} ({error})"))
    }
}

/// The router's registered RenderCache policy table.
#[doc(hidden)]
#[must_use]
pub fn policy_table(router: &crate::Router) -> RenderCachePolicyTable {
    router.render_cache_policies().clone()
}

/// Rewrites the Composite entry stored for `pattern` in place, so a test can
/// put the store into a state only a redeploy could otherwise produce.
///
/// The stored entry is decoded under the runtime's own key ring, its
/// [`SegmentGraph`](suprnova_live::render_cache::composite::SegmentGraph) is
/// handed to `edit`, every slot's surrounding digest is
/// recomputed from the edited graph and the unchanged shell, and the result
/// is re-encoded and published back under the same key with a fence one
/// token above the one it was found under. The next request for `pattern`
/// therefore sees the rewritten entry exactly as if the running build had
/// published it.
///
/// This is how a test reaches the drift cases that cannot be staged from
/// outside: a stored slot naming a component or contract digest the current
/// registry no longer has is a redeploy, and a redeploy cannot happen inside
/// one process. Nothing else about the entry moves - the header, the class,
/// the observed generation set, and the publication instant are all the ones
/// the real publisher wrote - so freshness and coherence decide exactly what
/// they decided before.
///
/// An `edit` that changes nothing is a legitimate use: the closure sees the
/// stored graph, which is the only way a test outside this crate can observe
/// how many nonce holes or header templates the publisher actually cut.
///
/// # Panics
///
/// Panics when no runtime is installed, `pattern` has no effective policy,
/// the key cannot be derived, no entry is stored for it, the stored entry is
/// not a Composite one, or the edited graph cannot be re-encoded and
/// published. Every one of those is a broken test setup, not a condition a
/// caller could handle.
#[doc(hidden)]
pub async fn rewrite_composite_for_test<F>(pattern: &str, edit: F)
where
    F: FnOnce(&mut suprnova_live::render_cache::composite::SegmentGraph),
{
    use suprnova_live::render_cache::composite::{CompositeEntry, surrounding_digest};
    use suprnova_live::render_cache::entry::{DecodedEntry, decode, encode_composite};
    use suprnova_live::render_cache::key::RenderKey;
    use suprnova_live::render_cache::store::{PublishOutcome, RenderStore as _};

    let runtime = super::RenderCache::runtime().expect("RenderCache installed");
    let policy = runtime
        .table
        .effective_policy(pattern)
        .expect("an effective policy for the rewritten route");
    let input = super::middleware::key_input_for_test(&runtime, pattern, &[], None, &policy)
        .expect("the key input of the rewritten route");
    let key = RenderKey::derive(&input, &runtime.keys).expect("derive the stored entry's key");
    let stored = runtime
        .l0
        .get(&key)
        .await
        .expect("read the stored entry")
        .expect("an entry is stored for the rewritten route");
    let decoded = decode(&stored.bytes, &runtime.keys, &runtime.limits).expect("decode the entry");
    let DecodedEntry::Composite(entry) = decoded else {
        panic!("the stored entry is not a Composite one");
    };
    let mut graph = entry.graph().clone();
    edit(&mut graph);
    let shell = entry.shell().clone();
    // Recomputed rather than carried over: the assembler recomputes these
    // from the graph and the shell on every hit, so an edit that moved a
    // slot would otherwise leave the entry rejecting itself for a reason
    // the test never asked for.
    for index in 0..graph.slots.len() {
        graph.slots[index].surrounding =
            surrounding_digest(&graph, &shell, index).expect("recompute a surrounding digest");
    }
    let rewritten = CompositeEntry::new(entry.header().clone(), graph, shell)
        .expect("the edited graph is a valid Composite entry");
    let bytes = encode_composite(&rewritten, &runtime.keys).expect("encode the edited entry");
    let mut fence = stored.fence;
    fence.token = fence
        .token
        .checked_add(1)
        .expect("a fresh publication token");
    let outcome = runtime
        .l0
        .publish(&key, bytes, fence, stored.published_at_ms, u64::MAX)
        .await
        .expect("publish the edited entry");
    assert_eq!(
        outcome,
        PublishOutcome::Published,
        "the rewritten entry must replace the one it was read from"
    );
}

/// Applies `edit` to a *copy* of the graph and header already stored for
/// `pattern`, without publishing anything, and runs the same publish-time
/// nested-composition check `build_composite_entry` runs before a real
/// composite is ever stored (`stitch::refuse_unsafe_nesting`). `None` means
/// the check would have allowed it; `Some(reason)` is the closed decline
/// reason label (`super::decline::LookupDeclineReason::as_str`) the real
/// publish path would have declined with.
///
/// `crate::live::LiveNestedSegment` is the typed way an author declares a
/// nested segment, but nothing yet drives a real render through the
/// body-cutting capture that would turn one into a stored `Segment::Nested`
/// (a later task), so this remains how a test drives the publish-time check
/// itself with a hand-built graph instead: the same shape
/// [`rewrite_composite_for_test`] uses to reach a hit-time case a redeploy
/// alone could otherwise produce, applied here to a publish-time one
/// nothing can produce through a real render yet.
///
/// # Panics
///
/// Panics on the same broken-test-setup conditions
/// [`rewrite_composite_for_test`] does.
#[doc(hidden)]
pub async fn nested_publish_check_for_test<F>(pattern: &str, edit: F) -> Option<&'static str>
where
    F: FnOnce(&mut suprnova_live::render_cache::composite::SegmentGraph),
{
    use suprnova_live::render_cache::entry::{DecodedEntry, decode};
    use suprnova_live::render_cache::key::RenderKey;
    use suprnova_live::render_cache::store::RenderStore as _;

    let runtime = super::RenderCache::runtime().expect("RenderCache installed");
    let policy = runtime
        .table
        .effective_policy(pattern)
        .expect("an effective policy for the checked route");
    let input = super::middleware::key_input_for_test(&runtime, pattern, &[], None, &policy)
        .expect("the key input of the checked route");
    let key = RenderKey::derive(&input, &runtime.keys).expect("derive the stored entry's key");
    let stored = runtime
        .l0
        .get(&key)
        .await
        .expect("read the stored entry")
        .expect("an entry is stored for the checked route");
    let decoded = decode(&stored.bytes, &runtime.keys, &runtime.limits).expect("decode the entry");
    let DecodedEntry::Composite(entry) = decoded else {
        panic!("the stored entry is not a Composite one");
    };
    let header = entry.header().clone();
    let mut graph = entry.graph().clone();
    edit(&mut graph);
    match super::stitch::refuse_unsafe_nesting(&runtime, &header, &graph).await {
        Ok(()) => None,
        Err(error) => Some(super::stitch::composite_build_error_reason(error).as_str()),
    }
}

/// Publishes a bare Complete entry directly to L0, under a key derived from
/// `fixture_pattern` exactly as [`suprnova_live::render_cache::key::RenderKey::for_test`]
/// derives one - no route registration, no dispatch, and no policy needed -
/// with the given class and freshness window, and returns the key it was
/// published under.
///
/// This is the only way a test can name an inner entry whose class or
/// freshness genuinely differs from an entry a real dispatch through this
/// harness would ever produce (every route this harness registers is
/// `PublicShellStitched`): [`nested_publish_check_for_test`] and
/// `stitch::refuse_unsafe_nesting` need a *stored*, decodable entry to
/// compare against, and forging one that
/// [`suprnova_live::render_cache::entry::decode`] will still accept needs
/// this runtime's own key ring, which an external test cannot reach any
/// other way.
///
/// # Panics
///
/// Panics on the same broken-test-setup conditions
/// [`rewrite_composite_for_test`] does, and if `class` and `fresh_ms`
/// cannot be encoded into a valid entry.
#[doc(hidden)]
pub async fn publish_bare_entry_for_test(
    fixture_pattern: &str,
    class: suprnova_live::render_cache::RepresentationClass,
    fresh_ms: u64,
) -> suprnova_live::render_cache::key::RenderKey {
    use suprnova_live::render_cache::entry::{CompleteEntry, EntryHeader, SafeHeaders, encode};
    use suprnova_live::render_cache::generation::GenerationSet;
    use suprnova_live::render_cache::key::RenderKey;
    use suprnova_live::render_cache::store::{PublicationFence, PublishOutcome, RenderStore as _};
    use suprnova_live::render_cache::variance::VarianceDescriptor;

    let runtime = super::RenderCache::runtime().expect("RenderCache installed");
    let key = RenderKey::for_test(&runtime.keys, fixture_pattern);
    let header = EntryHeader {
        key: key.clone(),
        class,
        variance: VarianceDescriptor::new(),
        published_at_ms: 0,
        fresh_ms,
        stale_servable_ms: 0,
        stale_on_error_ms: 0,
        observed: GenerationSet::default(),
        epoch: 1,
        seed_deadline_ms: None,
        status: 200,
        headers: SafeHeaders::from_pairs(Vec::<(String, String)>::new())
            .expect("empty headers are always safe"),
        content_encoding: None,
    };
    let entry = CompleteEntry::new(header, bytes::Bytes::from_static(b"nested fixture"));
    let bytes = encode(&entry, &runtime.keys).expect("encode the fixture entry");
    let fence = PublicationFence {
        epoch: 1,
        generation_digest: [0; 32],
        token: 0,
    };
    let outcome = runtime
        .l0
        .publish(&key, bytes, fence, 0, u64::MAX)
        .await
        .expect("publish the fixture entry");
    assert_eq!(
        outcome,
        PublishOutcome::Published,
        "a fresh fixture key must always publish"
    );
    key
}

#[cfg(test)]
mod tests {
    //! This crate's own unit tests never install a RenderCache runtime (the
    //! install tests in the parent module assert that none is left behind),
    //! so every read here stops at its first check. The integration tests
    //! under `framework/tests/render_cache/` cover every later one.
    use super::RenderCacheProbe;

    #[tokio::test]
    async fn every_read_reports_a_missing_runtime_by_route_and_nothing_else() {
        let probe = RenderCacheProbe::route("/probe-without-runtime/{id}")
            .params(&[("id", "param-value-7")])
            .login("login-7");
        let failures = [
            probe.key().err(),
            probe.l0().await.err(),
            probe.l1().await.err(),
        ];
        for failure in failures {
            let message = failure
                .expect("a read with no runtime installed fails")
                .to_string();
            assert!(
                message.contains("no RenderCache runtime is installed"),
                "the error says what went wrong: {message}"
            );
            assert!(
                message.contains("/probe-without-runtime/{id}"),
                "the error names the route pattern: {message}"
            );
            assert!(
                !message.contains("param-value-7") && !message.contains("login-7"),
                "the error carries no parameter value and no login: {message}"
            );
        }

        let message = RenderCacheProbe::clear_l0()
            .expect_err("clearing L0 with no runtime installed fails")
            .to_string();
        assert!(
            message.contains("no RenderCache runtime is installed"),
            "{message}"
        );
    }
}
