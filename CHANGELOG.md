# Changelog

A readable, per-version log of what changed in Suprnova. Each version
section is that version's release record. A version is released when its
version commit and matching `v<version>` tag are pushed atomically. Newest first.

## 3.1.0 - 2026-10-03

### Added

- **Second-factor attempt limits.** `TWO_FACTOR_MAX_ATTEMPTS` (default 5)
  and `TWO_FACTOR_LOCKOUT_MINUTES` (default 15, at most 43,200) set the
  sliding window in which second-factor failures lock an account's second
  factor; `TwoFactor::unlock` and Magnetar's `TwoFactorService::unlock`
  lift it. Password and second-factor lockouts count separately, so a
  password reset no longer lifts a second-factor lock.
  `Authenticatable::auth_epoch` lets a provider carry the epoch it read
  with the password hash through sign-in. This landed after the `v3.1.0`
  tag.
- **`Lang::reload`.** `Lang::reload().await` reloads the translation
  catalogs and, when their text changed, makes RenderCache pages built from
  the old translations miss; dev hot reload does the same. Call it from a
  deploy hook instead of `Translator::reload`, which left cached pages in
  the old language until their TTL. This landed after the `v3.1.0` tag.
- **Action directives pass arguments.** `live:click="remove(42)"` and
  `live:click="rename('draft', true)"` send literal arguments to the
  action's parameters in declared order; before, every action directive
  sent none, so an action with parameters could not be called from a
  template. Only literals are accepted (numbers, quoted strings, `true`,
  `false`, `null`), and `live:check` reports a wrong count or a literal of
  the wrong type at its line and column. The island root lists each
  action's parameter names in `data-suprnova-live-actions`. This landed
  after the `v3.1.0` tag.
- **Heap profiling.** With the framework's `heap-profiling` feature an
  application profiles its heap with dhat: the framework installs dhat's
  allocator, `#[suprnova::main]` starts the profiler, and a command that
  finishes or a server that shuts down gracefully writes `dhat-heap.json`,
  or the file `SUPRNOVA_HEAP_PROFILE` names, for DHAT's viewer, and prints
  the bytes allocated, the peak and the heap left at the end. Without the
  feature no dhat code is compiled. The workspace and new applications
  have a `profiling` Cargo profile, the release profile with debug
  symbols, and new applications a `heap-profiling` feature that turns on
  the framework's.
- **The in-memory cache sweeps its expired entries.** The cache the
  framework binds removes every entry whose time is up every
  `CACHE_SWEEP_INTERVAL` seconds (60 unless set; 0 turns it off), so keys
  written once and never read again no longer stay for the life of the
  process. `InMemoryCache::with_periodic_sweep` builds such a cache, and
  `CacheConfig::sweep_interval` sets the interval in code.
- **Schema dumps.** `suprnova schema:dump` writes the database's schema
  and its migration ledger to `database/schema/<engine>-schema.sql`, with
  `pg_dump`, `mysqldump` or `mariadb-dump`, or straight from SQLite.
  `migrate`, `migrate:fresh`, `serve` and `TestDatabase::fresh` load it
  into a database that has run no migration, then run only the newer
  migrations; `--schema-path` names another file. `--prune` deletes the
  migrations the dump records and keeps each name as a
  `PrunedMigration`, which fails with an error if a database ever needs
  to run it.
- **Strings and numbers.** `Str::slug`, `Str::mask`, `Str::limit`,
  `Str::limit_words` and `Str::excerpt` work in characters, never bytes.
  `Str::plural` and `Str::singular` follow the language of the current
  locale with doctrine/inflector's rules for English, French, Norwegian
  Bokmål, Portuguese, Spanish and Turkish, English for any other, and keep
  the word's case. `Lang::percentage` writes a percentage as the locale
  does, and `Lang::abbreviate` shortens a number to `1.23M` with the
  locale's digits.
- **A Redis facade.** `Redis::connection("default")` reaches `REDIS_URL`,
  and `Redis::define` names other connections in the bootstrap, or
  `Redis::define_client` with a client built elsewhere. A connection opens
  on its first command and opens again after the server drops it;
  `Redis::purge` and `Redis::connections` manage them. The common commands
  are typed methods (`get`, `set`, `incr`, `hgetall`, `lrange`, `zadd`,
  `scan` and the rest), `command` runs any other and returns a `RedisValue`,
  refusing, as pipelines and transactions do, those that would change the
  shared connection (`SUBSCRIBE`, `MULTI`, `SELECT`, `CLIENT REPLY` and the
  like), and `client()` gives the `redis` crate's connection, now
  re-exported as `suprnova::redis`. Reads are sent again after a lost
  connection, as `REDIS_COMMAND_RETRIES` says; writes never are. A
  subscription the server closes subscribes again. `rediss://` URLs connect
  over TLS, for the facade and for the cache, queue and rate limiter URLs;
  the framework installs rustls's `ring` provider unless the application
  installed one. `pipeline` sends its commands before reading a reply and
  `transaction` wraps them in `MULTI` and `EXEC`. `subscribe`, `psubscribe`
  and the blocking commands (`blpop`, `brpop`, `blmove`, `brpoplpush`,
  `bzpopmin`, `bzpopmax`) each run on a connection of their own. With
  `Redis::enable_events`, `Redis::listen` and `Redis::listen_for_failures`
  hear every command outside a pipeline or a transaction.
- **Log channels.** `LOG_CHANNEL` names the channel `tracing`'s events go
  to, still `stdout` unless set: `stderr` (or `errorlog`), `single`
  (`storage/logs/suprnova.log`), `daily` (a file a day, keeping
  `LOG_DAILY_DAYS`, 14), `monthly` (keeping 3), `syslog` (the local socket,
  `LOG_SYSLOG_FACILITY`), `null`, and `stack` (the channels `LOG_STACK`
  lists). `Log::define` adds a channel in the bootstrap and `Log::extend` a
  driver, through the `LogSink` trait. `Log::channel`, `Log::stack` and
  `Log::build` return a logger with Laravel's eight levels and a JSON
  context whose values fill `{key}` placeholders; a broken channel in a
  stack stops none of the others. File channels flush at once for errors,
  within a second otherwise, and on `Log::flush` and shutdown.
  `MAIL_LOG_CHANNEL` routes the `log` mail transport. A channel that does
  not exist fails boot. `logging::build_subscriber` builds the subscriber
  the server installs, for an application or a test that installs its own.
- **A Process facade.** `Process::command(["git", "status"])` runs a
  program with its arguments, each passed as it is through no shell, and
  `Process::shell("npm ci && npm run build")` runs a command line through
  the system shell, as Laravel runs a string command. `run` returns the exit
  code and the captured output; a nonzero exit is a failed result, and
  `throw` makes it an error. `path`, `env`, `input`, `quietly`, `tty`, an
  output callback, a timeout (60 seconds unless set) and an idle timeout
  are options. A timeout, an idle timeout, `stop`, or dropping a started
  process or the future of `run` kills the program with every process it
  started: its process group on Unix, its tree for a `tty` process and on
  Windows. A started process's timeouts hold whether or not anything waits
  on it. Results keep the raw bytes as well as the text. `start` returns the program running; `Process::pool()` runs
  processes side by side with an optional concurrency limit, and
  `Process::pipe()` feeds each output to the next input. `Process::fake()`
  stops every process in a test, answers by pattern, `describe` and
  sequence, and records each run for the assertions.
- **An SQS queue driver.** `QUEUE_DRIVER=sqs` queues jobs on Amazon SQS
  standard queues, configured by Laravel's variables: `SQS_PREFIX`,
  `SQS_QUEUE` and `SQS_SUFFIX` build the queue URL, `AWS_DEFAULT_REGION`
  (or `AWS_REGION`) names the region, and `AWS_ACCESS_KEY_ID` and
  `AWS_SECRET_ACCESS_KEY` sign the requests, or the default credential
  chain of AWS when they are not set. `SQS_ENDPOINT` points it at
  LocalStack or ElasticMQ, and `QUEUE_CONNECTIONS` and failover accept
  `sqs`. A job goes to the queue it names, and a worker receives from the
  queues `--queue` lists, or from `SQS_QUEUE`. A `nack` counts an attempt
  and a release does not, as on the other drivers, and a delay longer than
  the 15 minutes SQS allows still holds. With `SQS_OVERFLOW_ENABLED=true`, a
  job of 1 MiB or more is stored on a disk and SQS carries a pointer to it.
  The boot fails with no region, with a queue name and no `SQS_PREFIX`, and
  for a FIFO queue. A receive waits up to `SQS_WAIT_TIME_SECONDS` (default
  1) for a message, a throttled or failed request is tried up to three times,
  and `SqsConfig` with `SqsQueueDriver::new` builds a driver in code for a
  second connection. `SqsQueueDriver::call` sends any other SQS action. The
  driver is behind the `queue-sqs` feature, on by default.
- **A default disk.** `Storage::default_disk()` returns the disk
  `Storage::set_default_disk` names in code, or else `FILESYSTEM_DISK`.
  When the named disk is not registered once the bootstrap has run,
  `filesystem::bootstrap_from_env` fails and the server does not boot.
  With nothing named there is no default, and `default_disk()` returns an
  error that names `FILESYSTEM_DISK`. Calls that name their disk are
  unchanged. **What to check when you upgrade:** `FILESYSTEM_DISK` was not
  read before, so an application that carries `FILESYSTEM_DISK=local` from a
  Laravel `.env` now refuses to boot unless it registers a disk named
  `local`: register it, or remove the variable. An application that calls
  `filesystem::bootstrap_from_env` in its own bootstrap calls it after it
  registers its disks and sets the default.
- **A failed request's error report reaches `TestResponse`.** A
  response the framework builds from an error carries an `ErrorReport`
  in its in-process extensions: the error and each of its sources, or,
  for a panic the panic boundary caught, the panic message and the
  location it was raised at. That covers a `FrameworkError` a handler or
  middleware returns, a panic on an HTTP route or in a WebSocket
  upgrade's middleware, the 5xx the framework's session, session lock,
  throttle, rate limit, login throttle, timeout, and RenderCache
  middleware answer a failure with, and the 5xx of the payment webhook
  route and of the Live endpoints when the endpoint holds the error. The report never reaches a header or the body,
  so what a client receives is unchanged; with debug on, a 5xx body
  still carries `debug_message` as before. The new
  `TestResponse::from_response` builds from the response
  `handle_request` returns and keeps the report, and every failing
  assertion then ends with an `error report:` section, so a 500 says
  why it happened. `assert_inertia()` and the `AssertableInertia`
  assertions print it too, and the Inertia error page and validation
  redirect keep the report of the response they replace.
  `TestResponse::error_report` and
  `HttpResponse::error_report` expose it. The report belongs to its own
  response, so two requests in flight in one test process never mix
  their errors. To find a panic's location, the framework wraps the
  process panic hook once and calls the hook it replaced. This landed on
  main after the `v3.0.0` tag.
- **A development error page.** With debug on, a 5xx the framework
  built from an error or a caught panic, sent to a browser (an `Accept`
  that lists `text/html`) or an Inertia visit, becomes one HTML document
  with the same status: the error chain or the panic message and
  location, the stack frames recorded where the error was created, and
  the request's method, path, query, headers, matched route pattern and
  request id. The application's frames are shown; the frames of the
  framework, the async runtime, other dependencies and the standard
  library are collapsed behind a count. The `FrameworkError` and
  `AppError` constructors, the `From` conversions into `FrameworkError`,
  the `abort_with`, `abort_if` and `abort_unless` helpers and the
  `#[domain_error]` conversions are now `#[track_caller]`, so the page
  names the line that called them even in a build without debug info; a
  call from inside `Result::map_err` names that line of the toolchain,
  and the frames name the handler. Without debug info, the handler and
  middleware the framework's dispatch polls are the application's
  frames. The page redacts the `Authorization`, `Proxy-Authorization`,
  `Cookie` and `Set-Cookie` headers, every header and query parameter
  whose name contains `token`, `secret`, `password`, `key` or
  `signature`, every such parameter in any other shown value (a
  `Referer` or `X-Original-URI` that repeats the URL), and the password
  of a URL, and it never shows the request body, environment variables
  or configuration values. It loads nothing and runs no script, so it
  renders when the frontend build, the Vite manifest or Inertia is what
  failed, and it carries `Cache-Control: no-store` and a
  `Content-Security-Policy` that allows no script. For
  those responses it takes the place of the app's Inertia error page. A
  JSON client keeps the JSON body with `debug_message`, a 4xx and a 5xx
  built without an error are unchanged, and with debug off the framework
  records no frames, captures nothing about the request, and sends every
  response as before. This landed on main after the `v3.0.0` tag.
- **A Pusher-protocol broadcast driver.** `PusherBroadcastHub` publishes
  broadcasts through the REST API that Pusher Channels, Soketi and Laravel
  Reverb share, and still delivers them to in-process subscribers, so a
  `ws!` broadcasting route keeps working. `PusherConfig::from_env()` reads
  Laravel's `PUSHER_*` variables, and `from_env_prefix("REVERB")` reads
  Reverb's. `pusher_channel_auth` and `pusher_user_auth` answer Laravel
  Echo's authorization requests for private, presence and end-to-end
  encrypted channels. The new `Channel::visibility` picks each channel's
  Pusher name and defaults to private, so a channel that restricts
  `authorize` can never leak through a public Pusher channel. A name that
  would read as another channel's Pusher name, and an encrypted presence
  channel, are refused rather than published, and an invalid configuration
  fails when the hub is built. Publishing to an encrypted channel without a
  master key is an error, never a plaintext send. This landed on main after
  the `v3.0.0` tag (#130).
- **`InertiaProps` and `Data` follow serde's field names.** Both derives
  honor `#[serde(rename_all = "...")]` on the struct, and `rename`, `skip`,
  `skip_serializing` and `skip_deserializing` on a field, including the
  split `serialize = .., deserialize = ..` forms; any other serde attribute
  is a compile error. The names reach everything the client sees: Inertia
  props and partial reloads, the `?include=` allowlist, JSON:API members,
  request input, route-parameter injection, and `suprnova generate-types`.
  Validation errors of a `Data`, `#[derive(FormRequest)]` or `#[request]`
  struct are keyed by the input names at every nested level, so
  `Precognition-Validate-Only` matches what the client sent, and a message
  labels `unitPrice` as "unit price", as Laravel does. A raw identifier
  such as `r#type` is sent and read as `type`. A plain nested
  `#[derive(Deserialize, Validate)]` struct registers its names with the
  new `#[derive(suprnova::InputNames)]`. This landed on main after the
  `v3.0.0` tag (#124).

  Two consequences for existing code. A `#[serde(...)]` attribute on an
  `InertiaProps` or `Data` struct used to compile only next to another
  derive that registers `serde` (such as `schemars`), and did nothing
  there; it now takes effect, and an attribute the derive does not apply
  is a compile error. A localized validation message labels every key
  snake-cased, so a hand-written key `userID` reads "user i d", as in
  Laravel; snake_case keys read as before. A `#[json_resource]` struct may
  no longer send an attribute or relationship named `type` or `id`, which
  JSON:API reserves: a struct with a custom `id_field` and a field named
  `id`, or a field `r#type` (sent as `r#type` before), is now a compile
  error that asks for a `#[serde(rename = "...")]`.
- **The schema builder covers the rest of a Laravel migration.**
  `unsigned_id()` and `unsigned_foreign_id()` create Laravel's `BIGINT
  UNSIGNED` keys on MySQL, so a new table can point a foreign key at an
  existing Laravel table, and a model reads them with `key_type = "u64"`.
  New columns: `tiny_integer`, the `unsigned_` integers, `medium_text`,
  `long_text`, `enumeration` (Laravel's `enum`) and `remember_token`. New
  modifiers: `.unsigned()`, `.index()`, `.primary()`, `.precision(n)`,
  `.use_current()` and `.after(column)`. `t.primary(&[..])` makes a
  composite primary key, `t.foreign(column)` a foreign key on a column
  declared on its own, `.name(..)` names a key, and
  `drop_constrained_foreign_id` drops a key with its column. The actions have
  Laravel's shorthands, `cascade_on_delete()` and the rest. Where the
  databases differ the builder does what Laravel does: `unsigned` and
  `after` apply on MySQL only, and an enumeration is a string with a `CHECK`
  off MySQL. This landed on main after the `v3.0.0` tag (#122).
- **Native date-time columns for models.** `AsNativeDateTime` and
  `AsOptionalNativeDateTime` store a `DateTime<Utc>` in a column that keeps
  the zone (`timestamp with time zone` on Postgres, `TIMESTAMP` or
  `DATETIME` on MySQL); `AsNaiveDateTime` and `AsOptionalNaiveDateTime`
  store the UTC wall clock in one without a zone, the shape Laravel's
  `timestamps()` creates on Postgres. The schema builder gains Laravel's
  `timestamps_tz()`, `soft_deletes_tz()`, `datetimes()` and
  `soft_deletes_datetime()` to create them. Automatic timestamps, `touch()`,
  soft deletes and the touch of an owner all store through the declared
  cast, and query comparisons (`filter`, `where_between`, `where_date`,
  `update_all`, `where_has`) bind through it, so Postgres gets a native
  parameter. A model may declare `created_at` and `updated_at` as
  `Option<DateTime<Utc>>` for a table another application leaves NULL.
  `suprnova generate-types` now emits chrono's date and time types as
  `string` instead of `unknown`. This landed on main after the `v3.0.0` tag.
- **Laravel's `exists`, `accepted`, `digits`, `date_format`, date
  comparison, `prohibited`, `missing` and `exclude_if` rules.** `Exists`
  checks that a value names a row, scoped with `where_eq`; `check_value`
  binds a typed id, which Postgres needs for an integer column, and
  `check_each` checks every element of an array, one query per distinct
  value, with each failure under `field.<index>`. `Accepted`, `Digits`,
  `DateFormat` (chrono's format syntax, checked by a round trip like
  Laravel's),
  `After::new`, `AfterOrEqual::new`, `Before::new` and `BeforeOrEqual::new`
  (against a fixed date, `Now`, `Today`, `Tomorrow`, `Yesterday` or another
  field, with `.format(..)` for a field that is not ISO 8601), `Prohibited`,
  `Missing`, `ExcludeIf` and `ExcludeUnless` join the built-in rules.
  `validate!` now runs a row's rules in order, each expression evaluated
  once, and stops at an exclusion, as Laravel does. This landed on main
  after the `v3.0.0` tag.
- **A Data Object runs `validate!` and database rules.** The derive writes
  a Data Object's `FormRequest` impl, so it had no place for cross-field
  rules or for `Unique` and `Exists`. `#[data(after_validation = "fn")]`
  and `#[data(after_validation_async = "fn")]` name the functions that
  impl calls, and the impl a `from_route_param` field selects now runs the
  async stage too. That impl now calls the hooks through the trait, so an
  inherent `after_validation` method on such a Data Object, which it used
  to call by accident, no longer runs: name it with the attribute. The
  derive refuses both attributes on a struct that gets no `FormRequest`
  impl. This landed on main after the `v3.0.0` tag.

- **Joins and the remaining `where` helpers on both query builders.**
  `DB::table` and model queries gain `join`, `left_join`, `right_join` and
  `cross_join` with table aliases; closure conditions combining `on`,
  `or_on`, `where` and `or_where`; `join_sub` and `left_join_sub` against
  another builder; `where_exists` and `where_not_exists`; `where_any`,
  `where_all` and `where_none` with their `or_` forms; subqueries in
  `where_in` and `where_not_in`; the `or_where_*` family; and `reorder`.
  Every value stays bound and every identifier is quoted for the backend,
  so reports and exports no longer need raw SQL. A joined model query
  selects its own table's columns, so the joined table's `id` never
  overwrites the model's, and its soft-delete filter names its table.
  `DB::table` now quotes identifiers in every statement, so on Postgres a
  mixed-case name is case-sensitive, as in Laravel. An update or delete on
  a query with a join is refused rather than run without the join. This
  landed on main after the `v3.0.0` tag (#125, #128).
- **File and download responses.** `HttpResponse::file` shows a file
  inline, `HttpResponse::download` sends it as an attachment, and
  `HttpResponse::download_bytes` sends generated bytes; `Storage::response`
  and `Storage::download` do the same for a path on a named disk, through
  the disk's path guard. Files stream, the content type comes from the
  extension, a missing file answers 404 and a path the guard refuses 403.
  Every `Content-Disposition` follows RFC 6266, with an ASCII fallback
  transliterated as Laravel does and a `filename*` for the full name, so
  `Certificat·Joan Pérez.pdf` arrives intact and no name can inject a
  header. This landed on main after the `v3.0.0` tag (#126).
- **A configurable Inertia page lookup.** `inertia_response!` reads an
  optional `[package.metadata.suprnova.inertia]` table from the
  application's `Cargo.toml`: `pages_dir` moves the pages directory, and
  `page_file` maps a component name to its file with `{dir}`, `{name}` and
  the `lower`, `kebab` and `snake` filters. Angular and other layouts keep
  the compile-time page check, and a missing page names the exact path the
  macro looked for. Without the table nothing changes. This landed on main
  after the `v3.0.0` tag (#129).
- **The queue fake can let jobs through and records raw pushes.**
  `Queue::fake_except` and `QueueFakeGuard::except` record every job but
  the named ones, which reach the real queue. `Queue::push_raw` pushes an
  envelope's JSON form, and under the fake `raw_pushes` and `pushed_raw`
  read those pushes back. `assert_pushed_without_chain` fails for a job
  pushed with a chain. A chain whose first job `except` names runs on the
  real queue, and each later link is recorded unless `except` names it
  too. This landed on main after the `v3.0.0` tag.

- **Declarative authorization on handlers.** `#[authorize("update-post",
  post)]` on a `#[handler]` checks the gate against the model the route
  binds to `post`, and `#[authorize("create-post", Post)]` checks it
  against the type, for the user of the guard the route authenticated
  with. The check runs after route model binding and before
  the request body is read, so a forgotten `Gate::authorize` call can no
  longer leave a route open. Policies, async gates and the RBAC gate bridge
  all answer it. A guest gets 401, a denial 403, and `deny_as_not_found()`
  404. A parameter name the handler does not take fails to compile. This
  landed on main after the `v3.0.0` tag (#127).

- **Model changes after a save, `sync_without_detaching`, and after-commit
  callbacks.** A model reports what its last update changed
  (`was_changed`, `was_changed_any`, `get_changes`), and `get_original` and
  `get_raw_original` return the loaded values while its `updated` and
  `saved` observers run, as Laravel's do; once the save returns, the
  original is the saved row. An encrypted column counts as changed only
  when its decrypted value changed. `sync_without_detaching` attaches pivot rows
  without touching the existing ones. `DB::after_commit` runs a callback
  after the ambient `DB::transaction` commits, and never on rollback; a
  transaction started with `DB::begin_transaction` takes its own
  `tx.after_commit` and `Queue::push_after_commit_with_tx`, because ambient
  code is outside that transaction. This landed on main after the `v3.0.0`
  tag (#128).
- **OAuth identities carry the account's picture.** `OAuthIdentity` has
  `avatar_url`: the picture URL the provider reports, or none. Google
  fills it from `picture`, TikTok from `avatar_url`, Facebook from
  `picture.data.url` and X from `profile_image_url`; Apple reports none,
  and an empty value, or one of an unexpected shape, is none and never
  fails sign-in. Treat it as untrusted profile data, and check
  its scheme and length before you render, fetch or store it. A provider
  supplies it through `OAuthProvider::avatar_url`, which returns none
  unless the provider implements it, so a provider written before it
  compiles and signs in unchanged. This landed after the `v3.1.0` tag
  (#140).
- **An asynchronous multipart hook.** `MultipartRequestHooks` has
  `after_validation_async`, for checks that need the database before the
  handler runs. A multipart request runs `authorize` before it reads the
  body, then extraction with its field validation, `after_validation`,
  `after_validation_async` and the handler, each only after the one before
  succeeded. A hook that returns errors answers 422 with them, and an empty
  set of errors is success. An override needs `#[async_trait]`. This
  landed after the `v3.1.0` tag (#139).
- **Unsigned keys on every database.** A model field declared `u64` or
  `Option<u64>`, its key and foreign keys included, reads and writes on
  SQLite, Postgres and MySQL through the new `AsU64` and `AsOptionalU64`
  casts: the whole range on MySQL's unsigned columns, and `0` to
  `i64::MAX` on SQLite and Postgres, which store it signed. There a write
  of a larger value is refused before anything is sent, as a database
  error that names the column in the log, and a negative stored value
  fails to read the same way. A read by such a value answers what is true
  of every row: `find` returns none, so a model bound from a route answers
  404, `find_many` skips the id, and a filter on the key or any narrower
  integer field matches as it would on MySQL. `DB::table` compares and
  writes such a value the same way. A `u64`-keyed model binds from a
  route, `chunk_by_id` and `lazy_by_id` walk the whole `u64` range on
  MySQL, and the built-in user providers sign in a user whose id is a
  `u64`. This landed after the `v3.1.0` tag (#137).
- **Settings that match Laravel's schema.** In a package's `Cargo.toml`,
  `[package.metadata.suprnova.model] datetime_cast = "native"` makes every
  `DateTime<Utc>` field of that package's models without a cast of its
  own, the managed timestamps included, use `AsNativeDateTime`, for a
  time-zone-aware column, and `"naive"` uses `AsNaiveDateTime`, for the
  time-zone-free columns Laravel creates on Postgres. In the binary's
  package, `[package.metadata.suprnova.schema] unsigned_ids = true` makes
  `id()` and `foreign_id()` create unsigned columns on MySQL in every
  migration the binary runs, and changes nothing on Postgres and SQLite;
  `#[suprnova::main]` installs it, and `Schema::use_unsigned_ids()` does
  the same for a program without it. An unknown key or value fails the
  build. New applications carry both settings commented out. This landed
  after the `v3.1.0` tag (#137).

### Changed

- **Empty form fields are null, and a repeated name keeps its last value.**
  In a `MultipartRequest` and in url-encoded `FormRequest`, `req.form()`,
  `req.input()` and `Request::query_into`, an empty field is now a key
  holding null, as Laravel's default `ConvertEmptyStringsToNull` leaves it:
  a required `String` fails, an `Option` is `None`, a `#[serde(default)]`
  field is null, and a map or `serde_json::Value` keeps the key, so it can
  tell a cleared field from one never sent. An empty value used to pass as
  `""` or fail to parse as a number. A `HashMap<String, String>` target now
  fails on an empty field; use `Option<String>` values or
  `serde_json::Value`. A list keeps a null element in its place: `None` in
  a `Vec<Option<T>>`, and `validation-required` under its index, such as
  `ids.1`, in a `Vec<T>`. `#[derive(MultipartRequest)]` accepts
  `Vec<Option<T>>`, and a list of files still leaves out an empty file
  input. A field name sent more than once keeps its last value, where
  multipart kept the first part (even an empty one) and url-encoded forms
  answered 422 for a duplicate field; names ending in `[]` still collect
  every value. A url-encoded or query `bool` reads `1`/`0`, `true`/`false`
  and `on`/`off` in any case, as a multipart one does; JSON stays strict
  `true`/`false`. A url-encoded, JSON or query field that is missing or
  does not parse answers as a validation failure naming every such field
  (`validation-required`, `validation-integer` and the like), where it was
  a bare 422 with no `errors`, so an Inertia form gets the usual redirect
  with each error under its input. CSRF checks the form's `_token` (its
  last value) before `X-CSRF-TOKEN` and `X-XSRF-TOKEN`, as Laravel orders
  them, and an empty or `0` value counts as absent; a form body larger than
  64 KiB has its `_token` read up to the server's request body limit, where
  it used to fail CSRF. The requests and CSRF manuals list what still
  differs from Laravel. This landed after the `v3.1.0` tag.
- **The session, remember-me, auth-flow token and ceremony entities read
  whole rows on every column type.** Their time fields are the new public
  `suprnova::StoredDateTime`, which reads `DATETIME`, `TIMESTAMP`,
  `timestamp`, `timestamptz` and SQLite text, instead of `NaiveDateTime`,
  which failed on `TIMESTAMP` and `timestamptz`. Set them with `.into()`
  and read them with `.naive_utc()` or `.and_utc()`. This landed after the
  `v3.1.0` tag.
- **Magnetar rotations and second-factor lockouts.** Magnetar's
  `re_enroll` keeps the confirmed second factor gating sign-in until a code
  from the new secret confirms the rotation; the rotation waits in new
  `auth_two_factor` columns, `pending_secret` and `pending_recovery_codes`,
  which `default_schema::migrate` adds. `TwoFactorRow` gains those two
  fields, and a custom `TwoFactorStore` implements the new
  `confirm_rotation`. A second-factor lock or unlock never touches an
  `app_users` row (`LockoutFields::IDENTITY_IS_EMAIL`,
  `LockoutService::without_user_lock`), so an account registered as
  `two-factor:{id}` is never locked or unlocked by it. An account holds the
  framework's TOTP or a Magnetar second factor, never both: enrolling or
  confirming either answers 409 while the other exists, and disabling
  either one recovers an account that already has both. A custom host
  attaches `FrameworkTotpEnrollment` to its `TwoFactorService` through
  `with_other_second_factor`. This landed after the `v3.1.0` tag.
- **Live field and argument names must be ASCII and at most 128 bytes**,
  and a view-visible field named `component` is a compile error. Such
  names used to panic at registration or fail later. This landed after the
  `v3.1.0` tag.
- **Cron steps count from the first value.** `*/N` in the day-of-month and
  month fields counts from 1, as cron and Laravel do, so `*/2` means odd
  days and schedules using it shift; the timezone display now agrees with
  the scheduler. Ranges with steps (`1-15/7`) and mixed lists parse. This
  landed after the `v3.1.0` tag.
- **Workflow steps are named by module path and function name**, so
  same-named steps in different modules are different steps. Runs recorded
  with bare names still replay. This landed after the `v3.1.0` tag.
- **`TestContainerGuard` can no longer be built directly**; use
  `TestContainer::fake()`. `dispatch_argv_with_init` boots the process and
  waits for queued listeners after the command; `dispatch_argv` does
  neither. This landed after the `v3.1.0` tag.
- **`save` and `update` refuse a model that was never inserted.** A
  replica, or a new model from `first_or_new` or `find_or_new`, still has
  its reset key, and `save` updated the row with key 0. It now returns an
  error naming `persist()`, which inserts the model and returns it with its
  new key. Laravel's `save` inserts such a model; `save` here takes `&self`
  and cannot hand the new key back, which the Eloquent manual explains. This
  landed after the `v3.1.0` tag.
- **The broadcast fanout no longer uses sea-streamer.**
  `SeaStreamerBroadcastHub` keeps its name and runs on the framework's own
  Redis client, and `sea-streamer` and `sea-streamer-redis` left the
  dependency tree. It supports `redis://`, `rediss://` and `memory://`, a
  stream shared by hubs in one process; `stdio://` is an alias for
  `memory://` and no longer reads or writes stdin and stdout. `kafka://`
  and `file://`, which were never compiled in, now give a clear error, and
  `new_loopback` behaves like `new`. An event published after the
  constructor returns always arrives. Stream entries keep their `msg`
  field, so hubs of the earlier version on the same stream interoperate
  during a rolling deploy. This landed after the `v3.1.0` tag.
- **New applications' session, remember-me and auth-flow token tables use
  `DATETIME` on MySQL**, so these columns stay writable past 2038-01-19.
  Postgres and SQLite are unchanged. This landed after the `v3.1.0` tag.
- **Two-factor needs two new migrations.** Add
  `suprnova::auth_flows::two_factor::migration_attempts` (the
  `two_factor_attempts` table) and `migration_rotation` (the
  `two_factor_rotations` table) to your migrator. Without the first, every
  two-factor proof path answers 503 and names the migration; without the
  second, `TwoFactor::re_enroll` answers 503. A pending rotation keeps the
  confirmed secret gating sign-in until `confirm` promotes the new one,
  `enroll` cannot replace a pending rotation, and `disable` discards it.
  `enroll` answers 422 for a user id longer than 255 characters. This
  landed after the `v3.1.0` tag.
- **`Auth::password().register` returns a `Registration`**:
  `Created(user)` or `Accepted`, never the existing account. The
  `MagnetarPasswordAuthEngine::password_register` trait method returns it, and a
  custom engine gains `admit_host_sign_in` and `issue_host_session`, which
  refuse with 503 by default. The API starter answers every registration
  with one generic 202. This landed after the `v3.1.0` tag.
- **Magnetar's second-factor lockouts have their own table.** The default
  schema's migrate creates `auth_second_factor_lockouts`
  (`DefaultSecondFactorSchema`), and `TwoFactorService::new` takes a
  lockout service over it. `TwoFactorStore::set_confirmed` takes the
  enrollment snapshot that was checked, and the MySQL migration failures
  (`SwapFailure`, `MySqlMigrationFailure`) are boxed. This landed after the
  `v3.1.0` tag.
- **`Cache::tags_put` without a TTL applies `CACHE_DEFAULT_TTL`**, as
  `Cache::put` does. A tagged value that must never expire uses the new
  `Cache::tags_forever`. This landed after the `v3.1.0` tag.
- **`DiskExt::temporary_upload_url` returns a `TemporaryUploadUrl`** with
  `url`, `method` and `headers`, instead of a `String`. An S3 disk with
  server-side encryption signs headers such as
  `x-amz-server-side-encryption`, and an upload that did not send them was
  refused. Use `.url` and send `.headers` with the upload. Its `Debug`
  output redacts the signature and header values. This landed after the
  `v3.1.0` tag.
- **`Queue::bulk` honours debounce.** Every copy of a debounced job pushed
  through `Queue::bulk` used to run. A debounced burst now collapses onto
  its last job, and a job declaring both `debounce_for` and `unique_id` is
  refused, as `Queue::push` does. Laravel's `bulk` skips debounce; the
  queues manual explains the difference. This landed after the `v3.1.0`
  tag.
- **Multipart failures answer as validation errors under the field's
  input name.** A missing field, a text part that does not parse as its
  type or is not UTF-8, a part of the wrong kind, and a file a validator
  refuses (too large, not an image, a type `MimeType` does not allow)
  answer 422 with
  `errors` under the form input name, so the second file of a `files[]`
  field is `files.1`, and the Inertia validation middleware shows each
  error under its field. Before, a text part that did not parse answered
  400, a file over `MaxSize` 413, and the others 422 without `errors`. The
  messages come from the validation catalog (`validation-required`,
  `validation-max-file` and the rest), so an application's
  `lang/<locale>/validation.ftl` overrides them. A file over `MaxSize`
  stops the body at the chunk that crossed the limit and leaves no
  temporary file. A field's `max_count` answers 413 with the other
  request-wide limits, instead of 422, and so does a text part longer than
  the in-memory spill threshold (2 MiB by default), instead of 400, as
  Laravel answers `post_max_size`. A file validator checks only the file
  parts its field takes: a text part sent where a file belongs fails as
  `validation-file`, and a later part for a field that holds one file is
  ignored without being read into memory or a temporary file. For a field
  that holds one value, its first part decides it, valid or not, so
  several failing parts report one error; an empty part does not decide a
  field that is not a `String`, so the next part of its name can. An `UploadValidator` returns
  `FrameworkError::invalid_upload` for a validation failure, and its other
  errors keep their status. A `bool` field accepts `1`, `0`, `true`,
  `false`, `on` and `off`. An empty part, which Inertia sends for `null`,
  is none for an optional field that is not a `String` and missing for a
  required one, and a part of the wrong kind sent to an optional or list
  field fails instead of being ignored. `MultipartValue`, which the
  streaming parsers return, has a new `NonUtf8Text` variant for a text part
  that is not UTF-8, so a `match` over it needs an arm for it. This landed
  after the `v3.1.0` tag (#139).
- **`#[model]` takes the key type from the key field.** Without
  `key_type`, the key type is the type of the field `primary_key` names;
  it was `i64` whatever the field said. A `key_type` that disagrees with
  that field fails to compile, naming both. This landed after the `v3.1.0`
  tag (#137).
- **`pluck`, `value` and the aggregates read every key type, and say why a
  value does not read.** `pluck`, `pluck_keyed`, `value`, `value_or_fail`,
  `sole_value`, `sum`, `min`, `max` and `DB::scalar` take any type that
  implements the new `ColumnValue` trait, which every type SeaORM reads
  already does, and `u64`, which they now read on SQLite and Postgres
  too. A generic caller bound by `TryGetable` needs `ColumnValue`
  instead. `avg` reads an `f64` or a `rust_decimal::Decimal` (the new
  `AvgValue` trait); another type no longer compiles. Postgres and MySQL
  average exactly, so a `Decimal` average is exact there. SQLite averages
  as a REAL, so there the `Decimal` holds the shortest decimal that
  round-trips SQLite's floating-point answer. `pluck`, `pluck_keyed` and `value` used to drop a row whose
  value did not decode, so `pluck::<u64>` returned an empty list; they
  still skip a NULL, and any other value that does not decode is an error
  naming the column. This landed after the `v3.1.0` tag (#137).
- **The combobox submits the chosen option's value.** A selection used to
  write the option's label into the one bound field, so the value each
  option carries never reached the server. `suprnova.combobox` now binds
  two fields: `name` is the chosen option's value, held by a hidden input
  that a selection fills and an edit of the text clears, and
  `<name>_query` is the typed text, which a remote listbox answers. The
  macro takes the island's value as `value`, and the script marks that
  option selected. Bind the query field where you bound `name` before, and
  read the choice from `name`. This landed after the `v3.1.0` tag.
- **Live's limits are settings sized for large pages.** Every limit Live
  applies is a `LIVE_*` key in the application's `.env` file or a
  `LiveConfig::builder()` method, and a value outside a key's range fails
  boot with an error that names the key. The browser runtime had smaller
  limits of its own: it refused an island render over 32 KiB and a morph
  past 10,000 nodes or one second while the server allowed 1 MiB, and an
  action whose island root passed 1 MiB failed. The bootstrap now writes
  every limit into the page's configuration element and the browser applies
  those values, so it never refuses what the server allows. Requests,
  responses and island HTML default to 16 MiB; a morph takes up to
  1,000,000 nodes and keys with no deadline; an upload sends 8 MiB chunks
  of a file up to 1 GiB; an open document holds and replays up to 4,096
  asynchronous events; and a session streams to 64 open tabs, where it was
  8. A replay log holds up to 4 MiB per subscription and 256 MiB across the
  process, dropping its oldest entries first, and a reconnect that needed a
  dropped entry renders fresh. A tripped limit's message names the limit,
  the measured and configured values and the key, in the browser console
  and in the server log. `suprnova live:inspect` prints every limit by its
  key, and the Live chapter lists them with their ranges. `live:check` and
  `live:inspect` speak tooling protocol 2 and fall back to protocol 1 with
  an application built before it. This landed after the `v3.1.0` tag.
- **Less memory for the same work.** Model events are built only when a
  listener, a fake or a deferral will see them, so a query no longer
  copies every row it reads for a `Retrieved` event nobody hears. Eager
  loading reads each key from its field instead of serializing the whole
  row, and a runtime cast converts its column where it sits. The first
  Inertia page is written into one buffer, a file download reads each
  chunk straight into the bytes it sends, a body sent in several frames
  is held at its length, CORS path patterns are compiled once per
  configuration, and a broadcast's last channel takes the payload instead
  of a copy. A queue job receives its envelope's payload, the SQS driver
  parses a message once, sends a retried request's bytes without copying
  them and holds a reservation's body rather than a second envelope.
  `Context::push`, image transformations and encoding, filesystem
  `append` and `prepend`, `Collection::pluck`, the in-memory vector search,
  the feature-flag snapshot, Mailgun's form, composite render-cache shells
  and the docs builder each keep or copy less. A process result holds its
  output once when the output is valid UTF-8. In the Live engine a mount
  no longer copies its signed snapshot, action arguments are kept once,
  a full upload read returns the store's bytes, and the resource queue
  compacts in place. The Live browser runtime reads a server-sent event
  stream without copying what it already holds for every chunk, and an
  upload control response no longer allocates 16 KiB. Magnetar writes hex
  into one buffer, and its single-flight map holds a key only while a
  caller holds or waits on it. Every response, file, digest and snapshot
  is byte for byte what it was.
- **`CacheConfig` has a `sweep_interval` field.** Code that builds a
  `CacheConfig` as a struct literal has to set it, or use
  `CacheConfig::builder()`, which defaults it to 60.
- **`DynCast` has a `from_storage_json_owned` method** that converts a
  stored value the caller no longer needs. Its default calls
  `from_storage_json`; a cast whose in-memory value is the stored value
  returns it without copying.

- **Framework payments, features and RBAC models carry `DateTime<Utc>`
  timestamps.** The SeaORM `Model` of the six payments entities, of
  `features::entity`, and of the RBAC `Role` and `Permission` carries
  `DateTime<Utc>` (`Option<DateTime<Utc>>` for `canceled_at`, `paid_at` and
  `processed_at`) where it carried `String`, because their migrations
  create native timestamp columns. Code that read those fields as text
  needs the date-time type. This landed after the `v3.1.0` tag.
- **A mail message is checked on the dispatch path, and a refusal is a
  500.** `Mail::send`, `Mail::raw`, `Mail::html`, the queue worker and the
  notification mail channel check the message before `MessageSending`
  fires and before any transport sees it, including a transport bound with
  `Mail::set_transport` and the one behind `Mail::fake`. `Mail::queue` and
  `Mail::later` check it when they push. A refusal is an internal error, a
  500 with the detail in the log, where it was a 400 that echoed the
  address. Headers the message structure owns (`To`, `Cc`, `Bcc`, `From`,
  `Sender`, `Reply-To`, `Return-Path`, `Subject`, `Date`, `Message-ID`,
  `MIME-Version` and `Content-*`) are refused as caller headers. Postmark
  refuses a second tag, and SES refuses a tag or metadata it cannot carry,
  instead of dropping them. The address and header serializer is public as
  `suprnova::mail_wire` for transport authors. This landed after the
  `v3.1.0` tag.
- **The framework registers its own mail and notification jobs.** A
  worker dispatches `SendMailJob` and `SendNotificationJob` with no
  `register_job` line. Registering the same job type again is quiet; only
  a different type taking over a job name still warns. This landed after
  the `v3.1.0` tag.
- **A full Live instance ledger evicts instead of refusing mounts.** At
  `LIVE_LEDGER_MAX_INSTANCES`, a new mount or promotion removes the
  instance that expires soonest, preferring one with no claim in flight,
  and the evicted page's next action gets `refresh_required` and reloads
  fresh. Before, a full ledger refused every mount on the site until
  records expired, and anyone who could mount in a loop could keep
  everyone else out. `CapacityExceeded` now means only a record over a
  codec bound. A custom `InstanceRecordStore` implements two new methods,
  `soonest_expiring_instances` and `compare_and_remove`; the memory, SQL
  and Redis stores ship them. This landed after the `v3.1.0` tag.
- **Live form controls match the density of the suprnova.app forms.**
  Controls read at 14px through `--sn-density-control-font-size` (16px on
  a coarse pointer), fields are 40px tall, labels are 12px and sit 6px
  above their control, legends are 14px, and hints and errors are 12px. A
  textarea shows three rows and is at least two control heights tall.
  `--sn-density-field-gap` puts 12px between fields, checkboxes, switches
  and groups, and a group's options sit 8px apart. To pick it up in an
  application, run `live:add` again for `button`, `checkbox`, `combobox`,
  `datatable`, `date-picker`, `field`, `fieldset`, `input-otp`, `label`,
  `load-more`, `pagination`, `radio-group`, `switch`, `textarea` and
  `upload`. This landed after the `v3.1.0` tag.

- **Content negotiation treats `q=0` as a refusal.** `accepts`,
  `accepts_json`, `prefers`, `wants_json`, `expects_json` and
  `acceptable_content_types` leave out an `Accept` type weighted `q=0`, as
  RFC 9110 says, even beside `*/*`; the most specific matching range
  decides. Laravel lists such types as acceptable. This landed after the
  `v3.1.0` tag.

- **Vendor HTTP clients no longer follow redirects, and SQS overflow
  payloads move.** Pinecone and the HTTP mail drivers treat a 3xx as an
  error. SQS overflow payloads live under
  `sqs-payloads/<queue>-<digest>/`, so `SQS_OVERFLOW_FLUSH_ON_CLEAR` no
  longer deletes a same-named queue's payloads from another account,
  region or endpoint; payloads written before still read, but `clear` no longer sweeps
  them. Qdrant ids `"01"`, `"+1"` and non-canonical UUID spellings no longer
  map to the point of `"1"` or the canonical UUID, so items stored under
  such ids must be written again. A caller's version 5 UUID id is hashed
  like any other string, so it can never name a point a derived id holds;
  items stored under v5 UUID ids must be written again too. Resend tags go out as `tag_<i>` name and
  value pairs, and a tag Resend cannot carry is refused before sending.
  `DynNotification` gains `as_any`, with a default. This landed after the
  `v3.1.0` tag.

- **`build_docs` refuses chapters that would overwrite each other.** Two
  chapters with the same file name, or one named `catalog.md`, now fail
  with the new `ContentError::DuplicateChapterSlug` or
  `ContentError::ReservedChapterSlug`, naming the files, so an exhaustive
  `match` on `ContentError` needs the two arms. The same file listed twice
  is still allowed. This landed after the `v3.1.0` tag.

- **`IMAGE_MAX_ALLOC_BYTES` bounds the whole decode, and defaults to 1
  GiB.** It used to bound only the decoded RGBA size, so a decoder could
  allocate several times the limit while it worked. It now covers every
  buffer a decode and its source read hold, measured per format (the cost
  table is in `manual/images.md`), and the default rose from 256 MiB to 1
  GiB so a 48-megapixel photo in any 8-bit format still decodes. JPEGs
  decode through `zune-jpeg` (pinned at `0.5.16-rc2`, the first release that
  decodes every Huffman and arithmetic sampling correctly); lossless JPEGs
  still decode through `oxideav-mjpeg` and are refused by name above 67
  million samples. This landed after the `v3.1.0` tag.

- **Guard names, verification links and the default auth schema.** A guard
  name other than the default session or token guard may not contain `:`,
  because a non-default guard's principal is now `<guard>:<id>`. A token
  guard other than the default no longer copies its user into the default
  `Auth` view, so `Auth::has_user()` stays false for it. Email verification
  links issued before this change are refused and must be sent again. The
  default auth schema enforces one account per email with a unique index,
  so a migration over an `app_users` table that already holds duplicate
  emails stops with an error until they are merged. New, with defaults:
  `TokenGuard::named` and `UserProvider::verification_email`. This landed
  after the `v3.1.0` tag.

### Fixed

- **Unsigned keys, relation min and max, and raw bindings on every
  engine.** `with_min` and `with_max` read 32- and 16-bit integer columns
  on Postgres, and dates, text and times on every database, without an
  error; `_min_of` and `_max_of` return `Some(None)` for a value that is not
  a number, and the new `<rel>_min_as::<T>()` and `<rel>_max_as::<T>()`
  read the value itself. `DB::table`, raw fragments and joined-table
  columns compare a `u64` above `i64::MAX` as the database does for the
  stored data, including a REAL held in an INTEGER column on SQLite, and
  `DB::table` writes store such a value exactly in numeric and text
  columns while integer columns, and NUMERIC columns on SQLite, refuse it.
  `attach`, `attach_with` (extras included), `detach` and `sync` bind pivot
  ids by their column's type: on Postgres and SQLite a `u64` above
  `i64::MAX` is refused, naming the column, before anything is sent; on
  MySQL `sync` no longer duplicates unsigned pivot ids, and
  `BelongsToMany` and `MorphToMany` `get()` load rows whose pivot column is
  unsigned, where they returned none. On Postgres, a raw fragment sent
  first with an integer and later with a value above `i64::MAX` no longer
  fails with "incorrect binary data format". Model `update_all` and
  `upsert` of such a value to a field that is neither an integer nor text
  follow the column, as `DB::table` writes do. This landed after the
  `v3.1.0` tag (#137).
- **Durations too long for a date are errors, not panics.** A workflow
  lease or retry backoff too long for a date is refused at boot:
  `WORKFLOW_LOCK_TIMEOUT_SECS` above 253402300799, or
  `WORKFLOW_RETRY_BACKOFF_SECS` times `WORKFLOW_MAX_ATTEMPTS` above it,
  stops the worker at start with an error naming the setting, and lease
  refreshes return an error for such a lease. `Queue::later` and its
  variants, a job's `delay()`, and requeues in the default, SQS and
  database drivers return an error naming the delay, where they panicked
  or, past a chrono duration's range, requeued with no delay at all. This
  landed after the `v3.1.0` tag.
- **A refused cookieless request stores no session.** `CsrfMiddleware`
  marks a new, untouched session for storage only when the response
  succeeds or redirects. An anonymous request that creates a session and is
  refused with 401, or any other 4xx or 5xx, gets no `XSRF-TOKEN` and
  writes no session row, so it no longer turns into a 500 when the session
  store is unavailable; a session that was loaded or changed keeps its
  token. A
  cookieless JSON or `HEAD` bootstrap that succeeds still gets its token
  with its session. `SessionMiddleware` marks every session that
  `SessionStore::read` returns as loaded from the store, so a custom store
  that builds its sessions with `SessionData::new` no longer gets a write
  on each successful request through `CsrfMiddleware`. This landed after
  the `v3.1.0` tag.
- **Live uploads, private responses and tooling.** Finalized uploads free
  their pending slots when finalization commits, so a session no longer
  runs out of upload capacity until restart, and are reclaimed when they
  expire. A finalization that failed or stalled is reclaimed at expiry,
  and an app without a finalizer leaves uploads Ready instead of stuck in
  Finalizing. Cleanup deletes only temporary and uncommitted bytes; bytes
  finalization committed as output are never deleted. Every RenderCache
  response without shared-cache permission, public pages without
  `s-maxage` and zero-slot composites included, is `private, no-cache`.
  Making room for a file-store publication reads only the entries it
  evicts. A Live tooling timeout kills every process the helper started,
  and Ctrl+C still reaches the helper. A Live action with `validate =
  "arguments"` and a typed `#[validate(action = ...)]` hook compiles
  without a component-level `#[validate]` hook. Generated route helpers
  handle optional parameters (`{id?}`) as `route()` does, leaving an empty
  one out of the URL. Schema dump and load reach Postgres over a Unix
  socket named in the URL host (`postgres://%2Frun%2Fpostgresql/db`). This
  landed after the `v3.1.0` tag.
- **Scaffolded apps on MySQL 8.4 and Postgres, and dates past 2038.** The
  auth-flow token table's hash is `VARCHAR(64)`; MySQL 8.4 refused the
  UNIQUE key on the old `TEXT` column (error 1170), so a new app's
  migration stopped at the fourth step and the app could not boot. Run
  `migrate` again on an app that stopped there. The `--api` starter's
  `app_users` time columns are `timestamp with time zone` on Postgres and
  `DATETIME` on MySQL, which its `User` model and Magnetar read; an
  existing API app on Postgres converts them with the `ALTER TABLE` in the
  CLI chapter. The notifications and RenderCache ledger migrations create
  `DATETIME`, so writes keep working after 2038-01-19 on MySQL; tables
  created before keep `TIMESTAMP` and still work. A remember-me, auth-flow
  token or ceremony lifetime too large for a date is an error instead of a
  panic. This landed after the `v3.1.0` tag.
- **Generated routes and types, Inertia props and JSON:API.**
  `generate-types --routes` applies `group!` path and name prefixes, gives
  each repeated-handler alias its own helper, percent-encodes path values
  like `route()`, and uses serde's input keys in request interfaces;
  helpers for a second route of one handler get a new params interface
  name. An SSR exclusion glob such as `**/foo/*` matches where `**` spans a
  repeated literal. The redirect back for an empty Inertia response keeps
  the handler's cookies, security headers and error report. Replacing a
  lazy prop drops its `?include=` gate, a later dotted prop wins over an
  earlier lazy parent, and `App::inertia_shared("users.0.name")` reads into
  shared lists. A JSON-style `inertia_response!` prop that fails to
  serialize returns an error instead of panicking. JSON:API documents no
  longer repeat primary resources in `included`, an empty collection
  refuses unknown includes with 400, a requested include always returns an
  `included` array, and error pointers are escaped per RFC 6901. `i128` and
  `u128` route-parameter fields in Data DTOs extract instead of answering
  422, and DTOs with route-parameter fields compile without a direct `url`
  dependency. Live route intents can target resource routes, and `route()`
  and `try_route()` fill a catch-all `{*rest}` by the name `rest`, keeping
  its slashes. Numeric `expect!` matchers fail on NaN and other unordered
  values. Schema dump and load accept every TLS parameter spelling the
  application's connection accepts and pass the Postgres password through
  `password=`. The `Idempotency::remember` example key includes the
  authenticated user. Live component names, views and action text that
  mention development crate names compile, action arguments named `target`
  or `request` no longer break generated code, and `#[session]` Live fields
  load from and persist to the visitor's session once the action's outcome
  is accepted (they need `SessionMiddleware`). This landed after the
  `v3.1.0` tag.
- **Workers, the console and process lifecycle.** Queue, schedule and
  workflow workers, the `queue:*` commands and console commands boot the
  `#[injectable]` and `#[service]` inventory, so a job or command that
  resolves an action no longer fails with `ServiceNotFound`. The console
  also boots the runtime drivers and `#[policy]` gates, warns on stderr and
  goes on when a driver cannot boot, and waits for queued listeners before
  it exits. Workers, the queue and maintenance commands and console
  commands cancel and drain the supervisors the bootstrap started before
  they exit, with the same 5-second grace as `serve`; `down`, `up` and `schedule:list` run the application's
  bootstrap hook. `schedule:work` stops on SIGTERM while an inline task
  runs, stopping a task still running after the 30-second grace. A
  panicking `Terminable` hook no longer skips the hooks after it, and a
  graceful shutdown waits up to 5 seconds for hooks still running.
  `Context::get` and `Context::hidden_get` no longer deadlock when a custom
  `Deserialize` writes to the context. A `TestContainerGuard` or
  `TestQueryGuard` dropped on another thread clears only what it installed.
  Binding an `#[injectable]` by hand before boot no longer requires the
  dependencies only its generated constructor reads. `db:seed
  --class=<Name>` fails with not-found on an empty registry, and a
  poisoned registry fails instead of reporting nothing to run. A
  supervisor spawned during or after shutdown no longer starts outside the
  drain. A second `init_telemetry` while the first guard lives no longer
  takes over the global meter provider. `start_workflow!` accepts
  imported, re-exported, `crate::`, `self::` and `super::` paths, and
  `workflow:work` on a database other than Postgres exits with an error at
  startup instead of retrying forever. This landed after the `v3.1.0` tag.
- **Relations and soft deletes.** `destroy`, `delete_quietly`,
  `delete_or_fail` and trait-dispatched `delete` permanently deleted
  soft-delete rows; they now tombstone them, and `delete_or_fail` on an
  already trashed row is a 404. `with_count` and the `with_sum` family
  counted trashed rows and rows hidden by global scopes; they now cover
  exactly the rows `with` loads. `with([..])` and `with_count([..])` on one
  relation no longer panic, a count no longer makes `load_missing` skip the
  rows, and a failed or cancelled nested `load_missing` no longer erases
  relations already loaded. Relations declared with `lk = "..."` read and
  write by that column, and an `lk` naming no field is a compile error.
  Eager many-to-many honours `related_key`, a relation's `pivot_table`
  override is used when loading pivot context, and eager `HasManyThrough`
  skips rows reached through a trashed intermediate. Eager loads of a
  `with_tx(&tx)` or `on(name)` query run on that transaction or connection,
  `MassPrunable` deletes on the connection its dry run counted, and factory
  inserts of plain SeaORM rows join the surrounding `DB::transaction`.
  `with_min` and `with_max` of an integer column read on Postgres. `has`,
  `where_has` and `doesnt_have` work on models whose primary key is not
  `id`, and join a many-to-many on its declared `related_key`. Lazy
  `HasManyThrough` and `HasOneThrough` `get` and `count` apply the target
  model's global scopes, as eager loads do. A pivot model's own global
  scopes and soft-delete filter no longer drop attachments from
  many-to-many reads; as in Laravel, they apply when the pivot model is
  queried on its own. A soft `delete()` sets `updated_at` along with
  `deleted_at`, and `delete_or_fail` touches the owners named in
  `touches`, as `delete()` does. A `BelongsTo` declared without an owner
  key finds its owner by the owner model's primary key instead of `id`, in
  lazy and eager reads, counts and aggregates, `has` and owner touches.
  `has`, `where_has` and `doesnt_have` on a `MorphedByMany` relation work;
  they named a pivot column that does not exist. Every relation key default
  now comes from the models' primary keys, never a literal `id`:
  `HasManyThrough` and `HasOneThrough` without `second_local_key` join on
  the intermediate model's primary key, `MorphMany` and `MorphOne` report
  their parent key or the declared `lk`, and a `MorphTo` reports its
  targets' primary key, through the morph registry when the targets
  differ; `MorphTypeEntry` gains a `primary_key` field. This landed after
  the `v3.1.0` tag.
- **Magnetar hashing and sign-up races.** Magnetar password hashing runs on
  Tokio's blocking pool instead of stalling async workers. A magic-link or
  passkey sign-up that loses a race for a new email address answers as the
  existing account instead of failing. This landed after the `v3.1.0` tag.
- **RenderCache stays coherent under cancellation, races and flags.** A
  write that was cancelled at its generation advance, including a bulk
  write, `increment`, a soft delete, restore or force delete, could commit
  while cached pages stayed current; the write and its advance now share
  one transaction. An unrelated successful write no longer resumes serving
  pages whose invalidation failed: the missed invalidation is applied
  first. A process that wrote before installing RenderCache advances
  generations after the install. Pages rendered with a flag's compiled
  default refresh when the first rule for that flag is stored, and a page
  rendered during `set_flag` or `reload` no longer stays cached with the
  old answer. `DB::unprepared`, `DB::statement` and `statement_on` with a
  batch that begins with `SELECT` invalidate cached pages. Pages built from
  `EntityExt` or `QueryBuilder` reads, relation counts and aggregates, or
  through-relation loads refresh when those tables change.
  `RenderCache::advance_epoch` reaches the next request even with another
  request's authority read in flight. A rebuild that fails after the
  stale-on-error window closed returns its error instead of the expired
  entry, a node runs at most one background refresh per key, a cancelled
  L1 publish can no longer overwrite a newer entry, and L1 sweeps no longer
  scan the whole store. SQL Live record cleanup no longer deletes a fresh
  instance or reservation another node just created. Renewing an async
  subscription from an evicted position is refused, so the membership
  degrades instead of claiming continuity. The Live tooling helper's
  timeout ends the call even when a process it started keeps its output
  open. This landed after the `v3.1.0` tag.
- **Query builder, pagination and Eloquent.** Paginating, ordering and
  taking `first` of a union works on every engine, and `total` counts its
  rows; as in Laravel, ordering, limit and offset set before `union` apply
  to the first query and those set after it to the whole union. `count`,
  `sum`, `avg`, `min` and `max` work after `select(...)`, after an
  ordering, on a union and with `having`. `in_random_order` works on MySQL,
  and `skip` or `offset` without `limit` works on SQLite and MySQL.
  `chunk_by_id`, `lazy_by_id`, `cursor_paginate` and `Pagination::cursor`
  visit every row once whatever the query was ordered by, and apply an
  offset once. `chunk`, `chunk_map`, `each`, `chunk_by_id` and `lazy_by_id`
  honour the query's `limit` as a cap on the whole walk and its `offset` as
  a one-time skip, so `.limit(5).chunk_by_id(2, ..)` visits 5 rows instead
  of the whole table. `cursor_paginate` on a union pages the whole union.
  `count`, `sum` and `avg` return 0, and `min` and `max` return `None`,
  when an offset skips the aggregate's row or a grouped query has no rows,
  instead of failing with "aggregate query returned no row". `filter_json_contains` works on Postgres and MySQL with
  strings, objects and arrays. `create_or_first` inside a Postgres
  transaction returns the existing row, and a lost race creating a Live
  record no longer aborts the host's Postgres transaction. `Unique` and
  `Exists` inside `DB::transaction` see the transaction's own rows and no
  longer wait on its connection. `Collection::sort_by` and the by-value
  `Distinct` rule compare integers above 2^53 exactly. Index and foreign key
  names holding a backtick or double quote create and drop.
  `decrement(col, i64::MIN)` subtracts instead of panicking.
  `UniqueIdKind::Ulid.is_valid` rejects strings that overflow 128 bits.
  `QueryExecuted::to_raw_sql()` returns the SQL unchanged when a binding is
  missing or left over, and `QueryBuilder::count` and
  `Pagination::length_aware` report the `COUNT(*)` they ran to `DB::listen`
  and the query log. Inertia infinite scroll asks for the paginator's own
  page or cursor parameter. Concurrent `Schema::dump` calls to one path each
  write a whole file. `test_database!()` with no argument compiles and uses
  the crate's own `migrations::Migrator`. This landed after the `v3.1.0`
  tag.
- **Typed counts, union walks and decimal relation minimums.** Typed
  `QueryBuilder::count` keeps the whole count under a limit and returns 0
  past an offset, `exists` asks about the rows the limit and offset leave,
  and a `Pagination::length_aware` total counts every match, as Laravel
  does. `chunk_by_id`, `lazy_by_id`, `lazy` and `cursor` over a union visit
  each row once and finish; the cursor bounded only the first query, so the
  batches repeated forever. `<rel>_min_as::<Decimal>()` and
  `<rel>_max_as::<Decimal>()` read a `NUMERIC` or `DECIMAL` minimum or
  maximum exactly on Postgres and MySQL, and `::<String>()` reads its
  decimal text. This landed after the `v3.1.0` tag.
- **Debounced jobs, queue events, log subscribers and process timeouts.**
  Overlapping debounced dispatches keep the one that armed last: each takes
  its place from an atomic counter in the cache, so a newer push is never
  dropped for an older `Queue::bulk`, and a stalled dispatch does not also
  run. When max wait runs out inside one `Queue::bulk`, the bulk's last job
  runs at once instead of being dropped as superseded. A job deleted by
  middleware, or dropped as a superseded debounced dispatch, fires
  `JobAttempted`, as Laravel's worker does. Building a log subscriber with
  `logging::build_subscriber` changes nothing until it is used: each
  subscriber writes file lines in its own format, and building no longer
  resets the default channel, so a caller that relied on that calls
  `check_channels`. A process timeout's kill can no longer reach a process
  or group id that the owner reaped in the meantime. This landed after the
  `v3.1.0` tag.
- **Docs reading order, memory fanout streams and the image budget
  docs.** A docs chapter listed twice in the table of contents keeps one
  place in the previous and next chain, its first listing, and every
  catalog entry for it matches its JSON file. A `memory://` fanout stream
  is freed when its last hub drops instead of staying registered until the
  process exits. The `ImageConfig` docs give the real 1 GiB default for
  `IMAGE_MAX_ALLOC_BYTES`. This landed after the `v3.1.0` tag.
- **Sessions and remember-me tokens restore on MySQL and MariaDB.** A
  scaffolded application's session, remember-me and auth-flow token time
  columns are `TIMESTAMP` there, and the framework read them as a type the
  MySQL driver decodes only from `DATETIME`. Every session read failed: a
  form POST answered 419 and the next page 500, the remember-me cookie
  never signed anyone back in, and verification and reset links could not
  be used. The notification inbox and the ceremony store failed the same
  way, and on Postgres each failed on a `timestamptz` column. They now read
  `DATETIME`, `TIMESTAMP`, `timestamp`, `timestamptz` and SQLite text, and
  existing tables need no migration. A notification whose `read_at` does
  not decode is an error instead of unread. On MySQL, an expiry past
  2038-01-19 03:14:07 UTC is stored as that moment. This landed after the
  `v3.1.0` tag.
- **A new application registers and signs in on every database.** The
  `users` table and `User` model that `suprnova new` writes disagreed on
  the time column type, so registering failed on MySQL 8.4, MariaDB and
  Postgres. The migration now uses `.date_time()` and the model names
  `AsNaiveDateTime`. An existing application gives its `User` the casts for
  its columns: the native casts on MySQL and MariaDB, the naive ones on
  Postgres. See Authentication, "The scaffolded User model". This landed
  after the `v3.1.0` tag.
- **Two-factor flows on every engine.** A framework login completed with
  `TwoFactor::complete_challenge` stays signed in under the Magnetar
  engine. The two-factor credentials table and the attempt-counter
  migration create on MySQL and MariaDB. Concurrent second-factor
  admissions no longer deadlock across users on MySQL. On Postgres, a
  correct Magnetar second-factor code, every ceremony state change, passkey
  registration and removal, and account unlinking answered 500; they work.
  A committed two-factor confirmation is reported as confirmed even when
  its attempt record cannot settle. A password reset keeps an encrypted
  column that changed while it ran, and stores the new hash verbatim
  whatever the model's mutators. This landed after the `v3.1.0` tag.
- **Cache, maintenance mode and read-through disks.**
  `Cache::remember_forever` and `Cache::sear` no longer expire after
  `CACHE_DEFAULT_TTL`, and `down` with `MAINTENANCE_DRIVER=cache` no longer
  ends by itself after it. On Redis, `Cache::flush` matches `REDIS_PREFIX`
  literally, so a prefix holding `*`, `?`, `[` or `\` no longer deletes
  other applications' keys or misses its own, and a value extended with
  `Cache::touch` is still removed by `Cache::flush_tags`. Tag indexes no
  longer grow without bound for tags written often and rarely flushed.
  Concurrent `down` runs with the file driver no longer publish a
  half-written down file. The production guard for `on_one_server()` checks
  the bound cache store instead of `CACHE_DRIVER`; a custom shared
  `CacheStore` overrides the new `locks_are_shared` to return `true`. The
  cached feature-flag evaluator sees an admin change made during a
  concurrent miss and holds at most 4096 entries. Read-through promotions
  stream into the primary instead of holding the object in memory,
  unpromoted reads fetch only their range, and a delete or move during a
  promotion is not undone, on the same node or another one: a promotion
  asks the fallback again just before it publishes and after, a cancelled
  read still finishes that check, and a withdrawal deletes only the exact
  version the promotion wrote, never a writer's newer object. On a primary
  without versioned deletes (local, memory, and unversioned S3, Azure Blob
  and GCS) the promotion keeps its copy and logs a warning instead, so a
  delete on another node that lands between the last check and the publish
  can leave that copy behind. Versioned and conditional reads reach the
  fallback, and a refused move
  keeps the fallback copy. Ranged reads stop at the requested range: a
  server that ignores `Range` can no longer make a small read buffer the
  whole object, and S3, Azure Blob and GCS refuse a response that is not
  the requested range before reading its body, open-ended ranges included.
  This landed after the `v3.1.0` tag.
- **Queues, events and processes.** Cancelling `Transaction::commit()`
  while its COMMIT was in flight could drop its `after_commit` callbacks and
  `push_after_commit_with_tx` jobs although the rows committed; they now
  always run. A queued listener that dispatches another queued event no
  longer hangs queued event dispatch, `drain_queued` waits for and counts
  listeners admitted while it drains, and `EventDispatcher::defer` buffers
  only its own dispatcher's events. A debounced dispatch claims its window
  only after its envelope is queued, so a failed or cancelled dispatch can
  no longer get queued work dropped as superseded, and `max_debounce_wait`
  longer than the debounce token's lifetime forces a run again. Batches on
  the `sync` driver finish and fire their callbacks, a batch whose dispatch
  failed part way fires `catch` and `finally` once its queued jobs settle,
  and two concurrent first batch dispatches no longer lose one batch's
  tracking. `JobAttempted` fires for jobs that fail or time out terminally.
  A cache error while `ThrottlesExceptions` clears its counter no longer
  fails a completed job, and one while it counts a failure no longer
  replaces the job's own error or turns a backoff release into a failed
  attempt. A cancelled memory-queue `pop`, delayed `nack` or
  `release` keeps the job. `FailoverQueueDriver` counts a driver registered
  under two labels once. An after-commit `push_unique` whose job fails to
  serialize releases its lease. A started process whose child left its
  group can no longer have its timeout, `stop()` or drop signal an
  unrelated process group. This landed after the `v3.1.0` tag.
- **A Fluent message named `NUMBER` or `DATETIME` keeps the function
  callable.** Such a message broke every `NUMBER(...)` or `DATETIME(...)`
  call in its catalog. The message keeps its key and the functions still
  format. This landed after the `v3.1.0` tag.
- **Presence channels and WebSocket upgrades.** A presence channel whose
  last member left stayed in memory, so parameterized presence channels grew
  with churn; empty channels are now removed. A connection aborted at
  shutdown or cancelled during a re-subscribe left its presence member
  visible and its forwarder tasks running; cleanup, `presence.left`
  included, now runs on every exit. Headers middleware adds to a successful
  WebSocket upgrade, such as a session `Set-Cookie`, now reach the client on
  the 101, and terminable middleware runs for upgrade responses with status
  101 or the refusing status. WebSocket route parameters are percent-decoded
  like HTTP ones, subprotocol negotiation echoes the client's own spelling
  as RFC 6455 requires, and `sse::last_event_id` returns Unicode event ids
  instead of `None`. This landed after the `v3.1.0` tag.
- **Each named middleware runs once per route.** A middleware group
  included by two sibling groups, an alias listed twice, or a middleware
  named both through a group and on the route itself ran twice per
  request, so a throttle there counted each request twice. A route now runs
  each named middleware once, identified by its alias and parsed arguments,
  at its first occurrence, as Laravel's `uniqueMiddleware` does. Middleware
  added by type with `.middleware(M)` is never dropped. This landed after
  the `v3.1.0` tag.
- **Gates, OAuth starts, CSRF bootstraps and registrations.** A gate
  callback that calls `Gate::define` no longer deadlocks the request. An
  OAuth start that is the browser's first request (JSON or POST) sets the
  session cookie, so the callback binds instead of answering 400. A
  cookieless JSON or HEAD request that receives `XSRF-TOKEN` also receives
  its session, so the next POST no longer gets 419. Two registrations
  racing for one email create one account. This landed after the `v3.1.0`
  tag.
- **Images decode correctly at the sizes the framework allows.** JPEGs
  above about 22 megapixels decoded on no driver; they now decode up to the
  budget. JPEG colours were off by about 6 levels, arithmetic-coded,
  odd-sized 4:2:0 and 4:2:2, 4:4:0 and 4:1:1 JPEGs decoded wrong or not at
  all, and small progressive JPEGs could fail. GIFs from ImageMagick and
  Pillow decode, and the GIFs the default driver writes open in other
  software. The `magick` driver processes and sizes only the first frame of
  an animation, as the default driver does, and reports a GIF's logical
  screen as its size. This landed after the `v3.1.0` tag.
- **Localization.** `Accept-Language` negotiation honours q-values:
  `en;q=0.1, fr` picks `fr`, and a language sent with `q=0` is never
  chosen. `DATETIME()` formats in the catalog's own locale. A catalog edit
  saved during a reload is picked up by the next one. `Lang::has` is false
  for a message that has only attributes, matching `Lang::get`. A Fluent
  term and a message with the same name both resolve on the server, as in
  the browser. This landed after the `v3.1.0` tag.
- **Logging.** A custom driver's failed write or flush, and a file channel
  that cannot flush (a full disk), are reported once on stderr instead of
  dropping lines silently. `LogChannel::stack([...]).level(...)` filters
  every channel the stack lists. A refused second `init_subscriber` or
  `init_telemetry` no longer changes the live file format or the default
  channel. `Log::forget_channel` after rotation reopens the file in every
  stack that lists the channel. A default stack with several stdout
  channels writes an event when any of them keeps its level. Nested spans
  that share a field name log the inner value in the message and the
  context. This landed after the `v3.1.0` tag.
- **Markdown heading ids are unique, and process output keeps its
  characters.** A heading titled like a numbered duplicate (`Overview 2`)
  no longer shares an anchor with a repeated `Overview`. Streamed process
  output no longer drops a character split across reads after an invalid
  byte, and `wait_until` sees a final incomplete character. This landed
  after the `v3.1.0` tag.
- **Payments, mail, notifications and queues.** `PhoneNumber` and
  `CountryCode` validate when deserialized, where invalid values were
  accepted and `digits()` could panic. `MailFake::assert_not_outgoing`
  fails when the mailable was sent, not only queued. The `file` mail driver
  never overwrites a preview written in the same millisecond. Tagged Resend
  mail is accepted by Resend. Notification mail renders from the
  notification itself, so `data()` no longer has to hold every field
  `to_mail` reads. An SQS overflow job whose first send went unanswered
  keeps its payload when the retries are refused, and when SQS took both
  an unanswered send and its retry, each message now carries its own
  payload copy, so acknowledging one no longer leaves the other unreadable.
  The Redis queue and the broadcast fanout work with
  `redis://user:password@...` and `rediss://` URLs, taking credentials,
  database and TLS from the URL. This landed after the `v3.1.0` tag.
- **Middleware, sessions, uploads and test helpers.** A middleware group
  reused by two sibling groups no longer fails with `CycleDetected`. A
  `RateLimiter` counter that expired in the middle of a hit gets its expiry
  back, where it could refuse every later request until cleared. Flash keys
  containing `_flash.new.` or `_flash.old.` survive the next request, and
  `SessionData::decrement(key, i64::MIN)` saturates instead of panicking. A
  form body sent as `Application/X-WWW-Form-Urlencoded` (any case) is read
  by `Request::input`, CSRF, Pusher channel auth and identity rate limits.
  `set_global_upload_spill_threshold(usize::MAX)`, the documented way to
  keep uploads in memory, no longer panics. The Inertia error page compares
  `Accept` qvalues to three decimals. A redirect on a route without a
  session keeps `App::flash` values for the current response.
  `TestResponse::assert_cookie` matches only cookie names, not attributes
  such as `Path` or `HttpOnly`. This landed after the `v3.1.0` tag.
- **A Live page with several islands on SSE keeps its updates.** When an
  island's first event arrived before its subscribe answer, the browser
  treated it as traffic for an unknown subscription, retired the shared
  stream and showed every island on the page as "Updates degraded". Such an
  event is now held, counted against `LIVE_ASYNC_MAX_QUEUED_EVENTS`, and
  applied once that exact answer arrives, or dropped if the subscribe is
  refused; the same holds for the replacement subscribe that follows a
  failed island. A WebSocket transport still fails closed, as before. A
  document whose held and queued events pass the limit now reports the
  breach with the limit's key and reconnects, instead of degrading every
  island. This landed after the `v3.1.0` tag.
- **Payment webhooks, feature flags and RBAC work on Postgres, MySQL and
  MariaDB.** Their models used a text cast for native timestamp columns,
  so every payment webhook answered 500, `set_flag` and the feature admin
  failed, and `Role` and `Permission` could not read a row the RBAC
  helpers wrote. They now read and write the native columns; text written
  on SQLite still reads back. This landed after the `v3.1.0` tag.
- **Job batches settle on Postgres.** The documented `job_batches` schema
  declares `INTEGER` columns, which Postgres stores as 32-bit, and the
  repository read them as 64-bit, so every settlement failed: the job ran
  again after each visibility timeout and the batch callbacks never fired.
  The repository reads either width, and the documented epoch columns are
  now `BIGINT`; tables created from the earlier schema keep working. This
  landed after the `v3.1.0` tag.
- **The MariaDB vector store works against a real server.** `similar`
  looked for the vector index in a form MariaDB never prints, so every
  call failed, and the table `ensure_table_sql` emits could not be created
  because its key was too long for a vector index. The id column is now
  `VARBINARY(254)`, which also compares ids byte for byte as the other
  drivers do. This landed after the `v3.1.0` tag.
- **Native JSON columns have their own cast.** The structured casts bind
  and read text, which Postgres refuses for `json` and `jsonb` and MySQL
  for `JSON`, although their documentation said those columns worked. The
  new `AsNativeJson<T>` and `AsOptionalNativeJson<T>` store any serde type
  as JSON. The `AsBool` documentation now names a `BIGINT` column and sends
  a native boolean column to a plain `bool` field. This landed after the
  `v3.1.0` tag.
- **SMTP mail goes out from the return path.** A return path reached SMTP
  only as a header, so bounces still went to the author; the envelope
  sender is now the return path. This landed after the `v3.1.0` tag.
- **Queued mail and queued notifications send.** Nothing registered their
  jobs with the worker and the manuals never said to, so every one failed
  as an unknown job and dead-lettered, and with the sync driver the push
  itself failed. This landed after the `v3.1.0` tag.
- **Queued and notification mail fire `MessageSending` and `MessageSent`.**
  Only a direct send fired them, so switching to `queue` or sending through
  a notification cut off audit and metrics listeners. This landed after
  the `v3.1.0` tag.
- **A Live primary button looks like the primary action.** A
  `type="button"` button with the default primary variant rendered as a
  secondary control, and so did dialog, sheet and drawer triggers. This
  landed after the `v3.1.0` tag.
- **`sum` and `avg` of an integer column read on Postgres and MySQL.**
  Postgres answers `numeric` and MySQL `DECIMAL` for them, which `sum` and
  `avg` could not read, so they failed there while passing on SQLite. They
  now read either: an integer sum exactly, refusing a fractional or
  out-of-range sum with an error that quotes it, and `sum::<f64>` of an
  integer column now reads on SQLite too. `with_sum` and `with_avg` stored
  `0.0` on Postgres and MySQL; they now hold the value, and a value that is
  not numeric is an error instead of a silent zero. This landed after the
  `v3.1.0` tag.
- **Facebook sign-in receives the email.** The Facebook provider requested
  a bare `/me`, for which the Graph API returns only `id` and `name`; it
  now names `id`, `name`, `email` and `picture`. X requests
  `profile_image_url` among its user fields. This landed after the
  `v3.1.0` tag (#140).
- **`live:check` finds unescaped output wherever a template writes it.**
  It read `|safe` from an expression's text and skipped `{% let %}`, so
  `{% let y = x|safe %}{{ y }}`, `x|safe|lower` and `escape("none")`
  passed. It now reads the parsed template and follows a raw value through
  `let` and `set`, macro arguments, macros called as expressions, caller
  content, inherited blocks and `super()`, loops, `if let`, let chains and
  `match`, to the place it is written. A filter outside Askama's builtins
  and the framework's, a Rust macro in an expression, and a dynamic value
  in an unquoted attribute, an `on*` attribute or `script` or `style` text
  are reported as unproved, so a view that passed before can now need a
  change. This landed after the `v3.1.0` tag.
- **`live:check` handles views with many conditionals.** It counted every
  combination of `if` blocks, so eight independent ones exceeded its limit;
  each conditional's branches are now checked once. Its limits are sized
  for real templates (4 MiB of source, 262,144 nodes) and configurable.
  This landed after the `v3.1.0` tag.
- **`live:check` reports the real column.** Directive and markup
  diagnostics reported column 1; they now point at the attribute or tag,
  in the template that wrote it. This landed after the `v3.1.0` tag.
- **An action can dispatch more than 128 events.** An action's outcome
  refused more than 128 flash messages, events or effects; it now takes as
  many as `LIVE_MAX_RESPONSE_ITEMS` allows, 65,536 by default. This landed
  after the `v3.1.0` tag.
- **Reordering a long keyed list morphs in linear time.** The morph looked
  up each keyed element in the list of moved elements, so reordering every
  row took time quadratic in the row count; 10,000 rows took about 67 ms
  and now take under 20 ms. This landed after the `v3.1.0` tag.
- **A workflow step can take an integer argument.** `#[workflow_step]`
  handed the step's body to the workflow context in a closure that borrowed
  its arguments, and the context needs one it can keep, so a step taking a
  `Copy` argument such as `user_id: i64` failed to compile with E0373
  ("closure may outlive the current function"). The closure now takes its
  arguments by value.

- **Workers wait for their queued listeners.** `queue:work`,
  `schedule:work` and `workflow:work` now wait, up to ten seconds, for the
  queued event listeners still running when they stop, as the server
  does, and `schedule:run` does once its tasks have run; returning dropped
  them part way.
- **`Cache::bootstrap` keeps the cache store the application bound**, as
  the localization chapter says it does, instead of replacing it at boot.
- **The event dispatcher forgets its finished queued listeners** as it
  starts new ones; it kept every one until shutdown.
- **`DB::flush_query_log` releases the log's buffer** rather than keeping
  its capacity.
- **The in-memory vector search no longer panics on a NaN score**; such
  an item ranks below every other.
- **A `#[json_resource]` keeps its attributes in declaration order** when
  it drops a missing one; the last attribute used to move into its place.
- **The brute-force lockout map sweeps once each time it doubles.** With
  more than 1,024 current lockouts it swept on every new lockout.
- **The toast region lets go of toasts a morph removed**, which it timed
  and later dismissed for the life of the page, and resumes its timers
  when the focused toast is removed or the pointer has left it without the
  region hearing it.
- **The Live browser runtime reads a network chunk larger than one record**
  when the chunk holds only complete records; it ended the stream.

- **The Live chart follows the document's theme.** `render_chart` drew its
  SVG with charts-rs's light theme, a white background and fixed colors and
  font, so in a dark document the chart showed as a white box. The SVG now
  has a transparent background and no color or font of its own: its text,
  axis, grid and series carry classes that the chart component's
  stylesheet colors from `--sn-` tokens, and the chart's text takes the
  `--sn-font-sans` font. Series take the new `--sn-color-chart-1` to
  `--sn-color-chart-6` palette in order, defined for light and dark and
  mapped in the Tailwind preset, and a seventh series starts the palette
  again. An application that vendored the chart keeps its old stylesheet,
  which has no rule for these classes, so every series, axis and grid line
  draws in the text color. Run `suprnova live:add chart` to take the fix
  into it. This fix landed on main after the `v3.0.0` tag.
- **A cached render whose COMMIT fails answers 500 without a panic.** When
  a cache-miss render's handler ran and the database then refused the
  render transaction's COMMIT (a deferred constraint, a dropped
  connection), the render cache took the failure for a transaction that
  never opened: a debug build panicked, and the log said nothing about the
  commit. It now logs the database error under `suprnova::database` and
  answers the same 500, because the handler's writes rolled back and the
  rendered page would claim they landed. A render transaction that cannot
  open still renders uncached, as before. This fix landed on main after the
  `v3.0.0` tag.
- **A savepoint can be named with a reserved word.** `tx.savepoint("inner")`
  failed with a syntax error on PostgreSQL and MySQL, because the validated
  name went into the statement unquoted. Savepoint names are now quoted for
  the backend; they stay case-insensitive. This fix landed on main after the
  `v3.0.0` tag.
- **The sync queue driver runs a whole chain.** `SyncQueueDriver` ran a
  chain's first job and dropped every later link. It now runs the chain
  inline, link by link, as Laravel's sync queue does; a link that fails
  returns its error and the rest of the chain does not run. This fix
  landed on main after the `v3.0.0` tag.
- **`authorize_resource` checks the user of the route's guard.** On a route
  whose `AuthMiddleware` names a guard other than the default,
  `authorize_resource` checked the default guard's user instead of the one
  the route authenticated, so that user was refused. It now checks the
  route guard's user, as Laravel's `can` middleware does. This fix landed
  on main after the `v3.0.0` tag (#127).
- **A cached page that reads through a raw fragment is never served
  stale.** A `select_raw`, `where_raw`, `filter_raw` or `order_by_raw` on a
  model query can read a table the query doesn't name, but the render
  cache stored the page anyway, so a write to that table left it stale. A
  query carrying a raw fragment now keeps the page out of the cache, as raw
  `DB::select` does; the page is still served. A `select_raw` that is a
  bare number, such as `select_raw("1")`, doesn't count. This fix landed
  on main after the `v3.0.0` tag.
- **A destructured route parameter binds.** A handler parameter written
  `RouteParam(user): RouteParam<User>`, as the `RouteParam` docs show, looked
  up a route parameter named `param` and answered 400 to every request. It
  now reads the parameter its binding names, and a pattern with no single
  binding is a compile error. This fix landed on main after the `v3.0.0` tag.
- **SQLite rows keep computed columns.** An aggregate or `select_raw`
  expression came back missing from `DB::table` and `DB::select` rows on
  SQLite, because SQLite declares no type for a computed column. Such a
  column is now read by its value's runtime type. This fix landed on main
  after the `v3.0.0` tag.
- **A union arm keeps its soft-delete and scope filters.** A union of model
  queries rendered each arm's `WHERE` without its scopes, so soft-deleted
  rows came back. This fix landed on main after the `v3.0.0` tag.
- **A page cached from `where_has` sees writes to the related table.**
  `has`, `where_has`, `doesnt_have` and `where_relation` did not record the
  related or pivot table for the render cache, so a write there left the
  cached page stale. This fix landed on main after the `v3.0.0` tag.
- **The notifications table ships as a migration.** The manual said
  `suprnova migrate` creates it, but the schema was a SQL file only the
  framework's own tests loaded, so the database channel failed on its
  first write in a fresh app. Register
  `suprnova::notifications::migrations::CreateNotificationsTable` in your
  `Migrator`: it creates the same table and indexes, and running it over a
  table you created by hand from the old SQL file is safe on every engine.
  The SQL file is gone. This fix landed on main after the `v3.0.0` tag
  (#134).
- **Nullable JSON columns have casts.** `AsJson`, `AsArray`, `AsObject`,
  `AsCollection` and `AsArrayObject` store a non-null string, so
  `AsJson<Option<T>>` wrote the text `null` instead of SQL `NULL`, and a
  row whose column was `NULL` failed to load. `AsOptionalJson`,
  `AsOptionalArray`, `AsOptionalObject`, `AsOptionalCollection` and
  `AsOptionalArrayObject` map `None` to `NULL` and back, and store a value
  exactly as their non-optional cast does. This fix landed on main after
  the `v3.0.0` tag (#133).
- **`SESSION_TABLE` names the session table.** `SessionConfig::table_name`
  was read and then ignored: the database session driver always used
  `sessions`. The driver now reads and writes the configured table,
  `DatabaseSessionDriver::with_table` builds one over another table, and
  `Config::init` refuses a name that is not 1 to 63 ASCII letters, digits
  or underscores starting with a letter or underscore. What you have to
  change: an app that already set `SESSION_TABLE` now stores its sessions
  in that table, which its migration must create, and an empty
  `SESSION_TABLE` now fails boot. The driver also stops logging a session
  id when it skips a write, and the content of a stored payload it cannot
  parse. This fix landed on main after the `v3.0.0` tag (#132).
- **Re-running a framework migration no longer fails on an existing index.**
  The workflow and RenderCache migrations relied on `IF NOT EXISTS`, which
  MySQL and MariaDB drop from `CREATE INDEX`, and the payments, features and
  RBAC migrations created their indexes without it, so running one over
  tables that already existed failed with a duplicate index and blocked
  every migration after it. Each framework migration now creates an index
  only when it is missing, with the same columns and uniqueness as before.
  This fix landed on main after the `v3.0.0` tag (#136).
- **A `Data` object with a route-parameter field answers 422 for a body
  that does not fit.** Its extractor answered 400 where the default
  extractor answers 422 for the same malformed or unknown-key body. This
  fix landed on main after the `v3.0.0` tag.
- **A `Data` field may be named `key` or `map`.** The generated
  `Deserialize` named its locals after the fields, so such a field
  shadowed the visitor's own variables and the struct failed to compile.
  This fix landed on main after the `v3.0.0` tag.
- **`has`, `where_has` and `where_relation` work through a `BelongsTo`
  relation.** The existence query compared the related table's key with
  the foreign key as if the relation were a `HasMany`, so every existence
  query through a `BelongsTo` failed with an unknown column. It now joins
  the related row's owner key to the parent's foreign key. This fix landed
  on main after the `v3.0.0` tag.
- **A model declared beside `use sea_orm_migration::prelude::*` compiles.**
  That prelude brings `ExprTrait` into scope, whose `max` and `is_null`
  took over calls the `#[model]` macro emitted for relation counts and
  `MorphTo` relations. The macro now names those methods by path. This fix
  landed on main after the `v3.0.0` tag.
- **Persisting a replica stamps its timestamps.** `replicate` resets
  `created_at` and `updated_at` for the insert to fill, but `persist`
  wrote them as built: 1970-01-01, or NULL for an optional field. It now
  stamps a timestamp its builder left unset, as `create` does, and keeps
  one the builder set, so a factory can still backdate a row. This fix
  landed on main after the `v3.0.0` tag.
- **Touching an owner writes the owner's own date-time storage.** The
  `touches` cascade bound the time as RFC 3339 text whatever the owner's
  `updated_at` cast stored, which Postgres refuses for a native date-time
  column. It now stores the time through the owner's cast. This fix landed
  on main after the `v3.0.0` tag.
- **The temporal casts' documentation said Postgres accepts RFC 3339 text
  for a native column.** It refuses a text parameter for `timestamp` and
  `timestamp with time zone`; the module documentation now says so and
  points to the native casts. This fix landed on main after the `v3.0.0`
  tag.
- **`Unique` and `Exists` keep database errors out of the response.** A
  database rule that could not run returned the driver's error as its
  validation message, and a validation message is rendered into the 422
  body, so a client could read table names, column types and SQL. The rule
  now logs the cause under the `suprnova::validation` target and fails the
  field with `validation-unchecked`. This fix landed on main after the
  `v3.0.0` tag.
- **Precognition keeps the errors of an array's elements.** A
  `Precognition-Validate-Only` header naming `tag_ids` dropped the errors
  reported under `tag_ids.0`, `tag_ids.1` and so on, so a form validating
  the field saw success for an invalid array. A field now keeps the errors
  nested under it, and `tag_ids.*` matches the elements as Laravel's rule
  key does. This fix landed on main after the `v3.0.0` tag.
- **The date picker, upload, account menu and notification bell follow the
  theme.** Their stylesheets read `--sn-color-accent` and
  `--sn-color-on-accent`, which the token stylesheet never defined, so the
  selected day, the upload progress bar, the avatar and the unread count fell
  back to inherited colors and ignored every theme. The token stylesheet now
  defines both, from `--sn-color-primary` and `--sn-color-primary-contrast`,
  and the Tailwind preset maps them as `accent` and `on-accent`. An
  application that vendored these components with `live:add` needs no change:
  the framework serves the corrected stylesheet. This fix landed on main after
  the `v3.0.0` tag.
- **A first `live:model` edit on a public seed promotes it.** The browser
  runtime sends an immediate `live:model` edit on a public-seed island as a
  model synchronization with no action, and the action endpoint promoted a seed
  only for an action, so the first keystroke in such an island answered `500`.
  The endpoint now promotes the seed on that request, applies the proposals as
  it does on an instance, and runs no action. This fix landed on main after the
  `v3.0.0` tag.
- **The date picker's "Pick the parts" toggle no longer looks like a second
  field.** The base layer draws every `details` as a bordered surface for the
  collapsible, and the date picker's disclosure inherited it, so an empty box
  sat under the date input. The disclosure is now a plain toggle; the year,
  month and day strips keep their borders. Run `suprnova live:add date-picker`
  to take the fix into an application that vendored the component. This fix
  landed on main after the `v3.0.0` tag.
- **`suprnova generate-types` names and omits plain-struct keys the way serde
  does.** A struct a prop reaches that derives serde's `Serialize` sends the
  keys its `#[serde(...)]` attributes give it, but the generator declared every
  field under its Rust name. It now leaves out `skip` and `skip_serializing`
  fields, declares `skip_serializing_if` fields optional, names keys by
  `rename` and `rename_all` (and their `serialize = ...` forms), and drops the
  `r#` of a raw identifier. Other serde attributes, such as `flatten` and
  `transparent`, are still not read. A key that is not an identifier is now
  quoted, which also makes a raw identifier on a derived struct valid
  TypeScript (`"r#type"`, the key its derive sends). `#[derive(Data)]` and
  `#[derive(InertiaProps)]` structs keep their Rust names: their own
  `Serialize` never reads `#[serde(...)]`. This fix landed on main after the
  `v3.0.0` tag.

### Security

- **Input-only relationships stay out of JSON:API output.** A relationship
  marked `#[data(input_only, allow_include)]` was linked and includable in
  responses; it is now never sent. This landed after the `v3.1.0` tag.
- **The environment is written only where that is sound.** `Config::init`
  and `config::load_dotenv` refuse to write the process environment inside
  a Tokio runtime or after `#[suprnova::main]` loaded it, where another
  thread could read it mid-write. A failed load restores the real system
  values and registers no config. This landed after the `v3.1.0` tag.
- **Route bindings and pivot extras respect what models declare.**
  `RouteParam<Model>` ignored global scopes on models without
  `soft_deletes`, so a guessed id of another tenant's row bound to the
  handler; the binding now applies every global scope, and a hidden row is
  a 404. `attach_with` wrote pivot extras past the pivot model's casts, so
  an `AsEncrypted` column was stored as plaintext and an `AsHashed` one
  unhashed; extras now go through the pivot's casts and mutators, and an
  extra that does not decode into its field is a validation error. This
  landed after the `v3.1.0` tag.
- **RenderCache never stores or reuses a personalized page as shared.**
  Work a handler joined beside an identity-bound island mount
  (`tokio::join!`) had its principal, session and table reads dropped from
  the stitched shell's report, so a personalized shell could be stored as
  public; those reads now count. `PrivateCached` responses send
  `Cache-Control: private, no-cache` instead of `private, max-age`, which
  let a browser show the previous account's page to the next account on
  the same browser. Oversized Redis hint payloads are dropped before they
  are copied or queued. This landed after the `v3.1.0` tag.
- **Redis credentials stay out of the boot log.** When Redis was
  unreachable at boot, the cache error held the whole `REDIS_URL`, password
  included, and `CacheConfig` and `CacheConfigBuilder` printed it when
  debug-formatted. The error now names only the host and port or socket
  path, and the configurations print the URL as `redis://<redacted>`. This
  landed after the `v3.1.0` tag.
- **Second-factor codes are rate limited and single use.**
  `TwoFactor::verify` and `consume_recovery_code` ignored the account
  lockout, so a caller with the password could guess TOTP codes without
  limit; both now refuse while the second factor is locked, and every
  framework login is refused before it spends a code. A confirmation code
  and a TOTP code straddling a timestep are accepted once, and `confirm`
  confirms only the secret whose code was checked, in the framework and in
  Magnetar. Second-factor failures are counted in a table that no password
  identity can reach, so a decoy account named after the second-factor key
  can neither clear nor lock it. A password reset during the password
  check cancels the sign-in or challenge. HTTP Basic once, `Auth::once` and
  `once_using_id` refuse an account with a Magnetar second factor, and
  Magnetar's password, magic link, passkey and OAuth sign-ins refuse an
  account with framework TOTP. A remembered framework login is refused
  before it issues a credential, and account flows write only the
  verified-at or password column. This landed after the `v3.1.0` tag.
- **Local disks stay inside their root.** On Unix, a directory inside a
  local disk root swapped for a symlink during an operation could redirect
  a read, write, copy, rename, delete, listing or publish outside the root.
  Every path is now resolved one component at a time without following
  symlinks, and each operation runs relative to the directory it resolved.
  This landed after the `v3.1.0` tag.
- **Cached feature flags stay per identity.** Two identities whose user and
  team strings joined to the same text could share a cached flag decision.
  The cache key now keeps each part separate. This landed after the
  `v3.1.0` tag.
- **Fanout URL errors no longer print credentials.** A broadcasting fanout
  URL that failed to parse was copied into the error message, password
  included. The error now names the problem without the URL. This landed
  after the `v3.1.0` tag.
- **A JSON login is rate limited by its address.** `identity_key` and
  `names_identity` ignored JSON bodies, so a JSON login was keyed on the
  caller's IP and a `?email=` decoy opened a fresh per-address bucket on
  each request. A top-level string field of a JSON object body now names
  the identity under the same query-versus-body rule as a form field. This
  landed after the `v3.1.0` tag.
- **An SVG allowlist accepts only SVG.** A `MimeType` allowlist naming
  `image/svg+xml` accepted any non-markup text declared as SVG, script
  included, and refused real SVG files. A part now passes only when its
  root element is `<svg>`; anything else declared as SVG fails as the
  wrong type. This landed after the `v3.1.0` tag.
- **A route uses the guard that authenticated it.** Behind a guard other
  than the default, the email-verified gate, the role and permission
  middleware, the Live principal and render-cache identity reads all used
  the default guard's user, so a verified or privileged default-guard user
  let another guard's user through, and a page built for one guard could be
  served from cache to a visitor without that sign-in. A second token guard
  over another provider answered with the first guard's user. Each now
  reads the user of the route's own guard through that guard's provider.
  Live gated actions, uploads and subscriptions, `EmailVerification::verify`
  and the Pusher user and presence endpoints do the same: behind
  `AuthMiddleware::for_guard(name)` they act for that guard's user, as
  `<guard>:<id>` (default-guard principals are unchanged), a named guard's
  logout ends that guard's Live memberships, and a stream with a
  `:principal` topic is refused behind a non-default guard.
- **Encoded cookie names cannot stand in for prefixed cookies.** A cookie
  named `%5F%5FHost-suprnova_session` resumed the `__Host-` session; a
  prefix that appears only after decoding is now dropped.
- **A verification link verifies only the address it was sent to.**
  Changing the account's email no longer lets an old link verify the new
  address.
- **Password reset checks the token before hashing.** A dead or made-up
  token made the server compute an Argon2id hash first, in the framework's
  reset flow and in Magnetar's `PasswordManagementService`.
- **A session that loses its Magnetar authority ends its Live
  memberships** on this node, so it stops receiving events.
- **An image cannot allocate past its decode budget.** A crafted PNG
  inflated past `IMAGE_MAX_ALLOC_BYTES`, a lossless WebP's prefix-code
  tables were unbounded, a JPEG could be measured at one frame header and
  decoded at another, and path and disk sources were read into memory
  before the size check. Each is now counted against the budget before the
  memory is taken. A JPEG whose Extended XMP segments would make the
  decoder's reassembly read more than `IMAGE_MAX_ALLOC_BYTES` is refused
  before decoding; 100,000 stalled segments used to hold a core for
  minutes.
- **The mock payment provider refuses unsigned webhooks outside
  development.** It accepted them when the application registered a
  production or staging `AppConfig` in code with `APP_ENV` unset; it now
  requires both the configured environment and `APP_ENV` to be local,
  development or testing.
- **Queued notification events carry only `data()`.** The queued
  `NotificationSending`, `NotificationSent` and `NotificationFailed` events
  carried the whole serialized notification, including fields kept out of
  `data()` such as reset tokens.
- **Vendor API keys stay with their vendor.** Pinecone and the HTTP mail
  drivers followed redirects, forwarding `Api-Key` and
  `x-postmark-server-token` to another origin. Web push transport errors no
  longer carry the subscription endpoint URL into logs and
  `NotificationFailed`.
- **CORS patterns anchor every alternative.** `allow_origin_patterns`
  anchored only the first and last alternative of a pattern such as
  `a|b`, so `https://app.example.evil.test` matched
  `https://app\.example|...` and got a credentialed allow. Every
  alternative must now match the whole origin.
- **Rate limits cannot be sidestepped.** On the Redis limiter, a
  shorter-window quota sharing a key with a longer one deleted the longer
  quota's history; history now lasts for the longest window used on the
  key, as the memory driver already did. An identity-keyed limit read the
  query string or the body, so `?email=decoy` or a blank `?email=` reached
  a body address under a fresh quota; both are now read, and a request that
  names two different addresses shares one `{prefix}:{field}-ambiguous`
  bucket.
- **`MimeType` sniffs before it trusts the header.** Script text, or
  markup hidden after 16 KiB of whitespace, passed an image allowlist on a
  spoofed `Content-Type`. The header now counts only for types without
  magic bytes, such as `text/csv`.
- **An Inertia validation redirect stays on the host.** A same-host
  `Referer` such as `https://app.test//evil.test/x` sent the redirect to
  another host; it now falls back to the previous URL.
- **Signed URLs sign the order of a repeated parameter.** Swapping the
  values of a repeated key (`?mode=a&mode=b` to `?mode=b&mode=a`) kept the
  signature valid while changing what the handler read. URLs minted before
  still verify.
- **Mail headers and recipients cannot be injected.** Postmark and Mailgun
  received recipient lists joined from unquoted display names, so a name
  such as `attacker@example.com, Victim` added a recipient, and SMTP and
  the file transport wrote header names carrying CR, LF or NUL verbatim.
  Every transport now builds addresses and headers through one validating
  serializer: an address is exactly one RFC 5322 address with its display
  name quoted when needed, a header name follows the RFC 5322 grammar and
  is at most 76 bytes, and a header value, the subject included, refuses
  control characters other than tab. This landed after the `v3.1.0` tag.

## 3.0.0 - 2026-09-29

### Added

- **`FrameworkError::Timeout` tells a passed deadline from a failure.**
  `WorkflowHandle::wait_with_timeout` documented a timeout error that did not
  exist and returned `FrameworkError::Internal`, so a caller could not tell a
  workflow that is still running from a failed status query. The variant
  carries the deadline and what was awaited, `FrameworkError::timeout(elapsed,
  message)` builds one, `is_timeout()` asks for it, and it renders as `504
  Gateway Timeout`. `wait_with_timeout` and `wait_with_options` return it when
  the deadline fires.
- **`Queue::fake()` and `Bus::fake()` install their fakes.** Every other
  facade had `fake()`; these two had only the free function
  `testing::install_fake()`. Both return the same guard, and the free
  functions stay.
- **Failed-job commands: `queue:failed`, `queue:retry`, `queue:forget`,
  `queue:flush` and `queue:prune-failed`.** The manual and the code's own docs
  sent operators to commands that did not exist; the failed-job store could
  only be reached from code. The application binary now has all five, and the
  `suprnova` CLI forwards them. `queue:failed` lists id, connection, queue,
  job, failure time and the first line of the error. `queue:retry` takes one
  or more ids, or `all`. `queue:flush` deletes every failed job, or with
  `--hours N` the ones older than that. `queue:prune-failed` is the same with
  `--hours` defaulting to 24. A command exits non-zero when an id names no
  failed job.
- **Queued mail and notifications can wait for the commit.**
  `Mailable::after_commit(&self)` and `Notification::after_commit(&self)`
  default to `false`. When one answers `true`, `Mail::queue`, `Mail::later`
  and `Notify::queue` inside `DB::transaction` push at the commit, and a
  rollback discards the push, as `Job::after_commit` already does for a job.
  `QUEUE_AFTER_COMMIT=true` turns the same behavior on for every push in the
  process, as the `after_commit` option of a Laravel queue connection does;
  `EnvelopeOverrides { after_commit: Some(false), .. }` still sends one push
  ahead of the commit.
- **Queue connections select a driver.** One process-global driver received
  every push, and a job's connection was only a name on the lifecycle events.
  `Queue::register_connection(name, driver)` now registers a named connection
  next to the default one that `Queue::set_driver` installs, and a push goes
  to the connection it resolves to: a per-push
  `EnvelopeOverrides::connection`, then a `Queue::route`, then
  `Job::connection()`, then the default. `Queue::push`, `push_with`, `later`,
  `bulk` and `push_unique` all resolve it, the jobs of a batch each go to
  their own connection, and a failed job is retried on the connection it
  failed on. `queue:work --connection <name>` and
  `queue::worker::run_worker_on` drain one connection, and the worker carries
  its name on its events and failed-job records. `queue:pause` and
  `queue:resume` take `--connection`. `Queue::connection(name)` returns a
  connection's driver and `Queue::connection_names()` lists the registered
  ones; `Queue::size()` and the other counts and listings read the default
  connection. While no connection is registered nothing changes: every push
  reaches the one driver, and a connection name is a label. Once one is
  registered, a push to a name that is neither registered nor the default's is
  an error and pushes nothing, and a push that waits for a commit is refused
  before the commit. A chain runs on the connection of its first job, because
  the worker enqueues the next link in the step that settles the one before
  it; `Queue::chain().dispatch()` refuses a chain whose links resolve to
  different connections. `QUEUE_CONNECTIONS=redis,database` registers one
  connection per entry from the environment, each named for its driver. One
  driver has one label: an entry that names the driver `QUEUE_DRIVER` selects
  is a second name for the default connection, so a pause or a
  `Queue::forward_on` set under either name reaches the whole queue, and an
  entry that would put a second connection over a Redis stream or a jobs table
  that is already in use is refused at boot.
- **`Context` travels with queued work.** A value added to `Context` during a
  request was gone when a queued job, a queued mail or notification, or a
  queued event listener ran. A push now takes a `ContextSnapshot` of the
  visible and hidden bags and stores it on the envelope, and the worker runs
  the job inside a scope restored from it. The snapshot is taken when the push
  is made, so a push that waits for a commit carries it too. The lifecycle
  events around the job, such as `JobProcessing`, `JobProcessed` and
  `JobFailed`, are dispatched in the same scope, so a listener reads what the
  job read and what the job added. Every link of a chain and every job of a
  batch gets the snapshot of the code that dispatched it, and a queued
  listener gets the snapshot of the dispatch. The work runs on a copy, so what
  a job adds reaches neither the request nor the next job, and every attempt
  starts from the snapshot. Under the sync driver the copy shadows the
  caller's context for the length of the job. The query bag does not travel,
  so a job run inline no longer reads the request's query parameters. A job
  queued while a request is served carries the request's id as `_request_id`,
  which the request middleware adds to the context; a push made outside a
  request with nothing in its context writes the envelope it wrote before.
  `Context::dehydrating` and `Context::hydrated` register callbacks for the
  two ends, as Laravel's hooks of those names do, with `Context::dehydrate`,
  `Context::hydrate` and `Context::restored` as the functions behind them.
  `Envelope` gains the public field `context`, so code that builds one with a
  struct literal has to name it; an envelope written before the field existed
  decodes without context. Hidden values reach the queue store and the
  failed-job store, because the job needs them. They do not reach a log: the
  envelope a worker logs when no failed-job store is bound carries no hidden
  context, and the `Debug` output of `ContextSnapshot` and `ContextStore`
  names hidden keys only, and that of `FailedJob` gives the size of the
  envelope and not its text.
- **`ThrottleRequestsMiddleware::default()`.** The plain `throttle` alias had
  no limit to register: every constructor asked for a limit, a window and a
  key prefix, and the manual registered the alias with a `default()` that did
  not exist. The default is 60 requests a minute, counted for each signed-in
  user and for each client IP when nobody is signed in, the shape of Laravel's
  default `api` limiter. The numbers are
  `ThrottleRequestsMiddleware::DEFAULT_MAX_ATTEMPTS` and
  `DEFAULT_DECAY_SECONDS`. The user's bucket follows the user across routes,
  and the address's bucket is shared by every route; `.prefix(...)` gives a
  group of routes a budget of its own.
- **`RateLimitMiddleware::ip_based(max_requests, window)`.** The per-IP limit
  that every login form and public API needs took a backend `Arc`, a
  `SlidingWindowConfig` and a hand-written key closure, and the manual used an
  `ip_based` that did not exist. It uses the rate limiter the application
  installed, the one `RATE_LIMIT_DRIVER` selects, looked up when a request
  arrives, so the middleware can be built where routes are registered. The key
  is the address `Request::ip()` resolves through the trusted proxies, and it
  names the limit as well, so two limits with different numbers share no
  bucket. A request with no address to resolve gets a bucket of its own.
  `on_backend_error`, `only_when` and `key_reads_body` chain onto it; when no
  limiter is installed the backend error policy decides.
- **Middleware by name on routes.** Aliases and groups could be registered and
  resolved, but no route took a name: every `.middleware(...)` wanted a type,
  and nothing parsed `throttle:60,1`. Routes, groups of routes and the macro
  builders now have `.middleware_named("auth")`, which takes an alias, an
  alias with arguments, or a group that adds every middleware of the group in
  order. The name is resolved when the route is registered, so a name that is
  not registered stops the boot and never a request;
  `.try_middleware_named(...)` returns the error.
  `register_middleware_alias_with_args(name, |arguments| ...)` registers an
  alias that reads the arguments after the colon, and a group may list such an
  alias. `try_resolve_middleware_alias` says why an alias gave no middleware.
  `ThrottleRequestsMiddleware::from_alias_args` is the factory for the
  `throttle` alias: `throttle`, `throttle:60`, `throttle:60,5`,
  `throttle:60,5,prefix` and `throttle:api` for a named limiter.
- **Route groups take a name prefix and a controller.** Every route in an
  `admin.` group had to spell its full name, and every handler its full path.
  `group!(...).name("admin.users.")` now puts the prefix in front of the name
  of every route in the group, and a group inside adds its own after it; a
  route without a name stays without one. `group!("/admin/users", controller =
  controllers::admin::users, { get!("/", index), post!("/", store) })` names
  the module the handlers live in, so a route names its handler by function
  alone. A handler written as a path is taken as it is written, and a group
  inside names its own controller. The path prefix stays the first argument of
  the macro.
- **Optional route parameters and parameter constraints.** `/posts/{id?}` now
  matches `/posts` and `/posts/42`, and the handler finds no `id` on the short
  form. Several optional parameters fill from the left, the colon spelling
  `/posts/:id?` works, and an optional parameter can only be followed by
  optional ones. The route's middleware and name apply to every form, and
  `route(...)` leaves an optional segment out when it has no value. A route
  can hold a parameter to a constraint: `.where_number("id")`, `.where_alpha`,
  `.where_alpha_numeric`, `.where_uuid`, `.where_ulid`, `.where_in("status",
  [...])` and `.where_pattern("year", "[0-9]{4}")`, on `Router` routes and on
  `get!`, `post!` and the other route macros, where a constraint may name a
  parameter of the group's prefix. A value the constraint refuses is a 404, as
  if the route had not matched, and neither the route's middleware nor its
  handler runs. On a `Router` route `.try_constrain(param, ParamConstraint)`
  returns the error a constraint on a parameter the route does not have gives;
  `.constrain` and the `where_*` spellings stop the boot on it. A pattern has
  to match the whole value, and `\d` matches the digits of every script, so
  write `[0-9]` for ASCII digits. A WebSocket route takes optional parameters
  as well. A pattern with an optional parameter and an empty segment, and a
  second route for one form of an optional route, are refused when they are
  registered.
- **Console commands can be tested for what they print and ask:
  `console::test`.** `dispatch_argv` returns a `Result` and nothing else, so a
  test could tell that a command ran and not what it said, and a command that
  asks a question waited on the standard input of the test runner.
  `console::test(["users:purge", "--days", "30"]).expects_question("Delete 12
  users?", "yes").run().await` runs the command through the dispatcher the
  console binary uses and returns a `ConsoleRun` with `output()`, `errors()`,
  `exit_code()`, `error()` and `unasked_questions()`, and with
  `assert_successful`, `assert_failed`, `assert_output_contains`,
  `assert_errors_contain` and `assert_every_question_was_asked`. Help, the
  version, parse errors and the error of a failed command are collected as
  well. A command prints with `console::line` and `console::error_line` and
  asks with `console::ask` and `console::confirm`; what it prints with
  `println!` a test cannot see. Questions have to come in the order the test
  gave them, and a question with no prepared answer fails the command. The
  framework's own commands, `db:seed` and `model:prune` among them, print
  through the console now, and `make:command` generates a command that does.
- **`Schedule::command("emails:send --force")` puts a console command on the
  schedule.** The schedule took a `Task` or a closure, so a command of the
  application, or a builtin such as `model:prune`, could only be scheduled by
  wrapping it by hand. `command` takes the line the way it is typed behind the
  name of the console binary, splits it into words the way a shell does, and
  returns the `TaskBuilder` every other task uses, so `daily()`,
  `without_overlapping()` and `on_one_server()` apply. The command runs in the
  scheduler's process. The line is checked when the schedule is built: a name
  no command has, arguments the command does not take, and a quote that is not
  closed stop the boot, and `try_command` returns the error. The task is named
  by its command line and described by the command's about text.
- **`suprnova db:seed` and `suprnova model:prune`.** The console binary has
  had both commands, and the manual shows them through the CLI, but the CLI
  forwarded `migrate` and its siblings and not these two, so the step after
  `suprnova migrate` was `cargo run --bin console -- db:seed`. `suprnova
  db:seed` runs every seeder, and `suprnova db:seed UserSeeder` or
  `--class=UserSeeder` runs one. `suprnova model:prune` takes `--model=<Name>`
  and `--pretend`. Both run the project's console binary, which checks the
  names.
- **`QdrantVectorDriver::from_env()` and `MariaDbVectorDriver::from_env()`.**
  Only the Pinecone driver read its configuration from the environment, so
  choosing the vector store by environment needed hand-written `std::env::var`
  code for the other two. The Qdrant driver reads `QDRANT_URL` and, when it is
  set, `QDRANT_API_KEY`. The MariaDB driver reads `MARIADB_URL`, and
  `DATABASE_URL` when that is not set and names a MariaDB or a MySQL database,
  which is the setup with one engine for rows and vectors. A `DATABASE_URL` of
  another engine is not taken. A `mysql://` URL is taken as well, because a
  MariaDB is written that way too. The Qdrant client is built without its
  version check, which connects and blocks while the application boots. A
  variable that is missing is named in the error at boot, and the error never
  shows a URL.
- **Storage configures an S3 disk from the environment.** Mail, the queue and
  the rate limiter read their configuration from `.env` when the server boots;
  storage read no variable, so an S3 or S3-compatible disk (MinIO, RustFS, R2,
  B2) existed only when the application built an `S3Config` by hand, and the
  `.env` the Docker guide shows configured nothing. When `S3_BUCKET` is set,
  the server now registers an S3 disk named `s3` from `S3_BUCKET`,
  `S3_REGION`, `S3_ENDPOINT`, `S3_ACCESS_KEY`, `S3_SECRET_KEY` and `S3_ROOT`.
  With no keys set the driver uses the default credential chain of AWS, and
  `AWS_REGION` is read when `S3_REGION` is not set. **What to check when you
  upgrade:** an application that already sets `S3_BUCKET` for a disk of its
  own gets the `s3` disk as well, and the server does not boot when the
  variables describe no usable disk: no region, or one key without the other.
  A disk the application registered under the name `s3` is left as it is, and
  the variables are not read then. `S3Config::from_env()` returns the same
  configuration for a disk with a name of your own, and
  `filesystem::bootstrap_from_env()` is the function the server calls; the
  console binary boots no driver of the environment, so an application whose
  commands use the disk calls it in its own bootstrap. There is still no
  default disk, and `FILESYSTEM_DISK` is not read.
- **`Storage::url(disk, path)` returns the public URL of a file.** Storage had
  presigned links that expire, `temporary_url` and `temporary_upload_url`, and
  nothing for a file that is meant to be public, so applications built those
  URLs by hand and repeated the base URL of the disk wherever they did. A disk
  gets its public base URL with `Storage::set_public_url("public",
  "https://cdn.example.com/files")`, or a path of the application's own host,
  `/storage`. `Storage::url("public", "avatars/7.png")` joins the two and
  writes every character of the path that a URL gives a meaning to as `%XX`,
  so `c++ notes.pdf` is `c%2B%2B%20notes.pdf`. A disk with no public base URL
  returns an error, so a private disk hands out no link that can be guessed,
  and a path with a `.` or `..` segment is refused. A base URL is refused when
  it has a user or a password, a query or a fragment, a backslash, a `.` or
  `..` segment, or another scheme than `http` and `https`, and the error does
  not repeat the URL. `S3_PUBLIC_URL` gives the disk of the environment its
  base. The function takes the name of the disk, because `Storage::disk`
  returns the `opendal` operator itself, which carries no name.
- **`DB::monitor(max)` and the `db:monitor` command dispatch `DatabaseBusy`.**
  The event was public, exported from the crate root and listed with the
  database events the framework fires, and no code dispatched it: an
  application that listened for it to alert on a database that runs out of
  connections waited for an event that never came. `DB::monitor(max)` asks the
  server of each connection, the default one and every named one, how many
  connections it has from every client, and dispatches `DatabaseBusy` for a
  server with `max` or more. PostgreSQL counts `pg_stat_activity`, and MySQL
  and MariaDB give `threads_connected`, the way Laravel's `db:monitor` asks.
  SQLite has no server and is never busy. The server does the counting, so the
  answer is the same from every process, and the check can run on the
  schedule: `schedule.command("db:monitor --max 80").every_minute()`. Nothing
  runs it by itself. `DB::connection_counts()` returns the numbers and
  `DbConnection::server_connections()` the number of one connection.
  `DbConnection::connections_in_use()` is the other number, the connections of
  this process's own pool that are out of it now.
- **`suprnova::fake` is the `fake` crate, for factories written by hand.** The
  crate root had `Dummy`, `Fake` and `Faker`, and everything else a factory
  uses, the fakers of `fake::faker` and `rand::Rng` for `fake_with_rng`, was
  reachable through the hidden `suprnova::__fake` alone, which the factories
  chapter showed as the supported path. An application that added `fake` to
  its own dependencies had to keep its version the one of the framework, or
  the traits did not line up. `use
  suprnova::fake::faker::internet::en::SafeEmail` and `use
  suprnova::fake::rand::Rng` now work. `__fake` stays for the code
  `#[derive(Factory)]` generates. `#[derive(Dummy)]` generates code that names
  the crate `::fake`, so in an application without `fake` among its
  dependencies the struct says where the crate is: `#[dummy(crate_name =
  "suprnova::fake")]`.
- **`WorkflowWorker::try_with_config(config)` and
  `workflow::assert_no_duplicates()`.** `WorkflowWorker::new()` refuses to
  start when two `#[workflow]` functions have one name. `with_config` skipped
  that check and told the caller to run `registry::assert_no_duplicates`, a
  function of a module that is hidden from the documentation and was exported
  nowhere, so a worker with a config of the application's own lost the check
  without a word. `try_with_config` checks the config and the registry and
  returns the error where `new()` panics, and `assert_no_duplicates` is
  exported from `suprnova::workflow`. `with_config` stays the constructor with
  no check, and says so. `new()` makes its checks through `try_with_config`,
  so the two cannot come apart.
- **`Crypt::decrypt_string_with_origin` and its siblings return where a value
  came from.** `DecryptOrigin`, `KeyOrigin` and `AadVersion` were public, and
  every function that returned one was hidden, so the two questions of a key
  rotation, does this value still need a previous key and does it still use
  the legacy label, were answered in log warnings alone.
  `Crypt::decrypt_string_with_origin`, `Crypt::decrypt_string_for_with_origin`
  and `Crypt::decrypt_with_origin` return the value and its `DecryptOrigin`,
  and `DecryptOrigin::needs_reencryption()` says whether the value is to be
  written again. With them the job that ends a rotation can be written: read
  every encrypted value, write again what needs it, and remove the previous
  key when nothing does. For tests, `crypto::testing::encrypt_string_under`
  and `encrypt_string_for_under` write a value under a key and a label of the
  test's choice; they are compiled with the `testing` feature and replace the
  hidden `_test_encrypt_with` names the manual pointed at. The three types are
  exported from the crate root. An error for a decrypted value that does not
  decode as JSON no longer quotes the value: it says which kind of mistake it
  was and at which line and column. That is the error of `Crypt::decrypt`, of
  a pagination cursor, and of the casts `AsEncryptedArray`,
  `AsEncryptedObject` and `AsEncryptedCollection`, whose error is a validation
  error and was shown to the client with the decrypted value in it.
- **Metric attributes have types.** `inc_with`, `record_with` and `set_with`
  took `&[(&'static str, &str)]` and sent every value as a text, so a status
  `404` matched no filter on a number and no range, and the attributes the
  semantic conventions type as a number or as yes and no arrived as the wrong
  type. A value is now anything that becomes an `AttrValue`: a text, a whole
  number, a number with a fraction, or a `bool`.
  `counter.inc_with(&[("http.response.status_code", 404)])` sends a number.
  Values of one type are written as they are, and values of several types are
  each made an `AttrValue`: `("route", AttrValue::from("/posts")), ("error",
  AttrValue::from(true))`. Every call that passes texts compiles as before. A
  `u64` or `usize` that no `i64` holds is sent as the largest `i64`. A text is
  also a reference to what holds one, `&String`, `&&str`, `&Box<str>`,
  `&Arc<str>`, `&Rc<str>` and `&Cow<str>`, as it was when the values were
  `&str`. Two calls have to be written another way: an empty list,
  `inc_with(&[])`, is `inc()`, and a value that is `.as_ref()` names its type,
  `.as_str()`. `AttrValue` is `#[non_exhaustive]`.
- **A WebSocket handler sends from another task: `WsSocket::sender()` and
  `WsSocket::split()`.** Every method of `WsSocket` takes `&mut self`, so
  while a handler waited in `recv` nothing could send on the connection, and a
  broadcast, a timer or a finished job had to go through a `select!` loop and
  a channel of the handler's own. `sender()` returns a `WsSender`: it clones,
  its `send_text`, `send_binary` and `close` take `&self`, and every clone
  sends on the same connection. `split()` takes the socket apart into a
  `WsSender` and a `WsReceiver`, and the `WsReceiver` is also a `Stream` of
  the messages. The connection does not wait for its senders: when the handler
  returns the connection closes, every later send returns an error,
  `is_closed()` returns `true` and `closed()` completes, so a task that keeps
  a sender learns when to stop. The receiving half has to be read, because the
  answer to the heartbeat's ping arrives there.
- **An upload policy says which rule it breaks: `UploadPolicy::validate()` and
  `UploadPolicyError`.** A Live upload field whose policy the engine refused
  failed the registration of its component with one closed error,
  `invalid_component_upload_metadata`, whichever of the rules was broken: a
  limit that was not set or was zero, a media type or an extension that was
  not in canonical form, a media type declared twice, a dimension of zero, a
  finalize action that was missing or was no name. `UploadPolicy::validate()`
  returns the rule as an `UploadPolicyError`, and the registration logs it as
  an error before it fails. The error of the registration is what it was, so
  nothing of a component reaches a browser through it.
- **A Live operation that fails inside the engine says why.** The engine
  answered every failure of one of its own parts with one reason for the
  browser, `ExecutionFailed` or `LedgerUnavailable`, and threw the error away,
  so a panic of a component, a clock that gave no time and a ledger that was
  full read the same in every log. The browser still gets that reason and
  nothing more. `RefreshRequiredExecution::cause()` returns an
  `ExecutionFailure`, a closed value that names the part that failed and the
  kind of its error: the action, the lifecycle, the ledger, the clock, the
  snapshot, the view, the composition, a port of the host. It is `None` for a
  refresh that is no failure, such as a stale revision. The framework writes
  one warning for each operation that ends with a cause, with the component,
  the reason and the cause. `MountError::cause()` returns a `MountFailure` in
  the same way for a mount, private or public, and `MountError::ledger_kind()`
  is the kind of the ledger when the ledger refused. Neither value holds a
  text, an identifier of an instance or anything of a request.
- **`load` and `load_missing` on one model.** Both were methods of a
  collection only, so code with one `Post` in hand wrapped it in a collection
  of one to load its comments, or ran the query of the relation by hand and
  lost the cache of the model. `post.load(["comments"]).await?` and
  `post.load_missing(["comments.user"]).await?` load into the model itself,
  through the loader of the collections: nested names, the one query for each
  relation and the transaction of the caller are the same. `load_missing` runs
  no query for a relation that the model has.
- **`PaddleProvider::archive_customer` archives a Paddle customer.** Paddle
  has no endpoint that deletes a customer, so `delete_customer` of the Paddle
  adapter returns `PaymentError::NotSupported`, and its message named a way
  that did nothing: an update with `metadata: {"status": "archived"}` writes a
  key named `status` into the custom data of the customer, and the customer
  stays active. `archive_customer(provider_customer_id)` sets the status of
  the customer to `archived` at Paddle. A customer that Paddle does not know
  is `PaymentError::NotFound`. The message of `delete_customer` names
  `PaddleProvider::archive_customer`. What you have to change: code that
  archived a customer through `metadata` calls `archive_customer`, and removes
  the `status` key from the metadata if it does not want it there.
- **A supported way to test a cached route:
  `render_cache::testing::RenderCacheProbe`, `RenderCacheConfig::with_clock`
  and `database::testing::StatementCounter`.** A test that has to prove that
  the render cache served a response had the hidden hooks of the framework's
  own tests and nothing else: `key_for_route_for_test`,
  `inspect_route_for_test`, `inspect_l1_for_test`, `clear_l0_for_test`,
  `with_clock_for_test`, `observe_statements_for_test`.
  `RenderCacheProbe::route(pattern)`, with `.params(..)` and `.at_epoch(..)`,
  gives the key the route derives (`key()`), the entry of each tier for that
  key (`l0()`, `l1()`), and `RenderCacheProbe::clear_l0()` empties the first
  tier and leaves the second as it is. The probe derives the key with the
  function the middleware uses, so you tell it each dimension the policy of
  the route varies on: `.login(id)` for `Principal`, `.tenant(id)` for
  `Tenant`, `.locale(tag)` for `Locale` (the current locale of the process
  when you leave it out) and `.host(host)` for `Host`; a route that varies on
  `Host` and a probe with no host is an error that names the dimension. `l0()`
  reads through `MemoryRenderStore::peek`, which does not count as a use, so a
  probe between two requests does not change what the first tier evicts next;
  `RenderCache::inspect` counts as a use. `RenderCacheConfig::with_clock`
  installs the clock the runtime reads, so a test moves an entry through its
  freshness bands without a sleep. `StatementCounter::install(&mut
  connection)` counts the prepared statements run through a connection, so a
  test shows that a request the cache served ran none; unprepared SQL and
  transaction control are not counted. All of it is compiled with the
  `testing` feature, and no function of it panics: an error names the route
  and never a login, a parameter or a key. What you have to change: a test
  that called `RenderCacheConfig::with_clock_for_test` calls `with_clock`. The
  other hooks stay for the framework's own tests, and are compiled with the
  `testing` feature only.
- **A binding that lives for one request: `App::scoped`.** The container had
  bindings for the process and overrides for a test, and nothing between them,
  so a service that belongs to one request, such as the database handle of the
  tenant or an API client bound to the request, was global and leaked between
  requests, or was built by hand in every handler. `App::scoped::<T>(factory)`
  and `App::bind_scoped` register a binding whose factory runs at most once in
  a scope, at the first `App::get`, and whose value is dropped when the scope
  ends. The framework opens a scope for each request, each WebSocket session,
  each attempt of a queued job, each attempt of a queued listener, each run of
  a scheduled task, a workflow or a supervisor, and each console command. An
  after-commit callback, a hook that runs after the response and the body of a
  streamed response share the scope of the request that registered them.
  `App::run_scoped(future)` opens a scope of your own,
  `App::in_current_scope(future)` carries the scope of the caller into a
  future that you spawn, and `App::spawn_scoped` does both steps. Outside a
  scope, `App::resolve` returns an error that names the type, and `App::get`
  logs a warning and returns `None`: a scoped binding never builds a value
  that lives for the process. A test override wins over a scoped binding. A
  factory is synchronous and runs once: a second task of the scope that asks
  for the value while it is built waits for it on its thread, and a cycle of
  scoped factories, in one task or across tasks, is an error.
- **The permissions of a user answer the gate: `rbac::register_gate_bridge`.**
  The roles and permissions of RBAC and the gate did not know each other, so
  `Gate::allows_async("edit posts", ..)` did not see a permission that the
  user holds, and an application had two systems of authorization side by
  side. `suprnova::rbac::register_gate_bridge::<User>()`, called once in the
  bootstrap, makes every permission that a user holds, directly or through a
  role, an ability of the gate. An ability that is no permission of the user
  goes on to the gate definitions and the policies, so the bridge allows and
  never denies. It reads the permissions of a user once for a request, through
  `GateBridgeMiddleware`, which the call installs as the first global
  middleware. A grant or a revocation made before the first check of a request
  is seen by that check; after it, from the next request on. A check inside
  `DB::transaction` reads for itself and keeps nothing, so a grant that is
  rolled back does not answer after the rollback. A unit of work that runs
  inside a request in a scope of its own, such as a job that the sync queue
  driver runs inline, reads for itself too. A read that fails is logged and
  allows nothing. The permissions answer the async forms of the gate,
  `allows_async`, `authorize_async` and `inspect_async`. The forms without
  `async` cannot wait for a database and skip them. `Gate::before_async` is
  the hook the bridge is built on, and an application can register hooks of
  its own with it. A `Some(false)` from an async hook denies on the async
  forms only: a hook that must deny on every form belongs in `Gate::before`.
  Without the call nothing changes.
- **An application registers guards of its own: `Auth::extend` and
  `Auth::via_request`.** `Guard` was a public trait, but the manager built
  guards from two drivers, session and token, so a guard for an API key, a
  client certificate or a single sign-on could be written and not registered,
  and `Auth::guard("api_key")` and the middleware that takes a guard name
  could not use it. `Auth::extend(driver, factory)` registers the factory of a
  driver, and `GuardConfig::custom(driver, provider)` declares a guard of it.
  The factory gets the name of the guard and its provider.
  `Auth::via_request(name, resolver)` is the short form for a guard that reads
  the request: the resolver gets the request and answers with the user, and
  the guard is declared with
  `GuardConfig::custom(AuthManager::via_request_driver(name), provider)`. The
  middleware for the guard runs the resolver once for a request. Outside that
  middleware the guard reports no user, as the token guard does. An error of a
  factory or of a resolver fails the request with 500 and is never a guest. A
  guard of the application is read-only through the manager:
  `Auth::stateful_guard` returns an error for it, and so do `Auth::logout`,
  `Auth::login_id` and `Auth::login_remember` when such a guard is the default
  guard, before they change anything. A guard of the application has no `:` in
  its name: resolving such a guard, or registering a resolver for it with
  `via_request`, is an error. A logout forgets the users that the guards of
  the application resolved in the request, so the request does not sign itself
  back in. Declared as the default guard, a guard of the application answers
  `Auth::user` and the unnamed `AuthMiddleware::new()`. The principal that the
  middleware attests for Live is `<guard>:<id>` for a guard of the
  application, so the same id under two guards is two principals; a session
  user attests its bare id. What you have to change:
  `AuthManager::via_request` returns a `Result`. `GuardDriver` has the variant
  `Custom(String)` and is no longer `Copy`, so code that copies a driver
  borrows it or clones it, and a `match` that names every variant has one more
  to name.
- **`suprnova::eloquent::prevent_lazy_loading(true)` refuses a relation read
  that runs one query for each row of a list.** A template that reads
  `post.author()` for each of 50 posts runs 51 queries, and nothing said so
  until the load of production. With the switch on, a relation read on a model
  that came out of a query that returned more than one row, when the relation
  was not loaded with `with(..)`, `load(..)` or `load_missing(..)`, runs no
  query and returns an error that names the model and the relation. A model
  from `find`, `first`, a `get` that returned one row, or `create` reads its
  relations as before, as in Laravel. `count()` of a relation is not refused.
  `handle_lazy_loading_violation(handler)` registers one handler for the
  process, which gets a `LazyLoadingViolation` with the names of the model and
  the relation; with a handler the read goes on, so a staging system can log
  the reads and keep serving. `clear_lazy_loading_violation_handler()` removes
  it, and `preventing_lazy_loading()` reads the switch. The switch is off by
  default, and with it off every read behaves as before. Turn it on in the
  bootstrap of the application outside production.
- **A test moves the clock the framework reads: `suprnova::clock::now()` and
  `suprnova::testing::TestClock`.** The framework read the wall clock with
  `chrono::Utc::now()` in about 170 places, and `tokio::time::pause` moves the
  timers of Tokio and not those reads, so a test of a signed URL that expires,
  a session that idles out, a scheduled task that is due, a window of a rate
  limit or a model that becomes prunable had to sleep. Every such read goes
  through `suprnova::clock::now()`, and so do the timestamps the `#[model]`
  macro writes, the due check of a scheduled task without a time zone and its
  one-run-a-minute gate, and the touched-at stamp of the session. Without the
  `testing` feature `now()` is `Utc::now()` and nothing else. With it,
  `TestClock::freeze()` and `TestClock::travel_to(at)` stop the clock of the
  current thread at a time you move with `advance` and `set`, until the guard
  is dropped, and `TestClock::scope(at, |clock| future)` holds a time across
  `.await` and on a multi-thread runtime; `clock.run(future)` carries it into
  a task you spawn. A clock of one test never reaches another test that runs
  beside it. Three reads keep the wall clock, because each is compared with a
  clock a test cannot move: the health endpoint, the retry time of a workflow
  run (compared with `NOW()` of the database), and the window of the
  `RateLimiter` facade (its cache key expires in real time). The in-memory
  driver of `RateLimitMiddleware` measures with `tokio::time::Instant`, which
  `tokio::time::pause` moves, and the Redis driver reads the clock. What you
  have to change: code of your own that reads the time for a decision reads
  `suprnova::clock::now()` when its tests should be able to move it.
- **`suprnova::schema::Schema` writes a migration without an identifier enum
  or `ColumnDef` chains.** A migration can build its tables with
  `Schema::create(manager, "posts", |t| { t.id(); t.string("title");
  t.timestamps(); })` and `Schema::table`, `Schema::drop`,
  `Schema::drop_if_exists`, `Schema::rename`, `Schema::has_table` and
  `Schema::has_column`. The layer builds SeaORM's `Table::create()`,
  `Table::alter()`, `Index` and `ForeignKey` statements and runs them on the
  `SchemaManager` the migration is given, so every statement runs on the
  migration's own connection and transaction, and a SeaORM migration and a
  `Schema` migration can sit in one `Migrator`. `Blueprint` has `id()` (a
  `BIGINT` auto-increment primary key), `foreign_id(name).constrained(table)`
  with `on_delete` and `on_update`, eighteen column types, the modifiers
  `nullable`, `default`, `unique` and `length`, `index` and `unique` over
  several columns, and in `Schema::table` also `rename_column`, `drop_column`,
  `drop_index` and `drop_foreign`. A column is `NOT NULL` unless it is
  `nullable()`. `timestamps()` and `soft_deletes()` create the `created_at`,
  `updated_at` and `deleted_at` columns as `VARCHAR(255)`, because
  `#[suprnova::model]` stores a `DateTime<Utc>` field as RFC 3339 text unless
  the field declares a cast; `timestamp_tz` with a native cast gives a native
  column. On SQLite, adding or dropping a foreign key on an existing table is
  an error that is returned before any statement of the call runs, and every
  other alteration runs as its own statement. The layer is not re-exported at
  the crate root, where `suprnova::Schema` is SeaORM's `Schema`.
  `make:migration` generates SeaORM migrations as before.

### Changed

- **Magnetar's `CeremonyStore` takes a named request for its atomic
  transition.** `transition_and_consume` and `transition_and_consume_exact`
  took six and seven positional strings, so a transposed selector or state
  compiled and acted on the wrong ceremony. Both take a `TransitionAndConsume
  { transition, expected, next, consume }` whose two ceremonies are
  `CeremonyRef { selector, kind }`, and the exact form takes the consume id as
  its second argument. A store that overrides either method changes its
  signature to match.
- **`delete_all` on a soft-delete model soft-deletes.** It issued `DELETE` on
  every model, so `Post::query().filter(...).delete_all()` permanently removed
  rows that a row-level `delete()` would have trashed. On a model declared
  with `soft_deletes` it now writes the tombstone, and `updated_at` when the
  model manages timestamps, and the rows stay readable through
  `with_trashed()`. `force_delete_all()` is the explicit hard delete.
  `Builder::without_global_scope::<S>()` and
  `Builder::without_global_scopes()` are supported chain methods.
- **`#[scopes]` refuses a scope written in the wrong form.** A method in a
  `#[scopes]` block that took `&mut Builder<User>`, named the model instead of
  `Self`, or returned nothing was left an ordinary method, and no scope
  reached the builder. It is now a compile error on the signature that names
  the accepted shape, `fn name(query: Builder<Self>, ...) -> Builder<Self>`. A
  method that handles no builder still passes through unchanged, and
  `#[not_scope]` marks a helper that does handle one.
- **`suprnova serve` runs the pending migrations once, when it starts, and no
  longer on every save.** The backend runs under a file watcher, and an
  application started with no subcommand migrates before it serves, so every
  save of a source file ran every pending migration against the developer's
  database, the draft of a migration saved a moment ago included. A migration
  that has run is not run again when its file changes, so the database kept
  the schema of the first draft. `serve` now runs `migrate` once before the
  backend starts and starts the watched backend with `serve --no-migrate`. The
  frontend and the processes of `Suprnova.toml` start first and do not wait
  for it. When a file under `src/migrations` changes after that, `serve` says
  that it was not run and that `suprnova migrate` runs it; with `--json` that
  is the new event `{"type":"migrations_changed","ts":...}`, and the run at
  the start shows as a process named `migrate`. `--migrate always` restores
  the old behaviour, and `--migrate never` or `--no-migrate` runs no migration
  at all. When the run at the start fails, the backend is left to migrate by
  itself for that session, as before: it does not serve until the migrations
  pass. A project with no `src/migrations` directory is left to migrate by
  itself as well.
- **`seed::clear()` is a supported function of the `testing` feature.** It was
  hidden from the documentation as an internal helper, and it is the only way
  for the tests of an application to reset the registry of the seeders, which
  is one for the process and which the guard of the test container does not
  reset. The manual told readers to call it and said in the same breath that
  it was hidden. It is documented now and compiled with the `testing` feature
  alone, which is a default feature, so a test suite needs no change. A build
  with `default-features = false` and without `testing` no longer has the
  function.
- **The paginators serialise to Laravel's JSON shape.** `LengthAwarePaginator`
  had `data`, the counters and `path`, and no URL of a page and no `links`;
  the simple paginator had `has_more`, and the cursor paginator had the
  cursors and no URL. A front end that was written for Laravel's paginator
  JSON, the pagination components of the Inertia starter kits among them,
  could not read them. The three now have the fields of Laravel's `toArray()`:
  `first_page_url`, `last_page_url`, `next_page_url`, `prev_page_url` and
  `links` on the paginator with a total, `current_page_url`, `first_page_url`,
  `next_page_url`, `prev_page_url`, `from` and `to` on the simple one, and
  `next_page_url` and `prev_page_url` on the cursor paginator. `links` has the
  window of pages and the labels of Laravel. A URL is the `path` and the page
  parameter, and a `path` with a query string keeps it. Every field that was
  there is still there, and `path` is still left out when it is not set.
  `LengthAwarePaginator::links()`, `next_page_url()` and `previous_page_url()`
  return the same values in Rust, and the other two paginators have their own.
  A page parameter that the `path` has already is replaced, so the URL of the
  current request can be given as the `path`, and a fragment of the `path`
  stays at the end of the URL.
- **A web push subscription that cannot be used is
  `WebPushError::InvalidSubscription`.** An endpoint that is no URL, that is
  not `https`, that names an address in place of a host, or that names a host
  that is no push service was `WebPushError::Internal`, with the text
  `internal:` in front. A key that is not what RFC 8291 asks for was
  `WebPushError::Encryption`: a `p256dh` or an `auth` that is no base64url or
  has the wrong length, a `p256dh` that is not in uncompressed form or is no
  point of the P-256 curve. Bad stored data read as a fault of the crate, and
  no caller could tell the two apart. The new variant says which rule refused
  the subscription and is not retryable. `WebPushChannel` handles it as it
  handles a subscription that is gone: it logs a warning and the dispatch
  succeeds, so a queue does not send the job again, with every channel that
  delivered before this one. A stored route that is no subscription at all is
  handled the same way. It was an internal error. No warning of the channel
  has the endpoint, because the path of an endpoint is the token that reaches
  the browser: a warning has the host and `endpoint_sha256`, the first 16
  hexadecimal digits of the SHA-256 of the stored endpoint, which finds the
  row. The warning for a subscription that is gone had the whole endpoint. The
  errors of the crate's own headers stay `Internal`, and a payload that is too
  large and a failure of the encryption stay `Encryption`. A `match` on
  `WebPushError` that names every variant has one more to name.
- **A database error on the payment webhook path keeps its type:
  `PaymentError::Database`.** The webhook route made a text of every database
  error, `PaymentError::Internal(format!("{e}"))`, in twenty-two places, so
  the code that decides what to do with a failed webhook could not ask whether
  the failure was a lost connection, which the next attempt cures, or a
  violated constraint, which none does. The new variant has the
  `sea_orm::DbErr` as its source, reachable with `error.source()` and
  `downcast_ref`, and `From<sea_orm::DbErr>` makes a statement end in `?`. The
  text of the error is what it was, and the route answers 503 for every
  database error as it did: the variant is what a later decision can be made
  on, and none is made yet. `Debug` prints the variant with the text of the
  database error and not with its `Debug`, which for a violated constraint
  names the values of the key. A `match` on `PaymentError` that names every
  variant has one more to name.
- **`suprnova::payments` names the types it exports.** The module had `pub use
  dto::*;`, the only glob export of the framework. It exported the modules of
  `dto` as well as the types, so every type had two public paths,
  `payments::StartSessionRequest` and
  `payments::session::StartSessionRequest`, and a type that was added to `dto`
  was public in `payments` without anybody deciding so. The twenty-five types
  are named now. The paths through the modules, `payments::session::` and its
  seven siblings, are gone: the modules are reached as
  `payments::dto::session`.
- **`Http::send` decides once whether an attempt is retried.** The loop had
  the decision twice, for a response with a status of the server and for an
  attempt that got no response: the same check of the policy, of the method
  and of the `retry_when` predicate, in two blocks that a change had to keep
  alike by hand. It is one function now, and the two outcomes are retried by
  one rule. What is retried and how long the request waits is what it was.
- **The parser of `#[model]` has no `#[allow(dead_code)]` left.** Sixteen
  suppressions on `ModelInput` and `RelationDecl` said that a later task would
  read the field. The tasks are done and the fields are read, so the
  suppressions hid nothing and would have hidden a field that a later change
  leaves unread. They are removed with the comments that named the tasks.
- **The dogfood application's `bootstrap.rs` imports what it uses.** Its
  import of the framework had a blanket `#[allow(unused_imports)]`, which hid
  that `singleton` was imported and never used, and would have hidden every
  later one. The suppression and the import are removed.
- **The `Data` derive has no module-wide
  `#![allow(clippy::collapsible_if)]`.** The attribute turned the lint off for
  the whole file with no reason given. It is removed, and the nested
  conditions it hid are written as one.
- **`mail::mailable_registry::render_outgoing` takes its values as a struct.**
  The function had fourteen arguments in a row, four of them `Vec<Address>`
  one after the other: the recipients, the copies, the blind copies and the
  reply addresses. Two of them in the wrong order compile, and the mail goes
  to the wrong list. It takes `any`, `mailable_name` and a
  `RenderOutgoingParams` now, whose fields are named. `RenderOutgoingParams`
  implements `Default`, so a caller names the fields it has. A caller of
  `render_outgoing` has to build the struct. The framework has one, the job
  that sends a queued mail.
- **`OutputFormat::WebP` is lossy and honours `quality`, and
  `OutputFormat::WebPLossless` is new.** WebP was always written lossless, so
  `.quality(80).format(OutputFormat::WebP)` gave the same file for every
  quality, and for a photo a larger file than the JPEG it replaced. `WebP` is
  lossy now, at the quality of the pipeline, which is 70 when none is set. The
  built-in driver writes it lossless in two cases, because the lossy form of
  WebP cannot hold the image: when a pixel is not fully opaque, and when a
  side is longer than 16383 px. `WebPLossless` is always lossless and ignores
  the quality. Both have the content type `image/webp` and the extension
  `webp`. The ImageMagick driver writes `WebP` lossy with its alpha channel,
  and passes `-define webp:lossless=true` for `WebPLossless`. What you have to
  change: a `match` on `OutputFormat` that names every variant, as a driver of
  your own has, needs an arm for `WebPLossless`. Code that needs the pixels of
  a WebP to be exact asks for `WebPLossless`. A WebP source that is resized
  and not converted is written as `WebP`, so an opaque one is written lossy.
- **`Subscription::update` changes the prices of a subscription on Stripe and
  on Paddle, and the Paddle adapter cancels at once when it is asked to.**
  `update` with `new_price_refs` returned `PaymentError::NotSupported` on both
  adapters, so a change of plan was a cancellation and a new subscription. The
  list is the set of prices the subscription has after the call: an item whose
  price is in the list keeps its id and its quantity, an item whose price is
  not in the list is removed, and a price that is new is added with the
  quantity 1, except in a swap: when the change removes exactly one item and
  adds exactly one price, the new price takes the quantity of the item it
  replaces, so ten seats of one plan become ten seats of the other. The
  adapter computes the change from a read of the subscription, and the change
  is not atomic: a change made at the provider between the read and the write
  can be undone (Paddle takes the whole list) or left in place (Stripe changes
  items one by one), and the returned subscription shows the result. An empty
  list, and a list that names a price twice, is `PaymentError::Validation`, on
  both adapters and on the mock. The new field
  `UpdateSubscriptionRequest::proration` takes a
  `suprnova::payments::Proration`: `ProrateNow` bills the difference at once,
  `ProrateAtRenewal` puts it on the next invoice, and `DoNotProrate` charges
  nothing for the change. `None` is `ProrateAtRenewal`. On Paddle, a change of
  prices in the same call as `cancel_at_period_end` or with an
  `idempotency_key` is `NotSupported`. `cancel(id, false)` of the Paddle
  adapter cancels the subscription at once, and `cancel(id, true)` cancels it
  at the end of the billing period; it cancelled at the end of the period for
  both. Every call of the Stripe and the Paddle adapter has a deadline of 30
  seconds. A read that gets no answer in that time is a
  `PaymentError::Provider` whose text ends in "timed out". A call that changes
  something is a `PaymentError::Provider` whose text says that the outcome is
  unknown: read the state at the provider before you send the call again. The
  text of an error that the Stripe or the Paddle adapter builds from an error
  of its SDK holds the operation and the kind and code of the provider's
  error, and never the provider's message, an id or a URL: a Stripe API error
  gives its type, its code and the HTTP status, a Paddle API error its type
  and code, and a transport error comes without the URL of the request. Stripe
  `void` of a payment that is already captured is a `Validation` error whose
  text does not name the payment. The `NotFound` errors of
  `MockPaymentProvider` carry no id. What you have to change: every struct
  literal of `UpdateSubscriptionRequest` names the new field, `proration:
  None` to keep the default. Code that called `cancel(id, false)` on Paddle
  and counted on a cancellation at the end of the period passes `true`. A test
  that read an id out of a `NotFound` error of the mock reads it from its own
  request.
- **A polymorphic relation takes a target with any key, loads nested
  relations, and can be touched.** Three limits of `MorphTo` are gone. The id
  of a morph relation was an `i64`, so a model with a `String`, UUID or ULID
  key could not be the target: the id is the value of the key of the target,
  and the `<name>_id` column of the child has the type of that key. All
  targets of one relation have keys of one type, and a relation that mixes
  them does not compile, nor does a child whose `<name>_id` field has another
  type. A parent's `MorphMany` or `MorphOne` is not checked against the
  child's `<name>_id`, so keep that field at the key type of every parent that
  owns it. The lazy read of a `MorphTo` finds the target by its key and
  applies no global scope of the target; the eager load runs the query of the
  target and applies its global scopes. A nested eager load through `MorphTo`,
  `with(["commentable.user"])`, returned an error: the loader groups the
  targets by their type and loads the rest of the path once for each type, and
  a target type that does not have the relation is an error that names the
  type and the relation, whether or not a row of that type was loaded.
  `#[model(touches = [...])]` took `BelongsTo` relations only: it takes the
  name of a `MorphTo` relation, and a save or a delete of the child writes the
  `updated_at` of its owner, inside the transaction of the write when there is
  one. An owner without timestamps is skipped, a soft-deleted owner is not
  touched, and a null `<name>_id` touches nothing. A `<name>_type` that names
  none of the targets is an error of the write: `create`, `save`, `update`,
  `delete` and `force_delete` (and their `_with_tx` forms) resolve every
  `MorphTo` owner of `touches` after the `Creating`, `Saving`, `Updating` and
  `Deleting` listeners have run and before the statement, from the values the
  statement writes, so a listener that rewrites `<name>_type` or `<name>_id`
  decides the owner, and an owner that cannot be resolved returns `Err` with
  no row written, no later event dispatched and no owner touched. An empty
  collection checks a dotted path of `Collection::load` and `load_missing`
  too. What you have to change: `MorphTo::morph_id` and the id in the
  `Unknown` variant of the generated enum are a `serde_json::Value`, so code
  that reads the id as an integer calls `as_i64()`. `<relation>_loaded()` of a
  `MorphTo` returns the generated enum.

### Fixed

- **Magnetar logs a lockout status failure before the sign-in fails closed.**
  The password plugin answered `503` when the lockout store could not report
  an identity's status and logged nothing, while the failed-attempt path
  beside it logged its error. The status path now logs `lockout status
  unavailable; failing closed` with the store's error; the response is
  unchanged.
- **Magnetar logs a failure to record a rejected two-factor attempt.**
  Re-enrollment, confirmation and recovery-code rotation discarded the error
  from the lockout store when they recorded a failed attempt, so a store fault
  during an attack left no trace. Each now logs a warning with the error; the
  rejection itself is unchanged.
- **Magnetar's Redis abuse limiter says why Redis failed.** Every client error
  became `shared abuse-limiter backend failed`, so a refused connection, a
  timeout and a script error read the same in the log of a limiter that fails
  sign-in closed. The message now ends with the Redis client's own description
  of the fault.
- **A stored session payload that fails to parse is logged.** The database
  session driver read a damaged payload as an empty session, which signs the
  visitor out, and logged nothing. It now logs a warning with the parse error
  and without the session id. A failed delete of an expired session row is
  logged as well.
- **A Live request that cannot be prepared says why in the log.** When the
  Live runtime could not be bound or a request could not be prepared, the
  server answered `500 Live request preparation failed` and logged nothing. It
  now logs the error with the route pattern and the stage, on the HTTP path
  and on the WebSocket upgrade path; the response is unchanged.
- **The NOWPayments adapter's HTTP failures name their cause.** A failed
  request, an unreadable response body and a body that is not JSON each became
  a fixed `PaymentError::Provider` message. Each now ends with the underlying
  error, down to the refused connection or elapsed deadline for a transport
  failure, and never includes the request URL, which carries the payment id.
- **A failed Live upload cleanup run is logged.** The background loop that
  retires expired uploads discarded the result of every run, so a store that
  kept failing left the uploads in place with nothing in the log. A failed run
  now logs a warning, and the loop retries on its next interval as before.
- **A model's cast storage aliases stay out of its rustdoc.** `#[model]` emits
  a `pub type __Suprnova_Cast_Storage_<field>` for each cast field, and each
  one appeared in the documentation of the application's own models. They are
  hidden now.
- **A factory insert fires the model's lifecycle events.** `Factory::create`
  and `create_many` inserted through SeaORM directly, so `creating`, `saving`,
  `created` and `saved` never fired and no observer saw a factory's rows. They
  now take the same insert `Model::create` takes: a `creating` observer can
  change the attributes or cancel the insert, and `created` runs for every
  row. `create_quietly()` and `create_many_quietly()` insert with the events
  muted. A model whose key is not auto-increment keeps the key its factory
  set.
- **`DATABASE_URL=mariadb://...` connects.** Only `mysql://` was recognised,
  so the scheme a MariaDB operator naturally writes failed at connect with no
  supporting driver. `mariadb://` is accepted for the primary connection,
  named connections and read replicas, the migrator, and
  `MariaDbVectorDriver::from_url`, and `DB::driver_title()` answers `MariaDB`
  for it. The vector driver's pool error no longer quotes the connection URL,
  which carries the password.
- **`QUEUE_DRIVER=sync` and `QUEUE_DRIVER=null` select their drivers.** Both
  drivers existed, but the environment bootstrap knew only `memory`, `redis`
  and `database`, so `sync` became an in-memory queue with a warning. A
  `QUEUE_DRIVER` that names no driver is now a boot error in production, where
  an in-memory queue taken by mistake loses every job at the next restart;
  elsewhere it still falls back to memory, and the warning lists the accepted
  names.
- **A chained job's `Job::delay()` applies.** A chain built each link's
  envelope with `available_at` set to now, so a job that declares a delay ran
  at once when it was a link of a chain, head or not. `ChainLink` now records
  the delay at build time, as it records the queue, and the link becomes
  available that long after it is reified: at dispatch for the head, and when
  the link before it completes for every other link. `ChainLink` gains the
  public field `delay_secs`, so code that builds one with a struct literal has
  to name it. Chain payloads written before the field existed decode as links
  with no delay.
- **Batches, chains and failed-job retries go through the queue fake.** Only
  the `Queue::push` family checked `Queue::fake()`.
  `Queue::batch().dispatch()` and `Queue::chain().dispatch()` went to the
  driver, so a faked test failed with "queue driver not initialized" when no
  driver was installed and pushed real jobs when one was.
  `Queue::retry_failed` and `Queue::retry_all_failed` did the same. All four
  record in the fake now and write to no driver. A batch is still stored in
  the batch repository, so the id the caller receives names a batch.
  `queue::testing` gains `batched()`, `assert_batched`, `assert_batch_count`,
  `assert_nothing_batched`, `chained()`, `assert_chained` and
  `assert_nothing_chained`, with the records `FakedBatch` and `FakedChain`.
  The jobs of a batch and the head of a chain are recorded as pushes too, so
  `assert_pushed` sees them.
- **The queue worker honors a retry hint.** A job that failed with
  `FrameworkError::RateLimited` carrying a `retry_after` was retried on its
  own backoff, so a queued web push that a push service refused with `429` and
  `Retry-After` went back to the service early, or waited far longer than it
  was asked to. The worker now releases such a job for the hinted time, capped
  at 24 hours (`queue::retry::RETRY_HINT_CEILING`). Every other failure keeps
  the job's backoff. `queue::retry::delay_after_failure` is the function the
  worker calls.
- **The middleware priority list orders the chain.**
  `append_middleware_priority` and `prepend_middleware_priority` recorded a
  list that nothing read, so middleware ran in the order it was registered
  whatever the list said, and an application that relied on the documented
  order could run authentication before the session was loaded. A chain is now
  put in the order of the list when it runs, for a matched route, the
  fallback, an unrouted request and a WebSocket upgrade alike. A middleware
  the list names moves in front of any middleware the list places after it.
  Every other middleware keeps its place, so one registered after
  `AuthMiddleware` still runs after it. The list orders global, group and
  route middleware together. It sees the middleware registered by type; a
  middleware boxed by hand with `into_boxed` and added with
  `.middleware_boxed(...)` keeps its place. An empty list costs one read per
  request.
- **`#[derive(Command)]` without a `description` keeps clap's own about
  text.** The derive called `.about("")` when `#[console(description =
  "...")]` was left out, which replaced the about text clap had taken from the
  struct's doc comment or from `#[command(about = "...")]`. A command
  described the way clap users describe one showed an empty line in the
  console's list of commands. The derive now sets the about text only when a
  `description` is given, and a `description` still overrides the doc comment.
  `CommandEntry::about()` returns the text the help shows, whichever of the
  three it came from; `CommandEntry::description` stays the attribute's text
  and is empty when the attribute has none.
- **`suprnova schedule:list --timezone=<zone>` is accepted.** The
  application's own `schedule:list` has taken `--timezone` since the flag
  shipped, and the manual shows it through the CLI, but the CLI's subcommand
  took no arguments and ended with `unexpected argument '--timezone'` and exit
  code 2. The CLI now takes the flag and hands it to the application, which
  checks the zone name.
- **The scaffolded `src/tasks/mod.rs` shows a task that compiles.** Its
  example implemented a `ScheduledTask` trait with `name` and `schedule`
  methods. The trait is `Task`, its only method is `handle`, and the name and
  the times a task runs at are set where it is registered. Every new project
  received the wrong example, and copying it gave an unresolved import. The
  comment now shows what `make:task` generates, with the registration in
  `src/schedule.rs`.
- **A render cache policy that varies on `FeatureVersion`, `ConfigVersion` or
  `Application(name)` is refused when it is registered.** Nothing gives these
  three dimensions a value, so a route that declared one could not build its
  key and every request for it went past the cache, while the route looked
  cached in the code. `try_render_cache` and `try_render_cache_group` now
  return an error that names the route or the group and the dimension, for a
  full policy and for a patch that brings the dimension in, so the application
  stops at boot. The other six dimensions register as before. This refuses one
  arrangement that cached before: a group whose policy declared one of the
  three, with a patch on every route that replaced the dimensions. The group
  is refused now, and the dimension is to be taken out of its policy.
- **Telemetry is exported, each signal to its own path, and a signal can have
  an endpoint of its own.** With the `otel` feature the OTLP exporters had no
  HTTP client: each one failed to build with `no http client specified`, no
  trace, metric or log left the process, and the error was logged before a log
  subscriber existed, so nothing showed it. The exporters have the blocking
  `reqwest` client now, and an exporter that cannot be built is reported after
  the subscriber is installed. `init_telemetry` also gave the base
  `OTEL_EXPORTER_OTLP_ENDPOINT` to the exporter of each signal, and the
  exporter uses an endpoint it is given in code as it is written, so the URL
  of every signal was the root of the collector, where a collector has
  nothing, and `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT`,
  `OTEL_EXPORTER_OTLP_METRICS_ENDPOINT` and `OTEL_EXPORTER_OTLP_LOGS_ENDPOINT`
  had no effect. Each signal is now sent to its own path under the base,
  `/v1/traces`, `/v1/metrics` and `/v1/logs`, and a signal with a variable of
  its own is sent to that URL as it is written, as the OTLP specification
  says. Traces can go to one collector and metrics to another. The base
  endpoint is still what turns telemetry on. `OtelConfig` has the three
  endpoints as fields, `traces_endpoint`, `metrics_endpoint` and
  `logs_endpoint`, which `from_env` reads, so code that builds an `OtelConfig`
  with every field named has three more to name. A signal whose endpoint is no
  URL is left out and reported, and the other signals are exported; the report
  never has the endpoint in it, which can carry a password or a token. A base
  endpoint that was given a path of a signal to make up for the missing one,
  such as `http://collector:4318/v1/traces`, is to be the base again: the path
  is added to it. An endpoint is a URL with the scheme `http` or `https` and a
  host: a blank base endpoint does not turn telemetry on, and a signal whose
  endpoint is a path alone is left out and reported, where it was built and
  sent nothing. `OTEL_EXPORTER_OTLP_COMPRESSION=gzip` works; `zstd` is not
  compiled in and leaves the signals out, with the reason in the log. What the
  exporters log about their own requests is printed and not exported, so a
  process at debug level does not send lines about its own sending without
  end.
- **The session middleware does not panic on a poisoned lock.** The session,
  the cookies that wait for the response, the remember tokens that wait to be
  revoked and the list of the fresh Magnetar sessions were locked with
  `.lock().unwrap()` in thirty places, which panics when an earlier panic left
  the lock poisoned. The list of the fresh sessions is shared with the
  clean-up task that runs after the request, so a panic in one request could
  end the clean-up of it. The locks are taken with a function that goes on
  with the value of a poisoned lock. The three lists are changed by code of
  the framework alone, which cannot panic between two writes, so they are
  whole after any panic. The session is changed by closures of the
  application: when a closure of `session_mut` panics and a Live action or a
  listener catches the panic, the request goes on and can read the session,
  but the middleware does not store it. It retires a Magnetar session that the
  request issued, takes back a remember cookie that the promotion of a second
  factor issued, answers 500, and the stored session stays as it was, which is
  what such a request did before. The policy of the framework for locks is
  that a poisoned lock is an error or is recovered, and never a panic.
- **`generate-routes` finds a form request by every name it is declared
  with.** The command read the source for `#[derive(FormRequest)]` and for the
  helper attribute `#[form_request(..)]`. An application writes neither: the
  crate root exports the derive as `FormRequestDerive`, because `FormRequest`
  is the trait there, and the short form is the attribute `#[request]`. A
  struct with one of those was left out of the generated routes, so its
  request type was missing in the TypeScript, and no message said so. The
  command reads `#[request]`, `#[derive(FormRequestDerive)]` and
  `#[derive(FormRequest)]`, each bare or behind `suprnova::`, and the helper
  attribute. `generate-types` read `InertiaProps` and `Data` in both forms
  already, and the three detectors ask one function.
- **`chunk_by_id` and `lazy_by_id` walk a table whose key is no `i64`.** The
  cursor of the walk was an `i64`, so a model with a `String` key, or with
  `unique_id = "uuid"`, `"uuid_v4"` or `"ulid"`, got the first batch and then
  an error. The closure had run on that batch by then, and a table that fits
  in one batch never gave the error. The cursor is the value of the key as the
  model has it, so an integer key and a string key both walk, in the order of
  the key. For a key with no order in time, a UUID v4, a row that is inserted
  during the walk with a key below the cursor is not seen by that walk. A key
  that cannot be a cursor is refused before the first query and before the
  closure sees a row: a composite key, and a key column that is no integer and
  no text, such as a native `uuid::Uuid`, a timestamp or a decimal. `lazy()`
  and `cursor()` walk through `lazy_by_id`, so both hold for them. What you
  have to change: a walk over a model with such a key returned `Ok` when the
  table fit in one batch, and returns the error now. Use `chunk()` for it.

### Security

- **Global scopes can no longer be bypassed by a trashed view, a chained
  opt-out, or an `or_where`.** Scopes were applied when `Model::query()` built
  the builder, which left three holes. `with_trashed()` and `only_trashed()`
  started from a bare builder and ran with no global scope, so on a model with
  a tenant scope they returned every tenant's rows. `query().or_where(...)`
  folded into the scope's own term and read `(tenant_id = ? OR ...)`, and on a
  soft-delete model `(deleted_at IS NULL OR ...)`. And
  `query().without_global_scope::<S>()` compiled and did nothing. The
  soft-delete filter and the registered scopes are now folded in when the
  query runs: the statement is `<scopes> AND <your terms>` with an `OR` group
  as one atom, the trashed views lift only the soft-delete filter, and an
  opt-out lands wherever it is chained. `update_all`, `delete_all` and
  `increment_each` resolve the same way, so a mass write reaches only the rows
  a read would return. A scope that reads per-request state reads it when the
  query runs.
- **Behind a proxy, `Request::ip()` is the address the proxy saw, and no
  longer one the client wrote.** A proxy adds the address it saw to the right
  end of `X-Forwarded-For` and leaves what was there, and `ip()` returned the
  left end. Behind nginx, Traefik, HAProxy or a cloud load balancer, a client
  that sent the header itself chose the address the application saw: a new
  rate-limit bucket with every request, and the address of a payment provider
  for a webhook that checks `remote_addr`. `ip()` now reads the header from
  the right and returns the first address that is no trusted proxy. It reads
  every line of the header as one list, reads an entry with a port
  (`203.0.113.5:54321`, `[2001:db8::5]:443`) as its address, and ends at an
  entry it cannot read, where the answer is the proxy that wrote that entry.
  `X-Real-IP` is read only when the request has no `X-Forwarded-For` at all.
  **What to check when you upgrade:** every proxy between the client and the
  application has to be in `APP_TRUSTED_PROXIES`, not the last one alone. A
  proxy that is not listed is taken for the client, and all of its clients
  share one address; that includes the address a load balancer adds behind the
  client's, as the external Application Load Balancer of Google Cloud does.
  `APP_TRUSTED_PROXIES` takes ranges in CIDR form for that,
  `10.0.0.5,173.245.48.0/20,2400:cb00::/32`, which is how the edge of a
  content delivery network is listed; in code it is
  `TrustedProxiesConfig::and_networks` with `ProxyNetwork`. A range must hold
  proxies and nothing else, because a client that connects from a trusted
  address is believed like a proxy, and the range of every address
  (`0.0.0.0/0`) is refused. A proxy that writes `X-Real-IP` alone has to
  remove the `X-Forwarded-For` of the client. An IPv4 address that is written
  as an IPv6 one (`::ffff:10.0.0.5`) is the IPv4 address, in the headers and
  for the peer. `Request::ips()` still returns the whole chain and is a
  record, nothing to decide by.
- **A cap on the WebSocket connections one client address holds open:
  `RateLimitMiddleware::connections_per_ip(n)`.** The only limit on open
  connections was the server-wide `SERVER_MAX_CONNECTIONS`, one number for
  every client together, so a single address that opened WebSockets and kept
  them could use all of it and lock every other client out of HTTP and
  WebSocket alike. The broadcasting and WebSocket chapters showed
  `connections_per_ip` on the `ws!` route; it did not exist. The middleware
  answers `429 Too Many Requests` when the address already holds `n` sockets.
  A socket is counted until its session ends, close frame or none, and an
  upgrade that a later middleware refuses gives its place back at once. The
  address is the one `Request::ip()` resolves through the trusted proxies; an
  IPv6 address is counted with its /64 network, because one client holds a
  whole /64. The counts are kept per process; a clone shares them, so one cap
  can guard several routes, and `open_for(address)` reads a count. On a route
  that is no WebSocket route the cap counts the requests being handled at one
  time and does not cover a streamed response.
  `Request::hold_for_connection(guard)` is the part other middleware can use:
  it keeps a guard alive until the socket of an upgrade ends, where a guard
  the middleware holds itself is dropped at the handshake.
- **Magnetar does not quote decrypted state in an error.** Four places decrypt
  the state of a ceremony or of a device session and decode it, and the error
  for state of another shape was the message of the JSON decoder, which quotes
  the value it could not read. That state holds challenges, session grants and
  tokens, and two of the errors are shown to the client. The error has the
  kind of the mistake and its position now, and nothing of the content. It can
  happen where a deployment changes the shape of the state while a ceremony is
  under way.
- **An encrypted cookie opens under its own name and in no other way.**
  Release 1.3.0 bound the name of a cookie into its encryption and kept a
  fallback that still opened a value written without the name, to be removed
  in 1.4.0, which was never released. While the fallback was there, such a
  value opened in every cookie, so the binding protected nothing against it.
  The fallback is removed for cookies. It stays for the other purposes, where
  it opens stored values that do not expire as a cookie does: read those with
  `decrypt_string_for_with_origin` and write them again. What you have to
  change: `Cookie::read_encrypted(wire)` is removed, use
  `Cookie::read_encrypted_for(name, wire)`. `Crypt::encrypt_string` and
  `Crypt::encrypt` with `CryptPurpose::Cookie` return an error, use
  `Cookie::encrypted(name, value)` or `Crypt::encrypt_string_for`.
  `Crypt::decrypt_string`, `decrypt_string_with_origin`, `decrypt` and
  `decrypt_with_origin` with `CryptPurpose::Cookie` return an error, use
  `Cookie::read_encrypted_for` or `Crypt::decrypt_string_for`. A cookie that
  was written before 1.3.0, or with `encrypt_string`, does not open any more.
  For the session cookie and the remember cookie the user signs in again, and
  for the maintenance bypass the operator visits the secret URL again. No
  request fails for it.

## 2.1.0 - 2026-09-18

### Added

- **Live ships the foundations of its component library.** A token stylesheet
  with a base layer arrives as the `ui-styles` runtime artifact when a
  document opts in with `with_suprnova_ui()`; every rule sits in the
  `suprnova-ui` cascade layer and every visual value is a `--sn-` token with
  light and dark values, so the skin is removable with nothing breaking.
  `live:add` installs a component as one directory under
  `templates/suprnova-ui/` from its manifest, keeps files you edited, and
  accepts a third-party manifest under its own root; the framework serves the
  vendored stylesheet and script at `/suprnova-ui/<component>/<file>`. The
  form family ships as Askama macros the checker now expands, so a library
  view passes `live:check` like any other. The `suprnova.` namespace is
  reserved to the library and the registry refuses it from any other crate.
- **The Live component library gains its overlay family.** Tooltip, collapsible
  and accordion, popover, a single-level dropdown menu, dialog, sheet, and
  drawer install with `live:add` beside the form family. Each owns its open
  state through the native primitive (`details`, the `popover` attribute,
  `dialog`) before any script, makes no Live request to open or close, returns
  focus to its trigger on close, and keeps its open state across a morph under
  a stable key with `live:preserve.self`.
- **The Live component library gains its feedback and navigation families.**
  Alert, skeleton, spinner, progress, empty state, toast region and flash
  region; header bar, footer, sidebar, breadcrumbs, tabs, pagination and load
  more install with `live:add` beside the form and overlay families. Feedback
  presents state the server or runtime holds: an alert's role follows its
  variant with a non-color cue, loading presentation is bound through
  `live:loading` and shows on the runtime's own timing, progress is the native
  element with a label and readout, the empty state's reason is server state,
  and a toast announces once without taking focus while a critical error also
  renders as an alert. Navigation keeps route semantics: anchors with real
  URLs, `aria-current` from the server, an explicit local or route mode for
  tabs and pagination, Live pagination reflected with `history.replaceState`
  and no history entry, and load more as a keyed append from a button. The
  `live_key` view filter lands with them: a `live:key` inside a `{% for %}`
  loop passes through it, and it enforces at render time the key rule the
  checker enforces on literal keys.
- **The Live component library gains its data display family, and with it
  the built-in set is complete.** Separator, scroll area, aspect image, card,
  badge, avatar and avatar group, list group, description list and stat card
  install with `live:add` beside the earlier families; every one keeps
  document order and native semantics, and every status carries text. The
  chart renders on the server through `charts-rs`: `render_chart` in
  `suprnova::live::charts` draws bar or line marks from bounded typed series
  as trusted SVG, and the macro places a summary and a data table beside it,
  so no charting script reaches the browser. The datatable is a native table
  with one island per table whose sort, filter and page are `#[url]` fields
  reflected into a shareable URL after every action.
- **The Live component library gains its live-native family, the last one.**
  The upload widget renders the shipped upload protocol's states from the
  runtime's progress root and claims nothing durable before the finalizing
  action; the live feed and notification bell carry the runtime's stream
  status announcements, so a degraded, reconnecting or closed stream says
  so; the account menu is a details disclosure with a CSRF-protected
  sign-out form and a documented stitch slot under RenderCache. The
  custom-element tier, input OTP, date picker and combobox, enhances native
  controls it never replaces: each `sn-` element is a light-DOM
  `HTMLElement` defined by its own vendored file and holds no form value, so
  the form submits the same value with the script blocked. The combobox
  refuses a listbox rendered for an older query.
- **Session blocking serializes the requests that carry one session.**
  `SESSION_BLOCK=true`, or `SessionConfig::block(SessionBlock::default())`,
  makes the session middleware hold a cache lock for the session from load
  to write, and `block_session` on a route or group enables it for those
  routes alone. Both bounds are yours, the hold and the wait, and a request
  that waits past the bound answers `503` with `Retry-After`. Without it,
  two concurrent requests on one session wrote back last-writer-wins, so a
  flash set by a redirect could be lost to a request that started earlier.

### Changed

- **Live's supported browser baseline is Chrome and Edge 114, Firefox 128, and
  Safari 17.** The floor rises from Chrome and Edge 111 and Safari 16.4 to the
  first releases that ship the native `popover` attribute the component
  library's overlays own their open state with. The compatibility matrix's
  minimum slots move with it; Firefox stays at 128.

### Fixed

- **A template's `live:key` reaches the Live runtime.** The checker validated
  `live:key` and the manual named it, but the browser runtime's morph
  identity, morph controls and preservation scopes read only the engine's
  `data-suprnova-live-key`, so a keyed control written as the checker
  requires had no effect and every library component wrote the key twice.
  The runtime now reads `live:key`, keeps the engine spelling for the roots
  it renders, refuses an element carrying both with different values, and
  the components write `live:key` alone. The form gallery's save form also
  gained the `.prevent` modifier it lacked, so a Live submit no longer
  reloads the page from the form's own query.
- **A Live form holding an empty number input or an unselected select
  submits.** The runtime read such a control's null value as a mismatch with
  itself and refused the whole submit, so the action never ran and the
  browser submitted the form natively. The search input's model binding also
  used a debounce the runtime does not accept; it now uses 250 ms.
- **`live:check` checks every element a view renders.** A view that called a
  macro splicing `caller()` with an empty call block, as the validation summary
  is called, rendered no content after that call to the checker, so the
  component proved clean while the rest of the view went unchecked. The empty
  call is now empty content, and a view that renders nothing fails the check.
  Fixing it exposed errors the dogfood form gallery had hidden.
- **A declared model debounce is 100, 250, or 500 milliseconds.**
  `#[model(debounce = N)]` accepted 1 to 60000 ms, but the directive grammar
  the checker and the browser runtime share lists three durations, so any
  other value could never be bound by a template; it now fails to compile.
  `live:error` also accepts an action as its target, as a validation summary
  names one, matching what the runtime resolves.
- **A Live form of more than seven model fields submits.** The browser runtime
  refused any request carrying more than eight operations or model proposals,
  and any response with more than sixteen validation entries or eight events,
  though the framework's server accepts 128 of each; the refused submit never
  left the browser and was reported as a network failure. The browser now
  admits the server's counts, `live:check` refuses a `live:submit` form of
  more than 127 model fields, and a request refused for a limit is reported as
  a resource limit that shows the action's error feedback.
- **The combobox stays responsive and shows what the server answered.** Text
  that no option matched froze the page, because the element's observer
  watched attributes its own render rewrote even when nothing changed. The
  element also filtered the server's options by substring, hiding results a
  server search matched another way, such as an accent-insensitive or code
  match. It now writes only what changed, shows every option while the
  listbox answers the input's current text, keeps an answer to older text
  hidden, and filters by the typed text only for a fixed list, which
  `remote=false` selects.
- **A library custom element binds once however a morph moves it.** The
  combobox, input OTP, date picker, and password input bound their listeners
  on every connection, and the dialog, sheet, and drawer guarded on an
  attribute a morph removes, so a moved element answered one click twice and
  the password reveal toggled back at once. Every element now binds per
  connection, releases its listeners and observers when it leaves the
  document, and keeps them across an atomic move.
- **A toast holds while the pointer is anywhere on it or focus is inside it.**
  The region resumed its timers when the pointer left any child element, so
  moving from a toast's text to its padding let it time out under the
  pointer, and a toast could hide while its dismiss button had focus.
- **Nested tabs act on their own tabs.** A local tabs instance selected every
  tab below it and handled the events of a nested instance, so a click on an
  inner tab hid the outer panel.
- **The tooltip bubble stays open while the pointer moves onto it.** The
  bubble ignored the pointer, so it hid as the pointer left the trigger and
  its text could not be read or selected.
- **The select's dropdown indicator follows the text color.** It was a
  data-URI SVG, whose `currentColor` does not inherit the document's color, so
  it drew black on the dark scheme's surface.
- **The form controls show the island's values.** No form macro took a value,
  so a number input mounted at 1 rendered empty and a submit that changed
  nothing proposed an empty value. Each value control now takes `value=`,
  `checked=`, or `selected=`, radio and checkbox group inputs are keyed by
  value so a choice the user has not sent survives a re-render, and
  `authority=` marks the render that must replace what the user typed.
- **A checkbox group proposes the list of checked values.** The runtime read
  every checkbox as a boolean, so a group whose boxes disagreed could not be
  submitted. A checkbox group of any size, and any field that more than one
  checkbox binds, now proposes the checked values in document order; a single
  checkbox stays a boolean.
- **A model proposal its field cannot decode is a validation error on that
  field.** A proposal such as null for a `u64` field or a boolean for a list
  field was dropped silently: the action ran and the response carried no
  validation. The field now reports the error through `live:error` and keeps
  its value, and the action does not run.
- **The checker, the `live_key` filter, and the runtime accept one key
  alphabet.** The checker and the filter accepted a key beginning with `_`,
  `-`, `.`, or `:`, which the runtime refuses, so the island's first morph
  failed. A key now begins with an ASCII letter or digit everywhere, and
  `live:check` holds the element ids inside an island to the rule the runtime
  checks, refusing an invalid id (`invalid_element_id`) and a repeated one
  (`duplicate_element_id`), a literal id inside a loop included.
- **`live_key_digest` keys any value.** `live_key` fails the island's render
  for a value outside the key alphabet, so a row keyed by an email address
  failed the island for every viewer. The new filter turns any value into a
  stable key, one value always yielding the same key.
- **`live:check` checks a loop or match binding as that binding.** Inside a
  macro body, a name that a `for`, a `match` arm, or an `if let` bound was
  checked as the macro parameter of the same name, so a literal argument
  proved a directive that the loop's own values render.
- **`render_chart` returns an error for a value beyond 1e9.** The renderer's
  axis arithmetic overflowed and panicked from about 1e12; a value whose
  magnitude exceeds 1e9 is now an input error.
- **An application that cannot read its vendored components refuses to
  start.** `try_live_ui_assets()` reads `templates/suprnova-ui/` under the
  application base path on each request, so an application started
  elsewhere, such as from a container image that holds only the binary,
  answered 404 for every component stylesheet and script and started
  cleanly. Installing the route now fails and names the directory.
- **`live:add` replaces a file you never edited.** It compared bytes only, so
  after a library update it kept every installed file and reported it edited
  locally. It now records each file's digest beside the component and
  replaces a file whose bytes still match the record; a file you edited, or
  one no record vouches for, is kept and reported.
- **`live:add --manifest` refuses a file that is a symbolic link.** Each
  third-party file was read through links, so a component could install the
  bytes of any readable file as a template. A named file must be a regular
  file inside the manifest's directory.
- **A model edit that a render replaced is sent when it is typed again.** The
  browser compared each edit with the value it last proposed, so after a
  refused value and a render that replaced it, such as a reset, typing the same
  value again sent nothing: the control showed it with no error while the
  island held another value. The render an island applies is now the baseline
  the next edit is compared with, and a field's dirty state compares with the
  value that render gave its control.
- **A tooltip can be dismissed where it is shown.** A bubble that covers
  content could be dismissed only by moving the pointer or focus away, which
  WCAG 2.2 success criterion 1.4.13 does not accept. Escape now hides it with
  neither moved, and the next hover or focus of that trigger shows it again.
  The bubble still shows with no script in the page; the new `sn-tooltip`
  element carries the dismissal alone, and `live:add tooltip` installs it.

## 2.0.2 - 2026-09-14

### Changed

- **The Live endpoints carry no version segment.** `/__live/v1/action`,
  `/__live/v1/upload`, `/__live/v1/assets/*`, and the `/__live/v1/async/*`
  family now live at the same paths without `/v1`: `/__live/action`,
  `/__live/upload`, `/__live/assets/*`, `/__live/async/*`. Suprnova's version
  is the git tag and the browser runtime ships in lockstep with the framework,
  so a second version inside the URL space promised an evolution path that
  would never be used. Applications are unaffected: the framework registers
  these routes and the runtime builds every URL, and hand-written references
  to `/__live/` paths were never supported.

### Fixed

- **Every issuance the Live per-scope limit admits answers with a subscription
  that connects.** Concurrent issuances of one scope in the same millisecond
  mint identical descriptors, and the host's credential store kept only the
  last secret minted for a descriptor, so the earlier requests answered 403
  `async_authority_invalid` from the connect that issuance performs. About one
  issuance in five hundred also answered 503 `async_unavailable`, because the
  claims it published while its envelope context was built shared one slot
  with every concurrent issuance. A descriptor now keeps every unconsumed
  secret until each is consumed or expires, and the claims under construction
  are keyed by subscription id.

### Security

- **The lockfile takes the rustls TLS 1.3 handshake fix.** `rustls` 0.23.45
  replaces 0.23.40, which accepted TLS 1.3 handshake messages across
  encryption level boundaries (RUSTSEC-2026-0285, published on the release
  day); `aws-lc-rs` and `rustls-webpki` move with it. Every HTTPS client in
  the framework, from the HTTP facade to the Qdrant and payment adapters,
  resolves the fixed release.
- **A Live stream ends with the session that opened it.** An asynchronous
  membership was re-authorized against its Gate before every delivery but
  never against its session: a browser that logged out, or whose session was
  revoked, kept receiving events until the stream itself closed. Destroying a
  session on a node now retires every membership it opened there at once, for
  a plain logout, session invalidation, id regeneration, and "log out
  everywhere" alike, and delivery re-checks each membership's session against
  the session store at most once per ten seconds, so a session destroyed on
  another node stops receiving events within that interval.
- **A request's `Cache-Control` directives are honored by RenderCache.** A
  request carrying `no-cache` was answered from storage with `Age`, and a cold
  request carrying `no-store` seeded the cache for the next request.
  `no-store` now bypasses lookup and publication alike, and `no-cache` skips
  the lookup so the request is answered by a fresh render. Found by the
  2026-09-13 adversarial audit (ASTRA-13).
- **A render without a snapshot is never published.** When the snapshot
  transaction could not open, RenderCache rendered without a read view and
  still published if the generation reread agreed, so a render that
  interleaved with a concurrent multi-row write could store a mix of two
  database states. Such a render is now served but not stored (decline reason
  `snapshot_unavailable`), and its rebuild lease is released. Found by the
  2026-09-13 adversarial audit (ASTRA-08).
- **A cached response replays its `Content-Encoding`.** A pre-compressed body
  was stored without its content coding, so a hit served gzip bytes as plain
  text. The coding is stored with the body and replayed on every hit. Found by
  the 2026-09-13 adversarial audit (ASTRA-04).
- **A HEAD request never seeds the GET representation.** A cold HEAD on a
  route that renders nothing for HEAD stored an empty body under the key every
  GET shares, so later GETs answered zero bytes. A HEAD miss is now served as
  rendered and not stored (decline reason `head_render`). Found by the
  2026-09-13 adversarial audit (ASTRA-03).
- **The Live per-scope subscription limit holds under concurrency.** Issuance
  counted a scope's subscriptions, released the lock, awaited the authorizer,
  and inserted afterwards, so a burst of concurrent requests could all pass
  the count and all be admitted once authorization returned, well past the
  advertised limit of 512 per scope. The slot is now reserved under the same
  lock as the count, released on every error path, and handed to the record
  when it lands. Found by the 2026-09-13 adversarial audit (ASTRA-07).
- **A Live subscription stops receiving events once its Gate denies.** An
  existing asynchronous membership kept receiving newly published events after
  the stream's authorization Gate was redefined to deny the principal, because
  delivery compared the subscription's own retained authorization memo with
  itself. New subscriptions were correctly refused; the old stream was not.
  The runtime now records the principal a subscription was issued to, asks the
  Gate again before every delivery, and retires a membership the Gate no
  longer allows. Found by the 2026-09-13 adversarial audit (ASTRA-01).
- **A Live action declaring `transaction = "required"` is refused at
  registration.** The host's transaction port is a documented no-op, so the
  policy promised atomicity it never provided: each write committed on its own
  and nothing rolled back when a later stage failed. `LiveRegistry` now fails
  with `RegistryErrorKind::RequiredTransactionUnsupported` for such a
  component until the port installs a real ambient transaction; actions
  without the policy register as before. Found by the 2026-09-13 adversarial
  audit (ASTRA-05).
- **A cached route's named-connection reads stay on their connection.** A
  RenderCache miss renders inside a snapshot transaction on the primary
  database, and query routing preferred that transaction over a query's own
  `on("name")` or a model's declared connection, so opting a route into the
  cache changed which database its code read from. A tenant or auxiliary
  database read could return the primary's row, fail on a table the primary
  lacks, or publish the wrong content under a valid key. Reads bound for
  another connection now run there even inside an ambient transaction, and a
  render that read outside its snapshot is served but not stored (decline
  reason `foreign_connection_read`). Found by the 2026-09-13 adversarial audit
  (ASTRA-06).
- **A data write and its RenderCache invalidation commit together.** On the
  autocommit path, a model save, a query-builder write, or a raw statement
  landed its row first and advanced the dependency generations in a second
  transaction afterwards. When that second transaction failed, the row was
  durable, the API returned an error, and every cached page that depended on
  the row kept passing its coherence check and serving the pre-write content:
  a visibility, entitlement, or deletion change could stay invisible until the
  next successful invalidation. Every write terminal now runs the row write
  and its advancement inside one transaction opened for the purpose, so both
  commit or neither does. A ledger table that vanishes after RenderCache
  decided it was present now fails the write instead of being skipped with a
  warning. A write bound for a named connection, whose ledger lives on the
  primary, keeps its separate advancement; if that advancement fails, the
  process stops serving stored entries until one succeeds. Found by the
  2026-09-13 adversarial audit (ASTRA-10).
- **A handler's `Vary` contract is enforced before RenderCache stores.** A
  handler that varied its body on a request header of its own and said so with
  `Vary` was stored under a key built from the route policy alone, so the
  first variant's body was served to every other variant, with the `Vary`
  header missing from the hit. Wherever that header selected user, device, or
  experiment-specific content, one request's content reached another's.
  RenderCache now parses the response's `Vary` before publication and declines
  to store when it names `*` or a field the policy does not declare as a key
  dimension (decline reason `vary_undeclared`). Found by the 2026-09-13
  adversarial audit (ASTRA-09).
- **A handler's `Cache-Control: no-store` is honored by RenderCache.** A route
  opted into the cache stored a response whose handler said `no-store` and
  replayed it under the policy's own `public, max-age=60, s-maxage=60`, so a
  handler's "do not store" was ignored on the server and rewritten for every
  browser and proxy downstream. The eligibility check now reads the response's
  `Cache-Control` and declines storage on the `no-store` token (decline reason
  `no_store_directive`), and the declined response goes out exactly as the
  handler built it. Found by the 2026-09-13 adversarial audit (ASTRA-02).
- **A per-response CSP nonce is never replayed from the cache.** A public page
  that minted a nonce on every render and named it in
  `Content-Security-Policy` was stored as an ordinary complete entry, so every
  later hit carried the first render's nonce in both the header and the inline
  script. A nonce is the authorization token for inline script in one
  response; replaying it made that token readable to anyone who could fetch
  the page. RenderCache now declines to store such a response (decline reason
  `nonce_source_policy`); a hash-based policy caches as before, and stitched
  Live documents keep issuing a fresh nonce per hit. Found by the 2026-09-13
  adversarial audit (ASTRA-12).
- **A cached response keeps its isolation and execution headers.** RenderCache
  stored a response's `Content-Disposition`, `Cross-Origin-Opener-Policy`,
  `Cross-Origin-Embedder-Policy`, `Cross-Origin-Resource-Policy`,
  `Permissions-Policy`, and `X-Frame-Options` nowhere, so a cache hit served
  the same bytes without them. An HTML export that downloaded as an attachment
  on the first request rendered inline under the application's origin on the
  second, where any markup it carried ran with same-origin authority. The six
  headers now replay byte for byte from the stored representation. Found by
  the 2026-09-13 adversarial audit (ASTRA-11).
- **The lockfile sheds one unsound and three yanked dependency releases.**
  `event-listener` 5.4.2 replaces 5.4.1, whose stack-allocated listener was
  unconditionally `Send`/`Sync` and let a `!Send` tag cross threads in safe
  code (RUSTSEC-2026-0221); Suprnova's dependencies only use untagged events,
  so the unsound path was never exercised here. `spin` 0.9.9 and 0.10.1 and
  `chacha20` 0.10.2 replace releases their publishers had yanked, and
  `concurrent-queue` leaves the tree entirely.

## 2.0.1 - 2026-09-12

### Fixed

- **A `redis://` URL with a database index selects that database everywhere.**
  The queue driver and the fanout broadcast hub each carry a sea-streamer
  producer beside their direct redis connections, and sea-streamer does not
  read the logical database from the URL's path - so a queue pointed at
  `redis://host:6379/3` pushed jobs into database `0` while its consumer half
  waited on database `3`, and the fanout hub operated on `0` outright. Both
  now carry the index across explicitly, with the same parsing rule the redis
  client applies. A URL without a path keeps selecting database `0`, so
  nothing changes for the common form.
- **`suprnova generate-types` ends its output with one newline.** The blank
  line that separates one interface from the next was also written after the
  last one, so a project that enforces `git diff --check` failed with `new
  blank line at EOF` on a file it could not correct by hand: the next
  regeneration wrote the blank line straight back. Only the end of the file
  changes; the blank lines between declarations stay.

- **A scaffolded login or registration form shows its validation errors.**
  The generated auth controller declared an `errors` prop on `LoginProps`
  and `RegisterProps` and sent it as `None`. The framework seeds `errors` on
  every Inertia page from the session-flashed validation bag, and an
  explicit prop of the same name replaces that seed, so the page received
  `errors: null` and invalid credentials returned to a form that displayed
  nothing. Both props are now empty, the Login and Register pages of all
  three frontends read `useForm().errors`, and the scaffold's
  `inertia-props.ts` is the byte-exact output of `suprnova generate-types`
  for the scaffold's controllers, so a project's first regeneration no
  longer rewrites a file nobody edited.

- **A scaffolded application serves its built frontend.** The generated
  `routes.rs` registered no static-file fallback, so once Vite's dev server
  was not running every `/assets/*` URL in the HTML shell answered `404`,
  including inside the production image the scaffold's `Dockerfile` builds.
  `routes.rs` now ends with `fallback!(StaticFiles::public().handler())`:
  declared routes still win, dotfiles such as
  `public/assets/.vite/manifest.json` and path traversal are refused, and
  an unknown URL still renders the Inertia `Error` page.

- **A scaffolded application verifies email addresses and resets
  passwords.** Registration created and signed in the user without sending
  verification mail, and no route offered email verification or password
  recovery, although the generated `User` already implemented
  `MustVerifyEmail` and `CanResetPassword` and the `auth_flow_tokens`
  migration already shipped. Registration now mails a verification link
  through `EmailVerification::send_link` and continues to `/verify-email`,
  which shows the notice, resends the link, and consumes it on
  `/verify-email/verify` for the signed-in owner only. `/forgot-password`
  mails a reset link to a verified address (an unknown or unverified
  address gets the same answer and no mail) and `/reset-password` rotates
  the password through `PasswordReset::complete_with_outcome`, refusing to
  finish while the account's other sessions or remember-me tokens could
  not be revoked. The Vue, React and Svelte starters ship the
  `ForgotPassword`, `ResetPassword` and `VerifyEmail` pages and a "Forgot
  your password?" link on the login page. Links are built with `url::to`,
  so `APP_URL` must name the address users reach the application at, and
  registration needs a working mail transport: the `.env` the scaffold
  writes points `MAIL_DRIVER=smtp` at a local catcher on port 1025 (the
  Mailpit that `suprnova docker:compose --with-mailpit` adds), or set
  `MAIL_DRIVER=log` to print each message, link included, to the server log.

- **A fresh Svelte scaffold builds against `@inertiajs/svelte` 3.7.** The
  generated `main.ts` declared an `async setup`, and `@inertiajs/svelte` 3.7
  types `setup` as returning `SvelteRenderResult | void`, so a project
  scaffolded today (the template asks for `^3.6.1`, which now resolves to
  3.7.1) failed `svelte-check` inside `npm run build` before a single page
  had been written. `setup` is synchronous now and chains the translation
  catalog load onto the mount, so the ordering the template describes is
  unchanged.

### Documentation

- **The six locale mirrors of the 2.0.0 chapters follow the manual's own
  conventions.** The chapters translated since 1.3.7 wrapped Japanese and
  Simplified Chinese prose mid-sentence (1,653 line breaks that rendered as
  stray spaces), rendered 59 terms differently from the rest of their
  locale, translated 9 repeated headings a second way, and left 2 quotes
  unpaired. All of that is aligned with the older chapters, so a reader of
  any locale sees one vocabulary and unbroken sentences across the manual.

### Upgrading

- **These are scaffold fixes; an application generated with 2.0.0 keeps
  its generated files.** Upgrading the framework crate changes nothing in
  `src/` or `frontend/`. To pick the fixes up in an existing project, make
  the same edits by hand: drop the `errors` field from `LoginProps` and
  `RegisterProps` and read `useForm().errors` in the pages; add
  `fallback!(StaticFiles::public().handler())` as the last entry of
  `routes!`; and copy the `email_verification` and `password_reset`
  controllers, their routes, and the three auth pages from a project
  generated with 2.0.1. The `generate-types` output changes only at the
  end of the file.

## 2.0.0 - 2026-09-10

### Security

- **A bearer token and a web session are now separate identities.** The
  request-scoped user cache kept one current-user slot that everything wrote
  to and everything read from, so a browser session hydrated by
  `SessionMiddleware` satisfied a `TokenGuard` on the same request, and a user
  resolved for the web guard was handed back to code that had asked for the
  API guard. Bearer credentials now carry their own provenance in that cache:
  `BearerTokenMiddleware` records the identifier it validated in a bearer slot
  of its own, `TokenGuard` resolves and caches the full user there, and only
  something that arrived through a bearer credential can satisfy a token
  guard. Session guards are cached per guard name in the same place, so
  `Auth::guard("admin").user()` and `Auth::guard("web").user()` in one request
  no longer resolve to whichever of them ran first. The generic slots remain
  as a compatibility view for the static `Auth` facade, mirrored from the
  configured default guard alone, so `Auth::id()`, `Auth::check()` and
  `AuthMiddleware` behave exactly as they did for an application with one
  guard and for a token-only request that never installs a session.

- **A named guard keeps its own principal, its own remember-me credential, and
  its own revocation.** Logging in through `Auth::guard("admin")` wrote the
  identifier into the same session key the default guard uses, so two guards
  in one application shared one principal and signing out of either signed out
  of both. In the persisted session each guard now owns its entry under the
  `_auth_guards` map, and the remember-me cookie carries a guard-tagged carrier
  (`suprnova.remember.v1:` followed by the guard name and the credential) so a
  cookie issued for one guard cannot re-authenticate another. A cookie without
  that prefix is read as the default guard's, which is exactly what a cookie
  issued by an earlier release is, so nobody is signed out by the upgrade; a
  carrier naming a version this build does not understand is refused rather
  than guessed at. Revocation follows the same boundary: signing out, and the
  middleware's own rotation path, retire the exact selector the owning guard
  issued rather than every credential the user holds.

- **`BasicAuthMiddleware` no longer accepts a stale session slot as proof.**
  Its non-stateless form skipped the `Authorization` header whenever
  `Auth::check()` was true, and `Auth::check()` reads the request-scoped
  current-user slot, which anything earlier in the chain could have populated.
  It now asks for the persisted session principal of the guard it was
  configured with, and refuses outright when that guard is absent or is not a
  stateful guard, so a request is admitted without credentials only when a
  real session row says who it belongs to. The stateless form always re-read
  the header and is unchanged.

- **Remember-me credentials rotate as one atomic replacement.** Rotation used
  to remove the accepted credential and insert its successor as two writes: a
  failure between them consumed a valid credential and left the visitor with
  no way back in, and a crash left both rows live. The default schema now
  replaces one exact, still-valid row with its prepared successor in a single
  operation, and a store that cannot make the conditional removal and the
  replacement insert atomic fails closed rather than performing them
  separately. Selector matching is exact, so a credential is never retired by
  a prefix collision, and a synchronous identity transition inside a handler
  queues the exact credential it invalidated for revocation at the end of the
  request instead of leaving it live.

- **Two-factor admission is serialized, and a lockout write that fails takes
  the request down with it.** Two workers proving the same code could each
  read the attempt counter before either wrote it, so a brute-force budget
  admitted more attempts than it allowed. Verification now reserves attempt
  capacity inside the same serialized store operation that admits the
  attempt, returning both the reservation and any finalized-failure state
  observed in that one decision; a ceremony that cannot commit cancels its
  prepared proof, and the cancel path's default fails closed so an existing
  verifier implementation cannot silently leak reserved capacity. A lockout
  counter write that errors is no longer swallowed: the attempt is refused.
  Promoting a session that is waiting on a second factor is one atomic
  migration, and the bearer credential is suppressed before the storage call
  is awaited, so a timeout or a backend failure can never leave a credential
  attached to a session the framework did not commit.

- **Session rotation fails closed.** Rotating a session id destroys the old
  row and writes a new one. A destroy that errored was logged and stepped
  over, which left the previous authenticated row replayable by anyone
  holding the old cookie. The middleware now returns before writing the
  replacement and before issuing the new id, and expires the browser's old
  credential on the way out. Separately, the cookie that carries a fresh or
  rotated session is built before the row is committed: a cookie that cannot
  be constructed used to leave a session in the store that no browser could
  ever present, and now leaves nothing behind at all.

- **A session that outlives its user stops authorizing, and revoking a user's
  sessions reaches the named guards.** `AuthMiddleware` treated the presence
  of a persisted identifier as proof of an identity, so a deleted or
  soft-deleted user kept passing every guarded route until the session
  expired. It now resolves the user through the provider and clears the stale
  slot when the provider finds nothing; an application with no user provider
  bound at all keeps the identifier-only fast path, recognized by its own
  error rather than by matching message text, and every other provider
  failure is an error rather than a pass. `destroy_all_for_user` matched only
  the indexed `user_id` column, which is null for a session authenticated
  through a named guard alone, so those sessions survived a
  "sign out everywhere". It now compares the guard identities inside each
  surviving payload as well.

- **Device-authorization ceremonies transition atomically and are validated
  before consumption.** Approving a device code read the ceremony, then
  consumed it, then wrote the grant, so a replacement issued under the same
  selector between the read and the consume could be consumed instead of the
  record that was actually approved. The store contract now binds the
  consuming transaction to the exact record a prior read observed, and
  transitions one ceremony while consuming another in a single atomic step.
  Both methods default to failing closed, so an external store implementation
  stays source-compatible without silently getting the weaker behaviour.

- **A provider-token refresh whose outcome is unknown is fenced, not
  retried.** A linked-account refresh that started and then lost its answer
  left an ordinary claim that expired on schedule, so a second worker
  refreshed the same grant and one of the two results was discarded, taking a
  single-use refresh token with it. Starting an exchange now replaces the
  claim's owner with a reserved exchange owner while preserving the original
  deadline, so followers can tell a live exchange from an abandoned one, and
  a store must never reclaim a row whose owner is in that reserved namespace.
  Stores that do not implement the fence fail closed.

- **Web Push refuses to send through a transport that might follow a
  redirect.** `EndpointPolicy::Strict` validates the subscription endpoint
  URL, but validation only ever covered the initial URL: a client that
  follows redirects turns a validated endpoint into a `3xx` to anywhere, and
  reqwest follows redirects by default. A client this crate builds now has
  redirects forcibly disabled, and the new
  `WebPushClient::with_client_builder` applies every option a caller wants
  (proxy, TLS pinning, timeouts) while overriding the redirect policy.
  `WebPushClient::with_client`, which takes an already-built client whose
  redirect policy cannot be inspected, now refuses to send under `Strict`
  with `WebPushError::UnconfinedRedirects`, before encryption and before any
  request. `WebPushClient::allow_unconfined_redirects` is the explicit opt-out
  for a caller who knows their client is safe.

- **A signed URL is bound to the exact path it was signed for.** Signing and
  verification both trimmed a trailing slash before hashing, which made
  `/orders/1` and `/orders/1/` one signature, and made a proxy that appends a
  slash indistinguishable from a client that edits the path. The path is now
  hashed exactly as it appears, so a signature covers one path and one path
  only.

- **A cache key can no longer address a lock or a tag index.** Redis lock,
  tag, and key-tag records lived under a NUL sentinel after the configured
  prefix, and a caller-supplied key beginning with that sentinel landed in
  the same space, so a `Cache::forget` could release a distributed lock
  somebody else was holding. Values and each internal record type now carry
  distinct namespace components ahead of the caller's key, so the two spaces
  cannot meet. An ordinary key is stored exactly where it was before, so
  nothing already cached is orphaned. The in-memory driver got the same
  separation.

- **Payment webhooks reject what they cannot identify and classify duplicates
  by the database, not by message text.** A provider event with a missing,
  non-string, or whitespace-only identifier used to enter the shared
  idempotency namespace under a blank key, where it collided with every other
  such event; it is now rejected before any state is written. A concurrent
  duplicate is recognized from SeaORM's structured unique-violation code
  rather than from human-readable error text an unrelated failure can also
  contain, and only a re-read of a committed `processed_at` is acknowledged
  as one, so a mirror-write failure stays retryable. Stripe's signature
  timestamp is compared with an unsigned absolute difference, so an extreme
  `t=` value returns a signature error instead of overflowing, and a negative
  configured tolerance accepts only an exact match. The webhook route
  preserves the typed status of a body it refused, so an over-cap body is
  still a `413` rather than a flattened `400`.

- **A non-idempotent HTTP request is no longer replayed after a transport
  error.** The retry policy already required the explicit
  `retry_non_idempotent` opt-in before replaying a `POST` or `PATCH` that
  answered `5xx`, but the transport-error branch beside it did not check,
  so a request whose connection dropped after the server had accepted it was
  sent again. Both branches now apply the same rule.

- **Session lifetimes cannot overflow into mass expiry.** `SESSION_LIFETIME`
  and `SESSION_REMEMBER_LIFETIME` are minutes multiplied by sixty and then
  added to a stored timestamp in date arithmetic that panics on overflow, so
  an oversized value either aborted the process or wrapped into a deadline in
  the past that expired every session at once. Both are clamped to
  `MAX_SESSION_LIFETIME_MINUTES` before the multiplication, the database
  driver caps the same way for a configuration built in code, and a garbage
  collection cutoff that cannot be represented is skipped rather than sent to
  the database, which is the difference between collecting nothing and
  collecting everything.

- **Machine-to-machine cache identities are unambiguous.** The token broker's
  cache key concatenated the provider, the client, and the normalized scope
  set, so two different requests whose components happened to run together
  into the same string shared one cached token. The key is now a versioned
  domain with length-prefixed components, which no combination of inputs can
  make collide.

- **An idempotency lease is proven still held before its result is reported as
  fenced.** The lease refreshed periodically while the body ran, and a
  transient refresh error was treated as loss, while a body that finished
  between two refreshes was reported as fenced without anyone asking whether
  the lock was still there. A transient refresh error is now retried and only
  gives up after several consecutive failures, and one final owner-scoped
  refresh must succeed after the body completes before the outcome is
  reported as fenced; an error in that last check answers `FreshUnfenced`,
  because ownership is then unknown.

- **The encryption key ring is validated and installed before application
  bootstrap.** `Crypt` was initialized by `Server::from_config`, so anything
  that ran earlier - the bootstrap callback, a console command, a queue worker
  entry point that never builds a server - either found no key ring or built
  its own. `#[suprnova::main]` now loads the environment and then validates
  and installs the ring, in that order, before your bootstrap runs. Validation
  runs on every boot even after the ring is installed, so a production process
  with a missing or malformed `APP_KEY` still fails closed, while the
  process-wide key stays immutable. Laravel's `APP_PREVIOUS_KEYS` is accepted
  as an alias for `APP_KEY_PREVIOUS`; when both are set and disagree, the
  Suprnova name wins and the duplicate is named in a warning.

- **The archived `proc-macro-error2` crate is replaced by its maintained
  successor.** It was archived on 2026-06-07, is flagged unmaintained in
  RUSTSEC-2026-0173, and made every build warn that a future Rust release
  will reject it (E0365). `validator_derive` 0.20.1 moves to
  `proc-macro-error3` 3.1.1 and `sea-bae` 0.2.2 drops the dependency, so
  neither the advisory nor the warning appears any more. This landed on
  main after the `v2.0.0` tag; the tagged `Cargo.lock` still resolves
  `proc-macro-error2`.

### Added

- **Suprnova Live is part of the framework.** `suprnova::live` is a
  server-driven interaction engine: a component is a Rust struct whose state
  lives on the server, whose view is a checked Askama template, and whose
  actions run over a signed protocol from a small browser runtime that morphs
  the re-rendered HTML in place. There is no client-side state model to keep
  in sync, no build tool to install to use the shipped runtime, and no inline
  JavaScript in your documents. The engine ships as an internal crate the
  framework depends on unconditionally, so nothing has to be enabled; the
  browser half is published as `@suprnova/live` and its exact reviewed bytes
  are served by the framework itself. `manual/live.md` is the
  application-facing chapter, and a project created by `suprnova new` is Live
  ready out of the box: it writes `src/live/mod.rs` with an empty registry and
  a `routes()` function, binds the registry in `bootstrap.rs`, and installs
  the routes from `cmd/main.rs`.

- **Components are declared with `#[derive(LiveComponent)]` and `#[live]`.**
  The derive names the component and its view
  (`#[live(name = "app.counter", view = "live/counter.html")]`); the `#[live]`
  attribute on the `impl` block marks the methods the browser may invoke. A
  `#[public]` field is rendered and carried in the signed snapshot, a
  `#[model]` field additionally accepts browser proposals through
  `live:model`, and an `#[action]` method is the only entry point a request
  can reach, receiving validated arguments and returning typed outcomes such
  as a redirect or a flash. Every field type must implement `Default`; a fresh
  island starts from those defaults unless a mount hook says otherwise.
  Components are registered explicitly through `LiveRegistry::builder`, and
  the registry is immutable once the runtime assembles - a duplicate name or
  view, or a component whose actions need validation with no validation port
  bound, fails registration with a typed `RegistryError`.

- **`suprnova::view` is a checked server-rendered view contract for ordinary
  routes as well as Live components.** `#[suprnova::view(path = "...")]`
  declares a template, `TrustedHtml` is the one audited type a template may
  emit unescaped, and the `trusted_html` filter is how it gets there;
  `#[suprnova::view_filter]` declares a checked custom filter. Askama is the
  substrate, but handlers depend on the framework's own contracts rather than
  on the template engine's modules, and `TemplateFailure` is a closed,
  redacted failure set (`MissingData`, `InvalidData`, `Failed`) rather than
  the engine's own error text.

- **The `live:` directive grammar is closed and proved against your
  components.** A view binds behaviour with `live:click`, `live:submit`,
  `live:model`, `live:upload`, `live:key`, `live:loading` and the rest of the
  documented set - never an inline expression language, and never a
  server-returned script. `suprnova live:check` builds your application and
  runs the integrated checker over every registered view: an unknown action,
  an unknown model field, a raw `safe` filter, or an accessibility violation
  fails with the file, line, and column. `--allow-unproved` accepts the
  dynamic structures the checker deliberately makes no claim about.

- **`Router::try_live()` installs the reserved Live namespace once.** It
  registers `/__live/v1/action`, `/__live/v1/upload`, the
  `/__live/v1/async/*` control routes and WebSocket handshake, and the
  immutable `/__live/v1/assets/*` routes, and startup fails if an application
  route could claim `/__live`. `Router::try_live_with` takes a
  `LiveRouteGuard` whose middleware chain is applied to the action, upload,
  and asynchronous control routes and to the WebSocket upgrade, which is how
  an application attaches its own authentication, tenancy, and rate limiting;
  asset routes stay unguarded. Every reserved request carries a strict policy:
  session, origin, CSRF, principal, tenant, and rate-limit facts must all have
  been recorded by real middleware, and an asynchronous route that cannot see
  the complete set is refused rather than opening an anonymous transport.

- **A Live request proves its own origin, and using Live relaxes nothing
  else.** The shipped runtime sends the Live media type and the browser's own
  `Sec-Fetch-Site` header and carries no session token, so `CsrfMiddleware`
  verifies that proof for a Live operation on its own, whatever origin policy
  the application configured, and falls back to token validation for a
  cross-site or header-less request. Ordinary routes keep the configured
  policy, so an application no longer has to widen `OriginPolicy` for the
  whole application to let Live work. `AuthMiddleware::optional()` is the new
  guard form this needs: it records a principal when one exists and lets an
  anonymous request continue, so anonymous visitors can act on a public seed
  while an identity-bound island still refuses a request without principal
  evidence.

- **Documents place islands through `LiveDocument`.** A document route builds
  one from the request, mounts each island with `LiveMount`, and emits the
  bootstrap markup exactly once. `LiveMount::public_seed` declares an island
  any visitor may render, whose state is a reusable seed promoted to a real
  instance on the visitor's first action; `LiveMount::identity_bound` declares
  an island that belongs to the current session and principal, so its document
  route must authenticate. `LiveDocument::bootstrap` emits the inert
  configuration element and the ordered script tags with integrity attributes
  for the ESM or the classic strategy, adds the upload and asynchronous roles
  when a mounted component needs them and the Stimulus bridge on request, and
  rejects a second bootstrap or a mount after bootstrap.
  `Router::try_live_mount` registers a mount, and `Router::try_live_document`
  declares a document route with no startup mounts.

- **The framework serves the exact reviewed browser artifacts.** The ten
  deterministic build outputs are embedded and validated against their
  manifest on first use, failing closed on any drift in digest, length, file
  name, role, capability, or version, and are served from
  `/__live/v1/assets/<identity>/<file>` for `GET` and `HEAD` with immutable
  caching, strong digest validators, conditional requests, `nosniff`, and
  closed misses. Documents contain no inline executable code, so a strict
  `script-src 'self'` policy holds. `suprnova live:assets --out <dir>`
  publishes the same bytes to a CDN or a static directory atomically, treats
  an identical publication as up to date, and refuses to replace a directory
  whose bytes differ unless you pass `--replace`.

- **Live components accept file uploads under a declared, checked policy.** An
  `#[upload(policy = ...)]` attribute on a `#[model]` field declares maximum
  file count, declared and aggregate byte budgets, accepted media types, and
  replacement behaviour through `UploadPolicy::builder`, and the view binds it
  with `<input type="file" live:upload="avatar">`. The runtime creates,
  transfers, and completes the upload through `/__live/v1/upload`; the bytes
  wait in quarantine until the declared finalize action runs, when the
  framework hands them to the application's `UploadFinalizer`, alongside an
  optional `UploadScanner` and `UploadApplicationValidator`. Every control is
  authorized through the gate as
  `live:<component>.upload.<field>.<Control>` for each of `Create`,
  `Reacquire`, `Status`, `Queue`, `BeginTransfer`, `PutChunk`, `Complete`,
  `Accept`, `BeginFinalize`, `CommitFinalize`, `Cancel`, `Reject`, `Expire`,
  and `Fail`. Every request revalidates the current mount, principal, session,
  tenant, component, field, and document scope, a per-handle lock serializes
  chunk, completion, cancellation, action, finalization, and cleanup races,
  and chunk bodies reserve the shared in-flight budget before buffering.
  `Router::try_live_upload_reacquisition` declares an
  application-owned path outside the reserved namespace where a browser that
  lost its transfer grant can get a fresh one, answering only the session and
  principal that created the upload.

- **Islands update asynchronously over SSE, WebSocket, or polling.** A
  component declares the streams it listens to in the `#[live]` attribute
  (`streams(stream(name = "activity", topics("activity"),
  events(ActivityPosted)))`), the framework signs a bounded subscription
  descriptor for the visitor, and the browser runtime opens a native transport
  and falls back to polling when it cannot. Subscribing is authorized through
  the gate ability `live:<component>.stream.<name>`; the application publishes
  through `suprnova::live::LiveStreams`, with `refresh` telling subscribed
  islands to fresh-render and `event::<T>` delivering a typed payload to the
  island's registered handlers. Fanout, hop count, and per-document delivery
  are all bounded. Polling is an ordinary fresh render, so state catches up
  but event payloads published while a transport was unavailable are not
  replayed, which the runtime reports as a degraded stream rather than a
  current one.

- **The browser runtime is a strict TypeScript package that ships in core and
  optional bundles.** `@suprnova/live` bootstraps once per document, discovers
  islands, parses the closed directive grammar, gives each island bounded work
  and truthful pending and failed state, applies a response only after a
  successful morph through a pinned private Idiomorph adapter, and preserves
  focus, form state, controllers, scroll, and history across morphs and native
  navigations. Optional `uploads`, `async`, and `stimulus` bundles attach
  through a typed feature port, in ESM and classic forms; Stimulus is never
  bundled into core. There is no `eval`, no `new Function`, no
  server-returned script, and no inline expression language anywhere in it.

- **Four CLI commands cover the Live workflow.** `suprnova live:make <name>`
  scaffolds a component in `src/live/`, its view in `templates/live/`, and its
  registration in the `registry()` builder, declares the module, validates
  every target and refuses traversal and symlinks before writing, writes
  atomically, never overwrites, rolls back every file a failed run had
  written, and can report a dry run. `suprnova live:check`,
  `suprnova live:inspect`, and `suprnova live:assets` are thin clients of a
  hidden framework console command and a bounded, versioned JSON-lines
  protocol, so the CLI keeps no framework or engine dependency and fails
  closed with no writes on anything unsupported, stale, truncated, oversized,
  or unexpected. `live:inspect` reports the bound registry, configuration
  limits, installed upload capabilities, assembled runtime services, and the
  asset identity as presence booleans and counts, never state or secrets.

- **`suprnova::live::testing` prepares a router's runtime and mount catalog
  for in-process tests.** `prepare_live_router_for_test` gives a test the same
  runtime the server assembles, so a test can decode an island's snapshot from
  its `data-suprnova-live-snapshot` attribute, post an action with a real
  session cookie and `Sec-Fetch-Site: same-origin`, and assert on the accepted
  render through the application's real global middleware stack.

- **RenderCache stores a proven-safe copy of a route's response and serves the
  next matching request without running the handler.** It is opt-in per route
  and per group, it never changes what an application can do, and a route it
  declines still renders and serves correctly. `Router::try_render_cache`
  opts one already-registered route pattern in and
  `Router::try_render_cache_group` opts every route under a path prefix in;
  `RenderCache::install(router, RenderCacheConfig::from_env())` finishes the
  wiring after every middleware registration that establishes request-scoped
  locale, session, or identity. `RENDER_CACHE_ENABLED=false` is a real off
  switch at install time: a disabled configuration returns the router
  untouched, probes nothing, registers nothing, and leaves the process gate
  shut.

- **A cache policy states a representation class, a freshness policy, and how
  the response may be shared.** `RenderCachePolicy::builder` takes a
  `RepresentationClass` running widest to narrowest - `PublicShared`,
  `PublicShellStitched`, `PrivateCached`, `Uncacheable` - and
  `FreshnessPolicy::new(fresh_ms, stale_servable_ms, stale_on_error_ms)` sets
  how long a representation is fresh and then how far past that edge a stored
  copy may be served while a background rebuild runs or after a foreground
  rebuild failed. `SharedCachePolicy` controls what a shared cache in front of
  the application is told. A route inside a cached group can narrow its
  enclosing policy with a `PolicyPatch` instead of restating it, and may only
  make it narrower; pulling one route out of a cached group is a patch that
  sets the class to `Uncacheable`.

- **Variance is declared, never guessed.** A cached representation varies by
  route pattern, path parameters, and application build unless a policy says
  otherwise. `QueryPolicy::declared([...])` names the query parameters that
  distinguish representations, and any other query parameter on a request
  bypasses the cache for that request rather than being silently ignored.
  `.vary(VarianceDimension::Locale | ::Host | ::Tenant | ::Principal)`
  partitions by the negotiated locale, the request host, the current tenant,
  or the signed-in visitor, the last two as opaque key material; a
  `PrivateCached` route that declares neither `Principal` nor `Tenant` fails
  to build at all. `Media` and `Encoding` are declared together with their own
  closed set through `.vary_media(NegotiatedPolicy::declared([...],
  default)?)` and `.vary_encoding(...)`: the middleware negotiates the
  request's `Accept` or `Accept-Encoding` against that set per RFC 9110, with
  the highest quality winning, equal quality keeping the header's own
  left-to-right order, a wildcard compared as a literal token rather than
  expanded, and a `q=0`, out-of-range, or unparsable quality excluding a
  candidate rather than defaulting it. An absent, unmatched, or unparsable
  header resolves to the declared default and never panics. Both the render
  key and the `Vary` header take their value from that one resolution, so they
  cannot disagree.

- **A served hit is a real HTTP response with real validators.** It carries
  `ETag` as a strong validator a client can send back as `If-None-Match` for a
  `304`, plus `Cache-Control`, `Vary`, and `Age` in whole seconds since
  publication, which is the quickest local sign that a response came out of
  the store rather than out of a handler. A response served past its fresh
  interval additionally carries `Warning: 110 - "Response is Stale"`.
  Conditional requests and `HEAD` are answered from the stored entry.

- **Cached output is proved current against the database, not assumed.** A
  request-scoped collector attributes every read a handler makes to the thing
  it read: a model, a table, a configuration value, a feature flag, an
  authorization decision, an identity axis. Every supported write path on the
  other side advances the generation of what it changed, in the caller's own
  transaction where there is one - the ORM's model and bulk writes, the query
  builder facade, raw table writes, the payments hydration path, feature flag
  writes, and the RBAC role and permission statements including the new
  revocations. A hit reproves the generations it depends on before serving,
  either by rereading the ledger under `CoherenceMode::Authority` or against a
  validation lease under `CoherenceMode::Lease`, so nothing a write has
  invalidated can be served as current. `RenderCache::bump_permission_version`
  is the one invalidation an application calls by hand, from the code path
  that changes what a signed-in user may do; it advances a persisted
  generation every principal-keyed render observes, survives a restart, and
  joins the transaction the role change runs in.

- **Authorization, feature flags, and global scopes participate honestly.** A
  gate decision is judged by the identity axis its evaluation actually
  consulted, so a tenant-only consult needs only `Tenant` declared while
  anything that resolved principal material, or resolved nothing nameable at
  all, needs `Principal`. An RBAC-gated route caches and a permission grant or
  revocation rebuilds it, because the five role and permission tables are
  observed rather than treated as an unknown. A feature flag read observes a
  `Feature` generation whenever the snapshot holds that flag at any scope key,
  `set_flag` advances it, and a flag reload advances it for every flag its own
  diff found changed. An Eloquent `GlobalScope` declares
  `ScopeDependency::Constant` or keeps the conservative `PerRequest` default,
  and a per-request scope whose filter read nothing the collector can name is
  recorded as an undeclared read and narrows the render to `Uncacheable`
  rather than silently caching a tenant filter away.
  `suprnova::live::current_tenant()` is the instrumented accessor a gate body
  or a scope reaches for.

- **The write side is open in every process that writes through the ORM.** A
  queue worker, a scheduled task, or a console command writes through the same
  ORM the server does and never calls `RenderCache::install`, so its writes
  used to advance no generation and pages depending on them kept being served
  stale. The write side is now a process-wide tri-state probed at most once,
  never inside a caller's transaction, so every writing process advances the
  same generations the server does while an application with the cache
  disabled still issues no RenderCache SQL at all.

- **Three deployment profiles decide where entries and rebuild leadership
  live.** `RENDER_CACHE_PROFILE` selects `embedded` (a per-process file tier
  under `RENDER_CACHE_L1_DIR`, in-process leadership), `database` (entries in
  `suprnova_render_entries`, leases in `suprnova_render_leases`, Live instance
  records in `suprnova_live_instances` and `suprnova_live_promotions`), or
  `redis` (a Redis hash per key, plus a per-key publication token counter).
  `RENDER_CACHE_L1` and `RENDER_CACHE_COORDINATOR` override either half
  independently, so a deployment that wants its entries in the database and
  its leases in process says exactly that. Generation truth does not move: the
  database-backed ledger is the authority at every profile, which is what lets
  Redis lose everything it holds without anything stale being proved current.
  The full table is `RENDER_CACHE_ENABLED`, `RENDER_CACHE_PROFILE`,
  `RENDER_CACHE_L1`, `RENDER_CACHE_COORDINATOR`, `RENDER_CACHE_L0_ENTRIES`,
  `RENDER_CACHE_L0_BYTES`, `RENDER_CACHE_L1_DIR`, `RENDER_CACHE_L1_BYTES`,
  `RENDER_CACHE_REDIS_URL`, `RENDER_CACHE_REDIS_PREFIX`,
  `RENDER_CACHE_LEASE_MS`, `RENDER_CACHE_MAX_WAITERS`, `RENDER_CACHE_HINTS`,
  `RENDER_CACHE_FAILURE`, and `APP_BUILD_ID`. A closed-set variable given a
  value outside its set fails the boot with a message naming the variable and
  never repeating the value, because an environment value can carry a secret.
  The Live instance ledger has its own `LIVE_LEDGER_DRIVER`, `LIVE_REDIS_URL`,
  and `LIVE_REDIS_PREFIX`, and both installs fail closed at boot on a missing
  tier migration or an endpoint nothing answers.

- **A Live document can be cached as a shared shell with per-visitor
  islands.** A route declaring `RepresentationClass::PublicShellStitched`
  stores the shell once as a composite entry cut from the slots
  `LiveDocument::mount` captured, and on a hit the middleware attaches the
  prepared entry and still calls the route chain, so the route's own guard and
  tenant middleware decide the request before the Live completion middleware
  re-mounts every slot under authority derived for that request alone. A
  capture that is not exactly usable - a slot not found exactly once, a
  document digest that does not match - declines publication and stores
  nothing. The bounds are 32 slots, 64 nonce holes, 193 graph segments, and
  4,096 bytes each for slot parameters and fallbacks. Because every assembly
  is a distinct representation, no composite response answers `304` or honours
  `If-None-Match`, and a slotted assembly is sent
  `Cache-Control: private, no-store` while a zero-slot one keeps its class's
  private `max-age`.

- **A stored composite can name another stored entry as one of its
  segments.** `LiveNestedSegment` is the typed declaration and
  `Router::try_live_nested_segment` registers it. The inner entry keeps its
  own key and its own version, so it is invalidated, republished, and fenced
  on its own terms rather than the includer's, and each segment declares what
  happens when it cannot be resolved: fail the document, omit it, or serve a
  bounded fallback. Nesting is bounded to three levels and sixteen nested
  segments, a cycle is reported as a cycle even when it would also overrun the
  depth, and the assembled length is checked against the body bound before a
  single byte is copied. Publication is refused for a composite naming an
  inner segment of a wider representation class, one with a longer freshness
  window, a transitive cycle, a graph past the depth bound, or a
  `PrivateCached` inner segment that could never resolve. On a hit an
  identity-bound inner segment is reauthorized for the requesting visitor; one
  declared identity-free skips that, which is proven identity freedom rather
  than a weakening of it.

- **Nodes can tell each other that a generation just moved.**
  `RENDER_CACHE_HINTS` turns on a Redis pub/sub channel carrying the
  dependency digests an advance just touched, defaulting to on for the `redis`
  profile and off for the other two, riding the same
  `RENDER_CACHE_REDIS_URL` and `RENDER_CACHE_REDIS_PREFIX` as the cache
  itself. A hint's only power is to make a node revalidate earlier than its
  own lease would have: it can never extend or create a lease, prove an entry
  current, bypass the ledger read a hit still makes, or touch the authority
  epoch, which is why the channel is unauthenticated by design and why an
  unreachable hint endpoint does not refuse the boot the way an unreachable
  cache tier does. A hint carries no instant, so no clock-skew assumption
  between nodes is needed. A deployment with hints off, one whose channel is
  dead, and one that never had them serve the same entries and admit the same
  rebuilds; only the moment of revalidation differs.

- **Two console commands and nine telemetry counters are the operating
  surface.** `render-cache:inspect <key>` reports one stored entry's
  representation class, `body_bytes`, other metadata, and the current
  authority epoch, and it reads this process's in-process tier and nothing
  else, so on a shared profile it answers "what this node has in memory"
  rather than "what the deployment has stored". `render-cache:epoch-advance`
  is the emergency invalidation: it advances the authority epoch, which is
  baked into every lookup key, so stored entries go out of reach with nothing
  to enumerate and nothing to delete, and on the node that runs it the effect
  is immediate. Neither ever prints a stored body or a raw dependency
  identity, which is asserted rather than merely stated. The counters are
  `suprnova.render_cache.lookups`, `.hits`, `.publications`, `.rebuilds`,
  `.stitch.assemblies`, `.stitch.slots`, `.stitch.nested`, `.hints`, and
  `.epoch_rewinds`, all with closed low-cardinality attributes that never name
  a route, a key, a digest, a tier, or a provider.

- **A declined lookup says exactly which contract refused it.**
  `outcome="declined"` on the lookup counter now carries a `reason` attribute
  from a closed set of thirty-eight labels, computed from a typed value at the
  branch that actually declined rather than reconstructed from the response
  afterwards. They are grouped by contract: eligibility (`policy_uncacheable`,
  `method`, `status`, `streaming`, `sets_cookie`, `unsafe_header_name`),
  observation (`observation_overflowed`, `ledger_read_failed`,
  `handler_not_begun`), classification, key mismatch, Live document, and
  composite build. Every label is documented in the operations chapter, and a
  test asserts that rather than trusting the prose. `reason` is emitted only
  beside `outcome="declined"`; a hit and a miss carry none.

- **An authority epoch that goes backwards is detected, refused, and lifted
  past.** A database restore can move the epoch to a value the deployment has
  already used, which would let entries published under the old higher value
  be proved current again. An entry or a lease stamped above the authority is
  now refused at any age, before any dependency comparison, and the node that
  detects it lifts the ledger epoch past the stamp, drops its lease, clears
  its in-process tier, and increments
  `suprnova.render_cache.epoch_rewinds`.

- **NOWPayments joins Stripe and Paddle as a payment adapter.** The
  `suprnova-payments-nowpayments` crate creates hosted invoices, verifies
  payment notifications, and reads payment status with the merchant API key,
  and registers as `nowpayments` in the ordinary provider registry through
  `NowPaymentsProvider::from_env()`. It reads `NOWPAYMENTS_ENVIRONMENT`
  (`sandbox` or `production`, defaulting to `sandbox`; an unknown or blank
  value fails configuration), `NOWPAYMENTS_API_KEY`, `NOWPAYMENTS_IPN_SECRET`,
  and `NOWPAYMENTS_IPN_CALLBACK_URL`, and refuses blank credentials before any
  HTTP request. Its webhook endpoint is
  `POST /webhooks/payments/nowpayments` through the shared `webhook_routes`,
  and it needs the exact public HTTPS callback URL and request bodies that
  reach the adapter unchanged. `manual/payments-nowpayments.md` is the
  chapter.

- **RBAC has the revoking counterpart of every granting helper.** The surface
  exported the granting and checking halves of an access-control API and
  nothing that took access away, so removing an administrator's role meant
  composing statements against join tables whose semantics you had to infer -
  during an incident, which is when you reach for revocation.
  `remove_permission_from_role`, `remove_role_from_model` and
  `remove_permission_from_model` are the free functions, each with the
  `_on_guard` pairing the granting side already had, and `HasRoles::remove_role`
  and `HasRoles::remove_permission_to` pair at the call site. A name that
  exists on no such guard is an error, so a typo or a call aimed at the wrong
  guard is loud; an assignment the model or role does not hold is a no-op
  returning `Ok`, so a retry of a revocation that already landed is safe. Each
  call removes exactly the one assignment it names, with no bulk sweep, and
  every statement resolves its executor through the ambient transaction first,
  so a grant-and-revoke bundle inside `DB::transaction` commits or rolls back
  as one unit. Revocation is source-specific and does not contradict
  `has_permission_for_model`, which resolves a direct grant before a
  role-inherited one: taking a role away leaves a permission the model also
  holds directly still answering true, and the rustdoc names the second call
  that ends effective access.

- **An application can build its router asynchronously.**
  `Application::try_routes_async` takes a closure returning a future, and
  `Server::try_from_config_with_routes_async` is the asynchronous twin of
  `try_from_config_with_routes` that hosts it, sharing the same prologue and
  epilogue. It exists because `RenderCache::install` has to probe for the
  generation ledger's tables before it can assemble a runtime, and the route
  closure is the only place with both a container and a router - neither boot
  hook has one. `routes`, `try_routes`, and `try_routes_async` write the same
  slot, so the last one called is the one the server builds.

- **The Magnetar integration exposes what a partially completed sign-in
  actually returned.** `SignInOutcome` is public, so a magic-link, OAuth, or
  passkey callback that resolves to `SignInOutcome::FactorRequired` can be
  handled rather than being reported as a failure: the framework session is
  not bound, and the selector it carries can be completed through the retained
  host engine. `FactorAuth` and `MagnetarFactorAuthEngine` are the types that
  hold that continuation, and `install_magnetar_oauth_engine` and
  `install_magnetar_oauth_engine_with_factor` install an OAuth engine with or
  without one. Magnetar bootstrap failures now name what is missing instead of
  reporting a generic install error.

- **The queue driver contract reports whether it can honour a queue-name
  filter.** `QueueFilterCapability` (`Supported`, `Unsupported`, `Unknown`)
  is what `QueueDriver::queue_filter_capability` returns; the default is `Unknown`,
  not `Unsupported`, so a third-party driver that already overrides `pop_from`
  keeps working unchanged, and a decorator may reject a known `Unsupported`
  connection before polling but must let an `Unknown` driver answer for
  itself. `TerminalCallbackClaim` is the metadata a worker gets back when it
  atomically claims a finished batch's terminal callbacks, carrying the
  durable completion time and any cancellation visible in the same critical
  section, so the callback decision cannot be made from a snapshot that went
  stale before ownership was elected.

- **`Schedule::try_add` is the fallible sibling of `Schedule::add`.** Task
  name identifies a task in direct lookup and in the distributed keys used by
  `TaskBuilder::on_one_server` and `TaskBuilder::without_overlapping`, so one
  name cannot identify two registered entries and a schedule that registered
  the same name twice had two tasks contending for one lock. Names are now
  exact, case-sensitive, and unique: `add` panics on a duplicate and the
  existing task is retained, while `try_add` returns the error for code that
  would rather handle it.

- **Session lifetime bounds and the session migration error are public.**
  `MAX_SESSION_LIFETIME_SECS` and `MAX_SESSION_LIFETIME_MINUTES` are the
  clamps the environment parsing and the database driver apply, and
  `SessionMigrationError` is exported so a custom `SessionStore` can name the
  failure it returns.

- **Scaffolded projects are built in a production shape by construction.** The
  generated `Cargo.toml` depends on the framework with default features off
  and the nine non-`testing` defaults listed explicitly (`filesystem`,
  `database-sqlite`, `database-postgres`, `database-mysql`, `vector-mariadb`,
  `web-push`, `localization`, `magnetar-oauth`, `media`), and re-adds
  `features = ["testing"]` as a dev-dependency. Cargo's resolver pulls a
  dev-dependency's features into `cargo test` and other `--tests` builds only,
  so `cargo build --bin app` never compiles a test seam into a shipped binary,
  and `cargo test` is unchanged. `manual/deployment.md` documents the shape
  for an existing application to adopt.

- **The default cache build id comes from the application, not the
  framework.** `#[suprnova::main]` records the application crate's own
  `CARGO_PKG_VERSION` immediately after loading the environment, and
  `RenderCacheConfig::from_env` resolves `build_id` through an explicit
  `APP_BUILD_ID`, then that recorded application version, then this framework
  crate's own version only for a binary that never expanded
  `#[suprnova::main]`. `RenderCacheConfig::with_build_id` overrides whatever
  `from_env` chose, for an application that derives its own per-deploy
  identifier in code. Set `APP_BUILD_ID` explicitly once per deploy: it is
  mixed into every lookup key, and a package version rarely changes when you
  ship a template, a translation, or a handler fix.


### Changed

- **A worker on the failover queue connection now drains every connection,
  not just the primary.** `FailoverQueueDriver` documented the Laravel
  consequence it inherited: writes fell through the list, reads did not, so
  whatever failed over to a fallback sat there until somebody ran a second
  worker against that fallback directly. `pop` and `pop_from` now rotate their
  starting connection and then scan the whole list sequentially - rotation so
  a recovered, continuously busy primary cannot starve work that landed on a
  fallback, sequential so one call cannot reserve several jobs and hand back
  one. Each reservation is issued a fresh aggregate token that the driver maps
  back to the connection that really owns it, because inner tokens are not
  globally unique and two backends can legitimately mint the same UUID; an
  expired or unknown aggregate token is treated as stale rather than sent to
  an arbitrary connection. Counters and all three listings aggregate every
  configured connection in configured order and `clear` attempts every one, so
  what an operator inspects is the backlog this driver can actually consume.
  A driver declares whether it can honour a queue-name filter through
  `QueueDriver::queue_filter_capability`, which defaults to `Unknown` so an
  existing third-party driver is unaffected.

- **The minimum workflow lease is two seconds, and the first heartbeat fires
  immediately.** The heartbeat refreshes at `max(lock_timeout / 2, 1s)`, so a
  one-second lease was due for its first refresh at or after its own expiry:
  any claim latency or scheduling jitter opened a window another worker could
  walk through while the first was already running effects. The heartbeat's
  first tick is no longer skipped, which closes the claim-to-first-refresh
  window, and admission awaits an owned refresh before any user code runs, not
  only inside a step. `WORKFLOW_LOCK_TIMEOUT_SECS` below two is clamped with a
  warning that says why, and a configuration built in code that carries a
  shorter lease fails validation.

- **A payment provider without customer records can decline transaction-mirror
  hydration.** `WebhookHandler::mirrors_payment_transactions` defaults to
  `true`, so every existing provider behaves as it did; a provider that
  returns `false` still has its verified events persisted and deduplicated in
  the webhook audit log, refunds included, but no transaction mirror is
  fabricated for orders the application owns and must reconcile against
  authenticated provider state. `try_extract_payment_snapshot` is the fallible
  form of `extract_payment_snapshot` the hydration path uses: returning `Err`
  leaves the webhook pending so the provider retries it, while `Ok(None)` is
  reserved for an event that genuinely cannot supply a complete snapshot. Both
  are provided methods, so an existing driver compiles and behaves unchanged.

- **A Paddle `transaction.billed` is no longer read as money collected.** An
  issued invoice does not confirm collection, and treating it as a settlement
  marked orders paid that had not been. Approved refund and dispute
  adjustments are classified as adjustments rather than transactions, with
  their currency taken from `data.currency_code` and their settle time from
  the latest captured payment attempt where one is available; only
  `WebhookHandler::parse_event` classifies them, because the decision needs
  the payload's own action and approval status. The adapter also preserves the
  merchant correlation it was given through checkout instead of substituting a
  customer identifier that Paddle.js does not accept as a customer auth token,
  encodes non-string custom data as JSON strings consistently across customer
  and checkout requests, and gives its HTTP client the request deadline the
  pinned SDK does not set.

- **The framework crate carries three new modules and two new hard
  dependencies.** `suprnova::live`, `suprnova::render_cache`, and
  `suprnova::view` are unconditional, not feature-gated, so the framework now
  depends on the internal `suprnova-live` engine crate and on `askama` in
  every build. Neither module does anything until an application opts in:
  `Router::try_live()` is what installs Live's reserved routes, and
  `RenderCache::install` with a policy is what makes the cache do anything at
  all.

### Fixed

- **A batch job is not acknowledged until its accounting is durable.** The
  worker acknowledged a successful batch member and then wrote the batch
  bookkeeping, so a failure between the two left a batch permanently short one
  settlement and its completion callbacks never fired. The reservation is now
  held until every accounting write succeeds; a rejected or uncertain write
  leaves it intact so visibility expiry redelivers the job, and the
  repository's `(batch_id, job_id)` uniqueness makes the replay safe even when
  the first write took effect and only its response was lost. The batch
  repository is separately installable and may address a different database,
  which is why this cannot simply share the queue settlement's transaction.

- **A batch's terminal callbacks are elected exactly once.** Two jobs
  finishing the last two entries of a batch could both observe a pending count
  of zero and both run the completion callbacks. Settlement rows are now the
  source of truth and their parent batch row is locked before insertion, so
  concurrent jobs for one batch form a total order and exactly one final
  settlement observes zero - through row locks on PostgreSQL and MySQL, and
  through the writer lock the serialized transaction takes on SQLite. Claiming
  the callback bundle returns the durable completion time and any cancellation
  visible in that same critical section, so a worker cannot choose `then` from
  a snapshot that went stale before ownership was elected, and a non-empty
  batch whose pending count reached zero is sealed against positive growth. A
  cancellation already visible after an uncertain response is not restamped on
  redelivery. Envelope-construction failures collected while building a
  pending batch are surfaced at dispatch, which rejects the whole batch before
  any repository or driver mutation, instead of being swallowed by an
  infallible fluent builder.

- **The Redis queue driver fences every terminal operation against the
  delivery it was issued for.** `ack`, `nack`, `release`, and `settle` now
  compare the stream entry's consumer owner and delivery count with the
  generation captured by `pop`, and one Redis script applies any successor
  publication and the `XACK` together, so a delayed response that makes the
  caller retry finds the generation gone and the retry becomes a no-op instead
  of a duplicate publish. Because `nack` is inherently two commands (`XADD`
  then `XACK`), each reservation retains a per-token lifecycle that stays
  addressable through every failed operation and is removed only after the
  acknowledgement succeeds, so a retry resumes at the step that failed rather
  than republishing. The driver's at-least-once contract and the requirement
  that handlers be idempotent are now stated in the module documentation
  rather than implied. Consumer identities are isolated per process, so two
  workers cannot claim each other's pending entries.

- **A database queue reservation lasts as long as it was asked to.**
  `reserved_until` stores whole seconds and readers compare it with the
  floored current time, so the current fractional second was silently taken
  off every lease and a subsecond timeout could round to nothing. The absolute
  expiry instant is now rounded up.

- **Workflow step writes are fenced, an exhausted attempt budget terminalizes,
  and MySQL date columns match the entities that read them.** A step write
  from a worker that had already lost its claim could land on top of the
  worker that now owns the workflow; writes now carry the claim's fencing
  token. A row whose attempt budget is already exhausted could neither be
  claimed nor left pending forever: the claim statement now terminalizes at
  most one such row per poll, with disjoint cleanup and claim predicates so a
  large abandoned backlog cannot turn one worker poll into an unbounded write.
  The early workflow migrations declared MySQL date columns as `TIMESTAMP`
  while the public entities use `chrono::NaiveDateTime`, whose MySQL storage
  type is `DATETIME`; `NormalizeWorkflowDateTimesForMysql` is an additive
  migration that converts them and is a no-op on PostgreSQL and SQLite.

- **Cancelling a task no longer abandons a transaction's deferred effects.**
  An aborted `DB::transaction` rolls its database work back when SeaORM drops
  the transaction, but nothing rolled back a deferred queue push or released a
  held uniqueness lock for it, and an after-commit callback that was already
  running when the abort landed was dropped mid-effect. Callbacks now run as
  awaited child tasks, in registration order, so one already in flight
  survives its caller's cancellation, and the unstarted remainder is diverted
  to a detached task that either runs the after-commit list (the transaction
  did commit) or compensates (it did not). A panicking callback surfaces as an
  error instead of skipping the callbacks behind it, and a `COMMIT` the
  database refuses after the closure has already taken the request is handled
  rather than panicking.

- **Cross-disk copies and read-through caching clean up after a cancelled
  task.** A mid-stream failure already discarded the partial destination
  object, but a cancellation returns no error at all, so the writer was simply
  dropped and a truncated object or a staged multipart upload was left behind.
  The destination writer is now owned by a guard across the transfer: an error
  settles inline with the same abort and delete as before, while a
  cancellation diverts that cleanup to a detached task, and the cleanup itself
  runs to completion even if the awaiting task is cancelled during it. Cleanup
  never targets a published object, because another writer may have won the
  condition. Local filesystem work is kept alive until it finishes, since
  dropping a Tokio filesystem future does not stop the blocking work it has
  already submitted.

- **Two throttle clauses that hashed to the same storage identity no longer
  share one counter.** A rate-limit rule with several finite clauses could
  collide, so one clause's hits counted against another's budget. Colliding
  clauses now reserve deterministic, unambiguous counter and timer identities,
  computed once so the gate, the deferred hit, and the response headers all
  use the same key, and legacy keys are kept where no collision exists.

- **A sliding-window sweep no longer erases history a longer quota still
  needs.** The in-memory limiter dropped a bucket whose last hit was older
  than the window it was asked about, which discarded the record enforcing an
  already-observed longer quota on the same key. A bucket is now retained
  until its last recorded hit is older than both the supplied window and the
  longest quota window observed for it. A decrement below the minimum amount
  is handled rather than under-counting.

- **The in-memory cache driver rejects an increment against a non-integer
  value.** Redis `INCRBY` and `DECRBY` refuse a live non-integer and leave its
  value and TTL alone; the memory driver overwrote it, so the same code
  behaved differently against the two backends. It now parses before
  inserting and leaves the entry untouched on error. Separately, Redis `add`
  installs a missing untagged value and clears stale tag metadata in one
  script, so a newer tagged overwrite can no longer land between the
  conditional write and the cleanup that follows it.

- **Inertia one-shot session data survives a failed response.**
  `SessionMiddleware` ages `_flash.new.*` into `_flash.old.*` before the
  handler runs, so a response that failed while being constructed left those
  values to be deleted by the next request's aging pass - the user lost the
  validation errors or the flash message that explained what went wrong. A
  request-scoped guard now reflashes them on every uncommitted exit,
  cancellation included, and removes them only after the complete response has
  been built.

- **Fanout broadcasting waits for the backend to say it wrote.**
  `SeaProducer::send` only enqueues, and the returned future is what reports
  the actual backend write, so a delivery was reported as sent when it had
  only been queued. Every send in a pass is now polled for its receipt under
  one deadline, which also stops an unavailable broker from holding an
  application request open indefinitely. Membership heartbeats hold the read
  guard until every snapshot heartbeat is enqueued, so a concurrent untrack
  cannot be overtaken by a stale heartbeat.

- **`suprnova generate-types` never leaves a stale or truncated artifact
  behind.** A scan that ended early used to overwrite the output with whatever
  it had, so a transient parse failure silently deleted type definitions the
  application still used. File generation now refuses to overwrite an artifact
  after an incomplete scan, writes atomically through a collision-free sibling
  temporary file with bounded retries, and skips the write entirely when the
  contents are unchanged. An output symlink keeps its previous behaviour: the
  resolved target is replaced atomically and the link stays in place; a
  changed read-only destination is rejected.

- **Two `suprnova` command failures are reported instead of swallowed.**
  `suprnova new` reports a failed `git init` rather than presenting a project
  as fully created, and `suprnova workflow:install` validates the migration
  path before creating any directory, so an invalid path fails without leaving
  a half-made tree behind.

- **A savepoint and its deferred-effect registry agree on identity.** The
  savepoint statement and the registry mark used the caller's original
  spelling, but names are case-insensitive and PostgreSQL additionally aliases
  names sharing their first 63 ASCII bytes, so `ROLLBACK TO` could unwind a
  savepoint whose deferred effects the registry had filed under a different
  key. Both now share one validated, backend-canonical identity. The accepted
  64-byte API limit is unchanged.

- **`suprnova <command> --help` prints help instead of running the command.**
  The CLI declared its own `help` flag and consulted it only when no
  subcommand had been given, so every other `--help` printed the banner and
  then executed the command anyway. `suprnova migrate:fresh --help` dropped
  every table in the database, and the guard that would have stopped it
  refuses only in production while `APP_ENV` defaults to `local`. Clap now
  owns `-h` and `--help` on the top level and on every subcommand, in either
  argument order, and the curated banner is still what the top level prints.
  A test enumerates the subcommands from clap itself, so a subcommand added
  later is covered without anyone remembering to add it.

- **A scaffolded frontend no longer pins the CSRF token it read at boot.**
  The generated Vue, Svelte, and React entry points read
  `<meta name="csrf-token">` once at module load and attached that value to
  every Inertia visit. Logging in rotates the session, the captured token
  goes stale, and the next state-changing visit - typically the logout - was
  refused with `419 CSRF token mismatch`. Every generated application shipped
  with it. The hook is gone: the Inertia client reads the `XSRF-TOKEN` cookie
  `CsrfMiddleware` sets and echoes it back in `X-XSRF-TOKEN` itself, once per
  request, so the value that travels is the one the browser holds at that
  moment. Server-side verification is unchanged and both header names are
  still accepted. The manual chapter and the `suprnova::csrf` module
  documentation named the old hook as the thing to do; both now name it as
  the thing to avoid.

- **A scaffolded application's XSRF cookie is usable over local HTTP.**
  `CsrfMiddleware::new()` defaults the JS-readable `XSRF-TOKEN` cookie to
  `Secure`, while the generated `env.example` sets `SESSION_SECURE=false` for
  development, so a browser would neither store nor return the cookie over
  `http://localhost` and every state-changing request in development was
  refused with `419`. The generated bootstrap now passes its `SessionConfig`
  to `CsrfMiddleware::with_session_config`, which copies the session cookie's
  `Secure`, `SameSite`, `Domain`, `Path`, and lifetime onto the XSRF cookie
  so the two cannot drift apart. Nothing is weakened: token validation, the
  default origin policy, and the production guards are unchanged.

- **`suprnova make:command` appears on the help screen.** The command
  scaffolds a console command into `src/commands/` and had no line on the
  curated screen, so the only way to learn it existed was to read the source.
  The screen is now checked against the subcommand list clap reports, in both
  directions, so neither a missing line nor a line naming no command can
  survive.

### Upgrading

- **Most applications need no code change.** Live and RenderCache are both
  opt-in: `suprnova::live` does nothing until a router calls
  `Router::try_live()`, and `suprnova::render_cache` does nothing until a
  route is opted in and `RenderCache::install` is called. Everything else in
  this release is a fix to behaviour you already had. The version is 2.0.0
  because the framework's surface grew by two whole subsystems and because of
  the specific behaviour changes listed below, not because the ordinary
  application API was rearranged: no public item in the framework crate, the
  macro crate, or the payment, Magnetar, and Web Push adapter crates was
  removed or renamed.

- **Rebuild times and dependency footprint go up.** The framework now depends
  on the internal `suprnova-live` engine crate and on `askama` in every build,
  because `suprnova::live`, `suprnova::render_cache`, and `suprnova::view` are
  unconditional modules rather than features. There is nothing to enable and
  nothing to disable.

- **An application that never uses RenderCache pays one schema probe per
  process.** `RENDER_CACHE_ENABLED` defaults to `true`, so the first ORM write
  a process makes outside a transaction asks once whether the RenderCache
  migration is present; finding it absent, that process is closed for the rest
  of its life and issues no further RenderCache SQL. Set
  `RENDER_CACHE_ENABLED=false` to skip even that one statement. The probe
  never runs on a caller's transaction, so it cannot poison a write you are
  making.

- **Anyone building a `WebPushClient` from an already-built `reqwest::Client`
  must act.** `WebPushClient::with_client` now refuses to send under the
  default `EndpointPolicy::Strict` and returns
  `WebPushError::UnconfinedRedirects`, because an already-built client's
  redirect policy cannot be inspected and reqwest follows redirects by
  default. Move to `WebPushClient::with_client_builder`, which honours every
  transport option you were setting (proxy, TLS, timeouts) and forces
  redirects off, or call `WebPushClient::allow_unconfined_redirects` if you
  know your client is safe. `WebPushClient::new` is unaffected.

- **Signed URLs issued before the upgrade that carry a trailing slash stop
  verifying.** Signing and verification both used to trim a trailing slash
  before hashing; both now hash the path exactly. A URL signed as
  `/orders/1` still verifies at `/orders/1`, and only at `/orders/1` - a proxy
  that appends a slash now produces `SignatureVerdict::Invalid`. If a proxy or
  a framework in front of your application normalizes paths by adding a
  slash, sign the URL the way the request will arrive.

- **A schedule with two tasks of the same name now fails at registration.**
  `Schedule::add` panics on a duplicate, keeping the task already registered.
  Rename one of them, or switch to `Schedule::try_add` and handle the error.
  Names are exact and case-sensitive.

- **A workflow lease shorter than two seconds is clamped or refused.**
  `WORKFLOW_LOCK_TIMEOUT_SECS` below `2` is clamped to `2` with a warning
  naming the reason; a `WorkflowConfig` built in code with a shorter lease
  fails `validate`. If you were running a one-second lease deliberately, the
  heartbeat could not refresh it before it expired.

- **A MySQL application using workflows should add one migration.**
  `suprnova::workflow::migrations::NormalizeWorkflowDateTimesForMysql`
  converts the `workflows` and `workflow_steps` date columns from `TIMESTAMP`
  to `DATETIME`, which is what `chrono::NaiveDateTime` actually stores. It is
  additive, and a no-op on PostgreSQL and SQLite. MySQL may rebuild and lock
  both tables while applying it, so schedule it accordingly. A project created
  by `suprnova new` gets it wired automatically.

- **`AuthMiddleware` now resolves the user on every guarded request.** It used
  to accept the presence of a persisted identifier. The behaviour change is
  that a session belonging to a deleted or soft-deleted user stops
  authorizing. The cost is smaller than it looks: the resolved user is cached
  for the rest of the request, so a handler that already called `Auth::user()`
  pays nothing extra and the lookup has simply moved from the handler to the
  middleware. Only a guarded request whose handler never resolved the user
  gains a provider lookup it did not make before. An application with no user
  provider bound keeps the identifier-only path unchanged.
  `BasicAuthMiddleware` in its non-stateless
  form now requires the guard it names to exist and to be a stateful guard.

- **`destroy_all_for_user` costs more and revokes more.** It now reads the
  surviving session rows and compares the guard identities inside each
  payload, in addition to the indexed `user_id` match, so it reaches sessions
  authenticated through a named guard alone. Revocation is rare enough that
  correctness was chosen over index use; if you call it on a hot path, that is
  worth knowing.

- **A `POST` or `PATCH` retried after a transport error now needs
  `retry_non_idempotent`.** The `5xx` branch already required it; the
  transport-error branch did not. If you were relying on a dropped connection
  being retried for a non-idempotent request, opt in explicitly.

- **A production process with a missing or malformed `APP_KEY` now fails at
  `#[suprnova::main]`, not at `Server::from_config`.** That includes a console
  binary or a worker entry point that never builds a server. Local,
  development, and testing environments still generate a transient key and
  warn. `APP_PREVIOUS_KEYS` is accepted as an alias for `APP_KEY_PREVIOUS`;
  if both are set with different values, the Suprnova name wins and the
  duplicate is named in a warning you should act on.

- **A worker pointed at a `failover` queue connection now drains every
  connection in the list.** If you were running a second worker against a
  fallback connection directly - which the 1.3.3 notes told you to do - that
  worker and the failover worker will both be draining it. Remove the extra
  worker, or keep it and accept the competition. Counters and listings now
  aggregate every connection, so a dashboard reading `pending_size` on the
  failover connection will report a larger number than it did.

- **Custom store and driver implementations get fail-closed defaults, never
  weakened behaviour.** Magnetar's provider-token, ceremony, and remember
  stores gained methods for the atomic operations described above; each has a
  default that refuses rather than performing the operation non-atomically, so
  an external implementation still compiles but will report failure until it
  implements the method. `WebhookHandler::mirrors_payment_transactions` and
  `try_extract_payment_snapshot`, `QueueDriver::queue_filter_capability`, and
  `GlobalScope::dependency` all have defaults that preserve the previous
  behaviour exactly.

- **To adopt Live**, bind a registry during bootstrap with
  `App::singleton(crate::live::registry().expect("Live component registry"))`,
  install the reserved routes with `Router::try_live_with` and a guard
  carrying your `AuthMiddleware`,
  `LiveTenantMiddleware`, and `RateLimitMiddleware`, and register
  `CsrfMiddleware::new()` globally - Live verifies its own origin proof, so
  you do not need to widen `OriginPolicy` for the whole application, and if
  you widened it for something else, narrow it back. Use
  `AuthMiddleware::optional()` on the Live guard if you want anonymous
  visitors to act on public seeds; `AuthMiddleware::new()` answers `401` for
  every anonymous request before any engine work. Then run
  `suprnova live:make` and `suprnova live:check`. `manual/live.md` has the
  complete walkthrough.

- **To adopt RenderCache**, add
  `suprnova::render_cache::migration::Migration` to your `Migrator` (and
  `suprnova::render_cache::migration::TierMigration` as well if you run the
  `database` or `redis` profile), opt routes and groups in with
  `Router::try_render_cache` and `Router::try_render_cache_group`, and finish
  with `RenderCache::install`. Because `install` is asynchronous - it probes
  for the ledger's tables before assembling a runtime - the router has to be
  built through `Application::try_routes_async` rather than `try_routes`. The
  install has to come after every middleware that establishes request-scoped
  locale, session, or identity, and after every route and group has been opted
  in. A shared profile refuses to boot without its tier migration or with an
  endpoint nothing answers, which is deliberate.

- **Set `APP_BUILD_ID` once per deploy.** It is mixed into every RenderCache
  lookup key, so changing it is what stops a new build from serving entries
  the previous one published. Its default is your application crate's package
  version, which does not change when you ship a template, a translation, or a
  handler fix. A commit id works: `APP_BUILD_ID=$(git rev-parse --short HEAD)`.

- **Consider moving your `Cargo.toml` to the production build shape.** Declare
  `suprnova` with `default-features = false` plus the nine non-`testing`
  defaults you use, and re-add `features = ["testing"]` under
  `[dev-dependencies]`. Cargo pulls a dev-dependency's features into
  `cargo test` and other `--tests` builds only, so your shipped binaries stop
  carrying test seams while `cargo test` keeps working unchanged. A project
  created by `suprnova new` is already in this shape;
  `manual/deployment.md` documents it for an existing one.

- **If you consumed the standalone `@suprnova/live` runtime or wrote your own
  subscription host, the registered-event descriptor changed.**
  `DESCRIPTOR_SCHEMA_VERSION` moved from 1 to 2 and the descriptor's
  registered-event fields are now `maximum_hops`, `maximum_fanout`, and
  `payload_contract` rather than `maximumHops`, `maximumFanout`, and
  `payloadContract` - they were the only camelCase keys in a public JSON
  contract that is snake_case everywhere else. A descriptor signed at schema
  version 1 is refused with `SubscriptionErrorKind::InvalidDescriptor` rather
  than being read with three absent fields. Nothing in a 1.3.7 Suprnova
  application consumed this contract, so for most readers there is nothing to
  do.

## 1.3.7 - 2026-08-26

### Added

- **Where the Inertia error page middleware sits is now yours to choose, and documented.** `Inertia::install` registers `InertiaErrorPageMiddleware` innermost of the Inertia layer, so it covers the handler, the route middleware, and everything you register after that call - which is why the scaffold puts `CsrfMiddleware` below it. It does not cover anything registered *above* the call, because a middleware that answers without calling `next` hands its response to nothing registered inside it. The case that bites is a lapsed session posting a form: `CsrfMiddleware` registered above the install answers `419` with `{"message":"CSRF token mismatch."}` and the user gets the Inertia crash modal on the one flow they are most likely to hit; an outer rate limiter's `429` and an auth guard's `401` are the same. Registering the middleware yourself, further out, already worked in 1.3.6 - the type was public and registration is idempotent per type, so an earlier registration kept its place - but nothing said so and nothing in `install` acknowledged it, which made it an accident rather than a contract. It is a contract now: register `InertiaErrorPageMiddleware::new("Error")` after `SessionMiddleware` and `LocaleMiddleware` and before the middleware whose rejections it should cover, and `install` checks for it, logs at `debug`, and skips its own. The component you named at that registration is the one rendered, so you name the page once and `.error_page(...)` on the config becomes optional - it is still what makes `install` register a middleware for an app that does not place one itself. The two ordering rules are documented on the type and in the manual.

### Fixed

- **An SSR page has one `<title>`, and it is the page's own.** The HTML shell wrote its `default_title` and then the SSR worker's head verbatim, so every page rendering a title through Inertia's `Head` component produced a document with two `<title>` elements and the framework's generic one first. First is the one the browser tab, the crawler and the link preview read, so the real title never showed. A worker head carrying a title now replaces the shell's title rather than joining it - both `default_title` and a per-response `InertiaResponse::title(...)` stand down; a head without one leaves the shell's title exactly where it was.
- **The document declares the language it is written in.** The shell hardcoded `<html lang="en">`, so a reader switched to Japanese got Japanese prose in a document claiming to be English - a screen reader picks its voice from that attribute and a search engine takes it as the page's language signal. It now carries the locale in effect for the request: what `LocaleMiddleware` detected, then a `Lang::set_locale` override, then the configured `APP_LOCALE`, in the same BCP 47 form `Locale` renders (`pt-BR`, `zh-Hans`). This holds for the error page too, which is rendered on the way out and was the case that surfaced it. Without the `localization` feature the shell keeps `en`.

### Upgrading

- Nothing is required. Both fixes apply to every Inertia app on upgrade, and `Inertia::install` behaves exactly as it did for an app that does not register the error-page middleware itself.
- An app that spliced `<html lang="...">` into the finished document with a middleware of its own can delete it - the shell does it now, from the same locale that middleware was reading.
- An app whose `CsrfMiddleware`, rate limiter, or auth guard is registered **before** `Inertia::install` should register `InertiaErrorPageMiddleware::new("Error")` after `LocaleMiddleware` and before that middleware, so its rejections render the error page instead of reaching the client as raw JSON. `install` then skips adding its own, and the component you named at the registration is the one rendered, so `.error_page("Error")` on the config is optional - keep it or drop it. The scaffolded `bootstrap.rs` registers CSRF after the install, so a project generated by `suprnova new` needs no change.
- An app that renders its own `<title>` through Inertia's `Head` component under SSR will see the shell's title stop appearing in the document - both `InertiaConfig::default_title` and a per-response `InertiaResponse::title(...)`. That is the fix: the page's own title is the document's only one. If you were relying on the shell's title as a prefix or suffix, move it into the `Head` component where the rest of the title lives.

## 1.3.6 - 2026-08-26

### Added

- **Framework errors can render your own Inertia page instead of the client's crash modal.** A user without a permission clicked a nav link into a guarded route and got Inertia's "All Inertia requests must receive a valid Inertia response, however a plain JSON response was received" screen: the `403` carried the framework's JSON error body and no `X-Inertia` header, so the client refused it. The same held for an unrouted `404`, a rate-limited `429`, and a failing handler's `500`. Name a page component with `InertiaConfig::error_page("Error")` and those responses render that page at their original status, with `status`, `message`, and - when the error carried one - `request_id` props. Every header the error response set survives the swap except the ones that only described the body being replaced (`Content-*`, `Transfer-Encoding`) or governed how it could be stored (`Cache-Control`, `Expires`, `Age`, `ETag`, `Last-Modified`), so `Retry-After` on a `429`, `WWW-Authenticate` on a `401`, `Vary`, and `Set-Cookie` all still reach the client. The page sets `Cache-Control: no-cache, private` for itself: it carries your shared props, so it must never be stored by a shared cache and served to a different visitor, whatever the response it replaced permitted. An Inertia visit gets the JSON page object; a hard navigation gets the full HTML shell, so pasting the URL into the address bar works too. Everything with an owner is left alone: validation `422`s still redirect back to the form, `X-Inertia-Location` bounces and responses that already are Inertia pages pass through, and a client whose `Accept` prefers JSON keeps the exact body it got before. `suprnova new` scaffolds `frontend/src/pages/Error.*` and sets `.error_page("Error")`, so new projects are covered without doing anything.

### Fixed

- **A local disk no longer refuses a legitimate path because another task touched it.** The path guard resolved each component of a path with two probes and combined them into one verdict, so ordinary concurrent activity could be read as a symlink escape: a component that `canonicalize` had just reported missing, and that another task then created as an ordinary file, came back as `PermissionDenied` naming a symlink that was never there. It bit hardest where writers contend by design - a losing `write_with(..).if_not_exists(true)` racer got that refusal instead of `ConditionNotMatch` whenever the winner published the key between the two probes, which under a loaded test suite was roughly a third of runs. Each component is now classified from a single pass, `symlink_metadata` first: nothing there is free space, an ordinary file or directory is resolved and confined as before, and only a symlink that still cannot be resolved is refused. A component that vanishes mid-classification is looked at once more rather than refused. Every symlink refusal is unchanged.

### Upgrading

- Nothing changes for an existing app until it opts in. `InertiaConfig::error_page` defaults to `None`, and `Inertia::install` registers the error-page middleware only when a component is named, so error responses keep their exact bodies. To adopt it, add a page component named `Error` beside your others (it receives `status`, `message`, and an optional `request_id`) and chain `.error_page("Error")` onto the `InertiaConfig` you pass to `Inertia::install`. A handler that **panics** stays out of scope: the panic net wraps the whole middleware chain, so its synthesized `500` is built after every middleware has unwound. Return `Err(...)` rather than panicking and the error page covers it. Note that the gate is the body's **shape**, not its author: at an error status, an empty body, a JSON object whose `message` is a string, and the router's own `404 Not Found` text are rewritten no matter which middleware built them, and only `message` and `request_id` survive into the props. A response that must keep its own JSON body should key its text as something other than `message`, or set `X-Inertia: true` on itself. And register `LocaleMiddleware` **before** `Inertia::install`: the error page is rendered on the way out, after every middleware registered inside the Inertia layer has returned, so a locale scope opened inside it is already gone and every error page would render in the app's default locale. The scaffolded `bootstrap.rs` now does this, and the same reasoning applies to any request-scoped middleware of your own whose state the page's shared props read.

## 1.3.5 - 2026-08-26

### Changed

- **Every changelog section reads in all six manual translations.** The de, es,
  fr, ja, pt-BR and zh-Hans manuals used to carry the 1.3.0 to 1.3.2 sections in
  English behind a translator's note, and older sections with stray English
  lines; every section from 1.3.5 back to 0.1.0 is now translated, and the
  notes are gone.

### Fixed

- **Local-filesystem disks publish every object in one step.** `Storage::register_fs` and `register_fs_with` now stage `disk.write(...)`, `disk.writer(...)`, and `disk.copy(...)` as a temp file under `<root>/.suprnova-atomic/` and publish it onto the target with a single `rename(2)`, so none of them is ever observable at a partial length. Before this, the driver opened the target with `create + truncate` and streamed into it in place: a concurrent reader got an empty or half-written object for the whole duration of the write, and a crash mid-write left a truncated object at the live path. `abort()` on a writer now discards the staged file instead of failing with `Unsupported`.
- **`write_with(..).if_not_exists(true)` is a true exclusive create on a local disk.** It is published with `link(2)`, which fails atomically in the kernel when the target exists, so exactly one of any number of racing callers succeeds and every other one gets `ConditionNotMatch` having written nothing. A staged write published by a plain rename would have degraded the condition to a check followed by an overwrite, silently discarding all but the last writer - which is the opposite of what the primitive is reached for.
- **An `append` that creates the object is still an append.** Appends are the one in-place operation on a local disk, and that now holds for the first one too, so two writers appending to the same missing object both land instead of one staging its own copy and overwriting the other.

- **`suprnova serve` no longer rebuilds a project nobody has touched, and
  neither does `suprnova generate-types --watch`.** Both watchers classified a
  filesystem event by its path alone, and the generator reads every `.rs` file
  under the same `src/` tree they are watching - so on Linux, where the kernel
  reports those reads, each regeneration scheduled the next one. A freshly
  scaffolded project regenerated its types and restarted its backend every half
  second, forever, without a single source edit. Only events that mean the bytes
  on disk actually changed count now. `generate-types --watch` also had no
  debounce at all, so it acted on the first file of a burst rather than the last;
  it now shares `serve`'s 500 ms trailing edge, and both watchers share one
  implementation so the next fix cannot land in only one of them. The generator
  compares before it writes, so a regeneration whose output is byte-identical
  leaves the file, and its mtime, alone.

- **The backend watcher is scoped to the paths the server is built from.**
  `cargo watch` ran with no `-w`, so it watched the whole non-gitignored project:
  saving a Svelte component, or regenerating
  `frontend/src/types/inertia-props.ts`, rebuilt the framework and restarted the
  server. It now watches `src/`, `cmd/`, `Cargo.toml`, `Cargo.lock`, `.env`, and
  `lang/` - the build inputs plus the two trees read once at boot - each included
  only when it exists, since cargo-watch refuses a `-w` path that does not.
  `cmd/` is where the full-stack scaffold keeps the server binary's `main.rs`.
  The invocation also passes `--no-vcs-ignores`, because cargo-watch applies
  `.gitignore` to explicitly named `-w` roots and the scaffold ignores `.env`,
  which would otherwise leave `-w .env` watching nothing; `-w` has already
  narrowed the surface, so the flag cannot widen it. Frontend edits and generated
  `.ts` files no longer restart the backend.

- **`serde_json::Value` generates as `JsonValue` instead of `unknown`.** It used to
  degrade to `unknown` and warn that it "isn't a struct this project defines",
  advice that is wrong for a JSON document - and the scaffold's own login and
  register pages tripped it twice on every regeneration, so every fresh project
  warned out of the box. It now emits a recursive `JsonValue` alias, declared once
  at the top of the generated file and only when something references it. A bare
  `Value` maps there too, unless the project defines a `Value` struct of its own.

- **Neither `generate-types` nor `serve` reports a file it did not write as
  generated.** Because a pass now writes only when the emitted content differs,
  `Generated <path>` was a claim about the filesystem that was false on every
  rerun of an unchanged project. `generate-types` says `<path> is up to date`
  instead, in one-shot and `--watch` alike, and `serve`'s startup pass says
  `N type(s) up to date → <path>`, keeping the count. `serve`'s file watcher
  now stays silent on a regeneration that wrote nothing, in text and under
  `--json` both: a `types_regenerated` event means the generated file on disk is
  different now, so silence after a save tells you your edit did not change any
  prop shape.

### Upgrading

- **`.suprnova-atomic` is reserved at the root of every local disk.** The staging directory has to live inside the root - a sibling of the root can be on a different filesystem when the root is a mount point, and every rename would fail with `EXDEV` - so the name is reserved rather than merely conventional. Any path whose first component is `.suprnova-atomic` is now refused with a permission error (read, write, delete, stat, list alike), as is any path that resolves into the directory through a symlink, and the entry is filtered out of `files`, `directories`, `all_files`, and `all_directories`. If a disk root already contains a `.suprnova-atomic` entry of your own, it is no longer reachable through that disk: move it aside before upgrading. A regular file of that name is refused at registration with a message saying so, rather than failing later inside the driver. The name is exported as `suprnova::ATOMIC_STAGING_DIR` so backup and sync tooling can exclude it.
- **Publishing by rename replaces the target's inode.** Rewriting an object on a local disk no longer preserves its mode, owner, or hard links, and a reader holding an open descriptor keeps the old content instead of seeing the new bytes. That is the standard cost of atomic publishing, but it is a behavior change if you were relying on either.
- **A conditional write needs a filesystem with hard links.** `if_not_exists` is published with `link(2)`, which is unsupported on FAT, exFAT, and some network filesystems. There it fails outright rather than falling back to a check followed by an overwrite, because a fallback would hand you an exclusivity guarantee that does not hold. Nothing else on the disk is affected.
- **A first `append` that fails leaves an empty object.** An append is the one operation that is not published in a single step, so the object is created before the bytes land; a failed or aborted first append leaves it behind, exactly as an append onto an existing object always has.
- **A dangling symlink in the disk root is refused, not overwritten.** A path whose symlink target does not exist can no longer be written, appended to, copied onto, moved onto, or deleted through the disk. `1.3.4` replaced such a link with a regular file; the guard cannot prove where an unresolvable link leads, and creating through one creates the link's target anywhere on the host, so it now refuses. Remove the link outside the disk if you meant to write there.
- **Nothing sweeps the staging directory.** It holds in-flight temp files plus whatever a process that died mid-publish left behind, so a host in a crash loop grows it without bound. Emptying it while nothing is writing to the disk is safe; excluding it from backups is recommended.

## 1.3.4 - 2026-08-25

### Added

- **Read-through disks take a `copy` flag and resolve `copy` / `rename` across the fallback.** Set `copy: false` on `ReadThroughConfig` to serve fallback hits without writing them through, which turns the disk into a transparent overlay and narrows each fetch to the range you asked for. `copy` and `rename` now stream a source that lives only on the fallback across to the primary destination; a `rename` also deletes the fallback source, so a later read cannot resurrect the moved object. Conditions carry across that streaming path: `if_not_exists` still refuses an existing destination, a copy's source version selects which object the fallback hands over, and a copy's `if_match` is refused with `Unsupported` rather than silently dropped. A transfer that fails partway removes only a destination it created, so it cannot destroy an object that was already there.
- **Debounced jobs and debounced queued listeners.** `Job::debounce_for()` collapses
  a burst of dispatches into one run, one window after the most recent one, carrying
  the newest payload. It is the mirror of `push_unique`, which keeps the first
  dispatch and suppresses the rest. `Job::max_debounce_wait()` stops a continuous
  burst from deferring the work forever, and `Job::debounce_id(&self)` scopes the
  window per entity so twenty updates to one order collapse without touching
  another order's. `Queue::push_debounced(job, DebounceOptions)` sets the window at
  the call site, and `DebouncedListener::new(window, build).keyed_by(...)` debounces
  an event listener with the key derived from the event - a plain `QueuedListener`
  already honors a window the job itself declares. Every dispatch is still enqueued;
  the collapse is settled at the worker, which acknowledges a superseded envelope
  and emits `JobDebounced`. Debouncing fails open: an expired or evicted window runs
  the job rather than dropping it. Each actual run starts a fresh maximum-wait
  window, so a burst always measures its maximum wait from its own first dispatch
  rather than inheriting the previous burst's. A job cannot declare both
  `debounce_for` and `unique_id`, and chains and batches refuse a debounced job -
  a superseded link would strand the rest of its chain, and a superseded batch job
  would leave the batch's pending count above zero forever. The envelope carries two
  additive fields for this and stays byte-identical on the wire for every
  non-debounced push.

- **`Storage::register_read_through` composes two disks into a read-through disk.** Reads and metadata resolve against the primary first and fall back to the second disk; anything found on the fallback is written through to the primary, so a store migration completes under real traffic. Writes and listings stay on the primary, and a delete removes the object from both disks. Set `throw_on_promotion_failure` when a failed promotion must surface instead of degrading to a fallback read. A promotion is published atomically, so no reader can see a half-written object, and it carries the fallback object's content type, cache control, content disposition, content encoding, and user metadata across. A versioned or conditional read is passed through with its condition intact and served without being promoted.
- **`Queue::forward` redirects a whole queue by name.** Where `Queue::route` is
  keyed by job type, `Queue::forward("default", "high")` is keyed by queue name -
  the lever for retiring a pool, absorbing a backlog, or moving work off a pool you
  are about to take down, without touching a single job or route. It applies on
  both sides: new pushes that resolved to `default` land on `high`, *and* a worker
  started with `--queue=default` drains `high`, so the destination cannot collect
  work nobody claims. Forwarding `default` catches jobs that named no queue. A
  forward is a single lookup, never a chain, so a swap (`a -> b` with `b -> a`
  also registered) or a longer rotation is a coherent pool exchange rather than
  a loop - exactly like Laravel, whose resolver is the same single lookup.
  Pausing is still evaluated on the names a worker was started with, so
  `Queue::pause(&connection, "default")` stops that worker even while `default` is
  forwarded. `Queue::forward_on(from, to, connection)` restricts a forward to one
  connection name, compared against this process's connection name rather than a
  job's declared connection, so both halves of the redirect gate on the same
  value. `Queue::forward_for(from)` reads a forward back, and `Queue::try_forward`
  is the fallible sibling. The inspection calls (`Queue::pending_jobs` and its
  siblings) deliberately do not follow a forward, so a backlog left behind on a
  forwarded queue stays visible.

- **Read-shaped Redis commands retry a transient failure instead of surfacing it.**
  The connection manager already reconnected in the background, but the command
  that hit the dead socket still failed your call. `GET`, `EXISTS`, the `SCAN`
  and `SSCAN` pages behind `Cache::flush` / `Cache::flush_tags`, the queue
  driver's `XLEN` / `ZCARD` / `XPENDING` reads, and the rate limiter's
  `Retry-After` computation now retry once after a short pause.
  `REDIS_COMMAND_RETRIES` adds further retries on top, clamped at 10. Budget the
  retry in seconds rather than milliseconds: the second attempt waits for the
  replacement connection, so it costs the driver's whole connect and response
  budget, and a timed-out command counts as transient as well as a dropped one.
  Writes never retry at any setting: a transient error means the connection
  failed, not that the server refused the command, so repeating a `SET`, an
  `INCR`, a lock acquisition, a rate-limit hit, or a queue pop could run it
  twice. Error messages are unchanged, so anything matching on them keeps working.
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

- **The maintenance bypass cookie now expires on the server.** The 12-hour TTL was a `max-age` the browser enforced, so a captured cookie kept working until you rotated the secret. The encrypted payload now carries the deadline, and every request re-checks it.
- **`suprnova serve` runs a frontend-less project.** A project scaffolded with `suprnova new --api` has no `frontend/` directory, and `serve` rejected it as "No frontend directory found. Are you in a Suprnova project directory?" unless you passed `--backend-only`. It now skips the Vite pane and the TypeScript generation that feeds it, and serves the backend. `--frontend-only` still fails on such a project, with a message that says why.

### Upgrading

- **Bypass cookies issued before this release stop working.** The cookie's payload changed from the bare secret to a sealed `{ secret, expires_at }` object, and a payload with no deadline is refused. Visit the secret URL once after upgrading to get a new cookie. Nothing else changes: `down`, `up`, `--secret`, and `--with-secret` all behave as before.
- **An include path longer than five segments now returns its first five relationships instead of all of them.** Nothing outside a resource's allowlist was ever reachable, so no response gains data; a deep path loses its tail. One status code changes with it: a path whose over-deep tail names a relationship the resource does not allow is truncated before anything validates it, so it now returns `200` with the segments that survived where the full path used to return `400` - adjust any client or test asserting on that rejection. Raise the ceiling with `suprnova::max_relationship_depth(n)` if your API documents paths longer than that.
- **`DatabaseConfig` gained five public fields.** Code that builds one with a struct literal no longer compiles. Use `DatabaseConfig::from_env()` or `DatabaseConfig::builder()`, both of which fill the new fields with the defaults that preserve today's pool behavior.

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

- **OAuth can now be installed without replacing an application's existing
  password and session authority.** `MagnetarOAuthOnlyConfig` and
  `init_magnetar_oauth_only` install the default ceremony and provider engine
  while leaving the password and passkey slots empty. Applications with an
  existing `users` table can call `verify_oauth_identity`, map the verified
  provider subject themselves, and establish their normal framework session.

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

### Fixed

- **Installing OAuth no longer forces provider-backed applications into
  Magnetar web-binding validation.** The full `init_magnetar` path remains
  atomic and unchanged. The OAuth-only path reserves the engine slots during
  construction, publishes only OAuth, and fails rather than mixing two
  authentication authorities.

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

- Use `init_magnetar_oauth_only` instead of `init_magnetar` when the application
  already owns users, passwords, framework sessions, and remember-me state.
  OAuth-only callbacks use `verify_oauth_identity`; full Magnetar applications
  continue to use `complete`.

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

- **The SES transport now sends custom message headers.**
  `Mail::to(..).header("List-Unsubscribe", ...)` and `Mailable::headers()` were
  dropped silently under `MAIL_DRIVER=ses`: the `Content.Simple` request body
  had no `Headers` field and the raw-MIME builder never read
  `OutgoingMessage::headers`, even though every other transport forwards them.
  Both SES paths now carry them - `Headers` as SES v2's `{Name, Value}` list,
  raw MIME as real header lines - so unsubscribe links, threading headers and
  routing hints survive a driver swap. Header names are validated up front on
  both paths - CR, LF and NUL (the injection bytes, as the Mailgun transport
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
  "next step" that got skipped, which is why v0.5.10 and v0.6.1-v0.6.3 are
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
