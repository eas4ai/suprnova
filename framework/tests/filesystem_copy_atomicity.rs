#![cfg(feature = "filesystem")]

//! `copy_between_disks` must not leave a partial destination object when the
//! transfer fails mid-stream.
//!
//! The source disk here is a memory backend wrapped in a layer whose reader
//! yields exactly one chunk and then errors — simulating a source that fails
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
use suprnova::Storage;
use suprnova::filesystem::streaming::copy_between_disks;

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
    // up — a failed copy must never be observable as a partial/truncated file.
    assert!(
        !tmp.path().join("partial.bin").exists(),
        "a failed copy must not leave a partial destination file"
    );
}
