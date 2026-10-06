//! BIND-011: resources in two forms. A `ResourceController` keeps its
//! actions that take the request and routes as it did; the function form,
//! `resource!`, names a module whose `#[handler]` functions are the
//! actions, which take bound arguments. Both nest (`users.posts`), rename
//! parameters, and take `scoped`, `with_trashed` and `missing`.

use std::future::Future;
use std::pin::Pin;

use suprnova::http::text;
use suprnova::routing::{ResourceAction, ResourceController};
use suprnova::testing::TestDatabase;
use suprnova::{Request, Response, Router, api_resource, handler, model, resource, route};

use super::{get, run_sql, send, serve};

#[model(table = "rs_users", relations = {
    posts: HasMany<RsPost>,
})]
pub struct RsUser {
    pub id: i64,
    pub name: String,
}

#[model(table = "rs_posts")]
pub struct RsPost {
    pub id: i64,
    pub rs_user_id: i64,
    pub slug: String,
}

/// The seven actions of `posts`, as `#[handler]` functions.
pub mod posts {
    use super::*;

    #[handler]
    pub async fn index() -> Response {
        text("index")
    }
    #[handler]
    pub async fn create() -> Response {
        text("create")
    }
    #[handler]
    pub async fn store() -> Response {
        text("store")
    }
    #[handler]
    pub async fn show(post: RsPost) -> Response {
        text(format!("show {}", post.slug))
    }
    #[handler]
    pub async fn edit(post: RsPost) -> Response {
        text(format!("edit {}", post.slug))
    }
    #[handler]
    pub async fn update(post: RsPost) -> Response {
        text(format!("update {}", post.slug))
    }
    #[handler]
    pub async fn destroy(post: RsPost) -> Response {
        text(format!("destroy {}", post.slug))
    }
}

/// A nested resource's actions: the user and the post both bind.
pub mod user_posts {
    use super::*;

    #[handler]
    pub async fn index(user: RsUser) -> Response {
        text(format!("{} posts", user.name))
    }
    #[handler]
    pub async fn show(user: RsUser, post: RsPost) -> Response {
        text(format!("{} {}", user.name, post.slug))
    }
}

/// The same actions, reading the renamed parameters.
pub mod author_articles {
    use super::*;

    #[handler]
    pub async fn show(author: RsUser, article: RsPost) -> Response {
        text(format!("{} {}", author.name, article.slug))
    }
}

/// A resource controller whose actions take the request.
struct PostsCtl;

impl ResourceController for PostsCtl {
    fn show(&self, req: Request) -> Pin<Box<dyn Future<Output = Response> + Send>> {
        let post = req.param("post").map(str::to_owned).unwrap_or_default();
        Box::pin(async move { text(format!("controller {post}")) })
    }
    fn index(&self, _req: Request) -> Pin<Box<dyn Future<Output = Response> + Send>> {
        Box::pin(async { text("controller index") })
    }
}

async fn fixture() -> TestDatabase {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    run_sql(
        &db,
        &[
            "CREATE TABLE rs_users (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL)",
            "CREATE TABLE rs_posts (id INTEGER PRIMARY KEY AUTOINCREMENT, \
                rs_user_id INTEGER NOT NULL, slug TEXT NOT NULL)",
            "INSERT INTO rs_users (id, name) VALUES (1, 'ada'), (2, 'grace')",
            "INSERT INTO rs_posts (id, rs_user_id, slug) VALUES (1, 1, 'ada-post'), \
                (2, 2, 'grace-post')",
        ],
    )
    .await;
    db
}

#[tokio::test]
async fn bind_011_a_function_resource_action_receives_a_bound_model() {
    let _db = fixture().await;
    let router = resource!("posts", posts).register(Router::new());
    router
        .prepare_bindings()
        .expect("every action reads a declared parameter");
    let addr = serve(router).await;
    assert_eq!(get(addr, "/posts").await, (200, "index".to_owned()));
    assert_eq!(get(addr, "/posts/create").await, (200, "create".to_owned()));
    assert_eq!(
        get(addr, "/posts/1").await,
        (200, "show ada-post".to_owned())
    );
    assert_eq!(
        get(addr, "/posts/1/edit").await,
        (200, "edit ada-post".to_owned())
    );
    assert_eq!(
        send(addr, "PATCH", "/posts/2", &[], None).await,
        (200, "update grace-post".to_owned())
    );
    assert_eq!(get(addr, "/posts/9").await.0, 404);
    assert_eq!(
        route("posts.show", &[("post", "1")]).as_deref(),
        Some("/posts/1")
    );
}

#[tokio::test]
async fn bind_011_api_resource_and_only_select_actions() {
    let _db = fixture().await;
    let router = api_resource!("posts", posts, except = [destroy])
        .unnamed()
        .register(Router::new());
    assert!(
        router
            .match_route(&hyper::Method::DELETE, "/posts/1")
            .is_none(),
        "destroy is excepted"
    );
    let addr = serve(router).await;
    assert_eq!(
        get(addr, "/posts/1/edit").await.0,
        404,
        "an API resource has no edit"
    );
    assert_eq!(
        get(addr, "/posts/1").await,
        (200, "show ada-post".to_owned())
    );
}

#[tokio::test]
async fn bind_011_nested_resources_register_under_dotted_names() {
    let _db = fixture().await;
    let router = resource!("users.posts", user_posts, only = [index, show]).register(Router::new());
    let addr = serve(router).await;
    assert_eq!(
        get(addr, "/users/1/posts").await,
        (200, "ada posts".to_owned())
    );
    assert_eq!(
        get(addr, "/users/1/posts/1").await,
        (200, "ada ada-post".to_owned())
    );
    assert_eq!(
        route("users.posts.show", (("user", "1"), ("post", "1"))).as_deref(),
        Some("/users/1/posts/1")
    );
    assert_eq!(
        route("users.posts.index", &[("user", "2")]).as_deref(),
        Some("/users/2/posts")
    );
}

#[tokio::test]
async fn bind_011_parameters_renames_the_parameters() {
    let _db = fixture().await;
    let router = resource!("users.posts", author_articles, only = [show])
        .parameters([("users", "author"), ("posts", "article")])
        .unnamed()
        .register(Router::new());
    let addr = serve(router).await;
    assert_eq!(
        get(addr, "/users/2/posts/2").await,
        (200, "grace grace-post".to_owned())
    );
}

#[tokio::test]
async fn bind_011_scoped_gives_a_field_and_scopes_the_child() {
    let _db = fixture().await;
    let router = resource!("users.posts", user_posts, only = [show])
        .scoped([("post", "slug")])
        .unnamed()
        .register(Router::new());
    let addr = serve(router).await;
    assert_eq!(
        get(addr, "/users/1/posts/ada-post").await,
        (200, "ada ada-post".to_owned())
    );
    assert_eq!(get(addr, "/users/1/posts/grace-post").await.0, 404);

    // Without fields, `scoped` still scopes every nested child.
    let router = resource!("users.posts", user_posts, only = [show])
        .scoped([])
        .unnamed()
        .register(Router::new());
    let addr = serve(router).await;
    assert_eq!(get(addr, "/users/1/posts/2").await.0, 404);
    assert_eq!(get(addr, "/users/2/posts/2").await.0, 200);
}

#[tokio::test]
async fn bind_011_a_resource_controller_routes_as_it_did() {
    let router: Router = Router::new()
        .resource("rs-ctl/posts", PostsCtl)
        .only(&[ResourceAction::Index, ResourceAction::Show])
        .into();
    let (pattern, _, _) = router
        .match_route(&hyper::Method::GET, "/rs-ctl/posts/7")
        .expect("show matches");
    assert_eq!(pattern, "/rs-ctl/posts/{post}");
    router
        .prepare_bindings()
        .expect("controller actions carry no record");
    let addr = serve(router).await;
    assert_eq!(
        get(addr, "/rs-ctl/posts/7").await,
        (200, "controller 7".to_owned())
    );
    assert_eq!(
        get(addr, "/rs-ctl/posts").await,
        (200, "controller index".to_owned())
    );

    // A nested controller resource registers the nested paths too.
    let nested: Router = Router::new()
        .resource("rsusers.posts", PostsCtl)
        .only(&[ResourceAction::Show])
        .unnamed()
        .into();
    assert!(
        nested
            .match_route(&hyper::Method::GET, "/rsusers/3/posts/7")
            .is_some()
    );
}
