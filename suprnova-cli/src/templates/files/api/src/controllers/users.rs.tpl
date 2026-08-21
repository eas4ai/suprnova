//! User controller -- list, show, register, login.
//!
//! `list_users` and `show_user` query the canonical `app_users` table.
//! Registration and login use Magnetar through `Auth::password()`.

use serde::Deserialize;
use suprnova::{handler, Auth, FrameworkError, Request, Resource, Response};

use crate::models::user::User;
use crate::resources::user_resource::UserResource;

// ============================================================================
// GET /api/users
// ============================================================================

#[handler]
pub async fn list_users(_req: Request) -> Response {
    let users: Vec<UserResource> = User::all()
        .await?
        .into_iter()
        .map(UserResource::from)
        .collect();

    Ok(Resource::collection(users).render().await?)
}

// ============================================================================
// GET /api/users/:id
// ============================================================================

#[handler]
pub async fn show_user(req: Request) -> Response {
    let id: i64 = req
        .param("id")
        .map_err(FrameworkError::from)?
        .parse()
        .map_err(|_| FrameworkError::param_parse("id", "i64"))?;

    let user = User::find_by_id(id)
        .await?
        .ok_or_else(|| FrameworkError::model_not_found("User"))?;

    Ok(Resource::single(UserResource::from(user)).render().await?)
}

// ============================================================================
// POST /api/auth/register
// ============================================================================

#[derive(Deserialize)]
pub struct RegisterRequest {
    pub email: String,
    pub password: String,
}

#[handler]
pub async fn register(req: Request) -> Response {
    let body: RegisterRequest = req
        .json()
        .await
        .map_err(|e| FrameworkError::bad_request(e.to_string()))?;

    let user = Auth::password()
        .register(&body.email, &body.password)
        .await?;

    Ok(Resource::single(UserResource {
        id: user.id.to_string(),
        email: user.email,
    })
    .render()
    .await?)
}

// ============================================================================
// POST /api/auth/login
// ============================================================================

#[derive(Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[handler]
pub async fn login(req: Request) -> Response {
    let body: LoginRequest = req
        .json()
        .await
        .map_err(|e| FrameworkError::bad_request(e.to_string()))?;

    let (_user, session) = Auth::password()
        .authenticate(&body.email, &body.password, None, None)
        .await?;

    let token = session.token.ok_or_else(|| {
        FrameworkError::internal("authentication did not return a session token")
    })?;

    // Secret tokens redact their `Display` and `Debug` output. Exposure is
    // explicit only at the response boundary.
    suprnova::json_response!({
        "token": token.expose_secret()
    })
}
