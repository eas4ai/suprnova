# RenderCache Representations

A cached route does not store "a page". It stores a **representation**: one
concrete response, under one lookup key, in one or more storage layers, with
enough metadata beside it to answer a conditional request and to prove later
that it is still current. Two visitors get the same stored bytes only when
the key they derive is the same key, and the key is derived from what the
route declared - never from what the handler happened to do.

This chapter is about that stored thing. What forms a representation can
take (`Complete` and `Composite`), what goes into its key, which layers it
is written to, the `ETag`, `Cache-Control`, `Vary`, `Age`, and `Warning` a
served hit carries, the four freshness states it can be in, how it answers
`If-None-Match` and `HEAD`, and what `PrivateCached` and
`PublicShellStitched` actually store. *Why* a representation leaves the
fresh band - a write, an epoch advance - is the next chapter's subject; here
it is enough that the bands exist and that one representation sits in one of
them. Every example below is a route in this repository's dogfood application
(`app/src/live/mod.rs`) and is proved by a named test in
`app/tests/live_render_cache.rs`.

## Two entry forms

A stored entry is one of two kinds.

- **`Complete`** is a finished answer: a status, a set of replayable
  headers, and one body buffer. Serving it copies nothing and runs nothing.
  Every `PublicShared` and `PrivateCached` route stores this form.
- **`Composite`** is a shared **shell** with typed holes cut in it, plus a
  segment graph saying what goes back into each hole. Only
  `RepresentationClass::PublicShellStitched` stores this form, and only a
  Live document produces one.

The class you declare in the policy decides which form is even reachable.
`/live/public` and `/live/todos` both declare `PublicShared`;
`the_database_profile_serves_a_hit_through_the_sql_stores` reads
`/live/todos`'s published entry back out of the store and asserts it is an
`EntryKind::Complete` one, and
`the_public_document_is_a_hit_whose_seed_still_promotes` reads
`/live/public`'s back through `RenderCache::inspect_route_for_test` and
asserts the class it was stored under. That matters, because "it was stored"
and "it was silently declined" produce the same response: the claim has to
be made against the entry, not against what the visitor sees.

## The lookup key

The key a request derives is built from the route pattern, its path
parameters, the query parameters the policy declared, each declared variance
dimension's resolved value, the application build id (`APP_BUILD_ID`), and
the current authority epoch. Nothing else. A query parameter that arrives on
the request but is not named by `QueryPolicy::declared` bypasses the cache
for that request rather than being quietly dropped from the key, because
dropping it would serve the wrong page to whoever sent it.

The key is text an operator can hold: `RenderCache::key_for_route_for_test`
in `the_operator_commands_inspect_without_a_body_and_advance_the_epoch`
asserts that it starts with `rk1.`, and `render-cache:inspect` takes exactly
that text.

Because the epoch is part of the key, an epoch advance does not have to find
and delete anything. Every previously stored entry simply stops being
reachable by ordinary lookup at the next request. That is the mechanism the
[Operations](render-cache-operations.md) chapter's emergency invalidation
relies on.

## Which layers a policy writes to

There are two storage layers. **L0** is in-process memory, bounded by
`RENDER_CACHE_L0_ENTRIES` and `RENDER_CACHE_L0_BYTES`. **L1** is whatever
the deployment profile configures - a directory of files, a database table,
or Redis - and is shared by every process that points at it.

The policy builder stores in **L0 only** unless you say otherwise:
`StorageLayers::l0_only()` is the default. A route that is worth putting in
the shared tier declares it:

```rust
use suprnova::render_cache::{
    FreshnessPolicy, RenderCachePolicy, RepresentationClass, StorageLayers,
};

let router = router.try_render_cache(
    "/live/todos",
    RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .freshness(FreshnessPolicy::new(300_000, 60_000, 300_000)?)
        .layers(StorageLayers::l0_and_l1())
        .build()?,
)?;
```

That is `/live/todos`'s declaration from `app/src/live/mod.rs`. It is the
one document in that application whose bytes every node can share, so it is
the one that declares `l0_and_l1()`. Under the embedded profile, where L1 is
disabled unless `RENDER_CACHE_L1_DIR` names a directory, declaring the layer
changes nothing; under the Database profile the entry lands in
`suprnova_render_entries` and a second process finds it there.

`the_database_profile_serves_a_hit_through_the_sql_stores` is the proof.
It boots the application on the Database profile's providers, reads the
published entry directly out of L1 under the very key the middleware
derived, then empties L0 and asks again - and the second request is still
answered without the handler running. An in-memory hit would look identical
from the client's side, which is why the test reaches for the store.

Pick the layers per route rather than globally. L1 costs a round trip on a
miss that L0 alone does not, and an entry that only one node will ever ask
for is not worth putting where every node can see it.

## The metadata a served hit carries

Five response fields describe a served representation, and this is where
they are defined; the other chapters use them without restating them.

| Field | What it says |
|---|---|
| `ETag` | A strong validator over exactly the bytes sent. A client may send it back as `If-None-Match`. |
| `Cache-Control` | `private` for every class by default. A `PublicShared` route that sets `SharedCachePolicy::SMaxAge` also gets `public` and `s-maxage`, which is the only way a shared proxy is ever invited to keep the bytes. A `Composite` document with at least one island is `private, no-store`, whether it was assembled on a hit or rendered by the leader that published the shell. |
| `Vary` | Derived from the declared variance dimensions that imply a request header: `Locale` implies `Accept-Language`, `Media` implies `Accept`, `Encoding` implies `Accept-Encoding`. A dimension that implies none adds nothing. The names are emitted sorted by header name, not in the order you declared the dimensions. |
| `Age` | Whole seconds since the representation was published. Its presence is the simplest local proof that a response came out of the store. |
| `Warning` | `110 - "Response is Stale"`, and only on a response served past its fresh interval. |

The dimension-to-header mapping is `VarianceDimension::vary_header` in
`crates/suprnova-live/src/render_cache/variance.rs`. Two engine tests prove
the `Locale` and `Encoding` halves of it and the joined header value:
`a_descriptor_orders_dimensions_and_bounds_values`
(`crates/suprnova-live/tests/render_cache_variance.rs`) asserts a descriptor
carrying both reports `["Accept-Encoding", "Accept-Language"]`, and
`cache_control_and_vary_agree_with_class_variance_and_seed_deadline`
(`crates/suprnova-live/tests/render_cache_coherence.rs`) asserts the same
pair emits `Accept-Encoding, Accept-Language` and that a descriptor with no
header-implying dimension emits no `Vary` at all. `Media` implying `Accept`
is documented from the code; no test here pairs it.

Three of the response values are asserted against the running application:
`the_public_document_is_a_hit_whose_seed_still_promotes` reads
`private, max-age=300` off `/live/public` and requires an `Age` header on
the second request;
`the_private_document_is_cached_per_principal_and_never_crosses` reads
`private, max-age=60` off `/live/me`;
`the_dashboard_is_stitched_per_principal_from_one_shared_shell` reads
`private, no-store` off the dashboard on the render that publishes its shell
as well as on the assembled hit after it, because that value follows what the
bytes hold and not which path produced them.

## The four freshness states

Every hit resolves to exactly one of four states before anything is served.
`FreshnessPolicy::new(fresh_ms, stale_servable_ms, stale_on_error_ms)` sets
them. **The two stale windows are both measured from the end of the fresh
interval, not stacked one after the other** - this is the detail that trips
people up:

| State | Age since publication | What the visitor gets |
|---|---|---|
| Fresh | below `fresh_ms` | the stored bytes, no `Warning` |
| Stale-servable | past `fresh_ms` by less than `stale_servable_ms` | the stored bytes immediately, under `Warning`, with a bounded rebuild spawned behind the request |
| Stale-on-error | past `fresh_ms` by at least `stale_servable_ms`, and by less than `stale_on_error_ms` | a foreground rebuild; the stored bytes under `Warning` only if that rebuild itself fails |
| Dead | past `fresh_ms` by the larger of the two windows or more | nothing; the request renders |

`/live/todos` declares `FreshnessPolicy::new(300_000, 60_000, 300_000)`, so
it is fresh for five minutes, stale-servable for the sixth, stale-on-error
until ten minutes, and dead after that.

Two rules override the bands. A `PrivateCached` representation is **never**
served stale: past its fresh interval it is Dead, which is why `/live/me`
declares `FreshnessPolicy::new(60_000, 0, 0)` - a stale band there would
read as a promise the cache does not keep. And a stored public-seed
document whose promotion deadline has passed is Dead whatever its intervals
say, because a seed past its deadline can never be promoted again.

`stale_service_is_marked_and_rebuilt_in_the_background` drives `/live/todos`
across the first boundary on a controlled clock and asserts the served body,
`Warning: 110 - "Response is Stale"`, and `Age: 300`. What *causes* a
representation to leave the fresh band early - a write, an epoch advance -
is [RenderCache Generations](render-cache-generations.md)'s subject.

## Conditional requests and HEAD

A client that sends a served `ETag` back as `If-None-Match` gets a `304`
with no body, and a `HEAD` gets the headers with no body. Neither reaches
your handler:

```
GET  /live/todos                          -> 200, ETag: "..."
GET  /live/todos  If-None-Match: "..."    -> 304, empty body
HEAD /live/todos                          -> 200, same ETag, empty body
```

`conditional_and_head_requests_are_answered_from_the_stored_entry` asserts
all three against the running application, including that the render count
does not move across the last two.

One exception, and it is deliberate: a `Composite` response never answers
`304`. Every assembly is a distinct representation - fresh island identities,
a fresh bootstrap nonce where the document has one - so a `304` would tell
the client to pair the body it already holds with headers minted for this
request. The `ETag` on an assembled response is still strong over exactly
the bytes that were sent; it simply never matches a later request. Step 7
of `the_dashboard_is_stitched_per_principal_from_one_shared_shell` sends a
served validator straight back and asserts a `200` with a different `ETag`.

## A representation that belongs to one person

`RepresentationClass::PrivateCached` stores one representation per
signed-in visitor. It is refused at build time unless the policy also
declares `Principal` or `Tenant` variance, so the pairing cannot drift apart
by accident:

```rust
use suprnova::render_cache::{
    FreshnessPolicy, RenderCachePolicy, RepresentationClass, VarianceDimension,
};

let router = router.try_render_cache(
    "/live/me",
    RenderCachePolicy::builder(RepresentationClass::PrivateCached)
        .freshness(FreshnessPolicy::new(60_000, 0, 0)?)
        .vary(VarianceDimension::Principal)
        .build()?,
)?;
```

The handler behind it is an ordinary one. It resolves the signed-in visitor
and renders their name:

```rust
pub async fn me(_request: Request) -> Response {
    let result: Result<HttpResponse, FrameworkError> = async {
        let user = Auth::user_as::<User>()
            .await?
            .ok_or_else(|| FrameworkError::internal("Live account document without a principal"))?;
        html(&MeView { name: user.name })
    }
    .await;
    result.map_err(failed)
}
```

Nothing extra is wired up to make that cache. The route carries the same
`AuthMiddleware::redirect_to("/login")` the dashboard does, so an anonymous
visitor is redirected before the handler runs, and the principal itself is
resolved inside the render. Reading the signed-in visitor's identity out of
the session is classified as an **identity read**, not a session read, so
the render narrows to `PrivateCached`, the key carries opaque per-principal
material, and the two agree.

`the_private_document_is_cached_per_principal_and_never_crosses` signs in
two visitors, hits twice each with no render, and asserts that each body
names its own person and not the other's; a third visitor renders, because
they share nothing with either. The served `Cache-Control` is
`private, max-age=60`, so no shared proxy is ever offered the bytes. The
same test shows the other half of the bargain: the render resolves its
principal through the provider that reads the `users` table, so seeding a
third visitor invalidates every stored `/live/me` entry, and the next
request for each rebuilds. That is table-granular invalidation doing exactly
what [Generations](render-cache-generations.md) describes.

Two consequences of that classification are worth knowing before you
declare the class:

- An **anonymous** request to a `PrivateCached` route with `Principal`
  variance caches under the `Anonymous` key. The render resolved no
  identity, so no principal material was observed, the key says `Anonymous`,
  and the two agree. A signed-in visitor derives a `Private` key that can
  never reach that entry. This applies when such a request actually renders
  a `200`, which `/live/me` never does - its login redirect answers a `302`,
  and a `302` is refused by eligibility before any of this is consulted. The
  framework test that does reach it is
  `an_anonymous_render_resolving_identity_through_the_session_caches_anonymously`
  in `framework/tests/render_cache/middleware.rs`.
- A **named guard's** identifier is principal material in exactly the same
  way as the default guard's. Reading it records a principal read and, when
  there is an id, the value.

And one rule that has not moved: a route that reads the principal *without*
declaring `Principal` variance is declined from storage. There is no way to
key such an entry per visitor, so it is never stored rather than shared.
Every other session value still forces `Uncacheable`; see the
classification list in [RenderCache](render-cache.md).

## A shell with holes in it

`RepresentationClass::PublicShellStitched` is for a Live document whose
frame is the same for everybody and whose islands are not. The stored entry
holds the shell alone. No identity-bound island's markup and no signed
snapshot is ever inside the stored bytes; every hit re-mounts every island
for whoever is asking, under authority derived for that request.

This repository's dashboard is that route:

```rust
router.try_render_cache(
    "/live",
    RenderCachePolicy::builder(RepresentationClass::PublicShellStitched)
        .freshness(FreshnessPolicy::new(300_000, 60_000, 300_000)?)
        .build()?,
)
```

`the_dashboard_is_stitched_per_principal_from_one_shared_shell` asserts what
that buys and what it costs. The stored entry is an `EntryKind::Composite`
with three slots, one per identity-bound island. A second principal is
answered from that shell, and the two documents differ **only** in their
island tags - the test strips the three island tags from each and compares
what is left, byte for byte. The route's own login redirect still runs on
every hit: a stitched hit is forwarded through the route's whole middleware
chain before anything is served, so an anonymous visitor gets the redirect,
never an assembled document.

A stitched document with at least one slot is sent
`Cache-Control: private, no-store`, on the render that publishes the shell as
much as on every assembly after it. It holds one principal's islands under
authority re-derived for one request, and a `max-age` would let a shared
browser profile replay them to whoever sits down next; which path produced the
bytes does not change what is in them. A zero-slot `Composite`
carries no per-principal bytes at all, only a per-request nonce, so it keeps
the class's private `max-age` like any other private representation;
`a_zero_slot_composite_is_assembled_with_a_fresh_nonce_on_every_hit` in
`framework/tests/render_cache/stitch.rs` asserts that. Either way the class
refuses `SharedCachePolicy::SMaxAge` at policy build time, so no shared
proxy is ever offered the bytes.

Two limits to know: the class is meaningful only on a route whose chain ends
in the Live completion middleware, so use it with `LiveDocument::render` and
nothing else, and a stitched entry is never served by the stale-on-error
fallback and never triggers a background rebuild. The
[Generations](render-cache-generations.md) chapter says what that second one
means in practice.

### Why Suprnova diverges

Laravel has no server-side representation model at all. Its response-caching
packages store the rendered output of a route under a key you compose
yourself - typically the URL, sometimes the URL plus a hand-written suffix
for the logged-in user - and hand it back on the next request. There is one
form of stored thing, it is always a finished body, and whether two visitors
share it is a property of the string you built.

Suprnova makes the key a declaration and the form a consequence. You name
the class and the variance dimensions; the framework derives the key,
refuses `PrivateCached` without a partitioning dimension, compares what the
render actually observed against what the key actually said, and declines to
store the render when the two disagree. And because `PublicShellStitched`
exists, a page that is 95 percent shared and 5 percent private does not have
to choose between caching nothing and caching something it should not: the
shared part is stored once and the private part is re-rendered per request,
with the private bytes never entering the store.

## Next

- [RenderCache Generations](render-cache-generations.md) - how a stored
  representation stops being current, and what happens next
- [RenderCache](render-cache.md) - declaring policies, variance, and the
  reasons a render is never stored
- [Live](live.md) - the islands a stitched shell has holes for
