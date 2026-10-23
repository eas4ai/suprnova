//! The one process-wide featureflag global default every `render_cache`
//! test-support module shares.
//!
//! `suprnova::features::install_evaluator` reaches
//! `featureflag::evaluator::set_global_default`, which is backed by a
//! genuine process-wide `OnceLock` (`featureflag::evaluator::global`): only
//! the first caller's evaluator ever becomes reachable through
//! `is_enabled!`, and nothing can replace it afterward - there is no reset,
//! by design, matching a real application's single boot-time install.
//!
//! Two independent test-support modules in this one test binary
//! (`render_cache_middleware_support`, `render_cache_privacy_support`) each
//! drive `is_enabled!` over real HTTP dispatch and each need their own,
//! independently seeded, non-colliding flag names visible through that one
//! slot. `featureflag::evaluator::with_default` cannot substitute for the
//! global default here: both suites run the render inside a
//! `tokio::spawn`'d request-handling task, and `with_default`'s scope is a
//! synchronous closure that returns (and un-sets) before a spawned task
//! ever gets polled - see either module's own note on this.
//!
//! Before this module existed, each of the two called
//! `suprnova::features::install_evaluator` independently from its own
//! process-lifetime `OnceCell`, so whichever module's tests happened to run
//! first in the process silently won the slot forever and the other
//! module's flags were never visible: `is_enabled!` on the loser's flag
//! names fell through to the caller-supplied default because the winner's
//! evaluator had no rows for them at all. This module is the single place
//! that installs the slot instead: a `Chain` of both modules' evaluators,
//! built and seeded exactly once regardless of which module's tests happen
//! to run first, so the final installed evaluator no longer depends on
//! that race.
//!
//! `Chain::is_enabled` tries its first evaluator and falls through to the
//! second only when the first answers `None` (an unknown flag), so each
//! module's own flag reads are answered identically to what its own
//! standalone evaluator gave before this fix - the other side is only ever
//! consulted for a name it does not itself know. And an evaluator that does
//! not know a flag records no render-cache dependency for it
//! (`IdentityScopes::NONE` short-circuits
//! `suprnova::features::fields::observe_feature_read` before it touches the
//! collector), so neither module's observations leak into the other's.

use std::sync::Arc;
use std::time::Duration;

use featureflag::evaluator::EvaluatorExt;
use suprnova::features::{CachedEvaluator, DatabaseEvaluator};

static INSTALL: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

/// Seeds `render_cache_middleware_support`'s four flags. Names and rules
/// are that module's own; repeated here rather than imported because they
/// were private consts inline in its `install_feature_evaluator` before
/// this fix and this shared installer is not that module's public
/// contract.
///
/// The four cover the whole scope matrix fix round 7 turns on:
/// `user-scoped-flag` has a rule at the reader's own identity;
/// `team-scoped-flag` at the reader's team; `global-flag` at neither; and
/// `another-users-override-flag` has a rule at *someone else's* identity
/// plus a global rule, which is the case that distinguishes "record by flag
/// scope" from "record by the scope key that matched this reader".
async fn build_middleware_evaluator() -> Arc<DatabaseEvaluator> {
    let evaluator = Arc::new(
        DatabaseEvaluator::new_in_memory()
            .await
            .expect("in-memory feature evaluator (middleware support)"),
    );
    evaluator
        .set_flag("user-scoped-flag", "user:alice", true)
        .await
        .expect("seed the user-scoped flag");
    evaluator
        .set_flag("team-scoped-flag", "team:alpha", true)
        .await
        .expect("seed the team-scoped flag");
    evaluator
        .set_flag("global-flag", "", true)
        .await
        .expect("seed the globally scoped flag");
    evaluator
        .set_flag("another-users-override-flag", "", false)
        .await
        .expect("seed the global rule of the override flag");
    evaluator
        .set_flag("another-users-override-flag", "user:bob", true)
        .await
        .expect("seed bob's override");
    evaluator
}

/// Seeds `render_cache_privacy_support`'s three flags, wrapped in a
/// `CachedEvaluator` so a hit replays what the miss consulted - the same
/// evaluator shape that module's own installer built before this fix.
async fn build_privacy_evaluator() -> Arc<CachedEvaluator> {
    let database = DatabaseEvaluator::new_in_memory()
        .await
        .expect("in-memory feature evaluator (privacy support)");
    database
        .set_flag("privacy-user-scoped-flag", "user:alice", true)
        .await
        .expect("seed the user-scoped flag");
    database
        .set_flag("privacy-global-flag", "", true)
        .await
        .expect("seed the globally scoped flag");
    database
        .set_flag("privacy-override-flag", "", false)
        .await
        .expect("seed the global rule of the override flag");
    database
        .set_flag("privacy-override-flag", "user:bob", true)
        .await
        .expect("seed bob's override");
    Arc::new(CachedEvaluator::new(
        Arc::new(database),
        // Far longer than any test in this binary runs, so a second read
        // of one flag by one context is always a hit and never an expiry.
        Duration::from_secs(3_600),
    ))
}

/// Installs the combined evaluator as featureflag's process-global
/// default, exactly once. Every `render_cache` test-support module that
/// needs `is_enabled!` over real HTTP dispatch calls this instead of
/// installing its own.
pub async fn install() {
    INSTALL
        .get_or_init(|| async {
            let middleware = build_middleware_evaluator().await;
            let privacy = build_privacy_evaluator().await;
            suprnova::features::install_evaluator(Arc::new(middleware.chain(privacy)));
        })
        .await;
}
