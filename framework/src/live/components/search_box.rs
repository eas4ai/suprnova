//! Search presentation keeps retrieval and access decisions in the core builder.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::FrameworkError;
use crate::live::action::{LiveEventPayload, OutcomeMetadata, RegisteredEmission, action_result};
use crate::live::{
    ActionOutcome, ActionResult, ComponentContract, EventPayloadMetadata, InputLimits,
    LiveComponent, live,
};
use crate::search::{AttributeValue, Search, SearchRegistry};
use crate::view::{TrustedHtml, TrustedMarkupReason};

mod filters {
    pub use crate::view::filters::{live_key_digest, trusted_html};
}

/// A view projection omits hydrated models and preserves only core-built highlights.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SearchBoxHit {
    /// The selected key is checked against this signed result list before emission.
    key: String,
    /// Escaped text and code-owned highlight tags are produced by the core pipeline.
    formatted: BTreeMap<String, String>,
}

impl SearchBoxHit {
    /// Expose the selected document identity without exposing construction of trusted fragments.
    pub fn key(&self) -> &String {
        &self.key
    }

    /// The core's escaped fields remain immutable so callers cannot replace them with raw HTML.
    pub fn formatted(&self) -> &BTreeMap<String, String> {
        &self.formatted
    }

    /// Only formatting from this signed core result may cross the template trust boundary.
    pub fn highlight(&self, attribute: &str) -> Result<TrustedHtml, FrameworkError> {
        let html = self.formatted.get(attribute).ok_or_else(|| {
            FrameworkError::validation(
                "highlight",
                format!("attribute '{attribute}' has no formatted field"),
            )
        })?;
        let reason = TrustedMarkupReason::new(
            "Core search escaped document text before adding code-owned highlight tags",
        )
        .map_err(|error| FrameworkError::internal(format!("search highlight reason: {error}")))?;
        TrustedHtml::framework_generated(html.clone(), reason).map_err(|error| {
            FrameworkError::internal(format!("search highlight '{attribute}': {error}"))
        })
    }
}

/// Facet choices carry bound grammar literals rather than raw database expressions.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SearchBoxFacet {
    /// Attribute names come from the index's declared facet attributes.
    pub attribute: String,
    /// Labels remain ordinary escaped template text.
    pub label: String,
    /// The filter parser revalidates this expression on each search.
    pub filter: String,
    /// Counts share the core search's scoped candidate set.
    pub count: u64,
}

/// A registered event returns the chosen document identity to the hosting page.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SearchSelected {
    /// Index identity disambiguates model keys across mounted search islands.
    pub index: String,
    /// Only a key in the current signed result list can be selected.
    pub key: String,
}
impl EventPayloadMetadata for SearchSelected {
    const NAME: &'static str = "search.selected";
    const VERSION: u16 = 1;
}
impl LiveEventPayload for SearchSelected {}

/// Mount an index by name so applications need no model-specific search component.
/// RenderCache classification: varies. Query, viewer and selected facets determine
/// each result list, and each mounted widget owns one island.
#[derive(LiveComponent)]
#[live(
    name = "suprnova.search-box",
    view = "suprnova-ui/search-box/island.html",
    events(SearchSelected)
)]
pub struct SearchBox {
    /// Query proposals are bounded to one update after a settled edit.
    #[model(debounce = 250)]
    query: String,
    /// Checked facets are parsed through the index's declared filter grammar.
    #[model]
    filters: Vec<String>,
    /// Hidden controls send strings; checked integer sequences keep repeated queries ordered.
    #[model(debounce = 250)]
    sequence: String,
    /// Selection proposals have no authority until checked against the signed hits.
    #[model]
    selection: String,
    /// The hidden control's string is checked as a zero-based shown-list position before use.
    #[model]
    position: String,
    /// A stable server-owned control identity keeps repeated mounts' ARIA references distinct.
    #[locked]
    control_id: String,
    /// The mount owns the index, so a model proposal cannot switch it.
    #[locked]
    index: String,
    /// Bounded result counts prevent an island requesting unbounded retrieval.
    #[locked]
    per_page: u32,
    /// Result projections contain no model fields outside the index declaration.
    #[public]
    hits: Vec<SearchBoxHit>,
    /// Exact scoped facet counts provide optional checkable filters.
    #[public]
    facets: Vec<SearchBoxFacet>,
    /// The impression belongs to the session that rendered this signed list.
    #[public]
    impression: String,
}

#[live]
impl SearchBox {
    /// Server-owned mount parameters choose one registered index and a bounded page.
    #[mount]
    pub fn mount(index: String, per_page: u32) -> Result<Self, FrameworkError> {
        if !(1..=100).contains(&per_page) {
            return Err(FrameworkError::validation(
                "per_page",
                "per_page must be between 1 and 100",
            ));
        }
        SearchRegistry::by_index(&index)?;
        Ok(Self {
            query: String::new(),
            filters: Vec::new(),
            sequence: "0".into(),
            selection: String::new(),
            position: "0".into(),
            control_id: format!("search-{}", uuid::Uuid::new_v4()),
            index,
            per_page,
            hits: Vec::new(),
            facets: Vec::new(),
            impression: String::new(),
        })
    }

    /// Every render reruns the generic builder under the request's authenticated or guest scope.
    #[rendering]
    pub async fn search(&mut self) -> Result<(), FrameworkError> {
        self.sequence.parse::<u32>().map_err(|_| {
            FrameworkError::validation(
                "sequence",
                "sequence must be an integer between 0 and 4294967295",
            )
        })?;
        let entry = SearchRegistry::by_index(&self.index)?;
        let descriptor = (entry.descriptor)();
        let highlight = descriptor
            .attributes
            .iter()
            .filter(|attribute| attribute.searchable.is_some())
            .map(|attribute| attribute.name.clone())
            .collect::<Vec<_>>();
        let facets = descriptor
            .attributes
            .iter()
            .filter(|attribute| attribute.facet)
            .map(|attribute| attribute.name.clone())
            .collect::<Vec<_>>();
        let mut builder = Search::for_index(&self.index)?
            .search(&self.query)
            .highlight(highlight)
            .facets(facets)
            .limit(self.per_page as usize);
        for filter in &self.filters {
            builder = builder.filter(filter.as_str());
        }
        let result = builder.get().await?;
        self.hits = result
            .hits
            .into_iter()
            .map(|hit| SearchBoxHit {
                key: hit.key,
                formatted: hit.formatted,
            })
            .collect();
        self.impression = result.impression.unwrap_or_default();
        self.facets = result
            .facets
            .into_iter()
            .flat_map(|(attribute, values)| {
                values
                    .into_iter()
                    .filter_map(move |facet| facet_choice(&attribute, facet.value, facet.count))
            })
            .collect();
        Ok(())
    }

    /// A zero-result facet selection must remain recoverable even when no choices are returned.
    #[action]
    pub fn clear_filters(&mut self) -> Result<ActionOutcome, FrameworkError> {
        self.filters.clear();
        Ok(ActionOutcome::Render)
    }

    /// Validate and record the shown selection before returning its key through search.selected.
    #[action]
    pub async fn select(&mut self) -> Result<ActionResult, FrameworkError> {
        self.sequence.parse::<u32>().map_err(|_| {
            FrameworkError::validation(
                "sequence",
                "sequence must be an integer between 0 and 4294967295",
            )
        })?;
        let position = self.position.parse::<usize>().map_err(|_| {
            FrameworkError::validation("position", "position must be a nonnegative integer")
        })?;
        if self
            .hits
            .get(position)
            .is_none_or(|hit| hit.key != self.selection)
        {
            return Err(FrameworkError::validation(
                "key",
                "key and position must name a shown search result",
            ));
        }
        if !self.impression.is_empty() {
            Search::click(&self.impression, &self.selection, position).await?;
        }
        let registration = Self::__live_registration().map_err(|error| {
            FrameworkError::internal(format!("search-box registration: {error}"))
        })?;
        let descriptor = registration.into_engine();
        let event = RegisteredEmission::event(
            &descriptor,
            &SearchSelected {
                index: self.index.clone(),
                key: self.selection.clone(),
            },
            &InputLimits::default(),
        )
        .map_err(|error| FrameworkError::internal(format!("search selected event: {error}")))?;
        let metadata =
            OutcomeMetadata::new(Vec::new(), vec![event], Vec::new(), None).map_err(|error| {
                FrameworkError::internal(format!("search selected outcome: {error}"))
            })?;
        action_result::<Self>(ActionOutcome::NoRender, metadata)
            .map_err(|error| FrameworkError::internal(format!("search selected action: {error}")))
    }
}

fn facet_choice(attribute: &str, value: AttributeValue, count: u64) -> Option<SearchBoxFacet> {
    let literal = match value {
        AttributeValue::Text(value) => serde_json::Value::String(value),
        AttributeValue::Integer(value) => serde_json::Value::from(value),
        AttributeValue::Float(value) => serde_json::Value::from(value),
        AttributeValue::Bool(value) => serde_json::Value::Bool(value),
        AttributeValue::DateTime(value) => serde_json::Value::String(value.to_rfc3339()),
        AttributeValue::Null => serde_json::Value::Null,
        AttributeValue::Geo(_) | AttributeValue::List(_) => return None,
    };
    let label = literal
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| literal.to_string());
    let filter = if literal.is_null() {
        format!("{attribute} IS NULL")
    } else {
        format!("{attribute} = {literal}")
    };
    Some(SearchBoxFacet {
        attribute: attribute.to_owned(),
        label,
        filter,
        count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn selection_emits_the_validated_key_and_refuses_unknown_keys_or_positions() {
        let mut component = SearchBox {
            query: String::new(),
            filters: Vec::new(),
            sequence: "0".into(),
            selection: "one".into(),
            position: "0".into(),
            control_id: "search-test".into(),
            index: "notes".into(),
            per_page: 10,
            hits: vec![SearchBoxHit {
                key: "one".into(),
                formatted: BTreeMap::new(),
            }],
            facets: Vec::new(),
            impression: String::new(),
        };
        let accepted = component.select().await.expect("shown selection");
        assert_eq!(accepted.outcome(), &ActionOutcome::NoRender);
        let events = accepted.metadata().events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].name().as_str(), "search.selected");
        let crate::live::CanonicalValue::Object(payload) = events[0].payload() else {
            unreachable!("the typed selection event is an object");
        };
        assert_eq!(
            payload.get("key"),
            Some(&crate::live::CanonicalValue::String("one".into()))
        );
        component.selection = "unknown".into();
        assert!(component.select().await.is_err());
        component.selection = "one".into();
        component.position = "1".into();
        assert!(component.select().await.is_err());
        component.position = "0".into();
        component.sequence = "invalid".into();
        assert!(component.select().await.is_err());
    }

    #[test]
    fn invalid_page_sizes_are_rejected_before_search() {
        assert!(SearchBox::mount("notes".into(), 0).is_err());
        assert!(SearchBox::mount("notes".into(), 101).is_err());
    }

    #[test]
    fn only_framework_escaped_highlight_fragments_are_trusted() {
        let hit = SearchBoxHit {
            key: "one".into(),
            formatted: BTreeMap::from([(
                "title".into(),
                "<mark>Journal</mark> &lt;script&gt;".into(),
            )]),
        };
        let html = hit.highlight("title").expect("bounded formatted field");
        assert_eq!(html.as_str(), "<mark>Journal</mark> &lt;script&gt;");
        assert!(hit.highlight("secret").is_err());
    }
}
