# MongoDB

MongoDB stores documents instead of rows: each record is a BSON document
with nested values, arrays, and embedded documents. Suprnova talks to it
through the official `mongodb` driver, behind the `database-mongodb`
feature. The `Mongo` facade answers the connection, its database, and typed
collections, the way Laravel's `mongodb` connection does with the
`laravel-mongodb` package.

```rust
use suprnova::Mongo;
use suprnova::bson::{Document, doc};

Mongo::ping().await?;
let users = Mongo::collection::<Document>("users")?;
let ada = users.find_one(doc! { "name": "Ada" }).await?;
```

## Installation

The MongoDB backend is off by default, because the driver is a large
dependency tree that an SQL-only application never needs. To turn it on,
add the `database-mongodb` feature to the `suprnova` dependency:

```toml
[dependencies]
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v3.2.1", features = ["database-mongodb"] }
```

The feature brings the `mongodb` driver and the `bson` crate. You don't add
either to your own `Cargo.toml`: Suprnova re-exports BSON as
`suprnova::bson` and the driver's types under `suprnova::mongodb`, at the
versions it links. Without the feature, neither crate is in your build.

You need a MongoDB server. For local work, the MongoDB Community Server
runs on Linux, macOS, and Windows, or in Docker:

```bash
docker run -d --name mongodb -p 27017:27017 mongo:8
```

## Configuration

Two variables configure the connection, the same ones Laravel's MongoDB
page uses:

```ini
MONGODB_URI="mongodb://127.0.0.1:27017"
MONGODB_DATABASE="shop"
```

`MONGODB_URI` is the connection string. It starts with `mongodb://`, or with
`mongodb+srv://` for a MongoDB Atlas cluster:

```ini
MONGODB_URI="mongodb+srv://<username>:<password>@<cluster>.mongodb.net/?retryWrites=true&w=majority"
MONGODB_DATABASE="shop"
```

`MONGODB_DATABASE` names the database your application uses. When it's
unset, Suprnova takes the database from the URI's path, so
`mongodb://127.0.0.1:27017/shop` alone also selects `shop`.
`MONGODB_DATABASE` wins when both name one.

Pool and timeout options go in the URI's query string, as the driver reads
them: `maxPoolSize`, `minPoolSize`, `maxIdleTimeMS`, `maxConnecting`,
`connectTimeoutMS`, and `serverSelectionTimeoutMS`, among others. For
example, `mongodb://127.0.0.1:27017/?maxPoolSize=20` caps each server's pool
at 20 connections.

Suprnova checks the configuration before it uses it, without contacting
the server:

- An unset or blank `MONGODB_URI` is an error that names `MONGODB_URI`.
- A value that isn't a MongoDB connection string is an error that names
  `MONGODB_URI` and gives the driver's reason.
- A missing database, or a name MongoDB refuses (empty, longer than 63
  bytes, or holding `/`, `\`, `.`, a space, `"`, or `$`), is an error that
  names `MONGODB_DATABASE`.

None of these is a panic, and no error or `Debug` output shows the URI's
password or query string.

### Configure the connection in code

To configure the connection in code, build a `MongoConfig` and register it
in your configuration. The builder takes the URI, the database, and the pool
options, and it falls back to `MONGODB_URI` and `MONGODB_DATABASE` for what
you don't set:

```rust
use std::time::Duration;
use suprnova::{Config, MongoConfig};

pub fn register() -> Result<(), suprnova::FrameworkError> {
    Config::register(
        MongoConfig::builder()
            .uri("mongodb://127.0.0.1:27017")
            .database("shop")
            .max_pool_size(50)
            .min_pool_size(5)
            .max_idle_time(Duration::from_secs(300))
            .max_connecting(4)
            .connect_timeout(Duration::from_secs(5))
            .server_selection_timeout(Duration::from_secs(5))
            .build()?,
    );
    Ok(())
}
```

A pool option you set in the builder wins over the same option in the URI.
`build` refuses a `max_pool_size` or `max_connecting` of 0 and a
`min_pool_size` above `max_pool_size`, naming the option.

`MongoConfig::from_env()` is the builder with nothing set: the two variables
and nothing else.

### Named connections

The connection that `MONGODB_URI` configures is named `mongodb`, as in
Laravel's `config/database.php`; `DEFAULT_MONGO_CONNECTION` holds the name.
To reach a second server or database, add a named connection to the
builder:

```rust
use suprnova::{Config, MongoConfig, MongoConnectionConfig};

let analytics = MongoConnectionConfig {
    max_pool_size: Some(5),
    ..MongoConnectionConfig::new("mongodb://analytics.internal:27017", "events")
};

Config::register(
    MongoConfig::builder()
        .connection("analytics", analytics)
        .build()?,
);
```

A connection can't be named `mongodb` or have an empty name, and an error
about a named connection names it.

## Connect at boot

The server registers the connections in the container when it boots, if
`MONGODB_URI` is set or a `MongoConfig` is registered. The queue, schedule,
and workflow workers and the console register them the same way. It happens
before the cache and the queue boot, so the drivers that run on MongoDB find
the connection. With neither set, nothing is registered and the boot goes
on.

The driver connects lazily. Registering a connection opens no socket, so a
MongoDB server that is down doesn't stop your application from booting.
Instead, the first call that needs the server fails: `Mongo::ping()`
returns a `FrameworkError`, and so does a driver operation such as
`find_one` once `?` converts its error. The one exception is a
`mongodb+srv://` URI: the driver looks up its DNS records when the
connection is registered, because it needs them to know the hosts.

A `MONGODB_URI` that is set but invalid stops the boot of the server and
the workers with the error that names it. The console reports it and goes
on, so a command that doesn't use MongoDB still runs.

Your application's `bootstrap` hook runs before the boot registers the
connections. To use MongoDB there, for example to create indexes, register
the connections yourself first:

```rust
use suprnova::Mongo;

pub async fn register() -> Result<(), suprnova::FrameworkError> {
    // Reads the registered `MongoConfig`, or `MONGODB_URI` and
    // `MONGODB_DATABASE` when none is registered.
    Mongo::init().await?;
    Ok(())
}
```

`Mongo::init_with(config)` registers the connections of a `MongoConfig` you
pass. Both replace connections registered before. The boot keeps
connections that your application registered first.

The driver runs its pool and server monitors as Tokio tasks, so a connection
is opened from inside your application's runtime. Opening one anywhere else
is an error, not a panic.

## The Mongo facade

The facade answers the connections the boot registered:

| Call | Returns |
|---|---|
| `Mongo::connection()` | The `mongodb` connection, a `MongoConnection`. |
| `Mongo::connection_named(name)` | The connection `name`. `"mongodb"` is the default one. |
| `Mongo::database()` | The default connection's database, a `suprnova::mongodb::Database`. |
| `Mongo::collection::<T>(name)` | A typed `suprnova::mongodb::Collection<T>` in that database. |
| `Mongo::ping()` | `Ok(())` when the server answers `ping`. |

Every call returns a `Result`. With no connection registered, the error says
why: `MONGODB_URI` is unset, or invalid, or set while the boot hasn't run
yet in this process. An unknown name in `connection_named` is an error that
names it.

A `MongoConnection` has the same methods for one connection, plus `name()`
and `client()`, the driver's client for other databases, sessions, and
transactions:

```rust
use suprnova::Mongo;
use suprnova::bson::Document;

let analytics = Mongo::connection_named("analytics")?;
analytics.ping().await?;
let events = analytics.collection::<Document>("page_views");
let other = analytics.client().database("archive");
```

### Typed collections

A collection reads and writes your own types through serde. Nothing is sent
to the server until an operation runs:

```rust
use serde::{Deserialize, Serialize};
use suprnova::Mongo;
use suprnova::bson::doc;

#[derive(Debug, Serialize, Deserialize)]
struct Note {
    title: String,
    pages: i32,
}

let notes = Mongo::collection::<Note>("notes")?;
notes.insert_one(Note { title: "Draft".into(), pages: 3 }).await?;
let draft = notes.find_one(doc! { "title": "Draft" }).await?;
```

The collection is the driver's own type, so every driver operation works on
it: `find`, `aggregate`, `update_many`, `create_index`, and the rest. For the
driver's options, cursors, and other types, use `suprnova::mongodb::driver`,
which is the whole `mongodb` crate.

A driver operation returns the driver's error. In code that returns
`Result<_, FrameworkError>`, `?` converts it, and the driver's error stays
the source:

```rust
use suprnova::bson::{Document, doc};
use suprnova::{FrameworkError, Mongo};

async fn count_users() -> Result<u64, FrameworkError> {
    let users = Mongo::collection::<Document>("users")?;
    Ok(users.count_documents(doc! {}).await?)
}
```

### Health checks

`Mongo::ping()` sends `ping` to the server. A server that can't be reached
fails after the server selection timeout, 30 seconds unless the URI or the
builder sets another. The error names the connection and keeps the driver's
error as its source, which `FrameworkError::external_source` returns.

## Models

A document model is a struct that `#[suprnova::document]` stores in a
collection. It has the Eloquent calls of an SQL model: `create`, `find`,
`find_or_fail`, `all`, `query`, `save`, `update`, `delete`, `fresh`, and
`refresh`. The calls come from the `DocumentModel` trait, so import it where
you call them:

```rust
use suprnova::bson::doc;
use suprnova::{DocumentModel, FrameworkError};

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Address {
    pub street: String,
    pub city: String,
}

#[suprnova::document(
    collection = "users",
    fillable = ["name", "email", "password", "tags", "addresses"],
    hidden = ["password"],
    soft_deletes
)]
pub struct User {
    pub name: String,
    pub email: String,
    pub password: Option<String>,
    pub tags: Vec<String>,
    #[embeds_many]
    pub addresses: Vec<Address>,
    pub created_at: Option<suprnova::bson::DateTime>,
    pub updated_at: Option<suprnova::bson::DateTime>,
    pub deleted_at: Option<suprnova::bson::DateTime>,
}

async fn example() -> Result<(), FrameworkError> {
    let user = User::create(doc! {
        "name": "Ada",
        "email": "ada@example.com",
        "tags": ["math"],
    })
    .await?;

    let found = User::find(user.id).await?;
    let user = user.update(doc! { "name": "Ada Lovelace" }).await?;
    user.delete().await?;
    Ok(())
}
```

The macro derives `Clone` and `Debug`, and it implements serde's `Serialize`
for the struct, so don't derive those three yourself. The model's events
need the first two, and the third honours `hidden` and `visible`.

### Collection and connection

`collection = "..."` names the collection. Without it, the collection is the
plural snake-case name of the struct, so `BlogPost` is stored in
`blog_posts`. `connection = "..."` stores the model on a named connection
instead of the default `mongodb` one.

### Keys

Every document has a key, which MongoDB stores as `_id`. Without a
`primary_key`, the key is the field `id`. A struct without an `id` field
gets one: `pub id: ObjectId`, which the model generates when you make or
create it. You read it as `user.id`, and `User::find` takes it.

To use another field as the key, name it with `primary_key`. That field is
stored as `_id`, and you give its value when you create a document:

```rust
#[suprnova::document(collection = "products", primary_key = "sku", fillable = ["sku", "title"])]
pub struct Product {
    pub sku: String,
    pub title: String,
}

let anvil = Product::create(doc! { "sku": "A-1", "title": "Anvil" }).await?;
let same = Product::find("A-1").await?;
```

A key of type `ObjectId`, `String`, `i64`, or `i32` works out of the box. For
another type, implement `DocumentKey`: it says how a route parameter names a
key and whether the model can generate one.

In queries, the key's field name means `_id`, so
`User::query().where_("id", "=", id)` matches the stored `_id`.

### Mass assignment

`fillable` and `guarded` take the same lists as on an SQL model, and the
same rules apply: `create`, `update`, `make`, and `fill` drop a field the
guard refuses, or return an error when you turned on
`prevent_silently_discarding_attributes(true)`. Without either list, the
guard refuses the key. `suprnova::eloquent::unguarded` turns the guard off
for one task, as it does for SQL models.

A field that the struct doesn't declare is an error that names it, since the
model has nowhere to keep its value. A value of the wrong type, such as a
string for an `i64` field, is an error that names the field, and nothing is
written.

`make(attrs)` builds a model without storing it, and `fill(attrs)` sets
fields on one. Call `save` to store either.

### Field types and casts

Each field is stored through serde, so strings, numbers, booleans, `Vec`s,
nested structs, and `bson::Document` values keep their BSON types. A unit
enum is stored as its serde name, which is a string. Three field types get
the BSON type made for them:

| Field type | Stored as | Cast |
|---|---|---|
| `bson::DateTime` | BSON datetime | none needed |
| `chrono::DateTime<Utc>` | BSON datetime | `AsBsonDateTime` |
| `rust_decimal::Decimal` | `Decimal128` | `AsDecimal128` |

The macro selects the cast for a `chrono::DateTime<Utc>` or a `Decimal`
field, and for an `Option` of one. A BSON datetime holds milliseconds, so a
`chrono` time with finer precision reads back truncated to the millisecond.
Both casts also read a string, which is how a date or a decimal arrives in
attributes you build with `doc!`.

To store a field another way, write a type that implements
`DocumentCast<FieldType>` and name it in `casts`, as on an SQL model:

```rust
#[suprnova::document(casts = { colour = AsHexColour })]
pub struct Theme {
    pub colour: Colour,
}
```

When a stored document lacks a field, an `Option` field reads as `None` and a
`Vec` field reads as empty. Any other missing field is an error that names
it.

### Timestamps

When the struct has both `created_at` and `updated_at`, the model manages
them as BSON datetimes. `create` sets both, and `save`, `update`, `restore`,
and `increment` set `updated_at`. A struct with only one of the two fields
fails to compile. To turn the timestamps off, set `timestamps = false`. To
rename the fields, set `created_at = "..."` and `updated_at = "..."`.

### Soft deletes

With `soft_deletes`, `delete` sets the `deleted_at` field instead of removing
the document, and queries leave such trashed documents out. The field must
be an optional date-time. To use another field, set
`soft_deletes_column = "..."`.

```rust
user.delete().await?;                           // sets deleted_at
let trashed = User::only_trashed().get().await?;
let everyone = User::with_trashed().get().await?;

let user = trashed.into_vec().remove(0).restore().await?;
user.force_delete().await?;                     // removes the document
```

`restore` on a model without soft deletes is an error that names `restore`.

### Embedded documents

Mark a `Vec<T>` field `#[embeds_many]`, or an `Option<T>` field
`#[embeds_one]`, to store `T` values inside the document. `T` is a plain
struct that implements serde's `Serialize` and `Deserialize`. The model reads
and writes the field like any other, and it gains a method with the field's
name that answers the relation:

```rust
let mut user = User::find_or_fail(id).await?;
user.addresses()
    .save(&Address { street: "2 Engine Row".into(), city: "Leeds".into() })
    .await?;
user.addresses().destroy(&old_address).await?;
```

An `embeds_many` relation has `save`, `save_many`, `destroy`, and `clear`.
An `embeds_one` relation has `save` and `delete`. Each call writes to the
server and reloads the model from the result.

### Array and counter operators

These calls change one field on the server and reload the model:

| Call | Operator |
|---|---|
| `push(field, value)` | `$push`: append `value` |
| `push_unique(field, value)` | `$addToSet`: append unless present |
| `pull(field, value)` | `$pull`: remove every equal element |
| `increment(field, by)` | `$inc`, and set `updated_at` |
| `decrement(field, by)` | `$inc` with `-by`, and set `updated_at` |

```rust
user.push("tags", "rust").await?;
user.pull("tags", "math").await?;
user.increment("logins", 1).await?;
```

`field` can be a dotted path into an embedded document. A field the model
doesn't declare, or the key, is an error that names it, and so is an
`increment` amount that isn't an integer or a float.

### Events and observers

Document models fire the lifecycle events of SQL models, in Laravel's order:
`Saving`, then `Creating` or `Updating`, the write, `Created` or `Updated`,
and `Saved`. SQL models in Suprnova fire `Creating` before `Saving`; document
models keep Laravel's order. `delete` fires `Deleting`, `Trashed` when the model soft
deletes, and `Deleted`. `restore` fires `Restoring`, the update's events,
and `Restored`. Queries fire `Retrieving` and `Retrieved`. Each event is a
generic type in `suprnova::mongodb::events`, such as `Created<User>`.

A listener on `Saving`, `Creating`, `Updating`, `Deleting`, or `Restoring` can
cancel the write: the call returns a `400 Bad Request` error with the
listener's reason, and nothing is written. The first three carry the
attributes about to be written as a `bson::Document`, which the listener can
change.

To collect the callbacks in one place, implement `DocumentObserver` and
register it with `#[suprnova::observer]`, as for an SQL model:

```rust
use suprnova::bson::Document;
use suprnova::{DocumentObserver, EventResult};

pub struct UserObserver;

#[suprnova::observer(User)]
#[suprnova::async_trait]
impl DocumentObserver<User> for UserObserver {
    async fn creating(&self, attrs: &mut Document) -> EventResult {
        attrs.insert("tags", vec!["new"]);
        EventResult::ok()
    }
}
```

`User::observe(UserObserver).await` registers an observer by hand instead.

### Serialization

The model serializes as its fields: the key under its field name, an
`ObjectId` as its hex string, a datetime as RFC 3339 text, and a
`Decimal128` as its decimal text. `hidden = [...]` leaves fields out, and
`visible = [...]` writes only the fields it lists. So a handler can answer a
model as JSON:

```rust
let json = serde_json::to_value(&user)?;
// {"id":"6523...","name":"Ada","email":"ada@example.com","tags":["math"],...}
```

For the stored form, call `to_document()`, which writes the key as `_id`.

### Route binding

A document model binds from a route parameter by its key, like an SQL
model. A parameter that isn't a valid key, such as text that isn't an
`ObjectId`, binds nothing, so the route answers `404 Not Found` without a
query. A route with `with_trashed()` also binds trashed documents. A
parameter with a binding field, such as `{user:email}`, matches that field
as text.

### Why Suprnova diverges

- **Calls through a trait.** Laravel models extend
  `MongoDB\Laravel\Eloquent\Model`. A Suprnova document model implements the
  `DocumentModel` trait, so you import the trait to call `create` or `find`.
- **Declared fields.** A Laravel MongoDB model stores any attribute you set.
  A Suprnova model stores the fields its struct declares, so an attribute
  that names no field is an error instead of an extra stored field.
- **Embedded relations by field.** Laravel declares `embedsMany` in a
  relation method. Suprnova marks the field, so the embedded documents are
  part of the struct and read with it.

## Queries

A model's `query()` returns a `DocumentQuery`, the builder for its
collection. Chain conditions and finish with a call that runs the query:

```rust
use suprnova::{Direction, DocumentModel};

let adults = User::query()
    .where_("age", ">=", 18)
    .where_in("role", ["admin", "editor"])
    .order_by("name", Direction::Asc)
    .skip(10)
    .take(5)
    .get()
    .await?;
```

The method is `where_` because `where` is a Rust keyword.

### Conditions

| Call | Matches |
|---|---|
| `where_(field, op, value)` | `field` compared with `value` |
| `or_where(field, op, value)` | the same, joined with OR |
| `where_in(field, values)` | `field` equal to one of `values` (`$in`) |
| `where_not_in(field, values)` | `field` equal to none of `values` (`$nin`) |
| `where_null(field)` | `field` null or missing |
| `where_not_null(field)` | `field` present and not null |
| `where_between(field, low..=high)` | `field` from `low` to `high`, both included |
| `where_date(field, date)` | a datetime `field` on the UTC day `date` |
| `where_exists(field)` | `field` present, null or not |
| `where_raw(filter)` | a filter in MongoDB's query language |

`op` is one of `=`, `!=` (or `<>`), `<`, `<=`, `>`, `>=`, `like`, and
`not like`. `like` takes an SQL pattern, where `%` matches any run of
characters and `_` matches one character. It matches the whole value and
ignores case, so `where_("name", "like", "jo%")` matches `Joan` and `john`.

Conditions join with AND. `or_where` starts a new group, and the conditions
after it join that group. AND binds tighter than OR, as in SQL, so this
query matches people over 60, and admins in the core team:

```rust
let people = User::query()
    .where_("age", ">", 60)
    .or_where("role", "=", "admin")
    .where_("team", "=", "core")
    .get()
    .await?;
```

A field can be a dotted path into an embedded document, such as
`addresses.city`. `where_raw` takes its filter as written, so in it the key
is `_id`. An unknown operator, or `like` with a value that isn't a string,
makes the query an error that names `where_`.

### Order, offset, limit, and projection

`order_by(field, direction)` sorts, and later calls sort the ties of earlier
ones. `skip(n)` and `take(n)` set the offset and the limit. `project(fields)`
reads only those fields of each document. A projected query hydrates a model
only when its other fields are `Option` or `Vec` fields, so read projected
documents with `get_documents()`, which answers them as `bson::Document`
values.

### Reading

| Call | Returns |
|---|---|
| `get()` | the matching models, as a `Collection` |
| `get_documents()` | the matching documents, unread |
| `first()` | the first match, or `None` |
| `count()` | the number of matches |
| `exists()` | whether anything matches |
| `pluck(field)` | the value of `field` in each match |
| `distinct(field)` | the distinct values of `field` |
| `sum(field)`, `avg(field)`, `min(field)`, `max(field)` | one value, or `None` when nothing matches |
| `paginate(per_page, page)` | a `LengthAwarePaginator` with the total |
| `simple_paginate(per_page, page)` | a `Paginator` without a total |

The page is 1-based, and your handler passes it. For example, read it from
the query string:

```rust
use suprnova::Context;

let page = Context::query_param("page")
    .and_then(|page| page.parse().ok())
    .unwrap_or(1);
let users = User::query()
    .order_by("name", Direction::Asc)
    .paginate(15, page)
    .await?;
```

### Grouping and aggregates

`group_by(fields)` groups the matches. Add aggregates to it, then call
`get()`, which answers one `bson::Document` per group: the group fields and
the aggregates. `count()` adds `count`. `sum(field)`, `avg(field)`,
`min(field)`, and `max(field)` add `sum_<field>`, `avg_<field>`,
`min_<field>`, and `max_<field>`:

```rust
let per_role = User::query()
    .where_("active", "=", true)
    .group_by(["role"])
    .count()
    .avg("age")
    .order_by("count", Direction::Desc)
    .get()
    .await?;
// [{ "role": "user", "count": 12, "avg_age": 31.5 }, ...]
```

The order, skip, and take apply to the groups. `group_by` with no fields puts
every match in one group.

### Writing

These calls write to every matching document and return how many changed:

| Call | Writes |
|---|---|
| `update(doc)` | the fields of `doc` with `$set`, or `doc` as update operators |
| `increment(field, by)`, `decrement(field, by)` | `$inc` |
| `push(field, value)`, `pull(field, value)` | `$push`, `$pull` |
| `unset(fields)` | `$unset` |
| `delete()` | removes the matches, or trashes them when the model soft deletes |
| `force_delete()` | removes the matches, trashed or not |

`update` with plain fields leaves the other fields as they are. With update
operators, such as `doc! { "$inc": { "views": 1 } }`, it sends them as they
are. `update`, `increment`, and `decrement` also set `updated_at` when the
model manages timestamps. These calls don't fire model events, as Laravel's
query updates don't.

`upsert(values, unique_by)` inserts each value, or updates the document
whose `unique_by` fields equal the value's:

```rust
User::query()
    .upsert(
        vec![doc! { "email": "ada@example.com", "name": "Ada" }],
        &["email"],
    )
    .await?;
```

### Rendering without a server

`to_filter()` answers the parts of the find command the builder sends: the
filter, the sort, the skip, the limit, and the projection. `to_pipeline()`
answers the same query as aggregation stages. A group's `to_pipeline()`
answers its `$group` stage and the rest. Use them to check a query in a test:

```rust
let rendered = User::query()
    .where_("age", ">=", 18)
    .where_in("role", ["a", "b"])
    .to_filter()?;
assert_eq!(rendered.filter, doc! { "age": { "$gte": 18 }, "role": { "$in": ["a", "b"] } });
```

On a soft-deleting model, the filter includes the condition that leaves
trashed documents out.

### Chains that can't run

A chain that MongoDB can't run as written is an error that names the call,
not a query that matches more or less than you asked for. The builder checks
before it contacts the server:

- A write after `skip` or `take`: MongoDB writes every matching document.
- `upsert` after a condition, an order, `skip`, or `take`: it matches each
  value by its `unique_by` fields.
- `upsert` of a value without its `unique_by` fields, or without a key the
  model can't generate.
- `distinct(field)` ordered by another field, which the distinct values don't
  carry.
- `update` that mixes operators and fields, or that writes the key.
- A one-value aggregate such as `sum` with a projection.

### Why Suprnova diverges

- **Explicit page.** Laravel's `paginate` reads the page from the request.
  `paginate(per_page, page)` takes the page as an argument, so you can call
  it outside a request.
- **Writes take no limit.** Laravel MongoDB accepts a limit on some writes,
  such as `take(1)` before a delete. Suprnova refuses `skip` and `take` on
  every write, so a write never matches more or less than its conditions
  say. To write some of the matches, select their keys with `pluck` and
  write by them.
- **Aggregates by name.** Laravel's grouped aggregates come back as
  `aggregate`. Suprnova names each one after its function and field, so one
  group can carry several.

## Relations

This section describes relations between documents, and between documents
and SQL models.

## Queue

Set `QUEUE_DRIVER=mongodb` to keep queued jobs in MongoDB. The boot builds
three stores on the default connection:

- `MongoQueueDriver` over the `jobs` collection.
- `MongoFailedJobStore` over `failed_jobs`, which becomes the failed-job
  store, as `QUEUE_DRIVER=database` makes its table the store.
- `MongoBatchRepository` over `job_batches`, which becomes the batch
  repository unless your bootstrap installed one.

```ini
QUEUE_DRIVER=mongodb
MONGODB_URI="mongodb://127.0.0.1:27017"
MONGODB_DATABASE="shop"
```

Run workers with `queue:work` as for any other driver. The collections need
no migration: each store creates its indexes on its first operation.

A job is one document with Laravel's fields: `queue`, `payload`,
`attempts`, `reserved_at`, `available_at`, and `created_at`, the last three
as epoch seconds. Two fields of Suprnova's own hold the reservation:
`token`, which the worker settles the job with, and `reserved_until`, when
the reservation lapses.

A worker reserves a job with one `findOneAndUpdate`. It picks the earliest
available job on the worker's queues that no live reservation holds, sets
the reservation, and adds 1 to `attempts`. The server applies the update to
one document atomically, so two workers never reserve the same job. A job
whose worker died is reserved again once `reserved_until` passes, and that
delivery counts as an attempt.

`available_at` holds whole seconds. A job due when you push it is
available at once. A delayed job rounds its time up, so a worker never
reserves it early; it can start up to a second late.

`QUEUE_DRIVER=mongodb` uses the default connection and Laravel's
collection names. Each store also takes a connection and a collection of
your choice, for code that drives a store itself, such as a test or a
worker you start with `run_worker`:

```rust
use std::sync::Arc;
use suprnova::{Mongo, MongoBatchRepository, MongoFailedJobStore, MongoQueueDriver};

let reporting = Mongo::connection_named("reporting")?;
let jobs = Arc::new(MongoQueueDriver::with_collection(&reporting, "report_jobs")?);
let failed = MongoFailedJobStore::with_collection(&reporting, "report_failed_jobs")?;
let batches = MongoBatchRepository::new(&reporting);
```

A batch is one document. Each job that settles updates it once, moving
`pending_jobs` and `failed_jobs` with `$inc` and adding the job to
`settled_job_ids`, so jobs that finish at the same moment never lose a
count and a job delivered twice counts once. The `then`, `catch`, and
`finally` callbacks run once: the first worker to set `finished_at` runs
them.

## Cache

Set `CACHE_DRIVER=mongodb` to keep the cache in MongoDB. The boot builds
`MongoCache` on the default connection, over the `cache` collection for
entries and `cache_locks` for locks, with the `CACHE_PREFIX` and
`CACHE_DEFAULT_TTL` of the other drivers.

```ini
CACHE_DRIVER=mongodb
CACHE_PREFIX=shop-cache-
```

The `Cache` facade works as with any driver:

```rust
use std::time::Duration;
use suprnova::Cache;

Cache::put("report:today", &42, Some(Duration::from_secs(60))).await?;
Cache::increment("visits", 1).await?;
let added = Cache::add("slot:9", &"taken", Some(Duration::from_secs(30))).await?;
```

An entry is one document: `_id` (the prefix and the key), `value`,
`expires_at`, and `tags`. A value that is a whole number is stored as a
BSON integer, so `increment` adds to it on the server with `$inc` in one
`findOneAndUpdate`, and two increments at once never lose a step. Any other
value is stored as its JSON text.

Both collections have a TTL index on `expires_at`, which the store creates
on its first operation, so the server removes expired entries and locks.
The server's sweep runs once a minute, so every read also compares
`expires_at` with the time now and never returns an expired entry.

`Cache::add` is one upsert that matches only an expired entry: it inserts
a missing key, replaces an expired one, and fails on a live one, so two
callers never both add. Tags are an array on the entry, and
`Cache::flush_tags` deletes every entry whose tags hold one of the tags. A
lock is one document whose `_id` is the key: acquiring it is an upsert that
fails while another owner's lock is live, and releasing or refreshing it
checks the owner's token.

To use other collections or another connection, build the store and bind
it in your bootstrap:

```rust
use std::sync::Arc;
use suprnova::{App, CacheConfig, CacheStore, Mongo, MongoCache};

pub async fn register() -> Result<(), suprnova::FrameworkError> {
    Mongo::init().await?;
    let store = MongoCache::with_collections(
        &Mongo::connection()?,
        &CacheConfig::from_env()?,
        "app_cache",
        "app_cache_locks",
    )?;
    App::bind::<dyn CacheStore>(Arc::new(store));
    Ok(())
}
```

## Session

Set `SESSION_DRIVER=mongodb` to keep sessions in MongoDB. The session
middleware then stores them with `MongoSessionDriver`, in the collection
`SESSION_TABLE` names (`sessions` by default), on the MongoDB connection
`SESSION_CONNECTION` names or the default one.

```ini
SESSION_DRIVER=mongodb
SESSION_LIFETIME=120
```

Your bootstrap builds the middleware as it does for the database driver:

```rust
use suprnova::{SessionConfig, SessionMiddleware, global_middleware};

pub async fn register() {
    global_middleware!(SessionMiddleware::install(SessionConfig::from_env()).await);
}
```

The middleware is built before the boot registers the MongoDB connections,
so the driver looks the connection up on each call.

A session is one document: `session_id`, `user_id` (the default guard's
user), `guards` (each other guard signed in, with its user), `payload` (the
session data and the CSRF token), and `last_activity`. The store creates a
unique index on `session_id` on its first operation.

`destroy_for_user` and the other revocations find sessions by `user_id`
and `guards`, and delete them one at a time until none is left, so a
session signed in during the revocation goes too. A session a request
loaded and a revocation then deleted isn't written back. `gc` deletes the
sessions whose last activity is older than the lifetime, and the session
middleware's collector calls it every `SESSION_GC_INTERVAL` seconds.

The two-factor promotion renames the pending session to its new id with
one update of one document, so the old id stops working and the new one
holds the session in the same step.

### Why Suprnova diverges

- **No default URI.** Laravel's configuration falls back to
  `mongodb://localhost:27017` and the `laravel_app` database. Suprnova has
  no fallback: an unset `MONGODB_URI` is an error that names it, so an
  application never writes to a server it wasn't pointed at.
- **A facade of its own.** Laravel reaches MongoDB through
  `DB::connection('mongodb')`. Suprnova's `DB` facade speaks SQL, so
  MongoDB has the `Mongo` facade, and its default connection keeps
  Laravel's name, `mongodb`.
- **Configuration in code.** Laravel lists connections and driver options
  in `config/database.php`. Suprnova reads the default connection from the
  environment and takes named connections and pool options from
  `MongoConfig::builder()`.
- **Only the worker that holds a job settles it.** Laravel's database
  queue deletes a finished job by its id, so a worker whose reservation
  lapsed can delete a job another worker now runs. Suprnova's driver keeps
  a `token` and a `reserved_until` beside Laravel's fields, and settles a
  job by its token.
- **A release doesn't spend an attempt.** Laravel counts each reservation
  as an attempt and keeps the count when a job is released. Suprnova gives
  the delivery back, as its other drivers do.
- **Batch settlements count once.** Laravel's batch repository moves the
  counters on every settlement, so a job delivered twice counts twice.
  Suprnova records the settled jobs on the batch and moves the counters in
  the same update, so a redelivery changes nothing. MongoDB's 16 MiB
  document limit bounds a batch at about 300,000 jobs.
- **A chained job's successor is pushed before the acknowledgement.**
  Without a transaction, which a standalone server lacks, the driver can't
  enqueue the successor and drop the finished job in one step. The worker
  pushes first, as it does on Redis, so a crash between the two runs the
  finished job again rather than losing the rest of the chain.
- **Sessions are keyed by a `session_id` field, not `_id`.** A document's
  `_id` can't change, and the two-factor promotion has to give a session a
  new id atomically, so the id is a field with a unique index.

## Next

- [Database: Getting Started](database.md) - the SQL connections beside
  MongoDB
- [Eloquent](eloquent.md) - SQL models
- [Queues](queues.md) - queue drivers
- [Cache](cache.md) - cache stores
- [Session](session.md) - session drivers
