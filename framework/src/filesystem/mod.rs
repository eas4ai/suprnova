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
mod response;
pub mod streaming;

#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use disk::{ChecksumAlgorithm, DiskExt};
pub use streaming::copy_between_disks;

use crate::FrameworkError;
use opendal::{Operator, services};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use std::path::Path;

/// The name of the disk [`bootstrap_from_env`] registers.
pub const ENV_S3_DISK: &str = "s3";

/// What is written as `%XX` in one segment of the path of a public URL:
/// everything but the letters, the digits and `-`, `.`, `_`, `~`, which
/// are the characters a URL gives no meaning to.
///
/// A server decodes what need not have been encoded, so no link breaks
/// by it. Left as they are, `+` is a space to S3, and `;` starts a path
/// parameter on Tomcat and Jetty, which read `..;` as `..`.
const URL_PATH_SEGMENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// Register the disks the environment describes. The server calls this
/// when it boots, beside the queue, the rate limiter and the mail
/// transport, so an application sets the variables and has the disk.
///
/// When `S3_BUCKET` is set, an S3 disk is registered under the name `s3`
/// ([`ENV_S3_DISK`]), from the variables [`S3Config::from_env`] reads.
/// `S3_PUBLIC_URL` gives the disk its public base URL, see
/// [`Storage::url`]. With `S3_BUCKET` not set, nothing is registered.
///
/// A disk the application registered under the name `s3` is left as it
/// is, and the variables are not read then: the bootstrap of the
/// application runs first, and what it registered is what it meant. In
/// this the disk differs from the queue and the mail transport, where
/// the environment decides.
///
/// The disk is there when the server has booted its drivers. A `booted`
/// callback runs before that, and the console binary boots no driver of
/// the environment. An application that needs the disk in either place
/// calls this function in its own bootstrap.
///
/// There is no default disk. Every call names the disk it uses, so
/// `FILESYSTEM_DISK` is not read.
///
/// # Errors
///
/// When the variables describe no usable disk: no region, one half of
/// the pair of keys, an S3 driver that refuses the configuration, a
/// public URL that is none. The server does not boot then. No error
/// repeats the value of a variable.
pub fn bootstrap_from_env() -> Result<(), FrameworkError> {
    bootstrap_from_variables(|name| std::env::var(name).ok())
}

/// [`bootstrap_from_env`] with the variables looked up by `variable`.
fn bootstrap_from_variables(
    variable: impl Fn(&str) -> Option<String>,
) -> Result<(), FrameworkError> {
    // The disk of the application is looked for first. With it there the
    // variables describe nothing that will be used, and a fault in them
    // must not stop the boot.
    if registry::contains(ENV_S3_DISK) {
        return Ok(());
    }
    let Some(config) = S3Config::from_variables(&variable)? else {
        return Ok(());
    };
    Storage::register_s3(ENV_S3_DISK, config)?;
    if let Some(url) = set_variable(&variable, "S3_PUBLIC_URL") {
        let base = public_base(&url).map_err(|reason| {
            FrameworkError::internal(format!(
                "S3_PUBLIC_URL was refused: {reason}. Write an absolute URL \
                 (https://cdn.example.com/files) or a path of this host (/storage)"
            ))
        })?;
        registry::set_public_url(ENV_S3_DISK, base);
    }
    Ok(())
}

/// `base_url` as the base of the public URLs of a disk, with no slash at
/// its end, or the reason it is none. The reason never repeats the URL.
fn public_base(base_url: &str) -> Result<String, &'static str> {
    let base = base_url.trim();
    if base.is_empty() {
        return Err("it is empty");
    }
    if base.contains('\\') {
        // A browser reads a backslash as a slash, so `/\host` is `//host`:
        // a URL of another host.
        return Err("it has a backslash");
    }
    if base.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err("it has a space or a control character");
    }
    if base.contains(['?', '#']) {
        return Err("it has a query or a fragment");
    }
    let has_dot_segment = |path: &str| {
        path.split('/').any(|segment| {
            let segment = segment.to_ascii_lowercase().replace("%2e", ".");
            segment == "." || segment == ".."
        })
    };

    if base.starts_with('/') {
        if base.starts_with("//") {
            return Err("it begins with two slashes, which is a URL of another host");
        }
        if has_dot_segment(base) {
            return Err("it has a `.` or `..` segment");
        }
        return Ok(base.trim_end_matches('/').to_owned());
    }

    let (scheme, rest) = base
        .split_once("://")
        .ok_or("it has no scheme and is no path")?;
    if !(scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")) {
        return Err("its scheme is neither http nor https");
    }
    if rest.is_empty() || rest.starts_with('/') {
        // `https:///files` has nothing where the host is. A browser
        // takes the first segment of the path for the host.
        return Err("it has no host");
    }
    // What the parser makes of the URL is what a browser makes of it.
    let parsed = url::Url::parse(base).map_err(|_| "it is no URL")?;
    if parsed.host_str().is_none_or(str::is_empty) {
        return Err("it has no host");
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("it has a user or a password, which every link would show");
    }
    let path = rest.split_once('/').map_or("", |(_, path)| path);
    if has_dot_segment(path) {
        return Err("it has a `.` or `..` segment");
    }
    Ok(parsed.as_str().trim_end_matches('/').to_owned())
}

/// The value of `name`, trimmed, and `None` when it is not set or blank.
fn set_variable(variable: &impl Fn(&str) -> Option<String>, name: &str) -> Option<String> {
    variable(name)
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// Directory name reserved inside every local-filesystem disk root.
///
/// A local-filesystem disk stages every write and every `copy` as a temp file
/// under `<root>/.suprnova-atomic/` and publishes it onto the target in one
/// step, so a reader never observes a half-written object and a crash never
/// leaves a truncated one. `append` is the exception and writes in place. The
/// staging directory has to live *inside* the root: a sibling of the root can
/// sit on a different filesystem when the root is a mount point, and then every
/// rename fails with `EXDEV`.
///
/// Living inside the root is why the name is reserved rather than merely
/// conventional. `Storage::disk(..)` refuses any path whose first component is
/// this name - read, write, delete, stat, list alike - and refuses any path
/// that *resolves* into the directory through a symlink, so a caller can
/// neither reach into another writer's staging file nor collide with the name.
/// The entry is filtered out of listings so it never shows up as an object.
///
/// Exported because backup and sync tooling needs to name it: exclude it the
/// way you would exclude a lock directory. It holds in-flight temp files, plus
/// whatever a process that died mid-publish left behind - nothing sweeps those,
/// so an operator watching a crash loop should expect it to grow.
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
    // opendal creates the staging directory only when the path is missing; a
    // regular *file* of that name satisfies its `metadata` probe and
    // canonicalizes fine, so registration would succeed and the first write
    // would fail deep inside the driver with an opaque `create_dir_all` error.
    // A *symlink* there is worse: opendal canonicalizes `atomic_write_dir`, so
    // every staging file would land somewhere that is neither reserved nor
    // filtered from listings, defeating the reservation for the whole disk. Both
    // are refused here, where the message can say what to do - and the probe is
    // `symlink_metadata`, because `metadata` follows the link and would report
    // the symlink case as an ordinary directory.
    if let Ok(existing) = std::fs::symlink_metadata(&staging)
        && !existing.is_dir()
    {
        return Err(FrameworkError::internal(format!(
            "storage fs root '{root}' already holds a '{ATOMIC_STAGING_DIR}' \
             entry that is not a real directory; that name is reserved for \
             staging atomic writes, so move it aside before registering the disk"
        )));
    }
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
    /// Custom endpoint, for S3-compatible services (RustFS, MinIO, R2, etc.).
    pub endpoint: Option<String>,
    /// Static access key id. Leave `None` to use the default provider chain.
    pub access_key_id: Option<String>,
    /// Static secret access key. Leave `None` to use the default provider chain.
    pub secret_access_key: Option<String>,
    /// Root prefix within the bucket. All operations are relative to this prefix.
    pub root: Option<String>,
}

impl S3Config {
    /// The S3 disk the environment describes, and `None` when `S3_BUCKET`
    /// is not set. Use it to register the disk under a name of your own:
    ///
    /// ```rust,no_run
    /// use suprnova::{S3Config, Storage};
    ///
    /// # fn ex() -> Result<(), suprnova::FrameworkError> {
    /// if let Some(config) = S3Config::from_env()? {
    ///     Storage::register_s3("uploads", config)?;
    /// }
    /// # Ok(()) }
    /// ```
    ///
    /// | Variable | |
    /// |---|---|
    /// | `S3_BUCKET` | The bucket. With it not set there is no disk. |
    /// | `S3_REGION` | The region, which the driver needs. `AWS_REGION` and `AWS_DEFAULT_REGION` are read when it is not set. A service that is no AWS takes any name: `us-east-1`, `auto`. |
    /// | `S3_ENDPOINT` | The endpoint of a service that is no AWS: MinIO, RustFS, R2, B2. |
    /// | `S3_ACCESS_KEY`, `S3_SECRET_KEY` | The keys, both or none. |
    /// | `S3_ROOT` | A prefix inside the bucket that every path is under. |
    ///
    /// With no keys set the driver asks the default provider chain of
    /// AWS, which reads `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY`,
    /// the profile, and the role of the instance.
    ///
    /// # Errors
    ///
    /// When one of `S3_ACCESS_KEY` and `S3_SECRET_KEY` is set and the
    /// other is not. The driver would go on with the provider chain, and
    /// the disk would work with credentials nobody meant it to have, or
    /// fail on its first request with an error that names no variable.
    /// When no region is set, which the driver refuses with an error
    /// that names no variable either.
    pub fn from_env() -> Result<Option<Self>, FrameworkError> {
        Self::from_variables(&|name: &str| std::env::var(name).ok())
    }

    /// [`Self::from_env`] with the variables looked up by `variable`.
    fn from_variables(
        variable: &impl Fn(&str) -> Option<String>,
    ) -> Result<Option<Self>, FrameworkError> {
        let set = |name: &str| set_variable(variable, name);
        let Some(bucket) = set("S3_BUCKET") else {
            return Ok(None);
        };
        let (access_key_id, secret_access_key) = match (set("S3_ACCESS_KEY"), set("S3_SECRET_KEY"))
        {
            (Some(key), Some(secret)) => (Some(key), Some(secret)),
            (None, None) => (None, None),
            (Some(_), None) => {
                return Err(FrameworkError::internal(
                    "S3_ACCESS_KEY is set and S3_SECRET_KEY is not; set both, or neither to \
                     use the default credential chain",
                ));
            }
            (None, Some(_)) => {
                return Err(FrameworkError::internal(
                    "S3_SECRET_KEY is set and S3_ACCESS_KEY is not; set both, or neither to \
                     use the default credential chain",
                ));
            }
        };
        let region = set("S3_REGION")
            .or_else(|| set("AWS_REGION"))
            .or_else(|| set("AWS_DEFAULT_REGION"))
            .ok_or_else(|| {
                FrameworkError::internal(
                    "S3_BUCKET is set and no region is; set S3_REGION. A service that is no \
                     AWS takes any name, such as `us-east-1` or `auto`",
                )
            })?;
        Ok(Some(Self {
            bucket,
            region: Some(region),
            endpoint: set("S3_ENDPOINT"),
            access_key_id,
            secret_access_key,
            root: set("S3_ROOT"),
        }))
    }
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
    /// Every operation that publishes bytes at a path publishes them
    /// indivisibly. `write` and `writer` are staged as a temp file under
    /// [`ATOMIC_STAGING_DIR`] inside the root and `rename(2)`d onto the target;
    /// `copy` is staged the same way; `rename` is already one step. A
    /// concurrent reader therefore sees either the previous object or the new
    /// one - never a partial length - and a crash mid-write leaves no truncated
    /// object at the live path. The staging directory is created at
    /// registration, reserved, and hidden from listings.
    ///
    /// A `write_with(..).if_not_exists(true)` is published with `link(2)`
    /// rather than a rename, so it stays a genuine exclusive create: racing
    /// writers cannot all succeed, and every loser gets `ConditionNotMatch`.
    /// That needs a filesystem with hard links. On FAT, exFAT, and some network
    /// filesystems the publish fails rather than silently giving up
    /// exclusivity; every other operation is unaffected.
    ///
    /// `append` is the one in-place operation - staging an append would mean
    /// copying the whole object first - and that holds for the append that
    /// creates the object too, so two appenders racing to create one both land.
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

    /// Give the disk `disk` a public base URL, which makes it a disk of
    /// public files: [`Storage::url`] returns links to them.
    ///
    /// `base_url` is an absolute URL, `https://cdn.example.com/files`, or
    /// a path of the application's own host, `/storage`. It is where the
    /// root of the disk is served from. Serving it is not done here: a
    /// local disk needs a route or a web server in front of its
    /// directory, and a bucket needs to be public or behind a CDN.
    ///
    /// # Errors
    ///
    /// When no disk is registered under `disk`, and when `base_url` is
    /// neither of the two forms. An absolute URL has the scheme `http` or
    /// `https`, a host, and no user, password, query or fragment. A path
    /// begins with one `/`. Neither has a `.` or `..` segment, a
    /// backslash or a control character. The error says what is wrong
    /// and does not repeat the URL, which can carry a token.
    pub fn set_public_url(disk: &str, base_url: &str) -> Result<(), FrameworkError> {
        let base = public_base(base_url).map_err(|reason| {
            FrameworkError::internal(format!(
                "the public base URL of the storage disk '{disk}' was refused: {reason}. \
                 Write an absolute URL (https://cdn.example.com/files) or a path of this \
                 host (/storage)"
            ))
        })?;
        if !registry::set_public_url(disk, base) {
            return Err(FrameworkError::internal(format!(
                "storage disk '{disk}' not registered; register the disk before it is given \
                 a public URL"
            )));
        }
        Ok(())
    }

    /// The public URL of the file at `path` on the disk `disk`. Mirrors
    /// Laravel's `Storage::disk(..)->url(..)`.
    ///
    /// The URL is the public base URL of the disk and the path, each
    /// segment of it written so that it stays one segment: `a b#1.png`
    /// becomes `a%20b%231.png`. Nothing is asked of the disk, so the URL
    /// of a file that does not exist is returned as well.
    ///
    /// For a file that is not public use
    /// [`DiskExt::temporary_url`], which signs a link that expires.
    ///
    /// # Errors
    ///
    /// When the disk has no public base URL ([`Storage::set_public_url`]):
    /// a private disk must not hand out a link that can be guessed. When
    /// `path` is empty or has a `.` or `..` segment, which a browser
    /// resolves before it sends the request, so the link would lead
    /// somewhere else than the path says.
    pub fn url(disk: &str, path: &str) -> Result<String, FrameworkError> {
        let Some(base) = registry::public_url(disk)? else {
            return Err(FrameworkError::internal(format!(
                "storage disk '{disk}' has no public URL. Give it one with \
                 Storage::set_public_url, or sign a link that expires with temporary_url"
            )));
        };
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        if segments.is_empty() {
            return Err(FrameworkError::internal(format!(
                "`{path}` is no path of a file on the storage disk '{disk}'"
            )));
        }
        if segments
            .iter()
            .any(|segment| matches!(*segment, "." | ".."))
        {
            return Err(FrameworkError::internal(format!(
                "`{path}` has a `.` or `..` segment; a public URL is made from the path of \
                 the file itself"
            )));
        }
        let mut url = base;
        for segment in segments {
            url.push('/');
            url.extend(utf8_percent_encode(segment, URL_PATH_SEGMENT));
        }
        Ok(url)
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

#[cfg(test)]
mod env_bootstrap_tests {
    use super::*;

    fn variables(set: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |name| {
            set.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_owned())
        }
    }

    const MINIO: &[(&str, &str)] = &[
        ("S3_ENDPOINT", "http://localhost:9000"),
        ("S3_ACCESS_KEY", "minioadmin"),
        ("S3_SECRET_KEY", "minioadmin"),
        ("S3_BUCKET", "local"),
        ("S3_REGION", "us-east-1"),
    ];

    #[test]
    fn the_variables_of_the_docker_guide_describe_a_disk() {
        let config = S3Config::from_variables(&variables(MINIO))
            .expect("the variables are a whole configuration")
            .expect("the bucket is set");

        assert_eq!(config.bucket, "local");
        assert_eq!(config.region.as_deref(), Some("us-east-1"));
        assert_eq!(config.endpoint.as_deref(), Some("http://localhost:9000"));
        assert_eq!(config.access_key_id.as_deref(), Some("minioadmin"));
        assert_eq!(config.secret_access_key.as_deref(), Some("minioadmin"));
        assert_eq!(config.root, None);
    }

    #[test]
    fn with_no_bucket_there_is_no_disk() {
        for set in [
            variables(&[]),
            variables(&[("S3_BUCKET", "  ")]),
            variables(&[("S3_ENDPOINT", "http://localhost:9000")]),
        ] {
            assert!(
                S3Config::from_variables(&set)
                    .expect("no disk is no error")
                    .is_none()
            );
        }
    }

    #[test]
    fn with_no_keys_the_provider_chain_is_left_to_find_them() {
        let config = S3Config::from_variables(&variables(&[
            ("S3_BUCKET", "files"),
            ("AWS_REGION", "eu-central-1"),
            ("S3_ROOT", "/tenant-7"),
        ]))
        .expect("a configuration")
        .expect("the bucket is set");

        assert_eq!(config.access_key_id, None);
        assert_eq!(config.secret_access_key, None);
        assert_eq!(
            config.region.as_deref(),
            Some("eu-central-1"),
            "the region of the AWS variables is read when S3_REGION is not set"
        );
        assert_eq!(config.root.as_deref(), Some("/tenant-7"));
    }

    #[test]
    fn the_region_of_the_disk_wins_over_the_region_of_aws() {
        let config = S3Config::from_variables(&variables(&[
            ("S3_BUCKET", "files"),
            ("S3_REGION", "auto"),
            ("AWS_REGION", "eu-central-1"),
        ]))
        .expect("a configuration")
        .expect("the bucket is set");
        assert_eq!(config.region.as_deref(), Some("auto"));
    }

    #[test]
    fn one_key_without_the_other_is_refused_and_the_error_shows_no_key() {
        for (set, missing) in [
            (
                variables(&[
                    ("S3_BUCKET", "files"),
                    ("S3_REGION", "us-east-1"),
                    ("S3_ACCESS_KEY", "AKIA-EXAMPLE"),
                ]),
                "S3_SECRET_KEY",
            ),
            (
                variables(&[
                    ("S3_BUCKET", "files"),
                    ("S3_REGION", "us-east-1"),
                    ("S3_SECRET_KEY", "a-secret-value"),
                ]),
                "S3_ACCESS_KEY",
            ),
        ] {
            let error = S3Config::from_variables(&set).expect_err("half a pair of keys");
            let message = error.to_string();
            assert!(message.contains(missing), "{message}");
            assert!(
                !message.contains("AKIA-EXAMPLE") && !message.contains("a-secret-value"),
                "{message}"
            );
        }
    }

    #[test]
    fn a_bucket_with_no_region_names_the_variable_to_set() {
        let error = S3Config::from_variables(&variables(&[("S3_BUCKET", "files")]))
            .expect_err("the driver needs a region");
        assert!(error.to_string().contains("S3_REGION"), "{error}");

        // A blank region is no region.
        let error = S3Config::from_variables(&variables(&[
            ("S3_BUCKET", "files"),
            ("S3_REGION", " "),
            ("AWS_REGION", ""),
        ]))
        .expect_err("the driver needs a region");
        assert!(error.to_string().contains("S3_REGION"), "{error}");
    }

    #[test]
    fn a_fault_in_the_variables_does_not_stop_an_application_with_a_disk_of_its_own() {
        let _storage = Storage::fake();
        Storage::register_memory(ENV_S3_DISK);

        // Half a pair of keys and no region: the variables describe no
        // disk that would be used.
        bootstrap_from_variables(variables(&[
            ("S3_BUCKET", "files"),
            ("S3_ACCESS_KEY", "AKIA-EXAMPLE"),
        ]))
        .expect("the disk of the application is the disk");
    }

    #[test]
    fn a_public_url_that_is_refused_names_its_variable_and_not_its_value() {
        let _storage = Storage::fake();

        let error = bootstrap_from_variables(variables(&[
            ("S3_BUCKET", "local"),
            ("S3_REGION", "us-east-1"),
            (
                "S3_PUBLIC_URL",
                "https://files.internal.test/public?sig=a-signed-token",
            ),
        ]))
        .expect_err("a base URL has no query");

        let message = error.to_string();
        assert!(message.contains("S3_PUBLIC_URL"), "{message}");
        assert!(message.contains("query"), "{message}");
        assert!(
            !message.contains("a-signed-token") && !message.contains("internal.test"),
            "the value must not be repeated: {message}"
        );
    }

    #[test]
    fn the_disk_is_registered_under_the_name_s3() {
        let _storage = Storage::fake();

        bootstrap_from_variables(variables(MINIO)).expect("the disk registers");

        assert!(Storage::disks().contains(&ENV_S3_DISK.to_owned()));
        assert!(
            Storage::url(ENV_S3_DISK, "a.png").is_err(),
            "with no S3_PUBLIC_URL the disk is a private one"
        );
    }

    #[test]
    fn the_public_url_of_the_environment_is_the_public_url_of_the_disk() {
        let _storage = Storage::fake();

        bootstrap_from_variables(variables(&[
            ("S3_BUCKET", "local"),
            ("S3_REGION", "us-east-1"),
            ("S3_PUBLIC_URL", "https://cdn.example.com/files/"),
        ]))
        .expect("the disk registers");

        assert_eq!(
            Storage::url(ENV_S3_DISK, "avatars/7.png").expect("a public disk"),
            "https://cdn.example.com/files/avatars/7.png"
        );
    }

    #[test]
    fn with_no_bucket_nothing_is_registered() {
        let _storage = Storage::fake();
        let before = Storage::disks();

        bootstrap_from_variables(variables(&[
            ("S3_ENDPOINT", "http://localhost:9000"),
            ("S3_ACCESS_KEY", "half-a-pair"),
        ]))
        .expect("with no bucket the other variables are not read");

        assert_eq!(Storage::disks(), before);
    }

    #[tokio::test]
    async fn a_disk_the_application_registered_under_the_name_is_left_as_it_is() {
        let _storage = Storage::fake();
        Storage::register_memory(ENV_S3_DISK);
        let disk = Storage::disk(ENV_S3_DISK).expect("the disk of the application");
        disk.write("kept.txt", "of the application")
            .await
            .expect("a write");

        bootstrap_from_variables(variables(MINIO)).expect("nothing to do");

        let still = Storage::disk(ENV_S3_DISK).expect("a disk");
        assert_eq!(
            still.read("kept.txt").await.expect("a read").to_vec(),
            b"of the application",
            "the memory disk is still the one under the name"
        );
    }
}
