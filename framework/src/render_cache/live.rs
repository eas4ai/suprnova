//! Live document facts: public-seed documents are Complete representations
//! bounded by the seed deadline; identity-bound islands wait for stitching.

use std::fmt;

use bytes::Bytes;
use suprnova_live::identity::IslandSlot;
use suprnova_live::mount::DocumentMountKey;
use suprnova_live::render_cache::composite::ShellIsland;
use suprnova_live::view::DocumentCachePolicy;
use suprnova_live::view::DocumentResponseIntent;

use crate::live::{LiveMountKind, StitchSlotDescriptor};
use crate::render_cache::RepresentationClass;

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
    /// What a stitched shell would need to publish this document, recorded
    /// as the document was assembled. Default on a request that neither
    /// mounted an island nor rendered a document, and meaningless unless the
    /// route declared [`RepresentationClass::PublicShellStitched`].
    pub stitch: StitchCapture,
}

/// One identity-bound island as the document emitted it: what a later hit
/// needs to mount it again, and the exact bytes it has to replace.
///
/// The bytes are the island's own markup, taken from the `TrustedHtml` the
/// mount produced and handed to the template - not a re-render and not a
/// re-serialization - so a shell built by locating them inside the rendered
/// document finds them byte for byte or not at all.
#[derive(Clone, Eq, PartialEq)]
pub struct CapturedSlot {
    /// The typed declaration this slot re-mounts from.
    pub descriptor: StitchSlotDescriptor,
    /// The island markup this mount emitted.
    pub html: Bytes,
}

impl fmt::Debug for CapturedSlot {
    /// Prints the declaration and the island's length, never the island.
    ///
    /// An identity-bound island's markup is that principal's own rendered
    /// state and carries its signed snapshot in
    /// `data-suprnova-live-snapshot`. This type is reachable from a public
    /// derived `Debug` - [`StitchCapture`] to [`LiveDocumentFacts`] to
    /// [`super::collector::CollectorReport`], which
    /// [`super::collector::current_report`] hands to any caller - so a
    /// derived `Debug` here would put a snapshot and a user's HTML into
    /// whatever formatted a report. The same rule the engine applies to
    /// `TrustedHtml` and `DocumentRender`.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CapturedSlot")
            .field("descriptor", &self.descriptor)
            .field("html_bytes", &self.html.len())
            .finish()
    }
}

/// Everything one request's Live documents recorded for a stitched shell.
///
/// Accumulates across every mount and every document in the request, in the
/// order they happened, which is also the order they appear in the rendered
/// body. Nothing here decides anything on its own: a route that never
/// declared [`RepresentationClass::PublicShellStitched`] ignores all of it,
/// and [`document_declines`] is the only reader that turns `invalid` into a
/// decision.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StitchCapture {
    /// Identity-bound islands, in mount order.
    pub slots: Vec<CapturedSlot>,
    /// Public-seed islands, which stay inside the shell, in mount order.
    pub shell_islands: Vec<ShellIsland>,
    /// The Content Security Policy nonce the bootstrap markup stamped, if any.
    pub nonce: Option<String>,
    /// SHA-256 of the rendered document body.
    pub document_digest: Option<[u8; 32]>,
    /// Whether the capture is not a faithful account of this request: a
    /// bound was exceeded, two documents were rendered, or two bootstraps
    /// disagreed about the nonce. A stitched route declines rather than
    /// publishing a shell built from an account it cannot trust.
    pub invalid: bool,
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

/// Records one identity-bound island and its exact emitted bytes into the
/// active collector; a no-op outside a scope. Called from
/// `LiveDocument::mount`, at the same point as [`record_mount`] and for the
/// same reason: the mount is where the island's bytes and its declaration
/// are both known, and a handler that never calls `render` still made them.
pub fn record_stitch_slot(slot: CapturedSlot) {
    super::collector::observe_live_document_stitch_slot(slot);
}

/// Records one public-seed island as remaining inside the shell; a no-op
/// outside a scope. A public seed is the same for everybody, so a stitched
/// shell keeps its bytes and only records that the island is there.
pub fn record_shell_island(slot: &IslandSlot, key: &DocumentMountKey) {
    super::collector::observe_live_document_shell_island(ShellIsland {
        slot: slot.as_str().to_owned(),
        document_key: key.as_str().to_owned(),
    });
}

/// Records the Content Security Policy nonce one document's bootstrap
/// markup stamped; a no-op outside a scope, and a no-op for a bootstrap
/// that stamped no nonce, which cut no holes to fill. Called from
/// `LiveDocument::bootstrap` once the markup is accepted, so the recorded
/// value is the one the emitted script elements carry.
pub fn record_bootstrap_nonce(nonce: Option<&str>) {
    super::collector::observe_live_document_bootstrap_nonce(nonce);
}

/// Records the SHA-256 of one rendered document body; a no-op outside a
/// scope. Called from `LiveDocument::render` for the same bytes the
/// response carries, so a shell can be checked against the document it was
/// cut from.
pub fn record_document_digest(digest: [u8; 32]) {
    super::collector::observe_live_document_digest(digest);
}

/// Records that this request's capture cannot be represented at all - a
/// mount whose canonical parameters exceed the slot bound is the only
/// current producer - so a stitched route declines instead of publishing a
/// shell that is missing one of its islands. A no-op outside a scope.
pub fn record_stitch_capture_invalid() {
    super::collector::observe_live_document_stitch_invalid();
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
/// all: an identity-bound island on a route that did not declare stitching,
/// a stitched route whose capture is not trustworthy, a document that
/// declared `NoStore`, or a public-seed island whose deadline could not be
/// resolved. Returns `false` - never a class - for everything else; this can
/// only decline, never narrow or widen `classify`'s own output.
///
/// `declared` is the route's own declared class, not the class `classify`
/// produced: an identity-bound island is exactly what a
/// [`RepresentationClass::PublicShellStitched`] route exists to re-render on
/// every hit, so declaring that class is what turns the island from a reason
/// to decline into the reason to stitch. Every other class keeps the earlier
/// behavior unchanged, including a route that narrowed to `PrivateCached`.
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
pub fn document_declines(facts: Option<&LiveDocumentFacts>, declared: RepresentationClass) -> bool {
    let Some(facts) = facts else {
        return false;
    };
    let stitched = declared == RepresentationClass::PublicShellStitched;
    (facts.identity_bound_islands > 0 && !stitched)
        || (stitched && facts.stitch.invalid)
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
