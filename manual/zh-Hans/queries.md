# 查询构造器

当您想要查询一张表，却不想把它建模成一个类型化的 `#[suprnova::model]` 结构体时，就伸手去用 `DB::table(name)`。它会返回一个形态和类型化的 Eloquent `Builder<M>` 一样的可链式构造器，但会把行具体化成 `DynamicRow` - 一个带类型化访问器的 `serde_json::Map` newtype。这一章是写给审计日志、临时报表、仪表盘聚合数据，以及任何您还没有费心去建模的表的。想看类型化的对应物，请参见 [Eloquent](eloquent.md)。想在事务内部使用原始的 `DB::select`，或者搭配 `DB::listen` 做观测，请参见 [数据库](database.md)。

```rust
use suprnova::DB;

let rows = DB::table("audit_log")
    .select(["id", "event", "actor_id"])
    .filter("actor_id", 42i64)
    .filter_op("created_at", ">=", "2026-01-01")
    .order_by_desc("id")
    .limit(50)
    .get()
    .await?;

for row in rows.iter() {
    let id: i64 = row.get_int("id")?;
    let event: String = row.get_string("event")?;
    println!("{id}: {event}");
}
```

## 该选用哪种表面

三种查询表面互有重叠；请为这张表选出正确的那一种。

| 表的情况是… | 使用 | 返回 |
|---|---|---|
| 用 `#[suprnova::model]` 建过模 | `Model::query()` → `Builder<M>` | 类型化的 `M` 值 |
| 没建模，但您想要一个可链式的 WHERE/ORDER/LIMIT 形态 | `DB::table(name)` → `DbTableBuilder` | `DynamicRow` |
| 构造器表达不了的任何东西 - CTE、窗口函数、后端专属的 DDL | `DB::select` / `DB::statement` / `DB::affecting_statement` | `DynamicRow` / `bool` / `u64` |

`DbTableBuilder` 就是为中间这种情形存在的。您能拿到 WHERE / ORDER / LIMIT 这条链，却不必让自己绑死在一个 `#[suprnova::model]` 结构体上，也不必一路跌落到原始 SQL 字符串。

## 可链式的表面

`DB::table(name)` 返回一个 `DbTableBuilder`。把它逐步构建起来，然后调用一个终结方法来执行。

### 过滤

```rust
// 相等匹配。
DB::table("users").filter("email", "alice@example.com").get().await?;

// 任意运算符。允许列表：=, <>, <, <=, >, >=, LIKE, NOT LIKE, ILIKE, NOT ILIKE, IS, IS NOT。
DB::table("orders").filter_op("total", ">=", 100i64).get().await?;
DB::table("posts").filter_op("title", "LIKE", "%rust%").get().await?;

// 多个过滤条件之间是 AND 关系。
DB::table("audit_log")
    .filter("actor_id", 42i64)
    .filter_op("event", "<>", "noop")
    .get()
    .await?;
```

`filter` 和 `filter_op` 的右手侧都接受任何 `Into<SeaValue>`，覆盖了 `i64`、`String`、`&str`、`bool`、`f64`、`Option<T>`、`chrono::*`、`uuid::Uuid`，以及 `serde_json::Value` - 后端能理解的每一种列类型都在内。

### 选择列

```rust
// 默认是 SELECT *。
DB::table("users").get().await?;

// 只需要部分列时，就限定列。
DB::table("users").select(["id", "email"]).get().await?;
```

### 排序与取窗

```rust
DB::table("posts")
    .order_by_desc("created_at")
    .order_by_asc("title")
    .limit(20)
    .offset(40)
    .get()
    .await?;
```

`order_by_desc` 和 `order_by_asc` 会按插入顺序链接起来；生成的 SQL 会保留这个顺序。

### 终结方法

```rust
// 所有匹配的行。
let rows: Collection<DynamicRow> = DB::table("audit_log")
    .filter("actor_id", 42i64)
    .get()
    .await?;

// 第一行，或者 None。
let first: Option<DynamicRow> = DB::table("audit_log")
    .filter("event", "user.deleted")
    .first()
    .await?;

// 只要计数（渲染前会清掉任何 select/order/limit/offset - 计数语义不关心这些）。
let n: u64 = DB::table("audit_log")
    .filter("actor_id", 42i64)
    .count()
    .await?;
```

`get()` 返回的是 `Collection<DynamicRow>` - 和类型化模型用的是同一个集合包装器，带着同样的 `.iter()`、`.len()`、`.into_vec()` 表面。参见 [Eloquent 集合](eloquent-collections.md)。

### 插入、更新、删除

```rust
use suprnova::attrs;

// INSERT，返回新行的自增 id。
let id: i64 = DB::table("audit_log")
    .insert(attrs! { event: "user.created", actor_id: 42 })
    .await?;

// UPDATE，返回受影响的行数。
let updated: u64 = DB::table("audit_log")
    .filter("id", id)
    .update(attrs! { event: "user.created.v2" })
    .await?;

// DELETE，返回受影响的行数。
let deleted: u64 = DB::table("audit_log")
    .filter("actor_id", 42i64)
    .delete()
    .await?;
```

`attrs!` 宏在调用点构建这份列到值的映射。键是 SQL 标识符（经过校验），值是绑定的参数。

#### `update_all` 和 `delete_all` 别名

`update` 和 `delete` 是忠实于 Laravel 的命名。`Builder<M>` 风格的别名 - `update_all` 和 `delete_all` - 调用的是同一份实现。当调用点想表达的重点就是“这是针对整张表的”时，优先选用 `_all` 形态；它能让审阅者一眼看出这里少了一个 `filter`：

```rust
// 行为和 DB::table("rate_limits").delete().await? 一样，但 _all
// 后缀告诉审阅者“是的，我就是要清空这张表”。
DB::table("rate_limits").delete_all().await?;

// 带 WHERE 的批量更新 - 这里的 _all 后缀，
// 对应的是类型化 Builder<M> 里同一个操作的约定。
DB::table("sessions")
    .filter_op("expires_at", "<", chrono::Utc::now())
    .update_all(attrs! { status: "expired" })
    .await?;
```

#### update 或 delete 上的空 WHERE 会作用于每一行

`DB::table("x").delete().await?` 会移除这张表里的每一行。这是设计上就支持的行为 - 有时候您确实就是想清空一张表 - 但它很少是正确的。查看任何一次 `delete()` / `delete_all()` 调用时，都要检查它前面是否有一个 `filter`。`update` / `update_all` 也是同样的道理。

#### 插入操作的后端分歧

`RETURNING id` 用在 Postgres 和 SQLite 上。MySQL 不支持 `RETURNING`，所以构造器会运行这条 INSERT，再从结果里读出驱动程序逐连接的 `last_insert_id()`。这个不带模型的构造器假定的是一个标准的、自增的 `id` 主键。UUID、复合、改名，或者非整数的主键在这个表面上不受支持 - 请改用类型化的 [Eloquent](eloquent.md) `Model` 接口，它会去查阅模型定义来确定主键的形态。

## `DynamicRow` - 一个 JSON 映射上的类型化访问器

每一行由 `DB::table` 或 `DB::select` 返回时，都会具体化成 `DynamicRow`，一个带类型化访问器的 `serde_json::Map<String, Value>` newtype。每个取值方法都返回 `Result<T, FrameworkError>`，并在键缺失或类型不匹配时附上一条清晰的错误消息。

```rust
for row in rows.iter() {
    let id: i64                 = row.get_int("id")?;
    let event: String           = row.get_string("event")?;
    let active: bool            = row.get_bool("active")?;
    let weight: f64             = row.get_float("weight")?;
    let payload: serde_json::Value = row.get_value("payload")?;
}
```

对于可空的列，请用 `get_optional_*`。它们会区分“列缺失”（错误 - 架构不匹配）和“列存在，值是 SQL NULL”（`Ok(None)`）：

```rust
let title: Option<String> = row.get_optional_string("title")?;
let score: Option<i64>    = row.get_optional_int("score")?;
```

目前这个可选家族覆盖 `String` 和 `i64`。对于其他可空类型，请用 `get_value`，自己去匹配 `serde_json::Value::Null`，或者通过 `get_as::<Option<T>>`（任何 `T: DeserializeOwned`）来读取这一列。

要把一列反序列化成任何结构体或容器类型，请用 `get_as`。完整的 `serde_json` 反序列化表面都是可用的：

```rust
#[derive(serde::Deserialize)]
struct UserPrefs {
    theme: String,
    notifications: bool,
}

let prefs: UserPrefs    = row.get_as("prefs")?;
let tags: Vec<String>   = row.get_as("tags")?;
let when: chrono::DateTime<chrono::Utc> = row.get_as("created_at")?;
```

`DynamicRow` 会解引用到 `Map<String, Value>`，所以迭代和键存在性检查都能直接使用：

```rust
for (key, value) in row.iter() {
    println!("{key} = {value}");
}

if row.contains_key("deleted_at") { /* … */ }
```

## 标识符的信任边界

表名、列名、ORDER BY 方向，以及 SQL 运算符，都是逐字插值进 SQL 字符串的 - 它们**不会**被绑定为参数（SQL 不允许对标识符做占位符绑定）。请把每一个 `impl Into<String>` 参数都当作一个受信任的、编译期字面量。

```rust
// 安全 - 列名是一个常量；值是绑定的。
DB::table("users").filter("email", request.email()).get().await?;

// 不安全 - 永远不要把用户输入拼进一个列名里。
DB::table("users")
    .filter(request.user_supplied_column(), value)
    .get()
    .await?;
```

框架在 I/O 边界上强制执行一份严格的允许列表 - 标识符必须匹配 `[A-Za-z_][A-Za-z0-9_]*`，可以带一个可选的 `schema.` 前缀，运算符也必须来自一份固定的列表。违规会在任何 SQL 被渲染之前，就带着一个 `FrameworkError::Database` 失败关闭。这是一张安全网，不是一张许可证：请在您的代码里把标识符保持为字面量。

`filter` / `filter_op` 右手侧的值永远是绑定为参数的，从请求数据里拼接过来也是安全的。

## 原始查询

当构造器表达不了您需要的东西时 - 递归 CTE、窗口函数、后端专属的 DDL、`INSERT … ON CONFLICT DO UPDATE` - 就跌落到一个原始字符串。占位符要匹配活跃的后端（Postgres 用 `$1, $2, …`，MySQL 和 SQLite 用 `?`）；框架会从 `DatabaseConfig::url` 自动检测。

```rust
use suprnova::DB;
use sea_orm::Value;

// SELECT - 每一行都是 DynamicRow。
let rows = DB::select(
    "SELECT u.name, COUNT(p.id) AS post_count
     FROM users u LEFT JOIN posts p ON p.user_id = u.id
     GROUP BY u.id
     HAVING COUNT(p.id) > ?",
    vec![Value::from(5i64)],
).await?;

// SELECT - 只要第一行，对应 Laravel 的 DB::selectOne。
let alice = DB::select_one(
    "SELECT * FROM users WHERE email = ?",
    vec![Value::from("alice@example.com")],
).await?;

// SELECT - 第一行第一列，作为一个类型化的标量。
let total: i64 = DB::scalar(
    "SELECT COUNT(*) FROM users WHERE active = ?",
    vec![Value::from(true)],
).await?;

// INSERT - 只要有一行受到影响就返回 true。
DB::insert(
    "INSERT INTO users (name, active) VALUES (?, ?)",
    vec![Value::from("bob"), Value::from(true)],
).await?;

// UPDATE / DELETE - 返回受影响的行数。
let updated: u64 = DB::update(
    "UPDATE users SET active = ? WHERE id = ?",
    vec![Value::from(false), Value::from(1i64)],
).await?;

let deleted: u64 = DB::delete(
    "DELETE FROM users WHERE active = ?",
    vec![Value::from(false)],
).await?;

// 任何带绑定的预处理语句。
DB::statement(
    "UPDATE users SET votes = votes + ? WHERE id = ?",
    vec![Value::from(1i64), Value::from(42i64)],
).await?;

// DDL，或者其他拒绝占位符绑定的、不带绑定的语句。
DB::unprepared("CREATE INDEX idx_users_name ON users(name)").await?;

// 通用的“受影响行数”路径 - 用于 upsert 和其他不适合那些
// 具名助手方法的操作。
let n: u64 = DB::affecting_statement(
    "INSERT INTO counters (k, n) VALUES ($1, 1)
     ON CONFLICT (k) DO UPDATE SET n = counters.n + 1",
    vec![Value::from("page_views")],
).await?;
```

### 聚合列陷阱

像 `SELECT COUNT(*) AS n FROM t` 这样无类型的聚合，通过构造器的 `.count()` 助手能正常工作，但在 SQLite 上，从原始的 `DB::select` 行里读回来时可能会悄悄丢失。底层的行具体化器会遍历 sqlx 逐列的类型信息，而一个裸的聚合不带任何类型信息。如果您需要在 SQLite 上对原始的 `DB::select` 使用聚合，就把这个表达式包进 `CAST(… AS BIGINT)` 给它一个类型标签，或者用 `DB::scalar::<i64>`，它走的是 `query_one` + `try_get`，不依赖逐列的类型检测。

## 通往类型化 Eloquent 的桥梁

当这张表值得用一个 `#[suprnova::model]` 结构体来建模时，这个可链式的形态会延续过去。`Model::query()` 返回 `Builder<M>`，它带着同样的 `filter` / `filter_op` / `order_by_*` / `limit` / `offset` / `get` / `first` / `count` 表面 - 再加上一套宽得多的 WHERE 词汇（`filter_in`、`filter_between`、`filter_null`、`filter_has`、`filter_raw`，……）以及 Laravel 形态的别名（`db_where`、`where_in`、`where_between`、`where_null`、`where_has`、`where_raw`，……）。

```rust
use suprnova::Model;

let admins = User::query()
    .filter("role", "admin")
    .filter_op("created_at", ">=", since)
    .order_by_desc("created_at")
    .limit(20)
    .get()
    .await?;     // Collection<User> - 类型化的，不是 DynamicRow

let alice = User::query().filter("email", &email).first().await?;
let total = User::query().filter("active", true).count().await?;
// 注意：Builder<M>::count 返回 i64（与 Laravel 的 Eloquent 一致），
// 而 DbTableBuilder::count 返回 u64。两个表面给您的都是一个
// 非负的 SQL COUNT - 它们只是在线路类型上有差异。
```

完整的 `Builder<M>` 表面 - 每一种 WHERE 形态、聚合、关系、预加载、作用域、分页器、分块迭代 - 都在 [Eloquent](eloquent.md) 里。您在上面学到的这个可链式形态是同一个形态；差异只在于类型化程度和覆盖面。

## 路由到一个具名连接

`DB::table` 和这些原始助手方法默认使用主连接。要指向一个读副本、分片，或数据仓库连接池，就把这次调用钉住：

```rust
// 构造器钉在一个具名连接上。
let rows = DB::table("audit_log").on("warehouse").get().await?;

// 等价的简写形式。
let rows = DB::table_on("warehouse", "audit_log").get().await?;

// 原始的脱围方法也有 _on 变体。
let rows = DB::select_on("warehouse", "SELECT …", vec![]).await?;
let n    = DB::affecting_statement_on(
    "warehouse",
    "UPDATE …",
    vec![],
).await?;
```

当 `__read_replica__` 已被注册时，每一个读形态的终结方法都会自动路由经过它；写操作（`insert` / `update` / `delete` / `update_all` / `delete_all`）永远指向主连接。在一个 `DB::transaction` 闭包内部，活跃事务的连接会绝对优先 - `on(name)` 会被静默忽略，以保住原子性。完整的优先级链请参见[数据库 - 命名连接](database.md)。

### 为什么 Suprnova 有所不同

Laravel 的 `DB::table(...)` 是它那个不带模型的查询构造器；其底层每一行返回的是一个 `stdClass`（一个属性即列的 PHP 对象）。Suprnova 返回的是 `DynamicRow` - 一个带类型化访问器的 `serde_json::Map` newtype。这种访问器形态会在边界上就捕获列缺失和类型错误的问题，而不是让它在用户代码深处以一个属性访问异常的形式 panic。

`update`/`update_all` 和 `delete`/`delete_all` 这两对名字之所以并存，是因为类型化的 Eloquent `Builder<M>` 表面用 `_all` 后缀来让针对整张表的意图在调用点变得明确。与其挑一边站，这个不带模型的构造器把两者都发布了出来 - `update` 和 `delete` 逐字匹配 Laravel 的 `DB::table($t)->update(...)` 和 `->delete()`；`update_all` 和 `delete_all` 匹配的是用户在 `M` 上早已养成的那套肌肉记忆。

## 下一步

- [数据库](database.md) - `DB` 门面、带保存点的事务、`DB::listen` 可观测性、具名连接
- [Eloquent](eloquent.md) - 类型化的 `#[suprnova::model]` 结构体和完整的 `Builder<M>` 表面
- [分页](pagination.md) - 类型化构造器上的 `paginate` / `simple_paginate` / `cursor_paginate`
- [Eloquent 集合](eloquent-collections.md) - 两个表面上 `get()` 都会返回的那个 `Collection<T>`
- [迁移](migrations.md) - 定义这些构造器所查询的那个架构
