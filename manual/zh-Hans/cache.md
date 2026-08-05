# 缓存

Suprnova 提供一个 Laravel 形状的 `Cache` 门面，背后是两种驱动程序之一 - 内存或者 Redis - 在启动时通过 `CACHE_DRIVER` 显式选定。这个门面是叠在 `CacheStore` trait 之上的一层薄壁，所以自定义后端能用和内置后端一样的方式接进来。

## 门面

```rust
use suprnova::Cache;
use std::time::Duration;

Cache::put("user:1", &user, Some(Duration::from_secs(3600))).await?;

let cached: Option<User> = Cache::get("user:1").await?;

if Cache::has("user:1").await? {
    // 命中
}

Cache::forget("user:1").await?;
```

每个方法都会在门面这条边界上通过 `serde_json` 序列化，所以任何 `T: Serialize + DeserializeOwned` 都能来回还原。门面之下的那个 trait（`CacheStore`）只看得到不透明的 JSON 字符串。

## 应用启动

缓存是在 `Server::run()` 的驱动程序启动步骤里被绑定的（参见[请求生命周期](lifecycle.md)）。`Cache::bootstrap` 读取已配置的 `CacheConfig`（或者从环境变量构造一个），并根据 `CacheConfig::driver` 来分发：

- `Memory` - 绑定一个带着配置好的前缀和默认 TTL 的 `InMemoryCache`。总是会成功。
- `Redis` - 连接到 `REDIS_URL`，并绑定得到的那个 `RedisCache`。如果这个 URL 无法访问，就**失败关闭**。不存在悄悄降级到内存的情况。

工作进程（`queue:work`、`schedule:run`、`workflow:work`）会走同一套启动流程，所以一个使用 `Cache::get` 的任务，看到的后端和 HTTP 处理程序看到的是同一个。

### 为什么 Suprnova 有所不同

Laravel 的 `cache.php` 配置会选定一个默认的存储，而当一个配置错误的后端在某些代码路径上失败时，Laravel 会悄悄换成 `array`（进程内）。这对 `php artisan tinker` 来说是一个富有生产力的默认值，但在生产环境里是一个陷阱 - 一次单独的 Redis 未命中，就会悄无声息地改变应用里每一次标签清空和锁获取的保证。

Suprnova 选择了相反的默认值。`CACHE_DRIVER=memory` 是显式的（也是 `cargo run` 的默认值），而 `CACHE_DRIVER=redis` 配上一个无法访问的 Redis，会让 `Server::from_config` 返回一个错误。二进制文件会以非零状态退出，并带上一条补救消息；supervisord/systemd 看到的是一次启动失败，而不是一个半工作状态的应用。

## 配置

| 环境变量 | 含义 | 默认值 |
|---|---|---|
| `CACHE_DRIVER` | `memory` 或 `redis` | `memory` |
| `REDIS_URL` | Redis URL（只有在 `driver=redis` 时才会被查阅） | `redis://127.0.0.1:6379` |
| `REDIS_PREFIX` | 应用到每一次存储操作上的键前缀 | `suprnova_cache:` |
| `CACHE_DEFAULT_TTL` | `Cache::put(None)` 的默认 TTL，以秒计；`0` 表示没有默认值 | `3600` |

未设置的 `CACHE_DRIVER` 会解析成 `Memory`；任何其他的值（大小写不敏感，且会被去除首尾空白），如果不是 `memory`/`in-memory`/`inmemory`/`redis` 之一，就会在启动时返回一个错误。

如果您不想要环境变量解析，也可以用编程的方式构建这份配置：

```rust
use suprnova::{Config, CacheConfig, cache::CacheDriver};

Config::register(
    CacheConfig::builder()
        .driver(CacheDriver::Redis)
        .url("redis://cache.internal:6379")
        .prefix("myapp:")
        .default_ttl(7200)
        .build(),
);
```

`CacheConfigBuilder::build` 是确定性的 - 未设置的字段会回退到 `CacheConfig::default()`，而不是重新读取环境变量。

### `forever` 契约在各后端间保持一致

`Cache::forever` 和 `Cache::remember_forever` 会完全绕开 `CACHE_DEFAULT_TTL`；不管配置的默认值是什么，这个值都永不过期。`Cache::put(key, value, None)` 则确实会应用那个默认值 - 这正是设置一个默认值的意义所在。

默认 TTL 的决议发生在门面这一层。两种 `CacheStore` 后端都会在存储这条边界上，把 `None` 按字面意思对待（不设过期），这正是为什么 `forever` 在内存和 Redis 上都真的意味着永远。

## 读、写、删

```rust
use suprnova::Cache;
use std::time::Duration;

// 带一个显式的 TTL 写入
Cache::put("session:42", &session, Some(Duration::from_secs(1800))).await?;

// 永久写入 - 绕开 CACHE_DEFAULT_TTL
Cache::forever("config:features", &features).await?;

// 读取（未命中或已过期时是 None）
let session: Option<Session> = Cache::get("session:42").await?;

// 存在性检查 - true 表示存在且未过期
if Cache::has("session:42").await? { /* … */ }

// Laravel 式拼写的否定
if Cache::missing("session:42").await? { /* 预热 */ }

// 一次调用完成读取并删除
let one_shot: Option<String> = Cache::pull("notice:welcome:42").await?;

// 如果这个键存在并被移除了，就返回 true
Cache::forget("session:42").await?;

// 清空所有内容（两种后端上都是按前缀限定范围的）
Cache::flush().await?;
```

`Cache::pull` **不是**原子的 - 它是一次 `get`，后面跟着一次 `forget`，和 Laravel 的 `Repository::pull` 是同一个形态。要做原子的出队，请用 `Cache::lock`（见下文）。

### 不重写就刷新一个 TTL

```rust
let refreshed = Cache::touch("session:42", Duration::from_secs(1800)).await?;
```

如果这个键存在，并且 TTL 被延长了，`touch` 就返回 `true`，否则返回 `false`。存储的值本身不受影响。

## Add - 缺失时写入（原子操作）

```rust
let won = Cache::add(
    "daily:winner",
    &user_id,
    Some(Duration::from_secs(86_400)),
).await?;
if won {
    send_winner_email(user_id).await?;
}
```

只有当这个键为空（或者已经过期）时，`Cache::add` 才会写入。写入成功返回 `true`，发生竞争时返回 `false`。在两种内置后端上都是**原子的**：

- `InMemoryCache` 在存在性检查加插入的整个过程中持有一个写锁
- `RedisCache` 使用 `SET key value NX EX ttl`（或者不带 `EX` 的 `NX`）

不覆盖 `add_raw` 的自定义 `CacheStore` 实现，会回退到一次非原子的先检查再写入，这与 Laravel 的 `Repository::add` 在没有原生 `add` 的存储上的回退行为是一致的。

## Remember - 有则取，无则算

```rust
let user = Cache::remember(
    "user:1",
    Some(Duration::from_secs(3600)),
    || async { User::find(1).await },
).await?;

let cfg = Cache::remember_forever("config:app", || async {
    load_config_from_db().await
}).await?;
```

`remember` 只在未命中时才调用您的闭包，然后存储结果。这个闭包返回 `Result<T, FrameworkError>`，所以领域层的失败会通过 `?` 冒泡上去，而不会污染缓存。

`Cache::sear(key, default)` 是 `remember_forever` 的 Laravel 拼写别名。同样的实现，同样的语义 - 用两个名字同时发布，这样迁移过来的代码读起来还是一样的。

### Remember 无法防止缓存击穿

`remember` 是一对非原子的 `get`-然后-`put`。同一个冷键上的 N 次并发未命中，会把闭包运行 N 次，写入 N 份结果。这和 Laravel 的 `Repository::remember` 一模一样，对常见情形来说也没问题（闭包是幂等的，写入的结果都相同）。

但在下面这些情况下就不太妙了：

- 这个闭包代价高昂（计算耗时 1 秒以上，或者会打到一个缓慢的上游）
- 这个键足够热门，以至于一次冷缓存事件会一次性把 N 个请求都送到后端存储上
- 这个闭包除了计算这个值之外还有副作用

对于这些情况，请用 `Cache::lock` 把它包起来：

```rust
use suprnova::Cache;
use std::time::Duration;

let key = "rebuild:user:1";

if let Some(guard) = Cache::lock(key, Duration::from_secs(10)).await? {
    let user = Cache::remember(
        "user:1",
        Some(Duration::from_secs(3600)),
        || async { User::find(1).await },
    ).await?;
    guard.release().await?;
    return Ok(user);
}

// 竞争失败了 - 获胜者正在计算。读取它们写入的任何东西，
// 或者回退到一个陈旧的值。
let user = Cache::get::<User>("user:1").await?
    .ok_or_else(|| FrameworkError::internal("cache miss after losing rebuild lock"))?;
```

## 锁

`Cache::lock` 返回一个持有所有权令牌的 `LockGuard`。当背后是 Redis 时，锁是建议性的，并且跨进程。

```rust
use suprnova::Cache;
use std::time::Duration;

if let Some(guard) = Cache::lock("job:42", Duration::from_secs(30)).await? {
    do_exclusive_work().await?;
    guard.release().await?;
}
// Some(guard) 意味着我们拥有它。None 意味着另一个持有者抢先了一步。
```

这个守卫暴露了：

| 方法 | 用途 |
|---|---|
| `guard.token()` | 读取所有权令牌（Rust 侧的名字） |
| `guard.owner()` | 同一个值，Laravel 拼写的别名 |
| `guard.refresh(ttl)` | 延长 TTL - 如果我们不再拥有这把锁，就返回 `false` |
| `guard.release()` | 如果我们仍然拥有这把锁就释放它 - 如果令牌已经不匹配了，就返回 `false` |

这里故意**没有 `Drop` 自动释放**。一把 Redis 锁必须跨进程边界被确认；drop 时自动释放，要么会悄悄把一把已经被偷走的锁再偷回来（错误），要么会把释放失败隐藏进析构函数的 panic 里（更糟）。释放是显式的，这样错误才能传播出去。

`refresh` 让一个长时间运行的任务可以延长它自己的锁，以避免一次自我造成的超时 - 树内的使用者请参见[幂等性](idempotency.md)。

## 原子计数器

```rust
// 如果不存在就初始化为 0，然后自增。返回新的值。
let visits = Cache::increment("page:visits", 1).await?;

// 反向步长用的是同样的形态
let remaining = Cache::decrement("quota:remaining", 1).await?;

// 自定义的增量
let total = Cache::increment("stats:downloads", 10).await?;
```

在两种内置后端上都是原子的：`InMemoryCache` 使用一个加了写锁的 `HashMap::entry`；`RedisCache` 使用 `INCRBY`/`DECRBY`。存储的值是一个 JSON 编码的整数，所以用同一个键调用 `Cache::get::<i64>("page:visits")` 能来回还原。

## 带标签的缓存

标签让您能用一次调用，就使一整个家族的相关条目失效。典型的用例是那些按资源划分的缓存，当资源发生变化时必须一起清空。

```rust
use suprnova::Cache;
use std::time::Duration;

// 存储在一个或多个标签下
Cache::tags_put(
    &["users", "user:1"],
    "user:1:profile",
    &profile,
    Some(Duration::from_secs(3600)),
).await?;

Cache::tags_put(
    &["users", "user:1"],
    "user:1:posts",
    &posts,
    Some(Duration::from_secs(600)),
).await?;

// 更新路径：丢弃每一个打了 `user:1` 标签的键
Cache::flush_tags(&["user:1"]).await?;
```

标签归属是**逐条目**的：每一次带标签的写入，都会把这次写入的标签集合安装成这个条目的事实来源，替换掉之前的任何标签。这带来两个值得了解的后果：

- 对一个此前带过标签的键，做一次不带标签的 `Cache::put`，会**清除**这个条目的标签。之后对旧标签的一次 `flush_tags`，不会删除这个仍然存活、但已不带标签的值。
- 用 `tags_put(&["b"], …)` 覆盖 `tags_put(&["a"], …)`，会让这个条目只对 `flush_tags(&["b"])` 有反应。

陈旧的正向索引引用，会在清空遍历的过程中，以及在 `flush()` 时被修剪掉，所以对于那些被写入却从未被清空过的标签，它们不会无限累积。

## 两种后端

| 特性 | `InMemoryCache` | `RedisCache` |
|---|---|---|
| 跨进程共享 | 否 | 是 |
| 持久化 | 否 | 是，如果 Redis 为此做了配置 |
| 原子 `add` | 是（写锁） | 是（`SET NX`） |
| 原子 `increment`/`decrement` | 是（写锁） | 是（`INCRBY`/`DECRBY`） |
| 带标签的缓存 | 是 | 是 |
| 锁 | 是 | 是（跨进程） |
| 亚秒级 TTL | 是（`tokio::time::Instant`） | 是（`PX`/`PEXPIRE`） |
| 通过什么选择 | `CACHE_DRIVER=memory`（默认） | `CACHE_DRIVER=redis` |

没有 Database 缓存驱动程序 - 上面这两种后端就是框架发布的全部。自定义后端可以实现 `CacheStore`，并直接绑定进容器；请参见下面的测试注入模式。

### 内存中的过期

`InMemoryCache` 会**在读取时惰性地**驱逐已过期的条目：`get_raw`、`has` 和 `add_raw` 会在第一次观察到一个条目已过期时，把它清除掉。被反复访问的键永远不会积累“僵尸”条目。

一个会写入大量高基数、短命的键、却从不把它们读回来的工作负载，没有这样的触发时机。这种情况下，请从一个周期性任务里调用 `InMemoryCache::purge_expired()` - 它会返回被移除的条目数。Redis 在服务端自己处理过期；那里不需要这样的对应物。

### Redis 的 TTL 精度

每一个 Redis TTL 都会走 `PX` / `PEXPIRE`，而不是 `EX` / `EXPIRE`。这避开了两个陷阱：

- 亚秒级的 `Duration` 在 `EX` 下会被截断成 `0 秒`，而 Redis 会拒绝这个值（`SET … EX 0`），或者更糟，把它解读成“删除这个键”（`EXPIRE key 0`）。
- `Duration::ZERO` 在调用之前会被夹到 1 毫秒，所以这两条拒绝路径，用户代码都走不到。

## 测试

把一个 `InMemoryCache` 绑定进 `TestContainer`，门面就会像解析任何其他存储一样解析出它：

```rust
use std::sync::Arc;
use suprnova::{Cache, CacheStore, InMemoryCache};
use suprnova::container::testing::TestContainer;

#[tokio::test]
async fn cache_round_trips() {
    let _guard = TestContainer::fake();
    TestContainer::bind::<dyn CacheStore>(Arc::new(InMemoryCache::new()));

    Cache::put("k", &"v", None).await.unwrap();

    let v: Option<String> = Cache::get("k").await.unwrap();
    assert_eq!(v.as_deref(), Some("v"));
}
```

`TestContainer::bind` 写入的是线程本地作用域，所以并行的测试不会把缓存状态泄漏给彼此。三层查找模型请参见[服务容器](container.md)一章。

## 模式

几个值得叫出名字的、反复出现的形态：

```rust
// 层级式的、用冒号分隔的键 - 与 Laravel 使用的约定相同
Cache::put("users:1:profile", &profile, None).await?;
Cache::put("posts:123:comments:count", &count, None).await?;

// 按数据易变程度设置 TTL
Cache::put("stats:active", &count, Some(Duration::from_secs(60))).await?;
Cache::put("config:features", &features, Some(Duration::from_secs(3600))).await?;
Cache::forever("translations:en", &translations).await?;

// 围绕一次写入的、按标签失效的缓存
async fn update_user(id: i64, data: UserUpdate) -> Result<User, FrameworkError> {
    let user = User::update(id, data).await?;
    Cache::flush_tags(&[&format!("user:{}", id)]).await?;
    Ok(user)
}
```

## 下一步

- [配置](configuration.md) - `Config::register` 和环境变量如何结合
- [速率限制](rate-limiting.md) - 那个 Laravel 形状的 `RateLimiter` 门面就是建在 `Cache` 之上的
- [幂等性](idempotency.md) - 请求去重中间件从头到尾都在用 `Cache::lock`
- [服务容器](container.md) - `CacheStore` 是如何被绑定和解析的
- [错误模型](error-model.md) - 当 Redis 在请求中途无法访问时，`Cache::*` 会返回什么
