//! Settings an application changes at run time through the
//! [`Inertia`](crate::Inertia) facade, the way Laravel's `ResponseFactory`
//! holds them.
//!
//! They live on the active container's [`InertiaRegistry`](crate::InertiaRegistry),
//! beside the shared data, so they follow the same task-local, thread-local,
//! global lookup: a test that sets one under `TestContainer::fake()` cannot
//! change what tests running in parallel render.

use std::sync::{Arc, Mutex};

/// The function [`Inertia::transform_component_using`](crate::Inertia::transform_component_using)
/// installs: a new name for a component, or `None` to keep it.
pub(crate) type ComponentTransformer = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// The run-time Inertia settings of one container.
///
/// Each setting is replaced as one value under its lock and only ever read
/// by cloning it out, so a poisoned lock holds a whole value and is
/// recovered (`lock::recover`) rather than failing every later render.
#[derive(Default)]
pub(crate) struct InertiaRuntime {
    component_transformer: Mutex<Option<ComponentTransformer>>,
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
}
