//! Hold commands until Hyper finishes a response and flushes its bytes to the writer.
//!
//! A request's pending commands complete on one of two paths, because not
//! every connection that serves a request can tell when its bytes left:
//!
//! - **With a writer.** The framework server scopes each connection with
//!   [`after_response_connection`], which wraps its IO in
//!   [`AfterResponseIo`]. The commands start after the writer's next
//!   successful flush, once the response's bytes are on the wire, and a
//!   failed flush discards them.
//! - **Without a writer.** A request served through `handle_request` on a
//!   connection with no writer, such as the in-memory test client or a
//!   custom Hyper loop, still has a response to wait for. With no flush to
//!   watch, the end of the response's body is the last point the framework
//!   sees: the commands are spawned on the runtime there, as one task that
//!   runs them in order. A response Hyper sends without a body
//!   (informational, 204 or 304) spawns them as soon as it is built. There
//!   is no failed flush to see either, so they run even when the client
//!   has gone.
//!
//! `Bus::dispatch_after_response` queues only to the first path (`defer`):
//! a command dispatched after the response must start only after its bytes
//! flushed. `Concurrency::defer` queues to either (`try_defer`), since
//! running its tasks after the response is all it promises.

use crate::{FrameworkError, lock::recover as lock};
use bytes::Bytes;
use futures::{FutureExt, future::BoxFuture};
use http_body_util::combinators::BoxBody;
use hyper::body::{Body, Frame, SizeHint};
use std::cell::RefCell;
use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

pub(crate) type CommandFuture = BoxFuture<'static, Result<(), FrameworkError>>;
type Commands = Vec<CommandFuture>;
#[derive(Default)]
struct CompletionState {
    commands: Commands,
    tasks: tokio::task::JoinSet<()>,
}

type Completed = Arc<Mutex<CompletionState>>;

tokio::task_local! {
    static WRITER: Completed;
    static PENDING: RefCell<Commands>;
}

pub(super) fn defer(future: CommandFuture) -> Result<(), FrameworkError> {
    if WRITER.try_with(|_| ()).is_err() {
        return Err(FrameworkError::internal(
            "Bus::dispatch_after_response requires an HTTP response writer; use after_response_connection in a custom Hyper loop",
        ));
    }
    PENDING
        .try_with(|pending| pending.borrow_mut().push(future))
        .map_err(|_| {
            FrameworkError::internal(
                "Bus::dispatch_after_response requires an active HTTP response",
            )
        })
}

/// Hold `future` until the current response has been sent, or hand it
/// back when there is no response to wait for, outside a request.
///
/// Inside a request the future waits for the response whether or not a
/// response writer is in reach: with one it starts after the flush, and
/// without one when the response's body ends (see the module docs).
///
/// [`defer`] refuses a request without a writer, and a call outside a
/// request, with an error, which is right for the bus, where a command
/// dispatched after the response must not run at any other time.
/// `Concurrency::defer` runs its tasks outside a request anyway, through
/// the handle it returns, so it needs the future back.
pub(crate) fn try_defer(future: CommandFuture) -> Result<(), CommandFuture> {
    let mut held = Some(future);
    match PENDING.try_with(|pending| pending.borrow_mut().extend(held.take())) {
        Ok(()) => Ok(()),
        // Outside the request's scope the closure never ran, so the future
        // is still held.
        Err(_) => held.map_or(Ok(()), Err),
    }
}

/// Scope one request's pending commands and attach them to its final
/// outgoing body, on the completion path the connection allows.
pub(crate) async fn response<F>(future: F) -> hyper::Response<BoxBody<Bytes, Infallible>>
where
    F: Future<Output = hyper::Response<BoxBody<Bytes, Infallible>>>,
{
    PENDING
        .scope(RefCell::new(Vec::new()), async move {
            let response = future.await;
            let pending = PENDING.with(|pending| pending.take());
            if pending.is_empty() {
                return response;
            }
            let completion = Completion::current();
            if response.status().is_informational()
                || response.status() == hyper::StatusCode::NO_CONTENT
                || response.status() == hyper::StatusCode::NOT_MODIFIED
            {
                // Hyper never polls these bodies. With a writer, their
                // response is complete when its headers flush; without
                // one, nothing later than now marks it.
                completion.complete(pending);
                return response;
            }
            let (parts, inner) = response.into_parts();
            let body = CompletionBody {
                inner,
                pending: Mutex::new(Some(pending)),
                completion,
            };
            hyper::Response::from_parts(parts, BoxBody::new(body))
        })
        .await
}

/// Where a response's pending commands go once the response is complete.
enum Completion {
    /// The connection's writer: they start after its next successful flush.
    Writer(Completed),
    /// No writer in reach: they start at once on the runtime the response
    /// was built on. `None` when it was built outside a Tokio runtime,
    /// where nothing can start them.
    Runtime(Option<tokio::runtime::Handle>),
}

impl Completion {
    /// The path of the current request: its connection's writer when one
    /// is in scope, otherwise the current runtime.
    fn current() -> Self {
        match WRITER.try_with(Arc::clone) {
            Ok(completed) => Self::Writer(completed),
            Err(_) => Self::Runtime(tokio::runtime::Handle::try_current().ok()),
        }
    }

    /// Hand `commands` to this path.
    fn complete(&self, commands: Commands) {
        match self {
            Self::Writer(completed) => lock(completed).commands.extend(commands),
            Self::Runtime(Some(runtime)) => {
                // Detached: the commands run on and log their own failures.
                drop(runtime.spawn(run_in_order(commands)));
            }
            Self::Runtime(None) => tracing::error!(
                commands = commands.len(),
                "after-response commands did not run: the response was built outside a Tokio runtime"
            ),
        }
    }
}

/// Run `commands` one after another. A command that fails or panics
/// cannot change bytes already sent, so it is logged and the next one
/// runs.
async fn run_in_order(commands: Commands) {
    for command in commands {
        let outcome = std::panic::AssertUnwindSafe(command).catch_unwind().await;
        match outcome {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                tracing::error!(%error, "after-response command failed")
            }
            Err(_) => tracing::error!("after-response command panicked"),
        }
    }
}

struct CompletionBody {
    inner: BoxBody<Bytes, Infallible>,
    pending: Mutex<Option<Commands>>,
    completion: Completion,
}

impl CompletionBody {
    fn finish(&self) {
        if let Some(commands) = lock(&self.pending).take() {
            self.completion.complete(commands);
        }
    }
}

impl Drop for CompletionBody {
    fn drop(&mut self) {
        // Hyper can stop polling once Content-Length bytes have been written.
        self.finish();
    }
}

impl Body for CompletionBody {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        let frame = Pin::new(&mut self.inner).poll_frame(cx);
        if matches!(frame, Poll::Ready(None)) || self.inner.is_end_stream() {
            self.finish();
        }
        frame
    }

    fn is_end_stream(&self) -> bool {
        let ended = self.inner.is_end_stream();
        if ended {
            self.finish();
        }
        ended
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

/// Wrap a response writer so after-response commands start only after a successful flush.
/// Use [`after_response_connection`] to construct it and scope your custom Hyper connection.
pub struct AfterResponseIo<I> {
    inner: I,
    completed: Completed,
}

impl<I: AsyncRead + Unpin> AsyncRead for AfterResponseIo<I> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl<I: AsyncWrite + Unpin> AsyncWrite for AfterResponseIo<I> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[std::io::IoSlice<'_>],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match Pin::new(&mut self.inner).poll_flush(cx) {
            Poll::Ready(Ok(())) => {
                let mut completed = lock(&self.completed);
                // Reap finished handlers on keep-alive connections so task handles do not accumulate.
                while completed.tasks.try_join_next().is_some() {}
                let commands = std::mem::take(&mut completed.commands);
                if !commands.is_empty() {
                    completed.tasks.spawn(run_in_order(commands));
                }
                Poll::Ready(Ok(()))
            }
            Poll::Ready(Err(error)) => {
                lock(&self.completed).commands.clear();
                Poll::Ready(Err(error))
            }
            Poll::Pending => Poll::Pending,
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

/// Give a custom Hyper connection the write-completion hook the framework server uses.
/// Pass the wrapped IO to `TokioIo::new` and build `serve_connection` inside `serve`.
pub async fn after_response_connection<I, F, Fut>(io: I, serve: F) -> Fut::Output
where
    F: FnOnce(AfterResponseIo<I>) -> Fut,
    Fut: Future,
{
    let completed = Completed::default();
    let writer = AfterResponseIo {
        inner: io,
        completed: completed.clone(),
    };
    let result = WRITER.scope(completed.clone(), serve(writer)).await;
    let mut tasks = std::mem::take(&mut lock(&completed).tasks);
    // A connection that closes just after its response still owns the commands it started.
    while let Some(joined) = tasks.join_next().await {
        if let Err(error) = joined {
            tracing::error!(%error, "after-response task failed");
        }
    }
    result
}
