//! `GET /api/users` - cursor pagination over the real `users` table.
//!
//! Dogfoods the framework's keyset pagination end-to-end:
//! `Builder::cursor_paginate` for the query, `Inertia::paginate` for the
//! response, and `IntoInertiaScroll` for the scroll metadata the Inertia
//! v3 protocol attaches under `scrollProps.<key>`.
//!
//! This route used to page a 100-row in-memory `Vec` and reimplement
//! keyset paging by hand - decoding the cursor, filtering above or below
//! the boundary, detecting overflow, and re-encoding both neighbours,
//! about 80 lines of it. All of that is what `cursor_paginate` already
//! does against a database, so the fixture version was dogfooding a
//! parallel implementation rather than the framework's. It also could not
//! answer the question a benchmark asks - how does cursor pagination
//! behave over a table too large to hold in memory - because the whole
//! dataset was rebuilt per request.
//!
//! Rows are projected to [`PublicUserProps`], which omits `email`. This
//! route requires no session; see that type's module docs.
//!
//! Query params:
//! - `per_page` (default 20, clamped to 100)
//! - `cursor` (opaque, encrypted+MAC'd; read by `cursor_paginate` itself)
//! - `format=json` → raw paginator JSON instead of an Inertia response

use suprnova::{
    CursorPaginator, FrameworkError, HttpResponse, Inertia, IntoInertiaScroll, Model, Request,
    Response,
};

use crate::models::users::User;
use crate::props::PublicUserProps;

const DEFAULT_PER_PAGE: u64 = 20;

/// Ceiling on `?per_page=`. An endpoint that lets the caller pick its own
/// page size has only moved an unbounded response into a query parameter.
const MAX_PER_PAGE: u64 = 100;

/// Fetch one page and project it to the public shape.
///
/// The projection rebuilds the paginator rather than mapping in place:
/// `CursorPaginator` has no `map`/`through` (Laravel's paginators do -
/// a parity gap worth closing), but its fields are public, so carrying
/// the cursors across is a matter of moving them. The cursors stay valid
/// because they encode the *keyset boundary*, which is the user's `id` -
/// a property of the query, not of the shape the rows are serialised in.
async fn build_page(req: &Request) -> Result<CursorPaginator<PublicUserProps>, FrameworkError> {
    let per_page = req
        .query_param("per_page")
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(DEFAULT_PER_PAGE)
        .clamp(1, MAX_PER_PAGE);

    let page = <User as Model>::query().cursor_paginate(per_page).await?;

    Ok(CursorPaginator::new(
        page.data
            .into_iter()
            .map(|u| PublicUserProps {
                id: u.id,
                name: u.name,
            })
            .collect(),
        page.per_page,
        page.next_cursor,
        page.prev_cursor,
    ))
}

/// `GET /api/users[?cursor=<opaque>][&per_page=N][&format=json]`
///
/// Returns an Inertia response by default (the `Users/Index` page
/// component, with `props.users` set to the cursor-paginated rows and
/// scroll metadata wired through). Pass `?format=json` to receive the raw
/// paginator as JSON.
pub async fn index(req: Request) -> Response {
    index_inner(req).await.map_err(HttpResponse::from)
}

async fn index_inner(req: Request) -> Result<HttpResponse, FrameworkError> {
    let paginator = build_page(&req).await?;

    if req.query_param("format").as_deref() == Some("json") {
        // Raw JSON view of the paginator, through the same
        // `IntoInertiaScroll` bridge so the wire shape cannot drift from
        // the Inertia path.
        let (meta, data) = paginator.into_inertia_scroll();
        return Ok(HttpResponse::json(suprnova::serde_json::json!({
            "data": data,
            "meta": {
                "page_name": meta.page_name,
                "next": meta.next_page,
                "previous": meta.previous_page,
                "current": meta.current_page,
            },
        })));
    }

    Inertia::paginate("Users/Index", "users", paginator)
        .resolve(&req)
        .await
}
