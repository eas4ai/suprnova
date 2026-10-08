//! What a page render tells the recorder: Laravel's `Collector` and the
//! render half of its `RequestRecorder`.
//!
//! [`DevToolsMiddleware`](super::DevToolsMiddleware) scopes a [`Recorder`]
//! around the rest of the chain. When an [`InertiaResponse`] renders inside
//! it, the render hands over the component, where it was rendered, the
//! metadata of every prop it delivered, and the page object, and the
//! middleware reads them back after the response is built. Outside the
//! middleware there is no recorder and the render does no extra work.
//!
//! [`InertiaResponse`]: crate::InertiaResponse

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::{Map, Value};

use super::classify::PropMeta;
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
    shared_keys: Vec<String>,
    share_sources: HashMap<String, SourceLocation>,
    props: Vec<(String, PropMeta)>,
    component_path: Option<String>,
}

impl Collector {
    /// A collector for the render of `component` at `render_source`.
    pub(crate) fn new(component: &str, render_source: Option<SourceLocation>) -> Self {
        Self {
            component: component.to_string(),
            render_source,
            shared_keys: Vec::new(),
            share_sources: HashMap::new(),
            props: Vec::new(),
            component_path: None,
        }
    }

    /// The top-level keys the shared props supplied.
    pub(crate) fn shared_keys(&mut self, keys: Vec<String>) {
        self.shared_keys = keys;
    }

    /// Where the shared key `key` was shared.
    pub(crate) fn share_source(&mut self, key: &str, source: SourceLocation) {
        let root = key.split('.').next().unwrap_or(key);
        self.share_sources
            .insert(root.to_string(), source.refined_for(root));
    }

    /// Where the hook-shared key `key` was shared: the `InertiaConfig::hooks`
    /// call that installed the hooks, as it is. The keys are named in the
    /// hooks' `share` method, not below that call, so no line under it is
    /// looked for.
    pub(crate) fn hook_share_source(&mut self, key: &str, source: SourceLocation) {
        let root = key.split('.').next().unwrap_or(key);
        self.share_sources.insert(root.to_string(), source);
    }

    /// The metadata of the prop at `path`.
    pub(crate) fn prop(&mut self, path: &str, meta: PropMeta) {
        match self.props.iter_mut().find(|(known, _)| known == path) {
            Some((_, known)) => *known = meta,
            None => self.props.push((path.to_string(), meta)),
        }
    }

    /// Mark the prop at `path` rescued: its deferred resolver failed and
    /// the page left it out, Laravel's `propRescued`.
    pub(crate) fn rescued(&mut self, path: &str) {
        match self.props.iter_mut().find(|(known, _)| known == path) {
            Some((_, meta)) => meta.rescued = true,
            None => self.props.push((
                path.to_string(),
                PropMeta {
                    rescued: true,
                    ..PropMeta::default()
                },
            )),
        }
    }

    /// The component's page file.
    pub(crate) fn component_path(&mut self, path: Option<String>) {
        self.component_path = path;
    }

    /// The payload for the page object `page`, Laravel's `Collector::build`.
    ///
    /// Every prop path the render recorded is listed only when the client
    /// received it (or it was rescued), every top-level key of the page's
    /// props is listed, and a deep path is kept only when it carries
    /// metadata. A render prop gets the line that names it in the render
    /// call. `propValues` holds the value of each kept path from the page
    /// object, which is what the client received.
    pub(crate) fn build(self, page: Value) -> RenderPayload {
        let delivered = page.get("props").cloned().unwrap_or(Value::Null);
        let mut entries: Vec<(String, PropMeta)> = self
            .props
            .into_iter()
            .filter(|(path, meta)| meta.rescued || value_at(&delivered, path).is_some())
            .collect();
        if let Value::Object(top) = &delivered {
            for key in top.keys() {
                if !entries.iter().any(|(path, _)| path == key) {
                    entries.push((key.clone(), PropMeta::default()));
                }
            }
        }
        let render_text = self.render_source.and_then(|source| source.text());
        let mut props = Map::new();
        let mut prop_values = Map::new();
        for (path, meta) in entries {
            let shared = self.shared_keys.contains(&path);
            let share_source = self.share_sources.get(&path).copied();
            if path.contains('.') && !meta.has_metadata(shared, share_source.is_some()) {
                continue;
            }
            let mut object = meta.to_json(shared, share_source.map(SourceLocation::to_json));
            if !shared
                && share_source.is_none()
                && let (Some(source), Some(text)) = (self.render_source, render_text.as_deref())
                && let Some(line) = source.find_key_line_in(text, &path)
                && let Value::Object(map) = &mut object
            {
                let mut at = source.to_json();
                at["line"] = Value::from(line);
                map.insert("renderSource".to_string(), at);
            }
            if let Some(value) = value_at(&delivered, &path) {
                prop_values.insert(path.clone(), value.clone());
            }
            props.insert(path, object);
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

/// The value at the dotted `path` of `value`, Laravel's `Arr::get`.
fn value_at<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    if let Some(found) = value.get(path) {
        return Some(found);
    }
    path.split('.')
        .try_fold(value, |current, segment| match current {
            Value::Object(map) => map.get(segment),
            Value::Array(items) => segment.parse::<usize>().ok().and_then(|i| items.get(i)),
            _ => None,
        })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn indt_deep_paths_without_metadata_are_pruned_and_top_level_keys_kept() {
        let mut collector = Collector::new("Home", None);
        collector.shared_keys(vec!["auth".to_string()]);
        collector.prop(
            "auth.user.permissions",
            PropMeta {
                inertia_type: Some("defer"),
                ..PropMeta::default()
            },
        );
        collector.prop("auth.user.name", PropMeta::default());
        collector.prop("stats", PropMeta::default());
        collector.prop("never", PropMeta::default());
        let payload = collector.build(json!({
            "component": "Home",
            "props": {
                "auth": {"user": {"name": "Ada", "permissions": ["x"]}},
                "stats": {"count": 3},
                "errors": {},
            },
        }));
        let keys: Vec<&String> = payload.props.keys().collect();
        assert!(payload.props.contains_key("auth"));
        assert!(payload.props.contains_key("stats"));
        assert!(payload.props.contains_key("errors"));
        assert!(
            payload.props.contains_key("auth.user.permissions"),
            "{keys:?}"
        );
        assert!(!payload.props.contains_key("auth.user.name"), "{keys:?}");
        assert!(!payload.props.contains_key("never"), "not delivered");
        assert_eq!(payload.props["auth"]["shared"], true);
        assert_eq!(payload.prop_values["auth.user.permissions"], json!(["x"]));
        assert_eq!(payload.prop_values["stats"], json!({"count": 3}));
    }

    #[test]
    fn indt_a_rescued_prop_is_listed_without_a_value() {
        let mut collector = Collector::new("Home", None);
        collector.prop(
            "report",
            PropMeta {
                rescued: true,
                ..PropMeta::default()
            },
        );
        let payload = collector.build(json!({"props": {"errors": {}}}));
        assert_eq!(payload.props["report"]["rescued"], true);
        assert!(!payload.prop_values.contains_key("report"));
    }
}
