# 変更履歴

Suprnovaで何が変わったかを、バージョンごとに読みやすくまとめたログです。各バージョンのセクションは、そのバージョンのリリース記録です。バージョンは、バージョンコミットと対応する`v<version>`タグがアトミックにプッシュされたときにリリースされます。新しい順に並んでいます。

## 未リリース

## 1.3.0 - 2026-08-24

> The v1.3.0 release notes are intentionally kept in English to preserve the complete normative record.

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

### セキュリティ

- **メンテナンスモードのバイパス用シークレットが、定数時間で比較されるようになりました。**`MaintenanceMiddleware`は、素の文字列比較でシークレットのURLを照合しており、これは最初に異なるバイトで戻ります。シークレットはリクエストのパスに載って運ばれるベアラー認証情報であるため、そのタイミングの差が、どれだけの長さのプレフィックスを正しく推測できたかを攻撃者に教えていました。比較は今では`subtle::ConstantTimeEq`を介してバイト長の全体にわたって走り、長さの不一致のときにだけショートサーキットします - すぐ隣にあるバイパスクッキーの比較と同じ形です。

- **`rules::Url`が、スクリプトURIを拒否するようになりました。**このルールは、`url::Url`がパースできるあらゆるスキームを受け付けており、そこには`javascript:`と`vbscript:`も含まれていたため、検証済みのURLが、`href`へレンダリングされたときに依然としてスクリプト実行のシンクになり得ました。今では、Laravelの`url`ルールの形（`Illuminate\Support\Str::isUrl`の`^(PROTOCOLS)://HOST`パターン）を適用します: スキームはLaravelの許可リストに載っていなければならず、`://`が続かなければならず、**さらに**空でないホストが続かなければなりません - Laravelのホストのグループには`?`がないため、リストに載ったスキームであっても、ホストが存在しないか空である場合は決してマッチしません。スキームのリストと、`://`に加えてホストという要件は、Laravelそのままです。ホスト自体は、Laravelの正規表現ではなく`url`クレートによってパースされるため、いくつかのエッジケースは依然として異なります - 範囲外のポートは、ここでは拒否され、あちらでは受け付けられますし、IDNホストの正規化も異なります。新しい`Url::protocols(&[...])`は、Laravelの`url:http,https`をミラーします。`HttpUrl`は今ではその文字どおりのシュガーであり、独自のメッセージを保ちます。**挙動の変更:** これまで通っていた、リストにないスキームのURLは、今では失敗します - それを受け付けるつもりだったのなら、`Url::protocols(&["myapp"])`でそのスキームを名指ししてください。挙動の変更はさらに2つあります。`mailto:`、`data:`、`tel:`は、名前としてはLaravelの許可リストに載っていますが、authority成分を運ばないため、今では失敗します。そして`file:///etc/passwd`形式のパス - 最後の2つのスラッシュの間に何もない`scheme://` - も、空文字列もまたホストではないため、今では同様に失敗します。どちらも、Laravel自身の`://`に加えてホストという規則から導かれます。

- **Inertiaのレスポンスが、あらゆる場所で`Vary: X-Inertia`を広告するようになりました。**このヘッダーは、ページオブジェクトのレスポンス自体にしか設定されていませんでした。リダイレクト、404、422、そして静的なレスポンスはどれも運んでいなかったため、URLだけをキーにした共有キャッシュが、ハードなブラウザナビゲーションに対してJSONのページオブジェクトを、あるいはInertiaのXHRに対してHTMLシェルを配信してしまう可能性がありました。新しい`InertiaHeadersMiddleware` - `Inertia::install`によって、3つのうち最も外側として登録されます - は、それをすべてのレスポンスに設定し、Inertiaの訪問での空の`200`を、クライアントが非Inertiaとして拒否するレスポンスではなく`303`の戻りへ変えます。`InertiaVersionMiddleware`は今では、自分の`409`の前にセッションを再フラッシュするため、フラッシュされたエラーは、クライアントの追いかけのページ全体のGETを生き延びます。

- **Inertiaのレスポンスに関する3つの修正。**`InertiaResponse::location_for(&req, url)`は、InertiaのXHRには`409` + `X-Inertia-Location`を、ハードナビゲーションには素の`302` + `Location`を返すため、SPAの外側で始まったOAuthやSSOの跳ね返しが、ボディのない`409`で行き止まりになることはもうありません。既存の`location(url)`は、常に`409`という形を保ちます。新しい`App::clear_history()`は、履歴クリアのフラグをセッションへフラッシュするため、それはログアウトのリダイレクトを生き延び、実際にレンダリングされるページへ着地します - レスポンスごとの`.clear_history()`は、ブラウザが捨てるリダイレクトにしか印を付けておらず、直前のセッションの暗号化された履歴を復号可能なまま残していました。そして`once`のプロップは、今では完全なInertiaの訪問でのみスキップされます: 明示的な`router.reload({ only: ['stats'] })`は、何も返さないのではなく、それを再解決します。

- **SESのトランスポートが、カスタムのメッセージヘッダーを送るようになりました。**`Mail::to(..).header("List-Unsubscribe", ...)`と`Mailable::headers()`は、`MAIL_DRIVER=ses`の下でサイレントに捨てられていました: `Content.Simple`のリクエストボディには`Headers`フィールドがなく、生のMIMEのビルダーは`OutgoingMessage::headers`を一度も読んでいませんでした - 他のあらゆるトランスポートはそれらを転送しているにもかかわらずです。SESの両方の経路が、今ではそれらを運びます - `Headers`はSES v2の`{Name, Value}`のリストとして、生のMIMEは実際のヘッダー行として - そのため、購読解除のリンク、スレッド化のヘッダー、ルーティングのヒントは、ドライバーの差し替えを生き延びます。ヘッダー名は、両方の経路で先に検証されます - CR、LF、NUL（Mailgunのトランスポートがすでに拒否しているのと同じ、注入用のバイトです）と、有効なRFC 5322のフィールド名でないもの（空白、コロン、非ASCII）です - そのため、ファイルを添付することが、メッセージが受け付けられるかどうかを変えることは決してありません。

### 修正

- **PostgreSQL の論理削除でバックエンド対応のプレースホルダーを使用するようになり、生成されるタイムスタンプ書き込みで宣言済みのキャストが考慮されるようになりました。** `delete()` と `restore()` は、MySQL および SQLite の `?` プレースホルダーではなく、PostgreSQL の序数プレースホルダーを生成します。生成される作成、更新、保存、touch、論理削除の書き込みでも、各フィールドで宣言された `Cast` ストレージ型を介してタイムスタンプを変換するため、ネイティブの `TIMESTAMPTZ` カラムにテキスト値が渡されることはなくなりました。両方の不具合を報告し、[PR #3](https://github.com/eas4ai/suprnova/pull/3) で修正を提出してくださった [@i-am-v-alexander-v](https://github.com/i-am-v-alexander-v) に感謝します。
- **デフォルトの workspace および Magnetar gate の実行で、稼働中の PostgreSQL または MySQL サービスが不要になりました。** バックエンド固有の動作スイートは、明示的に ignored とされた適格性テストであり、構成済みのデータベースなしで意図的に実行すると引き続き失敗します。到達可能性のみを確認するテストと恒久的な gate 環境要件が削除されたため、無関係な変更の検証を実行するたびに外部データベースをセットアップする必要はありません。

- **入れ子になったバリデーションの失敗が、422のボディへ届くようになりました。**入れ子になった構造体や、バリデーションされる`Vec<T>`の要素に対する`#[validate(nested)]`の失敗は、バリデーターとレスポンスの間で落とされていました: リクエストは正しく422で拒否されていたものの、`errors`のマップは空で返ってきていたため、メッセージは何もレンダリングされず、クライアントはどのフィールドに問題があったのかを知ることができませんでした。入れ子の失敗は今では、トップレベルのものと並んで、Laravelのドット区切りの記法 - `address.street`、`items.1.name`、`order.items.2.sku` - へ平坦化されます。

- **Inertiaのページオブジェクトの`url`が、クエリ文字列を保つようになりました。**`page.url`はリクエストのパスだけだったため、`/users?page=2&sort=name`への訪問に対して、クライアントは`/users`を記録していました。その結果、あらゆる戻る/進むのナビゲーションと、あらゆる`router.reload()`が、ページネーションのカーソル、ソート、フィルタなしでそのページを再生していました。今ではパスにクエリを加えたものになります - `InertiaVersionMiddleware`が`X-Inertia-Location`のためにすでに使っていたのと同じ導出であるため、デフォルトではこの2つはバイト単位で一致します。新しい`InertiaConfig::url_resolver(...)`は、*ページオブジェクト*がそのページをどう名指しするかを上書きします（Laravelの`Inertia::resolveUrlUsing`です）。バージョンの跳ね返しは、到着したURLを名指しし続けます。それが、ブラウザの取得しなければならないURLだからです。

- **`Inertia::install`が、その設定をすべてのレスポンスへ適用するようになりました。**`Inertia::install`へ渡された設定は、3つのフィールドについて読まれた後、捨てられていました。そのため、明示的な`.with_config(...)`なしで構築されたすべての`InertiaResponse`は、`InertiaConfig::default()`からレンダリングされていました。`--frontend react`でスキャフォルドされたアプリは、環境に`SUPRNOVA_FRONTEND`が設定されていない限り、Svelteのエントリポイントを配信し、Reactのrefreshのプリアンブルを出しませんでした。設定の上で有効にしたSSRは、レスポンスへ一度も届きませんでした。そして、ページオブジェクトのアセットバージョンは、バージョンのミドルウェアのリゾルバとは別の設定から来ていました。インストールされた設定は今では、コンテナのInertiaレジストリ上に保持され、`InertiaResponse::new`はそこから出発します。レスポンスごとの`.with_config(...)`は引き続き上書きし、`Inertia::install`を一度も呼ばないアプリは変わらず、（フェイルクローズで）失敗したインストールは何も保持しません。副次的な効果として、本番のViteのマニフェストは今では、レスポンスごとではなくプロセスごとに一度だけパースされます。

- **スキャフォルドされたアプリが、Inertiaプロトコルのミドルウェアをインストールするようになりました。**`suprnova new`が書き出す`bootstrap.rs`は、セッション、ロケール、CSRF、includeの各ミドルウェアを登録していましたが、`Inertia::install`を一度も呼んでいませんでした。そのため、生成されたアプリは`InertiaVersionMiddleware`も`Inertia303Middleware`も持たず、直前のバンドルをまだ走らせているブラウザは、デプロイの後にリロードするよう決して伝えられず、リダイレクトする`PUT`/`PATCH`/`DELETE`は、クライアントが元の動詞で追いかけてしまう`302`のままでした。この呼び出しは今では`SessionMiddleware`の後に - バージョンのミドルウェアのセッションの再フラッシュが機能する場所に - 着地し、アセットが変わったときに上げるための名前付きの`INERTIA_VERSION`定数を伴い、プロジェクトが生成されたときのフロントエンドをピン留めします（`--frontend react`なら`.frontend(Frontend::React)`）。そのため、HTMLシェルは、Svelteのものへフォールバックするのではなく、そのフレームワークのViteのエントリポイントをロードします。生成される`.env`は今では、それに合わせて`SUPRNOVA_FRONTEND`を設定します。`--api`のスターターは変わりません。フロントエンドを持たないからです。

- **`Queue::push_unique`が、キューに載ったジョブをスキップされたと報告しなくなりました。**戻り値は`matches!(outcome, Idempotent::Fresh(()))`で計算されており、これは`Idempotent::FreshUnfenced`を`false`へ畳み込んでいました - エンベロープは*プッシュされた*が、重複排除のリースがプッシュの途中で失われた、という結果です。その真偽値で分岐する呼び出し元は、これから走ろうとしているジョブが、重複として抑制されたと伝えられていました。3つの結果は今では、すべて網羅的にマッチされます: リースを失った場合は、ジョブとその一意キーを名指しする`warn`とともに`true`を返し、本物の重複だけが`false`を返します。`push_unique_later`と`later_unique`は、同じ経路を共有しており、一緒に修正されています。

### 変更

- **現在の開発ブランチは SeaORM 2.0 を使用し、Rust 1.94.0 を必要とします。** Suprnova は Eloquent、`#[model]`、マイグレーション、データベースファサードのソース形式を維持します。SeaORM を直接呼び出すアプリケーションでは、SeaQuery の式メソッド用に `ExprTrait` をインポートし、事前構築済みの `Statement` 値には明示的な `*_raw` 接続メソッドを使用する必要があります。SeaQuery は 1.0 になり、MariaDB の直接ベクタードライバーは SQLx 0.9 を使用します。既存のデータベースにアプリケーションデータの移行は不要です。新規の PostgreSQL スキーマでは、引き続き serial ベースの主キーが使用されます。

- **パリティの基準が、Laravel 13.25.0へ移りました。**13.23.0、13.24.0、13.25.0のリリースノートを、項目ごとにフレームワーク自身の表面まで追跡しました。Suprnovaのコード経路に届いたものはすべて、このリリースで修正されているか、[`manual/parity.md`](manual/parity.md)の中に`not yet`または`by design no`と印の付いた行を持っています。

### アップグレード

2つの変更が、あなたの側でのコード変更なしに、動作中のアプリを変えうるものです。

- **`Inertia::install`へ渡す設定の項目が、効くようになりました。**それらは3つのフィールドについて読まれ、捨てられていました。あなたのインストール用の設定が`.ssr(...)`を設定している場合、SSRは今ではオンです: デプロイの前にワーカーを起動する（`suprnova ssr:start`）か、`.ssr(...)`の呼び出しを外してください。そこで設定した`.entry_point`、`.assets_base_url`、`.default_title`、`.encrypt_history(...)`も、今ではページへ届きます。

- **`rules::Url`が、より多くを拒否します。**これまで通っていて、もう通らなくなる値は次のとおりです。Laravelの許可リストの外にあるあらゆるスキーム（`javascript:`と`vbscript:`もその中に含まれます）。許可リストには載っているものの`://`のホストを運ばない`mailto:`、`data:`、`tel:`。そして`file:///path`のような、ホストが空の`scheme://`です。あるスキームを受け付けるつもりだったのなら、それを名指ししてください: `Url::protocols(&["myapp"])`。

## 1.2.3 - 2026-08-16

### 修正

- **日時キャストがデータベースネイティブの`CURRENT_TIMESTAMP`テキストを読み取れるようになりました。** `AsDateTime`、`AsImmutableDateTime`、`AsOptionalDateTime`は引き続き正規化されたRFC-3339を書き込み、読み取りではタイムゾーン付きのPostgreSQLテキストと、タイムゾーンを持たないSQLite/MySQL値も受け付けます。タイムゾーンを持たない値はUTCとして解釈されます。

## 1.2.2 - 2026-08-14

### 修正

- **属性ベースの書き込み全体で、nullableな非テキスト値をPostgreSQL上で扱えるようになりました。** 型付きの`Builder::update_all`と`Builder::upsert`、モデルを使わない`DB::table().insert/update`、多対多ピボットの追加属性は、明示的なJSON nullをSQLの`NULL`として出力し、nullでないすべての値は引き続きバインドします。これにより、PostgreSQLがbigint、integer、boolean、timestamp、およびその他の非テキストカラムに対して拒否する、テキスト型付きnullパラメータを送る代わりに、対象カラムの型が保持されます。複数行upsertは、形が不正な行の欠落または余分なカラムを黙ってnullに変換せず、拒否するようになりました。多対多ピボットの自動タイムスタンプは、テキストではなく型付きUTC日時としてバインドされます。

### セキュリティ

- **リリースゲートは、ワークスペース全体で休眠中のlockfileメタデータとコンパイル対象の依存関係を区別するようになりました。** Cargoはrust_decimalの未使用のオプション依存関係であるrkyv 0.7互換依存関係を`Cargo.lock`に記録します。ゲートは、rkyvもそのderive crateも、ワークスペースのどのメンバー、feature、target、依存関係エッジからも到達可能でないことを証明するようになりました。対応するRustSec例外は管理対象となっており、2026-11-14に期限切れになります。rust_decimalがこのレガシーなオプション依存関係を記録しなくなった時点で削除する必要があります。

## 1.2.1 - 2026-08-09

### 変更

- **SuprnovaはGitHubの`entrepeneur4lyf` organizationから`eas4ai`へ移動しました。** パッケージメタデータ、ドキュメント、依存関係の例、scaffoldテンプレートにあるリポジトリURLは、`github.com/eas4ai`を使うようになりました。新しいプロジェクトでは、監視対象の作者メールアドレス`shawn@eas4ai.com`も使われます。このリリースによるruntime動作の変更はありません。

## 1.2.0 - 2026-08-05

### 追加

- **マニュアルが7言語で提供されるようになりました。** `manual/es/`、`manual/fr/`、
  `manual/de/`、`manual/pt-BR/`、`manual/ja/`、`manual/zh-Hans/` のそれぞれが、全104章のマニュアル - すべての章、目次、そしてこの変更履歴 - を英語のソースから翻訳して収めています。英語は引き続き正典です: 章の構成、コードブロック、識別子、
  CLIコマンド、環境変数はソースとバイト単位で同一に保たれているため、翻訳された章がフレームワークの動作について英語と食い違うことはあり得ません - 読者の言語で語り直すだけです。

  翻訳は suprnova.app のために作成・レビューされました。同サイトはこのマニュアルを
  `/docs` としてレンダリングしています。各セクションはそこでレビュー台帳を持ちます:
  評決は英語と翻訳の両方のコンテンツハッシュに対して記録され、セクションが承認と数えられるには、2人の独立したレビュアーが正確に同じバイト列を承認しなければなりません。また、言語ごとの用語集が用語の裁定 - どの用語を英語のまま残し、どれを母語の語にするのか、そしてその理由 - を固定します。修正はどちらのリポジトリでも歓迎です - ここでの修正は、次回の同期でサイトに届きます。

## 1.1.0 - 2026-08-02

### 追加

- **ロケール単位のフォールバックチェーン。**`LocalizationConfig`に`parents`が追加されました（`APP_LOCALE_PARENTS`という、カンマ区切りの`child=parent`ペア、またはチェーン可能な`.parent(child, parent)`ビルダーです）。ロケールは、グローバルな`fallback_locale`へさらにフォールバックする前に、設定済みの兄弟ロケールを継承できます - `pt-BR`からの`pt-PT`、`en-GB`からの`en-AU`、というように、推移的に続きます。`Lang::get`/`try_get`/`get_with`/`try_get_with`/`has`はすべて、現在のロケールを先頭にしてこのチェーンをたどるため、これはバンドル済みのものだけでなく、あらゆる`Translator`ドライバーで機能します。不正な形式のペア、無効なロケール、二重に名付けられた子、あるいは循環（ロケールが自分自身を親として名付ける場合を含む）は、リクエスト時に劣化する代わりに、設定読み込み時にはっきりと失敗します。

  配信されるカタログは、事前にチェーンをフラット化した状態を保ちます: `FluentTranslator`は、各ロケールの`/_suprnova/lang/<locale>.ftl`カタログを畳み込みとして構築するようになりました - `en`/`en-*`ロケール向けの埋め込みフレームワークカタログを一番下に置き、続いてそのロケールの設定済み親チェーン、そして最後に自身の`*.ftl`ファイルという順です - そのため、チェーンされたロケールであっても、ブラウザが一度だけフェッチする自己完結型の単一ファイルのままであり、クライアント側でチェーンを意識する必要はありません。フラット化がカバーするのは設定済みの親のみです。末端の`fallback_locale`は、依然として`Lang`ファサードレベルのフォールバックであり、配信されるバイト列には焼き込まれません。

  これにより、差分形式のカタログが実用的になります: `lang/pt-PT/`ディレクトリは、`lang/pt-BR/`から実際に異なるわずかな文字列だけを保持でき、カタログ全体を複製する必要はありません。それを可能にするマージは、Fluent ASTレベルで動作します - 子の値が親の値を置き換え、アトリビュートは名前でマージされ（アトリビュートに言及しないオーバーライドが、そのアトリビュートを失うことはもうありません）、選択式は丸ごと置き換わり（CLDRの複数形カテゴリはロケール依存のため、バリアント単位のマージは筋が通りません）、子だけが持つエントリは追加されます。完全な契約については、`manual/localization.md`の新しい「フォールバックチェーン」セクションを参照してください。

### 変更

- **`LocalizationConfig`に`parents`フィールドが追加されました。**`from_env()`とビルダーは影響を受けません。リテラルな構造体コンストラクタ（`LocalizationConfig`を手作りで組み立てるテストなど）は、フィールドがもう1つ必要になります。
- **配信されるカタログのテキストは、すべてのロケールについてシリアライザで正規化されるようになりました。**ロケール内の複数ファイルマージ（1つのロケールディレクトリ内に複数の`.ftl`ファイルがある場合）も、単純なバンドルの上書きではなく、親チェーンと同じASTレベルのマージを通るようになりました。解決される翻訳結果は、以下の2つの厳密な改善点を除いて変わりません。ただし裏側のバイト列はいずれにせよ入れ替わります - `ETag`/`?v=<hash>`はアップグレード時に一度だけローテートします。改善点は次のとおりです: オーバーライドが、言及していないアトリビュートをサイレントに失うことはもうありません。また、アトリビュートのみのオーバーライドが、メッセージ自身の値を消してしまうこともなくなりました（以前はエラーになるか、フォールバック解決になっていました。今では、より前のオーバーライドの値に解決されます）。

## 1.0.0 - 2026-08-02

### 追加

- **ローカライゼーション。**`lang/<locale>/*.ftl`内のメッセージカタログ（[Fluent](https://projectfluent.org)）、`__!("key", name: value)`マクロを備えた`Lang`ファサード、リクエストごとのロケール検出（`LocaleMiddleware`: セッション → クッキー → `Accept-Language` → `APP_LOCALE`）、そしてICU4Xを介した数値、通貨、日付、時刻、リスト、相対時間のロケール対応フォーマットです。`manual/localization.md`がその章です。

  組み込みのバリデーションルールは、英語をハードコードしなくなりました。それぞれが、キー付きメッセージ（`validation-min`とその引数、そして英語のフォールバック）を返し、シリアライズ境界で一度だけ翻訳されます - そのため、スペイン語のアプリは`lang/es/validation.ftl`を投入するだけでスペイン語のバリデーションエラーを得られ、ルールのラップもフレームワークのメッセージのフォーク版も不要です。フィールド名は、`field-<name>`のルックアップを通じて人間可読な形になります。`Rule::passes`（および`ContextualRule`/`AsyncRule`）は、`Result<(), ValidationMessage>`を返すようになりました。カスタムルールの`Err("…".into())`という本体は、引き続きコンパイルが通り、そのままの形でレンダリングされますが、あなたの`impl`のシグネチャは新しい型を必要とします。

  ブラウザは、サーバーが解決したものと同じバイト列を受け取ります: マージ済みのカタログは、ETagとイミュータブルな`?v=<hash>`形式を伴って`/_suprnova/lang/<locale>.ftl`で配信され、3つのスターターキットはそれを`@fluent/bundle`でパースし、`suprnova generate-types`は`MessageKey`のユニオン型を出力するため、メッセージをリネームするとTypeScriptコンパイラがすべての呼び出し箇所を指し示します。

  Laravel流のPHP配列ではなくFluentを選んだのは、1つのフォーマットがサーバーとブラウザの両方に対応しなければならないからであり、またロシア語、ポーランド語、アラビア語を正しく扱えるのはCLDRの複数形カテゴリだからです - `trans_choice`の整数レンジではそれができません。だからこそ、ここには`trans_choice`はありません。デフォルトで有効な`localization`フィーチャーの裏にあります。`--no-default-features`でも、埋め込み済みの英語フォールバックを使って、引き続きコンパイルが通り、引き続きバリデーションも動作します。

- **`Paginator`向けの`IntoInertiaScroll`。**このトレイトは`LengthAwarePaginator`と`CursorPaginator`には実装されていましたが、シンプルなページネーターには実装されておらず、そのため`simple_paginate`の結果は`Inertia::paginate`にまったく渡せませんでした - `simple.rs`自身のモジュールドキュメントが、それをURL生成の経路として指し示しているにもかかわらずです。そのせいで、オフセットページネーションのInertiaコレクションは、リクエストごとの`COUNT(*)`と、スクロールメタデータの手組みとの間で選択を迫られていました。`next_page`は、計算された最終ページではなく、`LIMIT n+1`のオーバーフロー探査から得られます。合計値がないため、そこから計算するものがないからです。

### 修正

- **`suprnova generate-types`が、実行のたびに異なるファイルを出力していました。**トポロジカルソートは、`HashMap`をイテレートして作業キューの種を作っていました。Rustはプロセスごとにハッシュのイテレーション順序をランダム化するため、連続した実行が同じインターフェースを違う順序に並べていました。出力はコミットされる成果物であるため、実行のたびに差分が生まれていました - そして、理由もなく変動する生成ファイルは、人々が再生成をやめてしまうファイルであり、そうなると、それが説明しているはずのRustを静かに描写しなくなります。ディレクトリの走査もソートされるようになったため、出力はファイルシステムの順序にも依存しなくなりました。同じソースからの2回の実行は、今ではバイト単位で同一になります。

- **`topological_sort`は、自身のドキュメントコメントと正反対の動作をしていました。**依存先より先に依存元を出力していたのです。実害はありません - TypeScriptのインターフェースは、同じファイル内で後から宣言されるものを参照できるからです - そのため、順序ではなくコメントの方を修正しました。順序を直すと、何の得もないままコミット済みファイルをかき乱すことになります。

## 0.9.1 - 2026-08-01

3件の不具合です。いずれも、コードを読んで見つかったのではなく、コンテナ化されたハーネスの下でドッグフードアプリを走らせて見つかりました。そのすべてが、本番環境がプロセスを止めるような形でプロセスを止めることのないテストスイートには見えません。

それらは特定の順序で積み重なります: ローリングデプロイが、ジョブの途中でワーカーをSIGKILLし（1つ目）、そのジョブは、試行回数を一度もカウントしなかった再取得パスをたどります（2つ目）。

### 修正

- **`schedule:work`、`queue:work`、`workflow:work`が、SIGTERMを無視していました。**それぞれが`tokio::signal::ctrl_c()`だけをセレクトしており、これはSIGINTハンドラをインストールします - そのため、プロセス内のどこにもSIGTERMのハンドラが存在せず、しかもSIGTERMこそ、`docker stop`、Coolify、systemd、Kubernetesが送ってくるものです。3つとも、その`select!`の裏にはすでに慎重な有界ドレインを備えていました。ただし、それがスーパーバイザーの下で実行されたことは一度もありませんでした。修正前に計測したところ: `queue:work`コンテナへの`docker stop`は、40秒の猶予ウィンドウをまるごと使い切り、進行中のジョブを破壊したまま終了コード137で終了しました。PID 1として - コンテナが実行するのはこれです - カーネルはハンドルされていないSIGTERMをまるごと捨ててしまうため、プロセスは不格好に死んだのではなく、SIGKILLが来るまでまったく死にませんでした。`Server::run`はすでに両方のシグナルを正しく扱っており、そのリスナーは今では共有されているため、これはスケジューラーのループにおける、シグナルを取りこぼす窓も閉じます。

- **ワーカーを道連れにするジョブは、決してデッドレターに送られませんでした。***ハンドラ*が失敗するジョブはNACKされ、その試行がカウントされるため、`max_tries`の後にデッドレターに送られます。*ワーカーを道連れにする*ジョブ - OOM、アボート、セグフォルト、あるいは上記のSIGKILL - は何も決着しません。その予約は失効するだけで、どのドライバーも、かつてはそれをバイト単位で同一のまま再配送していました。そのようなジョブは不死身です: それを掴んだワーカーを次々に道連れにし、変わらぬ姿で戻ってきては次のワーカーを道連れにする、ということを、何かがワーカーを再起動し続ける限り繰り返します。3つのドライバーはすべて、ワーカーが死んだと判明した時点で試行をカウントするようになりました。`QUEUE_DRIVER`を切り替えても、ポイズンジョブを止められるかどうかが変わってはならないからです。`attempts`は今や、「ハンドラの失敗」ではなく「ワーカーへの配送」を意味します - `manual/queues.md`にドキュメント化されています。無関係な理由で失われたワーカーもまた、試行を1つ消費するからです。

- **…そして、使い果たされたジョブは今、ディスパッチされる前にデッドレターに送られます。**試行をカウントするだけでは、必要ではあっても十分ではありませんでした。あらゆるデッドレターの判定は、ハンドラが復帰することを前提とするワーカーの決着パスの中にありました - そのため、まさに復帰できなかったジョブに対してだけ、それが一度も実行されなかったのです。ドライバーの修正だけでは、カウンターは上昇するものの（計測値: 殺されたワーカー3台にわたって0 → 1 → 2）、それに対して何のアクションも起きませんでした。予算は今、ハンドラが走る前に使い切られます。最初の修正が正しく見えた後、コンテナ実験を再実行して初めて捕まりました。

- **デーモンには、tracingサブスクライバーがありませんでした。**`serve`は`init_telemetry`からそれを受け取ります。一方、`queue:work`、`schedule:work`、`schedule:run`、`workflow:work`は別の起動パスを通るため何も受け取っておらず、それらが発する`tracing::`の行はすべてどこにも届かず、`LOG_LEVEL`はそれらに対して無効でした。それこそが、これらが伝えるべきことのほとんどです - ジョブをデッドレターに送るワーカー、取りこぼしたティックをスキップするスケジューラー、解放できなかったロック。コンテナの中では、目に見える出力は起動時のバナーだけであり、プロセスはそのすべてを行いながら、アイドル状態に見えていました。このリリースにあった不具合のうち2件は、これが修正されるまで見えませんでした。

- **失敗ジョブストアが束縛されていない状態でのデッドレターは、サイレントな削除でした。**永続化ステップは`if let Some(store) = ..`の内側にあったため、ストアがないとこのアームはマッチせず、実行はACKへと素通りしていました - すぐ上にある失敗パスより静かで、そちらは少なくとも予約をそのまま残します。ストアが存在しないことは、壊れたストアより成功として扱われていたのです。今では、完全なエンベロープをERRORレベルでログ出力するようになりました。それこそが、`queue:retry`が再投入する対象だからです: 手作業で復旧できる作業と、消えてなくなった作業との違いです。

- **`QUEUE_DRIVER=database`は、失敗ジョブストアを束縛するようになりました。**`failed_jobs`は、そのドライバーの契約の一部です - `queue:retry`がそれを読み、`Queue::retry_failed`はそれなしには動作できません - しかし`bootstrap_from_env`はドライバーを配線する一方でストアを未設定のままにしていたため、アプリが手作業でストアを束縛しない限り、データベースバックエンドのキューは何もない場所へデッドレターしていました。`QUEUE_FAILED_DB_TABLE`経由で設定可能です。このドライバー限定です: `memory`は構造上一時的であり、`redis`には書き込む先のテーブルがありません。

- **Redisの再取得レイテンシは、`--visibility-timeout`に従うようになりました。**このフラグはXAUTOCLAIMのアイドル閾値を設定しますが、コンシューマーがどれくらいの頻度で確認するかは別のクロックが支配しており、ドライバーはそれをsea-streamerの30秒というデフォルトのままにしていました - そのため、`--visibility-timeout 5`は実際には「最大35秒」を意味していました。この間隔は今では設定済みのタイムアウトに追従し、1秒から30秒の範囲にクランプされるため、短いタイムアウトがXAUTOCLAIMストームになることはなく、長いタイムアウトは以前より再取得を速くする以外の結果にはなりません。

### 追加

- **`TaskBuilder::on_one_server()` / `on_one_server_for(ttl)`** - レプリカ全体で、期限が来たティックごとにスケジュールタスクをちょうど1回だけ実行します。これがなければ、そのティックのリーダーを誰も選出しません: 各`schedule:work`プロセスは独立にスケジュールを評価するため、3つのレプリカが、期限の来たすべてのタスクを毎分3回ずつ、ばらつきなく実行することが計測されました。3レプリカ上の毎晩の課金ジョブは、顧客ごとに3回ずつ課金していました。

  `without_overlapping()`はこれをカバーしませんし、できません: そのロックはタスクにキーが振られ、ハンドラが復帰したときに解放されるため、速いタスクは2つ目のレプリカが確認する前にロックを解放してしまいます。`on_one_server`は、タスク*とティック*にキーを振り、ハンドラを超えてロックを保持し続け、TTLで失効させます。2つは組み合わせられます。

  オプトインで、Laravelと一致します。フェイルクローズする点でLaravelと異なります: 選出は、その裏にあるキャッシュがどれだけ共有されているかにしか依存しないため、`CACHE_DRIVER=memory`かつシングルサーバー向けタスクがある本番環境の起動は、問題のタスクの名前を挙げて拒否されます。本当に単一のスケジューラーしか動かさないデプロイのためには、`SCHEDULE_ALLOW_MEMORY_LOCK_IN_PRODUCTION=true`があります。

### 変更

- `manual/deployment.md`は、「`schedule:work`プロセスをちょうど1つだけ動かす」ことをもはや唯一の選択肢としては述べなくなり、新たに**「クリーンに停止する」**セクションを得ました。このセクションは、サブシステムごとのドレインウィンドウ、それらを上回るようプラットフォームの終了猶予をどう見積もるか、そしてPID 1がシグナルハンドラの欠落を見た目以上に悪化させる理由をカバーします。

## 0.9.0 - 2026-07-31

### セキュリティ

- **認証の発行は、呼び出し元単位でしかスロットルできず、受信者単位ではできませんでした。**アドレスをキーにした制限は「1つのクライアントがうるさいか」には答えられますが、「1つのメールボックスが溢れさせられているか」には答えられません。ボットネットや単一のIPv6`/64`に分散した攻撃者は、あらゆるIP単位の予算を下回ったまま、1人の被害者の受信箱をパスワードリセットメールで埋め尽くすことができ、それを止めたはずの制限をフレームワークの中で表現するものは何もありませんでした - キー関数はパス、ヘッダー、クエリ文字列は読めても、フォームエンコードされたボディは読めなかったため、アドレスは、まさにそれを運んでいるルート上で見えなくなっていたのです。

  `identity_key`は、操作対象のアカウントにバケットのキーを振ります。クエリ文字列を先に読み、続いてバッファ済みのフォームボディを読むため、1つのキー関数が両方の形をカバーします。値はトリムされ小文字化されます。`Alice@Example.com`は`alice@example.com`と同じメールボックスに届きますし、シフトキーを押しっぱなしにするだけで回避できる制限は制限とは言えないからです。そしてハッシュ化もされます。レート制限のバックエンドは、多くの場合、プライマリデータベースよりアクセス制御が弱い共有Redisだからです。

  それを支えるのは、2つの新しいミドルウェアビルダーです。`key_reads_body(cap)`は、キーを振る前にボディをバッファします - オプトインです。バッファリングは、未認証の呼び出し元があなたにやらせることのできる作業であり、上限を超えるボディはキーなしで通過させるのではなく413で拒否されるからです。`only_when(pred)`は、自分に関係のないリクエストに対してはリミッターを丸ごとスキップします。これが、積み重ねられた受信者単位の予算が、受信者を指定しないルート上でサイレントに拘束力のある制限になってしまうのを防いでいます。

  ドッグフードアプリは今、発行グループの上に両方を積み重ねています: アドレスあたり5分に10回、受信者あたり15分に3回です。

Toriiのセッション、パスワード、OAuth、パスキーの各パスをレビューしたところ、8件の不具合が見つかり、すべてピン留めされたフォーク（`suprnova-torii-rs` `968b0be`）で修正されました。

- **期限切れのセッションが、リフレッシュによって息を吹き返すことがありました。**SeaORMのセッションリポジトリの`refresh`には有効期限の判定がなく、無条件に`expires_at`を延長していました。また`OpaqueSessionProvider::refresh_session`は、`get_session`が行う`is_expired()`チェックをスキップしていました。有効期限を過ぎて保持されたトークンが、無期限に更新され得たのです。両方の層で修正済みです。Suprnova自身の表面からは到達できません - `Torii`もフレームワークもセッションリフレッシュを公開していないからです - しかし、両クレートの公開APIではあります。
- **ログインフォームは、タイミングによってどのアカウントが存在するかを漏らしていました。**認証は、メールアドレスが一致しなかった時点でただちにリターンしており、Argon2をまるごとスキップしていました: 未知のアドレスで54µs、誤ったパスワードで719msと計測され、ネットワーク越しに読み取れる約13,000倍の差でした。どちらの失敗パスも、今ではダミーハッシュに対して検証を行うため、コストが同じになります。これは、Suprnovaのパスワードログインを通じて実際に到達*可能でした*。
- **JWTの`iss`クレームは、書き込まれてはいたものの検証されたことがありませんでした。**アルゴリズムのピン留めはすでに正しく行われていました - `alg: none`やHS/RSの混同は決して起こり得ませんでした - しかしissuerは飾りに過ぎず、署名鍵を共有する2つのサービスが互いのセッションを受け入れてしまう可能性がありました。issuerが設定されている場合、今では強制されます。
- **1回限りのはずのPKCEベリファイアが、2回クレームされることがありました。**消費は読み取りに続く削除という形だったため、同じ`csrf_state`に対する2つのOAuthコールバックが、どちらの削除も着地する前に、両方とも読み取れてしまうことがありました。今では1つの操作でクレームされます - Postgresでは`DELETE ... RETURNING`、SeaORMでは、影響を受けた行数で勝者を決めるプライマリキー削除です。
- **期限切れのセッションが、アクティブとして一覧表示されていました。**`find_by_user_id`には有効期限のフィルタがなく、期限切れの行はクリーンアップが走るまで残り続けるため、「サインイン中のデバイス」画面は、生きているセッションについては何も語らないまま、ユーザーに死んだセッションの失効操作を提供していました。
- **あるパスキーのルックアップが、`authenticate`と名付けられていました。**Toriiの`PasskeyService::authenticate_credential`はクレデンシャルIDを受け取り、それを所有するユーザーを返していました。そして`PasskeyAuth::authenticate`は、そこからセッションを発行していました。Toriiはパスキーを保存するだけです - WebAuthnへの依存を一切持たず、アサーションを検証できません。そのため、これらの呼び出しが証明できるのは、呼び出し元がクレデンシャルIDを知っていたということだけでした: ブラウザが平文で送信し、`allowCredentials`がセレモニーを開始できる誰にでも渡す値です。`find_user_by_credential`と`create_session_for_verified_credential`にリネームされ、どちらも検証が呼び出し元の仕事であることを文書化しています。Suprnovaを通じては到達できません。Suprnovaは`webauthn-rs`自体を自ら駆動し（`torii_integration::passkey`を参照）、Toriiにはクレデンシャルの保存のためにしか到達しないからです。
- **WebAuthnのチャレンジは、そのTTLの間ずっとリプレイ可能でした。**どちらのバックエンドも、読み取り時にチャレンジを消費しておらず、SeaORMの`get_challenge`は`expires_at`もまるごと無視して、期限切れのチャレンジを生きているものとして返していました。読み取りは今では両バックエンドで期限切れの行を除外し、新しい`take_challenge`が、ちょうど1回だけチャレンジをクレームします - PKCEの修正と同じ、削除が勝者を決める形です。

### 破壊的変更

- **Azure Blob StorageとGoogle Cloud Storageは、新しい`filesystem-azure`と`filesystem-gcs`フィーチャーの裏に移動しました。**`Storage::register_azblob`、`register_azblob_with`、`register_gcs`、`register_gcs_with`、`AzBlobConfig`、`GcsConfig`は、対応するフィーチャーを有効にしない限り、もう存在しません。どちらかのバックエンドを使っている場合は、依存関係に追加してください:

  ```toml
  suprnova = { git = "…", tag = "v…", features = ["filesystem-gcs"] }
  ```

  得られるのは、実行時の失敗ではなく、欠けている項目の名前を挙げるコンパイルエラーです。

  どちらのopendalサービスクレートも`rsa`を引き込みます。これはRUSTSEC-2023-0071（Marvinタイミング攻撃）を抱えており、上流に修正済みリリースがありません。これらは、`reqsign-core`のオプションの`rsa`が裏にある機能である`reqsign-core/jwt`を有効化する唯一のクレートだったため、それらをゲートすることで、3つのopendal経路すべてがそこへ至る道を一度に断ち切ります。`rsa`は今では*回避可能*です: `--no-default-features --features filesystem,database-postgres`は、それなしで解決でき、それでいてストレージサブシステムは維持されます。以前は、ストレージを何であれ維持したままそれを手放せるフィーチャーの組み合わせは存在しませんでした。

  標準のデフォルトビルドは、依然として`rsa`を抱えています - `database-mysql`はデフォルトフィーチャーであり、`sqlx-mysql 0.8.6`がそれに非オプションで依存しているためです - そのため、この監査上の例外は開いたままです。S3は意図的にゲートされて**いません**: `reqsign-aws-v4`は`jwt`なしで`reqsign-core`を使うため、S3ドライバーはそこへの経路に一度も関与しておらず、それをゲートすることは、何も取り除かないまま最も使われているクラウドバックエンドを壊すことになります。

### 追加

- **`suprnova --version`**、`-v`もclapのデフォルトである`-V`と同様に使えます。CLIに対して、他のあらゆるCLIが使うフラグでバージョンを尋ねたときに、使用方法のエラーが表示されるべきではありません。

### 修正

- **2つのRedis操作に、上限がありませんでした。**キャッシュのタグフラッシュは、タグのメンバー集合全体を`SMEMBERS`で読み取り、キーを1つずつ削除していました。そのため、メンバー数の多いタグはコネクションを詰まらせ、読み取りと削除の間に並行書き込みが失われることもあり得ました。タグは今では世代ベースになり、アトミックにフラッシュされ、上限付きの`SSCAN`でスキャンされます。遅延キューの昇格パスは、期限が来たすべてのジョブを1つの無制限な`ZRANGEBYSCORE`で移動させていたため、まとめて期限が来た滞留は、1つの巨大なスクリプトを生んでいました。今ではバッチ単位で昇格します。
- **2つのシャットダウンドレインが、永遠に待ち続けていました。**Ctrl-C時の`schedule:work`と、キャンセル後のワークフローワーカーは、どちらも期限なしにすべての進行中タスクをawaitしていたため、決して復帰しない1つのタスクが、`SIGKILL`が来るまでプロセスを開いたまま保持していました - オペレーターの目には、「止まらない」デーモンとして映ります。どちらも今では、有界の猶予を待ってから残りをアボートし、その件数を報告します。
- **リリースのバージョンピン留めスイープは、2つあるピン留め構文のうち片方しか認識していませんでした。**そのため、`cargo install --tag vX.Y.Z`という行を持ちながら依存関係のスニペットを持たないファイルは、一度も発見されませんでした。`suprnova-cli/README.md`は、3リリースにわたって読者にv0.6.0のインストールを案内し続けていました。`manual/cli.md`と`manual/cli-new.md`はv0.7.2のまま止まっていました。`manual/installation.md`は両方の形式を抱えており、片方だけが上がって、もう片方は凍りついていました。発見と書き換えは今では1つのパターンテーブルから読み取るようになり、ファイルのルールはその内容から導出されます。
- **`cargo doc`は、`filesystem`はあるが`testing`はないビルドすべてで失敗していました。**7つの`Storage::fake`イントラドキュメントリンクが解決できず、`lib.rs`は壊れたリンクを禁止しているためです。`testing`はデフォルトフィーチャーであるため、その組み合わせをビルドするゲートステップは一度も存在しませんでした。`check-feature-matrix.sh`は今ではビルドします。
- **Toriiのマイグレーションは、自分自身のスキーマの上で再生できませんでした。**そのため、`torii_migrations`という追跡テーブルを持たないままそれを保持しているデータベース - それをスキップしたダンプから復元されたものであれ、手作業でマイグレーションされたものであれ - は、管理下に置くことができませんでした。すべての`Table::create()`は`.if_not_exists()`を伴っていましたが、19個の`Index::create()`呼び出しはどれも伴っておらず、`ADD COLUMN locked_at`のalterも同様だったため、再生はテーブルを通り抜けた末に、最初の`CREATE INDEX`で息絶えていました。`IF NOT EXISTS`ではなく`has_index`/`has_column`を介して、ピン留めされたフォーク（`suprnova-torii-rs` `a0f956d`）で修正されました。sea-queryはそれをMySQL向けにサイレントに落としてしまうため、構文だけの修正では、デフォルトフィーチャーのビルドは壊れたままだったはずです。
- **失敗したToriiのマイグレーションは、エラーを返す代わりにプロセスをアボートしていました。**`SeaORMStorage::migrate`はマイグレーターをunwrapし、無条件に`Ok(())`を返していたため、失敗を`FrameworkError`へマッピングする`init_torii`側の処理は、到達不能なコードになっていました。
- **アプリ自身の`users`テーブルが、Toriiのものをサイレントに抑え込んでしまうことがありました。**`.if_not_exists()`は、「すでに自分のもの」と「すでに他の誰かのもの」を区別できないためです。マイグレーションは成功を報告し、認証は後になってカラム不足で失敗していました - これが、`--api`スターターがそのテーブルを`app_users`と名付けている理由です。Toriiのマイグレーションは今では、既存の`users`テーブルに必要なカラムが欠けている場合、マイグレーション時にそのカラムと対処法を挙げて警告します。既存のデプロイが起動し続けられるよう、ハードな失敗ではなく警告のままにしてあります。
- **RailwayとDigitalOceanのデプロイガイドは、プラットフォームのヘルスチェックを、Postgresをプローブし得るパスに向けていました。**どちらのプラットフォームも、そのチェックが失敗するとコンテナを再起動するため、そのアドバイスに従うと、データベースの瞬断がすべてのレプリカにまたがる再起動ループへと変わってしまいました。どちらも今では`/_suprnova/health/live`を使い、データベースはコンソールから手作業でプローブします。旧来のパスは引き続き解決します。すでにデプロイ済みのものに変更は必要ありません。

## 0.8.0 - 2026-07-30

外部のレッドチーム監査に対する是正対応です。監査は19件のP1指摘と、1.0に対するNO-GO判定を返しました。このリリースは、**19件すべて**を閉じます。加えて、それらを修正する過程で見つかった、監査が名指ししていなかった不具合もいくつか閉じます。

いくつかの修正は、サイレントな設定ミスを、意図的に起動拒否へと変えます。デプロイする前に**アップグレード**を読んでください - 問題なく動いていた本番アプリが、起動しなくなるかもしれません。

### アップグレード

以前は警告付きで（あるいはサイレントに）起動していた3つの設定が、今では本番環境でフェイルクローズします。それぞれのエラーは、それを解除する変数の名前を挙げ、リスクが本当に存在しないデプロイのためには、それぞれに明示的なオーバーライドが用意されています。

- **配信しないメールドライバー。**`MAIL_DRIVER`が未設定、`log`、`memory`、あるいは未知の値のいずれであっても、メールをレンダリングして破棄するトランスポートに解決されていました - そのため、パスワードリセットは、何も送信されないまま成功を報告していました。オーバーライド: `MAIL_ALLOW_NON_DELIVERING_IN_PRODUCTION=true`。
- **平文のSMTP。**4通りの認証情報の組み合わせのうち3つが、暗号化されていないトランスポートに帰着しており、両方とも未設定のケースは警告をログに出しながらもとにかく送信していました。オーバーライド: `MAIL_ALLOW_INSECURE_SMTP_IN_PRODUCTION=true`。
- **インメモリのレート制限。**そのバケットは1つのプロセスのヒープ上に存在するため、N台のレプリカの裏では、あらゆるクォータが実質N倍になり、デプロイのたびにリセットされます。`RATE_LIMIT_DRIVER`を`redis`に向けるか、本当に単一プロセスしか動かさないのであれば`RATE_LIMIT_ALLOW_MEMORY_IN_PRODUCTION=true`を設定してください。*未知の*ドライバー値も同じ理由で失敗します。メモリへフォールバックしていたからです - `RATE_LIMIT_DRIVER=Redis`のように大文字始まりのものは、設定されているように見えるため、本番環境に到達してしまう可能性が最も高いケースです。

開発、テスト、ステージングは、この3つのケースすべてにおいて変わりません。ステージングは意図的にゲートされていません: そこをハードに失敗させると、チームはオーバーライドをグローバルに設定するようになり、それは肝心なところでチェックを無力化してしまいます。

起動失敗ではない、2つの挙動変更です:

- **`fill`と`first_or_new`は、不正な形式の値を拒否します。**フィールドの型へデコードできない値は、以前はそのフィールドの`Default`になった上で`Ok`を返していました - `fill(attrs!{ age: "abc" })`は`age = 0`をセットし、成功を報告していました。今では、そのフィールドの名前を挙げた`ValidationError`を返し、モデルには触れません。未知のカラムは引き続きサイレントにスキップされ（Laravel互換）、数値の拡幅も引き続き機能します。
- **`/_suprnova/health?db=true`は、もうドライバーのエラーを返しません。**詳細はログへ移動しました。ボディは`"database": "error"`を保持し続けます。デバッグビルドには引き続き含まれます。`status`/`database`をパースするダッシュボードへの影響はありません。
- **`url::signature_has_not_expired`は、有効な署名を要求するようになり**、非推奨になりました。以前は、偽造されたURLに対しても`true`を返していました - 不正な署名は「期限切れ」ではありません。取り逃す期限そのものを一度も持っていなかったからです - そのため、それだけをガードにしていたハンドラは、偽造を受け入れていました。今では`has_valid_signature`と同一です。*期限切れ*と*無効*を区別するためにこれを使っていた場合（403の代わりに「新しいリンクをリクエストしてください」を描画するためなど）、3つの状態すべてを返す`url::signature_verdict`に切り替えてください。これはLaravelの`URL::signatureHasNotExpired`とは、意図的に異なります。

オプトインした場合にのみ、あなたの側で何かが必要になる、2つの追加機能です:

- **`QueueDriver`に`settle`と`release`が追加されました**。どちらもデフォルト実装を持つため、既存のドライバー実装は変更なくコンパイルが通り続けます。あなたのバックエンドが、後続の書き込みとACKを1つのトランザクションでコミットできるなら`settle`を実装してください。予約済みのメッセージをその場でリキューできるなら`release`を実装してください。
- **バッチの集計を、永続化できるようになりました。**`DatabaseBatchRepository`は、`job_batches`と`job_batch_settlements`という2つの新しいテーブルを必要とします - `jobs`や`failed_jobs`と同様に、あなたのマイグレーションに追加してください。スキーマは`manual/queues.md`にあります。`MemoryBatchRepository`のままであれば、何も変わりません。

### セキュリティ

- **Slowloris（SEC-07）。**hyperのヘッダー読み取りタイムアウトは、30秒とドキュメント化されていましたが実際には無効でした - コネクションビルダーにタイマーがインストールされている場合にのみ作動するところ、インストールされていなかったのです。クライアントは、コネクションと`SERVER_MAX_CONNECTIONS`のパーミットを、無期限に保持できました。今では作動し、`SERVER_HEADER_READ_TIMEOUT`経由で設定可能です。
- **マルチパートアップロード（SEC-05）。**上限は個々のパートのペイロードには適用されていましたが、生のストリームには適用されておらず、そのためボディは合計で上限を超えることがありました。今ではストリームで上限が課されます。
- **空のキーを使ったWebhook HMAC（SEC-08）。**どちらの支払いアダプターも空のシークレットを受け入れており、これは何でも検証を通してしまいます。両方で拒否するようになりました。
- **Paddleの署名パース（P2-11）。**奇数長、あるいは非16進の`paddle-signature`が、ピン留めされたSDKに到達し、その内部でパニックしていました。今では先に検証されます: 不正な形式の署名は401になります。
- **パスキーの登録とリセットトークン（SEC-01、SEC-02）。**既存のメールアドレスに対する匿名の登録、非所有者による登録、そして直近の再認証を伴わない所有者による登録は、それぞれ別個のステータスで拒否されます。パスワードログインは今では、再認証ウィンドウのタイムスタンプを刻むようになりました。
- **`dev:tls`（SEC-10）。**プロジェクトが、このコマンドが信頼するCAを選べてしまっていました。
- **生成されるDocker Compose（P2-12）。**このリポジトリにコミットされた認証情報のまま、PostgresとRedisをすべてのインターフェースに公開していました。今ではループバックに束縛され、スキャフォルドごとに生成されるパスワード、0600で書き込まれる`.env`、そしてシンボリックリンクされたターゲットの拒否が備わっています。
- **ヘルスエンドポイント（P2-01、CI-05）。**データベースへクエリを投げるかどうかを、`query.contains("db=true")`という部分文字列テストで決めていたため、`?nodb=true`でもプローブが走っていました。今では正しくパースされます。503は、ホスト、ポート、スキーマ、バージョンを名指ししていたドライバーのエラーを、もう埋め込みません。
- **認証情報発行のスロットリング（P2-02）。**リファレンスアプリの4つの認証発行ルートには、レート制限がまったくありませんでした。唯一レート制限を持っていたルートも、生の`x-forwarded-for`ヘッダーにバケットのキーを振っていました - これはどんなクライアントでも、リクエストごとに変えて新しいバケットを得ることができます。両方とも修正されました。発行の予算は4つのルート間で共有されるため、それらを切り替えても予算が倍増することはありません。
- **再配送されたチェーンのステップが、新しいidの下で後続を再プッシュしていました（DATA-02b、部分的）。**決着処理は、ACKする*前に*次のチェーンのリンクをプッシュします。これは意図的なものです: 先にACKすると、そのウィンドウでのクラッシュがチェーンを永久に失わせてしまいますが、重複はサイレントな消失とは違って回復可能だからです。しかし、後続のエンベロープはプッシュのたびに新しい`Uuid::new_v4()`を得ていたため、そのトレードオフによって生じた重複は、正当な新しいステップと区別がつきませんでした - ドライバーにとっても、アウトボックスにとっても、ハンドラにとっても。

  最後のそれこそが、本当のコストです。フレームワークの配送契約はat-least-onceであり、重複に対する答えは「ハンドラはべき等でなければならない」です - しかし、受け取る唯一の識別子である`env.id`にキーを振ったハンドラは、チェーンされたジョブについてはその契約を満たせませんでした。重複がそのたびに新しいidの下で届いていたからです。その契約は、構造上満たしようがなかったのです。

  後続のidは今では、先行のidから導出されたUUIDv5になりました。これは、その先行自身の再配送をまたいで安定しています。再配送されたステップは、以前にプッシュしたidを再プッシュします。スキーマの変更も、新しいフィールドも、新しい依存関係もありません。

  これにより、重複は**検出可能**になります。これが、DATA-02bの残りの部分に欠けていたプリミティブです。これは、プッシュをACKとアトミックにするわけではなく（それにはアウトボックスが必要です）、入ってくる重複を拒否するものもまだありません。どちらも未解決のままです。
- **署名付きURLは、あるURLを検証しながら別のURLを実行していました（SEC-04）。**正規化された形式は、クエリのペアをマップへと畳み込んでいたため、繰り返されたキーは**最後**の値だけを保持していました - 一方、`Request::query_param`は**最初**の値を返していました。そのため、正当に署名された`?user=victim`は、元の署名をそのままに`?user=attacker&user=victim`としてリプレイできてしまいました: 検証は`victim`で正規化されて通過し、ハンドラは`attacker`に対して動作していました。

  正規化された形式は今では、`(key, value)`でソートされたすべてのペアを保持するため、署名はパラメータの正確なマルチセットをカバーします - どの値を追加、削除、置換してもHMACが壊れます。繰り返された`signature`や`expires`は、まるごと拒否されます。どちらであれ2つあると、どちらが支配するかについて恣意的でない答えが存在しなくなるからです。

  `Request::query_param`は今では、繰り返されたキーを最後の値に解決するようになり、`query_params`や`Context::query_param`と一致します。3つのうち、食い違っていたのはこれだけであり、その食い違いこそが不具合のもう半分でした。**既存の署名付きリンクは、引き続き機能します** - 繰り返しキーがなければペイロードのバイト列は変わらず、これはテストで固定されています。未解決のあらゆるパスワードリセットリンクをサイレントに無効化してしまう正規化形式の変更は、この不具合そのものより悪いことになるからです。

  6件のリグレッションテストがあり、両方の攻撃順序、正当に繰り返されても署名・検証できなければならないキー、そして並べ替えの保証をカバーしています。変更*されていない*もの: `signature_has_not_expired`は、依然として偽造された署名を「期限切れではない」と報告します。これはLaravelの挙動であり、ドキュメント上の修正として意図的に据え置かれたもので、善意の「修正」に抗してそれを固定する専用のテストを持っています。
- **Postgres下でのRBAC。**SQLiteだけでなく、実際のPostgresに対して検証されました。
- **4件のRustSecアドバイザリーが、更新ではなく根絶されました。**Pineconeドライバーは、PineconeのREST APIに対して書き直され、`pinecone-sdk 0.1.2`を切り離しました - その最新リリースは2024-09-06付けです - それに伴い、`tonic 0.11 → rustls 0.22 → rustls-webpki 0.102`と、RUSTSEC-2026-0049 / -0098 / -0099 / -0104も切り離されました。この4件はすべて、`rustls-webpki >= 0.103.13`で上流で修正済みであり、このワークスペースは他のTLS利用箇所についてはすでにそちらに解決していました。1つの放棄されたクレートが、ツリーを脆弱な系列に留めていたのです。`.cargo/audit.toml`は、5件のignoreから1件へ減りました。このドライバーのAPIにとってこれが何を意味するかは、**変更**を参照してください。
- **監査の例外に、有効期限が設定されるようになりました。**`.cargo/audit.toml`のすべてのエントリが`OWNER`と`EXPIRES`日付を持つようになり、`scripts/check-audit.sh`は、オーナーの欠落、日付の欠落あるいはパース不能、または期限切れのいずれかがあると、リリースゲートを失敗させます。`cargo audit`には、期限付きignoreという概念がないため、「一時的に」追加されたものが、誰かがそのファイルを読み直すまで残り続けていました。残っているエントリ（RUSTSEC-2023-0071、`rsa`。これはそもそも修正済みリリースがまったくありません）には、オーナーと日付が設定されています。
- **到達可能性の主張は、宣言ではなく検証されます。**`scripts/check-feature-matrix.sh`は、実際の依存関係ツリーを解決し、`cargo audit`が実際に読み取る対象である`--all-features`を含め、どのビルドも`pinecone-sdk`、`rustls-webpki 0.102.x`、`tonic 0.11.x`を含まないことをアサートします。何も検証しないコメントによって正当化された例外は、誰かが依存関係を1つ追加した瞬間に真実でなくなります。

### 修正

- **データベースバックエンドのキューにおける、あらゆるリリースが、サイレントに何もしていませんでした。**`JobOutcome::Released`（ビジーな`WithoutOverlapping`ロック、レートリミッターのバックオフなど）は、「コピーをプッシュしてから、元のものをACKする」という形で実装されていました。エンベロープのidは`jobs`テーブルのプライマリキーであるため、コピーは、現在の予約を保持したままの行と衝突し、プッシュは`UNIQUE constraint failed: jobs.id`で失敗していました。ワーカーはその後、正しくACKを拒みました。そのため、要求された遅延は一度も適用されず、`JobReleased`イベントも発火せず、ジョブはただ、可視性の失効が再配送するまで留め置かれていました。リリースは今では、その場で完結する1回のドライバー呼び出しになりました。
- **部分的なバッチディスパッチが、すでにキューに入れていたジョブを孤児にしていました（DATA-02）。**`driver.push`がループの途中で失敗すると、`PendingBatch::dispatch`はバッチの行を削除していました - しかし、すでにキューに入っていたエンベロープには、そのバッチidが刻印されたままだったため、それぞれが、もはや存在しないバッチに対して決着しようとし、配送のたびに永遠に`Err(batch not found)`を返していました。バッチは今では、代わりに決着させられます: ディスパッチされなかったジョブは失敗として記録され、バッチはキャンセルされます。そのため、キューに入っていたものは正常に決着し、終端コールバックも引き続き発火します。
- **`url::has_valid_signature`が偽造されたURLを拒否することを、何もテストしていませんでした。**SEC-04の修正を検証している最中に見つかりました: 主要な署名付きURLガードを、あらゆる署名を受け入れるよう書き換えても、フレームワークのテストスイート全体が通過してしまったのです。
- **スキャフォルドされたアプリは、データベースをマイグレーションすることも、イメージをビルドすることもできませんでした（REL-01b）。**どちらのスキャフォルドも`default-run`を宣言していなかったため、`cargo run`をシェルアウトする9つのCLIラッパーすべてが、まっさらなプロジェクトで失敗していました。生成されるDockerfileには、5つの独立した不具合がありました - ロックファイルのCOPY漏れ、ロックなしの`npm ci`、宣言済みの2つのバイナリのうち1つだけをスタブするキャッシュステージ、viteが一度も作らないパスからコピーされるフロントエンドビルド、そして`inertia_response!`がコンパイル時に検証する`frontend/src/pages`のコピー漏れです。標準のスキャフォルドのイメージは、ビルドできませんでした。
- **`docker:init`は、あらゆるプロジェクト種別に対して同じDockerfileを出力していました。**`--api`プロジェクトでは、最初の命令である`COPY frontend/package.json`がまるごと失敗していました。APIプロジェクトは今では、フロントエンドを含まないDockerfileを受け取ります。
- **SQLのプレースホルダー（DATA-01）。**単一の方言を前提とするのではなく、バックエンドごとにレンダリングされるようになりました。
- **キューの決着（DATA-02a、P2-06c）。**後続処理は、予約がACKされる前に決着するようになり、ロック解放のエラーが、すでに成功したジョブをリトライに変えてしまうこともなくなりました。
- **キャンセルされたバッチは、`Then`ではなく`Catch`を発火していました。**
- **`Builder::clone`が、eager-loadの計画をサイレントに落としていました（P2-09a）。**`User::query().with("posts")`は、ページネーション、`count()`、クローンを行うあらゆるスコープなど、どこでクローンしても、リレーションを持たない行をエラーなしで返していました。
- **プレゼンスの名簿がメンバーを失っていました（P2-08）。**名簿は購読の前にスナップショットされていたため、そのウィンドウの間に参加した人は、どちらの名簿にも永久に現れませんでした。
- **Pineconeは、すべてのインデックス取得を直列化していました（P2-14）。**書き込みロックは、2回のネットワークラウンドトリップにまたがって保持されており、`tokio`の公平な`RwLock`のせいで、1つの冷えたインデックスが、あらゆる温まったインデックスを止めてしまっていました。
- **型ウォッチャーが、バーストを捨てていました（P2-13）。**リーディングエッジのデバウンスは、バーストの最初のファイルで再生成し、末尾の実行なしに残りを捨てていたため、最後の保存が反映されることは決してありませんでした。
- **`ssr:check`はハングすることがあり、アドレスを1つしか試しませんでした（P2-13）。**DNSは、タイムアウトの外側でまるごと実行されており、解決されたアドレスのうち最初の1つしか試されていませんでした - そのため、AAAAレコードを持ちながらIPv6経路を持たないホストは、v4でリッスンしているにもかかわらず、ワーカーがダウンしていると報告していました。
- **`suprnova serve`は、`cargo-watch`をピン留めせずにインストールしていました（P2-13）。**今ではメジャーバージョンの範囲付きで`--locked`になりました。
- **リリースのバンパーは、5つのREADMEだけを書き換え、他は何もしていませんでした。**4つのマニュアルの章と1つの公開ドキュメントコメントが、どのリリースでも一度も更新されないタグをピン留めしたままになっていました - そのドキュメントコメントは、2リリース分古くなっていました。発見は今では手作業で保守されていたリストに置き換わり、スモークテストは、バンパー自身のverifyステップを信頼するのではなく、更新後のツリーを独立してgrepします。
- **`db:sync`は、データベースのスキーマを信頼できる入力として扱っていました（CLI-01）。**
- **`migrate:fresh`は、`--force`と入力による確認の両方の裏にゲートされるようになりました（CLI-02）**。CLIだけでなく、アプリのバイナリでも同様です。
- **`log`メールドライバーは、Laravelと同じように、メッセージ全体をログに出力するようになりました**。そして本番環境では、bearerリンクをログに書き込まなくなりました。

### 追加

- **アトミックな終端決着（`QueueDriver::settle`、DATA-02）。**チェーンの後続とACKは今では、`DatabaseQueueDriver`上で一緒にコミットされ、その間でのクラッシュがチェーンの残りを失わせたり、次のステップを2回実行させたりするウィンドウを閉じます。予約をキーにした削除は、フェンスとしても機能します: 実行途中で可視性が失効したワーカーは何もコミットせず、`Settled::Stale`を報告するため、別のコンシューマーが今では所有しているメッセージに対して作業をエンキューすることができません。これができないドライバーは`Settled::Unsupported`を返し、ドキュメント化されたプッシュ・ビフォア・ACKの順序を維持します。
- **`DatabaseBatchRepository`（DATA-02）。**バッチの集計は再起動を生き延びます。`pending_jobs`/`failed_jobs`は、保存してデクリメントするのではなく、`(batch_id, job_id)`をキーにした決着行から導出されます - そのため、再配送されたジョブが、他のジョブがまだ実行中であるにもかかわらずバッチを「完了」に押し進めてしまうことはなく、この保護は1プロセスの中だけでなく、プロセスをまたいで保たれます。
- **`/_suprnova/health/live`と`/_suprnova/health/ready`。**liveness（生存確認）は何にも触れません。readiness（準備確認）は依存関係をプローブします。livenessプローブにデータベースチェックを組み込むと、データベースの瞬断が、すべてのレプリカのローリング再起動に変わってしまいます。これは、以前の単一のエンドポイントが誘発していたことです。`/_suprnova/health`は、ドキュメントどおりに引き続き機能します。
- **`SERVER_HEALTH_READINESS_TOKEN`。**readinessプローブ向けのオプションの共有シークレットで、一定時間で比較されます。これがない場合、readinessは404を返します - ルーティングされていないパスと見分けがつきません。それは実際にルーター自身の404そのものだからです。既存のプローブが引き続き機能するよう、デフォルトでは未設定です。
- **`MAIL_SMTP_ENCRYPTION`** - `starttls` | `tls` | `none`で、`ssl`と`null`はLaravel互換のエイリアスとして受け入れられます。未設定の場合は認証情報から導出され、以前の挙動を正確に再現します。これにより、ポート465の暗黙的TLSにも到達できるようになります: トランスポートはそれをサポートしていましたが、どんな環境変数の組み合わせを使ってもそれを選択することはできませんでした。
- **`SERVER_MAX_CONNECTIONS`と`SERVER_HEADER_READ_TIMEOUT`**が、まるごと欠けていた`manual/env-vars.md`にドキュメント化されました。

### 変更

監査自身の結論は、ゲートが470秒で通過し、19件のP1のうちどれも捕まえなかった、というものでした。このリリースのテスト作業の大半は、そこに狙いを定めています。

- **Postgresがゲートの中で実行されるようになりました。**6つのファイルにまたがる12件のテストが、一度も実行されたことがありませんでした。そのうち2件は、デフォルトで`localhost:5432`上にあるどんなPostgresに対してであれ`DROP TABLE`を向けてしまうことが判明し、どちらも`Crypt`を一度も初期化していなかったため、初めて実行されたときにどちらも失敗しました。
- **スキャフォルドのアサーションは、テンプレートのソースではなく、置換後にユーザーが受け取るバイト列を読むようになりました。**データベースを文字どおり`{package_name}`と名付けたドキュメントコメントを出荷しているAPIプロジェクトや、フレームワークが一度も読まない5つのメールキーを謳う`.env.example`を発見しました。
- **キューの障害注入。**ACKの喪失、再配送、リースの失効、部分的なディスパッチは、指定された呼び出しで指定された操作を失敗させるデコレーターによって駆動されるようになり、あらゆるケースが、スリープを使った競合ではなく決定的になりました。
- **支払いアダプターに、ネガティブテストが追加されました。**Stripeの`verify()`は、*有効な*署名で一度も演習されたことがなく、HMAC比較への到達に依存するあらゆる拒否パスが、実証されていませんでした。
- **Pineconeドライバーは、RESTで話すようになりました。***デフォルトでオフの`vector-pinecone`フィーチャーの裏にある、破壊的変更です。*動機は**セキュリティ**の下にあります。表面上の変更は次のとおりです:
  - `client()`はなくなりました - `PineconeClient`はもう存在しません。代わりとなるのは`control_plane_get`、`control_plane_post`、`data_plane_post`で、これらは、ドライバーの認証済みでホスト解決済みのトランスポート上で、あなた自身のリクエスト型とレスポンス型を使って、*あらゆる*Pineconeエンドポイントに到達できます。これは、以前の抜け道が持っていたよりも厳密に広い到達範囲です。
  - `json_to_metadata` → `metadata_from_json`となり、メタデータは今では`prost_types::Struct`ではなく`serde_json::Map`です。`decode_match_fields` → `decode_match`となり、`PineconeMatch`を受け取ります。`namespace()`は`&str`を返します。
  - 新規: `with_control_plane`、`with_api_version`、`with_index_host`（既知のホストを固定し、コントロールプレーンへのラウンドトリップをスキップします）、`index_host`、そして`PineconeVector`/`PineconeMatch`という通信用の型です。
  - `from_env`は引き続き`PINECONE_API_KEY`と`PINECONE_CONTROLLER_HOST`を読み取り、今では`PINECONE_API_VERSION`も読み取ります。
  - REST APIのバージョンは、浮動ではなく固定されています - `2025-04`、つまりドライバーのリクエストとレスポンスの形がそれに対して書かれたバージョンです。
  - もはや何も直列化されません。旧ドライバーは、`pinecone-sdk`が`&mut self`の裏でしかそれを公開していなかったため、名前ごとに1つの`Index`を`tokio::Mutex`の裏にキャッシュしていました。新しいものは、ホスト文字列をキャッシュし、`reqwest`のコネクションプールを共有します。
  - コントロールプレーンから知らされたホストは、レスポンスがどんなスキームを運んでいようと、常に`https`経由で連絡されます。
  - `Debug`は、APIキーを伏せ字にした形で手動実装されているため、ドライバーを保持する構造体に対する`#[derive(Debug)]`は、それを出力できません。
- **Pineconeの通信契約テスト。**実際に稼働している統合テストは`PINECONE_API_KEY`を必要とするため、ゲートの中では実行できません - そのせいで、RESTでの書き直しにおけるフィールド名（`topK`、`includeMetadata`、`vectorCount`）は、何にも支えられないまま残っていました。13件のテストが今では、ローカルの`wiremock`フェイクに対してドライバーを駆動し、それが実際に送信する正確なメソッド、パス、ヘッダー、JSONボディをアサートします。さらに、2xx以外がレスポンスとしてデコードされることは決してないこと、そしてエラーメッセージがAPIキーを運ぶことは決してないこともアサートします。これらは、ドライバーをPineconeの*ドキュメント化された*契約に固定します。ドキュメントが実際のサービスと一致していることを確認できるのは、`#[ignore]`されたテストだけです。

## 0.7.2 - 2026-07-28

### 修正

- **`generate-types`は、deriveを持たないネストしたprop構造体も解決します。**0.7.1のジェネレーターは、`InertiaProps`/`Data`をderiveしていない型を持つあらゆるpropフィールドを`unknown`へ劣化させていました - そのため、コミット済みの型ファイルを持つプロジェクトに対してジェネレーター（あるいは`suprnova serve`のウォッチャー）を再実行すると、`Array<AdminArticleRow>`のような実在のインターフェースが`unknown`に置き換わり、アプリ全体の型チェックが壊れていました。`src/`のどこかで定義されたプレーンな構造体は、今ではpropのルートから推移的に、実在のインターフェースへ解決されます。`unknown`（警告付き）は、プロジェクトが本当に定義していない型 - 外部クレートの型、enum、タプル構造体 - のために予約されています。

### 変更

- **`routes.ts`の生成は、オプトインになりました。**`generate-types`は、聞かれもせずに`frontend/src/types/routes.ts`をあらゆるプロジェクトに落とすことはもうありません。生成するには`--routes`を渡してください。

- **フロントエンドのスターター依存関係が更新されました。**`suprnova new`による新しいスキャフォルドは、今では現行バージョンを固定します: Vite ^8.1.5、Tailwind CSS ^4.3.3、Svelte ^5.56.8（vite-plugin-svelte ^7.2.0、svelte-check ^4.7.4）、React ^19.2.8（plugin-react ^6.0.4）、Vue ^3.5.40（plugin-vue ^6.0.8、vue-tsc ^3.3.8）、そして`@types/node` ^24（Node 24 LTSの型系列）です。TypeScriptは意図的に^6.0.3のままです: これは最新の6.x系であり、svelte-checkのpeer範囲（`^5 || ^6`）は、まだTypeScript 7を受け入れないためです。3つのスターターすべてが、更新後のセットに対してエンドツーエンドで検証されました（`npm install` + `npm run build`）。

## 0.7.1 - 2026-07-27

0.7.0のキュールーティングに対する、不具合修正のパスです。リリース後の全面レビューから生まれました。

### 修正

- **チェーンされたジョブは、もう宣言済みのキューを失いません。**`ChainLink`は、チェーン構築時にジョブの`max_tries`、`timeout`、`backoff`をキャプチャしていましたが、`Job::queue()`はキャプチャしていませんでした。そのため、直接プッシュされれば宣言済みのキューに届くジョブが、チェーンの一部としてディスパッチされると`default`に届いてしまっていました - route → job → defaultという解決順序のうち「job」の階層が、チェーンに対してはサイレントに消えていたのです。宣言済みのキューは今ではリンクにキャプチャされ、直接プッシュとまったく同じように解決されます。このリリースより前に書かれたチェーンのペイロードは、変更なくデコードでき（`serde(default)`）、宣言済みのキューを持たないリンクは、0.7.0が書き出したものとバイト単位で同一に直列化されます。
- **失敗ジョブのレコードは、そのジョブが死んだキューを運ぶようになりました。**ワーカーのデッドレターパスは、あらゆる`FailedJob`レコードに`queue = "default"`をハードコードしていたため、ルーティングされたジョブの失敗は、失敗ストアをそれを所有するプールでフィルタしているオペレーターには見えませんでした。レコードは今では、エンベロープのキュー（ルーティングされていないジョブには`default`）を運びます。
- **0.7.0のアップグレードノートは、`jobs`マイグレーションの重要性を過小に述べていました。**そこには「フィルタしないワーカーは影響を受けず、マイグレーションは不要」と書かれていましたが、`DatabaseQueueDriver::push`は、ジョブがルーティングされているかどうかにかかわらず、`INSERT`の中で`queue`カラムを名指しします - マイグレーションされていないテーブルに対する0.7.0のバイナリは、フィルタの有無を問わず**あらゆるプッシュ**を失敗させます。以下の0.7.0のセクションと`manual/queues.md`は修正されています: データベースドライバー上では、`ALTER TABLE`があらゆるデプロイにおいて必須であり、バイナリがロールする前に実行されなければなりません（古いバイナリは自身のカラムを明示的に列挙し、新しいカラムを無視するため、先にマイグレーションする順序は安全です）。

- **READMEは、もう`#[job]`マクロを謳いません。**そのようなマクロは存在しません - ジョブは`Job`トレイトを実装します。キューの行は今では、0.7.0のキュールーティングを含む、実際の表面を説明します。

### 変更

- **リリースパスは、今ではREADMEのバージョン参照を更新します。**`bump-workspace-version.py`は、READMEのピン留めされたインストールタグ、配布モデルの例、MSRVの行を、マニフェストとアトミックに書き換えます。パターンにマッチしなくなるほど文言が変わったREADMEは、リリースをはっきりと失敗させます。READMEは、v0.7.0が出荷されて以降、リリースパスの中の何もそれに触れていなかったため、v0.6.0を謳い続けていました。
- **コネクションルーティングは、名前解決のみであるとドキュメント化されました。**`Job::connection()`と`Queue::route`のコネクションフィールドは、`JobQueueing`/`JobQueued`のライフサイクルイベントが運ぶコネクション*名*を解決します。単一のプロセスグローバルなドライバーが引き続きすべてのプッシュを受け取るため、それらは別のドライバーを選択するわけではありません。rustdocと`manual/queues.md`は、以前は存在しないドライバー選択をほのめかしていました。キューの次元には影響がありません - こちらはエンドツーエンドで尊重されます。コネクションごとのドライバーは、今後の課題のままです。
- `ChainLink`に公開の`queue: Option<String>`フィールドが追加され、これはチェーンリンクの構造体リテラル構築を壊します。`ChainLink::from_job`（通常の経路）を通じて構築されるリンクへの影響はありません。

### アップグレード

データベースキュードライバー上で0.6.x以下から移行する場合は、バイナリをロールする**前**に、以下の0.7.0のマイグレーションを適用してください。これは、そのドライバー上のあらゆるデプロイに必要であり、`--queue`を使うものだけではありません。0.7.1自体にはマイグレーションは必要ありません。

## 0.7.0 - 2026-07-26

### セキュリティ

- **`ammonia`を4.1.4へアップグレード（RUSTSEC-2026-0213）。**4.1.3までのバージョンは、SVGの`animate`と`set`アニメーションタグを介したXSSを許してしまいます。`ammonia`は、Suprnovaのmarkdownパイプライン（`comrak` → `syntect` → `ammonia`）の末端にあるサニタイザーであるため、`content`を通じてユーザー入力のMarkdownをレンダリングするあらゆるアプリが、影響にさらされていました。このアドバイザリーは2026-07-21に公開されました - v0.6.5の出荷後です - そのため、**v0.6.5までのすべてのリリースが影響を受けます**。フレームワークをアップグレードすることが修正であり、アプリケーションコードの変更は必要ありません。

### 追加

- **キュールーティング。**ジョブは特定のキューとコネクションへディスパッチでき、ワーカーは特定のキューに専念させられます - Laravel 13の`Queue::route(...)`の表面を、型付きにしたものです。ジョブは`Job::queue()`/`Job::connection()`で自分自身の居場所を宣言します。オペレーターは、ジョブを編集することなく、`bootstrap::register()`の中の`Queue::route::<SendInvoice>(Some("redis"), Some("billing"))`で、それを一元的にオーバーライドできます。解決順序はroute、job、グローバルデフォルトの順で、routeの中の`None`フィールドは、クリアではなく先送りを意味します。`queue:work --queue=billing,default`は、それらのキューだけをドレインします。ルーティングされていないジョブは`default`に属するため、決して取り残されません。チェーンされたジョブは、チェーンのリンクが自身のジョブを型消去して保存するため、ルートを名前で解決します。
- **`QueueDriver::pop_from`。**フィルタリングされたpopで、尊重できないフィルタに対しては、あらゆるキューをサイレントにドレインするのではなく**拒否する**デフォルト実装を持ちます - `billing`をドレインするよう指示されたワーカーが、静かにすべてをドレインしてしまうと、間違ったプールが間違ったジョブを食い尽くすまで、正常に動いているデプロイと見分けがつきません。メモリドライバーとデータベースドライバーは、ネイティブにフィルタします。カスタムドライバーは、コンパイルが通り続け、このはっきりしたデフォルトを継承します。
- **`jobs`テーブルのスキーマをドキュメント化しました。**`manual/queues.md`は今では、`DatabaseQueueDriver`が実際に期待するDDLを掲載しています。これは、以前はドライバーのSQLを読むことでしか発見できませんでした。
- **Inertiaの`serverHead`オプションをドキュメント化しました。**サーバー主導の`<head>`要素（Inertia 3.5.0）は、フレームワークのサポートを一切必要としません: クライアントはそれらを普通のpropから読み取るため、どのハンドラもすでにそれらを提供できます。`manual/frontend-inertia-responses.md`を参照してください。

### 変更

- `Envelope`に`queue: Option<String>`フィールドが追加されました。これは`serde(default)`であり、存在しない場合はスキップされるため、ルーティングされていないエンベロープは、以前のバージョンが書き出したものとバイト単位で同一に直列化されます - 通信上の形式を凍結したテストは変更なく通過し、`schema_version`のバンプもなく、ローリングアップグレードの最中もバージョンの混在したフリートが相互運用できます。
- `WorkerConfig`に`queues: Vec<String>`フィールドが追加されました（空の場合はすべてをドレインする、以前の挙動のままです）。
- `ROADMAP.md`を削除しました。その設計原則は`manual/introduction.md`に、作業の取り決めは`manual/contributions.md`に、デプロイとスケールアウトの資料は`manual/deployment.md`に、それぞれ住んでいます。出荷済み/計画中のチェックリストは、古くなっていました。`README.md`が「上流との関係」のために指し示していたリンクは、すでに宙に浮いていました - その帰属表示は`LICENSE`に住んでいます。
- スキャフォルドのフロントエンドは今では、`@inertiajs/{svelte,react,vue3}`を（`^3.4.0`から）`^3.6.1`に固定します。3.4.0 → 3.6.1の範囲はクライアントサイドのみです - 上流のchangelogと`packages/core/src/types.ts`の`Page`契約に照らして監査したところ、3.6.1のクライアントが送るあらゆる`X-Inertia-*`ヘッダーは、すでに処理済みでした。
- `scripts/release.sh`は今では、そのバージョンの`CHANGELOG.md`セクションから取られたノートを添えて、GitHubリリース自体を公開します。以前はこれが、スキップされがちな手作業の「次のステップ」だったため、v0.5.10とv0.6.1–v0.6.3はタグのみで、Releasesページは古いバージョンのまま止まっていました。プリフライトはゲートの前に実行されるため、`gh`やchangelogセクションの欠落は数秒で失敗し、`origin`がGitHubでない限り、公開は自動的にスキップされます。

### アップグレード

データベースキュードライバー上の既存の`jobs`テーブルは、新しいカラムを追加**しなければなりません** - `push`は、ジョブがルーティングされているかどうかにかかわらず、`INSERT`の中でそれを名指しするため、マイグレーションされていないテーブルはあらゆるプッシュを失敗させます。先にマイグレーションしてから、バイナリをロールしてください（古いバイナリは自身のカラムを明示的に列挙し、新しいカラムを無視するため、その順序は安全です）:

```sql
ALTER TABLE jobs ADD COLUMN queue TEXT NULL;
CREATE INDEX idx_jobs_queue ON jobs(queue);
```

*（0.7.1で修正 - このノートは元々、フィルタしないデプロイにはマイグレーションが不要だと主張していました。）*

## 0.6.5 - 2026-07-21

### 追加

- **Stripeアダプターにおける、ホスト型のワンオフCheckout。**`SessionMode::OneOff`と空でない`price_refs`を伴う`Checkout::start_session`は、今ではホスト型のCheckout Session（`mode=payment`、price refごとに1つの明細項目、`allow_promotion_codes=true`）を作成し、`SessionPayload::StripeCheckoutRedirect`を返します。`amount_hint`のみのElementsの経路は変わりません。2つの形は、リクエストごとに選ばれます。
- **Stripe Managed Payments（Merchant of Record）のサポート。**`StripeProvider::with_managed_payments(true)` - あるいは`from_env()`での`STRIPE_MANAGED_PAYMENTS=true` - は、ホスト型のワンオフセッション作成時に`managed_payments[enabled]=true`を送信します。デフォルトではオフです。このフィールドはまるごと省略されるため、登録していないアカウントへの影響はありません。
- **`Checkout::session_status`。**新しいトレイトメソッド（デフォルト: `PaymentError::NotSupported`）で、セッションのプロバイダー側の状態を、新しい中立な`CheckoutSessionState`（`Open`/`Complete { paid, payment_ref, amount_total }`/`Expired`）として報告します。Stripeの実装は`GET /v1/checkout/sessions/{id}`をマッピングします。`payment_ref`は、ミラーテーブルとの突き合わせのために、セッションのPaymentIntent idを運びます。これは、リダイレクト復帰ページと消込スイープのための、サーバーサイド検証のプリミティブです。
- **`Promotions`ケイパビリティトレイト。**`create_promotion_code`は、事前作成されたクーポンから、顧客限定で、任意に期限を持ち、償還回数の上限が付いたコードを発行します。新しい`PaymentProvider::as_promotions()`（デフォルトは`None`）経由でクエリされます。Stripe（`POST /v1/promotion_codes`）とモックに実装されています。
- **上記に対応するための、`MockPaymentProvider`の拡張。**あらゆる`start_session`リクエストを記録し（`recorded_sessions()`）、セッションidごとに`session_status`をスクリプト化し（`script_session_status()` - スクリプト化されていない既知のセッションは`Open`を、未知のidは`NotFound`を報告します）、記録済みリクエストを伴う`Promotions`を実装します（`recorded_promotion_requests()`）。

## 0.6.4 - 2026-07-17

### 修正

- **Eloquentの集計は、データベースバックエンドをまたいで一貫してデコードされます。**生成される`count`、`sum`、`avg`、`min`、`max`の式は、今では1つの安定した内部結果エイリアスを使います。PostgreSQLは、そのドライバーが集計カラムをSQLiteとは異なる形でラベル付けすることによる偽のゼロや`None`をもう返しません。カラムの欠落や型の非互換によるエラーは、今ではサイレントにデフォルト値化されるのではなく、伝播するようになりました。
- **一括削除は、呼び出し元が指定するテーブル式を使えません。**実行可能な削除SQLは、常にモデルの検証済み静的な`M::TABLE`からターゲットを導出します。従来の公開レンダラー引数はソース互換のまま残りますが、削除ターゲットをリダイレクトしたり注入したりすることはできません。

## 0.6.3 - 2026-07-15

### 追加

- **型付きの生の読み取りが、トランザクションに固定されたコネクション上に留まれるようになりました。**`Transaction::backend()`はアクティブなバックエンドを公開し、`Transaction::query_all(Statement)`は、`QueryExecuted`の計装を保ったまま、型付きの集計やカスタムSQLをトランザクションを通じて実行します。ロックスコープの判定が計算済みの結果カラムに依存する場合でも、アプリケーションはもう、プールレベルのクエリや非公開のエグゼキューターへのアクセスを必要としません。

## 0.6.2 - 2026-07-15

### 修正

- **バインドされた生の述語は、バックエンドに中立です。**Eloquentの`filter_raw`と`where_raw`は、今ではあらゆるデータベースバックエンドで、可搬な`?`バインドマーカーを受け付けます。PostgreSQLでのレンダリングは、先行する述語、リレーションシップのサブクエリ、HAVING句、UNIONの各項をまたいで、それらを単調増加する`$N`の位置へ振り直します。既存の番号付きPostgreSQLフラグメントは、そのローカルなマーカー順によって正規化される一方、スタイルの混在やバインド数の不一致は、I/Oの前にバリデーションで失敗します。SQLを意識したスキャナーは、クォートされた文字列、識別子、コメント、ドル記号でクォートされた本体の内側にある疑問符を保持します。`??`は、バインドされた生のフラグメントの中で、リテラルな疑問符演算子を出力します。

## 0.6.1 - 2026-07-15

### 追加

- **監督下にある、観測可能なセッションクリーンアップ。**`SessionMiddleware::install`は、設定可能な`SESSION_GC_INTERVAL`の周期（デフォルトは1時間）を使い、`session_gc_metrics()`は、保護された運用系サーフェス向けに、プロセスローカルな実行回数、成功、失敗、削除された行数、最終結果のタイムスタンプを公開します。
- **上限付きのスライディングセッションのタッチ。**`SESSION_TOUCH_INTERVAL`は、アクティビティ書き込みの最小周期（デフォルトは5分）を制御し、アクティブなセッションがタッチの間に失効しないよう、セッション寿命の半分を上限としてクランプされます。

### 修正

- **状態を持たないリクエストは、もう永続的なセッションを作成しません。**有効なセッションクッキーを持たないリクエストは、セッションストアの読み書きを一切行わず、ハンドリングが状態を作らない限りセッションクッキーも受け取りません。既存のクリーンなセッションは、無条件のupsertとクッキーの入れ替わりを避け、旧来のクッキーは次のリクエストで移行し、裏側の行が失効しているクッキーは、空のセッションを再作成することなくクリアされます。

## 0.6.0 - 2026-07-10

### 追加

- **後方互換のデフォルトを保ったまま、オプトインになったフレームワークのサブシステム。**ファイルシステムストレージ、SQLite/Postgres/MySQLのデータベースドライバー、MariaDBのベクトルドライバー、そしてWeb Pushは、今では明示的なCargoフィーチャーを持ちます。既存のデフォルトビルドはこれらすべての機能を保持し続け、`default-features = false`の利用者は、ドライバーをゼロ個選ぶことも、使用するストレージ/データベース/ベクトル/プッシュの表面だけを選ぶこともできます。実行可能なフィーチャーマトリクスは、ドライバーゼロ、個別ドライバー、Nation X最小構成、デフォルト、全フィーチャーの各プロファイルを検証します。
- **生のP-256 VAPID秘密鍵インポート。**`VapidKey::from_bytes`は、既存のPKCS#8 PEMインポート/エクスポート経路に加えて、検証済みの32バイトビッグエンディアンP-256スカラーを受け付けます。

### 変更

- **VAPIDのJWTは、P-256で直接署名されるようになりました。**Web Pushは今では、RFC 8292のES256ヘッダー/クレームを直列化し、`p256`で署名します。生成される鍵、PEMのラウンドトリップ、公開鍵エンコーディング、24時間の寿命上限は保ったまま、汎用のJWT依存を取り除きました。
- **セキュリティ関連の依存関係の更新。**bcryptやammoniaを含む、脆弱なフレームワークの依存関係を更新し、シンタックスハイライトは維持したまま、Comrakの有効フィーチャーを絞り込みました。
- **Rust 1.91.1が、このリリースのMSRVです。**ワークスペースのすべてのパッケージが同じ`rust-version`を宣言し、生成されるDockerfileは対応するビルダーイメージを固定し、フルのリリースゲートは、まさにRust 1.91.1のツールチェーンでサポート対象のファイルシステムプロファイルをコンパイルします。
- **OpenDAL 0.58のセキュリティピン留め。**ファイルシステムフィーチャーは、公式のApache OpenDALコミット`ae99a3b016e354a1b2bb2baf0c70f9f9e134970a`にちょうど基づく最小限のフォークである、`eas4ai/opendal`のコミット`88717391eb72c9839d3f8e79fccad9f22fc3a1b4`をピン留めします。このフォークは、下流の利用者が公式のApache Reqsignコミット`b49cd2996b9d2d9944e84481f8835ff55b188b97`と`quick-xml` 0.41.0を解決できるよう、OpenDALコアとS3、GCS、Azure Blobが使うReqsignの宣言だけを変更しています。依存リポジトリのルートのCargoパッチは利用者へ伝播しないため、フォークが必要です - そうしなければ、公開される依存グラフが、脆弱な`quick-xml` 0.38/0.40を復活させてしまいかねません。

### 修正

- **アトミックなリリースバージョンメタデータ。**リリースのバンプは今では、`workspace.package.version`とバージョン付けされたすべての内部パス依存を、1つの検証済み操作で更新し、影響を受けるすべてのマニフェストをステージし、リリース前に`cargo check --workspace`で一時的な`0.6.0`ワークスペースを検証します。リリースバージョンは、プレリリースの数値に先頭ゼロを許さないルールを含め、厳密なSemVer 2.0として検証されます。バージョンに依存しない使い捨てのベアリモートスモークテストは、現在のソースとすでに`0.6.0`であるソースの両方から、後続のパッチリリースを導出し、ゲートの前にステージ済み/未ステージ/未追跡のリリースツリーを拒否し、タグが拒否されたときにコミット/タグの公開がアトミックに両方のrefをロールバックすることを証明し、実際のリモートに触れることなく通常のリリース手順を証明します。リリースバージョンは、プレリリースの遷移を含め、SemVerの優先順位に従って増加しなければなりません。スモークビルドの成果物は、呼び出し元の`CARGO_TARGET_DIR`が何であれ無視して、常に一時的なワークスペースの内側に留まります。
- **Rustdocが、サポート対象のあらゆるフィーチャー境界をカバーします。**OAuthモジュールは、公開の`OAuthAuth::complete`にリンクし、実行可能なマトリクスは、依存関係なしで、ドライバーゼロ、デフォルト、全フィーチャーのrustdocをビルドします。
- **ファイルシステムのストリームバリデーションが、セッションスコープになりました。**ローカルファイルシステムのライター、リスター、コピアーは、チャンクや項目ごとに1回ではなく、最初のI/Oの前に一度だけパスを解決し閉じ込めます。一方、アクティベートされたクローズ/アボート操作は、クリーンアップのために常にバックエンドへ到達します。既存のトラバーサルとシンボリックリンクの閉じ込めは、信頼できるファイルシステムに対しては引き続き強制されます。canonicalize-then-openのチェックは、ツリーを並行して変更するプリンシパルに対する競合を排除しません。

### セキュリティ

- **リリースゲートは、フェイルクローズします。**`release.sh`は、マニフェストを編集したりコミット/タグを作成したりする前に、正規のフルゲートへ委譲します。そのゲートは常に`cargo audit`を実行し、`cargo-audit`バイナリの欠落をエラーとして扱い、監査の失敗があれば必ず停止します。また、隔離された下流のファイルシステム利用者をビルドし監査して、正確なOpenDAL/Reqsignのソースリビジョンと、0.41未満の`quick-xml`が存在しないことをアサートします。新しいアドバイザリーのignoreは追加されていません。

## 0.5.10 - 2026-07-03

### 修正

- **`generate-types`は、もう自己参照する構造体を落としません。**自分自身の型を参照するフィールドを持つ構造体（`children: Vec<Self>`を持つツリーノード、たとえばスレッド型コメントのビューなど）は、型の依存グラフに自己エッジを作り、その入次数をゼロより上に固定していたため、Kahnのトポロジカルソートはそれを一度も出力しませんでした - それを参照するあらゆるインターフェースに、`svelte-check`/`tsc`を失敗させる宙に浮いた型名を残していたのです。自己エッジは今ではソートの前に取り除かれ、参照の循環（相互再帰）に捕らわれた構造体も、TSのインターフェースは宣言順に関係なく互いを参照できるため、落とされるのではなく任意の順序で出力されます。

## 0.5.9 - 2026-07-01

### 追加

- **`MAIL_FROM_NAME` - 認証フローのメールにおける、オプションの表示名。**メール確認、パスワードリセット、パスワード変更の各メーラブルは、`MAIL_FROM_NAME`が設定されている場合、今では`From`ヘッダーを`"Name <address>"`としてレンダリングします（送信時に読み取られるため、キューのserdeラウンドトリップを生き延びます）。`MAIL_FROM`は素のアドレスのままです。`MAIL_FROM_NAME`を未設定または空のままにしておくと、以前の素アドレスの挙動が保たれます。呼び出し箇所への変更はありません - メーラブル自身が環境変数を読み取ります。

## 0.5.8 - 2026-06-30

### 修正

- **`generate-types`のルートヘルパーは、常に有効なTypeScriptです。**あるモジュール内の複数のルートが1つのハンドラを共有する場合（たとえば、多数のfavicon/アセットURLをマッピングする`static_files::serve`の許可リストなど）、最初のものはハンドラ名を保持し、残りはルートパスから導出されたキーを得ていました - しかし、そのパスは部分的にしかサニタイズされておらず（`/ { } -` → `_`）、ファイル拡張子がキーに`.`を漏らしてしまっていました: `favicon_16x16.png: (...) => ...`。これはプロパティ名ではなくメンバーアクセスであるため、`tsc`/`svelte-check`は生成された`routes.ts`を拒否していました。導出されたキーは今では正当な識別子へサニタイズされます - 英数字以外の文字はすべて`_`になり、先頭が数字の場合は接頭辞が付きます - そのため`favicon-16x16.png` → `favicon_16x16_png`、`2fa.json` → `_2fa_json`となります。一意なハンドラ名には変更がありません。

## 0.5.7 - 2026-06-30

### 修正

- **`generate-types`は、もう宙に浮いた型参照を出力しません。**`InertiaProps`/`Data`をderiveしていない構造体（あるいはジェネレーターから見えない外部の型）を型に持つpropフィールドは、素の識別子として出力されていました - たとえば`user: UserInfo`のように - そのインターフェースが決して書き出されないため、`tsc`/`svelte-check`を失敗させるTypeScriptを生んでいました。そのような参照は今では`unknown`へ劣化するため（`user: unknown`、`Vec<T>` → `Array<unknown>`、`Option<T>` → `unknown | null`）、生成される出力は常に型チェックを通り、`generate-types`は、未解決の型とそれを参照しているフィールドの名前を挙げ、修正方法（`InertiaProps`/`Data`をそれにderiveすること）を添えた警告を出力します。ジェネリックパラメータと、解決済みのネストしたInertiaProps/Dataの型には影響がありません。

## 0.5.6 - 2026-06-29

### 変更

- **Sign in with Apple: RS256 JWKS検証。**`suprnova-apple-rs`をv0.3.1にバンプしました - AppleのIDトークンは今では、構造的に信頼されるのではなく、Appleが公開しているJWKS（RS256）に照らして検証されます。

## 0.5.5 - 2026-06-28

### 追加

- **`MagicLink`トークンパーパス。**パスワードレスのマジックリンクサインイントークン向けに、認証フローの`TokenPurpose`enumへ新しい`MagicLink`バリアントを追加しました。

## 0.5.4 - 2026-06-28

### 変更

- **組み立て可能なOAuth完了処理。**汎用のOAuth完了処理を、`verify_oauth_identity`（検証してアイデンティティを解決する）と、薄い`complete`に分割しました。これにより、アプリは、セッション完了の副作用をフルに引き起こすことなく、OAuthのアイデンティティを検証できます。

## 0.5.3 - 2026-06-28

### 修正

- **ワークスペースのバージョンメタデータを修正。**v0.5.2は、その`Cargo.toml`のバージョンバンプがステージされる前にタグ付けされプッシュされてしまったため、プッシュされたv0.5.2タグは依然として`version = "0.5.1"`のままでした。v0.5.3は、正しいワークスペースバージョンでリリースを切り直します - コードの変更はありません（v0.5.2のOAuth分割への影響はありません）。

## 0.5.2 - 2026-06-28

### 変更

- **組み立て可能なApple完了処理。**Apple Sign-Inの完了処理を、汎用のOAuth分割を反映する形で、`verify_apple_identity` + 薄い`complete_apple`に分割しました。（注: プッシュされたv0.5.2タグは、古い`0.5.1`のバージョンフィールドを抱えています - v0.5.3で修正されました。）

## 0.5.1 - 2026-06-28

### 変更

- **Appleクレートをリネーム。**Apple依存関係を、リネームされた`suprnova-apple-rs`リポジトリへ向け直しました。

## 0.5.0 - 2026-06-28

### 追加

- **Sign in with Apple。**AppleのためのOAuthトークン交換 + IDトークン検証 + ユーザーupsert。Appleのwell-knownエンドポイントと`form_post`レスポンスモード。`OAuthProviderConfig`上のApple固有のフィールド。アプリが`apple`への直接依存なしにApple Sign-Inを設定できるよう、`AppleKeyPair`が再エクスポートされました。

### 修正

- Appleの認可URLからPKCEパラメータを省くようにしました（存在するとAppleがリクエストを拒否するため）。

### 依存関係

- `torii`のマジック認証修正を取り込み、`apple-rs` v0.3.0を追加しました。

## 0.4.1 - 2026-06-26

### パフォーマンス

- リクエストごとの`Vec`再割り当てをなくすため、`MiddlewareChain`のサイズを事前に確保するようにしました。

### 修正

- 並行テスト実行下でも衝突しないよう、メンテナンスのdownファイルのパスを堅牢にしました。

### 文書

- フレームワークのドキュメント例をコンパイルチェックするようにし（`ignore` → `no_run`）、配布に関するノートをタグ付けされたGitHub Releasesと整合させ、`docs/`ツリー全体を無視するようにしました。

## 0.4.0 - 2026-06-22

### 変更

- **配布はgitで追跡されます。タグにピン留めする必要はありません。**スキャフォルドされたアプリは`suprnova = { git = "…/suprnova.git" }`に依存し、デフォルトブランチを追跡します。更新は`cargo update -p suprnova`で取得してください。バージョンはchangelogのために、タグ付けされたGitHub Releases（`v0.4.0`など）として公開されますが、`Cargo.lock`はすでに正確に解決されたコミットをピン留めしています - そのため、`tag`や`rev`を手作業でピン留めしなくても、ビルドは再現可能なままです。インストールのドキュメントは、もうコミットのピン留めを更新方法として提示しません。

## 0.3.0 - 2026-06-21

### 追加

- **Eloquentの読み取りに対するクエリ計装** - `Builder::get`、`Model::find`、`find_many`、`all`は、今では`QueryExecuted`を発火するため、モデルのSELECTとeager-loadのクエリが、書き込みや生クエリと並んで`DB::listen`とインメモリのクエリログに現れます。計装された`ExecutorChoice::statement_all`という読み取り終端を追加します。
- **リソースルートの認可** - `ResourceRoutes::authorize_resource::<U, R>()`は、慣例的な権限チェックを、生成されるすべてのリソースルートへルートごとのミドルウェアとして取り付けます（Laravelの`authorizeResource`互換）。アクション→権限のマッピングは、`index`/`show` → `view`、`create`/`store` → `create`、`edit`/`update` → `update`、`destroy` → `delete`です。あらゆるコントローラー本体が`Gate::authorize`を覚えていることに頼るのではなく、1回の呼び出しで7つのアクション全体の表面をゲートします。
- **アトミックなレート制限ヒット** - `RateLimiter::hit_and_check(key, max, decay)`は、固定ウィンドウを1回のラウンドトリップでインクリメントし検査して、バケットが今その上限を超えているかどうかを返します（`i64::MAX`は無制限を意味します）。
- **一定時間比較ヘルパー** - Webhook署名検証のための`constant_time_eq(a, b)`（subtleクレートに支えられています）です。`WebhookHandler::verify`のドキュメントは今では、一定時間でのダイジェスト比較を義務付けています。
- **Inertiaクライアントを3.4.0へ** - Svelte/React/Vueのスキャフォルドは今では、`@inertiajs/{svelte,react,vue3}`を（`3.1.1`から）`^3.4.0`に固定し、`router.poll`モード、動的な`usePoll`、`Inertia.once`、InfiniteScrollのキャンセル修正、awaitされるFormの`onSuccess`を取り込みます。サーバーはすでに、3.4.0のページオブジェクトとヘッダーの表面全体（once-props、prepend/deep-mergeのスクロール系列、`matchPropsOn`、rescued/sharedのprops）を発しているため、これはプロトコルの変更を伴わないクライアントの追随バンプです。
- **オプションのコネクション上限** - `SERVER_MAX_CONNECTIONS`（およびプログラム的な`Server::max_connections(n)`）は、acceptループにセマフォを設けて、同時にアクティブなコネクションを制限し、TCPレベルでバックプレッシャーをかけます。未設定 - あるいは`0` - の場合はコネクションを無制限のままにします（デフォルトで、変更なしです）。リバースプロキシや`LimitNOFILE`と組み合わせるための最後の砦であり、上流のレート制限の代替ではありません。
- **リダイレクト追従のオプトアウト** - `RequestBuilder::no_redirects()`は、リクエストを追従しないHTTPクライアントへ通し、`3xx`を追いかけるのではなくそのまま返します。リクエストURLが信頼できない入力の影響を受ける場合に使い、リダイレクトを介したSSRFのベクトル（悪意のあるエンドポイントが内部やクラウドメタデータのホストへリダイレクトすること）を塞いでください。デフォルトのクライアントは、一般的なクライアントの慣例に従って、引き続きリダイレクトに追従します。

### セキュリティ

- **リソースルート**は、認可レジストリの型消去ダウンキャストに対してパニックする代わりにフェイルクローズするようになり、`authorize_resource`の拒否/未認証のリクエストは、ハンドラが実行される前に拒否されます。
- **レートリミッター**は、アトミックにインクリメントして比較すること（`hit_and_check`）によって、固定ウィンドウのcheck-then-hit競合を塞ぎます。
- **キューの`RateLimited`ミドルウェア**は、今では、別個の`too_many_attempts` + `hit`のペアではなく、そのアトミックな`hit_and_check`を通じてジョブを通すため、並行するワーカーがどれもインクリメントする前に全員が予算チェックを通過して`max_attempts`を超えて過剰に通してしまうことはもうありません。
- **アップロードバリデーター**（`mimetypes`/`mime`）は、クライアントが提供する`Content-Type`を信頼するのではなく、アップロードされたバイト列をコンテンツスニッフィングします。
- **ファイルシステムのパスガード**は、以前の字句的な`../`/絶対パス/UNCチェックに加えて、パスを正規化することで、ストレージルートの外へのシンボリックリンクトラバーサルを捕捉します。
- **認証**は、パスワードレスログインのタイミングオラクルを塞ぎます - マッチしたもののパスワードを持たないアカウントにパスワードが与えられた場合、Eloquentとデータベースのどちらのユーザープロバイダーでも固定コストの検証を実行するようになりました - そして`dummy_verify`は設定済みのハッシャーを駆動するため、マッチしないユーザーの経路は一定時間になります。
- **Eloquent**は、`pluck`/`value`/`pluck_keyed`/`sole_value`と`sum`/`avg`/`min`/`max`の射影経路で、カラム識別子を検証します。
- **支払い** - モックプロバイダーのベリファイアは、開発環境の外ではフェイルクローズし、Webhookの送信元IPは、生の`X-Forwarded-For`ヘッダーではなく`TrustedProxiesConfig`（`req.ip()`）を通じて解決されるようになりました。
- **ファイルシステムのパスガード**は、書き込みターゲットがまだ存在しない場合、今では最も近い*実在する*祖先まで辿るようになり、直近の親が欠けた中間シンボリックリンクを仕込むことでガードをすり抜けるシンボリックリンクエスケープを塞ぎます。
- **`DB::init_with`**は、接続する前に環境を検証するようになり（`DB::init`と一致します）、そのエントリーポイントを通じて開発用のSQLiteフォールバックが本番環境でサイレントに起動することはもうありません。
- **静的ファイル配信**は、`.`/`..`トラバーサルだけでなく、ドットファイル（`.env`、`.git/config`、`.htpasswd`、先頭が`.`のあらゆるセグメント）も拒否します。
- **支払いのWebhook**は、同じ未処理イベントの並行リトライを、`FOR UPDATE`ロック + 再チェックで直列化し、ミラーテーブルの一意制約違反を、無害な適用済みとして扱います。`payments_subscription_items`に`UNIQUE(subscription_id, provider_item_id)`が追加されました。
- **RBAC**は、モデルの判別子を完全修飾型名にデフォルトするようになり、末端の名前を共有する2つの認証可能な型が、互いのロール/権限を継承してしまうことはもうありません。
- **`invalidate_session()`**は、（単にフラッシュするだけでなく）セッションidをローテートし、セッション固定の隙間を塞ぎます。キューの`WithoutOverlapping`ミドルウェアは、ジョブがパニックしたときでもキャッシュロックを解放します。
- **メールプロバイダー**は、web-pushクライアントと同様に、エラーレスポンスのボディ読み取りに上限（8 KiB）を課すため、悪意のあるエンドポイントが送信側のメモリを圧迫することはできません。
- **Web push**は、デフォルトのクライアントでHTTPリダイレクト追従を無効化するため、攻撃者の影響を受けたpushエンドポイントが、通知POSTを内部やクラウドメタデータのホストへ`3xx`でリダイレクトすること（SSRF）はもうできません。リダイレクトは今では、サイレントに追従されるリクエストではなく、拒否されたpushとして表面化します。
- **Stripeアダプター**の`Debug`は、Webhook署名シークレットを伏せ字にし、*さらに*（認証ヘッダーにAPIシークレットキーを運ぶ）`stripe::Client`にはプレースホルダーを出力するため、上流クライアント自身の`Debug`がどうであれ、どちらのシークレットも`StripeProvider`の`{:?}`を通じてログに届くことはありません。
- **Stripeアダプター**の`from_env`は、存在はするが空である認証情報を拒否するようになり、空の（つまり偽造可能な）Webhook HMACシークレットを持つクライアントを構築するのではなく、フェイルクローズします。
- **OAuthのメール検証**は、未知のプロバイダーに対してフェイルクローズします: `email`は運ぶが`email_verified`フラグを運ばないuserinfoペイロードは、もう検証済みとして扱われません。未知のプロバイダーは、今では`email_verified: true`をアサートするか、検証済みメールのエンドポイントを公開しなければならず、これは、アカウントをメールでキーにするアプリに対する、アカウント連携/乗っ取りのベクトルを塞ぎます。Google（明示的な`true`のみ）とGitHub（`/user`契約による検証）には変更がありません。

### 修正

- **ネストしたeager loading**（`with(["posts.comments"])`）は、今では定数個のクエリになります - 末尾のセグメントは、親ごとに1クエリ（N+1）ではなく、すべての親をまたぐ1つのバッチ化されたINクエリでロードされます。
- **`where_has`/`where_doesnt_have`**は、クロージャのカラムをターゲットテーブルで修飾するようになり、pivotとターゲットの両方に存在するカラムが、多対多のリレーションでambiguous-columnエラーを生むことはもうありません。
- **ソフトデリートの`delete`/`force_delete`/`touch`とファクトリーの`persist`**は、プライマリプールへフォールバックするのではなく、モデルの`#[model(connection = "…")]`ルーティングを尊重するようになりました（`restore`や他の書き込み経路と一致します）。
- **JSON:APIの`Maybe::Missing`**は、衝突しない番兵値を通信に使うようになったため、`{"__missing__": true}`という形をしたユーザーデータが、サイレントに取り除かれることはもうありません。
- **キューに入れられた通知**は、ワーカー上で再チェックされる`should_send`（チャネルごとの拒否権）と`after_sending`を尊重するようになりました - 以前は同期経路だけがそうしていました。
- **リリースされたジョブ**は、元のものをACKする前にリトライ用のコピーをプッシュするようになり、一時的なドライバーのプッシュエラーがジョブを失わせることはもうありません。
- **Paddleのadjustment（返金）Webhook**は、adjustment idの下にゼロ金額の行を挿入するのではなく、参照されているトランザクションidにミラーの更新をキーづけし、金額は`data.totals`から読み取るようになりました。
- **クエリ文字列を伴うSQLite URL**（`sqlite://db.sqlite?mode=rwc`）は、有効な単一クエリのコネクションURLと、クリーンなディスク上のファイル名を構築するようになりました。
- **HTTP**は`Accept`の`q`値を`[0,1]`にクランプし、ボディが事前にバッファされていた場合でも`FormRequest`の`max_body_bytes`を強制するようになりました。**WebSocket**の設定は、`max_missed_pings < 2`を拒否するようになりました（1では、最初のpingであらゆるコネクションが閉じられていました）。
- **Cron**は、日と曜日の両方が制限されている場合にOR意味論を使うようになりました（Vixie/POSIX互換）。Markdownの`plain_text`/抜粋は、意図的なスペース入り句読点を保持します。`CachedEvaluator`はキャッシュの増加に上限を設けます。`SupervisorRegistry::start_all`は、2回目の呼び出しで二重にspawnすることはもうありません。テストコンテナは、ポイズニングされたロックからその場で回復します。
- **スーパーバイザーの再起動バックオフ**は、少なくとも60秒の上限だけ稼働し続けた実行の後、100msの下限にリセットされるようになりました。そのため、長い期間健全に動いていたデーモンが終了した場合、以前の失敗の連発の間に積み上がったバックオフを引き継ぐのではなく、速やかに再起動します。実行が一度もその閾値に達しないクラッシュループは、依然として上限まで上り詰めるため、このリセットが不安定なスーパーバイザーを覆い隠してしまうことはありません。
- `filter_op`（演算子は許可リストで検証されます）、署名付きURL（Laravelのデフォルトの絶対署名とバイト互換ではありません）、`UniqueIdKind::is_valid`（呼び出し元向けのヘルパーであり、`find`に自動配線されているわけではありません）、識別子の長さ上限（64ではなく128）に関する、古くなったドキュメントを修正しました。

### ドキュメント

- リソースルートの認可（`authorize_resource`）をルーティングと認可の章にドキュメント化し、アトミックな`hit_and_check`カウンターをレート制限の章にドキュメント化しました。

## 0.2.0 - 2026-06-21

ロールベースのアクセス制御、Markdownコンテンツ/ドキュメントレンダリングパイプライン、そしてネイティブな静的ファイル配信を追加します。

### 追加

- **Tier-2 RBAC** - `HasRoles`トレイト。`role_has_permissions`ジョインによるロール+権限。`PermissionMiddleware`/`RoleMiddleware`（どちらもフェイルクローズ/デフォルト拒否）。`CreateRbacTables`マイグレーション。そして`create_role`/`create_permission`/`give_permission_to_role`ヘルパーです。
- **コンテンツレンダリング** - Markdownレンダリングとドキュメントビルドパイプラインです: `MarkdownRenderer`、`build_docs`、`DocsCatalog`/`DocsChapter`、見出し抽出、`slugify_heading`。レンダリングされたHTMLはサニタイズされます（comrak + syntect + ammonia）。
- **ネイティブな静的ファイル配信** - Webルートで`public/`ディレクトリを配信するための`StaticFiles::public()`フォールバックハンドラです。アプリ内で手作りされていたアセットごとの許可リストコントローラーを置き換えます。

### 修正

- 新しく生成されたアプリは、フレームワークレベルの`time = 0.3.47`互換ピン留めを継承するようになり、まっさらなスキャフォルドの依存関係解決における`time 0.3.48`由来のRust 1.96コヒーレンス衝突を避けます。

### ドキュメント

- 出荷済みの2つのスターターキット - **Nebula**（Breezeクラスの認証）と**Pulsar**（プロダクトサイト + コミュニティ） - を、マニュアル、README、ロードマップ全体にドキュメント化し、出荷済みの表面を軸にロードマップを再構成し、ドキュメント全体のバージョン参照を整合させました。

## 0.1.0 - 2026-06-10

Suprnovaの最初のリリースです。Suprnovaは、Rust向けのLaravelに着想を得たWebフレームワークで、Kitからフォークされ、独自の方向へ進んでいます。現時点での互換目標はLaravel 13.xです。

このリリースは、gitによる配布モデルを使います: フレームワークの利用者は`suprnova = { git = "https://github.com/eas4ai/suprnova.git" }`に依存し、CLIは`cargo install --git`でインストールします。

### 追加

#### HTTP、ルーティング、ミドルウェア

- ルートグループ、プレフィックス、パラメータ制約、名前付きルートを備えた`Router`
- `routes!`マクロによる、コンパイル時検証されるルート登録
- 7つの標準ルートを生成する、リソースルーティング（`Router::resource`）
- 署名付きURL（`url::signed_route`/`url::temporary_signed_route`フリー関数、および`Redirect::signed_route`/`Redirect::temporary_signed_route`）
- リダイレクトヘルパー - `Redirect::to`、`Redirect::back`、`Redirect::route`、`Redirect::with_input`、`Redirect::with_errors`、`with_flash`
- グローバル、グループ、ルートごとの層を持つミドルウェアトレイト
- 組み込みミドルウェア - CORS、CSRF、セッション、リクエストタイムアウト、リクエストID、throttle/ログインthrottle、署名付きURL検証、認証済み、メール確認済み、ブルートフォース対策
- アボートヘルパー（`abort`、`abort_unless`、`abort_if`）
- `suprnova::handle_request(...)` - ルーター + ミドルウェアチェーンに対して単一のhyperリクエストを処理する、公開アダプター

#### Inertia.jsフロントエンドブリッジ

- TypeScriptの型出力を伴う`#[derive(InertiaProps)]`
- コンパイル時のコンポーネント検証を伴う`inertia_response!`マクロ
- 3つのファーストクラスのスターターフロントエンド - **Svelte 5**（runes有効）、**React 19**、**Vue 3.5** - いずれもInertia 3.1.1 + Vite 8 + Tailwind v4の上に構築
- 部分リロード（`only`/`except`）、遅延プロパティ、永続レイアウト、暗号化された履歴、スクロール位置の保持
- ページネーター → Inertiaのprop配線のための`Inertia::paginate(component, key, paginator)`

#### Eloquent風ORM（SeaORMベース）

- SeaORMエンティティとユーザー向けのEloquent構造体を一度に生成する`#[suprnova::model]`アトリビュートマクロ
- フルセットの`Model`トレイト - `create`、`find`、`find_or_fail`、`find_many`、`all`、`query`、`save`、`update`、`delete`、`force_delete`、`refresh`、`fresh`、`replicate`、`replicate_into`、`increment`/`decrement`、`destroy`、`is`/`is_not`、`to_array`/`to_json`
- `Attrs`エンベロープによる、fillable/guardedなマスアサインメント
- 22種類のアトリビュートキャスト - 真偽値、整数、浮動小数点数、日付、enum、ハッシュ化、暗号化、JSON、コレクション、金額、タイムゾーン付き日時
- `#[suprnova::model]`によるアクセサ/ミューテータ
- 自動タイムスタンプ（`created_at`、`updated_at`）
- `force_delete`、`restore`、`trashed`、`only_trashed`、`with_trashed`を伴うソフトデリート（`deleted_at`）
- 11種類のリレーション - `HasOne`、`HasMany`、`BelongsTo`、`BelongsToMany`、`HasOneThrough`、`HasManyThrough`、`MorphOne`、`MorphMany`、`MorphTo`、`MorphToMany`、`MorphedByMany`
- ファミリーごとのmorph enumと、`APP_KEY_PREVIOUS`ローテーションを伴うmorphレジストリ
- `.with(...)`、`.with_count(...)`、`.load_missing(...)`によるeager loading
- `has`/`where_has`のための、相関EXISTSエンジン
- 16種類のライフサイクルイベント（retrieving、retrieved、creating、created、updating、updated、saving、saved、deleting、deleted、restoring、restored、force-deleting、force-deleted、replicating、trashed）
- inventoryによるメソッドごとの自動登録を伴う`Observer<M>`トレイト
- `#[scopes(M)]`によるローカルスコープ、`GlobalScope`によるグローバルスコープ
- `Collection<M>`のLaravel互換表面 - `pluck`、`key_by`、`group_by`、`where_in`、`first_where`、`contains_where`、`partition`など
- 3種類のページネーター - `paginate`（length-aware）、`simple_paginate`、`cursor_paginate` - いずれもLaravel形式のJSONへシリアライズ
- OOMを起こさない一括行イテレーションのための`chunk`/`lazy`/`cursor`
- `lock_for_update`/`shared_lock`による行レベルロック
- アドホックなクエリのための、`DynamicRow`を伴う`DB::table(...)`クエリビルダー
- セーブポイント、デッドロック時リトライ、複数コネクションでの読み書き分割を伴う`DB::transaction(...)`
- `DB::listen(...)` + `QueryExecuted`/`TransactionBegan`/`TransactionCommitted`/`TransactionRolledBack`イベント
- `Prunable`トレイト + `model:prune`コンソールコマンド
- `dump`/`dd`クエリヘルパーメソッド
- UUID/ULIDのプライマリキーのための`#[model(unique_id="...")]`

#### 認証

- `Authenticatable`トレイト + `EloquentUserProvider<M>`
- `Auth::attempt`、`Auth::login`、`Auth::user`、`Auth::user_or_fail`、`Auth::user_as<T>`、`Auth::logout`、`Auth::check`
- 複数の名前付き認証ガード（Webセッション、APIトークン）
- メール確認フロー - `EmailVerification`、`EnsureEmailVerifiedMiddleware`、署名付き確認URL、`EmailVerificationMail`
- パスワードリセットフロー - `PasswordReset`、throttleされたトークン、`PasswordChangedMail`、`PasswordResetLinkSent`イベント
- 二要素TOTP - 登録、検証、リカバリーコード、リプレイ保護
- ブルートフォース対策/ログインthrottle - IP + 識別子でキー化、`LoginThrottleMiddleware`
- 安定した不透明トークンによるremember-meクッキー
- 6種類の認証イベント - `LoginAttempted`、`LoggedIn`、`Authenticated`、`LoggedOut`、`PasswordResetLinkSent`、`EmailVerified`
- `github.com/eas4ai/suprnova-torii-rs`のToriiフォークに支えられたブラウザセッション

#### 認可

- `Gate`ファサード - `define`、`allows`、`denies`、`authorize`、`any`、`none`、`check`（同期・非同期の両バリアント）
- ポリシー登録のための`#[policy(Model)]`マクロ
- リソースルートの自動認可

#### 支払い

- プロバイダーに依存しない5トレイトの表面 - `Checkout`、`Payment`、`Subscription`、`CustomerStore`、`WebhookHandler`
- `PaymentProvider`という上位トレイト + `as_payment()`によるケイパビリティ照会
- DBミラー - `customers`、`subscriptions`、`subscription_items`、`payments`、`refunds`、`payment_webhook_events`（べき等性のためのUNIQUE）
- フロータグ付きの`SessionPayload`enum（ワンショット対サブスクリプション）
- ワークスペースクレートとしての、2つのリファレンスアダプター - `suprnova-payments-stripe`（ゲートウェイ、フルの`Payment`実装）、`suprnova-payments-paddle`（Merchant of Record、`Payment`実装なし）
- テスト用のモックプロバイダー

#### キュー、ジョブ、バッチ、チェーン

- `Job`トレイト - `handle`、`max_tries`、`backoff`、`timeout`、`fail_on_timeout`
- `Queue::push`、`Queue::push_later`、`Queue::push_unique`、`Queue::push_unique_later`
- ドライバー - `sync`、`null`、`redis`、`database`
- `JobMiddleware`トレイト - 6種類の組み込みミドルウェア
- バッチとチェーン - `Queue::batch(jobs).dispatch()`、流暢なチェーンビルダー、キャンセル、進捗追跡
- リプレイ機能付きの失敗ジョブストア
- グレースフルシャットダウン、設定可能な並行数、`catch_unwind`によるパニック回復、決着メトリクスを備えたワーカー
- キューイング、処理、失敗、リリース、ワーカーのライフサイクルをカバーする、12種類のキューイベント

#### ブロードキャストとWebSocket

- 型付きWebSocketエンドポイントのための`ws!()`マクロ + `Router::ws`
- `WsSocket`のSink/Stream分割
- `Supervisor`トレイトによる自動再起動スーパーバイザー
- `Channel`、`Private`、`Presence`の各チャネルを備えた`BroadcastHub`
- JSONエンベロープのプロトコル、presenceのjoin/leave/here、クラッシュ復旧を伴う設定可能なpresence TTL
- `EventDispatcher`への`Broadcastable`ブリッジ
- 設定可能なWS_TASKSドレインを伴う、pong不在時クローズのハートビート
- ルートごとのWebSocketミドルウェア
- 1 MiB/64 KiBのより安全なデフォルト + `WsConfig::generous()`ファクトリー
- オリジンポリシー + プロトコル違反時1011クローズ

#### 通知とメール

- `Notification`トレイト + `Notify::send(recipient, notification).await`
- メーラブル + Markdownテンプレートレンダリング
- データベース/メール/ブロードキャスト/web-pushの各チャネル
- VAPID署名 + RFC 8291 ECEペイロード暗号化（`suprnova-web-push`経由）
- VAPIDのsubject検証、retry-afterのパース、8 KiBの拒否ボディ上限
- 受信者の型付けのためのNotifiableトレイト

#### イベント

- 型付きイベントディスパッチャー - `EventFacade::dispatch`、`EventFacade::listen<E, L>`、`EventFacade::forget`
- キャンセル可能なsaving/updatingイベント（`EventResult::cancel`を返す）
- キュー可能なリスナー

#### ファイルシステム

- OpenDAL経由のマルチドライバーサポートを伴う`Storage::disk("name")` - local、S3、Azure、GCS
- move、copy、exists、size、mime、last-modified、prepend/append
- ストリーミングアップロードとダウンロード

#### キャッシュ

- `Cache::store("name")` + ドライバー登録
- ドライバー - memory、redis（上限付きconnect-timeout）、database、file
- `remember`、`forever`、`tags`、アトミックなincrement/decrement、ロック

#### ベクトルDB

- 4種類のドライバーを持つ`VectorDriver`トレイト - インメモリ、Qdrant（UUID-5 IDマッピング）、Pinecone（ネイティブな文字列ID）、MariaDBネイティブの`VECTOR(N)` + HNSWインデックス（11.7以降）
- コサイン/内積/ユークリッド距離

#### コンソールバイナリとCLI

- プロジェクトごとの`console`バイナリ - `php artisan`のRust版で、`#[suprnova::console::command]`経由でユーザー定義のコマンドを実行
- 型付き引数のための`#[derive(Command)]`
- `suprnova` CLI - `new`、`serve`、`migrate`、`db:sync`、`generate-types`、`key:generate`、`make:{controller,middleware,action,error,inertia,migration,task,command}`、`db:seed`、`model:prune`
- `--version`フラグ
- 3つのフロントエンドにまたがる、バックエンド + APIスターター向けのスキャフォルドテンプレート

#### フィーチャーフラグ

- スナップショット読み込みを伴う`DatabaseEvaluator`
- TTLを伴う`CachedEvaluator`
- `FeatureMiddleware`エクストラクター
- 管理用CRUD表面
- プロセス間でのサブ秒の伝播のための`FeatureSync`トレイト

#### スケジュール

- Cron式パーサー
- 組み立て可能な述語を伴う`Schedule::task(...)`
- シングルサーバーロック、重複実行の防止、ディスパッチ追跡
- `schedule:run`コンソールコマンド

#### バリデーション

- `validator` 0.20との統合
- `#[request]` + `#[derive(FormRequest)]`マクロ
- フォームごとのサイズ上限のための`#[form_request(max_body_bytes = N)]`
- ユーザーが書く`impl FormRequest`のためのオプトアウトである`#[form_request(custom_hooks)]`
- ライフサイクルフック - `authorize`、`after_validation`、`after_validation_async`

#### データベースドライバー

- SeaORMに支えられた、SQLite、Postgres、MySQL、MariaDBのサポート
- URLベースのドライバー検出
- マイグレーションシステム + `migrate`、`migrate:rollback`、`migrate:status`、`migrate:fresh`、`migrate:refresh`

#### HTTPクライアント

- `Http`ファサード - `RequestBuilder`を返す`get`/`post`/`put`/`patch`/`delete`。`.send().await`は`ClientResponse`を生成
- rustls TLS、30秒のデフォルトタイムアウト、`suprnova/<version>`のuser-agent
- `json`/`form`/`body`/`header`/`bearer_token`/`basic_auth`/`timeout`のチェーン可能なメソッド
- `RequestBuilder::retry(max_attempts, base_backoff)` - 一時的な失敗と5xxに対する指数バックオフ。`Retry-After`を尊重
- `fake_response(method, url_substring, status, body)` + `assert_sent`/`assert_not_sent`を伴う`Http::fake(|| async { ... }).await`テストガード

#### 暗号化

- `Crypt`静的ファサード + `EncryptionKey`（`crypto::*`）。12バイトのランダムノンスを伴うAES-256-GCM
- `encrypt_string`/`decrypt_string`/`encrypt<T>`/`decrypt<T>`
- クロスプロトコルのリプレイを防ぐ`CryptPurpose`のAADバインディング
- `APP_KEY_PREVIOUS`ローテーション
- 新しいキーを発行するための`suprnova key:generate` CLIコマンド

#### テスト

- `#[suprnova_test]`非同期テストマクロ
- 並行実行に安全なインスタンスを伴う`TestDatabase::fresh::<Migrator>()`
- テストごとのモックのための`TestContainer::bind`
- HTTPテストヘルパー - `Test::get`、`Test::post`、JSON/form/multipart
- キュー/メール/通知/イベントのフェイク
- `assert_emitted`、`assert_dispatched`、`assert_dispatched_times`

### 変更

- 認証確認とパスワードリセットのフローは、今ではTorii内部ではなく、設定済みのユーザープロバイダーを通じて動作するようになりました。
- 生成されるアプリは、`get_auth_password`を実装しなければなりません。スキャフォルドされたサンプルは、ログインを常にサイレントに失敗させるままにするのではなく、今ではっきりと失敗するようになりました。
- ローカルのリリースゲートは`scripts/release.sh`に配線されており、このリポジトリには、fmt、clippy、テスト、ドキュメント、フィーチャービルドのための、強制されるpre-pushフックが含まれています。
- スキャフォルドされた開発用ポートのドキュメントは、`dev:tls`と`--with-portless`のドキュメント化と共に、現行のバックエンド/フロントエンドのデフォルト（`8765`/`5765`）へ移されました。
- `MAIL_FROM`は、確認やリセットのトークンが発行される前に検証されるようになり、メール設定が無効な場合に認証フローの行が孤児になることを避けます。

### 修正

- Reactのスキャフォルドテンプレートが、リリースされたスターターからずれていた問題を修正しました。
- ルートのルートグループが、もう重複した`//`パスを生成しないようにしました。
- リテラルパスのリダイレクトが、今では意図したルーティング経路を通じてディスパッチされるようにしました。
- ブロードキャストのファンアウトテストが、今では`track`/`untrack`の結果を扱うようにしました。
- メールログドライバーは、レンダリングされたテキスト本文を出力するようになり、確認とパスワードリセットのリンクが、ローカル開発のログに現れるようにしました。
- パスワードリセットのテストカバレッジが、セッションとremember-meの失効挙動を固定するようにしました。

### 補足

- **配布モデル**: エンドツーエンドでgitベースです。`suprnova = { git = "https://github.com/eas4ai/suprnova.git" }`。CLIは`cargo install --git`経由です。crates.ioには何も公開されていません。
