//! Typed assertions over semantic component-harness outcomes.

use suprnova_live::execution::{AcceptedExecution, ExecutionResult, RefreshRequiredExecution};
use suprnova_live::identity::{Revision, RouteIdentity};

/// Assertion helpers that never format signed snapshots or arbitrary state.
pub struct HarnessAssertions;

impl HarnessAssertions {
    /// Requires one accepted outcome and returns its typed semantic result.
    #[must_use]
    pub fn accepted(result: &ExecutionResult) -> &AcceptedExecution {
        let ExecutionResult::Accepted(accepted) = result else {
            panic!("expected accepted Live execution");
        };
        accepted
    }

    /// Requires one refresh outcome and returns its typed recovery contract.
    #[must_use]
    pub fn refresh_required(result: &ExecutionResult) -> &RefreshRequiredExecution {
        let ExecutionResult::RefreshRequired(refresh) = result else {
            panic!("expected refresh-required Live execution");
        };
        refresh
    }

    /// Requires the accepted revision to equal the expected monotonic value.
    pub fn revision(accepted: &AcceptedExecution, expected: Revision) {
        assert_eq!(
            accepted.revision(),
            expected,
            "unexpected accepted revision"
        );
    }

    /// Requires rendered HTML and one expected non-secret fragment.
    pub fn html_contains(accepted: &AcceptedExecution, expected: &str) {
        let Some(render) = accepted.render() else {
            panic!("expected rendered Live output");
        };
        assert!(
            String::from_utf8_lossy(&render.body).contains(expected),
            "expected HTML fragment was absent"
        );
    }

    /// Requires one typed validation issue without formatting its surrounding state.
    pub fn validation_issue(accepted: &AcceptedExecution, path: &str, message: &str) {
        assert!(
            accepted.validation().issues().iter().any(|issue| {
                issue.path().as_str() == path && issue.message().as_str() == message
            }),
            "expected validation issue was absent"
        );
    }

    /// Requires one registered event by its safe browser identity and schema version.
    pub fn event(accepted: &AcceptedExecution, name: &str, version: u16) {
        assert!(
            accepted
                .result()
                .metadata()
                .events()
                .iter()
                .any(|event| event.name().as_str() == name && event.version() == version),
            "expected registered event was absent"
        );
    }

    /// Requires one registered effect by its safe browser identity and schema version.
    pub fn effect(accepted: &AcceptedExecution, name: &str, version: u16) {
        assert!(
            accepted
                .result()
                .metadata()
                .effects()
                .iter()
                .any(|effect| effect.name().as_str() == name && effect.version() == version),
            "expected registered effect was absent"
        );
    }

    /// Requires a terminal redirect to one already-resolved real route identity.
    pub fn redirect(accepted: &AcceptedExecution, expected: &RouteIdentity) {
        let suprnova_live::action::ActionOutcome::Redirect(route) = accepted.result().outcome()
        else {
            panic!("expected typed redirect outcome");
        };
        assert_eq!(route.route(), expected, "unexpected redirect route");
    }
}
