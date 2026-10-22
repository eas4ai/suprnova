//! PUB-008 cleanup covers retired versions and failed staging objects.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use bytes::Bytes;
use opendal::raw::{
    Layer, OpCopier, OpCopy, OpCreateDir, OpList, OpPresign, OpRead, OpRename, OpStat, OpWrite,
    RpCreateDir, RpPresign, RpRename, RpStat, Service, ServiceInfo, Servicer, oio,
};
use opendal::{Capability, Error, ErrorKind, OperationContext};
use suprnova::static_publish::{PageObject, PublishConfig, Publisher};

fn page(text: &'static str) -> PageObject {
    let mut page = PageObject::new(
        "/about",
        Bytes::from(text),
        Bytes::from_static(
            b"{\"component\":\"About\",\"props\":{},\"url\":\"/about\",\"version\":\"v1\"}",
        ),
    )
    .expect("page");
    page.assets = BTreeMap::from([(
        "assets/app-0123456789.js".into(),
        (
            Bytes::from_static(b"console.log(1)"),
            "application/javascript".into(),
        ),
    )]);
    page
}

fn local() -> (tempfile::TempDir, opendal::Operator) {
    let directory = tempfile::tempdir().expect("publication root");
    let operator = opendal::Operator::new(
        opendal::services::Fs::default().root(directory.path().to_str().expect("root path")),
    )
    .expect("filesystem operator");
    (directory, operator)
}

fn old(directory: &tempfile::TempDir, key: &str) {
    std::fs::File::options()
        .write(true)
        .open(directory.path().join(key))
        .expect("orphan file")
        .set_times(
            std::fs::FileTimes::new().set_modified(SystemTime::now() - Duration::from_secs(3600)),
        )
        .expect("age orphan");
}

#[tokio::test]
async fn pub_008_command_removes_old_orphans_assets_and_pages_and_reports_kept() {
    let (directory, disk) = local();
    let config = PublishConfig {
        grace_period: Duration::from_secs(60),
        ..PublishConfig::default()
    };
    let publisher = Publisher::from_operator(disk.clone(), "v1", config).expect("publisher");
    publisher
        .publish(vec![page("live")])
        .await
        .expect("publish live");
    for key in [
        "assets/orphan-aaaaaaaa.js",
        "_pub/v0/failed/about.html",
        "gone/index.json",
    ] {
        disk.write(key, "orphan").await.expect("orphan object");
        old(&directory, key);
    }
    disk.write("assets/recent-bbbbbbbb.js", "young")
        .await
        .expect("young orphan");
    let live = publisher.manifest().await.expect("manifest");
    let report = publisher.prune().await.expect("prune command pass");
    assert_eq!(
        report.removed,
        [
            "_pub/v0/failed/about.html",
            "assets/orphan-aaaaaaaa.js",
            "gone/index.json"
        ]
    );
    assert!(report.kept.contains(&"assets/recent-bbbbbbbb.js".into()));
    assert!(report.kept.contains(&"about/index.html".into()));
    assert!(report.kept.contains(&"about/index.json".into()));
    for object in &live.versions[&live.live].objects {
        assert!(report.kept.contains(&object.path));
        assert!(disk.exists(&object.path).await.expect("live object exists"));
    }
    for key in &report.removed {
        assert!(!disk.exists(key).await.expect("removed object absent"));
    }
    assert_eq!(publisher.manifest().await.expect("live manifest"), live);
}

#[tokio::test]
async fn pub_008_publication_automatically_prunes_old_unreferenced_objects() {
    let (directory, disk) = local();
    disk.write("assets/failed-aaaaaaaa.png", "failed staging")
        .await
        .expect("orphan");
    old(&directory, "assets/failed-aaaaaaaa.png");
    let publisher = Publisher::from_operator(
        disk.clone(),
        "v1",
        PublishConfig {
            grace_period: Duration::from_secs(60),
            ..PublishConfig::default()
        },
    )
    .expect("publisher");
    let outcome = publisher
        .publish(vec![page("live")])
        .await
        .expect("publication");
    assert_eq!(outcome.prune.removed, ["assets/failed-aaaaaaaa.png"]);
    assert!(
        !disk
            .exists("assets/failed-aaaaaaaa.png")
            .await
            .expect("removed")
    );
    assert!(disk.exists("about/index.html").await.expect("active alias"));
}

#[tokio::test]
async fn pub_008_grace_period_keeps_previous_revision_then_removes_expired_pages() {
    let disk = opendal::Operator::new(opendal::services::Memory::default()).expect("memory");
    let publisher =
        Publisher::from_operator(disk.clone(), "v1", PublishConfig::default()).expect("publisher");
    let first = publisher
        .publish(vec![page("one")])
        .await
        .expect("first revision");
    publisher
        .publish(vec![page("two")])
        .await
        .expect("second revision");
    let mut manifest = publisher.manifest().await.expect("manifest");
    let previous = manifest.versions[&first.version].objects.clone();
    let report = publisher.prune().await.expect("grace prune");
    for object in &previous {
        assert!(report.kept.contains(&object.path));
    }
    manifest
        .versions
        .get_mut(&first.version)
        .expect("previous revision")
        .retired_at_ms = Some(0);
    disk.write(
        "_pub/manifest.json",
        serde_json::to_vec(&manifest).expect("manifest bytes"),
    )
    .await
    .expect("age previous revision");
    let report = publisher.prune().await.expect("expired prune");
    let live_paths: Vec<_> = manifest.versions[&manifest.live]
        .objects
        .iter()
        .map(|object| &object.path)
        .collect();
    for object in &previous {
        if !live_paths.contains(&&object.path) {
            assert!(report.removed.contains(&object.path));
        }
    }
    assert!(
        !publisher
            .manifest()
            .await
            .expect("updated manifest")
            .versions
            .contains_key(&first.version)
    );
}

#[tokio::test]
async fn pub_008_unknown_orphan_timestamp_is_kept() {
    let disk = opendal::Operator::new(opendal::services::Memory::default()).expect("memory");
    disk.write("assets/orphan-aaaaaaaa.js", "unknown age")
        .await
        .expect("orphan");
    let publisher =
        Publisher::from_operator(disk.clone(), "v1", PublishConfig::default()).expect("publisher");
    let report = publisher.prune().await.expect("prune");
    assert!(report.kept.contains(&"assets/orphan-aaaaaaaa.js".into()));
    assert!(report.removed.is_empty());
}

#[derive(Debug)]
struct FailDelete;
impl Layer for FailDelete {
    fn apply_service(&self, inner: Servicer) -> Servicer {
        Arc::new(FailDeleteService { inner })
    }
}

#[derive(Debug)]
struct FailDeleteService {
    inner: Servicer,
}
impl Service for FailDeleteService {
    type Reader = oio::Reader;
    type Writer = oio::Writer;
    type Lister = oio::Lister;
    type Deleter = oio::Deleter;
    type Copier = oio::Copier;
    fn info(&self) -> ServiceInfo {
        self.inner.info()
    }
    fn capability(&self) -> Capability {
        self.inner.capability()
    }
    async fn create_dir(
        &self,
        ctx: &OperationContext,
        path: &str,
        args: OpCreateDir,
    ) -> opendal::Result<RpCreateDir> {
        self.inner.create_dir(ctx, path, args).await
    }
    fn copy(
        &self,
        ctx: &OperationContext,
        from: &str,
        to: &str,
        args: OpCopy,
        options: OpCopier,
    ) -> opendal::Result<Self::Copier> {
        self.inner.copy(ctx, from, to, args, options)
    }
    async fn rename(
        &self,
        ctx: &OperationContext,
        from: &str,
        to: &str,
        args: OpRename,
    ) -> opendal::Result<RpRename> {
        self.inner.rename(ctx, from, to, args).await
    }
    async fn presign(
        &self,
        ctx: &OperationContext,
        path: &str,
        args: OpPresign,
    ) -> opendal::Result<RpPresign> {
        self.inner.presign(ctx, path, args).await
    }
    fn read(
        &self,
        ctx: &OperationContext,
        path: &str,
        args: OpRead,
    ) -> opendal::Result<Self::Reader> {
        self.inner.read(ctx, path, args)
    }
    fn write(
        &self,
        ctx: &OperationContext,
        path: &str,
        args: OpWrite,
    ) -> opendal::Result<Self::Writer> {
        self.inner.write(ctx, path, args)
    }
    fn list(
        &self,
        ctx: &OperationContext,
        path: &str,
        args: OpList,
    ) -> opendal::Result<Self::Lister> {
        self.inner.list(ctx, path, args)
    }
    fn delete(&self, _: &OperationContext) -> opendal::Result<Self::Deleter> {
        Err(Error::new(
            ErrorKind::PermissionDenied,
            "injected delete failure",
        ))
    }
    async fn stat(
        &self,
        ctx: &OperationContext,
        path: &str,
        args: OpStat,
    ) -> opendal::Result<RpStat> {
        self.inner.stat(ctx, path, args).await
    }
}

#[tokio::test]
async fn pub_008_failed_delete_leaves_live_manifest_and_objects_serving() {
    let (directory, disk) = local();
    let publisher =
        Publisher::from_operator(disk.clone(), "v1", PublishConfig::default()).expect("publisher");
    publisher
        .publish(vec![page("live")])
        .await
        .expect("live revision");
    disk.write("assets/orphan-aaaaaaaa.js", "orphan")
        .await
        .expect("orphan");
    old(&directory, "assets/orphan-aaaaaaaa.js");
    let before = publisher.manifest().await.expect("live manifest");
    let failing = Publisher::from_operator(
        disk.clone().layer(FailDelete),
        "v1",
        PublishConfig {
            grace_period: Duration::from_secs(60),
            ..PublishConfig::default()
        },
    )
    .expect("failing publisher");
    let error = failing.prune().await.expect_err("injected delete failure");
    assert!(error.to_string().contains("injected delete failure"));
    assert_eq!(
        publisher.manifest().await.expect("manifest preserved"),
        before
    );
    for object in &before.versions[&before.live].objects {
        assert!(
            disk.exists(&object.path)
                .await
                .expect("live object preserved")
        );
    }
    assert_eq!(
        disk.read("about/index.html")
            .await
            .expect("live alias")
            .to_bytes(),
        Bytes::from_static(b"live")
    );
}
