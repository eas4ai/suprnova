//! Typed newtypes that an [`Evaluator`](featureflag::evaluator::Evaluator) stashes into a
//! [`Context`](featureflag::context::Context)'s [`Extensions`](featureflag::extensions::Extensions).
//!
//! featureflag's `context!` macro carries fields as a flat
//! `&[(&str, Value)]` slice at creation time. The macro then invokes
//! the active evaluator's [`on_new_context`](featureflag::evaluator::Evaluator::on_new_context)
//! hook, which is where evaluators translate the raw field slice into
//! `TypeId`-keyed values inside `Extensions`. Those values are what
//! [`is_enabled`](featureflag::evaluator::Evaluator::is_enabled) reads
//! on the hot path.
//!
//! We expose these newtypes publicly so:
//!
//! * downstream evaluators (and [`FeatureMiddleware`](crate::features::FeatureMiddleware))
//!   can populate `Extensions` themselves - anything that stashes a
//!   [`UserIdField`] participates in user-scoped flag resolution,
//!   regardless of which evaluator generated the context.
//! * consumers who construct contexts programmatically (without the
//!   `context!` macro) can `extensions_mut().insert(UserIdField::from_i64(42))`
//!   directly when they want to bypass the field-slice indirection.
//!
//! ```rust,no_run
//! use suprnova::features::fields::UserIdField;
//! use featureflag::context::{Context, ContextRef};
//! use featureflag::evaluator::Evaluator;
//! use featureflag::fields::Fields;
//!
//! // An evaluator receives a `ContextRef` in its `on_new_context` hook -
//! // that's where programmatic field insertion happens (rare; most
//! // callers use the `context!` macro instead).
//! struct MyEvaluator;
//! impl Evaluator for MyEvaluator {
//!     fn is_enabled(&self, _feature: &str, _ctx: &Context) -> Option<bool> {
//!         None
//!     }
//!     fn on_new_context(&self, mut ctx_ref: ContextRef<'_>, _fields: Fields<'_>) {
//!         ctx_ref.extensions_mut().insert(UserIdField::from_i64(42));
//!     }
//! }
//! ```
//!
//! # Why `String`?
//!
//! Magnetar (the framework's identity layer) uses opaque string user IDs -
//! UUID-shaped by default, but ultimately whatever the application wants.
//! Numeric-only ids would force every UUID-using app to either re-key
//! their identity model or skip feature-flag scoping entirely. String
//! covers both shapes: numeric apps still get to write
//! `context! { user_id = 42_i64 }` thanks to the
//! [`Evaluator::on_new_context`](featureflag::evaluator::Evaluator::on_new_context)
//! coercion in [`DatabaseEvaluator::on_new_context`](crate::features::DatabaseEvaluator),
//! and the [`UserIdField::as_i64`] helper round-trips back to `i64` for callers
//! that genuinely need the numeric form.
//!
//! # Naming
//!
//! The `Field` suffix is intentional. `UserId` alone collides with
//! `UserId`; the suffix makes it unambiguous that these are
//! feature-flag context fields, not domain identifiers.

/// Authenticated user identity carried in the feature-flag context.
///
/// Carries the application's user identifier as a `String` so opaque
/// (UUID, ULID) ids and numeric ids coexist behind the same shape.
/// Set from the `user_id` field of [`context!`](featureflag::context!) -
/// both string and i64 raw values are accepted; see
/// [`DatabaseEvaluator::on_new_context`](crate::features::DatabaseEvaluator).
/// The [`DatabaseEvaluator`](crate::features::DatabaseEvaluator) reads
/// this to look up `user:{id}`-scoped flags.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct UserIdField(pub String);

impl UserIdField {
    /// Construct from any string-shaped identifier (UUID, ULID, opaque
    /// token). The most common path for Magnetar-issued ids.
    pub fn new<S: Into<String>>(id: S) -> Self {
        Self(id.into())
    }

    /// Construct from a numeric id - the path numeric-keyed apps take
    /// when they don't want to hand-format strings.
    pub fn from_i64(id: i64) -> Self {
        Self(id.to_string())
    }

    /// Borrow the underlying id as `&str`. Cheap; no allocation.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Best-effort parse back to `i64`. Returns `None` when the id is
    /// non-numeric (UUIDs, ULIDs, etc.). Apps that depend on a numeric
    /// `users.id` column still get a clean round-trip.
    pub fn as_i64(&self) -> Option<i64> {
        self.0.parse().ok()
    }
}

/// Team / organization the user belongs to in the feature-flag context.
///
/// Set from the `team` field of [`context!`](featureflag::context!)
/// when the value is a string. The
/// [`DatabaseEvaluator`](crate::features::DatabaseEvaluator) reads
/// this to look up `team:{name}`-scoped flags.
///
/// String-typed (not enum) so applications stay free to define their
/// own team taxonomy without coordinating with the framework.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TeamField(pub String);

impl TeamField {
    /// Construct from any string-shaped team identifier.
    pub fn new<S: Into<String>>(team: S) -> Self {
        Self(team.into())
    }

    /// Borrow the underlying name as `&str`. Cheap; no allocation.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Which identity axes a feature-flag decision depends on.
///
/// Both halves are properties of the **flag**, not of the visitor who read
/// it. A flag with any `user:`-scoped rule depends on the reader's identity
/// even for a reader who falls through to the global rule, because a
/// different reader would have been given a different answer: publishing
/// that reader's page under a key the other reader also hits is the leak.
/// Recording by *matched* scope key instead of by flag scope gets exactly
/// that case wrong (fix round 7, finding 2).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct IdentityScopes {
    /// The flag has at least one rule keyed by the context's user id, so a
    /// read of it depends on the render-cache `Principal` dimension.
    pub(crate) principal: bool,
    /// The flag has at least one rule keyed by the context's team, so a
    /// read of it depends on the render-cache `Tenant` dimension.
    pub(crate) tenant: bool,
}

impl IdentityScopes {
    /// Neither axis: a globally scoped flag, or one with no rules at all.
    const NONE: Self = Self {
        principal: false,
        tenant: false,
    };

    /// The union of two records, used when a nested capture closes back
    /// into the one that contains it.
    fn merged(self, other: Self) -> Self {
        Self {
            principal: self.principal || other.principal,
            tenant: self.tenant || other.tenant,
        }
    }
}

thread_local! {
    /// Which identity axes [`observe_identity`] has been asked to record
    /// since the innermost [`capturing_identity_reads`] started.
    ///
    /// This exists for [`CachedEvaluator`](crate::features::CachedEvaluator)
    /// alone: a cache hit never reaches the inner evaluator, so it can never
    /// learn the flag's scope by itself, and it must replay on a hit exactly
    /// what the miss recorded. Diffing the collector's own observed sets
    /// across the inner call cannot substitute for this, in two ways that
    /// both leak: the miss's value may already be in the set (put there by
    /// an earlier accessor in the same render), and the miss may happen with
    /// no collector active at all (another route, a background job) while
    /// the hit happens inside one. Both make a diff record nothing where the
    /// flag genuinely does depend on identity.
    ///
    /// A thread-local rather than a task-local because
    /// [`Evaluator::is_enabled`](featureflag::evaluator::Evaluator::is_enabled)
    /// is synchronous: there is no await point between arming the capture
    /// and reading it back, so the evaluation cannot migrate to another
    /// thread in between. A panic out of the inner evaluation leaves the
    /// cell reset rather than restored; nothing reads it outside a capture,
    /// and the next capture resets it again.
    static CAPTURED_IDENTITY_READS: std::cell::Cell<IdentityScopes> =
        const { std::cell::Cell::new(IdentityScopes::NONE) };
}

/// Records the context's identity, on the axes `scopes` names, as
/// render-cache observations (fix round 6, Leak 4; narrowed to the flag's
/// own scopes in fix round 7, finding 2, and extended to the team axis in
/// finding 1).
///
/// `FeatureMiddleware` resolves the identity once (via the instrumented
/// [`Auth::id`](crate::Auth::id)) and opens a featureflag
/// [`Context`](featureflag::context::Context) scoped to it *before* the
/// render begins, so that resolution happens outside the RenderCache
/// collector's window entirely; the render then reads `is_enabled!` any
/// number of times, each time consulting the ambient thread-local context
/// rather than touching `Auth::id()` (or any other instrumented accessor)
/// again. Nothing observed the identity that actually shaped the flag
/// decision, so nothing narrowed and the guard had nothing to compare - a
/// real leak, not a hypothetical one, since scoped flags are the documented
/// purpose of this middleware and the reference application installs it.
///
/// The fix instruments the read, not the resolution: every evaluator this
/// framework ships (`DatabaseEvaluator`, `CachedEvaluator`) calls this from
/// inside its own `is_enabled`, which runs *during* the render, inside the
/// collector's window, every time `is_enabled!` is evaluated - including a
/// `CachedEvaluator` cache hit that never reaches its inner evaluator, since
/// a cached answer for a scoped flag is exactly as identity-dependent as a
/// fresh one.
///
/// `scopes` is what makes this a precise fix rather than a blunt one:
/// recording identity for *every* flag read made every page uncacheable for
/// every signed-in visitor, because the reference application installs
/// `FeatureMiddleware` globally and a globally scoped flag's answer does not
/// depend on the reader at all. The team axis records a
/// [`tenant`](crate::render_cache::collector::observe_tenant_value)
/// observation rather than a principal one because that is the dimension a
/// team partitions: a team-scoped decision published under a key with no
/// `Tenant` dimension is served to the next team.
///
/// A custom [`Evaluator`](featureflag::evaluator::Evaluator) outside the two
/// this framework ships remains outside what this can see, the same honest
/// boundary as headers, `Config::get`, and any other undeclared read - see
/// `crate::render_cache::middleware`'s own module doc.
pub(crate) fn observe_identity(scopes: IdentityScopes, context: &featureflag::context::Context) {
    if scopes == IdentityScopes::NONE {
        return;
    }
    // Recorded before the lookups below, and from `scopes` rather than from
    // what the context happened to carry, so a replayed cache hit records
    // the same axes as the miss even when this particular visitor has no id
    // on one of them.
    CAPTURED_IDENTITY_READS.with(|captured| captured.set(captured.get().merged(scopes)));
    if scopes.principal
        && let Some(field) = context
            .iter()
            .find_map(|c| c.extensions().get::<UserIdField>())
    {
        crate::render_cache::collector::observe_principal_value(field.as_str());
    }
    if scopes.tenant
        && let Some(field) = context
            .iter()
            .find_map(|c| c.extensions().get::<TeamField>())
    {
        crate::render_cache::collector::observe_tenant_value(field.as_str());
    }
}

/// Runs `evaluate` and reports which identity axes it recorded through
/// [`observe_identity`].
///
/// Nesting-safe: the axes the inner evaluation recorded are merged back into
/// whatever capture contains this one, so a `CachedEvaluator` wrapping
/// another `CachedEvaluator` does not hide the inner evaluation from the
/// outer entry.
pub(crate) fn capturing_identity_reads<R>(evaluate: impl FnOnce() -> R) -> (R, IdentityScopes) {
    let outer = CAPTURED_IDENTITY_READS.with(|captured| captured.replace(IdentityScopes::NONE));
    let result = evaluate();
    let inner = CAPTURED_IDENTITY_READS.with(std::cell::Cell::get);
    CAPTURED_IDENTITY_READS.with(|captured| captured.set(outer.merged(inner)));
    (result, inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_id_field_round_trips_numeric() {
        let f = UserIdField::from_i64(42);
        assert_eq!(f.as_str(), "42");
        assert_eq!(f.as_i64(), Some(42));
    }

    #[test]
    fn user_id_field_accepts_uuid_shape() {
        let id = "01HZK6V3J7Q5G4P8X9N2D1B0M3"; // ULID
        let f = UserIdField::new(id);
        assert_eq!(f.as_str(), id);
        assert_eq!(f.as_i64(), None, "non-numeric ids return None from as_i64");
    }

    /// A stand-in default evaluator: featureflag panics if a
    /// `Context::root()`-derived context is used while none is installed.
    struct NoopEvaluator;

    impl featureflag::evaluator::Evaluator for NoopEvaluator {
        fn is_enabled(
            &self,
            _feature: &str,
            _context: &featureflag::context::Context,
        ) -> Option<bool> {
            None
        }
    }

    /// The capture is nesting-safe: an inner capture reports its own axes
    /// and also hands them to the capture that contains it, so a
    /// `CachedEvaluator` wrapping another one does not hide the inner
    /// evaluation from the outer entry.
    ///
    /// Verified failing by removing the merge-back line at the end of
    /// `capturing_identity_reads`: the outer capture then reported only the
    /// inner one's axis, having lost its own.
    #[test]
    fn a_nested_capture_reports_its_axes_to_itself_and_to_the_capture_around_it() {
        featureflag::evaluator::with_default(std::sync::Arc::new(NoopEvaluator), || {
            let context = featureflag::context::Context::root();
            let (inner_axes, outer_axes) = capturing_identity_reads(|| {
                observe_identity(
                    IdentityScopes {
                        principal: true,
                        tenant: false,
                    },
                    &context,
                );
                let (_, inner) = capturing_identity_reads(|| {
                    observe_identity(
                        IdentityScopes {
                            principal: false,
                            tenant: true,
                        },
                        &context,
                    );
                });
                inner
            });
            assert_eq!(
                inner_axes,
                IdentityScopes {
                    principal: false,
                    tenant: true
                },
                "the inner capture reports only what it recorded"
            );
            assert_eq!(
                outer_axes,
                IdentityScopes {
                    principal: true,
                    tenant: true
                },
                "the capture around it sees its own axis and the inner one's"
            );
        });
    }

    #[test]
    fn team_field_accessors() {
        let t = TeamField::new("staff");
        assert_eq!(t.as_str(), "staff");
    }
}
