//! Task 5's body recorder: how a bypass test observes the served body by
//! address rather than by value.
//!
//! Split out of `mod.rs` by Task 5b's review, for the reason
//! [`super::probe`] gives. `mod.rs` keeps `dispatch_recording`, which every
//! dispatch in this suite goes through, and re-exports the three items
//! here, so no test's imports change. A pure move.

use super::*;

/// Where a recording dispatch collects one entry per response body frame:
/// the frame's address and its length, never its bytes.
pub type FrameLog = Arc<Mutex<Vec<(usize, usize)>>>;

/// A response body that records the address and length of every data frame
/// hyper pulls out of it, and changes nothing else - it forwards each frame
/// on untouched, so what it reports is what the connection went on to
/// write.
///
/// Task 5: this is the only way to prove that serving a hit hands hyper the
/// bytes L0 stores rather than a copy of them. The client side of
/// [`dispatch`] reads the response back over a real TCP connection, so the
/// body it collects is a fresh allocation whichever buffer the server wrote
/// from, and its address proves nothing either way. The frame this wrapper
/// sees is the last point at which the server still holds that buffer.
pub struct CountingBody<B> {
    inner: B,
    frames: FrameLog,
}

impl<B> CountingBody<B> {
    pub(super) fn new(inner: B, frames: FrameLog) -> Self {
        Self { inner, frames }
    }
}

impl<B> hyper::body::Body for CountingBody<B>
where
    B: hyper::body::Body<Data = Bytes> + Unpin,
{
    type Data = Bytes;
    type Error = B::Error;

    fn poll_frame(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Result<hyper::body::Frame<Self::Data>, Self::Error>>> {
        // Sound without `unsafe` and without a projection crate because
        // the bound above requires `B: Unpin`, which makes the whole
        // wrapper `Unpin` (its other field is an `Arc`).
        let this = self.get_mut();
        let polled = std::pin::Pin::new(&mut this.inner).poll_frame(cx);
        if let std::task::Poll::Ready(Some(Ok(frame))) = &polled
            && let Some(data) = frame.data_ref()
        {
            this.frames
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push((data.as_ptr() as usize, data.len()));
        }
        polled
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> hyper::body::SizeHint {
        self.inner.size_hint()
    }
}

/// Dispatches a `GET` request to `path`, recording the address and length
/// of every body frame the server hands hyper into `frames`. See
/// [`CountingBody`] for why the recording has to happen there and not on
/// the client side of this connection.
pub async fn dispatch_get_recording(
    harness: &Harness,
    path: &str,
    frames: &FrameLog,
) -> TestResponse {
    dispatch_recording(
        harness,
        hyper::Method::GET,
        path,
        &[],
        Some(Arc::clone(frames)),
        None,
    )
    .await
}

/// Where a timed dispatch collects one entry per request: how long the
/// server side took, never anything about the request or the response.
///
/// Task 7. A dispatch in this suite serves every request over a fresh
/// loopback TCP connection, so the time a client measures around
/// [`dispatch_get`](super::dispatch_get) is a connection plus a request,
/// and a caller that wants to report what the *middleware* cost cannot get
/// it from the client side at all. This is measured where the work
/// happens: around `handle_request`, inside the service the test host
/// runs, which is the same place [`CountingBody`] wraps the body and for
/// the same reason.
pub type ServerTimingLog = Arc<Mutex<Vec<std::time::Duration>>>;

/// Dispatches a `GET` request to `path`, recording how long the server side
/// of it took into `timings`.
///
/// The duration ends when the response value exists, which is before hyper
/// has written a byte of it and before the client has read one, so it
/// excludes the connection, the response write, and the client's own read.
/// A caller that wants the whole round trip measures around this call as
/// well; the two together are what the request cost and what the server
/// cost.
pub async fn dispatch_get_timed(
    harness: &Harness,
    path: &str,
    timings: &ServerTimingLog,
) -> TestResponse {
    dispatch_recording(
        harness,
        hyper::Method::GET,
        path,
        &[],
        None,
        Some(Arc::clone(timings)),
    )
    .await
}
