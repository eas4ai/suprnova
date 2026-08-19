use super::config::{Frontend, InertiaConfig};
use super::dotted;
use super::flash;
use super::prop::{
    DeferOptions, InertiaRequestExt, MergeMode, MergeStrategy, OnceOptions, PartialFilter, Prop,
    PropResolver, PropSource, ScrollMetadata, Visibility,
};
use crate::container::App;
use crate::csrf::csrf_token;
use crate::error::FrameworkError;
use crate::http::HttpResponse;
use indexmap::IndexMap;
use serde::Serialize;
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Pinned boxed task future used when resolving lazy Inertia props.
type TaskFuture = Pin<Box<dyn Future<Output = Result<TaskOutcome, FrameworkError>> + Send>>;

/// A single prop entry returned by `#[derive(Data)]`'s `__into_inertia_props`.
///
/// - `Eager` — the field's value is already serialized; inserted directly
///   into the response prop bag.
/// - `LazyOwned` — standard lazy / `#[data(lazy)]` / `#[data(lazy(inertia))]`.
///   Must pass the `?include=` + allowlist gate before resolution.
/// - `DeferredOwned` — `#[data(lazy(deferred))]`. Same `?include=` gate as
///   `LazyOwned`; the variant tag signals Inertia deferred-props protocol to
///   the client (follow-up XHR). For v1, resolved via the same code path as
///   `LazyOwned`.
/// - `ClosureOwned` — `#[data(lazy(closure))]`. Same `?include=` gate for v1;
///   future releases will resolve eagerly on the initial visit. The variant
///   tag is preserved for downstream protocol differentiation.
#[derive(Debug)]
pub enum PropEntry {
    /// Already-serialized eager value to be inserted into the prop bag verbatim.
    Eager(serde_json::Value),
    /// Lazy field gated by the `?include=` + per-DTO allowlist before resolution.
    LazyOwned {
        /// Name of the owning DTO struct (used for allowlist lookup).
        owner: &'static str,
        /// Name of the field within that DTO (used for include-set matching).
        field: &'static str,
        /// The lazy [`Prop`] whose resolver fires when the field is requested.
        prop: Prop,
    },
    /// Deferred field; resolved on a follow-up Inertia partial-reload XHR.
    DeferredOwned {
        /// Name of the owning DTO struct.
        owner: &'static str,
        /// Name of the field within that DTO.
        field: &'static str,
        /// The deferred [`Prop`] resolved on the follow-up XHR.
        prop: Prop,
    },
    /// Closure-resolved field. Same include-set gate as `LazyOwned` for v1.
    ClosureOwned {
        /// Name of the owning DTO struct.
        owner: &'static str,
        /// Name of the field within that DTO.
        field: &'static str,
        /// The closure-backed [`Prop`].
        prop: Prop,
    },
}

/// Marker trait implemented by `#[derive(Data)]`-derived types so
/// `Inertia::data` can dispatch on them. Carries the macro-generated
/// `__into_inertia_props` surface — users should not implement this
/// manually.
pub trait IntoInertiaData {
    /// Drain `self` into the macro-emitted `(prop_name, entry)` pairs the
    /// Inertia response merges into its prop bag.
    fn __into_inertia_props(self) -> Vec<(String, PropEntry)>;

    /// Fallible sibling of [`__into_inertia_props`](Self::__into_inertia_props):
    /// returns `Err(FrameworkError)` naming the offending field if a field's
    /// `Serialize` impl fails, instead of panicking.
    ///
    /// `#[derive(Data)]` overrides this with `?`-propagating per-field
    /// serialization. The default delegates to the infallible method, so a
    /// hand-written impl keeps working (its serialization happens there).
    /// Reach this through [`Inertia::try_data`](crate::Inertia::try_data)
    /// rather than calling it directly.
    fn __try_into_inertia_props(self) -> Result<Vec<(String, PropEntry)>, FrameworkError>
    where
        Self: Sized,
    {
        Ok(self.__into_inertia_props())
    }
}

/// Builder for Inertia.js page responses.
///
/// Construct with a component name, attach props with [`with`](Self::with),
/// [`always`](Self::always), [`lazy`](Self::lazy), [`optional`](Self::optional),
/// [`defer`](Self::defer), [`merge`](Self::merge), [`once`](Self::once), or
/// [`flash`](Self::flash). Optionally set a page title or override the
/// [`InertiaConfig`]. Then call [`resolve`](Self::resolve) with the current
/// request to produce an [`HttpResponse`].
pub struct InertiaResponse {
    component: String,
    props: IndexMap<String, Prop>,
    flash: serde_json::Map<String, Value>,
    config: InertiaConfig,
    title: Option<String>,
    /// Per-response history-encryption override. `Some(true)` forces
    /// encryption on, `Some(false)` forces off, `None` defers to the
    /// middleware task-local + config default. Maps to
    /// `Inertia::encryptHistory($bool)`.
    encrypt_history: Option<bool>,
    /// When `true`, the page object carries `clearHistory: true` so the
    /// client rotates its history-encryption key. Maps to
    /// `Inertia::clearHistory()`.
    clear_history: bool,
    /// Per-response override for the `preserveFragment` page-object
    /// flag. `None` defers to the session-flash flag set by
    /// `Redirect::preserve_fragment()`; `Some(true)` forces on;
    /// `Some(false)` forces off, defeating any inbound flashed `true`.
    /// Maps to `Inertia::preserveFragment()` per-response, with the
    /// session-flash mechanism mirroring Laravel's
    /// `redirect()->preserveFragment()` chainable.
    preserve_fragment: Option<bool>,
    /// Sidecar map for props registered via `prop_lazy_with_owner`.
    /// Maps the prop key to `(owner_struct_name, field_name)` so that
    /// `resolve_props` can call `Prop::resolve_with_owner` instead of
    /// the plain lazy path. Keyed by the same string as `props`.
    lazy_owned: IndexMap<String, (&'static str, &'static str)>,
}

impl InertiaResponse {
    /// Begin a new Inertia response for the given page component.
    ///
    /// The response starts from the config the app passed to
    /// [`crate::Inertia::install`], and falls back to
    /// [`InertiaConfig::default`] when nothing was installed, so an app or
    /// a test that never calls `install` needs no config of its own.
    /// Override for one response with [`with_config`](Self::with_config).
    pub fn new(component: impl Into<String>) -> Self {
        Self {
            component: component.into(),
            props: IndexMap::new(),
            flash: serde_json::Map::new(),
            // One `RwLock` read and one clone of the config per response:
            // a few short strings and paths, the version (a `String`
            // unless `.version_with(..)` made it a shared closure), and
            // refcount bumps for the manifest cache and url resolver.
            // Cheaper than the `InertiaConfig::default()` it replaces,
            // which read env vars and built a fresh manifest cache on
            // every response.
            config: crate::App::inertia_registry()
                .installed_config()
                .unwrap_or_default(),
            title: None,
            encrypt_history: None,
            clear_history: false,
            preserve_fragment: None,
            lazy_owned: IndexMap::new(),
        }
    }

    /// Override the default [`InertiaConfig`] for this response.
    ///
    /// Replaces the config wholesale, `version` included.
    /// [`InertiaVersionMiddleware`](crate::InertiaVersionMiddleware) still
    /// resolves the version [`Inertia::install`](crate::Inertia::install)
    /// was given, so a config here that doesn't carry the same
    /// `.version(...)` makes the page object advertise a version the
    /// middleware will bounce — the client takes one extra full page load
    /// after visiting that page. Set `.version(...)` on the override to
    /// match.
    pub fn with_config(mut self, config: InertiaConfig) -> Self {
        self.config = config;
        self
    }

    /// Set the `<title>` for the HTML shell on this response.
    ///
    /// On Inertia XHR responses the title is ignored — `<Head>` on the
    /// client manages document title for SPA visits. The configured title
    /// is only used for the initial HTML render.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Attach an eager prop. Honors partial-reload filtering per the v3
    /// protocol — when the client sends `X-Inertia-Partial-Data` matching
    /// the same component, this key is included only if it's in that list
    /// (and not in `X-Inertia-Partial-Except`).
    pub fn with<V: Serialize>(mut self, key: impl Into<String>, value: V) -> Self {
        let v = to_value_or_die(&value);
        self.props.insert(key.into(), Prop::eager(v));
        self
    }

    /// Attach an always-included prop. Bypasses partial-reload filtering —
    /// always returned in the response, even when the client requested a
    /// narrower set. Maps to Laravel's `Inertia::always($value)`.
    pub fn always<V: Serialize>(mut self, key: impl Into<String>, value: V) -> Self {
        let v = to_value_or_die(&value);
        self.props.insert(key.into(), Prop::eager(v).always());
        self
    }

    /// Attach an always-included prop backed by an async resolver — the
    /// resolver sibling of [`always`](Self::always). Maps to Laravel's
    /// `Inertia::always(fn () => ...)`: `AlwaysProp` accepts any value,
    /// closures included (`AlwaysProp.php`), and Suprnova splits that into
    /// two methods the way it already splits `.with`/`.lazy` and
    /// `.once`/`.once_with`. Reach for this when the always-included
    /// value is worth computing lazily — a DB read, an HTTP call — not
    /// when you already have the value in hand (`.always` covers that).
    pub fn always_with<F, Fut, V>(mut self, key: impl Into<String>, resolver: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<V, FrameworkError>> + Send + 'static,
        V: Serialize + 'static,
    {
        let resolver = make_resolver(resolver);
        self.props
            .insert(key.into(), Prop::from_resolver(resolver).always());
        self
    }

    /// Attach a lazy prop. The async closure runs only when the prop will
    /// actually be sent to the client — typically once on the initial visit
    /// or when explicitly requested via `X-Inertia-Partial-Data`. Maps to
    /// Laravel's `fn () => ...` prop pattern.
    ///
    /// Despite the name, this is **not** Laravel's `Inertia::lazy()` —
    /// that method is deprecated and behaves like `optional()` (skipped
    /// entirely on the initial visit; `LazyProp` is a straight alias for
    /// `OptionalProp`, `ResponseFactory.php:174-181`). Suprnova's `.lazy`
    /// is the plain-closure convention Laravel itself uses for a callable
    /// prop with no wrapper at all — included whenever the key passes
    /// partial-reload filtering, standard visits included. Reach for
    /// [`optional`](Self::optional) for the initial-visit-skipped
    /// behavior the name "lazy" suggests if you're coming from Laravel.
    pub fn lazy<F, Fut, V>(mut self, key: impl Into<String>, resolver: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<V, FrameworkError>> + Send + 'static,
        V: Serialize + 'static,
    {
        let resolver = make_resolver(resolver);
        self.props.insert(key.into(), Prop::from_resolver(resolver));
        self
    }

    /// Attach a lazy prop owned by a `#[derive(Data)]` DTO.
    ///
    /// The prop key is `field` (they are always identical in the DTO pattern).
    /// During resolution the `RequestIncludeSet` task-local is consulted via
    /// `Prop::resolve_with_owner`: the closure runs only when `field` appears
    /// in `?include=` AND is in the DTO's allowlist. Returns `400` to the
    /// client if the include set asks for a field not in the allowlist.
    ///
    /// Composition with `X-Inertia-Partial-Data`: partial-data is applied as
    /// a pre-resolution gate (the existing `should_include_eager` check), so
    /// the include-set gate and the partial-data filter compose correctly —
    /// a field must pass both to be resolved and returned.
    pub fn prop_lazy_with_owner(
        mut self,
        owner_struct_name: &'static str,
        field: &'static str,
        prop: Prop,
    ) -> Self {
        self.props.insert(field.to_string(), prop);
        self.lazy_owned
            .insert(field.to_string(), (owner_struct_name, field));
        self
    }

    /// Attach a fully composed [`Prop`] under `key`.
    ///
    /// The other builder methods each set one flag. This is how you set
    /// more than one — a deferred prop that also merges, a merge prop the
    /// client caches, an optional prop with a custom cache key:
    ///
    /// ```rust,no_run
    /// use suprnova::{InertiaResponse, Prop};
    /// use serde_json::json;
    ///
    /// let response = InertiaResponse::new("Feed/Index").prop(
    ///     "posts",
    ///     Prop::lazy(|| async { json!([{ "id": 1 }]) })
    ///         .defer()
    ///         .merge()
    ///         .match_on("id"),
    /// );
    /// # let _ = response;
    /// ```
    ///
    /// The prop replaces any earlier prop registered under the same key,
    /// like every other builder method.
    pub fn prop(mut self, key: impl Into<String>, prop: Prop) -> Self {
        self.props.insert(key.into(), prop);
        self
    }

    /// Build an `InertiaResponse` from the `Vec<(String, PropEntry)>` produced
    /// by a `#[derive(Data)]` DTO's `__into_inertia_props`.
    ///
    /// Dispatches on each entry variant:
    /// - `Eager` → inserted directly via the internal prop map (equivalent to `.with(key, value)`).
    /// - `LazyOwned` → routed through `prop_lazy_with_owner` so the
    ///   `?include=` + allowlist gate applies at resolution time.
    pub fn from_data_props(component: &'static str, props: Vec<(String, PropEntry)>) -> Self {
        let mut r = Self::new(component);
        for (k, entry) in props {
            match entry {
                PropEntry::Eager(v) => {
                    r.props.insert(k, Prop::eager(v));
                }
                PropEntry::LazyOwned { owner, field, prop }
                | PropEntry::DeferredOwned { owner, field, prop }
                | PropEntry::ClosureOwned { owner, field, prop } => {
                    r.props.insert(k, prop);
                    r.lazy_owned.insert(field.to_string(), (owner, field));
                }
            }
        }
        r
    }

    /// Attach an optional prop. Never included on standard visits;
    /// included only when explicitly requested via `X-Inertia-Partial-Data`
    /// on a matching partial reload. Maps to `Inertia::optional(...)`.
    pub fn optional<F, Fut, V>(mut self, key: impl Into<String>, resolver: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<V, FrameworkError>> + Send + 'static,
        V: Serialize + 'static,
    {
        let resolver = make_resolver(resolver);
        self.props
            .insert(key.into(), Prop::from_resolver(resolver).optional());
        self
    }

    /// Attach a deferred prop. The resolver is **not** called on the
    /// initial visit; the key is emitted under `deferredProps` so the
    /// client can issue a follow-up partial-reload XHR. On that
    /// follow-up the resolver runs and the value lands in `props`.
    /// Maps to `Inertia::defer(...)`.
    pub fn defer<F, Fut, V>(self, key: impl Into<String>, resolver: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<V, FrameworkError>> + Send + 'static,
        V: Serialize + 'static,
    {
        self.defer_with(key, DeferOptions::default(), resolver)
    }

    /// Attach a deferred prop with explicit options
    /// ([`DeferOptions::group`](crate::DeferOptions::group),
    /// [`DeferOptions::rescue`](crate::DeferOptions::rescue)). Maps to
    /// `Inertia::defer(..., $group)` and `Inertia::defer(..., rescue: true)`.
    pub fn defer_with<F, Fut, V>(
        mut self,
        key: impl Into<String>,
        options: DeferOptions,
        resolver: F,
    ) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<V, FrameworkError>> + Send + 'static,
        V: Serialize + 'static,
    {
        let resolver = make_resolver(resolver);
        let mut prop = Prop::from_resolver(resolver).defer().group(options.group);
        if options.rescue {
            prop = prop.rescue();
        }
        self.props.insert(key.into(), prop);
        self
    }

    /// Attach a mergeable prop with an eager value (append-at-root). The
    /// value lands in `props` AND the key is emitted under `mergeProps`
    /// so the client appends into existing client-side state on
    /// partial reloads. Maps to `Inertia::merge($value)`.
    pub fn merge<V: Serialize>(self, key: impl Into<String>, value: V) -> Self {
        self.merge_with(key, value, MergeStrategy::Append { match_on: None })
    }

    /// Attach a prepend-merge prop with an eager value. Maps to
    /// `Inertia::merge($value)->prepend()`.
    pub fn merge_prepend<V: Serialize>(self, key: impl Into<String>, value: V) -> Self {
        self.merge_with(key, value, MergeStrategy::Prepend { match_on: None })
    }

    /// Attach a deep-merge prop with an eager value. Maps to
    /// `Inertia::deepMerge($value)`.
    pub fn deep_merge<V: Serialize>(self, key: impl Into<String>, value: V) -> Self {
        self.merge_with(key, value, MergeStrategy::Deep { match_on: None })
    }

    /// Attach a mergeable prop with explicit strategy (append / prepend /
    /// deep) and optional `match_on` field for diff-merging by key.
    pub fn merge_with<V: Serialize>(
        mut self,
        key: impl Into<String>,
        value: V,
        strategy: MergeStrategy,
    ) -> Self {
        let v = to_value_or_die(&value);
        self.props
            .insert(key.into(), Prop::eager(v).merge_strategy(strategy));
        self
    }

    /// Attach a mergeable prop whose value comes from an async resolver
    /// instead of being materialized eagerly — append strategy, no
    /// `match_on`. The resolver sibling of [`InertiaResponse::merge`].
    /// Maps to `Inertia::merge(fn () => ...)` (`MergeProp` resolves a
    /// `Closure` value via `ResolvesCallables`,
    /// `inertia-laravel-2.0.25/src/MergeProp.php:24-29`).
    ///
    /// The resolver runs only when the merge prop will actually be sent
    /// — skipped by partial-reload filtering and by [`Prop::defer`] like
    /// any other resolver-backed prop. Reach for
    /// `.prop(key, Prop::lazy(...).merge())` instead when the prop also
    /// needs a visibility or cache flag.
    pub fn merge_lazy<F, Fut, V>(self, key: impl Into<String>, resolver: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<V, FrameworkError>> + Send + 'static,
        V: Serialize + 'static,
    {
        self.merge_lazy_with(key, MergeStrategy::Append { match_on: None }, resolver)
    }

    /// Attach a mergeable prop with an explicit [`MergeStrategy`] whose
    /// value comes from an async resolver. The resolver sibling of
    /// [`InertiaResponse::merge_with`].
    pub fn merge_lazy_with<F, Fut, V>(
        mut self,
        key: impl Into<String>,
        strategy: MergeStrategy,
        resolver: F,
    ) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<V, FrameworkError>> + Send + 'static,
        V: Serialize + 'static,
    {
        let resolver = make_resolver(resolver);
        self.props.insert(
            key.into(),
            Prop::from_resolver(resolver).merge_strategy(strategy),
        );
        self
    }

    /// Attach a once prop. The resolver runs the first time the client
    /// sees this key; on subsequent visits the client signals it already
    /// has the value via `X-Inertia-Except-Once-Props` and the resolver
    /// is skipped. Maps to `Inertia::once(...)`.
    pub fn once<F, Fut, V>(self, key: impl Into<String>, resolver: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<V, FrameworkError>> + Send + 'static,
        V: Serialize + 'static,
    {
        self.once_with(key, OnceOptions::default(), resolver)
    }

    /// Attach a once prop with explicit options
    /// ([`OnceOptions::until`](crate::OnceOptions::until),
    /// [`OnceOptions::as_key`](crate::OnceOptions::as_key),
    /// [`OnceOptions::fresh`](crate::OnceOptions::fresh)).
    pub fn once_with<F, Fut, V>(
        mut self,
        key: impl Into<String>,
        options: OnceOptions,
        resolver: F,
    ) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<V, FrameworkError>> + Send + 'static,
        V: Serialize + 'static,
    {
        let resolver = make_resolver(resolver);
        let mut prop = Prop::from_resolver(resolver).once();
        if let Some(cache_key) = options.cache_key {
            prop = prop.as_key(cache_key);
        }
        if let Some(expires_at) = options.expires_at {
            prop = prop.until(expires_at);
        }
        if options.fresh {
            prop = prop.fresh();
        }
        self.props.insert(key.into(), prop);
        self
    }

    /// Attach an infinite-scroll prop with an eager value. The
    /// framework normalizes the data shape: the value lands in `props`
    /// and the pagination metadata is emitted under `scrollProps`. The
    /// client's `<InfiniteScroll>` component reads both to drive
    /// next/previous fetches.
    ///
    /// A scroll prop always carries merge metadata — unlike a plain
    /// merge prop, it needs no explicit `.merge()` — defaulting to
    /// append and switching to prepend only when the client sends
    /// `X-Inertia-Infinite-Scroll-Merge-Intent: prepend`. This matches
    /// `ScrollProp::configureMergeIntent`
    /// (`inertia-laravel-2.0.25/src/ScrollProp.php:72-79`), which runs
    /// unconditionally on every response, fresh visits included.
    ///
    /// `scrollProps[key].reset` is `true` exactly when the client named
    /// `key` in `X-Inertia-Reset` — the same header a regular merge prop
    /// reads, and independent of the merge-intent header above
    /// (`Response.php:700-716`). A reset key is also excluded from
    /// `mergeProps` / `prependProps` for that response, so the client
    /// treats the value as a replacement instead of an append.
    ///
    /// Merges at the prop's root by default. When the value is itself an
    /// envelope (`{ data: [...], meta: {...} }`), reach for
    /// [`scroll_wrapped`](Self::scroll_wrapped) to target the nested
    /// field instead of the whole value.
    ///
    /// Maps to Laravel's `Inertia::scroll(...)`.
    pub fn scroll<V: Serialize>(
        self,
        key: impl Into<String>,
        metadata: ScrollMetadata,
        value: V,
    ) -> Self {
        let v = to_value_or_die(&value);
        self.attach_scroll(key.into(), None, metadata, Prop::eager(v))
    }

    /// Attach an infinite-scroll prop whose value is produced by an
    /// async resolver. Useful when the paginated data requires a DB
    /// query or other async work — common for real scroll loaders.
    pub fn scroll_with<F, Fut, V>(
        self,
        key: impl Into<String>,
        metadata: ScrollMetadata,
        resolver: F,
    ) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<V, FrameworkError>> + Send + 'static,
        V: Serialize + 'static,
    {
        let resolver = make_resolver(resolver);
        self.attach_scroll(key.into(), None, metadata, Prop::from_resolver(resolver))
    }

    /// Attach an infinite-scroll prop whose merge instruction targets a
    /// nested field of the value instead of the value itself —
    /// `key.wrap_key` rather than bare `key`. Use this when the value is
    /// an envelope (`{ data: [...], meta: {...} }`) and only the array
    /// inside should fold into what the client already holds; a plain
    /// [`scroll`](Self::scroll) merges the whole value.
    ///
    /// Equivalent to
    /// `Prop::eager(value).scroll(metadata).scroll_wrap(wrap_key)`
    /// attached under `key`.
    pub fn scroll_wrapped<V: Serialize>(
        self,
        key: impl Into<String>,
        wrap_key: impl Into<String>,
        metadata: ScrollMetadata,
        value: V,
    ) -> Self {
        let v = to_value_or_die(&value);
        self.attach_scroll(key.into(), Some(wrap_key.into()), metadata, Prop::eager(v))
    }

    /// Async-resolved sibling of [`scroll_wrapped`](Self::scroll_wrapped).
    pub fn scroll_with_wrapped<F, Fut, V>(
        self,
        key: impl Into<String>,
        wrap_key: impl Into<String>,
        metadata: ScrollMetadata,
        resolver: F,
    ) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<V, FrameworkError>> + Send + 'static,
        V: Serialize + 'static,
    {
        let resolver = make_resolver(resolver);
        self.attach_scroll(
            key.into(),
            Some(wrap_key.into()),
            metadata,
            Prop::from_resolver(resolver),
        )
    }

    fn attach_scroll(
        mut self,
        key: String,
        wrap_key: Option<String>,
        metadata: ScrollMetadata,
        prop: Prop,
    ) -> Self {
        let mut prop = prop.scroll(metadata);
        if let Some(wrap) = wrap_key {
            prop = prop.scroll_wrap(wrap);
        }
        self.props.insert(key, prop);
        self
    }

    /// Attach a paginator (`LengthAwarePaginator` or `CursorPaginator`)
    /// as a scroll prop under `key`. The paginator's metadata becomes
    /// the prop's `ScrollMetadata`; its rows become the prop value.
    ///
    /// Equivalent to `.scroll(key, paginator.into_inertia_scroll().0, paginator.into_inertia_scroll().1)`,
    /// but reads better at the call site.
    pub fn paginate<T>(
        self,
        key: &'static str,
        paginator: impl crate::pagination::IntoInertiaScroll<T>,
    ) -> Self
    where
        T: Serialize + 'static,
    {
        let (meta, data) = paginator.into_inertia_scroll();
        self.scroll(key, meta, data)
    }

    /// Attach a flash value to this response. Appears under the
    /// top-level `flash` field of the page object (not under `props`).
    /// Use for one-shot toasts / success messages.
    pub fn flash<V: Serialize>(mut self, key: impl Into<String>, value: V) -> Self {
        let v = to_value_or_die(&value);
        self.flash.insert(key.into(), v);
        self
    }

    // ---- Fallible (try_*) prop builders -------------------------------
    //
    // Each mirrors the infallible eager-prop method above but returns
    // `Err(FrameworkError)` (naming the prop key) instead of panicking when
    // the value's `Serialize` impl fails. The infallible siblings stay as
    // ergonomic escape hatches: on the HTTP request path a panic is caught
    // by the panic-recovery middleware and converted to a 500. Prefer the
    // `try_*` form when building responses off that path (queue workers, the
    // scheduler, CLI) where no such net exists, or whenever you want to
    // handle a serialization failure explicitly.

    /// Fallible sibling of [`with`](Self::with).
    pub fn try_with<V: Serialize>(
        mut self,
        key: impl Into<String>,
        value: V,
    ) -> Result<Self, FrameworkError> {
        let key = key.into();
        let v = to_value_or_err(&key, &value)?;
        self.props.insert(key, Prop::eager(v));
        Ok(self)
    }

    /// Fallible sibling of [`always`](Self::always).
    pub fn try_always<V: Serialize>(
        mut self,
        key: impl Into<String>,
        value: V,
    ) -> Result<Self, FrameworkError> {
        let key = key.into();
        let v = to_value_or_err(&key, &value)?;
        self.props.insert(key, Prop::eager(v).always());
        Ok(self)
    }

    /// Fallible sibling of [`merge_with`](Self::merge_with). The convenience
    /// wrappers ([`merge`](Self::merge), [`deep_merge`](Self::deep_merge),
    /// etc.) delegate to the infallible `merge_with`; use this when the
    /// merged value's serialization may fail.
    pub fn try_merge_with<V: Serialize>(
        mut self,
        key: impl Into<String>,
        value: V,
        strategy: MergeStrategy,
    ) -> Result<Self, FrameworkError> {
        let key = key.into();
        let v = to_value_or_err(&key, &value)?;
        self.props
            .insert(key, Prop::eager(v).merge_strategy(strategy));
        Ok(self)
    }

    /// Fallible sibling of [`scroll`](Self::scroll). For an async-resolved
    /// scroll value, [`scroll_with`](Self::scroll_with) is already fallible.
    pub fn try_scroll<V: Serialize>(
        self,
        key: impl Into<String>,
        metadata: ScrollMetadata,
        value: V,
    ) -> Result<Self, FrameworkError> {
        let key = key.into();
        let v = to_value_or_err(&key, &value)?;
        Ok(self.attach_scroll(key, None, metadata, Prop::eager(v)))
    }

    /// Fallible sibling of [`scroll_wrapped`](Self::scroll_wrapped). For an
    /// async-resolved wrapped scroll value,
    /// [`scroll_with_wrapped`](Self::scroll_with_wrapped) is already
    /// fallible.
    pub fn try_scroll_wrapped<V: Serialize>(
        self,
        key: impl Into<String>,
        wrap_key: impl Into<String>,
        metadata: ScrollMetadata,
        value: V,
    ) -> Result<Self, FrameworkError> {
        let key = key.into();
        let v = to_value_or_err(&key, &value)?;
        Ok(self.attach_scroll(key, Some(wrap_key.into()), metadata, Prop::eager(v)))
    }

    /// Fallible sibling of [`flash`](Self::flash).
    pub fn try_flash<V: Serialize>(
        mut self,
        key: impl Into<String>,
        value: V,
    ) -> Result<Self, FrameworkError> {
        let key = key.into();
        let v = to_value_or_err(&key, &value)?;
        self.flash.insert(key, v);
        Ok(self)
    }

    /// Force history encryption on or off for this response. Overrides
    /// both [`EncryptHistoryMiddleware`](crate::EncryptHistoryMiddleware)
    /// and [`InertiaConfig::encrypt_history_default`](crate::InertiaConfig::encrypt_history_default).
    /// Maps to `Inertia::encryptHistory($bool)`.
    pub fn encrypt_history(mut self, on: bool) -> Self {
        self.encrypt_history = Some(on);
        self
    }

    /// Mark **this** response so the client rotates its
    /// history-encryption key. Subsequent attempts to decrypt prior
    /// history entries fail and the client refetches them.
    ///
    /// Use this when the response you are returning *is* the page that
    /// should clear. When the clearing handler redirects — logout is the
    /// canonical case — reach for [`App::clear_history`](crate::App::clear_history)
    /// instead: the redirect's own response is discarded by the browser,
    /// so the flag has to ride the redirect and land on the page that
    /// actually renders. Maps to `Inertia::clearHistory()`, which is
    /// session-backed in Laravel for the same reason.
    pub fn clear_history(mut self) -> Self {
        self.clear_history = true;
        self
    }

    /// Set the `preserveFragment` flag on the page object. When the
    /// client receives a page with this flag set, it carries the URL
    /// fragment (`#anchor`) over to the new URL when this page is the
    /// destination of a redirect.
    ///
    /// Precedence: per-response wins over the session-flash flag set
    /// by [`Redirect::preserve_fragment`](crate::Redirect::preserve_fragment).
    /// Specifically, `.preserve_fragment(false)` defeats an inbound
    /// flashed `true`, so a destination controller can opt out of the
    /// fragment carry even when the redirect requested it.
    pub fn preserve_fragment(mut self, on: bool) -> Self {
        self.preserve_fragment = Some(on);
        self
    }

    /// Build a `409 Conflict` external-redirect response. The client
    /// performs `window.location = url`, doing a full page navigation
    /// (not an Inertia SPA visit). Maps to `Inertia::location($url)`.
    ///
    /// **When to use which redirect form:**
    /// - [`Redirect::to`](crate::Redirect::to) — standard 302/303 with
    ///   `Location` header. The normal case for redirects after form
    ///   submission inside the Inertia app.
    /// - [`InertiaResponse::redirect`](Self::redirect) — 409 +
    ///   `X-Inertia-Redirect` for soft Inertia SPA navigation; use
    ///   when the redirect must carry a `#fragment` (server `Location`
    ///   headers can't carry fragments through Inertia XHR).
    /// - [`InertiaResponse::location`](Self::location) — 409 +
    ///   `X-Inertia-Location` for full-page reload via
    ///   `window.location`; use to leave the Inertia app entirely.
    ///   Always returns the 409 form, so only reach for it where the
    ///   request is already known to be an Inertia visit — otherwise use
    ///   [`location_for`](Self::location_for), which falls back to a plain
    ///   `302` for a hard navigation.
    pub fn location(url: impl AsRef<str>) -> HttpResponse {
        HttpResponse::new()
            .status(409)
            .header("X-Inertia-Location", url.as_ref())
    }

    /// Request-aware external redirect — Laravel's `Inertia::location($url)`.
    ///
    /// - Inertia XHR (`X-Inertia: true`) → `409` + `X-Inertia-Location`,
    ///   which the client turns into `window.location = url`.
    /// - Anything else → a plain `302` + `Location`.
    ///
    /// Prefer this over [`location`](Self::location) in a handler. A hard
    /// navigation into an OAuth or SSO bounce carries no `X-Inertia`
    /// header, and a bare `409` with no `Location` gives that browser
    /// nowhere to go: the flow dead-ends on a blank page. Reach for
    /// [`location`](Self::location) only where the request is already
    /// known to be an Inertia visit.
    pub fn location_for<R: InertiaRequestExt + ?Sized>(
        req: &R,
        url: impl AsRef<str>,
    ) -> HttpResponse {
        if req.is_inertia() {
            Self::location(url)
        } else {
            HttpResponse::new()
                .status(302)
                .header("Location", url.as_ref())
        }
    }

    /// Build a `409 Conflict` Inertia-soft-redirect response. The client
    /// performs an Inertia SPA visit (not a full page navigation) to the
    /// target URL. The URL may include a `#fragment` which the client
    /// will land at after the visit. Counterpart to
    /// [`location`](Self::location) for the case where the redirect
    /// target is still inside the Inertia app.
    ///
    /// Maps to the Inertia v3 `X-Inertia-Redirect` protocol header.
    /// For standard server-side redirects (no fragment, plain
    /// post-form-submission) use [`Redirect::to`](crate::Redirect::to)
    /// instead — the auto-303 middleware will rewrite 302→303 for non-GET.
    pub fn redirect(url: impl AsRef<str>) -> HttpResponse {
        HttpResponse::new()
            .status(409)
            .header("X-Inertia-Redirect", url.as_ref())
    }

    /// Internal helper used by the `inertia_response!` macro to unfold a
    /// typed `Props` struct into individual eager props without re-serializing.
    ///
    /// Not part of the stable public API.
    #[doc(hidden)]
    pub fn __add_eager(&mut self, key: String, value: Value) {
        self.props.insert(key, Prop::eager(value));
    }

    /// Resolve the builder into an [`HttpResponse`] using request state.
    ///
    /// Async because Lazy / Optional / Defer / Merge / Once props may
    /// run DB queries or other futures inside their resolvers.
    ///
    /// - When the request has `X-Inertia: true`, returns the JSON page
    ///   object response (filtered for partial reloads, with all the
    ///   Tier 2 protocol fields populated).
    /// - Otherwise returns the HTML shell with the JSON page object
    ///   embedded in a sibling `<script type="application/json"
    ///   data-page="app">` element next to the empty `<div id="app">`
    ///   mount node — the Inertia 3 contract that `getInitialPageFromDOM`
    ///   reads.
    pub async fn resolve<R: InertiaRequestExt>(
        self,
        req: &R,
    ) -> Result<HttpResponse, FrameworkError> {
        let is_inertia_request = req.is_inertia();
        let filter = PartialFilter::build(req, &self.component);
        // Laravel gates the whole once-skip on `isInertia && !isPartial`
        // (`Response.php:307`). Honouring the client's "I already have
        // this cached" claim during an explicit partial reload means
        // `router.reload({ only: ['stats'] })` returns nothing at all for
        // the one key the user just asked for — the client asked BECAUSE
        // it wants a fresh value. A non-Inertia visit renders the page
        // from scratch and has no client cache to honour either.
        let except_once: Vec<String> = if is_inertia_request && !filter.matched {
            parse_csv_header(req, "X-Inertia-Except-Once-Props")
        } else {
            Vec::new()
        };
        // `X-Inertia-Reset` lists merge-prop keys the client wants to
        // start fresh from. We resolve their values normally (so the
        // client gets the current data) but omit the merge metadata so
        // the client treats the value as a replacement, not an append.
        // See `inertia-3.1.1/packages/core/src/requestParams.ts`: the
        // client puts reset keys into `only` AND `X-Inertia-Reset`, so
        // the partial filter already guarantees inclusion.
        let reset_keys: Vec<String> = parse_csv_header(req, "X-Inertia-Reset");
        // `X-Inertia-Error-Bag` scopes the `errors` prop under a named
        // bag, so multiple forms on a page can have isolated validation
        // errors. `errors: {}` becomes `errors: { bag_name: {} }`. When
        // validation parity wires real errors in, this is where they
        // get scoped.
        let error_bag: Option<String> = req
            .header("X-Inertia-Error-Bag")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        // `X-Inertia-Infinite-Scroll-Merge-Intent` tells the server
        // whether a follow-up infinite-scroll fetch wants the new chunk
        // appended or prepended to the existing accumulator; absent, it
        // defaults to append. It does not drive `reset` — that comes
        // from `X-Inertia-Reset` alone (`reset_keys` above), exactly
        // like a regular merge prop.
        let scroll_intent: Option<String> = req
            .header("X-Inertia-Infinite-Scroll-Merge-Intent")
            .map(|s| s.trim().to_lowercase())
            .filter(|s| s == "append" || s == "prepend");

        let Self {
            component,
            props,
            flash: response_flash,
            config,
            title,
            encrypt_history,
            clear_history,
            preserve_fragment,
            lazy_owned,
        } = self;

        // Page URL: path AND query, or the app's resolver. The client
        // writes this into `history.state`, so a bare path silently
        // resets pagination / sort / filter state on every back-forward
        // navigation. Laravel's `Response::getUrl` does the same, and
        // `InertiaVersionMiddleware` derives its `X-Inertia-Location`
        // from the same expression — so by default the two agree; a
        // `url_resolver` intentionally moves only this one, because the
        // 409 bounce has to name a URL the browser can actually fetch.
        let url = match config.url_resolver.as_ref() {
            Some(resolve_url) => resolve_url(req as &dyn InertiaRequestExt),
            None => req.path_and_query(),
        };

        // History-encryption precedence: per-response override (handler
        // wins) > middleware task_local > config default.
        let resolved_encrypt_history = encrypt_history
            .or_else(flash::encrypt_history_flag)
            .unwrap_or(config.encrypt_history_default);

        // preserve-fragment precedence: per-response override > session
        // flash (set by `Redirect::preserve_fragment()`) > false. The
        // session lookup is a no-op outside a `SessionMiddleware` scope.
        // `get_flash` removes the entry, so the flag is one-shot.
        let flashed_preserve_fragment =
            crate::session::session_mut(|s| s.get_flash::<bool>("_inertia.preserve_fragment"))
                .flatten()
                .unwrap_or(false);
        let resolved_preserve_fragment = preserve_fragment.unwrap_or(flashed_preserve_fragment);

        // clear-history precedence: per-response override OR the session
        // flash set by `App::clear_history()`. Either alone is enough —
        // unlike `preserve_fragment` there is no "force off" case, because
        // the only reason to ask for a history clear is that the previous
        // session must stop being readable. `get_flash` removes the entry,
        // so the flag survives exactly one hop; a flag that stuck around
        // would rotate the key on every navigation and defeat encrypted
        // history entirely. No-op outside a `SessionMiddleware` scope.
        let flashed_clear_history =
            crate::session::session_mut(|s| s.get_flash::<bool>("_inertia.clear_history"))
                .flatten()
                .unwrap_or(false);
        let resolved_clear_history = clear_history || flashed_clear_history;

        // Layer props in precedence order (later writes override earlier):
        //   1. Static shared registry  (App::inertia_share, App::inertia_share_lazy)
        //   2. Trait-registered shared data (InertiaSharedData::share)
        //   3. User-supplied props attached via the builder
        //
        // Track the union of (1) + (2) as `shared_keys` so the page
        // object can advertise them under `sharedProps` (the client
        // uses this for instant-swap during navigation — see
        // `inertia-3.1.1/packages/core/src/router.ts` `performInstantSwap`).
        let registry = App::inertia_registry();
        let mut merged: IndexMap<String, Prop> = IndexMap::new();
        let mut shared_keys: Vec<String> = Vec::new();
        for (k, v) in registry.snapshot_static()? {
            if !shared_keys.contains(&k) {
                shared_keys.push(k.clone());
            }
            merged.insert(k, v);
        }
        if let Some(provider) = registry.trait_provider()? {
            let trait_shared = provider.share(req, &component).await?;
            for (k, v) in trait_shared {
                if !shared_keys.contains(&k) {
                    shared_keys.push(k.clone());
                }
                merged.insert(k, v);
            }
        }
        for (k, v) in props {
            // Note: when user props override a shared key, we keep the
            // key in `shared_keys` per the Inertia v3 client contract —
            // the client reads the value from `props` (user's override)
            // and uses `sharedProps` only as a key list.
            merged.insert(k, v);
        }

        let (materialized, metadata) = resolve_props(
            merged,
            &filter,
            &except_once,
            &reset_keys,
            error_bag.as_deref(),
            scroll_intent.as_deref(),
            &lazy_owned,
            config.max_concurrent_resolvers,
            config.with_all_errors,
        )
        .await?;

        // Combine flash from three sources, in precedence order
        // (later writes override earlier so same-request entries win
        // over inherited cross-redirect entries):
        //   1. Session `_flash.old.*` — bridged from the previous
        //      request via `From<Redirect> for Response` then aged by
        //      `SessionMiddleware`.
        //   2. Task-local flash bag — same-request `App::flash`.
        //   3. Builder flash — same-request `InertiaResponse::flash`.
        let mut flash = flash::drain_session_flash_for_page();
        for (k, v) in flash::drain() {
            flash.insert(k, v);
        }
        for (k, v) in response_flash {
            flash.insert(k, v);
        }

        let page = build_page_object(
            &component,
            materialized,
            &config,
            url,
            &metadata,
            flash,
            resolved_encrypt_history,
            resolved_clear_history,
            resolved_preserve_fragment,
            shared_keys,
        );

        if is_inertia_request {
            Ok(build_json_response(&page))
        } else {
            // SSR runs only for HTML (non-XHR) visits. XHR is a JSON
            // page-object response and never needs prerender.
            let ssr_result = super::ssr::render(&config.ssr, req.path(), &page).await?;
            Ok(build_html_response(
                &page,
                &config,
                title.as_deref(),
                ssr_result.as_ref(),
            ))
        }
    }

    /// Build the page object without producing an HTTP response — used by
    /// tests that want to inspect the page object directly.
    #[cfg(test)]
    pub(crate) async fn build_page_object_for_test(
        self,
        url: String,
        filter: &PartialFilter,
    ) -> Value {
        let Self {
            component,
            props,
            flash: response_flash,
            config,
            title: _,
            encrypt_history,
            clear_history,
            preserve_fragment,
            lazy_owned,
        } = self;
        let (materialized, metadata) = resolve_props(
            props,
            filter,
            &[],
            &[],
            None,
            None,
            &lazy_owned,
            usize::MAX,
            config.with_all_errors,
        )
        .await
        .expect("test resolver should not fail");
        let resolved_encrypt_history = encrypt_history.unwrap_or(config.encrypt_history_default);
        // Test helper doesn't run inside a session scope by default,
        // so we never pick up a flashed flag here — only the explicit
        // override. Tests that DO drive a session scope via
        // `session_scope_for_test` pick up `_flash.old.*` via the
        // shared session-flash merge below.
        let resolved_preserve_fragment = preserve_fragment.unwrap_or(false);
        // The test helper does not exercise the shared-data registry.
        let shared_keys: Vec<String> = Vec::new();

        // Mirror the same three-tier flash precedence as `resolve`:
        // session-old < task-local < builder. Keeps the test helper
        // honest about the production drain path.
        let mut flash = flash::drain_session_flash_for_page();
        for (k, v) in flash::drain() {
            flash.insert(k, v);
        }
        for (k, v) in response_flash {
            flash.insert(k, v);
        }

        build_page_object(
            &component,
            materialized,
            &config,
            url,
            &metadata,
            flash,
            resolved_encrypt_history,
            clear_history,
            resolved_preserve_fragment,
            shared_keys,
        )
    }

    /// Build a `409 Conflict` response indicating an asset version mismatch.
    /// The client follows `X-Inertia-Location` for a fresh full-page visit.
    pub fn version_conflict(new_url: &str) -> HttpResponse {
        HttpResponse::new()
            .status(409)
            .header("X-Inertia-Location", new_url)
    }
}

/// Accumulator for Inertia v3 page-object metadata fields.
///
/// Each field corresponds to an optional top-level page-object property
/// — `deferredProps`, `rescuedProps`, `mergeProps`, etc. — and stays
/// empty when no props of that flavor are used in the response. The
/// `build_page_object` step only emits non-empty fields, so simple
/// responses keep their JSON small.
#[derive(Default)]
struct PageMetadata {
    deferred: IndexMap<String, Vec<String>>,
    rescued: Vec<String>,
    merge: Vec<String>,
    merge_prepend: Vec<String>,
    deep_merge: Vec<String>,
    match_props_on: Vec<String>,
    once: IndexMap<String, OnceMetadataEntry>,
    /// Infinite-scroll metadata: prop name → its `ScrollProp` payload
    /// (plus a `reset` flag read from `X-Inertia-Reset` membership).
    scroll: IndexMap<String, ScrollMetadataEntry>,
}

struct ScrollMetadataEntry {
    metadata: ScrollMetadata,
    /// `true` exactly when the client named this key in
    /// `X-Inertia-Reset`, so it should clear its accumulator before
    /// applying this response.
    reset: bool,
}

struct OnceMetadataEntry {
    /// The prop name (key in `props`). May differ from `cache_key`
    /// when the user supplied `OnceOptions::as_key`.
    prop_name: String,
    expires_at: Option<i64>,
}

/// Outcome of a single prop's async resolution.
///
/// Every metadata decision is made synchronously in `resolve_props`
/// before the resolver is even scheduled, so the only thing a completed
/// resolver still decides is whether its value lands in `props`.
enum TaskOutcome {
    Insert {
        key: String,
        value: Value,
    },
    /// Produced by `prop_lazy_with_owner` resolution when the field is not
    /// in the request's `?include=` set. The key is simply omitted from the
    /// response — no error, no null sentinel.
    Skip,
    Rescued {
        key: String,
    },
}

/// Parse a CSV header into a deduped list of trimmed, non-empty values.
fn parse_csv_header<R: InertiaRequestExt>(req: &R, name: &str) -> Vec<String> {
    req.header(name)
        .map(|raw| {
            raw.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Render each flashed bag's `{field: [messages]}` into the value shape
/// the Inertia client is typed against.
///
/// Laravel emits `$errors[0]` unless `$withAllErrors` is set
/// (`inertia-laravel-2.0.25/src/Middleware.php:196`), and Inertia's
/// `ErrorValue` is `string` by default (`inertia-3.6.1/packages/core/src/types.ts:59,100`)
/// — a bare string is what `useForm().errors.email` resolves to.
/// Emitting an array meant every page had to index `[0]`, which on a
/// string silently yields its first character.
///
/// Only session-flashed bags pass through here. An `errors` prop set by
/// a handler with `.with("errors", ...)` is never rewritten.
fn collapse_error_bags(
    bags: serde_json::Map<String, Value>,
    with_all_errors: bool,
) -> serde_json::Map<String, Value> {
    if with_all_errors {
        return bags;
    }
    bags.into_iter()
        .map(|(bag, fields)| {
            let collapsed = match fields {
                Value::Object(map) => Value::Object(
                    map.into_iter()
                        .map(|(field, messages)| {
                            // A non-array (or empty-array) value is left
                            // alone: it did not come from the canonical
                            // `with_errors` path, so there is no "first"
                            // message to pick.
                            let first = match messages {
                                Value::Array(mut items) if !items.is_empty() => items.remove(0),
                                other => other,
                            };
                            (field, first)
                        })
                        .collect(),
                ),
                other => other,
            };
            (bag, collapsed)
        })
        .collect()
}

/// Walk the prop bag, apply per-prop filtering / metadata rules, await
/// resolver closures concurrently, and return both the materialized prop
/// map and the page-object metadata.
///
/// Metadata and values are decided separately, on purpose. A prop's
/// merge, once, and deferred metadata is gated by the only/except lists
/// alone — Laravel computes each block from the unfiltered prop bag
/// (`inertia-laravel-2.0.25/src/Response.php:553-560`, `:725-736`) —
/// while whether the value itself ships goes through
/// [`PartialFilter::should_include`]. That split is what makes
/// `Prop::…defer().merge()` land its `deferredProps` entry on the first
/// visit and its `mergeProps` entry on both.
///
/// `reset_keys` is the `X-Inertia-Reset` list: merge-prop keys the
/// client wants to start fresh from. For those keys we resolve the
/// value normally but suppress the merge metadata, so the client
/// treats the value as a replacement rather than an append.
///
/// A scroll prop folds into that same merge protocol unconditionally —
/// unlike a plain merge prop, its direction defaults to append rather
/// than needing an explicit `.merge()` flag — and its per-key `reset`
/// flag is read straight from `reset_keys` too, independent of the
/// client's `X-Inertia-Infinite-Scroll-Merge-Intent` header.
#[allow(clippy::too_many_arguments)] // Internal helper; arguments group naturally as inputs.
async fn resolve_props(
    props: IndexMap<String, Prop>,
    filter: &PartialFilter,
    except_once: &[String],
    reset_keys: &[String],
    error_bag: Option<&str>,
    scroll_intent: Option<&str>,
    lazy_owned: &IndexMap<String, (&'static str, &'static str)>,
    max_concurrency: usize,
    with_all_errors: bool,
) -> Result<(serde_json::Map<String, Value>, PageMetadata), FrameworkError> {
    let mut materialized = serde_json::Map::new();
    let mut metadata = PageMetadata::default();

    // `errors` is always present per the Inertia v3 contract. Seed
    // with whatever the session has flashed under the canonical bag
    // keys (`errors.<bag>` — written by [`crate::http::Redirect::with_errors`]),
    // or an empty object when nothing flashed. The `X-Inertia-Error-Bag`
    // wrapping happens AFTER all props resolve — see the bottom of this
    // function. Doing it post-resolution means a handler that injects
    // errors via `.with("errors", {...})` still gets correctly scoped.
    //
    // The session lookup is bounded to a single bag prefix scan and is
    // a no-op outside a `SessionMiddleware` scope (silently produces
    // the empty object).
    // `pull_errors_flash` returns the raw `{bag: {field: [...]}}` map.
    // Resolve it to the Inertia shape, mirroring Laravel's
    // `resolveValidationErrors`:
    //  - `X-Inertia-Error-Bag` header → that bag's errors, flat; the
    //    post-pass below re-wraps them (and any handler-injected errors)
    //    under the bag name.
    //  - no header, `default` bag present → that bag's errors, flat
    //    (`{field: [...]}`) — what the Inertia client binds to directly
    //    (`page.props.errors.field`), not nested under `"default"`.
    //  - no header, no default bag → every bag, keyed by name.
    let session_errors: serde_json::Map<String, Value> =
        crate::session::session_mut(|s| s.pull_errors_flash()).unwrap_or_default();
    let session_errors = collapse_error_bags(session_errors, with_all_errors);
    let seeded_errors = match error_bag {
        Some(bag) => session_errors
            .get(bag)
            .cloned()
            .unwrap_or_else(|| Value::Object(serde_json::Map::new())),
        None => match session_errors.get("default") {
            Some(default_bag) => default_bag.clone(),
            None => Value::Object(session_errors),
        },
    };
    materialized.insert("errors".to_string(), seeded_errors);

    let mut tasks: Vec<TaskFuture> = Vec::new();
    let now_ms = chrono::Utc::now().timestamp_millis();

    for (key, prop) in props {
        // The absent sentinel (`when_loaded!` on an unloaded relation)
        // carries neither a value nor metadata. Checked first so flags
        // set on it cannot leak into the page object.
        if prop.is_absent() {
            continue;
        }

        // OWNER-TAGGED LAZY PATH (`#[derive(Data)]`)
        //
        // Gate order (spec):
        //   Stage 1 — resolve_with_owner: include-set membership check +
        //     per-DTO allowlist enforcement. Returns Err(400) when the
        //     requested field is not on the allowlist. This error MUST
        //     propagate before partial-data can silently swallow it.
        //   Stage 2 — partial-data filter: applied to the resolved
        //     Some(v) result as the final "only" gate, dot-notation and
        //     all — `X-Inertia-Partial-Data: user.name` narrows an
        //     owner-tagged field the same way it narrows an ordinary
        //     eager or lazy prop.
        //
        // Hoisted above every metadata block on purpose: `is_lazy()` is
        // false for any flagged prop, so an owner-tagged prop never owes
        // the page object metadata, and putting the partial-data filter
        // outside this branch is what used to drop disallowed-include
        // errors when X-Inertia-Partial-Data was narrower than ?include=.
        if prop.is_lazy()
            && let Some(&(owner, field)) = lazy_owned.get(&key)
        {
            let filter_clone = filter.clone();
            tasks.push(Box::pin(async move {
                match prop.resolve_with_owner(owner, field).await? {
                    None => Ok(TaskOutcome::Skip), // not in include set
                    Some(v) => {
                        // Stage 2: partial-data is the final "only" filter.
                        if filter_clone.should_include_eager(&key) {
                            let v = filter_clone.narrow(&key, v);
                            Ok(TaskOutcome::Insert { key, value: v })
                        } else {
                            Ok(TaskOutcome::Skip)
                        }
                    }
                }
            }));
            continue;
        }

        // The metadata gate. Laravel computes every metadata block from
        // the *unfiltered* prop bag and narrows it with the only/except
        // lists alone (`inertia-laravel-2.0.25/src/Response.php:553-560`,
        // `:725-736`), never asking whether the prop resolved. That is
        // what lets a deferred prop carry its merge instruction on the
        // very visit that withheld its value.
        let passes_lists = filter.should_include_eager(&key);

        // ---- once ----
        let mut client_has_cached = false;
        if prop.is_once() {
            let cache_key = prop.once_cache_key(&key);
            // Domain 20 audit D20-C: the server owns the expiry. Without
            // this a stale client can hold `X-Inertia-Except-Once-Props`
            // past the `until(...)` deadline and never see a fresh value.
            let server_expired = match prop.once_expires_at() {
                Some(ts) => now_ms >= ts,
                None => false,
            };
            client_has_cached =
                !prop.is_fresh() && !server_expired && except_once.iter().any(|k| k == &cache_key);
            if passes_lists {
                metadata.once.insert(
                    cache_key,
                    OnceMetadataEntry {
                        prop_name: key.clone(),
                        expires_at: prop.once_expires_at(),
                    },
                );
            }
        }

        // ---- merge ----
        //
        // A scroll prop derives its direction from the client's
        // merge-intent header below, so an explicit merge flag on the
        // same prop is ignored. `X-Inertia-Reset` names merge keys the
        // client wants to start fresh from: resolve the value normally
        // but drop the instruction, so the client replaces instead of
        // appending.
        if prop.scroll_metadata().is_none()
            && let Some(mode) = prop.merge_mode()
            && passes_lists
            && !reset_keys.iter().any(|k| k == &key)
        {
            for field in prop.match_on_fields() {
                metadata.match_props_on.push(format!("{key}.{field}"));
            }
            let paths = prop.merge_paths();
            match mode {
                // A prop merging at one or more nested paths never also
                // merges its whole value — Laravel's
                // `MergesProps::mergesAtRoot` (`MergesProps.php:126-129`)
                // turns root merging off the moment a path is named, so
                // the two are mutually exclusive per prop, never additive.
                MergeMode::Append if paths.is_empty() => metadata.merge.push(key.clone()),
                MergeMode::Append => {
                    for path in paths {
                        metadata.merge.push(format!("{key}.{path}"));
                    }
                }
                MergeMode::Prepend if paths.is_empty() => metadata.merge_prepend.push(key.clone()),
                MergeMode::Prepend => {
                    for path in paths {
                        metadata.merge_prepend.push(format!("{key}.{path}"));
                    }
                }
                // Deep merge already recurses into every nested field on
                // its own, so a path has nothing to narrow — Laravel
                // excludes deep-merge props from the root/path partition
                // entirely (`Response.php:590`, `:610`) and always emits
                // the bare key.
                MergeMode::Deep => metadata.deep_merge.push(key.clone()),
            }
        }

        // ---- deferred: announce instead of resolving ----
        if prop.is_defer() && !filter.should_include_optional(&key) {
            // A deferred prop the client already holds is not announced
            // again — otherwise it refetches on every navigation and
            // `once` buys nothing (`Response.php:653-673`).
            if !client_has_cached {
                metadata
                    .deferred
                    .entry(prop.defer_group().to_string())
                    .or_default()
                    .push(key);
            }
            continue;
        }

        // The client says it already holds this value: skip the
        // resolver. The `onceProps` entry emitted above still ships, and
        // the client fills the value back in from its own cache.
        if client_has_cached {
            continue;
        }

        if !filter.should_include(&key, &prop) {
            continue;
        }

        // ---- scroll ----
        //
        // Laravel folds every scroll prop into the merge protocol
        // unconditionally: `ScrollProp::configureMergeIntent` runs on
        // every resolution pass and always sets `merge = true`,
        // defaulting to append and switching to prepend only when the
        // client's intent header says so
        // (`inertia-laravel-2.0.25/src/ScrollProp.php:72-79`). `reset`
        // is decided independently, straight off `X-Inertia-Reset`
        // (`Response.php:700-716`) — not off the intent header, and a
        // reset key is excluded from the merge lists entirely, the
        // same exclusion a regular merge prop already gets.
        //
        // Gated on `passes_lists`, the same as the once/merge blocks
        // above — not on `filter.should_include(&key, &prop)`, which
        // already ran above this point to decide whether the *value*
        // resolves. Those two questions diverge for an `Always` prop:
        // `should_include` is unconditionally `true` for one (it
        // bypasses partial-reload filtering by design), but Laravel's
        // `resolveScrollProps`/`resolveMergeProps` still narrow by
        // `only`/`except` (`Response.php:553-560`, `:700-716`) — an
        // `Always` scroll prop outside the requested set must still
        // ship its value (so `should_include` is right to let it
        // through below) but must not also emit a merge instruction
        // for a key the client never fetched fresh rows for, or the
        // value that already shipped gets appended to on top of
        // itself. `inertia_prop_composition.rs`'s
        // `an_always_merge_prop_keeps_its_value_but_drops_merge_metadata_when_filtered_out`
        // pins the identical rule for a plain (non-scroll) merge prop.
        //
        // `match_on` folds in too, exactly the way Laravel's
        // `resolveMergeMatchingKeys` folds a `ScrollProp`'s
        // `matchesOn()` in alongside any other `Mergeable`
        // (`Response.php:558,641-652` — `getMergePropsForRequest`
        // gates only on `instanceof Mergeable && shouldMerge()`, no
        // scroll exclusion, and `resolveMergeMatchingKeys` doesn't
        // special-case `ScrollProp` either). This block keys every
        // match entry off the same `path` it already computes for the
        // merge/prepend push below: the bare key when unwrapped,
        // `key.wrap_key` when `.scroll_wrap(...)` is set. That is a
        // deliberate improvement over a byte-for-byte port of
        // Laravel's own wiring, not an accident of convenience:
        // `ScrollProp::configureMergeIntent` only ever calls the
        // single-argument `append($wrapper)` (`ScrollProp.php:72-79`),
        // never the two-argument `append($path, $matchOn)` overload
        // that prefixes `matchOn` with the path
        // (`MergesProps.php:136-151`) — so a wrapped Laravel
        // `ScrollProp` given a bare `matchOn('id')` emits an
        // unprefixed `"key.id"` match entry against a merge target of
        // `"key.wrapper"`, which the client's `mergeOrMatchItems`
        // prefix check (`inertia-3.6.1/.../response.ts:524-546`) can
        // never match — the entry is silently inert. T27's
        // `merge_with_path` has to leave that prefixing to the caller
        // because a prop can carry several paths at once and the
        // crate can't guess which one a given `match_on` field belongs
        // to (`spec-t27.md` design note 4). `.scroll_wrap` carries no
        // such ambiguity — it's a single `Option<String>`, the one
        // nesting point a scroll prop can have — so this block derives
        // the correct prefix itself instead of reproducing a match
        // that would silently never fire.
        //
        // `.deep_merge()` on a scroll prop is the one merge flag that
        // is NOT redundant: Laravel's `ScrollProp` constructor already
        // sets `$this->merge = true` (`ScrollProp.php:60`), so a
        // caller's own `->merge()`/`->prepend()` call has nothing left
        // to change — but `shouldDeepMerge()` routes the prop into
        // `resolveDeepMergeProps`, a completely different list, ahead
        // of the append/prepend computation
        // (`resolveAppendMergeProps`/`resolvePrependMergeProps` both
        // `reject(fn ($p) => $p->shouldDeepMerge())` first,
        // `Response.php:590,610`). A wrap key has nothing to narrow
        // under deep merge, same reasoning as the general merge
        // block's `MergeMode::Deep` arm above ignoring `merge_paths` —
        // deep merge already recurses through the entire value, so
        // this block deep-merges at the bare key even when
        // `.scroll_wrap(...)` is also set.
        if passes_lists && let Some(scroll_meta) = prop.scroll_metadata().cloned() {
            let is_reset = reset_keys.iter().any(|k| k == &key);
            if !is_reset {
                let is_deep = prop.merge_mode() == Some(MergeMode::Deep);
                let path = if is_deep {
                    key.clone()
                } else {
                    match prop.scroll_wrap_key() {
                        Some(wrap) => format!("{key}.{wrap}"),
                        None => key.clone(),
                    }
                };
                for field in prop.match_on_fields() {
                    metadata.match_props_on.push(format!("{path}.{field}"));
                }
                if is_deep {
                    metadata.deep_merge.push(path);
                } else {
                    match scroll_intent {
                        Some("prepend") => metadata.merge_prepend.push(path),
                        _ => metadata.merge.push(path),
                    }
                }
            }
            metadata.scroll.insert(
                key.clone(),
                ScrollMetadataEntry {
                    metadata: scroll_meta,
                    reset: is_reset,
                },
            );
        }

        // ---- value ----
        let rescue = prop.is_defer() && prop.rescues();
        // `Always` bypasses partial-reload filtering entirely — dot
        // notation included. Laravel re-injects the raw, unfiltered
        // `AlwaysProp` value after the only/except rebuild rather than
        // narrowing it (`inertia-laravel-2.0.25/src/Response.php:406-416`,
        // `resolveAlways`), so an always-visible prop must reach the
        // client whole even when the request's `X-Inertia-Partial-Data`
        // names a nested path inside it.
        let narrow_value = prop.visibility() != Visibility::Always;
        match prop.into_source() {
            // Unreachable: handled at the top of the loop. Listed so the
            // match stays exhaustive without a panic.
            PropSource::Absent => {}
            PropSource::Value(v) => {
                let v = if narrow_value {
                    filter.narrow(&key, v)
                } else {
                    v
                };
                materialized.insert(key, v);
            }
            PropSource::Resolver(resolver) if rescue => {
                let filter = filter.clone();
                tasks.push(Box::pin(async move {
                    match resolver().await {
                        Ok(v) => {
                            let v = if narrow_value {
                                filter.narrow(&key, v)
                            } else {
                                v
                            };
                            Ok(TaskOutcome::Insert { key, value: v })
                        }
                        Err(e) => {
                            tracing::warn!(
                                prop_key = %key,
                                error = %e,
                                "inertia deferred prop resolver failed; rescued per spec",
                            );
                            // Build the event on the current task so the
                            // REQUEST_ID task-local is in scope (a spawned
                            // task wouldn't inherit it). The dispatch
                            // itself is spawned per the documented
                            // ErrorOccurred best-effort contract — see
                            // `events/builtins.rs` and the matching
                            // pattern in `http/response.rs` — so we do
                            // not block the Inertia partial-response
                            // collector on listener execution.
                            let evt = crate::events::ErrorOccurred {
                                error_message: e.to_string(),
                                status_code: 500,
                                request_id: crate::logging::current_request_id()
                                    .map(|id| id.as_str().to_string()),
                            };
                            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                                handle.spawn(async move {
                                    let _ = crate::events::EventFacade::dispatch(evt).await;
                                });
                            }
                            Ok(TaskOutcome::Rescued { key })
                        }
                    }
                }));
            }
            PropSource::Resolver(resolver) => {
                let filter = filter.clone();
                tasks.push(Box::pin(async move {
                    let v = resolver().await?;
                    let v = if narrow_value {
                        filter.narrow(&key, v)
                    } else {
                        v
                    };
                    Ok(TaskOutcome::Insert { key, value: v })
                }));
            }
        }
    }

    // Domain 20 audit D20-E: bounded resolver fan-out. Without a cap a
    // page with N lazy props issues N parallel db/HTTP calls. The cap
    // is configurable via `InertiaConfig::max_concurrent_resolvers`;
    // `usize::MAX` (the explicit "no limit" sentinel set by the
    // builder for `max_concurrent_resolvers(0)`) disables it.
    //
    // `buffered` (vs. `buffer_unordered`) preserves input ordering in
    // the output stream — outcome order does not actually matter for
    // the materialized map / metadata population below, but stable
    // ordering keeps test snapshots predictable.
    use futures::stream::{self, StreamExt, TryStreamExt};
    let concurrency = max_concurrency.max(1);
    let outcomes: Vec<TaskOutcome> = stream::iter(tasks)
        .buffered(concurrency)
        .try_collect()
        .await?;

    for outcome in outcomes {
        match outcome {
            TaskOutcome::Insert { key, value } => {
                materialized.insert(key, value);
            }
            // Field was not in the request's `?include=` set — omit silently.
            TaskOutcome::Skip => {}
            TaskOutcome::Rescued { key } => {
                metadata.rescued.push(key);
            }
        }
    }

    // `X-Inertia-Error-Bag` scoping. Apply AFTER all props have
    // resolved so a handler-provided `errors` prop (via
    // `.with("errors", {...})`) gets correctly wrapped. Without this
    // post-pass, the seeded empty object would be wrapped here but
    // overwritten by the user prop, silently losing the bag.
    if let Some(bag) = error_bag
        && let Some(errors_val) = materialized.remove("errors")
    {
        let mut wrapper = serde_json::Map::new();
        wrapper.insert(bag.to_string(), errors_val);
        materialized.insert("errors".to_string(), Value::Object(wrapper));
    }

    // Dot-key nesting — Laravel's `Arr::set`-based `resolveArrayableProperties`
    // unpack step (`reference/inertia-laravel-2.0.25/src/Response.php:344-368`),
    // applied once to the fully resolved, fully filtered prop bag so it sees
    // exactly what's about to ship: eager, lazy, deferred-and-resolved,
    // merged, once, and scroll values alike, whether they came from the
    // response builder or the shared registry — both stored under their
    // literal (possibly dotted) key up to this point. A key with no dot
    // passes through as a plain insert. This never recurses into a prop's
    // *value* — the `errors` object above keeps whatever dotted validation
    // field names it carries internally; only top-level prop keys nest.
    let materialized = dotted::unpack_map(materialized);

    Ok((materialized, metadata))
}

#[allow(clippy::too_many_arguments)]
fn build_page_object(
    component: &str,
    materialized_props: serde_json::Map<String, Value>,
    config: &InertiaConfig,
    url: String,
    metadata: &PageMetadata,
    flash: serde_json::Map<String, Value>,
    encrypt_history: bool,
    clear_history: bool,
    preserve_fragment: bool,
    shared_keys: Vec<String>,
) -> Value {
    let mut page = serde_json::Map::new();
    page.insert(
        "component".to_string(),
        Value::String(component.to_string()),
    );
    page.insert("props".to_string(), Value::Object(materialized_props));
    page.insert("url".to_string(), Value::String(url));
    page.insert(
        "version".to_string(),
        Value::String(config.version.resolve()),
    );

    // Per spec, `encryptHistory` / `clearHistory` / `preserveFragment`
    // are only emitted when `true`. Falsy values are omitted to keep
    // the page object lean.
    if encrypt_history {
        page.insert("encryptHistory".to_string(), Value::Bool(true));
    }
    if clear_history {
        page.insert("clearHistory".to_string(), Value::Bool(true));
    }
    if preserve_fragment {
        page.insert("preserveFragment".to_string(), Value::Bool(true));
    }

    if !flash.is_empty() {
        page.insert("flash".to_string(), Value::Object(flash));
    }

    if !metadata.deferred.is_empty() {
        let deferred = metadata
            .deferred
            .iter()
            .map(|(group, keys)| {
                (
                    group.clone(),
                    Value::Array(keys.iter().cloned().map(Value::String).collect()),
                )
            })
            .collect::<serde_json::Map<_, _>>();
        page.insert("deferredProps".to_string(), Value::Object(deferred));
    }
    if !metadata.rescued.is_empty() {
        page.insert(
            "rescuedProps".to_string(),
            Value::Array(
                metadata
                    .rescued
                    .iter()
                    .cloned()
                    .map(Value::String)
                    .collect(),
            ),
        );
    }
    if !metadata.merge.is_empty() {
        page.insert(
            "mergeProps".to_string(),
            Value::Array(metadata.merge.iter().cloned().map(Value::String).collect()),
        );
    }
    if !metadata.merge_prepend.is_empty() {
        page.insert(
            "prependProps".to_string(),
            Value::Array(
                metadata
                    .merge_prepend
                    .iter()
                    .cloned()
                    .map(Value::String)
                    .collect(),
            ),
        );
    }
    if !metadata.deep_merge.is_empty() {
        page.insert(
            "deepMergeProps".to_string(),
            Value::Array(
                metadata
                    .deep_merge
                    .iter()
                    .cloned()
                    .map(Value::String)
                    .collect(),
            ),
        );
    }
    if !metadata.match_props_on.is_empty() {
        page.insert(
            "matchPropsOn".to_string(),
            Value::Array(
                metadata
                    .match_props_on
                    .iter()
                    .cloned()
                    .map(Value::String)
                    .collect(),
            ),
        );
    }
    if !metadata.once.is_empty() {
        let once = metadata
            .once
            .iter()
            .map(|(cache_key, entry)| {
                let mut m = serde_json::Map::new();
                m.insert("prop".to_string(), Value::String(entry.prop_name.clone()));
                m.insert(
                    "expiresAt".to_string(),
                    entry
                        .expires_at
                        .map(|t| Value::Number(serde_json::Number::from(t)))
                        .unwrap_or(Value::Null),
                );
                (cache_key.clone(), Value::Object(m))
            })
            .collect::<serde_json::Map<_, _>>();
        page.insert("onceProps".to_string(), Value::Object(once));
    }

    // `sharedProps` lists the keys that came from the shared registry
    // (static + trait). The client uses this during instant-swap visits
    // to carry shared values across navigations. Omit when empty so
    // small responses stay small.
    if !shared_keys.is_empty() {
        page.insert(
            "sharedProps".to_string(),
            Value::Array(shared_keys.into_iter().map(Value::String).collect()),
        );
    }

    // `scrollProps` carries infinite-scroll pagination metadata,
    // keyed by prop name. The `reset` flag is `true` exactly when the
    // client named this key in `X-Inertia-Reset`, telling the client to
    // clear its accumulator before applying this response instead of
    // folding it in as a follow-up next/previous fetch.
    if !metadata.scroll.is_empty() {
        let scroll = metadata
            .scroll
            .iter()
            .map(|(prop_key, entry)| {
                let mut m = serde_json::Map::new();
                m.insert(
                    "pageName".to_string(),
                    Value::String(entry.metadata.page_name.clone()),
                );
                m.insert(
                    "previousPage".to_string(),
                    entry.metadata.previous_page.clone().unwrap_or(Value::Null),
                );
                m.insert(
                    "nextPage".to_string(),
                    entry.metadata.next_page.clone().unwrap_or(Value::Null),
                );
                m.insert(
                    "currentPage".to_string(),
                    entry.metadata.current_page.clone().unwrap_or(Value::Null),
                );
                m.insert("reset".to_string(), Value::Bool(entry.reset));
                (prop_key.clone(), Value::Object(m))
            })
            .collect::<serde_json::Map<_, _>>();
        page.insert("scrollProps".to_string(), Value::Object(scroll));
    }

    Value::Object(page)
}

fn make_resolver<F, Fut, V>(resolver: F) -> PropResolver
where
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<V, FrameworkError>> + Send + 'static,
    V: Serialize + 'static,
{
    Arc::new(move || {
        let fut = resolver();
        Box::pin(async move {
            let value = fut.await?;
            serde_json::to_value(&value).map_err(|e| {
                FrameworkError::internal(format!(
                    "InertiaResponse resolver value failed to serialize: {}",
                    e
                ))
            })
        })
    })
}

/// Serialize an eager prop value to `Value`, panicking on `Serialize`
/// failure.
///
/// # Panics
///
/// Panics if `value`'s `Serialize` impl returns `Err`. This is a bug
/// in the value's type — a hand-written custom `Serialize`
/// implementation returning `Err` is the only path that triggers this.
///
/// The panic is caught by the request-level panic-recovery middleware
/// (`framework/src/middleware/chain.rs`, Domain 2 M1) and converted to
/// a 500 response, so the process stays up. To handle serialization
/// failure explicitly — required off the HTTP path, where no panic net
/// exists — use the fallible sibling of the builder method
/// ([`try_with`](InertiaResponse::try_with),
/// [`try_always`](InertiaResponse::try_always), etc.), which returns
/// `Result<Self, FrameworkError>`. [`InertiaResponse::lazy`] is an
/// alternative for values resolved asynchronously, though it also moves
/// the prop onto the partial-reload-gated lazy protocol.
fn to_value_or_die<V: Serialize>(value: &V) -> Value {
    serde_json::to_value(value).expect(
        "InertiaResponse prop value must serialize cleanly; check the type's Serialize impl",
    )
}

/// Fallible counterpart of [`to_value_or_die`]: serialize an eager prop
/// value to `Value`, returning a [`FrameworkError`] that names `key`
/// instead of panicking on `Serialize` failure. Backs the `try_*` builder
/// methods ([`InertiaResponse::try_with`] and siblings).
fn to_value_or_err<V: Serialize>(key: &str, value: &V) -> Result<Value, FrameworkError> {
    serde_json::to_value(value).map_err(|e| {
        FrameworkError::internal(format!(
            "InertiaResponse prop `{key}` failed to serialize: {e} \
             (the value's Serialize impl returned Err)"
        ))
    })
}

fn build_json_response(page: &Value) -> HttpResponse {
    HttpResponse::json(page.clone())
        .header("X-Inertia", "true")
        .header("Vary", "X-Inertia")
}

fn build_html_response(
    page: &Value,
    config: &InertiaConfig,
    title_override: Option<&str>,
    ssr: Option<&super::ssr::SsrResponse>,
) -> HttpResponse {
    let title = title_override.unwrap_or(&config.default_title);
    let csrf = csrf_token().unwrap_or_default();
    let csrf_attr = escape_html_attr(&csrf);
    let title_html = escape_html_text(title);

    let head_extras = if config.development {
        render_dev_head(config)
    } else {
        render_prod_head(config)
    };

    // Inertia 3 reads the initial page from a sibling
    // `<script type="application/json" data-page="app">` whose textContent
    // is the JSON envelope (see `@inertiajs/core` `getInitialPageFromDOM`).
    //
    // - SSR path: the worker's `body` is already wrapped by
    //   `buildSSRBody`, which emits the `<script>` + `<div
    //   data-server-rendered="true" id="app">` pair as one string. We
    //   inject it raw — wrapping it in another `<div id="app">` would
    //   produce duplicate IDs and break hydration.
    // - Non-SSR path: we emit the same shape ourselves with an empty
    //   mount div. Inside the script tag the JSON is raw (NOT
    //   HTML-attribute-encoded) and every `/` is backslash-escaped so a
    //   literal `</script>` substring inside a string field can't
    //   terminate the tag — this matches `buildSSRBody`'s escape.
    let ssr_head = ssr.map(|s| s.head.join("\n")).unwrap_or_default();
    let mount_block = if let Some(ssr) = ssr {
        ssr.body.clone()
    } else {
        let page_json = serde_json::to_string(page).unwrap_or_else(|_| "{}".to_string());
        let page_script = page_json.replace('/', "\\/");
        format!(
            "<script type=\"application/json\" data-page=\"app\">{page_script}</script>\n\
             <div id=\"app\"></div>",
        )
    };

    let html = format!(
        "<!DOCTYPE html>\n\
         <html lang=\"en\">\n\
         <head>\n\
         <meta charset=\"UTF-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\">\n\
         <meta name=\"csrf-token\" content=\"{csrf}\">\n\
         <title>{title}</title>\n\
         {ssr_head}\
         {head}\
         </head>\n\
         <body>\n\
         {mount_block}\n\
         </body>\n\
         </html>",
        csrf = csrf_attr,
        title = title_html,
        ssr_head = ssr_head,
        head = head_extras,
        mount_block = mount_block,
    );

    HttpResponse::html(html).header("Vary", "X-Inertia")
}

fn render_dev_head(config: &InertiaConfig) -> String {
    // HTML-escape vite_dev_server + entry_point before interpolation.
    // These are normally trusted config values, but a misconfigured
    // env / config file could otherwise break the shell or inject
    // markup into the dev-time HTML.
    //
    // For the React preamble, the same `vite_dev_server` value is used
    // inside a JS single-quoted string. We use `serde_json::to_string`
    // to produce a safe JS string literal (it produces a double-quoted
    // string that we re-wrap with the surrounding `'...'`-aware
    // shape).
    let server_attr = escape_html_attr(&config.vite_dev_server);
    let entry_attr = escape_html_attr(&config.entry_point);

    // React requires the `@react-refresh` preamble before any module loads;
    // Svelte and Vue have HMR built into their Vite plugins and don't need
    // any extra preamble script.
    let preamble = match config.frontend {
        Frontend::React => {
            // `serde_json::to_string` always produces a double-quoted JSON
            // literal (e.g. `"http://localhost:5173"`). Stripping the
            // surrounding `"` and wrapping with `'` keeps the existing
            // single-quote shape, while keeping all `\`/`'`/control-char
            // escapes that serde_json already applied.
            let js_server = serde_json::to_string(&config.vite_dev_server)
                .unwrap_or_else(|_| "\"\"".to_string());
            let js_server_inner = js_server.trim_matches('"');
            // Re-escape any embedded single quotes for the wrapping `'…'`.
            let js_server_safe = js_server_inner.replace('\'', "\\'");
            format!(
                "<script type=\"module\">\n\
                 import RefreshRuntime from '{js_server_safe}/@react-refresh'\n\
                 RefreshRuntime.injectIntoGlobalHook(window)\n\
                 window.$RefreshReg$ = () => {{}}\n\
                 window.$RefreshSig$ = () => (type) => type\n\
                 window.__vite_plugin_react_preamble_installed__ = true\n\
                 </script>\n"
            )
        }
        Frontend::Svelte | Frontend::Vue => String::new(),
    };

    format!(
        "{preamble}\
         <script type=\"module\" src=\"{server_attr}/@vite/client\"></script>\n\
         <script type=\"module\" src=\"{server_attr}/{entry_attr}\"></script>\n"
    )
}

fn render_prod_head(config: &InertiaConfig) -> String {
    // Resolve `entry_point` (e.g. `src/main.ts`) to the hashed output
    // files via Vite's manifest.json. When the manifest is missing or
    // doesn't contain the configured entry, fall back to the legacy
    // hardcoded `/{assets_base_url}/main.{js,css}` shape so apps
    // produced before the manifest layer keep booting. The fallback
    // path emits a tracing::warn! at first read inside
    // `InertiaConfig::vite_manifest`.
    let base = config.assets_base_url.trim_end_matches('/');
    let entry = &config.entry_point;
    if let Some(assets) = config.vite_manifest().and_then(|m| m.resolve_entry(entry)) {
        let mut out = String::new();
        for css in &assets.css {
            out.push_str(&format!(
                "<link rel=\"stylesheet\" href=\"{base}/{css}\">\n"
            ));
        }
        for js in &assets.js {
            out.push_str(&format!(
                "<script type=\"module\" src=\"{base}/{js}\"></script>\n"
            ));
        }
        for chunk in &assets.preload {
            out.push_str(&format!(
                "<link rel=\"modulepreload\" href=\"{base}/{chunk}\">\n"
            ));
        }
        out
    } else {
        // Manifest absent or entry not present — legacy fallback.
        format!(
            "<script type=\"module\" src=\"{base}/main.js\"></script>\n\
             <link rel=\"stylesheet\" href=\"{base}/main.css\">\n"
        )
    }
}

fn escape_html_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

fn escape_html_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn build_page_object_eager_only() {
        let resp = InertiaResponse::new("Home")
            .with("title", "Welcome")
            .with("count", 42u32);

        let filter = PartialFilter::default();
        let page = resp
            .build_page_object_for_test("/home".into(), &filter)
            .await;

        let obj = page.as_object().unwrap();
        assert_eq!(obj["component"], Value::String("Home".into()));
        assert_eq!(obj["url"], Value::String("/home".into()));
        assert_eq!(obj["version"], Value::String("1.0".into()));

        let props = obj["props"].as_object().unwrap();
        assert_eq!(props["title"], Value::String("Welcome".into()));
        assert_eq!(props["count"], Value::Number(42.into()));
        assert!(props["errors"].is_object());
    }

    #[tokio::test]
    async fn always_bypasses_filter() {
        let resp = InertiaResponse::new("Users")
            .with("users", json!([]))
            .always("flash", json!({"msg": "hi"}));

        let filter = PartialFilter {
            matched: true,
            only: Some(vec!["users".into()]),
            except: None,
        };
        let page = resp
            .build_page_object_for_test("/users".into(), &filter)
            .await;

        let props = page["props"].as_object().unwrap();
        assert!(props.contains_key("users"));
        assert!(props.contains_key("flash"));
    }

    #[tokio::test]
    async fn version_conflict_response_shape() {
        let r = InertiaResponse::version_conflict("/new-url");
        let hyper_resp = r.into_hyper();
        assert_eq!(hyper_resp.status(), 409);
        assert_eq!(
            hyper_resp.headers().get("X-Inertia-Location").unwrap(),
            "/new-url"
        );
    }

    #[test]
    fn html_escape_handles_critical_chars() {
        let attr = escape_html_attr(r#"a&b<c>d"e'f"#);
        assert_eq!(attr, "a&amp;b&lt;c&gt;d&quot;e&#x27;f");

        let text = escape_html_text("<script>");
        assert_eq!(text, "&lt;script&gt;");
    }

    #[test]
    fn dev_head_includes_react_preamble_for_react_only() {
        let cfg = InertiaConfig::new().frontend(Frontend::React);
        let head = render_dev_head(&cfg);
        assert!(head.contains("@react-refresh"));
        assert!(head.contains("__vite_plugin_react_preamble_installed__"));

        let cfg = InertiaConfig::new().frontend(Frontend::Svelte);
        let head = render_dev_head(&cfg);
        assert!(!head.contains("@react-refresh"));

        let cfg = InertiaConfig::new().frontend(Frontend::Vue);
        let head = render_dev_head(&cfg);
        assert!(!head.contains("@react-refresh"));
    }

    #[test]
    fn dev_head_loads_correct_entry_point_per_frontend() {
        let cfg = InertiaConfig::new().frontend(Frontend::Svelte);
        let head = render_dev_head(&cfg);
        assert!(head.contains("src/main.ts"));
        assert!(!head.contains("src/main.tsx"));

        let cfg = InertiaConfig::new().frontend(Frontend::React);
        let head = render_dev_head(&cfg);
        assert!(head.contains("src/main.tsx"));

        let cfg = InertiaConfig::new().frontend(Frontend::Vue);
        let head = render_dev_head(&cfg);
        assert!(head.contains("src/main.ts"));
    }

    #[tokio::test]
    async fn flash_emits_top_level_field() {
        let resp = InertiaResponse::new("Home").flash("toast", json!({"msg": "saved"}));
        let page = resp
            .build_page_object_for_test("/".into(), &PartialFilter::default())
            .await;
        let obj = page.as_object().unwrap();
        assert!(obj.contains_key("flash"));
        assert_eq!(obj["flash"]["toast"], json!({"msg": "saved"}));
    }

    #[tokio::test]
    async fn flash_field_absent_when_empty() {
        let resp = InertiaResponse::new("Home");
        let page = resp
            .build_page_object_for_test("/".into(), &PartialFilter::default())
            .await;
        let obj = page.as_object().unwrap();
        assert!(!obj.contains_key("flash"));
    }

    #[tokio::test]
    async fn defer_initial_visit_emits_deferred_props_no_resolve() {
        // Defer key NOT in partial-data → not resolved, emitted in
        // deferredProps under the default group.
        let resp = InertiaResponse::new("Users").defer("permissions", || async {
            // Should not run on initial visit. The Result type annotation
            // is required because Rust can't infer V from a never-resolved
            // future.
            #[allow(unreachable_code)]
            Ok::<Value, FrameworkError>({
                panic!("defer resolver should not run on initial visit");
            })
        });
        let page = resp
            .build_page_object_for_test("/".into(), &PartialFilter::default())
            .await;

        let obj = page.as_object().unwrap();
        assert!(obj["deferredProps"].is_object());
        let deferred = obj["deferredProps"].as_object().unwrap();
        let default_group = deferred["default"].as_array().unwrap();
        assert_eq!(default_group.len(), 1);
        assert_eq!(default_group[0], json!("permissions"));
        // And the prop is NOT in props.
        let props = obj["props"].as_object().unwrap();
        assert!(!props.contains_key("permissions"));
    }

    #[tokio::test]
    async fn merge_emits_merge_props_with_match_on() {
        let resp = InertiaResponse::new("Posts").merge_with(
            "posts",
            json!([{"id": 1}]),
            MergeStrategy::Append {
                match_on: Some("id".into()),
            },
        );
        let page = resp
            .build_page_object_for_test("/".into(), &PartialFilter::default())
            .await;

        let obj = page.as_object().unwrap();
        assert_eq!(obj["mergeProps"], json!(["posts"]));
        assert_eq!(obj["matchPropsOn"], json!(["posts.id"]));
        assert_eq!(obj["props"]["posts"], json!([{"id": 1}]));
    }

    #[tokio::test]
    async fn deep_merge_emits_deep_merge_props() {
        let resp = InertiaResponse::new("Chat").deep_merge("chat", json!({"messages": []}));
        let page = resp
            .build_page_object_for_test("/".into(), &PartialFilter::default())
            .await;

        let obj = page.as_object().unwrap();
        assert_eq!(obj["deepMergeProps"], json!(["chat"]));
    }

    #[tokio::test]
    async fn preserve_fragment_true_emits_flag() {
        let resp = InertiaResponse::new("Article").preserve_fragment(true);
        let page = resp
            .build_page_object_for_test("/article/new".into(), &PartialFilter::default())
            .await;
        let obj = page.as_object().unwrap();
        assert_eq!(obj["preserveFragment"], Value::Bool(true));
    }

    #[tokio::test]
    async fn preserve_fragment_default_omits_flag() {
        let resp = InertiaResponse::new("Article");
        let page = resp
            .build_page_object_for_test("/article".into(), &PartialFilter::default())
            .await;
        assert!(!page.as_object().unwrap().contains_key("preserveFragment"));
    }

    #[tokio::test]
    async fn preserve_fragment_false_omits_flag() {
        let resp = InertiaResponse::new("Article").preserve_fragment(false);
        let page = resp
            .build_page_object_for_test("/article".into(), &PartialFilter::default())
            .await;
        assert!(!page.as_object().unwrap().contains_key("preserveFragment"));
    }

    #[tokio::test]
    async fn redirect_response_shape() {
        let r = InertiaResponse::redirect("/articles/new#section");
        let hyper_resp = r.into_hyper();
        assert_eq!(hyper_resp.status(), 409);
        assert_eq!(
            hyper_resp.headers().get("X-Inertia-Redirect").unwrap(),
            "/articles/new#section"
        );
        // Distinct from `location`: must NOT carry X-Inertia-Location.
        assert!(hyper_resp.headers().get("X-Inertia-Location").is_none());
    }
}
