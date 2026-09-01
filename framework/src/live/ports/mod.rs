//! Framework-owned adapters for engine-defined host boundaries.

use std::sync::Arc;

use suprnova_live::action::ActionAuthorizationPort;
use suprnova_live::execution::{AcceptedOutcomeReporter, ExecutionTracePort, TransactionPort};
use suprnova_live::upload::{
    DirectUploadProvider, QuarantineStore, ReverseProxyUploadProvider, UploadApplicationValidator,
    UploadAuthorizationPort, UploadCleanupLedger, UploadFinalizer, UploadLedger, UploadProvider,
    UploadScanner, UploadValidationStore,
};
use suprnova_live::validation::ValidationPort;

pub(crate) mod authorization;
pub(crate) mod cancellation;
pub(crate) mod events;
pub(crate) mod response;
pub(crate) mod telemetry;
pub(crate) mod transaction;
pub(crate) mod upload;
pub(crate) mod upload_finalizer;
pub(crate) mod upload_ledger;
pub(crate) mod upload_provider;
pub(crate) mod upload_validation;
pub(crate) mod validation;

pub(crate) struct HostPorts {
    pub(crate) authorization: Arc<dyn ActionAuthorizationPort>,
    pub(crate) transaction: Arc<dyn TransactionPort>,
    pub(crate) validation: Arc<dyn ValidationPort>,
    pub(crate) reporter: Arc<dyn AcceptedOutcomeReporter>,
    pub(crate) trace: Arc<dyn ExecutionTracePort>,
    pub(crate) cancellation: Arc<cancellation::SuprnovaCancellationPort>,
    pub(crate) response: Arc<response::SuprnovaResponseIntentPort>,
    pub(crate) uploads: UploadHostPorts,
}

pub(crate) struct UploadHostPorts {
    pub(crate) operation_locks: Arc<super::upload::UploadOperationLocks>,
    pub(crate) ledger: Arc<dyn UploadLedger>,
    pub(crate) cleanup_ledger: Arc<dyn UploadCleanupLedger>,
    pub(crate) quarantine: Arc<dyn QuarantineStore>,
    pub(crate) provider: Arc<dyn UploadProvider>,
    pub(crate) provider_adapter: Arc<upload_provider::SuprnovaUploadProviderRouter>,
    pub(crate) reverse_proxy: Arc<dyn ReverseProxyUploadProvider>,
    pub(crate) reverse_proxy_adapter: Arc<upload_provider::SuprnovaReverseProxyUploadProvider>,
    pub(crate) direct: Arc<dyn DirectUploadProvider>,
    pub(crate) authorization_adapter: Arc<upload::SuprnovaUploadAuthorization>,
    pub(crate) authorization: Arc<dyn UploadAuthorizationPort>,
    pub(crate) scanner: Arc<dyn UploadScanner>,
    pub(crate) application_validation: Arc<dyn UploadApplicationValidator>,
    pub(crate) evidence: Arc<dyn UploadValidationStore>,
    pub(crate) finalizer: Arc<dyn UploadFinalizer>,
}

impl HostPorts {
    pub(crate) fn new(registry: &super::LiveRegistry) -> Result<Self, crate::FrameworkError> {
        let configured = crate::App::resolve::<super::LiveUploadHost>().ok();
        let limits = suprnova_live::limits::UploadLimits::new(
            suprnova_live::limits::UploadLimitConfig::reference(),
        )
        .map_err(|_| crate::FrameworkError::internal("Live upload limits were rejected"))?;
        let operation_locks = Arc::new(super::upload::UploadOperationLocks::default());
        let ledger = Arc::new(
            upload_ledger::SuprnovaUploadLedger::new(limits, Arc::clone(&operation_locks))
                .map_err(|_| crate::FrameworkError::internal("Live upload ledger was rejected"))?,
        );
        let quarantine = Arc::new(
            upload_provider::SuprnovaQuarantineStore::temporary(
                limits.max_concurrent_transfers().saturating_mul(2).max(2),
                limits.max_chunk_bytes(),
            )
            .map_err(|_| crate::FrameworkError::internal("Live quarantine store was rejected"))?,
        );
        let reverse_proxy = Arc::new(
            upload_provider::SuprnovaReverseProxyUploadProvider::new(
                Arc::clone(&quarantine),
                limits,
            )
            .map_err(|_| crate::FrameworkError::internal("Live upload provider was rejected"))?,
        );
        let direct = configured
            .as_ref()
            .and_then(super::LiveUploadHost::direct_provider)
            .unwrap_or_else(|| Arc::new(upload_provider::UnavailableDirectUploadProvider));
        let provider_adapter = Arc::new(
            upload_provider::SuprnovaUploadProviderRouter::new(
                Arc::clone(&reverse_proxy),
                Arc::clone(&direct),
                limits,
            )
            .map_err(|_| crate::FrameworkError::internal("Live upload provider was rejected"))?,
        );
        let evidence = Arc::new(upload_validation::SuprnovaUploadValidationStore::default());
        let upload_authorization = Arc::new(upload::SuprnovaUploadAuthorization);
        let uploads = UploadHostPorts {
            operation_locks,
            ledger: Arc::clone(&ledger) as Arc<dyn UploadLedger>,
            cleanup_ledger: ledger as Arc<dyn UploadCleanupLedger>,
            quarantine: quarantine as Arc<dyn QuarantineStore>,
            provider: Arc::clone(&provider_adapter) as Arc<dyn UploadProvider>,
            provider_adapter,
            reverse_proxy_adapter: Arc::clone(&reverse_proxy),
            reverse_proxy: reverse_proxy as Arc<dyn ReverseProxyUploadProvider>,
            direct: direct as Arc<dyn DirectUploadProvider>,
            authorization_adapter: Arc::clone(&upload_authorization),
            authorization: upload_authorization as Arc<dyn UploadAuthorizationPort>,
            scanner: configured
                .as_ref()
                .and_then(super::LiveUploadHost::scanner)
                .unwrap_or_else(|| Arc::new(upload_validation::SuprnovaUploadScanner)),
            application_validation: configured
                .as_ref()
                .and_then(super::LiveUploadHost::application_validator)
                .unwrap_or_else(|| Arc::new(upload_validation::SuprnovaUploadApplicationValidator)),
            evidence: evidence as Arc<dyn UploadValidationStore>,
            finalizer: configured
                .as_ref()
                .and_then(super::LiveUploadHost::finalizer)
                .unwrap_or_else(|| Arc::new(upload_finalizer::SuprnovaUploadFinalizer)),
        };
        Ok(Self {
            authorization: Arc::new(authorization::SuprnovaActionAuthorization),
            transaction: Arc::new(transaction::SuprnovaTransactionPort),
            validation: Arc::new(validation::SuprnovaValidationPort::new(registry.clone())),
            reporter: Arc::new(events::SuprnovaOutcomeReporter),
            trace: Arc::new(telemetry::SuprnovaExecutionTrace),
            cancellation: Arc::new(cancellation::SuprnovaCancellationPort),
            response: Arc::new(response::SuprnovaResponseIntentPort),
            uploads,
        })
    }

    pub(super) fn candidates(&self) -> HostPortCandidates {
        HostPortCandidates {
            authorization: Some(Arc::clone(&self.authorization)),
            transaction: Some(Arc::clone(&self.transaction)),
            validation: Some(Arc::clone(&self.validation)),
            reporter: Some(Arc::clone(&self.reporter)),
            trace: Some(Arc::clone(&self.trace)),
            cancellation: Some(Arc::clone(&self.cancellation)),
            response: Some(Arc::clone(&self.response)),
            upload_operation_locks: Some(Arc::clone(&self.uploads.operation_locks)),
            upload_ledger: Some(Arc::clone(&self.uploads.ledger)),
            upload_cleanup_ledger: Some(Arc::clone(&self.uploads.cleanup_ledger)),
            upload_quarantine: Some(Arc::clone(&self.uploads.quarantine)),
            upload_provider: Some(Arc::clone(&self.uploads.provider)),
            upload_provider_adapter: Some(Arc::clone(&self.uploads.provider_adapter)),
            upload_reverse_proxy: Some(Arc::clone(&self.uploads.reverse_proxy)),
            upload_reverse_proxy_adapter: Some(Arc::clone(&self.uploads.reverse_proxy_adapter)),
            upload_direct: Some(Arc::clone(&self.uploads.direct)),
            upload_authorization_adapter: Some(Arc::clone(&self.uploads.authorization_adapter)),
            upload_authorization: Some(Arc::clone(&self.uploads.authorization)),
            upload_scanner: Some(Arc::clone(&self.uploads.scanner)),
            upload_application_validation: Some(Arc::clone(&self.uploads.application_validation)),
            upload_evidence: Some(Arc::clone(&self.uploads.evidence)),
            upload_finalizer: Some(Arc::clone(&self.uploads.finalizer)),
        }
    }
}

pub(super) struct HostPortCandidates {
    pub(super) authorization: Option<Arc<dyn ActionAuthorizationPort>>,
    pub(super) transaction: Option<Arc<dyn TransactionPort>>,
    pub(super) validation: Option<Arc<dyn ValidationPort>>,
    pub(super) reporter: Option<Arc<dyn AcceptedOutcomeReporter>>,
    pub(super) trace: Option<Arc<dyn ExecutionTracePort>>,
    pub(super) cancellation: Option<Arc<cancellation::SuprnovaCancellationPort>>,
    pub(super) response: Option<Arc<response::SuprnovaResponseIntentPort>>,
    pub(super) upload_operation_locks: Option<Arc<super::upload::UploadOperationLocks>>,
    pub(super) upload_ledger: Option<Arc<dyn UploadLedger>>,
    pub(super) upload_cleanup_ledger: Option<Arc<dyn UploadCleanupLedger>>,
    pub(super) upload_quarantine: Option<Arc<dyn QuarantineStore>>,
    pub(super) upload_provider: Option<Arc<dyn UploadProvider>>,
    pub(super) upload_provider_adapter: Option<Arc<upload_provider::SuprnovaUploadProviderRouter>>,
    pub(super) upload_reverse_proxy: Option<Arc<dyn ReverseProxyUploadProvider>>,
    pub(super) upload_reverse_proxy_adapter:
        Option<Arc<upload_provider::SuprnovaReverseProxyUploadProvider>>,
    pub(super) upload_direct: Option<Arc<dyn DirectUploadProvider>>,
    pub(super) upload_authorization_adapter: Option<Arc<upload::SuprnovaUploadAuthorization>>,
    pub(super) upload_authorization: Option<Arc<dyn UploadAuthorizationPort>>,
    pub(super) upload_scanner: Option<Arc<dyn UploadScanner>>,
    pub(super) upload_application_validation: Option<Arc<dyn UploadApplicationValidator>>,
    pub(super) upload_evidence: Option<Arc<dyn UploadValidationStore>>,
    pub(super) upload_finalizer: Option<Arc<dyn UploadFinalizer>>,
}

impl HostPortCandidates {
    pub(super) fn production(
        registry: &super::LiveRegistry,
    ) -> Result<Self, crate::FrameworkError> {
        Ok(HostPorts::new(registry)?.candidates())
    }

    pub(super) fn finalize(
        self,
        missing: impl FnOnce(&'static str) -> crate::FrameworkError + Copy,
    ) -> Result<HostPorts, crate::FrameworkError> {
        Ok(HostPorts {
            authorization: self.authorization.ok_or_else(|| missing("authorization"))?,
            transaction: self.transaction.ok_or_else(|| missing("transaction"))?,
            validation: self.validation.ok_or_else(|| missing("validation"))?,
            reporter: self.reporter.ok_or_else(|| missing("event reporter"))?,
            trace: self.trace.ok_or_else(|| missing("telemetry"))?,
            cancellation: self.cancellation.ok_or_else(|| missing("cancellation"))?,
            response: self.response.ok_or_else(|| missing("response intent"))?,
            uploads: UploadHostPorts {
                operation_locks: self
                    .upload_operation_locks
                    .ok_or_else(|| missing("upload operation locks"))?,
                ledger: self.upload_ledger.ok_or_else(|| missing("upload ledger"))?,
                cleanup_ledger: self
                    .upload_cleanup_ledger
                    .ok_or_else(|| missing("upload cleanup ledger"))?,
                quarantine: self
                    .upload_quarantine
                    .ok_or_else(|| missing("upload quarantine"))?,
                provider: self
                    .upload_provider
                    .ok_or_else(|| missing("upload provider"))?,
                provider_adapter: self
                    .upload_provider_adapter
                    .ok_or_else(|| missing("upload provider"))?,
                reverse_proxy: self
                    .upload_reverse_proxy
                    .ok_or_else(|| missing("upload reverse-proxy provider"))?,
                reverse_proxy_adapter: self
                    .upload_reverse_proxy_adapter
                    .ok_or_else(|| missing("upload reverse-proxy progress"))?,
                direct: self
                    .upload_direct
                    .ok_or_else(|| missing("upload direct provider"))?,
                authorization_adapter: self
                    .upload_authorization_adapter
                    .ok_or_else(|| missing("upload authorization adapter"))?,
                authorization: self
                    .upload_authorization
                    .ok_or_else(|| missing("upload authorization"))?,
                scanner: self
                    .upload_scanner
                    .ok_or_else(|| missing("upload scanner"))?,
                application_validation: self
                    .upload_application_validation
                    .ok_or_else(|| missing("upload application validation"))?,
                evidence: self
                    .upload_evidence
                    .ok_or_else(|| missing("upload validation evidence"))?,
                finalizer: self
                    .upload_finalizer
                    .ok_or_else(|| missing("upload finalizer"))?,
            },
        })
    }
}
