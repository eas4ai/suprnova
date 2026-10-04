# Migrations

Migrations describe how your schema evolves - each file is a small Rust struct with `up()` and `down()` methods that the framework runs in timestamp order. Use them whenever you change tables, columns, indexes, or foreign keys; that change moves from your laptop to staging to production by running the same migrate command in each place.

Suprnova's migrations are SeaORM migrations underneath. The CLI generates them, the `Migrator` aggregates them, and `Application::migrations::<Migrator>()` plugs them into your app's boot. For the full per-command reference (flags, output samples, exit codes) see [CLI Migrations Reference](cli-migrations.md); this chapter covers what to put *inside* the files.

## Creating migrations

Generate a new migration file:

```bash
suprnova make:migration create_users_table
```

The generator writes a timestamped file under `src/migrations/` (creating the
directory the first time) and registers it in the `Migrator`:

```
src/migrations/
├── mod.rs                              ← the Migrator (CLI-managed)
└── m20240115_120000_create_users_table.rs
```

The filename is `m{YYYYMMDD}_{HHMMSS}_<name>.rs`; ordering is by filename, so
the timestamp prefix is what enforces a deterministic apply order.

### What the generator emits

`make:migration create_users_table` produces this skeleton:

```rust
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Users::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Users::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(Users::CreatedAt)
                            .timestamp()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .col(
                        ColumnDef::new(Users::UpdatedAt)
                            .timestamp()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Users::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
    CreatedAt,
    UpdatedAt,
}
```

The generator infers the table name from the migration name
(`create_X_table` → `X`, `add_Y_to_X` → `X`, `drop_X_table` → `X`). Anything
else becomes the literal name.

### The Migrator

`src/migrations/mod.rs` collects every migration into a single `Migrator`
that `MigratorTrait` walks. The CLI maintains this file when you
`make:migration`, so you rarely touch it by hand:

```rust
pub use sea_orm_migration::prelude::*;

mod m20240115_120000_create_users_table;
mod m20240115_130000_create_posts_table;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20240115_120000_create_users_table::Migration),
            Box::new(m20240115_130000_create_posts_table::Migration),
        ]
    }
}
```

Wire the migrator into your app's `main.rs` so `serve`, `migrate`,
`migrate:status`, `migrate:rollback`, and `migrate:fresh` all see the same
list:

```rust
use suprnova::Application;

#[suprnova::main]
async fn main() {
    Application::new()
        .config(my_app::config::register)
        .bootstrap(my_app::bootstrap::bootstrap)
        .routes(my_app::routes::register)
        .migrations::<my_app::migrations::Migrator>()
        .run()
        .await
}
```

The scaffolder writes this for you on `suprnova new`.

### Why Suprnova diverges

Most of the framework deliberately hides SeaORM - you write `#[suprnova::model]`
and `User::query().db_where(...)`, not `Entity::find().filter(...)`. Migrations
are the one place where SeaORM stays visible. Two reasons.

First, the schema builder compiles to SeaORM's own statements
(`Table::create()`, `Table::alter()`, `Index`, `ForeignKey`) and runs them on
the migration's `SchemaManager`. It adds no migration engine, so SeaORM is one
step away for everything the builder does not cover: a column type change, a
check constraint, a raw statement. You write that step with
`sea_orm_migration::prelude::*` in the same `up()`. Second, migration files are
pure Rust - your CI compiler verifies them.

If you need a SeaORM type the framework has not re-exported, the escape hatch
is `use suprnova::sea_orm;`.

The builder differs from Laravel's `Blueprint` in four ways:

- `timestamps()` and `soft_deletes()` create string columns, not native
  date-time columns. A model stores a `DateTime<Utc>` field as RFC 3339 text
  by default, and the string column is the one type that round-trips on all
  three backends. Laravel's native forms are `timestamps_tz()`,
  `datetimes()`, `soft_deletes_tz()` and `soft_deletes_datetime()`, paired
  with a native cast on the model.
- There is no table rebuild on SQLite. An operation SQLite cannot run in place
  returns an error that names the operation.
- There is no column type change. `Schema::table` adds, renames and drops
  columns, but it does not alter the type of an existing column.
- `suprnova make:migration` generates the SeaORM form. The schema builder is
  the alternative you write by hand.

See [The schema builder](#the-schema-builder).

## Migration structure

Every migration has two methods:

```rust
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    // Apply the change
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> { /* ... */ }

    // Reverse the change
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> { /* ... */ }
}
```

Both arms return `Result<(), DbErr>` - bubble errors with `?` and the framework
turns a failed migration into a non-zero exit so deploy pipelines abort.

## The schema builder

`suprnova::schema::Schema` is a shorter way to write the common migration. A
closure records columns, indexes and foreign keys on a `Blueprint`, and the
builder turns the description into SQL. This is the posts migration as a
complete file:

```rust
use sea_orm_migration::prelude::*;
use suprnova::schema::Schema;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        Schema::create(manager, "posts", |t| {
            t.id();
            t.string("title");
            t.text("body").nullable();
            t.string("slug").length(120).unique();
            t.boolean("published").default(false);
            t.foreign_id("author_id")
                .constrained("users")
                .on_delete(ForeignKeyAction::Cascade);
            t.index(&["published", "created_at"]);
            t.timestamps();
            t.soft_deletes();
        })
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        Schema::drop_if_exists(manager, "posts").await
    }
}
```

Every `Schema` function is `async`, takes the migration's `manager`, and
returns `Result<_, DbErr>`. The builder opens no connection of its own, so each
statement runs on the migration's connection and inside its transaction.

The layer lives at `suprnova::schema::Schema`. Import it by that path.
`suprnova::Schema` at the crate root is SeaORM's `Schema`, a different type.

### Entry points

| Function | Effect |
|----------|--------|
| `Schema::create(manager, table, closure)` | Creates the table, then its indexes. Foreign keys are part of `CREATE TABLE`. |
| `Schema::table(manager, table, closure)` | Alters an existing table. |
| `Schema::drop(manager, table)` | Drops the table. It is an error if the table does not exist. |
| `Schema::drop_if_exists(manager, table)` | Drops the table if it exists. |
| `Schema::rename(manager, from, to)` | Renames a table. |
| `Schema::has_table(manager, table)` | Returns `Result<bool, DbErr>`: whether the table exists. |
| `Schema::has_column(manager, table, column)` | Returns `Result<bool, DbErr>`: whether the column exists. |

### Column methods

A column is `NOT NULL` unless you call `.nullable()`. The table lists the column
each method creates. The SQLite column is the type name SQLite stores, and its
affinity is what the database reports.

| Method | SQLite | Postgres | MySQL |
|--------|--------|----------|-------|
| `id()` | `integer`, auto-increment, primary key | `bigserial`, primary key | `bigint`, auto-increment, primary key |
| `unsigned_id()` | as `id()` | as `id()` | `bigint unsigned`, auto-increment, primary key |
| `foreign_id(name)` | `integer` | `bigint` | `bigint` |
| `unsigned_foreign_id(name)` | `integer` | `bigint` | `bigint unsigned` |
| `big_integer(name)` | `integer` | `bigint` | `bigint` |
| `integer(name)` | `integer` | `integer` | `int` |
| `small_integer(name)` | `integer` | `smallint` | `smallint` |
| `tiny_integer(name)` | `integer` | `smallint` | `tinyint` |
| `unsigned_big_integer(name)`, `unsigned_integer(name)`, `unsigned_small_integer(name)`, `unsigned_tiny_integer(name)` | the signed type | the signed type | the type, `unsigned` |
| `boolean(name)` | `boolean` | `boolean` | `tinyint(1)` |
| `string(name)` | `varchar(255)` | `varchar(255)` | `varchar(255)` |
| `char(name, length)` | `char(length)` | `char(length)` | `char(length)` |
| `text(name)` | `text` | `text` | `text` (64 KB) |
| `medium_text(name)` | `text` | `text` | `mediumtext` (16 MB) |
| `long_text(name)` | `text` | `text` | `longtext` (4 GB) |
| `enumeration(name, &[values])` | `varchar` with `CHECK (name IN (..))` | `varchar(255)` with `CHECK (name IN (..))` | `enum(..)` |
| `float(name)` | `float` | `real` | `float` |
| `double(name)` | `double` | `double precision` | `double` |
| `decimal(name, precision, scale)` | `decimal(precision, scale)` | `numeric(precision, scale)` | `decimal(precision, scale)` |
| `date(name)` | `date_text` | `date` | `date` |
| `time(name)` | `time_text` | `time` | `time` |
| `date_time(name)` | `datetime_text` | `timestamp` | `datetime` |
| `timestamp_tz(name)` | `timestamp_with_timezone_text` | `timestamp with time zone` | `timestamp` |
| `json(name)` | `json_text` | `jsonb` | `json` |
| `uuid(name)` | `char(36)` | `uuid` | `char(36)` |
| `ulid(name)` | `char(26)` | `char(26)` | `char(26)` |
| `binary(name)` | `blob` | `bytea` | `blob` |
| `remember_token()` | nullable `varchar(100)` named `remember_token` | the same | the same |

`id()` is a `BIGINT` on Postgres and MySQL, and `foreign_id` has the same type.
The types match because MySQL refuses a foreign key between columns of
different types. A table has one `id()`.

Where the databases differ, the builder does what Laravel does rather than
refusing: `unsigned` applies on MySQL only, because Postgres and SQLite have no
unsigned integers; `tiny_integer`, `medium_text` and `long_text` take the
nearest type; and `enumeration` becomes a string with a `CHECK` off MySQL. An
`enumeration` value may contain a quote; the builder escapes it. The migration
fails on a value listed twice, which MySQL refuses, and on a value with a
backslash, which MySQL and Postgres read differently depending on server
settings.

A model field has to match the column's width on Postgres, whose driver reads
each integer type into one Rust type only: `i16` for `tiny_integer` and
`small_integer`, `i32` for `integer`, `i64` for `big_integer`, `id()` and
`foreign_id`. MySQL and SQLite read any signed integer column into `i64`.

#### Laravel tables on MySQL

Laravel's `id()` and `foreignId()` are `BIGINT UNSIGNED` on MySQL, and MySQL
refuses a foreign key whose type differs from the referenced column's, sign
included. A new table that points at a Laravel `users` table uses
`unsigned_foreign_id`:

```rust
Schema::create(manager, "orders", |t| {
    t.unsigned_id();
    t.unsigned_foreign_id("user_id").constrained("users").cascade_on_delete();
    t.enumeration("status", &["draft", "paid"]).default("draft");
    t.timestamps_tz();
})
.await?;
```

A model reads an unsigned column into an unsigned field: the MySQL driver
refuses to read an unsigned column into a signed type, `BIGINT UNSIGNED` into
an `i64` included. Declare the key `u64`, and the foreign key fields `u64` to
match. The model takes its key type from the key field, so it needs no
`key_type`:

```rust
#[model(table = "orders")]
pub struct Order {
    pub id: u64,
    pub user_id: u64,
    pub status: String,
}
```

The same model runs on Postgres and SQLite, which have no unsigned integers.
There, `unsigned_id()` creates a signed `BIGINT`, and a `u64` field holds `0`
to `i64::MAX` (9223372036854775807). Writing a larger value fails with an
error that names the column, before anything reaches the database. It is a
database error, so a client gets the generic 500 response and the log gets
the detail. Reading by a larger value finds nothing, because no row holds
one: `find` returns `None`, and a route that binds such a key answers 404. A
negative value in the column fails the read, naming the column. The tests of
a Laravel port can therefore run on the SQLite `TestDatabase`.

To give every migration Laravel's keys without writing `unsigned_id()`, set
`unsigned_ids` in the `Cargo.toml` of the package that builds your binary:

```toml
[package.metadata.suprnova.schema]
unsigned_ids = true
```

`#[suprnova::main]` reads the setting when the binary is built and installs it
before `main` runs. From then on, `id()` creates what `unsigned_id()` creates
and `foreign_id()` what `unsigned_foreign_id()` creates, in every migration
the binary runs, a library's migrations included. Postgres and SQLite columns
don't change. A key or a value the framework doesn't know fails the build and
names it. A new project carries the setting commented out in `Cargo.toml`.

A program that runs migrations without `#[suprnova::main]` installs the same
setting with one call before its first migration:

```rust
suprnova::schema::Schema::use_unsigned_ids();
```

### Modifiers

Each column method returns a builder. Chain the modifiers on it.

| Modifier | Effect |
|----------|--------|
| `.nullable()` | Allows `NULL`. |
| `.default(value)` | Sets the value the database stores when an insert leaves the column out. It takes a plain Rust value (`7`, `"draft"`, `false`) or a SeaQuery `Expr` such as `Expr::current_timestamp()`. |
| `.unique()` | Adds a unique index over this column alone, named `{table}_{column}_unique`. |
| `.length(n)` | Sets the length of a `string` column. On any other column type the migration fails with an error that names the column. Use `char(name, length)` for a fixed length. |
| `.index()` | Adds an index over this column alone, named `{table}_{column}_index`. |
| `.primary()` | Makes this column the primary key. For a key over several columns, `t.primary(&[..])`. |
| `.unsigned()` | `UNSIGNED` on MySQL, for an integer column. Ignored on Postgres and SQLite. On any other column type the migration fails. |
| `.precision(n)` | Fractional-second digits, 0 to 6, of a `date_time`, `timestamp_tz` or `time` column. MySQL keeps whole seconds without it, Postgres microseconds; SQLite stores text and ignores it. |
| `.use_current()` | Defaults a `date_time` or `timestamp_tz` column to the current time, `CURRENT_TIMESTAMP`. Combined with `.default(..)` the migration fails. |
| `.after(column)` | Places a column that `Schema::table` adds after `column`, on MySQL. Ignored on Postgres and SQLite; `Schema::create` refuses it, because MySQL does. |

A native date-time column on MySQL keeps whole seconds unless you set a
precision, and drops the fraction a model writes: MySQL rounds it, MariaDB
truncates it. Declare
`t.timestamp_tz("paid_at").precision(6)` to keep microseconds. MySQL's
`TIMESTAMP` also ends in 2038; `date_time` does not.

### Timestamps and soft deletes

`t.timestamps()` adds `created_at` and `updated_at`, both `NOT NULL`.
`t.soft_deletes()` adds a nullable `deleted_at`. These are `VARCHAR(255)`
columns on every backend, not native date-time columns.

The reason is the model. A `#[suprnova::model]` field of type `DateTime<Utc>`
with no declared cast uses `AsDateTime`, which stores RFC 3339 text
(see [Eloquent Mutators](eloquent-mutators.md)). Postgres refuses a text
parameter for a `timestamp` column. A string column round-trips on all three
backends.

For native date-time columns, use Laravel's helpers and give the model's
fields the matching cast (see
[Native date-time casts](eloquent-mutators.md#native-date-time-casts)):

| Helper | Columns | Cast |
|---|---|---|
| `t.timestamps_tz()` | nullable `created_at`, `updated_at` with a time zone | `AsNativeDateTime` |
| `t.soft_deletes_tz()` | nullable `deleted_at` with a time zone | `AsOptionalNativeDateTime` |
| `t.datetimes()` | nullable `created_at`, `updated_at` without a time zone | `AsNaiveDateTime` |
| `t.soft_deletes_datetime()` | nullable `deleted_at` without a time zone | `AsOptionalNaiveDateTime` |

```rust
Schema::create(manager, "orders", |t| {
    t.id();
    t.string("reference");
    t.timestamps_tz();
    t.soft_deletes_tz();
})
.await?;
```

```rust
use suprnova::{AsNativeDateTime, AsOptionalNativeDateTime, model};

#[model(
    table = "orders",
    soft_deletes,
    casts = {
        created_at = AsNativeDateTime,
        updated_at = AsNativeDateTime,
        deleted_at = AsOptionalNativeDateTime,
    },
)]
pub struct Order {
    pub id: i64,
    pub reference: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub deleted_at: Option<chrono::DateTime<chrono::Utc>>,
}
```

Declare a single native column with `t.timestamp_tz("published_at")` or
`t.date_time("published_at")` and the same casts.

To give every model in a package one of these casts without naming it on each
field, set `datetime_cast` in the package's `Cargo.toml`. For more
information, see
[Native date-time casts](eloquent-mutators.md#native-date-time-casts).

### Indexes and foreign keys

```rust
Schema::create(manager, "comments", |t| {
    t.id();
    t.foreign_id("post_id")
        .constrained("posts")
        .on_delete(ForeignKeyAction::Cascade)
        .on_update(ForeignKeyAction::Cascade);
    t.string("author");
    t.index(&["post_id", "author"]);
    t.unique(&["post_id", "author"]);
})
.await
```

- `t.index(&["a", "b"])` creates an index named `{table}_{columns}_index`.
- `t.unique(&["a", "b"])` creates a unique index named `{table}_{columns}_unique`.
- The columns are joined with `_` in the name.
- Indexes are separate `CREATE INDEX` statements that run after the table.
- `foreign_id(name).constrained(table)` creates a foreign key to the `id`
  column of `table`, named `{table}_{column}_foreign`.
- `.references(table, column)` points the key at another column. On MySQL that
  column must be a `BIGINT`.
- `.on_delete(action)` and `.on_update(action)` take a `ForeignKeyAction`. The
  database default (`NO ACTION`) applies when you do not call them.
- A `foreign_id` with no `constrained` or `references` call is a plain `BIGINT`
  column.
- `.cascade_on_delete()`, `.restrict_on_delete()`, `.null_on_delete()` and
  `.no_action_on_delete()` are short for `.on_delete(..)` with that action, and
  the `_on_update` forms for `.on_update(..)`.
- `.name("orders_state_fk")` names the constraint instead of
  `{table}_{column}_foreign`. A name used twice in one closure, by two keys or
  by a key and an index, fails the migration.
- `.null_on_delete()` and `.null_on_update()` need a nullable column. On a
  column the closure declares `NOT NULL` the migration fails before its first
  statement, because MySQL would refuse the key only after adding the column.
- `t.foreign("state_id")` creates a foreign key on a column declared on its own,
  or one the table already has in `Schema::table`. It takes the same
  `.references(table, column)`, `.constrained(table)`, `.name(..)` and actions,
  and without a referenced table the migration fails.
- `t.primary(&["post_id", "tag_id"])` makes a composite primary key, as a pivot
  table needs. A table has one primary key: `id()` and `primary` together fail,
  and so does a nullable column in the key.

```rust
Schema::create(manager, "post_tag", |t| {
    t.foreign_id("post_id").constrained("posts").cascade_on_delete();
    t.foreign_id("tag_id").constrained("tags").cascade_on_delete();
    t.primary(&["post_id", "tag_id"]);
})
.await
```

The builder refuses a name longer than 63 bytes for an index or a foreign key,
on every backend. It is the limit Postgres keeps, so a migration that runs on
one backend runs on all three.

### Altering a table

`Schema::table` accepts:

- new columns of any type, with the same modifiers
- `rename_column(from, to)` and `drop_column(name)`
- `index`, `unique` and `drop_index(name)`
- `foreign_id(..).constrained(..)`, `foreign(column)` and `drop_foreign(name)`
- `drop_constrained_foreign_id(column)`, which drops the key
  `{table}_{column}_foreign` and then the column. A key `.name(..)` named has
  another name: drop it with `drop_foreign(name)` and the column with
  `drop_column`.
- `primary(&[..])`, on Postgres and MySQL

```rust
Schema::table(manager, "posts", |t| {
    t.string("subtitle").nullable();
    t.rename_column("body", "content");
    t.index(&["subtitle"]);
})
.await
```

It runs the operations in the order you write them, each as its own statement.
Changing the type of an existing column is not supported. `rename_column`,
`drop_column`, `drop_index` and `drop_foreign` belong to `Schema::table`:
`Schema::create` returns an error if its closure records one.

The builder checks the description for the mistakes it can see before it runs
the first statement: a duplicate column, an index over a column the table does
not declare, an empty table name. A statement the database refuses stops the
call. On MySQL and SQLite the migrator runs a migration without a transaction,
so the statements before the refused one stay applied. On Postgres the
migration's transaction rolls them back.

Every refusal the builder makes is a `DbErr::Migration` whose text starts with
`schema:` and names the table, the column and the operation.

### SQLite limits

SQLite cannot run some alterations in place, and the builder does not rebuild
the table. `Schema::table` on SQLite returns an error, before it runs any
statement of the call, for:

- Adding or dropping a foreign key, `drop_constrained_foreign_id` included.
- Adding a primary key column, or a primary key.
- Adding a `NOT NULL` column with no default.
- Adding a column with `.use_current()`.

The errors read:

```text
schema: cannot add the foreign key `{name}` on the existing table `{table}`: SQLite cannot add or drop a foreign key on an existing table; create the key with the table in Schema::create, or write this step with SeaORM's SchemaManager
schema: cannot add the primary key column `{column}` to the existing table `{table}`: SQLite cannot add a PRIMARY KEY column; create the table with id() instead
schema: cannot add the NOT NULL column `{column}` to the existing table `{table}` without a default: SQLite refuses it; call .nullable() or .default(value)
```

Dropping a foreign key reads `cannot drop the foreign key` in the first text.

SQLite also refuses `CURRENT_TIMESTAMP` as the default of a column added to an
existing table, so use a constant default there; the builder refuses
`.use_current()` up front, and cannot see a `CURRENT_TIMESTAMP` passed through
`.default(..)`. It refuses to drop a column
that an index, a unique constraint or a foreign key covers: record the
`drop_index` earlier in the same closure.

### Why Suprnova diverges

- `id()` is a signed `BIGINT` on MySQL by default, where Laravel's is
  unsigned, because the MySQL driver reads a signed column only into a signed
  field, and Rust code usually keys a row with `i64`. `unsigned_id()` and
  `unsigned_foreign_id()` create Laravel's types one column at a time, and
  `unsigned_ids = true` under `[package.metadata.suprnova.schema]` makes
  `id()` and `foreign_id()` create them in every migration.
- Laravel's `enum` is `enumeration`: `enum` is a Rust keyword.
- `references(table, column)` takes both names in one call, where Laravel
  chains `->references($column)->on($table)`.
- `.unsigned()` applies to integers only. Laravel also lets it mark a decimal
  or floating point column; MySQL deprecated that in 8.0.17.
- Laravel's timestamp columns default to whole seconds (`timestamp(0)`) on
  Postgres. The builder keeps each database's own default, microseconds on
  Postgres and whole seconds on MySQL, unless `.precision(n)` sets one.
- `use_current()` together with `.default(..)` fails the migration. Laravel
  lets `useCurrent` win silently.

### Both styles in one Migrator

A `Schema` migration and a SeaORM migration implement the same
`MigrationTrait`, so both sit in one `Migrator`. One `up()` can mix them:
call `Schema::create` for the table, then a `manager` call of your own for
the step the builder does not cover.

## Schema operations

These are the SeaORM forms of the same operations. The [schema builder](#the-schema-builder) is the shorter alternative for the common cases.

### Creating tables

```rust
use sea_orm_migration::prelude::*;

async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
        .create_table(
            Table::create()
                .table(Users::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(Users::Id)
                        .integer()
                        .not_null()
                        .auto_increment()
                        .primary_key(),
                )
                .col(ColumnDef::new(Users::Email).string().not_null().unique_key())
                .col(ColumnDef::new(Users::Name).string().not_null())
                .col(ColumnDef::new(Users::PasswordHash).string().not_null())
                .col(ColumnDef::new(Users::CreatedAt).timestamp().not_null())
                .col(ColumnDef::new(Users::UpdatedAt).timestamp().not_null())
                .to_owned(),
        )
        .await
}

// Define the table and column identifiers
#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
    Email,
    Name,
    PasswordHash,
    CreatedAt,
    UpdatedAt,
}
```

### Dropping tables

```rust
async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
        .drop_table(Table::drop().table(Users::Table).to_owned())
        .await
}
```

### Column types

| Method | Database Type | Notes |
|--------|---------------|-------|
| `integer()` | INTEGER | 32-bit integer |
| `big_integer()` | BIGINT | 64-bit integer |
| `small_integer()` | SMALLINT | 16-bit integer |
| `float()` | FLOAT | Floating point |
| `double()` | DOUBLE | Double precision |
| `decimal()` | DECIMAL | Fixed-point |
| `string()` | VARCHAR(255) | Variable length string |
| `string_len(n)` | VARCHAR(n) | Custom length string |
| `text()` | TEXT | Long text |
| `boolean()` | BOOLEAN | True/false |
| `timestamp()` | TIMESTAMP | Date and time |
| `date()` | DATE | Date only |
| `time()` | TIME | Time only |
| `blob()` | BLOB | Binary data |
| `json()` | JSON | JSON data |
| `uuid()` | UUID | UUID type |

### Column modifiers

```rust
ColumnDef::new(Column::Name)
    .string()
    .not_null()                                // NOT NULL constraint
    .null()                                    // Allows NULL (default)
    .default("value")                          // Default value
    .default(Expr::current_timestamp())        // Function default (e.g. NOW())
    .unique_key()                              // UNIQUE constraint
    .primary_key()                             // PRIMARY KEY
    .auto_increment()                          // AUTO_INCREMENT
```

For surrogate primary keys, prefer `big_integer().auto_increment().primary_key()`
on real tables - `INTEGER` (32-bit) is fine for tiny lookup tables but the
scaffolded `users` table uses `BIGINT` because a 4-byte counter is the kind of
constraint you regret three years in.

## Adding columns

```rust
async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
        .alter_table(
            Table::alter()
                .table(Users::Table)
                .add_column(
                    ColumnDef::new(Users::PhoneNumber)
                        .string()
                        .null()
                )
                .to_owned(),
        )
        .await
}

async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
        .alter_table(
            Table::alter()
                .table(Users::Table)
                .drop_column(Users::PhoneNumber)
                .to_owned(),
        )
        .await
}
```

## Modifying columns

```rust
async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
        .alter_table(
            Table::alter()
                .table(Users::Table)
                .modify_column(
                    ColumnDef::new(Users::Name)
                        .string_len(500)  // Change VARCHAR(255) to VARCHAR(500)
                        .not_null()
                )
                .to_owned(),
        )
        .await
}
```

## Renaming columns

```rust
async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
        .alter_table(
            Table::alter()
                .table(Users::Table)
                .rename_column(Users::Name, Users::FullName)
                .to_owned(),
        )
        .await
}
```

## Indexes

### Creating indexes

```rust
async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
        .create_index(
            Index::create()
                .name("idx_users_email")
                .table(Users::Table)
                .col(Users::Email)
                .unique()  // Optional: make it unique
                .to_owned(),
        )
        .await
}
```

### Composite indexes

```rust
manager
    .create_index(
        Index::create()
            .name("idx_posts_user_created")
            .table(Posts::Table)
            .col(Posts::UserId)
            .col(Posts::CreatedAt)
            .to_owned(),
    )
    .await
```

### Dropping indexes

```rust
async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
        .drop_index(Index::drop().name("idx_users_email").to_owned())
        .await
}
```

## Foreign keys

### Adding foreign keys

```rust
async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
        .create_table(
            Table::create()
                .table(Posts::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(Posts::Id)
                        .integer()
                        .not_null()
                        .auto_increment()
                        .primary_key(),
                )
                .col(ColumnDef::new(Posts::UserId).integer().not_null())
                .col(ColumnDef::new(Posts::Title).string().not_null())
                .col(ColumnDef::new(Posts::Content).text().not_null())
                .foreign_key(
                    ForeignKey::create()
                        .name("fk_posts_user")
                        .from(Posts::Table, Posts::UserId)
                        .to(Users::Table, Users::Id)
                        .on_delete(ForeignKeyAction::Cascade)
                        .on_update(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await
}
```

### Foreign key actions

| Action | Description |
|--------|-------------|
| `Cascade` | Delete/update child rows automatically |
| `SetNull` | Set foreign key to NULL |
| `SetDefault` | Set foreign key to default value |
| `Restrict` | Prevent delete/update if referenced |
| `NoAction` | Similar to Restrict |

## Migration workflow

A typical change goes through four steps:

```bash
# 1. Generate the file (creates src/migrations/m{ts}_create_posts_table.rs
#    and updates src/migrations/mod.rs).
suprnova make:migration create_posts_table

# 2. Edit src/migrations/m{ts}_create_posts_table.rs to define your schema.

# 3. Apply the migration.
suprnova migrate

# 4. Regenerate SeaORM entity files from the live schema so the models
#    compile against the new shape. `db:sync` also runs any pending
#    migrations first (use --skip-migrations to skip that step).
suprnova db:sync
```

`db:sync` writes auto-generated entity glue to `src/models/entities/<table>.rs`
and a user-editable stub to `src/models/<table>.rs`. Re-running it updates the
entity files; your user stubs are left alone unless you pass
`--regenerate-models` (which overwrites them - keep custom methods elsewhere
or version-control before you run it).

### Auto-migrate on serve

Your application's `serve` and `web:run` subcommands apply any pending
migrations before opening the HTTP socket. The default policy is **fail-closed**: if `up()`
errors, the process aborts non-zero before bind, so a broken migration can
never reach traffic.

Two escape hatches:

| Flag / env | Effect |
|---|---|
| `--no-migrate` (on `serve` / `web:run`) | Skip the auto-migrate step entirely. Useful when migrations run from a separate deploy step. |
| `SUPRNOVA_AUTO_MIGRATE_BEST_EFFORT=true` | Opt back into the legacy log-and-continue behaviour. The process keeps booting on a migration error. Not recommended in production. |

`suprnova serve`, the development command, runs the pending migrations once
when it starts, then starts the watched backend as `serve --no-migrate`, so a
save of a source file does not run them again. Pass `--migrate always` to
`suprnova serve` to migrate on every restart of the backend. See
[suprnova serve](cli-serve.md#migrations).

Background workers (`queue:work`, `workflow:work`, `schedule:run`) do *not*
auto-migrate - they assume schema is already in place when they boot, since
running migrations from N workers concurrently would race.

### Running migrations in tests

`TestDatabase::fresh::<Migrator>()` spins up an isolated in-memory SQLite
database, runs every migration, and binds the connection into the test
container so `DB::connection()` and `#[inject]` resolve to it:

```rust
use suprnova::testing::TestDatabase;
use crate::migrations::Migrator;

#[tokio::test]
async fn users_table_is_created() {
    let db = TestDatabase::fresh::<Migrator>().await.unwrap();
    // `db` is dropped at the end of the test, clearing the container.
}
```

See [Database Tests](database-testing.md) for the full pattern (factories,
parallel safety, picking a real driver instead of in-memory SQLite).

## Best practices

### Always write down migrations

Always implement `down()` to allow rollbacks:

```rust
// Good: Reversible migration
async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager.create_table(/* ... */).await
}

async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager.drop_table(/* ... */).await
}
```

### Use descriptive names

```bash
# Good: Describes the change
suprnova make:migration add_email_verified_to_users
suprnova make:migration create_order_items_table
suprnova make:migration add_index_to_posts_slug

# Bad: Vague names
suprnova make:migration update_users
suprnova make:migration change_table
```

### One change per migration

Keep migrations focused on a single change:

```bash
# Good: Separate migrations
suprnova make:migration create_categories_table
suprnova make:migration add_category_id_to_posts

# Avoid: Multiple unrelated changes in one migration
```

### Test migrations both ways

Before committing, verify both directions work:

```bash
suprnova migrate           # Apply
suprnova migrate:rollback  # Rollback
suprnova migrate           # Apply again
```

## Squashing migrations

A project with years of migrations can replace them with one snapshot.
`suprnova schema:dump` writes the database's schema and its migration
ledger to `database/schema/<engine>-schema.sql`, where the engine is
`sqlite`, `postgres`, `mysql`, or `mariadb`:

```bash
suprnova schema:dump            # writes database/schema/postgres-schema.sql
suprnova schema:dump --prune    # and prunes the migrations the dump covers
```

Commit the dump. When `migrate`, `migrate:fresh`, or `serve` meets a
database that has run no migration, it loads the dump first and then runs
only the migrations newer than it. A database that has run a migration is
never loaded. Neither is a database that already holds tables while its
migration ledger is missing or empty, such as one ported from Laravel:
that is an error, and its tables stay as they were. To load another file,
pass `--schema-path <file>` to `migrate` or `migrate:fresh`; a path that
names no file is an error. `migrate:fresh` drops the views, routines,
sequences, and types of the current schema as well as its tables before
it loads a dump; objects in other Postgres schemas are yours to drop.

`TestDatabase::fresh` loads `database/schema/sqlite-schema.sql` the same
way when every migration the dump records is in the test's `Migrator`, so
tests on SQLite need a SQLite dump. A test with a `Migrator` of its own
runs that `Migrator` from scratch.

Postgres is dumped with `pg_dump` and loaded with `psql`, MySQL with
`mysqldump` and `mysql`, and MariaDB with `mariadb-dump` and `mariadb`.
SQLite needs no tool. These client tools must be on `PATH` wherever a
dump is written or loaded, and `pg_dump` must be at least the server's
major version. They reach the database the way your app does: the URL's
TLS settings (`sslmode`, `sslrootcert`, `sslcert`, and `sslkey` for
Postgres; `ssl-mode`, `ssl-ca`, `ssl-cert`, and `ssl-key` for MySQL and
MariaDB) and its socket go with them. The password reaches them through
`PGPASSWORD` or a private option file, never through their arguments,
and a password in `~/.my.cnf` does not replace it. A dump that fails
leaves the earlier file as it was, and a load that fails runs no
migration, even with `SUPRNOVA_AUTO_MIGRATE_BEST_EFFORT` set.

The image `suprnova docker:init` writes carries `database/schema` and
the Postgres client, so a new environment can start from the dump. For
MySQL or MariaDB, build it with `--build-arg DB_CLIENT=mariadb-client`;
for SQLite, with `--build-arg DB_CLIENT=`.

### Pruning

`--prune` deletes each migration the dump's ledger records and keeps its
name in your `Migrator`'s list:

```rust
vec![
    Box::new(suprnova::PrunedMigration::new("m20250101_000001_create_users")),
    Box::new(m20260301_120000_create_invoices::Migration),
]
```

The name stays because a database that ran the migration records it, and
SeaORM refuses a recorded migration it can't find. A database that would
need the pruned migration's schema, such as an empty one with no dump to
load, fails with an error that names the migration instead of skipping
it. A migration the dump does not record stays as it is. A migration
whose `name()` differs from its file name can't be matched to its file,
so `--prune` stops and changes nothing; rename the file or prune it by
hand. Rebuild the app after pruning.

### Why Suprnova diverges

- **Pruning keeps what the dump does not cover.** Laravel's `--prune`
  deletes every migration file, including any the dumped database never
  ran. Here only the migrations the dump records go, and each keeps its
  name.
- **A dump never loads over tables.** Laravel loads a dump into any
  database whose migrations table is empty, and a MySQL dump drops each
  table it creates. Here the dump has no `DROP TABLE`, and a database that
  already holds tables is an error.
- **No schema events.** Laravel fires `SchemaDumped`, `SchemaLoaded`, and
  `MigrationsPruned`. Suprnova's migration commands run before the
  bootstrap registers listeners, so no listener would hear them.
- **The file is named after the engine.** Laravel names it after the
  connection; Suprnova migrates one connection, `DATABASE_URL`.

## CLI commands at a glance

| Command | Description |
|---------|-------------|
| `suprnova make:migration <name>` | Create a new migration |
| `suprnova migrate` | Run all pending migrations |
| `suprnova migrate:status` | Show migration status |
| `suprnova migrate:rollback` | Rollback the last migration |
| `suprnova migrate:rollback --step 3` | Rollback the last 3 migrations |
| `suprnova migrate:fresh` | Drop all tables and re-run every migration |
| `suprnova schema:dump` | Write the schema and migration ledger to `database/schema/` |
| `suprnova schema:dump --prune` | Also replace the dumped migrations with their names |
| `suprnova db:sync` | Run migrations and regenerate entity files |
| `suprnova db:sync --skip-migrations` | Regenerate entity files without applying migrations |
| `suprnova db:sync --regenerate-models` | Also overwrite user-editable model stubs |

See [CLI Migrations Reference](cli-migrations.md) for the full per-command
reference (flags, output samples, exit codes).

## Next

- [CLI Migrations Reference](cli-migrations.md) - flag-by-flag reference for `migrate*` and `db:sync`
- [Database](database.md) - connection configuration, transactions, read/write split
- [Eloquent](eloquent.md) - the model layer your migrations feed
- [Seeding](seeding.md) - populating tables once their schema exists
- [Database Tests](database-testing.md) - `TestDatabase::fresh::<Migrator>()` and parallel-safe patterns
