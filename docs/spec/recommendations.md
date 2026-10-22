# Recommendations

Status: Agreed 2026-10-17
Prefix: REC

Drafted 2026-10-14 from the developer's direction of 2026-10-09 19:02 ("I
think we can borrow from nation_x-com and add a recommendation engine as
well; in the next version") and his rulings of 2026-10-14: two engines,
borrowed from nation-x_com's search-discovery spec, "one is sql and the
other is knowledge graph based"; the starter recommender ships with search
and the graph recommender follows it ("I agree on the plan"); and "the
purpose of the small one is that there isn't sufficient data for the large
one... the idea is to build data". REC-001 to REC-004 and REC-008 are the
starter engine, the data it builds and its manual chapter, in the search
commitment; REC-005 to REC-007 are the graph engine, Draft until its own
commitment, which extends the chapter. Redrafted the same day after the
adversarial review by gpt-6.1-sol: the starter's inputs are named, events
are immutable, reads are authorized for a viewer, explanations are
projected, batches publish atomically and are bounded. On 2026-10-17 the
developer confirmed the graph engine's design: a graph store trait with
the application's database as the default and a Memgraph driver beside
it, revocation of facts whose source changed, and media embeddings
through the `fastembed` provider. The Observed section describes framework
`cf0d4a464` and the nation-x_com sources read on 2026-10-14, and the graph
store sources read on 2026-10-17.

## Observed at cf0d4a464

Status: Observed

- The framework has no recommendation surface: no event table, no
  co-occurrence query, no materialized recommendations (`framework/src/`,
  searched for `recommend` on 2026-10-14). The pieces it would build on
  are the search document and scope (SEARCH-001, SEARCH-010), the
  embedder (SEARCH-006), the queue, the scheduler, the `Lock` facade
  (`framework/src/lock.rs`) and the vector drivers.
- nation-x_com (read only, `/home/shawn/workspace2/nation-x_com`): the
  spec `docs/specs/domains/search-discovery-recommendations-trending.md`
  names two engines and forbids treating them as one system. The Starter
  Recommender (high confidence, "should be part of the first
  search/discovery implementation") is SQL: `track_cooccurrence` over
  likes, completed listens, skipped listens and playlist adds,
  `entity_cooccurrence` over accepted follows, similarity by genre tags,
  duration, play count and artist match, genre top tracks with recency
  decay, and a fallback chain collaborative, genre affinity, popularity,
  recent public content; deterministic, explainable, batch-generated,
  filtered for blocked, muted, suspended, private and non-discoverable
  targets at read time, with run metadata and coverage statistics. The
  Knowledge-Graph/RAG Recommender (medium confidence) combines typed graph
  facts, embeddings, graph traversal, retrieval over authorized documents
  and chunks, taste and social context, and explanations constrained to
  retrieved, permitted evidence; it never bypasses access or moderation,
  every fact carries provenance and confidence, and it degrades to the
  starter engine when retrieval confidence is low. Its ranking is a
  blended score: eligibility, graph affinity, semantic similarity, social
  proximity, freshness, explicit feedback, trust risk and diversity
  controls.
- Its code (`src/domain/search/recommendations.rs`, 1325 lines) records
  `RecommendationEventInput` rows with a weight in basis points, scores
  events by type (listen and view 1000, like 1400, favorite, save and
  playlist add 1600, comment 1200, follow 800, anything else 500, plus the
  weight divided by 20; positives at 1111-1124 are listen, view, follow,
  like, favorite, save, playlist add and comment; negatives at 1127-1135
  are skip, hide and report), reads each actor's complete positive and
  negative history (780-805), finds peers through the targets a user's
  positive events share and scores candidates from the peers' positives
  minus the user's negatives (886-927), restricts them to eligible
  documents under an authorization limit, materializes
  `starter_recommendation` rows keyed without a run id (212-214) with a
  `recommendation_explanation` each that stores peer user ids and seed
  track ids (216-219), and expires the previous rows by actor and
  generator (1052-1055). Its impressions endpoint records a
  100-basis-point event; `TrendingService` ranks per surface and window;
  `recommendations:generate` runs on the schedule. Knowledge graph nodes
  carry a type (profile, entity, group, release, track, post, article,
  event, listing, service, genre, hashtag, location, provider, topic) and
  edges carry source, target, confidence, provenance, visibility and
  timestamps, each with an access scope and a source version.
- Qdrant 1.19.2 answers `recommend` queries from positive and negative
  point ids or raw vectors with the `average_vector`, `best_score` and
  `sum_scores` strategies under a filter, `discover` queries from a target
  and context pairs, and a distance matrix; EmbeddingGemma 2 embeds text,
  images, audio and video into one space through separate encoder graphs
  with their own preprocessing (SEARCH, Observed).
- Read 2026-10-17: nation-x re-derives a document's graph on each index
  write (`materialize_knowledge_graph`, `src/domain/search/rag.rs:649`):
  it marks the document node's active edges deleted, then upserts edges
  whose keys carry the new source version; removing a document marks its
  nodes revoked and its edges deleted (rag.rs:231-270). The framework's
  `Embedder` input already carries typed media parts and each profile
  declares the input kinds it accepts, but the `fastembed` and `http`
  providers refuse any media part (`framework/src/search/embedder/
  fastembed.rs:295`, `http.rs:294`), and a timestamp-free model's derived
  document carries source version 0 (`framework/src/search/indexing.rs:158`).
  Memgraph 3.13.2 (2026-10-09) is licensed under the Business Source
  License 1.1 and speaks the Bolt protocol; the pure-Rust `neo4rs` 0.8.0
  speaks Bolt 4.0 to 4.3; Kuzu was archived on 2025-10-10.

## Requirements

[REC-001] The framework MUST keep an append-only recommendation event
stream: `Recommend::record(event)` appends a `RecommendationEvent` with
the event type (view, consume, complete, skip, like, favorite, share,
comment, follow, impression, click, purchase, or an application-named
type), a weight in basis points, the actor (the request's authenticated
user, or a hashed session for a guest, never a client-supplied identity),
the target as a search document key (index and model key, SEARCH-001), the
surface, the time, the target's access scope at record time, a source
version and a dedupe key unique per actor; a second event with the same
actor and dedupe key MUST be ignored and the first kept unchanged, so an
event id always names the same fact; search impressions and clicks
(SEARCH-012, validated there) MUST arrive as events when recommendations
are configured; and the stream MUST never be pruned or rewritten by the
engines, only deleted by the application's own retention, which the
framework exposes as `recommendations:prune --older-than`.
Falsifier: a duplicate actor and dedupe key stores two rows or changes the first; an event without a target, scope or source version is accepted; an actor is taken from the request body; a validated search click is missing from the stream; or an engine deletes or rewrites events.
Mechanism: `recommendations`.
Rationale: the developer's purpose for the starter engine is to build the data the graph engine learns from; an immutable stream keyed the way the graph engine reads it is that data.
Status: Agreed 2026-10-14

[REC-002] The `starter` engine, behind a `Recommender` trait, MUST compute
recommendations from the event stream and the search documents of the
targets' indexes (their facet values, timestamps and scope), plus the
indexes' vectors when an embedder is configured, and nothing else. The
default scoring MUST classify every shipped event type: positives view and
consume 1000, complete 1200, comment 1200, like 1400, share 1500, favorite
1600, purchase 2000, follow 800, click 300; impression 0 (seen only);
negatives skip, hide and report; an application-named type is positive at
500 unless the application declares its class and score; each adds its
weight divided by 20. For each actor the batch MUST read at most
`RECOMMEND_HISTORY` positive and negative events (default the newest 500),
find peers through shared positive targets sampled to `RECOMMEND_PEERS`
(default 200), score candidates from the peers' positives minus the
actor's negatives and seen targets, then fill from the fallbacks in order
when the tier above returns fewer than the limit: attribute affinity over
the facet values of the actor's positives, popularity in
`RECOMMEND_POPULARITY_WINDOW`, then recent documents; every candidate MUST
pass the target index's scope for the viewer (SEARCH-010) at read time;
`Recommend::for_user(user).surface(s).limit(n)` MUST read the latest
completed run's rows for the viewer the request names (or `for_viewer`),
and `Recommend::similar_to(target)` MUST refuse a seed the viewer's scope
does not cover and answer from co-occurrence with the index's semantic
neighbours folded in when vectors exist; `recommendations:generate` MUST
run the batch on the schedule and on demand, incrementally over events
since the last completed run, under the `Lock` facade so two runs never
overlap, and MUST complete within the committed budget on the gate's
fixture of one million events.
Falsifier: a viewer with positives A and B, no negative or seen C and C within scope and limit gets no C that sampled peers with A and B hold; a target or seed outside the viewer's scope is recommended or accepted; a fallback tier is skipped when the one above is short; an actor's batch reads more than `RECOMMEND_HISTORY` events; two generate runs execute concurrently; or the fixture run exceeds its budget.
Mechanism: `recommendations`.
Rationale: nation-x's starter engine, which its spec ranks high-confidence and launchable without a model, with the bounds its own full-history reads lacked.
Status: Agreed 2026-10-14

[REC-003] Every materialized recommendation MUST carry a run id and a
projected explanation, and every batch a run record. The explanation names
the reason as counts and categories only: the number of peers and shared
targets, the affinity attribute values, or the fallback tier, with titles
of shared targets included only when the viewer's scope covers them, and
never a peer's identity or interactions. A run writes its rows under its
own id and marks itself complete in one statement; readers MUST read only
the latest completed run, so no reader sees a mix of two runs; the
previous run's rows MUST be deleted only after the new run is complete; a
run that fails MUST leave the previous run current and its own rows
removed. The run record (`recommendation_runs`) holds the time, duration,
events read, users covered, items scored, coverage and the fallback share,
and `recommendations:status` MUST print the last runs.
Falsifier: a recommendation without an explanation or run id; an explanation naming a peer or a target the viewer cannot see; a read returning rows of two runs; a failed run leaving no current run or leaving its rows; or a run without a record.
Mechanism: `recommendations`.
Rationale: nation-x's spec requires explainable results and run metadata; its stored peer ids and seed ids are what the review found leaking, so the projection is the explanation's shape.
Status: Agreed 2026-10-14

[REC-004] The starter engine's tables MUST be the graph engine's input
under a versioned contract the framework's migrations own
(`recommendation_events`, `recommendation_explanations`,
`starter_recommendations`, `recommendation_runs`, with
`recommendation_schema_version` recorded): every event row holds its id,
actor, target document key, type, weight, surface, time, access scope and
source version; every explanation row holds its event ids and attribute
values; every recommendation row holds its run id, actor, target document
key, score, rank and explanation id; and a provenance reference to an
event MUST be the pair of event id and the event's content digest, so a
later derivation can tell an intact event from one removed by retention.
Falsifier: a shipped table lacks one of the named columns; the schema version is absent; or a provenance pair cannot detect a removed or altered event.
Mechanism: `recommendations`.
Rationale: "the idea is to build data": the handoff is the point of shipping the small engine first, and a contract the gate can check today is the only one worth promising.
Status: Agreed 2026-10-14

[REC-005] The `graph` engine MUST keep a typed knowledge graph: nodes
(application-declared types, a search document key where one exists, an
access scope, a source version, a provenance pair) and edges (source,
target, type, confidence in basis points, a provenance pair pointing at an
event, a model relation or an embedding similarity, an access scope, a
source version, timestamps), derived by queued jobs from the event stream
and from model relations the application declares, with user-derived facts
aggregated rather than stored per event where the application marks them
sensitive. A derived fact MUST follow its source: a node or edge derived
from a document MUST be replaced when the document's source version
advances and revoked when the document leaves its index, one derived from
an event MUST be revoked when retention removes the event (its provenance
pair no longer verifies, REC-004), and a revoked fact MUST NOT be read,
traversed or cited.
Falsifier: a node or edge without a scope, source version or provenance pair; a derived edge that cannot be traced to its source; a sensitive fact stored per event when marked for aggregation; or, after a document's source version advances, the document leaves its index or retention removes an event, a fact derived from the old version, the document or the event is still read, traversed or cited.
Mechanism: `recommendations`.
Rationale: nation-x's graph rules: raw facts need provenance and confidence, sensitive facts are minimized, and a fact whose source changed or left is revoked.
Status: Agreed 2026-10-17

[REC-006] The graph engine's vectors MUST live in the search collection of
the target's index (SEARCH-003, SEARCH-006), so one vector serves search
and recommendation; `similar_to`, `for_user` and the discover query MUST
take their positive and negative points from the viewer's own events over
targets the viewer's scope covers, use the engine's recommend and discover
operations under the scope filter where the search engine is Qdrant and
the framework's own nearest-neighbour fold where it is the database. Media
parts MUST reach the `Embedder` trait as a typed input (text with ordered
image, audio and video parts, each with its media type), preprocessed per
the model's processor configuration, through a provider whose profile
declares the input kinds it accepts: the `fastembed` provider MUST embed
them through a shipped EmbeddingGemma 2 media profile whose encoder graphs
take image, audio and video, and the `http` provider stays text only
(SEARCH-006); an index declaring media parts over a model or provider that
accepts text only MUST fail at sync naming both.
Falsifier: a second collection holds a copy of a search vector; a recommend uses a point outside the viewer's scope or ignores the viewer's negatives; an image or audio part's preprocessed input differs from the model processor's reference output for the same fixture beyond the tolerance the fixture records; a media part of a kind the profile accepts is dropped before the model; or an index declaring audio parts over a text-only profile syncs.
Mechanism: `recommendations`.
Rationale: one collection is cheaper and keeps search and recommendation consistent; the developer's nation-x use case ("sounds like this track") needs the audio leg, and the input contract is what makes it buildable.
Status: Agreed 2026-10-17

[REC-007] The graph engine MUST authorize before it scores: every
candidate node, every traversed edge and every evidence item MUST pass the
viewer's scope as a hard filter, and only then be ranked by a blended score
of graph affinity, semantic similarity, social proximity (paths through
the actors the viewer's own follow events name), freshness, explicit
feedback and trust risk (the target's hide and report events weighed
against its positive events), with weights per surface and diversity
controls that cap one node, attribute value or popularity band per result
list; when retrieval confidence falls under the surface's threshold the
engine MUST return the starter engine's rows for the same viewer instead
and say so in the explanation; and every explanation MUST cite stored
evidence the viewer may see (events of their own, edges, documents) and
never a reason without one.
Falsifier: a node or edge outside the viewer's scope influences a score; a target ranks above an otherwise equal target with fewer hide and report events; a path through an actor the viewer follows adds nothing to social proximity; a list collapses into one attribute value past its cap; a low-confidence surface answers from the graph instead of the starter rows; or an explanation names a reason with no permitted stored evidence.
Mechanism: `recommendations`.
Rationale: nation-x's blended score with eligibility moved from a weight to a gate, which its own spec requires, and the fallback the developer agreed to.
Status: Agreed 2026-10-17

[REC-008] The manual MUST ship `manual/recommendations.md`, linked from
`documentation.md`, with the starter engine in the search commitment (the
event stream and its types, scoring defaults, scopes, explanations, runs,
retention, the commands and environment variables, a `### Why Suprnova
diverges` callout noting Laravel has no recommender) and the graph
engine's sections added by its commitment; every environment variable MUST
have a row in `manual/env-vars.md`.
Falsifier: the chapter is missing or unlinked when the starter engine ships; a shipped command, event type default or variable is undocumented.
Mechanism: `recommendations`.
Rationale: every subsystem ships with its chapter, the repository's house rule, in the commitment that ships it.
Status: Agreed 2026-10-14

[REC-009] The graph engine MUST keep its graph behind a `GraphStore`
trait chosen by `RECOMMEND_GRAPH_STORE`, with two drivers: `database`, the
default, holding nodes and edges in the application's own database on
SQLite, Postgres and MySQL/MariaDB and walking them with depth-bounded
SQL; and `memgraph`, speaking the Bolt protocol to the Memgraph server
that configuration names. A store MUST only persist facts and answer
depth-bounded traversals under the viewer's scope (REC-007); the framework
MUST compute the blended score (REC-007) from what the store returns, so
both drivers return the same ranked list for the same graph and vectors.
The graph is derived data: `recommendations:graph-rebuild` MUST rebuild it from the event stream and the declared model relations
into a new generation and switch readers to it only when the rebuild is
complete, so a store that lost its data (Memgraph holds its graph in
memory) is restored from the facts the stream still holds.
Falsifier: the graph engine needs a service beyond the application's database while `RECOMMEND_GRAPH_STORE` is unset; the two drivers rank the gate's fixture differently; a traversal returns a node or edge outside the viewer's scope or past its depth bound; a rebuild of an emptied store, with no event or model written between, yields a different graph; or a reader sees a partial rebuild.
Mechanism: `recommendations`.
Rationale: the developer, 2026-10-17 09:48: "I like memgraph but am open to other recommendations", then confirmed a database default beside the Memgraph driver, because design principle 3 forbids locking a feature to one backend.
Status: Agreed 2026-10-17
