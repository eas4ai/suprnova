//! Framework-owned adapters for engine-defined host boundaries.

use std::sync::Arc;

use suprnova_live::action::ActionAuthorizationPort;
use suprnova_live::execution::{AcceptedOutcomeReporter, ExecutionTracePort, TransactionPort};
use suprnova_live::validation::ValidationPort;

pub(crate) mod authorization;
pub(crate) mod cancellation;
pub(crate) mod events;
pub(crate) mod response;
pub(crate) mod telemetry;
pub(crate) mod transaction;
pub(crate) mod validation;

pub(crate) struct HostPorts {
    pub(crate) authorization: Arc<dyn ActionAuthorizationPort>,
    pub(crate) transaction: Arc<dyn TransactionPort>,
    pub(crate) validation: Arc<dyn ValidationPort>,
    pub(crate) reporter: Arc<dyn AcceptedOutcomeReporter>,
    pub(crate) trace: Arc<dyn ExecutionTracePort>,
    pub(crate) cancellation: Arc<cancellation::SuprnovaCancellationPort>,
    pub(crate) response: Arc<response::SuprnovaResponseIntentPort>,
}

impl HostPorts {
    pub(crate) fn new(registry: &super::LiveRegistry) -> Self {
        Self {
            authorization: Arc::new(authorization::SuprnovaActionAuthorization),
            transaction: Arc::new(transaction::SuprnovaTransactionPort),
            validation: Arc::new(validation::SuprnovaValidationPort::new(registry.clone())),
            reporter: Arc::new(events::SuprnovaOutcomeReporter),
            trace: Arc::new(telemetry::SuprnovaExecutionTrace),
            cancellation: Arc::new(cancellation::SuprnovaCancellationPort),
            response: Arc::new(response::SuprnovaResponseIntentPort),
        }
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
}

impl HostPortCandidates {
    pub(super) fn production(registry: &super::LiveRegistry) -> Self {
        HostPorts::new(registry).candidates()
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
        })
    }
}
