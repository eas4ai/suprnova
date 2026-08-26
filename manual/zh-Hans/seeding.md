# 数据填充

填充器会用夹具数据来填充数据库 - 也就是在任何真实用户做过任何事之前，您的应用就需要的那些行。想想一个默认的管理员账户、一份规范的国家列表、预发布环境上的演示文章，还有您本地开发迭代循环所依赖的那 50 个用户加 200 篇文章。它们是[迁移](migrations.md)在运行时的姊妹篇：迁移构建出空的架构，填充器把它填满。

一个填充器是一个实现了 `Seeder` trait 的零大小类型。框架维护着一个有序的、进程全局的注册表；逐项目的 `console db:seed` 命令会按注册顺序运行每一个已注册的填充器，或者通过 `--class=<Name>` 运行某一个指定的填充器。大多数填充器最终都只是几行代码，调用一个[模型工厂](eloquent.md)，让工厂去做生成行的工作。

```rust
use suprnova::{async_trait, Factory, FrameworkError, Seeder};
use crate::factories::UserFactory;

pub struct UsersSeeder;

#[async_trait]
impl Seeder for UsersSeeder {
    fn name() -> &'static str { "UsersSeeder" }

    async fn run() -> Result<(), FrameworkError> {
        UserFactory::new().count(50).create_many().await?;
        Ok(())
    }
}
```

在启动时注册它一次：

```rust
// src/bootstrap.rs
suprnova::seed::register::<crate::seeders::UsersSeeder>();
```

然后：

```bash
cargo run --bin console -- db:seed
# running seeder UsersSeeder
# (50 rows inserted)
```

这就是整个循环了。这一章剩下的部分，讲的是布局约定、更大的注册表组合模式、`--class` 定向标志、工厂集成、`without_events` 脱围机制，以及何时该用填充、何时该用迁移、何时该用工厂这个判断。

## 编写一个填充器

一个填充器就是一个单元类型加上一个 `Seeder` 实现。`name()` 是注册表的键（也是 `db:seed --class=<Name>` 用来匹配的东西），`run()` 是执行插入操作的那个 async fn。

```rust
// src/seeders/users_seeder.rs
use suprnova::{async_trait, Factory, FrameworkError, Seeder};

use crate::factories::UserFactory;

pub struct UsersSeeder;

#[async_trait]
impl Seeder for UsersSeeder {
    fn name() -> &'static str { "UsersSeeder" }

    async fn run() -> Result<(), FrameworkError> {
        UserFactory::new().count(50).create_many().await?;
        Ok(())
    }
}
```

`Seeder` 在 crate 根部被重新导出，所以 `use suprnova::Seeder` 就够了 - 您不需要再去 `suprnova::seed::Seeder` 里找它。`async_trait` 也被重新导出了（`use suprnova::async_trait`），因为这个 trait 方法返回一个 future，而 Rust 目前还不允许在 trait 里不借助它就写 `async fn`。

`FrameworkError` 这个返回类型，和框架里其他每一个异步表面用的是同一个错误信封；从一次工厂调用或者一次 `Model::create` 里把 `?` 冒泡出去，就是预期的形态。完整的分类请参见[错误模型](error-model.md)。

### 布局约定

镜照 Laravel 的 `database/seeders/` 目录，但放在源码根部：

```
src/
├── bootstrap.rs
├── factories/
│   ├── mod.rs
│   ├── user_factory.rs
│   └── post_factory.rs
├── seeders/
│   ├── mod.rs              // pub mod base_seeder; pub use base_seeder::BaseSeeder;
│   └── base_seeder.rs      // Seeder 实现，在 bootstrap.rs 里注册
└── …
```

手工生成这个文件 - 没有 `make:seeder` 生成器（这是一个只有十来行样板代码的文件）。这个填充器调用的那些工厂，也是同样的待遇。

### 一个运行其他填充器的填充器

Laravel 那个用单一顶层 `DatabaseSeeder::run` 去编排逐模型填充的惯用法，在这里也能用。与其在 bootstrap 里注册五个小填充器、指望它们的注册顺序，不如注册一个复合填充器，自己去调用剩下的那些：

```rust
use suprnova::{async_trait, Factory, FrameworkError, Seeder};

use crate::factories::{PostFactory, UserFactory};

pub struct BaseSeeder;

#[async_trait]
impl Seeder for BaseSeeder {
    fn name() -> &'static str { "BaseSeeder" }

    async fn run() -> Result<(), FrameworkError> {
        // 先来 50 个用户 - post 工厂生成的 author_id 落在 1..=50 里，
        // 这样这些引用才能解析成功。
        UserFactory::new().count(50).create_many().await?;

        // 200 篇引用上面那些用户 id 的文章。
        PostFactory::new().count(200).create_many().await?;

        Ok(())
    }
}
```

这是推荐的默认做法。它把依赖顺序（`users` 在 `posts` 之前）留在填充器内部，而不是散落在 bootstrap 文件各处，`db:seed --class=BaseSeeder` 就是一次单目标调用，能运行整捆填充。

如果您想按名字把填充器链起来，而不是直接调用工厂，就在这个复合填充器内部使用 `seed::run_one`：

```rust
async fn run() -> Result<(), FrameworkError> {
    suprnova::seed::run_one("UsersSeeder").await?;
    suprnova::seed::run_one("PostsSeeder").await?;
    suprnova::seed::run_one("CommentsSeeder").await?;
    Ok(())
}
```

这些子填充器仍然需要在 `bootstrap.rs` 里注册，`run_one` 才能找到它们。

## 填充器注册表

框架维护着一个进程全局的有序映射（`IndexMap<String, fn() -> _>`），记录着每一个已注册的填充器。三个旋钮控制着它。

### `register::<S>()`

把一个填充器按它的 `Seeder::name()` 加进这个注册表：

```rust
suprnova::seed::register::<crate::seeders::BaseSeeder>();
```

关于这个注册表，有两件事要知道：

- **顺序有影响。** `run_all` 会按填充器被注册的顺序去访问它们。如果 `B` 需要 `A` 产出的行，就先注册 `A`。
- **用同一个名字重新注册，会就地替换。** 这个槽位保留它原来的位置，函数指针会变。这是有意为之的 - 它让一个测试能在真正的填充器上面绑一个替身填充器，而不打乱顺序。在生产代码里，每个填充器在启动时都恰好注册一次。

### `run_all()`

按注册顺序运行每一个已注册的填充器。这就是裸的 `console db:seed` 调用所调用的东西。

```rust
suprnova::seed::run_all().await?;
```

在第一个错误上止步。已经跑过的那些填充器不会被回滚 - `run_all` 不会把这一批包进一个事务，因为大多数填充器跨越多条语句，而很多后端在嵌套事务上处理得并不干净。如果您需要回滚语义，就在填充器内部开启这个事务，把它全部的工作都留在那个作用域里。

### `run_one(name)`

只运行一个具名的填充器，不运行其他的。这就是 `db:seed --class=<Name>` 背后的引擎，在一次性脚本里也很好用：

```rust
suprnova::seed::run_one("AdminAccountSeeder").await?;
```

查不到时会返回 `FrameworkError::not_found("no seeder registered for \`X\`")`。这个控制台命令会把它传播成一个非零的退出码和一行 stderr 输出 - 不会悄无声息地什么都不做。

### `count()` 和 `is_registered(name)`

两个只读的助手方法，在断言“bootstrap 接好了预期的那些填充器”的测试里都很有用：

```rust
assert_eq!(suprnova::seed::count(), 3);
assert!(suprnova::seed::is_registered("BaseSeeder"));
```

在这个注册表的锁被污染时，两者都会返回零 / false（在记录一条错误日志之后），这让测试在面对上游的一次 panic 时，仍然保持确定性。

## `db:seed` 命令

`db:seed` 是一个框架自带的控制台命令 - 它随框架发布，并通过那个同样会捡起您自己 `#[command]` 的 `inventory` 注册表，自动落进您项目的 `console` 二进制文件里。这个二进制文件的机制请参见[控制台](console.md)；这一节讲的是填充器专属的那部分表面。

### 运行全部

```bash
cargo run --bin console -- db:seed
```

按顺序运行每一个已注册的填充器。在一个空注册表上，它会往 stderr 打印一条警告（`db:seed: no seeders registered - nothing to run`），然后以零退出 - 这对“有人在注册任何东西之前就运行了这个命令”这种情况来说是正确的行为，也能让那些没有填充过任何特定东西的测试套件不会因此失败。

### 运行一个填充器

三种可接受的形式，按它们看起来有多像 Laravel 递增排列：

```bash
cargo run --bin console -- db:seed --class=UsersSeeder
cargo run --bin console -- db:seed --class UsersSeeder
cargo run --bin console -- db:seed UsersSeeder
```

这三种形式都会按精确的名字，在这个注册表里查找这个填充器并运行它。

一次有针对性的运行会报告它的进度：

```text
  UsersSeeder .......................................................... RUNNING
  UsersSeeder ...................................................... 812 ms DONE

```

这些行会走到 stdout。一个光秃秃的 `db:seed` 保持沉默 - 否则一次完整的填充，会把它自己的输出埋在每个填充器一行的信息底下。每个填充器发出的那条 `tracing` 记录没有变化，它仍然是给机器看的那条通道。

一个未知的名字会快速失败：

```bash
cargo run --bin console -- db:seed --class=NotARealSeeder
# Error: no seeder registered for `NotARealSeeder`
# (exit 1)
```

一个格式错误的标志（`--class` 后面没有值，`--class=` 带一个空值，`--class --force`）也会快速失败，并带上一条点名预期形态的诊断信息。

### 从一个构建好的二进制文件

在一个容器化或者由 systemd 管理的部署里，这个控制台二进制文件位于 `target/release/console`（或者您的发布产物落地的任何地方）。语法一样，前面不带 `cargo`：

```bash
./console db:seed
./console db:seed --class=BaseSeeder
```

这个控制台二进制文件调用的是 `suprnova::console::dispatch_argv(std::env::args())`，它会路由经过和 `cargo run --bin console --` 同一个注册表。构建产物没有一条单独的分发路径。

## 与工厂组合

填充器几乎总是会去调用[工厂](eloquent.md)。这个工厂 trait 知道怎么构建一个模型的随机化实例；填充器负责给这些工厂调用排好顺序，以及处理任何不可随机化的接线（确定性的管理员凭据、联结表的行、文件上传）。

最小的工厂 + 填充器组合：

```rust
// src/factories/user_factory.rs
use suprnova::Factory;
use crate::models::users::User;

pub struct UserFactory;

impl Factory for UserFactory {
    type Model = User;

    fn definition() -> User {
        User {
            id: 0,                              // persist_via_seaorm 会把主键翻转成 NotSet
            name: "Factory User".into(),
            email: "factory@example.suprnova.app".into(),
            password: "factory-placeholder".into(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            ..Default::default()
        }
    }
}
```

```rust
// src/seeders/users_seeder.rs
use suprnova::{async_trait, Factory, FrameworkError, Seeder};
use crate::factories::UserFactory;

pub struct UsersSeeder;

#[async_trait]
impl Seeder for UsersSeeder {
    fn name() -> &'static str { "UsersSeeder" }

    async fn run() -> Result<(), FrameworkError> {
        UserFactory::new().count(50).create_many().await?;
        Ok(())
    }
}
```

这个流式构建器活在 `FactoryBuilder<M>` 上；您能在 `create_many` 之前链上的东西，和 Laravel 是一致的：

```rust
// 构建一行持久化的记录，带上覆盖：
let admin = UserFactory::new()
    .with(|u| u.email = "admin@example.com".into())
    .with(|u| u.role = "admin".into())
    .create()
    .await?;

// 构建 N 行持久化的记录，全都是管理员：
UserFactory::times(5)
    .with(|u| u.role = "admin".into())
    .create_many()
    .await?;

// 条件式状态 - 只有当这个标志被设置时才应用这个闭包：
UserFactory::times(10)
    .when(seed_admins, |b| b.with(|u| u.role = "admin".into()))
    .create_many()
    .await?;
```

`make` / `make_one` / `make_many` 是仅存在于内存里的姊妹方法（不插入），给那些不想要一次数据库往返的单元测试用。完整的工厂表面（包括 `prepend`、`Sequence`，以及那个从 `#[factory(model = "…")]` 属性生成标记结构体的 `#[derive(Factory)]` 宏）请参见 [Eloquent](eloquent.md) 一章。

### 幂等性是填充器自己的责任

`run_all` 不会拍快照，也不会包一个事务；如果一个填充器无条件地插入，重新运行它就会产出重复数据。让一个填充器可以安全地重新运行，有两种标准方式：

- **先重置。** 本地开发的“清空再重新填充”循环，通常做的是 `suprnova migrate:fresh && cargo run --bin console -- db:seed` - `migrate:fresh` 会删除并重建每一张表，所以这个填充器总是从空开始。这是大多数项目日常使用的形态。
- **Upsert / 先检查。** 对于一个必须和现有数据共存的填充器（生产环境里的一个默认管理员账户，一份规范的国家列表），就用一次查找来守住这次插入，或者用一条 upsert 查询。

```rust
async fn run() -> Result<(), FrameworkError> {
    let exists = User::query()
        .db_where("email", "admin@example.com")
        .exists()
        .await?;

    if !exists {
        let password_hash = suprnova::hashing::hash("change-me-on-first-login")?;
        User::create(attrs!{
            email: "admin@example.com",
            name: "Admin",
            password: password_hash,
        }).await?;
    }
    Ok(())
}
```

## 用 `without_events` 让模型事件静音

一个在循环里调用 `Model::create` 的填充器，会在每一行上触发每一个生命周期事件 - `Creating`、`Saving`、`Created`、`Saved`。那会唤醒任何已注册的 `Observer<M>`，运行任何已排队的广播监听器，还可能顺带入队上百个您其实并不想要的后台作业。`seed::without_events` 就是 Laravel `WithoutModelEvents` 的对应物：

```rust
use suprnova::{async_trait, FrameworkError, Seeder, seed};
use crate::models::users::User;

pub struct UsersSeeder;

#[async_trait]
impl Seeder for UsersSeeder {
    fn name() -> &'static str { "UsersSeeder" }

    async fn run() -> Result<(), FrameworkError> {
        seed::without_events(async {
            for i in 0..50 {
                User::create(attrs!{
                    name: format!("user{i}"),
                    email: format!("user{i}@example.com"),
                }).await?;
            }
            Ok(())
        }).await
    }
}
```

当这个内部的 future 在等待时，可取消的否决路径（`dispatch_cancellable`）和事件之后的扇出（`dispatch_after`）都会短路到 `Ok(())`。观察者保持沉默，广播器不会被唤醒，下游作业也不会入队。

这个效果是任务作用域的 - 只有在 `fut` 内部执行的工作会被静音。其他任务上的并发工作（HTTP 请求处理程序、在后台运行的队列工作进程、其他填充器）会继续正常触发事件。嵌套调用是可以组合的：一个内层的 `without_events` 块，会继承外层的标志。

### 工厂本来就会绕开模型事件

值得了解，因为它会改变您什么时候需要伸手去用 `without_events`：工厂是通过 `ActiveModelTrait::insert`（SeaORM 模型上的那个 `Persistable` 实现）来持久化的，不会经过 `Model` trait 的 `create` / `save` 方法。在一条工厂驱动的路径上，没有模型事件派发需要静音。`seed::without_events` 是给那些直接驱动 `Model` trait 的代码用的 - 通常是因为您需要工厂绕开的那种运行时形态的易用性，或者因为您在填充过程中触碰的这个模型，在生产环境里本该由一个观察者去响应，但在一次夹具加载期间不该如此。

实际上：如果您的填充器是一叠 `UserFactory::new().create_many()` 调用，您不需要 `without_events`。如果它是一个手写的 `User::create(attrs)` 循环，您大概需要。

## 在测试里使用填充器

这个控制台二进制文件驱动的同一个注册表，也能从一个 `#[tokio::test]` 里调用 - 当您想在一个集成测试前面摆好一份已知的夹具集合时，这很好用：

```rust
use serial_test::serial;
use suprnova::container::testing::TestContainer;
use suprnova::{DbConnection, seed};

use app::seeders::BaseSeeder;

#[tokio::test]
#[serial]
async fn dashboard_renders_seeded_posts() {
    // 重置这个注册表，这样前一个测试的注册就不会泄漏过来。
    seed::clear();

    let _guard = TestContainer::fake();
    let conn = sea_orm::Database::connect("sqlite::memory:").await.unwrap();
    app::migrations::Migrator::up(&conn, None).await.unwrap();
    TestContainer::singleton(DbConnection::from_raw(conn.clone()));

    // 注册您想要的那个填充器，运行它，然后针对这个全新的数据库做断言。
    seed::register::<BaseSeeder>();
    seed::run_all().await.unwrap();

    // ……针对已填充数据的控制器测试……

    seed::clear();
}
```

关于这个测试形态的两点说明：

- 当测试会改动这个进程全局的注册表时，`#[serial]` 是必需的 - 共享同一个注册表的并行测试会产生竞态。在您项目的 `Cargo.toml` 里加上 `serial_test` 作为开发依赖，才能拿到这个属性。
- `seed::clear()` 是一个 `#[doc(hidden)]` 的、仅供测试使用的助手方法。不要从生产代码里调用它；这个注册表在启动时构建一次，永远不会被重置。

更宽泛的测试装置约定（`#[suprnova_test]`、`TestContainer`、`TestDatabase::fresh::<Migrator>()`，以及每个外部表面的伪造实现）请参见[测试](testing.md)。

## 何时该填充、迁移，还是用工厂

这三种模式都是把行放进表里。这个决策通常很直白，但值得把分界线明确点出来，因为 PHP 团队经常把它们混到一起。

| 您想要… | 用 |
|---|---|
| 一列存在 | [迁移](migrations.md) |
| 一行必须存在才能让应用启动（默认管理员、单例的站点配置行、规范的货币列表） | **填充器** - 幂等，在每个环境里都运行，包括生产环境 |
| 一批随机化的行，用于本地开发或预发布（50 个用户，200 篇文章，1000 个事件） | 调用一个工厂的填充器 |
| 一个单元测试需要的一行 | 在测试内部直接调用的[工厂](eloquent.md) |
| 一行的形态 | [工厂](eloquent.md) |

要避免的错误：

- **不要从一条迁移里插入数据。** 迁移描述的是架构，不是状态。一条插入默认行的迁移，会在生产数据库上运行一次，然后再也不会运行 - 一列发生变化的那一刻，您就在迁移历史和填充器之间，分出了两个事实来源。把这个插入放进一个填充器；如果生产环境需要这一行，就把 `console db:seed --class=DefaultsSeeder` 作为部署的一部分来运行。
- **不要手工把夹具数据写进您的测试。** 伸手去用一个工厂。测试里五个 `User::create(attrs!{ … })` 代码块，在您加上一列 NOT NULL 的那一刻，就是五次重写。一个 `UserFactory::new().create()` 能扛住这一切。
- **不要把生产数据放进一个填充器。** 填充器是给应用运转所需要的那些行用的，不是给“这是我们要导入的 8,000 条历史记录”用的。导入是一次性脚本（为它们写一个 `#[command]`；参见[控制台](console.md)）。

### 为什么 Suprnova 有所不同

Laravel 发布了一个 `DatabaseSeeder` 类，带着一个 Eloquent 的填充器加载器能认出来的特例 `call($seeders)` 助手方法。Suprnova 没有 - 这个注册表是一个扁平的 `IndexMap`，每个填充器都是平级的，一个复合填充器要靠调用 `seed::run_one(name)`（或者直接调用那些子工厂）来串联。

原因和您在 Suprnova 别处看到的是同一种权衡：一个带着单一排序规则的通用注册表，比一个带着魔法根节点的类层级更容易推理。Laravel 这套模式之所以行得通，是因为 PHP 的类自动加载和静态的 `make()` 反射，让 `call([A::class, B::class])` 能按名字找到并实例化这些类；在 Rust 里，我们等于是在要求用户到处穿针引线地传递 `dyn Seeder` trait 对象，这比那个已经就位的函数指针注册表要笨重得多。

这个复合填充器的约定，找回了同样的易用性 - `BaseSeeder` 扮演的角色，正是 `DatabaseSeeder` 在 Laravel 里扮演的那个角色 - 而不需要框架把某一个名字奉为特例。

填充器的进度行是固定 80 列的纯文本。Laravel 会按终端宽度调整它那串点引导线，并给状态词上色；而读出终端的真实宽度，意味着一个这个框架并不携带的依赖，何况这份输出去的是一个经常被管道接进日志里的 stdout，在那里转义码就是噪声。耗时以整毫秒打印，不带千位分隔符。

## 在 Bootstrap 里注册

每一个填充器都需要在 `bootstrap.rs` 里有一次 `seed::register` 调用，和其他进程全局的接线（配置、观察者、监督程序、队列作业）放在一起。这个模式和 bootstrap 文件里其他地方用的是同一个形态：

```rust
// src/bootstrap.rs
pub async fn register() {
    // ……配置 + 容器绑定 + 认证接线……

    // 填充器。顺序有影响 - run_all 会按注册顺序访问。
    suprnova::seed::register::<crate::seeders::BaseSeeder>();
    suprnova::seed::register::<crate::seeders::DemoContentSeeder>();

    // ……观察者、监督程序、队列作业……
}
```

如果您忘了注册一个填充器，`console db:seed --class=X` 就会带着“no seeder registered for `X`”失败 - 这是一个清晰的信号，而不是一次悄悄的跳过。`seed::count()` 和 `seed::is_registered("…")` 这两个助手方法存在的意义，正是为了让一个测试能断言 bootstrap 注册了您期望的每一个填充器。

完整的文件结构，以及框架期望每个子系统接入的顺序，请参见[应用启动](bootstrap.md)。

## 下一步

- [迁移](migrations.md) - 填充/迁移这一对里架构那一半
- [Eloquent](eloquent.md) - 模型、工厂，以及每个填充器都会调用进去的那套 `Persistable` 机制
- [控制台](console.md) - 那个承载着 `db:seed` 和您自己的 `#[command]` 的逐项目 `console` 二进制文件
- [测试](testing.md) - `TestContainer`、`TestDatabase::fresh`，以及那些会触碰填充器注册表的测试所用的 `#[serial]` 模式
- [错误模型](error-model.md) - `FrameworkError` 是什么，以及 `run` 的 `Result<(), _>` 形态如何与框架的其余部分组合
