# Running on a Laravel database

Status: Draft
Prefix: LDB

Drafted 2026-10-05 from issue #141 ("a Suprnova app should run against an
existing Laravel database without migrating anything"). The developer
agreed on 2026-10-04 that this is the next piece of work, and on
2026-10-05 accepted the recommendation to do it, noting that its scope had
narrowed from the issue's ("I think you changed the scope a bit"). Which
scope is still his to rule (LDB-004). The plan for the framework's own
tables was posted on the issue on 2026-10-04. The Observed section
describes framework `2bd4bd53d` (v3.2.1), checked against the code and
Laravel 13's stubs by an independent reader on 2026-10-05.

## Observed at 2bd4bd53d

Status: Observed

The tables the framework writes under names Laravel also uses:

| Table | Suprnova | Laravel 13 | Mismatch |
|---|---|---|---|
| `failed_jobs` | operator-managed, no shipped migration: `id` TEXT UUID primary key, `job_name`, `envelope_json` TEXT, `exception` TEXT, `failed_at` INTEGER epoch (`framework/src/queue/failed.rs:263-276,308,313-328,463`) | big-integer `id`, unique `uuid`, `connection`, `queue`, `payload` and `exception` long text, `failed_at` timestamp, index on (connection, queue, failed_at) (`Queue/Console/stubs/failed_jobs.stub:15-23`) | key, uuid, payload, failed_at, exception width, index |
| `jobs` | operator-managed: `id` TEXT UUID, `job_name`, `queue` NULL when unrouted, `envelope_json`, epoch `available_at` and `reserved_until`, `reserved_token`, `attempts`, `created_at` (`framework/src/queue/database.rs:453-467`) | big-integer `id`, `queue` NOT NULL, `payload` long text, `attempts`, `reserved_at`, `available_at`, `created_at` (`jobs.stub:15-21`) | every column but `attempts`, `available_at`, `created_at` |
| `job_batches` | operator-managed: `id`, `name`, `total_jobs`, `options_json`, epoch timestamps, plus `job_batch_settlements` (`framework/src/queue/batch.rs:406-426,785-787`) | adds NOT NULL `pending_jobs`, `failed_jobs`, `failed_job_ids`, and `options` (`batches.stub:15-24`) | the insert leaves out NOT NULL counters, which Suprnova derives |
| `sessions` | scaffold migration: `id`, `user_id` text, `payload` JSON, `csrf_token` NOT NULL, `last_activity` datetime (`framework/src/session/driver/database.rs:57-81`) | adds `ip_address`, `user_agent`; integer `user_id` and `last_activity`; no `csrf_token` | columns, types and payload format |
| `notifications` | framework migration: `id` CHAR(36), `notifiable_id` VARCHAR(64), non-null timestamps; `type` is the notification's name and `notifiable_type` a table name (`framework/src/notifications/channels/database.rs:53-77`) | uuid `id`, morphs (big-integer id), nullable timestamps; `type` and `notifiable_type` hold class names | on Postgres, text into a uuid or big-integer column errors; type values differ |
| `cache`, `cache_locks` | not used: only memory and Redis drivers (`framework/src/cache/config.rs:17-24`) | `key`, `value`, `expiration` / `key`, `owner`, `expiration` | none today |
| `password_reset_tokens` | not used; `auth_flow_tokens` instead | `email` key, `token`, `created_at` | none today |

Also written under shared names: the scaffold's `users` table and the
framework's `features` table, not yet compared with Laravel's.

The failure the issue reports holds for a store the application binds
itself: the failed-job insert names columns Laravel's table lacks, the
worker does not acknowledge the job, and each redelivery dead-letters it
again, so the insert is retried every visibility timeout without end
(`framework/src/queue/worker.rs:929-945,1664-1671,1726-1747`). The store is
bound automatically only for `QUEUE_DRIVER=database`
(`framework/src/queue/mod.rs:1885-1888`), and on a pure Laravel database
the `jobs` insert fails first.

## Requirements

[LDB-001] For every table the framework creates, scaffolds, reads or
writes under a name Laravel also uses, `users` and `features` included, the
work MUST decide, table by table and from the comparison above, either to
use Laravel's layout or to move to a table name Laravel does not use, and
record the decision in this spec. A table a Laravel application and a
Suprnova application would both use differently at the same time
(`jobs`, `sessions`, `job_batches`) needs its own name.
Falsifier: on a database Laravel 13 created, a framework feature writing one of these tables fails or writes a row Laravel cannot read, or a table's decision is not recorded.
Mechanism: `laravel-database` (engine tests on MySQL and Postgres against a schema Laravel created).
Rationale: The plan posted on the issue on 2026-10-04, refined by the inventory: a Laravel worker sharing `jobs` would reserve Suprnova rows and fail on them.
Status: Draft

[LDB-002] `failed_jobs` MUST take Laravel's layout, with the job name inside
`payload` as `displayName` and long text for `payload` and `exception`,
and the framework MUST ship its migration.
Falsifier: a failed job on a Laravel-created `failed_jobs` table errors on insert, or a payload over 64 KiB is truncated or refused.
Mechanism: `laravel-database`.
Status: Draft

[LDB-003] A queue worker MUST check its failed-jobs table when it starts
and refuse to start on one it cannot write, naming the table and the
mismatch.
Falsifier: a worker starts against an unwritable failed-jobs table and retries its first failure without end.
Mechanism: `laravel-database`.
Status: Draft

## The application's data

[LDB-004] The scope covers the application's own data as Laravel wrote it,
inventoried first and then specified table by table: `$2y$` bcrypt
password hashes MUST sign in; polymorphic type columns holding PHP class
names MUST resolve; `remember_token` MUST work; Laravel's timestamp,
soft-delete and JSON column conventions MUST read and write; package
tables such as Sanctum's `personal_access_tokens` MUST be inventoried and
decided like the framework's own (LDB-001); and a Laravel and a Suprnova
application MUST be able to use one database at the same time.
Falsifier: a user created by Laravel cannot sign in to the Suprnova application; a morph relation Laravel wrote does not resolve; or a Laravel application sharing the database breaks on a row Suprnova wrote.
Mechanism: `laravel-database`.
Rationale: The issue asks for the whole database; the developer ruled for it on 2026-10-05 ("Yes I am accepting your recommendations").
Status: Draft
