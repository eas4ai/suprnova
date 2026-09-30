//! `#[data(after_validation = "fn")]` and `#[data(after_validation_async
//! = "fn")]`: the derive owns a Data Object's `FormRequest` impl, so these
//! are how its `validate!` rules and its database rules (`Exists`,
//! `Unique`) take part in request validation. Every test sends a real
//! HTTP request and extracts the Data Object from it, on both of the
//! derive's `FormRequest` paths: the default one, and the inlined one a
//! route-parameter field selects.

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::{Arc, Mutex};

use http_body_util::Full;
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};
use suprnova::error::FrameworkError;
use suprnova::rules::{
    Accepted, After, AlphaDash, DateBound, DateFormat, Digits, ExcludeUnless, Missing, RequiredIf,
};
use suprnova::testing::TestContainer;
use suprnova::{
    DbConnection, Exists, FormContext, FormRequest, HttpResponse, Request, ValidationErrors,
    validate,
};

// ---- Data Objects under test ----

/// Sync rules only, on the derive's default `FormRequest` path.
#[derive(Debug, suprnova::Data, validator::Validate)]
#[data(after_validation = "booking_rules")]
struct BookingDto {
    pub starts_on: String,
    pub ends_on: String,
    pub payment: String,
    pub card_number: Option<String>,
    pub terms: bool,
    pub promo: Option<String>,
}

fn booking_rules(dto: &BookingDto) -> Result<(), ValidationErrors> {
    let form: FormContext = [
        ("starts_on".to_string(), dto.starts_on.clone()),
        ("payment".to_string(), dto.payment.clone()),
    ]
    .into();
    validate! { dto =>
        starts_on => DateFormat(&["%Y-%m-%d"]);
        ends_on => DateFormat(&["%Y-%m-%d"]), After::new(DateBound::Field("starts_on")) => with form;
        card_number ?=> RequiredIf { other: "payment", value: "card" } => with form,
            ExcludeUnless { other: "payment", value: "card" } => with form;
        card_number ?: Digits(16);
        terms => Accepted;
        promo ?: Missing;
    }
}

/// A database rule, on the default path.
#[derive(Debug, suprnova::Data, validator::Validate)]
#[data(after_validation_async = "assignment_rules")]
struct AssignTagsDto {
    pub team_id: i64,
    pub tag_ids: Vec<i64>,
}

async fn assignment_rules(dto: &AssignTagsDto) -> Result<(), ValidationErrors> {
    let mut errs = ValidationErrors::new();
    Exists::new("teams", "id")
        .check_value(dto.team_id, &mut errs, "team_id")
        .await;
    Exists::new("tags", "id")
        .where_eq("team_id", dto.team_id)
        .check_each(&dto.tag_ids, &mut errs, "tag_ids")
        .await;
    errs.into_result()
}

/// Both hooks, on the inlined path a route-parameter field selects. That
/// path used to run the sync hook only.
#[derive(Debug, suprnova::Data, validator::Validate)]
#[data(
    after_validation = "rename_rules",
    after_validation_async = "rename_team_rules"
)]
struct RenameTagDto {
    #[data(from_route_param("team"))]
    pub team_id: i64,
    pub slug: String,
}

fn rename_rules(dto: &RenameTagDto) -> Result<(), ValidationErrors> {
    validate! { dto =>
        slug => AlphaDash;
    }
}

async fn rename_team_rules(dto: &RenameTagDto) -> Result<(), ValidationErrors> {
    let mut errs = ValidationErrors::new();
    Exists::new("teams", "id")
        .check_value(dto.team_id, &mut errs, "team_id")
        .await;
    errs.into_result()
}

// ---- Harness ----

/// Team 7 owns tags 1 and 2; team 8 owns tag 3. `TestContainer` is
/// thread-local and `#[tokio::test]` runs the server task on the test's
/// thread, so the extraction below sees this database.
async fn install_teams_db() {
    let raw = Database::connect("sqlite::memory:").await.unwrap();
    for sql in [
        "CREATE TABLE teams (id INTEGER PRIMARY KEY)",
        "CREATE TABLE tags (id INTEGER PRIMARY KEY, team_id INTEGER NOT NULL)",
        "INSERT INTO teams (id) VALUES (7), (8)",
        "INSERT INTO tags (id, team_id) VALUES (1, 7), (2, 7), (3, 8)",
    ] {
        raw.execute_raw(Statement::from_string(DbBackend::Sqlite, sql.to_string()))
            .await
            .unwrap();
    }
    TestContainer::singleton(DbConnection::from_raw(raw));
}

/// How a request asks for Precognition: not at all, for every field, or
/// for the fields a `Precognition-Validate-Only` header names.
#[derive(Clone, Copy)]
enum Precognition {
    Off,
    All,
    Only(&'static str),
}

/// Send one JSON request and return what `T::extract` made of it. The
/// server task hands the result back over a channel, so nothing waits on
/// a timer.
async fn extract<T>(
    route_params: &[(&str, &str)],
    body: serde_json::Value,
    precognition: Precognition,
) -> Result<T, FrameworkError>
where
    T: FormRequest + Send + 'static,
{
    let params: HashMap<String, String> = route_params
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (sent, received) = tokio::sync::oneshot::channel();
    let sent = Arc::new(Mutex::new(Some(sent)));

    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let svc = service_fn(move |hyper_req: hyper::Request<hyper::body::Incoming>| {
            let sent = sent.clone();
            let params = params.clone();
            async move {
                let result = T::extract(Request::new(hyper_req).with_params(params)).await;
                if let Some(sent) = sent.lock().unwrap().take() {
                    let _ = sent.send(result);
                }
                Ok::<_, Infallible>(HttpResponse::text("ok").into_hyper())
            }
        });
        let _ = http1::Builder::new()
            .serve_connection(TokioIo::new(stream), svc)
            .await;
    });

    let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let (mut sender, conn) =
        hyper::client::conn::http1::handshake::<_, Full<Bytes>>(TokioIo::new(stream))
            .await
            .unwrap();
    tokio::spawn(conn);
    let body = serde_json::to_vec(&body).unwrap();
    let mut request = hyper::Request::builder()
        .method("POST")
        .uri("http://localhost/submit")
        .header("content-type", "application/json")
        .header("content-length", body.len());
    match precognition {
        Precognition::Off => {}
        Precognition::All => request = request.header("Precognition", "true"),
        Precognition::Only(fields) => {
            request = request
                .header("Precognition", "true")
                .header("Precognition-Validate-Only", fields);
        }
    }
    sender
        .send_request(request.body(Full::new(Bytes::from(body))).unwrap())
        .await
        .unwrap();
    received.await.expect("the server extracted the request")
}

fn failed_keys(result: Result<impl std::fmt::Debug, FrameworkError>) -> Vec<String> {
    let errs = match result {
        Err(FrameworkError::Validation(errs)) => errs,
        other => panic!("expected a validation failure, got {other:?}"),
    };
    let mut keys: Vec<String> = errs.errors.keys().cloned().collect();
    keys.sort();
    keys
}

fn booking(changes: serde_json::Value) -> serde_json::Value {
    let mut body = serde_json::json!({
        "starts_on": "2026-10-01",
        "ends_on": "2026-10-03",
        "payment": "card",
        "card_number": "4242424242424242",
        "terms": true,
    });
    for (key, value) in changes.as_object().unwrap() {
        body[key] = value.clone();
    }
    body
}

// ---- Tests ----

#[tokio::test]
async fn a_data_object_runs_its_sync_rules() {
    let dto = extract::<BookingDto>(&[], booking(serde_json::json!({})), Precognition::Off)
        .await
        .expect("a valid booking extracts");
    assert_eq!(dto.ends_on, "2026-10-03");

    let result = extract::<BookingDto>(
        &[],
        booking(serde_json::json!({
            "ends_on": "2026-09-30",
            "card_number": "4242",
            "terms": false,
            "promo": "SAVE10",
        })),
        Precognition::Off,
    )
    .await;
    assert_eq!(
        failed_keys(result),
        ["card_number", "ends_on", "promo", "terms"]
    );
}

#[tokio::test]
async fn an_excluded_field_is_not_required() {
    let cash = booking(serde_json::json!({ "payment": "cash", "card_number": null }));
    assert!(
        extract::<BookingDto>(&[], cash, Precognition::Off)
            .await
            .is_ok()
    );

    let card = booking(serde_json::json!({ "card_number": null }));
    assert_eq!(
        failed_keys(extract::<BookingDto>(&[], card, Precognition::Off).await),
        ["card_number"]
    );
}

#[tokio::test]
async fn a_data_object_runs_its_database_rules() {
    let _guard = TestContainer::fake();
    install_teams_db().await;

    let ok = serde_json::json!({ "team_id": 7, "tag_ids": [1, 2] });
    let dto = extract::<AssignTagsDto>(&[], ok, Precognition::Off)
        .await
        .expect("team 7 owns tags 1 and 2");
    assert_eq!(dto.tag_ids, [1, 2]);

    let foreign = serde_json::json!({ "team_id": 7, "tag_ids": [2, 3, 4] });
    assert_eq!(
        failed_keys(extract::<AssignTagsDto>(&[], foreign, Precognition::Off).await),
        ["tag_ids.1", "tag_ids.2"]
    );

    let no_team = serde_json::json!({ "team_id": 99, "tag_ids": [] });
    assert_eq!(
        failed_keys(extract::<AssignTagsDto>(&[], no_team, Precognition::Off).await),
        ["team_id"]
    );
}

#[tokio::test]
async fn the_route_parameter_path_runs_the_async_hook() {
    let _guard = TestContainer::fake();
    install_teams_db().await;

    let dto = extract::<RenameTagDto>(
        &[("team", "7")],
        serde_json::json!({ "slug": "rust-2" }),
        Precognition::Off,
    )
    .await
    .expect("team 7 exists");
    assert_eq!(dto.team_id, 7);

    let result = extract::<RenameTagDto>(
        &[("team", "99")],
        serde_json::json!({ "slug": "rust-2" }),
        Precognition::Off,
    )
    .await;
    assert_eq!(failed_keys(result), ["team_id"]);
}

/// The stages bail at the first failure, so a bad slug never reaches the
/// database rule and only its own error comes back.
#[tokio::test]
async fn a_failing_sync_hook_keeps_the_database_rule_from_running() {
    let _guard = TestContainer::fake();
    install_teams_db().await;

    let result = extract::<RenameTagDto>(
        &[("team", "99")],
        serde_json::json!({ "slug": "not a slug!" }),
        Precognition::Off,
    )
    .await;
    assert_eq!(failed_keys(result), ["slug"]);
}

#[tokio::test]
async fn precognition_reports_the_async_hook_on_the_route_parameter_path() {
    let _guard = TestContainer::fake();
    install_teams_db().await;

    let result = extract::<RenameTagDto>(
        &[("team", "99")],
        serde_json::json!({ "slug": "rust-2" }),
        Precognition::All,
    )
    .await;
    match result {
        Err(FrameworkError::PrecognitionFailure(errs)) => {
            assert!(errs.errors.contains_key("team_id"), "got {errs:?}");
        }
        other => panic!("expected a Precognition failure, got {other:?}"),
    }

    let passing = extract::<RenameTagDto>(
        &[("team", "7")],
        serde_json::json!({ "slug": "rust-2" }),
        Precognition::All,
    )
    .await;
    assert!(
        matches!(passing, Err(FrameworkError::PrecognitionSuccess)),
        "got {passing:?}"
    );
}

fn precognition_keys(result: Result<impl std::fmt::Debug, FrameworkError>) -> Vec<String> {
    let errs = match result {
        Err(FrameworkError::PrecognitionFailure(errs)) => errs,
        other => panic!("expected a Precognition failure, got {other:?}"),
    };
    let mut keys: Vec<String> = errs.errors.keys().cloned().collect();
    keys.sort();
    keys
}

#[tokio::test]
async fn precognition_runs_the_async_hook_on_the_default_path() {
    let _guard = TestContainer::fake();
    install_teams_db().await;

    let foreign = serde_json::json!({ "team_id": 7, "tag_ids": [2, 3] });
    assert_eq!(
        precognition_keys(extract::<AssignTagsDto>(&[], foreign, Precognition::All).await),
        ["tag_ids.1"]
    );
}

/// An array rule reports each element under its index. Asking about the
/// array, or about its elements the way Laravel names the rule, keeps
/// those errors; asking about another field drops them.
#[tokio::test]
async fn validate_only_keeps_the_errors_of_an_arrays_elements() {
    let _guard = TestContainer::fake();
    install_teams_db().await;

    let body = || serde_json::json!({ "team_id": 7, "tag_ids": [1, 3, 4] });
    for fields in ["tag_ids", "tag_ids.*", "team_id,tag_ids"] {
        assert_eq!(
            precognition_keys(
                extract::<AssignTagsDto>(&[], body(), Precognition::Only(fields)).await
            ),
            ["tag_ids.1", "tag_ids.2"],
            "{fields}"
        );
    }
    let other = extract::<AssignTagsDto>(&[], body(), Precognition::Only("team_id")).await;
    assert!(
        matches!(other, Err(FrameworkError::PrecognitionSuccess)),
        "got {other:?}"
    );
}
