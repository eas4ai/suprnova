//! The stream client every Inertia kit scaffolds,
//! `frontend/src/lib/stream/core.ts`, run by Node against the framework's
//! router and middleware over a real socket: the session and CSRF checks a
//! stream request passes, the text, JSON and event streams it reads, a
//! rejected request, and a stream cancelled midway.
//!
//! Node 24 runs TypeScript by stripping its types, so the test runs the
//! template the kits write, not a copy of its logic. It is ignored by
//! default because it needs that Node.

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use futures::stream;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use serde_json::{Value, json};
use suprnova::session::{SessionConfig, SessionData, SessionMiddleware, SessionStore};
use suprnova::sse::{EndSignal, StreamedEvent};
use suprnova::{
    CsrfMiddleware, FrameworkError, HttpResponse, MiddlewareRegistry, Request, Response, Router,
    handle_request,
};
use tokio::sync::mpsc;

/// Sessions kept in memory for the run.
#[derive(Default)]
struct Sessions(Mutex<HashMap<String, SessionData>>);

#[async_trait]
impl SessionStore for Sessions {
    async fn read(&self, id: &str) -> Result<Option<SessionData>, FrameworkError> {
        Ok(self.0.lock().unwrap().get(id).cloned())
    }

    async fn write(&self, session: &SessionData) -> Result<(), FrameworkError> {
        self.0
            .lock()
            .unwrap()
            .insert(session.id.clone(), session.clone());
        Ok(())
    }

    async fn destroy(&self, id: &str) -> Result<(), FrameworkError> {
        self.0.lock().unwrap().remove(id);
        Ok(())
    }

    async fn destroy_for_user(&self, _user_id: &str) -> Result<u64, FrameworkError> {
        Ok(0)
    }

    async fn gc(&self) -> Result<u64, FrameworkError> {
        Ok(0)
    }
}

/// The items a task sends, as a stream a response can carry.
struct Paced<T>(mpsc::Receiver<T>);

impl<T> futures::Stream for Paced<T> {
    type Item = T;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<T>> {
        self.0.poll_recv(cx)
    }
}

/// Sends `items` 20 ms apart from a task, so each tends to reach the client
/// in its own read, then holds the channel open for `linger`.
fn paced<T: Send + 'static>(items: Vec<T>, linger: Duration) -> Paced<T> {
    let (tx, rx) = mpsc::channel(items.len().max(1));
    tokio::spawn(async move {
        for item in items {
            tokio::time::sleep(Duration::from_millis(20)).await;
            if tx.send(item).await.is_err() {
                return;
            }
        }
        tokio::time::sleep(linger).await;
    });
    Paced(rx)
}

/// The text `/chat` answers: the `ö` is split across two pieces, which a
/// client decoding each read on its own would mangle.
async fn chat(request: Request) -> Response {
    let id = request.header("X-STREAM-ID").unwrap_or("none").to_owned();
    let body: Value = request.json().await?;
    let message = body["message"].as_str().unwrap_or_default().to_owned();
    let pieces = [
        b"Hello ".to_vec(),
        b"w\xc3".to_vec(),
        b"\xb6rld".to_vec(),
        format!(", {message} ({id})").into_bytes(),
    ]
    .into_iter()
    .map(|piece| Ok::<_, Infallible>(Bytes::from(piece)))
    .collect();
    Ok(HttpResponse::stream_bytes(paced(pieces, Duration::ZERO)))
}

/// Two build events, a log line that is not JSON, and the end signal.
async fn deploy(request: Request) -> Response {
    let body: Value = request.json().await?;
    let events = vec![
        StreamedEvent::message(json!({ "step": "build", "environment": body["environment"] })),
        StreamedEvent::named("log", "plain text"),
        StreamedEvent::message(json!({ "step": "done" })),
    ]
    .into_iter()
    .collect::<Result<Vec<_>, _>>()
    .map_err(|error| FrameworkError::internal(error.to_string()))?;
    Ok(HttpResponse::event_stream(
        stream::iter(events),
        EndSignal::default(),
    ))
}

/// One event, then nothing for ten seconds: a stream to cancel.
async fn slow(_request: Request) -> Response {
    let first = StreamedEvent::message(json!({ "step": "started" }))
        .map_err(|error| FrameworkError::internal(error.to_string()))?;
    Ok(HttpResponse::event_stream(
        paced(vec![first], Duration::from_secs(10)),
        EndSignal::default(),
    ))
}

/// Two messages over a GET, for `EventSource`.
async fn progress(_request: Request) -> Response {
    let events = ["one", "two"]
        .into_iter()
        .map(StreamedEvent::message)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| FrameworkError::internal(error.to_string()))?;
    Ok(HttpResponse::event_stream(
        paced(events, Duration::ZERO),
        EndSignal::default(),
    ))
}

/// Serves `router` behind the session and CSRF middleware on a loopback
/// port, one task per connection, until the test ends.
async fn serve(router: Router) -> SocketAddr {
    suprnova::testing::install_test_encryption_key();
    let mut config = SessionConfig::default();
    config.cookie_secure = false;
    let registry = Arc::new(
        MiddlewareRegistry::new()
            .append(SessionMiddleware::with_store(
                config,
                Arc::new(Sessions::default()),
            ))
            .append(CsrfMiddleware::new()),
    );
    let router = Arc::new(router);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let (router, registry) = (router.clone(), registry.clone());
            tokio::spawn(async move {
                let service = service_fn(move |request| {
                    let (router, registry) = (router.clone(), registry.clone());
                    async move { Ok::<_, Infallible>(handle_request(router, registry, request).await) }
                });
                let _ = http1::Builder::new()
                    .serve_connection(TokioIo::new(tcp), service)
                    .await;
            });
        }
    });
    addr
}

/// The script Node runs next to the kit's `core.ts`.
const SCRIPT: &str = r#"
import assert from 'node:assert/strict'
import {
  JsonEventStream,
  StreamResponseError,
  csrfHeaders,
  openEventStream,
  readEventStream,
  streamJsonEvents,
  textStream,
} from './core.ts'

const base = process.env.BASE

// Frames split anywhere, CRLF split across reads, comments, multi-line data
// and an id that holds until the server sends another.
{
  const pieces = ['id: 7\r', '\nevent: a\ndata: x\r\ndata: y\r\n\r\n: note\n\n', 'data: z\n\n', 'data: lost']
  const body = new ReadableStream({
    start(controller) {
      for (const piece of pieces) controller.enqueue(new TextEncoder().encode(piece))
      controller.close()
    },
  })
  const frames = []
  await readEventStream(body, (frame) => frames.push(frame) > 0)
  assert.deepEqual(frames, [
    { event: 'a', id: '7', data: 'x\ny' },
    { event: null, id: '7', data: 'z' },
  ])
}

// A first visit gives the session and its XSRF-TOKEN cookie; with no meta
// tag the token comes from the cookie, as a page without one sends it.
const page = await fetch(`${base}/page`)
const cookie = page.headers.getSetCookie().map((line) => line.split(';')[0]).join('; ')
globalThis.document = { cookie, querySelector: () => null }
const token = csrfHeaders()['X-XSRF-TOKEN']
assert.ok(token, `no XSRF-TOKEN in ${cookie}`)
assert.deepEqual(csrfHeaders({ csrfToken: 't' }), { 'X-CSRF-TOKEN': 't' })
const headers = { Cookie: cookie }

// useStream: the text arrives in pieces and whole, with the stream's id.
{
  const stream = textStream('chat-1')
  const states = []
  const pieces = []
  let finished = 0
  const unsubscribe = stream.subscribe(() => states.push(stream.snapshot()))
  stream.listen({ onData: (text) => pieces.push(text), onFinish: () => finished++ })
  await stream.send(`${base}/chat`, { message: 'hi' }, { headers })
  assert.equal(stream.snapshot().data, 'Hello wörld, hi (chat-1)')
  assert.equal(pieces.join(''), 'Hello wörld, hi (chat-1)')
  assert.ok(states.some((state) => state.isFetching), 'never fetching')
  assert.ok(states.some((state) => state.isStreaming), 'never streaming')
  assert.deepEqual(
    { isFetching: stream.snapshot().isFetching, isStreaming: stream.snapshot().isStreaming },
    { isFetching: false, isStreaming: false },
  )
  assert.equal(finished, 1)
  assert.equal(textStream('chat-1'), stream, 'a subscribed stream is shared by id')
  unsubscribe()
  assert.notEqual(textStream('chat-1'), stream, 'the last reader leaving drops it')
}

// useJsonStream: a stream_json response parsed once it ends.
{
  const stream = textStream('export')
  stream.subscribe(() => undefined)
  await stream.send(`${base}/export`, {}, { headers, json: true })
  assert.deepEqual(stream.snapshot().jsonData, [{ id: 1 }, { id: 2 }])
}

// Without the token the server refuses the request: 419, reported once.
{
  globalThis.document = undefined
  const stream = textStream('refused')
  stream.subscribe(() => undefined)
  const errors = []
  stream.listen({ onError: (error) => errors.push(error) })
  await stream.send(`${base}/chat`, { message: 'x' }, { headers })
  assert.equal(errors.length, 1)
  assert.ok(errors[0] instanceof StreamResponseError, String(errors[0]))
  assert.equal(errors[0].status, 419)
  assert.equal(stream.snapshot().isFetching, false)
  globalThis.document = { cookie, querySelector: () => null }
}

// streamJsonEvents: every event in order, the log line reported as not JSON,
// and a name filter that keeps only `update`.
{
  const events = []
  const unparsed = []
  const sent = await streamJsonEvents({
    url: `${base}/deploy`,
    body: { environment: 'production' },
    signal: new AbortController().signal,
    headers,
    onEvent: (event, frame) => events.push([event, frame.event]),
    onParseError: (_error, data) => unparsed.push(data),
  })
  assert.equal(sent, true)
  assert.deepEqual(events, [
    [{ step: 'build', environment: 'production' }, 'update'],
    [{ step: 'done' }, 'update'],
  ])
  assert.deepEqual(unparsed, ['plain text'])

  const named = []
  await streamJsonEvents({
    url: `${base}/deploy`,
    body: {},
    signal: new AbortController().signal,
    headers,
    eventName: 'log',
    onEvent: (event) => named.push(event),
    onParseError: (_error, data) => named.push(`unparsed ${data}`),
  })
  assert.deepEqual(named, ['unparsed plain text'])

  assert.equal(
    await streamJsonEvents({
      url: `${base}/deploy`,
      signal: new AbortController().signal,
      onEvent: () => undefined,
      onBeforeSend: () => false,
    }),
    false,
  )
}

// useJsonEventStream: the events collected; a cancelled stream reports
// onCancel, not onError, and stops.
{
  const reports = []
  const stream = new JsonEventStream(() => ({
    headers,
    onFinish: () => reports.push('finish'),
    onCancel: () => reports.push('cancel'),
    onError: (error) => reports.push(`error ${error}`),
  }))
  await stream.send(`${base}/deploy`, { environment: 'staging' })
  assert.deepEqual(stream.snapshot().events, [{ step: 'build', environment: 'staging' }, { step: 'done' }])
  assert.deepEqual(reports, ['finish'])

  reports.length = 0
  const started = new Promise((resolve) => {
    const unsubscribe = stream.subscribe(() => {
      if (stream.snapshot().events.length > 0) {
        unsubscribe()
        resolve()
      }
    })
  })
  const sending = stream.send(`${base}/slow`, {})
  await started
  const before = Date.now()
  stream.cancel()
  await sending
  assert.ok(Date.now() - before < 2000, 'cancel did not stop the stream')
  assert.deepEqual(reports, ['cancel'])
  assert.deepEqual(stream.snapshot().isStreaming, false)
}

// useEventStream: EventSource messages joined until the end signal.
{
  const states = []
  await new Promise((resolve, reject) => {
    openEventStream(
      `${base}/progress`,
      () => ({ onComplete: resolve, onError: () => reject(new Error('EventSource error')) }),
      (state) => states.push(state),
    )
  })
  assert.deepEqual(states.at(-1), { message: 'one two', messageParts: ['one', 'two'] })
}

console.log('stream client ok')
"#;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Node 24, which runs the scaffold's TypeScript by stripping its types"]
async fn the_scaffolded_stream_client_reads_every_stream_the_framework_sends() {
    let router: Router = Router::new()
        .get("/page", |_request: Request| async {
            Ok(HttpResponse::text("page"))
        })
        .post("/chat", chat)
        .post("/export", |_request: Request| async {
            Ok(HttpResponse::stream_json(stream::iter(vec![
                json!({ "id": 1 }),
                json!({ "id": 2 }),
            ])))
        })
        .post("/deploy", deploy)
        .post("/slow", slow)
        .get("/progress", progress)
        .into();
    let addr = serve(router).await;

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("core.ts"),
        include_str!("../../../suprnova-cli/src/templates/files/frontend/stream/core.ts.tpl"),
    )
    .unwrap();
    std::fs::write(dir.path().join("client.test.mjs"), SCRIPT).unwrap();

    let output = tokio::time::timeout(
        Duration::from_secs(60),
        tokio::process::Command::new("node")
            .args([
                "--no-warnings",
                "--experimental-eventsource",
                "client.test.mjs",
            ])
            .current_dir(dir.path())
            .env("BASE", format!("http://{addr}"))
            .output(),
    )
    .await
    .expect("the Node script timed out")
    .expect("node runs");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success() && stdout.contains("stream client ok"),
        "node failed ({}):\n{stdout}\n{stderr}",
        output.status
    );
}
