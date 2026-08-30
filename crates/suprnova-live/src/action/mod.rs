//! Closed registered action dispatch, authorization, arguments, and outcomes.

mod arguments;
mod dispatch;
mod emission;
mod outcome;

pub use arguments::{
    ActionArgumentField, ActionArgumentSchema, PreparedActionArguments, RawActionArguments,
};
pub use dispatch::{
    ActionAuthorizationPort, ActionAuthorizationRequest, ActionDispatchFn, ActionEntry,
    ActionError, ActionErrorKind, ActionFuture, ActionTable, ActionTarget, AuthorizationDecision,
    AuthorizationRequirement, AuthorizedAction, IntoActionResult, TransactionPolicy,
};
pub use emission::{EmissionKind, LiveEffectPayload, LiveEventPayload, RegisteredEmission};
pub use outcome::{
    ActionOutcome, ActionResult, FlashIntent, OutcomeError, OutcomeErrorKind, OutcomeMetadata,
    RouteIntent, UrlIntent,
};
