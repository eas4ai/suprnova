//! Settings an application changes at run time through the
//! [`Inertia`](crate::Inertia) facade, the way Laravel's `ResponseFactory`
//! holds them.
//!
//! They live on the active container's [`InertiaRegistry`](crate::InertiaRegistry),
//! beside the shared data, so they follow the same task-local, thread-local,
//! global lookup: a test that sets one under `TestContainer::fake()` cannot
//! change what tests running in parallel render.

use std::sync::{Arc, Mutex};

use super::prop::InertiaRequestExt;
use super::ssr::SsrRequest;

/// The function [`Inertia::transform_component_using`](crate::Inertia::transform_component_using)
/// installs: a new name for a component, or `None` to keep it.
pub(crate) type ComponentTransformer = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// A per-request SSR condition: `true` turns SSR off for that request.
pub(crate) type SsrDisabledWhen = Arc<dyn Fn(&dyn InertiaRequestExt) -> bool + Send + Sync>;

/// The condition [`Inertia::disable_ssr`](crate::Inertia::disable_ssr) or
/// [`Inertia::disable_ssr_if`](crate::Inertia::disable_ssr_if) sets: SSR is
/// off for a request when it holds.
#[derive(Clone)]
pub(crate) enum SsrCondition {
    /// The same answer for every request.
    Always(bool),
    /// Decided per request.
    When(SsrDisabledWhen),
}

/// The function [`Inertia::configure_ssr_request_using`](crate::Inertia::configure_ssr_request_using)
/// installs.
pub(crate) type SsrRequestConfigurator = Arc<dyn Fn(SsrRequest) -> SsrRequest + Send + Sync>;

/// The run-time Inertia settings of one container.
///
/// Each setting is replaced as one value under its lock and only ever read
/// by cloning it out, so a poisoned lock holds a whole value and is
/// recovered (`lock::recover`) rather than failing every later render.
#[derive(Default)]
pub(crate) struct InertiaRuntime {
    component_transformer: Mutex<Option<ComponentTransformer>>,
    ssr_condition: Mutex<Option<SsrCondition>>,
    ssr_excluded: Mutex<Vec<String>>,
    ssr_request_configurator: Mutex<Option<SsrRequestConfigurator>>,
}

impl InertiaRuntime {
    /// Install the component transformer, replacing any earlier one.
    pub(crate) fn set_component_transformer(&self, transformer: ComponentTransformer) {
        *crate::lock::recover(&self.component_transformer) = Some(transformer);
    }

    /// The name `component` renders under: the transformer's answer, or the
    /// name itself when there is no transformer or it answers `None`, as
    /// Laravel's `transformComponent` keeps it.
    pub(crate) fn transform_component(&self, component: String) -> String {
        let transformer = crate::lock::recover(&self.component_transformer).clone();
        match transformer.and_then(|transform| transform(&component)) {
            Some(renamed) => renamed,
            None => component,
        }
    }

    /// Set the SSR condition, replacing any earlier one.
    pub(crate) fn set_ssr_condition(&self, condition: SsrCondition) {
        *crate::lock::recover(&self.ssr_condition) = Some(condition);
    }

    /// Whether SSR runs for `request`, Laravel's `HttpGateway::ssrIsEnabled`
    /// minus the path check: the condition when one is set, which can turn
    /// SSR on as well as off, else the configuration.
    pub(crate) fn ssr_enabled_for(
        &self,
        configured: bool,
        request: &dyn InertiaRequestExt,
    ) -> bool {
        let condition = crate::lock::recover(&self.ssr_condition).clone();
        match condition {
            Some(SsrCondition::Always(disabled)) => !disabled,
            Some(SsrCondition::When(disabled)) => !disabled(request),
            None => configured,
        }
    }

    /// Add SSR exclusion patterns to the ones already set, as Laravel's
    /// `except` merges them.
    pub(crate) fn add_ssr_exclusions(&self, patterns: impl IntoIterator<Item = String>) {
        crate::lock::recover(&self.ssr_excluded).extend(patterns);
    }

    /// The SSR exclusion patterns set at run time.
    pub(crate) fn ssr_exclusions(&self) -> Vec<String> {
        crate::lock::recover(&self.ssr_excluded).clone()
    }

    /// Install the SSR request configurator, replacing any earlier one.
    pub(crate) fn set_ssr_request_configurator(&self, configurator: SsrRequestConfigurator) {
        *crate::lock::recover(&self.ssr_request_configurator) = Some(configurator);
    }

    /// `request` as the configurator leaves it, or unchanged without one.
    pub(crate) fn configure_ssr_request(&self, request: SsrRequest) -> SsrRequest {
        let configurator = crate::lock::recover(&self.ssr_request_configurator).clone();
        match configurator {
            Some(configure) => configure(request),
            None => request,
        }
    }
}
