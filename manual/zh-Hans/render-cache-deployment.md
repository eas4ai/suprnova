# RenderCache 部署

一个只为自己做缓存的进程，除了内存什么都不需要。而负载均衡器后面的好几个进程，
需要就“存下来的是什么”“谁被允许重建一个条目”“什么时候某样东西不再为真”达成
一致 - 而且它们必须在谁都无法说服其他人相信“陈旧内容是最新的”的前提下达成一致。
RenderCache 用**配置档**来回答这件事：一个配置档指名一个进程要构建哪些提供者，
除此之外什么都不变。路由声明、策略、键、收集器、中间件流程和缝合，在每一个配置
档下都完全相同，也没有任何面向应用的类型在它们之间有差异。

本章讲的是你怎么挑一个并把它接好。三个配置档各提供什么、框架配置实际读取哪些
环境变量、一个共享配置档要能启动、你的应用必须列出哪个迁移、安装该放在你的启动
流程的什么位置，以及这些层级承诺什么、不承诺什么。本仓库的自用（dogfood）应用
默认跑 embedded 配置档，并由 `app/tests/live_render_cache.rs` 里的
`the_database_profile_serves_a_hit_through_the_sql_stores` 在 Database 配置档上
启动。

## 三个配置档

| 配置档 | L1 条目 | 重建领导权 | Live 实例记录 |
|---|---|---|---|
| `embedded`（默认） | 每个键一个文件，或者一个都没有 | 在进程内 | 在进程内 |
| `database` | `suprnova_render_entries` | `suprnova_render_leases` | `suprnova_live_instances`、`suprnova_live_promotions` |
| `redis` | 每个键一个 Redis 哈希 | 每个键一个 Redis 哈希，外加一个令牌计数器 | 每条记录一个 Redis 哈希 |

在它们之间**不会**移动的东西，是世代真相。数据库支撑的世代账本在每一个配置档下
都是权威：无论是哪一个层级交出的字节，时效性都是对着数据库证明的 - 在
`CoherenceMode::Authority` 下于命中时重读它，或者在 `CoherenceMode::Lease` 下
凭一份由更早一次读取授予的校验租约。这正是让加速器只是加速器的原因：Redis 可以
丢掉它持有的一切，而不会有任何陈旧的东西被证明为最新，因为 Redis 持有的东西
本来就不证明时效性。

按你实际需要共享什么来选：

- **`embedded`** 适用于单个进程，也适用于乐意各自留一份副本的好几个进程。设置
  `RENDER_CACHE_L1_DIR`，每个进程就多出一个能挺过自己重启的文件层级。
- **`database`** 适用于好几个节点应当共享已存储的条目、并为每个键选出一个重建
  领导者，而你又宁可不给部署再加一个活动部件的情况。
- **`redis`** 适用于共享层级的延迟比它的持久性更要紧的情况，而世代真相仍然由
  下面的数据库持有。

## 那些环境变量

`RenderCacheConfig::from_env` 在 `framework/src/render_cache/config.rs` 里读取
这些：

| 变量 | 默认值 | 含义 |
|---|---|---|
| `RENDER_CACHE_ENABLED` | `true` | 除 `false` 或 `0` 之外的任何值都算开；`false` 会让 `RenderCache::install` 成为一次空操作 |
| `RENDER_CACHE_PROFILE` | `embedded` | `embedded`、`database` 或 `redis`；它设定下面两行 |
| `RENDER_CACHE_L1` | 由配置档决定 | `disabled`、`file`、`database` 或 `redis` |
| `RENDER_CACHE_COORDINATOR` | 由配置档决定 | `local`、`database` 或 `redis` |
| `RENDER_CACHE_L0_ENTRIES` | 4,096 | 进程内条目数上限 |
| `RENDER_CACHE_L0_BYTES` | 128 MiB | 进程内字节数上限 |
| `RENDER_CACHE_L1_DIR` | 未设置 | 文件层级的目录；在 `embedded` 下，设置它就是打开 L1 的开关 |
| `RENDER_CACHE_L1_BYTES` | 1 GiB | 对文件层级是整个目录，对数据库和 Redis 层级是单个条目 |
| `RENDER_CACHE_REDIS_URL` | `REDIS_URL`，然后是 `redis://127.0.0.1:6379` | 两个 Redis 缓存层级连到哪里 |
| `RENDER_CACHE_REDIS_PREFIX` | `suprnova_render:` | 两个 Redis 缓存层级写入时所用的键命名空间 |
| `RENDER_CACHE_LEASE_MS` | 30,000 | 重建租约的存活时长 |
| `RENDER_CACHE_MAX_WAITERS` | 128 | 进程内等待者上限 |
| `RENDER_CACHE_FAILURE` | `open` | `open` 在提供者出故障时以不缓存的方式服务该路由，`closed` 应答 `503` |
| `APP_BUILD_ID` | 应用自己的包版本（见下文） | 把每一个条目限定在生成它的那次构建的命名空间下 |

配置档是一种简写，不是一把锁。`RENDER_CACHE_L1` 和 `RENDER_CACHE_COORDINATOR`
各自覆盖自己的那一半，所以一个想把条目放在数据库、却把重建租约留在进程内的部署
可以就这么说，而不必去挑一个最接近的完整配置档。

一个取值集合封闭的变量，如果被设成集合之外的东西，会让启动失败，并给出一条点名
该变量的消息。被拒绝的那个值绝不会在那条消息里被复述，因为一个环境变量的值可能
携带秘密。

**请显式设置 `APP_BUILD_ID`，每次部署设一次。** 它被混进每一个查找键，所以改动
它正是阻止一个新构建去服务上一个构建发布的条目的手段。缺了这个变量的时候，
`RenderCacheConfig::from_env` 会回退到你应用自己的包版本：`#[suprnova::main]`
会在加载环境的那一刻，从应用 crate 自身的编译中记录下 `CARGO_PKG_VERSION`，
而这里的默认值回退到的正是这个被记录下来的值。只有一个从未展开过
`#[suprnova::main]` 的二进制文件，才会再往下回退一层，落到这个**框架** crate
自身的版本上 - 之所以这样点名，是因为不然就很容易把它误认成应用自己的版本。
无论哪种情况，这个值都只在有人抬一次版本号时才会动，而一个包版本很少会随着
每次部署改变：一次改了模板、翻译或处理程序、却没有抬版本号的部署，会保持同
一个构建 id，因而可能服务上一个构建发布的条目。请把它设成每次发布都会变的
东西 - 一个提交 id 或者一个发布标识：

```bash
APP_BUILD_ID=$(git rev-parse --short HEAD)
```

一个从不读取环境的安装，会用 `RenderCacheConfig::with_build_id` 在代码里设置
同一个值 - 它会覆盖 `from_env` 本来选中的东西，包括一个显式的 `APP_BUILD_ID`
在内 - 供一个以编程方式自行推导每次部署标识的应用使用。

无论你设置的是哪个值，读取它的那个生产二进制文件，都被期望是在 Suprnova 的
[生产构建形态](deployment.md#production-build-shape)下构建出来的：默认特性
全部关闭，`testing` 只留给 `cargo test`。

Live 自己的实例账本是单独配置的，因为它是 Live 的权威，而不是缓存的存储：
`LIVE_LEDGER_DRIVER`（`memory`、`database` 或 `redis`）、`LIVE_REDIS_URL` 和
`LIVE_REDIS_PREFIX`。一个部署可以让缓存跑在一个层级上，而让账本跑在另一个上。

## 你的应用必须列出的那个迁移

RenderCache 的 schema 归框架所有、由应用来施加。你的 `Migrator` 把它列出来，
这样 `suprnova migrate` 就会把这些表和你自己的表一起建好：

```rust
Box::new(suprnova::render_cache::migration::Migration),
Box::new(suprnova::render_cache::migration::TierMigration),
```

那是 `app/src/migrations/mod.rs` 的原文，而这两者不可互换：

- **`Migration`** 创建那三张持有持久世代真相的 `suprnova_render_*` 表：当前
  世代、一份只追加的变更日志，以及权威纪元。每一个配置档都需要它，`embedded`
  也不例外，因为世代真相永远不会搬进某个缓存层级。
- **`TierMigration`** 创建数据库 L1 存储和数据库重建协调器要读的那四张表。只有
  会用到它们的配置档才需要它 - 但 `RenderCache::install` 在没有这些表时会拒绝
  以 Database 配置档启动，所以列出它，才使得 `RENDER_CACHE_PROFILE=database`
  成为你的应用真正可以做出的一个配置选择。

一个设置了 `RENDER_CACHE_ENABLED=false` 的应用两者都不必带：安装会原样返回
路由器，什么都不探测，不组装任何运行时，不注册任何中间件，也让写入侧不带埋点，
所以没有人为一个关着的缓存付账。

## 安装它

`RenderCache::install` 是异步的，因为它在组装任何东西之前，会探测那些表，并对
这份配置会用到的每一个不同的 Redis 端点各 ping 一次。
`Application::try_routes_async` 就是承载它的钩子。下面是 `app/src/live/mod.rs`，
而且拆成两个函数这一点值得照抄：

```rust
/// [`routes`] followed by the RenderCache middleware. This is the entry
/// point every server in this application uses.
pub async fn routes_with_render_cache(router: Router) -> Result<Router, FrameworkError> {
    routes_with_render_cache_with_config(router, RenderCacheConfig::from_env()?).await
}

#[doc(hidden)]
pub async fn routes_with_render_cache_with_config(
    router: Router,
    config: RenderCacheConfig,
) -> Result<Router, FrameworkError> {
    RenderCache::install(routes(router)?, config).await
}
```

`routes` 是同步的那内层一半：它注册保留的 Live 路由、文档路由和每一条缓存策略，
并且不安装任何中间件。`cmd/main.rs` 通过 `Application::try_routes_async` 够到
`routes_with_render_cache`，而 `app/examples/live_dogfood_host.rs` 里浏览器场景
的服务器则直接 await 它。

它下面那道配置接缝不是装饰。一个需要不同配置档、或者需要一个自己能拨动的时钟的
测试，没有别的入口；而且要紧的是，它装的是与服务器*相同*的路由、策略和中间件
顺序，唯一的差别是它传入的那份配置。两次自用启动走的都是它：
`the_database_profile_serves_a_hit_through_the_sql_stores` 传入一份 Database
配置档的配置，而 `stale_service_is_marked_and_rebuilt_in_the_background` 传入
一份带可调时钟的配置。参见
[RenderCache 运维](render-cache-operations.md)中的“测试一个被缓存的路由”。

有两条顺序规则，都归调用方负责：

1. 每一个路由和分组都必须在 `install` **之前**被接入，因为它只读取到那个时间点
   为止已经注册的内容。
2. `install` 是往全局中间件链上追加的，所以它必须运行在会话、语言环境和身份
   中间件**之后** - 缓存中间件在派生查找键时读的正是它们的请求范围状态。

安装是失败即关闭的，靠两次探测。它检查这份配置会用到的那些表是否存在，并对这份
配置会用到的每一个不同的 Redis 端点各 ping 一次。任何一项失败都会让启动停下，
并给出一句可操作的话，点名要修的迁移或变量，这样就不会有任何东西被拿到一张缺失
的表、或者一个没人应答的端点上去服务。（Live 自己的实例账本是单独探测的，由
`Server::run` 在任何请求被服务之前完成。）

## 挑选一个路由的条目住在哪里

配置档决定 L1 *是什么*；策略决定哪些路由用它。除非一个路由另行声明，否则构建器
只存入 L0，所以共享层级是被有意填充的：

```rust
RenderCachePolicy::builder(RepresentationClass::PublicShared)
    .freshness(FreshnessPolicy::new(300_000, 60_000, 300_000)?)
    .layers(StorageLayers::l0_and_l1())
    .build()?
```

`the_database_profile_serves_a_hit_through_the_sql_stores` 在 Database 配置档上
端到端地证明了这趟往返：用中间件派生出的那个键，把已发布的条目从 SQL 存储里读
回来，然后清空 L0，而下一个请求依然不经渲染就被应答。关于如何按路由决定，参见
[RenderCache 表示](render-cache-representations.md)。

## 一套符合性套件，覆盖每一个提供者

每一个存储都要应答同一套套件。
`framework/tests/render_cache/store_conformance.rs` 把引擎的提供者场景 - 它们
只针对 `RenderStore` trait 本身来写 - 跑在文件支撑的 L1、SQLite/PostgreSQL/MySQL
上的 SQL L1，以及 Redis L1 上，而进程内存储在引擎 crate 里应答同一套套件。
PostgreSQL、MySQL 和 Redis 通过被 ignore 的测试来跑，由
`scripts/check-postgres.sh`、`scripts/check-mysql.sh` 和
`scripts/check-redis.sh` 按名字挑出来，对着真实的服务器执行。一个提供者不会因为
它存在就在这里算作“受支持”；它受支持，是因为它通过了与其他每一个提供者相同的
那套说法。

## 这些层级承诺什么，又不承诺什么

- **没有跨节点等待。** 一个正在被另一个节点重建的键就是一次绕过：这个节点去
  渲染，什么也不发布。跨节点上有界的重复计算是可以接受的；两次被接受的发布则
  不行，而禁止第二次的正是存储自己的发布栅栏。
- **一个丢失的后端是一次未命中，绝不是一个错误的答案。** 驱逐、过期或者一次
  Redis 重启，会让条目未命中、让实例缺失。无论字节是谁交出来的，针对数据库世代
  账本的一致性检查在每一次命中上都会运行。
- **被篡改的字节是一次未命中。** 一行或一个哈希里的条目字节是一个已签名的编解码
  帧，所以一个撕裂、被截断或被改动过的值会通不过它的完整性检查，会被当作未命中
  而不是被服务。
- **数据库层级和 Redis 层级不会为了腾地方而驱逐。**
  `RENDER_CACHE_L1_BYTES` 在那里约束的是单个条目，而不是那张表或那片键空间；
  增长是靠保留期来限制的。只有文件层级会约束整个目录，因为那个目录只归它一个。
- **Redis 适配器面向的是单个 Redis 7 或更新的实例。** Redis Cluster 会被拒绝：
  这些脚本会碰它们没有声明的键，而且它们在脚本内部用 `TIME` 读取存储时钟。
- **MySQL 需要 8.0.19 或更新**，才能精确地对重复键做分类。更老的 MySQL 和
  MariaDB 报出的冲突，这个构建无法归因到某一张表，所以它会降级为一个“提供者
  不可用”错误 - 这是安全的方向，而且无论哪种情况都不会有东西被授予两次。
- **每一个跨节点的过期判定，都是在后端的时钟上做出的**，而且是在作用于它的那次
  操作内部读取的。一个时钟走得快的节点，既不能延长一份租约，也不能把一个存活的
  条目对同伴藏起来，更不能宣告一个同伴的记录已经到期。

### 为什么 Suprnova 有所不同

Laravel 的响应缓存包继承你已经配好的那个缓存存储，所以“把缓存部署到好几个节点
上”意思就是把 `CACHE_STORE` 指向 Redis，然后相信里面的东西是对的。那里没有单独
的“谁可以重建一个条目”的概念，没有能阻止两个工作进程为同一个键发布相互冲突字节
的栅栏，而且 - 后果最重的是 - 存储底下没有权威。如果 Redis 里有一个页面，这个
页面就会被服务；如果 Redis 被刷掉了，一切就重新计算。存储*就是*真相。

Suprnova 刻意把这两者分开。共享层级只持有字节，别的什么都没有；数据库在每一个
配置档下都持有世代真相，而一次命中要对照的正是它。这就是为什么在这里丢掉 Redis
付出的是延迟而不是正确性，为什么一次重建是被租出去的、一次发布是被栅栏挡住的，
而不是靠抢；也是为什么同样的路由声明可以从笔记本上的 `cargo run` 一路不加改动
地跑到一支由数据库协调的机群上。代价是你的应用必须列出一个迁移，以及命中路径上
一次普通键值缓存不必付的数据库读取 - 一条语句，或者在一份校验租约下是零条。
这次读取买到了什么，参见
[RenderCache 世代](render-cache-generations.md)。

## 下一步

- [RenderCache 运维](render-cache-operations.md) - 那些控制台命令、遥测、磁盘
  卫生，以及出问题时该怎么办
- [部署](deployment.md) - 周边的生产环境检查清单
- [迁移](migrations.md) - 上面那份 `Migrator` 列表是如何被施加的
