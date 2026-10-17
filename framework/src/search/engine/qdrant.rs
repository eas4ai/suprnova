//! Qdrant owns cosine, sparse ranking, text predicates, exact facets and weighted RRF.
//!
//! Each generation is `search_<index>_<generation>`. Without embedding dimensions
//! the named dense slot has size 1, carries no vector and is never queried. This
//! keeps the dense slot available for lexical-only and sparse-only indexes. Lexical retrieval
//! ranks one server-encoded BM25 sparse vector per weighted attribute with IDF.
//! A formula sums weighted attribute scores; a named sparse provider overrides this.
//! Text predicates filter words,
//! phrases and exclusions without scoring them. Empty queries use source version.
//! The framework supplies highlights. Prefixes use keyword token payloads because
//! Qdrant's `match.prefix` applies to keyword values, not full-text tokens.
//!
//! Writes read source versions before upserting. The queue's one-job-per-key
//! ordering makes this check safe. Direct callers must also serialize writes and
//! removals for a key. The shared generation control row serializes engine instances
//! across processes too, including import batches. Removals commit tombstones
//! before deleting and recheck versions under that shared lock. Tombstones are
//! scoped by index, generation and document key. SQL and Qdrant are not one atomic
//! transaction: a lost acknowledgement leaves the remote result uncertain.
//! Retry through the authoritative queue loader rather than replaying snapshots;
//! serialization does not fence an abandoned mutation that is still in flight.
//! Token payloads are replaced on every write and removed with the document;
//! vocabulary reads reconcile shared metadata from these payloads, counting a word once per document.
//! Vocabulary reads scan token payloads; categorical, geo and multi-column sorts
//! retain a bounded top-k while paging the full lexical universe. Ordered strings
//! and large-integer ranges use exact keyword dictionaries because native numeric
//! range bounds are f64. Named model column weights are combined before indexing;
//! Qdrant BM25 encodes each attribute's text with the text index's language options.
//! Boundary score ties require deeper attribute windows or extra native pages.
//! Threshold retrieval completes attribute sums over candidate ids; returned
//! candidates still obey the requested depth.
mod predicates;
mod storage;
mod threshold;
use super::*;
use crate::{
    search::{AttributeType, AttributeValue, LanguageConfig, SearchDocument, Sort, tokenize},
    vector::qdrant::{QdrantVectorDriver, SUPRNOVA_ID_PAYLOAD_KEY},
};
use qdrant_client::qdrant::{self as qb, Condition, Filter as QFilter};
use qdrant_client::{Payload, Qdrant, QdrantError};
use serde_json::{Value, json};
use std::{
    collections::{BTreeSet, HashMap},
    time::Duration,
};

/// Persistent retrieval shares metadata with indexing while leaving ranking in Qdrant.
pub struct QdrantSearchEngine {
    client: Arc<Qdrant>,
    mutations: tokio::sync::Mutex<()>,
    #[cfg(any(test, feature = "testing"))]
    max_attribute_limit: std::sync::atomic::AtomicU64,
}
impl QdrantSearchEngine {
    /// Use the vector driver's variables and bound every gRPC call with a timeout.
    pub fn from_env() -> Result<Self, FrameworkError> {
        Self::from_variables(|name| std::env::var(name).ok())
    }
    fn from_variables(variable: impl Fn(&str) -> Option<String>) -> Result<Self, FrameworkError> {
        let set = |name: &str| {
            variable(name)
                .map(|v| v.trim().to_owned())
                .filter(|v| !v.is_empty())
        };
        let url = set("QDRANT_URL")
            .ok_or_else(|| FrameworkError::param("QDRANT_URL: Qdrant search needs a gRPC URL"))?;
        let timeout = match set("SEARCH_QDRANT_TIMEOUT_SECS") {
            Some(v) => v.parse::<u64>().ok().filter(|v| *v > 0).ok_or_else(|| {
                FrameworkError::param("SEARCH_QDRANT_TIMEOUT_SECS: requires a positive integer")
            })?,
            None => 10,
        };
        Self::from_url(&url, set("QDRANT_API_KEY"), timeout)
    }
    /// Explicit configuration makes tests independent of process-global variables.
    pub fn from_url(
        url: &str,
        api_key: Option<String>,
        timeout_secs: u64,
    ) -> Result<Self, FrameworkError> {
        if timeout_secs == 0 {
            return Err(FrameworkError::param(
                "SEARCH_QDRANT_TIMEOUT_SECS: requires a positive integer",
            ));
        }
        let mut builder = Qdrant::from_url(url)
            .skip_compatibility_check()
            .timeout(Duration::from_secs(timeout_secs));
        if let Some(key) = api_key {
            builder = builder.api_key(key);
        }
        let client = builder.build().map_err(|_| {
            FrameworkError::param("QDRANT_URL: cannot construct the Qdrant search client")
        })?;
        Ok(Self::from_client(client))
    }
    fn from_client(client: Qdrant) -> Self {
        Self {
            client: Arc::new(client),
            mutations: tokio::sync::Mutex::new(()),
            #[cfg(any(test, feature = "testing"))]
            max_attribute_limit: std::sync::atomic::AtomicU64::new(0),
        }
    }
    /// Measures retrieval work without relying on timings in live Qdrant tests.
    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn take_attribute_limit_for_test(&self) -> u64 {
        self.max_attribute_limit
            .swap(0, std::sync::atomic::Ordering::Relaxed)
    }

    async fn ranked_query(
        &self,
        request: qb::QueryPoints,
    ) -> Result<qb::QueryResponse, QdrantError> {
        #[cfg(any(test, feature = "testing"))]
        self.max_attribute_limit.fetch_max(
            attribute_limit(&request.prefetch).max(
                request
                    .using
                    .as_deref()
                    .filter(|name| name.starts_with("bm25_"))
                    .map_or(0, |_| request.limit.unwrap_or(0)),
            ),
            std::sync::atomic::Ordering::Relaxed,
        );
        self.client.query(request).await
    }

    /// The native client permits schema inspection and engine-specific administration.
    pub fn client(&self) -> Result<&Qdrant, FrameworkError> {
        Ok(&self.client)
    }
    /// Render a validated scope-inclusive lexical predicate for diagnostics or native queries.
    pub async fn render_filter(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        q: &EngineQuery,
    ) -> Result<QFilter, FrameworkError> {
        let collection = collection_name(index, generation)?;
        let settings = self.read_settings(index, generation).await?;
        self.query_filter(index, &settings, q, &collection).await
    }
    async fn query_filter(
        &self,
        index: &IndexDescriptor,
        settings: &IndexSettings,
        q: &EngineQuery,
        collection: &str,
    ) -> Result<QFilter, FrameworkError> {
        validate_query(index, q)?;
        let mut lexical = q.clone();
        lexical.filter = None;
        let mut filter = predicates::lexical(settings, &lexical)?;
        if let Some(scope) = &q.filter {
            filter
                .must
                .push(self.bound_predicate(collection, scope).await?);
        }
        Ok(filter)
    }
    fn bound_predicate<'a>(
        &'a self,
        collection: &'a str,
        filter: &'a Filter,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Condition, FrameworkError>> + Send + 'a>,
    > {
        Box::pin(async move {
            match filter {
                Filter::And(children) | Filter::Or(children) => {
                    let mut conditions = Vec::new();
                    for child in children {
                        conditions.push(self.bound_predicate(collection, child).await?);
                    }
                    Ok(if matches!(filter, Filter::And(_)) {
                        QFilter::must(conditions).into()
                    } else if conditions.is_empty() {
                        QFilter::must_not([QFilter::default().into()]).into()
                    } else {
                        QFilter::should(conditions).into()
                    })
                }
                Filter::Not(child) => {
                    Ok(QFilter::must_not([self.bound_predicate(collection, child).await?]).into())
                }
                Filter::Gt(key, value)
                | Filter::Gte(key, value)
                | Filter::Lt(key, value)
                | Filter::Lte(key, value)
                | Filter::Between(key, value, _)
                    if dictionary_range(filter, value) =>
                {
                    // Native numeric ranges use f64 bounds. Exact keyword
                    // labels preserve large integers and ordered strings.
                    let field = format!("__value_{key}");
                    let mut seen = Vec::<String>::new();
                    let mut matching = Vec::new();
                    loop {
                        let exclusion = if seen.is_empty() {
                            QFilter::default()
                        } else {
                            QFilter::must_not([Condition::matches(&field, seen.clone())])
                        };
                        let response = self
                            .client
                            .facet(
                                qb::FacetCountsBuilder::new(collection, &field)
                                    .filter(exclusion)
                                    .limit(256)
                                    .exact(true),
                            )
                            .await
                            .map_err(|e| error(collection, e))?;
                        let exhausted = response.hits.len() < 256;
                        for hit in response.hits {
                            let Some(qb::facet_value::Variant::StringValue(label)) =
                                hit.value.and_then(|v| v.variant)
                            else {
                                return Err(error(
                                    collection,
                                    format!("attribute {key}: range label missing"),
                                ));
                            };
                            let value = match value {
                                AttributeValue::Text(_) => AttributeValue::Text(label.clone()),
                                _ => AttributeValue::Integer(label.parse().map_err(|_| {
                                    error(
                                        collection,
                                        format!("attribute {key}: invalid integer label"),
                                    )
                                })?),
                            };
                            let document = SearchDocument {
                                index: String::new(),
                                key: String::new(),
                                source_version: 0,
                                attributes: BTreeMap::from([(key.clone(), value)]),
                                embed_text: None,
                                title: None,
                            };
                            if filter.matches(&document) {
                                matching.push(label.clone());
                            }
                            seen.push(label);
                        }
                        if exhausted {
                            break;
                        }
                    }
                    Ok(if matching.is_empty() {
                        QFilter::must_not([QFilter::default().into()]).into()
                    } else {
                        Condition::matches(field, matching)
                    })
                }
                _ => predicates::predicate(filter),
            }
        })
    }
    async fn get(
        &self,
        collection: &str,
        key: &str,
    ) -> Result<Option<HashMap<String, qb::Value>>, FrameworkError> {
        let points = self
            .client
            .get_points(
                qb::GetPointsBuilder::new(
                    collection,
                    vec![QdrantVectorDriver::resolve_point_id(key)],
                )
                .with_payload(true),
            )
            .await
            .map_err(|e| error(collection, e))?;
        Ok(points.result.into_iter().next().map(|p| p.payload))
    }
    async fn apply_indexes(
        &self,
        collection: &str,
        schema: &BTreeMap<String, IndexSchema>,
    ) -> Result<(), FrameworkError> {
        for (field, spec) in schema {
            let mut builder =
                qb::CreateFieldIndexCollectionBuilder::new(collection, field.clone(), spec.ty)
                    .wait(true);
            if let Some(params) = &spec.params {
                builder = builder.field_index_params(params.clone());
            }
            self.client
                .create_field_index(builder)
                .await
                .map_err(|e| error(&format!("{collection} attribute {field}"), e))?;
        }
        Ok(())
    }
    async fn check_schema(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        settings: &IndexSettings,
    ) -> Result<(), FrameworkError> {
        let collection = collection_name(index, generation)?;
        let info = self
            .client
            .collection_info(&collection)
            .await
            .map_err(|e| error(&collection, e))?
            .result
            .ok_or_else(|| error(&collection, "collection info missing"))?;
        let params = info
            .config
            .and_then(|c| c.params)
            .ok_or_else(|| error(&collection, "collection config missing"))?;
        let dense = match params.vectors_config.and_then(|c| c.config) {
            Some(qb::vectors_config::Config::ParamsMap(map)) => map.map.get("dense").cloned(),
            _ => None,
        }
        .ok_or_else(|| error(&collection, "named dense vector missing"))?;
        if dense.size != settings.index_time.dimensions.unwrap_or(1) as u64
            || dense.distance != qb::Distance::Cosine as i32
        {
            return Err(error(
                &collection,
                "dense vector config disagrees with stored settings",
            ));
        }
        let sparse = params.sparse_vectors_config.unwrap_or_default().map;
        let names = sparse_names(settings);
        if sparse.len() != names.len()
            || names.iter().any(|name| {
                sparse
                    .get(name)
                    .is_none_or(|v| v.modifier != Some(qb::Modifier::Idf as i32))
            })
        {
            return Err(error(
                &collection,
                "sparse vector config disagrees with stored settings",
            ));
        }
        for (field, spec) in schema(index, settings)? {
            let actual = info.payload_schema.get(&field).ok_or_else(|| {
                error(
                    &collection,
                    format!("attribute {field}: payload index missing"),
                )
            })?;
            if actual.data_type != payload_type(spec.ty) as i32 {
                return Err(error(
                    &collection,
                    format!("attribute {field}: payload type differs from settings"),
                ));
            }
            if let Some(wanted) = spec.params {
                let actual = actual.params.as_ref().and_then(|p| p.index_params.as_ref());
                if !params_match(&wanted, actual) {
                    return Err(error(
                        &collection,
                        format!("attribute {field}: payload options differ from settings"),
                    ));
                }
            }
        }
        Ok(())
    }
    async fn scroll_page(
        &self,
        collection: &str,
        filter: QFilter,
        offset: Option<qb::PointId>,
        fields: Option<Vec<String>>,
    ) -> Result<qb::ScrollResponse, FrameworkError> {
        let mut builder = qb::ScrollPointsBuilder::new(collection)
            .filter(filter)
            .limit(256)
            .with_vectors(false);
        builder = match fields {
            Some(fields) => {
                builder.with_payload(qb::with_payload_selector::SelectorOptions::Include(
                    qb::PayloadIncludeSelector { fields },
                ))
            }
            None => builder.with_payload(true),
        };
        if let Some(offset) = offset {
            builder = builder.offset(offset);
        }
        self.client
            .scroll(builder)
            .await
            .map_err(|e| error(collection, e))
    }
    async fn exact_count(
        &self,
        collection: &str,
        filter: QFilter,
    ) -> Result<Total, FrameworkError> {
        let response = self
            .client
            .count(
                qb::CountPointsBuilder::new(collection)
                    .filter(filter)
                    .exact(true),
            )
            .await
            .map_err(|e| error(collection, e))?;
        let count = response
            .result
            .ok_or_else(|| error(collection, "exact count missing"))?
            .count;
        Ok(Total::Exact(count))
    }
    async fn sparse_query(
        &self,
        index: &IndexDescriptor,
        settings: &IndexSettings,
        q: &EngineQuery,
    ) -> Result<Option<SparseEmbedding>, FrameworkError> {
        let text = lexical_text(q);
        if text.is_empty() {
            return Ok(None);
        }
        let embedder = crate::search::SearchRegistry::sparse_embedder(index.name, settings)?;
        let vector = embedder.embed_sparse_query(&text).await?;
        vector.validate()?;
        Ok(Some(vector))
    }
    async fn query_points(
        &self,
        index: &IndexDescriptor,
        collection: &str,
        query: qb::Query,
        using: Option<&str>,
        filter: QFilter,
        depth: usize,
    ) -> Result<Vec<EngineCandidate>, FrameworkError> {
        if depth == 0 {
            return Ok(Vec::new());
        }
        let mut builder = qb::QueryPointsBuilder::new(collection)
            .query(query)
            .filter(filter)
            .limit(depth as u64)
            .with_payload(hit_payload(index));
        if let Some(using) = using {
            builder = builder.using(using);
        }
        self.stable_rank(index, collection, builder.build(), depth)
            .await
    }
    async fn stable_rank(
        &self,
        index: &IndexDescriptor,
        collection: &str,
        mut request: qb::QueryPoints,
        depth: usize,
    ) -> Result<Vec<EngineCandidate>, FrameworkError> {
        let lookahead = depth.saturating_add(1) as u64;
        request.limit = Some(lookahead);
        request.offset = None;
        grow_prefetch(&mut request.prefetch, lookahead, false);
        let response = self
            .ranked_query(request.clone())
            .await
            .map_err(|e| error(collection, e))?;
        let mut candidates = decode_scored(index, response.result)?;
        if candidates.len() <= depth || candidates[depth].score < candidates[depth - 1].score {
            candidates.truncate(depth);
            return Ok(candidates);
        }
        // Qdrant limits equal-score points by point id, which is not the
        // document key. Resolve the complete boundary group before truncating.
        // Composite prefetches must cover the lexical universe while paging
        // ties, otherwise increasing their window can replace earlier points.
        if !request.prefetch.is_empty() {
            let count = self
                .exact_count(collection, request.filter.clone().unwrap_or_default())
                .await?
                .value();
            grow_prefetch(&mut request.prefetch, count, true);
        }
        if matches!(
            request.query.as_ref().and_then(|q| q.variant.as_ref()),
            Some(qb::query::Variant::Nearest(_))
        ) {
            request.params.get_or_insert_default().exact = Some(true);
        }
        // Re-score the universe once per bounded page, rather than once per
        // small fixed slice when a large result boundary contains many ties.
        let page_size = depth.saturating_add(1).clamp(256, 4096) as u64;
        request.limit = Some(page_size);
        request.offset = Some(0);
        candidates.clear();
        loop {
            let response = self
                .ranked_query(request.clone())
                .await
                .map_err(|e| error(collection, e))?;
            let len = response.result.len();
            let tail = response.result.last().map(|p| f64::from(p.score));
            candidates.extend(decode_scored(index, response.result)?);
            candidates.sort_by(|a, b| {
                b.score
                    .total_cmp(&a.score)
                    .then_with(|| a.document.key.cmp(&b.document.key))
            });
            candidates.truncate(depth);
            let boundary = candidates.get(depth - 1).map(|c| c.score);
            if len < page_size as usize
                || matches!((tail,boundary),(Some(tail),Some(boundary)) if tail<boundary)
            {
                break;
            }
            request.offset = Some(request.offset.unwrap_or(0).saturating_add(page_size));
        }
        Ok(candidates)
    }
    async fn lexical_prefetch(
        &self,
        index: &IndexDescriptor,
        settings: &IndexSettings,
        q: &EngineQuery,
        vector: Option<&[f32]>,
        filter: QFilter,
    ) -> Result<qb::PrefetchQuery, FrameworkError> {
        let text = lexical_text(q);
        if native_bm25(settings) && !text.is_empty() {
            let document = bm25_document(index, settings, text)?;
            let mut children = sparse_names(settings)
                .into_iter()
                .map(|name| {
                    qb::PrefetchQueryBuilder::default()
                        .query(qb::Query::from(qb::VectorInput::from(document.clone())))
                        .using(name)
                        .filter(filter.clone())
                        .limit(q.depth as u64)
                        .build()
                })
                .collect::<Vec<_>>();
            // A lexical match can have no vector overlap, as with a prefix.
            // OrderBy contributes candidates only; the formula ignores its score.
            children.push(
                qb::PrefetchQueryBuilder::default()
                    .query(
                        qb::OrderByBuilder::new("__source_version")
                            .direction(qb::Direction::Desc as i32)
                            .build(),
                    )
                    .filter(filter.clone())
                    .limit(q.depth as u64)
                    .build(),
            );
            let weights = settings
                .index_time
                .searchable_attributes
                .iter()
                .map(|attr| attr.weight)
                .collect::<Vec<_>>();
            return Ok(qb::PrefetchQueryBuilder::default()
                .prefetch(children)
                .query(score_formula(&weights))
                .filter(filter)
                .limit(q.depth as u64)
                .build());
        }
        if !native_bm25(settings)
            && let Some(sparse) = self.sparse_query(index, settings, q).await?
        {
            let indices = sparse
                .indices
                .iter()
                .zip(&sparse.values)
                .filter(|(_, value)| **value != 0.0)
                .map(|(id, _)| i64::from(*id))
                .collect::<Vec<_>>();
            let zeros = if indices.is_empty() {
                filter.clone()
            } else {
                QFilter::must([
                    filter.clone().into(),
                    QFilter::must_not([Condition::matches("__sparse_indices", indices)]).into(),
                ])
            };
            let sparse: qb::Query =
                qb::VectorInput::new_sparse(sparse.indices, sparse.values).into();
            // Sparse nearest omits vectors with no common indices. Retain
            // prefix-only or phrase-only lexical matches at native score zero.
            let children = vec![
                qb::PrefetchQueryBuilder::default()
                    .query(sparse)
                    .using("sparse")
                    .filter(filter.clone())
                    .limit(q.depth as u64)
                    .build(),
                qb::PrefetchQueryBuilder::default()
                    .query(
                        qb::OrderByBuilder::new("__source_version")
                            .direction(qb::Direction::Desc as i32)
                            .build(),
                    )
                    .filter(zeros)
                    .limit(q.depth as u64)
                    .build(),
            ];
            return Ok(qb::PrefetchQueryBuilder::default()
                .prefetch(children)
                .query(score_formula(&[1.0]))
                .filter(filter)
                .limit(q.depth as u64)
                .build());
        }
        let mut prefetch = qb::PrefetchQueryBuilder::default()
            .filter(filter)
            .limit(q.depth as u64);
        if let Some(vector) = vector {
            validate_dense(index, settings, vector)?;
            prefetch = prefetch.query(vector.to_vec()).using("dense");
        } else {
            prefetch = prefetch.query(
                qb::OrderByBuilder::new("__source_version")
                    .direction(qb::Direction::Desc as i32)
                    .build(),
            );
        }
        Ok(prefetch.build())
    }
    async fn run_prefetch(
        &self,
        index: &IndexDescriptor,
        collection: &str,
        settings: &IndexSettings,
        mut prefetch: qb::PrefetchQuery,
    ) -> Result<Vec<EngineCandidate>, FrameworkError> {
        let depth = prefetch.limit.unwrap_or(0);
        if depth == 0 {
            return Ok(vec![]);
        }
        if let Some(candidates) = self
            .complete_attribute_scores(index, collection, settings, &mut prefetch)
            .await?
        {
            return Ok(candidates);
        }
        let mut builder = qb::QueryPointsBuilder::new(collection)
            .prefetch(prefetch.prefetch)
            .limit(depth)
            .with_payload(hit_payload(index));
        if let Some(query) = prefetch.query {
            builder = builder.query(query);
        }
        if let Some(filter) = prefetch.filter {
            builder = builder.filter(filter);
        }
        if let Some(using) = prefetch.using {
            builder = builder.using(using);
        }
        self.stable_rank(
            index,
            collection,
            builder.build(),
            usize::try_from(depth)
                .map_err(|_| error(collection, "candidate depth exceeds addressable size"))?,
        )
        .await
    }
    async fn native_sorted(
        &self,
        index: &IndexDescriptor,
        collection: &str,
        settings: &IndexSettings,
        q: &EngineQuery,
        vector: Option<&[f32]>,
        filter: QFilter,
    ) -> Result<Vec<EngineCandidate>, FrameworkError> {
        let count = self.exact_count(collection, filter.clone()).await?.value();
        if count == 0 || q.depth == 0 {
            return Ok(vec![]);
        }
        let mut universe = q.clone();
        universe.depth = usize::try_from(count).map_err(|_| {
            error(
                collection,
                "sorted lexical universe exceeds addressable size",
            )
        })?;
        let prefetch = self
            .lexical_prefetch(index, settings, &universe, vector, filter)
            .await?;
        let mut kept = Vec::new();
        let mut offset = 0u64;
        loop {
            let mut builder = qb::QueryPointsBuilder::new(collection)
                .prefetch(prefetch.prefetch.clone())
                .limit(256)
                .offset(offset)
                .with_payload(hit_payload(index));
            if let Some(query) = &prefetch.query {
                builder = builder.query(query.clone());
            }
            if let Some(filter) = &prefetch.filter {
                builder = builder.filter(filter.clone());
            }
            if let Some(using) = &prefetch.using {
                builder = builder.using(using.clone());
            }
            let response = self
                .ranked_query(builder.build())
                .await
                .map_err(|e| error(collection, e))?;
            let len = response.result.len();
            kept.extend(decode_scored(index, response.result)?);
            kept.sort_by(|a, b| {
                crate::search::builder::compare_sort(&a.document, &b.document, &q.sort)
                    .then_with(|| b.score.total_cmp(&a.score))
                    .then_with(|| a.document.key.cmp(&b.document.key))
            });
            kept.truncate(q.depth);
            if len < 256 {
                break;
            }
            offset += 256;
        }
        Ok(kept)
    }
    async fn lexical_candidates(
        &self,
        index: &IndexDescriptor,
        collection: &str,
        settings: &IndexSettings,
        q: &EngineQuery,
        vector: Option<&[f32]>,
        filter: QFilter,
    ) -> Result<Vec<EngineCandidate>, FrameworkError> {
        let sparse_ranking = !q.expanded.terms.is_empty() || !q.expanded.phrases.is_empty();
        if !q.sort.is_empty() && (sparse_ranking || vector.is_some()) {
            return self
                .native_sorted(index, collection, settings, q, vector, filter)
                .await;
        }
        if q.sort.is_empty() && sparse_ranking {
            let prefetch = self
                .lexical_prefetch(index, settings, q, vector, filter)
                .await?;
            return self
                .run_prefetch(index, collection, settings, prefetch)
                .await;
        }
        if q.sort.is_empty()
            && let Some(vector) = vector
        {
            validate_dense(index, settings, vector)?;
            return self
                .query_points(
                    index,
                    collection,
                    vector.to_vec().into(),
                    Some("dense"),
                    filter,
                    q.depth,
                )
                .await;
        }
        // Streaming top-k covers categorical, multi-column and geo sorts and keeps
        // nulls last. Qdrant's order_by supports only indexed numeric and datetime fields.
        let mut kept = Vec::new();
        let mut offset = None;
        loop {
            let page = self
                .scroll_page(collection, filter.clone(), offset, None)
                .await?;
            for point in page.result {
                let document = decode_document(index, point.payload)?;
                kept.push(EngineCandidate {
                    score: document.source_version as f64,
                    document,
                    highlights: None,
                });
            }
            kept.sort_by(|a, b| {
                crate::search::builder::compare_sort(&a.document, &b.document, &q.sort)
                    .then_with(|| b.document.source_version.cmp(&a.document.source_version))
                    .then_with(|| a.document.key.cmp(&b.document.key))
            });
            kept.truncate(q.depth);
            offset = page.next_page_offset;
            if offset.is_none() {
                break;
            }
        }
        Ok(kept)
    }
    async fn distribution(
        &self,
        index: &IndexDescriptor,
        collection: &str,
        filter: QFilter,
        attr: &str,
        presentation: (usize, FacetOrder),
        prefix: Option<&str>,
    ) -> Result<Vec<crate::search::FacetValue>, FrameworkError> {
        let (cap, order) = presentation;
        let decl = index.attribute(attr)?;
        if !decl.facet {
            return Err(error(index.name, format!("attribute {attr}: not a facet")));
        }
        if cap == 0 {
            return Ok(Vec::new());
        }
        let field = format!("__facet_{attr}");
        let normalized = prefix.map(|p| deunicode::deunicode(p).to_lowercase());
        let filter = if let Some(prefix) = &normalized {
            QFilter::must([
                filter.into(),
                Condition::matches_prefix(format!("__facet_search_{attr}"), prefix.clone()),
            ])
        } else {
            filter
        };
        // A point-level prefix filter can retain nonmatching elements of a
        // list. Grow the facet window until its matching top-cap is complete.
        let mut limit = cap as u64;
        let mut values;
        loop {
            let response = self
                .client
                .facet(
                    qb::FacetCountsBuilder::new(collection, &field)
                        .filter(filter.clone())
                        .limit(limit)
                        .exact(true),
                )
                .await
                .map_err(|e| error(&format!("{collection} attribute {attr}"), e))?;
            let exhausted = (response.hits.len() as u64) < limit;
            values = Vec::new();
            for hit in response.hits {
                let label = match hit.value.and_then(|v| v.variant) {
                    Some(qb::facet_value::Variant::StringValue(v)) => v,
                    _ => {
                        return Err(error(
                            collection,
                            format!("attribute {attr}: facet label missing"),
                        ));
                    }
                };
                if normalized.as_ref().is_some_and(|prefix| {
                    !deunicode::deunicode(&label)
                        .to_lowercase()
                        .starts_with(prefix)
                }) {
                    continue;
                }
                values.push(crate::search::FacetValue {
                    value: facet_value(decl.ty, &label, attr)?,
                    count: hit.count,
                });
            }
            if values.len() >= cap || exhausted {
                break;
            }
            limit = limit.checked_mul(2).ok_or_else(|| {
                error(
                    collection,
                    format!("attribute {attr}: facet search window exceeds limit"),
                )
            })?;
        }
        values.sort_by(|a, b| {
            b.count
                .cmp(&a.count)
                .then_with(|| a.value.facet_key().cmp(&b.value.facet_key()))
        });
        values.truncate(cap);
        if order == FacetOrder::Alphabetical {
            values.sort_by_key(|v| v.value.facet_key());
        }
        Ok(values)
    }
}
fn error(collection: &str, error: impl std::fmt::Display) -> FrameworkError {
    FrameworkError::internal(format!("Qdrant search collection {collection}: {error}"))
}
fn collection_name(
    index: &IndexDescriptor,
    generation: &GenerationId,
) -> Result<String, FrameworkError> {
    index.validate()?;
    GenerationId::new(generation.as_str())?;
    if generation.as_str() == "__control" {
        return Err(FrameworkError::param(
            "Qdrant search generation __control is reserved",
        ));
    }
    Ok(format!("search_{}_{}", index.name, generation.as_str()))
}
fn validate_query(index: &IndexDescriptor, q: &EngineQuery) -> Result<(), FrameworkError> {
    if let Some(filter) = &q.filter {
        filter.validate(index)?;
    }
    for sort in &q.sort {
        let (attr, geo) = match sort {
            Sort::Attribute { attribute, .. } => (attribute, false),
            Sort::Distance {
                attribute, point, ..
            } => {
                point.validate()?;
                (attribute, true)
            }
        };
        let decl = index.attribute(attr)?;
        if !decl.sortable
            || (geo && decl.ty != AttributeType::Geo)
            || (!geo && matches!(decl.ty, AttributeType::Geo | AttributeType::TextList))
        {
            return Err(error(index.name, format!("attribute {attr}: invalid sort")));
        }
    }
    Ok(())
}
fn validate_dense(
    index: &IndexDescriptor,
    settings: &IndexSettings,
    vector: &[f32],
) -> Result<(), FrameworkError> {
    if settings.index_time.dimensions != Some(vector.len())
        || vector.is_empty()
        || vector.iter().any(|n| !n.is_finite())
        || vector.iter().all(|n| *n == 0.0)
    {
        return Err(error(
            index.name,
            "dense vector requires configured dimensions and finite nonzero values",
        ));
    }
    Ok(())
}
struct IndexSchema {
    ty: qb::FieldType,
    params: Option<qb::payload_index_params::IndexParams>,
}
fn keyword() -> IndexSchema {
    IndexSchema {
        ty: qb::FieldType::Keyword,
        params: Some(
            qb::KeywordIndexParamsBuilder::default()
                .is_tenant(false)
                .prefix(true)
                .build()
                .into(),
        ),
    }
}
fn text_params(
    index: &IndexDescriptor,
    settings: &IndexSettings,
) -> Result<qb::TextIndexParams, FrameworkError> {
    let (tokenizer, stemmer, stop_words, folding) = match &settings.index_time.language {
        LanguageConfig::Server => ("word", None, &[][..], true),
        LanguageConfig::Qdrant {
            tokenizer,
            stemmer,
            stop_words,
            folding,
        } => (
            tokenizer.as_str(),
            stemmer.as_deref(),
            stop_words.as_slice(),
            *folding,
        ),
        _ => {
            return Err(error(
                index.name,
                "language configuration must be Qdrant or Server",
            ));
        }
    };
    let tokenizer = match tokenizer {
        "word" => qb::TokenizerType::Word,
        "whitespace" => qb::TokenizerType::Whitespace,
        "prefix" => qb::TokenizerType::Prefix,
        "multilingual" => qb::TokenizerType::Multilingual,
        _ => {
            return Err(error(
                index.name,
                format!("tokenizer {tokenizer}: unsupported"),
            ));
        }
    };
    let mut builder = qb::TextIndexParamsBuilder::new(tokenizer)
        .lowercase(true)
        .ascii_folding(folding)
        .phrase_matching(true)
        .min_token_len(1)
        .stopwords(qb::StopwordsSet {
            languages: vec![],
            custom: stop_words.to_vec(),
        });
    if let Some(language) = stemmer {
        builder = builder.snowball_stemmer(language.to_owned());
    }
    Ok(builder.build())
}
fn schema(
    index: &IndexDescriptor,
    settings: &IndexSettings,
) -> Result<BTreeMap<String, IndexSchema>, FrameworkError> {
    super::collection::validate_settings(index, settings)?;
    for attr in &index.attributes {
        if attr.name.starts_with("__") && attr.name != "__soft_deleted" {
            return Err(error(
                index.name,
                format!("attribute {}: reserved Qdrant payload namespace", attr.name),
            ));
        }
    }
    let text = text_params(index, settings)?;
    let integer = || IndexSchema {
        ty: qb::FieldType::Integer,
        params: Some(
            qb::IntegerIndexParamsBuilder::default()
                .lookup(true)
                .range(true)
                .build()
                .into(),
        ),
    };
    let mut schema = BTreeMap::from([
        ("__key".into(), keyword()),
        ("__source_version".into(), integer()),
        ("__words".into(), keyword()),
        ("__present".into(), keyword()),
        (
            "__text".into(),
            IndexSchema {
                ty: qb::FieldType::Text,
                params: Some(text.clone().into()),
            },
        ),
    ]);
    if !native_bm25(settings) {
        schema.insert("__sparse_indices".into(), integer());
    }
    for attr in &index.attributes {
        if attr.filterable || attr.sortable || attr.facet {
            schema.insert(format!("__value_{}", attr.name), keyword());
            let spec = match attr.ty {
                AttributeType::Text | AttributeType::TextList => keyword(),
                AttributeType::Integer => integer(),
                AttributeType::Float => IndexSchema {
                    ty: qb::FieldType::Float,
                    params: None,
                },
                AttributeType::Bool => IndexSchema {
                    ty: qb::FieldType::Bool,
                    params: None,
                },
                AttributeType::DateTime => IndexSchema {
                    ty: qb::FieldType::Datetime,
                    params: None,
                },
                AttributeType::Geo => IndexSchema {
                    ty: qb::FieldType::Geo,
                    params: None,
                },
            };
            schema.insert(attr.name.clone(), spec);
        }
        if attr.facet {
            schema.insert(format!("__facet_{}", attr.name), keyword());
            schema.insert(format!("__facet_search_{}", attr.name), keyword());
        }
        if attr.searchable.is_some() {
            schema.insert(
                format!("__search_{}", attr.name),
                IndexSchema {
                    ty: qb::FieldType::Text,
                    params: Some(text.clone().into()),
                },
            );
            schema.insert(format!("__words_{}", attr.name), keyword());
        }
    }
    Ok(schema)
}
fn params_match(
    wanted: &qb::payload_index_params::IndexParams,
    actual: Option<&qb::payload_index_params::IndexParams>,
) -> bool {
    use qb::payload_index_params::IndexParams;
    match (wanted, actual) {
        (IndexParams::TextIndexParams(w), Some(IndexParams::TextIndexParams(a))) => {
            w.tokenizer == a.tokenizer
                && w.lowercase == a.lowercase
                && w.ascii_folding == a.ascii_folding
                && w.phrase_matching == a.phrase_matching
                && w.stemmer == a.stemmer
                && w.stopwords.as_ref().is_none_or(|w| {
                    a.stopwords
                        .as_ref()
                        .map_or(w.custom.is_empty() && w.languages.is_empty(), |a| w == a)
                })
                && w.min_token_len == a.min_token_len
        }
        (IndexParams::KeywordIndexParams(w), Some(IndexParams::KeywordIndexParams(a))) => {
            w.is_tenant == a.is_tenant && w.prefix.is_some() == a.prefix.is_some()
        }
        (IndexParams::IntegerIndexParams(w), Some(IndexParams::IntegerIndexParams(a))) => {
            w.lookup == a.lookup && w.range == a.range
        }
        _ => false,
    }
}
fn text(value: &AttributeValue) -> String {
    match value {
        AttributeValue::Text(v) => v.clone(),
        AttributeValue::List(vs) => vs
            .iter()
            .filter_map(AttributeValue::as_text)
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}
// Word and prefix queries split on non-alphanumeric characters in Qdrant 1.19.
// Multilingual segmentation needs Qdrant's language dictionaries. Whitespace is
// only an approximation for its keyword payload and expansion vocabulary; source
// query text still reaches Qdrant intact for native ranking and text predicates.
fn word_slices<'a>(text: &'a str, settings: &IndexSettings) -> Vec<&'a str> {
    match &settings.index_time.language {
        LanguageConfig::Qdrant { tokenizer, .. }
            if matches!(tokenizer.as_str(), "whitespace" | "multilingual") =>
        {
            text.split_whitespace().collect()
        }
        _ => text
            .split(|ch: char| !ch.is_alphanumeric())
            .filter(|word| !word.is_empty())
            .collect(),
    }
}

fn normalize_query_word(word: &str, folding: bool) -> String {
    word.to_lowercase()
        .chars()
        .map(|ch| {
            let latin = matches!(
                ch,
                '\u{00c0}'..='\u{024f}' | '\u{0300}'..='\u{036f}' | '\u{1e00}'..='\u{1eff}'
            );
            if folding && latin {
                deunicode::deunicode(&ch.to_string()).to_lowercase()
            } else {
                ch.to_string()
            }
        })
        .collect()
}

pub(in crate::search) fn query_tokens(
    text: &str,
    settings: &IndexSettings,
) -> Vec<crate::search::expansion::ExpansionToken> {
    let folding = !matches!(
        &settings.index_time.language,
        LanguageConfig::Qdrant { folding: false, .. }
    );
    word_slices(text, settings)
        .into_iter()
        .map(|word| crate::search::expansion::ExpansionToken {
            source: word.into(),
            normalized: normalize_query_word(word, folding),
            source_length: word.chars().count(),
        })
        .collect()
}

pub(in crate::search) fn query_words(text: &str, settings: &IndexSettings) -> Vec<String> {
    let folding = !matches!(
        &settings.index_time.language,
        LanguageConfig::Qdrant { folding: false, .. }
    );
    word_slices(text, settings)
        .into_iter()
        .map(|word| normalize_query_word(word, folding))
        .collect()
}

fn word_list(text: &str, settings: &IndexSettings) -> BTreeSet<String> {
    use unicode_segmentation::UnicodeSegmentation;
    let words = if native_bm25(settings) {
        query_words(text, settings)
    } else if matches!(
        &settings.index_time.language,
        LanguageConfig::Qdrant { folding: false, .. }
    ) {
        text.unicode_words()
            .map(|word| normalize_query_word(word, false))
            .collect()
    } else {
        tokenize(text)
    };
    let stop = match &settings.index_time.language {
        LanguageConfig::Qdrant { stop_words, .. } => stop_words.as_slice(),
        _ => &[],
    };
    words.into_iter().filter(|w| !stop.contains(w)).collect()
}
fn payload(
    index: &IndexDescriptor,
    settings: &IndexSettings,
    doc: &IndexedDocument,
) -> Result<Payload, FrameworkError> {
    let mut data = serde_json::Map::new();
    let mut present = Vec::new();
    for (name, value) in &doc.document.attributes {
        let json = if let AttributeValue::Geo(v) = value {
            json!({"lat":v.lat,"lon":v.lng})
        } else {
            value.to_json()
        };
        data.insert(name.clone(), json);
        if index.attribute(name)?.filterable
            || index.attribute(name)?.sortable
            || index.attribute(name)?.facet
        {
            let label = if matches!(value, AttributeValue::List(_)) {
                crate::search::document::canonical_json(value.to_json())
            } else {
                value.facet_key()
            };
            if !matches!(value, AttributeValue::Null) {
                data.insert(format!("__value_{name}"), json!(label));
            }
        }
        if !matches!(value, AttributeValue::Null) {
            present.push(name.clone());
        }
        if index.attribute(name)?.facet {
            let values = if let AttributeValue::List(values) = value {
                values.iter().collect::<Vec<_>>()
            } else {
                vec![value]
            };
            let labels = values
                .into_iter()
                .filter(|v| !matches!(v, AttributeValue::Null))
                .map(AttributeValue::facet_key)
                .collect::<BTreeSet<_>>();
            data.insert(
                format!("__facet_search_{name}"),
                json!(
                    labels
                        .iter()
                        .map(|v| deunicode::deunicode(v).to_lowercase())
                        .collect::<BTreeSet<_>>()
                ),
            );
            data.insert(format!("__facet_{name}"), json!(labels));
        }
    }
    let mut all = Vec::new();
    for attr in &settings.index_time.searchable_attributes {
        let value = doc
            .document
            .attributes
            .get(&attr.name)
            .ok_or_else(|| error(index.name, format!("attribute {}: missing", attr.name)))?;
        let text = text(value);
        data.insert(
            format!("__words_{}", attr.name),
            json!(word_list(&text, settings)),
        );
        data.insert(format!("__search_{}", attr.name), json!(text));
        all.push(text);
    }
    let joined = all.join("\n");
    data.insert("__text".into(), json!(joined));
    data.insert("__words".into(), json!(word_list(&joined, settings)));
    data.insert("__present".into(), json!(present));
    if !native_bm25(settings)
        && let Some(sparse) = &doc.sparse
    {
        data.insert(
            "__sparse_indices".into(),
            json!(
                sparse
                    .indices
                    .iter()
                    .zip(&sparse.values)
                    .filter(|(_, value)| **value != 0.0)
                    .map(|(id, _)| *id)
                    .collect::<Vec<_>>()
            ),
        );
    }
    data.insert("__key".into(), json!(doc.document.key));
    data.insert(SUPRNOVA_ID_PAYLOAD_KEY.into(), json!(doc.document.key));
    data.insert(
        "__source_version".into(),
        json!(doc.document.source_version),
    );
    data.insert("__content_hash".into(), json!(doc.content_hash));
    data.insert("__embed_text".into(), json!(doc.document.embed_text));
    data.insert("__title".into(), json!(doc.document.title));
    Payload::try_from(Value::Object(data)).map_err(|e| error(index.name, e))
}
/// A hit decodes from these fields alone (see [`decode_document`]). Each
/// point also carries the search copies of its text and its word lists for
/// filtering; selecting the fields keeps those on the server instead of
/// sending them with every candidate of every query.
fn hit_payload(index: &IndexDescriptor) -> qb::with_payload_selector::SelectorOptions {
    let mut fields = vec![
        "__key".to_owned(),
        "__source_version".to_owned(),
        "__embed_text".to_owned(),
        "__title".to_owned(),
    ];
    fields.extend(index.attributes.iter().map(|attr| attr.name.to_owned()));
    qb::with_payload_selector::SelectorOptions::Include(qb::PayloadIncludeSelector { fields })
}
fn decode_document(
    index: &IndexDescriptor,
    payload: HashMap<String, qb::Value>,
) -> Result<SearchDocument, FrameworkError> {
    let data: Value = Payload::from(payload).into();
    let key = data
        .get("__key")
        .and_then(Value::as_str)
        .ok_or_else(|| error(index.name, "__key payload missing"))?
        .to_owned();
    let source_version = data
        .get("__source_version")
        .and_then(Value::as_i64)
        .ok_or_else(|| error(index.name, format!("key {key}: __source_version missing")))?;
    let mut attributes = BTreeMap::new();
    for attr in &index.attributes {
        let mut value = data.get(&attr.name).cloned().unwrap_or(Value::Null);
        if attr.ty == AttributeType::Geo && !value.is_null() {
            value = json!({"lat":value["lat"],"lng":value["lon"]});
        }
        attributes.insert(
            attr.name.clone(),
            AttributeValue::from_json(&value, attr.ty, &attr.name)?,
        );
    }
    let document = SearchDocument {
        index: index.name.into(),
        key,
        source_version,
        attributes,
        embed_text: data
            .get("__embed_text")
            .and_then(Value::as_str)
            .map(str::to_owned),
        title: data
            .get("__title")
            .and_then(Value::as_str)
            .map(str::to_owned),
    };
    document.validate(index)?;
    Ok(document)
}
fn decode_scored(
    index: &IndexDescriptor,
    points: Vec<qb::ScoredPoint>,
) -> Result<Vec<EngineCandidate>, FrameworkError> {
    let mut candidates = points
        .into_iter()
        .map(|p| {
            Ok(EngineCandidate {
                document: decode_document(index, p.payload)?,
                score: f64::from(p.score),
                highlights: None,
            })
        })
        .collect::<Result<Vec<_>, FrameworkError>>()?;
    candidates.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.document.key.cmp(&b.document.key))
    });
    Ok(candidates)
}
fn facet_value(
    ty: AttributeType,
    label: &str,
    attr: &str,
) -> Result<AttributeValue, FrameworkError> {
    let bad = || {
        FrameworkError::internal(format!(
            "Qdrant facet attribute {attr}: invalid label {label}"
        ))
    };
    Ok(match ty {
        AttributeType::Text | AttributeType::TextList => AttributeValue::Text(label.into()),
        AttributeType::Integer => AttributeValue::Integer(label.parse().map_err(|_| bad())?),
        AttributeType::Float => AttributeValue::Float(label.parse().map_err(|_| bad())?),
        AttributeType::Bool => AttributeValue::Bool(label.parse().map_err(|_| bad())?),
        AttributeType::DateTime => AttributeValue::DateTime(
            chrono::DateTime::parse_from_rfc3339(label)
                .map_err(|_| bad())?
                .with_timezone(&chrono::Utc),
        ),
        AttributeType::Geo => {
            let (lat, lng) = label.split_once(',').ok_or_else(bad)?;
            AttributeValue::from_json(
                &json!({"lat":lat.parse::<f64>().map_err(|_|bad())?,"lng":lng.parse::<f64>().map_err(|_|bad())?}),
                AttributeType::Geo,
                attr,
            )?
        }
    })
}

#[async_trait]
impl SearchEngine for QdrantSearchEngine {
    fn advances_render_cache(&self) -> bool {
        true
    }
    fn name(&self) -> &str {
        "qdrant"
    }
    fn capabilities(&self) -> EngineCapabilities {
        EngineCapabilities {
            native_highlight: false,
            native_crop: false,
            sparse: true,
            phrase: true,
            prefix: true,
            semantic: true,
            facets: true,
            geo: true,
            versioned: true,
            generations: true,
        }
    }
    async fn current_generation(
        &self,
        index: &str,
    ) -> Result<Option<GenerationId>, FrameworkError> {
        storage::current(index).await
    }
    async fn create_generation(
        &self,
        index: &IndexDescriptor,
        settings: &IndexSettings,
        generation: &GenerationId,
    ) -> Result<(), FrameworkError> {
        crate::search::render_cache::coordinated_write(
            index.name,
            Box::pin(async {
                let _lock = self.mutations.lock().await;
                let collection = collection_name(index, generation)?;
                let schema = schema(index, settings)?;
                let mut vectors = qb::VectorsConfigBuilder::default();
                vectors.add_named_vector_params(
                    "dense",
                    qb::VectorParamsBuilder::new(
                        settings.index_time.dimensions.unwrap_or(1) as u64,
                        qb::Distance::Cosine,
                    ),
                );
                let mut sparse = qb::SparseVectorsConfigBuilder::default();
                for name in sparse_names(settings) {
                    sparse.add_named_vector_params(
                        name,
                        qb::SparseVectorParamsBuilder::default().modifier(qb::Modifier::Idf),
                    );
                }
                let builder = qb::CreateCollectionBuilder::new(&collection)
                    .vectors_config(vectors)
                    .sparse_vectors_config(sparse);
                // A retry completes a partially created collection, but never mutates
                // the index-time identity of an already recorded generation.
                if let Some(old) = storage::settings_optional(index.name, generation).await? {
                    if old != *settings {
                        return Err(error(
                            &collection,
                            "generation already has different settings",
                        ));
                    }
                    return self.check_schema(index, generation, &old).await;
                }
                if !self
                    .client
                    .collection_exists(&collection)
                    .await
                    .map_err(|e| error(&collection, e))?
                {
                    self.client
                        .create_collection(builder)
                        .await
                        .map_err(|e| error(&collection, e))?;
                }
                self.apply_indexes(&collection, &schema).await?;
                self.check_schema(index, generation, settings).await?;
                storage::create(index.name, generation, settings).await
            }),
        )
        .await
    }
    async fn switch_generation(
        &self,
        index: &IndexDescriptor,
        from: Option<GenerationId>,
        to: GenerationId,
    ) -> Result<(), FrameworkError> {
        crate::search::render_cache::coordinated_write(
            index.name,
            Box::pin(async {
                let _lock = self.mutations.lock().await;
                self.read_settings(index, &to).await?;
                storage::switch(index.name, from, &to).await
            }),
        )
        .await
    }
    async fn drop_generation(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
    ) -> Result<(), FrameworkError> {
        crate::search::render_cache::coordinated_write(
            index.name,
            Box::pin(async {
                let _lock = self.mutations.lock().await;
                let collection = collection_name(index, generation)?;
                let txn = storage::locked(index.name, generation).await?;
                if storage::current_locked(&txn, index.name).await?.as_ref() == Some(generation) {
                    return Err(error(&collection, "cannot drop the current generation"));
                }
                if self
                    .client
                    .collection_exists(&collection)
                    .await
                    .map_err(|e| error(&collection, e))?
                {
                    self.client
                        .delete_collection(&collection)
                        .await
                        .map_err(|e| error(&collection, e))?;
                }
                storage::delete_metadata(txn, index.name, generation).await
            }),
        )
        .await
    }
    async fn write_settings(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        settings: &IndexSettings,
    ) -> Result<(), FrameworkError> {
        crate::search::render_cache::coordinated_write(
            index.name,
            Box::pin(async {
                let _lock = self.mutations.lock().await;
                schema(index, settings)?;
                self.check_schema(index, generation, settings).await?;
                storage::write_settings(index.name, generation, settings).await
            }),
        )
        .await
    }
    async fn read_settings(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
    ) -> Result<IndexSettings, FrameworkError> {
        collection_name(index, generation)?;
        let settings = storage::settings(index.name, generation).await?;
        self.check_schema(index, generation, &settings).await?;
        Ok(settings)
    }
    async fn stored_hashes(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        documents: &[IndexedDocument],
    ) -> Result<BTreeMap<String, String>, FrameworkError> {
        let collection = collection_name(index, generation)?;
        storage::settings(index.name, generation).await?;
        let mut hashes = BTreeMap::new();
        for batch in documents.chunks(256) {
            let points = self
                .client
                .get_points(
                    qb::GetPointsBuilder::new(
                        &collection,
                        batch
                            .iter()
                            .map(|d| QdrantVectorDriver::resolve_point_id(&d.document.key))
                            .collect::<Vec<_>>(),
                    )
                    .with_payload(
                        qb::with_payload_selector::SelectorOptions::Include(
                            qb::PayloadIncludeSelector {
                                fields: vec!["__key".into(), "__content_hash".into()],
                            },
                        ),
                    ),
                )
                .await
                .map_err(|e| error(&collection, e))?;
            for point in points.result {
                let data: Value = Payload::from(point.payload).into();
                let key = data["__key"]
                    .as_str()
                    .ok_or_else(|| error(&collection, "stored key missing"))?;
                let hash = data["__content_hash"]
                    .as_str()
                    .ok_or_else(|| error(&collection, "stored content hash missing"))?;
                hashes.insert(key.to_owned(), hash.to_owned());
            }
        }
        Ok(hashes)
    }
    async fn index(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        docs: &[IndexedDocument],
    ) -> Result<WriteOutcome, FrameworkError> {
        crate::search::render_cache::coordinated_write(
            index.name,
            Box::pin(async {
                let _lock = self.mutations.lock().await;
                let collection = collection_name(index, generation)?;
                let settings = self.read_settings(index, generation).await?;
                let bm25 = if native_bm25(&settings) {
                    Some(bm25_document(index, &settings, String::new())?)
                } else {
                    None
                };
                // Validate the complete batch before the first persistent change.
                for doc in docs {
                    doc.document.validate(index)?;
                    if doc.document.key.chars().count() > 255 {
                        return Err(FrameworkError::param(format!(
                            "search index {} key: exceeds 255 characters",
                            index.name
                        )));
                    }
                    let data: Value = payload(index, &settings, doc)?.into();
                    if payload_words(Some(&data))
                        .iter()
                        .any(|word| word.chars().count() > 255)
                    {
                        return Err(FrameworkError::param(format!(
                            "search index {}: vocabulary word exceeds 255 characters",
                            index.name
                        )));
                    }
                    if let Some(dense) = &doc.dense {
                        validate_dense(index, &settings, dense)?;
                    }
                    if let Some(sparse) = &doc.sparse {
                        sparse.validate()?;
                    }
                    if bm25.is_none() && doc.sparse.is_none() {
                        return Err(error(
                            &collection,
                            format!(
                                "key {}: sparse model requires a document embedding",
                                doc.document.key
                            ),
                        ));
                    }
                }
                let txn = storage::locked(index.name, generation).await?;
                let mut outcome = WriteOutcome::default();
                for doc in docs {
                    let key = &doc.document.key;
                    if storage::tombstone_on(&txn, index.name, generation, key)
                        .await?
                        .is_some_and(|v| v >= doc.document.source_version)
                    {
                        outcome.skipped_tombstoned += 1;
                        continue;
                    }
                    let old = self
                        .get(&collection, key)
                        .await?
                        .map(|p| Value::from(Payload::from(p)));
                    if old
                        .as_ref()
                        .and_then(|v| v["__source_version"].as_i64())
                        .is_some_and(|v| v > doc.document.source_version)
                    {
                        outcome.skipped_older += 1;
                        continue;
                    }
                    if old.as_ref().and_then(|v| v["__content_hash"].as_str())
                        == Some(doc.content_hash.as_str())
                    {
                        // Freshness advances even when content is unchanged. This keeps
                        // an older removal from deleting a later unchanged save.
                        self.client
                            .set_payload(
                                qb::SetPayloadPointsBuilder::new(
                                    &collection,
                                    Payload::try_from(
                                        json!({"__source_version":doc.document.source_version}),
                                    )
                                    .map_err(|e| error(&collection, e))?,
                                )
                                .points_selector(vec![QdrantVectorDriver::resolve_point_id(key)])
                                .wait(true),
                            )
                            .await
                            .map_err(|e| error(&collection, e))?;
                        outcome.skipped_unchanged += 1;
                        continue;
                    }
                    let mut vectors = HashMap::new();
                    if let Some(dense) = &doc.dense {
                        vectors.insert("dense".into(), qb::Vector::new_dense(dense.clone()));
                    }
                    if let Some(bm25) = &bm25 {
                        for attr in &settings.index_time.searchable_attributes {
                            let value =
                                doc.document.attributes.get(&attr.name).ok_or_else(|| {
                                    error(index.name, format!("attribute {}: missing", attr.name))
                                })?;
                            vectors.insert(
                                format!("bm25_{}", attr.name),
                                qb::Document {
                                    text: text(value),
                                    ..bm25.clone()
                                }
                                .into(),
                            );
                        }
                    } else if let Some(sparse) = &doc.sparse {
                        vectors.insert(
                            "sparse".into(),
                            qb::Vector::new_sparse(sparse.indices.clone(), sparse.values.clone()),
                        );
                    }
                    let point = qb::PointStruct::new(
                        QdrantVectorDriver::resolve_point_id(key),
                        qb::NamedVectors { vectors },
                        payload(index, &settings, doc)?,
                    );
                    self.client
                        .upsert_points(
                            qb::UpsertPointsBuilder::new(&collection, vec![point]).wait(true),
                        )
                        .await
                        .map_err(|e| error(&collection, e))?;
                    let data: Value = payload(index, &settings, doc)?.into();
                    storage::update_vocabulary(
                        &txn,
                        index.name,
                        generation,
                        &payload_words(old.as_ref()),
                        &payload_words(Some(&data)),
                    )
                    .await?;
                    outcome.written += 1;
                }
                storage::commit_write(txn, index.name).await?;
                Ok(outcome)
            }),
        )
        .await
    }
    async fn remove(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        keys: &[(String, i64)],
    ) -> Result<WriteOutcome, FrameworkError> {
        crate::search::render_cache::coordinated_write(
            index.name,
            Box::pin(async {
                let _lock = self.mutations.lock().await;
                let collection = collection_name(index, generation)?;
                storage::settings(index.name, generation).await?;
                if keys
                    .iter()
                    .any(|(key, _)| key.is_empty() || key.chars().count() > 255)
                {
                    return Err(FrameworkError::param(format!(
                        "search index {}: removal key requires 1 to 255 characters",
                        index.name
                    )));
                }
                let mut outcome = WriteOutcome::default();
                for (key, version) in keys {
                    let txn = storage::locked(index.name, generation).await?;
                    let old = self
                        .get(&collection, key)
                        .await?
                        .map(|p| Value::from(Payload::from(p)));
                    if old
                        .as_ref()
                        .and_then(|p| p["__source_version"].as_i64())
                        .is_some_and(|v| v > *version)
                        || storage::tombstone_on(&txn, index.name, generation, key)
                            .await?
                            .is_some_and(|v| v > *version)
                    {
                        storage::commit_write(txn, index.name).await?;
                        outcome.skipped_older += 1;
                        continue;
                    }
                    // Commit the tombstone before delete so a crash cannot lose it.
                    storage::record_tombstone_on(&txn, index.name, generation, key, *version)
                        .await?;
                    storage::commit_write(txn, index.name).await?;
                    // Reacquire the shared lock and reread: a newer save may have
                    // legitimately replaced this tombstone while it was released.
                    let txn = storage::locked(index.name, generation).await?;
                    let old = self
                        .get(&collection, key)
                        .await?
                        .map(|p| Value::from(Payload::from(p)));
                    if old
                        .as_ref()
                        .and_then(|p| p["__source_version"].as_i64())
                        .is_some_and(|v| v > *version)
                    {
                        storage::commit_write(txn, index.name).await?;
                        outcome.skipped_older += 1;
                        continue;
                    }
                    self.client
                        .delete_points(
                            qb::DeletePointsBuilder::new(&collection)
                                .points(vec![QdrantVectorDriver::resolve_point_id(key)])
                                .wait(true),
                        )
                        .await
                        .map_err(|e| error(&collection, e))?;
                    storage::update_vocabulary(
                        &txn,
                        index.name,
                        generation,
                        &payload_words(old.as_ref()),
                        &BTreeSet::new(),
                    )
                    .await?;
                    storage::commit_write(txn, index.name).await?;
                    outcome.written += 1;
                }
                Ok(outcome)
            }),
        )
        .await
    }
    async fn query(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        q: &EngineQuery,
    ) -> Result<EngineResult, FrameworkError> {
        let collection = collection_name(index, generation)?;
        let settings = self.read_settings(index, generation).await?;
        let filter = self.query_filter(index, &settings, q, &collection).await?;
        let total = self.exact_count(&collection, filter.clone()).await?;
        let candidates = self
            .lexical_candidates(index, &collection, &settings, q, None, filter)
            .await?;
        Ok(EngineResult { candidates, total })
    }
    async fn semantic(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        vector: &[f32],
        filter: Option<&Filter>,
        depth: usize,
    ) -> Result<EngineResult, FrameworkError> {
        let collection = collection_name(index, generation)?;
        let settings = self.read_settings(index, generation).await?;
        validate_dense(index, &settings, vector)?;
        if let Some(filter) = filter {
            filter.validate(index)?;
        }
        let filter = QFilter::must(match filter {
            Some(filter) => Some(self.bound_predicate(&collection, filter).await?),
            None => None,
        });
        let candidates = self
            .query_points(
                index,
                &collection,
                vector.to_vec().into(),
                Some("dense"),
                filter,
                depth,
            )
            .await?;
        let total = Total::Estimated(candidates.len() as u64);
        Ok(EngineResult { candidates, total })
    }
    async fn hybrid(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        q: &EngineQuery,
        vector: &[f32],
        ratio: f32,
    ) -> Result<Option<EngineResult>, FrameworkError> {
        if !ratio.is_finite() || !(0.0..=1.0).contains(&ratio) {
            return Err(error(index.name, "hybrid ratio must be between 0 and 1"));
        }
        let collection = collection_name(index, generation)?;
        let settings = self.read_settings(index, generation).await?;
        validate_dense(index, &settings, vector)?;
        let lexical = self.query_filter(index, &settings, q, &collection).await?;
        if ratio == 0.0 {
            let total = self.exact_count(&collection, lexical.clone()).await?;
            let candidates = self
                .lexical_candidates(index, &collection, &settings, q, Some(vector), lexical)
                .await?;
            return Ok(Some(EngineResult { candidates, total }));
        }
        if q.depth == 0 {
            return Ok(Some(EngineResult {
                candidates: vec![],
                total: Total::Estimated(0),
            }));
        }
        let mut scope = QFilter::must(match &q.filter {
            Some(filter) => Some(self.bound_predicate(&collection, filter).await?),
            None => None,
        });
        scope.must_not.extend(
            q.expanded
                .negatives
                .iter()
                .map(|word| Condition::matches_text("__text", word.clone())),
        );
        if ratio == 1.0 {
            let candidates = self
                .query_points(
                    index,
                    &collection,
                    vector.to_vec().into(),
                    Some("dense"),
                    scope,
                    q.depth,
                )
                .await?;
            let total = Total::Estimated(candidates.len() as u64);
            return Ok(Some(EngineResult { candidates, total }));
        }
        let mut lexical_prefetch = self
            .lexical_prefetch(index, &settings, q, Some(vector), lexical.clone())
            .await?;
        self.complete_attribute_scores(index, &collection, &settings, &mut lexical_prefetch)
            .await?;
        let prefetch = vec![
            lexical_prefetch,
            qb::PrefetchQueryBuilder::default()
                .query(vector.to_vec())
                .using("dense")
                .filter(scope.clone())
                .limit(q.depth as u64)
                .build(),
        ];
        let query = qb::Query {
            variant: Some(qb::query::Variant::Rrf(qb::Rrf {
                k: Some(60),
                weights: vec![1.0 - ratio, ratio],
            })),
        };
        let response = self
            .ranked_query(
                qb::QueryPointsBuilder::new(&collection)
                    .prefetch(prefetch.clone())
                    .query(query)
                    .filter(scope.clone())
                    .limit(q.depth.saturating_mul(2) as u64)
                    .with_payload(hit_payload(index))
                    .build(),
            )
            .await;
        let candidates = match response {
            Ok(response) => decode_scored(index, response.result)?,
            Err(e) if unsupported_weights(&e) => {
                // Only unsupported fusion syntax triggers fallback. Transport,
                // authentication and collection errors remain visible.
                self.ranked_query(
                    qb::QueryPointsBuilder::new(&collection)
                        .prefetch(prefetch)
                        .query(qb::Fusion::Rrf)
                        .filter(scope.clone())
                        .limit(q.depth as u64)
                        .build(),
                )
                .await
                .map_err(|e| error(&collection, e))?;
                let left = self
                    .lexical_candidates(index, &collection, &settings, q, Some(vector), lexical)
                    .await?;
                let right = self
                    .query_points(
                        index,
                        &collection,
                        vector.to_vec().into(),
                        Some("dense"),
                        scope,
                        q.depth,
                    )
                    .await?;
                let ranks = crate::search::weighted_rrf(&[
                    (
                        f64::from(1.0 - ratio),
                        left.iter().map(|c| c.document.key.clone()).collect(),
                    ),
                    (
                        f64::from(ratio),
                        right.iter().map(|c| c.document.key.clone()).collect(),
                    ),
                ]);
                let mut candidates = left
                    .into_iter()
                    .chain(right)
                    .map(|c| (c.document.key.clone(), c))
                    .collect::<HashMap<_, _>>();
                ranks
                    .into_iter()
                    .filter_map(|(key, score)| {
                        candidates.remove(&key).map(|mut c| {
                            c.score = score;
                            c
                        })
                    })
                    .collect()
            }
            Err(e) => return Err(error(&collection, e)),
        };
        let total = Total::Estimated(candidates.len() as u64);
        Ok(Some(EngineResult { candidates, total }))
    }
    async fn facets(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        q: &EngineQuery,
        attrs: &[String],
        cap: usize,
        order: FacetOrder,
    ) -> Result<FacetDistribution, FrameworkError> {
        let collection = collection_name(index, generation)?;
        let settings = self.read_settings(index, generation).await?;
        let filter = self.query_filter(index, &settings, q, &collection).await?;
        let mut result = BTreeMap::new();
        for attr in attrs {
            result.insert(
                attr.clone(),
                self.distribution(index, &collection, filter.clone(), attr, (cap, order), None)
                    .await?,
            );
        }
        Ok(result)
    }
    async fn facet_stats(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        q: &EngineQuery,
        attrs: &[String],
    ) -> Result<FacetStats, FrameworkError> {
        let collection = collection_name(index, generation)?;
        let settings = self.read_settings(index, generation).await?;
        let filter = self.query_filter(index, &settings, q, &collection).await?;
        let mut result = BTreeMap::new();
        for attr in attrs {
            let decl = index.attribute(attr)?;
            if !decl.facet
                || !matches!(
                    decl.ty,
                    AttributeType::Integer | AttributeType::Float | AttributeType::DateTime
                )
            {
                return Err(error(
                    index.name,
                    format!("attribute {attr}: stats require a numeric or datetime facet"),
                ));
            }
            let mut extrema = Vec::new();
            for direction in [qb::Direction::Asc, qb::Direction::Desc] {
                let response = self
                    .client
                    .scroll(
                        qb::ScrollPointsBuilder::new(&collection)
                            .filter(filter.clone())
                            .order_by(
                                qb::OrderByBuilder::new(attr.clone()).direction(direction as i32),
                            )
                            .limit(1)
                            .with_payload(qb::with_payload_selector::SelectorOptions::Include(
                                qb::PayloadIncludeSelector {
                                    fields: vec![attr.clone()],
                                },
                            )),
                    )
                    .await
                    .map_err(|e| error(&collection, e))?;
                if let Some(point) = response.result.into_iter().next() {
                    let data: Value = Payload::from(point.payload).into();
                    extrema.push(AttributeValue::from_json(&data[attr], decl.ty, attr)?);
                }
            }
            if extrema.len() == 2 {
                result.insert(
                    attr.clone(),
                    crate::search::FacetRange {
                        min: extrema.remove(0),
                        max: extrema.remove(0),
                    },
                );
            }
        }
        Ok(result)
    }
    async fn facet_search(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        q: &EngineQuery,
        attr: &str,
        prefix: &str,
        cap: usize,
    ) -> Result<FacetDistribution, FrameworkError> {
        let collection = collection_name(index, generation)?;
        let settings = self.read_settings(index, generation).await?;
        let filter = self.query_filter(index, &settings, q, &collection).await?;
        Ok(BTreeMap::from([(
            attr.to_owned(),
            self.distribution(
                index,
                &collection,
                filter,
                attr,
                (cap, FacetOrder::Count),
                Some(prefix),
            )
            .await?,
        )]))
    }
    async fn vocabulary(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
    ) -> Result<Arc<Vocabulary>, FrameworkError> {
        let collection = collection_name(index, generation)?;
        storage::settings(index.name, generation).await?;
        let _lock = self.mutations.lock().await;
        // The render already owns its SQL snapshot connection. Rebuilding
        // derived vocabulary must not acquire a second transaction inside it.
        let txn = if crate::search::render_cache::accounted_read() {
            None
        } else {
            Some(storage::locked(index.name, generation).await?)
        };
        let mut vocabulary = Vocabulary::default();
        let mut offset = None;
        loop {
            let page = self
                .scroll_page(
                    &collection,
                    QFilter::default(),
                    offset,
                    Some(vec!["__words".into()]),
                )
                .await?;
            for point in page.result {
                let data: Value = Payload::from(point.payload).into();
                if let Some(words) = data["__words"].as_array() {
                    for word in words
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<BTreeSet<_>>()
                    {
                        *vocabulary.words.entry(word.into()).or_default() += 1;
                    }
                }
            }
            offset = page.next_page_offset;
            if offset.is_none() {
                break;
            }
        }
        let vocabulary = if let Some(txn) = txn {
            storage::replace_vocabulary(&txn, index.name, generation, &vocabulary).await?;
            let refreshed = storage::vocabulary_on(&txn, index.name, generation).await?;
            storage::commit(txn, index.name).await?;
            refreshed
        } else {
            vocabulary
        };
        Ok(Arc::new(vocabulary))
    }
    async fn count(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        q: &EngineQuery,
    ) -> Result<Total, FrameworkError> {
        let collection = collection_name(index, generation)?;
        let settings = self.read_settings(index, generation).await?;
        self.exact_count(
            &collection,
            self.query_filter(index, &settings, q, &collection).await?,
        )
        .await
    }
}
fn unsupported_weights(error: &QdrantError) -> bool {
    let text = error.to_string().to_lowercase();
    (text.contains("unimplemented") || text.contains("invalid argument"))
        && (text.contains("rrf") || text.contains("weights") || text.contains("fusion"))
}

fn payload_type(ty: qb::FieldType) -> qb::PayloadSchemaType {
    match ty {
        qb::FieldType::Keyword => qb::PayloadSchemaType::Keyword,
        qb::FieldType::Integer => qb::PayloadSchemaType::Integer,
        qb::FieldType::Float => qb::PayloadSchemaType::Float,
        qb::FieldType::Bool => qb::PayloadSchemaType::Bool,
        qb::FieldType::Datetime => qb::PayloadSchemaType::Datetime,
        qb::FieldType::Geo => qb::PayloadSchemaType::Geo,
        qb::FieldType::Text => qb::PayloadSchemaType::Text,
        qb::FieldType::Uuid => qb::PayloadSchemaType::Uuid,
    }
}

fn dictionary_range(filter: &Filter, value: &AttributeValue) -> bool {
    let exact = |v: &AttributeValue| {
        matches!(v, AttributeValue::Text(_))
            || matches!(v,AttributeValue::Integer(n) if n.unsigned_abs()>=9_007_199_254_740_992)
    };
    exact(value) || matches!(filter,Filter::Between(_,_,hi) if exact(hi))
}

pub(in crate::search) fn native_bm25(settings: &IndexSettings) -> bool {
    settings
        .index_time
        .sparse_model
        .as_deref()
        .is_none_or(|name| name == "bm25")
}

fn sparse_names(settings: &IndexSettings) -> Vec<String> {
    if native_bm25(settings) {
        settings
            .index_time
            .searchable_attributes
            .iter()
            .map(|attr| format!("bm25_{}", attr.name))
            .collect()
    } else {
        vec!["sparse".into()]
    }
}

fn lexical_text(q: &EngineQuery) -> String {
    q.expanded
        .terms
        .iter()
        .flat_map(|t| t.alternatives.iter().chain(std::iter::once(&t.original)))
        .chain(q.expanded.phrases.iter().flatten())
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
        .join(" ")
}

// One constructor keeps ingest and query inference in the same token space.
fn bm25_document(
    index: &IndexDescriptor,
    settings: &IndexSettings,
    text: String,
) -> Result<qb::Document, FrameworkError> {
    text_params(index, settings)?;
    let (tokenizer, stemmer, stop_words, folding) = match &settings.index_time.language {
        LanguageConfig::Qdrant {
            tokenizer,
            stemmer,
            stop_words,
            folding,
        } => (
            tokenizer.as_str(),
            stemmer.as_deref(),
            stop_words.as_slice(),
            *folding,
        ),
        _ => ("word", None, &[][..], true),
    };
    let stemmer = match stemmer {
        Some(language) => json!({"type":"snowball", "language":language}),
        None => json!({"type":"none"}),
    };
    let stopwords = if stop_words.is_empty() {
        json!({})
    } else {
        json!({"custom":stop_words})
    };
    Ok(qb::Document {
        text,
        model: "qdrant/bm25".into(),
        options: HashMap::from([
            ("tokenizer".into(), qb::Value::from(tokenizer)),
            ("stemmer".into(), qb::Value::from(stemmer)),
            ("stopwords".into(), qb::Value::from(stopwords)),
            ("ascii_folding".into(), qb::Value::from(folding)),
            (
                "avg_len".into(),
                qb::Value::from(crate::search::SearchConfig::current()?.bm25_average_length as f64),
            ),
        ]),
    })
}

fn score_formula(weights: &[f32]) -> qb::Query {
    let mut defaults = HashMap::new();
    let sum = weights
        .iter()
        .enumerate()
        .map(|(i, weight)| {
            let variable = format!("$score[{i}]");
            defaults.insert(variable.clone(), qb::Value::from(0.0));
            qb::Expression {
                variant: Some(qb::expression::Variant::Mult(qb::MultExpression {
                    mult: vec![
                        qb::Expression {
                            variant: Some(qb::expression::Variant::Constant(*weight)),
                        },
                        qb::Expression {
                            variant: Some(qb::expression::Variant::Variable(variable)),
                        },
                    ],
                })),
            }
        })
        .collect();
    qb::Formula {
        expression: Some(qb::Expression {
            variant: Some(qb::expression::Variant::Sum(qb::SumExpression { sum })),
        }),
        defaults,
    }
    .into()
}

fn grow_prefetch(prefetches: &mut [qb::PrefetchQuery], limit: u64, exact: bool) {
    for prefetch in prefetches {
        prefetch.limit = Some(prefetch.limit.unwrap_or(0).max(limit));
        if exact
            && matches!(
                prefetch.query.as_ref().and_then(|q| q.variant.as_ref()),
                Some(qb::query::Variant::Nearest(_))
            )
        {
            prefetch.params.get_or_insert_default().exact = Some(true);
        }
        grow_prefetch(&mut prefetch.prefetch, limit, exact);
    }
}

fn payload_words(payload: Option<&Value>) -> BTreeSet<String> {
    payload
        .and_then(|p| p["__words"].as_array())
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

#[cfg(any(test, feature = "testing"))]
fn attribute_limit(prefetches: &[qb::PrefetchQuery]) -> u64 {
    prefetches
        .iter()
        .map(|p| {
            let own = p
                .using
                .as_deref()
                .filter(|name| name.starts_with("bm25_"))
                .map_or(0, |_| p.limit.unwrap_or(0));
            own.max(attribute_limit(&p.prefetch))
        })
        .max()
        .unwrap_or(0)
}

#[cfg(test)]
mod token_tests {
    use super::*;
    use crate::search::{QueryText, Synonym, Vocabulary, expansion::expand_with};

    #[test]
    fn keyword_words_follow_the_configured_token_boundaries() {
        for (tokenizer, expected) in [
            ("word", vec!["cafe", "北京", "rust", "lang", "o", "reilly"]),
            (
                "prefix",
                vec!["cafe", "北京", "rust", "lang", "o", "reilly"],
            ),
            ("whitespace", vec!["cafe", "北京", "rust-lang", "o'reilly"]),
            (
                "multilingual",
                vec!["cafe", "北京", "rust-lang", "o'reilly"],
            ),
        ] {
            let mut settings = IndexSettings::default();
            settings.index_time.language = LanguageConfig::Qdrant {
                tokenizer: tokenizer.into(),
                stemmer: None,
                stop_words: vec![],
                folding: true,
            };
            assert_eq!(
                query_words("Café 北京 rust-lang O'Reilly", &settings),
                expected,
                "{tokenizer}"
            );
            assert_eq!(
                word_list("Café 北京 rust-lang O'Reilly", &settings),
                expected.into_iter().map(str::to_owned).collect(),
                "{tokenizer}"
            );
            if let LanguageConfig::Qdrant { folding, .. } = &mut settings.index_time.language {
                *folding = false;
            }
            assert_eq!(query_words("Café", &settings), ["café"]);
        }
    }

    #[test]
    fn native_expansion_keeps_source_text_alongside_vocabulary_alternatives() {
        let mut settings = IndexSettings::default();
        settings.index_time.language = LanguageConfig::Qdrant {
            tokenizer: "whitespace".into(),
            stemmer: None,
            stop_words: vec![],
            folding: true,
        };
        settings.query.synonyms.push(Synonym {
            terms: vec!["cafe".into(), "coffee".into()],
            one_way: false,
        });
        let vocabulary = Vocabulary {
            words: [("rust-lang".into(), 1)].into_iter().collect(),
        };
        let query = QueryText::parse("Café rust-lagn \"北京 rust-lang\" -RUST-LANG");
        let expanded = expand_with(&query, &settings.query, &vocabulary, 64, &|text| {
            query_tokens(text, &settings)
        })
        .unwrap();
        assert_eq!(expanded.terms[0].original, "Café");
        assert_eq!(expanded.terms[0].alternatives, ["Café", "coffee"]);
        assert_eq!(expanded.terms[1].alternatives, ["rust-lagn", "rust-lang"]);
        assert_eq!(expanded.terms[1].typo_alternatives, ["rust-lang"]);
        assert_eq!(expanded.phrases, vec![vec!["北京", "rust-lang"]]);
        assert_eq!(expanded.negatives, ["RUST-LANG"]);
    }
}
