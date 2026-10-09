//! Authoritative upload validation, scanning, and metadata contract tests.

mod component_support;

use std::fmt::Write as _;
use std::sync::{Arc, Mutex, MutexGuard};

use sha2::{Digest, Sha256};
use suprnova_live::crypto::{KeyRecord, RootKey, SnapshotKeyRing};
use suprnova_live::identity::{ActionName, KeyId, ModelField, UnixMillis};
use suprnova_live::limits::{UploadLimitConfig, UploadLimits};
use suprnova_live::metadata::FieldMetadata;
use suprnova_live::snapshot::state::{FieldCategory, StateCodec};
use suprnova_live::upload::{
    AcceptedUploadType, ApplicationValidationDecision, ApplicationValidationInput,
    AuthoritativeUploadType, ClientUploadMetadata, DetectedUploadType, IntegrityEvidence,
    MediaDimensions, MediaHeaderProbe, PrepareTransfer, QuarantineBytes, ReadUpload,
    ScanDisposition, ScanFailurePolicy, ScanInput, ScanReason, TransferGrantCodec, TransferPlan,
    UploadApplicationValidator, UploadChecksum, UploadDimensionLimits, UploadError,
    UploadErrorKind, UploadFieldPolicy, UploadFuture, UploadHandle, UploadIdempotencyKey,
    UploadLedger, UploadMediaType, UploadProvider, UploadRecord, UploadReplacementPolicy,
    UploadRevision, UploadScanPolicy, UploadScanner, UploadService, UploadState,
    UploadValidationDisposition, UploadValidationRequest, UploadValidationService,
    UploadValidationStore, ValidatedUpload, ValidationStoreDisposition, VerifyTransfer,
};
use suprnova_live_test_support::{ControlledUploadAuthorization, MemoryUploadLedger};

use component_support::{fixture_host_scope, trusted_context_with_upload_authorization};

const HANDLE: &str = "018f47c1-2af0-7cc4-a001-000000000001";
const ROOT_SECRET: &[u8] = b"upload-validation-root-secret-000";

fn limits() -> UploadLimits {
    UploadLimits::new(UploadLimitConfig::reference()).expect("reference upload limits")
}

fn handle() -> UploadHandle {
    UploadHandle::parse(HANDLE).expect("fixture handle")
}

fn field() -> ModelField {
    ModelField::parse("avatar").expect("field")
}

fn idempotency(value: &str) -> UploadIdempotencyKey {
    UploadIdempotencyKey::parse(value).expect("idempotency")
}

fn checksum(bytes: &[u8]) -> UploadChecksum {
    let mut encoded = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        write!(&mut encoded, "{byte:02x}").expect("checksum text");
    }
    UploadChecksum::parse(&encoded).expect("checksum")
}

fn codec() -> TransferGrantCodec {
    let key = KeyRecord::new(
        KeyId::parse("upload-validation-key").expect("key id"),
        RootKey::new(ROOT_SECRET.to_vec()).expect("root key"),
        UnixMillis::new(0),
        UnixMillis::new(10_000),
        UnixMillis::new(20_000),
    )
    .expect("key record");
    TransferGrantCodec::new(SnapshotKeyRing::new(key, Vec::new()).expect("key ring"))
}

fn policy(
    accepted: Vec<UploadMediaType>,
    scan: UploadScanPolicy,
    dimensions: Option<UploadDimensionLimits>,
) -> UploadFieldPolicy {
    UploadFieldPolicy::new(
        3,
        4 * 1024 * 1024,
        UploadReplacementPolicy::RetirePrevious,
        accepted,
        dimensions,
        scan,
        ActionName::parse("save_avatar").expect("action name"),
    )
    .expect("upload policy")
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = vec![
        0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13, b'I', b'H', b'D', b'R',
    ];
    bytes.extend_from_slice(&width.to_be_bytes());
    bytes.extend_from_slice(&height.to_be_bytes());
    bytes.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0]);
    bytes
}

#[test]
fn upload_field_policy_is_bounded_and_digest_significant() {
    let dimensions =
        UploadDimensionLimits::new(2_048, 2_048, 4_194_304).expect("finite dimensions");
    let base = policy(
        vec![UploadMediaType::Png, UploadMediaType::Jpeg],
        UploadScanPolicy::Required {
            on_timeout: ScanFailurePolicy::Retry,
            on_unavailable: ScanFailurePolicy::Reject,
        },
        Some(dimensions),
    );
    let reordered = policy(
        vec![UploadMediaType::Jpeg, UploadMediaType::Png],
        UploadScanPolicy::Required {
            on_timeout: ScanFailurePolicy::Retry,
            on_unavailable: ScanFailurePolicy::Reject,
        },
        Some(dimensions),
    );
    let changed = policy(
        vec![UploadMediaType::Png],
        UploadScanPolicy::Required {
            on_timeout: ScanFailurePolicy::Retry,
            on_unavailable: ScanFailurePolicy::Reject,
        },
        Some(dimensions),
    );

    assert_eq!(base.contract_digest(), reordered.contract_digest());
    assert_ne!(base.contract_digest(), changed.contract_digest());
    assert_eq!(base.maximum_files(), 3);
    assert_eq!(base.maximum_file_bytes(), 4 * 1024 * 1024);
    assert_eq!(base.finalize_action().as_str(), "save_avatar");

    let field = FieldMetadata::new(
        ModelField::parse("avatar").expect("field"),
        FieldCategory::Model,
        StateCodec::Json,
        false,
    )
    .with_upload_policy(base)
    .expect("upload metadata");
    assert!(field.upload_policy().is_some());

    let pdf =
        AcceptedUploadType::application("application/pdf", &["pdf"]).expect("PDF content contract");
    let pdf_with_alias = AcceptedUploadType::application("application/pdf", &["pdf", "xpdf"])
        .expect("PDF alias contract");
    let custom = UploadFieldPolicy::new_with_accepted_types(
        1,
        1024,
        UploadReplacementPolicy::RetirePrevious,
        vec![pdf.clone()],
        None,
        UploadScanPolicy::Disabled,
        ActionName::parse("save_avatar").expect("action name"),
    )
    .expect("custom policy");
    let changed_custom = UploadFieldPolicy::new_with_accepted_types(
        1,
        1024,
        UploadReplacementPolicy::RetirePrevious,
        vec![pdf_with_alias.clone()],
        None,
        UploadScanPolicy::Disabled,
        ActionName::parse("save_avatar").expect("action name"),
    )
    .expect("changed custom policy");
    assert_ne!(custom.contract_digest(), changed_custom.contract_digest());
    assert!(
        UploadFieldPolicy::new_with_accepted_types(
            1,
            1024,
            UploadReplacementPolicy::RetirePrevious,
            vec![pdf, pdf_with_alias],
            None,
            UploadScanPolicy::Disabled,
            ActionName::parse("save_avatar").expect("action name"),
        )
        .is_err()
    );
}

#[test]
fn media_probe_classifies_magic_and_reads_only_bounded_headers() {
    let bytes = png(320, 240);
    assert_eq!(MediaHeaderProbe::classify(&bytes), DetectedUploadType::Png);
    assert_eq!(
        MediaHeaderProbe::probe(&bytes).expect("bounded PNG header"),
        Some(MediaDimensions::new(320, 240).expect("dimensions"))
    );
    assert_eq!(
        MediaHeaderProbe::prefix_limit(DetectedUploadType::Png),
        Some(32)
    );
    assert_eq!(
        MediaHeaderProbe::prefix_limit(DetectedUploadType::Jpeg),
        Some(256 * 1024)
    );
}

#[test]
fn truncated_and_hostile_media_headers_fail_closed_without_overflow() {
    for bytes in [
        vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a],
        b"GIF89a\xff".to_vec(),
        b"RIFF\xff\xff\xff\xffWEBPVP8X".to_vec(),
        vec![0xff, 0xd8, 0xff, 0xe1, 0xff, 0xff],
    ] {
        assert!(MediaHeaderProbe::probe(&bytes).is_err());
    }
    assert!(MediaDimensions::new(0, u32::MAX).is_err());
}

#[test]
fn client_names_and_mime_claims_are_display_metadata_not_paths() {
    let normalized = ClientUploadMetadata::new(" avatar.png ", Some("image/png"))
        .expect("bounded display metadata");
    assert_eq!(normalized.display_name(), "avatar.png");
    assert_eq!(normalized.claimed_media_type(), Some("image/png"));

    for invalid in [
        "../avatar.png",
        "folder/avatar.png",
        "folder\\avatar.png",
        ".",
        "..",
    ] {
        assert!(ClientUploadMetadata::new(invalid, Some("image/png")).is_err());
    }
}

struct MemoryProvider {
    handle: UploadHandle,
    bytes: QuarantineBytes,
}

impl MemoryProvider {
    fn new(bytes: &[u8]) -> Self {
        Self {
            handle: handle(),
            bytes: QuarantineBytes::copy_from_slice(bytes),
        }
    }
}

impl UploadProvider for MemoryProvider {
    fn prepare<'a>(
        &'a self,
        _request: PrepareTransfer<'a>,
    ) -> UploadFuture<'a, Result<TransferPlan, UploadError>> {
        Box::pin(async move { Err(UploadError::new(UploadErrorKind::UploadConflict)) })
    }

    fn verify<'a>(
        &'a self,
        request: VerifyTransfer<'a>,
    ) -> UploadFuture<'a, Result<IntegrityEvidence, UploadError>> {
        Box::pin(async move {
            if request.handle() != &self.handle {
                return Err(UploadError::new(UploadErrorKind::ScopeMismatch));
            }
            let actual = checksum(&self.bytes);
            if request.checksum() != &actual {
                return Err(UploadError::new(UploadErrorKind::ChecksumMismatch));
            }
            Ok(IntegrityEvidence::from_provider(
                self.bytes.len() as u64,
                actual,
            ))
        })
    }

    fn read<'a>(
        &'a self,
        request: ReadUpload<'a>,
    ) -> UploadFuture<'a, Result<QuarantineBytes, UploadError>> {
        Box::pin(async move {
            if request.handle() != &self.handle || request.maximum_bytes() == 0 {
                return Err(UploadError::new(UploadErrorKind::ScopeMismatch));
            }
            let start = usize::try_from(request.offset())
                .map_err(|_| UploadError::new(UploadErrorKind::InvalidField))?;
            if start > self.bytes.len() {
                return Err(UploadError::new(UploadErrorKind::InvalidField));
            }
            let end = start
                .saturating_add(request.maximum_bytes())
                .min(self.bytes.len());
            Ok(self.bytes.slice(start..end))
        })
    }

    fn cancel<'a>(
        &'a self,
        _handle: &'a UploadHandle,
    ) -> UploadFuture<'a, Result<(), UploadError>> {
        Box::pin(async move { Ok(()) })
    }

    fn cleanup<'a>(
        &'a self,
        _handle: &'a UploadHandle,
    ) -> UploadFuture<'a, Result<(), UploadError>> {
        Box::pin(async move { Ok(()) })
    }
}

struct ControlledScanner {
    disposition: Mutex<ScanDisposition>,
}

impl ControlledScanner {
    fn new(disposition: ScanDisposition) -> Self {
        Self {
            disposition: Mutex::new(disposition),
        }
    }
}

impl UploadScanner for ControlledScanner {
    fn scan<'a>(
        &'a self,
        input: ScanInput<'a>,
    ) -> UploadFuture<'a, Result<ScanDisposition, UploadError>> {
        Box::pin(async move {
            assert!(input.deadline() > input.started_at());
            assert_eq!(input.content().deadline(), input.deadline());
            assert_eq!(input.upload().detected_type(), DetectedUploadType::Png);
            let prefix = input.content().read(0, 8).await?;
            assert_eq!(&prefix[..], b"\x89PNG\r\n\x1a\n");
            Ok(lock(&self.disposition).clone())
        })
    }
}

struct ControlledApplication {
    decision: ApplicationValidationDecision,
}

struct ClassifyingApplication {
    classified: AuthoritativeUploadType,
}

struct OversizedReadApplication;

impl UploadApplicationValidator for OversizedReadApplication {
    fn validate<'a>(
        &'a self,
        input: ApplicationValidationInput<'a>,
    ) -> UploadFuture<'a, Result<ApplicationValidationDecision, UploadError>> {
        Box::pin(async move {
            input
                .content()
                .read(0, input.content().maximum_read_bytes() + 1)
                .await?;
            Ok(ApplicationValidationDecision::Allow)
        })
    }
}

impl UploadApplicationValidator for ClassifyingApplication {
    fn validate<'a>(
        &'a self,
        input: ApplicationValidationInput<'a>,
    ) -> UploadFuture<'a, Result<ApplicationValidationDecision, UploadError>> {
        Box::pin(async move {
            assert_eq!(input.upload().detected_type(), DetectedUploadType::Unknown);
            assert_eq!(input.content().deadline(), input.deadline());
            let prefix = input.content().read(0, 8).await?;
            assert!(prefix.starts_with(b"%PDF-1.7"));
            Ok(ApplicationValidationDecision::AllowAs(
                self.classified.clone(),
            ))
        })
    }
}

impl UploadApplicationValidator for ControlledApplication {
    fn validate<'a>(
        &'a self,
        input: ApplicationValidationInput<'a>,
    ) -> UploadFuture<'a, Result<ApplicationValidationDecision, UploadError>> {
        Box::pin(async move {
            assert_eq!(input.upload().detected_type(), DetectedUploadType::Png);
            assert!(input.deadline() > input.started_at());
            assert_eq!(input.content().deadline(), input.deadline());
            Ok(self.decision.clone())
        })
    }
}

#[derive(Default)]
struct MemoryValidationStore {
    evidence: Mutex<Option<ValidatedUpload>>,
}

impl UploadValidationStore for MemoryValidationStore {
    fn put<'a>(
        &'a self,
        evidence: ValidatedUpload,
    ) -> UploadFuture<'a, Result<ValidationStoreDisposition, UploadError>> {
        Box::pin(async move {
            let mut stored = lock(&self.evidence);
            match stored.as_ref() {
                Some(existing) if existing == &evidence => {
                    Ok(ValidationStoreDisposition::ExistingOutcome)
                }
                Some(_) => Err(UploadError::new(UploadErrorKind::UploadConflict)),
                None => {
                    *stored = Some(evidence);
                    Ok(ValidationStoreDisposition::Stored)
                }
            }
        })
    }

    fn load<'a>(
        &'a self,
        upload: &'a UploadHandle,
    ) -> UploadFuture<'a, Result<Option<ValidatedUpload>, UploadError>> {
        Box::pin(async move {
            Ok(lock(&self.evidence)
                .as_ref()
                .filter(|evidence| evidence.handle() == upload)
                .cloned())
        })
    }

    fn remove<'a>(&'a self, upload: &'a UploadHandle) -> UploadFuture<'a, Result<(), UploadError>> {
        Box::pin(async move {
            let mut evidence = lock(&self.evidence);
            if evidence
                .as_ref()
                .is_some_and(|evidence| evidence.handle() == upload)
            {
                *evidence = None;
            }
            Ok(())
        })
    }
}

struct ValidationFixture {
    context: suprnova_live::host::TrustedLiveRequestContext,
    ledger: Arc<MemoryUploadLedger>,
    evidence: Arc<MemoryValidationStore>,
    service: UploadValidationService,
    policy: UploadFieldPolicy,
    bytes: Vec<u8>,
}

fn validation_fixture(
    scan_policy: UploadScanPolicy,
    scanner: Option<Arc<dyn UploadScanner>>,
    application: Option<Arc<dyn UploadApplicationValidator>>,
) -> ValidationFixture {
    let bytes = png(320, 240);
    let policy = policy(
        vec![UploadMediaType::Png],
        scan_policy,
        Some(UploadDimensionLimits::new(1_024, 1_024, 1_048_576).expect("dimension policy")),
    );
    validation_fixture_with_content(bytes, policy, scanner, application)
}

fn validation_fixture_with_content(
    bytes: Vec<u8>,
    policy: UploadFieldPolicy,
    scanner: Option<Arc<dyn UploadScanner>>,
    application: Option<Arc<dyn UploadApplicationValidator>>,
) -> ValidationFixture {
    let authorization = Arc::new(ControlledUploadAuthorization::new());
    let context = trusted_context_with_upload_authorization(authorization);
    let authority = suprnova_live::upload::TransferGrantScope::new(
        handle(),
        context.mount().component().clone(),
        field(),
        fixture_host_scope(),
        1,
    );
    let ledger = Arc::new(MemoryUploadLedger::new(limits()).expect("ledger"));
    ledger
        .seed(
            UploadRecord::new(
                authority,
                UploadState::Verifying,
                UploadRevision::new(7),
                UnixMillis::new(1_000),
                UnixMillis::new(1_900),
            )
            .expect("record"),
        )
        .expect("seed");
    let authority_service =
        Arc::new(UploadService::new(ledger.clone(), codec(), limits()).expect("authority service"));
    let provider = Arc::new(MemoryProvider::new(&bytes));
    let evidence = Arc::new(MemoryValidationStore::default());
    let service = UploadValidationService::new(
        authority_service,
        provider,
        evidence.clone(),
        scanner,
        application,
        limits(),
    )
    .expect("validation service");
    ValidationFixture {
        context,
        ledger,
        evidence,
        service,
        policy,
        bytes,
    }
}

fn validation_request(
    fixture: &ValidationFixture,
    client: ClientUploadMetadata,
) -> UploadValidationRequest {
    UploadValidationRequest::new(
        handle(),
        field(),
        UploadRevision::new(7),
        idempotency("validate-avatar"),
        client,
        fixture.bytes.len() as u64,
        checksum(&fixture.bytes),
        fixture.policy.clone(),
    )
}

#[tokio::test]
async fn authoritative_acceptance_persists_exact_evidence_before_ready() {
    let fixture = validation_fixture(
        UploadScanPolicy::Required {
            on_timeout: ScanFailurePolicy::Retry,
            on_unavailable: ScanFailurePolicy::Reject,
        },
        Some(Arc::new(ControlledScanner::new(ScanDisposition::Clean))),
        Some(Arc::new(ControlledApplication {
            decision: ApplicationValidationDecision::Allow,
        })),
    );
    let outcome = fixture
        .service
        .validate(
            &fixture.context,
            validation_request(
                &fixture,
                ClientUploadMetadata::new("avatar.png", Some("image/png")).expect("client"),
            ),
            UnixMillis::new(1_001),
        )
        .await
        .expect("validation");

    assert_eq!(outcome.disposition(), UploadValidationDisposition::Ready);
    let accepted = outcome.evidence().expect("validated evidence");
    assert_eq!(accepted.ready_revision(), UploadRevision::new(8));
    assert_eq!(accepted.inspection().bytes(), fixture.bytes.len() as u64);
    assert_eq!(
        accepted.inspection().dimensions(),
        Some(MediaDimensions::new(320, 240).expect("dimensions"))
    );
    assert_eq!(
        fixture.evidence.load(&handle()).await.expect("store load"),
        Some(accepted.clone())
    );
    assert_eq!(
        fixture
            .ledger
            .load(&handle())
            .await
            .expect("ledger load")
            .expect("record")
            .state(),
        UploadState::Ready
    );
}

#[tokio::test]
async fn custom_types_require_authoritative_application_classification() {
    let accepted =
        AcceptedUploadType::application("application/pdf", &["pdf"]).expect("PDF content contract");
    let policy = UploadFieldPolicy::new_with_accepted_types(
        1,
        4 * 1024 * 1024,
        UploadReplacementPolicy::RetirePrevious,
        vec![accepted],
        None,
        UploadScanPolicy::Disabled,
        ActionName::parse("save_avatar").expect("action name"),
    )
    .expect("custom upload policy");
    let bytes = b"%PDF-1.7\ntrusted-classifier-fixture".to_vec();

    let unclassified = validation_fixture_with_content(bytes.clone(), policy.clone(), None, None);
    let outcome = unclassified
        .service
        .validate(
            &unclassified.context,
            validation_request(
                &unclassified,
                ClientUploadMetadata::new("document.pdf", Some("application/pdf")).expect("client"),
            ),
            UnixMillis::new(1_001),
        )
        .await
        .expect("typed rejection");
    assert_eq!(outcome.disposition(), UploadValidationDisposition::Rejected);
    assert_eq!(
        outcome.reason(),
        Some(suprnova_live::upload::UploadRejectionReason::TypeMismatch)
    );

    let classified_type =
        AuthoritativeUploadType::application("application/pdf").expect("classified type");
    let classified = validation_fixture_with_content(
        bytes,
        policy,
        None,
        Some(Arc::new(ClassifyingApplication {
            classified: classified_type.clone(),
        })),
    );
    let outcome = classified
        .service
        .validate(
            &classified.context,
            validation_request(
                &classified,
                ClientUploadMetadata::new("document.pdf", Some("application/pdf")).expect("client"),
            ),
            UnixMillis::new(1_001),
        )
        .await
        .expect("classified validation");
    assert_eq!(outcome.disposition(), UploadValidationDisposition::Ready);
    assert_eq!(
        outcome
            .evidence()
            .expect("evidence")
            .inspection()
            .authoritative_type(),
        Some(&classified_type)
    );
}

#[tokio::test]
async fn application_content_reads_cannot_exceed_the_shared_chunk_bound() {
    let fixture = validation_fixture(
        UploadScanPolicy::Disabled,
        None,
        Some(Arc::new(OversizedReadApplication)),
    );

    let error = fixture
        .service
        .validate(
            &fixture.context,
            validation_request(
                &fixture,
                ClientUploadMetadata::new("avatar.png", Some("image/png")).expect("client"),
            ),
            UnixMillis::new(1_001),
        )
        .await
        .expect_err("oversized application read");

    assert_eq!(error.kind(), UploadErrorKind::InputTooLarge);
    assert_eq!(
        fixture
            .ledger
            .load(&handle())
            .await
            .expect("load")
            .expect("record")
            .state(),
        UploadState::Verifying
    );
}

#[tokio::test]
async fn mime_extension_and_dimension_disagreement_rejects_authoritative_content() {
    let fixture = validation_fixture(UploadScanPolicy::Disabled, None, None);
    let outcome = fixture
        .service
        .validate(
            &fixture.context,
            validation_request(
                &fixture,
                ClientUploadMetadata::new("avatar.jpg", Some("image/jpeg")).expect("client"),
            ),
            UnixMillis::new(1_001),
        )
        .await
        .expect("typed rejection");

    assert_eq!(outcome.disposition(), UploadValidationDisposition::Rejected);
    assert_eq!(
        outcome.reason(),
        Some(suprnova_live::upload::UploadRejectionReason::TypeMismatch)
    );
    assert!(
        fixture
            .evidence
            .load(&handle())
            .await
            .expect("load")
            .is_none()
    );
    assert_eq!(
        fixture
            .ledger
            .load(&handle())
            .await
            .expect("ledger load")
            .expect("record")
            .state(),
        UploadState::Rejected
    );
}

#[tokio::test]
async fn scan_timeout_and_unavailable_policy_never_silently_accepts() {
    let retry = validation_fixture(
        UploadScanPolicy::Required {
            on_timeout: ScanFailurePolicy::Retry,
            on_unavailable: ScanFailurePolicy::Reject,
        },
        Some(Arc::new(ControlledScanner::new(ScanDisposition::TimedOut))),
        None,
    );
    let outcome = retry
        .service
        .validate(
            &retry.context,
            validation_request(
                &retry,
                ClientUploadMetadata::new("avatar.png", Some("image/png")).expect("client"),
            ),
            UnixMillis::new(1_001),
        )
        .await
        .expect("retry outcome");
    assert_eq!(outcome.disposition(), UploadValidationDisposition::Retry);
    assert_eq!(
        retry
            .ledger
            .load(&handle())
            .await
            .expect("load")
            .expect("record")
            .state(),
        UploadState::Verifying
    );

    let reject = validation_fixture(
        UploadScanPolicy::Required {
            on_timeout: ScanFailurePolicy::Retry,
            on_unavailable: ScanFailurePolicy::Reject,
        },
        None,
        None,
    );
    let outcome = reject
        .service
        .validate(
            &reject.context,
            validation_request(
                &reject,
                ClientUploadMetadata::new("avatar.png", Some("image/png")).expect("client"),
            ),
            UnixMillis::new(1_001),
        )
        .await
        .expect("rejected unavailable scanner");
    assert_eq!(outcome.disposition(), UploadValidationDisposition::Rejected);
    assert_eq!(
        outcome.reason(),
        Some(suprnova_live::upload::UploadRejectionReason::ScanUnavailable)
    );
}

#[tokio::test]
async fn scanner_and_application_rejections_remain_typed() {
    let scanner = validation_fixture(
        UploadScanPolicy::Required {
            on_timeout: ScanFailurePolicy::Reject,
            on_unavailable: ScanFailurePolicy::Reject,
        },
        Some(Arc::new(ControlledScanner::new(ScanDisposition::Rejected(
            ScanReason::parse("malware").expect("reason"),
        )))),
        None,
    );
    let scanner_outcome = scanner
        .service
        .validate(
            &scanner.context,
            validation_request(
                &scanner,
                ClientUploadMetadata::new("avatar.png", Some("image/png")).expect("client"),
            ),
            UnixMillis::new(1_001),
        )
        .await
        .expect("scanner rejection");
    assert_eq!(
        scanner_outcome.reason(),
        Some(suprnova_live::upload::UploadRejectionReason::ScanRejected)
    );

    let application = validation_fixture(
        UploadScanPolicy::Disabled,
        None,
        Some(Arc::new(ControlledApplication {
            decision: ApplicationValidationDecision::Reject(
                ScanReason::parse("application_policy").expect("reason"),
            ),
        })),
    );
    let application_outcome = application
        .service
        .validate(
            &application.context,
            validation_request(
                &application,
                ClientUploadMetadata::new("avatar.png", Some("image/png")).expect("client"),
            ),
            UnixMillis::new(1_001),
        )
        .await
        .expect("application rejection");
    assert_eq!(
        application_outcome.reason(),
        Some(suprnova_live::upload::UploadRejectionReason::ApplicationRejected)
    );
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
