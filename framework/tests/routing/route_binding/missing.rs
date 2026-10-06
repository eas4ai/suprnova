//! BIND-009: `missing(handler)` on a route, a group or a resource answers
//! instead of the 404 when a binding finds no row, finds no row its parent
//! owns, or receives a value that does not parse.

use suprnova::http::{HttpResponse, text};
use suprnova::testing::TestDatabase;
use suprnova::{Request, Response, Router, get, group, handler, model, resource};

use super::{get as get_path, run_sql, serve};

#[model(table = "mi_users", relations = {
    posts: HasMany<MiPost>,
})]
pub struct MiUser {
    pub id: i64,
    pub name: String,
}

#[model(table = "mi_posts")]
pub struct MiPost {
    pub id: i64,
    pub mi_user_id: i64,
    pub slug: String,
}

#[handler]
pub async fn show(post: MiPost) -> Response {
    text(post.slug)
}

#[handler]
pub async fn user_post(user: MiUser, post: MiPost) -> Response {
    text(format!("{} {}", user.name, post.slug))
}

pub mod posts {
    use super::*;

    #[handler]
    pub async fn show(post: MiPost) -> Response {
        text(post.slug)
    }
}

/// A nested resource's actions: the user and the post both bind.
pub mod user_posts {
    use super::*;

    #[handler]
    pub async fn show(user: MiUser, post: MiPost) -> Response {
        text(format!("{} {}", user.name, post.slug))
    }
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
            "CREATE TABLE mi_users (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL)",
            "CREATE TABLE mi_posts (id INTEGER PRIMARY KEY AUTOINCREMENT, \
                mi_user_id INTEGER NOT NULL, slug TEXT NOT NULL)",
            "INSERT INTO mi_users (id, name) VALUES (1, 'ada'), (2, 'grace')",
            "INSERT INTO mi_posts (id, mi_user_id, slug) VALUES (1, 1, 'ada-post'), \
                (2, 2, 'grace-post')",
        ],
    )
    .await;
    db
}

#[tokio::test]
async fn bind_009_a_route_missing_handler_replaces_the_404() {
    let _db = fixture().await;
    let router: Router = Router::new()
        .get("/posts/{post}", show)
        .missing(redirect_home)
        .get("/users/{user}/posts/{post:slug}", user_post)
        .missing(redirect_home)
        .into();
    let addr = serve(router).await;
    assert_eq!(
        get_path(addr, "/posts/1").await,
        (200, "ada-post".to_owned())
    );
    // No row.
    assert_eq!(
        get_path(addr, "/posts/99").await,
        (302, "missing at /posts/99".to_owned())
    );
    // A value that does not parse.
    assert_eq!(get_path(addr, "/posts/abc").await.0, 302);
    // A row the parent does not own.
    assert_eq!(
        get_path(addr, "/users/1/posts/grace-post").await,
        (302, "missing at /users/1/posts/grace-post".to_owned())
    );
}

#[tokio::test]
async fn bind_009_a_group_missing_handler_reaches_its_routes() {
    let _db = fixture().await;
    let router = group!("/g", {
        get!("/posts/{post}", show),
        get!("/users/{user}/posts/{post:slug}", user_post),
    })
    .missing(redirect_home)
    .register(Router::new());
    let addr = serve(router).await;
    assert_eq!(
        get_path(addr, "/g/posts/1").await,
        (200, "ada-post".to_owned())
    );
    // No row.
    assert_eq!(
        get_path(addr, "/g/posts/99").await,
        (302, "missing at /g/posts/99".to_owned())
    );
    // A value that does not parse, for the child and for the parent.
    assert_eq!(
        get_path(addr, "/g/posts/abc").await,
        (302, "missing at /g/posts/abc".to_owned())
    );
    assert_eq!(
        get_path(addr, "/g/users/abc/posts/ada-post").await,
        (302, "missing at /g/users/abc/posts/ada-post".to_owned())
    );
    // A row the parent does not own.
    assert_eq!(
        get_path(addr, "/g/users/1/posts/grace-post").await,
        (302, "missing at /g/users/1/posts/grace-post".to_owned())
    );
}

#[tokio::test]
async fn bind_009_a_resource_missing_handler_reaches_its_routes() {
    let _db = fixture().await;
    let router = resource!("posts", posts, only = [show])
        .missing(redirect_home)
        .unnamed()
        .register(Router::new());
    let addr = serve(router).await;
    assert_eq!(
        get_path(addr, "/posts/1").await,
        (200, "ada-post".to_owned())
    );
    assert_eq!(get_path(addr, "/posts/99").await.0, 302);
    assert_eq!(
        get_path(addr, "/posts/abc").await,
        (302, "missing at /posts/abc".to_owned())
    );

    // A nested, scoped resource: a missing, malformed or unowned binding,
    // of the child or of its parent, reaches the resource's handler.
    let router = resource!("users.posts", user_posts, only = [show])
        .scoped([("post", "slug")])
        .missing(redirect_home)
        .unnamed()
        .register(Router::new());
    let addr = serve(router).await;
    assert_eq!(
        get_path(addr, "/users/1/posts/ada-post").await,
        (200, "ada ada-post".to_owned())
    );
    for path in [
        "/users/1/posts/no-such-post",
        "/users/9/posts/ada-post",
        "/users/abc/posts/ada-post",
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
async fn bind_009_a_binder_that_finds_nothing_reaches_the_missing_handler() {
    let _db = fixture().await;
    let router: Router = Router::new()
        .get("/by-slug/{post}", show)
        .missing(redirect_home)
        .get("/plain/by-slug/{post}", show)
        .into();
    let router = router.bind("post", |value: String, _route| async move {
        use suprnova::Model;
        MiPost::query().filter("slug", value).first().await
    });
    let addr = serve(router).await;
    assert_eq!(
        get_path(addr, "/by-slug/ada-post").await,
        (200, "ada-post".to_owned())
    );
    assert_eq!(
        get_path(addr, "/by-slug/no-such-post").await,
        (302, "missing at /by-slug/no-such-post".to_owned())
    );
    assert_eq!(
        get_path(addr, "/plain/by-slug/no-such-post").await.0,
        404,
        "a route without `missing()` keeps the 404"
    );
}

#[tokio::test]
async fn bind_009_every_missing_site_replaces_the_404() {
    let _db = fixture().await;
    let sites: Vec<(&str, &str, Router)> = vec![
        (
            "Router::get(..).missing",
            "/posts/99",
            Router::new()
                .get("/posts/{post}", show)
                .missing(redirect_home)
                .into(),
        ),
        (
            "Router::any(..).missing",
            "/posts/99",
            Router::new()
                .any("/posts/{post}", show)
                .missing(redirect_home)
                .into(),
        ),
        (
            "get!(..).missing",
            "/posts/99",
            get!("/posts/{post}", show)
                .missing(redirect_home)
                .register(Router::new()),
        ),
        (
            "group!(..).missing",
            "/g/posts/99",
            group!("/g", { get!("/posts/{post}", show) })
                .missing(redirect_home)
                .register(Router::new()),
        ),
        (
            "Router::group(..).missing",
            "/g/posts/99",
            Router::new()
                .group("/g", |r| r.get("/posts/{post}", show))
                .missing(redirect_home)
                .into(),
        ),
        (
            "resource!(..).missing",
            "/posts/99",
            resource!("posts", posts, only = [show])
                .missing(redirect_home)
                .unnamed()
                .register(Router::new()),
        ),
    ];
    for (site, path, router) in sites {
        let addr = serve(router).await;
        assert_eq!(
            get_path(addr, path).await,
            (302, format!("missing at {path}")),
            "{site}"
        );
        let malformed = path.replace("99", "abc");
        assert_eq!(
            get_path(addr, &malformed).await,
            (302, format!("missing at {malformed}")),
            "{site}"
        );
    }
}

#[tokio::test]
async fn bind_009_a_route_without_missing_answers_404() {
    let _db = fixture().await;
    let addr = serve(Router::new().get("/posts/{post}", show).into()).await;
    assert_eq!(get_path(addr, "/posts/99").await.0, 404);
}
