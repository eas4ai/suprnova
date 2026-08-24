# 更新日志

一份可读的、逐版本记录 Suprnova 变更内容的日志。每个版本小节都是该版本的发布记录。当一个版本的版本提交与匹配的 `v<version>` 标签被原子性地推送时，这个版本就算发布了。按最新到最旧排列。

## 未发布

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

### 安全

- **维护模式的绕过密钥现在以常数时间比较。** `MaintenanceMiddleware` 此前用普通的字符串比较来匹配这个密钥 URL，而普通比较会在第一个不同的字节处返回。由于这个密钥是一个随请求路径携带的 bearer 凭据，这个耗时差异会告诉攻击者，他们已经猜对了多长的前缀。这次比较现在会通过 `subtle::ConstantTimeEq` 跑完完整的字节长度，只在长度不匹配时短路 - 与它旁边那个绕过 cookie 的比较是同一个形状。

- **`rules::Url` 现在会拒绝脚本 URI。** 这条规则此前接受任何 `url::Url` 能解析的协议方案，`javascript:` 和 `vbscript:` 也在其中，所以一个通过了验证的 URL，被渲染进一个 `href` 之后仍然可能是一个脚本执行的落点。它现在采用 Laravel 的 `url` 规则形状（`Illuminate\Support\Str::isUrl` 的 `^(PROTOCOLS)://HOST` 模式）：协议方案必须在 Laravel 的允许列表上、必须后跟 `://`，**并且**后面必须跟一个非空的主机 - Laravel 的主机分组没有 `?`，所以即使协议方案在列表上，一个缺失或为空的主机也永远不会匹配。协议方案列表以及“`://` 加主机”这条要求都逐字取自 Laravel；主机本身由 `url` crate 解析，而不是由 Laravel 的正则解析，所以少数几个边界情况仍然不同 - 一个超出范围的端口在这里被拒绝，在那边则被接受，IDN 主机的归一化方式也不一样。新的 `Url::protocols(&[...])` 对应 Laravel 的 `url:http,https`；`HttpUrl` 现在就是它的字面语法糖，并保留自己的消息。**行为变更：**一个协议方案不在列表上、此前能通过验证的 URL 现在会失败 - 如果您本来就打算接受它，请用 `Url::protocols(&["myapp"])` 点名这个协议方案。另有两处行为变更：`mailto:`、`data:` 和 `tel:` 按名字在 Laravel 的允许列表上，但不携带 authority 组成部分，所以它们现在会失败；而 `file:///etc/passwd` 这类路径 - `scheme://` 后面最后两个斜杠之间什么都没有 - 现在同样会失败，因为空字符串也不是一个主机。两者都是从 Laravel 自己那条“`://` 加主机”的规则推出来的。

- **Inertia 响应现在处处都会声明 `Vary: X-Inertia`。** 这个响应头此前只设置在页面对象响应本身上。重定向、404、422 和静态响应都不带它，所以一个仅以 URL 为键的共享缓存，可能会把 JSON 页面对象提供给一次硬性的浏览器导航，或者把 HTML 外壳提供给一次 Inertia XHR。新的 `InertiaHeadersMiddleware` - 由 `Inertia::install` 注册为三者中最外层的那个 - 会在每一个响应上设置它，并且会把一次 Inertia 访问上的空 `200` 变成一个 `303` 回跳，而不是一个被客户端当作非 Inertia 而拒绝的响应。`InertiaVersionMiddleware` 现在会在它的 `409` 之前重新 flash 会话，所以一条被 flash 进去的错误消息，能挺过客户端随后那次整页 GET。

- **三处 Inertia 响应修复。** `InertiaResponse::location_for(&req, url)` 对一次 Inertia XHR 返回 `409` + `X-Inertia-Location`，对一次硬性导航则返回一个普通的 `302` + `Location`，所以一次在 SPA 之外发起的 OAuth 或 SSO 弹回，不再会死在一个没有响应体的 `409` 上。既有的 `location(url)` 保持它始终为 `409` 的形状。新的 `App::clear_history()` 会把清除历史记录的标志 flash 进会话，让它挺过登出重定向，落到那个真正会被渲染的页面上 - 而逐响应的 `.clear_history()` 只标记了那个被浏览器丢掉的重定向，于是上一个会话的加密历史记录仍然可以被解密。另外，一个 `once` prop 现在只在一次完整的 Inertia 访问上才会被跳过：一次显式的 `router.reload({ only: ['stats'] })` 会重新解析它，而不是什么都不返回。

- **SES 传输现在会发送自定义的消息头。** 在 `MAIL_DRIVER=ses` 之下，`Mail::to(..).header("List-Unsubscribe", ...)` 和 `Mailable::headers()` 此前会被静默丢弃：`Content.Simple` 请求体里没有 `Headers` 字段，而那个原始 MIME 构建器从来没有读过 `OutgoingMessage::headers`，尽管其他每一个传输都会转发它们。SES 的两条路径现在都会携带它们 - `Headers` 采用 SES v2 的 `{Name, Value}` 列表形式，原始 MIME 则写成真正的请求头行 - 所以退订链接、会话串联请求头和路由提示都能挺过一次驱动程序切换。请求头名字在两条路径上都会被提前校验 - CR、LF 和 NUL（注入用的那几个字节，Mailgun 传输早已拒绝它们），以及任何不是合法 RFC 5322 字段名的东西（空格、冒号、非 ASCII 字符） - 所以附上一个文件永远不会改变一封消息会不会被接受。

### 修复

- **PostgreSQL 软删除现在使用后端感知的占位符，生成的时间戳写入也会遵循声明的转换。** `delete()` 和 `restore()` 会呈现 PostgreSQL 序号占位符，而不是 MySQL 和 SQLite 的 `?` 占位符。生成的创建、更新、保存、touch 和软删除写入也会通过每个字段声明的 `Cast` 存储类型转换时间戳，因此原生 `TIMESTAMPTZ` 列不再接收文本值。感谢 [@i-am-v-alexander-v](https://github.com/i-am-v-alexander-v) 报告这两个缺陷，并在 [PR #3](https://github.com/eas4ai/suprnova/pull/3) 中提交修复。
- **默认 workspace 和 Magnetar gate 运行不再需要实时 PostgreSQL 或 MySQL 服务。** 后端特定行为套件是显式且被忽略的资格测试；如果故意在没有其已配置数据库的情况下调用，这些测试仍会失败。仅测试可达性的测试和永久性 gate 环境要求已被移除，因此无关更改不必在每次验证运行时承担外部数据库设置成本。

- **嵌套的验证失败现在会到达 422 响应体。** 嵌套结构体上的、或者被验证的 `Vec<T>` 中某个元素上的 `#[validate(nested)]` 失败，此前会在验证器和响应之间丢失：请求确实被正确地以 422 拒绝了，但 `errors` 映射回来是空的，所以没有任何消息被渲染出来，客户端也没法分辨是哪个字段出了问题。嵌套的失败现在会和顶层的那些一起，被展平成 Laravel 的点分记法 - `address.street`、`items.1.name`、`order.items.2.sku`。

- **Inertia 页面对象的 `url` 现在保留查询字符串。** `page.url` 此前只有请求路径，所以对 `/users?page=2&sort=name` 的一次访问，客户端记录下来的是 `/users`。此后每一次前进/后退导航、每一次 `router.reload()`，都会在丢掉分页游标、排序和过滤条件的情况下重放这个页面。它现在是路径加查询 - 和 `InertiaVersionMiddleware` 早已用于 `X-Inertia-Location` 的推导方式相同，所以默认情况下两者逐字节一致。新的 `InertiaConfig::url_resolver(...)` 可以覆盖*页面对象*怎样给这个页面命名（Laravel 的 `Inertia::resolveUrlUsing`）；版本弹回仍然点名那个到达的 URL，因为那才是浏览器必须去获取的 URL。

- **`Inertia::install` 现在会把它的配置应用到每一个响应上。** 交给 `Inertia::install` 的那份配置此前只被读了三个字段，然后就被丢弃了，所以每一个没有显式 `.with_config(...)` 构建出来的 `InertiaResponse`，渲染时用的都是 `InertiaConfig::default()`。一个用 `--frontend react` 脚手架出来的应用，除非环境里设置了 `SUPRNOVA_FRONTEND`，否则提供的是 Svelte 的入口点，而且没有 React 的 refresh 前导脚本；在这份配置上启用的 SSR 从来到不了任何响应；页面对象的资产版本，也来自一份与版本中间件的解析器不同的配置。这份被安装的配置现在会保留在容器的 Inertia 注册表里，并且正是 `InertiaResponse::new` 的起点。逐响应的 `.with_config(...)` 仍然会覆盖它，从不调用 `Inertia::install` 的应用不受影响，而一次失败（失败即关闭）的安装什么都不会保留。附带的一个效果是，生产环境的 Vite 清单现在每个进程解析一次，而不是每个响应解析一次。

- **脚手架出来的应用现在会安装 Inertia 的协议中间件。** `suprnova new` 写出来的 `bootstrap.rs` 注册了会话、语言区域、CSRF 和 include 这几个中间件，却从来没有调用 `Inertia::install`，所以一个生成出来的应用既没有 `InertiaVersionMiddleware` 也没有 `Inertia303Middleware`：一个仍然跑着上一份 bundle 的浏览器，在部署之后从来不会被告知去重新加载；而一个做了重定向的 `PUT`/`PATCH`/`DELETE` 会停在一个 `302` 上，客户端可能带着原来的动词去追随它。这次调用现在落在 `SessionMiddleware` 之后 - 版本中间件的会话重新 flash 正是在那里才起作用 - 并带着一个具名的 `INERTIA_VERSION` 常量，供资产变化时递增；它还会钉住这个项目生成时所用的前端（`--frontend react` 对应 `.frontend(Frontend::React)`），这样 HTML 外壳加载的就是那个框架的 Vite 入口点，而不是回退到 Svelte 的那个。生成出来的 `.env` 现在也会相应地设置 `SUPRNOVA_FRONTEND`。`--api` 起始套件不受影响；它没有前端。

- **`Queue::push_unique` 不再把一个已入队的作业报告为被跳过。** 它的返回值此前是用 `matches!(outcome, Idempotent::Fresh(()))` 算出来的，这会把 `Idempotent::FreshUnfenced` 折叠成 `false` - 而那正是信封*确实*被推送了、但去重租约在推送途中丢失的那个结果。根据这个布尔值分支的调用方，会被告知一个即将运行的作业已经作为重复项被压制了。三个结果现在都会被穷尽匹配：租约丢失返回 `true`，并附带一条点名这个作业和它的唯一键的 `warn`，只有真正的重复项才返回 `false`。`push_unique_later` 和 `later_unique` 共用这条路径，也随之被修复。

### 变更

- **当前开发分支使用 SeaORM 2.0，并要求 Rust 1.94.0。** Suprnova 保留其 Eloquent、`#[model]`、迁移和数据库门面的源代码结构。直接调用 SeaORM 的应用程序必须导入 `ExprTrait` 以使用 SeaQuery 表达式方法，并对预构建的 `Statement` 值使用显式 `*_raw` 连接方法。SeaQuery 现为 1.0，直接 MariaDB 向量驱动程序使用 SQLx 0.9。现有数据库不需要迁移应用程序数据；全新的 PostgreSQL schema 保留基于 serial 的主键。

- **对等基线已挪到 Laravel 13.25.0。** 13.23.0、13.24.0 和 13.25.0 的发布说明被逐条追溯到了框架自己的接口上。每一件触及了 Suprnova 代码路径的事情，要么已经在这个版本里修复，要么在 [`manual/parity.md`](manual/parity.md) 里有一行标着 `not yet` 或 `by design no`。

### 升级

有两处变更，可以在您这边不改任何代码的情况下改变一个正在运行的应用。

- **您传给 `Inertia::install` 的那份配置上的设置，现在会生效了。** 它们此前只被读了三个字段，然后就被丢弃了。如果您的安装配置设置了 `.ssr(...)`，那么 SSR 现在是开着的：请在部署之前启动那个工作进程（`suprnova ssr:start`），或者去掉这次 `.ssr(...)` 调用。在那里设置的 `.entry_point`、`.assets_base_url`、`.default_title` 和 `.encrypt_history(...)` 现在也会到达页面。

- **`rules::Url` 拒绝得更多了。** 此前能通过、现在不再能通过的值有：任何在 Laravel 允许列表之外的协议方案，`javascript:` 和 `vbscript:` 都在其中；`mailto:`、`data:` 和 `tel:`，它们在允许列表上，但不携带 `://` 主机；以及主机为空的 `scheme://`，例如 `file:///path`。如果您本来就打算接受某个协议方案，请点名它：`Url::protocols(&["myapp"])`。

## 1.2.3 - 2026-08-16

### 修复

- **日期时间转换现在可以读取数据库原生的`CURRENT_TIMESTAMP`文本。** `AsDateTime`、`AsImmutableDateTime`和`AsOptionalDateTime`仍会写入规范的RFC-3339；读取时也接受带时区的PostgreSQL文本以及不带时区的SQLite/MySQL值。不带时区的值按UTC解释。

## 1.2.2 - 2026-08-14

### 修复

- **在 PostgreSQL 上，所有基于属性的写入现在都能正确处理可为空的非文本值。** 类型化的 `Builder::update_all` 和 `Builder::upsert`、无模型的 `DB::table().insert/update`，以及多对多中间表的额外属性，会将显式 JSON 空值作为 SQL `NULL` 发出，同时继续绑定每一个非空值。这样会保留目标列的类型，而不是发送被标为文本类型的空参数；PostgreSQL 会拒绝将这种参数用于 bigint、integer、boolean、timestamp 和其他非文本列。多行 upsert 现在也会拒绝缺少或多出的列，而不会悄悄把形状错误的行转换为空值。多对多中间表的自动时间戳会以类型化 UTC 日期时间而非文本的形式绑定。

### 安全

- **发布门现在会在整个 workspace 中区分休眠的 lockfile 元数据与已编译的依赖项。** Cargo 会在 `Cargo.lock` 中记录 rust_decimal 未使用的可选 rkyv 0.7 兼容依赖；该门现在会证明，从任何 workspace 成员、feature、target 或依赖边都无法到达 rkyv 及其 derive crate。对应的 RustSec 例外由项目负责，期限至 2026-11-14，并且必须在 rust_decimal 不再记录这个遗留可选依赖时移除。

## 1.2.1 - 2026-08-09

### 变更

- **Suprnova 已从 GitHub 的 `entrepeneur4lyf` 组织迁移到 `eas4ai`。** 软件包元数据、文档、依赖示例和 scaffold 模板中的仓库 URL 现在使用 `github.com/eas4ai`。新项目也使用受监控的作者邮箱 `shawn@eas4ai.com`。此版本没有改变任何运行时行为。

## 1.2.0 - 2026-08-05

### 新增

- **手册现以七种语言发布。** `manual/es/`、`manual/fr/`、`manual/de/`、
  `manual/pt-BR/`、`manual/ja/` 和 `manual/zh-Hans/` 各自收录了完整的
  104 章手册 - 每一章、目录以及这份更新日志 - 均译自英文源文本。英文仍然是规范版本: 章节结构、代码块、标识符、CLI 命令和环境变量与源文本保持逐字节一致，因此译文章节在框架行为的表述上永远不可能与英文相左 - 它只是用读者的语言来讲述。

  这些翻译是为 suprnova.app 制作并评审的，该站点将本手册渲染为其 `/docs`。每个小节在那里都有一份评审台账: 裁定针对英文与译文双方的内容哈希记录，一个小节要计为已通过，必须有两位独立评审者对完全相同的字节予以通过；各语言的术语表则固定了术语裁定（哪些术语保留英文、哪些采用本族语词，以及理由）。欢迎在任一仓库提交更正 - 在这里的修复会在下一次同步时到达站点。

## 1.1.0 - 2026-08-02

### 新增

- **逐语言区域的回退链。** `LocalizationConfig` 新增了 `parents` 字段（`APP_LOCALE_PARENTS`，逗号分隔的 `child=parent` 对，或者可链式调用的 `.parent(child, parent)` 构建器）：一个语言区域可以先继承一个已配置的同级语言区域，再进一步回退到全局的 `fallback_locale` - `pt-PT` 继承自 `pt-BR`，`en-AU` 继承自 `en-GB`，依此类推，且具有传递性。`Lang::get`/`try_get`/`get_with`/`try_get_with`/`has` 全都会沿着这条链走，当前语言区域优先，所以这对任何 `Translator` 驱动程序都有效，不只是内置的那个。一个格式错误的配对、一个无效的语言区域、一个被命名两次的子项，或者一个环（包括一个语言区域把自己列为自己的父级），都会在配置加载时明确地失败，而不是在请求时才劣化。

  已提供的语料表会提前按链展平：`FluentTranslator` 现在会把每个语言区域的 `/_suprnova/lang/<locale>.ftl` 语料表构建成一次折叠 - 底部是给 `en`/`en-*` 语言区域用的内嵌框架语料表，然后是该语言区域已配置的父级链，最后才是它自己的 `*.ftl` 文件 - 所以一个链式语言区域仍然是浏览器只需获取一次的单个自包含文件，不需要客户端感知这条链。展平只覆盖已配置的父级；末端的 `fallback_locale` 仍然只是 `Lang` 门面层面的回退，不会被烘焙进已提供的字节里。

  这让增量式的语料表变得可行：一个 `lang/pt-PT/` 目录可以只保存真正与 `lang/pt-BR/` 不同的那少数几个字符串，而不必是一份完整的重复语料表。让这一切成为可能的合并，是在 Fluent AST 层面进行的 - 子项的值会替换父项的值，属性按名字合并（一个没有提到某个属性的覆盖不会再丢失那个属性），选择表达式整体替换（CLDR 复数类别是与语言区域相关的，所以逐变体合并并不连贯），子项独有的条目则会追加进去。完整的契约参见 `manual/localization.md` 新增的“回退链”小节。

### 变更

- **`LocalizationConfig` 新增了 `parents` 字段。** `from_env()` 和这个构建器不受影响；手写的结构体字面量构造（测试里手动构建一个 `LocalizationConfig`）需要多写一个字段。
- **已提供的语料表文本现在对每个语言区域都做了序列化器归一化**，并且同一语言区域内的多文件合并（一个语言区域目录里有好几个 `.ftl` 文件）现在会走和父级链一样的 AST 层面合并，而不是简单的 bundle 覆盖。已解析出来的翻译结果保持不变，除了下面这两处严格意义上的改进；不管怎样，底层字节都会发生变化 - `ETag`/`?v=<hash>` 会在升级时轮换一次。这两处改进是：一个覆盖不再会静默丢弃它没有提到的那些属性，一个仅覆盖属性的条目不再会剥离消息本身的值（此前这要么是一个错误，要么会解析成一次回退；现在它会解析成更早那次覆盖的值）。

## 1.0.0 - 2026-08-02

### 新增

- **本地化。** `lang/<locale>/*.ftl` 里的消息语料表（[Fluent](https://projectfluent.org)）、一个带 `__!("key", name: value)` 宏的 `Lang` 门面、逐请求的语言区域检测（`LocaleMiddleware`：会话 → cookie → `Accept-Language` → `APP_LOCALE`），以及基于 ICU4X、能感知语言区域的数字、货币、日期、时间、列表和相对时间格式化。这一章是 `manual/localization.md`。

  内置的验证规则不再硬编码英文。每一条规则返回一个带键的消息（`validation-min` 加上它的参数和一个英文回退），只在序列化边界处翻译一次 - 所以一个西班牙语应用只需要放进 `lang/es/validation.ftl`，就能得到西班牙语的验证错误，不需要包装任何规则，也不需要为框架的消息 fork 一份副本。字段名通过一次 `field-<name>` 查找来人性化。`Rule::passes`（以及 `ContextualRule` / `AsyncRule`）现在返回 `Result<(), ValidationMessage>`；一个自定义规则里的 `Err("…".into())` 主体仍然能编译、仍然会原样渲染，但您 `impl` 里的签名需要改成这个新类型。

  浏览器拿到的，是和服务器解析出来的完全一样的字节：合并后的语料表以 `/_suprnova/lang/<locale>.ftl` 提供，带着一个 ETag 和一个不可变的 `?v=<hash>` 形式，三个起始套件都用 `@fluent/bundle` 解析它，`suprnova generate-types` 会产出一个 `MessageKey` 联合类型，这样重命名一条消息就会让 TypeScript 编译器指向每一个调用点。

  之所以用 Fluent 而不是 Laravel 风格的 PHP 数组，是因为同一种格式必须同时服务服务器和浏览器，也因为让俄语、波兰语和阿拉伯语正确的，正是 CLDR 复数类别 - `trans_choice` 的整数区间做不到这一点，这也是这里没有 `trans_choice` 的原因。位于一个默认开启的 `localization` feature 之后；`--no-default-features` 仍然能编译、仍然会做验证，使用内嵌的英文回退。

- **`Paginator` 的 `IntoInertiaScroll`。** 这个 trait 此前给 `LengthAwarePaginator` 和 `CursorPaginator` 都实现了，唯独没给简单分页器实现，所以 `simple_paginate` 的结果完全没法喂给 `Inertia::paginate` - 尽管 `simple.rs` 自己的模块文档还把它指为 URL 生成路径。这让偏移分页的 Inertia 集合只能在“每个请求一次 `COUNT(*)`”和“手写滚动元数据”之间二选一。`next_page` 来自 `LIMIT n+1` 的溢出探测，而不是一个算出来的末页，因为根本没有总数可供计算。

### 修复

- **`suprnova generate-types` 每次运行都会产出不同的文件。** 拓扑排序通过遍历一个 `HashMap` 来给它的工作队列播种，而 Rust 会按进程随机化哈希遍历顺序，所以连续几次运行会把同样的一批接口排出不同的顺序。这份输出是一个提交进版本库的产物，所以每次运行都会产生一个 diff - 而一个无缘无故就反复变动的生成文件，会让人们不再重新生成它，此后它就会悄悄地不再描述它自称描述的那份 Rust 代码。目录遍历现在也排序了，所以输出不再依赖文件系统顺序。同一份源码运行两次，现在会得到字节级相同的结果。

- **`topological_sort` 做的事和它的文档注释正好相反**，把依赖方排在了被依赖方前面。这是无害的 - 一个 TypeScript 接口可以引用同一文件里稍后才声明的另一个接口 - 所以被修正的是这条注释，而不是这个顺序，因为改动顺序只会打乱一个已被跟踪的文件，却没有带来任何好处。

## 0.9.1 - 2026-08-01

三个缺陷，全都是通过在一个容器化的测试装置下运行 dogfood 应用发现的，而不是靠读代码发现的。它们每一个，对于一个从不像生产环境那样真正停掉一个进程的测试套件来说都是不可见的。

它们会按一个特定的顺序复合发生：一次滚动部署 SIGKILL 掉一个正在处理作业的工作进程（第一个缺陷），而这个作业接下来会走上一条从未计入这次尝试的重新认领路径（第二个缺陷）。

### 修复

- **`schedule:work`、`queue:work` 和 `workflow:work` 都忽略了 SIGTERM。** 三者都只在 `tokio::signal::ctrl_c()` 上做 select，而这只会安装一个 SIGINT 处理程序 - 所以进程里的任何地方都没有 SIGTERM 的处理程序，而 SIGTERM 正是 `docker stop`、Coolify、systemd 和 Kubernetes 发送的信号。三者背后都已经在那个 `select!` 之后精心写好了一段有边界的排空逻辑；但在一个监督程序之下，它从未被执行过。修复前的实测：对一个 `queue:work` 容器执行 `docker stop`，会烧光它整整 40 秒的宽限窗口，然后带着被摧毁的飞行中作业以 137 退出。作为 PID 1 - 这正是一个容器里运行的东西 - 内核会直接丢弃一个未被处理的 SIGTERM，所以这个进程不是死得难看；它根本没有死，直到 SIGKILL 出现。`Server::run` 已经正确处理了这两个信号，它的监听器现在也被共享了，这同时也关上了调度器循环里一个漏掉信号的窗口。

- **一个杀死了自己工作进程的作业，永远没法被转入死信。** 一个*处理程序*失败的作业会被 nack，它的尝试次数会被计入，所以它会在 `max_tries` 之后转入死信。而一个*杀死自己工作进程*的作业 - OOM、abort、段错误，或者上面那个 SIGKILL - 什么都不会结算；它的预留只是单纯地失效，而过去每一个驱动程序都会把它字节级原样地重新投递。这样的作业是不死的：它杀死每一个认领它的工作进程，原封不动地回来，再杀死下一个，只要还有什么东西在不断重启工作进程，这个循环就不会停。三个驱动程序现在都会在得知一个工作进程死亡时就计入这次尝试，因为切换 `QUEUE_DRIVER` 不应该改变一个毒丸作业能不能被拦下来。`attempts` 现在的含义是“投递给一个工作进程的次数”，而不是“处理程序失败的次数” - 记录在 `manual/queues.md` 里，因为一个因不相关原因而丢失的工作进程，同样会烧掉一次尝试。

- **……而这个耗尽了尝试次数的作业，现在会在被派发之前就转入死信。** 只计入这次尝试是必要的，但还不够。此前每一个死信决策都活在工作进程的结算路径里，而那条路径假定处理程序会返回 - 所以它恰恰对那些没法返回的作业从未运行过。只做驱动程序的修复时，计数器确实会往上爬（实测：三个被杀死的工作进程分别让它经历了 0 → 1 → 2），但没有任何东西会据此采取行动。现在这个预算会在处理程序运行之前就被花掉。这一点，只有在第一个修复看起来已经正确之后，重新跑一遍这个容器实验，才捕捉到。

- **守护进程完全没有 tracing 订阅者。** `serve` 会从 `init_telemetry` 拿到一个；而 `queue:work`、`schedule:work`、`schedule:run` 和 `workflow:work` 走的是另一条启动路径，什么都没拿到，所以它们发出的每一行 `tracing::` 都石沉大海，`LOG_LEVEL` 对它们来说形同虚设。而这恰恰是它们大部分要说的话 - 一个工作进程把某个作业转入死信、一个调度器跳过了一次它错过的 tick、一把它释放不掉的锁。在一个容器里，唯一可见的输出就是启动横幅，而这个进程看起来无所事事，实际上却在做这一切。这次发布里的两个缺陷，在这个问题被修好之前都是不可见的。

- **没有绑定失败作业存储时，一次死信就是一次静默删除。** 持久化这一步坐落在 `if let Some(store) = ..` 里面，所以在没有存储的情况下，这个分支根本不匹配，执行会直接落到 ack 上 - 这比它正上方的失败路径还要安静，因为那条路径至少还保留了预留。一个缺失的存储被当成了比一个坏掉的存储更成功。它现在会在 ERROR 级别记录整个信封，因为那正是 `queue:retry` 用来重新推送的东西：能靠人手恢复的工作，和已经不复存在的工作之间的差别。

- **`QUEUE_DRIVER=database` 现在会绑定一个失败作业存储。** `failed_jobs` 是这个驱动程序契约的一部分 - `queue:retry` 会读它，`Queue::retry_failed` 离了它没法工作 - 但 `bootstrap_from_env` 接上了驱动程序，却把存储留成了未设置，所以一个数据库支持的队列，除非应用手动绑定了一个，否则会把死信转进虚无。可以通过 `QUEUE_FAILED_DB_TABLE` 配置。只有这个驱动程序需要它：`memory` 天生就是短暂的，而 `redis` 根本没有表可写。

- **Redis 的重新认领延迟现在跟随 `--visibility-timeout`。** 这个标志设置的是 XAUTOCLAIM 的空闲阈值，但另有一个独立的时钟决定消费者多久看一次，而驱动程序把它留在了 sea-streamer 的 30 秒默认值上 - 所以 `--visibility-timeout 5` 实际的意思是“最多 35 秒”。这个间隔现在会跟踪已配置的超时，并被夹在 1 秒到 30 秒之间，这样一个很短的超时就没法变成一场 XAUTOCLAIM 风暴，而一个很长的超时也只会让重新认领比以前更快，不会更慢。

### 新增

- **`TaskBuilder::on_one_server()` / `on_one_server_for(ttl)`** - 让一个计划任务在多个副本之间、每个到期 tick 恰好只运行一次。没有它，就没有任何东西会为一个 tick 选出一个领导者：每个 `schedule:work` 进程都会独立地评估这份计划，实测三个副本会把每一个到期任务每分钟都跑三次，分毫不差。一个跑在三个副本上的夜间账单作业，会把每一位客户都扣三次款。

  `without_overlapping()` 覆盖不了这种情况，也没法覆盖：它的锁以任务为键，并在处理程序返回时释放，所以一个很快的任务会在第二个副本查看之前就把锁腾出来了。`on_one_server` 同时以任务*和这次 tick* 为键，并且会把锁一直持有到处理程序返回之后，让它靠 TTL 过期。这两者可以组合使用。

  这是可选启用的，与 Laravel 一致。但在失败关闭这一点上偏离了 Laravel：这次选举的共享程度，取决于它背后的缓存有多共享，所以在 `CACHE_DRIVER=memory` 且存在一个单服务器任务的情况下，一次生产环境启动会被拒绝，并点出违规的任务名字，除非设置了 `SCHEDULE_ALLOW_MEMORY_LOCK_IN_PRODUCTION=true` - 留给那些确实只跑一个调度器的部署。

### 变更

- `manual/deployment.md` 不再把“只运行恰好一个 `schedule:work` 进程”写成唯一的选项，并新增了一个**优雅停止**小节，讲述每个子系统各自的排空窗口、如何把一个平台的终止宽限期设置得高于这些窗口，以及为什么 PID 1 会让一个缺失的信号处理程序，比听起来还要糟糕。

## 0.9.0 - 2026-07-31

### 安全

- **认证签发此前只能按调用方节流，没法按收件人节流。** 一个以地址为键的限制，回答的是“某一个客户端是不是太吵”；它回答不了“某一个邮箱是不是正在被灌爆”这个问题。一个分散在一个僵尸网络上、或者单个 IPv6 `/64` 里的攻击者，可以待在每一个按 IP 的预算之下，同时用密码重置邮件把某一个受害者的收件箱灌满，而框架里没有任何东西能表达出本该拦住它的那个限制 - 一个键函数能读到路径、请求头和查询字符串，却读不到一个表单编码的请求体，所以恰恰在承载这个地址的那条路由上，这个地址是不可见的。

  `identity_key` 会以被操作的这个账号为键，给一个桶建键。它先读查询字符串，再读一个已缓冲的表单请求体，所以一个键函数就能覆盖这两种形状；这个值会被裁剪空白并转成小写，因为 `Alice@Example.com` 和 `alice@example.com` 送达的是同一个邮箱，而一个靠按住 shift 键就能绕开的限制，算不上限制；它还会被哈希，因为一个限流后端往往是一个共享的 Redis，访问控制比主数据库要弱。

  两个新的中间件构建器为它提供支持。`key_reads_body(cap)` 会在建键之前缓冲请求体 - 这是可选启用的，因为缓冲是一件未认证的调用方能强迫您去做的工作，超过上限的请求体会被以 413 拒绝，而不是不建键就放行。`only_when(pred)` 会对那些它根本管不着的请求，整个跳过某个限流器，这正是防止一个叠加的按收件人预算，在那些根本没有指名收件人的路由上，悄悄变成生效限制的关键。

  dogfood 应用现在会在它的签发分组上把两者叠加起来：每个地址每 5 分钟 10 次，每个收件人每 15 分钟 3 次。

一次针对 Torii 的会话、密码、OAuth 和 passkey 路径的审查，发现了八个缺陷，全都已经在这个钉住版本的 fork（`suprnova-torii-rs` `968b0be`）里修复。

- **已过期的会话可以被刷新，重新活过来。** SeaORM 会话仓储的 `refresh` 没有过期谓词，会无条件地延长 `expires_at`，而 `OpaqueSessionProvider::refresh_session` 跳过了 `get_session` 会执行的那个 `is_expired()` 检查。一个持有到过期之后的令牌，可以被无限期地续期。已在两层都修复。无法通过 Suprnova 自己的接口触达 - `Torii` 和框架都没有暴露会话刷新 - 但它是这两个 crate 的公开 API。
- **登录表单会通过计时泄露哪些账号存在。** 只要邮箱匹配不上，认证就会立刻返回，完全跳过 Argon2：实测一个未知地址是 54 微秒，而一个错误密码是 719 毫秒，差出约 13000 倍，这在网络上是可读出来的。两条失败路径现在都会对着一个哑哈希做校验，所以耗时一样。这一个*确实*能通过 Suprnova 的密码登录触达。
- **JWT 的 `iss` 声明会被写入，但从未被校验过。** 算法钉定此前就已经是正确的 - `alg: none` 和 HS/RS 混淆从来都不可能发生 - 但签发者一直只是装饰，所以两个共享同一个签名密钥的服务，会互相接受对方的会话。现在在配置了一个签发者时会强制校验。
- **一个一次性的 PKCE 校验值可以被认领两次。** 消费它的方式此前是先读后删，所以对同一个 `csrf_state` 的两次 OAuth 回调可以都先读到它，然后才有任意一个删除真正落地。现在会在一次操作里完成认领 - 在 Postgres 上是 `DELETE ... RETURNING`，在 SeaORM 上则是一次主键删除，靠受影响的行数来挑出胜者。
- **已过期的会话被列成了活跃状态。** `find_by_user_id` 没有过期过滤条件，而过期的行会一直存活到清理任务运行为止，所以一个“您已登录的设备”界面，会把已经失效的会话提供给用户去撤销，却对那个真正存活的会话只字不提。
- **一个 passkey 查找被命名成了 `authenticate`。** Torii 的 `PasskeyService::authenticate_credential` 接受一个凭据 ID，返回拥有它的用户，而 `PasskeyAuth::authenticate` 会据此铸造一个会话。Torii 存的是 passkey - 它不带任何 WebAuthn 依赖，也没法校验一个断言，所以这些调用能证明的唯一一件事，就是调用方知道一个凭据 ID：这是一个浏览器会明文发送、`allowCredentials` 会交给任何能发起一次握手的人的值。已重命名为 `find_user_by_credential` 和 `create_session_for_verified_credential`，两个名字都点明了校验是调用方的职责。无法通过 Suprnova 触达，因为 Suprnova 自己驱动 `webauthn-rs`（参见 `torii_integration::passkey`），只在凭据存储这一件事上才会用到 Torii。
- **一个 WebAuthn 质询在它整个 TTL 期间都可以被重放。** 两个后端都不会在读取时消费掉一个质询，SeaORM 的 `get_challenge` 还完全忽略了 `expires_at`，把已过期的质询当作存活的返回。现在两个后端的读取都会排除已过期的行，一个新的 `take_challenge` 会让一个质询恰好只被认领一次 - 和 PKCE 修复同样的“删除决定胜者”形状。

### 破坏性变更

- **Azure Blob Storage 和 Google Cloud Storage 被挪到了新的 `filesystem-azure` 和 `filesystem-gcs` feature 后面。** 除非您启用了对应的 feature，否则 `Storage::register_azblob`、`register_azblob_with`、`register_gcs`、`register_gcs_with`、`AzBlobConfig` 和 `GcsConfig` 都不再存在。如果您用到了这两个后端中的任何一个，请把它加进您的依赖：

  ```toml
  suprnova = { git = "…", tag = "v…", features = ["filesystem-gcs"] }
  ```

  您得到的是一个点名缺失项的编译错误，而不是一次运行时失败。

  这两个 opendal 服务 crate 都会拉入 `rsa`，它携带着 RUSTSEC-2023-0071（Marvin 计时攻击），上游还没有修复版本。它们是仅有的两个开启了 `reqsign-core/jwt` 的 crate，而 `reqsign-core` 那个可选的 `rsa` 正是藏在这个 feature 后面，所以把它们挡在 feature 后面，就一次性切断了通向它的全部三条 opendal 路径。`rsa` 现在是*可以避开*的：`--no-default-features --features filesystem,database-postgres` 不依赖它就能解析成功，并且仍然拥有存储子系统。此前没有任何 feature 组合能在保留存储的同时甩掉它。

  一次开箱即用的默认构建仍然携带着 `rsa` - `database-mysql` 是一个默认 feature，`sqlx-mysql 0.8.6` 非可选地依赖它 - 所以这条审计例外仍然敞开着。S3 是刻意**没有**被挡在 feature 后面的：`reqsign-aws-v4` 拿到的是不带 `jwt` 的 `reqsign-core`，所以 S3 驱动程序从来没有贡献过这样一条路径，把它挡起来只会破坏用得最多的那个云后端，却什么都清除不了。

### 新增

- **`suprnova --version`**，同时支持 `-v` 以及 clap 默认的 `-V`。用其他每一个 CLI 都在用的那个标志去问一个 CLI 的版本，不应该打印出一条用法错误。

### 修复

- **两个 Redis 操作此前都没有上限。** 缓存的标签清空操作会用 `SMEMBERS` 读出一个标签的整个成员集合，再逐个键删除，所以一个成员很多的标签会拖住这个连接，一次并发写入还可能在读和删之间丢失；标签现在是基于世代的，会被原子性地清空，并用一个有边界的 `SSCAN` 来扫描。延迟队列的晋升流程，此前会用一次不设边界的 `ZRANGEBYSCORE` 搬动每一个到期作业，所以一批一起到期的积压作业，会产生一个单独的、庞大的脚本；它现在会分批晋升。
- **两处关闭时的排空操作此前会永远等下去。** `schedule:work` 在 Ctrl-C 时，以及工作流工作进程在取消之后，都会不设期限地等待每一个飞行中的任务，所以一个永远不返回的任务，会让进程一直开着，直到 `SIGKILL` 出现 - 运维人员看到的是一个“停不下来”的守护进程。两处现在都会等待一段有边界的宽限期，然后中止剩下的部分，并报告数量。
- **发布版本钉定的清扫此前只认识两种钉定写法里的一种**，所以每一个带着一行 `cargo install --tag vX.Y.Z`、却没有依赖片段的文件，从未被发现过。`suprnova-cli/README.md` 已经连续三个发布都在告诉读者去安装 v0.6.0；`manual/cli.md` 和 `manual/cli-new.md` 停在了 v0.7.2；`manual/installation.md` 两种写法都有，其中一种被提升了，另一种却冻结不动。发现和重写现在都从同一张模式表里读取，一个文件适用哪些规则，由它的内容本身决定。
- **任何带 `filesystem` 却不带 `testing` 的构建，`cargo doc` 都会失败** - 七个 `Storage::fake` 的文档内链接无法解析，而 `lib.rs` 禁止出现失效链接。`testing` 是一个默认 feature，所以此前从来没有任何关卡步骤构建过这种组合；`check-feature-matrix.sh` 现在会构建它。
- **Torii 自己的迁移，此前没法在它自己的架构之上被重放**，所以一个持有这份架构、却没有 `torii_migrations` 跟踪表的数据库 - 从一份跳过了它的转储恢复的，或者手动迁移过的 - 就没法被纳入管理。每一个 `Table::create()` 都带着 `.if_not_exists()`；19 个 `Index::create()` 调用里没有一个带，那条 `ADD COLUMN locked_at` 的 alter 也没带，所以重放会顺利地滑过那些表，然后死在第一个 `CREATE INDEX` 上。已经在这个钉住版本的 fork（`suprnova-torii-rs` `a0f956d`）里，通过 `has_index` / `has_column`，而不是 `IF NOT EXISTS`（sea-query 会在 MySQL 上静默丢弃它）来修复 - 单纯的语法修复本会让一个默认 feature 的构建仍然是坏的。
- **一次失败的 Torii 迁移，此前会中止整个进程，而不是返回一个错误。** `SeaORMStorage::migrate` 对这个迁移器做了 unwrap，并无条件地返回 `Ok(())`，所以 `init_torii` 把这个失败映射成 `FrameworkError` 的那段代码，根本是死代码，永远走不到。
- **一个应用自己的 `users` 表，此前会静默地压制 Torii 的那张表**，因为 `.if_not_exists()` 分不清“已经是我的了”和“已经是别人的了”。这次迁移报告成功，认证却在之后因为缺一列而失败 - 这正是 `--api` 起始套件把自己的表命名为 `app_users` 的原因。Torii 的迁移现在会在迁移时发出警告，如果一张既有的 `users` 表缺少它需要的列，就点出这些列和补救办法。它仍然只是一条警告，而不是一次硬失败，这样既有的部署才能继续启动。
- **Railway 和 DigitalOcean 的部署指南，此前把平台健康检查指向了一条可能探测 Postgres 的路径。** 这两个平台都会在那项检查失败时重启容器，所以照着这份建议做，会把一次数据库的短暂抖动，变成每一个副本上的一场重启循环。两份指南现在都改用 `/_suprnova/health/live`，数据库改由控制台手动探测。旧路径仍然可以解析；任何已经部署好的东西都不需要改动。

## 0.8.0 - 2026-07-30

对一次外部红队审计的补救。这次审计给出了 19 个 P1 级发现，以及对 1.0 的一个 NO-GO 裁定；这次发布关掉了**全部十九个**，外加若干在修复它们的过程中发现、审计本身没有点名的缺陷。

有几处修复，是刻意把一种静默的错误配置，变成了一次被拒绝的启动。部署之前请先读**升级**这一节 - 一个此前运行得好好的生产应用，可能会启动不起来。

### 升级

三种此前会带着一条警告（或者干脆悄无声息）启动的配置，现在在生产环境里会失败关闭。每一条错误都会点出解除它所需要的那个变量，每一种情况也都有一个显式的开关，留给那些真正不存在这个风险的部署。

- **一个不投递的邮件驱动程序。** `MAIL_DRIVER` 未设置、`log`、`memory`，或者一个无法识别的值，都会解析成一种渲染邮件、然后直接丢弃它的传输方式 - 所以密码重置会报告成功，实际上什么都没发出去。开关：`MAIL_ALLOW_NON_DELIVERING_IN_PRODUCTION=true`。
- **明文 SMTP。** 四种凭据组合里有三种会落在一个未加密的传输上，而两者都未设置的那种情况，此前只会记一条警告，照样发送。开关：`MAIL_ALLOW_INSECURE_SMTP_IN_PRODUCTION=true`。
- **内存限流器。** 它的桶活在单个进程的堆上，所以在 N 个副本背后，每一份配额实际上都是 N 倍，而且每次部署都会把它们重置。请把 `RATE_LIMIT_DRIVER` 指向 `redis`，或者，如果您确实只跑一个进程，就设置 `RATE_LIMIT_ALLOW_MEMORY_IN_PRODUCTION=true`。一个*无法识别*的驱动程序值，会因为同样的原因失败，因为它会回退到内存 - 大写的 `RATE_LIMIT_DRIVER=Redis` 是最有可能触达生产环境的情形，因为它看起来像是配置过的。

这三种情况在开发、测试和预发布环境里都不受影响。预发布环境是刻意没有被把关的：在那里硬性失败，只会逼着团队把这个开关全局打开，反而在真正要紧的地方解除了这项检查。

两处不属于启动失败的行为变化：

- **`fill` 和 `first_or_new` 会拒绝格式错误的值。** 一个没法解码成其字段类型的值，此前会变成那个字段的 `Default`，并返回 `Ok` - `fill(attrs!{ age: "abc" })` 会把 `age` 设成 `0`，并报告成功。它现在会返回一个点名该字段的 `ValidationError`，并让模型保持不变。未知的列仍然会被静默跳过（与 Laravel 保持一致），数值类型的放宽转换仍然照常工作。
- **`/_suprnova/health?db=true` 不再返回驱动程序错误。** 细节挪到了日志里；响应体仍然保留 `"database": "error"`。调试构建仍然会包含它。解析 `status` / `database` 的仪表盘不受影响。
- **`url::signature_has_not_expired` 现在要求一个有效的签名**，并且已被弃用。它此前会对一个伪造的 URL 回答 `true` - 一个坏签名并不是“已过期”，因为它从来就没有一个可以错过的过期时间 - 所以任何单靠它来把关的处理程序，都会接受伪造的链接。它现在和 `has_valid_signature` 完全等价。如果您此前是用它来区分*已过期*和*无效*（好去渲染“请重新申请一个链接”，而不是一个 403），请改用会返回全部三种状态的 `url::signature_verdict`。这是刻意偏离 Laravel 的 `URL::signatureHasNotExpired` 的地方。

两处新增功能，只有在您选择启用时才需要您做点什么：

- **`QueueDriver` 新增了 `settle` 和 `release`**，两者都带有默认实现，所以既有的驱动程序实现无需改动就能继续编译。如果您的后端能在一个事务里同时提交一次后续写入和一次确认，就实现 `settle`；如果它能原地把一条已预留的消息重新入队，就实现 `release`。
- **批次记账现在可以是持久化的了。** `DatabaseBatchRepository` 需要两张新表，`job_batches` 和 `job_batch_settlements` - 请把它们加进您的迁移，就像 `jobs` 和 `failed_jobs` 那样。架构在 `manual/queues.md` 里。如果您继续用 `MemoryBatchRepository`，什么都不会改变。

### 安全

- **Slowloris（SEC-07）。** hyper 的请求头读取超时，文档上写的是 30 秒，实际上却是不生效的 - 它只有在连接构建器上装了一个计时器时才会启动，而此前根本没有装。一个客户端可以无限期地持有一个连接、以及一个 `SERVER_MAX_CONNECTIONS` 名额。现在已经启动，并可以通过 `SERVER_HEADER_READ_TIMEOUT` 配置。
- **Multipart 上传（SEC-05）。** 这个上限此前只作用于单个部分的载荷，而不作用于原始流，所以一个请求体在总量上可以超出这个限制。现在会在流这一层设上限。
- **带一个空密钥的 Webhook HMAC（SEC-08）。** 两个支付适配器此前都接受一个空白密钥，而一个空密钥能验证过任何东西。现在两者都会拒绝它。
- **Paddle 签名解析（P2-11）。** 一个长度为奇数、或者不是十六进制的 `paddle-signature`，此前会一路传到那个钉住版本的 SDK 里，并在其内部 panic。现在会先做校验：一个格式错误的签名会得到一个 401。
- **Passkey 绑定与重置令牌（SEC-01、SEC-02）。** 针对一个既有邮箱的匿名绑定、非本人绑定，以及没有最近重新认证的本人绑定，现在都会分别以不同的状态码被拒绝。一次密码登录现在会盖上重新认证窗口的时间戳。
- **`dev:tls`（SEC-10）。** 此前一个项目可以自行选择这个命令信任哪个 CA。
- **生成出来的 Docker Compose（P2-12）。** 此前会在所有网络接口上发布 Postgres 和 Redis，凭据还被提交进了这个版本库。现在绑定在回环地址上，密码逐次脚手架生成，`.env` 以 0600 权限写入，并且会拒绝符号链接目标。
- **健康检查端点（P2-01、CI-05）。** 它此前是用 `query.contains("db=true")` 来决定要不要查询数据库 - 一个子串测试，所以 `?nodb=true` 也会触发这次探测。现在会被正确地解析。这个 503 不再内嵌那个会点出主机、端口、架构和版本的驱动程序错误。
- **凭据签发节流（P2-02）。** 参考应用里的四条认证签发路由此前完全没有速率限制，而唯一有限制的那条路由，把它的桶建在了原始的 `x-forwarded-for` 请求头上 - 而任何客户端都可以逐请求地改变它，来换来一个全新的桶。两者都已修复；签发预算现在由这四条路由共享，所以在它们之间轮换并不会让预算翻倍。
- **一个被重新投递的链上步骤，此前会用一个新 id 重新推送它的后继者（DATA-02b，部分修复）。** 结算会*在* ack 之前推送下一个链环节，这是刻意的：先 ack 意味着这段窗口内的一次崩溃，会永久性地丢失这条链剩下的部分，而一个重复是可以恢复的，静默丢失却不行。但这个后继者的信封，此前每次推送都会拿到一个全新的 `Uuid::new_v4()`，所以这笔交易产生的重复，无论对驱动程序、对一个发件箱，还是对处理程序来说，都和一个合法的新步骤没法区分。

  最后这一点才是真正的代价。框架的投递契约是至少一次，它对重复的回答是“处理程序必须是幂等的” - 但一个以 `env.id`（它收到的唯一标识符）为键的处理程序，没法为一个链式作业满足这份契约，因为这个重复每次到来时都带着一个新 id。这份契约从结构上就是没法被满足的。

  后继者的 id，现在是从它前驱者的 id 派生出来的一个 UUIDv5，这个值在前驱者自身的多次重新投递之间是稳定的。一个被重新投递的步骤，会重新推送它之前推送过的那个 id。没有架构变更，没有新字段，没有新依赖。

  这让这个重复变得**可检测**，而这正是 DATA-02b 剩下的部分所缺少的那个原语。它并没有让这次推送和这次 ack 变成原子的（那需要一个发件箱），也还没有任何东西会在进来的路上拒绝这个重复。这两点都还悬而未决。
- **签名 URL 校验的是一个 URL，执行的却是另一个（SEC-04）。** 这个规范形式此前会把查询参数对折叠进一个 map，所以一个重复的键只会保留它的**最后**一个值 - 而 `Request::query_param` 返回的却是**第一**个。因此，一个合法签名过的 `?user=victim`，可以在原始签名原封不动的情况下，被重放成 `?user=attacker&user=victim`：校验会针对 `victim` 做规范化并通过，而处理程序实际处理的却是 `attacker`。

  这个规范形式现在会携带每一个参数对，按 `(key, value)` 排序，所以签名覆盖的是参数的精确多重集合 - 增加、删除或替换任何一个值，都会破坏这个 HMAC。一个重复的 `signature` 或 `expires` 会被直接拒绝，因为两份中的任何一份，都没法给出一个不武断的答案，来说明该由哪一个说了算。

  `Request::query_param` 现在会把一个重复的键解析成它的最后一个值，和 `query_params` 以及 `Context::query_param` 保持一致；它此前是三者之中唯一意见不合的那一个，而这个分歧正是这个缺陷的另一半。**既有的签名链接仍然照常工作** - 在没有重复键的情况下，载荷字节保持不变，这一点由一个测试钉住，因为一次悄悄让每一个未过期的密码重置链接全部失效的规范形式变更，会比这个 bug 本身还要糟糕。

  六个回归测试，涵盖两种攻击顺序、一个必须仍然能签名并通过校验的合法重复键，以及这个重新排序的保证。*没有*改变的是：`signature_has_not_expired` 仍然会把一个伪造的签名报告成“未过期”。那是 Laravel 的行为，是被刻意作为一次文档修复而定下来的，并且有它自己的测试，钉住它不被一次好心的“纠正”改掉。
- **Postgres 之下的 RBAC。** 现在会针对一个真实的 Postgres 做校验，而不只是 SQLite。
- **四条 RustSec 公告被彻底消除，而不是续期。** Pinecone 驱动程序被针对 Pinecone 的 REST API 重写了，甩掉了 `pinecone-sdk 0.1.2` - 它最新的一次发布还停留在 2024-09-06 - 连带甩掉的还有 `tonic 0.11 → rustls 0.22 → rustls-webpki 0.102`，以及 RUSTSEC-2026-0049 / -0098 / -0099 / -0104。这四条此前都已经在 `rustls-webpki >= 0.103.13` 里于上游修复，这个工作空间的其他 TLS 使用者也早就解析到了这个版本；是一个被放弃维护的 crate，把这棵依赖树钉在了那条有漏洞的线上。`.cargo/audit.toml` 里的忽略项，从五条降到了一条。这对这个驱动程序的 API 意味着什么，参见**变更**。
- **审计例外现在会过期。** `.cargo/audit.toml` 里的每一条记录都带着一个 `OWNER` 和一个 `EXPIRES` 日期，`scripts/check-audit.sh` 会在所有者缺失、日期缺失或无法解析、又或者日期已经过了的情况下，让这个发布关卡失败。`cargo audit` 本身没有“会过期的忽略项”这个概念，所以一条“临时”加进去的忽略项，会一直留在那儿，直到有人重新读一遍这份文件。剩下的这一条（RUSTSEC-2023-0071，`rsa`，它根本没有修复版本）已经有了所有者和日期。
- **可达性主张是被检查的，而不是被断言的。** `scripts/check-feature-matrix.sh` 会解析真实的依赖树，并断言没有任何一种构建 - 包括 `cargo audit` 实际读取的那个 `--all-features` - 会包含 `pinecone-sdk`、`rustls-webpki 0.102.x` 或者 `tonic 0.11.x`。一个仅靠一条没有任何东西去验证的注释来证明合理性的例外，只要有人加一个依赖，就会立刻不再成立。

### 修复

- **数据库支持的队列上，每一次 release 此前都会静默地变成一次空操作。** `JobOutcome::Released` - 一把繁忙的 `WithoutOverlapping` 锁、一次限流器退避 - 此前的实现方式是“推送一份副本，然后 ack 原件”。信封 id 正是 `jobs` 表的主键，所以这份副本会和那一行仍然持有活跃预留的记录冲突，推送会以 `UNIQUE constraint failed: jobs.id` 失败。工作进程于是正确地拒绝了 ack，所以请求的延迟从未生效，`JobReleased` 事件也没有触发，这个作业就只是停在那儿，直到可见性超时才把它重新投递。现在，release 是原地完成的一次驱动程序调用。
- **一次部分成功的批次派发，会让它已经入队的那些作业变成孤儿（DATA-02）。** 当一次 `driver.push` 在循环中途失败时，`PendingBatch::dispatch` 会删掉这个批次行 - 但已经进了队列的那些信封，仍然盖着那个批次 id，所以它们每一个结算时面对的都是一个已经不存在的批次，每次投递都会返回 `Err(batch not found)`，永远如此。现在这个批次会被结算，而不是被删除：没能派发出去的作业会被记录为失败，这个批次会被取消，这样已经入队的那些作业能正常结算，终态回调也仍然会触发。
- **此前没有任何测试验证过 `url::has_valid_signature` 会拒绝一个伪造的 URL。** 是在校验 SEC-04 的修复时发现的：即使把这个主要的签名 URL 守卫改写成接受任何签名，整个框架测试套件依然能通过。
- **一个脚手架生成的应用，此前没法迁移它的数据库，也没法构建它的镜像（REL-01b）。** 两个脚手架都没有声明 `default-run`，所以全部九个会 shell 出去执行 `cargo run` 的 CLI 包装命令，在一个全新的项目上都会失败。生成出来的 Dockerfile 有五处相互独立的缺陷 - 缺一个锁文件的 COPY、不带锁的 `npm ci`、一个缓存阶段只 stub 了两个已声明二进制文件里的一个、前端构建从一个 vite 从不创建的路径复制，以及缺一份 `frontend/src/pages` 的复制，而 `inertia_response!` 恰恰会在编译期校验它。一个开箱即用的脚手架的镜像，此前根本构建不出来。
- **`docker:init` 此前给每一种项目类型都发出同一份 Dockerfile。** 在一个 `--api` 项目上，它的第一条指令 `COPY frontend/package.json` 就会直接失败。API 项目现在会拿到一份不带前端的 Dockerfile。
- **SQL 占位符（DATA-01）。** 现在会按后端各自渲染，而不是假定只有一种方言。
- **队列结算（DATA-02a、P2-06c）。** 后续写入现在会在预留被 ack 之前完成结算，一次释放锁时的错误，也不会再把一个已经成功的作业变成一次重试。
- **一个被取消的批次此前只会触发 `Catch`，从不触发 `Then`。**
- **`Builder::clone` 此前会静默丢弃预加载计划（P2-09a）。** `User::query().with("posts")` 无论在哪里被克隆 - 分页、`count()`，或者任何会克隆的作用域 - 都会返回不带任何关系、也不报错的行。
- **Presence 花名册此前会丢失成员（P2-08）。** 这份花名册此前会在订阅之前就被快照，所以任何在这段窗口内加入的人，会在两边都不出现，而且是永久性的。
- **Pinecone 此前会把每一次索引获取都串行化（P2-14）。** 这把写锁此前会横跨两次网络往返一直持有，而 `tokio` 这把公平的 `RwLock`，意味着一个冷索引会拖住每一个热索引。
- **类型监听器此前会丢弃突发的一批变更（P2-13）。** 前沿防抖此前会在一批变更里的第一个文件上就重新生成，然后丢弃剩下的，也没有一次收尾运行，所以最后一次保存永远不会生效。
- **`ssr:check` 此前可能会挂起，并且只会尝试一个地址（P2-13）。** DNS 完全跑在超时范围之外，而且只会尝试第一个解析出来的地址 - 所以一个带 AAAA 记录、却没有 IPv6 路由的主机，会在工作进程明明在监听 v4 的情况下，被报告为已下线。
- **`suprnova serve` 此前安装的 `cargo-watch` 没有钉定版本（P2-13）。** 现在会带着一个主版本号边界，以 `--locked` 方式安装。
- **发布版本提升脚本此前只改写五份 README，别的什么都不碰。** 四个手册章节和一条公开的文档注释里，钉着的标签从来没有被任何一次发布更新过 - 那条文档注释已经落后了两个发布版本。发现逻辑现在取代了那份手工维护的清单，冒烟测试也会独立地对已提升的目录树做 grep，而不是相信提升脚本自己那一步校验。
- **`db:sync` 此前把数据库架构当作可信输入来对待（CLI-01）。**
- **`migrate:fresh` 现在被挡在 `--force` 加一次类型化确认（CLI-02）的后面**，在应用二进制文件里和在 CLI 里都是如此。
- **`log` 邮件驱动程序现在会记录整条消息**，和 Laravel 一样，并且不再在生产环境里把持有者链接写进日志。

### 新增

- **原子性的终态结算（`QueueDriver::settle`，DATA-02）。** 链上的后继者和这次确认，现在会在 `DatabaseQueueDriver` 上一起提交，关上了那扇窗口 - 此前介于两者之间的一次崩溃，要么会永久丢失一条链剩下的部分，要么会把它的下一步跑两遍。这个以预留为键的删除同时还充当一道栅栏：一个可见性在运行途中过期的工作进程，什么都不会提交，只会报告 `Settled::Stale`，所以它没法为一条现在归另一个消费者所有的消息入队工作。没法做到这一点的驱动程序，会回答 `Settled::Unsupported`，并保持文档记载的“先推送再确认”顺序。
- **`DatabaseBatchRepository`（DATA-02）。** 批次记账现在扛得住一次重启，`pending_jobs`/`failed_jobs` 现在是从以 `(batch_id, job_id)` 为键的结算行派生出来的，而不是被存储起来再递减 - 所以一个被重新投递的作业，没法在它其他的作业还在运行时，就把一个批次推向“已完成”，这道防护跨越多个进程都成立，而不只是在单个进程内。
- **`/_suprnova/health/live` 和 `/_suprnova/health/ready`。** 存活探测什么都不碰；就绪探测则会探测依赖项。把一次数据库检查接进一个存活探测，会把一次数据库的短暂抖动，变成每一个副本的一场滚动重启，而此前那个单一的端点，恰恰会招来这种情况。`/_suprnova/health` 仍然完全按文档记载的方式工作。
- **`SERVER_HEALTH_READINESS_TOKEN`。** 就绪探测的一个可选共享密钥，以固定时间比较。没有它时，就绪探测会回答 404 - 和一条未路由的路径没法区分，因为它*本来就是*路由器自己的那个 404。默认未设置，这样既有的探测才能继续工作。
- **`MAIL_SMTP_ENCRYPTION`** - `starttls` | `tls` | `none`，`ssl` 和 `null` 作为与 Laravel 兼容的别名被接受。未设置时会从凭据推导，完全复现此前的行为。这同时也让端口 465 上的隐式 TLS 变得可达：这个传输此前就支持它，只是没有任何一种环境变量组合能选中它。
- **`SERVER_MAX_CONNECTIONS` 和 `SERVER_HEADER_READ_TIMEOUT`** 已经写进了 `manual/env-vars.md`，此前它们在那里完全是缺失的。

### 变更

这次审计自己的结论是，这个关卡在 470 秒内通过，却一个 19 个 P1 都没抓住。这次发布的大部分测试工作，瞄准的正是这一点。

- **Postgres 现在会在这个关卡里跑起来。** 分布在六个文件里的十二个测试此前从未执行过。其中两个，结果发现会把 `DROP TABLE` 对准默认情况下 `localhost:5432` 上碰巧存在的任何 Postgres，而且两者都从未初始化过 `Crypt`，所以它们第一次运行就都失败了。
- **脚手架断言现在读取的是一个用户实际收到的字节**，是替换之后的，而不是模板源码。这发现了一个 API 项目会带着一条把数据库字面地命名为 `{package_name}` 的文档注释一起发布，还有一份 `.env.example` 打广告似的列了五个框架从来不会读的邮件键。
- **队列故障注入。** ACK 丢失、重新投递、租约失效和部分派发，现在都由一个装饰器驱动，它会在指定的调用上让指定的操作失败，所以每一种情况都是确定性的，而不是一场靠 sleep 赌运气的竞态。
- **支付适配器现在有了反向测试。** Stripe 的 `verify()` 此前从未被一个*有效*签名实际演练过，所以每一条依赖“走到 HMAC 比较那一步”的拒绝路径，都是未经证明的。
- **Pinecone 驱动程序现在讲 REST 了。** *这是破坏性变更，藏在默认关闭的 `vector-pinecone` feature 后面。* 动机记在**安全**那一节；接口层面的变化是：
  - `client()` 没了 - 不再有 `PineconeClient` 这回事。取而代之的是 `control_plane_get`、`control_plane_post` 和 `data_plane_post`，它们能带着您自己的请求和响应类型，通过这个驱动程序已认证、已解析主机的传输，触达*任意* Pinecone 端点。这比旧的那个脱围机制能触达的范围严格地更大。
  - `json_to_metadata` → `metadata_from_json`，元数据现在是 `serde_json::Map`，而不是 `prost_types::Struct`。`decode_match_fields` → `decode_match`，接受一个 `PineconeMatch`。`namespace()` 返回 `&str`。
  - 新增：`with_control_plane`、`with_api_version`、`with_index_host`（钉定一个已知主机，跳过控制平面这一趟往返）、`index_host`，以及 `PineconeVector` / `PineconeMatch` 这两个线上传输类型。
  - `from_env` 仍然会读 `PINECONE_API_KEY` 和 `PINECONE_CONTROLLER_HOST`，现在还会读 `PINECONE_API_VERSION`。
  - 这个 REST API 版本是钉死的，不是浮动的 - `2025-04`，也就是这个驱动程序的请求和响应形状当初是照着哪个版本写的。
  - 不再有任何东西会被串行化了。旧的驱动程序此前会在一个 `tokio::Mutex` 背后为每个名字缓存一个 `Index`，因为 `pinecone-sdk` 只在 `&mut self` 背后暴露它；新的驱动程序缓存的是一个主机字符串，共享 `reqwest` 的连接池。
  - 从控制平面获知的一个主机，无论响应里携带的是什么协议，永远都会通过 `https` 联系。
  - `Debug` 是手写实现的，API 密钥会被掩去，所以一个持有这个驱动程序的结构体上的 `#[derive(Debug)]`，没法把它打印出来。
- **针对 Pinecone 的线上契约测试。** 那些实时集成测试需要一个 `PINECONE_API_KEY`，所以没法在这个关卡里运行 - 这让一次 REST 重写的字段名（`topK`、`includeMetadata`、`vectorCount`）此前没有任何东西撑腰。现在有十三个测试，会针对一个本地的 `wiremock` 伪造实现来驱动这个驱动程序，并断言它放上线路的确切方法、路径、请求头和 JSON 请求体，外加一个非 2xx 永远不会被解码成一个结果、一条错误消息永远不会携带 API 密钥。它们把这个驱动程序钉在 Pinecone *文档记载*的契约上；只有那些标了 `#[ignore]` 的测试，才能确认文档是不是真的和线上服务一致。

## 0.7.2 - 2026-07-28

### 修复

- **`generate-types` 现在能解析没有派生宏的嵌套 prop 结构体。** 0.7.1 的生成器此前会把任何类型没有派生 `InertiaProps`/`Data` 的 prop 字段降级成 `unknown` - 所以对一个带着已提交类型文件的项目重新运行这个生成器（或者 `suprnova serve` 的监听器），会把 `Array<AdminArticleRow>` 这样的真实接口替换成 `unknown`，并让整个应用的类型检查失灵。现在，`src/` 里任何地方定义的普通结构体，都会解析成它们真实的接口，从 prop 根节点开始传递地解析；`unknown`（带一条警告）现在只留给项目确实没有定义的那些类型 - 外部 crate 的类型、枚举、元组结构体。

### 变更

- **`routes.ts` 的生成现在是可选启用的。** `generate-types` 不再不由分说地把 `frontend/src/types/routes.ts` 塞进每一个项目；传入 `--routes` 来生成它。

- **前端起始套件的依赖已经刷新。** 从 `suprnova new` 生成的新脚手架，现在会钉定当前的版本：Vite ^8.1.5、Tailwind CSS ^4.3.3、Svelte ^5.56.8（vite-plugin-svelte ^7.2.0、svelte-check ^4.7.4）、React ^19.2.8（plugin-react ^6.0.4）、Vue ^3.5.40（plugin-vue ^6.0.8、vue-tsc ^3.3.8），以及 `@types/node` ^24（Node 24 LTS 的类型线）。TypeScript 刻意停留在 ^6.0.3：它是最新的 6.x，而 svelte-check 的对等依赖范围（`^5 || ^6`）还不接受 TypeScript 7。三个起始套件都针对刷新后的这套版本，做了端到端的校验（`npm install` 加 `npm run build`）。

## 0.7.1 - 2026-07-27

一次对 0.7.0 队列路由的缺陷修复，来自一次完整的发布后复查。

### 修复

- **链式作业不再会丢失它们已声明的队列。** `ChainLink` 此前会在建链时捕获一个作业的 `max_tries`、`timeout` 和 `backoff`，却唯独不捕获它的 `Job::queue()`，所以一个直接推送时会落在它已声明队列上的作业，在作为一条链的一部分被派发时，却会落在 `default` 上 - 路由 → 作业 → 默认这个解析顺序里，“作业”这一层，对链来说会悄无声息地消失。已声明的队列现在会被捕获在这个链环节上，解析方式和直接推送完全一样。在这次发布之前写下的链载荷，解码时不受影响（`serde(default)`），一个没有声明队列的链环节，序列化出来的字节和 0.7.0 写下的完全一致。
- **失败作业记录现在会携带这个作业死在哪个队列上。** 工作进程的死信路径此前会把 `queue = "default"` 硬编码进每一条 `FailedJob` 记录，所以一个已路由作业的失败，对一个按所属池筛选失败存储的运维人员来说是不可见的。这条记录现在会携带这个信封的队列（未路由作业则是 `default`）。
- **0.7.0 的升级说明，低估了 `jobs` 迁移的必要性。** 它此前写的是“未做过滤的工作进程不受影响，不需要迁移”，但 `DatabaseQueueDriver::push` 无论这个作业是否被路由，都会在它的 `INSERT` 里点名 `queue` 这一列 - 一个 0.7.0 的二进制文件对着一张没有迁移过的表，每一次推送都会失败，不管有没有过滤。下面的 0.7.0 小节和 `manual/queues.md` 已经更正：在数据库驱动程序上，这条 `ALTER TABLE` 对每一次部署都是必需的，而且必须在二进制文件滚动升级之前运行（更旧的二进制文件会显式列出自己的列，所以先迁移是安全的）。

- **README 不再宣传一个 `#[job]` 宏。** 根本不存在这样一个宏 - 作业实现的是 `Job` trait。队列那一行现在描述的是真实的接口，包括 0.7.0 的队列路由。

### 变更

- **发布流程现在会提升 README 里的版本引用。** `bump-workspace-version.py` 会和清单文件原子性地一起，改写 README 里钉定的安装标签、分发模型示例，以及 MSRV 那一行；一份被改写过、不再匹配某个模式的 README，会让发布明确地失败。README 此前从 v0.7.0 发布起就一直在宣传 v0.6.0，因为发布流程里没有任何东西碰过它。
- **连接路由的文档现在写明只是名字解析。** `Job::connection()` 以及 `Queue::route` 的连接字段，解析的是携带在 `JobQueueing` / `JobQueued` 生命周期事件上的连接*名字*；一个单一的、进程全局的驱动程序仍然会接收每一次推送，所以它们并不会选中一个不同的驱动程序。rustdoc 和 `manual/queues.md` 此前暗示了一种并不存在的驱动程序选择能力。队列这个维度不受影响 - 它是被端到端地遵守的。逐连接的驱动程序仍然是未来的工作。
- `ChainLink` 新增了一个公开的 `queue: Option<String>` 字段，这会破坏链环节的结构体字面量构造。通过 `ChainLink::from_job` 构建的链环节 - 这也是正常路径 - 不受影响。

### 升级

如果您在数据库队列驱动程序上，是从 ≤ 0.6.x 升级过来的，请在滚动二进制文件**之前**，先应用下面的 0.7.0 迁移；这对该驱动程序上的每一次部署都是必需的，不只是那些用了 `--queue` 的部署。0.7.1 本身不需要迁移。

## 0.7.0 - 2026-07-26

### 安全

- **把 `ammonia` 升级到了 4.1.4（RUSTSEC-2026-0213）。** 4.1.3 及之前的版本，允许通过 SVG 的 `animate` 和 `set` 动画标签发起 XSS。`ammonia` 是 Suprnova markdown 流水线末端的净化器（`comrak` → `syntect` → `ammonia`），所以任何通过 `content` 渲染用户提供的 Markdown 的应用都暴露在外。这条公告发布于 2026-07-21 - 在 v0.6.5 发布之后 - 所以**截至并包括 v0.6.5 的每一个发布都受影响**。升级框架就是这个修复；不需要任何应用层代码改动。

### 新增

- **队列路由。** 作业现在可以被派发到一个指定的队列和连接，工作进程也可以被专门指定给特定的队列 - 这是 Laravel 13 的 `Queue::route(...)` 接口，类型化之后的版本。一个作业用 `Job::queue()` / `Job::connection()` 声明自己的归属；一个运维人员可以在 `bootstrap::register()` 里用 `Queue::route::<SendInvoice>(Some("redis"), Some("billing"))` 集中覆盖它，而不需要编辑这个作业。解析顺序是路由、然后作业、然后全局默认，一个路由里的 `None` 字段是顺延，而不是清空。`queue:work --queue=billing,default` 只会排空这些队列。未路由的作业属于 `default`，所以它们永远不会被搁浅。链式作业按名字解析路由，因为一个链环节存储的是它被擦除类型之后的作业。
- **`QueueDriver::pop_from`。** 带过滤条件的 pop，它的默认实现会**拒绝**一个自己没法遵守的过滤条件，而不是静默地排空每一个队列 - 一个被告知去排空 `billing` 的工作进程，却悄悄排空了一切，这在错误的池子吃掉错误的作业之前，和一次正常工作的部署没法区分。内存和数据库驱动程序都原生支持过滤。自定义驱动程序仍然能编译，并继承这个明确报错的默认行为。
- **写下了 `jobs` 表的架构文档。** `manual/queues.md` 现在携带着 `DatabaseQueueDriver` 实际期望的那份 DDL，此前只能靠读驱动程序的 SQL 才能发现它。
- **写下了 Inertia 的 `serverHead` 选项的文档。** 服务器驱动的 `<head>` 元素（Inertia 3.5.0）不需要任何框架层面的支持：客户端会从一个普通的 prop 里读取它们，所以任何处理程序都已经可以提供它们了。参见 `manual/frontend-inertia-responses.md`。

### 变更

- `Envelope` 新增了一个 `queue: Option<String>` 字段。它是 `serde(default)`，缺失时会被跳过，所以一个未路由的信封，序列化出来的字节和更早版本写下的完全一致 - 那个冻结的线上格式测试原样通过，没有 `schema_version` 的提升，混合版本的集群在一次滚动升级期间也能互操作。
- `WorkerConfig` 新增了一个 `queues: Vec<String>` 字段（为空 = 排空一切，也就是此前的行为）。
- 移除了 `ROADMAP.md`。它的设计原则活在 `manual/introduction.md` 里，工作约定活在 `manual/contributions.md` 里，部署和横向扩展的材料活在 `manual/deployment.md` 里；那份已发布/计划中的清单已经过时了。`README.md` 里指向它、用来说明“与上游的关系”的那个指针，此前就已经是悬空的了 - 那份归属声明活在 `LICENSE` 里。
- 脚手架前端现在把 `@inertiajs/{svelte,react,vue3}` 钉在 `^3.6.1`（此前是 `^3.4.0`）。3.4.0 → 3.6.1 这个区间只涉及客户端 - 对照上游的更新日志，以及 `packages/core/src/types.ts` 里的 `Page` 契约审查过，3.6.1 客户端会发送的每一个 `X-Inertia-*` 请求头，都已经被处理了。
- `scripts/release.sh` 现在会自己发布 GitHub release，说明取自这个版本 `CHANGELOG.md` 里的那个小节。此前这是一个会被漏掉的手动“下一步”，这正是 v0.5.10 和 v0.6.1–v0.6.3 只有标签、Releases 页面停在一个过时版本上的原因。预检会在这个关卡之前运行，所以一个缺失的 `gh` 或者缺失的更新日志小节，会在几秒内就失败，而且除非 `origin` 是 GitHub，否则发布会被自动跳过。

### 升级

数据库队列驱动程序上既有的 `jobs` 表**必须**添加这一新列 - `push` 无论这个作业是否被路由，都会在它的 `INSERT` 里点名它，所以一张没有迁移过的表，每一次推送都会失败。请先迁移，再滚动二进制文件（更旧的二进制文件会显式列出自己的列，忽略这个新列，所以这个顺序是安全的）：

```sql
ALTER TABLE jobs ADD COLUMN queue TEXT NULL;
CREATE INDEX idx_jobs_queue ON jobs(queue);
```

*（已在 0.7.1 中更正 - 这条说明原本声称未做过滤的部署不需要迁移。）*

## 0.6.5 - 2026-07-21

### 新增

- **Stripe 适配器里托管的一次性 Checkout。** 带着 `SessionMode::OneOff` 和非空 `price_refs` 的 `Checkout::start_session`，现在会创建一个托管的 Checkout Session（`mode=payment`，每个价格引用一个行项目，`allow_promotion_codes=true`），并返回 `SessionPayload::StripeCheckoutRedirect`。仅用 `amount_hint` 的 Elements 路径不受影响；两种形状按请求各自选择。
- **Stripe Managed Payments（记录商户）支持。** `StripeProvider::with_managed_payments(true)` - 或者在 `from_env()` 里设置 `STRIPE_MANAGED_PAYMENTS=true` - 会在创建托管的一次性 session 时发送 `managed_payments[enabled]=true`。默认关闭；这个字段会被整个省略，所以未开通的账号不受影响。
- **`Checkout::session_status`。** 新的 trait 方法（默认：`PaymentError::NotSupported`），以新的中性类型 `CheckoutSessionState`（`Open` / `Complete { paid, payment_ref, amount_total }` / `Expired`）报告一个 session 在提供商那一侧的状态。Stripe 的实现映射的是 `GET /v1/checkout/sessions/{id}`；`payment_ref` 携带这个 session 的 PaymentIntent id，用于和镜像表关联。这是重定向返回页面和对账扫描所需要的服务器端校验原语。
- **`Promotions` 能力 trait。** `create_promotion_code` 会基于一张预先创建好的优惠券，铸造一个限定客户、可选带过期时间、有兑换次数上限的优惠码。通过新的 `PaymentProvider::as_promotions()`（默认 `None`）查询。Stripe（`POST /v1/promotion_codes`）和 mock 都已实现。
- **`MockPaymentProvider` 为上面这些功能做了升级。** 记录每一次 `start_session` 请求（`recorded_sessions()`），按 session id 编排 `session_status` 的脚本（`script_session_status()` - 没被编排脚本的已知 session 会报告 `Open`，未知 id 则是 `NotFound`），并带着已记录的请求实现了 `Promotions`（`recorded_promotion_requests()`）。

## 0.6.4 - 2026-07-17

### 修复

- **Eloquent 聚合在各个数据库后端上现在解码一致。** 生成出来的 `count`、`sum`、`avg`、`min` 和 `max` 表达式，现在使用同一个稳定的内部结果别名。PostgreSQL 不再返回虚假的零或者 `None`，因为它的驱动程序给聚合列打标签的方式和 SQLite 不一样，而列缺失或类型不兼容的错误现在会传播出来，而不是被静默地设成默认值。
- **批量删除没法使用调用方提供的表表达式。** 可执行的删除 SQL，永远从模型已校验的静态 `M::TABLE` 派生它的目标。这个遗留的公开渲染器参数在源码层面仍然兼容，但没法重定向或者注入删除目标。

## 0.6.3 - 2026-07-15

### 新增

- **类型化的原始读取，现在可以留在一个事务已钉定的连接上。** `Transaction::backend()` 会暴露当前活跃的后端，`Transaction::query_all(Statement)` 会在这个事务内执行类型化的聚合查询或自定义 SQL，同时保留 `QueryExecuted` 的插桩。当一个受锁作用域限定的决策依赖于计算出来的结果列时，应用不再需要一个池级别的查询，也不再需要访问私有的执行器。

## 0.6.2 - 2026-07-15

### 修复

- **带绑定参数的原始谓词现在与后端无关。** Eloquent 的 `filter_raw` 和 `where_raw`，现在在每一个数据库后端上都接受可移植的 `?` 绑定标记；PostgreSQL 渲染时，会把它们在此前的谓词、关系子查询、HAVING 子句和 UNION 分支之间，重新定位到单调递增的 `$N` 位置上。既有的、已编号的 PostgreSQL 片段，会按它们各自局部的标记顺序被归一化，而混用不同风格、或者绑定数量不匹配的情况，会在做任何 I/O 之前就校验失败。这个感知 SQL 的扫描器，会保留引号字符串、标识符、注释和美元符引用体内部的问号；`??` 会在一个带绑定的原始片段里，发出一个字面的问号运算符。

## 0.6.1 - 2026-07-15

### 新增

- **可观测的、受监督的会话清理。** `SessionMiddleware::install` 使用可配置的 `SESSION_GC_INTERVAL` 节奏（默认一小时），而 `session_gc_metrics()` 会为受保护的运维接口，暴露进程本地的运行、成功、失败、已删除行数，以及上一次结果的时间戳。
- **有边界的滑动会话触碰。** `SESSION_TOUCH_INTERVAL` 控制着最小的活动写入节奏（默认五分钟），并被夹在会话生命周期的一半以内，这样活跃的会话就没法在两次触碰之间过期。

### 修复

- **无状态请求不再创建持久化会话。** 没有携带有效会话 cookie 的请求，不会执行任何会话存储的读或写，除非处理过程真的创建了状态，否则也不会收到会话 cookie。既有的干净会话，会避免无条件的 upsert 和 cookie 变动，遗留的 cookie 会在它们下一次请求时迁移，而那些背后行已经过期的 cookie，会被清理掉，且不会重新创建空会话。

## 0.6.0 - 2026-07-10

### 新增

- **可选启用的框架子系统，带向后兼容的默认值。** 文件系统存储、SQLite/Postgres/MySQL 数据库驱动程序、MariaDB 向量驱动程序，以及 Web Push，现在都有了显式的 Cargo feature。既有的默认构建会保留全部这些能力，而 `default-features = false` 的使用者，可以选择零驱动程序，或者只选自己用到的存储/数据库/向量/推送接口。这份可执行的 feature 矩阵，会校验零驱动程序、单个驱动程序、Nation X 最小化、默认，以及全部 feature 这几种配置。
- **原始的 P-256 VAPID 私钥导入。** `VapidKey::from_bytes` 现在除了既有的 PKCS#8 PEM 导入/导出路径之外，还接受一个经校验的、32 字节大端序的 P-256 标量。

### 变更

- **VAPID JWT 现在直接用 P-256 签名。** Web Push 现在会序列化 RFC 8292 的 ES256 请求头/声明，并用 `p256` 给它们签名，移除了那个通用的 JWT 依赖，同时保留了已生成的密钥、PEM 往返、公钥编码，以及 24 小时的生命周期边界。
- **安全依赖刷新。** 更新了有漏洞的框架依赖，包括 bcrypt 和 ammonia，并在保留语法高亮的同时，收窄了 Comrak 启用的 feature。
- **Rust 1.91.1 是这次发布的 MSRV。** 每一个工作空间成员包都声明同一个 `rust-version`，生成出来的 Dockerfile 会钉定匹配的构建器镜像，完整的发布关卡会用精确的 Rust 1.91.1 工具链，编译受支持的文件系统配置。
- **OpenDAL 0.58 安全钉定。** 这个 filesystem feature 钉定了 `eas4ai/opendal` 的提交 `88717391eb72c9839d3f8e79fccad9f22fc3a1b4`，一个恰好基于官方 Apache OpenDAL 提交 `ae99a3b016e354a1b2bb2baf0c70f9f9e134970a` 的最小化 fork。这个 fork 只改动了 OpenDAL 核心加上 S3、GCS 和 Azure Blob 所使用的 Reqsign 声明，这样下游使用者才能解析到官方 Apache Reqsign 的提交 `b49cd2996b9d2d9944e84481f8835ff55b188b97` 和 `quick-xml` 0.41.0。需要一个 fork 的原因是，一个依赖仓库根目录的 Cargo patch 不会传播给使用者；不这样做，已发布的依赖图仍可能恢复出有漏洞的 `quick-xml` 0.38/0.40。

### 修复

- **原子性的发布版本元数据。** 这次版本提升，现在会在一次已校验的操作里，同时更新 `workspace.package.version` 和每一个带版本号的内部路径依赖，暂存每一份受影响的清单文件，并在发布之前，用 `cargo check --workspace` 证明一个临时的 `0.6.0` 工作空间是可行的。发布版本号会按严格的 SemVer 2.0 校验，包括数字预发布段不能有前导零这条规则。与版本无关的一次性裸远程冒烟测试，会同时从当前源码和一个已经是 `0.6.0` 的源码，派生出一个更晚的补丁发布，会在这个关卡之前拒绝有暂存/未暂存/未跟踪改动的发布目录树，会证明原子性的提交/标签发布，在一个标签被拒绝时会把两个引用都回滚，并且会证明正常的发布流程不会碰到真实的远程仓库。发布版本号必须按 SemVer 优先级递增，包括预发布阶段之间的过渡。冒烟测试构建产物永远留在它们自己的临时工作空间内，忽略调用方的任何 `CARGO_TARGET_DIR`。
- **Rustdoc 覆盖了每一个受支持的 feature 边界。** OAuth 模块链接到公开的 `OAuthAuth::complete`，这份可执行的矩阵，会在没有任何依赖的情况下，构建零驱动程序、默认，以及全部 feature 的 rustdoc。
- **文件系统流校验现在是会话作用域的。** 本地文件系统的写入器、列举器和复制器，现在会在第一次 I/O 之前解析并限定它们的路径一次，而不是每个分块/条目都做一次，与此同时，已激活的关闭/中止操作，永远会触达后端去做清理。既有的遍历和符号链接限制，对一个可信的文件系统仍然生效；先规范化再打开的检查，并不能消除针对一个正在并发修改这棵目录树的主体的竞态。

### 安全

- **发布关卡现在会失败关闭。** `release.sh` 会在改写清单文件、或者创建提交/标签之前，先委托给这个规范的完整关卡；这个关卡永远会运行 `cargo audit`，把一个缺失的 `cargo-audit` 二进制文件当作一个错误，并在任何审计失败时停下来。它还会构建并审计一个隔离出来的下游文件系统使用者，断言精确的 OpenDAL/Reqsign 源码版本，并且没有低于 0.41 的 `quick-xml`。没有新增任何公告忽略项。

## 0.5.10 - 2026-07-03

### 修复

- **`generate-types` 不再丢弃自引用结构体。** 一个带有引用自身类型的字段的结构体（一个带 `children: Vec<Self>` 的树节点，比如一个带层级的评论视图），会在类型依赖图里产生一条自环边，把它的入度钉在零以上，所以 Kahn 的拓扑排序永远不会把它输出出来 - 让每一个引用它的接口，都带着一个失效的类型名，导致 `svelte-check`/`tsc` 失败。自环边现在会在排序之前被剥离，任何困在一个引用环里（相互递归）的结构体，现在会以任意顺序被输出，而不是被丢弃，因为 TS 接口本来就可以不分声明顺序地互相引用。

## 0.5.9 - 2026-07-01

### 新增

- **`MAIL_FROM_NAME` - 认证流程邮件上的可选显示名。** 邮箱验证、密码重置和密码已修改这几个 mailable，现在会在设置了 `MAIL_FROM_NAME` 时，把它们的 `From` 请求头渲染成 `"Name <address>"`（在发送时读取，这样它才能撑过队列的 serde 往返）。`MAIL_FROM` 仍然只是一个裸地址；把 `MAIL_FROM_NAME` 留空或不设置，会保持此前那种裸地址的行为。没有任何调用点需要改动 - 这些 mailable 会自己读取这个环境变量。

## 0.5.8 - 2026-06-30

### 修复

- **`generate-types` 的路由辅助函数现在永远是合法的 TypeScript。** 当一个模块里的好几条路由共享同一个处理程序时（比如一个 `static_files::serve` 的白名单，映射着一大堆 favicon/资源 URL），第一条会保留处理程序的名字，其余的则会拿到一个从路由路径派生出来的键 - 但这个路径此前只被部分净化过（`/ { } -` → `_`），所以一个文件扩展名会把一个 `.` 泄漏进这个键：`favicon_16x16.png: (...) => ...`。这是成员访问，不是一个属性名，所以 `tsc`/`svelte-check` 会拒绝生成出来的 `routes.ts`。派生出来的键现在会被净化成合法的标识符 - 每一个非字母数字字符都变成 `_`，一个前导数字会被加上前缀 - 所以 `favicon-16x16.png` → `favicon_16x16_png`，`2fa.json` → `_2fa_json`。唯一的处理程序名不受影响。

## 0.5.7 - 2026-06-30

### 修复

- **`generate-types` 不再产出悬空的类型引用。** 一个类型是某个没有派生 `InertiaProps`/`Data` 的结构体（或者一个生成器看不到的外部类型）的 prop 字段，此前会被产出成一个裸标识符 - 比如 `user: UserInfo` - 产出一份因为那个接口从未被写出来而让 `tsc`/`svelte-check` 失败的 TypeScript。这样的引用，现在会降级成 `unknown`（`user: unknown`；`Vec<T>` → `Array<unknown>`；`Option<T>` → `unknown | null`），所以生成出来的输出永远能通过类型检查，`generate-types` 也会打印一条警告，点出那个没能解析的类型，以及引用它的那个字段，并给出修复办法（给它派生 `InertiaProps`/`Data`）。泛型参数和已解析的嵌套 InertiaProps/Data 类型不受影响。

## 0.5.6 - 2026-06-29

### 变更

- **用 Apple 登录：RS256 JWKS 校验。** 把 `suprnova-apple-rs` 提升到 v0.3.1 - Apple 的 ID 令牌现在会针对 Apple 已发布的 JWKS（RS256）来校验，而不是在结构上被直接信任。

## 0.5.5 - 2026-06-28

### 新增

- **`MagicLink` 令牌用途。** 认证流程的 `TokenPurpose` 枚举上新增了 `MagicLink` 这个变体，用于无密码的魔法链接登录令牌。

## 0.5.4 - 2026-06-28

### 变更

- **可组合的 OAuth 完成流程。** 把通用的 OAuth 完成流程拆分成 `verify_oauth_identity`（校验并解析身份）和一个薄薄的 `complete`，这样应用就可以在不触发完整会话完成副作用的情况下，校验一个 OAuth 身份。

## 0.5.3 - 2026-06-28

### 修复

- **更正工作空间版本元数据。** v0.5.2 在它的 `Cargo.toml` 版本提升被暂存之前，就已经被打了标签并推送，所以推送出去的 v0.5.2 标签，读到的仍然是 `version = "0.5.1"`。v0.5.3 用正确的工作空间版本重新切出这次发布 - 没有代码改动（v0.5.2 的 OAuth 拆分不受影响）。

## 0.5.2 - 2026-06-28

### 变更

- **可组合的 Apple 完成流程。** 把 Apple Sign-In 的完成流程拆分成 `verify_apple_identity` 加一个薄薄的 `complete_apple`，与通用的 OAuth 拆分保持一致。（说明：推送出去的 v0.5.2 标签携带着一个过时的 `0.5.1` 版本字段 - 已在 v0.5.3 中修复。）

## 0.5.1 - 2026-06-28

### 变更

- **重命名了 Apple crate。** 把 Apple 依赖重新指向改名后的 `suprnova-apple-rs` 仓库。

## 0.5.0 - 2026-06-28

### 新增

- **用 Apple 登录。** 针对 Apple 的 OAuth 令牌交换 + ID 令牌校验 + 用户 upsert；Apple 的知名端点和 `form_post` 响应模式；`OAuthProviderConfig` 上特定于 Apple 的字段；重新导出的 `AppleKeyPair`，让应用不需要一个直接的 `apple` 依赖就能配置 Apple Sign-In。

### 修复

- 从 Apple 的授权 URL 里省略 PKCE 参数（Apple 在它们存在时会拒绝这个请求）。

### 依赖

- 采纳了 `torii` 的魔法认证修复；新增 `apple-rs` v0.3.0。

## 0.4.1 - 2026-06-26

### 性能

- 预先给 `MiddlewareChain` 分配大小，消除每请求一次的 `Vec` 重新分配。

### 修复

- 让维护模式的停机文件路径，在并行测试运行下也不会冲突。

### 文档

- 对框架的文档示例做编译检查（`ignore` → `no_run`）；把分发说明和已打标签的 GitHub Releases 对齐；忽略整个 `docs/` 目录树。

## 0.4.0 - 2026-06-22

### 变更

- **分发现在是 git 跟踪的；您不需要钉在标签上。** 脚手架生成的应用依赖 `suprnova = { git = "…/suprnova.git" }`，并跟踪默认分支；用 `cargo update -p suprnova` 拉取更新。版本会作为已打标签的 GitHub Releases（`v0.4.0`……）发布，供更新日志使用，但 `Cargo.lock` 已经钉定了精确解析出来的那个提交 - 所以构建在不手动钉定一个 `tag` 或 `rev` 的情况下，也能保持可复现。安装文档不再把钉定提交呈现为更新路径。

## 0.3.0 - 2026-06-21

### 新增

- **面向 Eloquent 读取的查询插桩** - `Builder::get`、`Model::find`、`find_many` 和 `all` 现在都会发出 `QueryExecuted`，所以模型的 SELECT 和预加载查询，现在会和写入、原始查询一起，出现在 `DB::listen` 和内存查询日志里。新增了带插桩的 `ExecutorChoice::statement_all` 读取终端。
- **资源路由授权** - `ResourceRoutes::authorize_resource::<U, R>()` 会把这个约定俗成的能力检查，作为逐路由中间件，挂到每一条生成出来的资源路由上（与 Laravel 的 `authorizeResource` 保持一致）。动作到能力的映射是：`index`/`show` → `view`，`create`/`store` → `create`，`edit`/`update` → `update`，`destroy` → `delete`。一次调用就能给整个七个动作的接口加上门，而不需要依赖每一个控制器方法体自己记得写一个 `Gate::authorize`。
- **原子性的限流命中** - `RateLimiter::hit_and_check(key, max, decay)` 会在一次往返里，同时递增一个固定窗口并测试它，返回这个桶现在是否已经超出限制（`i64::MAX` 表示不限）。
- **固定时间比较辅助函数** - `constant_time_eq(a, b)`（由 subtle 支撑），用于 webhook 签名校验；`WebhookHandler::verify` 的文档现在强制要求固定时间的摘要比较。
- **Inertia 客户端提升到 3.4.0** - Svelte/React/Vue 脚手架现在会把 `@inertiajs/{svelte,react,vue3}` 钉在 `^3.4.0`（此前是 `3.1.1`），带来了 `router.poll` 模式、动态的 `usePoll`、`Inertia.once`、InfiniteScroll 的取消修复，以及可等待的 Form `onSuccess`。服务器端已经在发出完整的 3.4.0 页面对象和请求头接口（一次性 prop、前置/深合并这一族滚动选项、`matchPropsOn`、被救回的/共享的 prop），所以这只是一次客户端版本追平，没有协议变化。
- **可选的连接上限** - `SERVER_MAX_CONNECTIONS`（以及编程方式的 `Server::max_connections(n)`），会用接受循环上的一个信号量，限定并发活跃连接的数量，在 TCP 这一层施加背压。未设置 - 或者设成 `0` - 会让连接保持不设上限（默认行为，未改变）。这是一道配合反向代理和 `LimitNOFILE` 使用的后盾，不是上游速率限制的替代品。
- **可以选择退出重定向跟随** - `RequestBuilder::no_redirects()` 会让一个请求走一个不跟随重定向的 HTTP 客户端，这样一个 `3xx` 会被原样返回，而不是被追着走。当请求 URL 受不受信任的输入影响时使用它，用来关闭一个基于重定向的 SSRF 向量（一个恶意端点把请求重定向到一个内部或云元数据主机）。默认客户端仍然会跟随重定向，与通用客户端的惯例保持一致。

### 安全

- **资源路由** 现在会在授权注册表那次类型擦除的向下转型上失败关闭，而不是 panic，`authorize_resource` 的拒绝 / 未认证请求，都会在处理程序运行之前就被拒绝。
- **限流器** 通过原子性地递增并比较（`hit_and_check`），关闭了一个固定窗口的“先检查后命中”竞态。
- **队列的 `RateLimited` 中间件** 现在通过那个原子性的 `hit_and_check` 来放行作业，而不是用一对分开的 `too_many_attempts` + `hit`，所以并发的工作进程不会再全部先通过预算检查，再由其中某一个去递增，从而超出 `max_attempts` 放行。
- **上传校验器**（`mimetypes` / `mime`）现在会对上传的字节做内容嗅探，而不是信任客户端提供的 `Content-Type`。
- **文件系统路径守卫** 现在会对路径做规范化，以捕获超出存储根目录的符号链接遍历，超出了此前那种词法层面的 `../` / 绝对路径 / UNC 检查。
- **认证** 关闭了一个无密码登录的计时预言机 - 一个匹配到了、但没有设密码的账号，被给了一个密码时，现在无论是 Eloquent 还是数据库用户提供者，都会跑一次固定成本的校验 - 而 `dummy_verify` 会驱动已配置的哈希器，让不匹配用户的路径也是固定时间的。
- **Eloquent** 现在会在 `pluck` / `value` / `pluck_keyed` / `sole_value` 以及 `sum` / `avg` / `min` / `max` 这些投影路径上，校验列标识符。
- **支付** - 这个 mock 提供者的校验器，在开发环境之外会失败关闭，webhook 的来源 IP，现在通过 `TrustedProxiesConfig`（`req.ip()`）解析，而不是一个原始的 `X-Forwarded-For` 请求头。
- **文件系统路径守卫** 现在会在一个写入目标还不存在时，一路走到最近的一个*确实存在*的祖先目录，关闭了一个符号链接逃逸 - 此前一个种在半路、紧邻父目录缺失的符号链接，能溜过这道守卫。
- **`DB::init_with`** 现在会在连接之前校验环境（与 `DB::init` 保持一致），所以那个开发环境的 SQLite 回退，没法再通过这个入口在生产环境里静默启动了。
- **静态文件服务** 现在会拒绝点文件（`.env`、`.git/config`、`.htpasswd`，任何以 `.` 开头的路径段），不只是拒绝 `.`/`..` 遍历。
- **支付 webhook** 现在会用一把 `FOR UPDATE` 锁加一次重新检查，把对同一个未处理事件的并发重试串行化，并把镜像表的唯一性冲突当作良性的“已经应用过”来对待；`payments_subscription_items` 新增了一个 `UNIQUE(subscription_id, provider_item_id)`。
- **RBAC** 现在会把模型判别符默认成完全限定的类型名，所以两个共享同一个叶子名字的可认证类型，没法再继承对方的角色/权限了。
- **`invalidate_session()`** 现在会轮换会话 id（而不只是清空），关闭了一个会话固定漏洞；队列的 `WithoutOverlapping` 中间件，现在即使在这个作业 panic 时，也会释放它的缓存锁。
- **邮件提供者** 现在会给错误响应体的读取设上限（8 KiB），与 web push 客户端保持一致，这样一个恶意端点就没法拖垮发送方的内存。
- **Web push** 现在会在默认客户端上禁用 HTTP 重定向跟随，这样一个被攻击者操纵的推送端点，就没法再把一次通知 POST 用 `3xx` 重定向到一个内部或云元数据主机（SSRF）。一次重定向现在会表现为一次被拒绝的推送，而不是一次被静默跟随的请求。
- **Stripe 适配器** 的 `Debug` 现在会掩去 webhook 签名密钥，*并且*会为 `stripe::Client`（它在自己的认证请求头里携带着这个 API 密钥）打印一个占位符，所以无论上游客户端自己的 `Debug` 怎么实现，`StripeProvider` 的一次 `{:?}` 都没法把任何一个密钥泄漏进日志。
- **Stripe 适配器** 的 `from_env` 现在会拒绝存在但为空的凭据，失败关闭，而不是构造出一个带着空（因此可伪造）webhook HMAC 密钥的客户端。
- **OAuth 邮箱校验** 现在对无法识别的提供商会失败关闭：一个携带 `email`、却没有 `email_verified` 标志的 userinfo 载荷，不再被当作已校验。一个未知的提供商现在必须断言 `email_verified: true`，或者暴露一个已校验邮箱端点，这关闭了一个针对以邮箱为账号键的应用的账号关联/劫持向量。Google（只认显式的 `true`）和 GitHub（由 `/user` 契约本身校验）不受影响。

### 修复

- **嵌套预加载**（`with(["posts.comments"])`）现在的查询数量是常数级的 - 尾段会用一次跨越所有父级的批量 IN 查询来加载，而不是每个父级一次查询（N+1）。
- **`where_has`/`where_doesnt_have`** 现在会用目标表来限定闭包里的列，所以一个在中间表和目标表上都存在的列，在多对多关系上不会再产生一个歧义列错误。
- **软删除的 `delete`/`force_delete`/`touch` 以及工厂的 `persist`** 现在会遵守模型的 `#[model(connection = "…")]` 路由（与 `restore` 和其他写入路径保持一致），而不是回退到主连接池。
- **JSON:API 的 `Maybe::Missing`** 现在使用一个不会冲突的线上哨兵值，所以形如 `{"__missing__": true}` 的用户数据不会再被静默剥离。
- **已入队的通知** 现在会遵守 `should_send`（逐渠道否决）和 `after_sending`，并在工作进程上重新检查它们 - 此前只有同步路径会这样做。
- **被 release 的作业** 现在会在 ack 原件之前先推送这份重试副本，所以一次瞬时的驱动程序推送错误，不会再丢掉这个作业。
- **Paddle adjustment（退款）webhook** 现在会以被引用的交易 id 为键来更新镜像，并从 `data.totals` 读取金额，而不是在 adjustment id 下插入一行零金额的记录。
- **携带查询字符串的 SQLite URL**（`sqlite://db.sqlite?mode=rwc`）现在会构建出一个有效的单查询连接 URL，以及一个干净的磁盘文件名。
- **HTTP** 现在会把 `Accept` 的 `q` 值夹在 `[0,1]` 之间，并且即便请求体已经被预先缓冲过，也会强制执行一个 `FormRequest` 的 `max_body_bytes`；**WebSocket** 配置现在会拒绝 `max_missed_pings < 2`（此前设成 1 会在每个连接的第一次 ping 时就把它关掉）。
- **Cron** 的月中日和周中日，在两者都受限制时使用 OR 语义（与 Vixie/POSIX 保持一致）；Markdown 的 `plain_text`/摘要会保留刻意留白的空格标点；`CachedEvaluator` 会限定自己缓存的增长；`SupervisorRegistry::start_all` 第二次调用时不会再重复 spawn；测试容器现在能从一把已中毒的锁原地恢复。
- **监督程序重启退避** 现在会在一次运行保持存活至少 60 秒这个上限之后，重置回 100 毫秒这个下限，所以一个健康运行了很长一段时间才退出的守护进程，会立刻重启，而不是继承此前一次失败爆发期间攀升上去的退避时间。一个运行时长从未达到这个阈值的崩溃循环，仍然会爬升到这个上限，所以这次重置永远不会掩盖一个正在抽搐的监督程序。
- 更正了关于 `filter_op`（运算符是按允许列表校验的）、签名 URL（与 Laravel 默认的绝对签名不是字节兼容的）、`UniqueIdKind::is_valid`（一个调用方辅助函数，并没有自动接入 `find`），以及标识符长度上限（是 128，不是 64）这几处过时的文档。

### 文档说明

- 在路由和授权章节里，写下了资源路由授权（`authorize_resource`）的文档；在速率限制章节里，写下了这个原子性的 `hit_and_check` 计数器的文档。

## 0.2.0 - 2026-06-21

新增基于角色的访问控制、一条 Markdown 内容/文档渲染流水线，以及原生的静态文件服务。

### 新增

- **二级 RBAC** - `HasRoles` trait；带一张 `role_has_permissions` 连接表的角色 + 权限；`PermissionMiddleware` / `RoleMiddleware`（两者都失败关闭 / 默认拒绝）；`CreateRbacTables` 迁移；以及 `create_role` / `create_permission` / `give_permission_to_role` 这几个辅助函数。
- **内容渲染** - Markdown 渲染和一条文档构建流水线：`MarkdownRenderer`、`build_docs`、`DocsCatalog` / `DocsChapter`、标题提取，以及 `slugify_heading`。渲染出来的 HTML 会被净化（comrak + syntect + ammonia）。
- **原生静态文件服务** - `StaticFiles::public()` 这个后备处理程序，会在网站根路径上提供一个 `public/` 目录，取代了应用里手写的逐资源白名单控制器。

### 修复

- 新生成的应用会继承一个框架层面的 `time = 0.3.47` 兼容性钉定，避免新脚手架的依赖解析中，`time 0.3.48` 带来的 Rust 1.96 一致性冲突。

### 文档说明

- 在整本手册、README 和路线图里，写下了两个已发布起始套件的文档 - **Nebula**（Breeze 级别的认证）和 **Pulsar**（产品网站 + 社区） - 围绕已发布的这部分接口重构了路线图；并在文档全篇统一了版本引用。

## 0.1.0 - 2026-06-10

首次发布的 Suprnova。Suprnova 是一个受 Laravel 启发的 Rust web 框架，从 Kit fork 而来，走上了自己的方向。今天的对齐目标是 Laravel 13.x。

这次发布采用 git 分发模型：框架的使用者依赖 `suprnova = { git = "https://github.com/eas4ai/suprnova.git" }`，CLI 用 `cargo install --git` 安装。

### 新增

#### HTTP、路由与中间件

- 带路由分组、前缀、参数约束、命名路由的 `Router`
- 通过 `routes!` 宏做编译期校验的路由注册
- 资源路由（`Router::resource`），生成七条标准路由
- 签名 URL（`url::signed_route` / `url::temporary_signed_route` 自由函数，加上 `Redirect::signed_route` / `Redirect::temporary_signed_route`）
- 重定向辅助函数 - `Redirect::to`、`Redirect::back`、`Redirect::route`、`Redirect::with_input`、`Redirect::with_errors`、`with_flash`
- 带全局、分组和逐路由层级的 Middleware trait
- 内置中间件 - CORS、CSRF、会话、请求超时、请求 ID、节流 / 登录节流、签名 URL 校验、已认证、邮箱已验证、暴力破解
- Abort 辅助函数（`abort`、`abort_unless`、`abort_if`）
- `suprnova::handle_request(...)` - 用于针对一个路由器 + 中间件链，服务单个 hyper 请求的公开适配器

#### Inertia.js 前端桥接

- 带 TypeScript 类型产出的 `#[derive(InertiaProps)]`
- 带编译期组件校验的 `inertia_response!` 宏
- 三个一等公民起始前端 - **Svelte 5**（启用 runes）、**React 19**、**Vue 3.5** - 全都基于 Inertia 3.1.1 + Vite 8 + Tailwind v4
- 部分重新加载（`only` / `except`）、延迟 prop、持久布局、加密历史、滚动位置保留
- `Inertia::paginate(component, key, paginator)`，用于分页器 → Inertia prop 接线

#### Eloquent 风格 ORM（基于 SeaORM）

- `#[suprnova::model]` 属性宏，一次性产出一个 SeaORM 实体，以及面向用户的 Eloquent 结构体
- 完整的 `Model` trait - `create`、`find`、`find_or_fail`、`find_many`、`all`、`query`、`save`、`update`、`delete`、`force_delete`、`refresh`、`fresh`、`replicate`、`replicate_into`、`increment`/`decrement`、`destroy`、`is`/`is_not`、`to_array`/`to_json`
- 带 `Attrs` 信封的可填充 / 受保护批量赋值
- 22 种属性转换 - 布尔值、整数、浮点数、日期、枚举、已哈希、已加密、JSON、集合、金额、带时区的日期时间
- 通过 `#[suprnova::model]` 实现的访问器 / 修改器
- 自动时间戳（`created_at`、`updated_at`）
- 带 `force_delete`、`restore`、`trashed`、`only_trashed`、`with_trashed` 的软删除（`deleted_at`）
- 十一种关系类型 - `HasOne`、`HasMany`、`BelongsTo`、`BelongsToMany`、`HasOneThrough`、`HasManyThrough`、`MorphOne`、`MorphMany`、`MorphTo`、`MorphToMany`、`MorphedByMany`
- 逐家族的 morph 枚举 + 带 `APP_KEY_PREVIOUS` 轮换的 morph 注册表
- 通过 `.with(...)`、`.with_count(...)`、`.load_missing(...)` 实现的预加载
- 面向 `has` / `where_has` 的相关 EXISTS 引擎
- 十六个生命周期事件（retrieving、retrieved、creating、created、updating、updated、saving、saved、deleting、deleted、restoring、restored、force-deleting、force-deleted、replicating、trashed）
- 带按方法通过 inventory 自动注册的 `Observer<M>` trait
- 通过 `#[scopes(M)]` 实现的本地作用域，通过 `GlobalScope` 实现的全局作用域
- `Collection<M>` 的 Laravel 接口 - `pluck`、`key_by`、`group_by`、`where_in`、`first_where`、`contains_where`、`partition` 等等
- 三种分页器 - `paginate`（长度感知）、`simple_paginate`、`cursor_paginate` - 全都序列化成 Laravel 形状的 JSON
- 用于批量行迭代、且不会 OOM 的 `chunk` / `lazy` / `cursor`
- `lock_for_update` / `shared_lock` 行级锁
- 带 `DynamicRow`（用于临时查询）的 `DB::table(...)` 查询构造器
- 带保存点、死锁重试、多连接读写分离的 `DB::transaction(...)`
- `DB::listen(...)` 加 `QueryExecuted` / `TransactionBegan` / `TransactionCommitted` / `TransactionRolledBack` 事件
- `Prunable` trait 加 `model:prune` 控制台命令
- `dump` / `dd` 查询辅助方法
- 用于 UUID / ULID 主键的 `#[model(unique_id="...")]`

#### Auth

- `Authenticatable` trait 加 `EloquentUserProvider<M>`
- `Auth::attempt`、`Auth::login`、`Auth::user`、`Auth::user_or_fail`、`Auth::user_as<T>`、`Auth::logout`、`Auth::check`
- 多个具名守卫（web 会话、API 令牌）
- 邮箱验证流程 - `EmailVerification`、`EnsureEmailVerifiedMiddleware`、签名验证 URL、`EmailVerificationMail`
- 密码重置流程 - `PasswordReset`、有节流的令牌、`PasswordChangedMail`、`PasswordResetLinkSent` 事件
- 双因素 TOTP - 绑定、校验、恢复码、重放防护
- 暴力破解 / 登录节流 - 按 IP + 标识符建键，`LoginThrottleMiddleware`
- 带稳定不透明令牌的记住我 cookie
- 六个认证事件 - `LoginAttempted`、`LoggedIn`、`Authenticated`、`LoggedOut`、`PasswordResetLinkSent`、`EmailVerified`
- 由 `github.com/eas4ai/suprnova-torii-rs` 这个 Torii fork 支撑的浏览器会话

#### 授权

- `Gate` 门面 - `define`、`allows`、`denies`、`authorize`、`any`、`none`、`check`（同步 + 异步两种变体）
- 用于策略注册的 `#[policy(Model)]` 宏
- 资源路由自动授权

#### 支付

- 与提供商无关的五 trait 接口 - `Checkout`、`Payment`、`Subscription`、`CustomerStore`、`WebhookHandler`
- `PaymentProvider` 这个总括 trait，加上通过 `as_payment()` 实现的能力查询
- 数据库镜像 - `customers`、`subscriptions`、`subscription_items`、`payments`、`refunds`、`payment_webhook_events`（带 UNIQUE 以实现幂等性）
- 带流程标记的 `SessionPayload` 枚举（一次性 vs 订阅）
- 两个作为工作空间 crate 的参考适配器 - `suprnova-payments-stripe`（网关，完整的 `Payment` 实现），`suprnova-payments-paddle`（记录商户，没有 `Payment` 实现）
- 面向测试的 mock 提供者

#### 队列、作业、批次与链

- `Job` trait - `handle`、`max_tries`、`backoff`、`timeout`、`fail_on_timeout`
- `Queue::push`、`Queue::push_later`、`Queue::push_unique`、`Queue::push_unique_later`
- 驱动程序 - `sync`、`null`、`redis`、`database`
- `JobMiddleware` trait - 六个内置中间件
- 批次和链 - `Queue::batch(jobs).dispatch()`、fluent 链构建器、取消、进度跟踪
- 带重放的失败作业存储
- 带优雅停机、可配置并发度、通过 `catch_unwind` 实现 panic 恢复、结算指标的工作进程
- 十二个覆盖排队、处理、失败、release、工作进程生命周期的队列事件

#### 广播与 WebSocket

- `ws!()` 宏 + `Router::ws`，用于类型化的 WebSocket 端点
- `WsSocket` 的 Sink/Stream 拆分
- 通过 `Supervisor` trait 实现的自动重启监督程序
- 带 `Channel`、`Private`、`Presence` 频道的 `BroadcastHub`
- JSON 信封协议、presence 的 join/leave/here，带崩溃恢复的可配置 presence TTL
- 桥接到 `EventDispatcher` 的 `Broadcastable`
- 带可配置 WS_TASKS 排空的、无 pong 即关闭心跳
- 逐路由的 WebSocket 中间件
- 1 MiB / 64 KiB 更安全的默认值 + `WsConfig::generous()` 工厂
- 来源策略 + 违反协议时以 1011 关闭

#### 通知与邮件

- `Notification` trait + `Notify::send(recipient, notification).await`
- Mailable + Markdown 模板渲染
- 数据库 / 邮件 / 广播 / web push 渠道
- VAPID 签名 + RFC 8291 ECE 载荷加密（通过 `suprnova-web-push`）
- VAPID 主体校验、retry-after 解析、8 KiB 拒绝响应体上限
- 用于收件人类型化的 Notifiable trait

#### 事件

- 类型化的事件分发器 - `EventFacade::dispatch`、`EventFacade::listen<E, L>`、`EventFacade::forget`
- 可取消的 saving/updating 事件（返回 `EventResult::cancel`）
- 可入队的监听器

#### 文件系统

- 带多驱动程序支持的 `Storage::disk("name")` - 通过 OpenDAL 实现的本地、S3、Azure、GCS
- 移动、复制、是否存在、大小、mime、最后修改时间、前置/追加
- 流式上传和下载

#### 缓存

- `Cache::store("name")` + 驱动程序注册
- 驱动程序 - memory、redis（带有边界的连接超时）、database、file
- `remember`、`forever`、`tags`、原子递增/递减、锁

#### 向量数据库

- 带四种驱动程序的 `VectorDriver` trait - 内存、Qdrant（UUID-5 id 映射）、Pinecone（原生字符串 id）、MariaDB 原生 `VECTOR(N)` + HNSW 索引（11.7+）
- 余弦 / 点积 / 欧几里得距离

#### 控制台二进制文件与 CLI

- 逐项目的 `console` 二进制文件 - `php artisan` 的 Rust 对应物，通过 `#[suprnova::console::command]` 运行用户定义的命令
- 用于类型化参数的 `#[derive(Command)]`
- `suprnova` CLI - `new`、`serve`、`migrate`、`db:sync`、`generate-types`、`key:generate`、`make:{controller,middleware,action,error,inertia,migration,task,command}`、`db:seed`、`model:prune`
- `--version` 标志
- 面向三种前端的后端 + API 起始套件的脚手架模板

#### 功能标志

- 带快照加载的 `DatabaseEvaluator`
- 带 TTL 的 `CachedEvaluator`
- `FeatureMiddleware` 提取器
- 管理端 CRUD 接口
- 用于跨进程亚秒级传播的 `FeatureSync` trait

#### 调度

- Cron 表达式解析器
- 带可组合谓词的 `Schedule::task(...)`
- 单服务器锁、防重叠、派发跟踪
- `schedule:run` 控制台命令

#### 验证

- `validator` 0.20 集成
- `#[request]` + `#[derive(FormRequest)]` 宏
- 逐表单大小上限的 `#[form_request(max_body_bytes = N)]`
- 面向用户自写 `impl FormRequest` 的可选退出项 `#[form_request(custom_hooks)]`
- 生命周期钩子 - `authorize`、`after_validation`、`after_validation_async`

#### 数据库驱动程序

- 由 SeaORM 支撑的 SQLite、Postgres、MySQL、MariaDB 支持
- 基于 URL 的驱动程序检测
- 迁移系统 + `migrate`、`migrate:rollback`、`migrate:status`、`migrate:fresh`、`migrate:refresh`

#### HTTP 客户端

- `Http` 门面 - `get` / `post` / `put` / `patch` / `delete`，返回一个 `RequestBuilder`；`.send().await` 产出一个 `ClientResponse`
- rustls TLS、30 秒默认超时、`suprnova/<version>` user-agent
- `json` / `form` / `body` / `header` / `bearer_token` / `basic_auth` / `timeout` 这几个可链式调用的方法
- `RequestBuilder::retry(max_attempts, base_backoff)` - 面向瞬时失败和 5xx 的指数退避；遵守 `Retry-After`
- `Http::fake(|| async { ... }).await` 测试守卫，带 `fake_response(method, url_substring, status, body)` + `assert_sent` / `assert_not_sent`

#### 加密

- `Crypt` 静态门面 + `EncryptionKey`（`crypto::*`）；带 12 字节随机 nonce 的 AES-256-GCM
- `encrypt_string` / `decrypt_string` / `encrypt<T>` / `decrypt<T>`
- 防止跨协议重放的 `CryptPurpose` AAD 绑定
- `APP_KEY_PREVIOUS` 轮换
- 用于铸造新密钥的 `suprnova key:generate` CLI 命令

#### 测试

- `#[suprnova_test]` 异步测试宏
- 带并行安全实例的 `TestDatabase::fresh::<Migrator>()`
- 用于逐测试 mock 的 `TestContainer::bind`
- HTTP 测试辅助函数 - `Test::get`、`Test::post`、JSON / form / multipart
- Queue / Mail / Notification / Event 伪造实现
- `assert_emitted`、`assert_dispatched`、`assert_dispatched_times`

### 变更

- 认证校验和密码重置流程，现在通过已配置的用户提供者运行，而不是 Torii 内部机制。
- 生成出来的应用必须实现 `get_auth_password`；脚手架生成的示例现在会明确地失败，而不是让登录永远静默失败。
- 本地发布关卡现在接入了 `scripts/release.sh`，这个仓库也带上了一个强制执行的 pre-push 钩子，用于 fmt、clippy、测试、文档和 feature 构建。
- 脚手架生成的开发端口文档，改成了当前的后端/前端默认值（`8765` / `5765`），并写下了 `dev:tls` 和 `--with-portless` 的文档。
- `MAIL_FROM` 现在会在验证或重置令牌被签发之前先校验，避免在邮件配置无效时留下孤立的认证流程行。

### 修复

- React 脚手架模板与已发布起始套件之间的偏差。
- 根路由分组不再生成重复的 `//` 路径。
- 字面路径重定向现在会通过预期的路由路径派发。
- 广播扇出测试现在能处理 `track` / `untrack` 的结果。
- 邮件 log 驱动程序现在会发出渲染后的文本正文，所以验证和密码重置链接会出现在本地开发日志里。
- 密码重置的测试覆盖，钉住了会话和记住我的撤销行为。

### 说明

- **分发模型**：端到端基于 git。`suprnova = { git = "https://github.com/eas4ai/suprnova.git" }`；CLI 通过 `cargo install --git` 安装。没有任何东西发布到 crates.io。
