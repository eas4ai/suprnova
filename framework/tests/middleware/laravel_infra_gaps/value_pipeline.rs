//! `Pipeline::of(value)`: a pipeline that sends any value through steps,
//! and the HTTP pipeline's `pipe_all`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serial_test::serial;
use suprnova::http::text;
use suprnova::middleware::{PipelineFuture, PipelineNext, PipelineStep};
use suprnova::testing::TestDatabase;
use suprnova::{DB, FrameworkError, Middleware, Next, Pipeline, Request, Response, async_trait};

fn add_one(value: i32, next: PipelineNext<i32>) -> PipelineFuture<i32> {
    next(value + 1)
}

fn double(value: i32, next: PipelineNext<i32>) -> PipelineFuture<i32> {
    next(value * 2)
}

#[tokio::test]
async fn steps_run_in_order_and_then_return_answers_the_value() {
    let answer = Pipeline::of(5)
        .through([add_one, double])
        .then_return()
        .await;
    assert_eq!(answer.ok(), Some(12));
}

#[tokio::test]
async fn async_closures_and_async_fns_are_steps() {
    async fn triple(value: i32, next: PipelineNext<i32>) -> Result<i32, FrameworkError> {
        next(value * 3).await
    }
    let answer = Pipeline::of(1)
        .pipe(|value: i32, next: PipelineNext<i32>| async move { next(value + 10).await })
        .pipe(triple)
        .then_return()
        .await
        .unwrap();
    assert_eq!(answer, 33);
}

/// A step written as a type, recording its label.
struct Record {
    label: &'static str,
    ran: Arc<Mutex<Vec<&'static str>>>,
}

impl PipelineStep<Vec<&'static str>> for Record {
    fn handle(
        &self,
        mut value: Vec<&'static str>,
        next: PipelineNext<Vec<&'static str>>,
    ) -> PipelineFuture<Vec<&'static str>> {
        self.ran.lock().unwrap().push(self.label);
        value.push(self.label);
        next(value)
    }
}

fn record(label: &'static str, ran: &Arc<Mutex<Vec<&'static str>>>) -> Record {
    Record {
        label,
        ran: ran.clone(),
    }
}

#[tokio::test]
async fn through_then_pipe_all_runs_a_then_b_then_c() {
    let ran = Arc::new(Mutex::new(Vec::new()));
    let value = Pipeline::of(Vec::new())
        .through([record("a", &ran)])
        .pipe_all([record("b", &ran), record("c", &ran)])
        .then_return()
        .await
        .unwrap();
    assert_eq!(value, ["a", "b", "c"]);
    assert_eq!(*ran.lock().unwrap(), ["a", "b", "c"]);
}

#[tokio::test]
async fn through_replaces_the_steps_and_pipe_appends() {
    let ran = Arc::new(Mutex::new(Vec::new()));
    let value = Pipeline::of(Vec::new())
        .pipe(record("dropped", &ran))
        .through([record("a", &ran)])
        .pipe(record("b", &ran))
        .then_return()
        .await
        .unwrap();
    assert_eq!(value, ["a", "b"]);
}

#[tokio::test]
async fn then_runs_the_steps_around_the_destination_and_answers_its_result() {
    let ran = Arc::new(Mutex::new(Vec::new()));
    let after = ran.clone();
    let answer: String = Pipeline::of(4)
        .pipe(move |value: i32, next: PipelineNext<i32, String>| {
            let after = after.clone();
            async move {
                let inner = next(value + 1).await?;
                after.lock().unwrap().push("after the destination");
                Ok(format!("<{inner}>"))
            }
        })
        .then(|value| async move { Ok(format!("{value}!")) })
        .await
        .unwrap();
    assert_eq!(answer, "<5!>");
    assert_eq!(*ran.lock().unwrap(), ["after the destination"]);
}

#[tokio::test]
async fn a_step_that_answers_an_error_stops_the_pipeline_and_finally_still_runs() {
    let later_ran = Arc::new(AtomicBool::new(false));
    let finally_ran = Arc::new(AtomicBool::new(false));
    let later = later_ran.clone();
    let finished = finally_ran.clone();
    let outcome = Pipeline::of(1)
        .pipe(|_value: i32, _next: PipelineNext<i32>| async move {
            Err::<i32, _>(FrameworkError::internal("refused"))
        })
        .pipe(move |value: i32, next: PipelineNext<i32>| {
            let later = later.clone();
            async move {
                later.store(true, Ordering::SeqCst);
                next(value).await
            }
        })
        .finally(move || finished.store(true, Ordering::SeqCst))
        .then_return()
        .await;
    assert!(outcome.is_err());
    assert!(!later_ran.load(Ordering::SeqCst), "a later step ran");
    assert!(finally_ran.load(Ordering::SeqCst), "finally was skipped");
}

#[tokio::test]
async fn finally_runs_when_the_pipeline_succeeds() {
    let finally_ran = Arc::new(AtomicBool::new(false));
    let finished = finally_ran.clone();
    let answer = Pipeline::of(2)
        .pipe(add_one)
        .finally(move || finished.store(true, Ordering::SeqCst))
        .then_return()
        .await
        .unwrap();
    assert_eq!(answer, 3);
    assert!(finally_ran.load(Ordering::SeqCst));
}

#[tokio::test]
async fn a_pipeline_without_steps_answers_the_value() {
    assert_eq!(Pipeline::of("plain").then_return().await.unwrap(), "plain");
}

// --- within_transaction ----------------------------------------------------

async fn notes_database() -> TestDatabase {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    db.execute_unprepared(
        "CREATE TABLE notes (id INTEGER PRIMARY KEY AUTOINCREMENT, body TEXT NOT NULL)",
    )
    .await
    .unwrap();
    db
}

async fn note_count() -> i64 {
    DB::scalar("SELECT COUNT(*) FROM notes", Vec::<sea_orm::Value>::new())
        .await
        .unwrap()
}

fn write_note(value: i32, next: PipelineNext<i32>) -> PipelineFuture<i32> {
    Box::pin(async move {
        DB::insert(
            "INSERT INTO notes (body) VALUES ('written by a step')",
            Vec::<sea_orm::Value>::new(),
        )
        .await?;
        next(value).await
    })
}

fn fail(_value: i32, _next: PipelineNext<i32>) -> PipelineFuture<i32> {
    Box::pin(async { Err(FrameworkError::internal("a later step failed")) })
}

#[tokio::test]
#[serial]
async fn within_transaction_rolls_back_a_write_when_a_later_step_fails() {
    let _db = notes_database().await;
    let outcome = Pipeline::of(1)
        .through([write_note, fail])
        .within_transaction()
        .then_return()
        .await;
    assert!(outcome.is_err());
    assert_eq!(
        note_count().await,
        0,
        "the failed pipeline's write was kept"
    );
}

#[tokio::test]
#[serial]
async fn within_transaction_commits_when_every_step_passes() {
    let _db = notes_database().await;
    let outcome = Pipeline::of(1)
        .pipe(write_note)
        .within_transaction()
        .then_return()
        .await;
    assert_eq!(outcome.unwrap(), 1);
    assert_eq!(note_count().await, 1);
}

#[tokio::test]
#[serial]
async fn without_a_transaction_the_write_of_a_failed_pipeline_stays() {
    let _db = notes_database().await;
    let outcome = Pipeline::of(1)
        .through([write_note, fail])
        .then_return()
        .await;
    assert!(outcome.is_err());
    assert_eq!(note_count().await, 1);
}

// --- The HTTP pipeline's pipe_all ------------------------------------------

static HTTP_RAN: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

macro_rules! recording {
    ($name:ident, $label:literal) => {
        struct $name;

        #[async_trait]
        impl Middleware for $name {
            async fn handle(&self, request: Request, next: Next) -> Response {
                HTTP_RAN.lock().unwrap().push($label);
                next(request).await
            }
        }
    };
}

recording!(First, "first");
recording!(Second, "second");

/// Serve one request whose handler sends it through `pipeline`, and answer
/// the status of the pipeline's response.
async fn through_http(pipeline: Pipeline) -> u16 {
    use std::convert::Infallible;

    use http_body_util::{BodyExt, Full};
    use hyper::body::{Bytes, Incoming};
    use hyper::service::service_fn;
    use hyper_util::rt::TokioIo;

    let pipeline = Arc::new(Mutex::new(Some(pipeline)));
    let (client, server) = tokio::io::duplex(64 * 1024);
    tokio::spawn(async move {
        let service = service_fn(move |hyper_request: hyper::Request<Incoming>| {
            let pipeline = pipeline.lock().unwrap().take();
            async move {
                let request = Request::new(hyper_request);
                let status = match pipeline {
                    Some(pipeline) => match pipeline
                        .send(request)
                        .then(|_request| async { text("done") })
                        .await
                    {
                        Ok(_) => 200,
                        Err(_) => 500,
                    },
                    None => 500,
                };
                Ok::<_, Infallible>(
                    hyper::Response::builder()
                        .status(status)
                        .body(Full::new(Bytes::new()))
                        .unwrap(),
                )
            }
        });
        let _ = hyper::server::conn::http1::Builder::new()
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
    let request = hyper::Request::builder()
        .uri("/")
        .header("host", "localhost")
        .body(Full::new(Bytes::new()))
        .unwrap();
    let response = sender.send_request(request).await.unwrap();
    let status = response.status().as_u16();
    let _ = response.into_body().collect().await;
    status
}

#[tokio::test]
#[serial]
async fn the_http_pipeline_pipe_all_appends_after_what_through_set() {
    HTTP_RAN.lock().unwrap().clear();
    let pipeline = Pipeline::new().through([First]).pipe_all([Second, Second]);
    assert_eq!(pipeline.len(), 3);
    assert_eq!(through_http(pipeline).await, 200);
    assert_eq!(*HTTP_RAN.lock().unwrap(), ["first", "second", "second"]);
}

#[tokio::test]
#[serial]
async fn the_http_pipeline_through_after_pipe_all_replaces_everything() {
    let pipeline = Pipeline::new().pipe_all([Second, Second]).through([First]);
    assert_eq!(pipeline.len(), 1);
}
