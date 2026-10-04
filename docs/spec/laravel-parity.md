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
taking a builder, which may correlate with `where_column`. Every value in a
join or exists clause MUST be bound as a parameter, never written into the
SQL text, and every table, alias and column MUST be quoted for the active
backend. A model query with a join MUST select the model's own table
columns unless told otherwise, so a joined table's `id` never overwrites
the model's.
Falsifier: one of the three query shapes in issue #125 (chained left joins with a table alias and aliased columns, a left join against a grouped subquery, a correlated `where_exists`) returns rows that differ from the same query written as raw SQL; a value given to a join's `where` appears in the generated SQL text; or a model query joining another table hydrates the joined table's `id` into the model.
Mechanism: `par-joins`.
Rationale: Issue #125. Laravel `Illuminate\Database\Query\Builder::join`, `leftJoin`, `rightJoin`, `crossJoin`, `joinSub`, `leftJoinSub`, `whereExists`, and `JoinClause`.
Status: Agreed 2026-10-01

[PAR-002] The framework MUST offer file responses: a response streaming a
file from a path with the content type its extension implies and
`Content-Disposition: inline`; a download response with `attachment` and
a filename the caller chooses; a download of in-memory bytes with a given
content type; and the same two responses for a path on a named storage
disk, resolved through the disk so its path guard applies. Every
`Content-Disposition` header it writes MUST follow RFC 6266: an ASCII
`filename` fallback, plus `filename*=UTF-8''` with percent-encoding when
the name is not plain printable ASCII, and no character of the name may
change the header's structure.
Falsifier: a download named `Certificat·Joan Pérez.pdf` produces a header without a correct `filename*` or with a raw non-ASCII byte in `filename`; a name holding a quote, backslash, CR or LF changes the header's structure; a disk download of `../secret` reads outside the disk root; or a file response sends a content type or disposition other than the one specified.
Mechanism: `par-file-responses`.
Rationale: Issue #126. Laravel `ResponseFactory::file`, `download`, `streamDownload`, and `FilesystemAdapter::download` and `response`.
Status: Agreed 2026-10-01

[PAR-003] `#[handler]` MUST accept `#[authorize(ability, Type)]`, which
authorizes the ability against a model type, and `#[authorize(ability,
param)]`, which authorizes it against the route-bound model the handler
receives as `param`. The check MUST run after route model binding and
before the handler body, through the async gate, so policies and the RBAC
gate bridge both apply. It MUST answer 401 when no user is authenticated,
403 when the gate denies, and 404 when the policy denies as not found. A
`param` the handler does not take MUST be a compile error.
Falsifier: a handler declaring `#[authorize("update", post)]` runs its body for a user the policy denies; answers 403 to a guest; answers 403 where the policy denies as not found; answers 403 instead of 404 for a route model that does not exist; or compiles when `post` is not one of its parameters.
Mechanism: `par-authorize`.
Rationale: Issue #127. Laravel 13 `Illuminate\Routing\Attributes\Controllers\Authorize`, and the `can` middleware it applies.
Status: Agreed 2026-10-01

[PAR-004] After a successful save, a model MUST report which attributes
that save changed: `was_changed` for one attribute or several, and
`get_changes` for all of them. While the save's `updated` and `saved`
observers run, `get_original` and `get_raw_original` MUST return each
attribute's value as it was loaded before the save; once the save returns,
the original values are the saved ones. The rest follows Laravel: a save
that changes nothing leaves the previous save's changes in place, and an
insert reports no changes.
Falsifier: in an `updated` observer for a save that flips `is_admin`, `was_changed("is_admin")` is false, `was_changed` reports an attribute the save did not change, `get_changes` omits or adds an attribute, or `get_raw_original("is_admin")` is not the value loaded before the save; or after the save returns, `get_original("is_admin")` still reports the value loaded before it.
Mechanism: `par-model-changes`.
Rationale: Issue #128. Laravel `HasAttributes::wasChanged`, `getChanges`, `getOriginal`, `getRawOriginal`, `syncChanges` and `syncOriginal`, as `Model::save`, `performUpdate` and `finishSave` call them.
Status: Agreed 2026-10-01

[PAR-005] A belongs-to-many relation MUST offer `sync_without_detaching`,
which attaches each given id the relation does not already hold and leaves
every existing pivot row, its pivot columns included, untouched.
Falsifier: after `sync_without_detaching` with id 3 on a relation that holds 1 and 2, the relation holds anything other than 1, 2 and 3, or a pivot column of the rows for 1 or 2 changed.
Mechanism: `par-sync-without-detaching`.
Rationale: Issue #128. Laravel `InteractsWithPivotTable::syncWithoutDetaching`.
Status: Agreed 2026-10-01

[PAR-006] The `DB::table` builder and the model query builder MUST offer
`where_any`, `or_where_any`, `where_all`, `or_where_all`, `where_none` and
`or_where_none` (one comparison across several columns, grouped in
parentheses), `or_where_in` and `or_where_not_in` with a list or a
subquery, `where_in` and `where_not_in` with a subquery, `or_where_raw`
with bound values, `or_where_null` and `or_where_not_null`, and `reorder`,
which drops the orderings already set and may set a new one.
Falsifier: one of these helpers returns rows that differ from the equivalent raw SQL; a grouped helper lets an `or` escape its parentheses, so `where("a", 1).where_any(["b", "c"], "=", 2)` returns a row whose `a` is not 1; or a subquery used by `where_in` loses its own bound values.
Mechanism: `par-query-helpers`.
Rationale: Issue #128. Laravel `Builder::whereAny`, `whereAll`, `whereNone`, `orWhereIn`, `orWhereNotIn`, `orWhereRaw`, `orWhereNull`, `orWhereNotNull`, `reorder`.
Status: Agreed 2026-10-01

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
chain.
Falsifier: under `except`, a named job is recorded instead of dispatched or another job is dispatched instead of recorded; a raw push cannot be read back; or `assert_pushed_without_chain` passes for a job pushed with a chain.
Mechanism: `par-queue-fake`.
Rationale: Laravel `QueueFake::except`, `pushRaw`, `rawPushes`, `assertPushedWithoutChain`.
Status: Agreed 2026-10-01

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
the place of the app's Inertia error page. Every other response, and every
response with debug off, MUST stay as it is today.
Falsifier: with debug on, a browser request to a handler that returns an error gets JSON, or a page without the error's message or without the 500 status; an Inertia visit to it gets the app's Inertia error page; a request with `Accept: application/json` gets HTML; or, with debug off, a browser request gets anything but the response it got before the page existed.
Mechanism: `par-debug-error-page`.
Rationale: Laravel renders its exception page when `APP_DEBUG` is on and the request does not expect JSON, and the Inertia documentation keeps that page in local development, where the client shows it in its modal.
Status: Agreed 2026-10-02

[PAR-013] With debug on, the framework MUST record the stack frames at the
place where an error first became a `FrameworkError` or an `AppError`, or
where a panic happened, and the page MUST list them: the application's
frames shown, and the frames of the standard library, the async runtime,
other dependencies and the framework itself collapsed behind a count. With
debug off, the framework MUST NOT record frames.
Falsifier: a handler whose `?` turns a database error into a `FrameworkError` yields a page whose frames do not name that handler; a panicking handler's page does not name the function that panicked; a `std`, `tokio` or `hyper` frame is shown outside a collapsed group; or, with debug off, an error records frames.
Mechanism: `par-debug-error-page`.
Status: Agreed 2026-10-02

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
`Storage::set_default_disk`, or else by `FILESYSTEM_DISK`, and
`Storage::default_disk()` MUST return the disk registered under that
name. With neither set, `Storage::default_disk()` MUST return an error
that names `FILESYSTEM_DISK`. A call that names its disk MUST behave as
it does today.
Falsifier: with `FILESYSTEM_DISK=uploads` and a disk registered as `uploads`, bytes written through `Storage::default_disk()` cannot be read through `Storage::disk("uploads")`; `Storage::set_default_disk("archive")` does not win over `FILESYSTEM_DISK`; with neither set, `Storage::default_disk()` returns a disk, or an error that does not name `FILESYSTEM_DISK`; or `Storage::disk(name)` answers differently with a default set.
Mechanism: `par-default-disk`.
Rationale: Laravel's `Storage::disk()` with no name and `Storage::put` use `filesystems.default`, which reads `FILESYSTEM_DISK`.
Status: Agreed 2026-10-02

[PAR-017] When a default disk is named, startup MUST fail if no disk is
registered under that name once the application's bootstrap and the
environment's disks are in place: `filesystem::bootstrap_from_env`,
which the server and the worker commands run at boot, MUST return an
error that names the missing disk and `FILESYSTEM_DISK`.
Falsifier: with `FILESYSTEM_DISK=uploads` and no `uploads` disk registered, `bootstrap_from_env` returns `Ok`; its error does not name `uploads` or `FILESYSTEM_DISK`; or with `FILESYSTEM_DISK=s3` and `S3_BUCKET` set, it fails although it registered the `s3` disk itself.
Mechanism: `par-default-disk`.
Status: Agreed 2026-10-02

## SQS queue driver

The developer ruled on 2026-10-01 to build an SQS queue driver, a managed
queue outside the application database and Redis, after the default disk.

[PAR-018] With `QUEUE_DRIVER=sqs`, the framework MUST queue jobs on Amazon
SQS standard queues. A push MUST send the job to the queue its envelope
names, or to `SQS_QUEUE` when it names none, and a delayed job MUST NOT be
received before its time, a delay longer than the 15 minutes SQS allows on
one message included. A pop MUST receive one message and hide it for the
worker's visibility timeout. An acknowledgement MUST delete it, a `nack`
MUST return it after the requeue delay with one more attempt, and a
`release` MUST return it after the delay with the same number of attempts.
A worker MUST receive from the queues its `--queue` list names, in order,
and from `SQS_QUEUE` when the list is empty. `size`, `pending_size`,
`delayed_size` and `reserved_size` MUST report the approximate counts SQS
keeps for `SQS_QUEUE`, and `clear` MUST purge it and return the count it
held.
Falsifier: against an SQS endpoint, a pushed job is never received; a job pushed to the queue `emails` is received from `SQS_QUEUE`; a job delayed 20 minutes is received before 20 minutes have passed; an acknowledged job is received again; a nacked job comes back with attempts not one higher, or a released job with its attempts changed; a worker with `--queue=emails` receives a job from `SQS_QUEUE`; or `size` reports other than the counts SQS returns.
Mechanism: `par-sqs-queue`.
Rationale: Laravel's `SqsJob` counts every receive as an attempt, so its release counts one, and it passes SQS a delay over 900 seconds, which SQS refuses.
Status: Agreed 2026-10-02

[PAR-019] The `sqs` driver MUST read its configuration from the
environment. `SQS_PREFIX`, `SQS_QUEUE` (default `default`) and
`SQS_SUFFIX` MUST build the queue URL as Laravel does, and a queue name
that is already a URL MUST be used as it is. `AWS_DEFAULT_REGION`, or else
`AWS_REGION`, names the region, and `SQS_ENDPOINT` points the driver at a
service that is not AWS. Requests MUST be signed with AWS Signature
Version 4 for `sqs` in that region, with `AWS_ACCESS_KEY_ID`,
`AWS_SECRET_ACCESS_KEY` and `AWS_SESSION_TOKEN` when they are set, or else
with AWS's default credential chain. Boot MUST fail with an error that
names the variable when no region is set, when the queue is not a URL and
`SQS_PREFIX` is not set, or when the queue is a FIFO queue, whose name ends
in `.fifo`. `QUEUE_CONNECTIONS` and `QUEUE_FAILOVER_CONNECTIONS` MUST
accept `sqs`.
Falsifier: with `SQS_PREFIX=https://sqs.us-east-1.amazonaws.com/123456789012`, `SQS_QUEUE=jobs` and `SQS_SUFFIX=-prod`, a push names a queue URL other than `https://sqs.us-east-1.amazonaws.com/123456789012/jobs-prod`; a request carries no Signature Version 4 `Authorization` header scoped to the region and `sqs`; boot succeeds with no region, with a plain queue name and no prefix, or with `SQS_QUEUE=jobs.fifo`; or `QUEUE_CONNECTIONS=sqs` is refused.
Mechanism: `par-sqs-queue`.
Rationale: a FIFO queue needs a message group and a deduplication ID on each job, and the parity map does not rule `onGroup` or `withDeduplicator` to build, so a FIFO queue is refused at boot rather than sent messages SQS rejects.
Status: Agreed 2026-10-02

[PAR-020] With `SQS_OVERFLOW_ENABLED=true`, a job whose payload is 1 MiB
or more, SQS's message limit, or every job when `SQS_OVERFLOW_ALWAYS=true`,
MUST be stored on a filesystem disk, the one `SQS_OVERFLOW_DISK` names or
else the default disk, and sent to SQS as a pointer to it. A pop MUST
return the stored job. An acknowledgement MUST delete the stored payload
unless `SQS_OVERFLOW_DELETE_AFTER_PROCESSING=false`, and `clear` MUST
delete the stored payloads too when `SQS_OVERFLOW_FLUSH_ON_CLEAR=true`.
Without overflow, a push over the limit MUST fail with an error that names
the limit and `SQS_OVERFLOW_ENABLED`. Boot MUST fail when overflow is on
and the disk is not registered.
Falsifier: with overflow on, a 2 MiB job is sent to SQS whole, or a pop returns the pointer instead of the job; its stored payload survives an acknowledgement with delete-after-processing on, or survives `clear` with flush-on-clear on; with overflow off, a 2 MiB push succeeds or its error names neither the limit nor `SQS_OVERFLOW_ENABLED`; or boot succeeds with overflow on and no disk registered under the name.
Mechanism: `par-sqs-queue`.
Rationale: Laravel keeps overflow payloads in a cache store, which can evict one before its job runs; a disk keeps it until the job is done.
Status: Agreed 2026-10-02

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
`env` MUST add a variable to the environment the process inherits, `input`
MUST be written to its standard input, and an output callback MUST receive
each chunk of standard output and standard error as it arrives unless
`quietly` is set, and `tty` MUST hand the process the terminal's standard
input and output, capturing nothing. A program that cannot be started
MUST be an error that names it.
Falsifier: running `sh -c 'printf out; printf err >&2; exit 3'` does not give the output `out`, the error output `err`, exit code 3 and a failed result, or `run` returns an error for it; an argument to `Process::command` holding `; touch marker` is run by a shell, so `marker` exists; `Process::shell("printf 'b\na\n' | sort")` does not output `a\nb\n`; `path`, `env` or `input` does not reach the process; the callback misses a chunk, or is called under `quietly`; output is captured under `tty`; or a missing program gives a result, or an error that does not name it.
Mechanism: `par-process`.
Rationale: Laravel's `Process::run` takes a string, run through the shell, or an array, run as it is; `shell` and `command` are the two, kept apart so the shell is never reached by accident.
Status: Agreed 2026-10-02

[PAR-022] A process MUST be killed, with every process it started, when
it runs past its `timeout` (60 seconds unless set, none after `forever`),
and `run` MUST then return a timeout error that names the command and the
timeout. `start` MUST return a running process with its id, whether it is
still running, the output so far and since the last read, a way to send it
a signal, `stop` (a terminate signal, then a kill after a grace period)
and `wait`, which returns its result. Dropping a running process, or the
future of `run` before it completes, MUST kill it with every process it
started.
Falsifier: `sh -c 'sleep 30 & sleep 30'` with a one-second timeout does not return a timeout error naming the command within five seconds, or either `sleep` is still running afterwards; a started process reports no id, reports running after it exited, or `stop` leaves it running; or after the `run` future or a started process is dropped, the process or a child of it is still running.
Mechanism: `par-process`.
Status: Agreed 2026-10-02

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
shell line as given) MUST get
the faked result, its output, error output and exit code, and any other
command an empty successful one, unless `prevent_stray_processes` makes it
an error that names the command. A described fake MUST support a run that
lasts a given number of `running` checks, and a sequence MUST answer its
results in turn. Every faked run, start, pool and pipe MUST be recorded,
and `assert_ran`, `assert_ran_times`, `assert_ran_in_order`,
`assert_not_ran` and `assert_nothing_ran` MUST fail the test when the
record does not match.
Falsifier: with a fake installed, a command that creates a file creates it; a matching command gets another result; an unmatched command errors without `prevent_stray_processes` or runs with it; a sequence answers out of turn; a described process stops reporting running before its count; or an assertion passes on a record that does not match it.
Mechanism: `par-process`.
Rationale: Laravel's `Process::fake` with pattern handlers, `describe`, `sequence`, `preventStrayProcesses` and the assertions.
Status: Agreed 2026-10-02

## Log channels

The developer ruled on 2026-10-01 to build log channels: files, rotation,
retention, flushing on shutdown and several outputs at once, with stdout
the default. Slack and the other vendor sinks are deferred, not refused.

[PAR-026] `LOG_CHANNEL` MUST name the channel the application's log
events go to, and `stdout` when it is not set, writing what the framework
writes today. The built-in channels are `stdout`, `stderr` (also named
`errorlog`), `single`, `daily`, `monthly`, `syslog`, `null`, and `stack`,
which writes to every channel `LOG_STACK` lists (`single` when it is not
set). `Log::define(name, channel)` in the bootstrap MUST add a channel
under a name, and `Log::extend(driver, factory)` MUST add a driver a
defined channel can use. A `LOG_CHANNEL` or `LOG_STACK` that names no
channel MUST fail boot with an error that names it.
Falsifier: with `LOG_CHANNEL` unset, an `info!` event is not written to stdout; with `LOG_CHANNEL=single` the event is not in the file, or is also on stdout; a defined channel, or one on an extended driver, does not receive the events of the default channel it is; or boot succeeds with `LOG_CHANNEL=nosuch`, or its error does not name `nosuch`.
Mechanism: `par-log-channels`.
Rationale: Laravel's `config/logging.php` channels and `LOG_CHANNEL`, with stdout as the default the developer kept.
Status: Agreed 2026-10-02

[PAR-027] `single` MUST append each record to one file, `logs/suprnova.log`
under the storage directory unless the channel sets a path, creating the
directories. `daily` MUST write to a file named for the day
(`suprnova-2026-10-02.log`) and keep the newest `LOG_DAILY_DAYS` (14)
files, deleting older ones; `monthly` MUST write to a file named for the
month (`suprnova-2026-10.log`) and keep the newest 3. The day and the month
are those of the framework clock, in UTC.
Falsifier: a record is not appended to the single file, or its directory is not created; a record written after midnight goes to the previous day's file; with 20 dated files and `LOG_DAILY_DAYS=14`, other than the 14 newest remain after a write; or a monthly channel keeps other than the 3 newest months.
Mechanism: `par-log-channels`.
Status: Agreed 2026-10-02

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
(`user` unless set) and the severity of the record's level. Off Unix it
MUST fail boot with an error that says so.
Falsifier: a record reaches the socket with another priority than facility times 8 plus severity, or does not reach it; or an unknown facility boots.
Mechanism: `par-log-channels`.
Status: Agreed 2026-10-02

## Redis facade

The developer ruled on 2026-10-01 to build a thin managed Redis API:
configuration, connection lifecycle, commands, pipelines and an escape
hatch to the client, with dedicated connections for subscriptions and
blocking commands. Redis Cluster is deferred, not refused. The funnel and
throttle limiters stay unbuilt by the developer's ruling of 2026-09-30.

[PAR-031] `Redis::connection(name)` MUST return the connection with that
name: `default`, which reaches `REDIS_URL` (`redis://127.0.0.1:6379` when
it is unset), or one the bootstrap gives with `Redis::define(name, url)`,
which replaces any connection of that name. A connection MUST open on its
first command, not before, and MUST open again after it is lost.
`Redis::purge(name)` MUST forget the connection, which closes once no
handle holds it, and `Redis::connections()` MUST list the names of the
connections resolved and not purged. A name with no connection, or a URL
that is not a Redis URL, MUST be an error that names the connection.
Falsifier: the default connection reaches another server or database than `REDIS_URL` names, or than `redis://127.0.0.1:6379` when it is unset; a defined connection reaches another database than its URL's; resolving a connection to an address where nothing listens fails before a command is sent; after the server drops the connection, a read, or the write after it, fails although the server is up; `connections()` misses a resolved name or lists a purged one; a purged connection that no handle holds stays open; or an unknown name, a URL such as `http://x`, or such a `REDIS_URL`, is not an error naming the connection.
Mechanism: `par-redis`.
Rationale: Laravel's `RedisManager` and the `redis` connections of `config/database.php`; Suprnova names connections in the bootstrap instead of a config file.
Status: Agreed 2026-10-03

[PAR-032] A connection MUST run the common commands as typed methods
(`get`, `set`, `set_ex`, `del`, `exists`, `incr`, `decr`, `expire`, `ttl`,
`mget`, `hset`, `hget`, `hgetall`, `hdel`, `lpush`, `rpush`, `lpop`,
`rpop`, `lrange`, `sadd`, `srem`, `smembers`, `zadd`, `zrange`,
`zrangebyscore`, `publish`, `eval` and `scan`), any other command with
`command(name, args)`, which returns the reply, and give the underlying
`redis` client with `client()`. A read, typed or one of Laravel's
retryable commands given to `command`, MUST be sent again after a lost
connection: once, and once more for each retry `REDIS_COMMAND_RETRIES`
adds; another command MUST NOT be. While `Redis::enable_events()` is in
force, each command a connection runs outside a pipeline or a transaction
MUST be reported to the listeners `Redis::listen` adds, with the
connection's name, the command, its arguments and its duration, and each
command that fails to the listeners `Redis::listen_for_failures` adds,
with its error. Events are off until enabled.
Falsifier: a typed command, or `command("LRANGE", ...)`, returns other than the server's reply; `client()` is not a client of the same server and database; a read fails after the server dropped the connection once; a write is applied twice after a lost connection; with events enabled a command is not reported, or is reported with the wrong connection name, command or arguments, or a failed command is not reported to the failure listeners; or a command is reported while events are off.
Mechanism: `par-redis`.
Rationale: Laravel's `Connection::command`, `client`, `listen`, `listenForFailures`, `CommandExecuted`, `CommandFailed` and `PhpRedisConnection::RETRYABLE_COMMANDS`.
Status: Agreed 2026-10-03

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
with the separator, as Laravel's `Str::slug` does. `Str::mask(value,
character, index, length)` MUST replace the characters from `index`
(counted from the end when negative) for `length` characters (to the end
when `None`) with `character`. `Str::limit(value, limit, end)` MUST keep
the first `limit` characters and append `end` when it cut, and
`Str::limit_words(value, limit, end)` MUST cut at the last space within
the limit. `Str::excerpt(text, phrase, radius, omission)` MUST return the
first match of the phrase, case-insensitive, with up to `radius`
characters on each side and `omission` where it cut, or `None` when the
phrase is absent. Every count is in characters, not bytes.
Falsifier: `Str::slug("Laravel 5 Framework", "-")` is not `laravel-5-framework`, `Str::slug("Œuvre d'art_2 @home", "-")` is not `oeuvre-dart-2-at-home`, or `Str::slug("foo bar", "_")` is not `foo_bar`; `Str::mask("taylor@example.com", '*', 3, None)` is not `tay***************`, or `Str::mask("taylor@example.com", '*', -15, Some(3))` is not `tay***@example.com`; `Str::limit("The quick brown fox jumps over the lazy dog", 20, "...")` is not `The quick brown fox...`, or `Str::limit_words("The quick brown fox", 12, "...")` is not `The quick...`; `Str::excerpt("This is my name", "my", 3, "...")` is not `Some("...is my na...")`; or a multibyte value is cut inside a character.
Mechanism: `par-strings`.
Rationale: Laravel's `Str::slug`, `mask`, `limit` and `excerpt`.
Status: Agreed 2026-10-03

[PAR-036] `Str::plural(word, count)` and `Str::singular(word)` MUST
inflect the word by the rules of the language of the current `Lang`
locale: English, French, Norwegian Bokmål, Portuguese, Spanish or
Turkish, as Laravel's `Pluralizer` does with doctrine/inflector, and by
the English rules for any other language. A count of 1 or -1 MUST leave
the word as it is, and the result MUST keep the word's case: lower,
upper, first letter capital, or each word capital.
Falsifier: in English `car` is not `cars`, `child` is not `children`, `person` is not `people`, `sheep` is not `sheep`, `Car` is not `Cars`, `CAR` is not `CARS`, `cars` with a count of 1 is not `cars`, or `Str::singular("people")` is not `person`; in French `cheval` is not `chevaux`; in Spanish `ciudad` is not `ciudades`; in Portuguese `cão` is not `cães`; in Norwegian Bokmål `bil` is not `biler`; in Turkish `kitap` is not `kitaplar`; or under a locale with none of these languages `car` is not `cars`.
Mechanism: `par-strings`.
Rationale: Laravel's `Str::plural`, `Str::singular` and `Pluralizer::useLanguage`; Suprnova takes the language from the request's locale instead of a process-wide setting.
Status: Agreed 2026-10-03

[PAR-037] `Lang::percentage(value, precision)` MUST format `value` as a
percentage, `10` as ten percent, with `precision` fraction digits, the
way the current locale writes one. `Lang::abbreviate(value, precision)`
MUST divide the value by the largest of a thousand, a million, a
billion, a trillion and a quadrillion that it reaches, write it with
`precision` fraction digits in the current locale's number format, and
append `K`, `M`, `B`, `T` or `Q`, as Laravel's `Number::abbreviate` does;
a value under a thousand is written as it is.
Falsifier: in `en` `Lang::percentage(10.0, 0)` is not `10%`, or `Lang::percentage(12.345, 1)` is not `12.3%`; in `de` `Lang::percentage(10.0, 0)` is not `10 %` with the locale's space; in `en` `Lang::abbreviate(1000.0, 0)` is not `1K`, `Lang::abbreviate(489939.0, 0)` is not `490K`, `Lang::abbreviate(1230000.0, 2)` is not `1.23M`, `Lang::abbreviate(-2500.0, 1)` is not `-2.5K`, or `Lang::abbreviate(999.0, 0)` is not `999`; or in `de` `Lang::abbreviate(1230000.0, 2)` is not `1,23M`.
Mechanism: `par-strings`.
Rationale: Laravel's `Number::percentage` and `Number::abbreviate`; ICU4X writes the percentage, and its compact format is not in the ICU4X release the framework uses, so the suffixes are Laravel's.
Status: Agreed 2026-10-03

## Schema dump

The developer ruled on 2026-10-01 to build `schema:dump`, at lower
priority: it writes a schema snapshot and its migration baseline, loads
it before newer migrations, prunes only when asked, and is tested on each
supported database. Laravel's `SchemaDumped`, `SchemaLoaded` and
`MigrationsPruned` events are not built: the migration commands run
before the bootstrap registers any listener, so none could hear them.

[PAR-038] `schema:dump` on the app binary MUST write the schema of the
`DATABASE_URL` database to `database/schema/<engine>-schema.sql`, where
the engine is `sqlite`, `postgres`, `mysql` or `mariadb`, or to the file
`--path` names: the statements that create every table, index, view and
constraint the database holds, without any table's rows, followed by one
`INSERT` for each row of the Migrator's ledger table (`seaql_migrations`
unless the Migrator names another). Postgres MUST be dumped with
`pg_dump`, MySQL with `mysqldump` and MariaDB with `mariadb-dump`, each
given the password through its environment or a file only the current
user can read, never as an argument; SQLite MUST be read through its own
connection. When the tool is missing or fails, the command MUST exit with
an error that names the tool, and an earlier dump file MUST stay as it
was. The developer CLI's `suprnova schema:dump` MUST run the app
binary's.
Falsifier: on any of SQLite, Postgres, MySQL and MariaDB, after `migrate` the dump file is absent, holds a row of a table other than the ledger, lacks a table or index the migrations created, or lacks an applied migration's ledger row; the password appears in the dump tool's arguments; or with the tool missing the command exits zero or changes an earlier dump file.
Mechanism: `par-schema-dump`.
Rationale: Laravel's `schema:dump` and its `SchemaState` classes, which run the same tools; the ledger is written by Suprnova, the same `INSERT` statements on every engine.
Status: Agreed 2026-10-03

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

[PAR-042] `MultipartRequestHooks` MUST offer `after_validation_async`.
A multipart request MUST run its stages in this order, each only after
the one before it succeeded: `authorize`, before the body is read;
extraction with its field validation (PAR-043); `after_validation`;
`after_validation_async`; the handler. A hook's non-empty
`ValidationErrors` MUST answer as a validation failure, a 422 whose body
holds `errors`, which the Inertia validation middleware turns into a
redirect back with the errors for an Inertia request; an empty set of
errors counts as success. The default `after_validation_async` succeeds.
Falsifier: a multipart request whose `after_validation_async` returns an error reaches its handler, answers anything but a 422 with that field's error in `errors`, or through Inertia does not redirect back with it in `props.errors`; a stage runs after an earlier one failed, the async hook before the sync one, or `authorize` after the body was read; or an empty error set fails the request.
Mechanism: `par-multipart-validation`.
Rationale: Issue #139: upload forms need database checks before the handler runs, which `FormRequest` already allows.
Status: Agreed 2026-10-04

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
(`validation-mimetypes`). Each message MUST come from the validation
catalog by that key, so an application's `lang/<locale>/validation.ftl`
overrides it. An `UploadValidator` MUST be able to return a validation
failure with a catalog key, apart from an operational error, which keeps
its own status. A field failure found while the body streams, as
`MaxSize` is, MUST stop reading the body after the chunk that crossed
the limit, skip both hooks and the handler, and remove every temporary
file the extraction wrote. A limit on the whole request, the body's byte
cap, `max_parts` and a field's `max_count`, MUST refuse with 413 without
reading the body further, and wins when one chunk crosses both kinds.
Falsifier: a multipart form missing a required file, or with a PDF as the second element of a `#[field("files[]")]` field of images, answers anything but a 422 whose `errors` holds `files.1`, or through Inertia does not redirect back with it in `props.errors`; a text part that does not parse answers 400; a message ignores an application catalog's entry for its key; an oversized file is read past the chunk that crossed `MaxSize`, or leaves a temporary file behind; a validator's operational error becomes a 422; or a body over its cap, too many parts or too many files for `max_count` is read further or answers other than 413.
Mechanism: `par-multipart-validation`.
Rationale: Issue #139: today these failures answer 400, 413 or a 422 without `errors`, so an Inertia form shows no error under the field; `max_count` answered 422 and moves to 413 with the other request-wide limits, as the issue asks.
Status: Agreed 2026-10-04

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

[PAR-045] An application MAY set `[package.metadata.suprnova.model]`
`datetime_cast` and `[package.metadata.suprnova.schema]` `unsigned_ids`
in a package's `Cargo.toml`; without them nothing changes. The model
table is read from the package that declares the model, by `#[model]`;
the schema table from the package of the binary, by `#[suprnova::main]`,
which installs it before anything runs. `datetime_cast = "native"` makes
every `DateTime<Utc>` and `Option<DateTime<Utc>>` field of a model with
no cast of its own, the managed timestamps included, use
`AsNativeDateTime` or `AsOptionalNativeDateTime`, a time-zone-aware
column; `"naive"` uses `AsNaiveDateTime` or `AsOptionalNaiveDateTime`, for
the time-zone-free columns Laravel creates on Postgres; a field's own
cast wins. The setting chooses casts and converts no column: the manual
and the scaffold's comment MUST say which column types each value needs
and show the per-field override for a column that differs. SQLite MAY
store a native date-time as its driver's text, so long as the value
round-trips. `unsigned_ids = true` makes `id()` and `foreign_id()` create
what `unsigned_id()` and `unsigned_foreign_id()` create, unsigned on
MySQL and unchanged on Postgres and SQLite, in every migration the
binary runs; a program that runs migrations without `#[suprnova::main]`
MUST be able to install the same setting with one documented call. A
key or value in either table that the framework does not know MUST fail
the build of the macro that reads it, naming it. The scaffold's
`Cargo.toml` MUST carry both settings commented out, saying they match
Laravel's MySQL schema.
Falsifier: with `datetime_cast = "native"` or `"naive"`, a model's managed timestamp uses another cast, or a field's explicit cast is replaced; a native value does not round-trip on any of SQLite, Postgres and MySQL; with `unsigned_ids = true`, `migrate` on the app binary against MySQL creates a signed `id` or foreign key, or changes a Postgres or SQLite column; a migrations library run by a binary with the setting misses it, or a program using the documented call does; without the tables any column or cast differs from today's; an unknown key or value builds; or a new scaffold lacks the commented settings.
Mechanism: `par-laravel-defaults`.
Rationale: Issue #137; the schema setting reaches the migrations through `#[suprnova::main]` because migrations build their schema at run time, where no macro reads `Cargo.toml`, and compiling it in keeps one schema for every environment.
Status: Agreed 2026-10-04
