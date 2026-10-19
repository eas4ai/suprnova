//! User controller -- list, show, register, login.
//!
//! `list_users` and `show_user` query the canonical `app_users` table.
//! Registration and login use Magnetar through `Auth::password()`. Both
//! bodies are form requests that fold the address before their rules run
//! (`crate::email_address`), so the rules check what is stored, and an
//! address over 254 characters, longer than any deliverable one, gets a
//! `422` naming the field.

use serde::Deserialize;
use suprnova::{
    Auth, FormRequest, FrameworkError, HttpResponse, Request, Resource, Response, Validate, handler,
};

use crate::email_address;
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

#[derive(Deserialize, Validate)]
pub struct RegisterRequest {
    #[validate(
        email(message = "Please enter a valid email address"),
        length(max = 254, message = "Email must be at most 254 characters")
    )]
    pub email: String,
    pub password: String,
}

impl FormRequest for RegisterRequest {
    /// Fold the address before the rules run, so they check what is stored.
    fn prepare_for_validation(req: &mut Request) -> Result<(), FrameworkError> {
        req.transform_input("email", |email| email_address::normalize(&email));
        Ok(())
    }
}

#[handler]
pub async fn register(body: RegisterRequest) -> Response {
    // An address that already has an account is `Registration::Accepted`:
    // its account is neither changed nor returned. Both outcomes get the
    // same answer, so this endpoint does not reveal which addresses are
    // registered. The client signs in through `POST /api/auth/login`.
    let _registration = Auth::password()
        .register(&body.email, &body.password)
        .await?;

    Ok(HttpResponse::json(serde_json::json!({
        "message": "Registration received. Sign in with your email and password."
    }))
    .status(202))
}

// ============================================================================
// POST /api/auth/login
// ============================================================================

#[derive(Deserialize, Validate)]
pub struct LoginRequest {
    #[validate(
        email(message = "Please enter a valid email address"),
        length(max = 254, message = "Email must be at most 254 characters")
    )]
    pub email: String,
    pub password: String,
}

impl FormRequest for LoginRequest {
    /// Fold the address before the rules run, so they check what is looked
    /// up.
    fn prepare_for_validation(req: &mut Request) -> Result<(), FrameworkError> {
        req.transform_input("email", |email| email_address::normalize(&email));
        Ok(())
    }
}

#[handler]
pub async fn login(body: LoginRequest) -> Response {
    let (_user, session) = Auth::password()
        .authenticate(&body.email, &body.password, None, None)
        .await?;

    let token = session
        .token
        .ok_or_else(|| FrameworkError::internal("authentication did not return a session token"))?;

    // Secret tokens redact their `Display` and `Debug` output. Exposure is
    // explicit only at the response boundary.
    suprnova::json_response!({
        "token": token.expose_secret()
    })
}
