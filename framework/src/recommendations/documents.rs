//! Search owns candidate filtering and live hydration owns the final authorization check.
use super::RecommendationTarget;
use crate::{EngineQuery, FrameworkError, SearchDocument, SearchRegistry, Viewer};
use std::collections::BTreeMap;

pub(crate) type Catalog = BTreeMap<RecommendationTarget, SearchDocument>;

pub(crate) async fn visible(viewer: &Viewer) -> Result<Catalog, FrameworkError> {
    catalog(viewer, false).await
}

pub(crate) async fn additional(viewer: &Viewer) -> Result<Catalog, FrameworkError> {
    catalog(viewer, true).await
}

async fn catalog(viewer: &Viewer, exclude_guest: bool) -> Result<Catalog, FrameworkError> {
    let mut documents = Catalog::new();
    for index in SearchRegistry::indexes()? {
        let entry = SearchRegistry::by_index(index)?;
        let descriptor = (entry.descriptor)();
        let engine = SearchRegistry::engine_for(index)?;
        let generation = SearchRegistry::generation_for(index).await?;
        let guest = crate::search::apply_scope(descriptor, &Viewer::Guest, None)?;
        if exclude_guest && guest.is_none() {
            continue;
        }
        let extra = if exclude_guest {
            guest.map(|filter| crate::Filter::Not(Box::new(filter)))
        } else {
            None
        };
        let mut query = EngineQuery {
            expanded: Default::default(),
            filter: crate::search::apply_scope(descriptor, viewer, extra)?,
            sort: Vec::new(),
            depth: 1,
            highlights: false,
            crop: BTreeMap::new(),
        };
        let count = engine.count(descriptor, &generation, &query).await?.value();
        query.depth = usize::try_from(count)
            .map_err(|_| {
                FrameworkError::param("recommendation search catalog exceeds addressable size")
            })?
            .max(1);
        let result = engine.query(descriptor, &generation, &query).await?;
        let keys: Vec<_> = result
            .candidates
            .iter()
            .map(|hit| hit.document.key.clone())
            .collect();
        for chunk in keys.chunks(500) {
            for document in (entry.load_documents)(chunk.to_vec()).await? {
                if !(entry.evaluate_scope)(viewer, &document)
                    || !crate::search::evaluate_scope(descriptor, viewer, &document)
                {
                    continue;
                }
                if !chunk.contains(&document.key) || document.index != index {
                    return Err(FrameworkError::internal(
                        "recommendation search loader returned an unrequested document",
                    ));
                }
                documents.insert(
                    RecommendationTarget {
                        index: index.into(),
                        key: document.key.clone(),
                    },
                    document,
                );
            }
        }
    }
    Ok(documents)
}
pub(crate) fn facets(
    document: &SearchDocument,
) -> Result<BTreeMap<String, Vec<String>>, FrameworkError> {
    let descriptor = (SearchRegistry::by_index(&document.index)?.descriptor)();
    let mut values = BTreeMap::new();
    for attribute in descriptor.attributes.iter().filter(|a| a.facet) {
        let Some(value) = document.attributes.get(&attribute.name) else {
            continue;
        };
        let labels = match value {
            crate::AttributeValue::Null => continue,
            crate::AttributeValue::List(values) => values.iter().map(|v| v.facet_key()).collect(),
            value => vec![value.facet_key()],
        };
        values.insert(format!("{}:{}", document.index, attribute.name), labels);
    }
    Ok(values)
}
pub(crate) fn recency(document: &SearchDocument) -> i64 {
    ["published_at", "created_at", "updated_at"]
        .into_iter()
        .find_map(|name| match document.attributes.get(name) {
            Some(crate::AttributeValue::DateTime(time)) => Some(time.timestamp_micros()),
            Some(crate::AttributeValue::Integer(time)) => Some(*time),
            _ => None,
        })
        .unwrap_or(document.source_version)
}
