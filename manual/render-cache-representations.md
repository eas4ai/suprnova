# RenderCache Representations

A cached route does not store "a page". It stores a **representation**: one
concrete response, under one lookup key, in one or more storage layers, with
enough metadata beside it to answer a conditional request and to prove later
that it is still current. Two visitors get the same stored bytes only when
the key they derive is the same key, and the key is derived from what the
route declared - never from what the handler happened to do.

This chapter is about that stored thing. What forms a representation can
take (`Complete` and `Composite`), what goes into its key, which layers it
is written to, how it answers `If-None-Match` and `HEAD`, and what
`PrivateCached` and `PublicShellStitched` actually store. Whether a stored
representation is still *current* is the next chapter's subject; here it is
enough that one exists. Every example below is a route in this repository's
dogfood application (`app/src/live/mod.rs`) and is proved by a named test in
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

## Conditional requests and HEAD

A served `Complete` hit carries a strong `ETag`. A client that sends it back
as `If-None-Match` gets a `304` with no body, and a `HEAD` gets the headers
with no body. Neither reaches your handler:

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
  never reach that entry.
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

An assembled document is sent `Cache-Control: private, no-store`. It holds
one principal's islands under authority re-derived for one request, and a
`max-age` would let a shared browser profile replay them to whoever sits
down next. The class refuses `SharedCachePolicy::SMaxAge` at policy build
time for the same reason.

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
