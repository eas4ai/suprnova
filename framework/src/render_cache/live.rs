//! Live document facts: public-seed documents are Complete representations
//! bounded by the seed deadline; identity-bound islands wait for stitching.

use suprnova_live::render_cache::RepresentationClass;
use suprnova_live::view::{DocumentCachePolicy, DocumentResponseIntent};

use crate::live::LiveMountKind;

/// What a Live document render told the cache.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiveDocumentFacts {
    /// The document's typed cache intent.
    pub cache: DocumentCachePolicy,
    /// Number of public-seed islands.
    pub public_seed_islands: usize,
    /// Number of identity-bound islands.
    pub identity_bound_islands: usize,
    /// Earliest public-seed expiry among the mounted islands.
    pub seed_deadline_ms: Option<u64>,
}

/// Records the facts into the active collector; a no-op outside a scope.
pub fn record_document(
    intent: &DocumentResponseIntent,
    kinds: &[LiveMountKind],
    seed_deadline_ms: Option<u64>,
) {
    let facts = LiveDocumentFacts {
        cache: intent.cache(),
        public_seed_islands: kinds
            .iter()
            .filter(|kind| **kind == LiveMountKind::PublicSeed)
            .count(),
        identity_bound_islands: kinds
            .iter()
            .filter(|kind| **kind == LiveMountKind::IdentityBound)
            .count(),
        seed_deadline_ms,
    };
    super::collector::observe_live_document(facts);
}

/// Narrows the classified class with the document's facts. A route with no
/// Live document (`facts` is `None`) keeps whatever `classify` decided.
#[must_use]
pub fn document_class(
    facts: Option<&LiveDocumentFacts>,
    classified: RepresentationClass,
) -> RepresentationClass {
    let Some(facts) = facts else {
        return classified;
    };
    if facts.identity_bound_islands > 0 || facts.cache == DocumentCachePolicy::NoStore {
        return RepresentationClass::Uncacheable;
    }
    if facts.public_seed_islands > 0 && facts.seed_deadline_ms.is_none() {
        return RepresentationClass::Uncacheable;
    }
    match facts.cache {
        DocumentCachePolicy::Private => classified.narrowest(RepresentationClass::PrivateCached),
        DocumentCachePolicy::Public | DocumentCachePolicy::NoStore => classified,
    }
}

/// Milliseconds until the seed deadline, `Some(0)` when it has passed, `None` without seeds.
#[must_use]
pub fn seed_remaining_ms(facts: &LiveDocumentFacts, now_ms: u64) -> Option<u64> {
    facts
        .seed_deadline_ms
        .map(|deadline| deadline.saturating_sub(now_ms))
}
