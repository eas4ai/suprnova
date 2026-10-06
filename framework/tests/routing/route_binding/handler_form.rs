//! BIND-015: the handler form. An argument binds from the route parameter
//! its name names whenever its type implements `RouteBinding`, written as
//! the type alone; a primitive stays a path value; any other type stays a
//! form request. Bound arguments are extracted first, so a missing row is a
//! 404 before the body is validated. Two body readers are refused at
//! startup; a generic handler is exempt.

use suprnova::http::text;
use suprnova::testing::TestDatabase;
use suprnova::{FromRequest, Request, Response, Router, handler, model, request};

use super::{get, message, refusal, run_sql, send, serve};

#[model(table = "hf_posts", fillable = ["title"])]
pub struct HfPost {
    pub id: i64,
    pub title: String,
}

#[request]
pub struct HfPostForm {
    #[validate(length(min = 3))]
    pub title: String,
}

#[handler]
pub async fn show(post: HfPost) -> Response {
    text(format!("post {}", post.title))
}

#[handler]
pub async fn show_id(id: i64) -> Response {
    text(format!("id {id}"))
}

#[handler]
pub async fn store(form: HfPostForm) -> Response {
    text(format!("stored {}", form.title))
}

#[handler]
pub async fn update(form: HfPostForm, post: HfPost) -> Response {
    text(format!("updated {} to {}", post.title, form.title))
}

#[handler]
pub async fn maybe(post: Option<HfPost>) -> Response {
    text(match post {
        Some(post) => format!("some {}", post.title),
        None => "none".to_owned(),
    })
}

#[handler]
pub async fn twice(first: HfPostForm, second: HfPostForm) -> Response {
    text(format!("{} {}", first.title, second.title))
}

#[handler]
pub async fn with_request(post: HfPost, req: Request) -> Response {
    text(format!("{} at {}", post.title, req.path()))
}

/// A generic handler: the generated code cannot choose for `T`, so it reads
/// the body, and the handler carries no record.
#[handler]
pub async fn generic<T: FromRequest + Send + 'static>(id: i64, _body: T) -> Response {
    text(format!("generic {id}"))
}

async fn fixture() -> TestDatabase {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    run_sql(
        &db,
        &[
            "CREATE TABLE hf_posts (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL)",
            "INSERT INTO hf_posts (id, title) VALUES (1, 'first')",
        ],
    )
    .await;
    db
}

#[tokio::test]
async fn bind_015_a_type_alone_binds_from_its_parameter() {
    let _db = fixture().await;
    let addr = serve(Router::new().get("/posts/{post}", show).into()).await;
    assert_eq!(get(addr, "/posts/1").await, (200, "post first".to_owned()));
    assert_eq!(get(addr, "/posts/2").await.0, 404);
}

#[tokio::test]
async fn bind_015_a_primitive_stays_a_path_value_and_another_type_a_form_request() {
    let _db = fixture().await;
    let router: Router = Router::new()
        .get("/ids/{id}", show_id)
        .post("/posts", store)
        .into();
    router
        .prepare_bindings()
        .expect("both routes are well formed");
    let addr = serve(router).await;
    assert_eq!(get(addr, "/ids/12").await, (200, "id 12".to_owned()));
    assert_eq!(
        send(addr, "POST", "/posts", &[], Some(r#"{"title":"hello"}"#)).await,
        (200, "stored hello".to_owned())
    );
    assert_eq!(
        send(addr, "POST", "/posts", &[], Some(r#"{"title":"x"}"#))
            .await
            .0,
        422,
        "the form is validated"
    );
}

#[tokio::test]
async fn bind_015_a_missing_row_answers_404_before_the_body_is_validated() {
    let _db = fixture().await;
    let addr = serve(Router::new().put("/posts/{post}", update).into()).await;
    // The form is declared first and its body is invalid, yet the missing
    // row answers first.
    let (status, body) = send(addr, "PUT", "/posts/99", &[], Some(r#"{"title":"x"}"#)).await;
    assert_eq!((status, message(&body).as_str()), (404, "HfPost not found"));
    // With the row there, the form is read and validated.
    assert_eq!(
        send(addr, "PUT", "/posts/1", &[], Some(r#"{"title":"x"}"#))
            .await
            .0,
        422
    );
    assert_eq!(
        send(addr, "PUT", "/posts/1", &[], Some(r#"{"title":"later"}"#)).await,
        (200, "updated first to later".to_owned())
    );
}

#[tokio::test]
async fn bind_015_an_option_binds_none_when_its_optional_parameter_is_absent() {
    let _db = fixture().await;
    let addr = serve(Router::new().get("/maybe/{post?}", maybe).into()).await;
    assert_eq!(get(addr, "/maybe").await, (200, "none".to_owned()));
    assert_eq!(get(addr, "/maybe/1").await, (200, "some first".to_owned()));
    assert_eq!(
        get(addr, "/maybe/5").await.0,
        404,
        "a value that matches nothing"
    );
}

#[tokio::test]
async fn bind_015_a_bound_argument_and_the_request_mix() {
    let _db = fixture().await;
    let addr = serve(Router::new().get("/with/{post}", with_request).into()).await;
    assert_eq!(
        get(addr, "/with/1").await,
        (200, "first at /with/1".to_owned())
    );
}

#[tokio::test]
async fn bind_015_two_body_readers_are_refused_at_startup() {
    let router: Router = Router::new().post("/twice", twice).into();
    let error = refusal(&router);
    assert!(error.contains("POST /twice"), "{error}");
    assert!(error.contains("`first: HfPostForm`"), "{error}");
    assert!(error.contains("`second: HfPostForm`"), "{error}");
    // Driven through `handle_request`, the same refusal answers every
    // request.
    let addr = serve(router).await;
    assert_eq!(
        send(addr, "POST", "/twice", &[], Some(r#"{"title":"abc"}"#))
            .await
            .0,
        500
    );
}

#[tokio::test]
async fn bind_015_a_generic_handler_is_exempt_from_the_startup_checks() {
    // `id` is not declared by the path: a recorded handler would be
    // refused, a generic one is not checked.
    let router: Router = Router::new().post("/generic", generic::<HfPostForm>).into();
    router
        .prepare_bindings()
        .expect("a generic handler carries no record");
    let addr = serve(router).await;
    let (status, _) = send(addr, "POST", "/generic", &[], Some(r#"{"title":"abc"}"#)).await;
    assert_eq!(
        status, 400,
        "the undeclared `id` is missing at request time"
    );
}
