use super::config::{Frontend, InertiaConfig};
use super::dotted;
use super::flash;
use super::prop::ProvidesScrollMetadata;
use super::prop::{
    DeferOptions, InertiaRequestExt, MergeMode, MergeStrategy, OnceOptions, PartialFilter, Prop,
    PropResolver, PropSource, ScrollMetadata, Visibility,
};
use super::providers::{
    PropertyContext, ProvidesInertiaProperties, ProvidesInertiaProperty, RenderContext,
};
use crate::container::App;
use crate::csrf::csrf_token;
use crate::error::FrameworkError;
use crate::http::HttpResponse;
use indexmap::IndexMap;
use serde::Serialize;
use serde_json::Value;
use std::borrow::Cow;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Pinned boxed task future used when resolving lazy Inertia props.
type TaskFuture = Pin<Box<dyn Future<Output = Result<TaskOutcome, FrameworkError>> + Send>>;

/// A single prop entry returned by `#[derive(Data)]`'s `__into_inertia_props`.
///
/// - `Eager` - the field's value is already serialized; inserted directly
///   into the response prop bag.
/// - `LazyOwned` - standard lazy / `#[data(lazy)]` / `#[data(lazy(inertia))]`.
///   Must pass the `?include=` + allowlist gate before resolution.
/// - `DeferredOwned` - `#[data(lazy(deferred))]`. Same `?include=` gate as
///   `LazyOwned`; the variant tag signals Inertia deferred-props protocol to
///   the client (follow-up XHR). For v1, resolved via the same code path as
///   `LazyOwned`.
/// - `ClosureOwned` - `#[data(lazy(closure))]`. Same `?include=` gate for v1;
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
/// `__into_inertia_props` surface - users should not implement this
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
    /// Per-response override for big-integer markers. `None` defers to
    /// [`InertiaConfig::preserve_big_integers`]. Maps to Laravel's
    /// `Response::preserveBigIntegers($bool)`.
    preserve_big_integers: Option<bool>,
    /// Sidecar map for props registered via `prop_lazy_with_owner`.
    /// Maps the prop key to `(owner_struct_name, field_name)` so
    /// `resolve_props` can run `Prop::passes_include_gate` ahead of the
    /// ordinary resolution path for exactly these props, instead of the
    /// plain lazy path every other resolver-backed prop takes. Keyed by
    /// the same string as `props`.
    lazy_owned: IndexMap<String, (&'static str, &'static str)>,
    /// [`ProvidesInertiaProperties`] values given by
    /// [`provide`](Self::provide), expanded at render in this order.
    providers: Vec<Arc<dyn ProvidesInertiaProperties>>,
    /// Values for the root template only, never the page props. Maps to
    /// `Inertia::render(...)->withViewData(...)`.
    view_data: super::root_template::InertiaViewData,
    /// Whether the shared props join the page: the shared registry and the
    /// middleware hooks' `share` and `share_once`. Always on, except for an
    /// error page the application's error callback rendered without
    /// `with_shared_data()` (PAR-062).
    shared_data: bool,
    /// Where in the application's code the response was built, which
    /// Inertia DevTools shows as the render source: the caller of
    /// [`new`](Self::new), through `#[track_caller]`, or the route
    /// definition of a `Router::inertia` page.
    render_source: &'static std::panic::Location<'static>,
}

/// Request-scoped snapshot of session values that an Inertia response delivers once.
///
/// `SessionMiddleware` ages `_flash.new.*` into `_flash.old.*` before the
/// handler runs. Merely peeking at those old values is therefore insufficient:
/// when response construction fails, the next request's aging pass would
/// delete them. This guard selectively reflashes the values on every
/// uncommitted exit, including cancellation, and removes them only after the
/// complete response has been built.
///
/// Three kinds of entry are staged: the aged one-shot values (`_flash.old.*`,
/// validation bags and the Inertia flash data among them), the Inertia flash
/// data this request wrote (`_flash.new.inertia.flash_data`), and the two
/// history flags, which are plain session entries that last until a page
/// emits them. The page pulls all three; only the aged values need moving
/// back when it fails, since the others stay where they are.
struct StagedInertiaSessionValues {
    entries: Vec<(String, Value)>,
    committed: bool,
}

impl StagedInertiaSessionValues {
    const OLD_PREFIX: &'static str = "_flash.old.";
    const NEW_PREFIX: &'static str = "_flash.new.";
    /// The flashed preserve-fragment flag an earlier release wrote. Read
    /// beside [`flash::PRESERVE_FRAGMENT`] so a session that holds it across
    /// an upgrade still delivers it.
    const LEGACY_PRESERVE_FRAGMENT: &'static str = "_inertia.preserve_fragment";
    /// The flashed clear-history flag an earlier release wrote, read beside
    /// [`flash::CLEAR_HISTORY`] for the same reason: a logout in flight
    /// across an upgrade must still clear the history.
    const LEGACY_CLEAR_HISTORY: &'static str = "_inertia.clear_history";

    fn stage() -> Self {
        let new_flash_data = flash::flash_data_new_key();
        let entries = crate::session::session()
            .map(|session| {
                session
                    .data
                    .iter()
                    .filter_map(|(key, value)| {
                        let staged = match key.strip_prefix(Self::OLD_PREFIX) {
                            Some(name) => Self::is_inertia_value(name),
                            None => {
                                *key == new_flash_data
                                    || key == flash::CLEAR_HISTORY
                                    || key == flash::PRESERVE_FRAGMENT
                            }
                        };
                        staged.then(|| (key.clone(), value.clone()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self {
            entries,
            committed: false,
        }
    }

    fn is_inertia_value(name: &str) -> bool {
        !name.starts_with('_')
            || name == Self::LEGACY_PRESERVE_FRAGMENT
            || name == Self::LEGACY_CLEAR_HISTORY
    }

    fn value_at(&self, full_key: &str) -> Option<&Value> {
        self.entries
            .iter()
            .find_map(|(key, value)| (key == full_key).then_some(value))
    }

    /// Whether a history flag is pending: the session entry Laravel's key
    /// names, or the flash an earlier release wrote.
    fn flag(&self, key: &str, legacy: &str) -> bool {
        let legacy_key = format!("{}{legacy}", Self::OLD_PREFIX);
        [key, legacy_key.as_str()]
            .iter()
            .any(|key| self.value_at(key).and_then(Value::as_bool) == Some(true))
    }

    /// The pending clear-history flag, emitted as `clearHistory: true`.
    fn clear_history(&self) -> bool {
        self.flag(flash::CLEAR_HISTORY, Self::LEGACY_CLEAR_HISTORY)
    }

    /// The pending preserve-fragment flag, emitted as `preserveFragment: true`.
    fn preserve_fragment(&self) -> bool {
        self.flag(flash::PRESERVE_FRAGMENT, Self::LEGACY_PRESERVE_FRAGMENT)
    }

    fn error_bags(&self) -> serde_json::Map<String, Value> {
        let prefix = format!("{}errors.", Self::OLD_PREFIX);
        self.entries
            .iter()
            .filter_map(|(key, value)| {
                key.strip_prefix(&prefix)
                    .map(|bag| (bag.to_string(), value.clone()))
            })
            .collect()
    }

    /// The session's part of `page.flash`: the plain session flashes the
    /// previous request left, then the Inertia flash data, each only while
    /// the session still holds what was staged (a handler that pulled the
    /// flash data before the render sends none).
    fn page_flash(&self) -> serde_json::Map<String, Value> {
        let visible = flash::drain_session_flash_for_page();
        let mut out: serde_json::Map<String, Value> = self
            .entries
            .iter()
            .filter_map(|(key, value)| {
                let name = key.strip_prefix(Self::OLD_PREFIX)?;
                (!name.starts_with('_')
                    && !name.starts_with("errors.")
                    && name != flash::FLASH_DATA
                    && visible.get(name) == Some(value))
                .then(|| (name.to_string(), value.clone()))
            })
            .collect();
        let held = crate::session::session();
        for key in [flash::flash_data_old_key(), flash::flash_data_new_key()] {
            if let Some(Value::Object(data)) = self.value_at(&key)
                && held.as_ref().and_then(|session| session.data.get(&key)) == self.value_at(&key)
            {
                out.extend(data.iter().map(|(k, v)| (k.clone(), v.clone())));
            }
        }
        out
    }

    fn commit(mut self) {
        crate::session::session_mut(|session| {
            for (key, staged_value) in &self.entries {
                if session.data.get(key) == Some(staged_value) {
                    session.data.remove(key);
                    session.dirty = true;
                }
            }
        });
        self.committed = true;
    }

    fn rollback(mut self) {
        self.reflash();
        self.committed = true;
    }

    fn reflash(&self) {
        crate::session::session_mut(|session| {
            for (old_key, staged_value) in &self.entries {
                if session.data.get(old_key) != Some(staged_value) {
                    continue;
                }
                let Some(name) = old_key.strip_prefix(Self::OLD_PREFIX) else {
                    continue;
                };
                session.data.remove(old_key);
                let new_key = format!("{}{name}", Self::NEW_PREFIX);
                match (session.data.get_mut(&new_key), staged_value) {
                    // Inertia flash data this request added to: keep both,
                    // what it added winning.
                    (Some(Value::Object(newer)), Value::Object(older))
                        if name == flash::FLASH_DATA =>
                    {
                        for (k, v) in older {
                            newer.entry(k.clone()).or_insert_with(|| v.clone());
                        }
                    }
                    (Some(_), _) => {}
                    (None, _) => {
                        session.data.insert(new_key, staged_value.clone());
                    }
                }
                session.dirty = true;
            }
        });
    }
}

impl Drop for StagedInertiaSessionValues {
    fn drop(&mut self) {
        if !self.committed {
            self.reflash();
        }
    }
}

/// Preserve aged Inertia session values when eager response construction
/// fails before [`InertiaResponse::resolve`] establishes its staging guard.
pub(super) fn reflash_session_values_after_eager_error(error: FrameworkError) -> FrameworkError {
    StagedInertiaSessionValues::stage().rollback();
    error
}

impl InertiaResponse {
    /// Begin a new Inertia response for the given page component.
    ///
    /// The response starts from the config the app passed to
    /// [`crate::Inertia::install`], and falls back to
    /// [`InertiaConfig::default`] when nothing was installed, so an app or
    /// a test that never calls `install` needs no config of its own.
    /// Override for one response with [`with_config`](Self::with_config).
    #[track_caller]
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
            preserve_big_integers: None,
            lazy_owned: IndexMap::new(),
            providers: Vec::new(),
            view_data: super::root_template::InertiaViewData::default(),
            shared_data: true,
            render_source: std::panic::Location::caller(),
        }
    }

    /// Name `location` as where this response was rendered, for a page
    /// whose render call is the framework's own: a `Router::inertia`
    /// route is rendered where the route was defined.
    pub(crate) fn with_render_source(
        mut self,
        location: &'static std::panic::Location<'static>,
    ) -> Self {
        self.render_source = location;
        self
    }

    /// Leave the shared props out of this page: the shared registry and the
    /// middleware hooks' `share` and `share_once`. For an error page the
    /// application's error callback renders without
    /// [`with_shared_data`](crate::InertiaErrorResponse::with_shared_data),
    /// as Laravel's `ExceptionResponse` leaves them out.
    pub(crate) fn without_shared_data(mut self) -> Self {
        self.shared_data = false;
        self
    }

    /// Override the default [`InertiaConfig`] for this response.
    ///
    /// Replaces the config wholesale, `version` included.
    /// [`InertiaVersionMiddleware`](crate::InertiaVersionMiddleware) still
    /// resolves the version [`Inertia::install`](crate::Inertia::install)
    /// was given, so a config here that doesn't carry the same
    /// `.version(...)` makes the page object advertise a version the
    /// middleware will bounce - the client takes one extra full page load
    /// after visiting that page. Set `.version(...)` on the override to
    /// match.
    pub fn with_config(mut self, config: InertiaConfig) -> Self {
        self.config = config;
        self
    }

    /// Set the `<title>` for the HTML shell on this response.
    ///
    /// On Inertia XHR responses the title is ignored - `<Head>` on the
    /// client manages document title for SPA visits. The configured title
    /// is only used for the initial HTML render.
    ///
    /// Under [SSR](InertiaConfig::ssr) it may not reach the document at
    /// all: when the worker's head carries a `<title>` - which it does for
    /// every page rendering one through Inertia's `Head` component - the
    /// page's own title is the document's only one, and this value and
    /// [`InertiaConfig::default_title`] are both left out. A document with
    /// two titles shows the first, so the framework's would win over the
    /// page's real one; set the title in `Head` rather than here when SSR
    /// is on.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Hand the root template a value under `key` for this response,
    /// replacing an earlier one; Laravel's `withViewData`.
    ///
    /// The value reaches only the application's
    /// [root template](crate::InertiaConfig::root_template), which reads it
    /// with `view.get("key")`, and never the page props: an Inertia visit's
    /// JSON and the first visit's page data leave it out. Use it for what
    /// the first-load HTML must carry without running JavaScript, such as
    /// the meta tags a link preview reads. The framework's own document
    /// places none.
    ///
    /// Any serializable value: placed with `{{ value }}`, a string displays
    /// as itself and any other value as its JSON. A `Serialize` impl that
    /// fails panics, as [`with`](Self::with) does;
    /// [`try_with_view_data`](Self::try_with_view_data) returns the error.
    pub fn with_view_data<V: Serialize>(mut self, key: impl Into<String>, value: V) -> Self {
        let value = to_value_or_die(&value);
        self.view_data.insert(key.into(), value);
        self
    }

    /// Fallible sibling of [`with_view_data`](Self::with_view_data): returns
    /// an error naming `key` when the value's `Serialize` impl fails.
    pub fn try_with_view_data<V: Serialize>(
        mut self,
        key: impl Into<String>,
        value: V,
    ) -> Result<Self, FrameworkError> {
        let key = key.into();
        let value = serde_json::to_value(&value).map_err(|e| {
            reflash_session_values_after_eager_error(FrameworkError::internal(format!(
                "InertiaResponse view data `{key}` failed to serialize: {e} \
                 (the value's Serialize impl returned Err)"
            )))
        })?;
        self.view_data.insert(key, value);
        Ok(self)
    }

    /// Register `prop` under `key`, replacing any earlier prop there.
    ///
    /// Every builder method goes through here. A replacement drops the
    /// earlier prop's `#[derive(Data)]` include gate with it: the gate
    /// belongs to the prop `prop_lazy_with_owner` registered, not to the
    /// key, so a plain prop put under the same key is sent like any other.
    fn put_prop(&mut self, key: impl Into<String>, prop: Prop) {
        let key = key.into();
        self.lazy_owned.shift_remove(&key);
        self.props.insert(key, prop);
    }

    /// Attach an eager prop. Honors partial-reload filtering per the v3
    /// protocol - when the client sends `X-Inertia-Partial-Data` matching
    /// the same component, this key is included only if it's in that list
    /// (and not in `X-Inertia-Partial-Except`).
    pub fn with<V: Serialize>(mut self, key: impl Into<String>, value: V) -> Self {
        let v = to_value_or_die(&value);
        self.put_prop(key.into(), Prop::eager(v));
        self
    }

    /// Attach an always-included prop. Bypasses partial-reload filtering -
    /// always returned in the response, even when the client requested a
    /// narrower set. Maps to Laravel's `Inertia::always($value)`.
    pub fn always<V: Serialize>(mut self, key: impl Into<String>, value: V) -> Self {
        let v = to_value_or_die(&value);
        self.put_prop(key.into(), Prop::eager(v).always());
        self
    }

    /// Attach an always-included prop backed by an async resolver - the
    /// resolver sibling of [`always`](Self::always). Maps to Laravel's
    /// `Inertia::always(fn () => ...)`: `AlwaysProp` accepts any value,
    /// closures included (`AlwaysProp.php`), and Suprnova splits that into
    /// two methods the way it already splits `.with`/`.lazy` and
    /// `.once`/`.once_with`. Reach for this when the always-included
    /// value is worth computing lazily - a DB read, an HTTP call - not
    /// when you already have the value in hand (`.always` covers that).
    pub fn always_with<F, Fut, V>(mut self, key: impl Into<String>, resolver: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<V, FrameworkError>> + Send + 'static,
        V: Serialize + 'static,
    {
        let resolver = make_resolver(resolver);
        self.put_prop(key.into(), Prop::from_resolver(resolver).always());
        self
    }

    /// Attach a lazy prop. The async closure runs only when the prop will
    /// actually be sent to the client - typically once on the initial visit
    /// or when explicitly requested via `X-Inertia-Partial-Data`. Maps to
    /// Laravel's `fn () => ...` prop pattern.
    ///
    /// Despite the name, this is **not** Laravel's `Inertia::lazy()` -
    /// that method is deprecated and behaves like `optional()` (skipped
    /// entirely on the initial visit; `LazyProp` is a straight alias for
    /// `OptionalProp`, `ResponseFactory.php:174-181`). Suprnova's `.lazy`
    /// is the plain-closure convention Laravel itself uses for a callable
    /// prop with no wrapper at all - included whenever the key passes
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
        self.put_prop(key.into(), Prop::from_resolver(resolver));
        self
    }

    /// Attach a lazy prop owned by a `#[derive(Data)]` DTO.
    ///
    /// The prop key is `field` (they are always identical in the DTO
    /// pattern). `resolve_props` looks the key up in the sidecar map this
    /// method populates and runs `Prop::passes_include_gate(owner, field)` -
    /// which consults the `RequestIncludeSet` task-local - ahead of
    /// every other block: the closure runs only when `field` appears in
    /// `?include=` AND is on the DTO's allowlist. Returns `400` to the
    /// client if the include set asks for a field not in the allowlist,
    /// before partial-data filtering gets a chance to swallow that error.
    ///
    /// Composition with `X-Inertia-Partial-Data`: once the include-set
    /// gate above passes, the prop reaches the same partial-reload check
    /// every other prop does (`PartialFilter::should_include`, which
    /// dispatches to `should_include_eager` for a plain lazy field or
    /// `should_include_optional` for a deferred one) - synchronously,
    /// before the resolver closure is ever invoked. So a field must pass
    /// both gates to be resolved and returned; failing either skips the
    /// closure entirely rather than running it and discarding the result.
    pub fn prop_lazy_with_owner(
        mut self,
        owner_struct_name: &'static str,
        field: &'static str,
        prop: Prop,
    ) -> Self {
        self.put_prop(field.to_string(), prop);
        self.lazy_owned
            .insert(field.to_string(), (owner_struct_name, field));
        self
    }

    /// Attach a fully composed [`Prop`] under `key`.
    ///
    /// The other builder methods each set one flag. This is how you set
    /// more than one - a deferred prop that also merges, a merge prop the
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
        self.put_prop(key.into(), prop);
        self
    }

    /// Attach a value that converts itself when it is sent, with its key
    /// path, its sibling props and the request - Laravel's
    /// `ProvidesInertiaProperty` as a prop value. Shorthand for
    /// `.prop(key, Prop::property(value))`.
    pub fn with_property(
        self,
        key: impl Into<String>,
        value: impl ProvidesInertiaProperty + 'static,
    ) -> Self {
        self.prop(key, Prop::property(value))
    }

    /// Build an `InertiaResponse` from the `Vec<(String, PropEntry)>` produced
    /// by a `#[derive(Data)]` DTO's `__into_inertia_props`.
    ///
    /// Dispatches on each entry variant:
    /// - `Eager` → inserted directly via the internal prop map (equivalent to `.with(key, value)`).
    /// - `LazyOwned` → routed through `prop_lazy_with_owner` so the
    ///   `?include=` + allowlist gate applies at resolution time.
    #[track_caller]
    pub fn from_data_props(component: &'static str, props: Vec<(String, PropEntry)>) -> Self {
        let mut r = Self::new(component);
        r.put_data_props(props);
        r
    }

    /// Add a `#[derive(Data)]` object's props to this response, any number
    /// of them - Laravel's page props taking several Data objects
    /// (PAR-051). Lazy fields keep the `?include=` and allowlist gate they
    /// have under [`Inertia::data`](crate::Inertia::data); a later prop
    /// under the same key replaces an earlier one.
    ///
    /// Panics where [`Inertia::data`](crate::Inertia::data) does, on a
    /// field whose `Serialize` impl fails; the request's panic boundary
    /// turns that into a 500. Use [`try_with_data`](Self::try_with_data)
    /// to handle it instead.
    pub fn with_data<T: IntoInertiaData>(mut self, data: T) -> Self {
        self.put_data_props(data.__into_inertia_props());
        self
    }

    /// Fallible sibling of [`with_data`](Self::with_data): returns
    /// `Err(FrameworkError)` naming the field whose `Serialize` impl
    /// failed instead of panicking.
    pub fn try_with_data<T: IntoInertiaData>(mut self, data: T) -> Result<Self, FrameworkError> {
        let props = data
            .__try_into_inertia_props()
            .map_err(reflash_session_values_after_eager_error)?;
        self.put_data_props(props);
        Ok(self)
    }

    /// Expand a [`ProvidesInertiaProperties`] value into this page's props
    /// at render, with the page's [`RenderContext`] - Laravel's provider in
    /// `Inertia::render($component, [$provider, ...])`.
    ///
    /// Give a page any number of them: their props merge in the order they
    /// were given, a later provider winning over an earlier one, and the
    /// page's own props (`.with`, `.prop` and the rest) win over every
    /// provider's, whatever the call order. A provider's props win over
    /// the shared props.
    pub fn provide(mut self, provider: impl ProvidesInertiaProperties + 'static) -> Self {
        self.providers.push(Arc::new(provider));
        self
    }

    /// Register the props a `#[derive(Data)]` object produced, routing its
    /// owner-tagged lazy fields through the include gate.
    fn put_data_props(&mut self, props: Vec<(String, PropEntry)>) {
        for (k, entry) in props {
            match entry {
                PropEntry::Eager(v) => {
                    self.put_prop(k, Prop::eager(v));
                }
                PropEntry::LazyOwned { owner, field, prop }
                | PropEntry::DeferredOwned { owner, field, prop }
                | PropEntry::ClosureOwned { owner, field, prop } => {
                    self.put_prop(k, prop);
                    self.lazy_owned.insert(field.to_string(), (owner, field));
                }
            }
        }
    }

    /// Attach an optional prop. Never included on standard visits; on a
    /// matching partial reload, included whenever its key passes the
    /// only/except lists. Maps to `Inertia::optional(...)`.
    pub fn optional<F, Fut, V>(mut self, key: impl Into<String>, resolver: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<V, FrameworkError>> + Send + 'static,
        V: Serialize + 'static,
    {
        let resolver = make_resolver(resolver);
        self.put_prop(key.into(), Prop::from_resolver(resolver).optional());
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
        self.put_prop(key.into(), prop);
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
        self.put_prop(key.into(), Prop::eager(v).merge_strategy(strategy));
        self
    }

    /// Attach a mergeable prop whose value comes from an async resolver
    /// instead of being materialized eagerly - append strategy, no
    /// `match_on`. The resolver sibling of [`InertiaResponse::merge`].
    /// Maps to `Inertia::merge(fn () => ...)` (`MergeProp` resolves a
    /// `Closure` value via `ResolvesCallables`,
    /// `inertia-laravel-2.0.25/src/MergeProp.php:24-29`).
    ///
    /// The resolver runs only when the merge prop will actually be sent -
    /// skipped by partial-reload filtering and by [`Prop::defer`] like
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
        self.put_prop(
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
    /// ([`OnceOptions::once`](crate::OnceOptions::once),
    /// [`OnceOptions::until`](crate::OnceOptions::until),
    /// [`OnceOptions::as_key`](crate::OnceOptions::as_key),
    /// [`OnceOptions::fresh`](crate::OnceOptions::fresh)) - Laravel's
    /// `Inertia::once(fn () => ...)->once($value, $as, $until)`.
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
        let prop = Prop::from_resolver(resolver).once_with(options);
        self.put_prop(key.into(), prop);
        self
    }

    /// Attach an infinite-scroll prop with an eager value. The
    /// framework normalizes the data shape: the value lands in `props`
    /// and the pagination metadata is emitted under `scrollProps`. The
    /// client's `<InfiniteScroll>` component reads both to drive
    /// next/previous fetches.
    ///
    /// A scroll prop always carries merge metadata - unlike a plain
    /// merge prop, it needs no explicit `.merge()` - defaulting to
    /// append and switching to prepend only when the client sends
    /// `X-Inertia-Infinite-Scroll-Merge-Intent: prepend`. This matches
    /// `ScrollProp::configureMergeIntent`
    /// (`inertia-laravel-2.0.25/src/ScrollProp.php:72-79`), which runs
    /// unconditionally on every response, fresh visits included.
    ///
    /// `scrollProps[key].reset` is `true` exactly when the client named
    /// `key` in `X-Inertia-Reset` - the same header a regular merge prop
    /// reads, and independent of the merge-intent header above
    /// (`Response.php:700-716`). A reset key is also excluded from
    /// `mergeProps` / `prependProps` for that response, so the client
    /// treats the value as a replacement instead of an append.
    ///
    /// Merges under the wrapper `data` - `key.data` - as Laravel's
    /// `Inertia::scroll($value, $wrapper = 'data')` does, since a
    /// paginator or resource serializes its rows there. Reach for
    /// [`scroll_wrapped`](Self::scroll_wrapped) to name another wrapper,
    /// or for [`paginate`](Self::paginate), which ships bare rows and
    /// merges at the prop's root.
    ///
    /// Maps to Laravel's `Inertia::scroll(...)`.
    pub fn scroll<V: Serialize>(
        self,
        key: impl Into<String>,
        metadata: impl ProvidesScrollMetadata,
        value: V,
    ) -> Self {
        let v = to_value_or_die(&value);
        self.attach_scroll(key.into(), None, metadata, Prop::eager(v))
    }

    /// Attach an infinite-scroll prop whose value is produced by an
    /// async resolver. Useful when the paginated data requires a DB
    /// query or other async work - common for real scroll loaders.
    pub fn scroll_with<F, Fut, V>(
        self,
        key: impl Into<String>,
        metadata: impl ProvidesScrollMetadata,
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

    /// Attach an infinite-scroll prop whose merge instruction targets the
    /// named field of the value - `key.wrap_key` - instead of the default
    /// `key.data` of [`scroll`](Self::scroll). Use this when the value is
    /// an envelope whose list sits under another field
    /// (`{ items: [...], meta: {...} }`), so only that list folds into
    /// what the client already holds.
    ///
    /// Equivalent to
    /// `Prop::eager(value).scroll(metadata).scroll_wrap(wrap_key)`
    /// attached under `key`.
    pub fn scroll_wrapped<V: Serialize>(
        self,
        key: impl Into<String>,
        wrap_key: impl Into<String>,
        metadata: impl ProvidesScrollMetadata,
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
        metadata: impl ProvidesScrollMetadata,
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

    /// Attach an infinite-scroll prop loaded by `resolver` whose value
    /// describes its own pages - a paginator, typically. Laravel's
    /// `Inertia::scroll(fn () => User::paginate())`, where the metadata is
    /// read from the loaded paginator.
    ///
    /// The value is serialized whole and merges under `key.data`, where a
    /// paginator keeps its rows. The `scrollProps` entry ships with the
    /// value, so a visit that withholds the value ships none. See
    /// [`Prop::scroll_lazy`].
    pub fn scroll_lazy<F, Fut, V>(self, key: impl Into<String>, resolver: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<V, FrameworkError>> + Send + 'static,
        V: Serialize + ProvidesScrollMetadata + 'static,
    {
        self.prop(
            key,
            Prop::scroll_lazy(resolver, |value: &V| value.scroll_metadata()),
        )
    }

    /// Attach an infinite-scroll prop loaded by `resolver` whose page facts
    /// `metadata` builds from the loaded value - Laravel's
    /// `Inertia::scroll($value, 'data', fn ($value) => ...)`. See
    /// [`Prop::scroll_lazy`].
    pub fn scroll_lazy_with<F, Fut, V, MF, M>(
        self,
        key: impl Into<String>,
        resolver: F,
        metadata: MF,
    ) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<V, FrameworkError>> + Send + 'static,
        V: Serialize + 'static,
        MF: Fn(&V) -> M + Send + Sync + 'static,
        M: ProvidesScrollMetadata,
    {
        self.prop(key, Prop::scroll_lazy(resolver, metadata))
    }

    fn attach_scroll(
        mut self,
        key: String,
        wrap_key: Option<String>,
        metadata: impl ProvidesScrollMetadata,
        prop: Prop,
    ) -> Self {
        let mut prop = prop.scroll(metadata);
        if let Some(wrap) = wrap_key {
            prop = prop.scroll_wrap(wrap);
        }
        self.put_prop(key, prop);
        self
    }

    /// Attach a paginator (`LengthAwarePaginator`, `Paginator` or
    /// `CursorPaginator`) as a scroll prop under `key`. The paginator's
    /// metadata becomes the prop's `ScrollMetadata`; its rows become the
    /// prop value.
    ///
    /// The value is the bare list of rows, so the merge instruction stays
    /// at the prop's root ([`Prop::scroll_at_root`]): under the default
    /// `<key>.data` wrapper of [`scroll`](Self::scroll) the client would
    /// find nothing to append to and would replace the list instead.
    pub fn paginate<T>(
        self,
        key: &'static str,
        paginator: impl crate::pagination::IntoInertiaScroll<T>,
    ) -> Self
    where
        T: Serialize + 'static,
    {
        let (meta, data) = paginator.into_inertia_scroll();
        let value = to_value_or_die(&data);
        self.prop(key, Prop::eager(value).scroll(meta).scroll_at_root())
    }

    /// Attach a flash value to this response. Appears under the
    /// top-level `flash` field of the page object (not under `props`).
    /// Use for one-shot toasts / success messages.
    ///
    /// Laravel's `Response::flash`: with a session in scope the value goes
    /// into the session's Inertia flash data at once, like
    /// [`Inertia::flash`](crate::Inertia::flash), so it reaches the next
    /// page response even when this one is never sent, and
    /// [`Inertia::get_flashed`](crate::Inertia::get_flashed) sees it.
    /// Without a session it rides on this response alone.
    pub fn flash<V: Serialize>(mut self, key: impl Into<String>, value: V) -> Self {
        let v = to_value_or_die(&value);
        self.put_flash(key.into(), v);
        self
    }

    /// Put one flash entry where [`flash`](Self::flash) documents it goes.
    fn put_flash(&mut self, key: String, value: Value) {
        let mut entry = serde_json::Map::new();
        entry.insert(key.clone(), value.clone());
        if !flash::put_in_session(entry) {
            self.flash.insert(key, value);
        }
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
        self.put_prop(key, Prop::eager(v));
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
        self.put_prop(key, Prop::eager(v).always());
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
        self.put_prop(key, Prop::eager(v).merge_strategy(strategy));
        Ok(self)
    }

    /// Fallible sibling of [`scroll`](Self::scroll). For an async-resolved
    /// scroll value, [`scroll_with`](Self::scroll_with) is already fallible.
    pub fn try_scroll<V: Serialize>(
        self,
        key: impl Into<String>,
        metadata: impl ProvidesScrollMetadata,
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
        metadata: impl ProvidesScrollMetadata,
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
        self.put_flash(key, v);
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
    /// should clear. When the clearing handler redirects - logout is the
    /// canonical case - reach for [`App::clear_history`](crate::App::clear_history)
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

    /// Send every integer beyond JavaScript's safe range (plus or minus
    /// 9007199254740991) in props and flash as `{"$bigint": "<digits>"}`,
    /// with `preserveBigIntegers: true` on the page, so the client restores
    /// it as an exact `BigInt`; `false` sends plain numbers. Overrides
    /// [`InertiaConfig::preserve_big_integers`] for this response. Maps to
    /// Laravel's `Response::preserveBigIntegers($bool)`.
    pub fn preserve_big_integers(mut self, on: bool) -> Self {
        self.preserve_big_integers = Some(on);
        self
    }

    /// An external redirect: a full page navigation the client performs
    /// with `window.location = url` rather than an Inertia visit - Laravel's
    /// `Inertia::location($url)`, also reachable as
    /// [`Inertia::location`](crate::Inertia::location).
    ///
    /// The answer depends on the request, whose facts the server scopes for
    /// every request it dispatches (the
    /// [`InertiaHeadersMiddleware`](crate::InertiaHeadersMiddleware) refines
    /// them with the application's hooks):
    ///
    /// - an Inertia visit gets `409` + `X-Inertia-Location`, the only form
    ///   the client follows out of the app;
    /// - anything else gets a `302` + `Location`, or, when `target` is a
    ///   [`Redirect`](crate::Redirect), that redirect as it is, its status,
    ///   flash and cookies included - a hard navigation into an OAuth or SSO
    ///   bounce has no use for a `409` and would dead-end on it.
    ///
    /// Outside a dispatched request, in a unit test for one, no visit is in
    /// scope and the answer is the `302`; [`location_for`](Self::location_for)
    /// decides from a request given explicitly.
    ///
    /// **When to use which redirect form:**
    /// - [`Redirect::to`](crate::Redirect::to) - standard 302/303 with
    ///   `Location` header. The normal case for redirects after form
    ///   submission inside the Inertia app.
    /// - [`InertiaResponse::redirect`](Self::redirect) - 409 +
    ///   `X-Inertia-Redirect` for soft Inertia SPA navigation; a redirect
    ///   with a `#fragment` on an Inertia visit becomes this on its own.
    /// - [`InertiaResponse::location`](Self::location) - to leave the
    ///   Inertia app entirely.
    pub fn location(target: impl Into<InertiaLocation>) -> HttpResponse {
        let target = target.into();
        let is_inertia = super::visit::current().is_some_and(|visit| visit.is_inertia);
        match (target.0, is_inertia) {
            (LocationTarget::Url(url), true) => Self::inertia_location(&url),
            (LocationTarget::Url(url), false) => {
                HttpResponse::new().status(302).header("Location", url)
            }
            (LocationTarget::Redirect(response), false) => response,
            (LocationTarget::Redirect(response), true) => {
                let url = response.header_value("Location").unwrap_or("/").to_string();
                let carried: Vec<(String, String)> = response
                    .headers()
                    .filter(|(name, _)| name.eq_ignore_ascii_case("Set-Cookie"))
                    .map(|(name, value)| (name.to_string(), value.to_string()))
                    .collect();
                Self::inertia_location(&url).with_headers(carried)
            }
        }
    }

    /// The `409` + `X-Inertia-Location` an Inertia visit follows out of
    /// the app.
    fn inertia_location(url: &str) -> HttpResponse {
        HttpResponse::new()
            .status(409)
            .header("X-Inertia-Location", url)
    }

    /// Request-aware external redirect - Laravel's `Inertia::location($url)`.
    ///
    /// - Inertia XHR (`X-Inertia: true`) → `409` + `X-Inertia-Location`,
    ///   which the client turns into `window.location = url`.
    /// - Anything else → a plain `302` + `Location`.
    ///
    /// The same answer as [`location`](Self::location), decided from the
    /// request given rather than the one the Inertia middleware is
    /// handling, so it works on a route without that middleware.
    pub fn location_for<R: InertiaRequestExt + ?Sized>(
        req: &R,
        url: impl AsRef<str>,
    ) -> HttpResponse {
        if req.is_inertia() {
            Self::inertia_location(url.as_ref())
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
    /// instead - the auto-303 middleware will rewrite 302→303 for non-GET.
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
        self.put_prop(key, Prop::eager(value));
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
    ///   mount node - the Inertia 3 contract that `getInitialPageFromDOM`
    ///   reads. Both carry [`InertiaConfig::mount_id`], `app` by default.
    pub async fn resolve<R: InertiaRequestExt>(
        mut self,
        req: &R,
    ) -> Result<HttpResponse, FrameworkError> {
        self.prepare_component()
            .map_err(reflash_session_values_after_eager_error)?;
        let staged_session = StagedInertiaSessionValues::stage();
        let is_inertia_request = req.is_inertia();
        let filter = PartialFilter::build(req, &self.component);
        // Laravel gates the whole once-skip on `isInertia && !isPartial`
        // (`Response.php:307`). Honouring the client's "I already have
        // this cached" claim during an explicit partial reload means
        // `router.reload({ only: ['stats'] })` returns nothing at all for
        // the one key the user just asked for - the client asked BECAUSE
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
        // See `inertia-3.8.0/packages/core/src/requestParams.ts:139,149-150`:
        // the client puts reset keys into `only` AND `X-Inertia-Reset`, so
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
        // defaults to append. It does not drive `reset` - that comes
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
            preserve_big_integers,
            lazy_owned,
            providers,
            view_data,
            shared_data,
            render_source,
        } = self;
        // For the Inertia middleware, which tells a partial reload of this
        // page from a navigation by it when it records the previous URL.
        super::visit::record_rendered_component(&component);
        // Inertia DevTools records what this render resolved when its
        // middleware scoped a recorder for the request; otherwise none of
        // the collection below runs.
        let recorder = super::devtools::current_recorder();
        let mut collector = recorder.as_ref().map(|_| {
            let mut collector = super::devtools::Collector::new(
                &component,
                Some(super::devtools::SourceLocation::of(render_source)),
            );
            collector.component_path(super::pages::find_page_file(&config, &component).map(
                |path| {
                    std::fs::canonicalize(&path)
                        .unwrap_or(path)
                        .display()
                        .to_string()
                },
            ));
            collector
        });

        // Page URL: path AND query, or the app's resolver. The client
        // writes this into `history.state`, so a bare path silently
        // resets pagination / sort / filter state on every back-forward
        // navigation. Laravel's `Response::getUrl` does the same, and
        // `InertiaVersionMiddleware` derives its `X-Inertia-Location`
        // from the same expression - so by default the two agree; a
        // `url_resolver` intentionally moves only this one, because the
        // 409 bounce has to name a URL the browser can actually fetch.
        //
        // The URL carries the public root, the URL the browser shows
        // (PFX-005): the default derivation is an application path and
        // always gets it; a resolver's root-relative path gets it unless it
        // is already under the root.
        let url = page_url(config.url_resolver.as_ref(), req);

        // History-encryption precedence: per-response override (handler
        // wins) > middleware task_local > config default.
        let resolved_encrypt_history = encrypt_history
            .or_else(flash::encrypt_history_flag)
            .unwrap_or(config.encrypt_history_default);

        // preserve-fragment precedence: per-response override > the session
        // entry `Redirect::preserve_fragment()` sets > false. The session
        // lookup is a no-op outside a `SessionMiddleware` scope. The entry
        // lasts until a page emits it, however many redirects come first;
        // the staged session guard removes it only after the complete
        // response has been constructed, so a response error keeps it.
        let resolved_preserve_fragment =
            preserve_fragment.unwrap_or_else(|| staged_session.preserve_fragment());

        // clear-history precedence: per-response override OR the session
        // entry `App::clear_history()` sets. Either alone is enough -
        // unlike `preserve_fragment` there is no "force off" case, because
        // the only reason to ask for a history clear is that the previous
        // session must stop being readable. The entry lasts until a page
        // emits it, so a logout followed by two redirects still clears; the
        // page that emits it removes it, since a flag that stuck around
        // would rotate the key on every navigation and defeat encrypted
        // history entirely. No-op outside a `SessionMiddleware` scope.
        let resolved_clear_history = clear_history || staged_session.clear_history();

        // Layer props in precedence order (later writes override earlier):
        //   1. Static shared registry  (App::inertia_share, App::inertia_share_lazy)
        //   2. Trait-registered shared data (InertiaSharedData::share)
        //   3. User-supplied props attached via the builder
        //
        // Track the union of (1) + (2) as `shared_keys` so the page
        // object can advertise them under `sharedProps` (the client
        // uses this for instant-swap during navigation - see
        // `inertia-3.8.0/packages/core/src/router.ts:633`, `performInstantSwap`).
        //
        // A dotted share key contributes only its ROOT segment. The
        // registry stores `"user.name"` literally and `unpack_map` nests
        // it into `props.user.name` at the end of `resolve_props`, but
        // the client filters `sharedProps` against `props` with a flat
        // `key in current.props` test and then spreads the survivors
        // into the page it renders mid-swap, or hands them to a
        // `pageProps` callback
        // (`inertia-3.8.0/packages/core/src/router.ts:636-645`). A raw
        // `"user.name"` entry fails that lookup, so `user` would be
        // *absent* - not stale - for that frame and any layout reading
        // `props.user.name` throws. Laravel never hits this because
        // `Inertia::share` runs `Arr::set` at share time
        // (`inertia-laravel-2.0.25/src/ResponseFactory.php:94`), so its
        // shared bag is already keyed by the root segment.
        let registry = App::inertia_registry();
        let mut merged: IndexMap<String, Prop> = IndexMap::new();
        let mut shared_keys: Vec<String> = Vec::new();
        fn track_shared(shared_keys: &mut Vec<String>, k: &str) {
            let root = k.split('.').next().unwrap_or(k);
            if !shared_keys.iter().any(|existing| existing == root) {
                shared_keys.push(root.to_string());
            }
        }
        // An error page the application's callback rendered without
        // `with_shared_data()` takes none of the shared layers (PAR-062).
        let shares = if shared_data {
            registry.snapshot_static()?
        } else {
            Vec::new()
        };
        for (k, v) in shares {
            track_shared(&mut shared_keys, &k);
            merged.insert(k, v);
        }
        // Providers expand with the render context (PAR-051): the shared
        // ones after the keyed shares, and their keys are shared keys; the
        // page's after every shared layer, under the page's own props.
        let context = RenderContext::new(&component, req);
        let shared_providers = if shared_data {
            registry.shared_providers()?
        } else {
            Vec::new()
        };
        for provider in shared_providers {
            for (k, v) in provider.to_inertia_properties(&context)? {
                track_shared(&mut shared_keys, &k);
                merged.insert(k, v);
            }
        }
        let trait_provider = if shared_data {
            registry.trait_provider()?
        } else {
            None
        };
        if let Some(provider) = trait_provider {
            let trait_shared = provider.share(req, &component).await?;
            for (k, v) in trait_shared {
                track_shared(&mut shared_keys, &k);
                merged.insert(k, v);
            }
        }
        for provider in &providers {
            merged.extend(provider.to_inertia_properties(&context)?);
        }
        // The `share` and `share_once` middleware hooks, run for this
        // request before the handler (Laravel's middleware `share()`).
        let visit = super::visit::current();
        if let Some(visit) = visit.as_ref().filter(|_| shared_data) {
            for (k, v) in visit.shared() {
                track_shared(&mut shared_keys, k);
                merged.insert(k.clone(), v.clone());
            }
        }
        for (k, v) in props {
            // Note: when user props override a shared key, we keep the
            // key in `shared_keys` per the Inertia v3 client contract -
            // the client reads the value from `props` (user's override)
            // and uses `sharedProps` only as a key list.
            merged.insert(k, v);
        }
        // Inertia DevTools: which keys the shared props supplied and where
        // each was shared, and the metadata of every prop, read from the
        // flags before resolution consumes the props (PAR-073).
        if let Some(collector) = collector.as_mut() {
            let mut keys = vec![ERRORS_KEY.to_string()];
            keys.extend(shared_keys.iter().filter(|k| *k != ERRORS_KEY).cloned());
            collector.shared_keys(keys);
            if shared_data {
                for (key, location) in registry.share_sources() {
                    collector.share_source(&key, super::devtools::SourceLocation::of(location));
                }
                if let Some(visit) = visit.as_ref()
                    && let Some(location) = visit.hooks_location()
                {
                    let source = super::devtools::SourceLocation::of(location);
                    for key in visit.shared().keys() {
                        collector.hook_share_source(key, source);
                    }
                }
            }
            let classify_request = super::devtools::ClassifyRequest::of(req);
            if !merged.contains_key(ERRORS_KEY) {
                collector.prop(ERRORS_KEY, super::devtools::errors_meta());
            }
            for (key, prop) in &merged {
                collector.prop(key, super::devtools::classify(key, prop, &classify_request));
            }
        }
        // Every response shares the validation errors, as Laravel's
        // middleware does with `'errors' => Inertia::always(...)`, so
        // `errors` heads the list; with `expose_shared_props` off there is
        // no list at all (PAR-051).
        let shared_keys = if config.expose_shared_props {
            let mut keys = Vec::with_capacity(shared_keys.len() + 1);
            keys.push(ERRORS_KEY.to_string());
            keys.extend(shared_keys.into_iter().filter(|k| k != ERRORS_KEY));
            keys
        } else {
            Vec::new()
        };

        let (mut materialized, metadata) = resolve_props(
            merged,
            &filter,
            &except_once,
            &reset_keys,
            error_bag.as_deref(),
            scroll_intent.as_deref(),
            &lazy_owned,
            config.max_concurrent_resolvers,
            config.with_all_errors,
            staged_session.error_bags(),
            req,
        )
        .await?;
        if let Some(collector) = collector.as_mut() {
            for key in &metadata.rescued {
                collector.rescued(key);
            }
        }

        // Combine flash from three sources, in precedence order
        // (later writes override earlier so same-request entries win
        // over inherited cross-redirect entries):
        //   1. Session `_flash.old.*` - bridged from the previous
        //      request via `From<Redirect> for Response` then aged by
        //      `SessionMiddleware`.
        //   2. Task-local flash bag - same-request `App::flash`.
        //   3. Builder flash - same-request `InertiaResponse::flash`.
        let mut flash = staged_session.page_flash();
        for (k, v) in flash::drain() {
            flash.insert(k, v);
        }
        for (k, v) in response_flash {
            flash.insert(k, v);
        }

        // Big-integer markers go on the finished props and flash, the
        // values Laravel's `encodeBigIntegersWhenEnabled` sees.
        let resolved_preserve_big_integers =
            preserve_big_integers.unwrap_or(config.preserve_big_integers);
        if resolved_preserve_big_integers {
            encode_big_integers_in(&mut materialized);
            encode_big_integers_in(&mut flash);
        }

        let page = build_page_object(
            &component,
            ResolvedProps {
                props: materialized,
                metadata,
            },
            &config,
            url,
            flash,
            PageObjectFlags {
                encrypt_history: resolved_encrypt_history,
                clear_history: resolved_clear_history,
                preserve_fragment: resolved_preserve_fragment,
                preserve_big_integers: resolved_preserve_big_integers,
            },
            shared_keys,
        );
        let response = if is_inertia_request {
            build_json_response(&page)?
        } else {
            // SSR runs only for HTML (non-XHR) visits. XHR is a JSON
            // page-object response and never needs prerender.
            // The `root_view` middleware hook chooses the configuration the
            // document is written with.
            let config = match visit.as_ref().and_then(|visit| visit.hooks()) {
                Some(hooks) => hooks.root_view(req, config),
                None => config,
            };
            // In development the dispatch goes hot (PAR-058): the
            // configuration supplies the Vite dev server as the hot URL.
            let ssr = config.ssr_for_dispatch();
            let ssr_result = super::ssr_gateway::gateway()
                .dispatch(&ssr, req, &page)
                .await?;
            match config.root_template_for(req).application() {
                Some(template) => build_template_response(
                    template,
                    &page,
                    &config,
                    title.as_deref(),
                    ssr_result.as_ref(),
                    &view_data,
                )?,
                None => build_html_response(&page, &config, title.as_deref(), ssr_result.as_ref())?,
            }
        };
        // Inertia DevTools records the page only once the response that
        // carries it was built: a JSON encoding, SSR dispatch or root
        // template that fails returned above, and the entry then records
        // the error response the client gets, with no page and no prop
        // values (PAR-072, PAR-073).
        if let (Some(recorder), Some(collector)) = (recorder.as_ref(), collector.take()) {
            recorder.page_rendered(collector.build(page));
        }
        staged_session.commit();
        Ok(response)
    }

    /// The steps of Laravel's `ResponseFactory::render` that settle the
    /// component before the page is built: the transformer
    /// [`Inertia::transform_component_using`](crate::Inertia::transform_component_using)
    /// installed, then the [`InertiaConfig::ensure_pages_exist`] check on
    /// the name it gives.
    fn prepare_component(&mut self) -> Result<(), FrameworkError> {
        // The request hooks of the Inertia middleware: a `version` answer
        // replaces the configured version for this page, as the version
        // check compares against it, and a `url_resolver` the configured
        // one.
        if let Some(visit) = super::visit::current() {
            if let Some(version) = visit.version() {
                self.config.version = super::config::VersionResolver::Static(version.to_string());
            }
            if let Some(resolver) = visit.hooks().and_then(|hooks| hooks.url_resolver()) {
                self.config.url_resolver = Some(resolver);
            }
        }
        let component = std::mem::take(&mut self.component);
        self.component = App::inertia_registry()
            .runtime()
            .transform_component(component);
        if self.config.ensure_pages_exist {
            super::pages::ensure_page_exists(&self.config, &self.component)?;
        }
        Ok(())
    }

    /// Build the page object without producing an HTTP response - used by
    /// tests that want to inspect the page object directly.
    #[cfg(test)]
    pub(crate) async fn build_page_object_for_test(
        self,
        url: String,
        filter: &PartialFilter,
    ) -> Value {
        let staged_session = StagedInertiaSessionValues::stage();
        let Self {
            component,
            props,
            flash: response_flash,
            config,
            title: _,
            encrypt_history,
            clear_history,
            preserve_fragment,
            preserve_big_integers,
            lazy_owned,
            providers: _,
            view_data: _,
            shared_data: _,
            render_source: _,
        } = self;
        let (mut materialized, metadata) = resolve_props(
            props,
            filter,
            &[],
            &[],
            None,
            None,
            &lazy_owned,
            usize::MAX,
            config.with_all_errors,
            staged_session.error_bags(),
            &TestRequest,
        )
        .await
        .expect("test resolver should not fail");
        let resolved_encrypt_history = encrypt_history.unwrap_or(config.encrypt_history_default);
        // Test helper doesn't run inside a session scope by default,
        // so we never pick up a flashed flag here - only the explicit
        // override. Tests that DO drive a session scope via
        // `session_scope_for_test` pick up `_flash.old.*` via the
        // shared session-flash merge below.
        let resolved_preserve_fragment =
            preserve_fragment.unwrap_or_else(|| staged_session.preserve_fragment());
        let resolved_clear_history = clear_history || staged_session.clear_history();
        // The test helper does not exercise the shared-data registry.
        let shared_keys: Vec<String> = Vec::new();

        // Mirror the same three-tier flash precedence as `resolve`:
        // session-old < task-local < builder. Keeps the test helper
        // honest about the production drain path.
        let mut flash = staged_session.page_flash();
        for (k, v) in flash::drain() {
            flash.insert(k, v);
        }
        for (k, v) in response_flash {
            flash.insert(k, v);
        }

        let resolved_preserve_big_integers =
            preserve_big_integers.unwrap_or(config.preserve_big_integers);
        if resolved_preserve_big_integers {
            encode_big_integers_in(&mut materialized);
            encode_big_integers_in(&mut flash);
        }

        let page = build_page_object(
            &component,
            ResolvedProps {
                props: materialized,
                metadata,
            },
            &config,
            url,
            flash,
            PageObjectFlags {
                encrypt_history: resolved_encrypt_history,
                clear_history: resolved_clear_history,
                preserve_fragment: resolved_preserve_fragment,
                preserve_big_integers: resolved_preserve_big_integers,
            },
            shared_keys,
        );
        staged_session.commit();
        page
    }

    /// Build a `409 Conflict` response indicating an asset version mismatch.
    /// The client follows `X-Inertia-Location` for a fresh full-page visit.
    ///
    /// `X-Inertia-Version` carries the current version,
    /// [`Inertia::get_version`](crate::Inertia::get_version), as the version
    /// middleware's 409 does: the client reads it so a poll or a background
    /// prop load does not force a full reload after a deploy.
    pub fn version_conflict(new_url: &str) -> HttpResponse {
        HttpResponse::new()
            .status(409)
            .header("X-Inertia-Location", new_url)
            .header("X-Inertia-Version", crate::Inertia::get_version())
    }
}

/// Where [`InertiaResponse::location`] sends the visitor: a URL, or a
/// [`Redirect`](crate::Redirect) whose target an Inertia visit follows and a
/// plain visit receives as it is - Laravel's `location($url)` takes a
/// string or a `RedirectResponse` the same way.
pub struct InertiaLocation(LocationTarget);

enum LocationTarget {
    Url(String),
    /// The redirect as a response, converted when the location was built so
    /// its flash data reaches the session either way.
    Redirect(HttpResponse),
}

impl From<&str> for InertiaLocation {
    fn from(url: &str) -> Self {
        Self(LocationTarget::Url(url.to_string()))
    }
}

impl From<String> for InertiaLocation {
    fn from(url: String) -> Self {
        Self(LocationTarget::Url(url))
    }
}

impl From<&String> for InertiaLocation {
    fn from(url: &String) -> Self {
        Self(LocationTarget::Url(url.clone()))
    }
}

impl From<std::borrow::Cow<'_, str>> for InertiaLocation {
    fn from(url: std::borrow::Cow<'_, str>) -> Self {
        Self(LocationTarget::Url(url.into_owned()))
    }
}

impl From<crate::http::Redirect> for InertiaLocation {
    fn from(redirect: crate::http::Redirect) -> Self {
        let response: crate::http::Response = redirect.into();
        Self(LocationTarget::Redirect(response.unwrap_or_else(|e| e)))
    }
}

/// Accumulator for Inertia v3 page-object metadata fields.
///
/// Each field corresponds to an optional top-level page-object property -
/// `deferredProps`, `rescuedProps`, `mergeProps`, etc. - and stays
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

/// What `resolve_props` hands on to `build_page_object`: the materialized
/// prop bag and the metadata that describes it. They are produced together
/// and read together, so they travel as one value.
struct ResolvedProps {
    /// Emitted as `props`.
    props: serde_json::Map<String, Value>,
    /// Emitted as the optional `deferredProps` / `mergeProps` / ... fields.
    metadata: PageMetadata,
}

/// The three page-object flags the client acts on after a visit. They
/// share one type, so they travel as named fields: two positional `bool`s
/// swapped at a call site would still compile and silently keep a history
/// the handler asked to clear, or drop a fragment it asked to keep.
struct PageObjectFlags {
    /// Emitted as `encryptHistory: true` when set, and omitted otherwise.
    encrypt_history: bool,
    /// Emitted as `clearHistory: true` when set, and omitted otherwise.
    clear_history: bool,
    /// Emitted as `preserveFragment: true` when set, and omitted otherwise.
    preserve_fragment: bool,
    /// Emitted as `preserveBigIntegers: true` when set, and omitted
    /// otherwise; the props and flash then carry `$bigint` markers.
    preserve_big_integers: bool,
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
    /// A [`Prop::scroll_lazy`] value, with the `scrollProps` entry its
    /// loader described, or `None` when this response ships no entry.
    /// Boxed so the common `Insert` outcome stays small.
    InsertScroll {
        key: String,
        value: Value,
        scroll: Option<Box<ScrollMetadataEntry>>,
    },
    Rescued {
        key: String,
    },
}

/// Parse an Inertia list header (`X-Inertia-Reset`,
/// `X-Inertia-Except-Once-Props`) by the rule every Inertia list header
/// follows: split on `,`, empty segments dropped, nothing trimmed, as
/// Laravel's `PropsResolver::parseHeader` reads it. Empty when the header
/// is absent or names nothing.
fn parse_csv_header<R: InertiaRequestExt>(req: &R, name: &str) -> Vec<String> {
    req.header(name)
        .and_then(super::prop::parse_header_list)
        .unwrap_or_default()
}

/// Render each flashed bag's `{field: [messages]}` into the value shape
/// the Inertia client is typed against.
///
/// Laravel emits `$errors[0]` unless `$withAllErrors` is set
/// (`inertia-laravel-2.0.25/src/Middleware.php:196`), and Inertia's
/// `ErrorValue` is `string` by default (`inertia-3.8.0/packages/core/src/types.ts:59,100`) -
/// a bare string is what `useForm().errors.email` resolves to.
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
/// alone - Laravel computes each block from the unfiltered prop bag
/// (`inertia-laravel-2.0.25/src/Response.php:553-560`, `:725-736`) -
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
/// A scroll prop folds into that same merge protocol unconditionally -
/// unlike a plain merge prop, its direction defaults to append rather
/// than needing an explicit `.merge()` flag - and its per-key `reset`
/// flag is read straight from `reset_keys` too, independent of the
/// client's `X-Inertia-Infinite-Scroll-Merge-Intent` header.
/// The `errors` prop the session's validation errors make, Laravel's
/// `Middleware::resolveValidationErrors` (inertia-laravel 3.5.1):
///
/// - with `X-Inertia-Error-Bag`, `{<bag>: <default bag>}` when the session
///   holds a `default` bag;
/// - without it, the `default` bag flat (`{field: message}`), which is what
///   the client binds `page.props.errors.field` to;
/// - otherwise every bag the session holds, keyed by name, which is `{}`
///   when it holds none.
///
/// A bag the validation redirect flashed under the header's name is a named
/// bag, so a form that sent `X-Inertia-Error-Bag: login` reads its errors
/// back as `errors.login` either way.
fn session_errors_prop(
    mut session_errors: serde_json::Map<String, Value>,
    error_bag: Option<&str>,
) -> Value {
    match (session_errors.remove("default"), error_bag) {
        (Some(default_bag), Some(bag)) => {
            let mut wrapped = serde_json::Map::new();
            wrapped.insert(bag.to_string(), default_bag);
            Value::Object(wrapped)
        }
        (Some(default_bag), None) => default_bag,
        (None, _) => Value::Object(session_errors),
    }
}

/// The one prop key the Inertia v3 contract guarantees on every page
/// object. Named rather than spelled out at each site because three
/// separate rules key off it: the session seed, the `X-Inertia-Error-Bag`
/// post-pass, and the always-visible exemption from partial-reload
/// filtering.
const ERRORS_KEY: &str = "errors";

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
    session_errors: serde_json::Map<String, Value>,
    request: &dyn InertiaRequestExt,
) -> Result<(serde_json::Map<String, Value>, PageMetadata), FrameworkError> {
    let mut materialized = serde_json::Map::new();
    let mut metadata = PageMetadata::default();
    // A `Prop::property` value converts with its sibling props (PAR-051),
    // the whole prop bag before resolution, Laravel's `PropertyContext`.
    // The loop below consumes the bag, so a copy is kept, and only when a
    // property prop needs it.
    let siblings: IndexMap<String, Prop> = if props.values().any(Prop::is_property) {
        props.clone()
    } else {
        IndexMap::new()
    };

    // `errors` is always present per the Inertia v3 contract. Seed
    // with whatever the session has flashed under the canonical bag
    // keys (`errors.<bag>` - written by [`crate::http::Redirect::with_errors`]),
    // or an empty object when nothing flashed. The `X-Inertia-Error-Bag`
    // wrapping happens AFTER all props resolve - see the bottom of this
    // function. Doing it post-resolution means a handler that injects
    // errors via `.with("errors", {...})` still gets correctly scoped.
    //
    // The caller supplies a single staged bag-prefix snapshot. It is empty
    // outside a `SessionMiddleware` scope and is committed only after the
    // complete response succeeds.
    // Resolve it to the Inertia shape, Laravel's `resolveValidationErrors`
    // (see `session_errors_prop`). A handler or shared `errors` prop
    // replaces it below; only that one is wrapped by the post-pass.
    let errors_supplied = props.contains_key(ERRORS_KEY);
    let session_errors = collapse_error_bags(session_errors, with_all_errors);
    materialized.insert(
        ERRORS_KEY.to_string(),
        session_errors_prop(session_errors, error_bag),
    );

    let mut tasks: Vec<TaskFuture> = Vec::new();
    let now = crate::clock::now();
    let now_ms = now.timestamp_millis();
    // The keys that reach the page, in registration order. Eager values
    // land in `materialized` at once and resolver values only after every
    // task finishes, so the map alone records which source was faster,
    // not which prop came first. Dotted keys compose in this order below.
    let mut registered: Vec<String> = Vec::new();

    for (key, prop) in props {
        // The absent sentinel (`when_loaded!` on an unloaded relation)
        // carries neither a value nor metadata. Checked first so flags
        // set on it cannot leak into the page object.
        if prop.is_absent() {
            continue;
        }

        // OWNER-TAGGED INCLUDE GATE (`#[derive(Data)]`) - Stage 1.
        //
        // Gate order (spec):
        //   Stage 1 - include-set membership + per-DTO allowlist. A
        //     field the request never opted into is dropped outright:
        //     no value, no metadata, no `deferredProps` announcement,
        //     because it is not part of this response at all. A field
        //     that IS named by `?include=` but is off the allowlist
        //     raises Err(400), and that error MUST propagate before
        //     partial-data can silently swallow it.
        //   Stage 2 - everything below: metadata, the deferred announce,
        //     the partial-data filter, and narrowing, all reached the
        //     same way an ordinary prop reaches them.
        //
        // The gate sits here, ahead of every other block, rather than
        // inside a fast path keyed on `Prop::is_lazy()`. `is_lazy()` is
        // false for any *flagged* prop - a `#[data(lazy(deferred))]`
        // field is `Visibility::Deferred` - so a fast path conditioned on
        // it never saw the flagged props at all and they resolved off the
        // ordinary path with no include check anywhere in it. Running the
        // gate for every owner-tagged prop closes that, and leaves the
        // flag-free ones to reach the identical outcome through the
        // ordinary path below: they own no metadata, so every block down
        // to `should_include` is a no-op for them.
        if let Some(&(owner, field)) = lazy_owned.get(&key)
            && !prop.passes_include_gate(owner, field)?
        {
            continue;
        }

        // The metadata gates. Laravel decides a prop's metadata from the
        // only/except lists, never from whether its value resolved, which
        // is what lets a deferred prop carry its merge instruction on the
        // very visit that withheld its value. `passes_lists` is the path
        // rule a value passes (the key is, descends from, or leads to an
        // `only` entry); `carries_instructions` is the stricter rule its
        // `merge` and `once` instructions pass (an `only` entry is the key
        // or an ancestor), Laravel's `isIncludedInPartialMetadata`.
        let passes_lists = filter.should_include_eager(&key);
        let carries_instructions = filter.should_include_metadata(&key);

        // ---- once ----
        let mut client_has_cached = false;
        if prop.is_once() {
            let cache_key = prop.once_cache_key(&key);
            // Domain 20 audit D20-C: the server owns the expiry. Without
            // this a stale client can hold `X-Inertia-Except-Once-Props`
            // past the `until(...)` deadline and never see a fresh value.
            let expires_at = prop.once_expires_at_for(now);
            let server_expired = match expires_at {
                Some(ts) => now_ms >= ts,
                None => false,
            };
            client_has_cached =
                !prop.is_fresh() && !server_expired && except_once.iter().any(|k| k == &cache_key);
            if carries_instructions {
                metadata.once.insert(
                    cache_key,
                    OnceMetadataEntry {
                        prop_name: key.clone(),
                        expires_at,
                    },
                );
            }
        }

        // A once prop the client already holds, and that is not deferred,
        // keeps its `onceProps` entry and nothing else: Laravel excludes it
        // through `excludeAlreadyLoadedProp`, which collects the once
        // instruction alone. A deferred one goes through the
        // `IgnoreFirstLoad` branch first and keeps its merge instruction.
        let held_once = client_has_cached && !prop.is_defer();

        // ---- merge ----
        //
        // A scroll prop derives its direction from the client's
        // merge-intent header below, so an explicit merge flag on the
        // same prop is ignored. `X-Inertia-Reset` names merge keys the
        // client wants to start fresh from: resolve the value normally
        // but drop the instruction, so the client replaces instead of
        // appending.
        if !prop.is_scroll()
            && let Some(mode) = prop.merge_mode()
            && carries_instructions
            && !held_once
            && !reset_keys.iter().any(|k| k == &key)
        {
            for field in prop.match_on_fields() {
                metadata.match_props_on.push(format!("{key}.{field}"));
            }
            push_merge_paths(&mut metadata, &key, &prop, mode);
        }

        // ---- scroll ----
        //
        // Laravel folds every scroll prop into the merge protocol: the
        // `ScrollProp` constructor sets `merge = true`, and
        // `configureMergeIntent` appends at its wrapper (`data` by
        // default), or prepends there when the client's
        // `X-Inertia-Infinite-Scroll-Merge-Intent` says `prepend`
        // (`inertia-laravel-3.5.1/src/ScrollProp.php`). `reset` comes
        // from `X-Inertia-Reset` alone, and a reset key ships no merge
        // instruction, as for any merge prop.
        //
        // A deferred scroll prop on a visit that is not a partial reload
        // is excluded before `configureMergeIntent` runs, so Laravel's
        // `excludeDeferredProp` collects the merge instruction at the bare
        // key, and ships no `scrollProps` entry: nothing is on screen yet
        // for a cursor to describe (`PropsResolver.php`). The follow-up
        // partial reload configures the wrapper and gets both.
        //
        // `match_on` fields are relative to the prop - `{key}.{field}` -
        // with no wrapper prefix: Laravel's `matchesOn` entries are
        // prefixed with the prop path alone, so `match_on("data.id")` on
        // `posts` is `posts.data.id`, which the client matches against the
        // `posts.data` merge path.
        //
        // Gate: the lists, as for the once and merge blocks above, and
        // placed above the `continue`s below for the same reason: the
        // instructions follow the only/except lists, not whether the value
        // resolved. An `Always` scroll prop outside the requested set still
        // ships its value but no merge instruction, or the client would
        // append the rows it already holds to themselves.
        //
        // A `Prop::scroll_lazy` prop's facts come from its loaded value, so
        // its entry is recorded when the value resolves (`scroll_entry`
        // carries the `reset` flag there) and not on a visit that
        // withholds the value.
        //
        // The merge instruction passes the stricter `carries_instructions`
        // gate and the held-once rule, like every other merge instruction;
        // the `scrollProps` entry, the cursor, needs `passes_lists` alone,
        // as Laravel collects a scroll prop's cursor for every scroll prop
        // it resolves.
        let mut scroll_entry: Option<bool> = None;
        if passes_lists && prop.is_scroll() {
            let is_reset = reset_keys.iter().any(|k| k == &key);
            let announced_only = prop.is_defer() && !filter.matched;
            if !is_reset && carries_instructions && !held_once {
                for field in prop.match_on_fields() {
                    metadata.match_props_on.push(format!("{key}.{field}"));
                }
                let path = match prop.scroll_wrap_key() {
                    Some(wrap) if !announced_only => format!("{key}.{wrap}"),
                    _ => key.clone(),
                };
                if prop.merge_mode() == Some(MergeMode::Deep) {
                    // Deep merge recurses through the whole value, so it
                    // targets the bare key whatever the wrapper.
                    metadata.deep_merge.push(key.clone());
                } else if !announced_only && scroll_intent == Some("prepend") {
                    metadata.merge_prepend.push(path);
                } else {
                    metadata.merge.push(path);
                }
            }
            if !announced_only {
                scroll_entry = Some(is_reset);
                if let Some(scroll_meta) = prop.scroll_metadata().cloned() {
                    metadata.scroll.insert(
                        key.clone(),
                        ScrollMetadataEntry {
                            metadata: scroll_meta,
                            reset: is_reset,
                        },
                    );
                }
            }
        }

        // ---- deferred: announce instead of resolving ----
        if prop.is_defer() && !filter.should_include_optional(&key) {
            // `resolveDeferredProps` returns `[]` the moment the request
            // is partial, before it inspects a single prop
            // (`inertia-laravel-2.0.25/src/Response.php:661-663`). A
            // partial reload IS the client working through announcements
            // it already holds; re-announcing the deferred keys it did
            // not ask for this round would send it back for them again.
            //
            // A deferred prop the client already holds is not announced
            // again either - otherwise it refetches on every navigation
            // and `once` buys nothing (`Response.php:653-673`).
            if !filter.matched && !client_has_cached {
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

        // The validation-error bag defaults to always-visible, so it
        // bypasses partial-reload filtering the way an `Always` prop
        // does. Laravel shares it as
        // `Inertia::always($this->resolveValidationErrors($request))`
        // (`inertia-laravel-2.0.25/src/Middleware.php:61`): `only` never
        // drops it, and neither does `except`, because `resolveAlways`
        // re-injects it right after `Arr::forget` ran
        // (`Response.php:406-416`).
        //
        // Suprnova seeds the session-flashed bag ahead of this loop,
        // where no filter can reach it. A handler-supplied
        // `.with("errors", …)` prop arrives here instead, and without
        // this exemption a partial reload that filtered it out fell back
        // to the seeded bag - usually `{}` - handing the client an
        // *empty* errors object rather than omitting the key. That is
        // the destructive shape: the client folds a partial response in
        // with `{...current.props, ...response.props}`
        // (`inertia-3.8.0/packages/core/src/response.ts:440`), so `{}`
        // wipes the errors it was already displaying, while an absent
        // key would have left them alone.
        //
        // An explicit visibility flag still wins - `.optional()` on the
        // errors key means the caller wants it withheld, the same way
        // `Inertia::optional(...)` under the `errors` key overrides the
        // middleware's `AlwaysProp` in Laravel's merged bag.
        let is_errors_bag = key == ERRORS_KEY && prop.visibility() == Visibility::Standard;

        if !is_errors_bag && !filter.should_include(&key, &prop) {
            continue;
        }

        // ---- value ----
        let rescue = prop.is_defer() && prop.rescues();
        // Dotted `only`/`except` entries narrow literal values only. A
        // value that came from a resolver or a prop object - `always`
        // included, whatever the request names inside it - ships whole,
        // as Laravel's `PropsResolver::resolveProps` stops filtering below
        // a value that was not a literal array. The errors bag is never
        // narrowed either.
        let narrow_value = !is_errors_bag && narrows_as_literal(&key, &prop);
        registered.push(key.clone());
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
            // A property converts now, with its context, and ships whole,
            // as Laravel ships an object's conversion.
            PropSource::Property(value) => {
                let context = PropertyContext::new(&key, &siblings, request);
                match value.to_inertia_property(&context) {
                    Ok(v) => {
                        materialized.insert(key, v);
                    }
                    Err(e) if rescue => {
                        tasks.push(Box::pin(async move { Ok(rescued_outcome(key, e)) }));
                    }
                    Err(e) => return Err(e),
                }
            }
            // A scroll loader's value ships whole, as Laravel ships a
            // closure's result, and brings the facts its loader read.
            PropSource::ScrollResolver(loader) => {
                tasks.push(Box::pin(async move {
                    match loader().await {
                        Ok((value, facts)) => Ok(TaskOutcome::InsertScroll {
                            scroll: scroll_entry.map(|reset| {
                                Box::new(ScrollMetadataEntry {
                                    metadata: facts,
                                    reset,
                                })
                            }),
                            key,
                            value,
                        }),
                        Err(e) if rescue => Ok(rescued_outcome(key, e)),
                        Err(e) => Err(e),
                    }
                }));
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
                            // Render the full source chain once and reuse it
                            // for both the log and the event - otherwise a
                            // `FrameworkError::External` would report only
                            // its context line and drop the wrapped failure,
                            // the same reason `http/response.rs` renders the
                            // chain before logging its 5xx.
                            let logged = crate::error::render_error_chain(&e);
                            tracing::warn!(
                                prop_key = %key,
                                error = %logged,
                                "inertia deferred prop resolver failed; rescued per spec",
                            );
                            // Build the event on the current task so the
                            // REQUEST_ID task-local is in scope (a spawned
                            // task wouldn't inherit it). The dispatch
                            // itself is spawned per the documented
                            // ErrorOccurred best-effort contract - see
                            // `events/builtins.rs` and the matching
                            // pattern in `http/response.rs` - so we do
                            // not block the Inertia partial-response
                            // collector on listener execution.
                            let evt = crate::events::ErrorOccurred {
                                error_message: logged,
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
    // the output stream - outcome order does not actually matter for
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
            TaskOutcome::InsertScroll { key, value, scroll } => {
                if let Some(entry) = scroll {
                    metadata.scroll.insert(key.clone(), *entry);
                }
                materialized.insert(key, value);
            }
            TaskOutcome::Rescued { key } => {
                metadata.rescued.push(key);
            }
        }
    }

    // Put the values back in registration order: `errors` first, where
    // the session seed put it, then every prop in the order it was
    // registered, so `dotted::unpack_map` composes a later dotted key over
    // an earlier parent whether either one was eager or resolver-backed.
    let mut ordered = serde_json::Map::with_capacity(materialized.len());
    if let Some(errors) = materialized.remove(ERRORS_KEY) {
        ordered.insert(ERRORS_KEY.to_string(), errors);
    }
    for key in registered {
        if let Some(value) = materialized.remove(&key) {
            ordered.insert(key, value);
        }
    }
    ordered.extend(materialized);
    let mut materialized = ordered;

    // `X-Inertia-Error-Bag` scoping of a handler-provided `errors` prop
    // (via `.with("errors", {...})`), applied AFTER all props have resolved
    // so the prop the handler set is the one wrapped. The session's errors
    // were shaped when they were seeded. The value is wrapped in place:
    // moving the key would move another prop out of registration order.
    if errors_supplied
        && let Some(bag) = error_bag
        && let Some(errors_val) = materialized.get_mut(ERRORS_KEY)
    {
        let mut wrapper = serde_json::Map::new();
        wrapper.insert(bag.to_string(), errors_val.take());
        *errors_val = Value::Object(wrapper);
    }

    // Dot-key nesting - Laravel's `Arr::set`-based `resolveArrayableProperties`
    // unpack step (`reference/inertia-laravel-2.0.25/src/Response.php:344-368`),
    // applied once to the fully resolved, fully filtered prop bag so it sees
    // exactly what's about to ship: eager, lazy, deferred-and-resolved,
    // merged, once, and scroll values alike, whether they came from the
    // response builder or the shared registry - both stored under their
    // literal (possibly dotted) key up to this point. A key with no dot
    // passes through as a plain insert. This never recurses into a prop's
    // *value* - the `errors` object above keeps whatever dotted validation
    // field names it carries internally; only top-level prop keys nest.
    let materialized = dotted::unpack_map(materialized);

    Ok((materialized, metadata))
}

/// Record where a merge prop folds in: its root, or the nested paths it
/// names, under `mergeProps` / `prependProps` / `deepMergeProps`.
///
/// Mirrors Laravel's `collectMergeableMetadata`
/// (`inertia-laravel-3.5.1/src/PropsResolver.php`): a deep merge always
/// emits the bare key, since it recurses into every field already; a
/// prop that names any path never also merges its whole value
/// (`MergesProps::mergesAtRoot`); and the append and prepend path lists
/// are separate, so one prop can append at one path and prepend at
/// another. The paths [`Prop::merge_with_path`] names follow the prop's
/// root direction.
fn push_merge_paths(metadata: &mut PageMetadata, key: &str, prop: &Prop, mode: MergeMode) {
    let following = prop.merge_paths();
    let (append_following, prepend_following): (&[String], &[String]) = match mode {
        MergeMode::Deep => {
            metadata.deep_merge.push(key.to_string());
            return;
        }
        MergeMode::Append => (following, &[]),
        MergeMode::Prepend => (&[], following),
    };
    let appends = prop.append_paths().iter().chain(append_following);
    let prepends = prop.prepend_paths().iter().chain(prepend_following);
    if appends.clone().next().is_none() && prepends.clone().next().is_none() {
        match mode {
            MergeMode::Prepend => metadata.merge_prepend.push(key.to_string()),
            _ => metadata.merge.push(key.to_string()),
        }
        return;
    }
    metadata
        .merge
        .extend(appends.map(|path| format!("{key}.{path}")));
    metadata
        .merge_prepend
        .extend(prepends.map(|path| format!("{key}.{path}")));
}

/// The request `build_page_object_for_test` resolves against: a test of
/// the page object has no request, and its props read none.
#[cfg(test)]
struct TestRequest;

#[cfg(test)]
impl InertiaRequestExt for TestRequest {
    fn path(&self) -> &str {
        "/"
    }
    fn header(&self, _name: &str) -> Option<&str> {
        None
    }
}

/// Report a rescued deferred prop's failure - the log line and the
/// best-effort `ErrorOccurred` event - and the outcome that leaves its key
/// out of `props` and lists it under `rescuedProps`. Used by the scroll
/// loader arm of `resolve_props`; the resolver arm reports the same way.
fn rescued_outcome(key: String, e: FrameworkError) -> TaskOutcome {
    let logged = crate::error::render_error_chain(&e);
    tracing::warn!(
        prop_key = %key,
        error = %logged,
        "inertia deferred prop resolver failed; rescued per spec",
    );
    let evt = crate::events::ErrorOccurred {
        error_message: logged,
        status_code: 500,
        request_id: crate::logging::current_request_id().map(|id| id.as_str().to_string()),
    };
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.spawn(async move {
            let _ = crate::events::EventFacade::dispatch(evt).await;
        });
    }
    TaskOutcome::Rescued { key }
}

/// The largest integer JavaScript represents exactly, `2^53 - 1`.
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

/// Replace every integer in `values` beyond plus or minus
/// [`MAX_SAFE_INTEGER`], at any depth, with `{"$bigint": "<digits>"}`, the
/// marker the Inertia client restores as a `BigInt`. Laravel's
/// `PreservesBigIntegers::encodeBigIntegers`. Floats are not integers and
/// stay as they are, as do object keys.
fn encode_big_integers_in(values: &mut serde_json::Map<String, Value>) {
    for value in values.values_mut() {
        encode_big_integers(value);
    }
}

/// [`encode_big_integers_in`] for one value, in place.
fn encode_big_integers(value: &mut Value) {
    match value {
        Value::Number(number) => {
            let beyond = match (number.as_u64(), number.as_i64()) {
                (Some(unsigned), _) => unsigned > MAX_SAFE_INTEGER,
                (None, Some(signed)) => signed.unsigned_abs() > MAX_SAFE_INTEGER,
                (None, None) => false,
            };
            if beyond {
                let mut marker = serde_json::Map::with_capacity(1);
                marker.insert("$bigint".to_string(), Value::String(number.to_string()));
                *value = Value::Object(marker);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(encode_big_integers),
        Value::Object(map) => map.values_mut().for_each(encode_big_integers),
        Value::Null | Value::Bool(_) | Value::String(_) => {}
    }
}

/// Whether Laravel's resolver would walk this prop's value as a literal,
/// so dotted `only`/`except` entries narrow it.
///
/// A flag-free prop with a materialized value is a literal array in
/// Laravel's terms. Every flag makes it a prop object (`AlwaysProp`,
/// `OptionalProp`, `DeferProp`, `MergeProp`, `OnceProp`, `ScrollProp`),
/// whose resolved value ships whole. A flag-free resolver ships whole too,
/// except under a dotted key: Laravel's `unpackDotProps` calls a dotted
/// key's closure before the walk, which leaves its value a literal.
fn narrows_as_literal(key: &str, prop: &Prop) -> bool {
    let flag_free = prop.visibility() == Visibility::Standard
        && prop.merge_mode().is_none()
        && !prop.is_once()
        && prop.scroll_metadata().is_none();
    flag_free && (prop.as_value().is_some() || (prop.has_resolver() && key.contains('.')))
}

fn build_page_object(
    component: &str,
    resolved: ResolvedProps,
    config: &InertiaConfig,
    url: String,
    flash: serde_json::Map<String, Value>,
    flags: PageObjectFlags,
    shared_keys: Vec<String>,
) -> Value {
    let ResolvedProps {
        props: materialized_props,
        metadata,
    } = resolved;
    let PageObjectFlags {
        encrypt_history,
        clear_history,
        preserve_fragment,
        preserve_big_integers,
    } = flags;
    let mut page = serde_json::Map::new();
    page.insert(
        "component".to_string(),
        Value::String(component.to_string()),
    );
    page.insert("props".to_string(), Value::Object(materialized_props));
    page.insert("url".to_string(), Value::String(url));
    page.insert(
        "version".to_string(),
        Value::String(config.resolved_version()),
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
    if preserve_big_integers {
        page.insert("preserveBigIntegers".to_string(), Value::Bool(true));
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
/// in the value's type - a hand-written custom `Serialize`
/// implementation returning `Err` is the only path that triggers this.
///
/// The panic is caught by the request-level panic-recovery middleware
/// (`framework/src/middleware/chain.rs`, Domain 2 M1) and converted to
/// a 500 response, so the process stays up. To handle serialization
/// failure explicitly - required off the HTTP path, where no panic net
/// exists - use the fallible sibling of the builder method
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
        reflash_session_values_after_eager_error(FrameworkError::internal(format!(
            "InertiaResponse prop `{key}` failed to serialize: {e} \
             (the value's Serialize impl returned Err)"
        )))
    })
}

/// The error a page that cannot be encoded answers with: a `500`, never a
/// `200` with an empty page, as Laravel's `JsonResponse` throws on an
/// encoding failure.
fn page_encoding_error(error: &serde_json::Error) -> FrameworkError {
    FrameworkError::internal(format!(
        "the Inertia page object could not be encoded as JSON: {error}"
    ))
}

/// The JSON page object an Inertia visit answers with.
///
/// Generic over the page so a test can hand it a value the encoder
/// refuses; the framework always passes the page `Value`.
fn build_json_response<P: Serialize + ?Sized>(page: &P) -> Result<HttpResponse, FrameworkError> {
    // Serialized from the borrowed page, the same bytes `HttpResponse::json`
    // writes, without first cloning the whole page to hand it over.
    let body = serde_json::to_vec(page).map_err(|error| page_encoding_error(&error))?;
    Ok(HttpResponse::bytes_body(body, "application/json")
        .header("X-Inertia", "true")
        .header("Vary", "X-Inertia"))
}

/// Writes JSON into a buffer with `/` backslash-escaped and `<` and `>`
/// written as `\u003c` and `\u003e`, so nothing inside a string field can
/// end or change the state of the page's `<script>` element.
///
/// `/` keeps a `</script>` from closing the element. `<` and `>` keep a
/// `<!--` or a `<script` from putting the HTML tokenizer into the escaped
/// script states, where the real `</script>` no longer closes the element
/// and the mount element after it becomes script text: what Laravel's
/// `JSON_HEX_TAG` and Inertia 3.7.1's initial page JSON prevent. (The name
/// is the first escape it wrote.)
///
/// The three characters are not JSON syntax, so they only occur inside
/// strings, where both escapes are valid JSON. Escaping byte by byte is
/// sound: in UTF-8 the bytes `0x2F`, `0x3C` and `0x3E` only ever encode
/// those characters themselves, never part of a longer character. The
/// escapes go straight into the one document buffer, with no copy of the
/// page (MEM-003).
struct SlashEscaping<'a>(&'a mut Vec<u8>);

impl std::io::Write for SlashEscaping<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let mut rest = bytes;
        while let Some(at) = rest
            .iter()
            .position(|byte| matches!(byte, b'/' | b'<' | b'>'))
        {
            let escape: &[u8] = match rest[at] {
                b'<' => b"\\u003c",
                b'>' => b"\\u003e",
                _ => b"\\/",
            };
            self.0.extend_from_slice(&rest[..at]);
            self.0.extend_from_slice(escape);
            rest = &rest[at + 1..];
        }
        self.0.extend_from_slice(rest);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The first-visit HTML document, the page JSON written into its
/// `<script>` element.
///
/// Generic over the page for the same reason as [`build_json_response`].
/// A page that cannot be encoded is an error, not a document with an
/// empty page.
fn build_html_response<P: Serialize + ?Sized>(
    page: &P,
    config: &InertiaConfig,
    title_override: Option<&str>,
    ssr: Option<&super::ssr::SsrResponse>,
) -> Result<HttpResponse, FrameworkError> {
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
    //   inject it raw - wrapping it in another `<div id="app">` would
    //   produce duplicate IDs and break hydration.
    // - Non-SSR path: we emit the same shape ourselves with an empty
    //   mount div. Inside the script tag the JSON is raw (NOT
    //   HTML-attribute-encoded): every `/` is backslash-escaped so a
    //   literal `</script>` substring inside a string field can't
    //   terminate the tag, and `<` and `>` are `\u003c` and `\u003e` so a
    //   `<!--<script>` can't stop the real end tag from closing it - this
    //   matches `buildSSRBody`'s escape and Laravel's `JSON_HEX_TAG`.
    let ssr_head = ssr.map(|s| s.head.join("\n")).unwrap_or_default();

    // A page that renders its own `<title>` through Inertia's `Head`
    // component sends it back in the SSR head, which is injected verbatim
    // below. Emitting the default as well leaves the document with two
    // `<title>` elements and the generic one first - and first is the one
    // browsers, crawlers and the pre-hydration tab read, so the page's
    // real title would never be seen. The page's own head wins, which is
    // exactly what `default_title` documents itself as: a default.
    let title_line = !contains_title_element(&ssr_head);

    // The document is written into one buffer, the page JSON serialized
    // straight into it. Serializing to a string, escaping that into a
    // second, and formatting both into a third and a fourth made an
    // initial visit allocate its page four times over.
    let mut html = String::new();
    html.push_str("<!DOCTYPE html>\n<html lang=\"");
    html.push_str(&document_language_attr());
    html.push_str(
        "\">\n<head>\n<meta charset=\"UTF-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\">\n\
         <meta name=\"csrf-token\" content=\"",
    );
    html.push_str(&csrf_attr);
    html.push_str("\">\n");
    if title_line {
        html.push_str("<title>");
        html.push_str(&title_html);
        html.push_str("</title>\n");
    }
    html.push_str(&ssr_head);
    html.push_str(&head_extras);
    html.push_str("</head>\n<body>\n");
    let html = if let Some(ssr) = ssr {
        html.push_str(&ssr.body);
        html.push_str("\n</body>\n</html>");
        html
    } else {
        let mount_id = escape_html_attr(&config.mount_id);
        let mut html = html.into_bytes();
        html.extend_from_slice(b"<script type=\"application/json\" data-page=\"");
        html.extend_from_slice(mount_id.as_bytes());
        html.extend_from_slice(b"\">");
        serde_json::to_writer(SlashEscaping(&mut html), page)
            .map_err(|error| page_encoding_error(&error))?;
        html.extend_from_slice(b"</script>\n<div id=\"");
        html.extend_from_slice(mount_id.as_bytes());
        html.extend_from_slice(b"\"></div>\n</body>\n</html>");
        // Only UTF-8 was written: the JSON serializer's output and ASCII.
        String::from_utf8(html)
            .unwrap_or_else(|invalid| String::from_utf8_lossy(invalid.as_bytes()).into_owned())
    };

    Ok(HttpResponse::html(html).header("Vary", "X-Inertia"))
}

/// The first visit through the application's root template (RDOC-001).
///
/// The template gets the values the framework's own document is built
/// from, as parts it places: the same title rule, CSRF token, SSR output,
/// Vite tags, language and mount id, and the response's view data. The
/// page JSON is written into the template's output by the body part, never
/// into a string of its own.
fn build_template_response(
    template: super::root_template::ApplicationTemplate,
    page: &Value,
    config: &InertiaConfig,
    title_override: Option<&str>,
    ssr: Option<&super::ssr::SsrResponse>,
    view_data: &super::root_template::InertiaViewData,
) -> Result<HttpResponse, FrameworkError> {
    let csrf = csrf_token().unwrap_or_default();
    let ssr_head = ssr.map(|s| s.head.join("\n")).unwrap_or_default();
    let title = (!contains_title_element(&ssr_head))
        .then(|| title_override.unwrap_or(config.default_title.as_str()));
    let assets = if config.development {
        render_dev_head(config)
    } else {
        render_prod_head(config)
    };
    let lang = document_language();
    super::root_template::render(
        template,
        super::root_template::RootInputs {
            page,
            title,
            csrf_token: &csrf,
            ssr_head: &ssr_head,
            ssr_body: ssr.map(|s| s.body.as_str()),
            assets: &assets,
            lang: &lang,
            mount_id: &config.mount_id,
            view: view_data,
        },
    )
}

/// Whether an SSR head fragment already carries a `<title>` element.
///
/// A substring test rather than a parse, deliberately: the head is
/// injected into the document verbatim either way, and the only decision
/// it feeds is whether the framework adds its own default title on top.
/// Two narrowings keep the cheap test from costing the document its only
/// title:
///
/// - The opening tag has to be followed by `>` or whitespace, so a custom
///   element - `<title-bar>` - is not mistaken for a title.
/// - HTML comments are removed first, so a `<title>` somebody commented
///   out cannot suppress the default and leave the document with none.
///
/// One false positive remains by construction: a literal `<title>` inside
/// an attribute value, as in `<meta content="&lt;title&gt;">` written
/// unescaped. Inertia's `Head` escapes attribute content, so reaching it
/// means hand-building a head string that is already invalid markup, and
/// the cost is a missing default title rather than a broken document. A
/// real parse is not worth carrying for that.
fn contains_title_element(head: &str) -> bool {
    const TAG: &str = "<title";
    let lower = strip_html_comments(&head.to_ascii_lowercase());
    lower.match_indices(TAG).any(|(at, _)| {
        lower[at + TAG.len()..]
            .chars()
            .next()
            .is_some_and(|c| c == '>' || c.is_whitespace())
    })
}

/// `head` with every `<!-- ... -->` run removed. An unterminated comment
/// swallows the rest of the input, which is what a browser does with one
/// too.
fn strip_html_comments(head: &str) -> String {
    let mut out = String::with_capacity(head.len());
    let mut rest = head;
    while let Some(open) = rest.find("<!--") {
        out.push_str(&rest[..open]);
        match rest[open + 4..].find("-->") {
            Some(close) => rest = &rest[open + 4 + close + 3..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// The `<html lang>` attribute value, HTML-escaped only if it needs to be.
///
/// A BCP 47 tag is letters, digits and hyphens, so the escape is a no-op
/// for every value that reaches here from a parsed
/// [`Locale`](crate::Locale). The check is still made rather than assumed:
/// with the `localization` feature off this is a constant, but with it on
/// the value comes from whatever the request negotiated, and an attribute
/// written into the shell unescaped on the strength of an assumption is
/// how injection bugs are shaped. Escaping costs an allocation, so it is
/// paid only when a character actually needs it.
fn document_language_attr() -> Cow<'static, str> {
    let lang = document_language();
    if lang.contains(['&', '<', '>', '"', '\'']) {
        Cow::Owned(escape_html_attr(&lang))
    } else {
        lang
    }
}

/// The BCP 47 language the document shell declares in `<html lang>`.
///
/// Screen readers pick their voice from this attribute and search engines
/// take it as the page's language signal, so a document serving Japanese
/// prose while declaring English is wrong in a way no amount of correct
/// translation fixes. It follows the locale in effect for the request -
/// the task-local scope [`LocaleMiddleware`](crate::LocaleMiddleware)
/// opens, then a process-global override, then the configured default -
/// which is what makes the value right for an error page rendered on the
/// way out as much as for a handler's page.
///
/// The value keeps the casing [`Locale`](crate::Locale) renders
/// (`pt-BR`, `zh-Hans`) rather than being lowercased: `:lang()` selectors
/// and font stacks are written in that form.
///
/// `Lang::locale()` is one allocation for the rendered tag, plus - only
/// when no locale scope and no global override are in play - a clone of
/// the resolved `LocalizationConfig` that it takes `default_locale` out
/// of. Avoiding that clone would mean a new cheaper entry point on the
/// localization module rather than anything this shell can do, so it is
/// left alone here.
#[cfg(feature = "localization")]
fn document_language() -> Cow<'static, str> {
    Cow::Owned(crate::localization::Lang::locale().as_str())
}

/// Without the `localization` feature there is no per-request locale to
/// follow, so the shell keeps the `en` it has always declared.
#[cfg(not(feature = "localization"))]
fn document_language() -> Cow<'static, str> {
    Cow::Borrowed("en")
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
    let base = asset_base(&config.assets_base_url);
    let entry = &config.entry_point;
    let url = |file: &str| escape_html_attr(&format!("{base}/{file}"));
    if let Some(assets) = config.vite_manifest().and_then(|m| m.resolve_entry(entry)) {
        let mut out = String::new();
        for css in &assets.css {
            out.push_str(&format!(
                "<link rel=\"stylesheet\" href=\"{}\">\n",
                url(css)
            ));
        }
        for js in &assets.js {
            out.push_str(&format!(
                "<script type=\"module\" src=\"{}\"></script>\n",
                url(js)
            ));
        }
        for chunk in &assets.preload {
            out.push_str(&format!(
                "<link rel=\"modulepreload\" href=\"{}\">\n",
                url(chunk)
            ));
        }
        out
    } else {
        // Manifest absent or entry not present - legacy fallback.
        format!(
            "<script type=\"module\" src=\"{}\"></script>\n\
             <link rel=\"stylesheet\" href=\"{}\">\n",
            url("main.js"),
            url("main.css")
        )
    }
}

/// The page object's `url`: the request's path and query, or what the
/// application's resolver derives, carrying the public root (PFX-005).
///
/// The default derivation is an application path and always gets the root;
/// a resolver's root-relative path gets it unless it is already under the
/// root. Its query is normalised as Laravel's `fullUrl()` normalises it
/// through Symfony - pairs parsed, sorted by key, re-encoded per RFC 3986 -
/// so the client compares the same page URLs Laravel sends (PAR-056); a
/// resolver's URL is the application's own and is left as it returns it.
/// Either `String` is owned here and becomes the URL itself when it keeps
/// its bytes, as it does at the host root with a query already in that
/// form, so the first page does not copy its URL a second time (MEM-003).
fn page_url(resolver: Option<&super::config::UrlResolver>, req: &dyn InertiaRequestExt) -> String {
    match resolver {
        Some(resolve_url) => crate::routing::root::rooted_owned(resolve_url(req)),
        None => crate::routing::root::prefixed_owned(
            super::query_string::normalize_path_and_query(req.path_and_query()),
        ),
    }
}

/// The base the Vite tags name their files under (PFX-005).
///
/// A root-relative `assets_base_url`, one that starts with a single `/`
/// such as the default `/assets`, is served by the application and gets
/// the public root in front. An absolute or network-path base (a CDN) is
/// another host's path and is left as it is.
///
/// The base is classified before its trailing slashes are removed, so a
/// base of `/`, which serves the assets at the root itself, is root-relative
/// too and gives the root alone.
fn asset_base(assets_base_url: &str) -> std::borrow::Cow<'_, str> {
    let trimmed = assets_base_url.trim_end_matches('/');
    if assets_base_url.starts_with('/') && !assets_base_url.starts_with("//") {
        std::borrow::Cow::Owned(crate::routing::root::prefixed(trimmed))
    } else {
        std::borrow::Cow::Borrowed(trimmed)
    }
}

/// Escape `s` for a double- or single-quoted HTML attribute value.
pub(crate) fn escape_html_attr(s: &str) -> String {
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

    /// A request whose path and query is one `String` the test built, so a
    /// test can tell the page URL that is this buffer from a copy of it.
    struct OwnedPathRequest {
        path_and_query: std::sync::Mutex<Option<String>>,
        address: usize,
    }

    impl OwnedPathRequest {
        fn new(path_and_query: String) -> Self {
            Self {
                address: path_and_query.as_ptr() as usize,
                path_and_query: std::sync::Mutex::new(Some(path_and_query)),
            }
        }
    }

    impl InertiaRequestExt for OwnedPathRequest {
        fn path(&self) -> &str {
            "/page"
        }

        fn path_and_query(&self) -> String {
            self.path_and_query
                .lock()
                .expect("the path lock")
                .take()
                .expect("the page reads its path and query once")
        }

        fn header(&self, _name: &str) -> Option<&str> {
            None
        }
    }

    /// MEM-003: at the host root the first page's URL is the path and query
    /// the request built, or the `String` the application's resolver
    /// returned, never a copy of it. Under a root the default URL is one new
    /// buffer, the root followed by the path.
    #[tokio::test]
    async fn mem_audit_the_first_page_url_is_the_path_the_request_built() {
        let path = format!("/page?q={}", "a".repeat(4_000));
        for root in ["", "/billing"] {
            let request = OwnedPathRequest::new(path.clone());
            let url =
                crate::routing::root::scope(Arc::from(root), async { page_url(None, &request) })
                    .await;
            assert_eq!(url, format!("{root}{path}"));
            if root.is_empty() {
                assert_eq!(
                    url.as_ptr() as usize,
                    request.address,
                    "the request's path and query was copied"
                );
            } else {
                assert_eq!(url.capacity(), url.len(), "one exact buffer");
            }
        }

        // A resolved URL that keeps its bytes, at the host root or already
        // under the root, is the URL itself.
        for (root, resolved, expected, kept) in [
            ("", "/resolved?q=1", "/resolved?q=1", true),
            (
                "/billing",
                "/billing/resolved?q=1",
                "/billing/resolved?q=1",
                true,
            ),
            ("/billing", "/resolved?q=1", "/billing/resolved?q=1", false),
        ] {
            let resolved = resolved.to_owned();
            let address = resolved.as_ptr() as usize;
            let slot = std::sync::Mutex::new(Some(resolved));
            let resolver: crate::inertia::config::UrlResolver = Arc::new(move |_request| {
                slot.lock()
                    .expect("the resolver lock")
                    .take()
                    .expect("the page resolves its URL once")
            });
            let request = OwnedPathRequest::new("/page".to_owned());
            let url = crate::routing::root::scope(Arc::from(root), async {
                page_url(Some(&resolver), &request)
            })
            .await;
            assert_eq!(url, expected);
            if kept {
                assert_eq!(
                    url.as_ptr() as usize,
                    address,
                    "the resolved URL was copied"
                );
            }
        }
    }

    #[test]
    fn a_commented_out_title_does_not_pass_for_one() {
        // The consequence of getting this wrong is a document with no
        // title at all: the framework stands its default down, and the
        // element it stood down for is inside a comment.
        assert!(!contains_title_element("<!-- <title>Old</title> -->"));
        assert!(!contains_title_element(
            "<meta name=\"a\" content=\"b\">\n<!--\n<title>Old</title>\n-->"
        ));
        // An unterminated comment swallows the rest, the way a browser
        // parses one.
        assert!(!contains_title_element("<!-- <title>Old</title>"));
        // A real title beside a commented-out one still counts.
        assert!(contains_title_element(
            "<!-- <title>Old</title> --><title>New</title>"
        ));
        // And the plain cases are unmoved.
        assert!(contains_title_element("<title>Hi</title>"));
        assert!(contains_title_element("<TITLE lang=\"en\">Hi</TITLE>"));
        assert!(!contains_title_element(
            "<title-bar data-x=\"1\"></title-bar>"
        ));
        assert!(!contains_title_element(""));
    }

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
        assert_eq!(obj["version"], Value::String(String::new()));

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
                match_on: Some(vec!["id".into()]),
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

    /// A page the JSON encoder refuses, standing in for any value it
    /// cannot write.
    struct Unencodable;

    impl Serialize for Unencodable {
        fn serialize<S: serde::Serializer>(&self, _serializer: S) -> Result<S::Ok, S::Error> {
            Err(serde::ser::Error::custom("this page cannot be encoded"))
        }
    }

    /// PAR-053 (JE-03): Laravel's `JsonResponse` throws on an encoding
    /// failure, so the client gets an error, never a `200` carrying `{}`.
    #[test]
    fn inp_a_page_that_cannot_be_encoded_answers_an_error_not_an_empty_page() {
        let Err(json) = build_json_response(&Unencodable) else {
            panic!("an Inertia visit must not get a 200 with an empty page");
        };
        assert_eq!(json.status_code(), 500);
        assert!(json.to_string().contains("cannot be encoded"), "{json}");

        let Err(html) = build_html_response(&Unencodable, &InertiaConfig::default(), None, None)
        else {
            panic!("a first visit must not get a 200 with an empty page");
        };
        assert_eq!(html.status_code(), 500);
    }
}
