# Changelog

A readable, per-version log of what changed in Suprnova. Each version
section is that version's release record. A version is released when its
version commit and matching `v<version>` tag are pushed atomically. Newest first.

## Unreleased

### Fixed

- **`generate-types` resolves nested prop structs without derives.** 0.7.1's
  generator degraded any prop field whose type didn't derive
  `InertiaProps`/`Data` to `unknown` — so re-running the generator (or the
  `suprnova serve` watcher) over a project with a committed types file
  replaced real interfaces like `Array<AdminArticleRow>` with `unknown` and
  broke type-checking across the app. Plain structs defined anywhere in
  `src/` now resolve to their real interfaces, transitively from the prop
  roots; `unknown` (with a warning) is reserved for types the project
  genuinely doesn't define — external crate types, enums, tuple structs.

### Changed

- **`routes.ts` generation is opt-in.** `generate-types` no longer drops
  `frontend/src/types/routes.ts` into every project unasked; pass
  `--routes` to generate it.

- **Frontend starter dependencies refreshed.** New scaffolds from
  `suprnova new` now pin current versions: Vite ^8.1.5, Tailwind CSS ^4.3.3,
  Svelte ^5.56.8 (vite-plugin-svelte ^7.2.0, svelte-check ^4.7.4),
  React ^19.2.8 (plugin-react ^6.0.4), Vue ^3.5.40 (plugin-vue ^6.0.8,
  vue-tsc ^3.3.8), and `@types/node` ^24 (the Node 24 LTS types line).
  TypeScript stays at ^6.0.3 deliberately: it is the latest 6.x, and
  svelte-check's peer range (`^5 || ^6`) does not yet admit TypeScript 7.
  All three starters were verified end to end (`npm install` +
  `npm run build`) against the refreshed set.

## 0.7.1 — 2026-07-27

A defect-fix pass over 0.7.0's queue routing, from a full post-release review.

### Fixed

- **Chained jobs no longer lose their declared queue.** `ChainLink` captured a
  job's `max_tries`, `timeout`, and `backoff` at chain-build time but not its
  `Job::queue()`, so a job that landed on its declared queue when pushed
  directly landed on `default` when dispatched as part of a chain — the "job"
  tier of the route → job → default resolution order silently vanished for
  chains. The declared queue is now captured on the link and resolved exactly
  like a direct push. Chain payloads written before this release decode
  unchanged (`serde(default)`), and a link with no declared queue serializes
  byte-identically to what 0.7.0 wrote.
- **Failed-job records carry the queue the job died on.** The worker's
  dead-letter path hardcoded `queue = "default"` into every `FailedJob`
  record, so failures of a routed job were invisible to an operator filtering
  the failed store by the pool that owns them. The record now carries the
  envelope's queue (`default` for unrouted jobs).
- **The 0.7.0 upgrade note understated the `jobs` migration.** It read
  "unfiltered workers are unaffected and need no migration", but
  `DatabaseQueueDriver::push` names the `queue` column in its `INSERT`
  whether or not the job is routed — a 0.7.0 binary against an un-migrated
  table fails **every push**, filtered or not. The 0.7.0 section below and
  `manual/queues.md` are corrected: on the database driver the `ALTER TABLE`
  is required for every deployment, and it must run before binaries roll
  (older binaries list their columns explicitly, so migrating first is safe).

- **README no longer advertises a `#[job]` macro.** No such macro exists —
  jobs implement the `Job` trait. The queues row now describes the real
  surface, including 0.7.0's queue routing.

### Changed

- **The release path now bumps README version references.**
  `bump-workspace-version.py` rewrites the README's pinned install tag, the
  distribution-model example, and the MSRV line atomically with the
  manifests, and a reworded README that stops matching a pattern fails the
  release loudly. The README had advertised v0.6.0 since v0.7.0 shipped
  because nothing in the release path touched it.
- **Connection routing is documented as name-resolution only.**
  `Job::connection()` and the connection field of `Queue::route` resolve the
  connection *name* carried on the `JobQueueing` / `JobQueued` lifecycle
  events; a single process-global driver still receives every push, so they
  do not select a different driver. The rustdoc and `manual/queues.md`
  previously implied driver selection that does not exist. The queue
  dimension is unaffected — it is honored end to end. Per-connection drivers
  remain future work.
- `ChainLink` gained a public `queue: Option<String>` field, which breaks
  struct-literal construction of chain links. Links built through
  `ChainLink::from_job` — the normal path — are unaffected.

### Upgrading

Coming from ≤ 0.6.x on the database queue driver, apply the 0.7.0 migration
below **before** rolling binaries; it is required for every deployment on
that driver, not just ones using `--queue`. 0.7.1 itself needs no migration.

## 0.7.0 — 2026-07-26

### Security

- **Upgraded `ammonia` to 4.1.4 (RUSTSEC-2026-0213).** Versions through 4.1.3
  allow XSS via SVG `animate` and `set` animation tags. `ammonia` is the
  sanitizer at the end of Suprnova's markdown pipeline
  (`comrak` → `syntect` → `ammonia`), so any app rendering user-supplied
  Markdown through `content` was exposed. The advisory was published
  2026-07-21 — after v0.6.5 shipped — so **every release up to and including
  v0.6.5 is affected**. Upgrading the framework is the fix; no application
  code changes are required.

### Added

- **Queue routing.** Jobs can be dispatched to a specific queue and connection,
  and workers can be dedicated to specific queues — the Laravel 13
  `Queue::route(...)` surface, typed. A job states its own home with
  `Job::queue()` / `Job::connection()`; an operator overrides it centrally with
  `Queue::route::<SendInvoice>(Some("redis"), Some("billing"))` in
  `bootstrap::register()`, without editing the job. Resolution is route, then
  job, then global default, and a `None` field in a route defers rather than
  clearing. `queue:work --queue=billing,default` drains only those queues.
  Unrouted jobs belong to `default`, so they are never stranded. Chained jobs
  resolve routes by name, since a chain link stores its job erased.
- **`QueueDriver::pop_from`.** Filtering pop, with a default implementation that
  **rejects** a filter it cannot honor rather than silently draining every
  queue — a worker told to drain `billing` that quietly drains everything is
  indistinguishable from a working deployment until the wrong pool eats the
  wrong jobs. The memory and database drivers filter natively. Custom drivers
  keep compiling and inherit the loud default.
- **Documented the `jobs` table schema.** `manual/queues.md` now carries the DDL
  `DatabaseQueueDriver` actually expects, which was previously only discoverable
  by reading the driver's SQL.
- **Documented Inertia's `serverHead` option.** Server-driven `<head>` elements
  (Inertia 3.5.0) need no framework support: the client reads them from an
  ordinary prop, so any handler can already supply them. See
  `manual/frontend-inertia-responses.md`.

### Changed

- `Envelope` gained a `queue: Option<String>` field. It is `serde(default)` and
  skipped when absent, so an unrouted envelope serializes byte-identically to
  what previous versions wrote — the frozen wire-format test passes unchanged,
  there is no `schema_version` bump, and mixed-version fleets interoperate
  during a rolling upgrade.
- `WorkerConfig` gained a `queues: Vec<String>` field (empty = drain everything,
  the previous behaviour).
- Removed `ROADMAP.md`. Its design principles live in `manual/introduction.md`,
  the working agreement in `manual/contributions.md`, and the deployment and
  scale-out material in `manual/deployment.md`; the shipped/planned checklists
  had gone stale. `README.md`'s pointer to it for "the relationship to upstream"
  was already dangling — that attribution lives in `LICENSE`.
- Scaffold frontends now pin `@inertiajs/{svelte,react,vue3}` at `^3.6.1`
  (from `^3.4.0`). The 3.4.0 → 3.6.1 range is client-side only — audited against
  the upstream changelog and the `Page` contract in `packages/core/src/types.ts`,
  every `X-Inertia-*` header the 3.6.1 client sends was already handled.
- `scripts/release.sh` now publishes the GitHub release itself, with notes taken
  from the version's `CHANGELOG.md` section. Previously this was a manual
  "next step" that got skipped, which is why v0.5.10 and v0.6.1–v0.6.3 are
  tag-only and the Releases page sat on a stale version. Preflight runs before
  the gate so a missing `gh` or changelog section fails in seconds, and
  publishing is skipped automatically unless `origin` is GitHub.

### Upgrading

Existing `jobs` tables on the database queue driver **must** add the new
column — `push` names it in its `INSERT` whether or not the job is routed, so
an un-migrated table fails every push. Migrate first, then roll binaries
(older binaries list their columns explicitly and ignore the new one, so that
order is safe):

```sql
ALTER TABLE jobs ADD COLUMN queue TEXT NULL;
CREATE INDEX idx_jobs_queue ON jobs(queue);
```

*(Corrected in 0.7.1 — this note originally claimed unfiltered deployments
needed no migration.)*

## 0.6.5 — 2026-07-21

### Added

- **Hosted one-off Checkout in the Stripe adapter.** `Checkout::start_session`
  with `SessionMode::OneOff` and non-empty `price_refs` now creates a hosted
  Checkout Session (`mode=payment`, one line item per price ref,
  `allow_promotion_codes=true`) and returns
  `SessionPayload::StripeCheckoutRedirect`. The `amount_hint`-only Elements
  path is unchanged; the two shapes are picked per request.
- **Stripe Managed Payments (merchant-of-record) support.**
  `StripeProvider::with_managed_payments(true)` — or
  `STRIPE_MANAGED_PAYMENTS=true` in `from_env()` — sends
  `managed_payments[enabled]=true` on hosted one-off session creation. Off by
  default; the field is omitted entirely so non-enrolled accounts are
  unaffected.
- **`Checkout::session_status`.** New trait method (default:
  `PaymentError::NotSupported`) reporting a session's provider-side state as
  the new neutral `CheckoutSessionState` (`Open` /
  `Complete { paid, payment_ref, amount_total }` / `Expired`). The Stripe impl
  maps `GET /v1/checkout/sessions/{id}`; `payment_ref` carries the session's
  PaymentIntent id for mirror-table correlation. This is the server-side
  verification primitive for redirect return pages and reconciliation sweeps.
- **`Promotions` capability trait.** `create_promotion_code` mints a
  customer-restricted, optionally expiring, redemption-capped code off a
  pre-created coupon. Queried via the new
  `PaymentProvider::as_promotions()` (default `None`). Implemented for Stripe
  (`POST /v1/promotion_codes`) and the mock.
- **`MockPaymentProvider` upgrades for the above.** Records every
  `start_session` request (`recorded_sessions()`), scripts `session_status`
  per session id (`script_session_status()` — unscripted known sessions
  report `Open`, unknown ids `NotFound`), and implements `Promotions` with
  recorded requests (`recorded_promotion_requests()`).

## 0.6.4 — 2026-07-17

### Fixed

- **Eloquent aggregates decode consistently across database backends.** Generated
  `count`, `sum`, `avg`, `min`, and `max` expressions now use one stable internal
  result alias. PostgreSQL no longer returns false zeroes or `None` because its
  driver labels aggregate columns differently from SQLite, and missing-column or
  incompatible-type errors now propagate instead of being silently defaulted.
- **Mass deletes cannot use caller-supplied table expressions.** Executable
  delete SQL always derives its target from the model's validated static
  `M::TABLE`. The legacy public renderer argument remains source-compatible but
  cannot redirect or inject the delete target.

## 0.6.3 — 2026-07-15

### Added

- **Typed raw reads can stay on a transaction's pinned connection.**
  `Transaction::backend()` exposes the active backend and
  `Transaction::query_all(Statement)` executes typed aggregate or custom SQL
  through the transaction while preserving `QueryExecuted` instrumentation.
  Applications no longer need a pool-level query or private executor access
  when a lock-scoped decision depends on computed result columns.

## 0.6.2 — 2026-07-15

### Fixed

- **Bound raw predicates are backend-neutral.** Eloquent `filter_raw` and
  `where_raw` now accept portable `?` bind markers on every database backend;
  PostgreSQL rendering rebases them to monotonic `$N` positions across prior
  predicates, relationship subqueries, HAVING clauses, and UNION arms. Existing
  numbered PostgreSQL fragments are normalized by their local marker order,
  while mixed styles and bind-count mismatches fail validation before I/O.
  The SQL-aware scanner preserves question marks inside quoted strings,
  identifiers, comments, and dollar-quoted bodies; `??` emits a literal
  question-mark operator in a bound raw fragment.

## 0.6.1 — 2026-07-15

### Added

- **Observable supervised session cleanup.** `SessionMiddleware::install`
  uses the configurable `SESSION_GC_INTERVAL` cadence (one hour by default),
  while `session_gc_metrics()` exposes process-local run, success, failure,
  removed-row, and last-result timestamps for protected operations surfaces.
- **Bounded sliding-session touches.** `SESSION_TOUCH_INTERVAL` controls the
  minimum activity-write cadence (five minutes by default) and is capped at
  half the session lifetime so active sessions cannot expire between touches.

### Fixed

- **State-free requests no longer create durable sessions.** Requests without
  a valid session cookie perform no session-store read or write and receive no
  session cookie unless handling creates state. Existing clean sessions avoid
  unconditional upserts and cookie churn, legacy cookies migrate on their next
  request, and cookies whose backing rows have expired are cleared without
  recreating empty sessions.

## 0.6.0 — 2026-07-10

### Added

- **Opt-in framework subsystems with backward-compatible defaults.** Filesystem
  storage, SQLite/Postgres/MySQL database drivers, the MariaDB vector driver,
  and Web Push now have explicit Cargo features. Existing default builds retain
  all of these capabilities, while `default-features = false` consumers can
  select zero drivers or only the storage/database/vector/push surface they use.
  The executable feature matrix verifies zero-driver, individual-driver,
  Nation X minimal, default, and all-feature profiles.
- **Raw P-256 VAPID private-key import.** `VapidKey::from_bytes` accepts a
  validated 32-byte big-endian P-256 scalar alongside the existing PKCS#8 PEM
  import/export path.

### Changed

- **VAPID JWTs are signed directly with P-256.** Web Push now serializes the
  RFC 8292 ES256 header/claims and signs them with `p256`, removing the generic
  JWT dependency while preserving generated keys, PEM round trips, public-key
  encoding, and the 24-hour lifetime bound.
- **Security dependency refresh.** Updated vulnerable framework dependencies,
  including bcrypt and ammonia, and narrowed Comrak's enabled features while
  retaining syntax highlighting.
- **Rust 1.91.1 is the release MSRV.** Every workspace package declares the
  same `rust-version`, generated Dockerfiles pin the matching builder image,
  and the full release gate compiles the supported filesystem profile with the
  exact Rust 1.91.1 toolchain.
- **OpenDAL 0.58 security pin.** The filesystem feature pins
  `entrepeneur4lyf/opendal` commit
  `88717391eb72c9839d3f8e79fccad9f22fc3a1b4`, a minimal fork based exactly on
  official Apache OpenDAL commit
  `ae99a3b016e354a1b2bb2baf0c70f9f9e134970a`. The fork changes only the
  Reqsign declarations used by OpenDAL core plus S3, GCS, and Azure Blob so
  downstream consumers resolve official Apache Reqsign commit
  `b49cd2996b9d2d9944e84481f8835ff55b188b97` and `quick-xml` 0.41.0. A fork is
  required because a dependency repository's root Cargo patches do not
  propagate to consumers; the published graph could otherwise restore
  vulnerable `quick-xml` 0.38/0.40.

### Fixed

- **Atomic release version metadata.** The release bump now updates
  `workspace.package.version` and every versioned internal path dependency in
  one validated operation, stages every affected manifest, and proves a
  temporary `0.6.0` workspace with `cargo check --workspace` before release.
  Release versions are validated as strict SemVer 2.0, including the numeric
  prerelease leading-zero rule. Version-agnostic disposable bare-remote smokes
  derive a later patch release from both the current source and an already
  `0.6.0` source, reject staged/unstaged/untracked release trees before the
  gate, prove atomic commit/tag publication rolls both refs back when a tag is
  rejected, and prove the normal release sequence without touching the real
  remote. Release versions must increase by SemVer precedence, including
  prerelease transitions. Smoke build artifacts always stay inside their
  temporary workspace, ignoring any caller `CARGO_TARGET_DIR`.
- **Rustdoc covers every supported feature boundary.** The OAuth module links
  to public `OAuthAuth::complete`, and the executable matrix builds zero-driver,
  default, and all-feature rustdoc with no dependencies.
- **Filesystem stream validation is session-scoped.** Local filesystem writers,
  listers, and copiers resolve and confine their paths once before first I/O
  instead of once per chunk/item, while activated close/abort operations always
  reach the backend for cleanup. Existing traversal and symlink confinement
  remain enforced for a trusted filesystem; canonicalize-then-open checks do
  not eliminate races against a principal concurrently mutating the tree.

### Security

- **The release gate fails closed.** `release.sh` delegates to the canonical
  full gate before editing manifests or creating commits/tags; that gate always
  runs `cargo audit`, treats a missing `cargo-audit` binary as an error, and
  stops on any audit failure. It also builds and audits an isolated downstream
  filesystem consumer, asserting exact OpenDAL/Reqsign source revisions and no
  `quick-xml` below 0.41. No new advisory ignores were added.

## 0.5.10 — 2026-07-03

### Fixed

- **`generate-types` no longer drops self-referencing structs.** A struct with a
  field that references its own type (a tree node with `children: Vec<Self>`,
  e.g. a threaded-comment view) created a self-edge in the type-dependency
  graph, pinning its in-degree above zero so Kahn's topological sort never
  emitted it — leaving every interface that referenced it with a dangling type
  name that failed `svelte-check`/`tsc`. Self-edges are now stripped before
  sorting, and any structs trapped in a reference cycle (mutual recursion) are
  emitted in arbitrary order rather than dropped, since TS interfaces may
  reference one another regardless of declaration order.

## 0.5.9 — 2026-07-01

### Added

- **`MAIL_FROM_NAME` — optional display name on auth-flow emails.** The
  email-verification, password-reset, and password-changed mailables now render
  their `From` header as `"Name <address>"` when `MAIL_FROM_NAME` is set (read
  at send time so it survives the queue's serde round-trip). `MAIL_FROM` stays a
  bare address; leaving `MAIL_FROM_NAME` unset or blank keeps the previous
  bare-address behavior. No change to any call site — the mailables read the env
  var themselves.

## 0.5.8 — 2026-06-30

### Fixed

- **`generate-types` route helpers are always valid TypeScript.** When several
  routes in a module share one handler (e.g. a `static_files::serve` whitelist
  mapping many favicon/asset URLs), the first kept the handler name and the rest
  got a key derived from the route path — but the path was only partly
  sanitized (`/ { } -` → `_`), so a file extension leaked a `.` into the key:
  `favicon_16x16.png: (...) => ...`. That is member access, not a property name,
  so `tsc`/`svelte-check` rejected the generated `routes.ts`. Derived keys are
  now sanitized to legal identifiers — every non-alphanumeric character becomes
  `_` and a leading digit is prefixed — so `favicon-16x16.png` → `favicon_16x16_png`
  and `2fa.json` → `_2fa_json`. Unique handler names are untouched.

## 0.5.7 — 2026-06-30

### Fixed

- **`generate-types` no longer emits dangling type references.** A prop field
  whose type is a struct that doesn't derive `InertiaProps`/`Data` (or an
  external type the generator can't see) was emitted as a bare identifier — e.g.
  `user: UserInfo` — producing TypeScript that fails `tsc`/`svelte-check`
  because that interface is never written. Such references now degrade to
  `unknown` (`user: unknown`; `Vec<T>` → `Array<unknown>`; `Option<T>` →
  `unknown | null`), so generated output always type-checks, and
  `generate-types` prints a warning naming the unresolved type and the field
  that references it, with the fix (derive `InertiaProps`/`Data` on it).
  Generic parameters and resolved nested InertiaProps/Data types are
  unaffected.

## 0.5.6 — 2026-06-29

### Changed

- **Sign in with Apple: RS256 JWKS verification.** Bump `suprnova-apple-rs` to
  v0.3.1 — Apple ID tokens are now verified against Apple's published JWKS
  (RS256) instead of being trusted structurally.

## 0.5.5 — 2026-06-28

### Added

- **`MagicLink` token purpose.** New `MagicLink` variant on the auth-flow
  `TokenPurpose` enum, for passwordless magic-link sign-in tokens.

## 0.5.4 — 2026-06-28

### Changed

- **Composable OAuth completion.** Split the generic OAuth completion into
  `verify_oauth_identity` (verify + resolve the identity) and a thin `complete`,
  so apps can verify an OAuth identity without triggering the full
  session-completion side effects.

## 0.5.3 — 2026-06-28

### Fixed

- **Correct workspace version metadata.** v0.5.2 was tagged and pushed before
  its `Cargo.toml` version bump was staged, so the pushed v0.5.2 tag still reads
  `version = "0.5.1"`. v0.5.3 re-cuts the release with the correct workspace
  version — no code change (the v0.5.2 OAuth split is unaffected).

## 0.5.2 — 2026-06-28

### Changed

- **Composable Apple completion.** Split Apple Sign-In completion into
  `verify_apple_identity` + a thin `complete_apple`, mirroring the generic OAuth
  split. (Note: the pushed v0.5.2 tag carries a stale `0.5.1` version field —
  fixed in v0.5.3.)

## 0.5.1 — 2026-06-28

### Changed

- **Renamed Apple crate.** Repoint the Apple dependency to the renamed
  `suprnova-apple-rs` repository.

## 0.5.0 — 2026-06-28

### Added

- **Sign in with Apple.** OAuth token exchange + ID-token verification + user
  upsert for Apple; Apple well-known endpoints and the `form_post` response
  mode; Apple-specific fields on `OAuthProviderConfig`; `AppleKeyPair`
  re-exported so apps configure Apple Sign-In without a direct `apple`
  dependency.

### Fixed

- Omit PKCE parameters from the Apple authorize URL (Apple rejects the request
  when they are present).

### Dependencies

- Consume the `torii` magic-auth fix; add `apple-rs` v0.3.0.

## 0.4.1 — 2026-06-26

### Performance

- Pre-size `MiddlewareChain` to eliminate per-request `Vec` reallocations.

### Fixed

- Make the maintenance down-file path collision-proof under parallel test runs.

### Docs

- Compile-check the framework's doc examples (`ignore` → `no_run`); reconcile
  the distribution notes with the tagged GitHub Releases; ignore the whole
  `docs/` tree.

## 0.4.0 — 2026-06-22

### Changed

- **Distribution is git-tracked; you don't pin to tags.** Scaffolded apps
  depend on `suprnova = { git = "…/suprnova.git" }` and track the default
  branch; pull updates with `cargo update -p suprnova`. Versions are published
  as tagged GitHub Releases (`v0.4.0`, …) for the changelog, but `Cargo.lock`
  already pins the exact resolved commit — so builds stay reproducible without
  hand-pinning a `tag` or `rev`. The installation docs no longer present
  commit-pinning as the update path.

## 0.3.0 — 2026-06-21

### Added

- **Query instrumentation for Eloquent reads** — `Builder::get`, `Model::find`,
  `find_many`, and `all` now emit `QueryExecuted`, so model SELECTs and
  eager-load queries surface in `DB::listen` and the in-memory query log
  alongside writes and raw queries. Adds the instrumented
  `ExecutorChoice::statement_all` read terminal.
- **Resource-route authorization** — `ResourceRoutes::authorize_resource::<U, R>()`
  attaches the conventional ability check to every generated resource route as
  per-route middleware (Laravel `authorizeResource` parity). The action→ability
  map is `index`/`show` → `view`, `create`/`store` → `create`,
  `edit`/`update` → `update`, `destroy` → `delete`. One call gates the whole
  seven-action surface instead of relying on every controller body to remember
  a `Gate::authorize`.
- **Atomic rate-limit hit** — `RateLimiter::hit_and_check(key, max, decay)`
  increments a fixed window and tests it in a single round-trip, returning
  whether the bucket is now over its limit (`i64::MAX` means unlimited).
- **Constant-time comparison helper** — `constant_time_eq(a, b)` (subtle-backed)
  for webhook signature verification; `WebhookHandler::verify` docs now mandate
  constant-time digest comparison.
- **Inertia client to 3.4.0** — the Svelte/React/Vue scaffolds now pin
  `@inertiajs/{svelte,react,vue3}` at `^3.4.0` (from `3.1.1`), picking up
  `router.poll` modes, dynamic `usePoll`, `Inertia.once`, the InfiniteScroll
  cancel fix, and awaited Form `onSuccess`. The server already emits the full
  3.4.0 page-object and header surface (once-props, the prepend/deep-merge
  scroll family, `matchPropsOn`, rescued/shared props), so this is a
  client-currency bump with no protocol change.
- **Optional connection cap** — `SERVER_MAX_CONNECTIONS` (and the programmatic
  `Server::max_connections(n)`) bounds concurrently active connections with a
  semaphore on the accept loop, applying back-pressure at the TCP level. Unset —
  or `0` — leaves connections unbounded (the default, unchanged). A backstop to
  pair with a reverse proxy and `LimitNOFILE`, not a replacement for upstream
  rate limiting.
- **Opt out of redirect-following** — `RequestBuilder::no_redirects()` routes a
  request through a non-following HTTP client so a `3xx` is returned as-is
  instead of chased. Use it when the request URL is influenced by untrusted
  input, to close a redirect-based SSRF vector (a hostile endpoint redirecting
  toward an internal or cloud-metadata host). The default client still follows
  redirects, matching general-client convention.

### Security

- **Resource routes** fail closed on the authorization registry's type-erased
  downcast instead of panicking, and `authorize_resource` denials /
  unauthenticated requests are refused before the handler runs.
- **Rate limiter** closes a fixed-window check-then-hit race by incrementing and
  comparing atomically (`hit_and_check`).
- **Queue `RateLimited` middleware** now admits jobs through that atomic
  `hit_and_check` instead of a separate `too_many_attempts` + `hit` pair, so
  concurrent workers can no longer all pass the budget check before any of them
  increments and over-admit past `max_attempts`.
- **Upload validators** (`mimetypes` / `mime`) content-sniff the uploaded bytes
  instead of trusting the client-supplied `Content-Type`.
- **Filesystem path guard** canonicalizes paths to catch symlink traversal out
  of the storage root, beyond the prior lexical `../` / absolute / UNC checks.
- **Auth** closes a passwordless-login timing oracle — a matched-but-passwordless
  account given a password now runs a fixed-cost verify, across both the Eloquent
  and database user providers — and `dummy_verify` drives the configured hasher so
  the unmatched-user path is constant-time.
- **Eloquent** validates column identifiers on the `pluck` / `value` /
  `pluck_keyed` / `sole_value` and `sum` / `avg` / `min` / `max` projection
  paths.
- **Payments** — the mock provider's verifier fails closed outside a development
  environment, and webhook source IPs resolve through `TrustedProxiesConfig`
  (`req.ip()`) rather than a raw `X-Forwarded-For` header.
- **Filesystem path guard** now walks to the nearest *existing* ancestor when a
  write target doesn't exist yet, closing a symlink escape where a planted
  intermediate symlink with a missing immediate parent slipped past the guard.
- **`DB::init_with`** validates the environment before connecting (matching
  `DB::init`), so the dev SQLite fallback can no longer boot silently in
  production through that entry point.
- **Static-file serving** rejects dotfiles (`.env`, `.git/config`, `.htpasswd`,
  any leading-`.` segment), not just `.`/`..` traversal.
- **Payment webhooks** serialize concurrent retries of the same unprocessed
  event with a `FOR UPDATE` lock + re-check, and treat mirror-table unique
  violations as benign already-applied; `payments_subscription_items` gains a
  `UNIQUE(subscription_id, provider_item_id)`.
- **RBAC** defaults the model discriminator to the fully-qualified type name, so
  two authenticatable types sharing a leaf name can no longer inherit each
  other's roles/permissions.
- **`invalidate_session()`** rotates the session id (not just flushes), closing a
  session-fixation gap; the queue `WithoutOverlapping` middleware releases its
  cache lock even when the job panics.
- **Mail providers** cap error-response body reads (8 KiB), matching the
  web-push client, so a hostile endpoint can't drive sender memory.
- **Web push** disables HTTP redirect-following on the default client, so an
  attacker-influenced push endpoint can no longer `3xx`-redirect a notification
  POST toward an internal or cloud-metadata host (SSRF). A redirect now surfaces
  as a rejected push rather than a silently followed request.
- **Stripe adapter** `Debug` redacts the webhook signing secret *and* prints a
  placeholder for the `stripe::Client` (which carries the API secret key in its
  auth header), so neither secret can reach logs through a `{:?}` of
  `StripeProvider`, regardless of the upstream client's own `Debug`.
- **Stripe adapter** `from_env` rejects present-but-blank credentials, failing
  closed instead of constructing a client with an empty (and therefore forgeable)
  webhook HMAC secret.
- **OAuth email verification** fails closed for unrecognised providers: a
  userinfo payload carrying an `email` but no `email_verified` flag is no longer
  treated as verified. An unknown provider must now assert `email_verified: true`
  or expose a verified-emails endpoint, closing an account-link/takeover vector
  for apps that key accounts on email. Google (explicit-`true`-only) and GitHub
  (verified-by-the-`/user`-contract) are unchanged.

### Fixed

- **Nested eager loading** (`with(["posts.comments"])`) is now a constant number
  of queries — the tail segment loads in one batched IN query across all
  parents instead of one query per parent (N+1).
- **`where_has`/`where_doesnt_have`** qualify closure columns with the target
  table, so a column present on both pivot and target no longer produces an
  ambiguous-column error on many-to-many relations.
- **Soft-delete `delete`/`force_delete`/`touch` and factory `persist`** honor a
  model's `#[model(connection = "…")]` routing (matching `restore` and the
  other write paths) instead of falling back to the primary pool.
- **JSON:API `Maybe::Missing`** uses a non-collidable wire sentinel, so user
  data shaped like `{"__missing__": true}` is no longer silently stripped.
- **Queued notifications** honor `should_send` (per-channel veto) and
  `after_sending`, re-checked on the worker — previously only the synchronous
  path did.
- **Released jobs** push the retry copy before acking the original, so a transient
  driver push error no longer drops the job.
- **Paddle adjustment (refund) webhooks** key the mirror update off the referenced
  transaction id and read amounts from `data.totals`, instead of inserting a
  zero-amount row under the adjustment id.
- **SQLite URLs** carrying a query string (`sqlite://db.sqlite?mode=rwc`) build a
  valid single-query connection URL and a clean on-disk filename.
- **HTTP** clamps `Accept` `q`-values to `[0,1]` and enforces a `FormRequest`'s
  `max_body_bytes` even when the body was pre-buffered; **WebSocket** config
  rejects `max_missed_pings < 2` (1 closed every connection on its first ping).
- **Cron** day-of-month and day-of-week use OR semantics when both are restricted
  (Vixie/POSIX parity); Markdown `plain_text`/excerpts preserve intentional
  spaced punctuation; `CachedEvaluator` bounds its cache growth;
  `SupervisorRegistry::start_all` no longer double-spawns on a second call; the
  test container recovers in place from a poisoned lock.
- **Supervisor restart backoff** resets to the 100 ms floor after a run that
  stays up at least the 60 s cap, so a daemon that ran healthily for a long
  stretch and then exits restarts promptly instead of inheriting backoff that
  climbed during an earlier failure burst. A crash loop whose runs never reach
  the threshold still ramps to the cap, so the reset never masks a flapping
  supervisor.
- Corrected stale docs on `filter_op` (operators are allowlist-validated), signed
  URLs (not byte-compatible with Laravel's default absolute signatures),
  `UniqueIdKind::is_valid` (a caller helper, not auto-wired into `find`), and the
  identifier length cap (128, not 64).

### Documentation

- Documented resource-route authorization (`authorize_resource`) in the routing
  and authorization chapters, and the atomic `hit_and_check` counter in the
  rate-limiting chapter.

## 0.2.0 — 2026-06-21

Adds role-based access control, a Markdown content / docs-rendering pipeline, and
native static-file serving.

### Added

- **Tier-2 RBAC** — `HasRoles` trait; roles + permissions with a
  `role_has_permissions` join; `PermissionMiddleware` / `RoleMiddleware` (both
  fail-closed / default-deny); the `CreateRbacTables` migration; and
  `create_role` / `create_permission` / `give_permission_to_role` helpers.
- **Content rendering** — Markdown rendering and a docs-build pipeline:
  `MarkdownRenderer`, `build_docs`, `DocsCatalog` / `DocsChapter`, heading
  extraction and `slugify_heading`. Rendered HTML is sanitized
  (comrak + syntect + ammonia).
- **Native static-file serving** — `StaticFiles::public()` fallback handler for
  serving a `public/` directory at the web root, replacing hand-rolled per-asset
  whitelist controllers in apps.

### Fixed

- Freshly generated apps inherit a framework-level `time = 0.3.47` compatibility
  pin, avoiding Rust 1.96 coherence conflicts from `time 0.3.48` in fresh
  scaffold dependency resolutions.

### Documentation

- Documented the two shipped starter kits — **Nebula** (Breeze-tier auth) and
  **Pulsar** (product site + community) — across the manual, README, and roadmap;
  restructured the roadmap around the shipped surface; and reconciled version
  references throughout the docs.

## 0.1.0 — 2026-06-10

The initial Suprnova release. Suprnova is a Laravel-inspired web
framework for Rust, forked from Kit and taken in its own direction.
Today's parity target is Laravel 13.x.

This release uses the git distribution model: framework consumers depend
on `suprnova = { git = "https://github.com/entrepeneur4lyf/suprnova.git" }`,
and the CLI installs with `cargo install --git`.

### Added

#### HTTP, routing, and middleware

- `Router` with route groups, prefixes, parameter constraints, named routes
- Compile-time-validated route registration via the `routes!` macro
- Resource routing (`Router::resource`) producing the seven standard routes
- Signed URLs (`url::signed_route` / `url::temporary_signed_route` free
  functions, plus `Redirect::signed_route` / `Redirect::temporary_signed_route`)
- Redirect helpers — `Redirect::to`, `Redirect::back`, `Redirect::route`,
  `Redirect::with_input`, `Redirect::with_errors`, `with_flash`
- Middleware trait with global, group, and per-route layers
- Built-in middleware — CORS, CSRF, session, request timeout,
  request ID, throttle / login throttle, signed-URL verify,
  authenticated, email-verified, brute-force
- Abort helpers (`abort`, `abort_unless`, `abort_if`)
- `suprnova::handle_request(...)` — public adapter to serve a single
  hyper request against a router + middleware chain

#### Inertia.js frontend bridge

- `#[derive(InertiaProps)]` with TypeScript type emission
- `inertia_response!` macro with compile-time component validation
- Three first-class starter frontends — **Svelte 5** (runes-on),
  **React 19**, **Vue 3.5** — all on Inertia 3.1.1 + Vite 8 + Tailwind v4
- Partial reloads (`only` / `except`), deferred props, persistent
  layout, encrypted history, scroll preservation
- `Inertia::paginate(component, key, paginator)` for paginator → Inertia
  prop wiring

#### Eloquent-style ORM (over SeaORM)

- `#[suprnova::model]` attribute macro that emits a SeaORM entity and
  the user-facing Eloquent struct in one shot
- Full `Model` trait — `create`, `find`, `find_or_fail`, `find_many`,
  `all`, `query`, `save`, `update`, `delete`, `force_delete`, `refresh`,
  `fresh`, `replicate`, `replicate_into`, `increment`/`decrement`,
  `destroy`, `is`/`is_not`, `to_array`/`to_json`
- Fillable / guarded mass-assignment with `Attrs` envelope
- 22 attribute casts — booleans, integers, floats, dates, enums,
  hashed, encrypted, JSON, collections, money, datetime with timezone
- Accessors / mutators via `#[suprnova::model]`
- Auto-timestamps (`created_at`, `updated_at`)
- Soft deletes (`deleted_at`) with `force_delete`, `restore`, `trashed`,
  `only_trashed`, `with_trashed`
- Eleven relation kinds — `HasOne`, `HasMany`, `BelongsTo`,
  `BelongsToMany`, `HasOneThrough`, `HasManyThrough`, `MorphOne`,
  `MorphMany`, `MorphTo`, `MorphToMany`, `MorphedByMany`
- Per-family morph enums + morph registry with `APP_KEY_PREVIOUS` rotation
- Eager loading via `.with(...)`, `.with_count(...)`, `.load_missing(...)`
- Correlated EXISTS engine for `has` / `where_has`
- Sixteen lifecycle events (retrieving, retrieved, creating, created,
  updating, updated, saving, saved, deleting, deleted, restoring,
  restored, force-deleting, force-deleted, replicating, trashed)
- `Observer<M>` trait with per-method auto-registration via inventory
- Local scopes via `#[scopes(M)]`, global scopes via `GlobalScope`
- `Collection<M>` Laravel surface — `pluck`, `key_by`, `group_by`,
  `where_in`, `first_where`, `contains_where`, `partition`, etc.
- Three paginators — `paginate` (length-aware), `simple_paginate`,
  `cursor_paginate` — all serializing to Laravel-shape JSON
- `chunk` / `lazy` / `cursor` for bulk-row iteration without OOM
- `lock_for_update` / `shared_lock` row-level locking
- `DB::table(...)` query builder with `DynamicRow` for ad-hoc queries
- `DB::transaction(...)` with savepoints, retry-on-deadlock,
  multi-connection read/write split
- `DB::listen(...)` + `QueryExecuted` / `TransactionBegan` /
  `TransactionCommitted` / `TransactionRolledBack` events
- `Prunable` trait + `model:prune` console command
- `dump` / `dd` query-helper methods
- `#[model(unique_id="...")]` for UUID / ULID primary keys

#### Auth

- `Authenticatable` trait + `EloquentUserProvider<M>`
- `Auth::attempt`, `Auth::login`, `Auth::user`, `Auth::user_or_fail`,
  `Auth::user_as<T>`, `Auth::logout`, `Auth::check`
- Multiple named guards (web session, API token)
- Email verification flow — `EmailVerification`,
  `EnsureEmailVerifiedMiddleware`, signed verification URLs,
  `EmailVerificationMail`
- Password reset flow — `PasswordReset`, throttled tokens,
  `PasswordChangedMail`, `PasswordResetLinkSent` event
- Two-factor TOTP — enroll, verify, recovery codes, replay protection
- Brute-force / login throttle — IP + identifier keyed,
  `LoginThrottleMiddleware`
- Remember-me cookies with stable opaque tokens
- Six auth events — `LoginAttempted`, `LoggedIn`, `Authenticated`,
  `LoggedOut`, `PasswordResetLinkSent`, `EmailVerified`
- Browser sessions backed by the Torii fork at
  `github.com/entrepeneur4lyf/suprnova-torii-rs`

#### Authorization

- `Gate` facade — `define`, `allows`, `denies`, `authorize`, `any`,
  `none`, `check` (sync + async variants)
- `#[policy(Model)]` macro for policy registration
- Resource-route auto-authorization

#### Payments

- Provider-agnostic five-trait surface — `Checkout`, `Payment`,
  `Subscription`, `CustomerStore`, `WebhookHandler`
- `PaymentProvider` umbrella trait + capability-querying via `as_payment()`
- DB mirror — `customers`, `subscriptions`, `subscription_items`,
  `payments`, `refunds`, `payment_webhook_events` (UNIQUE for idempotency)
- Flow-tagged `SessionPayload` enum (one-shot vs subscription)
- Two reference adapters as workspace crates —
  `suprnova-payments-stripe` (gateway, full `Payment` impl),
  `suprnova-payments-paddle` (Merchant of Record, no `Payment` impl)
- Mock provider for tests

#### Queue, jobs, batches, chains

- `Job` trait — `handle`, `max_tries`, `backoff`, `timeout`,
  `fail_on_timeout`
- `Queue::push`, `Queue::push_later`, `Queue::push_unique`,
  `Queue::push_unique_later`
- Drivers — `sync`, `null`, `redis`, `database`
- `JobMiddleware` trait — six built-in middleware
- Batches and chains — `Queue::batch(jobs).dispatch()`, fluent chain
  builder, cancellation, progress tracking
- Failed-jobs store with replay
- Worker with graceful shutdown, configurable concurrency, panic
  recovery via `catch_unwind`, settlement metrics
- Twelve queue events covering queueing, processing, failure, release,
  worker lifecycle

#### Broadcasting and WebSockets

- `ws!()` macro + `Router::ws` for typed WebSocket endpoints
- `WsSocket` Sink/Stream split
- Auto-restart supervisors via `Supervisor` trait
- `BroadcastHub` with `Channel`, `Private`, `Presence` channels
- JSON-envelope protocol, presence join/leave/here, configurable
  presence TTL with crash recovery
- `Broadcastable` bridge to `EventDispatcher`
- Close-on-no-pong heartbeat with configurable WS_TASKS drain
- Per-route WebSocket middleware
- 1 MiB / 64 KiB safer defaults + `WsConfig::generous()` factory
- Origin policy + 1011 close-on-protocol-violation

#### Notifications and mail

- `Notification` trait + `Notify::send(recipient, notification).await`
- Mailable + Markdown template rendering
- Database / mail / broadcast / web-push channels
- VAPID signing + RFC 8291 ECE payload encryption (via
  `suprnova-web-push`)
- VAPID subject validation, retry-after parsing, 8 KiB rejection-body cap
- Notifiable trait for recipient typing

#### Events

- Typed event dispatcher — `EventFacade::dispatch`,
  `EventFacade::listen<E, L>`, `EventFacade::forget`
- Cancellable saving/updating events (return `EventResult::cancel`)
- Queueable listeners

#### Filesystem

- `Storage::disk("name")` with multi-driver support — local, S3,
  Azure, GCS via OpenDAL
- Move, copy, exists, size, mime, last-modified, prepend/append
- Streaming uploads and downloads

#### Cache

- `Cache::store("name")` + driver registration
- Drivers — memory, redis (with bounded connect-timeout), database, file
- `remember`, `forever`, `tags`, atomic increment/decrement, locks

#### Vector DB

- `VectorDriver` trait with four drivers — in-memory, Qdrant
  (UUID-5 ID mapping), Pinecone (native string IDs), MariaDB native
  `VECTOR(N)` + HNSW indexes (11.7+)
- Cosine / dot / euclidean distance

#### Console binary and CLI

- Per-project `console` binary — Rust analogue of `php artisan`,
  runs user-defined commands via `#[suprnova::console::command]`
- `#[derive(Command)]` for typed arguments
- `suprnova` CLI — `new`, `serve`, `migrate`, `db:sync`,
  `generate-types`, `key:generate`, `make:{controller,middleware,action,error,inertia,migration,task,command}`,
  `db:seed`, `model:prune`
- `--version` flag
- Scaffold templates for backend + API starters across three frontends

#### Feature flags

- `DatabaseEvaluator` with snapshot loading
- `CachedEvaluator` with TTL
- `FeatureMiddleware` extractor
- Admin CRUD surface
- `FeatureSync` trait for sub-second propagation across processes

#### Schedule

- Cron expression parser
- `Schedule::task(...)` with composable predicates
- Single-server locks, overlap prevention, dispatch tracking
- `schedule:run` console command

#### Validation

- `validator` 0.20 integration
- `#[request]` + `#[derive(FormRequest)]` macros
- `#[form_request(max_body_bytes = N)]` per-form size cap
- `#[form_request(custom_hooks)]` opt-out for user-written
  `impl FormRequest`
- Lifecycle hooks — `authorize`, `after_validation`,
  `after_validation_async`

#### Database drivers

- SeaORM-backed support for SQLite, Postgres, MySQL, MariaDB
- URL-based driver detection
- Migration system + `migrate`, `migrate:rollback`, `migrate:status`,
  `migrate:fresh`, `migrate:refresh`

#### HTTP client

- `Http` facade — `get` / `post` / `put` / `patch` / `delete`
  returning a `RequestBuilder`; `.send().await` produces a
  `ClientResponse`
- rustls TLS, 30s default timeout, `suprnova/<version>` user-agent
- `json` / `form` / `body` / `header` / `bearer_token` / `basic_auth`
  / `timeout` chainable methods
- `RequestBuilder::retry(max_attempts, base_backoff)` — exponential
  backoff for transient failures and 5xx; respects `Retry-After`
- `Http::fake(|| async { ... }).await` test guard with
  `fake_response(method, url_substring, status, body)` +
  `assert_sent` / `assert_not_sent`

#### Encryption

- `Crypt` static facade + `EncryptionKey` (`crypto::*`); AES-256-GCM
  with 12-byte random nonces
- `encrypt_string` / `decrypt_string` / `encrypt<T>` / `decrypt<T>`
- `CryptPurpose` AAD binding preventing cross-protocol replay
- `APP_KEY_PREVIOUS` rotation
- `suprnova key:generate` CLI command for minting fresh keys

#### Testing

- `#[suprnova_test]` async test macro
- `TestDatabase::fresh::<Migrator>()` with parallel-safe instances
- `TestContainer::bind` for per-test mocks
- HTTP test helpers — `Test::get`, `Test::post`, JSON / form / multipart
- Queue / Mail / Notification / Event fakes
- `assert_emitted`, `assert_dispatched`, `assert_dispatched_times`

### Changed

- Auth verification and password-reset flows now operate through the
  configured user provider instead of Torii internals.
- Generated apps must implement `get_auth_password`; scaffolded examples
  now fail loudly instead of allowing login to always fail silently.
- The local release gate is wired into `scripts/release.sh`, and the repo
  includes an enforced pre-push hook for fmt, clippy, tests, docs, and
  feature builds.
- Scaffolded dev-port documentation moved to the current backend/frontend
  defaults (`8765` / `5765`), with `dev:tls` and `--with-portless`
  documented.
- `MAIL_FROM` is validated before verification or reset tokens are issued,
  avoiding orphaned auth-flow rows when mail configuration is invalid.

### Fixed

- React scaffold template drift from the released starter.
- Root route groups no longer generate duplicate `//` paths.
- Literal-path redirects now dispatch through the intended routing path.
- Broadcasting fanout tests now handle `track` / `untrack` results.
- The mail log driver emits the rendered text body, so verification and
  password-reset links surface in local development logs.
- Password-reset coverage pins session and remember-me revocation behavior.

### Notes

- **Distribution model**: git-based end-to-end.
  `suprnova = { git = "https://github.com/entrepeneur4lyf/suprnova.git" }`;
  CLI via `cargo install --git`. Nothing is published to crates.io.
