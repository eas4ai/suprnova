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

/// Records the context's resolved user id, if any, as a render-cache
/// principal observation (fix round 6, Leak 4).
///
/// `FeatureMiddleware` resolves the identity once (via the instrumented
/// [`Auth::id`](crate::Auth::id)) and opens a featureflag
/// [`Context`](featureflag::context::Context) scoped to it *before* the
/// render begins, so that resolution happens outside the RenderCache
/// collector's window entirely; the render then
/// reads `is_enabled!` any number of times, each time consulting the
/// ambient thread-local context rather than touching `Auth::id()` (or any
/// other instrumented accessor) again. Nothing observed the identity that
/// actually shaped the flag decision, so nothing narrowed and the guard had
/// nothing to compare - a real leak, not a hypothetical one, since
/// user-scoped flags are the documented purpose of this middleware and the
/// reference application installs it.
///
/// The fix instruments the read, not the resolution: every evaluator this
/// framework ships (`DatabaseEvaluator`, `CachedEvaluator`) calls this at
/// the top of its own `is_enabled`, which runs *during* the render, inside
/// the collector's window, every single time `is_enabled!` is evaluated -
/// including a `CachedEvaluator` cache hit that never reaches its inner
/// evaluator, since a cached answer for a user-scoped flag is exactly as
/// dependent on that user's identity as a fresh one. A custom [`Evaluator`](featureflag::evaluator::Evaluator)
/// outside these two remains outside what this can see, the same honest
/// boundary as headers, `Config::get`, and any other undeclared read - see
/// `crate::render_cache::middleware`'s own module doc.
pub(crate) fn observe_context_principal(context: &featureflag::context::Context) {
    if let Some(field) = context
        .iter()
        .find_map(|c| c.extensions().get::<UserIdField>())
    {
        crate::render_cache::collector::observe_principal_value(field.as_str());
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

    #[test]
    fn team_field_accessors() {
        let t = TeamField::new("staff");
        assert_eq!(t.as_str(), "staff");
    }
}
