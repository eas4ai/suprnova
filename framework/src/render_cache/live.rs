//! Live document facts: public-seed documents are Complete representations
//! bounded by the seed deadline; identity-bound islands wait for stitching.

use suprnova_live::view::DocumentCachePolicy;
use suprnova_live::view::DocumentResponseIntent;

use crate::live::LiveMountKind;

/// What a request's Live mounts and rendered document, if any, told the
/// cache. Accumulates rather than replaces: a document that mounts more
/// than one island, or a handler that mounts more than one `LiveDocument`
/// in the same request, folds every mount's facts together (counts add,
/// the deadline takes the minimum), and `no_store` is sticky once set.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LiveDocumentFacts {
    /// Number of public-seed islands mounted so far.
    pub public_seed_islands: usize,
    /// Number of identity-bound islands mounted so far.
    pub identity_bound_islands: usize,
    /// Earliest public-seed promotion deadline among the mounted islands.
    pub seed_deadline_ms: Option<u64>,
    /// Whether any rendered document in this request declared `NoStore`.
    /// Sticky: sets to `true` and never resets (see this struct's own doc).
    pub no_store: bool,
}

/// Records one successful island mount into the active collector; a no-op
/// outside a scope. Called from `LiveDocument::mount`, immediately after a
/// mount succeeds and before the caller can do anything else with it - not
/// from `render`, because `MountedIsland::html()` is `pub` and `TrustedHtml`
/// is `Display`, so a handler can mount an island and hand-build its own
/// response without ever calling `render` at all. Recording at `mount`
/// means the fact exists regardless of whether `render` is ever reached.
pub fn record_mount(kind: LiveMountKind, seed_deadline_ms: Option<u64>) {
    super::collector::observe_live_document_mount(kind, seed_deadline_ms);
}

/// Records a rendered document's cache intent into the active collector; a
/// no-op outside a scope. Only `NoStore` is recorded: `Private` and
/// `Public` neither narrow nor widen the server-side cache's class (see
/// [`document_declines`]'s own doc for why), so recording them here would
/// have nothing to do with them. Called from `LiveDocument::render`, the
/// only place an intent is known at all - a handler that bypasses `render`
/// has no intent to honor, which is exactly why the mount facts above are
/// captured earlier, at `mount`, rather than here.
pub fn record_document_intent(intent: &DocumentResponseIntent) {
    if intent.cache() == DocumentCachePolicy::NoStore {
        super::collector::observe_live_document_no_store();
    }
}

/// Whether the Live facts recorded so far forbid storing this render at
/// all: any identity-bound island, a document that declared `NoStore`, or
/// a public-seed island whose deadline could not be resolved. Returns
/// `false` - never a class - for everything else; this can only decline,
/// never narrow or widen `classify`'s own output.
///
/// Deliberately absent: a document's `Private`/`Public` cache intent.
/// `DocumentResponseIntent::html()` defaults to `Private`, so mapping it to
/// `RepresentationClass::PrivateCached` would demote every Live document,
/// with no `ClassificationReason` behind the demotion for
/// `key_used_different_values_than_the_render_saw` to check the key
/// against. The route's declared `RenderCachePolicy` - not the document's
/// intent - is this server-side cache's class; the intent governs only the
/// downstream `Cache-Control` a browser or CDN sees. `NoStore` still
/// declines, because an author who said "do not store" meant this cache too.
#[must_use]
pub fn document_declines(facts: Option<&LiveDocumentFacts>) -> bool {
    let Some(facts) = facts else {
        return false;
    };
    facts.identity_bound_islands > 0
        || facts.no_store
        || (facts.public_seed_islands > 0 && facts.seed_deadline_ms.is_none())
}

/// Milliseconds until the seed deadline, `Some(0)` when it has passed, `None` without seeds.
#[must_use]
pub fn seed_remaining_ms(facts: &LiveDocumentFacts, now_ms: u64) -> Option<u64> {
    facts
        .seed_deadline_ms
        .map(|deadline| deadline.saturating_sub(now_ms))
}
