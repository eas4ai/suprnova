//! Panic-contained deterministic component lifecycle executor.

use std::future::{Future, poll_fn};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::task::Poll;

use crate::canonical::CanonicalValue;
use crate::registry::ComponentDescriptor;
use crate::snapshot::state::StateExposure;
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
    ParamsChanged(&'a CanonicalValue),
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

    /// Reconstructs a child and applies one registered verified parameter update.
    pub async fn params_changed<'a>(
        &self,
        descriptor: &ComponentDescriptor,
        hydration: &HydrationContext<'a>,
        parameters: &'a CanonicalValue,
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
