//! Non-authoritative reporting of already accepted Live outcomes.

use suprnova_live::component::LiveFuture;
use suprnova_live::execution::{
    AcceptedExecutionReport, AcceptedOutcomeReporter, HostError, HostErrorKind,
};

pub(crate) struct SuprnovaOutcomeReporter;

pub(crate) async fn dispatch_accepted_outcome(
    revision: u64,
    outcome: suprnova_live::ledger::AcceptedOutcomeKind,
) -> Result<(), HostError> {
    let event = crate::live::LiveOutcomeAccepted::new(revision, outcome);
    crate::events::EventFacade::dispatch(event)
        .await
        .map_err(|_| HostError::new(HostErrorKind::Reporting))
}

impl AcceptedOutcomeReporter for SuprnovaOutcomeReporter {
    fn report(&self, report: AcceptedExecutionReport) -> LiveFuture<'_, Result<(), HostError>> {
        Box::pin(dispatch_accepted_outcome(
            report.revision().get(),
            report.outcome(),
        ))
    }
}
