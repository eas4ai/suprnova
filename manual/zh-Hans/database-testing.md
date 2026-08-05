# 数据库测试

这是 [测试](testing.md) 那一章在数据库方面的姊妹篇。那一章覆盖的是测试装置 - `#[suprnova_test]`、`describe!` / `test!`、`expect!`，以及进程内的伪造实现 - 而这一章覆盖的是当您的测试需要一个数据库时会发生什么变化：`TestDatabase` 如何为您构建一个数据库、隔离究竟是怎么工作的、工厂和填充器接在哪里，以及一个内存中的 SQLite 什么时候够用、什么时候不够用。

## 两个构造函数

每一个数据库测试都从构建一个 `TestDatabase` 开始。两个构造函数，两种意图。

### `TestDatabase::fresh::<Migrator>()`

构建一个内存中的 SQLite 数据库，端到端地运行您的迁移器，并把这个连接注册进测试容器，这样任何调用 `DB::connection()` 或 `App::resolve::<DbConnection>()` 的代码都会解析到它。对于任何会触碰真实架构的东西，这都是正确的默认选择。

```rust
use suprnova::testing::TestDatabase;
use crate::migrations::Migrator;

#[tokio::test]
async fn user_lifecycle_end_to_end() {
    let db = TestDatabase::fresh::<Migrator>().await.unwrap();

    let alice = User::create(attrs! {
        name: "Alice", email: "alice@example.com",
    })
    .await
    .unwrap();

    assert!(alice.id > 0);
    // 想绕过模型表面直接查询时：
    let row = users::Entity::find_by_id(alice.id)
        .one(db.conn())
        .await
        .unwrap();
    assert!(row.is_some());
}
```

`Migrator` 是您应用的 `MigratorTrait` 实现 - 与生产环境的 `suprnova migrate` 命令运行的是同一个类型。通过让真实的迁移器穿过测试架构，您让架构漂移变得不可能：迁移器忘了加的一列，不可能悄悄出现在测试数据库里。

`test_database!()` 宏是给常见情形（`crate::migrations::Migrator`）用的语法糖：

```rust
use suprnova::test_database;

#[tokio::test]
async fn shortcut() {
    let db = test_database!();          // == TestDatabase::fresh::<crate::migrations::Migrator>()
    // ...
}

// 或者用一个自定义的迁移器路径：
let db = test_database!(my_crate::CustomMigrator);
```

### `TestDatabase::sqlite_memory()`

同样的容器和注册表接线，但**不运行任何迁移器**。当这个测试想要对列的形态做精确控制时就用它 - 通常是转换往返测试、查询构造器的 SQL 表面测试，或者驱动程序层面的边界情形，这些场合下一个完整的迁移器只会是多余的噪音：

```rust
let db = TestDatabase::sqlite_memory().await.unwrap();
db.execute_unprepared(
    "CREATE TABLE casts_t (id INTEGER PRIMARY KEY, payload BLOB)",
)
.await
.unwrap();

// 然后直接写入，用类型化的助手方法读回来：
let row = db.fetch_one(
    "INSERT INTO casts_t (payload) VALUES (?) RETURNING id, payload",
    vec![sea_orm::Value::Bytes(Some(Box::new(b"hello".to_vec())))],
).await.unwrap();
```

`sqlite_memory()` 是 `fresh()` 所建立在其上的那个基础 - `fresh` 会调用它，然后再运行您的迁移器。凡是 `fresh` 能做的事，您在这里都能做；只是您要自己带上 DDL。

### `execute_unprepared`、`fetch_one`、`fetch_all`

`TestDatabase` 重新导出了您在测试里最常用到的三种 SeaORM 执行形态，这样测试文件就不必再去引入 `ConnectionTrait`：

| 方法 | 用于 |
| --- | --- |
| `execute_unprepared(sql)` | 不带占位符的 DDL 或 DML。返回 `Result<(), FrameworkError>` |
| `fetch_one(sql, bindings)` | 单行 SELECT。零行时报错 |
| `fetch_all(sql, bindings)` | 全行 SELECT |

这些绑定是 `Vec<sea_orm::Value>` - 和生产环境查询路径用的是同一个形态。这个连接的后端（两个构造函数都是 SQLite）已经为您准备好了，所以一个 `?` 占位符是正确的写法。

## 隔离究竟是怎么工作的

每个测试一个全新数据库的模型，就是这个隔离机制本身。每一次调用 `fresh()` 或 `sqlite_memory()`，都会打开一个新的 `sqlite::memory:` 连接，在 SQLite 之下，这是一个完全独立的数据库实例 - 没有共享的架构，没有共享的行，没有其他测试能看进去。这里没有事务包装器，没有需要选择接入的 `RefreshDatabase` trait，也没有需要记住的回滚：*下一个*测试会拿到一个干净的空数据库，因为它是自己构建的。

当 `TestDatabase` 值被丢弃时，会依次发生三件事：

1. 持有的那个 `TestContainerGuard` 会清空线程本地的测试容器，这样任何后续的 `App::get::<DbConnection>()` 都不会再找到那个测试连接。
2. 如果这是进程里*最后*一个存活的 `TestContainerGuard`，具名的 [`ConnectionRegistry`](database.md#named-connections) 就会被清空。（`FAKE_GUARDS` 上的一份引用计数保证，一个内层测试的丢弃不会抹掉一个并发的外层测试仍然依赖的连接名 - 这正是引入这份引用计数所要解决的那个老陷阱。）
3. SQLite 连接自身会丢弃，销毁掉这个内存数据库。

因为状态是重新构建而不是被回滚的，这份隔离比 `BEGIN`/`ROLLBACK` 包装更强：没有已提交的状态会被误留下来，没有嵌套事务的怪癖，测试之间也没有序列计数器的漂移。代价是您要为每个测试运行一次迁移器付出成本（对大多数架构下的 SQLite 来说微不足道；如果它变成了一个实实在在的成本，请参见下面的“在多个测试之间共享一个已迁移的数据库”）。

## 为什么这个连接池被钉死在一个连接上

两个构造函数都用 `max_connections(1)` 和 `min_connections(1)` 来构建这个数据库。这是 `sqlite::memory:` 的一处承重结构，不是一条通用策略。

`sqlite::memory:` 是一个逐连接的数据库 - 连接池里每一个*新*连接都会是一个独立的、空的 SQLite 实例。一个大小为 2 的连接池，会意味着您一半的查询看到的是已迁移的数据库，另一半看到的是一个空数据库。把这个连接池钉在一个连接上，能让测试里的每一次查询都落在迁移器运行时所用的那同一个内存数据库上。

其后果是：一个要演练真实连接并发的测试（两个事务互相竞争，副本路由，一个队列工作进程在一个请求处理程序也在操作数据库的同时命中它），需要一个真实的数据库。参见下面的“何时内存中的 SQLite 不够用”。

## 测试里的工厂

工厂会生产随机化的模型实例，并（可选地）把它们持久化。持久化路径会自动解析出那个已绑定的测试连接 - 不需要为测试再单独接一条工厂侧的接线。

```rust
use crate::factories::UserFactory;

#[tokio::test]
async fn factory_round_trip() {
    let _db = TestDatabase::fresh::<Migrator>().await.unwrap();

    // 只在内存里：最快，没有数据库往返。
    let alice = UserFactory::new()
        .with(|u| u.email = "alice@example.com".into())
        .make();
    assert_eq!(alice.email, "alice@example.com");

    // 持久化一个 + 返回插入后的模型（已分配 id）。
    let bob = UserFactory::new().create().await.unwrap();
    assert!(bob.id > 0);

    // 批量：依次持久化 50 个。
    let many = UserFactory::times(50).create_many().await.unwrap();
    assert_eq!(many.len(), 50);
}
```

两个值得了解的模式：

**工厂的插入操作会绕开模型事件。** 撑起 `create()` / `create_many()` 的那个 `Persistable` 实现，是直接通过 SeaORM 的 `ActiveModelTrait::insert` 来写入的 - 它*不会*经过那个会派发 `Creating` / `Created` / `Saving` / `Saved` 的 `Model::create` 表面。一个断言“构建这个夹具时没有任何观察者触发”的测试不需要任何特殊处理；而一个断言“`Created` 观察者确实触发了”的测试，必须驱动 `Model::create(...)`（或 `save()`），而不是一个工厂。

**`create_many` 不做事务处理。** 插入是顺序进行的。如果一个后面的行失败了，前面的行不会被回滚。如果一个测试需要原子性，就把这次调用包进您自己的 `DB::transaction`：

```rust
DB::transaction(|tx| async move {
    UserFactory::times(50).create_many().await?;
    PostFactory::times(200).create_many().await?;
    Ok::<_, FrameworkError>(())
}).await.unwrap();
```

完整的工厂表面（状态、序列、`with` 关系、`count`、`times`、`make_one` / `create_one`）请参见 [Eloquent 工厂](eloquent-factories.md)。

## 测试里的填充器

填充器是您用一个稳定名字注册进框架的填充器注册表的函数。从测试里驱动它们有两种模式，各自对应一种不同的意图。

### 按名字运行单个填充器

```rust
use suprnova::seed;
use my_app::seeders::UsersSeeder;

#[tokio::test]
async fn users_seeder_populates_fixtures() {
    let _db = TestDatabase::fresh::<Migrator>().await.unwrap();

    seed::register::<UsersSeeder>();
    seed::run_one("UsersSeeder").await.unwrap();

    let count = User::query().count().await.unwrap();
    assert!(count > 0);
}
```

### 运行完整的启动填充器集合

```rust
use serial_test::serial;
use suprnova::seed;

#[tokio::test]
#[serial]
async fn full_seed_lands_expected_row_counts() {
    seed::clear();                              // 从一个已知为空的注册表开始
    let _db = TestDatabase::fresh::<Migrator>().await.unwrap();

    seed::register::<my_app::seeders::UsersSeeder>();
    seed::register::<my_app::seeders::PostsSeeder>();
    seed::run_all().await.unwrap();

    let users = User::query().count().await.unwrap();
    let posts = Post::query().count().await.unwrap();
    assert_eq!(users, 50);
    assert_eq!(posts, 200);

    seed::clear();
}
```

两个重要的契约细节：

**填充器注册表是进程全局的。**`seed::register::<S>()` 会插入一个以 `S::name()` 为键的 `RwLock<IndexMap>`。一个会改动这个注册表的测试，应该在入口调用 `seed::clear()`，注册它需要的填充器，运行，再在出口调用一次 `clear()` - 并且这个测试自身应该是 `#[serial_test::serial]` 的，这样两个并行的测试就不会为这个注册表打起来。`#[suprnova_test]` **不会**自动注册填充器；只有您自己在 `bootstrap.rs` 或测试方法体里显式调用 `seed::register::<>()`，才会把它们放进这个注册表。

**模型驱动的填充 vs 工厂驱动的填充。** 一个在 `for` 循环里调用 `User::create(...)` 的填充器，会在每一行上派发 `Creating` / `Saving` / `Created` / `Saved`，并调用每一个已注册的观察者。对于这种扇出是不受欢迎的批量填充场景，就把这个循环包进 `seed::without_events`：

```rust
seed::without_events(async {
    for i in 0..50 {
        User::create(attrs! { name: format!("user{i}"), email: format!("user{i}@example.com") }).await?;
    }
    Ok::<_, FrameworkError>(())
}).await?;
```

这份静音是**任务作用域**的 - 只有在这个 future 内部执行的工作会被压住；并发的请求处理程序和队列工作进程会继续照常触发事件。工厂（`create_many`）已经绕开了事件路径，所以在它们周围用不上 `without_events`。

参见 [数据填充](seeding.md) 了解填充器的编写表面，[Eloquent 工厂](eloquent-factories.md) 了解两者之间的关系。

## 并行安全的数据库测试

`cargo test` 按线程并行运行测试。默认的 `#[suprnova_test]` 展开（也就是 `#[tokio::test]`，即每个测试一个 `current_thread` 运行时）对此天然安全，原因有两个：

- **每个测试都有自己的 `sqlite::memory:` 连接。** 测试之间不共享数据库状态。
- **已绑定的连接活在线程本地的 `TestContainer` 里。** 测试之间不共享容器绑定。

您不需要去想的事情：`DB::connection()`、`App::resolve`、工厂的持久化、模型 trait 的写入 - 这些全都会透明地落在正确的那个逐测试数据库上。

您*确实*需要去想的事情：

| 表面 | 为什么它是进程全局的 | 缓解方式 |
| --- | --- | --- |
| `ConnectionRegistry`（`DB::register_named`、`__read_replica__`） | 进程共享的单一个 `RwLock<HashMap>` | 任何注册或读取具名连接的测试都用 `#[serial_test::serial]` |
| 填充器注册表 | 单一个 `RwLock<IndexMap>` | `#[serial_test::serial]` + 在入口和出口调用 `seed::clear()` |
| Eloquent 的观察者 / 作用域注册表 | 按 `TypeId::<M>()` 建立键 | 每个测试都应该用一个独有的模型结构体，或者标上 `#[serial]` 并调用这个注册表的 `clear()` 助手方法 |
| 具名的查询日志（`DB::enable_query_log`） | 进程全局的单一个环形缓冲 | 如果断言要读这个日志，就用 `#[serial]` |

连接注册表上的引用计数，让这件事比听起来的样子更安全：一个持有 `TestContainerGuard` 的测试，即便某个*兄弟*测试的守卫被丢弃了，也能让这个注册表保持存活。对于那些真的会改动这个注册表的测试，您仍然想要 `#[serial]`，这样它们的读和写就不会交错。

### 多线程运行时的注意事项

`#[suprnova_test]` 会展开成带默认 `current_thread` 运行时的 `#[tokio::test]`，所以这条线程本地的容器路径始终有效。如果您显式地让一个测试选择接入多线程运行时：

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn parallel_io_test() {
    let _db = TestDatabase::fresh::<Migrator>().await.unwrap();
    // 问题：用 tokio::spawn 派生出来的任务，可能运行在一个和构建
    // 这个 TestDatabase 时不同的工作线程上。它们看不到那个线程本地的
    // TestContainer 绑定，DB::connection() 会返回全局（生产）
    // 容器的值，或者报错。
}
```

两种修法，取决于这个测试做什么：

1. **直接访问连接** - 不管是哪个工作线程在读它，`db.conn()` 都会返回正确的那个 `&DatabaseConnection`。如果这个测试始终只通过 `db` 句柄（而不是通过 `DB::connection()`）和数据库对话，多线程运行时就没问题。

2. **`TestContainer::scope`** - 把测试方法体包进 `TestContainer::scope(async { ... }).await`，并在其内部绑定您的伪造实现（以及这个数据库连接）。这个作用域会把容器绑定到任务本地层，这一层即使在运行时把这个 future 跨工作线程调度时，也会在每次 await 之后保留下来。对于派生出来的子任务，请用 `TestContainer::spawn`（而不是裸的 `tokio::spawn`），这样这个任务本地容器就会被捕获，并重新安装进这个被派生出来的 future 内部。

完整的任务本地 / 线程本地 / 全局分层，请参见[服务容器 → 查找顺序](container.md)。

## SQLite 内存数据库 vs 真实的 Postgres / MySQL / MariaDB

`TestDatabase` 是刻意做成只支持 SQLite 的。这个驱动程序是硬编码到 `sqlite::memory:` 的；没有 `TestDatabase::postgres()`、`fresh_with_url()`，或者一个由环境变量驱动的变体。对于绝大多数的测试表面 - 模型的增删改查、查询构造器的形态、转换往返、关系加载、观察者触发顺序、软删除语义 - 内存中的 SQLite 就是正确的工具：零设置、零网络、每个测试只要几毫秒、完美的隔离、CI 里不需要保活任何外部服务。

有四种情况，内存中的 SQLite 是不够用的：

1. **驱动程序专属的 SQL。** 一个用到 Postgres 的 `LATERAL`、`JSONB` 运算符、`ON CONFLICT ... WHERE`，MySQL 的窗口函数，或者任何其他方言专属表面的查询，在 SQLite 上跑不起来。模型加构造器这条路径尽量保持通用，但一个断言 Postgres 形态输出的原始 SQL 测试，需要一个 Postgres。
2. **真实连接争用下的并发。** 内存中的 SQLite 是单连接的（参见“为什么这个连接池被钉死在一个连接上”）。那些让两个事务互相竞争、演练读副本路由的负载，或者测量死锁重试的测试，需要一个多连接的服务器。
3. **向量 / NoSQL / 时序表面。** Suprnova 的 MariaDB `VECTOR` 驱动程序、Qdrant 集成、Pinecone 集成，以及类似的非 SQL 驱动程序，在 SQLite 里完全没法建模。
4. **生产环境对等的冒烟测试。** 少数几个“这在我们实际部署的那个真实数据库上真的能跑吗？”的测试，被限定在 CI 里运行，即便单元测试这一层是 SQLite，这些测试也值得保留。

对这四种情况，模式都是一样的：完全走出 `TestDatabase`，针对一个由运维人员提供的、`DATABASE_URL` 风格的环境变量，构建一个 `DbConnection`，用环境变量把这个测试挡住，让它在这个变量缺失时跳过，并标上 `#[serial]`，这样两个这样的测试就不会为共享的真实数据库打起来。`framework/tests/vector_mariadb.rs` 里的 `MARIADB_URL` 模式就是那个标准范例：

```rust
use serial_test::serial;
use suprnova::database::{DatabaseConfig, DbConnection};

async fn maybe_real_db(test_name: &str) -> Option<DbConnection> {
    let url = match std::env::var("POSTGRES_TEST_URL") {
        Ok(u) if !u.is_empty() => u,
        _ => {
            eprintln!("[{test_name}] skipping: POSTGRES_TEST_URL not set");
            return None;
        }
    };
    let config = DatabaseConfig::builder().url(&url).build();
    Some(DbConnection::connect(&config).await.expect("real DB connects"))
}

#[tokio::test]
#[serial]
async fn jsonb_operator_works_against_postgres() {
    let Some(conn) = maybe_real_db("jsonb_operator_works_against_postgres").await else {
        return;
    };
    // 直接针对 conn 驱动 Postgres 专属的 SQL。
}
```

这个约定始终不变：用目标驱动程序来命名这个环境变量（`POSTGRES_TEST_URL`、`MYSQL_TEST_URL`、`MARIADB_URL`），打印一行跳过信息，这样在本地运行这个套件的开发者能看到这个测试被跳过了（而不是被悄悄地判定通过），并且在这个测试模块的开头文档注释里记录这个环境变量，好让 CI 能把它接上。

## 一个实操示例

把这一章的一切都组合起来的完整应用实战模式：

```rust
use app::migrations::Migrator;
use app::models::posts::Post;
use app::models::users::User;
use serial_test::serial;
use suprnova::testing::TestDatabase;
use suprnova::{Model, attrs, seed, FrameworkError};

#[tokio::test]
#[serial]
async fn users_and_posts_full_seed_round_trip() {
    // 1. 空的填充器注册表。
    seed::clear();

    // 2. 一个带上了应用迁移器的、全新的内存数据库。
    let db = TestDatabase::fresh::<Migrator>().await.unwrap();

    // 3. 注册这个测试关心的那些填充器。
    seed::register::<app::seeders::UsersSeeder>();
    seed::register::<app::seeders::PostsSeeder>();

    // 4. 在 without_events 内部驱动这次填充，这样观察者的扇出
    //    就不会试图去入队任务（这里没有任何队列在运行）。
    seed::without_events(async {
        seed::run_all().await
    }).await.unwrap();

    // 5. 通过模型表面和原始连接把数据读回来。
    let user_count = User::query().count().await.unwrap();
    assert_eq!(user_count, 50);

    let raw_post_count = db.fetch_one(
        "SELECT COUNT(*) AS n FROM posts",
        vec![],
    ).await.unwrap();
    let n: i64 = raw_post_count.try_get("", "n").unwrap();
    assert_eq!(n, 200);

    // 6. 在一个全新的模型上演练那条可取消的观察者路径。
    let alice = User::create(attrs! {
        name: "Alice", email: "alice@example.com",
    }).await.unwrap();
    assert!(alice.id > 0);

    seed::clear();
}
```

第 5 步正是证明这份接线成立的地方：这个模型查询和这次原始的 `fetch_one`，读的是同一个内存数据库 - 模型表面是因为 `DB::connection()` 的查找找到了那个 `TestContainer` 绑定，原始的 `fetch_one` 是因为 `db.conn()` 直接返回的就是那同一个连接。

## 交叉引用

- [测试](testing.md) - 测试装置、`expect!`、`describe!`、`test!`、伪造实现。
- [数据库](database.md#testing) - 引入 `TestDatabase` 的那个表面层测试小节。
- [Eloquent 工厂](eloquent-factories.md) - 工厂的定义语法、状态、序列、关系。
- [数据填充](seeding.md) - 填充器的编写、顺序、幂等性。
- [服务容器](container.md) - 任务本地 vs 线程本地 vs 全局查找，决定的是测试内部 `DB::connection()` 会解析到什么。
- [模拟和伪造](mocking.md) - `Storage::fake`、`Mail::fake`、`Queue::fake`、`Notification::fake`，以及用于替换掉伪造 HTTP 客户端和其他外部表面的 trait 绑定模式。
- [HTTP 测试](http-tests.md) - 在绑定了一个 `TestDatabase` 的情况下，驱动处理程序穿过这套路由栈。
