//! Quarantined streaming provider and Tokio file-store contract tests.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};
use suprnova_live::identity::UnixMillis;
use suprnova_live::limits::{UploadLimitConfig, UploadLimits};
use suprnova_live::upload::{
    ChunkBody, ChunkDisposition, PrepareTransfer, QuarantineBytes, QuarantineObject,
    QuarantineStore, QuarantinedFileProvider, RemoveDisposition, ReverseProxyUploadProvider,
    TransferCheckpoint, TransferDisposition, UploadChecksum, UploadError, UploadErrorKind,
    UploadFuture, UploadHandle, UploadProvider, VerifyTransfer, WriteChunk,
};
use suprnova_live_test_support::{FileStoreFault, TokioFileQuarantineStore};
use tokio::sync::Notify;

const HANDLE: &str = "018f47c1-2af0-7cc4-a001-000000000001";
const OTHER_HANDLE: &str = "018f8f3a-7b2c-4d5e-8f90-abcdef012345";
static NEXT_TEMP_ROOT: AtomicU64 = AtomicU64::new(1);

struct TempRoot(PathBuf);

impl TempRoot {
    fn new() -> Self {
        let sequence = NEXT_TEMP_ROOT.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "suprnova-live-upload-{}-{sequence}",
            std::process::id()
        ));
        std::fs::create_dir(&path).expect("create isolated temporary root");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let safe_name = self
            .0
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("suprnova-live-upload-"));
        if safe_name && self.0.parent() == Some(std::env::temp_dir().as_path()) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

struct TestBody {
    parts: VecDeque<Result<QuarantineBytes, UploadError>>,
    calls: usize,
}

impl TestBody {
    fn bytes(parts: &[&[u8]]) -> Self {
        Self {
            parts: parts
                .iter()
                .map(|part| Ok(QuarantineBytes::copy_from_slice(part)))
                .collect(),
            calls: 0,
        }
    }

    fn interrupted(prefix: &[u8]) -> Self {
        Self {
            parts: VecDeque::from([
                Ok(QuarantineBytes::copy_from_slice(prefix)),
                Err(UploadError::new(UploadErrorKind::BodyInterrupted)),
            ]),
            calls: 0,
        }
    }
}

impl ChunkBody for TestBody {
    fn next_chunk<'a>(
        &'a mut self,
        _maximum_bytes: usize,
    ) -> suprnova_live::upload::UploadFuture<'a, Result<Option<QuarantineBytes>, UploadError>> {
        Box::pin(async move {
            self.calls += 1;
            self.parts.pop_front().transpose()
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StorePausePoint {
    Create,
    Remove,
    Sync,
    Write,
}

struct ControlledStore {
    inner: Arc<TokioFileQuarantineStore>,
    pause: Mutex<Option<StorePausePoint>>,
    entered: Notify,
    release: Notify,
}

impl ControlledStore {
    fn new(inner: Arc<TokioFileQuarantineStore>) -> Self {
        Self {
            inner,
            pause: Mutex::new(None),
            entered: Notify::new(),
            release: Notify::new(),
        }
    }

    fn pause_once(&self, point: StorePausePoint) {
        *self.pause.lock().expect("controlled-store pause lock") = Some(point);
    }

    async fn wait_until_paused(&self) {
        self.entered.notified().await;
    }

    async fn pause_after_success(&self, point: StorePausePoint) {
        let selected = {
            let mut selected = self.pause.lock().expect("controlled-store pause lock");
            if *selected == Some(point) {
                selected.take();
                true
            } else {
                false
            }
        };
        if selected {
            self.entered.notify_one();
            self.release.notified().await;
        }
    }
}

impl QuarantineStore for ControlledStore {
    fn create_exclusive<'a>(
        &'a self,
        object: &'a QuarantineObject,
    ) -> UploadFuture<'a, Result<(), UploadError>> {
        Box::pin(async move {
            self.inner.create_exclusive(object).await?;
            self.pause_after_success(StorePausePoint::Create).await;
            Ok(())
        })
    }

    fn write_at<'a>(
        &'a self,
        object: &'a QuarantineObject,
        offset: u64,
        bytes: &'a [u8],
    ) -> UploadFuture<'a, Result<(), UploadError>> {
        Box::pin(async move {
            self.inner.write_at(object, offset, bytes).await?;
            self.pause_after_success(StorePausePoint::Write).await;
            Ok(())
        })
    }

    fn sync<'a>(
        &'a self,
        object: &'a QuarantineObject,
    ) -> UploadFuture<'a, Result<(), UploadError>> {
        Box::pin(async move {
            self.inner.sync(object).await?;
            self.pause_after_success(StorePausePoint::Sync).await;
            Ok(())
        })
    }

    fn read_at<'a>(
        &'a self,
        object: &'a QuarantineObject,
        offset: u64,
        maximum_bytes: usize,
    ) -> UploadFuture<'a, Result<QuarantineBytes, UploadError>> {
        self.inner.read_at(object, offset, maximum_bytes)
    }

    fn remove<'a>(
        &'a self,
        object: &'a QuarantineObject,
    ) -> UploadFuture<'a, Result<RemoveDisposition, UploadError>> {
        Box::pin(async move {
            let disposition = self.inner.remove(object).await?;
            self.pause_after_success(StorePausePoint::Remove).await;
            Ok(disposition)
        })
    }
}

struct RetiringBody {
    provider: Arc<QuarantinedFileProvider<TokioFileQuarantineStore>>,
    bytes: Option<QuarantineBytes>,
}

impl ChunkBody for RetiringBody {
    fn next_chunk<'a>(
        &'a mut self,
        _maximum_bytes: usize,
    ) -> suprnova_live::upload::UploadFuture<'a, Result<Option<QuarantineBytes>, UploadError>> {
        Box::pin(async move {
            if self.bytes.is_some() {
                self.provider.retire();
            }
            Ok(self.bytes.take())
        })
    }
}

fn handle(value: &str) -> UploadHandle {
    UploadHandle::parse(value).expect("fixture handle")
}

fn checksum(bytes: &[u8]) -> UploadChecksum {
    let mut encoded = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        write!(&mut encoded, "{byte:02x}").expect("format checksum");
    }
    UploadChecksum::parse(&encoded).expect("sha-256 checksum")
}

fn limits() -> UploadLimits {
    UploadLimits::new(UploadLimitConfig::reference()).expect("reference limits")
}

async fn store(root: &TempRoot) -> Arc<TokioFileQuarantineStore> {
    Arc::new(
        TokioFileQuarantineStore::open(root.path(), 8, 1024 * 1024)
            .await
            .expect("file quarantine store"),
    )
}

async fn prepared(
    root: &TempRoot,
    expected_bytes: u64,
) -> (
    Arc<TokioFileQuarantineStore>,
    QuarantinedFileProvider<TokioFileQuarantineStore>,
) {
    let store = store(root).await;
    let provider = QuarantinedFileProvider::new(store.clone(), limits()).expect("provider");
    provider
        .prepare(PrepareTransfer::new(
            &handle(HANDLE),
            expected_bytes,
            "fixture.bin",
            UnixMillis::new(1_000),
        ))
        .await
        .expect("prepare transfer");
    (store, provider)
}

async fn controlled_provider(
    root: &TempRoot,
) -> (
    Arc<ControlledStore>,
    Arc<QuarantinedFileProvider<ControlledStore>>,
) {
    let inner = store(root).await;
    let store = Arc::new(ControlledStore::new(inner));
    let provider = Arc::new(
        QuarantinedFileProvider::new(store.clone(), limits()).expect("controlled provider"),
    );
    (store, provider)
}

async fn root_file_count(root: &TempRoot) -> usize {
    let mut entries = tokio::fs::read_dir(root.path())
        .await
        .expect("read quarantine root");
    let mut count = 0;
    while entries
        .next_entry()
        .await
        .expect("read quarantine entry")
        .is_some()
    {
        count += 1;
    }
    count
}

#[tokio::test]
async fn client_names_never_influence_quarantine_paths() {
    let root = TempRoot::new();
    let store = store(&root).await;
    let provider = QuarantinedFileProvider::new(store.clone(), limits()).expect("provider");
    let upload = handle(HANDLE);

    let plan = provider
        .prepare(PrepareTransfer::new(
            &upload,
            4,
            "../../served.html",
            UnixMillis::new(1_000),
        ))
        .await
        .expect("prepare transfer");
    let checkpoint = provider.checkpoint(&upload).expect("checkpoint");
    let replay = provider
        .prepare(PrepareTransfer::new(
            &upload,
            4,
            "another-client-name.txt",
            UnixMillis::new(1_001),
        ))
        .await
        .expect("exact preparation replay");
    let stored_path = store.path_for_test(checkpoint.object());

    assert_eq!(plan.disposition(), TransferDisposition::Prepared);
    assert_eq!(replay.disposition(), TransferDisposition::ExistingOutcome);
    assert!(stored_path.starts_with(root.path()));
    assert!(!stored_path.to_string_lossy().contains("served.html"));
    assert_eq!(stored_path.parent(), Some(root.path()));
    let diagnostics = format!("{provider:?} {checkpoint:?} {plan:?}");
    assert!(!diagnostics.contains("served.html"));
    assert!(!diagnostics.contains(checkpoint.object().storage_key()));
}

#[test]
fn quarantine_object_keys_are_fixed_canonical_path_segments() {
    let object = QuarantineObject::generate().expect("generated quarantine object");
    assert_eq!(object.storage_key().len(), 64);
    assert!(
        object
            .storage_key()
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    );
    for invalid in [
        "",
        "../object",
        "/absolute",
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
    ] {
        assert!(QuarantineObject::parse_storage_key(invalid).is_err());
    }
}

#[tokio::test]
async fn chunks_stream_with_exact_duplicate_replay_and_whole_integrity() {
    let root = TempRoot::new();
    let (_store, provider) = prepared(&root, 11).await;
    let upload = handle(HANDLE);
    let chunk_checksum = checksum(b"hello world");
    let request = WriteChunk::new(&upload, 0, 0, 11, &chunk_checksum);
    let mut body = TestBody::bytes(&[b"hello ", b"world"]);

    let first = provider
        .write_chunk(request, &mut body)
        .await
        .expect("write chunk");
    let mut duplicate_body = TestBody::bytes(&[]);
    let duplicate = provider
        .write_chunk(
            WriteChunk::new(&upload, 0, 0, 11, &chunk_checksum),
            &mut duplicate_body,
        )
        .await
        .expect("duplicate chunk");
    let evidence = provider
        .verify(VerifyTransfer::new(&upload, &checksum(b"hello world")))
        .await
        .expect("whole-file integrity");

    assert_eq!(first.disposition(), ChunkDisposition::Stored);
    assert_eq!(duplicate.disposition(), ChunkDisposition::ExistingOutcome);
    assert_eq!(duplicate_body.calls, 0);
    assert_eq!(evidence.bytes(), 11);
    assert_eq!(evidence.checksum(), &checksum(b"hello world"));
}

#[tokio::test]
async fn interrupted_and_checksum_failed_chunks_are_retryable_without_accepting_state() {
    let root = TempRoot::new();
    let (_store, provider) = prepared(&root, 8).await;
    let upload = handle(HANDLE);
    let expected = checksum(b"complete");

    let mut interrupted = TestBody::interrupted(b"comp");
    let error = provider
        .write_chunk(
            WriteChunk::new(&upload, 0, 0, 8, &expected),
            &mut interrupted,
        )
        .await
        .expect_err("interrupted body");
    assert_eq!(error.kind(), UploadErrorKind::BodyInterrupted);

    let mut corrupt = TestBody::bytes(&[b"corrupt!"]);
    let error = provider
        .write_chunk(WriteChunk::new(&upload, 0, 0, 8, &expected), &mut corrupt)
        .await
        .expect_err("chunk checksum mismatch");
    assert_eq!(error.kind(), UploadErrorKind::ChecksumMismatch);

    let mut retry = TestBody::bytes(&[b"complete"]);
    provider
        .write_chunk(WriteChunk::new(&upload, 0, 0, 8, &expected), &mut retry)
        .await
        .expect("retry stores complete chunk");
    provider
        .verify(VerifyTransfer::new(&upload, &expected))
        .await
        .expect("retry verifies");
}

#[tokio::test]
async fn chunk_streaming_rejects_overrun_before_accepting_retry_state() {
    let root = TempRoot::new();
    let (_store, provider) = prepared(&root, 4).await;
    let upload = handle(HANDLE);
    let expected = checksum(b"safe");
    let mut overrun = TestBody::bytes(&[b"safe", b"x"]);

    let error = provider
        .write_chunk(WriteChunk::new(&upload, 0, 0, 4, &expected), &mut overrun)
        .await
        .expect_err("body overrun");
    assert_eq!(error.kind(), UploadErrorKind::InputTooLarge);

    let mut retry = TestBody::bytes(&[b"safe"]);
    provider
        .write_chunk(WriteChunk::new(&upload, 0, 0, 4, &expected), &mut retry)
        .await
        .expect("bounded retry");
}

#[tokio::test]
async fn short_store_reads_and_writes_are_completed_without_whole_file_buffering() {
    let root = TempRoot::new();
    let (store, provider) = prepared(&root, 12).await;
    store.set_write_fragment_limit(Some(2));
    store.set_read_fragment_limit(Some(3));
    let upload = handle(HANDLE);
    let expected = checksum(b"fragmented!!");
    let mut body = TestBody::bytes(&[b"frag", b"ment", b"ed!!"]);

    provider
        .write_chunk(WriteChunk::new(&upload, 0, 0, 12, &expected), &mut body)
        .await
        .expect("fragmented write");
    let evidence = provider
        .verify(VerifyTransfer::new(&upload, &expected))
        .await
        .expect("fragmented read");

    assert_eq!(evidence.bytes(), 12);
    assert!(store.maximum_observed_write() <= 2);
    assert!(store.maximum_observed_read() <= 3);
}

#[tokio::test]
async fn provider_failures_do_not_commit_chunks_and_retries_remain_available() {
    let root = TempRoot::new();
    let (store, provider) = prepared(&root, 4).await;
    let upload = handle(HANDLE);
    let expected = checksum(b"safe");
    store.set_fault(FileStoreFault::Write);
    let mut body = TestBody::bytes(&[b"safe"]);

    let error = provider
        .write_chunk(WriteChunk::new(&upload, 0, 0, 4, &expected), &mut body)
        .await
        .expect_err("controlled disk failure");
    assert_eq!(error.kind(), UploadErrorKind::ProviderUnavailable);

    store.set_fault(FileStoreFault::None);
    let mut retry = TestBody::bytes(&[b"safe"]);
    provider
        .write_chunk(WriteChunk::new(&upload, 0, 0, 4, &expected), &mut retry)
        .await
        .expect("retry after provider recovery");
    store.set_fault(FileStoreFault::Sync);
    let error = provider
        .verify(VerifyTransfer::new(&upload, &expected))
        .await
        .expect_err("sync failure blocks readiness");
    assert_eq!(error.kind(), UploadErrorKind::ProviderUnavailable);
    store.set_fault(FileStoreFault::None);
    provider
        .verify(VerifyTransfer::new(&upload, &expected))
        .await
        .expect("verification retry after sync recovery");
}

#[tokio::test]
async fn canceled_store_write_clears_pending_and_exact_retry_commits_once() {
    let root = TempRoot::new();
    let (store, provider) = controlled_provider(&root).await;
    let upload = handle(HANDLE);
    provider
        .prepare(PrepareTransfer::new(
            &upload,
            4,
            "canceled-write.bin",
            UnixMillis::new(1_000),
        ))
        .await
        .expect("prepare controlled transfer");
    store.pause_once(StorePausePoint::Write);
    let task = tokio::spawn({
        let provider = provider.clone();
        let upload = upload.clone();
        async move {
            let expected = checksum(b"safe");
            let mut body = TestBody::bytes(&[b"safe"]);
            provider
                .write_chunk(WriteChunk::new(&upload, 0, 0, 4, &expected), &mut body)
                .await
        }
    });
    store.wait_until_paused().await;
    task.abort();
    assert!(task.await.expect_err("write task aborted").is_cancelled());

    let expected = checksum(b"safe");
    let mut retry = TestBody::bytes(&[b"safe"]);
    let receipt = provider
        .write_chunk(WriteChunk::new(&upload, 0, 0, 4, &expected), &mut retry)
        .await
        .expect("exact retry after canceled store write");
    assert_eq!(receipt.disposition(), ChunkDisposition::Stored);
    assert_eq!(
        provider
            .checkpoint(&upload)
            .expect("committed checkpoint")
            .committed_bytes(),
        4
    );
    provider
        .verify(VerifyTransfer::new(&upload, &expected))
        .await
        .expect("retry bytes verify");
    let mut duplicate_body = TestBody::bytes(&[]);
    let duplicate = provider
        .write_chunk(
            WriteChunk::new(&upload, 0, 0, 4, &expected),
            &mut duplicate_body,
        )
        .await
        .expect("accepted retry is idempotent");
    assert_eq!(duplicate.disposition(), ChunkDisposition::ExistingOutcome);
    assert_eq!(duplicate_body.calls, 0);
    provider
        .cancel(&upload)
        .await
        .expect("cleanup retried object");
    assert_eq!(root_file_count(&root).await, 0);
}

#[tokio::test]
async fn canceled_store_create_is_reclaimed_before_exact_prepare_retry() {
    let root = TempRoot::new();
    let (store, provider) = controlled_provider(&root).await;
    let upload = handle(HANDLE);
    store.pause_once(StorePausePoint::Create);
    let task = tokio::spawn({
        let provider = provider.clone();
        let upload = upload.clone();
        async move {
            provider
                .prepare(PrepareTransfer::new(
                    &upload,
                    4,
                    "canceled-create.bin",
                    UnixMillis::new(1_000),
                ))
                .await
        }
    });
    store.wait_until_paused().await;
    assert_eq!(root_file_count(&root).await, 1);
    task.abort();
    assert!(task.await.expect_err("create task aborted").is_cancelled());

    let plan = provider
        .prepare(PrepareTransfer::new(
            &upload,
            4,
            "retry-create.bin",
            UnixMillis::new(1_001),
        ))
        .await
        .expect("exact prepare retry reclaims canceled reservation");
    assert_eq!(plan.disposition(), TransferDisposition::Prepared);
    assert_eq!(root_file_count(&root).await, 1);
    provider
        .cancel(&upload)
        .await
        .expect("cleanup retry object");
    assert_eq!(root_file_count(&root).await, 0);
}

#[tokio::test]
async fn provider_retirement_reclaims_an_aborted_unpublished_creation() {
    let root = TempRoot::new();
    let (store, provider) = controlled_provider(&root).await;
    let upload = handle(HANDLE);
    store.pause_once(StorePausePoint::Create);
    let task = tokio::spawn({
        let provider = provider.clone();
        let upload = upload.clone();
        async move {
            provider
                .prepare(PrepareTransfer::new(
                    &upload,
                    4,
                    "retired-create.bin",
                    UnixMillis::new(1_000),
                ))
                .await
        }
    });
    store.wait_until_paused().await;
    assert_eq!(root_file_count(&root).await, 1);
    task.abort();
    assert!(task.await.expect_err("create task aborted").is_cancelled());

    let retirement = provider
        .retire_and_cleanup()
        .await
        .expect("retirement reclaims unpublished creation");
    assert!(retirement.canceled);
    assert_eq!(root_file_count(&root).await, 0);
    assert_eq!(provider.descriptor_permits().active(), 0);
    assert_eq!(provider.chunk_permits().active(), 0);
    let error = provider
        .prepare(PrepareTransfer::new(
            &upload,
            4,
            "cannot-revive.bin",
            UnixMillis::new(1_001),
        ))
        .await
        .expect_err("retired provider cannot revive creation");
    assert_eq!(error.kind(), UploadErrorKind::ServiceRetired);
    tokio::task::yield_now().await;
    assert_eq!(root_file_count(&root).await, 0);
}

#[tokio::test]
async fn canceled_verification_and_removal_awaits_remain_exactly_retryable() {
    let root = TempRoot::new();
    let (store, provider) = controlled_provider(&root).await;
    let upload = handle(HANDLE);
    let expected = checksum(b"safe");
    provider
        .prepare(PrepareTransfer::new(
            &upload,
            4,
            "verify-cancel.bin",
            UnixMillis::new(1_000),
        ))
        .await
        .expect("prepare controlled transfer");
    let mut body = TestBody::bytes(&[b"safe"]);
    provider
        .write_chunk(WriteChunk::new(&upload, 0, 0, 4, &expected), &mut body)
        .await
        .expect("write complete transfer");

    store.pause_once(StorePausePoint::Sync);
    let verify = tokio::spawn({
        let provider = provider.clone();
        let upload = upload.clone();
        let expected = expected.clone();
        async move {
            provider
                .verify(VerifyTransfer::new(&upload, &expected))
                .await
        }
    });
    store.wait_until_paused().await;
    verify.abort();
    assert!(
        verify
            .await
            .expect_err("verification task aborted")
            .is_cancelled()
    );
    provider
        .verify(VerifyTransfer::new(&upload, &expected))
        .await
        .expect("exact verification retry");

    store.pause_once(StorePausePoint::Remove);
    let cancel = tokio::spawn({
        let provider = provider.clone();
        let upload = upload.clone();
        async move { provider.cancel(&upload).await }
    });
    store.wait_until_paused().await;
    assert_eq!(root_file_count(&root).await, 0);
    cancel.abort();
    assert!(
        cancel
            .await
            .expect_err("remove task aborted")
            .is_cancelled()
    );
    provider
        .cleanup(&upload)
        .await
        .expect("idempotent removal retry clears retained state");
    provider
        .cleanup(&upload)
        .await
        .expect("duplicate removal remains idempotent");
    assert_eq!(root_file_count(&root).await, 0);
}

#[tokio::test]
async fn checkpoints_restore_partial_transfer_without_reusing_client_paths() {
    let root = TempRoot::new();
    let store = store(&root).await;
    let first = QuarantinedFileProvider::new(store.clone(), limits()).expect("first provider");
    let upload = handle(HANDLE);
    first
        .prepare(PrepareTransfer::new(
            &upload,
            10,
            "../ignored.txt",
            UnixMillis::new(1_000),
        ))
        .await
        .expect("prepare");
    let first_checksum = checksum(b"hello");
    let mut first_body = TestBody::bytes(&[b"hello"]);
    first
        .write_chunk(
            WriteChunk::new(&upload, 0, 0, 5, &first_checksum),
            &mut first_body,
        )
        .await
        .expect("first chunk");
    let checkpoint = first.checkpoint(&upload).expect("checkpoint");
    let persisted = TransferCheckpoint::new(
        checkpoint.handle().clone(),
        QuarantineObject::parse_storage_key(checkpoint.object().storage_key())
            .expect("persisted object key"),
        checkpoint.expected_bytes(),
        checkpoint.created_at(),
        checkpoint.chunks().collect(),
        checkpoint.committed_bytes(),
    )
    .expect("persisted checkpoint round trip");
    drop(first);

    let recovered = QuarantinedFileProvider::new(store, limits()).expect("recovered provider");
    recovered.recover(persisted).expect("recover checkpoint");
    let second_checksum = checksum(b"world");
    let mut second_body = TestBody::bytes(&[b"world"]);
    recovered
        .write_chunk(
            WriteChunk::new(&upload, 1, 5, 5, &second_checksum),
            &mut second_body,
        )
        .await
        .expect("second chunk");
    recovered
        .verify(VerifyTransfer::new(&upload, &checksum(b"helloworld")))
        .await
        .expect("recovered whole integrity");
}

#[tokio::test]
async fn cancellation_cleanup_and_shutdown_are_idempotent_and_fail_closed() {
    let root = TempRoot::new();
    let (store, provider) = prepared(&root, 4).await;
    let upload = handle(HANDLE);

    store.set_fault(FileStoreFault::Remove);
    let error = provider
        .cancel(&upload)
        .await
        .expect_err("controlled cleanup failure");
    assert_eq!(error.kind(), UploadErrorKind::ProviderUnavailable);
    store.set_fault(FileStoreFault::None);
    provider
        .cleanup(&upload)
        .await
        .expect("cleanup retry after provider recovery");
    provider
        .cancel(&upload)
        .await
        .expect("duplicate cancellation");
    provider.cleanup(&upload).await.expect("duplicate cleanup");

    let other = handle(OTHER_HANDLE);
    let retirement = provider.retire();
    assert!(retirement.canceled);
    let error = provider
        .prepare(PrepareTransfer::new(
            &other,
            4,
            "other.bin",
            UnixMillis::new(1_001),
        ))
        .await
        .expect_err("retired provider");
    assert_eq!(error.kind(), UploadErrorKind::ServiceRetired);
}

#[tokio::test]
async fn descriptor_and_chunk_concurrency_are_hard_bounded() {
    let root = TempRoot::new();
    let store = Arc::new(
        TokioFileQuarantineStore::open(root.path(), 1, 1024 * 1024)
            .await
            .expect("bounded store"),
    );
    let mut config = UploadLimitConfig::reference();
    config.max_concurrent_transfers = 1;
    let bounded = UploadLimits::new(config).expect("bounded limits");
    let provider = QuarantinedFileProvider::new(store, bounded).expect("provider");

    assert_eq!(provider.descriptor_permits().max_active(), 1);
    assert_eq!(provider.chunk_permits().max_active(), 1);
    let descriptor = provider
        .descriptor_permits()
        .try_acquire()
        .expect("descriptor permit");
    assert!(provider.descriptor_permits().try_acquire().is_err());
    drop(descriptor);
    let chunk = provider
        .chunk_permits()
        .try_acquire()
        .expect("chunk permit");
    assert!(provider.chunk_permits().try_acquire().is_err());
    drop(chunk);
}

#[tokio::test]
async fn retained_chunk_metadata_has_a_hard_per_file_bound() {
    let root = TempRoot::new();
    let store = store(&root).await;
    let mut config = UploadLimitConfig::reference();
    config.max_chunks_per_file = 1;
    let bounded = UploadLimits::new(config).expect("bounded limits");
    let provider = QuarantinedFileProvider::new(store, bounded).expect("provider");
    let upload = handle(HANDLE);
    provider
        .prepare(PrepareTransfer::new(
            &upload,
            2,
            "bounded.bin",
            UnixMillis::new(1_000),
        ))
        .await
        .expect("prepare");
    let first_checksum = checksum(b"a");
    let mut first = TestBody::bytes(&[b"a"]);
    provider
        .write_chunk(
            WriteChunk::new(&upload, 0, 0, 1, &first_checksum),
            &mut first,
        )
        .await
        .expect("first chunk");
    let second_checksum = checksum(b"b");
    let mut second = TestBody::bytes(&[b"b"]);
    let error = provider
        .write_chunk(
            WriteChunk::new(&upload, 1, 1, 1, &second_checksum),
            &mut second,
        )
        .await
        .expect_err("chunk metadata bound");

    assert_eq!(error.kind(), UploadErrorKind::ResourceExhausted);
    assert_eq!(second.calls, 0);
}

#[tokio::test]
async fn retirement_during_streaming_cancels_without_committing_partial_work() {
    let root = TempRoot::new();
    let store = store(&root).await;
    let provider = Arc::new(QuarantinedFileProvider::new(store, limits()).expect("provider"));
    let upload = handle(HANDLE);
    provider
        .prepare(PrepareTransfer::new(
            &upload,
            4,
            "shutdown.bin",
            UnixMillis::new(1_000),
        ))
        .await
        .expect("prepare");
    let expected = checksum(b"stop");
    let mut body = RetiringBody {
        provider: provider.clone(),
        bytes: Some(QuarantineBytes::from_static(b"stop")),
    };

    let error = provider
        .write_chunk(WriteChunk::new(&upload, 0, 0, 4, &expected), &mut body)
        .await
        .expect_err("retirement cancels stream");

    assert_eq!(error.kind(), UploadErrorKind::TransferCanceled);
    assert_eq!(
        provider
            .checkpoint(&upload)
            .expect("quiescent checkpoint")
            .committed_bytes(),
        0
    );
}
