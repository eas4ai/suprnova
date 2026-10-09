//! End-to-end tests for the Tier 5 Precognition flow.
//!
//! `hyper::body::Incoming` isn't constructible outside hyper, so these
//! tests bind a one-shot TCP listener, send a real HTTP request through
//! a hyper client, and assert on the response shape.

use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use serde::Deserialize;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use suprnova::{
    Bail, FormRequest, FrameworkError, FromRequest, HttpResponse, Middleware, MiddlewareRegistry,
    Next, Precognition, Precognitive, Request, Response, Router, async_trait,
};
use validator::Validate;

#[derive(Deserialize, Validate, suprnova::FormRequestDerive)]
#[form_request(custom_hooks)]
struct DatabaseForm {
    #[validate(email)]
    email: String,
    #[validate(range(min = 1))]
    count: u32,
}

#[async_trait]
impl FormRequest for DatabaseForm {
    async fn after_validation_async(&self) -> Result<(), suprnova::ValidationErrors> {
        use suprnova::AsyncRule;
        let mut errors = suprnova::ValidationErrors::new();
        suprnova::Unique::new("precognition_users", "email")
            .check_async(&self.email, &mut errors, "email")
            .await;
        errors.into_result()
    }
}

#[derive(Deserialize, Validate)]
struct HookForm {
    #[validate(email)]
    email: String,
    asynchronous: bool,
}

#[async_trait]
impl FormRequest for HookForm {
    fn after_validation(&self) -> Result<(), suprnova::ValidationErrors> {
        if self.asynchronous {
            return Ok(());
        }
        let mut errors = suprnova::ValidationErrors::new();
        errors.add("other", "synchronous hook refused");
        errors.into_result()
    }

    async fn after_validation_async(&self) -> Result<(), suprnova::ValidationErrors> {
        let mut errors = suprnova::ValidationErrors::new();
        errors.add("other", "asynchronous hook refused");
        errors.into_result()
    }
}

#[derive(Deserialize)]
struct IndexedForm {
    tags: Vec<String>,
}

impl Validate for IndexedForm {
    fn validate(&self) -> Result<(), validator::ValidationErrors> {
        let mut errors = validator::ValidationErrors::new();
        if self.tags.get(3).is_some_and(|tag| tag.is_empty()) {
            errors.add("tags.3", validator::ValidationError::new("required"));
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}
impl FormRequest for IndexedForm {}

#[derive(Deserialize, Validate)]
struct QueryForm {
    #[validate(email)]
    email: String,
    #[validate(length(min = 2))]
    tags: Vec<String>,
    #[validate(nested)]
    profile: QueryProfile,
}

#[derive(Deserialize, Validate)]
struct QueryProfile {
    #[validate(length(min = 2))]
    name: String,
}
impl FormRequest for QueryForm {}

#[derive(Deserialize, Validate)]
struct FileForm {
    photo: suprnova::UploadedFile<suprnova::MaxSize<4>>,
    #[validate(length(min = 2))]
    caption: String,
}
impl FormRequest for FileForm {}

#[derive(suprnova::MultipartRequest)]
struct LiveUpload {
    #[field("photo")]
    photo: suprnova::UploadedFile<suprnova::MaxSize<4>>,
    #[field("count")]
    count: u32,
}

#[derive(suprnova::MultipartRequest)]
struct NoPlaceholderUpload {
    #[field("count")]
    count: u32,
    #[field("address")]
    address: std::net::IpAddr,
}

#[derive(Deserialize, Validate)]
struct NoPlaceholderForm {
    email: String,
    address: std::net::IpAddr,
}
impl FormRequest for NoPlaceholderForm {}

#[derive(Deserialize, Validate)]
struct NoPlaceholderDateForm {
    #[validate(email)]
    email: String,
    count: u32,
    profile: DateProfile,
}
impl FormRequest for NoPlaceholderDateForm {}

#[derive(Deserialize)]
struct DateProfile {
    birth_date: chrono::NaiveDate,
}

mod data_routes {
    use super::*;
    use suprnova::{delete, get, handler, post, routes};

    #[handler]
    async fn database(input: DatabaseForm) -> Response {
        BODY_CALLS.fetch_add(1, Ordering::SeqCst);
        suprnova::text(input.count.to_string())
    }
    #[handler]
    async fn hooks(input: HookForm) -> Response {
        suprnova::text(input.email)
    }
    #[handler]
    async fn indexed(input: IndexedForm) -> Response {
        suprnova::text(input.tags.len().to_string())
    }
    #[handler]
    async fn query(input: QueryForm) -> Response {
        BODY_CALLS.fetch_add(1, Ordering::SeqCst);
        suprnova::text(format!("{} {}", input.email, input.profile.name))
    }
    #[handler]
    async fn file(input: FileForm) -> Response {
        BODY_CALLS.fetch_add(1, Ordering::SeqCst);
        suprnova::text(format!("{} {}", input.caption, input.photo.size))
    }
    #[handler]
    async fn multipart(input: LiveUpload) -> Response {
        BODY_CALLS.fetch_add(1, Ordering::SeqCst);
        suprnova::text(format!("{} {}", input.count, input.photo.size))
    }
    #[handler]
    async fn no_placeholder(input: NoPlaceholderForm) -> Response {
        BODY_CALLS.fetch_add(1, Ordering::SeqCst);
        suprnova::text(format!("{} {}", input.email, input.address))
    }
    #[handler]
    async fn multipart_no_placeholder(input: NoPlaceholderUpload) -> Response {
        BODY_CALLS.fetch_add(1, Ordering::SeqCst);
        suprnova::text(format!("{} {}", input.count, input.address))
    }
    #[handler]
    async fn date_no_placeholder(input: NoPlaceholderDateForm) -> Response {
        BODY_CALLS.fetch_add(1, Ordering::SeqCst);
        suprnova::text(format!(
            "{} {} {}",
            input.email, input.count, input.profile.birth_date
        ))
    }
    routes! {
        post!("/database", database).middleware(Precognitive),
        post!("/hooks", hooks).middleware(Precognitive),
        post!("/indexed", indexed).middleware(Precognitive),
        get!("/query", query).middleware(Precognitive),
        delete!("/query", query).middleware(Precognitive),
        post!("/file", file).middleware(Precognitive),
        post!("/multipart", multipart).middleware(Precognitive),
        post!("/no-placeholder", no_placeholder).middleware(Precognitive),
        post!("/multipart-no-placeholder", multipart_no_placeholder).middleware(Precognitive),
        post!("/date-no-placeholder", date_no_placeholder).middleware(Precognitive),
    }
}

fn error_keys(response: &hyper::Response<Bytes>) -> Vec<String> {
    assert_eq!(response.status(), 422);
    assert_eq!(response.headers().get("Precognition").unwrap(), "true");
    let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
    body["errors"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect()
}

#[tokio::test]
async fn precognition_selected_database_rule_runs_after_unlisted_derived_or_parse_failure() {
    use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};
    let _counter_guard = COUNTERS.lock().await;
    BODY_CALLS.store(0, Ordering::SeqCst);
    let _guard = suprnova::testing::TestContainer::fake();
    let database = Database::connect("sqlite::memory:").await.unwrap();
    for sql in [
        "CREATE TABLE precognition_users (email TEXT NOT NULL)",
        "INSERT INTO precognition_users (email) VALUES ('used@example.com')",
    ] {
        database
            .execute_raw(Statement::from_string(DbBackend::Sqlite, sql))
            .await
            .unwrap();
    }
    suprnova::testing::TestContainer::singleton(suprnova::DbConnection::from_raw(database));
    for count in [serde_json::json!(0), serde_json::json!("bad")] {
        let body = serde_json::json!({"email":"used@example.com", "count":count});
        let response = send(
            data_routes::register(),
            "/database",
            &[
                ("Precognition", "true"),
                ("Precognition-Validate-Only", "email"),
                ("Content-Type", "application/json"),
            ],
            &body.to_string(),
        )
        .await;
        assert_eq!(error_keys(&response), ["email"]);
    }
    assert_eq!(BODY_CALLS.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn precognition_unlisted_parse_failure_keeps_selected_validation_on_every_source() {
    let _counter_guard = COUNTERS.lock().await;
    BODY_CALLS.store(0, Ordering::SeqCst);
    for (content_type, body) in [
        ("application/json", r#"{"email":"ada@example.com","count":"bad"}"#.to_owned()),
        ("application/json", r#"{"email":"ada@example.com"}"#.to_owned()),
        ("application/x-www-form-urlencoded", "email=ada%40example.com&count=bad".to_owned()),
        ("multipart/form-data; boundary=BOUNDARY", "--BOUNDARY\r\nContent-Disposition: form-data; name=\"email\"\r\n\r\nada@example.com\r\n--BOUNDARY\r\nContent-Disposition: form-data; name=\"count\"\r\n\r\nbad\r\n--BOUNDARY--\r\n".to_owned()),
    ] {
        for (only, status) in [("email", 204), ("count", 422)] {
            let response = send(protocol_routes::register(), "/bound/7", &[
                ("Precognition", "true"), ("Precognition-Validate-Only", only),
                ("Content-Type", content_type),
            ], &body).await;
            assert_eq!(response.status(), status, "{content_type}: {only}");
            if status == 204 { assert_success(&response); } else { assert_eq!(error_keys(&response), ["count"]); }
        }
    }
    assert_eq!(BODY_CALLS.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn precognition_unlisted_json_shapes_use_typed_placeholders() {
    for body in [
        r#"{"email":"ada@example.com","count":false,"tags":"bad"}"#,
        r#"{"email":"ada@example.com","count":{},"tags":[{},false]}"#,
    ] {
        let response = send(
            protocol_routes::register(),
            "/bound/7",
            &[
                ("Precognition", "true"),
                ("Precognition-Validate-Only", "email"),
                ("Content-Type", "application/json"),
            ],
            body,
        )
        .await;
        assert_success(&response);
    }
}

#[tokio::test]
async fn precognition_empty_selection_drops_every_derived_error() {
    for only in ["", " , , "] {
        assert_success(
            &send(
                protocol_routes::register(),
                "/form",
                &[
                    ("Precognition", "true"),
                    ("Precognition-Validate-Only", only),
                    ("Content-Type", "application/json"),
                ],
                r#"{"email":"bad","password":"short"}"#,
            )
            .await,
        );
    }
}

#[tokio::test]
async fn precognition_hooks_keep_unlisted_messages_after_selected_rules_pass() {
    for asynchronous in [true, false] {
        let body = serde_json::json!({"email":"ada@example.com","asynchronous":asynchronous});
        for only in ["email", ""] {
            let response = send(
                data_routes::register(),
                "/hooks",
                &[
                    ("Precognition", "true"),
                    ("Precognition-Validate-Only", only),
                    ("Content-Type", "application/json"),
                ],
                &body.to_string(),
            )
            .await;
            assert_eq!(error_keys(&response), ["other"]);
        }
    }
}

#[tokio::test]
async fn precognition_selection_and_should_validate_share_exact_wildcard_match() {
    for (only, matches) in [
        ("tags", false),
        ("tags.*", true),
        ("tags.3", true),
        ("tags.3.*", false),
    ] {
        assert_marked_selection(only, Some(vec![only.into()]), &[("tags.3", matches)]).await;
        let response = send(
            data_routes::register(),
            "/indexed",
            &[
                ("Precognition", "true"),
                ("Precognition-Validate-Only", only),
                ("Content-Type", "application/json"),
            ],
            r#"{"tags":["a","b","c",""]}"#,
        )
        .await;
        if matches {
            assert_eq!(error_keys(&response), ["tags.3"]);
        } else {
            assert_success(&response);
        }
    }
    let absent = Request::for_test("POST", "/indexed");
    assert_eq!(absent.validate_only(), None);
    assert!(absent.should_validate("tags.3"));
    assert_marked_selection(
        " tags.* , , email ",
        Some(vec!["tags.*".into(), "email".into()]),
        &[
            ("tags", false),
            ("tags.3", true),
            ("tags.", false),
            ("tags.3.name", false),
            ("email", true),
        ],
    )
    .await;
    assert_marked_selection("", Some(vec![]), &[("email", false)]).await;
}

async fn assert_marked_selection(
    only: &str,
    expected: Option<Vec<String>>,
    checks: &[(&'static str, bool)],
) {
    let request = Request::for_test_with_headers(
        "POST",
        "/indexed",
        [
            ("Precognition", "true"),
            ("Precognition-Validate-Only", only),
        ],
    );
    let checks = checks.to_vec();
    let next: Next = Arc::new(move |request| {
        let expected = expected.clone();
        let checks = checks.clone();
        Box::pin(async move {
            assert!(request.is_precognitive());
            assert_eq!(request.validate_only(), expected);
            for (field, expected) in checks {
                assert_eq!(request.should_validate(field), expected, "{field}");
            }
            suprnova::text("checked")
        })
    });
    let response = Precognitive
        .handle(request, next)
        .await
        .unwrap_or_else(|response| {
            panic!("selection middleware returned {}", response.status_code())
        });
    assert_eq!(response.body(), b"checked");
}

#[test]
fn precognition_unmarked_selection_includes_every_field() {
    for only in ["email", ""] {
        for attempting in [false, true] {
            let mut headers = vec![("Precognition-Validate-Only", only)];
            if attempting {
                headers.push(("Precognition", "true"));
            }
            let request = Request::for_test_with_headers("POST", "/real", headers);
            assert!(!request.is_precognitive());
            assert_eq!(request.validate_only(), None);
            assert!(request.should_validate("avatar"));
            assert!(request.should_validate("email"));
        }
    }
}

#[tokio::test]
async fn precognition_get_and_delete_validate_query_lists_and_encoded_objects() {
    let _counter_guard = COUNTERS.lock().await;
    BODY_CALLS.store(0, Ordering::SeqCst);
    for method in ["GET", "DELETE"] {
        for (email, status) in [("", 422), ("ada%40example.com", 204)] {
            let path = format!(
                "/query?email={email}&tags[]=a&tags[]=b&profile=%7B%22name%22%3A%22Ada%22%7D"
            );
            let response = send_method(
                data_routes::register(),
                method,
                &path,
                &[("Precognition", "true")],
                "",
            )
            .await;
            assert_eq!(response.status(), status);
            if status == 422 {
                assert_eq!(error_keys(&response), ["email"]);
            } else {
                assert_success(&response);
            }
        }
        for path in [
            "/query?email=ada%40example.com&tags[]=a&profile=%7B%22name%22%3A%22Ada%22%7D",
            "/query?email=ada%40example.com&tags[]=a&tags[]=b&profile=%7B%22name%22%3A%22A%22%7D",
        ] {
            let response = send_method(
                data_routes::register(),
                method,
                path,
                &[("Precognition", "true")],
                "",
            )
            .await;
            assert_eq!(response.status(), 422);
        }
    }
    assert_eq!(BODY_CALLS.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn precognition_query_body_wins_and_selection_is_shared() {
    let response = send_method(
        data_routes::register(),
        "GET",
        "/query?email=bad&tags[]=a&profile=%7B%22name%22%3A%22A%22%7D",
        &[
            ("Precognition", "true"),
            ("Content-Type", "application/json"),
        ],
        r#"{"email":"ada@example.com","tags":["a","b"],"profile":{"name":"Ada"}}"#,
    )
    .await;
    assert_success(&response);
    let response = send_method(
        data_routes::register(),
        "DELETE",
        "/query?email=ada%40example.com&tags[]=a&profile=%7B%22name%22%3A%22A%22%7D",
        &[
            ("Precognition", "true"),
            ("Precognition-Validate-Only", "email"),
        ],
        "",
    )
    .await;
    assert_success(&response);
}

#[tokio::test]
async fn precognition_multipart_file_rules_and_derive_never_reach_handler() {
    let _counter_guard = COUNTERS.lock().await;
    BODY_CALLS.store(0, Ordering::SeqCst);
    for path in ["/file", "/multipart"] {
        for (file, status) in [("hi", 204), ("hello", 422)] {
            let body = format!(
                "--BOUNDARY\r\nContent-Disposition: form-data; name=\"photo\"; filename=\"a.txt\"\r\nContent-Type: text/plain\r\n\r\n{file}\r\n--BOUNDARY\r\nContent-Disposition: form-data; name=\"caption\"\r\n\r\nHi\r\n--BOUNDARY\r\nContent-Disposition: form-data; name=\"count\"\r\n\r\n2\r\n--BOUNDARY--\r\n"
            );
            let response = send(
                data_routes::register(),
                path,
                &[
                    ("Precognition", "true"),
                    ("Content-Type", "multipart/form-data; boundary=BOUNDARY"),
                ],
                &body,
            )
            .await;
            assert_eq!(response.status(), status, "{path}: {file}");
            if status == 204 {
                assert_success(&response);
            } else {
                assert_eq!(error_keys(&response), ["photo"]);
            }
        }
    }
    assert_eq!(BODY_CALLS.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn precognition_multipart_derive_narrows_fields_and_recovers_unlisted_parse_failure() {
    for (only, status) in [("photo", 204), ("count", 422), ("", 204)] {
        let body = "--BOUNDARY\r\nContent-Disposition: form-data; name=\"photo\"; filename=\"a.txt\"\r\n\r\nhi\r\n--BOUNDARY\r\nContent-Disposition: form-data; name=\"count\"\r\n\r\nbad\r\n--BOUNDARY--\r\n";
        let response = send(
            data_routes::register(),
            "/multipart",
            &[
                ("Precognition", "true"),
                ("Precognition-Validate-Only", only),
                ("Content-Type", "multipart/form-data; boundary=BOUNDARY"),
            ],
            body,
        )
        .await;
        assert_eq!(response.status(), status);
    }
}

#[derive(Clone)]
struct InlineValidation;
#[async_trait]
impl Middleware for InlineValidation {
    async fn handle(&self, request: Request, _next: Next) -> Response {
        let input = request.validate::<SignupRequest>().await?;
        Ok(HttpResponse::text(input.email))
    }
}

#[tokio::test]
async fn precognition_inline_validation_answers_protocol_and_returns_real_value() {
    for (marked, only, body, status) in [
        (true, "email", r#"{"email":"bad","password":"short"}"#, 422),
        (
            true,
            "email",
            r#"{"email":"ada@example.com","password":"short"}"#,
            204,
        ),
        (
            false,
            "email",
            r#"{"email":"ada@example.com","password":"longenough"}"#,
            200,
        ),
        (
            false,
            "email",
            r#"{"email":"ada@example.com","password":"short"}"#,
            422,
        ),
    ] {
        let router = suprnova::post!("/inline", |_request: Request| async {
            suprnova::text("unused")
        })
        .middleware(Precognitive)
        .middleware(InlineValidation)
        .register(Router::new());
        let mut headers = vec![
            ("Content-Type", "application/json"),
            ("Precognition-Validate-Only", only),
        ];
        if marked {
            headers.push(("Precognition", "true"));
        }
        let response = send(router, "/inline", &headers, body).await;
        assert_eq!(response.status(), status);
        if marked {
            assert_eq!(response.headers().get("Precognition").unwrap(), "true");
            if status == 204 {
                assert_success(&response);
            } else {
                assert_eq!(error_keys(&response), ["email"]);
            }
        } else if status == 200 {
            assert_eq!(response.body(), "ada@example.com");
        }
    }
}

#[tokio::test]
async fn precognition_unselected_type_without_placeholder_answers_its_parse_error() {
    let response = send(
        data_routes::register(),
        "/no-placeholder",
        &[
            ("Precognition", "true"),
            ("Precognition-Validate-Only", "email"),
            ("Content-Type", "application/json"),
        ],
        r#"{"email":"ada@example.com","address":false}"#,
    )
    .await;
    assert_eq!(error_keys(&response), ["address"]);
    let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
    assert_eq!(
        body["errors"]["address"],
        serde_json::json!(["The address field must be a string."])
    );
    assert_eq!(body["message"], body["errors"]["address"][0]);
}

#[tokio::test]
async fn precognition_multipart_selected_parse_error_precedes_missing_placeholder() {
    for (count, field, message) in [
        ("bad", "count", "The count field must be an integer."),
        ("7", "address", "The address field format is invalid."),
    ] {
        let body = format!(
            "--selection\r\nContent-Disposition: form-data; name=\"count\"\r\n\r\n{count}\r\n--selection\r\nContent-Disposition: form-data; name=\"address\"\r\n\r\nbad\r\n--selection--\r\n"
        );
        let response = send(
            data_routes::register(),
            "/multipart-no-placeholder",
            &[
                ("Precognition", "true"),
                ("Precognition-Validate-Only", "count"),
                ("Content-Type", "multipart/form-data; boundary=selection"),
            ],
            &body,
        )
        .await;
        assert_eq!(error_keys(&response), [field]);
        let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        assert_eq!(body["errors"][field], serde_json::json!([message]));
        assert_eq!(body["message"], message);
    }
}

#[tokio::test]
async fn precognition_unlisted_json_date_parse_failure_keeps_its_field_path() {
    let response = send(
        data_routes::register(),
        "/date-no-placeholder",
        &[
            ("Precognition", "true"),
            ("Precognition-Validate-Only", "email"),
            ("Content-Type", "application/json"),
        ],
        r#"{"email":"ada@example.com","count":"bad","profile":{"birth_date":"2026-"}}"#,
    )
    .await;
    assert_eq!(error_keys(&response), ["profile.birth_date"]);
    let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
    assert_eq!(
        body["errors"]["profile.birth_date"],
        serde_json::json!(["The profile.birth date field must be a string."])
    );
    assert_eq!(body["message"], body["errors"]["profile.birth_date"][0]);
}

#[derive(Deserialize, Validate)]
struct SignupRequest {
    #[validate(email)]
    pub email: String,
    #[validate(length(min = 8))]
    pub password: String,
}

impl FormRequest for SignupRequest {}

/// Spawn a one-shot server that routes through `SignupRequest::extract`
/// and returns whatever the conversion produces. Returns the address.
async fn spawn() -> SocketAddr {
    spawn_with_middleware(true).await
}

async fn spawn_with_middleware(enabled: bool) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        if let Ok((stream, _)) = listener.accept().await {
            let io = TokioIo::new(stream);
            let svc = service_fn(
                move |hyper_req: hyper::Request<hyper::body::Incoming>| async move {
                    let req = Request::new(hyper_req);
                    let next: Next = Arc::new(|req| {
                        Box::pin(async move {
                            match SignupRequest::extract(req).await {
                                Ok(_form) => {
                                    // In a real handler this is where the body runs.
                                    suprnova::HttpResponse::json(serde_json::json!({
                                        "ok": true
                                    }))
                                    .status(200)
                                    .ok()
                                }
                                Err(e) => Err(e.into()),
                            }
                        })
                    });
                    let response = if enabled {
                        Precognitive.handle(req, next).await
                    } else {
                        next(req).await
                    };
                    let resp = response.unwrap_or_else(|response| response);
                    Ok::<_, Infallible>(resp.into_hyper())
                },
            );
            let _ = http1::Builder::new().serve_connection(io, svc).await;
        }
    });
    addr
}

async fn post_json(
    addr: SocketAddr,
    body: serde_json::Value,
    headers: &[(&str, &str)],
) -> hyper::Response<Bytes> {
    let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let io = TokioIo::new(stream);
    let (mut sender, conn) = hyper::client::conn::http1::handshake::<_, Full<Bytes>>(io)
        .await
        .unwrap();
    tokio::spawn(async move {
        let _ = conn.await;
    });

    let body_bytes = serde_json::to_vec(&body).unwrap();
    let mut req = hyper::Request::builder()
        .method("POST")
        .uri("http://localhost/signup")
        .header("content-type", "application/json")
        .header("content-length", body_bytes.len());
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let req = req.body(Full::new(Bytes::from(body_bytes))).unwrap();

    let resp = sender.send_request(req).await.unwrap();
    let (parts, body) = resp.into_parts();
    let collected = body.collect().await.unwrap();
    hyper::Response::from_parts(parts, collected.to_bytes())
}

#[tokio::test]
async fn precognition_success_returns_204() {
    let addr = spawn().await;
    let resp = post_json(
        addr,
        serde_json::json!({"email": "a@b.com", "password": "longenough"}),
        &[("Precognition", "true")],
    )
    .await;
    assert_eq!(resp.status(), 204);
    assert_eq!(resp.headers().get("Precognition").unwrap(), "true");
    assert_eq!(resp.headers().get("Precognition-Success").unwrap(), "true");
    assert_eq!(resp.headers().get("Vary").unwrap(), "Precognition");
}

#[tokio::test]
async fn precognition_failure_returns_422_with_filtered_errors() {
    let addr = spawn().await;
    // Both email and password invalid; ask only about email.
    let resp = post_json(
        addr,
        serde_json::json!({"email": "not-an-email", "password": "short"}),
        &[
            ("Precognition", "true"),
            ("Precognition-Validate-Only", "email"),
        ],
    )
    .await;
    assert_eq!(resp.status(), 422);
    assert_eq!(resp.headers().get("Precognition").unwrap(), "true");
    assert_eq!(resp.headers().get("Vary").unwrap(), "Precognition");
    let body: serde_json::Value = serde_json::from_slice(resp.body()).unwrap();
    let errors = body["errors"].as_object().unwrap();
    assert!(errors.contains_key("email"));
    assert!(
        !errors.contains_key("password"),
        "password error should be filtered out: {:?}",
        errors
    );
}

#[tokio::test]
async fn precognition_only_unrequested_fields_failing_returns_204() {
    // password is invalid but client only asked about email which IS
    // valid. From the client's perspective, the answer they asked for
    // is "OK" - return 204, not 422.
    let addr = spawn().await;
    let resp = post_json(
        addr,
        serde_json::json!({"email": "a@b.com", "password": "short"}),
        &[
            ("Precognition", "true"),
            ("Precognition-Validate-Only", "email"),
        ],
    )
    .await;
    assert_eq!(resp.status(), 204);
}

#[tokio::test]
async fn non_precognition_request_runs_handler_on_valid_input() {
    let addr = spawn().await;
    let resp = post_json(
        addr,
        serde_json::json!({"email": "a@b.com", "password": "longenough"}),
        &[], // no Precognition header
    )
    .await;
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = serde_json::from_slice(resp.body()).unwrap();
    assert_eq!(body["ok"], true);
}

#[tokio::test]
async fn non_precognition_invalid_returns_422_without_precognition_headers() {
    let addr = spawn().await;
    let resp = post_json(
        addr,
        serde_json::json!({"email": "bad", "password": "short"}),
        &[],
    )
    .await;
    assert_eq!(resp.status(), 422);
    assert!(resp.headers().get("Precognition").is_none());
    assert_eq!(resp.headers().get("Vary").unwrap(), "Precognition");
}

#[tokio::test]
async fn precognition_case_insensitive() {
    let addr = spawn().await;
    let resp = post_json(
        addr,
        serde_json::json!({"email": "a@b.com", "password": "longenough"}),
        &[("Precognition", "TRUE")],
    )
    .await;
    assert_eq!(resp.status(), 204);
}

#[tokio::test]
async fn precognition_header_without_middleware_runs_real_validation() {
    let resp = post_json(
        spawn_with_middleware(false).await,
        serde_json::json!({"email": "a@b.com", "password": "short"}),
        &[
            ("Precognition", "true"),
            ("Precognition-Validate-Only", "email"),
        ],
    )
    .await;
    assert_eq!(resp.status(), 422);
    assert!(resp.headers().get("Precognition").is_none());
    let body: serde_json::Value = serde_json::from_slice(resp.body()).unwrap();
    assert!(body["errors"].get("password").is_some());
}

#[test]
fn precognition_validation_messages_summarise_all_fields() {
    for count in 1..=3 {
        let mut errors = suprnova::ValidationErrors::new();
        errors.add("email", "Email is required.");
        if count > 1 {
            errors.add("email", "Email is invalid.");
        }
        if count > 2 {
            errors.add("password", "Password is required.");
        }
        for error in [
            suprnova::FrameworkError::Validation(errors.clone()),
            suprnova::FrameworkError::PrecognitionFailure(errors.clone()),
        ] {
            let response: suprnova::HttpResponse = error.into();
            let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
            let first = body["errors"].as_object().unwrap().values().next().unwrap()[0]
                .as_str()
                .unwrap();
            let expected = match count {
                1 => first.to_owned(),
                2 => format!("{first} (and 1 more error)"),
                _ => format!("{first} (and 2 more errors)"),
            };
            assert_eq!(response.status_code(), 422);
            assert_eq!(body["message"], expected);
        }
    }
}

// The protocol tests below drive the router over an in-memory connection.
async fn send(
    router: Router,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> hyper::Response<Bytes> {
    send_method(router, "POST", path, headers, body).await
}

async fn send_method(
    router: Router,
    method: &str,
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
            .unwrap();
    tokio::spawn(async move {
        let _ = connection.await;
    });
    let mut request = hyper::Request::builder()
        .method(method)
        .uri(path)
        .header("host", "localhost")
        .header("content-length", body.len());
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let request = request
        .body(Full::new(Bytes::copy_from_slice(body.as_bytes())))
        .unwrap();
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        sender.send_request(request),
    )
    .await
    .unwrap()
    .unwrap();
    let (parts, body) = response.into_parts();
    let bytes = tokio::time::timeout(std::time::Duration::from_secs(20), body.collect())
        .await
        .unwrap()
        .unwrap()
        .to_bytes();
    hyper::Response::from_parts(parts, bytes)
}

static COUNTERS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static BODY_CALLS: AtomicUsize = AtomicUsize::new(0);
static EXTRACT_CALLS: AtomicUsize = AtomicUsize::new(0);

struct Checked;
#[async_trait]
impl FromRequest for Checked {
    async fn from_request(_request: Request) -> Result<Self, FrameworkError> {
        EXTRACT_CALLS.fetch_add(1, Ordering::SeqCst);
        Ok(Self)
    }
}
struct Missing;
#[async_trait]
impl FromRequest for Missing {
    async fn from_request(_request: Request) -> Result<Self, FrameworkError> {
        Err(FrameworkError::domain("Missing model", 404))
    }
}

#[derive(suprnova::RouteBinding)]
enum Category {
    Known,
}

/// A database model proves missing bindings beat a passing draft form.
#[suprnova::model(table = "precognition_members")]
pub struct PrecognitionMember {
    /// The key resolved from the route.
    pub id: i64,
}

#[derive(suprnova::Data, Validate)]
struct BoundForm {
    #[data(from_route_param("id"))]
    id: u64,
    #[validate(email)]
    email: String,
    count: u32,
    tags: Option<Vec<String>>,
}

#[derive(Deserialize, Validate)]
struct DeniedForm {}
impl FormRequest for DeniedForm {
    fn authorize(_request: &Request) -> bool {
        false
    }
}

#[derive(suprnova::Data, Validate)]
struct BoundUpload {
    #[data(from_route_param("id"))]
    id: u64,
    #[serde(rename = "caption")]
    title: String,
    #[serde(skip_serializing)]
    photo: suprnova::UploadedFile,
}

mod protocol_routes {
    use super::*;
    use suprnova::{get, group, handler, post, routes};

    async fn plain(_request: Request) -> Response {
        BODY_CALLS.fetch_add(1, Ordering::SeqCst);
        suprnova::text("plain body")
    }

    #[handler]
    async fn empty() -> Response {
        BODY_CALLS.fetch_add(1, Ordering::SeqCst);
        suprnova::text("body")
    }
    #[handler]
    async fn checked(id: i64, _input: Checked) -> Response {
        BODY_CALLS.fetch_add(1, Ordering::SeqCst);
        Ok(HttpResponse::text(id.to_string()))
    }
    #[handler]
    async fn lookup(_category: Category, _input: Checked) -> Response {
        BODY_CALLS.fetch_add(1, Ordering::SeqCst);
        suprnova::text("bound body")
    }
    #[handler]
    async fn generic<T: FromRequest>(_input: T) -> Response {
        BODY_CALLS.fetch_add(1, Ordering::SeqCst);
        suprnova::text("generic body")
    }
    #[handler]
    async fn form(input: SignupRequest) -> Response {
        BODY_CALLS.fetch_add(1, Ordering::SeqCst);
        Ok(HttpResponse::text(input.email))
    }
    #[handler]
    async fn form_then_path(input: SignupRequest, id: i64, page: Option<u32>) -> Response {
        BODY_CALLS.fetch_add(1, Ordering::SeqCst);
        suprnova::text(format!("{} {id} {page:?}", input.email))
    }
    #[handler]
    async fn generic_form_then_path<T: FromRequest>(input: T, id: i64) -> Response {
        let _ = input;
        BODY_CALLS.fetch_add(1, Ordering::SeqCst);
        suprnova::text(id.to_string())
    }
    #[handler]
    async fn form_then_model(input: SignupRequest, member: PrecognitionMember) -> Response {
        BODY_CALLS.fetch_add(1, Ordering::SeqCst);
        suprnova::text(format!("{} {}", input.email, member.id))
    }
    #[handler]
    async fn bound(input: BoundForm) -> Response {
        BODY_CALLS.fetch_add(1, Ordering::SeqCst);
        Ok(HttpResponse::json(
            serde_json::json!({"id": input.id, "count": input.count, "tags": input.tags}),
        ))
    }
    #[handler]
    async fn upload(input: BoundUpload) -> Response {
        BODY_CALLS.fetch_add(1, Ordering::SeqCst);
        Ok(HttpResponse::json(
            serde_json::json!({"id":input.id,"caption":input.title,"size":input.photo.size}),
        ))
    }
    #[handler]
    async fn missing(_input: Missing) -> Response {
        BODY_CALLS.fetch_add(1, Ordering::SeqCst);
        suprnova::text("body")
    }
    #[handler]
    async fn denied(_input: DeniedForm) -> Response {
        BODY_CALLS.fetch_add(1, Ordering::SeqCst);
        suprnova::text("body")
    }
    struct Controller;
    impl Controller {
        #[handler(Self = Controller)]
        async fn store(_request: Request) -> Response {
            BODY_CALLS.fetch_add(1, Ordering::SeqCst);
            suprnova::text("body")
        }
    }
    routes! {
        post!("/empty", empty).middleware(Precognitive),
        post!("/checked/{id}", checked).middleware(Precognitive),
        post!("/lookup/{_category}", lookup).middleware(Precognitive),
        post!("/generic", generic::<Checked>).middleware(Precognitive),
        post!("/form", form).middleware(Precognitive),
        post!("/form-path/{id}/{page?}", form_then_path).middleware(Precognitive),
        post!("/generic-form-path/{id}", generic_form_then_path::<SignupRequest>).middleware(Precognitive),
        post!("/form-model/{member}", form_then_model).middleware(Precognitive),
        post!("/bound/{id}", bound).middleware(Precognitive),
        post!("/real-bound/{id}", bound),
        post!("/upload/{id}", upload).middleware(Precognitive),
        post!("/real-upload/{id}", upload),
        post!("/real", empty),
        post!("/plain", plain).middleware(Precognitive),
        post!("/real-plain", plain),
        post!("/real-form", form),
        post!("/missing", missing).middleware(Precognitive),
        post!("/denied", denied).middleware(Precognitive),
        group!("/group", {
            post!("/method", Controller::store),
            post!("/closure", |_request: Request| async {
                BODY_CALLS.fetch_add(1, Ordering::SeqCst);
                suprnova::text("closure body")
            }),
        }).middleware(Precognitive),
        get!("/unused", empty),
    }
}

fn assert_success(response: &hyper::Response<Bytes>) {
    assert_eq!(response.status(), 204);
    assert_eq!(response.headers().get("Precognition").unwrap(), "true");
    assert_eq!(
        response.headers().get("Precognition-Success").unwrap(),
        "true"
    );
    assert_eq!(response.headers().get("Vary").unwrap(), "Precognition");
    assert!(response.body().is_empty());
}

#[tokio::test]
async fn precognition_passing_form_runs_later_path_extractors_before_success() {
    let _counter_guard = COUNTERS.lock().await;
    BODY_CALLS.store(0, Ordering::SeqCst);
    for (path, status) in [
        ("/form-path/not-an-integer", 400),
        ("/form-path/7/not-an-integer", 400),
        ("/form-path/7", 204),
        ("/form-path/7/2", 204),
        ("/generic-form-path/not-an-integer", 400),
        ("/generic-form-path/7", 204),
    ] {
        let response = send(
            protocol_routes::register(),
            path,
            &[
                ("Precognition", "true"),
                ("Content-Type", "application/json"),
            ],
            r#"{"email":"ada@example.com","password":"longenough"}"#,
        )
        .await;
        assert_eq!(response.status(), status, "{path}");
        assert_eq!(response.headers().get("Precognition").unwrap(), "true");
        if status == 204 {
            assert_success(&response);
        } else {
            assert!(response.headers().get("Precognition-Success").is_none());
        }
    }
    assert_eq!(BODY_CALLS.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn precognition_form_then_path_real_requests_keep_values_and_error_order() {
    let _counter_guard = COUNTERS.lock().await;
    BODY_CALLS.store(0, Ordering::SeqCst);
    let valid = r#"{"email":"ada@example.com","password":"longenough"}"#;
    let invalid = r#"{"email":"bad","password":"longenough"}"#;
    for (path, body, status, expected) in [
        ("/form-path/not-an-integer", valid, 400, None),
        ("/form-path/7/not-an-integer", valid, 400, None),
        ("/form-path/not-an-integer", invalid, 422, None),
        ("/form-path/7", valid, 200, Some("ada@example.com 7 None")),
        (
            "/form-path/7/2",
            valid,
            200,
            Some("ada@example.com 7 Some(2)"),
        ),
        ("/generic-form-path/not-an-integer", valid, 400, None),
        ("/generic-form-path/not-an-integer", invalid, 422, None),
        ("/generic-form-path/7", valid, 200, Some("7")),
    ] {
        let response = send(
            protocol_routes::register(),
            path,
            &[("Content-Type", "application/json")],
            body,
        )
        .await;
        assert_eq!(response.status(), status, "{path}: {body}");
        assert!(response.headers().get("Precognition").is_none());
        if let Some(expected) = expected {
            assert_eq!(response.body(), expected);
        }
    }
    assert_eq!(BODY_CALLS.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn precognition_form_then_model_missing_binding_wins_in_both_modes() {
    use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};
    let _counter_guard = COUNTERS.lock().await;
    BODY_CALLS.store(0, Ordering::SeqCst);
    let _guard = suprnova::testing::TestContainer::fake();
    let database = Database::connect("sqlite::memory:").await.unwrap();
    for sql in [
        "CREATE TABLE precognition_members (id INTEGER PRIMARY KEY)",
        "INSERT INTO precognition_members (id) VALUES (7)",
    ] {
        database
            .execute_raw(Statement::from_string(DbBackend::Sqlite, sql))
            .await
            .unwrap();
    }
    suprnova::testing::TestContainer::singleton(suprnova::DbConnection::from_raw(database));
    for (marked, key, status) in [
        (true, "99", 404),
        (true, "7", 204),
        (false, "99", 404),
        (false, "7", 200),
    ] {
        let mut headers = vec![("Content-Type", "application/json")];
        if marked {
            headers.push(("Precognition", "true"));
        }
        let response = send(
            protocol_routes::register(),
            &format!("/form-model/{key}"),
            &headers,
            r#"{"email":"ada@example.com","password":"longenough"}"#,
        )
        .await;
        assert_eq!(response.status(), status);
        assert_eq!(response.headers().get("Precognition").is_some(), marked);
        if marked {
            assert_eq!(response.headers().get("Precognition").unwrap(), "true");
        }
        if status == 204 {
            assert_success(&response);
        } else if status == 200 {
            assert_eq!(response.body(), "ada@example.com 7");
        } else {
            assert!(response.headers().get("Precognition-Success").is_none());
        }
    }
    assert_eq!(BODY_CALLS.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn precognition_macro_extracts_before_skipping_body() {
    let _counter_guard = COUNTERS.lock().await;
    BODY_CALLS.store(0, Ordering::SeqCst);
    EXTRACT_CALLS.store(0, Ordering::SeqCst);
    let response = send(
        protocol_routes::register(),
        "/checked/7",
        &[("Precognition", "true")],
        "",
    )
    .await;
    assert_success(&response);
    assert_eq!(EXTRACT_CALLS.load(Ordering::SeqCst), 1);
    assert_eq!(BODY_CALLS.load(Ordering::SeqCst), 0);
    let failure = send(
        protocol_routes::register(),
        "/checked/no",
        &[("Precognition", "true")],
        "",
    )
    .await;
    assert_eq!(failure.status(), 400);
    assert_eq!(failure.headers().get("Precognition").unwrap(), "true");
    assert_eq!(EXTRACT_CALLS.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn precognition_resolves_route_bindings_and_generic_extractors_before_success() {
    let _counter_guard = COUNTERS.lock().await;
    BODY_CALLS.store(0, Ordering::SeqCst);
    EXTRACT_CALLS.store(0, Ordering::SeqCst);
    for path in ["/lookup/known", "/generic"] {
        assert_success(
            &send(
                protocol_routes::register(),
                path,
                &[("Precognition", "true")],
                "",
            )
            .await,
        );
    }
    assert_eq!(EXTRACT_CALLS.load(Ordering::SeqCst), 2);
    let missing = send(
        protocol_routes::register(),
        "/lookup/missing",
        &[("Precognition", "true")],
        "",
    )
    .await;
    assert_eq!(missing.status(), 404);
    assert_eq!(missing.headers().get("Precognition").unwrap(), "true");
    assert_eq!(EXTRACT_CALLS.load(Ordering::SeqCst), 2);
    assert_eq!(BODY_CALLS.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn precognition_macro_no_args_method_and_closure_skip_bodies() {
    let _counter_guard = COUNTERS.lock().await;
    BODY_CALLS.store(0, Ordering::SeqCst);
    for path in ["/empty", "/group/method", "/group/closure"] {
        assert_success(
            &send(
                protocol_routes::register(),
                path,
                &[("Precognition", "TrUe")],
                "",
            )
            .await,
        );
    }
    assert_eq!(BODY_CALLS.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn precognition_plain_function_skips_body_only_when_marked() {
    let _counter_guard = COUNTERS.lock().await;
    BODY_CALLS.store(0, Ordering::SeqCst);
    assert_success(
        &send(
            protocol_routes::register(),
            "/plain",
            &[("Precognition", "true")],
            "",
        )
        .await,
    );
    assert_eq!(BODY_CALLS.load(Ordering::SeqCst), 0);
    let real = send(
        protocol_routes::register(),
        "/real-plain",
        &[("Precognition", "true")],
        "",
    )
    .await;
    assert_eq!(real.status(), 200);
    assert_eq!(real.body(), "plain body");
    assert_eq!(BODY_CALLS.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn precognition_macro_and_form_extractor_share_success_headers() {
    let _counter_guard = COUNTERS.lock().await;
    BODY_CALLS.store(0, Ordering::SeqCst);
    for (path, body) in [
        ("/empty", ""),
        ("/form", r#"{"email":"a@b.com","password":"longenough"}"#),
    ] {
        assert_success(
            &send(
                protocol_routes::register(),
                path,
                &[
                    ("Precognition", "true"),
                    ("Content-Type", "application/json"),
                ],
                body,
            )
            .await,
        );
    }
    assert_eq!(BODY_CALLS.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn precognition_opt_in_route_without_middleware_runs_body() {
    let _counter_guard = COUNTERS.lock().await;
    BODY_CALLS.store(0, Ordering::SeqCst);
    let response = send(
        protocol_routes::register(),
        "/real",
        &[("Precognition", "true")],
        "",
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(response.body(), "body");
    assert!(response.headers().get("Precognition").is_none());
    assert!(response.headers().get("Vary").is_none());
    assert_eq!(BODY_CALLS.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn precognition_unmarked_form_validates_all_fields_and_runs_on_success() {
    let _counter_guard = COUNTERS.lock().await;
    BODY_CALLS.store(0, Ordering::SeqCst);
    let headers = [
        ("Precognition", "true"),
        ("Precognition-Validate-Only", "email"),
        ("Content-Type", "application/json"),
    ];
    let invalid = send(
        protocol_routes::register(),
        "/real-form",
        &headers,
        r#"{"email":"a@b.com","password":"short"}"#,
    )
    .await;
    assert_eq!(invalid.status(), 422);
    assert!(invalid.headers().get("Precognition").is_none());
    let body: serde_json::Value = serde_json::from_slice(invalid.body()).unwrap();
    assert!(body["errors"]["password"].is_array());
    let valid = send(
        protocol_routes::register(),
        "/real-form",
        &headers,
        r#"{"email":"a@b.com","password":"longenough"}"#,
    )
    .await;
    assert_eq!(valid.status(), 200);
    assert_eq!(BODY_CALLS.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn precognition_middleware_marks_403_and_404_extractor_responses() {
    let _counter_guard = COUNTERS.lock().await;
    BODY_CALLS.store(0, Ordering::SeqCst);
    for (path, status) in [("/missing", 404), ("/denied", 403)] {
        let response = send(
            protocol_routes::register(),
            path,
            &[
                ("Precognition", "true"),
                ("Content-Type", "application/json"),
            ],
            "{}",
        )
        .await;
        assert_eq!(response.status(), status);
        assert_eq!(response.headers().get("Precognition").unwrap(), "true");
        assert_eq!(response.headers().get("Vary").unwrap(), "Precognition");
        assert!(response.headers().get("Precognition-Success").is_none());
    }
    assert_eq!(BODY_CALLS.load(Ordering::SeqCst), 0);
}

#[derive(Clone)]
struct Answer {
    status: u16,
    vary: &'static str,
    error: bool,
}
#[async_trait]
impl Middleware for Answer {
    async fn handle(&self, request: Request, _next: Next) -> Response {
        let response = HttpResponse::json(serde_json::json!({"attempting":request.is_attempting_precognition(), "marked":request.is_precognitive()})).status(self.status).header("Vary", self.vary);
        if self.error {
            Err(response)
        } else {
            Ok(response)
        }
    }
}
fn answer_router(answer: Answer) -> Router {
    suprnova::post!("/answer", |_request: Request| async {
        suprnova::text("body")
    })
    .middleware(Precognitive)
    .middleware(answer)
    .register(Router::new())
}

#[tokio::test]
async fn precognition_vary_joins_existing_values_on_real_and_marked_responses() {
    for headers in [vec![], vec![("Precognition", "true")]] {
        let response = send(
            answer_router(Answer {
                status: 200,
                vary: "Accept, X-Inertia",
                error: false,
            }),
            "/answer",
            &headers,
            "",
        )
        .await;
        assert_eq!(
            response.headers().get("Vary").unwrap(),
            "Accept, X-Inertia, Precognition"
        );
    }
}
#[tokio::test]
async fn precognition_vary_does_not_duplicate_case_insensitive_token() {
    for vary in [
        "Precognition",
        "Accept, Precognition",
        "Accept, precognition",
    ] {
        let response = send(
            answer_router(Answer {
                status: 200,
                vary,
                error: false,
            }),
            "/answer",
            &[("Precognition", "true")],
            "",
        )
        .await;
        assert_eq!(response.headers().get("Vary").unwrap(), vary);
    }
}
#[tokio::test]
async fn precognition_later_middleware_failures_keep_envelope() {
    for status in [401, 403, 404, 409, 423] {
        for error in [false, true] {
            let response = send(
                answer_router(Answer {
                    status,
                    vary: "Accept",
                    error,
                }),
                "/answer",
                &[("Precognition", "true")],
                "",
            )
            .await;
            assert_eq!(response.status(), status);
            assert_eq!(response.headers().get("Precognition").unwrap(), "true");
            assert_eq!(
                response.headers().get("Vary").unwrap(),
                "Accept, Precognition"
            );
            let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
            assert_eq!(body["marked"], true);
        }
    }
}
#[tokio::test]
async fn precognition_request_predicates_distinguish_attempt_from_mark() {
    for value in ["true", "TRUE", "TrUe", "false", "1", "true ", ""] {
        let request = Request::for_test_with_headers("POST", "/answer", [("Precognition", value)]);
        let expected = value.eq_ignore_ascii_case("true");
        assert_eq!(request.is_attempting_precognition(), expected);
        assert!(!request.is_precognitive());
        let response = send(
            answer_router(Answer {
                status: 200,
                vary: "",
                error: false,
            }),
            "/answer",
            &[("Precognition", value)],
            "",
        )
        .await;
        // HTTP trims trailing whitespace before Request receives a header.
        let expected = value.trim().eq_ignore_ascii_case("true");
        let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        assert_eq!(body["attempting"], expected);
        assert_eq!(body["marked"], expected);
        assert_eq!(response.headers().get("Precognition").is_some(), expected);
    }
    assert!(!Request::for_test("POST", "/answer").is_attempting_precognition());
}

#[tokio::test]
async fn precognition_bound_form_parse_failure_uses_shared_field_errors() {
    for (content_type, body) in [
        ("application/json", r#"{"email":"a@b.com","count":"bad"}"#),
        (
            "application/x-www-form-urlencoded",
            "email=a%40b.com&count=bad",
        ),
    ] {
        let response = send(
            protocol_routes::register(),
            "/bound/7",
            &[("Precognition", "true"), ("Content-Type", content_type)],
            body,
        )
        .await;
        assert_eq!(response.status(), 422);
        assert_eq!(response.headers().get("Precognition").unwrap(), "true");
        let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        assert!(body["errors"]["count"].is_array(), "{body}");
        assert_eq!(body["message"], body["errors"]["count"][0]);
    }
}
#[tokio::test]
async fn precognition_bound_form_rejects_unsupported_and_missing_content_types() {
    for content_type in [None, Some("text/plain"), Some("application/xml")] {
        let mut headers = vec![("Precognition", "true")];
        if let Some(content_type) = content_type {
            headers.push(("Content-Type", content_type));
        }
        let response = send(
            protocol_routes::register(),
            "/bound/7",
            &headers,
            r#"{"email":"a@b.com","count":1}"#,
        )
        .await;
        assert_eq!(response.status(), 415);
        assert_eq!(response.headers().get("Precognition").unwrap(), "true");
    }
}
#[tokio::test]
async fn precognition_bound_form_path_wins_and_nested_form_still_parses() {
    let _counter_guard = COUNTERS.lock().await;
    BODY_CALLS.store(0, Ordering::SeqCst);
    for (content_type, body) in [
        (
            "application/json",
            r#"{"id":999,"email":"a@b.com","count":2,"tags":["a","b"]}"#,
        ),
        (
            "Application/X-WWW-Form-Urlencoded; charset=utf-8",
            "id=999&email=a%40b.com&count=2&tags[]=a&tags[]=b",
        ),
    ] {
        let response = send(
            protocol_routes::register(),
            "/bound/7",
            &[("Precognition", "true"), ("Content-Type", content_type)],
            body,
        )
        .await;
        assert_success(&response);
        let real = send(
            protocol_routes::register(),
            "/real-bound/7",
            &[("Precognition", "true"), ("Content-Type", content_type)],
            body,
        )
        .await;
        assert_eq!(real.status(), 200);
        let body: serde_json::Value = serde_json::from_slice(real.body()).unwrap();
        assert_eq!(body["id"], 7);
        assert_eq!(body["count"], 2);
        assert_eq!(body["tags"], serde_json::json!(["a", "b"]));
    }
    assert_eq!(BODY_CALLS.load(Ordering::SeqCst), 2);
}
#[tokio::test]
async fn precognition_bound_form_malformed_json_is_marked_422() {
    for body in ["{", "[]", "null"] {
        let response = send(
            protocol_routes::register(),
            "/bound/7",
            &[
                ("Precognition", "true"),
                ("Content-Type", "application/json"),
            ],
            body,
        )
        .await;
        assert_eq!(response.status(), 422);
        assert_eq!(response.headers().get("Precognition").unwrap(), "true");
    }
}

#[derive(Clone)]
struct Precheck {
    bail: bool,
    alternate: bool,
}
#[async_trait]
impl Middleware for Precheck {
    async fn handle(&self, request: Request, _next: Next) -> Response {
        Precognition::precognitive(&request, || async {
            if self.bail {
                let default = Err(HttpResponse::text("default").status(409));
                return Err(if self.alternate {
                    Bail::precognition(default, Err(HttpResponse::text("validation").status(423)))
                } else {
                    Bail::with(default)
                });
            }
            Ok(suprnova::text("value"))
        })
        .await
    }
}
fn helper_router(bail: bool, alternate: bool) -> Router {
    suprnova::post!("/check", |_request: Request| async {
        suprnova::text("body")
    })
    .middleware(Precognitive)
    .middleware(Precheck { bail, alternate })
    .register(Router::new())
}
#[tokio::test]
async fn precognition_helper_returns_value_or_success_and_selects_bail() {
    for (marked, bail, status, body) in [
        (false, false, 200, "value"),
        (true, false, 204, ""),
        (false, true, 409, "default"),
        (true, true, 423, "validation"),
    ] {
        let headers = if marked {
            vec![("Precognition", "true")]
        } else {
            vec![]
        };
        let response = send(helper_router(bail, true), "/check", &headers, "").await;
        assert_eq!(response.status(), status);
        assert_eq!(response.body(), body);
        if marked && !bail {
            assert_success(&response);
        }
    }
}
#[tokio::test]
async fn precognition_helper_common_bail_is_kept_for_both_request_modes() {
    for marked in [false, true] {
        let headers = if marked {
            vec![("Precognition", "true")]
        } else {
            vec![]
        };
        let response = send(helper_router(true, false), "/check", &headers, "").await;
        assert_eq!(response.status(), 409);
        assert_eq!(response.body(), "default");
    }
}

#[test]
fn precognition_validation_empty_bag_keeps_fallback_and_single_field_uses_message() {
    let response: HttpResponse =
        FrameworkError::Validation(suprnova::ValidationErrors::new()).into();
    let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
    assert_eq!(body["message"], "The given data was invalid.");
    let response: HttpResponse = FrameworkError::ValidationError {
        field: "email".into(),
        message: "Invalid email".into(),
    }
    .into();
    let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
    assert_eq!(body["message"], "Invalid email");
}

fn upload_body(with_file: bool) -> String {
    let mut body = String::from(
        "--BOUNDARY\r\nContent-Disposition: form-data; name=\"id\"\r\n\r\n999\r\n--BOUNDARY\r\nContent-Disposition: form-data; name=\"caption\"\r\n\r\nHoliday\r\n",
    );
    if with_file {
        body.push_str("--BOUNDARY\r\nContent-Disposition: form-data; name=\"photo\"; filename=\"a.txt\"\r\nContent-Type: text/plain\r\n\r\nhello\r\n");
    }
    body.push_str("--BOUNDARY--\r\n");
    body
}

#[tokio::test]
async fn precognition_bound_multipart_keeps_files_and_path_values() {
    let _counter_guard = COUNTERS.lock().await;
    BODY_CALLS.store(0, Ordering::SeqCst);
    let headers = [
        ("Precognition", "true"),
        ("Content-Type", "multipart/form-data; boundary=BOUNDARY"),
    ];
    assert_success(
        &send(
            protocol_routes::register(),
            "/upload/7",
            &headers,
            &upload_body(true),
        )
        .await,
    );
    assert_eq!(BODY_CALLS.load(Ordering::SeqCst), 0);
    let real = send(
        protocol_routes::register(),
        "/real-upload/7",
        &headers,
        &upload_body(true),
    )
    .await;
    assert_eq!(real.status(), 200);
    let body: serde_json::Value = serde_json::from_slice(real.body()).unwrap();
    assert_eq!(
        body,
        serde_json::json!({"id":7,"caption":"Holiday","size":5})
    );
    assert_eq!(BODY_CALLS.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn precognition_bound_multipart_missing_file_is_a_shared_field_failure() {
    let response = send(
        protocol_routes::register(),
        "/upload/7",
        &[
            ("Precognition", "true"),
            ("Content-Type", "multipart/form-data; boundary=BOUNDARY"),
        ],
        &upload_body(false),
    )
    .await;
    assert_eq!(response.status(), 422);
    assert_eq!(response.headers().get("Precognition").unwrap(), "true");
    let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
    assert!(body["errors"]["photo"].is_array(), "{body}");
    assert_eq!(body["message"], body["errors"]["photo"][0]);
}

#[derive(Clone)]
struct MultipleVary {
    present: bool,
}
#[async_trait]
impl Middleware for MultipleVary {
    async fn handle(&self, _request: Request, _next: Next) -> Response {
        Ok(HttpResponse::text("answer")
            .header("Vary", "Accept")
            .header(
                "Vary",
                if self.present {
                    "X-Inertia, precognition"
                } else {
                    "X-Inertia"
                },
            ))
    }
}

#[tokio::test]
async fn precognition_vary_checks_all_lines_and_nested_middleware_is_idempotent() {
    for present in [false, true] {
        let router = suprnova::post!("/vary", |_request: Request| async {
            suprnova::text("body")
        })
        .middleware(Precognitive)
        .middleware(Precognitive)
        .middleware(MultipleVary { present })
        .register(Router::new());
        let response = send(router, "/vary", &[("Precognition", "true")], "").await;
        let values: Vec<&str> = response
            .headers()
            .get_all("Vary")
            .iter()
            .map(|value| value.to_str().unwrap())
            .collect();
        assert_eq!(
            values,
            if present {
                vec!["Accept", "X-Inertia, precognition"]
            } else {
                vec!["Accept, X-Inertia, Precognition"]
            }
        );
        assert_eq!(response.headers().get_all("Precognition").iter().count(), 1);
    }
}
