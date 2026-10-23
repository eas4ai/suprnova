//! Precognition hooks (PAR-162): `Precognition::after_validation` turns any
//! error bag into the Precognition answer, as Laravel's
//! `Precognition::afterValidationHook` does, and while a form request's or
//! a multipart form's after-validation hooks run for a request that lists
//! fields in `Precognition-Validate-Only`, `Precognition::should_validate`
//! answers the selection and a database rule on an unlisted field neither
//! queries nor fails, as Laravel's `filterPrecognitiveRules` drops the
//! rules of unlisted fields before the validator runs. Errors a hook adds
//! stay unfiltered.
//!
//! The database tests use an in-memory SQLite connection bound through
//! `TestContainer`. A rule that runs against a table that does not exist
//! reports `validation-unchecked`, so a `204` there proves no query ran.

use std::convert::Infallible;
use std::sync::Arc;

use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};
use serde::Deserialize;
use suprnova::http::upload::MultipartRequestHooks;
use suprnova::testing::TestContainer;
use suprnova::{
    AsyncRule, Exists, FormRequest, Middleware, MiddlewareRegistry, Next, Precognition,
    Precognitive, Request, Response, Router, Unique, ValidationErrors, async_trait,
};
use validator::Validate;

// ---- Fixtures --------------------------------------------------------------

/// The Falsifier's form: a `Unique` rule on `email` in the async hook.
#[derive(Deserialize, Validate, suprnova::FormRequestDerive)]
#[form_request(custom_hooks)]
struct SignupForm {
    #[validate(email)]
    email: String,
    #[validate(range(min = 1))]
    count: u32,
}

#[async_trait]
impl FormRequest for SignupForm {
    async fn after_validation_async(&self) -> Result<(), ValidationErrors> {
        let mut errors = ValidationErrors::new();
        Unique::new("infra_users", "email")
            .check_async(&self.email, &mut errors, "email")
            .await;
        errors.into_result()
    }
}

/// `Exists::check_value` on a scalar and `Exists::check_each` on an array.
#[derive(Deserialize, Validate, suprnova::FormRequestDerive)]
#[form_request(custom_hooks)]
struct AssignForm {
    team_id: i64,
    tag_ids: Vec<i64>,
}

#[async_trait]
impl FormRequest for AssignForm {
    async fn after_validation_async(&self) -> Result<(), ValidationErrors> {
        let mut errors = ValidationErrors::new();
        Exists::new("infra_teams", "id")
            .check_value(self.team_id, &mut errors, "team_id")
            .await;
        Exists::new("infra_tags", "id")
            .check_each(&self.tag_ids, &mut errors, "tag_ids")
            .await;
        errors.into_result()
    }
}

/// Reports what `Precognition::should_validate` answers inside each hook,
/// as an error the hook adds, which is never filtered.
#[derive(Deserialize, Validate, suprnova::FormRequestDerive)]
#[form_request(custom_hooks)]
struct ProbeForm {
    email: String,
    count: u32,
    asynchronous: bool,
}

fn probe(stage: &str) -> ValidationErrors {
    let mut errors = ValidationErrors::new();
    errors.add(
        "probe",
        format!(
            "{stage} email={} count={}",
            Precognition::should_validate("email"),
            Precognition::should_validate("count")
        ),
    );
    errors
}

#[async_trait]
impl FormRequest for ProbeForm {
    fn after_validation(&self) -> Result<(), ValidationErrors> {
        if self.asynchronous {
            return Ok(());
        }
        probe("sync").into_result()
    }

    async fn after_validation_async(&self) -> Result<(), ValidationErrors> {
        probe("async").into_result()
    }
}

/// The multipart form's hooks take the same selection.
#[derive(suprnova::MultipartRequest)]
#[multipart(custom_hooks)]
struct SignupUpload {
    #[field("email")]
    email: String,
    #[field("count")]
    count: u32,
}

#[async_trait]
impl MultipartRequestHooks for SignupUpload {
    async fn after_validation_async(&self) -> Result<(), ValidationErrors> {
        let mut errors = ValidationErrors::new();
        Unique::new("infra_users", "email")
            .check_async(&self.email, &mut errors, "email")
            .await;
        errors.into_result()
    }
}

/// A middleware that validates headers itself and hands the bag to
/// `Precognition::after_validation`, as Laravel's hook works with any
/// validator.
#[derive(Clone)]
struct HeaderCheck;

#[async_trait]
impl Middleware for HeaderCheck {
    async fn handle(&self, request: Request, next: Next) -> Response {
        let mut bag = ValidationErrors::new();
        if request.header("X-Bad-Email").is_some() {
            bag.add("email", "The email must be a valid email address.");
        }
        Precognition::after_validation(&request, bag)?;
        next(request).await
    }
}

mod routes {
    use super::*;
    use suprnova::{handler, post, routes};

    #[handler]
    async fn signup(input: SignupForm) -> Response {
        suprnova::text(input.count.to_string())
    }
    #[handler]
    async fn assign(input: AssignForm) -> Response {
        suprnova::text(input.tag_ids.len().to_string())
    }
    #[handler]
    async fn probe(input: ProbeForm) -> Response {
        suprnova::text(format!("{} {}", input.email, input.count))
    }
    #[handler]
    async fn upload(input: SignupUpload) -> Response {
        suprnova::text(format!("{} {}", input.email, input.count))
    }
    routes! {
        post!("/signup", signup).middleware(Precognitive),
        post!("/assign", assign).middleware(Precognitive),
        post!("/probe", probe).middleware(Precognitive),
        post!("/upload", upload).middleware(Precognitive),
    }
}

fn header_router() -> Router {
    suprnova::post!("/check", |_request: Request| async {
        suprnova::text("body")
    })
    .middleware(Precognitive)
    .middleware(HeaderCheck)
    .register(Router::new())
}

/// Bind an in-memory database holding `statements` for the guard's scope.
async fn bind_database(statements: &[&str]) {
    let database = Database::connect("sqlite::memory:")
        .await
        .expect("an in-memory database");
    for sql in statements {
        database
            .execute_raw(Statement::from_string(DbBackend::Sqlite, *sql))
            .await
            .expect("the fixture statement runs");
    }
    TestContainer::singleton(suprnova::DbConnection::from_raw(database));
}

async fn send(
    router: Router,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> hyper::Response<Bytes> {
    let router = Arc::new(router);
    let registry = Arc::new(MiddlewareRegistry::new());
    let (client, server) = tokio::io::duplex(256 * 1024);
    tokio::spawn(async move {
        let service = service_fn(move |request| {
            let router = router.clone();
            let registry = registry.clone();
            async move { Ok::<_, Infallible>(suprnova::handle_request(router, registry, request).await) }
        });
        let _ = http1::Builder::new()
            .serve_connection(TokioIo::new(server), service)
            .await;
    });
    let (mut sender, connection) =
        hyper::client::conn::http1::handshake::<_, Full<Bytes>>(TokioIo::new(client))
            .await
            .expect("handshake");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    let mut request = hyper::Request::builder()
        .method("POST")
        .uri(path)
        .header("host", "localhost")
        .header("content-length", body.len());
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let request = request
        .body(Full::new(Bytes::copy_from_slice(body.as_bytes())))
        .expect("a request");
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        sender.send_request(request),
    )
    .await
    .expect("the request answers in time")
    .expect("a response");
    let (parts, body) = response.into_parts();
    let bytes = tokio::time::timeout(std::time::Duration::from_secs(20), body.collect())
        .await
        .expect("the body arrives in time")
        .expect("a body")
        .to_bytes();
    hyper::Response::from_parts(parts, bytes)
}

fn assert_success(response: &hyper::Response<Bytes>) {
    assert_eq!(response.status(), 204, "{:?}", response.body());
    assert_eq!(
        response
            .headers()
            .get("Precognition-Success")
            .map(|value| value.as_bytes()),
        Some(&b"true"[..])
    );
    assert_eq!(
        response
            .headers()
            .get("Precognition")
            .map(|value| value.as_bytes()),
        Some(&b"true"[..])
    );
}

fn body_json(response: &hyper::Response<Bytes>) -> serde_json::Value {
    serde_json::from_slice(response.body()).expect("a JSON body")
}

fn error_keys(response: &hyper::Response<Bytes>) -> Vec<String> {
    assert_eq!(response.status(), 422, "{:?}", response.body());
    let mut keys: Vec<String> = body_json(response)["errors"]
        .as_object()
        .expect("an errors object")
        .keys()
        .cloned()
        .collect();
    keys.sort();
    keys
}

fn precognitive_json(only: Option<&'static str>) -> Vec<(&'static str, &'static str)> {
    let mut headers = vec![
        ("Precognition", "true"),
        ("Content-Type", "application/json"),
    ];
    if let Some(only) = only {
        headers.push(("Precognition-Validate-Only", only));
    }
    headers
}

const USERS: &[&str] = &[
    "CREATE TABLE infra_users (email TEXT NOT NULL)",
    "INSERT INTO infra_users (email) VALUES ('used@example.com')",
];

const SIGNUP_BODY: &str = r#"{"email":"used@example.com","count":1}"#;

// ---- Database rules in hooks follow the selection ---------------------------

#[tokio::test]
async fn unique_in_a_hook_does_not_fail_an_unlisted_field() {
    let _guard = TestContainer::fake();
    bind_database(USERS).await;

    let response = send(
        routes::register(),
        "/signup",
        &precognitive_json(Some("count")),
        SIGNUP_BODY,
    )
    .await;
    assert_success(&response);
}

#[tokio::test]
async fn unique_in_a_hook_never_queries_for_an_unlisted_field() {
    let _guard = TestContainer::fake();
    // No `infra_users` table: a query would fail and report the field.
    bind_database(&[]).await;

    let response = send(
        routes::register(),
        "/signup",
        &precognitive_json(Some("count")),
        SIGNUP_BODY,
    )
    .await;
    assert_success(&response);

    // The same rule on a listed field queries, and the missing table
    // reports it, so the `204` above is the skip.
    let response = send(
        routes::register(),
        "/signup",
        &precognitive_json(Some("email")),
        SIGNUP_BODY,
    )
    .await;
    assert_eq!(error_keys(&response), ["email"]);
}

#[tokio::test]
async fn unique_in_a_hook_keeps_a_listed_field_and_an_ordinary_request() {
    let _guard = TestContainer::fake();
    bind_database(USERS).await;

    for only in [Some("email"), Some("count,email"), None] {
        let response = send(
            routes::register(),
            "/signup",
            &precognitive_json(only),
            SIGNUP_BODY,
        )
        .await;
        assert_eq!(error_keys(&response), ["email"], "{only:?}");
    }

    let response = send(
        routes::register(),
        "/signup",
        &[("Content-Type", "application/json")],
        SIGNUP_BODY,
    )
    .await;
    assert_eq!(error_keys(&response), ["email"]);
    assert!(response.headers().get("Precognition").is_none());
}

#[tokio::test]
async fn exists_check_each_runs_only_under_the_wildcard_key() {
    let _guard = TestContainer::fake();
    bind_database(&[
        "CREATE TABLE infra_teams (id INTEGER NOT NULL)",
        "INSERT INTO infra_teams (id) VALUES (7)",
        "CREATE TABLE infra_tags (id INTEGER NOT NULL)",
        "INSERT INTO infra_tags (id) VALUES (1)",
    ])
    .await;
    let body = r#"{"team_id":99,"tag_ids":[1,5]}"#;

    for only in ["tag_ids", "", "count"] {
        let response = send(
            routes::register(),
            "/assign",
            &precognitive_json(Some(only)),
            body,
        )
        .await;
        assert_success(&response);
    }

    for (only, keys) in [
        (Some("tag_ids.*"), vec!["tag_ids.1"]),
        (Some("team_id"), vec!["team_id"]),
        (Some("team_id,tag_ids.*"), vec!["tag_ids.1", "team_id"]),
        (None, vec!["tag_ids.1", "team_id"]),
    ] {
        let response = send(
            routes::register(),
            "/assign",
            &precognitive_json(only),
            body,
        )
        .await;
        assert_eq!(error_keys(&response), keys, "{only:?}");
    }
}

#[tokio::test]
async fn a_multipart_hook_takes_the_same_selection() {
    let _guard = TestContainer::fake();
    bind_database(USERS).await;
    let body = "--BOUNDARY\r\nContent-Disposition: form-data; name=\"email\"\r\n\r\nused@example.com\r\n--BOUNDARY\r\nContent-Disposition: form-data; name=\"count\"\r\n\r\n1\r\n--BOUNDARY--\r\n";
    let headers = |only: &'static str| {
        vec![
            ("Precognition", "true"),
            ("Precognition-Validate-Only", only),
            ("Content-Type", "multipart/form-data; boundary=BOUNDARY"),
        ]
    };

    let response = send(routes::register(), "/upload", &headers("count"), body).await;
    assert_success(&response);

    let response = send(routes::register(), "/upload", &headers("email"), body).await;
    assert_eq!(error_keys(&response), ["email"]);
}

// ---- Precognition::should_validate -----------------------------------------

#[test]
fn should_validate_is_true_outside_a_hook() {
    assert!(Precognition::should_validate("email"));
    assert!(Precognition::should_validate("tag_ids.*"));
}

#[tokio::test]
async fn should_validate_answers_the_selection_inside_each_hook() {
    for (asynchronous, stage) in [(false, "sync"), (true, "async")] {
        let body =
            format!(r#"{{"email":"a@example.com","count":1,"asynchronous":{asynchronous}}}"#);
        for (only, expected) in [
            (Some("email"), "email=true count=false"),
            (Some("count"), "email=false count=true"),
            (Some(""), "email=false count=false"),
            (None, "email=true count=true"),
        ] {
            let response = send(
                routes::register(),
                "/probe",
                &precognitive_json(only),
                &body,
            )
            .await;
            assert_eq!(error_keys(&response), ["probe"], "{stage} {only:?}");
            assert_eq!(
                body_json(&response)["errors"]["probe"][0],
                format!("{stage} {expected}"),
                "{only:?}"
            );
        }

        // An ordinary request selects every field.
        let response = send(
            routes::register(),
            "/probe",
            &[("Content-Type", "application/json")],
            &body,
        )
        .await;
        assert_eq!(
            body_json(&response)["errors"]["probe"][0],
            format!("{stage} email=true count=true")
        );
    }
}

// ---- Precognition::after_validation ----------------------------------------

#[tokio::test]
async fn after_validation_answers_204_for_an_empty_bag_on_a_precognitive_request() {
    let response = send(header_router(), "/check", &[("Precognition", "true")], "").await;
    assert_success(&response);
    assert!(response.body().is_empty());
}

#[tokio::test]
async fn after_validation_answers_422_with_the_bag_on_a_precognitive_request() {
    for headers in [
        vec![("Precognition", "true"), ("X-Bad-Email", "1")],
        vec![
            ("Precognition", "true"),
            ("Precognition-Validate-Only", "email"),
            ("X-Bad-Email", "1"),
        ],
    ] {
        let response = send(header_router(), "/check", &headers, "").await;
        assert_eq!(error_keys(&response), ["email"]);
        assert_eq!(
            response
                .headers()
                .get("Precognition")
                .map(|value| value.as_bytes()),
            Some(&b"true"[..])
        );
        assert!(response.headers().get("Precognition-Success").is_none());
    }
}

#[tokio::test]
async fn after_validation_passes_or_fails_an_ordinary_request() {
    let response = send(header_router(), "/check", &[], "").await;
    assert_eq!(response.status(), 200);
    assert_eq!(response.body(), "body");

    let response = send(header_router(), "/check", &[("X-Bad-Email", "1")], "").await;
    assert_eq!(error_keys(&response), ["email"]);
    assert!(response.headers().get("Precognition").is_none());
}
