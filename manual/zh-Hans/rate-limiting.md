# 速率限制

Suprnova 提供两个互补的速率限制表面：

| 表面 | 使用场景…… | 后端 |
|---------|-------------|---------|
| `RateLimiterDriver` + `RateLimitMiddleware` | 您想要针对任意存储（Redis ZSET、内存中的双端队列）做严格的滑动窗口强制执行 | `dyn RateLimiterDriver` |
| `RateLimiter` + `ThrottleRequestsMiddleware` | 您想要 Laravel 形状的具名限流器、`attempt()` 工作流回调，或者 `X-RateLimit-*` 响应头 | `Cache` 存储（内存或 Redis） |

滑动窗口驱动程序是 Suprnova 的原生形状 - 每个请求一个槽位，没有单独的计时器键，在 Redis 上做原子的 Lua 求值。Laravel 门面则是迁移过来的应用会去用的东西，也是具名限流器 / 响应回调这套模式所需要的。两者按设计共存，一条路由可以把两者都叠加上。

## 滑动窗口驱动程序 SPI

`RateLimiterDriver` 是这个滑动窗口算法的存储 SPI。每一个键都追踪着一个命中时间戳的双端队列。每一次 `try_acquire` 时，比 `now - window` 更早的条目都会被驱逐；如果剩余计数低于 `max_requests`，`now` 就会被追加进去，这次调用被接受。否则就拒绝。

```rust
use std::sync::Arc;
use std::time::Duration;
use suprnova::rate_limit::memory::InMemoryRateLimiter;
use suprnova::rate_limit::{RateLimiterDriver, SlidingWindowConfig};

let limiter: Arc<dyn RateLimiterDriver> = Arc::new(InMemoryRateLimiter::new());
let cfg = SlidingWindowConfig {
    max_requests: 60,
    window: Duration::from_secs(60),
};
let ok = limiter.try_acquire("user:42", &cfg).await?;
if !ok {
    let wait = limiter.retry_after("user:42", &cfg).await?;
    // wait 是一个 Option<Duration>，表示还要多久，桶里最老的那个
    // 槽位才会过期。
}
```

### 内置驱动程序

| 驱动程序 | 存储 | 通过什么选定 |
|--------|---------|--------------|
| `InMemoryRateLimiter` | 逐进程的 `HashMap<String, Bucket>`，配合 `tokio::time::Instant`，这样 `start_paused` 测试就能驱动这个时钟 | `RATE_LIMIT_DRIVER=memory`（默认） |
| `RedisRateLimiter` | Redis ZSET + 原子的 Lua check-and-record | `RATE_LIMIT_DRIVER=redis` + `RATE_LIMIT_REDIS_URL` |

`bootstrap_from_env()` 会把匹配的驱动程序接进容器。在生产环境之外，一个未知的驱动程序值会回退到内存，并记一条 `warn!` 日志。

### 生产环境对内存驱动程序失败关闭

在生产环境里，解析到内存限流器，是一次启动失败：

```
refusing to boot in production: RATE_LIMIT_DRIVER is unset, which defaults
to the in-memory limiter. Per-process buckets mean every configured quota
is multiplied by your replica count and reset by every deploy...
```

内存驱动程序把它的桶保存在一个进程的堆里。在 N 个副本背后，每一个副本都各自保有自己的计数，所以一个“15 分钟 5 次尝试”的密码重置节流，实际上是 5N 次，并且每一次部署都会把它们全部重置为零。您配置的限制，不是您实际得到的限制 - 而且没有任何东西会告诉您这一点，因为这些请求会成功，而这正是从外部看一个正常工作的节流应该有的样子。它会以一次撞库或者账户枚举事件的形式浮现出来，而不是一个错误。

一个**未被识别的**驱动程序值，出于同样的原因会失败：它会回退到内存。`RATE_LIMIT_DRIVER=Redis` - 大写了首字母 - 原本只会在启动时警告一次，然后悄悄地让一个多副本部署逐进程节流下去。这正是最有可能触达生产环境的情形，因为它看起来是配置过的。

要么把它指向 Redis：

```env
RATE_LIMIT_DRIVER=redis
RATE_LIMIT_REDIS_URL=redis://cache.internal:6379
```

或者，如果您确实只运行单个进程，就明确说出来：

```env
RATE_LIMIT_ALLOW_MEMORY_IN_PRODUCTION=true
```

开发、测试和**预发布**环境不受影响。预发布环境特意没有被把关，理由和邮件那道防护一样：强制失败会促使团队把这个覆盖项全局设置上，从而恰好在最要紧的地方解除了这道检查。

### `RateLimitMiddleware`

这是围绕驱动程序的 HTTP 包装器。用一个 `key_fn` 闭包构造它，来驱动逐请求的桶选择：

```rust
use std::sync::Arc;
use std::time::Duration;
use suprnova::container::App;
use suprnova::rate_limit::{
    BackendErrorPolicy, RateLimitMiddleware, RateLimiterDriver, SlidingWindowConfig,
};

let limiter: Arc<dyn RateLimiterDriver> =
    App::resolve_make::<dyn RateLimiterDriver>().unwrap();

let mw = RateLimitMiddleware::new(
    limiter,
    SlidingWindowConfig {
        max_requests: 100,
        window: Duration::from_secs(60),
    },
    |req| format!("route:{}", req.path()),
)
.on_backend_error(BackendErrorPolicy::FailClosed);
```

被拒绝时（超出配额），它会返回带着 `Retry-After` 请求头的 HTTP 429。

### 不只按调用方限流，也要按收件人限流

一个以地址建键的限制，回答的是*某一个客户端是不是发出了太多请求*这个问题。它没法回答*某一个邮箱是不是正在被灌爆*这个问题。一个分散在僵尸网络、代理池，或者单个 IPv6 `/64` 里的攻击者，能在每一个按 IP 计的预算之下都保持不超标，同时给一个受害者发出成千上万封密码重置邮件 - 被耗尽的资源是那个收件箱，而这些请求唯一的共同点，就是受害者的地址。反过来也一样糟：在运营商级 NAT 或者一个办公网关背后，按 IP 的限制会因为一个成员的行为，惩罚一整群人。

`identity_key` 会针对*被操作*的那个账户来给一个桶建键：

```rust
use suprnova::rate_limit::{identity_key, names_identity};

let per_recipient = RateLimitMiddleware::new(
    limiter.clone(),
    SlidingWindowConfig { max_requests: 3, window: Duration::from_secs(900) },
    |req| identity_key(req, "email", "auth-issuance"),
)
.key_reads_body(4096)
.only_when(|req| names_identity(req, "email"))
.on_backend_error(BackendErrorPolicy::FailClosed);
```

把它*叠加*在一个按 IP 的限流器*旁边*，而不是用一个替换另一个。两者各自能捕捉对方捕捉不到的东西：按 IP 能阻止一台主机枚举许多地址；按收件人则能阻止许多台主机针对同一个地址。

有三个细节承载着这份安全性：

- **`key_reads_body`** 会在这个键被计算出来之前，先缓冲这个请求体（缓冲到给定的上限为止），这样这个字段既能从一个表单编码的 POST 里读出来，也能从一个查询字符串里读出来。它是选择加入的，因为缓冲是一份工作，一个未认证的调用方能够强迫您去做；这个上限则约束住了它。一个超出上限的请求体，会被用 413 拒绝，而不是不建键就放行 - 否则给请求体填充内容，就会成为逃出这个限制的一条路。
- **`only_when`** 会为那些没有指名任何人的请求跳过这个限流器。没有它，那些请求就会落进 `identity_key` 的地址回退里，被计入*这个*限流器的配额 - 而由于按收件人的预算通常是这一对里更紧的那个，它就会静默地变成每一条不指名任何人的路由上那个起约束作用的限制。
- **这个值会被归一化并哈希。** `Alice@Example.com` 和 `alice@example.com` 到达的是同一个邮箱，必须共享同一个桶，否则改一下大小写就能绕过这个限制。之所以要哈希这个结果，是因为一个速率限制后端经常是一个共享的 Redis，访问控制比主数据库要弱，而一次键转储不应该读起来像是一份“谁正在重置密码”的名单。

### 后端错误策略

`BackendErrorPolicy` 管辖的是，当限流器*后端*本身出错时会发生什么 - 比如 Redis 不可达 - 这和一个请求正当地超出了它的配额是两回事。这时后端没法做出判断，所以中间件必须在可用性和这个限制的保证之间做出选择。

| 策略 | 行为 | 何时使用 |
|--------|-----------|-------------|
| `FailOpen`（默认） | 放行请求；记 `warn` 级别的日志 | 大多数公开 API - 一次限流器故障不应该拖垮流量 |
| `FailClosed` | 用 HTTP 503 + `Retry-After: 1` 拒绝；记 `error` 级别的日志 | 敏感路由（登录、密码重置、支付），在这些场景下，后端故障期间不受限的流量，比短暂拒绝要更糟 |

在中间件上用 `.on_backend_error(BackendErrorPolicy::FailClosed)` 来选择。配额耗尽的请求始终是 429，无论策略是什么 - 这个策略只影响后端错误时的兜底行为。

## 由缓存支撑的 Laravel 形状门面

`RateLimiter`（这个结构体）镜照的是 `Illuminate\Cache\RateLimiter`。它是一个建在 Suprnova [`Cache`](cache.md) 门面之上的固定窗口计数器。当您需要具名限流器、`attempt()` 工作流，或者任何时候您想要 Laravel 应用期望的那些 `X-RateLimit-*` 请求头时，就用它。

### 存储布局

对于一个衰减 `D` 秒的尝试计数器键 `K`：

- `K` - 一个 i64 计数器，每次 `hit` 都会递增。初始值是 0（通过 `Cache::add`）。
- `K:timer` - 一个 i64 的、窗口结束时刻的 unix 秒数时间戳，通过 `Cache::add` 设置，这样窗口里只有第一个调用方能钉住这个截止时间。

这两个键带着相同的 TTL，所以窗口结束时，缓存会自动把它们清理掉。当计数器已经达到 `max_attempts`，但 `:timer` 已经不在了时，`too_many_attempts` 会重置这个计数器 - 这正是让窗口在一段配额耗尽期之后向前滑动的原因。

### 计数器 API

```rust
use suprnova::RateLimiter;

// 消耗一次尝试；如果窗口缺失，就初始化它。
let n = RateLimiter::hit("login:1.2.3.4", 60).await?;

// 消耗一次尝试，并在同一次原子往返里测试这个限制。
// 当这次命中把这个桶推过了 `max`（拒绝这个
// 请求）时返回 `true`，被接受时返回 `false`。请用它来代替单独的
// `too_many_attempts` + `hit` 这一对：把检查和命中拆成两次调用，
// 会让并发请求溜过这个限制（一次先检查后行动的竞态）。
// `max` 传 `i64::MAX` 意味着“无限制” - 总是放行，但依然计数。
let over_limit = RateLimiter::hit_and_check("login:1.2.3.4", 5, 60).await?;
if over_limit { /* 返回 429 */ }

// 按 N 递增；对“按成本加权”的限制很有用（每个请求
// 消耗不止一次尝试）。
let n = RateLimiter::increment("api:user:1", 60, 5).await?;

// 读取当前计数（从未命中过或者已过期时为 0）。
let attempts = RateLimiter::attempts("login:1.2.3.4").await?;

// 距离窗口重新打开还有多少秒（没有窗口打开时为 0）。
let secs = RateLimiter::available_in("login:1.2.3.4").await?;

// 触发之前还剩多少次重试。
let remaining = RateLimiter::remaining("login:1.2.3.4", 5).await?;
// retries_left 是 remaining 的 Laravel 拼法别名。
let remaining = RateLimiter::retries_left("login:1.2.3.4", 5).await?;

// 这个桶现在是不是正处于超限状态（窗口仍然打开）？
let over = RateLimiter::too_many_attempts("login:1.2.3.4", 5).await?;

// 只丢弃计数器（计时器还在 - 窗口依然被钉住）。
RateLimiter::reset_attempts("login:1.2.3.4").await?;

// 把计数器和计时器都丢弃。
RateLimiter::clear("login:1.2.3.4").await?;
```

### `attempt()` 工作流

只有当这个桶还在配额之内时，才运行一个回调；只有回调真正运行了，这次命中才会被消耗：

```rust
let result = RateLimiter::attempt(
    "login:1.2.3.4",
    5,
    || async { do_login_work().await },
    60,
).await?;
match result {
    Some(value) => { /* 回调运行了，尝试被计数 */ }
    None => { /* 超出限制，回调没有运行 */ }
}
```

这对登录表单来说是正确的形状 - 除非工作真的触达了这个回调，否则您不会消耗一次尝试。

### 具名限流器

在启动时注册，在请求时解析。Laravel 那一侧的名字 `for` 是一个 Rust 保留关键字，所以 Rust 这一侧的主要名字是 `define`；那个字面意义上的 Laravel 别名，则通过 `r#for` 暴露出来。

```rust
use suprnova::{Limit, RateLimiter};

// 在启动时 - `define` 是 Rust 这一侧的主要名字。
RateLimiter::define("api", |req| {
    // 是 `req.ip()`，不是裸的 `X-Forwarded-For` 请求头 - 见下文。
    let key = req.ip().unwrap_or_else(|| "anon".into());
    Limit::per_minute(60).by(format!("ip:{key}")).into()
});

// Laravel 那一侧的别名 - 在关键字转义拼法下的同一个东西。
RateLimiter::r#for("uploads", |_req| Limit::per_hour(100).into());

// 解析。
let cb = RateLimiter::limiter("api").unwrap();
let limit_result = cb(&request);
```

一个具名限流器回调，返回一个 [`LimitResult`]，可以从以下几种东西构造出来：

- 单个 `Limit` - 应用这个限制。
- 一个 `Vec<Limit>` - 应用每一个限制；最先触发的那个胜出。
- 一个 `HttpResponse` - 立即用这个响应短路（用于通过 `Limit::none()` 实现的“管理员拥有无限制访问权限”，或者直接拒绝这个请求）。

### 清理键

`RateLimiter::clean_rate_limiter_key(key)` 会从一个键里剥掉 `&abc;` 这种 HTML 实体标记 - Laravel 把它用在那些要经过 `htmlentities` 往返的用户提供字符串上。Suprnova 精确地复现了这个剥离阶段，但**不会**前置那个 `htmlentities` 编码（那只对非 UTF-8 输入才要紧，和 Rust 的 `String` 无关）。这个函数在 Suprnova 内部是确定性且幂等的；需要和一个 PHP 服务做到字节级一致哈希的消费方，应该自己对输入跑一遍 `htmlentities` 预处理步骤。

```rust
assert_eq!(RateLimiter::clean_rate_limiter_key("a&amp;b"), "aab");
```

## `Limit` 构建器

这是具名限流器回调返回的那个数据类型。简写构造函数镜照的是 Laravel 的 `Limit::per*`：

```rust
use suprnova::Limit;
use std::time::Duration;

Limit::per_second(10, 1);           // 每 1 秒 10 次（max_attempts、decay_seconds）
Limit::per_minute(60);              // 每分钟 60 次
Limit::per_minutes(5, 100);         // 每 5 分钟 100 次（衰减参数在前，Laravel 签名）
Limit::per_hour(1_000);             // 1000/小时
Limit::per_hours(6, 5_000);         // 每 6 小时 5000 次
Limit::per_day(10_000);             // 10000/天
Limit::per_days(7, 50_000);         // 每 7 天 50000 次
Limit::new(123, Duration::from_secs(45));  // 裸构造函数

// 构建器链。
let l = Limit::per_minute(5)
    .by("user:42")
    .response(|req| {
        suprnova::HttpResponse::text("blocked").status(429)
    })
    .after(|response| response.status_code() >= 400);
```

- `.by(key)` - 设置这个桶键。空键代表“全局”（每一个调用方共享同一个桶）。
- `.response(callback)` - 当这个限制被触发时，生成一个自定义响应；默认是纯粹的 429 "Too Many Attempts."。
- `.after(callback)` - 只有当 `callback(response)` 返回 true 时，才消耗这次尝试。规范用法：只统计失败的登录（`after(|r| r.status_code() >= 400)`）。

`Limit::none()` 返回一个 `Unlimited`（一个 `max_attempts = i64::MAX` 的 `GlobalLimit`）。从一个具名限流器里返回它，是 Laravel 用来绕开限制的模式。`GlobalLimit` 本身是 `Limit` 之上带着一个空键的薄封装，保留它是为了和 `Illuminate\Cache\RateLimiting\GlobalLimit` 保持对等。

## `ThrottleRequestsMiddleware`

这是围绕缓存支撑门面的 HTTP 包装器。镜照的是 `Illuminate\Routing\Middleware\ThrottleRequests`。三个构造函数：

```rust
use suprnova::{Limit, ThrottleRequestsMiddleware};

// 具名限流器 - 在请求时通过 RateLimiter::limiter(name) 解析。
ThrottleRequestsMiddleware::by_name("api");

// 内联的 max/decay/prefix - 字面意义上的 Laravel `throttle:60,1` 形状。
ThrottleRequestsMiddleware::with(60, 1, "myroute");

// 显式的 Limit 列表 - 最先触发的那个胜出；最符合 Rust 惯用法。
ThrottleRequestsMiddleware::with_limits(vec![
    Limit::per_hour(5_000).by("user:1"),
    Limit::per_minute(60).by("user:1"),
]);
```

把它接进一个路由组：

```rust
use suprnova::{Limit, RateLimiter, Router, ThrottleRequestsMiddleware};

RateLimiter::define("api", |req| {
    Limit::per_minute(60)
        .by(req.ip().unwrap_or_else(|| "anon".into()))
        .into()
});

let router = Router::new()
    .get("/api/items", list_items)
    .post("/api/items", create_item)
    .middleware(ThrottleRequestsMiddleware::by_name("api"));
```

### 以 `req.ip()` 建键，绝不要以标头建键

`X-Forwarded-For` 是由调用方提供的。一个以裸标头建键的限流器，只要每次请求发送一个不同的值，就会被打败 - 攻击者可以自己挑桶，这样配额就变成了逐请求的，而不是逐客户端的。

`Request::ip()` 是安全的读取方式。它只有在**这个 TCP 对端列在 `APP_TRUSTED_PROXIES` 里**时，才会返回 `X-Forwarded-For` / `X-Real-IP`；否则就返回对端地址，所以来自除您自己代理之外任何人的标头都会被忽略。

这条推论同样要紧：当那个变量未设置时 - 也就是默认情况 - 在一个终结代理背后，`req.ip()` 会在每一次请求上都返回*这个代理自己的*地址，应用里的每一个按 IP 的限制，都会塌缩进同一个共享的桶。`ThrottleRequestsMiddleware::with(20, 1, "login")` 这时候的意思，就变成了所有用户加在一起每分钟 20 次尝试，而任何一个调用方都能把它花光，从而把所有人都锁在外面。部署在 nginx、Traefik、ALB 或者 Cloudflare 背后，就意味着要设置 [`APP_TRUSTED_PROXIES`](env-vars.md#behind-a-reverse-proxy-set-app_trusted_proxies)。

### 响应头

每一个被包装的响应都带着：

- `X-RateLimit-Limit` - 配置好的 `max_attempts`。
- `X-RateLimit-Remaining` - 这个桶还剩下的重试次数。

429 响应还会额外带着：

- `Retry-After` - 距离窗口重新打开还有多少秒。
- `X-RateLimit-Reset` - 这个桶重新打开时刻的 unix 秒数时间戳。

这精确匹配了 Laravel `ThrottleRequests::getHeaders` 的形状。

### 缺失的具名限流器

当一条路由被接到 `by_name("X")` 上，但在 `X` 这个名字下没有注册任何限流器时，中间件会返回 HTTP 503，响应体里指名了这个缺失的限流器。Laravel 会抛出 `MissingRateLimiterException`；我们把它表现成一个 HTTP 响应，这样一次配置错误的启动就不会让工作线程 panic。

### 驱动程序与门面的组合

这两个中间件可以在同一个路由器上共存。先叠加滑动窗口驱动程序来保证底层的公平性，再叠加缓存支撑的节流来做逐端点的具名限制：

```rust
let router = Router::new()
    .get("/api/items", list_items)
    .middleware(RateLimitMiddleware::new(limiter_driver, cfg, key_fn))
    .middleware(ThrottleRequestsMiddleware::by_name("api"));
```

## 配置

驱动程序 SPI 通过环境变量配置；缓存支撑的门面，则在您配置 [`Cache`](cache.md) 存储（内存或 Redis）的地方一并配置。

| 变量 | 用途 | 默认值 |
|----------|---------|---------|
| `RATE_LIMIT_DRIVER` | 驱动程序 SPI 的启动引导 | `memory`（生产环境里被拒绝 - 见上文） |
| `RATE_LIMIT_ALLOW_MEMORY_IN_PRODUCTION` | 生产环境失败关闭的覆盖项 | 未设置 |
| `RATE_LIMIT_REDIS_URL` | Redis 驱动程序 | `redis://127.0.0.1:6379` |
| `RATE_LIMIT_PREFIX` | Redis 键前缀 | `suprnova:` |
| `CACHE_DRIVER` / `REDIS_URL` / `CACHE_DEFAULT_TTL` / `REDIS_PREFIX` | 缓存支撑的 `RateLimiter` 门面（参见 [`Cache`](cache.md)） | 各不相同 |

## 从 Laravel 迁移

| Laravel | Suprnova |
|---------|----------|
| `RateLimiter::for('api', fn ($req) => Limit::perMinute(60))` | `RateLimiter::define("api", \|req\| Limit::per_minute(60).into())` 或者 `RateLimiter::r#for(...)` |
| `RateLimiter::hit($key, $decay)` | `RateLimiter::hit(key, decay).await?` |
| `RateLimiter::tooManyAttempts($key, $max)` | `RateLimiter::too_many_attempts(key, max).await?` |
| `RateLimiter::availableIn($key)` | `RateLimiter::available_in(key).await?` |
| `RateLimiter::attempt($key, $max, $cb, $decay)` | `RateLimiter::attempt(key, max, \|\| async { ... }, decay).await?` |
| `RateLimiter::retriesLeft($key, $max)` | `RateLimiter::retries_left(key, max).await?` |
| `RateLimiter::cleanRateLimiterKey($key)` | `RateLimiter::clean_rate_limiter_key(key)` |
| `Limit::perMinute(60)->by($ip)->response(fn () => abort(429))` | `Limit::per_minute(60).by(ip).response(\|_\| HttpResponse::text("...").status(429))` |
| `Limit::perMinutes(3, 100)` | `Limit::per_minutes(3, 100)` |
| `Limit::none()` | `Limit::none()` |
| `throttle:api` 中间件 | `ThrottleRequestsMiddleware::by_name("api")` |
| `throttle:60,1` 中间件 | `ThrottleRequestsMiddleware::with(60, 1, "")` |
| `X-RateLimit-Limit/Remaining/Reset` + `Retry-After` 响应头 | 相同的响应头，相同的形状 |

### 为什么 Suprnova 有所不同

Laravel 只提供一种形状：`Illuminate\Cache\RateLimiter`（缓存支撑的固定窗口计数器），配上作为其 HTTP 包装器的 `Illuminate\Routing\Middleware\ThrottleRequests`。Suprnova 既提供那种形状，*又*提供一个原生的滑动窗口驱动程序 SPI，因为有两个真实的问题，需要两个真实的答案。

一个缓存支撑的计数器，是“我有具名限流器、响应回调、只统计失败登录的 after 回调，并且我想和 Laravel 的迁移保持源码兼容”这个问题的正确答案。而对“我需要针对一个 Redis ZSET、用原子 Lua 求值、没有单独计时器键的精确逐请求滑动窗口强制执行”这个问题，它就是错误答案了。第二个问题才是大多数触及 Tokio 并发极限的 Rust 服务真正遇到的，所以 `RateLimiterDriver` + `RateLimitMiddleware` 是并存的，而不是被挡在一个 feature 标志后面。

后端错误策略也是 Suprnova 的一项新增内容。Laravel 的中间件从不会浮现出一个“限流器坏掉了”这样的判断，因为 PHP 那种逐请求的生命周期把它藏起来了 - 下一个请求会拿到一个全新的进程。而一个失去 Redis 连接十秒钟的长期存活的 Tokio 工作线程，必须决定拿那段窗口期间到达的请求怎么办；`BackendErrorPolicy::FailOpen`（默认）与 `FailClosed` 之间的选择，正是把这个决定明确地暴露了出来。

## 下一步

- [中间件](middleware.md) - 中间件如何在请求链里组合、运行和短路
- [缓存](cache.md) - Laravel 形状的 `RateLimiter` 门面构建于其上的那个存储
- [配置](configuration.md) - 缓存和 Redis 后端的类型化配置
- [认证流程](auth-flows.md) - `LoginThrottleMiddleware` 和暴力破解锁定模式，构建在这个表面之上
- [错误模型](error-model.md) - 为什么 `Result<HttpResponse, HttpResponse>` 能让中间件干净地短路
