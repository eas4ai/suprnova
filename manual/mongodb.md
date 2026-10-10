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

This section describes document models, structs that
`#[suprnova::document]` stores in a collection.

## Queries

This section describes the document query builder that a model's `query()`
returns.

## Relations

This section describes relations between documents, and between documents
and SQL models.

## Queue

This section describes the `mongodb` queue driver, its batch repository, and
its failed-job store.

## Cache

This section describes the `mongodb` cache store and its locks.

## Session

This section describes the `mongodb` session driver.

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

## Next

- [Database: Getting Started](database.md) - the SQL connections beside
  MongoDB
- [Eloquent](eloquent.md) - SQL models
- [Queues](queues.md) - queue drivers
- [Cache](cache.md) - cache stores
- [Session](session.md) - session drivers
