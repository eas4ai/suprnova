//! `AssertableInertia` - fluent assertions over an Inertia page object,
//! parsed from either an Inertia XHR response body or the `<script
//! type="application/json" data-page="...">` element embedded in a
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
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{Map, Value};

use super::response::fail_with_report;
use crate::{ErrorReport, HttpResponse};

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
///
/// Unlike `TestResponse`, whose assertions are all `assert_*`-prefixed,
/// this type's method names (`component`, `has`, `missing`, `where_`,
/// `count`, `has_flash`, ...) drop the prefix entirely - matching
/// Laravel's `Inertia\Testing\AssertableInertia`, whose equivalent
/// methods are bare the same way. The contract doesn't change with the
/// name: every method here still panics on failure exactly like its
/// `assert_*`-prefixed siblings.
///
/// Built from a response that carries an
/// [`ErrorReport`] - an Inertia error page that replaced a failed
/// request's `500`, most often - every failing assertion ends with that
/// report, the same way [`crate::testing::TestResponse`]'s do.
pub struct AssertableInertia {
    component: String,
    url: String,
    version: String,
    props: Value,
    flash: Value,
    deferred_props: Map<String, Value>,
    reload: Option<Reloader>,
    report: Option<ErrorReport>,
    /// The dotted path from the page's props to this scope's value;
    /// `None` at the root. Prefixes every path a failure names.
    scope: Option<String>,
    /// The keys of `props` an assertion touched, for the check a scope runs
    /// when its callback returns ([`AssertableInertia::scope`]). Behind a
    /// lock because the assertions take `&self`, and the page is `Sync`.
    interacted: Mutex<Vec<String>>,
}

impl AssertableInertia {
    /// Parse a page object out of an [`HttpResponse`] - the type
    /// [`crate::InertiaResponse::resolve`] returns.
    ///
    /// Handles both shapes a resolved Inertia response can take: when the
    /// response carries an `X-Inertia` header, the body is the JSON page
    /// object directly; otherwise the body is the HTML shell and the page
    /// object is read out of its first `<script type="application/json"
    /// data-page="...">` element, whatever id
    /// [`InertiaConfig::mount_id`](crate::InertiaConfig::mount_id) gave it
    /// (a server-rendered/SSR body embeds a different shape via
    /// `buildSSRBody` and is not covered here - no test in this codebase
    /// asserts against one today).
    ///
    /// # Panics
    ///
    /// Panics if neither shape is found, if what's found isn't valid
    /// JSON, or if it's missing any of `component`, `props`, `url`,
    /// `version`. `encryptHistory` / `clearHistory` are not required -
    /// the page-object builder omits them rather than emitting `false`
    /// (`framework/src/inertia/response.rs` `build_page_object`).
    pub fn from_response(response: &HttpResponse) -> Self {
        let report = response.error_report().cloned();
        let page = if response.header_value("X-Inertia").is_some() {
            serde_json::from_slice(response.body()).unwrap_or_else(|e| {
                fail_with_report(
                    format!(
                        "AssertableInertia::from_response(...): X-Inertia response body is \
                         not valid JSON: {e}"
                    ),
                    report.as_ref(),
                )
            })
        } else {
            let html = String::from_utf8_lossy(response.body());
            match page_object_from_html(&html) {
                Some(Ok(page)) => page,
                Some(Err(e)) => fail_with_report(
                    format!(
                        "AssertableInertia::from_response(...): found the <script \
                         type=\"application/json\" data-page=...> element, but its content \
                         is not valid JSON: {e}"
                    ),
                    report.as_ref(),
                ),
                None => fail_with_report(
                    "AssertableInertia::from_response(...): no Inertia page object found - no \
                     X-Inertia header and no <script type=\"application/json\" \
                     data-page=...> element in the body"
                        .to_string(),
                    report.as_ref(),
                ),
            }
        };
        Self::from_page(page, report)
    }

    /// Build directly from an already-parsed page object [`Value`] and
    /// the error report of the response it came from.
    /// [`crate::testing::TestResponse::assert_inertia`] uses this after
    /// parsing the response body itself, so the two entry points share
    /// one validation path.
    pub(crate) fn from_page(page: Value, report: Option<ErrorReport>) -> Self {
        let fail = |message: String| -> ! { fail_with_report(message, report.as_ref()) };
        let Some(obj) = page.as_object() else {
            fail(format!(
                "AssertableInertia: page object is not a JSON object: {page}"
            ));
        };
        for key in ["component", "props", "url", "version"] {
            if !obj.contains_key(key) {
                fail(format!(
                    "AssertableInertia: page object is missing required key `{key}`: {page}"
                ));
            }
        }
        let component = obj["component"]
            .as_str()
            .unwrap_or_else(|| {
                fail(format!(
                    "AssertableInertia: `component` is not a string: {page}"
                ))
            })
            .to_string();
        let url = obj["url"]
            .as_str()
            .unwrap_or_else(|| fail(format!("AssertableInertia: `url` is not a string: {page}")))
            .to_string();
        let version = obj["version"]
            .as_str()
            .unwrap_or_else(|| {
                fail(format!(
                    "AssertableInertia: `version` is not a string: {page}"
                ))
            })
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
            report,
            scope: None,
            interacted: Mutex::new(Vec::new()),
        }
    }

    /// Fail an assertion with `message`, followed by the error report of
    /// the response this page came from, when it carries one.
    fn fail(&self, message: String) -> ! {
        fail_with_report(message, self.report.as_ref())
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

    /// Assert the page's component name, and that the component has a page
    /// file.
    ///
    /// The file check runs when an Inertia configuration is installed on
    /// the active container ([`crate::Inertia::install`]) with
    /// [`testing_ensure_pages_exist`](crate::InertiaConfig::testing_ensure_pages_exist)
    /// on, which it is by default: the component must have a file under
    /// its [`pages_dir`](crate::InertiaConfig::pages_dir) with one of its
    /// [`page_extensions`](crate::InertiaConfig::page_extensions), the
    /// lookup [`ensure_pages_exist`](crate::InertiaConfig::ensure_pages_exist)
    /// does at render time. A test asserting a component nobody built would
    /// otherwise pass. With no configuration installed there is no
    /// directory to look in, and no check. Laravel's `component($value)`.
    pub fn component(&self, expected: &str) -> &Self {
        self.assert_component("component", expected, None)
    }

    /// Assert the page's component name, with the page-file check of
    /// [`Self::component`] forced on (`true`) or off (`false`) for this
    /// call whatever the configuration says. Forced on with no
    /// configuration installed, the check looks in the default
    /// `frontend/src/pages`. Laravel's `component($value, $shouldExist)`.
    pub fn component_exists(&self, expected: &str, should_exist: bool) -> &Self {
        self.assert_component("component_exists", expected, Some(should_exist))
    }

    /// The component assertion behind [`Self::component`] and
    /// [`Self::component_exists`]: the name, then the page file when
    /// `should_exist` (or, when `None`, the installed configuration) asks
    /// for it. `method` names the assertion in a failure.
    fn assert_component(&self, method: &str, expected: &str, should_exist: Option<bool>) -> &Self {
        if self.component != expected {
            self.fail(format!(
                "AssertableInertia::{method}({expected:?})\n  Expected: {expected:?}\n  \
                 Received: {:?}",
                self.component
            ));
        }
        let installed = crate::App::inertia_registry().installed_config();
        let config = match (should_exist, installed) {
            (Some(false), _) => return self,
            (Some(true), installed) => installed.unwrap_or_default(),
            (None, Some(config)) if config.testing_ensure_pages_exist => config,
            (None, _) => return self,
        };
        if crate::inertia::ensure_page_exists(&config, expected).is_err() {
            self.fail(format!(
                "AssertableInertia::{method}({expected:?})\n  Inertia page component file \
                 [{expected}] does not exist.\n  Looked for: {expected}.{{{}}} under {}\n  \
                 Create the page, fix the name, or turn the check off with \
                 InertiaConfig::testing_ensure_pages_exist(false) (or for this assertion \
                 with component_exists({expected:?}, false)).",
                config.page_extensions.join(","),
                config.pages_dir.display()
            ));
        }
        self
    }

    /// Assert the page's `url`.
    pub fn url(&self, expected: &str) -> &Self {
        if self.url != expected {
            self.fail(format!(
                "AssertableInertia::url({expected:?})\n  Expected: {expected:?}\n  Received: \
                 {:?}",
                self.url
            ));
        }
        self
    }

    /// Assert the page's asset `version`. The default resolver hashes
    /// the configured asset URL or the Vite manifest, and the version is
    /// the empty string when neither exists, as in a test that hasn't
    /// built a frontend.
    pub fn version(&self, expected: &str) -> &Self {
        if self.version != expected {
            self.fail(format!(
                "AssertableInertia::version({expected:?})\n  Expected: {expected:?}\n  \
                 Received: {:?}",
                self.version
            ));
        }
        self
    }

    /// Read the value at a dot-separated `path` into the page's `props`
    /// (into this scope's value, inside a scope). A numeric segment indexes
    /// a JSON array (`"items.0.id"`); every other segment looks up an
    /// object key. Returns `Value::Null` for a path that doesn't resolve -
    /// use [`Self::has`] to assert presence. Reading is not an assertion,
    /// so it does not count as touching the prop for a scope's check.
    pub fn prop(&self, path: &str) -> Value {
        dot_path(&self.props, path).cloned().unwrap_or(Value::Null)
    }

    /// Assert a prop exists at `path`.
    pub fn has(&self, path: &str) -> &Self {
        self.present("has", path);
        self
    }

    /// Assert a prop exists at every one of `paths`. Laravel's `hasAll`.
    pub fn has_all<I, S>(&self, paths: I) -> &Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        for path in paths {
            self.present("has_all", path.as_ref());
        }
        self
    }

    /// Assert a prop exists at one of `paths` at least. Every path counts
    /// as touched for a scope's check, present or not. Laravel's `hasAny`.
    pub fn has_any<I, S>(&self, paths: I) -> &Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let paths: Vec<String> = paths.into_iter().map(|p| p.as_ref().to_string()).collect();
        for path in &paths {
            self.interacts_with(path);
        }
        if !paths
            .iter()
            .any(|path| dot_path(&self.props, path).is_some())
        {
            let full: Vec<String> = paths.iter().map(|path| self.path_of(path)).collect();
            self.fail(format!(
                "AssertableInertia::has_any({full:?})\n  none of the props is present\n  props: \
                 {}",
                self.props
            ));
        }
        self
    }

    /// Assert no prop exists at `path`.
    pub fn missing(&self, path: &str) -> &Self {
        if dot_path(&self.props, path).is_some() {
            self.fail(format!(
                "AssertableInertia::missing({:?})\n  prop unexpectedly present\n  props: {}",
                self.path_of(path),
                self.props
            ));
        }
        self
    }

    /// Assert no prop exists at any of `paths`. Laravel's `missingAll`.
    pub fn missing_all<I, S>(&self, paths: I) -> &Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        for path in paths {
            self.missing(path.as_ref());
        }
        self
    }

    /// Assert the prop at `path` equals `expected`.
    pub fn where_(&self, path: &str, expected: impl Into<Value>) -> &Self {
        self.interacts_with(path);
        let expected = expected.into();
        let actual = dot_path(&self.props, path);
        if actual != Some(&expected) {
            self.fail(format!(
                "AssertableInertia::where_({:?}, ...)\n  Expected: {expected}\n  Received: {}",
                self.path_of(path),
                shown(actual)
            ));
        }
        self
    }

    /// Assert the prop at `path` exists and does not equal `value`.
    /// Laravel's `whereNot`.
    pub fn where_not(&self, path: &str, value: impl Into<Value>) -> &Self {
        let actual = self.present("where_not", path);
        let value = value.into();
        if *actual == value {
            self.fail(format!(
                "AssertableInertia::where_not({:?}, ...)\n  Expected anything but: {value}\n  \
                 Received: {actual}",
                self.path_of(path)
            ));
        }
        self
    }

    /// Assert the prop at `path` exists and is `null`. Laravel's
    /// `whereNull`.
    pub fn where_null(&self, path: &str) -> &Self {
        let actual = self.present("where_null", path);
        if !actual.is_null() {
            self.fail(format!(
                "AssertableInertia::where_null({:?})\n  Expected: null\n  Received: {actual}",
                self.path_of(path)
            ));
        }
        self
    }

    /// Assert the prop at `path` exists and is not `null`. Laravel's
    /// `whereNotNull`.
    pub fn where_not_null(&self, path: &str) -> &Self {
        let actual = self.present("where_not_null", path);
        if actual.is_null() {
            self.fail(format!(
                "AssertableInertia::where_not_null({:?})\n  Expected: anything but null\n  \
                 Received: null",
                self.path_of(path)
            ));
        }
        self
    }

    /// Assert every `(path, expected)` pair as [`Self::where_`] does. Takes
    /// pairs or a `serde_json::Map`. Laravel's `whereAll`.
    pub fn where_all<I, K, V>(&self, pairs: I) -> &Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: Into<Value>,
    {
        for (path, expected) in pairs {
            self.where_(path.as_ref(), expected);
        }
        self
    }

    /// Assert the prop at `path` exists and has one of the JSON types
    /// `types` names, alternatives joined by `|` (`"integer|null"`).
    ///
    /// The names are the ones Laravel's `whereType` compares PHP's
    /// `gettype` against: `string`, `integer` (a number without a
    /// fraction), `double` (a number with one), `boolean`, `array` (a JSON
    /// array or object, both PHP arrays) and `null`. Any other name fails
    /// the assertion, so a typo cannot pass by matching nothing.
    pub fn where_type(&self, path: &str, types: &str) -> &Self {
        let actual = self.present("where_type", path);
        let full = self.path_of(path);
        let mut matched = false;
        for name in types.split('|') {
            let Some(is) = json_type_is(name) else {
                self.fail(format!(
                    "AssertableInertia::where_type({full:?}, {types:?})\n  unknown type {name:?} \
                     - expected string, integer, double, boolean, array or null, joined by |"
                ));
            };
            matched |= is(actual);
        }
        if !matched {
            self.fail(format!(
                "AssertableInertia::where_type({full:?}, {types:?})\n  Property [{full}] is not \
                 of expected type [{types}].\n  Received: {actual}"
            ));
        }
        self
    }

    /// Assert every `(path, types)` pair as [`Self::where_type`] does.
    /// Laravel's `whereAllType`.
    pub fn where_all_type<I, K, T>(&self, pairs: I) -> &Self
    where
        I: IntoIterator<Item = (K, T)>,
        K: AsRef<str>,
        T: AsRef<str>,
    {
        for (path, types) in pairs {
            self.where_type(path.as_ref(), types.as_ref());
        }
        self
    }

    /// Assert the prop at `path` contains `expected`: every element of
    /// `expected` when it is an array, else `expected` itself. An array
    /// prop contains a value among its elements, an object among its
    /// values, and any other prop only by equalling it. Laravel's
    /// `whereContains`.
    pub fn where_contains(&self, path: &str, expected: impl Into<Value>) -> &Self {
        let actual = self.present("where_contains", path);
        let wanted = match expected.into() {
            Value::Array(items) => items,
            other => vec![other],
        };
        let pool: Vec<&Value> = match actual {
            Value::Array(items) => items.iter().collect(),
            Value::Object(map) => map.values().collect(),
            scalar => vec![scalar],
        };
        let absent: Vec<String> = wanted
            .iter()
            .filter(|value| !pool.contains(value))
            .map(Value::to_string)
            .collect();
        if !absent.is_empty() {
            self.fail(format!(
                "AssertableInertia::where_contains({:?}, ...)\n  Property does not contain [{}]\n  \
                 Received: {actual}",
                self.path_of(path),
                absent.join(", ")
            ));
        }
        self
    }

    /// Assert the array (or object) prop at `path` has `expected`
    /// elements.
    pub fn count(&self, path: &str, expected: usize) -> &Self {
        self.interacts_with(path);
        let actual = dot_path(&self.props, path);
        if actual.and_then(length_of) != Some(expected) {
            self.fail(format!(
                "AssertableInertia::count({:?}, {expected})\n  Expected: an array of length \
                 {expected}\n  Received: {}",
                self.path_of(path),
                actual
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "<missing or not an array>".to_string())
            ));
        }
        self
    }

    /// Assert the array (or object) prop at `path` has from `min` to `max`
    /// elements, both included. Laravel's `countBetween`.
    pub fn count_between(&self, path: &str, min: usize, max: usize) -> &Self {
        self.interacts_with(path);
        let actual = dot_path(&self.props, path);
        let in_range = actual
            .and_then(length_of)
            .is_some_and(|length| (min..=max).contains(&length));
        if !in_range {
            self.fail(format!(
                "AssertableInertia::count_between({:?}, {min}, {max})\n  Expected: an array \
                 with {min} to {max} elements\n  Received: {}",
                self.path_of(path),
                actual
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "<missing or not an array>".to_string())
            ));
        }
        self
    }

    /// Run `callback` over the object or array at `path` as its own
    /// `AssertableInertia`, whose paths and failure messages carry the full
    /// dotted path (`user.name`) and whose component, url, version and
    /// flash are this page's. When the callback returns, the scope fails
    /// if a prop in it was touched by no assertion, unless
    /// [`Self::etc`] was called in it: a page that starts sending a prop
    /// nobody asserted on is how a leaked field shows up.
    pub fn scope(&self, path: &str, callback: impl FnOnce(&AssertableInertia)) -> &Self {
        self.interacts_with(path);
        let full = self.path_of(path);
        let value = match dot_path(&self.props, path) {
            Some(value @ (Value::Object(_) | Value::Array(_))) => value.clone(),
            other => self.fail(format!(
                "AssertableInertia::scope({full:?}, ...)\n  Property [{full}] is not \
                 scopeable: {}",
                shown(other)
            )),
        };
        let scope = self.child(full, value);
        callback(&scope);
        scope.interacted();
        self
    }

    /// Assert a prop exists at `path`, then run `callback` over it as
    /// [`Self::scope`] does. Laravel's `has($key, $callback)`.
    pub fn has_with(&self, path: &str, callback: impl FnOnce(&AssertableInertia)) -> &Self {
        self.present("has_with", path);
        self.scope(path, callback)
    }

    /// Assert the array at `path` has `count` elements, then run
    /// `callback` over its first element as [`Self::first`] does. The
    /// other elements are not checked (`etc()` on the array's scope); the
    /// first element's scope still is. Laravel's `has($key, $length,
    /// $callback)`.
    pub fn has_count_with(
        &self,
        path: &str,
        count: usize,
        callback: impl FnOnce(&AssertableInertia),
    ) -> &Self {
        self.present("has_count_with", path);
        self.count(path, count);
        self.scope(path, |elements| {
            elements.first(callback).etc();
        })
    }

    /// Run `callback` over the first element of this page's props, or of
    /// this scope's object or array, as [`Self::scope`] does. Fails when
    /// there is no element.
    pub fn first(&self, callback: impl FnOnce(&AssertableInertia)) -> &Self {
        let Some(key) = self.keys().into_iter().next() else {
            self.fail(self.empty_scope_message("first"));
        };
        self.scope(&key, callback)
    }

    /// Run `callback` over every element of this page's props, or of this
    /// scope's object or array, each as [`Self::scope`] does. Fails when
    /// there is no element.
    pub fn each(&self, mut callback: impl FnMut(&AssertableInertia)) -> &Self {
        let keys = self.keys();
        if keys.is_empty() {
            self.fail(self.empty_scope_message("each"));
        }
        for key in keys {
            self.scope(&key, &mut callback);
        }
        self
    }

    /// Count every prop of this scope as touched, so the scope passes with
    /// props no assertion named. Laravel's `etc`.
    pub fn etc(&self) -> &Self {
        let keys = self.keys();
        let mut interacted = self
            .interacted
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        *interacted = keys;
        self
    }

    /// The value at `path`, failing `method`'s assertion when there is
    /// none. Counts the path as touched.
    fn present(&self, method: &str, path: &str) -> &Value {
        self.interacts_with(path);
        match dot_path(&self.props, path) {
            Some(value) => value,
            None => self.fail(format!(
                "AssertableInertia::{method}({:?})\n  prop not present\n  props: {}",
                self.path_of(path),
                self.props
            )),
        }
    }

    /// `path` from the page's props root: this scope's path, then `path`.
    fn path_of(&self, path: &str) -> String {
        match &self.scope {
            Some(scope) if path.is_empty() => scope.clone(),
            Some(scope) => format!("{scope}.{path}"),
            None => path.to_string(),
        }
    }

    /// Count the prop `path` starts at as touched, for the scope's check.
    fn interacts_with(&self, path: &str) {
        let key = path.split('.').next().unwrap_or(path).to_string();
        let mut interacted = self
            .interacted
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if !interacted.contains(&key) {
            interacted.push(key);
        }
    }

    /// The keys of this scope's props: an object's keys, an array's
    /// indexes.
    fn keys(&self) -> Vec<String> {
        match &self.props {
            Value::Object(map) => map.keys().cloned().collect(),
            Value::Array(items) => (0..items.len()).map(|i| i.to_string()).collect(),
            _ => Vec::new(),
        }
    }

    /// Fail a scope one of whose props no assertion touched. The root
    /// never checks, as Laravel's `assertInertia` does not.
    fn interacted(&self) {
        let Some(scope) = &self.scope else {
            return;
        };
        let interacted = self
            .interacted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let untouched: Vec<String> = self
            .keys()
            .into_iter()
            .filter(|key| !interacted.contains(key))
            .collect();
        if !untouched.is_empty() {
            self.fail(format!(
                "AssertableInertia: unexpected properties were found in scope [{scope}]: {}\n  \
                 Assert on them, or call etc() in the scope to allow the rest.\n  props: {}",
                untouched.join(", "),
                self.props
            ));
        }
    }

    /// The message for `first` or `each` on a scope with no element.
    fn empty_scope_message(&self, method: &str) -> String {
        let target = if method == "first" {
            "the first element"
        } else {
            "each element"
        };
        let of = match &self.scope {
            Some(scope) => format!("property [{scope}]"),
            None => "the root level".to_string(),
        };
        format!(
            "AssertableInertia::{method}(...)\n  Cannot scope onto {target} of {of} because it \
             is empty."
        )
    }

    /// A scope over `value`, at the full path `scope`, with this page's
    /// fields.
    fn child(&self, scope: String, value: Value) -> AssertableInertia {
        AssertableInertia {
            component: self.component.clone(),
            url: self.url.clone(),
            version: self.version.clone(),
            props: value,
            flash: self.flash.clone(),
            deferred_props: self.deferred_props.clone(),
            reload: self.reload.clone(),
            report: self.report.clone(),
            scope: Some(scope),
            interacted: Mutex::new(Vec::new()),
        }
    }

    /// Assert the page's `flash` data has `key`, optionally equal to
    /// `expected`. Pass `None::<serde_json::Value>` to check presence
    /// only. `key` follows the same dot-path rule as [`Self::prop`].
    pub fn has_flash<V: Into<Value>>(&self, key: &str, expected: Option<V>) -> &Self {
        let actual = dot_path(&self.flash, key);
        if actual.is_none() {
            self.fail(format!(
                "AssertableInertia::has_flash({key:?}, ...)\n  flash key not present\n  flash: \
                 {}",
                self.flash
            ));
        }
        if let Some(expected) = expected {
            let expected = expected.into();
            if actual != Some(&expected) {
                self.fail(format!(
                    "AssertableInertia::has_flash({key:?}, Some(...))\n  Expected: \
                     {expected}\n  Received: {}",
                    actual.unwrap()
                ));
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
        reloaded.assert_component("component", &self.component, Some(false));
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
        reloaded.assert_component("component", &self.component, Some(false));
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
            self.fail(
                "AssertableInertia::reload_only/reload_except/load_deferred_props: no reloader \
                 attached - call `.with_reload(...)` first; see \
                 manual/http-tests.md#testing-inertia-responses"
                    .to_string(),
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

/// The value for a failure message, `<missing>` for a path that resolved
/// to nothing.
fn shown(value: Option<&Value>) -> String {
    value
        .map(Value::to_string)
        .unwrap_or_else(|| "<missing>".to_string())
}

/// The number of elements of an array or object; `None` for any other
/// value, which has no length to count.
fn length_of(value: &Value) -> Option<usize> {
    match value {
        Value::Array(items) => Some(items.len()),
        Value::Object(map) => Some(map.len()),
        _ => None,
    }
}

/// The test for one of the type names Laravel's `whereType` compares PHP's
/// `gettype` against, `None` for a name it does not know. A JSON array and
/// a JSON object are both PHP arrays.
fn json_type_is(name: &str) -> Option<fn(&Value) -> bool> {
    Some(match name.trim() {
        "string" => Value::is_string,
        "integer" => |value: &Value| value.is_i64() || value.is_u64(),
        "double" => Value::is_f64,
        "boolean" => Value::is_boolean,
        "array" => |value: &Value| value.is_array() || value.is_object(),
        "null" => Value::is_null,
        _ => return None,
    })
}

/// Extract the JSON page object from a hard-navigation HTML shell's first
/// `<script type="application/json" data-page="...">` element, whatever its
/// id: the attribute carries the configured mount id, `app` by default. The
/// element's content is standard JSON with every `/` escaped as `\/`
/// (`framework/src/inertia/response.rs` `build_html_response`) - a
/// valid JSON escape `serde_json` parses natively, so no unescaping is
/// needed.
///
/// Returns `None` when the element itself isn't present, and
/// `Some(Err(_))` when it's present but its content doesn't parse -
/// distinguishing "the element is missing" from "the element is there
/// but malformed" so [`AssertableInertia::from_response`] can report
/// the real cause instead of misreporting a found-but-broken element as
/// absent.
pub(crate) fn page_object_from_html(html: &str) -> Option<Result<Value, serde_json::Error>> {
    const OPEN: &str = r#"<script type="application/json" data-page=""#;
    let id_at = html.find(OPEN)? + OPEN.len();
    // The id is written attribute-escaped, so the first `">` closes the tag.
    let start = html[id_at..].find("\">")? + id_at + 2;
    let end = html[start..].find("</script>")? + start;
    Some(serde_json::from_str(&html[start..end]))
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
