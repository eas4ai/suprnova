# 服务容器

容器是 Suprnova 存放您应用服务的地方 - 数据库连接池、邮件驱动程序、您的 `Arc<MyService>`。您在启动时把值绑定到其中，并在处理程序和工作进程里解析它们。它是 Suprnova 对应 Laravel 服务容器的等价物，但有一个重要的区别：查找首先是任务本地的，因此并发运行的测试不会看到彼此的绑定。

## 两个部分

| 类型 | 角色 |
|---|---|
| `Container` | 底层注册表：持有绑定、工厂和单例 |
| `App` | 您实际调用的全局门面 - `App::bind`、`App::get` 等 |

您几乎总是调用 `App::*`，而不是直接构造一个 `Container`。容器是内部底层设施；`App` 门面才是 API。

## 查找顺序

每一次 `App::get` / `App::make` 调用都会按顺序检查**三层**：

```
         任务本地
            │
            ▼  （未命中）
         线程本地
            │
            ▼  （未命中）
           全局
            │
            ▼  （未命中）
          None
```

这很重要，因为：

- **逐请求状态通过任务本地实现** - Inertia 共享数据、flash bag、request id。每个请求透明地获得自己的一层。
- **测试使用线程本地** - `let _g = TestContainer::fake();` 之后跟着 `TestContainer::bind(...)`，会在一个线程内部绑定，而不触碰全局容器，因此并行测试不会把服务泄漏给彼此。该守卫在被释放时会清空测试容器。
- **应用级服务通过全局实现** - 在启动时绑定一次，在任何地方都能解析。

您几乎不需要考虑一个绑定存在于哪一层 - `App::bind` 会把它放在合适的地方，`App::get` 无论它存在于哪里都能找到它。只有当某些东西在并发下表现异常时，这个模型才会变得重要，那时[测试](testing.md)一章里有详细说明。

## 绑定一个值

根据您拥有的东西，有五种方式可以把某样东西放进容器：

### `App::singleton(value)` - 拥有所有权，在查找时克隆

对于任何应该永远存活的 `T: Any + Send + Sync + 'static` 值。`Clone` 约束是加在*获取方法*（`App::get`）上的，而不是绑定上 - 值只被存储一次，放在一个 `Arc` 里，每次 `get` 时都从那个 `Arc` 中克隆出来：

```rust
use suprnova::App;

App::singleton(MyConfig {
    timeout_secs: 30,
    retries: 3,
});

let cfg = App::get::<MyConfig>().expect("registered at boot");
println!("{}", cfg.timeout_secs);
```

值只被存储一次；`App::get::<MyConfig>()` 返回一个克隆。把它用于那些克隆代价很低、类似配置的普通数据。

### `App::bind(Arc<T>)` - 用于 trait 和共享服务

对于 trait 对象，或任何您想放在 `Arc` 后面的东西：

```rust
use std::sync::Arc;
use suprnova::App;

let store: Arc<dyn KeyValueStore> = Arc::new(RedisStore::connect(url)?);
App::bind(store);

let store = App::make::<dyn KeyValueStore>().expect("bound at boot");
store.put("hello", b"world").await?;
```

`App::make::<T>()` 返回 `Arc<T>` 的克隆（代价低廉的原子引用计数递增）。把它用于任何跨线程共享的服务，尤其是 trait 对象。

### `App::factory(|| { … })` - 按需构建

当构造这个值这件事应该发生在第一次使用时（或者每一次使用时）：

```rust
App::factory(|| {
    HttpClient::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("http client config is hand-rolled and known-good")
});
```

`App::factory` 注册一个*具体类型*的工厂（`Fn() -> T`）；`App::bind_factory` 注册一个 *trait 对象*的工厂（`Fn() -> Arc<T>`）。两个闭包都不返回 `Result` - 要么在闭包内部处理构造失败（在启动时 panic，或者构建一个哨兵值），要么在自己用 `?` 构造出值之后，使用普通的 `App::singleton` / `App::bind`。两者都在任何容器锁之外调用闭包，所以一个重新进入容器的工厂不会死锁，一个昂贵的构造函数也不会阻塞其他绑定。

### `App::*_if_absent(value)` - 对启动顺序友好的注册方式

有时一个默认服务是由某个服务 crate 注册的，而应用只想在该服务存在时才覆盖它。`_if_absent` 变体让您可以注册一个不会覆盖已有绑定的默认值：

```rust
// 在起步套件或库 crate 内部：
App::singleton_if_absent(DefaultMailDriver::new());

// 在您应用的 bootstrap.rs 中：
App::singleton(MyCustomMailDriver::new());  // 因为后运行而胜出
```

`bind_if_absent`、`singleton_if_absent` 以及工厂变体都返回 `bool` - 如果确实插入了就是 `true`，如果已经存在绑定就是 `false`。

## 解析一个值

两个读取方法，加上它们返回 `Result` 的对应版本：

```rust
// 克隆出已绑定的值：
let cfg: MyConfig = App::get::<MyConfig>().expect("bound at boot");

// 克隆 Arc：
let store: Arc<dyn KeyValueStore> = App::make().expect("bound at boot");

// 同样的效果，但返回 Result，用于可失败路径中的 `?` 惯用法：
let cfg = App::resolve::<MyConfig>()?;
let store = App::resolve_make::<dyn KeyValueStore>()?;
```

`resolve` 和 `resolve_make` 返回 `Result<_, FrameworkError>`（具体来说，当查找未命中时是 `ServiceNotFound` 变体）- 在处理程序路径中很有用，那里缺失的服务应该表现为带有恰当日志的 500，而不是一次 panic。

成员检查（很少需要）：

```rust
if App::has::<MyConfig>() { … }
if App::has_binding::<dyn KeyValueStore>() { … }
```

## 绑定发生的地方

标准的位置是 `src/bootstrap.rs` - 一个在启动时运行一次的函数：

```rust
use std::sync::Arc;
use suprnova::App;
use crate::services::{MyService, RealEmailGateway};

pub async fn register() {
    // 普通单例
    App::singleton(MyAppConfig {
        max_uploads_per_user: 100,
    });

    // trait 对象服务
    let gateway: Arc<dyn EmailGateway> = Arc::new(RealEmailGateway::new());
    App::bind(gateway);

    // 惰性服务（首次使用时构建）
    App::bind_factory::<dyn HttpClient, _>(|| {
        Arc::new(ReqwestClient::with_timeout(30))
    });
}
```

函数名 `register` 与脚手架的默认值（`src/bootstrap.rs::register`）相匹配；返回类型是 `()`，而不是 `Result`。启动期间发生的绑定错误（例如驱动程序连接失败）应该通过驱动程序/服务的构造函数传播，而不是从 `register` 本身传播 - 完整的启动装配请参见[应用启动](bootstrap.md)。

框架自己也会在启动期间调用进容器：

- `App::init()` 首先运行，初始化注册表
- `App::boot_services()` 解析启动期依赖（驱动程序、加密密钥等）- 让您的服务能看到一个已完全启动的框架
- 您的 `bootstrap_fn` 在那之后运行，因此它可以依赖框架的服务已经可用这一点

完整的启动顺序请参见[应用启动](bootstrap.md)。

## Inertia 共享数据

容器也是 Inertia 共享数据存放的地方。三个便捷 API 让这一点变得明确：

```rust
use suprnova::App;

// 即时值 - 只序列化一次，被每一次 Inertia 响应复用。
App::inertia_share("appName", "Suprnova");

// 惰性值 - 解析器逐次响应运行。用于需要异步工作的
// 逐请求数据。
App::inertia_share_lazy("locale", || async {
    Ok::<_, suprnova::FrameworkError>(detect_locale().await)
});

// 向逐请求的 flash bag 推送一条单独的 flash 记录。
App::flash("message", "Saved!");
```

这些读取自 `Container::inertia()`，它返回 `&Arc<InertiaRegistry>` - 如果您需要更底层的访问，可以直接与它交互。共享数据如何最终进入页面响应，请参见 [Inertia / 前端](frontend.md)。

## 为什么是三层？

任务本地 → 线程本地 → 全局这个级联的存在只有一个原因：**并发下的隔离**。三件事因此受益：

**逐请求隔离。** Inertia 的 flash bag 通过任务本地层逐请求绑定。两个并发请求不会看到彼此的 flash，因为它们的任务本地容器不会重叠。当请求的任务结束时，绑定就会蒸发。

**逐测试隔离。** 一个绑定了假邮件驱动程序的测试不应该看到某个兄弟测试绑定的假驱动程序。`TestContainer::fake()` 返回一个线程本地守卫，`TestContainer::bind` / `TestContainer::singleton` 会把写入路由到当前激活的作用域中。并行测试保持相互隔离：

```rust
use std::sync::Arc;
use suprnova::container::testing::TestContainer;
use suprnova::suprnova_test;

#[suprnova_test]
async fn one_test_binds_a_fake() {
    let _guard = TestContainer::fake();
    TestContainer::bind::<dyn Mailer>(Arc::new(FakeMailer::new()));

    // ……这个测试使用 FakeMailer
    // 并行运行的兄弟测试看不到它
}
```

对于多线程 tokio 运行时 - 那里 future 可能会在工作线程之间迁移 - 请改用 `TestContainer::scope(async { ... })`；它会安装一个能在迁移中存活的任务本地覆盖。

**启动期覆盖。** 应用代码可以覆盖库 crate 注册的默认值。`_if_absent` 变体和分层查找结合在一起，让库 crate 可以拥有干净的默认注册方式，而不必与应用层的覆盖相冲突。

## 常见模式

### 绑定一个持有数据库连接池的结构体

您几乎不会直接这样做 - 框架自己会绑定数据库连接池。但如果您有自己的子系统，持有一个昂贵的共享资源：

```rust
let pool = MyResourcePool::connect(url).await?;
App::bind(Arc::new(pool));

// 之后：
let pool = App::resolve_make::<MyResourcePool>()?;
let conn = pool.checkout().await?;
```

`App::make` 返回 `Option<Arc<T>>`，与 `.expect(...)` 搭配使用；`App::resolve_make` 返回 `Result<Arc<T>, FrameworkError::ServiceNotFound>`，在可失败代码中与 `?` 搭配使用。使用与您调用方错误处理方案相匹配的那一个。

### 在测试中把一个默认值换成伪造实现

```rust
use std::sync::Arc;
use suprnova::container::testing::TestContainer;
use suprnova::suprnova_test;

#[suprnova_test]
async fn order_dispatches_email() {
    let fake = Arc::new(FakeEmailGateway::new());
    let fake_for_assert = Arc::clone(&fake);

    let _guard = TestContainer::fake();
    TestContainer::bind::<dyn EmailGateway>(fake);

    place_order(123).await.expect("place_order succeeds");

    assert_eq!(fake_for_assert.sent_count(), 1);
}
```

### 惰性的昂贵构造

```rust
// 在第一次请求时构建嵌入模型，而不是在启动时。
App::bind_factory::<dyn EmbeddingModel, _>(|| {
    Arc::new(
        OnnxEmbedding::load_from_disk("/models/all-mini-lm.onnx")
            .expect("embedding model must load"),
    )
});
```

对于需要向运维人员暴露结构化错误的可失败构造，请自己在 `bootstrap()` 中用 `?` 构建出这个值，一旦它准备好了，再调用 `App::bind(...)`。

## 为什么 Suprnova 有所不同

Laravel 的容器只有一个全局作用域 - 绑定是全局的，测试之间的隔离需要 `setUp` / `tearDown` 的纪律，加上框架的逐测试数据库事务。PHP 的每进程一个请求模型让这一点意外地变得安全：每个请求都是一个全新的进程，意味着容器每次都会被重置。

Rust 的进程模型正好相反 - 一个进程在许多线程上服务许多并发请求。一个只有全局作用域的容器会意味着一个线程里的测试可能看到另一个线程绑定的伪造实现，或者一个请求可能看到另一个请求的逐请求数据。这就是为什么 Suprnova 有这个三层级联：任务本地用于逐请求，线程本地用于逐测试，全局用于应用级。

容器的 API 和 Laravel 的一样；查找机制不同，是因为运行时不同。

## 下一步

- [应用启动](bootstrap.md) - 绑定代码放在哪里
- [配置](configuration.md) - 与服务并列的类型化配置注册
- [测试](testing.md) - `TestContainer::fake` 和 `#[suprnova_test]`
- [锁策略](lock-policy.md) - 为什么被污染锁的恢复在一个基于容器的应用中很重要
