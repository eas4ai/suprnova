//! Pure response-application planner with commit-after-morph semantics.

use super::{RenderPayload, ResponseOutcome, UpdateResponse};
use crate::error::RecoveryInstruction;

/// Observed browser morph phase when planning an accepted HTML response.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MorphDisposition {
    /// Morph has not run; used for redirects, no-render, and rejected outcomes.
    NotAttempted,
    /// Preflight and morph succeeded, so browser state may commit afterward.
    Succeeded,
    /// Server accepted but browser morph failed; original request must not replay.
    FailedAfterAcceptance,
}

/// One semantic application phase in the normative browser ordering model.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApplicationStep {
    /// Perform real document navigation and no other response work.
    Navigate,
    /// Validate morph feasibility before touching current DOM.
    PreflightMorph,
    /// Reconcile the current island with server-rendered HTML.
    Morph,
    /// Validate the explicit no-render outcome in place of morphing.
    ValidateNoRender,
    /// Atomically install the accepted snapshot and browser revision.
    CommitSnapshotAndRevision,
    /// Reconcile model proposals and validation metadata.
    ReconcileModelsAndValidation,
    /// Restore focus according to the later browser policy.
    RestoreFocus,
    /// Dispatch declared events.
    DispatchEvents,
    /// Invoke registered effects without evaluating arbitrary script.
    RunRegisteredEffects,
    /// Settle loading, dirty, success, and error feedback.
    SettleFeedback,
    /// Keep existing DOM and browser authority unchanged.
    RetainDom,
    /// Obtain fresh authorized island state without replaying the accepted request.
    RequestFreshRenderWithoutReplay,
    /// Obtain fresh authorized island state for a classified server rejection.
    RequestFreshIsland,
    /// Stop Live processing for this boundary while preserving ordinary HTML.
    StopLive,
}

/// Produces the normative semantic application order for a fully parsed response.
#[must_use]
pub fn application_plan(
    response: &UpdateResponse,
    morph: MorphDisposition,
) -> Vec<ApplicationStep> {
    if response.redirect().is_some() {
        return vec![ApplicationStep::Navigate];
    }
    match response.outcome() {
        ResponseOutcome::Rejected => {
            return vec![ApplicationStep::RetainDom, ApplicationStep::SettleFeedback];
        }
        ResponseOutcome::RefreshRequired => {
            return if response
                .error()
                .is_some_and(|error| error.recovery() == RecoveryInstruction::Navigate)
            {
                vec![ApplicationStep::Navigate]
            } else {
                vec![
                    ApplicationStep::RetainDom,
                    ApplicationStep::RequestFreshIsland,
                    ApplicationStep::SettleFeedback,
                ]
            };
        }
        ResponseOutcome::Fatal => {
            return if response
                .error()
                .is_some_and(|error| error.recovery() == RecoveryInstruction::Navigate)
            {
                vec![ApplicationStep::Navigate]
            } else {
                vec![
                    ApplicationStep::RetainDom,
                    ApplicationStep::StopLive,
                    ApplicationStep::SettleFeedback,
                ]
            };
        }
        ResponseOutcome::Accepted | ResponseOutcome::Duplicate => {}
    }
    match response.render() {
        Some(RenderPayload::Html(_)) if morph == MorphDisposition::FailedAfterAcceptance => vec![
            ApplicationStep::PreflightMorph,
            ApplicationStep::Morph,
            ApplicationStep::RequestFreshRenderWithoutReplay,
        ],
        Some(RenderPayload::Html(_)) => vec![
            ApplicationStep::PreflightMorph,
            ApplicationStep::Morph,
            ApplicationStep::CommitSnapshotAndRevision,
            ApplicationStep::ReconcileModelsAndValidation,
            ApplicationStep::RestoreFocus,
            ApplicationStep::DispatchEvents,
            ApplicationStep::RunRegisteredEffects,
            ApplicationStep::SettleFeedback,
        ],
        Some(RenderPayload::NoRender) => vec![
            ApplicationStep::ValidateNoRender,
            ApplicationStep::CommitSnapshotAndRevision,
            ApplicationStep::ReconcileModelsAndValidation,
            ApplicationStep::RestoreFocus,
            ApplicationStep::DispatchEvents,
            ApplicationStep::RunRegisteredEffects,
            ApplicationStep::SettleFeedback,
        ],
        None => vec![ApplicationStep::RetainDom, ApplicationStep::SettleFeedback],
    }
}
