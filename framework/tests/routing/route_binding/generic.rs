//! A generic `#[handler]` function carries no record, so the startup checks
//! skip it (BIND-004), and its generic arguments read the body (BIND-015).
//! Its concrete bound arguments still bind as a recorded handler's do: in
//! path order (BIND-015), by the route's binding field (BIND-004), scoped
//! through the parent (BIND-006), through the router's binders (BIND-007),
//! through the soft-deletable lookup on a route with `with_trashed()`
//! (BIND-008), and answering the route's `missing()` handler (BIND-009).

use std::fmt::Display;
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use suprnova::http::{HttpResponse, text};
use suprnova::testing::TestDatabase;
use suprnova::{
    FrameworkError, FromRequest, Model, Request, Response, RouteBinding, Router, get, group,
    handler, model,
};

use super::{get as get_path, run_sql, serve};

#[model(table = "gn_users", relations = {
    posts: HasMany<GnPost>,
})]
pub struct GnUser {
    pub id: i64,
    pub name: String,
}

#[model(table = "gn_posts", soft_deletes)]
pub struct GnPost {
    pub id: i64,
    pub gn_user_id: i64,
    pub slug: String,
    pub deleted_at: Option<DateTime<Utc>>,
}

/// A model with no relations: no child can be scoped under it.
#[model(table = "gn_tags")]
pub struct GnTag {
    pub id: i64,
    pub name: String,
}

/// The generic arguments' type. It binds too, so a generic argument that
/// says `body` shows the body was read, not the route.
pub struct GnBody(&'static str);

#[suprnova::async_trait]
impl RouteBinding for GnBody {
    fn route_key_name() -> &'static str {
        "key"
    }
    fn route_key(&self) -> String {
        self.0.to_owned()
    }
    async fn resolve_route_binding(
        _value: &str,
        _field: Option<&str>,
    ) -> Result<Option<Self>, FrameworkError> {
        Ok(Some(GnBody("route")))
    }
}

#[suprnova::async_trait]
impl FromRequest for GnBody {
    async fn from_request(_req: Request) -> Result<Self, FrameworkError> {
        Ok(GnBody("body"))
    }
}

impl Display for GnBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

/// The order the two types below were looked up in.
static LOOKUPS: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

fn note_lookup(name: &'static str) {
    LOOKUPS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(name);
}

/// A type that notes when it is looked up, bound by the first parameter.
pub struct GnFirst(String);

#[suprnova::async_trait]
impl RouteBinding for GnFirst {
    fn route_key_name() -> &'static str {
        "key"
    }
    fn route_key(&self) -> String {
        self.0.clone()
    }
    async fn resolve_route_binding(
        value: &str,
        _field: Option<&str>,
    ) -> Result<Option<Self>, FrameworkError> {
        note_lookup("first");
        Ok(Some(GnFirst(value.to_owned())))
    }
}

/// A type that notes when it is looked up, bound by the second parameter.
pub struct GnSecond(String);

#[suprnova::async_trait]
impl RouteBinding for GnSecond {
    fn route_key_name() -> &'static str {
        "key"
    }
    fn route_key(&self) -> String {
        self.0.clone()
    }
    async fn resolve_route_binding(
        value: &str,
        _field: Option<&str>,
    ) -> Result<Option<Self>, FrameworkError> {
        note_lookup("second");
        Ok(Some(GnSecond(value.to_owned())))
    }
}

#[handler]
pub async fn user_post<B: FromRequest + Display + Send + 'static>(
    user: GnUser,
    post: GnPost,
    extra: B,
) -> Response {
    text(format!("{} {} {extra}", user.name, post.slug))
}

/// [`user_post`] with its arguments declared against the path's order.
#[handler]
pub async fn post_user<B: FromRequest + Display + Send + 'static>(
    post: GnPost,
    extra: B,
    user: GnUser,
) -> Response {
    text(format!("{} {} {extra}", user.name, post.slug))
}

#[handler]
pub async fn show_post<B: FromRequest + Display + Send + 'static>(
    post: GnPost,
    extra: B,
) -> Response {
    text(format!("{} {extra}", post.slug))
}

#[handler]
pub async fn show_user<B: FromRequest + Display + Send + 'static>(
    user: GnUser,
    extra: B,
) -> Response {
    text(format!("{} {extra}", user.name))
}

#[handler]
pub async fn maybe_post<B: FromRequest + Display + Send + 'static>(
    post: Option<GnPost>,
    extra: B,
) -> Response {
    let post = post.map_or_else(|| "none".to_owned(), |post| post.slug);
    text(format!("{post} {extra}"))
}

#[handler]
pub async fn tag_post<B: FromRequest + Display + Send + 'static>(
    tag: GnTag,
    post: GnPost,
    extra: B,
) -> Response {
    text(format!("{} {} {extra}", tag.name, post.slug))
}

#[handler]
pub async fn ordered<B: FromRequest + Display + Send + 'static>(
    second: GnSecond,
    first: GnFirst,
    extra: B,
) -> Response {
    text(format!("{} {} {extra}", first.0, second.0))
}

/// The `missing()` handler: it sees the request.
async fn redirect_home(request: Request) -> Response {
    Ok(HttpResponse::text(format!("missing at {}", request.path())).status(302))
}

async fn fixture() -> TestDatabase {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    run_sql(
        &db,
        &[
            "CREATE TABLE gn_users (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL)",
            "CREATE TABLE gn_posts (id INTEGER PRIMARY KEY AUTOINCREMENT, \
                gn_user_id INTEGER NOT NULL, slug TEXT NOT NULL, deleted_at TEXT)",
            "CREATE TABLE gn_tags (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL)",
            "INSERT INTO gn_users (id, name) VALUES (1, 'ada'), (2, 'grace')",
            "INSERT INTO gn_posts (id, gn_user_id, slug, deleted_at) VALUES \
                (1, 1, 'ada-post', NULL), (2, 2, 'grace-post', NULL), \
                (3, 1, 'ada-gone', '2026-01-01T00:00:00+00:00')",
            "INSERT INTO gn_tags (id, name) VALUES (1, 'rust')",
        ],
    )
    .await;
    db
}

#[tokio::test]
async fn bind_006_a_generic_handler_scopes_a_child_through_its_parent() {
    let _db = fixture().await;
    let routes: Router = Router::new()
        .get("/plain/users/{user}/posts/{post}", user_post::<GnBody>)
        .get("/scoped/users/{user}/posts/{post}", user_post::<GnBody>)
        .scope_bindings()
        .get(
            "/fields/users/{user}/posts/{post:slug}",
            user_post::<GnBody>,
        )
        .get("/open/users/{user}/posts/{post:slug}", user_post::<GnBody>)
        .without_scoped_bindings()
        .into();
    let router = group!("/g", {
        get!("/users/{user}/posts/{post}", user_post::<GnBody>)
    })
    .scope_bindings()
    .register(routes);
    let addr = serve(router).await;

    // `scope_bindings()` on the route and on a group: Grace's post under
    // Ada answers 404, Ada's own binds.
    for prefix in ["/scoped", "/g"] {
        assert_eq!(
            get_path(addr, &format!("{prefix}/users/1/posts/2")).await.0,
            404,
            "{prefix}: Grace's post is not Ada's"
        );
        assert_eq!(
            get_path(addr, &format!("{prefix}/users/1/posts/1")).await,
            (200, "ada ada-post body".to_owned()),
            "{prefix}"
        );
    }
    // A binding field scopes the child by itself.
    assert_eq!(
        get_path(addr, "/fields/users/1/posts/grace-post").await.0,
        404
    );
    assert_eq!(
        get_path(addr, "/fields/users/1/posts/ada-post").await,
        (200, "ada ada-post body".to_owned())
    );
    // The controls: no field and no scoping, or `without_scoped_bindings()`.
    assert_eq!(
        get_path(addr, "/plain/users/1/posts/2").await,
        (200, "ada grace-post body".to_owned())
    );
    assert_eq!(
        get_path(addr, "/open/users/1/posts/grace-post").await,
        (200, "ada grace-post body".to_owned())
    );
}

#[tokio::test]
async fn bind_004_a_generic_handler_binds_by_the_routes_binding_field() {
    let _db = fixture().await;
    let addr = serve(
        Router::new()
            .get("/posts/{post:slug}", show_post::<GnBody>)
            .into(),
    )
    .await;
    assert_eq!(
        get_path(addr, "/posts/ada-post").await,
        (200, "ada-post body".to_owned())
    );
    assert_eq!(
        get_path(addr, "/posts/1").await.0,
        404,
        "the field is the slug, not the key"
    );
}

#[tokio::test]
async fn bind_015_a_generic_handler_binds_in_path_order() {
    let _db = fixture().await;
    let router: Router = Router::new()
        .get("/order/{first}/{second}", ordered::<GnBody>)
        .get("/users/{user}/posts/{post}", post_user::<GnBody>)
        .scope_bindings()
        .into();
    let addr = serve(router).await;

    // `second` is declared first; `first` comes first in the path.
    LOOKUPS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clear();
    assert_eq!(
        get_path(addr, "/order/a/b").await,
        (200, "a b body".to_owned())
    );
    assert_eq!(
        *LOOKUPS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        ["first", "second"]
    );

    // Declared child first, the post is still scoped to the user before it
    // in the path.
    assert_eq!(get_path(addr, "/users/1/posts/2").await.0, 404);
    assert_eq!(
        get_path(addr, "/users/1/posts/1").await,
        (200, "ada ada-post body".to_owned())
    );
}

#[tokio::test]
async fn bind_007_a_generic_handler_binds_through_the_routers_binders() {
    let _db = fixture().await;
    let router: Router = Router::new()
        .get("/posts/{post}", show_post::<GnBody>)
        .get("/users/{user}", show_user::<GnBody>)
        .into();
    // `bind` registered after the route, finding a post by its slug.
    let router = router
        .bind("post", |value: String, route| async move {
            assert_eq!(route.pattern(), "/posts/{post}");
            GnPost::query().filter("slug", value).first().await
        })
        .model::<GnUser, _, _>("user", |value| async move {
            Ok(GnUser {
                name: format!("guest {value}"),
                ..GnUser::default()
            })
        });
    let addr = serve(router).await;
    assert_eq!(
        get_path(addr, "/posts/grace-post").await,
        (200, "grace-post body".to_owned())
    );
    assert_eq!(get_path(addr, "/posts/nothing").await.0, 404);
    // `model` binds by the route key and falls back for a missing row.
    assert_eq!(
        get_path(addr, "/users/2").await,
        (200, "grace body".to_owned())
    );
    assert_eq!(
        get_path(addr, "/users/99").await,
        (200, "guest 99 body".to_owned())
    );
}

#[tokio::test]
async fn bind_008_a_generic_handler_binds_a_trashed_row_only_with_trashed() {
    let _db = fixture().await;
    let router: Router = Router::new()
        .get("/posts/{post}", show_post::<GnBody>)
        .get("/trashed/posts/{post}", show_post::<GnBody>)
        .with_trashed()
        .get("/users/{user}/posts/{post:slug}", user_post::<GnBody>)
        .get(
            "/trashed/users/{user}/posts/{post:slug}",
            user_post::<GnBody>,
        )
        .with_trashed()
        .into();
    let addr = serve(router).await;
    assert_eq!(get_path(addr, "/posts/3").await.0, 404);
    assert_eq!(
        get_path(addr, "/trashed/posts/3").await,
        (200, "ada-gone body".to_owned())
    );
    // A scoped child, through the parent's soft-deletable lookup.
    assert_eq!(get_path(addr, "/users/1/posts/ada-gone").await.0, 404);
    assert_eq!(
        get_path(addr, "/trashed/users/1/posts/ada-gone").await,
        (200, "ada ada-gone body".to_owned())
    );
    assert_eq!(
        get_path(addr, "/trashed/users/2/posts/ada-gone").await.0,
        404,
        "a trashed row the parent does not own"
    );
}

#[tokio::test]
async fn bind_009_a_generic_handlers_route_answers_its_missing_handler() {
    let _db = fixture().await;
    let router: Router = Router::new()
        .get("/posts/{post}", show_post::<GnBody>)
        .missing(redirect_home)
        .get("/users/{user}/posts/{post:slug}", user_post::<GnBody>)
        .missing(redirect_home)
        .into();
    let addr = serve(router).await;
    assert_eq!(
        get_path(addr, "/posts/1").await,
        (200, "ada-post body".to_owned())
    );
    for path in [
        // No row.
        "/posts/99",
        // A value that does not parse.
        "/posts/abc",
        "/users/abc/posts/ada-post",
        // A row the parent does not own.
        "/users/1/posts/grace-post",
    ] {
        assert_eq!(
            get_path(addr, path).await,
            (302, format!("missing at {path}")),
            "{path}"
        );
    }
}

#[tokio::test]
async fn bind_015_a_generic_handlers_optional_argument_binds_none_when_absent() {
    let _db = fixture().await;
    let addr = serve(
        Router::new()
            .get("/maybe/{post?}", maybe_post::<GnBody>)
            .into(),
    )
    .await;
    assert_eq!(
        get_path(addr, "/maybe").await,
        (200, "none body".to_owned())
    );
    assert_eq!(
        get_path(addr, "/maybe/1").await,
        (200, "ada-post body".to_owned())
    );
    assert_eq!(get_path(addr, "/maybe/99").await.0, 404);
}

#[tokio::test]
async fn bind_004_a_generic_handler_is_not_refused_at_startup_and_never_binds_past_a_check() {
    // Each route breaks a check a recorded handler is refused for at
    // startup: a scoped child whose parent declares no relation for it, a
    // binder of another type, and a bound parameter the path does not
    // declare. The router starts; each request answers the error instead
    // of binding the arguments unchecked.
    let _db = fixture().await;
    let router: Router = Router::new()
        .get("/tags/{tag}/posts/{post}", tag_post::<GnBody>)
        .scope_bindings()
        .get("/users/{user}", show_user::<GnBody>)
        .get("/undeclared", show_post::<GnBody>)
        .into();
    let router = router.bind(
        "user",
        |value: String, _route| async move { Ok(Some(value)) },
    );
    router
        .prepare_bindings()
        .expect("a generic handler carries no record, so nothing is refused at startup");
    let addr = serve(router).await;
    let (status, body) = get_path(addr, "/tags/1/posts/1").await;
    assert_eq!(status, 500, "a child with no relation to scope it: {body}");
    assert!(!body.contains("ada-post"), "{body}");
    let (status, body) = get_path(addr, "/users/1").await;
    assert_eq!(status, 500, "a binder of another type: {body}");
    assert!(!body.contains("ada"), "{body}");
    assert_eq!(
        get_path(addr, "/undeclared").await.0,
        400,
        "the undeclared `post` is missing at request time"
    );
}
