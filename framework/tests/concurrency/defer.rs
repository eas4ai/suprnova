//! PAR-190: `Concurrency::defer` runs its tasks after the response has been
//! sent, and outside a request when its handle is awaited or dropped.
//!
//! A request is served over an in-memory connection the way the server
//! serves one: `handle_request` inside `after_response_connection`, so the
//! response's flush is what releases the deferred tasks.

use crate::support::{exists, wait_until};
use crate::tasks::{FailWith, Touch, TouchAfter};
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use serial_test::serial;
use std::convert::Infallible;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use suprnova::concurrency::Task;
use suprnova::{
    Concurrency, ConcurrentTask, HttpResponse, MiddlewareRegistry, Router,
    after_response_connection,
};

/// A response read by the client, with the connection that served it kept
/// open: closing it would cut off the after-response work it still owns.
struct Served {
    status: u16,
    body: String,
    took: Duration,
    _connection: Connection,
}

/// The tasks of one in-memory connection, aborted when the test is done.
struct Connection {
    server: tokio::task::JoinHandle<Result<(), hyper::Error>>,
    client: tokio::task::JoinHandle<Result<(), hyper::Error>>,
    _sender: hyper::client::conn::http1::SendRequest<Full<Bytes>>,
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.server.abort();
        self.client.abort();
    }
}

/// Serve one GET of `uri` through `router` and read the whole response.
/// With `writer`, the connection is scoped the way the server scopes one,
/// so the response's flush releases deferred work; without it, there is no
/// response writer in reach.
async fn serve(router: Router, uri: &str, writer: bool) -> Served {
    let router = Arc::new(router);
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    let service = hyper::service::service_fn(move |request| {
        let router = router.clone();
        async move {
            Ok::<_, Infallible>(
                suprnova::server::handle_request(
                    router,
                    Arc::new(MiddlewareRegistry::new()),
                    request,
                )
                .await,
            )
        }
    });
    let server = if writer {
        tokio::spawn(after_response_connection(server_io, move |io| async move {
            hyper::server::conn::http1::Builder::new()
                .serve_connection(hyper_util::rt::TokioIo::new(io), service)
                .await
        }))
    } else {
        tokio::spawn(
            hyper::server::conn::http1::Builder::new()
                .serve_connection(hyper_util::rt::TokioIo::new(server_io), service),
        )
    };
    let (mut sender, connection) = hyper::client::conn::http1::handshake::<_, Full<Bytes>>(
        hyper_util::rt::TokioIo::new(client_io),
    )
    .await
    .expect("handshake");
    let client = tokio::spawn(connection);
    let request = hyper::Request::builder()
        .uri(uri)
        .header("Host", "localhost")
        .body(Full::new(Bytes::new()))
        .expect("request");
    let started = Instant::now();
    let response = sender.send_request(request).await.expect("a response");
    let status = response.status().as_u16();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("the body")
        .to_bytes();
    Served {
        status,
        body: String::from_utf8_lossy(&body).into_owned(),
        took: started.elapsed(),
        _connection: Connection {
            server,
            client,
            _sender: sender,
        },
    }
}

fn touch(path: &Path, after_millis: u64) -> Task {
    Task::new::<TouchAfter>(Touch {
        path: path.to_path_buf(),
        after_millis,
    })
}

/// A router whose `/defer` handler defers `tasks()` on `driver`, or with
/// `Concurrency::defer` on the default driver when it is `None`, and
/// records whether they wait for the response.
fn deferring_router(
    driver: Option<&'static str>,
    tasks: impl Fn() -> Vec<Task> + Send + Sync + 'static,
    after_response: Arc<AtomicBool>,
) -> Router {
    let tasks = Arc::new(tasks);
    Router::new()
        .get("/defer", move |_| {
            let tasks = tasks.clone();
            let after_response = after_response.clone();
            async move {
                let deferred = match driver {
                    Some(driver) => Concurrency::driver(driver).defer(tasks()),
                    None => Concurrency::defer(tasks()),
                }
                .map_err(HttpResponse::from)?;
                after_response.store(deferred.after_response(), Ordering::SeqCst);
                Ok(HttpResponse::text("answered"))
            }
        })
        .into()
}

#[tokio::test]
async fn a_handler_that_defers_answers_first_and_the_task_runs_after_the_response() {
    let dir = tempfile::tempdir().expect("a directory");
    let ran = dir.path().join("ran");
    let after_response = Arc::new(AtomicBool::new(false));
    let path = ran.clone();
    let router = deferring_router(
        None,
        move || vec![touch(&path, 500)],
        after_response.clone(),
    );

    let served = serve(router, "/defer", true).await;
    let answered = Instant::now();

    assert_eq!((served.status, served.body.as_str()), (200, "answered"));
    assert!(
        after_response.load(Ordering::SeqCst),
        "the tasks wait for the response"
    );
    assert!(
        served.took < Duration::from_millis(500),
        "the handler did not wait for the task: {:?}",
        served.took
    );
    assert!(
        !exists(&ran),
        "the task has not run when the response arrives"
    );
    assert!(
        wait_until(Duration::from_secs(2), || exists(&ran)).await,
        "the task ran within two seconds of the response"
    );
    assert!(answered.elapsed() < Duration::from_secs(2));
}

#[tokio::test]
#[tracing_test::traced_test]
async fn a_failing_deferred_task_is_logged_and_leaves_the_response_and_the_others_intact() {
    let dir = tempfile::tempdir().expect("a directory");
    let ran = dir.path().join("ran");
    let path = ran.clone();
    let router = deferring_router(
        Some("tokio"),
        move || {
            vec![
                Task::new::<FailWith>("the deferred export broke".to_owned()),
                touch(&path, 0),
            ]
        },
        Arc::new(AtomicBool::new(false)),
    );

    let served = serve(router, "/defer", true).await;

    assert_eq!((served.status, served.body.as_str()), (200, "answered"));
    assert!(
        wait_until(Duration::from_secs(2), || exists(&ran)).await,
        "the other deferred task still ran"
    );
    assert!(
        wait_until(Duration::from_secs(2), || logs_contain(
            "deferred concurrent task failed"
        ))
        .await
    );
    assert!(logs_contain("the deferred export broke"));
    assert!(logs_contain(FailWith::NAME));
}

#[cfg(unix)]
#[tokio::test]
#[serial]
async fn a_handler_can_defer_on_the_process_driver() {
    crate::support::use_this_binary();
    let dir = tempfile::tempdir().expect("a directory");
    let ran = dir.path().join("ran");
    let after_response = Arc::new(AtomicBool::new(false));
    let path = ran.clone();
    let router = deferring_router(
        Some("process"),
        move || vec![touch(&path, 0)],
        after_response.clone(),
    );

    let served = serve(router, "/defer", true).await;

    assert_eq!((served.status, served.body.as_str()), (200, "answered"));
    assert!(after_response.load(Ordering::SeqCst));
    assert!(
        wait_until(Duration::from_secs(5), || exists(&ran)).await,
        "the console process ran the task"
    );
}

#[tokio::test]
async fn without_a_response_writer_the_deferred_task_still_runs() {
    let dir = tempfile::tempdir().expect("a directory");
    let ran = dir.path().join("ran");
    let after_response = Arc::new(AtomicBool::new(true));
    let path = ran.clone();
    let router = deferring_router(
        Some("tokio"),
        move || vec![touch(&path, 0)],
        after_response.clone(),
    );

    let served = serve(router, "/defer", false).await;

    assert_eq!((served.status, served.body.as_str()), (200, "answered"));
    assert!(
        !after_response.load(Ordering::SeqCst),
        "with no writer the handle runs the tasks"
    );
    assert!(wait_until(Duration::from_secs(2), || exists(&ran)).await);
}

#[tokio::test]
async fn outside_a_request_awaiting_the_handle_runs_the_tasks() {
    let dir = tempfile::tempdir().expect("a directory");
    let first: PathBuf = dir.path().join("first");
    let second: PathBuf = dir.path().join("second");

    let deferred = Concurrency::driver("tokio")
        .defer([touch(&first, 0), touch(&second, 50)])
        .expect("the driver exists");
    assert!(!deferred.after_response());
    assert!(!exists(&first), "nothing runs before the handle is awaited");
    deferred.await;

    assert!(exists(&first) && exists(&second));
}

#[tokio::test]
async fn outside_a_request_dropping_the_handle_starts_the_tasks() {
    let dir = tempfile::tempdir().expect("a directory");
    let ran = dir.path().join("ran");

    drop(
        Concurrency::driver("sync")
            .defer([touch(&ran, 0)])
            .expect("the driver exists"),
    );

    assert!(
        wait_until(Duration::from_secs(2), || exists(&ran)).await,
        "the dropped handle started the task"
    );
}

#[test]
fn outside_a_runtime_dropping_the_handle_runs_the_tasks_before_it_returns() {
    let dir = tempfile::tempdir().expect("a directory");
    let ran = dir.path().join("ran");

    drop(
        Concurrency::driver("sync")
            .defer([touch(&ran, 0)])
            .expect("the driver exists"),
    );

    assert!(
        exists(&ran),
        "with no runtime the drop runs the task itself"
    );
}
