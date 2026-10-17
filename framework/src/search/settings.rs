//! Two settings classes separate immediate query changes from generation builds.

use super::document::{AttributeType, canonical_json};
use crate::FrameworkError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

/// Attribute weights retain declaration order for engine-native ranking.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WeightedAttribute {
    /// A declared text attribute names the engine column to weight.
    pub name: String,
    /// Positive finite weights scale that column's native relevance.
    pub weight: f32,
}

/// Embedding sources keep semantic content independent of lexical ranking.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbedSource {
    /// Declared fields are joined in this order when preparing an embedding.
    pub attributes: Vec<String>,
    /// A title field supplies the model profile's title placeholder.
    pub title: Option<String>,
}

/// Storage paths keep byte loading outside model inference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaSource {
    /// The projection includes this path even when it is not lexical text.
    pub field: String,
    /// Capabilities are validated before a generation can sync.
    pub kind: MediaKind,
    /// Disks are code-owned and never selected by uploaded content.
    pub disk: String,
}

/// A declared kind binds storage bytes to one processor and provider capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    /// Still pixels use the image encoder.
    Image,
    /// Waveforms use the audio encoder.
    Audio,
    /// Sampled frames use the vision encoder and video placeholders.
    Video,
}

impl MediaKind {
    pub(crate) fn accepted(self, inputs: &super::InputKinds) -> bool {
        match self {
            Self::Image => inputs.image,
            Self::Audio => inputs.audio,
            Self::Video => inputs.video,
        }
    }
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Audio => "audio",
            Self::Video => "video",
        }
    }
}

/// Language configuration is engine-specific because tokenizers differ.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "engine", rename_all = "snake_case")]
pub enum LanguageConfig {
    /// MySQL and MariaDB inherit the server's full-text configuration.
    #[default]
    Server,
    /// Postgres needs a text search configuration for vectors and queries.
    Postgres {
        /// The trusted configuration name belongs to index settings.
        configuration: String,
    },
    /// SQLite FTS5 needs its tokenizer configuration at generation creation.
    Sqlite {
        /// The FTS5 tokenizer string is code-owned, never request-owned.
        tokenizer: String,
    },
    /// Qdrant's text payload index carries its own language controls.
    Qdrant {
        /// The engine tokenizer names its text splitting convention.
        tokenizer: String,
        /// An optional stemmer preserves Qdrant's language-specific behavior.
        stemmer: Option<String>,
        /// Stop words belong to the text index, separate from query removal.
        stop_words: Vec<String>,
        /// Folding controls accents in the engine's vocabulary.
        folding: bool,
    },
}

/// Typo limits bound query expansion without rewriting document tokens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypoTolerance {
    /// Indexes can disable correction when exact identifiers matter.
    pub enabled: bool,
    /// Unicode scalar length at which one edit is allowed.
    pub one_edit_min_length: usize,
    /// Unicode scalar length at which two edits are allowed.
    pub two_edit_min_length: usize,
    /// Named words remain exact, independently of their length.
    pub disable_on_words: BTreeSet<String>,
    /// Named attributes remain exact when engines render field alternatives.
    pub disable_on_attributes: BTreeSet<String>,
}

impl Default for TypoTolerance {
    fn default() -> Self {
        Self {
            enabled: true,
            one_edit_min_length: 5,
            two_edit_min_length: 9,
            disable_on_words: BTreeSet::new(),
            disable_on_attributes: BTreeSet::new(),
        }
    }
}

/// Synonym groups allow both symmetric phrases and directional replacements.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Synonym {
    /// For one-way groups the first term is the source; all others are replacements.
    pub terms: Vec<String>,
    /// False expands any group member to every other member.
    pub one_way: bool,
}

/// Facet presentation is applied after selecting the most frequent values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FacetOrder {
    /// Counts descending make common values easy to find.
    #[default]
    Count,
    /// Alphabetical presentation keeps a fixed navigation order.
    Alphabetical,
}

/// Query settings take effect without rebuilding indexed documents.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct QuerySettings {
    /// Correction limits are applied to the engine vocabulary.
    pub typo_tolerance: TypoTolerance,
    /// Synonyms are expanded before engine query rendering.
    pub synonyms: Vec<Synonym>,
    /// Query-only stop words are removed before retrieval.
    pub stop_words: BTreeSet<String>,
    /// Distinct keeps the highest-ranked hit for each value of this attribute.
    pub distinct_attribute: Option<String>,
    /// The cap bounds accessible ranked hits, independently of facet counting.
    pub hits_cap: usize,
    /// Hybrid retrieval takes this many candidates from each leg.
    pub fusion_depth: usize,
    /// The default semantic weight applies when a builder specifies no ratio.
    pub hybrid_ratio: f32,
    /// Validated opening markup is inserted after escaping document text.
    pub highlight_pre_tag: String,
    /// The matching closing tag ends a highlighted span.
    pub highlight_post_tag: String,
    /// A plain text marker identifies omitted text and is HTML-escaped.
    pub crop_marker: String,
    /// Facet counts are complete but only this many values are returned.
    pub facet_value_cap: usize,
    /// Presentation follows count or alphabet after cap selection.
    pub facet_order: FacetOrder,
}

impl Default for QuerySettings {
    fn default() -> Self {
        Self {
            typo_tolerance: TypoTolerance::default(),
            synonyms: Vec::new(),
            stop_words: BTreeSet::new(),
            distinct_attribute: None,
            hits_cap: 1000,
            fusion_depth: 200,
            hybrid_ratio: 0.0,
            highlight_pre_tag: "<em>".into(),
            highlight_post_tag: "</em>".into(),
            crop_marker: "...".into(),
            facet_value_cap: 100,
            facet_order: FacetOrder::Count,
        }
    }
}

/// Index-time settings describe a generation and must not mutate a live one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct IndexTimeSettings {
    /// Tokenizer and language choices belong to the engine's native index.
    pub language: LanguageConfig,
    /// Columns and their weights determine engine-native lexical relevance.
    pub searchable_attributes: Vec<WeightedAttribute>,
    /// Source fields determine the semantic document input.
    pub embed_source: Option<EmbedSource>,
    /// A nonempty override replaces the profile's document template.
    pub embed_template: String,
    /// Filter types let engines bind values without inferring a schema.
    pub filterable_attributes: BTreeMap<String, AttributeType>,
    /// Sort types let engines keep missing values last.
    pub sortable_attributes: BTreeMap<String, AttributeType>,
    /// Facet types describe both counting and stats support.
    pub facet_attributes: BTreeMap<String, AttributeType>,
    /// An optional provider overrides SEARCH_EMBEDDER for this index.
    pub embedder: Option<String>,
    /// Declaration order binds media placeholders to encoder feature rows.
    pub embed_media: Vec<MediaSource>,
    /// The model identity is part of the generation's content digest.
    pub model: Option<String>,
    /// Registered sparse provider identity pins the lexical vector space to this generation.
    pub sparse_model: Option<String>,
    /// A declared dimension catches incompatible vectors before storage.
    pub dimensions: Option<usize>,
}

impl IndexTimeSettings {
    /// Canonical JSON makes a generation digest independent of map insertion order.
    pub fn digest(&self) -> String {
        let bytes = canonical_json(serde_json::json!(self));
        hex::encode(Sha256::digest(bytes.as_bytes()))
    }
}

/// Settings can be read back exactly while distinguishing rebuilds from query changes.
///
/// ```
/// use suprnova::{IndexSettings, SettingsChange};
/// let old = IndexSettings::default();
/// let mut new = old.clone();
/// new.query.hits_cap = 50;
/// assert_eq!(old.index_time.digest(), new.index_time.digest());
/// assert!(!SettingsChange::between(&old, &new).requires_generation());
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct IndexSettings {
    /// Query settings apply when the next query is built.
    pub query: QuerySettings,
    /// Index-time settings identify one complete generation.
    pub index_time: IndexTimeSettings,
}

impl IndexSettings {
    /// Validate code-owned markup and numeric limits before an engine stores settings.
    pub fn validate(&self) -> Result<(), FrameworkError> {
        super::highlight::validate_tags(
            &self.query.highlight_pre_tag,
            &self.query.highlight_post_tag,
        )?;
        if self.query.hits_cap == 0
            || self.query.fusion_depth == 0
            || self.query.facet_value_cap == 0
        {
            return Err(FrameworkError::param(
                "search settings hits_cap, fusion_depth and facet_value_cap must be positive",
            ));
        }
        if !self.query.hybrid_ratio.is_finite() || !(0.0..=1.0).contains(&self.query.hybrid_ratio) {
            return Err(FrameworkError::param(
                "search hybrid_ratio must be between 0 and 1",
            ));
        }
        let typo = &self.query.typo_tolerance;
        if typo.one_edit_min_length == 0 || typo.two_edit_min_length < typo.one_edit_min_length {
            return Err(FrameworkError::param(
                "search typo_tolerance length thresholds are invalid",
            ));
        }
        for synonym in &self.query.synonyms {
            if synonym.terms.len() < 2 || synonym.terms.iter().any(|term| term.trim().is_empty()) {
                return Err(FrameworkError::param(
                    "search synonyms require at least two nonempty terms",
                ));
            }
        }
        let mut names = BTreeSet::new();
        for attr in &self.index_time.searchable_attributes {
            if attr.name.is_empty()
                || !names.insert(&attr.name)
                || !attr.weight.is_finite()
                || attr.weight <= 0.0
            {
                return Err(FrameworkError::param(format!(
                    "searchable attribute '{}' requires a unique name and positive finite weight",
                    attr.name
                )));
            }
        }
        if self
            .index_time
            .sparse_model
            .as_ref()
            .is_some_and(|name| name.trim().is_empty())
        {
            return Err(FrameworkError::param(
                "search index_time.sparse_model requires a nonempty registered provider name",
            ));
        }
        if self.index_time.dimensions == Some(0) {
            return Err(FrameworkError::param(
                "search embedding dimensions must be positive",
            ));
        }
        for (name, ty) in &self.index_time.facet_attributes {
            if self.index_time.filterable_attributes.get(name) != Some(ty) {
                return Err(FrameworkError::param(format!(
                    "facet attribute '{name}' must also be filterable with the same type"
                )));
            }
        }
        Ok(())
    }
}

/// Changed fields are grouped so a sync command can explain its actions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SettingsChange {
    /// These paths take effect on the next query.
    pub query_time: Vec<String>,
    /// These paths require a new generation before switching readers.
    pub index_time: Vec<String>,
}

impl SettingsChange {
    /// Compare each named setting, rather than hiding several changes in one digest.
    pub fn between(old: &IndexSettings, new: &IndexSettings) -> Self {
        fn changes(a: serde_json::Value, b: serde_json::Value) -> Vec<String> {
            match (a.as_object(), b.as_object()) {
                (Some(a), Some(b)) => a
                    .keys()
                    .chain(b.keys())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .filter(|key| a.get(*key) != b.get(*key))
                    .cloned()
                    .collect(),
                _ => Vec::new(),
            }
        }
        Self {
            query_time: changes(serde_json::json!(old.query), serde_json::json!(new.query)),
            index_time: changes(
                serde_json::json!(old.index_time),
                serde_json::json!(new.index_time),
            ),
        }
    }

    /// Any index-time field change needs a separate complete generation.
    pub fn requires_generation(&self) -> bool {
        !self.index_time.is_empty()
    }
}
