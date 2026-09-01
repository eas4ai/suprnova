//! Closed table lookup, current authorization, and panic-contained dispatch.

use std::any::Any;
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::future::{Future, poll_fn};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::task::Poll;

use crate::component::ComponentError;
use crate::host::HostCapabilities;
use crate::identity::{ActionName, ComponentName};
use crate::limits::InputLimits;
use crate::metadata::ActionMetadata;

use super::{ActionOutcome, ActionResult, PreparedActionArguments, RawActionArguments};

/// Bounded boxed future used by authorization and erased action dispatchers.
pub type ActionFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Whether an action is public or requires a fresh host authorization decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthorizationRequirement {
    /// No protected read/effect is reachable through this action.
    Public,
    /// The current request principal and resource policy must allow the action.
    Current,
}

/// Whether an action participates in the host transaction coordinated by Task 11.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransactionPolicy {
    /// The action does not request a host transaction.
    None,
    /// The action requires a host transaction around its durable work.
    Required,
}

/// Closed current-authorization decision returned by the host adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthorizationDecision {
    /// Current request authority permits the registered action.
    Allow,
    /// Current request authority denies the registered action.
    Deny,
}

/// Safe registered identities supplied to the host authorization provider.
#[derive(Clone, Copy)]
pub struct ActionAuthorizationRequest<'a> {
    component: &'a ComponentName,
    action: &'a ActionName,
}

impl<'a> ActionAuthorizationRequest<'a> {
    pub(crate) const fn new(component: &'a ComponentName, action: &'a ActionName) -> Self {
        Self { component, action }
    }

    /// Returns the registered component identity.
    #[must_use]
    pub const fn component(&self) -> &ComponentName {
        self.component
    }

    /// Returns the registered action identity.
    #[must_use]
    pub const fn action(&self) -> &ActionName {
        self.action
    }
}

impl fmt::Debug for ActionAuthorizationRequest<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ActionAuthorizationRequest")
            .field("component", &self.component.as_str())
            .field("action", &self.action.as_str())
            .finish()
    }
}

/// Host-owned current authorization service.
pub trait ActionAuthorizationPort: Send + Sync {
    /// Rechecks the current principal/resource policy for one registered action.
    fn authorize<'a>(
        &'a self,
        request: ActionAuthorizationRequest<'a>,
    ) -> ActionFuture<'a, Result<AuthorizationDecision, ActionError>>;
}

/// Opaque proof that this invocation satisfied its registered authorization policy.
#[derive(Clone)]
pub struct AuthorizedAction {
    component: ComponentName,
    action: ActionName,
    current: bool,
}

impl AuthorizedAction {
    fn new(component: &ComponentName, action: &ActionName, current: bool) -> Self {
        Self {
            component: component.clone(),
            action: action.clone(),
            current,
        }
    }

    /// Returns the registered component identity covered by the proof.
    #[must_use]
    pub const fn component(&self) -> &ComponentName {
        &self.component
    }

    /// Returns the registered action identity covered by the proof.
    #[must_use]
    pub const fn action(&self) -> &ActionName {
        &self.action
    }

    /// Returns whether the host performed a fresh current authorization check.
    #[must_use]
    pub const fn is_current(&self) -> bool {
        self.current
    }
}

impl fmt::Debug for AuthorizedAction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<AuthorizedAction>")
    }
}

/// Type-erased action receiver; concrete targets are selected only by generated code.
pub trait ActionTarget: Any + Send {
    /// Returns the target for generated exact-type downcasting.
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl<T: Any + Send> ActionTarget for T {
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Generated noncapturing erased dispatcher for one exact concrete action method.
///
/// Generated action bodies must tolerate re-invocation before commit. A host
/// transaction may roll back its revision claim and retry the method; Live
/// guarantees at most one accepted committed outcome, not one method call or
/// exactly-once effects in external services.
pub type ActionDispatchFn = for<'a> fn(
    &'a mut dyn ActionTarget,
    &'a AuthorizedAction,
    &'a PreparedActionArguments,
) -> ActionFuture<'a, Result<ActionResult, ActionError>>;

/// One registered action contract and its exact generated dispatcher.
#[derive(Clone)]
pub struct ActionEntry {
    metadata: ActionMetadata,
    dispatcher: ActionDispatchFn,
}

impl ActionEntry {
    /// Binds canonical action metadata to one generated exact-method dispatcher.
    #[must_use]
    pub const fn new(metadata: ActionMetadata, dispatcher: ActionDispatchFn) -> Self {
        Self {
            metadata,
            dispatcher,
        }
    }

    /// Returns the complete registered action metadata.
    #[must_use]
    pub const fn metadata(&self) -> &ActionMetadata {
        &self.metadata
    }
}

impl fmt::Debug for ActionEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ActionEntry")
            .field("name", &self.metadata.name().as_str())
            .finish_non_exhaustive()
    }
}

/// Immutable descriptor-owned action table keyed only by validated public names.
#[derive(Clone, Debug, Default)]
pub struct ActionTable {
    entries: BTreeMap<ActionName, ActionEntry>,
}

impl ActionTable {
    /// Builds a closed table and rejects duplicate public names.
    pub fn new(entries: Vec<ActionEntry>) -> Result<Self, ActionError> {
        let mut indexed = BTreeMap::new();
        for entry in entries {
            if indexed
                .insert(entry.metadata.name().clone(), entry)
                .is_some()
            {
                return Err(ActionError::new(ActionErrorKind::DuplicateAction));
            }
        }
        Ok(Self { entries: indexed })
    }

    /// Returns the number of registered action dispatchers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns whether no action dispatcher is registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(crate) fn matches_metadata(&self, metadata: &[ActionMetadata]) -> bool {
        self.entries.len() == metadata.len()
            && metadata.iter().all(|registered| {
                self.entries
                    .get(registered.name())
                    .is_some_and(|entry| entry.metadata == *registered)
            })
    }

    /// Validates and converts raw arguments without invoking authorization or component code.
    pub fn prepare(
        &self,
        action: &ActionName,
        raw: RawActionArguments,
        limits: &InputLimits,
    ) -> Result<PreparedActionArguments, ActionError> {
        let entry = self
            .entries
            .get(action)
            .ok_or_else(|| ActionError::new(ActionErrorKind::UnknownAction))?;
        PreparedActionArguments::prepare(entry.metadata.arguments(), raw, limits)
    }

    /// Returns metadata only for an explicitly registered action entry.
    pub fn metadata(&self, action: &ActionName) -> Result<&ActionMetadata, ActionError> {
        self.entries
            .get(action)
            .map(ActionEntry::metadata)
            .ok_or_else(|| ActionError::new(ActionErrorKind::UnknownAction))
    }

    /// Produces invocation authority for one registered action.
    pub async fn authorize(
        &self,
        component: &ComponentName,
        capabilities: &HostCapabilities,
        action: &ActionName,
    ) -> Result<AuthorizedAction, ActionError> {
        let entry = self
            .entries
            .get(action)
            .ok_or_else(|| ActionError::new(ActionErrorKind::UnknownAction))?;
        match entry.metadata.authorization() {
            AuthorizationRequirement::Public => Ok(AuthorizedAction::new(component, action, false)),
            AuthorizationRequirement::Current => {
                let port = capabilities
                    .action_authorization()
                    .ok_or_else(|| ActionError::new(ActionErrorKind::AuthorizationUnavailable))?;
                let request = ActionAuthorizationRequest::new(component, action);
                match catch_action_future(|| port.authorize(request))?.await? {
                    AuthorizationDecision::Allow => {
                        Ok(AuthorizedAction::new(component, action, true))
                    }
                    AuthorizationDecision::Deny => {
                        Err(ActionError::new(ActionErrorKind::AuthorizationDenied))
                    }
                }
            }
        }
    }

    /// Dispatches an exact entry using separately prepared arguments and authorization proof.
    pub async fn dispatch_prepared(
        &self,
        action: &ActionName,
        target: &mut dyn ActionTarget,
        authorization: &AuthorizedAction,
        arguments: &PreparedActionArguments,
    ) -> Result<ActionResult, ActionError> {
        let entry = self
            .entries
            .get(action)
            .ok_or_else(|| ActionError::new(ActionErrorKind::UnknownAction))?;
        if authorization.action() != action {
            return Err(ActionError::new(ActionErrorKind::AuthorizationDenied));
        }
        catch_action_future(|| (entry.dispatcher)(target, authorization, arguments))?.await
    }

    /// Selects one registered entry, authorizes current work, and panic-contains dispatch.
    pub async fn invoke(
        &self,
        component: &ComponentName,
        capabilities: &HostCapabilities,
        action: &ActionName,
        target: &mut dyn ActionTarget,
        raw: RawActionArguments,
        limits: &InputLimits,
    ) -> Result<ActionResult, ActionError> {
        let arguments = self.prepare(action, raw, limits)?;
        let authorization = self.authorize(component, capabilities, action).await?;
        self.dispatch_prepared(action, target, &authorization, &arguments)
            .await
    }
}

fn catch_action_future<'a, T>(
    operation: impl FnOnce() -> ActionFuture<'a, Result<T, ActionError>>,
) -> Result<impl Future<Output = Result<T, ActionError>> + Send + 'a, ActionError>
where
    T: Send + 'a,
{
    let future = catch_unwind(AssertUnwindSafe(operation))
        .map_err(|_| ActionError::new(ActionErrorKind::Panicked))?;
    Ok(poll_action_future(future))
}

async fn poll_action_future<T>(
    mut future: ActionFuture<'_, Result<T, ActionError>>,
) -> Result<T, ActionError> {
    let result =
        poll_fn(
            |context| match catch_unwind(AssertUnwindSafe(|| future.as_mut().poll(context))) {
                Ok(Poll::Ready(result)) => Poll::Ready(result),
                Ok(Poll::Pending) => Poll::Pending,
                Err(_) => Poll::Ready(Err(ActionError::new(ActionErrorKind::Panicked))),
            },
        )
        .await;
    if catch_unwind(AssertUnwindSafe(|| drop(future))).is_err() {
        return Err(ActionError::new(ActionErrorKind::Panicked));
    }
    result
}

/// Converts supported authored action return values into the closed semantic contract.
pub trait IntoActionResult {
    /// Converts authored return state without exposing application failures.
    fn into_action_result(self) -> Result<ActionResult, ActionError>;
}

impl IntoActionResult for () {
    fn into_action_result(self) -> Result<ActionResult, ActionError> {
        Ok(ActionResult::render())
    }
}

impl IntoActionResult for ActionResult {
    fn into_action_result(self) -> Result<ActionResult, ActionError> {
        Ok(self)
    }
}

impl IntoActionResult for ActionOutcome {
    fn into_action_result(self) -> Result<ActionResult, ActionError> {
        Ok(ActionResult::from_outcome(self))
    }
}

impl<T: IntoActionResult> IntoActionResult for Result<T, ComponentError> {
    fn into_action_result(self) -> Result<ActionResult, ActionError> {
        self.map_err(|_| ActionError::new(ActionErrorKind::ComponentFailure))?
            .into_action_result()
    }
}

/// Closed action dispatch failure category.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActionErrorKind {
    /// No descriptor-owned entry matched the browser-selected name.
    UnknownAction,
    /// A descriptor attempted to register the same public action twice.
    DuplicateAction,
    /// Raw arguments were malformed, unknown, missing, null, or out of bounds.
    InvalidArguments,
    /// Current authorization was required but no host capability was installed.
    AuthorizationUnavailable,
    /// Current authorization denied the action.
    AuthorizationDenied,
    /// Generated dispatch did not match the registered concrete component type.
    DispatcherContract,
    /// Component code returned a closed application failure.
    ComponentFailure,
    /// Component or generated dispatch code panicked.
    Panicked,
    /// The semantic result violated its closed outcome contract.
    InvalidOutcome,
}

/// Redacted registered-action failure.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct ActionError {
    kind: ActionErrorKind,
}

impl ActionError {
    pub(crate) const fn new(kind: ActionErrorKind) -> Self {
        Self { kind }
    }

    pub(crate) const fn invalid_arguments() -> Self {
        Self::new(ActionErrorKind::InvalidArguments)
    }

    /// Creates the generated exact-type mismatch failure.
    #[must_use]
    pub const fn dispatcher_contract() -> Self {
        Self::new(ActionErrorKind::DispatcherContract)
    }

    /// Returns the stable closed failure category.
    #[must_use]
    pub const fn kind(self) -> ActionErrorKind {
        self.kind
    }
}

impl fmt::Display for ActionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            ActionErrorKind::UnknownAction => "unknown_live_action",
            ActionErrorKind::DuplicateAction => "duplicate_live_action",
            ActionErrorKind::InvalidArguments => "invalid_live_action_arguments",
            ActionErrorKind::AuthorizationUnavailable => "live_action_authorization_unavailable",
            ActionErrorKind::AuthorizationDenied => "live_action_authorization_denied",
            ActionErrorKind::DispatcherContract => "live_action_dispatcher_contract",
            ActionErrorKind::ComponentFailure => "live_action_component_failure",
            ActionErrorKind::Panicked => "live_action_panicked",
            ActionErrorKind::InvalidOutcome => "invalid_live_action_outcome",
        })
    }
}

impl fmt::Debug for ActionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl Error for ActionError {}
