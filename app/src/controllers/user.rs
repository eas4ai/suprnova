use suprnova::{
    FrameworkError, HttpResponse, Inertia, InertiaResponse, Model, Paginator, Request, Response,
    json_response, redirect, route,
};

use crate::models::users::User;
use crate::props::{PublicUserProps, UserDetailProps};

/// Page size for the user directory. Fixed rather than caller-supplied:
/// `/api/users` accepts `?per_page=` and clamps it, but this route is the
/// plain HTML/Inertia directory and has no reason to expose the knob.
const PER_PAGE: u64 = 20;

/// `GET /users` - one page of the public user directory.
///
/// `simple_paginate`, not `paginate`: the length-aware paginator issues a
/// `COUNT(*)` beside the page query, and counting a users table of any
/// real size costs more than serving the page does. Laravel's
/// `simplePaginate()` makes the same trade. The cost is that the response
/// carries "is there a next page" instead of "how many pages are there",
/// which is what the overflow probe can honestly answer.
pub async fn index(req: Request) -> Response {
    index_inner(req).await.map_err(HttpResponse::from)
}

async fn index_inner(req: Request) -> Result<HttpResponse, FrameworkError> {
    let page = <User as Model>::query()
        .order_by_asc("id")
        .simple_paginate(PER_PAGE)
        .await?;

    // Rebuild rather than map: `Paginator` has no `map`/`through` (see
    // the note in `controllers::paginated_users`). The counters carry
    // across unchanged - they describe the query, not the row shape.
    let projected = Paginator::new(
        page.data
            .into_iter()
            .map(|u| PublicUserProps {
                id: u.id,
                name: u.name,
            })
            .collect(),
        page.current_page,
        page.per_page,
        page.has_more,
    );

    Inertia::paginate("Users/Index", "users", projected)
        .resolve(&req)
        .await
}

/// `GET /users/{id}` - one user, with their profile eager-loaded.
///
/// The eager load is the point of this route in the dogfood: it is a
/// primary-key fetch plus a `HasOne` in a single round trip, which is the
/// shape most "show one record and its detail" pages take.
pub async fn show(req: Request) -> Response {
    show_inner(req).await.map_err(HttpResponse::from)
}

async fn show_inner(req: Request) -> Result<HttpResponse, FrameworkError> {
    let id: i64 = req
        .param("id")?
        .parse()
        .map_err(|_| FrameworkError::param_parse("id", "i64"))?;

    let user = <User as Model>::query()
        .with(["profile"])
        .filter("id", id)
        .first()
        .await?
        .ok_or_else(|| FrameworkError::not_found("user"))?;

    // `None` here means "this user has no profile row", not "the eager
    // load missed" - the loader distinguishes the two, and the HasOne
    // test in `relations_dogfood` pins that it does not borrow a
    // neighbour's profile to fill the gap.
    let bio = user.profile_loaded().map(|p| p.bio.clone());

    InertiaResponse::new("Users/Show")
        .with(
            "user",
            UserDetailProps {
                id: user.id,
                name: user.name,
                bio,
            },
        )
        .resolve(&req)
        .await
}

/// Example: Create a user and redirect to the user list
pub async fn store(_req: Request) -> Response {
    // ... create user logic would go here ...

    // Redirect to users.index (compile-time validated!)
    redirect!("users.index").into()
}

/// Example: Redirect to a specific user with query params
pub async fn redirect_example(_req: Request) -> Response {
    // Generate a URL using route()
    let url = route("users.show", &[("id", "42")]);
    println!("Generated URL: {:?}", url);

    // Redirect with query parameters (compile-time validated!)
    redirect!("users.index")
        .query("page", "1")
        .query("sort", "name")
        .into()
}

/// Example: Inertia redirect that preserves the URL fragment across
/// the redirect. The destination's `InertiaResponse` will emit
/// `preserveFragment: true` so the client carries over its current
/// `#anchor` to the new URL.
///
/// Maps to Laravel's `redirect()->preserveFragment()`.
pub async fn preserve_fragment_example(_req: Request) -> Response {
    redirect!("users.index").preserve_fragment().into()
}

/// Example: opt out of SSR for this specific request. The destination
/// renders client-side only even when `InertiaConfig::ssr.enabled` is
/// `true`. Useful for routes that depend on per-request state SSR
/// can't see (geolocation, session-only flash, etc.) or for debugging.
///
/// Maps to Laravel's `Inertia::disable_ssr()`.
pub async fn ssr_opt_out_example(_req: Request) -> Response {
    suprnova::App::disable_ssr_for_request();
    json_response!({
        "ssr_disabled_for_this_request": true,
        "note": "If SSR is enabled globally, this route still renders CSR-only.",
    })
}
