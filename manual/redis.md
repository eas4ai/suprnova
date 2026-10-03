# Redis

The `Redis` facade runs Redis commands of your own: a counter, a leaderboard,
a lock that outlives a request, a message to another service. The cache, the
queue, and the rate limiter already talk to Redis through their drivers; the
facade is for everything else. It is Laravel's `Redis` facade over named
connections, with typed methods for the common commands and the `redis`
client underneath.

```rust
use suprnova::Redis;

let redis = Redis::connection("default")?;
redis.set("greeting", "hello").await?;
let greeting = redis.get("greeting").await?; // Some("hello")
let visits = redis.incr("visits", 1).await?;
```

## Connections

The `default` connection reaches the server `REDIS_URL` names, and
`redis://127.0.0.1:6379` when it is unset. The database index is the URL's
path: `redis://127.0.0.1:6379/2` is database 2. To use other servers or
databases, name them in the bootstrap:

```rust
use suprnova::Redis;

pub async fn register() {
    Redis::define("sessions", "redis://10.0.0.5:6379/0").expect("a Redis URL");
    Redis::define("local", "unix:///run/redis/redis.sock").expect("a Redis URL");
}
```

`Redis::define` takes `redis://`, `rediss://`, and `unix://` URLs, with a
user and password when the server needs them. It replaces any connection of
that name, `default` included. A URL that is not a Redis URL is an error
that names the connection, never the URL, since the URL may hold a
password.

For what a URL cannot say, such as TLS with certificates of your own or a
server a Sentinel names, build the client with the re-exported `redis`
crate and give it to `Redis::define_client(name, client)`.

`Redis::connection(name)` returns the connection. Resolving it sends
nothing: the connection opens on its first command, and opens again on its
own after the server drops it. Every clone of a connection shares the one
connection to the server.

`Redis::connections()` lists the names resolved so far.
`Redis::purge(name)` forgets one; it closes once nothing holds it, and the
next `Redis::connection(name)` opens a new one. A name that no `define`
gave, other than `default`, is an error.

## Commands

The common commands are typed methods. Values go in and come out as text:

| Kind | Methods |
|---|---|
| Strings | `get`, `set`, `set_ex`, `mget`, `incr`, `decr` |
| Keys | `del`, `exists`, `expire`, `ttl`, `scan` |
| Hashes | `hset`, `hget`, `hgetall`, `hdel` |
| Lists | `lpush`, `rpush`, `lpop`, `rpop`, `lrange` |
| Sets | `sadd`, `srem`, `smembers` |
| Sorted sets | `zadd`, `zrange`, `zrangebyscore` |
| Other | `publish`, `eval` |

```rust
use suprnova::Redis;

let redis = Redis::connection("default")?;
redis.zadd("leaderboard", "ada", 3120.0).await?;
redis.zadd("leaderboard", "grace", 2980.0).await?;
let top = redis.zrange("leaderboard", 0, 9).await?;

redis.set_ex("otp:42", "813204", 300).await?;
let keys = redis.scan("otp:*").await?;
```

`scan` walks every cursor and returns every matching key, so the server is
never blocked the way `KEYS` blocks it.

`command` runs any other command and returns the reply as a `RedisValue`:

```rust
use suprnova::{Redis, RedisValue};

let redis = Redis::connection("default")?;
redis.command("PFADD", &["visitors", "ada", "grace"]).await?;
if let RedisValue::Int(count) = redis.command("PFCOUNT", &["visitors"]).await? {
    println!("{count} distinct visitors");
}
```

`command` sends a blocking command, such as `BLMPOP` or `XREAD` with
`BLOCK`, on a connection of its own, as the typed blocking methods do. It
refuses the commands that would change the connection every other command
shares, and the error names what to use instead: `subscribe` for
`SUBSCRIBE` and its family, `transaction` for `MULTI` and `EXEC`, the URL
for `SELECT` and `AUTH`, and a connection of your own from
`suprnova::redis::Client` for `WATCH` and `MONITOR`.

A `RedisValue` is `Nil`, `Int`, `Bytes` for a bulk string, `Status` for
a reply such as `OK`, `Array`, or, from a server speaking RESP3, `Map`,
`Double`, or `Bool`. `as_str` and `as_int` read the common cases. Binary
values go through `command`, whose arguments are bytes.

For what the facade does not cover, `client()` gives the connection the
commands use, as the `redis` crate's `ConnectionManager`. The crate is
re-exported as `suprnova::redis`, so you don't need to add it to your
dependencies:

```rust
use suprnova::Redis;

let redis = Redis::connection("default")?;
let mut client = redis.client()?;
let length: usize = suprnova::redis::cmd("XLEN")
    .arg("events")
    .query_async(&mut client)
    .await?;
```

The shared connection waits up to five seconds for a reply. A command that
blocks for longer belongs on a connection of its own; see
[Blocking commands](#blocking-commands).

### Lost connections

When the server drops the connection, a read is sent again on the new
connection: once, and once more for each retry `REDIS_COMMAND_RETRIES`
adds. The typed reads retry, and so do the commands Laravel lists as safe
to retry when they go through `command`: `GET`, `MGET`, `HGETALL`,
`LRANGE`, `SMEMBERS`, `ZRANGE`, `TTL`, `EXISTS`, and the rest. A write is
never sent again, since the server may have applied it before the
connection dropped; it returns the error, and the next command reaches the
new connection.

## Pipelines and transactions

`pipeline` sends every command its closure queues before it reads a reply,
and returns the replies in order. It saves a round trip per command:

```rust
use suprnova::Redis;

let redis = Redis::connection("default")?;
let replies = redis
    .pipeline(|pipe| {
        for id in 1..=100 {
            pipe.set(&format!("user:{id}:seen"), "1");
        }
    })
    .await?;
```

A pipeline is not atomic: another client's commands can run between its
own. `transaction` sends the same commands inside `MULTI` and `EXEC`, so
they run together, and when Redis rejects one as it is queued, an unknown
command or a wrong number of arguments, none of them runs:

```rust
use suprnova::Redis;

let redis = Redis::connection("default")?;
let replies = redis
    .transaction(|pipe| {
        pipe.decr("stock:42", 1);
        pipe.rpush("orders", &["42"]);
    })
    .await?;
```

The closure queues with `command` and with these typed methods: `set`,
`set_ex`, `get`, `del`, `incr`, `decr`, `expire`, `hset`, `lpush`, `rpush`,
`sadd`, `zadd`, and `publish`.

## Subscriptions

`subscribe` and `psubscribe` open a connection of their own and return a
subscription. Its `next` waits for the next message:

```rust
use suprnova::Redis;

let redis = Redis::connection("default")?;
let mut orders = redis.psubscribe(&["orders.*"]).await?;

tokio::spawn(async move {
    while let Some(message) = orders.next().await {
        // message.channel is "orders.created", message.pattern is Some("orders.*")
        println!("{}: {:?}", message.channel, message.payload_str());
    }
});

redis.publish("orders.created", "42").await?;
```

The shared connection keeps running commands while a subscription is open.
Dropping the subscription closes its connection.

## Blocking commands

`blpop`, `brpop`, `blmove`, `brpoplpush`, `bzpopmin`, and `bzpopmax` each run
on a connection of their own, opened for the call. A call that waits never
holds up another command, and it waits for its whole timeout; a zero
timeout waits until something arrives:

```rust
use std::time::Duration;
use suprnova::{Redis, RedisSide};

let redis = Redis::connection("default")?;
if let Some((list, job)) = redis.blpop(&["jobs:high", "jobs:low"], Duration::from_secs(5)).await? {
    println!("{job} from {list}");
}
let moved = redis
    .blmove("jobs:low", "jobs:running", RedisSide::Left, RedisSide::Right, Duration::ZERO)
    .await?;
```

## Events

`Redis::enable_events()` reports each command a connection runs to the
listeners `Redis::listen` adds, with the connection's name, the command, its
arguments, and how long it took. `Redis::listen_for_failures` hears the
commands that fail, with the error. Events are off until enabled, and
commands inside a pipeline or a transaction are not reported:

```rust
use suprnova::Redis;

Redis::listen(|event| {
    if event.duration.as_millis() > 50 {
        tracing::warn!(command = %event.command, connection = %event.connection, "slow Redis command");
    }
});
Redis::listen_for_failures(|event| {
    tracing::error!(command = %event.command, error = %event.error, "Redis command failed");
});
Redis::enable_events();
```

A listener runs on the task that sent the command, so keep it quick.

## Testing

The facade talks to a real server. To test code that uses it, give
`default`, or the connection the code names, a database of its own in the
test:

```rust
use suprnova::Redis;

#[tokio::test]
async fn counts_visits() {
    Redis::define("default", "redis://127.0.0.1:6379/15").unwrap();
    let redis = Redis::connection("default").unwrap();
    redis.del(&["visits"]).await.unwrap();
    // run the code under test
    assert_eq!(redis.get("visits").await.unwrap().as_deref(), Some("1"));
}
```

Connections are process-global, like the other facades. A connection opened
in one test's runtime opens again in the next test's, so every
`#[tokio::test]` can use it.

### Why Suprnova diverges

- **Connections are named in the bootstrap.** Laravel reads them from
  `config/database.php`; `Redis::define` does the same job in code, with
  the URL holding the host, port, database, user, and password.
- **No key prefix.** Laravel's phpredis client prefixes the keys of every
  command because it knows which arguments are keys. `command` cannot know
  that, so a prefix would apply to some commands and not others. Put the
  prefix in the key: `format!("myapp:{key}")`.
- **Only reads retry.** With `command_retries` set, Laravel sends any
  command again after a lost connection, writes included. Suprnova never
  sends a write twice.
- **A subscription is a value.** Laravel's `subscribe` takes a callback and
  blocks the process; here it returns a subscription that a task reads.
- **Blocking commands get their own connection.** In Laravel they block
  the connection every other command uses.
- **Typed methods take and return text.** Binary values go through
  `command` and `client`.

## Next

- [Cache](cache.md) - caching through the Redis driver
- [Queues](queues.md) - the Redis queue driver
- [Rate Limiting](rate-limiting.md) - limits kept in Redis
