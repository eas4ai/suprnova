//! Explicit prop boundaries keep reusable values separate from request state.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::sync::Arc;

use bytes::Bytes;
use indexmap::IndexMap;
use serde::Serialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use super::Prop;
use crate::{App, FrameworkError, Inertia, InertiaResponse};

/// The field contract emitted by `#[derive(InertiaProps)]` with `#[inertia(cacheable)]`.
///
/// Every field belongs to one list. Wire names include serde renames.
pub trait CacheableInertiaProps {
    /// Fields whose values the shared shell may retain.
    const PUBLIC: &'static [&'static str];
    /// Fields resolved afresh after the route's guards on every request.
    const PER_REQUEST: &'static [&'static str];
    /// Converts fields without running their lazy resolvers.
    fn cache_props(&self) -> Result<IndexMap<String, Prop>, FrameworkError>;
}

/// Converts an eager value or a lazy `Prop` at a declared cache boundary.
pub trait IntoInertiaCacheProp {
    /// Serialization errors propagate before any representation is published.
    fn into_cache_prop(self) -> Result<Prop, FrameworkError>;

    /// Custom serializers must not record request reads as shell dependencies.
    #[doc(hidden)]
    fn into_per_request_cache_prop(self) -> Result<Prop, FrameworkError>
    where
        Self: Sized,
    {
        crate::render_cache::collector::slot_call(|| self.into_cache_prop())
    }
}

impl<T: Serialize> IntoInertiaCacheProp for T {
    fn into_cache_prop(self) -> Result<Prop, FrameworkError> {
        serde_json::to_value(self)
            .map(Prop::eager)
            .map_err(|error| FrameworkError::internal(format!("Inertia cache prop: {error}")))
    }
}

impl IntoInertiaCacheProp for Prop {
    fn into_cache_prop(self) -> Result<Prop, FrameworkError> {
        Ok(self)
    }
}

impl IntoInertiaCacheProp for &Prop {
    fn into_cache_prop(self) -> Result<Prop, FrameworkError> {
        Ok(self.clone())
    }
}

impl<T: CacheableInertiaProps + ?Sized> CacheableInertiaProps for &T {
    const PUBLIC: &'static [&'static str] = T::PUBLIC;
    const PER_REQUEST: &'static [&'static str] = T::PER_REQUEST;
    fn cache_props(&self) -> Result<IndexMap<String, Prop>, FrameworkError> {
        <T as CacheableInertiaProps>::cache_props(*self)
    }
}

/// The typed prop conversion used by `inertia_response!`.
///
/// Ordinary serializable values keep eager props. A cacheable props derive
/// implements this contract with its checked public and request boundaries.
///
/// The macro borrows through wrappers before calling this trait by its qualified
/// path, so an inherent `apply_props` does not replace the checked conversion.
pub trait InertiaPageProps {
    /// Adds props without serializing a lazy resolver or losing its cache boundary.
    fn apply_props(&self, response: InertiaResponse) -> Result<InertiaResponse, FrameworkError>;
}

impl<T: Serialize> InertiaPageProps for T {
    fn apply_props(
        &self,
        mut response: InertiaResponse,
    ) -> Result<InertiaResponse, FrameworkError> {
        let value = serde_json::to_value(self).map_err(|error| {
            FrameworkError::internal(format!(
                "inertia_response!({}): typed props failed to serialize: {error}",
                response.component
            ))
        })?;
        match value {
            Value::Object(props) => {
                for (key, value) in props {
                    response.__add_eager(key, value);
                }
                Ok(response)
            }
            other => Err(FrameworkError::internal(format!(
                "inertia_response!({}): page props must serialize to a JSON object, got {other}",
                response.component
            ))),
        }
    }
}

#[derive(Clone)]
/// A checked registration binds shell fields to one route.
pub(crate) struct CachePage {
    route: String,
    public: BTreeSet<String>,
    per_request: BTreeSet<String>,
    public_shared: BTreeSet<String>,
    per_request_shared: BTreeSet<String>,
}

impl CachePage {
    fn is_public(&self, key: &str) -> bool {
        !self.per_request.contains(key)
            && (self.public.contains(key) || self.public_shared.contains(key))
    }

    fn validate_keys<'a>(
        &self,
        keys: impl IntoIterator<Item = &'a str>,
    ) -> Result<(), FrameworkError> {
        for key in keys {
            let root = key.split('.').next().unwrap_or(key);
            if !self.is_public(root)
                && !self.per_request.contains(root)
                && !self.per_request_shared.contains(root)
            {
                return Err(FrameworkError::internal(format!(
                    "Inertia cache route `{}` has undeclared prop `{root}`; list it as public or per-request",
                    self.route
                )));
            }
        }
        Ok(())
    }

    fn observes_public_session(&self) -> bool {
        self.is_public("errors") || self.is_public("flash")
    }

    /// Declaration changes must reject an old shell even within one build.
    pub(crate) fn contract(&self) -> String {
        hex::encode(Sha256::digest(
            serde_json::json!([
                self.route,
                self.public,
                self.per_request,
                self.public_shared,
                self.per_request_shared
            ])
            .to_string()
            .as_bytes(),
        ))
    }

    fn shell(&self, page: &Value) -> Value {
        let props: Map<String, Value> = page["props"]
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(key, _)| self.is_public(key))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        let scroll: Map<String, Value> = page["scrollProps"]
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(key, _)| self.is_public(key))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        serde_json::json!({"component": page["component"], "props": props, "scrollProps": scroll, "contract": self.contract()})
    }
}

/// Reuse is permitted only for the component and declaration in this request.
pub(crate) struct PageAssembly {
    schema: Option<Arc<CachePage>>,
    cached: Option<Value>,
    declared: bool,
}

impl PageAssembly {
    /// A contract mismatch resolves public props afresh.
    pub(crate) fn new(schema: Option<Arc<CachePage>>, component: &str, declared: bool) -> Self {
        let cached = schema.as_ref().and_then(|schema| {
            cached_shell(component)
                .filter(|shell| shell["contract"].as_str() == Some(schema.contract().as_str()))
        });
        Self {
            schema,
            cached,
            declared,
        }
    }

    /// Declared pages keep session and response work out of a reusable shell.
    pub(crate) fn enabled(&self) -> bool {
        self.declared
    }

    /// A public session-derived share must retain the ordinary privacy refusal.
    pub(crate) fn observes_public_session(&self) -> bool {
        self.schema
            .as_ref()
            .is_some_and(|schema| schema.observes_public_session())
    }

    /// Unknown prop keys cannot enter the shell implicitly.
    pub(crate) fn validate(
        &self,
        props: &IndexMap<String, Prop>,
        has_providers: bool,
    ) -> Result<(), FrameworkError> {
        let Some(schema) = &self.schema else {
            return Ok(());
        };
        schema.validate_keys(props.keys().map(String::as_str))?;
        if has_providers {
            return Err(FrameworkError::internal(format!(
                "Inertia cache route `{}` uses an opaque page prop provider",
                schema.route
            )));
        }
        Ok(())
    }

    /// Request fields must not contribute observations to the public shell.
    pub(crate) fn request_slot(&self, key: &str) -> bool {
        self.schema.as_ref().map_or(self.declared, |schema| {
            !schema.is_public(key.split('.').next().unwrap_or(key))
        })
    }

    /// Missing optional or expired values still run their current resolver.
    pub(crate) fn cached_value(&self, key: &str) -> Option<Value> {
        self.cached
            .as_ref()
            .and_then(|cached| super::dotted::arr_get(&cached["props"], key))
    }

    /// SSR and hydration must receive the same assembled page.
    pub(crate) fn assemble(&self, mut page: Value) -> Result<Value, FrameworkError> {
        let (Some(schema), Some(cached)) = (&self.schema, &self.cached) else {
            return Ok(page);
        };
        if let (Some(saved), Some(page)) = (cached["scrollProps"].as_object(), page.as_object_mut())
        {
            let current = page
                .entry("scrollProps")
                .or_insert_with(|| serde_json::json!({}));
            if let Some(current) = current.as_object_mut() {
                current.extend(saved.clone());
            }
        }
        assemble_page(schema, page)
    }

    /// Publication retains only declared public data and checks the final body digest.
    pub(crate) fn capture(
        &self,
        page: &Value,
        response: &crate::HttpResponse,
    ) -> Result<(), FrameworkError> {
        if let Some(schema) = &self.schema {
            capture(&schema.shell(page), response.body(), &schema.route)?;
        } else if self.declared {
            // An unregistered alias renders privately and must never publish as Complete.
            capture(&Value::Null, response.body(), "")?;
        }
        if self.declared {
            super::visit::mark_cacheable();
        }
        Ok(())
    }

    /// A direct render without protocol middleware still needs private transport headers.
    pub(crate) fn finish(&self, response: crate::HttpResponse) -> crate::HttpResponse {
        if self.enabled() && super::visit::current().is_none() {
            response.replace_header("Cache-Control", "private, no-store")
        } else {
            response
        }
    }
}

/// Cache state travels with the request used by property conversions.
pub(super) struct ResolveContext<'a> {
    /// Conversions retain access to the current request.
    pub(super) request: &'a dyn super::InertiaRequestExt,
    /// Ordinary responses keep their existing resolution path.
    pub(super) cache: Option<&'a PageAssembly>,
}

impl Inertia {
    /// Registers a typed cacheable page and classifies every shared prop at boot.
    ///
    /// Call after registering shared props and before `RenderCache::install`.
    /// List `errors` and `flash` too. Session-derived public shares still decline storage.
    /// Opaque shared providers and middleware hooks
    /// cannot prove their keys at boot and are refused for cacheable pages.
    pub fn cache_page<P: CacheableInertiaProps>(
        route: &str,
        component: &str,
        public_shared: &[&str],
        per_request_shared: &[&str],
    ) -> Result<(), FrameworkError> {
        let names = |values: &[&str]| values.iter().map(|v| (*v).to_owned()).collect();
        let mut page = CachePage {
            route: route.to_owned(),
            public: names(P::PUBLIC),
            per_request: names(P::PER_REQUEST),
            public_shared: names(public_shared),
            per_request_shared: names(per_request_shared),
        };
        page.public_shared.insert("root".to_owned());
        if !page.public.is_disjoint(&page.per_request)
            || !page.public_shared.is_disjoint(&page.per_request_shared)
        {
            return Err(FrameworkError::internal(format!(
                "Inertia cache route `{route}` has conflicting prop declarations"
            )));
        }
        let registry = App::inertia_registry();
        if registry.trait_provider()?.is_some()
            || !registry.shared_providers()?.is_empty()
            || registry
                .installed_config()
                .is_some_and(|config| config.hooks.is_some())
        {
            return Err(FrameworkError::internal(format!(
                "Inertia cache route `{route}` has an opaque shared prop provider or middleware hook; register named shared props so every prop can be checked at boot"
            )));
        }
        let shared = registry.snapshot_static()?;
        for key in shared
            .iter()
            .map(|(key, _)| key.as_str())
            .chain(["errors", "flash"])
        {
            let root = key.split('.').next().unwrap_or(key);
            if !page.public_shared.contains(root) && !page.per_request_shared.contains(root) {
                return Err(FrameworkError::internal(format!(
                    "Inertia cache route `{route}` has unlisted shared prop `{root}`; list it as public or per-request"
                )));
            }
        }
        let mut pages = crate::lock::write(&registry.cache_pages, "Inertia cache pages")?;
        let key = (route.to_owned(), component.to_owned());
        if pages.contains_key(&key) {
            return Err(FrameworkError::internal(format!(
                "Inertia cache component `{component}` is already registered for route `{route}`"
            )));
        }
        pages.insert(key, Arc::new(page));
        Ok(())
    }
}

impl InertiaResponse {
    /// Attaches props whose cache boundaries the derive checked.
    ///
    /// Only the registered request route can reuse public props. Other routes
    /// render all props privately and decline publication.
    /// Lazy public props run on a miss; per-request props run on every visit.
    pub fn cacheable<P: CacheableInertiaProps>(mut self, props: P) -> Result<Self, FrameworkError> {
        if super::publication::active() && !P::PER_REQUEST.is_empty() {
            return Err(super::publication::refusal(format!(
                "render uses per-request props: {}",
                P::PER_REQUEST.join(", ")
            )));
        }
        let registry = App::inertia_registry();
        let pages = crate::lock::read(&registry.cache_pages, "Inertia cache pages")?;
        let route = CACHE_ROUTE.try_with(Clone::clone).ok();
        let page = pages
            .iter()
            .find(|((registered_route, component), _)| {
                component == &self.component && route.as_ref() == Some(registered_route)
            })
            .map(|(_, page)| page.clone());
        drop(pages);
        let names = |values: &[&str]| {
            values
                .iter()
                .map(|v| (*v).to_owned())
                .collect::<BTreeSet<_>>()
        };
        if let Some(page) = &page
            && (page.public != names(P::PUBLIC) || page.per_request != names(P::PER_REQUEST))
        {
            return Err(FrameworkError::internal(format!(
                "Inertia cache route `{}` uses a different props declaration",
                page.route
            )));
        }
        self.props.extend(props.cache_props()?);
        self.cache_page = page;
        self.cache_declared = true;
        Ok(self)
    }
}

struct CachedAssembly {
    public: Value,
    entry: suprnova_live::render_cache::composite::CompositeEntry,
}

tokio::task_local! {
    static CACHED_SHELL: CachedAssembly;
    static CACHE_ROUTE: String;
}

/// The handler selects its registration after its guards have run.
pub(crate) async fn with_route<F: Future>(route: String, future: F) -> F::Output {
    CACHE_ROUTE.scope(route, future).await
}

fn cached_shell(component: &str) -> Option<Value> {
    CACHED_SHELL
        .try_with(|page| {
            (page.public["component"].as_str() == Some(component)).then(|| page.public.clone())
        })
        .ok()
        .flatten()
}

/// Task-local state prevents prepared shells crossing concurrent visitors.
pub(crate) async fn with_shell<F: Future>(
    shell: Value,
    entry: suprnova_live::render_cache::composite::CompositeEntry,
    future: F,
) -> F::Output {
    CACHED_SHELL
        .scope(
            CachedAssembly {
                public: shell,
                entry,
            },
            future,
        )
        .await
}

fn assemble_page(schema: &CachePage, page: Value) -> Result<Value, FrameworkError> {
    CACHED_SHELL
        .try_with(|cached| {
            super::super::render_cache::inertia_page::assemble_page(&cached.entry, schema, page)
                .map_err(|()| {
                    FrameworkError::internal("Inertia request props could not be assembled safely")
                })
        })
        .map_err(|_| FrameworkError::internal("Inertia public shell scope is missing"))?
}

/// The collector attributes reads while a request resolver is polled.
pub(crate) async fn in_slot<F: Future>(slot: bool, future: F) -> F::Output {
    if slot {
        crate::render_cache::collector::slot_scope(future).await
    } else {
        future.await
    }
}

/// The public page data and digest captured before route middleware can rewrite it.
#[doc(hidden)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InertiaDocumentFacts {
    /// Serialized public props; never contains a request prop or session flag.
    pub shell: Bytes,
    /// Digest of the actual response, to decline post-render body rewrites.
    pub digest: [u8; 32],
    /// The registered route pattern, or empty when no registration matches.
    pub route: String,
}

fn capture(shell: &Value, body: &[u8], route: &str) -> Result<(), FrameworkError> {
    let shell = serde_json::to_vec(shell)
        .map_err(|error| FrameworkError::internal(format!("Inertia cache shell: {error}")))?;
    crate::render_cache::collector::observe_inertia_document(InertiaDocumentFacts {
        shell: Bytes::from(shell),
        digest: Sha256::digest(body).into(),
        route: route.to_owned(),
    });
    Ok(())
}

/// Refuse unchecked shared keys and policies before the cache runtime is installed.
pub(crate) fn validate_boot(
    router: &crate::Router,
    policies: &crate::render_cache::registry::RenderCachePolicyTable,
) -> Result<(), FrameworkError> {
    let registry = App::inertia_registry();
    let pages: BTreeMap<_, _> =
        crate::lock::read(&registry.cache_pages, "Inertia cache pages")?.clone();
    for page in pages.values() {
        let has_route_hooks = [hyper::Method::GET, hyper::Method::HEAD]
            .iter()
            .any(|method| {
                router
                    .get_route_middleware(method, &page.route)
                    .iter()
                    .chain(crate::middleware::get_global_middleware().iter())
                    .any(crate::middleware::inertia_hooks_of)
            });
        if has_route_hooks {
            return Err(FrameworkError::internal(format!(
                "Inertia cache route `{}` has an opaque middleware hook; use named shared props",
                page.route
            )));
        }
        if registry.trait_provider()?.is_some()
            || !registry.shared_providers()?.is_empty()
            || registry
                .installed_config()
                .is_some_and(|config| config.hooks.is_some())
        {
            return Err(FrameworkError::internal(format!(
                "Inertia cache route `{}` has an opaque shared prop provider or middleware hook",
                page.route
            )));
        }
        if policies.effective_policy(&page.route).is_none_or(|p| {
            p.class() != crate::render_cache::RepresentationClass::PublicShellStitched
        }) {
            return Err(FrameworkError::internal(format!(
                "Inertia cache route `{}` needs a PublicShellStitched RenderCache policy",
                page.route
            )));
        }
        let shared = registry.snapshot_static()?;
        for (key, _) in &shared {
            let root = key.split('.').next().unwrap_or(key);
            if !page.public_shared.contains(root) && !page.per_request_shared.contains(root) {
                return Err(FrameworkError::internal(format!(
                    "Inertia cache route `{}` has unlisted shared prop `{root}`",
                    page.route
                )));
            }
        }
    }
    Ok(())
}

/// Late or manually boxed hooks must fail before any eager share can run.
pub(crate) fn refuse_request_hooks(request: &crate::Request) -> Result<(), FrameworkError> {
    let registry = App::inertia_registry();
    let pages = crate::lock::read(&registry.cache_pages, "Inertia cache pages")?;
    if let Some(route) = request.route_pattern()
        && pages.keys().any(|(pattern, _)| pattern == route)
    {
        return Err(FrameworkError::internal(format!(
            "Inertia cache route `{route}` has an opaque middleware hook; use named shared props"
        )));
    }
    Ok(())
}

/// A shell built without a session cannot replace a later session's public errors.
pub(crate) fn route_observes_public_session(route: &str) -> Result<bool, FrameworkError> {
    if !crate::session::middleware::session_scope_installed() {
        return Ok(false);
    }
    let registry = App::inertia_registry();
    let pages = crate::lock::read(&registry.cache_pages, "Inertia cache pages")?;
    Ok(pages
        .iter()
        .any(|((pattern, _), page)| pattern == route && page.observes_public_session()))
}
