# 数据库

Suprnova 的数据库层，用一个 Laravel 形状的 `DB` 门面包住了 SeaORM：原始查询脱围方法、一个不带模型的查询构造器、带保存点和死锁重试的事务、给读副本和分片用的连接注册表，以及一整套镜照 Laravel 13 `DB::listen` / `QueryExecuted` / 查询日志 API 的可观测性表面。

Eloquent ORM（`use suprnova::eloquent::*`）建立在这一层之上，活在 [eloquent.md](eloquent.md) 里。当您想要一个类型化的模型时，去那里；当您想对一张没建模的表做原始查询，或者想观测框架运行的每一次查询时，这一页就是您要的。

## 配置

```rust
use suprnova::{Config, DB, DatabaseConfig};

// 在 bootstrap.rs 中
Config::register(DatabaseConfig::from_env());
DB::init().await.expect("DB::init failed");
```

`DatabaseConfig::from_env` 会读取 `DATABASE_URL`，以及（可选地）连接池的可调参数 `DB_MAX_CONNECTIONS`、`DB_MIN_CONNECTIONS`、`DB_CONNECT_TIMEOUT`、`DB_LOGGING`。当 `DATABASE_URL` 未设置时，这份配置会回退到 `sqlite://./database.db` - 这对零配置的开发来说很方便；生产环境的启动会通过 `validate_for_environment` 拒绝这个回退，这样您就不会在 `APP_ENV=production` 时意外发布一个 SQLite 文件。

URL → 驱动程序检测：

```text
postgres://user:pass@host/db       → DatabaseType::Postgres
postgresql://user:pass@host/db     → DatabaseType::Postgres
mysql://user:pass@host/db          → DatabaseType::Mysql
sqlite://./file.db                 → DatabaseType::Sqlite
sqlite::memory:                    → DatabaseType::Sqlite
```

## 原始查询

`DB` 门面发布了完整的 Laravel 13 原始脱围表面。每一个助手方法都会经过同一个装配了埋点的执行器 - 每一次调用都会触发 `QueryExecuted`（参见[可观测性](#可观测性)）。

绑定是 `sea_orm::Value` - 这是框架有意不重新包装的少数几个 sea_orm 类型之一，因为每一个抵达线路的值都要经过它。`Value::from(...)` 对数据库能理解的每一种基元类型都管用。

```rust
use suprnova::DB;
use sea_orm::Value;

// SELECT - 全部行，作为 DynamicRow。
let users = DB::select(
    "SELECT * FROM users WHERE active = ?",
    vec![Value::from(true)],
).await?;

// SELECT - 只要第一行。
let alice = DB::select_one(
    "SELECT * FROM users WHERE name = ?",
    vec![Value::from("alice")],
).await?;

// SELECT - 第一行第一列，作为一个类型化的值。
let count: i64 = DB::scalar(
    "SELECT COUNT(*) FROM users",
    vec![],
).await?;

// INSERT - 返回 bool（至少一行受到影响时为 true）。
DB::insert(
    "INSERT INTO users (name, active) VALUES (?, ?)",
    vec![Value::from("bob"), Value::from(true)],
).await?;

// UPDATE / DELETE - 返回受影响的行数。
let updated = DB::update(
    "UPDATE users SET active = ? WHERE id = ?",
    vec![Value::from(false), Value::from(1)],
).await?;
let deleted = DB::delete(
    "DELETE FROM users WHERE active = ?",
    vec![Value::from(false)],
).await?;

// 任何带绑定的预处理语句。
DB::statement(
    "UPDATE users SET votes = votes + ? WHERE id = ?",
    vec![Value::from(1), Value::from(42)],
).await?;

// DDL，不带绑定 - `unprepared` 对应的是 Laravel 的 `DB::unprepared`，
// 用于那些拒绝占位符绑定的语句（CREATE INDEX、ALTER TABLE、VACUUM）。
DB::unprepared("CREATE INDEX idx_users_name ON users(name)").await?;

// affecting_statement 是 update/delete 内部使用的那个显式形态 -
// 对那些两个名字都不适合的操作（例如 INSERT...ON CONFLICT DO UPDATE），
// 直接跌落到它。
let affected = DB::affecting_statement(
    "INSERT INTO users (id, name) VALUES (?, ?) ON CONFLICT(id) DO UPDATE SET name = excluded.name",
    vec![Value::from(1), Value::from("alice")],
).await?;
```

### 占位符语法

SQLite 加 MySQL 用 `?`。Postgres 用 `$1`、`$2`、……。活跃的后端是从 `DatabaseConfig::url` 自动检测出来的。

### DynamicRow

无类型的行会具体化成 `DynamicRow` - 一个带类型化访问器的 `serde_json::Map` newtype：

```rust
for row in users {
    let id: i64 = row.get_int("id")?;
    let name: String = row.get_string("name")?;
    let nickname: Option<String> = row.get_optional_string("nickname")?;
    let score: Option<i64> = row.get_optional_int("score")?;
    // 反序列化任意一个 T（chrono::DateTime、您自己的结构体，等等）：
    let prefs: UserPrefs = row.get_as("prefs")?;
}
```

`get_*` 在列缺失*或*为 null 时都会报错。`get_optional_*` 只在列缺失时报错，对 SQL NULL 会返回 `Ok(None)`。完整的访问器列表是 `get_int` / `get_string` / `get_bool` / `get_float` / `get_value` / `get_as<T>`，再加上 `get_optional_string` / `get_optional_int`；对于没有专用 `get_optional_*` 的可空类型，伸手去用 `get_value` 加一次 `serde_json::Value` 匹配，或者用 `get_as::<Option<T>>`。

## 不带模型的查询构造器 - `DB::table`

对那些您还没有费心用 `#[suprnova::model]` 建模的表做临时查询时，`DB::table(...)` 返回一个形态和 Eloquent `Builder<M>` 一样的可链式构造器，但会把行具体化成 `DynamicRow`：

```rust
use suprnova::{DB, attrs};

let rows = DB::table("audit_log")
    .select(["id", "event", "actor_id"])
    .filter("actor_id", 42i64)
    .filter_op("created_at", ">=", "2025-01-01")
    .order_by_desc("id")
    .limit(50)
    .get()
    .await?;

let first = DB::table("audit_log")
    .filter("event", "user.deleted")
    .first()
    .await?;

let count = DB::table("audit_log")
    .filter("actor_id", 42i64)
    .count()
    .await?;

let id = DB::table("audit_log")
    .insert(attrs! { event: "user.created", actor_id: 42 })
    .await?;

let updated = DB::table("audit_log")
    .filter("id", id)
    .update(attrs! { event: "user.created.v2" })
    .await?;

let deleted = DB::table("audit_log")
    .filter("actor_id", 42i64)
    .delete()
    .await?;
```

### 标识符上的信任边界

表名、列名、ORDER BY 方向，以及 SQL 运算符，都是逐字插值进 SQL 字符串的 - 它们**不会**被绑定为参数（SQL 不允许对标识符做占位符绑定）。请把每一个 `impl Into<String>` 参数都当作一个**受信任**的字面量：

```rust
// 安全 - 列名是一个常量。
DB::table("users").filter("email", request.email()).get().await?;

// 不安全 - 永远不要把用户输入拼进一个列名里。
DB::table("users").filter(&request.column_name(), value).get().await?;
```

值（`filter` / `filter_op` 的右手侧）**确实会**被绑定为参数，对用户输入是安全的。

框架对标识符（`[A-Za-z_][A-Za-z0-9_]*`，可以带一个可选的 `schema.` 前缀）和运算符（`=`、`<>`、`<`、`<=`、`>`、`>=`、`LIKE`、`NOT LIKE`、`ILIKE`、`NOT ILIKE`、`IS`、`IS NOT`）都强制执行一份严格的允许列表。违规会在 SQL 字符串被渲染之前，就在 I/O 边界上报错。

## 事务

三个入口点，每一个都接好了 `QueryExecuted` / `TransactionBeginning` / `TransactionCommitted` / `TransactionRolledBack` 这些观测钩子。

### 闭包形式

```rust
use suprnova::DB;

DB::transaction(|_tx| {
    Box::pin(async move {
        let mut alice = User::query().filter("name", "alice").first_or_fail().await?;
        alice.balance -= 30;
        alice.save().await?;

        let mut bob = User::query().filter("name", "bob").first_or_fail().await?;
        bob.balance += 30;
        bob.save().await?;
        Ok::<(), suprnova::FrameworkError>(())
    })
}).await?;
```

`Ok(_)` 时提交。`Err(_)` 时回滚并把这个错误传播出去。

闭包内部的操作会通过一个 `tokio::task_local` 自动拿到那个活跃的事务 - 您**不需要**在每一次模型调用里都穿针引线地传一个 `&tx` 句柄。嵌套的 `DB::transaction` 会返回一个数据库错误；嵌套回滚行为请用 `tx.savepoint(...)`。

对于必须在同一个被钉住的连接上执行的类型化聚合或自定义 SQL，请直接使用这个事务句柄：

```rust
use sea_orm::{DbBackend, Statement};

DB::transaction(|tx| {
    Box::pin(async move {
        let backend = tx.backend();
        let rows = tx.query_all(Statement::from_string(
            backend,
            "SELECT CAST(COUNT(*) AS BIGINT) AS total FROM orders".to_owned(),
        )).await?;
        let total = rows[0].try_get::<i64>("", "total")?;
        Ok::<_, suprnova::FrameworkError>(total)
    })
}).await?;
```

`query_all` 会发出正常的 `QueryExecuted` 观测，返回类型化的 SeaORM `QueryResult` 行。对动态的值请用带绑定的 `Statement::from_sql_and_values`；不要插值不受信任的输入。

### 死锁重试

```rust
DB::transaction_with_attempts(5, |_tx| {
    Box::pin(async move {
        // 和上面一样的闭包方法体。在 SQLSTATE 40001 / 40P01，
        // 或者任何包含“deadlock”的错误（不区分大小写）上，
        // 会从头重新运行。
        Ok::<(), suprnova::FrameworkError>(())
    })
}).await?;
```

### 手动形式

```rust
use suprnova::{DB, attrs};

let tx = DB::begin_transaction().await?;

// 逐模型：这些 `*_with_tx` 垫片会把一次 CRUD 操作钉在这个手动事务上。
User::create_with_tx(&tx, attrs! { name: "alice" }).await?;
Order::create_with_tx(&tx, attrs! { user_id: 1, total: 30 }).await?;

// 逐查询：`Builder::with_tx(&tx)` 会把一条构造器链钉住。
let stale = Order::query()
    .filter("status", "pending")
    .with_tx(&tx)
    .get()
    .await?;

if some_condition() {
    tx.rollback().await?;
} else {
    tx.commit().await?;
}
```

手动模式**不会**安装这个任务本地 - 每一个应该在这个事务内部运行的操作都得自己选择接入，要么通过一次链式查询上的 `Builder::with_tx(&tx)`，要么通过某一个 `Model::*_with_tx` 垫片（`create_with_tx`、`save_with_tx`、`delete_with_tx`，等等）。忘了选择接入的操作，会针对全局连接池运行，**不属于**这个事务。

持有一个 `Transaction` 句柄，会在它的整个生命周期里钉住一个连接池连接；请在调用 `begin_transaction()` 之前，预先加载您需要读取的任何行，在 SQLite（单一共享连接）上尤其如此。

### 保存点

```rust
DB::transaction(|tx| {
    Box::pin(async move {
        Order::create(/* ... */).await?;

        tx.savepoint("after_order").await?;
        if let Err(e) = Payment::charge().await {
            // 丢掉这次支付尝试，但保留这个订单。
            tx.rollback_to("after_order").await?;
        }
        Ok::<(), suprnova::FrameworkError>(())
    })
}).await?;
```

全部三个一等公民的后端都支持 `SAVEPOINT` / `ROLLBACK TO SAVEPOINT` - SQLite 也包括在内。

## 可观测性

Laravel 13 的 `DB::listen` / `QueryExecuted` / 查询日志表面，通过 Suprnova 的事件派发器被移植到了 Rust。

### `DB::listen` - 直接回调

```rust
use suprnova::{DB, QueryExecuted};

// 在 bootstrap.rs 中（或一个服务提供者里）。
DB::listen(|event: &QueryExecuted| {
    tracing::debug!(
        sql = %event.sql,
        bindings = ?event.bindings,
        time_ms = event.time.as_millis(),
        connection = %event.connection_name,
        "query executed",
    );
})?;
```

监听器是**在这个执行器助手方法内部同步运行的**。一个缓慢的监听器会拖慢这次查询 - 请让直接回调保持轻量。对任何可能失败的东西，优先选用下面的 `EventFacade` 路径；它走的是 `dispatch_best_effort`，能容忍错误。

### `EventFacade` 派发路径

`QueryExecuted` 是一个真正的 `suprnova::Event` - 通过这个派发器去监听，就能拿到排队、可伪造、容错的投递：

```rust
use suprnova::{EventFacade, Listener, QueryExecuted, FrameworkError};
use std::sync::Arc;

struct LogToDatabase;

#[suprnova::async_trait]
impl Listener<QueryExecuted> for LogToDatabase {
    async fn handle(&self, event: &QueryExecuted) -> Result<(), FrameworkError> {
        // 即便这个监听器自己也在查询数据库，重入守卫也会
        // 防止无限递归。
        DB::statement(
            "INSERT INTO query_log (sql, time_ms) VALUES (?, ?)",
            vec![event.sql.clone().into(), (event.time.as_millis() as i64).into()],
        ).await?;
        Ok(())
    }
}

// 在 bootstrap.rs 中。
EventFacade::listen::<QueryExecuted, _>(Arc::new(LogToDatabase)).await;
```

走这条路径的监听器：

- 通过 `dispatch_best_effort` 运行 - 一个失败的监听器**不会**让这次查询失败。
- 当它们自己发出一次查询时会被短路（重入守卫）。
- 在测试里可以用 `Event::fake()` 来断言派发，而不真正运行监听器。

### 内存中的查询日志

```rust
DB::enable_query_log()?;

User::query().filter("active", true).get().await?;
Order::query().count().await?;

let log = DB::get_query_log()?;
for query in &log {
    println!("{} ({}ms)", query.sql, query.time.as_millis());
}

DB::flush_query_log()?;     // 丢弃条目，保持启用
DB::disable_query_log()?;   // 停止捕获
let still_capturing = DB::logging();
```

这个日志是**无边界的** - 每一次被捕获的查询都会让它继续增长，直到进程退出，`flush_query_log()` 运行，或者 `disable_query_log()` 被调用。用它来做开发，不要把它当作一个长期运行的生产环境分析器。

### 事务生命周期事件

`TransactionBeginning`、`TransactionCommitted`，和 `TransactionRolledBack` 是真正的 `suprnova::Event` 类型 - 通过 `EventFacade::listen` 去监听它们，来驱动审计、分布式锁，或者补偿逻辑。

```rust
EventFacade::listen::<TransactionCommitted, _>(Arc::new(AuditCommit)).await;
EventFacade::listen::<TransactionRolledBack, _>(Arc::new(MetricRollback)).await;
```

全部三个事务入口点（`DB::transaction` / `DB::transaction_with_attempts` / `DB::begin_transaction` + `Transaction::commit`/`rollback`）都会触发这些事件。一个泄漏的、被丢弃却没有显式提交/回滚的手动 `Transaction` 句柄不会发出任何事件 - SeaORM 的 `Drop` 实现是同步的，没法触达那个异步派发器。

### `QueryExecuted` 载荷

```rust
pub struct QueryExecuted {
    pub sql: String,
    pub bindings: Vec<String>,         // 以调试格式渲染（`{:?}`）
    pub time: std::time::Duration,
    pub connection_name: String,
    pub read_write_type: Option<ReadWriteType>,
    pub result: Result<(), String>,    // 驱动程序出错时为 Err
}
```

`to_raw_sql()` 会把捕获到的绑定替换进这条 SQL 里，供展示用：

```rust
let query = /* 从一个监听器里捕获到的 */;
println!("{}", query.to_raw_sql());
// SELECT * FROM users WHERE id = 42 AND active = true
```

这个替换是**调试格式**的（不是 SQL 安全的转义），只供日志输出使用。永远不要把这个结果再喂回一次查询。

### 覆盖范围

目前，`QueryExecuted` 会为每一次经过那些装配了埋点的 `ExecutorChoice` 助手方法的查询触发：

- `DB` 上的每一个原始助手方法（`select` / `select_one` / `scalar` / `insert` / `update` / `delete` / `statement` / `affecting_statement` / `unprepared`）。
- `DbTableBuilder`（不带模型的构造器）上的每一个终结方法。
- `DB::transaction` / `DB::begin_transaction` 的 BEGIN / COMMIT / ROLLBACK 会触发事务事件。
- `DbConnection::connect` 会触发 `ConnectionEstablished`。

Eloquent ORM（`Builder<M>::get` / `first` / `count`，模型的增删改查）今天是直接匹配 `ExecutorChoice` 的 `Tx` / `Pool` 分支，而不是经过这些装配了埋点的助手方法 - 采用这些助手方法（从而拿到这个观测钩子）的工作，落在 Eloquent 模块那一边。

## 连接元数据

```rust
let name = DB::database_name()?;        // 对 postgres://.../myapp 是 "myapp"
let driver = DB::driver_name()?;        // "postgres" | "mysql" | "sqlite"
let title = DB::driver_title()?;        // "Postgres" | "MySQL" | "SQLite"
let version = DB::server_version().await?;  // "15.5" | "8.0.36" | "3.42.0"
```

`server_version` 会发出一条后端专属的内省查询（Postgres 加 MySQL 是 `SELECT VERSION()`，SQLite 是 `SELECT sqlite_version()`）。如果您经常调用它，就缓存这个结果 - 每一次调用都是一次往返。

## 命名连接

对于读副本、被分片的分片，或者逐模型的数据仓库连接池：

```rust
// 在 bootstrap.rs 中
DB::register_named("__read_replica__", read_config).await?;
DB::register_named("warehouse", warehouse_config).await?;

// 逐查询路由：
let rows = User::query().on("__read_replica__").get().await?;
let warehouse_rows = DB::table("audit_log").on("warehouse").get().await?;
let raw = DB::select_on("warehouse", "SELECT ...", vec![]).await?;
```

`__read_replica__` 这个名字是众所周知的：一旦注册，每一个读形态的终结方法都会自动路由经过它。写操作会忽略这个副本，指向主连接。要为特定操作重新选回主连接，请用 `Builder::on_write_connection`（逐查询）或者 `#[model(connection = "...")]`（逐模型的默认值）。

保留名字：

- `__primary__` - 默认连接池。不能被注册（它是 `DB::connection()` 的返回值）。
- `__read_replica__` - 众所周知的读副本。任何注册在这个名字下的连接都会接管读路由。

完整的优先级链请参见 [eloquent.md → 多连接路由](eloquent.md#multi-connection-routing)（构造器的事务覆盖 → 环境事务 → 构造器的 `on(name)` → 模型默认值 → `__read_replica__` → 主连接）。

## 测试

`TestDatabase` 会构建一个内存中的 SQLite 数据库，把它注册进测试容器，这样 `DB::connection()` 就会解析到它，并运行您的迁移：

```rust
use suprnova::testing::TestDatabase;
use crate::migrations::Migrator;

#[tokio::test]
async fn test_user_creation() {
    let db = TestDatabase::fresh::<Migrator>().await.unwrap();
    // 任何调用 DB::connection() 的代码，现在拿到的都是这个内存数据库。
    let _ = CreateUser::run("alice@example.com").await.unwrap();
}

// `test_database!()` 是这个宏的简写形式。
let db = test_database!();
```

对于那些自己构建临时架构的测试：

```rust
let db = TestDatabase::sqlite_memory().await.unwrap();
db.execute_unprepared("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)").await.unwrap();
```

当一个 `TestDatabase` 被丢弃时，这个测试容器会被清空，这个连接注册表会被清空 - 没有跨测试的泄漏。那些会改动进程范围状态的测试（这个注册表、监听器注册表、查询日志）应该标注 `#[serial_test::serial]`，这样它们就不会互相冲撞。

## 下一步

- [Eloquent](eloquent.md) - 建立在这一层之上的、类型化的 `#[suprnova::model]` ORM
- [迁移](migrations.md) - `Migrator`、`make:migration`，以及 `db:sync` 工作流
- [数据库测试](database-testing.md) - `TestDatabase`、夹具加载，以及串行测试的标注
- [事件](events.md) - `QueryExecuted` / `TransactionCommitted` 监听器背后的那个派发器
- [配置](configuration.md) - 把 `DatabaseConfig` 和您其他的类型化配置一起注册

## 表面索引

| 表面 | Laravel 对应物 |
| --- | --- |
| `DB::init` / `DB::init_with` / `DB::connection` / `DB::is_connected` / `DB::get` | `DB::connection()` |
| `DB::table(name)` → `DbTableBuilder` | `DB::table($name)` |
| `DB::select` / `select_one` / `scalar` / `insert` / `update` / `delete` / `statement` / `affecting_statement` / `unprepared` | `DB::select` / `selectOne` / `scalar` / `insert` / `update` / `delete` / `statement` / `affectingStatement` / `unprepared` |
| `DB::transaction` / `transaction_with_attempts` / `begin_transaction` | `DB::transaction($cb, $attempts)` / `DB::beginTransaction` |
| `Transaction::commit` / `rollback` / `savepoint` / `rollback_to` | `DB::commit` / `rollBack` / 保存点助手方法 |
| `DB::listen(callback)` | `DB::listen` |
| `DB::enable_query_log` / `disable_query_log` / `get_query_log` / `flush_query_log` / `logging` | `DB::enableQueryLog` / `disableQueryLog` / `getQueryLog` / `flushQueryLog` / `logging` |
| `DB::database_name` / `driver_name` / `driver_title` / `server_version` | `getDatabaseName` / `getDriverName` / `getDriverTitle` / `getServerVersion` |
| `DB::register_named` / `named` / `select_on` / `table_on` / `statement_on` / `affecting_statement_on` | 多连接的 `DB::connection($name)` |
| `QueryExecuted` / `TransactionBeginning` / `TransactionCommitted` / `TransactionRolledBack` / `ConnectionEstablished` / `DatabaseBusy` | `Illuminate\Database\Events\*` |
| `DatabaseConfig::builder()` / `from_env` / `validate_for_environment` | `config/database.php` |
| `TestDatabase::fresh::<M>` / `sqlite_memory` / `execute_unprepared` / `fetch_one` / `fetch_all` | `RefreshDatabase` 测试 trait |
