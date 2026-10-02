# Laravel Parity Map

The honest, feature-by-feature mapping between Laravel 13.x and Suprnova.
Use this when you're asking "does Suprnova have X?" and want a yes/no/where
answer in one row.

Sections mirror the Laravel docs index so a Laravel developer can scan
top-to-bottom. Within each section the columns are always the same:

| Laravel | Suprnova | Status | Notes / link |
|---|---|---|---|

The **Status** column uses four values:

| Symbol | Meaning |
|---|---|
| **shipped** | Same surface, same behaviour (often same method names) |
| **diverged** | Same job, different shape because Rust makes a better choice possible |
| **not built** | No first-party version; the Notes column gives the route |
| **by design no** | Won't ship - explanation in the Notes column |

The relevant chapter (where one exists) is linked from the **Notes** column.

Suprnova ships every Laravel 13.x surface across the 30 documented
domains. The rows marked **not built** and **by design no** are the gaps.

## Architecture concepts

| Laravel | Suprnova | Status | Notes / link |
|---|---|---|---|
| Request Lifecycle | `Application` → `Server` → `handle_request` chain | shipped | [Lifecycle](lifecycle.md) |
| Service Container | `Container` + `App` facade, three-layer (task / thread / global) | diverged | The task-local and thread-local layers hold test overrides, and the global layer holds the bindings of the application. A binding that lives for one request or other unit of work is a scoped binding, `App::scoped`. [Container](container.md), [Scoped bindings](container.md#appscoped-and-appbind_scoped---one-value-per-unit-of-work) |
| Contextual binding (`when()->needs()->give()`) | No contextual bindings - one binding per trait per container layer | by design no | The container is `TypeId`-keyed with no runtime reflection to key a binding on "who is asking". Compose explicitly: pass the dependency in, or bind a distinct newtype per consumer. [Container](container.md) |
| Service Providers | `bootstrap()` function + `#[service]`, `#[policy]`, `#[command]`, observer macros | diverged | No registration class - bootstrap is one function; macros use `inventory` for compile-time registration. [Bootstrap](bootstrap.md) |
| Facades | Static `App::get`, `Cache::*`, `Mail::*`, `Auth::*`, `Storage::*`, `Queue::*`, `Bus::*`, `EventFacade::*`, `Notify::*`, `Gate::*`, `DB::*`, `Vector::*`; the scheduler is a `Schedule` instance (`schedule.task(...)`) | shipped | Same call shape; the facades are real types, not aliases |
| Contracts | Traits - `MailTransport`, `CacheStore`, `Hasher`, `Channel`, `VectorDriver`, `Evaluator`, `PaymentProvider`, etc. | shipped | All public seams live on traits; bind by trait, swap implementations freely |

## Getting started

| Laravel | Suprnova | Status | Notes / link |
|---|---|---|---|
| Installation | `cargo install --git …suprnova-cli` then `suprnova new <name>` | shipped | [Installation](installation.md) |
| Configuration | Typed config structs registered via `Config::register` | diverged | Compile-time typed instead of array bags. [Configuration](configuration.md) |
| Agentic Development (AI) | No first-class AI SDK in framework | by design no | Use the crates you'd use anyway (`async-openai`, `anthropic-rs`, `tokenizers`, etc.) under `App::bind(Arc<dyn YourLlm>)` |
| Directory Structure | `src/{actions,bootstrap,controllers,middleware,models,routes}` | shipped | Same intent, Rust-idiomatic layout. [Structure](structure.md) |
| Frontend | Inertia v3 over Svelte 5 / React 19 / Vue 3.5 | shipped | [Frontend](frontend.md), [Pages](frontend-pages.md), [TS Types](frontend-typescript-types.md) |
| Starter Kits | **Nebula** (auth), **Pulsar** (full product site) and the **Directory starter** (listings and paid publication), plus the plain `suprnova new` scaffold | shipped | Nebula is the Breeze equivalent; Pulsar adds docs, blog, community, and RBAC; the Directory starter adds listings, moderation, and paid publication. [Starter Kits](starter-kits.md) |
| Deployment | Single binary; Docker / Railway / DO / Hetzner recipes | diverged | One artifact, not a PHP runtime + opcache + FPM. [Deployment](deployment.md) |

## The basics

| Laravel | Suprnova | Status | Notes / link |
|---|---|---|---|
| Route definitions | `routes!` macro + `get!` / `post!` / `put!` / `patch!` / `delete!` / `any!` / `head!` / `options!` / `fallback!` / `ws!` | shipped | [Routing](routing.md) |
| Route parameters | `{id}` path params + `req.param("id")` | shipped | Optional params via `{id?}`: `/posts/{id?}` matches `/posts` and `/posts/42`, and an optional parameter can only be followed by optional ones. Constraints are methods on the route, not a `where!` macro: `.where_number("id")`, `.where_alpha`, `.where_alpha_numeric`, `.where_uuid`, `.where_ulid`, `.where_in("status", [...])`, `.where_pattern("year", "[0-9]{4}")`. A value the constraint refuses is a 404 and no middleware runs. [Routing](routing.md) |
| Route names | `.name("posts.show")` on the route + `route("posts.show", &[("id", "42")])` | shipped | [URL Generation](urls.md) |
| Route groups | `group!` macro with the path prefix as its first argument + `.middleware()` / `.name("admin.")` on the group, and `controller = path::to::module` inside the macro | shipped | Group middleware is flattened onto each route at register time. `.name("admin.users.")` puts the prefix in front of the name of every route in the group, and a nested group adds its own after it. `controller = ...` names the module the handlers live in, so a route names its handler by function alone. [Routing](routing.md) |
| Resource routes | `Router::resource("posts", PostController)` registers the 7 standard routes | shipped | `Router::api_resource`, `only(...)`, `except(...)` all supported |
| Signed URLs | `sign_url(...)`, `sign_route(...)`, `verify_signature(...)` | shipped | HMAC-SHA256 with `APP_KEY` |
| Route model binding | `#[handler]` extracts `RouteParam<Post>` from `{post}` via `AutoRouteBinding` | shipped | `AutoRouteBinding` has blanket impls for `#[suprnova::model]` types and their SeaORM models |
| Rate limiting | `.middleware_named("throttle:60,1")` on a route + `RateLimiter::r#for` | shipped | Register the alias once with `register_middleware_alias_with_args("throttle", ThrottleRequestsMiddleware::from_alias_args)`. `throttle`, `throttle:60`, `throttle:60,5`, `throttle:60,5,prefix` and `throttle:api` (a named limiter) are read. [Rate Limiting](rate-limiting.md), [Middleware](middleware.md) |
| Middleware | `impl Middleware` trait; register globally or per-route | shipped | [Middleware](middleware.md) |
| Middleware groups + aliases | `register_middleware_group`, `register_middleware_alias`, `register_middleware_alias_with_args` + `.middleware_named("auth")` on a route or group | shipped | A name is looked up when the route is registered, so a name that is not registered stops the boot, not a request. `.try_middleware_named(...)` returns the error. [Middleware](middleware.md) |
| CSRF Protection | `CsrfMiddleware` + `csrf_token()` / `csrf_field()` / `csrf_meta_tag()` | shipped | Per-session token validation is the default. Optional `SameOriginOnly`, `AllowSameSite`, and `OriginOnly` policies consult `Sec-Fetch-Site`; origin enforcement is not enabled by default. [CSRF](csrf.md) |
| Controllers | `#[handler] pub async fn show(req: Request) -> Response` | shipped | Controllers are modules of free functions, not classes. [Controllers](controllers.md) |
| Single-action controllers | A handler is already a single function; group into modules | shipped | The Rust convention - no `__invoke` ceremony |
| Requests | `Request` struct with `.input()`, `.param()`, `.query()`, `.header()`, `.cookie()`, `.json()`, etc.; uploads through `#[derive(MultipartRequest)]` | shipped | [Requests](requests.md) |
| Form Requests | `#[derive(Data, Validate)]` (`Data` implements `FormRequest`) | shipped | Validation runs as you extract |
| File uploads | `#[derive(MultipartRequest)]` fields of type `UploadedFile`; streaming multipart with size + part caps | diverged | Auto-spill to tempfile above threshold |
| Responses | `HttpResponse` builders + `json_response!()` / `text_response!()` / `Redirect::to` / Inertia responses | shipped | [Responses](responses.md) |
| Streamed responses (`eventStream`, `stream`, `streamJson`) | `HttpResponse::sse(...)` / `event_stream(...)` / `stream_bytes(...)` / `stream_json(...)` | shipped | Same wire shapes `@laravel/stream-{react,vue,svelte}`'s hooks expect. [SSE](sse.md) |
| `withoutCookie` / `withoutCookies` | `.without_cookie(name)` / `.without_cookies([...])` on `HttpResponse`, `Response`, `Redirect`, `RedirectRouteBuilder` | shipped | `Cookie::forget_with(name, path, domain)` for a cookie that wasn't set at `/` |
| Views (Blade) | Server-rendered Inertia pages (Svelte/React/Vue), or checked Askama views through `#[view]` + `document_response` | diverged | Inertia is the main view layer. Use [Pages](frontend-pages.md) instead of Blade |
| Asset Bundling (Vite) | Vite 8 ships in every scaffold; `suprnova serve` runs Vite + backend together | shipped | Manifest reading + HMR auto-wired |
| Static assets (`public/`, served by the web server in Laravel) | `StaticFiles::public()` in-process fallback handler serving `public/` at the web root | shipped | `StaticFiles::from_dir(...)` + `cache_control(...)`; no separate web server needed |
| URL Generation | `url::to("/path")`, `route("posts.show", …)`, `redirect(...)`, `redirect_to(...)` | shipped | [URL Generation](urls.md) |
| Session | `session()`, `session_mut()`, flash through `App::flash`, a redirect's `.flash(...)`, or the session's `flash` | shipped | Database-backed by default via `DatabaseSessionDriver`; the encrypted browser cookie carries the session identifier and activity-touch metadata, not the session data bag. [Session](session.md) |
| Cookie queue (`Cookie::queue`) | `Cookie::queue`/`queued`/`unqueue`/`expire` - a task-local jar `SessionMiddleware` drains onto the response | shipped | Requires `SessionMiddleware` in the chain; queued by name, not name+path like Laravel's `CookieJar` |
| Validation | `#[derive(Validate)]` + 35 built-in rules + `Rule`/`ValueRule`/`AsyncRule` traits | shipped | `Url` uses Laravel's scheme allowlist and `Url::protocols([...])` mirrors `url:http,https`. Async rules (e.g. `Unique`) hit the DB. `ArrayKeys`/`Distinct` are `ValueRule`s over `serde_json::Value`, matching Laravel's `array:keys` and `distinct`. `InArray` takes the other field's list directly instead of Laravel's `in_array:other.*` rule string, and `Contains`/`DoesntContain` match JSON string elements exactly. `Gt`/`Gte`/`Lt`/`Lte` take an explicit `CompareWith` operand (literal number, numeric sibling, or sibling compared by character count) instead of inferring Laravel's four size measures; array and file comparisons have no counterpart. [Validation](validation.md) |
| `Password` rule (`Password::defaults()`, `uncompromised()`) | `Password::min(n)` + strength builders (`.letters()`, `.mixed_case()`, `.numbers()`, `.symbols()`) + `.uncompromised()` | shipped | Have I Been Pwned k-anonymity check; fails open on a network error, matching Laravel's `NotPwnedVerifier`. [Validation](validation.md#password-strength) |
| Error Handling | `FrameworkError`, `AppError`, `HttpError` trait, panic boundary in `execute_chain_safely` | shipped | [Error Handling](errors.md), [Error Model](error-model.md) |
| Logging | `tracing` subscriber with structured fields, `LogFormat` (json / pretty) | diverged | In json mode (the production default) one log line is a JSON document; `request_id` always present. [Logging](logging.md) |
| Log channels / file drivers (`single`, `daily`, `monthly`, `stack`) | `tracing` writes structured lines to stdout; the platform rotates and ships them | by design no | Containers, systemd, and every log shipper already do rotation and retention. Re-implementing it in-process duplicates the platform and hides logs from it. [Logging](logging.md) |
| Abort helpers | `abort_if(cond, status, msg)`, `abort_unless(...)`, `abort_with(status, msg)` | shipped | Same shape as Laravel's `abort_if` family |

## Digging deeper

| Laravel | Suprnova | Status | Notes / link |
|---|---|---|---|
| Artisan Console | Per-app `console` binary built from `#[command]` + `#[derive(Command)]` | shipped | [Console](console.md). `cargo run --bin console <subcommand>` |
| Tinker (REPL) | No REPL | by design no | Write a one-off `cargo run --bin xxx` script or a `#[suprnova_test]` |
| Broadcasting | `BroadcastHub` + `Channel` / `PrivateChannel` / `PresenceChannel` + `Broadcastable` | shipped | sea-streamer fanout for multi-node. [Broadcasting](broadcasting.md) |
| Cache | `Cache::get/put/forget/remember/rememberForever/increment/...` + `InMemoryCache`, `RedisCache` | shipped | Atomic ops + tagged cache + cache locks (`LockGuard`). [Cache](cache.md) |
| Collections | `eloquent::Collection<M>` with Laravel-shape methods | shipped | Derefs to a `[M]` slice, so slice idioms work; `into_vec()` for `Vec` ones. [Collections](eloquent-collections.md) |
| Concurrency | Tokio everywhere - `tokio::spawn`, `tokio::join!`, `tokio::select!` | shipped | The whole framework is async. The Laravel `Concurrency::run([...])` facade doesn't ship; Tokio is the answer |
| Context | `Context::add` / `Context::get` / `ContextStore` + a `ContextSnapshot` that travels with queued jobs, queued mail and notifications, and queued event listeners | shipped | The snapshot is taken when the push is made and stored on the queue envelope. The worker runs the job in a scope restored from it, on a copy, so what a job adds reaches neither the request nor the next job. The query bag does not travel. `Context::dehydrating` / `Context::hydrated` register callbacks for the two ends. [Context](context.md) |
| Contracts | All public seams are traits | shipped | See the "Architecture / Contracts" row above |
| Events | `EventFacade::dispatch(e).await?`, `impl Event`, `EventDispatcher`, queued listeners, subscribers | shipped | [Events](events.md) |
| File Storage | `Storage::disk("local"\|"s3"\|"azblob"\|"gcs"\|"memory")` over OpenDAL | shipped | Same `put/get/delete/copy/move/exists` surface. Path-traversal protection built in. `Storage::url(disk, path)` returns the public URL of a file. It takes the disk name, and it returns an error for a disk with no public base URL: set one with `Storage::set_public_url(disk, base)`, or `S3_PUBLIC_URL` for the `s3` disk. The path is percent-encoded and a `.` or `..` segment is refused. On a local disk, `put`/`write`, the streaming writer, and `copy` are all staged under `<root>/.suprnova-atomic/` and published in one step, and a conditional write is published with `link(2)` so it stays a true exclusive create; `append` is the one in-place operation. Laravel's local driver writes straight to the target, where a partial length is observable and a crash truncates the live object. `Storage::register_read_through` composes two disks into a read-through disk that promotes fallback hits onto the primary, with `copy: false` to skip promotion and fallback-spanning `copy` / `rename`. [Filesystem](filesystem.md) |
| Helpers | Equivalents are in their home modules (no kitchen-sink `helpers.md`) | diverged | E.g. URL helpers live in [urls.md](urls.md), string helpers in `std`/`heck`, array helpers in `std::collections` - Rust does this with crates, not a global namespace |
| HTTP Client | `Http::get/post/...` builder + `Http::fake(...)` for tests | shipped | Auto-records requests; `assert_sent` / `assert_not_sent`; `.retry_when(predicate)` narrows the built-in retry policy with a `RetryContext`. [HTTP Client](http-client.md) |
| Image (`Illuminate\Image`) | `Image::from_bytes/from_path/from_disk/from_upload/from_stream` + the same operation and terminal surface | shipped | Lives in `suprnova::media`. Two drivers like Laravel's `gd`/`imagick`: `IMAGE_DRIVER=oxideav` (default, pure Rust) or `magick`. Reads and writes PNG, JPEG, WebP, GIF, BMP; no AVIF. Header-checked decode limits. [Images](images.md) |
| HEIC decoding in the default driver | `IMAGE_DRIVER=magick` on a host with the libheif delegate | by design no | HEVC is patent-encumbered and the only credible pure-Rust decoder is dual AGPL/commercial, so no built-in decoder ships. Same shape as Laravel, where GD cannot read HEIC at all and Imagick needs the delegate compiled into both the binary and the PHP extension. [Images](images.md#why-suprnova-diverges) |
| Localization | `Lang::get` / `get_with` / `try_get` / `has` + the `__!("key", name: value)` macro over Fluent `.ftl` catalogs in `lang/<locale>/`, `LocaleMiddleware` detection, translated validation messages, ICU4X formatting | shipped | The same catalog is served to the browser at `/_suprnova/lang/<locale>.ftl` and typed by `generate-types`. [Localization](localization.md) |
| Mail | `Mail::to(...).send(MyMail { ... }).await?` + drivers `smtp/ses/mailgun/postmark/sendgrid/resend/log/memory/file` | shipped | `Mailable` trait + Tera-rendered HTML/text bodies; SES sends carry `TenantName` / `ConfigurationSetName` / `ListManagementOptions`; queued dispatch routes via `.on_queue(...)` / `.on_connection(...)`, outranking `Queue::route`. [Mail](mail.md) |
| Notifications | `Notify::send(&user, notif).await?` + channels `mail/database/broadcast/webpush` | shipped | `Notifiable` trait + `Notification` per channel; queued dispatch (`Notify::queue`) carries per-notification `queue`/`timeout`/`fail_on_timeout`/`max_tries`/`backoff` onto each channel's job via the same `EnvelopeOverrides` primitive Mail uses. [Notifications](notifications.md), [Web Push](web-push.md) |
| Package Development | Workspace adapter crates (e.g. `suprnova-payments-stripe`) | shipped | Same shape as Laravel packages: depend on the framework, bind into the container, expose macros if needed |
| Processes (running shell commands) | `tokio::process::Command` from the stdlib | by design no | No facade - Tokio's API is already the right shape |
| Queues | `Queue::push(job).await?` + drivers `sync/memory/database/redis/sqs/null`, batches, chains, `JobMiddleware`, `FailedJobStore` | shipped | [Queues](queues.md) |
| SQS queue driver | `QUEUE_DRIVER=sqs` over SQS standard queues, Laravel's `SQS_PREFIX` / `SQS_QUEUE` / `SQS_SUFFIX`, the AWS credential chain, and overflow of large jobs | shipped | A release costs no attempt, delays past 15 minutes hold, overflow goes to a disk rather than a cache store, and FIFO queues are refused at boot. [Queues](queues.md#amazon-sqs) |
| Process facade | `Process::command(args)` and `Process::shell(line)`, results, timeouts and idle timeouts, started processes, pools with a concurrency limit, pipes, and `Process::fake` with patterns, `describe`, sequences and assertions | shipped | The command and the shell line are separate methods; a kill reaches every process the program started; pools take a concurrency limit. [Processes](processes.md) |
| Job-declared delay | `fn delay() -> Option<Duration>` on `Job`, honored by `Queue::push`, `Queue::bulk` and each link of a chain | shipped | A link after the head waits that long after the link before it completes. An explicit `Queue::push_later` / `Queue::later(delay, job)` call always wins over the job's own default. [Queues](queues.md) |
| `Queue::forward` | `Queue::forward(from, to)` / `Queue::forward_on(from, to, connection)`, applied to the envelope and to the worker's `--queue` list | shipped | Queue-to-queue only: `connection` gates the redirect rather than selecting a driver, and is compared against the process connection name so the push and the worker's claim gate on the same value; `to` is required where Laravel's is optional. [Queues](queues.md) |
| Unique-job skipped event | `queue::events::UniqueJobSkipped { job_name, unique_id, connection }` | shipped | Fired on the push side when `push_unique` dedupes; the call still returns `Ok(false)` |
| Queue pausing (`queue:pause` / `queue:resume`) | `Queue::pause`/`resume`/`pause_all`/`resume_all`/`is_paused`/`paused_queues`, cache-backed, with `QueuePaused` / `QueueResumed` / `QueuesPaused` / `QueuesResumed` events | shipped | A per-queue pause only takes effect on a worker started with an explicit `--queue=...` list; `resume_all` doesn't clear a per-queue pause. A running worker also emits `WorkerQueuePaused` / `WorkerQueueResumed` once per transition and `queue:work` prints a line for each; their `queue` field is `Option<String>`, because a worker started without `--queue` has no queue names to report under a global pause. [Queues](queues.md) |
| Retrying transient Redis commands | Read-shaped Redis commands retry once on a connection-level failure, each attempt awaiting the driver's reconnect budget; `REDIS_COMMAND_RETRIES` adds more | shipped | Laravel's `command_retries` covers every command through one dispatch point and a 60-entry allowlist; Suprnova retries per call site, and no setting makes a write or a queue pop retry. [Cache](cache.md) |
| After-commit dispatch (`afterCommit()`) | `fn after_commit() -> bool` on `Job`, `EnvelopeOverrides::after_commit` per push, `Queue::push_after_commit` | shipped | The whole push waits for the commit, events included, and a rollback discards it; a deferred `push_unique` still takes its lock immediately so dedupe works inside the transaction. Manual `DB::begin_transaction` never defers. [Queues](queues.md) |
| Failover queue connection | `FailoverQueueDriver` over an ordered connection list, via `QUEUE_DRIVER=failover` + `QUEUE_FAILOVER_CONNECTIONS` | shipped | Writes fall through the list; workers drain every connection (`pop` rotates across them) and the counters and listings sum all of them. `QueueFailedOver` is edge-triggered, and `bulk_push` falls through per envelope so each keeps its own delay. [Queues](queues.md) |
| `ShouldBeUniqueUntilProcessing` | `fn unique_until_processing() -> bool` on `Job`, released after the middleware pass and before the handler | shipped | Owner-scoped release, so a redelivered attempt never releases a newer dispatch's lock. A job a middleware releases back onto the queue keeps its lock. [Queues](queues.md) |
| Debounced jobs (`#[DebounceFor]`) | `Job::debounce_for` / `max_debounce_wait` / `debounce_id`, plus `Queue::push_debounced(job, DebounceOptions)` | shipped | Trait methods rather than a class attribute, so a typo is a compile error. Every dispatch is enqueued and the collapse is settled at the worker; a job declaring both `debounce_for` and `unique_id` is refused with a `FrameworkError` where Laravel throws. Chains and batches refuse a debounced job outright. [Queues](queues.md) |
| Debounced queued listeners | `Job::debounce_for` on the listener's job, or `DebouncedListener::new(window, build).keyed_by(...)` | shipped | Laravel puts the attribute on the listener class; Suprnova's listener-to-job bridge already runs through `Queue::push`, so declaring it on the job covers the common case and `DebouncedListener` covers a per-registration window. [Events](events.md) |
| Queue inspection (`pendingJobs` / `delayedJobs` / `reservedJobs`) | `Queue::pending_jobs(queue)` / `delayed_jobs` / `reserved_jobs`, `Option<&str>` collapsing Laravel's `all*Jobs()` twin into one call | shipped | `InspectedJob` DTO (`id`/`queue`/`name`/`attempts`/`payload`/`created_at`); the trait default is an honest `Err` rather than an empty collection; `sync`/`null` override with `Ok(vec![])`; Redis's `reserved_jobs` is per-consumer. Unlike Laravel these do not follow a `Queue::forward`, so they report the literal queue you name - which is how a backlog left behind on a forwarded queue stays visible. [Queues](queues.md) |
| Schedule per-task timezone | `.timezone(chrono_tz::Tz)` / `.try_timezone("name")` per task, `Schedule::timezone` default, `schedule:list --timezone` | shipped | Typed `chrono_tz::Tz` instead of Laravel's string; the schedule-wide default is `Schedule::timezone` in `schedule::register` rather than an `app.schedule_timezone` config key, and an unpinned task keeps the process-local zone. [Scheduling](scheduling.md) |
| Rate Limiting | `RateLimiter::r#for(...)` (or `RateLimiter::define(...)`), `ThrottleRequestsMiddleware`, `RateLimitMiddleware` | shipped | Sliding window via `SlidingWindowConfig`. [Rate Limiting](rate-limiting.md) |
| Search (Scout) | No first-party full-text search adapter | not built | Semantic search ships via [Vector](vector.md); for keyword search, use `meilisearch-sdk` / `elasticsearch` directly |
| Strings (helpers) | `heck` crate (case conversions), `std::str`, `regex` | diverged | Same crates the rest of the Rust ecosystem uses; no `Str::camel($x)` global |
| Task Scheduling | `Schedule::call/command/task` + `impl Task` + cron syntax + `schedule:run` worker | shipped | `schedule.command("emails:send --force")` takes a console command line the way it is typed behind the binary name and returns the same `TaskBuilder` as the other forms. The line is checked when the schedule is built, and `try_command` returns the error. The command runs in the scheduler's process. [Scheduling](scheduling.md) |
| Idempotency keys | `Idempotency::remember(key, ttl, body)` - Stripe-style replay protection | shipped | Caller namespaces the key with the route + user / business identity. [Idempotency](idempotency.md) |
| Request timeout | `TimeoutMiddleware` configurable per route | shipped | Rust-native - abort the in-flight future, free the worker. [Timeout](timeout.md) |
| Feature Flags (Pennant) | `Feature` + `Evaluator` + `FeatureMiddleware` + admin CRUD | shipped | Sub-second propagation via `FeatureSync` trait. [Feature Flags](feature-flags.md) |
| Observability (Pulse) | OpenTelemetry via `init_telemetry`, `Metrics`, `tracing` everywhere | diverged | OTel is the lingua franca for Rust observability - point your collector at the binary. [Observability](observability.md) |
| Telescope (debug dashboard) | No first-party dashboard | not built | The framework's tracing + OTel output covers most diagnostic needs |
| Pulse (perf dashboard) | No first-party dashboard | not built | Same as Telescope - surface metrics with your existing observability stack |
| Vector search | `Vector::register(name, driver)` / `Vector::store(name)` over memory, Qdrant, Pinecone or MariaDB drivers | shipped | No "Postgres pgvector only" gatekeeping. [Vector Search](vector.md) |

### Suprnova-exclusive (no Laravel equivalent)

| Suprnova | What it is | Notes / link |
|---|---|---|
| `ws!()` macro + WebSocket handlers | Typed WS routes that share the router + middleware stack | [WebSockets](websockets.md) |
| Workflows | Long-running stateful work with retries, sleep, step boundaries | [Workflows](workflows.md) |
| Supervisors | `Supervisor` trait with panic-catch auto-restart for long-lived tokio tasks | [Supervisors](supervisors.md) |
| Web Push (VAPID) | Browser push notifications as a first-class channel | [Web Push](web-push.md) |
| Multi-connection read/write split | `READ_REPLICA_CONNECTION_NAME` + `DB::on("read").select(...)` | [Database](database.md) |
| HTTP/2 + WebSocket on the same socket | `hyper.with_upgrades()` in `Server::run` | [Lifecycle](lifecycle.md) |
| Markdown content + docs pipeline | `MarkdownRenderer` (sanitised comrak → syntect → ammonia) + `build_docs(DocsBuildConfig)` → searchable `DocsCatalog` of `DocsChapter`s | Heading extraction + `slugify_heading`; powers Markdown docs / blog with no separate static-site generator |

## Security

| Laravel | Suprnova | Status | Notes / link |
|---|---|---|---|
| Authentication | `Auth::user/check/login/logout/attempt`, `Authenticatable` trait, `Guard` per name | shipped | [Authentication](authentication.md) |
| Multiple guards | `Guard` registered by name (`web`, `api`, …) via `AuthManager`; a guard of your own through `Auth::extend(driver, factory)` and `GuardConfig::custom(driver, provider)`, or `Auth::via_request(name, resolver)` for a guard that reads the request | shipped | `SessionGuard`, `TokenGuard`, and the guards of your application. Laravel's `viaRequest` closure gets the request when the guard resolves; here the middleware for the guard runs the resolver, so the guard reports no user outside it. A guard of your application is read-only through the manager, its name has no `:`, and it attests the principal `<guard>:<id>` for Live. [Guards of your application](authentication.md#guards-of-your-application) |
| User providers | `EloquentUserProvider<U>`, `DatabaseUserProvider`, custom via `UserProvider` trait | shipped | [Auth Flows](auth-flows.md) |
| Email Verification | `EmailVerification` + `EnsureEmailVerifiedMiddleware` + `EmailVerificationMail`; `MustVerifyEmail` contract | shipped | Provider-backed and actor-bound - [Auth flows](auth-flows.md) |
| Password Reset | `PasswordReset` + Magnetar first-email-proof transaction or verified `UserProvider` fallback + reset/change mail | shipped | Magnetar handles atomic first proof; provider-backed apps can reset already verified users - [Auth flows](auth-flows.md) |
| Brute-force throttling | Magnetar lockout engine + `BruteForce` + `LoginThrottleMiddleware` | shipped | Account lockout plus framework IP/route limiting |
| Two-Factor (TOTP) | Framework `TwoFactor` compatibility facade plus Magnetar factor engine | shipped | Recovery codes, replay protection, and factor-gated integrated sign-in |
| Remember-me | Magnetar purpose-bound rotating credential behind the framework cookie | shipped | Auth-epoch checks, rotation, anomaly handling, and legacy fallback |
| OAuth (Socialite) | Magnetar provider registry and `Auth::oauth(provider)` facade | shipped | OAuth, Apple `form_post`, PKCE/state binding, verified identity policy - [OAuth](oauth.md) |
| Sanctum (API tokens) | `BearerTokenMiddleware` over Magnetar bearer sessions | diverged | Authenticates bearer sessions; no separate Sanctum token-management API |
| Passport (OAuth server) | Magnetar is an OAuth client, not an authorization server | not built | Run a dedicated IdP (Hydra, Keycloak) behind Suprnova |
| Fortify (auth backend) | Framework `Auth`/`auth_flows` facades over Magnetar engines | shipped | Framework owns HTTP, mail, events, cookies, and application binding |
| Authorization (Policies / Gates) | `Gate::allows/denies` + `#[policy] impl PostPolicy` + `Authorizable` trait + macro registration + `Gate::default_denial_response` | shipped | [Authorization](authorization.md) |
| Roles & permissions (spatie/laravel-permission) | `HasRoles` trait + `roles` / `permissions` / `role_permissions` tables (`CreateRbacTables`) + `RoleMiddleware` / `PermissionMiddleware` (fail-closed) | shipped | First-party, not a community package. `create_role` / `give_permission_to_role` / `assign_role_to_model` helpers. `register_gate_bridge::<User>()` makes a held permission allow the same ability on the gate; it allows and never denies, and it answers the async forms (`allows_async`, `authorize_async`, `inspect_async`) only. [Roles and permissions](authorization.md#roles-and-permissions), [Answering the gate with permissions](authorization.md#answering-the-gate-with-permissions) |
| Encryption | `Crypt::encrypt/decrypt` + `CryptPurpose` AAD binding | shipped | AES-256-GCM, key rotation via `APP_KEY_PREVIOUS`. [Encryption](encryption.md) |
| Hashing | `hash::*` + `BcryptHasher`, `Argon2idHasher`, `Argon2iHasher`, `needs_rehash`, `is_hashed`, `verify` | shipped | Bcrypt default; argon2id available. [Hashing](hashing.md) |

## Database

| Laravel | Suprnova | Status | Notes / link |
|---|---|---|---|
| DB::table('users')->where(...)->get() | `DB::table("users").filter_op("id", "=", 1).get().await?` | shipped | [Database](database.md), [Queries](queries.md) |
| Multiple connections | `DB::named("read")`, `DB::table_on(...)`, a builder's `.on("read")` + `ConnectionRegistry` | shipped | Read/write split first-class |
| Transactions | `DB::transaction(\|tx\| Box::pin(async move { ... })).await?` | shipped | Savepoints + retry-on-deadlock |
| Query events | `QueryListener` + `QueryExecuted` event | shipped | `DB::listen(\|q\| { ... })` |
| Raw expressions | `DB::select("...", &[...])` with bound values | shipped | Pass values as bindings; the SQL string itself is not checked for interpolation |
| Postgres / MySQL / SQLite | All three first-class via SeaORM | shipped | URL detection in `database::config::database_type()` |
| Postgres `keepalives_*` DSN options | `DB_IDLE_TIMEOUT` / `DB_MAX_LIFETIME` / `DB_ACQUIRE_TIMEOUT` / `DB_TEST_BEFORE_ACQUIRE` / `DB_PING_AFTER_IDLE` pool liveness | diverged | sqlx exposes no TCP keepalive setter, so Suprnova recycles and pings pooled connections instead. [Database](database.md#pool-liveness) |
| MariaDB | Through the MySQL driver (`mysql://`); the vector driver is MariaDB-specific | diverged | MariaDB 11.7+ keeps vectors next to relational rows. [Vector Search](vector.md) |
| Redis | Used by drivers (cache/queue/rate-limit) - no separate `Redis::*` facade | diverged | Reach for `redis` crate directly when you need ad-hoc commands; cache/queue/rate-limit cover 95% of typical use |
| MongoDB | No first-party adapter | not built | Use `mongodb` crate directly via `App::bind` |
| Query Builder | `Builder<M>` with `db_where` / `or_where` / `where_in` / `where_between` / `where_null` / `where_has` / `with` / `with_count` / `order_by` / `group_by` / `having` / `paginate` / etc. | shipped | [Queries](queries.md) |
| `whereBinary()` family | `Builder::where_binary` / `or_where_binary` / `where_not_binary` / `or_where_not_binary`, and `DB::table(...).where_binary(...)` | shipped | MySQL and MariaDB emit `= binary`; Postgres and SQLite return an error instead of a collation-dependent match. [Queries](queries.md) |
| Pagination | `LengthAwarePaginator`, `Paginator` (simple), `CursorPaginator` | shipped | All three serialise to the fields of Laravel's `toArray()`: `next_page_url` and `prev_page_url` on each, `first_page_url` on `LengthAwarePaginator` and `Paginator`, and `last_page_url` and `links` on the paginator with a total. A page URL is the `path` from `with_path` plus the page parameter, and a `path` with a query string keeps it. [Pagination](pagination.md) |
| Migrations | `#[derive(DeriveMigrationName)] struct M;` + `up`/`down` + `Migrator`, written with SeaORM's builder or the [schema builder](migrations.md#the-schema-builder) | shipped | Run via `suprnova migrate`/`migrate:rollback`/`migrate:status`/`migrate:fresh`. [Migrations](migrations.md), [CLI Migrations](cli-migrations.md) |
| Schema builder | `Schema::create(manager, "posts", \|t\| { t.id(); t.string("title"); t.timestamps(); })`, `Schema::table`, `Schema::drop_if_exists` from `suprnova::schema::Schema` | diverged | Takes the migration's `manager` instead of a global facade, so it runs in the migration's transaction. `timestamps()` and `soft_deletes()` create string columns. No column type change and no table rebuild on SQLite. [The schema builder](migrations.md#the-schema-builder) |
| Seeders | `Seeder` trait + `db:seed` subcommand | shipped | Per-model factories. [Seeding](seeding.md) |

## Eloquent ORM

| Laravel | Suprnova | Status | Notes / link |
|---|---|---|---|
| `class User extends Model` | `#[suprnova::model(table = "users")] struct User { ... }` | shipped | The macro generates the SeaORM `Model` beside the struct and converts between them. [Eloquent](eloquent.md) |
| Find / first / get | `User::find(id)`, `User::query().first()`, `User::all()`, `Builder::get` | shipped | All async |
| Create / update / delete | `User::create(attrs)`, `user.update(attrs)`, `user.delete()` | shipped | `attrs! { name: "...", email: "..." }` macro for partial attrs |
| Mass assignment guards | `#[model(fillable = [...])]` / `#[model(guarded = [...])]` + `unguarded \|\| { ... }` scope | shipped | `prevent_silently_discarding_attributes()` for strict mode |
| Soft deletes | `#[model(soft_deletes)]` on a model that declares `deleted_at` + `SoftDeletes` trait | shipped | `with_trashed()`, `only_trashed()`, `restore()`, `force_delete()`. `with_trashed()` and `only_trashed()` lift only the soft-delete filter; every registered global scope still applies. `delete_all()` writes the tombstone and `force_delete_all()` deletes the rows. |
| Prunable / MassPrunable | `#[prunable] impl Prunable for User { ... }` + `model:prune` worker | shipped | Deletes the model's own rows; relations are not pruned with it |
| Timestamps | Auto `created_at`/`updated_at` if columns are present | shipped | Disable via `#[model(timestamps = false)]` |
| Primary key types | i64 default; UUID / ULID via `#[model(unique_id = "uuid")]` or `unique_id = "ulid"` | shipped | Auto-generates id on insert |
| Local scopes | `#[scopes(User)] impl User { fn active(q: Builder<Self>) -> Builder<Self> { ... } }` | shipped | Method dispatch on `Builder<M>`, and `User::active()` as a static form. A method in the block that takes or returns a `Builder` in any other form is a compile error naming the accepted shape; `#[not_scope]` marks a helper that handles one legitimately. |
| Global scopes | `impl GlobalScope<Article> for TenantScope { fn apply(...) }` + `ScopeRegistry::register::<Article, _>(TenantScope)` | shipped | Opt out with `Article::without_global_scope::<TenantScope>()` or `Article::without_global_scopes()`, which also chain onto a builder. The scopes run when the query runs, so `query().or_where(...)` never folds into a scope's own terms, and `update_all`, `delete_all` and `increment_each` reach only the rows a read would return. |
| Relationships (11 kinds) | `HasOne`, `HasMany`, `BelongsTo`, `BelongsToMany`, `HasOneThrough`, `HasManyThrough`, `MorphOne`, `MorphMany`, `MorphTo`, `MorphToMany`, `MorphedByMany` | shipped | Per-family morph enum. A `MorphTo` target can have an `i64`, `String`, UUID or ULID key, and a nested eager load goes through it: `with(["commentable.user"])`. [Relationships](eloquent-relationships.md#morphto-keys) |
| `wherePivot` family (incl. the closure form) | `where_pivot` / `where_pivot_op` / `where_pivot_in` / `where_pivot_not_in` / `where_pivot_null` / `where_pivot_not_null` / `where_pivot_between` / `where_pivot_not_between` / `where_pivot_group` plus `or_` twins | diverged | Reads only - a pivot filter never narrows `attach` / `detach` / `sync`, and eager loads do not carry it. [Relationships](eloquent-relationships.md) |
| Eager loading | `User::query().with(["posts", "posts.comments"]).get()` | shipped | `EagerLoadDispatch` is sealed; only macro-generated relations can implement it |
| Lazy loading prevention (`preventLazyLoading`) | `prevent_lazy_loading(bool)` makes a relation read fail on a model of a multi-row query; `handle_lazy_loading_violation(handler)` logs instead of failing | diverged | Same rule as Laravel's. `refresh()` drops the mark where Laravel keeps it, and the rows of `cursor()` and `lazy()` are marked where Laravel's `cursor()` marks none. [Preventing lazy loading](eloquent.md#preventing-lazy-loading) |
| Aggregates on relations | `with_count(["posts"])`, `with_sum(("orders", "total"))`, `with_avg`, `with_min`, `with_max` | shipped | One grouped query per aggregate |
| `whereHas` / `whereDoesntHave` | `where_has("posts", \|q\| q.db_where("published", true))` | shipped | Correlated EXISTS engine |
| `load` / `loadMissing` | `user.load(["posts"]).await?` and `user.load_missing(["posts"]).await?` on one model (`Model` trait), and the same on a collection | shipped | `load_missing` loads only relations not already loaded, at every level of a dotted name. [`load` / `load_missing` on one model](eloquent.md#load--load_missing-on-one-model) |
| Cloning a record | `user.replicate()` / `user.replicate_into::<OtherType>()` | shipped | Dispatches `Replicating` event |
| Touching parent timestamps | `#[model(touches = ["post"])]` | shipped | One `UPDATE` per `BelongsTo` or `MorphTo` owner, one level deep and event-free (no grandparent recursion, no parent `saved` event). `without_touching` / `without_touching_on::<M, _, _>()` to skip. [Parent touching](eloquent.md#parent-touching) |
| Observers | `impl Observer<User>` + `#[suprnova::observer(User)]` | shipped | 16 lifecycle events |
| 16 lifecycle events | `Retrieving`, `Retrieved`, `Creating`, `Created`, `Saving`, `Saved`, `Updating`, `Updated`, `Deleting`, `Deleted`, `Trashed`, `Restoring`, `Restored`, `Replicating`, `ForceDeleting`, `ForceDeleted` | shipped | Per-model `events::*` submodule. `EventResult::cancel(_)` short-circuits with a 400; pruning has its own `Prunable::pruning` hook |
| Mutators / Accessors | `#[accessor] fn full_name(&self) -> String { ... }` + `#[mutator] fn set_password(&mut self, value: Value) -> Result<(), FrameworkError>` for fields in `mutators = [...]` | shipped | [Mutators](eloquent-mutators.md) |
| Casts (31 built-in) | `casts! { AsString, AsInt, AsFloat, AsBool, AsJson, AsArray, AsArrayObject, AsObject, AsCollection, AsOptionalJson, AsOptionalArray, AsOptionalArrayObject, AsOptionalObject, AsOptionalCollection, AsDate, AsDateTime, AsImmutableDate, AsImmutableDateTime, AsOptionalDateTime, AsNativeDateTime, AsOptionalNativeDateTime, AsNaiveDateTime, AsOptionalNaiveDateTime, AsTimestamp, AsDecimal, AsEnum<E>, AsEncrypted, AsEncryptedObject, AsEncryptedArray, AsEncryptedCollection, AsHashed }` | shipped | Implement `Cast` for custom. The `AsOptional*` forms map a nullable column, where `None` is SQL `NULL` |
| Collections | `Collection<M>` with `pluck`, `filter`, `map`, `each`, `chunk`, `groupBy`, `keyBy`, `sort_by`, `where_eq`, `where_in`, `first`, `last`, `len`, `is_empty`, `to_array` and Laravel friends; derefs to a `[M]` slice | shipped | [Collections](eloquent-collections.md) |
| `modelKeys()` | `Builder::model_keys().await?` (no hydration, qualified key) and `Collection::model_keys()` | shipped | Both return `Vec<M::Key>`; the builder terminal projects `users.id` so it survives joins |
| API Resources | `#[derive(Data)]` with `#[json_resource("type")]` + `IntoJsonResource` + `JsonApiResponse` + fieldsets + includes | shipped | JSON:API documents; plain Laravel-style JSON comes from `#[derive(Data)]` serialisation. `?include=` paths are capped at `max_relationship_depth` (default 5), matching `JsonApiResource::$maxRelationshipDepth`. [API Resources](eloquent-resources.md) |
| Serialization | `#[model(hidden = [...], visible = [...], appends = [...])]` | shipped | Same control over which attributes serialise. [Serialization](eloquent-serialization.md) |
| Factories | `#[derive(Factory)]` on the model generates `UserFactory`: `UserFactory::new().count(5).create_many().await?` | shipped | `Sequence` counter for varying values. [Factories](eloquent-factories.md) |
| Lifecycle: chunking / lazy / cursor | `Builder::chunk(n, \|page\| async { ... })`, `lazy()`, `cursor()` | shipped | Memory-bounded iteration over large tables |
| Pessimistic locking | `Builder::lock_for_update()`, `shared_lock()` | shipped | Inside a transaction |
| `refreshForUpdate()` | `model.refresh_for_update().await?` | shipped | `SELECT ... FOR UPDATE` reload; no-op lock on SQLite. [Row locking](eloquent.md#row-locking) |
| `inOrderOf(col, values)` | `Builder::in_order_of(col, values)` | shipped | Bound `CASE WHEN` ordering; unlisted values sort last. Typed builder only. [Ordering](eloquent.md#ordering) |
| `orWhereKey` / `orWhereKeyNot` | `Builder::or_where_key(id)` / `Builder::or_where_key_not(id)` | shipped | Fold into the preceding clause as a disjunction; `or_filter_key` / `or_filter_key_not` aliases |
| `whereJsonContains` family | `where_json_contains` and `where_json_length` on the query builder (driver-aware) | shipped | On SQLite, contains is a substring match |

## Pagination

| Laravel | Suprnova | Status | Notes / link |
|---|---|---|---|
| `LengthAwarePaginator` | `LengthAwarePaginator` (page + total + per_page + last_page) | shipped | `Builder::paginate(n).await?` |
| `Paginator` (simple) | `Paginator` (page + per_page + has_more, no count) | shipped | `Builder::simple_paginate(n).await?` |
| `CursorPaginator` | `CursorPaginator` (opaque cursor token + direction) | shipped | `Builder::cursor_paginate(n).await?`; deterministic for infinite scroll |
| Inertia integration | `IntoInertiaScroll` trait + `ScrollMetadata` | shipped | Wires straight into Inertia's `WhenVisible` / `merge` |

## AI (Laravel ships it in the framework; we don't gatekeep)

| Laravel | Suprnova | Status | Notes / link |
|---|---|---|---|
| AI SDK | No first-party AI SDK | by design no | Bring the crate you already use (`async-openai`, `anthropic-sdk`, `ollama-rs`, `tokenizers`, etc.) and bind under `App` |
| MCP (Model Context Protocol) | No first-party MCP server adapter | by design no | The Rust MCP crates (`mcp-rs`, `mcp-sdk-rust`) sit cleanly under the existing routing / supervisor surface |
| Boost (Laravel coding agent) | n/a | by design no | Out of framework scope |

## Testing

| Laravel | Suprnova | Status | Notes / link |
|---|---|---|---|
| `php artisan test` | `cargo test` | shipped | [Testing](testing.md) |
| Pest / PHPUnit style | `#[suprnova_test]` (async-aware) + `expect!()` Jest-like assertions + `describe!()` / `test!()` BDD macros | shipped | All three work interchangeably |
| Feature tests (HTTP) | Drive `handle_request(router, registry, req)` in the same process, normally through a loopback hyper connection so the server receives a real `Incoming` body | shipped | [HTTP Tests](http-tests.md) |
| `TestResponse` wrapper | `suprnova::testing::TestResponse` - fluent `assert_status` / `assert_json_path` / `assert_cookie` / `assert_session_has` and friends, all chaining `&Self` | shipped | [HTTP Tests](http-tests.md#fluent-response-assertions-with-testresponse) |
| Inertia testing helpers | `suprnova::testing::AssertableInertia` - `component`/`url`/`version`/`prop`/`has`/`missing`/`where_`/`count`/`has_flash`, plus `reload_only`/`reload_except`/`load_deferred_props` via a caller-supplied `with_reload` closure | shipped | [HTTP Tests](http-tests.md#testing-inertia-responses) |
| Console tests | `suprnova::console::test(["users:purge", "--days", "30"]).expects_question("Delete 12 users?", "yes").run().await` returns a `ConsoleRun` with `output()`, `errors()`, `exit_code()`, `assert_successful()`, `assert_failed()`, `assert_output_contains()`, `assert_errors_contain()` and `assert_every_question_was_asked()` | shipped | Runs the command through the dispatcher the console binary uses. A command prints with `console::line` and `console::error_line` and asks with `console::ask` and `console::confirm`; what it prints with `println!` is not collected. Questions must come in the order the test gives them. |
| Browser tests (Dusk) | n/a in framework - use Playwright / WebdriverIO / `gstack` agent browser | by design no | Cross-language tooling already exists; we don't reinvent it |
| Database tests | `TestDatabase::fresh::<Migrator>()` | shipped | Creates a fresh per-test in-memory SQLite database, applies migrations, registers it in the test container, and discards that isolated database/container state on drop; it does not wrap each test in a rollback transaction. [Database Tests](database-testing.md) |
| Mocking & fakes | Per-facade fakes: `MailFake`, `NotifyFakeGuard`, `EventFakeGuard`, `Queue::fake`, `Bus::fake`, `Http::fake`, `Storage::fake` | shipped | `Queue::fake()` and `Bus::fake()` return a guard that uninstalls the fake on drop. Recorded calls + assertion helpers. [Mocking](mocking.md) |
| `QueueFake` job uuids | `queue::testing::pushed_with_id::<J>()` | shipped | The fake stamps an envelope id per push and emits the same `JobQueued` a real push does |
| Time travel | `TestClock::freeze()`, `TestClock::travel_to(at)`, then `advance` / `set` on the guard; `TestClock::scope` for a multi-thread runtime | diverged | The clock stands still until you move it, and it is per thread (guard) or per future (scope), not process-wide, so parallel tests keep their own time. Tokio's `pause` / `advance` move only Tokio's timers. [Moving the clock](testing.md#moving-the-clock) |
| Container isolation | `TestContainer::fake()`, then `TestContainer::bind(...)` - thread-local | diverged | Parallel-safe by construction. [Container](container.md) |

## Payments (Laravel's Cashier; ours is provider-generic)

| Laravel | Suprnova | Status | Notes / link |
|---|---|---|---|
| Cashier (Stripe) | `suprnova-payments-stripe` adapter crate behind generic `Payment` / `Subscription` / `CustomerStore` / `WebhookHandler` traits | diverged | Generic surface, concrete adapter. [Payments](payments.md), [Stripe Adapter](payments-stripe.md) |
| Cashier (Paddle) | `suprnova-payments-paddle` adapter | diverged | Merchant-of-Record flow + no direct `Payment` impl (Paddle owns the gateway). [Paddle Adapter](payments-paddle.md) |
| Custom provider | Implement `PaymentProvider`, `Checkout`, `Subscription`, `CustomerStore` and `WebhookHandler`; checkout returns a `SessionPayload` | shipped | [Provider Guide](payments-provider-guide.md) |
| Inertia checkout components | Documented dispatch loops for Svelte / React / Vue against `SessionPayload.flow` | shipped | [Payments Frontend](payments-frontend.md). Ready-made payment pages ship in the Directory starter ([Starter Kits](starter-kits.md)) |
| Subscription lifecycles | `Subscription::subscribe / update / cancel / get` (where the provider supports them) | shipped | `NotSupported` returned where the provider doesn't (e.g. Paddle `subscribe` and price-set replacement) |
| Webhook idempotency | `payments_webhook_events` mirror table with `UNIQUE(provider, provider_event_id)` | shipped | Stripe-style replay protection |
| Mirror tables | `payments_customers`, `payments_payment_methods`, `payments_subscriptions`, `payments_subscription_items`, `payments_transactions`, `payments_webhook_events` | shipped | A `provider_metadata` JSONB column on each for adapter-specific fields (`payments_webhook_events` keeps the raw `payload` instead) |

## Frontend (Laravel has Blade + starter kits; we have Inertia)

| Laravel | Suprnova | Status | Notes / link |
|---|---|---|---|
| Blade | n/a - Inertia is the view layer | diverged | [Frontend](frontend.md) |
| Inertia.js | First-class: v3 over Svelte 5 / React 19 / Vue 3.5 | shipped | [Inertia Responses](frontend-inertia-responses.md), [Pages](frontend-pages.md) |
| `Route::inertia($uri, $component, $props)` | `Router::inertia(path, component, props)` | shipped | Returns a `RouteBuilder`, so `.name(...)` / `.middleware(...)` chain; `Router::view` is the older alias |
| Page URL resolution (`Inertia::resolveUrlUsing`) | `page.url` is path + query; override with `InertiaConfig::url_resolver` | shipped | Default derivation matches the version middleware's `X-Inertia-Location` byte for byte; a `url_resolver` changes `page.url` only |
| Inertia protocol middleware (`Vary`, empty response, version bounce) | `InertiaHeadersMiddleware` + `InertiaVersionMiddleware` + `Inertia303Middleware` - three of the middlewares `Inertia::install` wires (validation-error redirect and error pages are the next two rows) | shipped | `Vary: X-Inertia` on every response; empty `200` on an Inertia visit becomes `303` back; the 409 bounce re-flashes the session |
| Validation-error redirect (`Middleware::resolveValidationErrors`, `$withAllErrors`) | `InertiaValidationRedirectMiddleware`, wired by `Inertia::install`; `InertiaConfig::with_all_errors(bool)` | shipped | A `422` on an Inertia visit becomes `303` back with the errors flashed; a field's value collapses to its first message unless `with_all_errors(true)`. [Inertia Responses](frontend-inertia-responses.md#validation-failures) |
| Error pages (`Inertia::render('Error', ['status' => …])` from the exception handler) | `InertiaConfig::error_page("Error")`; `InertiaErrorPageMiddleware`, wired by `Inertia::install` when a component is named | shipped | One config line instead of an exception-handler edit. A `403` / `404` / `429` / `500` on an Inertia visit or a browser navigation renders the named page at the original status, with `status` / `message` / `request_id` props; API clients, `422`s, `X-Inertia-Location` bounces, and any body that is not an error envelope are untouched. The three starters ship the page and enable it. [Inertia Responses](frontend-inertia-responses.md#error-pages) |
| External redirect + history clearing | `InertiaResponse::location_for(&req, url)`, `App::clear_history()` | shipped | `location_for` is `409` for XHR and `302` for a hard navigation; `App::clear_history()` survives the logout redirect |
| `Inertia::share` / `getShared` / `flushShared` | `App::inertia_share` / `_lazy` / `_once`, `App::inertia_shared(key)`, `App::flush_inertia_shared()` | shipped | Dot-key nesting via `Arr::set` semantics; per-request `InertiaSharedData::share(&req, component)` can vary by page. A dotted share stays flat until the response's unpacking pass, so `only`/`except` match an ancestor entry (`only: ['auth']` reaches `auth.user`) where Laravel gets the same result from `Arr::set` at share time |
| Partial reloads | `#[derive(Data)]` + `current_include_set().includes("subset")` + Inertia's partial-reload protocol | shipped | Type-safe include sets. `?include=` gates every lazy flavor including `lazy(deferred)`, and runs ahead of `X-Inertia-Partial-Data` so a disallowed include still returns 400. `errors` is exempt from `only`/`except`, matching Laravel's `Inertia::always` share |
| Deferred props | `.defer(…)` / `.defer_with(…, DeferOptions)`, or `Prop::…defer()` | shipped | Inertia v3 deferred-props protocol; `DeferOptions` carries the group and the rescue flag. `deferredProps` ships on the initial visit only - `resolveDeferredProps` returns `[]` on any matched partial |
| Merge props | `.merge` / `.merge_prepend` / `.deep_merge` / `.merge_with(MergeStrategy)` / `.merge_lazy` / `.merge_lazy_with`, or `Prop::…merge().merge_with_path(...)` | shipped | Inertia v3 merge protocol; `match_on` takes one field or several; `merge_with_path` merges a nested field instead of the prop's root |
| Prop composition (`defer()->merge()`, `merge()->once()`, `optional()->once()`) | `Prop` flag builder + `InertiaResponse::prop(key, prop)` | shipped | `Prop` is a struct of orthogonal flags, mirroring the PHP adapter's `Deferrable` / `Mergeable` / `Onceable` interfaces |
| Encrypt history | `EncryptHistoryMiddleware` | shipped | History encrypted at rest in the client |
| Scroll position | `.scroll` / `.scroll_with` / `.scroll_wrapped` / `.paginate` + `ScrollMetadata` / `ProvidesScrollMetadata` | shipped | Auto-restore on navigation; `reset` reads `X-Inertia-Reset`, matching `resolveScrollProps` |
| TypeScript types | `suprnova generate-types` reads `#[derive(InertiaProps)]` and emits `inertia-props.ts` | shipped | [TypeScript Types](frontend-typescript-types.md) |
| Vite manifest reading | Auto-wired via `InertiaConfig::manifest_path` | shipped | HMR in dev, hashed assets in prod. `Inertia::install` fails closed in production when the manifest is missing |
| Asset version from the build manifest | `InertiaConfig` default: `VersionResolver::from_manifest(manifest_path)` | shipped | Hash of the manifest bytes; static `"1.0"` fallback when there is no build to hash |
| Inertia SSR (`inertia:start-ssr`) | `InertiaConfig::ssr(...)` on the config passed to `Inertia::install`, worker launched by `suprnova ssr:start` | shipped | Out-of-process worker over HTTP loopback; falls back to CSR on error or timeout unless `ssr_throw_on_error(true)`. `InertiaConfig::ssr_bundle_path(...)` gates dispatch on the built bundle existing on disk (mirrors `ensure_bundle_exists`), toggled with `.ssr_ensure_bundle_exists(bool)` (on by default once a bundle path is set); `suprnova new` scaffolds `frontend/src/ssr.{ts,tsx}` and a `build:ssr` script for every starter; `suprnova ssr:check` verifies the worker's `GET /health` route. [Inertia Responses](frontend-inertia-responses.md) |

## CLI

| Laravel | Suprnova | Status | Notes / link |
|---|---|---|---|
| `php artisan` | Per-app `console` binary built from `#[command]` macros | shipped | [Console](console.md), [CLI overview](cli.md) |
| `make:controller` / `make:model` / etc. | `suprnova make:controller / make:middleware / make:action / make:error / make:inertia / make:migration / make:task` | shipped | [Generators](cli-generators.md) |
| `serve` | `suprnova serve` (backend + Vite dev server together) | shipped | [Serve](cli-serve.md). Skips the Vite pane on a `--api` project instead of refusing to start. |
| `migrate` family | `suprnova migrate / migrate:rollback / migrate:status / migrate:fresh` | shipped | [Migrations CLI](cli-migrations.md) |
| `db:seed` | `cargo run --bin console db:seed` (via per-app console) | shipped | Seeders registered via `Seeder` trait; a targeted run prints RUNNING / DONE with elapsed milliseconds |
| `schedule:run` / `schedule:work` / `schedule:list` | Same names on the app binary, forwarded by the `suprnova` CLI | shipped | [Scheduling CLI](cli-scheduling.md) |
| `queue:work` | Same name on the app binary | shipped | Graceful shutdown on SIGTERM/SIGINT |
| `tinker` | No REPL | by design no | See the row in "Digging deeper" |

## Deployment

| Laravel | Suprnova | Status | Notes / link |
|---|---|---|---|
| `php artisan optimize` | `cargo build --release` | diverged | One binary, no opcache step |
| `php artisan config:cache` | Typed config is compile-time-checked already | diverged | No runtime cache to invalidate |
| `php artisan route:cache` | Routes are macro-expanded at compile time | diverged | The router is built at boot from already-typed routes |
| Envoy (SSH deploys) | Use any orchestrator - Docker, systemd, Kubernetes, fly.io, Railway | by design no | The binary is the deploy artifact |
| Forge / Vapor | Not ours to ship - but the recipes for Railway, DO, and Hetzner cover the same job | diverged | [Deployment](deployment.md), [Railway](deployment-railway.md), [Digital Ocean](deployment-digital-ocean.md), [Hetzner](deployment-hetzner.md) |
| Maintenance mode (`php artisan down` / `up`) | `./app down` / `./app up` - bypass secret with a server-checked 12-hour expiry, custom retry/message/except paths, `file` or `cache` driver | shipped | [Deployment](deployment.md) |
| Horizon (queue dashboard) | No first-party dashboard | not built | Failed-job inspection via `suprnova queue:failed`, `queue:retry`, `queue:forget`, `queue:flush` and `queue:prune-failed`, which the app binary runs too |

## Packages (Laravel's official packages - ours either ship in core, ship as adapters, or are deliberate gaps)

| Laravel package | Suprnova | Status | Notes / link |
|---|---|---|---|
| Cashier (Stripe) | `suprnova-payments-stripe` | shipped | Generic + adapter. [Payments](payments.md) |
| Cashier (Paddle) | `suprnova-payments-paddle` | shipped | MoR flow. [Payments](payments.md) |
| Dusk | n/a | by design no | Cross-language browser tooling already exists (Playwright, etc.) |
| Envoy | n/a | by design no | Containers / systemd / orchestrators do the job |
| Fortify | Replaced by `auth_flows` | shipped | Same job, integrated. [Auth Flows](auth-flows.md) |
| Folio | n/a - page-based routing isn't idiomatic Rust | by design no | Use `routes!` for explicit routing |
| Homestead | n/a - use Docker / DevContainers | by design no | [Docker recipe](cli-docker.md) |
| Horizon | n/a | not built | Failed jobs surface through `suprnova queue:failed` and the commands beside it |
| Mix | Replaced by Vite | diverged | Vite ships in every frontend scaffold (`suprnova new --api` has no frontend) |
| Octane | n/a - we are already long-lived Tokio | by design no | Single binary, always warm, no FPM to swap out |
| Passport | n/a | not built | Run a dedicated IdP (Hydra, Keycloak) behind Suprnova |
| Pennant (feature flags) | Re-implemented as `features::*` | shipped | [Feature Flags](feature-flags.md) |
| Pint (PHP code style) | `cargo fmt` + `cargo clippy` | diverged | Standard Rust toolchain |
| Precognition | Precognitive requests (the `Precognition` header) answered by `FormRequest` extraction, with the same `#[derive(Data, Validate)]` types | shipped | Validation runs without calling the handler |
| Prompts (CLI UI) | Use the `dialoguer` / `inquire` crate when needed | by design no | Rust ecosystem already covers this |
| Pulse | n/a | not built | OTel metrics and traces |
| Reverb (WebSocket server) | Built into Suprnova (`ws!()` + `BroadcastHub`) | diverged | No separate server needed - it's the same process |
| Sail (Docker dev) | `suprnova-cli` ships Docker recipes inline | shipped | [CLI Docker](cli-docker.md) |
| Sanctum | `BearerTokenMiddleware` over Magnetar bearer sessions | diverged | No separate package or personal-access-token management surface |
| Scout (full-text search) | n/a | not built | Semantic search ships via [Vector](vector.md); keyword search through `meilisearch-sdk` / `elasticsearch` |
| Socialite | Magnetar provider registry and `Auth::oauth(provider)` | shipped | [OAuth](oauth.md) |
| Telescope | n/a | not built | Tracing + OTel cover the diagnostic gap |
| Valet | n/a - Rust apps run directly | by design no | `suprnova serve` is the dev runner |

## Macros (Rust-specific surface; closest Laravel analogues for context)

Suprnova ships a wide set of proc-macros that don't have a Laravel analogue
because Laravel doesn't have macros - it has runtime reflection. Including
them here so you don't miss them.

| Macro | Closest Laravel idea | What it does |
|---|---|---|
| `#[suprnova::model]` | `extends Model` | Generates SeaORM entity + impls `Model` trait |
| `#[suprnova::observer(M)]` | `User::observe(UserObserver::class)` | Registers an `Observer<M>` impl via `inventory` |
| `#[scopes(M)]` | Local scopes on a model | Adds methods to `Builder<M>` |
| `#[accessor]` / `#[mutator]` | Eloquent accessors / mutators | Field-level get/set hooks |
| `#[handler]` | Controller `__invoke` | Auto-extracts typed params from `Request` |
| `#[command]` / `#[derive(Command)]` | Artisan command class | Registers a console subcommand |
| `#[policy]` | Policy class | Registers a `Policy` impl via `inventory` |
| `#[service(T)]` | Service provider `register` | Binds `T` into the container |
| `#[injectable]` | Constructor injection | Generates an `App::make`-backed constructor |
| `#[derive(InertiaProps)]` | Inertia props | TypeScript codegen + Inertia serialization |
| `#[derive(Data)]` | Request DTO | Extractable from `Request` with include-set support |
| `#[derive(FormRequest)]` | `FormRequest` class | Validation + auth gate + transformation |
| `#[derive(Factory)]` | Model factory | Faker-backed test data generation |
| `#[derive(Data)]` + `#[json_resource]` | API Resource | JSON:API + Laravel-shape serialization |
| `#[workflow]` / `#[workflow_step]` | n/a in Laravel | Long-running stateful work |
| `routes!` + `get!` / `post!` / `ws!` etc. | `Route::get` / `Route::post` | Compile-time route registration |
| `casts!` | `protected $casts = [...]` | Per-model cast declaration |
| `attrs!` | Mass-assignment array | Partial-attribute builder |
| `json_response!` / `text_response!` | `response()->json(...)` | Quick `Ok(HttpResponse::...)` |

See [Macros](macros.md) for the full reference.

## Helper functions (Laravel's global helpers; ours are typed)

Laravel ships hundreds of small globals (`str_replace_first`, `array_flatten`,
`now()`, `tap()`, `optional()` …). Most of them have a direct Rust equivalent
in `std` or a small standard crate, so Suprnova doesn't reintroduce them as a
single namespace. The ones that *are* useful to have aliased ship under their
home module.

| Laravel helper | Suprnova / Rust equivalent | Where |
|---|---|---|
| `auth()` | `Auth::user().await?` | [Authentication](authentication.md) |
| `cache()` | `Cache::get/put/...` | [Cache](cache.md) |
| `config('app.name')` | `Config::get::<AppConfig>()?.name` | [Configuration](configuration.md) |
| `csrf_token()` | `csrf_token()` (same name) | [CSRF](csrf.md) |
| `dd()` | `Builder::dd()` (Eloquent query dump-and-die) / `dbg!()` from the stdlib | `Builder::dump()` / `Builder::dd()` exist for query inspection; use `dbg!()` for general values |
| `env('APP_KEY')` | `env("APP_KEY")` / `env_required("APP_KEY")` / `env_optional("APP_KEY")` | [Configuration](configuration.md), [Env Vars](env-vars.md) |
| `now()` | `chrono::Utc::now()` (re-exported as `suprnova::chrono`) | - |
| `optional($x)->y` | `x.as_ref().map(\|x\| x.y)` | Rust handles this with `Option<T>` directly |
| `redirect('/')` | `redirect("/")` (same name) | [Routing](routing.md) |
| `request()` | `Request` is passed into your handler | [Requests](requests.md) |
| `response()` | `HttpResponse::json/text/redirect/...` | [Responses](responses.md) |
| `route('posts.show', ['post' => 1])` | `url("posts.show", &[("post", "1")])` | [URL Generation](urls.md) |
| `session('key')` | `session().get("key")` | [Session](session.md) |
| `str()` / `Str::camel($x)` | `heck` crate methods (`ToUpperCamelCase`, etc.) | - |
| `tap($x, fn) → $x` | `tap` from `tap` crate, or `dbg!` for quick inspection | Use the `tap` crate idiomatically |
| `today()` | `chrono::Utc::now().date_naive()` | - |
| `value($x)` | Just call the closure: `x()` | n/a - Rust closures need no helper |
| `view('home', $data)` | Inertia response: `Inertia::render("Home", data)` | [Inertia Responses](frontend-inertia-responses.md) |

## What Suprnova doesn't have

Every **not built** row above in one place, with the route to take
instead:

| Area | What's missing | Route instead |
|---|---|---|
| Search (Scout - keyword) | Algolia / Meilisearch / Elastic adapter | Use `meilisearch-sdk` / `elasticsearch` directly; [Vector](vector.md) handles semantic search |
| Passport (OAuth server) | First-party OAuth identity provider | Run Hydra / Keycloak behind Suprnova |
| Telescope (debug dashboard) | Web UI for requests / queries / events / cache hits | Use OTel + tracing output ([Observability](observability.md)) |
| Pulse (perf dashboard) | Web UI for slow queries / errors / hot routes | Same: the OTel surface |
| Horizon (queue dashboard) | Web UI for queue depth / failed jobs / throughput | `suprnova queue:failed` (and `queue:retry`, `queue:forget`, `queue:flush`, `queue:prune-failed`) and OTel metrics |
| MongoDB | First-party document-database adapter | The `mongodb` crate directly, bound via `App::bind` |

## What we won't ship (and why)

| Laravel feature | Why Suprnova doesn't have it |
|---|---|
| Tinker (REPL) | Rust doesn't have a productive REPL story for compiled binaries. A short `#[suprnova_test]` or a one-off `cargo run --bin <thing>` script does the job |
| Blade templates | Inertia is the view layer; we don't ship a parallel server-rendered template engine |
| `helpers.md` kitchen-sink | Rust ships `std` + small focused crates (`heck`, `chrono`, `regex`); we don't reintroduce a single global namespace |
| Mix | Vite covers it and ships in every scaffold |
| Octane | Suprnova is already long-lived Tokio; there's no FPM mode to optimise out of |
| Dusk (browser tests) | Cross-language tooling (Playwright, WebdriverIO, `gstack` agent browser) already solves this |
| Sail (Docker dev) | Docker recipes ship inline ([CLI Docker](cli-docker.md)); no separate package needed |
| Valet | `suprnova serve` is the dev server |
| Envoy (SSH deploys) | Containers / systemd / orchestrators do the job; we don't need a bespoke SSH DSL |
| Concurrency facade (`Concurrency::run`) | Tokio (`tokio::join!` / `tokio::spawn` / `tokio::select!`) is the answer; no facade needed |
| Processes facade | `tokio::process::Command` is already the right shape |
| First-party AI SDK / MCP / Boost | Pick the Rust crates you already use; we don't gatekeep |
| Dedicated Redis facade | Cache/queue/rate-limit cover 95% of typical use; reach for the `redis` crate when you need ad-hoc commands |
| Strings facade | `heck`, `regex`, `std::str` cover it; no `Str::camel($x)` global |
| Prompts (CLI UI library) | `dialoguer` / `inquire` already exist; we don't reinvent |
| Laravel-style PHP/JSON translation files | Localization ships, but the catalog format is Fluent `.ftl` - one format the server and the browser both parse. `trans_choice` has no equivalent either: Fluent selects CLDR plural categories inside the message. [Localization](localization.md) |
| `php artisan dev --tabs` (TUI multi-pane dev-process mode) | Single-terminal, `[name]`-prefixed output is the Rust dev-tooling norm (`cargo watch`, `bacon`, `just`) - `suprnova serve` already gives every process (backend, frontend, and any `Suprnova.toml` entry) its own colored prefix and auto-restart. A tabbed TUI is a second interaction model for a signal this already provides; `--stream`'s job - one scriptable, real-time output stream - ships as `suprnova serve --json` (NDJSON, one event per line). [Serve](cli-serve.md#extra-dev-processes) |

## How this list stays honest

Every row in the **shipped** column is verifiable by:

1. Grepping `framework/src/lib.rs` for the named export
2. Running the framework test suite (`cargo test --workspace`)
3. Reading the linked chapter

Every row in the **not built** column has its route in the Notes column. Every
row in the **by design no** column has a one-sentence reason in the Notes column;
those reasons are the design principles in [Introduction](introduction.md)
applied to a specific feature.

Last reviewed against Laravel 13.25.0.

If you find a Laravel feature you reach for that isn't on this map, open
an issue - it either has a Suprnova answer that's missing a row, or it's
a real gap and we want to know.

## Next

- [From Laravel](from-laravel.md) - the same map, narrated as a side-by-side
- [Introduction](introduction.md) - the design principles this parity work follows
- [`documentation.md`](documentation.md) - the master TOC across every chapter
