# Registro de mudanças

Um log legível, por versão, do que mudou no Suprnova. Cada seção de
versão é o registro de lançamento daquela versão. Uma versão é
lançada quando seu commit de versão e a tag `v<version>` correspondente
são enviados atomicamente. Mais recentes primeiro.

## Não lançado

## 1.3.1 - 2026-08-24

> The v1.3.1 release notes are intentionally kept in English to preserve the complete normative record.

### Corrigido

- **Provider-backed applications can reset verified users again.** When no Magnetar engine is installed, `PasswordReset` uses an explicitly reset-capable `UserProvider` and framework `auth_flow_tokens` for already verified accounts. `EloquentUserProvider<M>` opts in when `M` implements `MustVerifyEmail + CanResetPassword`; no `app_users` migration is required.
- **The published framework line now contains both post-release repair sets.** The translated 1.3.0 changelog layout and headings, CJK wrapping, localized anchors, glossary terms, and prose punctuation are reconciled instead of split across divergent local and remote branches.
- **Post-tag CLI and Magnetar hardening is included.** Development-process cleanup uses the completed process-group fallback, and the local qualification contracts cover the released refs and plugin-SDK SQLite lanes.

### Segurança

- **The provider fallback never treats password reset as first mailbox proof.** Unknown and unverified addresses receive the same no-mail response. Install Magnetar when an unverified account must prove mailbox ownership through reset so credential cleanup, auth-epoch advancement, and revocation remain atomic. Provider fallback completion reports framework session and remember revocation failures through `PasswordResetOutcome`.

### Atualizando

- **Move every `v1.3.0` Git dependency to `v1.3.1`.** Applications with their own `users` table keep their configured `UserProvider`; they do not initialize the default `app_users` engine merely to reset an already verified account. Applications that use Magnetar credentials or unverified-account first proof continue to initialize Magnetar.

## 1.3.0 - 2026-08-24

> The v1.3.0 release notes are intentionally kept in English to preserve the complete normative record.

### Segurança

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

### Adicionado

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

### Corrigido

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

### Alterado

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

### Obsoleto

- **`Cookie::read_encrypted` is now the v1-only legacy reader.** Code that mints with
  `Cookie::encrypted` and reads with `read_encrypted` fails at runtime on the first value written
  after this release; switch to `read_encrypted_for(name, wire)`. The un-contexted
  `CryptPurpose::Cookie` entry points are also superseded. Both removals are scheduled for 1.4.0.

### Atualizando
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

### Segurança

- **O segredo de bypass do modo de manutenção é comparado em tempo
  constante.** O `MaintenanceMiddleware` casava a URL do segredo com uma
  comparação de string simples, que retorna no primeiro byte diferente.
  Como o segredo é uma credencial bearer carregada no path da
  solicitação, essa diferença de tempo dizia a um atacante o tamanho do
  prefixo que ele tinha acertado. A comparação agora percorre o
  comprimento completo em bytes via `subtle::ConstantTimeEq`, fazendo
  curto-circuito somente numa diferença de comprimento - o mesmo formato
  da comparação de cookie de bypass ao lado dela.

- **`rules::Url` agora rejeita URIs de script.** A regra aceitava
  qualquer esquema que `url::Url` conseguisse interpretar, incluindo
  `javascript:` e `vbscript:`, então uma URL validada ainda podia ser um
  sink de execução de script ao ser renderizada em um `href`. Ela agora
  aplica o formato da regra `url` do Laravel (o padrão
  `^(PROTOCOLS)://HOST` de `Illuminate\Support\Str::isUrl`): o esquema
  precisa estar na allowlist do Laravel, ser seguido por `://`, **e** ser
  seguido por um host não vazio - o grupo de host do Laravel não tem `?`,
  então um host ausente ou vazio nunca casa, mesmo com um esquema
  listado. A lista de esquemas e a exigência de `://` mais host são as do
  Laravel, ao pé da letra; o host em si é interpretado pelo crate `url`
  em vez do regex do Laravel, então alguns casos de borda ainda diferem -
  uma porta fora do intervalo é rejeitada aqui e aceita lá, e hosts IDN
  normalizam de forma diferente. O novo `Url::protocols(&[...])` espelha
  o `url:http,https` do Laravel; `HttpUrl` agora é açúcar sintático
  literal para ele e mantém a própria mensagem. **Mudança de
  comportamento:** uma URL com um esquema não listado que antes validava
  agora falha - nomeie o esquema com `Url::protocols(&["myapp"])` se a
  intenção era aceitá-lo. Mais duas mudanças de comportamento: `mailto:`,
  `data:`, e `tel:` estão na allowlist do Laravel pelo nome, mas não
  carregam componente de autoridade, então agora falham; e paths no
  estilo `file:///etc/passwd` - `scheme://` sem nada entre as duas
  últimas barras - agora também falham, já que uma string vazia também
  não é um host. As duas decorrem da própria regra de `://` mais host do
  Laravel.

- **Respostas Inertia agora anunciam `Vary: X-Inertia` em todo lugar.** O
  header era definido apenas nas próprias respostas de objeto de página.
  Redirects, 404s, 422s, e respostas estáticas não carregavam nenhum,
  então um cache compartilhado chaveado só pela URL podia servir o objeto
  de página JSON para uma navegação dura do navegador, ou o shell HTML
  para um XHR do Inertia. O novo `InertiaHeadersMiddleware` - registrado
  por `Inertia::install` como o mais externo dos três - o define em toda
  resposta, e transforma um `200` vazio numa visita Inertia em um `303`
  de volta, em vez de uma resposta que o cliente rejeita como não
  Inertia. O `InertiaVersionMiddleware` agora refaz o flash da sessão
  antes do seu `409`, para que um erro flashado sobreviva ao GET de
  página inteira de acompanhamento do cliente.

- **Três correções de resposta Inertia.** `InertiaResponse::location_for(&req, url)`
  retorna `409` + `X-Inertia-Location` para um XHR do Inertia e um `302` + `Location` simples para uma navegação dura, então um bounce de OAuth
  ou SSO iniciado fora da SPA não termina mais num beco sem saída com um
  `409` sem corpo. O `location(url)` existente mantém seu formato
  sempre-`409`. O novo `App::clear_history()` faz flash da flag de
  limpeza de histórico na sessão, para que ela sobreviva ao redirect de
  logout e chegue à página que de fato renderiza - o `.clear_history()`
  por resposta marcava apenas o redirect que o navegador joga fora,
  deixando o histórico criptografado da sessão anterior
  descriptografável. E uma prop `once` agora é pulada somente numa visita
  Inertia completa: um `router.reload({ only: ['stats'] })` explícito a
  resolve de novo em vez de não retornar nada.

- **O transporte SES agora envia headers de mensagem customizados.** `Mail::to(..)
  .header("List-Unsubscribe", ...)` e `Mailable::headers()` eram
  descartados silenciosamente sob `MAIL_DRIVER=ses`: o corpo de
  solicitação `Content.Simple` não tinha campo `Headers` e o builder de
  MIME bruto nunca lia `OutgoingMessage::
  headers`, ainda que todo outro transporte os encaminhe. Os dois
  caminhos do SES agora os carregam - `Headers` como a lista
  `{Name, Value}` do SES v2, MIME bruto como linhas de header reais -
  para que links de descadastro, headers de threading e dicas de
  roteamento sobrevivam a uma troca de driver. Nomes de header são
  validados de antemão nos dois caminhos - CR, LF e NUL (os bytes de
  injeção, como o transporte do Mailgun já recusa) e qualquer coisa que
  não seja um nome de campo RFC 5322 válido (espaços, dois-pontos, não
  ASCII) - então anexar um arquivo nunca muda se uma mensagem é aceita.

### Corrigido


- **Falhas de validação aninhadas agora chegam ao corpo do 422.** Falhas
  de `#[validate(nested)]` em um struct aninhado ou em um elemento de um
  `Vec<T>` validado eram descartadas entre o validador e a resposta: a
  solicitação era corretamente rejeitada com 422, mas o mapa `errors`
  voltava vazio, então nenhuma mensagem era renderizada e o cliente não
  conseguia dizer qual campo estava errado. Falhas aninhadas agora são
  achatadas na notação pontilhada do Laravel - `address.street`,
  `items.1.name`, `order.items.2.sku` - ao lado das de nível superior.

- **O `url` do objeto de página do Inertia mantém a query string.**
  `page.url` era apenas o path da solicitação, então o cliente registrava
  `/users` para uma visita a `/users?page=2&sort=name`. Toda navegação
  para trás/frente e todo `router.reload()` então reproduziam a página
  sem seu cursor de paginação, ordenação, ou filtros. Agora é path mais
  query - a mesma derivação que o `InertiaVersionMiddleware` já usava
  para `X-Inertia-Location`, então por padrão os dois concordam byte a
  byte. O novo `InertiaConfig::url_resolver(...)` sobrescreve como o
  *objeto de página* nomeia a página (o `Inertia::resolveUrlUsing` do
  Laravel); o bounce de versão continua nomeando a URL que chegou, porque
  é essa a URL que o navegador precisa buscar.

- **`Inertia::install` agora aplica sua config a toda resposta.** A
  config entregue a `Inertia::install` era lida em busca de três campos e
  depois descartada, então todo `InertiaResponse` construído sem um
  `.with_config(...)` explícito era renderizado a partir de
  `InertiaConfig::default()`. Um app com scaffold criado com
  `--frontend react` servia o ponto de entrada do Svelte e nenhum
  preâmbulo de refresh do React, a menos que `SUPRNOVA_FRONTEND`
  estivesse definido no ambiente; o SSR habilitado na config nunca
  alcançava uma resposta; e a versão de asset do objeto de página vinha
  de uma config diferente da do resolver do middleware de versão. A
  config instalada agora é retida no registro Inertia do contêiner, e é
  dela que `InertiaResponse::new` parte. O `.with_config(...)` por
  resposta ainda sobrescreve, apps que nunca chamam `Inertia::install`
  ficam inalterados, e um install que falhou (falha fechada) não retém
  nada. Como efeito colateral, o manifesto de produção do Vite agora é
  interpretado uma vez por processo em vez de uma vez por resposta.

- **Apps com scaffold agora instalam os middlewares do protocolo
  Inertia.** O `bootstrap.rs` escrito por `suprnova new` registrava os
  middlewares de sessão, locale, CSRF e include, mas nunca chamava
  `Inertia::install`, então um app gerado não tinha nem
  `InertiaVersionMiddleware` nem `Inertia303Middleware`: um navegador
  ainda rodando o bundle anterior nunca era avisado para recarregar
  depois de um deploy, e um `PUT`/`PATCH`/`DELETE` que redirecionava
  continuava num `302` que o cliente podia seguir com o verbo original. A
  chamada agora fica depois do `SessionMiddleware` - onde o refazer do
  flash de sessão do middleware de versão funciona - com uma constante
  nomeada `INERTIA_VERSION` para incrementar quando os assets mudarem, e
  ela fixa o frontend com o qual o projeto foi gerado
  (`.frontend(Frontend::React)` para `--frontend react`), para que o
  shell HTML carregue o ponto de entrada do Vite daquele framework em vez
  de recair para o do Svelte. O `.env` gerado agora define
  `SUPRNOVA_FRONTEND` para casar. O starter `--api` está inalterado; ele
  não tem frontend.

- **`Queue::push_unique` não relata mais um job enfileirado como
  pulado.** O valor de retorno era calculado com
  `matches!(outcome, Idempotent::Fresh(()))`, que dobrava
  `Idempotent::FreshUnfenced` em `false` - o desfecho em que o envelope
  *foi* enviado, mas o lease de dedupe foi perdido no meio do push.
  Chamadores que ramificavam sobre esse booleano eram informados de que
  um job prestes a rodar tinha sido suprimido como duplicata. Os três
  desfechos agora são casados exaustivamente: um lease perdido retorna
  `true` com um `warn` nomeando o job e sua chave única, e apenas uma
  duplicata real retorna `false`. `push_unique_later` e `later_unique`
  compartilham o caminho e são corrigidos junto.

### Alterado


- **A linha de base de paridade passou para o Laravel 13.25.0.** As notas
  de lançamento 13.23.0, 13.24.0 e 13.25.0 foram rastreadas item a item
  contra a própria superfície do framework. Tudo o que alcançou um
  caminho de código do Suprnova ou está corrigido nesta versão ou tem uma
  linha em [`parity.md`](../parity.md) marcada como `not yet`
  ou `by design no`.

### Atualizando

Duas mudanças podem alterar um app em execução sem nenhuma mudança de
código do seu lado.

- **Configurações na config que você passa para `Inertia::install` agora
  fazem efeito.** Elas eram lidas em busca de três campos e descartadas.
  Se a sua config de install define `.ssr(...)`, o SSR agora está ligado:
  inicie o worker (`suprnova ssr:start`) antes de implantar, ou remova a
  chamada `.ssr(...)`. `.entry_point`, `.assets_base_url`,
  `.default_title` e `.encrypt_history(...)` definidos ali também
  alcançam a página agora.

- **`rules::Url` rejeita mais.** Valores que antes passavam e não passam
  mais: qualquer esquema fora da allowlist do Laravel, `javascript:` e
  `vbscript:` entre eles; `mailto:`, `data:` e `tel:`, que estão na
  allowlist mas não carregam host após `://`; e `scheme://` com host
  vazio, como `file:///path`. Se a sua intenção era aceitar um esquema,
  nomeie-o: `Url::protocols(&["myapp"])`.

## 1.2.3 - 2026-08-16

### Corrigido

- **Os casts de data e hora agora leem o texto `CURRENT_TIMESTAMP` nativo do
  banco de dados.** `AsDateTime`, `AsImmutableDateTime` e
  `AsOptionalDateTime` continuam escrevendo RFC-3339 canônico, mas as leituras
  também aceitam texto do PostgreSQL com fuso e valores do SQLite/MySQL sem
  fuso. Valores sem fuso são interpretados como UTC.

## 1.2.2 - 2026-08-14

### Corrigido

- **Valores anuláveis não textuais agora funcionam em todas as escritas
  baseadas em attributes no PostgreSQL.** `Builder::update_all` e
  `Builder::upsert` tipados, `DB::table().insert/update` sem model e extras de
  pivot many-to-many emitem nulls JSON explícitos como `NULL` SQL, continuando
  a vincular todos os valores não nulos. Isso preserva o tipo da coluna de
  destino em vez de enviar um parâmetro null tipado como texto que o PostgreSQL
  rejeita para colunas bigint, integer, boolean, timestamp e outras não
  textuais. Upserts de várias linhas agora também rejeitam colunas ausentes ou
  extras em vez de converter silenciosamente uma linha malformada em null.
  Timestamps automáticos de pivots many-to-many são vinculados como datetimes
  UTC tipados em vez de texto.

### Segurança

- **O gate de lançamento agora distingue metadados dormentes do lockfile de
  dependências compiladas em todo o workspace.** O Cargo registra a dependência
  opcional de compatibilidade rkyv 0.7 não utilizada do rust_decimal em
  `Cargo.lock`; o gate agora comprova que nem o rkyv nem seu crate de derive são
  alcançáveis por qualquer membro do workspace, feature, target ou aresta de
  dependência. A exceção correspondente do RustSec é atribuída, expira em
  2026-11-14 e deve ser removida quando o rust_decimal deixar de registrar essa
  dependência opcional legada.

## 1.2.1 - 2026-08-09

### Alterado

- **O Suprnova mudou da organização `entrepeneur4lyf` para `eas4ai` no
  GitHub.** URLs do
  repositório em metadados de pacotes, documentação, exemplos de dependências e
  templates de scaffold agora usam `github.com/eas4ai`. Projetos novos também
  usam o e-mail de autor monitorado `shawn@eas4ai.com`. Esta versão não mudou o
  comportamento em runtime.

## 1.2.0 - 2026-08-05

### Adicionado

- **O manual é distribuído em sete idiomas.** `manual/es/`, `manual/fr/`,
  `manual/de/`, `manual/pt-BR/`, `manual/ja/` e `manual/zh-Hans/` trazem
  cada um o manual completo de 104 capítulos - cada capítulo, o sumário
  e este registro de mudanças - traduzido a partir da fonte em inglês. O
  inglês continua canônico: a estrutura dos capítulos, os blocos de
  código, os identificadores, os comandos de CLI e as variáveis de
  ambiente são mantidos byte a byte idênticos à fonte, então um capítulo
  traduzido nunca pode discordar do inglês sobre o que o framework faz -
  apenas dizê-lo no idioma do leitor.

  As traduções foram produzidas e revisadas para o suprnova.app, que
  renderiza este manual como o seu `/docs`. Cada seção carrega lá um
  registro de revisão: os veredictos são registrados contra hashes de
  conteúdo tanto do inglês quanto da tradução, dois revisores
  independentes precisam aprovar os bytes exatos para que uma seção
  conte como aprovada, e glossários por idioma fixam as decisões de
  terminologia (quais termos ficam em inglês, quais tomam a palavra
  nativa, e por quê). Correções são bem-vindas em qualquer um dos dois
  repositórios - uma correção aqui chega ao site na sua próxima
  sincronização.

## 1.1.0 - 2026-08-02

### Adicionado

- **Cadeias de fallback por locale.** `LocalizationConfig` ganha
  `parents` (`APP_LOCALE_PARENTS`, pares `child=parent` separados por
  vírgula, ou o construtor encadeável `.parent(child, parent)`): um
  locale pode herdar de um locale irmão configurado antes de recuar
  ainda mais para o `fallback_locale` global - `pt-PT` a partir de
  `pt-BR`, `en-AU` a partir de `en-GB`, e assim por diante,
  transitivamente. `Lang::get`/`try_get`/`get_with`/`try_get_with`/`has`
  percorrem a cadeia inteira, começando pelo locale atual, então isso
  funciona para qualquer driver `Translator`, não só o embutido. Um par
  malformado, um locale inválido, um filho nomeado duas vezes, ou um
  ciclo (incluindo um locale se nomeando como seu próprio pai) falha de
  forma explícita no carregamento da config, em vez de degradar em
  tempo de solicitação.

  Os catálogos servidos já chegam achatados ao longo da cadeia,
  calculados com antecedência: o `FluentTranslator` agora constrói o
  catálogo `/_suprnova/lang/<locale>.ftl` de cada locale como um fold -
  o catálogo do framework embutido na base para locales `en`/`en-*`,
  depois a cadeia de pais configurada do locale, depois seus próprios
  arquivos `*.ftl` - de forma que um locale encadeado continua sendo um
  único arquivo autocontido que o navegador busca uma vez, sem que o
  cliente precise ter consciência da cadeia. O achatamento cobre só os
  pais configurados; o `fallback_locale` terminal continua sendo um
  fallback no nível da facade `Lang`, e não fica embutido nos bytes
  servidos.

  Isso torna práticos os catálogos em estilo delta: um diretório
  `lang/pt-PT/` pode conter só o punhado de strings que realmente
  diferem de `lang/pt-BR/`, em vez de um catálogo duplicado completo. O
  merge que torna isso possível funciona no nível da AST do Fluent - o
  valor do filho substitui o do pai, os atributos são mesclados por
  nome (um override que não menciona um atributo deixa de perdê-lo),
  expressões `select` são substituídas por inteiro (as categorias
  plurais do CLDR dependem do locale, então mesclar variante por
  variante não seria coerente), e entradas exclusivas do filho são
  anexadas. Veja a nova seção "Fallback chains" de
  `manual/localization.md` para o contrato completo.

### Alterado

- **`LocalizationConfig` ganhou o campo `parents`.** `from_env()` e o
  construtor não são afetados; um construtor de struct literal (testes
  que constroem um `LocalizationConfig` à mão) precisa de mais um
  campo.
- **O texto de catálogo servido agora é normalizado pelo serializer
  para todo locale**, e o merge multi-arquivo intra-locale (vários
  arquivos `.ftl` em um mesmo diretório de locale) agora passa pelo
  mesmo merge no nível de AST usado nas cadeias de pais, em vez da
  simples sobrescrita de bundle. As traduções resolvidas ficam
  inalteradas, exceto pelas duas melhorias estritas abaixo; os bytes
  subjacentes giram de qualquer forma - `ETag`/`?v=<hash>` gira uma vez
  na atualização. As melhorias: um override não descarta mais
  silenciosamente os atributos que não menciona, e um override que
  contém somente atributos não elimina mais o valor próprio da
  mensagem (anteriormente um erro ou uma resolução de fallback; agora
  ele resolve para o valor do override anterior).

## 1.0.0 - 2026-08-02

### Adicionado

- **Localização.** Catálogos de mensagens em `lang/<locale>/*.ftl`
  ([Fluent](https://projectfluent.org)), uma facade `Lang` com a macro
  `__!("key", name: value)`, detecção de locale por solicitação
  (`LocaleMiddleware`: sessão → cookie → `Accept-Language` →
  `APP_LOCALE`), e formatação sensível a locale para números, moeda,
  datas, horários, listas e tempos relativos sobre o ICU4X.
  `manual/localization.md` é o capítulo.

  As regras de validação embutidas param de fixar o inglês no código.
  Cada uma retorna uma mensagem com chave (`validation-min` mais seus
  argumentos e um fallback em inglês), traduzida uma única vez na
  fronteira de serialização - assim um app em espanhol recebe erros de
  validação em espanhol só ao incluir `lang/es/validation.ftl`, sem
  envolver a regra e sem um fork das mensagens do framework. Os nomes
  de campo são humanizados por uma busca `field-<name>`. `Rule::passes`
  (e `ContextualRule` / `AsyncRule`) agora retornam
  `Result<(), ValidationMessage>`; o corpo `Err("…".into())` de uma
  regra personalizada ainda compila e ainda renderiza literalmente, mas
  a assinatura no seu `impl` precisa do novo tipo.

  O navegador recebe os mesmos bytes que o servidor resolveu: o
  catálogo mesclado é servido em `/_suprnova/lang/<locale>.ftl` com um
  ETag e uma forma imutável `?v=<hash>`, os três starter kits o
  interpretam com `@fluent/bundle`, e `suprnova generate-types` emite
  uma union `MessageKey` para que renomear uma mensagem aponte o
  compilador TypeScript para cada call site.

  Fluent em vez de arrays PHP no estilo Laravel porque um único formato
  precisa servir tanto o servidor quanto o navegador, e porque as
  categorias plurais do CLDR são o que acerta russo, polonês e árabe -
  os intervalos inteiros do `trans_choice` não conseguem, e é por isso
  que não há `trans_choice` aqui. Atrás de uma feature `localization`
  ativada por padrão; `--no-default-features` ainda compila e ainda
  valida, usando os fallbacks em inglês embutidos.

- **`IntoInertiaScroll` para `Paginator`.** A trait estava implementada
  para `LengthAwarePaginator` e `CursorPaginator`, mas não para o
  paginador simples, então resultados de `simple_paginate` não
  conseguiam alimentar `Inertia::paginate` de forma alguma - apesar de
  a própria documentação do módulo `simple.rs` apontar para ele como o
  caminho de geração de URL. Isso deixava coleções Inertia paginadas
  por offset com a escolha entre um `COUNT(*)` por solicitação e
  montar à mão os metadados de scroll. `next_page` vem da sonda de
  overflow do `LIMIT n+1`, em vez de uma última página calculada, já
  que não há total a partir do qual calculá-la.

### Corrigido

- **`suprnova generate-types` emitia um arquivo diferente a cada
  execução.** A ordenação topológica semeava sua fila de trabalho
  iterando um `HashMap`, e o Rust randomiza a ordem de iteração do
  hash por processo, então execuções consecutivas ordenavam as mesmas
  interfaces de formas diferentes. A saída é um artefato versionado,
  então toda execução produzia um diff - e um arquivo gerado que muda
  sem motivo é um arquivo que as pessoas param de regenerar, depois do
  que ele silenciosamente para de descrever o Rust que afirma
  descrever. A varredura de diretório também passou a ser ordenada,
  então a saída também não depende mais da ordem do sistema de
  arquivos. Duas execuções da mesma fonte agora são idênticas byte a
  byte.

- **`topological_sort` fazia o oposto do seu comentário de
  documentação**, emitindo dependentes antes de dependências.
  Inofensivo - uma interface TypeScript pode referenciar uma declarada
  mais adiante no mesmo arquivo - então o comentário foi corrigido em
  vez da ordem, o que teria reembaralhado um arquivo versionado sem
  nenhum benefício.

## 0.9.1 - 2026-08-01

Três defeitos, todos encontrados rodando o app de dogfood sob um
harness containerizado, em vez de lendo o código. Cada um deles é
invisível para uma suíte de testes que nunca para um processo do jeito
que produção para.

Eles se compõem numa ordem específica: um deploy rolling manda SIGKILL
em um worker no meio de um job (o primeiro), e esse job então toma um
caminho de reclaim que nunca contou a tentativa (o segundo).

### Corrigido

- **`schedule:work`, `queue:work` e `workflow:work` ignoravam
  SIGTERM.** Cada um selecionava só sobre `tokio::signal::ctrl_c()`,
  que instala um handler de SIGINT - então SIGTERM não tinha handler
  algum em lugar nenhum do processo, e SIGTERM é o que `docker stop`,
  Coolify, systemd e Kubernetes enviam. Os três já tinham um drain
  cuidadoso e limitado atrás daquele `select!`; nada disso jamais
  havia executado sob um supervisor. Medido antes da correção: um
  `docker stop` num contêiner `queue:work` queimava toda sua janela de
  graça de 40s e saía com código 137, com o job em voo destruído. Como
  PID 1 - que é o que um contêiner executa -, o kernel descarta um
  SIGTERM não tratado diretamente, então o processo não morria mal;
  ele simplesmente não morria até o SIGKILL. `Server::run` já tratava
  os dois sinais corretamente e seu listener agora é compartilhado, o
  que também fecha uma janela de sinal perdido no loop do agendador.

- **Um job que matava seu worker nunca podia ser dead-lettered.** Um
  job cujo *handler* falha recebe nack e tem sua tentativa contada,
  então ele vai para dead-letter depois de `max_tries`. Um job que
  *mata seu worker* - OOM, abort, segfault, ou o SIGKILL acima - não
  liquida nada; sua reserva simplesmente expira, e todo driver
  costumava reentregá-lo byte-idêntico. Um job assim é imortal: mata
  cada worker que o reivindica, volta inalterado, e mata o próximo,
  por quanto tempo qualquer coisa reiniciar workers. Os três drivers
  agora cobram a tentativa no momento em que descobrem que um worker
  morreu, porque trocar `QUEUE_DRIVER` não pode mudar se um job
  envenenado pode ser parado. `attempts` agora significa "entregas a
  um worker" em vez de "falhas de handler" - documentado em
  `manual/queues.md`, porque um worker perdido por razões alheias
  também queima uma tentativa.

- **…e o job esgotado agora vai para dead-letter antes de ser
  despachado.** Contar a tentativa era necessário, mas não suficiente.
  Toda decisão de dead-letter vivia no caminho de liquidação do
  worker, que assume que o handler retorna - então ela nunca rodava
  exatamente para os jobs que não conseguiam retornar. Só com a
  correção do driver o contador subia (medido: 0 → 1 → 2 ao longo de
  três workers mortos) e nada agia sobre isso. O orçamento agora é
  gasto antes do handler rodar. Descoberto só ao rodar de novo o
  experimento no contêiner, depois que a primeira correção parecia
  correta.

- **Os daemons não tinham subscriber de tracing.** `serve` recebe um
  de `init_telemetry`; `queue:work`, `schedule:work`, `schedule:run` e
  `workflow:work` passam por um caminho de boot diferente e não
  recebiam nenhum, então toda linha `tracing::` que emitiam ia parar
  em lugar nenhum, e `LOG_LEVEL` era inerte para eles. Isso é a maior
  parte do que eles têm a dizer - um worker mandando um job para
  dead-letter, um agendador pulando um tick que perdeu, um lock que
  não conseguiu liberar. Num contêiner a única saída visível era o
  banner de inicialização, e o processo parecia ocioso enquanto fazia
  tudo isso. Dois dos defeitos deste release eram invisíveis até isso
  ser corrigido.

- **Um dead-letter sem um armazenamento de failed-jobs vinculado era
  uma exclusão silenciosa.** O passo de persistência ficava dentro de
  `if let Some(store) = ..`, então sem um store o braço não casava e a
  execução caía direto no ack - mais silencioso que o caminho de falha
  logo acima, que ao menos deixa a reserva intacta. Um store ausente
  era tratado como mais bem-sucedido que um quebrado. Agora ele
  registra o envelope inteiro em ERROR, porque é isso que
  `queue:retry` reempurra: a diferença entre trabalho recuperável à
  mão e trabalho que deixou de existir.

- **`QUEUE_DRIVER=database` agora vincula um armazenamento de
  failed-jobs.** `failed_jobs` faz parte do contrato desse driver -
  `queue:retry` o lê e `Queue::retry_failed` não funciona sem ele -
  mas `bootstrap_from_env` conectava o driver e deixava o store sem
  definir, então uma fila apoiada em banco de dados mandava para
  dead-letter no nada, a menos que o app vinculasse um à mão.
  Configurável via `QUEUE_FAILED_DB_TABLE`. Só para este driver:
  `memory` é efêmero por construção e `redis` não tem tabela para
  escrever.

- **A latência de reclaim do Redis agora acompanha
  `--visibility-timeout`.** A flag define o limiar de idle do
  XAUTOCLAIM, mas um clock separado governa a frequência com que um
  consumer olha, e o driver a deixava no padrão de 30s do
  sea-streamer - então `--visibility-timeout 5` na prática significava
  "até 35 segundos". O intervalo agora acompanha o timeout
  configurado, limitado entre 1s e 30s, de forma que um timeout curto
  não pode virar uma tempestade de XAUTOCLAIM, e um longo só pode
  tornar o reclaim mais rápido que antes.

### Adicionado

- **`TaskBuilder::on_one_server()` / `on_one_server_for(ttl)`** -
  executa uma tarefa agendada exatamente uma vez por tick devido,
  entre réplicas. Sem isso nada elege um líder para um tick: cada
  processo `schedule:work` avalia o schedule independentemente, e três
  réplicas foram medidas executando toda tarefa devida três vezes, a
  cada minuto, sem variância nenhuma. Um job noturno de faturamento em
  três réplicas faturava cada cliente três vezes.

  `without_overlapping()` não cobre isso e não pode: seu lock é
  chaveado na tarefa e liberado quando o handler retorna, então uma
  tarefa rápida o libera antes de uma segunda réplica olhar.
  `on_one_server` chaveia na tarefa *e no tick* e segura o lock além
  do handler, deixando-o expirar por TTL. Os dois se compõem.

  Opt-in, seguindo o Laravel. Diverge do Laravel ao falhar de forma
  fechada: a eleição só é tão compartilhada quanto o cache por trás
  dela, então um boot de produção com `CACHE_DRIVER=memory` e uma
  tarefa de servidor único é recusado, nomeando as tarefas culpadas,
  com `SCHEDULE_ALLOW_MEMORY_LOCK_IN_PRODUCTION=true` para
  deployments que genuinamente rodam um único scheduler.

### Alterado

- `manual/deployment.md` não diz mais "rode exatamente um processo
  `schedule:work`" como única opção, e ganha uma seção **Stopping
  cleanly** cobrindo as janelas de drain por subsistema, como
  dimensionar a graça de terminação de uma plataforma acima delas, e
  por que PID 1 torna um handler de sinal ausente pior do que parece.

## 0.9.0 - 2026-07-31

### Segurança

- **A emissão de auth só podia ser limitada por caller, nunca por
  destinatário.** Um limite chaveado por endereço responde "um cliente
  está barulhento?"; não consegue responder "uma caixa de entrada está
  sendo inundada?". Um atacante espalhado por uma botnet ou por um
  único `/64` IPv6 permanecia abaixo de todo orçamento por IP enquanto
  enchia a caixa de entrada de uma vítima com e-mail de redefinição de
  senha, e nada no framework conseguia expressar o limite que teria
  impedido isso - uma key function conseguia ler o path, os
  cabeçalhos e a query string, mas não um corpo form-encoded, então o
  endereço era invisível exatamente na rota que o carrega.

  `identity_key` chaveia um bucket na conta sendo afetada. Ela lê a
  query string primeiro e depois um corpo de formulário bufferizado,
  então uma única key function cobre as duas formas; o valor é
  aparado e colocado em minúsculas, porque `Alice@Example.com` chega
  na mesma caixa de entrada que `alice@example.com`, e um limite
  contornável segurando o shift não é um limite; e é hasheado, porque
  um backend de rate limit é frequentemente um Redis compartilhado com
  controle de acesso mais fraco que o do banco de dados primário.

  Dois novos construtores de middleware dão suporte a isso.
  `key_reads_body(cap)` bufferiza o corpo antes de chavear - opt-in,
  porque bufferizar é trabalho que um caller não autenticado consegue
  te obrigar a fazer, e um corpo acima do cap é recusado com 413 em
  vez de passado adiante sem chave. `only_when(pred)` pula um limiter
  inteiramente para solicitações sobre as quais ele não tem nada a
  dizer, o que é o que impede um orçamento por destinatário empilhado
  de silenciosamente virar o limite vinculante em rotas que não
  nomeiam destinatário nenhum.

  O app de dogfood agora empilha os dois no seu grupo de emissão: 10
  a cada 5 minutos por endereço, 3 a cada 15 minutos por destinatário.

Uma revisão dos caminhos de sessão, senha, OAuth e passkey do Torii
revelou oito defeitos, todos corrigidos no fork fixado
(`suprnova-torii-rs` `968b0be`).

- **Sessões expiradas podiam ser renovadas de volta à vida.** O
  `refresh` do repositório de sessão do SeaORM não tinha predicado de
  expiração e estendia `expires_at` incondicionalmente, e
  `OpaqueSessionProvider::refresh_session` pulava a checagem
  `is_expired()` que `get_session` faz. Um token mantido além de sua
  expiração podia ser renovado indefinidamente. Corrigido nas duas
  camadas. Não alcançável pela própria superfície do Suprnova - nem o
  `Torii` nem o framework expõem renovação de sessão - mas é API
  pública dos dois crates.
- **O formulário de login vazava quais contas existem, por timing.** A
  autenticação retornava assim que o e-mail não batia, pulando o
  Argon2 inteiramente: medido em 54µs para um endereço desconhecido
  contra 719ms para uma senha errada, uma diferença de ~13.000x
  legível pela rede. Os dois caminhos de falha agora verificam contra
  um hash fictício, então custam o mesmo. Este *era* alcançável pelo
  login por senha do Suprnova.
- **A claim `iss` do JWT era escrita, mas nunca verificada.** A
  fixação de algoritmo já estava correta - `alg: none` e a confusão
  HS/RS nunca foram possíveis - mas o issuer era decoração, então dois
  serviços compartilhando uma chave de assinatura aceitariam as
  sessões um do outro. Agora aplicado quando um issuer é configurado.
- **Um verificador PKCE de uso único podia ser reivindicado duas
  vezes.** O consumo era uma leitura seguida de uma exclusão, então
  dois callbacks OAuth para o mesmo `csrf_state` podiam ambos ler
  antes de qualquer exclusão acontecer. Agora reivindicado em uma
  única operação - `DELETE ... RETURNING` no Postgres, uma exclusão
  por chave primária cuja contagem de linhas afetadas escolhe o
  vencedor no SeaORM.
- **Sessões expiradas eram listadas como ativas.** `find_by_user_id`
  não tinha filtro de expiração, e linhas expiradas sobrevivem até a
  limpeza rodar, então uma tela de "dispositivos em que você está
  conectado" oferecia aos usuários sessões mortas para revogar,
  sem dizer nada sobre a que estava viva.
- **Uma busca de passkey se chamava `authenticate`.** O
  `PasskeyService::authenticate_credential` do Torii recebia um ID de
  credencial e retornava o usuário dono, e `PasskeyAuth::authenticate`
  cunhava uma sessão a partir disso. O Torii armazena passkeys - não
  carrega dependência de WebAuthn nenhuma e não consegue verificar uma
  assertion, então a única coisa que essas chamadas provavam era que o
  caller conhecia um ID de credencial: um valor que o navegador envia
  em claro e que `allowCredentials` entrega a qualquer um que consiga
  iniciar uma cerimônia. Renomeado para `find_user_by_credential` e
  `create_session_for_verified_credential`, ambos documentando que a
  verificação é trabalho do caller. Não alcançável através do
  Suprnova, que dirige o próprio `webauthn-rs` (veja
  `torii_integration::passkey`) e só alcança o Torii para
  armazenamento de credenciais.
- **Um desafio WebAuthn podia sofrer replay durante todo o seu TTL.**
  Nenhum dos dois backends consumia um desafio na leitura, e o
  `get_challenge` do SeaORM também ignorava `expires_at` por
  completo, retornando desafios expirados como se estivessem vivos.
  Leituras agora excluem linhas expiradas nos dois backends, e um novo
  `take_challenge` reivindica um exatamente uma vez - a mesma forma de
  "a exclusão decide o vencedor" da correção do PKCE.

### Mudanças incompatíveis

- **Azure Blob Storage e Google Cloud Storage se mudaram para trás
  das novas features `filesystem-azure` e `filesystem-gcs`.**
  `Storage::register_azblob`, `register_azblob_with`, `register_gcs`,
  `register_gcs_with`, `AzBlobConfig` e `GcsConfig` não existem mais a
  menos que você ative a feature correspondente. Se você usa qualquer
  um dos dois backends, adicione-a à sua dependência:

  ```toml
  suprnova = { git = "…", tag = "v…", features = ["filesystem-gcs"] }
  ```

  Você recebe um erro de compilação nomeando o item ausente, não uma
  falha em tempo de execução.

  Os dois crates de serviço do opendal puxam `rsa`, que carrega o
  RUSTSEC-2023-0071 (o ataque de timing Marvin) sem release corrigido
  upstream. Eram os únicos crates ativando `reqsign-core/jwt`, a
  feature atrás da qual está o `rsa` opcional do `reqsign-core`, então
  colocá-los atrás de gate corta os três caminhos do opendal até ele
  de uma vez. `rsa` agora é *evitável*: `--no-default-features
  --features filesystem,database-postgres` resolve sem ele e ainda
  tem o subsistema de storage. Antes, nenhuma combinação de features
  conseguia se livrar dele mantendo o storage de alguma forma.

  Um build padrão de fábrica ainda carrega `rsa` - `database-mysql` é
  uma feature padrão e `sqlx-mysql 0.8.6` depende dele de forma não
  opcional - então a exceção de auditoria continua aberta. O S3
  deliberadamente **não** fica atrás de gate: `reqsign-aws-v4` usa
  `reqsign-core` sem `jwt`, então o driver S3 nunca contribuiu com um
  caminho até ele, e colocá-lo atrás de gate quebraria o backend de
  nuvem mais usado sem remover nada.

### Adicionado

- **`suprnova --version`**, com `-v` além do `-V` padrão do clap.
  Perguntar a versão de uma CLI com a flag que toda outra CLI usa não
  devia imprimir um erro de uso.

### Corrigido

- **Duas operações do Redis não tinham limite superior.** O flush de
  tag do cache lia o conjunto de membros inteiro de uma tag com
  `SMEMBERS` e excluía chave por chave, então uma tag com uma
  associação grande travava a conexão, e uma escrita concorrente podia
  se perder entre a leitura e a exclusão; tags agora são baseadas em
  geração, liberadas atomicamente, e varridas com um `SSCAN` limitado.
  O passo de promoção da fila atrasada movia todo job devido em um
  único `ZRANGEBYSCORE` sem limite, então um backlog que vencia junto
  produzia um único script enorme; agora ele promove em lotes.
- **Dois drains de shutdown esperavam para sempre.** `schedule:work`
  no Ctrl-C e o worker de workflow após cancelamento aguardavam cada
  um toda tarefa em voo sem prazo, então uma tarefa que nunca
  retornava mantinha o processo aberto até o `SIGKILL` - um operador
  vê um daemon que "não para". Os dois agora esperam uma graça
  limitada, depois abortam o que resta e reportam a contagem.
- **A varredura de fixação de versão do release só reconhecia uma das
  duas sintaxes de fixação**, então todo arquivo carregando uma linha
  `cargo install --tag vX.Y.Z` e nenhum trecho de dependência nunca
  era descoberto. `suprnova-cli/README.md` vinha dizendo aos leitores
  para instalar a v0.6.0 havia três releases; `manual/cli.md` e
  `manual/cli-new.md` estavam parados na v0.7.2; `manual/installation.md`
  carregava as duas formas e tinha uma atualizada enquanto a outra
  congelava. A descoberta e a reescrita agora leem de uma única tabela
  de padrões, e as regras de um arquivo derivam do seu conteúdo.
- **`cargo doc` falhava para qualquer build com `filesystem`, mas sem
  `testing`** - sete links intra-doc de `Storage::fake` não
  conseguiam resolver, e `lib.rs` nega links quebrados. `testing` é
  uma feature padrão, então nenhum passo de gate jamais tinha
  construído essa combinação; `check-feature-matrix.sh` agora faz
  isso.
- **As migrações do Torii não podiam ser replayadas sobre seu próprio
  schema**, então um banco de dados que o mantinha sem a tabela de
  rastreamento `torii_migrations` - restaurado de um dump que a pulou,
  ou migrado à mão - não podia ser trazido sob gestão. Todo
  `Table::create()` carregava `.if_not_exists()`; nenhuma das 19
  chamadas `Index::create()` carregava, nem o alter `ADD COLUMN
  locked_at`, então o replay passava pelas tabelas e morria no
  primeiro `CREATE INDEX`. Corrigido no fork fixado
  (`suprnova-torii-rs` `a0f956d`) via `has_index` / `has_column` em
  vez de `IF NOT EXISTS`, que o sea-query silenciosamente descarta
  para MySQL - a correção sintática teria deixado quebrado um build
  com as features padrão.
- **Uma migração do Torii que falhava abortava o processo em vez de
  retornar um erro.** `SeaORMStorage::migrate` desembrulhava
  (`unwrap`) o migrador e retornava `Ok(())` incondicionalmente,
  então o mapeamento que `init_torii` fazia da falha para um
  `FrameworkError` era código inalcançável.
- **A própria tabela `users` de um app suprimia silenciosamente a do
  Torii**, porque `.if_not_exists()` não consegue distinguir "já é
  minha" de "já é de outra pessoa". A migração reportava sucesso e a
  autenticação falhava depois numa coluna ausente - a razão pela qual
  o starter `--api` nomeia sua tabela `app_users`. A migração do
  Torii agora avisa, no momento da migração, quando uma tabela
  `users` existente não tem colunas que ela exige, nomeando as
  colunas e o remédio. Continua sendo um aviso, não uma falha dura,
  para que deployments existentes continuem inicializando.
- **Os guias de deployment do Railway e da DigitalOcean apontavam o
  health check da plataforma para um path que podia sondar o
  Postgres.** As duas plataformas reiniciam o contêiner quando essa
  checagem falha, então seguir o conselho transformava uma soluço de
  banco de dados num loop de reinício em toda réplica. As duas agora
  usam `/_suprnova/health/live`, com o banco de dados sondado à mão
  pelo console. Os paths legados ainda resolvem; nada já implantado
  precisa mudar.

## 0.8.0 - 2026-07-30

Remediação de uma auditoria externa de red team. A auditoria retornou 19
achados P1 e um veredito NO-GO para o 1.0; este release fecha **os
dezenove**, mais vários defeitos encontrados enquanto os corrigia que a
auditoria não tinha nomeado.

Várias correções deliberadamente transformam uma configuração incorreta
silenciosa num boot recusado. Leia **Atualizando** antes de fazer
deploy - um app de produção que vinha rodando feliz pode não iniciar.

### Atualizando

Três configurações que costumavam inicializar com um aviso (ou em
silêncio) agora falham de forma fechada em produção. Cada erro nomeia
a variável que o destrava, e cada uma tem uma sobrescrita explícita
para o deployment onde o risco genuinamente não existe.

- **Um driver de mail que não entrega.** `MAIL_DRIVER` sem definir,
  `log`, `memory`, ou um valor não reconhecido, todos resolviam para
  um transporte que renderiza o mail e o descarta - então
  redefinições de senha reportavam sucesso enquanto nada era enviado.
  Sobrescrita: `MAIL_ALLOW_NON_DELIVERING_IN_PRODUCTION=true`.
- **SMTP em texto claro.** Três das quatro combinações de credenciais
  caíam num transporte não criptografado, e o caso com as duas
  ausentes registrava um aviso e enviava assim mesmo. Sobrescrita:
  `MAIL_ALLOW_INSECURE_SMTP_IN_PRODUCTION=true`.
- **O rate limiter em memória.** Seus buckets vivem no heap de um
  único processo, então atrás de N réplicas todo quota é na verdade
  N× e cada deploy os reseta. Aponte `RATE_LIMIT_DRIVER` para
  `redis`, ou defina `RATE_LIMIT_ALLOW_MEMORY_IN_PRODUCTION=true` se
  você genuinamente roda um único processo. Um valor de driver *não
  reconhecido* falha pelo mesmo motivo, porque caía de volta para
  memory - `RATE_LIMIT_DRIVER=Redis`, com maiúscula, é o caso com
  mais chance de chegar a produção porque parece configurado.

Desenvolvimento, testes e staging ficam inalterados nos três casos.
Staging deliberadamente não passa por esse gate: falhar
implacavelmente nele empurra os times a definir a sobrescrita
globalmente, o que desarma a checagem exatamente onde ela importa.

Duas mudanças de comportamento que não são falhas de boot:

- **`fill` e `first_or_new` rejeitam valores malformados.** Um valor
  que não conseguia decodificar para o tipo do seu campo costumava
  virar o `Default` daquele campo e retornar `Ok` -
  `fill(attrs!{ age: "abc" })` definia `age = 0` e reportava sucesso.
  Agora retorna um `ValidationError` nomeando o campo, e deixa o
  model intocado. Colunas desconhecidas continuam sendo puladas
  silenciosamente (paridade com o Laravel), e o alargamento numérico
  continua funcionando.
- **`/_suprnova/health?db=true` não retorna mais o erro do driver.**
  O detalhe se muda para o log; o corpo mantém `"database": "error"`.
  Builds de debug ainda o incluem. Dashboards que fazem parse de
  `status` / `database` não são afetados.
- **`url::signature_has_not_expired` agora exige uma assinatura
  válida**, e está descontinuada. Costumava responder `true` para uma
  URL forjada - uma assinatura ruim não está "expirada", porque nunca
  teve uma expiração para perder - então qualquer handler que se
  protegia só com ela aceitava forjas. Agora é idêntica a
  `has_valid_signature`. Se você a usava para distinguir *expirada*
  de *inválida* (para renderizar "peça um link novo" em vez de um
  403), mude para `url::signature_verdict`, que retorna os três
  estados. Isso diverge deliberadamente do `URL::signatureHasNotExpired`
  do Laravel.

Duas adições que só pedem algo de você se você optar por entrar:

- **`QueueDriver` ganhou `settle` e `release`**, os dois com
  implementações padrão, então impls de driver existentes continuam
  compilando sem alteração. Implemente `settle` se seu backend
  consegue commitar uma escrita de acompanhamento e uma confirmação
  em uma única transação; implemente `release` se ele consegue
  reenfileirar uma mensagem reservada no lugar.
- **A contabilidade de batch agora pode ser durável.**
  `DatabaseBatchRepository` precisa de duas tabelas novas,
  `job_batches` e `job_batch_settlements` - adicione-as às suas
  migrações, como com `jobs` e `failed_jobs`. O schema está em
  `manual/queues.md`. Nada muda se você permanecer no
  `MemoryBatchRepository`.

### Segurança

- **Slowloris (SEC-07).** O timeout de leitura de header do hyper era
  documentado como 30s, mas estava inerte - ele só arma quando um
  timer é instalado no connection builder, e nenhum era. Um cliente
  podia segurar uma conexão, e uma permissão de
  `SERVER_MAX_CONNECTIONS`, indefinidamente. Agora armado e
  configurável via `SERVER_HEADER_READ_TIMEOUT`.
- **Uploads multipart (SEC-05).** O cap se aplicava aos payloads de
  cada part individual, mas não ao stream bruto, então um corpo podia
  exceder o limite em agregado. Agora limitado no stream.
- **HMAC de webhook com chave vazia (SEC-08).** Os dois adaptadores de
  pagamento aceitavam um secret em branco, que verifica qualquer
  coisa. Recusado nos dois.
- **Parsing de assinatura da Paddle (P2-11).** Um `paddle-signature`
  de tamanho ímpar ou não hexadecimal chegava ao SDK fixado e entrava
  em panic dentro dele. Agora validado primeiro: uma assinatura
  malformada é um 401.
- **Cadastro de passkey e tokens de reset (SEC-01, SEC-02).** Cadastro
  anônimo contra um e-mail existente, cadastro por um não-dono, e
  cadastro pelo dono sem reautenticação recente são cada um recusados
  com status distintos. Um login por senha agora carimba a janela de
  reautenticação.
- **`dev:tls` (SEC-10).** Um projeto podia escolher a CA em que o
  comando confia.
- **Docker Compose gerado (P2-12).** Publicava Postgres e Redis em
  todas as interfaces com credenciais commitadas neste repositório.
  Agora vinculado a loopback com senhas geradas por scaffold, `.env`
  escrito com 0600, e alvos symlinkados recusados.
- **Endpoint de health (P2-01, CI-05).** Ele decidia se consultava o
  banco de dados com `query.contains("db=true")` - um teste de
  substring, então `?nodb=true` também rodava a sonda. Agora
  interpretado corretamente. O 503 não embute mais o erro do driver,
  que nomeava hosts, portas, schemas e versões.
- **Limitação de emissão de credenciais (P2-02).** As quatro rotas de
  emissão de auth no app de referência não carregavam rate limit
  nenhum, e a única rota que carregava chaveava seu bucket no header
  bruto `x-forwarded-for` - que qualquer cliente pode variar a cada
  solicitação para obter um bucket novo. As duas corrigidas; o
  orçamento de emissão é compartilhado entre as quatro rotas, então
  alternar entre elas não o multiplica.
- **Um step de chain redelivered reempurrava seu sucessor sob um id
  novo (DATA-02b, parcial).** A liquidação empurra o próximo elo da
  chain *antes* de dar ack, deliberadamente: dar ack primeiro
  significa que um crash nessa janela perde a chain permanentemente, e
  uma duplicata é recuperável onde uma perda silenciosa não é. Mas o
  envelope do sucessor recebia um `Uuid::new_v4()` novo a cada push,
  então a duplicata produzida por essa troca era indistinguível de um
  step novo legítimo - para o driver, para um outbox, e para o
  handler.

  Esse último é o custo real. O contrato de entrega do framework é
  at-least-once e sua resposta para duplicatas é "handlers precisam
  ser idempotentes" - mas um handler chaveado em `env.id`, o único
  identificador que recebe, não conseguia satisfazer esse contrato
  para um job encadeado, porque a duplicata chegava sob um id novo
  toda vez. O contrato era insatisfazível por construção.

  O id do sucessor agora é um UUIDv5 derivado do de seu predecessor,
  que é estável ao longo das próprias redeliveries desse predecessor.
  Um step redelivered reempurra o id que empurrou antes. Nenhuma
  mudança de schema, nenhum campo novo, nenhuma dependência nova.

  Isso torna a duplicata **detectável**, que é a primitiva que faltava
  no resto do DATA-02b. Não torna o push atômico com o ack (isso
  precisa do outbox), e nada ainda rejeita a duplicata na entrada. Os
  dois continuam em aberto.
- **URLs assinadas verificavam uma URL e executavam outra (SEC-04).**
  A forma canônica colapsava pares de query num map, então uma chave
  repetida mantinha só seu **último** valor - enquanto
  `Request::query_param` retornava o **primeiro**. Um `?user=victim`
  legitimamente assinado podia então ser replayado como
  `?user=attacker&user=victim` com a assinatura original intacta: a
  verificação canonicalizava sobre `victim` e passava, e o handler
  agia sobre `attacker`.

  A forma canônica agora carrega todo par, ordenado por `(key,
  value)`, então a assinatura cobre o multiset exato de parâmetros -
  adicionar, remover ou substituir qualquer valor quebra o HMAC. Um
  `signature` ou `expires` repetido é recusado de imediato, já que
  duas ocorrências de qualquer um dos dois não deixam resposta não
  arbitrária sobre qual delas vale.

  `Request::query_param` agora resolve uma chave repetida para seu
  último valor, casando com `query_params` e `Context::query_param`;
  era o único dos três que discordava, e essa discordância era a
  outra metade do defeito. **Links assinados existentes continuam
  funcionando** - sem chaves repetidas os bytes do payload ficam
  inalterados, o que um teste fixa, porque uma mudança de forma
  canônica que invalidasse silenciosamente todo link de redefinição
  de senha pendente seria pior que o bug.

  Seis testes de regressão, incluindo as duas ordens de ataque, uma
  chave legitimamente repetida que ainda precisa assinar e verificar,
  e a garantia de reordenação. *Não* mudado: `signature_has_not_expired`
  continua reportando uma assinatura forjada como "não expirada".
  Esse é o comportamento do Laravel, foi resolvido deliberadamente
  como uma correção de documentação, e tem seu próprio teste
  fixando-o contra uma "correção" bem-intencionada.
- **RBAC sob Postgres.** Verificado contra um Postgres real, e não só
  contra SQLite.
- **Quatro avisos do RustSec eliminados, não renovados.** O driver do
  Pinecone foi reescrito contra a API REST do Pinecone, derrubando
  `pinecone-sdk 0.1.2` - cujo release mais novo data de 2024-09-06 -
  e com ele `tonic 0.11 → rustls 0.22 → rustls-webpki 0.102` e o
  RUSTSEC-2026-0049 / -0098 / -0099 / -0104. Os quatro foram
  corrigidos upstream em `rustls-webpki >= 0.103.13`, que este
  workspace já resolvia para seus outros usuários de TLS; um único
  crate abandonado prendia a árvore na linha vulnerável.
  `.cargo/audit.toml` caiu de cinco exceções para uma. Veja
  **Alterado** para o que isso significa para a API do driver.
- **Exceções de auditoria agora expiram.** Toda entrada em
  `.cargo/audit.toml` carrega um `OWNER` e uma data `EXPIRES`, e
  `scripts/check-audit.sh` falha o gate de release num owner ausente,
  numa data ausente ou não interpretável, ou numa já vencida. `cargo
  audit` não tem noção de uma exceção com prazo, então uma adicionada
  "temporariamente" ficava até alguém reler o arquivo. A entrada
  restante (RUSTSEC-2023-0071, `rsa`, que não tem release corrigido
  nenhum) tem dono e data.
- **Alegações de alcançabilidade são checadas, não simplesmente
  afirmadas.** `scripts/check-feature-matrix.sh` resolve árvores de
  dependência reais e garante que nenhum build - incluindo
  `--all-features`, que é o que `cargo audit` de fato lê - contém
  `pinecone-sdk`, `rustls-webpki 0.102.x` ou `tonic 0.11.x`. Uma
  exceção justificada por um comentário que nada verifica deixa de
  ser verdade na primeira vez que alguém adiciona uma dependência.

### Corrigido

- **Todo release numa fila apoiada em banco de dados era silenciosamente
  um no-op.** `JobOutcome::Released` - um lock `WithoutOverlapping`
  ocupado, um backoff de rate limiter - era implementado como "empurra
  uma cópia, depois dá ack no original". O id do envelope é a chave
  primária da tabela `jobs`, então a cópia colidia com a linha que
  ainda segurava a reserva viva, e o push falhava com
  `UNIQUE constraint failed: jobs.id`. O worker então corretamente se
  recusava a dar ack, então o atraso solicitado nunca era aplicado,
  nenhum evento `JobReleased` disparava, e o job simplesmente ficava
  parado até a expiração de visibilidade redeliverá-lo. Releases agora
  são uma única chamada de driver, feita no lugar.
- **Um dispatch de batch parcial orfanava os jobs que já tinha
  enfileirado (DATA-02).** Quando um `driver.push` falhava no meio do
  loop, `PendingBatch::dispatch` excluía a linha do batch - mas os
  envelopes já na fila continuavam carimbados com aquele id de batch,
  então cada um deles liquidava contra um batch que não existia mais,
  retornando `Err(batch not found)` a cada entrega, para sempre. O
  batch agora é liquidado em vez disso: jobs não despachados são
  registrados como falhas e o batch é cancelado, então os enfileirados
  liquidam normalmente e os callbacks terminais ainda disparam.
- **Nada testava que `url::has_valid_signature` rejeita uma URL
  forjada.** Encontrado ao verificar a correção do SEC-04: a suíte
  inteira do framework passava com a guarda primária de URL assinada
  reescrita para aceitar qualquer assinatura.
- **Um app com scaffold não conseguia migrar seu banco de dados nem
  construir sua imagem (REL-01b).** Nenhum dos dois scaffolds
  declarava `default-run`, então todos os nove wrappers de CLI que
  chamam `cargo run` via shell falhavam num projeto recém-criado. O
  Dockerfile gerado tinha cinco defeitos independentes - um COPY de
  lockfile ausente, `npm ci` sem um lock, um estágio de cache
  stubando um dos dois binários declarados, um build de frontend
  copiado de um path que o vite nunca cria, e um COPY ausente de
  `frontend/src/pages`, que `inertia_response!` valida em tempo de
  compilação. A imagem de um scaffold de fábrica não conseguia
  construir.
- **`docker:init` emitia um único Dockerfile para todo tipo de
  projeto.** Num projeto `--api`, sua primeira instrução, `COPY
  frontend/package.json`, falhava de imediato. Projetos API agora
  recebem um Dockerfile sem frontend.
- **Placeholders SQL (DATA-01).** Renderizados por backend, em vez de
  assumir um único dialeto.
- **Liquidação de fila (DATA-02a, P2-06c).** Follow-ups liquidam antes
  de a reserva receber ack, e um erro de liberação de lock não
  converte mais um job já bem-sucedido num retry.
- **Um batch cancelado disparava `Catch`, nunca `Then`.**
- **`Builder::clone` descartava silenciosamente o plano de eager-load
  (P2-09a).** `User::query().with("posts")` clonado em qualquer
  lugar - paginação, `count()`, qualquer scope que clona - retornava
  linhas sem relações e sem erro.
- **Rosters de presence perdiam membros (P2-08).** O roster tinha seu
  snapshot tirado antes de assinar, então quem entrasse nessa janela
  não aparecia em nenhum dos dois, permanentemente.
- **O Pinecone serializava toda aquisição de índice (P2-14).** O lock
  de escrita era segurado ao longo de dois round trips de rede, e o
  `RwLock` justo do `tokio` fazia um índice frio travar todo índice
  quente.
- **O type watcher descartava rajadas (P2-13).** O debounce de borda
  de subida regenerava no primeiro arquivo de uma rajada e descartava
  o resto sem uma execução final, então o último save nunca fazia
  efeito.
- **`ssr:check` podia travar, e tentava um único endereço (P2-13).** O
  DNS rodava totalmente fora do timeout, e só o primeiro endereço
  resolvido era tentado - então um host com um registro AAAA e sem
  rota IPv6 reportava o worker fora do ar enquanto ele estava
  escutando em v4.
- **`suprnova serve` instalava `cargo-watch` sem fixação (P2-13).**
  Agora com `--locked` e um limite de versão major.
- **O bumper de release reescrevia cinco READMEs e nada mais.** Quatro
  capítulos do manual e um doc comment público fixavam tags que
  release nenhum jamais atualizava - o doc comment estava dois
  releases desatualizado. A descoberta agora substitui a lista mantida
  à mão, e o smoke test faz grep na árvore atualizada
  independentemente, em vez de confiar no próprio passo de verificação
  do bumper.
- **`db:sync` tratava o schema do banco de dados como entrada confiável
  (CLI-01).**
- **`migrate:fresh` fica atrás de gate por `--force` mais uma
  confirmação tipada (CLI-02)**, tanto no binário do app quanto na
  CLI.
- **O driver de mail `log` agora registra a mensagem inteira**, como o
  Laravel faz, e não escreve mais links bearer no log em produção.

### Adicionado

- **Liquidação terminal atômica (`QueueDriver::settle`, DATA-02).** O
  sucessor da chain e a confirmação agora commitam juntos no
  `DatabaseQueueDriver`, fechando a janela em que um crash entre os
  dois ou perdia o resto de uma chain, ou rodava seu próximo step duas
  vezes. A exclusão chaveada pela reserva serve também como fence: um
  worker cuja visibilidade expirou no meio da execução não commita
  nada e reporta `Settled::Stale`, então não consegue enfileirar
  trabalho para uma mensagem que outro consumer agora possui. Drivers
  que não conseguem fazer isso respondem `Settled::Unsupported` e
  mantêm a ordem documentada de push-antes-do-ack.
- **`DatabaseBatchRepository` (DATA-02).** A contabilidade de batch
  sobrevive a um restart, e `pending_jobs`/`failed_jobs` são derivados
  de linhas de liquidação chaveadas por `(batch_id, job_id)`, em vez de
  armazenados e decrementados - então um job redelivered não consegue
  levar um batch a "finished" enquanto seus outros jobs ainda estão
  rodando, e a salvaguarda vale entre processos, não só dentro de um.
- **`/_suprnova/health/live` e `/_suprnova/health/ready`.** Liveness
  não toca em nada; readiness sonda dependências. Conectar uma
  checagem de banco de dados numa sonda de liveness transforma um
  soluço de banco de dados num restart em cascata de toda réplica, o
  que o único endpoint anterior convidava.
  `/_suprnova/health` continua funcionando exatamente como
  documentado.
- **`SERVER_HEALTH_READINESS_TOKEN`.** Secret compartilhado opcional
  para a sonda de readiness, comparado em tempo constante. Sem ele,
  readiness responde 404 - indistinguível de um path não roteado,
  porque *é* o próprio 404 do router. Não definido por padrão, então
  sondas existentes continuam funcionando.
- **`MAIL_SMTP_ENCRYPTION`** - `starttls` | `tls` | `none`, com `ssl` e
  `null` aceitos como aliases compatíveis com o Laravel. Sem definir,
  deriva das credenciais, reproduzindo exatamente o comportamento
  anterior. Isso também torna alcançável o TLS implícito na porta 465:
  o transporte já suportava, mas nenhuma combinação de variáveis de
  ambiente conseguia selecioná-lo.
- **`SERVER_MAX_CONNECTIONS` e `SERVER_HEADER_READ_TIMEOUT`**
  documentados em `manual/env-vars.md`, onde estavam totalmente
  ausentes.

### Alterado

A própria conclusão da auditoria foi que o gate passou em 470s e não
pegou nenhum dos 19 P1s. A maior parte do trabalho de testes deste
release mira nisso.

- **Postgres roda no gate.** Doze testes em seis arquivos nunca
  tinham executado. Dois deles se revelaram apontando `DROP TABLE`
  para qualquer Postgres que estivesse em `localhost:5432` por
  padrão, e nenhum dos dois jamais tinha inicializado `Crypt`, então
  os dois falhavam na primeira vez que rodavam.
- **Asserções de scaffold leem os bytes que um usuário recebe**, após
  a substituição, em vez da fonte do template. Encontrou um projeto
  API entregando um doc comment nomeando um banco de dados
  literalmente `{package_name}`, e um `.env.example` anunciando cinco
  chaves de mail que o framework nunca lê.
- **Injeção de falha na fila.** Perda de ACK, redelivery, expiração de
  lease e dispatch parcial são dirigidos por um decorator que faz uma
  operação nomeada falhar numa chamada nomeada, então todo caso é
  determinístico em vez de uma corrida de sleep.
- **Adaptadores de pagamento têm testes negativos.** O `verify()` da
  Stripe nunca tinha sido exercitado com uma assinatura *válida*,
  então todo caminho de rejeição que depende de chegar à comparação
  HMAC não estava provado.
- **O driver do Pinecone fala REST.** *Incompatível, atrás da feature
  `vector-pinecone`, desligada por padrão.* A motivação está em
  **Segurança**; as mudanças de superfície são:
  - `client()` sumiu - não existe mais `PineconeClient`. No lugar
    ficam `control_plane_get`, `control_plane_post` e
    `data_plane_post`, que alcançam *qualquer* endpoint do Pinecone
    com seus próprios tipos de request e response sobre o transporte
    autenticado e com host resolvido do driver. Isso é estritamente
    mais alcance do que o trapdoor antigo tinha.
  - `json_to_metadata` → `metadata_from_json`, e metadata agora é
    `serde_json::Map` em vez de `prost_types::Struct`.
    `decode_match_fields` → `decode_match`, recebendo um
    `PineconeMatch`. `namespace()` retorna `&str`.
  - Novo: `with_control_plane`, `with_api_version`, `with_index_host`
    (fixa um host conhecido e pula o round trip do control plane),
    `index_host`, e os tipos de wire `PineconeVector` /
    `PineconeMatch`.
  - `from_env` ainda lê `PINECONE_API_KEY` e
    `PINECONE_CONTROLLER_HOST`, e agora também `PINECONE_API_VERSION`.
  - A versão da API REST é fixada, não flutuante - `2025-04`, a versão
    contra a qual as formas de request e response do driver foram
    escritas.
  - Nada mais serializa. O driver antigo cacheava um
    `Index` por nome atrás de um `tokio::Mutex` porque `pinecone-sdk`
    só o expunha atrás de `&mut self`; o novo cacheia uma string de
    host e compartilha o pool de conexões do `reqwest`.
  - Um host aprendido do control plane é sempre contatado sobre
    `https`, qualquer que seja o scheme que a resposta carregue.
  - `Debug` é implementado à mão com a API key redigida, então um
    `#[derive(Debug)]` numa struct que guarda um driver não consegue
    imprimi-la.
- **Testes de contrato de wire para o Pinecone.** Os testes de
  integração ao vivo precisam de uma `PINECONE_API_KEY` e por isso não
  conseguem rodar no gate - o que deixava os nomes de campo de uma
  reescrita REST (`topK`, `includeMetadata`, `vectorCount`) apoiados
  em nada. Treze testes agora dirigem o driver contra um fake
  `wiremock` local e verificam o método, path, headers e corpo JSON
  exatos que ele coloca na rede, mais que um não-2xx nunca é
  decodificado como resultado, e que uma mensagem de erro nunca
  carrega a API key. Eles fixam o driver ao contrato *documentado* do
  Pinecone; só os testes marcados `#[ignore]` conseguem confirmar que
  a documentação bate com o serviço ao vivo.

## 0.7.2 - 2026-07-28

### Corrigido

- **`generate-types` resolve structs de prop aninhados sem derives.** O
  gerador da 0.7.1 degradava para `unknown` todo campo de prop cujo
  tipo não derivasse `InertiaProps`/`Data` - então rodar de novo o
  gerador (ou o watcher do `suprnova serve`) sobre um projeto com um
  arquivo de types commitado substituía interfaces reais como
  `Array<AdminArticleRow>` por `unknown` e quebrava a checagem de
  tipos em todo o app. Structs simples definidos em qualquer lugar em
  `src/` agora resolvem para suas interfaces reais, transitivamente a
  partir das raízes de prop; `unknown` (com um aviso) fica reservado
  para tipos que o projeto genuinamente não define - tipos de crates
  externos, enums, tuple structs.

### Alterado

- **A geração de `routes.ts` é opt-in.** `generate-types` não deposita
  mais `frontend/src/types/routes.ts` em todo projeto sem ser pedido;
  passe `--routes` para gerá-lo.

- **Dependências dos starters de frontend atualizadas.** Scaffolds
  novos de `suprnova new` agora fixam versões atuais: Vite ^8.1.5,
  Tailwind CSS ^4.3.3, Svelte ^5.56.8 (vite-plugin-svelte ^7.2.0,
  svelte-check ^4.7.4), React ^19.2.8 (plugin-react ^6.0.4), Vue
  ^3.5.40 (plugin-vue ^6.0.8, vue-tsc ^3.3.8), e `@types/node` ^24 (a
  linha de types do Node 24 LTS). O TypeScript fica em ^6.0.3
  deliberadamente: é o mais recente da linha 6.x, e o range de peer do
  svelte-check (`^5 || ^6`) ainda não admite TypeScript 7. Os três
  starters foram verificados de ponta a ponta (`npm install` +
  `npm run build`) contra o conjunto atualizado.

## 0.7.1 - 2026-07-27

Uma passada de correção de defeitos sobre o roteamento de fila da
0.7.0, a partir de uma revisão completa pós-release.

### Corrigido

- **Jobs encadeados não perdem mais sua fila declarada.** `ChainLink`
  capturava o `max_tries`, `timeout` e `backoff` de um job no momento
  de construção da chain, mas não seu `Job::queue()`, então um job que
  caía na fila declarada quando empurrado diretamente caía em
  `default` quando despachado como parte de uma chain - o nível "job"
  da ordem de resolução rota → job → default sumia silenciosamente
  para chains. A fila declarada agora é capturada no link e resolvida
  exatamente como um push direto. Payloads de chain escritos antes
  deste release decodificam sem alteração (`serde(default)`), e um
  link sem fila declarada serializa byte-idêntico ao que a 0.7.0
  escrevia.
- **Registros de failed-job carregam a fila em que o job morreu.** O
  caminho de dead-letter do worker fixava `queue = "default"` em todo
  registro `FailedJob`, então falhas de um job roteado eram invisíveis
  para um operador filtrando o store de falhas pelo pool que as
  possui. O registro agora carrega a fila do envelope (`default` para
  jobs não roteados).
- **A nota de upgrade da 0.7.0 subestimava a migração de `jobs`.**
  Dizia "workers sem filtro não são afetados e não precisam de
  migração", mas `DatabaseQueueDriver::push` nomeia a coluna `queue`
  em seu `INSERT` esteja o job roteado ou não - um binário 0.7.0
  contra uma tabela não migrada falha **todo push**, filtrado ou não.
  A seção 0.7.0 abaixo e `manual/queues.md` estão corrigidas: no
  driver de banco de dados o `ALTER TABLE` é obrigatório para todo
  deployment, e precisa rodar antes de os binários subirem (binários
  mais antigos listam suas colunas explicitamente, então migrar
  primeiro é seguro).

- **O README não anuncia mais uma macro `#[job]`.** Nenhuma macro
  dessas existe - jobs implementam a trait `Job`. A linha de filas
  agora descreve a superfície real, incluindo o roteamento de fila da
  0.7.0.

### Alterado

- **O caminho de release agora atualiza as referências de versão do
  README.** `bump-workspace-version.py` reescreve a tag de instalação
  fixada do README, o exemplo de modelo de distribuição, e a linha de
  MSRV atomicamente com os manifestos, e um README reformulado que
  para de casar com um padrão falha o release de forma explícita. O README
  vinha anunciando a v0.6.0 desde que a v0.7.0 foi lançada, porque nada
  no caminho de release o tocava.
- **O roteamento de conexão é documentado como só resolução de nome.**
  `Job::connection()` e o campo de conexão de `Queue::route` resolvem
  o *nome* de conexão carregado nos eventos de ciclo de vida
  `JobQueueing` / `JobQueued`; um único driver global de processo
  ainda recebe todo push, então eles não selecionam um driver
  diferente. O rustdoc e `manual/queues.md` antes davam a entender uma
  seleção de driver que não existe. A dimensão de fila não é afetada -
  ela é honrada de ponta a ponta. Drivers por conexão continuam sendo
  trabalho futuro.
- `ChainLink` ganhou um campo público `queue: Option<String>`, o que
  quebra a construção por struct literal de links de chain. Links
  construídos através de `ChainLink::from_job` - o caminho normal -
  não são afetados.

### Atualizando

Vindo de ≤ 0.6.x no driver de fila de banco de dados, aplique a
migração da 0.7.0 abaixo **antes** de subir os binários; ela é
obrigatória para todo deployment nesse driver, não só os que usam
`--queue`. A própria 0.7.1 não precisa de migração nenhuma.

## 0.7.0 - 2026-07-26

### Segurança

- **`ammonia` atualizado para 4.1.4 (RUSTSEC-2026-0213).** Versões até
  a 4.1.3 permitem XSS via tags de animação SVG `animate` e `set`.
  `ammonia` é o sanitizer no fim do pipeline de markdown do Suprnova
  (`comrak` → `syntect` → `ammonia`), então todo app renderizando
  Markdown fornecido por usuário através de `content` estava exposto.
  O aviso foi publicado em 2026-07-21 - depois que a v0.6.5 foi
  lançada - então **todo release até e incluindo a v0.6.5 é afetado**.
  Atualizar o framework é a correção; nenhuma mudança de código de
  aplicação é necessária.

### Adicionado

- **Roteamento de fila.** Jobs podem ser despachados para uma fila e
  conexão específicas, e workers podem ser dedicados a filas
  específicas - a superfície `Queue::route(...)` do Laravel 13,
  tipada. Um job declara sua própria casa com `Job::queue()` /
  `Job::connection()`; um operador a sobrescreve centralmente com
  `Queue::route::<SendInvoice>(Some("redis"), Some("billing"))` em
  `bootstrap::register()`, sem editar o job. A resolução é rota,
  depois job, depois default global, e um campo `None` numa rota
  adia em vez de limpar. `queue:work --queue=billing,default` drena
  só aquelas filas. Jobs não roteados pertencem a `default`, então
  nunca ficam encalhados. Jobs encadeados resolvem rotas por nome, já
  que um link de chain guarda seu job apagado (erased).
- **`QueueDriver::pop_from`.** Pop com filtro, com uma implementação
  padrão que **rejeita** um filtro que não consegue honrar, em vez de
  silenciosamente drenar toda fila - um worker instruído a drenar
  `billing` que silenciosamente drena tudo é indistinguível de um
  deployment funcionando até o pool errado comer os jobs errados. Os
  drivers de memory e database filtram nativamente. Drivers
  personalizados continuam compilando e herdam o padrão explícito.
- **Schema da tabela `jobs` documentado.** `manual/queues.md` agora
  carrega o DDL que `DatabaseQueueDriver` de fato espera, o que antes
  só era descobrível lendo o SQL do driver.
- **Opção `serverHead` do Inertia documentada.** Elementos `<head>`
  dirigidos pelo servidor (Inertia 3.5.0) não precisam de suporte
  nenhum do framework: o cliente os lê de uma prop comum, então
  qualquer handler já pode fornecê-los. Veja
  `manual/frontend-inertia-responses.md`.

### Alterado

- `Envelope` ganhou um campo `queue: Option<String>`. É
  `serde(default)` e pulado quando ausente, então um envelope não
  roteado serializa byte-idêntico ao que versões anteriores
  escreviam - o teste de wire-format congelado passa sem alteração,
  não há bump de `schema_version`, e frotas de versão mista
  interoperam durante um upgrade rolling.
- `WorkerConfig` ganhou um campo `queues: Vec<String>` (vazio = drena
  tudo, o comportamento anterior).
- Removido `ROADMAP.md`. Seus princípios de design vivem em
  `manual/introduction.md`, o acordo de trabalho em
  `manual/contributions.md`, e o material de deployment e scale-out
  em `manual/deployment.md`; as checklists de shipped/planned tinham
  ficado desatualizadas. O ponteiro do `README.md` para ele, para "a
  relação com o upstream", já estava quebrado - essa atribuição vive
  em `LICENSE`.
- Frontends de scaffold agora fixam `@inertiajs/{svelte,react,vue3}`
  em `^3.6.1` (a partir de `^3.4.0`). O intervalo 3.4.0 → 3.6.1 é só
  client-side - auditado contra o changelog upstream e o contrato
  `Page` em `packages/core/src/types.ts`, todo header `X-Inertia-*`
  que o cliente 3.6.1 envia já era tratado.
- `scripts/release.sh` agora publica o próprio release do GitHub, com
  notas tiradas da seção `CHANGELOG.md` da versão. Antes isso era um
  "próximo passo" manual que ficava sendo pulado, motivo pelo qual a
  v0.5.10 e a v0.6.1–v0.6.3 são só-tag e a página de Releases ficou
  numa versão desatualizada. O preflight roda antes do gate, então um
  `gh` ausente ou uma seção de changelog ausente falha em segundos, e
  a publicação é pulada automaticamente a menos que `origin` seja o
  GitHub.

### Atualizando

Tabelas `jobs` existentes no driver de fila de banco de dados
**precisam** adicionar a coluna nova - `push` a nomeia em seu
`INSERT` esteja o job roteado ou não, então uma tabela não migrada
falha todo push. Migre primeiro, depois suba os binários (binários
mais antigos listam suas colunas explicitamente e ignoram a nova,
então essa ordem é segura):

```sql
ALTER TABLE jobs ADD COLUMN queue TEXT NULL;
CREATE INDEX idx_jobs_queue ON jobs(queue);
```

*(Corrigido na 0.7.1 - esta nota originalmente afirmava que
deployments sem filtro não precisavam de migração.)*

## 0.6.5 - 2026-07-21

### Adicionado

- **Checkout avulso hospedado no adaptador Stripe.**
  `Checkout::start_session` com `SessionMode::OneOff` e `price_refs`
  não vazio agora cria uma Checkout Session hospedada
  (`mode=payment`, um line item por price ref,
  `allow_promotion_codes=true`) e retorna
  `SessionPayload::StripeCheckoutRedirect`. O caminho Elements só com
  `amount_hint` fica inalterado; as duas formas são escolhidas por
  solicitação.
- **Suporte a Stripe Managed Payments (merchant-of-record).**
  `StripeProvider::with_managed_payments(true)` - ou
  `STRIPE_MANAGED_PAYMENTS=true` em `from_env()` - envia
  `managed_payments[enabled]=true` na criação de sessão avulsa
  hospedada. Desligado por padrão; o campo é totalmente omitido, então
  contas não cadastradas não são afetadas.
- **`Checkout::session_status`.** Novo método de trait (padrão:
  `PaymentError::NotSupported`) reportando o estado do lado do
  provider de uma sessão como o novo `CheckoutSessionState` neutro
  (`Open` / `Complete { paid, payment_ref, amount_total }` /
  `Expired`). A impl da Stripe mapeia
  `GET /v1/checkout/sessions/{id}`; `payment_ref` carrega o id do
  PaymentIntent da sessão para correlação com a tabela espelho. Esta
  é a primitiva de verificação do lado do servidor para páginas de
  retorno de redirect e varreduras de reconciliação.
- **Trait de capacidade `Promotions`.** `create_promotion_code` cunha
  um código restrito a um cliente, opcionalmente expirável, com
  limite de resgates, a partir de um cupom pré-criado. Consultada via
  o novo `PaymentProvider::as_promotions()` (padrão `None`).
  Implementada para a Stripe (`POST /v1/promotion_codes`) e para o
  mock.
- **Atualizações do `MockPaymentProvider` para o acima.** Registra
  toda solicitação `start_session` (`recorded_sessions()`), roteiriza
  `session_status` por id de sessão (`script_session_status()` -
  sessões conhecidas sem roteiro reportam `Open`, ids desconhecidos
  `NotFound`), e implementa `Promotions` com solicitações registradas
  (`recorded_promotion_requests()`).

## 0.6.4 - 2026-07-17

### Corrigido

- **Agregados Eloquent decodificam de forma consistente entre backends
  de banco de dados.** Expressões geradas de `count`, `sum`, `avg`,
  `min` e `max` agora usam um único alias interno estável de
  resultado. O PostgreSQL não retorna mais zeros falsos ou `None`
  porque seu driver rotula colunas agregadas de forma diferente do
  SQLite, e erros de coluna ausente ou tipo incompatível agora se
  propagam em vez de serem silenciosamente substituídos por um
  default.
- **Exclusões em massa não podem usar expressões de tabela fornecidas
  pelo caller.** O SQL de exclusão executável sempre deriva seu alvo
  do `M::TABLE` estático validado do model. O argumento legado
  público do renderer continua compatível na fonte, mas não consegue
  redirecionar ou injetar o alvo da exclusão.

## 0.6.3 - 2026-07-15

### Adicionado

- **Leituras raw tipadas podem ficar na conexão fixada de uma
  transação.** `Transaction::backend()` expõe o backend ativo e
  `Transaction::query_all(Statement)` executa SQL agregado tipado ou
  personalizado através da transação, preservando a instrumentação
  `QueryExecuted`. Aplicações não precisam mais de uma consulta no
  nível do pool ou de acesso a executor privado quando uma decisão com
  escopo de lock depende de colunas de resultado computadas.

## 0.6.2 - 2026-07-15

### Corrigido

- **Predicados raw vinculados são neutros quanto ao backend.** O
  `filter_raw` e o `where_raw` do Eloquent agora aceitam marcadores de
  bind `?` portáveis em todo backend de banco de dados; a
  renderização do PostgreSQL os rebaseia para posições `$N`
  monotônicas ao longo de predicados anteriores, subconsultas de
  relacionamento, cláusulas HAVING e ramos de UNION. Fragmentos
  numerados existentes do PostgreSQL são normalizados pela sua ordem
  local de marcadores, enquanto estilos misturados e incompatibilidades
  entre contagem de marcadores falham a validação antes do I/O. O
  scanner sensível a SQL preserva pontos de interrogação dentro de
  strings entre aspas, identificadores, comentários e corpos com
  dollar-quoting; `??` emite um operador literal de ponto de
  interrogação num fragmento raw vinculado.

## 0.6.1 - 2026-07-15

### Adicionado

- **Limpeza de sessão supervisionada e observável.**
  `SessionMiddleware::install` usa a cadência configurável de
  `SESSION_GC_INTERVAL` (uma hora por padrão), enquanto
  `session_gc_metrics()` expõe execução, sucesso, falha, linhas
  removidas e timestamps do último resultado, locais ao processo,
  para superfícies de operações protegidas.
- **Touches de sessão deslizante limitados.** `SESSION_TOUCH_INTERVAL`
  controla a cadência mínima de escrita de atividade (cinco minutos
  por padrão) e é limitado à metade do tempo de vida da sessão, para
  que sessões ativas não possam expirar entre touches.

### Corrigido

- **Solicitações sem estado não criam mais sessões duráveis.**
  Solicitações sem um cookie de sessão válido não fazem leitura nem
  escrita no session store, e não recebem cookie de sessão, a menos
  que o tratamento crie estado. Sessões limpas existentes evitam
  upserts incondicionais e churn de cookie, cookies legados migram na
  próxima solicitação, e cookies cujas linhas de apoio expiraram são
  limpos sem recriar sessões vazias.

## 0.6.0 - 2026-07-10

### Adicionado

- **Subsistemas do framework opt-in, com padrões compatíveis com
  versões anteriores.** O storage de filesystem, os drivers de banco
  de dados SQLite/Postgres/MySQL, o driver de vetor do MariaDB, e o
  Web Push agora têm features explícitas do Cargo. Builds padrão
  existentes mantêm todas essas capacidades, enquanto consumidores com
  `default-features = false` podem selecionar zero drivers ou só a
  superfície de storage/banco de dados/vetor/push que usam. A matriz
  de features executável verifica os perfis zero-driver,
  driver-individual, Nation X mínimo, padrão e all-feature.
- **Importação de chave privada VAPID P-256 crua.** `VapidKey::from_bytes`
  aceita um escalar P-256 big-endian de 32 bytes validado, ao lado do
  caminho existente de import/export PKCS#8 PEM.

### Alterado

- **JWTs VAPID são assinados diretamente com P-256.** O Web Push agora
  serializa o header/claims ES256 do RFC 8292 e os assina com `p256`,
  removendo a dependência genérica de JWT, preservando as chaves
  geradas, os round trips de PEM, a codificação de chave pública, e o
  limite de tempo de vida de 24 horas.
- **Atualização de dependências de segurança.** Dependências
  vulneráveis do framework atualizadas, incluindo bcrypt e ammonia, e
  as features ativadas do Comrak estreitadas, mantendo
  retaining syntax highlighting.
- **Rust 1.91.1 é o MSRV do release.** Todo pacote do workspace
  declara o mesmo `rust-version`, Dockerfiles gerados fixam a imagem
  de builder correspondente, e o gate de release completo compila o
  perfil de filesystem suportado com o toolchain exato do Rust
  1.91.1.
- **Fixação de segurança do OpenDAL 0.58.** A feature de filesystem
  fixa o commit `88717391eb72c9839d3f8e79fccad9f22fc3a1b4` de
  `eas4ai/opendal`, um fork mínimo baseado exatamente no
  commit oficial `ae99a3b016e354a1b2bb2baf0c70f9f9e134970a` do Apache
  OpenDAL. O fork muda só as declarações do Reqsign usadas pelo core
  do OpenDAL mais S3, GCS e Azure Blob, para que consumidores
  downstream resolvam o commit oficial `b49cd2996b9d2d9944e84481f8835ff55b188b97`
  do Apache Reqsign e `quick-xml` 0.41.0. Um fork é necessário porque
  os patches de Cargo na raiz de um repositório de dependência não se
  propagam para os consumidores; o grafo publicado, do contrário,
  poderia restaurar o `quick-xml` 0.38/0.40 vulnerável.

### Corrigido

- **Metadados de versão de release atômicos.** O bump de release agora
  atualiza `workspace.package.version` e toda dependência de path
  interna versionada numa única operação validada, coloca no stage
  todo manifesto afetado, e prova um workspace `0.6.0` temporário com
  `cargo check --workspace` antes do release. Versões de release são
  validadas como SemVer 2.0 estrito, incluindo a regra de zero à
  esquerda para prerelease numérico. Smokes descartáveis
  agnósticos-a-versão em remote nu derivam um patch release posterior
  tanto da fonte atual quanto de uma fonte já em `0.6.0`, rejeitam
  árvores de release staged/unstaged/untracked antes do gate, provam
  que a publicação atômica de commit/tag reverte as duas referências
  quando uma tag é rejeitada, e provam a sequência normal de release
  sem tocar no remote real. Versões de release precisam aumentar por
  precedência SemVer, incluindo transições de prerelease. Artefatos de
  build do smoke sempre ficam dentro do seu workspace temporário,
  ignorando qualquer `CARGO_TARGET_DIR` do caller.
- **O rustdoc cobre toda fronteira de feature suportada.** O módulo
  OAuth linka para o `OAuthAuth::complete` público, e a matriz
  executável constrói rustdoc zero-driver, padrão e all-feature sem
  dependências.
- **A validação de stream de filesystem tem escopo de sessão.**
  Writers, listers e copiers de filesystem local resolvem e confinam
  seus paths uma vez antes do primeiro I/O, em vez de uma vez por
  chunk/item, enquanto operações ativadas de close/abort sempre
  alcançam o backend para limpeza. O confinamento existente de
  traversal e symlink continua aplicado para um filesystem confiável;
  checagens de canonicalize-então-open não eliminam corridas contra um
  principal mutando a árvore concorrentemente.

### Segurança

- **O gate de release falha de forma fechada.** `release.sh` delega
  para o gate completo canônico antes de editar manifestos ou criar
  commits/tags; esse gate sempre roda `cargo audit`, trata um binário
  `cargo-audit` ausente como um erro, e para em qualquer falha de
  auditoria. Também constrói e audita um consumidor de filesystem
  downstream isolado, garantindo revisões exatas de fonte do
  OpenDAL/Reqsign e nenhum `quick-xml` abaixo de 0.41. Nenhuma
  exceção de aviso nova foi adicionada.

## 0.5.10 - 2026-07-03

### Corrigido

- **`generate-types` não descarta mais structs autorreferentes.** Um
  struct com um campo que referencia seu próprio tipo (um nó de árvore
  com `children: Vec<Self>`, por exemplo uma view de comentários em
  thread) criava uma self-edge no grafo de dependência de tipos,
  fixando seu in-degree acima de zero, então a ordenação topológica de
  Kahn nunca o emitia - deixando toda interface que o referenciasse
  com um nome de tipo pendurado que falhava no `svelte-check`/`tsc`.
  Self-edges agora são removidas antes da ordenação, e quaisquer
  structs presos num ciclo de referência (recursão mútua) são
  emitidos em ordem arbitrária em vez de descartados, já que
  interfaces TS podem referenciar umas às outras independentemente da
  ordem de declaração.

## 0.5.9 - 2026-07-01

### Adicionado

- **`MAIL_FROM_NAME` - nome de exibição opcional nos e-mails de
  auth-flow.** Os mailables de verificação de e-mail, redefinição de
  senha e senha alterada agora renderizam seu header `From` como
  `"Name <address>"` quando `MAIL_FROM_NAME` está definido (lido no
  momento do envio, então sobrevive ao round-trip de serde da fila).
  `MAIL_FROM` continua sendo um endereço puro; deixar `MAIL_FROM_NAME`
  sem definir ou em branco mantém o comportamento anterior de
  endereço puro. Nenhuma mudança em nenhum call site - os próprios
  mailables leem a env var.

## 0.5.8 - 2026-06-30

### Corrigido

- **Os route helpers do `generate-types` agora são sempre TypeScript
  válido.** Quando várias rotas num módulo compartilham um handler
  (por exemplo uma whitelist de `static_files::serve` mapeando várias
  URLs de favicon/asset), a primeira mantinha o nome do handler e as
  demais recebiam uma chave derivada do path da rota - mas o path era
  só parcialmente sanitizado (`/ { } -` → `_`), então uma extensão de
  arquivo vazava um `.` para dentro da chave: `favicon_16x16.png:
  (...) => ...`. Isso é acesso a membro, não um nome de propriedade,
  então `tsc`/`svelte-check` rejeitavam o `routes.ts` gerado. Chaves
  derivadas agora são sanitizadas para identificadores válidos - todo
  caractere não alfanumérico vira `_` e um dígito inicial recebe um
  prefixo - então `favicon-16x16.png` → `favicon_16x16_png` e
  `2fa.json` → `_2fa_json`. Nomes de handler únicos ficam intocados.

## 0.5.7 - 2026-06-30

### Corrigido

- **`generate-types` não emite mais referências de tipo penduradas.**
  Um campo de prop cujo tipo é um struct que não deriva
  `InertiaProps`/`Data` (ou um tipo externo que o gerador não
  consegue ver) era emitido como um identificador solto - por exemplo
  `user: UserInfo` - produzindo TypeScript que falha no
  `tsc`/`svelte-check` porque essa interface nunca é escrita. Tais
  referências agora degradam para `unknown` (`user: unknown`;
  `Vec<T>` → `Array<unknown>`; `Option<T>` → `unknown | null`), então
  a saída gerada sempre passa na checagem de tipos, e `generate-types`
  imprime um aviso nomeando o tipo não resolvido e o campo que o
  referencia, com a correção (derivar `InertiaProps`/`Data` nele).
  Parâmetros genéricos e tipos InertiaProps/Data aninhados resolvidos
  não são afetados.

## 0.5.6 - 2026-06-29

### Alterado

- **Sign in with Apple: verificação JWKS RS256.** Bump do
  `suprnova-apple-rs` para v0.3.1 - tokens de ID da Apple agora são
  verificados contra o JWKS publicado da Apple (RS256) em vez de
  confiados estruturalmente.

## 0.5.5 - 2026-06-28

### Adicionado

- **Propósito de token `MagicLink`.** Nova variante `MagicLink` no
  enum `TokenPurpose` de auth-flow, para tokens de login sem senha por
  magic link.

## 0.5.4 - 2026-06-28

### Alterado

- **Conclusão de OAuth componível.** Divide a conclusão genérica de
  OAuth em `verify_oauth_identity` (verifica + resolve a identidade) e
  um `complete` fino, para que apps consigam verificar uma identidade
  OAuth sem disparar todos os efeitos colaterais da conclusão de
  sessão.

## 0.5.3 - 2026-06-28

### Corrigido

- **Metadados de versão de workspace corrigidos.** A v0.5.2 foi
  taggeada e enviada antes de o bump de versão do seu `Cargo.toml` ser
  colocado no stage, então a tag v0.5.2 enviada ainda lê `version =
  "0.5.1"`. A v0.5.3 recorta o release com a versão de workspace
  correta - nenhuma mudança de código (a divisão de OAuth da v0.5.2
  não é afetada).

## 0.5.2 - 2026-06-28

### Alterado

- **Conclusão de Apple componível.** Divide a conclusão do Sign-In da
  Apple em `verify_apple_identity` + um `complete_apple` fino,
  espelhando a divisão genérica de OAuth. (Nota: a tag v0.5.2 enviada
  carrega um campo de versão `0.5.1` desatualizado - corrigido na
  v0.5.3.)

## 0.5.1 - 2026-06-28

### Alterado

- **Crate da Apple renomeado.** Reaponta a dependência da Apple para o
  repositório renomeado `suprnova-apple-rs`.

## 0.5.0 - 2026-06-28

### Adicionado

- **Sign in with Apple.** Troca de token OAuth + verificação de ID
  token + upsert de usuário para a Apple; endpoints well-known da
  Apple e o modo de resposta `form_post`; campos específicos da Apple
  em `OAuthProviderConfig`; `AppleKeyPair` reexportado para que apps
  configurem o Sign-In with Apple sem uma dependência direta de
  `apple`.

### Corrigido

- Omite parâmetros PKCE da URL de authorize da Apple (a Apple rejeita
  a solicitação quando eles estão presentes).

### Dependências

- Consome a correção de magic-auth do `torii`; adiciona `apple-rs`
  v0.3.0.

## 0.4.1 - 2026-06-26

### Desempenho

- Pré-dimensiona `MiddlewareChain` para eliminar realocações de `Vec`
  por solicitação.

### Corrigido

- Torna o path do arquivo de manutenção (down-file) à prova de colisão
  sob execuções de teste paralelas.

### Docs

- Compile-checa os exemplos de doc do framework (`ignore` → `no_run`);
  reconcilia as notas de distribuição com as GitHub Releases taggeadas;
  ignora a árvore `docs/` inteira.

## 0.4.0 - 2026-06-22

### Alterado

- **A distribuição é rastreada por git; você não fixa em tags.** Apps
  com scaffold dependem de `suprnova = { git = "…/suprnova.git" }` e
  seguem a branch padrão; puxe atualizações com `cargo update -p
  suprnova`. Versões são publicadas como GitHub Releases taggeadas
  (`v0.4.0`, …) para o changelog, mas `Cargo.lock` já fixa o commit
  exato resolvido - então builds continuam reprodutíveis sem fixar
  `tag` ou `rev` à mão. A documentação de instalação não apresenta
  mais a fixação por commit como o caminho de atualização.

## 0.3.0 - 2026-06-21

### Adicionado

- **Instrumentação de query para leituras Eloquent** - `Builder::get`,
  `Model::find`, `find_many`, e `all` agora emitem `QueryExecuted`,
  então SELECTs de model e queries de eager-load aparecem em
  `DB::listen` e no log de query em memória junto com escritas e
  queries raw. Adiciona o terminal de leitura instrumentado
  `ExecutorChoice::statement_all`.
- **Autorização de rota de recurso** -
  `ResourceRoutes::authorize_resource::<U, R>()` anexa a checagem de
  habilidade convencional a toda rota de recurso gerada, como
  middleware por rota (paridade com o `authorizeResource` do
  Laravel). O mapa ação→habilidade é `index`/`show` → `view`,
  `create`/`store` → `create`, `edit`/`update` → `update`,
  `destroy` → `delete`. Uma única chamada faz gate na superfície
  inteira de sete ações, em vez de depender que todo corpo de
  controlador lembre de um `Gate::authorize`.
- **Hit atômico de rate limit** - `RateLimiter::hit_and_check(key, max,
  decay)` incrementa uma janela fixa e a testa num único round-trip,
  retornando se o bucket agora está acima do seu limite (`i64::MAX`
  significa ilimitado).
- **Helper de comparação em tempo constante** - `constant_time_eq(a,
  b)` (apoiado em subtle) para verificação de assinatura de webhook;
  a documentação de `WebhookHandler::verify` agora exige comparação de
  digest em tempo constante.
- **Cliente Inertia para 3.4.0** - os scaffolds Svelte/React/Vue agora
  fixam `@inertiajs/{svelte,react,vue3}` em `^3.4.0` (a partir de
  `3.1.1`), ganhando os modos `router.poll`, `usePoll` dinâmico,
  `Inertia.once`, a correção de cancelamento do InfiniteScroll, e o
  `onSuccess` aguardado do Form. O servidor já emite a superfície
  completa de objeto de página e headers da 3.4.0 (once-props, a
  família de scroll prepend/deep-merge, `matchPropsOn`, props
  resgatadas/compartilhadas), então isso é um bump de atualidade do
  cliente sem mudança de protocolo.
- **Limite de conexão opcional** - `SERVER_MAX_CONNECTIONS` (e o
  `Server::max_connections(n)` programático) limita conexões
  concorrentemente ativas com um semáforo no accept loop, aplicando
  contrapressão no nível TCP. Sem definir - ou `0` - deixa as conexões
  sem limite (o padrão, inalterado). Um backstop para parear com um
  proxy reverso e `LimitNOFILE`, não um substituto para rate limiting
  upstream.
- **Opção de não seguir redirects** - `RequestBuilder::no_redirects()`
  roteia uma solicitação através de um cliente HTTP que não segue
  redirects, então um `3xx` é retornado como está, em vez de
  perseguido. Use quando a URL da solicitação é influenciada por
  entrada não confiável, para fechar um vetor de SSRF baseado em
  redirect (um endpoint hostil redirecionando para um host interno ou
  de metadados de nuvem). O cliente padrão continua seguindo
  redirects, seguindo a convenção geral de cliente.

### Segurança

- **Rotas de recurso** falham de forma fechada no downcast type-erased
  do registro de autorização em vez de entrar em panic, e negações de
  `authorize_resource` / solicitações não autenticadas são recusadas
  antes do handler rodar.
- **O rate limiter** fecha uma corrida de check-then-hit de janela
  fixa incrementando e comparando atomicamente (`hit_and_check`).
- **O middleware `RateLimited` de fila** agora admite jobs através
  daquele `hit_and_check` atômico, em vez de um par separado de
  `too_many_attempts` + `hit`, então workers concorrentes não
  conseguem mais todos passar na checagem de orçamento antes de
  qualquer um deles incrementar, e super-admitir além de
  `max_attempts`.
- **Validadores de upload** (`mimetypes` / `mime`) fazem content-sniff
  dos bytes enviados em vez de confiar no `Content-Type` fornecido
  pelo cliente.
- **A guarda de path de filesystem** canonicaliza paths para pegar
  traversal por symlink para fora da raiz de storage, além das
  checagens léxicas anteriores de `../` / absoluto / UNC.
- **Auth** fecha um oráculo de timing de login sem senha - uma conta
  casada mas sem senha, recebendo uma senha, agora roda uma
  verificação de custo fixo, tanto no provedor de usuário Eloquent
  quanto no de banco de dados - e `dummy_verify` dirige o hasher
  configurado, então o caminho de usuário não casado é de tempo
  constante.
- **Eloquent** valida identificadores de coluna nos caminhos de
  projeção de `pluck` / `value` / `pluck_keyed` / `sole_value` e
  `sum` / `avg` / `min` / `max`.
- **Pagamentos** - o verificador do provider mock falha de forma
  fechada fora de um ambiente de desenvolvimento, e IPs de origem de
  webhook resolvem através de `TrustedProxiesConfig` (`req.ip()`), em
  vez de um header `X-Forwarded-For` bruto.
- **A guarda de path de filesystem** agora caminha até o ancestral
  *existente* mais próximo quando um alvo de escrita ainda não
  existe, fechando um escape por symlink em que um symlink
  intermediário plantado com um pai imediato ausente escapava da
  guarda.
- **`DB::init_with`** valida o ambiente antes de conectar (casando com
  `DB::init`), então o fallback de SQLite de dev não consegue mais
  inicializar silenciosamente em produção por essa porta de entrada.
- **A entrega de arquivo estático** rejeita dotfiles (`.env`,
  `.git/config`, `.htpasswd`, qualquer segmento começando com `.`),
  não só traversal de `.`/`..`.
- **Webhooks de pagamento** serializam retries concorrentes do mesmo
  evento não processado com um lock `FOR UPDATE` + reverificação, e
  tratam violações de unique na tabela espelho como já-aplicado
  benigno; `payments_subscription_items` ganha um
  `UNIQUE(subscription_id, provider_item_id)`.
- **RBAC** usa por padrão o nome de tipo totalmente qualificado como
  discriminador de model, então dois tipos autenticáveis
  compartilhando um nome de folha não conseguem mais herdar os
  papéis/permissões um do outro.
- **`invalidate_session()`** rotaciona o id de sessão (não só faz
  flush), fechando uma brecha de fixação de sessão; o middleware
  `WithoutOverlapping` de fila libera seu lock de cache mesmo quando o
  job entra em panic.
- **Providers de mail** limitam a leitura do corpo de resposta de erro
  (8 KiB), casando com o cliente de web push, então um endpoint
  hostil não consegue drenar a memória do remetente.
- **O web push** desativa o seguimento de redirect HTTP no cliente
  padrão, então um endpoint de push influenciado por atacante não
  consegue mais redirecionar `3xx` um POST de notificação para um
  host interno ou de metadados de nuvem (SSRF). Um redirect agora
  aparece como um push rejeitado, em vez de uma solicitação seguida
  silenciosamente.
- **O adaptador Stripe** redige o secret de assinatura de webhook no
  `Debug` *e* imprime um placeholder para o `stripe::Client` (que
  carrega a API secret key no seu header de auth), então nenhum
  secret consegue chegar aos logs através de um `{:?}` de
  `StripeProvider`, independentemente do próprio `Debug` do cliente
  upstream.
- **O adaptador Stripe** `from_env` rejeita credenciais
  presentes-mas-em-branco, falhando de forma fechada em vez de
  construir um cliente com um secret HMAC de webhook vazio (e,
  portanto, forjável).
- **A verificação de e-mail OAuth** falha de forma fechada para
  providers não reconhecidos: um payload de userinfo carregando um
  `email`, mas sem flag `email_verified`, não é mais tratado como
  verificado. Um provider desconhecido agora precisa afirmar
  `email_verified: true` ou expor um endpoint de e-mails verificados,
  fechando um vetor de vínculo/takeover de conta para apps que
  chaveiam contas por e-mail. Google (só-`true`-explícito) e GitHub
  (verificado pelo contrato do `/user`) não são afetados.

### Corrigido

- **O eager loading aninhado** (`with(["posts.comments"])`) agora é um
  número constante de queries - o segmento final carrega numa única
  query IN em lote ao longo de todos os pais, em vez de uma query por
  pai (N+1).
- **`where_has`/`where_doesnt_have`** qualificam colunas de closure com
  a tabela alvo, então uma coluna presente tanto no pivot quanto no
  alvo não produz mais um erro de coluna ambígua em relações
  many-to-many.
- **O `delete`/`force_delete`/`touch` de soft-delete e o `persist` de
  factory** honram o roteamento `#[model(connection = "…")]` de um
  model (casando com `restore` e os outros caminhos de escrita) em
  vez de cair de volta no pool primário.
- **O `Maybe::Missing` do JSON:API** usa uma sentinela de wire
  não-colidível, então dados de usuário no formato
  `{"__missing__": true}` não são mais silenciosamente removidos.
- **Notificações enfileiradas** honram `should_send` (veto por canal)
  e `after_sending`, reverificados no worker - antes só o caminho
  síncrono fazia isso.
- **Jobs released** empurram a cópia de retry antes de dar ack no
  original, então um erro transiente de push do driver não descarta
  mais o job.
- **Webhooks de ajuste (reembolso) da Paddle** chaveiam a atualização
  espelho pelo id de transação referenciado e leem os valores de
  `data.totals`, em vez de inserir uma linha de valor zero sob o id do
  ajuste.
- **URLs SQLite** carregando uma query string
  (`sqlite://db.sqlite?mode=rwc`) constroem uma URL de conexão de
  query única válida e um nome de arquivo em disco limpo.
- **HTTP** limita valores `q` de `Accept` a `[0,1]` e aplica o
  `max_body_bytes` de um `FormRequest` mesmo quando o corpo foi
  pré-bufferizado; a config de **WebSocket** rejeita
  `max_missed_pings < 2` (1 fechava toda conexão no seu primeiro
  ping).
- **Cron** usa semântica OR para dia-do-mês e dia-da-semana quando os
  dois são restritos (paridade Vixie/POSIX); `plain_text`/excertos de
  Markdown preservam pontuação espaçada intencional; `CachedEvaluator`
  limita o crescimento do seu cache; `SupervisorRegistry::start_all`
  não faz mais double-spawn numa segunda chamada; o contêiner de teste
  se recupera no lugar de um lock envenenado.
- **O backoff de restart do supervisor** volta ao piso de 100 ms
  depois de uma execução que fica de pé por pelo menos o teto de 60
  s, então um daemon que rodou saudável por um longo período e depois
  sai reinicia prontamente, em vez de herdar um backoff que subiu
  durante uma rajada de falhas anterior. Um crash loop cujas execuções
  nunca alcançam o limiar continua subindo até o teto, então o reset
  nunca mascara um supervisor instável.
- Corrigida documentação desatualizada sobre `filter_op` (operadores
  são validados por allowlist), URLs assinadas (não compatíveis byte a
  byte com as assinaturas absolutas padrão do Laravel),
  `UniqueIdKind::is_valid` (um helper de caller, não conectado
  automaticamente em `find`), e o limite de tamanho de identificador
  (128, não 64).

### Documentação

- Documentada a autorização de rota de recurso (`authorize_resource`)
  nos capítulos de roteamento e autorização, e o contador atômico
  `hit_and_check` no capítulo de rate limiting.

## 0.2.0 - 2026-06-21

Adiciona controle de acesso baseado em papéis, um pipeline de
renderização de conteúdo Markdown / docs, e entrega nativa de
arquivo estático.

### Adicionado

- **RBAC Tier-2** - trait `HasRoles`; papéis + permissões com um join
  `role_has_permissions`; `PermissionMiddleware` / `RoleMiddleware`
  (os dois fail-closed / default-deny); a migração
  `CreateRbacTables`; e os helpers `create_role` /
  `create_permission` / `give_permission_to_role`.
- **Renderização de conteúdo** - renderização de Markdown e um
  pipeline de build de docs: `MarkdownRenderer`, `build_docs`,
  `DocsCatalog` / `DocsChapter`, extração de heading e
  `slugify_heading`. O HTML renderizado é sanitizado
  (comrak + syntect + ammonia).
- **Entrega nativa de arquivo estático** - handler de fallback
  `StaticFiles::public()` para servir um diretório `public/` na raiz
  web, substituindo controladores de whitelist por asset feitos à mão
  em apps.

### Corrigido

- Apps recém-gerados herdam uma fixação de compatibilidade `time =
  0.3.47` no nível do framework, evitando conflitos de coerência do
  Rust 1.96 vindos de `time 0.3.48` em resoluções de dependência de
  scaffold recém-criado.

### Documentação

- Documentados os dois starter kits lançados - **Nebula** (auth nível
  Breeze) e **Pulsar** (site de produto + comunidade) - ao longo do
  manual, README e roadmap; roadmap reestruturado em torno da
  superfície lançada; e referências de versão reconciliadas ao longo
  da documentação.

## 0.1.0 - 2026-06-10

O release inicial do Suprnova. Suprnova é um framework web para Rust
inspirado no Laravel, feito como fork do Kit e levado numa direção
própria. O alvo de paridade de hoje é o Laravel 13.x.

Este release usa o modelo de distribuição por git: consumidores do
framework dependem de
`suprnova = { git = "https://github.com/eas4ai/suprnova.git" }`,
e a CLI se instala com `cargo install --git`.

### Adicionado

#### HTTP, roteamento e middleware

- `Router` com grupos de rota, prefixos, restrições de parâmetro, rotas
  nomeadas
- Registro de rota validado em tempo de compilação via a macro
  `routes!`
- Roteamento de recurso (`Router::resource`) produzindo as sete rotas
  padrão
- URLs assinadas (funções livres `url::signed_route` /
  `url::temporary_signed_route`, mais `Redirect::signed_route` /
  `Redirect::temporary_signed_route`)
- Helpers de redirect - `Redirect::to`, `Redirect::back`,
  `Redirect::route`, `Redirect::with_input`, `Redirect::with_errors`,
  `with_flash`
- Trait de middleware com camadas globais, de grupo e por rota
- Middleware embutido - CORS, CSRF, sessão, timeout de solicitação, ID
  de solicitação, throttle / throttle de login, verificação de URL
  assinada, autenticado, e-mail verificado, força bruta
- Helpers de abort (`abort`, `abort_unless`, `abort_if`)
- `suprnova::handle_request(...)` - adaptador público para servir uma
  única solicitação hyper contra um router + chain de middleware

#### Ponte de frontend Inertia.js

- `#[derive(InertiaProps)]` com emissão de tipos TypeScript
- Macro `inertia_response!` com validação de componente em tempo de
  compilação
- Três frontends starter de primeira classe - **Svelte 5** (com
  runes), **React 19**, **Vue 3.5** - todos sobre Inertia 3.1.1 + Vite
  8 + Tailwind v4
- Reloads parciais (`only` / `except`), props diferidas, layout
  persistente, histórico criptografado, preservação de scroll
- `Inertia::paginate(component, key, paginator)` para conexão de
  paginador → prop Inertia

#### ORM estilo Eloquent (sobre o SeaORM)

- Macro de atributo `#[suprnova::model]` que emite uma entity SeaORM e
  o struct Eloquent voltado ao usuário em uma só tacada
- Trait `Model` completa - `create`, `find`, `find_or_fail`,
  `find_many`, `all`, `query`, `save`, `update`, `delete`,
  `force_delete`, `refresh`, `fresh`, `replicate`, `replicate_into`,
  `increment`/`decrement`, `destroy`, `is`/`is_not`, `to_array`/`to_json`
- Mass-assignment fillable / guarded com envelope `Attrs`
- 22 casts de attribute - booleanos, inteiros, floats, datas, enums,
  hashed, encrypted, JSON, coleções, dinheiro, datetime com fuso
  horário
- Acessadores / mutadores via `#[suprnova::model]`
- Timestamps automáticos (`created_at`, `updated_at`)
- Soft deletes (`deleted_at`) com `force_delete`, `restore`,
  `trashed`, `only_trashed`, `with_trashed`
- Onze tipos de relação - `HasOne`, `HasMany`, `BelongsTo`,
  `BelongsToMany`, `HasOneThrough`, `HasManyThrough`, `MorphOne`,
  `MorphMany`, `MorphTo`, `MorphToMany`, `MorphedByMany`
- Enums de morph por família + registro de morph com rotação de
  `APP_KEY_PREVIOUS`
- Eager loading via `.with(...)`, `.with_count(...)`,
  `.load_missing(...)`
- Motor EXISTS correlacionado para `has` / `where_has`
- Dezesseis eventos de ciclo de vida (retrieving, retrieved, creating,
  created, updating, updated, saving, saved, deleting, deleted,
  restoring, restored, force-deleting, force-deleted, replicating,
  trashed)
- Trait `Observer<M>` com auto-registro por método via inventory
- Scopes locais via `#[scopes(M)]`, scopes globais via `GlobalScope`
- Superfície `Collection<M>` do Laravel - `pluck`, `key_by`,
  `group_by`, `where_in`, `first_where`, `contains_where`,
  `partition`, etc.
- Três paginadores - `paginate` (length-aware), `simple_paginate`,
  `cursor_paginate` - todos serializando para JSON no formato Laravel
- `chunk` / `lazy` / `cursor` para iteração de linhas em massa sem OOM
- Locking a nível de linha `lock_for_update` / `shared_lock`
- Construtor de consultas `DB::table(...)` com `DynamicRow` para
  queries ad-hoc
- `DB::transaction(...)` com savepoints, retry em deadlock, split de
  leitura/escrita multi-conexão
- `DB::listen(...)` + eventos `QueryExecuted` / `TransactionBegan` /
  `TransactionCommitted` / `TransactionRolledBack`
- Trait `Prunable` + comando de console `model:prune`
- Métodos helper de query `dump` / `dd`
- `#[model(unique_id="...")]` para chaves primárias UUID / ULID

#### Autenticação

- Trait `Authenticatable` + `EloquentUserProvider<M>`
- `Auth::attempt`, `Auth::login`, `Auth::user`, `Auth::user_or_fail`,
  `Auth::user_as<T>`, `Auth::logout`, `Auth::check`
- Múltiplos guards nomeados (sessão web, token de API)
- Fluxo de verificação de e-mail - `EmailVerification`,
  `EnsureEmailVerifiedMiddleware`, URLs de verificação assinadas,
  `EmailVerificationMail`
- Fluxo de redefinição de senha - `PasswordReset`, tokens com
  throttle, `PasswordChangedMail`, evento `PasswordResetLinkSent`
- TOTP de dois fatores - cadastro, verificação, códigos de
  recuperação, proteção contra replay
- Força bruta / throttle de login - chaveado por IP + identificador,
  `LoginThrottleMiddleware`
- Cookies remember-me com tokens opacos estáveis
- Seis eventos de auth - `LoginAttempted`, `LoggedIn`,
  `Authenticated`, `LoggedOut`, `PasswordResetLinkSent`,
  `EmailVerified`
- Sessões de navegador apoiadas no fork do Torii em
  `github.com/eas4ai/suprnova-torii-rs`

#### Autorização

- Facade `Gate` - `define`, `allows`, `denies`, `authorize`, `any`,
  `none`, `check` (variantes síncrona + assíncrona)
- Macro `#[policy(Model)]` para registro de policy
- Auto-autorização de rota de recurso

#### Pagamentos

- Superfície de cinco traits agnóstica a provider - `Checkout`,
  `Payment`, `Subscription`, `CustomerStore`, `WebhookHandler`
- Trait guarda-chuva `PaymentProvider` + consulta de capacidade via
  `as_payment()`
- Espelho no banco de dados - `customers`, `subscriptions`,
  `subscription_items`, `payments`, `refunds`,
  `payment_webhook_events` (UNIQUE para idempotência)
- Enum `SessionPayload` marcado por fluxo (avulso vs assinatura)
- Dois adaptadores de referência como crates do workspace -
  `suprnova-payments-stripe` (gateway, impl completa de `Payment`),
  `suprnova-payments-paddle` (Merchant of Record, sem impl de
  `Payment`)
- Provider mock para testes

#### Fila, jobs, batches, chains

- Trait `Job` - `handle`, `max_tries`, `backoff`, `timeout`,
  `fail_on_timeout`
- `Queue::push`, `Queue::push_later`, `Queue::push_unique`,
  `Queue::push_unique_later`
- Drivers - `sync`, `null`, `redis`, `database`
- Trait `JobMiddleware` - seis middleware embutidos
- Batches e chains - `Queue::batch(jobs).dispatch()`, construtor
  fluente de chain, cancelamento, rastreamento de progresso
- Armazenamento de failed-jobs com replay
- Worker com shutdown gracioso, concorrência configurável,
  recuperação de panic via `catch_unwind`, métricas de liquidação
- Doze eventos de fila cobrindo enfileiramento, processamento, falha,
  release, ciclo de vida do worker

#### Transmissão e WebSockets

- Macro `ws!()` + `Router::ws` para endpoints WebSocket tipados
- Split Sink/Stream de `WsSocket`
- Supervisors com auto-restart via trait `Supervisor`
- `BroadcastHub` com canais `Channel`, `Private`, `Presence`
- Protocolo de envelope JSON, presence join/leave/here, TTL de
  presence configurável com recuperação de crash
- Ponte `Broadcastable` para o `EventDispatcher`
- Heartbeat de close-on-no-pong com drain de WS_TASKS configurável
- Middleware de WebSocket por rota
- Padrões mais seguros de 1 MiB / 64 KiB + factory
  `WsConfig::generous()`
- Política de origem + close 1011 em violação de protocolo

#### Notificações e correio

- Trait `Notification` + `Notify::send(recipient, notification).await`
- Mailable + renderização de template Markdown
- Canais de banco de dados / mail / broadcast / web-push
- Assinatura VAPID + criptografia de payload ECE do RFC 8291 (via
  `suprnova-web-push`)
- Validação de subject VAPID, parsing de retry-after, cap de 8 KiB no
  corpo de rejeição
- Trait Notifiable para tipagem de destinatário

#### Eventos

- Dispatcher de evento tipado - `EventFacade::dispatch`,
  `EventFacade::listen<E, L>`, `EventFacade::forget`
- Eventos saving/updating canceláveis (retornam
  `EventResult::cancel`)
- Listeners enfileiráveis

#### Sistema de arquivos

- `Storage::disk("name")` com suporte multi-driver - local, S3, Azure,
  GCS via OpenDAL
- Mover, copiar, existência, tamanho, mime, última modificação,
  prepend/append
- Uploads e downloads via streaming

#### Cache

- `Cache::store("name")` + registro de driver
- Drivers - memory, redis (com connect-timeout limitado), database,
  file
- `remember`, `forever`, `tags`, incremento/decremento atômico, locks

#### Banco de dados vetorial

- Trait `VectorDriver` com quatro drivers - in-memory, Qdrant
  (mapeamento de ID via UUID-5), Pinecone (IDs string nativos),
  MariaDB nativo `VECTOR(N)` + índices HNSW (11.7+)
- Distância cosseno / produto interno / euclidiana

#### Binário do console e CLI

- Binário `console` por projeto - análogo Rust do `php artisan`, roda
  comandos definidos pelo usuário via `#[suprnova::console::command]`
- `#[derive(Command)]` para argumentos tipados
- CLI `suprnova` - `new`, `serve`, `migrate`, `db:sync`,
  `generate-types`, `key:generate`,
  `make:{controller,middleware,action,error,inertia,migration,task,command}`,
  `db:seed`, `model:prune`
- Flag `--version`
- Templates de scaffold para starters de backend + API nos três
  frontends

#### Sinalizadores de recursos

- `DatabaseEvaluator` com carregamento de snapshot
- `CachedEvaluator` com TTL
- Extractor `FeatureMiddleware`
- Superfície CRUD de admin
- Trait `FeatureSync` para propagação sub-segundo entre processos

#### Agendamento

- Parser de expressão cron
- `Schedule::task(...)` com predicados componíveis
- Locks de servidor único, prevenção de overlap, rastreamento de
  dispatch
- Comando de console `schedule:run`

#### Validação

- Integração com `validator` 0.20
- Macros `#[request]` + `#[derive(FormRequest)]`
- Cap de tamanho por formulário `#[form_request(max_body_bytes = N)]`
- `#[form_request(custom_hooks)]` para opt-out num `impl FormRequest`
  escrito pelo usuário
- Hooks de ciclo de vida - `authorize`, `after_validation`,
  `after_validation_async`

#### Drivers de banco de dados

- Suporte apoiado em SeaORM para SQLite, Postgres, MySQL, MariaDB
- Detecção de driver baseada em URL
- Sistema de migração + `migrate`, `migrate:rollback`,
  `migrate:status`, `migrate:fresh`, `migrate:refresh`

#### Cliente HTTP

- Facade `Http` - `get` / `post` / `put` / `patch` / `delete`
  retornando um `RequestBuilder`; `.send().await` produz um
  `ClientResponse`
- TLS rustls, timeout padrão de 30s, user-agent `suprnova/<version>`
- Métodos encadeáveis `json` / `form` / `body` / `header` /
  `bearer_token` / `basic_auth` / `timeout`
- `RequestBuilder::retry(max_attempts, base_backoff)` - backoff
  exponencial para falhas transientes e 5xx; respeita `Retry-After`
- Guarda de teste `Http::fake(|| async { ... }).await` com
  `fake_response(method, url_substring, status, body)` +
  `assert_sent` / `assert_not_sent`

#### Criptografia

- Facade estática `Crypt` + `EncryptionKey` (`crypto::*`);
  AES-256-GCM com nonces aleatórios de 12 bytes
- `encrypt_string` / `decrypt_string` / `encrypt<T>` / `decrypt<T>`
- Vinculação AAD `CryptPurpose` prevenindo replay cross-protocol
- Rotação de `APP_KEY_PREVIOUS`
- Comando de CLI `suprnova key:generate` para cunhar chaves novas

#### Testes

- Macro de teste assíncrono `#[suprnova_test]`
- `TestDatabase::fresh::<Migrator>()` com instâncias seguras para
  paralelismo
- `TestContainer::bind` para mocks por teste
- Helpers de teste HTTP - `Test::get`, `Test::post`, JSON / form /
  multipart
- Fakes de Queue / Mail / Notification / Event
- `assert_emitted`, `assert_dispatched`, `assert_dispatched_times`

### Alterado

- Os fluxos de verificação de auth e redefinição de senha agora operam
  através do provedor de usuário configurado, em vez de internals do
  Torii.
- Apps gerados precisam implementar `get_auth_password`; exemplos com
  scaffold agora falham de forma explícita, em vez de deixar o login
  sempre falhar silenciosamente.
- O gate de release local está conectado em `scripts/release.sh`, e o
  repositório inclui um hook pre-push obrigatório para fmt, clippy,
  testes, docs e builds de feature.
- A documentação de porta de dev com scaffold se mudou para os
  padrões atuais de backend/frontend (`8765` / `5765`), com `dev:tls`
  e `--with-portless` documentados.
- `MAIL_FROM` é validado antes de tokens de verificação ou redefinição
  serem emitidos, evitando linhas órfãs de auth-flow quando a
  configuração de mail é inválida.

### Corrigido

- Drift do template de scaffold do React em relação ao starter
  lançado.
- Grupos de rota raiz não geram mais paths `//` duplicados.
- Redirects de path literal agora despacham pelo caminho de
  roteamento pretendido.
- Testes de fanout de transmissão agora tratam resultados de `track`
  / `untrack`.
- O driver de log de mail emite o corpo de texto renderizado, então
  links de verificação e redefinição de senha aparecem nos logs de
  desenvolvimento local.
- A cobertura de redefinição de senha fixa o comportamento de
  revogação de sessão e remember-me.

### Notas

- **Modelo de distribuição**: baseado em git de ponta a ponta.
  `suprnova = { git = "https://github.com/eas4ai/suprnova.git" }`;
  CLI via `cargo install --git`. Nada é publicado no crates.io.
