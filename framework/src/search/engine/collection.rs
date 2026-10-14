//! A deterministic in-memory BM25 driver supports tests and small local indexes.
use super::*;
use crate::search::{
    document::{AttributeType, AttributeValue},
    expansion::{ExpandedQuery, tokenize},
    query::{Sort, SortDirection},
    results::{FacetRange, FacetValue},
};
use std::{
    cmp::Ordering,
    collections::{BTreeSet, HashMap},
    sync::{OnceLock, RwLock, RwLockReadGuard, RwLockWriteGuard},
    time::{Duration, Instant},
};
#[derive(Default)]
struct State {
    indexes: HashMap<String, IndexState>,
}
#[derive(Default)]
struct IndexState {
    current: Option<GenerationId>,
    generations: BTreeMap<GenerationId, Generation>,
}
struct Generation {
    state: &'static str,
    settings: IndexSettings,
    documents: BTreeMap<String, IndexedDocument>,
    tokenized: OnceLock<BTreeMap<String, Vec<Field>>>,
    tombstones: BTreeMap<String, (i64, Instant)>,
    flush: Option<(i64, Instant)>,
}
/// In-memory storage keeps generations isolated and locks each mutation atomically.
pub struct CollectionEngine {
    state: RwLock<State>,
    tombstone_retention: Duration,
}
impl Default for CollectionEngine {
    fn default() -> Self {
        Self::new()
    }
}
impl CollectionEngine {
    /// The standard retention matches SEARCH_TOMBSTONE_HOURS's default.
    pub fn new() -> Self {
        Self {
            state: RwLock::new(State::default()),
            tombstone_retention: Duration::from_secs(24 * 3600),
        }
    }
    /// Explicit retention lets applications apply their configured tombstone window.
    pub fn with_tombstone_hours(hours: u64) -> Result<Self, FrameworkError> {
        let seconds = hours.checked_mul(3600).ok_or_else(|| {
            FrameworkError::param("SEARCH_TOMBSTONE_HOURS: exceeds duration range")
        })?;
        Ok(Self {
            state: RwLock::new(State::default()),
            tombstone_retention: Duration::from_secs(seconds),
        })
    }
    fn read(&self) -> Result<RwLockReadGuard<'_, State>, FrameworkError> {
        self.state
            .read()
            .map_err(|_| FrameworkError::internal("collection search engine: state lock poisoned"))
    }
    fn write(&self) -> Result<RwLockWriteGuard<'_, State>, FrameworkError> {
        self.state
            .write()
            .map_err(|_| FrameworkError::internal("collection search engine: state lock poisoned"))
    }
}
fn generation<'a>(
    state: &'a State,
    index: &IndexDescriptor,
    id: &GenerationId,
) -> Result<&'a Generation, FrameworkError> {
    index.validate()?;
    GenerationId::new(id.0.clone())?;
    state
        .indexes
        .get(index.name)
        .and_then(|i| i.generations.get(id))
        .ok_or_else(|| {
            FrameworkError::param(format!(
                "search index {} generation {}: does not exist",
                index.name, id.0
            ))
        })
}
fn generation_mut<'a>(
    state: &'a mut State,
    index: &IndexDescriptor,
    id: &GenerationId,
) -> Result<&'a mut Generation, FrameworkError> {
    index.validate()?;
    GenerationId::new(id.0.clone())?;
    state
        .indexes
        .get_mut(index.name)
        .and_then(|i| i.generations.get_mut(id))
        .ok_or_else(|| {
            FrameworkError::param(format!(
                "search index {} generation {}: does not exist",
                index.name, id.0
            ))
        })
}
pub(super) fn validate_settings(
    index: &IndexDescriptor,
    settings: &IndexSettings,
) -> Result<(), FrameworkError> {
    index.validate()?;
    settings.validate()?;
    for weighted in &settings.index_time.searchable_attributes {
        if index.attribute(&weighted.name)?.searchable.is_none() {
            return Err(FrameworkError::param(format!(
                "search attribute {}: is not searchable",
                weighted.name
            )));
        }
    }
    for (attributes, role) in [
        (&settings.index_time.filterable_attributes, "filterable"),
        (&settings.index_time.sortable_attributes, "sortable"),
        (&settings.index_time.facet_attributes, "facet"),
    ] {
        for (name, ty) in attributes {
            let declaration = index.attribute(name)?;
            let enabled = match role {
                "filterable" => declaration.filterable,
                "sortable" => declaration.sortable,
                _ => declaration.facet,
            };
            if !enabled || declaration.ty != *ty {
                return Err(FrameworkError::param(format!(
                    "search attribute {name}: {role} settings disagree with its declaration"
                )));
            }
        }
    }
    if let Some(source) = &settings.index_time.embed_source {
        for name in source.attributes.iter().chain(source.title.iter()) {
            index.attribute(name)?;
        }
    }
    if let Some(name) = &settings.query.distinct_attribute {
        index.attribute(name)?;
    }
    for name in &settings.query.typo_tolerance.disable_on_attributes {
        index.attribute(name)?;
    }
    Ok(())
}
struct Field {
    name: String,
    words: Vec<String>,
    weight: f64,
    frequencies: HashMap<String, usize>,
}

impl Field {
    fn count(&self, word: &str, prefix: bool) -> usize {
        if prefix {
            self.words
                .iter()
                .filter(|candidate| candidate.starts_with(word))
                .count()
        } else {
            self.frequencies.get(word).copied().unwrap_or(0)
        }
    }
}

fn fields(document: &SearchDocument, settings: &IndexSettings) -> Vec<Field> {
    settings
        .index_time
        .searchable_attributes
        .iter()
        .filter_map(|a| {
            document.attributes.get(&a.name).map(|v| {
                let text = v.searchable_text().unwrap_or_default();
                let words = tokenize(&text);
                let mut frequencies = HashMap::new();
                for word in &words {
                    *frequencies.entry(word.clone()).or_insert(0) += 1;
                }
                Field {
                    name: a.name.clone(),
                    words,
                    weight: f64::from(a.weight),
                    frequencies,
                }
            })
        })
        .collect()
}
// The generation's write lock invalidates this cache before changing content or
// settings. Matching and BM25 can borrow one immutable tokenization per document.
fn field_cache(generation: &Generation) -> &BTreeMap<String, Vec<Field>> {
    generation.tokenized.get_or_init(|| {
        generation
            .documents
            .iter()
            .map(|(key, document)| {
                (
                    key.clone(),
                    fields(&document.document, &generation.settings),
                )
            })
            .collect()
    })
}
fn phrase_matches(words: &[String], phrase: &[String]) -> bool {
    !phrase.is_empty() && words.windows(phrase.len()).any(|w| w == phrase)
}
fn alternative_matches(field: &Field, alternative: &str, prefix: bool) -> usize {
    // Normalized single ASCII words need no allocation or transliteration.
    if !alternative.is_empty()
        && alternative
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() && !byte.is_ascii_uppercase())
    {
        return field.count(alternative, prefix);
    }
    let parts = tokenize(alternative);
    if parts.len() > 1 {
        return field
            .words
            .windows(parts.len())
            .filter(|w| *w == parts)
            .count();
    }
    if let Some(part) = parts.first() {
        field.count(part, prefix)
    } else {
        0
    }
}
fn term_count(
    field: &Field,
    term: &crate::search::expansion::ExpandedTerm,
    settings: &IndexSettings,
    query_disabled: &BTreeSet<String>,
) -> usize {
    let disabled = settings
        .query
        .typo_tolerance
        .disable_on_attributes
        .contains(&field.name)
        || query_disabled.contains(&field.name);
    term.alternatives
        .iter()
        .chain(std::iter::once(&term.original))
        .filter(|a| !disabled || !term.typo_alternatives.contains(a))
        .map(|a| alternative_matches(field, a, term.prefix && a == &term.original))
        .max()
        .unwrap_or(0)
}
fn lexical_matches(fs: &[Field], settings: &IndexSettings, q: &ExpandedQuery) -> bool {
    q.terms.iter().all(|term| {
        fs.iter()
            .any(|field| term_count(field, term, settings, &q.typo_disabled_attributes) > 0)
    }) && q
        .phrases
        .iter()
        .all(|phrase| fs.iter().any(|field| phrase_matches(&field.words, phrase)))
        && !q.negatives.iter().any(|negative| {
            fs.iter()
                .any(|field| alternative_matches(field, negative, false) > 0)
        })
}
pub(super) fn validate_query(
    index: &IndexDescriptor,
    q: &EngineQuery,
) -> Result<(), FrameworkError> {
    if let Some(f) = &q.filter {
        f.validate(index)?;
    }
    for sort in &q.sort {
        let (attribute, is_geo) = match sort {
            Sort::Attribute { attribute, .. } => (attribute, false),
            Sort::Distance {
                attribute, point, ..
            } => {
                point.validate()?;
                (attribute, true)
            }
        };
        let decl = index.attribute(attribute)?;
        if !is_geo && matches!(decl.ty, AttributeType::Geo | AttributeType::TextList) {
            return Err(FrameworkError::param(format!(
                "sort attribute {attribute}: requires an ordered scalar attribute or an explicit distance sort"
            )));
        }
        if !decl.sortable || (is_geo && decl.ty != AttributeType::Geo) {
            return Err(FrameworkError::param(format!(
                "sort attribute {attribute}: requires a sortable {}attribute",
                if is_geo { "geo " } else { "" }
            )));
        }
    }
    Ok(())
}
fn matching<'a>(g: &'a Generation, q: &EngineQuery) -> Vec<&'a IndexedDocument> {
    let tokenized = field_cache(g);
    g.documents
        .values()
        .filter(|d| {
            q.filter.as_ref().is_none_or(|f| f.matches(&d.document))
                && lexical_matches(&tokenized[&d.document.key], &g.settings, &q.expanded)
        })
        .collect()
}
struct Bm25Stats {
    average_length: f64,
    document_count: f64,
    frequencies: Vec<f64>,
}
fn bm25_stats(g: &Generation, q: &ExpandedQuery) -> Bm25Stats {
    let documents = field_cache(g);
    let n = documents.len() as f64;
    let average = documents
        .values()
        .map(|fs| fs.iter().map(|field| field.words.len()).sum::<usize>())
        .sum::<usize>() as f64
        / n.max(1.0);
    let frequencies = q
        .terms
        .iter()
        .map(|term| {
            documents
                .values()
                .filter(|fs| {
                    fs.iter().any(|field| {
                        term_count(field, term, &g.settings, &q.typo_disabled_attributes) > 0
                    })
                })
                .count() as f64
        })
        .collect();
    Bm25Stats {
        average_length: average,
        document_count: n,
        frequencies,
    }
}
fn bm25(document: &SearchDocument, g: &Generation, q: &ExpandedQuery, stats: &Bm25Stats) -> f64 {
    let fs = &field_cache(g)[&document.key];
    let length = fs.iter().map(|field| field.words.len()).sum::<usize>() as f64;
    let denominator = 1.2 * (0.25 + 0.75 * length / stats.average_length.max(1.0));
    let mut score = 0.0;
    for (term, df) in q.terms.iter().zip(&stats.frequencies) {
        let tf = fs
            .iter()
            .map(|field| {
                term_count(field, term, &g.settings, &q.typo_disabled_attributes) as f64
                    * field.weight
            })
            .sum::<f64>();
        let idf = (1.0 + (stats.document_count - df + 0.5) / (df + 0.5)).ln();
        score += idf * tf * 2.2 / (tf + denominator).max(f64::MIN_POSITIVE);
    }
    for phrase in &q.phrases {
        score += fs
            .iter()
            .filter(|field| phrase_matches(&field.words, phrase))
            .map(|field| field.weight)
            .sum::<f64>();
    }
    score
}
fn missing(value: Option<&AttributeValue>) -> bool {
    value.is_none_or(|v| matches!(v, AttributeValue::Null))
}
fn sort_compare(a: &EngineCandidate, b: &EngineCandidate, sorts: &[Sort]) -> Ordering {
    for sort in sorts {
        let (attribute, direction) = match sort {
            Sort::Attribute {
                attribute,
                direction,
            }
            | Sort::Distance {
                attribute,
                direction,
                ..
            } => (attribute, direction),
        };
        let av = a.document.attributes.get(attribute);
        let bv = b.document.attributes.get(attribute);
        let order = match (missing(av), missing(bv)) {
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            (true, true) => Ordering::Equal,
            (false, false) => {
                let comparison = match (sort, av, bv) {
                    (
                        Sort::Distance { point, .. },
                        Some(AttributeValue::Geo(ap)),
                        Some(AttributeValue::Geo(bp)),
                    ) => ap.distance_to(point).total_cmp(&bp.distance_to(point)),
                    (_, Some(av), Some(bv)) => av.compare(bv).unwrap_or(Ordering::Equal),
                    _ => Ordering::Equal,
                };
                if *direction == SortDirection::Desc {
                    comparison.reverse()
                } else {
                    comparison
                }
            }
        };
        if order != Ordering::Equal {
            return order;
        }
    }
    b.score
        .total_cmp(&a.score)
        .then_with(|| a.document.key.cmp(&b.document.key))
}
pub(super) fn facet_decl(
    index: &IndexDescriptor,
    attr: &str,
    stats: bool,
) -> Result<(), FrameworkError> {
    let decl = index.attribute(attr)?;
    if !decl.facet
        || (stats
            && !matches!(
                decl.ty,
                AttributeType::Integer | AttributeType::Float | AttributeType::DateTime
            ))
    {
        return Err(FrameworkError::param(format!(
            "facet attribute {attr}: requires a {}facet declaration",
            if stats { "numeric or datetime " } else { "" }
        )));
    }
    Ok(())
}
fn facet_distribution(
    index: &IndexDescriptor,
    g: &Generation,
    q: &EngineQuery,
    attrs: &[String],
    cap: usize,
    order: FacetOrder,
) -> Result<FacetDistribution, FrameworkError> {
    let docs = matching(g, q);
    let mut result = BTreeMap::new();
    for attr in attrs {
        facet_decl(index, attr, false)?;
        let mut counts: BTreeMap<String, (AttributeValue, u64)> = BTreeMap::new();
        for doc in &docs {
            if let Some(value) = doc.document.attributes.get(attr) {
                let values = match value {
                    AttributeValue::List(vs) => vs.iter().collect::<Vec<_>>(),
                    AttributeValue::Null => vec![],
                    _ => vec![value],
                };
                let mut seen = BTreeSet::new();
                for value in values {
                    let key = value.facet_key();
                    if seen.insert(key.clone()) {
                        let entry = counts.entry(key).or_insert((value.clone(), 0));
                        entry.1 += 1;
                    }
                }
            }
        }
        let mut values = counts
            .into_values()
            .map(|(value, count)| FacetValue { value, count })
            .collect::<Vec<_>>();
        values.sort_by(|a, b| {
            b.count
                .cmp(&a.count)
                .then_with(|| a.value.facet_key().cmp(&b.value.facet_key()))
        });
        values.truncate(cap);
        if order == FacetOrder::Alphabetical {
            values.sort_by_key(|a| a.value.facet_key());
        }
        result.insert(attr.clone(), values);
    }
    Ok(result)
}
fn validate_documents(
    index: &IndexDescriptor,
    documents: &[IndexedDocument],
    dimensions: Option<usize>,
) -> Result<(), FrameworkError> {
    for document in documents {
        document.document.validate(index)?;
        if let Some(dense) = &document.dense {
            if dense.is_empty() || dense.iter().any(|value| !value.is_finite()) {
                return Err(FrameworkError::param(format!(
                    "search index {} key {}: embedding must be finite and nonempty",
                    index.name, document.document.key
                )));
            }
            if dimensions.is_some_and(|expected| dense.len() != expected) {
                return Err(FrameworkError::param(format!(
                    "search index {} key {}: embedding dimensions do not match settings",
                    index.name, document.document.key
                )));
            }
        }
        if let Some(sparse) = &document.sparse {
            if sparse.indices.len() != sparse.values.len()
                || sparse.values.iter().any(|value| !value.is_finite())
            {
                return Err(FrameworkError::param(format!(
                    "search index {} key {}: invalid sparse embedding",
                    index.name, document.document.key
                )));
            }
            return Err(FrameworkError::param(format!(
                "search index {}: collection does not support sparse embeddings",
                index.name
            )));
        }
    }
    Ok(())
}
fn apply_write(
    generation: &mut Generation,
    document: &IndexedDocument,
    outcome: &mut WriteOutcome,
) {
    let key = &document.document.key;
    let version = document.document.source_version;
    if generation
        .tombstones
        .get(key)
        .is_some_and(|(removed, _)| version <= *removed)
    {
        outcome.skipped_tombstoned += 1;
        return;
    }
    if let Some(existing) = generation.documents.get_mut(key) {
        if version < existing.document.source_version {
            outcome.skipped_older += 1;
            return;
        }
        if document.content_hash == existing.content_hash {
            existing.document.source_version = version;
            outcome.skipped_unchanged += 1;
            return;
        }
    }
    generation.tombstones.remove(key);
    generation.tokenized.take();
    generation.documents.insert(key.clone(), document.clone());
    outcome.written += 1;
}
#[async_trait]
impl SearchEngine for CollectionEngine {
    fn name(&self) -> &str {
        "collection"
    }
    fn capabilities(&self) -> EngineCapabilities {
        EngineCapabilities {
            phrase: true,
            prefix: true,
            semantic: true,
            facets: true,
            geo: true,
            versioned: true,
            generations: true,
            ..EngineCapabilities::default()
        }
    }
    async fn stored_hashes(
        &self,
        index: &IndexDescriptor,
        id: &GenerationId,
        documents: &[IndexedDocument],
    ) -> Result<BTreeMap<String, String>, FrameworkError> {
        let mut state = self.write()?;
        let generation = generation_mut(&mut state, index, id)?;
        let mut hashes = BTreeMap::new();
        for proposed in documents {
            if generation.flush.is_some_and(|(cutoff, removed_at)| {
                removed_at.elapsed() < self.tombstone_retention
                    && proposed.document.source_version <= cutoff
            }) {
                continue;
            }
            if let Some(stored) = generation.documents.get_mut(&proposed.document.key) {
                if stored.content_hash == proposed.content_hash {
                    stored.document.source_version = stored
                        .document
                        .source_version
                        .max(proposed.document.source_version);
                }
                hashes.insert(proposed.document.key.clone(), stored.content_hash.clone());
            }
        }
        Ok(hashes)
    }
    async fn record_flush(
        &self,
        index: &IndexDescriptor,
        id: &GenerationId,
        cutoff: i64,
    ) -> Result<(), FrameworkError> {
        let mut state = self.write()?;
        let generation = generation_mut(&mut state, index, id)?;
        if generation
            .flush
            .is_none_or(|(previous, _)| cutoff >= previous)
        {
            generation.flush = Some((cutoff, Instant::now()));
        }
        Ok(())
    }
    async fn keys_before_version(
        &self,
        index: &IndexDescriptor,
        id: &GenerationId,
        cutoff: i64,
        limit: usize,
    ) -> Result<Vec<(String, i64)>, FrameworkError> {
        index.validate()?;
        if limit == 0 {
            return Err(FrameworkError::param(
                "search removal chunk must be positive",
            ));
        }
        GenerationId::new(id.0.clone())?;
        let state = self.read()?;
        let generation = state
            .indexes
            .get(index.name)
            .and_then(|state| state.generations.get(id))
            .ok_or_else(|| {
                FrameworkError::param(format!(
                    "search index `{}` generation `{}` does not exist",
                    index.name,
                    id.as_str()
                ))
            })?;
        Ok(generation
            .documents
            .values()
            .filter(|doc| doc.document.source_version <= cutoff)
            .take(limit)
            .map(|doc| (doc.document.key.clone(), doc.document.source_version))
            .collect())
    }
    async fn current_generation(
        &self,
        index: &str,
    ) -> Result<Option<GenerationId>, FrameworkError> {
        Ok(self
            .read()?
            .indexes
            .get(index)
            .and_then(|i| i.current.clone()))
    }
    async fn retire_index(&self, index: &str) -> Result<(), FrameworkError> {
        if let Some(index) = self.write()?.indexes.get_mut(index) {
            index.current = None;
            for generation in index.generations.values_mut() {
                generation.state = "retired";
            }
        }
        Ok(())
    }
    async fn generation_states(
        &self,
        index: &str,
    ) -> Result<Vec<(GenerationId, String)>, FrameworkError> {
        Ok(self
            .read()?
            .indexes
            .get(index)
            .map(|state| {
                state
                    .generations
                    .iter()
                    .map(|(id, generation)| (id.clone(), generation.state.into()))
                    .collect()
            })
            .unwrap_or_default())
    }
    async fn create_generation(
        &self,
        index: &IndexDescriptor,
        settings: &IndexSettings,
        id: &GenerationId,
    ) -> Result<(), FrameworkError> {
        index.validate()?;
        GenerationId::new(id.0.clone())?;
        validate_settings(index, settings)?;
        let mut state = self.write()?;
        let i = state.indexes.entry(index.name.into()).or_default();
        if i.generations.contains_key(id) {
            return Err(FrameworkError::param(format!(
                "search index {} generation {}: already exists",
                index.name, id.0
            )));
        }
        i.generations.insert(
            id.clone(),
            Generation {
                state: "building",
                settings: settings.clone(),
                documents: BTreeMap::new(),
                tokenized: OnceLock::new(),
                tombstones: BTreeMap::new(),
                flush: None,
            },
        );
        Ok(())
    }
    async fn switch_generation(
        &self,
        index: &IndexDescriptor,
        from: Option<GenerationId>,
        to: GenerationId,
    ) -> Result<(), FrameworkError> {
        index.validate()?;
        let mut state = self.write()?;
        generation(&state, index, &to)?;
        let i = state.indexes.get_mut(index.name).ok_or_else(|| {
            FrameworkError::param(format!("search index {}: does not exist", index.name))
        })?;
        if i.current != from {
            return Err(FrameworkError::param(format!(
                "search index {}: current generation changed",
                index.name
            )));
        }
        if let Some(old) = &i.current
            && let Some(generation) = i.generations.get_mut(old)
        {
            generation.state = "retired";
        }
        if let Some(generation) = i.generations.get_mut(&to) {
            generation.state = "current";
        }
        i.current = Some(to);
        Ok(())
    }
    async fn drop_generation(
        &self,
        index: &IndexDescriptor,
        id: &GenerationId,
    ) -> Result<(), FrameworkError> {
        let mut state = self.write()?;
        generation(&state, index, id)?;
        let i = state.indexes.get_mut(index.name).ok_or_else(|| {
            FrameworkError::param(format!("search index {}: does not exist", index.name))
        })?;
        if i.current.as_ref() == Some(id) {
            return Err(FrameworkError::param(format!(
                "search index {}: cannot drop active generation {}",
                index.name, id.0
            )));
        }
        i.generations.remove(id);
        Ok(())
    }
    async fn write_settings(
        &self,
        index: &IndexDescriptor,
        id: &GenerationId,
        settings: &IndexSettings,
    ) -> Result<(), FrameworkError> {
        validate_settings(index, settings)?;
        let mut state = self.write()?;
        let g = generation_mut(&mut state, index, id)?;
        if g.settings.index_time.digest() != settings.index_time.digest() && !g.documents.is_empty()
        {
            return Err(FrameworkError::param(format!(
                "search index {}: index-time settings require a new generation",
                index.name
            )));
        }
        g.tokenized.take();
        g.settings = settings.clone();
        Ok(())
    }
    async fn read_settings(
        &self,
        index: &IndexDescriptor,
        id: &GenerationId,
    ) -> Result<IndexSettings, FrameworkError> {
        let state = self.read()?;
        Ok(generation(&state, index, id)?.settings.clone())
    }
    async fn index(
        &self,
        index: &IndexDescriptor,
        id: &GenerationId,
        docs: &[IndexedDocument],
    ) -> Result<WriteOutcome, FrameworkError> {
        let mut state = self.write()?;
        let generation = generation_mut(&mut state, index, id)?;
        validate_documents(index, docs, generation.settings.index_time.dimensions)?;
        let now = Instant::now();
        generation
            .tombstones
            .retain(|_, (_, created)| now.duration_since(*created) < self.tombstone_retention);
        let mut outcome = WriteOutcome::default();
        for document in docs {
            if generation.flush.is_some_and(|(cutoff, removed_at)| {
                now.duration_since(removed_at) < self.tombstone_retention
                    && document.document.source_version <= cutoff
            }) {
                outcome.skipped_tombstoned += 1;
                continue;
            }
            apply_write(generation, document, &mut outcome);
        }
        Ok(outcome)
    }
    async fn remove(
        &self,
        index: &IndexDescriptor,
        id: &GenerationId,
        keys: &[(String, i64)],
    ) -> Result<WriteOutcome, FrameworkError> {
        let mut state = self.write()?;
        let g = generation_mut(&mut state, index, id)?;
        let now = Instant::now();
        g.tombstones
            .retain(|_, (_, created)| now.duration_since(*created) < self.tombstone_retention);
        let mut result = WriteOutcome::default();
        for (key, version) in keys {
            if g.documents
                .get(key)
                .is_some_and(|d| d.document.source_version > *version)
                || g.tombstones.get(key).is_some_and(|(v, _)| *v > *version)
            {
                result.skipped_older += 1;
                continue;
            }
            g.documents.remove(key);
            g.tokenized.take();
            g.tombstones.insert(key.clone(), (*version, now));
            result.written += 1;
        }
        Ok(result)
    }
    async fn query(
        &self,
        index: &IndexDescriptor,
        id: &GenerationId,
        q: &EngineQuery,
    ) -> Result<EngineResult, FrameworkError> {
        validate_query(index, q)?;
        let state = self.read()?;
        let g = generation(&state, index, id)?;
        let docs = matching(g, q);
        let total = Total::Exact(docs.len() as u64);
        let stats = bm25_stats(g, &q.expanded);
        let mut candidates = docs
            .into_iter()
            .map(|d| EngineCandidate {
                document: d.document.clone(),
                score: bm25(&d.document, g, &q.expanded, &stats),
                highlights: None,
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|a, b| sort_compare(a, b, &q.sort));
        candidates.truncate(q.depth);
        Ok(EngineResult { candidates, total })
    }
    async fn semantic(
        &self,
        index: &IndexDescriptor,
        id: &GenerationId,
        vector: &[f32],
        filter: Option<&Filter>,
        depth: usize,
    ) -> Result<EngineResult, FrameworkError> {
        if vector.is_empty() || vector.iter().any(|v| !v.is_finite()) {
            return Err(FrameworkError::param(format!(
                "search index {}: query embedding must be finite and nonempty",
                index.name
            )));
        }
        if let Some(filter) = filter {
            filter.validate(index)?;
        }
        let state = self.read()?;
        let g = generation(&state, index, id)?;
        if g.settings
            .index_time
            .dimensions
            .is_some_and(|dim| dim != vector.len())
        {
            return Err(FrameworkError::param(format!(
                "search index {}: query embedding dimensions do not match settings",
                index.name
            )));
        }
        let norm = vector
            .iter()
            .map(|v| f64::from(*v).powi(2))
            .sum::<f64>()
            .sqrt();
        if norm == 0.0 {
            return Err(FrameworkError::param(format!(
                "search index {}: zero query embedding",
                index.name
            )));
        }
        let mut candidates = vec![];
        for doc in g.documents.values() {
            if filter.is_some_and(|f| !f.matches(&doc.document)) {
                continue;
            }
            if let Some(dense) = &doc.dense {
                if dense.len() != vector.len() {
                    return Err(FrameworkError::param(format!(
                        "search index {} key {}: embedding dimension mismatch",
                        index.name, doc.document.key
                    )));
                }
                let document_norm = dense
                    .iter()
                    .map(|v| f64::from(*v).powi(2))
                    .sum::<f64>()
                    .sqrt();
                if document_norm == 0.0 {
                    continue;
                }
                let score = dense
                    .iter()
                    .zip(vector)
                    .map(|(a, b)| f64::from(*a) * f64::from(*b))
                    .sum::<f64>()
                    / (norm * document_norm);
                candidates.push(EngineCandidate {
                    document: doc.document.clone(),
                    score: score.clamp(-1.0, 1.0),
                    highlights: None,
                });
            }
        }
        candidates.sort_by(|a, b| sort_compare(a, b, &[]));
        candidates.truncate(depth);
        let total = Total::Estimated(candidates.len() as u64);
        Ok(EngineResult { candidates, total })
    }
    async fn facets(
        &self,
        index: &IndexDescriptor,
        id: &GenerationId,
        q: &EngineQuery,
        attrs: &[String],
        cap: usize,
        order: FacetOrder,
    ) -> Result<FacetDistribution, FrameworkError> {
        validate_query(index, q)?;
        let state = self.read()?;
        facet_distribution(index, generation(&state, index, id)?, q, attrs, cap, order)
    }
    async fn facet_stats(
        &self,
        index: &IndexDescriptor,
        id: &GenerationId,
        q: &EngineQuery,
        attrs: &[String],
    ) -> Result<FacetStats, FrameworkError> {
        validate_query(index, q)?;
        let state = self.read()?;
        let g = generation(&state, index, id)?;
        let docs = matching(g, q);
        let mut result = BTreeMap::new();
        for attr in attrs {
            facet_decl(index, attr, true)?;
            let mut values = docs
                .iter()
                .filter_map(|d| d.document.attributes.get(attr))
                .filter(|v| !matches!(v, AttributeValue::Null))
                .cloned()
                .collect::<Vec<_>>();
            values.sort_by(|a, b| a.compare(b).unwrap_or(Ordering::Equal));
            if let (Some(min), Some(max)) = (values.first(), values.last()) {
                result.insert(
                    attr.clone(),
                    FacetRange {
                        min: min.clone(),
                        max: max.clone(),
                    },
                );
            }
        }
        Ok(result)
    }
    async fn facet_search(
        &self,
        index: &IndexDescriptor,
        id: &GenerationId,
        q: &EngineQuery,
        attr: &str,
        prefix: &str,
        cap: usize,
    ) -> Result<FacetDistribution, FrameworkError> {
        let mut distribution = self
            .facets(index, id, q, &[attr.into()], usize::MAX, FacetOrder::Count)
            .await?;
        let prefix = deunicode::deunicode(prefix).to_lowercase();
        if let Some(values) = distribution.get_mut(attr) {
            values.retain(|v| {
                deunicode::deunicode(&v.value.facet_key())
                    .to_lowercase()
                    .starts_with(&prefix)
            });
            values.truncate(cap);
        }
        Ok(distribution)
    }
    async fn vocabulary(
        &self,
        index: &IndexDescriptor,
        id: &GenerationId,
    ) -> Result<Arc<Vocabulary>, FrameworkError> {
        let state = self.read()?;
        let g = generation(&state, index, id)?;
        let mut words = BTreeMap::new();
        for fs in field_cache(g).values() {
            let unique = fs
                .iter()
                .flat_map(|field| field.words.iter())
                .collect::<BTreeSet<_>>();
            for word in unique {
                *words.entry(word.clone()).or_insert(0) += 1;
            }
        }
        Ok(Arc::new(Vocabulary {
            words: words.into(),
        }))
    }
    async fn count(
        &self,
        index: &IndexDescriptor,
        id: &GenerationId,
        q: &EngineQuery,
    ) -> Result<Total, FrameworkError> {
        validate_query(index, q)?;
        let state = self.read()?;
        Ok(Total::Exact(
            matching(generation(&state, index, id)?, q).len() as u64,
        ))
    }
}
