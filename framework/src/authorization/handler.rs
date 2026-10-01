//! The runtime half of [`#[authorize]`](crate::authorize) on a
//! [`#[handler]`](crate::handler).
//!
//! The macro emits a call to one of these functions for each attribute,
//! after the route parameters are bound and before the request body is read.
//! They are public only because macro output in the application's crate
//! calls them; nothing else should.

use std::any::Any;
use std::sync::Arc;

use super::Gate;
use crate::FrameworkError;
use crate::auth::Auth;

/// Authorize `ability` on `resource` for the request's user, or answer the
/// error the handler returns instead of running.
///
/// The user is the one [`Auth::user`] resolves on the default guard. The
/// macro does not know the application's user type, so the gate is asked
/// about the type-erased user and keys its lookup by the concrete type
/// behind it: the gates, `#[policy]` methods and before-hooks registered
/// for that type all answer, as they would for
/// [`Gate::authorize_async`]. Async gates and async before-hooks answer
/// too, among them the hook
/// [`register_gate_bridge`](crate::rbac::register_gate_bridge) installs.
///
/// # Errors
///
/// - 401 (`Unauthenticated.`) when no user is authenticated.
/// - The denial as [`Gate::authorize_async`] maps it: 403 for a bare
///   denial, or the status a rich [`Response`](super::Response) carries,
///   404 for `Response::deny_as_not_found()`.
/// - The error [`Auth::user`] returns when the user cannot be resolved.
#[doc(hidden)]
pub async fn __authorize_handler<R>(ability: &str, resource: &R) -> Result<(), FrameworkError>
where
    R: Sync + 'static,
{
    let Some(user) = Auth::user().await? else {
        return Err(FrameworkError::domain("Unauthenticated.", 401));
    };
    let user: Arc<dyn Any + Send + Sync> = user.into_arc_any();
    Gate::inspect_erased_async(ability, &*user, resource)
        .await
        .authorize()
        .map(|_| ())
}

/// Authorize `ability` on the type `R` rather than on an instance of it,
/// for the `#[authorize("create", Post)]` form.
///
/// The gate is keyed by type, so a [`Default`] value of `R` stands in for
/// the type, as it does for
/// [`authorize_resource`](crate::ResourceRoutes::authorize_resource).
/// Every `#[suprnova::model]` struct implements `Default`.
///
/// # Errors
///
/// As [`__authorize_handler`].
#[doc(hidden)]
pub async fn __authorize_handler_type<R>(ability: &str) -> Result<(), FrameworkError>
where
    R: Default + Send + Sync + 'static,
{
    let marker = R::default();
    __authorize_handler(ability, &marker).await
}
