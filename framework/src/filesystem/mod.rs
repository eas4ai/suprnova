//! Storage facade backed by [`opendal`].
//!
//! Disks are registered once at boot via `Storage::register_*` and looked up
//! by name through [`Storage::disk`]. The lookup returns the underlying
//! [`opendal::Operator`] directly, so consumers get the full streaming surface
//! ([`Operator::writer`], [`Operator::reader`], [`Operator::presign_read`],
//! [`Operator::list`], [`Operator::stat`], …) without us proxying each method.
//!
//! Drivers are first-class peers - there is no "default backend" the others
//! degrade into. `register_fs`, `register_memory`, `register_s3`,
//! `register_azblob`, and `register_gcs` each translate an explicit config
//! struct into the matching `opendal::services::*` builder.
//!
//! Azure and GCS are behind the `filesystem-azure` and `filesystem-gcs`
//! features. Both drivers pull `rsa`, which carries RUSTSEC-2023-0071 with
//! no fixed release upstream, so they are opt-in rather than a cost every
//! consumer pays. S3 is not gated - it never depended on `rsa`. The
//! rationale is in `framework/Cargo.toml` under those features.
//!
//! # Example
//!
//! ```rust,no_run
//! use suprnova::Storage;
//!
//! # async fn doc() -> Result<(), suprnova::FrameworkError> {
//! Storage::register_fs("local", "./storage")?;
//! let disk = Storage::disk("local")?;
//! disk.write("notes/hello.txt", "hello world").await?;
//! let bytes = disk.read("notes/hello.txt").await?;
//! assert_eq!(&bytes.to_vec(), b"hello world");
//! # Ok(())
//! # }
//! ```

mod disk;
mod path_guard;
mod read_through;
mod registry;
pub mod streaming;

#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use disk::{ChecksumAlgorithm, DiskExt};
pub use streaming::copy_between_disks;

use crate::FrameworkError;
use opendal::{Operator, services};
use std::path::Path;

/// Directory name reserved inside every local-filesystem disk root.
///
/// A local-filesystem disk stages every non-append write as a temp file under
/// `<root>/.suprnova-atomic/` and `rename(2)`s it onto the target, so a reader
/// never observes a half-written object and a crash never leaves a truncated
/// one. The staging directory has to live *inside* the root: a sibling of the
/// root can sit on a different filesystem when the root is a mount point, and
/// then every rename fails with `EXDEV`.
///
/// Living inside the root is why the name is reserved rather than merely
/// conventional. `Storage::disk(..)` refuses any path whose first component is
/// this name - read, write, delete, stat, list alike - so a caller can neither
/// reach into another writer's staging file nor collide with the name, and the
/// entry is filtered out of listings so it never shows up as an object.
///
/// Exported because backup and sync tooling needs to name it: the directory
/// holds only in-flight temp files, so exclude it the way you would exclude a
/// lock directory.
pub const ATOMIC_STAGING_DIR: &str = ".suprnova-atomic";

/// Build the `opendal` local-filesystem service for `root` with atomic writes
/// configured.
///
/// Shared by [`Storage::register_fs_with`] and the `read_through` tests so both
/// exercise the same staging configuration; a disk built any other way takes
/// opendal's non-atomic quick path and writes in place.
///
/// `root` must already be valid UTF-8. The staging path is `root` joined with
/// [`ATOMIC_STAGING_DIR`], which is pure ASCII, so the re-encode only fails if
/// the caller broke that contract - reported rather than lossily converted,
/// since a mangled staging path would silently stage somewhere else.
pub(crate) fn atomic_fs_service(root: &str) -> Result<services::Fs, FrameworkError> {
    let staging = Path::new(root).join(ATOMIC_STAGING_DIR);
    let staging = staging.to_str().ok_or_else(|| {
        FrameworkError::internal("storage fs atomic staging directory path is not valid UTF-8")
    })?;
    Ok(services::Fs::default().root(root).atomic_write_dir(staging))
}

/// Static facade for the named-disk storage system.
///
/// `Storage` itself holds no state; all disks live in a process-global
/// registry populated by the `register_*` constructors. Look one up with
/// [`Storage::disk`] and operate on it through the returned [`Operator`].
///
/// The `# Testing` notes below name `Storage::fake` as a code span rather
/// than an intra-doc link on purpose: it lives in the `testing` module,
/// gated on `any(test, feature = "testing")`, so a link to it fails to
/// resolve under e.g. `--no-default-features --features filesystem` - and
/// `lib.rs` denies broken intra-doc links, so that is a build failure, not
/// a cosmetic one. Don't promote them back to links.
pub struct Storage;

/// Configuration for the S3 driver.
///
/// Mirrors `opendal::services::S3` - credentials and region are optional so
/// the underlying SDK can fall back to its credential providers (environment,
/// IMDS, profile chain) when omitted.
///
/// The `Debug` impl masks `secret_access_key` (the only secret-bearing
/// field) as `Some("[REDACTED]")` / `None` so a stray `dbg!()` or
/// `tracing::info!(?config)` does not leak AWS credentials. Pattern
/// mirrors [`crate::EncryptionKey`]'s redacting `Debug`.
#[derive(Clone, Default)]
pub struct S3Config {
    /// Bucket name. Required.
    pub bucket: String,
    /// AWS region (e.g. `"us-east-1"`).
    pub region: Option<String>,
    /// Custom endpoint, for S3-compatible services (MinIO, R2, etc.).
    pub endpoint: Option<String>,
    /// Static access key id. Leave `None` to use the default provider chain.
    pub access_key_id: Option<String>,
    /// Static secret access key. Leave `None` to use the default provider chain.
    pub secret_access_key: Option<String>,
    /// Root prefix within the bucket. All operations are relative to this prefix.
    pub root: Option<String>,
}

impl std::fmt::Debug for S3Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("S3Config")
            .field("bucket", &self.bucket)
            .field("region", &self.region)
            .field("endpoint", &self.endpoint)
            .field("access_key_id", &self.access_key_id)
            .field(
                "secret_access_key",
                &self.secret_access_key.as_ref().map(|_| "[REDACTED]"),
            )
            .field("root", &self.root)
            .finish()
    }
}

/// Configuration for the Azure Blob Storage driver.
///
/// The `Debug` impl masks `account_key` (the storage account secret)
/// so a stray `dbg!()` or `tracing::info!(?config)` does not leak the
/// shared key.
///
/// Requires the `filesystem-azure` feature.
#[cfg(feature = "filesystem-azure")]
#[derive(Clone, Default)]
pub struct AzBlobConfig {
    /// Container name. Required.
    pub container: String,
    /// Storage account name.
    pub account_name: String,
    /// Storage account key.
    pub account_key: String,
    /// Custom endpoint (e.g. the Azurite emulator or a sovereign cloud). When
    /// omitted, the standard public endpoint
    /// `https://{account_name}.blob.core.windows.net` is used.
    pub endpoint: Option<String>,
    /// Root prefix within the container.
    pub root: Option<String>,
}

#[cfg(feature = "filesystem-azure")]
impl std::fmt::Debug for AzBlobConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `account_key` is a String (not Option<String>), so render
        // it as a marker that distinguishes "set" from "empty" without
        // leaking the value.
        let account_key_repr = if self.account_key.is_empty() {
            "[unset]"
        } else {
            "[REDACTED]"
        };
        f.debug_struct("AzBlobConfig")
            .field("container", &self.container)
            .field("account_name", &self.account_name)
            .field("account_key", &account_key_repr)
            .field("endpoint", &self.endpoint)
            .field("root", &self.root)
            .finish()
    }
}

/// Configuration for the Google Cloud Storage driver.
///
/// The `Debug` impl masks `credential` (the inline JSON service-account
/// key) so a stray `dbg!()` or `tracing::info!(?config)` does not leak
/// the JSON key bytes. `credential_path` is NOT redacted because it's a
/// filesystem path, not the credential itself.
///
/// Requires the `filesystem-gcs` feature.
#[cfg(feature = "filesystem-gcs")]
#[derive(Clone, Default)]
pub struct GcsConfig {
    /// Bucket name. Required.
    pub bucket: String,
    /// Inline JSON credential blob. Leave `None` to use ADC / metadata server.
    pub credential: Option<String>,
    /// Path to a service-account JSON file on disk.
    pub credential_path: Option<String>,
    /// Custom endpoint (rare, mainly for fakegcs / testing).
    pub endpoint: Option<String>,
    /// Root prefix within the bucket.
    pub root: Option<String>,
}

#[cfg(feature = "filesystem-gcs")]
impl std::fmt::Debug for GcsConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GcsConfig")
            .field("bucket", &self.bucket)
            .field(
                "credential",
                &self.credential.as_ref().map(|_| "[REDACTED]"),
            )
            .field("credential_path", &self.credential_path)
            .field("endpoint", &self.endpoint)
            .field("root", &self.root)
            .finish()
    }
}

/// Configuration for a read-through disk.
///
/// Both `primary` and `fallback` name disks that are already registered.
/// Registration resolves them once, so a later `Storage::forget` on either
/// name leaves this disk working against the operators it captured - disks are
/// meant to be registered once at boot, and the alternative would be a disk
/// that starts failing halfway through a request.
///
/// # How a promotion is published
///
/// A promotion is published so that no reader can observe it half-written,
/// because the object it writes is exactly the one another cold reader routes
/// by existence. Where the primary advertises a `rename` - the local
/// filesystem, which creates the target file and then fills it in place - the
/// bytes are staged at a unique sibling path and renamed into place. Where it
/// does not (in-memory, S3, Azure Blob, GCS), a write is already a single
/// indivisible publish and the promotion writes the target directly,
/// conditional on the object not already existing. A backend offering neither
/// guarantee would leave that window open; no driver Suprnova ships is one.
///
/// That condition is what a staged promotion gives up. Its path is unique, so
/// a no-clobber condition on it would be vacuous, and the target is published
/// by a rename that overwrites: a write landing on the primary between the
/// promotion's last existence check and its rename is overwritten by the
/// promoted copy. On a primary without a rename the condition holds and there
/// is no such window.
///
/// # Versioned and conditional reads
///
/// A read carrying a version or an `If-Match` / `If-None-Match` /
/// `If-Modified-Since` / `If-Unmodified-Since` condition is replayed onto the
/// fallback with that condition intact, and is served but never promoted:
/// writing an old version or a validator-matched body to the primary would
/// publish it as the live object.
///
/// # Copying and moving across the fallback
///
/// `copy` and `rename` resolve the source against the primary first. When only
/// the fallback holds it, the object is streamed across and the destination
/// lands on the primary - without that, either call would fail on an object
/// the disk happily reads. A `rename` also deletes the fallback's copy of the
/// source, on both branches, or the next read would promote it straight back
/// and undo the move.
///
/// The two branches order that delete differently, and the order is the
/// contract. When the primary holds the source, the fallback copy goes first:
/// it is unreachable through this disk while the primary has the object, so
/// nothing observable is lost, and a rename that then fails leaves the primary
/// still holding the source, so a retry re-enters the same branch and renames
/// again. When only the fallback holds it, the delete can only come after the
/// destination is in place, so a move that fails between the two leaves the
/// destination written and the source still there - safe to retry.
///
/// A move the primary would refuse is refused before anything is deleted: a
/// primary with no `rename`, a guarded move onto a primary with no conditional
/// `rename`, and a guarded move onto a destination that already exists all fail
/// with the fallback source untouched.
///
/// Conditions travel with the operation on the streaming branch too:
/// `if_not_exists` becomes a conditional write on the destination, and a copy's
/// source version selects which object the fallback hands over. A copy's
/// `if_match` is refused with `Unsupported` rather than ignored - it is a
/// condition the backend applies inside its own copy, which is the one call
/// this branch cannot make. Because those conditions are answered by whichever
/// disk holds the source, a driver that supports a plain `copy` but not a
/// conditional one - a local directory is exactly that - accepts
/// `if_not_exists` on a fallback-only source and refuses it on its own.
#[derive(Clone, Debug)]
pub struct ReadThroughConfig {
    /// Name of the disk that answers writes and listings, and that promoted
    /// objects are written to. Required.
    pub primary: String,
    /// Name of the disk consulted when the primary does not hold an object.
    /// Must differ from `primary`. Required.
    pub fallback: String,
    /// Whether a fallback hit is written through to the primary.
    ///
    /// Defaults to `true`. Set it to `false` to serve fallback hits without
    /// promoting them, which turns the disk into a transparent read-only
    /// overlay - useful when the primary is a small cache you do not want a
    /// one-off read to fill, or when the fallback is authoritative and the
    /// primary only ever holds objects you put there deliberately.
    ///
    /// The flag governs read-time promotion and nothing else: writes, deletes,
    /// metadata, listings, and the `copy` / `rename` destinations all behave
    /// identically either way.
    pub copy: bool,
    /// Whether a failed promotion fails the read.
    ///
    /// Defaults to `false`, which is the safer production posture: the caller
    /// still receives the fallback's bytes and the failure is logged, so an
    /// unwritable primary degrades throughput instead of returning errors. Set
    /// it to `true` when a silent loss of promotion would hide a real fault -
    /// a migration you are trying to complete, for instance.
    ///
    /// Has no effect when `copy` is `false`: there is no promotion to fail.
    pub throw_on_promotion_failure: bool,
}

impl Default for ReadThroughConfig {
    /// `copy` defaults to `true`, matching Laravel's constructor default, so
    /// `..Default::default()` yields a promoting disk. A derived `Default`
    /// would silently give `false` and turn every abbreviated call site into a
    /// non-promoting overlay.
    fn default() -> Self {
        Self {
            primary: String::new(),
            fallback: String::new(),
            copy: true,
            throw_on_promotion_failure: false,
        }
    }
}

/// Default resilience layer applied by the cloud convenience constructors
/// ([`Storage::register_s3`], and `register_azblob` / `register_gcs` when
/// their features are on - named here as plain code rather than intra-doc
/// links precisely because they may not exist in this build, and
/// `lib.rs` denies broken links).
///
/// Object stores routinely return transient throttling / 5xx errors, so the
/// convenience constructors retry by default. Callers who need a different
/// policy (more retries, timeouts, logging, metrics) use the `_with` variants,
/// which apply no default layer and hand over full control of the stack. Local
/// filesystem and in-memory disks are not wrapped - they have no transient
/// failures worth retrying.
fn default_cloud_resilience(op: Operator) -> Operator {
    op.layer(opendal::layers::RetryLayer::new().with_max_times(3))
}

impl Storage {
    /// Look up a registered disk by name and return its [`Operator`].
    ///
    /// Returns `Err(FrameworkError::Internal)` if no disk is registered under
    /// `name`. The returned `Operator` is cheap to clone (it is `Arc`-backed).
    pub fn disk(name: &str) -> Result<Operator, FrameworkError> {
        registry::get(name)
    }

    /// Register a local filesystem disk rooted at `root`.
    ///
    /// The root directory is created if it does not already exist. Paths
    /// passed to subsequent `disk.write(...)`, `disk.read(...)`, etc. are
    /// resolved relative to this root.
    ///
    /// # Atomic writes
    ///
    /// Every non-`append` write is staged as a temp file under
    /// [`ATOMIC_STAGING_DIR`] inside the root and `rename(2)`d onto the target,
    /// so a concurrent reader sees either the previous object or the new one -
    /// never a partial length - and a crash mid-write leaves no truncated
    /// object at the live path. `append` writes in place by design; staging one
    /// would mean copying the whole object first. The staging directory is
    /// created at registration, reserved, and hidden from listings.
    ///
    /// Equivalent to [`Storage::register_fs_with`] with an identity closure.
    ///
    /// # Testing
    ///
    /// The disk registry is process-global. Tests that call any `register_*`
    /// method directly race on this shared state when run in parallel - wrap
    /// them in a `Storage::fake` guard, which serializes fake-using tests
    /// process-wide and wipes the registry on drop.
    pub fn register_fs(
        name: impl Into<String>,
        root: impl AsRef<Path>,
    ) -> Result<(), FrameworkError> {
        Self::register_fs_with(name, root, |op| op)
    }

    /// Register a local filesystem disk with a custom layer stack applied to
    /// the underlying [`Operator`] before it lands in the registry.
    ///
    /// Writes are atomic and [`ATOMIC_STAGING_DIR`] is reserved; see
    /// [`Storage::register_fs`].
    ///
    /// # Available layers
    ///
    /// Suprnova enables these `suprnova::opendal::layers::*` types out of the
    /// box (each gated behind one `opendal` feature in `framework/Cargo.toml`):
    ///
    /// - [`RetryLayer`](https://docs.rs/opendal/0.58/opendal/layers/struct.RetryLayer.html) -
    ///   exponential-backoff retries on transient 5xx / throttling.
    /// - [`TimeoutLayer`](https://docs.rs/opendal/0.58/opendal/layers/struct.TimeoutLayer.html) -
    ///   per-operation timeout.
    /// - [`LoggingLayer`](https://docs.rs/opendal/0.58/opendal/layers/struct.LoggingLayer.html) -
    ///   debug-level structured logs for every operation.
    /// - [`TracingLayer`](https://docs.rs/opendal/0.58/opendal/layers/struct.TracingLayer.html) -
    ///   `tracing` spans per operation; bridges to OTel through
    ///   `tracing-opentelemetry` when the framework's `otel` feature is on.
    /// - [`PrometheusClientLayer`](https://docs.rs/opendal/0.58/opendal/layers/struct.PrometheusClientLayer.html) -
    ///   histograms + counters for the `prometheus-client` registry.
    ///
    /// Layer order matters: outermost layer wraps everything inside it. The
    /// idiomatic stack is `RetryLayer → TimeoutLayer → LoggingLayer`, so a
    /// timed-out attempt still logs and a retry covers transport failures.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use std::time::Duration;
    /// use suprnova::opendal::layers::{
    ///     LoggingLayer, RetryLayer, TimeoutLayer, TracingLayer,
    /// };
    /// use suprnova::Storage;
    ///
    /// # fn ex() -> Result<(), Box<dyn std::error::Error>> {
    /// Storage::register_fs_with("local", "./storage", |op| {
    ///     op.layer(RetryLayer::new().with_max_times(3))
    ///       .layer(TimeoutLayer::new().with_timeout(Duration::from_secs(30)))
    ///       .layer(LoggingLayer::default())
    ///       .layer(TracingLayer::new())
    /// })?;
    /// # Ok(()) }
    /// ```
    pub fn register_fs_with(
        name: impl Into<String>,
        root: impl AsRef<Path>,
        layer_fn: impl FnOnce(Operator) -> Operator,
    ) -> Result<(), FrameworkError> {
        // Reject non-UTF-8 roots rather than silently mangling them with a
        // lossy conversion (which could root the disk at the wrong directory).
        let root_str = root
            .as_ref()
            .to_str()
            .ok_or_else(|| FrameworkError::internal("storage fs root path is not valid UTF-8"))?;
        let builder = atomic_fs_service(root_str)?;
        // `PathGuardLayer` is applied to the raw FS operator before the user's
        // `layer_fn` runs, so the traversal guard sits closest to the backend
        // and the caller's own layers (retry, logging, tracing) wrap it. The
        // caller can add layers but cannot strip the guard.
        let guarded = Operator::new(builder)
            .map_err(|e| FrameworkError::internal(format!("opendal fs init: {e}")))?
            .layer(path_guard::PathGuardLayer);
        let layered = layer_fn(guarded);
        registry::register(name, layered);
        Ok(())
    }

    /// Register an in-memory disk. Useful for tests, ephemeral buffers, and
    /// any case where persistence is explicitly not required.
    ///
    /// Equivalent to [`Storage::register_memory_with`] with an identity closure.
    ///
    /// # Testing
    ///
    /// The disk registry is process-global. Tests that call any `register_*`
    /// method directly race on this shared state when run in parallel - wrap
    /// them in a `Storage::fake` guard, which serializes fake-using tests
    /// process-wide and wipes the registry on drop.
    pub fn register_memory(name: impl Into<String>) {
        Self::register_memory_with(name, |op| op)
    }

    /// Register an in-memory disk with a custom layer stack.
    ///
    /// Memory backend construction is infallible, so the closure always runs.
    /// Useful for testing layer behaviour without external services.
    ///
    /// See [`Storage::register_fs_with`] for the full list of available
    /// layers (retry, timeout, logging, tracing, prometheus-client).
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use suprnova::opendal::layers::{LoggingLayer, RetryLayer};
    /// use suprnova::Storage;
    ///
    /// Storage::register_memory_with("scratch", |op| {
    ///     op.layer(RetryLayer::new().with_max_times(2))
    ///       .layer(LoggingLayer::default())
    /// });
    /// ```
    pub fn register_memory_with(
        name: impl Into<String>,
        layer_fn: impl FnOnce(Operator) -> Operator,
    ) {
        let raw = Operator::new(services::Memory::default())
            .expect("opendal memory service is infallible");
        let layered = layer_fn(raw);
        registry::register(name, layered);
    }

    /// Register an S3 (or S3-compatible) disk.
    ///
    /// Applies a default [`RetryLayer`](opendal::layers::RetryLayer)
    /// (`with_max_times(3)`) so transient throttling / 5xx errors are retried.
    /// Use [`Storage::register_s3_with`] for full control of the layer stack
    /// (it applies no default layer).
    ///
    /// # Testing
    ///
    /// The disk registry is process-global. Tests that call any `register_*`
    /// method directly race on this shared state when run in parallel - wrap
    /// them in a `Storage::fake` guard, which serializes fake-using tests
    /// process-wide and wipes the registry on drop.
    pub fn register_s3(name: impl Into<String>, config: S3Config) -> Result<(), FrameworkError> {
        Self::register_s3_with(name, config, default_cloud_resilience)
    }

    /// Register an S3 disk with a custom layer stack applied to the
    /// [`Operator`] before it lands in the registry.
    ///
    /// Production S3 deployments need retries (for throttling and transient
    /// 5xx), timeouts, and observability. See [`Storage::register_fs_with`]
    /// for the full list of available layers (retry, timeout, logging,
    /// tracing, prometheus-client).
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use prometheus_client::registry::Registry;
    /// use std::time::Duration;
    /// use suprnova::opendal::layers::{
    ///     LoggingLayer, PrometheusClientLayer, RetryLayer, TimeoutLayer, TracingLayer,
    /// };
    /// use suprnova::{S3Config, Storage};
    ///
    /// # fn ex() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut registry = Registry::default();
    /// let metrics_layer = PrometheusClientLayer::new(&mut registry);
    ///
    /// Storage::register_s3_with(
    ///     "uploads",
    ///     S3Config { bucket: "my-bucket".into(), region: Some("us-east-1".into()), ..Default::default() },
    ///     |op| {
    ///         op.layer(RetryLayer::new().with_max_times(3))
    ///           .layer(TimeoutLayer::new().with_timeout(Duration::from_secs(30)))
    ///           .layer(LoggingLayer::default())
    ///           .layer(TracingLayer::new())
    ///           .layer(metrics_layer)
    ///     },
    /// )?;
    /// # Ok(()) }
    /// ```
    pub fn register_s3_with(
        name: impl Into<String>,
        config: S3Config,
        layer_fn: impl FnOnce(Operator) -> Operator,
    ) -> Result<(), FrameworkError> {
        if config.bucket.trim().is_empty() {
            return Err(FrameworkError::internal(
                "S3 storage config requires a non-empty `bucket`",
            ));
        }
        let mut builder = services::S3::default().bucket(&config.bucket);
        if let Some(region) = config.region.as_deref() {
            builder = builder.region(region);
        }
        if let Some(endpoint) = config.endpoint.as_deref() {
            builder = builder.endpoint(endpoint);
        }
        if let Some(key) = config.access_key_id.as_deref() {
            builder = builder.access_key_id(key);
        }
        if let Some(secret) = config.secret_access_key.as_deref() {
            builder = builder.secret_access_key(secret);
        }
        if let Some(root) = config.root.as_deref() {
            builder = builder.root(root);
        }
        let raw = Operator::new(builder)
            .map_err(|e| FrameworkError::internal(format!("opendal s3 init: {e}")))?;
        let layered = layer_fn(raw);
        registry::register(name, layered);
        Ok(())
    }

    /// Register an Azure Blob Storage disk.
    ///
    /// Applies a default [`RetryLayer`](opendal::layers::RetryLayer)
    /// (`with_max_times(3)`) so transient throttling / 5xx errors are retried.
    /// Use [`Storage::register_azblob_with`] for full control of the layer
    /// stack (it applies no default layer).
    ///
    /// # Testing
    ///
    /// The disk registry is process-global. Tests that call any `register_*`
    /// method directly race on this shared state when run in parallel - wrap
    /// them in a `Storage::fake` guard, which serializes fake-using tests
    /// process-wide and wipes the registry on drop.
    ///
    /// Requires the `filesystem-azure` feature.
    #[cfg(feature = "filesystem-azure")]
    pub fn register_azblob(
        name: impl Into<String>,
        config: AzBlobConfig,
    ) -> Result<(), FrameworkError> {
        Self::register_azblob_with(name, config, default_cloud_resilience)
    }

    /// Register an Azure Blob Storage disk with a custom layer stack applied
    /// to the [`Operator`] before it lands in the registry.
    ///
    /// See [`Storage::register_fs_with`] for the full list of available
    /// layers (retry, timeout, logging, tracing, prometheus-client) and a
    /// canonical ordering example.
    ///
    /// Requires the `filesystem-azure` feature.
    #[cfg(feature = "filesystem-azure")]
    pub fn register_azblob_with(
        name: impl Into<String>,
        config: AzBlobConfig,
        layer_fn: impl FnOnce(Operator) -> Operator,
    ) -> Result<(), FrameworkError> {
        if config.container.trim().is_empty()
            || config.account_name.trim().is_empty()
            || config.account_key.trim().is_empty()
        {
            return Err(FrameworkError::internal(
                "Azure Blob storage config requires non-empty `container`, `account_name`, and `account_key`",
            ));
        }
        // opendal's Azblob backend requires an explicit endpoint. When the
        // caller omits it, derive the standard public Azure Blob endpoint from
        // the account name; an explicit endpoint (e.g. the Azurite emulator or
        // a sovereign cloud) is used as-is.
        let endpoint = config
            .endpoint
            .clone()
            .unwrap_or_else(|| format!("https://{}.blob.core.windows.net", config.account_name));
        let mut builder = services::Azblob::default()
            .container(&config.container)
            .account_name(&config.account_name)
            .account_key(&config.account_key)
            .endpoint(&endpoint);
        if let Some(root) = config.root.as_deref() {
            builder = builder.root(root);
        }
        let raw = Operator::new(builder)
            .map_err(|e| FrameworkError::internal(format!("opendal azblob init: {e}")))?;
        let layered = layer_fn(raw);
        registry::register(name, layered);
        Ok(())
    }

    /// Register a Google Cloud Storage disk.
    ///
    /// Applies a default [`RetryLayer`](opendal::layers::RetryLayer)
    /// (`with_max_times(3)`) so transient throttling / 5xx errors are retried.
    /// Use [`Storage::register_gcs_with`] for full control of the layer stack
    /// (it applies no default layer).
    ///
    /// # Testing
    ///
    /// The disk registry is process-global. Tests that call any `register_*`
    /// method directly race on this shared state when run in parallel - wrap
    /// them in a `Storage::fake` guard, which serializes fake-using tests
    /// process-wide and wipes the registry on drop.
    ///
    /// Requires the `filesystem-gcs` feature.
    #[cfg(feature = "filesystem-gcs")]
    pub fn register_gcs(name: impl Into<String>, config: GcsConfig) -> Result<(), FrameworkError> {
        Self::register_gcs_with(name, config, default_cloud_resilience)
    }

    /// Register a Google Cloud Storage disk with a custom layer stack applied
    /// to the [`Operator`] before it lands in the registry.
    ///
    /// See [`Storage::register_fs_with`] for the full list of available
    /// layers (retry, timeout, logging, tracing, prometheus-client) and a
    /// canonical ordering example.
    ///
    /// Requires the `filesystem-gcs` feature.
    #[cfg(feature = "filesystem-gcs")]
    pub fn register_gcs_with(
        name: impl Into<String>,
        config: GcsConfig,
        layer_fn: impl FnOnce(Operator) -> Operator,
    ) -> Result<(), FrameworkError> {
        if config.bucket.trim().is_empty() {
            return Err(FrameworkError::internal(
                "GCS storage config requires a non-empty `bucket`",
            ));
        }
        let mut builder = services::Gcs::default().bucket(&config.bucket);
        if let Some(credential) = config.credential.as_deref() {
            builder = builder.credential(credential);
        }
        if let Some(path) = config.credential_path.as_deref() {
            builder = builder.credential_path(path);
        }
        if let Some(endpoint) = config.endpoint.as_deref() {
            builder = builder.endpoint(endpoint);
        }
        if let Some(root) = config.root.as_deref() {
            builder = builder.root(root);
        }
        let raw = Operator::new(builder)
            .map_err(|e| FrameworkError::internal(format!("opendal gcs init: {e}")))?;
        let layered = layer_fn(raw);
        registry::register(name, layered);
        Ok(())
    }

    /// Register a read-through disk over two already-registered disks.
    ///
    /// Reads and metadata resolve against `primary` first and fall back to
    /// `fallback`; unless [`ReadThroughConfig::copy`] is `false`, anything
    /// found on the fallback is written through to the primary, so the working
    /// set migrates under real traffic. Writes and listings are primary-only,
    /// and a delete removes the object from both. A `copy` or `rename` whose
    /// source lives only on the fallback streams it across to the primary
    /// destination.
    ///
    /// Equivalent to [`Storage::register_read_through_with`] with an identity
    /// closure.
    ///
    /// # Errors
    ///
    /// Returns `Err(FrameworkError::Internal)` when `primary` or `fallback` is
    /// empty, when they name the same disk, when either names `name` itself,
    /// or when either disk is not registered.
    ///
    /// # Testing
    ///
    /// The disk registry is process-global. Tests that call any `register_*`
    /// method directly race on this shared state when run in parallel - wrap
    /// them in a `Storage::fake` guard, which serializes fake-using tests
    /// process-wide and wipes the registry on drop.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use suprnova::{ReadThroughConfig, Storage};
    ///
    /// # fn ex() -> Result<(), suprnova::FrameworkError> {
    /// Storage::register_memory("new-store");
    /// Storage::register_fs("legacy-store", "./storage/legacy")?;
    /// Storage::register_read_through(
    ///     "assets",
    ///     ReadThroughConfig {
    ///         primary: "new-store".into(),
    ///         fallback: "legacy-store".into(),
    ///         ..Default::default()
    ///     },
    /// )?;
    /// # Ok(()) }
    /// ```
    pub fn register_read_through(
        name: impl Into<String>,
        config: ReadThroughConfig,
    ) -> Result<(), FrameworkError> {
        Self::register_read_through_with(name, config, |op| op)
    }

    /// Register a read-through disk with a custom layer stack applied to the
    /// composed [`Operator`] before it lands in the registry.
    ///
    /// The closure wraps the read-through behavior, so a `RetryLayer` added
    /// here retries the composite operation - including the fallback lookup -
    /// rather than only the primary. See [`Storage::register_fs_with`] for the
    /// full list of available layers.
    ///
    /// # Errors
    ///
    /// Same as [`Storage::register_read_through`].
    pub fn register_read_through_with(
        name: impl Into<String>,
        config: ReadThroughConfig,
        layer_fn: impl FnOnce(Operator) -> Operator,
    ) -> Result<(), FrameworkError> {
        let name = name.into();
        let primary_name = config.primary.trim();
        let fallback_name = config.fallback.trim();

        if primary_name.is_empty() {
            return Err(FrameworkError::internal(
                "read-through disk config requires a non-empty `primary` disk name",
            ));
        }
        if fallback_name.is_empty() {
            return Err(FrameworkError::internal(
                "read-through disk config requires a non-empty `fallback` disk name",
            ));
        }
        if primary_name == fallback_name {
            return Err(FrameworkError::internal(format!(
                "read-through disk '{name}' requires distinct `primary` and `fallback` disks; both name '{primary_name}'"
            )));
        }
        if primary_name == name || fallback_name == name {
            return Err(FrameworkError::internal(format!(
                "read-through disk '{name}' cannot reference itself as its `primary` or `fallback`"
            )));
        }

        let primary = registry::get(primary_name)?;
        let fallback = registry::get(fallback_name)?;

        // The layer captures a clone of the *un-layered* primary so the
        // promotion write and the existence probes can use the high-level
        // operator API. It is the same backend as the stack the layer wraps,
        // so there is no second disk and no way to recurse.
        let composed = primary.clone().layer(read_through::ReadThroughLayer {
            primary,
            fallback,
            copy: config.copy,
            throw_on_promotion_failure: config.throw_on_promotion_failure,
        });

        registry::register(name, layer_fn(composed));
        Ok(())
    }

    /// Drop a registered disk by name, returning whether it was present.
    ///
    /// Mirrors Laravel's `FilesystemManager::forgetDisk`. Useful for
    /// configuration reloads or tests that need to swap a disk implementation
    /// at runtime without spinning up `Storage::fake`.
    pub fn forget(name: &str) -> bool {
        registry::forget(name)
    }

    /// Drop every registered disk.
    ///
    /// Mirrors Laravel's `FilesystemManager::purge()` (which clears every
    /// disk when called without arguments). Production code rarely needs
    /// this; tests should prefer `Storage::fake`, which combines a purge
    /// with a process-wide mutex.
    pub fn purge() {
        registry::purge()
    }

    /// Return the sorted names of every currently-registered disk.
    ///
    /// Handy for diagnostic endpoints, admin dashboards, and tests that need
    /// to assert the boot-time disk set.
    pub fn disks() -> Vec<String> {
        registry::names()
    }

    /// Install a fake (in-memory, isolated) storage environment for the
    /// duration of a test.
    ///
    /// Returns a [`testing::StorageFakeGuard`] that:
    /// - Serializes against other `Storage::fake()` callers via a process-wide
    ///   `Mutex` (so parallel `#[tokio::test]` cases do not race on the
    ///   registry), and
    /// - Resets the registry on drop.
    ///
    /// A `"default"` memory disk is pre-registered for convenience; tests can
    /// register additional disks under whatever names they like.
    #[cfg(any(test, feature = "testing"))]
    pub fn fake() -> testing::StorageFakeGuard {
        testing::install_fake()
    }
}
