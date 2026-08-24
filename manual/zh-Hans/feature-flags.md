# 功能标志

Suprnova 的功能标志系统，把编译期的 `Feature` 声明，和持久化到一张 `features` 表里的运行时覆盖结合了起来。一个标志在求值时刻的值，按下面的顺序决定：

1. `features` 表里一个限定了作用域的行 - `user:42` 或者 `team:staff`。
2. `features` 表里的全局行（作用域 `""`）。
3. 内置在 `Feature` 声明里的那个编译期 `default`。

通过管理端 CRUD 做的翻转，会在这次变更调用返回之前，就传播到活跃的评估器。熔断开关标志是真的实时禁用，而不是“在下一个 TTL 窗口之内”。

## 快速上手

```rust
// app/src/features.rs - 您应用引用的每一个标志都住在这里。
use suprnova::features::Feature;

pub const NEW_CHECKOUT_FLOW: Feature<'static> = Feature::new("new-checkout-flow", false);
```

```rust
// app/src/bootstrap.rs - 在启动期间把这条链接好，只需一次。
use std::time::Duration;
use suprnova::features::{bootstrap_database_cached, FeatureMiddleware};

pub async fn register() {
    // ……DB::init、会话，等等。

    bootstrap_database_cached(Duration::from_secs(60))
        .await
        .expect("feature flags wired");

    global_middleware!(FeatureMiddleware::new());
}
```

```rust
// 任意处理程序 - Feature::is_enabled() 会针对逐请求的上下文来解析。
use crate::features::NEW_CHECKOUT_FLOW;

pub async fn index(req: Request) -> Response {
    let banner = if NEW_CHECKOUT_FLOW.is_enabled() {
        Some("Try the new checkout - faster, fewer steps.")
    } else {
        None
    };
    // ...
}
```

```rust
// 从一个管理端路由或者 CLI 里翻转这个标志：
use suprnova::features::admin;

let actor_id = Auth::id();  // Option<String> - 系统发起的变更为 None
admin::upsert("new-checkout-flow", "", true, None, actor_id).await?;
//                                  ^   ^                  ^
//                                  |   |                  └ 审计：是谁翻转了它
//                                  |   └ 启用
//                                  └ scope_key: "" = 全局，"user:42" = 限定作用域的覆盖
```

下一次 `NEW_CHECKOUT_FLOW.is_enabled()` 调用会观察到 `true` - 包括任何缓存的评估器条目，它已经在 `admin::upsert` 内部被同步地置为失效。

## 组成部分

### `Feature<'a>`

这是编译期声明，携带着标志的名字，以及一个缺席时使用的默认值。

```rust
pub const KILL_SWITCH_PAYMENTS: Feature<'static> =
    Feature::new("kill-switch.payments", true);
//                                      ^ 默认值：true（支付默认启用，直到被禁用）
```

把每一条声明都集中放在 `app/src/features.rs` 里，能给您：

- 一个单一的地方，可以在有运维人员问“存在哪些标志？”时去 grep
- 标志名字的编译期唯一性 - 调用点上的一个笔误没法通过编译
- 一个显而易见的地方，可以写一条文档注释来解释这个标志控制着什么

调用 `flag.is_enabled()`，可以针对环境上下文（由 [`FeatureMiddleware`](#featuremiddleware) 搭建）来读取；或者调用 `flag.is_enabled_in(Some(&ctx))`，传入一个具体的 [`Context`](https://docs.rs/featureflag/latest/featureflag/context/struct.Context.html)。

`feature!` 和 `is_enabled!` 这两个宏，也从 `suprnova::*` 重新导出，供那些不想导入这个常量的调用点使用：

```rust
use suprnova::is_enabled;

if is_enabled!("new-checkout-flow", false) {
    // ...
}
```

### `DatabaseEvaluator`

在启动时，以及每次 [`reload()`](#流程控制-标志传播) 时，把 `features` 表读入一份内存快照。热路径（`is_enabled`）是完全同步的 - 不会逐请求查一次数据库，评估器内部也没有 `block_on`。

查找时的解析顺序，从最具体的开始：

1. `user:{id}` - 当请求上下文携带一个 `UserIdField` 时。
2. `team:{name}` - 当上下文携带一个 `TeamField` 时。
3. `""` - 全局标志。
4. `None` - 这一行不存在，编译期默认值接管。

### `CachedEvaluator`

把 `(feature, user, team)` 的查找结果记忆化在一个 `DashMap` 背后，TTL 由您来选。热路径依然是同步的；当 [`admin::upsert`](#管理端-crud) 写入一个标志时，条目会被同步地丢弃。

TTL 为零时，会退化成“没有缓存” - 每次调用都会落到内层评估器上。这对那些标志数量不多、只想要传播管道而不想要缓存的应用很有用。

### `FeatureMiddleware`

打开一个逐请求的 featureflag 上下文，由用户定义的提取器来填充。默认值：

- `user_id` - 来自 `Auth::id()`。
- `team` - 无。

通过这个构造器覆盖其中任意一个：

```rust
let middleware = FeatureMiddleware::new()
    .with_user_id_extractor(|req| {
        // 自定义：从一个请求头里取，而不是从会话里。
        req.header("X-User-Id").map(String::from)
    })
    .with_team_from_header("X-Team");
// 或者：.with_team_extractor(|req| your_custom_team_resolver(req))

global_middleware!(middleware);
```

### 管理端 CRUD

`suprnova::features::admin` 是 `features` 表的持久化层。可以在管理端处理程序、CLI 工具、部署脚本里使用它 - 任何需要翻转一个标志的地方都行：

```rust
use suprnova::features::admin;

// 创建或更新一个全局标志。
admin::upsert("kill-switch.payments", "", false, Some("ops-2026-05-19".into()), actor_id).await?;
// 参数：name、scope_key、enabled、description、actor_id

// 限定用户作用域的覆盖（优先于全局）。
admin::upsert("new-checkout-flow", "user:42", true, None, actor_id).await?;

// 完全移除一行 - 标志会回退到编译期默认值。
admin::delete("kill-switch.payments", "", actor_id).await?;

// 供管理端 UI 表格读取。
let all_flags = admin::list().await?;
let one_row = admin::get("kill-switch.payments", "").await?;
```

每一次变更都会触发对应的[事件](#事件)，并调用 [`features::sync::notify`](#流程控制-标志传播)，这样任何绑定进 App 容器的活跃评估器，都会在这次调用返回之前刷新。

`actor_id: Option<String>` 是那个审计指针。传入运维人员的用户 id（和您认证层签发的是同一个）；对于系统发起的变更（CLI、部署迁移等等），留 `None`。

## 流程控制：标志传播

让“管理端翻转立即可见”得以运作的那个 trait：

```rust
#[async_trait]
pub trait FeatureSync: Send + Sync + 'static {
    async fn on_flag_changed(&self, feature: &str, scope_key: &str);
}
```

实现者会对变更做出反应：

- `DatabaseEvaluator::on_flag_changed` 会调用 `self.reload()` - 拉取完整的快照。
- `CachedEvaluator::on_flag_changed` 会调用 `self.invalidate(feature)` - 丢弃这个名字下的每一条缓存条目。

规范的链条是一个 `CompositeFeatureSync`，它**把数据源排在缓存之前** - 缓存必须在数据源刷新*之后*才失效，否则一个并发的读取者可能会打到空缓存上，落到那个过期的数据源上，然后用旧值把缓存重新填回去。

```rust
let composite = CompositeFeatureSync::new(
    vec![database.clone() as Arc<dyn FeatureSync>], // 数据源在前
    vec![cached.clone() as Arc<dyn FeatureSync>],   // 缓存在后
);
App::bind::<dyn FeatureSync>(composite);
```

`features::sync::notify(feature, scope_key)` 会从容器里解析出 `Arc<dyn FeatureSync>`，并等待 `on_flag_changed`。当没有绑定任何 sync 时，这是一个空操作 - 对于那些只写数据库、没有活跃评估器可刷新的进程外管理端工具来说，这正是正确的行为。

## 启动引导辅助函数

`bootstrap_database_cached(ttl)` 用一次调用就把一切接好：

```rust
let features = bootstrap_database_cached(Duration::from_secs(60))
    .await
    .expect("feature flags wired");

// 可选：留住 features.database 的句柄，用来安排周期性的重新加载，
// 或者暴露管理端的差异视图。大多数应用都会丢弃这个句柄，让
// 由通知驱动的刷新来完成这件事。
```

它做的事情：

1. 针对主数据库连接，构造出 `DatabaseEvaluator`。
2. 用请求的 TTL，把它包进 `CachedEvaluator`。
3. 调用 `install_evaluator(cached)` - 设置全局的 featureflag 默认值，*并且*翻转一个框架自有的“已安装”追踪器，这样中间件就不会记那条“无评估器”的告警日志。
4. 用正确的插槽顺序构建一个 `CompositeFeatureSync`，并把它绑定进 App 容器。

对于想要拿到任意一层直接句柄的调用方，会返回 `BootstrappedFeatures { database, cached }`。

如果您的拓扑不是 `Cached(Database)` - 一个 Redis 支撑的缓存、一个远程同步源、一条多层链条 - 请用同样的这些原语手动把链条接好。`bootstrap_database_cached` 是一个便利手段，不是一份契约。

## 迁移

框架拥有 `features` 表的架构：

```rust
// app/src/migrations/mod.rs
vec![
    // ……您应用的迁移……
    Box::new(suprnova::features::migrations::CreateFeaturesTable),
]
```

架构：

```sql
features (
    id          BIGINT      PRIMARY KEY AUTO_INCREMENT,
    name        VARCHAR(255) NOT NULL,
    scope_key   VARCHAR(255) NOT NULL DEFAULT '',
    enabled     BOOLEAN     NOT NULL,
    description TEXT,
    updated_by  VARCHAR(255),
    created_at  TIMESTAMP   NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at  TIMESTAMP   NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE INDEX (name, scope_key)
)
```

`scope_key` 会把作用域种类内联地带上（`"user:42"`、`"team:staff"`，全局则是 `""`），这样读取路径就能保持是针对一个唯一索引的单次字符串查找。

## 用户和团队 id

`UserIdField` 和 `TeamField` 是两个类型化的扩展，存放在
`featureflag::Context::extensions` 里。两者都是字符串类型，这样不透明的框架或
Magnetar 用户 ID 与数值型 `app_users.id` 值，就能共享同一种评估形状。

手动搭建一个上下文（在中间件之外）：

```rust
use featureflag::context;
use std::sync::Arc;

let ctx = featureflag::evaluator::with_default(cached.clone(), || {
    // 字符串型用户 id - UUID、ULID，任何不透明的东西。
    context! { user_id = "01HZK6V3J7Q5G4P8X9N2D1B0M3".to_string(), team = "staff".to_string() }
});

// 数值型 id 依然能用 - 框架会在 on_new_context 那一刻，把 i64 强制转换成 String。
let ctx_numeric = featureflag::evaluator::with_default(cached.clone(), || {
    context! { user_id = 42_i64 }
});
```

## 事件

有两个事件，会从管理端 CRUD 路径上触发：

```rust
pub struct FeatureUpdated {
    pub name: String,
    pub scope_key: String,
    pub enabled: bool,
    pub actor_id: Option<String>,
}

pub struct FeatureDeleted {
    pub name: String,
    pub scope_key: String,
    pub actor_id: Option<String>,
}
```

通过框架的事件分发器监听它们，把数据喂给一份审计日志、一条 Slack 告警，或者您需要的任何下游流水线：

```rust
EventFacade::listen::<FeatureUpdated, _>(Arc::new(FlagChangeAuditor)).await;
```

**`is_enabled` 不会触发一个读路径事件。** 每一个检查标志的请求，都会把事件量乘上被检查的标志数量 - 这对一份变更审计的故事来说没问题，但对读路径追踪来说是禁止性的。如果您的部署需要采样式的读路径审计，请叠加一个自定义评估器，把记录写进一个有界的日志通道（一个 Redis stream，或者一个扇出队列，取决于规模）。

## 缺失评估器检测

如果 `FeatureMiddleware` 已经安装，但没有通过 `install_evaluator` / `bootstrap_database_cached` 注册任何评估器，每一个标志都会静默地返回它的编译期默认值 - 这是一种在 QA 阶段很难捕捉的严重配置错误。中间件会在第一个观察到这个状态的请求上，为每个进程精确地发出一条 `tracing::warn!`：

```
WARN suprnova::features: FeatureMiddleware is in the stack but no feature-flag evaluator is installed.
     is_enabled!() calls will return compile-time defaults until features::bootstrap_database_cached(...)
     or features::install_evaluator(...) is called during app boot.
```

这个翻转用的是 `AtomicBool::swap`，所以启动时的一场并发请求风暴，会被串行化成一次警告发出，而不是每个工作线程发一次。

## 测试

有两种模式，取决于您在验证什么。

### 隔离地为一个 Feature 做单元测试

使用 `featureflag::evaluator::with_default`，在一个同步闭包内部，把一个替身评估器限定在作用域里：

```rust
#[test]
fn flag_enabled_returns_new_path() {
    use featureflag::evaluator::with_default;
    use suprnova::features::DatabaseEvaluator;

    let flagger = Arc::new(tokio_test::block_on(async {
        let e = DatabaseEvaluator::new_in_memory().await.unwrap();
        e.set_flag("new-checkout-flow", "", true).await.unwrap();
        e
    }));

    with_default(flagger, || {
        assert!(crate::features::NEW_CHECKOUT_FLOW.is_enabled());
    });
}
```

`DatabaseEvaluator::new_in_memory()` 是一个只用于测试的辅助函数，它会启动自己的 SQLite，并跑一次 `CreateFeaturesTable`，这样这个测试就能保持自成一体。不要在生产路径里用它。

### 端到端地做传播的集成测试

数据库用 `TestDatabase::fresh::<TestMigrator>()`，FeatureSync 用 `TestContainer::bind`（**不是** `App::bind`） - 否则同一个进程里的并行测试，会通过那个全局容器，互相覆盖对方的绑定：

```rust
#[tokio::test]
async fn admin_upsert_propagates_to_cached_chain() {
    use std::sync::Arc;
    use std::time::Duration;
    use suprnova::features::sync::FeatureSync;
    use suprnova::features::{admin, CachedEvaluator, CompositeFeatureSync, DatabaseEvaluator};
    use suprnova::features::migrations::CreateFeaturesTable;
    use suprnova::testing::{TestContainer, TestDatabase};

    struct TestMigrator;
    impl sea_orm_migration::MigratorTrait for TestMigrator {
        fn migrations() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
            vec![Box::new(CreateFeaturesTable)]
        }
    }

    let _db = TestDatabase::fresh::<TestMigrator>().await.unwrap();

    let database = Arc::new(DatabaseEvaluator::new().await.unwrap());
    let cached = Arc::new(CachedEvaluator::new(
        database.clone() as Arc<dyn featureflag::evaluator::Evaluator + Send + Sync>,
        Duration::from_secs(60),
    ));
    let composite = Arc::new(CompositeFeatureSync::new(
        vec![database.clone() as Arc<dyn FeatureSync>],
        vec![cached.clone() as Arc<dyn FeatureSync>],
    ));
    TestContainer::bind::<dyn FeatureSync>(composite);

    let ctx = featureflag::evaluator::with_default(cached.clone(), || {
        featureflag::context! { user_id = "user-42".to_string() }
    });

    assert_eq!(cached.is_enabled("new-feature", &ctx), None);
    admin::upsert("new-feature", "", true, None, None).await.unwrap();
    assert_eq!(cached.is_enabled("new-feature", &ctx), Some(true)); // 瞬时传播
}
```

完整的组合测试集合，请参见 `framework/tests/features.rs`。

### 为什么 Suprnova 有所不同

Laravel Pennant 会按需针对数据库解析每一个标志（带有可选的、驱动程序层面的逐请求记忆化）。PHP 那种一个请求一个进程的模型，让逐请求的一次数据库命中很廉价，因为这个连接是专属的，并且会随请求一起消亡。

Suprnova 的进程模型正好相反 - 一个长期存活的二进制文件，服务着成千上万个并发请求。如果每次标志检查都要打一次数据库，连接池的负载就会被标志检查的次数放大。这条两层链条（`DatabaseEvaluator` 快照 + `CachedEvaluator` TTL）就是 Rust 原生的答案：热路径针对内存数据完全同步运行，而 `FeatureSync` trait 让由运维人员发起的变更获得亚秒级的传播，不需要轮询式的重新加载。形状和 Pennant 是一样的 - 定义一个标志，在一个处理程序里检查它，从一个管理端路由覆盖它。管线不一样，是因为运行时不一样。

## 设计说明

- **为什么用同步评估器而不是异步的？** featureflag 的 `is_enabled` 是热路径。一个异步评估器要么会强迫使用 `block_on`（容易死锁），要么会迫使每一个处理程序在读取标志时都 `.await`（人体工程学上的灾难）。框架通过一份由 `FeatureSync` 异步刷新的内存快照，桥接了同步和异步。

- **为什么用一个单独的 `FeatureSync` trait，而不是扩展 `Evaluator`？** featureflag 的 `Evaluator` 归上游那个 crate 所有；我们没法给它加方法。`FeatureSync` 是一个姊妹 trait，应用会在同样这些具体类型上实现它。这个 trait 对象在 App 容器里被单独绑定，这样一个进程就能叠加多个评估器，同时依然能正确地路由通知。

- **为什么 `set_flag` 在 `DatabaseEvaluator` 上是 `pub` 的？** 为了测试方便。生产环境的写入路径是 `admin::upsert`；`set_flag` 的存在，是为了让测试能够填充标志，而不必去搭建一个 `EventFacade` 监听器。两条路径都会调用 `features::sync::notify`，所以不管走哪条，传播契约都成立。

- **为什么没有 `FeatureRetrieved` 事件？** 因为量太大。一个处理程序如果每个请求检查十个标志，就会每个请求触发十个事件 - 对一个 1k req/s 的服务来说，那就是每小时 3600 万个事件，远远超出任何审计流水线的信噪比。真正发布出去的是变更路径的审计（`FeatureUpdated` / `FeatureDeleted`）；如果需要读路径的采样，可以通过一个自定义评估器包装层，叠加在上面。

## 下一步

- [中间件](middleware.md) - `FeatureMiddleware` 要放在 `SessionMiddleware` 之后；这一章讲的是顺序和全局栈
- [事件](events.md) - 监听 `FeatureUpdated` / `FeatureDeleted`，以驱动审计日志、Slack 告警，或者下游流水线
- [服务容器](container.md) - `dyn FeatureSync` 绑定是如何被解析的，以及为什么 `TestContainer::bind` 是为并行测试而存在的
- [测试](testing.md) - 这一章依赖的 `TestDatabase::fresh::<M>()` 和 `TestContainer::fake` 模式
- [认证](authentication.md) - `Auth::id()` 是默认的用户 id 提取器，为管理端变更提供 `actor_id`

外部资料：[featureflag crate 文档](https://docs.rs/featureflag) 覆盖了上游的 `Evaluator`、`Context` 和 `Feature` 这些原语。`suprnova::features::admin` 是完整的 CRUD 门面 - 用 `cargo doc --open -p suprnova` 来浏览。
