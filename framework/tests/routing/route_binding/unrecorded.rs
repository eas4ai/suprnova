//! Handlers the router does not reach through a route's own record: a
//! route's `missing()` handler, a recorded `#[handler]` a closure route
//! calls, and one a recorded route's handler calls with its request. Their bound arguments bind under the settings of the route they
//! answer, as the route's own handler's do: in path order (BIND-015), by
//! its binding fields (BIND-004), scoped through the parent (BIND-006),
//! through the router's binders (BIND-007) and with `with_trashed()`
//! (BIND-008). A closure route answers its `missing()` handler (BIND-009);
//! a `missing()` handler whose own binding finds nothing answers 404.

use std::future::Future;

use chrono::{DateTime, Utc};
use suprnova::http::{HttpResponse, text};
use suprnova::testing::TestDatabase;
use suprnova::{
    FromRequest, Middleware, MiddlewareRegistry, Model, Next, Request, Response, RouteBinding,
    RouteParam, Router, handler, model,
};

use super::{get as get_path, refusal, run_sql, send, serve, serve_with};

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

/// A recorded route handler that hands its request to [`user_post`].
#[handler]
pub async fn forward_user_post(req: Request) -> Response {
    user_post(req).await
}

/// A recorded route handler that binds the user itself, then hands its
/// request to [`user_post`].
#[handler]
pub async fn bound_forward_user_post(user: OuUser, req: Request) -> Response {
    let _ = user;
    user_post(req).await
}

/// A recorded route handler that hands its request to [`show_user`].
#[handler]
pub async fn forward_show_user(req: Request) -> Response {
    show_user(req).await
}

/// A recorded route handler that hands its request to [`show_post`].
#[handler]
pub async fn forward_show_post(req: Request) -> Response {
    show_post(req).await
}

/// A `missing()` handler that binds the child alone, not its parent.
#[handler]
pub async fn missing_post(post: OuPost) -> Response {
    Ok(HttpResponse::text(format!("hook {}", post.slug)).status(302))
}

/// A recorded route handler that binds the user, then hands its request to
/// [`show_post`], which binds the child alone.
#[handler]
pub async fn bound_forward_show_post(user: OuUser, req: Request) -> Response {
    let _ = user;
    show_post(req).await
}

/// A recorded route handler that binds nothing.
#[handler]
pub async fn binds_nothing(req: Request) -> Response {
    text(format!("route at {}", req.path()))
}

/// A generic handler whose bound argument's type is its type parameter.
#[handler]
pub async fn show_item<T: RouteBinding>(item: RouteParam<T>) -> Response {
    let kind = std::any::type_name::<T>()
        .rsplit("::")
        .next()
        .unwrap_or_default();
    text(format!("{kind} {}", item.route_key()))
}

/// Route middleware that answers with a `#[handler]` it calls on the
/// request, before the route binds anything.
pub struct AnswerWith(fn(Request) -> std::pin::Pin<Box<dyn Future<Output = Response> + Send>>);

#[suprnova::async_trait]
impl Middleware for AnswerWith {
    async fn handle(&self, request: Request, _next: Next) -> Response {
        (self.0)(request).await
    }
}

fn answer_user_post(req: Request) -> std::pin::Pin<Box<dyn Future<Output = Response> + Send>> {
    Box::pin(user_post(req))
}

fn answer_show_post(req: Request) -> std::pin::Pin<Box<dyn Future<Output = Response> + Send>> {
    Box::pin(show_post(req))
}

fn answer_user_post_result(
    req: Request,
) -> std::pin::Pin<Box<dyn Future<Output = Response> + Send>> {
    Box::pin(async move { user_post_result(req).await.map_err(HttpResponse::from) })
}

/// Calls the handler and answers its error with a 404 of its own, which
/// carries no error report.
fn answer_user_post_result_own_404(
    req: Request,
) -> std::pin::Pin<Box<dyn Future<Output = Response> + Send>> {
    Box::pin(async move {
        user_post_result(req)
            .await
            .map_err(|_| HttpResponse::text("gone").status(404))
    })
}

/// Calls the handler and recovers from its error with a 200.
fn answer_user_post_result_recovered(
    req: Request,
) -> std::pin::Pin<Box<dyn Future<Output = Response> + Send>> {
    Box::pin(async move {
        Ok(user_post_result(req)
            .await
            .unwrap_or_else(|_| HttpResponse::text("recovered")))
    })
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
    // Its return type cannot carry the route's `missing()` response; the
    // route answers it all the same, and never binds an unowned row.
    let _db = fixture().await;
    let router: Router = Router::new()
        .get("/users/{user}/posts/{post}", |req| async move {
            user_post_result(req).await.map_err(HttpResponse::from)
        })
        .scope_bindings()
        .missing(redirect_home)
        .into();
    let addr = serve(router).await;
    assert_eq!(
        get_path(addr, "/users/1/posts/2").await,
        (302, "missing at /users/1/posts/2".to_owned()),
        "the route's missing() response, though the handler cannot return it"
    );
    assert_eq!(
        get_path(addr, "/users/1/posts/1").await,
        (200, "ada ada-post".to_owned())
    );
}

// ── A recorded handler another route handler calls ───────────────────────

#[tokio::test]
async fn bind_006_a_handler_a_route_handler_calls_never_binds_a_row_the_parent_does_not_own() {
    // The route's own handler takes the request and hands it on; the
    // handler it calls binds under the route's settings.
    let _db = fixture().await;
    let router: Router = Router::new()
        .get("/users/{user}/posts/{post}", forward_user_post)
        .scope_bindings()
        .get("/bound/users/{user}/posts/{post}", bound_forward_user_post)
        .scope_bindings()
        .get("/fields/users/{user}/posts/{post:slug}", forward_user_post)
        .get("/plain/users/{user}/posts/{post}", forward_user_post)
        .into();
    let addr = serve(router).await;
    for path in [
        "/users/1/posts/2",
        "/bound/users/1/posts/2",
        "/fields/users/1/posts/grace-post",
    ] {
        let (status, body) = get_path(addr, path).await;
        assert_eq!(status, 404, "{path}: {body}");
        assert!(!body.contains("grace-post"), "{path}: {body}");
    }
    for path in [
        "/users/1/posts/1",
        "/bound/users/1/posts/1",
        "/fields/users/1/posts/ada-post",
    ] {
        assert_eq!(
            get_path(addr, path).await,
            (200, "ada ada-post".to_owned()),
            "{path}"
        );
    }
    // The control: nothing scopes the plain route.
    assert_eq!(
        get_path(addr, "/plain/users/1/posts/2").await,
        (200, "ada grace-post".to_owned())
    );
}

#[tokio::test]
async fn bind_008_a_handler_a_route_handler_calls_binds_through_binders_trashed_rows_and_missing() {
    let _db = fixture().await;
    let router: Router = Router::new()
        .get("/trashed/users/{user}", forward_show_user)
        .with_trashed()
        .get("/users/{user}", forward_show_user)
        .get("/posts/{post}", forward_show_post)
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

    let router: Router = Router::new().get("/users/{user}", forward_show_user).into();
    let addr = serve(router.bind("user", user_by_name)).await;
    assert_eq!(
        get_path(addr, "/users/grace").await,
        (200, "grace".to_owned())
    );
}

// ── A handler middleware calls, a child bound alone, instantiations ───────

#[tokio::test]
async fn bind_006_a_handler_route_middleware_calls_binds_under_the_routes_settings() {
    // The middleware answers before the route binds its own handler's
    // arguments, so the handler it calls is not the one the route planned.
    let _db = fixture().await;
    let router: Router = Router::new()
        .get("/users/{user}/posts/{post}", show_post)
        .scope_bindings()
        .middleware(AnswerWith(answer_user_post))
        .get("/same/users/{user}/posts/{post}", user_post)
        .scope_bindings()
        .middleware(AnswerWith(answer_user_post))
        .get("/idle/users/{user}/posts/{post}", binds_nothing)
        .scope_bindings()
        .middleware(AnswerWith(answer_user_post))
        .into();
    let addr = serve(router).await;
    for prefix in ["", "/same", "/idle"] {
        let path = format!("{prefix}/users/1/posts/2");
        let (status, body) = get_path(addr, &path).await;
        assert_eq!(status, 404, "{path}: {body}");
        assert!(!body.contains("grace-post"), "{path}: {body}");
        assert_eq!(
            get_path(addr, &format!("{prefix}/users/1/posts/1")).await,
            (200, "ada ada-post".to_owned()),
            "{prefix}"
        );
    }
}

#[tokio::test]
async fn bind_006_a_handler_that_binds_a_scoped_child_alone_never_gets_a_row_the_route_refused() {
    // Each handler binds the post without the user. The route binds the
    // user, so the post is looked up through it all the same.
    let _db = fixture().await;
    let router: Router = Router::new()
        .get("/users/{user}/posts/{post}", user_post)
        .scope_bindings()
        .missing(missing_post)
        .get("/fields/users/{user}/posts/{post:slug}", user_post)
        .missing(missing_post)
        .get("/nested/users/{user}/posts/{post}", bound_forward_show_post)
        .scope_bindings()
        .get("/mw/users/{user}/posts/{post}", user_post)
        .scope_bindings()
        .middleware(AnswerWith(answer_show_post))
        .into();
    let addr = serve(router).await;
    for path in [
        "/users/1/posts/2",
        "/fields/users/1/posts/grace-post",
        "/nested/users/1/posts/2",
        "/mw/users/1/posts/2",
    ] {
        let (status, body) = get_path(addr, path).await;
        assert_eq!(status, 404, "{path}: {body}");
        assert!(!body.contains("grace-post"), "{path}: {body}");
    }
    assert_eq!(
        get_path(addr, "/nested/users/1/posts/1").await,
        (200, "ada-post".to_owned())
    );
    assert_eq!(
        get_path(addr, "/mw/users/1/posts/1").await,
        (200, "ada-post".to_owned())
    );
}

#[tokio::test]
async fn bind_004_each_instantiation_of_a_generic_handler_plans_its_own_arguments() {
    // One closure route calls two instantiations whose bound argument types
    // differ; each binds its own type.
    let _db = fixture().await;
    let router: Router = Router::new()
        .get("/items/{item}", |req: Request| async move {
            if req.header("x-kind") == Some("post") {
                via_closure(show_item::<OuPost>(req).await)
            } else {
                via_closure(show_item::<OuUser>(req).await)
            }
        })
        .into();
    let addr = serve(router).await;
    assert_eq!(
        send(addr, "GET", "/items/2", &[("x-kind", "user")], None).await,
        (200, "OuUser 2".to_owned())
    );
    assert_eq!(
        send(addr, "GET", "/items/2", &[("x-kind", "post")], None).await,
        (200, "OuPost 2".to_owned())
    );
}

#[tokio::test]
async fn bind_007_a_missing_handlers_request_time_refusal_names_it_as_such() {
    let _env = super::testing_environment_async().await;
    // Only the generic `missing()` handler binds `user`, and the binder for
    // it returns a `String`: the route starts, and the miss answers the
    // refusal, naming the `missing()` handler.
    let _db = fixture().await;
    let router: Router = Router::new()
        .get("/users/{user}/posts/{post}", show_post)
        .missing(missing_generic::<Request>)
        .into();
    let router = router.bind(
        "user",
        |value: String, _route| async move { Ok(Some(value)) },
    );
    router
        .prepare_bindings()
        .expect("a generic missing() handler is not checked at startup");
    let addr = serve(router).await;
    let (status, body) = get_path(addr, "/users/1/posts/99").await;
    assert_eq!(status, 500, "{body}");
    assert!(body.contains("`missing()` handler"), "{body}");
}

#[tokio::test]
async fn bind_009_a_handler_of_another_return_type_middleware_calls_answers_the_missing_response() {
    // Route middleware calls the handler before the route's own runs; its
    // return type cannot carry the `missing()` response, which the route
    // answers with all the same.
    let _db = fixture().await;
    let router: Router = Router::new()
        .get("/users/{user}/posts/{post}", show_post)
        .scope_bindings()
        .middleware(AnswerWith(answer_user_post_result))
        .missing(redirect_home)
        .into();
    let addr = serve(router).await;
    assert_eq!(
        get_path(addr, "/users/1/posts/2").await,
        (302, "missing at /users/1/posts/2".to_owned())
    );
    assert_eq!(
        get_path(addr, "/users/1/posts/1").await,
        (200, "ada ada-post".to_owned())
    );
}

/// Global middleware that marks every response on its way out, as a
/// session cookie or a security header would be added.
pub struct MarkResponse;

#[suprnova::async_trait]
impl Middleware for MarkResponse {
    async fn handle(&self, request: Request, next: Next) -> Response {
        match next(request).await {
            Ok(response) => Ok(response.header("X-Marked", "yes")),
            Err(response) => Err(response.header("X-Marked", "yes")),
        }
    }
}

#[tokio::test]
async fn bind_009_a_kept_missing_response_passes_through_the_routes_middleware() {
    // A handler the route's handler calls keeps the `missing()` response;
    // the route answers with it inside the chain, so the response-side
    // middleware marks it as it marks the route's own `missing()` response.
    let _db = fixture().await;
    let router: Router = Router::new()
        .get("/users/{user}/posts/{post}", |req| async move {
            user_post_result(req).await.map_err(HttpResponse::from)
        })
        .scope_bindings()
        .missing(redirect_home)
        .get("/mw/users/{user}/posts/{post}", show_post)
        .scope_bindings()
        .middleware(AnswerWith(answer_user_post_result))
        .missing(redirect_home)
        .into();
    let addr = serve_with(router, MiddlewareRegistry::new().append(MarkResponse)).await;
    let (status, headers, body) =
        crate::http_wire::request(addr, "GET", "/users/1/posts/2", &[]).await;
    assert_eq!(
        (status, body.as_str()),
        (302, "missing at /users/1/posts/2")
    );
    assert_eq!(
        headers.get("x-marked").map(String::as_str),
        Some("yes"),
        "the missing() response passes through the middleware: {headers:?}"
    );
    // A handler route middleware calls still gets the `missing()` response,
    // and the global middleware outside it marks it too.
    let (status, headers, body) =
        crate::http_wire::request(addr, "GET", "/mw/users/1/posts/2", &[]).await;
    assert_eq!(
        (status, body.as_str()),
        (302, "missing at /mw/users/1/posts/2")
    );
    assert_eq!(
        headers.get("x-marked").map(String::as_str),
        Some("yes"),
        "the missing() response passes through the global middleware: {headers:?}"
    );
}

/// Route middleware that marks every response on its way out.
pub struct MarkRoute;

#[suprnova::async_trait]
impl Middleware for MarkRoute {
    async fn handle(&self, request: Request, next: Next) -> Response {
        match next(request).await {
            Ok(response) => Ok(response.header("X-Route", "yes")),
            Err(response) => Err(response.header("X-Route", "yes")),
        }
    }
}

#[tokio::test]
async fn bind_009_a_missing_response_kept_inside_middleware_passes_through_every_middleware_outside_it()
 {
    // The inner route middleware calls a handler that cannot return the
    // `missing()` response; the outer route middleware and the global one
    // each mark the `missing()` response, as they mark the route's own.
    let _db = fixture().await;
    let router: Router = Router::new()
        .get("/users/{user}/posts/{post}", show_post)
        .scope_bindings()
        .middleware(MarkRoute)
        .middleware(AnswerWith(answer_user_post_result))
        .missing(redirect_home)
        .into();
    let addr = serve_with(router, MiddlewareRegistry::new().append(MarkResponse)).await;
    let (status, headers, body) =
        crate::http_wire::request(addr, "GET", "/users/1/posts/2", &[]).await;
    assert_eq!(
        (status, body.as_str()),
        (302, "missing at /users/1/posts/2")
    );
    for header in ["x-marked", "x-route"] {
        assert_eq!(
            headers.get(header).map(String::as_str),
            Some("yes"),
            "`{header}` on the missing() response: {headers:?}"
        );
    }
}

#[tokio::test]
async fn bind_009_a_caller_that_answers_the_miss_with_its_own_404_answers_the_missing_response() {
    // Middleware and a closure route each map the handler's error to a 404
    // of their own, with no error report: still the miss, so the route
    // answers its `missing()` response.
    let _db = fixture().await;
    let router: Router = Router::new()
        .get("/mw/users/{user}/posts/{post}", show_post)
        .scope_bindings()
        .middleware(AnswerWith(answer_user_post_result_own_404))
        .missing(redirect_home)
        .get("/closure/users/{user}/posts/{post}", |req| async move {
            user_post_result(req)
                .await
                .map_err(|_| HttpResponse::text("gone").status(404))
        })
        .scope_bindings()
        .missing(redirect_home)
        .into();
    let addr = serve(router).await;
    for path in ["/mw/users/1/posts/2", "/closure/users/1/posts/2"] {
        assert_eq!(
            get_path(addr, path).await,
            (302, format!("missing at {path}")),
            "{path}"
        );
    }
}

#[tokio::test]
async fn bind_009_a_caller_that_recovers_from_the_miss_keeps_its_answer() {
    let _db = fixture().await;
    let router: Router = Router::new()
        .get("/users/{user}/posts/{post}", show_post)
        .scope_bindings()
        .middleware(AnswerWith(answer_user_post_result_recovered))
        .missing(redirect_home)
        .into();
    let addr = serve(router).await;
    assert_eq!(
        get_path(addr, "/users/1/posts/2").await,
        (200, "recovered".to_owned())
    );
}
