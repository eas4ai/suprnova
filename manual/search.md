# Search

You search Eloquent models through one typed API. The engine ranks documents
with its own relevance score. Suprnova expands your query, applies your viewer's
scope, fuses lexical and semantic results, hydrates models, and formats matches.
You choose `database`, `qdrant`, `collection`, or `null` without changing the
model's search calls.

The default is lexical search on your application's database. You need no
embedding model or external search service. For vector storage without model
search, see [Vector](vector.md).

## Declare a searchable model

Place `#[derive(Searchable)]` below `#[suprnova::model]` so the derive sees the
model's generated metadata. In a generated application, this declaration works
with the default features and the application's Serde and SeaORM dependencies:

```rust
use suprnova::{AttributeValue, Filter, FrameworkError, IndexSettings, Searchable, Viewer};

#[suprnova::model(table = "articles", timestamps = false)]
#[derive(Searchable)]
#[searchable(
    index = "articles",
    should_index = "is_published",
    with = "prepare_search",
    scope = "article_scope",
    scope_fields("owner_id"),
    settings = "article_settings",
    geo(name = "location", lat = "lat", lng = "lng")
)]
pub struct Article {
    pub id: i64,
    #[searchable(text(weight = 2.0), embed, title)]
    pub title: String,
    #[searchable(text, embed)]
    pub body: String,
    #[searchable(facet, sortable)]
    pub category: Option<String>,
    #[searchable(facet, sortable)]
    pub price: f64,
    #[searchable(filterable)]
    pub published: bool,
    #[searchable(filterable)]
    pub owner_id: String,
    pub lat: f64,
    pub lng: f64,
}

impl Article {
    fn is_published(&self) -> Result<bool, FrameworkError> {
        Ok(self.published)
    }

    async fn prepare_search(models: Vec<Self>) -> Result<Vec<Self>, FrameworkError> {
        Ok(models)
    }
}

fn article_scope(viewer: &Viewer) -> Filter {
    Filter::Eq(
        "owner_id".into(),
        AttributeValue::Text(viewer.id().unwrap_or("__guest__").into()),
    )
}

fn article_settings() -> IndexSettings {
    IndexSettings::default()
}
```

Create the matching application table through your [migrator](migrations.md).
Use an owner value that your application never assigns for the guest branch.
After adding `suprnova::search::migrations()` to the migrator, provision the
initial persistent index before model writes:

```bash
cargo run -- migrate
cargo run --bin console -- search:index articles
cargo run --bin console -- search:import articles
```

The index name defaults to the model's table name. Field declaration order is
the searchable attribute order. You can combine these field attributes:

| Attribute | Meaning |
|---|---|
| `text` | Include the field in lexical ranking. Its default weight is 1. |
| `text(weight = 2.0)` | Apply a positive finite native column weight. `text, weight = 2.0` is equivalent. |
| `filterable` | Allow bound comparisons in filters and scopes. |
| `sortable` | Allow attribute ordering. |
| `facet` | Count values and allow filtering; this implies `filterable`. |
| `embed` | Join the field into the embedding source in declaration order. |
| `title` | Supply the embedding template's title. |

Supported field types are `String`, integer types through `i64` and `u64`,
`f32`, `f64`, `bool`, `chrono::NaiveDateTime`, `chrono::DateTime<chrono::Utc>`,
`DateTimeUtc`, and `Vec<String>`, including their `Option` forms. Search stores
integers as `i64` and rejects unsigned overflow. Text roles accept strings and
text lists. Optional values remain null; non-finite floats are errors.

The struct options are `index`, `soft_delete`, `should_index`, `with`, `scope`,
`scope_fields("field", ...)`, `settings`, `touches("relation", ...)`, and
`geo(name = "location", lat = "lat", lng = "lng")`. `should_index` names a model method returning
`Result<bool, FrameworkError>`; false removes an existing document. `with` names
an async model method taking and returning `Vec<Self>` through `Result`, so you
can load relations once per chunk before `make_searchable_using` projects rows.
`scope` names `fn(&Viewer) -> Filter`, and `scope_fields` names every
filterable field it reads; the two come together. `settings` names
`fn() -> IndexSettings`.

The geo option synthesizes a filterable, sortable geo attribute from numeric
latitude and longitude fields. Coordinates use degrees, latitude -90 to 90 and
longitude -180 to 180. Distances use meters. The coordinate source fields do not
become public search attributes unless you declare them separately.

`to_search_document()` returns a `Result<SearchDocument, FrameworkError>` with
the textual model key, index, declared typed attributes, embed text, and title.
Timestamped projections use `updated_at` in UTC microseconds. A model without
`updated_at` gets a version the framework keeps per row in the
`search_source_versions` table: every save raises it inside the save's own
transaction, so a rolled-back save leaves it unchanged, and reading a row never
raises it. `to_search_document()` is synchronous and cannot query, so Eloquent
loads the kept versions in one query beside the rows it returns, and the
projection reads the version of the row as loaded or last saved in this
process; a row another process saved since then reads the newer version on its
next load. Undeclared model fields stay outside the document.

## Index settings

Return settings from your code-owned function and run
`search:sync-index-settings [index]`. The command prints changed fields by
class and reports whether it builds a generation. You can read stored settings
with `SearchRegistry::engine_for(index)?.read_settings(descriptor, &generation)`.

| Query-time setting | Default | Effect on the next query after sync |
|---|---|---|
| `typo_tolerance` | Enabled, one edit from 5 scalars, two from 9 | Correction thresholds and excluded words or attributes. |
| `synonyms` | Empty | Two-way groups or one-way source/replacement groups. |
| `stop_words` | Empty | Remove words from the query before retrieval. |
| `distinct_attribute` | None | Keep the first ranked hit for each value. |
| `hits_cap` | 1000 | Bound accessible ranked hits. |
| `fusion_depth` | 200 | Bound candidates from each hybrid leg. |
| `hybrid_ratio` | 0 | Choose lexical versus semantic weight when the builder gives none. |
| `highlight_pre_tag`, `highlight_post_tag` | `<em>`, `</em>` | Wrap escaped matches with validated markup. |
| `crop_marker` | `...` | Mark omitted text. |
| `facet_value_cap`, `facet_order` | 100, `FacetOrder::Count` | Select frequent values, then order by count or alphabetically. |

`SEARCH_HITS_CAP` and `SEARCH_FUSION_DEPTH` supply deployment defaults when
the code keeps those settings at their defaults.

Every change to `IndexTimeSettings` requires a new generation: `language`,
`searchable_attributes` with weights, `embed_source`, `embed_template`,
`filterable_attributes`, `sortable_attributes`, `facet_attributes`, `embedder`,
`model`, `sparse_model`, and `dimensions`. The derive supplies attribute sets and
embed sources from the declaration. An empty `embed_template` uses the model
profile's template. Facets must also be filterable with the same type.

Use `LanguageConfig::Postgres { configuration: "english".into() }` for a
Postgres text configuration; `LanguageConfig::Sqlite { tokenizer:
"porter unicode61".into() }` for SQLite stemming; or
`LanguageConfig::Qdrant { tokenizer: "word".into(), stemmer: None,
stop_words: vec![], folding: true }` for Qdrant's text index.
`LanguageConfig::Server`, the default, uses each engine's defaults. MySQL and
MariaDB accept only server language settings. A language setting for another
engine is rejected.

## Query models

Import `suprnova::Searchable` to call `Article::search`. Fluent methods defer
validation; terminal methods return `Result`. Using the model above, you can
run this in an authenticated handler after importing the listed types:

```rust
use suprnova::{FrameworkError, Searchable};

async fn article_titles() -> Result<Vec<String>, FrameworkError> {
    let result = Article::search("gardem")
        .highlight(["title", "body"])
        .limit(10)
        .get()
        .await?;
    Ok(result.hits.into_iter().map(|hit| hit.model.title).collect())
}
```

These examples each start a new query over that model:

| Method | Example | Result or behavior |
|---|---|---|
| `filter` | `Article::search("garden").filter("price >= 10")` | Add typed AST or grammar predicates; repeated calls combine with AND. |
| `facets` | `Article::search("garden").facets(["category"])` | Request value counts. |
| `facet_stats` | `Article::search("garden").facet_stats(["price"])` | Request typed min/max. |
| `sort_by` | `Article::search("garden").sort_by("price", SortDirection::Asc)` | Order by a declared sortable attribute. |
| `sort_by_distance` | `Article::search("garden").sort_by_distance("location", GeoPoint { lat: 40.0, lng: -73.0 }, SortDirection::Asc)` | Order by meters from a point. |
| `group_by` | `Article::search("garden").group_by("category", 3)` | Keep a representative and up to two additional group members. |
| `highlight` | `Article::search("garden").highlight(["title"])` | Return formatted fields. |
| `crop` | `Article::search("garden").crop("body", 120)` | Format a bounded fragment around the first match. |
| `hybrid` | `Article::search("garden").hybrid(0.5)` | Fuse lexical and semantic candidates. |
| `rerank` | `Article::search("garden").rerank("BAAI/bge-reranker-base", 20)` | Rerank a bounded leading slice. |
| `limit` | `Article::search("garden").limit(10)` | Bound the returned page. |
| `offset` | `Article::search("garden").offset(10)` | Skip ranked hits. |
| `hits_cap` | `Article::search("garden").hits_cap(100)` | Reduce, never increase, the index's accessible cap. |
| `for_viewer` | `Article::search("garden").for_viewer(Viewer::Guest)` | Apply an explicit viewer's scope. |
| `with_trashed` | `Article::search("garden").with_trashed()` | Include soft-deleted rows only if the index also opts in. |
| `query` | `Article::search("garden").query(|query| query.filter("published", true))` | Adjust the Eloquent hydration query once. |
| `get` | `Article::search("garden").get().await?` | Return `SearchResults<Article>`. |
| `paginate` | `Article::search("garden").paginate(10, 2).await?` | Page 2, with 10 hits per page. |
| `simple_paginate` | `Article::search("garden").simple_paginate(10, 2).await?` | The same result envelope, including `pagination.has_more`. |
| `keys` | `Article::search("garden").keys().await?` | Return scoped, live-checked `Vec<String>`. |
| `raw` | `Article::search("garden").raw().await?` | Return `Vec<RawHit>` containing declared attributes. |
| `facet_search` | `Article::search("garden").facet_search("category", "out").await?` | Return matching facet values and counts. |
| `search_query` | `Article::search("garden").search_query().limit` | Inspect the typed plan without executing it. |

Import `SortDirection`, `GeoPoint`, and `Viewer` from `suprnova` for the table's
examples. `SearchBuilder::new` is the constructor behind `Model::search`.
Pagination uses `(per_page, page)`, with positive one-based pages. Keys break
score ties, so pages slice one ranked list on an unchanged index. The default
page limit is 20. The hit cap limits navigation, not exact lexical totals or
facets.

Each hit contains `model`, `key`, `engine_score`, `normalized_score` in [0, 1]
within the returned result, `engine`, `formatted`, optional `geo_distance`, and
`group_members`. The response adds `Total::Exact` or `Total::Estimated`,
`facets`, `facet_stats`, `processing_time_ms`, engine name, query, pagination,
and an optional analytics impression. Semantic top-k makes totals estimated.
Scores from different engines have different scales.

Hydration loads models through Eloquent. Missing rows are dropped and their
index removals are queued; stale rows outside scope are dropped too. The
`query` callback controls hydration, not engine candidates, totals, or facets.
Use search filters and the declared scope for access restrictions.

For several registered indexes, use federation:

```rust
use suprnova::{Search, Viewer};

let results = Search::federated(["articles", "notes"])
    .search("garden")
    .for_viewer(Viewer::Guest)
    .highlight(["title"])
    .paginate(10, 1)
    .await?;
```

Both indexes must declare the requested attributes. `FederatedBuilder` supports
`search`, `filter`, `for_viewer`, `facets`, `facet_stats`, `highlight`, `crop`,
`hybrid`, `limit`, `offset`, `get`, `keys`, and `paginate`. It merges per-index
ranks by reciprocal rank fusion with k = 60, using your index order for ties.
Keys remain `(index, key)` pairs; facets and stats remain grouped by index.
`Search::for_index("articles")?.search("garden")` provides runtime index
selection with filters, facets, highlights, attribute sorting, limits, offsets,
viewer selection, and a `get` returning declared projections after hydration.

## Filters

Filters name declared filterable attributes and bind values separately from
engine syntax. You can pass a `Filter` tree or a string to `.filter(...)`:

```rust
use suprnova::{AttributeValue, Filter, Searchable};

let results = Article::search("garden")
    .filter(Filter::Gte("price".into(), AttributeValue::Float(10.0)))
    .filter("category IN ['outdoors', 'books'] AND NOT published = false")
    .get()
    .await?;
```

The grammar includes every operator below:

| Grammar | Typed form | Meaning |
|---|---|---|
| `category = 'books'`, `category == 'books'` | `Filter::Eq` | Equality; list attributes match an element. |
| `category != 'books'` | `Filter::Ne` | Negated equality, including missing values unless you compare null. |
| `category IN ['books', 'outdoors']` | `Filter::In` | Any listed value. |
| `category NOT IN ['hidden']` | `Filter::NotIn` | Exclude any listed value. |
| `price > 10`, `price >= 10` | `Filter::Gt`, `Filter::Gte` | Exclusive/inclusive lower bound. |
| `price < 20`, `price <= 20` | `Filter::Lt`, `Filter::Lte` | Exclusive/inclusive upper bound. |
| `price 10 TO 20` | `Filter::Between` | Inclusive range. |
| `category IS NULL`, `category IS NOT NULL` | `Filter::IsNull`, `Filter::IsNotNull` | Missing/null or present. |
| `(published = true AND price < 20) OR category = 'books'` | `Filter::And`, `Filter::Or` | Boolean composition. |
| `NOT (category = 'hidden')` | `Filter::Not` | Negation. |
| `_geoRadius(40, -73, 1000)` | `Filter::GeoRadius` | Within 1000 meters; grammar requires exactly one filterable geo attribute. |

Keywords are case-insensitive. Precedence is NOT, AND, OR; parentheses override
it. Quote strings with single or double quotes and escape the next character
with a backslash. Numbers, booleans, and null are typed; datetime literals are
UTC-compatible timestamps such as `2026-01-01T00:00:00Z`. Ordered comparisons
exclude missing values. Use the AST to name a geo attribute when the index has
more than one. Invalid names, types, reversed ranges, negative radii, or
non-finite coordinates fail before engine access. Grammar input is bounded to
64 KiB and 4096 tokens; filter trees are bounded to 64 levels.

## Facets and stats

`.facets(["category"])` counts the full filtered lexical candidate set before
distinct, grouping, the hit cap, pagination, or semantic top-k. Counts remain
exact, including on Qdrant. The most frequent values survive `facet_value_cap`;
`FacetOrder::Alphabetical` changes their presentation after selection.

`.facet_stats(["price"])` returns typed `min` and `max` over the same set for
numeric or datetime facets. A facet without non-null values has no extrema.
`.facet_search("category", "out")` provides type-ahead values with counts
under the same query, filters, soft-delete policy, and scope. A value's count
equals the exact lexical total when you filter the query to that value.

## Sorting and geo distance

Attribute and distance sorts precede relevance in lexical and hybrid modes.
Missing values sort last in both directions; relevance and then the key break
ties. Repeated `sort_by` calls add ordered clauses. Geo sorts populate each
hit's `geo_distance` in meters. Radius filters and distance sorts use the same
Haversine model. `group_by("category", 3)` keeps the first ranked representative
and at most three total members, including it. The representative's
`group_members` holds the additional members. Distinct is a query setting,
`distinct_attribute`, that keeps the first hit for each value.

## Highlights and crops

Request `.highlight(["title", "body"])` or `.crop("body", 120)` to populate
`hit.formatted`. Corrected words, synonyms, original-word prefixes, and phrases
are marked in the original text. Crops count visible Unicode characters
around the first match and never cut a tag. Postgres uses native headline
matches; SQLite uses native FTS5 matches and snippets. MySQL, MariaDB, Qdrant,
and collection use the framework formatter.

Document text is HTML-escaped before tags are added. Settings accept matching
`mark`, `em`, `strong`, `b`, or `span` tags with at most one `class` attribute.
Event handlers, other attributes, and mismatched tags fail validation. Tags
come from code, never request parameters; the crop marker is escaped text.

In Live, use the `trusted_html` filter only on the checked `TrustedHtml` value
that the component supplies. For a `SearchBoxHit` named `hit`, the shipped
template uses:

```html
{% let markup = hit.highlight("title")? %}
{{ markup|trusted_html }}
```

Keep ordinary document text escaped. The kits render only core-produced
formatted fields as HTML.

## Typo tolerance, synonyms, and stop words

Typos expand query words against the generation's vocabulary. The budgets are
zero edits below 5 Unicode scalars, one from 5 through 8, and two from 9.
Damerau-Levenshtein distance counts a transposition as one edit. Numbers,
including numeric forms that overflow, are never corrected. Named words and
attributes in `disable_on_words` and `disable_on_attributes` stay exact for
typos; synonyms still apply. Quoted phrases and negative words do not receive
ordinary word correction. Tokens longer than 64 scalars skip correction.
Only the first 32 query tokens receive correction. Queries are bounded to
64 KiB and 1024 source tokens.

Set `query.typo_tolerance.enabled = false` to disable correction. Alternatives
include the original and are capped by `SEARCH_EXPANSION_TERMS` (64 per word),
with higher document frequency first and stable ties. Only the original last
word gets prefix matching, not its typo corrections. SQLite vocabulary comes
from `fts5vocab`; Postgres uses `ts_stat`, refreshed after builds and every
`SEARCH_VOCABULARY_REFRESH` writes (1000 by default). MySQL, MariaDB, and Qdrant
maintain document frequencies and remove zero-frequency words.

For example, add this in your settings function:

```rust
let mut settings = suprnova::IndexSettings::default();
settings.query.synonyms.push(suprnova::Synonym {
    terms: vec!["bike".into(), "bicycle".into()],
    one_way: false,
});
settings.query.synonyms.push(suprnova::Synonym {
    terms: vec!["nyc".into(), "new york".into()],
    one_way: true,
});
settings.query.stop_words.insert("the".into());
```

Two-way groups expand any member; one-way groups expand the first term to the
others. Multi-word source and replacement synonyms match as phrases. Query
stop words are independent of the engine's index-time stop words. Expansion
does not override an engine's minimum token length, stemming, or stop list.

## Hybrid search and embedders

`SEARCH_EMBEDDER=none` is lexical. A positive `.hybrid(ratio)` without an
embedder fails with an error naming `SEARCH_EMBEDDER`. With an embedder, zero
keeps lexical order, one keeps cosine order, and intermediate ratios use
weighted reciprocal rank fusion: lexical weight `1 - ratio`, semantic weight
`ratio`. Each leg takes `fusion_depth` candidates under the same filter and
scope. Qdrant performs native fusion; other engines use the framework's RRF
with k = 60. Ratios must be finite and between 0 and 1 inclusive. Sorts still
take precedence.

The `Embedder` trait exposes `profile`, `embed_documents`, `embed_query`, and
optional sparse operations. You choose `fastembed` or `http`, or register your
own provider with `SearchRegistry::register_embedder`. An index's
`index_time.embedder` overrides the deployment provider. Model profiles own
the model id and revision, dimensions, token context, query prefix, document
template, pooling, normalization, Matryoshka dimensions, precision, output key,
input kinds, and artifact files. The local media profile accepts image, audio
and video parts. The HTTP provider and default local profile accept text only.
Unsupported parts fail.

### Local inference

Enable the `search-fastembed` Cargo feature and select
`SEARCH_EMBEDDER=fastembed`. Enabling `search-fastembed` makes the `ort` crate's
build script download the ONNX Runtime library at compile time. Your build
machine needs network access for that feature. The default build downloads
nothing.

The default local model is `hf:onnx-community/embeddinggemma-2-ONNX`,
EmbeddingGemma 2 text, fp32, 768 dimensions, mean pooling, L2 normalization,
and an 8192-token context. Its model artifacts total about 1.04 GiB
(1,116,768,801 bytes). Model download happens on first load into
`SEARCH_MODEL_CACHE`, which defaults to `storage/search/models`.

You pass unprefixed text. The framework applies `task: search result | query: `
to queries and `title: {title} | text: {text}` to documents; an absent title
becomes `none`. Local inference truncates with the model tokenizer after
preparation, including prefixes and special tokens, and runs on blocking
threads. Vectors must have the profile's dimensions and finite values. A
supported Matryoshka reduction re-normalizes the truncated vector.

The shipped profiles are listed below. All normalize their output. Names also
resolve through their repository ids. BGE's instruction means
`Represent this sentence for searching relevant passages: `.

| Profile | Dimensions | Context tokens | Pooling | Query prefix / document template | Reduced dimensions |
|---|---:|---:|---|---|---|
| `embeddinggemma-2` | 768 | 8192 | Mean | `task: search result \| query: ` / `title: {title} \| text: {text}` | 512, 256, 128 |
| `embeddinggemma-2-media` | 768 | 8192 | Mean | Same as EmbeddingGemma 2, with ordered media parts | 512, 256, 128 |
| `embeddinggemma-300m` | 768 | 2048 | Mean | Same as EmbeddingGemma 2 | 512, 256, 128 |
| `bge-small-en-v1.5` | 384 | 512 | CLS | BGE instruction / `{text}` | None |
| `bge-base-en-v1.5` | 768 | 512 | CLS | BGE instruction / `{text}` | None |
| `bge-large-en-v1.5` | 1024 | 512 | CLS | BGE instruction / `{text}` | None |
| `bge-m3` | 1024 | 8192 | CLS | Empty / `{text}` | None |
| `multilingual-e5-small` | 384 | 512 | Mean | `query: ` / `passage: {text}` | None |
| `multilingual-e5-base` | 768 | 512 | Mean | `query: ` / `passage: {text}` | None |
| `multilingual-e5-large` | 1024 | 512 | Mean | `query: ` / `passage: {text}` | None |
| `nomic-embed-text-v1.5` | 768 | 8192 | Mean | `search_query: ` / `search_document: {text}` | 512, 256, 128, 64 |
| `gte-base-en-v1.5` | 768 | 8192 | CLS | Empty / `{text}` | None |
| `gte-large-en-v1.5` | 1024 | 8192 | CLS | Empty / `{text}` | None |
| `mxbai-embed-large-v1` | 1024 | 512 | CLS | Empty / `{text}` | 512 |
| `snowflake-arctic-embed-xs` | 384 | 512 | CLS | Empty / `{text}` | None |
| `snowflake-arctic-embed-s` | 384 | 512 | CLS | Empty / `{text}` | None |
| `snowflake-arctic-embed-m` | 768 | 512 | CLS | Empty / `{text}` | None |
| `snowflake-arctic-embed-l` | 1024 | 512 | CLS | Empty / `{text}` | None |
| `all-MiniLM-L6-v2` | 384 | 256 | Mean | Empty / `{text}` | None |
| `all-MiniLM-L12-v2` | 384 | 128 | Mean | Empty / `{text}` | None |

Nomic's 8192-token context requires an export with the model's RoPE scaling.
The shipped Arctic and mxbai profiles use no retrieval prefix; declare a
custom profile if you need another preparation convention.

`SEARCH_EMBED_MODEL` accepts a fastembed named model, `hf:owner/repository`,
`hf:owner/repository@revision`, or `path:/absolute/model-directory`. For an
unrecognized export, provide `FastembedConfig.profile: Some(ModelProfile)` in
registered configuration. Declare the graph, optional external tensor file,
`tokenizer.json`, `config.json`, `tokenizer_config.json`,
`special_tokens_map.json`, pooling, precision, and optional output key.

`SEARCH_EMBED_PRECISION` accepts `fp32`, `int8`, or `q4`; fp16 is refused.
The default graph names are `onnx/model.onnx` with `onnx/model.onnx_data`,
`onnx/model_quantized.onnx`, and `onnx/model_q4.onnx`, respectively. Dynamic
quantization uses singleton inference so document batches and queries agree.
BGE-M3 sparse inference uses its joint int8 export and external tensors.
`SEARCH_EMBED_BATCH` otherwise bounds inference batches, default 64. A local
model counts it in texts of 512 tokens: attention memory grows with the square
of a batch's longest text, so a longer text takes the room of several by that
square, and one longer than that runs alone. Texts of similar length share a
batch, measured with the model's own tokenizer.
`SEARCH_EMBED_DIMENSIONS` selects a supported reduced dimension.

Each cache directory has `manifest.json` with repository, resolved revision,
recording time, and a SHA256 and size for every file. Local directories receive
an artifact-derived revision. Generated special-token maps and the text graph
that binds unused media inputs to empty tensors are recorded as derived files
with their own hashes and derivation descriptions. Every cached file is
verified before loading; a changed byte is refused. A verified cache works
offline. Keep the manifest with its artifacts. Generation settings pin the
resolved model revision and dimensions; `ModelProfile::provenance()` exposes
the manifest digest and file hashes, and baseline records include them.

### Media embeddings

Enable `search-fastembed`. Set `SEARCH_EMBEDDER=fastembed` and
`SEARCH_EMBED_PROFILE=embeddinggemma-2-media`.
The default model stays `hf:onnx-community/embeddinggemma-2-ONNX`.
The media profile verifies the text, vision and audio graphs, their external
weights and processor configuration in one cache manifest. It returns 768
normalized values. The fp32 graphs, weights and tokenizer need about
2.8 GiB in the model cache. Text-only documents keep the text batch planner.

Declare storage path fields in the order the model should receive their parts:

```rust
#[searchable(embed)]
pub caption: String,
#[searchable(media(kind = "image", disk = "private"))]
pub poster: Option<String>,
#[searchable(media(kind = "audio", disk = "private"))]
pub soundtrack: String,
#[searchable(media(kind = "video", disk = "private"))]
pub clip: Option<String>,
```

These attributes belong on fields of a `Searchable` model. A null optional
path contributes no part. A model with only media paths embeds empty text
with its declared parts. The worker reads each path asynchronously through
`Storage`. `SEARCH_MEDIA_MAX_BYTES` limits each part to 33554432 bytes by
default. Set a smaller positive limit to reduce worker memory. Larger limits
are refused. A size check and a bounded range read apply to every part.
Store changed media at a new path when reindexing an existing document.

Images accept PNG, JPEG and WebP. Audio accepts PCM WAV, mixes channels to
mono and resamples to 16 kHz with a Hann-windowed sinc filter. It computes
the model's log-mel features and mask.
Audio is limited to 30 seconds. Image decoding has dimension and memory caps.
The default `search-media` feature exposes the Rust processors without ONNX.

Video requires a host `ffmpeg` binary. `SEARCH_FFMPEG_BINARY` selects its path
and defaults to `ffmpeg`. A missing binary fails with that variable's name.
The worker samples frames at 1 fps, keeps at most 32 frames and applies the
video processor's 140-token image budget. Long clips use uniform samples
across their duration. Each decode has a 30-second timeout.
Temporary input and frame files are removed when decoding ends.

Boot and `search:sync-index-settings` reject a declared kind that the model
profile cannot accept. The error names the index and model. The default
EmbeddingGemma 2 profile and the `http` provider remain text only. The HTTP
provider refuses media even if a caller supplies a custom profile.

### OpenAI-compatible HTTP inference

Select `SEARCH_EMBEDDER=http`, set `SEARCH_EMBED_BASE_URL` to the deployment's
base URL, and set `SEARCH_EMBED_MODEL`. The provider sends
`POST {base}/embeddings` with the OpenAI-compatible shape:

```json
{"model":"your-model","input":["prepared text"],"dimensions":768,"encoding_format":"float"}
```

`dimensions` is omitted when you do not request it. The response is
`{"data":[{"index":0,"embedding":[0.1,0.2]}]}`, with actual vectors matching
the configured dimensions. Response indexes must cover each batch exactly
once; the provider restores input order. Unknown models use configured
dimensions or one boot-time probe, an empty query prefix, and `{text}` as the
document template. `SEARCH_EMBED_QUERY_PREFIX` and
`SEARCH_EMBED_DOCUMENT_TEMPLATE` override preparation for unknown models.
Known models use their shipped profile. You can supply an explicit profile
through `HttpEmbedder::with_profile`.

The destination comes from deployment configuration, never a request or
tenant. HTTPS is required except for `localhost`, `127.0.0.1`, and `::1`.
URLs with credentials, query strings, or fragments are rejected. No redirects
are followed. `SEARCH_EMBED_API_KEY` is a bearer key sent only to the pinned
destination. Defaults are 64 inputs per batch, a 30-second timeout including
the response body, 16 MiB per response, and three additional attempts for 429
or 5xx only. Retries are capped at 16; `Retry-After` is bounded. HTTP uses no
provider-specific fields. Hosted models with published limits are built in:
OpenAI's `text-embedding-3-small`, `text-embedding-3-large` and
`text-embedding-ada-002` (8192 tokens), and Google's `gemini-embedding-001`
(2048 tokens) and `gemini-embedding-2` (8192 tokens), with or without an
`owner/` prefix such as OpenRouter's `openai/text-embedding-3-small`. Any
other model outside the shipped profiles takes its context from the
service's model list: OpenRouter's `embeddings/models` (`context_length`),
or `models` with vLLM's `max_model_len`, LM Studio's or Mistral's
`max_context_length`. The list is read on the first embedding, not at boot,
so the application starts while the service is down. When the list names no
context for the model, the provider uses 8192 tokens, the limit OpenAI,
Mistral and Google's newer models publish; when the list cannot be read, it
uses 8192 for that embedding, logs a warning, and reads the list again next
time. `SEARCH_EMBED_MAX_TOKENS` wins over the profile, the list and the
default, for a model with a smaller context or a server that caps input
below it.
Every prepared input is cut to the model's context before it is sent. Set `SEARCH_EMBED_TOKENIZER` to the model's own
`tokenizer.json`, published beside the weights in its Hugging Face repository,
and the cut falls after the last token that fits. Without it the protocol
gives no way to count tokens, so the cut is in UTF-8 bytes, four short of the
context: it never exceeds the context, and it keeps about a quarter of what
the model can read in English text. Suprnova never downloads the file.

## Reranking

`.rerank("BAAI/bge-reranker-base", 20)` runs a named fastembed cross-encoder on
the leading 20 representatives after retrieval, distinct, and grouping, before
pagination. It replaces their order and leaves the remaining tail in order.
You need `search-fastembed` for shipped local rerankers. Their runtimes load
lazily when first used; boot registers supported repository ids, short names,
and enum spellings without downloading every model. You can register a custom
`Reranker` with `SearchRegistry::register_reranker`; its positional scores must
be finite. A missing reranker or invalid depth fails at the terminal method.

## Engines

Set `SEARCH_DRIVER` globally or call
`SearchRegistry::set_index_driver("articles", "qdrant")?` at bootstrap. Register
custom engines with `SearchRegistry::register_engine`. To move an index, select
its destination driver, configure that service, then run `search:reindex`
against it. Query call sites stay the same. Keep the old service available
until its queued work drains, and remove its storage separately when retired.

### Database

The database engine uses your application's connection, including a model's
named connection, and a side table per index generation. It stores typed
attributes, versions, hashes, and generation metadata beside native indexes.

| Dialect | Native index and rank | Tokenization, stemming, stop words, minimum length | Prefixes, phrases, highlights |
|---|---|---|---|
| SQLite | FTS5 external-content table, triggers, weighted `bm25()` | `unicode61` by default, case/diacritic folding, no stop list or server minimum; optional `porter unicode61` stems English | Last-word `*`, adjacent quoted phrases; native `highlight()` and `snippet()` matches |
| Postgres | Stored weighted `tsvector`, GIN, `ts_rank` normalized by document length | The server's `default_text_search_config` by default, copied when a generation is created so a later server change cannot split its vectors from its queries; a named configuration overrides it (`english` stems and drops stop words, `simple` keeps every word); no InnoDB minimum | `:*`, `<->` phrase adjacency; native `ts_headline` matches |
| MySQL | Per-column FULLTEXT plus combined FULLTEXT, weighted sum of `MATCH ... AGAINST (... IN BOOLEAN MODE)` | Server parser and collation; no framework stemming; InnoDB minimum commonly 3 and server stop words apply | Native `*` and quoted phrases, subject to indexed tokens; framework formatting |
| MariaDB | Same FULLTEXT and weighted boolean relevance path | Server parser and collation; no framework stemming; server token minimum and stop words apply | Native `*` and quoted phrases, subject to indexed tokens; framework formatting |

Postgres maps descending distinct attribute weights to its four A/B/C/D
classes; weights beyond four classes share D. A generation built with the
server's language owns a text search configuration named after its table with
a `_ts` suffix; the engine drops it with the generation. Ranking uses `ts_rank`
rather than `ts_rank_cd`: cover density rewards query words that sit close
together, which suits all-word queries, while a search matches any query
word. MySQL/MariaDB text matching
follows the index collation, while filters and facets preserve exact values.
`search:status` reports `innodb_ft_min_token_size`, `innodb_ft_enable_stopword`,
and the server/user stop-word table settings. Configure these on the server and
rebuild affected indexes. Suprnova does not disguise document tokens to bypass
native limits, so short words or stop words can match differently by dialect.

For a database index's semantic leg, register a [Vector driver](vector.md)
under the index name, or set `SEARCH_VECTOR_STORE` to a registered store name.
You can use any Vector driver. Points carry generation and content identities;
durable pending delivery repairs interrupted writes. Retrieval asks the store
for four times the fusion depth, applies the SQL filter and scope to those
keys, and doubles the request until the filtered depth is complete or the
store is exhausted. A Vector driver cannot filter, so a narrow filter, such as
one user's private documents among many users' documents, can read most of
the store on every semantic query. Qdrant applies the filter inside its own
index, so prefer it for large, narrowly scoped data. Keep the store persistent
when your search index must survive process exits.

### Qdrant

Set `QDRANT_URL` to the gRPC URL, normally port 6334, and optionally
`QDRANT_API_KEY`. `SEARCH_QDRANT_TIMEOUT_SECS` bounds calls, default 10 seconds.
Install the SQL search migrations too: generations, pending work, tombstones,
and vocabulary use your application's database.

Qdrant 1.19 or later is required. Each `search_<index>_<generation>` collection
has a named cosine `dense` vector, a language-configured text payload index,
typed payload indexes, keys, and source versions. A lexical index needs no dense
model; its unused dense schema has size 1.

Default lexical ranking uses Qdrant's own `qdrant/bm25` model. Qdrant encodes
each weighted searchable attribute's text on the server into a named
`bm25_<attribute>` sparse vector with the IDF modifier. A Qdrant formula sums
the attribute scores multiplied by their declared weights. This works on
self-hosted Qdrant without `search-fastembed` or a model download. Setting
`index_time.sparse_model = Some("bm25".into())` selects the same default path.
Reindex existing default Qdrant generations to adopt this vector layout.

Document and query inference use identical options from the index's language
settings: tokenizer, stemmer, stop words, and ASCII folding. The default uses
`word`, ASCII folding, no stemmer, and an empty stopword set. Disabling stemming
and stopword removal uses Qdrant's language-neutral options, which need Qdrant
1.19 or later. `LanguageConfig::Server` uses these defaults too.
`SEARCH_BM25_AVERAGE_LENGTH` supplies `avg_len`, default 256. Qdrant's defaults
for `k` (1.2) and `b` (0.75) stay in effect. Reindex after changing the average
length. Text payload indexes enforce words, phrases, and exclusions separately
from ranking. Normalized keyword words support prefix filters and the typo
vocabulary; they never score a hit.

Native queries keep the source spelling for ranking, phrases, and exclusions.
Qdrant tokenizes that text itself; normalized typo and synonym alternatives
are added separately. Keyword words and typo expansion split `word` and `prefix`
text on non-alphanumeric characters and `whitespace` text on whitespace, then
lowercase and fold accents as configured. For `multilingual`, keyword words and
expansion use whitespace as an approximation because Qdrant segments text with
language dictionaries. Ranking and text predicates still use Qdrant's own
multilingual segmentation. Reindex existing generations to update keyword words.

For a learned sparse model, configure `SEARCH_EMBED_SPARSE_MODEL` on the local
provider and set the generation's `sparse_model` to that registered provider's
name, such as `fastembed`. Providers must use one vector space for documents
and expanded queries, returning finite weights with sorted unique token ids.
Learned sparse document fields combine their declared attribute weights.
These providers use one named `sparse` vector with the IDF modifier.
Qdrant performs native weighted RRF for hybrid queries.

Qdrant's default text tokenizer is `word`; supported alternatives are
`whitespace`, `prefix`, and `multilingual`. Code settings can change its
tokenizer, stemmer, stop words, and ASCII folding, requiring a generation.
The text index lowercases tokens, enables phrase matching, and indexes tokens
from one character. The default has ASCII folding and no stemmer or stop list.
Text predicates support any-word matching, prefixes, required per-field
phrases, and exclusions; payload filters and exact facets still apply.
The default BM25 leg uses the same language options as the text predicates.

### Collection and null

Collection stores documents and generations in memory and uses deterministic
BM25, the framework's case/diacritic-folding tokenizer, no native stemming or
stop list, no minimum token length, original-last-word prefixes, and
field-local phrase matching. Use it for tests with fresh engine instances.
It does not persist between processes. Null acknowledges writes, retains
generation settings, and returns empty results; it reports no semantic or
lexical retrieval capabilities.

## Indexing

Model create, update, restore, delete, and force-delete hooks synchronize search.
Ambient transactions, explicit handles, and nested savepoints defer work until
the outer commit; rollback discards it. With `SEARCH_QUEUE=true`, the default,
the configured connection and queue receive registered framework jobs. Run a
worker for `SEARCH_QUEUE_CONNECTION` and `SEARCH_QUEUE_NAME`, for example:

```bash
cargo run -- queue:work --connection search --queue search
```

Set those names only after configuring the corresponding queue connection.
With `SEARCH_QUEUE=false`, deferred inline work runs after commit. Scoped model
mutations synchronize inline after commit even when queuing is enabled, before
the request returns, so changed access does not wait for a worker.

`SearchIndexJob` carries index, key, source version, operation, and optional
generation. It reloads authoritative rows on execution and removes missing or
ineligible rows. Chunk jobs load and prepare a whole chunk before projecting
it. Older versions cannot replace newer ones. Equal content hashes with the
same generation settings skip embedding and engine writes while preserving
the version watermark.

Removal tombstones prevent delayed retries from resurrecting documents for
`SEARCH_TOMBSTONE_HOURS`, default 24 hours. Schedule `search:status --prune`
to expire tombstones and clean eligible retired generations:

```rust
schedule.add(schedule.try_command("search:status --prune")?.daily());
```

Soft-deleted models normally leave the index. Add `#[searchable(soft_delete)]`
to retain a filterable `__soft_deleted` flag, and call `.with_trashed()` to read
them. Both declaration and query must opt in.

Bulk APIs are `Article::make_all_searchable().await?`,
`Article::remove_all_from_search().await?`, and the free functions
`suprnova::search::searchable::<Article>(&keys).await?` and
`unsearchable::<Article>(&keys).await?`, where keys are `String`s. They chunk
work by `SEARCH_CHUNK`, default 500. Mass `update_all`, `delete_all`, and
`force_delete_all` do not emit model events. Import after mass updates; flush
then import after hard mass deletes so orphaned documents leave the index.

To suppress automatic synchronization within one awaited operation, use
`Article::without_syncing_to_search(|| async { /* model writes */ Ok(()) })`.
It is task-local; other tasks continue syncing. Reimport the affected rows
afterward. Declare `#[searchable(touches("comments"))]` on a parent whose
document depends on that relation. `search_touches` captures dependent parent
keys when related models change, then reloads parents and their chunk hook.

### Generations and switching

`search:reindex` and index-time settings changes build a hidden generation.
Live writes target both current and building generations. A durable pending-job
ledger, separate from queue storage, records deliveries and transaction-local
intents. After bulk import, the coordinator drains work through a watermark,
checks under the publication fence, and atomically switches readers. The old
generation remains until no pending job references it.

Drain waits have a 30-second deadline. A stalled worker or continuous writes
can time out a build while the current generation remains readable; rerun to
resume. Status recovers interrupted dispatches. SQL and external Qdrant/vector
services do not share an atomic transaction; retained job identities and
authoritative reloads support retries after interrupted acknowledgements.

## Scopes and the viewer

An index scope returns a `Filter` over the filterable fields its
`scope_fields` name; a scope that reads any other field fails the read with an
error naming the field. A recommendation event records the target's values
for exactly those fields as its access boundary, so name only what the scope
reads, for every viewer, signed in or not. Every read
path applies it inside engine candidates: hits, keys, raw projections, facets,
stats, totals, groups, federation, the JSON endpoint, and recommendation reads.
The request supplies `Viewer::current()` through Auth. Background code can use
`.for_viewer(viewer)`; without a request identity it receives `Viewer::Guest`.

Hydration rechecks the live row against the scope. Writes to scoped models
refresh access after commit before returning. Keep ownership and visibility
in the declared filterable document and scope, not just in a hydration callback
or a presentation filter. The model example above limits reads to its owner.

## Analytics and suggestions

Set `SEARCH_ANALYTICS=true` to retain searches. It is off by default. The
`search_queries` table records index, original and folded query, filters, hit
count, and processing time. `SEARCH_ANALYTICS_VIEWER=true` additionally stores
authenticated viewer ids; guest identities are never stored in query rows.
`search_impressions` holds a session hash and the exact shown keys with
zero-based positions. `search_clicks` records accepted clicks.

`Search::click(&impression, &key, position).await?` accepts only an unexpired
impression from the current server session containing that key at that
position. Analytics-disabled, missing, expired, other-session, and forged
position reports fail. With [recommendations](recommendations.md) enabled,
shown impressions and validated clicks also become recommendation events.

`Search::suggest("articles", "gar", 10).await?` returns Unicode-case-folded
retained queries for that index by frequency with stable ties. The limit is
at most 1000; disabled analytics returns none. `search:analytics` reports top
queries, zero-result queries, and click positions. Retention defaults to 90
days through `SEARCH_ANALYTICS_RETENTION_DAYS`. Schedule pruning explicitly:

```rust
schedule.add(schedule.try_command("search:analytics-prune")?.daily());
```

Pruning deletes linked clicks, impressions, and queries and also expires search
tombstones. Recommendation event retention remains an independent application
decision.

## Search pages in RenderCache

A search that runs on a route opted into [RenderCache](render-cache.md) is
stored like any other render. The stored page depends on each index the
search read. Any change to one of those indexes makes the page stale, so the
next request renders it again:

- Indexing or removing documents, one at a time or in bulk, including
  imports and queued `SearchIndexJob` work.
- Flushes, generation creation, reindexing, generation switches, and index
  deletion.
- Settings sync.
- Tombstone pruning and pending vector writes.

A write to another index leaves the page alone. Each logical index is one
dependency, so a switch to a new generation invalidates the page the same way
a document write does.

Declare every query parameter the search reads. A request that carries an
undeclared query parameter bypasses the cache, and each declared value gets
its own entry, so page 2 is stored apart from page 1:

```rust
use suprnova::{FrameworkError, Router};
use suprnova::render_cache::{
    FreshnessPolicy, QueryPolicy, RenderCachePolicy, RepresentationClass,
};

fn cache_docs_search(router: Router) -> Result<Router, FrameworkError> {
    router.try_render_cache(
        "/docs/search",
        RenderCachePolicy::builder(RepresentationClass::PublicShared)
            .freshness(FreshnessPolicy::new(300_000, 60_000, 300_000)?)
            .query(QueryPolicy::declared(["q", "page"]))
            .build()?,
    )
}
```

Some searches must run on every request. RenderCache serves the page they
render but never stores it, and records a `declined` outcome with one of these
reasons (see [RenderCache Operations](render-cache-operations.md)):

| Search | Reason |
|---|---|
| The index declares a [scope](#scopes-and-the-viewer), or the search calls `.for_viewer(...)`. | `viewer_scoped_search` |
| `SEARCH_ANALYTICS=true` is set, so each request records an impression. | `analytics_search` |
| The search calls `.without_render_cache()`. | `search_opt_out` |

When you turn analytics on, RenderCache also stops serving search pages that
it stored while analytics was off.

`.without_render_cache()` keeps one search out of the cache. It is available
on `Model::search`, `Search::for_index`, and `Search::federated`:

```rust
use suprnova::{FrameworkError, Search};

async fn live_stock_count(query: &str) -> Result<usize, FrameworkError> {
    let results = Search::for_index("inventory")?
        .search(query)
        .without_render_cache()
        .get()
        .await?;
    Ok(results.hits.len())
}
```

Inside a cached render, an unscoped search with analytics off runs as
`Viewer::Guest` and does not read the session. Without a scope the viewer
changes no result, and a session read would make the page uncacheable.

A custom engine needs no extra code. The registry guards each of its writes
and advances the index after the write finishes.

## JSON endpoint

Register an explicit allowlist with `Search::routes(&["articles"])?`, which
returns a `Router`, or `router.try_search_endpoint(&["articles"])?` on your
existing router. Unknown or duplicate registration names fail. Attach session
and your authentication middleware, and apply your rate-limiting policy.
Analytics-enabled endpoint requests require a nonempty server session.

`GET /search` accepts:

| Parameter | Default | Meaning |
|---|---|---|
| `index` | None | An allowlisted index name. |
| `q` | Empty | Query words, phrases, and exclusions. |
| `filter` | None | The typed filter grammar above. |
| `facets` | None | Comma-separated declared facet attributes. |
| `sort` | None | Comma-separated `attribute:asc` or `attribute:desc`; omitted direction is ascending. |
| `page` | 1 | Positive one-based page. |
| `per_page` | 20, bounded by the index cap | Positive page size, at most the effective hit cap. |

For example, `GET /search?index=articles&q=gardem&page=1&per_page=10` returns:

```json
{
  "hits": [{
    "attributes": {"title": {"type": "text", "value": "Garden planning"}},
    "key": "1",
    "score": 2.5,
    "normalized_score": 1.0,
    "formatted": {"title": "<em>Garden</em> planning"}
  }],
  "facets": {},
  "total": 1,
  "total_estimated": false,
  "page": 1,
  "per_page": 10,
  "processing_time_ms": 2,
  "impression": "550e8400-e29b-41d4-a716-446655440000"
}
```

The example abbreviates the document attributes. The actual response contains
only declared attributes, using `AttributeValue`'s `type`/`value` encoding,
and no additional hydrated model fields. `impression` is present only when
recorded. An unlisted or missing index returns a non-disclosing 404. Invalid
filters, facets, sorting, page sizes, or page numbers, and a `q` over 65536
bytes or 1024 words, return 422 errors naming the failing parameter;
undeclared attributes are refused.

`POST /search/click` accepts JSON
`{"impression":"...","key":"1","position":0}` and returns
`{"recorded":true}` after validation. It requires the same session and a
valid CSRF token under the [CSRF middleware](csrf.md).

## Live search-box

Install the shipped component with `suprnova live:add search-box`. It installs
the `search-input` dependency, Askama macro and island templates, stylesheet,
and JavaScript, and registers `suprnova::live::components::SearchBox`.
Mount it with server-owned parameters `{"index":"articles","per_page":10}`;
`per_page` must be 1-100. The index cannot change through a model proposal.

The presentational macro's arguments are:

```text
search_box(name, label, query, hits, facets, filters, impression,
           sequence=0, control_id="query")
```

The island binds query text with a 250 ms debounce and renders highlighted
results and optional facet checkboxes. Arrow keys, Home, End, Enter, and Escape
control the combobox/listbox. A query sequence rejects stale results, including
repeated text from an older request. Each mount owns its control ids. Clear
filters remains available when selected facets produce no results.

Selection validates the key against the signed shown list, records its click
when analytics supplies an impression, and emits `search.selected` version 1.
The browser event is `suprnova:search.selected` with `{index, key}`. Use it to
navigate or update the hosting page. See [Live](live.md) for mounting and
event handling. The Live kit's notes list uses the same scoped model search
and checked highlights within its own note-list component.

## Starter kits

The Svelte, React, Vue, and Live kits search notes through `Note::search` with
title/body highlights and an owner scope. Inertia search cursors bind the query
and offset; the Live list loads ranked pages and records only displayed
impressions. Empty search keeps the notes listing behavior.

Register an account first, then run:

```bash
cargo run --bin console -- db:seed
```

`NotesSeeder` attaches three example notes to the first registered account:
Garden planning, Ocean journal, and Library reading. It creates no account of
its own and fails with registration instructions when none exists. Repeated
seeding does not duplicate those titles for that owner. Sign in as that account
and search `oceam` to find Ocean journal through a one-letter typo. For rows
that predate search, run `cargo run --bin console -- search:import notes`.

The kit migrator lists both `search::migrations()` and
`recommendations::migrations()`. Bootstrap provisions the initial notes index,
registers `NotesSeeder`, and schedules search maintenance. The default search
driver uses the kit's database.

## Relevance and latency baseline

`search:baseline` measures the complete committed SciFact BEIR test split:
5183 documents, 300 queries, and 339 judgments. Claims and judgments are
CC BY 4.0; abstracts are ODC-By 1.0. Attribution, licenses, corpus, queries,
and judgments live in `framework/tests/search/fixtures/`.

Calibration indexes the same corpus into Meilisearch and Typesense, waits for
imports, and measures each rival over a small grid of its own search settings:
Meilisearch's `matchingStrategy` (`last`, `frequency` and `all`), and
Typesense's `drop_tokens_threshold` (1 and 10) with `text_match_type`
(`max_score` and `sum_score`). Every grid entry warms every query, then records
nDCG@10 and p50/p95/p99 latency. The entry with the highest nDCG@10 becomes the
rival's score; a tie goes to the lower p95. `baseline.json` keeps the chosen
settings and the whole grid. The committed calibration is:

| Rival | Version | Chosen settings | Lexical nDCG@10 | p95 |
|---|---|---|---:|---:|
| Meilisearch | 1.55.0 | `matchingStrategy` `frequency` | 0.4665 | 2.232596 ms |
| Typesense | 30.2 | `drop_tokens_threshold` 10, `text_match_type` `max_score` | 0.4274 | 52.005666 ms |

Lexical mode must reach the stronger rival minus the committed absolute
tolerance, 0.05: the floor is about 0.416543. Hybrid mode at the benchmark's
ratio 0.5 must exceed 0.466543. Every engine's committed p95 budget is
104.011332 ms, about 104.0 ms. It is two times the slower rival's p95, with its
chosen settings, from the same calibration run. If a rival refuses a request,
calibration stops with the error and leaves `baseline.json` as it was.
`search:baseline calibrate` writes the budget and
the loader re-derives it; a hand-edited budget that disagrees is refused.

Supply `SEARCH_BASELINE_MEILISEARCH_URL`, `SEARCH_BASELINE_MEILISEARCH_KEY`,
`SEARCH_BASELINE_TYPESENSE_URL`, and `SEARCH_BASELINE_TYPESENSE_KEY` through
your deployment's environment, then run from this repository:

```bash
cargo run --release -p app --bin console -- search:baseline calibrate
cargo run --release -p app --bin console -- search:baseline run --engine sqlite
```

Review and commit the replacement `baseline.json` after calibration.
`SEARCH_BASELINE_TOLERANCE` defaults to 0.05. An uncalibrated fixture is refused
unless you explicitly set `SEARCH_BASELINE_ALLOW_UNCALIBRATED=1`; that override
does not turn provisional numbers into rival measurements.

Framework measurements warm an isolated generation with a fixed vocabulary.
Each sample asks for the first page of 10 hits at the depth the builder
requests for that page. The engine sample includes parsing, expansion,
retrieval, and fusion. Query
inference has separate percentiles. Corpus embedding, indexing, vocabulary
preparation, and hydration are outside that sample; vocabulary preparation is
reported separately. Records under `target/search-baseline/<engine>.json`
include engine, model profile and provenance, fixture digest, rival versions,
measurement time, and tree hash.

`BudgetCheck::for_build()` enforces p95 only in an optimized build and reports
it in a debug build. Relevance floors apply in both. The rivals run as release
binaries, so the release gate measures with
`cargo nextest run --cargo-profile release`. For example:

```bash
cargo nextest run --cargo-profile release -p suprnova --test search \
    -E 'test(/^baseline::baseline_(collection|sqlite)$/)'
```

Live engine measurement tests have a fifteen-minute nextest limit. When
`SEARCH_EMBEDDER` is set to a model provider, the corpus is embedded first;
run `cargo nextest run --profile search-model` for its six-hour limit. Add
`--features search-fastembed` for local inference, and `--run-ignored only`
when selecting ignored service or real-model tests. Postgres and MySQL baseline
runs read `PG_TEST_URL` and `MYSQL_TEST_URL`; Qdrant service tests use
`QDRANT_TEST_URL`, while the console uses `QDRANT_URL`. Use disposable test
databases. These test variables are listed in [Environment Variables](env-vars.md).

## Matching rule

A document needs any ordinary query word. The engine's own relevance ranks
documents holding more of those words higher. Quoted phrases are required,
and minus-prefixed words are excluded. The last ordinary word also matches
prefixes. Engines can order the same matches differently.

For `apple pie "fresh baked" -green`, with no additional typo or synonym
alternatives, the renderings are:

| Engine | Rendered predicate |
|---|---|
| Collection | `(apple OR pie*) AND phrase("fresh baked") AND NOT green` |
| SQLite | `(("apple") OR ("pie"*)) AND "fresh baked"`, with `rowid NOT IN (... MATCH '"green"')` |
| Postgres | `((('apple')) \| (('pie':*))) & ('fresh' <-> 'baked') & !('green')` |
| MySQL | `(pie*) ("apple") +"fresh baked" -"green"`, plus a MATCH guard requiring an ordinary positive word |
| MariaDB | `(pie*) ("apple") +"fresh baked" -"green"`, with the same guard |
| Qdrant | `must=[should(text_any(apple), text_any(pie) OR prefix(pie)), per-field phrase("fresh baked")]; must_not=[text(green)]` |
| Null | No hits. |

MySQL and MariaDB put the prefix group first because their boolean parser can
ignore a prefix after a quoted term. Runtime values remain bound even when
the engine query syntax contains these rendered clauses.

## Commands

Run commands through your application's console binary, such as
`cargo run --bin console -- search:status`. Unknown options fail.

| Command | Arguments and flags | Purpose |
|---|---|---|
| `search:index` | `<index>` | Create an initial generation. |
| `search:import` | `<Model\|index>` | Import eligible rows in chunks. |
| `search:flush` | `<Model\|index>` | Remove indexed documents with versioned removal fences. |
| `search:delete-index` | `<index>` | Drain pending references and remove index storage. |
| `search:sync-index-settings` | `[index]` | Sync one or all declared indexes; report classes and build as needed. |
| `search:reindex` | `<Model\|index>` | Build or resume a generation and publish it after draining. |
| `search:status` | `[index] [--prune]` | Report generations and server token limits; optionally prune. |
| `search:analytics` | `[--index name] [--days N]` | Report retained demand, misses, and click positions; default 30 days, bounded by retention. |
| `search:analytics-prune` | None | Delete expired analytics and prune tombstones. |
| `search:baseline` | `calibrate` or `run [--engine collection\|sqlite\|postgres\|mysql\|qdrant]` | Replace calibration or measure engines; run defaults to collection and SQLite. |

Model names can be unambiguous short names or fully qualified names.
`search:index`, `search:delete-index`, settings sync, and status take index names.
The full [environment reference](env-vars.md#search) covers queue, embedding,
Qdrant, analytics, and calibration controls.

## Testing search

Use a fresh collection engine and isolated registry for deterministic tests.
This runnable async test requires no database or download:

```rust
#[tokio::test]
async fn collection_search_conforms() -> Result<(), suprnova::FrameworkError> {
    use std::sync::Arc;
    use suprnova::search::{CollectionEngine, testing::conformance};

    conformance::run(Arc::new(CollectionEngine::new()), conformance::Options::default()).await
}
```

The `testing` feature exposes conformance. Its isolated generations exercise
filters, facets, stats, ranking, scope predicates, phrases, exclusions, sorts,
geo, versioned writes, and generation switching. Semantic checks use
three-dimensional vectors; configure a persistent vector store accordingly.
It restores an existing current generation on success; a new index retains an
empty active generation. `Options` lets you choose an isolated `index_name`
and optional `test_semantic`/`test_geo` checks.

For model tests, `SearchRegistry::isolated(async { ... }).await` separates
services. `SearchConfig::scope(config, future).await?` scopes configuration;
`FakeEmbedder::new(16)?` gives deterministic normalized vectors without network
or inference. Register it under a provider name and select that name in the
index's settings. Keep real-provider tests separate from engine conformance.

### Why Suprnova diverges

Laravel Scout supplies drivers for hosted search products and a model-oriented
builder. Suprnova keeps native ranking in your database or Qdrant, then adds
query expansion, formatting, and fusion around it. There are no Algolia,
Meilisearch, or Typesense search drivers; the baseline measures against
Meilisearch and Typesense on the same judged corpus.

Generations separate complete rebuilds from live reads. Viewer scopes run
inside every candidate read and are checked again on live rows. Typed filter
grammar validates attributes and binds values before a driver sees them.
These guarantees also cover facets, federation, and recommendation reads.

## Next

- [Recommendations](recommendations.md) - record interactions and read scoped suggestions.
- [Vector](vector.md) - configure semantic storage for database search.
- [Queues](queues.md) - run indexing workers on your configured connection.
- [Live](live.md) - mount search-box and handle its selection event.
- [Environment Variables](env-vars.md) - all search configuration defaults.
