//! Focused assertion helpers for application Live tests.

use super::{ActionOutcome, ActionResult};

/// Fluent assertions over one semantic Live action result.
#[derive(Clone, Copy, Debug)]
pub struct ActionAssertion<'result> {
    result: &'result ActionResult,
}

impl<'result> ActionAssertion<'result> {
    /// Starts assertions for one completed semantic action result.
    #[must_use]
    pub const fn new(result: &'result ActionResult) -> Self {
        Self { result }
    }

    /// Asserts that the action requested fresh island rendering.
    pub fn assert_rendered(self) {
        assert!(
            matches!(self.result.outcome(), ActionOutcome::Render),
            "expected Live action to render, got {:?}",
            self.result.outcome()
        );
    }

    /// Asserts that the action completed without fresh island rendering.
    pub fn assert_not_rendered(self) {
        assert!(
            matches!(self.result.outcome(), ActionOutcome::NoRender),
            "expected Live action not to render, got {:?}",
            self.result.outcome()
        );
    }

    /// Asserts that the action requested an ordinary registered-route navigation.
    pub fn assert_redirected(self) {
        assert!(
            self.result.outcome().redirects(),
            "expected Live action redirect, got {:?}",
            self.result.outcome()
        );
    }
}
