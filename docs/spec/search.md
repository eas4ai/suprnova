# Search

Status: Agreed 2026-10-14
Prefix: SEARCH

Drafted 2026-10-14 from the developer's direction of 2026-10-09 18:56 ("I
think I want to table scout and design our own search in the next
version"), his brief of 2026-10-14 ("a very robust search feature using
qdrant and/or MariaDB that can rival common search solutions", the ones
Laravel Scout offers: Algolia, Meilisearch, Typesense), and his rulings of
the same morning: engines run as Docker services in the gate like the
other databases; the embedder is fastembed or an OpenAI-compatible HTTP
provider, never provider-specific; the user chooses the model and the
framework ships one good default, EmbeddingGemma 2 at its native 768
dimensions ("I just want a good default"); search ships with the starter
recommender (`recommendations.md`), whose purpose is to build the data the
graph recommender needs. Redrafted the same day after the
adversarial review by gpt-6.1-sol (40 findings, `redraft`), and again on
the developer's ruling of 05:51 ("This isn't acceptable") against the
first redraft's answer to Sol: search runs on each engine's own full-text
index and relevance score, engines may order differently, and the
framework adds on top what they lack (typo tolerance and synonyms by query
expansion, facets, highlights, scopes, hybrid fusion, rerank, the
component and endpoint); generations and versions close the indexing
races, and the public endpoint, the embedders and the highlights carry
trust boundaries. The Observed section describes framework `cf0d4a464`, Laravel Scout 11
(docs:scout), Qdrant 1.19.2, fastembed 7.1.1, the EmbeddingGemma 2 model
card and OpenRouter's embeddings endpoint, all read on 2026-10-14.

## Observed at cf0d4a464

Status: Observed

- Full-text search today is a filter, not a ranking: `where_full_text`
  (`framework/src/eloquent/builder.rs:2449`, `where_full_text_with` at
  2478, `or_where_full_text` at 2504) and the DB facade's
  (`framework/src/database/db_facade.rs:823,834,851`) emit `MATCH (...)
  AGAINST (? IN NATURAL LANGUAGE MODE | IN BOOLEAN MODE)` on MySQL and
  MariaDB and `to_tsvector('english', c1) || to_tsvector('english', c2) @@
  plainto_tsquery | websearch_to_tsquery` on Postgres, with the GIN index
  built over the same expression (`framework/src/database/full_text.rs:6-34,
  144-190`). SQLite refuses with an error naming it and pointing at
  `where_like` or an FTS5 virtual table (`full_text.rs:12-13,189-200`). No
  call exposes a relevance score, typo tolerance, facets, highlighting or
  synonyms. The engines' own tokenizers disagree: InnoDB indexes no token
  shorter than `innodb_ft_min_token_size` (3) and drops its stopword list
  even in boolean mode, Postgres' `english` configuration stems and drops
  stop words while `simple` does neither, SQLite's `unicode61` tokenizer
  folds case and diacritics and keeps every token.
- Vectors are one dense embedding per item behind `VectorDriver`
  (`framework/src/vector/driver.rs:64-85`: `upsert`, `similar(store, query,
  k)`, `delete`, `count`; `VectorItem { id, embedding, metadata }`,
  `driver.rs:9-18`), registered by name (`Vector::register`,
  `Vector::store`, `framework/src/vector/mod.rs:68-80`). Drivers: memory,
  Qdrant over gRPC on port 6334 through `qdrant-client` 1.18 with cosine
  distance (`framework/src/vector/qdrant.rs:3,131,154`; `QDRANT_URL`,
  `QDRANT_API_KEY`, `manual/env-vars.md:424-425`), Pinecone over REST, and
  MariaDB. `similar` takes no filter, there are no sparse vectors, no
  payload indexes and no embedding provider: the application computes
  embeddings itself (`manual/vector.md`, Quickstart). Nothing under
  `framework/src/` embeds text.
- The hooks indexing needs exist: `Observer<M>` with `created`, `updated`,
  `saved`, `deleted` (`framework/src/eloquent/observers.rs:130,195-213`),
  collected through inventory; `Queue::push` (`framework/src/queue/mod.rs:411`),
  `Queue::push_after_commit` (431-437, which cannot see a transaction
  started by hand) and `push_after_commit_with_tx` (448-468, bound to an
  explicit handle), over the `Job` trait (`framework/src/queue/job.rs:59`),
  whose failed jobs are retried (66-67); `#[command]` handlers; the
  scheduler; the `Lock` facade (`framework/src/lock.rs`).
- The kits search notes with `LIKE`: the Inertia kits lower-case the term
  and build a contains pattern (`suprnova-cli/src/templates/files/backend/controllers/notes.rs.tpl:124-137`);
  the Live kit's controller forwards `GET /notes?search=` into the
  component's mount parameters
  (`suprnova-cli/src/templates/files/live/src/controllers/notes.rs.tpl:19-30`)
  and the component runs the same `LIKE` over title and body
  (`suprnova-cli/src/templates/files/live/src/live/note_list.rs.tpl:159-164`).
  A one-letter typo finds nothing.
- The Live library has `search-input` (FORM-003, a bounded `live:model`)
  and `combobox` (FORM-008, FORM-011, FORM-012) and no result list with
  highlights or facets (`crates/suprnova-live/components/`).
- The gate's standing services are Docker containers that
  `scripts/setup-gate-databases.sh` creates once and never pulls during a
  gate step: `suprnova-gate-mysql` (`mysql:8.4`, lines 25-35), beside the
  host's Postgres, MariaDB and Redis. The script's Mongo block
  (`suprnova-gate-mongo`, `mongo:8.2`, lines 75-100) sits after its
  `exit "$status"` at line 70 and does not run; the container on this
  machine predates that line. No Qdrant container exists;
  `qdrant/qdrant:v1.19.2` is the current image.
- Laravel Scout 11 (docs:scout) is the surface developers know: the
  `Searchable` trait with `toSearchableArray`, `shouldBeSearchable`,
  `searchableAs`, `makeSearchableUsing`; queued indexing through model
  observers with `scout.queue`, `scout.after_commit`, `scout.chunk`,
  `scout.soft_delete`; `Model::search('q')->where()->whereIn()->whereNotIn()
  ->orderBy()->take()->get() | paginate() | keys() | raw()` with a
  `query` callback for hydration; `makeAllSearchable`, `removeAllFromSearch`,
  `withoutSyncingToSearch`; the commands `scout:import`, `scout:flush`,
  `scout:index`, `scout:delete-index`, `scout:sync-index-settings`; the
  engines algolia, meilisearch, typesense, database (a `LIKE` filter),
  collection and null. Meilisearch's defaults are the ranking rules
  `words, typo, proximity, attribute, sort, exactness`, typo tolerance of
  one edit from five characters and two from nine, prefix search on the
  last word, phrases in double quotes and negative words with a leading
  minus, synonyms and stop words per index, `_formatted` highlights with
  `<em>` tags and cropping, facet distributions with counts and facet
  stats, hybrid search with a `semanticRatio` combining ranking scores,
  and a pagination cap of 1000 hits. Typesense adds `group_by`, geo sort
  with distance, and per-attribute weights; Algolia adds query
  suggestions from analytics.
- Qdrant 1.19.2: the Query API with `prefetch`, named dense and sparse
  vectors (`using`), `rrf` and `dbsf` fusion, rescoring stages and
  formulas; payload indexes of type keyword (with prefix), integer, float,
  bool, geo, datetime, uuid and text (tokenizer word, whitespace, prefix,
  multilingual; `stemmer`, `stopwords`, `ascii_folding`,
  `phrase_matching`); a text index answers the predicates `match.text`
  (all terms), `match.text_any`, `match.phrase` and `match.prefix`, not a
  lexical score; the facet endpoint (`key`, `limit`, `filter`, `exact`);
  the recommend (`positive`, `negative`, `strategy`, with a filter) and
  discover queries; groups; filters `match`, range, datetime range, geo
  radius, nested, `is_empty`.
- fastembed 7.1.1 (Apache-2.0, ONNX Runtime, sync API): dense text models
  from 384 to 1024 dimensions, among them `BAAI/bge-m3` (1024,
  multilingual, 8192 tokens), `intfloat/multilingual-e5-large` (1024),
  `BAAI/bge-large-en-v1.5` (1024), `mixedbread-ai/mxbai-embed-large-v1`
  (1024), `Alibaba-NLP/gte-large-en-v1.5` (1024), `nomic-embed-text-v1.5`
  (768), EmbeddingGemma 300M (768, fp32, int8 and q4) and
  `sentence-transformers/all-MiniLM-L6-v2` (384); sparse models
  `Qdrant/Splade_PP_en_v1`, bge-m3 sparse and OpenSearch neural sparse v3;
  rerankers `BAAI/bge-reranker-base`, `bge-reranker-v2-m3`,
  `jina-reranker-v1-turbo-en`, `jina-reranker-v2-base-multilingual`; and
  a user-defined text model from files through `try_new_from_user_defined`
  (`UserDefinedEmbeddingModel`: the ONNX graph bytes, the tokenizer files
  `tokenizer.json`, `config.json`, `special_tokens_map.json`,
  `tokenizer_config.json`, a pooling choice, a quantization mode, external
  initializer files; `src/text_embedding/init.rs:155-192`), fed the
  encoded `input_ids`, `attention_mask` and `token_type_ids` and read
  through the output key the model declares (`src/text_embedding/impl.rs`).
  EmbeddingGemma 2 is not in its named list.
- EmbeddingGemma 2 (`google/embeddinggemma-2`, Apache-2.0, published
  2026-10-06): 768 native dimensions with Matryoshka truncation to 512,
  256 and 128 after L2 re-normalization; one 8192-token context for text,
  code, images, video and audio in one vector space; a 270M-parameter
  text backbone with separately loadable vision (170M) and audio (300M)
  encoders; MTEB multilingual v2 61.4, English v2 68.5, code v1 78.7 at
  768; task prefixes `task: search result | query: {q}` for queries and
  `title: {title} | text: {body}` (`title: none` without one) for
  documents; float32 or bfloat16, float16 returns NaN silently. The ONNX
  export `onnx-community/embeddinggemma-2-ONNX` (2026-10-06) ships the text
  model in fp32, int8, q4, fp16 and q4f16 with mean pooling and
  normalization in its documented pipeline, and the vision and audio
  encoders as separate graphs with their own preprocessor configuration.
  No inference of that export has run in this repository yet.
- OpenRouter's `POST https://openrouter.ai/api/v1/embeddings` answered
  the OpenAI request shape on 2026-10-14 for the free
  `nvidia/llama-nemotron-embed-vl-1b-v2:free` (text and image input, 2048
  native dimensions, NVIDIA Open Model License over the Llama 3.2
  license): `model` and `input` (a string or an array) in, `data[]` with
  `embedding` and `index`, `usage`, `model`, `provider` and `id` out, in
  about a second; a `dimensions` of 768 was honored. The developer named it
  as the test target for the `http` embedder.
- The developer's nation-x_com application (read only,
  `/home/shawn/workspace2/nation-x_com/src/domain/search/`) has the shape
  the recommenders borrow: `search_documents` with a generated Postgres
  `tsvector` and GIN index, `search_index_jobs` enqueued in the writer's
  transaction and claimed by a leased worker, projections per target type,
  content-hash skips, an OpenAI-style HTTP embedding provider with
  generations behind deployment-owned endpoints, a viewer-authorization
  predicate joined into every query, revocation before stale artifacts
  are used, keyset cursors bound to the query and viewer, and no text
  relevance, typo tolerance, facets or highlighting.

## Requirements

[SEARCH-001] A model MUST become searchable from its declaration:
`#[derive(Searchable)]` with attributes for the index name (default the
table name), the searchable attributes in ranking order with an optional
weight each, the filterable, sortable and facet attributes with a declared
type each (string, integer, float, bool, datetime, geo point; facet implies
filterable), an embed source (the attributes whose text is embedded, with
the title attribute named), a `should_be_searchable` predicate, and
`search_touches`, the relations whose change re-indexes this model; a
derived `to_search_document() -> Result<SearchDocument>` MUST return the
model's key, the index, a source version (the model's `updated_at` when it
has one, else a monotonic version the framework keeps), each declared
attribute's value typed as declared, the embed text, and nothing
undeclared; `make_searchable_using` MUST let the model load relations
before a chunk is documented.
Falsifier: a derived document omits a declared attribute, carries an undeclared one, lacks a source version, or stores a value against its declared type; a model whose predicate is false reaches an index; a related model's save does not re-index a parent that names its relation in `search_touches`; or documents of a chunk are built one model at a time.
Mechanism: `search`.
Rationale: Scout's `Searchable` surface is what developers know, and the document is the one shape every engine, the embedder and the recommenders read; the typed attributes are what filters, facets and sorts validate against.
Status: Agreed 2026-10-14

[SEARCH-002] An index MUST carry settings declared in code and applied by
`search:sync-index-settings`, in two classes. Query-time settings take
effect on the next query: typo tolerance (on by default; one edit from five
characters and two from nine, counted in Unicode scalar values with
Damerau-Levenshtein distance, a transposition being one edit; off on named
words, named attributes, and every token that is a number), synonyms
(two-way by default, one-way when declared, a multi-word synonym matching
as a phrase), the stop words removed from a query before it reaches the
engine, the distinct attribute, the hits cap (default 1000), the fusion
depth and hybrid default ratio, the highlight tags and crop marker, and
the facet value cap and order. Index-time settings start a new index
generation (SEARCH-005): the engine's language configuration (the Postgres
text search configuration, the FTS5 tokenizer, Qdrant's text index
tokenizer, stemmer, stop words and folding; MariaDB and MySQL take the
server's; by default Postgres pins a copy of the server's
`default_text_search_config` to the generation, SQLite uses the `unicode61`
tokenizer, and Qdrant the `word` tokenizer with folding and no stemmer or
stop words), the searchable attributes with a weight each, applied as the
engine's own column weights, the embed source and template, the
filterable, sortable and facet attribute set with types, the embedder,
model and dimensions. The command MUST report which class each change
fell in and whether a generation build started, and the settings MUST be
readable back from the engine as the framework wrote them.
Falsifier: a query-time setting changed and synced leaves the next query answered by the old setting on any engine; an index-time setting changed and synced is applied to the live generation in place; a weight declared on an attribute does not reach the engine's weighting; or the settings read back differ from what was written.
Mechanism: `search`.
Rationale: Meilisearch's settings are the yardstick the developer named; the two classes are what makes "takes effect at once" and "never a partial index" both true, and the weights ride each engine's own mechanism rather than a framework scorer (the developer's ruling of 2026-10-14 05:51).
Status: Agreed 2026-10-14

[SEARCH-003] Engines MUST sit behind a `SearchEngine` trait, selected by
`SEARCH_DRIVER` and overridable per index, and each MUST index and rank
with its own full-text machinery: `database` on the application's own
database, MariaDB and MySQL through a FULLTEXT index queried in boolean
mode with the `MATCH ... AGAINST` relevance summed over the weighted
searchable columns, Postgres through a stored `tsvector` built with
`setweight` per attribute under the index's text search configuration, a
GIN index and `ts_rank` normalized by document length, SQLite through an FTS5 external-content table
with `bm25()` column weights, and the semantic leg in the configured
`VectorDriver` store (any of its drivers); `qdrant` on a collection per
index generation (Qdrant 1.19 or later) with a named dense vector; as
the lexical leg, one named sparse vector per weighted searchable attribute,
which Qdrant encodes from the attribute's text with its own BM25 model and
ranks with its IDF modifier, the attribute scores summed by their weights in
a Qdrant formula query, or one named sparse vector from the configured
sparse model when one is set; the text payload index filtering words,
phrases and exclusions; payload indexes for every filterable, sortable and facet attribute, and Qdrant's
own fusion; `collection` from models in memory with a BM25 scorer for
tests; `null` accepting writes and answering empty. The builder semantics
(filters, facets, sort, scope, pagination, highlights, hybrid, grouping)
MUST hold on every engine; the order of hits is the engine's, and the
manual MUST document each engine's differences (tokenization, stemming,
stop words, minimum token length, prefix and phrase support). The
framework MUST NOT re-tokenize documents for the engine's ranking or
disguise tokens to defeat an engine's limits; on Qdrant, whose prefix match
reads only keyword values and which reports no vocabulary, the framework MAY
store each document's normalized words as a keyword payload for prefix
filters and the typo vocabulary, which never ranks a hit; `search:status` MUST report MariaDB's and
MySQL's minimum token length and stopword state so the operator can set
them. An application MUST move an index between engines by configuration
alone, with a reindex.
Falsifier: a filter, facet, sort, scope, page, highlight, hybrid ratio or group behaves differently on two engines for the same documents and query; a document's searchable text reaches an engine's ranking other than as the attribute text the engine tokenizes itself; a Qdrant hit's lexical score ignores an attribute's weight; an engine cannot be chosen per index; `search:status` on MariaDB omits the server's minimum token length; or a search call site changes when the engine does.
Mechanism: `search`.
Rationale: the developer's brief names Qdrant and MariaDB, design principle 3 forbids one backend, and his ruling of 2026-10-14 05:51 keeps each engine's tuned ranking, speed and stemming in charge; what is shared is the API, not the order.
Status: Agreed 2026-10-14

[SEARCH-004] `Model::search(query)` MUST return a builder whose terminal
methods return `Result`: `filter` (a typed expression over declared
attributes: equality, `in`, `not in`, ranges, null checks, boolean
combinations, a geo radius; built through methods or parsed from the
documented filter grammar, always bound, never interpolated), `facets`,
`facet_stats` (min and max of a numeric or datetime facet), `sort_by`
(attribute or distance from a point, missing values last), `group_by(attr,
per_group)`, `highlight`, `crop`, `hybrid(ratio)` from 0 (lexical) to 1
(semantic), `rerank(model, depth)`, `limit`, `offset`, `paginate`,
`simple_paginate`, `keys`, `raw`, a `query` callback for hydration, and
`get`; the query string MUST accept double-quoted phrases and
minus-prefixed negative words; hits MUST be models hydrated through
Eloquent with each hit's engine relevance score, its normalization to [0,
1] within the result, the engine that produced it, formatted fields, geo distance when sorted by distance, and group members,
with the total (exact on every engine for lexical queries, estimated and
flagged as such when a semantic leg's top-k bounds it), the facet
distribution and the processing time; pages MUST be slices of one ranked
list with ties broken by document key; a hit whose row is gone MUST be
dropped and its removal queued; and `Search::federated([...])` MUST search
several indexes in one call, merging by reciprocal rank fusion over the per-index
results with the index order as the tie-break.
Falsifier: a filter, facet, facet stat, sort, group, highlight, phrase, negative word or hybrid ratio is ignored by an engine; a filter value reaches an engine unbound; a hit is returned for a deleted row; two consecutive pages overlap or skip a hit on an unchanged index and engine; or two indexes cannot be searched in one call.
Mechanism: `search`.
Rationale: Scout's builder plus what Meilisearch and Typesense add (facets with stats, geo sort, grouping, phrases and negatives, hybrid, federated search) is the bar the developer set; bound filters are the injection boundary.
Status: Agreed 2026-10-14

[SEARCH-005] Indexing MUST follow the model and never lose a write.
`created`, `updated`, `restored`, `deleted` and force-deleted events
index, re-index or remove the document carrying its source version, after
commit when the write runs in a transaction (the ambient `DB::transaction`,
an explicit handle, and a savepoint whose outermost transaction commits),
through the queue by default (`SEARCH_QUEUE`, with connection and queue
configurable) and as deferred inline work after commit when the queue is
off; an engine MUST ignore a write whose source version is older than the
stored one and MUST keep a removal tombstone for `SEARCH_TOMBSTONE_HOURS`
(default 24) so a retried or reordered job cannot resurrect a removed
document; a write whose content hash and generation settings digest match
the stored ones MUST skip the engine; soft-deleted models MUST leave the
index unless the index opts into `soft_delete`, which stores a filterable
`__soft_deleted` flag; `Model::without_syncing_to_search`,
`make_all_searchable`, `remove_all_from_search`, `searchable()` and
`unsearchable()` on a query or collection MUST work in chunks
(`SEARCH_CHUNK`); and the commands `search:import`, `search:flush`,
`search:index`, `search:delete-index`, `search:sync-index-settings`,
`search:reindex` and `search:status` MUST exist. A generation build
(`search:reindex`, or an index-time settings change) MUST write every
document changed during the build to both the old and the new generation,
switch readers to the new one atomically only after the bulk import and
every write queued before the switch have landed, keep the old generation
until no queued job names it, then drop it.
Falsifier: a save inside a rolled-back transaction (ambient, explicit handle or savepoint) indexes its document, or a committed save with the queue off reaches the engine before commit; an older update applied after a newer one wins; a retried update recreates a document removed inside the tombstone window; an unchanged save writes the engine, or a settings change is skipped as unchanged; a document saved or deleted during a reindex is missing or stale in the new generation after the switch; or a query sees a partial generation.
Mechanism: `search`.
Rationale: Scout's pipeline semantics with nation-x's hash skip and generations, closed against the reorder, retry and reindex races the review found.
Status: Agreed 2026-10-14

[SEARCH-006] Embeddings MUST come from an `Embedder` trait with
`embed_documents`, `embed_query` and a `ModelProfile` (model id and
revision, dimensions, context length in tokens, query prefix, document
template, pooling, normalization, truncation dimensions, the input kinds
it accepts), chosen by `SEARCH_EMBEDDER` and overridable per index. The
`fastembed` provider (feature `search-fastembed`) MUST load any of
fastembed's named models or a user-defined ONNX text model by Hugging Face
repository id or local path, taking the graph file (and its external data
file), `tokenizer.json`, `config.json`, `tokenizer_config.json` and
`special_tokens_map.json`, a pooling choice, the precision variant and the
output key from the profile, resolving a repository id to a revision on
first load and recording the revision and the sha256 digest of each file in the
cache (`SEARCH_MODEL_CACHE`) and on the index generation, refusing a
cached file whose hash changed, and MUST run inference on a blocking
thread. The `http` provider MUST speak the OpenAI-compatible embeddings
request (`POST {base}/embeddings` with `input`, `model` and optional
`dimensions`) against a base URL set in configuration, never by a request
or a tenant, over HTTPS unless the host is loopback, following no redirect,
sending the bearer key from `SEARCH_EMBED_API_KEY` only to that host,
batched, with a timeout, a bounded response size and a bounded retry on
429 and 5xx, and MUST carry no provider-specific field. Every provider MUST
validate each vector (the profile's dimensions after any truncation, finite
values, L2-normalized when the profile says so) and MUST truncate input to
the profile's context length in tokens; truncation to a Matryoshka
dimension re-normalizes. The default model MUST be EmbeddingGemma 2
(`onnx-community/embeddinggemma-2-ONNX`, fp32, 768 dimensions, mean
pooling, normalized, its task prefixes) with its profile shipped and
qualified by a real-inference test the gate leaves ignored; profiles MUST
ship for the named fastembed models (bge, e5, nomic, gte, mxbai, arctic,
MiniLM, EmbeddingGemma 300M) and be declarable for any other model. The
framework MUST apply the profile's query prefix and document template so
application code never writes them. An index with no embedder configured
is lexical, and a semantic or hybrid query on it MUST fail with an error
naming `SEARCH_EMBEDDER`.
Falsifier: a query is embedded without its model's query prefix while documents carry their template; a model outside fastembed's named list cannot be configured with the files above; a cached model file with a changed hash is loaded; the http provider follows a redirect, sends the key to another host, accepts an unbounded body, or requires a field a generic OpenAI-compatible server lacks; a vector of the wrong dimension or with a non-finite value is stored; inference runs on a Tokio worker; or the default profile is not EmbeddingGemma 2 at 768.
Mechanism: `search`.
Rationale: the developer's rulings of 2026-10-14: fastembed or an OpenAI-compatible HTTP provider, never provider-specific; the user picks the model and the framework ships one good default; the provenance, destination and vector checks are the boundaries the review found open.
Status: Agreed 2026-10-14

[SEARCH-007] The engine's relevance is the ranking; the framework works on
the query before it and on the result after it. Before: the query is
expanded to the engine's own query syntax, each word to itself, its
synonyms and the vocabulary words within its typo budget joined by the
engine's OR, the last word also as the engine's prefix match, quoted
phrases as the engine's phrase match, minus-prefixed words as the engine's
exclusion, the index's stop words removed; the expansion MUST be bounded by
`SEARCH_EXPANSION_TERMS` (default 64 per word, highest document frequency
first). The vocabulary of an index generation MUST come from the engine's
own index where it exposes one (FTS5's `fts5vocab`, Postgres' `ts_stat`
over the stored `tsvector`) and from a framework word list maintained on
every index and remove where it does not (MariaDB, MySQL, Qdrant), with
zero-frequency words dropped. After: with a hybrid ratio r the lexical
result (the engine's top `fusion depth`) and the semantic result (the top
`fusion depth` by cosine from the index's vectors under the same filters)
are fused by weighted reciprocal rank fusion with weights 1 - r and r (in
Qdrant, its own fusion); r = 0 is the engine's lexical order and r = 1 the
cosine order; `sort_by` orders first in every mode with relevance as the
tie-break; `distinct` keeps the first hit per value; `rerank(model, depth)`
re-scores the top `depth` hits with the named fastembed cross-encoder last
and replaces their order.
Falsifier: with r = 0 a one-edit match on a five-letter word is missed, a prefix of the last word does not match, a quoted phrase matches out of order, a negative word's document is returned, or the order differs from the engine's own order for the expanded query; with r = 1 the order is not the cosine order; a `sort_by` query is ordered by relevance; an expansion exceeds the bound; or a removed document's words stay in a framework-kept vocabulary.
Mechanism: `search`.
Rationale: the developer's ruling of 2026-10-14 05:51: the databases' own ranking does the ranking; expansion and fusion are what Meilisearch and Typesense add that the databases lack, and both compose with native scoring.
Status: Agreed 2026-10-14

[SEARCH-008] Every hit MUST offer formatted fields on request: each
matched word, corrected, prefixed, synonym and phrase matches included,
wrapped in the index's highlight tags, cropped to the query's length
around the first match with the index's crop marker, produced by the
engine where it has a highlighter (Postgres `ts_headline`, FTS5
`highlight()` and `snippet()`) and by the framework over the expanded
terms where it has none (MariaDB, MySQL, Qdrant), the document text
HTML-escaped before the tags are added in both paths. The tags and marker
MUST come from the index settings in code, never from a request, and MUST
be validated at sync to an element name from the framework's allowlist
(`mark`, `em`, `strong`, `b`, `span`) with at most a `class` attribute.
Falsifier: a hit found through a typo whose highlight leaves the matched word unmarked on any engine; a document containing `<script>` rendered unescaped in a formatted field; a crop that cuts inside a tag; a tag with an event handler accepted at sync; or a request parameter that changes the tags.
Mechanism: `search`.
Rationale: highlights are how a result list shows why it matched; escaping the text and validating the markup are both needed, since either alone leaves an executable path.
Status: Agreed 2026-10-14

[SEARCH-009] A query that asks for facets MUST count, for each facet
attribute, the values over the complete filtered candidate set of the
query, before distinct, the hits cap and any semantic top-k, exactly on
every engine (Qdrant's facet endpoint with `exact`); the values returned
MUST be the most frequent up to the facet value cap, then presented in
the index's order (by count or alphabetically); `facet_stats` MUST return
the min and max of a numeric or datetime facet over the same set;
`facet_search(attr, prefix)` MUST return matching facet values with counts
for a type-ahead; and a facet count MUST equal the exact total of the same
query filtered by that value on the same generation.
Falsifier: a facet count disagrees with the exact total of the same lexical query filtered to that value on any engine; a less frequent value is returned past the cap in place of a more frequent one; or a facet stat lies outside the matching documents' values.
Mechanism: `search`.
Rationale: facets are the Algolia, Meilisearch and Typesense feature a catalogue or directory cannot do without, and one counting universe is what makes their numbers trustworthy.
Status: Agreed 2026-10-14

[SEARCH-010] An index MAY declare a scope: a function from the viewer
(the request's authenticated user, or a guest) to a filter over declared
filterable attributes. When an index declares a scope, every read path
(`get`, `keys`, `raw`, facets, facet stats, totals, groups, federated
search, the JSON endpoint and the recommenders' reads) MUST apply it
inside the engine's candidate filter, taking the viewer from the request
state or from `for_viewer`, and a read that cannot name a viewer MUST
apply the guest scope. A write that changes a scope attribute MUST reach
the index after commit before the request completes, whatever the queue
setting, and every hydrated hit MUST be re-checked against the scope on
the live row so a stale document is dropped.
Falsifier: a document outside the viewer's scope appears in hits, keys, raw results, a facet count, a total, a group or a federated merge; a scoped index answers an unscoped read; a model made private is still returned to a viewer who could see it before the request that changed it completed; or a hit whose live row fails the scope is returned.
Mechanism: `search`.
Rationale: nation-x joins a viewer predicate into every search and revokes before stale artifacts are used; the graph recommender's rule that retrieval never bypasses access starts here.
Status: Agreed 2026-10-14

[SEARCH-011] The framework MUST ship a Live `search-box` component
(`search-input` with a results listbox, highlights, optional facets, the
combobox keyboard pattern of FORM-008, stale results discarded by request
order) bound to a searchable model through a bounded `live:model`, and a
JSON endpoint registered by `Search::routes()` for the indexes the
application names there, answering `GET /search?q&index&filter&facets&sort
&page` with the filter parsed by the grammar of SEARCH-004 over declared
attributes only, the viewer's scope applied, a `422` with the failing
parameter on invalid input, and a response holding only the document's
declared attributes, formatted fields, facets and pagination, never a
hydrated model's other fields. The Inertia and Live kits MUST search notes
through `Note::search` with highlights, replacing the `LIKE` filter, so a
fresh scaffold on SQLite finds a seeded note by a one-letter typo in a
word of at least five letters.
Falsifier: a fresh kit's notes search misses a seeded note by a one-letter typo in a five-letter word; the endpoint returns a document outside the viewer's scope, an index not named in `Search::routes()`, an undeclared field, or accepts an undeclared filter attribute without a `422`; the component returns a stale result list after a newer query answered, or its listbox fails the FORM-008 keyboard checks.
Mechanism: `search`.
Rationale: the kits are where a developer meets the feature; the endpoint's grammar, allowlist and projection are the trust boundary a public search surface needs.
Status: Agreed 2026-10-14

[SEARCH-012] Analytics MUST be opt-in (`SEARCH_ANALYTICS`): when on, the
framework records each search (index, query, filters, hit count,
processing time, the viewer's id only when `SEARCH_ANALYTICS_VIEWER` is
set, never a guest identity), the impressions a result list shows (an
impression id, document keys with positions) and the clicks reported
through `Search::click(impression, document, position)`, accepted only
when the impression belongs to the reporting viewer's session and shows
that document at that position, in `search_queries`, `search_impressions`
and `search_clicks`; `search:analytics-prune` MUST run on the schedule and
delete rows older than `SEARCH_ANALYTICS_RETENTION_DAYS` (default 90);
`search:analytics` MUST summarize top queries, zero-result queries and
click positions; `Search::suggest(index, prefix)` MUST answer query
suggestions from the retained queries of that index; and impressions and
clicks MUST flow into the recommendation event stream (REC-001) when
recommendations are configured.
Falsifier: analytics off writes a row; analytics on misses a click or records a guest identity; a click for a document the impression did not show is accepted; a row older than the retention survives the prune; or a click is not visible to the recommenders as an event.
Mechanism: `search`.
Rationale: zero-result queries and suggestions are how an index gets better; validated clicks are the data the developer wants the starter recommender to build, without a client being able to forge them.
Status: Agreed 2026-10-14

[SEARCH-013] The framework MUST ship a relevance and latency baseline
measured against the rivals the developer named: a public judged
collection as the fixture corpus and query set under
`framework/tests/search/fixtures/` (a BEIR set with its relevance
judgments and license recorded), and a calibration command that indexes
the same corpus into Meilisearch and Typesense running on the developer's
dedicated server (addresses and keys from `SEARCH_BASELINE_MEILISEARCH_URL`,
`SEARCH_BASELINE_TYPESENSE_URL` and their key variables), runs the same
queries, and commits their nDCG@10 with their versions into the baseline
file. The gate MUST then require, per engine, that lexical mode reaches
the rivals' lexical nDCG@10 within the committed tolerance, that hybrid
mode with the default model exceeds it, that a fixture query answers
under the committed p95 budget on the gate's fixture size, and that no
run falls under the committed numbers; every result MUST be recorded with
the engine, the model profile, the rivals' versions and the tree hash.
Falsifier: an engine's lexical nDCG@10 falls under the rivals' score minus the tolerance, or hybrid does not exceed it; a p95 exceeds its budget; the baseline file lacks the rivals' versions or the fixture's license; or the record omits the engine, profile or tree.
Mechanism: `search`.
Rationale: "rival common search solutions" needs a measurement against those solutions on the same judged data, not a bar set by our own first run (the side note of 2026-10-14 06:03, accepted by the developer); the rivals run on his dedicated server ("I want to put them on the server", 06:13) so the gate here pulls nothing.
Status: Agreed 2026-10-14

[SEARCH-014] The manual MUST ship `manual/search.md`, linked from
`documentation.md`, covering the derive and the two settings classes,
each engine's language configuration and documented differences, engines
and how to move between them, the
embedders and the model choice with the default and its provenance
record, the query builder with the filter grammar, facets, highlights,
hybrid, grouping, geo and federated search, indexing, versions and
generations, scopes, the component and endpoint, analytics and
suggestions, the baseline, the commands, and a `### Why Suprnova
diverges` callout against Scout; every environment variable it introduces
MUST have a row in `manual/env-vars.md`.
Falsifier: the chapter is missing or unlinked; a shipped command, builder method, setting or environment variable is undocumented.
Mechanism: `search`.
Rationale: every subsystem ships with its chapter, the repository's house rule.
Status: Agreed 2026-10-14
