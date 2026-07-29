//! Phase 3 dogfood: JSON:API resources + Gate-authorized deletion.
//!
//! Three endpoints:
//!
//! * `GET /api/users/{id}` — JSON:API single-resource envelope via
//!   `Resource::single`. Supports `?fields[users]=...` sparse fieldsets
//!   scoped by `IncludeMiddleware`.
//!
//! * `GET /api/v3/users` — JSON:API collection envelope via
//!   `Resource::collection`.  Same sparse-fieldset support.
//!
//! * `DELETE /api/posts/{id}` — demonstrates `Gate::authorize` with the
//!   `PostPolicy`. The current user is loaded via `Auth::user_as::<User>()`,
//!   which resolves the session's string `user_id` through the registered
//!   `EloquentUserProvider<User>`.

use crate::models::{posts::Post, users::User};
use crate::resources::user_resource::UserResource;
use suprnova::{Auth, FrameworkError, Gate, HttpResponse, Model, Request, Resource, Response};

// ---------------------------------------------------------------------------
// GET /api/users/{id}
// ---------------------------------------------------------------------------

/// Return a single user as a JSON:API resource object.
///
/// Sparse fieldsets are applied automatically by `IncludeMiddleware`;
/// consumers pass `?fields[users]=email` to receive only the listed
/// attributes.
pub async fn show_user(req: Request) -> Response {
    show_user_inner(req).await.map_err(HttpResponse::from)
}

async fn show_user_inner(req: Request) -> Result<HttpResponse, FrameworkError> {
    let raw_id = req.param("id")?;
    let user_id: i64 = raw_id
        .parse()
        .map_err(|_| FrameworkError::param_parse("id", "i64"))?;

    let user = User::find_by_id(user_id)
        .await?
        .ok_or_else(|| FrameworkError::not_found("user"))?;

    Resource::single(UserResource::from(user)).render().await
}

// ---------------------------------------------------------------------------
// GET /api/v3/users
// ---------------------------------------------------------------------------

/// Largest page this endpoint will serve, and its default.
///
/// `User::find_all()` has no bound at all: it materialises every row into
/// memory and then renders every one of them. On a real users table that
/// is an availability problem before it is anything else, and this
/// controller is a worked example people copy.
const MAX_PAGE: u64 = 100;
const DEFAULT_PAGE: u64 = 25;

/// Return a bounded page of users as a JSON:API collection.
///
/// Sparse fieldsets work the same as on the single-resource endpoint.
/// `?limit=` and `?offset=` page through; `limit` is clamped to
/// [`MAX_PAGE`] rather than honoured blindly, because a caller-supplied
/// page size is a caller-supplied amount of server memory.
pub async fn list_users(req: Request) -> Response {
    list_users_inner(req).await.map_err(HttpResponse::from)
}

/// Resolve `(limit, offset)` from the query string.
///
/// Clamped rather than validated-then-rejected: a too-large `limit` is a
/// reasonable thing for a client to ask for and an unreasonable thing for
/// a server to grant, so cap it instead of 400-ing the request. An
/// unparseable value falls back to the default for the same reason —
/// `?limit=all` should serve a page, not an error.
///
/// Extracted so the clamp is testable without a database; the endpoint
/// itself is session-gated.
fn page_bounds(params: &std::collections::HashMap<String, String>) -> (u64, u64) {
    let parse = |key: &str, default: u64| -> u64 {
        params
            .get(key)
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(default)
    };
    (
        parse("limit", DEFAULT_PAGE).clamp(1, MAX_PAGE),
        parse("offset", 0),
    )
}

async fn list_users_inner(req: Request) -> Result<HttpResponse, FrameworkError> {
    let (limit, offset) = page_bounds(&req.query_params());

    let users = User::query().limit(limit).offset(offset).get().await?;
    let resources: Vec<UserResource> = users.into_iter().map(UserResource::from).collect();
    Resource::collection(resources).render().await
}

// ---------------------------------------------------------------------------
// DELETE /api/posts/{id}
// ---------------------------------------------------------------------------

/// Delete a post after authorizing via `Gate::authorize("delete-post", ...)`.
///
/// The gate is registered automatically at boot via the
/// `#[policy(User, Post)]` impl on `PostPolicy` (inventory-based
/// registration). If the current user doesn't own the post the gate
/// returns `Err(FrameworkError::Unauthorized)` which is mapped to 403.
pub async fn delete_post(req: Request) -> Response {
    delete_post_inner(req).await.map_err(HttpResponse::from)
}

async fn delete_post_inner(req: Request) -> Result<HttpResponse, FrameworkError> {
    let raw_id = req.param("id")?;
    let post_id: i64 = raw_id
        .parse()
        .map_err(|_| FrameworkError::param_parse("id", "i64"))?;

    let current_user = Auth::user_as::<User>()
        .await?
        .ok_or(FrameworkError::Unauthorized)?;

    let post = Post::find_by_id(post_id)
        .await?
        .ok_or_else(|| FrameworkError::not_found("post"))?;

    Gate::authorize("delete-post", &current_user, &post)?;
    <Post as suprnova::eloquent::Model>::delete(post).await?;

    Ok(HttpResponse::json(
        suprnova::serde_json::json!({ "deleted": true }),
    ))
}

#[cfg(test)]
mod page_bounds_tests {
    //! `list_users` used to call `User::find_all()` — every row into
    //! memory, every row rendered. These pin the bound that replaced it.

    use super::{DEFAULT_PAGE, MAX_PAGE, page_bounds};
    use std::collections::HashMap;

    fn params(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn no_query_yields_a_bounded_default() {
        let (limit, offset) = page_bounds(&params(&[]));
        assert_eq!(limit, DEFAULT_PAGE);
        assert_eq!(offset, 0);
        assert!(
            limit <= MAX_PAGE,
            "the default must itself be within the cap"
        );
    }

    /// The property that matters: a caller-supplied page size is a
    /// caller-supplied amount of server memory, so it is capped rather
    /// than honoured.
    #[test]
    fn an_oversized_limit_is_clamped_not_honoured() {
        for requested in ["101", "1000", "18446744073709551615"] {
            let (limit, _) = page_bounds(&params(&[("limit", requested)]));
            assert_eq!(
                limit, MAX_PAGE,
                "?limit={requested} must be clamped to {MAX_PAGE}, not served"
            );
        }
    }

    #[test]
    fn a_reasonable_limit_is_respected() {
        let (limit, offset) = page_bounds(&params(&[("limit", "10"), ("offset", "40")]));
        assert_eq!(limit, 10);
        assert_eq!(offset, 40);
    }

    /// Zero would mean "serve nothing forever" and negative parses fail;
    /// both fall to the floor rather than producing an empty or panicking
    /// query.
    #[test]
    fn a_zero_or_unparseable_limit_still_yields_a_usable_page() {
        assert_eq!(page_bounds(&params(&[("limit", "0")])).0, 1);
        assert_eq!(
            page_bounds(&params(&[("limit", "all")])).0,
            DEFAULT_PAGE,
            "an unparseable limit should serve a page, not an error"
        );
        assert_eq!(page_bounds(&params(&[("limit", "-5")])).0, DEFAULT_PAGE);
    }
}
