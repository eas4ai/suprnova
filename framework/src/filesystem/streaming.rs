//! Cross-disk streaming copy.
//!
//! [`copy_between_disks`] streams bytes from one registered storage disk to
//! another via opendal's `Reader` / `Writer` APIs. The body is consumed in
//! 64 KiB chunks (set explicitly via [`reader_with`](opendal::Operator::reader_with)
//! `.chunk(...)` so backends with smaller defaults don't materialise the whole
//! file in memory), making the helper safe for arbitrarily large objects.
//!
//! Because it is built on the `Operator` abstraction, the source and
//! destination disks can be backed by *any* opendal driver pair -
//! filesystem → S3, S3 → Azure Blob, in-memory → GCS, and so on.
//!
//! # Example
//!
//! ```rust,no_run
//! use suprnova::{Storage, filesystem::streaming::copy_between_disks};
//!
//! # async fn doc() -> Result<(), suprnova::FrameworkError> {
//! Storage::register_fs("local", "./storage")?;
//! Storage::register_memory("scratch");
//!
//! let bytes_copied =
//!     copy_between_disks("local", "uploads/big.bin", "scratch", "big.bin").await?;
//! assert!(bytes_copied > 0);
//! # Ok(())
//! # }
//! ```

use super::Storage;
use crate::FrameworkError;
use futures::TryStreamExt;
use opendal::{Operator, Writer};

/// Streaming chunk size for the reader. 64 KiB strikes a balance between
/// syscall / network round-trips and memory pressure for large files.
const STREAM_CHUNK_BYTES: usize = 64 * 1024;

/// Copy `src_path` from disk `src` to `dest_path` on disk `dest`, streaming
/// bytes in 64 KiB chunks via opendal's reader/writer.
///
/// Returns the total number of bytes transferred on success. The destination
/// writer is explicitly `close()`-d so backends that finalise the upload on
/// close (e.g. S3 multipart) actually commit the object.
///
/// # Errors
///
/// - `FrameworkError::Internal` if either disk is not registered, the source
///   object cannot be opened, the destination cannot be opened, a chunk read
///   fails mid-stream, a chunk write fails, or the final close fails. Each
///   boundary uses a distinct message prefix so failures are identifiable in
///   logs.
///
/// If the task running the transfer is cancelled, the same abort+delete
/// cleanup is diverted to a detached task instead of being skipped: a
/// cancelled copy must not leave a partial destination object (or staged
/// upload parts) behind either.
pub async fn copy_between_disks(
    src: &str,
    src_path: &str,
    dest: &str,
    dest_path: &str,
) -> Result<u64, FrameworkError> {
    let src_op = Storage::disk(src)?;
    let dest_op = Storage::disk(dest)?;

    // `reader_with(..).chunk(N).await` builds a reader that fetches at most N
    // bytes per stream item - this is what makes the "streams in 64 KiB
    // chunks" guarantee real on backends whose default chunk is the whole
    // file (notably the in-memory service used in tests).
    let reader = src_op
        .reader_with(src_path)
        .chunk(STREAM_CHUNK_BYTES)
        .await
        .map_err(|e| FrameworkError::internal(format!("open source: {e}")))?;

    let writer = dest_op
        .writer(dest_path)
        .await
        .map_err(|e| FrameworkError::internal(format!("open dest: {e}")))?;

    // Once the writer is open, a mid-stream failure can leave a partial object
    // at `dest_path`. The guard below owns the writer across the transfer:
    // errors settle inline with abort+delete before propagating (a failed
    // copy must never be observable as a truncated destination object),
    // while task cancellation - which drops the future instead of
    // returning an error - diverts the same cleanup to a detached task.
    let mut guard = WriterGuard::new(dest_op.clone(), dest, dest_path, writer);
    let result = stream_to_writer(reader, guard.writer()).await;
    guard.settle(result).await
}

/// Owns the destination writer across the transfer loop so cleanup runs on
/// every exit path, including task cancellation.
///
/// Errors settle inline ([`WriterGuard::settle`]) with the same
/// abort+delete the old code ran. If the guard is dropped first - the task
/// was aborted or panicked mid-transfer, so no error ever comes back -
/// the unclosed writer's staged/partial output is diverted to a detached
/// task instead of being left behind.
pub(crate) struct WriterGuard {
    writer: Option<Writer>,
    dest_op: Operator,
    dest: String,
    dest_path: String,
    complete: bool,
    delete_destination: bool,
}

impl WriterGuard {
    pub(crate) fn new(dest_op: Operator, dest: &str, dest_path: &str, writer: Writer) -> Self {
        Self {
            writer: Some(writer),
            dest_op,
            dest: dest.to_string(),
            dest_path: dest_path.to_string(),
            complete: false,
            delete_destination: true,
        }
    }

    /// Abort only: an existing destination or conditional-write winner is not
    /// this transfer's object to delete.
    pub(crate) fn preserve_destination(mut self) -> Self {
        self.delete_destination = false;
        self
    }

    pub(crate) fn writer(&mut self) -> &mut Writer {
        self.writer
            .as_mut()
            .expect("writer is taken only while settling")
    }

    /// Settle a finished transfer. Success disarms the guard (the loop
    /// already closed the writer); failure runs abort+delete and then
    /// propagates the original error.
    ///
    /// Cleanup runs on a detached task that is awaited here: if this
    /// task is cancelled mid-cleanup, the detached task still runs it to
    /// completion instead of abandoning the remainder.
    pub(crate) async fn settle<T, E>(mut self, result: Result<T, E>) -> Result<T, E> {
        match result {
            Ok(total) => {
                self.complete = true;
                Ok(total)
            }
            Err(err) => {
                self.cleanup().await;
                Err(err)
            }
        }
    }

    /// Discard a failed transfer or an unpublished promotion that lost a race.
    /// The cleanup task retains ownership if its caller is cancelled.
    pub(crate) async fn cleanup(mut self) {
        let writer = self.writer.take();
        self.complete = true;
        let cleanup = run_cleanup(
            writer,
            self.dest_op.clone(),
            self.dest.clone(),
            self.dest_path.clone(),
            self.delete_destination,
            "discarded",
        );
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                let _ = handle.spawn(cleanup).await;
            }
            // Foreign executors have no runtime to spawn onto.
            Err(_) => cleanup.await,
        }
    }
}

/// Abort the staged write, then delete any visible partial - best
/// effort, failures only logged. `kind` is `"discarded"` for the error
/// path and `"cancelled"` for the divert path, so logs say which exit
/// the transfer took.
async fn run_cleanup(
    writer: Option<Writer>,
    dest_op: Operator,
    dest: String,
    dest_path: String,
    delete_destination: bool,
    kind: &'static str,
) {
    if let Some(mut writer) = writer
        && let Err(abort_err) = writer.abort().await
    {
        tracing::warn!(
            disk = dest,
            path = dest_path,
            error = %abort_err,
            "failed to abort writer while cleaning up a {kind} cross-disk copy"
        );
    }
    if delete_destination && let Err(delete_err) = dest_op.delete(&dest_path).await {
        tracing::warn!(
            disk = dest,
            path = dest_path,
            error = %delete_err,
            "failed to delete partial destination while cleaning up a {kind} cross-disk copy"
        );
    }
}

impl Drop for WriterGuard {
    fn drop(&mut self) {
        if self.complete || self.writer.is_none() {
            return;
        }
        // Cancelled (or panicked) before the transfer settled: an open writer
        // or a closed but unpublished promotion may still own staged output.
        // Divert cleanup to a detached task that outlives this task.
        let writer = self.writer.take();
        let dest_op = self.dest_op.clone();
        let dest = self.dest.clone();
        let dest_path = self.dest_path.clone();
        spawn_cleanup(run_cleanup(
            writer,
            dest_op,
            dest,
            dest_path,
            self.delete_destination,
            "cancelled",
        ));
    }
}

/// Run `future` on a detached task that outlives the spawning task's
/// cancellation. Cancellation-divert only, never the happy path; the
/// cleanup needs no ambient context (operator, writer, and paths are all
/// owned), so nothing is propagated. Without a running runtime
/// (shutdown) there is nothing to spawn onto, which is logged.
fn spawn_cleanup(future: impl std::future::Future<Output = ()> + Send + 'static) {
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.spawn(future);
    } else {
        tracing::error!(
            "cross-disk copy cancelled with no async runtime running; \
             its partial destination may remain"
        );
    }
}

/// Stream the full source object into an already-open destination writer.
///
/// Split out from [`copy_between_disks`] so the caller can clean up a partial
/// destination if any step here fails. Consumes the `reader`; borrows the
/// `writer` so the caller can still `abort()` it on error.
async fn stream_to_writer(
    reader: opendal::Reader,
    writer: &mut opendal::Writer,
) -> Result<u64, FrameworkError> {
    // Full range - copy the entire object. Stream item is `io::Result<Bytes>`.
    let stream = reader
        .into_bytes_stream(..)
        .await
        .map_err(|e| FrameworkError::internal(format!("stream open: {e}")))?;
    let mut stream = std::pin::pin!(stream);

    let mut total: u64 = 0;
    while let Some(chunk) = stream
        .try_next()
        .await
        .map_err(|e| FrameworkError::internal(format!("stream read: {e}")))?
    {
        total += chunk.len() as u64;
        writer
            .write(chunk)
            .await
            .map_err(|e| FrameworkError::internal(format!("write: {e}")))?;
    }

    writer
        .close()
        .await
        .map_err(|e| FrameworkError::internal(format!("close: {e}")))?;

    Ok(total)
}
