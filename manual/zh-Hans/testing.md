# 测试

这是 Suprnova 测试表面的枢纽章节 - 宏、进程内数据库、容器伪造实现，以及您的测试二进制文件会用到的加密密钥辅助工具。深入讲解的章节和它并排放着：路由 + 中间件见[HTTP 测试](http-tests.md)，`TestDatabase` 周边的一切见[数据库测试](database-testing.md)，七个外部表面（Mail、Notify、Queue、Bus、Events、Storage、HTTP 客户端）见[模拟和伪造](mocking.md)。读这一章来了解盒子里都有什么；需要长篇讲解时再跳到旁边的章节。

## 组成部分

| 组成部分 | 角色 |
|---|---|
| `#[tokio::test]` + `TestDatabase::fresh::<Migrator>()` | 默认的主力 - 框架里每一个真实的测试都用它 |
| `#[suprnova_test]` | 属性宏糖 - 运行 `App::init()` + `App::boot_services()`，并为您构建一个 `TestDatabase` |
| `describe!` + `test!` | Jest 形态的分组宏，和 `expect!` 搭配，给出具名的失败输出 |
| `expect!` | 带类型化匹配器（相等性、option、result、字符串、vec、排序）的 fluent 断言宏 |
| `TestDatabase::fresh` / `sqlite_memory` | 内存 SQLite + 容器注册，可以带着您的迁移器，也可以不带 |
| `TestContainer::fake` / `scope` / `spawn` | 线程本地或者任务本地的 DI 覆盖，在并行测试之间彼此密封 |
| `install_test_encryption_key[ring]` | 给那些会碰到加密类型转换或者签名负载的测试用的、确定性的 `APP_KEY` |
| 逐表面的 `fake()` 辅助函数 | Mail、Notify、Queue、Bus、Events、Storage、HTTP - 参见[模拟](mocking.md) |
| `TestResponse` | 对 HTTP 测试 `(status, headers, body)` 三元组的 fluent 断言 - 参见 [HTTP 测试](http-tests.md#fluent-response-assertions-with-testresponse) |
| `AssertableInertia` | 对 Inertia 页面对象的 fluent 断言 - 参见 [HTTP 测试](http-tests.md#testing-inertia-responses) |

您不会在一个测试里用到所有这些东西。一个典型的 action 测试会用到前三个；一个 DI 很重的测试会加上 `TestContainer`；一个 HTTP 测试会把 `TestDatabase` 换成 `handle_request` 管道；一个支付测试会装上这个加密密钥环。

## 默认的主力

框架里每一个真实的测试看起来都是这样：

```rust
use suprnova::testing::TestDatabase;
use crate::migrations::Migrator;

#[tokio::test]
async fn create_user_persists_it() {
    let db = TestDatabase::fresh::<Migrator>().await.unwrap();

    let alice = User::create(attrs! {
        name: "Alice",
        email: "alice@example.com",
    })
    .await
    .unwrap();

    assert!(alice.id > 0);

    let row = users::Entity::find_by_id(alice.id)
        .one(db.conn())
        .await
        .unwrap();
    assert!(row.is_some());
}
```

`TestDatabase::fresh::<M>()` 会打开一个全新的 `sqlite::memory:` 连接，端到端地运行您的迁移器，并把这个连接注册进测试容器。之后任何调用 `DB::connection()` 或者 `App::resolve::<DbConnection>()` 的代码都会解析到它 - 包括 `#[suprnova::model]` 这个查询构造器，以及您从容器里解析出来的任何服务。当这个 `TestDatabase` 被丢弃时，这份注册也会跟着一起消失。

`test_database!()` 这个宏是针对 `crate::migrations::Migrator` 这种情况的单行糖：

```rust
use suprnova::test_database;

#[tokio::test]
async fn shortcut() {
    let db = test_database!();         // == TestDatabase::fresh::<crate::migrations::Migrator>()
    // …
}
```

对于想要精确控制列形状的测试（类型转换的往返、查询构造器的 SQL 表面），请使用 `TestDatabase::sqlite_memory()` - 相同的容器接线，没有迁移器。DDL 由您掌握。完整的目录，加上 `execute_unprepared` / `fetch_one` / `fetch_all` 这些辅助函数，请参见[数据库测试](database-testing.md)。

## `#[suprnova_test]` - 当您想要糖的时候

`#[suprnova_test]` 是一个包裹 `#[tokio::test]` 的属性宏，它会调用 `App::init()` + `App::boot_services()`，让 `#[injectable]` 类型能被解析，并绑定一个全新的 `TestDatabase`。它是上面那种显式形态之上的可选糖，在一个测试需要解析容器注册过的服务时很有用：

```rust
use suprnova::suprnova_test;
use suprnova::{App, testing::TestDatabase};

#[suprnova_test]
async fn create_user_via_action(db: TestDatabase) {
    let action = App::resolve::<CreateUserAction>().unwrap();
    let user = action.execute("test@example.com").await.unwrap();

    assert_eq!(user.email, "test@example.com");
    assert!(user.id > 0);
}
```

如果这个函数接受一个 `TestDatabase` 参数（按名字），这个宏就会把这个全新的数据库绑定到那个名字上。如果没有，这个数据库依然会被构造和注册（这样 `DB::connection()` 才能工作） - 只是不会被绑定到一个局部变量上。

用 `migrator = …` 这个键覆盖迁移器：

```rust
#[suprnova_test(migrator = my_crate::tests::IsolatedMigrator)]
async fn create_user_with_isolated_schema(db: TestDatabase) {
    // …
}
```

未知的键会是一个编译错误（打错字的 `migrtor = …` 不会静默地保留默认迁移器）。

## `describe!` 和 `test!` - 当分组能帮上忙时

对于同一个 action 有很多种情况的测试文件，Jest 形态的 `describe!` + `test!` 组合会给您嵌套的分组和具名的失败输出：

```rust
use suprnova::{App, describe, test, expect, testing::TestDatabase};
use crate::migrations::Migrator;

describe!("ListTodosAction", {
    test!("returns empty list when no todos exist", async fn(db: TestDatabase) {
        let todos = App::resolve::<ListTodosAction>().unwrap().execute().await.unwrap();
        expect!(todos).to_be_empty();
    });

    test!("returns all todos", async fn(db: TestDatabase) {
        Todo::create(attrs! { title: "Buy bread" }).await.unwrap();
        Todo::create(attrs! { title: "Walk dog" }).await.unwrap();

        let todos = App::resolve::<ListTodosAction>().unwrap().execute().await.unwrap();
        expect!(todos).to_have_length(2);
    });

    describe!("with pagination", {
        test!("returns first page", async fn(db: TestDatabase) {
            // 嵌套的分组会组合起来
        });
    });
});
```

`test!` 接受三种形态：

```rust
// 带 TestDatabase 参数的异步测试
test!("creates a user", async fn(db: TestDatabase) { … });

// 不带数据库的异步测试
test!("calculates the right sum", async fn() { … });

// 同步测试
test!("adds numbers", fn() { … });
```

这个具名测试的包装器，会把测试名字一路传进 `expect!` 的机制里，这样一次失败就会浮现出来：

```text
Test: "returns all todos"
  at src/actions/todo_action.rs:25

  expect!(actual).to_equal(expected)

  Expected: 2
  Received: 0
```

没有 `describe!`/`test!` 时，您得到的是标准的 `panic!` 输出。有了它们，位置和人类可读的测试名字会引出这条消息。

## `expect!` - 匹配器目录

`expect!(value)` 返回一个 `Expect<T>` 包装器。这些匹配器是按 `T` 类型化的 - 在一个 `String` 上调用 `to_be_some()` 是一个编译错误，不是一次运行时 panic。

```rust
use suprnova::expect;

// 相等性（T: Debug + PartialEq）
expect!(actual).to_equal(expected);
expect!(actual).to_not_equal(unexpected);

// 布尔值
expect!(condition).to_be_true();
expect!(condition).to_be_false();

// Option<T>
expect!(option).to_be_some();
expect!(option).to_be_none();
expect!(option).to_contain_value(5);     // Some(5) 检查

// Result<T, E>
expect!(result).to_be_ok();
expect!(result).to_be_err();

// String / &str
expect!(s).to_contain("substring");
expect!(s).to_start_with("prefix");
expect!(s).to_end_with("suffix");
expect!(s).to_have_length(10);
expect!(s).to_be_empty();

// Vec<T>
expect!(v).to_have_length(3);
expect!(v).to_contain(&item);
expect!(v).to_be_empty();

// 排序（T: Debug + PartialOrd）
expect!(10).to_be_greater_than(5);
expect!(5).to_be_less_than(10);
expect!(10).to_be_greater_than_or_equal(10);
expect!(5).to_be_less_than_or_equal(5);
```

您可以在 `test!` 之外使用 `expect!` - 失败消息里的文件/行号来自 `concat!(file!(), ":", line!())`。这个具名测试的表头是唯一一个这个宏自己不会添加的东西。

## `TestContainer` - 不会渗漏的 DI 伪造实现

容器那一章详细讲了[三层查找](container.md)。对测试来说，两个入口点是 `TestContainer::fake()`（线程本地）和 `TestContainer::scope(…).await`（任务本地）。

### 线程本地，常见情况

`TestContainer::fake()` 返回一个 守卫。直到这个 守卫 被丢弃之前，`TestContainer::singleton` / `bind` / `factory` 的写入都会落在线程本地的覆盖层上，并遮蔽全局容器：

```rust
use std::sync::Arc;
use suprnova::App;
use suprnova::testing::TestContainer;

#[tokio::test]
async fn order_dispatches_email() {
    let _guard = TestContainer::fake();

    let fake = Arc::new(FakeEmailGateway::new());
    let probe = Arc::clone(&fake);
    TestContainer::bind::<dyn EmailGateway>(fake);

    place_order(123).await.unwrap();

    assert_eq!(probe.sent_count(), 1);
}
```

`TestDatabase::fresh` / `sqlite_memory` 会在内部装上它们自己的 `TestContainer::fake` 守卫 - 除非您正在测试这个注册表本身，否则不要把它们叠起来用。

### 任务本地，用于 `multi_thread` 运行时

这个线程本地层是设置在调用了 `fake()` 的那个 OS 线程上的。一个 `multi_thread` 的 tokio 运行时可能会在一次 `.await` 之间，把您的 future 迁移到另一个工作线程上，于是这个覆盖就悄悄消失了。`TestContainer::scope` 通过把这个覆盖绑定到这个 future 本身来解决这个问题：

```rust
use suprnova::testing::TestContainer;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cross_worker_safe() {
    TestContainer::scope(async {
        TestContainer::bind::<dyn HttpClient>(Arc::new(FakeHttpClient::new()));
        do_async_work_that_may_hop_workers().await;
    })
    .await;
}
```

被 `tokio::spawn` 出来的子任务不会继承 tokio 的任务本地状态；请改用 `TestContainer::spawn` - 它会捕获当前作用域的容器，并在这个被 spawn 出的 future 内部重新装上它：

```rust
TestContainer::scope(async {
    TestContainer::bind::<dyn HttpClient>(Arc::new(FakeHttpClient::new()));
    let h = TestContainer::spawn(async {
        App::make::<dyn HttpClient>().unwrap()  // 能看到这个伪造实现
    });
    let _client = h.await.unwrap();
})
.await;
```

### 为什么会有一个 `FAKE_GUARDS` 引用计数

这个线程本地容器是逐测试的，但 Suprnova 还有一个按名字建键（`__read_replica__`，自定义的连接标签）的进程全局 `ConnectionRegistry`，它能在一次线程本地重置之后存活下来。一个天真的 `Drop` 实现，会在*任何* `TestContainerGuard` 消失时都调用 `ConnectionRegistry::clear()` - 这会在另一个并发测试运行到一半时，抹掉它的具名连接。

修复办法是一个进程范围的 `AtomicUsize`（`FAKE_GUARDS`）。`fake()` 会让它加一；`drop` 会让它减一；只有回落到零的那次转变才会清空这个具名注册表。两个使用 `__read_replica__` 的并行测试是安全的：不管哪一个 守卫 最后被丢弃，清空的权利都归它。

您不会从一个测试里调用这个东西 - 它是从 `TestContainerGuard` 的 `Drop` 里运行的。只有在您正在调试一个“具名连接在测试中途消失”的症状时，才需要知道它的存在，这通常意味着一个兄弟测试忘了先等自己的 守卫 被丢弃。

## 加密密钥测试辅助工具

那些练习加密类型转换（在一个 `#[model(...)]` 上的 `casts = { secret = AsEncrypted }`）、签名负载，或者密钥环的旧密钥回退的测试，需要在进程内装上一个 `APP_KEY`。框架在 `testing` 这个 feature 下提供了两个仅供测试使用的辅助函数：

```rust
use suprnova::testing::install_test_encryption_key;

#[tokio::test]
async fn cast_roundtrip() {
    install_test_encryption_key();   // 幂等；确定性的 32 字节全零密钥
    let db = TestDatabase::sqlite_memory().await.unwrap();
    // … 加密 + 读回来 …
}
```

`install_test_encryption_key` 是幂等的 - 底层的 `Crypt` 门面是由 `OnceLock` 支撑的，所以第二次调用是一个空操作。大多数类型转换测试的二进制文件，会在每一个会碰到加密类型转换的测试里调用它；第一次胜出，其余的都不用付出代价。

对于轮换测试（在旧密钥下写入，在新密钥下读取），请使用密钥环变体：

```rust
use suprnova::crypto::EncryptionKey;
use suprnova::testing::install_test_encryption_keyring;

let new = EncryptionKey::from_base64("...").unwrap();
let old = EncryptionKey::from_base64("...").unwrap();
let installed = install_test_encryption_keyring(new, vec![old]);
assert!(installed, "first install wins");
```

这个密钥环辅助函数只有在这次调用真的装上了这个密钥环时（这个 `OnceLock` 之前是空的）才会返回 `true`。要在一次轮换测试里，用一个任意的密钥铸造密文，请使用 `suprnova::crypto::_test_encrypt_with`，而不要装两次。

这两个辅助函数在 crypto 层都是 `#[doc(hidden)]` 的，并在 `testing` 模块下重新导出 - 它们仅供测试使用，并且会绕过生产环境的 `APP_KEY` 校验路径。

## `testing` feature 与生产构建

`suprnova` 把它的测试辅助函数（`Storage::fake()`、`TestContainer`、`TestDatabase`，以及 `_test_install_key` 这类密钥轮换钩子）暴露在一个名为 `testing` 的 Cargo feature 后面。这个 feature 在默认集合里，所以引用它的测试套件不费力就能拿到它们：

```toml
[dependencies]
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.2.4" }

[dev-dependencies]
# `testing` 通过上面那条依赖传递性地开启了 - 不需要额外写什么。
```

这些钩子都是 `#[doc(hidden)]` 的，并带着 `_test_` 前缀，所以即使这个 feature 开着，惯用的应用代码也够不着它们。真正承重的那道防护是 `Server::from_config`：它在**每一次**启动时都会校验 `APP_KEY`，而不只是在密钥环尚未初始化时才校验。一把预先装好的测试密钥没法绕过这项检查 - 无论进程里有没有什么东西预先装过一把密钥，只要 `APP_KEY` 缺失或格式有误，启动就会快速失败。

如果您更希望这些辅助函数根本不被链接进您的生产产物（纵深防御），那就把 `suprnova` 的默认 feature 关掉来依赖它，只启用您要发布的那些：

```toml
[dependencies]
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.2.4", default-features = false, features = ["..."] }

[dev-dependencies]
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.2.4", features = ["testing", "..."] }
```

这是一次收紧，不是一次修复 - 不管您选哪种姿态，真正堵住那个可利用点的都是启动校验。

### 为什么 Suprnova 有所不同

Laravel 的 PHP 测试装置几乎不费力就拿到了并行测试的隔离，因为它的运行时对每个请求都是单线程的，而且测试会为每个文件 fork 一个新进程。Suprnova 的测试二进制文件则是一个进程，在一个或多个工作线程上并发运行许多 `#[tokio::test]`。一个单一的全局容器，就意味着两个测试一旦在某个工作线程上重叠，其中一个测试的伪造实现就会渗进另一个测试的查找里。

这就是 `TestContainer` 两种口味都有的原因 - 线程本地用于常见的 `current_thread` 情形，任务本地用于 `multi_thread`。进程级全局的 `ConnectionRegistry` 上那个带引用计数的 `FAKE_GUARDS` 清理逻辑，存在的理由也一样：没法做成逐测试的共享状态，至少得知道在另一个测试还在依赖它的时候不要把自己抹掉。

匹配器目录（`expect!`）是类型化的，因为 Rust 允许它这样。Jest 的 `expect(x).toBeSome()` 只有在运行时才知道 `x` 是不是一个 `Option`；Suprnova 的 `Expect<T>` 在编译期就知道，所以用错匹配器是一个构建错误，而不是一个不稳定的测试。

## 每一部分位于何处

| 组成部分 | 源码位置 |
|---|---|
| `#[suprnova_test]` 属性宏 | `suprnova-macros/src/suprnova_test.rs` |
| `describe!` / `test!` 过程宏 | `suprnova-macros/src/describe.rs`、`test_macro.rs` |
| `expect!` 宏 + `Expect<T>` 匹配器 | `framework/src/lib.rs`（宏）、`framework/src/testing/expect.rs`（实现） |
| `TestDatabase::fresh` / `sqlite_memory` / 辅助函数 | `framework/src/database/testing.rs` |
| `test_database!` 宏 | `framework/src/database/testing.rs` |
| `TestContainer` + `TestContainerGuard` + `FAKE_GUARDS` | `framework/src/container/testing.rs` |
| `install_test_encryption_key[ring]` | `framework/src/testing/mod.rs` |
| 逐表面的伪造实现（Mail、Notify、Queue、Bus、Events、Storage、HTTP） | 逐领域的 `testing` 子模块 - 参见[模拟](mocking.md) |
| `TestResponse` | `framework/src/testing/response.rs` |
| `AssertableInertia`、`ReloadRequest` | `framework/src/testing/inertia.rs` |

## 运行测试

标准的 cargo 调用方式同样适用：

```bash
# 整个工作空间
cargo test --workspace

# 一个 crate
cargo test -p suprnova

# 按名字过滤一个测试（子串匹配）
cargo test create_user_persists_it

# 带 println! 和 dbg! 的输出
cargo test -- --nocapture
```

Suprnova 不会自带一个测试运行器；框架和 cargo 的测试运行器集成在一起。数据库测试默认并行运行 - 线程本地容器和逐测试的内存 SQLite，正是为此而设计的。

## 下一步

- [HTTP 测试](http-tests.md) - 通过 `handle_request` 驱动完整的请求管道
- [数据库测试](database-testing.md) - `TestDatabase`、测试里的工厂、测试里的填充器、并行安全的数据库测试
- [模拟和伪造](mocking.md) - 七个外部表面的伪造实现，以及它们共享的那些模式
- [服务容器](container.md) - `TestContainer` 会覆盖的那个三层查找
- [错误模型](error-model.md) - 您会用来做断言的那些 `FrameworkError` 形态
