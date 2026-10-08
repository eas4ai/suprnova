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
`enabled` unset records only when `APP_ENV` is set and names `local`
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
Falsifier: with `enabled` unset a request under `APP_ENV=local` leaves no entry or one under an unset or production `APP_ENV` leaves one; with `enabled(false)` a request under `APP_ENV=local` leaves an entry, or with `enabled(true)` one under a production `APP_ENV` leaves none; a request to `/_inertia/devtools/entries` or `/_suprnova/health` is recorded; a request under an `except` pattern of its own is recorded; a prop value that cannot be serialized, or a storage path that cannot be written, changes the status, body or headers of the response beyond the devtools headers; or a write failure is logged on every request.
Mechanism: `par-inertia-devtools`.
Rationale: Rows DT-01 and DT-10; Laravel's `DevTools::enabled` defaults to the `local` environment, `devtools.except` skips its own and other tooling's paths, and `RequestRecorder::respondedWith` swallows every failure so a passive observer cannot turn the user's response into a 500.
Status: Agreed 2026-10-08

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
