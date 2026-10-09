# Running on a Laravel database

Status: Agreed 2026-10-05
Prefix: LDB

Drafted 2026-10-05 from issue #141 ("a Suprnova app should run against an
existing Laravel database without migrating anything"). The developer
agreed on 2026-10-04 that this is the next piece of work, and on
2026-10-05 ruled its scope to be the whole database Laravel created, the
application's own data included ("Yes I am accepting your
recommendations"). The builder's recommendations on the questions an
audit raised the same day were confirmed on 2026-10-05 and are written
into the requirements below. The plan for the framework's own tables was
posted on the issue on 2026-10-04, and the wider scope on 2026-10-05. The
Observed section describes framework `2bd4bd53d` (v3.2.1), checked
against the code by independent readers on 2026-10-05; the cited code is
unchanged at `c34ee7b15`. Laravel references cite
`reference/framework-13.27.0/src/Illuminate/`, its `config/`, and the
`reference/laravel-13.10.1` skeleton.

## Observed at 2bd4bd53d

Status: Observed

The tables the framework or the scaffold writes under names Laravel or a
package in scope also uses:

| Table | Suprnova | Laravel 13 or the package | Mismatch |
|---|---|---|---|
| `failed_jobs` | operator-managed, no shipped migration: `id` TEXT UUID primary key, `connection`, `queue`, `job_name`, `envelope_json` TEXT, `exception` TEXT, `failed_at` INTEGER epoch (`framework/src/queue/failed.rs:263-276,308,313-328,463`) | big-integer `id`, unique `uuid`, `connection`, `queue`, `payload` and `exception` long text, `failed_at` timestamp, index on (connection, queue, failed_at) (`Queue/Console/stubs/failed_jobs.stub:15-23`) | key, uuid, payload, failed_at, exception width, index |
| `jobs` | operator-managed: `id` TEXT UUID, `job_name`, `queue` NULL when unrouted, `envelope_json`, epoch `available_at` and `reserved_until`, `reserved_token`, `attempts`, `created_at` (`framework/src/queue/database.rs:453-467`) | big-integer `id`, `queue` NOT NULL, `payload` long text, `attempts`, `reserved_at`, `available_at`, `created_at` (`jobs.stub:15-21`); the `database` connection reads the queue `default` unless `DB_QUEUE` names another (skeleton `config/queue.php:42`) | every column but `attempts`, `available_at`, `created_at` |
| `job_batches` | operator-managed: `id`, `name`, `total_jobs`, `options_json`, epoch timestamps, plus `job_batch_settlements` (`framework/src/queue/batch.rs:406-426,785-787`) | `id`, `name`, `total_jobs`, NOT NULL `pending_jobs`, `failed_jobs` and `failed_job_ids`, a nullable `options`, integer timestamps (`batches.stub:15-24`) | the insert names `options_json`, which Laravel's table lacks, and leaves out the NOT NULL counters, which Suprnova derives from `job_batch_settlements` |
| `sessions` | scaffold migration: `id`, `user_id` text, `payload` JSON, `csrf_token` NOT NULL, `last_activity` datetime (`framework/src/session/driver/database.rs:57-81`; `suprnova-cli/src/templates/files/backend/migrations/create_sessions_table.rs.tpl:10-27`) | `id`, nullable big-integer `user_id`, `ip_address`, `user_agent`, `payload` as base64 of JSON under the skeleton's `'serialization' => 'json'` (skeleton `config/session.php:231`; PHP `serialize` when that key is absent, `Session/SessionManager.php:204,221`), integer `last_activity`; no `csrf_token` (`Session/Console/stubs/database.stub:15-20`) | columns, types and payload format |
| `notifications` | framework migration: `id` CHAR(36), `notifiable_id` VARCHAR(64), non-null timestamps (`framework/src/notifications/migrations/m_create_notifications_table.rs:46-78`); the channel writes `type` as the notification's `notification_name()` and `notifiable_type` as the string the application passes to `DatabaseChannel::new` (`framework/src/notifications/channels/database.rs:34,53-77`; `framework/src/notifications/mod.rs:68`); the read helpers decode `notifiable_id` as text (`framework/src/notifications/database_read.rs:36,56`) | uuid `id`, `notifiable_type` and big-integer `notifiable_id` from `morphs`, nullable `read_at` and timestamps; `type` and `notifiable_type` hold class names (`Notifications/Console/stubs/notifications.stub:15-20`) | on Postgres the insert of text into the uuid and big-integer columns errors; on MySQL and Postgres the reads cannot decode a big-integer `notifiable_id` as text; the manual's examples store `"OrderShipped"` and `"users"` (`manual/notifications.md:39,211`), not class names |
| `users` | scaffold migration and model: signed big-integer `id`, non-null `created_at` and `updated_at`; the model declares `id: i64` and non-`Option` timestamps (`suprnova-cli/src/templates/files/backend/migrations/create_users_table.rs.tpl:10-52`; `backend/models/user.rs.tpl:33-43`) | `id()`, unsigned on MySQL; nullable `email_verified_at` and timestamps; `remember_token` (skeleton `database/migrations/0001_01_01_000000_create_users_table.php:14-22`) | the model cannot read an unsigned `id` on MySQL or a NULL timestamp |
| `features` | framework migration: `name`, `scope_key`, `enabled`, `description`, `updated_by`, timestamps, unique (`name`, `scope_key`) (`framework/src/features/migrations/m_create_features_table.rs:7-17`) | laravel/pennant: `name`, `scope`, `value`, nullable timestamps, unique (`name`, `scope`) (its migration is not in `reference/`) | every column but `name` and the timestamps; `migrate` fails (below) |
| `roles`, `permissions` | framework migration: `name`, `guard_name`, nullable `display_name`, timestamps, unique (`name`, `guard_name`); assignments in `model_roles`, `model_permissions` and `role_permissions` (`framework/src/rbac/migrations/m_create_rbac_tables.rs:17-60,79-80`; `framework/src/rbac/entity.rs:25,52`) | spatie/laravel-permission: no `display_name`; assignments in `model_has_roles`, `model_has_permissions` and `role_has_permissions` (its migration is not in `reference/`) | the RBAC reads a `display_name` spatie's tables lack; assignments live in other tables |
| `cache`, `cache_locks` | not used: only memory and Redis drivers (`framework/src/cache/config.rs:17-24`) | `key`, `value`, `expiration` / `key`, `owner`, `expiration` (`Cache/Console/stubs/cache.stub:14-24`) | none |
| `password_reset_tokens` | not used; `auth_flow_tokens` instead (`framework/src/auth_flows/token_store.rs:448`) | `email` key, `token`, `created_at` | none |

`workflows` shares its name with the third-party laravel-workflow
package's table. Magnetar's tables are `auth_*` and the payments tables
`payments_*`; neither collides with Laravel or a first-party package.

Framework and scaffold migrations change a table Laravel or a package
created when it already exists:

- The scaffold's sessions migration skips the create on Laravel's
  `sessions`, then adds `idx_sessions_user_id` and
  `idx_sessions_last_activity` to it (`create_sessions_table.rs.tpl:13,30-50`).
- `CreateNotificationsTable` adds two indexes to Laravel's
  `notifications` (`m_create_notifications_table.rs:87-106`), and
  `NotificationTimestampsToDatetime` converts its MySQL `TIMESTAMP`
  columns to `DATETIME`
  (`framework/src/notifications/migrations/m_notifications_timestamps_to_datetime.rs:36-43`).
- `CreateFeaturesTable` skips the create on Pennant's `features`
  (`m_create_features_table.rs:72`), then fails creating its unique
  index on `scope_key`, a column Pennant's table lacks (:104-113), so
  `migrate` fails. `FeatureTimestampsToDatetime` would convert Pennant's
  MySQL `TIMESTAMP` columns
  (`framework/src/features/migrations/m_features_timestamps_to_datetime.rs:36`).
  The dogfood app reads `features` at boot (`app/src/bootstrap.rs:229`).
- `CreateRbacTables` skips the creates on spatie's `roles` and
  `permissions`, then adds a unique index on (`name`, `guard_name`) to
  each (`m_create_rbac_tables.rs:102-112,153-163`).

A deployed Suprnova application that uses the database queue holds
`failed_jobs`, `jobs` and `job_batches` in the layouts above, created by
hand from `manual/queues.md:1215-1250` or by its own migration, as the
dogfood app does (`app/src/migrations/m_2026_08_01_queue_tables.rs:40-143`);
a scaffolded one holds `sessions` in the scaffold's layout. An operator
can already choose other names: `QUEUE_DB_TABLE` and
`QUEUE_FAILED_DB_TABLE` (`framework/src/queue/mod.rs:1859,1885-1886`),
`SESSION_TABLE` (`framework/src/session/config.rs:233`) and
`DatabaseBatchRepository::with_tables` (`framework/src/queue/batch.rs:497-509`).

No worker checks its failed-jobs table before it starts.
`run_labelled_worker` enters its loop directly
(`framework/src/queue/worker.rs:736-750`), and the store's first use is
`handle_dead_letter` (:1726-1747). `run_worker` returns `()` (:698-710);
`run_worker_on` returns a `Result` (:726-734), which `queue:work` turns
into exit status 1 (`framework/src/app/mod.rs:1801-1806`).

The failure the issue reports holds for a store the application binds
itself: the failed-job insert names columns Laravel's table lacks, the
worker does not acknowledge the job, and each redelivery dead-letters it
again, so the insert is retried every visibility timeout without end
(`framework/src/queue/worker.rs:929-945,1664-1671,1726-1747`). The store is
bound automatically only for `QUEUE_DRIVER=database`
(`framework/src/queue/mod.rs:1885-1888`), and on a pure Laravel database
the `jobs` insert fails first.

Passwords. The framework's bcrypt driver writes `$2b$`
(`framework/src/hashing/driver.rs:140`), marks `$2a$`, `$2x$` and `$2y$`
hashes for a rehash to `$2b$` (:183-186), and verifies a `$2y$` hash
(:162), but reports a mismatch for any password over 71 bytes before it
verifies (:148-152; `MAX_BCRYPT_PASSWORD_BYTES`,
`framework/src/hashing/mod.rs:119`). Magnetar applies the same cap
(`crates/suprnova-magnetar/src/password/hash.rs:25,440-442`) and upgrades
every bcrypt hash to Argon2id on a valid sign-in (:243-247,362-367).
Laravel 13 hashes a password of any length by its first 72 bytes
(`BCRYPT_LIMIT` unset, `config/hashing.php:34`) and, with `HASH_VERIFY`
true by default (:33), throws on a hash whose `password_get_info`
algorithm is not bcrypt (`Hashing/BcryptHasher.php:82-92`). PHP 8.4
reports `$2b$` as `unknown`.

Polymorphic types. A model registers one stored type string, its
`morph_type` (`framework/src/eloquent/relations/morph_registry.rs:29-47`).
The macro takes any string literal (`suprnova-macros/src/model/parse.rs:978`),
so `morph_type = "App\\Models\\Post"` resolves a class name Laravel
wrote, but no model can accept a second name for itself.

Remember-me. The framework keeps selector and verifier rows in its own
`remember_tokens` table (`framework/src/auth/remember.rs:441-465`) and
never reads `users.remember_token`. Magnetar's password reset clears the
user's remember-token column (`crates/suprnova-magnetar/src/storage/tokens.rs:355`).

Columns. PAR-044 and PAR-045 (Agreed 2026-10-04) give `u64` keys and the
`datetime_cast` and `unsigned_ids` settings. Soft deletes use
`deleted_at` by default (`framework/src/eloquent/soft_deletes.rs:67-69`).
Every stored time is read as a UTC wall clock
(`framework/src/database/stored_datetime.rs:13-17`).

## Tables and layouts

[LDB-001] A Suprnova application MUST run on a database Laravel 13
created, with the tables of Laravel, of Laravel's first-party packages
and of spatie/laravel-permission; tables of other third-party packages
are outside this spec. Every table the framework or the scaffold
creates, reads or writes under a name one of these uses MUST take that
table's layout: its columns, types, nullability, keys and indexes as
Laravel's or the package's migration creates them, except that a table
Suprnova creates holds its time columns as DATETIME on MySQL where that
migration says TIMESTAMP (LDB-011). Table by table: `failed_jobs`
(LDB-002), `jobs`, `job_batches`, `sessions` and `notifications` take
Laravel's layout, under Laravel's names by default, with
`notifications.notifiable_id` in the form `NOTIFICATIONS_MORPH_KEY`
selects, as Laravel's `morphs` follows its default morph key type: `int`
(the default) for Laravel's big-integer form, or `uuid` or `ulid` for the
forms `uuidMorphs` and `ulidMorphs` create; and with `sessions.user_id`
as Laravel's nullable big integer, or, when the default guard's user
model has a `unique_id` key, as the nullable column `foreignUuid` creates
for a UUID key or `foreignUlid` for a ULID key; the scaffold's `users`
takes the Laravel 13 skeleton's; `features` takes laravel/pennant's; and
`roles` and `permissions` take spatie/laravel-permission's (LDB-008). Data the framework keeps that a
layout has no column for MUST live in a table under a name none of these
uses, as `job_batch_settlements` does. `cache`, `cache_locks`,
`password_reset_tokens` and the tables of the first-party packages not
named here share no name with a framework table and stay unused.
Falsifier: with the committed Laravel 13 schema loaded on SQLite, MySQL or Postgres, a framework feature that uses `failed_jobs`, `jobs`, `job_batches`, `sessions`, `notifications`, `users`, `features`, `roles` or `permissions` fails on it or needs a column it lacks; or, on an empty database, a table the framework's or the scaffold's migrations create under one of these names differs from Laravel's or the package's layout in a column, type, nullability, key or index, other than DATETIME for TIMESTAMP on MySQL, with `notifications.notifiable_id` checked under `NOTIFICATIONS_MORPH_KEY` set to `int`, `uuid` and `ulid`, and `sessions.user_id` with an integer-keyed and a `unique_id`-keyed user model.
Mechanism: `laravel-database`.
Rationale: Ruled 2026-10-05: the scope is the whole database, the framework's tables use Laravel's layouts, and the tables of Laravel's first-party packages and of spatie/laravel-permission are in scope. A Suprnova layout under a Laravel name breaks both sides: Suprnova's inserts fail on Laravel's table, and Laravel's code fails on Suprnova's rows (Observed).
Status: Agreed 2026-10-05

[LDB-011] A table Laravel or a package in scope created MUST stay as
Laravel left it: the framework's and the scaffold's migrations MUST NOT
create, alter or index it, and `migrate` MUST succeed on it. The
migrations that change such a table MUST skip it:
`CreateNotificationsTable` and `NotificationTimestampsToDatetime` on
`notifications`, `CreateFeaturesTable` and `FeatureTimestampsToDatetime`
on `features`, `CreateRbacTables` on `roles` and `permissions`, and the
scaffold's sessions migration on `sessions`. A table Suprnova creates
under a name in LDB-001 MUST follow the framework's rule for MySQL time
columns: DATETIME, never TIMESTAMP.
Falsifier: after the framework's and a fresh scaffold's migrations run on SQLite, MySQL or Postgres against the committed Laravel 13 schema, `migrate` fails, or a table Laravel or a package created differs from before in a column, type, nullability, default or index; or on MySQL a table Suprnova created under a name in LDB-001 has a TIMESTAMP column.
Mechanism: `laravel-database`.
Rationale: Ruled 2026-10-05: tables Laravel created stay exactly as Laravel left them, and tables Suprnova creates follow the framework's DATETIME rule on MySQL, where TIMESTAMP refuses any time after 2038-01-19.
Status: Agreed 2026-10-05

[LDB-010] The framework's shipped migrations MUST upgrade an existing
Suprnova application in place. They MUST reshape a `failed_jobs` table
in the earlier Suprnova layout into Laravel's, move the earlier `jobs`,
`job_batches` and `sessions` tables, and reshape the earlier
`notifications`, `features`, `roles` and `permissions` tables, into the
layouts of LDB-001, with their rows, keeping the data those layouts have
no column for in tables of their own (LDB-001), so no queued, delayed,
reserved or failed job is lost or run twice, every batch keeps its
counts, no signed-in user is signed out, and no notification, flag, role,
permission or assignment is lost. Every other framework table an earlier
release created MUST keep working after the upgrade. A `users` table is
the application's own migration, which the framework never reshapes; the
manual gives the migration that moves an earlier scaffold's `users` to
the skeleton's layout.
Falsifier: on SQLite, MySQL or Postgres holding the earlier layouts with rows, after `migrate` a job queued, delayed or reserved before it does not run exactly once, a failed job cannot be listed, retried or forgotten by its id, a batch reports other total, pending or failed counts, or a session issued before it no longer authenticates or loses its CSRF token; or after `migrate` a `notifications`, `features`, `roles` or `permissions` table an earlier release created is not in the layout of LDB-001, or a notification, flag, role, permission or assignment it held is lost.
Mechanism: `laravel-database`.
Rationale: Ruled 2026-10-05: shipped migrations upgrade existing Suprnova applications in place, so no queued job is stranded and no user is signed out. A change of default alone would strand both, and LDB-003 would refuse to start their workers. Reshaping the earlier `notifications`, `features`, `roles` and `permissions` keeps one layout per name, as LDB-001 requires, so the feature flags and the RBAC never support two.
Status: Agreed 2026-10-05

## Queue tables

[LDB-002] `failed_jobs` MUST take Laravel 13's layout: a big-integer
`id`, a unique `uuid`, `connection`, `queue`, long-text `payload` and
`exception`, `failed_at`, and the index on (`connection`, `queue`,
`failed_at`). Each failed job MUST get a fresh uuid, never its
envelope's id, and `payload` MUST be the envelope with the job's name
added as `displayName` and that uuid as `uuid`. `connection` MUST name
the Suprnova connection in a form no connection in Laravel 13's default
`config/queue.php` takes, so Laravel's `queue:retry` refuses the row.
The framework MUST ship the migration that creates the table, with
`failed_at` as DATETIME on MySQL. `queue:failed`, `queue:retry`,
`queue:forget` and `queue:flush` MUST work on a table Laravel created
and on one the shipped migration created. A row Laravel wrote MUST list,
and `queue:retry` MUST refuse it with an error naming its uuid.
Falsifier: on a `failed_jobs` table Laravel 13 or the shipped migration created, on SQLite, MySQL or Postgres: a dead-lettered job's row reuses the envelope id or shares its uuid with another row, or a second dead-letter of the same envelope is refused; its `payload` does not decode to the envelope with `displayName` equal to the job name and `uuid` equal to the row's; its `connection` is `sync`, `database`, `beanstalkd`, `sqs`, `redis`, `deferred`, `background` or `failover`; `failed_at` is more than a second from the failure; a 1 MiB payload or exception is truncated or refused; `queue:failed` does not list the row; `queue:retry <uuid>` does not push its envelope and delete the row; `queue:forget <uuid>` does not delete it; `queue:flush --hours=N` deletes a row newer than N hours or keeps an older one; `queue:retry` pushes a row Laravel wrote; or the shipped migration's columns, nullability or index differ from Laravel's stub other than DATETIME for `failed_at` on MySQL.
Mechanism: `laravel-database`.
Rationale: Ruled 2026-10-05: a fresh uuid per row, because the envelope id would make a redelivered dead-letter violate `uuid UNIQUE` and loop without end (`framework/src/queue/worker.rs:1655-1662`); `uuid` in the payload, which Laravel's provider reads when it logs a failure again (`Queue/Failed/DatabaseUuidFailedJobProvider.php:55-58`); and a connection Laravel does not have, so its retry throws before it forgets the row (`Queue/Console/RetryCommand.php:47-58,141-148`; `Queue/QueueManager.php:199-201`).
Status: Agreed 2026-10-05

[LDB-003] Before its first pop, a worker started by `queue:work`,
`run_worker_on` or `run_worker` whose failed-jobs store is the database
store MUST check that table and refuse to start, with an error naming
the table and the missing or mismatched column, when the table is
missing, lacks a column the store writes, is in the earlier Suprnova
layout, or on MySQL holds `payload` or `exception` narrower than
LONGTEXT. `queue:work` MUST then exit
non-zero. `run_worker` MUST return a `Result` that carries this error,
as `run_worker_on` does, because public surface returns `Result` and
never panics. The check MUST accept a table Laravel 13 or the shipped
migration created.
Falsifier: with the database failed-jobs store, `queue:work`, `run_worker_on` or `run_worker` pops a job, or returns without an error before shutdown, against a failed-jobs table that is missing, lacks a column the store writes, is in the earlier Suprnova layout, or on MySQL holds a TEXT `payload` or `exception`; its error does not name the table and the column; `queue:work` exits zero after refusing; `run_worker` has no way to return the error; a worker refuses a table Laravel 13 or the shipped migration created; or a worker whose failed-jobs store is not the database store refuses to start over that table.
Mechanism: `laravel-database`.
Rationale: Ruled 2026-10-05: `run_worker` refusing to start needs a fallible public API. A worker that starts on a table it cannot write retries its first failure every visibility timeout without end (Observed), and a MySQL TEXT column that refuses a payload over 64 KiB starts the same loop.
Status: Agreed 2026-10-05

## The application's data

[LDB-004] A bcrypt hash Laravel wrote (`$2y$`) MUST verify through
`Auth::attempt` and through Magnetar for every password Laravel
accepted, passwords of 72 bytes and longer included, which bcrypt judges
by their first 72 bytes as PHP does. By default the framework MUST keep
writing `$2b$` hashes and Magnetar MUST keep upgrading a bcrypt hash to
Argon2id on a valid sign-in. An application that shares its database
with a Laravel application MUST be able to turn on one setting under
which every password hash the framework and Magnetar write is a `$2y$`
bcrypt hash, a valid sign-in rewrites a `$2b$` or Argon2id hash as one,
and Magnetar upgrades none to Argon2id, so Laravel 13 with
`HASH_VERIFY=true` verifies every password Suprnova signed in, reset or
registered.
Falsifier: a user whose `$2y$` hash Laravel 13 wrote in the committed fixtures, 72-byte and 100-byte passwords included, cannot sign in through `Auth::attempt` or Magnetar with the password Laravel hashed; with default settings a hash the framework writes is not `$2b$`, or Magnetar does not upgrade a bcrypt hash to Argon2id on a valid sign-in; with the setting on, a hash the framework or Magnetar writes on sign-in, password reset or registration is not reported as `bcrypt` by PHP 8.4's `password_get_info` or fails its `password_verify`; or, with the setting on, a user holding a `$2b$` or Argon2id hash signs in through Suprnova and the stored hash is not then a `$2y$` hash that PHP 8.4's `password_verify` accepts.
Mechanism: `laravel-database`.
Rationale: Ruled 2026-10-05: Suprnova keeps `$2b$` and the Argon2id upgrade by default, and a setting gives both up for an application that shares its database with Laravel, whose hasher throws on a `$2b$` or Argon2id hash while `HASH_VERIFY` is true (Observed); passwords of 72 bytes and longer that Laravel accepted verify on Suprnova.
Status: Agreed 2026-10-05

[LDB-005] A polymorphic relation MUST resolve a type column holding a
PHP class name, such as `App\Models\Post`, when the model declares it as
its `morph_type`. A model MUST be able to register extra aliases that
reads accept: a `MorphTo` loads the owner whose `morph_type` or alias
the row holds, and a `MorphOne` or `MorphMany`, direct or eager, matches
rows holding either. Writes MUST store the `morph_type`.
Falsifier: over rows Laravel wrote, some holding `App\Models\Post` and some `post` for one model that declares one as its `morph_type` and the other as an alias, a `MorphTo`, `MorphOne` or `MorphMany`, direct or eager, misses or misloads a row; or a write stores an alias.
Mechanism: `laravel-database`.
Rationale: Ruled 2026-10-05: a Laravel database that adopted `Relation::morphMap` late holds both forms for one model, and the registry holds one string per model (Observed).
Status: Agreed 2026-10-05

[LDB-006] Beyond PAR-044 and PAR-045, a model over a table Laravel
created MUST read and write rows whose `created_at` and `updated_at` are
NULL, soft-delete through `deleted_at`, and read and write Laravel's
`json` columns, on SQLite, MySQL and Postgres. The scaffold's `User`
MUST read and write the `users` table Laravel 13's skeleton creates,
whose `id` is unsigned on MySQL and whose timestamps are nullable.
Falsifier: on SQLite, MySQL or Postgres, a row Laravel wrote with NULL `created_at`, `updated_at` or `deleted_at`, or a value in a `json` column, fails to read or does not round-trip through a model; a soft delete of a Laravel row leaves `deleted_at` NULL, or the default query returns a row Laravel trashed; or a fresh scaffold's `User` fails to find, update or create a row in a `users` table Laravel created.
Mechanism: `laravel-database`.
Rationale: PAR-044 and PAR-045 give unsigned keys and Laravel's date-time columns; nullable timestamps and the scaffold's `User` are what they leave out (Observed).
Status: Agreed 2026-10-05

[LDB-007] Suprnova MUST leave `users.remember_token` as Laravel left
it, so Laravel's own remember-me keeps working on a shared database:
sign-in, remember-me sign-in, sign-out and session revocation, through
the framework, Magnetar or the scaffold, MUST NOT write the column. A
password reset MAY clear it, as the reset Laravel documents replaces it
(`reference/docs-13.x/passwords.md:175`). Accepting a remember-me cookie
Laravel issued is outside this spec.
Falsifier: after a Suprnova sign-in with and without remember-me, a sign-in through Suprnova's remember-me cookie, a sign-out, or a revocation of the user's sessions, `users.remember_token` differs from the value Laravel stored.
Mechanism: `laravel-database`.
Rationale: Ruled 2026-10-05: Suprnova leaves `remember_token` intact, and accepting a Laravel-issued remember cookie is out of scope.
Status: Agreed 2026-10-05

[LDB-008] On a database where laravel/pennant created `features`, the
framework's feature flags MUST read and write the flags Pennant stores
there: a flag Pennant stored for the global scope or for a model reads
the same through the framework, a stored value other than `false` reads
as enabled, as Pennant's `active()` does, and the framework stores `true`
or `false`, as the row Pennant would store. On a database where
spatie/laravel-permission created its tables, the RBAC MUST read and
write `roles` and `permissions` in spatie's layout, without
`display_name`, MUST honor the role and permission assignments spatie
recorded, and MUST read and write assignments in `model_has_roles`,
`model_has_permissions` and `role_has_permissions`, with `model_type`
written as the model's `morph_type` (LDB-005).
Falsifier: against the committed Pennant and spatie fixture rows on SQLite, MySQL or Postgres, a framework flag check answers enabled for a stored `false` or disabled for any other stored value, a rich value such as `"blue"` included; a flag the framework stores differs in `name`, `scope` or `value` from the row Pennant wrote for the same flag, value and scope; a role or permission query fails for want of `display_name`; a user spatie assigned a role or permission, directly or through a role, fails the framework's check for it; or a role or permission the framework assigns is missing from `model_has_roles`, `model_has_permissions` or `role_has_permissions`, or is written with a `model_type` other than the model's `morph_type`.
Mechanism: `laravel-database`.
Rationale: Ruled 2026-10-05: spatie/laravel-permission is in scope because its `roles` and `permissions` collide with the framework's RBAC, which reads a `display_name` spatie lacks, and Pennant is a first-party package whose `features` collides with the framework's (Observed). Pennant counts a feature active when its value is anything other than `false` (`reference/docs-13.x/pennant.md:699`).
Status: Agreed 2026-10-05

## Sharing the database

[LDB-009] A Laravel 13 application MUST keep working on the database
while a Suprnova application, with the setting of LDB-004 on, writes to
it. Each row Suprnova writes into a table Laravel or a package created
MUST decode through Laravel's or the package's reader of that table, a
session row as base64 JSON, as the Laravel 13 skeleton configures it, and
Laravel MUST sign in every user Suprnova signed in, reset or registered.
Suprnova's session garbage collection MUST NOT delete a session row it
did not write. With that setting on, Suprnova's default queue MUST be one
Laravel's default worker does not read, so Laravel's worker with its
default settings MUST NOT reserve a job Suprnova queued with its default
settings, and a Suprnova worker with its default settings MUST NOT
reserve a job Laravel queued; with the setting off, the default queue
stays `default`.
Falsifier: on SQLite, MySQL or Postgres holding the committed Laravel 13 schema and rows, a row Suprnova writes into `jobs`, `job_batches`, `failed_jobs`, `sessions`, `notifications` or `features` fails the decode that Laravel 13's or the package's reader of that table performs, reproduced in the test from that source; a password hash Suprnova writes fails the PHP check of LDB-004; with the setting on, a job Suprnova queues with default settings lands on the queue `default`, which Laravel's `database` connection reads by default, or with the setting off it lands anywhere else; with the setting on, a Suprnova worker with default settings reserves a job a Laravel fixture row queued; or Suprnova's session garbage collection deletes a session row a Laravel fixture wrote.
Mechanism: `laravel-database`.
Rationale: The issue asks that a Laravel and a Suprnova application use one database at the same time. No Laravel application runs at test time, so each table's reader is reproduced from Laravel's source and passwords are checked with the host `php` (Mechanism).
Status: Agreed 2026-10-05

## Documentation

[LDB-012] The manual MUST have a chapter on running a Suprnova
application on a Laravel database, linked from `manual/documentation.md`,
with a runnable example, a `### Why Suprnova diverges` section and a
closing `## Next`. It MUST name each table of LDB-001 with the layout it
takes, the upgrade of LDB-010 and what it moves, the setting of LDB-004
and what it gives up, and how a model registers morph aliases. It MUST
say that an application whose `users.id` is unsigned on MySQL sets
`unsigned_ids = true` (PAR-045), and that Laravel's `queue:retry all`
stops at the first failed job Suprnova wrote, while
`queue:retry --queue=<queue>` naming a queue only Laravel uses retries
Laravel's own. The scaffold's comment on `unsigned_ids` MUST say that the
scaffold's `users.id` is unsigned on MySQL. The chapter MUST state that
Suprnova does not support Sanctum tokens Laravel issued, columns
Laravel's encrypter wrote (Fortify's two-factor secrets and recovery
codes, and any `encrypted` cast), remember-me cookies Laravel issued, or
a Laravel application or database server whose time zone is not UTC.
Falsifier: a test reading the chapter finds it missing or not linked from `manual/documentation.md`, without a runnable example, a `### Why Suprnova diverges` section or a closing `## Next`, or not naming a table of LDB-001, the upgrade, the setting of LDB-004, morph aliases, `unsigned_ids = true` for an unsigned `users.id` on MySQL, how Laravel's `queue:retry all` and `queue:retry --queue` treat the rows Suprnova wrote, or one of the unsupported items: Laravel-issued Sanctum tokens, columns Laravel's encrypter wrote, Laravel-issued remember-me cookies, time zones other than UTC; or a new scaffold's comment on `unsigned_ids` does not say its `users.id` is unsigned on MySQL.
Mechanism: `laravel-database`.
Rationale: Ruled 2026-10-05: Laravel-issued Sanctum tokens, columns encrypted by Laravel's encrypter and non-UTC time zones are out of scope, and the manual says so.
Status: Agreed 2026-10-05

## Mechanism

Ruled 2026-10-05: `laravel-database` runs this spec's tests on SQLite
and on the standing Postgres and MySQL services, each in a throwaway
database it drops on exit. The tests load, for each engine, the Laravel
13 schema Laravel's migrations create on it, since they create a
different one on each, and rows committed beside them, generated once
with PHP 8.4 from a Laravel 13 skeleton with the in-scope packages
installed, and written by Laravel itself: `$2y$` users,
among them passwords of 72 bytes and longer, polymorphic rows holding a
class name and an alias for one model, a notification, a queued job, a
failed job, a batch, Pennant flags, and spatie roles, permissions and
assignments. A test MAY call the host `php` for password checks only
(`password_get_info`, `password_verify`). No Laravel application runs at
test time, so what Laravel's code does with a row Suprnova wrote is
reproduced from Laravel's source.
