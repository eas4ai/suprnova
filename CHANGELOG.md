# Changelog

A readable, per-version log of what changed in Suprnova. Each version
section is that version's release record. A version is released when its
version commit and matching `v<version>` tag are pushed atomically. Newest first.

## 1.3.4 - 2026-08-25

### Added

- **A paused worker now tells you it is paused.** `queue:work` prints one line per
  transition - `2026-08-25 14:03:11 Queue billing PAUSED`, and `RESUMED` on the way
  back - and the worker emits `WorkerQueuePaused` / `WorkerQueueResumed` so you can
  route the same signal into your own alerting. These are the worker-side pair; the
  existing `QueuePaused` / `QueueResumed` fire in whichever process ran
  `queue:pause`, which is never the worker, so until now a worker that went quiet
  because somebody paused its queue was indistinguishable from a hung one. Each
  event fires once per transition, not once per poll. Their `queue` field is
  optional: a worker started without `--queue` drains everything and has no queue
  names to report under `pause_all`, so it reports `None` rather than inventing a
  name a listener could match on.
- **`?include=` paths are capped at five segments, and `max_relationship_depth` moves the ceiling.** A cyclic relationship graph turns `?include=author.posts.author.posts...` into fan-out a client controls, bounded only by the query string. Paths are now truncated while they parse; call `suprnova::max_relationship_depth(n)` in `bootstrap::register()` to change the limit, or pass `0` to turn includes off.
- **`Gt`, `Gte`, `Lt`, and `Lte` compare a field against a number or against another field.** `CompareWith` names the operand and the measure in one value: `Number` for a literal, `NumericField` for a numeric sibling, and `LengthField` for a sibling compared by character count. An operand the rule cannot measure fails the field instead of panicking.
- **Three membership rules join the built-in set: `InArray`, `Contains`, and `DoesntContain`.** `InArray` checks a value against another field's list, and you pass the list directly instead of naming the field in a rule string. `Contains` and `DoesntContain` run over a JSON array and match a parameter only against a string element, so `1` and `"1"` stay distinct.
- **The database pool now has liveness knobs.** `DB_IDLE_TIMEOUT`, `DB_MAX_LIFETIME`, `DB_ACQUIRE_TIMEOUT`, `DB_TEST_BEFORE_ACQUIRE`, and `DB_PING_AFTER_IDLE` control when the pool closes, recycles, and pings a connection, with matching `DatabaseConfig::builder()` setters. Each is unset by default, so an existing deployment's pool behaves exactly as it did. Use them when a NAT gateway or firewall drops idle connections: sqlx exposes no libpq `keepalives_*` equivalent, so pool recycling is the mechanism.
- **`db:seed <Class>` reports its progress.** A targeted run prints a `RUNNING` line before the seeder and an elapsed-milliseconds `DONE` line after it. A bare `db:seed` stays silent. The formatter, `suprnova::two_column_detail`, is available to your own `#[command]` handlers.
- **Many-to-many relations now filter on pivot columns.** `where_pivot`, `where_pivot_op`, `where_pivot_in`, `where_pivot_not_in`, `where_pivot_null`, `where_pivot_not_null`, `where_pivot_between`, `where_pivot_not_between`, `where_pivot_group`, and their `or_` twins constrain `get`, `first`, and `count` on `BelongsToMany`, `MorphToMany`, and `MorphedByMany`. `where_pivot_group` takes a closure and renders one parenthesised group, so it stays atomic inside a following `or_where_pivot`. Pivot filters apply to reads only: `attach`, `attach_with`, `detach`, and `sync` return an error while one is set, and eager loading does not carry them.
- **`where_binary` compares column values byte for byte.** The family (`where_binary`, `or_where_binary`, `where_not_binary`, `or_where_not_binary`) ships on `Builder<M>`, and `where_binary` and `where_not_binary` ship on `DB::table(...)`. MySQL and MariaDB emit `= binary`; Postgres and SQLite return an error when the query renders, rather than falling back to a collation-dependent match.
- **`Builder::try_to_sql_with_bindings_for` renders SQL for a dialect without panicking.** It is the fallible sibling of `to_sql_with_bindings_for`, for the cases where a builder legitimately cannot render for a backend.
- **`Model::refresh_for_update` reloads a row under a `FOR UPDATE` lock.** Call it inside a transaction when you need the row's current state and the exclusive lock in one statement. SQLite has no row-level locking, so the lock clause is a no-op there.
- **`Builder::or_where_key` and `Builder::or_where_key_not` add primary-key filters as a disjunction.** Both fold into the preceding `WHERE` clause the same way `or_where` does, and both ship `or_filter_key` and `or_filter_key_not` aliases.
- **`Builder::in_order_of` sorts rows into an explicit sequence.** Pass a column and the values in the order you want them; rows whose value is not in the list sort last. The values bind as parameters, so they are safe to take from request data.

### Fixed

- **`suprnova serve` runs a frontend-less project.** A project scaffolded with `suprnova new --api` has no `frontend/` directory, and `serve` rejected it as "No frontend directory found. Are you in a Suprnova project directory?" unless you passed `--backend-only`. It now skips the Vite pane and the TypeScript generation that feeds it, and serves the backend. `--frontend-only` still fails on such a project, with a message that says why.

### Upgrading

- **An include path longer than five segments now returns its first five relationships instead of all of them.** Nothing outside a resource's allowlist was ever reachable, so no response gains data; a deep path loses its tail. One status code changes with it: a path whose over-deep tail names a relationship the resource does not allow is truncated before anything validates it, so it now returns `200` with the segments that survived where the full path used to return `400` - adjust any client or test asserting on that rejection. Raise the ceiling with `suprnova::max_relationship_depth(n)` if your API documents paths longer than that.
- **`DatabaseConfig` gained five public fields.** Code that builds one with a struct literal no longer compiles. Use `DatabaseConfig::from_env()` or `DatabaseConfig::builder()`, both of which fill the new fields with the defaults that preserve today's pool behaviour.
## 1.3.3 - 2026-08-25

### Added

- **Failover queue connection.** `FailoverQueueDriver` wraps an ordered list of
  connections: a push the first one refuses is retried on the next, and so on
  down the list. Wire it from env with `QUEUE_DRIVER=failover` plus
  `QUEUE_FAILOVER_CONNECTIONS=redis,database` (each entry reads its own
  driver's variables, so a `database` entry still needs `DB::init()` first and
  still brings its failed-jobs store), or build it directly with
  `FailoverQueueDriver::new(vec![(label, driver), ...])`. Only writes fall
  through: `push` and `bulk_push` walk the list, while `pop`, `pop_from`,
  `ack`, `nack`, `release`, `settle`, `clear`, all four counters and all three
  inspection listings delegate to the first connection and no other, because a
  reservation token is meaningful only to the driver that issued it. The
  operational consequence is documented rather than papered over: a worker on
  the failover connection drains the primary only, so whatever failed over to a
  fallback needs its own worker. `bulk_push` pushes each envelope separately
  rather than forwarding a batch, which both preserves each envelope's own
  `available_at` (Laravel #60950) and keeps a batch the primary half-accepted
  from being re-pushed wholesale onto the fallback. A refusal dispatches
  `queue::events::QueueFailedOver { connection, job_name, exception }`,
  edge-triggered: a connection reports itself once when it enters failure and
  stays quiet until a later push succeeds on it and re-arms it, so an outage
  produces one alert instead of one per dispatch. When every connection
  refuses, the push returns the last connection's error. An empty connection
  list, a missing or blank `QUEUE_FAILOVER_CONNECTIONS`, a nested `failover`
  entry, and an entry naming a driver that doesn't exist are all boot errors -
  the warn-and-fall-back-to-memory behaviour stays on `QUEUE_DRIVER` itself,
  where a typo can't splice an ephemeral backend into a durable chain.
- **Queue inspection API.** `Queue::pending_jobs(queue)` / `delayed_jobs` /
  `reserved_jobs` list the actual envelopes behind the existing
  `pending_size`/`delayed_size`/`reserved_size` counters, as `InspectedJob`
  DTOs (`id`, `queue`, `name`, `attempts`, `payload`, `created_at`) - mirrors
  Laravel's `InspectedJob`. A single `Option<&str>` queue filter collapses
  Laravel's `pendingJobs($queue)` / `allPendingJobs()` pair (and the
  `delayedJobs`/`reservedJobs` equivalents) into one call each. The
  `QueueDriver` trait default is an honest `Err` - not Laravel's
  Beanstalkd/SQS empty-collection default, which reads as "nothing queued"
  even when there plainly is - so a driver that has not implemented
  inspection says so; `sync`/`null` override with `Ok(vec![])` because for
  them that really is the truth. The memory, database, and Redis drivers all
  implement the full listing: the memory driver's delayed storage moved from
  a bare `DelayQueue<Envelope>` (which cannot be iterated) to a
  `DelayQueue<Uuid>` plus an id-keyed map; the database driver reuses the
  size counters' exact predicates plus `ORDER BY available_at`, and a row
  whose `envelope_json` fails to decode is still listed (`id: None`,
  `payload: {"unparseable": true}`) rather than dropped, so one poison row
  can't blind an operator to the rest of the queue; Redis's `reserved_jobs`
  is scoped to this consumer's in-process reservations (documented), and
  `pending_jobs` scans the stream via `XRANGE` in batches. `Queue::fake()`
  gained matching `pending_jobs()`/`delayed_jobs()` helpers, projecting
  recorded pushes with `attempts` always `0` and `created_at` always `None`.
- **After-commit dispatch.** `Job::after_commit()` holds a push until the
  surrounding `DB::transaction` commits, so a worker on another process can
  never pop an envelope that describes rows the transaction has not made
  durable yet. The whole push waits, not just the driver write: the envelope
  build, `JobQueueing` and `JobQueued` all happen at commit time, so no
  listener is ever told about a job a rollback then discards. A rollback
  discards the push entirely; outside a transaction the push happens
  immediately, which is what lets a job type declare the opt-in without every
  dispatch site knowing whether its code path is transactional. Per dispatch,
  `EnvelopeOverrides::after_commit` outranks the job: `Some(true)` (with the
  shorthand `Queue::push_after_commit(job)`) defers a job that did not opt in,
  and `Some(false)` is Laravel's `beforeCommit()`. A deferred `Queue::push`
  re-resolves `Job::delay()` against the commit rather than the push, while
  `Queue::push_later` / `later` / `later_with` carry the caller's absolute
  timestamp through unchanged. `Queue::push_unique` takes its dedupe lock
  immediately even when the envelope is deferred, so a duplicate inside the
  same transaction is still suppressed, and a rollback releases that lock
  owner-scoped. `Queue::bulk` defers as a unit. `Queue::fake()` records a push
  immediately, deferral and all, matching Laravel's `Bus::fake`. Manual
  `DB::begin_transaction` never defers - it installs no ambient transaction, so
  there is no commit to hang a callback on. Every ending that leaves the commit
  unlanded compensates identically, including a `COMMIT` the database refuses
  and a leaked `TxHandle` that blocks one, and `Transaction::rollback_to` counts
  as one for the scope it unwinds: a push deferred inside a savepoint is
  discarded when that savepoint rolls back and its lock is released right then,
  while anything registered before the savepoint is untouched. Queued mail,
  notifications, batches and chains do not defer yet.
- **Unique-until-processing jobs.** `Job::unique_until_processing()` releases the
  uniqueness lock when processing begins - after the job's middleware pass,
  immediately before the handler runs - instead of holding it for the full
  `unique_for` window, which is what you want when the lock exists to coalesce
  queued duplicates rather than to serialize execution. A job that a middleware
  releases back onto the queue keeps its lock, because it has not started
  processing; a job a middleware deletes or dead-letters gives its lock up.
  Release is owner-scoped: `Queue::push_unique` records the cache lock's owner
  token on the envelope (`Envelope::unique_lock_owner`, an additive field that
  leaves the frozen wire format byte-identical for every non-unique push), and
  the worker releases with that token, so a redelivered attempt can never
  force-release a lock a newer dispatch now holds. The supporting idempotency
  surface is public too: `Idempotency::commit_on_success_owned` hands the body
  the lock owner and returns it, and `Idempotency::release_owned(key, owner)`
  releases owner-scoped, reporting `Ok(false)` rather than an error when the
  lock is absent or held by somebody else. Plain `unique_id` jobs are unchanged
  and still let the `unique_for` TTL be the dedupe window.
- **`Gate::default_denial_response` customizes the default shape of a bare denial.** Mirrors
  Laravel's `Gate::defaultDenialResponse($response)`. Set once - typically in
  `bootstrap::register()` - it reshapes exactly two outcomes: a bare `false` (a bool gate -
  `Gate::define` / `Gate::define_async`, including a `#[policy]` method returning `bool` - or a
  `before`/`after` hook that decided `false`) and an evaluation nothing else decided at all (an
  undefined ability with no hook opinion either). All of those used to collapse to a bare
  `Response::deny()` (a 403); now they surface as whatever `Response` the default carries, e.g.
  `Response::deny_as_not_found()` for a 404 that hides a resource's existence application-wide
  instead of gate by gate. The default applies to bare `false` only - a gate registered with
  `define_with` / `define_async_with` already returned the `Response` it wanted, and that always
  passes through `Gate::inspect` untouched, matching Laravel's own rule that the default never
  substitutes for a returned `Response` object. A default shaped as `Response::allow()` is
  rejected (logged, ignored) rather than silently inverting every bool gate to allowed - see
  `Gate::default_denial_response`'s doc comment for the one place this deliberately diverges from
  Laravel, which has no such guard.
- **The `Password` validation rule family ships, including the Have I Been Pwned
  `uncompromised()` check.** `Password::min(n)` plus the strength builders
  (`.max()`, `.letters()`, `.mixed_case()`, `.numbers()`, `.symbols()`) port
  Laravel's `Password` rule regexes verbatim - a plain space satisfies
  `.symbols()`, matching Laravel's `\p{Z}` separator class. `.uncompromised()`
  (or `.uncompromised_with_threshold(n)`) checks the password against Have I
  Been Pwned's k-anonymity range API: only the first 5 characters of the
  password's SHA-1 hash ever leave the process, and a network failure,
  timeout, or non-2xx response fails open rather than blocking signups,
  exactly like Laravel's `NotPwnedVerifier`. Because that check is an HTTP
  round trip, `Password` is the one built-in rule implementing both `Rule`
  (strength only, for sync `validate!` rows) and `AsyncRule` (strength, then
  the HIBP check, for `after_validation_async`) - calling the sync path on a
  `Password` configured with `uncompromised()` is a loud, developer-facing
  error rather than a silent skip. `Password::defaults_with(...)` sets the
  process-wide default `Password::defaults()` returns. New `HIBP_TIMEOUT_SECS`
  env var (default 30s). `Http::fake_response_text(...)` is the new raw-body
  sibling of `fake_response(...)` for tests against `text/plain` upstream
  APIs like HIBP's.
- **A scheduled task can now name the timezone its cron expression is read
  in, and `schedule:list` can render the whole schedule in any zone.**
  `.timezone(chrono_tz::Tz)` pins one task, `.try_timezone("Area/City")` is
  the fallible sibling for a zone name that only exists at runtime, and
  `Schedule::timezone(tz)` sets a default for every task registered after
  it. Nothing changes for a task that pins no zone: it is still evaluated
  against the process's local zone. A pinned zone affects due-ness only -
  the scheduler still ticks once per process minute and the same-minute
  dedup gate is untouched. Note that a zone observing daylight saving makes
  some wall-clock minutes happen twice and others not at all, so a task
  pinned to such a minute can run twice or be skipped; the scheduling
  chapter carries the full warning. `schedule:list` gained a `--timezone`
  option and two columns: the zone a printed expression is written in, and
  the next minute the task fires. A pinned task's expression is rewritten
  into the listing's zone, splitting into several lines when it straddles
  midnight there, and is left exactly as written when a faithful rewrite is
  impossible - across a daylight-saving transition, when a day rollover
  would have to move a restricted day-of-month and day-of-week together, or
  when it would have to decide how long February is. `chrono_tz::Tz` is
  re-exported from the crate root, so consuming apps do not add `chrono-tz`
  to their own `Cargo.toml`.
- **A Laravel-shaped image subsystem, in `suprnova::media` behind the default-on
  `media` feature.**
  `Image::from_bytes/from_path/from_disk/from_upload/from_stream` builds a lazy
  pipeline - `resize`, `scale`, `crop`, `cover`, `contain`, `rotate` at any
  angle, `flip_vertically`/`flip_horizontally`, `blur`, `sharpen`, `grayscale`,
  `to_format`, `quality` - finished with `to_bytes`, `to_response`, `save`,
  `store`, `dimensions`, `mime_type`, or `dominant_color`. Reads and writes
  PNG, JPEG, WebP, GIF, and BMP; AVIF output is deferred until the in-house
  AV1 encoder publishes, at which point it is one new `OutputFormat` variant
  and no other change. Like Laravel's `gd`/`imagick` split there are two
  drivers: `IMAGE_DRIVER=oxideav` (the default) runs on the pure-Rust
  [OxideAV](https://github.com/OxideAV) codec family with no native library
  and nothing to install, and `IMAGE_DRIVER=magick` shells out to a
  host-installed ImageMagick 7 for wider input support including HEIC.
  Decode limits (`IMAGE_MAX_DIMENSION`, `IMAGE_MAX_ALLOC_BYTES`) are checked
  against the input's own header before anything is allocated - including the
  inner bitstream of an extended WebP, whose advisory canvas size cannot be
  used to smuggle a larger frame past the gate - and all pixel work runs on a
  blocking thread. The `magick` driver pins the input coder by name rather
  than letting ImageMagick pick one from the bytes, and bounds every
  invocation with `IMAGE_MAGICK_TIMEOUT_SECS`. `ImageDriver` is the trait
  boundary for anything else. The module is named `media` because the
  OxideAV-backed audio and video surfaces will live beside it.
  [Images](manual/images.md)
- **The WebP gate carries one fixed, non-configurable bound.** A WebP declares
  its real decoded size in its innermost bitstream chunk, so the framework
  walks the container to find it; that walk visits at most 4096 chunks per
  level and follows two levels of nesting, and a file past either is refused
  rather than measured. Reporting a number from an unfinished walk would be a
  gate that enough filler chunks could step around. No `IMAGE_MAX_*` variable
  affects it and the error says as much. A 300-frame animation is unaffected;
  a 4100-frame one is refused. [Images](manual/images.md#one-bound-is-not-configurable)

### Changed

- **`DB::transaction` can now return `Err` after a successful commit**, when an
  after-commit callback fails: the message reads `after-commit callback failed
  (the transaction itself committed): …`, the closure's return value is lost and
  its writes are not. `DB::transaction_with_attempts` never retries that error,
  however deadlock-shaped the callback's own message reads - re-running a closure
  whose writes are already durable would apply them twice.
- **New validation catalog key: `validation-password-unverifiable`.** A custom
  `UncompromisedVerifier` that returns `Err` no longer puts its own error text
  in the 422 body verbatim. That text is logged at `error` instead, and the
  response carries this key, rendering as "The { $field } could not be checked
  against known data leaks. Please try again." - the check did not run, which is
  not the same as the password being bad, and infrastructure detail does not
  belong in a client response. An app shipping its own validation catalog has to
  add the key, or its users see the built-in English fallback.
- **The `Image` upload validator is now `ImageFile`.** `suprnova::Image` is the
  new image-manipulation pipeline type, matching `Illuminate\Image\Image`,
  and the magic-byte upload rule takes the name Laravel gives the same rule
  class, `Illuminate\Validation\Rules\ImageFile`. Migration is one line per
  use site: `UploadedFile<(Image, MaxSize<N>)>` becomes
  `UploadedFile<(ImageFile, MaxSize<N>)>`. Pre-1.0 churn absorbed by the
  git-tag distribution model.

### Removed

- **The unused direct `image` dependency is gone.** It had been a base
  dependency with zero use sites anywhere in the workspace, pulling JPEG, PNG,
  WebP, and GIF codecs in for nothing; dropping it removes `gif`, `image-webp`,
  `zune-jpeg`, `color_quant`, and `weezl` from the tree. The crate itself still
  appears transitively, with only its `png` feature, behind `totp-rs`'s
  QR-code rendering. The new image subsystem is built on the OxideAV crates
  behind the `media` feature instead.

### Upgrading

- **`Image` is a different type now; the upload validator is `ImageFile`.**
  Source-breaking for anyone using the magic-byte upload rule. Rename it at
  every use site: `UploadedFile<(Image, MaxSize<N>)>` becomes
  `UploadedFile<(ImageFile, MaxSize<N>)>`. `suprnova::Image` still resolves, but
  it is now the image-manipulation pipeline type, so a missed rename fails to
  compile rather than changing behaviour silently.
- **`EnvelopeOverrides` gained a public `after_commit: Option<bool>` field.**
  Every construction in this repo and in the scaffolded templates uses
  `..Default::default()`, which needs no change. Code that builds an
  `EnvelopeOverrides` with an exhaustive struct literal has to name the new
  field; `after_commit: None` keeps today's behaviour, which is to defer to
  `Job::after_commit()`. Nothing else changes: `after_commit()` defaults to
  `false`, so no existing job starts waiting for a commit it did not before.
- **`Envelope` gained a public `unique_lock_owner: Option<String>` field.** The
  wire format is unchanged - the field is `#[serde(default)]` and skipped when
  `None`, so envelopes round-trip byte-identically in both directions and
  `schema_version` stays at 2 - but any code that builds an `Envelope` with a
  struct literal now has to name it. Add `unique_lock_owner: None` unless you
  are deliberately carrying a uniqueness lock across the push. Code that only
  reads envelopes, or builds them through `Queue::push` and its siblings, needs
  no change.

## 1.3.2 - 2026-08-25

### Added

- **OAuth providers can now be registered through `MagnetarConfig::oauth`.** Suprnova re-exports the `OAuthProvider` contract, all five first-party provider and configuration types, and the HTTP, revocation, abuse-limiter, authorization, and auto-link types an application needs. Custom providers no longer require a direct `suprnova-magnetar` dependency or a hand-retained `MagnetarHostEngine`.

- **A production OAuth transport and framework limiter adapter now ship at the crate root.** `ReqwestOAuthTransport` implements token, userinfo, and revocation I/O with redirects disabled by default, a 30-second timeout, a default `User-Agent`, and a 1 MiB response cap. `FrameworkAbuseLimiter` reuses the configured `RateLimiterDriver`; apps no longer hand-write either adapter.

### Fixed

- **`init_magnetar` now publishes OAuth with password and passkey services as one reserved installation.** The OAuth service is built before publication, and all three engine slots remain hidden while the reservation is active. A failed or duplicate OAuth configuration cannot leave password and passkey state visible without the configured OAuth registry.

- **Custom providers can supply userinfo headers.** `OAuthProvider::userinfo_headers` is merged with the host-owned bearer header, enabling requirements such as GitHub's `User-Agent` and media-type `Accept` headers without allowing a provider to replace `Authorization`.

### Upgrading

- **The Magnetar cutover in `4faaa933` removed Torii's OAuth installation path without wiring its replacement into the default initializer.** The old workaround required constructing a custom host engine, calling `oauth_service`, and installing the adapter separately. Replace that workaround with `MagnetarConfig::from_sea_orm(database).oauth(oauth_config)` and one `init_magnetar` call.

- **GitHub community providers must handle verified email explicitly.** GitHub `/user` usually omits non-public email, while the verified primary address requires `/user/emails`. Return `email: None` to use the email-completion ceremony, or point `userinfo_endpoint` at a host adapter that combines both responses; never treat a public but unverified address as ownership.

## 1.3.1 - 2026-08-24

### Fixed

- **Provider-backed applications can reset verified users again.** When no Magnetar engine is installed, `PasswordReset` uses an explicitly reset-capable `UserProvider` and framework `auth_flow_tokens` for already verified accounts. `EloquentUserProvider<M>` opts in when `M` implements `MustVerifyEmail + CanResetPassword`; no `app_users` migration is required.
- **The published framework line now contains both post-release repair sets.** The translated 1.3.0 changelog layout and headings, CJK wrapping, localized anchors, glossary terms, and prose punctuation are reconciled instead of split across divergent local and remote branches.
- **Post-tag CLI and Magnetar hardening is included.** Development-process cleanup uses the completed process-group fallback, and the local qualification contracts cover the released refs and plugin-SDK SQLite lanes.

### Security

- **The provider fallback never treats password reset as first mailbox proof.** Unknown and unverified addresses receive the same no-mail response. Install Magnetar when an unverified account must prove mailbox ownership through reset so credential cleanup, auth-epoch advancement, and revocation remain atomic. Provider fallback completion reports framework session and remember revocation failures through `PasswordResetOutcome`.

### Upgrading

- **Move every `v1.3.0` Git dependency to `v1.3.1`.** Applications with their own `users` table keep their configured `UserProvider`; they do not initialize the default `app_users` engine merely to reset an already verified account. Applications that use Magnetar credentials or unverified-account first proof continue to initialize Magnetar.


## 1.3.0 - 2026-08-24

### Security

- **Magnetar now fences credential and session mutations to the authenticated
  actor and account auth epoch.** Password, passkey, linked-account,
  two-factor, opaque-session, JWT, remember, OAuth, and device-authorization
  writes reject stale or revoked actors. The first successful password-reset,
  magic-link, or OAuth verified-email proof on an unverified account advances
  the epoch and atomically removes provisional credentials, sessions, remember
  state, and squatter TOTP enrollment. Verified accounts preserve legitimate
  credentials during password reset. Email verification requires the
  authenticated token owner, and OAuth never auto-links an unverified existing
  account from email alone.

- **A protocol-relative `_previous.url` can no longer produce an off-origin open redirect through
  `Redirect::back()`, on either the write side or the read side.** `SessionMiddleware` no longer
  persists a protocol-relative current URL: the write goes through the identical sanitizer
  `InertiaValidationRedirectMiddleware` uses for its `Referer` check, and a request path shaped
  like `//host` (or carrying an ASCII control byte) is never recorded - without this, an app's
  `fallback!` route (the standard Inertia/SPA app-shell pattern, where any unmatched path answers
  `200`) could have `GET //evil.test/anything` persist that path verbatim. `SessionData::previous_url()`
  now applies the same check on every **read**, too, so a session cookie that survived an upgrade
  from a release before this fix - already carrying a raw, unsanitized value no write in the
  current process ever produced - self-heals to "nothing recorded" instead of being trusted.
  Together, neither an old poisoned cookie nor a new malicious request can hand `Redirect::back()`,
  `Redirect::refresh()`, or `url::previous()` an off-origin `Location`. When a value fails either
  check it's treated as absent rather than replaced with a synthesized one, so a genuinely good
  previous URL is never clobbered.
- **The Inertia validation-redirect bridge's `Referer` check closed two more same-origin bypasses.**
  `InertiaValidationRedirectMiddleware`'s `303` target only rejected a `Referer` starting with the
  literal `//` or `/\` prefix - a value like `Referer: /<TAB>/evil.test` slipped through, because
  the WHATWG URL parser strips ASCII tab and newline from the whole string before comparing
  origins, so a browser reads that as `//evil.test` and follows the `303` off-origin. The check now
  rejects any ASCII control byte (C0 or DEL) anywhere in the candidate, not only within the two
  named prefixes. Separately, the last-resort fallback - the failing request's own path, used when
  neither `Referer` nor the session's previous URL is usable - was never sanitized: an origin-form
  HTTP request-target is syntactically free to start with `//`, so a raw client or a
  non-normalizing proxy could turn the "safe last resort" into an off-origin redirect too. Both
  legs now share one root-relative check, falling back to `/` if even the request's own path fails
  it.
- **Cookie ciphertext is now bound to its logical cookie name with contexted v2 AAD.** `Cookie::encrypted` /
  `Cookie::read_encrypted_for` stop a value minted for one cookie slot from decrypting in another slot,
  while the logical-name binding keeps a later `__Host-` / `__Secure-` wire-prefix flip safe. The
  version-less compatibility window tries v2 across the whole key ring, then v1 across the whole ring,
  so existing cookies survive the rollout; the v1 fallback preserves the old replay weakness until its
  scheduled 1.4.0 removal.
- **Session and remember-me cookie prefixes are validated at boot and enforced at render time.**
  `SESSION_COOKIE_PREFIX=__Host-` requires `Secure`, `Path=/`, and no `Domain`; `__Secure-` requires
  `Secure`. Invalid boot combinations fail before serving, and the renderer rewrites invalid prefixed
  headers instead of letting browsers discard them silently.

### Added

- **Suprnova authentication now runs on the internal Magnetar engine.** The
  framework-owned `Auth` facade preserves existing password, magic-link,
  passkey, OAuth, bearer, lockout, session, and two-factor call sites while
  removing the Torii dependency. The default engine installs password/session
  and passkey adapters atomically, stores lifecycle delivery leases in the
  application database, and shares the application's canonical `i64`
  `app_users` identities.
- **A shape-aware authentication migration runner now covers Torii, Suprnova
  web, and Suprnova API sources.** Dry runs bind a stable plan id to durable
  row and schema fingerprints plus destination identity decisions. Apply uses
  transactional imports, retry ledgers, shape-owned cleanup, and collision
  refusal. MySQL uses a write-barrier-protected shadow swap with pre-copy
  journals, row and schema parity, resumable renames, and cleanup-preserving
  restore.
- **`MAIL_DRIVER=file` writes one RFC 5322 `.eml` per message** to `MAIL_FILE_PATH` (default
  `storage_path("mail")`; a relative value anchors at the application base directory, not the process
  CWD), so local mail can be opened in a mail client instead of read out of a log line. The
  file carries the same header superset SMTP emits, including `X-Priority`, `Importance`, `X-Tag`,
  `X-Metadata-*`, and `Return-Path`. Like `log` and `memory`, it does not deliver: a production boot
  refuses it unless `MAIL_ALLOW_NON_DELIVERING_IN_PRODUCTION=true`.
- **`FrameworkError::External` carries the error it wraps.** `FrameworkError::from_external(e)` and
  `FrameworkError::from_external_with("saving user", e)` keep the original error reachable as a
  `std::error::Error` source instead of melting it into a string. `FrameworkError::external_source()`
  returns it for downcasting - use that rather than `source()`, which yields the shared `Arc` handle.
  Both constructors map to HTTP 500.
- **5xx logs now render the full error source chain.** `render_error_chain` walks `source()` and is
  wired into the framework-error log line, the `ErrorOccurred` event payload, and the `debug_message`
  field emitted under `APP_DEBUG=true`. Client-facing response bodies are unchanged and 5xx bodies
  stay sanitised.
- **`InertiaResponse::scroll_wrapped` / `scroll_with_wrapped` / `try_scroll_wrapped`.** Nest a scroll
  prop's merge instruction under `<key>.<wrap_key>` instead of the bare key - `mergeProps:
  ["users.data"]` rather than `["users"]` - for a value that's itself an envelope (`{ data: [...], meta:
  {...} }`). Laravel's `ScrollProp` wraps under `"data"` unconditionally; Suprnova's built-in paginators
  hand back a bare row array, so this is opt-in rather than a default every caller has to work around.
  New `ProvidesScrollMetadata` trait (`page_name` / `previous_page` / `next_page` / `current_page`, with
  a default `scroll_metadata()`) mirrors Laravel's interface of the same name for a paginator this crate
  doesn't know about; `LengthAwarePaginator`, `Paginator`, and `CursorPaginator` now implement it instead
  of building `ScrollMetadata` by hand. A scroll prop's `.match_on(...)` fields now also emit into
  `matchPropsOn`, matching Laravel's `resolveMergeMatchingKeys` (`Response.php:641-652`), which folds a
  `ScrollProp`'s `matchesOn()` in the same as any other merge prop - the match entry keys off wherever the
  prop actually merges, `<key>` unwrapped or `<key>.<wrap_key>` under `.scroll_wrap(...)`.
- **`Prop::merge_with_path`, multi-field `match_on`, and resolver-backed merge props.**
  `Prop::merge_with_path(path)` merges a nested field inside a prop's value instead of the whole
  prop - `Prop::eager(v).merge().merge_with_path("data")` emits `mergeProps: ["<key>.data"]`, and a
  path-merging prop never also merges its root; `.deep_merge()` ignores it, since a deep merge
  already recurses into every field. `Prop::match_on` now takes one field or several in one call
  (`match_on(["id", "slug"])`) on top of the `match_on("id").match_on("slug")` chaining `Prop`
  composition already supports. `InertiaResponse::merge_lazy` / `merge_lazy_with` add the
  resolver-backed siblings of `.merge` / `.merge_with`, matching Laravel's
  `Inertia::merge(fn () => ...)`.
- **Partial-reload `only`/`except` understand dot notation.** `X-Inertia-Partial-Data: user.name`
  narrows the `user` prop to `{ name: ... }` instead of requiring the whole value or nothing;
  `X-Inertia-Partial-Except: user.email` prunes just that field, leaving the rest of `user` in place.
  `except` wins on a path both headers name, a bare entry still means the whole prop, and an unknown
  or type-mismatched nested path drops silently without touching its siblings. `Always` props are
  unaffected - they always ship whole.
- **Dot-key prop nesting.** `.with("user.name", value)` (and any other prop-attaching method, eager or
  resolved) now nests into `props.user` instead of shipping a literal `"user.name"` key, matching
  Laravel's `Arr::set`-based `resolveArrayableProperties` unpacking. Two calls sharing a prefix -
  `.with("user.name", …)` then `.with("user.age", …)` - accumulate into one object; a key with no dot is
  unaffected. `App::inertia_share*` shared-registry keys nest the same way on the wire. The unpacking
  only ever touches top-level prop *keys* - it never recurses into a prop's value, so a validation
  `errors` bag keeps whatever dotted field names it carries internally.
- **`App::inertia_shared(key)` / `App::flush_inertia_shared()`.** Laravel's `Inertia::getShared` /
  `Inertia::flushShared`, reading and clearing the static share registry (`App::inertia_share` / `_lazy`
  / `_once`). `inertia_shared` supports the same dot notation as `inertia_share` for the read side; it
  returns `None` for a lazy or once share (there's no request to resolve one against) and for an
  unregistered key. `flush_inertia_shared` clears only the static registry - a trait provider registered
  via `App::register_inertia_shared` is untouched, matching Laravel (there's no per-request state there
  to flush).
- **`InertiaResponse::always_with(key, resolver)`.** The async-resolver sibling of `.always(key, value)`,
  for an always-included prop expensive enough to be worth resolving lazily - Laravel's
  `Inertia::always(fn () => …)` (`AlwaysProp` accepts any value, closures included).
- **`InertiaSharedData::share` now receives the page component name**, so a provider can vary its output
  by page - Laravel's `RenderContext`. See Upgrading.
- **Inertia prop composition.** A `Prop` now carries orthogonal flags instead of being one of nine
  closed variants, so a single prop can be deferred *and* mergeable, mergeable *and* cached, or
  optional *and* cached - the combinations the Inertia 3 protocol expects and a closed enum could
  not spell. Build one with `Prop::eager` / `Prop::lazy` / `Prop::from_resolver` / `Prop::absent`,
  chain `.always()`, `.optional()`, `.defer()`, `.group()`, `.rescue()`, `.merge()`, `.prepend()`,
  `.deep_merge()`, `.match_on()`, `.once()`, `.as_key()`, `.until()`, `.fresh()`, `.scroll()`, and
  attach it with the new `InertiaResponse::prop(key, prop)`. A `defer().merge()` prop is announced
  under `deferredProps` on the first render and arrives under `mergeProps` on the follow-up request.
  New `MergeMode` and `Visibility` types describe the flags; every existing builder shortcut
  (`.with`, `.always`, `.lazy`, `.optional`, `.defer`, `.merge*`, `.once*`) is unchanged.
- **Queue pause / resume.** `Queue::pause(connection, queue)` / `resume` / `pause_all()` /
  `resume_all()` / `is_paused(connection, queue)` / `paused_queues(connection, &queues)`, backed by
  `Cache` the same way the restart signal is - `resume_all` does not clear a per-queue pause,
  matching Laravel. The worker's claim gate sits right before every pop, so an in-flight job always
  finishes; a global pause short-circuits `--queue=...` filtering the same way Laravel's
  `pausedQueues` does, and a per-queue pause only takes effect on a worker started with an explicit
  `--queue=...` list. New CLI commands `queue:pause [queue] [--all]` / `queue:resume [queue] [--all]`
  (alias `queue:continue`), plus `QUEUE_PAUSABLE=false` for an operator to disable the feature -
  an unpausable worker ignores pause signals, and `queue:pause` itself refuses to run. New events:
  `QueuePaused` / `QueueResumed` / `QueuesPaused` / `QueuesResumed`.
- **`suprnova::testing::TestResponse`** - a fluent, Laravel-`TestResponse`-shaped wrapper over the
  `(status, headers, body)` triple every HTTP test harness already produces: `assert_status`,
  `assert_ok`, `assert_redirect`, `assert_json`, `assert_json_path`, `assert_json_count`,
  `assert_see`, `assert_header`, `assert_cookie`, and (given `.with_session_store(...)`)
  `assert_session_has`. Every assertion returns `&Self` and panics on failure, the same contract as
  `expect!`. Nothing about how a test drives a request has to change.
- **`suprnova new` scaffolds an SSR entry.** Every starter (Svelte, React, Vue) now ships
  `frontend/src/ssr.{ts,tsx}` and a `build:ssr` npm script (`vite build --ssr`), wired to its own
  output directory (`frontend/bootstrap/ssr/`) so the SSR bundle never collides with the client
  build in `public/assets/`.
- **`InertiaConfig::ssr_bundle_path(path)` / `.ssr_ensure_bundle_exists(bool)`.** The SSR gateway
  can now check the built bundle exists on disk before dispatching a render, mirroring Laravel's
  `ensure_bundle_exists` config - a worker that was never started, or a bundle that was never
  built, fails fast instead of paying `ssr_timeout` on a connection that was never going to
  succeed. Opt in with `.ssr_bundle_path(...)`; unlike Laravel's `BundleDetector` the path is never
  auto-detected, so existing SSR configs (and tests) that don't set one are unaffected.
- **Validation failures on an Inertia visit now redirect back instead of returning `422` JSON.**
  `Inertia::install` registers a fourth middleware, `InertiaValidationRedirectMiddleware`, which
  turns a validation `422` on an `X-Inertia` request into a `303` to the form page with the errors
  flashed - so `useForm().errors` fills in with no handler code. The Inertia client treats any
  response without an `X-Inertia` header as non-Inertia and shows its error modal, so the old `422`
  could never reach `form.errors`. Non-Inertia requests keep the `422` envelope, Precognition
  dry-runs are untouched, and `X-Inertia-Error-Bag` scopes the flashed bag. The redirect target is
  the same-origin `Referer`, then the session's previous URL, then the request's own path run
  through that same sanitizer, falling back to `/` if even that fails it - never trusted verbatim.
- **`InertiaConfig::with_all_errors(bool)`** - keep every validation message per field instead of
  collapsing to the first. Mirrors Laravel's `Inertia\Middleware::$withAllErrors`.
- **`suprnova::testing::AssertableInertia`** - fluent, Laravel-`AssertableInertia`-shaped assertions
  over an Inertia page object, parsed from either an `X-Inertia` JSON response or a hard-navigation
  HTML shell's embedded `<script data-page="app">` element: `component`, `url`, `version`, `prop`,
  `has`, `missing`, `where_`, `count`, `has_flash`. Build one from an `HttpResponse` with
  `AssertableInertia::from_response`, or from a `TestResponse` with the new
  `TestResponse::assert_inertia()`. `reload_only`, `reload_except`, and `load_deferred_props` replay
  a partial reload against a caller-supplied `with_reload(...)` closure - Suprnova's HTTP tests cross
  a real socket, so there's no single in-process test client to hardcode against.
- **`Cookie::queue`/`queued`/`unqueue`/`expire`.** A task-local cookie jar - Laravel's `CookieJar` -
  lets any code queue a cookie for the next outgoing response without holding an `HttpResponse` to
  attach it to: an event listener, a container-bound service, middleware ahead of the handler.
  Backed by the same per-request slot `Auth::login_remember` already uses to carry the remember-me
  cookie past the handler boundary; `SessionMiddleware` drains it onto the response next to the
  session cookie. `Cookie::expire(name, path, domain)` queues a deletion cookie built with
  `Cookie::forget_with`. Requires `SessionMiddleware` in the route's middleware chain - outside it,
  all four calls are a silent no-op, matching `App::flash`'s behavior outside a flash scope.
- **`HttpResponse::event_stream(stream, end)` and `HttpResponse::stream_json(stream)`.** Laravel's
  `ResponseFactory::eventStream` / `streamJson`, and the exact wire shapes
  `@laravel/stream-{react,vue,svelte}`'s `useEventStream` / `useJsonStream` expect. `event_stream`
  frames a `Stream<Item = sse::StreamedEvent>` as `event: update` per item unless the item names its
  own event, JSON-encodes any non-string payload, and appends a configurable terminal frame
  (`EndSignal::default()` is `data: </stream>`; `EndSignal::None` omits it). `stream_json` streams
  any `Stream<Item = impl Serialize>` as one incrementally-flushed JSON array. Both are built on the
  existing `sse`/`stream_bytes` body pipeline, so they share its cancellation and panic-isolation
  behavior with the rest of the framework.
- **`suprnova serve` respawns a crashed dev process instead of tearing the whole session down.**
  Exponential backoff between attempts - 200ms, doubling on each consecutive crash, capped at 5s,
  resetting to the floor once a process has stayed up 30s. `--no-restart` opts out and restores the
  previous behaviour. `--restart-tries <N>` (default `5`, matching Laravel's `--restart-tries=5`)
  gives up retrying a process after that many consecutive crashes instead of retrying forever,
  printing an actionable message and leaving the other processes - and the session itself - running.
  `--timestamps` prefixes every forwarded line with `HH:MM:SS`. A new `Suprnova.toml`
  `[[serve.process]]` array lets a project declare its own dev processes - Laravel's
  `DevCommands::register` - to run alongside the backend and frontend, each with its own `[name]`
  prefix and an optional color; an unknown key or a blank `name`/`command` in an entry is now a hard
  parse error instead of silently ignored or a later opaque spawn failure. `--json` emits one JSON
  object per line (NDJSON) on stdout instead - process start, output, exit, restart-scheduled,
  restart-succeeded, gave-up, types-regenerated, and shutdown events, including the file watcher's
  own regeneration notices and the `Ctrl+C` handler's shutdown notice, both of which now stay off
  stdout under `--json` too - for scripting and log pipelines; combining it with `--timestamps` is
  harmless but redundant, since every event already carries its own timestamp.
- **`RequestBuilder::retry_when(predicate)`.** A predicate consulted before every retry the
  built-in policy (`.retry(...)` / `.retry_non_idempotent(...)`) would otherwise make, receiving a
  `RetryContext { attempt, method, url, outcome: RetryOutcome::TransportError | Status(u16) }`. It
  composes with the policy rather than replacing it: `false` vetoes a retry the policy would have
  made; it can never force one past `max_attempts` or one the policy wouldn't otherwise attempt
  (a 4xx status, or a non-idempotent method without `retry_non_idempotent`).
- **`#[model(touches = [...])]` now actually touches.** After a child is created, saved, updated, or
  deleted, each `BelongsTo` owner named in the list gets one
  `UPDATE <owner> SET updated_at = ? WHERE <key> = ?`, on the same executor as the write that
  triggered it - so inside a `DB::transaction` the touch joins that transaction and rolls back with
  it. An owner whose model has `timestamps = false` is skipped, not written and not an error
  (Laravel 13.25 closed the same gap). Owners reached through a `NULL` foreign key, and soft-deleted
  owners, are skipped too. A `touches` entry that doesn't name a declared `BelongsTo` relation is now
  a compile error; polymorphic owners are not supported yet.
- **`without_touching_on::<M, _, _>(fut)`** - Laravel's `Model::withoutTouchingOn([M::class], $cb)`.
  Suppresses both `m.touch()` and any owner cascade targeting `M`, while owners of other types keep
  bumping. Scopes nest, and the existing `without_touching` now suppresses the owner cascade as well
  as direct `touch()` calls.
- **`Model::touch_owners()` / `touch_owners_with_tx(tx)`** - Laravel's `touchOwners()`, for when you
  wrote the child row through a path the framework doesn't own.
- **Value-shaped validation rules: `ArrayKeys` and `Distinct`.** A new `ValueRule` trait
  (`passes(&self, value: &serde_json::Value)`) sits alongside `Rule`, sharing the same
  keyed-message contract. `rules::ArrayKeys(&[...])` rejects a JSON object carrying any key
  outside the allowed list (Laravel's `array:keys`, #60918); `rules::Distinct { ignore_case,
  strict }` rejects a JSON array with a repeated element (Laravel's `distinct`). `validate!` rows
  accept either kind of rule in the same field list - dispatch is automatic, chosen by which trait
  the rule implements, not by new row syntax.
- **`Job::delay()`** - jobs can declare a default delay (`fn delay() -> Option<Duration>`, default
  `None`), honored by `Queue::push` and `Queue::bulk`: `available_at` becomes `now + delay` instead
  of `now`. An explicit call-site delay still wins - `Queue::push_later(job, at)` and
  `Queue::later(delay, job)` use the caller's timestamp verbatim and never consult `Job::delay()`.
- **`Notification::{queue, timeout, fail_on_timeout, max_tries, backoff}`.** A queued notification
  (`Notify::queue`) now carries its own queue-tuning defaults onto every per-channel
  `SendNotificationJob` push via the `EnvelopeOverrides` primitive `Mail::on_queue` uses -
  `fail_on_timeout(&self) == true` dead-letters on the first timeout instead of retrying, matching
  Laravel's `#[FailOnTimeout]` notification attribute (#61072). All five default to
  `SendNotificationJob`'s existing `Job` defaults, so a notification that overrides nothing is
  unaffected.
- **`Mail::on_queue` / `Mail::on_connection` + `Queue::push_with`/`later_with`.** A queued mailable
  now routes itself with `Mail::to(..).on_queue("emails").queue(mailable)`, or defaults via
  `Mailable::queue(&self)`. Both outrank any `Queue::route` registered for the job and the job's own
  `Job::queue()`/`Job::connection()` - the new `EnvelopeOverrides` primitive behind them
  (`Queue::push_with(job, overrides)` / `Queue::later_with(delay, job, overrides)`) also covers
  timeout, fail-on-timeout, max-tries, and backoff for one push. `MailFake`'s queued snapshots now
  carry the resolved `queue`, with `queued_on(...)` / `assert_queued_on(name, queue)` to assert it.
- **`Application::http_bootstrap(f)`** - an HTTP-only boot hook. It runs after `bootstrap` and only
  on the `serve` / `web:run` path, so the queue, schedule, and workflow workers and the console
  binary never run it. Worker and console container images no longer need a built frontend manifest
  to boot: `Inertia::install` fails closed in production when it is missing, and that check now only
  runs on a process that actually serves HTTP.
- **`Router::inertia(path, component, props)`** - Laravel's `Route::inertia`, for a static page
  whose handler would be one line. Registers `GET` (HEAD falls through to it) and returns a
  `RouteBuilder`, so the route can be named and given middleware. `Router::view` is retained as an
  alias.
- **SES v2 send options.** The SES transport now emits `TenantName`, `ConfigurationSetName`, and
  `ListManagementOptions` on `SendEmail`. Each has a transport-level default
  (`SesMailTransport::tenant_name` / `configuration_set_name` / `list_management`) and a
  per-message header override (`X-SES-TENANT-NAME`, `X-SES-CONFIGURATION-SET`,
  `X-SES-LIST-MANAGEMENT-OPTIONS`), with the header winning. The headers are consumed when the
  request is built and never rendered into the message.
- **`without_cookies` on every response builder.** `HttpResponse`, `Response` (via `ResponseExt`),
  `Redirect`, and `RedirectRouteBuilder` all expire a list of cookies in one call, and `Redirect`
  /`RedirectRouteBuilder` gained the single-name `without_cookie` they were missing. New
  `Cookie::forget_with(name, path, domain)` builds a deletion cookie scoped to the path and domain
  the original was set with - a plain `forget` never clears a cookie set outside `/`.
- **`Queue::fake()` stamps an envelope id on every captured push.** `pushed_with_id::<J>()` returns
  `(job, id)` pairs, and the fake now dispatches the same `JobQueueing` / `JobQueued` pair a real
  driver push does - carrying that id - so a test can correlate a captured push with what its
  listeners saw. Existing fake helpers are unchanged.
- **`UniqueJobSkipped` queue event.** `Queue::push_unique` now dispatches
  `queue::events::UniqueJobSkipped { job_name, unique_id, connection }` when it suppresses a
  duplicate, so a dedupe is observable instead of silent. The call's return value is unchanged
  (`Ok(false)`).
- **`model_keys()` on the query builder and on collections.** `User::query().model_keys().await?`
  returns every matching row's primary key without hydrating a single model, projecting the
  table-qualified key (`users.id`) so the query survives a join. `Collection::model_keys()` is the
  already-hydrated counterpart. `#[suprnova::model]` now also declares the key's Rust type as
  `EloquentModel::Key`, so both return the type `key_type` names rather than a caller-chosen
  turbofish.

### Fixed

- **PostgreSQL soft deletes now use backend-aware placeholders, and generated timestamp writes
  honor declared casts.** `delete()` and `restore()` render PostgreSQL ordinal placeholders instead
  of MySQL and SQLite `?` placeholders. Generated create, update, save, touch, and soft-delete
  writes also convert timestamps through each field's declared `Cast` storage type, so native
  `TIMESTAMPTZ` columns no longer receive text values. Thanks to
  [@i-am-v-alexander-v](https://github.com/i-am-v-alexander-v) for reporting both defects and
  submitting a fix in [PR #3](https://github.com/eas4ai/suprnova/pull/3).
- **Default workspace and Magnetar gate runs no longer require live PostgreSQL or MySQL services.**
  Backend-specific behavior suites are explicit, ignored qualification tests that still fail when
  deliberately invoked without their configured database. Reachability-only tests and permanent
  gate environment requirements were removed, so unrelated changes don't pay for external database
  setup on every verification run.

- **`PartialFilter::narrow` is now `pub`.** Its four sibling predicates (`should_include`,
  `should_include_eager`, `should_include_optional`, and the type itself) were already public, but the
  narrowing pass that makes `should_include_eager`'s `true` answer correct - trimming a resolved value
  down to the dotted paths an `only`/`except` entry actually asked for - was `pub(crate)`. A caller
  building custom partial-reload handling on top of `PartialFilter` had no public way to reproduce that
  narrowing and would ship a value whole under a dotted `only` entry even though `should_include_eager`
  reported the key as included.
- **`MailFake`'s `QueuedSnapshot` can now assert on `.on_connection(...)`.** `Queue::fake()` gained
  `assert_pushed_on_connection` in Wave 3 alongside `assert_pushed_on_queue`; `Mail::fake()` only got the
  queue half, so a mailable queued with a connection override was resolved and applied to the real
  dispatch but unassertable through the fake. New `QueuedSnapshot::connection`, `MailFake::queued_on_connection`,
  and `MailFake::assert_queued_on_connection` close the gap, mirroring `assert_queued_on`'s shape.
- **A dotted shared prop was unreachable by a bare `only` entry.** `App::inertia_share("auth.user", …)`
  followed by `router.reload({ only: ['auth'] })` returned `props: {"errors":{}}` - the share vanished
  outright. The registry stores `auth.user` as one literal key and the `Arr::set` unpacking pass only
  nests it after every prop has resolved, so the partial-reload gate saw the still-flat key and matched
  it against neither `auth` nor anything else. `only`/`except` entries are now symmetric: an entry may
  name a prop's key exactly, a path *inside* it (`user.name`, which narrows), or an **ancestor** of it
  (`auth` against the key `auth.user`, which ships the prop whole, because the caller asked for the whole
  root). A bare `except: ['auth']` drops every prop key beneath it the same way `Arr::forget` drops the
  whole subtree in Laravel's already-nested bag. The prefix must end on a segment boundary, so an
  unrelated `authAgent.user` prop is untouched by either list. Laravel never hits this because
  `Inertia::share` runs `Arr::set` at share time; Suprnova's registry cannot, since a lazy share has no
  value to nest until the request resolves it.
- **A `#[data(lazy(deferred))]` field bypassed the `?include=` allowlist.** The owner-tagged resolution
  path in `resolve_props` selected props with `Prop::is_lazy()`, which is false for anything carrying a
  flag - and a deferred field is `Visibility::Deferred`. The field therefore resolved off the ordinary
  prop path, where no include-set check exists, and shipped to any client that sent the deferred
  follow-up regardless of whether the request opted the field in. `Prop::resolve_with_owner` now gates
  every resolver-backed owner-tagged prop, flags or not, and `resolve_props` runs that gate ahead of
  every other block: a field outside `?include=` is dropped whole (no value, no `deferredProps`
  announcement), and a field named by `?include=` but off the DTO's allowlist raises its `400` before
  `X-Inertia-Partial-Data` can absorb it. Not a regression - the pre-Wave-4 code gated on the `Prop::Lazy`
  enum variant, which a `Prop::Defer` also failed - but a real hole either way.
- **`deferredProps` was re-announced on a matched partial reload.** A partial that named one deferred key
  still advertised every *other* deferred key back to the client, which then fetched them again, and
  again on the next partial. Laravel's `resolveDeferredProps` returns `[]` the moment the request is
  partial, before it inspects a single prop (`Response.php:661-663`); the block is now dropped whole on
  any matched partial. A partial reload aimed at a different component is a standard visit for this gate,
  as for every other, so its announcements are unaffected.
- **The `errors` bag filtered differently depending on where the errors came from.** The session-flashed
  bag is seeded ahead of the resolve loop and no partial-reload filter could reach it, while a handler's
  own `.with("errors", …)` went through the ordinary gates - so `only: ['errors.email']` shipped the whole
  seeded bag but a one-field handler bag, and `only: ['users']` replaced the handler's bag with the seeded
  one instead of leaving the key alone. Both paths now treat `errors` as always-visible, matching
  Laravel's middleware, which shares it as `Inertia::always(...)` and re-injects the raw value through
  `resolveAlways` after the `only`/`except` rebuild. This is the shape the client needs: it folds a
  partial response in with `{...current.props, ...response.props}`, so an empty `errors` object wipes
  messages already on screen where an unfiltered one leaves them correct. An explicit visibility flag on
  the key still wins, so `.prop("errors", Prop::eager(…).optional())` behaves optionally.
- **`Queue::fake()` can now observe per-push `EnvelopeOverrides`.** A job pushed through
  `Queue::push_with`/`Queue::later_with` was indistinguishable from a plain `Queue::push` under
  the fake - `FakePush` carried only the payload and `available_at`, so the override never left
  the facade and nothing could assert a test dispatched to the right queue or connection. New
  `queue::testing::pushed_with_overrides::<J>() -> Vec<(J, EnvelopeOverrides)>` returns each
  captured push paired with what it declared; `assert_pushed_on_queue::<J>(queue)` and
  `assert_pushed_on_connection::<J>(connection)` cover the common single-field case, mirroring
  `MailFake::assert_queued_on`. Every other entry point (`push`, `push_later`, `bulk`,
  `push_unique`, the chain/batch dispatchers) still takes no overrides and records
  `EnvelopeOverrides::default()`, so a plain push reads under the fake exactly as "no override
  declared."
- **An SSR worker that stalled mid-response body could hang a render forever.** `SsrConfig::timeout`
  bounded only the wait for response headers; once headers arrived, reading the body had no
  timeout of its own, so a worker that accepted the connection, sent headers, then stopped sending
  data left the request hanging past the configured timeout instead of falling back to CSR (or
  erroring, under `ssr_throw_on_error`). Both phases now share one deadline, so the configured
  timeout bounds the whole SSR call, as its own doc already promised.
- **Queued cookies - including the remember-me cookie `Auth::login_remember` sets - were silently
  dropped on three internal fail-closed paths in `SessionMiddleware`.** A session read failure, a
  session write failure, and a session-cookie encryption failure each returned a synthesized `500`
  directly, bypassing the pending-cookie drain that runs at the end of `handle`. Anything queued via
  `Cookie::queue` that request - including a remember-me token row already committed to the
  database - never reached the client as a `Set-Cookie` header. All three paths now drain pending
  cookies before returning, the same as a handler-returned error or a redirect. This does not cover
  an uncaught panic, matching Laravel's own queued cookies being lost to one.
- **`Queue::push_unique` now honors `Job::delay()`, matching `Queue::push`, `Queue::push_with`, and
  `Queue::bulk`.** It previously computed `available_at` from `Utc::now()` directly, so a job that
  declared a default delay (`fn delay() -> Option<Duration>`) dispatched immediately when pushed
  through `push_unique` instead of after that delay. `Queue::push_unique_later` and
  `Queue::later_unique` are unaffected - they already take an explicit timestamp or delay from the
  caller and never consult `Job::delay()`, the same rule `push_later`/`later` follow.

### Changed

- **The current development branch uses SeaORM 2.0 and requires Rust 1.94.0.** Suprnova preserves
  its Eloquent, `#[model]`, migration, and database-facade source shapes. Applications that call
  SeaORM directly must import `ExprTrait` for SeaQuery expression methods and use explicit
  `*_raw` connection methods for prebuilt `Statement` values. SeaQuery is now 1.0, and the direct
  MariaDB vector driver uses SQLx 0.9. Existing databases require no application data migration;
  fresh PostgreSQL schemas retain serial-backed primary keys.
- **Three more unused dependencies removed.** `pretty_assertions` and `qrcode` leave the framework
  crate (`totp-rs` already carries the `qr` feature, so QR provisioning for two-factor enrolment is
  unaffected), and `notify-debouncer-mini` leaves the CLI (`notify` itself stays - the `serve` and
  `generate-types` watchers use it directly). All three were confirmed unused by `cargo-udeps` plus
  a source-wide search that covers doc tests.
- **`suprnova-macros` no longer depends on `serde` or `serde_derive_internals`.** Neither was used: the
  `::serde::Serialize` paths the macros emit resolve in the downstream crate, not in the macro crate
  itself. No effect on generated code.
- **`MergeStrategy`'s `match_on` now carries more than one field name.** `Append`, `Prepend`, and `Deep`
  each widen from `match_on: Option<String>` to `match_on: Option<Vec<String>>`, so
  `InertiaResponse::merge_with` / `merge_lazy_with` can dedupe on several fields the same way
  `.prop(key, Prop::eager(v).match_on([...]))` already could - before this, the response-builder
  shortcuts were strictly less expressive than building a `Prop` directly. See Upgrading.
- **Scroll props now emit Laravel-identical `reset` and merge semantics.** `scrollProps[key].reset` is
  `true` exactly when the client named `key` in `X-Inertia-Reset`, matching Laravel's
  `resolveScrollProps` - not `true` on every visit lacking an `X-Inertia-Infinite-Scroll-Merge-Intent`
  header, as before. A scroll prop now also carries merge metadata unconditionally, defaulting to
  append: a fresh visit (no headers at all) emits `reset: false` plus a `mergeProps` entry, where it
  previously emitted `reset: true` and no merge metadata. A key in `X-Inertia-Reset` is excluded from
  `mergeProps` / `prependProps` for that response, the same exclusion a regular merge prop already had.
- **`ssr:check` now verifies the SSR worker's `GET /health` route answers 2xx**, rather than only
  confirming that something accepted a TCP connection. Every `@inertiajs/{vue3,react,svelte}/server`
  worker answers `/health` out of the box, so this needed no change on the worker side - matches
  Laravel's `Inertia\Ssr\HttpGateway::isHealthy()`.
- **The Inertia `errors` prop now carries one string per field, not an array.** A session-flashed
  validation bag renders as `{ email: "The email field is required." }` rather than
  `{ email: ["The email field is required."] }`, matching Laravel's default and Inertia's own
  `ErrorValue = string`. `InertiaConfig::with_all_errors(true)` restores the array shape. An
  `errors` prop a handler sets itself is passed through untouched, and the session flash
  (`Redirect::with_errors`, `session.pull_errors_flash()`) still stores arrays - only the rendered
  page prop changes.
- **`Model::TOUCHES` moved from an inherent const to `EloquentModel`.** The parent-touch cascade
  lives on a `Model` trait default, and a trait default can't read an inherent const.
  `Comment::TOUCHES` still resolves - it now needs `use suprnova::EloquentModel;` in scope. Models
  without a `touches` attribute get the trait's empty default.
- **`RelationEntry` gained `related_updated_at_column`.** Anything constructing a `RelationEntry` by
  hand needs the extra field; nothing in-tree does, the macro emits them all.
- **`Router::view` now rejects props that aren't a JSON object.** It previously ignored them
  silently, registering a route that rendered an empty prop bag with no diagnostic. `null` is still
  accepted as "no props"; `Router::try_inertia` is the fallible form.
- **The Inertia asset version now defaults to a hash of the Vite build manifest** instead of the
  literal `"1.0"`, so a deploy invalidates long-lived clients without anyone remembering to bump a
  string. `InertiaConfig::manifest_path(...)` re-points the resolver with it; an explicit
  `.version(...)` / `.version_with(...)` still wins. With no manifest on disk - local development -
  the version falls back to `"1.0"`, which is what every app saw before, so nothing changes until
  you build. New `VersionResolver::from_manifest(path)` exposes the resolver directly.

### Deprecated

- **`Cookie::read_encrypted` is now the v1-only legacy reader.** Code that mints with
  `Cookie::encrypted` and reads with `read_encrypted` fails at runtime on the first value written
  after this release; switch to `read_encrypted_for(name, wire)`. The un-contexted
  `CryptPurpose::Cookie` entry points are also superseded. Both removals are scheduled for 1.4.0.

### Upgrading
- **Cookie decrypt warnings now have two independent axes.** A `KeyOrigin::Previous(index)` warning means
  re-encrypt the value under the current `APP_KEY` and remove that previous key only after the rotation
  tail is gone; an `AadVersion::Legacy` warning means re-issue the cookie through the name-bound API
  before the 1.4.0 fallback removal. A value can report both.
- **`SESSION_COOKIE_PREFIX` is opt-in.** Deploy `__Host-` only with HTTPS, `SESSION_SECURE=true`,
  `SESSION_PATH=/`, and no `SESSION_DOMAIN`; local HTTP scaffolds leave it empty. `CsrfMiddleware`'s
  `with_session_config` keeps the literal `XSRF-TOKEN` name; use
  `.xsrf_cookie_name("__Host-XSRF-TOKEN")` when a client is configured for that separate name.
- **`DecryptOrigin` is now a two-axis `#[non_exhaustive]` struct.** Read its `key` and `aad` fields
  independently and keep a wildcard-compatible match strategy for the `KeyOrigin` /
  `AadVersion` enums.
- **`SessionConfig` and `CookieOptions` are now `#[non_exhaustive]`.** Struct literals and functional
  record updates in application code must move to `Type::default()` followed by public-field
  assignments or builder methods.

- **`FrameworkError` is now `#[non_exhaustive]`.** A `match` on it in your own code needs a wildcard
  arm. This is the last release in which adding a variant would have been a breaking change.
- **`MergeStrategy::Append`/`Prepend`/`Deep`'s `match_on` field is now `Option<Vec<String>>`, not
  `Option<String>`.** A call site constructing the struct-literal form directly - `MergeStrategy::Append
  { match_on: Some("id".into()) }` - no longer compiles; wrap the field name in a `Vec`:
  `Some(vec!["id".into()])`. `match_on: None` is unaffected and needs no change.
- **A matched partial reload no longer emits `deferredProps`.** Code reading `page.deferredProps`
  off a partial-reload response - a custom deferred-loading component, a test snapshot, an
  end-to-end assertion - will now find the key absent where it used to list the deferred props the
  request did not name. Read the announcements off the initial (non-partial) visit, which is where
  Laravel puts them and where the official client reads them.
- **A bare `except` entry now drops dotted prop keys beneath it.** `X-Inertia-Partial-Except: auth`
  previously left a prop registered under `auth.user` in the response, because the gate compared
  whole keys. It is dropped now. If a page relied on a bare `except` entry pruning only the exact
  key, name the exact key (`except: ['auth.user']`) or narrow with a dotted path instead.
- **`errors` ignores `only`/`except`.** A partial reload that filtered a handler-supplied
  `.with("errors", …)` prop out, or narrowed it with a dotted entry, now ships it whole. Tests
  asserting a sliced or empty `errors` object on a partial reload need updating. To keep the bag
  out of a response deliberately, flag it - `.prop("errors", Prop::eager(…).optional())` - rather
  than relying on the partial-reload lists.
- **`Prop::resolve_with_owner` gates flagged props too.** It previously resolved any prop that was
  not `Prop::is_lazy()` - an eager value *or* a resolver carrying a flag - without consulting the
  include set. It now gates every resolver-backed prop and only lets an already-materialized value
  through ungated. A `#[data(lazy(deferred))]` field consequently needs `?include=<field>` on the
  request before it resolves or is announced, the same as every other lazy flavor. Add the field to
  the request's `?include=` list, or drop the `lazy(...)` attribute if it was never meant to be
  opt-in.
- **Scroll prop `reset` no longer follows the merge-intent header.** Code that reads
  `page.scrollProps[key].reset` directly - a custom infinite-scroll component, a test snapshot - will
  see `reset: false` (plus a `mergeProps` entry) on a plain revisit that used to read `reset: true` and
  carry no merge metadata. The official `<InfiniteScroll>` component behaves differently only on a
  plain revisit: it listens for `reset` on every `router` `success` event, not only an explicit
  `router.reload()`, so a normal revisit no longer clears its accumulated state unless the server
  actually named the key in `X-Inertia-Reset`, which matches Laravel. Send `X-Inertia-Reset: <key>`
  explicitly wherever the old "any non-append/prepend visit resets" behavior was relied upon.
- **`Prop::match_on` takes `impl MatchOnFields`, not `impl Into<String>`.** The new bound is what
  lets one call name several fields (`match_on(["id", "slug"])`), and its impl list is deliberately
  closed - `&str`, `String`, `[T; N]`, and `Vec<T>` only. A blanket impl over `IntoIterator` is not
  available: coherence rejects it against the `&str` and `String` impls, since nothing stops those
  types from gaining an `IntoIterator` impl later. Three argument types that compiled before no
  longer do: `&String`,
  `Cow<'_, str>`, and `Box<str>`. Pass a `&str` at the call site instead - `match_on(name.as_str())`
  for a `&String`, `match_on(name.as_ref())` for a `Cow<'_, str>`, `match_on(&*name)` for a
  `Box<str>`.
- **A dotted `only`/`except` entry now narrows its top-level prop instead of excluding it
  entirely.** Before this fix, `X-Inertia-Partial-Data: user.name` made `should_include_eager`
  look for an exact-match `"user"` entry, found none, and silently dropped the whole `user` prop -
  a client asking for one field of `user` got nothing. Any frontend page component that happened to
  rely on that gap (treating a dotted `router.reload({ only: [...] })` as equivalent to omitting the
  key) now receives `{ user: { name: ... } }` instead. No code changes are required - this is what
  the Inertia v3 protocol already specifies the request/response contract to mean. The same fix
  applies to `should_include_optional`, and its effect is operationally bigger: a dotted `only` entry
  (`permissions.read`) now counts as an explicit request for an `Optional` or `Defer` prop's
  top-level key, which previously required a bare entry (`permissions`) to trigger at all. A request
  that used to skip that prop's resolver entirely now runs it - if the resolver hits a database or an
  external service, a client already sending dotted partial-reload requests starts issuing that work
  on requests that previously did none. Watch resolver call volume after upgrading if your app has
  `Optional`/`Defer` props with dotted partial-reload traffic.
- **`InertiaSharedData::share` now takes the page component name.** Add a `component: &str` parameter
  after `req`:
  ```diff
  -async fn share(&self, req: &dyn InertiaRequestExt) -> Result<IndexMap<String, Prop>, FrameworkError>
  +async fn share(&self, req: &dyn InertiaRequestExt, component: &str) -> Result<IndexMap<String, Prop>, FrameworkError>
  ```
  Ignore it (`_component`) if your provider doesn't need to vary by page - Laravel's `RenderContext`
  carries the same pairing (`component`, `request`) for `ProvidesInertiaProperties::toInertiaProperties`.
- **`Prop` is a struct, not an enum.** Its variants are gone; construct and read props through
  methods:
  - `Prop::Eager(v)` -> `Prop::eager(v)`
  - `Prop::EagerNone` -> `Prop::absent()`
  - `Prop::Always(v)` -> `Prop::eager(v).always()`
  - `Prop::Lazy(r)` -> `Prop::from_resolver(r)` (`Prop::lazy(closure)` is unchanged)
  - `Prop::Optional(r)` -> `Prop::from_resolver(r).optional()`
  - `match prop { Prop::Eager(v) => … }` -> `prop.as_value()`
  - `matches!(prop, Prop::Lazy(_))` -> `prop.is_lazy()`; `matches!(prop, Prop::EagerNone)` ->
    `prop.is_absent()`
  The `DeferConfig`, `MergeConfig`, `OnceConfig`, and `ScrollConfig` payload structs are removed -
  their fields are flags on `Prop` now. `Prop::is_deferred()` is renamed `Prop::has_resolver()`,
  which is what it always meant. `DeferOptions`, `OnceOptions`, `MergeStrategy`, `ScrollMetadata`,
  and every `InertiaResponse` builder method are unchanged, so an app that only uses the response
  builder needs no edits. Apps that build props by hand - typically an `InertiaSharedData`
  implementation - need the renames above.

- **This fix protects sessions you already have, not only requests from here on.** Upgrading alone
  is enough: a session cookie written by an earlier release can carry a `_previous.url` that was
  never sanitized, and `SessionData::previous_url()` now discards it on read the first time that
  session is used post-upgrade, rather than trusting it because it's already stored. You don't need
  to invalidate existing sessions, migrate the session table, or force a re-login. A request whose
  path looks protocol-relative (`//host`) also no longer updates the recorded previous URL going
  forward - if your app's `fallback!` route (or any 200-answering route reachable on an unusual
  path) ever legitimately relied on such a path becoming the `Redirect::back()` target, it won't
  anymore. Either way, the previous, safe value in the session is left in place instead (or
  `Redirect::back(fallback)`'s own fallback wins, if nothing safe was ever recorded). No code change
  is needed unless you were depending on the exact edge case this closes, which was already an
  open-redirect risk.
- **Drop the `[0]` from every `errors.<field>` binding in your pages.** With the new default shape
  `errors.email` is a string, so `errors.email[0]` renders its first character instead of the
  message. Change the TypeScript type from `string[]` to `string` at the same time. If you would
  rather not touch your pages, set `InertiaConfig::with_all_errors(true)` on the config you pass to
  `Inertia::install` and add the `errorValueType: string[]` module augmentation for
  `@inertiajs/core`. The starter frontends ship the new shape.
- **A handler that hand-rolled the redirect-back after a validation failure can delete it.** The
  bridge is automatic now; a handler that still redirects itself keeps working, because the
  middleware only acts on a `422` that carries a populated `errors` object.
- **A crashed `suprnova serve` child now respawns instead of ending the session.** If you relied on
  a crash stopping `suprnova serve` outright (a CI smoke check, a script that treats exit as
  "something's wrong"), pass `--no-restart` to restore that behaviour exactly. Retries are also
  bounded by default: a process that crashes 5 times in a row stops being retried (raise the limit
  with `--restart-tries`, or use `--no-restart` for the original one-crash-and-done behaviour).
- **`Model::TOUCHES` is no longer an inherent const.** Code that read `Comment::TOUCHES` directly
  needs `use suprnova::EloquentModel;` (or `suprnova::eloquent::EloquentModel`) in scope - the const
  moved there so the parent-touch cascade, a `Model` trait default, can read it. A `grep -rn TOUCHES`
  over your app finds every call site; most apps have none, since the const previously did nothing
  at runtime.
- **`RelationEntry` gained a field.** Only code that constructs a `RelationEntry` by hand needs a
  change - add `related_updated_at_column` to the literal. The macro-generated relation registrations
  the framework ships already emit it, so an ordinary app doing nothing but declaring relations
  through `#[suprnova::model]` is unaffected.
- **`Router::view` with non-object props now panics at boot.** It previously registered silently
  with an empty prop bag; `view` delegates to `Router::inertia`, which requires an object (or
  `null`) and panics otherwise. If a `view` call might carry non-object props, switch to
  `Router::try_inertia` and handle the `Err` - otherwise nothing changes for you.
- **The Inertia version manifest default can change your version string the moment a build
  exists.** An app or test that hardcodes `X-Inertia-Version: 1.0` keeps working only until a Vite
  manifest shows up on disk; once one does, the version becomes the manifest hash instead. If you
  need the old constant, read it from `VersionResolver::from_manifest(path)` yourself or pin
  `.version(...)` explicitly. Expect the first deploy after upgrading to force one full-page reload
  cycle for already-connected clients - one-time, and the point of the change. The no-manifest
  fallback value is exported as `suprnova::MANIFEST_VERSION_FALLBACK`, so you never need to
  hardcode `"1.0"` again.
- **Move `Inertia::install` and `global_middleware!` registration out of `bootstrap::register`.**
  Put them in a new function and pass it to `.http_bootstrap(...)` instead - the scaffold's new
  shape is a sync `register_http_stack()` called as
  `.http_bootstrap(|| async { bootstrap::register_http_stack() })`. Apps that skip this keep today's
  behavior, worker-boot failure on a missing frontend manifest included.

## 1.2.4 - 2026-08-18

### Security

- **The maintenance-mode bypass secret is compared in constant time.**
  `MaintenanceMiddleware` matched the secret URL with a plain string
  compare, which returns at the first differing byte. Because the secret is
  a bearer credential carried in the request path, that timing difference
  told an attacker how long a prefix they had guessed correctly. The
  compare now runs over the full byte length via `subtle::ConstantTimeEq`,
  short-circuiting only on a length mismatch - the same shape as the
  bypass-cookie compare next to it.

- **`rules::Url` now rejects script URIs.** The rule accepted any scheme
  `url::Url` could parse, `javascript:` and `vbscript:` included, so a
  validated URL could still be a script-execution sink when rendered into
  an `href`. It now applies Laravel's `url` rule shape
  (`Illuminate\Support\Str::isUrl`'s `^(PROTOCOLS)://HOST` pattern): the
  scheme must be on Laravel's allowlist, be followed by `://`, **and** be
  followed by a non-empty host - Laravel's host group has no `?`, so an
  absent or empty host never matches even with a listed scheme. The scheme
  list and the `://`-plus-host requirement are Laravel's verbatim; the host
  itself is parsed by the `url` crate rather than Laravel's regex, so a few
  edge cases still differ - an out-of-range port is rejected here and
  accepted there, and IDN hosts normalise differently. New
  `Url::protocols(&[...])` mirrors Laravel's `url:http,https`; `HttpUrl`
  is now literal sugar for it and keeps its own message. **Behaviour
  change:** a URL with an unlisted scheme that used to validate now
  fails - name the scheme with `Url::protocols(&["myapp"])` if you meant
  to accept it. Two more behaviour changes: `mailto:`, `data:`, and
  `tel:` are on Laravel's allowlist by name but don't carry an authority
  component, so they now fail; and `file:///etc/passwd`-style paths -
  `scheme://` with nothing between the last two slashes - now fail too,
  since an empty string isn't a host either. Both follow from Laravel's
  own `://`-plus-host rule.

- **Inertia responses now advertise `Vary: X-Inertia` everywhere.** The
  header was set only on the page-object responses themselves. Redirects,
  404s, 422s, and static responses carried none, so a shared cache keyed on
  the URL alone could serve the JSON page object to a hard browser
  navigation, or the HTML shell to an Inertia XHR. The new
  `InertiaHeadersMiddleware` - registered by `Inertia::install` as the
  outermost of the three - sets it on every response, and turns an empty
  `200` on an Inertia visit into a `303` back rather than a response the
  client rejects as non-Inertia. `InertiaVersionMiddleware` now re-flashes
  the session before its `409`, so a flashed error survives the client's
  follow-up full-page GET.

- **Three Inertia response fixes.** `InertiaResponse::location_for(&req, url)`
  returns `409` + `X-Inertia-Location` for an Inertia XHR and a plain `302` + `Location` for a hard navigation, so an OAuth or SSO bounce entered
  outside the SPA no longer dead-ends on a body-less `409`. The existing
  `location(url)` keeps its always-`409` shape. New `App::clear_history()`
  flashes the history-clear flag into the session so it survives the logout
  redirect and lands on the page that actually renders - the per-response
  `.clear_history()` marked only the redirect the browser throws away,
  leaving the previous session's encrypted history decryptable. And a
  `once` prop is now skipped only on a full Inertia visit: an explicit
  `router.reload({ only: ['stats'] })` re-resolves it instead of returning
  nothing.

- **The SES transport now sends custom message headers.** `Mail::to(..)
  .header("List-Unsubscribe", ...)` and `Mailable::headers()` were dropped
  silently under `MAIL_DRIVER=ses`: the `Content.Simple` request body had no
  `Headers` field and the raw-MIME builder never read `OutgoingMessage::
  headers`, even though every other transport forwards them. Both SES paths
  now carry them - `Headers` as SES v2's `{Name, Value}` list, raw MIME as
  real header lines - so unsubscribe links, threading headers and routing
  hints survive a driver swap. Header names are validated up front on both
  paths - CR, LF and NUL (the injection bytes, as the Mailgun transport
  already refuses) and anything that is not a valid RFC 5322 field name
  (spaces, colons, non-ASCII) - so attaching a file never changes whether a
  message is accepted.

### Fixed

- **Nested validation failures now reach the 422 body.** `#[validate(nested)]`
  failures on a nested struct or on an element of a validated `Vec<T>` were
  dropped between the validator and the response: the request was correctly
  rejected with 422, but the `errors` map came back empty, so no message
  rendered and the client could not tell which field was at fault. Nested
  failures are now flattened into Laravel's dotted notation -
  `address.street`, `items.1.name`, `order.items.2.sku` - alongside the
  top-level ones.

- **The Inertia page object's `url` keeps the query string.** `page.url` was
  the request path only, so the client recorded `/users` for a visit to
  `/users?page=2&sort=name`. Every back/forward navigation and every
  `router.reload()` then replayed the page without its pagination cursor,
  sort, or filters. It is now path plus query - the same derivation
  `InertiaVersionMiddleware` already used for `X-Inertia-Location`, so by
  default the two agree byte for byte. New
  `InertiaConfig::url_resolver(...)` overrides how the *page object* names
  the page (Laravel's `Inertia::resolveUrlUsing`); the version bounce keeps
  naming the URL that arrived, because that is the URL the browser has to
  fetch.

- **`Inertia::install` now applies its config to every response.** The
  config handed to `Inertia::install` was read for three fields and then
  dropped, so every `InertiaResponse` built without an explicit
  `.with_config(...)` rendered from `InertiaConfig::default()`. An app
  scaffolded with `--frontend react` served the Svelte entry point and no
  React refresh preamble unless `SUPRNOVA_FRONTEND` was set in the
  environment; SSR enabled on the config never reached a response; and the
  page object's asset version came from a different config than the
  version middleware's resolver. The installed config is now retained on
  the container's Inertia registry and is what `InertiaResponse::new`
  starts from. Per-response `.with_config(...)` still overrides, apps that
  never call `Inertia::install` are unchanged, and a failed (fail-closed)
  install retains nothing. As a side effect the production Vite manifest
  is now parsed once per process rather than once per response.

- **Scaffolded apps now install the Inertia protocol middlewares.** The
  `bootstrap.rs` written by `suprnova new` registered the session, locale,
  CSRF and include middlewares but never called `Inertia::install`, so a
  generated app had neither `InertiaVersionMiddleware` nor
  `Inertia303Middleware`: a browser still running the previous bundle was
  never told to reload after a deploy, and a `PUT`/`PATCH`/`DELETE` that
  redirected stayed on a `302` the client could follow with the original
  verb. The call now lands after `SessionMiddleware` - where the version
  middleware's session re-flash works - with a named `INERTIA_VERSION`
  constant to bump when assets change, and it pins the frontend the
  project was generated with (`.frontend(Frontend::React)` for
  `--frontend react`), so the HTML shell loads that framework's Vite entry
  point instead of falling back to Svelte's. The generated `.env` now sets
  `SUPRNOVA_FRONTEND` to match. The `--api` starter is unchanged; it has
  no frontend.

- **`Queue::push_unique` no longer reports a queued job as skipped.** The
  return value was computed with `matches!(outcome, Idempotent::Fresh(()))`,
  which folded `Idempotent::FreshUnfenced` into `false` - the outcome where
  the envelope *was* pushed but the dedupe lease was lost mid-push. Callers
  branching on that boolean were told a job that was about to run had been
  suppressed as a duplicate. All three outcomes are now matched exhaustively:
  a lost lease returns `true` with a `warn` naming the job and its unique
  key, and only a real duplicate returns `false`. `push_unique_later` and
  `later_unique` share the path and are fixed with it.

### Changed

- **Parity baseline moved to Laravel 13.25.0.** The 13.23.0, 13.24.0 and
  13.25.0 release notes were traced item by item to the framework's own
  surface. Everything that reached a Suprnova code path is either fixed in
  this release or has a row in [`manual/parity.md`](manual/parity.md) marked
  `not yet` or `by design no`.

### Upgrading

Two changes can alter a running app without any code change on your side.

- **Settings on the config you pass to `Inertia::install` now take effect.**
  They were read for three fields and dropped. If your install config sets
  `.ssr(...)`, SSR is now on: start the worker (`suprnova ssr:start`) before
  deploying, or drop the `.ssr(...)` call. `.entry_point`,
  `.assets_base_url`, `.default_title` and `.encrypt_history(...)` set there
  also reach the page now.

- **`rules::Url` rejects more.** Values that used to pass and no longer do:
  any scheme outside Laravel's allowlist, `javascript:` and `vbscript:`
  among them; `mailto:`, `data:` and `tel:`, which are on the allowlist but
  carry no `://` host; and `scheme://` with an empty host, such as
  `file:///path`. If you meant to accept a scheme, name it:
  `Url::protocols(&["myapp"])`.

## 1.2.3 - 2026-08-16

### Fixed

- **Datetime casts now read database-native `CURRENT_TIMESTAMP` text.**
  `AsDateTime`, `AsImmutableDateTime`, and `AsOptionalDateTime` continue to
  write canonical RFC-3339, while reads also accept PostgreSQL's
  timezone-bearing text and timezone-free SQLite/MySQL text. Timezone-free
  values are interpreted as UTC, matching the framework's UTC timestamp
  contract.

## 1.2.2 - 2026-08-14

### Fixed

- **Nullable non-text values now work across attribute-based writes on
  PostgreSQL.** Typed `Builder::update_all` and `Builder::upsert`, model-less
  `DB::table().insert/update`, and many-to-many pivot extras render explicit
  JSON nulls as SQL `NULL` while continuing to bind every non-null value. This
  preserves the target column's type instead of sending a text-typed null
  parameter that PostgreSQL rejects for bigint, integer, boolean, timestamp,
  and other non-text columns. Multi-row upserts now also reject missing or
  extra columns instead of silently converting a malformed row shape to null.
  Automatic many-to-many pivot timestamps are bound as typed UTC datetimes
  instead of text.

### Security

- **The release gate now distinguishes dormant lockfile metadata from compiled
  dependencies across the whole workspace.** Cargo records rust_decimal's
  unused optional rkyv 0.7 compatibility dependency in `Cargo.lock`; the gate
  now proves that neither rkyv nor its derive crate is reachable from any
  workspace member, feature, target, or dependency edge. The corresponding
  RustSec exception is owned, expires on 2026-11-14, and must be removed when
  rust_decimal no longer records that legacy optional dependency.

## 1.2.1 - 2026-08-09

### Changed

- **Suprnova moved to the `eas4ai` GitHub organization.** Repository URLs in
  package metadata, documentation, dependency examples, and scaffold templates
  now use `github.com/eas4ai`. New projects also use the monitored
  `shawn@eas4ai.com` author email. This release made no runtime behavior
  changes.

## 1.2.0 - 2026-08-05

### Added

- **The manual ships in seven languages.** `manual/es/`, `manual/fr/`,
  `manual/de/`, `manual/pt-BR/`, `manual/ja/` and `manual/zh-Hans/` each
  carry the full 104-chapter manual - every chapter, the table of
  contents, and this changelog - translated from the English source.
  English remains canonical: chapter structure, code blocks, identifiers,
  CLI commands and environment variables are held byte-identical to the
  source, so a translated chapter can never disagree with the English
  about what the framework does, only say it in the reader's language.

  The translations were produced and reviewed for suprnova.app, which
  renders this manual as its `/docs`. Every section carries a review
  ledger there: verdicts are recorded against content hashes of both the
  English and the translation, two independent reviewers must pass the
  exact bytes for a section to count as approved, and per-locale
  glossaries pin the terminology rulings (which terms stay English,
  which take the native word, and why). Corrections are welcome in
  either repo - a fix here reaches the site on its next sync.

## 1.1.0 - 2026-08-02

### Added

- **Per-locale fallback chains.** `LocalizationConfig` gains `parents`
  (`APP_LOCALE_PARENTS`, comma-separated `child=parent` pairs, or the
  chainable `.parent(child, parent)` builder): a locale can inherit from a
  configured sibling before falling further back to the global
  `fallback_locale` - `pt-PT` from `pt-BR`, `en-AU` from `en-GB`, and so
  on, transitively. `Lang::get`/`try_get`/`get_with`/`try_get_with`/`has`
  all walk the chain, current locale first, so this works for any
  `Translator` driver, not just the bundled one. A malformed pair, an
  invalid locale, a child named twice, or a cycle (including a locale
  naming itself as its own parent) fails loudly at config load rather
  than degrading at request time.

  Served catalogs stay chain-flattened ahead of time: `FluentTranslator`
  now builds each locale's `/_suprnova/lang/<locale>.ftl` catalog as a
  fold - the embedded framework catalog at the bottom for `en`/`en-*`
  locales, then the locale's configured parent chain, then its own
  `*.ftl` files - so a chained locale is still one self-contained file
  the browser fetches once, with no client-side chain awareness needed.
  Flattening covers configured parents only; the terminal
  `fallback_locale` is still a `Lang`-facade-level fallback, not baked
  into the served bytes.

  This makes delta-style catalogs practical: a `lang/pt-PT/` directory
  can hold only the handful of strings that actually differ from
  `lang/pt-BR/`, rather than a full duplicate catalog. The merge that
  makes it possible works at the Fluent AST level - a child's value
  replaces the parent's, attributes merge by name (an override that
  doesn't mention an attribute no longer loses it), select expressions
  replace whole (CLDR plural categories are locale-dependent, so
  variant-by-variant merging isn't coherent), and child-only entries
  append. See `manual/localization.md`'s new "Fallback chains" section
  for the full contract.

### Changed

- **`LocalizationConfig` gained the `parents` field.** `from_env()` and
  the builder are unaffected; a literal struct constructor (tests
  building a `LocalizationConfig` by hand) needs one more field.
- **Served catalog text is now serializer-normalized for every locale**,
  and intra-locale multi-file merging (several `.ftl` files in one
  locale directory) now goes through the same AST-level merge as parent
  chains rather than simple bundle-overriding. Resolved translations are
  unchanged except for the two strict improvements below; the
  underlying bytes rotate regardless - `ETag`/`?v=<hash>` rotates once
  on upgrade. The improvements: an override no longer silently drops
  the attributes it doesn't mention, and an attributes-only override no
  longer strips the message's own value (previously an error or a
  fallback resolution; it now resolves to the earlier override's
  value).

## 1.0.0 - 2026-08-02

### Added

- **Localization.** Message catalogs in `lang/<locale>/*.ftl`
  ([Fluent](https://projectfluent.org)), a `Lang` facade with the
  `__!("key", name: value)` macro, per-request locale detection
  (`LocaleMiddleware`: session → cookie → `Accept-Language` →
  `APP_LOCALE`), and locale-aware formatting for numbers, currency,
  dates, times, lists, and relative times over ICU4X. `manual/localization.md`
  is the chapter.

  The built-in validation rules stop hardcoding English. Each returns a
  keyed message (`validation-min` plus its arguments and an English
  fallback), translated once at the serialization boundary - so a Spanish
  app gets Spanish validation errors by dropping in
  `lang/es/validation.ftl`, with no rule wrapping and no forked copy of
  the framework's messages. Field names humanize through a `field-<name>`
  lookup. `Rule::passes` (and `ContextualRule` / `AsyncRule`) now return
  `Result<(), ValidationMessage>`; a custom rule's `Err("…".into())` body
  still compiles and still renders verbatim, but the signature in your
  `impl` needs the new type.

  The browser gets the same bytes the server resolved: the merged catalog
  is served at `/_suprnova/lang/<locale>.ftl` with an ETag and an
  immutable `?v=<hash>` form, the three starter kits parse it with
  `@fluent/bundle`, and `suprnova generate-types` emits a `MessageKey`
  union so renaming a message points the TypeScript compiler at every
  call site.

  Fluent rather than Laravel-style PHP arrays because one format has to
  serve both the server and the browser, and because CLDR plural
  categories are what gets Russian, Polish, and Arabic right -
  `trans_choice`'s integer ranges cannot, which is why there is no
  `trans_choice` here. Behind a default-on `localization` feature;
  `--no-default-features` still compiles and still validates, using the
  embedded English fallbacks.

- **`IntoInertiaScroll` for `Paginator`.** The trait was implemented for
  `LengthAwarePaginator` and `CursorPaginator` but not for the simple
  paginator, so `simple_paginate` results could not feed
  `Inertia::paginate` at all - despite `simple.rs`'s own module docs
  pointing at it as the URL-generation path. That left offset-paginated
  Inertia collections with a choice between a `COUNT(*)` per request and
  hand-rolling the scroll metadata. `next_page` comes from the
  `LIMIT n+1` overflow probe rather than a computed last page, there
  being no total to compute one from.

### Fixed

- **`suprnova generate-types` emitted a different file on every run.**
  The topological sort seeded its work queue by iterating a `HashMap`,
  and Rust randomises hash iteration order per process, so consecutive
  runs ordered the same interfaces differently. The output is a
  checked-in artifact, so every run produced a diff - and a generated
  file that churns for no reason is one people stop regenerating, after
  which it quietly stops describing the Rust it claims to. The directory
  walk is sorted too, so the output no longer depends on filesystem
  order either. Two runs of the same source are now byte-identical.

- **`topological_sort` did the opposite of its doc comment**, emitting
  dependents before dependencies. Harmless - a TypeScript interface may
  reference one declared later in the same file - so the comment is
  corrected rather than the order, which would have reshuffled a tracked
  file for no benefit.

## 0.9.1 - 2026-08-01

Three defects, all found by running the dogfood app under a containerised
harness rather than by reading the code. Every one of them is invisible to
a test suite that never stops a process the way production stops it.

They compound in a specific order: a rolling deploy SIGKILLs a worker
mid-job (the first), and that job then takes a reclaim path that never
counted the attempt (the second).

### Fixed

- **`schedule:work`, `queue:work` and `workflow:work` ignored SIGTERM.**
  Each selected on `tokio::signal::ctrl_c()` alone, which installs a
  SIGINT handler - so SIGTERM had no handler anywhere in the process, and
  SIGTERM is what `docker stop`, Coolify, systemd and Kubernetes send. All
  three already had a careful bounded drain behind that `select!`; none of
  it had ever executed under a supervisor. Measured before the fix: a
  `docker stop` on a `queue:work` container burned its whole 40s grace
  window and exited 137 with the in-flight job destroyed. As PID 1 - which
  is what a container runs - the kernel discards an unhandled SIGTERM
  outright, so the process did not die badly; it did not die at all until
  SIGKILL. `Server::run` already handled both signals correctly and its
  listener is now shared, which also closes a missed-signal window in the
  scheduler's loop.

- **A job that killed its worker could never be dead-lettered.** A job
  whose *handler* fails is nacked and its attempt counted, so it
  dead-letters after `max_tries`. A job that *kills its worker* - OOM,
  abort, segfault, or the SIGKILL above - settles nothing; its reservation
  merely lapses, and every driver used to redeliver it byte-identical.
  Such a job is immortal: it kills each worker that claims it, comes back
  unchanged, and kills the next one, for as long as anything restarts
  workers. All three drivers now charge the attempt where they learn a
  worker died, because swapping `QUEUE_DRIVER` must not change whether a
  poison job can be stopped. `attempts` now means "deliveries to a worker"
  rather than "handler failures" - documented in `manual/queues.md`,
  because a worker lost for unrelated reasons burns an attempt too.

- **…and the exhausted job is now dead-lettered before it is dispatched.**
  Counting the attempt was necessary and not sufficient. Every
  dead-letter decision lived in the worker's settlement path, which
  assumes the handler returns - so it never ran for exactly the jobs that
  could not return. With the driver fix alone the counter climbed
  (measured: 0 → 1 → 2 across three killed workers) and nothing acted on
  it. The budget is now spent before the handler runs. Caught only by
  re-running the container experiment after the first fix looked correct.

- **The daemons had no tracing subscriber.** `serve` gets one from
  `init_telemetry`; `queue:work`, `schedule:work`, `schedule:run` and
  `workflow:work` come through a different boot path and got nothing, so
  every `tracing::` line they emit went nowhere and `LOG_LEVEL` was inert
  for them. That is most of what they have to say - a worker
  dead-lettering a job, a scheduler skipping a tick it lost, a lock it
  could not release. In a container the only visible output was the
  startup banner, and the process looked idle while doing all of it. Two
  of the defects in this release were invisible until this was fixed.

- **A dead-letter with no failed-jobs store bound was a silent deletion.**
  The persist step sat inside `if let Some(store) = ..`, so with no store
  the arm did not match and execution fell through to the ack - quieter
  than the failure path directly above it, which at least leaves the
  reservation intact. An absent store was treated as more successful than
  a broken one. It now logs the full envelope at ERROR, because that is
  what `queue:retry` re-pushes: the difference between work recoverable by
  hand and work that ceased to exist.

- **`QUEUE_DRIVER=database` now binds a failed-jobs store.** `failed_jobs`
  is part of that driver's contract - `queue:retry` reads it and
  `Queue::retry_failed` cannot work without it - but `bootstrap_from_env`
  wired the driver and left the store unset, so a database-backed queue
  dead-lettered into nothing unless the app bound one by hand. Configurable
  via `QUEUE_FAILED_DB_TABLE`. Only for this driver: `memory` is ephemeral
  by construction and `redis` has no table to write to.

- **Redis reclaim latency now follows `--visibility-timeout`.** The flag
  sets XAUTOCLAIM's idle threshold, but a separate clock governs how often
  a consumer looks, and the driver left it at sea-streamer's 30s default -
  so `--visibility-timeout 5` really meant "up to 35 seconds". The
  interval now tracks the configured timeout, clamped to 1s..=30s so a
  short timeout cannot become an XAUTOCLAIM storm and a long one can only
  make reclaim faster than before.

### Added

- **`TaskBuilder::on_one_server()` / `on_one_server_for(ttl)`** - run a
  scheduled task exactly once per due tick across replicas. Without it
  nothing elects a leader for a tick: each `schedule:work` process
  evaluates the schedule independently, and three replicas were measured
  running every due task three times, every minute, with no variance. A
  nightly billing job on three replicas billed every customer three times.

  `without_overlapping()` does not cover this and cannot: its lock is
  keyed on the task and released when the handler returns, so a fast task
  frees it before a second replica looks. `on_one_server` keys on the task
  *and the tick* and holds the lock past the handler, letting it expire on
  TTL. The two compose.

  Opt-in, matching Laravel. Diverges from Laravel in failing closed: the
  election is only as shared as the cache behind it, so a production boot
  with `CACHE_DRIVER=memory` and a single-server task is refused, naming
  the offending tasks, with `SCHEDULE_ALLOW_MEMORY_LOCK_IN_PRODUCTION=true`
  for deployments that genuinely run one scheduler.

### Changed

- `manual/deployment.md` no longer says "run exactly one `schedule:work`
  process" as the only option, and gains a **Stopping cleanly** section
  covering the drain windows per subsystem, how to size a platform's
  termination grace above them, and why PID 1 makes a missing signal
  handler worse than it sounds.

## 0.9.0 - 2026-07-31

### Security

- **Auth issuance could only be throttled per caller, never per
  recipient.** An address-keyed limit answers "is one client noisy"; it
  cannot answer "is one mailbox being flooded". An attacker spread across
  a botnet or a single IPv6 `/64` stayed under every per-IP budget while
  filling one victim's inbox with password-reset mail, and nothing in the
  framework could express the limit that would have stopped it - a key
  function could read the path, headers, and query string, but not a
  form-encoded body, so the address was invisible on exactly the route
  that carries it.

  `identity_key` keys a bucket on the account being acted on. It reads the
  query string first and then a buffered form body, so one key function
  covers both shapes; the value is trimmed and lowercased, because
  `Alice@Example.com` reaches the same mailbox as `alice@example.com` and
  a limit bypassed by holding down shift is not a limit; and it is hashed,
  because a rate-limit backend is frequently a shared Redis with weaker
  access control than the primary database.

  Two new middleware builders support it. `key_reads_body(cap)` buffers
  the body before keying - opt-in, because buffering is work an
  unauthenticated caller gets to make you do, and a body over the cap is
  refused with 413 rather than passed through unkeyed. `only_when(pred)`
  skips a limiter entirely for requests it has nothing to say about,
  which is what keeps a stacked per-recipient budget from silently
  becoming the binding limit on routes that name no recipient.

  The dogfood app now stacks both on its issuance group: 10 per 5 minutes
  per address, 3 per 15 minutes per recipient.

A review of Torii's session, password, OAuth, and passkey paths turned up
eight defects, all fixed in the pinned fork (`suprnova-torii-rs` `968b0be`).

- **Expired sessions could be refreshed back to life.** The SeaORM session
  repository's `refresh` had no expiry predicate and unconditionally extended
  `expires_at`, and `OpaqueSessionProvider::refresh_session` skipped the
  `is_expired()` check that `get_session` performs. A token held past its
  expiry could be renewed indefinitely. Fixed at both layers. Not reachable
  through Suprnova's own surface - neither `Torii` nor the framework exposes
  session refresh - but it is public API of both crates.
- **The login form leaked which accounts exist, by timing.** Authentication
  returned as soon as the email missed, skipping Argon2 entirely: measured at
  54µs for an unknown address against 719ms for a wrong password, a ~13,000x
  gap readable over a network. Both failure paths now verify against a dummy
  hash so they cost the same. This one *was* reachable through Suprnova's
  password login.
- **The JWT `iss` claim was written but never verified.** Algorithm pinning
  was already correct - `alg: none` and HS/RS confusion were never possible -
  but the issuer was decoration, so two services sharing a signing key would
  accept each other's sessions. Now enforced when an issuer is configured.
- **A single-use PKCE verifier could be claimed twice.** Consumption was a
  read followed by a delete, so two OAuth callbacks for the same `csrf_state`
  could both read it before either delete landed. Now claimed in one
  operation - `DELETE ... RETURNING` on Postgres, a primary-key delete whose
  affected-row count picks the winner on SeaORM.
- **Expired sessions were listed as active.** `find_by_user_id` had no expiry
  filter, and expired rows survive until cleanup runs, so a "devices you're
  signed in on" screen offered users dead sessions to revoke while saying
  nothing about the live one.
- **A passkey lookup was named `authenticate`.** Torii's
  `PasskeyService::authenticate_credential` took a credential ID and returned
  the owning user, and `PasskeyAuth::authenticate` minted a session from it.
  Torii stores passkeys - it carries no WebAuthn dependency and cannot verify
  an assertion, so the only thing those calls proved was that the caller knew
  a credential ID: a value the browser sends in the clear and
  `allowCredentials` hands to anyone who can start a ceremony. Renamed to
  `find_user_by_credential` and `create_session_for_verified_credential`, both
  documenting that verification is the caller's job. Not reachable through
  Suprnova, which drives `webauthn-rs` itself (see
  `torii_integration::passkey`) and reaches Torii only for credential storage.
- **A WebAuthn challenge was replayable for its whole TTL.** Neither backend
  consumed a challenge on read, and the SeaORM `get_challenge` also ignored
  `expires_at` entirely, returning expired challenges as live. Reads now
  exclude expired rows on both backends, and a new `take_challenge` claims one
  exactly once - the same delete-decides-the-winner shape as the PKCE fix.

### Breaking

- **Azure Blob Storage and Google Cloud Storage moved behind the new
  `filesystem-azure` and `filesystem-gcs` features.** `Storage::register_azblob`,
  `register_azblob_with`, `register_gcs`, `register_gcs_with`, `AzBlobConfig`
  and `GcsConfig` no longer exist unless you enable the matching feature. If
  you use either backend, add it to your dependency:

  ```toml
  suprnova = { git = "…", tag = "v…", features = ["filesystem-gcs"] }
  ```

  You get a compile error naming the missing item, not a runtime failure.

  Both opendal service crates pull `rsa`, which carries RUSTSEC-2023-0071
  (the Marvin timing attack) with no fixed release upstream. They were the
  only crates enabling `reqsign-core/jwt`, the feature `reqsign-core`'s
  optional `rsa` sits behind, so gating them severs all three opendal paths
  to it at once. `rsa` is now *avoidable*: `--no-default-features --features
  filesystem,database-postgres` resolves without it and still has the
  storage subsystem. Previously no feature combination could shed it while
  keeping storage at all.

  A stock default build still carries `rsa` - `database-mysql` is a default
  feature and `sqlx-mysql 0.8.6` depends on it non-optionally - so the audit
  exception stays open. S3 is deliberately **not** gated: `reqsign-aws-v4`
  takes `reqsign-core` without `jwt`, so the S3 driver never contributed a
  path, and gating it would break the most-used cloud backend while removing
  nothing.

### Added

- **`suprnova --version`**, with `-v` as well as clap's default `-V`. Asking a
  CLI its version with the flag every other CLI uses should not print a usage
  error.

### Fixed

- **Two Redis operations had no upper bound.** The cache's tag flush read a
  tag's whole member set with `SMEMBERS` and deleted key by key, so a tag with
  a large membership stalled the connection and a concurrent write could be
  lost between the read and the delete; tags are now generation-based, flushed
  atomically, and scanned with a bounded `SSCAN`. The delayed-queue promotion
  pass moved every due job in one unbounded `ZRANGEBYSCORE`, so a backlog that
  came due together produced a single enormous script; it now promotes in
  batches.
- **Two shutdown drains waited forever.** `schedule:work` on Ctrl-C and the
  workflow worker after cancellation both awaited every in-flight task with no
  deadline, so one task that never returned held the process open until
  `SIGKILL` - an operator sees a daemon that "doesn't stop". Both now wait a
  bounded grace, then abort what remains and report the count.
- **The release version-pin sweep only recognised one of the two pin
  syntaxes**, so every file carrying a `cargo install --tag vX.Y.Z` line and
  no dependency snippet was never discovered. `suprnova-cli/README.md` had
  been telling readers to install v0.6.0 for three releases; `manual/cli.md`
  and `manual/cli-new.md` sat at v0.7.2; `manual/installation.md` carried
  both forms and had one bumped while the other froze. Discovery and rewrite
  now read from one pattern table, and a file's rules are derived from its
  content.
- **`cargo doc` failed for any build with `filesystem` but without
  `testing`** - seven `Storage::fake` intra-doc links could not resolve, and
  `lib.rs` denies broken links. `testing` is a default feature, so no gate
  step had ever built that combination; `check-feature-matrix.sh` now does.
- **Torii's migrations could not be replayed over their own schema**, so a
  database holding it without the `torii_migrations` tracking table - restored
  from a dump that skipped it, or migrated by hand - could not be brought under
  management. Every `Table::create()` carried `.if_not_exists()`; none of the 19
  `Index::create()` calls did, nor did the `ADD COLUMN locked_at` alter, so
  replay sailed through the tables and died on the first `CREATE INDEX`. Fixed
  in the pinned fork (`suprnova-torii-rs` `a0f956d`) via `has_index` /
  `has_column` rather than `IF NOT EXISTS`, which sea-query silently drops for
  MySQL - the syntactic fix would have left a default-featured build broken.
- **A failed Torii migration aborted the process instead of returning an
  error.** `SeaORMStorage::migrate` unwrapped the migrator and returned
  `Ok(())` unconditionally, so `init_torii`'s mapping of the failure into a
  `FrameworkError` was unreachable code.
- **An app's own `users` table silently suppressed Torii's**, because
  `.if_not_exists()` cannot tell "already mine" from "already somebody
  else's". The migration reported success and authentication failed later on
  a missing column - the reason the `--api` starter names its table
  `app_users`. Torii's migration now warns at migrate time when an existing
  `users` table lacks columns it requires, naming the columns and the remedy.
  It stays a warning rather than a hard failure so existing deployments keep
  booting.
- **The Railway and DigitalOcean deployment guides pointed the platform
  health check at a path that could probe Postgres.** Both platforms restart
  the container when that check fails, so following the advice turned a
  database blip into a restart loop across every replica. Both now use
  `/_suprnova/health/live`, with the database probed by hand from the
  console. The legacy paths still resolve; nothing already deployed needs
  changing.

## 0.8.0 - 2026-07-30

Remediation of an external red-team audit. The audit returned 19 P1
findings and a NO-GO verdict for 1.0; this release closes **all nineteen**,
plus a number of defects found while fixing them that the audit had not
named.

Several fixes deliberately turn a silent misconfiguration into a refused
boot. Read **Upgrading** before deploying - a production app that has been
running happily may not start.

### Upgrading

Three configurations that used to boot with a warning (or in silence) now
fail closed in production. Each error names the variable that unblocks it,
and each has an explicit override for the deployment where the risk is
genuinely absent.

- **A non-delivering mail driver.** `MAIL_DRIVER` unset, `log`, `memory`,
  or an unrecognised value all resolved to a transport that renders mail
  and discards it - so password resets reported success while nothing was
  sent. Override: `MAIL_ALLOW_NON_DELIVERING_IN_PRODUCTION=true`.
- **Cleartext SMTP.** Three of the four credential combinations landed on
  an unencrypted transport, and the both-unset case logged a warning and
  sent anyway. Override: `MAIL_ALLOW_INSECURE_SMTP_IN_PRODUCTION=true`.
- **The in-memory rate limiter.** Its buckets live in one process's heap,
  so behind N replicas every quota is really N× and each deploy resets
  them. Point `RATE_LIMIT_DRIVER` at `redis`, or set
  `RATE_LIMIT_ALLOW_MEMORY_IN_PRODUCTION=true` if you genuinely run one
  process. An *unrecognised* driver value fails for the same reason,
  because it fell back to memory - `RATE_LIMIT_DRIVER=Redis`, capitalised,
  is the case most likely to reach production because it looks configured.

Development, testing and staging are unchanged in all three cases. Staging
is deliberately not gated: hard-failing it pushes teams to set the
override globally, which disarms the check where it matters.

Two behaviour changes that are not boot failures:

- **`fill` and `first_or_new` reject malformed values.** A value that
  cannot decode into its field's type used to become that field's
  `Default` and return `Ok` - `fill(attrs!{ age: "abc" })` set `age = 0`
  and reported success. It now returns a `ValidationError` naming the
  field, and leaves the model untouched. Unknown columns are still skipped
  silently (Laravel parity), and numeric widening still works.
- **`/_suprnova/health?db=true` no longer returns the driver error.** The
  detail moves to the log; the body keeps `"database": "error"`. Debug
  builds still include it. Dashboards parsing `status` / `database` are
  unaffected.
- **`url::signature_has_not_expired` now requires a valid signature**, and
  is deprecated. It used to answer `true` for a forged URL - a bad
  signature is not "expired", because it never had an expiry to miss - so
  any handler guarding on it alone accepted forgeries. It is now identical
  to `has_valid_signature`. If you were using it to tell *expired* from
  *invalid* (to render "request a fresh link" rather than a 403), switch to
  `url::signature_verdict`, which returns all three states. This diverges
  from Laravel's `URL::signatureHasNotExpired`, deliberately.

Two additions that need something from you only if you opt in:

- **`QueueDriver` gained `settle` and `release`**, both with default
  implementations, so existing driver impls keep compiling unchanged.
  Implement `settle` if your backend can commit a follow-up write and an
  acknowledgement in one transaction; implement `release` if it can requeue
  a reserved message in place.
- **Batch accounting can now be durable.** `DatabaseBatchRepository` needs
  two new tables, `job_batches` and `job_batch_settlements` - add them to
  your migrations, as with `jobs` and `failed_jobs`. The schema is in
  `manual/queues.md`. Nothing changes if you stay on
  `MemoryBatchRepository`.

### Security

- **Slowloris (SEC-07).** hyper's header-read timeout was documented as
  30s but inert - it only arms when a timer is installed on the connection
  builder, and none was. A client could hold a connection, and a
  `SERVER_MAX_CONNECTIONS` permit, indefinitely. Now armed and
  configurable via `SERVER_HEADER_READ_TIMEOUT`.
- **Multipart uploads (SEC-05).** The cap applied to individual part
  payloads but not to the raw stream, so a body could exceed the limit in
  aggregate. Now capped at the stream.
- **Webhook HMAC with an empty key (SEC-08).** Both payment adapters
  accepted a blank secret, which verifies anything. Refused on both.
- **Paddle signature parsing (P2-11).** An odd-length or non-hex
  `paddle-signature` reached the pinned SDK and panicked inside it. Now
  validated first: a malformed signature is a 401.
- **Passkey enrolment and reset tokens (SEC-01, SEC-02).** Anonymous
  enrolment against an existing email, non-owner enrolment, and owner
  enrolment without recent reauth are each refused with distinct statuses.
  A password login now stamps the reauth window.
- **`dev:tls` (SEC-10).** A project could choose the CA the command
  trusts.
- **Generated Docker Compose (P2-12).** Published Postgres and Redis on
  all interfaces with credentials committed in this repository. Now bound
  to loopback with per-scaffold generated passwords, `.env` written 0600,
  and symlinked targets refused.
- **Health endpoint (P2-01, CI-05).** It decided whether to query the
  database with `query.contains("db=true")` - a substring test, so
  `?nodb=true` ran the probe too. Now parsed properly. The 503 no longer
  embeds the driver error, which named hosts, ports, schemas and versions.
- **Credential issuance throttling (P2-02).** The four auth-issuance
  routes in the reference app carried no rate limit at all, and the one
  route that did keyed its bucket on the raw `x-forwarded-for` header -
  which any client can vary per request to get a fresh bucket. Both fixed;
  the issuance budget is shared across the four routes so rotating between
  them does not multiply it.
- **A redelivered chain step re-pushed its successor under a new id
  (DATA-02b, partial).** Settlement pushes the next chain link *before*
  acking, deliberately: acking first means a crash in that window loses
  the chain permanently, and a duplicate is recoverable where silent loss
  is not. But the successor's envelope got a fresh `Uuid::new_v4()` on
  every push, so the duplicate produced by that trade was
  indistinguishable from a legitimate new step - to the driver, to an
  outbox, and to the handler.

  That last one is the real cost. The framework's delivery contract is
  at-least-once and its answer to duplicates is "handlers must be
  idempotent" - but a handler keyed on `env.id`, the only identifier it
  receives, could not satisfy that contract for a chained job, because the
  duplicate arrived under a new id every time. The contract was
  unsatisfiable by construction.

  The successor's id is now a UUIDv5 derived from its predecessor's, which
  is stable across that predecessor's own redeliveries. A redelivered step
  re-pushes the id it pushed before. No schema change, no new field, no
  new dependency.

  This makes the duplicate **detectable**, which is the primitive the rest
  of DATA-02b was missing. It does not make the push atomic with the ack
  (that needs the outbox), and nothing yet rejects the duplicate on the way
  in. Both remain open.
- **Signed URLs verified one URL and executed another (SEC-04).** The
  canonical form collapsed query pairs into a map, so a repeated key kept
  only its **last** value - while `Request::query_param` returned the
  **first**. A legitimately signed `?user=victim` could therefore be
  replayed as `?user=attacker&user=victim` with the original signature
  untouched: verification canonicalised over `victim` and passed, and the
  handler acted on `attacker`.

  The canonical form now carries every pair, sorted by `(key, value)`, so
  the signature covers the exact multiset of parameters - adding,
  removing, or substituting any value breaks the HMAC. A repeated
  `signature` or `expires` is refused outright, since two of either leaves
  no non-arbitrary answer to which one governs.

  `Request::query_param` now resolves a repeated key to its last value,
  matching `query_params` and `Context::query_param`; it was the only one
  of the three that disagreed, and that disagreement was the other half of
  the defect. **Existing signed links keep working** - with no repeated
  keys the payload bytes are unchanged, which a test pins, because a
  canonical-form change that silently invalidated every outstanding
  password-reset link would be worse than the bug.

  Six regression tests, including both attack orderings, a legitimately
  repeated key that must still sign and verify, and the reordering
  guarantee. *Not* changed: `signature_has_not_expired` still reports a
  forged signature as "not expired". That is Laravel's behaviour, was
  settled deliberately as a documentation fix, and has its own test
  pinning it against a well-meaning "correction".
- **RBAC under Postgres.** Verified against a real Postgres rather than
  SQLite alone.
- **Four RustSec advisories eliminated, not renewed.** The Pinecone driver
  was rewritten against Pinecone's REST API, dropping `pinecone-sdk 0.1.2` -
  whose newest release dates from 2024-09-06 - and with it
  `tonic 0.11 → rustls 0.22 → rustls-webpki 0.102` and
  RUSTSEC-2026-0049 / -0098 / -0099 / -0104. All four were fixed upstream
  in `rustls-webpki >= 0.103.13`, which this workspace already resolved
  for its other TLS users; one abandoned crate held the tree on the
  vulnerable line. `.cargo/audit.toml` is down from five ignores to one.
  See **Changed** for what this means for the driver's API.
- **Audit exceptions now expire.** Every entry in `.cargo/audit.toml`
  carries an `OWNER` and an `EXPIRES` date, and `scripts/check-audit.sh`
  fails the release gate on a missing owner, a missing or unparseable
  date, or a lapsed one. `cargo audit` has no notion of an expiring
  ignore, so one added "temporarily" stayed until somebody re-read the
  file. The remaining entry (RUSTSEC-2023-0071, `rsa`, which has no fixed
  release at all) is owned and dated.
- **Reachability claims are checked, not asserted.**
  `scripts/check-feature-matrix.sh` resolves real dependency trees and
  asserts that no build - including `--all-features`, which is what
  `cargo audit` actually reads - contains `pinecone-sdk`,
  `rustls-webpki 0.102.x` or `tonic 0.11.x`. An exception justified by a
  comment nothing verifies stops being true the first time someone adds a
  dependency.

### Fixed

- **Every release on a database-backed queue was silently a no-op.**
  `JobOutcome::Released` - a busy `WithoutOverlapping` lock, a rate-limiter
  backoff - was implemented as "push a copy, then ack the original". The
  envelope id is the `jobs` table's primary key, so the copy collided with
  the row still holding the live reservation and the push failed with
  `UNIQUE constraint failed: jobs.id`. The worker then correctly declined
  to ack, so the requested delay was never applied, no `JobReleased` event
  fired, and the job simply parked until visibility expiry redelivered it.
  Releases are now one driver call, done in place.
- **A partial batch dispatch orphaned the jobs it had already queued
  (DATA-02).** When a `driver.push` failed mid-loop,
  `PendingBatch::dispatch` deleted the batch row - but the envelopes
  already in the queue were still stamped with that batch id, so each of
  them settled against a batch that no longer existed, returning
  `Err(batch not found)` on every delivery, forever. The batch is now
  settled instead: undispatched jobs are recorded as failures and the batch
  is cancelled, so the queued ones settle normally and the terminal
  callbacks still fire.
- **Nothing tested that `url::has_valid_signature` rejects a forged URL.**
  Found while verifying the SEC-04 fix: the entire framework suite passed
  with the primary signed-URL guard rewritten to accept any signature.
- **A scaffolded app could not migrate its database or build its image
  (REL-01b).** Neither scaffold declared `default-run`, so all nine CLI
  wrappers that shell out to `cargo run` failed on a fresh project. The
  generated Dockerfile had five independent defects - a missing lockfile
  COPY, `npm ci` without a lock, a cache stage stubbing one of two
  declared binaries, a frontend build copied from a path vite never
  creates, and a missing `frontend/src/pages` copy that
  `inertia_response!` validates at compile time. A stock scaffold's image
  could not build.
- **`docker:init` emitted one Dockerfile for every project type.** On an
  `--api` project its first instruction, `COPY frontend/package.json`,
  failed outright. API projects now get a frontend-free Dockerfile.
- **SQL placeholders (DATA-01).** Rendered per backend rather than
  assuming one dialect.
- **Queue settlement (DATA-02a, P2-06c).** Follow-ups settle before the
  reservation is acked, and a lock-release error no longer converts an
  already-succeeded job into a retry.
- **A cancelled batch fired `Catch`, never `Then`.**
- **`Builder::clone` silently dropped the eager-load plan (P2-09a).**
  `User::query().with("posts")` cloned anywhere - pagination, `count()`,
  any scope that clones - returned rows with no relations and no error.
- **Presence rosters lost members (P2-08).** The roster was snapshotted
  before subscribing, so anyone joining in that window appeared in
  neither, permanently.
- **Pinecone serialised every index acquisition (P2-14).** The write lock
  was held across two network round trips, and `tokio`'s fair `RwLock`
  meant one cold index stalled every warm one.
- **The type watcher discarded bursts (P2-13).** Leading-edge debounce
  regenerated on the first file of a burst and dropped the rest with no
  trailing run, so the last save never took effect.
- **`ssr:check` could hang, and tried one address (P2-13).** DNS ran
  outside the timeout entirely, and only the first resolved address was
  tried - so a host with an AAAA record and no IPv6 route reported the
  worker down while it was listening on v4.
- **`suprnova serve` installed `cargo-watch` unpinned (P2-13).** Now
  `--locked` with a major-version bound.
- **The release bumper rewrote five READMEs and nothing else.** Four
  manual chapters and a public doc comment pinned tags that no release
  ever updated - the doc comment was two releases stale. Discovery now
  replaces the hand-maintained list, and the smoke test greps the bumped
  tree independently rather than trusting the bumper's own verify step.
- **`db:sync` treated the database schema as trusted input (CLI-01).**
- **`migrate:fresh` is gated behind `--force` plus a typed confirmation
  (CLI-02)**, in the app binary as well as the CLI.
- **The `log` mail driver now logs the whole message**, as Laravel does,
  and no longer writes bearer links to the log in production.

### Added

- **Atomic terminal settlement (`QueueDriver::settle`, DATA-02).** The
  chain successor and the acknowledgement now commit together on
  `DatabaseQueueDriver`, closing the window where a crash between them
  either lost the rest of a chain or ran its next step twice. The
  reservation-keyed delete doubles as a fence: a worker whose visibility
  expired mid-run commits nothing and reports `Settled::Stale`, so it
  cannot enqueue work for a message another consumer now owns. Drivers that
  cannot do this answer `Settled::Unsupported` and keep the documented
  push-before-ack ordering.
- **`DatabaseBatchRepository` (DATA-02).** Batch accounting survives a
  restart, and `pending_jobs`/`failed_jobs` are derived from settlement
  rows keyed `(batch_id, job_id)` rather than stored and decremented - so a
  redelivered job cannot drive a batch to "finished" while its other jobs
  are still running, and the guard holds across processes rather than
  within one.
- **`/_suprnova/health/live` and `/_suprnova/health/ready`.** Liveness
  touches nothing; readiness probes dependencies. Wiring a database check
  into a liveness probe turns a database blip into a rolling restart of
  every replica, which the single previous endpoint invited.
  `/_suprnova/health` keeps working exactly as documented.
- **`SERVER_HEALTH_READINESS_TOKEN`.** Optional shared secret for the
  readiness probe, compared in constant time. Without it, readiness
  answers 404 - indistinguishable from an unrouted path, because it *is*
  the router's own 404. Unset by default so existing probes keep working.
- **`MAIL_SMTP_ENCRYPTION`** - `starttls` | `tls` | `none`, with `ssl` and
  `null` accepted as Laravel-compatible aliases. Unset derives from the
  credentials, reproducing the previous behaviour exactly. This also makes
  implicit TLS on port 465 reachable: the transport supported it, but no
  combination of environment variables could select it.
- **`SERVER_MAX_CONNECTIONS` and `SERVER_HEADER_READ_TIMEOUT`** documented
  in `manual/env-vars.md`, where they had been missing entirely.

### Changed

The audit's own conclusion was that the gate passed in 470s and caught
none of the 19 P1s. Most of this release's test work is aimed at that.

- **Postgres runs in the gate.** Twelve tests across six files had never
  executed. Two of them turned out to aim `DROP TABLE` at whatever
  Postgres was on `localhost:5432` by default, and neither had ever
  initialised `Crypt`, so both failed the first time they ran.
- **Scaffold assertions read the bytes a user receives**, after
  substitution, rather than the template source. Found an API project
  shipping a doc comment naming a database literally `{package_name}`, and
  a `.env.example` advertising five mail keys the framework never reads.
- **Queue fault injection.** ACK loss, redelivery, lease lapse and partial
  dispatch are driven by a decorator that fails a named operation on a
  named call, so every case is deterministic rather than a sleep race.
- **Payment adapters have negative tests.** Stripe's `verify()` had never
  been exercised with a *valid* signature, so every rejection path that
  depends on reaching the HMAC comparison was unproven.
- **The Pinecone driver speaks REST.** *Breaking, behind the
  off-by-default `vector-pinecone` feature.* Motivation is under
  **Security**; the surface changes are:
  - `client()` is gone - there is no `PineconeClient` any more. Replacing
    it are `control_plane_get`, `control_plane_post` and `data_plane_post`,
    which reach *any* Pinecone endpoint with your own request and response
    types over the driver's authenticated, host-resolved transport. That
    is strictly more reach than the old trapdoor had.
  - `json_to_metadata` → `metadata_from_json`, and metadata is now
    `serde_json::Map` rather than `prost_types::Struct`. `decode_match_fields`
    → `decode_match`, taking a `PineconeMatch`. `namespace()` returns
    `&str`.
  - New: `with_control_plane`, `with_api_version`, `with_index_host`
    (pins a known host and skips the control-plane round trip),
    `index_host`, and the `PineconeVector` / `PineconeMatch` wire types.
  - `from_env` still reads `PINECONE_API_KEY` and
    `PINECONE_CONTROLLER_HOST`, and now also `PINECONE_API_VERSION`.
  - The REST API version is pinned, not floated - `2025-04`, the version
    the driver's request and response shapes were written against.
  - Nothing serializes any more. The old driver cached one `Index` per
    name behind a `tokio::Mutex` because `pinecone-sdk` exposed it only
    behind `&mut self`; the new one caches a host string and shares
    `reqwest`'s connection pool.
  - A host learned from the control plane is always contacted over
    `https`, whatever scheme the response carries.
  - `Debug` is implemented by hand with the API key redacted, so a
    `#[derive(Debug)]` on a struct holding a driver can't print it.
- **Wire-contract tests for Pinecone.** The live integration tests need a
  `PINECONE_API_KEY` and so cannot run in the gate - which left a REST
  rewrite's field names (`topK`, `includeMetadata`, `vectorCount`) resting
  on nothing. Thirteen tests now drive the driver against a local
  `wiremock` fake and assert the exact method, path, headers and JSON body
  it puts on the wire, plus that a non-2xx is never decoded as a result
  and that an error message never carries the API key. They pin the driver
  to Pinecone's *documented* contract; only the `#[ignore]`d tests can
  confirm the documentation matches the live service.

## 0.7.2 - 2026-07-28

### Fixed

- **`generate-types` resolves nested prop structs without derives.** 0.7.1's
  generator degraded any prop field whose type didn't derive
  `InertiaProps`/`Data` to `unknown` - so re-running the generator (or the
  `suprnova serve` watcher) over a project with a committed types file
  replaced real interfaces like `Array<AdminArticleRow>` with `unknown` and
  broke type-checking across the app. Plain structs defined anywhere in
  `src/` now resolve to their real interfaces, transitively from the prop
  roots; `unknown` (with a warning) is reserved for types the project
  genuinely doesn't define - external crate types, enums, tuple structs.

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

## 0.7.1 - 2026-07-27

A defect-fix pass over 0.7.0's queue routing, from a full post-release review.

### Fixed

- **Chained jobs no longer lose their declared queue.** `ChainLink` captured a
  job's `max_tries`, `timeout`, and `backoff` at chain-build time but not its
  `Job::queue()`, so a job that landed on its declared queue when pushed
  directly landed on `default` when dispatched as part of a chain - the "job"
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
  whether or not the job is routed - a 0.7.0 binary against an un-migrated
  table fails **every push**, filtered or not. The 0.7.0 section below and
  `manual/queues.md` are corrected: on the database driver the `ALTER TABLE`
  is required for every deployment, and it must run before binaries roll
  (older binaries list their columns explicitly, so migrating first is safe).

- **README no longer advertises a `#[job]` macro.** No such macro exists -
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
  dimension is unaffected - it is honored end to end. Per-connection drivers
  remain future work.
- `ChainLink` gained a public `queue: Option<String>` field, which breaks
  struct-literal construction of chain links. Links built through
  `ChainLink::from_job` - the normal path - are unaffected.

### Upgrading

Coming from ≤ 0.6.x on the database queue driver, apply the 0.7.0 migration
below **before** rolling binaries; it is required for every deployment on
that driver, not just ones using `--queue`. 0.7.1 itself needs no migration.

## 0.7.0 - 2026-07-26

### Security

- **Upgraded `ammonia` to 4.1.4 (RUSTSEC-2026-0213).** Versions through 4.1.3
  allow XSS via SVG `animate` and `set` animation tags. `ammonia` is the
  sanitizer at the end of Suprnova's markdown pipeline
  (`comrak` → `syntect` → `ammonia`), so any app rendering user-supplied
  Markdown through `content` was exposed. The advisory was published
  2026-07-21 - after v0.6.5 shipped - so **every release up to and including
  v0.6.5 is affected**. Upgrading the framework is the fix; no application
  code changes are required.

### Added

- **Queue routing.** Jobs can be dispatched to a specific queue and connection,
  and workers can be dedicated to specific queues - the Laravel 13
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
  queue - a worker told to drain `billing` that quietly drains everything is
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
  what previous versions wrote - the frozen wire-format test passes unchanged,
  there is no `schema_version` bump, and mixed-version fleets interoperate
  during a rolling upgrade.
- `WorkerConfig` gained a `queues: Vec<String>` field (empty = drain everything,
  the previous behaviour).
- Removed `ROADMAP.md`. Its design principles live in `manual/introduction.md`,
  the working agreement in `manual/contributions.md`, and the deployment and
  scale-out material in `manual/deployment.md`; the shipped/planned checklists
  had gone stale. `README.md`'s pointer to it for "the relationship to upstream"
  was already dangling - that attribution lives in `LICENSE`.
- Scaffold frontends now pin `@inertiajs/{svelte,react,vue3}` at `^3.6.1`
  (from `^3.4.0`). The 3.4.0 → 3.6.1 range is client-side only - audited against
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
column - `push` names it in its `INSERT` whether or not the job is routed, so
an un-migrated table fails every push. Migrate first, then roll binaries
(older binaries list their columns explicitly and ignore the new one, so that
order is safe):

```sql
ALTER TABLE jobs ADD COLUMN queue TEXT NULL;
CREATE INDEX idx_jobs_queue ON jobs(queue);
```

*(Corrected in 0.7.1 - this note originally claimed unfiltered deployments
needed no migration.)*

## 0.6.5 - 2026-07-21

### Added

- **Hosted one-off Checkout in the Stripe adapter.** `Checkout::start_session`
  with `SessionMode::OneOff` and non-empty `price_refs` now creates a hosted
  Checkout Session (`mode=payment`, one line item per price ref,
  `allow_promotion_codes=true`) and returns
  `SessionPayload::StripeCheckoutRedirect`. The `amount_hint`-only Elements
  path is unchanged; the two shapes are picked per request.
- **Stripe Managed Payments (merchant-of-record) support.**
  `StripeProvider::with_managed_payments(true)` - or
  `STRIPE_MANAGED_PAYMENTS=true` in `from_env()` - sends
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
  per session id (`script_session_status()` - unscripted known sessions
  report `Open`, unknown ids `NotFound`), and implements `Promotions` with
  recorded requests (`recorded_promotion_requests()`).

## 0.6.4 - 2026-07-17

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

## 0.6.3 - 2026-07-15

### Added

- **Typed raw reads can stay on a transaction's pinned connection.**
  `Transaction::backend()` exposes the active backend and
  `Transaction::query_all(Statement)` executes typed aggregate or custom SQL
  through the transaction while preserving `QueryExecuted` instrumentation.
  Applications no longer need a pool-level query or private executor access
  when a lock-scoped decision depends on computed result columns.

## 0.6.2 - 2026-07-15

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

## 0.6.1 - 2026-07-15

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

## 0.6.0 - 2026-07-10

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
  `eas4ai/opendal` commit
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

## 0.5.10 - 2026-07-03

### Fixed

- **`generate-types` no longer drops self-referencing structs.** A struct with a
  field that references its own type (a tree node with `children: Vec<Self>`,
  e.g. a threaded-comment view) created a self-edge in the type-dependency
  graph, pinning its in-degree above zero so Kahn's topological sort never
  emitted it - leaving every interface that referenced it with a dangling type
  name that failed `svelte-check`/`tsc`. Self-edges are now stripped before
  sorting, and any structs trapped in a reference cycle (mutual recursion) are
  emitted in arbitrary order rather than dropped, since TS interfaces may
  reference one another regardless of declaration order.

## 0.5.9 - 2026-07-01

### Added

- **`MAIL_FROM_NAME` - optional display name on auth-flow emails.** The
  email-verification, password-reset, and password-changed mailables now render
  their `From` header as `"Name <address>"` when `MAIL_FROM_NAME` is set (read
  at send time so it survives the queue's serde round-trip). `MAIL_FROM` stays a
  bare address; leaving `MAIL_FROM_NAME` unset or blank keeps the previous
  bare-address behavior. No change to any call site - the mailables read the env
  var themselves.

## 0.5.8 - 2026-06-30

### Fixed

- **`generate-types` route helpers are always valid TypeScript.** When several
  routes in a module share one handler (e.g. a `static_files::serve` whitelist
  mapping many favicon/asset URLs), the first kept the handler name and the rest
  got a key derived from the route path - but the path was only partly
  sanitized (`/ { } -` → `_`), so a file extension leaked a `.` into the key:
  `favicon_16x16.png: (...) => ...`. That is member access, not a property name,
  so `tsc`/`svelte-check` rejected the generated `routes.ts`. Derived keys are
  now sanitized to legal identifiers - every non-alphanumeric character becomes
  `_` and a leading digit is prefixed - so `favicon-16x16.png` → `favicon_16x16_png`
  and `2fa.json` → `_2fa_json`. Unique handler names are untouched.

## 0.5.7 - 2026-06-30

### Fixed

- **`generate-types` no longer emits dangling type references.** A prop field
  whose type is a struct that doesn't derive `InertiaProps`/`Data` (or an
  external type the generator can't see) was emitted as a bare identifier - e.g.
  `user: UserInfo` - producing TypeScript that fails `tsc`/`svelte-check`
  because that interface is never written. Such references now degrade to
  `unknown` (`user: unknown`; `Vec<T>` → `Array<unknown>`; `Option<T>` →
  `unknown | null`), so generated output always type-checks, and
  `generate-types` prints a warning naming the unresolved type and the field
  that references it, with the fix (derive `InertiaProps`/`Data` on it).
  Generic parameters and resolved nested InertiaProps/Data types are
  unaffected.

## 0.5.6 - 2026-06-29

### Changed

- **Sign in with Apple: RS256 JWKS verification.** Bump `suprnova-apple-rs` to
  v0.3.1 - Apple ID tokens are now verified against Apple's published JWKS
  (RS256) instead of being trusted structurally.

## 0.5.5 - 2026-06-28

### Added

- **`MagicLink` token purpose.** New `MagicLink` variant on the auth-flow
  `TokenPurpose` enum, for passwordless magic-link sign-in tokens.

## 0.5.4 - 2026-06-28

### Changed

- **Composable OAuth completion.** Split the generic OAuth completion into
  `verify_oauth_identity` (verify + resolve the identity) and a thin `complete`,
  so apps can verify an OAuth identity without triggering the full
  session-completion side effects.

## 0.5.3 - 2026-06-28

### Fixed

- **Correct workspace version metadata.** v0.5.2 was tagged and pushed before
  its `Cargo.toml` version bump was staged, so the pushed v0.5.2 tag still reads
  `version = "0.5.1"`. v0.5.3 re-cuts the release with the correct workspace
  version - no code change (the v0.5.2 OAuth split is unaffected).

## 0.5.2 - 2026-06-28

### Changed

- **Composable Apple completion.** Split Apple Sign-In completion into
  `verify_apple_identity` + a thin `complete_apple`, mirroring the generic OAuth
  split. (Note: the pushed v0.5.2 tag carries a stale `0.5.1` version field -
  fixed in v0.5.3.)

## 0.5.1 - 2026-06-28

### Changed

- **Renamed Apple crate.** Repoint the Apple dependency to the renamed
  `suprnova-apple-rs` repository.

## 0.5.0 - 2026-06-28

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

## 0.4.1 - 2026-06-26

### Performance

- Pre-size `MiddlewareChain` to eliminate per-request `Vec` reallocations.

### Fixed

- Make the maintenance down-file path collision-proof under parallel test runs.

### Docs

- Compile-check the framework's doc examples (`ignore` → `no_run`); reconcile
  the distribution notes with the tagged GitHub Releases; ignore the whole
  `docs/` tree.

## 0.4.0 - 2026-06-22

### Changed

- **Distribution is git-tracked; you don't pin to tags.** Scaffolded apps
  depend on `suprnova = { git = "…/suprnova.git" }` and track the default
  branch; pull updates with `cargo update -p suprnova`. Versions are published
  as tagged GitHub Releases (`v0.4.0`, …) for the changelog, but `Cargo.lock`
  already pins the exact resolved commit - so builds stay reproducible without
  hand-pinning a `tag` or `rev`. The installation docs no longer present
  commit-pinning as the update path.

## 0.3.0 - 2026-06-21

### Added

- **Query instrumentation for Eloquent reads** - `Builder::get`, `Model::find`,
  `find_many`, and `all` now emit `QueryExecuted`, so model SELECTs and
  eager-load queries surface in `DB::listen` and the in-memory query log
  alongside writes and raw queries. Adds the instrumented
  `ExecutorChoice::statement_all` read terminal.
- **Resource-route authorization** - `ResourceRoutes::authorize_resource::<U, R>()`
  attaches the conventional ability check to every generated resource route as
  per-route middleware (Laravel `authorizeResource` parity). The action→ability
  map is `index`/`show` → `view`, `create`/`store` → `create`,
  `edit`/`update` → `update`, `destroy` → `delete`. One call gates the whole
  seven-action surface instead of relying on every controller body to remember
  a `Gate::authorize`.
- **Atomic rate-limit hit** - `RateLimiter::hit_and_check(key, max, decay)`
  increments a fixed window and tests it in a single round-trip, returning
  whether the bucket is now over its limit (`i64::MAX` means unlimited).
- **Constant-time comparison helper** - `constant_time_eq(a, b)` (subtle-backed)
  for webhook signature verification; `WebhookHandler::verify` docs now mandate
  constant-time digest comparison.
- **Inertia client to 3.4.0** - the Svelte/React/Vue scaffolds now pin
  `@inertiajs/{svelte,react,vue3}` at `^3.4.0` (from `3.1.1`), picking up
  `router.poll` modes, dynamic `usePoll`, `Inertia.once`, the InfiniteScroll
  cancel fix, and awaited Form `onSuccess`. The server already emits the full
  3.4.0 page-object and header surface (once-props, the prepend/deep-merge
  scroll family, `matchPropsOn`, rescued/shared props), so this is a
  client-currency bump with no protocol change.
- **Optional connection cap** - `SERVER_MAX_CONNECTIONS` (and the programmatic
  `Server::max_connections(n)`) bounds concurrently active connections with a
  semaphore on the accept loop, applying back-pressure at the TCP level. Unset -
  or `0` - leaves connections unbounded (the default, unchanged). A backstop to
  pair with a reverse proxy and `LimitNOFILE`, not a replacement for upstream
  rate limiting.
- **Opt out of redirect-following** - `RequestBuilder::no_redirects()` routes a
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
- **Auth** closes a passwordless-login timing oracle - a matched-but-passwordless
  account given a password now runs a fixed-cost verify, across both the Eloquent
  and database user providers - and `dummy_verify` drives the configured hasher so
  the unmatched-user path is constant-time.
- **Eloquent** validates column identifiers on the `pluck` / `value` /
  `pluck_keyed` / `sole_value` and `sum` / `avg` / `min` / `max` projection
  paths.
- **Payments** - the mock provider's verifier fails closed outside a development
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
  of queries - the tail segment loads in one batched IN query across all
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
  `after_sending`, re-checked on the worker - previously only the synchronous
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

## 0.2.0 - 2026-06-21

Adds role-based access control, a Markdown content / docs-rendering pipeline, and
native static-file serving.

### Added

- **Tier-2 RBAC** - `HasRoles` trait; roles + permissions with a
  `role_has_permissions` join; `PermissionMiddleware` / `RoleMiddleware` (both
  fail-closed / default-deny); the `CreateRbacTables` migration; and
  `create_role` / `create_permission` / `give_permission_to_role` helpers.
- **Content rendering** - Markdown rendering and a docs-build pipeline:
  `MarkdownRenderer`, `build_docs`, `DocsCatalog` / `DocsChapter`, heading
  extraction and `slugify_heading`. Rendered HTML is sanitized
  (comrak + syntect + ammonia).
- **Native static-file serving** - `StaticFiles::public()` fallback handler for
  serving a `public/` directory at the web root, replacing hand-rolled per-asset
  whitelist controllers in apps.

### Fixed

- Freshly generated apps inherit a framework-level `time = 0.3.47` compatibility
  pin, avoiding Rust 1.96 coherence conflicts from `time 0.3.48` in fresh
  scaffold dependency resolutions.

### Documentation

- Documented the two shipped starter kits - **Nebula** (Breeze-tier auth) and
  **Pulsar** (product site + community) - across the manual, README, and roadmap;
  restructured the roadmap around the shipped surface; and reconciled version
  references throughout the docs.

## 0.1.0 - 2026-06-10

The initial Suprnova release. Suprnova is a Laravel-inspired web
framework for Rust, forked from Kit and taken in its own direction.
Today's parity target is Laravel 13.x.

This release uses the git distribution model: framework consumers depend
on `suprnova = { git = "https://github.com/eas4ai/suprnova.git" }`,
and the CLI installs with `cargo install --git`.

### Added

#### HTTP, routing, and middleware

- `Router` with route groups, prefixes, parameter constraints, named routes
- Compile-time-validated route registration via the `routes!` macro
- Resource routing (`Router::resource`) producing the seven standard routes
- Signed URLs (`url::signed_route` / `url::temporary_signed_route` free
  functions, plus `Redirect::signed_route` / `Redirect::temporary_signed_route`)
- Redirect helpers - `Redirect::to`, `Redirect::back`, `Redirect::route`,
  `Redirect::with_input`, `Redirect::with_errors`, `with_flash`
- Middleware trait with global, group, and per-route layers
- Built-in middleware - CORS, CSRF, session, request timeout,
  request ID, throttle / login throttle, signed-URL verify,
  authenticated, email-verified, brute-force
- Abort helpers (`abort`, `abort_unless`, `abort_if`)
- `suprnova::handle_request(...)` - public adapter to serve a single
  hyper request against a router + middleware chain

#### Inertia.js frontend bridge

- `#[derive(InertiaProps)]` with TypeScript type emission
- `inertia_response!` macro with compile-time component validation
- Three first-class starter frontends - **Svelte 5** (runes-on),
  **React 19**, **Vue 3.5** - all on Inertia 3.1.1 + Vite 8 + Tailwind v4
- Partial reloads (`only` / `except`), deferred props, persistent
  layout, encrypted history, scroll preservation
- `Inertia::paginate(component, key, paginator)` for paginator → Inertia
  prop wiring

#### Eloquent-style ORM (over SeaORM)

- `#[suprnova::model]` attribute macro that emits a SeaORM entity and
  the user-facing Eloquent struct in one shot
- Full `Model` trait - `create`, `find`, `find_or_fail`, `find_many`,
  `all`, `query`, `save`, `update`, `delete`, `force_delete`, `refresh`,
  `fresh`, `replicate`, `replicate_into`, `increment`/`decrement`,
  `destroy`, `is`/`is_not`, `to_array`/`to_json`
- Fillable / guarded mass-assignment with `Attrs` envelope
- 22 attribute casts - booleans, integers, floats, dates, enums,
  hashed, encrypted, JSON, collections, money, datetime with timezone
- Accessors / mutators via `#[suprnova::model]`
- Auto-timestamps (`created_at`, `updated_at`)
- Soft deletes (`deleted_at`) with `force_delete`, `restore`, `trashed`,
  `only_trashed`, `with_trashed`
- Eleven relation kinds - `HasOne`, `HasMany`, `BelongsTo`,
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
- `Collection<M>` Laravel surface - `pluck`, `key_by`, `group_by`,
  `where_in`, `first_where`, `contains_where`, `partition`, etc.
- Three paginators - `paginate` (length-aware), `simple_paginate`,
  `cursor_paginate` - all serializing to Laravel-shape JSON
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
- Email verification flow - `EmailVerification`,
  `EnsureEmailVerifiedMiddleware`, signed verification URLs,
  `EmailVerificationMail`
- Password reset flow - `PasswordReset`, throttled tokens,
  `PasswordChangedMail`, `PasswordResetLinkSent` event
- Two-factor TOTP - enroll, verify, recovery codes, replay protection
- Brute-force / login throttle - IP + identifier keyed,
  `LoginThrottleMiddleware`
- Remember-me cookies with stable opaque tokens
- Six auth events - `LoginAttempted`, `LoggedIn`, `Authenticated`,
  `LoggedOut`, `PasswordResetLinkSent`, `EmailVerified`
- Browser sessions backed by the Torii fork at
  `github.com/eas4ai/suprnova-torii-rs`

#### Authorization

- `Gate` facade - `define`, `allows`, `denies`, `authorize`, `any`,
  `none`, `check` (sync + async variants)
- `#[policy(Model)]` macro for policy registration
- Resource-route auto-authorization

#### Payments

- Provider-agnostic five-trait surface - `Checkout`, `Payment`,
  `Subscription`, `CustomerStore`, `WebhookHandler`
- `PaymentProvider` umbrella trait + capability-querying via `as_payment()`
- DB mirror - `customers`, `subscriptions`, `subscription_items`,
  `payments`, `refunds`, `payment_webhook_events` (UNIQUE for idempotency)
- Flow-tagged `SessionPayload` enum (one-shot vs subscription)
- Two reference adapters as workspace crates -
  `suprnova-payments-stripe` (gateway, full `Payment` impl),
  `suprnova-payments-paddle` (Merchant of Record, no `Payment` impl)
- Mock provider for tests

#### Queue, jobs, batches, chains

- `Job` trait - `handle`, `max_tries`, `backoff`, `timeout`,
  `fail_on_timeout`
- `Queue::push`, `Queue::push_later`, `Queue::push_unique`,
  `Queue::push_unique_later`
- Drivers - `sync`, `null`, `redis`, `database`
- `JobMiddleware` trait - six built-in middleware
- Batches and chains - `Queue::batch(jobs).dispatch()`, fluent chain
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

- Typed event dispatcher - `EventFacade::dispatch`,
  `EventFacade::listen<E, L>`, `EventFacade::forget`
- Cancellable saving/updating events (return `EventResult::cancel`)
- Queueable listeners

#### Filesystem

- `Storage::disk("name")` with multi-driver support - local, S3,
  Azure, GCS via OpenDAL
- Move, copy, exists, size, mime, last-modified, prepend/append
- Streaming uploads and downloads

#### Cache

- `Cache::store("name")` + driver registration
- Drivers - memory, redis (with bounded connect-timeout), database, file
- `remember`, `forever`, `tags`, atomic increment/decrement, locks

#### Vector DB

- `VectorDriver` trait with four drivers - in-memory, Qdrant
  (UUID-5 ID mapping), Pinecone (native string IDs), MariaDB native
  `VECTOR(N)` + HNSW indexes (11.7+)
- Cosine / dot / euclidean distance

#### Console binary and CLI

- Per-project `console` binary - Rust analogue of `php artisan`,
  runs user-defined commands via `#[suprnova::console::command]`
- `#[derive(Command)]` for typed arguments
- `suprnova` CLI - `new`, `serve`, `migrate`, `db:sync`,
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
- Lifecycle hooks - `authorize`, `after_validation`,
  `after_validation_async`

#### Database drivers

- SeaORM-backed support for SQLite, Postgres, MySQL, MariaDB
- URL-based driver detection
- Migration system + `migrate`, `migrate:rollback`, `migrate:status`,
  `migrate:fresh`, `migrate:refresh`

#### HTTP client

- `Http` facade - `get` / `post` / `put` / `patch` / `delete`
  returning a `RequestBuilder`; `.send().await` produces a
  `ClientResponse`
- rustls TLS, 30s default timeout, `suprnova/<version>` user-agent
- `json` / `form` / `body` / `header` / `bearer_token` / `basic_auth`
  / `timeout` chainable methods
- `RequestBuilder::retry(max_attempts, base_backoff)` - exponential
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
- HTTP test helpers - `Test::get`, `Test::post`, JSON / form / multipart
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
  `suprnova = { git = "https://github.com/eas4ai/suprnova.git" }`;
  CLI via `cargo install --git`. Nothing is published to crates.io.
