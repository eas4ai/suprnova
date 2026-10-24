//! Request diagnostics kept only in memory while the application's debug gate is open.
//!
//! Install [`DebugBar::middleware`] globally. The server opens its task-local
//! scope outside panic recovery and error rendering so the record describes the
//! response the browser received, including failures outside the handler.

mod cache;
mod collector;
mod toolbar;

pub(crate) use collector::{
    active, event, in_current_scope, next_span, note_context, note_request, record, span,
};
pub(crate) use toolbar::SkipInjection;

use bytes::Bytes;
use http_body_util::{BodyExt, Full, combinators::BoxBody};
use hyper::header::{CACHE_CONTROL, HeaderValue};
use std::collections::VecDeque;
use std::convert::Infallible;
use std::sync::{Arc, Mutex, Weak};

use crate::{Config, FrameworkError, Middleware, Next, Request, Response};

type WireResponse = hyper::Response<BoxBody<Bytes, Infallible>>;
type Store = Arc<Mutex<VecDeque<Arc<collector::Record>>>>;
const ID_HEADER: &str = "x-debugbar-id";
const ROUTE: &str = "/_debugbar/";

/// Installs request diagnostics through the same global middleware API as other facades.
pub struct DebugBar;

impl DebugBar {
    /// Build an isolated bounded store for a server's middleware registration.
    ///
    /// `DEBUGBAR_KEEP` must be a positive integer; an invalid limit fails boot
    /// rather than silently keeping an unexpected number of private records.
    /// The debug and `DEBUGBAR` gates are checked afresh for each request.
    pub fn middleware() -> Result<DebugBarMiddleware, FrameworkError> {
        let keep = match std::env::var("DEBUGBAR_KEEP") {
            Ok(value) => value
                .parse::<usize>()
                .ok()
                .filter(|keep| *keep > 0)
                .ok_or_else(|| {
                    FrameworkError::internal("DEBUGBAR_KEEP must be a positive integer")
                })?,
            Err(std::env::VarError::NotPresent) => 100,
            Err(_) => {
                return Err(FrameworkError::internal(
                    "DEBUGBAR_KEEP must be a positive integer",
                ));
            }
        };
        Ok(DebugBarMiddleware {
            keep,
            store: Arc::new(Mutex::new(VecDeque::new())),
        })
    }
}

/// Carries a server's diagnostic store without changing application middleware ordering.
///
/// Its registration lets the HTTP boundary collect the entire request, including
/// panic recovery. Clones share only this registration's bounded in-memory store.
#[derive(Clone)]
pub struct DebugBarMiddleware {
    keep: usize,
    store: Store,
}

/// Middleware identity metadata must not keep diagnostic records alive after
/// the server registration is dropped.
#[derive(Clone)]
pub(crate) struct WeakDebugBar {
    keep: usize,
    store: Weak<Mutex<VecDeque<Arc<collector::Record>>>>,
}

impl WeakDebugBar {
    pub(crate) fn upgrade(&self) -> Option<DebugBarMiddleware> {
        self.store.upgrade().map(|store| DebugBarMiddleware {
            keep: self.keep,
            store,
        })
    }
}

#[async_trait::async_trait]
impl Middleware for DebugBarMiddleware {
    async fn handle(&self, request: Request, next: Next) -> Response {
        next(request).await
    }
}

impl DebugBarMiddleware {
    pub(crate) fn downgrade(&self) -> WeakDebugBar {
        WeakDebugBar {
            keep: self.keep,
            store: Arc::downgrade(&self.store),
        }
    }

    /// Let integration tests verify retention and the closed gate without exposing diagnostic values.
    #[cfg(any(test, feature = "testing"))]
    #[doc(hidden)]
    pub fn record_count_for_test(&self) -> Result<usize, FrameworkError> {
        self.store
            .lock()
            .map(|store| store.len())
            .map_err(|_| FrameworkError::internal("debug bar store lock was poisoned"))
    }

    pub(crate) fn enabled(&self) -> bool {
        let enabled = Config::is_debug()
            && !std::env::var("DEBUGBAR").is_ok_and(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "false" | "0" | "off" | "no"
                )
            });
        if !enabled {
            self.store
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .clear();
        }
        enabled
    }

    /// Answer diagnostics without running application middleware. The HTTP
    /// boundary records this request too, under the same retention limit.
    pub(crate) fn lookup<B>(
        &self,
        request: &hyper::Request<B>,
        root: &str,
    ) -> Option<WireResponse> {
        let path = request
            .uri()
            .path()
            .strip_prefix(root)
            .unwrap_or(request.uri().path());
        let id = path.strip_prefix(ROUTE)?;
        let _span = span("handler", None);
        collector::note_lookup(id, root);
        let record = if request.method() == hyper::Method::GET {
            self.store
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .iter()
                .find(|record| record.id == id)
                .cloned()
        } else {
            None
        };
        Some(match record {
            Some(record) => crate::HttpResponse::json(record.json())
                .header("X-Request-Id", uuid::Uuid::new_v4().to_string())
                .header("Cache-Control", "no-store")
                .into_hyper(),
            None => crate::HttpResponse::text(crate::http::NOT_FOUND_BODY)
                .status(404)
                .header("X-Request-Id", uuid::Uuid::new_v4().to_string())
                .header("Cache-Control", "no-store")
                .into_hyper(),
        })
    }

    pub(crate) async fn serve<'a, B: 'a>(
        &'a self,
        request: hyper::Request<B>,
        response: impl FnOnce(hyper::Request<B>) -> futures::future::BoxFuture<'a, WireResponse>,
    ) -> WireResponse {
        let collector = collector::Collector::capture(&request);
        let browser = crate::error::debug_page::production_browser(&request);
        let is_head = request.method() == hyper::Method::HEAD;
        let (mut response, record) = collector::serve(collector, response(request)).await;
        if !self.enabled() {
            return response;
        }
        let id = record.id.clone();
        let record = Arc::new(record);
        {
            let mut store = self
                .store
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            while store.len() >= self.keep {
                store.pop_front();
            }
            store.push_back(Arc::clone(&record));
        }
        if let Ok(id) = HeaderValue::from_str(&id) {
            response.headers_mut().insert(ID_HEADER, id);
        }
        if browser && !is_head && toolbar::injectable(&response) {
            let (mut parts, body) = response.into_parts();
            let bytes = match body.collect().await {
                Ok(body) => body.to_bytes(),
                Err(never) => match never {},
            };
            let body = toolbar::inject(&bytes, &record, &mut parts.headers, self.keep);
            // The rewritten body is this request's own diagnostics, which no
            // shared or private cache may replay; every other response keeps
            // the cache headers the application set, so an immutable asset
            // stays cacheable with the debug bar on.
            parts
                .headers
                .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
            response = hyper::Response::from_parts(
                parts,
                Full::new(body).map_err(|never| match never {}).boxed(),
            );
        }
        response
    }
}

pub(crate) fn cache_store(store: Arc<dyn crate::CacheStore>) -> Arc<dyn crate::CacheStore> {
    // Handles can be retained from bootstrap or a previous request. The observer
    // checks the current collector per operation and skips formatting when absent.
    Arc::new(cache::ObservedCache(store))
}

/// Observe successful pushes without requiring a Debug bound on application jobs.
pub(crate) async fn push_job(
    driver: &dyn crate::QueueDriver,
    envelope: crate::Envelope,
) -> Result<(), FrameworkError> {
    let shown = active().then(|| (envelope.job_name.clone(), format!("{envelope:?}")));
    driver.push(envelope).await?;
    if let Some((name, shown)) = shown {
        record("queue", &name, &shown);
    }
    Ok(())
}

pub(crate) async fn push_jobs(
    driver: &dyn crate::QueueDriver,
    envelopes: Vec<crate::Envelope>,
) -> Result<(), FrameworkError> {
    let shown = active().then(|| {
        envelopes
            .iter()
            .map(|envelope| (envelope.job_name.clone(), format!("{envelope:?}")))
            .collect::<Vec<_>>()
    });
    driver.bulk_push(envelopes).await?;
    if let Some(shown) = shown {
        for (name, shown) in shown {
            record("queue", &name, &shown);
        }
    }
    Ok(())
}
