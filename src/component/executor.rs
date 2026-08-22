//! Panic-contained deterministic component lifecycle executor.

use std::fmt;
use std::future::{Future, poll_fn};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::task::Poll;

use crate::action::{ActionError, ActionErrorKind, ActionResult, RawActionArguments};
use crate::canonical::CanonicalValue;
use crate::child::VerifiedChildParametersV1;
use crate::identity::ActionName;
use crate::limits::InputLimits;
use crate::registry::ComponentDescriptor;
use crate::snapshot::state::StateExposure;
use crate::validation::{
    BagPolicy, ErrorBag, ValidationEngine, ValidationEngineError, ValidationPort,
    ValidationRequest, ValidationStatus,
};
use crate::view::IslandRender;

use super::{
    ComponentError, ComponentInstance, HydrationContext, LifecycleError, LifecycleErrorKind,
    LifecyclePhase, LiveFuture, MountContext, RenderContext,
};

/// Complete in-memory output produced before signing or authority publication.
pub struct LifecycleOutput {
    render: IslandRender,
    state: CanonicalValue,
    memo: CanonicalValue,
}

/// Complete pre-signing result of one registered action request.
pub struct ActionExecutionOutput {
    result: ActionResult,
    render: Option<IslandRender>,
    state: CanonicalValue,
    memo: CanonicalValue,
    validation: ErrorBag,
    action_executed: bool,
}

impl ActionExecutionOutput {
    /// Returns the validated semantic action result.
    #[must_use]
    pub const fn result(&self) -> &ActionResult {
        &self.result
    }

    /// Returns fresh island HTML only when the semantic outcome requires rendering.
    #[must_use]
    pub const fn render(&self) -> Option<&IslandRender> {
        self.render.as_ref()
    }

    /// Returns complete successor instanced state.
    #[must_use]
    pub const fn state(&self) -> &CanonicalValue {
        &self.state
    }

    /// Returns complete successor lifecycle memo.
    #[must_use]
    pub const fn memo(&self) -> &CanonicalValue {
        &self.memo
    }

    /// Returns bounded validation issues kept separate from binding failures.
    #[must_use]
    pub const fn validation(&self) -> &ErrorBag {
        &self.validation
    }

    /// Returns whether the registered Rust action body ran.
    #[must_use]
    pub const fn action_executed(&self) -> bool {
        self.action_executed
    }
}

impl fmt::Debug for ActionExecutionOutput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ActionExecutionOutput")
            .field("outcome", self.result.outcome())
            .field("rendered", &self.render.is_some())
            .field("validation_issue_count", &self.validation.len())
            .field("action_executed", &self.action_executed)
            .finish()
    }
}

/// Closed failure category for the action-aware component executor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActionExecutionErrorKind {
    /// Registered action lookup, arguments, authorization, dispatch, or outcome failed.
    Action(ActionErrorKind),
    /// Host-neutral validation orchestration failed.
    Validation,
    /// Component reconstruction, lifecycle, or teardown failed.
    Lifecycle,
}

/// Redacted action-aware component execution failure.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct ActionExecutionError {
    kind: ActionExecutionErrorKind,
    teardown_failed: bool,
}

impl ActionExecutionError {
    fn action(error: ActionError) -> Self {
        Self {
            kind: ActionExecutionErrorKind::Action(error.kind()),
            teardown_failed: false,
        }
    }

    fn validation(_error: ValidationEngineError) -> Self {
        Self {
            kind: ActionExecutionErrorKind::Validation,
            teardown_failed: false,
        }
    }

    fn lifecycle(_error: LifecycleError) -> Self {
        Self {
            kind: ActionExecutionErrorKind::Lifecycle,
            teardown_failed: false,
        }
    }

    fn with_teardown_failure(mut self) -> Self {
        self.teardown_failed = true;
        self
    }

    /// Returns the stable closed failure category.
    #[must_use]
    pub const fn kind(self) -> ActionExecutionErrorKind {
        self.kind
    }

    /// Returns whether component cleanup also failed after the primary error.
    #[must_use]
    pub const fn teardown_failed(self) -> bool {
        self.teardown_failed
    }
}

impl fmt::Display for ActionExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            ActionExecutionErrorKind::Action(_) => "live_action_execution_failure",
            ActionExecutionErrorKind::Validation => "live_action_validation_failure",
            ActionExecutionErrorKind::Lifecycle => "live_action_lifecycle_failure",
        })
    }
}

impl fmt::Debug for ActionExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl std::error::Error for ActionExecutionError {}

impl LifecycleOutput {
    /// Returns the completed island render data.
    #[must_use]
    pub const fn render(&self) -> &IslandRender {
        &self.render
    }

    /// Returns complete instanced component state.
    #[must_use]
    pub const fn state(&self) -> &CanonicalValue {
        &self.state
    }

    /// Returns complete instanced lifecycle memo.
    #[must_use]
    pub const fn memo(&self) -> &CanonicalValue {
        &self.memo
    }

    pub(crate) fn into_parts(self) -> (IslandRender, CanonicalValue, CanonicalValue) {
        (self.render, self.state, self.memo)
    }
}

impl std::fmt::Debug for LifecycleOutput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("<LifecycleOutput:redacted>")
    }
}

/// Stateless executor that owns each component object for exactly one request.
#[derive(Clone, Copy, Debug, Default)]
pub struct ComponentExecutor;

#[derive(Clone, Copy)]
enum RegisteredOperation<'a> {
    None,
    ParamsChanged(&'a VerifiedChildParametersV1),
    LazyComplete,
}

impl ComponentExecutor {
    /// Creates a stateless lifecycle executor.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Constructs, renders, dehydrates, and tears down one initial component.
    pub async fn initial_mount(
        &self,
        descriptor: &ComponentDescriptor,
        mount: &MountContext<'_>,
    ) -> Result<LifecycleOutput, LifecycleError> {
        let hooks = descriptor.hooks().ok_or_else(|| {
            LifecycleError::new(LifecycleErrorKind::HooksUnavailable, LifecyclePhase::Mount)
        })?;
        let instance =
            catch_future(|| hooks.factory().mount(mount), LifecyclePhase::Mount)?.await?;
        self.execute_owned(
            descriptor,
            instance,
            mount.render(),
            false,
            RegisteredOperation::None,
        )
        .await
    }

    /// Reconstructs a new object from verified state for one ordinary request.
    pub async fn reconstruct(
        &self,
        descriptor: &ComponentDescriptor,
        hydration: &HydrationContext<'_>,
    ) -> Result<LifecycleOutput, LifecycleError> {
        let hooks = descriptor.hooks().ok_or_else(|| {
            LifecycleError::new(
                LifecycleErrorKind::HooksUnavailable,
                LifecyclePhase::Hydrate,
            )
        })?;
        let instance = catch_future(
            || hooks.factory().hydrate(hydration),
            LifecyclePhase::Hydrate,
        )?
        .await?;
        self.execute_owned(
            descriptor,
            instance,
            hydration.render(),
            true,
            RegisteredOperation::None,
        )
        .await
    }

    /// Reconstructs verified state, authorizes and validates current work, then invokes one
    /// descriptor-owned action before producing the required successor render/state.
    #[allow(
        clippy::too_many_arguments,
        reason = "the action boundary keeps every authority and policy dependency explicit"
    )]
    pub async fn action(
        &self,
        descriptor: &ComponentDescriptor,
        hydration: &HydrationContext<'_>,
        action: &ActionName,
        raw_arguments: RawActionArguments,
        limits: &InputLimits,
        validation_engine: &ValidationEngine,
        validation_port: &dyn ValidationPort,
        bag_policy: BagPolicy,
    ) -> Result<ActionExecutionOutput, ActionExecutionError> {
        let metadata = descriptor
            .actions()
            .metadata(action)
            .map_err(ActionExecutionError::action)?
            .clone();
        let arguments = descriptor
            .actions()
            .prepare(action, raw_arguments, limits)
            .map_err(ActionExecutionError::action)?;
        let hooks = descriptor.hooks().ok_or_else(|| {
            ActionExecutionError::lifecycle(LifecycleError::new(
                LifecycleErrorKind::HooksUnavailable,
                LifecyclePhase::Hydrate,
            ))
        })?;
        let instance = catch_future(
            || hooks.factory().hydrate(hydration),
            LifecyclePhase::Hydrate,
        )
        .map_err(ActionExecutionError::lifecycle)?
        .await
        .map_err(ActionExecutionError::lifecycle)?;
        self.execute_action_owned(
            descriptor,
            instance,
            hydration,
            action,
            &metadata,
            &arguments,
            validation_engine,
            validation_port,
            bag_policy,
        )
        .await
    }

    /// Reconstructs a child and applies one registered verified parameter update.
    ///
    /// A raw canonical browser map cannot substitute for the separately verified
    /// child capability:
    ///
    /// ```compile_fail
    /// use suprnova_live::canonical::CanonicalValue;
    /// use suprnova_live::component::{ComponentExecutor, HydrationContext};
    /// use suprnova_live::registry::ComponentDescriptor;
    ///
    /// fn raw_parameters_are_not_authority(
    ///     executor: &ComponentExecutor,
    ///     descriptor: &ComponentDescriptor,
    ///     hydration: &HydrationContext<'_>,
    ///     raw: &CanonicalValue,
    /// ) {
    ///     let _ = executor.params_changed(descriptor, hydration, raw);
    /// }
    /// ```
    pub async fn params_changed<'a>(
        &self,
        descriptor: &ComponentDescriptor,
        hydration: &HydrationContext<'a>,
        parameters: &'a VerifiedChildParametersV1,
    ) -> Result<LifecycleOutput, LifecycleError> {
        if !descriptor.supports_params_changed() {
            return Err(LifecycleError::new(
                LifecycleErrorKind::HooksUnavailable,
                LifecyclePhase::ParamsChanged,
            ));
        }
        let hooks = descriptor.hooks().ok_or_else(|| {
            LifecycleError::new(
                LifecycleErrorKind::HooksUnavailable,
                LifecyclePhase::ParamsChanged,
            )
        })?;
        let instance = catch_future(
            || hooks.factory().hydrate(hydration),
            LifecyclePhase::Hydrate,
        )?
        .await?;
        self.execute_owned(
            descriptor,
            instance,
            hydration.render(),
            true,
            RegisteredOperation::ParamsChanged(parameters),
        )
        .await
    }

    /// Reconstructs a child and invokes only its registered lazy completion hook.
    pub async fn lazy_complete(
        &self,
        descriptor: &ComponentDescriptor,
        hydration: &HydrationContext<'_>,
    ) -> Result<LifecycleOutput, LifecycleError> {
        if !descriptor.supports_lazy_complete() {
            return Err(LifecycleError::new(
                LifecycleErrorKind::HooksUnavailable,
                LifecyclePhase::LazyComplete,
            ));
        }
        let hooks = descriptor.hooks().ok_or_else(|| {
            LifecycleError::new(
                LifecycleErrorKind::HooksUnavailable,
                LifecyclePhase::LazyComplete,
            )
        })?;
        let instance = catch_future(
            || hooks.factory().hydrate(hydration),
            LifecyclePhase::Hydrate,
        )?
        .await?;
        self.execute_owned(
            descriptor,
            instance,
            hydration.render(),
            true,
            RegisteredOperation::LazyComplete,
        )
        .await
    }

    async fn execute_owned<'a>(
        &self,
        descriptor: &ComponentDescriptor,
        mut instance: Box<dyn ComponentInstance>,
        context: &RenderContext<'a>,
        hydrated: bool,
        operation: RegisteredOperation<'a>,
    ) -> Result<LifecycleOutput, LifecycleError> {
        let identity_phase = if hydrated {
            LifecyclePhase::Hydrate
        } else {
            LifecyclePhase::Mount
        };
        let contract_matches = catch_value(
            || instance.metadata().contract_digest() == descriptor.contract_digest(),
            identity_phase,
        );
        let primary = match contract_matches {
            Err(error) => Err(error),
            Ok(false) => Err(LifecycleError::new(
                LifecycleErrorKind::ContractMismatch,
                identity_phase,
            )),
            Ok(true) => {
                self.run_pipeline(instance.as_mut(), context, hydrated, operation)
                    .await
            }
        };
        let teardown = match catch_future(|| instance.teardown(), LifecyclePhase::Teardown) {
            Ok(future) => future.await,
            Err(error) => Err(error),
        };
        let dropped = catch_unwind(AssertUnwindSafe(|| drop(instance))).map_err(|_| {
            LifecycleError::new(LifecycleErrorKind::Panicked, LifecyclePhase::Teardown)
        });
        match (primary, teardown, dropped) {
            (Ok(output), Ok(()), Ok(())) => Ok(output),
            (Ok(_), Err(error), Ok(())) | (Ok(_), Ok(()), Err(error)) => Err(error),
            (Ok(_), Err(error), Err(_)) => Err(error.with_teardown_failure()),
            (Err(error), Ok(()), Ok(())) => Err(error),
            (Err(error), Err(_), _) | (Err(error), Ok(()), Err(_)) => {
                Err(error.with_teardown_failure())
            }
        }
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "private orchestration mirrors the explicit public authority boundary"
    )]
    async fn execute_action_owned<'a>(
        &self,
        descriptor: &ComponentDescriptor,
        mut instance: Box<dyn ComponentInstance>,
        hydration: &HydrationContext<'a>,
        action: &ActionName,
        metadata: &crate::metadata::ActionMetadata,
        arguments: &crate::action::PreparedActionArguments,
        validation_engine: &ValidationEngine,
        validation_port: &dyn ValidationPort,
        bag_policy: BagPolicy,
    ) -> Result<ActionExecutionOutput, ActionExecutionError> {
        let contract_matches = catch_value(
            || instance.metadata().contract_digest() == descriptor.contract_digest(),
            LifecyclePhase::Hydrate,
        )
        .map_err(ActionExecutionError::lifecycle);
        let primary = match contract_matches {
            Err(error) => Err(error),
            Ok(false) => Err(ActionExecutionError::lifecycle(LifecycleError::new(
                LifecycleErrorKind::ContractMismatch,
                LifecyclePhase::Hydrate,
            ))),
            Ok(true) => {
                self.run_action_pipeline(
                    descriptor,
                    instance.as_mut(),
                    hydration,
                    action,
                    metadata,
                    arguments,
                    validation_engine,
                    validation_port,
                    bag_policy,
                )
                .await
            }
        };
        let teardown = match catch_future(|| instance.teardown(), LifecyclePhase::Teardown) {
            Ok(future) => future.await.map_err(ActionExecutionError::lifecycle),
            Err(error) => Err(ActionExecutionError::lifecycle(error)),
        };
        let dropped = catch_unwind(AssertUnwindSafe(|| drop(instance))).map_err(|_| {
            ActionExecutionError::lifecycle(LifecycleError::new(
                LifecycleErrorKind::Panicked,
                LifecyclePhase::Teardown,
            ))
        });
        match (primary, teardown, dropped) {
            (Ok(output), Ok(()), Ok(())) => Ok(output),
            (Ok(_), Err(error), Ok(())) | (Ok(_), Ok(()), Err(error)) => Err(error),
            (Ok(_), Err(error), Err(_)) => Err(error.with_teardown_failure()),
            (Err(error), Ok(()), Ok(())) => Err(error),
            (Err(error), Err(_), _) | (Err(error), Ok(()), Err(_)) => {
                Err(error.with_teardown_failure())
            }
        }
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the ordered action pipeline exposes all policy boundaries explicitly"
    )]
    async fn run_action_pipeline<'a>(
        &self,
        descriptor: &ComponentDescriptor,
        instance: &mut dyn ComponentInstance,
        hydration: &HydrationContext<'a>,
        action: &ActionName,
        metadata: &crate::metadata::ActionMetadata,
        arguments: &crate::action::PreparedActionArguments,
        validation_engine: &ValidationEngine,
        validation_port: &dyn ValidationPort,
        bag_policy: BagPolicy,
    ) -> Result<ActionExecutionOutput, ActionExecutionError> {
        let context = hydration.render();
        catch_future(|| instance.hydrated(context), LifecyclePhase::Hydrate)
            .map_err(ActionExecutionError::lifecycle)?
            .await
            .map_err(ActionExecutionError::lifecycle)?;

        // The capability is minted only after verified reconstruction and `hydrated` complete.
        let authorization = descriptor
            .actions()
            .authorize(
                descriptor.metadata().identity(),
                context.request().capabilities(),
                action,
            )
            .await
            .map_err(ActionExecutionError::action)?;

        let mut validation = ErrorBag::default();
        let request = ValidationRequest::new(
            metadata.validation().clone(),
            hydration.state(),
            arguments.canonical(),
        )
        .with_action(action);
        let status = validation_engine
            .validate(validation_port, request, &mut validation, bag_policy)
            .await
            .map_err(ActionExecutionError::validation)?;
        let (result, action_executed) = if status == ValidationStatus::Invalid {
            (ActionResult::render(), false)
        } else {
            let target: &mut dyn crate::action::ActionTarget = instance;
            let result = descriptor
                .actions()
                .dispatch_prepared(action, target, &authorization, arguments)
                .await
                .map_err(ActionExecutionError::action)?;
            let result = ActionResult::new(
                result.outcome().clone(),
                result.metadata().clone(),
                descriptor,
            )
            .map_err(|_| {
                ActionExecutionError::action(ActionError::new(ActionErrorKind::InvalidOutcome))
            })?;
            (result, true)
        };

        let render = if result.outcome().requires_render() {
            catch_future(|| instance.rendering(context), LifecyclePhase::Rendering)
                .map_err(ActionExecutionError::lifecycle)?
                .await
                .map_err(ActionExecutionError::lifecycle)?;
            let render = catch_future(|| instance.render(context), LifecyclePhase::Render)
                .map_err(ActionExecutionError::lifecycle)?
                .await
                .map_err(ActionExecutionError::lifecycle)?;
            catch_future(|| instance.rendered(context), LifecyclePhase::Rendered)
                .map_err(ActionExecutionError::lifecycle)?
                .await
                .map_err(ActionExecutionError::lifecycle)?;
            Some(render)
        } else {
            None
        };
        catch_future(
            || instance.dehydrating(context),
            LifecyclePhase::Dehydrating,
        )
        .map_err(ActionExecutionError::lifecycle)?
        .await
        .map_err(ActionExecutionError::lifecycle)?;
        let state = catch_sync(
            || instance.dehydrate(StateExposure::Instanced),
            LifecyclePhase::Dehydrate,
        )
        .map_err(ActionExecutionError::lifecycle)?;
        let memo = catch_sync(|| instance.dehydrate_memo(), LifecyclePhase::Dehydrate)
            .map_err(ActionExecutionError::lifecycle)?;
        Ok(ActionExecutionOutput {
            result,
            render,
            state,
            memo,
            validation,
            action_executed,
        })
    }

    async fn run_pipeline<'a>(
        &self,
        instance: &mut dyn ComponentInstance,
        context: &RenderContext<'a>,
        hydrated: bool,
        operation: RegisteredOperation<'a>,
    ) -> Result<LifecycleOutput, LifecycleError> {
        if hydrated {
            catch_future(|| instance.hydrated(context), LifecyclePhase::Hydrate)?.await?;
        }
        match operation {
            RegisteredOperation::None => {}
            RegisteredOperation::ParamsChanged(parameters) => {
                catch_future(
                    || instance.params_changed(context, parameters),
                    LifecyclePhase::ParamsChanged,
                )?
                .await?;
            }
            RegisteredOperation::LazyComplete => {
                catch_future(
                    || instance.lazy_complete(context),
                    LifecyclePhase::LazyComplete,
                )?
                .await?;
            }
        }
        catch_future(|| instance.rendering(context), LifecyclePhase::Rendering)?.await?;
        let render = catch_future(|| instance.render(context), LifecyclePhase::Render)?.await?;
        catch_future(|| instance.rendered(context), LifecyclePhase::Rendered)?.await?;
        catch_future(
            || instance.dehydrating(context),
            LifecyclePhase::Dehydrating,
        )?
        .await?;
        let state = catch_sync(
            || instance.dehydrate(StateExposure::Instanced),
            LifecyclePhase::Dehydrate,
        )?;
        let memo = catch_sync(|| instance.dehydrate_memo(), LifecyclePhase::Dehydrate)?;
        Ok(LifecycleOutput {
            render,
            state,
            memo,
        })
    }
}

fn catch_future<'a, T>(
    operation: impl FnOnce() -> LiveFuture<'a, Result<T, ComponentError>>,
    phase: LifecyclePhase,
) -> Result<impl Future<Output = Result<T, LifecycleError>> + Send + 'a, LifecycleError>
where
    T: Send + 'a,
{
    let future = catch_unwind(AssertUnwindSafe(operation))
        .map_err(|_| LifecycleError::new(LifecycleErrorKind::Panicked, phase))?;
    Ok(poll_future(future, phase))
}

async fn poll_future<T>(
    mut future: LiveFuture<'_, Result<T, ComponentError>>,
    phase: LifecyclePhase,
) -> Result<T, LifecycleError> {
    let result =
        poll_fn(
            |context| match catch_unwind(AssertUnwindSafe(|| future.as_mut().poll(context))) {
                Ok(Poll::Ready(Ok(value))) => Poll::Ready(Ok(value)),
                Ok(Poll::Ready(Err(_))) => Poll::Ready(Err(LifecycleError::new(
                    LifecycleErrorKind::ComponentFailure,
                    phase,
                ))),
                Ok(Poll::Pending) => Poll::Pending,
                Err(_) => Poll::Ready(Err(LifecycleError::new(
                    LifecycleErrorKind::Panicked,
                    phase,
                ))),
            },
        )
        .await;
    let dropped = catch_unwind(AssertUnwindSafe(|| drop(future)));
    match (result, dropped) {
        (Ok(value), Ok(())) => Ok(value),
        (Ok(_), Err(_)) => Err(LifecycleError::new(LifecycleErrorKind::Panicked, phase)),
        (Err(error), _) => Err(error),
    }
}

fn catch_value<T>(
    operation: impl FnOnce() -> T,
    phase: LifecyclePhase,
) -> Result<T, LifecycleError> {
    catch_unwind(AssertUnwindSafe(operation))
        .map_err(|_| LifecycleError::new(LifecycleErrorKind::Panicked, phase))
}

fn catch_sync<T>(
    operation: impl FnOnce() -> Result<T, ComponentError>,
    phase: LifecyclePhase,
) -> Result<T, LifecycleError> {
    match catch_unwind(AssertUnwindSafe(operation)) {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(_)) => Err(LifecycleError::new(
            LifecycleErrorKind::ComponentFailure,
            phase,
        )),
        Err(_) => Err(LifecycleError::new(LifecycleErrorKind::Panicked, phase)),
    }
}
