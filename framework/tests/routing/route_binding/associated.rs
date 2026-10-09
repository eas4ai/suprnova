//! BIND-003: a `#[handler(Self = Type)]` function inside an `impl` block
//! carries the record a free handler carries. It binds ahead of the body
//! the way a free handler does, and the startup checks (BIND-004, BIND-006,
//! BIND-007, BIND-013, BIND-015) see it, through the boot path and through
//! `handle_request`.

use suprnova::database::EntityExt;
use suprnova::http::text;
use suprnova::testing::TestDatabase;
use suprnova::{Response, RouteParam, Router, Server, handler, model, request};

use super::{get, message, refusal, run_sql, send, serve};

#[model(table = "as_posts", fillable = ["slug", "title"])]
pub struct AsPost {
    pub id: i64,
    pub slug: String,
    pub title: String,
}

impl EntityExt for as_post::Entity {}

/// A parent that declares no relationship.
#[model(table = "as_users", fillable = ["name"])]
pub struct AsUser {
    pub id: i64,
    pub name: String,
}

#[request]
pub struct AsForm {
    #[validate(length(min = 3))]
    pub title: String,
}

pub struct AsPosts;

impl AsPosts {
    #[handler(Self = AsPosts)]
    pub async fn show(post: AsPost) -> Response {
        text(format!("show {}", post.title))
    }

    #[handler(Self = AsPosts)]
    pub async fn wrapped(post: RouteParam<AsPost>) -> Response {
        text(format!("wrapped {}", post.title))
    }

    #[handler(Self = AsPosts)]
    pub async fn bare(post: as_post::Model) -> Response {
        text(format!("bare {}", post.title))
    }

    #[handler(Self = AsPosts)]
    pub async fn by_id(id: i64) -> Response {
        text(format!("id {id}"))
    }

    #[handler(Self = AsPosts)]
    pub async fn update(form: AsForm, post: AsPost) -> Response {
        text(format!("updated {} to {}", post.title, form.title))
    }

    #[handler(Self = AsPosts)]
    pub async fn twice(first: AsForm, second: AsForm) -> Response {
        text(format!("{} {}", first.title, second.title))
    }

    #[handler(Self = AsPosts)]
    pub async fn user_post(user: AsUser, post: AsPost) -> Response {
        text(format!("{} {}", user.name, post.title))
    }
}

async fn fixture() -> TestDatabase {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    run_sql(
        &db,
        &[
            "CREATE TABLE as_posts (id INTEGER PRIMARY KEY AUTOINCREMENT, slug TEXT NOT NULL, title TEXT NOT NULL)",
            "CREATE TABLE as_users (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL)",
            "INSERT INTO as_posts (slug, title) VALUES ('hello', 'first')",
        ],
    )
    .await;
    db
}

#[tokio::test]
async fn bind_003_an_associated_handler_binds_as_a_free_one_does() {
    let _db = fixture().await;
    let router: Router = Router::new()
        .get("/posts/{post}", AsPosts::show)
        .get("/slugs/{post:slug}", AsPosts::show)
        .get("/wrapped/{post}", AsPosts::wrapped)
        .get("/bare/{post}", AsPosts::bare)
        .get("/ids/{id}", AsPosts::by_id)
        .into();
    router
        .prepare_bindings()
        .expect("every route declares what its handler reads");
    let addr = serve(router).await;
    assert_eq!(get(addr, "/posts/1").await, (200, "show first".to_owned()));
    assert_eq!(
        get(addr, "/slugs/hello").await,
        (200, "show first".to_owned()),
        "the binding field reaches the associated handler's binding"
    );
    assert_eq!(
        get(addr, "/wrapped/1").await,
        (200, "wrapped first".to_owned())
    );
    assert_eq!(get(addr, "/bare/1").await, (200, "bare first".to_owned()));
    assert_eq!(get(addr, "/ids/7").await, (200, "id 7".to_owned()));
    let (status, body) = get(addr, "/posts/9").await;
    assert_eq!((status, message(&body).as_str()), (404, "AsPost not found"));
}

#[tokio::test]
async fn bind_003_a_missing_row_answers_404_before_an_associated_handler_reads_the_body() {
    let _db = fixture().await;
    let addr = serve(Router::new().put("/posts/{post}", AsPosts::update).into()).await;
    // The form is declared first and its body is invalid, yet the missing
    // row answers first: the router bound the post ahead of the handler.
    let (status, body) = send(addr, "PUT", "/posts/9", &[], Some(r#"{"title":"x"}"#)).await;
    assert_eq!((status, message(&body).as_str()), (404, "AsPost not found"));
    assert_eq!(
        send(addr, "PUT", "/posts/1", &[], Some(r#"{"title":"later"}"#)).await,
        (200, "updated first to later".to_owned())
    );
}

#[test]
fn bind_003_an_associated_handler_reading_an_undeclared_parameter_is_refused() {
    let router: Router = Router::new().get("/users/{user}", AsPosts::by_id).into();
    let error = refusal(&router);
    assert!(error.contains("GET /users/{user}"), "{error}");
    assert!(error.contains("`id`"), "{error}");
    assert!(error.contains("does not declare"), "{error}");

    let router: Router = Router::new().get("/users/{user}", AsPosts::show).into();
    let error = refusal(&router);
    assert!(error.contains("`post`"), "{error}");
}

#[test]
fn bind_003_an_associated_handler_refused_at_startup_stops_the_boot() {
    let router: Router = Router::new().get("/users/{user}", AsPosts::by_id).into();
    let result =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| Server::from_config(router)));
    let outcome = result.expect("a refused router must not panic the boot");
    let error = outcome.err().expect("the boot must return the refusal");
    assert!(error.to_string().contains("GET /users/{user}"), "{error}");
    assert!(error.to_string().contains("`id`"), "{error}");
}

#[tokio::test]
async fn bind_003_an_associated_handler_refused_at_startup_refuses_handle_request() {
    let _db = fixture().await;
    let router: Router = Router::new()
        .get("/users/{user}", AsPosts::by_id)
        .get("/fine/{post}", AsPosts::show)
        .into();
    let addr = serve(router).await;
    // A router whose checks fail answers every request with the error, as
    // a server built from it would refuse to start.
    let (status, body) = get(addr, "/fine/1").await;
    assert_eq!(status, 500, "{body}");
}

#[test]
fn bind_003_the_other_startup_checks_see_an_associated_handler() {
    // BIND-004: a binding field that is not a column.
    let router: Router = Router::new()
        .get("/posts/{post:nope}", AsPosts::show)
        .into();
    let error = refusal(&router);
    assert!(error.contains("`nope`"), "{error}");

    // BIND-006: a scoped child whose parent declares no relationship.
    let router: Router = Router::new()
        .get("/users/{user}/posts/{post:slug}", AsPosts::user_post)
        .into();
    let error = refusal(&router);
    assert!(error.contains("`AsUser`"), "{error}");
    assert!(error.contains("`posts`"), "{error}");

    // BIND-007: a resolver of another type than the argument it binds.
    let router: Router = Router::new().get("/posts/{post}", AsPosts::show).into();
    let router = router.bind(
        "post",
        |value: String, _route| async move { Ok(Some(value)) },
    );
    let error = refusal(&router);
    assert!(error.contains("String"), "{error}");
    assert!(error.contains("AsPost"), "{error}");

    // BIND-015: two arguments that read the body.
    let router: Router = Router::new().post("/twice", AsPosts::twice).into();
    let error = refusal(&router);
    assert!(error.contains("POST /twice"), "{error}");
    assert!(error.contains("`first: AsForm`"), "{error}");
    assert!(error.contains("`second: AsForm`"), "{error}");
}
