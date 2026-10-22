//! The listener owns the publication worker and its shutdown lifetime.

use super::{PageObject, PublishOutcome, Publisher, error, safe_path};
use crate::middleware::{BoxedMiddleware, MiddlewareChain};
use crate::render_cache::collector::{self, Collector};
use crate::{FrameworkError, HttpResponse, MiddlewareRegistry, Request, Router};
use bytes::Bytes;
use sha2::{Digest, Sha256};
use std::{
    any::TypeId,
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, OnceLock, RwLock},
};
use tokio::sync::{Mutex, Notify};

const GENERATION_CHANNEL: &str = "suprnova.static-publication.generations";

static RUNTIME: OnceLock<RwLock<Option<Arc<StaticPublication>>>> = OnceLock::new();

/// Application publication settings and pending dependency regenerations.
pub struct StaticPublication {
    routes: matchit::Router<PublicRoute>,
    refusals: Vec<(String, String)>,
    publisher: Arc<Publisher>,
    middleware: RwLock<Vec<BoxedMiddleware>>,
    source: RwLock<Option<opendal::Operator>>,
    #[cfg(feature = "media")]
    image_options: RwLock<super::images::ImageOptions>,
    pending: Mutex<BTreeSet<String>>,
    notify: Notify,
    paths: RwLock<BTreeSet<String>>,
    scheduled_at: Mutex<Option<tokio::time::Instant>>,
    node: String,
    hub: Option<Arc<dyn crate::BroadcastHub>>,
    hub_task: std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
}

struct PublicRoute {
    pattern: String,
    handler: Arc<crate::routing::BoxedHandler>,
    middleware: Vec<BoxedMiddleware>,
}

impl StaticPublication {
    /// Registers the boot task without running it before the listener binds.
    pub fn install(router: &Router, publisher: Publisher) -> Result<Arc<Self>, FrameworkError> {
        let patterns = router.public_page_patterns();
        let paths = patterns
            .iter()
            .filter(|p| !p.contains(['{', '*']))
            .cloned()
            .collect();
        let refusals = crate::inertia::publication::boot_refusals(router)?;
        let mut routes = matchit::Router::new();
        for pattern in patterns {
            let (_, handler, _) = router
                .match_route(&hyper::Method::GET, &pattern)
                .ok_or_else(|| error(format!("route {pattern}: missing GET handler")))?;
            routes
                .insert(
                    &pattern,
                    PublicRoute {
                        pattern: pattern.clone(),
                        handler,
                        middleware: router.get_route_middleware(&hyper::Method::GET, &pattern),
                    },
                )
                .map_err(error)?;
        }
        let runtime = Arc::new(Self {
            routes,
            refusals,
            publisher: Arc::new(publisher),
            middleware: RwLock::new(crate::middleware::get_global_middleware()),
            source: RwLock::new(None),
            #[cfg(feature = "media")]
            image_options: RwLock::new(super::images::ImageOptions::default()),
            pending: Mutex::new(BTreeSet::new()),
            notify: Notify::new(),
            paths: RwLock::new(paths),
            scheduled_at: Mutex::new(None),
            node: uuid::Uuid::now_v7().to_string(),
            hub: crate::App::make::<dyn crate::BroadcastHub>(),
            hub_task: std::sync::Mutex::new(None),
        });
        *crate::lock::write(
            RUNTIME.get_or_init(|| RwLock::new(None)),
            "static publication runtime",
        )? = Some(runtime.clone());
        runtime.listen_to_hub()?;
        Ok(runtime)
    }
    fn listen_to_hub(self: &Arc<Self>) -> Result<(), FrameworkError> {
        let Some(hub) = &self.hub else { return Ok(()) };
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return Ok(());
        };
        let mut task = self
            .hub_task
            .lock()
            .map_err(|_| error("publication hub task lock poisoned"))?;
        if task.is_some() {
            return Ok(());
        }
        let mut receiver = hub.subscribe(GENERATION_CHANNEL);
        let weak = Arc::downgrade(self);
        *task = Some(handle.spawn(async move {
            loop {
                let message = match receiver.recv().await {
                    Ok(message) => message,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
                        tracing::error!(count,"static publication invalidation receiver lagged; timed revalidation covers missed tags");
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                };
                let Some(runtime) = weak.upgrade() else { break };
                if message.event != "generations" || message.data.get("source").and_then(serde_json::Value::as_str) == Some(&runtime.node) { continue; }
                let tags = message.data.get("tags").cloned().and_then(|value|serde_json::from_value::<Vec<String>>(value).ok());
                match tags {
                    Some(tags) if tags.iter().all(|tag|tag.len()==64 && tag.bytes().all(|b|b.is_ascii_hexdigit())) => {
                        if let Err(failure) = runtime.invalidate(tags).await {tracing::error!(%failure,"static publication hub invalidation failed");}
                    }
                    _ => tracing::error!("static publication hub received malformed dependency tags"),
                }
            }
        }));
        Ok(())
    }
    /// Installs a configured source disk for Vite assets and referenced images.
    pub fn assets(&self, source: opendal::Operator) -> Result<(), FrameworkError> {
        *crate::lock::write(&self.source, "publication asset source")? = Some(source);
        Ok(())
    }
    /// Selects image formats and retained PNGs before publication starts.
    #[cfg(feature = "media")]
    pub fn images(&self, options: super::images::ImageOptions) -> Result<(), FrameworkError> {
        *crate::lock::write(&self.image_options, "publication image options")? = options;
        Ok(())
    }
    /// Adds a concrete parameterized path to the boot inventory.
    pub fn add_path(&self, path: &str) -> Result<(), FrameworkError> {
        let path = safe_path(path)?;
        self.routes
            .at(&path)
            .map_err(|_| error(format!("route {path}: not marked public")))?;
        crate::lock::write(&self.paths, "static publication paths")?.insert(path);
        Ok(())
    }
    /// Reads the installed service for console commands and application jobs.
    pub fn current() -> Result<Option<Arc<Self>>, FrameworkError> {
        RUNTIME
            .get()
            .map(|slot| crate::lock::read(slot, "static publication runtime").map(|v| v.clone()))
            .transpose()
            .map(Option::flatten)
    }
    /// Reads publication status without page bodies.
    pub async fn status(&self) -> super::PublishStatus {
        let mut status = self.publisher.status().await;
        status.pending += self.pending.lock().await.len();
        if let Ok(manifest) = self.publisher.manifest().await {
            status.live_version = manifest.live;
        }
        status
    }
    /// Renders and publishes the marked concrete paths through one pipeline.
    pub async fn publish(&self) -> Result<PublishOutcome, FrameworkError> {
        let paths = crate::lock::read(&self.paths, "static publication paths")?
            .iter()
            .cloned()
            .collect();
        self.publish_paths(paths, false).await
    }
    async fn publish_paths(
        &self,
        paths: Vec<String>,
        partial: bool,
    ) -> Result<PublishOutcome, FrameworkError> {
        let render = || async move {
            let refusals = &self.refusals;
            if let Some((route, cause)) = refusals.first() {
                return Err(error(format!(
                    "route {route}: publication refused: {cause}"
                )));
            }
            let mut pages = Vec::new();
            for path in paths {
                if let Some((route, cause)) = refusals.iter().find(|(route, _)| route == &path) {
                    return Err(error(format!(
                        "route {route}: publication refused: {cause}"
                    )));
                }
                match self.render(&path).await {
                    Ok(page) => {
                        self.validate_assets(&page)?;
                        pages.push(page);
                    }
                    Err(e) => return Err(e),
                }
            }
            Ok(pages)
        };
        if partial {
            self.publisher.publish_partial_with(render).await
        } else {
            self.publisher.publish_with(render).await
        }
    }
    /// The origin and worker use this same session-free render.
    pub async fn render(&self, path: &str) -> Result<PageObject, FrameworkError> {
        let path = safe_path(path)?;
        let matched = self
            .routes
            .at(&path)
            .map_err(|_| error(format!("route {path}: not marked public")))?;
        let route = matched.value;
        if let Some((_, cause)) = self
            .refusals
            .iter()
            .find(|(pattern, _)| pattern == &route.pattern)
        {
            return Err(error(format!("route {path}: {cause}")));
        }
        let pattern = route.pattern.clone();
        let handler = route.handler.clone();
        let params = matched
            .params
            .iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect();
        let request = Request::for_publication(&path)?
            .with_params(params)
            .with_route_pattern(pattern.clone());
        let middleware =
            crate::lock::read(&self.middleware, "static publication middleware")?.clone();
        if middleware
            .iter()
            .chain(route.middleware.iter())
            .any(crate::middleware::inertia_hooks_of)
        {
            return Err(error(format!(
                "route {path}: opaque Inertia middleware hooks cannot be published"
            )));
        }
        let mut chain = MiddlewareChain::new();
        chain.extend(
            middleware
                .into_iter()
                .chain(route.middleware.clone())
                .filter(public_middleware),
        );
        let route = pattern.clone();
        let ((response, capture, refusal), mut report) = Collector::scope(async {
            let result = crate::inertia::publication::scope(
                route.clone(),
                crate::inertia::cache::with_route(
                    route,
                    crate::security_headers::scope_publication(chain.execute(request, handler)),
                ),
            )
            .await;
            (result, collector::current_report().unwrap_or_default())
        })
        .await;
        let response = response.map_err(|e| {
            error(refusal.unwrap_or_else(|| {
                format!("route {path}: render returned HTTP {}", e.status_code())
            }))
        })?;
        report.fold_gate_into_content();
        if report.storable().is_none()
            || report.context.session_read
            || report.context.principal_read
            || report.context.secret_context_read
            || report.context.foreign_connection_read
            || !report.undeclared.is_empty()
        {
            return Err(error(format!(
                "route {path}: public render reads session, identity, secret or unobservable context"
            )));
        }
        let capture = capture.ok_or_else(|| {
            error(format!(
                "route {path}: handler produced no checked public Inertia document"
            ))
        })?;
        if capture.route != pattern
            || response.status_code() != 200
            || Sha256::digest(response.body()).as_slice() != capture.digest
        {
            return Err(error(format!(
                "route {path}: incomplete or rewritten public document"
            )));
        }
        let mut page = PageObject::new(&path, capture.html, capture.json)?;
        page.headers = response.headers().fold(
            BTreeMap::<String, String>::new(),
            |mut headers, (name, value)| {
                let name = name.to_ascii_lowercase();
                if name == "vary"
                    && let Some(previous) = headers.get_mut(&name)
                {
                    previous.push_str(", ");
                    previous.push_str(value);
                } else {
                    headers.insert(name, value.to_string());
                }
                headers
            },
        );
        page.headers.remove("content-length");
        page.dependency_tags = report
            .observed
            .iter()
            .map(|id| hex::encode(id.digest()))
            .collect();
        self.prepare(&mut page).await?;
        Ok(page)
    }
    fn validate_assets(&self, page: &PageObject) -> Result<(), FrameworkError> {
        for path in super::assets::referenced_assets(&page.html, &page.path)? {
            if !page.assets.contains_key(&path) {
                return Err(error(format!(
                    "route {}: referenced asset {path} has no source disk; configure StaticPublication::assets",
                    page.path
                )));
            }
        }
        let json: serde_json::Value = serde_json::from_slice(&page.json).map_err(error)?;
        if json["version"].as_str() != Some(&self.publisher.asset_version) {
            return Err(error(format!(
                "route {}: Inertia asset version does not match publication version",
                page.path
            )));
        }
        Ok(())
    }
    async fn prepare(&self, page: &mut PageObject) -> Result<(), FrameworkError> {
        let source = crate::lock::read(&self.source, "publication asset source")?.clone();
        let html_inventory = super::assets::referenced_assets(&page.html, &page.path)?;
        let vite_inventory = if let Some(source) = &source {
            super::assets::vite_assets(source, "assets/.vite/manifest.json", &html_inventory)
                .await?
        } else {
            BTreeSet::new()
        };
        let mut inventory = html_inventory;
        inventory.extend(vite_inventory.iter().cloned());
        #[cfg(not(feature = "media"))]
        for path in &inventory {
            if super::assets::is_raster(path) {
                return Err(error(format!(
                    "route {}: raster {path} requires the media feature",
                    page.path
                )));
            }
        }
        if let Some(source) = &source {
            #[cfg(feature = "media")]
            let mut options =
                crate::lock::read(&self.image_options, "publication image options")?.clone();
            #[cfg(feature = "media")]
            {
                options.document_path.clone_from(&page.path);
                let prepared = super::images::prepare(&page.html, source, &options).await?;
                page.html = prepared.html;
                (page.html, page.json) = crate::inertia::publication::rewrite_public_image_urls(
                    &page.html,
                    &page.json,
                    &prepared.replacements,
                )?;
                for asset in prepared.assets {
                    page.assets
                        .insert(asset.path, (asset.bytes, asset.content_type));
                }
            }
            // HTML variants replace their original references; Vite consumers retain their keys.
            let mut referenced = super::assets::referenced_assets(&page.html, &page.path)?;
            referenced.extend(vite_inventory);
            for path in referenced {
                if !page.assets.contains_key(&path) {
                    let bytes = source.read(&path).await.map_err(error)?.to_bytes();
                    let raster = !super::assets::is_vector_or_icon(&path)
                        && (super::assets::is_raster(&path)
                            || infer::get(&bytes)
                                .is_some_and(|kind| kind.mime_type().starts_with("image/")));
                    #[cfg(not(feature = "media"))]
                    if raster {
                        return Err(error(format!(
                            "route {}: raster {path} requires the media feature",
                            page.path
                        )));
                    }
                    #[cfg(feature = "media")]
                    if raster {
                        let asset = super::images::prepare_fallback(&path, bytes, options.quality)
                            .await
                            .map_err(|failure| error(format!("route {}: {failure}", page.path)))?;
                        page.assets
                            .insert(asset.path, (asset.bytes, asset.content_type));
                        continue;
                    }
                    let mime = crate::http::file_response::content_type_from_extension(
                        std::path::Path::new(&path),
                    );
                    page.assets.insert(path, (bytes, mime));
                }
            }
        }
        let paths = crate::lock::read(&self.paths, "static publication paths")?;
        let mapping: BTreeMap<_, _> = paths
            .iter()
            .map(|path| {
                (
                    path.clone(),
                    format!("/{}/index.json", path.trim_matches('/')).replace("//", "/"),
                )
            })
            .collect();
        let mapping = serde_json::to_string(&mapping)
            .map_err(error)?
            .replace('<', "\\u003c");
        let metadata = format!(
            r#"<meta name="suprnova-static-version" content="{}"><script type="application/json" id="suprnova-static-pages">{mapping}</script>"#,
            self.publisher.asset_version
        );
        let html = std::str::from_utf8(&page.html).map_err(error)?;
        page.html = Bytes::from(if html.contains("</head>") {
            html.replacen("</head>", &format!("{metadata}</head>"), 1)
        } else {
            format!("{metadata}{html}")
        });
        Ok(())
    }
    /// Coalesces a committed dependency tag until the worker drains its queue.
    pub async fn invalidate(
        &self,
        tags: impl IntoIterator<Item = String>,
    ) -> Result<(), FrameworkError> {
        self.pending.lock().await.extend(tags);
        self.notify.notify_one();
        Ok(())
    }
    /// Publishes only pages depending on the queued committed tags.
    pub async fn regenerate_pending(&self) -> Result<Option<PublishOutcome>, FrameworkError> {
        let tags = std::mem::take(&mut *self.pending.lock().await);
        if tags.is_empty() {
            return Ok(None);
        }
        let manifest = self.publisher.manifest().await?;
        let paths: BTreeSet<_> = manifest
            .versions
            .get(&manifest.live)
            .into_iter()
            .flat_map(|v| &v.objects)
            .filter(|o| {
                o.path.ends_with(".html") && o.dependency_tags.iter().any(|tag| tags.contains(tag))
            })
            .map(|o| o.url.clone())
            .collect();
        if paths.is_empty() {
            return Ok(None);
        }
        match self.publish_paths(paths.into_iter().collect(), true).await {
            Ok(outcome) => Ok(Some(outcome)),
            Err(e) => {
                self.pending.lock().await.extend(tags);
                Err(e)
            }
        }
    }
    /// Registers a scheduler pass; the interval still controls page freshness.
    pub fn schedule(
        self: &Arc<Self>,
        schedule: &mut crate::Schedule,
    ) -> Result<(), FrameworkError> {
        let runtime = self.clone();
        schedule.try_add(
            schedule
                .call(move || {
                    let runtime = runtime.clone();
                    async move { runtime.revalidate_if_due().await }
                })
                .every_minute()
                .name("static:revalidate"),
        )?;
        Ok(())
    }
    async fn revalidate_if_due(&self) -> Result<(), FrameworkError> {
        let mut previous = self.scheduled_at.lock().await;
        if previous.is_some_and(|time| time.elapsed() < self.publisher.config.revalidate) {
            return Ok(());
        }
        self.publish().await?;
        *previous = Some(tokio::time::Instant::now());
        Ok(())
    }
    /// Uses the publication retention policy for on-demand cleanup.
    pub async fn prune(&self) -> Result<super::PruneOutcome, FrameworkError> {
        self.publisher.prune().await
    }

    /// Serves public objects or publishes an uncovered path from its first render.
    pub(crate) async fn respond(
        &self,
        path: &str,
        headers: &hyper::HeaderMap,
        query: Option<&str>,
    ) -> Result<Option<HttpResponse>, FrameworkError> {
        let manifest = self.publisher.manifest().await?;
        if let Some(asset) = manifest
            .versions
            .values()
            .flat_map(|v| &v.objects)
            .find(|o| {
                o.asset
                    && (o.path == path.trim_start_matches('/')
                        || (!o.path.starts_with("_pub/") && o.url == path))
            })
        {
            use futures::StreamExt;
            let length = match asset.headers.get("content-length") {
                Some(length) => length.parse::<u64>().map_err(error)?,
                None => self
                    .publisher
                    .disk
                    .stat(&asset.path)
                    .await
                    .map_err(error)?
                    .content_length(),
            };
            let reader = self
                .publisher
                .disk
                .reader_with(&asset.path)
                .chunk(65536)
                .await
                .map_err(error)?;
            let stream = reader.into_bytes_stream(0..length).await.map_err(error)?;
            let stream = stream.scan((), |_, chunk| {
                futures::future::ready(match chunk {
                    Ok(bytes) => Some(Ok::<_, std::convert::Infallible>(bytes)),
                    Err(failure) => {
                        // Content-Length makes a short stream fail the transfer instead of appearing complete.
                        tracing::error!(%failure, "static publication asset stream failed");
                        None
                    }
                })
            });
            let mut response =
                HttpResponse::stream_bytes(stream).header("Content-Length", length.to_string());
            for (name, value) in &asset.headers {
                response = response.replace_header(name, value);
            }
            return Ok(Some(response));
        }
        if query.is_some_and(|query| !query.is_empty())
            || {
                let directives = crate::render_cache::middleware::header_cache_directives(
                    headers
                        .get_all("cache-control")
                        .iter()
                        .filter_map(|value| value.to_str().ok()),
                );
                directives.no_store || directives.no_cache
            }
            || headers.keys().any(|name| {
                matches!(
                    name.as_str(),
                    "x-inertia-partial-component"
                        | "x-inertia-partial-data"
                        | "x-inertia-partial-except"
                        | "x-inertia-except-once-props"
                        | "x-inertia-reset"
                )
            })
        {
            return Ok(None);
        }
        let json = headers
            .get("x-inertia")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| !v.is_empty() && v != "0")
            || path.ends_with("/index.json");
        if let Some(version) = headers
            .get("x-inertia-version")
            .and_then(|v| v.to_str().ok())
            && version != self.publisher.asset_version
        {
            return Ok(None);
        }
        let canonical = if path.ends_with("/index.json") {
            path.trim_end_matches("index.json").trim_end_matches('/')
        } else {
            path
        };
        let canonical = if canonical.is_empty() { "/" } else { canonical };
        if self.routes.at(canonical).is_err() {
            return Ok(None);
        }
        let manifest = self.publisher.manifest().await?;
        let object = manifest.versions.get(&manifest.live).and_then(|version| {
            version.objects.iter().find(|o| {
                if json {
                    o.url
                        == if canonical == "/" {
                            "/index.json".to_owned()
                        } else {
                            format!("{canonical}/index.json")
                        }
                } else {
                    o.url == canonical
                }
            })
        });
        if let Some(object) = object {
            let body = self
                .publisher
                .disk
                .read(&object.path)
                .await
                .map_err(error)?
                .to_bytes();
            return Ok(Some(object_response(body, &object.headers, json)));
        }
        self.add_path(canonical)?;
        let page = match self.render(canonical).await {
            Ok(page) => page,
            Err(e) => {
                tracing::error!(route = canonical,error = %e,"public origin fallback refused");
                return Ok(None);
            }
        };
        self.validate_assets(&page)?;
        let body = if json {
            page.json.clone()
        } else {
            page.html.clone()
        };
        let normalized = self.publisher.representation_headers(
            &page.headers,
            if json {
                "application/json"
            } else {
                "text/html; charset=utf-8"
            },
            &body,
        );
        let mut response = object_response(body, &normalized, json);
        match self.publisher.publish_partial(vec![page]).await {
            Ok(_) => {
                let committed = self.publisher.manifest().await?;
                if let Some(object) = committed.versions.get(&committed.live).and_then(|v| {
                    v.objects.iter().find(|o| {
                        !o.asset
                            && o.url
                                == if json {
                                    if canonical == "/" {
                                        "/index.json".to_string()
                                    } else {
                                        format!("{canonical}/index.json")
                                    }
                                } else {
                                    canonical.to_string()
                                }
                    })
                }) {
                    response = object_response(
                        self.publisher
                            .disk
                            .read(&object.path)
                            .await
                            .map_err(error)?
                            .to_bytes(),
                        &object.headers,
                        json,
                    );
                }
            }
            Err(e) => {
                tracing::error!(route=canonical,error=%e,"origin fallback publication failed")
            }
        }
        Ok(Some(response))
    }
}

fn object_response(body: Bytes, headers: &BTreeMap<String, String>, json: bool) -> HttpResponse {
    let mut response = HttpResponse::bytes(
        body,
        if json {
            "application/json"
        } else {
            "text/html; charset=utf-8"
        },
    );
    for (name, value) in headers {
        if !name.eq_ignore_ascii_case("set-cookie") {
            response = response.header(name, value);
        }
    }
    response = response.replace_header(
        "Content-Type",
        if json {
            "application/json"
        } else {
            "text/html; charset=utf-8"
        },
    );
    if json {
        response = response.replace_header("X-Inertia", "true");
    }
    response
}

fn public_middleware(middleware: &BoxedMiddleware) -> bool {
    let kind = crate::middleware::type_of(middleware);
    ![
        TypeId::of::<crate::SessionMiddleware>(),
        TypeId::of::<crate::CsrfMiddleware>(),
        TypeId::of::<crate::render_cache::RenderCacheMiddleware>(),
    ]
    .into_iter()
    .any(|excluded| kind == Some(excluded))
}

pub(crate) async fn committed_generations(digests: Vec<[u8; 32]>) -> Result<(), FrameworkError> {
    let runtime = StaticPublication::current()?;
    let tags: Vec<String> = digests.into_iter().map(hex::encode).collect();
    if let Some(runtime) = &runtime {
        runtime.invalidate(tags.clone()).await?;
    }
    let hub = crate::App::make::<dyn crate::BroadcastHub>()
        .or_else(|| runtime.as_ref().and_then(|runtime| runtime.hub.clone()));
    if let Some(hub) = hub {
        let source = runtime
            .as_ref()
            .map(|runtime| runtime.node.clone())
            .unwrap_or_else(|| uuid::Uuid::now_v7().to_string());
        hub.publish(crate::BroadcastEnvelope::new(
            GENERATION_CHANNEL,
            "generations",
            serde_json::json!({"source":source,"tags":tags}),
        ))
        .await?;
    }
    Ok(())
}

impl Drop for StaticPublication {
    fn drop(&mut self) {
        let task = self
            .hub_task
            .get_mut()
            .unwrap_or_else(|poison| poison.into_inner());
        if let Some(task) = task.take() {
            task.abort();
        }
    }
}

pub(crate) struct PublicationTask {
    worker: tokio::task::JoinHandle<()>,
    publisher: Publisher,
}
impl PublicationTask {
    pub(crate) async fn shutdown(&mut self) {
        self.worker.abort();
        let _ = (&mut self.worker).await;
        self.publisher.wait_for_delivery().await;
    }
}
impl Drop for PublicationTask {
    fn drop(&mut self) {
        self.worker.abort();
    }
}

/// Called only after the listening socket exists; dropping its guard cancels staging.
pub(crate) fn spawn_after_bind(middleware: &MiddlewareRegistry) -> Option<PublicationTask> {
    let runtime = match StaticPublication::current() {
        Ok(Some(runtime)) => runtime,
        Ok(None) => return None,
        Err(e) => {
            tracing::error!(error = %e,"static publication boot task unavailable");
            return None;
        }
    };
    match crate::lock::write(&runtime.middleware, "static publication middleware") {
        Ok(mut target) => *target = middleware.global_middleware().to_vec(),
        Err(e) => {
            tracing::error!(error = %e,"static publication middleware unavailable");
            return None;
        }
    }
    if let Err(failure) = runtime.listen_to_hub() {
        tracing::error!(%failure,"static publication hub consumer unavailable");
    }
    let publisher = runtime.publisher.as_ref().clone();
    let worker = tokio::spawn(async move {
        let _ = runtime.publish().await;
        let mut interval = tokio::time::interval(runtime.publisher.config.revalidate);
        interval.tick().await;
        loop {
            tokio::select! {
                _ = interval.tick() => { let _ = runtime.publish().await; }
                _ = runtime.notify.notified() => { tokio::task::yield_now().await; let _ = runtime.regenerate_pending().await; }
            }
        }
    });
    Some(PublicationTask { worker, publisher })
}
