# Recommendations

You record interactions with searchable documents and read recommendations
through `Recommend`. The starter engine uses SQL events, search facets,
timestamps, scopes, and optional search vectors. You need no embedding model
to generate collaborative, attribute-affinity, popularity, and recent results.
The `Recommender` trait lets you replace the engine without changing callers.

The purpose of the starter engine is to build the data the graph recommender
needs: immutable events, materialized recommendations, explanations, and run
records under a versioned storage contract. Two engines ship: `starter`, the
default, and `graph`, which you select with `RECOMMEND_ENGINE=graph` once your
events carry enough shared engagement (see
[Rank with the graph engine](#rank-with-the-graph-engine)).

## Record events

Install the migrations and provision the target's [search index](search.md)
first. In an authenticated request, this function records a view of an existing
searchable note and returns its provenance:

```rust
use suprnova::{
    EventProvenance, EventType, FrameworkError, Recommend,
    RecommendationEvent, RecommendationTarget,
};

async fn record_note_view(key: &str, dedupe: &str) -> Result<EventProvenance, FrameworkError> {
    let target = RecommendationTarget { index: "notes".into(), key: key.into() };
    let event = RecommendationEvent::new(EventType::View, target, None, "notes", dedupe).await?;
    Recommend::record(event).await
}
```

`RecommendationEvent::new(event_type, target, actor, surface, dedupe_key)` loads
the registered target's document. It captures its source version, the index's
guest access filter as canonical JSON, and the current UTC time. A missing or
ineligible target is an error. The access-scope snapshot describes the fact's
record-time policy; it does not replace the current viewer's scope on reads.
Recording requires a target, a non-null scope equal to the descriptor's guest
filter, and a source version. The event also stores its type, weight, actor,
surface, dedupe key, and scoring snapshot.

With `actor=None`, Auth supplies the authenticated user. For guests, resolve
`Actor::current(Some(server_session_id)).await?` or
`Actor::guest(server_session_id)?` and pass that actor to the constructor.
Guest actors store a lowercase SHA256 session digest, never the raw session.
Trusted background code can use `Actor::User(user_id)`. Never take an actor
from a request body: `Actor` and `RecommendationEvent` have no `Deserialize`
implementation. Resolve identities on the server.

A dedupe key is unique per actor. Repeating an actor/key pair returns the
original event id and digest and leaves that first event unchanged, even if
the retry changes its type or weight. Separate actors can use the same key.
Use a stable interaction identity for retries. Surface and dedupe key allow
1-128 bytes without control characters; target index allows 128 bytes and
target key 255.

### Event types and scoring

Every event's score is its base plus signed integer `weight / 20`. Weight is
in basis points from -10000 to 10000, defaults to zero, and division truncates
toward zero. Class determines how the event affects recommendations; changing
weight does not turn an impression into positive affinity.

| Event type | Rust variant | Class | Base score |
|---|---|---|---:|
| `view` | `EventType::View` | Positive | 1000 |
| `consume` | `EventType::Consume` | Positive | 1000 |
| `complete` | `EventType::Complete` | Positive | 1200 |
| `skip` | `EventType::Skip` | Negative | -1000 |
| `hide` | `EventType::Hide` | Negative | -1000 |
| `report` | `EventType::Report` | Negative | -1000 |
| `like` | `EventType::Like` | Positive | 1400 |
| `favorite` | `EventType::Favorite` | Positive | 1600 |
| `share` | `EventType::Share` | Positive | 1500 |
| `comment` | `EventType::Comment` | Positive | 1200 |
| `follow` | `EventType::Follow` | Positive | 800 |
| `impression` | `EventType::Impression` | Seen | 0 |
| `click` | `EventType::Click` | Positive | 300 |
| `purchase` | `EventType::Purchase` | Positive | 2000 |
| Application name | `EventType::Custom(String)` | Positive | 500 |

`EventType::from_name` recognizes the shipped names even when supplied through
`Custom`. Declare application classes and base scores through
`RecommendConfig.event_types` or `RECOMMEND_EVENT_TYPES`:

```json
{"playlist_add":{"class":"positive","score":1600},"dismiss":{"class":"negative","score":-1200}}
```

Classes are `positive`, `negative`, and `seen`. Override scores must be between
-1000000 and 1000000. Overrides also accept shipped event names. Existing
events keep their stored class and score when configuration changes.

## Generate with the starter engine

The batch reads at most `RECOMMEND_HISTORY` newest positive and negative
events per actor combined, ordered by time and id; the default is 500. Seen
impressions are aggregated separately in SQL and do not consume those slots.
Shared positive targets identify peers, sampled to `RECOMMEND_PEERS` (200)
by their most recent shared activity. Candidate scores sum sampled peers'
positive scores. A collaborative candidate needs at least `RECOMMEND_MIN_PEERS`
(3) distinct peers: with fewer, it can still arrive through a later tier, which
names no peers. Otherwise a suggestion whose peer count is 1 would point at that
one person, and the suggestion would be one of their own views or purchases.
`Recommend::similar_to` counts co-occurrence under the same rule. Negative and
previously seen/interacted targets are excluded.
Scores sort descending with `(index, key)` as a deterministic tie-break.

When the higher tier is short, results fill in this order:

1. Collaborative candidates from shared positive targets, with search semantic
   neighbours folded in when vectors exist.
2. Attribute affinity over declared facet values of authorized positive seeds.
3. Popularity from positive activity in `RECOMMEND_POPULARITY_WINDOW`, default
   30 UTC days.
4. Recent eligible documents, using `published_at`, `created_at`, or
   `updated_at` when declared, otherwise the search source version.

Declare facet and timestamp attributes on your searchable models to supply
those signals. Semantic retrieval reuses the target index's provider, document
profile, generation, and vector store. It creates no second vector store.
Every target passes the viewer's search scope inside candidate retrieval and
again against its live document before display. Removed or inaccessible
documents cannot supply displayed evidence.

Run `cargo run --bin console -- recommendations:generate` on demand. Add this task to your
application's schedule:

```rust
schedule.add(
    schedule.command("recommendations:generate").hourly().without_overlapping()
);
```

`schedule.command("recommendations:generate").hourly().without_overlapping()`
uses the same generation path as an on-demand command. The starter also holds
a renewable cache lock named `recommendations:generate`, with
`RECOMMEND_LOCK_TTL_SECS` defaulting to 3600 seconds. Configure a shared lock
backend when several processes generate recommendations. Contention returns
`GenerationOutcome::Locked` immediately; a lost lease aborts the run.

### Runs and explanations

Runs have `running`, `complete`, or `failed` states. A run stages rows under
its own id and publishes by marking itself complete in one statement. Reads
select only the latest completed run and retrieve rows and evidence in one
database statement. Previous rows are removed only after publication. A failed
run removes its staged rows and explanations, keeps an error record, and leaves
the previous completed run current.

Generation tracks a completed high-water mark, new events, capped history
snapshots, and a search catalog digest. It regenerates affected actor/surface
pairs and copies unaffected rows into the next snapshot. History and catalog
checks catch late commits and changed content. A filtered `--user` or
`--surface` run preserves other rows and does not consume their pending global
delta.

`recommendation_runs` records start time, duration in milliseconds, events
read, new events, users covered, items scored, coverage, fallback share,
high-water mark, catalog digest, history snapshots, state, and error.
`recommendations:status` prints the ten latest attempts. The committed
million-event fixture has a 300-second generation budget; run its ignored
budget test separately from ordinary application tests.

Every materialized row carries its run id and explanation id. The public
explanation contains peer and shared-target counts, authorized affinity
attribute values, a tier (`collaborative`, `semantic`, `affinity`, `popularity`,
or `recent`), and shared-target titles only where the viewer can see them.
It exposes no peer identity, peer interaction, or private evidence id. Stored
evidence retains event provenance for derivation without exposing it in the
display projection.

## Read recommendations

Select the subject and surface, and let the request resolve the viewer:

```rust
use suprnova::{FrameworkError, Recommend, Recommendation, Viewer};

async fn home_recommendations(user_id: &str) -> Result<Vec<Recommendation>, FrameworkError> {
    let viewer = Viewer::current().await?;
    Recommend::for_user(user_id)
        .surface("home")
        .limit(10)
        .for_viewer(viewer)
        .get()
        .await
}
```

`for_user` takes a string-like user id, not an authorization decision. Your
application authorizes which subject a caller may request. The viewer comes
from Auth when `get()` executes or from explicit `for_viewer`. Without an
identity, the guest scope applies. `Recommend::for_actor(actor)` supports a
trusted guest actor too. The default surface is `home`, the limit is 10,
zero returns no rows, and limits above 10000 fail. Before a completed run
exists, materialized reads return no recommendations.

`Recommend::similar_to(RecommendationTarget { index: "notes".into(),
key: "1".into() }).for_viewer(viewer).limit(10).get().await?` computes
co-occurrence with optional semantic neighbours on demand. It refuses a
missing or inaccessible seed before scoring. The viewer's negatives and seen
targets are excluded. Its results have `run_id=None`; materialized results
name the completed run. Both return a target, declared `SearchDocument`,
score, one-based returned rank, and projected explanation.

## Rank with the graph engine

The graph engine keeps a knowledge graph of your search documents and ranks
from it live, at read time. Every node and edge it holds names its source, and
every hop it takes stays inside the viewer's scope. When the graph supports a
list too thinly, it answers with the starter engine's rows for the same viewer
and says so, so turning it on early costs nothing.

### Turn it on

Set `RECOMMEND_ENGINE=graph`. `Recommend::generate` then publishes the starter
run as before, because those rows are the fallback, and pushes a
`GraphDeriveJob` onto your queue, so the graph engine needs a configured queue.
The job rebuilds the graph into a new generation and switches readers to it
only when the rebuild completes; a failed rebuild leaves readers on the
previous one. Run `recommendations:graph-rebuild` or
`Recommend::rebuild_graph()` to rebuild at once.

### What the graph holds

A rebuild reads the event stream, within each actor's `RECOMMEND_HISTORY`
bound, and the relations you declare:

- **Shared engagement.** Two documents get a `co_engaged` edge when at least
  `RECOMMEND_MIN_PEERS` actors hold both among their most recent
  `RECOMMEND_GRAPH_FANOUT` positives. Every event class counts, but only the
  aggregate is stored, so no edge describes one person.
- **Public engagement.** For the event types `RECOMMEND_PUBLIC_EVENT_TYPES`
  lists, each signed-in actor gets an `engaged` edge to the documents they
  engaged with. No type is public unless you list it; a type that is not
  public never becomes a per-actor fact.
- **Declared relations.** A relation reads related documents from a
  document's search projection, so the graph follows it without loading
  models. Its edges run both ways, as `relation:<name>` and
  `relation:<name>:inverse`.

```rust
use suprnova::{AttributeValue, FrameworkError, GraphRelation, Recommend, RecommendationTarget};

// Each track points at its artist, at 80 percent confidence.
pub fn declare_relations() -> Result<(), FrameworkError> {
    Recommend::graph_relation(GraphRelation::new("artist", "tracks", 8_000, |document| {
        Ok(match document.attributes.get("artist_id") {
            Some(AttributeValue::Integer(id)) => vec![RecommendationTarget {
                index: "artists".into(),
                key: id.to_string(),
            }],
            _ => Vec::new(),
        })
    })?)
}
```

Every fact carries the scope of the document it describes, that document's
source version, and its provenance: an event's id and content digest, or the
relation and the document revision it was read from. A rebuild is
deterministic, so rebuilding from the same stream and documents reproduces the
same facts. Between rebuilds the graph follows its sources: a search write
queues a `GraphDocumentJob` that removes what a deleted document held and
replaces what a newer revision changed, and `Recommend::prune` removes the
facts resting on the events it deletes. A removed fact is never read,
traversed or cited again.

### Where the graph lives

The graph lives behind the `GraphStore` trait, which `RECOMMEND_GRAPH_STORE`
selects:

- `database`, the default, keeps the graph in your application's database
  (SQLite, Postgres, MySQL or MariaDB), in the tables the recommendation
  migrations create, so the graph engine needs no other service.
- `memgraph` keeps it in a Memgraph server over the Bolt protocol. Enable the
  `graph-memgraph` feature and set `RECOMMEND_GRAPH_URL` to a `bolt://` or
  `bolt+s://` address without credentials; `RECOMMEND_GRAPH_USERNAME` and
  `RECOMMEND_GRAPH_PASSWORD` are read only when the store connects. Memgraph
  holds the graph in memory, so after its data is lost, run
  `recommendations:graph-rebuild` to restore it from the stream.

A store only persists facts and answers bounded hops; the framework applies
the viewer's scope at every hop and computes every score, so both stores rank
the same graph the same way. Bind your own `GraphStore` in the container to
replace both.

### How the graph ranks

A read authorizes before it scores. Seeds, negatives, every traversed edge
and every candidate must pass the viewer's scope, and a final candidate must
still exist and pass it when the read loads it. The blended score then weighs:

- graph affinity, the strongest path of at most `RECOMMEND_GRAPH_DEPTH` hops
  from one of the viewer's positives, weighted by that positive;
- semantic similarity in the search collection's own vectors (see below);
- social proximity (see below);
- freshness, from `published_at`, `created_at` or `updated_at`;
- the viewer's negatives near the candidate, subtracted;
- trust risk, the target's hide and report events weighed against its
  positive events, subtracted.

`RECOMMEND_GRAPH_SURFACES` sets per-surface weights, a confidence threshold
and diversity caps in basis points and counts; a surface it does not name uses
the defaults:

```bash
RECOMMEND_GRAPH_SURFACES='{"home":{"affinity":5000,"threshold":6000,"per_value":2}}'
```

The caps keep one list from collapsing: at most `per_value` results share one
facet value, at most `per_node` are reached through one intermediate node, and
at most `per_band` come from the most popular tenth of the catalog. The
confidence is the share of the list that graph paths or social proximity
support. Below the surface's `threshold` the engine returns the starter's rows,
with `explanation.engine` set to `starter` and `explanation.fallback_reason`
saying why.

A graph row's explanation has `engine` set to `graph` and `evidence` listing
the stored facts it rests on: the viewer's own events, as id and digest, the
edges of the path, and the seed documents, every one of which the viewer may
see. It never names a peer or a followed actor.

### Social proximity

Social proximity counts followed actors who publicly engaged with a target.
Set `RECOMMEND_ACTOR_INDEX` to the search index whose documents are your
users, keyed by user id; a viewer's `follow` events on that index's documents
name the actors they follow. Only `engaged` edges, which exist only for
public event types, count, and a target gains social proximity only when at
least `RECOMMEND_SOCIAL_MIN_ACTORS` followed actors agree on it, so a viewer's
list never reveals what one followed account did. The explanation's
`peer_count` reports how many agreed.

### Similar items, vectors and discover

The graph engine reads the vectors your search collection already stores; it
keeps no copy. Where the index's search engine is Qdrant, it uses Qdrant's own
recommend and discover operations by point id, under the viewer's scope
filter. On the database and collection engines, it folds the stored vectors
into one query, the positives' mean pushed away from the negatives', and ranks
the result. The positive and negative points are always the viewer's own
events on documents the viewer may see. A graph read never steers by another
person's events: `Recommend::for_user(id)` read by a viewer other than that
user answers with the starter's rows for the user, with `fallback_reason`
saying why, and so does `Recommend::for_actor(actor)` with a guest actor read
by a signed-in viewer.

`Recommend::similar_to(target)` seeds the graph and the vectors with one
document; the viewer's negatives push the vectors away and, like the targets
the viewer has already seen, are left out of the list.
`Recommend::discover(target)` searches near a document and ranks the
neighbourhood by context pairs built from the viewer's own positive and
negative events. Every negative joins a pair, repeating positives when there
are fewer of them, or pairing with the seed when the viewer has none. It needs
the graph engine and refuses a seed the viewer cannot see.

## Bridge search events

Set both `SEARCH_ANALYTICS=true` and `RECOMMEND_ENABLED=true` to forward shown
search impressions and validated clicks into the event stream. The bridge
uses the authenticated actor or a server-session hash, surface `search`, and
dedupe keys derived from impression id, event type, and shown position.
Impressions count as seen; clicks contribute the click score.

Search verifies the impression's session, expiration, key, and zero-based
position before forwarding a click. A forged key or position supplies no
recommendation event. The `POST /search/click` endpoint also requires CSRF.
See [Search](search.md) for analytics middleware and endpoint registration.

`RECOMMEND_ENABLED` controls automatic search forwarding. It does not disable
explicit `Recommend::record` or generation. Turning search analytics off
stops its event production; it does not erase recommendation history.

## Verify provenance and control retention

`Recommend::record` returns `EventProvenance { id, digest }`. The digest is
SHA256 over canonical event content, including its stored class and score.
Keep the pair together when you derive evidence. Verification re-reads content
and recomputes the digest:

```rust
use suprnova::{EventProvenance, FrameworkError, ProvenanceState, Recommend};

async fn evidence_is_intact(reference: &EventProvenance) -> Result<bool, FrameworkError> {
    Ok(Recommend::verify_provenance(reference).await? == ProvenanceState::Intact)
}
```

`Intact` means content matches, `Altered` means the id exists with changed or
invalid content, and `Removed` means retention removed the event. A
configuration change does not rewrite an earlier scoring snapshot.

Engines never prune or rewrite the append-only event stream. Your application
owns retention. Run `recommendations:prune --older-than 90` only when that is
your retention policy; the argument is a positive number of days. There is
no automatic event-expiration default. Pruning search analytics is separate.

## Commands

Run these through your application's binary:

| Command | Arguments and flags | Purpose |
|---|---|---|
| `recommendations:generate` | `[--surface name] [--user id]` | Publish a complete snapshot or report lock contention. |
| `recommendations:status` | None | Print the ten latest runs, including failures. |
| `recommendations:prune` | `--older-than days` | Apply your event retention policy. |
| `recommendations:graph-rebuild` | None | Rebuild the graph engine's facts into a new generation. |

Value flags accept `--name value` or `--name=value`. Unknown, repeated, empty,
and missing values fail. You can also call `Recommend::generate(GenerateRun)`,
`Recommend::status(limit)`, and `Recommend::prune(days)`; status limits are at
most 1000. `GenerateRun` has optional `user` and `surface` strings.

## Migrations and the data contract

Extend your application's migration list with
`suprnova::recommendations::migrations()`. The kits already include this and
`suprnova::search::migrations()`. The recommendation migrations create:

| Table | Stored contract |
|---|---|
| `recommendation_events` | Immutable id, actor kind/id, target index/key, type, weight, surface, UTC microsecond time, access scope (the index scope's guest filter and the target's values for the index's `scope_fields` when the event was recorded), source version, dedupe key, class, and score. |
| `recommendation_explanations` | Run id, event-id/digest provenance pairs, attribute values, peer count, shared-target count and references, and fallback tier. |
| `starter_recommendations` | Run id, actor, surface, target index/key, score, rank, and explanation id. |
| `recommendation_runs` | Publication state and batch statistics, watermarks, catalog digest, history snapshots, and error. |
| `recommendation_schema` | `recommendation_schema_version`, 2 once the graph migration runs. |
| `recommendation_graph_generations` | Graph generations and which one readers use. |
| `recommendation_graph_nodes` | Nodes per generation: key, type, target, scope, source version, and provenance. |
| `recommendation_graph_edges` | Edges per generation: key, endpoints, type, confidence, scope, source version, provenance, and times. |

The data this engine builds is the graph engine's input under that versioned
contract. Event identity plus content digest distinguishes intact evidence
from facts removed by retention. These tables preserve the handoff without
requiring a graph to serve starter recommendations; the graph engine reads
them when you select it.

## Variables

| Variable | Default | Purpose |
|---|---|---|
| `RECOMMEND_ENABLED` | `false` | Forward validated search events automatically. |
| `RECOMMEND_HISTORY` | `500` | Newest positive and negative events per actor combined. |
| `RECOMMEND_PEERS` | `200` | Most recent shared peers to sample. |
| `RECOMMEND_MIN_PEERS` | `3` | Distinct peers a collaborative candidate needs. |
| `RECOMMEND_POPULARITY_WINDOW` | `30` | UTC days of positive activity for popularity. |
| `RECOMMEND_LOCK_TTL_SECS` | `3600` | Renewable generation lock lifetime. |
| `RECOMMEND_EVENT_TYPES` | `{}` | JSON class and base-score overrides. |
| `RECOMMEND_ENGINE` | `starter` | `starter` or `graph`. |
| `RECOMMEND_GRAPH_STORE` | `database` | `database` or `memgraph`. |
| `RECOMMEND_GRAPH_URL` | unset | The Memgraph Bolt address, without credentials. |
| `RECOMMEND_GRAPH_USERNAME` | unset | The Memgraph user, read when the store connects. |
| `RECOMMEND_GRAPH_PASSWORD` | unset | The Memgraph password, read when the store connects. |
| `RECOMMEND_GRAPH_DEPTH` | `2` | Most hops in a graph path, 1 to 4. |
| `RECOMMEND_GRAPH_FANOUT` | `50` | Most edges followed out of one node per hop. |
| `RECOMMEND_GRAPH_MAX_NODES` | `2000` | Most nodes one walk reaches. |
| `RECOMMEND_PUBLIC_EVENT_TYPES` | unset | Comma-separated event types other viewers may see. |
| `RECOMMEND_SOCIAL_MIN_ACTORS` | `3` | Followed actors who must agree for social proximity. |
| `RECOMMEND_ACTOR_INDEX` | unset | The search index whose documents are users. |
| `RECOMMEND_GRAPH_SURFACES` | `{}` | JSON per-surface weights, threshold, and caps. |

`RecommendConfig::current()` uses registered configuration, environment
configuration, or a task-local override from `RecommendConfig::scope`.
Malformed values and zero numeric bounds fail with the variable name.
See [Environment Variables](env-vars.md#recommendations) for types and limits.

## Testing

Use a migrated test database, a scoped searchable test model, a fresh
`CollectionEngine`, and `SearchRegistry::isolated` to exercise the real event
and read paths without external services. Set `SearchConfig.queue=false`
within `SearchConfig::scope` when your test needs inline indexing. Add a
`FakeEmbedder` only when testing semantic neighbours, and keep provider
qualification separate from ranking tests.

Record shared positives for the subject and three peers, an unseen candidate
the peers share, and negative and seen facts for the subject. Generate a run and check the candidate,
fallback order, authorized explanations, immutable dedupe result, and
provenance after retention. Also check that a failed run leaves the previous
completed run readable and that another viewer cannot read an inaccessible
seed or target.

The repository's suite includes these behaviors and an ignored million-event
budget fixture:

```bash
cargo nextest run -p suprnova --test recommendations
cargo nextest run -p suprnova --test recommendations --run-ignored only \
    -E 'test(million_events_complete_within_the_committed_budget)'
```

### Why Suprnova diverges

Laravel has no first-party recommender. Suprnova supplies an immutable event
stream, an explainable SQL starter, and a knowledge-graph engine behind a
replaceable trait. You reuse search documents, scopes, and vectors, keep the
graph in your own database or in Memgraph, and retain provenance from every
recommendation back to its events instead of tying interaction history to a
hosted recommendation service.

## Next

- [Search](search.md) - declare targets, scopes, facets, and semantic providers.
- [Task Scheduling](scheduling.md) - register generation and retention tasks.
- [Cache](cache.md) - configure the generation lock backend.
- [Migrations](migrations.md) - register the versioned data contract.
- [Environment Variables](env-vars.md) - configure bounds and event scores.
