#![cfg(feature = "filesystem")]

//! `copy_between_disks` must not leave a partial destination object when the
//! transfer fails mid-stream.
//!
//! The source disk here is a memory backend wrapped in a layer whose reader
//! yields exactly one chunk and then errors - simulating a source that fails
//! after the destination writer has already received data. The destination is
//! a real filesystem disk, so a partial write is visible on disk: the test
//! proves the file is gone after the failed copy.

use opendal::raw::{
    Layer, OpCopier, OpCopy, OpCreateDir, OpList, OpPresign, OpRead, OpRename, OpStat, OpWrite,
    RpCreateDir, RpPresign, RpRead, RpRename, RpStat, Service, ServiceInfo, Servicer, oio,
};
use opendal::{
    Buffer, BytesRange, Capability, Error, ErrorKind, Metadata, OperationContext, Result,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use suprnova::filesystem::streaming::copy_between_disks;
use suprnova::{ReadThroughConfig, Storage};

/// Layer whose reader returns one 1 KiB chunk and then fails on the next read.
#[derive(Debug, Clone, Copy)]
struct FailAfterOneChunkLayer;

impl Layer for FailAfterOneChunkLayer {
    fn apply_service(&self, inner: Servicer) -> Servicer {
        Arc::new(FailService { inner })
    }
}

#[derive(Debug)]
struct FailService {
    inner: Servicer,
}

impl Service for FailService {
    type Reader = FailReader;
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
    ) -> Result<RpCreateDir> {
        self.inner.create_dir(ctx, path, args).await
    }

    async fn stat(&self, ctx: &OperationContext, path: &str, args: OpStat) -> Result<RpStat> {
        self.inner.stat(ctx, path, args).await
    }

    fn read(&self, _ctx: &OperationContext, _path: &str, _args: OpRead) -> Result<Self::Reader> {
        Ok(FailReader)
    }

    fn write(&self, ctx: &OperationContext, path: &str, args: OpWrite) -> Result<Self::Writer> {
        self.inner.write(ctx, path, args)
    }

    fn delete(&self, ctx: &OperationContext) -> Result<Self::Deleter> {
        self.inner.delete(ctx)
    }

    fn list(&self, ctx: &OperationContext, path: &str, args: OpList) -> Result<Self::Lister> {
        self.inner.list(ctx, path, args)
    }

    fn copy(
        &self,
        ctx: &OperationContext,
        from: &str,
        to: &str,
        args: OpCopy,
        opts: OpCopier,
    ) -> Result<Self::Copier> {
        self.inner.copy(ctx, from, to, args, opts)
    }

    async fn rename(
        &self,
        ctx: &OperationContext,
        from: &str,
        to: &str,
        args: OpRename,
    ) -> Result<RpRename> {
        self.inner.rename(ctx, from, to, args).await
    }

    async fn presign(
        &self,
        ctx: &OperationContext,
        path: &str,
        args: OpPresign,
    ) -> Result<RpPresign> {
        self.inner.presign(ctx, path, args).await
    }
}

struct FailReader;

impl oio::Read for FailReader {
    async fn open(&self, _range: BytesRange) -> Result<(RpRead, Box<dyn oio::ReadStreamDyn>)> {
        Ok((
            RpRead::new(Metadata::default()),
            Box::new(FailReadStream { sent_chunk: false }),
        ))
    }

    async fn read(&self, _range: BytesRange) -> Result<(RpRead, Buffer)> {
        Err(Error::new(
            ErrorKind::Unexpected,
            "injected direct read failure",
        ))
    }
}

struct FailReadStream {
    sent_chunk: bool,
}

impl oio::ReadStream for FailReadStream {
    async fn read(&mut self) -> Result<Buffer> {
        if self.sent_chunk {
            Err(Error::new(
                ErrorKind::Unexpected,
                "injected mid-stream read failure",
            ))
        } else {
            self.sent_chunk = true;
            Ok(Buffer::from(vec![0u8; 1024]))
        }
    }
}

/// Layer that lets the first destination write through and stalls every
/// later write forever. The transfer parks mid-flight with an unclosed
/// writer after observable progress, so aborting the copy task lands
/// deterministically mid-transfer instead of racing completion.
#[derive(Debug, Clone)]
struct GateWritesLayer {
    writes: Arc<std::sync::atomic::AtomicUsize>,
    close_gate: Option<(bool, Arc<AtomicUsize>)>,
}

impl Layer for GateWritesLayer {
    fn apply_service(&self, inner: Servicer) -> Servicer {
        Arc::new(GateWritesService {
            inner,
            writes: self.writes.clone(),
            close_gate: self.close_gate.clone(),
        })
    }
}

#[derive(Debug)]
struct GateWritesService {
    inner: Servicer,
    writes: Arc<std::sync::atomic::AtomicUsize>,
    close_gate: Option<(bool, Arc<AtomicUsize>)>,
}

impl Service for GateWritesService {
    type Reader = oio::Reader;
    type Writer = GateWriter;
    type Lister = oio::Lister;
    type Deleter = oio::Deleter;
    type Copier = GateCopier;

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
    ) -> Result<RpCreateDir> {
        self.inner.create_dir(ctx, path, args).await
    }

    async fn stat(&self, ctx: &OperationContext, path: &str, args: OpStat) -> Result<RpStat> {
        self.inner.stat(ctx, path, args).await
    }

    fn read(&self, ctx: &OperationContext, path: &str, args: OpRead) -> Result<Self::Reader> {
        self.inner.read(ctx, path, args)
    }

    fn write(&self, ctx: &OperationContext, path: &str, args: OpWrite) -> Result<Self::Writer> {
        Ok(GateWriter {
            inner: self.inner.write(ctx, path, args)?,
            writes: self.writes.clone(),
            close_gate: self.close_gate.clone(),
        })
    }

    fn delete(&self, ctx: &OperationContext) -> Result<Self::Deleter> {
        self.inner.delete(ctx)
    }

    fn list(&self, ctx: &OperationContext, path: &str, args: OpList) -> Result<Self::Lister> {
        self.inner.list(ctx, path, args)
    }

    fn copy(
        &self,
        ctx: &OperationContext,
        from: &str,
        to: &str,
        args: OpCopy,
        opts: OpCopier,
    ) -> Result<Self::Copier> {
        Ok(GateCopier {
            inner: self.inner.copy(ctx, from, to, args, opts)?,
            reached: self.close_gate.as_ref().map(|(_, reached)| reached.clone()),
        })
    }

    async fn rename(
        &self,
        ctx: &OperationContext,
        from: &str,
        to: &str,
        args: OpRename,
    ) -> Result<RpRename> {
        self.inner.rename(ctx, from, to, args).await
    }

    async fn presign(
        &self,
        ctx: &OperationContext,
        path: &str,
        args: OpPresign,
    ) -> Result<RpPresign> {
        self.inner.presign(ctx, path, args).await
    }
}

struct GateWriter {
    inner: oio::Writer,
    writes: Arc<std::sync::atomic::AtomicUsize>,
    close_gate: Option<(bool, Arc<AtomicUsize>)>,
}

struct GateCopier {
    inner: oio::Copier,
    reached: Option<Arc<AtomicUsize>>,
}

impl oio::Copy for GateCopier {
    async fn next(&mut self) -> Result<Option<usize>> {
        self.inner.next().await
    }

    async fn close(&mut self) -> Result<Metadata> {
        if let Some(reached) = &self.reached {
            while self.inner.next().await?.is_some() {}
            reached.store(1, Ordering::SeqCst);
            std::future::pending::<()>().await;
        }
        self.inner.close().await
    }

    async fn abort(&mut self) -> Result<()> {
        self.inner.abort().await
    }
}

impl oio::Write for GateWriter {
    async fn write(&mut self, bs: Buffer) -> Result<()> {
        // Counted only after the inner write completes, so `writes == 1`
        // proves bytes reached the backend - not merely that `write()`
        // was called. (`Writer` is `&mut`-exclusive, so no two writes
        // race here.)
        if self.writes.load(std::sync::atomic::Ordering::SeqCst) == 0 {
            let outcome = self.inner.write(bs).await;
            self.writes
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            outcome
        } else {
            // Never resolves: the transfer parks here until aborted.
            std::future::pending().await
        }
    }

    async fn close(&mut self) -> Result<Metadata> {
        if let Some((close_first, reached)) = &self.close_gate {
            if *close_first {
                self.inner.close().await?;
            }
            reached.store(1, Ordering::SeqCst);
            while reached.load(Ordering::SeqCst) == 1 {
                tokio::task::yield_now().await;
            }
            if reached.load(Ordering::SeqCst) == 4 {
                return Err(Error::new(ErrorKind::Unexpected, "injected close failure"));
            }
        }
        self.inner.close().await
    }

    async fn abort(&mut self) -> Result<()> {
        let result = self.inner.abort().await;
        if let Some((_, reached)) = &self.close_gate {
            reached.store(2, Ordering::SeqCst);
        }
        result
    }
}

/// Count the staged (not yet published) files of an atomic fs disk.
fn staging_file_count(root: &std::path::Path) -> usize {
    std::fs::read_dir(root.join(suprnova::filesystem::ATOMIC_STAGING_DIR))
        .map(|entries| entries.count())
        .unwrap_or(0)
}

/// P4-08: cancelling the copy task mid-transfer must divert the same
/// abort+delete cleanup the error path runs. The write gate parks the
/// transfer after its first observable write with the writer unclosed,
/// so the abort lands deterministically mid-flight.
#[tokio::test]
async fn copy_cleans_up_staging_and_destination_on_cancel() {
    use std::sync::atomic::Ordering;

    let _guard = Storage::fake();
    let tmp = tempfile::tempdir().expect("create tempdir");

    // Source: plain memory disk holding a real multi-chunk object.
    Storage::register_memory("cancel_src");
    Storage::disk("cancel_src")
        .expect("src disk")
        .write("big.bin", vec![0xABu8; 4 * 1024 * 1024])
        .await
        .expect("seed source");

    // Destination: fs disk whose second write stalls forever.
    let writes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    Storage::register_fs_with("cancel_fs_dest", tmp.path(), {
        let writes = writes.clone();
        move |op| {
            op.layer(GateWritesLayer {
                writes: writes.clone(),
                close_gate: None,
            })
        }
    })
    .expect("fs dest");

    let handle = tokio::spawn(copy_between_disks(
        "cancel_src",
        "big.bin",
        "cancel_fs_dest",
        "cancelled.bin",
    ));

    // The first write must land before the abort: proves the transfer is
    // parked mid-flight with an unclosed writer.
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while writes.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the gated transfer must perform its first write");

    handle.abort();
    let _ = handle.await;

    // The diverted cleanup runs detached: staged state must disappear,
    // the destination must never materialize, and no further write may
    // land after the abort.
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while staging_file_count(tmp.path()) != 0 || tmp.path().join("cancelled.bin").exists() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("cancelled copy must leave no staged chunk and no destination");
    assert_eq!(
        writes.load(Ordering::SeqCst),
        1,
        "no further destination write may land after the abort"
    );
}

/// Exercise the public read-through copy and move paths with a real fs primary.
async fn cancel_read_through_transfer(rename: bool, destination_exists: bool) {
    let _guard = Storage::fake();
    let tmp = tempfile::tempdir().expect("create tempdir");
    Storage::register_memory("fallback");
    let fallback = Storage::disk("fallback").expect("fallback disk");
    fallback
        .write("source.bin", vec![0xAB; 4 * 1024 * 1024])
        .await
        .expect("seed source");
    if destination_exists {
        std::fs::write(tmp.path().join("destination.bin"), b"original")
            .expect("seed existing destination");
    }
    let writes = Arc::new(AtomicUsize::new(0));
    Storage::register_fs_with("primary", tmp.path(), {
        let writes = writes.clone();
        move |op| {
            op.layer(GateWritesLayer {
                writes,
                close_gate: None,
            })
        }
    })
    .expect("register primary");
    Storage::register_read_through(
        "assets",
        ReadThroughConfig {
            primary: "primary".into(),
            fallback: "fallback".into(),
            ..Default::default()
        },
    )
    .expect("register read-through");
    let assets = Storage::disk("assets").expect("read-through disk");
    let transfer = tokio::spawn(async move {
        if rename {
            assets.rename("source.bin", "destination.bin").await
        } else {
            assets
                .copy("source.bin", "destination.bin")
                .await
                .map(|_| ())
        }
    });
    tokio::time::timeout(Duration::from_secs(10), async {
        while writes.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("first write reaches the primary");
    assert_ne!(
        staging_file_count(tmp.path()),
        0,
        "test reaches staged state"
    );
    transfer.abort();
    assert!(
        transfer
            .await
            .expect_err("transfer is cancelled")
            .is_cancelled()
    );
    tokio::time::timeout(Duration::from_secs(2), async {
        while staging_file_count(tmp.path()) != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("cancelled read-through transfer removes staged data");
    assert!(fallback.exists("source.bin").await.expect("source exists"));
    if destination_exists {
        assert_eq!(
            std::fs::read(tmp.path().join("destination.bin"))
                .expect("existing destination remains"),
            b"original"
        );
    } else {
        assert!(!tmp.path().join("destination.bin").exists());
    }
}

#[tokio::test]
async fn read_through_copy_cleans_staging_on_cancel() {
    cancel_read_through_transfer(false, false).await;
}

#[tokio::test]
async fn read_through_copy_preserves_existing_destination_on_cancel() {
    cancel_read_through_transfer(false, true).await;
}

#[tokio::test]
async fn read_through_rename_cleans_staging_on_cancel() {
    cancel_read_through_transfer(true, false).await;
}

#[tokio::test]
async fn read_through_rename_preserves_existing_destination_on_cancel() {
    cancel_read_through_transfer(true, true).await;
}

async fn cancel_read_through_promotion(close_first: bool) {
    let _guard = Storage::fake();
    let tmp = tempfile::tempdir().expect("create tempdir");
    Storage::register_memory("fallback");
    let fallback = Storage::disk("fallback").expect("fallback disk");
    fallback
        .write("source.bin", "cold bytes")
        .await
        .expect("seed source");
    let reached = Arc::new(AtomicUsize::new(0));
    Storage::register_fs_with("primary", tmp.path(), {
        let reached = reached.clone();
        move |op| {
            op.layer(GateWritesLayer {
                writes: Arc::new(AtomicUsize::new(0)),
                close_gate: Some((close_first, reached)),
            })
        }
    })
    .expect("register primary");
    Storage::register_read_through(
        "assets",
        ReadThroughConfig {
            primary: "primary".into(),
            fallback: "fallback".into(),
            ..Default::default()
        },
    )
    .expect("register read-through");
    let assets = Storage::disk("assets").expect("read-through disk");
    let promotion = tokio::spawn(async move { assets.read("source.bin").await });
    tokio::time::timeout(Duration::from_secs(10), async {
        while reached.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("promotion reaches close gate");
    // A concurrent primary write wins, and cleanup must never delete it.
    std::fs::write(tmp.path().join("source.bin"), b"concurrent winner").expect("publish winner");
    promotion.abort();
    assert!(
        promotion
            .await
            .expect_err("promotion is cancelled")
            .is_cancelled()
    );
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let siblings = std::fs::read_dir(tmp.path())
                .expect("primary directory")
                .map(|entry| entry.expect("directory entry").file_name())
                .filter(|name| {
                    name != "source.bin" && name != suprnova::filesystem::ATOMIC_STAGING_DIR
                })
                .count();
            if siblings == 0 && staging_file_count(tmp.path()) == 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("cancelled promotion removes writer and sibling staging");
    assert_eq!(
        std::fs::read(tmp.path().join("source.bin")).expect("winner remains"),
        b"concurrent winner"
    );
    assert!(
        fallback
            .exists("source.bin")
            .await
            .expect("fallback remains")
    );
}

#[tokio::test]
async fn read_through_promotion_cleans_staging_on_cancel_during_close() {
    cancel_read_through_promotion(false).await;
}

#[tokio::test]
async fn read_through_promotion_cleans_staging_on_cancel_after_close() {
    cancel_read_through_promotion(true).await;
}

#[tokio::test]
async fn read_through_direct_promotion_aborts_writer_on_cancel() {
    let _guard = Storage::fake();
    Storage::register_memory("fallback");
    Storage::disk("fallback")
        .expect("fallback")
        .write("source.bin", "cold bytes")
        .await
        .expect("seed source");
    let reached = Arc::new(AtomicUsize::new(0));
    Storage::register_memory_with("primary", {
        let reached = reached.clone();
        move |op| {
            op.layer(GateWritesLayer {
                writes: Arc::new(AtomicUsize::new(0)),
                close_gate: Some((false, reached)),
            })
        }
    });
    Storage::register_read_through(
        "assets",
        ReadThroughConfig {
            primary: "primary".into(),
            fallback: "fallback".into(),
            ..Default::default()
        },
    )
    .expect("register read-through");
    let primary = Storage::disk("primary").expect("primary");
    assert!(
        !primary.info().capability().rename,
        "exercise direct promotion"
    );
    let assets = Storage::disk("assets").expect("read-through");
    let promotion = tokio::spawn(async move { assets.read("source.bin").await });
    tokio::time::timeout(Duration::from_secs(10), async {
        while reached.load(Ordering::SeqCst) != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("direct promotion reaches close");
    promotion.abort();
    assert!(
        promotion
            .await
            .expect_err("promotion is cancelled")
            .is_cancelled()
    );
    tokio::time::timeout(Duration::from_secs(2), async {
        while reached.load(Ordering::SeqCst) != 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("direct promotion must abort its backend writer");
    assert!(!primary.exists("source.bin").await.expect("primary answers"));
}

#[tokio::test]
async fn read_through_conditional_copy_preserves_concurrent_winner() {
    let _guard = Storage::fake();
    let tmp = tempfile::tempdir().expect("create tempdir");
    Storage::register_memory("fallback");
    Storage::disk("fallback")
        .expect("fallback")
        .write("source.bin", "cold bytes")
        .await
        .expect("seed source");
    let reached = Arc::new(AtomicUsize::new(0));
    Storage::register_fs_with("primary", tmp.path(), {
        let reached = reached.clone();
        move |op| {
            op.layer(GateWritesLayer {
                writes: Arc::new(AtomicUsize::new(0)),
                close_gate: Some((false, reached)),
            })
        }
    })
    .expect("register primary");
    Storage::register_read_through(
        "assets",
        ReadThroughConfig {
            primary: "primary".into(),
            fallback: "fallback".into(),
            ..Default::default()
        },
    )
    .expect("register read-through");
    let assets = Storage::disk("assets").expect("read-through");
    let transfer = tokio::spawn(async move {
        assets
            .copy_with("source.bin", "destination.bin")
            .if_not_exists(true)
            .await
    });
    tokio::time::timeout(Duration::from_secs(10), async {
        while reached.load(Ordering::SeqCst) != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("conditional copy reaches close");
    std::fs::write(tmp.path().join("destination.bin"), b"concurrent winner")
        .expect("publish winner");
    reached.store(3, Ordering::SeqCst);
    let error = transfer
        .await
        .expect("copy task finishes")
        .expect_err("conditional publish loses race");
    assert_eq!(error.kind(), ErrorKind::ConditionNotMatch);
    assert_eq!(staging_file_count(tmp.path()), 0);
    assert_eq!(
        std::fs::read(tmp.path().join("destination.bin")).expect("concurrent winner remains"),
        b"concurrent winner"
    );
}

#[tokio::test]
async fn copy_cleans_up_partial_destination_on_midstream_failure() {
    let _guard = Storage::fake();
    let tmp = tempfile::tempdir().expect("create tempdir");

    // Source: memory disk whose reader fails after one chunk has been read
    // (and therefore after that chunk has been written to the destination).
    Storage::register_memory_with("atomic_fail_src", |op| op.layer(FailAfterOneChunkLayer));
    // Destination: a real filesystem disk so the partial write is observable.
    Storage::register_fs("atomic_fs_dest", tmp.path()).expect("fs dest");

    let result = copy_between_disks(
        "atomic_fail_src",
        "anything",
        "atomic_fs_dest",
        "partial.bin",
    )
    .await;
    assert!(
        result.is_err(),
        "a mid-stream source failure must surface as an error"
    );

    // The one chunk that was written before the failure must have been cleaned
    // up - a failed copy must never be observable as a partial/truncated file.
    assert!(
        !tmp.path().join("partial.bin").exists(),
        "a failed copy must not leave a partial destination file"
    );
}

async fn copy_preserves_existing_destination(local: bool, fail: bool) {
    let _guard = Storage::fake();
    let root = tempfile::tempdir().expect("destination root");
    if local {
        Storage::register_fs("destination", root.path()).expect("local destination");
    } else {
        Storage::register_memory("destination");
    }
    if fail {
        Storage::register_memory_with("source", |op| op.layer(FailAfterOneChunkLayer));
    } else {
        Storage::register_memory("source");
    }
    Storage::disk("source")
        .expect("source")
        .write("source.bin", "replacement")
        .await
        .expect("seed source metadata");
    let destination = Storage::disk("destination").expect("destination");
    destination
        .write("destination.bin", "original")
        .await
        .expect("seed destination");
    let result = copy_between_disks("source", "source.bin", "destination", "destination.bin").await;
    if fail {
        let error = result.expect_err("source fails after destination writer opens");
        assert!(
            error.to_string().contains("injected direct read failure"),
            "{error}"
        );
    } else {
        assert_eq!(result.expect("copy succeeds"), 11);
    }
    assert_eq!(
        destination
            .read("destination.bin")
            .await
            .expect("destination survives")
            .to_vec(),
        if fail {
            b"original".as_slice()
        } else {
            b"replacement".as_slice()
        }
    );
    assert_eq!(staging_file_count(root.path()), 0);
}

#[tokio::test]
async fn failed_copy_preserves_existing_memory_destination() {
    copy_preserves_existing_destination(false, true).await;
}

#[tokio::test]
async fn failed_copy_preserves_existing_local_destination() {
    copy_preserves_existing_destination(true, true).await;
}

#[tokio::test]
async fn successful_copy_replaces_existing_memory_destination() {
    copy_preserves_existing_destination(false, false).await;
}

#[tokio::test]
async fn successful_copy_replaces_existing_local_destination() {
    copy_preserves_existing_destination(true, false).await;
}

#[tokio::test]
async fn read_through_ordinary_copy_preserves_concurrent_winner_on_failure() {
    let _guard = Storage::fake();
    let root = tempfile::tempdir().expect("primary root");
    Storage::register_memory("fallback");
    Storage::disk("fallback")
        .expect("fallback")
        .write("source.bin", "cold bytes")
        .await
        .expect("seed fallback");
    let reached = Arc::new(AtomicUsize::new(0));
    Storage::register_fs_with("primary", root.path(), {
        let reached = reached.clone();
        move |op| {
            op.layer(GateWritesLayer {
                writes: Arc::new(AtomicUsize::new(0)),
                close_gate: Some((false, reached)),
            })
        }
    })
    .expect("primary");
    Storage::register_read_through(
        "assets",
        ReadThroughConfig {
            primary: "primary".into(),
            fallback: "fallback".into(),
            ..Default::default()
        },
    )
    .expect("read-through");
    let assets = Storage::disk("assets").expect("assets");
    let transfer = tokio::spawn(async move { assets.copy("source.bin", "destination.bin").await });
    tokio::time::timeout(Duration::from_secs(10), async {
        while reached.load(Ordering::SeqCst) != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("copy reaches close after observing absent destination");
    std::fs::write(root.path().join("destination.bin"), b"concurrent winner")
        .expect("publish winner");
    reached.store(4, Ordering::SeqCst);
    let error = transfer
        .await
        .expect("copy finishes")
        .expect_err("close fails before publishing");
    assert!(
        error.to_string().contains("injected close failure"),
        "{error}"
    );
    assert_eq!(staging_file_count(root.path()), 0);
    assert_eq!(
        std::fs::read(root.path().join("destination.bin")).expect("winner survives"),
        b"concurrent winner"
    );
}

#[tokio::test]
async fn read_through_native_copy_cleans_staging_on_cancel() {
    let _guard = Storage::fake();
    let root = tempfile::tempdir().expect("primary root");
    std::fs::write(root.path().join("source.bin"), b"primary bytes").expect("seed native source");
    std::fs::write(root.path().join("destination.bin"), b"original").expect("seed destination");
    Storage::register_memory("fallback");
    let reached = Arc::new(AtomicUsize::new(0));
    Storage::register_fs_with("primary", root.path(), {
        let reached = reached.clone();
        move |op| {
            op.layer(GateWritesLayer {
                writes: Arc::new(AtomicUsize::new(0)),
                close_gate: Some((false, reached)),
            })
        }
    })
    .expect("primary");
    Storage::register_read_through(
        "assets",
        ReadThroughConfig {
            primary: "primary".into(),
            fallback: "fallback".into(),
            ..Default::default()
        },
    )
    .expect("read-through");
    let assets = Storage::disk("assets").expect("assets");
    let transfer = tokio::spawn(async move { assets.copy("source.bin", "destination.bin").await });
    tokio::time::timeout(Duration::from_secs(10), async {
        while reached.load(Ordering::SeqCst) != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("native copier stages bytes before publication");
    assert_ne!(
        staging_file_count(root.path()),
        0,
        "private copy stage exists"
    );
    transfer.abort();
    assert!(transfer.await.expect_err("copy cancelled").is_cancelled());
    tokio::time::timeout(Duration::from_secs(2), async {
        while staging_file_count(root.path()) != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("cancelled native copier removes its stage");
    assert_eq!(
        std::fs::read(root.path().join("destination.bin")).expect("destination"),
        b"original"
    );
}
