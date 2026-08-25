//! Shared provider conformance and constrained direct-transfer capability tests.

use std::{
    collections::HashMap,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use bytes::Bytes;
use sha2::{Digest, Sha256};
use suprnova_live::{
    identity::UnixMillis,
    limits::{UploadLimitConfig, UploadLimits},
    upload::{
        BoundedHeaders, ChunkBody, ChunkDisposition, DirectPartReference,
        DirectTransferInstruction, DirectUploadProvider, PrepareTransfer, QuarantinedFileProvider,
        ReportDirectPart, ReverseProxyUploadProvider, TransferDisposition, TransferInstruction,
        TransferMethod, TrustedProviderOrigin, TrustedProviderUrl, UploadChecksum, UploadError,
        UploadErrorKind, UploadHandle, UploadPart, UploadProvider, VerifyTransfer, WriteChunk,
    },
};
use suprnova_live_test_support::{DirectProviderConformanceAdapter, TokioFileQuarantineStore};

const PRIMARY: &str = "550e8400-e29b-41d4-a716-446655440000";
const FOREIGN: &str = "01890f3a-7b2c-7def-8123-456789abcdef";

fn handle(value: &str) -> UploadHandle {
    UploadHandle::parse(value).expect("canonical test handle")
}

fn checksum(bytes: &[u8]) -> UploadChecksum {
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").expect("write to string");
    }
    UploadChecksum::parse(&encoded).expect("sha-256 checksum")
}

fn limits() -> UploadLimits {
    let mut config = UploadLimitConfig::reference();
    config.max_file_bytes = 8;
    config.max_aggregate_bytes = 16;
    config.max_chunk_bytes = 4;
    config.max_chunks_per_file = 2;
    config.max_in_flight_bytes = 4;
    UploadLimits::new(config).expect("conformance limits")
}

struct TestBody {
    bytes: Option<Bytes>,
}

impl TestBody {
    fn new(bytes: &[u8]) -> Self {
        Self {
            bytes: Some(Bytes::copy_from_slice(bytes)),
        }
    }
}

impl ChunkBody for TestBody {
    fn next_chunk<'a>(
        &'a mut self,
        _maximum_bytes: usize,
    ) -> Pin<Box<dyn Future<Output = Result<Option<Bytes>, UploadError>> + Send + 'a>> {
        Box::pin(async move { Ok(self.bytes.take()) })
    }
}

#[async_trait]
trait ProviderConformance: Send + Sync {
    async fn prepare(
        &self,
        handle: &UploadHandle,
        expected_bytes: u64,
    ) -> Result<Vec<TransferInstruction>, UploadError>;

    async fn submit(
        &self,
        handle: &UploadHandle,
        index: u32,
        offset: u64,
        bytes: &[u8],
    ) -> Result<ChunkDisposition, UploadError>;

    async fn submit_foreign(
        &self,
        source: &UploadHandle,
        target: &UploadHandle,
        bytes: &[u8],
    ) -> Result<(), UploadError>;

    async fn verify(
        &self,
        handle: &UploadHandle,
        expected: &UploadChecksum,
    ) -> Result<u64, UploadError>;

    async fn cancel(&self, handle: &UploadHandle) -> Result<(), UploadError>;
    async fn expire(&self, handle: &UploadHandle) -> Result<(), UploadError>;
    async fn cleanup(&self, handle: &UploadHandle) -> Result<(), UploadError>;
}

struct FileHarness {
    provider: QuarantinedFileProvider<TokioFileQuarantineStore>,
}

#[async_trait]
impl ProviderConformance for FileHarness {
    async fn prepare(
        &self,
        handle: &UploadHandle,
        expected_bytes: u64,
    ) -> Result<Vec<TransferInstruction>, UploadError> {
        let plan = self
            .provider
            .prepare(PrepareTransfer::new(
                handle,
                expected_bytes,
                "client-name.bin",
                UnixMillis::new(1_000),
            ))
            .await?;
        Ok(plan.instructions().cloned().collect())
    }

    async fn submit(
        &self,
        handle: &UploadHandle,
        index: u32,
        offset: u64,
        bytes: &[u8],
    ) -> Result<ChunkDisposition, UploadError> {
        let expected = checksum(bytes);
        let mut body = TestBody::new(bytes);
        self.provider
            .write_chunk(
                WriteChunk::new(handle, index, offset, bytes.len() as u64, &expected),
                &mut body,
            )
            .await
            .map(|receipt| receipt.disposition())
    }

    async fn submit_foreign(
        &self,
        _source: &UploadHandle,
        target: &UploadHandle,
        bytes: &[u8],
    ) -> Result<(), UploadError> {
        self.submit(target, 0, 0, bytes).await.map(|_| ())
    }

    async fn verify(
        &self,
        handle: &UploadHandle,
        expected: &UploadChecksum,
    ) -> Result<u64, UploadError> {
        self.provider
            .verify(VerifyTransfer::new(handle, expected))
            .await
            .map(|evidence| evidence.bytes())
    }

    async fn cancel(&self, handle: &UploadHandle) -> Result<(), UploadError> {
        self.provider.cancel(handle).await
    }

    async fn expire(&self, handle: &UploadHandle) -> Result<(), UploadError> {
        self.provider.expire(handle).await
    }

    async fn cleanup(&self, handle: &UploadHandle) -> Result<(), UploadError> {
        self.provider.cleanup(handle).await
    }
}

struct DirectHarness {
    provider: DirectProviderConformanceAdapter,
    instructions: Mutex<HashMap<(UploadHandle, u32), DirectTransferInstruction>>,
}

impl DirectHarness {
    fn remember(&self, handle: &UploadHandle, instruction: &DirectTransferInstruction) {
        self.instructions.lock().expect("instruction lock").insert(
            (handle.clone(), instruction.part().index()),
            instruction.clone(),
        );
    }

    fn instruction(&self, handle: &UploadHandle, index: u32) -> DirectTransferInstruction {
        self.instructions
            .lock()
            .expect("instruction lock")
            .get(&(handle.clone(), index))
            .cloned()
            .expect("prepared direct instruction")
    }
}

#[async_trait]
impl ProviderConformance for DirectHarness {
    async fn prepare(
        &self,
        handle: &UploadHandle,
        expected_bytes: u64,
    ) -> Result<Vec<TransferInstruction>, UploadError> {
        let plan = self
            .provider
            .prepare(PrepareTransfer::new(
                handle,
                expected_bytes,
                "client-name.bin",
                UnixMillis::new(1_000),
            ))
            .await?;
        for instruction in plan.instructions() {
            if let Some(direct) = instruction.as_direct() {
                self.remember(handle, direct);
            }
        }
        Ok(plan.instructions().cloned().collect())
    }

    async fn submit(
        &self,
        handle: &UploadHandle,
        index: u32,
        _offset: u64,
        bytes: &[u8],
    ) -> Result<ChunkDisposition, UploadError> {
        let instruction = self.instruction(handle, index);
        self.provider
            .store_part_for_test(&instruction, bytes, UnixMillis::new(1_001))?;
        let receipt = self
            .provider
            .report_part(ReportDirectPart::new(
                handle,
                instruction.part().clone(),
                instruction.reference().clone(),
                UnixMillis::new(1_001),
            ))
            .await?;
        if let Some(next) = receipt
            .next_instruction()
            .and_then(TransferInstruction::as_direct)
        {
            self.remember(handle, next);
        }
        Ok(receipt.disposition())
    }

    async fn submit_foreign(
        &self,
        source: &UploadHandle,
        target: &UploadHandle,
        bytes: &[u8],
    ) -> Result<(), UploadError> {
        let instruction = self.instruction(source, 0);
        self.provider
            .store_part_for_test(&instruction, bytes, UnixMillis::new(1_001))?;
        self.provider
            .report_part(ReportDirectPart::new(
                target,
                instruction.part().clone(),
                instruction.reference().clone(),
                UnixMillis::new(1_001),
            ))
            .await
            .map(|_| ())
    }

    async fn verify(
        &self,
        handle: &UploadHandle,
        expected: &UploadChecksum,
    ) -> Result<u64, UploadError> {
        self.provider
            .verify(VerifyTransfer::new(handle, expected))
            .await
            .map(|evidence| evidence.bytes())
    }

    async fn cancel(&self, handle: &UploadHandle) -> Result<(), UploadError> {
        self.provider.cancel(handle).await
    }

    async fn expire(&self, handle: &UploadHandle) -> Result<(), UploadError> {
        self.provider.expire(handle).await
    }

    async fn cleanup(&self, handle: &UploadHandle) -> Result<(), UploadError> {
        self.provider.cleanup(handle).await
    }
}

async fn assert_provider_conformance(provider: &dyn ProviderConformance) {
    let primary = handle(PRIMARY);
    let foreign = handle(FOREIGN);
    let instructions = provider
        .prepare(&primary, 8)
        .await
        .expect("prepare primary transfer");
    assert!(!instructions.is_empty());
    assert!(instructions.iter().all(TransferInstruction::is_constrained));

    let replay = provider
        .prepare(&primary, 8)
        .await
        .expect("exact prepare replay");
    assert_eq!(replay, instructions);

    let error = provider
        .submit_foreign(&primary, &foreign, b"abcd")
        .await
        .expect_err("cross-upload part");
    assert!(matches!(
        error.kind(),
        UploadErrorKind::ScopeMismatch | UploadErrorKind::UploadConflict
    ));

    let error = provider
        .verify(&primary, &checksum(b"abcdefgh"))
        .await
        .expect_err("completion before all parts");
    assert_eq!(error.kind(), UploadErrorKind::IncompleteTransfer);

    assert_eq!(
        provider
            .submit(&primary, 0, 0, b"abcd")
            .await
            .expect("first part"),
        ChunkDisposition::Stored
    );
    assert_eq!(
        provider
            .submit(&primary, 0, 0, b"abcd")
            .await
            .expect("exact part replay"),
        ChunkDisposition::ExistingOutcome
    );
    assert_eq!(
        provider
            .submit(&primary, 1, 4, b"efgh")
            .await
            .expect("second part"),
        ChunkDisposition::Stored
    );
    assert_eq!(
        provider
            .verify(&primary, &checksum(b"abcdefgh"))
            .await
            .expect("complete integrity"),
        8
    );

    provider
        .prepare(&foreign, 4)
        .await
        .expect("prepare foreign transfer");
    provider.cancel(&foreign).await.expect("cancel");
    provider.cancel(&foreign).await.expect("idempotent cancel");
    provider
        .expire(&foreign)
        .await
        .expect("expire canceled upload");
    provider.expire(&foreign).await.expect("idempotent expiry");
    provider
        .cleanup(&foreign)
        .await
        .expect("cleanup canceled upload");
    provider
        .cleanup(&foreign)
        .await
        .expect("idempotent cleanup");
}

#[tokio::test]
async fn file_and_direct_providers_share_one_conformance_contract() {
    let root = TempRoot::new();
    let file_store = Arc::new(
        TokioFileQuarantineStore::open(root.path(), 2, 16)
            .await
            .expect("file store"),
    );
    let file = FileHarness {
        provider: QuarantinedFileProvider::new(file_store, limits()).expect("file provider"),
    };
    assert_provider_conformance(&file).await;

    let origin = TrustedProviderOrigin::parse("https://uploads.example.test")
        .expect("trusted provider origin");
    let direct = DirectHarness {
        provider: DirectProviderConformanceAdapter::new(limits(), origin)
            .expect("direct conformance adapter"),
        instructions: Mutex::new(HashMap::new()),
    };
    assert_provider_conformance(&direct).await;
}

#[test]
fn direct_instruction_types_reject_unbounded_or_cross_origin_authority() {
    let origin = TrustedProviderOrigin::parse("https://uploads.example.test")
        .expect("trusted provider origin");
    assert!(TrustedProviderOrigin::parse("http://uploads.example.test").is_err());
    assert!(TrustedProviderOrigin::parse("https://user@uploads.example.test").is_err());
    assert!(TrustedProviderOrigin::parse("https://uploads.example.test/path").is_err());
    assert!(TrustedProviderUrl::parse("https://evil.example.test/part", &origin).is_err());
    assert!(
        TrustedProviderUrl::parse("https://uploads.example.test/part#fragment", &origin).is_err()
    );
    assert!(TrustedProviderUrl::parse("https://uploads.example.test/\\evil", &origin).is_err());
    let oversized_url = format!("https://uploads.example.test/{}", "a".repeat(2_048));
    assert!(TrustedProviderUrl::parse(&oversized_url, &origin).is_err());

    let endpoint = TrustedProviderUrl::parse(
        "https://uploads.example.test/part?credential=secret",
        &origin,
    )
    .expect("bound endpoint");
    let headers =
        BoundedHeaders::parse(&[("x-upload-token", "header-secret")]).expect("bounded headers");
    let part = UploadPart::new(0, 0, 4).expect("bounded part");
    let reference =
        DirectPartReference::parse("00112233445566778899aabbccddeeff").expect("part reference");

    let valid = DirectTransferInstruction::new(
        TransferMethod::Put,
        endpoint.clone(),
        headers.clone(),
        part.clone(),
        reference.clone(),
        UnixMillis::new(1_000),
        UnixMillis::new(901_000),
        4,
    )
    .expect("maximum bounded lifetime");
    assert!(valid.is_current(UnixMillis::new(900_999)));
    assert!(!valid.is_current(UnixMillis::new(901_000)));
    assert!(
        DirectTransferInstruction::new(
            TransferMethod::Put,
            endpoint.clone(),
            headers.clone(),
            part.clone(),
            reference.clone(),
            UnixMillis::new(1_000),
            UnixMillis::new(1_000),
            4,
        )
        .is_err()
    );
    assert!(
        DirectTransferInstruction::new(
            TransferMethod::Put,
            endpoint.clone(),
            headers.clone(),
            part.clone(),
            reference.clone(),
            UnixMillis::new(1_000),
            UnixMillis::new(901_001),
            4,
        )
        .is_err()
    );
    assert!(
        DirectTransferInstruction::new(
            TransferMethod::Put,
            endpoint,
            headers,
            part,
            reference,
            UnixMillis::new(1_000),
            UnixMillis::new(2_000),
            3,
        )
        .is_err()
    );
    assert!(UploadPart::new(0, u64::MAX, 2).is_err());
    assert!(BoundedHeaders::parse(&[("x-upload-token", "bad\r\nvalue")]).is_err());
    assert!(BoundedHeaders::parse(&[("host", "uploads.example.test")]).is_err());
    assert!(BoundedHeaders::parse(&[("sec-fetch-site", "same-origin")]).is_err());
    assert!(BoundedHeaders::parse(&[("x-part", "one"), ("x-part", "two")]).is_err());
    let oversized_value = "x".repeat(1_025);
    assert!(BoundedHeaders::parse(&[("x-part", &oversized_value)]).is_err());
    let names = (0..17)
        .map(|index| format!("x-part-{index}"))
        .collect::<Vec<_>>();
    let too_many = names
        .iter()
        .map(|name| (name.as_str(), "value"))
        .collect::<Vec<_>>();
    assert!(BoundedHeaders::parse(&too_many).is_err());
    assert!(DirectPartReference::parse("00112233445566778899AABBCCDDEEFF").is_err());
}

#[test]
fn direct_instruction_diagnostics_redact_provider_credentials() {
    let origin = TrustedProviderOrigin::parse("https://uploads.example.test")
        .expect("trusted provider origin");
    let endpoint = TrustedProviderUrl::parse(
        "https://uploads.example.test/part?credential=url-secret",
        &origin,
    )
    .expect("bound endpoint");
    let instruction = DirectTransferInstruction::new(
        TransferMethod::Put,
        endpoint,
        BoundedHeaders::parse(&[("x-upload-token", "header-secret")]).expect("bounded headers"),
        UploadPart::new(0, 0, 4).expect("bounded part"),
        DirectPartReference::parse("00112233445566778899aabbccddeeff").expect("part reference"),
        UnixMillis::new(1_000),
        UnixMillis::new(2_000),
        4,
    )
    .expect("constrained instruction");

    let debug = format!("{instruction:?}");
    for sentinel in [
        "url-secret",
        "header-secret",
        "00112233445566778899aabbccddeeff",
    ] {
        assert!(!debug.contains(sentinel));
    }
    assert!(instruction.is_constrained());
}

#[tokio::test]
async fn direct_adapter_rejects_excess_parts_and_renews_expired_instructions() {
    let origin = TrustedProviderOrigin::parse("https://uploads.example.test")
        .expect("trusted provider origin");
    let mut config = UploadLimitConfig::reference();
    config.max_file_bytes = 12;
    config.max_aggregate_bytes = 16;
    config.max_chunk_bytes = 4;
    config.max_chunks_per_file = 2;
    config.max_in_flight_bytes = 4;
    let bounded = UploadLimits::new(config).expect("bounded limits");
    let provider =
        DirectProviderConformanceAdapter::new(bounded, origin.clone()).expect("direct adapter");
    let upload = handle(PRIMARY);
    let error = provider
        .prepare(PrepareTransfer::new(
            &upload,
            12,
            "too-many-parts.bin",
            UnixMillis::new(1_000),
        ))
        .await
        .expect_err("part metadata bound");
    assert_eq!(error.kind(), UploadErrorKind::InputTooLarge);

    let provider = DirectProviderConformanceAdapter::new(limits(), origin).expect("direct adapter");
    let plan = provider
        .prepare(PrepareTransfer::new(
            &upload,
            4,
            "bounded.bin",
            UnixMillis::new(1_000),
        ))
        .await
        .expect("prepare direct transfer");
    let instruction = plan
        .instructions()
        .next()
        .and_then(TransferInstruction::as_direct)
        .expect("direct instruction");
    let error = provider
        .store_part_for_test(instruction, b"safe", instruction.expires_at())
        .expect_err("exclusive instruction expiry");
    assert_eq!(error.kind(), UploadErrorKind::UploadExpired);

    let refreshed = provider
        .prepare(PrepareTransfer::new(
            &upload,
            4,
            "bounded.bin",
            instruction.expires_at(),
        ))
        .await
        .expect("refresh expired direct instruction");
    assert_eq!(
        refreshed.disposition(),
        TransferDisposition::ExistingOutcome
    );
    let refreshed_instruction = refreshed
        .instructions()
        .next()
        .and_then(TransferInstruction::as_direct)
        .expect("refreshed instruction");
    assert_ne!(
        refreshed_instruction.reference(),
        instruction.reference(),
        "renewal must retire the expired provider-part binding"
    );
    assert!(refreshed_instruction.is_current(instruction.expires_at()));
    provider
        .store_part_for_test(refreshed_instruction, b"safe", instruction.expires_at())
        .expect("refreshed direct instruction stores the part");

    let debug = format!("{plan:?}");
    assert!(!debug.contains(instruction.endpoint().as_str()));
    assert!(!debug.contains(instruction.reference().as_str()));
}

struct TempRoot {
    path: PathBuf,
}

impl TempRoot {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let suffix = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "suprnova-live-direct-provider-{}-{suffix}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("create temporary root");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
