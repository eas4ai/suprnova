# Query Builder

When you want to query a table without modelling it as a typed
`#[suprnova::model]` struct, reach for `DB::table(name)`. It returns a
chainable builder shaped like the typed Eloquent `Builder<M>`, but
materialises rows as `DynamicRow` - a `serde_json::Map` newtype with
typed accessors. This is the chapter for audit logs, ad-hoc reports,
dashboard aggregates, and any table you haven't bothered to model. For
the typed equivalent, see [Eloquent](eloquent.md). For raw `DB::select`
inside transactions or with `DB::listen` observation, see
[Database](database.md).

```rust
use suprnova::DB;

let rows = DB::table("audit_log")
    .select(["id", "event", "actor_id"])
    .filter("actor_id", 42i64)
    .filter_op("created_at", ">=", "2026-01-01")
    .order_by_desc("id")
    .limit(50)
    .get()
    .await?;

for row in rows.iter() {
    let id: i64 = row.get_int("id")?;
    let event: String = row.get_string("event")?;
    println!("{id}: {event}");
}
```

## When to use which surface

Three query surfaces overlap; pick the right one for the table.

| Table is… | Use | Returns |
|---|---|---|
| Modeled with `#[suprnova::model]` | `Model::query()` → `Builder<M>` | typed `M` values |
| Unmodeled but you want a chainable WHERE/ORDER/LIMIT shape | `DB::table(name)` → `DbTableBuilder` | `DynamicRow` |
| Anything the builders can't express - CTEs, window functions, backend DDL | `DB::select` / `DB::statement` / `DB::affecting_statement` | `DynamicRow` / `bool` / `u64` |

`DbTableBuilder` exists for the middle case. You get the WHERE / ORDER /
LIMIT chain without committing to a `#[suprnova::model]` struct and
without dropping all the way to raw SQL strings.

## The chainable surface

`DB::table(name)` returns a `DbTableBuilder`. Build it up, then call a
terminal method to execute.

### Filtering

```rust
// Equality.
DB::table("users").filter("email", "alice@example.com").get().await?;

// Arbitrary operator. Allowlist: =, <>, <, <=, >, >=, LIKE, NOT LIKE,
// ILIKE, NOT ILIKE, IS, IS NOT.
DB::table("orders").filter_op("total", ">=", 100i64).get().await?;
DB::table("posts").filter_op("title", "LIKE", "%rust%").get().await?;

// Multiple filters AND together.
DB::table("audit_log")
    .filter("actor_id", 42i64)
    .filter_op("event", "<>", "noop")
    .get()
    .await?;
```

`filter` and `filter_op` both accept any `Into<SeaValue>` for the
right-hand side, which covers `i64`, `String`, `&str`, `bool`, `f64`,
`Option<T>`, `chrono::*`, `uuid::Uuid`, and `serde_json::Value` - every
column type the backend understands.

A `u64` compares as an integer. Postgres and SQLite have no unsigned
integers, so no integer column there holds a `u64` above `i64::MAX`, and a
comparison with one gets its answer without sending the value: `=`, `>`,
`>=` and `where_in` match no row, and `<>`, `<`, `<=` and `where_not_in`
match every row whose column is not NULL. That is the answer MySQL gives
for rows that all hold smaller values. To compare a decimal or
floating-point column with a large number, pass the value as that type.

The rest of the `WHERE` vocabulary uses the Laravel names:

```rust
// Lists, and NULL checks.
DB::table("users").where_in("role", ["admin", "editor"]).get().await?;
DB::table("users").where_not_in("id", [1i64, 2]).get().await?;
DB::table("users").where_null("deleted_at").get().await?;
DB::table("users").where_not_null("verified_at").get().await?;

// Two columns compared, with no value.
DB::table("orders").where_column("shipped_at", "paid_at").get().await?;

// A raw fragment. Write each value as `?` and pass it in the bindings;
// Postgres gets `$N` markers numbered for their place in the statement.
DB::table("orders")
    .where_raw("total * ? > budget", vec![1.2.into()])
    .get()
    .await?;
```

An empty `where_in` list matches no row, and an empty `where_not_in`
list excludes none.

#### OR conditions

Each `or_*` method folds its condition into the one before it, so an
`OR` widens that one condition and never the whole `WHERE` clause:

```rust
// WHERE active = ? AND (role IN (?, ?) OR invited_by IS NOT NULL)
DB::table("users")
    .filter("active", true)
    .where_in("role", ["admin", "editor"])
    .or_where_not_null("invited_by")
    .get()
    .await?;
```

The `or_` family is `or_where_in`, `or_where_not_in`, `or_where_null`,
`or_where_not_null`, `or_where_raw`, and the three grouped helpers below.

#### One comparison across several columns

`where_any` compares several columns with one operator and value, and
matches when any of them does. `where_all` matches when every column
does, and `where_none` when none does:

```rust
// WHERE (code LIKE ? OR description LIKE ?)
let found = DB::table("products")
    .where_any(["code", "description"], "like", format!("%{search}%"))
    .get()
    .await?;

// WHERE is_admin = ? AND NOT (banned = ? OR suspended = ?)
let staff = DB::table("users")
    .filter("is_admin", true)
    .where_none(["banned", "suspended"], "=", true)
    .get()
    .await?;
```

The comparisons sit in parentheses, so an `OR` inside never reaches the
conditions around it: `filter("a", 1).where_any(["b", "c"], "=", 2)`
returns only rows whose `a` is 1. `or_where_any`, `or_where_all`, and
`or_where_none` fold the group into the condition before it. An empty
column list adds no condition.

#### Subqueries

`where_in`, `where_not_in`, and their `or_` forms also take another
`DB::table` builder, which runs as a subquery. `where_exists` and
`where_not_exists` take one too, and the subquery can refer to the outer
table through `where_column`:

```rust
// Rooms with a slot on Monday.
let booked = DB::table("rooms")
    .where_in("id", DB::table("slots").select(["room_id"]).filter("day", "mon"))
    .get()
    .await?;

// Users who have written a post.
let authors = DB::table("users")
    .where_exists(
        DB::table("posts")
            .select_raw("1")
            .where_column("posts.author_id", "users.id"),
    )
    .get()
    .await?;
```

A subquery's values stay bound parameters, in the position they take in
the statement. On Postgres the `$N` numbers continue through the
subquery.

#### Byte-exact comparison

`where_binary` compares the raw bytes of a column instead of matching
under its collation, so `"Alice"` does not match `"alice"` or `"ALICE"`:

```rust
DB::table("users").where_binary("email", submitted).get().await?;
DB::table("users").where_not_binary("email", submitted).get().await?;
```

This is a MySQL and MariaDB feature - it emits their `binary` operator
modifier, `email = binary ?`. Postgres and SQLite have no equivalent, so
on those backends every terminal returns an error when the statement
renders, before any query runs. Suprnova refuses rather than falling back
to a plain `=`, because a fallback would compare under the column's
collation and return rows you asked to exclude.

If you need case-sensitive matching on Postgres or SQLite, set a
case-sensitive collation on the column, or use `DB::select` with a
backend-specific expression.

### Selecting columns

```rust
// Default is SELECT *.
DB::table("users").get().await?;

// Restrict columns when you only need some.
DB::table("users").select(["id", "email"]).get().await?;

// An alias renames a column in the result; `table.*` selects one
// table's columns, which matters once a query joins another table.
DB::table("users").select(["users.*", "email as login"]).get().await?;

// Add an expression with select_raw, and group the rows.
DB::table("orders")
    .select(["status_id"])
    .select_raw("COUNT(*) AS total")
    .group_by("status_id")
    .get()
    .await?;
```

`select` replaces the list each time you call it. `select_raw` adds an
expression to the end of the list, so it combines with `select`. The
expression is written into the query as given, so never build it from
request data.

### Joins

`join`, `left_join`, and `right_join` take the table and one `ON`
condition between two columns. To join a table under an alias, write
`"table as alias"`. `cross_join` takes only the table. This query lists
every post with its category and author:

```rust
let rows = DB::table("posts")
    .left_join("categories", "categories.id", "=", "posts.category_id")
    .left_join("users as authors", "authors.id", "=", "posts.author_id")
    .select([
        "posts.title",
        "categories.name as category_name",
        "authors.name as author_name",
    ])
    .order_by_asc("posts.title")
    .get()
    .await?;

for row in rows.iter() {
    let author: Option<String> = row.get_optional_string("author_name")?;
}
```

The rows come back as `DynamicRow`. Columns that the joined tables share,
such as `id` or `name`, overwrite one another in the row, so alias them
in `select`.

For more than one condition, `join_with`, `left_join_with`, and
`right_join_with` pass a `JoinClause` to a closure. `on` and `or_on`
compare two columns. `filter`, `filter_op`, `or_filter`, and
`or_filter_op` compare a column with a value, and `db_where`,
`db_where_op`, `or_where`, and `or_where_op` are their Laravel names:

```rust
// INNER JOIN users ON users.id = posts.author_id
//   AND (posts.views > ? OR users.role = ?)
let rows = DB::table("posts")
    .join_with("users", |join| {
        join.on("users.id", "=", "posts.author_id")
            .db_where_op("posts.views", ">", 100)
            .or_where("users.role", "editor")
    })
    .get()
    .await?;
```

A join condition's value is a bound parameter, like every other value.
An `or_*` condition folds into the condition before it, as in the
`WHERE` clause. A join other than `cross_join` needs at least one
condition: the query fails with an error before it runs otherwise.

To join a subquery, pass another `DB::table` builder and an alias to
`join_sub` or `left_join_sub`. `join_sub_with` and `left_join_sub_with`
take a closure instead of one condition. This query counts each status's
orders, with zero for a status that has none:

```rust
let totals = DB::table("orders")
    .select(["status_id"])
    .select_raw("COUNT(*) AS total")
    .group_by("status_id");

let rows = DB::table("statuses")
    .left_join_sub(totals, "order_totals", "order_totals.status_id", "=", "statuses.id")
    .select(["statuses.name"])
    .select_raw("COALESCE(order_totals.total, 0) AS total")
    .get()
    .await?;
```

The subquery's values bind ahead of the values of the conditions and
`WHERE` clauses after it, so the order you call the methods in doesn't
matter.

`get` and `count` record a read of every table the query names,
including joined tables and the tables a subquery names, so a cached
page that ran the query is invalidated when any of them changes. A
`select_raw`, `where_raw`, or `or_where_raw` fragment can read a table
the builder doesn't name, so a query that carries one anywhere, subqueries
included, keeps the page out of the cache: the response is still served,
but it is never stored. A `select_raw` that is a bare number, such as the
`select_raw("1")` in an `EXISTS` subquery, reads nothing and doesn't
count. See [Render cache](render-cache.md).

### Ordering and windowing

```rust
DB::table("posts")
    .order_by_desc("created_at")
    .order_by_asc("title")
    .limit(20)
    .offset(40)
    .get()
    .await?;
```

`order_by_desc` and `order_by_asc` chain in insertion order; the
generated SQL preserves it.

`reorder()` drops every ordering set so far, and `reorder_by(col,
Direction::Asc)` drops them and orders by `col` instead. Use it on a base
query before you reuse it as a subquery:

```rust
use suprnova::Direction;

let recent = DB::table("slots").select(["room_id"]).order_by_desc("starts_at");
let rooms = DB::table("rooms")
    .where_in("id", recent.reorder())
    .reorder_by("name", Direction::Asc)
    .get()
    .await?;
```

### Terminals

```rust
// All matching rows.
let rows: Collection<DynamicRow> = DB::table("audit_log")
    .filter("actor_id", 42i64)
    .get()
    .await?;

// First row or None.
let first: Option<DynamicRow> = DB::table("audit_log")
    .filter("event", "user.deleted")
    .first()
    .await?;

// Just the count (clears any select/order/limit/offset before
// rendering - count semantics don't care about those).
let n: u64 = DB::table("audit_log")
    .filter("actor_id", 42i64)
    .count()
    .await?;
```

On a query with `group_by`, `count()` returns the number of groups.

`get()` returns `Collection<DynamicRow>` - the same collection wrapper
typed models use, with the same `.iter()`, `.len()`, `.into_vec()`
surface. See [Eloquent Collections](eloquent-collections.md).

### Inserts, updates, deletes

```rust
use suprnova::attrs;

// INSERT, returns the new row's auto-increment id.
let id: i64 = DB::table("audit_log")
    .insert(attrs! { event: "user.created", actor_id: 42 })
    .await?;

// UPDATE, returns rows affected.
let updated: u64 = DB::table("audit_log")
    .filter("id", id)
    .update(attrs! { event: "user.created.v2" })
    .await?;

// DELETE, returns rows affected.
let deleted: u64 = DB::table("audit_log")
    .filter("actor_id", 42i64)
    .delete()
    .await?;
```

The `attrs!` macro builds the column-to-value map at the call site.
Keys are SQL identifiers (validated) and values are bound as
parameters. An explicit null is emitted as SQL `NULL` because the JSON
attribute map no longer carries its original Rust type; all non-null values
remain parameter-bound. The same rule applies to typed Eloquent mass writes
and many-to-many pivot extras.

A `u64` above `i64::MAX` binds as an unsigned integer, which a MySQL
unsigned column stores exactly. Postgres and SQLite have no integer column
that holds it, so there `insert` and `update` refuse it before anything
is sent. The refusal is a database error that names the column: a client
gets the generic 500 response and the log gets the detail.

#### `update_all` and `delete_all` aliases

`update` and `delete` are the Laravel-faithful names. The
`Builder<M>`-style aliases - `update_all` and `delete_all` - call the
same implementation. Prefer the `_all` form when the table-wide intent
is the point of the call site; it makes a missing `filter` visible to
reviewers:

```rust
// Same behaviour as DB::table("rate_limits").delete().await? but the
// _all suffix tells reviewers "yes, I meant to truncate the table".
DB::table("rate_limits").delete_all().await?;

// Mass update with a WHERE - the _all suffix here matches the typed
// Builder<M> convention for the same operation.
DB::table("sessions")
    .filter_op("expires_at", "<", chrono::Utc::now())
    .update_all(attrs! { status: "expired" })
    .await?;
```

#### Empty WHERE on update or delete operates on every row

`DB::table("x").delete().await?` removes every row in the table. That
is supported by design - sometimes you really do want to truncate -
but it's rarely correct. Always look at a `delete()` / `delete_all()`
call and check whether there's a `filter` in front of it. The same is
true of `update` / `update_all`.

`update` and `delete` return an error on a builder with a join. The
statement they render names one table, so it would ignore the join and
change rows the join was there to exclude. To narrow the rows by another
table, use `where_in` or `where_exists` with a subquery.

#### Insert backend split

`RETURNING id` is used on Postgres and SQLite. MySQL doesn't support
`RETURNING`, so the builder runs the INSERT and reads the driver's
per-connection `last_insert_id()` from the result. The model-less
builder assumes a standard `id` auto-increment primary key. UUID,
composite, renamed, or non-integer primary keys aren't supported on
this surface - use the typed [Eloquent](eloquent.md) `Model` interface
instead, which consults the model definition for primary-key shape.

## `DynamicRow` - typed accessors over a JSON map

Every row returned by `DB::table` or `DB::select` materialises as
`DynamicRow`, a `serde_json::Map<String, Value>` newtype with typed
accessors. Each getter returns `Result<T, FrameworkError>` with a
clear error message on missing key or type mismatch.

```rust
for row in rows.iter() {
    let id: i64                 = row.get_int("id")?;
    let event: String           = row.get_string("event")?;
    let active: bool            = row.get_bool("active")?;
    let weight: f64             = row.get_float("weight")?;
    let payload: serde_json::Value = row.get_value("payload")?;
}
```

For nullable columns, use `get_optional_*`. These distinguish "column
missing" (error - schema mismatch) from "column present, value SQL
NULL" (`Ok(None)`):

```rust
let title: Option<String> = row.get_optional_string("title")?;
let score: Option<i64>    = row.get_optional_int("score")?;
```

The optional family covers `String` and `i64`. For other
nullable types, use `get_value` and match on `serde_json::Value::Null`
yourself, or read the column through `get_as::<Option<T>>` (any
`T: DeserializeOwned`).

To deserialise a column into any struct or container type, use
`get_as`. The full `serde_json` deserialisation surface is available:

```rust
#[derive(serde::Deserialize)]
struct UserPrefs {
    theme: String,
    notifications: bool,
}

let prefs: UserPrefs    = row.get_as("prefs")?;
let tags: Vec<String>   = row.get_as("tags")?;
let when: chrono::DateTime<chrono::Utc> = row.get_as("created_at")?;
```

`DynamicRow` derefs to `Map<String, Value>`, so iteration and
key-existence checks work directly:

```rust
for (key, value) in row.iter() {
    println!("{key} = {value}");
}

if row.contains_key("deleted_at") { /* … */ }
```

## Identifier trust boundary

Table names, column names, aliases, ORDER BY directions, and SQL
operators are written into the SQL string - they are NOT bound as
parameters (SQL doesn't allow placeholder-bound identifiers). Treat
every `impl Into<String>` argument as a trusted, compile-time literal.

```rust
// Safe - the column name is a constant; the value is bound.
DB::table("users").filter("email", request.email()).get().await?;

// UNSAFE - never splice user input into a column name.
DB::table("users")
    .filter(request.user_supplied_column(), value)
    .get()
    .await?;
```

The framework enforces a strict allowlist at the I/O boundary -
identifiers must match `[A-Za-z_][A-Za-z0-9_]*` with one optional
`schema.` prefix, and operators must come from a fixed list. Violations
fail closed with a `FrameworkError::Database` before any SQL is
rendered. The builder then quotes every identifier for the backend:
backticks on MySQL and MariaDB, double quotes on Postgres and SQLite.
That's a safety net, not a license: keep identifiers literal in your
code. `select_raw` and `where_raw` fragments are written as given and
never checked.

Values on the right-hand side of `filter` / `filter_op`, in `where_in`
lists, in join conditions, and in raw-fragment bindings are always bound
as parameters and safe to splice through from request data.

## Raw queries

When the builder can't express what you need - recursive CTEs, window
functions, backend-specific DDL, `INSERT … ON CONFLICT DO UPDATE` -
drop to a raw string. Placeholders match the active backend (`$1, $2,
…` for Postgres, `?` for MySQL and SQLite); the framework auto-detects
from `DatabaseConfig::url`.

```rust
use suprnova::DB;
use sea_orm::Value;

// SELECT - every row as DynamicRow.
let rows = DB::select(
    "SELECT u.name, COUNT(p.id) AS post_count
     FROM users u LEFT JOIN posts p ON p.user_id = u.id
     GROUP BY u.id
     HAVING COUNT(p.id) > ?",
    vec![Value::from(5i64)],
).await?;

// SELECT - first row only, mirrors Laravel's DB::selectOne.
let alice = DB::select_one(
    "SELECT * FROM users WHERE email = ?",
    vec![Value::from("alice@example.com")],
).await?;

// SELECT - first column of first row as a typed scalar.
let total: i64 = DB::scalar(
    "SELECT COUNT(*) FROM users WHERE active = ?",
    vec![Value::from(true)],
).await?;

// INSERT - true when at least one row was affected.
DB::insert(
    "INSERT INTO users (name, active) VALUES (?, ?)",
    vec![Value::from("bob"), Value::from(true)],
).await?;

// UPDATE / DELETE - return the rows-affected count.
let updated: u64 = DB::update(
    "UPDATE users SET active = ? WHERE id = ?",
    vec![Value::from(false), Value::from(1i64)],
).await?;

let deleted: u64 = DB::delete(
    "DELETE FROM users WHERE active = ?",
    vec![Value::from(false)],
).await?;

// Any prepared statement with bindings.
DB::statement(
    "UPDATE users SET votes = votes + ? WHERE id = ?",
    vec![Value::from(1i64), Value::from(42i64)],
).await?;

// DDL or other no-binding statements that reject placeholder binding.
DB::unprepared("CREATE INDEX idx_users_name ON users(name)").await?;

// Generic "rows affected" path - for upserts and operations that
// don't fit the named helpers.
let n: u64 = DB::affecting_statement(
    "INSERT INTO counters (k, n) VALUES ($1, 1)
     ON CONFLICT (k) DO UPDATE SET n = counters.n + 1",
    vec![Value::from("page_views")],
).await?;
```

### Computed columns on SQLite

SQLite declares no type for a computed column: `COUNT(*) AS n`,
`COALESCE(t.total, 0) AS total`, or any other `select_raw` expression.
Rows from `DB::table` and `DB::select` still carry the column. Suprnova
reads each such value by its runtime type, so an integer comes back as a
JSON integer, a real as a JSON number, text as a string, and `NULL` as
`null`:

```rust
let rows = DB::select(
    "SELECT actor_id, COUNT(*) AS n FROM audit_log GROUP BY actor_id",
    vec![],
).await?;
let n: i64 = rows[0].get_int("n")?;
```

## Bridge to typed Eloquent

When the table is worth a `#[suprnova::model]` struct, the chainable
shape carries over. `Model::query()` returns `Builder<M>`, which
ships the same `filter` / `filter_op` / `order_by_*` / `limit` /
`offset` / `get` / `first` / `count` surface - plus a much wider WHERE
vocabulary (`filter_in`, `filter_between`, `filter_null`, `filter_has`,
`filter_raw`, …) and Laravel-shape aliases (`db_where`, `where_in`,
`where_between`, `where_null`, `where_has`, `where_raw`, …).

```rust
use suprnova::Model;

let admins = User::query()
    .filter("role", "admin")
    .filter_op("created_at", ">=", since)
    .order_by_desc("created_at")
    .limit(20)
    .get()
    .await?;     // Collection<User> - typed, not DynamicRow

let alice = User::query().filter("email", &email).first().await?;
let total = User::query().filter("active", true).count().await?;
// Note: Builder<M>::count returns i64 (matches Laravel's Eloquent),
// whereas DbTableBuilder::count returns u64. Both surfaces give you a
// non-negative SQL COUNT - they only differ in their wire type.
```

The full `Builder<M>` surface - every WHERE shape, aggregates,
relations, eager loading, scopes, paginators, chunk iteration - is in
[Eloquent](eloquent.md). The chainable shape you learned above is the
same shape; the differences are typing and reach. The joins, the
grouped `where_any` family, the `or_` helpers, subqueries, and `reorder`
work the same way on both builders, and a model query takes a
`DB::table` builder as its subquery.

## Routing to a named connection

`DB::table` and the raw helpers default to the primary connection. To
target a read replica, shard, or warehouse pool, pin the call:

```rust
// Builder pinned to a named connection.
let rows = DB::table("audit_log").on("warehouse").get().await?;

// Equivalent shorthand.
let rows = DB::table_on("warehouse", "audit_log").get().await?;

// Raw escapes have _on variants too.
let rows = DB::select_on("warehouse", "SELECT …", vec![]).await?;
let n    = DB::affecting_statement_on(
    "warehouse",
    "UPDATE …",
    vec![],
).await?;
```

When `__read_replica__` is registered, every read-shape terminal
auto-routes through it; writes (`insert` / `update` / `delete` /
`update_all` / `delete_all`) always target the primary. Inside a
`DB::transaction` closure the active transaction's connection wins
absolutely - `on(name)` is silently ignored to preserve atomicity. See
[Database - Named connections](database.md) for the full precedence
chain.

### Why Suprnova diverges

Laravel's `DB::table(...)` is its model-less query builder; under the
hood it returns a `stdClass` per row (a PHP object whose properties
are the columns). Suprnova returns `DynamicRow` instead - a
`serde_json::Map` newtype with typed accessors. The accessor shape
catches missing-column and wrong-type errors at the boundary instead
of panicking deep in user code with a property-access exception.

The dual `update`/`update_all` and `delete`/`delete_all` names exist
because the typed Eloquent `Builder<M>` surface uses the `_all` suffix
to make table-wide intent explicit at the call site. Rather than pick
a side, the model-less builder ships both - `update` and `delete`
match Laravel's `DB::table($t)->update(...)` and `->delete()` letter
for letter; `update_all` and `delete_all` match the convention `M`
users will already have in their muscle memory.

`where_binary` returns an error on Postgres and SQLite where Laravel
throws a `RuntimeException` from the base grammar. The reason is the
same and only the mechanism differs: public-surface code in Suprnova
returns `Result` rather than panicking, so the refusal arrives as an
`Err` from the terminal instead of an exception from the grammar.

An `or_*` call folds into the condition before it. Laravel keeps a flat
list of conditions joined by `and` and `or`, and SQL precedence then
binds each `and` before any `or`. So `where(a)->where(b)->orWhere(c)`
means `(a AND b) OR c` in Laravel and `a AND (b OR c)` here. Suprnova's
rule matches the model builder's, and an `or` never widens past a
condition written before it. When you port a query that mixes the two,
check the SQL it renders; for Laravel's reading, write the condition
with `where_raw`.

Laravel's `join` and `whereExists` also take a closure that builds the
subquery, an Eloquent builder, or a raw expression. Here a subquery is
always a `DB::table` builder, and a join's closure only adds conditions.
The column-list helpers take the operator every time -
`where_any(cols, "=", value)` - because Rust has no optional arguments.
`update` and `delete` refuse a join, where Laravel's MySQL grammar
renders `UPDATE ... JOIN`, because the portable statement would ignore
it.

## Next

- [Database](database.md) - `DB` facade, transactions with savepoints,
  `DB::listen` observability, named connections
- [Eloquent](eloquent.md) - typed `#[suprnova::model]` structs and the
  full `Builder<M>` surface
- [Pagination](pagination.md) - `paginate` / `simple_paginate` /
  `cursor_paginate` on typed builders
- [Eloquent Collections](eloquent-collections.md) - the `Collection<T>`
  returned by `get()` on both surfaces
- [Migrations](migrations.md) - defining the schema the builders query
