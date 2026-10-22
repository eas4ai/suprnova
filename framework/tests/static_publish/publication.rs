use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use bytes::Bytes;
use opendal::raw::{
    Layer, OpCopier, OpCopy, OpList, OpRead, OpStat, OpWrite, RpStat, Service, ServiceInfo,
    Servicer, oio,
};
use opendal::{Buffer, Capability, Metadata, OperationContext, Operator, services};
use suprnova::{
    FrameworkError,
    static_publish::{
        PageObject, PublishConfig, Publisher,
        cdn::{CdnCapabilities, CdnDriver, MemoryCdnDriver, Purge},
    },
};

fn memory() -> Operator {
    Operator::new(services::Memory::default()).unwrap()
}
fn publisher(version: &str) -> (Publisher, Operator) {
    let disk = memory();
    (
        Publisher::from_operator(disk.clone(), version, PublishConfig::default()).unwrap(),
        disk,
    )
}
fn page_at(path: &str, text: &str) -> PageObject {
    PageObject::new(path, Bytes::from(format!("<html>{text}</html>")),
        Bytes::from(serde_json::json!({"component":"About","props":{"text":text},"url":path,"version":"v1"}).to_string())).unwrap()
}
fn page(text: &str) -> PageObject {
    page_at("/about", text)
}
async fn manifest_bytes(disk: &Operator) -> Bytes {
    disk.read("_pub/manifest.json").await.unwrap().to_bytes()
}

#[tokio::test]
async fn pub_002_html_json_and_atomic_manifest_on_any_disk() {
    let (p, disk) = publisher("v1");
    let outcome = p.publish(vec![page("one")]).await.unwrap();
    let manifest = p.manifest().await.unwrap();
    assert_eq!(manifest.live, outcome.version);
    let live = &manifest.versions[&manifest.live];
    assert_eq!(live.objects.len(), 2);
    for object in &live.objects {
        assert!(object.path.starts_with("_pub/v1/"));
        assert!(disk.exists(&object.path).await.unwrap());
        assert!(!object.headers["cache-control"].contains("immutable"));
        assert!(object.headers["cache-control"].contains("stale-while-revalidate="));
        assert!(object.headers["cache-control"].contains("stale-if-error="));
        assert!(!object.headers.contains_key("set-cookie"));
        assert_eq!(object.headers["etag"], format!("\"{}\"", object.digest));
    }
    assert_ne!(live.objects[0].path, live.objects[1].path);
    assert!(disk.exists("about/index.html").await.unwrap());
    assert!(disk.exists("about/index.json").await.unwrap());
}

#[tokio::test]
async fn pub_003_unchanged_targeted_pages_do_not_upload_or_switch() {
    let disk = memory();
    let state = Arc::new(Probe::default());
    let driver = Arc::new(MemoryCdnDriver::default());
    let p = Publisher::from_operator(
        disk.layer(ProbeLayer(state.clone())),
        "v1",
        PublishConfig::default(),
    )
    .unwrap()
    .with_cdn(driver.clone());
    let first = p
        .publish(vec![page("one"), page_at("/other", "two")])
        .await
        .unwrap();
    let writes = state
        .writes
        .lock()
        .unwrap()
        .iter()
        .filter(|key| key.as_str() != "_pub/status.json" && key.as_str() != "_pub/lease.json")
        .count();
    let events = driver.events().unwrap().len();
    let next = p.publish_partial(vec![page("one")]).await.unwrap();
    assert_eq!(next.uploaded, 0);
    assert_eq!(next.skipped, 2);
    assert_eq!(next.version, first.version);
    assert_eq!(
        state
            .writes
            .lock()
            .unwrap()
            .iter()
            .filter(|key| key.as_str() != "_pub/status.json" && key.as_str() != "_pub/lease.json")
            .count(),
        writes
    );
    assert_eq!(driver.events().unwrap().len(), events);
}

#[tokio::test]
async fn pub_003_failed_render_keeps_manifest_and_status_names_failure() {
    let (p, disk) = publisher("v1");
    p.publish(vec![page("one")]).await.unwrap();
    let before = manifest_bytes(&disk).await;
    let result = p
        .publish_with(|| async {
            Err::<Vec<PageObject>, _>(FrameworkError::internal("broken render"))
        })
        .await;
    assert!(result.is_err());
    assert_eq!(manifest_bytes(&disk).await, before);
    let status = p.status().await;
    assert!(status.last_outcome.contains("broken render"));
    assert!(!status.live_version.is_empty());
    assert_eq!(status.pending, 0);
}

#[tokio::test]
async fn pub_002_cdn_targeted_regeneration_copies_omitted_pages_into_new_prefix() {
    let (p, disk) = publisher("v1");
    let p = p.with_cdn(Arc::new(MemoryCdnDriver::default()));
    p.publish(vec![page("one"), page_at("/other", "keep")])
        .await
        .unwrap();
    let next = p.publish_partial(vec![page("changed")]).await.unwrap();
    assert_eq!(next.uploaded, 2);
    let manifest = p.manifest().await.unwrap();
    let live = &manifest.versions[&manifest.live];
    assert_eq!(live.objects.len(), 4);
    for object in &live.objects {
        assert!(object.path.starts_with(&format!("_pub/{}/", next.version)));
        assert!(disk.exists(&object.path).await.unwrap());
        if object.url == "/other" {
            assert_eq!(
                disk.read(&object.path).await.unwrap().to_bytes(),
                Bytes::from_static(b"<html>keep</html>")
            );
        }
    }
}

#[tokio::test]
async fn pub_002_shared_assets_are_deduplicated_and_conflicts_refused() {
    let (p, disk) = publisher("v1");
    let asset = (
        Bytes::from_static(b"shared script"),
        "application/javascript".into(),
    );
    let mut first = page("one");
    first
        .assets
        .insert("build/app-abcd1234.js".into(), asset.clone());
    let mut second = page_at("/other", "two");
    second.assets.insert("build/app-abcd1234.js".into(), asset);
    let outcome = p
        .publish(vec![first.clone(), second.clone()])
        .await
        .unwrap();
    assert_eq!(outcome.uploaded, 5);
    let manifest = p.manifest().await.unwrap();
    let assets: Vec<_> = manifest.versions[&manifest.live]
        .objects
        .iter()
        .filter(|object| !object.path.starts_with("_pub/"))
        .collect();
    assert_eq!(assets.len(), 1);
    assert_eq!(
        assets[0].headers["cache-control"],
        "public, max-age=31536000, immutable"
    );
    let before = manifest_bytes(&disk).await;
    second.assets.insert(
        "build/app-abcd1234.js".into(),
        (
            Bytes::from_static(b"different"),
            "application/javascript".into(),
        ),
    );
    assert!(
        p.publish(vec![first, second.clone()])
            .await
            .unwrap_err()
            .to_string()
            .contains("conflicting objects")
    );
    assert_eq!(manifest_bytes(&disk).await, before);
    assert!(
        p.publish(vec![second])
            .await
            .unwrap_err()
            .to_string()
            .contains("changed its content")
    );
}

#[tokio::test]
async fn pub_002_page_headers_clamp_browser_lifetime_and_forbid_credentials_and_nonces() {
    let disk = memory();
    let config = PublishConfig {
        browser_max_age: Duration::from_secs(900),
        revalidate: Duration::from_secs(10),
        ..Default::default()
    };
    let p = Publisher::from_operator(disk, "v1", config).unwrap();
    p.publish(vec![page("one")]).await.unwrap();
    let manifest = p.manifest().await.unwrap();
    assert!(
        manifest.versions[&manifest.live]
            .objects
            .iter()
            .all(|object| object.headers["cache-control"].contains("max-age=10,"))
    );
    for (header, value) in [
        ("Set-Cookie", "token=private"),
        ("Content-Security-Policy", "script-src 'nonce-secret'"),
    ] {
        let mut page = page("private");
        page.headers.insert(header.into(), value.into());
        assert!(p.publish(vec![page]).await.is_err());
    }
}

#[tokio::test]
async fn pub_003_dependency_changes_update_manifest_without_reuploading_content() {
    let (p, _) = publisher("v1");
    let first = p.publish(vec![page("one")]).await.unwrap();
    let mut changed = page("one");
    changed.dependency_tags.push("new-dependency".into());
    let next = p.publish(vec![changed]).await.unwrap();
    assert_eq!(next.uploaded, 0);
    assert_eq!(next.skipped, 2);
    assert_ne!(next.version, first.version);
    let manifest = p.manifest().await.unwrap();
    assert!(
        manifest.versions[&manifest.live]
            .objects
            .iter()
            .all(|object| object.dependency_tags == ["new-dependency"])
    );
}

#[derive(Default)]
struct FailingCdn {
    events: Mutex<Vec<String>>,
    fail_switch: AtomicBool,
    stall_switch: AtomicBool,
}
#[async_trait]
impl CdnDriver for FailingCdn {
    fn capabilities(&self) -> CdnCapabilities {
        CdnCapabilities {
            urls: true,
            ..Default::default()
        }
    }
    async fn switch_version(&self, version: &str) -> Result<(), FrameworkError> {
        self.events
            .lock()
            .unwrap()
            .push(format!("switch:{version}"));
        if self.stall_switch.swap(false, Ordering::SeqCst) {
            std::future::pending::<()>().await;
        }
        if self.fail_switch.swap(false, Ordering::SeqCst) {
            Err(FrameworkError::internal(
                "switch failed after provider accepted it",
            ))
        } else {
            Ok(())
        }
    }
    async fn rollback_version(&self, previous: Option<&str>) -> Result<(), FrameworkError> {
        self.events
            .lock()
            .unwrap()
            .push(format!("rollback:{}", previous.unwrap_or("initial")));
        Ok(())
    }
    async fn purge(&self, _: Purge) -> Result<(), FrameworkError> {
        Ok(())
    }
}

#[tokio::test]
async fn pub_003_switch_failure_restores_previous_cdn_revision_and_keeps_manifest() {
    let (p, disk) = publisher("v1");
    let driver = Arc::new(FailingCdn::default());
    let p = p.with_cdn(driver.clone());
    let first = p.publish(vec![page("one")]).await.unwrap();
    let before = manifest_bytes(&disk).await;
    driver.fail_switch.store(true, Ordering::SeqCst);
    assert!(p.publish(vec![page("two")]).await.is_err());
    assert_eq!(manifest_bytes(&disk).await, before);
    assert_eq!(
        driver.events.lock().unwrap().last().unwrap(),
        &format!("rollback:{}", first.version)
    );
}

#[tokio::test]
async fn pub_003_manifest_failure_preserves_live_and_mirror_failure_is_post_commit() {
    for refusal in ["_pub/manifest.json", "about/index.json"] {
        let disk = memory();
        let state = Arc::new(Probe::default());
        let p = Publisher::from_operator(
            disk.clone().layer(ProbeLayer(state.clone())),
            "v1",
            PublishConfig::default(),
        )
        .unwrap();
        p.publish(vec![page("one")]).await.unwrap();
        let before = manifest_bytes(&disk).await;
        let html = disk.read("about/index.html").await.unwrap().to_bytes();
        let json = disk.read("about/index.json").await.unwrap().to_bytes();
        *state.refuse.lock().unwrap() = Some(refusal.into());
        let result = p.publish(vec![page("two")]).await;
        if refusal == "_pub/manifest.json" {
            assert!(result.is_err());
            assert_eq!(manifest_bytes(&disk).await, before);
            assert_eq!(
                disk.read("about/index.html").await.unwrap().to_bytes(),
                html
            );
            assert_eq!(
                disk.read("about/index.json").await.unwrap().to_bytes(),
                json
            );
        } else {
            assert!(
                result.is_ok(),
                "a mirror failure does not undo the manifest commit"
            );
            assert_ne!(manifest_bytes(&disk).await, before);
            let manifest = p.manifest().await.unwrap();
            for object in &manifest.versions[&manifest.live].objects {
                assert!(disk.exists(&object.path).await.unwrap());
            }
        }
    }
}

#[tokio::test]
async fn pub_003_manifest_failure_rolls_back_cdn_including_initial_configuration() {
    for initial in [false, true] {
        let disk = memory();
        let state = Arc::new(Probe::default());
        let driver = Arc::new(FailingCdn::default());
        let p = Publisher::from_operator(
            disk.clone().layer(ProbeLayer(state.clone())),
            "v1",
            PublishConfig::default(),
        )
        .unwrap()
        .with_cdn(driver.clone());
        let before = if initial {
            None
        } else {
            p.publish(vec![page("one")]).await.unwrap();
            Some(manifest_bytes(&disk).await)
        };
        *state.refuse.lock().unwrap() = Some("_pub/manifest.json".into());
        assert!(p.publish(vec![page("two")]).await.is_err());
        assert!(
            driver
                .events
                .lock()
                .unwrap()
                .last()
                .unwrap()
                .starts_with("rollback:")
        );
        if let Some(before) = before {
            assert_eq!(manifest_bytes(&disk).await, before);
        } else {
            assert!(!disk.exists("_pub/manifest.json").await.unwrap());
            assert_eq!(
                driver.events.lock().unwrap().last().unwrap(),
                "rollback:initial"
            );
        }
    }
}

#[tokio::test]
async fn pub_003_upload_failure_and_timeout_never_change_the_live_manifest() {
    let disk = memory();
    let state = Arc::new(Probe::default());
    let config = PublishConfig {
        timeout: Duration::from_millis(30),
        ..Default::default()
    };
    let p = Publisher::from_operator(disk.clone().layer(ProbeLayer(state.clone())), "v1", config)
        .unwrap();
    p.publish(vec![page("one")]).await.unwrap();
    let before = manifest_bytes(&disk).await;
    *state.refuse.lock().unwrap() = Some(".json".into());
    assert!(p.publish(vec![page("two")]).await.is_err());
    assert_eq!(manifest_bytes(&disk).await, before);
    state.stall.store(true, Ordering::SeqCst);
    assert!(
        p.publish(vec![page("three")])
            .await
            .unwrap_err()
            .to_string()
            .contains("timed out")
    );
    assert_eq!(manifest_bytes(&disk).await, before);
    assert_eq!(state.active.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn pub_003_uploads_run_in_parallel_within_the_configured_bound() {
    let disk = memory();
    let state = Arc::new(Probe::default());
    state.delay.store(true, Ordering::SeqCst);
    let config = PublishConfig {
        concurrency: 2,
        ..Default::default()
    };
    let p = Publisher::from_operator(disk.layer(ProbeLayer(state.clone())), "v1", config).unwrap();
    let pages = (0..4)
        .map(|index| page_at(&format!("/page-{index}"), "one"))
        .collect();
    p.publish(pages).await.unwrap();
    assert_eq!(state.maximum.load(Ordering::SeqCst), 2);
}

#[derive(Debug, Default)]
struct Probe {
    writes: Mutex<Vec<String>>,
    refuse: Mutex<Option<String>>,
    stall: AtomicBool,
    delay: AtomicBool,
    active: AtomicUsize,
    maximum: AtomicUsize,
    maximum_asset_read: std::sync::atomic::AtomicU64,
    force_copy: AtomicBool,
    disable_copy: AtomicBool,
    copies: AtomicUsize,
    force_metadata: AtomicBool,
    metadata_writes: Mutex<Vec<(String, std::collections::BTreeMap<String, String>)>>,
    force_cas: AtomicBool,
    cas: tokio::sync::Mutex<std::collections::BTreeMap<String, String>>,
    cas_sequence: AtomicUsize,
    cas_rejections: AtomicUsize,
    inject_manifest: Mutex<Option<(Operator, Bytes)>>,
    commit_mode: AtomicUsize,
    fail_manifest_read: AtomicBool,
    conditional_writes: Mutex<Vec<(String, String)>>,
    gate_manifest: AtomicBool,
    manifest_entered: tokio::sync::Notify,
    manifest_release: tokio::sync::Notify,
}
#[derive(Clone, Debug)]
struct ProbeLayer(Arc<Probe>);
impl Layer for ProbeLayer {
    fn apply_service(&self, inner: Servicer) -> Servicer {
        Arc::new(ProbeService {
            inner,
            state: self.0.clone(),
        })
    }
}
#[derive(Debug)]
struct ProbeService {
    inner: Servicer,
    state: Arc<Probe>,
}
impl Service for ProbeService {
    type Reader = ProbeReader;
    type Writer = ProbeWriter;
    type Lister = oio::Lister;
    type Deleter = oio::Deleter;
    type Copier = oio::Copier;
    fn info(&self) -> ServiceInfo {
        self.inner.info()
    }
    fn capability(&self) -> Capability {
        let mut capability = self.inner.capability();
        if self.state.force_copy.load(Ordering::SeqCst) {
            capability.copy = true;
        }
        if self.state.disable_copy.load(Ordering::SeqCst) {
            capability.copy = false;
        }
        if self.state.force_cas.load(Ordering::SeqCst) {
            capability.write_with_if_match = true;
        }
        if self.state.force_metadata.load(Ordering::SeqCst) {
            capability.write_with_user_metadata = true;
        }
        capability
    }
    async fn create_dir(
        &self,
        ctx: &OperationContext,
        path: &str,
        args: opendal::raw::OpCreateDir,
    ) -> opendal::Result<opendal::raw::RpCreateDir> {
        self.inner.create_dir(ctx, path, args).await
    }
    async fn rename(
        &self,
        ctx: &OperationContext,
        from: &str,
        to: &str,
        args: opendal::raw::OpRename,
    ) -> opendal::Result<opendal::raw::RpRename> {
        self.inner.rename(ctx, from, to, args).await
    }
    async fn presign(
        &self,
        ctx: &OperationContext,
        path: &str,
        args: opendal::raw::OpPresign,
    ) -> opendal::Result<opendal::raw::RpPresign> {
        self.inner.presign(ctx, path, args).await
    }
    fn read(
        &self,
        ctx: &OperationContext,
        path: &str,
        args: OpRead,
    ) -> opendal::Result<Self::Reader> {
        if path == "_pub/manifest.json" && self.state.fail_manifest_read.load(Ordering::SeqCst) {
            return Err(opendal::Error::new(
                opendal::ErrorKind::PermissionDenied,
                "injected reconciliation read failure",
            ));
        }
        Ok(ProbeReader {
            inner: self.inner.read(ctx, path, args)?,
            state: self.state.clone(),
            tracked: path.ends_with("large.bin"),
        })
    }
    async fn stat(
        &self,
        ctx: &OperationContext,
        path: &str,
        args: OpStat,
    ) -> opendal::Result<RpStat> {
        if self.state.force_cas.load(Ordering::SeqCst) {
            let etags = self.state.cas.lock().await;
            let reply = self.inner.stat(ctx, path, args).await?;
            let etag = etags
                .get(path)
                .cloned()
                .unwrap_or_else(|| "\"cas-external\"".to_string());
            Ok(reply.map_metadata(|metadata| metadata.with_etag(etag)))
        } else {
            self.inner.stat(ctx, path, args).await
        }
    }
    fn list(
        &self,
        ctx: &OperationContext,
        path: &str,
        args: OpList,
    ) -> opendal::Result<Self::Lister> {
        self.inner.list(ctx, path, args)
    }
    fn delete(&self, ctx: &OperationContext) -> opendal::Result<Self::Deleter> {
        self.inner.delete(ctx)
    }
    fn copy(
        &self,
        ctx: &OperationContext,
        from: &str,
        to: &str,
        args: OpCopy,
        opts: OpCopier,
    ) -> opendal::Result<Self::Copier> {
        self.state.copies.fetch_add(1, Ordering::SeqCst);
        if self.state.force_copy.load(Ordering::SeqCst) {
            let (inner, ctx, from, to) = (
                self.inner.clone(),
                ctx.clone(),
                from.to_string(),
                to.to_string(),
            );
            return Ok(Box::new(oio::OneShotCopier::new(async move {
                use opendal::raw::oio::{Read as _, Write as _};
                let metadata = inner
                    .stat(&ctx, &from, OpStat::new())
                    .await?
                    .into_metadata();
                let reader = inner.read(&ctx, &from, OpRead::new())?;
                let (_, bytes) = reader
                    .read(opendal::BytesRange::new(0, Some(metadata.content_length())))
                    .await?;
                let mut write = OpWrite::new();
                if let Some(value) = metadata.content_type() {
                    write = write.with_content_type(value);
                }
                if let Some(value) = metadata.cache_control() {
                    write = write.with_cache_control(value);
                }
                let mut writer = inner.write(&ctx, &to, write)?;
                writer.write(bytes).await?;
                writer.close().await
            })));
        }
        self.inner.copy(ctx, from, to, args, opts)
    }
    fn write(
        &self,
        ctx: &OperationContext,
        path: &str,
        args: OpWrite,
    ) -> opendal::Result<Self::Writer> {
        self.state.writes.lock().unwrap().push(path.to_owned());
        if let Some(etag) = args.if_match() {
            self.state
                .conditional_writes
                .lock()
                .unwrap()
                .push((path.to_string(), etag.to_string()));
        }
        let mut refuse = self.state.refuse.lock().unwrap();
        if path != "_pub/lease.json" && refuse.as_ref().is_some_and(|suffix| path.ends_with(suffix))
        {
            refuse.take();
            return Err(opendal::Error::new(
                opendal::ErrorKind::PermissionDenied,
                "injected storage refusal",
            ));
        }
        drop(refuse);
        let counted = path.starts_with("_pub/")
            && path != "_pub/manifest.json"
            && path != "_pub/status.json"
            && path != "_pub/lease.json";
        let expected = args.if_match().map(str::to_string);
        let absent = args.if_not_exists() || args.if_none_match() == Some("*");
        let semantic = self.state.force_cas.load(Ordering::SeqCst);
        if let Some(metadata) = args.user_metadata()
            && self.state.force_metadata.load(Ordering::SeqCst)
        {
            self.state.metadata_writes.lock().unwrap().push((
                path.to_string(),
                metadata
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
            ));
        }
        let args = if semantic || self.state.force_metadata.load(Ordering::SeqCst) {
            let (native, _): (OpWrite, opendal::raw::OpWriter) = opendal::options::WriteOptions {
                content_type: args.content_type().map(str::to_string),
                cache_control: args.cache_control().map(str::to_string),
                if_not_exists: absent && !semantic,
                ..Default::default()
            }
            .into();
            native
        } else {
            args
        };
        let inner = self.inner.write(ctx, path, args)?;
        if counted {
            let active = self.state.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.state.maximum.fetch_max(active, Ordering::SeqCst);
        }
        Ok(ProbeWriter {
            path: path.to_string(),
            service: self.inner.clone(),
            ctx: ctx.clone(),
            expected,
            absent,
            manifest: path == "_pub/manifest.json",
            inner,
            state: self.state.clone(),
            counted,
        })
    }
}
struct ProbeWriter {
    path: String,
    service: Servicer,
    ctx: OperationContext,
    expected: Option<String>,
    absent: bool,
    manifest: bool,
    inner: oio::Writer,
    state: Arc<Probe>,
    counted: bool,
}
impl Drop for ProbeWriter {
    fn drop(&mut self) {
        if self.counted {
            self.state.active.fetch_sub(1, Ordering::SeqCst);
        }
    }
}
impl oio::Write for ProbeWriter {
    async fn write(&mut self, bytes: Buffer) -> opendal::Result<()> {
        if self.counted && self.state.stall.load(Ordering::SeqCst) {
            std::future::pending::<()>().await;
        }
        if self.counted && self.state.delay.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        self.inner.write(bytes).await
    }
    async fn close(&mut self) -> opendal::Result<Metadata> {
        if self.manifest && self.state.gate_manifest.load(Ordering::SeqCst) {
            self.state.manifest_entered.notify_one();
            self.state.manifest_release.notified().await;
        }
        let mode = if self.manifest {
            self.state.commit_mode.swap(0, Ordering::SeqCst)
        } else {
            0
        };
        if mode == 2 {
            tokio::time::sleep(Duration::from_secs(5)).await;
            return Err(opendal::Error::new(
                opendal::ErrorKind::PermissionDenied,
                "late PUT rejection",
            ));
        }
        let metadata = if self.state.force_cas.load(Ordering::SeqCst) {
            let mut etags = self.state.cas.lock().await;
            let injected = if self.manifest {
                self.state.inject_manifest.lock().unwrap().take()
            } else {
                None
            };
            if let Some((disk, bytes)) = injected {
                disk.write(&self.path, bytes).await?;
                let sequence = self.state.cas_sequence.fetch_add(1, Ordering::SeqCst);
                etags.insert(self.path.clone(), format!("\"cas-{sequence}\""));
            }
            let exists = match self
                .service
                .stat(&self.ctx, &self.path, OpStat::new())
                .await
            {
                Ok(_) => true,
                Err(failure) if failure.kind() == opendal::ErrorKind::NotFound => false,
                Err(failure) => return Err(failure),
            };
            let current = exists.then(|| {
                etags
                    .get(&self.path)
                    .map(String::as_str)
                    .unwrap_or("\"cas-external\"")
            });
            if self.absent && exists
                || self
                    .expected
                    .as_deref()
                    .is_some_and(|expected| Some(expected) != current)
            {
                if self.manifest {
                    self.state.cas_rejections.fetch_add(1, Ordering::SeqCst);
                }
                return Err(opendal::Error::new(
                    opendal::ErrorKind::ConditionNotMatch,
                    "semantic conditional write rejected",
                ));
            }
            let metadata = self.inner.close().await?;
            let sequence = self.state.cas_sequence.fetch_add(1, Ordering::SeqCst);
            etags.insert(self.path.clone(), format!("\"cas-{sequence}\""));
            metadata
        } else {
            self.inner.close().await?
        };
        if mode == 3 {
            self.state.fail_manifest_read.store(true, Ordering::SeqCst);
        }
        if mode == 1 || mode == 3 {
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
        Ok(metadata)
    }
    async fn abort(&mut self) -> opendal::Result<()> {
        self.inner.abort().await
    }
}

#[tokio::test]
async fn pub_003_status_survives_a_new_console_process_after_failed_publication() {
    let (p, disk) = publisher("v1");
    p.publish(vec![page("one")]).await.unwrap();
    let live = p.manifest().await.unwrap().live;
    assert!(
        p.publish_with(|| async {
            Err::<Vec<PageObject>, _>(FrameworkError::internal("render refused"))
        })
        .await
        .is_err()
    );
    let next_process = Publisher::from_operator(disk, "v1", PublishConfig::default()).unwrap();
    let status = next_process.status().await;
    assert!(status.last_outcome.contains("render refused"));
    assert_eq!(status.live_version, live);
    assert_eq!(status.pending, 0);
}

#[tokio::test]
async fn pub_002_explicit_no_store_page_is_never_published() {
    let (p, disk) = publisher("v1");
    let mut page = page("private");
    page.headers
        .insert("CACHE-CONTROL".into(), "private, no-store".into());
    assert!(
        p.publish(vec![page])
            .await
            .unwrap_err()
            .to_string()
            .contains("no-store")
    );
    assert!(!disk.exists("_pub/manifest.json").await.unwrap());
}

#[tokio::test]
async fn pub_002_full_inventory_drops_obsolete_pages_and_assets_after_grace() {
    let disk = memory();
    let p = Publisher::from_operator(disk.clone(), "v1", PublishConfig::default()).unwrap();
    let mut obsolete = page_at("/obsolete", "old");
    obsolete.assets.insert(
        "build/obsolete-abcd1234.js".into(),
        (
            Bytes::from_static(b"old script"),
            "application/javascript".into(),
        ),
    );
    p.publish(vec![page("keep"), obsolete]).await.unwrap();
    let initial = p.manifest().await.unwrap();
    let old_page = initial.versions[&initial.live]
        .objects
        .iter()
        .find(|object| object.url == "/obsolete")
        .unwrap()
        .path
        .clone();
    p.publish_partial(vec![page("keep")]).await.unwrap();
    assert_eq!(p.manifest().await.unwrap().live, initial.live);
    p.publish(vec![page("keep")]).await.unwrap();
    let mut manifest = p.manifest().await.unwrap();
    assert_eq!(manifest.versions[&manifest.live].objects.len(), 2);
    assert!(disk.exists(&old_page).await.unwrap());
    assert!(disk.exists("build/obsolete-abcd1234.js").await.unwrap());
    for (id, version) in &mut manifest.versions {
        if id != &manifest.live {
            version.retired_at_ms = Some(0);
        }
    }
    disk.write("_pub/manifest.json", serde_json::to_vec(&manifest).unwrap())
        .await
        .unwrap();
    let report = p.prune().await.unwrap();
    assert!(report.removed.contains(&old_page));
    assert!(
        report
            .removed
            .contains(&"build/obsolete-abcd1234.js".into())
    );
    assert!(!disk.exists(&old_page).await.unwrap());
    assert!(!disk.exists("build/obsolete-abcd1234.js").await.unwrap());
}

#[tokio::test]
async fn pub_003_cdn_switch_timeout_rolls_back_without_changing_manifest() {
    let disk = memory();
    let config = PublishConfig {
        timeout: Duration::from_millis(30),
        ..Default::default()
    };
    let driver = Arc::new(FailingCdn::default());
    let p = Publisher::from_operator(disk.clone(), "v1", config)
        .unwrap()
        .with_cdn(driver.clone());
    let first = p.publish(vec![page("one")]).await.unwrap();
    let before = manifest_bytes(&disk).await;
    driver.stall_switch.store(true, Ordering::SeqCst);
    assert!(
        p.publish(vec![page("two")])
            .await
            .unwrap_err()
            .to_string()
            .contains("switch timed out")
    );
    assert_eq!(manifest_bytes(&disk).await, before);
    assert_eq!(
        driver.events.lock().unwrap().last().unwrap(),
        &format!("rollback:{}", first.version)
    );
}

#[tokio::test]
async fn pub_002_representation_headers_match_final_bytes() {
    let (p, disk) = publisher("v1");
    let mut rendered = page("one");
    rendered
        .headers
        .insert("ETag".into(), "stale-render-etag".into());
    rendered.headers.insert("Content-Length".into(), "1".into());
    p.publish(vec![rendered]).await.unwrap();
    let manifest = p.manifest().await.unwrap();
    for object in &manifest.versions[&manifest.live].objects {
        assert_eq!(object.headers["etag"], format!("\"{}\"", object.digest));
        assert_eq!(
            object.headers["content-length"],
            disk.read(&object.path)
                .await
                .unwrap()
                .to_bytes()
                .len()
                .to_string()
        );
    }
}

#[tokio::test]
async fn pub_002_only_fingerprinted_assets_are_immutable() {
    let (p, _) = publisher("v1");
    let mut rendered = page("one");
    for name in [
        "favicon.svg",
        "manifest.webmanifest",
        "ordinaryfilename.js",
        "app-ABC12345.js",
        "app.ABC12345.js",
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef.webp",
    ] {
        rendered.assets.insert(
            name.into(),
            (
                Bytes::from_static(b"asset"),
                "application/octet-stream".into(),
            ),
        );
    }
    p.publish(vec![rendered]).await.unwrap();
    let manifest = p.manifest().await.unwrap();
    for object in &manifest.versions[&manifest.live].objects {
        let immutable = object.headers["cache-control"].contains("immutable");
        if object.url.contains("-ABC12345") {
            assert!(immutable);
        } else {
            assert!(!immutable, "{}", object.url);
        }
    }
}

fn with_icon(text: &str) -> PageObject {
    let mut rendered = page("page");
    rendered.assets.insert(
        "favicon.svg".into(),
        (Bytes::from(text.to_owned()), "image/svg+xml".into()),
    );
    rendered
}

#[tokio::test]
async fn pub_003_mutable_assets_use_versioned_keys_and_keep_previous_objects_on_failure() {
    for cdn in [false, true] {
        let disk = memory();
        let state = Arc::new(Probe::default());
        let mut p = Publisher::from_operator(
            disk.clone().layer(ProbeLayer(state.clone())),
            "v1",
            PublishConfig::default(),
        )
        .unwrap();
        if cdn {
            p = p.with_cdn(Arc::new(MemoryCdnDriver::default()));
        }
        p.publish(vec![with_icon("old icon")]).await.unwrap();
        let old_manifest = p.manifest().await.unwrap();
        let old_asset = old_manifest.versions[&old_manifest.live]
            .objects
            .iter()
            .find(|o| o.asset)
            .unwrap()
            .path
            .clone();
        assert!(!disk.exists("favicon.svg").await.unwrap());
        assert_eq!(
            disk.read(&old_asset).await.unwrap().to_bytes(),
            Bytes::from_static(b"old icon")
        );
        let before = manifest_bytes(&disk).await;
        for refusal in [".json", "_pub/manifest.json"] {
            *state.refuse.lock().unwrap() = Some(refusal.into());
            let mut changed = with_icon("new icon");
            changed.html = Bytes::from_static(b"changed page");
            changed.json = page("changed").json;
            assert!(p.publish(vec![changed]).await.is_err());
            assert_eq!(manifest_bytes(&disk).await, before);
            assert_eq!(
                disk.read(&old_asset).await.unwrap().to_bytes(),
                Bytes::from_static(b"old icon")
            );
        }
        p.publish(vec![with_icon("new icon")]).await.unwrap();
        let manifest = p.manifest().await.unwrap();
        let asset = manifest.versions[&manifest.live]
            .objects
            .iter()
            .find(|object| object.asset)
            .unwrap();
        assert_eq!(
            disk.read(&asset.path).await.unwrap().to_bytes(),
            Bytes::from_static(b"new icon")
        );
        assert!(asset.path.contains("/_assets/favicon.svg"));
        assert!(!asset.headers["cache-control"].contains("immutable"));
        p.publish(vec![page("page")]).await.unwrap();
        assert!(disk.exists(&old_asset).await.unwrap());
        let mut manifest = p.manifest().await.unwrap();
        for (id, version) in &mut manifest.versions {
            if id != &manifest.live {
                version.retired_at_ms = Some(0);
            }
        }
        disk.write("_pub/manifest.json", serde_json::to_vec(&manifest).unwrap())
            .await
            .unwrap();
        let report = p.prune().await.unwrap();
        assert!(report.removed.contains(&old_asset));
        assert!(!disk.exists(&old_asset).await.unwrap());
    }
}

#[tokio::test]
#[serial_test::serial]
async fn pub_003_fix_1_manifest_is_the_only_origin_switch_and_assets_exist() {
    use suprnova::static_publish::StaticPublication;
    use suprnova::{HttpResponse, InertiaProps, MiddlewareRegistry, Router};
    let _container = suprnova::testing::TestContainer::fake();
    #[derive(InertiaProps)]
    #[inertia(cacheable)]
    struct Props {
        #[inertia(public)]
        title: String,
    }
    let router: Router = Router::new()
        .get("/about", |_| async {
            Ok(HttpResponse::text("unexpected render"))
        })
        .get("/other", |_| async {
            Ok(HttpResponse::text("unexpected render"))
        })
        .into();
    let router = router
        .public_page::<Props>("/about", "About")
        .unwrap()
        .public_page::<Props>("/other", "About")
        .unwrap();
    let disk = memory();
    let state = Arc::new(Probe::default());
    let writer = Arc::new(
        Publisher::from_operator(
            disk.clone().layer(ProbeLayer(state.clone())),
            "v1",
            PublishConfig::default(),
        )
        .unwrap(),
    );
    let make = |path: &str, text: &str| {
        let mut page = page_at(path, text);
        page.html = Bytes::from(format!(
            r#"<html><link rel="stylesheet" href="/theme.css">{text}</html>"#
        ));
        page.assets.insert(
            "theme.css".into(),
            (Bytes::copy_from_slice(text.as_bytes()), "text/css".into()),
        );
        page
    };
    writer
        .publish(vec![make("/about", "old"), make("/other", "old")])
        .await
        .unwrap();
    StaticPublication::install(
        &router,
        Publisher::from_operator(disk.clone(), "v1", PublishConfig::default()).unwrap(),
    )
    .unwrap();
    let client = suprnova::testing::TestClient::new(router, MiddlewareRegistry::new());
    state.gate_manifest.store(true, Ordering::SeqCst);
    let publish = writer.clone();
    let work = tokio::spawn(async move {
        publish
            .publish(vec![make("/about", "new"), make("/other", "new")])
            .await
    });
    state.manifest_entered.notified().await;
    let old = client.get("/about").send().await;
    assert!(old.body_text().contains("old"));
    let html = old.body_text();
    let asset = html
        .split("href=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    assert_eq!(
        disk.read(asset.trim_start_matches('/'))
            .await
            .unwrap()
            .to_bytes(),
        Bytes::from_static(b"old"),
        "old document keeps its own asset before commit"
    );
    state.manifest_release.notify_one();
    work.await.unwrap().unwrap();
    for path in ["/about", "/other"] {
        let response = client.get(path).send().await;
        let html = response.body_text();
        assert!(html.contains("new"));
        let asset = html
            .split("href=\"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        assert!(
            asset.starts_with("/_pub/v1/"),
            "mutable assets use versioned keys: {asset}"
        );
        assert_eq!(
            disk.read(asset.trim_start_matches('/'))
                .await
                .unwrap()
                .to_bytes(),
            Bytes::from_static(b"new")
        );
        let delivered = client.get(asset).send().await;
        delivered.assert_ok();
        assert_eq!(delivered.body_text(), "new");
    }
}

#[tokio::test]
async fn pub_002_fix_h_refuses_vary_dimensions_missing_from_the_key() {
    for vary in [
        "*",
        "Accept-Language",
        "X-Inertia, Cookie",
        "X-Inertia-Partial-Data",
    ] {
        let (publisher, _) = publisher("v1");
        let mut page = page("public");
        page.headers.insert("Vary".into(), vary.into());
        let result = publisher.publish(vec![page]).await;
        assert!(result.is_err(), "unsupported Vary {vary} must refuse");
        assert!(result.unwrap_err().to_string().contains("Vary"));
    }
}

#[tokio::test]
async fn pub_003_fix_b_separate_publishers_refuse_an_occupied_storage_lease() {
    let disk = memory();
    let first =
        Arc::new(Publisher::from_operator(disk.clone(), "v1", PublishConfig::default()).unwrap());
    let second = Publisher::from_operator(disk.clone(), "v1", PublishConfig::default()).unwrap();
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let task = {
        let entered = entered.clone();
        let release = release.clone();
        tokio::spawn(async move {
            first
                .publish_with(|| async move {
                    entered.notify_one();
                    release.notified().await;
                    Ok(vec![page("first")])
                })
                .await
        })
    };
    entered.notified().await;
    let competing = second.publish(vec![page("second")]).await;
    release.notify_one();
    task.await.unwrap().unwrap();
    assert!(
        competing.is_err(),
        "separate instances must share a storage lease"
    );
    assert!(competing.unwrap_err().to_string().contains("lease"));
}

#[tokio::test]
async fn pub_003_fix_b_changed_manifest_is_not_overwritten() {
    let (publisher, disk) = publisher("v1");
    let foreign = serde_json::to_vec(&suprnova::static_publish::Manifest {
        live: "foreign".into(),
        ..Default::default()
    })
    .unwrap();
    let bytes = foreign.clone();
    let target = disk.clone();
    let result = publisher
        .publish_with(|| async move {
            target
                .write("_pub/manifest.json", bytes)
                .await
                .map_err(|e| FrameworkError::internal(e.to_string()))?;
            Ok(vec![page("ours")])
        })
        .await;
    assert!(
        result.is_err(),
        "a concurrent manifest write must refuse the commit"
    );
    assert_eq!(manifest_bytes(&disk).await, Bytes::from(foreign));
}

#[tokio::test]
async fn pub_002_fix_l_fingerprints_follow_the_shared_vite_pattern() {
    let (publisher, _) = publisher("v1");
    let mut page = page("public");
    for name in [
        "brand-background.svg",
        "site-manifest.json",
        "app-settings.js",
        "main-AbC123_x.js",
        "main-lowercas.js",
        "main.ABC12345.js",
    ] {
        page.assets.insert(
            name.into(),
            (
                Bytes::from_static(b"asset"),
                "application/octet-stream".into(),
            ),
        );
    }
    publisher.publish(vec![page]).await.unwrap();
    let manifest = publisher.manifest().await.unwrap();
    for object in manifest.versions[&manifest.live]
        .objects
        .iter()
        .filter(|o| o.asset)
    {
        assert_eq!(
            object.headers["cache-control"].contains("immutable"),
            object.url.ends_with("main-AbC123_x.js"),
            "{}",
            object.url
        );
    }
}

#[tokio::test]
async fn pub_002_fix_l_fingerprint_override_matches_static_files() {
    let (invalid, _) = publisher("v1");
    assert!(invalid.fingerprint("[").is_err());
    let (publisher, _) = publisher("v1");
    let publisher = publisher.fingerprint(r"\.release$").unwrap();
    let mut page = page("public");
    page.assets.insert(
        "main.release.js".into(),
        (
            Bytes::from_static(b"asset"),
            "application/javascript".into(),
        ),
    );
    publisher.publish(vec![page]).await.unwrap();
    let manifest = publisher.manifest().await.unwrap();
    assert!(
        manifest.versions[&manifest.live]
            .objects
            .iter()
            .find(|o| o.asset)
            .unwrap()
            .headers["cache-control"]
            .contains("immutable")
    );
}

#[tokio::test]
async fn pub_003_fix_b_conditional_backends_receive_the_manifest_etag() {
    let state = Arc::new(Probe::default());
    state.force_cas.store(true, Ordering::SeqCst);
    let publisher = Publisher::from_operator(
        memory().layer(ProbeLayer(state.clone())),
        "v1",
        PublishConfig::default(),
    )
    .unwrap();
    publisher.publish(vec![page("one")]).await.unwrap();
    publisher.publish(vec![page("two")]).await.unwrap();
    assert!(
        state
            .conditional_writes
            .lock()
            .unwrap()
            .iter()
            .any(|(path, etag)| path == "_pub/manifest.json" && etag.starts_with("\"cas-")),
        "a conditional backend must receive the original manifest ETag"
    );
}

#[tokio::test]
async fn pub_003_round2_2_memory_crashed_owner_is_recovered() {
    let (publisher, disk) = publisher("v1");
    disk.write(
        "_pub/lease.json",
        br#"{"owner":"previous","expires_at_ms":0}"#.as_slice(),
    )
    .await
    .unwrap();
    let result = publisher.publish(vec![page("ours")]).await;
    assert!(
        result.is_ok(),
        "an expired crashed owner must recover without operator action: {result:?}"
    );
    assert!(disk.exists("_pub/manifest.json").await.unwrap());
}

fn asset_router() -> suprnova::Router {
    #[derive(suprnova::InertiaProps)]
    #[inertia(cacheable)]
    struct Props {
        #[inertia(public)]
        title: String,
    }
    let router: suprnova::Router = suprnova::Router::new()
        .get("/about", |_| async {
            Ok(suprnova::HttpResponse::text("unexpected render"))
        })
        .into();
    router.public_page::<Props>("/about", "About").unwrap()
}

#[tokio::test]
#[serial_test::serial]
async fn pub_002_round2_4_asset_queries_bypass_page_private_rules() {
    let _container = suprnova::testing::TestContainer::fake();
    let router = asset_router();
    let (publisher, disk) = publisher("v1");
    let mut rendered = page("public");
    rendered.html = Bytes::from_static(br#"<link rel="stylesheet" href="/theme.css?v=1">"#);
    rendered.assets.insert(
        "theme.css".into(),
        (Bytes::from_static(b"body{}"), "text/css".into()),
    );
    publisher.publish(vec![rendered]).await.unwrap();
    let manifest = publisher.manifest().await.unwrap();
    let asset = manifest.versions[&manifest.live]
        .objects
        .iter()
        .find(|object| object.asset)
        .unwrap();
    suprnova::static_publish::StaticPublication::install(
        &router,
        Publisher::from_operator(disk, "v1", PublishConfig::default()).unwrap(),
    )
    .unwrap();
    let client = suprnova::testing::TestClient::new(router, suprnova::MiddlewareRegistry::new());
    let response = client
        .get(format!("/{}?v=1", asset.path))
        .header("Cache-Control", "no-store")
        .header("X-Inertia-Partial-Data", "title")
        .send()
        .await;
    response.assert_ok();
    assert_eq!(response.body_text(), "body{}");
    assert_eq!(
        response.header("Cache-Control"),
        Some(asset.headers["cache-control"].as_str())
    );
}

#[tokio::test]
#[serial_test::serial]
async fn pub_002_round2_7_origin_assets_stream_in_bounded_chunks() {
    let _container = suprnova::testing::TestContainer::fake();
    let router = asset_router();
    let disk = memory();
    let state = Arc::new(Probe::default());
    let disk = disk.layer(ProbeLayer(state.clone()));
    let publisher = Publisher::from_operator(disk.clone(), "v1", PublishConfig::default()).unwrap();
    let mut rendered = page("public");
    let bytes = Bytes::from(vec![b'z'; 3 * 1024 * 1024]);
    rendered.assets.insert(
        "large.bin".into(),
        (bytes.clone(), "application/octet-stream".into()),
    );
    publisher.publish(vec![rendered]).await.unwrap();
    let manifest = publisher.manifest().await.unwrap();
    let asset = manifest.versions[&manifest.live]
        .objects
        .iter()
        .find(|object| object.asset)
        .unwrap();
    suprnova::static_publish::StaticPublication::install(
        &router,
        Publisher::from_operator(disk, "v1", PublishConfig::default()).unwrap(),
    )
    .unwrap();
    use http_body_util::BodyExt;
    use hyper::body::Body;
    use hyper_util::rt::TokioIo;
    let streamed = Arc::new(AtomicBool::new(false));
    let observed = streamed.clone();
    let (server_io, client_io) = tokio::io::duplex(65536);
    let router = Arc::new(router);
    let registry = Arc::new(suprnova::MiddlewareRegistry::new());
    let service = hyper::service::service_fn(move |request| {
        let (router, registry, observed) = (router.clone(), registry.clone(), observed.clone());
        async move {
            let response = suprnova::handle_request(router, registry, request).await;
            observed.store(
                response.body().size_hint().exact().is_none(),
                Ordering::SeqCst,
            );
            Ok::<_, std::convert::Infallible>(response)
        }
    });
    let server = tokio::spawn(async move {
        hyper::server::conn::http1::Builder::new()
            .serve_connection(TokioIo::new(server_io), service)
            .await
    });
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(client_io))
        .await
        .unwrap();
    let connection = tokio::spawn(connection);
    let request = hyper::Request::builder()
        .uri(format!("/{}", asset.path))
        .body(http_body_util::Empty::<Bytes>::new())
        .unwrap();
    let response = sender.send_request(request).await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-length"], "3145728");
    assert_eq!(
        response.into_body().collect().await.unwrap().to_bytes(),
        bytes
    );
    assert!(
        streamed.load(Ordering::SeqCst),
        "origin must return a streaming body before collection"
    );
    assert!(
        state.maximum_asset_read.load(Ordering::SeqCst) <= 65536,
        "asset reads must stay bounded"
    );
    server.abort();
    connection.abort();
}

struct ProbeReader {
    inner: oio::Reader,
    state: Arc<Probe>,
    tracked: bool,
}
impl oio::Read for ProbeReader {
    async fn open(
        &self,
        range: opendal::BytesRange,
    ) -> opendal::Result<(opendal::raw::RpRead, Box<dyn oio::ReadStreamDyn>)> {
        self.inner.open(range).await
    }
    async fn read(
        &self,
        range: opendal::BytesRange,
    ) -> opendal::Result<(opendal::raw::RpRead, Buffer)> {
        if self.tracked {
            self.state
                .maximum_asset_read
                .fetch_max(range.size().unwrap_or(u64::MAX), Ordering::SeqCst);
        }
        self.inner.read(range).await
    }
}

#[tokio::test]
async fn pub_002_round2_5_metadata_excludes_representation_headers() {
    let state = Arc::new(Probe::default());
    state.force_metadata.store(true, Ordering::SeqCst);
    let disk = memory();
    let p = Publisher::from_operator(
        disk.clone().layer(ProbeLayer(state.clone())),
        "v1",
        PublishConfig::default(),
    )
    .unwrap();
    let mut rendered = page("public");
    rendered
        .headers
        .insert("content-security-policy".into(), "script-src 'self'".into());
    rendered.headers.insert(
        "strict-transport-security".into(),
        "max-age=31536000".into(),
    );
    p.publish(vec![rendered]).await.unwrap();
    let writes = state.metadata_writes.lock().unwrap();
    let (key, metadata) = writes
        .iter()
        .find(|(key, _)| key.starts_with("_pub/") && key.ends_with(".html"))
        .unwrap();
    assert!(metadata.contains_key("content-security-policy"));
    assert!(metadata.contains_key("strict-transport-security"));
    for name in ["content-type", "content-length", "etag", "content-encoding"] {
        assert!(
            !metadata.contains_key(name),
            "representation header {name} cannot be policy metadata"
        );
    }
    assert_eq!(
        disk.stat(key).await.unwrap().content_type(),
        Some("text/html; charset=utf-8")
    );
}

#[tokio::test]
#[serial_test::serial]
async fn pub_002_round2_3_mutable_dependencies_share_the_new_revision() {
    let _container = suprnova::testing::TestContainer::fake();
    for disable_copy in [false, true] {
        let router = asset_router();
        let state = Arc::new(Probe::default());
        state.force_copy.store(true, Ordering::SeqCst);
        state.disable_copy.store(disable_copy, Ordering::SeqCst);
        let disk = memory().layer(ProbeLayer(state.clone()));
        let p = Publisher::from_operator(disk.clone(), "v1", PublishConfig::default()).unwrap();
        let make = |css: &'static [u8]| {
            let mut page = page("public");
            page.html = Bytes::from_static(br#"<link rel="stylesheet" href="/theme.css">"#);
            for (key, bytes, mime) in [
                ("theme.css", css, "text/css"),
                ("bg.png", b"pixels".as_slice(), "image/png"),
                ("bg-AbC12345.png", b"fingerprint".as_slice(), "image/png"),
            ] {
                page.assets
                    .insert(key.into(), (Bytes::from_static(bytes), mime.into()));
            }
            page
        };
        p.publish(vec![make(b"body{background:url(./bg.png)}")])
            .await
            .unwrap();
        let second = p
            .publish(vec![make(
                b"body{background:url(./bg.png),url(./bg-AbC12345.png);color:red}",
            )])
            .await
            .unwrap();
        let manifest = p.manifest().await.unwrap();
        let assets = &manifest.versions[&manifest.live].objects;
        let bg = assets.iter().find(|o| o.url == "/bg.png").unwrap();
        assert_eq!(bg.path, format!("_pub/{}/_assets/bg.png", second.version));
        assert!(assets.iter().any(|o| o.path == "bg-AbC12345.png"));
        suprnova::static_publish::StaticPublication::install(
            &router,
            Publisher::from_operator(disk, "v1", PublishConfig::default()).unwrap(),
        )
        .unwrap();
        let client =
            suprnova::testing::TestClient::new(router, suprnova::MiddlewareRegistry::new());
        for (key, body) in [("bg.png", "pixels"), ("bg-AbC12345.png", "fingerprint")] {
            let response = client
                .get(format!("/_pub/{}/_assets/{key}", second.version))
                .send()
                .await;
            response.assert_ok();
            assert_eq!(response.body_text(), body);
        }
        if disable_copy {
            assert_eq!(state.copies.load(Ordering::SeqCst), 0);
        } else {
            assert!(
                state.copies.load(Ordering::SeqCst) > 0,
                "reuse should use a server-side copy"
            );
        }
    }
}

#[path = "fencing.rs"]
mod fencing;
