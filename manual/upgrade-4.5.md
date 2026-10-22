# Upgrading to 4.5

This chapter moves an application from Suprnova 4.4.0 to 4.5.0. Each section
matches an entry in the 4.5.0 [changelog](../CHANGELOG.md).

## Who this chapter is for

- A person upgrading an existing application from 4.4 to 4.5.
- A maintainer looking up the exact bold lead sentence of a changelog entry.

## How to use this chapter

Find the changelog entry below, then read **Before**, **Now** and **Do**.
Three changes can need an edit: a `match` over one of three enums with no
wildcard arm, a custom `UserProvider` that upgrades password hashes at
sign-in, and a custom graph store. One change adds a column that your
migrations create. On MySQL and MariaDB an email address now has to equal the
stored one; read that entry before you deploy if your users have accented
addresses. Five fixes live in files that `suprnova new` writes into a project,
so an existing project copies them by hand; the section before the last one
lists the edits.

## Upgrade in this order

1. Point application dependencies and the CLI at the release you are
   upgrading to, `v4.5.0` or later. The examples pin the current release:

   ```toml
   suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v4.5.1" }
   ```

   ```bash
   cargo update -p suprnova
   cargo install --force --git https://github.com/eas4ai/suprnova.git --tag v4.5.1 suprnova-cli
   cargo check --all-targets
   ```

2. Add the arms the three new enum variants need, where `cargo check` reports
   them.
3. Run your migrations, so `recommendation_runs` has its `ranking_digest`
   column before the next generation runs.
4. If you changed `SEARCH_EMBED_MODEL`, or its revision, since the index was
   last built, run `search:reindex` for each affected index.
5. Copy the starter kit edits from the section below into your project.
6. Run your tests.

## Changed entries

### Three public enums have new variants.

**Before:** `Revocation`, `RegistrationOutcome` and `MetadataErrorKind` had
the variants 4.4 documented.

**Now:** `Revocation` gains `DocumentRelationsBefore(target, version)`,
`RegistrationOutcome` gains `Unavailable` and `MetadataErrorKind` gains
`TooManyBrowserTools`.

**Do:** where your code matches one of them with no wildcard arm, add the
arm. The `Revocation` arm does real work; its own entry below describes it.
`RegistrationOutcome::Unavailable` comes back from `register_password` when
the store refused the insert as a duplicate while no account holds the exact
address; answer it as you answer `Existing`, so a collision reveals nothing.
`MetadataErrorKind::TooManyBrowserTools` reports a Live page that would offer
more than 64 browser tools; the browser tools entry below has the details.

### `UserProvider` gained `rehash_password_for_sign_in(user, credentials, recorded_epoch)`, the rehash the session guard's `attempt` calls.

**Before:** after the password validated, the session guard's `attempt` called
`rehash_password_if_required(user, credentials)` and then read the user again.

**Now:** `attempt` calls `rehash_password_for_sign_in(user, credentials,
recorded_epoch)` and keeps the credential epoch it captured when it verified
the password, unless the method returns the snapshot it wrote. The default
calls `rehash_password_if_required` only for a user with no credential epoch.
A user with a password hash always has one, so for such users the default
rewrites nothing. `EloquentUserProvider` and `DatabaseUserProvider` implement
the new method. `once` and `Auth::logout_other_devices` still call
`rehash_password_if_required`.

**Do:** a custom provider that upgrades hashes at sign-in implements
`rehash_password_for_sign_in`: check that the stored epoch and hash are still
the ones `attempt` verified, write the new hash in that same atomic step, and
return the record you wrote, or `Ok(None)` when nothing changed. Return an
error when the epoch moved; that fails the sign-in. Until you implement it,
such a provider keeps its old hashes at sign-in and reports nothing.
[Password rehash on sign-in](auth-flows.md#password-rehash-on-sign-in) has a
complete example. A provider that never rewrites hashes needs no edit.

### Custom graph stores must handle `DocumentRelationsBefore`, which removes stale relation facts while keeping the document node and its engagement edges.

**Before:** a document revision was processed with
`Revocation::DocumentBefore(target, version)`, which removed the older
document node and every edge touching it, `engaged` and `co_engaged` edges
included. The job then wrote the node and its declared relations again, and
the engagement edges stayed gone until a full rebuild.

**Now:** the job saves the new node in place, then revokes
`Revocation::DocumentRelationsBefore(target, version)`, which removes only
the relation-derived edges and auxiliary nodes older than `version` and keeps
the document node and its event-backed edges. `DocumentBefore` keeps its
broader behaviour. A retry of the same revision after a failure finishes the
relation writes.

**Do:** in a custom `GraphStore`, add the arm to `revoke`: delete the edges
that declared relations produced for the document at a source version older
than `version`, and the auxiliary nodes only those relations created; leave
the document node and the engagement edges in place. `DatabaseGraphStore` and
`MemgraphGraphStore` already do. [Recommendations](recommendations.md)
describes each revocation kind.

### Serialized recommendation explanations now omit social and shared-engagement edge references.

**Before:** a graph explanation's `evidence` listed every edge, including the
`engaged` and `co_engaged` kinds. Their keys are deterministic, so a client
that knew which accounts the viewer follows could recompute them and learn
which account engaged with the recommended item.

**Now:** serializing a `RecommendationExplanation` leaves those two edge kinds
out. The server-side value keeps them, so verification on the server still
works. Declared relation edges, events and documents are unchanged.

**Do:** a client that counted or displayed `edge` evidence sees fewer entries.
Verify engagement with the server-side explanation, never with the serialized
one.

### `RecommendationRun` now exposes `ranking_digest`.

**Before:** a run recorded nothing about the settings that ranked it. After
you raised `RECOMMEND_MIN_PEERS`, an unchanged incremental run kept copying a
user's previous list, and reads returned rows a single peer had produced.

**Now:** each run stores a digest of its ranking settings. An incremental run
regenerates every user whose previous run carries a different digest, and
reads enforce the current peer minimum on stored rows, so a list can shrink
after the upgrade until the next full run.

**Do:** run your migrations. The column comes with
`suprnova::recommendations::migrations()`, which the kits already append to
the migrator. Until it exists, generating recommendations fails with a
database error about the missing column.

### Starter recommendation reads load ranked candidates in bounded pages and load similarity histories only for selected peers and the viewer.

**Before:** a starter read loaded the whole visible catalog and every peer
history, then filtered. Only the graph engine asked a search engine which keys
it holds.

**Now:** a starter read pages through the ranked candidates with a small
lookahead, hydrates them by key, and loads similarity histories only for the
selected peers and the viewer. To do that it asks the search engine's
`stored_keys` which candidates the index still holds, as graph reads already
did.

**Do:** a custom `SearchEngine` that serves recommendations implements
`stored_keys`; the default returns an error, so starter reads fail until it
does. The built-in engines implement it.

### Indexing and semantic queries now require the provider's model id and revision to match the generation.

**Before:** a generation recorded the embedding dimensions only. Switching
`SEARCH_EMBED_MODEL` to another model with the same dimensions mixed its
vectors with the old ones.

**Now:** a generation pins the model id and revision. Indexing or querying
with a different model fails with an error that names the pinned and the
resolved model and tells you to rebuild.

**Do:** if you changed `SEARCH_EMBED_MODEL`, or its revision, since the last
build, run `search:reindex <Model or index>` before you deploy. Otherwise
nothing.

### Successful search requests now bootstrap session and CSRF cookies for new clients.

**Before:** a first JSON search from a client without a cookie returned an
impression that no later click could validate, because the click arrived
without the session that issued it.

**Now:** the search route runs the CSRF middleware, so the response sets the
session and CSRF cookies.

**Do:** a client that reports clicks sends back the cookies the search
response set, and the CSRF token as on any other request that changes state.
A browser already does.

### Search touches now report an error when a declared relation key is missing or cannot be read from the model.

**Before:** `#[searchable(touches("comments"))]` read the related key from
the child's JSON output. A child that hid or renamed the key in its output
silently skipped the parent, which kept a stale document.

**Now:** the key is read from the model's stored columns. A relation whose key
column the model cannot supply fails the write with an error that begins
`search touches`.

**Do:** nothing, unless a write fails with that error; then name a relation
whose key is a column of the child model.

### On MySQL and MariaDB, an email address now matches an account only when it equals the stored address after trimming and lowercasing.

**Before:** lookups by email compared through the column's collation. The
default MySQL and MariaDB collations fold accents and some letter pairs, so
`jose@example.com` matched an account stored as `josé@example.com` at sign-in,
password reset, email verification, passkeys and OAuth linking, and a magic
link could reach the wrong mailbox.

**Now:** a lookup matches only when the address equals the stored one byte for
byte after trimming and lowercasing. The verification stamp compares with
`BINARY`. Postgres and SQLite already compared exactly. A sign-up whose
address collides with an existing account only under the looser comparison
answers the generic response and creates nothing.

**Do:** nothing in code. A user who typed an unaccented form of an accented
address types it exactly after upgrading. A custom `UserStore` implements
`find_by_email` itself: compare normalized bytes there rather than trusting
the collation.

### Quoted phrases now constrain semantic and hybrid searches using each engine's lexical phrase matching rules.

**Before:** a quoted phrase ranked semantic and hybrid results but did not
exclude documents without it.

**Now:** those searches exclude documents missing a required phrase and refill
the slots they filtered, so the result keeps its depth.

**Do:** nothing. A query with a quoted phrase returns fewer, exact results.

### Database search vector delivery now uses retryable claims.

**Before:** a vector delivery ran inside the SQL transaction that recorded it,
so a slow vector store held the connection and could deadlock a shared pool.

**Now:** a delivery claims its row, leaves the transaction, calls the vector
store with a 45-second timeout and records the result. A second delivery of
the same row while the first is running reports an in-flight error and is
retried.

**Do:** nothing.

### New starter kits offer more search results only when another accessible page is available.

**Now:** the notes search in a freshly scaffolded project reads
`result.pagination.has_more` instead of comparing the total with the hits
shown. **Do:** copy the edit into your project; the starter kit section below
names the lines.

### Newly generated starter explanations retain every contributing history event.

**Now:** an explanation generated under 4.5 keeps every history event that
produced it, so verification can tell when a contributor was removed.
Explanations generated before the upgrade keep their old provenance.
**Do:** run `recommendations:generate` once after upgrading.

### Eloquent counter updates now use the saved row to check search eligibility and invalidate cached pages.

**Now:** `increment` and `decrement` reload the row they changed before
deciding whether the model belongs in its index and which cached pages to
drop. **Do:** nothing.

### Event retries now resolve their original provenance before checking the target's current existence or access scope.

**Now:** a retried recommendation event is matched to its first write before
the target is loaded, so a target deleted or rescoped since then no longer
fails the retry. **Do:** nothing.

### Worker boot now constructs configured search providers before validating searchable indexes.

**Now:** `queue:work` and the other worker processes build the embedding
provider first, so index validation at boot sees the same provider the server
uses. **Do:** nothing.

### Configured search providers are now available inside application bootstrap hooks for server, worker and console processes.

**Now:** the framework runs search boot before your `bootstrap` hook, so the
hook can create a generation with the configured provider. **Do:** nothing;
the starter kits already do this, and their `expect` message changed to name
the likely causes of a failure.

### Qdrant now supports retained flush fences and index deletion through the search maintenance commands.

**Now:** `search:flush` records a fence that Qdrant keeps for the configured
tombstone lifetime, and `search:delete-index` removes the index's collections
and generation metadata. **Do:** nothing.

### Qdrant generation deletion can now resume from the dropping state while ordinary writes continue to reject that state.

**Now:** pruning a generation whose deletion was interrupted finishes it
instead of failing. **Do:** nothing.

### Qdrant hash lookups now retain the source version when skipping unchanged documents, preserving stale-write protection.

**Now:** a document skipped as unchanged keeps its latest version, so a delayed
older removal cannot delete it. **Do:** nothing.

### Loading a searchable model without timestamps now reads its content and search version together.

**Now:** one SELECT returns the row and its kept version on Postgres, MySQL and
SQLite; a locking read keeps its row lock until the version is loaded.
**Do:** nothing.

### Saving a searchable model without timestamps now updates the search version carried by that same instance, including saves in an explicit transaction.

**Now:** after `save()` or `save_with_tx()`, `to_search_document()` on the
instance you saved reports the new version. **Do:** nothing.

### Graph traversal now replaces earlier paths with stronger later paths, including when the node limit is full.

**Now:** a stronger route found later replaces a weaker route that reached the
item first, and the explanation names the route used. **Do:** nothing.

## Fixed entries that change behaviour

### Agent tools now refuse guards that use a different user provider from the caller's default guard, preventing matching ids from signing in as another person.

**Before:** a tool whose `guard` used another user provider loaded the account
in that provider whose id equalled the caller's user id.

**Now:** such a tool is refused as a configuration error when the application
constructs it. An id in one provider does not name the same person in
another. [Agents](agents.md) says so beside the `guard` attribute.

**Do:** keep agent tools on guards that share the default guard's provider. An
application that keeps administrators in their own table cannot expose
admin-only tools over MCP in 4.5.

### Live components now reject more than 64 browser tools on the server, preventing oversized tool metadata from disabling ordinary clicks.

**Before:** a page could offer any number of browser tools, and oversized tool
metadata disabled ordinary clicks in the browser.

**Now:** constructing a component whose page would offer more than 64 tools
fails with `MetadataErrorKind::TooManyBrowserTools`.

**Do:** nothing, unless a page offers more than 64 tools; split that page.

### Recommendations created before an upgrade now obey the current peer minimum, and changing ranking settings refreshes unchanged users on the next full run.

**Before:** rows a single peer had produced under 4.2 stayed readable after
the peer minimum rose, and an unchanged incremental run kept copying them.

**Now:** reads enforce the current `RECOMMEND_MIN_PEERS` on stored rows, and
the `ranking_digest` entry above describes the refresh.

**Do:** run your migrations, as that entry says. Expect shorter collaborative
lists until the next full run.

### Tool schemas publish integer fields with their bounds and no JSON Schema format.

**Before:** an integer field's schema carried the Rust type's format, such as
`"format": "uint64"`, which clients do not know; Claude Code logged
`unknown format "uint64" ignored` for each one.

**Now:** the schema says `"type": "integer"` with the field's `minimum` and
`maximum` and no `format`. The bounds carry the same information, and the
WebMCP browser schemas follow the same rule. [Agents](agents.md) says so beside
`#[tool_input]`. (4.5.2)

**Do:** run `suprnova generate-types` once after the bump, so the frontend's
`tool-schemas.ts` matches what the server publishes.

## Starter kit edits to copy

These fixes changed files that `suprnova new` writes into a project. Your
project has its own copies, so the upgrade does not change them; make the
edits by hand.

- **Starter kit notes search now stops offering more results at the accessible
  hit cap, even when more notes match the query.** In `src/controllers/notes.rs`,
  and in `src/live/note_list.rs` in the Live kit, the `more` flag compared the
  total hit count with the hits shown, so it offered a page past the accessible
  hit cap. Replace that line with `let more = result.pagination.has_more;`.
- **Stream requests now use the fresh CSRF cookie after sign-in, so Inertia
  navigation no longer leaves them sending a stale token.** In
  `frontend/src/lib/stream/core.ts`, `csrfHeaders` prefers the XSRF cookie,
  which the server refreshes, over the `csrf-token` meta tag, which Inertia
  navigation leaves stale.
- **Stream readers mounted together with the same id now share their text and
  callbacks in all three Inertia kits.** In the same file, `textStream(id)`
  returns one `TextStream` instance per id.
- **Event streams now stay open after a connection error so the browser can
  reconnect and continue receiving messages.** In the same file,
  `openEventStream` leaves the `EventSource` open on an `error` event.
- **Cancelling a text stream from a response callback now leaves its status
  clear, and callbacks cannot overwrite a replacement request's state.** In the
  same file, `TextStream` checks that a request still owns the stream after
  each callback before it writes state.

The four stream fixes touch one file, `frontend/src/lib/stream/core.ts`, which
is the same in the React, Vue and Svelte kits and carries no project-specific
text. The simplest upgrade is to scaffold a throwaway project with the new CLI
and copy that file over yours. Port the four changes by hand only if you
edited the file.

## Fixed entries with no upgrade step

- **Two tools with one name in different crates now fail the build at link
  time.** An application that built and booted before has no such duplicate,
  because the boot check refused it; the change moves the failure to the link
  step, where the duplicate symbol `__suprnova_tool_<name>` names the tool. A
  crate that forbids unsafe code still builds. (4.5.2)
- **The locked dependency set no longer carries the webpki advisories
  RUSTSEC-2026-0381, 0382 and 0383.** Nothing to change in code. Your own
  application's lock file is separate and keeps the versions it resolved when
  you created the project, so the tag fixes nothing there. After the bump,
  run `cargo update -p rustls-webpki -p webpki-roots@1.0.9 -p
  webpki-root-certs@1.0.9 -p ammonia -p quinn-proto -p rustls` (append
  `@<version>` from your lock wherever cargo calls a name ambiguous), then
  `cargo audit` until it reports no vulnerabilities. Locks created from earlier
  tags also hold `ammonia`, `quinn-proto` and `rustls` versions with open
  advisories, which the framework's own lock left behind before 4.5.0. (4.5.1)
- **Database search vector updates and deletions now release their SQL
  connection before calling the vector store, avoiding deadlocks on shared
  pools.** Nothing to change.
- **Search touches now find parent keys even when a child hides or renames them
  in JSON output.** Nothing to change.
- **Retiring a database search generation now removes list tables for
  attributes that have since been removed from the index definition.** Nothing
  to change.
- **Database search now detects MySQL and MariaDB separately for each named
  connection, so generation tables use the correct collation.** Nothing to
  change.
- **Incrementing or decrementing a counter now adds or removes search results
  when the new value crosses a model's indexing threshold.** Nothing to change.
- **A first JSON search without a cookie now returns an impression that a later
  click from the same client can validate.** Nothing to change; the cookies
  entry above says what a client sends back.
- **Retrying a recorded recommendation event now succeeds after its target is
  deleted or its access scope changes.** Nothing to change.
- **Starter explanations now include peers' shared-target events and all
  repeated positive events, so verification detects removed contributors.**
  Nothing to change; run `recommendations:generate` once, as the explanations
  entry above says.
- **Reading a short starter recommendation list no longer loads the whole
  visible catalog, and still fills the list after filtering inaccessible
  targets.** Nothing to change, unless you wrote a search engine; see the
  starter reads entry above.
- **Semantic and hybrid searches now exclude documents missing required quoted
  phrases and refill filtered candidates to preserve result depth.** Nothing to
  change.
- **Replacing an embedding provider with another model of the same dimensions
  now reports a rebuild error instead of silently mixing incompatible
  vectors.** Nothing to change; run `search:reindex` if you changed the model.
- **Workers with searchable models now start with an environment-selected HTTP
  or fastembed provider.** Nothing to change.
- **Fresh starter kits can create the notes search generation with the
  configured provider during bootstrap.** Nothing to change; your copy of
  `src/bootstrap.rs` keeps working, and you may adopt the new `expect`
  message, which names `SEARCH_EMBEDDER` and the search migrations.
- **Qdrant flushes now block delayed writes, including keys absent at the
  start, and deleting an index removes its collections and generation
  metadata.** Nothing to change.
- **Pruning a retired Qdrant generation now removes its collection and metadata
  instead of failing after deletion starts.** Nothing to change.
- **Unchanged Qdrant documents now keep their latest version, so delayed older
  removals cannot delete them.** Nothing to change.
- **A concurrent save can no longer make a loaded search document carry a newer
  version than the row content it contains.** Nothing to change.
- **A borrowed save no longer leaves a searchable model reporting its previous
  search version after the database has saved a new revision.** Nothing to
  change.
- **Graph recommendations now use a stronger indirect route even when a weaker
  direct route reached the item first, and explain the route used.** Nothing to
  change.
- **Editing a document now keeps its recommendation engagement signals while
  rebuilding declared relations, including when a document job is retried.**
  Nothing to change, unless you wrote a graph store; see the
  `DocumentRelationsBefore` entry above.
- **Recommendation explanations no longer reveal which followed accounts
  engaged through edge keys that a client can recompute.** Nothing to change;
  the serialized explanations entry above says what a client no longer sees.
- **Adding a source to a Content Security Policy now changes only the policy
  containing its directive, keeping other policies in the header unchanged.**
  Nothing to change.
- **Live now announces interrupted actions to screen readers in the shipped
  browser bundles.** Nothing to change.
- **A sign-up whose address collides with an existing account only under the
  database's looser comparison answers the same generic response as a
  registered address and creates nothing, instead of a conflict error.**
  Nothing to change, unless your code matches `RegistrationOutcome`; see the
  enum entry above.
- **Schema watchers now report new form request refusals even when the
  generated file stays unchanged, without repeating unchanged refusals on
  every edit.** Nothing to change.
- **Postgres schema dumps now remove entire multiline extension comments even
  when they contain semicolons, preventing invalid snapshots.** Nothing to
  change. Run `schema:dump` again for a clean snapshot.
- **Headless Live tools now report when their growing state no longer fits a
  follow-up request, so agents can restart instead of receiving an unusable
  handle.** Nothing to change.
- **Concurrent fresh database migrations now recover when another migration
  creates the authorization server or agent auth tables first on Postgres.**
  Nothing to change.
- **Concurrent agent assertions now share one provider key-set fetch,
  preventing unknown keys from triggering duplicate requests to the
  provider.** Nothing to change.
- **Live tool actions now accept optional dates and date-times, and date-times
  in lists and maps, without failing before the action runs.** Nothing to
  change.
- **Password resets now cancel password sign-ins already in progress, so an old
  password cannot create a new session after the reset.** Nothing to change.
- **Built-in approval and agent claim forms now refuse agent tool submissions,
  even when the tool has your signed-in session and a valid form token.**
  Nothing to change.
- **On MySQL and MariaDB, accent-insensitive email comparisons could let a
  different mailbox receive a magic link for another account.** Nothing to
  change; the email entry above describes the new rule.
- **On MySQL and MariaDB, an email change that differed only by accents could
  still receive a verification stamp for the old address.** Nothing to change.

## Next

- [Auth Flows](auth-flows.md) documents the password rehash hooks.
- [Recommendations](recommendations.md) documents the graph store and its
  revocations.
- [Search](search.md) documents `search:reindex`, touches and analytics.
- [Upgrading to 4.4](upgrade-4.4.md) covers the preceding release.
