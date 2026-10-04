//! Streaming multipart upload support.
//!
//! Public API:
//! - `#[derive(MultipartRequest)]` - strongly-typed extractor for handlers
//! - `UploadedFile<V>` - single uploaded file with validator `V`
//! - `parse_multipart_streaming` - low-level helper for advanced parsers
//! - `MultipartRequestHooks` - `authorize` / `after_validation` /
//!   `after_validation_async` lifecycle hooks
//!
//! # Streaming model
//!
//! Each multipart part is collected into one of two backings:
//!
//! - **Memory** (`Bytes`) - fast path for small parts. Default cap is
//!   2 MiB, configurable via [`set_global_upload_spill_threshold`].
//! - **Disk** (`tempfile::NamedTempFile`) - spill path for large parts.
//!   Chunks are streamed into a temp file as they arrive from the
//!   transport, so a 200 MiB video upload never resides fully in RAM.
//!
//! `UploadedFile::store_as` streams from disk-backed parts directly to
//! the destination storage in 64 KiB chunks via `opendal::Operator::writer` -
//! true streaming, not a final-write of a buffered blob.
//!
//! Body is consumed exactly once per request. The derive macro
//! dispatches by `#[field("name")]` so multiple files + text fields
//! in one handler share the same parse.
//!
//! # Failures
//!
//! A failure that belongs to one field - a missing required field, a
//! value that does not parse, a file a validator refuses - answers 422
//! with [`ValidationErrors`] under the field's input name, so a form can
//! show it under the field. A limit on the whole request - the body's
//! byte cap, the part ceiling, a field's `max_count`, a text part's
//! in-memory limit - answers 413.

use crate::error::{FrameworkError, ValidationErrors};
use crate::validation::message::ValidationMessage;
use bytes::Bytes;
use futures::StreamExt;
use http_body_util::BodyDataStream;
use multer::Multipart;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tempfile::NamedTempFile;
#[cfg(feature = "filesystem")]
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;

pub mod validators;
use validators::UploadValidator;

/// Default per-request multipart body cap when none is configured.
/// 25 MiB matches what most production apps want as their default
/// upper bound - large enough for typical document/image uploads,
/// small enough that an unauthenticated client can't trivially DoS.
pub const DEFAULT_MAX_MULTIPART_BODY_BYTES: usize = 25 * 1024 * 1024;

/// Default in-memory buffer size before a single part spills to a
/// temp file. 2 MiB - small enough that typical avatar/image uploads
/// stay in memory (fast path), large enough that buffer thrashing is
/// rare for legitimate uploads.
pub const DEFAULT_UPLOAD_SPILL_THRESHOLD: usize = 2 * 1024 * 1024;

/// Maximum bytes captured into the sniff buffer for magic-byte content
/// inference. `infer::get` only needs the first ~32 bytes for every
/// format it recognises; 16 KiB is comfortably generous and bounds the
/// buffer size for arbitrarily large parts.
const SNIFF_BYTES: usize = 16 * 1024;

/// File name prefix of the temp files large parts spill to, so an operator
/// can tell them from other temp files.
const SPILL_FILE_PREFIX: &str = "suprnova-upload-";

/// Streaming chunk size used when copying a disk-backed part to the
/// destination storage. 64 KiB matches the cross-disk streaming helper
/// in [`crate::filesystem::streaming`] and balances syscall/network
/// round-trips against memory pressure.
#[cfg(feature = "filesystem")]
const STORE_AS_CHUNK_BYTES: usize = 64 * 1024;

static GLOBAL_MAX_BODY: AtomicUsize = AtomicUsize::new(0);
static GLOBAL_SPILL_THRESHOLD: AtomicUsize = AtomicUsize::new(0);

/// Set the process-global cap on multipart request body size, in bytes.
///
/// Called at boot - typically from `bootstrap.rs` - to override the
/// compile-time [`DEFAULT_MAX_MULTIPART_BODY_BYTES`]. Setting `0` is
/// special: it means "use the default". Setting `usize::MAX` disables
/// the cap entirely.
///
/// Per-struct overrides via `#[multipart(max_body_bytes = N)]` still
/// take precedence.
///
/// Thread-safe; can be called multiple times. The most recent value
/// wins for any subsequent request.
pub fn set_global_max_multipart_body_bytes(bytes: usize) {
    GLOBAL_MAX_BODY.store(bytes, Ordering::SeqCst);
}

/// Read the currently-configured global cap. Returns the default if
/// [`set_global_max_multipart_body_bytes`] has never been called or was
/// called with `0`.
pub fn global_max_multipart_body_bytes() -> usize {
    let stored = GLOBAL_MAX_BODY.load(Ordering::SeqCst);
    if stored == 0 {
        DEFAULT_MAX_MULTIPART_BODY_BYTES
    } else {
        stored
    }
}

/// Set the process-global spill threshold for multipart parts. Parts
/// whose accumulated bytes exceed this value spill from memory to a
/// `tempfile::NamedTempFile` so the framework never materialises an
/// arbitrarily large body in RAM.
///
/// A text part cannot spill: form text must fit in memory, so a text part
/// over this value answers 413, as a body over its cap does. Lowering the
/// threshold to keep less of each file in memory lowers the largest text
/// field accepted with it.
///
/// Setting `0` is special: it means "use [`DEFAULT_UPLOAD_SPILL_THRESHOLD`]".
/// Setting `usize::MAX` effectively disables spilling (every part is
/// buffered fully - only do this if you're certain about your body cap).
///
/// Thread-safe; can be called multiple times. The most recent value
/// wins for any subsequent request.
pub fn set_global_upload_spill_threshold(bytes: usize) {
    GLOBAL_SPILL_THRESHOLD.store(bytes, Ordering::SeqCst);
}

/// Read the currently-configured spill threshold. Returns the default
/// if [`set_global_upload_spill_threshold`] has never been called or
/// was called with `0`.
pub fn global_upload_spill_threshold() -> usize {
    let stored = GLOBAL_SPILL_THRESHOLD.load(Ordering::SeqCst);
    if stored == 0 {
        DEFAULT_UPLOAD_SPILL_THRESHOLD
    } else {
        stored
    }
}

/// Default ceiling on the number of parts accepted from a single
/// multipart request.
///
/// The total-byte cap bounds raw payload bytes, but multipart framing
/// (boundary + per-part headers) is not counted toward it - so without a
/// part ceiling a body composed of many tiny parts can drive unbounded
/// `MultipartPayload` growth while staying within the byte budget. 1000
/// mirrors PHP's `max_input_vars` and sits comfortably above any
/// legitimate form.
pub const DEFAULT_MAX_MULTIPART_PARTS: usize = 1000;

static GLOBAL_MAX_PARTS: AtomicUsize = AtomicUsize::new(0);

/// Count of multipart parts spilled from memory to a temp file since
/// process start (see [`upload_tempfiles_spilled_total`]).
static UPLOAD_TEMPFILES_SPILLED: AtomicUsize = AtomicUsize::new(0);

/// Set the process-global ceiling on the number of parts accepted from a
/// single multipart request. Once a request presents more than this many
/// parts the parser rejects it with HTTP 413 *before reading the
/// offending part*, so allocation stays bounded regardless of per-field
/// configuration.
///
/// Setting `0` is special: it means "use [`DEFAULT_MAX_MULTIPART_PARTS`]".
/// Setting `usize::MAX` disables the ceiling.
///
/// Thread-safe; the most recent value wins for any subsequent request.
pub fn set_global_max_multipart_parts(parts: usize) {
    GLOBAL_MAX_PARTS.store(parts, Ordering::SeqCst);
}

/// Read the currently-configured part ceiling. Returns the default if
/// [`set_global_max_multipart_parts`] has never been called or was called
/// with `0`.
pub fn global_max_multipart_parts() -> usize {
    let stored = GLOBAL_MAX_PARTS.load(Ordering::SeqCst);
    if stored == 0 {
        DEFAULT_MAX_MULTIPART_PARTS
    } else {
        stored
    }
}

/// Number of multipart parts that have spilled from memory to a temp file
/// since process start.
///
/// A monotonically increasing process-global counter, useful as an
/// upload-pressure signal. Oversized *text* parts are rejected with HTTP
/// 413 at the in-memory spill threshold and never spill, so this counter
/// reflects only file parts that legitimately exceeded the threshold.
pub fn upload_tempfiles_spilled_total() -> usize {
    UPLOAD_TEMPFILES_SPILLED.load(Ordering::SeqCst)
}

/// Limits enforced while streaming a multipart body into a
/// [`MultipartPayload`].
///
/// Construct one and hand it to [`parse_multipart_streaming_with_limits`].
/// The [`parse_multipart_streaming`] convenience wrapper fills it from the
/// process-global accessors; `#[derive(MultipartRequest)]` fills it from
/// the per-struct `#[multipart(...)]` / `#[field(..., max_count = N)]`
/// attributes.
pub struct MultipartLimits<'a> {
    /// Hard ceiling on total accumulated body bytes across all parts.
    /// Exceeding it returns HTTP 413; a declared `Content-Length` above it
    /// is rejected before any body byte is read.
    pub max_body_bytes: usize,
    /// Hard ceiling on the number of parts. The (ceiling + 1)-th part is
    /// rejected with HTTP 413 before it is read, bounding allocation
    /// against many-tiny-parts flooding.
    pub max_parts: usize,
    /// Per-part in-memory byte threshold before a file part spills to a
    /// temp file. A text part that crosses this threshold is rejected with
    /// HTTP 413 rather than spilled, at the chunk that crossed it: like the
    /// byte cap, it bounds the request's memory, so it is not a field
    /// error.
    pub spill_threshold: usize,
    /// Per-field count ceilings keyed by wire field name. When a field
    /// reaches its ceiling, the next part carrying that name is rejected
    /// with HTTP 413 before it is read - so the (ceiling + 1)-th part
    /// never allocates. Like the other two ceilings it bounds the whole
    /// request, so it answers 413 rather than a field error. Names absent
    /// from this list are bounded only by `max_parts`.
    pub per_field_max_counts: &'a [(&'a str, usize)],
}

/// Underlying storage for an [`UploadedFile`] part.
///
/// Pre-allocated by the multipart parser based on whether the part
/// crossed the spill threshold. End users construct `UploadedFile` via
/// the parser + derive macro - they don't build this enum directly.
#[doc(hidden)]
pub enum UploadedFileBacking {
    /// Small part - buffered entirely in memory.
    Memory(Bytes),
    /// Large part - written to a temp file as the body streamed in.
    /// The temp file is auto-deleted when this enum drops, so partial
    /// uploads abandoned mid-request never accumulate on disk.
    Disk(NamedTempFile),
}

/// A single uploaded file with associated validator `V`.
///
/// Backed either by an in-memory `Bytes` (for parts below the spill
/// threshold) or a `tempfile::NamedTempFile` (for larger parts streamed
/// to disk as they arrived). Use `UploadedFile::store_as` to write
/// to a registered storage disk - that path is fully streaming for
/// disk-backed parts and a single-op write for in-memory parts.
///
/// To inspect the raw bytes, call [`UploadedFile::bytes`] (async - the
/// disk-backed path reads asynchronously). For size checks, prefer the
/// synchronous [`UploadedFile::size`] accessor.
pub struct UploadedFile<V: UploadValidator = ()> {
    backing: UploadedFileBacking,
    /// Total size of the part in bytes. Pre-computed during parsing so
    /// callers (and the `after_validation` sync hook) can size-check
    /// without doing async I/O.
    pub size: u64,
    /// File extension inferred from magic bytes captured during parse
    /// (a bounded ≤16 KiB sniff buffer). `None` when the format is
    /// unknown; callers should fall back to `"bin"` - the
    /// [`UploadedFile::extension_from_magic`] helper does exactly that.
    inferred_extension: Option<&'static str>,
    /// Original filename declared by the client, if any. Untrusted input - never use as a filesystem path.
    pub file_name: Option<String>,
    /// `Content-Type` the client declared for this part, if any. Untrusted - verify against `inferred_extension` for security-sensitive paths.
    pub content_type: Option<String>,
    _v: std::marker::PhantomData<V>,
}

impl<V: UploadValidator> UploadedFile<V> {
    /// Internal: construct an `UploadedFile` backed by an in-memory
    /// `Bytes`. Called by the derive macro after the parser handed it a
    /// `MultipartValue::File` for a part that stayed under the spill
    /// threshold.
    #[doc(hidden)]
    pub fn from_memory(
        bytes: Bytes,
        file_name: Option<String>,
        content_type: Option<String>,
        inferred_extension: Option<&'static str>,
    ) -> Self {
        let size = bytes.len() as u64;
        Self {
            backing: UploadedFileBacking::Memory(bytes),
            size,
            inferred_extension,
            file_name,
            content_type,
            _v: std::marker::PhantomData,
        }
    }

    /// Internal: construct an `UploadedFile` backed by a temp file on
    /// disk. Called by the derive macro after the parser handed it a
    /// `MultipartValue::File` for a part that exceeded the spill
    /// threshold.
    #[doc(hidden)]
    pub fn from_disk(
        temp: NamedTempFile,
        size: u64,
        file_name: Option<String>,
        content_type: Option<String>,
        inferred_extension: Option<&'static str>,
    ) -> Self {
        Self {
            backing: UploadedFileBacking::Disk(temp),
            size,
            inferred_extension,
            file_name,
            content_type,
            _v: std::marker::PhantomData,
        }
    }

    /// Read the entire upload into memory.
    ///
    /// For in-memory parts this is a cheap `Bytes::clone()`. For
    /// disk-backed parts this asynchronously reads the temp file - so
    /// it allocates `size` bytes plus reads `size` bytes from disk.
    /// Prefer `UploadedFile::store_as` whenever the destination is a
    /// storage disk: that path streams in 64 KiB chunks and never
    /// holds the full upload in RAM.
    ///
    /// # Errors
    ///
    /// Returns [`FrameworkError::Internal`] if the disk-backed read
    /// fails (e.g. the temp file was deleted out from under us, or the
    /// process lost permissions). In-memory reads are infallible.
    pub async fn bytes(&self) -> Result<Bytes, FrameworkError> {
        match &self.backing {
            UploadedFileBacking::Memory(b) => Ok(b.clone()),
            UploadedFileBacking::Disk(temp) => {
                let path = temp.path().to_owned();
                let data = tokio::fs::read(&path).await.map_err(|e| {
                    FrameworkError::internal(format!("read uploaded temp file: {e}"))
                })?;
                Ok(Bytes::from(data))
            }
        }
    }

    /// Stream the upload directly to a storage disk.
    ///
    /// For in-memory parts: a single `Operator::write` call.
    ///
    /// For disk-backed parts: open the temp file with `tokio::fs::File`,
    /// open an `Operator::writer` on the destination, and copy 64 KiB
    /// chunks until EOF. The destination writer is explicitly closed so
    /// backends that finalise on close (S3 multipart, Azure block blob)
    /// commit the object before this method returns.
    ///
    /// # Errors
    ///
    /// Returns [`FrameworkError::Internal`] on any I/O failure - opening
    /// the temp file, reading from it, opening the destination writer,
    /// writing a chunk, or closing the destination writer. Each path
    /// uses a distinct message prefix so failures are identifiable in
    /// structured logs.
    #[cfg(feature = "filesystem")]
    pub async fn store_as(
        &self,
        disk: &opendal::Operator,
        path: &str,
    ) -> Result<(), FrameworkError> {
        match &self.backing {
            UploadedFileBacking::Memory(bytes) => {
                disk.write(path, bytes.clone())
                    .await
                    .map_err(|e| FrameworkError::internal(format!("storage write: {e}")))?;
            }
            UploadedFileBacking::Disk(temp) => {
                let mut reader = tokio::fs::File::open(temp.path()).await.map_err(|e| {
                    FrameworkError::internal(format!("open uploaded temp file: {e}"))
                })?;
                let mut writer = disk
                    .writer(path)
                    .await
                    .map_err(|e| FrameworkError::internal(format!("open storage writer: {e}")))?;
                let mut buf = vec![0u8; STORE_AS_CHUNK_BYTES];
                loop {
                    let n = reader.read(&mut buf).await.map_err(|e| {
                        FrameworkError::internal(format!("read uploaded temp file: {e}"))
                    })?;
                    if n == 0 {
                        break;
                    }
                    writer
                        .write(Bytes::copy_from_slice(&buf[..n]))
                        .await
                        .map_err(|e| FrameworkError::internal(format!("storage write: {e}")))?;
                }
                writer
                    .close()
                    .await
                    .map_err(|e| FrameworkError::internal(format!("storage close: {e}")))?;
            }
        }
        Ok(())
    }

    /// Return the canonical file extension derived from the **content's**
    /// magic bytes captured during parsing (a bounded ≤16 KiB sniff
    /// buffer), NOT from the client-supplied filename. Returns `"bin"`
    /// when the content type cannot be identified (binary blobs,
    /// unrecognised formats).
    ///
    /// Synchronous - the extension is pre-computed during multipart
    /// parsing, so this never re-reads the spilled temp file.
    ///
    /// # Why this matters
    ///
    /// The client-supplied filename is untrusted. A request like
    /// `avatar=@evil.exe` where the body is real PNG bytes would
    /// otherwise be stored with a `.exe` extension if the storage path
    /// is derived from `file_name`. Use this method whenever the path
    /// you write to disk is content-addressed rather than caller-named.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # use suprnova::http::upload::UploadedFile;
    /// # #[cfg(feature = "filesystem")]
    /// # async fn ex(file: UploadedFile, disk: suprnova::opendal::Operator, user_id: u64)
    /// #     -> Result<(), Box<dyn std::error::Error>> {
    /// let path = format!("avatars/{}.{}", user_id, file.extension_from_magic());
    /// file.store_as(&disk, &path).await?;
    /// # Ok(()) }
    /// ```
    pub fn extension_from_magic(&self) -> &'static str {
        self.inferred_extension.unwrap_or("bin")
    }
}

/// Order-preserving list of fields from a multipart body. Duplicate
/// names survive intact (for `photos[]`-style array uploads).
#[derive(Default)]
pub struct MultipartPayload {
    /// Parsed multipart parts in submission order. Duplicate names are preserved for `name[]`-style array uploads.
    pub fields: Vec<(String, MultipartValue)>,
}

/// A parsed multipart field - either a file part (whose backing is
/// either in-memory or a disk-spilled temp file) or a text part.
pub enum MultipartValue {
    /// File part. `backing` decides where the content lives; `size`
    /// is the byte count; `inferred_extension` is the result of
    /// `infer::get` over the bounded sniff buffer captured during parse -
    /// pre-computed so the derive macro can hand it to
    /// [`UploadedFile::from_memory`] / [`UploadedFile::from_disk`]
    /// without re-reading the spilled temp file. `sniff` is the same
    /// bounded ≤16 KiB prefix captured during parse, surfaced here so
    /// `validate_final` callers (the derive macro, primarily) can run
    /// content-aware checks without re-reading the spilled file.
    File {
        /// Where the bytes live - in-memory `Bytes` or a disk-spilled temp file.
        backing: UploadedFileBacking,
        /// Total size of the part in bytes.
        size: u64,
        /// Client-declared filename, if any. Untrusted input - never use as a filesystem path.
        file_name: Option<String>,
        /// Client-declared `Content-Type`, if any. Untrusted - cross-check with `inferred_extension`.
        content_type: Option<String>,
        /// Extension inferred from the magic-byte sniff buffer; `None` when the format is unknown.
        inferred_extension: Option<&'static str>,
        /// Bounded ≤16 KiB prefix captured during parse, for content-aware validators.
        sniff: Vec<u8>,
    },
    /// Text part - a non-file field carrying its UTF-8 value.
    Text(String),
    /// Text part whose bytes are not UTF-8, as sent. Kept apart from
    /// `Text` rather than refused, because the parser cannot tell which key
    /// the failure belongs under: the extractor reports it under the
    /// field's input name with its type's key, as it does text that does
    /// not parse.
    NonUtf8Text(Vec<u8>),
}

/// Internal: what `collect_part` read from one part. A text part never
/// leaves memory, so only a file part carries a backing that may be a temp
/// file.
enum Collected {
    /// A text part's bytes, before they are checked as UTF-8.
    Text(Vec<u8>),
    /// A file part.
    File(CollectedPart),
}

/// Internal: a file part as the parser read it, before it becomes a
/// `MultipartValue::File`. Keeps the inner-loop signature small.
struct CollectedPart {
    backing: PartBacking,
    size: u64,
    sniff: Vec<u8>,
    inferred_extension: Option<&'static str>,
}

/// Internal: the byte buffer underlying a `CollectedPart`. Either an
/// in-memory `Vec<u8>` (for small parts) or a `NamedTempFile` (for
/// spilled parts). Converted to [`UploadedFileBacking`] by the parse loop.
enum PartBacking {
    Memory(Vec<u8>),
    Disk(NamedTempFile),
}

/// Translate a multer error into a `FrameworkError`, distinguishing "the
/// client sent something malformed" (400) from "we cut the stream off
/// ourselves for exceeding the raw byte cap" (413).
///
/// Needed because multer reports a failure in the underlying stream as an
/// opaque read error - by the time it surfaces here, the fact that *we*
/// produced it is gone. `tripped` carries that one bit back out.
fn multipart_stream_error(
    err: multer::Error,
    tripped: &AtomicBool,
    cap: usize,
    stage: &str,
) -> FrameworkError {
    if tripped.load(Ordering::Relaxed) {
        return FrameworkError::Domain {
            message: format!("multipart body exceeds {cap} bytes (cap)"),
            status_code: 413,
        };
    }
    FrameworkError::Domain {
        message: format!("multipart {stage}: {err}"),
        status_code: 400,
    }
}

/// The request-wide byte budget, threaded through part collection.
///
/// These three always travel together - the ceiling, the running total,
/// and the flag saying we already cut the transport off for exceeding it -
/// so they are one parameter rather than three.
struct BodyBudget<'a> {
    /// Ceiling on accumulated payload bytes across all parts.
    cap: usize,
    /// Running total, shared across every part in this request.
    used: &'a mut usize,
    /// Set when the raw-stream counter tripped; see
    /// [`multipart_stream_error`].
    raw_cap_tripped: &'a AtomicBool,
}

/// Stream a single part out of `field`, spilling a file part to a temp file
/// once the accumulated buffer crosses `spill_threshold` bytes.
///
/// Updates `*budget.used` after each chunk and short-circuits with a 413 if
/// the running total exceeds `budget.cap`, or if a text part crosses
/// `spill_threshold`. Validators see the bounded sniff buffer + current
/// accumulated size and may also short-circuit.
async fn collect_part<F>(
    field: &mut multer::Field<'_>,
    name: &str,
    per_field_validator: &mut F,
    spill_threshold: usize,
    budget: &mut BodyBudget<'_>,
    is_text: bool,
    check_chunks: bool,
) -> Result<Collected, FrameworkError>
where
    F: FnMut(&str, &[u8], u64) -> Result<(), FrameworkError>,
{
    let mut mem: Vec<u8> = Vec::new();
    let mut spill: Option<(NamedTempFile, tokio::fs::File)> = None;
    let mut size: u64 = 0;
    let mut sniff: Vec<u8> = Vec::with_capacity(SNIFF_BYTES.min(spill_threshold + 1));

    while let Some(chunk) = field
        .chunk()
        .await
        .map_err(|e| multipart_stream_error(e, budget.raw_cap_tripped, budget.cap, "chunk"))?
    {
        size = size.saturating_add(chunk.len() as u64);

        // Global body cap. `saturating_add` guards against `usize`
        // wraparound on pathologically large streams.
        *budget.used = budget.used.saturating_add(chunk.len());
        if *budget.used > budget.cap {
            return Err(FrameworkError::Domain {
                message: format!("multipart body exceeds {} bytes (cap)", budget.cap),
                status_code: 413,
            });
        }

        // Capture sniff bytes up to SNIFF_BYTES. Once the buffer is
        // full, additional chunks contribute nothing to it - bound is
        // hard so a 200 MiB upload's sniff stays at 16 KiB.
        let remaining_sniff = SNIFF_BYTES.saturating_sub(sniff.len());
        if remaining_sniff > 0 {
            let take = remaining_sniff.min(chunk.len());
            sniff.extend_from_slice(&chunk[..take]);
        }

        match &mut spill {
            None => {
                // In-memory fast path. Once `mem` crosses
                // `spill_threshold`, drain into a fresh temp file and
                // switch backing for every subsequent chunk.
                mem.extend_from_slice(&chunk);
                if mem.len() > spill_threshold {
                    // A text part must fit in memory: the spill threshold
                    // is a sizing hint for opaque file payloads, not for
                    // arbitrary form fields. Refuse an oversized text part
                    // here, the moment it crosses the threshold, instead
                    // of streaming the rest to a temp file only to refuse
                    // it after the part is fully consumed. It bounds the
                    // request's memory, as the body cap does, so it is a
                    // request-wide 413 rather than a field error; the body
                    // cap, checked above, answers when one chunk crosses
                    // both.
                    if is_text {
                        return Err(FrameworkError::Domain {
                            message: format!(
                                "text field '{name}' exceeds the {spill_threshold}-byte in-memory limit (cap)"
                            ),
                            status_code: 413,
                        });
                    }
                    UPLOAD_TEMPFILES_SPILLED.fetch_add(1, Ordering::SeqCst);
                    let temp = tempfile::Builder::new()
                        .prefix(SPILL_FILE_PREFIX)
                        .tempfile()
                        .map_err(|e| {
                            FrameworkError::internal(format!("create upload tempfile: {e}"))
                        })?;
                    let mut writer = tokio::fs::File::create(temp.path()).await.map_err(|e| {
                        FrameworkError::internal(format!("open upload tempfile: {e}"))
                    })?;
                    writer.write_all(&mem).await.map_err(|e| {
                        FrameworkError::internal(format!("spill upload tempfile: {e}"))
                    })?;
                    mem.clear();
                    spill = Some((temp, writer));
                }
            }
            Some((_, writer)) => {
                writer
                    .write_all(&chunk)
                    .await
                    .map_err(|e| FrameworkError::internal(format!("write upload tempfile: {e}")))?;
            }
        }

        // Validator callback. Streaming-aware signature: bounded sniff
        // buffer + total accumulated size. Validators that care about
        // content (ImageFile) consult sniff; validators that care about
        // size (MaxSize) consult size. Fires AFTER the body cap so a
        // 413 from the cap takes precedence when one chunk crosses both.
        // Returning here reads no further chunk, and drops the spill file
        // this part was writing. Only for a part the caller checks: see
        // [`Checked`].
        if check_chunks {
            per_field_validator(name, &sniff, size)?;
        }
    }

    if is_text {
        // The loop refuses a text part at the threshold, before it could
        // spill, so all of its bytes are in `mem`.
        return Ok(Collected::Text(mem));
    }

    let inferred_extension = if sniff.is_empty() {
        None
    } else {
        infer::get(&sniff).map(|k| k.extension())
    };

    let backing = if let Some((temp, mut writer)) = spill {
        writer
            .flush()
            .await
            .map_err(|e| FrameworkError::internal(format!("flush upload tempfile: {e}")))?;
        // Drop the writer so the OS file handle closes before the
        // consumer (`store_as` / `bytes()`) re-opens the path. Saves an
        // edge case where buffered writes haven't flushed yet on some
        // platforms.
        drop(writer);
        PartBacking::Disk(temp)
    } else {
        PartBacking::Memory(mem)
    };

    Ok(Collected::File(CollectedPart {
        backing,
        size,
        sniff,
        inferred_extension,
    }))
}

/// Stream the body of `req` into a `MultipartPayload`, capped at
/// `max_body_bytes` total accumulated bytes across all parts, bounded to
/// `max_parts` parts, spilling file parts above `spill_threshold` to temp
/// files, and enforcing any per-field `max_count` ceilings in
/// `per_field_max_counts` during streaming.
///
/// The `per_field_validator` callback fires after each chunk with
/// `(field_name, sniff_buffer, total_size_so_far)`. Validators may
/// short-circuit oversized or wrong-content fields at the chunk
/// boundary.
///
/// The total-body cap is enforced BEFORE `per_field_validator` runs,
/// so it fires even when no validator has been configured for the
/// field (e.g. `UploadedFile<()>` or plain `Option<String>` fields).
///
/// A text part whose bytes are not UTF-8 is not an error here: it arrives
/// as [`MultipartValue::NonUtf8Text`], so the caller, which knows the
/// field's type, decides how to report it.
///
/// # Errors
///
/// - 400 if the request is malformed (missing content-type, bad boundary)
/// - 413 if a declared `Content-Length`, the accumulated body size, the
///   number of parts, or the parts of one field (`per_field_max_counts`)
///   exceed the configured ceiling, or a text part exceeds
///   `spill_threshold`, before the body is read further
/// - 422 [`FrameworkError::Validation`] when `per_field_validator` refuses
///   a file with [`FrameworkError::invalid_upload`]: the message goes under
///   the part's input name, a trailing `[]` replaced by the part's
///   zero-based index among the parts of its name, and the body is read no
///   further
/// - Any other error `per_field_validator` returns, unchanged
/// - 500 for I/O failures spilling to / writing the temp file
///
/// On any error, every temp file the parse wrote is removed before this
/// returns.
pub async fn parse_multipart_streaming_with_limits<F>(
    req: crate::http::Request,
    limits: MultipartLimits<'_>,
    per_field_validator: F,
) -> Result<MultipartPayload, FrameworkError>
where
    F: FnMut(&str, &[u8], u64) -> Result<(), FrameworkError>,
{
    parse_parts(req, limits, Checked::EveryPart, per_field_validator).await
}

/// The parse `#[derive(MultipartRequest)]` runs: as
/// [`parse_multipart_streaming_with_limits`], except that
/// `per_file_validator` sees only the parts the extractor takes as files,
/// so the check of a field's file can fail the request only for a file
/// that field would hold.
///
/// A text part is never checked as a file: the extractor reports it where
/// a file belongs (`validation-file`). A field that holds one file, named
/// in `single_files`, takes the first part of its name that does not leave
/// the file out; every later part of that name is skipped, neither checked
/// nor kept, so it never reaches memory or a temp file.
#[doc(hidden)]
pub async fn parse_multipart_for_extractor<F>(
    req: crate::http::Request,
    limits: MultipartLimits<'_>,
    single_files: &[&str],
    per_file_validator: F,
) -> Result<MultipartPayload, FrameworkError>
where
    F: FnMut(&str, &[u8], u64) -> Result<(), FrameworkError>,
{
    parse_parts(
        req,
        limits,
        Checked::TakenFiles { single_files },
        per_file_validator,
    )
    .await
}

/// Which parts the chunk validator is called for.
enum Checked<'a> {
    /// Every part, text or file: the contract of the public parsers, whose
    /// caller sees the wire name alone.
    EveryPart,
    /// The file parts the extractor takes (see
    /// [`parse_multipart_for_extractor`]).
    TakenFiles {
        /// The wire names of the fields that hold one file.
        single_files: &'a [&'a str],
    },
}

async fn parse_parts<F>(
    req: crate::http::Request,
    limits: MultipartLimits<'_>,
    checked: Checked<'_>,
    mut per_field_validator: F,
) -> Result<MultipartPayload, FrameworkError>
where
    F: FnMut(&str, &[u8], u64) -> Result<(), FrameworkError>,
{
    let MultipartLimits {
        max_body_bytes,
        max_parts,
        spill_threshold,
        per_field_max_counts,
    } = limits;

    let content_type = req
        .content_type()
        .ok_or_else(|| FrameworkError::Domain {
            message: "missing content-type".into(),
            status_code: 400,
        })?
        .to_string();
    let boundary = multer::parse_boundary(&content_type).map_err(|e| FrameworkError::Domain {
        message: format!("invalid multipart boundary: {e}"),
        status_code: 400,
    })?;

    // Pre-reject an honestly-declared oversized body before reading a
    // single frame, mirroring the generic body path (`body_bytes_with_cap`).
    // A client that lies (small Content-Length, large body) is still caught
    // progressively by the per-chunk byte cap inside `collect_part`.
    if let Some(declared) = req
        .header("content-length")
        .and_then(|v| v.parse::<u64>().ok())
        && declared > max_body_bytes as u64
    {
        return Err(FrameworkError::Domain {
            message: format!("multipart body exceeds {max_body_bytes} bytes (cap)"),
            status_code: 413,
        });
    }

    let (_parts, body) = req.into_parts();
    // `BodyStream` would yield `Result<Frame<Bytes>, _>` and `Frame<Bytes>`
    // does not impl `Into<Bytes>` (multer's bound). `BodyDataStream` drops
    // trailer frames and yields `Result<Bytes, hyper::Error>` directly,
    // which is exactly what multer wants.
    //
    // Multipart bodies are never pre-buffered by middleware (CSRF only
    // buffers form-urlencoded). If we somehow see a buffered body here
    // it's a programming error - return a clear 400 rather than
    // silently truncating.
    let incoming = match body {
        crate::http::BodyState::Streaming(inc) => inc,
        crate::http::BodyState::Buffered(_) => {
            return Err(FrameworkError::Domain {
                message: "multipart upload received a pre-buffered body - this is a \
                          framework bug; multipart bodies must arrive as streams"
                    .into(),
                status_code: 400,
            });
        }
        crate::http::BodyState::Consumed => {
            return Err(FrameworkError::Domain {
                message: "multipart upload received a fully consumed body - middleware \
                          drained the body without buffering"
                    .into(),
                status_code: 400,
            });
        }
    };
    // SEC-05: cap the RAW stream, not just the bytes that reach a part.
    //
    // `total_bytes` below only accumulates payload bytes handed back by
    // `field.chunk()`. Multipart framing - the preamble before the first
    // boundary, boundary lines, and part headers - never reaches that
    // counter, and `max_parts` only counts parts multer actually yields.
    // So a body that is *entirely* framing trips neither cap: a gigantic
    // preamble, or a part header that never terminates, keeps multer
    // buffering while `total_bytes` and `part_count` both sit at zero.
    //
    // The `Content-Length` pre-check above catches an honest client, but a
    // chunked body declares no length at all, which is exactly why this was
    // reachable in practice.
    //
    // Counting at the transport closes all of those at once, because every
    // byte on the wire passes through here whether or not multer ever
    // attributes it to a part.
    let raw_cap_tripped = Arc::new(AtomicBool::new(false));
    let counted = {
        let tripped = raw_cap_tripped.clone();
        let cap = max_body_bytes as u64;
        let mut raw_seen: u64 = 0;
        BodyDataStream::new(incoming).map(move |chunk| match chunk {
            Ok(bytes) => {
                raw_seen = raw_seen.saturating_add(bytes.len() as u64);
                if raw_seen > cap {
                    // multer surfaces this as an opaque stream-read failure,
                    // so record the cause out-of-band: the error sites below
                    // consult this to answer 413 rather than a misleading 400.
                    tripped.store(true, Ordering::Relaxed);
                    Err(std::io::Error::other(format!(
                        "multipart raw body exceeds {cap} bytes (cap)"
                    )))
                } else {
                    Ok(bytes)
                }
            }
            Err(e) => Err(std::io::Error::other(e)),
        })
    };
    let mut multipart = Multipart::new(counted, boundary);

    let mut payload = MultipartPayload::default();
    let mut total_bytes: usize = 0;

    // Per-field count ceilings (`max_count`) enforced during streaming.
    // `cap_for` maps a field's wire name to its ceiling; `seen_for` counts
    // the parts seen per name, which is also each part's zero-based index
    // among the parts of its name (`files[]` -> `files.1`). Its size is
    // bounded by `max_parts`.
    let cap_for: std::collections::HashMap<&str, usize> =
        per_field_max_counts.iter().copied().collect();
    let mut seen_for: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut part_count: usize = 0;
    // Under `Checked::TakenFiles`, whether each field that holds one file
    // has taken its part, by its position in `single_files`.
    let mut file_taken = match &checked {
        Checked::EveryPart => Vec::new(),
        Checked::TakenFiles { single_files } => vec![false; single_files.len()],
    };

    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|e| multipart_stream_error(e, &raw_cap_tripped, max_body_bytes, "parse"))?
    {
        let name = field.name().unwrap_or_default().to_string();
        let file_name = field.file_name().map(|s| s.to_string());
        let mime = field.content_type().map(|m| m.to_string());

        // Bound the total part count before reading the part. The byte cap
        // limits raw payload bytes but not the *number* of parts (multipart
        // framing bytes aren't counted toward it), so without this a body
        // of many tiny parts could grow `payload.fields` unbounded within
        // the byte budget. Reject the (max_parts + 1)-th part before it
        // allocates.
        part_count += 1;
        if part_count > max_parts {
            return Err(FrameworkError::Domain {
                message: format!("multipart request exceeds {max_parts} parts (cap)"),
                status_code: 413,
            });
        }

        // Clones the name only for its first part.
        let index = match seen_for.get_mut(name.as_str()) {
            Some(seen) => {
                let index = *seen;
                *seen += 1;
                index
            }
            None => {
                seen_for.insert(name.clone(), 1);
                0
            }
        };

        // Per-field `max_count`: reject the (ceiling + 1)-th part carrying
        // this name before it is read, so the extra part never allocates.
        if let Some(&cap) = cap_for.get(name.as_str())
            && index >= cap
        {
            return Err(FrameworkError::Domain {
                message: format!("field '{name}' exceeds max_count {cap}"),
                status_code: 413,
            });
        }

        // Which chunks the validator sees, and, for a field that holds one
        // file, skip a part once the field has taken one: multer passes over
        // the part's bytes, which still count toward the raw byte cap.
        let (check_chunks, single_file) = match &checked {
            Checked::EveryPart => (true, None),
            Checked::TakenFiles { single_files } => {
                let single_file = single_files.iter().position(|single| *single == name);
                if single_file.is_some_and(|at| file_taken.get(at).copied().unwrap_or(false)) {
                    continue;
                }
                (file_name.is_some(), single_file)
            }
        };

        // Classification: presence of `filename=` in Content-Disposition
        // is the canonical marker of a file part. Text parts may carry
        // a `Content-Type`, so we don't use `mime.is_some()` as the
        // discriminator.
        let collected = collect_part(
            &mut field,
            &name,
            &mut per_field_validator,
            spill_threshold,
            &mut BodyBudget {
                cap: max_body_bytes,
                used: &mut total_bytes,
                raw_cap_tripped: &raw_cap_tripped,
            },
            file_name.is_none(),
            check_chunks,
        )
        .await
        .map_err(|err| match err {
            FrameworkError::InvalidUpload(message) => {
                let mut errors = ValidationErrors::new();
                errors.add(field_error_key(&name, Some(index)), *message);
                FrameworkError::validation_errors(errors)
            }
            other => other,
        })?;

        let value = match collected {
            Collected::File(part) => {
                let backing = match part.backing {
                    PartBacking::Memory(v) => UploadedFileBacking::Memory(Bytes::from(v)),
                    PartBacking::Disk(t) => UploadedFileBacking::Disk(t),
                };
                MultipartValue::File {
                    backing,
                    size: part.size,
                    file_name,
                    content_type: mime,
                    inferred_extension: part.inferred_extension,
                    sniff: part.sniff,
                }
            }
            Collected::Text(bytes) => match String::from_utf8(bytes) {
                Ok(text) => MultipartValue::Text(text),
                Err(not_utf8) => MultipartValue::NonUtf8Text(not_utf8.into_bytes()),
            },
        };

        // The field takes the first part that does not leave its file out,
        // as `take_file` reads it.
        if let Some(at) = single_file
            && !leaves_file_out(&value)
            && let Some(taken) = file_taken.get_mut(at)
        {
            *taken = true;
        }

        payload.fields.push((name, value));
    }

    Ok(payload)
}

/// Stream the body of `req` into a `MultipartPayload`, invoking
/// `per_field_validator(name, sniff, size)` after each chunk so the
/// caller can short-circuit oversized parts at byte boundaries.
///
/// Stream the body of `req` into a [`MultipartPayload`], capped at
/// `max_body_bytes` total bytes and spilling file parts above
/// `spill_threshold` to temp files.
///
/// A convenience over [`parse_multipart_streaming_with_limits`] for callers
/// that only need to pin the byte cap and spill threshold: the part-count
/// ceiling defaults to [`global_max_multipart_parts`] and no per-field
/// `max_count` ceilings are applied. `Content-Length` pre-rejection and the
/// global part-count cap still apply.
pub async fn parse_multipart_streaming_with_cap<F>(
    req: crate::http::Request,
    max_body_bytes: usize,
    spill_threshold: usize,
    per_field_validator: F,
) -> Result<MultipartPayload, FrameworkError>
where
    F: FnMut(&str, &[u8], u64) -> Result<(), FrameworkError>,
{
    parse_multipart_streaming_with_limits(
        req,
        MultipartLimits {
            max_body_bytes,
            max_parts: global_max_multipart_parts(),
            spill_threshold,
            per_field_max_counts: &[],
        },
        per_field_validator,
    )
    .await
}

/// Thin wrapper around [`parse_multipart_streaming_with_limits`] that
/// fills [`MultipartLimits`] from the process-global accessors
/// ([`global_max_multipart_body_bytes`], [`global_max_multipart_parts`],
/// [`global_upload_spill_threshold`]) and applies no per-field count
/// ceilings. Callers that need to pin limits to known values - or enforce
/// per-field `max_count` - should call
/// [`parse_multipart_streaming_with_limits`] directly;
/// `#[derive(MultipartRequest)]` does exactly that.
pub async fn parse_multipart_streaming<F>(
    req: crate::http::Request,
    per_field_validator: F,
) -> Result<MultipartPayload, FrameworkError>
where
    F: FnMut(&str, &[u8], u64) -> Result<(), FrameworkError>,
{
    parse_multipart_streaming_with_limits(
        req,
        MultipartLimits {
            max_body_bytes: global_max_multipart_body_bytes(),
            max_parts: global_max_multipart_parts(),
            spill_threshold: global_upload_spill_threshold(),
            per_field_max_counts: &[],
        },
        per_field_validator,
    )
    .await
}

/// Lifecycle hooks for multipart request structs. Mirrors
/// `FormRequest::authorize` / `after_validation` /
/// `after_validation_async` so users have one mental model.
///
/// `#[derive(MultipartRequest)]` emits an empty `impl MultipartRequestHooks for MyStruct {}`
/// unless the struct carries `#[multipart(custom_hooks)]`. With
/// `custom_hooks`, the user provides the impl themselves; it needs
/// `#[async_trait]` only when it overrides `after_validation_async`.
///
/// # Stage order
///
/// The extractor runs `authorize`, before any byte of the body is read;
/// then the extraction, with each field's validation; then
/// `after_validation`; then `after_validation_async`; then the handler.
/// Each stage runs only after the one before it succeeded, so a database
/// check in the async hook never sees a malformed or missing field.
///
/// # Hook errors
///
/// A hook's non-empty `ValidationErrors` answers 422 with `errors`, which
/// `InertiaValidationRedirectMiddleware` turns into a redirect back with the
/// errors for an Inertia form. An empty set counts as success. Errors use
/// the input names extraction errors use: a key that starts with a Rust
/// field name is reported under that field's `#[field(...)]` name, with a
/// trailing `[]` dropped, so `photos.1` on a field read from `photos[]` is
/// the second photo.
#[async_trait::async_trait]
pub trait MultipartRequestHooks {
    /// Called BEFORE the body is consumed. Return `false` to short-circuit
    /// with `FrameworkError::Unauthorized` (maps to HTTP 403 in this codebase).
    fn authorize(_req: &crate::http::Request) -> bool {
        true
    }

    /// Called AFTER the struct is fully constructed and every field passed
    /// its own validation. Return a non-empty `Err(ValidationErrors)` to
    /// surface cross-field validation failures as a 422 response.
    fn after_validation(&self) -> Result<(), crate::error::ValidationErrors> {
        Ok(())
    }

    /// Async cross-field hook, run after [`after_validation`] succeeded and
    /// before the handler: the place for database checks such as `Unique`
    /// or `Exists`, which need `.await`. Return a non-empty
    /// `Err(ValidationErrors)` to answer 422.
    ///
    /// ```rust,no_run
    /// use suprnova::http::upload::{MultipartRequestHooks, UploadedFile};
    /// use suprnova::{AsyncRule, MultipartRequest, Unique, ValidationErrors, async_trait};
    ///
    /// #[derive(MultipartRequest)]
    /// #[multipart(custom_hooks)]
    /// pub struct NewAlbum {
    ///     #[field("slug")]
    ///     pub slug: String,
    ///     #[field("photos[]")]
    ///     pub photos: Vec<UploadedFile>,
    /// }
    ///
    /// #[async_trait]
    /// impl MultipartRequestHooks for NewAlbum {
    ///     async fn after_validation_async(&self) -> Result<(), ValidationErrors> {
    ///         let mut errs = ValidationErrors::new();
    ///         Unique::new("albums", "slug")
    ///             .check_async(&self.slug, &mut errs, "slug")
    ///             .await;
    ///         errs.into_result()
    ///     }
    /// }
    /// ```
    ///
    /// The default succeeds.
    ///
    /// [`after_validation`]: Self::after_validation
    async fn after_validation_async(&self) -> Result<(), crate::error::ValidationErrors> {
        Ok(())
    }
}

/// The key a field's validation error goes under: the form input name,
/// with a trailing `[]` replaced by the part's zero-based index among the
/// parts of that name, as Laravel names array elements (`files[]` gives
/// `files.1` for the second file). Without an index, as for a missing
/// field, the name without its `[]`.
#[doc(hidden)]
pub fn field_error_key(name: &str, index: Option<usize>) -> String {
    match (name.strip_suffix("[]"), index) {
        (Some(base), Some(index)) => format!("{base}.{index}"),
        (Some(base), None) => base.to_string(),
        (None, _) => name.to_string(),
    }
}

/// A hook error's key in input names. `names` pairs each name a hook may
/// use for a field - its Rust name, its `#[field]` name with `[]` - with
/// that field's input name; the first segment of `key` is looked up in it,
/// and a key that names no field is kept as it is.
#[doc(hidden)]
pub fn hook_error_key(key: &str, names: &[(&str, &str)]) -> String {
    let (head, rest) = match key.split_once('.') {
        Some((head, rest)) => (head, Some(rest)),
        None => (key, None),
    };
    let input = names
        .iter()
        .find(|(name, _)| *name == head)
        .map_or(head, |(_, input)| *input);
    match rest {
        Some(rest) => format!("{input}.{rest}"),
        None => input.to_string(),
    }
}

/// Why one field failed extraction, which picks the catalog key of its
/// message.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldFailure {
    /// A required field is missing (`validation-required`).
    Required,
    /// Text that does not parse as an integer type (`validation-integer`).
    Integer,
    /// Text that does not parse as a float type (`validation-numeric`).
    Numeric,
    /// Text that does not parse as `bool` (`validation-boolean`).
    Boolean,
    /// Text that does not parse as any other type (`validation-format`).
    Format,
    /// A text part where a file belongs (`validation-file`).
    File,
    /// A file part where text belongs, or a part that is not UTF-8 for a
    /// `String` field, which reads any other text (`validation-string`).
    String,
}

impl FieldFailure {
    /// The catalog-keyed message, with an English fallback naming `key`
    /// the way the catalog's `$field` does.
    fn message(self, key: &str) -> ValidationMessage {
        let field = key.replace('_', " ");
        let (catalog_key, fallback) = match self {
            Self::Required => (
                "validation-required",
                format!("The {field} field is required."),
            ),
            Self::Integer => (
                "validation-integer",
                format!("The {field} field must be an integer."),
            ),
            Self::Numeric => (
                "validation-numeric",
                format!("The {field} field must be a number."),
            ),
            Self::Boolean => (
                "validation-boolean",
                format!("The {field} field must be true or false."),
            ),
            Self::Format => (
                "validation-format",
                format!("The {field} field format is invalid."),
            ),
            Self::File => (
                "validation-file",
                format!("The {field} field must be a file."),
            ),
            Self::String => (
                "validation-string",
                format!("The {field} field must be a string."),
            ),
        };
        ValidationMessage::keyed(catalog_key).fallback(fallback)
    }
}

/// File `failure` for the part named `name` at `index` in `errors`.
#[doc(hidden)]
pub fn add_field_failure(
    errors: &mut ValidationErrors,
    name: &str,
    index: Option<usize>,
    failure: FieldFailure,
) {
    let key = field_error_key(name, index);
    let message = failure.message(&key);
    errors.add(key, message);
}

/// What one part gave the field it belongs to.
#[doc(hidden)]
pub enum Taken<T> {
    /// The field's value.
    Value(T),
    /// Nothing: the way a client leaves a file out.
    Absent,
    /// A failure, already filed in the error set.
    Invalid,
}

/// Whether `value` is how a client leaves a file out: an empty text part
/// (Inertia's `null` file) or a file part with no file name and no bytes
/// (an empty file input). One rule for both the extractor and the parser,
/// so the part the parser checks for a field that holds one file is the
/// part the extractor takes.
fn leaves_file_out(value: &MultipartValue) -> bool {
    match value {
        MultipartValue::Text(text) => text.is_empty(),
        MultipartValue::NonUtf8Text(_) => false,
        MultipartValue::File {
            size, file_name, ..
        } => *size == 0 && file_name.as_deref().is_none_or(str::is_empty),
    }
}

/// Turn one part into a file field's value.
///
/// An empty text part (Inertia's `null` file) and a file part with no file
/// name and no bytes (an empty file input) are how clients leave a file
/// out, so they are [`Taken::Absent`], never an empty file a validator
/// would refuse. Other text, UTF-8 or not, is a failure, as is a file
/// `validator` refuses with [`FrameworkError::invalid_upload`]; both are
/// filed under the part's key. Any other validator error is returned.
#[doc(hidden)]
pub fn take_file<V: UploadValidator>(
    validator: &V,
    value: MultipartValue,
    name: &str,
    index: usize,
    errors: &mut ValidationErrors,
) -> Result<Taken<UploadedFile<V>>, FrameworkError> {
    if leaves_file_out(&value) {
        return Ok(Taken::Absent);
    }
    match value {
        MultipartValue::Text(_) | MultipartValue::NonUtf8Text(_) => {
            add_field_failure(errors, name, Some(index), FieldFailure::File);
            Ok(Taken::Invalid)
        }
        MultipartValue::File {
            backing,
            size,
            file_name,
            content_type,
            inferred_extension,
            sniff,
        } => match validator.validate_final(&sniff, size, content_type.as_deref()) {
            Ok(()) => Ok(Taken::Value(match backing {
                UploadedFileBacking::Memory(bytes) => {
                    UploadedFile::from_memory(bytes, file_name, content_type, inferred_extension)
                }
                UploadedFileBacking::Disk(temp) => {
                    UploadedFile::from_disk(temp, size, file_name, content_type, inferred_extension)
                }
            })),
            Err(FrameworkError::InvalidUpload(message)) => {
                errors.add(field_error_key(name, Some(index)), *message);
                Ok(Taken::Invalid)
            }
            Err(other) => Err(other),
        },
    }
}

/// Turn one part into a text field's value: the text read by `parse`.
///
/// Empty text that `parse` cannot read is [`Taken::Absent`]: it is how
/// Inertia sends `null`, so an optional number or `bool` is `None` and a
/// required one is missing, as Laravel's `ConvertEmptyStringsToNull` makes
/// them. A type that can hold empty text, such as `String`, keeps it, as a
/// `FormRequest` does for a JSON `""` or a urlencoded `name=`. Other text
/// that does not parse files `failure`, the key for `T`'s kind, and so does
/// a part that is not UTF-8, which parses as no type; a file part files
/// [`FieldFailure::String`].
#[doc(hidden)]
pub fn take_text<T>(
    value: MultipartValue,
    name: &str,
    index: usize,
    failure: FieldFailure,
    parse: fn(&str) -> Option<T>,
    errors: &mut ValidationErrors,
) -> Taken<T> {
    match value {
        MultipartValue::Text(text) => match parse(&text) {
            Some(parsed) => Taken::Value(parsed),
            None if text.is_empty() => Taken::Absent,
            None => {
                add_field_failure(errors, name, Some(index), failure);
                Taken::Invalid
            }
        },
        MultipartValue::NonUtf8Text(_) => {
            add_field_failure(errors, name, Some(index), failure);
            Taken::Invalid
        }
        MultipartValue::File { .. } => {
            add_field_failure(errors, name, Some(index), FieldFailure::String);
            Taken::Invalid
        }
    }
}

/// Read a text field through its type's `FromStr`.
#[doc(hidden)]
pub fn parse_from_str<T: std::str::FromStr>(text: &str) -> Option<T> {
    text.parse().ok()
}

/// Read a `bool` field the way forms send one. `bool::from_str` takes only
/// `true` and `false`, but Inertia sends `1` and `0` and a checked HTML
/// checkbox sends `on`. This takes what Laravel's `boolean` rule takes
/// (`1`, `0`, `true`, `false`) plus `on` and `off`, the words in any case.
#[doc(hidden)]
pub fn parse_form_bool(text: &str) -> Option<bool> {
    match text {
        "1" => Some(true),
        "0" => Some(false),
        word if word.eq_ignore_ascii_case("true") || word.eq_ignore_ascii_case("on") => Some(true),
        word if word.eq_ignore_ascii_case("false") || word.eq_ignore_ascii_case("off") => {
            Some(false)
        }
        _ => None,
    }
}

#[cfg(test)]
mod key_tests {
    use super::*;

    #[test]
    fn an_array_name_takes_the_parts_index() {
        assert_eq!(field_error_key("files[]", Some(1)), "files.1");
        assert_eq!(field_error_key("files[]", None), "files");
        assert_eq!(field_error_key("avatar", Some(3)), "avatar");
        assert_eq!(field_error_key("avatar", None), "avatar");
    }

    #[test]
    fn a_form_bool_takes_laravels_values_and_checkbox_words() {
        for (text, value) in [
            ("1", true),
            ("0", false),
            ("true", true),
            ("FALSE", false),
            ("On", true),
            ("off", false),
        ] {
            assert_eq!(parse_form_bool(text), Some(value), "{text}");
        }
        for text in ["", "yes", "no", "2", " 1", "truee"] {
            assert_eq!(parse_form_bool(text), None, "{text:?}");
        }
    }

    #[test]
    fn empty_text_is_absent_only_for_a_type_that_cannot_hold_it() {
        let mut errors = ValidationErrors::new();
        let empty = || MultipartValue::Text(String::new());
        assert!(matches!(
            take_text(
                empty(),
                "n",
                0,
                FieldFailure::Integer,
                parse_from_str::<u32>,
                &mut errors
            ),
            Taken::Absent
        ));
        assert!(matches!(
            take_text(empty(), "s", 0, FieldFailure::Format, parse_from_str::<String>, &mut errors),
            Taken::Value(text) if text.is_empty()
        ));
        assert!(errors.is_empty(), "{errors}");
    }

    #[test]
    fn a_hook_key_moves_to_the_input_name() {
        let names = [
            ("cover", "covers"),
            ("covers[]", "covers"),
            ("caption", "title"),
        ];
        assert_eq!(hook_error_key("cover.1", &names), "covers.1");
        assert_eq!(hook_error_key("covers[].1", &names), "covers.1");
        assert_eq!(hook_error_key("caption", &names), "title");
        assert_eq!(hook_error_key("covers.1", &names), "covers.1");
        assert_eq!(hook_error_key("elsewhere.2", &names), "elsewhere.2");
    }
}
