//! The builder applies scope before retrieval and retains native relevance through hydration.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Instant;

use super::{
    Searchable,
    config::SearchConfig,
    document::{AttributeType, AttributeValue, GeoPoint, SearchDocument, canonical_json},
    embedder::validate_embedding,
    engine::{EngineCandidate, GenerationId, SearchEngine},
    expansion::expand,
    fusion::{federated_rrf, normalize, weighted_rrf},
    highlight::highlight,
    indexing::{IndexOperation, SearchIndexJob},
    query::{EngineQuery, Filter, GroupBy, Rerank, SearchQuery, Sort, SortDirection},
    registry::{IndexDescriptor, SearchRegistry},
    results::{
        FacetDistribution, FacetStats, FederatedHit, FederatedResults, Hit, RawHit,
        SearchPagination, SearchResults, Total,
    },
    scope::{Viewer, apply_scope},
    settings::IndexSettings,
};
use crate::{FrameworkError, eloquent::Builder};
use sea_orm::{EntityTrait, IntoActiveModel, PrimaryKeyTrait};
use serde::Serialize;

type HydrationCallback<M> = Box<dyn FnOnce(Builder<M>) -> Builder<M> + Send>;

/// Runtime index selection still executes the registered model's generic search builder.
pub struct IndexSearchBuilder {
    index: String,
    builder: SearchBuilder<()>,
}

impl IndexSearchBuilder {
    pub(crate) fn new(index: String) -> Self {
        Self {
            index,
            builder: SearchBuilder::new(""),
        }
    }

    /// Query text is parsed by the same core grammar as a statically named model.
    pub fn search(mut self, query: impl Into<String>) -> Self {
        self.builder.plan.text = super::query::QueryText::parse(&query.into());
        self
    }

    /// Bind filters against the resolved index before any driver read.
    pub fn filter(mut self, filter: impl Into<SearchFilter>) -> Self {
        self.builder = self.builder.filter(filter);
        self
    }

    /// Facets retain the generic builder's complete candidate counting universe.
    pub fn facets(mut self, attributes: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.builder = self.builder.facets(attributes);
        self
    }

    /// Formatting always uses code-owned tags and escaped source text.
    pub fn highlight(mut self, attributes: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.builder = self.builder.highlight(attributes);
        self
    }

    /// Sorting validates the declaration before binding engine input.
    pub fn sort_by(mut self, attribute: impl Into<String>, direction: SortDirection) -> Self {
        self.builder = self.builder.sort_by(attribute, direction);
        self
    }

    /// A bounded result window prevents an island from hydrating unbounded rows.
    pub fn limit(mut self, limit: usize) -> Self {
        self.builder = self.builder.limit(limit);
        self
    }

    /// Offsets preserve the same deterministic order used by the generic builder.
    pub fn offset(mut self, offset: usize) -> Self {
        self.builder = self.builder.offset(offset);
        self
    }

    /// Explicit viewer context keeps background and component reads scoped.
    pub fn for_viewer(mut self, viewer: Viewer) -> Self {
        self.builder = self.builder.for_viewer(viewer);
        self
    }

    /// Projection excludes model-only fields while preserving scores, highlights and analytics.
    pub async fn get(self) -> Result<SearchResults<RawHit>, FrameworkError> {
        let entry = SearchRegistry::by_index(&self.index)?;
        (entry.search)(self.builder.plan, self.builder.grammar, self.builder.viewer).await
    }
}

/// Derive-generated dispatch invokes generic hydration before exposing a document projection.
#[doc(hidden)]
pub async fn search_projected<M>(
    plan: SearchQuery,
    grammar: Vec<String>,
    viewer: Option<Viewer>,
) -> Result<SearchResults<RawHit>, FrameworkError>
where
    M: Searchable + From<<M::Entity as EntityTrait>::Model>,
    <M::Entity as EntityTrait>::Model: From<M>
        + IntoActiveModel<<M::Entity as EntityTrait>::ActiveModel>
        + Serialize
        + Send
        + Sync,
    <M::Entity as EntityTrait>::ActiveModel: Send,
    <<M::Entity as EntityTrait>::PrimaryKey as PrimaryKeyTrait>::ValueType:
        Send + Into<sea_orm::Value>,
{
    let builder = SearchBuilder::<M> {
        plan,
        grammar,
        viewer,
        hydration: None,
        model: PhantomData,
    };
    let (result, documents) = builder.execute().await?;
    Ok(SearchResults {
        hits: result
            .hits
            .into_iter()
            .map(|hit| project_hit(hit, &documents))
            .collect::<Result<_, _>>()?,
        total: result.total,
        facets: result.facets,
        facet_stats: result.facet_stats,
        processing_time_ms: result.processing_time_ms,
        engine: result.engine,
        query: result.query,
        pagination: result.pagination,
        impression: result.impression,
    })
}

fn project_hit<M>(
    hit: Hit<M>,
    documents: &HashMap<String, SearchDocument>,
) -> Result<Hit<RawHit>, FrameworkError> {
    let document = documents.get(&hit.key).ok_or_else(|| {
        FrameworkError::internal(format!(
            "search key '{}' lost its declared projection",
            hit.key
        ))
    })?;
    Ok(Hit {
        model: RawHit {
            attributes: document.attributes.clone(),
        },
        key: hit.key,
        engine_score: hit.engine_score,
        normalized_score: hit.normalized_score,
        engine: hit.engine,
        formatted: hit.formatted,
        geo_distance: hit.geo_distance,
        group_members: hit
            .group_members
            .into_iter()
            .map(|member| project_hit(member, documents))
            .collect::<Result<_, _>>()?,
    })
}

/// Deferred model queries keep fluent configuration infallible and terminal errors explicit.
/// Attribute and numeric validation runs at each terminal before any driver read.
pub struct SearchBuilder<M> {
    plan: SearchQuery,
    viewer: Option<Viewer>,
    grammar: Vec<String>,
    hydration: Option<HydrationCallback<M>>,
    model: PhantomData<M>,
}

impl<M> SearchBuilder<M> {
    /// Start with bounded lexical retrieval and ordinary Eloquent hydration.
    pub fn new(query: impl Into<String>) -> Self {
        Self {
            plan: SearchQuery::new(query),
            viewer: None,
            grammar: Vec::new(),
            hydration: None,
            model: PhantomData,
        }
    }
    /// Typed AST predicates are validated against the model declaration at execution.
    pub fn filter(mut self, filter: impl Into<SearchFilter>) -> Self {
        match filter.into() {
            SearchFilter::Expression(filter) => {
                self.plan.filter = Some(match self.plan.filter.take() {
                    Some(previous) => Filter::And(vec![previous, filter]),
                    None => filter,
                })
            }
            SearchFilter::Grammar(text) => self.grammar.push(text),
        }
        self
    }
    /// Named facets count all filtered lexical candidates before pagination.
    pub fn facets(mut self, attributes: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.plan.facets = attributes.into_iter().map(Into::into).collect();
        self
    }
    /// Numeric and datetime extrema use the same complete candidate set as facets.
    pub fn facet_stats(mut self, attributes: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.plan.facet_stats = attributes.into_iter().map(Into::into).collect();
        self
    }
    /// Attribute sorting precedes relevance and always places missing values last.
    pub fn sort_by(mut self, attribute: impl Into<String>, direction: SortDirection) -> Self {
        self.plan.sort.push(Sort::Attribute {
            attribute: attribute.into(),
            direction,
        });
        self
    }
    /// Geo ordering returns the distance in meters on each hydrated hit.
    pub fn sort_by_distance(
        mut self,
        attribute: impl Into<String>,
        point: GeoPoint,
        direction: SortDirection,
    ) -> Self {
        self.plan.sort.push(Sort::Distance {
            attribute: attribute.into(),
            point,
            direction,
        });
        self
    }
    /// Group representatives retain at most per_group total members including themselves.
    pub fn group_by(mut self, attribute: impl Into<String>, per_group: usize) -> Self {
        self.plan.group = Some(GroupBy {
            attribute: attribute.into(),
            per_group,
        });
        self
    }
    /// Requested text fields use engine highlights or the framework's escaped formatter.
    pub fn highlight(mut self, attributes: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.plan.highlight = attributes.into_iter().map(Into::into).collect();
        self
    }
    /// Crops are measured in visible Unicode characters and imply formatted output.
    pub fn crop(mut self, attribute: impl Into<String>, length: usize) -> Self {
        self.plan.crop.insert(attribute.into(), length);
        self
    }
    /// Semantic ratio zero preserves lexical order and one preserves cosine order.
    pub fn hybrid(mut self, ratio: f32) -> Self {
        self.plan.hybrid_ratio = Some(ratio);
        self
    }
    /// A registered cross-encoder reranks the bounded leading slice last.
    pub fn rerank(mut self, model: impl Into<String>, depth: usize) -> Self {
        self.plan.rerank = Some(Rerank {
            model: model.into(),
            depth,
        });
        self
    }
    /// Page limits are applied after scope rechecks, distinct and grouping.
    pub fn limit(mut self, limit: usize) -> Self {
        self.plan.limit = limit;
        self
    }
    /// Offsets slice the same key-tied ranked list on an unchanged generation.
    pub fn offset(mut self, offset: usize) -> Self {
        self.plan.offset = offset;
        self
    }
    /// A request can reduce the index's pagination cap but cannot increase it.
    pub fn hits_cap(mut self, cap: usize) -> Self {
        self.plan.hits_cap = Some(cap);
        self
    }
    /// Explicit viewers make scheduled and background reads as scoped as request reads.
    pub fn for_viewer(mut self, viewer: Viewer) -> Self {
        self.viewer = Some(viewer);
        self
    }
    /// Trashed rows are returned only when both the declaration and query opt in.
    pub fn with_trashed(mut self) -> Self {
        self.plan.with_trashed = true;
        self
    }
    /// Inspect the typed plan for diagnostics without exposing any rendered engine syntax.
    pub fn search_query(&self) -> &SearchQuery {
        &self.plan
    }
}

/// Both AST and grammar filters enter the same validation boundary.
pub enum SearchFilter {
    /// Typed expressions keep bound values independent of query syntax.
    Expression(Filter),
    /// Grammar is parsed against the actual model declaration at execution.
    Grammar(String),
}
impl From<Filter> for SearchFilter {
    fn from(filter: Filter) -> Self {
        Self::Expression(filter)
    }
}
impl From<String> for SearchFilter {
    fn from(text: String) -> Self {
        Self::Grammar(text)
    }
}
impl From<&str> for SearchFilter {
    fn from(text: &str) -> Self {
        Self::Grammar(text.into())
    }
}

impl<M> SearchBuilder<M>
where
    M: Searchable + From<<M::Entity as EntityTrait>::Model>,
    <M::Entity as EntityTrait>::Model: From<M>
        + IntoActiveModel<<M::Entity as EntityTrait>::ActiveModel>
        + Serialize
        + Send
        + Sync,
    <M::Entity as EntityTrait>::ActiveModel: Send,
    <<M::Entity as EntityTrait>::PrimaryKey as PrimaryKeyTrait>::ValueType:
        Send + Into<sea_orm::Value>,
{
    /// Modify one bound Eloquent query used to hydrate all candidates together.
    pub fn query<F>(mut self, callback: F) -> Self
    where
        F: FnOnce(Builder<M>) -> Builder<M> + Send + 'static,
    {
        self.hydration = Some(Box::new(callback));
        self
    }

    /// Retrieve models with ordering, scopes, facets and safe formatting preserved.
    pub async fn get(self) -> Result<SearchResults<M>, FrameworkError> {
        Ok(self.execute().await?.0)
    }

    /// Keys receive the same live-row scope recheck and deletion cleanup as hydrated hits.
    pub async fn keys(self) -> Result<Vec<String>, FrameworkError> {
        Ok(self
            .get()
            .await?
            .hits
            .into_iter()
            .map(|hit| hit.key)
            .collect())
    }

    /// Raw output projects declared attributes after live hydration checks have passed.
    pub async fn raw(self) -> Result<Vec<RawHit>, FrameworkError> {
        let (result, documents) = self.execute().await?;
        result
            .hits
            .into_iter()
            .map(|hit| {
                documents
                    .get(&hit.key)
                    .map(|document| RawHit {
                        attributes: document.attributes.clone(),
                    })
                    .ok_or_else(|| {
                        FrameworkError::internal(format!(
                            "search hit '{}' lost its document",
                            hit.key
                        ))
                    })
            })
            .collect()
    }

    /// Full pagination uses one-based page numbers and the engine's exact lexical total.
    pub async fn paginate(
        mut self,
        per_page: usize,
        page: usize,
    ) -> Result<SearchResults<M>, FrameworkError> {
        if page == 0 || per_page == 0 {
            return Err(FrameworkError::param(
                "search pagination page and per_page must be positive",
            ));
        }
        self.plan.limit = per_page;
        self.plan.offset = (page - 1)
            .checked_mul(per_page)
            .ok_or_else(|| FrameworkError::param("search pagination offset overflow"))?;
        self.get().await
    }

    /// Simple pagination exposes has_more while retaining the common response envelope.
    pub async fn simple_paginate(
        self,
        per_page: usize,
        page: usize,
    ) -> Result<SearchResults<M>, FrameworkError> {
        self.paginate(per_page, page).await
    }

    /// Facet type-ahead applies the same query, caller filter, soft-delete policy and scope.
    pub async fn facet_search(
        self,
        attribute: impl Into<String>,
        prefix: &str,
    ) -> Result<FacetDistribution, FrameworkError> {
        let descriptor = M::descriptor();
        let attribute = attribute.into();
        validate_facet(descriptor, &attribute, false)?;
        let viewer = match self.viewer {
            Some(viewer) => viewer,
            None => Viewer::current().await?,
        };
        let plan = bind_grammar(descriptor, self.plan, self.grammar)?;
        let retrieval = prepare(descriptor, &plan, &viewer).await?;
        retrieval
            .engine
            .facet_search(
                descriptor,
                &retrieval.generation,
                &retrieval.query,
                &attribute,
                prefix,
                retrieval.settings.query.facet_value_cap,
            )
            .await
    }

    async fn execute(
        self,
    ) -> Result<(SearchResults<M>, HashMap<String, SearchDocument>), FrameworkError> {
        let started = Instant::now();
        let descriptor = M::descriptor();
        let viewer = match self.viewer {
            Some(viewer) => viewer,
            None => Viewer::current().await?,
        };
        let plan = bind_grammar(descriptor, self.plan, self.grammar)?;
        let mut retrieved = retrieve(descriptor, &plan, &viewer).await?;
        let hydrated =
            Self::hydrate_with_refills(&mut retrieved, &plan, &viewer, self.hydration).await?;
        let mut hits = format_hits(hydrated.allowed, &retrieved, &plan)?;
        hits = group_hits(hits, plan.group.as_ref(), &hydrated.documents)?;
        rerank_hits(&mut hits, &hydrated.documents, &plan).await?;
        hits.truncate(retrieved.hits_cap);
        let pagination = pagination(&plan, hits.len());
        let mut hits: Vec<Hit<M>> = hits
            .into_iter()
            .skip(plan.offset)
            .take(plan.limit)
            .collect();
        normalize_hits(&mut hits);
        let processing_time_ms = elapsed_ms(started);
        let impression = super::analytics::record(
            descriptor.name,
            &plan,
            &viewer,
            retrieved.total.value(),
            processing_time_ms,
            hits.iter().map(|hit| hit.key.clone()).collect(),
        )
        .await?;
        Ok((
            SearchResults {
                impression,
                hits,
                total: retrieved.total,
                facets: retrieved.facets,
                facet_stats: retrieved.facet_stats,
                processing_time_ms,
                engine: retrieved.engine_name,
                query: plan.text.original.clone(),
                pagination,
            },
            hydrated.documents,
        ))
    }
    async fn hydrate_with_refills(
        retrieved: &mut Retrieved,
        plan: &SearchQuery,
        viewer: &Viewer,
        callback: Option<HydrationCallback<M>>,
    ) -> Result<Hydrated<M>, FrameworkError> {
        let descriptor = M::descriptor();
        let mut hydration = M::query();
        if descriptor.soft_delete && plan.with_trashed {
            hydration = hydration.__disable_named_scope("soft_deletes");
        }
        if let Some(callback) = callback {
            hydration = callback(hydration);
        }
        let universe = usize::try_from(retrieved.total.value()).unwrap_or(usize::MAX);
        let lexical = plan
            .hybrid_ratio
            .unwrap_or(retrieved.settings.query.hybrid_ratio)
            == 0.0;
        let mut hydrated;
        loop {
            let count = retrieved.candidates.len();
            hydrated = Self::hydrate_candidates(retrieved, plan, viewer, &hydration).await?;
            if !lexical
                || count < retrieved.query.depth
                || retrieved.query.depth >= universe
                || enough_representatives(
                    hydrated.allowed.iter().map(|(ranked, _)| &ranked.candidate),
                    &retrieved.settings,
                    plan,
                    retrieved.needed,
                )
            {
                break;
            }
            refill_lexical(descriptor, retrieved, universe).await?;
        }
        Ok(hydrated)
    }

    async fn hydrate_candidates(
        retrieved: &mut Retrieved,
        plan: &SearchQuery,
        viewer: &Viewer,
        hydration: &Builder<M>,
    ) -> Result<Hydrated<M>, FrameworkError> {
        let descriptor = M::descriptor();
        let entry = SearchRegistry::by_index(descriptor.name)?;
        let keys = retrieved
            .candidates
            .iter()
            .map(|candidate| {
                super::indexing::model_key_value::<M::Key>(
                    &candidate.candidate.document.key,
                    M::primary_key_name(),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let hydrate = hydration.clone().where_in(M::primary_key_name(), keys);
        let mut models = if retrieved.candidates.is_empty() {
            HashMap::new()
        } else {
            hydrate
                .get()
                .await?
                .into_vec()
                .into_iter()
                .map(|model| {
                    let key = super::indexing::document_key(&model.primary_key_value_json())?;
                    Ok((key, model))
                })
                .collect::<Result<HashMap<_, _>, FrameworkError>>()?
        };
        let missing_keys = retrieved
            .candidates
            .iter()
            .filter(|candidate| !models.contains_key(&candidate.candidate.document.key))
            .map(|candidate| candidate.candidate.document.key.clone())
            .collect::<Vec<_>>();
        let still_live = if missing_keys.is_empty() {
            BTreeSet::new()
        } else {
            (entry.load_documents)(missing_keys)
                .await?
                .into_iter()
                .map(|document| document.key)
                .collect()
        };
        let mut documents = HashMap::new();
        let mut allowed = Vec::new();
        for candidate in retrieved.candidates.drain(..) {
            let key = &candidate.candidate.document.key;
            if let Some(model) = models.remove(key) {
                let live = model.to_search_document()?;
                if !model.should_be_searchable()? || !(entry.evaluate_scope)(viewer, &live) {
                    continue;
                }
                let deleted = !M::SOFT_DELETES_COLUMN.is_empty()
                    && model
                        .field_value(M::SOFT_DELETES_COLUMN)
                        .is_some_and(|value| !value.is_null());
                if deleted && !(descriptor.soft_delete && plan.with_trashed) {
                    continue;
                }
                documents.insert(key.clone(), candidate.candidate.document.clone());
                allowed.push((candidate, model));
            } else {
                // Hydration callbacks and global scopes can hide a live row. Check the
                // inventory loader before recording a removal so filtering is not deletion.
                if !still_live.contains(key) {
                    SearchIndexJob {
                        index: descriptor.name.into(),
                        key: key.clone(),
                        source_version: candidate.candidate.document.source_version,
                        op: IndexOperation::Remove,
                        generation: Some(retrieved.generation.clone()),
                    }
                    .enqueue(None)
                    .await?;
                }
            }
        }
        Ok(Hydrated { allowed, documents })
    }
}

struct Hydrated<M> {
    allowed: Vec<(RankedCandidate, M)>,
    documents: HashMap<String, SearchDocument>,
}

fn format_hits<M>(
    allowed: Vec<(RankedCandidate, M)>,
    retrieved: &Retrieved,
    plan: &SearchQuery,
) -> Result<Vec<Hit<M>>, FrameworkError> {
    let scores: HashMap<_, _> = normalize(
        &allowed
            .iter()
            .map(|(candidate, _)| {
                (
                    candidate.candidate.document.key.clone(),
                    candidate.rank_score,
                )
            })
            .collect::<Vec<_>>(),
    )
    .into_iter()
    .collect();
    let mut seen_distinct = BTreeSet::new();
    let mut hits = Vec::new();
    for (candidate, model) in allowed {
        let document = &candidate.candidate.document;
        if let Some(attribute) = &retrieved.settings.query.distinct_attribute {
            let identity = value_identity(document.attributes.get(attribute));
            if !seen_distinct.insert(identity) {
                continue;
            }
        }
        let key = document.key.clone();
        let formatted = format_fields(
            document,
            candidate.candidate.highlights.as_ref(),
            &retrieved.query,
            &retrieved.settings,
            plan,
            retrieved.native_crop,
        )?;
        hits.push(Hit {
            model,
            key: key.clone(),
            engine_score: candidate.candidate.score,
            normalized_score: scores.get(&key).copied().unwrap_or(0.0),
            engine: retrieved.engine_name.clone(),
            formatted,
            geo_distance: geo_distance(document, &plan.sort),
            group_members: Vec::new(),
        });
    }
    Ok(hits)
}

fn group_hits<M>(
    mut hits: Vec<Hit<M>>,
    group: Option<&GroupBy>,
    documents: &HashMap<String, SearchDocument>,
) -> Result<Vec<Hit<M>>, FrameworkError> {
    if let Some(group) = group {
        let mut positions = HashMap::new();
        let mut grouped: Vec<Hit<M>> = Vec::new();
        for hit in hits {
            let document = documents
                .get(&hit.key)
                .ok_or_else(|| FrameworkError::internal("search grouping lost a document"))?;
            let identity = value_identity(document.attributes.get(&group.attribute));
            if let Some(position) = positions.get(&identity).copied() {
                let representative: &mut Hit<M> = &mut grouped[position];
                if representative.group_members.len() + 1 < group.per_group {
                    representative.group_members.push(hit);
                }
            } else {
                positions.insert(identity, grouped.len());
                grouped.push(hit);
            }
        }
        hits = grouped;
    }
    Ok(hits)
}

async fn rerank_hits<M>(
    hits: &mut [Hit<M>],
    documents: &HashMap<String, SearchDocument>,
    plan: &SearchQuery,
) -> Result<(), FrameworkError> {
    if let Some(rerank) = &plan.rerank {
        let depth = rerank.depth.min(hits.len());
        let inputs = hits[..depth]
            .iter()
            .map(|hit| {
                documents
                    .get(&hit.key)
                    .cloned()
                    .ok_or_else(|| FrameworkError::internal("search reranking lost a document"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let rescored = SearchRegistry::reranker(&rerank.model)?
            .rerank(&plan.text.original, &inputs)
            .await?;
        if rescored.len() != depth || rescored.iter().any(|score| !score.is_finite()) {
            return Err(FrameworkError::param(format!(
                "reranker '{}' returned invalid scores",
                rerank.model
            )));
        }
        let mut rank_scores = HashMap::new();
        for (hit, score) in hits[..depth].iter_mut().zip(rescored) {
            hit.normalized_score = score;
            rank_scores.insert(hit.key.clone(), score);
        }
        hits[..depth].sort_by(|a, b| {
            rank_scores[&b.key]
                .total_cmp(&rank_scores[&a.key])
                .then_with(|| a.key.cmp(&b.key))
        });
    }
    Ok(())
}

fn normalize_hits<M>(hits: &mut [Hit<M>]) {
    let maximum = hits
        .iter()
        .flat_map(|hit| {
            std::iter::once(hit.normalized_score).chain(
                hit.group_members
                    .iter()
                    .map(|member| member.normalized_score),
            )
        })
        .filter(|score| score.is_finite())
        .fold(0.0_f64, f64::max);
    for hit in hits {
        hit.normalized_score = if maximum > 0.0 {
            (hit.normalized_score / maximum).clamp(0.0, 1.0)
        } else {
            0.0
        };
        for member in &mut hit.group_members {
            member.normalized_score = if maximum > 0.0 {
                (member.normalized_score / maximum).clamp(0.0, 1.0)
            } else {
                0.0
            };
        }
    }
}

fn bind_grammar(
    descriptor: &IndexDescriptor,
    mut plan: SearchQuery,
    grammar: Vec<String>,
) -> Result<SearchQuery, FrameworkError> {
    if let Some(filter) = &plan.filter {
        filter.validate(descriptor)?;
    }
    for text in grammar {
        let filter = Filter::parse(&text, descriptor)?;
        plan.filter = Some(match plan.filter.take() {
            Some(previous) => Filter::And(vec![previous, filter]),
            None => filter,
        });
    }
    Ok(plan)
}

fn validate_facet(
    descriptor: &IndexDescriptor,
    name: &str,
    stats: bool,
) -> Result<(), FrameworkError> {
    let attribute = descriptor.attribute(name)?;
    if !attribute.facet {
        return Err(FrameworkError::param(format!(
            "attribute '{name}' is not a facet in index '{}'",
            descriptor.name
        )));
    }
    if stats
        && !matches!(
            attribute.ty,
            AttributeType::Integer | AttributeType::Float | AttributeType::DateTime
        )
    {
        return Err(FrameworkError::param(format!(
            "attribute '{name}' requires a numeric or datetime facet for stats"
        )));
    }
    Ok(())
}

fn validate_plan(descriptor: &IndexDescriptor, plan: &SearchQuery) -> Result<(), FrameworkError> {
    if let Some(filter) = &plan.filter {
        filter.validate(descriptor)?;
    }
    descriptor.validate()?;
    if plan.limit == 0 || plan.hits_cap == Some(0) {
        return Err(FrameworkError::param(
            "search limit and hits_cap must be positive",
        ));
    }
    if plan
        .hybrid_ratio
        .is_some_and(|r| !r.is_finite() || !(0.0..=1.0).contains(&r))
    {
        return Err(FrameworkError::param(
            "search hybrid ratio must be between 0 and 1",
        ));
    }
    if plan.with_trashed && !descriptor.soft_delete {
        return Err(FrameworkError::param(format!(
            "index '{}' does not opt into soft_delete",
            descriptor.name
        )));
    }
    for name in &plan.facets {
        validate_facet(descriptor, name, false)?;
    }
    for name in &plan.facet_stats {
        validate_facet(descriptor, name, true)?;
    }
    for sort in &plan.sort {
        let (name, point) = match sort {
            Sort::Attribute { attribute, .. } => (attribute, None),
            Sort::Distance {
                attribute, point, ..
            } => (attribute, Some(point)),
        };
        let attribute = descriptor.attribute(name)?;
        if !attribute.sortable {
            return Err(FrameworkError::param(format!(
                "attribute '{name}' is not sortable"
            )));
        }
        if let Some(point) = point {
            if attribute.ty != AttributeType::Geo {
                return Err(FrameworkError::param(format!(
                    "attribute '{name}' is not geo"
                )));
            }
            point.validate()?;
        }
    }
    for name in plan.highlight.iter().chain(plan.crop.keys()) {
        if !matches!(
            descriptor.attribute(name)?.ty,
            AttributeType::Text | AttributeType::TextList
        ) {
            return Err(FrameworkError::param(format!(
                "highlight attribute '{name}' must be text"
            )));
        }
    }
    if plan.crop.values().any(|length| *length == 0) {
        return Err(FrameworkError::param("search crop length must be positive"));
    }
    if let Some(group) = &plan.group {
        descriptor.attribute(&group.attribute)?;
        if group.per_group == 0 {
            return Err(FrameworkError::param(format!(
                "group attribute '{}' requires positive per_group",
                group.attribute
            )));
        }
    }
    if let Some(rerank) = &plan.rerank {
        if rerank.depth == 0 {
            return Err(FrameworkError::param(format!(
                "reranker '{}' requires positive depth",
                rerank.model
            )));
        }
        SearchRegistry::reranker(&rerank.model)?;
    }
    Ok(())
}

struct Prepared {
    engine: Arc<dyn SearchEngine>,
    generation: GenerationId,
    settings: IndexSettings,
    query: EngineQuery,
    ratio: f32,
    hits_cap: usize,
    needed: usize,
}

// One extra representative distinguishes a full last page from a page with a successor.
pub(super) fn lexical_depth(offset: usize, limit: usize, hits_cap: usize) -> usize {
    offset.saturating_add(limit).saturating_add(1).min(hits_cap)
}

async fn prepare(
    descriptor: &IndexDescriptor,
    plan: &SearchQuery,
    viewer: &Viewer,
) -> Result<Prepared, FrameworkError> {
    validate_plan(descriptor, plan)?;
    let engine = SearchRegistry::engine_for(descriptor.name)?;
    let generation = SearchRegistry::generation_for(descriptor.name).await?;
    let settings = engine.read_settings(descriptor, &generation).await?;
    settings.validate()?;
    let config = SearchConfig::current()?;
    let ratio = plan.hybrid_ratio.unwrap_or(settings.query.hybrid_ratio);
    let hits_cap = plan
        .hits_cap
        .unwrap_or(settings.query.hits_cap)
        .min(settings.query.hits_cap);
    let mut filter = apply_scope(descriptor, viewer, plan.filter.clone())?;
    if descriptor.soft_delete && !plan.with_trashed {
        let alive = Filter::Eq("__soft_deleted".into(), AttributeValue::Bool(false));
        filter = Some(match filter {
            Some(filter) => Filter::And(vec![filter, alive]),
            None => alive,
        });
    }
    let vocabulary = engine.vocabulary(descriptor, &generation).await?;
    let expanded = expand(
        &plan.text,
        &settings.query,
        &vocabulary,
        config.expansion_terms,
    )?;
    let needed = lexical_depth(plan.offset, plan.limit, hits_cap)
        .max(plan.rerank.as_ref().map_or(0, |rerank| rerank.depth))
        .min(hits_cap);
    let depth = if ratio > 0.0 {
        settings.query.fusion_depth.max(needed)
    } else {
        needed
    };
    let query = EngineQuery {
        expanded,
        filter,
        sort: plan.sort.clone(),
        depth,
        highlights: !plan.highlight.is_empty() || !plan.crop.is_empty(),
        crop: plan.crop.clone(),
    };
    Ok(Prepared {
        engine,
        generation,
        settings,
        query,
        ratio,
        hits_cap,
        needed,
    })
}

struct RankedCandidate {
    candidate: EngineCandidate,
    rank_score: f64,
}
struct Retrieved {
    engine_name: String,
    native_crop: bool,
    generation: GenerationId,
    settings: IndexSettings,
    query: EngineQuery,
    candidates: Vec<RankedCandidate>,
    total: Total,
    facets: FacetDistribution,
    facet_stats: FacetStats,
    hits_cap: usize,
    needed: usize,
}

async fn refill_lexical(
    descriptor: &IndexDescriptor,
    retrieved: &mut Retrieved,
    universe: usize,
) -> Result<(), FrameworkError> {
    retrieved.query.depth = retrieved.query.depth.saturating_mul(2).min(universe);
    let engine = SearchRegistry::engine_for(descriptor.name)?;
    let page = engine
        .query(descriptor, &retrieved.generation, &retrieved.query)
        .await?;
    validate_candidates(descriptor, &page.candidates)?;
    retrieved.candidates = page
        .candidates
        .into_iter()
        .map(|candidate| RankedCandidate {
            rank_score: candidate.score,
            candidate,
        })
        .collect();
    Ok(())
}

async fn federated_live_documents(
    descriptor: &IndexDescriptor,
    retrieved: &mut Retrieved,
    plan: &SearchQuery,
    viewer: &Viewer,
) -> Result<HashMap<String, SearchDocument>, FrameworkError> {
    let entry = SearchRegistry::by_index(descriptor.name)?;
    let universe = usize::try_from(retrieved.total.value()).unwrap_or(usize::MAX);
    let lexical = plan
        .hybrid_ratio
        .unwrap_or(retrieved.settings.query.hybrid_ratio)
        == 0.0;
    loop {
        let keys = retrieved
            .candidates
            .iter()
            .map(|ranked| ranked.candidate.document.key.clone())
            .collect();
        let live_by_key: HashMap<_, _> = (entry.load_documents)(keys)
            .await?
            .into_iter()
            .map(|document| (document.key.clone(), document))
            .collect();
        let eligible = retrieved
            .candidates
            .iter()
            .filter(|ranked| {
                live_by_key
                    .get(&ranked.candidate.document.key)
                    .is_some_and(|live| {
                        (entry.evaluate_scope)(viewer, live)
                            && !(descriptor.soft_delete
                                && !plan.with_trashed
                                && matches!(
                                    live.attributes.get("__soft_deleted"),
                                    Some(AttributeValue::Bool(true))
                                ))
                    })
            })
            .map(|ranked| &ranked.candidate);
        if !lexical
            || retrieved.candidates.len() < retrieved.query.depth
            || retrieved.query.depth >= universe
            || enough_representatives(eligible, &retrieved.settings, plan, retrieved.needed)
        {
            return Ok(live_by_key);
        }
        refill_lexical(descriptor, retrieved, universe).await?;
    }
}

async fn retrieve(
    descriptor: &IndexDescriptor,
    plan: &SearchQuery,
    viewer: &Viewer,
) -> Result<Retrieved, FrameworkError> {
    let prepared = prepare(descriptor, plan, viewer).await?;
    let Prepared {
        engine,
        generation,
        settings,
        query,
        ratio,
        hits_cap,
        needed,
    } = prepared;
    let mut query = query;
    let mut lexical = engine.query(descriptor, &generation, &query).await?;
    if ratio == 0.0 && (settings.query.distinct_attribute.is_some() || plan.group.is_some()) {
        while lexical.candidates.len()
            < usize::try_from(lexical.total.value()).unwrap_or(usize::MAX)
            && !enough_representatives(&lexical.candidates, &settings, plan, needed)
        {
            let previous = lexical.candidates.len();
            query.depth = query
                .depth
                .saturating_mul(2)
                .min(usize::try_from(lexical.total.value()).unwrap_or(usize::MAX));
            lexical = engine.query(descriptor, &generation, &query).await?;
            if lexical.candidates.len() <= previous {
                break;
            }
        }
    }
    let facets = if plan.facets.is_empty() {
        BTreeMap::new()
    } else {
        engine
            .facets(
                descriptor,
                &generation,
                &query,
                &plan.facets,
                settings.query.facet_value_cap,
                settings.query.facet_order,
            )
            .await?
    };
    let facet_stats = if plan.facet_stats.is_empty() {
        BTreeMap::new()
    } else {
        engine
            .facet_stats(descriptor, &generation, &query, &plan.facet_stats)
            .await?
    };
    validate_candidates(descriptor, &lexical.candidates)?;
    let total;
    let mut candidates;
    if ratio == 0.0 {
        total = lexical.total;
        candidates = lexical
            .candidates
            .into_iter()
            .map(|candidate| RankedCandidate {
                rank_score: candidate.score,
                candidate,
            })
            .collect::<Vec<_>>();
    } else {
        let embedder = match settings.index_time.embedder.as_deref() {
            Some(name) => SearchRegistry::embedder_named(descriptor.name, name)?,
            None => SearchRegistry::embedder_for(descriptor.name)?,
        };
        let vector = embedder.embed_query(&plan.text.original).await?;
        let vector = validate_embedding(
            vector.dense,
            embedder.profile(),
            settings.index_time.dimensions,
        )?;
        if let Some(mut native) = engine
            .hybrid(descriptor, &generation, &query, &vector.dense, ratio)
            .await?
        {
            validate_candidates(descriptor, &native.candidates)?;
            native.candidates.retain(|candidate| {
                !matches_negative(&candidate.document, &settings, &query.expanded.negatives)
            });
            total = native.total;
            candidates = native
                .candidates
                .into_iter()
                .map(|candidate| RankedCandidate {
                    rank_score: candidate.score,
                    candidate,
                })
                .collect();
        } else {
            let mut semantic = engine
                .semantic(
                    descriptor,
                    &generation,
                    &vector.dense,
                    query.filter.as_ref(),
                    query.depth,
                )
                .await?;
            validate_candidates(descriptor, &semantic.candidates)?;
            semantic.candidates.retain(|candidate| {
                !matches_negative(&candidate.document, &settings, &query.expanded.negatives)
            });
            let semantic_keys = semantic
                .candidates
                .iter()
                .map(|hit| hit.document.key.clone())
                .collect::<Vec<_>>();
            let lexical_keys = lexical
                .candidates
                .iter()
                .map(|hit| hit.document.key.clone())
                .collect::<Vec<_>>();
            let ranks = if ratio == 1.0 {
                semantic
                    .candidates
                    .iter()
                    .map(|hit| (hit.document.key.clone(), hit.score))
                    .collect()
            } else {
                weighted_rrf(&[
                    (1.0 - f64::from(ratio), lexical_keys),
                    (f64::from(ratio), semantic_keys),
                ])
            };
            let mut by_key: HashMap<_, _> = semantic
                .candidates
                .into_iter()
                .map(|candidate| (candidate.document.key.clone(), candidate))
                .collect();
            if ratio != 1.0 {
                for candidate in lexical.candidates {
                    by_key.insert(candidate.document.key.clone(), candidate);
                }
            }
            candidates = ranks
                .into_iter()
                .filter_map(|(key, score)| {
                    by_key.remove(&key).map(|candidate| RankedCandidate {
                        candidate,
                        rank_score: score,
                    })
                })
                .collect::<Vec<_>>();
            total = Total::Estimated(candidates.len() as u64);
        }
    }
    if ratio > 0.0 && !plan.sort.is_empty() {
        candidates.sort_by(|a, b| {
            compare_sort(&a.candidate.document, &b.candidate.document, &plan.sort)
                .then_with(|| b.rank_score.total_cmp(&a.rank_score))
                .then_with(|| a.candidate.document.key.cmp(&b.candidate.document.key))
        });
    }

    Ok(Retrieved {
        engine_name: engine.name().into(),
        native_crop: engine.capabilities().native_crop,
        generation,
        settings,
        query,
        candidates,
        total,
        facets,
        facet_stats,
        hits_cap,
        needed,
    })
}

/// Semantic retrieval shares lexical exclusions across builders and judged baselines.
pub(super) fn matches_negative(
    document: &SearchDocument,
    settings: &IndexSettings,
    negatives: &[String],
) -> bool {
    !negatives.is_empty()
        && settings
            .index_time
            .searchable_attributes
            .iter()
            .filter_map(|attribute| document.attributes.get(&attribute.name))
            .filter_map(AttributeValue::searchable_text)
            .flat_map(|text| super::expansion::tokenize(&text))
            .any(|word| negatives.contains(&word))
}

fn validate_candidates(
    descriptor: &IndexDescriptor,
    candidates: &[EngineCandidate],
) -> Result<(), FrameworkError> {
    let mut keys = BTreeSet::new();
    for candidate in candidates {
        candidate.document.validate(descriptor)?;
        if !candidate.score.is_finite() || !keys.insert(&candidate.document.key) {
            return Err(FrameworkError::param(format!(
                "index '{}' returned non-finite scores or duplicate key '{}'",
                descriptor.name, candidate.document.key
            )));
        }
    }
    Ok(())
}

pub(crate) fn compare_sort(a: &SearchDocument, b: &SearchDocument, sorts: &[Sort]) -> Ordering {
    for sort in sorts {
        let (va, vb, direction) = match sort {
            Sort::Attribute {
                attribute,
                direction,
            } => (
                a.attributes.get(attribute).cloned(),
                b.attributes.get(attribute).cloned(),
                direction,
            ),
            Sort::Distance {
                attribute,
                point,
                direction,
            } => {
                let distance = |document: &SearchDocument| match document.attributes.get(attribute)
                {
                    Some(AttributeValue::Geo(value)) => {
                        Some(AttributeValue::Float(value.distance_to(point)))
                    }
                    _ => None,
                };
                (distance(a), distance(b), direction)
            }
        };
        let va = va.filter(|value| !matches!(value, AttributeValue::Null));
        let vb = vb.filter(|value| !matches!(value, AttributeValue::Null));
        let order = match (va, vb) {
            (Some(a), Some(b)) => {
                let order = a.compare(&b).unwrap_or(Ordering::Equal);
                if *direction == SortDirection::Desc {
                    order.reverse()
                } else {
                    order
                }
            }
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        };
        if order != Ordering::Equal {
            return order;
        }
    }
    Ordering::Equal
}

fn format_fields(
    document: &SearchDocument,
    native: Option<&BTreeMap<String, String>>,
    query: &EngineQuery,
    settings: &IndexSettings,
    plan: &SearchQuery,
    native_crop: bool,
) -> Result<BTreeMap<String, String>, FrameworkError> {
    let mut fields = BTreeMap::new();
    for name in plan.highlight.iter().chain(plan.crop.keys()) {
        if let Some(text) = document
            .attributes
            .get(name)
            .and_then(AttributeValue::searchable_text)
        {
            // Drivers advertising native crop return complete fragments within the budget.
            let formatted = if !plan.crop.contains_key(name) || native_crop {
                native
                    .and_then(|fields| fields.get(name))
                    .filter(|formatted| native_fragment_is_safe(formatted, &settings.query))
                    .cloned()
            } else {
                None
            };
            fields.insert(
                name.clone(),
                match formatted {
                    Some(formatted) => formatted,
                    None => highlight(
                        &text,
                        &query.expanded,
                        &settings.query,
                        plan.crop.get(name).copied(),
                    )?,
                },
            );
        }
    }
    Ok(fields)
}

fn native_fragment_is_safe(mut fragment: &str, settings: &super::settings::QuerySettings) -> bool {
    let mut open = false;
    while !fragment.is_empty() {
        if let Some(rest) = fragment.strip_prefix(&settings.highlight_pre_tag) {
            if open {
                return false;
            }
            open = true;
            fragment = rest;
            continue;
        }
        if let Some(rest) = fragment.strip_prefix(&settings.highlight_post_tag) {
            if !open {
                return false;
            }
            open = false;
            fragment = rest;
            continue;
        }
        if fragment.starts_with('&') {
            let Some(entity) = [
                "&amp;", "&lt;", "&gt;", "&quot;", "&apos;", "&#x27;", "&#39;",
            ]
            .into_iter()
            .find(|entity| fragment.starts_with(entity)) else {
                return false;
            };
            fragment = &fragment[entity.len()..];
            continue;
        }
        let Some(character) = fragment.chars().next() else {
            break;
        };
        if matches!(character, '<' | '>' | '\'' | '"') {
            return false;
        }
        fragment = &fragment[character.len_utf8()..];
    }
    !open
}

fn value_identity(value: Option<&AttributeValue>) -> String {
    match value {
        Some(AttributeValue::Float(v)) if *v == 0.0 => {
            canonical_json(serde_json::json!(AttributeValue::Float(0.0)))
        }
        Some(AttributeValue::Geo(point)) => {
            let point = GeoPoint {
                lat: if point.lat == 0.0 { 0.0 } else { point.lat },
                lng: if point.lng == 0.0 { 0.0 } else { point.lng },
            };
            canonical_json(serde_json::json!(AttributeValue::Geo(point)))
        }
        value => canonical_json(serde_json::json!(value.unwrap_or(&AttributeValue::Null))),
    }
}

fn enough_representatives<'a>(
    candidates: impl IntoIterator<Item = &'a EngineCandidate>,
    settings: &IndexSettings,
    plan: &SearchQuery,
    needed: usize,
) -> bool {
    let mut distinct = BTreeSet::new();
    let mut groups = BTreeMap::<String, usize>::new();
    let mut group_order = Vec::new();
    let mut count = 0;
    for candidate in candidates {
        let document = &candidate.document;
        if let Some(attribute) = &settings.query.distinct_attribute
            && !distinct.insert(value_identity(document.attributes.get(attribute)))
        {
            continue;
        }
        if let Some(group) = &plan.group {
            let identity = value_identity(document.attributes.get(&group.attribute));
            let members = groups.entry(identity.clone()).or_default();
            if *members == 0 {
                count += 1;
                group_order.push(identity);
            }
            *members += 1;
        } else {
            count += 1;
        }
    }
    count >= needed
        && plan.group.as_ref().is_none_or(|group| {
            group_order
                .iter()
                .take(needed)
                .all(|identity| groups[identity] >= group.per_group)
        })
}

fn geo_distance(document: &SearchDocument, sorts: &[Sort]) -> Option<f64> {
    sorts.iter().find_map(|sort| match sort {
        Sort::Distance {
            attribute, point, ..
        } => match document.attributes.get(attribute) {
            Some(AttributeValue::Geo(value)) => Some(value.distance_to(point)),
            _ => None,
        },
        _ => None,
    })
}
fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}
fn pagination(plan: &SearchQuery, count: usize) -> SearchPagination {
    SearchPagination {
        limit: plan.limit,
        offset: plan.offset,
        page: (plan.offset / plan.limit).saturating_add(1),
        has_more: count > plan.offset.saturating_add(plan.limit),
    }
}

/// Heterogeneous indexes share the same scoped pipeline and a combined RRF page.
pub struct FederatedBuilder {
    indexes: Vec<String>,
    plan: SearchQuery,
    viewer: Option<Viewer>,
    grammar: Vec<String>,
}
impl FederatedBuilder {
    /// Preserve requested index order because it is the tie-break in a federation.
    pub fn new(indexes: Vec<String>) -> Self {
        Self {
            indexes,
            plan: SearchQuery::new(""),
            viewer: None,
            grammar: Vec::new(),
        }
    }
    /// Set the shared query text before any index-specific expansion.
    pub fn search(mut self, text: impl Into<String>) -> Self {
        self.plan.text = super::query::QueryText::parse(&text.into());
        self
    }
    /// Every index resolves grammar against its own declarations.
    pub fn filter(mut self, filter: impl Into<SearchFilter>) -> Self {
        match filter.into() {
            SearchFilter::Expression(filter) => {
                self.plan.filter = Some(match self.plan.filter.take() {
                    Some(previous) => Filter::And(vec![previous, filter]),
                    None => filter,
                })
            }
            SearchFilter::Grammar(text) => self.grammar.push(text),
        };
        self
    }
    /// Apply the same named viewer to every index's scope.
    pub fn for_viewer(mut self, viewer: Viewer) -> Self {
        self.viewer = Some(viewer);
        self
    }
    /// Count named facets within each index's complete lexical candidate set.
    pub fn facets(mut self, attributes: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.plan.facets = attributes.into_iter().map(Into::into).collect();
        self
    }
    /// Expose extrema under each index name rather than mixing schemas.
    pub fn facet_stats(mut self, attributes: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.plan.facet_stats = attributes.into_iter().map(Into::into).collect();
        self
    }
    /// Formatted text uses each index's validated highlight settings.
    pub fn highlight(mut self, attributes: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.plan.highlight = attributes.into_iter().map(Into::into).collect();
        self
    }
    /// Crop source text under each index's own marker and tags.
    pub fn crop(mut self, attribute: impl Into<String>, length: usize) -> Self {
        self.plan.crop.insert(attribute.into(), length);
        self
    }
    /// The shared semantic ratio still uses each index's own model provider.
    pub fn hybrid(mut self, ratio: f32) -> Self {
        self.plan.hybrid_ratio = Some(ratio);
        self
    }
    /// Limit the merged page after per-index ranking.
    pub fn limit(mut self, limit: usize) -> Self {
        self.plan.limit = limit;
        self
    }
    /// Offset into the stable index-order-tied combined list.
    pub fn offset(mut self, offset: usize) -> Self {
        self.plan.offset = offset;
        self
    }
    /// Heterogeneous results are projected documents with live eligibility rechecks.
    pub async fn get(self) -> Result<FederatedResults, FrameworkError> {
        let started = Instant::now();
        if self.indexes.is_empty()
            || self.indexes.iter().collect::<BTreeSet<_>>().len() != self.indexes.len()
        {
            return Err(FrameworkError::param(
                "federated search requires unique nonempty indexes",
            ));
        }
        let viewer = match self.viewer {
            Some(viewer) => viewer,
            None => Viewer::current().await?,
        };
        let mut lists = Vec::new();
        let mut hits = HashMap::new();
        let mut total = 0_u64;
        let mut estimated = false;
        let mut facets = BTreeMap::new();
        let mut facet_stats = BTreeMap::new();
        for index in &self.indexes {
            let entry = SearchRegistry::by_index(index)?;
            let descriptor = (entry.descriptor)();
            if let Some(filter) = &self.plan.filter {
                filter.validate(descriptor)?;
            }
            let plan = bind_grammar(descriptor, self.plan.clone(), self.grammar.clone())?;
            let mut retrieved = retrieve(descriptor, &plan, &viewer).await?;
            let mut live_by_key =
                federated_live_documents(descriptor, &mut retrieved, &plan, &viewer).await?;
            total = total.saturating_add(retrieved.total.value());
            estimated |= matches!(retrieved.total, Total::Estimated(_));
            facets.insert(index.clone(), retrieved.facets);
            facet_stats.insert(index.clone(), retrieved.facet_stats);
            let mut seen_distinct = BTreeSet::new();
            let mut keys = Vec::new();
            for ranked in retrieved.candidates {
                let candidate = ranked.candidate;
                let Some(live) = live_by_key.remove(&candidate.document.key) else {
                    SearchIndexJob {
                        index: index.clone(),
                        key: candidate.document.key,
                        source_version: candidate.document.source_version,
                        op: IndexOperation::Remove,
                        generation: Some(retrieved.generation.clone()),
                    }
                    .enqueue(None)
                    .await?;
                    continue;
                };
                if !(entry.evaluate_scope)(&viewer, &live) {
                    continue;
                }
                if descriptor.soft_delete
                    && !plan.with_trashed
                    && matches!(
                        live.attributes.get("__soft_deleted"),
                        Some(AttributeValue::Bool(true))
                    )
                {
                    continue;
                }
                if let Some(attribute) = &retrieved.settings.query.distinct_attribute
                    && !seen_distinct
                        .insert(value_identity(candidate.document.attributes.get(attribute)))
                {
                    continue;
                }
                let key = candidate.document.key.clone();
                let formatted = format_fields(
                    &candidate.document,
                    candidate.highlights.as_ref(),
                    &retrieved.query,
                    &retrieved.settings,
                    &plan,
                    retrieved.native_crop,
                )?;
                hits.insert(
                    (index.clone(), key.clone()),
                    FederatedHit {
                        index: index.clone(),
                        key: key.clone(),
                        attributes: candidate.document.attributes,
                        score: 0.0,
                        engine: retrieved.engine_name.clone(),
                        formatted,
                    },
                );
                keys.push(key);
                if keys.len() == retrieved.hits_cap {
                    break;
                }
            }
            lists.push((index.clone(), keys));
        }
        let ranks = federated_rrf(&lists);
        let pagination = pagination(&self.plan, ranks.len());
        let mut merged = Vec::new();
        for rank in ranks
            .into_iter()
            .skip(self.plan.offset)
            .take(self.plan.limit)
        {
            if let Some(mut hit) = hits.remove(&(rank.index, rank.key)) {
                hit.score = rank.score;
                merged.push(hit);
            }
        }
        Ok(FederatedResults {
            hits: merged,
            total: if estimated {
                Total::Estimated(total)
            } else {
                Total::Exact(total)
            },
            facets,
            facet_stats,
            processing_time_ms: elapsed_ms(started),
            query: self.plan.text.original,
            pagination,
        })
    }
    /// Federated keys include their index because primary keys are not globally unique.
    pub async fn keys(self) -> Result<Vec<(String, String)>, FrameworkError> {
        Ok(self
            .get()
            .await?
            .hits
            .into_iter()
            .map(|hit| (hit.index, hit.key))
            .collect())
    }
    /// Full merged pagination uses one-based pages.
    pub async fn paginate(
        mut self,
        per_page: usize,
        page: usize,
    ) -> Result<FederatedResults, FrameworkError> {
        if per_page == 0 || page == 0 {
            return Err(FrameworkError::param(
                "federated pagination page and per_page must be positive",
            ));
        }
        self.plan.limit = per_page;
        self.plan.offset = (page - 1)
            .checked_mul(per_page)
            .ok_or_else(|| FrameworkError::param("federated pagination offset overflow"))?;
        self.get().await
    }
}
