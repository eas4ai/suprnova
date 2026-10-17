# Recommendations

You record interactions with searchable documents and read recommendations
through `Recommend`. The starter engine uses SQL events, search facets,
timestamps, scopes, and optional search vectors. You need no embedding model
to generate collaborative, attribute-affinity, popularity, and recent results.
The `Recommender` trait lets you replace the engine without changing callers.

The purpose of the starter engine is to build the data the graph recommender
needs: immutable events, materialized recommendations, explanations, and run
records under a versioned storage contract. The shipped engine is `starter`.

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
| `recommendation_schema` | `recommendation_schema_version`, initialized to 1. |

The data this engine builds is the graph engine's input under that versioned
contract. Event identity plus content digest distinguishes intact evidence
from facts removed by retention. These tables preserve the handoff without
requiring a graph to serve starter recommendations. Graph traversal and media
retrieval are not exposed by the shipped starter API.

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
stream and an explainable SQL starter behind a replaceable trait. You reuse
search documents, scopes, and vectors, publish complete runs, and retain
provenance for the graph recommender's input instead of tying interaction
history to a hosted recommendation service.

## Next

- [Search](search.md) - declare targets, scopes, facets, and semantic providers.
- [Task Scheduling](scheduling.md) - register generation and retention tasks.
- [Cache](cache.md) - configure the generation lock backend.
- [Migrations](migrations.md) - register the versioned data contract.
- [Environment Variables](env-vars.md) - configure bounds and event scores.
