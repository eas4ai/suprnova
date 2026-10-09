use suprnova::opendal::raw::{
    OpCopier, OpCopy, OpCreateDir, OpList, OpPresign, OpRename, OpStat, RpCreateDir, RpPresign,
    RpRename, RpStat,
};
use suprnova::opendal::{OperationContext, Result};
use suprnova::{
    Storage,
    opendal::{Operator, services},
};

#[tokio::test]
async fn copies_and_moves_accept_names_and_handles_on_both_sides() {
    let _fake = Storage::fake();
    Storage::register_memory("local");
    Storage::register_memory("archive");
    let source = Storage::disk("local").expect("source");
    let destination = Storage::disk("archive").expect("destination");
    source.write("a.txt", "hello").await.expect("write");
    assert_eq!(
        Storage::copy_to_disk("local", "a.txt", &destination, "copy.txt")
            .await
            .expect("copy"),
        5
    );
    assert_eq!(
        source.read("a.txt").await.expect("source stays").to_vec(),
        b"hello"
    );
    assert_eq!(
        destination
            .read("copy.txt")
            .await
            .expect("copy bytes")
            .to_vec(),
        b"hello"
    );
    Storage::move_to_disk("local", "a.txt", "archive", "a.txt")
        .await
        .expect("move");
    assert!(!source.exists("a.txt").await.expect("source deleted"));
    assert_eq!(
        destination
            .read("a.txt")
            .await
            .expect("destination bytes")
            .to_vec(),
        b"hello"
    );
    Storage::copy_to_disk(destination.clone(), "a.txt", "local", "back.txt")
        .await
        .expect("source handle");
    Storage::move_to_disk(&source, "back.txt", destination, "back.txt")
        .await
        .expect("both handles");
    assert!(!source.exists("back.txt").await.expect("moved"));
}

#[tokio::test]
async fn failed_copy_keeps_the_source_and_existing_destination() {
    let _fake = Storage::fake();
    let source = Storage::disk("default").expect("source");
    source.write("a.txt", "keep").await.expect("write");
    assert!(
        Storage::move_to_disk(&source, "a.txt", "missing", "a.txt")
            .await
            .is_err()
    );
    assert_eq!(source.read("a.txt").await.expect("kept").to_vec(), b"keep");
    assert!(
        Storage::copy_to_disk("missing", "a.txt", &source, "a.txt")
            .await
            .is_err()
    );
    source
        .write("dest.txt", "previous")
        .await
        .expect("destination");
    assert!(
        Storage::copy_to_disk(&source, "missing.txt", &source, "dest.txt")
            .await
            .is_err()
    );
    assert_eq!(
        source
            .read("dest.txt")
            .await
            .expect("previous stays")
            .to_vec(),
        b"previous"
    );
}

#[tokio::test]
async fn empty_files_are_copied_and_then_deleted_when_moved() {
    let _fake = Storage::fake();
    Storage::register_memory("archive");
    let source = Storage::disk("default").expect("source");
    source.write("empty", "").await.expect("write empty");
    assert_eq!(
        Storage::move_to_disk(&source, "empty", "archive", "empty")
            .await
            .expect("move empty"),
        0
    );
    assert!(!source.exists("empty").await.expect("deleted"));
    assert_eq!(
        Storage::disk("archive")
            .expect("archive")
            .read("empty")
            .await
            .expect("exists")
            .len(),
        0
    );
}

#[test]
fn forget_accepts_one_name_and_lists_and_purge_preserves_other_disks() {
    let _fake = Storage::fake();
    for name in ["a", "b", "c"] {
        Storage::register_memory(name);
    }
    assert!(Storage::forget(["a", "b"]));
    assert!(Storage::disk("a").is_err());
    assert!(Storage::disk("b").is_err());
    assert!(Storage::disk("c").is_ok());
    assert!(!Storage::forget(Vec::<String>::new()));
    assert!(!Storage::forget("missing"));
    Storage::register_memory("one");
    assert!(Storage::forget("one"));
    assert!(Storage::disk("one").is_err());
    Storage::register_memory("slice");
    let slice = vec!["slice".to_owned(), "missing".to_owned()];
    assert!(Storage::forget(slice.as_slice()));
    Storage::register_memory("array");
    let array = ["array"];
    let borrowed: &[&str; 1] = &array;
    assert!(Storage::forget(borrowed));
    Storage::register_memory("vector");
    assert!(Storage::forget(vec!["vector"]));
    Storage::purge("c");
    assert!(Storage::disk("c").is_err());
    assert!(Storage::disk("default").is_ok());
    Storage::purge_all();
    assert!(Storage::disks().is_empty());
    Storage::register_memory("legacy");
    Storage::purge_all();
    assert!(Storage::disks().is_empty());
}

#[tokio::test]
async fn set_replaces_the_disk_and_returns_the_same_backend() {
    let _fake = Storage::fake();
    let old = Storage::disk("default").expect("old");
    old.write("old", "old").await.expect("old bytes");
    Storage::set_public_url("default", "/old").expect("old URL");
    let ready = Operator::new(services::Memory::default()).expect("memory disk");
    Storage::set("default", ready.clone());
    let fetched = Storage::disk("default").expect("replacement");
    assert!(!fetched.exists("old").await.expect("old disk replaced"));
    ready.write("shared", "new").await.expect("ready write");
    assert_eq!(
        fetched.read("shared").await.expect("same backend").to_vec(),
        b"new"
    );
    fetched
        .write("reverse", "yes")
        .await
        .expect("fetched write");
    assert!(ready.exists("reverse").await.expect("same ready disk"));
    assert!(Storage::url("default", "shared").is_err());
    assert!(old.exists("old").await.expect("old handle stays valid"));
}

#[tokio::test]
async fn moving_to_the_same_file_fails_without_deleting_it() {
    let _fake = Storage::fake();
    let disk = Storage::disk("default").expect("disk");
    disk.write("keep", "original").await.expect("write");
    assert!(
        Storage::move_to_disk("default", "keep", &disk, "keep")
            .await
            .is_err()
    );
    assert!(
        Storage::copy_to_disk(&disk, "keep", disk.clone(), "keep")
            .await
            .is_err()
    );
    assert!(
        Storage::move_to_disk(&disk, "keep", disk.clone(), "/keep")
            .await
            .is_err()
    );
    assert_eq!(
        disk.read("keep").await.expect("original survives").to_vec(),
        b"original"
    );
}

#[derive(Debug)]
struct RefuseMutation {
    inner: suprnova::opendal::raw::Servicer,
    refuse_write: bool,
}
impl suprnova::opendal::raw::Service for RefuseMutation {
    type Reader = suprnova::opendal::raw::oio::Reader;
    type Writer = suprnova::opendal::raw::oio::Writer;
    type Lister = suprnova::opendal::raw::oio::Lister;
    type Deleter = suprnova::opendal::raw::oio::Deleter;
    type Copier = suprnova::opendal::raw::oio::Copier;

    fn info(&self) -> suprnova::opendal::raw::ServiceInfo {
        self.inner.info()
    }
    fn capability(&self) -> suprnova::opendal::Capability {
        self.inner.capability()
    }
    fn read(
        &self,
        ctx: &suprnova::opendal::OperationContext,
        path: &str,
        args: suprnova::opendal::raw::OpRead,
    ) -> suprnova::opendal::Result<Self::Reader> {
        self.inner.read(ctx, path, args)
    }
    fn write(
        &self,
        ctx: &suprnova::opendal::OperationContext,
        path: &str,
        args: suprnova::opendal::raw::OpWrite,
    ) -> suprnova::opendal::Result<Self::Writer> {
        if self.refuse_write {
            return Err(suprnova::opendal::Error::new(
                suprnova::opendal::ErrorKind::PermissionDenied,
                "destination is read-only",
            ));
        }
        self.inner.write(ctx, path, args)
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

    fn delete(
        &self,
        _: &suprnova::opendal::OperationContext,
    ) -> suprnova::opendal::Result<Self::Deleter> {
        Err(suprnova::opendal::Error::new(
            suprnova::opendal::ErrorKind::PermissionDenied,
            "source deletion refused",
        ))
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

#[derive(Debug, Clone)]
struct RefuseMutationLayer {
    refuse_write: bool,
}
impl suprnova::opendal::raw::Layer for RefuseMutationLayer {
    fn apply_service(
        &self,
        inner: suprnova::opendal::raw::Servicer,
    ) -> suprnova::opendal::raw::Servicer {
        std::sync::Arc::new(RefuseMutation {
            inner,
            refuse_write: self.refuse_write,
        })
    }
}

#[tokio::test]
async fn write_failure_keeps_source_and_delete_failure_keeps_both_copies() {
    let source = Operator::new(services::Memory::default()).expect("source");
    let destination = Operator::new(services::Memory::default()).expect("destination");
    source
        .write("source", "payload")
        .await
        .expect("seed source");
    let readonly = destination
        .clone()
        .layer(RefuseMutationLayer { refuse_write: true });
    assert!(
        Storage::move_to_disk(&source, "source", readonly, "destination")
            .await
            .is_err()
    );
    assert_eq!(
        source
            .read("source")
            .await
            .expect("source after write failure")
            .to_vec(),
        b"payload"
    );
    let undeletable = source.clone().layer(RefuseMutationLayer {
        refuse_write: false,
    });
    let error = Storage::move_to_disk(undeletable, "source", &destination, "destination")
        .await
        .expect_err("delete failed");
    assert!(error.to_string().contains("delete moved source"));
    assert_eq!(
        source
            .read("source")
            .await
            .expect("source after delete failure")
            .to_vec(),
        b"payload"
    );
    assert_eq!(
        destination
            .read("destination")
            .await
            .expect("destination copy committed")
            .to_vec(),
        b"payload"
    );
}
