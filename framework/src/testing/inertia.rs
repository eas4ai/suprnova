//! `AssertableInertia` - fluent assertions over an Inertia page object,
//! parsed from either an Inertia XHR response body or the `<script
//! type="application/json" data-page="app">` element embedded in a
//! hard-navigation HTML shell (see `framework/src/inertia/response.rs`
//! `build_json_response` / `build_html_response`). Laravel's
//! `Inertia\Testing\AssertableInertia` equivalent: assertions panic with
//! an expected/actual excerpt on failure - the same testing-surface
//! contract as [`crate::testing::TestResponse`] and
//! [`crate::testing::Expect`].
//!
//! Two ways to build one:
//! - [`AssertableInertia::from_response`] - works directly on a
//!   [`crate::HttpResponse`], the type `InertiaResponse::resolve`
//!   returns, for a test that drives the response pipeline without a
//!   socket. Handles both response shapes.
//! - [`crate::testing::TestResponse::assert_inertia`] - the entry point
//!   for a loopback-socket test already holding a
//!   [`crate::testing::TestResponse`]. It only handles the JSON shape (a
//!   real Inertia visit sends `X-Inertia: true` and gets JSON back), and
//!   panics with an actionable message if the response isn't one.
//!
//! ## Reloading for partial-reload / deferred-props assertions
//!
//! [`AssertableInertia::reload_only`],
//! [`reload_except`](AssertableInertia::reload_except), and
//! [`load_deferred_props`](AssertableInertia::load_deferred_props) mirror
//! Inertia's client-side partial reload: they build the request headers a
//! real follow-up XHR would send ([`ReloadRequest::headers`]) and replay
//! them. Unlike Laravel, where `ReloadRequest` reissues the request
//! through the same in-process PHP kernel the original test used,
//! Suprnova's HTTP tests cross a real hyper/TCP wire and every test file
//! owns its own `spawn_server` / `request` harness (see
//! `manual/http-tests.md`) - there is no single "the test client" to
//! reach for. So these methods carry no built-in transport: attach one
//! with [`AssertableInertia::with_reload`], a closure from a
//! [`ReloadRequest`] to a future producing the reloaded
//! [`AssertableInertia`], wired to whatever harness the test already
//! uses. Calling a reload method without one attached panics with that
//! instruction. See `manual/http-tests.md#testing-inertia-responses` for
//! a worked example.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::{Map, Value};

use crate::HttpResponse;

/// Closure that replays a [`ReloadRequest`] and returns the reloaded
/// page's assertions. See the module docs for why this is a
/// caller-supplied closure rather than a built-in HTTP client.
type Reloader = Arc<
    dyn Fn(ReloadRequest) -> Pin<Box<dyn Future<Output = AssertableInertia> + Send>> + Send + Sync,
>;

/// Fluent assertions over a parsed Inertia page object.
///
/// Build with [`from_response`](Self::from_response) or
/// [`crate::testing::TestResponse::assert_inertia`]. Every assertion
/// returns `&Self` and panics on failure - the same contract as
/// [`crate::testing::TestResponse`] and [`crate::testing::Expect`].
pub struct AssertableInertia {
    component: String,
    url: String,
    version: String,
    props: Value,
    flash: Value,
    deferred_props: Map<String, Value>,
    reload: Option<Reloader>,
}

impl AssertableInertia {
    /// Parse a page object out of an [`HttpResponse`] - the type
    /// [`crate::InertiaResponse::resolve`] returns.
    ///
    /// Handles both shapes a resolved Inertia response can take: when the
    /// response carries an `X-Inertia` header, the body is the JSON page
    /// object directly; otherwise the body is the HTML shell and the page
    /// object is read out of its `<script type="application/json"
    /// data-page="app">` element (a server-rendered/SSR body embeds a
    /// different shape via `buildSSRBody` and is not covered here - no
    /// test in this codebase asserts against one today).
    ///
    /// # Panics
    ///
    /// Panics if neither shape is found, if what's found isn't valid
    /// JSON, or if it's missing any of `component`, `props`, `url`,
    /// `version`. `encryptHistory` / `clearHistory` are not required -
    /// the page-object builder omits them rather than emitting `false`
    /// (`framework/src/inertia/response.rs` `build_page_object`).
    pub fn from_response(response: &HttpResponse) -> Self {
        let page = if response.header_value("X-Inertia").is_some() {
            serde_json::from_slice(response.body()).unwrap_or_else(|e| {
                panic!(
                    "AssertableInertia::from_response(...): X-Inertia response body is not \
                     valid JSON: {e}"
                )
            })
        } else {
            let html = String::from_utf8_lossy(response.body());
            page_object_from_html(&html).unwrap_or_else(|| {
                panic!(
                    "AssertableInertia::from_response(...): no Inertia page object found - no \
                     X-Inertia header and no <script type=\"application/json\" \
                     data-page=\"app\"> element in the body"
                )
            })
        };
        Self::from_page(page)
    }

    /// Build directly from an already-parsed page object [`Value`].
    /// [`crate::testing::TestResponse::assert_inertia`] uses this after
    /// parsing the response body itself, so the two entry points share
    /// one validation path.
    pub(crate) fn from_page(page: Value) -> Self {
        let Some(obj) = page.as_object() else {
            panic!("AssertableInertia: page object is not a JSON object: {page}");
        };
        for key in ["component", "props", "url", "version"] {
            if !obj.contains_key(key) {
                panic!("AssertableInertia: page object is missing required key `{key}`: {page}");
            }
        }
        let component = obj["component"]
            .as_str()
            .unwrap_or_else(|| panic!("AssertableInertia: `component` is not a string: {page}"))
            .to_string();
        let url = obj["url"]
            .as_str()
            .unwrap_or_else(|| panic!("AssertableInertia: `url` is not a string: {page}"))
            .to_string();
        let version = obj["version"]
            .as_str()
            .unwrap_or_else(|| panic!("AssertableInertia: `version` is not a string: {page}"))
            .to_string();
        let props = obj["props"].clone();
        let flash = obj
            .get("flash")
            .cloned()
            .unwrap_or_else(|| Value::Object(Map::new()));
        let deferred_props = obj
            .get("deferredProps")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        Self {
            component,
            url,
            version,
            props,
            flash,
            deferred_props,
            reload: None,
        }
    }

    /// Attach the closure [`Self::reload_only`], [`Self::reload_except`],
    /// and [`Self::load_deferred_props`] replay a [`ReloadRequest`]
    /// through. See the module docs.
    pub fn with_reload<F, Fut>(mut self, reloader: F) -> Self
    where
        F: Fn(ReloadRequest) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = AssertableInertia> + Send + 'static,
    {
        self.reload = Some(Arc::new(move |request| Box::pin(reloader(request))));
        self
    }

    /// Assert the page's component name.
    pub fn component(&self, expected: &str) -> &Self {
        if self.component != expected {
            panic!(
                "AssertableInertia::component({expected:?})\n  Expected: {expected:?}\n  \
                 Received: {:?}",
                self.component
            );
        }
        self
    }

    /// Assert the page's `url`.
    pub fn url(&self, expected: &str) -> &Self {
        if self.url != expected {
            panic!(
                "AssertableInertia::url({expected:?})\n  Expected: {expected:?}\n  Received: \
                 {:?}",
                self.url
            );
        }
        self
    }

    /// Assert the page's asset `version`. The default resolver hashes
    /// the Vite manifest, or falls back to
    /// [`crate::MANIFEST_VERSION_FALLBACK`] when none exists yet - pass
    /// that constant rather than hardcoding `"1.0"` in a test that
    /// hasn't built a frontend.
    pub fn version(&self, expected: &str) -> &Self {
        if self.version != expected {
            panic!(
                "AssertableInertia::version({expected:?})\n  Expected: {expected:?}\n  \
                 Received: {:?}",
                self.version
            );
        }
        self
    }

    /// Read the value at a dot-separated `path` into the page's `props`.
    /// A numeric segment indexes a JSON array (`"items.0.id"`); every
    /// other segment looks up an object key. Returns `Value::Null` for a
    /// path that doesn't resolve - use [`Self::has`] to assert presence.
    pub fn prop(&self, path: &str) -> Value {
        dot_path(&self.props, path).cloned().unwrap_or(Value::Null)
    }

    /// Assert a prop exists at `path`.
    pub fn has(&self, path: &str) -> &Self {
        if dot_path(&self.props, path).is_none() {
            panic!(
                "AssertableInertia::has({path:?})\n  prop not present\n  props: {}",
                self.props
            );
        }
        self
    }

    /// Assert no prop exists at `path`.
    pub fn missing(&self, path: &str) -> &Self {
        if dot_path(&self.props, path).is_some() {
            panic!(
                "AssertableInertia::missing({path:?})\n  prop unexpectedly present\n  props: {}",
                self.props
            );
        }
        self
    }

    /// Assert the prop at `path` equals `expected`.
    pub fn where_(&self, path: &str, expected: impl Into<Value>) -> &Self {
        let expected = expected.into();
        let actual = dot_path(&self.props, path);
        if actual != Some(&expected) {
            panic!(
                "AssertableInertia::where_({path:?}, ...)\n  Expected: {expected}\n  Received: \
                 {}",
                actual
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "<missing>".to_string())
            );
        }
        self
    }

    /// Assert the array prop at `path` has `expected` elements.
    pub fn count(&self, path: &str, expected: usize) -> &Self {
        let actual = dot_path(&self.props, path);
        let len = match actual {
            Some(Value::Array(items)) => Some(items.len()),
            _ => None,
        };
        if len != Some(expected) {
            panic!(
                "AssertableInertia::count({path:?}, {expected})\n  Expected: an array of \
                 length {expected}\n  Received: {}",
                actual
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "<missing or not an array>".to_string())
            );
        }
        self
    }

    /// Assert the page's `flash` data has `key`, optionally equal to
    /// `expected`. Pass `None::<serde_json::Value>` to check presence
    /// only. `key` follows the same dot-path rule as [`Self::prop`].
    pub fn has_flash<V: Into<Value>>(&self, key: &str, expected: Option<V>) -> &Self {
        let actual = dot_path(&self.flash, key);
        if actual.is_none() {
            panic!(
                "AssertableInertia::has_flash({key:?}, ...)\n  flash key not present\n  flash: \
                 {}",
                self.flash
            );
        }
        if let Some(expected) = expected {
            let expected = expected.into();
            if actual != Some(&expected) {
                panic!(
                    "AssertableInertia::has_flash({key:?}, Some(...))\n  Expected: \
                     {expected}\n  Received: {}",
                    actual.unwrap()
                );
            }
        }
        self
    }

    /// Replay this page as a partial reload requesting only `only`, and
    /// assert the reload landed on the same component/url/version and
    /// that every requested key is present.
    ///
    /// # Panics
    ///
    /// Panics if no reloader is attached (see [`Self::with_reload`]).
    pub async fn reload_only<I, S>(&self, only: I) -> AssertableInertia
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let only: Vec<String> = only.into_iter().map(Into::into).collect();
        let reloaded = self.replay(Some(only.clone()), None).await;
        reloaded.component(&self.component);
        reloaded.url(&self.url);
        reloaded.version(&self.version);
        for key in &only {
            reloaded.has(key);
        }
        reloaded
    }

    /// Replay this page as a partial reload excluding `except`, and
    /// assert none of the excluded keys are present in the reload.
    ///
    /// # Panics
    ///
    /// Panics if no reloader is attached (see [`Self::with_reload`]).
    pub async fn reload_except<I, S>(&self, except: I) -> AssertableInertia
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let except: Vec<String> = except.into_iter().map(Into::into).collect();
        let reloaded = self.replay(None, Some(except.clone())).await;
        reloaded.component(&self.component);
        reloaded.url(&self.url);
        reloaded.version(&self.version);
        for key in &except {
            reloaded.missing(key);
        }
        reloaded
    }

    /// Replay every group named in this page's `deferredProps` as one
    /// partial reload - the follow-up XHR the Inertia client issues
    /// right after the initial visit to resolve every deferred prop at
    /// once.
    ///
    /// # Panics
    ///
    /// Panics if no reloader is attached (see [`Self::with_reload`]).
    pub async fn load_deferred_props(&self) -> AssertableInertia {
        let keys: Vec<String> = self
            .deferred_props
            .values()
            .filter_map(Value::as_array)
            .flatten()
            .filter_map(|k| k.as_str().map(str::to_string))
            .collect();
        self.reload_only(keys).await
    }

    async fn replay(
        &self,
        only: Option<Vec<String>>,
        except: Option<Vec<String>>,
    ) -> AssertableInertia {
        let Some(reload) = self.reload.clone() else {
            panic!(
                "AssertableInertia::reload_only/reload_except/load_deferred_props: no reloader \
                 attached - call `.with_reload(...)` first; see \
                 manual/http-tests.md#testing-inertia-responses"
            );
        };
        let request = ReloadRequest {
            url: self.url.clone(),
            component: self.component.clone(),
            version: self.version.clone(),
            only,
            except,
        };
        let mut reloaded = reload(request).await;
        // Carry the same reloader forward so a further reload off the
        // result doesn't need `.with_reload(...)` reattached.
        reloaded.reload = Some(reload);
        reloaded
    }
}

/// Resolve a dot-separated `path` against `root`. A numeric segment
/// indexes into a JSON array; every other segment looks up an object
/// key.
fn dot_path<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = root;
    for segment in path.split('.') {
        current = match current {
            Value::Object(map) => map.get(segment)?,
            Value::Array(items) => items.get(segment.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(current)
}

/// Extract the JSON page object from a hard-navigation HTML shell's
/// `<script type="application/json" data-page="app">` element. The
/// element's content is standard JSON with every `/` escaped as `\/`
/// (`framework/src/inertia/response.rs` `build_html_response`) - a
/// valid JSON escape `serde_json` parses natively, so no unescaping is
/// needed.
fn page_object_from_html(html: &str) -> Option<Value> {
    const OPEN: &str = r#"<script type="application/json" data-page="app">"#;
    let start = html.find(OPEN)? + OPEN.len();
    let end = html[start..].find("</script>")? + start;
    serde_json::from_str(&html[start..end]).ok()
}

/// A recorded partial-reload request, built by
/// [`AssertableInertia::reload_only`],
/// [`AssertableInertia::reload_except`], and
/// [`AssertableInertia::load_deferred_props`] and handed to the closure
/// attached with [`AssertableInertia::with_reload`]. Mirrors what the
/// Inertia client sends on a follow-up XHR against the same page.
#[derive(Debug, Clone)]
pub struct ReloadRequest {
    /// The page's URL - the same request path (and query) to reissue.
    pub url: String,
    /// The page's component name, sent as `X-Inertia-Partial-Component`
    /// whenever [`Self::only`] or [`Self::except`] is set.
    pub component: String,
    /// The page's asset version, sent as `X-Inertia-Version`.
    pub version: String,
    /// Prop keys to request, sent as `X-Inertia-Partial-Data` (comma
    /// joined). `None` when this reload has no whitelist.
    pub only: Option<Vec<String>>,
    /// Prop keys to exclude, sent as `X-Inertia-Partial-Except` (comma
    /// joined). `None` when this reload has no blacklist.
    pub except: Option<Vec<String>>,
}

impl ReloadRequest {
    /// The header pairs a real Inertia partial reload sends: always
    /// `X-Inertia: true` and `X-Inertia-Version`, plus
    /// `X-Inertia-Partial-Component` and `X-Inertia-Partial-Data` /
    /// `X-Inertia-Partial-Except` whenever [`Self::only`] /
    /// [`Self::except`] is set. Feed these into whatever harness sends
    /// the replayed request - see
    /// `manual/http-tests.md#testing-inertia-responses`.
    pub fn headers(&self) -> Vec<(String, String)> {
        let mut headers = vec![
            ("X-Inertia".to_string(), "true".to_string()),
            ("X-Inertia-Version".to_string(), self.version.clone()),
        ];
        if self.only.is_some() || self.except.is_some() {
            headers.push((
                "X-Inertia-Partial-Component".to_string(),
                self.component.clone(),
            ));
        }
        if let Some(only) = &self.only {
            headers.push(("X-Inertia-Partial-Data".to_string(), only.join(",")));
        }
        if let Some(except) = &self.except {
            headers.push(("X-Inertia-Partial-Except".to_string(), except.join(",")));
        }
        headers
    }
}
