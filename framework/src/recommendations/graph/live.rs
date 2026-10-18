//! A stored fact is a copy taken when it was derived, and its source can change or leave
//! before the queued job that revokes it runs, or while a rebuild that read it publishes.
//! So a read checks every fact against its live source before it walks, scores or cites
//! it (REC-005, REC-007). It checks what the read relies on, not the version number, so a
//! document saved again keeps counting while its graph job waits.
use super::{Gate, GraphEdge, GraphNode, GraphProvenance};
use crate::{
    FrameworkError, SearchDocument, SearchRegistry, Viewer,
    recommendations::{RecommendationTarget, storage},
};
use async_trait::async_trait;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::{Mutex, PoisonError},
};

/// Keys bound per `IN` list.
const KEYS_PER_STATEMENT: usize = 200;
/// Keys loaded per search document batch.
const DOCUMENTS_PER_LOAD: usize = 500;

/// The live search documents for `targets`: loaded through each index's loader and held
/// by the generation the index's searches read. A document that left its index, because
/// its predicate turned false or `unsearchable` removed it, is absent, and so is one of an
/// index that is no longer registered.
///
/// # Errors
///
/// Returns an error when a loader or the search engine fails.
pub(crate) async fn indexed_documents<'a>(
    targets: impl IntoIterator<Item = &'a RecommendationTarget>,
) -> Result<BTreeMap<RecommendationTarget, SearchDocument>, FrameworkError> {
    let mut by_index: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
    for target in targets {
        by_index
            .entry(target.index.as_str())
            .or_default()
            .insert(target.key.clone());
    }
    let mut documents = BTreeMap::new();
    for (index, keys) in by_index {
        let Ok(entry) = SearchRegistry::by_index(index) else {
            continue;
        };
        let descriptor = (entry.descriptor)();
        let engine = SearchRegistry::engine_for(index)?;
        let generation = SearchRegistry::generation_for(index).await?;
        let keys: Vec<String> = keys.into_iter().collect();
        for chunk in keys.chunks(DOCUMENTS_PER_LOAD) {
            let held = engine.stored_keys(descriptor, &generation, chunk).await?;
            let held: Vec<String> = chunk
                .iter()
                .filter(|key| held.contains(*key))
                .cloned()
                .collect();
            if held.is_empty() {
                continue;
            }
            for document in (entry.load_documents)(held).await? {
                documents.insert(
                    RecommendationTarget {
                        index: index.to_owned(),
                        key: document.key.clone(),
                    },
                    document,
                );
            }
        }
    }
    Ok(documents)
}

/// One read's view of the live sources, loaded once per document and event.
pub(crate) struct LiveFacts {
    viewer: Viewer,
    documents: Mutex<HashMap<RecommendationTarget, Option<SearchDocument>>>,
    events: Mutex<HashMap<i64, bool>>,
}

impl LiveFacts {
    pub(crate) fn new(viewer: &Viewer) -> Self {
        Self {
            viewer: viewer.clone(),
            documents: Mutex::default(),
            events: Mutex::default(),
        }
    }

    /// The live documents among `targets` the viewer may see.
    ///
    /// # Errors
    ///
    /// Returns an error when a loader or the search engine fails.
    pub(crate) async fn visible(
        &self,
        targets: &[RecommendationTarget],
    ) -> Result<BTreeMap<RecommendationTarget, SearchDocument>, FrameworkError> {
        self.load_documents(targets.iter()).await?;
        Ok(targets
            .iter()
            .filter_map(|target| {
                self.document(target)
                    .filter(|document| self.permits(document))
                    .map(|document| (target.clone(), document))
            })
            .collect())
    }

    fn document(&self, target: &RecommendationTarget) -> Option<SearchDocument> {
        self.documents
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(target)
            .cloned()
            .flatten()
    }

    fn permits(&self, document: &SearchDocument) -> bool {
        let Ok(entry) = SearchRegistry::by_index(&document.index) else {
            return false;
        };
        (entry.evaluate_scope)(&self.viewer, document)
            && crate::search::evaluate_scope((entry.descriptor)(), &self.viewer, document)
    }

    async fn load_documents<'a>(
        &self,
        targets: impl Iterator<Item = &'a RecommendationTarget>,
    ) -> Result<(), FrameworkError> {
        let missing: BTreeSet<RecommendationTarget> = {
            let known = self
                .documents
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            targets
                .filter(|target| !known.contains_key(*target))
                .cloned()
                .collect()
        };
        if missing.is_empty() {
            return Ok(());
        }
        let found = indexed_documents(&missing).await?;
        let mut known = self
            .documents
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        for target in missing {
            let document = found.get(&target).cloned();
            known.insert(target, document);
        }
        Ok(())
    }

    async fn load_events(&self, ids: impl Iterator<Item = i64>) -> Result<(), FrameworkError> {
        let missing: Vec<i64> = {
            let known = self.events.lock().unwrap_or_else(PoisonError::into_inner);
            let mut missing: Vec<i64> = ids.filter(|id| !known.contains_key(id)).collect();
            missing.sort_unstable();
            missing.dedup();
            missing
        };
        let mut present = BTreeSet::new();
        for chunk in missing.chunks(KEYS_PER_STATEMENT) {
            let list = vec!["?"; chunk.len()].join(", ");
            for row in storage::rows(
                &format!("SELECT id FROM recommendation_events WHERE id IN ({list})"),
                chunk.iter().map(|id| sea_orm::Value::from(*id)).collect(),
            )
            .await?
            {
                present.insert(row.get_int("id")?);
            }
        }
        let mut known = self.events.lock().unwrap_or_else(PoisonError::into_inner);
        for id in missing {
            known.insert(id, present.contains(&id));
        }
        Ok(())
    }

    /// Load what `provenance` items rest on.
    async fn load_provenance<'a>(
        &self,
        provenance: impl Iterator<Item = &'a GraphProvenance> + Clone,
    ) -> Result<(), FrameworkError> {
        self.load_events(provenance.clone().filter_map(GraphProvenance::event_id))
            .await?;
        self.load_documents(provenance.filter_map(GraphProvenance::source))
            .await
    }

    /// Whether the source a fact was derived from still stands: its event is still in
    /// the stream, or its document is still indexed and, when it has advanced past the
    /// revision the fact was read from, its relation still finds `other`, the fact's node
    /// at the far end from that document.
    fn holds(&self, provenance: &GraphProvenance, other: &str) -> Result<bool, FrameworkError> {
        match provenance {
            GraphProvenance::Event(event) => Ok(self
                .events
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(&event.id)
                .copied()
                .unwrap_or(false)),
            GraphProvenance::Relation {
                name,
                source,
                source_version,
            } => {
                let Some(document) = self.document(source) else {
                    return Ok(false);
                };
                if document.source_version <= *source_version {
                    return Ok(true);
                }
                Ok(super::related_keys(name, &document)?.is_some_and(|keys| keys.contains(other)))
            }
        }
    }
}

#[async_trait]
impl Gate for LiveFacts {
    /// A document node passes while its live document is indexed and the viewer may see
    /// it; every node needs its provenance to hold.
    async fn nodes(&self, nodes: Vec<GraphNode>) -> Result<Vec<GraphNode>, FrameworkError> {
        self.load_documents(nodes.iter().filter_map(|node| node.target.as_ref()))
            .await?;
        self.load_provenance(nodes.iter().map(|node| &node.provenance))
            .await?;
        let mut kept = Vec::with_capacity(nodes.len());
        for node in nodes {
            let live = node.target.as_ref().is_none_or(|target| {
                self.document(target)
                    .is_some_and(|document| self.permits(&document))
            });
            if live && self.holds(&node.provenance, &node.key)? {
                kept.push(node);
            }
        }
        Ok(kept)
    }

    /// An edge passes while its provenance holds; its two nodes pass on their own.
    async fn edges(&self, edges: Vec<GraphEdge>) -> Result<Vec<GraphEdge>, FrameworkError> {
        self.load_provenance(edges.iter().map(|edge| &edge.provenance))
            .await?;
        let mut kept = Vec::with_capacity(edges.len());
        for edge in edges {
            // A relation edge runs from its source document or, as an inverse, to it.
            let other = match edge.provenance.source() {
                Some(source) if edge.source == super::document_node_key(source) => &edge.target,
                _ => &edge.source,
            };
            if self.holds(&edge.provenance, other)? {
                kept.push(edge);
            }
        }
        Ok(kept)
    }
}
