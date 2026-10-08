//! What kind of prop each prop is, as the extension shows it: Laravel's
//! `PropClassifier`.
//!
//! Laravel has one class per kind of prop (`AlwaysProp`, `DeferProp`,
//! `OptionalProp`, `MergeProp`, `ScrollProp`, `OnceProp`), and the class
//! picks the pill. A Suprnova [`Prop`] composes those as flags, so the
//! flags are read in that order and the first that applies names the
//! type, which is the class Laravel's builder for the same flags would
//! have made.

use serde_json::{Map, Value};

use super::entry::DEFERRED_HEADER;
use crate::inertia::prop::{InertiaRequestExt, MergeMode, Prop, Visibility, header_is_truthy};

/// The metadata of one prop path.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PropMeta {
    /// `always`, `defer`, `optional`, `merge`, `scroll` or `once`.
    pub(crate) inertia_type: Option<&'static str>,
    /// The group a deferred prop was loaded with.
    pub(crate) defer_group: Option<String>,
    /// The client asked for the prop to be reset (`X-Inertia-Reset`).
    pub(crate) reset: bool,
    /// The prop is resolved once and kept by the client.
    pub(crate) once: bool,
    /// `append` or `prepend`, for a prop that merges.
    pub(crate) merge_direction: Option<&'static str>,
    /// The prop merges deeply, or matches items on a key.
    pub(crate) deep_merge: bool,
    /// The prop's resolver failed and was rescued.
    pub(crate) rescued: bool,
}

/// The facts of the request a classification reads.
#[derive(Debug, Clone, Default)]
pub(crate) struct ClassifyRequest {
    /// The visit loads deferred props (`X-Inertia-Devtools-Deferred`).
    deferred: bool,
    /// The keys of `X-Inertia-Reset`.
    reset: Vec<String>,
    /// The scroll merge intent is `prepend`.
    scroll_prepends: bool,
}

impl ClassifyRequest {
    /// The facts of `request`.
    pub(crate) fn of<R: InertiaRequestExt + ?Sized>(request: &R) -> Self {
        Self {
            deferred: request
                .header(DEFERRED_HEADER)
                .is_some_and(|value| header_is_truthy(value.as_bytes())),
            reset: request
                .header("X-Inertia-Reset")
                .map(|value| {
                    value
                        .split(',')
                        .filter(|key| !key.is_empty())
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
            scroll_prepends: request
                .header("X-Inertia-Infinite-Scroll-Merge-Intent")
                .is_some_and(|value| value.trim().eq_ignore_ascii_case("prepend")),
        }
    }
}

/// The metadata of the prop `prop` resolved at `path`.
pub(crate) fn classify(path: &str, prop: &Prop, request: &ClassifyRequest) -> PropMeta {
    // A deferred prop that is not a scroll prop is Laravel's `DeferProp`.
    // It counts as deferred only when the deferred-props visit delivers it;
    // reloaded by hand it is a plain prop with no type and no group.
    let is_defer_prop = prop.is_defer() && !prop.is_scroll();
    let deferred_delivery = is_defer_prop && request.deferred;
    let inertia_type = if prop.visibility() == Visibility::Always {
        Some("always")
    } else if is_defer_prop {
        deferred_delivery.then_some("defer")
    } else if prop.visibility() == Visibility::Optional {
        Some("optional")
    } else if prop.merge_mode().is_some() && !prop.is_scroll() {
        Some("merge")
    } else if prop.is_scroll() {
        Some("scroll")
    } else if prop.is_once() {
        Some("once")
    } else {
        None
    };
    let defer_group = (prop.is_defer() && (!is_defer_prop || deferred_delivery))
        .then(|| prop.defer_group().to_string());
    PropMeta {
        inertia_type,
        defer_group,
        reset: request.reset.iter().any(|key| key == path),
        once: prop.is_once(),
        merge_direction: merge_direction(prop, request),
        deep_merge: prop.merge_mode() == Some(MergeMode::Deep) || !prop.match_on_fields().is_empty(),
        rescued: false,
    }
}

/// The metadata of the `errors` prop the framework shares on every page,
/// as Laravel's middleware shares it with `Inertia::always`.
pub(crate) fn errors_meta() -> PropMeta {
    PropMeta {
        inertia_type: Some("always"),
        ..PropMeta::default()
    }
}

/// How a merging prop meets the client's data, read from the prop rather
/// than the page object so a deep merge keeps its direction: `prepend`
/// for a prop that prepends at its root, or only at paths; `append`
/// otherwise; `None` for a prop that does not merge.
fn merge_direction(prop: &Prop, request: &ClassifyRequest) -> Option<&'static str> {
    if prop.is_scroll() {
        let prepends = request.scroll_prepends && prop.merge_mode() != Some(MergeMode::Deep);
        return Some(if prepends { "prepend" } else { "append" });
    }
    let mode = prop.merge_mode()?;
    let follows_root = |paths: &[String]| !paths.is_empty();
    let prepends_nested = !prop.prepend_paths().is_empty()
        || (mode == MergeMode::Prepend && follows_root(prop.merge_paths()));
    let appends_nested = !prop.append_paths().is_empty()
        || (mode == MergeMode::Append && follows_root(prop.merge_paths()));
    let prepends_at_root = mode == MergeMode::Prepend && !prepends_nested && !appends_nested;
    if prepends_at_root || (prepends_nested && !appends_nested) {
        Some("prepend")
    } else {
        Some("append")
    }
}

impl PropMeta {
    /// Whether this path carries anything beyond being a prop, Laravel's
    /// `propHasMetadata`: a deep path without it is pruned.
    pub(crate) fn has_metadata(&self, shared: bool, has_source: bool) -> bool {
        shared
            || has_source
            || self.inertia_type.is_some()
            || self.defer_group.is_some()
            || self.reset
            || self.once
            || self.merge_direction.is_some()
            || self.deep_merge
            || self.rescued
    }

    /// The object an entry's `props` carries for this path, key for key
    /// Laravel's `Collector::addProp`: `shared` and `inertiaType` always,
    /// the rest only when they say something.
    pub(crate) fn to_json(&self, shared: bool, share_source: Option<Value>) -> Value {
        let mut object = Map::new();
        object.insert("shared".to_string(), Value::Bool(shared));
        object.insert(
            "inertiaType".to_string(),
            self.inertia_type.map_or(Value::Null, |kind| Value::String(kind.to_string())),
        );
        if let Some(group) = &self.defer_group {
            object.insert("deferGroup".to_string(), Value::String(group.clone()));
        }
        if let Some(source) = share_source {
            object.insert("shareSource".to_string(), source);
        }
        if self.reset {
            object.insert("reset".to_string(), Value::Bool(true));
        }
        if self.once {
            object.insert("once".to_string(), Value::Bool(true));
        }
        if let Some(direction) = self.merge_direction {
            object.insert("mergeDirection".to_string(), Value::String(direction.to_string()));
        }
        if self.deep_merge {
            object.insert("deepMerge".to_string(), Value::Bool(true));
        }
        if self.rescued {
            object.insert("rescued".to_string(), Value::Bool(true));
        }
        Value::Object(object)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn plain() -> ClassifyRequest {
        ClassifyRequest::default()
    }

    fn deferred() -> ClassifyRequest {
        ClassifyRequest {
            deferred: true,
            ..ClassifyRequest::default()
        }
    }

    #[test]
    fn indt_each_kind_of_prop_gets_its_pill() {
        let value = || Prop::eager(json!([1]));
        assert_eq!(classify("a", &value().always(), &plain()).inertia_type, Some("always"));
        assert_eq!(classify("a", &value().optional(), &plain()).inertia_type, Some("optional"));
        assert_eq!(classify("a", &value().merge(), &plain()).inertia_type, Some("merge"));
        assert_eq!(classify("a", &value().once(), &plain()).inertia_type, Some("once"));
        assert_eq!(classify("a", &value(), &plain()).inertia_type, None);
        // Composed flags: the first in Laravel's order wins.
        assert_eq!(classify("a", &value().merge().once(), &plain()).inertia_type, Some("merge"));
        assert_eq!(classify("a", &value().optional().once(), &plain()).inertia_type, Some("optional"));
    }

    #[test]
    fn indt_a_deferred_prop_is_defer_only_on_the_deferred_visit() {
        let prop = Prop::eager(json!(1)).defer().group("sidebar");
        let on_deferred = classify("a", &prop, &deferred());
        assert_eq!(on_deferred.inertia_type, Some("defer"));
        assert_eq!(on_deferred.defer_group.as_deref(), Some("sidebar"));
        let by_hand = classify("a", &prop, &plain());
        assert_eq!(by_hand.inertia_type, None);
        assert_eq!(by_hand.defer_group, None);
    }

    #[test]
    fn indt_merge_direction_and_deep_merge_follow_the_flags() {
        let value = || Prop::eager(json!([1]));
        assert_eq!(classify("a", &value().merge(), &plain()).merge_direction, Some("append"));
        assert_eq!(classify("a", &value().prepend(), &plain()).merge_direction, Some("prepend"));
        let deep = classify("a", &value().deep_merge(), &plain());
        assert_eq!(deep.merge_direction, Some("append"));
        assert!(deep.deep_merge);
        let matched = classify("a", &value().merge().match_on("id"), &plain());
        assert!(matched.deep_merge);
        assert_eq!(classify("a", &value(), &plain()).merge_direction, None);
    }

    #[test]
    fn indt_reset_reads_the_reset_header() {
        let request = ClassifyRequest {
            reset: vec!["feed".to_string()],
            ..ClassifyRequest::default()
        };
        assert!(classify("feed", &Prop::eager(json!([])).merge(), &request).reset);
        assert!(!classify("other", &Prop::eager(json!([])).merge(), &request).reset);
    }

    #[test]
    fn indt_the_props_object_carries_what_says_something() {
        let meta = PropMeta {
            inertia_type: Some("defer"),
            defer_group: Some("default".to_string()),
            rescued: true,
            ..PropMeta::default()
        };
        assert_eq!(
            meta.to_json(false, None),
            json!({"shared": false, "inertiaType": "defer", "deferGroup": "default", "rescued": true})
        );
        assert_eq!(
            PropMeta::default().to_json(true, Some(json!({"file": "f", "line": 3}))),
            json!({"shared": true, "inertiaType": null, "shareSource": {"file": "f", "line": 3}})
        );
    }
}
