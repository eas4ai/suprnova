//! Hold commands until Hyper finishes a response and flushes its bytes to the writer.

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

type CommandFuture = BoxFuture<'static, Result<(), FrameworkError>>;
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

/// Scope one request's pending commands and attach them to its final outgoing body.
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
            // defer refused a command without a writer, so this is present for any pending command.
            let completed = WRITER.with(Arc::clone);
            if response.status().is_informational()
                || response.status() == hyper::StatusCode::NO_CONTENT
                || response.status() == hyper::StatusCode::NOT_MODIFIED
            {
                // Hyper never polls these bodies. Their response is complete when its headers flush.
                lock(&completed).commands.extend(pending);
                return response;
            }
            let (parts, inner) = response.into_parts();
            let body = CompletionBody {
                inner,
                pending: Mutex::new(Some(pending)),
                completed,
            };
            hyper::Response::from_parts(parts, BoxBody::new(body))
        })
        .await
}

struct CompletionBody {
    inner: BoxBody<Bytes, Infallible>,
    pending: Mutex<Option<Commands>>,
    completed: Completed,
}

impl CompletionBody {
    fn finish(&self) {
        if let Some(commands) = lock(&self.pending).take() {
            lock(&self.completed).commands.extend(commands);
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
                    completed.tasks.spawn(async move {
                        for command in commands {
                            // A handler failure cannot change bytes already sent. Report it and continue.
                            let outcome =
                                std::panic::AssertUnwindSafe(command).catch_unwind().await;
                            match outcome {
                                Ok(Ok(())) => {}
                                Ok(Err(error)) => {
                                    tracing::error!(%error, "after-response command failed")
                                }
                                Err(_) => tracing::error!("after-response command panicked"),
                            }
                        }
                    });
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
