//! Route middleware for RBAC checks.

use std::marker::PhantomData;
use std::sync::Arc;

use async_trait::async_trait;

use crate::auth::Auth;
use crate::error::FrameworkError;
use crate::http::{HttpResponse, Request, Response};
use crate::middleware::{Middleware, Next};

use super::HasRoles;
use super::has_roles::{has_permission_for_model_on_guard, has_role_for_model_on_guard};

/// The user the route's guard authenticated, as `U`, and the guard whose
/// grants apply when it is not the default guard.
///
/// The route's guard is the one the last `AuthMiddleware` that passed the
/// request on checked, as for `#[authorize]`. A user of another guard,
/// such as the default guard's user in the same session, never stands in
/// for it. The default guard's grants are the ones the `HasRoles` methods
/// read and write, under `"web"`; another guard's grants live under that
/// guard's name, where the `*_on_guard` helpers write them, as Spatie's
/// `role:editor,admin` reads the `admin` guard's roles.
async fn route_subject<U: HasRoles>() -> Result<Option<(Arc<U>, Option<String>)>, FrameworkError> {
    let Some(user) = Auth::route_user().await? else {
        return Ok(None);
    };
    let Ok(user) = user.into_arc_any().downcast::<U>() else {
        return Ok(None);
    };
    Ok(Some((user, Auth::route_guard_other_than_default())))
}

fn unauthorized_response(redirect_to: Option<&str>, request: &Request) -> HttpResponse {
    match redirect_to {
        Some(path) if request.is_inertia() => HttpResponse::text("")
            .status(409)
            .header("X-Inertia-Location", path),
        Some(path) => HttpResponse::new().status(302).header("Location", path),
        None => FrameworkError::Unauthorized.into(),
    }
}

/// Middleware that requires the authenticated user to have a role.
///
/// `U` is the concrete authenticated user type stored by [`Auth`].
/// Compose after [`crate::AuthMiddleware`] so unauthenticated users are
/// handled consistently before role checks run. The user checked is the
/// one the route's guard authenticated: behind
/// `AuthMiddleware::new().for_guard("admin")`, the `admin` user, with the
/// roles granted on the `admin` guard.
pub struct RoleMiddleware<U> {
    role: String,
    redirect_to: Option<String>,
    marker: PhantomData<fn() -> U>,
}

impl<U> RoleMiddleware<U> {
    /// Create middleware requiring `role`.
    pub fn new(role: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            redirect_to: None,
            marker: PhantomData,
        }
    }

    /// Create middleware requiring `role`, redirecting denials to `path`.
    ///
    /// Inertia requests receive `409` with `X-Inertia-Location`; normal browser
    /// requests receive `302 Location`.
    pub fn redirect_to(role: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            redirect_to: Some(path.into()),
            marker: PhantomData,
        }
    }

    fn unauthorized_response(&self, request: &Request) -> HttpResponse {
        unauthorized_response(self.redirect_to.as_deref(), request)
    }
}

#[async_trait]
impl<U> Middleware for RoleMiddleware<U>
where
    U: HasRoles,
{
    async fn handle(&self, request: Request, next: Next) -> Response {
        let Some((user, guard)) = route_subject::<U>().await? else {
            return Err(self.unauthorized_response(&request));
        };
        let allowed = match guard {
            None => user.has_role(&self.role).await?,
            Some(guard) => {
                has_role_for_model_on_guard(
                    &user.rbac_model_type(),
                    &user.rbac_model_id(),
                    &self.role,
                    &guard,
                )
                .await?
            }
        };

        if allowed {
            next(request).await
        } else {
            Err(self.unauthorized_response(&request))
        }
    }
}

/// Middleware that requires the authenticated user to have a permission.
///
/// Direct model permissions and permissions inherited through assigned roles
/// are both accepted. The user checked is the one the route's guard
/// authenticated, with the grants of that guard, as for [`RoleMiddleware`].
pub struct PermissionMiddleware<U> {
    permission: String,
    redirect_to: Option<String>,
    marker: PhantomData<fn() -> U>,
}

impl<U> PermissionMiddleware<U> {
    /// Create middleware requiring `permission`.
    pub fn new(permission: impl Into<String>) -> Self {
        Self {
            permission: permission.into(),
            redirect_to: None,
            marker: PhantomData,
        }
    }

    /// Create middleware requiring `permission`, redirecting denials to `path`.
    ///
    /// Inertia requests receive `409` with `X-Inertia-Location`; normal browser
    /// requests receive `302 Location`.
    pub fn redirect_to(permission: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            permission: permission.into(),
            redirect_to: Some(path.into()),
            marker: PhantomData,
        }
    }

    fn unauthorized_response(&self, request: &Request) -> HttpResponse {
        unauthorized_response(self.redirect_to.as_deref(), request)
    }
}

#[async_trait]
impl<U> Middleware for PermissionMiddleware<U>
where
    U: HasRoles,
{
    async fn handle(&self, request: Request, next: Next) -> Response {
        let Some((user, guard)) = route_subject::<U>().await? else {
            return Err(self.unauthorized_response(&request));
        };
        let allowed = match guard {
            None => user.has_permission_to(&self.permission).await?,
            Some(guard) => {
                has_permission_for_model_on_guard(
                    &user.rbac_model_type(),
                    &user.rbac_model_id(),
                    &self.permission,
                    &guard,
                )
                .await?
            }
        };

        if allowed {
            next(request).await
        } else {
            Err(self.unauthorized_response(&request))
        }
    }
}
