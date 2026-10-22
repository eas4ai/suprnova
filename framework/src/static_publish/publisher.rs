//! Staging is cancellable; the manifest changes only after complete uploads.

use super::{
    Manifest, ObjectEntry, VersionEntry,
    cdn::{CdnDriver, Purge},
    error, safe_path, stem,
};
use crate::{FrameworkError, Storage};
use bytes::Bytes;
use futures::{StreamExt, future::BoxFuture, stream::FuturesUnordered};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    sync::Arc,
    time::Duration,
};
use tokio::sync::Mutex;

const MANIFEST: &str = "_pub/manifest.json";
const STATUS: &str = "_pub/status.json";

/// Lifetimes and resource bounds applied to every publication path.
#[derive(Clone, Debug)]
pub struct PublishConfig {
    /// Maximum simultaneous disk uploads; zero is rejected.
    pub concurrency: usize,
    /// Bounds rendering, staging, delivery and rollback so shutdown can drain owned work.
    pub timeout: Duration,
    /// Time between automatic page regenerations.
    pub revalidate: Duration,
    /// Browser lifetime, clamped to `revalidate`.
    pub browser_max_age: Duration,
    /// Shared cache lifetime.
    pub s_maxage: Duration,
    /// How long an edge may serve stale while regenerating.
    pub stale_while_revalidate: Duration,
    /// How long an edge may serve stale after an origin failure.
    pub stale_if_error: Duration,
    /// Retention measured from the time a version stops being live.
    pub grace_period: Duration,
}
impl Default for PublishConfig {
    fn default() -> Self {
        Self {
            concurrency: 4,
            timeout: Duration::from_secs(120),
            revalidate: Duration::from_secs(60),
            browser_max_age: Duration::ZERO,
            s_maxage: Duration::from_secs(60),
            stale_while_revalidate: Duration::from_secs(30),
            stale_if_error: Duration::from_secs(300),
            grace_period: Duration::from_secs(86400),
        }
    }
}

/// Both representations from one checked public render.
#[derive(Clone, Debug)]
pub struct PageObject {
    /// Canonical route path without a query string.
    pub path: String,
    /// Complete HTML from the public renderer.
    pub html: Bytes,
    /// Full Inertia JSON from that same render.
    pub json: Bytes,
    /// Public delivery headers, including the trusted document's hash CSP.
    pub headers: BTreeMap<String, String>,
    /// Dependency identity digests recorded during rendering.
    pub dependency_tags: Vec<String>,
    /// Referenced assets, keyed by their canonical disk-relative path.
    pub assets: BTreeMap<String, (Bytes, String)>,
}
impl PageObject {
    /// Validates the URL before it can become a storage key.
    pub fn new(path: &str, html: Bytes, json: Bytes) -> Result<Self, FrameworkError> {
        Ok(Self {
            path: safe_path(path)?,
            html,
            json,
            headers: BTreeMap::new(),
            dependency_tags: Vec::new(),
            assets: BTreeMap::new(),
        })
    }
}

/// Counts describe actual staged uploads and reused content.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct PublishOutcome {
    /// The selected revision below `_pub/`.
    pub version: String,
    /// Objects whose bytes were newly uploaded.
    pub uploaded: usize,
    /// Objects with unchanged content digests.
    pub skipped: usize,
    /// Cleanup result; a cleanup error never undoes a successful switch.
    pub prune: PruneOutcome,
}
/// Cleanup names every removed and retained object.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct PruneOutcome {
    /// Keys removed from storage.
    pub removed: Vec<String>,
    /// Keys protected by the live revision or a grace period.
    pub kept: Vec<String>,
    /// Cleanup failure after a successful switch, if any.
    pub error: Option<String>,
}
/// Operator state has no page bodies or credentials.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct PublishStatus {
    /// The last complete live revision.
    pub live_version: String,
    /// Success or the last actionable failure.
    pub last_outcome: String,
    /// Work waiting or currently running.
    pub pending: usize,
}

/// A disk and an optional CDN share one serialized publication pipeline.
#[derive(Clone)]
pub struct Publisher {
    pub(crate) disk: opendal::Operator,
    pub(crate) asset_version: String,
    pub(crate) config: PublishConfig,
    driver: Option<Arc<dyn CdnDriver>>,
    fingerprint: regex::Regex,
    serial: Arc<Mutex<()>>,
    status: Arc<Mutex<PublishStatus>>,
    work: Arc<super::work::Work>,
}
struct Upload {
    entry: ObjectEntry,
    bytes: Bytes,
    source: Option<String>,
}

impl Publisher {
    /// Uses any registered disk; no service-specific storage restriction applies.
    pub fn new(
        disk: &str,
        asset_version: &str,
        config: PublishConfig,
    ) -> Result<Self, FrameworkError> {
        Self::from_operator(Storage::disk(disk)?, asset_version, config)
    }
    /// Accepts an application-owned OpenDAL operator.
    pub fn from_operator(
        disk: opendal::Operator,
        asset_version: &str,
        config: PublishConfig,
    ) -> Result<Self, FrameworkError> {
        if asset_version.is_empty()
            || !asset_version
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
            || matches!(asset_version, "." | "..")
        {
            return Err(error(
                "asset version must be a nonempty path-safe identifier",
            ));
        }
        if config.concurrency == 0 || config.timeout.is_zero() || config.revalidate.is_zero() {
            return Err(error(
                "concurrency, timeout and revalidate must be positive",
            ));
        }
        Ok(Self {
            disk,
            asset_version: asset_version.into(),
            config,
            driver: None,
            fingerprint: crate::page_budget::fingerprint_regex(
                crate::page_budget::DEFAULT_FINGERPRINT_PATTERN,
            )?,
            serial: Arc::new(Mutex::new(())),
            status: Arc::new(Mutex::new(PublishStatus::default())),
            work: Arc::new(super::work::Work::default()),
        })
    }
    /// Installs the same driver surface for switching and invalidation.
    pub fn with_cdn(mut self, driver: Arc<dyn CdnDriver>) -> Self {
        self.driver = Some(driver);
        self
    }
    /// Uses the same file-stem convention as StaticFiles while validating overrides before publication.
    pub fn fingerprint(mut self, pattern: impl AsRef<str>) -> Result<Self, FrameworkError> {
        self.fingerprint = crate::page_budget::fingerprint_regex(pattern.as_ref())?;
        Ok(self)
    }
    fn fingerprinted(&self, path: &str) -> bool {
        std::path::Path::new(path)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .is_some_and(|stem| self.fingerprint.is_match(stem))
    }
    /// Reads the durable truth rather than a process-local manifest copy.
    pub async fn manifest(&self) -> Result<Manifest, FrameworkError> {
        match self.disk.read(MANIFEST).await {
            Ok(bytes) => serde_json::from_slice(&bytes.to_bytes()).map_err(error),
            Err(e) if e.kind() == opendal::ErrorKind::NotFound => Ok(Manifest::default()),
            Err(e) => Err(error(e)),
        }
    }
    /// Reports the last outcome, including one recorded by an earlier process.
    pub async fn status(&self) -> PublishStatus {
        let mut current = self.status.lock().await.clone();
        current.pending = self.work.pending();
        if !current.last_outcome.is_empty() {
            return current;
        }
        let durable = match self.disk.read(STATUS).await {
            Ok(bytes) => serde_json::from_slice::<PublishStatus>(&bytes.to_bytes()).map_err(error),
            Err(failure) if failure.kind() == opendal::ErrorKind::NotFound => return current,
            Err(failure) => Err(error(failure)),
        };
        match durable {
            Ok(mut status) => {
                status.pending = current.pending;
                status
            }
            Err(failure) => {
                tracing::error!(error = %failure, "static publication status could not be read");
                PublishStatus {
                    last_outcome: format!("status read failed: {failure}"),
                    ..current
                }
            }
        }
    }
    async fn persist_status(&self, status: &PublishStatus) {
        let result = async {
            let mut durable = status.clone();
            durable.pending = 0;
            let bytes = serde_json::to_vec(&durable).map_err(error)?;
            let mut write = self.disk.write_with(STATUS, bytes);
            if self.disk.info().capability().write_with_cache_control {
                write = write.cache_control("no-store");
            }
            write.await.map_err(error)?;
            Ok::<_, FrameworkError>(())
        }
        .await;
        if let Err(failure) = result {
            tracing::error!(error = %failure, "static publication status could not be persisted");
        }
    }
    /// Publishes already rendered pages through the same path the boot task uses.
    pub async fn publish(&self, pages: Vec<PageObject>) -> Result<PublishOutcome, FrameworkError> {
        self.publish_with(move || async move { Ok(pages) }).await
    }
    /// Replaces the complete page and asset inventory, including removals.
    /// Rendering and staging share the configured timeout.
    pub async fn publish_with<F, Fut>(&self, render: F) -> Result<PublishOutcome, FrameworkError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Vec<PageObject>, FrameworkError>>,
    {
        self.publish_mode(render, false).await
    }
    /// Regenerates selected pages while keeping omitted pages and assets live.
    pub async fn publish_partial(
        &self,
        pages: Vec<PageObject>,
    ) -> Result<PublishOutcome, FrameworkError> {
        self.publish_partial_with(move || async move { Ok(pages) })
            .await
    }
    /// Renders a targeted regeneration through the same upload and switch sequence.
    pub async fn publish_partial_with<F, Fut>(
        &self,
        render: F,
    ) -> Result<PublishOutcome, FrameworkError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Vec<PageObject>, FrameworkError>>,
    {
        self.publish_mode(render, true).await
    }
    async fn publish_mode<F, Fut>(
        &self,
        render: F,
        retain_omitted: bool,
    ) -> Result<PublishOutcome, FrameworkError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Vec<PageObject>, FrameworkError>>,
    {
        let work = self.work.begin();
        let deadline = tokio::time::Instant::now()
            .checked_add(self.config.timeout)
            .ok_or_else(|| error("publication timeout exceeds the clock range"))?;
        let forward = deadline
            - self
                .config
                .timeout
                .div_f64(4.0)
                .min(Duration::from_secs(10));
        let serial = tokio::time::timeout_at(forward, self.serial.clone().lock_owned())
            .await
            .map_err(|_| error("publication serialization timed out"))?;
        let acquired = tokio::time::timeout_at(
            forward,
            super::lease::StorageLease::acquire(self.disk.clone(), self.config.timeout, deadline),
        )
        .await
        .map_err(|_| error("publication lease acquisition timed out"))
        .and_then(|result| result);
        let mut lease = match acquired {
            Ok(lease) => lease,
            Err(failure) => {
                let result = Err(failure);
                self.record_result(&result, deadline).await;
                return result;
            }
        };
        lease.track(work);
        let staged = tokio::time::timeout_at(forward, async {
            let snapshot = super::lease::ManifestSnapshot::load(&self.disk).await?;
            let previous = &snapshot.manifest;
            self.status
                .lock()
                .await
                .live_version
                .clone_from(&previous.live);
            let pages = render().await?;
            let (next, uploads, outcome) = self.plan(previous, pages, retain_omitted)?;
            self.stage(previous, &uploads).await?;
            Ok::<_, FrameworkError>((snapshot, next, uploads, outcome))
        })
        .await
        .map_err(|_| error("render or upload timed out"))
        .and_then(|result| result);
        let (snapshot, next, uploads, outcome) = match staged {
            Ok(staged) => staged,
            Err(failure) => {
                let result = Err(failure);
                self.record_result(&result, deadline).await;
                self.release_lease(&mut lease, deadline).await;
                return result;
            }
        };
        // From this point caller cancellation drops only the join handle, not delivery.
        let publisher = self.clone();
        tokio::spawn(async move {
            let _serial = serial;
            let result = publisher
                .deliver(snapshot, next, uploads, outcome, &lease, forward, deadline)
                .await;
            publisher.record_result(&result, deadline).await;
            publisher.release_lease(&mut lease, deadline).await;
            result
        })
        .await
        .map_err(|failure| error(format!("publication delivery task failed: {failure}")))?
    }
    async fn record_result(
        &self,
        result: &Result<PublishOutcome, FrameworkError>,
        deadline: tokio::time::Instant,
    ) {
        let mut status = self.status.lock().await;
        status.pending = self.work.pending().saturating_sub(1);
        match result {
            Ok(outcome) => {
                status.live_version.clone_from(&outcome.version);
                status.last_outcome = format!(
                    "published: uploaded {}, skipped {}",
                    outcome.uploaded, outcome.skipped
                );
            }
            Err(failure) => {
                status.last_outcome = failure.to_string();
                tracing::error!(%failure,"static publication failed; previous manifest remains live");
            }
        }
        let snapshot = status.clone();
        drop(status);
        if tokio::time::timeout_at(deadline, self.persist_status(&snapshot))
            .await
            .is_err()
        {
            tracing::error!("static publication status persistence timed out");
        }
    }
    async fn release_lease(
        &self,
        lease: &mut super::lease::StorageLease,
        deadline: tokio::time::Instant,
    ) {
        match tokio::time::timeout_at(deadline, lease.release()).await {
            Ok(Ok(())) => {}
            failure => tracing::error!(
                ?failure,
                "static publication lease release failed; expiry remains in force"
            ),
        }
    }
    pub(crate) async fn wait_for_delivery(&self) {
        self.work.drain().await;
    }
    async fn deliver(
        &self,
        snapshot: super::lease::ManifestSnapshot,
        mut next: Manifest,
        uploads: Vec<Upload>,
        mut outcome: PublishOutcome,
        lease: &super::lease::StorageLease,
        forward: tokio::time::Instant,
        deadline: tokio::time::Instant,
    ) -> Result<PublishOutcome, FrameworkError> {
        tokio::time::timeout_at(forward, async {
            lease.verify().await?;
            snapshot.verify(&self.disk).await
        })
        .await
        .map_err(|_| error("delivery validation timed out"))??;
        let previous = &snapshot.manifest;
        if next.live == previous.live {
            outcome.prune = self.maintenance(previous, deadline).await;
            return Ok(outcome);
        }
        if let Some(driver) = &self.driver {
            let switched = tokio::time::timeout_at(forward, driver.switch_version(&next.live))
                .await
                .map_err(|_| error("CDN switch timed out"))
                .and_then(|result| result);
            if let Err(failure) = switched {
                return Err(self.rollback_cdn(previous, failure, deadline).await);
            }
        }
        if tokio::time::Instant::now() >= forward {
            return Err(self
                .rollback_cdn(
                    previous,
                    error("publication timed out before manifest commit"),
                    deadline,
                )
                .await);
        }
        // Conditional commit fences both a changed manifest and a lost storage lease.
        let now = now_ms();
        if let Some(version) = next.versions.get_mut(&previous.live) {
            version.retired_at_ms = Some(now);
        }
        let committed = tokio::time::timeout_at(forward, async {
            lease.verify().await?;
            snapshot.commit(&self.disk, &next).await
        })
        .await
        .map_err(|_| error("manifest commit timed out"))
        .and_then(|result| result);
        if let Err(failure) = committed {
            return Err(self.rollback_cdn(previous, failure, deadline).await);
        }
        if let Some(driver) = &self.driver {
            let capabilities = driver.capabilities();
            let request = if capabilities.tags {
                Some(Purge::Tags(
                    uploads
                        .iter()
                        .flat_map(|u| u.entry.dependency_tags.clone())
                        .collect::<BTreeSet<_>>()
                        .into_iter()
                        .collect(),
                ))
            } else if capabilities.urls {
                Some(Purge::Urls(
                    uploads.iter().map(|u| u.entry.url.clone()).collect(),
                ))
            } else if capabilities.prefix {
                Some(Purge::Prefix("/".into()))
            } else {
                None
            };
            if let Some(request) = request
                && let Err(e) = tokio::time::timeout_at(deadline, driver.purge(request))
                    .await
                    .map_err(|_| error("CDN purge timed out"))
                    .and_then(|result| result)
            {
                tracing::error!(error = %e, "static publication switched but CDN purge failed");
            }
        }
        outcome.prune = self.maintenance(&next, deadline).await;
        Ok(outcome)
    }
    async fn maintenance(
        &self,
        manifest: &Manifest,
        deadline: tokio::time::Instant,
    ) -> PruneOutcome {
        if self.driver.is_none()
            && tokio::time::timeout_at(deadline, self.mirror(manifest))
                .await
                .is_err()
        {
            tracing::error!("static publication mirror deadline exceeded; manifest remains live");
        }
        match tokio::time::timeout_at(deadline, self.retention()).await {
            Ok(outcome) => outcome,
            Err(_) => PruneOutcome {
                error: Some("retention deadline exceeded; live objects retained".into()),
                ..Default::default()
            },
        }
    }
    // Bucket mirrors are maintained only after the durable manifest selects complete objects.
    async fn mirror(&self, manifest: &Manifest) {
        if let Some(version) = manifest.versions.get(&manifest.live) {
            for object in version.objects.iter().filter(|object| !object.asset) {
                let key = if object.url.ends_with("/index.json") {
                    object.url.trim_start_matches('/').to_string()
                } else if object.url == "/" {
                    "index.html".to_string()
                } else {
                    format!("{}/index.html", object.url.trim_matches('/'))
                };
                let source = self
                    .disk
                    .read(&object.path)
                    .await
                    .map(|bytes| bytes.to_bytes());
                if let Ok(source) = &source
                    && let Ok(current) = self.disk.read(&key).await
                    && current.to_bytes() == *source
                    && let Ok(metadata) = self.disk.stat(&key).await
                    && (!self.disk.info().capability().write_with_cache_control
                        || metadata.cache_control()
                            == object.headers.get("cache-control").map(String::as_str))
                    && (!self.disk.info().capability().write_with_user_metadata
                        || metadata.user_metadata().is_some_and(|values| {
                            policy_headers(&object.headers)
                                .iter()
                                .all(|(key, value)| values.get(key) == Some(value))
                        }))
                {
                    continue;
                }
                let mirrored = match source {
                    Ok(bytes) => self.write(&key, &bytes, &object.headers).await,
                    Err(failure) => Err(error(failure)),
                };
                if let Err(failure) = mirrored {
                    tracing::error!(%failure,key,"static publication committed; stable mirror will converge on retry");
                }
            }
        }
    }
    async fn retention(&self) -> PruneOutcome {
        match self.prune_locked().await {
            Ok(prune) => prune,
            Err(e) => {
                tracing::error!(error=%e,"static publication prune failed; live objects retained");
                PruneOutcome {
                    error: Some(e.to_string()),
                    ..PruneOutcome::default()
                }
            }
        }
    }
    async fn rollback_cdn(
        &self,
        previous: &Manifest,
        failure: FrameworkError,
        deadline: tokio::time::Instant,
    ) -> FrameworkError {
        if let Some(driver) = &self.driver {
            let rollback = tokio::time::timeout_at(
                deadline,
                driver.rollback_version(
                    (!previous.live.is_empty()).then_some(previous.live.as_str()),
                ),
            )
            .await
            .map_err(|_| error("CDN rollback timed out"))
            .and_then(|result| result);
            if let Err(rollback) = rollback {
                tracing::error!(error = %rollback, "static publication CDN rollback failed");
                return error(format!("{failure}; CDN rollback failed: {rollback}"));
            }
        }
        failure
    }
    pub(crate) fn representation_headers(
        &self,
        original: &BTreeMap<String, String>,
        content_type: &str,
        bytes: &Bytes,
    ) -> BTreeMap<String, String> {
        let mut headers: BTreeMap<_, _> = original
            .iter()
            .map(|(k, v)| (k.to_ascii_lowercase(), v.clone()))
            .collect();
        headers.insert("content-type".into(), content_type.into());
        headers.insert("content-length".into(), bytes.len().to_string());
        headers.insert("cache-control".into(), self.page_control());
        headers.insert("etag".into(), format!("\"{}\"", digest(bytes)));
        headers
    }
    fn plan(
        &self,
        previous: &Manifest,
        pages: Vec<PageObject>,
        retain_omitted: bool,
    ) -> Result<(Manifest, Vec<Upload>, PublishOutcome), FrameworkError> {
        let revision = format!("{}/{}", self.asset_version, uuid::Uuid::now_v7().simple());
        let mut uploads = Vec::new();
        let mut seen = BTreeSet::new();
        for mut page in pages {
            let mut asset_paths = BTreeMap::new();
            for key in page.assets.keys() {
                let fingerprinted = self.fingerprinted(key);
                let target = if fingerprinted {
                    key.clone()
                } else {
                    format!("_pub/{revision}/_assets/{key}")
                };
                asset_paths.insert(key.clone(), format!("/{target}"));
            }
            (page.html, page.json) =
                super::assets::version_assets(&page.html, &page.json, &page.path, &asset_paths)?;
            let path = safe_path(&page.path)?;
            for (_, value) in page
                .headers
                .iter()
                .filter(|(name, _)| name.eq_ignore_ascii_case("vary"))
            {
                if let Some(field) = value
                    .split(',')
                    .map(str::trim)
                    .filter(|field| !field.is_empty())
                    .find(|field| {
                        !["x-inertia", "x-inertia-version"]
                            .iter()
                            .any(|allowed| allowed.eq_ignore_ascii_case(field))
                    })
                {
                    return Err(error(format!(
                        "route {path}: Vary {field} has no publication key dimension"
                    )));
                }
            }
            if !seen.insert(path.clone()) {
                return Err(error(format!("duplicate page {path}")));
            }
            if page.headers.iter().any(|(name, value)| {
                name.eq_ignore_ascii_case("cache-control")
                    && value
                        .to_ascii_lowercase()
                        .split(',')
                        .any(|part| part.trim() == "no-store")
            }) {
                return Err(error(format!(
                    "route {path}: Cache-Control no-store forbids publication"
                )));
            }
            if page
                .headers
                .keys()
                .any(|h| h.eq_ignore_ascii_case("set-cookie"))
                || page.headers.values().any(|v| v.contains("'nonce-"))
            {
                return Err(error(format!("route {path}: credential or nonce header")));
            }
            let base = stem(&path);
            for (ext, content_type, bytes) in [
                ("html", "text/html; charset=utf-8", page.html),
                ("json", "application/json", page.json),
            ] {
                let headers = self.representation_headers(&page.headers, content_type, &bytes);
                let content_digest = digest(&bytes);
                let stable_base = if path == "/" {
                    String::new()
                } else {
                    format!("{}/", path.trim_matches('/'))
                };
                uploads.push(Upload {
                    entry: ObjectEntry {
                        path: format!("_pub/{revision}/{base}.{ext}"),
                        url: if ext == "html" {
                            path.clone()
                        } else {
                            format!("/{stable_base}index.json")
                        },
                        digest: content_digest,
                        asset: false,
                        asset_version: self.asset_version.clone(),
                        dependency_tags: page.dependency_tags.clone(),
                        headers,
                    },
                    bytes,
                    source: None,
                });
            }
            let mutable_graph = page.assets.keys().any(|key| !self.fingerprinted(key));
            for (key, (bytes, content_type)) in page.assets {
                safe_path(&format!("/{key}"))?;
                if key.starts_with("_pub/") {
                    return Err(error("asset uses reserved _pub prefix"));
                }
                let fingerprinted = self.fingerprinted(&key);
                let headers = BTreeMap::from([
                    ("content-type".into(), content_type),
                    ("content-length".into(), bytes.len().to_string()),
                    ("etag".into(), format!("\"{}\"", digest(&bytes))),
                    (
                        "cache-control".into(),
                        if fingerprinted {
                            "public, max-age=31536000, immutable".into()
                        } else {
                            self.page_control()
                        },
                    ),
                ]);
                let entry = ObjectEntry {
                    path: asset_paths[&key].trim_start_matches('/').to_string(),
                    url: format!("/{key}"),
                    digest: digest(&bytes),
                    asset: true,
                    asset_version: self.asset_version.clone(),
                    dependency_tags: Vec::new(),
                    headers,
                };
                if fingerprinted && mutable_graph {
                    // Relative imports from a mutable entry resolve inside this revision too.
                    let mut relative = entry.clone();
                    relative.path = format!("_pub/{revision}/_assets/{key}");
                    relative.url = format!("/{}", relative.path);
                    let source = previous
                        .versions
                        .get(&previous.live)
                        .and_then(|version| {
                            version.objects.iter().find(|old| {
                                old.path == entry.path
                                    && old.digest == entry.digest
                                    && old.headers == entry.headers
                            })
                        })
                        .map(|old| old.path.clone());
                    uploads.push(Upload {
                        entry: relative,
                        bytes: bytes.clone(),
                        source,
                    });
                }
                uploads.push(Upload {
                    entry,
                    bytes,
                    source: None,
                });
            }
        }
        // Shared assets are uploaded once. A path with conflicting bytes or headers is never raced.
        let mut unique: BTreeMap<String, Upload> = BTreeMap::new();
        for upload in uploads {
            if let Some(old) = unique.get(&upload.entry.path) {
                if old.entry.digest != upload.entry.digest
                    || old.entry.headers != upload.entry.headers
                {
                    return Err(error(format!(
                        "conflicting objects at {}",
                        upload.entry.path
                    )));
                }
            } else {
                unique.insert(upload.entry.path.clone(), upload);
            }
        }
        let mut uploads: Vec<_> = unique.into_values().collect();
        let live = previous.versions.get(&previous.live);
        let mut outcome = PublishOutcome {
            version: revision.clone(),
            ..PublishOutcome::default()
        };
        let old_object = |upload: &Upload| {
            live.and_then(|v| v.objects.iter().find(|o| o.url == upload.entry.url))
        };
        let unchanged = |upload: &Upload| {
            old_object(upload)
                .filter(|o| o.headers == upload.entry.headers && o.digest == upload.entry.digest)
        };
        for upload in &uploads {
            if let Some(driver) = &self.driver {
                driver.validate_headers(&upload.entry.headers)?;
            }
            if upload
                .entry
                .headers
                .get("cache-control")
                .is_some_and(|value| value.split(',').any(|part| part.trim() == "immutable"))
                && old_object(upload).is_some_and(|old| old.digest != upload.entry.digest)
            {
                return Err(error(format!(
                    "fingerprinted asset {} changed its content",
                    upload.entry.path
                )));
            }
            if unchanged(upload).is_some() {
                outcome.skipped += 1;
            } else {
                outcome.uploaded += 1;
            }
        }
        let changed = uploads.iter().any(|upload| {
            old_object(upload).is_none_or(|old| {
                old.digest != upload.entry.digest
                    || old.headers != upload.entry.headers
                    || old.dependency_tags != upload.entry.dependency_tags
                    || old.asset_version != upload.entry.asset_version
            })
        });
        let removed = !retain_omitted
            && live.is_some_and(|version| {
                version
                    .objects
                    .iter()
                    .any(|old| !uploads.iter().any(|upload| upload.entry.url == old.url))
            });
        if !changed && !removed {
            outcome.version.clone_from(&previous.live);
            return Ok((previous.clone(), Vec::new(), outcome));
        }
        let mut next = previous.clone();
        let mut objects = Vec::new();
        let omitted: Vec<_> = live
            .into_iter()
            .flat_map(|v| &v.objects)
            .filter(|old| {
                retain_omitted && !uploads.iter().any(|upload| upload.entry.url == old.url)
            })
            .cloned()
            .collect();
        for old in &omitted {
            let mut entry = old.clone();
            if self.driver.is_some() && !entry.asset && entry.path.starts_with("_pub/") {
                let suffix = if let Some(page) = old.url.strip_suffix("/index.json") {
                    format!("{}.json", stem(if page.is_empty() { "/" } else { page }))
                } else {
                    format!("{}.html", stem(&old.url))
                };
                entry.path = format!("_pub/{revision}/{suffix}");
                uploads.push(Upload {
                    entry: entry.clone(),
                    bytes: Bytes::new(),
                    source: Some(old.path.clone()),
                });
            } else {
                objects.push(entry);
            }
        }
        for upload in &mut uploads {
            if upload.source.is_none()
                && let Some(old) = unchanged(upload)
                && self.driver.is_none()
                && !upload.entry.asset
            {
                upload.entry.path.clone_from(&old.path);
            }
            objects.push(upload.entry.clone());
        }
        objects.sort_by(|a, b| a.path.cmp(&b.path));
        next.live = revision.clone();
        next.versions.insert(
            revision,
            VersionEntry {
                asset_version: self.asset_version.clone(),
                published_at_ms: now_ms(),
                retired_at_ms: None,
                objects,
            },
        );
        Ok((next, uploads, outcome))
    }
    async fn stage(&self, previous: &Manifest, uploads: &[Upload]) -> Result<(), FrameworkError> {
        let mut remaining = uploads.iter();
        let mut pending = FuturesUnordered::new();
        loop {
            while pending.len() < self.config.concurrency {
                match remaining.next() {
                    Some(upload) => pending.push(self.stage_one(previous, upload)),
                    None => break,
                }
            }
            match pending.next().await {
                Some(result) => result?,
                None => return Ok(()),
            }
        }
    }
    fn stage_one<'a>(
        &'a self,
        previous: &'a Manifest,
        upload: &'a Upload,
    ) -> BoxFuture<'a, Result<(), FrameworkError>> {
        Box::pin(async move {
            let old = previous.versions.get(&previous.live).and_then(|version| {
                version.objects.iter().find(|old| {
                    old.digest == upload.entry.digest
                        && old.headers == upload.entry.headers
                        && old.url == upload.entry.url
                })
            });
            if old.is_some_and(|old| old.path == upload.entry.path) {
                return Ok(());
            }
            if let Some(source) = upload
                .source
                .as_deref()
                .or_else(|| old.map(|old| old.path.as_str()))
            {
                if self.disk.info().capability().copy {
                    self.disk
                        .copy(source, &upload.entry.path)
                        .await
                        .map(|_| ())
                        .map_err(error)
                } else {
                    let bytes = self.disk.read(source).await.map_err(error)?.to_bytes();
                    self.write(&upload.entry.path, &bytes, &upload.entry.headers)
                        .await
                }
            } else {
                self.write(&upload.entry.path, &upload.bytes, &upload.entry.headers)
                    .await
            }
        })
    }
    async fn write(
        &self,
        key: &str,
        bytes: &Bytes,
        headers: &BTreeMap<String, String>,
    ) -> Result<(), FrameworkError> {
        let mut write = self.disk.write_with(key, bytes.clone());
        let capability = self.disk.info().capability();
        if capability.write_with_content_type
            && let Some(value) = headers.get("content-type")
        {
            write = write.content_type(value);
        }
        if capability.write_with_cache_control
            && let Some(value) = headers.get("cache-control")
        {
            write = write.cache_control(value);
        }
        if capability.write_with_user_metadata {
            write = write.user_metadata(policy_headers(headers));
        }
        write.await.map_err(error)?;
        Ok(())
    }
    fn page_control(&self) -> String {
        format!(
            "public, max-age={}, s-maxage={}, stale-while-revalidate={}, stale-if-error={}",
            self.config
                .browser_max_age
                .min(self.config.revalidate)
                .as_secs(),
            self.config.s_maxage.as_secs(),
            self.config.stale_while_revalidate.as_secs(),
            self.config.stale_if_error.as_secs()
        )
    }
    /// Uses the same retention pass as the successful publication path.
    pub async fn prune(&self) -> Result<PruneOutcome, FrameworkError> {
        let work = self.work.begin();
        let deadline = tokio::time::Instant::now()
            .checked_add(self.config.timeout)
            .ok_or_else(|| error("prune timeout exceeds the clock range"))?;
        let serial = tokio::time::timeout_at(deadline, self.serial.clone().lock_owned())
            .await
            .map_err(|_| error("prune serialization timed out"))?;
        let mut lease = tokio::time::timeout_at(
            deadline,
            super::lease::StorageLease::acquire(self.disk.clone(), self.config.timeout, deadline),
        )
        .await
        .map_err(|_| error("prune lease timed out"))??;
        lease.track(work);
        let publisher = self.clone();
        tokio::spawn(async move {
            let _serial = serial;
            let result = tokio::time::timeout_at(deadline, publisher.prune_locked())
                .await
                .map_err(|_| error("prune timed out; live references retained"))
                .and_then(|result| result);
            publisher.release_lease(&mut lease, deadline).await;
            result
        })
        .await
        .map_err(|failure| error(format!("prune task failed: {failure}")))?
    }
    async fn prune_locked(&self) -> Result<PruneOutcome, FrameworkError> {
        let snapshot = super::lease::ManifestSnapshot::load(&self.disk).await?;
        let mut manifest = snapshot.manifest.clone();
        let now = now_ms();
        let grace = u64::try_from(self.config.grace_period.as_millis()).unwrap_or(u64::MAX);
        let retained: BTreeSet<_> = manifest
            .versions
            .iter()
            .filter(|(id, version)| {
                **id == manifest.live
                    || version
                        .retired_at_ms
                        .is_none_or(|time| now.saturating_sub(time) < grace)
            })
            .map(|(id, _)| id.clone())
            .collect();
        let mut protected: BTreeSet<_> = manifest
            .versions
            .iter()
            .filter(|(id, _)| retained.contains(*id))
            .flat_map(|(_, version)| version.objects.iter().map(|object| object.path.clone()))
            .collect();
        for (id, version) in &manifest.versions {
            if retained.contains(id) {
                protected.extend(
                    version
                        .objects
                        .iter()
                        .filter(|object| object.asset && object.path.starts_with("_pub/"))
                        .map(|object| object.url.trim_start_matches('/').to_owned()),
                );
            }
        }
        protected.insert(MANIFEST.to_string());
        protected.insert(super::lease::LEASE.to_string());
        protected.insert("_pub/status.json".to_string());
        // Bucket mirrors converge after commit; retain the live page aliases.
        if let Some(live) = manifest.versions.get(&manifest.live) {
            for object in &live.objects {
                if !object.asset
                    && object.path.starts_with("_pub/")
                    && object.path.ends_with(".html")
                {
                    let base = object.url.trim_matches('/');
                    protected.insert(if base.is_empty() {
                        "index.html".into()
                    } else {
                        format!("{base}/index.html")
                    });
                } else if !object.asset
                    && object.path.starts_with("_pub/")
                    && object.path.ends_with(".json")
                {
                    protected.insert(object.url.trim_start_matches('/').to_string());
                }
            }
        }
        let expired: BTreeSet<_> = manifest
            .versions
            .iter()
            .filter(|(id, _)| !retained.contains(*id))
            .flat_map(|(_, version)| {
                version.objects.iter().flat_map(|object| {
                    let alias = (object.asset && object.path.starts_with("_pub/"))
                        .then(|| object.url.trim_start_matches('/').to_owned());
                    std::iter::once(object.path.clone()).chain(alias)
                })
            })
            .filter(|path| !protected.contains(path))
            .collect();
        // Publication exclusively owns this operator's root. Enumerate it so
        // failed staging uploads and files absent from any manifest are also
        // collected. Unknown timestamps are retained rather than guessed old.
        let entries = self
            .disk
            .list_with("")
            .recursive(true)
            .await
            .map_err(error)?;
        let mut paths: BTreeSet<_> = entries
            .into_iter()
            .filter(|entry| !entry.metadata().is_dir())
            .map(|entry| entry.path().to_string())
            .collect();
        paths.extend(expired.iter().cloned());
        let mut report = PruneOutcome::default();
        for key in paths {
            if protected.contains(&key) {
                report.kept.push(key);
                continue;
            }
            let old = if expired.contains(&key) {
                true
            } else {
                let metadata = match self.disk.stat(&key).await {
                    Ok(metadata) => metadata,
                    Err(failure) if failure.kind() == opendal::ErrorKind::NotFound => continue,
                    Err(failure) => return Err(error(failure)),
                };
                metadata.last_modified().is_some_and(|time| {
                    std::time::SystemTime::from(time)
                        .duration_since(std::time::UNIX_EPOCH)
                        .ok()
                        .and_then(|time| u64::try_from(time.as_millis()).ok())
                        .is_some_and(|time| now.saturating_sub(time) >= grace)
                })
            };
            if old {
                self.disk.delete(&key).await.map_err(error)?;
                report.removed.push(key);
            } else {
                report.kept.push(key);
            }
        }
        let before = manifest.versions.len();
        manifest.versions.retain(|id, _| retained.contains(id));
        if before != manifest.versions.len() {
            snapshot.commit(&self.disk, &manifest).await?;
        }
        Ok(report)
    }
}

fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
fn now_ms() -> u64 {
    u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap_or(0)
}

// Native storage and the CDN own representation headers after encoding or compression.
fn policy_headers(headers: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    headers
        .iter()
        .filter(|(name, _)| {
            !matches!(
                name.as_str(),
                "content-type" | "content-length" | "etag" | "content-encoding"
            )
        })
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}
