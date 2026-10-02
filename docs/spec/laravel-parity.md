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
location. The report MUST NOT reach the response's headers or body.
Falsifier: a handler or middleware that returns an error, or panics, yields a response without a report; a report holds an error from another request; or a report's text appears in the response's headers or body.
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
