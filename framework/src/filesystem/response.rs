//! File responses for a path on a named disk: Laravel's
//! `Storage::download` and `Storage::response`.
//!
//! The file is resolved and read through the disk's own [`Operator`], so
//! whatever the disk enforces applies: a local disk's path guard refuses
//! `..` before anything is read, and a disk with no local paths (S3, an
//! in-memory disk) is read the same way as any other. The headers come from
//! the same helpers as [`HttpResponse::file`], so a file gets the same
//! content type and `Content-Disposition` either way.

use super::Storage;
use crate::FrameworkError;
use crate::http::file_response::{
    BUFFERED_BODY_LIMIT, content_type_from_extension, file_not_found,
};
use crate::http::{ContentDisposition, HttpResponse};
use bytes::Bytes;
use futures::Stream;
use opendal::{EntryMode, ErrorKind, Operator};
use std::convert::Infallible;
use std::path::Path;
use std::pin::Pin;
use std::task::{Context, Poll};

/// Chunk size for a streamed disk body; matches the local file stream.
const DISK_CHUNK_SIZE: usize = 64 * 1024;

impl Storage {
    /// A download of the file at `path` on the disk `disk`: Laravel's
    /// `Storage::disk($disk)->download($path, $name)`.
    ///
    /// The browser saves the file under `name`, or under the last segment
    /// of `path` when `name` is `None`. The content type is the one the
    /// extension of `path` implies (`application/octet-stream` when it
    /// implies none), never what the disk or the content claims. A file
    /// above 1 MiB is streamed from the disk in 64 KiB chunks.
    ///
    /// `path` goes through the disk, so a local disk's path guard
    /// applies: a path that would leave the disk root is refused before
    /// anything is read. That makes this the form to use when a request
    /// names the file.
    ///
    /// # Errors
    ///
    /// A 404 error when nothing is at `path` or it is a directory; a 403
    /// error when the disk refuses the path (a `..` on a local disk); a
    /// 500 error when the disk is not registered or the read fails.
    /// Convert with `.map_err(HttpResponse::from)` in a handler that
    /// returns [`Response`](crate::Response).
    ///
    /// ```rust,no_run
    /// use suprnova::{HttpResponse, Request, Response, Storage};
    ///
    /// async fn invoice(_req: Request) -> Response {
    ///     Storage::download("local", "invoices/2026-0042.pdf", Some("Factura Pérez.pdf"))
    ///         .await
    ///         .map_err(HttpResponse::from)
    /// }
    /// ```
    pub async fn download(
        disk: &str,
        path: &str,
        name: Option<&str>,
    ) -> Result<HttpResponse, FrameworkError> {
        disk_response(disk, path, name, ContentDisposition::Attachment).await
    }

    /// The file at `path` on the disk `disk` for the browser to show:
    /// Laravel's `Storage::disk($disk)->response($path, $name)`.
    ///
    /// The same response as [`Storage::download`], but
    /// `Content-Disposition: inline`.
    ///
    /// # Errors
    ///
    /// The same as [`Storage::download`].
    pub async fn response(
        disk: &str,
        path: &str,
        name: Option<&str>,
    ) -> Result<HttpResponse, FrameworkError> {
        disk_response(disk, path, name, ContentDisposition::Inline).await
    }
}

/// The response `download` and `response` share; only the disposition
/// differs.
async fn disk_response(
    disk_name: &str,
    path: &str,
    name: Option<&str>,
    disposition: ContentDisposition,
) -> Result<HttpResponse, FrameworkError> {
    let disk = Storage::disk(disk_name)?;
    let meta = disk
        .stat(path)
        .await
        .map_err(|error| disk_error(disk_name, path, error))?;
    if meta.mode() == EntryMode::DIR {
        return Err(file_not_found());
    }

    let length = meta.content_length();
    let content_type = content_type_from_extension(Path::new(path));
    let response = disk_body(&disk, disk_name, path, length, content_type).await?;

    let own_name = path.rsplit('/').next().unwrap_or_default();
    let filename = name.unwrap_or(own_name);
    Ok(response.header("Content-Disposition", disposition.header_value(filename)))
}

/// The body for `length` bytes at `path`: buffered at or below
/// [`BUFFERED_BODY_LIMIT`], streamed above it, the same split a local file
/// gets. Both reads are bounded by `length`, so an object that grows after
/// the `stat` cannot make the response larger than it declared.
async fn disk_body(
    disk: &Operator,
    disk_name: &str,
    path: &str,
    length: u64,
    content_type: String,
) -> Result<HttpResponse, FrameworkError> {
    if length == 0 {
        return Ok(
            HttpResponse::bytes_body(Bytes::new(), content_type).header("Content-Length", "0")
        );
    }
    let reader = disk
        .reader_with(path)
        .chunk(DISK_CHUNK_SIZE)
        .await
        .map_err(|error| disk_error(disk_name, path, error))?;

    if length <= BUFFERED_BODY_LIMIT {
        let bytes = reader
            .read(0..length)
            .await
            .map_err(|error| disk_error(disk_name, path, error))?
            .to_bytes();
        let read = bytes.len();
        Ok(
            HttpResponse::bytes_body(bytes, content_type)
                .header("Content-Length", read.to_string()),
        )
    } else {
        let stream = reader
            .into_bytes_stream(0..length)
            .await
            .map_err(|error| disk_error(disk_name, path, error))?;
        Ok(HttpResponse::stream_bytes(EndOnError::new(stream))
            .header("Content-Type", content_type)
            .header("Content-Length", length.to_string()))
    }
}

/// Map a disk error to the error a handler can answer with. The 404 and
/// 403 messages name no path, so they can reach the client; a 500 keeps
/// the disk and path in the message, which only reaches the log.
fn disk_error(disk_name: &str, path: &str, error: opendal::Error) -> FrameworkError {
    match error.kind() {
        ErrorKind::NotFound | ErrorKind::IsADirectory | ErrorKind::NotADirectory => {
            file_not_found()
        }
        ErrorKind::PermissionDenied => FrameworkError::domain("Access to the file is denied", 403),
        _ => FrameworkError::from_external_with(
            format!("storage response({disk_name}: {path})"),
            error,
        ),
    }
}

/// A disk byte stream as an infallible body stream.
///
/// The framework's streaming bodies cannot carry an error, so a read that
/// fails mid-body ends the stream instead. The body declared its full
/// `Content-Length`, so hyper's length-delimited encoder sees bytes still
/// owed and aborts the connection: the client gets a broken transfer, never
/// a body that looks complete but is short.
struct EndOnError<S> {
    inner: S,
    finished: bool,
}

impl<S> EndOnError<S> {
    fn new(inner: S) -> Self {
        Self {
            inner,
            finished: false,
        }
    }
}

impl<S> Stream for EndOnError<S>
where
    S: Stream<Item = std::io::Result<Bytes>> + Unpin,
{
    type Item = Result<Bytes, Infallible>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if this.finished {
            return Poll::Ready(None);
        }
        match Pin::new(&mut this.inner).poll_next(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Some(Ok(chunk))) => Poll::Ready(Some(Ok(chunk))),
            Poll::Ready(Some(Err(error))) => {
                this.finished = true;
                tracing::warn!(
                    error = %error,
                    "storage stream read failed; ending body short so the \
                     connection aborts instead of completing a partial payload"
                );
                Poll::Ready(None)
            }
            Poll::Ready(None) => {
                this.finished = true;
                Poll::Ready(None)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    #[tokio::test]
    async fn a_read_error_ends_the_body_after_the_chunks_before_it() {
        let chunks = futures::stream::iter(vec![
            Ok(Bytes::from_static(b"abc")),
            Err(std::io::Error::other("connection reset")),
            Ok(Bytes::from_static(b"never sent")),
        ]);
        let mut stream = EndOnError::new(chunks);
        let mut emitted = Vec::new();
        while let Some(chunk) = stream.next().await {
            emitted.extend_from_slice(&chunk.expect("infallible stream chunk"));
        }
        assert_eq!(emitted, b"abc");
        assert!(stream.finished);
    }
}
