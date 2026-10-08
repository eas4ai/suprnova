//! What a page render tells the recorder: Laravel's `Collector` and the
//! render half of its `RequestRecorder`.
//!
//! [`DevToolsMiddleware`](super::DevToolsMiddleware) scopes a [`Recorder`]
//! around the rest of the chain. When an [`InertiaResponse`] renders inside
//! it, the render hands over the component, where it was rendered, its page
//! file and the page object, and the middleware reads them back after the
//! response is built. Outside the middleware there is no recorder and the
//! render does no extra work.
//!
//! [`InertiaResponse`]: crate::InertiaResponse

use std::sync::{Arc, Mutex};

use serde_json::{Map, Value, json};

use super::source::SourceLocation;

tokio::task_local! {
    static RECORDER: Arc<Recorder>;
}

/// The slot one request's render writes into.
#[derive(Debug, Default)]
pub(crate) struct Recorder {
    payload: Mutex<Option<RenderPayload>>,
}

/// What a render recorded, ready for the entry.
#[derive(Debug, Clone)]
pub(crate) struct RenderPayload {
    /// The component that rendered.
    pub(crate) component: String,
    /// The metadata of every kept prop path.
    pub(crate) props: Map<String, Value>,
    /// The value of every kept prop path, as the client received it.
    pub(crate) prop_values: Map<String, Value>,
    /// Where the page was rendered.
    pub(crate) render_source: Option<SourceLocation>,
    /// The component's page file, when one was found.
    pub(crate) component_path: Option<String>,
    /// The page object the response carried.
    pub(crate) page: Value,
}

impl Recorder {
    /// Keep what a render recorded; a later render of the same request
    /// replaces it, as the page that is sent is the last one built.
    pub(crate) fn page_rendered(&self, payload: RenderPayload) {
        *crate::lock::recover(&self.payload) = Some(payload);
    }

    /// What the request's render recorded, if a page rendered.
    pub(crate) fn take(&self) -> Option<RenderPayload> {
        crate::lock::recover(&self.payload).take()
    }
}

/// Run `fut` with `recorder` as the request's recorder.
pub(crate) async fn scope<F: std::future::Future>(recorder: Arc<Recorder>, fut: F) -> F::Output {
    RECORDER.scope(recorder, fut).await
}

/// The recorder of the request in scope, when DevTools records it.
pub(crate) fn current() -> Option<Arc<Recorder>> {
    RECORDER.try_with(Arc::clone).ok()
}

/// The facts of one render, collected while it resolves, from which the
/// [`RenderPayload`] is built.
#[derive(Debug)]
pub(crate) struct Collector {
    component: String,
    render_source: Option<SourceLocation>,
    component_path: Option<String>,
}

impl Collector {
    /// A collector for the render of `component` at `render_source`.
    pub(crate) fn new(component: &str, render_source: Option<SourceLocation>) -> Self {
        Self {
            component: component.to_string(),
            render_source,
            component_path: None,
        }
    }

    /// The component's page file.
    pub(crate) fn component_path(&mut self, path: Option<String>) {
        self.component_path = path;
    }

    /// The payload for the page object `page`, Laravel's `Collector::build`:
    /// every top-level key of the page's props is listed, and `propValues`
    /// holds the value of each from the page object, which is what the
    /// client received.
    pub(crate) fn build(self, page: Value) -> RenderPayload {
        let mut props = Map::new();
        let mut prop_values = Map::new();
        if let Some(Value::Object(top)) = page.get("props") {
            for (key, value) in top {
                props.insert(key.clone(), json!({"shared": false, "inertiaType": null}));
                prop_values.insert(key.clone(), value.clone());
            }
        }
        RenderPayload {
            component: self.component,
            props,
            prop_values,
            render_source: self.render_source,
            component_path: self.component_path,
            page,
        }
    }
}
