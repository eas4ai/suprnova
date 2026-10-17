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
    postings: BTreeMap<String, BTreeMap<String, Posting>>,
    lengths: BTreeMap<String, usize>,
    document_words: BTreeMap<String, BTreeSet<String>>,
    total_length: usize,
    vocabulary: OnceLock<Arc<Vocabulary>>,
    #[cfg(test)]
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
#[cfg(test)]
struct Field {
    name: String,
    words: Vec<String>,
    weight: f64,
    frequencies: HashMap<String, usize>,
}

#[cfg(test)]
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

#[cfg(test)]
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
#[cfg(test)]
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
#[cfg(test)]
fn phrase_matches(words: &[String], phrase: &[String]) -> bool {
    !phrase.is_empty() && words.windows(phrase.len()).any(|w| w == phrase)
}
#[cfg(test)]
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
#[cfg(test)]
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
#[cfg(test)]
fn lexical_matches(fs: &[Field], settings: &IndexSettings, q: &ExpandedQuery) -> bool {
    let word_matches = |term: &crate::search::ExpandedTerm| {
        fs.iter()
            .any(|field| term_count(field, term, settings, &q.typo_disabled_attributes) > 0)
    };
    (!q.terms.iter().any(|term| !term.required)
        || q.terms
            .iter()
            .filter(|term| !term.required)
            .any(word_matches))
        && q.terms.iter().filter(|term| term.required).all(|term| {
            fs.iter()
                .any(|field| term_count(field, term, settings, &q.typo_disabled_attributes) > 0)
        })
        && q.phrases
            .iter()
            .all(|phrase| fs.iter().any(|field| phrase_matches(&field.words, phrase)))
        && !q.negatives.iter().any(|negative| {
            fs.iter()
                .any(|field| alternative_matches(field, negative, false) > 0)
        })
}
/// Positions are separated by field so phrases never cross attribute boundaries.
struct Posting {
    positions: BTreeMap<usize, Vec<usize>>,
}

type FieldCounts<'a> = BTreeMap<&'a str, Vec<usize>>;

fn remove_postings(g: &mut Generation, key: &str) {
    if let Some(words) = g.document_words.remove(key) {
        for word in words {
            if let Some(postings) = g.postings.get_mut(&word) {
                postings.remove(key);
                if postings.is_empty() {
                    g.postings.remove(&word);
                }
            }
        }
    }
    if let Some(length) = g.lengths.remove(key) {
        g.total_length -= length;
    }
}

fn index_postings(g: &mut Generation, document: &SearchDocument) {
    let mut unique = BTreeSet::new();
    let mut length = 0;
    for (field, attribute) in g
        .settings
        .index_time
        .searchable_attributes
        .iter()
        .enumerate()
    {
        let words = document
            .attributes
            .get(&attribute.name)
            .and_then(AttributeValue::searchable_text)
            .map(|text| tokenize(&text))
            .unwrap_or_default();
        length += words.len();
        for (position, word) in words.into_iter().enumerate() {
            unique.insert(word.clone());
            g.postings
                .entry(word)
                .or_default()
                .entry(document.key.clone())
                .or_insert_with(|| Posting {
                    positions: BTreeMap::new(),
                })
                .positions
                .entry(field)
                .or_default()
                .push(position);
        }
    }
    g.total_length += length;
    g.lengths.insert(document.key.clone(), length);
    g.document_words.insert(document.key.clone(), unique);
}

fn phrase_counts<'a>(g: &'a Generation, phrase: &[String]) -> FieldCounts<'a> {
    let mut counts = BTreeMap::new();
    let Some(first) = phrase.first().and_then(|word| g.postings.get(word)) else {
        return counts;
    };
    for (key, posting) in first {
        for (field, positions) in &posting.positions {
            let count = positions
                .iter()
                .filter(|start| {
                    phrase.iter().enumerate().skip(1).all(|(offset, word)| {
                        g.postings
                            .get(word)
                            .and_then(|docs| docs.get(key))
                            .and_then(|posting| posting.positions.get(field))
                            .is_some_and(|positions| {
                                positions.binary_search(&(*start + offset)).is_ok()
                            })
                    })
                })
                .count();
            if count > 0 {
                counts.entry(key.as_str()).or_insert_with(|| {
                    vec![0; g.settings.index_time.searchable_attributes.len()]
                })[*field] = count;
            }
        }
    }
    counts
}

fn alternative_counts<'a>(g: &'a Generation, alternative: &str, prefix: bool) -> FieldCounts<'a> {
    let parts = tokenize(alternative);
    if parts.len() > 1 {
        return phrase_counts(g, &parts);
    }
    let mut counts = BTreeMap::new();
    let Some(word) = parts.first() else {
        return counts;
    };
    // A sorted vocabulary bounds prefix work to the matching word range.
    for (candidate, postings) in g.postings.range(word.clone()..) {
        if if prefix {
            !candidate.starts_with(word)
        } else {
            candidate != word
        } {
            break;
        }
        for (key, posting) in postings {
            let fields = counts
                .entry(key.as_str())
                .or_insert_with(|| vec![0; g.settings.index_time.searchable_attributes.len()]);
            for (field, positions) in &posting.positions {
                fields[*field] += positions.len();
            }
        }
        if !prefix {
            break;
        }
    }
    counts
}

fn term_frequencies<'a>(
    g: &'a Generation,
    q: &ExpandedQuery,
    term: &crate::search::ExpandedTerm,
) -> BTreeMap<&'a str, f64> {
    let attributes = &g.settings.index_time.searchable_attributes;
    let mut maxima: FieldCounts<'a> = BTreeMap::new();
    for alternative in term
        .alternatives
        .iter()
        .chain(std::iter::once(&term.original))
        .collect::<BTreeSet<_>>()
    {
        let typo = term.typo_alternatives.contains(alternative);
        for (key, counts) in
            alternative_counts(g, alternative, term.prefix && alternative == &term.original)
        {
            for (field, count) in counts.into_iter().enumerate() {
                if count == 0
                    || (typo
                        && (q.typo_disabled_attributes.contains(&attributes[field].name)
                            || g.settings
                                .query
                                .typo_tolerance
                                .disable_on_attributes
                                .contains(&attributes[field].name)))
                {
                    continue;
                }
                let fields = maxima
                    .entry(key)
                    .or_insert_with(|| vec![0; attributes.len()]);
                fields[field] = fields[field].max(count);
            }
        }
    }
    maxima
        .into_iter()
        .map(|(key, fields)| {
            (
                key,
                fields
                    .iter()
                    .zip(attributes)
                    .map(|(tf, attr)| *tf as f64 * f64::from(attr.weight))
                    .sum(),
            )
        })
        .collect()
}

fn word_candidates<'a>(
    g: &'a Generation,
    q: &ExpandedQuery,
    groups: &[BTreeMap<&'a str, f64>],
) -> BTreeMap<&'a str, f64> {
    let mut candidates = BTreeMap::new();
    let optional = q
        .terms
        .iter()
        .enumerate()
        .filter(|(_, term)| !term.required)
        .collect::<Vec<_>>();
    if optional.is_empty() {
        if let Some(group) = groups.first() {
            candidates.extend(group.keys().map(|key| (*key, 0.0)));
        } else if let Some(phrase) = q.phrases.first() {
            candidates.extend(phrase_counts(g, phrase).keys().map(|key| (*key, 0.0)));
        } else {
            candidates.extend(g.documents.keys().map(|key| (key.as_str(), 0.0)));
        }
    } else {
        for (at, _) in optional {
            candidates.extend(groups[at].keys().map(|key| (*key, 0.0)));
        }
    }
    candidates
}

fn evaluate<'a>(g: &'a Generation, q: &EngineQuery) -> BTreeMap<&'a str, f64> {
    let groups = q
        .expanded
        .terms
        .iter()
        .map(|term| term_frequencies(g, &q.expanded, term))
        .collect::<Vec<_>>();
    let mut candidates = word_candidates(g, &q.expanded, &groups);
    for (_, group) in q
        .expanded
        .terms
        .iter()
        .zip(&groups)
        .filter(|(term, _)| term.required)
    {
        candidates.retain(|key, _| group.contains_key(key));
    }
    let phrases = q
        .expanded
        .phrases
        .iter()
        .map(|phrase| phrase_counts(g, phrase))
        .collect::<Vec<_>>();
    for phrase in &phrases {
        candidates.retain(|key, _| phrase.contains_key(key));
    }
    for negative in &q.expanded.negatives {
        for key in alternative_counts(g, negative, false).keys() {
            candidates.remove(key);
        }
    }
    candidates.retain(|key, _| {
        q.filter
            .as_ref()
            .is_none_or(|filter| filter.matches(&g.documents[*key].document))
    });
    let n = g.documents.len() as f64;
    let average = g.total_length as f64 / n.max(1.0);
    let denominators = candidates
        .keys()
        .map(|key| {
            let length = g.lengths[*key] as f64;
            (*key, 1.2 * (0.25 + 0.75 * length / average.max(1.0)))
        })
        .collect::<HashMap<_, _>>();
    // Add only matched postings, in query-group order to preserve the scan's
    // floating-point scores without visiting every group for every document.
    for group in &groups {
        let df = group.len() as f64;
        let idf = (1.0 + (n - df + 0.5) / (df + 0.5)).ln();
        for (key, tf) in group {
            if let Some(score) = candidates.get_mut(key) {
                *score += idf * tf * 2.2 / (tf + denominators[key]).max(f64::MIN_POSITIVE);
            }
        }
    }
    for phrase in &phrases {
        for (key, counts) in phrase {
            if let Some(score) = candidates.get_mut(key) {
                *score += counts
                    .iter()
                    .zip(&g.settings.index_time.searchable_attributes)
                    .filter(|(count, _)| **count > 0)
                    .map(|(_, attr)| f64::from(attr.weight))
                    .sum::<f64>();
            }
        }
    }
    candidates
}

fn matching<'a>(g: &'a Generation, q: &EngineQuery) -> Vec<&'a IndexedDocument> {
    evaluate(g, q)
        .keys()
        .map(|key| &g.documents[*key])
        .collect()
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
#[cfg(test)]
fn scan_matching<'a>(g: &'a Generation, q: &EngineQuery) -> Vec<&'a IndexedDocument> {
    let tokenized = field_cache(g);
    g.documents
        .values()
        .filter(|d| {
            q.filter.as_ref().is_none_or(|f| f.matches(&d.document))
                && lexical_matches(&tokenized[&d.document.key], &g.settings, &q.expanded)
        })
        .collect()
}
#[cfg(test)]
struct Bm25Stats {
    average_length: f64,
    document_count: f64,
    frequencies: Vec<f64>,
}
#[cfg(test)]
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
#[cfg(test)]
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
    remove_postings(generation, key);
    index_postings(generation, &document.document);
    generation.vocabulary.take();
    #[cfg(test)]
    generation.tokenized.take();
    generation.documents.insert(key.clone(), document.clone());
    outcome.written += 1;
}
#[async_trait]
impl SearchEngine for CollectionEngine {
    fn advances_render_cache(&self) -> bool {
        true
    }
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
        crate::search::render_cache::external_write(
            index.name,
            Box::pin(async {
                let mut state = self.write()?;
                let generation = generation_mut(&mut state, index, id)?;
                if generation
                    .flush
                    .is_none_or(|(previous, _)| cutoff >= previous)
                {
                    generation.flush = Some((cutoff, Instant::now()));
                }
                Ok(())
            }),
        )
        .await
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
        crate::search::render_cache::external_write(
            index,
            Box::pin(async {
                if let Some(index) = self.write()?.indexes.get_mut(index) {
                    index.current = None;
                    for generation in index.generations.values_mut() {
                        generation.state = "retired";
                    }
                }
                Ok(())
            }),
        )
        .await
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
        crate::search::render_cache::external_write(
            index.name,
            Box::pin(async {
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
                        postings: BTreeMap::new(),
                        lengths: BTreeMap::new(),
                        document_words: BTreeMap::new(),
                        total_length: 0,
                        vocabulary: OnceLock::new(),
                        #[cfg(test)]
                        tokenized: OnceLock::new(),
                        tombstones: BTreeMap::new(),
                        flush: None,
                    },
                );
                Ok(())
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
        crate::search::render_cache::external_write(
            index.name,
            Box::pin(async {
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
            }),
        )
        .await
    }
    async fn drop_generation(
        &self,
        index: &IndexDescriptor,
        id: &GenerationId,
    ) -> Result<(), FrameworkError> {
        crate::search::render_cache::external_write(
            index.name,
            Box::pin(async {
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
            }),
        )
        .await
    }
    async fn write_settings(
        &self,
        index: &IndexDescriptor,
        id: &GenerationId,
        settings: &IndexSettings,
    ) -> Result<(), FrameworkError> {
        crate::search::render_cache::external_write(
            index.name,
            Box::pin(async {
                validate_settings(index, settings)?;
                let mut state = self.write()?;
                let g = generation_mut(&mut state, index, id)?;
                if g.settings.index_time.digest() != settings.index_time.digest()
                    && !g.documents.is_empty()
                {
                    return Err(FrameworkError::param(format!(
                        "search index {}: index-time settings require a new generation",
                        index.name
                    )));
                }
                #[cfg(test)]
                g.tokenized.take();
                g.settings = settings.clone();
                Ok(())
            }),
        )
        .await
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
        crate::search::render_cache::external_write(
            index.name,
            Box::pin(async {
                let mut state = self.write()?;
                let generation = generation_mut(&mut state, index, id)?;
                validate_documents(index, docs, generation.settings.index_time.dimensions)?;
                let now = Instant::now();
                generation.tombstones.retain(|_, (_, created)| {
                    now.duration_since(*created) < self.tombstone_retention
                });
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
            }),
        )
        .await
    }
    async fn remove(
        &self,
        index: &IndexDescriptor,
        id: &GenerationId,
        keys: &[(String, i64)],
    ) -> Result<WriteOutcome, FrameworkError> {
        crate::search::render_cache::external_write(
            index.name,
            Box::pin(async {
                let mut state = self.write()?;
                let g = generation_mut(&mut state, index, id)?;
                let now = Instant::now();
                g.tombstones.retain(|_, (_, created)| {
                    now.duration_since(*created) < self.tombstone_retention
                });
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
                    remove_postings(g, key);
                    g.documents.remove(key);
                    g.vocabulary.take();
                    #[cfg(test)]
                    g.tokenized.take();
                    g.tombstones.insert(key.clone(), (*version, now));
                    result.written += 1;
                }
                Ok(result)
            }),
        )
        .await
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
        let evaluated = evaluate(g, q);
        let total = Total::Exact(evaluated.len() as u64);
        let mut ranked = evaluated.into_iter().collect::<Vec<_>>();
        ranked.sort_by(|(ak, a), (bk, b)| {
            crate::search::builder::compare_sort(
                &g.documents[*ak].document,
                &g.documents[*bk].document,
                &q.sort,
            )
            .then_with(|| b.total_cmp(a))
            .then_with(|| ak.cmp(bk))
        });
        ranked.truncate(q.depth);
        let candidates = ranked
            .into_iter()
            .map(|(key, score)| EngineCandidate {
                document: g.documents[key].document.clone(),
                score,
                highlights: None,
            })
            .collect();
        Ok(EngineResult { candidates, total })
    }
    async fn stored_vectors(
        &self,
        index: &IndexDescriptor,
        id: &GenerationId,
        keys: &[String],
    ) -> Result<std::collections::HashMap<String, Vec<f32>>, FrameworkError> {
        let state = self.read()?;
        let g = generation(&state, index, id)?;
        Ok(keys
            .iter()
            .filter_map(|key| {
                g.documents
                    .get(key)
                    .and_then(|doc| doc.dense.clone())
                    .map(|dense| (key.clone(), dense))
            })
            .collect())
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
        Ok(g.vocabulary
            .get_or_init(|| {
                Arc::new(Vocabulary {
                    words: g
                        .postings
                        .iter()
                        .map(|(word, postings)| (word.clone(), postings.len() as u64))
                        .collect(),
                })
            })
            .clone())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::{baseline::Fixture, expansion::expand, query::QueryText};

    fn descriptor() -> IndexDescriptor {
        let mut index = crate::search::baseline::descriptor();
        index.name = "postings_reference";
        for attribute in &mut index.attributes {
            attribute.filterable = true;
            attribute.sortable = true;
            attribute.facet = true;
        }
        index
    }

    fn document(
        index: &IndexDescriptor,
        key: &str,
        title: &str,
        text: &str,
        version: i64,
    ) -> IndexedDocument {
        let document = SearchDocument {
            index: index.name.into(),
            key: key.into(),
            source_version: version,
            attributes: BTreeMap::from([
                ("title".into(), AttributeValue::Text(title.into())),
                ("text".into(), AttributeValue::Text(text.into())),
            ]),
            embed_text: None,
            title: None,
        };
        IndexedDocument {
            content_hash: document.content_hash("reference"),
            document,
            dense: None,
            sparse: None,
        }
    }

    async fn compare(
        engine: &CollectionEngine,
        index: &IndexDescriptor,
        id: &GenerationId,
        text: &str,
    ) -> Result<(), FrameworkError> {
        let settings = engine.read_settings(index, id).await?;
        let vocabulary = engine.vocabulary(index, id).await?;
        let mut q = EngineQuery {
            expanded: expand(&QueryText::parse(text), &settings.query, &vocabulary, 64)?,
            filter: None,
            sort: vec![],
            depth: 10,
            highlights: false,
            crop: Default::default(),
        };
        // Ranking, candidate totals, filtering and explicit sorts use the same oracle.
        for mode in 0..3 {
            if mode == 1 {
                q.filter = Some(Filter::Ne(
                    "title".into(),
                    AttributeValue::Text("apple pie".into()),
                ));
            } else if mode == 2 {
                q.sort = vec![Sort::Attribute {
                    attribute: "title".into(),
                    direction: SortDirection::Desc,
                }];
            }
            let result = engine.query(index, id, &q).await?;
            let state = engine.read()?;
            let g = generation(&state, index, id)?;
            let docs = scan_matching(g, &q);
            assert_eq!(result.total.value(), docs.len() as u64, "{text}");
            let stats = bm25_stats(g, &q.expanded);
            let mut expected = docs
                .into_iter()
                .map(|d| EngineCandidate {
                    document: d.document.clone(),
                    score: bm25(&d.document, g, &q.expanded, &stats),
                    highlights: None,
                })
                .collect::<Vec<_>>();
            expected.sort_by(|a, b| sort_compare(a, b, &q.sort));
            expected.truncate(10);
            assert_eq!(
                result
                    .candidates
                    .iter()
                    .map(|hit| (&hit.document.key, hit.score))
                    .collect::<Vec<_>>(),
                expected
                    .iter()
                    .map(|hit| (&hit.document.key, hit.score))
                    .collect::<Vec<_>>(),
                "query {text:?}, mode {mode}",
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn scifact_postings_equal_scan_for_fifty_queries() -> Result<(), FrameworkError> {
        let fixture = Fixture::load(crate::search::baseline::fixture_directory())?;
        let index = descriptor();
        let engine = CollectionEngine::new();
        let id = GenerationId::default();
        engine
            .create_generation(&index, &index.index_settings(), &id)
            .await?;
        let docs = fixture
            .corpus
            .iter()
            .map(|d| document(&index, &d.id, &d.title, &d.text, 1))
            .collect::<Vec<_>>();
        for chunk in docs.chunks(200) {
            engine.index(&index, &id, chunk).await?;
        }
        for q in fixture.queries.iter().take(50) {
            compare(&engine, &index, &id, &q.text).await?;
        }
        Ok(())
    }

    #[tokio::test]
    async fn postings_equal_scan_after_versioned_writes_and_removals() -> Result<(), FrameworkError>
    {
        let index = descriptor();
        let engine = CollectionEngine::new();
        let id = GenerationId::default();
        let mut settings = index.index_settings();
        settings
            .query
            .typo_tolerance
            .disable_on_attributes
            .insert("title".into());
        settings.index_time.searchable_attributes[0].weight = 2.0;
        settings.query.synonyms.push(crate::search::Synonym {
            terms: vec!["fresh baked".into(), "oven warm".into()],
            one_way: false,
        });
        engine.create_generation(&index, &settings, &id).await?;
        let docs = vec![
            document(&index, "a", "apple pie", "fresh baked apple apple pie", 1),
            document(&index, "b", "applesauce", "oven warm pie apple", 1),
            document(&index, "c", "apple", "fresh baked green", 1),
            document(&index, "d", "fresh", "baked apple pie", 1),
        ];
        engine.index(&index, &id, &docs).await?;
        let queries = [
            "apple pie",
            "applr pie",
            "apple appl",
            "apple -green",
            "-green",
            "",
            "\"fresh baked\"",
            "apple \"fresh baked\" -green",
            "apple \"pie fresh\"",
            "apple -absent",
        ];
        for q in queries {
            compare(&engine, &index, &id, q).await?;
        }
        let required = engine
            .query(
                &index,
                &id,
                &EngineQuery {
                    expanded: expand(
                        &QueryText::parse("apple \"fresh baked\" -green"),
                        &settings.query,
                        engine.vocabulary(&index, &id).await?.as_ref(),
                        64,
                    )?,
                    filter: None,
                    sort: vec![],
                    depth: 10,
                    highlights: false,
                    crop: Default::default(),
                },
            )
            .await?;
        let mut keys = required
            .candidates
            .iter()
            .map(|hit| hit.document.key.as_str())
            .collect::<Vec<_>>();
        keys.sort_unstable();
        assert_eq!(keys, ["a", "b"]);
        let changed = document(&index, "a", "banana", "banana orchard", 3);
        engine
            .index(&index, &id, std::slice::from_ref(&changed))
            .await?;
        assert_eq!(
            engine
                .index(&index, &id, &[docs[0].clone()])
                .await?
                .skipped_older,
            1
        );
        assert_eq!(
            engine
                .remove(&index, &id, &[("a".into(), 2)])
                .await?
                .skipped_older,
            1
        );
        engine.remove(&index, &id, &[("b".into(), 4)]).await?;
        assert_eq!(
            engine
                .index(&index, &id, &[docs[1].clone()])
                .await?
                .skipped_tombstoned,
            1
        );
        let mut same = changed.clone();
        same.document.source_version = 5;
        assert_eq!(
            engine.index(&index, &id, &[same]).await?.skipped_unchanged,
            1
        );
        assert_eq!(
            engine
                .remove(&index, &id, &[("a".into(), 4)])
                .await?
                .skipped_older,
            1
        );
        for q in queries {
            compare(&engine, &index, &id, q).await?;
        }
        engine
            .remove(
                &index,
                &id,
                &[("a".into(), 6), ("c".into(), 6), ("d".into(), 6)],
            )
            .await?;
        assert!(engine.vocabulary(&index, &id).await?.words.is_empty());
        let state = engine.read()?;
        let g = generation(&state, &index, &id)?;
        assert!(g.postings.is_empty());
        assert!(g.lengths.is_empty());
        assert_eq!(g.total_length, 0);
        Ok(())
    }
}
