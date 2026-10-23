# Running on a Laravel Database

A Suprnova application can run on a database a Laravel 13 application
created, and it can run next to that Laravel application on the same
database. This chapter lists the tables Suprnova shares with Laravel and the
layout each one takes, explains how an existing Suprnova application
upgrades its own tables to those layouts, and describes the one setting an
application turns on when a Laravel application uses the database at the
same time.

## The tables Suprnova shares with Laravel

Every table the framework or a new scaffold creates under a name that
Laravel, a first-party Laravel package or spatie/laravel-permission also
uses takes that table's layout: the same columns, types, nullability, keys
and indexes that Laravel's or the package's migration creates. There is one
difference. On MySQL, a table Suprnova creates holds its time columns as
`DATETIME` where Laravel's migration says `TIMESTAMP`, because `TIMESTAMP`
refuses any time after 2038-01-19.

| Table | Layout | Created by |
|---|---|---|
| `jobs` | Laravel's `jobs` | `queue::migrations::CreateJobsTable` |
| `job_batches` | Laravel's `job_batches` | `queue::migrations::CreateJobBatchesTable` |
| `failed_jobs` | Laravel's `failed_jobs` | `queue::migrations::CreateFailedJobsTable` |
| `sessions` | the Laravel 13 skeleton's `sessions` | `session::migrations::CreateSessionsTable` |
| `notifications` | Laravel's `notifications` | `notifications::migrations::CreateNotificationsTable` |
| `users` | the Laravel 13 skeleton's `users` | the scaffold's `create_users_table` |
| `features` | laravel/pennant's `features` | `features::migrations::CreateFeaturesTable` |
| `roles`, `permissions` | spatie/laravel-permission's | `rbac::migrations::CreateRbacTables` |
| `model_has_roles`, `model_has_permissions`, `role_has_permissions` | spatie/laravel-permission's | `rbac::migrations::CreateRbacTables` |

Each migration creates its table when it is missing, and leaves a table
Laravel or a package already created exactly as it is. It never alters or
indexes Laravel's table.

Some columns follow the key type of your models, as Laravel's do:

- `notifications.notifiable_id` follows `NOTIFICATIONS_MORPH_KEY`: `int`
  (the default) for Laravel's `morphs`, `uuid` for `uuidMorphs`, or `ulid`
  for `ulidMorphs`.
- `sessions.user_id` follows the key of the default guard's user model.
  Pass `SessionUserKey::Integer` (Laravel's `foreignId`), `SessionUserKey::Uuid`
  (`foreignUuid`) or `SessionUserKey::Ulid` (`foreignUlid`) to
  `CreateSessionsTable::new`.
- `model_has_roles.model_id` and `model_has_permissions.model_id` follow
  `RBAC_MODEL_KEY`: `int`, `uuid` or `ulid`.

Data the framework keeps that Laravel's layout has no column for lives in
tables of its own, under names Laravel does not use:

| Framework table | Holds |
|---|---|
| `suprnova_jobs_reservations` | the token and deadline of each job a worker reserved |
| `job_batch_settlements` | which jobs of a batch settled, so a redelivered job is not counted twice |
| `suprnova_feature_details` | a flag's description and the actor who last changed it |
| `suprnova_role_details`, `suprnova_permission_details` | the display names an earlier release kept |

`cache`, `cache_locks`, `password_reset_tokens` and the tables of the other
first-party packages share no name with a framework table, and Suprnova
leaves them alone.

### What each table holds

- `jobs`: a Suprnova job's `payload` is its envelope with Laravel's
  `displayName` and `uuid` added. Suprnova's workers only reserve rows
  Suprnova wrote, so a Suprnova worker never runs a Laravel job.
- `failed_jobs`: each failed job gets a fresh `uuid`. Its `connection` is
  `suprnova:` followed by the Suprnova connection name, a form no
  connection in Laravel's default `config/queue.php` takes.
- `job_batches`: the counters are kept in Laravel's columns, and `options`
  holds a PHP-serialized array that Laravel's batch repository reads.
- `sessions`: the `payload` is base64-encoded JSON, as the Laravel 13
  skeleton serializes it. Session garbage collection only deletes rows
  Suprnova wrote.
- `features`: a flag is stored as Pennant stores it. The global scope is
  `__laravel_null`, and a user's scope is `App\Models\User|<id>`.
  `FEATURES_USER_SCOPE` names another class or morph alias. A stored value
  other than `false` reads as enabled, as Pennant's `active()` decides.
- `users`: Suprnova never writes `remember_token`, so Laravel's own
  remember-me keeps working. A password reset may clear it, as Laravel's
  documented reset does.

## Registering the migrations

List the framework's migrations in your `Migrator`, after your own:

```rust,no_run
use sea_orm_migration::{MigrationTrait, MigratorTrait};
use suprnova::features::migrations::{
    CreateFeaturesTable, FeatureTimestampsToDatetime, FeaturesToPennantLayout,
};
use suprnova::notifications::migrations::{
    CreateNotificationsTable, NotificationTimestampsToDatetime, NotificationsToLaravelLayout,
};
use suprnova::queue::migrations::{CreateFailedJobsTable, CreateJobBatchesTable, CreateJobsTable};
use suprnova::rbac::migrations::{CreateRbacTables, RbacToSpatieLayout};
use suprnova::session::migrations::{CreateSessionsTable, SessionUserKey};

pub struct Migrator;

impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            // ... your own migrations, the users table first ...
            Box::new(CreateSessionsTable::new(SessionUserKey::Integer)),
            Box::new(CreateJobsTable),
            Box::new(CreateJobBatchesTable),
            Box::new(CreateFailedJobsTable),
            Box::new(CreateNotificationsTable),
            Box::new(NotificationTimestampsToDatetime),
            Box::new(NotificationsToLaravelLayout),
            Box::new(CreateFeaturesTable),
            Box::new(FeatureTimestampsToDatetime),
            Box::new(FeaturesToPennantLayout),
            Box::new(CreateRbacTables),
            Box::new(RbacToSpatieLayout),
        ]
    }
}
```

Then run `suprnova migrate`. On a database Laravel created, the migrations
add only the framework's own tables listed above.

## Upgrading an existing Suprnova application

An application an earlier release created has its own layouts under these
names. The same migrations upgrade it in place when `suprnova migrate`
runs:

- `failed_jobs` is reshaped into Laravel's layout. Each failed job keeps its
  id as its `uuid`, so `queue:failed`, `queue:retry <id>` and
  `queue:forget <id>` still find it.
- `jobs` is moved into Laravel's layout. A queued job stays queued, a
  delayed job stays delayed, and a reserved job stays reserved by the same
  token until the same deadline, so the worker that holds it settles it and
  no job runs twice.
- `job_batches` is moved into Laravel's layout, and each batch keeps its
  total, pending and failed counts.
- `sessions` is moved into the skeleton's layout. Every session keeps its
  id, its user, its data and its CSRF token, so nobody is signed out.
- `notifications`, `features`, `roles` and `permissions` are reshaped into
  the layouts above, with every notification, flag, role, permission and
  assignment. The earlier `model_roles`, `model_permissions` and
  `role_permissions` are moved into spatie's tables and dropped.

Each table is copied aside as `suprnova_earlier_<table>` before it is
reshaped, and its rows move in chunks. A `migrate` that stops part way
starts again where it stopped on the next run, and moves no row twice.
Every other table an earlier release created keeps working unchanged.

The framework never reshapes `users`: it is your application's own
migration. This migration moves an earlier scaffold's `users` into the
Laravel 13 skeleton's layout, keeping every row and id. If your `users` has
columns of your own, add them to `Schema::create` and to both column lists.

```rust,no_run
use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_orm::DbBackend;
use suprnova::schema::Schema;

/// Moves the `users` table an earlier scaffold created into the Laravel 13
/// skeleton's layout. Every row keeps its id. A run that stops part way
/// starts again from `users_earlier` on the next `migrate`.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        if !manager.has_table("users_earlier").await? {
            db.execute_unprepared("CREATE TABLE users_earlier AS SELECT * FROM users")
                .await?;
        }
        Schema::drop_if_exists(manager, "users").await?;
        Schema::create(manager, "users", |t| {
            t.unsigned_id();
            t.string("name");
            t.string("email").unique();
            t.date_time("email_verified_at").precision(0).nullable();
            t.string("password");
            t.remember_token();
            t.date_time("created_at").precision(0).nullable();
            t.date_time("updated_at").precision(0).nullable();
        })
        .await?;
        db.execute_unprepared(
            "INSERT INTO users (id, name, email, email_verified_at, password, \
             remember_token, created_at, updated_at) \
             SELECT id, name, email, email_verified_at, password, remember_token, \
             created_at, updated_at FROM users_earlier",
        )
        .await?;
        Schema::drop_if_exists(manager, "users_earlier").await?;
        if manager.get_database_backend() == DbBackend::Postgres {
            // The rows kept their ids; the next one follows the largest.
            db.execute_unprepared(
                "SELECT setval(pg_get_serial_sequence('users', 'id'), \
                 COALESCE((SELECT MAX(id) FROM users), 0) + 1, false)",
            )
            .await?;
        }
        Ok(())
    }

    /// Leaves the skeleton's layout, which holds every earlier row.
    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}
```

After it runs, the scaffold's `User` model reads `id` into a `u64`, and its
timestamps into `Option<DateTime<Utc>>`.

## Sharing the database with a running Laravel application

When a Laravel application uses the database at the same time, turn on one
setting:

```bash
LARAVEL_SHARED_DATABASE=true
```

or call `LaravelDatabase::share(true)` in your bootstrap. With the setting
on:

- Every password hash the framework and Magnetar write is a `$2y$` bcrypt
  hash, which Laravel's hasher accepts with `HASH_VERIFY=true`. A valid
  sign-in rewrites a `$2b$` or Argon2id hash as `$2y$`, at the configured
  bcrypt cost or at the stored hash's cost when that is higher, and
  Magnetar no longer upgrades bcrypt hashes to Argon2id. So Laravel signs
  in every user Suprnova signed in, registered or reset. If Suprnova can't
  write the `$2y$` hash, the sign-in fails with that error, because Laravel
  couldn't sign that user in. The stored hash stays as it was, and the next
  sign-in tries the rewrite again.
- A job queued with default settings is stored on the queue `suprnova`
  instead of `default`. Laravel's `database` connection reads `default`, so
  Laravel's worker never reserves a Suprnova job. Inside Suprnova the
  queue keeps its name: a job routed to `default` and a worker started with
  `--queue=default` both use the stored `suprnova`.

The setting gives up two things the framework does by default:

- Argon2id. Without the setting, Magnetar upgrades a bcrypt hash to
  Argon2id on a valid sign-in. With it, every hash stays bcrypt, which
  judges a password by its first 72 bytes.
- The stored queue name `default`. A tool that reads the `jobs` table
  directly, Laravel's `queue:monitor` included, sees `suprnova` for
  Suprnova's unrouted jobs.

Without the setting, `$2y$` hashes Laravel wrote still verify through
`Auth::attempt` and Magnetar, passwords of 72 bytes and longer included.

### Laravel's `queue:retry` and Suprnova's failed jobs

Laravel's `queue:retry` cannot retry a failed job Suprnova wrote, because
Laravel has no queue connection named `suprnova:<connection>`. So
`queue:retry all` stops at the first failed job Suprnova wrote. Run
`queue:retry --queue=<queue>` with a queue only Laravel uses, and Laravel
retries its own failed jobs. Retry Suprnova's failed jobs with Suprnova's
`queue:retry`.

## Polymorphic types and morph aliases

Laravel stores a PHP class name, such as `App\Models\Post`, in a
polymorphic `*_type` column. Declare it as the model's `morph_type`. A
Laravel application that adopted `Relation::morphMap` late holds both the
class name and the alias, such as `post`, for one model. Register the
alias with `morph_aliases`:

```rust,no_run
use suprnova::model;

#[model(
    table = "posts",
    morph_type = "App\\Models\\Post",
    morph_aliases = ["post"],
    relations = {
        comments: MorphMany<Comment> { name = "commentable" },
    },
)]
pub struct Post {
    pub id: u64,
    pub title: String,
}

#[model(table = "comments", relations = {
    commentable: MorphTo { name = "commentable", targets = [Post] },
})]
pub struct Comment {
    pub id: u64,
    pub commentable_type: String,
    pub commentable_id: u64,
    pub body: String,
}
```

Reads accept every name: a `MorphTo` loads the post a row names by either
form, and a `MorphOne` or `MorphMany`, direct or eager, matches rows that
hold either. Writes store the `morph_type`. The RBAC stores a model's
`morph_type` as spatie's `model_type`, so declare `morph_type =
"App\\Models\\User"` on your user model when spatie's tables hold Laravel's
assignments.

## Unsigned keys on MySQL

Your new tables use unsigned IDs on MySQL by default. You use `id()` and
`foreign_id()` to match Laravel's keys, and `timestamps()` to create
nullable timestamps. A new scaffold's `users.id` is already unsigned.
Declare unsigned keys as `u64` in your models.

If your existing tables use signed IDs, keep that shape in new migrations
with the opt-out:

```toml
[package.metadata.suprnova.schema]
unsigned_ids = false
```

`#[suprnova::main]` installs it before migrations. Without that entry point,
you call `suprnova::schema::Schema::set_unsigned_ids(false)` before your first
migration. You keep signed columns on Postgres and SQLite with either
setting. See [Migrations](migrations.md).

## What Suprnova does not support

- Sanctum tokens Laravel issued. Suprnova does not read
  `personal_access_tokens`.
- Columns Laravel's encrypter wrote, such as Fortify's two-factor secrets
  and recovery codes, and any column with an `encrypted` cast. Suprnova
  cannot decrypt them.
- Remember-me cookies Laravel issued. A user signed in to Laravel signs in
  to Suprnova separately.
- A Laravel application or a database server whose time zone is not UTC.
  Suprnova reads every stored time as UTC.

### Why Suprnova diverges

Laravel owns each of these tables in its own application, and a second
application writing to them is not a case Laravel plans for. Suprnova takes
Laravel's layouts so both applications read the same rows, and it keeps
what Laravel's layouts cannot hold in tables of its own instead of adding
columns to Laravel's tables. A failed job's connection names a connection
Laravel does not have, so Laravel's `queue:retry` refuses it instead of
running a job Laravel cannot decode. The setting is off by default because
`$2b$`, Argon2id and the queue name `default` are the better choices for an
application that does not share its database.

## Next

- [Database](database.md) - connections, transactions and the query builder
- [Migrations](migrations.md) - the schema builder and `unsigned_ids`
- [Queues](queues.md) - workers, failed jobs and batches
- [From Laravel](from-laravel.md) - how Laravel habits map to Suprnova
