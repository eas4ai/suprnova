//! Handlers the router does not reach through a route's own record: a
//! route's `missing()` handler, and a recorded `#[handler]` a closure route
//! calls. Their bound arguments bind under the settings of the route they
//! answer, as the route's own handler's do: in path order (BIND-015), by
//! its binding fields (BIND-004), scoped through the parent (BIND-006),
//! through the router's binders (BIND-007) and with `with_trashed()`
//! (BIND-008). A closure route answers its `missing()` handler (BIND-009);
//! a `missing()` handler whose own binding finds nothing answers 404.

use chrono::{DateTime, Utc};
use suprnova::http::{HttpResponse, text};
use suprnova::testing::TestDatabase;
use suprnova::{FromRequest, Model, Request, Response, Router, handler, model};

use super::{get as get_path, refusal, run_sql, serve};

#[model(table = "ou_users", soft_deletes, relations = {
    posts: HasMany<OuPost>,
})]
pub struct OuUser {
    pub id: i64,
    pub name: String,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[model(table = "ou_posts")]
pub struct OuPost {
    pub id: i64,
    pub ou_user_id: i64,
    pub slug: String,
}

#[handler]
pub async fn user_post(user: OuUser, post: OuPost) -> Response {
    text(format!("{} {}", user.name, post.slug))
}

/// [`user_post`] with its arguments declared against the path's order.
#[handler]
pub async fn post_user(post: OuPost, user: OuUser) -> Response {
    text(format!("{} {}", user.name, post.slug))
}

#[handler]
pub async fn show_post(post: OuPost) -> Response {
    text(post.slug)
}

#[handler]
pub async fn show_user(user: OuUser) -> Response {
    text(user.name)
}

/// A generic route handler, which carries no record.
#[handler]
pub async fn generic_user_post<B: FromRequest + Send + 'static>(
    user: OuUser,
    post: OuPost,
    _extra: B,
) -> Response {
    text(format!("{} {}", user.name, post.slug))
}

/// [`user_post`] with a return type that cannot carry a `missing()`
/// response.
#[handler]
pub async fn user_post_result(
    user: OuUser,
    post: OuPost,
) -> Result<HttpResponse, suprnova::FrameworkError> {
    Ok(HttpResponse::text(format!("{} {}", user.name, post.slug)))
}

/// A `missing()` handler that binds the child that missed and its parent.
#[handler]
pub async fn missing_user_post(user: OuUser, post: OuPost) -> Response {
    Ok(HttpResponse::text(format!("hook {} {}", user.name, post.slug)).status(302))
}

/// [`missing_user_post`] with its arguments declared against the path's
/// order.
#[handler]
pub async fn missing_post_user(post: OuPost, user: OuUser) -> Response {
    Ok(HttpResponse::text(format!("hook {} {}", user.name, post.slug)).status(302))
}

/// A `missing()` handler that binds the parent only.
#[handler]
pub async fn missing_user(user: OuUser) -> Response {
    Ok(HttpResponse::text(format!("no post for {}", user.name)).status(302))
}

/// A generic `missing()` handler, which carries no record.
#[handler]
pub async fn missing_generic<B: FromRequest + Send + 'static>(
    user: OuUser,
    post: OuPost,
    _extra: B,
) -> Response {
    Ok(HttpResponse::text(format!("hook {} {}", user.name, post.slug)).status(302))
}

/// A plain `missing()` handler: it sees the request.
async fn redirect_home(request: Request) -> Response {
    Ok(HttpResponse::text(format!("missing at {}", request.path())).status(302))
}

/// What a closure route does with the response of the handler it calls:
/// it marks it, so the closure is more than the handler under another name.
fn via_closure(response: Response) -> Response {
    response.map(|response| response.header("X-Via", "closure"))
}

/// Find a user by name, for `Router::bind`.
async fn user_by_name(
    value: String,
    _route: suprnova::MatchedRoute,
) -> Result<Option<OuUser>, suprnova::FrameworkError> {
    OuUser::query().filter("name", value).first().await
}

async fn fixture() -> TestDatabase {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    run_sql(
        &db,
        &[
            "CREATE TABLE ou_users (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, \
                deleted_at TEXT)",
            "CREATE TABLE ou_posts (id INTEGER PRIMARY KEY AUTOINCREMENT, \
                ou_user_id INTEGER NOT NULL, slug TEXT NOT NULL)",
            "INSERT INTO ou_users (id, name, deleted_at) VALUES (1, 'ada', NULL), \
                (2, 'grace', NULL), (3, 'eve', '2026-01-01T00:00:00+00:00')",
            "INSERT INTO ou_posts (id, ou_user_id, slug) VALUES (1, 1, 'ada-post'), \
                (2, 2, 'grace-post'), (3, 3, 'eve-post')",
        ],
    )
    .await;
    db
}

// ── A route's `missing()` handler ───────────────────────────────────────────

#[tokio::test]
async fn bind_009_a_missing_handler_never_binds_a_row_the_parent_does_not_own() {
    // `/users/1/posts/2` names Grace's post under Ada. The route misses it,
    // and so must its `missing()` handler, whatever order it declares its
    // arguments in: it answers 404 instead of running on Grace's post.
    let _db = fixture().await;
    let router: Router = Router::new()
        .get("/users/{user}/posts/{post}", user_post)
        .scope_bindings()
        .missing(missing_user_post)
        .get("/reversed/users/{user}/posts/{post}", user_post)
        .scope_bindings()
        .missing(missing_post_user)
        .get("/parent/users/{user}/posts/{post}", user_post)
        .scope_bindings()
        .missing(missing_user)
        .into();
    let addr = serve(router).await;
    for path in ["/users/1/posts/2", "/reversed/users/1/posts/2"] {
        let (status, body) = get_path(addr, path).await;
        assert_eq!(status, 404, "{path}: {body}");
        assert!(!body.contains("grace-post"), "{path}: {body}");
    }
    // A handler that binds the parent alone runs, on the parent the route
    // bound.
    assert_eq!(
        get_path(addr, "/parent/users/1/posts/2").await,
        (302, "no post for ada".to_owned())
    );
    // The route binds as before.
    assert_eq!(
        get_path(addr, "/users/1/posts/1").await,
        (200, "ada ada-post".to_owned())
    );
}

#[tokio::test]
async fn bind_009_a_missing_handler_binds_by_the_routes_field_binder_and_with_trashed() {
    let _db = fixture().await;
    let router: Router = Router::new()
        .get("/named/users/{user:name}/posts/{post}", user_post)
        .missing(missing_user)
        .get("/trashed/users/{user}/posts/{post}", user_post)
        .with_trashed()
        .missing(missing_user)
        .into();
    let addr = serve(router).await;
    // The route's binding field: `grace` is a name, not a key.
    assert_eq!(
        get_path(addr, "/named/users/grace/posts/99").await,
        (302, "no post for grace".to_owned())
    );
    // `with_trashed()`: Eve is soft-deleted.
    assert_eq!(
        get_path(addr, "/trashed/users/3/posts/99").await,
        (302, "no post for eve".to_owned())
    );

    // A binder covers the `missing()` handler's argument too.
    let router: Router = Router::new()
        .get("/users/{user}/posts/{post}", user_post)
        .missing(missing_user)
        .into();
    let addr = serve(router.bind("user", user_by_name)).await;
    assert_eq!(
        get_path(addr, "/users/grace/posts/99").await,
        (302, "no post for grace".to_owned())
    );
}

#[tokio::test]
async fn bind_009_a_generic_missing_handler_and_one_on_a_generic_route_bind_under_the_route() {
    let _db = fixture().await;
    let router: Router = Router::new()
        .get("/users/{user}/posts/{post}", user_post)
        .scope_bindings()
        .missing(missing_generic::<Request>)
        .get(
            "/generic/users/{user}/posts/{post}",
            generic_user_post::<Request>,
        )
        .scope_bindings()
        .missing(missing_user_post)
        .into();
    let addr = serve(router).await;
    for path in ["/users/1/posts/2", "/generic/users/1/posts/2"] {
        let (status, body) = get_path(addr, path).await;
        assert_eq!(status, 404, "{path}: {body}");
        assert!(!body.contains("grace-post"), "{path}: {body}");
    }
    assert_eq!(
        get_path(addr, "/generic/users/1/posts/1").await,
        (200, "ada ada-post".to_owned())
    );
}

#[tokio::test]
async fn bind_007_a_missing_handler_bound_by_a_binder_of_another_type_is_refused_at_startup() {
    // The route's handler does not bind `user`; its `missing()` handler
    // does, and the binder returns a `String`.
    let router: Router = Router::new()
        .get("/users/{user}/posts/{post}", show_post)
        .missing(missing_user)
        .into();
    let router = router.bind(
        "user",
        |value: String, _route| async move { Ok(Some(value)) },
    );
    let error = refusal(&router);
    assert!(error.contains("GET /users/{user}/posts/{post}"), "{error}");
    assert!(error.contains("missing_user"), "{error}");
    assert!(error.contains("String"), "{error}");
}

// ── A recorded handler a closure route calls ───────────────────────────────

#[tokio::test]
async fn bind_006_a_handler_a_closure_route_calls_binds_under_the_routes_settings() {
    let _db = fixture().await;
    let router: Router = Router::new()
        .get("/scoped/users/{user}/posts/{post}", |req| async move {
            via_closure(user_post(req).await)
        })
        .scope_bindings()
        .get("/reversed/users/{user}/posts/{post}", |req| async move {
            via_closure(post_user(req).await)
        })
        .scope_bindings()
        .get("/fields/users/{user}/posts/{post:slug}", |req| async move {
            via_closure(user_post(req).await)
        })
        .get("/plain/users/{user}/posts/{post}", |req| async move {
            via_closure(user_post(req).await)
        })
        .into();
    let addr = serve(router).await;
    // Scoped by `scope_bindings()` in either declaration order, and by a
    // binding field.
    for path in [
        "/scoped/users/1/posts/2",
        "/reversed/users/1/posts/2",
        "/fields/users/1/posts/grace-post",
    ] {
        assert_eq!(get_path(addr, path).await.0, 404, "{path}");
    }
    assert_eq!(
        get_path(addr, "/reversed/users/1/posts/1").await,
        (200, "ada ada-post".to_owned())
    );
    assert_eq!(
        get_path(addr, "/fields/users/1/posts/ada-post").await,
        (200, "ada ada-post".to_owned())
    );
    // The control: nothing scopes the plain route.
    assert_eq!(
        get_path(addr, "/plain/users/1/posts/2").await,
        (200, "ada grace-post".to_owned())
    );
}

#[tokio::test]
async fn bind_008_a_handler_a_closure_route_calls_binds_through_binders_trashed_rows_and_missing() {
    let _db = fixture().await;
    let router: Router = Router::new()
        .get("/trashed/users/{user}", |req| async move {
            via_closure(show_user(req).await)
        })
        .with_trashed()
        .get("/users/{user}", |req| async move {
            via_closure(show_user(req).await)
        })
        .get("/posts/{post}", |req| async move {
            via_closure(show_post(req).await)
        })
        .missing(redirect_home)
        .into();
    let addr = serve(router).await;
    assert_eq!(get_path(addr, "/users/3").await.0, 404);
    assert_eq!(
        get_path(addr, "/trashed/users/3").await,
        (200, "eve".to_owned())
    );
    assert_eq!(
        get_path(addr, "/posts/99").await,
        (302, "missing at /posts/99".to_owned())
    );

    let router: Router = Router::new()
        .get("/users/{user}", |req| async move {
            via_closure(show_user(req).await)
        })
        .into();
    let addr = serve(router.bind("user", user_by_name)).await;
    assert_eq!(
        get_path(addr, "/users/grace").await,
        (200, "grace".to_owned())
    );
}

#[tokio::test]
async fn bind_006_a_handler_of_another_return_type_a_closure_calls_binds_under_the_route() {
    // Its return type cannot carry the route's `missing()` response, so a
    // miss answers its 404 error; it still never binds an unowned row.
    let _db = fixture().await;
    let router: Router = Router::new()
        .get("/users/{user}/posts/{post}", |req| async move {
            user_post_result(req).await.map_err(HttpResponse::from)
        })
        .scope_bindings()
        .missing(redirect_home)
        .into();
    let addr = serve(router).await;
    let (status, body) = get_path(addr, "/users/1/posts/2").await;
    assert_eq!(status, 404, "{body}");
    assert!(!body.contains("grace-post"), "{body}");
    assert_eq!(
        get_path(addr, "/users/1/posts/1").await,
        (200, "ada ada-post".to_owned())
    );
}
