//! Typed lazy-island scheduling without streamed or unsolicited HTML authority.

use std::error::Error;
use std::fmt;

const MAX_PRESENTATION_BYTES: usize = 1_024;

/// Developer policy for normal lazy execution.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LazyPolicy {
    /// Render semantic SSR state and request completion through normal Live work.
    Deferred,
    /// Complete during the initial server render.
    Eager,
}

/// Host/test execution profile that can force deterministic eager completion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LazyExecutionMode {
    /// Browser-capable request honoring the developer policy.
    Browser,
    /// Test harness forces completion without browser scheduling.
    TestEager,
    /// Non-browser rendering forces completion synchronously.
    NonBrowserEager,
}

/// Bounded semantic text accompanying server-rendered pending content.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LazyPresentation(String);

impl LazyPresentation {
    /// Creates meaningful non-empty pending presentation text.
    pub fn new(value: &str) -> Result<Self, LazyError> {
        let valid = !value.trim().is_empty()
            && value.len() <= MAX_PRESENTATION_BYTES
            && value.chars().all(|character| !character.is_control());
        if !valid {
            return Err(LazyError);
        }
        Ok(Self(value.to_owned()))
    }

    /// Returns the server-rendered semantic pending text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.0
    }
}

/// Closed semantic presentation state for one lazy island.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LazyPresentationState {
    /// Meaningful initial SSR placeholder.
    Placeholder,
    /// A registered completion request is currently pending.
    Loading,
    /// Completion produced no result content.
    Empty,
    /// Child-local completion failed and may be retried or refreshed.
    Error,
    /// Completion succeeded and ordinary Live rendering may proceed.
    Success,
}

/// Closed operation scheduled through ordinary child lifecycle/revision work.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LazyOperation {
    /// Invoke only the registered `lazy_complete` lifecycle hook.
    LazyComplete,
}

impl LazyOperation {
    /// Returns the stable internal operation name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LazyComplete => "lazy_complete",
        }
    }
}

/// Deferred server operation; deliberately contains no HTML or transport patch.
#[derive(Clone)]
pub struct LazyCompletionRequest {
    presentation: LazyPresentation,
    operation: LazyOperation,
}

impl LazyCompletionRequest {
    /// Returns the semantic SSR presentation already visible to the user.
    #[must_use]
    pub const fn presentation(&self) -> &LazyPresentation {
        &self.presentation
    }

    /// Returns the only registered server operation this request can schedule.
    #[must_use]
    pub const fn operation(&self) -> LazyOperation {
        self.operation
    }

    /// Returns the semantic state already rendered before scheduling.
    #[must_use]
    pub const fn initial_state(&self) -> LazyPresentationState {
        LazyPresentationState::Placeholder
    }

    /// Returns the semantic state used while ordinary Live work is pending.
    #[must_use]
    pub const fn loading_state(&self) -> LazyPresentationState {
        LazyPresentationState::Loading
    }
}

impl fmt::Debug for LazyCompletionRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LazyCompletionRequest")
            .field("presentation", &"<redacted>")
            .field("operation", &self.operation)
            .finish()
    }
}

/// Server-side scheduling decision for one lazy island.
#[derive(Clone, Debug)]
pub enum LazyCompletion {
    /// Complete through the current server render/test loop.
    Eager,
    /// Queue a registered ordinary Live lifecycle operation.
    Deferred(LazyCompletionRequest),
}

/// Typed server completion result with no streamed-HTML authority.
///
/// ```compile_fail
/// use suprnova_live::component::lazy::LazyServerCompletion;
/// let completion = LazyServerCompletion::Render;
/// let _unsolicited_patch = completion.html;
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LazyServerCompletion {
    /// Run the ordinary render/dehydrate/sign response path.
    Render,
    /// Present the registered empty state without a transport fragment.
    Empty,
    /// Recover only this child through its registered error path.
    Failed,
}

impl LazyServerCompletion {
    /// Returns the semantic state associated with the typed completion.
    #[must_use]
    pub const fn presentation_state(self) -> LazyPresentationState {
        match self {
            Self::Render => LazyPresentationState::Success,
            Self::Empty => LazyPresentationState::Empty,
            Self::Failed => LazyPresentationState::Error,
        }
    }
}

/// Validated lazy-island declaration.
#[derive(Clone, Debug)]
pub struct LazyMount {
    policy: LazyPolicy,
    presentation: LazyPresentation,
}

impl LazyMount {
    /// Declares lazy policy and meaningful SSR pending presentation.
    #[must_use]
    pub const fn new(policy: LazyPolicy, presentation: LazyPresentation) -> Self {
        Self {
            policy,
            presentation,
        }
    }

    /// Resolves execution without manufacturing HTML or a transport response.
    #[must_use]
    pub fn schedule(&self, mode: LazyExecutionMode) -> LazyCompletion {
        if self.policy == LazyPolicy::Eager
            || matches!(
                mode,
                LazyExecutionMode::TestEager | LazyExecutionMode::NonBrowserEager
            )
        {
            LazyCompletion::Eager
        } else {
            LazyCompletion::Deferred(LazyCompletionRequest {
                presentation: self.presentation.clone(),
                operation: LazyOperation::LazyComplete,
            })
        }
    }
}

/// Closed validation error for lazy presentation declarations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LazyError;

impl fmt::Display for LazyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("invalid_lazy_presentation")
    }
}

impl Error for LazyError {}
