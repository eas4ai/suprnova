//! Policy-based authorization gates.
//!
//! Mirrors Laravel's `Gate` facade: register named abilities (or full
//! `Policy` impls keyed by resource type) against the global gate, then
//! call `Gate::allows("update", &user, &post)` from anywhere - controllers,
//! middleware, Inertia view models.
//!
//! The [`Authorizable`] shim adds `user.can("update", &post)` directly on
//! the user type for a more fluent call site.
//!
//! # Authorizing a handler
//!
//! [`#[authorize]`](crate::authorize) on a [`#[handler]`](crate::handler)
//! declares the check instead of calling the gate in the body, where it
//! only works if nobody forgets it. It names a parameter the route binds:
//!
//! ```rust,no_run
//! use suprnova::http::text;
//! use suprnova::{Response, RouteParam, authorize, handler};
//!
//! #[suprnova::model(table = "posts")]
//! pub struct Post {
//!     pub id: i64,
//! }
//!
//! #[handler]
//! #[authorize("update", post)]
//! pub async fn update(post: RouteParam<Post>) -> Response {
//!     text(format!("updated {}", post.id))
//! }
//! # fn main() {}
//! ```
//!
//! A name the handler does not take as a parameter does not compile:
//!
//! ```compile_fail
//! use suprnova::http::text;
//! use suprnova::{Response, RouteParam, authorize, handler};
//!
//! #[suprnova::model(table = "posts")]
//! pub struct Post {
//!     pub id: i64,
//! }
//!
//! #[handler]
//! #[authorize("update", post)]
//! pub async fn update(article: RouteParam<Post>) -> Response {
//!     text(format!("updated {}", article.id))
//! }
//! # fn main() {}
//! ```

mod gate;
mod registry;
mod response;

pub use gate::Gate;
pub use response::Response;

/// User-side ergonomic shim for [`Gate`]: `user.can(action, &resource)`
/// instead of `Gate::allows(action, &user, &resource)`. Mirrors
/// Laravel's `Authorizable` trait.
///
/// `impl Authorizable for YourUser {}` is enough - every method has
/// a default body that delegates to [`Gate`]. The trait requires
/// `Sized + 'static` so the type-erased registry can dispatch via
/// `TypeId` (same constraints [`Gate::define`] imposes).
pub trait Authorizable: Sized + 'static {
    /// `true` iff the gate registered for `(action, Self, R)` allows.
    /// Missing gates deny by default.
    ///
    /// Synchronous, so async before-hooks do not run, neither their allows
    /// nor their denials - among them the hook
    /// [`register_gate_bridge`](crate::rbac::register_gate_bridge) installs.
    /// Ask [`Authorizable::can_async`] when a permission should answer.
    fn can<R: 'static>(&self, action: &str, resource: &R) -> bool {
        Gate::allows(action, self, resource)
    }
    /// Opposite of [`Authorizable::can`].
    ///
    /// Skips async before-hooks like [`Authorizable::can`], so a denial from
    /// [`Gate::before_async`] is not enforced here; see
    /// [`Authorizable::cannot_async`].
    fn cannot<R: 'static>(&self, action: &str, resource: &R) -> bool {
        Gate::denies(action, self, resource)
    }
    /// Authorize the action, returning the denial as an error.
    ///
    /// A bare denial maps to `FrameworkError::Unauthorized` (403). A rich
    /// denial - from a [`Gate::define_with`] gate that returned a [`Response`]
    /// with a custom message/status - maps to `FrameworkError::Domain`
    /// carrying that message and status (e.g. 404 from
    /// `Response::deny_as_not_found()`).
    ///
    /// Synchronous like [`Authorizable::can`], with the same limit: an async
    /// before-hook's denial is not enforced here, and a permission connected
    /// by [`register_gate_bridge`](crate::rbac::register_gate_bridge) answers
    /// [`Authorizable::authorize_async`] only.
    fn authorize<R: 'static>(
        &self,
        action: &str,
        resource: &R,
    ) -> Result<(), crate::FrameworkError> {
        Gate::authorize(action, self, resource)
    }
    /// Async sibling of [`Authorizable::can`].
    fn can_async<'a, R>(
        &'a self,
        action: &'a str,
        resource: &'a R,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = bool> + Send + 'a>>
    where
        Self: Sync,
        R: 'static + Sync,
    {
        Box::pin(Gate::allows_async(action, self, resource))
    }
    /// Async sibling of [`Authorizable::cannot`].
    fn cannot_async<'a, R>(
        &'a self,
        action: &'a str,
        resource: &'a R,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = bool> + Send + 'a>>
    where
        Self: Sync,
        R: 'static + Sync,
    {
        Box::pin(Gate::denies_async(action, self, resource))
    }
    /// Async sibling of [`Authorizable::authorize`]. Same error mapping:
    /// bare denials become `FrameworkError::Unauthorized`, rich denials
    /// (from `Gate::define_async_with`) become `FrameworkError::Domain`
    /// preserving the message and status.
    fn authorize_async<'a, R>(
        &'a self,
        action: &'a str,
        resource: &'a R,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<(), crate::FrameworkError>> + Send + 'a>,
    >
    where
        Self: Sync,
        R: 'static + Sync,
    {
        Box::pin(Gate::authorize_async(action, self, resource))
    }
}

// ── inventory-based policy registration ──────────────────────────────────────

/// Registration record emitted by `#[policy]` via `inventory::submit!`.
///
/// `register` is a zero-arg closure that calls `Gate::define` (for a `bool`
/// method) or `Gate::define_with` (for a `Response` method) for one action.
#[doc(hidden)]
pub struct __PolicyRegistration {
    pub register: fn(),
}

inventory::collect!(__PolicyRegistration);

/// Eagerly run all `#[policy]` gate registrations.
///
/// Called automatically from `Server::serve`. May also be called manually in
/// tests. Safe to call multiple times - the inner `Once` ensures each
/// registered closure runs exactly once.
pub fn init_policies() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        for reg in inventory::iter::<__PolicyRegistration> {
            (reg.register)();
        }
    });
}
