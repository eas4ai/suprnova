use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::error::FrameworkError;

/// Minimal request abstraction used by [`crate::inertia::InertiaResponse::resolve`]
/// and [`PartialFilter::build`].
///
/// Production code uses [`crate::http::Request`] (which implements this
/// trait via the blanket impl below). Tests provide a tiny mock without
/// having to construct a real `hyper::Request<hyper::body::Incoming>` —
/// `Incoming` cannot be built outside hyper's connection internals.
pub trait InertiaRequestExt: Send + Sync {
    /// Path component of the request URI (no query string).
    fn path(&self) -> &str;
    /// Path **and** query string of the request URI (`/users?page=2`).
    ///
    /// This is what the Inertia page object's `url` field carries by
    /// default, and what `InertiaVersionMiddleware` always puts in
    /// `X-Inertia-Location`. [`InertiaConfig::url_resolver`] overrides the
    /// former, not the latter. The client writes `page.url` into
    /// `history.state`, so dropping the query here silently resets
    /// pagination, sorting, and filter state on every back/forward
    /// navigation and every `router.reload()`.
    ///
    /// Provided (rather than required) so a hand-rolled test mock that
    /// only knows its path keeps compiling; the default is the path
    /// alone. Real requests override it.
    ///
    /// [`InertiaConfig::url_resolver`]: crate::InertiaConfig::url_resolver
    fn path_and_query(&self) -> String {
        self.path().to_string()
    }
    /// Look up an HTTP header value by name (case-insensitive per HTTP spec).
    fn header(&self, name: &str) -> Option<&str>;
    /// Whether this request originated from the Inertia client (`X-Inertia: true`).
    fn is_inertia(&self) -> bool {
        self.header("X-Inertia")
            .map(|v| v == "true")
            .unwrap_or(false)
    }
    /// Whether this is a prefetch visit. The Inertia client sets
    /// `Purpose: prefetch` on hover/intent prefetches; handlers can
    /// use this to skip expensive side effects (logging, analytics
    /// counters, cache warmups) on a request that may never become a
    /// real navigation.
    fn is_prefetch(&self) -> bool {
        self.header("Purpose")
            .map(|v| v.eq_ignore_ascii_case("prefetch"))
            .unwrap_or(false)
    }
}

impl InertiaRequestExt for crate::http::Request {
    fn path(&self) -> &str {
        crate::http::Request::path(self)
    }
    fn path_and_query(&self) -> String {
        // Same expression `InertiaVersionMiddleware` uses to build its
        // `X-Inertia-Location`, so the 409 bounce and the page object it
        // bounces to always name the same URL.
        self.uri()
            .path_and_query()
            .map(|pq| pq.as_str().to_string())
            .unwrap_or_else(|| crate::http::Request::path(self).to_string())
    }
    fn header(&self, name: &str) -> Option<&str> {
        crate::http::Request::header(self, name)
    }
    fn is_inertia(&self) -> bool {
        crate::http::Request::is_inertia(self)
    }
}

// Blanket impl so callers can pass `&Request`, `&MockRequest`, etc.
// interchangeably without worrying about ref depth.
impl<T: InertiaRequestExt + ?Sized> InertiaRequestExt for &T {
    fn path(&self) -> &str {
        (**self).path()
    }
    fn path_and_query(&self) -> String {
        (**self).path_and_query()
    }
    fn header(&self, name: &str) -> Option<&str> {
        (**self).header(name)
    }
    fn is_inertia(&self) -> bool {
        (**self).is_inertia()
    }
    fn is_prefetch(&self) -> bool {
        (**self).is_prefetch()
    }
}

/// Future returned by a prop resolver.
///
/// Backs every closure-resolved [`Prop`]. Resolvers can do async work
/// (DB queries, HTTP calls) because we're under Tokio. Errors are
/// surfaced through [`FrameworkError`] so they become 500 responses just
/// like any other handler failure — unless the prop is deferred and
/// carries [`Prop::rescue`].
pub type PropFuture = Pin<Box<dyn Future<Output = Result<Value, FrameworkError>> + Send>>;

/// Closure stored inside a resolver-backed [`Prop`].
///
/// `Arc` so the response can be cloned (cheap) before resolving;
/// `Send + Sync + 'static` so it can be moved across `.await` points.
pub type PropResolver = Arc<dyn Fn() -> PropFuture + Send + Sync>;

/// Builder for the options passed to
/// [`InertiaResponse::defer_with`](crate::InertiaResponse::defer_with).
#[derive(Debug, Clone)]
pub struct DeferOptions {
    pub(crate) group: String,
    pub(crate) rescue: bool,
}

impl Default for DeferOptions {
    fn default() -> Self {
        Self {
            group: "default".to_string(),
            rescue: false,
        }
    }
}

impl DeferOptions {
    /// Build a `DeferOptions` with defaults (group `"default"`, rescue disabled).
    pub fn new() -> Self {
        Self::default()
    }

    /// Bucket the deferred prop under a named group so multiple
    /// resolvers fetch together in a single follow-up XHR.
    pub fn group(mut self, group: impl Into<String>) -> Self {
        self.group = group.into();
        self
    }

    /// Catch resolver errors instead of failing the page render. The
    /// failed key is omitted from `props` and reported under
    /// `rescuedProps` so the client renders its `rescue` slot.
    pub fn rescue(mut self) -> Self {
        self.rescue = true;
        self
    }
}

/// Merge mode plus optional match field(s), in one value.
///
/// The shape [`InertiaResponse::merge_with`](crate::InertiaResponse::merge_with)
/// takes. [`Prop::merge_strategy`] unpacks it onto the prop's flags.
///
/// `match_on` is `Option<Vec<String>>` rather than a bare `Vec<String>` so
/// `None` keeps meaning "no dedupe field at all" without also overloading
/// an empty vec with that meaning. Build the field list with
/// [`MatchOnFields::into_match_on_fields`] — `Some(["id",
/// "slug"].into_match_on_fields())` names two fields the same way
/// `Prop::match_on(["id", "slug"])` does; `Some("id".into_match_on_fields())`
/// or `Some(vec!["id".to_string()])` names one. Before this type widened,
/// a variant here could carry at most one field name, which made these
/// builder shortcuts strictly less expressive than building a [`Prop`]
/// directly and calling [`Prop::match_on`].
#[derive(Clone)]
pub enum MergeStrategy {
    /// Append items to the array at the prop's root. Maps to
    /// `Inertia::merge(...)`.
    Append {
        /// Unique-key field name(s) used to dedupe array elements; `None` appends without dedupe.
        match_on: Option<Vec<String>>,
    },
    /// Prepend items to the array at the prop's root. Maps to
    /// `Inertia::merge(...)->prepend()`.
    Prepend {
        /// Unique-key field name(s) used to dedupe array elements; `None` prepends without dedupe.
        match_on: Option<Vec<String>>,
    },
    /// Deep-merge structures. Maps to `Inertia::deepMerge(...)`.
    Deep {
        /// Unique-key field name(s) used to dedupe arrays found inside the deep-merged structure.
        match_on: Option<Vec<String>>,
    },
}

/// Pagination metadata for an infinite-scroll prop.
///
/// Mirrors the Inertia v3 `ScrollProp` shape (see
/// `inertia-3.1.1/packages/core/src/types.ts:213`). Page identifiers are
/// `serde_json::Value` to support both offset pagination (numbers) and
/// cursor pagination (strings), matching Laravel's `paginate()`,
/// `simplePaginate()`, and `cursorPaginate()`.
#[derive(Clone, Debug)]
pub struct ScrollMetadata {
    /// The query-string parameter name (e.g. `"page"`, `"cursor"`).
    /// The client puts the next page identifier under this key when
    /// fetching the next chunk.
    pub page_name: String,
    /// Identifier for the previous page; `None` means "we're at the
    /// first page, no previous fetch possible".
    pub previous_page: Option<Value>,
    /// Identifier for the next page; `None` means "we're at the last
    /// page, no further fetch".
    pub next_page: Option<Value>,
    /// Identifier for the current page.
    pub current_page: Option<Value>,
}

impl ScrollMetadata {
    /// Build new metadata with the given page-name parameter.
    pub fn new(page_name: impl Into<String>) -> Self {
        Self {
            page_name: page_name.into(),
            previous_page: None,
            next_page: None,
            current_page: None,
        }
    }

    /// Set the current page identifier.
    pub fn current<V: Into<Value>>(mut self, page: V) -> Self {
        self.current_page = Some(page.into());
        self
    }

    /// Set the previous-page identifier (None = no previous page).
    pub fn previous<V: Into<Value>>(mut self, page: V) -> Self {
        self.previous_page = Some(page.into());
        self
    }

    /// Set the next-page identifier (None = no next page).
    pub fn next<V: Into<Value>>(mut self, page: V) -> Self {
        self.next_page = Some(page.into());
        self
    }
}

/// Anything that can describe its position in a paginated, infinite-scroll
/// listing: the query-string parameter name, and the previous/next/current
/// page identifiers. Mirrors Laravel's `ProvidesScrollMetadata` interface
/// (`inertia-laravel-2.0.25/src/ProvidesScrollMetadata.php:5-25`).
///
/// The three built-in paginators (`LengthAwarePaginator`, `Paginator`,
/// `CursorPaginator`) implement this in `framework/src/pagination/inertia.rs`
/// instead of building [`ScrollMetadata`] field by field. Implement it for a
/// paginator this crate doesn't know about — a third-party crate's cursor
/// type, a hand-rolled repository result — the same way, then call
/// [`scroll_metadata`](Self::scroll_metadata) to build the value
/// [`InertiaResponse::scroll`](crate::InertiaResponse::scroll) /
/// [`Prop::scroll`] expects.
///
/// That gets you `.scroll` / `Prop::scroll`, not
/// [`InertiaResponse::paginate`](crate::InertiaResponse::paginate) /
/// [`Inertia::paginate`](crate::Inertia::paginate) — those take `impl
/// [`IntoInertiaScroll`](crate::IntoInertiaScroll)<T>`, a separate trait
/// this one does not imply. There is no blanket bridge from one to the
/// other: the three built-in paginators above already implement both
/// traits, so a blanket `impl<P: ProvidesScrollMetadata> IntoInertiaScroll<T>
/// for P` would conflict (`E0119`) with their existing, concrete
/// `IntoInertiaScroll` impls — and even without that conflict, this
/// trait alone has no way to hand back the `Vec<T>` of rows
/// `into_inertia_scroll` needs. Implement [`IntoInertiaScroll`](crate::IntoInertiaScroll)
/// directly, alongside this trait, for a type that should also work with
/// `.paginate`.
pub trait ProvidesScrollMetadata {
    /// The query-string parameter name the client puts the next page
    /// identifier under (`"page"`, `"cursor"`, …).
    fn page_name(&self) -> String;

    /// Identifier for the page before this one; `None` at the start of
    /// the listing.
    fn previous_page(&self) -> Option<Value>;

    /// Identifier for the page after this one; `None` at the end of the
    /// listing.
    fn next_page(&self) -> Option<Value>;

    /// Identifier for this page.
    fn current_page(&self) -> Option<Value>;

    /// Build the [`ScrollMetadata`] this type describes.
    fn scroll_metadata(&self) -> ScrollMetadata {
        ScrollMetadata {
            page_name: self.page_name(),
            previous_page: self.previous_page(),
            next_page: self.next_page(),
            current_page: self.current_page(),
        }
    }
}

/// Builder for the options passed to
/// [`InertiaResponse::once_with`](crate::InertiaResponse::once_with).
#[derive(Debug, Clone, Default)]
pub struct OnceOptions {
    pub(crate) cache_key: Option<String>,
    pub(crate) expires_at: Option<i64>,
    pub(crate) fresh: bool,
}

impl OnceOptions {
    /// Build an `OnceOptions` with defaults (no override, no expiry, not fresh).
    pub fn new() -> Self {
        Self::default()
    }

    /// Override the cache key the client uses to dedupe this prop.
    /// Defaults to the prop's name. Map to `Inertia::once()->as('key')`.
    pub fn as_key(mut self, key: impl Into<String>) -> Self {
        self.cache_key = Some(key.into());
        self
    }

    /// Expire the cached value at the given millis-since-epoch timestamp.
    /// The client invalidates and refetches once now() exceeds this.
    /// Maps to `Inertia::once()->until($timestamp)`.
    pub fn until(mut self, expires_at_ms: i64) -> Self {
        self.expires_at = Some(expires_at_ms);
        self
    }

    /// Force the resolver to run even when the client claims to have a
    /// cached value via `X-Inertia-Except-Once-Props`. Server-side override.
    /// Maps to `Inertia::once()->fresh()`.
    pub fn fresh(mut self) -> Self {
        self.fresh = true;
        self
    }
}

/// Where a prop's value comes from.
///
/// Orthogonal to every flag on [`Prop`]: a value-backed prop and a
/// resolver-backed prop carry the same flags, which is why `.merge()`
/// works the same on `.merge(key, value)` and on
/// `.prop(key, Prop::lazy(..).merge())`.
#[derive(Clone)]
pub(crate) enum PropSource {
    /// Materialized when the prop was built.
    Value(Value),
    /// Produced by an async closure when the prop resolves.
    Resolver(PropResolver),
    /// Absent sentinel. `when_loaded!` produces this when the named
    /// relation is not preloaded on the source entity: the key is left
    /// out of the response entirely — no null, no error.
    Absent,
}

/// How the client folds a merge prop's value into the value it already
/// holds.
///
/// Set by [`Prop::merge`], [`Prop::prepend`], and [`Prop::deep_merge`].
/// Reaches the client as membership in the page object's `mergeProps`,
/// `prependProps`, or `deepMergeProps` array.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeMode {
    /// Append the incoming items after the ones the client holds.
    /// Maps to `Inertia::merge(...)`.
    Append,
    /// Prepend the incoming items before the ones the client holds.
    /// Maps to `Inertia::merge(...)->prepend()`.
    Prepend,
    /// Recursively merge structures instead of concatenating at the
    /// root. Maps to `Inertia::deepMerge(...)`.
    Deep,
}

/// A prop's partial-reload visibility.
///
/// One field rather than three booleans because the three are
/// contradictory: a prop cannot both bypass partial filtering and
/// require an explicit request. [`Prop::always`], [`Prop::optional`],
/// and [`Prop::defer`] each set this, so the last one called wins and
/// the earlier one is erased.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Visibility {
    /// Included on a standard visit; on a matching partial reload,
    /// included only when the only/except lists allow it. What
    /// `.with(...)` and `.lazy(...)` produce.
    #[default]
    Standard,
    /// Included on every response, partial-reload filtering ignored.
    /// Maps to `Inertia::always(...)`.
    Always,
    /// Never included on a standard visit; included only when the key
    /// appears in `X-Inertia-Partial-Data`. Maps to
    /// `Inertia::optional(...)`.
    Optional,
    /// Like [`Optional`](Self::Optional), and additionally announced
    /// under `deferredProps` on the visit that skipped it so the client
    /// knows to fetch it. Maps to `Inertia::defer(...)`.
    Deferred,
}

/// A page prop: a value (or a resolver that produces one) plus the
/// orthogonal flags that decide when it resolves and how the client
/// folds it into the page.
///
/// The flags compose. `Prop::lazy(...).defer().merge()` is a prop the
/// initial visit announces under `deferredProps` and the follow-up
/// partial reload delivers under `mergeProps`. That is the same
/// composition the PHP adapter expresses by implementing several
/// interfaces on one class rather than by choosing one of several
/// classes — `DeferProp implements Deferrable, IgnoreFirstLoad,
/// Mergeable, Onceable` (`inertia-laravel-2.0.25/src/DeferProp.php:5`).
/// A closed enum could not spell it, which is why this is a struct.
///
/// Build one with [`eager`](Self::eager), [`lazy`](Self::lazy),
/// [`from_resolver`](Self::from_resolver), or [`absent`](Self::absent),
/// then chain flags, then attach it with
/// [`InertiaResponse::prop`](crate::InertiaResponse::prop). For the
/// single-flag cases the response builder's own shortcuts
/// (`.with`, `.always`, `.lazy`, `.optional`, `.defer`, `.merge`,
/// `.once`, `.scroll`) read better and produce the same prop.
///
/// Nothing here consults the request. Which props resolve, which land in
/// `props`, and which page-object metadata is emitted are decided in
/// `InertiaResponse::resolve`.
#[derive(Clone)]
pub struct Prop {
    source: PropSource,
    visibility: Visibility,
    /// Read only when `visibility` is [`Visibility::Deferred`].
    defer_group: Option<String>,
    /// Read only when `visibility` is [`Visibility::Deferred`].
    rescue: bool,
    merge: Option<MergeMode>,
    match_on: Vec<String>,
    /// Nested paths within the prop's value to merge at, instead of the
    /// root. Read only when `merge` is [`Some`]; ignored on
    /// [`MergeMode::Deep`], which already recurses into every field.
    merge_paths: Vec<String>,
    once: bool,
    /// Read only when `once` is set.
    once_key: Option<String>,
    /// Read only when `once` is set.
    expires_at: Option<i64>,
    /// Read only when `once` is set.
    fresh: bool,
    scroll: Option<ScrollMetadata>,
    /// Read only when `scroll` is `Some`. Set by
    /// [`scroll_wrap`](Self::scroll_wrap).
    scroll_wrap: Option<String>,
}

impl std::fmt::Debug for Prop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut s = f.debug_struct("Prop");
        match &self.source {
            PropSource::Value(v) => s.field("value", v),
            PropSource::Resolver(_) => s.field("value", &"<resolver>"),
            PropSource::Absent => s.field("value", &"<absent>"),
        };
        s.field("visibility", &self.visibility);
        // `group`/`rescue` are read only when `visibility` is `Deferred`
        // (see the fields' own doc comments), but each is shown here
        // whenever it was actually *set* too, not only when it applies.
        // Both are stored unconditionally by `.group(...)`/`.rescue()`
        // regardless of call order relative to `.defer()`, so a prop that
        // never ended up deferred can still be carrying one — and that is
        // exactly the case someone staring at a `{:?}` dump to find out
        // "why didn't my rescue/group apply" needs to see, not have
        // hidden because the flag looks inapplicable at a glance.
        if self.visibility == Visibility::Deferred || self.defer_group.is_some() {
            s.field("group", &self.defer_group());
        }
        if self.visibility == Visibility::Deferred || self.rescue {
            s.field("rescue", &self.rescue);
        }
        if let Some(mode) = self.merge {
            s.field("merge", &mode);
        }
        if !self.match_on.is_empty() {
            s.field("match_on", &self.match_on);
        }
        if !self.merge_paths.is_empty() {
            s.field("merge_paths", &self.merge_paths);
        }
        if self.once {
            s.field("once_key", &self.once_key)
                .field("expires_at", &self.expires_at)
                .field("fresh", &self.fresh);
        }
        if let Some(meta) = &self.scroll {
            s.field("scroll_page_name", &meta.page_name);
        }
        // Shown whenever `.scroll_wrap(...)` was called, not only when
        // `.scroll(...)` was also set — the same reasoning as `group`/
        // `rescue` above: a wrap key set on a prop that never got a
        // `scroll(...)` call is read by nothing, and that silence is
        // precisely what needs to be visible when debugging it.
        if let Some(wrap) = &self.scroll_wrap {
            s.field("scroll_wrap", wrap);
        }
        s.finish_non_exhaustive()
    }
}

/// One or more `matchOn` field names — what [`Prop::match_on`] accepts.
///
/// A single string names one field (`.match_on("id")`); an array or
/// `Vec` names several in one call (`.match_on(["id", "slug"])`),
/// matching Laravel's `matchOn(string|array $matchOn)`
/// (`inertia-laravel-2.0.25/src/MergesProps.php:70-75`, which wraps a
/// scalar in an array via `Arr::wrap`).
///
/// Deliberately **not** implemented via `IntoIterator<Item = impl
/// Into<String>>`: `&str` itself implements `IntoIterator` over `char`,
/// and `char: Into<String>` compiles, so that shortcut would silently
/// turn `.match_on("id")` into two one-letter fields, `"i"` and `"d"`.
/// The impls below are closed over the shapes that mean "one or more
/// whole field names," so that trap can't happen.
pub trait MatchOnFields {
    /// Consume `self` into the field names to append, in order.
    fn into_match_on_fields(self) -> Vec<String>;
}

impl MatchOnFields for &str {
    fn into_match_on_fields(self) -> Vec<String> {
        vec![self.to_string()]
    }
}

impl MatchOnFields for String {
    fn into_match_on_fields(self) -> Vec<String> {
        vec![self]
    }
}

impl<T: Into<String>, const N: usize> MatchOnFields for [T; N] {
    fn into_match_on_fields(self) -> Vec<String> {
        self.into_iter().map(Into::into).collect()
    }
}

impl<T: Into<String>> MatchOnFields for Vec<T> {
    fn into_match_on_fields(self) -> Vec<String> {
        self.into_iter().map(Into::into).collect()
    }
}

impl Prop {
    fn with_source(source: PropSource) -> Self {
        Self {
            source,
            visibility: Visibility::Standard,
            defer_group: None,
            rescue: false,
            merge: None,
            match_on: Vec::new(),
            merge_paths: Vec::new(),
            once: false,
            once_key: None,
            expires_at: None,
            fresh: false,
            scroll: None,
            scroll_wrap: None,
        }
    }

    // ---- sources -------------------------------------------------------

    /// A prop whose value is already materialized.
    ///
    /// The building block behind `.with(key, value)` and `.always(key,
    /// value)`; reach for it directly when you need to attach flags that
    /// the response builder has no shortcut for.
    pub fn eager(value: Value) -> Self {
        Self::with_source(PropSource::Value(value))
    }

    /// The absent sentinel: a prop that never reaches the response.
    ///
    /// `when_loaded!` produces this when the named relation was not
    /// preloaded on the source entity. The key is omitted entirely — no
    /// `null`, no error, and no page-object metadata even if flags are
    /// set on it.
    pub fn absent() -> Self {
        Self::with_source(PropSource::Absent)
    }

    /// A prop backed by an already-boxed [`PropResolver`].
    ///
    /// Use this when you built the resolver yourself — an
    /// [`InertiaSharedData`](crate::InertiaSharedData) implementation
    /// that needs a fallible closure, for instance. [`lazy`](Self::lazy)
    /// is the shorthand for an infallible one.
    pub fn from_resolver(resolver: PropResolver) -> Self {
        Self::with_source(PropSource::Resolver(resolver))
    }

    /// A prop backed by an async closure returning a
    /// [`serde_json::Value`] directly, with no `Result` wrapping.
    ///
    /// The closure runs only when the prop will actually be sent.
    pub fn lazy<F, Fut>(f: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Value> + Send + 'static,
    {
        Self::from_resolver(Arc::new(move || {
            let fut = f();
            Box::pin(async move { Ok(fut.await) })
        }))
    }

    // ---- visibility ----------------------------------------------------

    /// Include this prop in every response, partial-reload filtering
    /// ignored. Maps to `Inertia::always(...)`.
    ///
    /// Erases any earlier [`optional`](Self::optional) or
    /// [`defer`](Self::defer): the three are one setting.
    pub fn always(mut self) -> Self {
        self.visibility = Visibility::Always;
        self
    }

    /// Withhold this prop until the client asks for it by name in
    /// `X-Inertia-Partial-Data`. Maps to `Inertia::optional(...)`.
    ///
    /// Erases any earlier [`always`](Self::always) or
    /// [`defer`](Self::defer).
    pub fn optional(mut self) -> Self {
        self.visibility = Visibility::Optional;
        self
    }

    /// Withhold this prop *and* announce it under `deferredProps`, so
    /// the client issues a follow-up partial reload for it. Maps to
    /// `Inertia::defer(...)`.
    ///
    /// Erases any earlier [`always`](Self::always) or
    /// [`optional`](Self::optional).
    pub fn defer(mut self) -> Self {
        self.visibility = Visibility::Deferred;
        self
    }

    /// Bucket a deferred prop under a named group, so every key in the
    /// group is fetched by one follow-up request. Defaults to
    /// `"default"`.
    ///
    /// Stored unconditionally and read only when the prop is deferred,
    /// so `.group("g").defer()` and `.defer().group("g")` are the same
    /// prop. On a prop that is not deferred it has no effect.
    pub fn group(mut self, group: impl Into<String>) -> Self {
        self.defer_group = Some(group.into());
        self
    }

    /// Catch a deferred resolver's error instead of failing the
    /// response: the key is omitted from `props` and listed under
    /// `rescuedProps` so the client can render its `rescue` slot.
    ///
    /// Read only when the prop is deferred. On any other prop a resolver
    /// error still propagates as a 500 — a prop the page renders on the
    /// first paint has no rescue slot to fall back to.
    pub fn rescue(mut self) -> Self {
        self.rescue = true;
        self
    }

    // ---- merge ---------------------------------------------------------

    /// Append this prop's value to what the client already holds instead
    /// of replacing it. Maps to `Inertia::merge(...)`.
    pub fn merge(mut self) -> Self {
        self.merge = Some(MergeMode::Append);
        self
    }

    /// Prepend instead of appending. Maps to
    /// `Inertia::merge(...)->prepend()`.
    pub fn prepend(mut self) -> Self {
        self.merge = Some(MergeMode::Prepend);
        self
    }

    /// Merge structures recursively instead of concatenating at the
    /// root. Maps to `Inertia::deepMerge(...)`.
    pub fn deep_merge(mut self) -> Self {
        self.merge = Some(MergeMode::Deep);
        self
    }

    /// Merge into a nested path within this prop's value instead of the
    /// root. Maps to Laravel's `Inertia::merge($value)->append('data')`
    /// / `->prepend('data')` — the path form of `append`/`prepend`
    /// (`inertia-laravel-2.0.25/src/MergesProps.php:136-173`).
    ///
    /// Calls accumulate, so a prop with two mergeable fields can name
    /// each independently: `.merge().merge_with_path("data").merge_with_path("meta")`
    /// emits `mergeProps: ["<key>.data", "<key>.meta"]`. Naming any path
    /// also suppresses the plain root-level entry for this prop — a
    /// path-merging prop never also merges its whole value, matching
    /// `MergesProps::mergesAtRoot` (`MergesProps.php:126-129`).
    ///
    /// Read only when [`merge`](Self::merge), [`prepend`](Self::prepend),
    /// or [`merge_strategy`](Self::merge_strategy) sets an
    /// [`Append`](MergeMode::Append) or [`Prepend`](MergeMode::Prepend)
    /// mode. [`deep_merge`](Self::deep_merge) ignores it — a deep merge
    /// already recurses into every nested field, so there is nothing a
    /// path narrows (Laravel excludes deep-merge props from the
    /// root/path partition entirely, `Response.php:590`, `:610`).
    ///
    /// To dedupe array elements at that nested path too, include the
    /// path in the [`match_on`](Self::match_on) field name yourself —
    /// `.merge_with_path("data").match_on("data.id")` emits
    /// `matchPropsOn: ["<key>.data.id"]`. This does not infer the prefix
    /// for you, unlike Laravel's two-argument `append('data', 'id')`.
    ///
    /// Silently inert on a [`scroll`](Self::scroll) prop: a scroll prop's
    /// merge instruction is computed by a separate code path that reads
    /// [`scroll_wrap`](Self::scroll_wrap)'s single wrap key, not this
    /// method's accumulated path list, so `.scroll(meta).merge_with_path("data")`
    /// stores a path nothing ever reads. Use
    /// [`scroll_wrap`](Self::scroll_wrap) to nest a scroll prop's merge
    /// target instead.
    pub fn merge_with_path(mut self, path: impl Into<String>) -> Self {
        self.merge_paths.push(path.into());
        self
    }

    /// Name the field(s) the client dedupes array elements on, so a
    /// refetch that overlaps the current window replaces matching rows
    /// in place rather than appending copies. Emitted as `matchPropsOn`.
    ///
    /// Takes one field (`.match_on("id")`) or several in one call
    /// (`.match_on(["id", "slug"])`) — see [`MatchOnFields`]. Calls also
    /// accumulate, so `.match_on("id").match_on("slug")` and
    /// `.match_on(["id", "slug"])` emit the same `matchPropsOn`. The
    /// client uses the **first** entry whose path prefix matches a given
    /// merge path (`inertia-3.6.1/packages/core/src/response.ts:534-543`),
    /// so give each path at most one field.
    pub fn match_on(mut self, fields: impl MatchOnFields) -> Self {
        self.match_on.extend(fields.into_match_on_fields());
        self
    }

    /// Apply a [`MergeStrategy`] — mode plus optional match field(s) — in
    /// one call. Backs
    /// [`InertiaResponse::merge_with`](crate::InertiaResponse::merge_with).
    pub fn merge_strategy(self, strategy: MergeStrategy) -> Self {
        let (prop, match_on) = match strategy {
            MergeStrategy::Append { match_on } => (self.merge(), match_on),
            MergeStrategy::Prepend { match_on } => (self.prepend(), match_on),
            MergeStrategy::Deep { match_on } => (self.deep_merge(), match_on),
        };
        match match_on {
            Some(fields) => prop.match_on(fields),
            None => prop,
        }
    }

    // ---- once ----------------------------------------------------------

    /// Let the client cache this value across navigations. The resolver
    /// is skipped on a later visit where the client says it still holds
    /// the value, via `X-Inertia-Except-Once-Props`. Maps to
    /// `Inertia::once(...)`.
    pub fn once(mut self) -> Self {
        self.once = true;
        self
    }

    /// Override the cache key the client dedupes on. Defaults to the
    /// prop's own name; override it so several pages can share one
    /// cached value under different prop names. Maps to
    /// `Inertia::once()->as('key')`.
    ///
    /// Read only when [`once`](Self::once) is set.
    pub fn as_key(mut self, key: impl Into<String>) -> Self {
        self.once_key = Some(key.into());
        self
    }

    /// Expire the cached value at the given millis-since-epoch
    /// timestamp. The server stops honouring the client's cache claim
    /// past this point, so a stale client cannot pin an old value
    /// forever. Maps to `Inertia::once()->until($timestamp)`.
    ///
    /// Read only when [`once`](Self::once) is set.
    pub fn until(mut self, expires_at_ms: i64) -> Self {
        self.expires_at = Some(expires_at_ms);
        self
    }

    /// Resolve even when the client claims to hold a cached value.
    /// Maps to `Inertia::once()->fresh()`.
    ///
    /// Read only when [`once`](Self::once) is set.
    pub fn fresh(mut self) -> Self {
        self.fresh = true;
        self
    }

    // ---- scroll --------------------------------------------------------

    /// Attach infinite-scroll pagination metadata, emitted next to the
    /// value under `scrollProps`. Maps to `Inertia::scroll(...)`.
    ///
    /// The prop always carries merge metadata: appending by default,
    /// switching to prepend only when the client's
    /// `X-Inertia-Infinite-Scroll-Merge-Intent` header says so, and
    /// dropped for a key named in `X-Inertia-Reset` (`scrollProps[key].reset`
    /// mirrors that header independently). An explicit
    /// [`merge`](Self::merge) / [`prepend`](Self::prepend) flag on the
    /// same prop is therefore redundant, not read.
    /// [`deep_merge`](Self::deep_merge) is the one flag that still has
    /// an effect: it routes the prop into `deepMergeProps` instead,
    /// matching Laravel's `ScrollProp` (`ScrollProp implements
    /// Mergeable`, `Response.php:590,610`).
    /// [`scroll_wrap`](Self::scroll_wrap) nests the merge path under a
    /// field inside the value instead of the value's root.
    pub fn scroll(mut self, metadata: ScrollMetadata) -> Self {
        self.scroll = Some(metadata);
        self
    }

    /// Nest this scroll prop's merge instruction under `<key>.<wrap_key>`
    /// instead of the bare key. Read only when [`scroll`](Self::scroll) is
    /// also set; on any other prop it's stored and ignored, the same way
    /// [`group`](Self::group) is ignored on a non-deferred prop.
    ///
    /// Reach for this when the prop's value is itself an envelope —
    /// `{ data: [...], meta: {...} }` — and only the array inside should
    /// fold into what the client already holds. Laravel's `ScrollProp`
    /// wraps under `"data"` unconditionally
    /// (`inertia-laravel-2.0.25/src/ScrollProp.php:58-64`); Suprnova's
    /// built-in paginators hand back a bare row array, so this is opt-in
    /// rather than a default every caller has to work around. Maps to
    /// `Inertia::scroll($value, $wrapper)`.
    ///
    /// Ignored when the prop also carries [`deep_merge`](Self::deep_merge):
    /// deep merge already recurses through the entire value, so there is
    /// no nested path left for a wrapper to narrow — the same reason the
    /// general merge block ignores [`merge_with_path`](Self::merge_with_path)
    /// under [`MergeMode::Deep`].
    pub fn scroll_wrap(mut self, wrap_key: impl Into<String>) -> Self {
        self.scroll_wrap = Some(wrap_key.into());
        self
    }

    // ---- accessors -----------------------------------------------------

    /// This prop's partial-reload visibility.
    pub fn visibility(&self) -> Visibility {
        self.visibility
    }

    /// True if this prop must appear regardless of partial-reload filtering.
    pub fn is_always(&self) -> bool {
        self.visibility == Visibility::Always
    }

    /// True if the prop is withheld until the client requests it by name.
    pub fn is_optional(&self) -> bool {
        self.visibility == Visibility::Optional
    }

    /// True if the prop is deferred: withheld *and* announced under
    /// `deferredProps`.
    pub fn is_defer(&self) -> bool {
        self.visibility == Visibility::Deferred
    }

    /// True for the [`absent`](Self::absent) sentinel.
    pub fn is_absent(&self) -> bool {
        matches!(self.source, PropSource::Absent)
    }

    /// True for a resolver-backed prop carrying no flags at all — what
    /// `.lazy(...)` and `#[derive(Data)]`'s plain lazy fields produce.
    /// Says nothing about the `?include=` allowlist gate, which
    /// [`resolve_with_owner`](Self::resolve_with_owner) applies to every
    /// resolver-backed owner-tagged prop, flags or not.
    pub fn is_lazy(&self) -> bool {
        matches!(self.source, PropSource::Resolver(_))
            && self.visibility == Visibility::Standard
            && self.merge.is_none()
            && !self.once
            && self.scroll.is_none()
    }

    /// True if the prop's value comes from a closure rather than being
    /// materialized already.
    pub fn has_resolver(&self) -> bool {
        matches!(self.source, PropSource::Resolver(_))
    }

    /// The already-materialized value, if this prop has one.
    pub fn as_value(&self) -> Option<&Value> {
        match &self.source {
            PropSource::Value(v) => Some(v),
            _ => None,
        }
    }

    /// The defer group, `"default"` when none was named.
    pub fn defer_group(&self) -> &str {
        self.defer_group.as_deref().unwrap_or("default")
    }

    /// Whether a resolver error on this prop is rescued rather than
    /// propagated. Only consulted for a deferred prop.
    pub fn rescues(&self) -> bool {
        self.rescue
    }

    /// How the client should fold this prop's value in, if at all.
    pub fn merge_mode(&self) -> Option<MergeMode> {
        self.merge
    }

    /// The fields named by [`match_on`](Self::match_on), in call order.
    pub fn match_on_fields(&self) -> &[String] {
        &self.match_on
    }

    /// The paths named by [`merge_with_path`](Self::merge_with_path), in call order.
    pub fn merge_paths(&self) -> &[String] {
        &self.merge_paths
    }

    /// Whether the client caches this prop across navigations.
    pub fn is_once(&self) -> bool {
        self.once
    }

    /// The cache key the client dedupes this prop on: the
    /// [`as_key`](Self::as_key) override, or `prop_key` when none was set.
    pub fn once_cache_key(&self, prop_key: &str) -> String {
        self.once_key
            .clone()
            .unwrap_or_else(|| prop_key.to_string())
    }

    /// The cached value's expiry in millis since the epoch, if any.
    pub fn once_expires_at(&self) -> Option<i64> {
        self.expires_at
    }

    /// Whether the server refuses the client's cache claim for this prop.
    pub fn is_fresh(&self) -> bool {
        self.fresh
    }

    /// The infinite-scroll pagination metadata, if this is a scroll prop.
    pub fn scroll_metadata(&self) -> Option<&ScrollMetadata> {
        self.scroll.as_ref()
    }

    /// The nested-merge wrapper key set by [`scroll_wrap`](Self::scroll_wrap),
    /// if any.
    pub fn scroll_wrap_key(&self) -> Option<&str> {
        self.scroll_wrap.as_deref()
    }

    /// Consume the prop and hand back its source. Read every flag you
    /// need first — this is the last thing `resolve_props` does with a
    /// prop.
    pub(crate) fn into_source(self) -> PropSource {
        self.source
    }

    // ---- resolution ----------------------------------------------------

    /// Produce this prop's value, awaiting the resolver if it has one.
    ///
    /// The request-aware materialization — which props resolve at all,
    /// and the `deferredProps` / `mergeProps` / `onceProps` /
    /// `scrollProps` metadata — lives in `InertiaResponse::resolve` and
    /// uses this method internally.
    pub async fn resolve(self) -> Result<Value, FrameworkError> {
        match self.source {
            PropSource::Value(v) => Ok(v),
            PropSource::Resolver(r) => r().await,
            // Callers reach the absent sentinel through
            // `resolve_with_owner`, which returns `Ok(None)`. `Null` is
            // the safe fallback so a stray call here cannot panic.
            PropSource::Absent => Ok(Value::Null),
        }
    }

    /// Resolution path used by `#[derive(Data)]`-generated code. Consults
    /// the request's [`crate::data::RequestIncludeSet`] AND the per-DTO
    /// allowlist before invoking the lazy closure.
    ///
    /// - If this is the absent sentinel: returns `Ok(None)`.
    /// - If `field` is NOT in the request's include set: returns
    ///   `Ok(None)` (caller omits the field from the response).
    /// - If `field` IS in the include set but NOT in the DTO's allowlist:
    ///   returns `Err` with status 400 — the `IncludeError::UnknownInclude`
    ///   message body lists the field and the allowed includes.
    /// - If `field` IS in both: invokes the closure and returns
    ///   `Ok(Some(value))`.
    /// - An already-materialized value resolves without gating: the
    ///   include set decides whether a *resolver* runs, and an eager
    ///   value is in memory either way.
    ///
    /// Flags on the prop make no difference. A `#[data(lazy(deferred))]`
    /// field is `Visibility::Deferred`, so [`is_lazy`](Self::is_lazy) is
    /// false for it — gating on that predicate handed every flagged
    /// owner-tagged prop a free pass around the allowlist.
    pub async fn resolve_with_owner(
        self,
        owner_struct_name: &str,
        field: &str,
    ) -> Result<Option<Value>, FrameworkError> {
        if !self.passes_include_gate(owner_struct_name, field)? {
            return Ok(None);
        }
        Ok(Some(self.resolve().await?))
    }

    /// The `?include=` + allowlist decision behind
    /// [`resolve_with_owner`](Self::resolve_with_owner), split out so
    /// `resolve_props` can apply it to a *flagged* owner-tagged prop
    /// without also taking over that prop's resolution — a deferred prop
    /// still needs the announce path, and one carrying `.rescue()` still
    /// needs its rescue-aware resolver arm.
    ///
    /// `Ok(false)` means "omit this field from the response entirely":
    /// the absent sentinel, or a field the request never opted into.
    /// `Err` is the 400 an out-of-allowlist `?include=` earns, and the
    /// caller must let it propagate rather than swallow it — the whole
    /// point of running this gate ahead of partial-reload filtering.
    pub(crate) fn passes_include_gate(
        &self,
        owner_struct_name: &str,
        field: &str,
    ) -> Result<bool, FrameworkError> {
        use crate::data::{IncludeError, current_include_set, registry};

        // Absent sentinel — relation was not preloaded; silently skip.
        if self.is_absent() {
            return Ok(false);
        }
        if !self.has_resolver() {
            return Ok(true);
        }
        if !current_include_set().includes(field) {
            return Ok(false);
        }
        if !registry::is_allowed(owner_struct_name, field) {
            return Err(IncludeError::UnknownInclude {
                field: field.to_string(),
                allowed: registry::allowed_for(owner_struct_name)
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
            }
            .into_framework_error());
        }
        Ok(true)
    }
}

/// Decision engine for partial-reload filtering.
///
/// Built from the request's `X-Inertia-Partial-*` headers and the component
/// name of the response being rendered. Per the v3 protocol:
///
/// - If `X-Inertia-Partial-Component` is absent or does not match the
///   response's component, the filter is inactive (treat as a standard
///   visit — no filtering applied).
/// - If `X-Inertia-Partial-Data` is set, treat it as a whitelist.
/// - If `X-Inertia-Partial-Except` is set, treat it as a blacklist that
///   takes precedence over the whitelist on conflicts.
/// - Props flagged [`Visibility::Always`] bypass this filter.
/// - Props flagged [`Visibility::Optional`] or [`Visibility::Deferred`]
///   use the explicit-only predicate (must be in `only`).
/// - The `errors` prop is always returned (handled by the caller).
#[derive(Debug, Clone, Default)]
pub struct PartialFilter {
    /// True when the request's `X-Inertia-Partial-Component` matched the
    /// response's component. When false, no filtering is applied to
    /// [`Visibility::Standard`] props, and [`Visibility::Optional`] /
    /// [`Visibility::Deferred`] props are excluded outright.
    pub matched: bool,
    /// Whitelist of prop keys (parsed from `X-Inertia-Partial-Data`).
    pub only: Option<Vec<String>>,
    /// Blacklist of prop keys (parsed from `X-Inertia-Partial-Except`).
    pub except: Option<Vec<String>>,
}

impl PartialFilter {
    /// Build a filter from the request and the response's component name.
    pub fn build<R: InertiaRequestExt + ?Sized>(req: &R, component: &str) -> Self {
        let partial_component = req.header("X-Inertia-Partial-Component");
        let matched = partial_component.map(|c| c == component).unwrap_or(false);

        if !matched {
            return Self::default();
        }

        let parse_csv = |raw: &str| -> Vec<String> {
            raw.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        };

        Self {
            matched: true,
            only: req.header("X-Inertia-Partial-Data").map(parse_csv),
            except: req.header("X-Inertia-Partial-Except").map(parse_csv),
        }
    }

    /// Whether an Eager-or-Lazy prop with `key` should be included.
    ///
    /// On a non-partial-reload (or partial reload targeting a different
    /// component), inclusion defaults to true. On a matched partial
    /// reload, inclusion follows the only/except rules per the v3 spec
    /// (except wins).
    ///
    /// An `only` entry may name `key` exactly (`"user"`), a dotted path
    /// *inside* it (`"user.name"`), or an *ancestor* of it
    /// (`"auth"` against the prop key `"auth.user"`); all three forms
    /// make `key` participate here. The first two leave
    /// [`narrow`](Self::narrow) to trim the resolved value down to the
    /// requested nested paths; the third ships it whole, because the
    /// caller asked for the whole root.
    ///
    /// The ancestor form is what keeps a dotted *prop key* reachable.
    /// `App::inertia_share("auth.user", …)` stores the literal key
    /// `auth.user` and `unpack_map` nests it into `props.auth.user` only
    /// after every prop has resolved, so a client asking for
    /// `only=auth` has to match the still-flat key here or the share
    /// disappears. Laravel never needs this because `Inertia::share`
    /// runs `Arr::set` at share time
    /// (`inertia-laravel-2.0.25/src/ResponseFactory.php:94`), leaving
    /// `Arr::get($props, 'auth')` a plain top-level lookup.
    ///
    /// `except` is dot-aware in one direction only. A dotted except
    /// entry (`"user.email"`) prunes a field out of an otherwise-included
    /// `user`, it does not drop `user` altogether. A bare entry, or one
    /// naming an ancestor of `key`, drops the prop outright — Laravel's
    /// `Arr::forget($props, 'auth')` takes the whole `auth` subtree with
    /// it, `auth.user` included
    /// (`inertia-laravel-2.0.25/src/Response.php:292-294`).
    pub fn should_include_eager(&self, key: &str) -> bool {
        if !self.matched {
            return true;
        }
        let mut included = match &self.only {
            Some(list) => list.iter().any(|k| entry_selects_key(k, key)),
            None => true,
        };
        if included
            && let Some(except) = &self.except
            && except.iter().any(|k| k == key || dotted_ancestor(k, key))
        {
            included = false;
        }
        included
    }

    /// Whether an Optional prop with `key` should be included.
    ///
    /// Per the v3 protocol, Optional props are **never** included on a
    /// standard visit (or a partial reload targeting another component)
    /// and **only** included on a matched partial reload when the key
    /// appears in `X-Inertia-Partial-Data` and not in
    /// `X-Inertia-Partial-Except`.
    ///
    /// Dot-aware the same way
    /// [`should_include_eager`](Self::should_include_eager) is:
    /// `"permissions.read"` in `only` counts as an explicit request for
    /// `"permissions"`, narrowed later by [`narrow`](Self::narrow) — this
    /// is what lets a dotted request against a
    /// `Defer`/`Optional` prop actually trigger its resolver. The
    /// ancestor form works here too, so a lazily shared `auth.user`
    /// still resolves under `only=auth`, and a bare `except` entry drops
    /// every prop key beneath it.
    pub fn should_include_optional(&self, key: &str) -> bool {
        if !self.matched {
            return false;
        }
        let in_only = match &self.only {
            Some(list) => list.iter().any(|k| entry_selects_key(k, key)),
            None => return false, // Optional requires explicit request
        };
        if !in_only {
            return false;
        }
        if let Some(except) = &self.except
            && except.iter().any(|k| k == key || dotted_ancestor(k, key))
        {
            return false;
        }
        true
    }

    /// Dispatch the per-prop inclusion predicate.
    ///
    /// Reads the prop's [`Visibility`] and nothing else: `Always`
    /// bypasses the filter, `Optional` and `Deferred` require the key to
    /// appear in `X-Inertia-Partial-Data`, and `Standard` follows the
    /// only/except rules. The absent sentinel is never included.
    ///
    /// This answers "does the value ship". It deliberately does **not**
    /// answer "does this prop's metadata ship" — merge, once, and
    /// deferred metadata are gated by
    /// [`should_include_eager`](Self::should_include_eager) alone, the
    /// way Laravel gates them (`inertia-laravel-2.0.25/src/Response.php:553-560`),
    /// so a deferred prop still carries its merge instruction on the
    /// visit that skipped its value.
    pub fn should_include(&self, key: &str, prop: &Prop) -> bool {
        if prop.is_absent() {
            return false;
        }
        match prop.visibility() {
            Visibility::Always => true,
            Visibility::Optional | Visibility::Deferred => self.should_include_optional(key),
            Visibility::Standard => self.should_include_eager(key),
        }
    }

    /// Narrow a resolved prop's value down to the nested paths named by
    /// dot-notation entries in `only`/`except`, for the given top-level
    /// `key`.
    ///
    /// Laravel walks the dotted path *before* resolution, on the raw,
    /// often-closure-backed prop bag
    /// (`inertia-laravel-2.0.25/src/Response.php:273-297`,
    /// `Arr::get`/`Arr::set`). Suprnova resolves every prop's value
    /// first — necessarily, since resolvers are async — and narrows the
    /// already-materialized [`Value`] afterward. The shape this produces
    /// is exactly what `only=user.name` is documented to mean:
    /// `{"user": {"name": ...}}`. The client reconstructs the full
    /// object by deep-merging that slice onto whatever it already holds
    /// for `user`
    /// (`inertia-3.6.1/packages/core/src/response.ts:414-425`).
    ///
    /// A path that doesn't resolve against `value` — an unknown field,
    /// or one that drills through a scalar or an array instead of an
    /// object — contributes nothing for that path and does not affect
    /// any other requested path. This is a deliberate divergence from
    /// Laravel's `Arr::get`, whose missing-key default is `null`: a
    /// stray `null` here would overwrite a field the client's own
    /// merge-on-top reconciliation already has cached, which is worse
    /// than omitting it.
    ///
    /// Only called for a key that has already passed
    /// [`should_include`](Self::should_include) or
    /// [`should_include_eager`](Self::should_include_eager) — this
    /// method decides shape, not inclusion. The caller must not call it
    /// for an `Always` prop: Laravel's `resolveAlways` re-injects an
    /// `AlwaysProp`'s raw, unfiltered value
    /// (`inertia-laravel-2.0.25/src/Response.php:406-416`), never
    /// narrowed.
    ///
    /// Public alongside [`should_include_eager`](Self::should_include_eager)
    /// and [`should_include_optional`](Self::should_include_optional): a
    /// caller building its own `InertiaResponse`-like surface on top of
    /// this type — a custom adapter, a test harness — gets `true` from
    /// `should_include_eager("user")` under `only=["user.name"]`, but has
    /// no way to reproduce the narrowing that makes that `true` correct
    /// without this method, and would ship `user` whole instead.
    pub fn narrow(&self, key: &str, value: Value) -> Value {
        if !self.matched {
            return value;
        }

        let mut narrowed = if let Some(list) = &self.only {
            let mut bare = false;
            let mut nested_paths: Vec<Vec<&str>> = Vec::new();
            for entry in list {
                // An exact match asks for the whole prop; so does an
                // entry naming an ancestor of a dotted prop key
                // (`only=auth` against the key `auth.user`) — the
                // requested root contains this prop entire, so there is
                // no nested path left to trim to.
                if entry == key || dotted_ancestor(entry, key) {
                    bare = true;
                    break;
                }
                if let Some(rest) = dotted_child(entry, key) {
                    nested_paths.push(rest.split('.').collect());
                }
            }
            if bare || nested_paths.is_empty() {
                value
            } else {
                narrow_to_paths(&value, &nested_paths)
            }
        } else {
            value
        };

        if let Some(except) = &self.except {
            for entry in except {
                if let Some(rest) = dotted_child(entry, key) {
                    let segments: Vec<&str> = rest.split('.').collect();
                    remove_path(&mut narrowed, &segments);
                }
            }
        }

        narrowed
    }
}

/// `Some(rest)` when `entry` names a dotted path *inside* `key` — `key`
/// followed by `.` and at least one more segment. `None` for an exact
/// match (`entry == key`, handled separately by callers) and for an
/// unrelated key that merely shares a prefix — `"userAgent"` must not
/// match `key = "user"`, which a plain [`str::starts_with`] would
/// wrongly allow.
fn dotted_child<'a>(entry: &'a str, key: &str) -> Option<&'a str> {
    entry
        .strip_prefix(key)
        .and_then(|rest| rest.strip_prefix('.'))
        .filter(|rest| !rest.is_empty())
}

/// True when `entry` names a strict *ancestor* of `key` — the mirror of
/// [`dotted_child`], asking the same question with the two arguments
/// swapped. `entry = "auth"` against `key = "auth.user"` is an ancestor;
/// `"authAgent"` is not, and neither is an exact match (callers handle
/// that case separately).
///
/// This is the case a dotted *prop key* creates: the props map is flat
/// until `unpack_map` runs, so `auth.user` is one literal key that an
/// `only`/`except` entry of `auth` has to be able to reach.
fn dotted_ancestor(entry: &str, key: &str) -> bool {
    dotted_child(key, entry).is_some()
}

/// True when an `only` entry selects `key` in any of its three forms:
/// an exact match, a dotted path inside `key`, or an ancestor of `key`.
/// Shared by [`PartialFilter::should_include_eager`] and
/// [`PartialFilter::should_include_optional`] so the two never drift.
fn entry_selects_key(entry: &str, key: &str) -> bool {
    entry == key || dotted_child(entry, key).is_some() || dotted_ancestor(entry, key)
}

/// Build a fresh JSON object containing only the requested nested
/// `paths` out of `value`. A path that does not resolve — an unknown
/// key, or a segment that walks into a scalar or an array rather than
/// an object — contributes nothing and does not affect any other
/// requested path.
fn narrow_to_paths(value: &Value, paths: &[Vec<&str>]) -> Value {
    let mut result = Value::Object(serde_json::Map::new());
    for path in paths {
        if let Some(found) = get_path(value, path) {
            set_path(&mut result, path, found.clone());
        }
    }
    result
}

/// Walk `path` through `value`'s object nesting, returning the value at
/// the end. `None` the instant a segment is missing or the current
/// value is not a JSON object — a dotted path into a scalar or an array
/// has nothing to find, and is treated exactly like an unknown key.
fn get_path<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = value;
    for segment in path {
        current = current.as_object()?.get(*segment)?;
    }
    Some(current)
}

/// Write `leaf` into `target` at the nested `path`, creating
/// intermediate objects as needed. Only ever called with a `target`
/// that is (or becomes) an object at every level along `path` —
/// [`narrow_to_paths`] always starts from an empty object, so there is
/// never a pre-existing non-object value to reconcile with.
///
/// This module deliberately does **not** reuse [`super::dotted::arr_get`]
/// for the walk that backs `set_path`/[`get_path`]/[`remove_path`], even
/// though both are dot-notation walkers over `serde_json::Value`.
/// `arr_get` checks for an *exact*, undotted key first
/// (`object.get(key)` before ever splitting on `.`), mirroring Laravel's
/// `Arr::get`'s `static::exists($array, $key)` short-circuit — correct at
/// the *props-array* level, where a literal dotted key like
/// `"user.name"` can legitimately be one whole top-level prop key rather
/// than a path. `narrow` operates one level below that: inside an
/// already-resolved prop's own JSON *value*, where the dotted `only`/
/// `except` entry is always a path to walk, never a literal key to
/// match first. Reusing `arr_get` here would silently import the wrong
/// semantics — a value shaped like `{"a.b": 1}` would match on
/// `only=["a.b"]` as an exact key instead of failing to resolve `a` then
/// `b` as a path, which is what this task's dot-notation contract
/// requires. If a future refactor is tempted to unify these two
/// walkers, this is why they don't share one.
fn set_path(target: &mut Value, path: &[&str], leaf: Value) {
    match path.split_first() {
        None => {}
        Some((head, [])) => {
            if let Some(obj) = target.as_object_mut() {
                obj.insert((*head).to_string(), leaf);
            }
        }
        Some((head, rest)) => {
            if let Some(obj) = target.as_object_mut() {
                let entry = obj
                    .entry((*head).to_string())
                    .or_insert_with(|| Value::Object(serde_json::Map::new()));
                set_path(entry, rest, leaf);
            }
        }
    }
}

/// Delete the value at the nested `path` inside `target`, if present. A
/// no-op when any intermediate segment is missing or is not an object —
/// removing something that was never there is not an error.
fn remove_path(target: &mut Value, path: &[&str]) {
    match path.split_first() {
        None => {}
        Some((head, [])) => {
            if let Some(obj) = target.as_object_mut() {
                obj.remove(*head);
            }
        }
        Some((head, rest)) => {
            if let Some(obj) = target.as_object_mut()
                && let Some(child) = obj.get_mut(*head)
            {
                remove_path(child, rest);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn lazy_resolver(value: Value) -> PropResolver {
        Arc::new(move || {
            let v = value.clone();
            Box::pin(async move { Ok(v) })
        })
    }

    fn failing_resolver() -> PropResolver {
        Arc::new(|| Box::pin(async move { Err(FrameworkError::internal("resolver exploded")) }))
    }

    #[test]
    fn filter_inactive_when_component_does_not_match() {
        let filter = PartialFilter::default();
        assert!(!filter.matched);
        assert!(filter.should_include_eager("any_key"));
        // Optional excluded when filter inactive.
        assert!(!filter.should_include_optional("any_key"));
    }

    #[test]
    fn filter_with_only_whitelist() {
        let filter = PartialFilter {
            matched: true,
            only: Some(vec!["users".into(), "events".into()]),
            except: None,
        };
        assert!(filter.should_include_eager("users"));
        assert!(filter.should_include_eager("events"));
        assert!(!filter.should_include_eager("auth"));
    }

    #[test]
    fn filter_with_except_blacklist() {
        let filter = PartialFilter {
            matched: true,
            only: None,
            except: Some(vec!["auth".into()]),
        };
        assert!(filter.should_include_eager("users"));
        assert!(!filter.should_include_eager("auth"));
    }

    #[test]
    fn filter_except_takes_precedence_over_only() {
        let filter = PartialFilter {
            matched: true,
            only: Some(vec!["users".into(), "auth".into()]),
            except: Some(vec!["auth".into()]),
        };
        assert!(filter.should_include_eager("users"));
        assert!(!filter.should_include_eager("auth"));
    }

    #[test]
    fn optional_excluded_on_standard_visit() {
        let filter = PartialFilter::default();
        assert!(!filter.should_include_optional("permissions"));
    }

    #[test]
    fn optional_excluded_when_only_unset_on_partial() {
        // Matched filter, no `only` list — optional must remain excluded
        // because it requires explicit listing.
        let filter = PartialFilter {
            matched: true,
            only: None,
            except: None,
        };
        assert!(!filter.should_include_optional("permissions"));
    }

    #[test]
    fn optional_included_only_when_in_only_list() {
        let filter = PartialFilter {
            matched: true,
            only: Some(vec!["permissions".into()]),
            except: None,
        };
        assert!(filter.should_include_optional("permissions"));
        assert!(!filter.should_include_optional("users"));
    }

    #[test]
    fn optional_excluded_when_in_except_even_if_in_only() {
        let filter = PartialFilter {
            matched: true,
            only: Some(vec!["permissions".into()]),
            except: Some(vec!["permissions".into()]),
        };
        assert!(!filter.should_include_optional("permissions"));
    }

    #[test]
    fn should_include_dispatches_per_visibility() {
        let filter = PartialFilter {
            matched: true,
            only: Some(vec!["wanted".into()]),
            except: None,
        };
        let always = Prop::eager(json!(1)).always();
        let eager = Prop::eager(json!(2));
        let lazy = Prop::from_resolver(lazy_resolver(json!(3)));
        let optional = Prop::from_resolver(lazy_resolver(json!(4))).optional();
        let deferred = Prop::from_resolver(lazy_resolver(json!(5))).defer();
        let absent = Prop::absent();

        // Always wins regardless of key.
        assert!(filter.should_include("ignored", &always));
        // Standard visibility: in-only -> in, out-of-only -> out.
        assert!(filter.should_include("wanted", &eager));
        assert!(!filter.should_include("nope", &eager));
        assert!(filter.should_include("wanted", &lazy));
        assert!(!filter.should_include("nope", &lazy));
        // Optional and Deferred: explicit request only.
        assert!(filter.should_include("wanted", &optional));
        assert!(!filter.should_include("nope", &optional));
        assert!(filter.should_include("wanted", &deferred));
        assert!(!filter.should_include("nope", &deferred));
        // The absent sentinel is never included, whatever the key.
        assert!(!filter.should_include("wanted", &absent));
    }

    #[test]
    fn visibility_flags_are_mutually_exclusive_and_last_wins() {
        assert_eq!(
            Prop::eager(json!(1)).optional().always().visibility(),
            Visibility::Always
        );
        assert_eq!(
            Prop::eager(json!(1)).always().optional().visibility(),
            Visibility::Optional
        );
        assert_eq!(
            Prop::eager(json!(1)).always().defer().visibility(),
            Visibility::Deferred
        );
        assert_eq!(
            Prop::eager(json!(1)).defer().always().visibility(),
            Visibility::Always
        );
        assert_eq!(Prop::eager(json!(1)).visibility(), Visibility::Standard);
    }

    #[test]
    fn merge_flags_replace_one_another() {
        assert_eq!(
            Prop::eager(json!(1)).merge().merge_mode(),
            Some(MergeMode::Append)
        );
        assert_eq!(
            Prop::eager(json!(1)).merge().prepend().merge_mode(),
            Some(MergeMode::Prepend)
        );
        assert_eq!(
            Prop::eager(json!(1)).prepend().deep_merge().merge_mode(),
            Some(MergeMode::Deep)
        );
        assert_eq!(Prop::eager(json!(1)).merge_mode(), None);
    }

    #[test]
    fn match_on_accumulates_in_call_order() {
        let p = Prop::eager(json!(1))
            .merge()
            .match_on("id")
            .match_on("slug");
        assert_eq!(p.match_on_fields(), ["id".to_string(), "slug".to_string()]);
    }

    #[test]
    fn match_on_accepts_an_array_in_one_call_and_still_chains_with_single_calls() {
        let p = Prop::eager(json!(1)).merge().match_on(["id", "slug"]);
        assert_eq!(p.match_on_fields(), ["id".to_string(), "slug".to_string()]);

        let p = Prop::eager(json!(1))
            .merge()
            .match_on("id")
            .match_on(["slug", "uuid"]);
        assert_eq!(
            p.match_on_fields(),
            ["id".to_string(), "slug".to_string(), "uuid".to_string()]
        );
    }

    #[test]
    fn merge_with_path_accumulates_and_is_stored_even_without_a_merge_mode() {
        let p = Prop::eager(json!(1))
            .merge()
            .merge_with_path("data")
            .merge_with_path("meta");
        assert_eq!(p.merge_paths(), ["data".to_string(), "meta".to_string()]);

        // Stored unconditionally, like `group()`/`rescue()` — read only
        // when a merge mode is set. See the `merge_with_path_alone_…`
        // integration test in `inertia_merge_paths.rs` for the wire-level
        // proof that it has no effect here.
        let ignored = Prop::eager(json!(1)).merge_with_path("data");
        assert_eq!(ignored.merge_paths(), ["data".to_string()]);
        assert_eq!(ignored.merge_mode(), None);
    }

    #[test]
    fn merge_strategy_maps_onto_the_flags() {
        let p = Prop::eager(json!(1)).merge_strategy(MergeStrategy::Append {
            match_on: Some(vec!["id".into()]),
        });
        assert_eq!(p.merge_mode(), Some(MergeMode::Append));
        assert_eq!(p.match_on_fields(), ["id".to_string()]);

        let p = Prop::eager(json!(1)).merge_strategy(MergeStrategy::Prepend { match_on: None });
        assert_eq!(p.merge_mode(), Some(MergeMode::Prepend));
        assert!(p.match_on_fields().is_empty());

        let p = Prop::eager(json!(1)).merge_strategy(MergeStrategy::Deep {
            match_on: Some(vec!["uuid".into()]),
        });
        assert_eq!(p.merge_mode(), Some(MergeMode::Deep));
        assert_eq!(p.match_on_fields(), ["uuid".to_string()]);
    }

    /// Pins the widening this task exists for: `MergeStrategy` used to
    /// carry `match_on: Option<String>`, so a builder shortcut like
    /// `InertiaResponse::merge_with` could express at most one dedupe
    /// field — strictly less than `.prop(k, Prop::eager(v).match_on([...]))`
    /// could already do. `match_on: Option<Vec<String>>` closes that gap;
    /// this proves a `MergeStrategy` can carry more than one field name
    /// and `merge_strategy` forwards all of them in order, matching what
    /// `Prop::match_on` does directly.
    #[test]
    fn merge_strategy_carries_more_than_one_match_on_field() {
        let p = Prop::eager(json!(1)).merge_strategy(MergeStrategy::Append {
            match_on: Some(vec!["id".into(), "slug".into()]),
        });
        assert_eq!(p.merge_mode(), Some(MergeMode::Append));
        assert_eq!(p.match_on_fields(), ["id".to_string(), "slug".to_string()]);
    }

    #[test]
    fn defer_group_defaults_to_default_and_is_overridable() {
        assert_eq!(Prop::eager(json!(1)).defer().defer_group(), "default");
        assert_eq!(
            Prop::eager(json!(1))
                .defer()
                .group("attributes")
                .defer_group(),
            "attributes"
        );
        // `group` is order-independent: it is a stored field, read only
        // when the visibility is Deferred.
        assert_eq!(
            Prop::eager(json!(1))
                .group("attributes")
                .defer()
                .defer_group(),
            "attributes"
        );
    }

    #[test]
    fn once_cache_key_defaults_to_the_prop_key() {
        let p = Prop::eager(json!(1)).once();
        assert!(p.is_once());
        assert_eq!(p.once_cache_key("plans"), "plans");
        assert_eq!(p.once_expires_at(), None);
        assert!(!p.is_fresh());

        let p = Prop::eager(json!(1))
            .once()
            .as_key("roles")
            .until(42)
            .fresh();
        assert_eq!(p.once_cache_key("memberRoles"), "roles");
        assert_eq!(p.once_expires_at(), Some(42));
        assert!(p.is_fresh());
    }

    #[test]
    fn is_lazy_is_true_only_for_an_unflagged_resolver() {
        assert!(Prop::from_resolver(lazy_resolver(json!(1))).is_lazy());
        assert!(!Prop::eager(json!(1)).is_lazy());
        assert!(!Prop::absent().is_lazy());
        assert!(
            !Prop::from_resolver(lazy_resolver(json!(1)))
                .optional()
                .is_lazy()
        );
        assert!(
            !Prop::from_resolver(lazy_resolver(json!(1)))
                .merge()
                .is_lazy()
        );
        assert!(
            !Prop::from_resolver(lazy_resolver(json!(1)))
                .once()
                .is_lazy()
        );
        assert!(
            !Prop::from_resolver(lazy_resolver(json!(1)))
                .scroll(ScrollMetadata::new("page"))
                .is_lazy()
        );
    }

    #[tokio::test]
    async fn prop_resolve_eager() {
        let p = Prop::eager(json!({"hi": 1}));
        assert_eq!(p.as_value(), Some(&json!({"hi": 1})));
        let v = p.resolve().await.unwrap();
        assert_eq!(v, json!({"hi": 1}));
    }

    #[tokio::test]
    async fn prop_resolve_always() {
        let p = Prop::eager(json!("yo")).always();
        assert!(p.is_always());
        let v = p.resolve().await.unwrap();
        assert_eq!(v, json!("yo"));
    }

    #[tokio::test]
    async fn prop_resolve_lazy_awaits_closure() {
        let p = Prop::from_resolver(lazy_resolver(json!([1, 2, 3])));
        assert!(p.as_value().is_none());
        let v = p.resolve().await.unwrap();
        assert_eq!(v, json!([1, 2, 3]));
    }

    #[tokio::test]
    async fn prop_resolve_optional_awaits_closure() {
        let p = Prop::from_resolver(lazy_resolver(json!({"perm": "read"}))).optional();
        assert!(p.is_optional());
        let v = p.resolve().await.unwrap();
        assert_eq!(v, json!({"perm": "read"}));
    }

    #[tokio::test]
    async fn prop_resolve_propagates_resolver_error() {
        let p = Prop::from_resolver(failing_resolver());
        let err = p.resolve().await.unwrap_err();
        assert!(err.to_string().contains("resolver exploded"));
    }

    #[tokio::test]
    async fn absent_prop_resolves_to_null_rather_than_panicking() {
        // Callers reach the absent sentinel through `resolve_with_owner`,
        // which returns `Ok(None)`. A stray `resolve` must still be safe.
        assert!(Prop::absent().is_absent());
        assert_eq!(Prop::absent().resolve().await.unwrap(), Value::Null);
    }

    #[test]
    fn prop_marker_predicates() {
        assert!(Prop::eager(json!(1)).always().is_always());
        assert!(!Prop::eager(json!(1)).is_always());

        assert!(
            Prop::from_resolver(lazy_resolver(json!(1)))
                .optional()
                .is_optional()
        );
        assert!(!Prop::from_resolver(lazy_resolver(json!(1))).is_optional());

        assert!(
            Prop::from_resolver(lazy_resolver(json!(1)))
                .defer()
                .is_defer()
        );
        assert!(!Prop::from_resolver(lazy_resolver(json!(1))).is_defer());

        assert!(Prop::from_resolver(lazy_resolver(json!(1))).has_resolver());
        assert!(
            Prop::from_resolver(lazy_resolver(json!(1)))
                .once()
                .has_resolver()
        );
        assert!(!Prop::eager(json!(1)).has_resolver());
        assert!(!Prop::absent().has_resolver());
    }

    #[test]
    fn rescue_is_stored_regardless_of_visibility() {
        assert!(Prop::eager(json!(1)).rescue().rescues());
        assert!(!Prop::eager(json!(1)).rescues());
        assert!(Prop::eager(json!(1)).defer().rescue().rescues());
    }

    #[test]
    fn scroll_metadata_round_trips() {
        let p = Prop::eager(json!([])).scroll(ScrollMetadata::new("cursor").current("c-1"));
        let meta = p.scroll_metadata().expect("scroll flag set");
        assert_eq!(meta.page_name, "cursor");
        assert_eq!(meta.current_page, Some(json!("c-1")));
        assert!(Prop::eager(json!([])).scroll_metadata().is_none());
    }

    #[test]
    fn debug_lists_the_flags_that_are_set() {
        let rendered = format!(
            "{:?}",
            Prop::eager(json!(1)).defer().group("g").merge().once()
        );
        assert!(rendered.contains("Deferred"), "got {rendered}");
        assert!(rendered.contains("\"g\""), "got {rendered}");
        assert!(rendered.contains("Append"), "got {rendered}");
        assert!(rendered.contains("once_key"), "got {rendered}");
    }

    /// `group`, `rescue`, and `scroll_wrap` are each read only under a
    /// condition (deferred visibility; a scroll prop) that the caller
    /// below never satisfies, so every one of these three calls is a
    /// silent no-op. `{:?}` is exactly where someone debugging "why
    /// didn't my group/rescue/wrap apply" looks first, so all three must
    /// show up regardless — hiding them because they look inapplicable
    /// is what made them hard to debug in the first place.
    #[test]
    fn debug_shows_group_rescue_and_scroll_wrap_even_when_stored_but_ignored() {
        let rendered = format!(
            "{:?}",
            Prop::eager(json!(1))
                .group("ungrouped") // not deferred: this group is never read
                .rescue() // not deferred: this rescue is never read
                .scroll_wrap("data") // no `.scroll(...)`: this wrap is never read
        );
        assert!(
            rendered.contains("\"ungrouped\""),
            "group set on a non-deferred prop must still show: got {rendered}"
        );
        assert!(
            rendered.contains("rescue: true"),
            "rescue set on a non-deferred prop must still show: got {rendered}"
        );
        assert!(
            rendered.contains("scroll_wrap"),
            "scroll_wrap set without .scroll(...) must still show: got {rendered}"
        );
        assert!(
            rendered.contains("\"data\""),
            "scroll_wrap's key must be visible: got {rendered}"
        );
    }

    // ---- T26: dot-notation only/except --------------------------------

    #[test]
    fn should_include_eager_treats_a_dotted_only_entry_as_participation() {
        let filter = PartialFilter {
            matched: true,
            only: Some(vec!["user.name".into()]),
            except: None,
        };
        assert!(
            filter.should_include_eager("user"),
            "a dotted only entry must make its top-level key participate"
        );
        assert!(!filter.should_include_eager("other"));
    }

    #[test]
    fn should_include_eager_except_stays_bare_for_the_inclusion_decision() {
        let filter = PartialFilter {
            matched: true,
            only: None,
            except: Some(vec!["user.email".into()]),
        };
        // A dotted except entry prunes a field later, via `narrow` — it
        // must not drop the whole key here. Only a bare except entry does.
        assert!(filter.should_include_eager("user"));
    }

    #[test]
    fn should_include_eager_bare_except_still_excludes_the_whole_key() {
        let filter = PartialFilter {
            matched: true,
            only: None,
            except: Some(vec!["user".into()]),
        };
        assert!(!filter.should_include_eager("user"));
    }

    #[test]
    fn should_include_optional_is_dot_aware_too() {
        let filter = PartialFilter {
            matched: true,
            only: Some(vec!["permissions.read".into()]),
            except: None,
        };
        assert!(filter.should_include_optional("permissions"));
        assert!(!filter.should_include_optional("other"));
    }

    #[test]
    fn narrow_returns_the_whole_value_when_the_filter_is_not_matched() {
        let filter = PartialFilter::default();
        let value = json!({"name": "a", "email": "b"});
        assert_eq!(filter.narrow("user", value.clone()), value);
    }

    #[test]
    fn narrow_returns_the_whole_value_when_only_names_the_bare_key() {
        let filter = PartialFilter {
            matched: true,
            only: Some(vec!["user".into()]),
            except: None,
        };
        let value = json!({"name": "a", "email": "b"});
        assert_eq!(filter.narrow("user", value.clone()), value);
    }

    #[test]
    fn narrow_builds_a_nested_object_from_a_dotted_only_entry() {
        let filter = PartialFilter {
            matched: true,
            only: Some(vec!["user.name".into()]),
            except: None,
        };
        let value = json!({"name": "a", "email": "b"});
        assert_eq!(filter.narrow("user", value), json!({"name": "a"}));
    }

    #[test]
    fn narrow_bare_only_entry_wins_over_a_narrower_dotted_entry() {
        let filter = PartialFilter {
            matched: true,
            only: Some(vec!["user".into(), "user.name".into()]),
            except: None,
        };
        let value = json!({"name": "a", "email": "b"});
        assert_eq!(filter.narrow("user", value.clone()), value);
    }

    #[test]
    fn narrow_removes_a_dotted_except_path_leaving_siblings() {
        let filter = PartialFilter {
            matched: true,
            only: None,
            except: Some(vec!["user.email".into()]),
        };
        let value = json!({"name": "a", "email": "b"});
        assert_eq!(filter.narrow("user", value), json!({"name": "a"}));
    }

    #[test]
    fn narrow_except_wins_over_only_on_the_same_nested_path() {
        let filter = PartialFilter {
            matched: true,
            only: Some(vec!["user.email".into()]),
            except: Some(vec!["user.email".into()]),
        };
        let value = json!({"name": "a", "email": "b"});
        assert_eq!(filter.narrow("user", value), json!({}));
    }

    #[test]
    fn narrow_drops_an_unknown_nested_path_without_touching_its_siblings() {
        let filter = PartialFilter {
            matched: true,
            only: Some(vec![
                "user.name".into(),
                "user.bogus".into(),
                "user.email".into(),
            ]),
            except: None,
        };
        let value = json!({"name": "a", "email": "b"});
        assert_eq!(
            filter.narrow("user", value),
            json!({"name": "a", "email": "b"})
        );
    }

    #[test]
    fn narrow_drops_a_path_that_walks_through_a_scalar_intermediate() {
        let filter = PartialFilter {
            matched: true,
            only: Some(vec!["config.level.nested".into(), "config.theme".into()]),
            except: None,
        };
        let value = json!({"theme": "dark", "level": 3});
        assert_eq!(filter.narrow("config", value), json!({"theme": "dark"}));
    }

    // ---- T26 review follow-up: multi-segment recursion + prefix guard --

    #[test]
    fn narrow_builds_a_three_segment_nested_object_from_a_dotted_only_entry() {
        // Pins the recursive arm of `set_path`/`get_path` beyond one
        // nested level — every other `only` test in this module bottoms
        // out after a single segment, so this is the only coverage for
        // `Some((head, rest))` actually recursing instead of just
        // terminating on `Some((head, []))`.
        let filter = PartialFilter {
            matched: true,
            only: Some(vec!["user.profile.city".into()]),
            except: None,
        };
        let value = json!({"profile": {"city": "NYC", "zip": "10001"}, "name": "A"});
        assert_eq!(
            filter.narrow("user", value),
            json!({"profile": {"city": "NYC"}})
        );
    }

    #[test]
    fn narrow_removes_a_three_segment_nested_except_path_leaving_siblings() {
        // The `except` counterpart: pins `remove_path`'s recursive arm
        // beyond one nested level.
        let filter = PartialFilter {
            matched: true,
            only: None,
            except: Some(vec!["user.profile.zip".into()]),
        };
        let value = json!({"profile": {"city": "NYC", "zip": "10001"}, "name": "A"});
        assert_eq!(
            filter.narrow("user", value),
            json!({"profile": {"city": "NYC"}, "name": "A"})
        );
    }

    #[test]
    fn should_include_eager_a_prefix_sharing_key_does_not_count_as_a_dotted_child() {
        // `dotted_child`'s own doc comment names this hazard: "userAgent"
        // sharing a plain string prefix with "user" must not be treated
        // as a dotted path *into* "user". Without the `strip_prefix('.')`
        // guard, `only=["userAgent.name"]` would wrongly make the
        // unrelated `user` prop participate.
        let filter = PartialFilter {
            matched: true,
            only: Some(vec!["userAgent.name".into()]),
            except: None,
        };
        assert!(!filter.should_include_eager("user"));
    }

    // ---- ancestor entries: the mirror of the dotted-child cases above ----

    #[test]
    fn should_include_eager_a_bare_only_entry_reaches_a_dotted_prop_key_beneath_it() {
        // `App::inertia_share("auth.user", …)` stores the literal key
        // `auth.user`; `unpack_map` nests it only after every prop has
        // resolved. A client asking for `only=auth` is asking for
        // everything under that root, so the key has to participate.
        let filter = PartialFilter {
            matched: true,
            only: Some(vec!["auth".into()]),
            except: None,
        };
        assert!(filter.should_include_eager("auth.user"));
        assert!(!filter.should_include_eager("authAgent.user"));
        assert!(!filter.should_include_eager("other.user"));
    }

    #[test]
    fn should_include_optional_a_bare_only_entry_reaches_a_dotted_prop_key_beneath_it() {
        let filter = PartialFilter {
            matched: true,
            only: Some(vec!["auth".into()]),
            except: None,
        };
        assert!(filter.should_include_optional("auth.user"));
        assert!(!filter.should_include_optional("other.user"));
    }

    #[test]
    fn a_bare_except_entry_drops_every_dotted_prop_key_beneath_it() {
        // `Arr::forget($props, ['auth'])` on Laravel's already-nested
        // shared bag takes the whole `auth` subtree with it, `auth.user`
        // included (`inertia-laravel-2.0.25/src/Response.php:292-294`).
        let filter = PartialFilter {
            matched: true,
            only: None,
            except: Some(vec!["auth".into()]),
        };
        assert!(!filter.should_include_eager("auth.user"));
        assert!(filter.should_include_eager("authAgent.user"));

        let filter = PartialFilter {
            matched: true,
            only: Some(vec!["auth".into()]),
            except: Some(vec!["auth".into()]),
        };
        assert!(!filter.should_include_optional("auth.user"));
    }

    #[test]
    fn narrow_ships_a_dotted_prop_key_whole_for_an_ancestor_only_entry() {
        // The caller asked for `auth`, and `auth.user` is one whole prop
        // *inside* that root — there is nothing left to narrow, so the
        // value ships as-is and `unpack_map` nests it afterwards.
        let filter = PartialFilter {
            matched: true,
            only: Some(vec!["auth".into()]),
            except: None,
        };
        let value = json!({"id": 1, "name": "Todd"});
        assert_eq!(filter.narrow("auth.user", value.clone()), value);

        // An ancestor entry alongside a narrower dotted one still ships
        // whole, the same way `only=["user", "user.name"]` does:
        // `Arr::set($newProps, 'auth', …)` writes the full subtree and the
        // later leaf write only overwrites that leaf.
        let filter = PartialFilter {
            matched: true,
            only: Some(vec!["auth".into(), "auth.user.name".into()]),
            except: None,
        };
        assert_eq!(filter.narrow("auth.user", value.clone()), value);
    }

    #[test]
    fn scroll_wrap_key_reads_back_the_nested_path_segment() {
        let p = Prop::eager(json!([]))
            .scroll(ScrollMetadata::new("page"))
            .scroll_wrap("data");
        assert_eq!(p.scroll_wrap_key(), Some("data"));
    }

    #[test]
    fn scroll_wrap_key_is_none_when_never_set() {
        let p = Prop::eager(json!([])).scroll(ScrollMetadata::new("page"));
        assert_eq!(p.scroll_wrap_key(), None);
    }

    struct FixedCursorPage;

    impl ProvidesScrollMetadata for FixedCursorPage {
        fn page_name(&self) -> String {
            "cursor".to_string()
        }
        fn previous_page(&self) -> Option<Value> {
            Some(json!("prev-token"))
        }
        fn next_page(&self) -> Option<Value> {
            Some(json!("next-token"))
        }
        fn current_page(&self) -> Option<Value> {
            Some(json!("cur-token"))
        }
    }

    #[test]
    fn provides_scroll_metadata_default_impl_builds_scroll_metadata_from_the_four_methods() {
        let meta = FixedCursorPage.scroll_metadata();
        assert_eq!(meta.page_name, "cursor");
        assert_eq!(meta.previous_page, Some(json!("prev-token")));
        assert_eq!(meta.next_page, Some(json!("next-token")));
        assert_eq!(meta.current_page, Some(json!("cur-token")));
    }
}
