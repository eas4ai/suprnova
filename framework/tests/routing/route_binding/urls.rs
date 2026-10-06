//! BIND-012: `route()` and `try_route()` take, for each parameter, a string
//! or a bound value, named or, for a single value, positional. A bound
//! value fills its parameter with its binding field when the route names
//! one, else its route key, percent-encoded as any value is. Calls written
//! with strings produce the URLs they did.

use suprnova::http::text;
use suprnova::testing::TestDatabase;
use suprnova::{Model, Response, RouteParam, Router, attrs, handler, model, route, try_route};

use super::run_sql;

#[model(table = "ur_posts", fillable = ["slug"])]
pub struct UrPost {
    pub id: i64,
    pub slug: String,
}

#[model(table = "ur_pages", route_key = "slug", fillable = ["slug"])]
pub struct UrPage {
    pub id: i64,
    pub slug: String,
}

#[model(table = "ur_users", fillable = ["name"])]
pub struct UrUser {
    pub id: i64,
    pub name: String,
}

#[handler]
pub async fn noop() -> Response {
    text("")
}

async fn fixture() -> TestDatabase {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    run_sql(
        &db,
        &[
            "CREATE TABLE ur_posts (id INTEGER PRIMARY KEY AUTOINCREMENT, slug TEXT NOT NULL)",
            "CREATE TABLE ur_pages (id INTEGER PRIMARY KEY AUTOINCREMENT, slug TEXT NOT NULL)",
            "CREATE TABLE ur_users (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL)",
        ],
    )
    .await;
    db
}

fn register_routes() -> Router {
    Router::new()
        .get("/ur/posts/{post}", noop)
        .name("ur.posts.show")
        .get("/ur/slugs/{post:slug}", noop)
        .name("ur.posts.slug")
        .get("/ur/pages/{page}", noop)
        .name("ur.pages.show")
        .get("/ur/users/{user}/posts/{post:slug}", noop)
        .name("ur.users.posts.show")
        .get("/ur/plain/{id}", noop)
        .name("ur.plain")
        .get("/ur/home", noop)
        .name("ur.home")
}

#[tokio::test]
async fn bind_012_a_bound_value_fills_its_route_key_or_its_binding_field() {
    let _db = fixture().await;
    let _router = register_routes();
    let post = UrPost::create(attrs! { slug: "a b/c" }).await.unwrap();
    let id = post.id;

    assert_eq!(
        route("ur.posts.show", &post),
        Some(format!("/ur/posts/{id}"))
    );
    assert_eq!(
        route("ur.slugs.show-unknown", &post),
        None,
        "an unknown name is still None"
    );
    // The field's value, percent-encoded as a string value is.
    assert_eq!(
        route("ur.posts.slug", &post).as_deref(),
        Some("/ur/slugs/a%20b%2Fc")
    );
    // Positional by value, and through `RouteParam`.
    assert_eq!(
        route("ur.posts.slug", RouteParam(post.clone())).as_deref(),
        Some("/ur/slugs/a%20b%2Fc")
    );

    // A route key set with `route_key` fills a parameter without a field.
    let page = UrPage::create(attrs! { slug: "about" }).await.unwrap();
    assert_eq!(
        route("ur.pages.show", &page).as_deref(),
        Some("/ur/pages/about")
    );
}

#[tokio::test]
async fn bind_012_named_values_mix_strings_and_bound_values() {
    let _db = fixture().await;
    let _router = register_routes();
    let user = UrUser::create(attrs! { name: "ada" }).await.unwrap();
    let post = UrPost::create(attrs! { slug: "hello" }).await.unwrap();
    let url = route("ur.users.posts.show", (("user", &user), ("post", &post)));
    assert_eq!(url, Some(format!("/ur/users/{}/posts/hello", user.id)));
    let url = route("ur.users.posts.show", (("user", "7"), ("post", &post)));
    assert_eq!(url.as_deref(), Some("/ur/users/7/posts/hello"));
    assert_eq!(
        route("ur.posts.slug", ("post", &post)).as_deref(),
        Some("/ur/slugs/hello")
    );
}

#[tokio::test]
async fn bind_012_try_route_takes_bound_values_and_reports_missing_ones() {
    let _db = fixture().await;
    let _router = register_routes();
    let post = UrPost::create(attrs! { slug: "hello" }).await.unwrap();
    assert_eq!(
        try_route("ur.posts.slug", &post).unwrap(),
        "/ur/slugs/hello"
    );
    assert_eq!(
        try_route("ur.users.posts.show", ("post", &post))
            .unwrap_err()
            .to_string(),
        "Route 'ur.users.posts.show' is missing required path parameter(s): user"
    );
}

#[test]
fn bind_012_calls_written_with_strings_produce_the_same_urls() {
    let _router = register_routes();
    assert_eq!(
        route("ur.plain", &[("id", "42")]).as_deref(),
        Some("/ur/plain/42")
    );
    assert_eq!(route("ur.home", &[]).as_deref(), Some("/ur/home"));
    let slice: &[(&str, &str)] = &[("id", "../x")];
    assert_eq!(
        route("ur.plain", slice).as_deref(),
        Some("/ur/plain/..%2Fx")
    );
    assert_eq!(
        try_route("ur.plain", &[("id", "1")]).unwrap(),
        "/ur/plain/1"
    );
    // A field in the pattern does not change what a string fills.
    assert_eq!(
        route("ur.posts.slug", &[("post", "plain")]).as_deref(),
        Some("/ur/slugs/plain")
    );
}
