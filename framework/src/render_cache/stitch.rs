//! Serving prepared hits on stitched routes: a Complete entry replays after
//! the route chain ran; a Composite entry is assembled from re-mounted
//! islands.
//!
//! [`RepresentationClass::PublicShellStitched`](super::RepresentationClass::PublicShellStitched)
//! is the one class whose gate has to run again on every hit, so the global
//! [`RenderCacheMiddleware`](super::middleware::RenderCacheMiddleware) never
//! answers such a route where it stands: it attaches the decoded entry to
//! the request (see `Request::attach_prepared_hit`) and calls the next
//! layer, so the route's own authorization guard, tenant middleware, and
//! anything else the route declared all run exactly as they do on a miss.
//! The Live completion middleware - the last middleware before the handler -
//! is what finally serves the hit, through `serve_prepared`. A request the
//! chain refuses first never reaches here at all, and the prepared hit is
//! dropped unread with the request.

use suprnova_live::render_cache::composite::CompositeEntry;
use suprnova_live::render_cache::entry::DecodedEntry;

use crate::http::{Request, Response};
use crate::middleware::Next;
use crate::telemetry::metrics::Metrics;

use super::RenderCachePolicy;
use super::collector;
use super::middleware::conditional_response;
use super::telemetry as render_cache_telemetry;

/// A hit the RenderCache middleware decoded, checked, and handed to the
/// route chain instead of serving itself.
///
/// Everything [`serve_prepared`] needs to answer the request once the chain
/// has run, and nothing the chain could invalidate: the freshness decision
/// was already made against the entry's own header, and re-deciding it after
/// the chain would compare the same values again.
pub(crate) struct PreparedHit {
    /// The decoded stored representation.
    pub(crate) entry: DecodedEntry,
    /// The route's effective policy, for the served cache metadata.
    pub(crate) policy: RenderCachePolicy,
    /// When the entry was published, for `Age`.
    pub(crate) published_at_ms: u64,
    /// The instant the middleware evaluated freshness at.
    pub(crate) now_ms: u64,
    /// The `Warning` header a stale-servable entry carries, if any.
    pub(crate) warning: Option<&'static str>,
}

/// Marks the handler boundary and serves the hit the RenderCache middleware
/// prepared, if there is one.
///
/// [`collector::begin_handler`] runs first on every request that gets this
/// far, hit or miss: this is the last middleware before the route handler,
/// so everything after it is a content read, and a stitched route is
/// classified from content reads alone. Without it a stitched route's
/// content bucket would be empty on every request and nothing would ever be
/// published (see `CollectorReport::handler_began`).
pub(crate) async fn serve_prepared(mut request: Request, next: Next) -> Response {
    collector::begin_handler();
    let Some(hit) = request.take_prepared_hit() else {
        return next(request).await;
    };
    let hit = *hit;
    match hit.entry {
        DecodedEntry::Complete(entry) => conditional_response(
            request.method().as_str(),
            request.header("if-none-match"),
            &hit.policy,
            &entry,
            hit.published_at_ms,
            hit.now_ms,
            hit.warning,
        ),
        DecodedEntry::Composite(entry) => {
            assemble_hit(
                request,
                next,
                entry,
                hit.policy,
                hit.published_at_ms,
                hit.now_ms,
                hit.warning,
            )
            .await
        }
    }
}

/// Serves a Composite entry.
///
/// Task 8 replaces this body with request-time assembly. Until then a
/// Composite entry is served the way a fail-document outcome is: by the
/// route's own handler, uncached.
async fn assemble_hit(
    request: Request,
    next: Next,
    _entry: CompositeEntry,
    _policy: RenderCachePolicy,
    _published_at_ms: u64,
    _now_ms: u64,
    _warning: Option<&'static str>,
) -> Response {
    count_assembly("fail_document");
    next(request).await
}

/// Counts one composite assembly attempt under its closed outcome label.
fn count_assembly(outcome: &'static str) {
    Metrics::counter(render_cache_telemetry::STITCH_ASSEMBLIES)
        .inc_with(&[(render_cache_telemetry::OUTCOME, outcome)]);
}

/// Counts one slot outcome inside a composite assembly.
#[allow(
    dead_code,
    reason = "Task 8's assembler is the only producer of slot outcomes"
)]
fn count_slot(outcome: &'static str) {
    Metrics::counter(render_cache_telemetry::STITCH_SLOTS)
        .inc_with(&[(render_cache_telemetry::OUTCOME, outcome)]);
}
