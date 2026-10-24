//! Middleware chain execution engine

use super::{BoxedMiddleware, MiddlewareFuture, Next};
use crate::http::{Request, Response};
use crate::routing::BoxedHandler;
use std::sync::Arc;

/// Builds and executes the middleware chain
///
/// The chain runs from the outside in:
/// 1. Global middleware (first to run)
/// 2. Route-level middleware. Route-*group* middleware is not a distinct
///    runtime layer: group finalization flattens it into each grouped
///    route's `(method, pattern)` middleware (see
///    `routing::group::GroupBuilder::try_finalize`), so by execution time
///    it is indistinguishable from middleware attached to the route
///    directly. Runtime behavior is correct - group middleware still runs
///    ahead of route middleware because it is registered first - but
///    introspection cannot tell group from route middleware apart.
/// 3. The actual route handler (innermost)
pub struct MiddlewareChain {
    middleware: Vec<BoxedMiddleware>,
    outermost: usize,
}

impl MiddlewareChain {
    /// Create a new empty middleware chain
    pub fn new() -> Self {
        Self {
            middleware: Vec::new(),
            outermost: 0,
        }
    }

    /// Create a new middleware chain with enough pre-allocated capacity to
    /// hold `n` entries without re-allocating as middleware is pushed or
    /// extended into it.
    ///
    /// The request hot path builds a chain of `1 (RequestId) + global.len() + route.len()`
    /// entries on every request. Using [`Self::new()`] there forces the backing `Vec`
    /// to grow (and copy) two-to-three times as the chain is assembled via `push` then
    /// `extend` then `extend`. Pre-sizing to the known total collapses that to a single
    /// allocation, eliminating per-request re-allocation overhead.
    pub fn with_capacity(n: usize) -> Self {
        Self {
            middleware: Vec::with_capacity(n),
            outermost: 0,
        }
    }

    /// Add middleware to the chain
    ///
    /// Registration order is retained except for security policy placement
    /// and the configured priority list (see [`Self::execute`]).
    pub fn push(&mut self, middleware: BoxedMiddleware) {
        self.middleware.push(middleware);
    }

    /// Reserve a server lifecycle wrapper outside application policy sorting.
    /// This keeps request-id scope around security and its error handling.
    pub(crate) fn push_outermost(&mut self, middleware: BoxedMiddleware) {
        self.middleware.insert(self.outermost, middleware);
        self.outermost += 1;
    }

    /// Add multiple middleware to the chain
    pub fn extend(&mut self, middleware: impl IntoIterator<Item = BoxedMiddleware>) {
        self.middleware.extend(middleware);
    }

    /// Avoid a server policy scope when the application did not install one.
    pub(crate) fn has_security_headers(&self) -> bool {
        self.middleware.iter().any(|middleware| {
            super::identity::type_of(middleware)
                == Some(std::any::TypeId::of::<crate::SecurityHeaders>())
        })
    }

    /// Execute the middleware chain with the given request and final handler
    ///
    /// The chain is executed from outside-in:
    /// - Security policy and route overrides precede other application middleware
    /// - Other middleware keep registration order unless priority moves them
    /// - Each middleware can call `next(request)` to continue the chain
    /// - The final handler is called at the end of the chain
    ///
    /// # Priority
    ///
    /// Typed [`SecurityHeaders`](crate::SecurityHeaders) registration always
    /// runs outside caches and error middleware. Route and group
    /// [`SecurityHeadersOverride`](crate::SecurityHeadersOverride) instances run
    /// immediately inside it, before cache lookup, and
    /// [`ConsentPageHeaders`](crate::ConsentPageHeaders) inside those. The server's request-id
    /// lifecycle wrapper remains outside application middleware.
    ///
    /// The middleware priority list
    /// ([`append_middleware_priority`](crate::middleware::append_middleware_priority))
    /// is applied first. A middleware the list names moves in front of any
    /// middleware the list places after it, whichever was added first.
    /// Every other middleware keeps its place. This is the one point every
    /// request path runs a chain through, so the order holds for a matched
    /// route, the fallback, an unrouted request and a WebSocket upgrade
    /// alike.
    ///
    /// # Panics
    ///
    /// This composition primitive does NOT catch panics. A panic in any
    /// middleware or in the handler unwinds straight out of `execute`,
    /// exactly like any other async fn. The request-path safety net that
    /// converts a panic into a sanitized 500 (plus the structured log and
    /// the `ErrorOccurred` dispatch) lives at the server boundary -
    /// `server::execute_chain_safely` for HTTP, `handle_ws_upgrade` for
    /// WebSocket upgrades - so that standardized handling happens exactly
    /// once, where the request lifecycle owns it, rather than being
    /// duplicated inside this layer-agnostic primitive. The end-to-end
    /// behavior is locked by `tests/middleware_panic_safety.rs`. A consumer
    /// driving a chain outside that boundary is responsible for its own
    /// `catch_unwind` if it wants the same guarantee.
    pub async fn execute(self, request: Request, handler: Arc<BoxedHandler>) -> Response {
        if self.middleware.is_empty() {
            // No middleware - call handler directly
            return dispatch(request, &handler).await;
        }
        let middleware = super::identity::sort_by_priority(self.middleware, self.outermost);

        // Build the chain from inside-out
        // Start with the actual handler as the innermost "next"
        let handler_clone = handler.clone();
        let mut next: Next = Arc::new(move |req| dispatch(req, &handler_clone));

        // Wrap each middleware around the next, from last to first
        // This creates the correct execution order: first middleware runs first
        for middleware in middleware.into_iter().rev() {
            let current_next = next;
            let name =
                crate::debugbar::active().then(|| super::identity::diagnostic_name(&middleware));
            let mw = middleware;
            next = Arc::new(move |req| {
                let n = current_next.clone();
                let m = mw.clone();
                if !crate::debugbar::active() {
                    return m(req, n);
                }
                let name = name.clone().unwrap_or_else(|| "boxed middleware".into());
                Box::pin(async move {
                    crate::debugbar::note_request(&req);
                    let children = Arc::new(std::sync::Mutex::new(std::time::Duration::ZERO));
                    let child_time = Arc::clone(&children);
                    let wrapped: Next = Arc::new(move |request| {
                        let next = Arc::clone(&n);
                        let time = Arc::clone(&child_time);
                        Box::pin(async move {
                            let _time = crate::debugbar::next_span(time);
                            let response = next(request).await;
                            crate::debugbar::note_context();
                            response
                        })
                    });
                    let _span = crate::debugbar::span(&name, Some(children));
                    m(req, wrapped).await
                }) as MiddlewareFuture
            });
        }

        // Execute the outermost middleware (which was the first added)
        next(request).await
    }
}

fn dispatch(request: Request, handler: &Arc<BoxedHandler>) -> MiddlewareFuture {
    crate::debugbar::note_request(&request);
    let span = crate::debugbar::span("handler", None);
    let future = dispatch_inner(request, handler);
    if let Some(span) = span {
        Box::pin(async move {
            let _span = span;
            future.await
        })
    } else {
        future
    }
}

fn dispatch_inner(request: Request, handler: &Arc<BoxedHandler>) -> MiddlewareFuture {
    // Closures and plain functions have only a Request argument. Typed
    // handlers stop after extraction in the handler macro instead.
    if request.is_precognitive()
        && request
            .route_settings()
            .is_some_and(|route| route.skips_precognitive_body())
    {
        Box::pin(async { Err(crate::FrameworkError::PrecognitionSuccess.into()) })
    } else {
        let handler = handler.clone();
        Box::pin(crate::render_cache::inertia_page::serve_prepared(
            request,
            Arc::new(move |request| handler(request)),
        ))
    }
}

impl Default for MiddlewareChain {
    fn default() -> Self {
        Self::new()
    }
}
