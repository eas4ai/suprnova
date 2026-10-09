# Laravel parity

Status: Draft
Prefix: PAR

Suprnova tracks Laravel 13.34.0 and Inertia 3.7.1. The parity map,
`feature-map/laravel/parity.jsonl`, holds one row per Laravel item with the
classification of it: build, not built by design, or does not apply. The
developer ruled on part of the map: the recommendations accepted on
2026-09-30, and the open questions of the third review on 2026-10-01. A row
says "Developer ruling <date>" only where the developer ruled on it. This
file turns the rows ruled "build" into requirements, one commitment at a
time; the map stays the complete list. A requirement here names the
capability and the Laravel behavior it matches; the map rows it covers say
"Developer ruling ...: build" until the work ships, when they are linked to
the Suprnova items that implement them.

Every requirement inherits the repository gate, which runs the database
tests against Postgres, MariaDB and MySQL as well as SQLite.

## Requests from an application port

An application team porting a Laravel 13 application filed issues #125 to
#129 for the gaps it met in application code. The developer asked for
them to be done on 2026-10-01. The queue fake gaps (PAR-009) are among the
recommendations the developer accepted on 2026-09-30.

[PAR-001] The `DB::table` builder and the model query builder MUST offer
joins: `join`, `left_join`, `right_join` and `cross_join` against a table
with an optional alias; a closure form whose conditions combine `on`,
`or_on`, `where` and `or_where`; `join_sub` and `left_join_sub` against
another builder under an alias; and `where_exists` and `where_not_exists`
taking a builder, which may correlate with `where_column`. An `or`
condition MUST be flat, as Laravel compiles it: `where(a).or_where(b).where(c)`
is `a OR b AND c`, never `(a OR b) AND c`, so a chain ported from Laravel
compiles to the same SQL. `where_column` MUST accept an operator between
the two columns, `cross_join` MUST accept the closure form for its `ON`
conditions, and a builder with a join MUST offer `update` and `delete`
over the joined rows. Every value in a join or exists clause MUST be bound
as a parameter, never written into the SQL text, and every table, alias
and column MUST be quoted for the active backend; an identifier or operator
the builder does not know MUST be refused. A model query with a join MUST
select the model's own table columns unless told otherwise, so a joined
table's `id` never overwrites the model's.
Falsifier: one of the three query shapes in issue #125 (chained left joins with a table alias and aliased columns, a left join against a grouped subquery, a correlated `where_exists`) returns rows that differ from the same query written as raw SQL; `where("a", 1).or_where("b", 2).where("c", 3)` compiles with parentheses around the `or`, or returns rows that `a = 1 OR b = 2 AND c = 3` does not; `where_column("a", ">", "b")` is refused or ignores the operator; `cross_join` with a closure drops its conditions; an `update` or `delete` on a joined builder is refused or touches rows the join does not match; a value given to a join's `where` appears in the generated SQL text; `where("a; drop", 1)` or an operator such as `=;` compiles; or a model query joining another table hydrates the joined table's `id` into the model.
Mechanism: `par-joins`.
Rationale: Issue #125 and row 001 of the parity log (the flat `or`, `whereColumn`'s operator, `crossJoin`'s `ON` and the joined `update` and `delete` of `Illuminate\Database\Query\Builder`); the identifier and operator refusals are SQL safety and stay.
Status: Agreed 2026-10-08

[PAR-002] The framework MUST offer file responses: a response streaming a
file from a path with the content type its extension implies and
`Content-Disposition: inline`; a download response with `attachment` and
a filename the caller chooses; a download of in-memory bytes with a given
content type; a streaming download that writes a body as it is produced,
under a filename; and the same responses for a path on a named storage
disk, resolved through the disk so its path guard applies. A file and a
download response MUST accept extra headers and a disposition argument,
MUST send `Last-Modified` from the file's modification time and
`Accept-Ranges: bytes`, MUST answer a `Range` request with `206`, the
requested bytes and `Content-Range`, and `416` with `Content-Range: bytes
*/<size>` for a range the file cannot satisfy, and MUST send the
`Cache-Control` the caller sets. Every `Content-Disposition` header it
writes MUST follow RFC 6266: an ASCII `filename` fallback, plus
`filename*=UTF-8''` with percent-encoding when the name is not plain
printable ASCII, and no character of the name may change the header's
structure.
Falsifier: a download named `Certificat·Joan Pérez.pdf` produces a header without a correct `filename*` or with a raw non-ASCII byte in `filename`; a name holding a quote, backslash, CR or LF changes the header's structure; a disk download of `../secret` reads outside the disk root; a file response sends a content type or disposition other than the one specified; `Range: bytes=0-9` on a file response answers anything but `206` with those ten bytes and `Content-Range: bytes 0-9/<size>`, or `Range: bytes=<size>-` anything but `416`; a file response lacks `Last-Modified` or `Accept-Ranges`; a header or `Cache-Control` the caller passes is missing; or a streaming download buffers its whole body before the first byte is sent.
Mechanism: `par-file-responses`.
Rationale: Issue #126 and row 002 of the parity log: Laravel's `ResponseFactory::file`, `download` and `streamDownload` take headers and a disposition, and `BinaryFileResponse` answers ranges with `Last-Modified`; the inline disposition, the extension-based type and the slash handling are the earlier spec's choices and stay.
Status: Agreed 2026-10-08

[PAR-003] `#[handler]` MUST accept `#[authorize(ability, Type)]`, which
authorizes the ability against a model type, and `#[authorize(ability,
param)]`, which authorizes it against the route-bound model the handler
receives as `param`; the ability MAY be a string or a variant of an enum
that converts into the ability name. The check MUST run after route model
binding and before the handler body, through the async gate, so policies
and the RBAC gate bridge both apply. A policy whose method takes an
optional user MUST be consulted for a guest, as Laravel consults a
nullable-user policy: the guest passes when it allows and gets 403 when
it denies; a policy that requires a user MUST answer 401 to a guest. It
MUST answer 403 when the gate denies, and 404 when the policy denies as
not found. A `param` the handler does not take MUST be a compile error.
Falsifier: a handler declaring `#[authorize("update", post)]` runs its body for a user the policy denies; answers 401 to a guest whose policy method accepts an optional user, or runs its body for a guest that method denies; answers 403 to a guest where the policy requires a user; answers 403 where the policy denies as not found; answers 403 instead of 404 for a route model that does not exist; `#[authorize(Ability::Update, post)]` with an enum that converts into the ability does not compile or checks another ability; or it compiles when `post` is not one of its parameters.
Mechanism: `par-authorize`.
Rationale: Issue #127 and row 003 of the parity log: Laravel's `Authorize` middleware passes `null` to a policy method typed with a nullable user and accepts a `BackedEnum` ability; the compile-time parameter check and stacked attributes are the typed form and stay.
Status: Agreed 2026-10-08

[PAR-004] After a successful save, a model MUST report which attributes
that save changed: `was_changed` for one attribute or several,
`get_changes` for all of them, and `get_previous` for each changed
attribute's value before the save. A save whose attributes are all equal
to the values loaded MUST run no `UPDATE` and fire neither `updating` nor
`updated`, while `saving` and `saved` still fire, as Laravel's `save`
skips `performUpdate` for a clean model; dirtiness is judged against the
values loaded or last saved in memory, not by reading the row again.
While the save's `updated` and `saved` observers run, `get_original` and
`get_raw_original` MUST return each attribute's value as it was loaded
before the save; once the save returns, the original values are the saved
ones. An insert reports no changes.
Falsifier: in an `updated` observer for a save that flips `is_admin`, `was_changed("is_admin")` is false, `was_changed` reports an attribute the save did not change, `get_changes` omits or adds an attribute, `get_previous("is_admin")` is not the value before the save, or `get_raw_original("is_admin")` is not the value loaded before the save; after the save returns, `get_original("is_admin")` still reports the value loaded before it; or a save of a model whose attributes are unchanged issues an `UPDATE`, fires `updating` or `updated`, or skips `saving` or `saved`.
Mechanism: `par-model-changes`.
Rationale: Issue #128 and row 004 of the parity log: Laravel's `Model::save` calls `performUpdate` only when `isDirty`, and `HasAttributes::getPrevious` returns the previous values `syncChanges` kept.
Status: Agreed 2026-10-08

[PAR-005] A belongs-to-many relation MUST offer `sync` and
`sync_without_detaching`, which attach each given id the relation does not
already hold and, for `sync`, detach the ids not given; both MUST accept
pivot columns per id and update the pivot row of an id already held when
its given columns differ; both MUST return the attached, detached and
updated id lists, as Laravel's `sync` does; both MUST honour the
relation's pivot filters (`where_pivot` and its kin), so a filtered
relation syncs only the rows the filter sees; and both MUST touch the
parent's `updated_at` when the relation touches its parent. Every other
existing pivot row, its pivot columns included, MUST stay untouched; the
operation runs in one transaction and an id of the wrong type is refused.
Falsifier: after `sync_without_detaching` with id 3 on a relation that holds 1 and 2, the relation holds anything other than 1, 2 and 3, or a pivot column of the rows for 1 or 2 changed; `sync([2, 3])` on that relation does not report `attached: [3]`, `detached: [1]`, or reports an update for an unchanged row; `sync_without_detaching` with `{2: {role: "lead"}}` does not change row 2's `role` or does not report it under `updated`; a `where_pivot("role", "lead")` relation's `sync` detaches a row the filter does not see, or is refused; or a touching relation's `sync` leaves the parent's `updated_at` as it was.
Mechanism: `par-sync-without-detaching`.
Rationale: Issue #128 and row 005 of the parity log: Laravel's `InteractsWithPivotTable::sync`, `syncWithoutDetaching`, `formatRecordsList`, `attachNew` and `touchIfTouching`; the always-on transaction and the typed id matching are stricter and stay.
Status: Agreed 2026-10-08

[PAR-006] The `DB::table` builder and the model query builder MUST offer
`where_any`, `or_where_any`, `where_all`, `or_where_all`, `where_none` and
`or_where_none` (one comparison across several columns, grouped in
parentheses), `or_where_in` and `or_where_not_in` with a list or a
subquery, `where_in` and `where_not_in` with a subquery, `or_where_raw`
with bound values, `or_where_null` and `or_where_not_null`, and `reorder`,
which drops the orderings already set and may set a new one. Every `or_`
helper MUST be flat as PAR-001 says; `db_where(column, value)`, its
sibling `filter(column, value)` and `or_where(column, value)` MUST be
the two-argument shortcut for `=` (Rust keeps `where` as a keyword;
`db_where` is the developer's spelling from 2026-05-19, `r#where` the
raw-identifier spelling of Laravel's name, both kept: the developer,
2026-10-09 07:12); and a `db_where`, `filter` or `or_where` whose bound
value is null MUST compile to `IS NULL`, as Laravel turns a null
comparison into `whereNull`.
Falsifier: one of these helpers returns rows that differ from the equivalent raw SQL; a grouped helper lets an `or` escape its parentheses, so `where("a", 1).where_any(["b", "c"], "=", 2)` returns a row whose `a` is not 1; a subquery used by `where_in` loses its own bound values; `or_where("b", 2)` after `where("a", 1)` compiles with parentheses; `db_where("a", 1)`, `filter("a", 1)` or `r#where("a", 1)` compiles to anything but `a = 1`; or `where("deleted_at", Value::Null)` compiles to `= NULL` and matches nothing.
Mechanism: `par-query-helpers`.
Rationale: Issue #128 and row 006 of the parity log: Laravel's `Builder::where` with two arguments, its `whereNull` rewrite for a null value, and the flat `orWhere*` helpers.
Status: Agreed 2026-10-09

[PAR-007] `DB::after_commit` MUST run a callback after the `DB::transaction`
around it commits, at once when no `DB::transaction` is open, and never when
that transaction rolls back. A transaction opened with
`DB::begin_transaction` is a handle, not ambient state: the handle MUST
offer `tx.after_commit`, which runs a callback after `tx.commit()` and never
after a rollback, a drop without a commit, or a rollback to a savepoint taken
before the callback was registered.
Falsifier: a `DB::after_commit` callback runs inside a `DB::transaction` that rolls back, runs before that transaction commits, or does not run when no transaction is open; or a `tx.after_commit` callback runs before `tx.commit()`, or after the handle rolls back, is dropped uncommitted, or rolls back to a savepoint taken before the callback.
Mechanism: `par-after-commit`.
Rationale: Issue #128. Laravel `DatabaseManager::afterCommit` through `ManagesTransactions::afterCommit`. In Laravel `DB::beginTransaction()` puts the connection itself in the transaction, so `afterCommit` waits for it; a Suprnova manual transaction is a handle only the calls naming it use, so its after-commit work names it too. The developer accepted this reading on 2026-10-01 (decision 01M3W9VVCVG5NYE2SWJVWG0QN3).
Status: Agreed 2026-10-01

[PAR-008] `inertia_response!` MUST read an optional page lookup from the
application crate's `Cargo.toml`, under
`[package.metadata.suprnova.inertia]`: `pages_dir`, a directory relative
to the crate, and `page_file`, a file pattern in which `{dir}` is the
component name up to its last `/`, `{name}` the last segment, and a
`|lower`, `|kebab` or `|snake` filter may follow either. When a lookup is
set, the macro MUST accept exactly the components whose file exists at the
resolved path and name that path in its error. Without one, the lookup
MUST stay `frontend/src/pages/{Component}` with the extensions `svelte`,
`tsx`, `jsx` and `vue`.
Falsifier: with `pages_dir = "resources/angular/pages"` and `page_file = "{dir}/{name|lower}.page.ts"`, `Tramits/BaixaMatricula/Create` fails although `resources/angular/pages/Tramits/BaixaMatricula/create.page.ts` exists, or compiles although it does not; or, without the keys, a starter project's page lookup changes.
Mechanism: `par-inertia-pages`.
Rationale: Issue #129. Inertia leaves page resolution to the client adapter; the compile-time check is Suprnova's own, so the lookup must follow the application's layout.
Status: Agreed 2026-10-01

## Testing fakes

[PAR-009] The queue fake MUST offer `except`, which fakes every job but
the named job types and sends those to the real queue; MUST record raw
pushes and return them on request; and MUST offer
`assert_pushed_without_chain`, which passes only for a job pushed with no
chain and, given a closure, only for such a job the closure accepts, as
`assert_pushed_on` filters. A raw push whose payload is not a job
envelope MUST be refused.
Falsifier: under `except`, a named job is recorded instead of dispatched or another job is dispatched instead of recorded; a raw push cannot be read back; `assert_pushed_without_chain` passes for a job pushed with a chain; `assert_pushed_without_chain(|job| job.id == 7)` passes when only a job with another id was pushed without a chain, or fails when one with id 7 was; or a raw push of a string that is no envelope is recorded.
Mechanism: `par-queue-fake`.
Rationale: Laravel `QueueFake::except`, `pushRaw`, `rawPushes` and `assertPushedWithoutChain` with its callback (row 009 of the parity log); refusing a raw payload that is not an envelope fails closed and stays.
Status: Agreed 2026-10-08

## Test diagnostics

The developer ruled on 2026-10-01 that a test should see why a request it
sent failed, from that request alone, rather than through a process-wide
collection of logged errors, and made it the first priority after the
port requests.

[PAR-010] A response the framework builds from an error MUST carry that
request's error report in process: the error and its source chain, or,
when the panic boundary caught a panic, the panic message and its
location. With debug off, the report MUST NOT reach the response's
headers or body.
Falsifier: a handler or middleware that returns an error, or panics, yields a response without a report; a report holds an error from another request; or, with debug off, a report's text appears in the response's headers or body.
Mechanism: `par-test-diagnostics`.
Rationale: Laravel keeps the exceptions a test request logged in a process-wide `LoggedExceptionCollection` and `TestResponse` appends them to a failing assertion. A report carried by the response itself gives the same message without crossing between tests that run concurrently in one process. A development error page can render the same report.
Status: Agreed 2026-10-01

[PAR-011] `TestResponse` MUST build from a framework response, keeping its
error report, and every assertion that fails on a response carrying one
MUST include the report in its failure message. `TestResponse` MUST expose
the report for a test to inspect.
Falsifier: `assert_status(200)` or `assert_ok()` on a 500 that a handler error caused fails without that error's message; a panic boundary 500 fails without the panic message; or, with two requests in flight at once in one test process, a failure message names the other request's error.
Mechanism: `par-test-diagnostics`.
Rationale: Laravel `TestResponse::assertStatus` and the other status assertions append the request's exceptions to the failure. Building from the response also spares every test the body-collecting boilerplate `TestResponse::new` needs today.
Status: Agreed 2026-10-01

## Development error page

The developer ruled on 2026-10-01 to build a development error page,
second in priority after test diagnostics: the error chain, the useful
stack frames and the request context, with secrets redacted, working when
the frontend build is broken. A SQL-debugging dashboard is separate work.

[PAR-012] With debug on, a response with status 500 or above that carries
an error report (PAR-010), sent to an Inertia visit or to a request whose
`Accept` header lists `text/html`, MUST be replaced by the development
error page: an HTML document with the same status that shows the report's
error chain, or the panic message and location, the stack frames (PAR-013)
and the request context (PAR-014). For those responses the page MUST take
the place of the app's Inertia error page. With debug off, a response with
status 500 or above sent to a request whose `Accept` header lists
`text/html` and that is not an Inertia visit MUST be an HTML error view
with the same status and no detail of the error, as Laravel renders its
error view in production; an Inertia visit keeps the app's Inertia error
page, and a request that accepts JSON keeps its JSON body. Every other
response MUST stay as it is today.
Falsifier: with debug on, a browser request to a handler that returns an error gets JSON, or a page without the error's message or without the 500 status; an Inertia visit to it gets the app's Inertia error page; a request with `Accept: application/json` gets HTML; with debug off, a browser request to it gets a JSON body, a page that shows the error's message or a frame, or a status other than the error's; with debug off an Inertia visit gets anything but the app's Inertia error page, or an `Accept: application/json` request anything but its JSON body.
Mechanism: `par-debug-error-page`.
Rationale: Laravel renders its exception page when `APP_DEBUG` is on and the request does not expect JSON, and its minimal error view when debug is off (row 012 of the parity log); the Inertia documentation keeps the debug page in local development, where the client shows it in its modal.
Status: Agreed 2026-10-08

[PAR-013] With debug on, the framework MUST record the stack frames at the
place where an error first became a `FrameworkError` or an `AppError`, or
where a panic happened, and at the place each earlier error of the chain
was raised when that error carries its own frames, and the page MUST list
them: the application's frames shown, each with the source line and a few
lines around it when the source file is readable, and the frames of the
standard library, the async runtime, other dependencies and the framework
itself collapsed behind a count; each error of the chain MUST show its own
trace. With debug off, the framework MUST NOT record frames.
Falsifier: a handler whose `?` turns a database error into a `FrameworkError` yields a page whose frames do not name that handler, or whose application frame shows no source lines although the file is readable; a page for a chained error shows one trace for the whole chain where an inner error recorded its own; a panicking handler's page does not name the function that panicked; a `std`, `tokio` or `hyper` frame is shown outside a collapsed group; or, with debug off, an error records frames.
Mechanism: `par-debug-error-page`.
Rationale: Laravel's exception renderer shows a source snippet per frame and a trace per exception of the chain (row 013 of the parity log); argument types have no counterpart in a Rust backtrace.
Status: Agreed 2026-10-08

[PAR-014] The page MUST show the request's method, path and query, its
headers, the matched route pattern when there is one, and the request id.
It MUST redact the values of the `Authorization`, `Proxy-Authorization`,
`Cookie` and `Set-Cookie` headers; the value of any header or query
parameter whose name contains `token`, `secret`, `password`, `key` or
`signature`, in any letter case; and the password of any URL with
credentials in the error chain. It MUST NOT show the request body,
environment variables or configuration values.
Falsifier: a request carrying `Authorization: Bearer s3cr3t`, a session cookie, `X-Api-Key: s3cr3t` and `?token=s3cr3t` yields a page containing `s3cr3t` or the cookie's value; an error whose message holds `postgres://app:hunter2@db/app` yields a page containing `hunter2`; or the page lacks the method, the path or the request id.
Mechanism: `par-debug-error-page`.
Status: Agreed 2026-10-02

[PAR-015] The page MUST be one HTML document that requests nothing: no
script, stylesheet, font, image or frame from any URL, and no JavaScript.
It MUST render when the app has no frontend build or Vite manifest, and
when Inertia or the view layer is what failed. Every value it shows MUST
be HTML-escaped, and the response MUST carry `Cache-Control: no-store` and
a `Content-Security-Policy` that allows no script.
Falsifier: the page contains `<script`, `<link`, `<img`, `<iframe`, `@import` or `url(`; an error message holding `<script>alert(1)</script>` appears unescaped; an app with no frontend build or Vite manifest, or a request whose Inertia render itself fails, gets no page; or the response lacks `Cache-Control: no-store`, or a `Content-Security-Policy` that forbids script.
Mechanism: `par-debug-error-page`.
Status: Agreed 2026-10-02

## Default disk

The developer ruled on 2026-10-01 to build a default filesystem disk,
validated at startup, third in priority.

[PAR-016] The application's default disk MUST be the disk named by
`Storage::set_default_disk`, or else by `FILESYSTEM_DISK`, or else `local`,
as Laravel's `filesystems.default` reads `FILESYSTEM_DISK` with `local` as
its default, so a Laravel `.env` configures storage without edits; and
`Storage::default_disk()` MUST return the disk registered under that name.
With no `local` disk registered and neither setting present,
`Storage::default_disk()` MUST return an error that names `local` and
`FILESYSTEM_DISK`. A call that names its disk MUST behave as it does
today.
Falsifier: with `FILESYSTEM_DISK=uploads` and a disk registered as `uploads`, bytes written through `Storage::default_disk()` cannot be read through `Storage::disk("uploads")`; `Storage::set_default_disk("archive")` does not win over `FILESYSTEM_DISK`; with neither set and a `local` disk registered, `Storage::default_disk()` is not that disk; with neither set and no `local` disk, it returns a disk, or an error that names neither `local` nor `FILESYSTEM_DISK`; or `Storage::disk(name)` answers differently with a default set.
Mechanism: `par-default-disk`.
Rationale: Laravel's `Storage::disk()` with no name and `Storage::put` use `filesystems.default`, `env('FILESYSTEM_DISK', 'local')` (row 016 of the parity log).
Status: Agreed 2026-10-08

[PAR-017] When a default disk is named, startup MUST fail if no disk is
registered under that name once the application's bootstrap and the
environment's disks are in place: `filesystem::bootstrap_from_env`,
which the server and the worker commands run at boot, MUST return an
error that names the missing disk and `FILESYSTEM_DISK`. The environment's
`s3` disk MUST be configured by Laravel's names too: `AWS_ACCESS_KEY_ID`,
`AWS_SECRET_ACCESS_KEY`, `AWS_DEFAULT_REGION`, `AWS_BUCKET`, `AWS_URL`,
`AWS_ENDPOINT` and `AWS_USE_PATH_STYLE_ENDPOINT`, read when the `S3_`
names the framework reads today are absent, so a Laravel `.env`
configures the disk without edits.
Falsifier: with `FILESYSTEM_DISK=uploads` and no `uploads` disk registered, `bootstrap_from_env` returns `Ok`; its error does not name `uploads` or `FILESYSTEM_DISK`; with `FILESYSTEM_DISK=s3` and `S3_BUCKET` set, it fails although it registered the `s3` disk itself; or with `FILESYSTEM_DISK=s3`, `AWS_BUCKET`, `AWS_DEFAULT_REGION`, `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY` set and no `S3_` name, it fails to register the `s3` disk, or registers one that does not use those values.
Mechanism: `par-default-disk`.
Rationale: Laravel's `config/filesystems.php` `s3` disk reads the `AWS_*` names (row 017 of the parity log); the startup check is Suprnova's own design and stays.
Status: Agreed 2026-10-08

## SQS queue driver

The developer ruled on 2026-10-01 to build an SQS queue driver, a managed
queue outside the application database and Redis, after the default disk.

[PAR-018] With `QUEUE_DRIVER=sqs`, the framework MUST queue jobs on Amazon
SQS queues. A push MUST send the job to the queue its envelope
names, or to `SQS_QUEUE` when it names none, and a delayed job MUST NOT be
received before its time, a delay longer than the 15 minutes SQS allows on
one message included. A pop MUST receive one message and hide it for the
worker's visibility timeout. An acknowledgement MUST delete it, a `nack`
MUST return it after the requeue delay with one more attempt, and a
`release` MUST return it after the delay with the same number of attempts.
A worker MUST receive from the queues its `--queue` list names, in order,
and from `SQS_QUEUE` when the list is empty. `size`, `pending_size`,
`delayed_size`, `reserved_size` and `clear` MUST take the queue they
report or purge, `SQS_QUEUE` when none is given, so an application with
several queues counts and clears one; the sizes MUST report the
approximate counts SQS keeps and `clear` MUST return the count the queue
held. The same queue argument MUST exist on every queue driver's `size`
and `clear`.
Falsifier: against an SQS endpoint, a pushed job is never received; a job pushed to the queue `emails` is received from `SQS_QUEUE`; a job delayed 20 minutes is received before 20 minutes have passed; an acknowledged job is received again; a nacked job comes back with attempts not one higher, or a released job with its attempts changed; a worker with `--queue=emails` receives a job from `SQS_QUEUE`; `size` reports other than the counts SQS returns; `size("emails")` reports `SQS_QUEUE`'s count, or `clear("emails")` purges `SQS_QUEUE`; or another driver's `size` or `clear` refuses a queue name.
Mechanism: `par-sqs-queue`.
Rationale: Laravel's `SqsQueue::size($queue)` and `clear($queue)` take the queue (row 018 of the parity log); `SqsJob` counts every receive as an attempt, so its release counts one, and it passes SQS a delay over 900 seconds, which SQS refuses, so the attempt and delay rules stay.
Status: Agreed 2026-10-08

[PAR-019] The `sqs` driver MUST read its configuration from the
environment. `SQS_PREFIX`, `SQS_QUEUE` (default `default`) and
`SQS_SUFFIX` MUST build the queue URL as Laravel does, and a queue name
that is already a URL MUST be used as it is. `AWS_DEFAULT_REGION`, or else
`AWS_REGION`, names the region, and `SQS_ENDPOINT` points the driver at a
service that is not AWS. Requests MUST be signed with AWS Signature
Version 4 for `sqs` in that region, with `AWS_ACCESS_KEY_ID`,
`AWS_SECRET_ACCESS_KEY` and `AWS_SESSION_TOKEN` when they are set, or else
with AWS's default credential chain. A queue whose name ends in `.fifo`
MUST be sent FIFO messages, as Laravel's `getQueueableOptions` builds them:
`MessageGroupId` from the job's message group, `default` when the job sets
none, and `MessageDeduplicationId` from the job's deduplication id, a
digest of the payload when the job sets none, with no per-message delay,
which FIFO queues refuse. Boot MUST fail with an error that names the
variable when no region is set or when the queue is not a URL and
`SQS_PREFIX` is not set. `QUEUE_CONNECTIONS` and
`QUEUE_FAILOVER_CONNECTIONS` MUST accept `sqs`.
Falsifier: with `SQS_PREFIX=https://sqs.us-east-1.amazonaws.com/123456789012`, `SQS_QUEUE=jobs` and `SQS_SUFFIX=-prod`, a push names a queue URL other than `https://sqs.us-east-1.amazonaws.com/123456789012/jobs-prod`; a request carries no Signature Version 4 `Authorization` header scoped to the region and `sqs`; boot succeeds with no region, or with a plain queue name and no prefix; boot refuses `SQS_QUEUE=jobs.fifo`; a push to `jobs.fifo` carries no `MessageGroupId` or no `MessageDeduplicationId`, carries a group other than the job's, or carries a `DelaySeconds`; or `QUEUE_CONNECTIONS=sqs` is refused.
Mechanism: `par-sqs-queue`.
Rationale: Laravel's `SqsQueue::getQueueableOptions` sends `MessageGroupId` and `MessageDeduplicationId` for a `.fifo` queue (row 019 of the parity log); the region and prefix boot failures fail closed on missing configuration and stay.
Status: Agreed 2026-10-08

[PAR-020] With `SQS_OVERFLOW_ENABLED=true`, a job whose payload is 1 MiB
or more, SQS's message limit, or every job when `SQS_OVERFLOW_ALWAYS=true`,
MUST be stored outside the message and sent to SQS as a pointer: in the
cache store `SQS_OVERFLOW_STORE` names, as Laravel keeps them, or else on
the filesystem disk `SQS_OVERFLOW_DISK` names, or else on the default
disk. A pop MUST return the stored job. An acknowledgement MUST delete the
stored payload unless `SQS_OVERFLOW_DELETE_AFTER_PROCESSING=false`, and
`clear` MUST delete the stored payloads of the cleared queue, and only
those, when `SQS_OVERFLOW_FLUSH_ON_CLEAR=true`. Without overflow, a push
over the limit MUST fail with an error that names the limit and
`SQS_OVERFLOW_ENABLED`. Boot MUST fail when overflow is on and the named
store or disk is not registered.
Falsifier: with overflow on, a 2 MiB job is sent to SQS whole, or a pop returns the pointer instead of the job; with `SQS_OVERFLOW_STORE=redis` the payload is not in that cache store, or is on a disk; its stored payload survives an acknowledgement with delete-after-processing on, or survives `clear` of its queue with flush-on-clear on, or `clear` of another queue deletes it; with overflow off, a 2 MiB push succeeds or its error names neither the limit nor `SQS_OVERFLOW_ENABLED`; or boot succeeds with overflow on and no store or disk registered under the name.
Mechanism: `par-sqs-queue`.
Rationale: Laravel's `SqsQueue::overflow` stores the payload in the cache store `queue.connections.sqs.overflow.store` names (row 020 of the parity log); clearing only the cleared queue's payloads stays, since Laravel's flush of the whole store destroys other queues' payloads.
Status: Agreed 2026-10-08

## Process facade

The developer ruled on 2026-10-01 to build a focused Process facade:
argument-based execution, captured output, timeouts, cancellation and
cleanup, bounded concurrency, and fakes that stop real execution in tests,
with pipes and idle timeouts as requirements of their own. On 2026-10-02
the developer reversed the argument-based part, so a command line run
through the shell is built as well.

[PAR-021] `Process::command(args)` MUST run the program `args[0]` with the
remaining arguments passed to it as they are, through no shell, and
`Process::shell(line)` MUST run `line` through the system shell (`sh -c`,
or `cmd /C` on Windows), as Laravel runs a string command. `run` MUST
return a result with the exit code and the full standard output and
standard error, and a nonzero exit MUST be a result that reports failure,
not an error; `throw` MUST turn a failed result into an error that carries
the exit code and both outputs. `path` MUST set the working directory,
`env` MUST add a variable to the environment the process inherits and
MUST remove an inherited variable when given no value for it, `input`
MUST be written to its standard input, and an output callback MUST receive
each chunk of standard output and standard error as it arrives unless
`quietly` is set. `tty` MUST hand the process the terminal's standard
input and output, capturing nothing, and MUST be an error that names the
redirected stream when standard input or output is not a terminal. The
command line a result or an error reports MUST quote each argument as a
POSIX shell would read it. A program that cannot be started MUST be an
error that names it.
Falsifier: running `sh -c 'printf out; printf err >&2; exit 3'` does not give the output `out`, the error output `err`, exit code 3 and a failed result, or `run` returns an error for it; an argument to `Process::command` holding `; touch marker` is run by a shell, so `marker` exists; `Process::shell("printf 'b\na\n' | sort")` does not output `a\nb\n`; `path`, `env` or `input` does not reach the process; `env("HOME", None)` leaves `HOME` in the process's environment; the callback misses a chunk, or is called under `quietly`; output is captured under `tty`, or `tty` with standard input redirected from a file runs instead of erroring; the reported command line of `["printf", "a b"]` is not `printf 'a b'`; or a missing program gives a result, or an error that does not name it.
Mechanism: `par-process`.
Rationale: Laravel's `PendingProcess::env` with a `false` value unsets a variable, Symfony's `Process::setTty` refuses a non-terminal, and `getCommandLine` escapes each argument (row 021 of the parity log); `shell` and `command` are kept apart so the shell is never reached by accident, and `quietly` dropping the callback stays.
Status: Agreed 2026-10-08

[PAR-022] A process MUST be killed, with every process it started, when
it runs past its `timeout` (60 seconds unless set, none after `forever`),
and `run` MUST then return a timeout error that names the command and the
timeout. `start` MUST return a running process with its id, whether it is
still running, the output so far and since the last read, a way to send it
a signal, `stop` and `wait`, which returns its result. `stop(grace,
signal)` MUST send the given signal, a terminate signal when none is
given, wait up to `grace` (10 seconds unless set) for the process to exit,
and then kill it, as Laravel's `InvokedProcess::stop` does through
Symfony's `Process::stop`. Dropping a running process, or the future of
`run` before it completes, MUST kill it with every process it started.
Falsifier: `sh -c 'sleep 30 & sleep 30'` with a one-second timeout does not return a timeout error naming the command within five seconds, or either `sleep` is still running afterwards; a started process reports no id, reports running after it exited, or `stop` leaves it running; `stop` on a process that ignores the terminate signal returns before ten seconds without killing it, or `stop(Duration::from_secs(1), Signal::Interrupt)` sends anything but an interrupt first; or after the `run` future or a started process is dropped, the process or a child of it is still running.
Mechanism: `par-process`.
Rationale: Laravel's `InvokedProcess::stop($timeout = 10, $signal = null)` and Symfony's `Process::stop` (row 022 of the parity log); killing every descendant and the watchdog stay, since an orphaned process is the unsafe outcome.
Status: Agreed 2026-10-08

[PAR-023] `idle_timeout` MUST kill a process, with every process it
started, when it writes no output for that long, and `run` MUST then
return an idle timeout error that names the command; a process that keeps
writing MUST run on past the idle timeout.
Falsifier: `sleep 30` with a one-second idle timeout does not return an idle timeout error within five seconds; or a process that prints every 200 milliseconds for three seconds is killed by a one-second idle timeout.
Mechanism: `par-process`.
Status: Agreed 2026-10-02

[PAR-024] `Process::pool()` MUST run the processes added to it, each under
the key it was added with or its position, and return every result under
its key in the order added, a failure of one not stopping the others.
With `concurrency(n)`, at most `n` MUST run at once. `Process::pipe()`
MUST run its processes in order, each with the previous one's output as
its input, and return the last result, or the first failed result without
running the rest.
Falsifier: six processes that each run one second, in a pool with concurrency 2, ever run more than two at once or finish in under three seconds; a result is missing or under another key; a failing process stops the others; or a pipe of `printf 'b\na\n'` and `sort` does not return `a\nb\n`, or a pipe whose first process fails runs the second.
Mechanism: `par-process`.
Status: Agreed 2026-10-02

[PAR-025] `Process::fake()` MUST stop every process from running while its
guard lives: a command matching a faked pattern (`*` matches any run of
characters in the command line: the arguments joined by spaces, or the
shell line as given) MUST get the faked result, its output, error output
and exit code, and any other command an empty successful one, unless
`prevent_stray_processes` makes it an error that names the command. A
fake MAY be a closure that receives the pending process (its command,
path, environment and input) and returns the result, as Laravel's closure
handlers do. A described fake MUST support a run that lasts a given number
of `running` checks and MUST replay its output and error output lines in
the order they were described, each line ending in a newline as Laravel's
`FakeProcessDescription` writes it; a sequence MUST answer its results in
turn. Every faked run, start, pool and pipe MUST be recorded, and
`assert_ran`, `assert_ran_times` (once when no count is given),
`assert_ran_in_order`, `assert_not_ran` and `assert_nothing_ran` MUST fail
the test when the record does not match.
Falsifier: with a fake installed, a command that creates a file creates it; a matching command gets another result; an unmatched command errors without `prevent_stray_processes` or runs with it; a closure fake does not receive the command's arguments, path, environment or input, or its result is not the one returned; a sequence answers out of turn; a described process stops reporting running before its count, replays its error output before an output line described earlier, or writes a line without its trailing newline; `assert_ran_times("ls")` with no count passes when `ls` ran twice; or an assertion passes on a record that does not match it.
Mechanism: `par-process`.
Rationale: Laravel's `Process::fake` with closure and pattern handlers, `describe`, `sequence`, `preventStrayProcesses` and the assertions (rows 025 and 025.2 of the parity log); the fake answering every command stays.
Status: Agreed 2026-10-08

## Log channels

The developer ruled on 2026-10-01 to build log channels: files, rotation,
retention, flushing on shutdown and several outputs at once, with stdout
the default. Slack and the other vendor sinks are deferred, not refused.

[PAR-026] `LOG_CHANNEL` MUST name the channel the application's log
events go to, and `stdout` when it is not set, writing what the framework
writes today. The built-in channels are `stdout`, `stderr` (also named
`errorlog`), `single`, `daily`, `monthly`, `syslog`, `null`, `stack`,
which writes to every channel `LOG_STACK` lists (`single` when it is not
set), and `custom`, a channel whose `LOG_CHANNEL_DRIVER` names a driver
`Log::extend` added, so every name Laravel's `config/logging.php` gives
resolves from a Laravel `.env` without edits. `Log::define(name, channel)`
in the bootstrap MUST add a channel under a name, and `Log::extend(driver,
factory)` MUST add a driver a defined or the `custom` channel can use. A
`LOG_CHANNEL` or `LOG_STACK` that names no channel MUST fail boot with an
error that names it.
Falsifier: with `LOG_CHANNEL` unset, an `info!` event is not written to stdout; with `LOG_CHANNEL=single` the event is not in the file, or is also on stdout; `LOG_CHANNEL=errorlog`, `LOG_CHANNEL=null` or `LOG_CHANNEL=stack` with `LOG_STACK=single,stderr` fails boot or writes elsewhere than Laravel's channel of that name would; `LOG_CHANNEL=custom` with `LOG_CHANNEL_DRIVER` naming an extended driver does not reach that driver; a defined channel, or one on an extended driver, does not receive the events of the default channel it is; or boot succeeds with `LOG_CHANNEL=nosuch`, or its error does not name `nosuch`.
Mechanism: `par-log-channels`.
Rationale: Laravel's `config/logging.php` channels, `LOG_CHANNEL` and the `custom` driver's `via` (row 026 of the parity log), with stdout as the default the developer kept and the boot failure on an unknown channel kept.
Status: Agreed 2026-10-08

[PAR-027] `single` MUST append each record to one file, `logs/suprnova.log`
under the storage directory unless the channel sets a path, creating the
directories. `daily` MUST write to a file named for the day
(`suprnova-2026-10-02.log`) and keep the newest `LOG_DAILY_DAYS` files,
7 when it is not set as Laravel's `createDailyDriver` defaults, deleting
older ones; `monthly` MUST write to a file named for the month
(`suprnova-2026-10.log`) and keep the newest 3. The day and the month
are those of the framework clock, in UTC.
Falsifier: a record is not appended to the single file, or its directory is not created; a record written after midnight goes to the previous day's file; with 20 dated files and `LOG_DAILY_DAYS=14`, other than the 14 newest remain after a write; with 10 dated files and `LOG_DAILY_DAYS` unset, other than the 7 newest remain; or a monthly channel keeps other than the 3 newest months.
Mechanism: `par-log-channels`.
Rationale: Laravel's `LogManager::createDailyDriver` keeps `$config['days'] ?? 7` files (row 027 of the parity log); the `suprnova.log` name and the UTC clock are the earlier spec's choices and stay.
Status: Agreed 2026-10-08

[PAR-028] A `stack` MUST write each record to every channel it lists, and
a channel that fails, such as a file it cannot open, MUST NOT stop the
others. `Log::channel(name)` MUST return a logger that writes to that
channel only, `Log::stack(names)` one that writes to each, and
`Log::build(channel)` one for a channel that has no name, each with the
eight levels from `emergency` to `debug` and a context. A `{key}` in a
message MUST be replaced with the context's value under `key`.
`Log::channels`, `Log::forget_channel`, `Log::default_channel` and
`Log::set_default_channel` MUST report and change the channels in use and
the default. `MAIL_LOG_CHANNEL` MUST name the channel the `log` mail
transport writes to.
Falsifier: a stacked channel misses a record, or an unwritable one stops another; `Log::channel("daily")` writes to the default channel too, or a stack or a built channel misses a record; `"user {id}"` with the context `id = 7` is not written as `user 7`; `Log::set_default_channel` does not move the events that follow; or with `MAIL_LOG_CHANNEL` set a logged mail is not written to that channel.
Mechanism: `par-log-channels`.
Rationale: Laravel's `Log::channel`, `Log::stack`, `Log::build`, `LogManager` and `replace_placeholders`.
Status: Agreed 2026-10-02

[PAR-029] File channels MAY buffer, but MUST flush: when `Log::flush` is
called, at once for a record at `error` or above, within one second of a
write otherwise, and when the server or a worker shuts down, so that no
record written before a clean exit is lost.
Falsifier: after `Log::flush`, a record written before it is not in the file; an `error` record is not in the file as soon as the call that wrote it returns; an `info` record is not in the file two seconds after it was written; or a record written before a clean shutdown is not in the file after it.
Mechanism: `par-log-channels`.
Status: Agreed 2026-10-02

[PAR-030] `syslog` MUST send each record as an RFC 3164 datagram to the
local syslog socket (`LOG_SYSLOG_SOCKET`, `/dev/log` unless set, or
`/var/run/syslog` on macOS), with the facility `LOG_SYSLOG_FACILITY`
(`user` unless set), the severity of the record's level, and the
application's name from `APP_NAME` as the ident (`suprnova` when it is
not set), as Laravel's syslog channel names the application. Off Unix it
MUST fail boot with an error that says so, and an unknown facility MUST
fail boot too.
Falsifier: a record reaches the socket with another priority than facility times 8 plus severity, or does not reach it; with `APP_NAME=Shop` the datagram's tag is not `Shop`; or an unknown facility boots.
Mechanism: `par-log-channels`.
Rationale: Laravel's `createSyslogDriver` passes `config('app.name')` as the ident (row 030 of the parity log); the unknown-facility boot failure and the Unix-only refusal stay.
Status: Agreed 2026-10-08

## Redis facade

The developer ruled on 2026-10-01 to build a thin managed Redis API:
configuration, connection lifecycle, commands, pipelines and an escape
hatch to the client, with dedicated connections for subscriptions and
blocking commands. Redis Cluster is deferred, not refused. The funnel and
throttle limiters stay unbuilt by the developer's ruling of 2026-09-30.

[PAR-031] `Redis::connection(name)` MUST return the connection with that
name: `default`, which reaches `REDIS_URL` when it is set, or else the
server `REDIS_HOST` (`127.0.0.1`), `REDIS_PORT` (`6379`), `REDIS_PASSWORD`
(none) and `REDIS_DB` (`0`) describe, as Laravel's `config/database.php`
reads them; `cache`, the same server on `REDIS_CACHE_DB` (`1`); or one
the bootstrap gives with `Redis::define(name, url)`, which replaces any
connection of that name. Every key a typed method sends MUST carry the
prefix `REDIS_PREFIX` names, `<app name as a slug>-database-` when it is
not set, as Laravel's `redis.options.prefix`; `command(name, args)` sends
its arguments as given. A connection MUST open on its first command, not
before, and MUST open again after it is lost. `Redis::purge(name)` MUST
forget the connection, which closes once no handle holds it, and
`Redis::connections()` MUST list the names of the connections resolved and
not purged. A name with no connection, or a URL that is not a Redis URL,
MUST be an error that names the connection.
Falsifier: the default connection reaches another server or database than `REDIS_URL` names, or, with `REDIS_URL` unset, than `REDIS_HOST`, `REDIS_PORT`, `REDIS_PASSWORD` and `REDIS_DB` describe; `Redis::connection("cache")` reaches another database than `REDIS_CACHE_DB`; with `REDIS_PREFIX=shop-` a `set("k", v)` writes a key other than `shop-k`, or with it unset and `APP_NAME=My Shop` a key other than `my-shop-database-k`; a defined connection reaches another database than its URL's; resolving a connection to an address where nothing listens fails before a command is sent; after the server drops the connection, a read, or the write after it, fails although the server is up; `connections()` misses a resolved name or lists a purged one; a purged connection that no handle holds stays open; or an unknown name, a URL such as `http://x`, or such a `REDIS_URL`, is not an error naming the connection.
Mechanism: `par-redis`.
Rationale: Laravel's `RedisManager` and the `redis` connections and `options.prefix` of `config/database.php` (row 031 of the parity log); connecting on the first command and rejecting unknown URL schemes stay.
Status: Agreed 2026-10-08

[PAR-032] A connection MUST run the common commands as typed methods
(`get`, `set`, `set_ex`, `del`, `exists`, `incr`, `decr`, `expire`, `ttl`,
`mget`, `hset`, `hget`, `hgetall`, `hdel`, `lpush`, `rpush`, `lpop`,
`rpop`, `lrange`, `sadd`, `srem`, `smembers`, `zadd`, `zrange`,
`zrangebyscore`, `publish`, `eval`, `evalsha` and `scan`), any other
command with `command(name, args)`, which returns the reply, a raw command
line with `execute_raw(args)`, as Laravel's `executeRaw`, and give the
underlying `redis` client with `client()`. A read, typed or one of
Laravel's retryable commands given to `command`, MUST be sent again after
a lost connection: once, and once more for each retry
`REDIS_COMMAND_RETRIES` adds; another command MUST NOT be, and `MULTI` and
`SUBSCRIBE` on the shared connection MUST be refused. While
`Redis::enable_events()` is in force, each command a connection runs
outside a pipeline or a transaction MUST be dispatched as a
`RedisCommandExecuted` event through the application's event dispatcher,
with the connection's name, the command, its arguments and its duration,
and each command that fails as a `RedisCommandFailed` event with its
error, so `Event::listen` hears them; `Redis::listen` and
`Redis::listen_for_failures` MUST keep receiving them too. Events are off
until enabled.
Falsifier: a typed command, `evalsha` with a loaded script, `execute_raw(["LRANGE", ...])` or `command("LRANGE", ...)` returns other than the server's reply; `client()` is not a client of the same server and database; a read fails after the server dropped the connection once; a write is applied twice after a lost connection; `MULTI` through `command` on the shared connection is sent; with events enabled a listener registered with `Event::listen` for `RedisCommandExecuted` does not hear a command, or hears the wrong connection name, command or arguments, a failed command reaches no `RedisCommandFailed` listener, or a `Redis::listen` listener stops hearing commands; or a command is reported while events are off.
Mechanism: `par-redis`.
Rationale: Laravel's `Connection::command`, `executeRaw`, `client`, `listen`, `listenForFailures`, the `CommandExecuted` and `CommandFailed` events it dispatches through the application dispatcher, and `PhpRedisConnection::RETRYABLE_COMMANDS` (row 032 of the parity log); never resending writes and refusing `MULTI` or `SUBSCRIBE` on the shared connection stay, since a resent `INCR` double-applies.
Status: Agreed 2026-10-08

[PAR-033] `pipeline(|pipe| ...)` MUST send every command the closure
queues before it reads a reply, and return the replies in order.
`transaction(|pipe| ...)` MUST send them inside `MULTI` and `EXEC` and
return their replies in order, so that when Redis rejects one of them as
it is queued, none is applied.
Falsifier: a pipeline waits for a reply before it sends its next command, or returns its replies out of order; or a transaction holding a command Redis rejects when it is queued applies any of the others, or returns its replies out of order.
Mechanism: `par-redis`.
Status: Agreed 2026-10-03

[PAR-034] `subscribe(channels)` and `psubscribe(patterns)` MUST open a
connection of their own and yield each message, with its channel, the
pattern it matched if any, and its payload, until the subscription is
dropped, which closes that connection. A blocking command (`blpop`,
`brpop`, `blmove`, `brpoplpush`, `bzpopmin`, `bzpopmax`) MUST run on a
connection of its own and wait as long as its timeout says, so a call
that waits never delays another command and is never cut short.
Falsifier: a published message is not yielded, or is yielded with the wrong channel, pattern or payload; a command on the connection waits while a subscription is open; a dropped subscription stays subscribed; a `get` waits behind a `blpop` on the same connection; or a `blpop` with a two-second timeout returns before the two seconds pass when nothing is pushed, or misses an element pushed while it waits.
Mechanism: `par-redis`.
Status: Agreed 2026-10-03

## Str and Number subset

The developer ruled on 2026-10-01 to build a useful subset of `Str` and
`Number`, not the whole `Stringable`: slug, mask, excerpt and limit;
pluralization that is language-specific; percentage and abbreviate
through `Lang`.

[PAR-035] `Str::slug(title, separator)` MUST spell the title in ASCII,
lower-case it, write `@` as `at`, and join its runs of letters and digits
with the separator, as Laravel's `Str::slug` does, and `Str::slug_in(title,
separator, language)` MUST transliterate by the named language's rules as
Laravel's third argument does. `Str::mask(value, character, index,
length)` MUST replace the characters from `index` (counted from the end
when negative) for `length` characters (to the end when `None`) with
`character`. `Str::limit(value, limit, end)` MUST keep the first `limit`
characters and append `end` when it cut. `Str::words(value, words, end)`
MUST keep the first `words` words, a word being a run of non-space
characters however it is spelled, and append `end` when it cut, as
Laravel's `Str::words` does, and `Str::limit_words(value, limit, end)` MUST
cut at the last space within the limit. `Str::excerpt(text, phrase, radius,
omission)` MUST return the first match of the phrase, case-insensitive,
with up to `radius` characters on each side and `omission` where it cut,
trimming Laravel's invisible characters (the `INVISIBLE_CHARACTERS` set)
from the ends it cut, or `None` when the phrase is absent. Every count is
in characters, not bytes.
Falsifier: `Str::slug("Laravel 5 Framework", "-")` is not `laravel-5-framework`, `Str::slug("Œuvre d'art_2 @home", "-")` is not `oeuvre-dart-2-at-home`, or `Str::slug("foo bar", "_")` is not `foo_bar`; `Str::slug_in("Ärger", "-", "de")` is not `aerger`; `Str::mask("taylor@example.com", '*', 3, None)` is not `tay***************`, or `Str::mask("taylor@example.com", '*', -15, Some(3))` is not `tay***@example.com`; `Str::limit("The quick brown fox jumps over the lazy dog", 20, "...")` is not `The quick brown fox...`; `Str::words("Perfectly balanced, as all things should be.", 3, " >>>")` is not `Perfectly balanced, as >>>`, or `Str::words("<b>bold</b> text here", 2, "...")` keeps other than the first two space-separated runs; `Str::limit_words("The quick brown fox", 12, "...")` is not `The quick...`; `Str::excerpt("This is my name", "my", 3, "...")` is not `Some("...is my na...")`, or an excerpt keeps a zero-width space at a cut end; or a multibyte value is cut inside a character.
Mechanism: `par-strings`.
Rationale: Laravel's `Str::slug` with its language argument, `Str::words`, `Str::mask`, `Str::limit` and `Str::excerpt` (row 035 of the parity log); counting characters in `limit` stays.
Status: Agreed 2026-10-08

[PAR-036] `Str::plural(word, count)` and `Str::singular(word)` MUST
inflect the word by the rules of the language of the current `Lang`
locale: English, Esperanto, French, Italian, Norwegian Bokmål, Portuguese,
Spanish or Turkish, as Laravel's `Pluralizer` does with doctrine/inflector,
and by the English rules for any other language. A count of 1 or -1 MUST
leave the word as it is, and the result MUST keep the word's case: lower,
upper, first letter capital, or each word capital. `Str::plural_studly`
and `Str::plural_pascal` MUST inflect the last word of a studly-cased
value, and a count given with `prepend_count` MUST be written before the
word, as Laravel's `Str::plural($value, $count, prependCount: true)`.
Falsifier: in English `car` is not `cars`, `child` is not `children`, `person` is not `people`, `sheep` is not `sheep`, `Car` is not `Cars`, `CAR` is not `CARS`, `cars` with a count of 1 is not `cars`, or `Str::singular("people")` is not `person`; in French `cheval` is not `chevaux`; in Spanish `ciudad` is not `ciudades`; in Portuguese `cão` is not `cães`; in Norwegian Bokmål `bil` is not `biler`; in Turkish `kitap` is not `kitaplar`; in Italian `gatto` is not `gatti`; in Esperanto `hundo` is not `hundoj`; `Str::plural_studly("VerifiedHuman", 2)` is not `VerifiedHumans`; `Str::plural("car", 3)` with `prepend_count` is not `3 cars`; or under a locale with none of these languages `car` is not `cars`.
Mechanism: `par-strings`.
Rationale: Laravel's `Str::plural`, `Str::singular`, `Str::pluralStudly`, `Str::pluralPascal` and `Pluralizer::useLanguage` with the eight languages doctrine/inflector ships (row 036 of the parity log); Suprnova takes the language from the request's locale instead of a process-wide setting.
Status: Agreed 2026-10-08

[PAR-037] `Lang::percentage(value, precision)` MUST format `value` as a
percentage, `10` as ten percent, with `precision` fraction digits rounded
half up on the decimal value as Laravel's `Number::percentage` rounds, the
way the current locale writes one; `Lang::percentage_in(value, precision,
locale)` MUST do the same in the named locale. `Lang::format(value,
precision, max_precision)` and `Lang::percentage` with a `max_precision`
MUST write at most that many fraction digits, dropping trailing zeros, as
Laravel's `maxPrecision`. `Lang::use_locale(locale)` MUST set the locale
these functions use when no request locale is in force, and
`Lang::with_locale(locale, closure)` MUST run the closure under that
locale and restore the previous one. An infinite value MUST be written as
`∞` and a value that is not a number as `NaN`, as Laravel writes them.
`Lang::abbreviate(value, precision)` MUST divide the value by the largest
of a thousand, a million, a billion, a trillion and a quadrillion that it
reaches, write it with `precision` fraction digits in the current locale's
number format, and append `K`, `M`, `B`, `T` or `Q`, as Laravel's
`Number::abbreviate` does; a value under a thousand is written as it is.
Falsifier: in `en` `Lang::percentage(10.0, 0)` is not `10%`, `Lang::percentage(12.345, 1)` is not `12.3%`, or `Lang::percentage(0.12345, 4)` is not `0.1235%`; `Lang::percentage_in(10.0, 0, "de")` is not `10 %` with the locale's space; `Lang::format(1.2300, 2, Some(4))` writes more than two fraction digits or keeps a trailing zero past the first, or `Lang::percentage(12.5, 0)` under `max_precision` 2 is not `12.5%`; after `Lang::use_locale("de")` with no request locale `Lang::percentage(10.0, 0)` is not `10 %`, or `Lang::with_locale("fr", ..)` leaves `fr` in force after it returns; `Lang::format(f64::INFINITY, 0)` is not `∞` or `Lang::format(f64::NAN, 0)` is not `NaN`; in `en` `Lang::abbreviate(1000.0, 0)` is not `1K`, `Lang::abbreviate(489939.0, 0)` is not `490K`, `Lang::abbreviate(1230000.0, 2)` is not `1.23M`, `Lang::abbreviate(-2500.0, 1)` is not `-2.5K`, or `Lang::abbreviate(999.0, 0)` is not `999`; or in `de` `Lang::abbreviate(1230000.0, 2)` is not `1,23M`.
Mechanism: `par-strings`.
Rationale: Laravel's `Number::percentage` with its locale argument and `maxPrecision`, `Number::useLocale`, `Number::withLocale`, its `∞` and `NaN` output and its own test expecting `0.1235%` for `percentage(0.12345, 4)` (rows 037 and 037.1 of the parity log); the Lang locale as the default stays, and the compact suffixes are Laravel's because ICU4X's compact format is not in the release the framework uses.
Status: Agreed 2026-10-08

## Schema dump

The developer ruled on 2026-10-01 to build `schema:dump`, at lower
priority: it writes a schema snapshot and its migration baseline, loads
it before newer migrations, prunes only when asked, and is tested on each
supported database. Laravel's `SchemaDumped`, `SchemaLoaded` and
`MigrationsPruned` events are not built: the migration commands run
before the bootstrap registers any listener, so none could hear them.

[PAR-038] `schema:dump` on the app binary MUST write the schema of the
`DATABASE_URL` database, or of the connection `--database` names, to
`database/schema/<engine>-schema.sql`, where the engine is `sqlite`,
`postgres`, `mysql` or `mariadb`, or to the file `--path` names: the
statements that create every table, index, view and constraint the
database holds, without any table's rows, followed by one `INSERT` for
each row of the Migrator's ledger table (`seaql_migrations` unless the
Migrator names another), which `--without-migration-data` MUST leave out.
Postgres MUST be dumped with `pg_dump`, MySQL with `mysqldump` and MariaDB
with `mariadb-dump`, each given the password through its environment or a
file only the current user can read, never as an argument; SQLite MUST be
read through its own connection. When the tool is missing or fails, the
command MUST exit with an error that names the tool, and an earlier dump
file MUST stay as it was. The developer CLI's `suprnova schema:dump` MUST
run the app binary's.
Falsifier: on any of SQLite, Postgres, MySQL and MariaDB, after `migrate` the dump file is absent, holds a row of a table other than the ledger, lacks a table or index the migrations created, or lacks an applied migration's ledger row; with `--without-migration-data` the file holds a ledger row; with `--database reporting` the dump is of another connection than `reporting`, or an unknown name does not fail with an error naming it; the password appears in the dump tool's arguments; or with the tool missing the command exits zero or changes an earlier dump file.
Mechanism: `par-schema-dump`.
Rationale: Laravel's `DumpCommand` takes `--database` and `--without-migration-data`, which turns off `SchemaState::withMigrationTable` (row 038 of the parity log); the ledger is written by Suprnova, the same `INSERT` statements on every engine, and its agreed choices, events included, stay.
Status: Agreed 2026-10-08

[PAR-039] When the migration ledger records no migration and a dump file
exists for the engine, or `--schema-path` names one, `migrate`,
`migrate:fresh` and the migration `serve` runs MUST load the file into
the database before any migration runs, and then run only the
migrations the loaded ledger does not record. `TestDatabase::fresh` MUST
load `database/schema/sqlite-schema.sql` the same way. Postgres MUST be
loaded with `psql`, MySQL with `mysql`, MariaDB with `mariadb`, and
SQLite through its own connection. A failed load MUST stop the command
with an error before any migration runs. A database whose ledger records
a migration MUST NOT be loaded.
Falsifier: on any of SQLite, Postgres, MySQL and MariaDB, an empty database migrated with a dump of the first migrations and one newer migration lacks a table from the dump, runs one of the dumped migrations again, or lacks the newer migration's table; a database with an applied migration is loaded; or a failing load lets a migration run.
Mechanism: `par-schema-dump`.
Rationale: Laravel's `MigrateCommand::loadSchemaState`, which loads only when no migration has run.
Status: Agreed 2026-10-03

[PAR-040] `schema:dump --prune`, on the app binary and through the
developer CLI, MUST, after the dump succeeds, delete the file of each
migration in `src/migrations/` whose name the dump's ledger records,
remove its `mod` line from `mod.rs`, and replace its entry in the
Migrator's list with `PrunedMigration::new("<name>")`, which keeps the
name without the code. A migration the ledger does not record MUST stay.
Without `--prune`, and when the dump fails, no file changes. A
`PrunedMigration` that has to run, up or down, MUST fail with an error
that names it and `database/schema`, so a database never skips a pruned
migration's schema; a database whose ledger records it migrates as
before.
Falsifier: after `--prune`, an applied migration's file remains, a pending migration's file is gone, or `mod.rs` still declares a deleted module; `migrate` fails against a database that applied the pruned migrations; `migrate` succeeds on an empty database with no dump file; or a failed dump pruned a file.
Mechanism: `par-schema-dump`.
Rationale: Laravel's `--prune` deletes every migration file, applied or not; Suprnova keeps the ones the dump does not cover, and keeps each pruned name, because SeaORM refuses a ledger row whose migration is not in the list.
Status: Agreed 2026-10-03

## Requests from an application port, second round

The same application team filed issues #137, #139 and #140 for the next
gaps it met. The developer asked for them to be done on 2026-10-04 ("let's
knock out the issues").

[PAR-041] `OAuthIdentity`, which `verify_oauth_identity` returns, MUST
carry `avatar_url`: the URL of the account's picture as the provider
reports it, or none, documented as untrusted profile data that an
application checks (scheme, length) before it renders, fetches or stores
it. A provider supplies it through an `OAuthProvider` method,
`avatar_url`, which reads the same profile response `resolve_identity`
reads and returns none by default, so a provider written before it keeps
compiling unchanged and reports none; `ProviderIdentity` MUST NOT change.
Google MUST take it from the userinfo `picture` claim, TikTok from
`avatar_url`, Facebook from `picture.data.url`, with its profile request
naming the fields `id`, `name`, `email` and `picture`, and X from
`profile_image_url` with that among the user fields it requests; Apple
reports none. A profile without a picture, or with an empty one, MUST
give none, and sign-in MUST proceed as before. The manual's provider
chapter MUST show a provider adding the method, and the GitHub provider
crate MUST ship a release that implements it from GitHub's `avatar_url`.
Falsifier: a Google, TikTok, Facebook or X callback whose profile carries a picture returns no `avatar_url`, or a value other than the provider's, in `OAuthIdentity`; a profile without one, or with an empty string, fails sign-in or returns a value; the Facebook profile request does not name `email` and `picture`, or the X one `profile_image_url`; a provider that does not implement `avatar_url` fails to compile or fails sign-in; or `ProviderIdentity` gains a field.
Mechanism: `par-oauth-avatar`.
Rationale: Socialite's `getAvatar()`; issue #140 asked for Google, and every provider that reports a picture fills it. The developer ruled on 2026-10-04 that it must not break existing providers, so it is a defaulted trait method, not a new identity field. The Facebook plugin's documentation names `/me?fields=id,name,email`, but it requests a bare `/me`, for which the Graph API returns only `id` and `name`, so Facebook sign-in never received an email.
Status: Agreed 2026-10-04

[PAR-042] A form request and a multipart request MUST run their stages in
this order: `prepare_for_validation`, which MAY change the input before
anything reads it; `authorize`, before the body is validated; extraction
with its field validation (PAR-043 for multipart); `after_validation`;
`after_validation_async`; the handler. On a real request the two hooks
MUST run after the derived stage whether or not it reported errors, and
their errors MUST be merged with the rule errors into one answer, as
Laravel's `after` hooks run alongside the rules; on a marked request
PAR-083 governs what runs. `MultipartRequestHooks` MUST offer
`after_validation_async`, whose default succeeds. A non-empty
`ValidationErrors` MUST answer as a validation failure: a 422 whose body
holds `errors` for a request that accepts JSON or sends an Inertia visit,
the latter turned into a redirect back with the errors by the Inertia
validation middleware; and for a request whose `Accept` prefers
`text/html` without `X-Inertia`, a plain HTML form, a redirect back with
the errors and the input flashed to the session, as Laravel redirects a
failed form post. An empty set of errors counts as success.
Falsifier: a multipart request whose `after_validation_async` returns an error reaches its handler, answers anything but a 422 with that field's error in `errors`, or through Inertia does not redirect back with it in `props.errors`; a stage runs after an earlier one failed, the async hook before the sync one, `authorize` after the body was read, or `prepare_for_validation` after `authorize`; a `prepare_for_validation` that lower-cases `email` leaves the handler or the rules with the original case; on a real request whose `email` rule fails, an `after_validation` error on `name` is missing from the 422; a classic form post with `Accept: text/html` and no `X-Inertia` answers a 422 instead of a redirect back, or redirects without the errors and the old input in the session; or an empty error set fails the request.
Mechanism: `par-multipart-validation`.
Rationale: Issue #139 and row 042 of the parity log: Laravel's `FormRequest::prepareForValidation` runs before `passesAuthorization`, its `after` hooks run alongside the rules, and `failedValidation` redirects a non-JSON request back with the errors and input; the 303 for every Inertia method stays, since browsers follow it like Laravel's 302.
Status: Agreed 2026-10-08

[PAR-043] A multipart extraction failure that belongs to one field MUST
answer as `ValidationErrors` under the field's form input name, the
`#[field(...)]` name where one is given, with a trailing `[]` replaced by
a zero-based index, so `#[field("files[]")]` gives `files.1` for the
second file; hook errors use the same names. The failures are: a missing
required field (`validation-required`); a text part that does not parse
as its field's type (`validation-integer`, `validation-numeric` or
`validation-boolean` by that type, `validation-format` for any other); a
file part where text belongs or a text part where a file belongs
(`validation-file`, `validation-string`); and a file an `UploadValidator`
refuses as a validation failure: too large for `MaxSize`
(`validation-max-file`, with the limit in kilobytes as Laravel words it),
not an image (`validation-image`), or of a type `MimeType` does not allow
(`validation-mimetypes`), where an allowed type MAY be a `type/*`
wildcard that admits every subtype, as Laravel's `mimetypes` rule does,
except that `image/*` MUST NOT admit `image/svg+xml`: an SVG document
passes only when `image/svg+xml` is named explicitly, as Laravel's
`image` rule leaves SVG out unless asked (the developer, 2026-10-09
05:58).
Each message MUST come from the validation catalog by that key, so an
application's `lang/<locale>/validation.ftl` overrides it. An
`UploadValidator` MUST be able to return a validation failure with a
catalog key, apart from an operational error, which keeps its own status.
A field failure found while the body streams, as `MaxSize` is, MUST stop
reading the body after the chunk that crossed the limit, skip both hooks
and the handler, and remove every temporary file the extraction wrote. A
limit on the whole request, the body's byte cap, `max_parts` and a
field's `max_count`, MUST refuse with 413 without reading the body
further, and wins when one chunk crosses both kinds.
Falsifier: a multipart form missing a required file, or with a PDF as the second element of a `#[field("files[]")]` field of images, answers anything but a 422 whose `errors` holds `files.1`, or through Inertia does not redirect back with it in `props.errors`; a text part that does not parse answers 400; a message ignores an application catalog's entry for its key; `MimeType::allow(["image/*"])` refuses a PNG, admits a PDF or admits an SVG document, or `MimeType::allow(["image/svg+xml"])` refuses one; an oversized file is read past the chunk that crossed `MaxSize`, or leaves a temporary file behind; a validator's operational error becomes a 422; or a body over its cap, too many parts or too many files for `max_count` is read further or answers other than 413.
Mechanism: `par-multipart-validation`.
Rationale: Issue #139 and row 043 of the parity log: Laravel's `mimetypes` rule matches `image/*` by `ValidatesAttributes::validateMimetypes`; the typed field errors, the 413 limits and the magic-byte check stay.
Status: Agreed 2026-10-09

[PAR-044] Without `key_type`, `#[model]` MUST take the key type from the
primary-key field's declared type, the field `primary_key` names, and a
`key_type` that disagrees with that field MUST fail to compile with an
error naming both. A model field declared `u64` or `Option<u64>`, the key
and foreign keys included, MUST read and write on SQLite, Postgres and
MySQL: the whole `u64` range on MySQL's unsigned columns, and
`0..=i64::MAX` on SQLite and Postgres, which have no unsigned integers
and store it in a signed column. On those two, writing a value above
`i64::MAX` MUST fail with an error naming the column before anything is
sent, never a panic, truncation or wrap, and reading a negative value
MUST fail with an error naming the column.
Falsifier: a model whose key field is `u64` and that names no `key_type` gets an `i64` key; a disagreeing `key_type` compiles; on any of SQLite, Postgres and MySQL, a `u64`-keyed model with a `u64` foreign key and an `Option<u64>` field returns a wrong id, field, related record, order or count, or fails, through create with a generated id, update, `find`, `find_many`, a `where` on the key, a direct and an eager relation, `with_count` and a paginated query; on SQLite or Postgres a write of `i64::MAX + 1` panics or is sent, or a stored negative value reads as a `u64`.
Mechanism: `par-laravel-defaults`.
Rationale: Issue #137 and its follow-up comment: a Laravel table's keys are unsigned on MySQL, while SeaORM reads a `u64` only on MySQL and its Postgres and SQLite binders unwrap the conversion, so a model over one could not run on `TestDatabase`, which is SQLite.
Status: Agreed 2026-10-04

[PAR-045] Migrations and models MUST default to Laravel's MySQL shape:
`id()` and `foreign_id()` MUST create unsigned columns on MySQL (unchanged
on Postgres and SQLite), as Laravel's `Blueprint::id` and `foreignId` do,
and `timestamps()` MUST create nullable `created_at` and `updated_at`, as
Laravel's `Blueprint::timestamps` does. An application MAY set
`[package.metadata.suprnova.schema]` `unsigned_ids = false` in the
binary's `Cargo.toml` to keep signed ids, and `[package.metadata.suprnova.model]`
`datetime_cast` to choose the model casts: the model table is read from
the package that declares the model, by `#[model]`; the schema table from
the package of the binary, by `#[suprnova::main]`, which installs it
before anything runs. `datetime_cast = "native"` makes every
`DateTime<Utc>` and `Option<DateTime<Utc>>` field of a model with no cast
of its own, the managed timestamps included, use `AsNativeDateTime` or
`AsOptionalNativeDateTime`, a time-zone-aware column; `"naive"` uses
`AsNaiveDateTime` or `AsOptionalNaiveDateTime`, for the time-zone-free
columns Laravel creates on Postgres; a field's own cast wins. The setting
chooses casts and converts no column: the manual and the scaffold's
comment MUST say which column types each value needs and show the
per-field override for a column that differs. SQLite MAY store a native
date-time as its driver's text, so long as the value round-trips. A
program that runs migrations without `#[suprnova::main]` MUST be able to
install the schema setting with one documented call. A key or value in
either table that the framework does not know MUST fail the build of the
macro that reads it, naming it. The scaffold's `Cargo.toml` MUST carry
both settings commented out, saying the defaults match Laravel's MySQL
schema and what each setting changes.
Falsifier: without any setting, `migrate` on the app binary against MySQL creates a signed `id` or foreign key column, or `timestamps()` creates a `NOT NULL` `created_at` on any engine; with `unsigned_ids = false`, an `id` on MySQL is unsigned; with `datetime_cast = "native"` or `"naive"`, a model's managed timestamp uses another cast, or a field's explicit cast is replaced; a native value does not round-trip on any of SQLite, Postgres and MySQL; a migrations library run by a binary with the setting misses it, or a program using the documented call does; an unknown key or value builds; or a new scaffold lacks the commented settings.
Mechanism: `par-laravel-defaults`.
Rationale: Issue #137 and row 045 of the parity log: Laravel's `Blueprint::id` is `bigIncrements`, unsigned on MySQL, `foreignId` is `unsignedBigInteger`, and `timestamps()` adds `nullable()`; the settings stay for an application that wants the other shape, and the schema setting reaches the migrations through `#[suprnova::main]` because migrations build their schema at run time.
Status: Agreed 2026-10-08

## Inertia protocol

The developer's rulings of 2026-10-07 on the parity map ("I am going to
accept your recommendations", 16:14; every row defaults to build toward
Laravel's behaviour) cover the Inertia server adapter, inertia-laravel 3.5.1
and the client's protocol in Inertia.js 3.8.0: the headers, props
resolution, prop types, JSON encoding, middleware, response factory,
response shape and the request bodies the client sends. The map rows are
listed in `docs/superpowers/notes/parity-rulings-2026-10-07.md` (untracked);
the application-owned root document rows (RF-01, RS-02, RS-05, BL-04) are
RDOC-001 to RDOC-004 and RDOC-006, and the client nonce row (H03) is
RDOC-005, built with SEC-003. Laravel adapter references cite
`reference/inertia-laravel-3.5.1/src/`.

[PAR-046] A request MUST count as an Inertia visit when its `X-Inertia`
header holds any value but an empty string or `0`, as PHP's boolean cast
reads it, and every Inertia JSON response MUST carry `X-Inertia: true`.
The asset version MUST resolve, in order, from the configured asset URL
setting when one is set, else from the Vite manifest's hash, else to the
empty string; `Inertia::version(value or function)` MUST set it at run time
and `Inertia::get_version()` MUST read it. On an Inertia `GET` whose
`X-Inertia-Version` differs from the current version the server MUST
answer `409 Conflict` before the handler runs, with `X-Inertia-Location`
holding the request's absolute URL (scheme, host, path and query) and
`X-Inertia-Version` holding the current version; a visit by any other
method MUST pass through, as today.
Falsifier: a request with `X-Inertia: 1` gets the HTML document, or one with `X-Inertia: 0` or an empty header gets JSON; with no asset URL setting and no manifest the version is `1.0`; the version 409 lacks `X-Inertia-Version` or carries a relative `X-Inertia-Location`; or a version set with `Inertia::version` differs from what `get_version` returns or what the 409 compares against.
Mechanism: `par-inertia-protocol`.
Rationale: Rows HD-01, HD-10, MW-02, MW-11, R04 and RF-05; Laravel's `Request::inertia()` casts the header to a boolean, and the client reads `X-Inertia-Version` on the 409 to spare async visits a forced reload.
Status: Agreed 2026-10-07

[PAR-047] On a partial reload (`X-Inertia-Partial-Component` naming the
page's component) the server MUST read `X-Inertia-Partial-Data` and
`X-Inertia-Partial-Except` as comma lists whose empty segments are dropped
and whose entries are not trimmed, and treat an empty header as absent;
`except` applies after `only`, and `always` props ignore both. A dotted
entry MUST narrow literal values only: a prop whose value came from a
resolver or a prop object ships whole when its path or an ancestor is
selected, and an entry whose path resolves to nothing yields `[]`. An
`optional` or `defer` prop MUST resolve on a partial reload whenever its
path passes the `only` and `except` lists, a reload with `except` alone
included. `merge` and `once` instructions MUST be emitted only when an
`only` entry is the prop or an ancestor of it; a deeper entry ships the
whole prop with no instruction; and a `once` prop the client already holds
(not deferred) emits its `onceProps` entry and no `mergeProps` entry.
Falsifier: with `X-Inertia-Partial-Data: a,,b` a prop named `b ` is selected or `a` dropped; an empty `X-Inertia-Partial-Data` yields no props; `only: users.name` on a resolver-backed `users` drops its other fields; `only: missing.path` yields anything but `[]`; a partial reload with `X-Inertia-Partial-Except: x` alone omits an optional or deferred prop whose path is not `x`; `only: items.data` on a merge prop `items` emits a `mergeProps` entry; or a held `once` prop appears in `mergeProps`.
Mechanism: `par-inertia-protocol`.
Rationale: Rows HD-03, HD-04, PR-07, PR-08, PR-12, PR-13 and PT-01; the client receives different data from Laravel's `PropsResolver` today, and the manual's divergence entry on narrowing goes.
Status: Agreed 2026-10-07

[PAR-048] An empty `200` on an Inertia visit MUST become a redirect back:
to the `Referer` when it passes the same-origin check the validation
redirect applies, else to the session's previous URL, else to the fallback,
else to `/`, with status `302`, or `303` for `PUT`, `PATCH` and `DELETE`;
`Inertia::back(status, fallback)` MUST follow the same order. A redirect on
an Inertia visit whose `Location` carries a `#fragment` MUST become `409
Conflict` with `X-Inertia-Redirect` holding the location, unless the
request is a prefetch (`X-Moz`, `Purpose` or `Sec-Purpose` equal to
`prefetch`); a prefetch MUST record no previous URL either. An Inertia `GET`
that matched a route and is not a prefetch, not precognitive and not a
partial reload of the same component MUST record the session's previous URL
while `store_previous_url` (default on) is set. With `X-Inertia-Error-Bag`
the `errors` prop MUST be `{<bag>: <default bag>}` when the session holds a
default bag, `{}` when it holds no errors, and the named bags alone
otherwise.
Falsifier: an empty 200 on an Inertia `POST` with a same-origin `Referer` redirects elsewhere than the `Referer`, or with a status other than 302; one on `PUT` is not 303; a redirect to `/page#section` on an Inertia visit arrives as a plain 302, or as a 409 when the request carries `Sec-Purpose: prefetch`; an Inertia GET visit leaves the previous URL unchanged with `store_previous_url` on, or changes it when off or on a partial reload of the same component; or the `errors` prop with `X-Inertia-Error-Bag: form` and no session errors is not `{}`.
Mechanism: `par-inertia-protocol`.
Rationale: Rows HD-08, HD-12, HD-13, MW-03, MW-06, R06, R09 and RF-18; browsers and fetch follow a 302 after `POST` as `GET`, so the 303-for-every-method rule protected nothing, and the open-redirect check on previous URLs stays.
Status: Agreed 2026-10-07

[PAR-049] The `Inertia` facade MUST expose at run time what Laravel's
`ResponseFactory` does: `share` taking a key and value, a map, a Data
object or a provider, a dotted key nesting at share time; `get_shared(key,
default)` and `get_shared_all()`; `version` and `get_version` (PAR-046);
`transform_component_using(fn)` rewriting a component name before render,
a `None` result keeping the name; `disable_ssr(bool or fn(&Request) ->
bool)` deciding per request, able to turn SSR on as well as off;
`without_ssr(patterns)` excluding paths with Laravel's rules (leading and
trailing slashes trimmed, `*` matching any characters, each pattern tried
against the path and the full URL); `configure_ssr_request_using(fn)`
adjusting the request sent to the SSR server; `location(url or redirect)`
answering `409` with `X-Inertia-Location` to an Inertia visit and a `302`
redirect otherwise, `location_for` kept; and an `ensure_pages_exist`
setting that makes rendering a component with no page file under the
configured pages directory an error, for a name given as a string as well
as through the macro.
Falsifier: `share("a.b", 1)` is read back as a flat `a.b` key; `get_shared("missing", 7)` is not 7; a transformer returning `Some("Other")` renders the original component; `disable_ssr(|r| r.path() == "/no")` leaves SSR off for `/yes` when the configuration has it off, or on for `/no`; `without_ssr("admin/*")` fails to exclude `/admin/users` or excludes `/adminx`; the SSR request lacks a header set through `configure_ssr_request_using`; `location("/x")` on a request without `X-Inertia` answers 409; or with `ensure_pages_exist` on, `InertiaResponse::new("Missing")` renders 200.
Mechanism: `par-inertia-protocol`.
Rationale: Rows HM-04, RF-02, RF-03, RF-07, RF-11, RF-12, RF-13, RF-15 and RF-16; a Laravel application's `Inertia::` calls port by name, and the manual's always-409 claim for `location` is wrong for 3.5.1.
Status: Agreed 2026-10-07

[PAR-050] Inertia flash data (`Inertia::flash` with a key, a map or an
enum key, and `InertiaResponse::flash`) MUST live in the session under
`inertia.flash_data`, be emitted as `page.flash` and pulled when a page is
rendered, be kept when the response is a redirect, and be readable and
removable with `Inertia::get_flashed(request)` and `pull_flashed(request)`.
The `clear_history` and `preserve_fragment` flags MUST persist in the
session until a page response emits them (`clearHistory: true`,
`preserveFragment: true`), however many redirects come first;
`Redirect::preserve_fragment()` MUST set the flag.
Falsifier: flash set on a request that answers a redirect, followed by a second redirect, is gone at the page after it; `get_flashed` returns nothing for data `pull_flashed` would remove; a logout that calls `clear_history` and answers two redirects renders the next page without `clearHistory: true`; or a `preserve_fragment` set before two redirects is not emitted.
Mechanism: `par-inertia-protocol`.
Rationale: Rows RF-08, RF-09, RF-17, RF-19 and MW-05; a one-request flash lets a logout followed by two redirects leave private pages in the history.
Status: Agreed 2026-10-07

[PAR-051] A page's props and the shared props MUST accept any number of
provider values (a Data object or a type implementing
`ProvidesInertiaProperties`), each expanded at render with a render
context of the component name and the request, and a prop value MAY
implement `ProvidesInertiaProperty` to be converted with a property
context of its key path, its sibling props and the request. The page
object MUST carry `sharedProps`, the top-level segment of every shared key
with `errors` included when the validation redirect shares it, while
`expose_shared_props` (default on) is set, and omit it when unset.
Falsifier: two providers given to a response lose either's keys; a provider sees a component name other than the page's; a `ProvidesInertiaProperty` value is converted without its key path or its siblings; `sharedProps` lacks `errors` after a validation failure or lists a nested key; or it is present with `expose_shared_props` off.
Mechanism: `par-inertia-protocol`.
Rationale: Rows PR-01, PR-03 and PR-04; the client reads `sharedProps` during instant visits, and the shared-data provider already shows the context form.
Status: Agreed 2026-10-07

[PAR-052] Merge props MUST take `append(path or paths, match_on)` and
`prepend(path or paths, match_on)` at nested paths, the two-argument form
adding `{path}.{match_on}` to `matchPropsOn`, and `match_on` MUST replace
the list on each call, emitted as `{path}.{field}`. `once` props MUST take
`as(key or enum)`, `until(DateTime, Duration or seconds)`, `fresh(bool)`
and `once(bool, as, until)`, with `expiresAt` in milliseconds from the
expiry, the server's refusal of a stale client claim kept;
`Inertia::share_once(key, fn)` MUST return a once prop taking the same
options. A `scroll` prop MUST default its wrapper to `data`, take
`match_on` relative to the prop and emit it as `{key}.{path}` with no
wrapper prefix, carry metadata from a length-aware, simple or cursor
paginator (page name, current, previous and next, encoded cursors) or from
a `ProvidesScrollMetadata` value or a function of the loaded value, and,
when deferred, announce the bare key in `mergeProps` on the first visit and
`{key}.{wrapper}` when the data arrives.
Falsifier: `append(["a.items", "b"], "id")` emits anything but both merge paths with `a.items.id` and `b.id` in `matchPropsOn`; `match_on("x").match_on("y")` emits both; `once` with `until(60)` emits an `expiresAt` other than now plus 60 s in milliseconds; `share_once` cannot take `as` or `fresh`; a scroll prop `posts` with `match_on("data.id")` emits `posts.data.data.id`; a cursor paginator's current page or custom page name is missing from `scrollProps`; or a deferred scroll prop announces `{key}.data` on the first visit.
Mechanism: `par-inertia-protocol`.
Rationale: Rows PT-06, PT-07, PT-10, PT-11, PT-12, PT-13, PT-14 and PT-15; the merge, once and scroll instructions are shapes the client reads, so a difference changes what the page shows.
Status: Agreed 2026-10-07

[PAR-053] The page JSON written into the first-visit `<script>` MUST
escape `<` and `>` as `<` and `>`, and `/` as today. A page
that cannot be encoded MUST answer an error response, never a `200` with an
empty page. With `preserve_big_integers` (configuration or
`InertiaResponse::preserve_big_integers(bool)`) every integer beyond plus
or minus 9007199254740991 in props and flash, at any depth, MUST be emitted
as `{"$bigint": "<digits>"}` and the page MUST carry `preserveBigIntegers:
true`.
Falsifier: a prop holding `<!--<script>` reaches the first-visit document unescaped, so a DOM parser finds no mount element; a serializer that fails yields 200; with `preserve_big_integers` on, 9007199254740993 is emitted as a JSON number or the flag is missing; or with it off an integer is wrapped.
Mechanism: `par-inertia-protocol`.
Rationale: Rows JE-01, JE-03, JE-04, JE-05 and H02; a confirmed defect (a prop swallows the closing script tag and the mount element) and the client's exact-integer restoration.
Status: Agreed 2026-10-07

[PAR-054] An application MUST be able to replace the Inertia middleware's
decisions through a hook trait (`version`, `share`, `share_once`,
`root_view`, `url_resolver`, `on_empty_response`, `on_version_change` and
`on_redirect_with_fragment`, each defaulting to the framework's behaviour)
and to register the Inertia middleware stack on a route group instead of
globally, so a group without it emits no `Vary: X-Inertia` and no Inertia
conversion.
Falsifier: an `on_empty_response` hook returning a 204 still yields the redirect back; an `on_version_change` hook returning a page still yields the 409; a route group registered without the Inertia stack carries `Vary: X-Inertia` or has its 302 turned into 303; or a group registered with it lacks them.
Mechanism: `par-inertia-protocol`.
Rationale: Rows MW-08 and MW-10; a hook trait fits the framework's pattern, and an API group should not carry the Inertia headers.
Status: Agreed 2026-10-07

[PAR-055] A request body the Inertia client sends MUST be read as the
client shapes it: `multipart/form-data` with bracketed and indexed names
(`photos[0]`, `user[name]`, `tags[]`), and the same names in a URL-encoded
body or a `GET` query string, MUST decode into nested form data (objects
for bracketed keys, lists for `[]` and indexed keys, files where a part
carries a filename), so a form request and `Request::input` read
`user.name` and `tags` as Laravel does, and a multipart body MUST NOT
answer `415`.
Falsifier: a multipart `POST` with `photos[0]` and `user[name]` answers 415, or a form request declaring `user.name` reads nothing; `GET /items?filters[status]=x&tags[]=a&tags[]=b` yields a flat `filters[status]` key or a `tags` of one element; or an indexed name decodes out of order.
Mechanism: `par-inertia-protocol`.
Rationale: Rows P16 and P17; the client sends file forms as multipart with bracketed names and GET data in brackets format, so a drop-in form fails today.
Status: Agreed 2026-10-07

[PAR-056] The page object's `url` MUST carry the request's path and its
query string normalised as Laravel's `fullUrl` does through Symfony: pairs
parsed, sorted by key, and re-encoded per RFC 3986 (space as `%20`,
reserved characters percent-encoded), so the client's comparison of page
URLs matches Laravel's.
Falsifier: a request to `/s?b=2&a=1%20x` yields a page `url` other than `/s?a=1%20x&b=2`; or one to `/s?a=%2Fx` yields `a=/x`.
Mechanism: `par-inertia-protocol`.
Rationale: Row RS-07, settled from Symfony 7.3 `Request::normalizeQueryString`.
Status: Agreed 2026-10-07

## Inertia SSR, error pages and commands

The second parity round the developer ordered on 2026-10-07 (17:35): the
fifteen build rows of inertia-laravel 3.5.1's SSR gateway, exception
handling and commands (SS-02, SS-03, SS-04, SS-05, SS-08, SS-10, SS-13,
SS-14, EX-01, EX-02, EX-03, EX-06, EX-07, CM-01 and CM-02). SS-07 keeps the
5 second timeout and CM-03 keeps `ssr:check` under its name, and CM-04,
EX-04, EX-05, EX-08 and SS-15 are not applicable, as the log of the rulings
says.

[PAR-057] SSR MUST be on by default (`SsrConfig::enabled` true, the worker
at `http://127.0.0.1:13714`) and gated by bundle detection: while
`ensure_bundle_exists` (default on) is set, a first visit is dispatched to
the worker only when an SSR bundle exists at the configured `bundle_path`
or at one of the conventional paths `frontend/bootstrap/ssr/ssr.js`,
`frontend/bootstrap/ssr/app.js`, `frontend/bootstrap/ssr/ssr.mjs`,
`frontend/bootstrap/ssr/app.mjs`, `public/js/ssr.js` and `public/js/app.js`
under the working directory; with none found the visit MUST render on the
client, with no request to the worker and no error, and
`ensure_bundle_exists(false)` MUST dispatch without the check.
Falsifier: `SsrConfig::default().enabled` is false; with the default configuration and no bundle on disk a first visit sends a request to the worker or answers an error; with a bundle at `frontend/bootstrap/ssr/ssr.mjs` and no `bundle_path` the visit is not dispatched; or with `ensure_bundle_exists(false)` and no bundle the visit is not dispatched.
Mechanism: `par-inertia-ssr`.
Rationale: Rows SS-03 and SS-04; Laravel's `inertia.ssr.enabled` defaults to true behind `BundleDetector`, so an application opts out of SSR rather than in, and an application without a bundle sees no change.
Status: Agreed 2026-10-08

[PAR-058] In development a first visit MUST go hot, dispatching SSR to
`{hot url}/__inertia_ssr` and skipping the bundle check, while the hot file
(`SsrConfig::hot_file`, default `public/hot`) exists or `SsrConfig::hot_url`
is set: the address is the hot URL, else the file's content, else the Vite
dev server's URL. `suprnova serve` MUST write the file while it runs Vite
with `@inertiajs/vite` declared in `frontend/package.json`, and remove it
when Vite stops. Without the file and without `hot_url`, and always in
production, the visit MUST post to the worker's `/render`.
Falsifier: in development with the hot file present the SSR request goes to `{url}/render` or a missing bundle stops it; without the file and without `hot_url` a dev server listening at its port is asked; in production the request goes to the hot URL; a configured `hot_url` is not the address used; or `serve` writes the file when `frontend/package.json` declares no `@inertiajs/vite`.
Mechanism: `par-inertia-ssr`.
Rationale: Row SS-05; Laravel's `HttpGateway::dispatch` posts to `{hot url}/__inertia_ssr` while `Vite::isRunningHot`, the hot file its Vite plugin writes, which removes the separate SSR process during development; the first wording, "with the Vite dev server running", read as a port probe (adversary finding 1 of inertia-ssr-errors-commands, declined).
Status: Agreed 2026-10-08

[PAR-059] A worker answer whose JSON is empty, `null`, `false` or not an
object MUST render on the client. A non-2xx answer MUST be read as the
worker's error JSON (`error`, `type`, `hint`, `browserApi`, `stack`,
`sourceLocation`) and dispatched as an `SsrRenderFailed` event carrying the
page's component and url and those fields, the type one of `browser-api`,
`component-resolution`, `render`, `connection` and `unknown`; a transport
failure MUST dispatch the same event with type `connection`. With
`throw_on_error` the visit MUST fail with an error naming the component
and, when given, the source location; without it the visit MUST render on
the client and the `on_error` hook MUST still fire. The SSR client MUST
speak TLS to a worker at an `https` URL.
Falsifier: a worker answer of `{}` or `null` yields a document without the page data element or the mount element; a 500 from the worker dispatches no `SsrRenderFailed`, or one without the component; a refused connection dispatches none with type `connection`; with `throw_on_error` the error names no component; or an `https` worker URL receives plain HTTP on the wire.
Mechanism: `par-inertia-ssr`.
Rationale: Rows SS-02, SS-08 and SS-14; an empty SSR result gave a page the client could not start, the worker's error details and the event make SSR failures diagnosable, and the client spoke plain HTTP only.
Status: Agreed 2026-10-08

[PAR-060] The SSR call MUST be an `SsrGateway` trait bound in the container,
with the HTTP gateway as the default binding, so an application binds its
own: `dispatch(page, request)`, `is_healthy()` (`GET {url}/health`
successful), and the capabilities `disable(condition)`, `except(paths)` and
`configure_request_using(fn)`, each a no-op by default that an
implementation overrides. `Inertia::disable_ssr`, `without_ssr` and
`configure_ssr_request_using` MUST act on the bound gateway, and
`Inertia::ssr_is_healthy()` MUST return its health.
Falsifier: a gateway an application bound is not the one a first visit dispatches through; `disable_ssr(true)` still dispatches through a bound gateway that implements `disable`; `ssr_is_healthy()` is true when `/health` answers 500 or does not answer; or the HTTP gateway is not what dispatches when nothing is bound.
Mechanism: `par-inertia-ssr`.
Rationale: Rows SS-10 and SS-13; the trait-and-driver principle of `manual/introduction.md`, and the health check the commands read lives on the gateway as Laravel's `HasHealthCheck` does.
Status: Agreed 2026-10-08

[PAR-061] The application binary MUST have `ssr:start`, `ssr:stop` and
`ssr:check` reading the installed Inertia configuration: `ssr:start`
refuses when SSR is disabled, finds the bundle as PAR-057 does (failing
when none exists, warning when the configured one is missing and a
conventional one is used), refuses a runtime (`SsrConfig::runtime`, default
`node`) that cannot be found when `ensure_runtime_exists` is set, asks a
running worker to shut down first and refuses to start when it did not
stop, and runs the worker in the foreground, forwarding the `SIGINT` or
`SIGTERM` it receives and reporting its stderr as errors; `ssr:stop` MUST
`GET {url}/shutdown`, succeeding when the worker closed the connection
and, with `--graceful`, when none was running; `ssr:check` MUST fail when
the gateway has no health check or the worker is unhealthy. The `suprnova`
CLI's `ssr:start`, `ssr:stop` and `ssr:check` MUST run the project's
application binary's command of the same name, flags passed through, the
way `suprnova serve` runs the backend, so the installed configuration
decides; the CLI MUST NOT depend on the framework crate.
Falsifier: `ssr:start` starts a worker with SSR disabled, with no bundle, or with a missing runtime under `ensure_runtime_exists`, or starts beside a worker whose `/shutdown` answered without stopping; `ssr:stop` fails with `--graceful` when no worker runs, or succeeds when the worker is running and did not close; `ssr:check` succeeds against a worker whose `/health` fails; the CLI's `ssr:start` runs anything but the application binary's `ssr:start` with the flags given, or runs it from a directory that is not the project; or `suprnova-cli/Cargo.toml` names the `suprnova` crate among its dependencies.
Mechanism: `par-inertia-ssr`.
Rationale: Rows CM-01, CM-02 and CM-03; Laravel's `inertia:start-ssr`, `inertia:stop-ssr` and `inertia:check-ssr` run inside the application and read its configuration, which only the application binary can here, and the developer ruled on 2026-10-08 10:58 that compiling the framework into the CLI for them is nonsense, after the second round had the CLI link the framework crate to share the code.
Status: Agreed 2026-10-08

[PAR-062] `Inertia::handle_exceptions_using(callback)` MUST let the
application decide every error response the framework renders, for every
request type, a handler panic the boundary caught included: the callback
receives an `InertiaErrorResponse` with the status, the error report, the
request and the response the framework would send, and returns a
replacement response or nothing, which keeps the original;
`InertiaErrorResponse::render(component, props)` MUST render an Inertia
page with the original status, including the shared props (the shared
registry and the middleware `share` and `share_once` hooks) only when
`with_shared_data()` was called. `InertiaConfig::error_page(component)`
MUST install the default callback, which keeps today's rule: an Inertia
visit or an HTML-wanting request answered with the framework's error body
and a status from 400 to 599 gets the page with the sanitised message, the
request id and `Cache-Control: no-cache, private`, shared data included,
and every other response stays as it is. With debug on, the response the
callback receives for a 5xx is the development error page (PAR-012), which
the default callback keeps.
Falsifier: a callback returning a 418 for a 404 still yields the error page or the original 404; a callback returning nothing changes the response; a page the callback rendered loses the original status; a rendered page carries shared props without `with_shared_data()`, or `error_page(component)` renders one without them; a handler panic reaches the client without the callback seeing it; a JSON API client's 404 is replaced when the callback returned nothing; or `error_page(component)` alone no longer renders the page for an Inertia visit's 403.
Mechanism: `par-inertia-ssr`.
Rationale: Rows EX-01, EX-02, EX-03, EX-06 and EX-07; Laravel decides each rendered error in `handleExceptionsUsing` with `ExceptionResponse`, shared data opt-in, and the one-line `error_page` setup stays as sugar over the default callback.
Status: Agreed 2026-10-08

## Inertia testing helpers and the TypeScript generator

The third parity round the developer ordered on 2026-10-07 (17:35): the
sixteen build rows of inertia-laravel 3.5.1's testing surface (TS-01,
TS-03, TS-06, TS-07, TS-08, TS-10, TS-12, TS-13, TS-14, TS-15 and TS-16)
and of the Inertia.js 3.8.0 types the generator writes (T01, T02, T03, T05
and T06). Laravel references cite `reference/inertia-laravel-3.5.1/src/Testing/`
and `reference/framework-13.35.0/src/Illuminate/Testing/Fluent/`; the
client types cite `reference/inertia-3.8.0/packages/core/src/types.ts`.

[PAR-063] `suprnova::testing::TestClient` MUST drive requests through the
framework's request path in-process: built from a router and a middleware
registry, and optionally the session store and cookie name the test's
session middleware uses, it sends `get`, `post`, `put`, `patch`, `delete`
and `send(method, path)` requests with headers, a JSON or form body and an
Inertia visit's headers (`inertia()`, with the version), over an
in-memory connection and never a port, carries the session cookie from
each response to the next request, and returns a `TestResponse` that
keeps the response's error report and the attached session store. A
`TestResponse` the client returned MUST reload Inertia pages through the
same client with no reloader attached by hand.
Falsifier: a value put in the session by one request is absent from the next request of the same client; a request skips a middleware the registry registers; a response's `error_report()` is `None` for a handler that returned an error; `assert_session_has` on a client response fails for a value the session holds when the client was given the store; or `assert_inertia().reload_only(..)` on a client response panics for a missing reloader.
Mechanism: `par-inertia-testing`.
Rationale: Rows TS-01 and TS-10; Laravel's `MakesHttpRequests` is the one test client every `ReloadRequest` and `assertInertia` reaches for, where every Suprnova test file copied its own socket harness.
Status: Agreed 2026-10-08

[PAR-064] `TestResponse::assert_inertia()` MUST read the page object from
an `X-Inertia` JSON body or from a first visit's HTML document (its
`data-page` element), and `assert_inertia_with(callback)` MUST hand the
page to the callback and return the response for chaining.
`AssertableInertia::component(name)` MUST also check that a page file for
`name` exists under the installed configuration's `pages_dir` with one of
its `page_extensions` while `InertiaConfig::testing_ensure_pages_exist` is
on, which it is by default, skipping the check when no Inertia
configuration is installed; `component_exists(name, bool)` MUST decide the
check for one call.
Falsifier: `assert_inertia()` panics on a first-visit HTML response that carries a page object; `assert_inertia_with` returns anything but the response or skips the callback; with the default configuration installed and no page file, `component("Missing")` passes, or with `testing_ensure_pages_exist(false)` it fails for the file alone; `component_exists("Missing", false)` checks the file, or `component_exists("Home", true)` passes without one; or `component` checks a file with no configuration installed.
Mechanism: `par-inertia-testing`.
Rationale: Rows TS-01 and TS-03; Laravel's `assertInertia` works on the ordinary page response and chains, and `component` checks the page file under `inertia.testing.ensure_pages_exist`, default true, with the view finder that PAR-049's `ensure_pages_exist` already ports.
Status: Agreed 2026-10-08

[PAR-065] `AssertableInertia` MUST offer Laravel's `AssertableJson` prop
assertions over dotted paths: `has`, `has_all`, `has_any`, `missing`,
`missing_all`, `count`, `count_between`, `where_`, `where_not`,
`where_null`, `where_not_null`, `where_all`, `where_type` (`string`,
`integer`, `double`, `boolean`, `array`, `null`, joined by `|`),
`where_all_type`, `where_contains`, and the scoped forms `scope(path,
callback)`, `first(callback)`, `each(callback)`, `has_with(path,
callback)` and `has_count_with(path, count, callback)`, each callback
receiving the nested value as an `AssertableInertia` whose paths and
messages carry the full dotted path. A scope MUST fail when it ends with a
prop no assertion in it touched, unless `etc()` was called in it; the root
level MUST NOT enforce that. Every assertion MUST return `&Self`.
Falsifier: `has_all(["a", "b"])` passes with `b` absent, or `has_any(["a", "b"])` fails with `a` present; `where_type("n", "integer|null")` fails for `null` or passes for `"1"`; `where_contains("tags", "x")` passes for an array without `x`; `count_between("items", 1, 3)` passes for four items; `scope("user", |u| { u.where_("name", "Ada"); })` passes with an untouched `user.email`, or fails after `u.etc()`; a failure inside `scope("user", ..)` names `user.name` without the prefix; a page with an untouched root prop fails `assert_inertia_with(|page| { page.component("Home"); })`; or `first` or `each` on an empty array passes.
Mechanism: `par-inertia-testing`.
Rationale: Row TS-06; ported Laravel tests use scoped, multi-key, type, null and range assertions, and the every-prop-checked rule is how `AssertableJson` catches a prop a page leaks.
Status: Agreed 2026-10-08

[PAR-066] `AssertableInertia::reload(callback)` MUST replay the page as a
full reload and assert the same component, url and version, `reload_only`
and `reload_except` MUST take an optional callback over the reloaded page
and keep their key checks, and `load_deferred_props(groups, callback)`
MUST reload only the props of the named deferred groups, every group when
none is named. A reload MUST go through the client the page came from, or
through the closure `with_reload` attached, and `ReloadRequest::headers`
MUST send `X-Inertia`, `X-Inertia-Version` and the partial headers only
when a list is set.
Falsifier: `reload(..)` on a response whose reload lands on another component passes; `load_deferred_props(["stats"], ..)` requests a prop of another group; `load_deferred_props([], ..)` leaves a deferred group unrequested; a `reload_only(["users"], |r| ..)` skips its callback; a reload of a client response needs `with_reload`; or a full reload sends `X-Inertia-Partial-Component`.
Mechanism: `par-inertia-testing`.
Rationale: Rows TS-07, TS-08 and TS-10; Laravel's `reload` replays through the application and `loadDeferredProps` takes the groups to load.
Status: Agreed 2026-10-08

[PAR-067] `AssertableInertia` MUST offer `missing_flash(key)`, `to_page()`
(the whole page: component, props, url, version, flash, and
`encryptHistory` and `clearHistory` only when the page sets them) and the
readers `encrypt_history()` and `clear_history()`; `TestResponse` MUST
offer `inertia_page()`, `inertia_props(path)` (the whole props with no
path) and `assert_inertia_flash(key, expected)` and
`assert_inertia_flash_missing(key)`, which read the Inertia flash data
the session holds after a redirect through the attached session store.
When the page sets `preserveBigIntegers`, every `{"$bigint": "<digits>"}`
marker in props and flash MUST be decoded to its integer before any
assertion or reader sees it.
Falsifier: `missing_flash("toast")` passes with `toast` flashed; `to_page()` carries `encryptHistory` for a page without it, or lacks `clearHistory` for one with it; `inertia_props(None)` is not the props object, or `inertia_props("user.name")` is not the value; after a redirect that flashed `toast`, `assert_inertia_flash("toast", ..)` fails with the store attached, or `assert_inertia_flash_missing("toast")` passes; or with `preserveBigIntegers` on, `where_("id", 9007199254740993_i64)` fails against the marker.
Mechanism: `par-inertia-testing`.
Rationale: Rows TS-12, TS-13, TS-14, TS-15 and TS-16; Laravel's `toArray`, `inertiaPage`, `inertiaProps`, `assertInertiaFlash` and the marker decoding in `fromTestResponse`.
Status: Agreed 2026-10-08

[PAR-068] `suprnova generate-types` MUST write, beside the props
interfaces, a `Pages` interface mapping each component name to the props
interface the handler renders it with, read from `inertia_response!` and
`InertiaResponse::new` with a typed props struct; a `SharedProps`
interface holding the framework's `root: string` and the fields of the
struct `Inertia::share_data` is given or of the one marked
`#[inertia_props(shared)]`; `Errors` as `Record<string, string>`, or
`Record<string, string[]>` when the project turns `with_all_errors(true)`
on (a call under `src/`, read as `preserve_big_integers` is);
`PageProps<C extends keyof Pages>` as `Pages[C] & SharedProps & { errors:
Errors }`; and a `declare module '@inertiajs/core'` augmentation setting
`sharedPageProps` to `SharedProps`, `errorValueType` to `string`, or to
`string[]` under `with_all_errors(true)`, and `flashDataType` to the
struct marked `#[inertia_props(flash)]` when one exists; when a file
under `frontend/src` other than the generated one already declares that
module, the generator MUST write no augmentation and say so, naming the
file, so a project with a hand-written one upgrades without a conflicting
merge. The `InertiaProps` derive MUST accept those two markers, and the
starter kits MUST declare `@inertiajs/core`, ship the generated file in
that shape, and read `root` through the augmentation.
Falsifier: a project rendering `Home` with `HomeProps` gets no `Pages` entry `Home: HomeProps`; a project with its own `declare module '@inertiajs/core'` in `frontend/src/global.d.ts` gets a second augmentation in the generated file, or gets none without a notice naming that file; `SharedProps` lacks `root` or a field of the struct `Inertia::share_data` is given; the augmentation is missing or names another `sharedPageProps`; with `.with_all_errors(true)` under `src/` the `errorValueType` is `string` or `Errors` is not `Record<string, string[]>`, or without it either is the array form; a struct marked `#[inertia_props(flash)]` is not the `flashDataType`; `#[inertia_props(shared)]` fails to compile; a kit's `package.json` lacks `@inertiajs/core`; a kit page types `root` by hand; or a kit's `inertia-props.ts` differs from the generator's output for the kit.
Mechanism: `par-inertia-testing`.
Rationale: Rows T01, T02 and T06; Inertia's `Page<SharedProps>.props` is `PageProps & SharedProps & { errors }` and its `InertiaConfig` merging types `usePage()`, the kits reach `@inertiajs/core` today only by hoisting, the struct form of sharing is `Inertia::share_data` (the third round's wording said `Inertia::share`, whose arguments are a key and a value), `with_all_errors(true)` sends every message per field as an array, and a project upgrading with its own augmentation would otherwise merge two declarations of one key (the developer's heads-up of 2026-10-08 11:10).
Status: Agreed 2026-10-08

[PAR-069] The generator MUST map `i64`, `u64`, `i128`, `u128`, `isize` and
`usize` to `number | bigint` when the project preserves big integers (a
`preserve_big_integers(true)` call under `src/`, or `--big-integers`), and
to `number` otherwise; narrower integers and floats MUST stay `number`.
Falsifier: with `.preserve_big_integers(true)` in `src/bootstrap.rs` an `i64` field is `number`; without it or the flag an `i64` field is `number | bigint`; or an `i32` or `f64` field is anything but `number`.
Mechanism: `par-inertia-testing`.
Rationale: Row T03; with `preserveBigIntegers` the client restores a wide integer as a `BigInt`, so the type follows the transport.
Status: Agreed 2026-10-08

[PAR-070] A generated route helper MUST carry `component` in its
`UrlMethodPair` when the route's handler renders exactly one Inertia
component, named in `inertia_response!`, `InertiaResponse::new` or
`Router::inertia`, and omit it otherwise; `RouteConfig` MUST declare
`component?: string`.
Falsifier: the helper of a handler calling `inertia_response!(&req, "Users/Index", ..)` lacks `component: 'Users/Index'`; a handler naming two components gets one of them; a `Router::inertia("/about", "About", ..)` route's helper lacks `About`; or `RouteConfig` has no `component`.
Mechanism: `par-inertia-testing`.
Rationale: Row T05; Inertia's `UrlMethodPair.component` lets an instant visit resolve the page without a round trip, which the helper can name since the handler's component is a literal.
Status: Agreed 2026-10-08

## Inertia DevTools server support

The fourth parity round the developer ordered on 2026-10-07 (17:35), ruled
build at 17:23 ("roll back my ruling... Build it"): the server side of the
Inertia DevTools browser extension, inertia-laravel 3.5.1's `DevTools`
namespace (rows DT-01 to DT-10) and the Precognition request type row
of the Precognition group (row 057), about 2,700 lines of PHP upstream. References cite
`reference/inertia-laravel-3.5.1/src/DevTools/`. The round also carries
the third round's three next-feature items: PAR-061 revised so the CLI
delegates to the application binary (the developer, 2026-10-08 10:58), and
PAR-068 revised to name `Inertia::share_data` and to type the errors under
`with_all_errors(true)`.

[PAR-071] DevTools recording MUST be gated by `InertiaConfig::devtools`:
an application that never calls `InertiaConfig::devtools(..)` records
nothing, adds no header or tag and leaves the devtools paths to its own
routing, as Laravel records only once its package is installed (the
developer, 2026-10-09 05:58; the scaffold and the dogfood app make the
call); once called, `enabled` unset records only when `APP_ENV` is set
and names `local`
(an unset `APP_ENV`, which the framework otherwise reads as local, does
not: the developer, 2026-10-08 14:25), `true` and `false` decide
outright; a request whose path matches one of the `except`
patterns (Laravel's `Request::is` rule: `*` matches any characters, the
leading slash dropped; default `_inertia/devtools*` and `_suprnova/*`) is
not recorded. Recording MUST never change the response the request gets:
a failure anywhere in recording (an unserializable value, a misconfigured
redaction list, a storage error) is swallowed and the entry dropped, a
storage write failure is logged once at warn and suppresses recording for
30 seconds, and with devtools off no header, tag or entry is produced.
Falsifier: an application without a `devtools(..)` call leaves an entry, adds a header or tag or answers `/_inertia/devtools/entries` itself under `APP_ENV=local`; with `enabled` unset after the call a request under `APP_ENV=local` leaves no entry or one under an unset or production `APP_ENV` leaves one; with `enabled(false)` a request under `APP_ENV=local` leaves an entry, or with `enabled(true)` one under a production `APP_ENV` leaves none; a request to `/_inertia/devtools/entries` or `/_suprnova/health` is recorded; a request under an `except` pattern of its own is recorded; a prop value that cannot be serialized, or a storage path that cannot be written, changes the status, body or headers of the response beyond the devtools headers; or a write failure is logged on every request.
Mechanism: `par-inertia-devtools`.
Rationale: Rows DT-01 and DT-10; Laravel's `DevTools::enabled` defaults to the `local` environment, `devtools.except` skips its own and other tooling's paths, and `RequestRecorder::respondedWith` swallows every failure so a passive observer cannot turn the user's response into a 500.
Status: Agreed 2026-10-09

[PAR-072] Every recorded request MUST produce one entry: a ULID `id`; the
`tabUuid`, `batchId` and `visitId` read from the `X-Inertia-Devtools-Tab`,
`-Parent` (an Inertia request's only) and `-Visit` request headers; the
UTC timestamp; the method and full URL; the status; the redirect
location (`X-Inertia-Location`, else `Location` on a 3xx); the server
time in milliseconds; the request type, in this precedence: a request
with a `Precognition` header is `precognition`; one without `X-Inertia`
is `initial` when it rendered an Inertia page, else `http`; then
`X-Inertia-Devtools-Deferred` is `deferred`, `X-Inertia-Devtools-Poll` is
`poll`, `X-Inertia-Partial-Component` is `partial`, a prefetch (`Purpose`
or `Sec-Purpose` of `prefetch`) is `prefetch`, else `navigate`; the
request and response headers; the request body (omitted with reason
`non-inertia-request` for a non-Inertia write, else the JSON or form
input with uploads summarized as name, size and MIME type, else the raw
text, `empty` when none, `binary` when not UTF-8); the response body (the
page object for an Inertia render, else a textual body up to 256,000
bytes, omitted with reason `non-textual`, `streamed` or `too-large`
otherwise); the component, its page file when found, the route (name,
pattern, handler name) and the render source (the file and line of the
render call). Every recorded response MUST carry `X-Inertia-Devtools-Id`
(the entry id) and `X-Inertia-Devtools-Parent-Out` (the incoming parent
of an Inertia request, else the entry id; a prefetch's own id), and
`X-Inertia-Devtools-Base-Path` when the public root is not empty; a `200`
HTML response that rendered a page for a non-Inertia request MUST carry
`<script data-inertia-devtools-id type="application/json">"<id>"</script>`
before `</body>`, with `data-inertia-devtools-base-path` when the root is
not empty.
Falsifier: two recorded requests share an id, or an id is not a ULID; a request with `Precognition: true` and `X-Inertia: true` is recorded as anything but `precognition`; a plain `GET` that rendered a page is not `initial`, or one that rendered none is not `http`; a partial reload with `X-Inertia-Devtools-Deferred` is `partial`; a `Purpose: prefetch` visit is `navigate`; the entry of a non-Inertia `POST` carries its body; an entry lacks the request headers, the status, the component of a rendered page or the render call's file; a recorded response lacks `X-Inertia-Devtools-Id` or `-Parent-Out`, or carries `-Base-Path` at the host root; `-Parent-Out` of an Inertia request with `X-Inertia-Devtools-Parent: p` is not `p`; or a first visit's `200` document lacks the id tag, or an Inertia visit's JSON carries one.
Mechanism: `par-inertia-devtools`.
Rationale: Rows DT-02, DT-04 and the Precognition group's row 057; Laravel's `IncomingEntryBuilder`, `RequestRecorder::recordResponse` and `DevToolsHeader`, which the extension reads to attach entries to tabs, batches and visits.
Status: Agreed 2026-10-08

[PAR-073] An entry of a rendered page MUST classify each resolved prop
path: `inertiaType` one of `always`, `defer` (a deferred prop delivered
on an `X-Inertia-Devtools-Deferred` request, with its `deferGroup`),
`optional`, `merge`, `scroll` or `once`, in that precedence for a prop
whose flags compose, or none; `reset` when the path is in
`X-Inertia-Reset`; `once`; `mergeDirection` `append` or `prepend`;
`deepMerge` for a deep merge or a `match_on`; `rescued` for a deferred
resolver that failed and was rescued; `shared` for a top-level key the
shared props supplied, with its `shareSource` (the file and line of the
`Inertia::share`, `share_data` or `share_once` call, or of the hook, that
supplied it) and, for a render prop, its `renderSource`. Deep paths with
no metadata MUST be pruned while every top-level key is kept, and
`propValues` MUST hold the value of each kept path as the client received
it, redacted.
Falsifier: `Inertia::always` is not `always`; a deferred prop delivered on an `X-Inertia-Devtools-Deferred` reload is not `defer` with its group, or one delivered on a manual partial reload is `defer`; a prepend merge has `mergeDirection: append`; a `match_on` prop lacks `deepMerge`; a key `Inertia::share` supplied lacks `shared` or a `shareSource` naming the file of the call; a path in `X-Inertia-Reset` lacks `reset`; a rescued deferred prop lacks `rescued`; a nested path with no metadata survives, or a top-level key is pruned; or `propValues` carries a value the client did not receive.
Mechanism: `par-inertia-devtools`.
Rationale: Row DT-03; Laravel's `PropClassifier` and `Collector`, which the extension renders as type pills and source links; Suprnova's `Prop` composes the flags one PHP class each carries, so the precedence picks the pill.
Status: Agreed 2026-10-08

[PAR-074] With devtools enabled the application MUST answer
`GET /_inertia/devtools/entries` with the entries' metadata newest first,
filtered by `component`, `type` and `exclude` (comma lists of request
types), `offset` and `limit`, and `GET /_inertia/devtools/entries/{id}`
with the stored entry, `404 {"message": "Not found."}` for an id that is
not a ULID or names no entry; a request is allowed always when
`APP_ENV` is set and names `local` (never for an unset `APP_ENV`: the
developer, 2026-10-08 14:25) and elsewhere only when the configured
`devtools.gate` ability allows the request's user (a guest when none),
else `403 {"message": "Forbidden."}`; the endpoints MUST run inside the
Inertia middleware stack with the session in scope, reflash the session,
count as XHR so they never become the previous URL, and never be
recorded.
Falsifier: `entries` lists oldest first or ignores `component=Home`, `type=navigate,partial`, `exclude=poll`, `offset` or `limit`; `entries/not-a-ulid` or an unknown ULID answers anything but `404 {"message": "Not found."}`; a request under an unset or production `APP_ENV` with no gate configured, or one the gate denies, answers anything but `403 {"message": "Forbidden."}`, or a `Local` request is denied; an entry request ages the flash a `POST` left, or becomes the previous URL; or an entry request leaves an entry.
Mechanism: `par-inertia-devtools`.
Rationale: Rows DT-05, DT-06 and DT-09; Laravel's `EntriesController`, `Authorize`, `PreserveFlashData` and `PreventPreviousUrlTracking`, since the extension fetches an entry the moment the headers arrive, racing the redirect the app is about to follow.
Status: Agreed 2026-10-08

[PAR-075] Entries MUST be stored one JSON file each under
`devtools.storage.path` (default `storage_path("inertia-devtools")`) with
an index of their metadata; entries older than `storage.ttl` hours
(default 24) MUST be pruned when a request ends and the last prune is
older than `storage.prune_interval` seconds (default 300); a tab MUST
keep its newest `storage.limit` entries (default 100, `0` unlimited).
Before storage the configured `redact.keys` (default `password`,
`password_confirmation`, `current_password`, `token`, `_token`,
`access_token`, `refresh_token`, `secret`, `client_secret`, `api_key`),
`redact.headers` (default `cookie`, `set-cookie`, `authorization`,
`proxy-authorization`, `x-xsrf-token`, `x-csrf-token`) and the URL query
parameters named by the keys MUST be replaced by `[REDACTED]` in headers,
bodies, prop values and URLs, case-insensitively, and a leaf that cannot
be serialized by `[UNSERIALIZABLE]`.
Falsifier: an entry file is missing or unreadable as JSON, or the index does not list it; an entry 25 hours old survives a request after the prune interval, or one 1 hour old is pruned; a tab with 101 entries at the default limit keeps the oldest; a stored entry carries a `password` value, a `Cookie` header's value, or `?token=abc` unredacted, or a `Password` key escapes by case; or an unserializable prop leaf drops the entry.
Mechanism: `par-inertia-devtools`.
Rationale: Rows DT-07 and DT-08; Laravel's `EntriesRepository`, `EntryStore` and `RedactsSensitiveData`, and the extension reads the index for its list and the files for detail.
Status: Agreed 2026-10-08

## Starter kits on Inertia 3.8

The fifth parity round the developer ordered on 2026-10-07 (17:35): the
fifteen build rows of the starter-kit group (K01, K05, K06, K07, K10, K11,
K12, K13, K14, K15, K16, K17, K18, K19 and K21) on Inertia.js 3.8.0. K20
is settled (Laravel's own kits register with Inertia's `Form` and no live
validation) and K10 is met by PAR-068 (the kits declare `@inertiajs/core`
and read `usePage()` typed by the generated augmentation). The rows'
siblings are built: the multipart request bodies of P16 (PAR-05x), the
cursor metadata of O19, the hot file of S07 (PAR-058) and the types of
T02 (PAR-068); the client nonce row H03 stays with RDOC-005 in the
security-headers commitment, and the kits read the nonce element it
describes. Client references cite `reference/inertia-3.8.0/packages/`.

[PAR-076] Every starter kit (`svelte`, `react`, `vue`) MUST declare its
adapter, `@inertiajs/core` and `@inertiajs/vite` at `^3.8.0`; resolve its
pages through the Vite plugin's `pages` shorthand (`pages: './pages'`, no
hand-written `import.meta.glob`); build its SSR entry through the plugin so
`npm run build:ssr` lands `frontend/bootstrap/ssr/ssr.js`, the path
`suprnova ssr:start` reads (PAR-061); and pass `createInertiaApp` a
`title` callback that appends the application's name, `serverHead: true`,
and `nonce` read from the document's `<meta property="csp-nonce">` when
one is present (RDOC-005's element; `undefined` on the server and when the
document has none). Each kit MUST have a `check` script that type-checks
it, and `npm run check`, `npm run build` and `npm run build:ssr` MUST pass
on a fresh scaffold. Suprnova's sources, the scaffold's comments and the
manual MUST cite the Inertia.js client at 3.8.0, not 3.6.1.
Falsifier: a kit manifest names an `@inertiajs/*` package below `^3.8.0` or lacks `@inertiajs/vite`; an entry resolves pages with its own glob, or passes no `title`, `serverHead` or `nonce`; `npm run check`, `npm run build` or `npm run build:ssr` fails on a fresh scaffold, or the SSR bundle lands anywhere but `frontend/bootstrap/ssr/ssr.js`; a tracked source, template or manual line cites `inertia-3.6.1` or pins `^3.6.1`.
Mechanism: `par-starter-kits`.
Rationale: Rows K01, K05, K18 and K19; Inertia 3.8.0 is current, its Vite plugin (`packages/vite/src/index.ts`) owns page resolution and the development SSR endpoint PAR-058's hot file already sends first visits to, and `createInertiaApp`'s `nonce`, `serverHead` and `title` options (`packages/core/src/types.ts`) are what a page needs under a CSP, for head tags set from Rust, and for a titled tab.
Status: Agreed 2026-10-08

[PAR-077] Every in-application navigation in a kit page or layout MUST be
a `Link` (an external URL stays an anchor), the sign-out a `Link` with
`method="post"` rendered `as="button"`, and the users list's rows MUST
prefetch on hover; every page MUST set its title with `Head`; each kit
MUST ship a guest layout for the pages under `auth/` and an application
layout (navigation, the signed-in user, the flash toast) for every other
page, both applied through `createInertiaApp`'s `layout` option so a
layout keeps its state across visits between its pages, and the dashboard
MUST set the layout's heading with `setLayoutProps`, which the next page
that sets none does not show.
Falsifier: a kit page or layout reaches an application route through an `<a href>`, or a page renders without `Head`; a page outside `auth/` renders without the application layout, or the layout remounts between two application pages so state kept in it resets; the dashboard's heading set with `setLayoutProps` does not show, or shows on the next page.
Mechanism: `par-starter-kits`.
Rationale: Rows K06, K07 and K16; the adapters' `Link` and `Head` and the `layout` option with `setLayoutProps` (`packages/core/src/layout.ts`) keep a new application's chrome mounted and its tab titled, where the kits' anchors reload the document on every click and each page renders its own frame.
Status: Agreed 2026-10-08

[PAR-078] The scaffold MUST declare a flash struct marked
`#[inertia_props(flash)]` carrying a `toast` with a kind and a message,
flash one through `Inertia::flash` after sign-in, registration, a
password-reset link request, a password reset, email verification, a
verification resend and sign-out, and each kit's layouts MUST show
`page.flash`'s toast once, typed by the generated `flashDataType`.
Falsifier: a sign-in, registration, reset request, reset, verification, resend or sign-out shows no toast on the page it lands on, or the toast shows again on the next visit; the flash struct lacks the marker, so `usePage().flash` is untyped.
Mechanism: `par-starter-kits`.
Rationale: Row K17; the server sends `page.flash` and the generator types it (PAR-068), but no kit page read it, so a new application showed no feedback after any action.
Status: Agreed 2026-10-08

[PAR-079] Each kit's dashboard MUST render a `stats` prop deferred, with
`Deferred` and a fallback; a `recent_notes` prop marked optional, loaded
with `WhenVisible` when scrolled into view; poll `stats` with `usePoll` on
`only: ['stats']`; and let the signed-in user change their display name
through `useHttp` with an optimistic update, the new name showing before
the server answers and a 422 reverting it and showing the field's error.
A `Notes/Index` page MUST list the signed-in user's own notes with
`InfiniteScroll` over `Inertia::paginate` of a cursor paginator, remember
its `search` filter with `useRemember`, create a note through the `Form`
component, and open a note through an instant visit (a `Link` naming
`component="Notes/Show"` and `pageProps` from the row, so the show page
renders from the row before the server answers). Every auth page (Login,
Register, ForgotPassword, ResetPassword and the VerifyEmail resend) MUST
submit through the `Form` component, showing its `errors` and disabling
its button while `processing`. No kit page MUST list or show another
user's account or note.
Falsifier: a kit dashboard or notes page lacks one of `Deferred`, `WhenVisible`, `usePoll`, `useHttp`, `InfiniteScroll`, `useRemember`, `Form` or an instant-visit `Link`; the name change waits for the server before showing, or keeps the new name after a 422; the notes list requests page numbers instead of cursors; an auth page submits with `useForm` or a native submit handler; a kit page lists other users or shows another user's note.
Mechanism: `par-starter-kits`.
Rationale: Rows K11, K12, K13, K14, K15 and K21; the server already resolves deferred, optional, merge and scroll props and sends cursor metadata, and its 422 body carries `errors`, but no kit page used the client components and hooks that consume them, so a new application had nothing to copy. The list is the signed-in user's own notes: Laravel's kits ship no list page, and a directory of accounts would hand every member every other member's name and email.
Status: Agreed 2026-10-08

[PAR-080] The scaffold's backend MUST ship what those pages need, behind
the same authentication as the dashboard: a `notes` table and model owned
by a user; the dashboard handler sending the signed-in user, `stats`
deferred (counts of the user's own notes) and `recent_notes` optional (the
user's five newest); a notes index with `cursor_paginate` over the user's
own notes filtered by a `search` query parameter through
`Inertia::paginate`; a notes show that answers `404` for a note of another
user; a notes store validating `title` and flashing a toast; a `POST
/profile/name` JSON handler answering `200` with the user or `422` with
`errors` through the framework's validation; and a fresh scaffold MUST
compile against the framework it pins.
Falsifier: a fresh scaffold fails `cargo check`; `GET /notes` answers without cursor `scrollProps`, lists another user's note, or a second page is requested by number; `GET /notes/{id}` of another user's note answers anything but `404`; `POST /profile/name` with an empty name answers anything but `422` with `errors.name`; a signed-out request to `/notes` or `/profile/name` is not sent to the sign-in page.
Mechanism: `par-starter-kits`.
Rationale: Rows K12, K14 and K21 need a server the page can scroll, post JSON to and update; the scaffold's dashboard handler sent one eager prop and nothing paginated, and the data a new application already has is its own users, which no member should browse.
Status: Agreed 2026-10-08

## Precognition

Laravel Precognition 2.0.0 over framework 13.35.0: live validation through
the official clients, which read a server that marks every answer, skips
handler bodies and validates only the fields a request asks about.

[PAR-081] A `Precognitive` route middleware MUST be the opt-in for
Precognition: every response from a route or group carrying it MUST have
`Precognition` joined to its `Vary` header (appended to an existing value),
precognitive or not; a request whose `Precognition` header names `true`
(any letter case) MUST be marked precognitive by the middleware, and every
response to a marked request MUST carry `Precognition: true`, whatever
produced it (a `401`, `403`, `404`, `409` or `423` from a later middleware
or an extractor included). A route without the middleware MUST ignore the
header and run as a real request. `Request::is_precognitive()` (marked by
the middleware) and `Request::is_attempting_precognition()` (the header
alone) MUST be public, so middleware and rules can read them.
Falsifier: a response from a route carrying the middleware lacks `Precognition` in `Vary`, or an existing `Vary` value is replaced instead of joined; a `403` or `404` answered to a marked request lacks `Precognition: true`; a route without the middleware answers `204` to a request carrying the header, or a form request on it validates only the listed fields; `is_precognitive()` is true on a route without the middleware, or `is_attempting_precognition()` is false while the header names `TRUE`.
Mechanism: `par-precognition`.
Rationale: Rows 002, 003, 006, 028, 029, 040 and 043 of the Precognition group; Laravel's `HandlePrecognitiveRequests` sets `Vary` on every response of the route and `Precognition: true` on a precognitive one, and the official client throws `Did not receive a Precognition response` on any answer without the header, so its `onForbidden` and `onNotFound` handlers never fire against a server that marks only its own `204` and `422`; row 001 keeps the case-insensitive header read.
Status: Agreed 2026-10-08

[PAR-082] On a marked request the framework MUST run the route's
extractors (route parameters and bindings resolve, form requests
validate) and then answer `204` with `Precognition-Success: true` without
calling the handler body, a closure or a method alike. One code path MUST
answer every precognitive form request: a form request bound through a
route parameter MUST check the content type and mark its parse-failure
`422` as the body-bound path does. A `Precognition::precognitive` helper
MUST run the closure it is given, answer the response the closure bails
with (its precognition-specific response on a marked request, its default
otherwise), and otherwise answer `204` with `Precognition-Success: true`
on a marked request and the closure's value on any other.
Falsifier: a handler body runs on a marked request (a row is written, a mail is queued, a counter moves); a marked request whose form request passes answers anything but `204` with `Precognition-Success: true`; a route-parameter-bound form request on a marked request answers a `422` without `Precognition: true`, or accepts a body of an unsupported content type; the helper's bail answers the default response on a marked request that has a precognition-specific one, or the helper returns the closure's value instead of `204` on a marked request.
Mechanism: `par-precognition`.
Rationale: Rows 021, 022, 026 and 027 of the Precognition group; Laravel's `PrecognitionControllerDispatcher` and `PrecognitionCallableDispatcher` resolve parameters and abort `204`, so a live validation request never runs a side effect, and `precognitive()` (`Foundation/helpers.php`) is the same shape for code outside the dispatch; Suprnova ran every handler whose extractors passed.
Status: Agreed 2026-10-08

[PAR-083] `Precognition-Validate-Only` MUST narrow the rules that run, not
the errors reported: on a marked request only the listed fields' rules run
through every stage (the derived rules, the synchronous hook and the
asynchronous hook), so a listed field's database rule runs even when an
unlisted field fails or does not parse; a listed name MUST match a field
exactly, `*` standing for one non-empty segment (`tags.*` covers `tags.3`,
`tags` does not); an empty header value MUST validate nothing and answer
`204`; errors an after-validation hook adds MUST never be filtered, so any
hook error answers `422`; and `Request::validate_only()` MUST expose the
parsed list with `Request::should_validate(field)` applying the match, so
a handler or rule can narrow its own checks.
Falsifier: a listed field whose asynchronous rule fails answers `204` because an unlisted field failed an earlier stage; a parse failure on an unlisted field answers `422` for a request whose listed fields parse and pass; `tags` in the list keeps an error on `tags.3`, or `tags.*` drops one; an empty `Precognition-Validate-Only` answers `422` with every error; a hook error is dropped by the list; `should_validate("tags.3")` disagrees with the match the server applied.
Mechanism: `par-precognition`.
Rationale: Rows 012, 013, 016, 018, 019, 030 and 039 of the Precognition group; Laravel's `filterPrecognitiveRules` removes unlisted attributes' rules before the validator runs and matches `^name$` with `*` as `[^.]+`, the official client sends an empty list when nothing is touched and clears errors by exact key, and `Precognition::afterValidationHook` answers `204` only when the message bag is empty after the hooks; Suprnova ran every stage with bail-on-first and filtered afterwards by prefix, answering `204` for a field whose check never ran; row 017 keeps the trimmed split.
Status: Agreed 2026-10-08

[PAR-084] Live validation MUST read the data the official client sends:
a precognitive `GET` or `DELETE` MUST validate its query parameters
(`key[]` as a list, a JSON-encoded object as the object), a precognitive
multipart request MUST validate its files and fields through the same
path as a JSON body, and validation a handler or middleware runs inline
through `Request::validate::<T>()` MUST apply the same narrowing and
answer `204` or `422` with the Precognition headers on a marked request.
Falsifier: a precognitive `GET` with `?email=` answers `204` while the rule on `email` fails; a precognitive `multipart/form-data` request answers `415`, or reaches a handler with a file the rules reject; `Request::validate::<T>()` on a marked request answers a `422` without `Precognition: true`, validates an unlisted field, or returns the typed value so the handler continues.
Mechanism: `par-precognition`.
Rationale: Rows 031, 032, 033, 047 and 049 of the Precognition group; Laravel's `validationData()` is `all()`, query merged with body and files included, and the client sends `GET` and `DELETE` data in the query and multipart with `validateFiles()`; Suprnova's form request read only the body.
Status: Agreed 2026-10-08

[PAR-085] A marked request MUST NOT save the session (flash data is
neither aged nor consumed, no session row is written) and MUST NOT become
the previous URL, in the session middleware and in the Inertia
middleware's previous-URL store alike.
Falsifier: a toast flashed before a live validation request is gone from the next page; the session's `updated_at` or payload changes across a marked request that changed nothing; `redirect back` after a marked `GET` lands on the live validation URL.
Mechanism: `par-precognition`.
Rationale: Rows 034 and 035 of the Precognition group; Laravel's `StartSession` skips the save and the previous URL for a precognitive request, and inertia-laravel's `shouldStoreCurrentUrl` excludes it; a background validation request that ages flash data uses up the message meant for the next page.
Status: Agreed 2026-10-08

[PAR-086] The `422` of a precognitive request and of a real one MUST carry
`{ "message": ..., "errors": { field: [messages] } }` where `message` is
the first error's message followed by ` (and N more errors)` when N is
greater than one, ` (and 1 more error)` when it is one, and nothing when
it is the only error.
Falsifier: a `422` with three errors carries a fixed banner, `(and 2 more error)`, or no count; a `422` with one error carries a count.
Mechanism: `par-precognition`.
Rationale: Row 009 of the Precognition group; Laravel's `ValidationException::summarize` builds the message, and the JSON shape is part of a drop-in even though the official client reads only `errors`.
Status: Agreed 2026-10-08

[PAR-087] The test client MUST offer `with_precognition()`, setting
`Precognition: true` on the request, and the test response
`assert_successful_precognition()`, asserting `204` with
`Precognition-Success: true`.
Falsifier: `with_precognition()` sends no `Precognition` header; `assert_successful_precognition()` passes on a `204` without `Precognition-Success: true`, or on a `200`.
Mechanism: `par-precognition`.
Rationale: Rows 036 and 037 of the Precognition group; Laravel's `MakesHttpRequests::withPrecognition` and `TestResponse::assertSuccessfulPrecognition`.
Status: Agreed 2026-10-08

[PAR-088] The manual MUST have a Precognition chapter linked from the
table of contents: the `Precognitive` middleware, live validation with the
Vue, React and Svelte clients and Inertia's `withPrecognition`, client
configuration, arrays and wildcards, rules that differ for live validation
through `is_precognitive()`, file uploads, side effects, the session rule
and testing; `manual/validation.md` MUST no longer list unlisted parse
failures as a divergence.
Falsifier: `manual/documentation.md` links no Precognition chapter; the chapter lacks a section on one of the listed topics; `manual/validation.md` still says an unlisted malformed field blocks a live validation answer.
Mechanism: `par-precognition`.
Rationale: Rows 061 and 063 of the Precognition group; Laravel's `precognition.md` covers these and Suprnova's manual had only scattered mentions.
Status: Agreed 2026-10-08

## Laravel 13.35.0 API delta

The members Laravel 13.35.0 added or changed since the parity program's
baseline that the review ruled build: the thirty-seven delta rows of the
parity log, grouped by area.

[PAR-089] `Schedule::always_on_one_server()` MUST make every task of the
schedule run on one server, as each task's `on_one_server` does, unless a
task opts out with `on_every_server()`. A `schedule:interrupt` command MUST
record an interrupt mark in the cache, `Schedule::has_been_interrupted_since(when)`
MUST answer whether a mark newer than `when` exists, and a schedule run
MUST consult it before starting each task after the first, stopping the
run when interrupted, as Laravel's `ScheduleRunCommand` does between
repeats; the mark MUST be cleared when the next run starts.
Falsifier: with `always_on_one_server()` two schedulers on the same cache store both run a task in the same minute, or a task marked `on_every_server()` runs on one only; `schedule:interrupt` leaves `has_been_interrupted_since(start)` false, or a run started after the interrupt does not clear it; or a run that is interrupted after its first task starts its second.
Mechanism: `par-laravel-delta`.
Rationale: Rows `Schedule::$alwaysOnOneServer`, `Schedule::alwaysOnOneServer`, `Schedule::hasBeenInterruptedSince` and the two facade spellings of the Laravel 13.35.0 delta; Laravel's `Schedule::alwaysOnOneServer`, its `schedule:interrupt` command and `ScheduleRunCommand`'s interrupt check.
Status: Agreed 2026-10-09

[PAR-090] `chunk_by_id` MUST take its cursor from the stored key as the
database returned it, not from the value after the model's casts, so a cast
primary key never produces a cursor that misses or repeats rows.
`DB::with_default_connection(name, future)` MUST run the future with that
connection as the default for every `DB` call and model query inside it,
scoped to the task so a concurrent request keeps its own default, and
`DB::default_connection()` MUST name the connection in force.
Falsifier: a model whose key has a cast chunks by id and a row is missed or seen twice; inside `with_default_connection("reporting", ..)` a `DB::table` query or a model query reaches the default connection, or a concurrent task's query reaches `reporting`; or `default_connection()` names the wrong one inside or after the scope.
Mechanism: `par-laravel-delta`.
Rationale: Rows `BuildsQueries::getLastIdFromChunk` and `DB::setDefaultConnection` of the delta; 13.35.0 reads the raw stored key in `getLastIdFromChunk`, and a process-wide default switch would leak across concurrent requests in a Rust server, so the scope is the task.
Status: Agreed 2026-10-09

[PAR-091] `#[model]` MUST accept a `defaults` declaration naming attribute
values a new instance starts with: `Model::new()` and `create` with
partial attributes MUST fill every declared default the caller did not
give, before the attributes are read or saved, and MUST leave a given
attribute as given; a model without the declaration MUST behave as it
does today.
Falsifier: a model declaring `defaults(status = "draft", votes = 0)` created with only a `title` saves another `status` or `votes`, or created with `status = "live"` saves `draft`; `Model::new()` reads `status` as anything but `draft` before a save; or a model without the declaration changes its construction.
Mechanism: `par-laravel-delta`.
Rationale: Rows `HasDefaultAttributes`, `HasDefaultAttributes::defaults` and `HasAttributes::mergeDefaultAttributes` of the delta; Laravel 13.35.0's `HasDefaultAttributes` merges `defaults()` into a new model's attributes.
Status: Agreed 2026-10-09

[PAR-092] Relations MUST offer the keyed and chunked operations Laravel
13.35.0 added: `chunk_map(size, closure)` on belongs-to-many and the
through relations, running the closure per related record in chunks of
`size` and returning the mapped values in order; `first_or_create` and
`increment_or_create(attributes, column, default, step, extra)` on has-one,
has-many and belongs-to-many relations, creating the related record with
the relation's keys set (and the pivot row attached for belongs-to-many)
or incrementing `column` by `step` on the record found; `find_or_new(key)`
on the through relations, searching the records the relation reaches and
returning a new unsaved instance when none matches; and `is(model)` on
has-one-through, answering with one query whether the relation's target is
that model.
Falsifier: `chunk_map` on a belongs-to-many of 25 records with size 10 calls the closure other than 25 times or returns values out of order; `increment_or_create` on a has-many finds no record and creates one without the foreign key, or finds one and leaves `column` unchanged or changes it by other than `step`; `first_or_create` on a belongs-to-many creates the record without attaching it; `find_or_new` on a through relation returns a record the relation does not reach, or a saved instance when none matches; or `is` loads the target instead of asking the database once, or answers true for another model.
Mechanism: `par-laravel-delta`.
Rationale: Rows `BelongsToMany::chunkMap`, `BelongsToMany::incrementOrCreate`, `HasOneOrMany::incrementOrCreate`, `HasOneOrManyThrough::chunkMap`, `HasOneOrManyThrough::findOrNew` and `HasOneThrough::is` of the delta.
Status: Agreed 2026-10-09

[PAR-093] `queue:work` MUST accept Laravel's worker controls: `--sleep`
(seconds to pause when no job is available), `--tries`, `--timeout`,
`--once` (process one job and exit), `--stop-when-empty` and `--memory`
(megabytes; the worker exits with status 12 once its resident memory
exceeds it, so a supervisor restarts it), beside its existing options.
The queue fake MUST offer `assert_not_pushed::<J>(closure)` and a closure
filter on `assert_pushed_on_queue`. `Bus::dispatch_after_response(command)`
MUST run the command after the response has been sent, and the bus fake
MUST offer `dispatched::<C>()`, `dispatched_sync::<C>()` and
`dispatched_after_response::<C>()`, each returning the captured commands
of that type for the iterator to filter.
Falsifier: `queue:work --once` processes a second job; `--stop-when-empty` keeps the worker running on an empty queue; `--memory=1` on a worker whose resident memory is above one megabyte does not exit with status 12 after a job; `--sleep=3` polls an empty queue more often than every three seconds; `assert_not_pushed::<J>(|j| j.id == 7)` passes when such a job was pushed, or `assert_pushed_on_queue::<J>("emails", |j| j.id == 7)` passes when only another job of the type reached that queue; a command dispatched after the response runs before the response's bytes are sent, or is missing from `dispatched_after_response::<C>()`; or `dispatched::<C>()` misses a dispatched command.
Mechanism: `par-laravel-delta`.
Rationale: Rows `WorkCommand::$signature`, `artisan queue:work`, `Queue::assertNotPushed`, `Queue::assertPushedOn`, `Bus::dispatched`, `Bus::dispatchedSync` and `Bus::dispatchedAfterResponse` of the delta; Laravel's `queue:work` options, its exit status 12 on the memory limit, and `BusFake`'s accessors.
Status: Agreed 2026-10-09

[PAR-094] `HttpResponse::markdown(body)` and the `markdown` helper MUST
build a `200` response with `Content-Type: text/markdown; charset=utf-8`
and the body as given, as `text` and `html` build theirs.
Falsifier: `markdown("# Hi")` answers another content type or charset, another status, or a changed body.
Mechanism: `par-laravel-delta`.
Rationale: Rows `ResponseFactory::markdown` and `Response::markdown` of the delta; Laravel 13.35.0 added the constructor.
Status: Agreed 2026-10-09

[PAR-095] The router MUST accept the `QUERY` method: `query!(path,
handler)` registers a route for it, a `QUERY` request to a route
registered with `any!` MUST reach it, and `QUERY` MUST be listed where the
router names the methods it accepts, so a client sending `QUERY` is not
refused at the router.
Falsifier: a `QUERY` request to a `query!` route answers `405` or `404`; a `QUERY` request to an `any!` route is refused; or `query!` is missing from the routing macros or the manual's method list.
Mechanism: `par-laravel-delta`.
Rationale: Rows `Router::$verbs`, `Router::query` and `Route::query` of the delta; Laravel 13.35.0 added `QUERY` to `Router::$verbs` and `Route::query`.
Status: Agreed 2026-10-09

[PAR-096] `Auth::logout_other_devices(password)` MUST verify the password
against the current user, and on success invalidate every other session
of that user and every remember token but the current session's, leaving
the current session signed in; a wrong password MUST leave every session
as it is and answer a validation failure on `password`.
Falsifier: after `logout_other_devices` with the right password another session of the user still answers as signed in, or the current session is signed out; with a wrong password another session is signed out, or the answer is not a validation failure on `password`.
Mechanism: `par-laravel-delta`.
Rationale: Row `Auth::logoutOtherDevices` of the delta; Laravel's `SessionGuard::logoutOtherDevices` keeps the current session and rehashes the password into the remember cookie, a mechanism Suprnova does not need since its sessions are revoked by id.
Status: Agreed 2026-10-09

[PAR-097] The notification fake MUST offer `sent::<N>(recipient, closure)`,
returning the recorded notifications of that type sent to the recipient
that the closure accepts, `assert_sent_to::<N>(recipient, closure)` and
`assert_not_sent_to::<N>(recipient)`, so a test can assert that one
notification type did or did not reach a recipient and inspect its
contents; the existing untyped assertions MUST keep working.
Falsifier: `assert_not_sent_to::<Invoice>(route)` passes when an `Invoice` reached the route; `assert_sent_to::<Invoice>(route, |n| n.total == 10)` passes when only an `Invoice` with another total reached it; `sent::<Invoice>(route, |_| true)` misses a recorded `Invoice` or returns another type; or an existing `assert_sent_to(route, name)` call stops compiling.
Mechanism: `par-laravel-delta`.
Rationale: Rows `Notification::assertSentTo`, `Notification::assertNotSentTo` and `Notification::sent` of the delta; a typed closure covers Laravel's property-map form, as the event fakes do.
Status: Agreed 2026-10-09

[PAR-098] `Storage` MUST offer `copy_to_disk` and `move_to_disk`, each
taking the source disk and path and the destination disk and path, where a
disk is named by a string or a disk handle, moving by copying and then
deleting the source; `Storage::forget` MUST take one name or a list of
names; `Storage::purge(name)` MUST drop one disk and `Storage::purge_all()`
every disk; and `Storage::set(name, disk)` MUST store a ready-made disk
under a name, replacing any disk of that name.
Falsifier: `move_to_disk("local", "a.txt", "archive", "a.txt")` leaves the source or does not write the destination; `copy_to_disk` with a disk handle as the destination is refused; `forget(["a", "b"])` leaves one registered; `purge("a")` drops another disk, or `purge_all()` leaves one; or a disk stored with `set` is not the one `disk(name)` returns.
Mechanism: `par-laravel-delta`.
Rationale: Rows `Storage::copyToDisk`, `Storage::moveToDisk`, `Storage::forgetDisk`, `Storage::purge` and `Storage::set` of the delta; Laravel 13.35.0's `FilesystemManager` takes a disk name or an enum for each.
Status: Agreed 2026-10-09

## Laravel API gaps: data

The members of Laravel 13.35.0's database, Eloquent, migration, pagination,
factory and seeding surface that the parity review found missing or
differing in Suprnova and ruled build: the forty-five rows of the log's
data areas, grouped by area.

[PAR-099] `first_or_fail` on both builders MUST accept an optional
message and, without one, MUST name the model (or table) in its default
message, the status staying `404`. `DB::transaction` MUST nest: an inner
call inside an open transaction MUST run in a savepoint that its failure
rolls back alone, `DB::transaction_level()` MUST report the depth, and
`DB::after_rollback(closure)` MUST run the closure when the enclosing
transaction rolls back. `DB::before_starting_transaction(listener)` MUST
run each listener before `BEGIN`, and a listener's error MUST stop the
transaction from starting and be the caller's error. `DB::raw(expression)`
MUST be accepted as a value in `update`, as a `group_by` term and in
`having`, written into the SQL unbound and never as a bound parameter;
`select_raw`, `where_raw` and `order_by_raw` keep their bound parameters.
Falsifier: `first_or_fail` with a message answers another text, or its default message lacks the model's name; an inner `DB::transaction` that fails rolls the outer one back, or `transaction_level()` is not 2 inside it; `after_rollback` runs on a commit or not on a rollback; a `before_starting_transaction` listener runs after `BEGIN`, or its error lets the transaction start; or `update` with `DB::raw("views + 1")` binds the expression as a string or refuses it.
Mechanism: `par-laravel-gaps-data`.
Rationale: Rows `BuildsQueries::firstOrFail`, `ManagesTransactions`, `Connection::beforeStartingTransaction` and `DB::raw` of the parity log's data areas; Laravel nests transactions through savepoints, and SeaORM's savepoint support carries it.
Status: Agreed 2026-10-09

[PAR-100] The model query builder MUST offer `apply_scopes()`, returning
the query with every global scope and the soft-delete filter applied, as
the query runs them; `has_morph`, `doesnt_have_morph`, `where_has_morph`
and `where_doesnt_have_morph` over a polymorphic relation, taking the
related model types or `*` for every type including rows with no type,
with an optional constraint closure per type; and `force_destroy` on a
soft-deleting model MUST remove rows that are already soft-deleted.
Falsifier: `apply_scopes()` compiles without a registered scope's clause or without the soft-delete filter; `where_has_morph("commentable", ["Post"], ..)` returns a row whose commentable is another type, or `*` leaves out a row with a null type; or `force_destroy` leaves a soft-deleted row in the table.
Mechanism: `par-laravel-gaps-data`.
Rationale: Rows `Builder::applyScopes`, `QueriesRelationships::doesntHaveMorph` and `SoftDeletes::forceDestroy`.
Status: Agreed 2026-10-09

[PAR-101] A loaded model collection MUST offer `find(key)`, `find_model(&model)`
and `find_many(keys)` by primary key, with `find_or(key, default)`;
`load_with(relation, closure)`, eager loading a relation onto the loaded
models under the closure's constraints; `unique_models()` deduplicating by
primary key and keeping the last copy, as Laravel's `Collection::unique`
does (the general `unique()` keeps its whole-value meaning); and
`diff_models(other)` comparing by primary key.
Falsifier: `find(3)` on a collection holding key 3 returns `None`, or `find_many([1, 3])` misses one; `load_with("posts", |q| q.where("published", true))` loads an unpublished post; `unique_models()` on two loads of the same row keeps the first copy or both; or `diff_models` keeps a model whose key the other collection holds.
Mechanism: `par-laravel-gaps-data`.
Rationale: Rows `Collection::find`, `Collection::load`, `Collection::unique` and `Collection::diff`; Laravel compares models by key, not by value.
Status: Agreed 2026-10-09

[PAR-102] `AsEnumCollection<E>` MUST store a list of enums as the strings
`AsEnum` stores for each, so a column Laravel wrote with
`AsEnumCollection::of` reads back the same; appended attributes MUST honour
the model's hidden and visible lists, and `append(name)` MUST add an
appended attribute at run time; `get_raw_original()` MUST return every
attribute as loaded and `get_raw_original_or(attribute, default)` the
default when the attribute was not loaded.
Falsifier: a JSON column holding `["draft","live"]` written by Laravel does not read into `AsEnumCollection<Status>`, or a write stores anything but those strings; a hidden appended attribute appears in the model's array, or `append("full_name")` does not add it; or `get_raw_original()` misses a loaded attribute, or `get_raw_original_or("missing", 7)` is not 7.
Mechanism: `par-laravel-gaps-data`.
Rationale: Rows `AsEnumCollection::of`, `HasAttributes::$appends` and `HasAttributes::getRawOriginal`.
Status: Agreed 2026-10-09

[PAR-103] The hidden and visible lists MUST apply to every array and JSON
output of a model, appended attributes included and the model's `Serialize`
output as well, and a model MUST offer `make_hidden`, `make_hidden_if`,
`make_visible` and `make_visible_if`, changing the lists for that instance
so every later conversion honours them; declaring both lists on one model
stays refused.
Falsifier: a model with `hidden = ["secret"]` serializes `secret` through `Serialize`, or an appended attribute named in `hidden` appears; after `make_hidden("email")` the next `to_array` still carries `email`, or after `make_visible("secret")` it is still absent; `make_hidden_if(false, ..)` hides; or a model declaring both lists compiles.
Mechanism: `par-laravel-gaps-data`.
Rationale: Rows `HidesAttributes`, `HidesAttributes::$hidden`, `$visible`, `makeHidden` and `makeHiddenIf`; today `to_array_except` filters one output and serde ignores the lists.
Status: Agreed 2026-10-09

[PAR-104] A factory's `count(n)` and `times(n)` MUST make `create` and
`make` produce `n` models as a typed many-result, and `create_many` MUST
take a count or a list of per-record attribute sets, each merged over the
definition, as Laravel's `Factory::count`, `times` and `createMany` do.
Falsifier: `factory.count(3).create()` saves one row, or returns a single model; `times(3).make()` yields other than three; `create_many(2)` saves other than two, or `create_many([attrs_a, attrs_b])` saves records without those attributes.
Mechanism: `par-laravel-gaps-data`.
Rationale: Rows `Factory::count`, `Factory::createMany` and `Factory::times`.
Status: Agreed 2026-10-09

[PAR-105] A pivot model whose table has no `id` MUST be deletable by its
key pair (`detach` by both keys), and a morph pivot by the pair plus the
type; `limit` and `take` on a has-one or has-many relation MUST cap each
parent's related rows during eager loading, not the rows of every parent
together; and a morph-one or morph-many relation MUST offer `create`,
`save` and `upsert` that fill the owner's id and type.
Falsifier: a pivot row on a table without `id` cannot be deleted by its two keys, or a morph pivot by its keys and type; `with("comments", |q| q.limit(2))` over three parents returns two comments in total instead of two per parent; or `post.comments().create(attrs)` on a morph-many saves a row without the owner's id or type.
Mechanism: `par-laravel-gaps-data`.
Rationale: Rows `AsPivot::delete`, `MorphPivot::delete`, `HasOneOrMany::limit`, `HasOneOrMany::take` and `MorphOneOrMany`; Laravel caps eager loads per parent with a window function.
Status: Agreed 2026-10-09

[PAR-106] A resource MUST offer `merge_when(condition, fields)`, merging a
group of fields when the condition holds; the model builder MUST offer
`with_exists(relation)`, loading a boolean existence flag per row without
loading the relation, and a resource `when_exists_loaded(relation, value)`
reading it; and a JSON:API resource collection's `with` meta MUST stay at
the top level without merging the first item's meta, with an
application-wide setting for the `jsonapi` member.
Falsifier: `merge_when(false, ..)` merges, or `merge_when(true, ..)` leaves a field out; `with_exists("comments")` loads the comments or yields no flag, or `when_exists_loaded` reads a flag that was not loaded as present; a collection's top-level meta carries the first item's meta; or the `jsonapi` member cannot be set once for the application.
Mechanism: `par-laravel-gaps-data`.
Rationale: Rows `ConditionallyLoadsAttributes::filter`, `ConditionallyLoadsAttributes::whenExistsLoaded` and `AnonymousResourceCollection::with`.
Status: Agreed 2026-10-09

[PAR-107] The schema builder MUST create what Laravel creates: `boolean`
as `tinyint(1)` on MySQL and SQLite, round-tripping 0 and 1; `float` as
double precision (Laravel's precision 53) unless a precision is given;
`timestamp_tz` as `timestamp(0) with time zone` on Postgres and `datetime`
on SQLite unless a precision is given; and `ulid(name, length)` beside
`ulid(name)`.
Falsifier: `boolean("flag")` declares `boolean` on SQLite, or a stored 1 reads back as anything but true; `float("ratio")` creates a single-precision column on Postgres; `timestamp_tz("at")` keeps fractional seconds on Postgres or declares `text` on SQLite; or `ulid("code", 20)` is refused.
Mechanism: `par-laravel-gaps-data`.
Rationale: Rows `Blueprint::boolean`, `Blueprint::float`, `Blueprint::timestampTz` and `Blueprint::ulid`.
Status: Agreed 2026-10-09

[PAR-108] A cursor paginator MUST key on several columns when the query
orders by several, reading each column's value from the row, with
`Cursor::parameter(name)` for a named lookup; a page MUST offer
`through(transform)`, returning a page of another item type with the same
metadata; and a length-aware page MUST report `last_page` 1 for an empty
result and MUST treat a page number below 1, or not a number, as page 1,
as Laravel's paginator does.
Falsifier: `cursor_paginate` over `order_by("created_at").order_by("id")` builds a cursor without `id`, or `cursor.parameter("id")` is `None`; `page.through(|u| u.name)` loses `total` or `per_page`; `paginate` over no rows reports `last_page: 0`; or `?page=0` or `?page=abc` answers anything but page 1.
Mechanism: `par-laravel-gaps-data`.
Rationale: Rows `AbstractCursorPaginator::getParametersForItem`, `AbstractCursorPaginator::setCollection`, `Cursor::parameter` and `LengthAwarePaginator::__construct`.
Status: Agreed 2026-10-09

[PAR-109] The `DB::table` builder MUST offer `in_random_order()`,
`max`, `oldest`, `where_not_between`, `or_where_between` and
`or_where_not_between` as the model builder does, with a subquery or a
`DB::raw` expression accepted as the column where the model builder
accepts one; `in_random_order_seeded(seed)` on both builders MUST order by a seeded
random order beside the unseeded `in_random_order()`, which keeps
compiling without an argument (Rust has no optional argument, so the
seeded order is its own method); and
`oldest` and `latest` on the model builder MUST order by the model's
declared creation column.
Falsifier: `DB::table("posts").max("views")` is missing or returns the wrong value; `in_random_order_seeded(42)` twice returns different orders on the same rows, or `in_random_order()` no longer compiles; `where_not_between("views", 1, 5)` on `DB::table` is missing, or its `or_` form groups wrongly; or `oldest()` on a model whose creation column is `added_at` orders by `created_at`.
Mechanism: `par-laravel-gaps-data`.
Rationale: Rows `Builder::inRandomOrder`, `Builder::max`, `Builder::oldest` and `Builder::whereNotBetween`; identifier validation stays, with raw expressions through the raw-specific forms.
Status: Agreed 2026-10-09

[PAR-110] A seeder's `run` MUST be able to call other seeders with
parameters, silently or once (`call`, `call_silent`, `call_once`, `call_with`),
printing a `RUNNING` and a `DONE` line with the duration for each unless
silent, and MUST be able to resolve container services; `db:seed` MUST
refuse to run in production without `--force`, MUST accept `--database`,
and MUST run the root seeder that sets the order when no seeder is named.
Falsifier: `call(["UserSeeder", "PostSeeder"])` runs them out of order or prints no lines, `call_silent` prints, `call_once` runs a seeder twice, or `call_with(seeder, params)` hands no parameters; a seeder cannot resolve a bound service; `db:seed` runs under `APP_ENV=production` without `--force`, or refuses with it; `--database reporting` seeds another connection; or `db:seed` with no name does not run the root seeder.
Mechanism: `par-laravel-gaps-data`.
Rationale: Rows `Seeder`, `Seeder::call` and `artisan db:seed`; Laravel's `db:seed` confirms in production and `Seeder::call` reports each seeder.
Status: Agreed 2026-10-09

## Laravel API gaps: HTTP

The members of Laravel 13.35.0's routing, controller, request, response,
URL, middleware, CSRF, validation, error, view and session surface that
the parity review found missing or differing in Suprnova and ruled build:
the thirty-two rows of the log's HTTP areas, grouped by area. Two rows of
the same areas stay as they are: `TrustProxies::at`, which the log keeps
because trusting every address would let any caller forge
`X-Forwarded-For`, and `View::with`, which chained `with` and `prop` calls
already cover.

[PAR-111] `Exceptions::reportable(callback)` MUST register a callback that
every reported error reaches before the default log, typed as Laravel's
closures are: a callback taking `&FrameworkError` runs for every error,
and one taking another error type `E` runs only for an error whose wrapped
source downcasts to `E`; `.stop()` on the returned registration MUST keep
the later callbacks and the default log from running for an error it
handles. `Exceptions::report(&error)` MUST report an error the code
handles itself, and every error the framework turns into a `5xx`
response, every failed queue job attempt and every console command that
returns an error MUST be reported the same way; the `ErrorOccurred` event
stays. `Exceptions::dont_retry::<E>()` and
`Exceptions::dont_retry_when(predicate)` MUST name the errors that end a
queued job's retries, `Exceptions::should_stop_retries(&error)` MUST
answer for them, and the worker MUST fail a job at once, without a further
attempt, when it is true.
Falsifier: a callback taking `&std::io::Error` does not run when `FrameworkError::from_external(io_error)` becomes a 500, or runs for `FrameworkError::internal("x")`; a registration with `.stop()` lets the `framework error` log line or a later callback run; `Exceptions::report(&err)` reaches no callback; a queued job that fails, or a console command that returns `Err`, reaches no callback; or a job failing with an error `dont_retry_when` accepts runs again before its tries are spent.
Mechanism: `par-laravel-gaps-http`.
Rationale: Rows `Handler::reportable` and `Exceptions::shouldStopRetries` of the parity log's HTTP areas; Laravel sends HTTP, queue and console errors through `Handler::report`, and its worker asks `shouldStopRetries` before it retries.
Status: Agreed 2026-10-09

[PAR-112] The development error page MUST name the type of the error it
reports, and `ErrorReport::type_name()` MUST return it: the path
`std::any::type_name` gives for the concrete type handed to
`FrameworkError::from_external` or `from_external_with` (for an
`std::io::Error`, whatever `std::any::type_name::<std::io::Error>()`
reports on the building toolchain),
`FrameworkError::<Variant>` for the framework's own errors
(`FrameworkError::ModelNotFound`), and `panic` for a panic. With
`APP_EDITOR` set, every frame with a location MUST link to its line in the
editor, as Laravel's `Frame::editorHref` does: a name from Laravel's
editor list (`vscode`, `phpstorm`, `idea`, `cursor`, `zed`, `sublime` and
the others `ResolvesDumpSource` names) uses that editor's URL format, a
value holding `{file}` and `{line}` is a template filled with the absolute
file path and the line, and any other name gives
`<name>://open?file={file}&line={line}`; without `APP_EDITOR` no frame
carries a link. The source lines shown for application frames stay.
Falsifier: the page for a 500 from `FrameworkError::from_external(std::io::Error::other("disk"))` does not contain the text `std::any::type_name::<std::io::Error>()` returns, or a panic's page does not name `panic`; with `APP_EDITOR=vscode` an application frame at `src/handlers.rs:12` carries no `vscode://file/<absolute path>/src/handlers.rs:12` link; `APP_EDITOR=myeditor://{file}#{line}` is not filled in; or a frame carries a link with `APP_EDITOR` unset.
Mechanism: `par-laravel-gaps-http`.
Rationale: Rows `Renderer\Exception::class` and `Renderer\Frame::__construct`; Laravel's page heads with the exception class and links each frame through `app.editor`. Argument lists have no counterpart, since Rust frames carry no argument values.
Status: Agreed 2026-10-09

[PAR-113] `CsrfMiddleware` MUST check the token on every method but `GET`,
`HEAD` and `OPTIONS`, `QUERY` and extension methods such as `PROPFIND`
included, as Laravel's `isReading` does, and MUST read `_token` from a
JSON request body as it reads it from a form body: ahead of the
`X-CSRF-TOKEN` and `X-XSRF-TOKEN` headers, with `""` and `"0"` counting as
no value; the same-origin pass stays off by default.
`regenerate_session_id()` MUST also issue a new CSRF token, as Laravel's
`Session::regenerate` does, so a token read before it is refused after
it; the old session row is still destroyed.
Falsifier: a `QUERY` request without a token reaches its `query!` route behind `CsrfMiddleware`, or a `PROPFIND` request without one answers anything but 419; a `POST` with `Content-Type: application/json`, the body `{"_token": "<session token>"}` and no header answers 419; a JSON body whose `_token` is `""` beside a valid `X-CSRF-TOKEN` header answers 419; a same-origin request without a token passes under the default policy; or after `regenerate_session_id()` the session's CSRF token is the one it had before.
Mechanism: `par-laravel-gaps-http`.
Rationale: Rows `ValidateCsrfToken`, `VerifyCsrfToken` and `Session::regenerate`; Laravel reads the token with `$request->input('_token')`, which covers JSON bodies, and `Store::regenerate` calls `regenerateToken`.
Status: Agreed 2026-10-09

[PAR-114] A resource controller MUST declare its own middleware, as
Laravel's `HasMiddleware` does: `ResourceController::middleware()` returns
a list of `ControllerMiddleware` values, each a middleware value or an
alias name such as `"auth"` or `"throttle:60,1"`, scoped with
`only(actions)` or `except(actions)`, and a module named by `resource!`
MUST be able to declare the same list in a `pub fn middleware()`. The
resource's routes MUST run each listed middleware on exactly the actions
it is scoped to, after the group and route middleware, an unknown alias
failing the registration as `middleware_named` does, and
`ResourceRoutes::middleware` and `ResourceDef::middleware` MUST scope a
`ControllerMiddleware` the same way at registration. A route MUST be able
to leave out a middleware its group gives it, as Laravel's
`withoutMiddleware` does, with `without_middleware::<M>()` or
`without_middleware_named(name)`; the global middleware stays.
Falsifier: a controller whose `middleware()` lists `ControllerMiddleware::new(Auth).only(&[ResourceAction::Store])` runs `Auth` on `index` or not on `store`; `.except(&[ResourceAction::Index])` runs it on `index`; an alias in the list is not resolved, or an unknown alias registers; `ResourceRoutes::middleware` scopes differently from the trait's list; the controller's middleware runs before the group's; or a route with `.without_middleware::<EnsureJson>()` in a group with `EnsureJson` still runs it, or a sibling route stops running it.
Mechanism: `par-laravel-gaps-http`.
Rationale: Rows `HasMiddleware`, `Controllers\Middleware`, `Controllers\Middleware::__construct`, `ControllerMiddlewareOptions` and `ControllerMiddlewareOptions::except`; Laravel's `Route::gatherMiddleware` appends the controller's middleware to the route's and removes the excluded ones.
Status: Agreed 2026-10-09

[PAR-115] A request whose path matches no route of its method but a route
of another method MUST answer `405` with an `Allow` header listing those
methods (`HEAD` beside `GET`) and Laravel's message, `The DELETE method is
not supported for route posts/1. Supported methods: GET, HEAD.`, and an
`OPTIONS` request to such a path MUST answer `200` with an empty body and
an `Allow` header joined without spaces, as Laravel's `getRouteForMethods`
does; both answers run through the global middleware as the `404` does, a
route's constraints count as they do for a match, and a path no route
matches keeps the `404` or the fallback. `route_has(names)` MUST return
true only when every name is registered, as Laravel's `Route::has` does.
Falsifier: `DELETE /posts/1` on a router holding only `GET /posts/{id}` answers 404, or 405 without `Allow: GET, HEAD` or with another message; `OPTIONS /posts/1` there answers other than 200 with an empty body and `Allow: GET,HEAD`; a CORS preflight to that path loses the CORS middleware's answer; a path no route matches, or one every route's constraints refuse, answers 405; or `route_has(&["home", "missing"])` is true while `missing` is unregistered.
Mechanism: `par-laravel-gaps-http`.
Rationale: Rows `RouteCollectionInterface::match` and `RouteCollectionInterface::hasNamedRoute`; Laravel's `AbstractRouteCollection::handleMatchedRoute` checks the other verbs before its 404, and `Router::has` takes one name or several.
Status: Agreed 2026-10-09

[PAR-116] A `ValidateSignature` middleware MUST let a request with a valid
signature through and answer every other one, a missing, wrong or expired
signature alike, with `403` and the message `Invalid signature.`, as
Laravel's `InvalidSignatureException` does;
`ValidateSignature::relative(ignore)` and
`ValidateSignature::from_alias_args`, which reads `relative` and then the
parameters to ignore (`signed:relative,utm_source`), MUST build it, every
form verifying the path and query Suprnova signs. The parameters it is
told to ignore, by `ignore(names)`, by the alias arguments or for the
whole application with `ValidateSignature::except(names)`, MUST be left
out of the verified text, and
`url::has_valid_signature_ignoring(request, names)` MUST leave them out the
same way. `signature_verdict` keeps telling `Expired` from `Invalid`, and
the key-sorted signed text and the refusal of a repeated `signature` or
`expires` stay.
Falsifier: a route behind `ValidateSignature::new()` runs its handler for a signed URL with a changed query, an expired `temporary_signed_route` URL or no signature, or answers one of them with anything but 403 `Invalid signature.`; a valid signed URL is refused; `ValidateSignature::new().ignore(["utm_source"])` or the alias `signed:relative,utm_source` refuses a valid signed URL with `&utm_source=mail` appended; `has_valid_signature_ignoring(&request, &["utm_source"])` is false for it; or a repeated `signature` passes.
Mechanism: `par-laravel-gaps-http`.
Rationale: Rows `ValidateSignature::relative`, `InvalidSignatureException::__construct` and `UrlGenerator::hasValidRelativeSignature`; the signed text stays the sorted path and query, as Laravel's `signedRoute` sorts parameters before it signs.
Status: Agreed 2026-10-09

[PAR-117] `url::current(request)` MUST return the absolute URL of the
request without its query, as Laravel's `current()` does: the `APP_URL`
origin, the public root and the path, while `url::full`, the signature
check and the redirects that send a request back keep the path and the
query. `url::previous_path(fallback)` MUST return the path of the
session's previous URL without its query, the public root or a trailing
slash, `/` when nothing is left, and the fallback's path when no previous
URL is recorded. `url::secure_with(path, segments)` MUST append each
segment percent-encoded as a path segment, as Laravel's
`secure($path, $parameters)` does, keeping the `APP_URL` base and the
https upgrade. `Redirect::guest` MUST store the request's own path and
query as the intended URL only for a `GET` request that does not expect
JSON, and the session's previous URL for any other request, never the
`Referer`, with the same-site check kept.
Falsifier: `url::current` for `GET /invoices?page=2` with `APP_URL=https://example.com` is anything but `https://example.com/invoices`; `url::full` there loses `?page=2`; `url::previous_path("/")` after a recorded previous URL `/billing/invoices/?page=2` under the root `/billing` is anything but `/invoices`, or with no previous URL anything but `/`; `url::secure_with("users", &["a b", "7"])` is anything but `https://<APP_URL host>/users/a%20b/7`; a `POST` or a JSON `GET` behind `Redirect::guest` stores its own URL as the intended URL, or does not store a recorded previous URL; a plain `GET` stores anything but its own path and query; or a `Referer` header becomes the intended URL.
Mechanism: `par-laravel-gaps-http`.
Rationale: Rows `UrlGenerator::current`, `UrlGenerator::previousPath`, `UrlGenerator::secure`, `Redirector::guest` and `Response::redirectGuest`; the `APP_URL` base, which a forged `Host` header cannot change, the https upgrade and the refusal to read `Referer` stay.
Status: Agreed 2026-10-09

[PAR-118] `Request::ajax()` MUST be true only for
`X-Requested-With: XMLHttpRequest` exactly, as Symfony's
`isXmlHttpRequest` compares it. `Request::host()` MUST lowercase the host,
strip only a numeric port, and return `None` for a host holding anything
but letters, digits, `-`, `_`, `.`, `:` and the brackets of an IPv6
literal, so `http_host` and `scheme_and_http_host` never carry one. A
`TrustHosts` middleware, `TrustHosts::at(patterns, subdomains)`, MUST
answer `400` with the message `Bad request.` for a request whose host is
invalid or matches none of the trusted patterns, matched without regard
to case as Laravel's are, `subdomains` adding the `APP_URL` host and its
subdomains, which are also the default; it trusts every host in the local
environment, as Laravel's does. `Cookie::forget` MUST take its path,
domain and SameSite from the session configuration (`SESSION_PATH` or the
public root, `SESSION_DOMAIN`, `SESSION_SAME_SITE`), so the deletion
cookie matches the cookie it deletes; `Secure` stays forced and
`forget_with` still overrides the path and the domain.
Falsifier: `ajax()` is true for `X-Requested-With: xmlhttprequest`; `host()` for `Host: Example.COM:8080` is anything but `example.com`, or for `Host: example.com:abc` drops `:abc`; `host()` for `Host: bad<host>` is not `None`, or `scheme_and_http_host` carries it; a request for `evil.test` behind `TrustHosts::at([r"^example\.com$"], false)` reaches its handler, or one for `api.example.com` behind `TrustHosts::new()` with `APP_URL=https://example.com` is refused; or `Cookie::forget("prefs")` under `SESSION_PATH=/app`, `SESSION_DOMAIN=.example.com` and `SESSION_SAME_SITE=strict` lacks `Path=/app`, `Domain=.example.com`, `SameSite=Strict` or `Secure`.
Mechanism: `par-laravel-gaps-http`.
Rationale: Rows `Request::isXmlHttpRequest`, `Request::schemeAndHttpHost`, `CookieJar::forget` and `Cookie::forget`; Laravel's cookie jar takes its defaults from `session.path`, `session.domain` and `session.same_site`, and an untrusted host answers 400 through `RequestExceptionInterface`.
Status: Agreed 2026-10-09

[PAR-119] The English catalog MUST carry `validation-ip`, `The { $field }
field must be a valid IP address.`, so `#[validate(ip)]` no longer shows
the generic message, and the numeric `max` message (`validation-range`
with `$kind` `max`) MUST read `must not be greater than { $max }`.
`RequiredIf::when(condition)` MUST build Laravel's `Rule::requiredIf`: a
rule taking a boolean or a closure answering one, that requires the field
with `validation-required`'s message when the condition holds and passes
it otherwise. `ImageFile` MUST accept only the types Laravel's `image`
rule does, JPEG, PNG, GIF, BMP, WebP, AVIF, HEIC and HEIF, and refuse
TIFF, PSD, ICO and JPEG XL, SVG staying reachable only through an
explicit `MimeType` allowlist; a `Dimensions` upload validator MUST check
Laravel's `dimensions` constraints (`min_width`, `max_width`,
`min_height`, `max_height`, `width`, `height`, `ratio`), refusing with
`validation-dimensions`, `The { $field } field has invalid image
dimensions.`
Falsifier: `#[validate(ip)]` on a field `address` holding `999.1.1.1` reports anything but `The address field must be a valid IP address.`; `#[validate(range(max = 10))]` on 11 reports `must be at most 10`; `RequiredIf::when(true)` passes an empty value, or `when(false)` or a closure answering false refuses one; `ImageFile` passes a TIFF, PSD, ICO or JPEG XL file, or refuses a JPEG, WebP, AVIF or HEIC one; or `Dimensions` with a `max_width` of 100 passes a PNG 101 pixels wide, or refuses one 100 pixels wide.
Mechanism: `par-laravel-gaps-http`.
Rationale: Rows `ValidatesAttributes::validateIp`, `Rule::requiredIf`, `File::image` and `Numeric::max`; the bounds already match, and the field type already supplies the numeric check.
Status: Agreed 2026-10-09

[PAR-120] `make:middleware` MUST write a middleware whose `handle` passes
the request to `next` and returns its response, printing nothing, as
Laravel's stub does; it MUST accept a nested name (`Admin/EnsureRole`
writes `src/middleware/admin/ensure_role.rs` and declares the modules on
the way), and `--test` MUST also write a test that runs the middleware.
`make:view` MUST scaffold a checked view: the template under `templates/`
(`admin.dashboard` and `admin/dashboard` both write
`templates/admin/dashboard.html`) and a `#[view]` struct naming it,
refusing to overwrite an existing file unless `--force` is given, with
`--test` writing a test that renders it. `make:inertia` MUST accept a
nested name, `--force` and `--test` the same way.
Falsifier: the file `make:middleware Audit` writes contains `println!` or does anything but return `next(request).await`; `make:middleware Admin/EnsureRole` fails or writes outside `src/middleware/admin/`; `--test` writes no test file; `make:view admin.dashboard` writes no template or no `#[view]` struct naming `admin/dashboard.html`, overwrites an existing file without `--force`, or refuses with it; or `make:inertia Admin/Users --force --test` is refused.
Mechanism: `par-laravel-gaps-http`.
Rationale: Rows `artisan make:middleware` and `artisan make:view`; the `Middleware` suffix and the `src/middleware` path follow Rust layout and stay.
Status: Agreed 2026-10-09

[PAR-121] The existence filters on the model builder MUST read a
polymorphic owner relation (a `MorphTo`) the way Laravel's `has` does:
`has`, `or_has`, `has_count`, `doesnt_have` and `or_doesnt_have` on such a
relation MUST go through the morph existence query over every registered
owner type, so a row whose type column names a registered type whose table
holds the id counts as having its owner, and a row whose owner is missing,
soft-deleted under the owner's scopes, or of an unregistered type does
not; `where_has` and `where_doesnt_have` with a typed predicate on such a
relation MUST apply the predicate to the owners of the predicate's model
type, as `where_has_morph` with that one type does.
Falsifier: `MorphComment::query().has("commentable").get()` returns no comment while a comment points at an existing registered post; `doesnt_have("commentable")` leaves out a comment whose post row was deleted or whose type is unregistered; `has_count("commentable", "=", 1)` matches no comment with an owner; or `where_has::<MorphPost, _>("commentable", |q| q.filter("title", "kept"))` returns a comment whose post has another title.
Mechanism: `par-laravel-gaps-data`.
Rationale: Laravel's `Builder::has` routes a `MorphTo` relation to `hasMorph($relation, ['*'])` (`Eloquent/Concerns/QueriesRelationships.php`); found on 2026-10-09 while fixing finding 4 of report 7a904d31 (item has-on-a-morph-to-relation-matches-nothing), where the generic probe rendered a constant false; the typed predicate narrows to one owner type because a Rust closure over one model cannot run against every owner table.
Status: Agreed 2026-10-09
