# Changelog

A readable, per-version log of what changed in Suprnova. Each version
section is that version's release record. A version is released when its
version commit and matching `v<version>` tag are pushed atomically. Newest first.

## Unreleased

### Fixed

- **The maintenance-mode bypass secret is compared in constant time.**
  `MaintenanceMiddleware` matched the secret URL with a plain string
  compare, which returns at the first differing byte. Because the secret is
  a bearer credential carried in the request path, that timing difference
  told an attacker how long a prefix they had guessed correctly. The
  compare now runs over the full byte length via `subtle::ConstantTimeEq`,
  short-circuiting only on a length mismatch — the same shape as the
  bypass-cookie compare next to it.

- **`rules::Url` now rejects script URIs.** The rule accepted any scheme
  `url::Url` could parse, `javascript:` and `vbscript:` included, so a
  validated URL could still be a script-execution sink when rendered into
  an `href`. It now matches Laravel's `url` rule exactly
  (`Illuminate\Support\Str::isUrl`'s `^(PROTOCOLS)://HOST` pattern): the
  scheme must be on Laravel's allowlist, be followed by `://`, **and** be
  followed by a non-empty host - Laravel's host group has no `?`, so an
  absent or empty host never matches even with a listed scheme. New
  `Url::protocols(&[...])` mirrors Laravel's `url:http,https`; `HttpUrl`
  is now literal sugar for it and keeps its own message. **Behaviour
  change:** a URL with an unlisted scheme that used to validate now
  fails - name the scheme with `Url::protocols(&["myapp"])` if you meant
  to accept it. Two more behaviour changes: `mailto:`, `data:`, and
  `tel:` are on Laravel's allowlist by name but don't carry an authority
  component, so they now fail; and `file:///etc/passwd`-style paths -
  `scheme://` with nothing between the last two slashes - now fail too,
  since an empty string isn't a host either. Both match Laravel exactly.

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
  `InertiaVersionMiddleware` already used for `X-Inertia-Location`, so the
  two can no longer disagree. New `InertiaConfig::url_resolver(...)`
  overrides the derivation (Laravel's `Inertia::resolveUrlUsing`).

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
  returns `409` + `X-Inertia-Location` for an Inertia XHR and a plain `302`
  + `Location` for a hard navigation, so an OAuth or SSO bounce entered
  outside the SPA no longer dead-ends on a body-less `409`. The existing
  `location(url)` keeps its always-`409` shape. New `App::clear_history()`
  flashes the history-clear flag into the session so it survives the logout
  redirect and lands on the page that actually renders - the per-response
  `.clear_history()` marked only the redirect the browser throws away,
  leaving the previous session's encrypted history decryptable. And a
  `once` prop is now skipped only on a full Inertia visit: an explicit
  `router.reload({ only: ['stats'] })` re-resolves it instead of returning
  nothing.

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
  verb. The call now lands after `SessionMiddleware` — where the version
  middleware's session re-flash works — with a named `INERTIA_VERSION`
  constant to bump when assets change, and it pins the frontend the
  project was generated with (`.frontend(Frontend::React)` for
  `--frontend react`), so the HTML shell loads that framework's Vite entry
  point instead of falling back to Svelte's. The generated `.env` now sets
  `SUPRNOVA_FRONTEND` to match. The `--api` starter is unchanged; it has
  no frontend.

### Changed

- **Parity baseline moved to Laravel 13.25.0.** The 13.23.0, 13.24.0 and
  13.25.0 release notes were traced item by item to the framework's own
  surface. Everything that reached a Suprnova code path is either fixed in
  this release or has a row in [`manual/parity.md`](manual/parity.md) marked
  `not yet` or `by design no`.

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
