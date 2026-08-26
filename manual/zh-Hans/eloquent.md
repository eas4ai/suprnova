# Eloquent API

Suprnova 的 Eloquent 层，为 Laravel 开发者提供了他们已经熟悉的 API，其实现只是 SeaORM 之上的一层薄封装。从 Laravel 文档里复制代码，把 PHP 语法换成 Rust，加上 `.await?`，就能跑起来。

整个这一层，就是一个结构体属性（`#[suprnova::model]`）、一个 trait（`Model`），外加一个可链式调用的查询构造器（`Builder<M>`）- 仅此而已。在幕后，这个宏会生成一个 SeaORM 的 `Entity`、`Model`、`ActiveModel`，以及一个 `Column` 枚举，再加上每一个 Eloquent trait 的实现。SeaORM 的类型仍然可以直接触达，用于 Eloquent 表面覆盖不到的少数场景（参见[SeaORM 脱围机制](#落到-seaorm)）。

## 目录

- [快速上手](#快速上手)
- [`#[suprnova::model]` 属性](#suprnova-model-属性)
- [模型模块布局](#模型模块布局)
- [查找行](#查找行)
- [创建与更新](#创建与更新)
- [删除与软删除](#删除与软删除)
- [查询构造器 - 双 API](#查询构造器-双-api)
- [行锁](#行锁)
- [事务](#事务)
- [作用域](#作用域)
- [关系](#关系)
- [预加载](#预加载)
- [分页](#分页)
- [分块与惰性迭代](#分块与惰性迭代)
- [集合](#集合)
- [批量赋值](#批量赋值)
- [转换](#转换)
- [访问器与修改器](#访问器与修改器)
- [时间戳](#时间戳)
- [观察者与生命周期事件](#观察者与生命周期事件)
- [可修剪](#可修剪)
- [多连接路由](#多连接路由)
- [复制](#复制)
- [调试 - dump 与 dd](#调试-dump-与-dd)
- [测试模型](#测试模型)
- [落到 SeaORM](#落到-seaorm)
- [从 `database::Model` 迁移](#从-database-model-迁移)
- [`DB` 门面 - 无模型查询](#db-门面-无模型查询)
- [Laravel 13 对等 - 关系存在性 + 轻量捷径](#关系存在性-轻量捷径)

## 快速上手

结构体上的一个属性，就能把它变成一个功能完备的 Eloquent 模型：

```rust
use chrono::{DateTime, Utc};
use suprnova::{model, Model};

#[model(table = "users")]
pub struct User {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

声明完之后，您就可以这样写：

- `User::query()` - 启动一个流式查询构造器。
- `User::find(id).await?` - 按主键获取。
- `User::find_or_fail(id).await?` - 同上，但找不到时会带着 `ModelNotFound` 报错。
- `User::all().await?` - 取出每一行。
- `User::create(attrs!{ name: "Alice", email: "alice@example.com" }).await?` -
  插入，并经过批量赋值过滤。
- `User::filter("email", "alice@example.com").first().await?` -
  匹配上的那一行。
- `user.update(attrs!{ name: "Alice B" }).await?` - 局部更新。
- `user.save().await?` - 持久化内存中的改动。
- `user.delete().await?` - 移除这一行。
- `user.refresh().await?` / `user.fresh().await?` / `user.replicate().await?` -
  Laravel 生命周期里剩下的那些方法。

这个面向用户的结构体（这里是 `User`）**就是**您的处理程序和控制器所携带的那个类型。这个宏会生成一个逐模型的内部模块（`user::`），里面放着 SeaORM 的 `Entity`、`Column`、`ActiveModel` 和 `Model` 类型，用于您想直接落到 SeaORM 上的场景。这个结构体还会注册进一个由 inventory 支撑的 `ModelEntry`，这样管理端和工具代码就能在启动时枚举出每一个模型。

## `#[suprnova::model]` 属性

这是声明一个模型的唯一入口。每一个属性都是可选的；默认值经过了调校，让一个带着 `id` + `created_at` + `updated_at` 的结构体，零配置就能当一个 Suprnova 模型来用。

### 宏属性参考

| 属性 | 类型 | 默认值 | 说明 |
|-----------|------|---------|-------|
| `table` | 字符串 | 结构体名称的 snake_case 复数形式 | 覆盖表名 |
| `primary_key` | 字符串 | `"id"` | 覆盖主键列名 |
| `key_type` | 类型 | `i64` | 主键类型 - UUID 用 `String`，遗留架构用 `i32` |
| `auto_increment` | 布尔值 | `true` | UUID 主键请禁用 |
| `connection` | 字符串 | `"default"` | 多连接应用用它来指名一个非默认连接 |
| `fillable` | 字符串列表 | （默认 = `guarded = ["id"]`） | 批量赋值的允许列表 |
| `guarded` | 字符串列表 | 两者都未设置时为 `["id"]` | 批量赋值的拒绝列表（和 `fillable` 互斥） |
| `casts` | `field = CastType` 映射 | `{}` | 逐列的转换 |
| `hidden` | 字符串列表 | `[]` | 从 `to_json` / `to_array` 中排除 |
| `visible` | 字符串列表 | （全部） | `hidden` 的允许列表版本（两者互斥） |
| `appends` | 字符串列表 | `[]` | 序列化时要包含进来的访问器 |
| `soft_deletes` | 标志 | `false` | 启用 `deleted_at` 列，以及墓碑语义 |
| `soft_deletes_column` | 字符串 | `"deleted_at"` | 覆盖软删除列名 |
| `timestamps` | 标志 / 布尔值 | 当 `created_at` 和 `updated_at` 都存在时为 `true` | 禁用自动管理的时间戳 |
| `created_at` | 字符串 | `"created_at"` | 覆盖列名 |
| `updated_at` | 字符串 | `"updated_at"` | 覆盖列名 |
| `touches` | 关系名称列表 | `[]` | 在此模型被创建、保存、更新或删除后，其父行的 `updated_at` 会被提升的 `BelongsTo` 关系 |
| `mutators` | 字符串列表 | `[]` | 这些字段名的 JSON 填充路径，会路由经过一个 `set_<field>(value)` 修改器方法 |

### 完整示例

```rust
use chrono::{DateTime, Utc};
use serde_json::Value as Json;
use suprnova::{model, AsBool, AsEncrypted, AsJson};

#[model(
    table = "users",
    fillable = ["name", "email", "preferences"],
    casts = {
        active = AsBool,
        preferences = AsJson<Json>,
        api_token = AsEncrypted,
    },
    hidden = ["password", "remember_token", "api_token"],
    appends = ["full_name"],
    soft_deletes,
    timestamps,
)]
pub struct User {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub password: String,
    pub remember_token: Option<String>,
    pub api_token: Option<String>,
    pub active: bool,
    pub preferences: Json,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}
```

### 函数级宏

函数级宏和这个结构体属性搭配使用：

- 在一个 `fn name(&self) -> T` 上标注 `#[accessor]`，就会让它变成一个
  Eloquent 访问器。当 `name` 被列进 `appends = [...]` 时，模型的
  `to_array()` 就会调用它（`to_json()` 则经由 `to_array` → 字符串这条委托路径拿到它）。
- 在一个 `fn set_name(&mut self, value: serde_json::Value)` 上标注
  `#[mutator]`，就会让它变成一个 Eloquent 修改器。当 `name` 被列进
  `mutators = [...]` 时，模型的 JSON 填充路径就会路由经过它。
- 在一个 `impl Model { ... }` 块上标注 `#[suprnova::scopes(Model)]`：每一个签名匹配 `fn name(query: Builder<Self>[, args…]) -> Builder<Self>`
  的方法，都会同时变成 `Builder<Self>` 上一个可链式调用的
  `.scope_name(args)`，以及一个 `Model::scope_name(args)` 快捷方式。没有函数级的 `#[scope]` 形式 - 作用域是按 impl 块声明的。
- 全局作用域是通过 `GlobalScope` trait 做的一次运行时注册，经由
  `Model::global_scope::<GS>()` 应用。没有函数级的 `#[global_scope]`
  宏 - 完整的模式请参见[宏](macros.md#suprnova-scopes-model)。
- 在一个 `impl Prunable for T { ... }` 上标注 `#[prunable]`，会经由
  inventory 注册这个修剪器，这样 `model:prune` 就能找到它。

## 模型模块布局

`#[suprnova::model]` 会把您面向用户的结构体（例如 `Post`）留在父级作用域，并生成一个和它并列的 `pub mod`，名字是这个结构体名的 snake_case 形式（`post`）。SeaORM 的那些类型，就住在这个内部模块里。

对于一个在 `app/src/models/posts.rs` 里声明的模型：

```rust
use chrono::{DateTime, Utc};
use suprnova::model;

#[model(table = "posts", fillable = ["title", "body"], timestamps)]
pub struct Post {
    pub id: i64,
    pub title: String,
    pub body: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// 约定：把这个宏在内部模块里生成的那些 SeaORM 类型重新导出，这样调用点
// 就能用不带前缀的名字。Suprnova 自己的 dogfood 模型都带着这一行
// （参见 `app/src/models/users.rs`、`app/src/models/posts.rs` 等）。
pub use post::{ActiveModel, Column, Entity};
```

现在，从 `crate::models::posts` 就能触达这些项：

| 路径 | 是什么 |
|------|-----------|
| `crate::models::posts::Post` | 您面向用户的结构体 - 这个 Eloquent 模型 |
| `crate::models::posts::post::Entity` | `posts` 表的 SeaORM `EntityTrait` 实现 |
| `crate::models::posts::post::Column` | SeaORM 的 `Column` 枚举（每一列一个变体） |
| `crate::models::posts::post::ActiveModel` | 用于插入/更新的 SeaORM `ActiveModel` |
| `crate::models::posts::post::Model` | SeaORM 形态的行（存储类型的列） |
| `crate::models::posts::{Entity, Column, ActiveModel}` | 上面那条 `pub use` 约定；不是自动生成的 |

关于这个内部模块的 `Model`，有两件事要知道：

1. 它是**SeaORM 形态**的行，不是您的 `Post` 结构体。经过转换的列，在这里携带的是它们的 `Storage` 类型（例如 `bool` 会变成底层的整数），而您结构体里的 `__eager` / `__pivot` 这两个运行时字段在这里也不存在。
2. `From<post::Model> for Post` 和 `From<Post> for post::Model` 会在这两种形态之间搭桥。往返转换的模式，请参见[落到 SeaORM](#落到-seaorm)。

`Model` 是故意**没有**被纳入这条常规的父级重新导出的 - 面向用户的
`Post` 已经在父级作用域占用了 `Post` 这个名字，而 `post::Model` 是一个独立的类型，调用者需要这个内部形态时，会通过 `post::Model`（或者
`From` 转换）来触达它。

### 何时该伸手去用这个内部模块

Eloquent 表面（`Model` trait + `Builder<M>`）覆盖了绝大多数查询。当您需要只有 SeaORM 才有的功能时，就伸手去用 `post::*`：

- **原始查询构造** - 当 Eloquent 没有暴露您想要的那个辅助方法时，用 SeaORM 的 `EntityTrait::find()` 链。
- **自定义 join 逻辑** - 针对一个 Eloquent 的 `with(...)` 没有建模的关系，经由 `QuerySelect::join()` 显式构建 `JoinType::*` join。
- **SeaORM 原生子查询** - 通过 `Entity::find().select_only()`。
- **朴素的 `ActiveModel` 变更** - 用于您想绕开 Eloquent 生命周期（没有观察者，没有自动时间戳）的少数场景。

```rust
// 常见情况 - Column 经由上面那条 `pub use post::{...}` 约定，
// 在父级模块层面被重新导出。
use crate::models::posts::Column;

let drafts = Post::query()
    .db_where(Column::Status, "draft")
    .get()
    .await?;

// 高级用户的情况 - 直接伸手进这个内部模块去拿 SeaORM 的 Entity。
// 这正是那条父级 `pub use` 没有暴露出来的东西。
use crate::models::posts::post;
use suprnova::sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

let db = suprnova::DB::connection()?;
let rows: Vec<post::Model> = post::Entity::find()
    .filter(post::Column::Status.eq("published"))
    .all(db.inner())
    .await?;

// 需要的时候，桥接回 Eloquent 的形态。
let posts: Vec<Post> = rows.into_iter().map(Post::from).collect();
```

如果您发现自己经常为了同一个操作而伸手去用这个内部模块，那就是一个信号，说明 Eloquent 缺了一个辅助方法 - 去开一个 issue，或者把这个辅助方法加到 `Model` / `Builder` 表面上。

## 查找行

```php
// Laravel
$user = User::find(1);
$user = User::findOrFail(1);          // 找不到时抛出异常
$users = User::findMany([1, 2, 3]);
```

```rust
// Suprnova
let user: Option<User> = User::find(1).await?;
let user: User = User::find_or_fail(1).await?;
let users: Vec<User> = User::find_many([1, 2, 3]).await?;
```

`find_or_fail` 会返回 `FrameworkError::ModelNotFound`（冒泡到控制器时是 HTTP 404）。

### `first_or_create` / `update_or_create` / `first_or_new` / `first_or`

```php
// Laravel
$user = User::firstOrCreate(
    ['email' => 'alice@example.com'],
    ['name' => 'Alice'],
);
$user = User::updateOrCreate(
    ['email' => 'alice@example.com'],
    ['name' => 'Alice Updated'],
);
$user = User::firstOrNew(['email' => 'alice@example.com']);  // 未保存
```

```rust
// Suprnova
let user = User::first_or_create(
    attrs! { email: "alice@example.com" },          // 用于查找的键
    attrs! { name: "Alice" },                       // create 时附加的额外字段
).await?;

let user = User::update_or_create(
    attrs! { email: "alice@example.com" },
    attrs! { name: "Alice Updated" },
).await?;

let user = User::first_or_new(
    attrs! { email: "alice@example.com" },
).await?;   // 返回一个未保存的 User；调用者自行显式保存
```

用于查找的键放在第一个 map 里；create 路径上要附加的额外字段放在第二个 map 里。通过 `first_or_new` 返回一个未保存的模型，让调用者可以在 `save().await?` 之前进一步修改它。

## 创建与更新

### 创建

```php
// Laravel
$user = User::create([
    'name' => 'Alice',
    'email' => 'alice@example.com',
]);
```

```rust
// Suprnova
let user = User::create(attrs! {
    name: "Alice",
    email: "alice@example.com",
}).await?;
```

`attrs!` 是一个会产出 `Attrs` 值（一个类型化的 JSON map）的宏。纯 JSON 也可以 - `User::create(serde_json::json!({"name": "Alice", "email": "..."}))`。`Fillable` 过滤器会在 `create` 内部运行；非 fillable 的字段会被静默丢弃，和 Laravel 的行为一致。

### 保存 / 更新

```php
// Laravel
$user->name = 'Alice B';
$user->save();

$user->update(['name' => 'Alice B']);
```

```rust
// Suprnova
user.name = "Alice B".into();
user.save().await?;

user.update(attrs! { name: "Alice B" }).await?;
```

`save()` 会遍历每一个非主键字段，经由 `Set(...)` 把它们设到这个 ActiveModel 上，调用 SeaORM 的 `update()`，再返回规范的那一行。`update(attrs)` 走的是同一条流程，但会先应用一份局部的属性 map（运行 `Fillable` 过滤器和任何已声明的修改器）。

### 递增 / 递减

```php
// Laravel
$user->increment('login_count');
$user->increment('login_count', 5);
$user->decrement('credits', 10);
User::where('plan', 'free')->increment('quota_reset_count');
```

```rust
// Suprnova
user.increment("login_count", 1).await?;
user.increment("login_count", 5).await?;
user.decrement("credits", 10).await?;
User::filter("plan", "free").increment("quota_reset_count", 1).await?;
```

`increment` / `decrement` 会发出 `UPDATE table SET col = col + N WHERE ...` 这样的 SQL - 对并发更新是原子的，不会有读改写竞态。既可以在一个已取出的模型实例上使用（WHERE 子句里用这一行的主键），也可以作为构造器上的一个终结方法使用（用这条链上的 WHERE 子句）。

### Fresh / refresh / replicate

```php
// Laravel
$user->refresh();                          // 从数据库重新加载
$user->refreshForUpdate();                 // 在一把行级锁之下重新加载
$copy = $user->fresh();                    // 获取并返回一个副本
$replica = $user->replicate();             // 未保存的克隆，带着一个全新的主键
$replica = $user->replicate(['email']);    // 跳过一个字段
```

```rust
// Suprnova
user.refresh().await?;
user.refresh_for_update().await?;
let copy: User = user.fresh().await?;
let replica: User = user.replicate().await?;
let replica: User = user.replicate_except(["email"]).await?;
```

`refresh` 是原地变更；`fresh` 返回一个另外取出来的副本。`refresh_for_update` 就是在一把 `SELECT ... FOR UPDATE` 行级锁之下的 `refresh` - 当您需要在一条语句里同时拿到这一行的当前值和那把排他锁时，请在一个事务内部用它。和 `refresh` 不一样，`refresh_for_update` 会绕过每一个注册过的全局作用域，也绕过 `#[model(soft_deletes)]` 的过滤：它连一行被丢进回收站的记录也会重新加载，而且 `deleted_at` 会带着值回来。这次重新加载是一次在锁之下的按主键查找 - 要是像给一次普通读取那样给它加上作用域，就等于给管理工具和跨租户的调用方，对一行他们本来就握着引用的记录，交回一个假的“找不到”。`replicate` 会构建一个内存中的克隆，并把主键重置（对这个键类型调用 `Default::default()`）。调用者需要自行显式保存。

当这一行已经不存在时，`refresh` 和 `refresh_for_update` 都会返回一个错误，而不是让这个模型继续握着陈旧的值。SQLite 没有行级锁，所以 `refresh_for_update` 在那里是不加锁地重新加载的 - 参见[行锁](#行锁)。

### Replicating 事件

`replicate` 和 `replicate_except` 会在构建好这个内存中的克隆之后、返回它之前，触发逐模型的 `Replicating { source, replica }` 事件。`replica` 这个字段是一个 `Arc<tokio::sync::Mutex<Self>>`，这样监听器就能在调用者看到这个副本之前先修改它 - 用于给标题加上 `(copy)` 前缀、清空标志位、重置派生列之类的场景很有用。

```rust
use suprnova::events::{EventFacade, Listener};
use async_trait::async_trait;

pub struct PrefixTitle;

#[async_trait]
impl Listener<post::events::Replicating> for PrefixTitle {
    async fn handle(&self, e: &post::events::Replicating)
        -> Result<(), FrameworkError>
    {
        let mut replica = e.replica.lock().await;
        replica.title = format!("(copy) {}", replica.title);
        Ok(())
    }
}

// 在启动时接好线一次：
EventFacade::listen::<post::events::Replicating, _>(
    std::sync::Arc::new(PrefixTitle)
).await;
```

### 跨类型复制

```rust
let replica: UserDraft = user.replicate_into().await?;  // 跨类型克隆
```

一个 Suprnova 的分歧点 - Laravel 做不到这一点，因为 PHP 没有类型。在把一个草稿模型提升为最终模型、或者反过来的时候很有用。

`replicate_into<T>` **不会**触发 `Replicating`（这个事件携带的是 `Arc<Mutex<Self>>`，所以哪怕触发了，源类型上的监听器也没法修改这个跨类型的副本）。想要逐 `T` 设置的调用者，应该在调用 `T::save` 之前，对返回的这个 `T` 自行处理 - 正常的 `Saving` / `Created` 链仍然会在 `save` 内部触发。

## 删除与软删除

### 软删除标志

把 `soft_deletes` 加到这个宏属性上，再给这个结构体加一列
`deleted_at: Option<DateTime<Utc>>`：

```rust
#[model(table = "users", soft_deletes, timestamps)]
pub struct User {
    pub id: i64,
    pub email: String,
    pub deleted_at: Option<DateTime<Utc>>,
    // ...
}
```

### 生命周期

```rust
user.delete().await?;             // UPDATE：把 deleted_at 设成 NOW()
user.trashed();                   // -> true
let trashed = User::with_trashed().find(user.id).await?.unwrap();
trashed.restore().await?;         // UPDATE：把 deleted_at 设成 NULL

let only_dead = User::only_trashed().get().await?;
let all_including_dead = User::with_trashed().get().await?;

user.force_delete().await?;       // 真正的 DELETE
```

### 默认作用域

当设置了 `soft_deletes` 时，这个宏会覆盖 `Model::query()`，让默认的读取自动过滤掉被丢弃的行。`with_trashed()` 和 `only_trashed()` 可以重新选择加入。具体来说：`User::find(id)` 会跳过被丢弃的行；`User::with_trashed().find(id)` 则会找到它们。

## 查询构造器 - 双 API

`Builder<M>` 是 `User::query()`、`User::filter(...)`、`User::db_where(...)`，以及其他每一个不会终结这条链的静态方法所返回的那个可链式调用的查询类型。

### 命名说明：双 API

`where` 是一个 Rust 关键字，所以那个纯等值判断的 where 方法，没法沿用 Laravel 的名字。这里没有去选一个赢家，而是让每一个 where 形态的方法都**同时**以一个 Rust 习惯的名字（`filter`、`filter_in`、`filter_null`、……）和一个 Laravel 形态的名字（`db_where`、`where_in`、`where_null`、……）发布。它们都是同一份规范实现之上的别名 - 用哪个，看您的肌肉记忆更习惯哪个。

```rust
// Rust 开发者：
User::query().filter("active", true).filter_in("role", ["admin"]).get().await?;

// Laravel 开发者：
User::db_where("active", true).where_in("role", ["admin"]).get().await?;

// 同一个查询。同一个结果。不同的肌肉记忆。
```

### Where 捷径

```php
// Laravel
$users = User::where('email', $email)->get();
$users = User::where('age', '>=', 18)->get();
$users = User::where('email', 'like', '%@example.com')->get();
```

```rust
// Suprnova - 选哪个系列都可以；两者都能编译，两者都有文档说明。

// Rust 形态（filter 系列）：
let users = User::query().filter("email", &email).get().await?;
let users = User::query().filter_op("age", ">=", 18).get().await?;
let users = User::query().filter_like("email", "%@example.com").get().await?;

// Laravel 形态（db_where / where_* 系列）：
let users = User::db_where("email", &email).get().await?;
let users = User::query().db_where_op("age", ">=", 18).get().await?;
let users = User::query().where_like("email", "%@example.com").get().await?;
```

### Where 变体

每一行都有两种等价的 Suprnova 形态 - Rust 形态（`filter*`）和 Laravel 形态（`db_where` / `where_*`）。两者调用的是同一份规范实现；两者都标了 `#[doc(alias = "...")]`，这样 rustdoc 搜索用哪个名字都能找到。

| Laravel | Suprnova（Rust 形态） | Suprnova（Laravel 形态） | 说明 |
|---------|----------------------|--------------------------|-------|
| `->where(col, val)` | `.filter(col, val)` | `.db_where(col, val)` | 等值判断 |
| `->where(col, op, val)` | `.filter_op(col, op, val)` | `.db_where_op(col, op, val)` | 任意运算符 |
| `->orWhere(...)` | `.or_filter(...)` | `.or_where(...)` | |
| `->orWhereKey(id)` | `.or_filter_key(id)` | `.or_where_key(id)` | 作为一个“或”分支的主键过滤 |
| `->orWhereKeyNot(id)` | `.or_filter_key_not(id)` | `.or_where_key_not(id)` | 作为一个“或”分支的取反主键过滤 |
| `->whereNot(col, val)` | `.filter_not(col, val)` | `.where_not(col, val)` | |
| `->whereIn(col, vals)` | `.filter_in(col, vals)` | `.where_in(col, vals)` | |
| `->whereNotIn(col, vals)` | `.filter_not_in(col, vals)` | `.where_not_in(col, vals)` | |
| `->whereBetween(col, [a, b])` | `.filter_between(col, a..=b)` | `.where_between(col, a..=b)` | Rust 区间 |
| `->whereNotBetween(col, [a, b])` | `.filter_not_between(col, a..=b)` | `.where_not_between(col, a..=b)` | |
| `->whereNull(col)` | `.filter_null(col)` | `.where_null(col)` | |
| `->whereNotNull(col)` | `.filter_not_null(col)` | `.where_not_null(col)` | |
| `->whereDate(col, '2026-05-19')` | `.filter_date(col, NaiveDate)` | `.where_date(col, NaiveDate)` | |
| `->whereMonth(col, 5)` | `.filter_month(col, 5)` | `.where_month(col, 5)` | |
| `->whereDay(col, 19)` | `.filter_day(col, 19)` | `.where_day(col, 19)` | |
| `->whereYear(col, 2026)` | `.filter_year(col, 2026)` | `.where_year(col, 2026)` | |
| `->whereTime(col, '12:30')` | `.filter_time(col, NaiveTime)` | `.where_time(col, NaiveTime)` | |
| `->whereLike(col, pattern)` | `.filter_like(col, pattern)` | `.where_like(col, pattern)` | |
| `->whereNotLike(col, pattern)` | `.filter_not_like(col, pattern)` | `.where_not_like(col, pattern)` | |
| `->whereBinary(col, val)` | `.filter_binary(col, val)` | `.where_binary(col, val)` | 逐字节精确；仅限 MySQL 和 MariaDB |
| `->orWhereBinary(col, val)` | `.or_filter_binary(col, val)` | `.or_where_binary(col, val)` | |
| `->whereNotBinary(col, val)` | `.filter_not_binary(col, val)` | `.where_not_binary(col, val)` | |
| `->orWhereNotBinary(col, val)` | `.or_filter_not_binary(col, val)` | `.or_where_not_binary(col, val)` | |
| `->whereJsonContains(col, v)` | `.filter_json_contains(col, v)` | `.where_json_contains(col, v)` | 按后端分发 |
| `->whereJsonLength(col, op, n)` | `.filter_json_length(col, op, n)` | `.where_json_length(col, op, n)` | |
| `->whereColumn(a, b)` | `.filter_column(a, b)` | `.where_column(a, b)` | 列对列比较 |
| `->whereExists(closure)` | `.filter_exists(builder)` | `.where_exists(builder)` | 子查询 |
| `->whereHas(rel, closure)` | `.filter_has(rel, fn)` | `.where_has(rel, fn)` | 关系谓词（10B） |
| `->whereDoesntHave(rel)` | `.filter_doesnt_have(rel)` | `.where_doesnt_have(rel)` | （10B） |
| `->whereRelation(rel, col, op, v)` | `.filter_relation(...)` | `.where_relation(...)` | （10B） |
| `->whereRaw(sql, bindings)` | `.filter_raw(sql, bindings)` | `.where_raw(sql, bindings)` | |

`binary` 这一家子比较的是原始字节，而不是在这个列的排序规则之下做匹配。MySQL 和 MariaDB 发出 `col = binary ?`；Postgres 和 SQLite 没有对应的运算符，所以在那些后端上，一个终结方法会在这条语句渲染时返回一个错误，而不是回退成一个取决于排序规则的 `=`。参见[逐字节精确的比较](queries.md#byte-exact-comparison)。

带绑定参数的原始谓词，在 SQLite、MySQL 和 PostgreSQL 上都用可移植的 `?` 占位符：

```rust
let rows = User::query()
    .filter("active", true)
    .filter_raw(
        "score >= ? AND role = ?",
        vec![serde_json::json!(80), serde_json::json!("admin")],
    )
    .get()
    .await?;
```

在 PostgreSQL 上，Suprnova 会把这些占位符重新定位到更早的那些查询绑定之后，所以这个示例会把 `active` 渲染成 `$1`，把这条原始谓词渲染成 `$2`/`$3`。要在一段带绑定参数的原始片段里表示字面的问号运算符，就用 `??`，比如 `"payload ?? 'enabled' AND status = ?"`。既有的 `$N` 片段仍然会被接受，但可移植的占位符能避免调用点和查询位置绑死。混用占位符风格，以及占位符和绑定参数数量不匹配，都会在数据库 I/O 之前被拒绝。和每一个原始表达式一样，这段 SQL 文本必须是可信的；不可信的值只能放进绑定参数的 vector 里。

### 排序

```php
$users = User::orderBy('name', 'asc')->get();
$users = User::orderByDesc('created_at')->get();
$users = User::latest()->get();        // 捷径：orderBy(created_at, desc)
$users = User::oldest()->get();        // 捷径：orderBy(created_at, asc)
$users = User::inRandomOrder()->get();
```

```rust
let users = User::query().order_by("name", Direction::Asc).get().await?;
let users = User::query().order_by_desc("created_at").get().await?;
let users = User::latest().get().await?;
let users = User::oldest().get().await?;
let users = User::query().in_random_order().get().await?;
```

`Direction::Asc` / `Direction::Desc` 是从 SeaORM 重新导出的那个 Suprnova 枚举。

#### 按一个明确的序列排序

`in_order_of` 会把行按您列出的顺序排好。任何值不在这个列表里的行，都排在所有在列表里的行之后。

```php
$users = User::inOrderOf('role', ['admin', 'member', 'guest'])->get();
```

```rust
let users = User::query()
    .in_order_of("role", ["admin", "member", "guest"])
    .get()
    .await?;
```

Suprnova 会把这一句渲染成一个带绑定参数的 `CASE` 表达式，所以这些值是参数，从请求数据里取也是安全的：

```sql
ORDER BY CASE WHEN role = ? THEN 0 WHEN role = ? THEN 1 WHEN role = ? THEN 2 ELSE 3 END
```

列名是一个 SQL 标识符，不是一个参数。请把它写死，或者从一份允许列表里挑，和其他每一个列参数一样。一个空的值列表根本不会添加任何排序，所以您可以有条件地把这个序列拼出来，而不必给空的情况开特例。

对于一个使用了 `AsEnum<E>` 转换的列，请把每一个变体都过一遍 `as_ref()`。那才是这个转换实际存下去的那个字符串：

```rust
let users = User::query()
    .in_order_of("role", [Role::Admin.as_ref(), Role::Member.as_ref()])
    .get()
    .await?;
```

`in_order_of` 只发布在带类型的 `Builder<M>` 表面上。不带模型的 `DB::table(...)` 构造器只按列和方向排序。

### 分组 + having

```php
$rows = User::groupBy('role')->having('count(*)', '>', 5)->get();
```

```rust
let rows = User::query()
    .group_by("role")
    .having_op("count(*)", ">", 5)
    .get()
    .await?;
```

### 限制 / 偏移量

```php
$users = User::limit(10)->offset(20)->get();
$users = User::take(10)->skip(20)->get();   // 别名
```

```rust
let users = User::query().limit(10).offset(20).get().await?;
let users = User::query().take(10).skip(20).get().await?;
```

### Select / add_select / select_raw

```rust
let users = User::query().select(["id", "name", "email"]).get().await?;
let users = User::query().select("name").add_select("email").get().await?;
let rows  = User::query().select_raw("count(*) as total, role")
    .group_by("role")
    .get_raw()
    .await?;
```

`get_raw()` 会返回原始的列形态结果，用于 `select_raw` 里选出的列和模型架构不匹配的场景；`get()` 返回 `Vec<User>`，并且要求选出的列足以填满这个模型结构体。

### 去重

```rust
let emails: Vec<String> = User::query().distinct().pluck("email").await?;
```

### 聚合

```rust
let count   = User::count().await?;
let count   = User::filter("active", true).count().await?;
let sum     = User::sum::<f64>("balance").await?;
let avg     = Order::avg::<f64>("total").await?;
let min     = Order::min::<DateTime<Utc>>("created_at").await?;
let max     = Order::max::<DateTime<Utc>>("created_at").await?;
let exists  = User::filter("email", &email).exists().await?;
let missing = User::filter("email", &email).doesnt_exist().await?;
```

聚合方法在返回类型上是泛型的，因为 SeaORM 需要知道该把这个数据库标量强转成什么。类型的默认值：`count -> i64`；`sum`/`avg` 携带一个显式的类型参数。Suprnova 会在内部给生成的聚合表达式起别名，这样在 PostgreSQL、MySQL 和 SQLite 上解码出来的都是同一个类型化的结果。`sum` 和 `avg` 在匹配集合为空时返回零，而 `min` 和 `max` 则返回 `None`。请求的 Rust 类型不兼容，或者结果列缺失，都是一个数据库错误；它绝不会被转换成一个看似合理的零或者 `None`。

### 终结方法

```rust
let users:  Vec<User>          = User::all().await?;
let first:  Option<User>       = User::first().await?;
let user:   User               = User::first_or_fail().await?;
let value:  Option<String>     = User::filter("...").value("email").await?;
let emails: Vec<String>        = User::pluck::<String>("email").await?;
let keyed:  HashMap<i64, String> = User::pluck_keyed::<i64, String>("id", "name").await?;
let ids:    Vec<i64>           = User::query().model_keys().await?;
```

`to_sql` 会返回下一个终结方法本来会发出的那份带参数的 SQL - 用于调试，或者构建视图时很有用。这些绑定参数可以通过 `.to_sql_with_bindings() -> (String, Vec<Value>)` 拿到。

`model_keys` 是只取键的终结方法：它投影**限定的**主键（`users.id`），并且从不水合模型，所以“哪些行匹配？”这个问题每一个匹配只需一个列，而不需要完整行。限定名称使它能在查询连接另一张也有 `id` 的表时正常工作。构建器上已有的任何 `select(...)` 都会被丢弃 - 调用方请求的是键。

### 并集

```rust
let first  = User::filter("active", true);
let second = User::filter("role", "admin");
let users  = first.union(second).get().await?;
let users  = first.union_all(second).get().await?;
```

## 行锁

有两个构造器方法，会在 SELECT 时请求一个逐行的数据库锁：

```rust
// 独占写锁 - 会阻塞其他试图锁定或写入同一批行的事务，
// 直到这个事务提交为止。
let order = Order::query()
    .filter("id", 42)
    .lock_for_update()
    .first_or_fail()
    .await?;

// 共享读锁 - 允许其他共享读者，阻塞写者。
let inventory = Inventory::query()
    .filter("sku", sku)
    .shared_lock()
    .first_or_fail()
    .await?;
```

各后端发出的 SQL：

| 后端  | `lock_for_update()` | `shared_lock()`        |
|----------|---------------------|------------------------|
| Postgres | `FOR UPDATE`        | `FOR SHARE`            |
| MySQL    | `FOR UPDATE`        | `LOCK IN SHARE MODE`   |
| SQLite   | （没有 SQL，见下文） | （没有 SQL，见下文）    |

这个锁子句会被追加在这条复合语句的最末尾 - 在每一个 `UNION` 分支、每一个 `ORDER BY`、每一个 `LIMIT` / `OFFSET` 之后。两个构造器的 `union(...)`，接上 `.lock_for_update()`，会在最外层恰好发出**一个** `FOR UPDATE`，不是每个分支各发一个。

要把一个您已经握在手里的模型重新加载，并在同一条语句里拿到那把锁，请用 `refresh_for_update`：

```rust
DB::transaction(|tx| async move {
    let mut order = Order::find_or_fail(42).await?;
    order.refresh_for_update().await?;   // SELECT ... WHERE id = ? FOR UPDATE
    order.status = "processed".into();
    order.save_with_tx(&tx).await?;
    Ok(())
}).await?;
```

### 在事务内部使用

这个锁只有**在事务内部**才会发挥实际作用 - 没有事务的话，SQL 仍然会发出，但这个锁会在语句结束时就释放。请搭配 `DB::transaction(...)` 使用：

```rust
DB::transaction(|tx| async move {
    let order = Order::query()
        .filter("id", 42)
        .lock_for_update()
        .first_or_fail()
        .with_tx(&tx)
        .await?;
    // 其他试图锁定 id=42 的事务，会在这里阻塞，直到提交为止。
    order.status = "processed".into();
    order.save_with_tx(&tx).await?;
    Ok(())
}).await?;
```

### `lock_for_update` vs `shared_lock`

大多数“先读后写”的流程，想要的是 `lock_for_update`。一个共享锁仍然会让另一个 `shared_lock` 读者，在后续的 `UPDATE` 上和您形成竞态 - 只有 `FOR UPDATE` 是互斥的。

`shared_lock` 适合那种一致性快照读 - 您读取一行，从中得出一个决策，但不写回 - 比如一次不会自己去扣减库存的库存检查。

### SQLite

SQLite 没有行级锁。它只有文件级的事务锁（`BEGIN IMMEDIATE` / `BEGIN EXCLUSIVE`）。这些锁方法在 SQLite 路径上被**保留了下来**，这样跨后端的代码才能编译，但它们不会发出任何 SQL。

每个进程里，`lock_for_update` / `shared_lock` 第一次针对一个 SQLite 后端运行时，框架会在 `suprnova::eloquent::lock` 这个 tracing target 上记一条 `warn!`。这样就能把这个空操作暴露出来，又不会在高吞吐的代码路径上刷屏。

如果您在 SQLite 上需要跨行的竞争保证，就把这段临界区包进一个显式的 `BEGIN IMMEDIATE` 事务里 - 在文件级别，这会阻塞其他每一个写者。

### v1 里没有的东西

- **`NOWAIT` / `SKIP LOCKED`** - 对作业队列的认领流程很有用，但它们会增加 API 表面。推迟到有真实的消费者需要它们时再做。

## 事务

Suprnova 为数据库事务提供了三个入口，外加通过保存点实现的嵌套回滚。其中两个 - 闭包形式和重试死锁的辅助方法 - 会安装一个环境式的上下文，这样闭包内部的模型操作就会自动路由经过这个事务，调用者不必在每一个调用点上手动传递一个句柄。

### 闭包形式 - `DB::transaction`

闭包形式是常见的情况。这个闭包会收到一个 `&Transaction`，可以用它通过 `savepoint(name)` 打检查点；闭包内部的每一个 `Model::*` / `Builder::*` 操作，都会经由一个名叫 `CURRENT_TX` 的 `tokio::task_local!`，自动路由经过这个事务。

```rust
use suprnova::{DB, FrameworkError, Model};

DB::transaction(|_tx| {
    Box::pin(async move {
        let mut alice = User::query().filter("name", "alice").first_or_fail().await?;
        alice.balance -= 30;
        alice.save().await?;

        let mut bob = User::query().filter("name", "bob").first_or_fail().await?;
        bob.balance += 30;
        bob.save().await?;
        Ok::<(), FrameworkError>(())
    })
}).await?;
```

- 闭包返回 `Ok` → **提交**。
- 闭包返回 `Err` → **回滚**（原始错误会继续传播）。
- 闭包 panic → 回滚（这个正在进行中的事务，会在栈展开时被丢弃；SeaORM 的 `DatabaseTransaction::drop` 会执行回滚）。

闭包内部的读取，能看到同一个事务里的写入（这是靠每一次叶子 SQL 调用都会做的 `CURRENT_TX` 查找）。进程启动之后的第一次 `DB::transaction` 调用，会从 `DB::connection()` 上取到数据库后端；后续的调用会复用同一个连接注册表。

这个签名用了一个高阶 trait bound 加上 `Pin<Box<dyn Future>>`，这样闭包就能跨越 `.await` 点借用 `tx`：

```rust
DB::transaction(|tx| {
    Box::pin(async move {
        // ……保存点之前的工作……
        tx.savepoint("inner").await?;
        // ……内部工作……
        if some_condition {
            tx.rollback_to("inner").await?;
        }
        Ok::<(), FrameworkError>(())
    })
}).await?;
```

`Box::pin(async move { ... })` 这个形态，是为了让这个 future 能在一次 `.await` 之后仍然使用 `&tx` 所付出的代价 - 没有它，这次借用的生命周期就没法逃出这个闭包体。这和 SeaORM 的 `TransactionTrait::transaction` 签名相呼应。

### 保存点 - `tx.savepoint(name)` / `tx.rollback_to(name)`

保存点会给这个事务打检查点，这样您就能丢弃一段内部工作，而不必中止外层的提交。三个后端上都能用 - 即便 SQLite 没有行级锁，它的 `SAVEPOINT` 也是完全可用的。

```rust
DB::transaction(|tx| {
    Box::pin(async move {
        let mut account = Account::query().filter("id", id).first_or_fail().await?;
        account.balance = 200;
        account.save().await?;     // 外层 tx 提交时，这个改动就会被提交

        tx.savepoint("audit_trail").await?;

        let entry = AuditEntry::create(attrs! { actor_id: actor, ... }).await?;
        if audit_validation_failed(&entry) {
            tx.rollback_to("audit_trail").await?;
            // audit_trail 这一行没了；account 的更新仍待提交
        }

        Ok::<(), FrameworkError>(())
    })
}).await?;
```

这个保存点名字会被原样内插进 SQL 里 - 请用一个静态的标识符，**不要**拼接用户输入。

### 嵌套的 `DB::transaction` 会在运行时被拒绝

```rust
DB::transaction(|_outer| Box::pin(async move {
    let inner = DB::transaction(|_inner| Box::pin(async move {
        Ok::<(), FrameworkError>(())
    })).await;
    // inner is Err(FrameworkError::Database(
    //     "nested DB::transaction is not supported; use tx.savepoint(name) for nested rollback"
    // ))
    Ok::<(), FrameworkError>(())
})).await?;
```

SeaORM 的 `DatabaseConnection::begin()` 不能组合 - 在一个已经持有事务的连接上调用它，会启动一个全新的物理事务，这个事务会独立于外层事务提交/回滚。这是一个悄无声息的数据完整性陷阱，所以 `DB::transaction` 会提前检查 `CURRENT_TX`，返回一个数据库错误，而不是产出错误的语义。嵌套行为请用 `tx.savepoint(name)`。

### 死锁重试 - `DB::transaction_with_attempts`

Postgres 的 `SERIALIZABLE` 读取，以及 MySQL 的行级锁，都可能抛出序列化失败 / 死锁错误，而这些错误是靠重试这个事务来解决的。`transaction_with_attempts` 会每次从头运行这个闭包，最多运行 `attempts` 次：

```rust
DB::transaction_with_attempts(3, |_tx| {
    Box::pin(async move {
        // SERIALIZABLE 隔离级别下的逻辑，可能会和一个并发的
        // tx 形成竞态，并在提交时暴露出 SQLSTATE 40001 / 40P01。
        let inventory = Inventory::query()
            .filter("sku", sku)
            .lock_for_update()
            .first_or_fail()
            .await?;
        if inventory.units < requested {
            return Err(FrameworkError::bad_request("out of stock"));
        }
        Inventory::query()
            .filter("sku", sku)
            .update(attrs! { units: inventory.units - requested })
            .await?;
        Ok::<(), FrameworkError>(())
    })
}).await?;
```

检测的方式，是对内部错误的 Display 字符串做子串匹配：

- Postgres 的 SQLSTATE `40001`（serialization_failure）
- Postgres 的 SQLSTATE `40P01`（deadlock_detected）
- 不区分大小写的 `"deadlock"` 子串匹配（覆盖 MySQL 的
  `Deadlock found when trying to get lock`，以及任何面向用户暴露出来的死锁字符串）

在最后一次尝试时，这个错误会原样传播。这个闭包在每一次尝试时都会从头运行 - 请捕获拥有所有权的状态，或者 `Arc`，而不是 `&mut` 引用，这样这条重试路径才有明确定义。

> **注意事项：** 因为这个检测机制包含一个不区分大小写的
> `"deadlock"` 子串匹配（这是 MySQL 需要的，它的驱动程序不会暴露
> 一个 SQLSTATE），所以任何 `Display` 里包含这个词的内部错误，都
> 会触发一次重试。当您从一个 `transaction_with_attempts` 闭包内部
> 抛出自己的错误时，请在消息里避开 `"deadlock"` 这个词 - 否则一个
> 不相关的验证错误，会在传播之前重试最多 `attempts` 次。Postgres
> 的 SQLSTATE 匹配（`40001` / `40P01`）才是可靠的信号；这个启发式
> 规则只针对 MySQL。

### 手动形式 - `DB::begin_transaction` + `*_with_tx` 薄封装

当这个事务的生命周期不适合用一个闭包来表达时（例如跨越多个控制流分支），就打开一个手动的 `Transaction`，让每一个操作显式地选择加入它：

```rust
let tx = DB::begin_transaction().await?;

let mut user = User::query()
    .filter("name", "alice")
    .with_tx(&tx)
    .first_or_fail()
    .await?;
user.balance = 500;
user.save_with_tx(&tx).await?;

if some_condition {
    let mut other = User::query()
        .filter("name", "bob")
        .with_tx(&tx)
        .first_or_fail()
        .await?;
    other.update_with_tx(&tx, attrs! { balance: 200i64 }).await?;
}

tx.commit().await?;  // 或者 tx.rollback().await?;
```

手动模式**不会**安装 `CURRENT_TX`。请用 `Builder::with_tx(&tx)`，或者 `Model::*_with_tx(&tx, ...)` 这些薄封装，把单个操作纳入这个事务的范围：

| Trait 方法        | 手动变体                            |
|---------------------|-------------------------------------------|
| `Model::create`     | `Model::create_with_tx(&tx, attrs)`       |
| `Model::save`       | `Model::save_with_tx(&tx)`                |
| `Model::update`     | `Model::update_with_tx(&tx, attrs)`       |
| `Model::delete`     | `Model::delete_with_tx(&tx)`              |
| `Model::force_delete` | `Model::force_delete_with_tx(&tx)`      |
| `Builder::*`        | `Builder::with_tx(&tx).*`                 |

持有一个 `Transaction`，会在这个句柄的整个生命周期里固定占用一个连接池连接。在 SQLite 上，这个连接池只有一个连接，所以任何针对同一个数据库的并行非事务读取，都会阻塞到这个事务完成为止 - **请在 `DB::begin_transaction()` 之前，就把任何预检行加载好**，并把每一个依赖它的写入都路由经过返回的这个 `tx`。

`Transaction::commit` / `Transaction::rollback` 会消耗这个句柄，并且需要对内部的 SeaORM 事务做一次 `Arc::try_unwrap`；如果在提交/回滚的那一刻，还有任何 `TxHandle` 的克隆（来自 `tx.handle()` / `Builder::with_tx(&tx)`）活着，两者都会带着一个 "TxHandle clones still alive" 错误失败。正确的修法，是在调用 `commit` 之前丢弃您的 `Builder<M>` / 尚未释放的那些句柄 - 框架拒绝让一次半未提交的写入，和一个持有同一个 tx 的并行写者形成竞态。

### 优先级

把一个操作路由经过某个连接时，遵循三层优先级：

1. **构造器级别的覆盖** - `Builder::with_tx(&tx)`，或者任何
   `Model::*_with_tx(&tx, ...)` 薄封装。显式胜过环境式。
2. **环境式的 `CURRENT_TX`** - 由 `DB::transaction` /
   `DB::transaction_with_attempts` 为这个闭包的任务作用域安装。
3. **连接池兜底** - `DB::connection()` 会返回那个全局的
   `DbConnection` 单例。

在 `DB::transaction(|tx| ...)` 内部，显式调用
`Builder::with_tx(&other_tx)`，会把这一条查询路由经过 `other_tx` -
绕开这个环境式的 `CURRENT_TX`。这几乎肯定是一个 bug；这条覆盖路径的存在，是为了手动形式，不是为了覆盖这个闭包自己的 tx。

### `with_tx` 与全局作用域

一个携带着 `tx_override` 的构造器，仍然会遵从全局作用域、本地作用域，以及预加载计划 - 这个覆盖只会改变连接的路由方式，不会改变 SQL。

### 限制（v1）

- **关系预加载** - `Builder::with(["posts"])` 和
  `Collection::load(["posts"])` 会把预加载用的 `IN (...)` 子查询路由经过 `DB::connection()`，不会经过这个活跃的事务。
  `DB::transaction` 闭包内部尚未提交的写入，对通过 `.with(...)`
  加载的关系是**不可见**的。目前请把事务内的工作限定在直接的
  `Model::*` / `Builder::*` / `DB::table(...)` 调用上；把关系加载推迟到外层写入落地之后（在手动路径上则是推迟到
  `DB::begin_transaction` 之前）。这是一个已知的缝隙 - 那个路由辅助函数（`ExecutorChoice`）已经就位在每一个 SQL 叶子节点上；卡住的地方在于 `EagerLoadDispatch::eager_load` 接受的是一个具体的 `&DatabaseConnection`，而这个宏为每一种关系类型生成的都是这个签名。后续会有一次扫尾工作，把这个 trait 适配到这个分发辅助函数上。
- **Postgres 上的 DDL** - 在一个事务内部，`DB::statement(...)` 会针对这个 tx 连接运行这条 DDL，这在 Postgres 上是允许的；MySQL 会隐式提交，因此在一个 Suprnova 事务内部不受支持（这和 Laravel 的
  `DB::transaction` 注意事项一致）。

## 作用域

Suprnova 提供了两种作用域，和 Laravel 相呼应：

- **本地作用域** - 构造器上的扩展方法，按模型用 `#[suprnova::scopes(Model)]` 声明。被标注的那个 `impl` 块里的每一个自由函数，都会同时变成 `Model::name()`（一个静态的启动方法）和 `Builder::name()`（一个可链式调用的方法）。
- **全局作用域** - `GlobalScope<M>` 的实现，经由 `ScopeRegistry::register::<M, _>(scope)` 在启动时注册。每一次 `Model::query()` 调用，都会自动叠加上它们。

### 本地作用域

声明本地作用域的方式，是给它们这样的形状：`fn(query: Builder<Self>, args...) -> Builder<Self>`：

```rust
#[suprnova::scopes(User)]
impl User {
    pub fn active(query: Builder<Self>) -> Builder<Self> {
        query.filter("active", true)
    }

    pub fn popular(query: Builder<Self>, threshold: i64) -> Builder<Self> {
        query.filter_op("followers_count", ">", threshold)
    }
}

// 既可以当启动方法用，也可以当可链式调用的方法用：
let active_users  = User::active().get().await?;
let popular_users = User::query().active().popular(500).get().await?;
```

在同一个 `impl` 块里声明的非作用域方法（第一个参数不是 `query: Builder<Self>` 的任何方法），会原样透传，不受影响。

### 全局作用域

全局作用域会应用在每一次 `Model::query()` 调用上。经典的用例是多租户 - 每一次读取都会被限定在当前租户范围内，而不需要每一个调用者都手动传递这个过滤条件。

```rust
use suprnova::eloquent::scopes::{GlobalScope, ScopeRegistry};

pub struct TenantScope;

impl GlobalScope<Article> for TenantScope {
    fn apply(&self, query: Builder<Article>) -> Builder<Article> {
        // 从一个 task-local / AtomicI64 / 或者任何存放逐请求状态的地方，
        // 读取当前租户。
        query.filter("tenant_id", current_tenant_id())
    }
}

// 在启动时 - 通常在您的 provider/bootstrap 模块内部：
ScopeRegistry::register::<Article, _>(TenantScope);

// 每一次读取都会自动被限定在活跃租户范围内：
let scoped = Article::query().get().await?;
```

同一个模型上的多个作用域，会按注册顺序组合 - 先注册的先运行，所以它的过滤子句会先出现在这条 WHERE 链里。AND 组合的过滤条件不关心顺序，但对任何副作用顺序可见的子句（例如排序、having、原始片段）来说，从左到右的顺序是有影响的。

### 退出一个全局作用域

`#[suprnova::model]` 宏接触到的每一个模型，都会被生成两个静态辅助方法：

```rust
// 按类型绕开恰好一个已注册的作用域。其他作用域仍然会生效。
let all_tenants = Article::without_global_scope::<TenantScope>().get().await?;

// 绕开每一个已注册的作用域。管理端工具的常见模式。
let everything = Article::without_global_scopes().get().await?;
```

**重要：** 这些退出辅助方法必须是入口点。在一个已经由 `Model::query()` 返回的构造器上链接 `.without_global_scope::<S>()`，并不会撤销已经运行过的作用域 - `Model::query()` 会在构造时就急切地应用这些作用域，所以这个屏蔽设置得太晚了。要拿到正确的语义，请用上面那些逐模型的静态辅助方法。

### 全局作用域在哪里生效

| 路径 | 全局作用域会生效吗？ |
|------|----------------------|
| `Model::query()` | 会 - 规范的、带作用域的入口点 |
| `Model::without_global_scope::<S>()` | 会，但去掉 `S` |
| `Model::without_global_scopes()` | 不会 |
| `Model::find(id)` | 不会 - 主键查找会直接经过 SeaORM |
| `Model::find_many([...])` | 不会 - 原因相同 |
| `Model::all()` | 不会 - 原因相同 |

这和 Laravel 相呼应：`Eloquent\Model::find` 不会触发 `addGlobalScopes`。想要带作用域的主键查找的调用者，请用 `Self::query().filter("id", pk).first().await?`。

### 软删除与全局作用域共存

`#[suprnova::model(soft_deletes)]` 会经由一个独立的字符串标签机制安装 `deleted_at IS NULL` 过滤条件，不经过这个类型化的作用域注册表。两层会组合起来：

- `Model::query()` 会过滤掉被丢弃的行，**并且**运行每一个已注册的作用域。
- `Model::without_global_scopes()` 会丢弃已注册的作用域，但保留软删除过滤条件 - 想要读取每一个列集合的管理端工具，默认仍然会排除被丢弃的行。
- `Model::with_trashed()` 和 `Model::only_trashed()` 会跳过软删除过滤，也会绕开这个注册表（它们构建的是一个全新的、不带作用域的构造器）。如果您需要在被丢弃的行上做感知作用域的读取，就搭配 `.without_global_scope::<S>()` 使用。

## 关系

Suprnova 支持每一种 Eloquent 关系类型。它们在 `#[suprnova::model]` 上的 `relations = { ... }` 块里声明，这个宏会针对每一个已声明的关系，生成结构体上的一个方法、一个已加载访问器（`<name>_loaded()`）、一个计数访问器（`<name>_count()`），以及预加载器会调用进去的那个分发分支。本节覆盖的是逐类型的形状和选项表；关于连接键解析、多态注册表、中间表行，以及多态枚举到底层表示的映射的深入探讨，请参见[Eloquent 关系](eloquent-relationships.md)。目前支持的关系类型：

| 类型                | 基数 | 跨族 | 底层实现 |
|---------------------|----------|-----------------|-----------|
| `HasOne<R>`         | 一      | 否              | 针对 `<parent>_id` 的 `IN` 查询 |
| `BelongsTo<R>`      | 一      | 否              | 针对这一行外键的 `IN` 查询 |
| `HasMany<R>`        | 多     | 否              | 和 `HasOne` 相同，返回 `Vec<R>` |
| `BelongsToMany<R, P>` | 多   | 否              | 中间表 `P`，INNER JOIN + `pivot::<P>()` |
| `HasOneThrough<B, R>`  | 一   | 否              | 两次查询的 JOIN，`parent → B → R` |
| `HasManyThrough<B, R>` | 多  | 否              | 和上面相同，返回 `Vec<R>` |
| `MorphOne<R>`       | 一      | 是             | `IN` + `<name>_type = "<self>"` 过滤 |
| `MorphMany<R>`      | 多     | 是             | 和 `MorphOne` 相同，返回 `Vec<R>` |
| `MorphTo`           | 一      | 是（子级 → 多个族） | 在声明处生成一个按族的枚举 |
| `MorphToMany<R, P>` | 多     | 是             | 多态多对多中间表 `P` |
| `MorphedByMany<R, P>` | 多   | 是（反向） | 同一张中间表，反过来扫描 |

### `relations = { ... }` 语法

每一个关系声明都携带同样的外层形状：关系名、类型、相关类型（以及适用场景下的中间表/中间类型），外加一个 `{ ... }` 选项块。

```rust
use suprnova::model;

#[model(
    table = "users",
    relations = {
        // HasMany<R>
        posts: HasMany<crate::models::Post> {
            fk = "author_id",         // 覆盖默认的 `user_id`
        },
        // BelongsToMany<R, Pivot>
        roles: BelongsToMany<crate::models::Role, crate::models::RoleUser> {
            with_pivot = ["assigned_at"],
            with_timestamps,
        },
    },
)]
pub struct User {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
```

常见选项：

| 选项                     | 关系类型                | 用途 |
|----------------------------|-------------------------------|---------|
| `fk = "..."`               | 每一种带有子级外键的类型    | 子级上指向父级的那一列。默认 = `<snake(parent_struct)>_id`。 |
| `lk = "..."`               | 一/多类型                | 父级上用作连接键的那一列。默认 = `"id"`。 |
| `related_key = "..."`      | `BelongsToMany`、`MorphToMany` | 相关端主键列的名字。默认 = `"id"`。当相关模型用的主键不是 `id` 时必填。 |
| `with_pivot = ["...", ...]` | `BelongsToMany`、`MorphToMany` | 中间表上要在这次 join 里暴露出来的额外列。 |
| `with_timestamps`          | `BelongsToMany`、`MorphToMany` | 在 attach/sync 时给 `created_at` / `updated_at` 打上时间戳。 |
| `with_default = \|\| { ... }` | `BelongsTo`                 | 当外键为 null，或者父级缺失时，产出一个默认值的闭包。 |
| `first_key`, `second_key`, `second_local_key` | `HasOneThrough`、`HasManyThrough` | JOIN 键的覆盖 - 见下文的穿透小节。 |
| `name = "..."`             | 每一种多态类型              | 多态族名称（例如 `"commentable"`、`"taggable"`）。驱动子级/中间表上的 `<name>_id` / `<name>_type` 列。 |
| `targets = [T1, T2, ...]`  | `MorphTo`                     | 具体多态目标的列表。这个宏会在声明处生成一个 `<Name>Morph` 枚举，每一个目标一个变体，外加一个 `Unknown(String, i64)`。 |
| `target_morph_type = "..."` | `MorphedByMany`              | 在中间表上标识目标族的那个多态类型字符串。 |
| `pivot_table`, `pivot_foreign_key`, `pivot_related_key` | `BelongsToMany`、`MorphToMany` | 当默认值不合适时，中间表一侧的列/表覆盖。 |

### `HasOne<R>` 与 `BelongsTo<R>`

两个方向上都是一对一。`HasOne` 活在父级一侧，调用的是 `R::query().filter(<fk>, <self.id>).first()`。`BelongsTo` 活在子级一侧，从 `self` 上读取外键，然后调用 `R::query().filter(<owner_key>, <fk_value>).first()`。

```rust
#[model(table = "users", relations = {
    profile: HasOne<crate::models::Profile>,
})]
pub struct User { /* ... */ }

#[model(table = "profiles", relations = {
    user: BelongsTo<crate::models::User>,
})]
pub struct Profile {
    pub id: i64,
    pub user_id: i64,
    pub bio: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

let user = User::find(1).await?.unwrap();
let profile: Option<Profile> = user.profile().first().await?;

let profile = Profile::find(42).await?.unwrap();
let owner: Option<User> = profile.user().first().await?;
```

`BelongsTo` 支持 `with_default = || R { ... }`，它会在外键为 null，或者父级那一行缺失时触发。这个默认值闭包会逐次调用运行（逐预加载行也一样）- 非常适合在一个已删除的用户仍然留有评论时，提供一个空的替身：

```rust
#[model(table = "comments", relations = {
    author: BelongsTo<crate::models::User> {
        with_default = || User {
            name: "[deleted]".into(),
            ..Default::default()
        },
    },
})]
pub struct Comment { /* ... */ }

let c = Comment::find(99).await?.unwrap();
// 永远是 Some - 当 user 那一行缺失时，这个默认值就会触发。
let author = c.author().first().await?.unwrap();
```

### `HasMany<R>`

父级一侧的一对多。返回一个流式构造器；可以链接 filter / order / latest / take / get / count，然后终结。

```rust
#[model(table = "users", relations = {
    posts: HasMany<crate::models::Post> {
        fk = "author_id",
    },
})]
pub struct User { /* ... */ }

let u = User::find(1).await?.unwrap();

// 这个用户的每一篇 post，默认排序：
let posts: Vec<Post> = u.posts().get().await?;

// 过滤 + 排序 + 分页：
let recent = u.posts()
    .filter("published", true)
    .latest()                          // ORDER BY created_at DESC
    .take(10)
    .get()
    .await?;

// 单独的 COUNT - 不获取行：
let total: i64 = u.posts().count().await?;
```

可用的终结方法：`.first()`、`.get()`、`.count()`。可用的可链式过滤方法：`.filter` / `.db_where`、`.filter_in` / `.where_in`、`.order_by`、`.latest`、`.oldest`、`.limit`、`.take`。

### `BelongsToMany<R, P>` - 作为一等公民的中间表

通过一个 `#[suprnova::model]` 声明的中间表实现的多对多。这个中间表是一个一等公民的模型，有它自己的行身份 - 不是一个元组，也不是一个隐藏的哈希表。相比 Laravel 那种匿名中间表的形态，有两个关键的好处：

1. 这个中间表行是类型安全的。请经由 `r.pivot::<P>().<column>` 读取
   `with_pivot` 列，永远不要经由 `r.pivot.get("...")`。
2. 这个中间表模型可以从框架的其他部分触达（工厂、作用域、转换、钩子），和任何其他模型一样。

```rust
#[model(table = "role_user", fillable = ["user_id", "role_id", "assigned_at"])]
pub struct RoleUser {
    pub id: i64,
    pub user_id: i64,
    pub role_id: i64,
    pub assigned_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[model(table = "users", relations = {
    roles: BelongsToMany<crate::models::Role, RoleUser> {
        with_pivot = ["assigned_at"],
        with_timestamps,
    },
})]
pub struct User { /* ... */ }

let u = User::find(1).await?.unwrap();
let admin = Role::create(attrs! { name: "admin" }).await?;

// Attach + sync 修改器
u.roles().attach(admin.id).await?;
u.roles().attach_with(admin.id, attrs! { assigned_at: chrono::Utc::now() }).await?;
u.roles().sync([role_a.id, role_b.id, role_c.id]).await?;
u.roles().detach(admin.id).await?;

// 通过逐行的向下转型访问器读取中间表数据：
let roles = u.roles().get().await?;
for r in &roles {
    let p: &RoleUser = r.pivot::<RoleUser>();
    println!("user {} got role {} at {:?}", p.user_id, p.role_id, p.assigned_at);
}
```

- `.attach(id)` - INSERT 一条中间表行。重复时会报错，除非您的中间表允许重复（框架不会在 Rust 层去重；要幂等就用 `.sync`）。
- `.attach_with(id, attrs! { ... })` - 带着额外中间表列的 INSERT。当 `with_timestamps` 开启时会打上时间戳。
- `.detach(id)` - DELETE 连接父级 → id 的中间表行。
- `.sync([ids...])` - 差量应用：attach 新增的，detach 缺失的，交集部分保持不动。包在一个事务里。

`.get()` 会返回 `Vec<R>`，每一行内部的 `__pivot` 字段上都盖着中间表数据。`.pivot::<P>()` 这个访问器，会把 `Arc<dyn Any>` 向下转型成您声明的那个中间表类型。用错误的类型调用它会 panic - 请让这个类型和您声明的中间表匹配。

### `HasOneThrough<B, R>` 与 `HasManyThrough<B, R>`

经过一个中间者 `B`，触达最终目标 `R`。适合这种场景：这个关系要跨越两张表，但您不需要把这个中间者暴露出来（`A → B → R`）。

```rust
#[model(table = "countries", relations = {
    posts: HasManyThrough<crate::models::User, crate::models::Post>,
})]
pub struct Country {
    pub id: i64,
    pub name: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

let c = Country::find(1).await?.unwrap();
let posts: Vec<Post> = c.posts().get().await?;
```

这个分发器会从结构体名推断出 JOIN 键。覆盖项：

| 选项              | 默认值                          | 说明 |
|---------------------|----------------------------------|-------------|
| `first_key`         | `<snake(parent_struct)>_id`      | 中间者 `B` 上指向父级 `A` 的那一列。 |
| `second_key`        | `<snake(intermediate_struct)>_id` | 最终目标 `R` 上指向中间者 `B` 的那一列。 |
| `second_local_key`  | `"id"`                           | 中间者 `B` 上被 `second_key` 匹配的那一列。当 `B` 用的主键不是 `id` 时必填。 |

父级的主键列，是从这个模型的 `primary_key` 声明里读取的（默认是 `"id"`）- `HasManyThrough` / `HasOneThrough` 上没有 `local_key` 覆盖项；如果您需要一个不是 `id` 的父级键，就通过 `#[suprnova::model]` 属性去改父级的主键。

```rust
#[model(table = "countries", relations = {
    posts: HasManyThrough<crate::models::User, crate::models::Post> {
        first_key = "country_id",
        second_key = "author_id",
    },
})]
pub struct Country { /* ... */ }
```

### `MorphTo`：`targets = [...]` 与按族的枚举

多态关系会把一个子级行指向几个父级族当中的一个。子级携带着一对 `(<name>_id, <name>_type)`；`*_type` 列存放的是每一个父级声明的那个多态类型字符串。

`MorphTo` 活在子级一侧。它的声明通过 `targets = [...]` 列出它可以指向的每一个父级族。这个宏会生成一个按族的枚举，名字是 `<RelationName>Morph`（匹配这个关系名的 PascalCase 形式，后缀 `Morph`），每一个目标类型一个变体，外加一个 `Unknown(String, i64)`，用于那些 `<name>_type` 值和任何已注册目标都不匹配的遗留行。

```rust
#[model(table = "posts", morph_type = "post")]
pub struct Post { /* ... */ }

#[model(table = "videos", morph_type = "video")]
pub struct Video { /* ... */ }

#[model(table = "comments", relations = {
    commentable: MorphTo {
        name = "commentable",
        targets = [
            crate::models::Post,
            crate::models::Video,
        ],
    },
})]
pub struct Comment {
    pub id: i64,
    pub commentable_id: i64,
    pub commentable_type: String,
    pub body: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

let c = Comment::find(1).await?.unwrap();
match c.commentable().get().await? {
    CommentableMorph::Post(post)   => println!("comment on post {}", post.title),
    CommentableMorph::Video(video) => println!("comment on video {}", video.url),
    // 遗留 / 悬空的行 - `<name>_type` 不匹配任何目标，
    // 或者 morph_type 匹配了，但 `<name>_id` 那一行已经不存在了。
    CommentableMorph::Unknown(ty, id) => {
        eprintln!("comment {} points at unknown {ty}#{id}", c.id);
    }
}
```

每一个目标结构体上的 `morph_type = "..."` 属性，就是加载器在插入时写进子级 `<name>_type` 列、在读取时用来过滤的那个东西。没有 `morph_type` 时，框架会从 `to_snake(struct_name)` 推导出这个类型字符串。

`MorphTo` 的分发 - 也就是这个按族的枚举怎么挑出正确的变体 - 会查询这个运行时的多态注册表（由每一个 `#[suprnova::model(morph_type = "...")]` 声明填充的那个 inventory）。对每一个已声明的目标，这个取数帮助函数会查找这个目标的 `TypeId`，读取已注册的 `morph_type` 字符串，再把它和子级行上存储的 `<name>_type` 值做比较。按声明顺序，第一个匹配的获胜。没有显式 `morph_type` 属性的目标，会回落到 `to_snake(target_type_name)` - 这和父级一侧的 `MorphMany` / `MorphOne` 在写入时用来盖类型字符串戳的默认值相同，所以两侧能保持一致。这意味着自定义的 `morph_type` 值（例如一个叫 `Post` 的结构体上的 `morph_type = "blog_post"`，或者任何不走常规的字符串），不需要改动声明处就能正确分发。

### `MorphOne<R>` 和 `MorphMany<R>` - 父级一侧

`MorphTo` 的反方向：一个父级类型声明它拥有的那个多态的一或多关系。`MorphOne` 从 `.first()` 返回 `Option<R>`；`MorphMany` 从 `.get()` 返回 `Vec<R>`。两者都会用 `self.id` 和父级的 `morph_type`，去过滤子级的 `(<name>_id, <name>_type)` 这对列。

```rust
#[model(table = "posts", morph_type = "post", relations = {
    comments: MorphMany<crate::models::Comment> {
        name = "commentable",
    },
    cover: MorphOne<crate::models::Image> {
        name = "imageable",
    },
})]
pub struct Post { /* ... */ }

#[model(table = "videos", morph_type = "video", relations = {
    comments: MorphMany<crate::models::Comment> {
        name = "commentable",
    },
})]
pub struct Video { /* ... */ }

let post = Post::find(1).await?.unwrap();
let post_comments: Vec<Comment> = post.comments().get().await?;
let post_cover:    Option<Image> = post.cover().first().await?;

let video = Video::find(1).await?.unwrap();
let video_comments: Vec<Comment> = video.comments().get().await?;
// post.comments() 只返回 `commentable_type = "post"` 的行；
// video.comments() 只返回 `commentable_type = "video"` 的行。
```

和 `HasMany` / `HasOne` 一样的可链式调用表面：`.filter` / `.db_where`、`.order_by` / `.latest` / `.oldest`、`.limit` / `.take`、`.first` / `.get` / `.count`。

### `MorphToMany<R, P>` 和 `MorphedByMany<R, P>`

多态多对多。这张共享的中间表 `P`，携带着外键对，外加一个 `<name>_type` 鉴别器列。一端声明 `MorphToMany`（例如 `Post.tags()`、`Video.tags()`），另一端为每一个目标族声明一个 `MorphedByMany`（例如 `Tag.posts()`、`Tag.videos()`）。

```rust
#[model(table = "taggables", fillable = ["tag_id", "taggable_id", "taggable_type"])]
pub struct Taggable {
    pub id: i64,
    pub tag_id: i64,
    pub taggable_id: i64,
    pub taggable_type: String,
}

#[model(table = "posts", morph_type = "post", relations = {
    tags: MorphToMany<crate::models::Tag, Taggable> {
        name = "taggable",
    },
})]
pub struct Post { /* ... */ }

#[model(table = "videos", morph_type = "video", relations = {
    tags: MorphToMany<crate::models::Tag, Taggable> {
        name = "taggable",
    },
})]
pub struct Video { /* ... */ }

// 反向：Tag 为每一个目标族声明一个 MorphedByMany。
#[model(table = "tags", relations = {
    posts: MorphedByMany<crate::models::Post, Taggable> {
        name = "taggable",
        target_morph_type = "post",
    },
    videos: MorphedByMany<crate::models::Video, Taggable> {
        name = "taggable",
        target_morph_type = "video",
    },
})]
pub struct Tag { /* ... */ }

let post  = Post::find(1).await?.unwrap();
let video = Video::find(1).await?.unwrap();
let tag   = Tag::create(attrs! { name: "rust" }).await?;

// `attach` / `attach_with` / `detach` / `sync` 的工作方式和
// BelongsToMany 一样。`<name>_type` 列会自动从
// 调用方那个父级的 `morph_type` 落进去。
post.tags().attach(tag.id).await?;
video.tags().attach(tag.id).await?;          // 独立的绑定
post.tags().sync([tag_a.id, tag_b.id]).await?;

// 反方向 - Tag 按族拆分：
let posts_with_tag:  Vec<Post>  = tag.posts().get().await?;   // 类型化为 "post"
let videos_with_tag: Vec<Video> = tag.videos().get().await?;  // 类型化为 "video"
```

`MorphedByMany` 的 `target_morph_type` 是必填的，因为在 `Tag` 声明处的这个宏，没法反射出目标的 `morph_type = "..."` 属性（它活在一个独立的 `#[suprnova::model]` 调用里）。显式设置它，能让每一个 `MorphedByMany` 分支，对自己在扫描哪个族保持诚实。

### 脱围机制：手写的关系方法

在 `relations = { ... }` 里声明的那些关系，是预加载分发器（以及 `with`、`with_count` 等）唯一知道的关系。如果一个关系对这个宏的形状来说太不寻常了 - 比如一个跨两张中间表做聚合的查询，或者一个反规范化缓存表的类型化视图 - 您可以把它从 `relations = { ... }` 里省略掉，改写一个朴素的固有 impl：

```rust
impl User {
    /// 这个用户创作的、或者被打上标签的 posts。跨越了两个关系，
    /// 因此没法表示成单一一条 `relations = { ... }` 声明 - 手写的。
    pub async fn posts_touched(&self) -> Result<Vec<Post>, FrameworkError> {
        let authored: Vec<Post> = self.posts().get().await?;
        let tagged:   Vec<Post> = /* ...custom query... */;
        // ……合并 + 去重……
        Ok(/* ... */)
    }
}
```

这样的方法会失去预加载支持 - `User::with(["posts_touched"])` 会报错，因为这个分发器没有 `posts_touched` 对应的分支。宏内声明，仍然是框架知道该怎么预加载、计数、聚合和谓词过滤的那条路径。

### v1 的限制

有少数几件事，v1 的这个表面暂时搁置了。每一件事在它的声明处也都有文档说明 - 这里把它们收集起来，方便查看：

- **多态 ID 只支持 `i64`。** `MorphTo::morph_id` 被硬编码成了 `i64`，所以任何用作 `MorphTo` 目标的模型，都必须声明一个 `i64` 主键，子级表的 `<name>_id` 列也必须是 `i64`。字符串 / 以字符串表示的 UUID 多态外键是 v2 的事。
- **不支持穿过 `MorphTo` 做嵌套预加载。** 这个按族的枚举会抹掉子级类型，所以像 `with(["commentable.user"])` 这样的点号路径没法尾递归 - 这个分发器会返回一个类型化的错误。请按族逐个解决：对这个枚举做匹配，再对每一个变体分别调用 `with(["user"])`。

## 预加载

预加载能避开 N+1 查询。Suprnova 不会用 `posts.len()` 次查询去取每个用户的 posts，而是不管加载了多少父级行，每个顶层关系只发一次查询。

完整的表面 - 扁平列表、嵌套路径、计数、聚合，以及谓词过滤的预加载 - 都是通过 `#[suprnova::model]` 在每个模型上生成的那些辅助方法来触达的：

```rust
// 单个关系：
let users = User::with(["posts"]).get().await?;
for u in &users {
    for p in u.posts_loaded() { /* ... */ }
}

// 多个关系：
let users = User::with(["posts", "profile"]).get().await?;

// 嵌套路径 - 三次查询（users + posts + comments），没有 N+1：
let users = User::with(["posts.comments"]).get().await?;
let p1 = users[0].posts_loaded()[0];
let comments = p1.comments_loaded();

// 更深的嵌套也能正常工作：
let users = User::with(["posts.comments.author"]).get().await?;

// 和父级行一起计数：
let users = User::with_count(["posts"]).get().await?;
for u in &users {
    println!("{} has {} posts", u.name, u.posts_count());
}

// 聚合 - 对一个关系列做 Sum / Avg / Min / Max。符合人体工学的
// 读取方式，是这个宏生成的 `<rel>_sum_of(col)` 访问器。
let users = User::with_sum(("posts", "views")).get().await?;
let sum: f64 = users[0]
    .posts_sum_of("views")
    .expect("with_sum populated the cache");

// 同一个关系上的多个聚合可以组合 - 缓存键用的是
// 宽形态的 `<rel>_<kind>_<col>`，所以不同的种类和
// 不同的列不会冲突：
let users = User::with_sum(("posts", "views"))
    .with_avg(("posts", "views"))
    .with_min(("posts", "id"))
    .get()
    .await?;
let u = &users[0];
let sum = u.posts_sum_of("views").unwrap();   // Some(_) - views 的求和
let avg = u.posts_avg_of("views").unwrap();   // Some(_) - views 的均值
let min = u.posts_min_of("id").unwrap();      // Some(Some(_)) - 非空的分组
let max = u.posts_max_of("id");               // None - with_max 没被调用

// 过滤预加载出来的子级。这个宏会为每一个关系生成一个类型化的
// `with_where_<rel>(closure)` 静态辅助方法，这样这个闭包的
// 参数类型就能被推断出来 - 不需要显式写出 `Builder<Post>`：
let users = User::with_where_posts(|q| q.filter("published", true))
    .get()
    .await?;
// 返回的这个 `Builder<User>`，可以和任何其他基础查询
// 构造器方法链接起来：
let users = User::with_where_posts(|q| q.filter("published", true))
    .filter("active", true)
    .get()
    .await?;
// 泛型形式仍然可用 - 当关系名是在运行时计算出来的时候很有用 -
// 但您需要在这个闭包上写明目标类型：
let users = User::query()
    .with_where(("posts", |q: Builder<Post>| q.filter("published", true)))
    .get()
    .await?;
// 每一个 u.posts_loaded() 都只包含已发布的 posts。
```

### 缓存布局

逐行的 `__eager` 缓存单元，用下面这些方式做键：

- `<rel>`（单纯的关系名）用于 `with` 和 `with_count`。
- `<rel>_<kind>_<col>`（例如 `posts_sum_views`）用于四种聚合类型 -
  `with_sum` / `with_avg` / `with_min` / `with_max`。这个宽键让同一个关系上的多个聚合，能在同一行上共存，而不会互相覆盖。

| 方法                              | 缓存键            | 缓存单元类型   | 空分组时的值 |
|-------------------------------------|----------------------|-------------------|-------------------|
| `with(["posts"])`                   | `posts`              | `Vec<Post>`       | `Vec::new()`      |
| `with(["profile"])`                 | `profile`            | `Option<Profile>` | `None`            |
| `with_count(["posts"])`             | `posts`              | `u64`             | `0`               |
| `with_sum(("posts","views"))`       | `posts_sum_views`    | `f64`             | `0.0`             |
| `with_avg(("posts","views"))`       | `posts_avg_views`    | `f64`             | `0.0`             |
| `with_min(("posts","id"))`          | `posts_min_id`       | `Option<f64>`     | `None`            |
| `with_max(("posts","id"))`          | `posts_max_id`       | `Option<f64>`     | `None`            |

这个宏会在每一个模型上生成匹配的访问器：

- `<rel>_loaded()` - 对集合类关系：`&[Post]`（如果这个关系没有被预加载就会 panic）。对单值关系：`Option<&Profile>`。
- `<rel>_count()` - `u64`。如果没调用 `with_count(["..."])` 就会
  panic。
- `<rel>_sum_of(col)` / `<rel>_avg_of(col)` - 返回 `Option<f64>`
  （如果没调用匹配的 `with_sum` / `with_avg` 就是 `None`）。
- `<rel>_min_of(col)` / `<rel>_max_of(col)` - 返回
  `Option<Option<f64>>`：外层 `Option` 表示“有没有调用 `with_min` /
  `with_max`？”，内层 `Option` 表示“SQL 是不是因为分组为空而返回了
  NULL？”。

这些访问器就是符合人体工学的那个表面 - 请经由它们读取，而不要直接伸手进 `__eager.get_aggregate::<T>(...)`。它们在底层用的是同一个缓存键，经由 `eloquent::relations::aggregate_cache_key` 构建。

### 在同一个关系上组合聚合

这个宽缓存键意味着，您可以在一次查询里，在同一个关系上叠加任意多个 `with_*` 调用 - 不会冲突：

```rust
let users = User::with_sum(("posts", "views"))
    .with_avg(("posts", "views"))
    .with_min(("posts", "id"))
    .with_max(("posts", "id"))
    .get()
    .await?;

let u = &users[0];
let total_views: f64 = u.posts_sum_of("views").unwrap();
let avg_views:   f64 = u.posts_avg_of("views").unwrap();

// Min/Max 是双层 Option，因为 SQL 的 min/max 在为空时会是 NULL：
match u.posts_min_of("id") {
    None              => panic!("with_min not called"),
    Some(None)        => println!("no posts yet"),
    Some(Some(min))   => println!("smallest post id: {min}"),
}

// 当匹配的 `with_*` 被跳过时，这个访问器会返回 `None`：
assert!(u.posts_avg_of("score").is_none()); // 从没用 col="score" 调用过
```

### 聚合与 INTEGER 列

对一个 INTEGER 列做 SUM，落进缓存时会是 `f64`。这个分发分支会先试 `try_get::<Option<f64>>`，再回落到 `try_get::<Option<i64>>().map(|n| n as f64)`，这样 SQLite 那种保留 INTEGER 的 COUNT/SUM 类型，就不会被静默强转成 `0.0`。不管源列是什么类型，都请经由这个宏生成的访问器读取。

### `with_where` 谓词路由

`User::with_where_posts(|q| q.filter("published", true))` 会在 `filter_in(<fk>, parent_ids)` 这条 IN 查询发出**之前**，把一个闭包应用到内部的 `Builder<Post>` 上，所以只有匹配的子级行才会进到缓存里。这个宏会为每一个已声明的关系生成一个类型化的 `with_where_<rel>` 静态辅助方法，所以这个闭包的参数类型是从方法签名推断出来的。

泛型形式 `with_where(("posts", |q: Builder<Post>| q.filter("published", true)))` 仍然可用 - 当关系名是在运行时计算出来的时候很有用，或者当您已经持有一个 `Builder<User>`、想附加一个谓词时也很有用。它要求在这个闭包上写明目标类型，因为这个谓词要经过一个 `Box<dyn Any>`，Rust 没法只从关系名推断出类型。（Rust 的孤儿规则不允许这个宏直接在 `Builder<User>` 上加一个类型化方法，所以这个类型化的简写形式只在模型上提供 - `User::with_where_<rel>` - 不是一个构造器链上的方法。）

对多态类型来说，这个谓词是针对相关表那条查询运行的 - 不是针对中间表扫描。

`with_where` 在除 `MorphTo` 之外的每一种关系类型上都支持。MorphTo 那个按族的枚举会抹掉子级类型，所以没有单一的 `Builder<R>` 能覆盖所有变体。穿过 MorphTo 做嵌套预加载，在 v1 里也不支持 - 当 `commentable` 是一个 `MorphTo` 时，`with(["commentable.user"])` 会从这个递归预加载分发器那里返回一个错误。

### `Collection::load` / `load_missing`

当您已经取出了行，事后又想预加载关系时：

```rust
use suprnova::Collection;

let mut users: Collection<User> = User::all().await?.into();
users.load(["posts.comments"]).await?;
```

`load_missing` 是逐行的：集合里的每一行都会被独立地分区。已经缓存了这个具名关系的行保持不动；没有缓存的行会加载这个关系。这和 Laravel 的 `$collection->loadMissing(...)` 语义相呼应。

对嵌套路径来说，这个分区会在每一层重复。以 `load_missing(["posts.comments"])` 为例：

- 没有缓存 `posts` 的行，会加载**完整**的路径 - `posts` 加上它们的 `comments`。
- 已经缓存了 `posts` 的行，会递归进这些已缓存的 posts，只对那些还没缓存 comments 的 posts 加载 `comments`。

同样这种逐行分区，会在一条更长的点号路径的每一个后续片段上重复（`"posts.comments.author"` 等）- 每一步，只有缺失那个片段的行会被批量加载。

## 分页

三种分页器类型，都是构建在 `Builder<M>` 之上的：

| 方法 | 返回类型 | 每页的查询次数 | 何时使用 |
|--------|---------|------------------|----------|
| `paginate(per_page)` | `LengthAwarePaginator<M>` | 2（COUNT + LIMIT） | UI 需要总页数 |
| `simple_paginate(per_page)` | `Paginator<M>` | 1（LIMIT + 1） | 大表；只有“下一页”按钮 |
| `cursor_paginate(per_page)` | `CursorPaginator<M>` | 1（LIMIT + 1） | 无限滚动；深度分页 |

三者都实现了 `Serialize`，用的是 Laravel 标准的 JSON 形态，所以能直接发给 Inertia / JSON 消费者，不需要重新塑形。

### 长度感知

```rust
use suprnova::LengthAwarePaginator;

let page: LengthAwarePaginator<User> = User::query()
    .filter("active", true)
    .order_by_desc("created_at")
    .paginate(20)
    .await?;

// page.data: Vec<User>
// page.total: u64 - 跨所有页面的总行数
// page.last_page: u64 - 从 1 开始计数的最后一页索引
// page.current_page: u64
// page.per_page: u64
// page.from / page.to: Option<u64> - 从 1 开始计数的窗口边界
// page.path: Option<String> - 用于生成链接的可选基础 URL
```

页码参数的解析，是经由 `Context::query_param`，从活跃请求里读取 `?page=N`。要在同一个页面上，用各自的查询键给多个列表分页，就用 `paginate_using`：

```rust
let posts = Post::query().paginate_using("posts_page", 10).await?;
let comments = Comment::query().paginate_using("comments_page", 25).await?;
```

**JSON 形态：**

```json
{
  "data": [...],
  "current_page": 1,
  "last_page": 3,
  "per_page": 10,
  "total": 25,
  "from": 1,
  "to": 10,
  "path": "/api/users"
}
```

`path` 在未设置时，会从 JSON 里省略。

### 简单分页（不计数）

`paginate` 总是运行两次查询 - 一次 `COUNT(*)`，加上这次分页取数。在大表上，单是这次计数就可能主导请求耗时。`simple_paginate` 完全跳过这次计数；取而代之的是获取 `per_page + 1` 行，并通过 `has_more` 标志报告是否还有下一页：

```rust
use suprnova::Paginator;

let page: Paginator<User> = User::query()
    .order_by_desc("id")
    .simple_paginate(20)
    .await?;

// page.has_more: bool - 超出 per_page 之外是否还有多余的一行？
// page.current_page、page.per_page、page.data、page.path：同上。
```

**JSON 形态：**

```json
{
  "data": [...],
  "current_page": 1,
  "per_page": 10,
  "has_more": true
}
```

### 游标分页（键集）

游标分页是无限滚动、深度分页，或者任何一种场景的首选 - 只要一份稳定的行顺序，加上每页 O(1) 的低成本寻址，比一个数字页码 UI 更有价值。它是双向的 - 会读取 `?cursor=<opaque>` 这个查询参数，按这个游标的方向向前或向后走，并且在页面的邻居存在时，同时给出 `next_cursor` 和 `prev_cursor`（和 Laravel 的 `cursorPaginate()` 相呼应）。

```rust
use suprnova::CursorPaginator;

let page: CursorPaginator<User> = User::query()
    .cursor_paginate(20)
    .await?;

// page.data: Vec<User>
// page.per_page: u64
// page.next_cursor: Option<String> - 下一页的不透明游标（最后一页时是 None）
// page.prev_cursor: Option<String> - 上一页的不透明游标（第一页时是 None）
// page.path: Option<String>
```

游标是经由 `CursorPaginator::encode_value` **加密并认证**过的 - 它们编码的是这个键集边界（模型的主键）加上一个方向标签，用框架的 `APP_KEY` 做了 AES-256-GCM 封装。篡改会产出一个 400 ParamParse 错误；这个游标对客户端是不透明的，没有这个密钥就伪造不了。

下一次请求会通过 `?cursor=<opaque>` 传递这个游标：

```
GET /api/users?cursor=eyJ0IjoiQmlnSW50IiwidiI6MTAwLCJkIjoibmV4dCJ9...
```

游标分页会**替换**这个构造器上任何已有的 `ORDER BY` - `gt(boundary)` 要确定性地切片，需要一个稳定的主键 ASC 排序。

**JSON 形态：**

```json
{
  "data": [...],
  "per_page": 10,
  "next_cursor": "...",
  "prev_cursor": null,
  "path": "/api/users"
}
```

`next_cursor` 和 `prev_cursor` 作为 JSON 键永远存在（不存在时会发出 `null`），这样客户端的 schema 就能依赖这个字段一定存在；`path` 在未设置时会被省略。

### 错误

| 条件 | 变体 | HTTP |
|-----------|---------|------|
| `per_page == 0` | `FrameworkError::ParamError { param_name: "per_page" }` | 400 |
| 无效的游标（base64、JSON 有问题，或者 HMAC 校验失败） | 来自 `Crypt::decrypt_string` 的 `FrameworkError::Internal` | 500 |
| 底层数据库故障 | `FrameworkError::Database` | 500 |

游标认证失败会以 `Internal` 的形式暴露出来（不是 `ParamParse`），这样一个被篡改的游标就不会向客户端泄露协议层面的信息；响应体里仍然会带着一个人类可读的原因。

### 在真实请求之外读取查询参数

测试、控制台命令和后台工作进程，都不会在一个 hyper 请求内部运行 - 所以 `Context::query_param("page")` 会返回 `None`，`paginate` 会回落到第 1 页。需要针对一个具体页面做测试的用例，可以安装一个逐线程的覆盖：

```rust
use suprnova::context::Context;

#[tokio::test]
async fn paginate_page_2() {
    Context::test_clear_query();
    Context::test_set_query("page", "2");

    let page = User::query().paginate(10).await.unwrap();
    assert_eq!(page.current_page, 2);

    Context::test_clear_query();
}
```

`test_set_query` / `test_clear_query` 是挂在 `testing` 这个 feature 后面的（在 `framework/Cargo.toml` 里默认启用），所以发布构建永远看不到这个表面。

## 分块与惰性迭代

`Builder<M>` 上有七个流式入口，让您能在有限内存里处理大型结果集。按权衡来选：

| 方法 | 分页方式 | 并发安全？ | 返回类型 |
|--------|-----------|------------------|---------|
| `chunk(n, async \|batch\| { ... })` | OFFSET | 不 | `Result<(), _>` |
| `chunk_by_id(n, async \|batch\| { ... })` | 主键游标 | **是** | `Result<(), _>` |
| `chunk_map(n, async \|batch\| { ... })` | OFFSET | 不 | `Collection<U>` |
| `each(async \|row\| { ... })` | OFFSET，大小为 1 | 不 | `Result<(), _>` |
| `lazy()` | 主键游标，批大小 1000 | **是** | `LazyCollection<M>` |
| `lazy_by_id(batch_size)` | 主键游标，自定义批大小 | **是** | `LazyCollection<M>` |
| `cursor()` | `lazy()` 的别名 | **是** | `LazyCollection<M>` |

### chunk - OFFSET 分页批次

```rust
use suprnova::{Collection, Model};

User::query().chunk(100, |batch: Collection<User>| async move {
    for user in &batch {
        send_welcome_email(user).await?;
    }
    Ok(())
}).await?;
```

这个闭包每批会收到一个 `Collection<M>` - 切片形态的访问（`.iter()`、索引）经由 `Deref` 直接就能用。

`chunk` 是 OFFSET 分页的，**在并发插入下不安全**：在下一批的偏移量之前插入的行会被跳过；在偏移量之前删除的行，会导致某个移进它们位置的行被处理两次。对写入负载下的表做生产级批量处理，请用 `chunk_by_id`。

### chunk_by_id - 主键游标批次，并发安全

```rust
User::query().chunk_by_id(500, |batch| async move {
    for user in &batch {
        reindex_user(user).await?;
    }
    Ok(())
}).await?;
```

每一批都用 `WHERE id > last_id ORDER BY id ASC LIMIT n` 来过滤，所以迭代中途插入的、主键在这个游标之上的行，会落进后面的某一批（或者被后续的一次运行捡起来）- 它们永远不会导致原有的行被跳过或者重复。

`chunk_by_id` 要求一个 `i64` 主键。主键是 `String` / `Uuid` 的模型，请用带着 OFFSET 那个注意事项的 `chunk`。（把这个游标形态泛化到非 `i64` 键，在后续计划清单上。）

### chunk_map - chunk + 逐块 map

```rust
let totals: Collection<i64> = Order::query()
    .chunk_map(1000, |batch| async move {
        let sum: i64 = batch.iter().map(|o| o.amount).sum();
        Ok(Collection::from_vec(vec![sum]))
    })
    .await?;
```

把每一批都通过 `f` 做映射，拼接映射后的输出，返回单一一个 `Collection<U>`。只有当 `U` 严格小于 `M` 时，内存才是有界的 - 当您产出的是摘要（逐批的总计、id、聚合），而不是变换后的行时，就选这个。

### each - 逐行处理，OFFSET

```rust
User::query().each(|user| async move {
    send_welcome_email(&user).await?;
    Ok(())
}).await?;
```

是 `chunk(1, ...)` 的语法糖 - 每一行一次查询。对大数据集，请换成 `lazy()`，它在内部会分批（默认每次取 1000 行），同时仍然会一次向消费者暴露一行。

### lazy / lazy_by_id / cursor - 流

```rust
let mut stream = User::query().lazy();
while let Some(row) = stream.next().await {
    let user = row?;
    println!("{}", user.email);
}
```

`lazy()` 返回一个 `LazyCollection<M>` - 一个 `Send` 的流包装器，逐行产出 `Result<M, FrameworkError>`。反压是自然工作的：一个慢消费者会停在这个 `await` 点上，下一批只有在内存缓冲区排空之后才会去取。

`lazy()` 会经由主键游标分批，默认大小是 1000 行。用 `lazy_by_id(500)` 可以覆盖这个批大小。`cursor()` 是 Laravel 那边的名字，是 `lazy()` 的一个零成本别名。

和 `chunk_by_id` 一样的 `i64` 主键限制。

### 分块内部的预加载

全部七个入口，都会带着一个醒目的 `FrameworkError::internal`，提前拒绝 `.with(...)`。这个构造器跨批次的克隆，会丢掉这个类型擦除的预加载计划（它那个装箱的 `dyn Any` 谓词，没有收紧公开 API 就没法克隆），所以尊重这个计划会导致跨批次悄无声息的不一致。需要时，请在逐块闭包内部重新应用 `.with(...)` - 每一批的 `Collection<M>`，都能和 `load(...)` / `load_missing(...)` 组合起来：

```rust
User::query().chunk(100, |batch| async move {
    let mut batch = batch;
    batch.load("posts").await?;
    for u in &batch {
        let posts = u.posts_loaded();
        // ...
    }
    Ok(())
}).await?;
```

## 集合

`Collection<T>` 是 Suprnova 那个 Laravel 形状的集合类型 - 是 `Builder::get`（这里 `T` 是模型）、`Model::all`、`pluck` / `chunk_map`，以及其他每一个产出多于一行的终结方法的返回类型。它会解引用成 `&[T]`，所以既有的 Vec 调用点不用改动就能继续用；Laravel 那套表面是叠在上面的。本节是日常会用到的表面；完整的方法索引、通用与感知模型两种表面的区分，`LazyCollection<M>` 这个流式包装器，以及借用与消费的规则，都在[Eloquent 集合](eloquent-collections.md)。

### 通用表面

不管 `T` 是什么，每一个 `Collection<T>` 上都能用：

```rust
use suprnova::Collection;

let nums: Collection<i32> = Collection::from_vec(vec![3, 1, 4, 1, 5, 9]);

nums.first();              // Some(&3)
nums.last();               // Some(&9)
nums.len();                // 6
nums.is_empty();           // false
nums.contains(&4);         // true
// 谓词闭包收到的是 `&&T` - 注意这个双重解引用 `**n`：
nums.first_where(|n| **n > 3);    // Some(&4)
nums.contains_where(|n| **n > 8); // true
// 要计数，就内联运行这个谓词：`nums.iter().filter(|n| **n > 2).count()` - 4
```

变换方法会消费 `self`，返回一个新的 `Collection`：

```rust
let doubled: Collection<i32> = nums.clone().map(|n| n * 2);
let evens:   Collection<i32> = nums.clone().filter(|n| n % 2 == 0);
let chunks:  Vec<Collection<i32>> = nums.clone().chunk(2); // [[3,1],[4,1],[5,9]]
let unique:  Collection<i32> = nums.clone().unique();
let sorted:  Collection<i32> = nums.clone().sort();
```

### `Collection<M>` 上感知模型的方法

当 `T` 是一个模型时，额外的字符串键方法，会路由经过这个宏生成的 `field_value(name)` 访问器：

```rust
let users: Collection<User> = User::query().get().await?;

let emails: Collection<String> = users.pluck::<String>("email");
let by_role: HashMap<String, Vec<User>> =
    users.clone().group_by::<String>("role");
let active: Collection<User> = users.clone().where_eq("active", true);

let total: f64 = users.clone().sum::<f64>("balance");
let avg:   f64 = users.clone().avg::<f64>("balance");
let max:   Option<i64> = users.clone().max::<i64>("login_count");
```

基于闭包的 `pluck_by`，是那个类型化的替代方案 - 当字段名原本需要一次类型系统没法检查的字符串查找时很有用：

```rust
let names: Collection<String> = users.pluck_by(|u| u.name.clone());
```

逐行的 `field_value(name)` 返回 `Option<serde_json::Value>` - 当列名不匹配任何已声明字段时是 `None`。序列化失败的自定义转换，也会以 `None` 的形式暴露出来。字符串键方法会静默跳过这些行；闭包形式会在闭包体内部短路，让调用者自己决定。

### 通过 `LazyCollection` 流式处理

对太大而没法物化的数据集，`Builder::lazy()` / `lazy_by_id(n)` / `cursor()` 会返回一个 `LazyCollection<M>` - 一个按主键游标分批取行的 `Stream` 包装器。参见[分块与惰性迭代](#分块与惰性迭代)。

### 在一个集合上预加载

`Collection::load(["posts"])` / `load_missing(["posts"])` 执行的，是和一条 `Builder::with(...)` 链发出的同一套预加载分发，只是针对的是一个已有的集合。`load_missing` 是逐行的：集合里的每一行都会被分到“需要加载”/“已经加载”两个桶里，只有缺失的那些才会被批量加载。参见[预加载](#预加载)。

## 批量赋值

### Fillable 允许列表

```rust
#[model(
    table = "users",
    fillable = ["name", "email"],
)]
pub struct User { /* ... */ }

User::create(attrs! {
    name: "Alice",
    email: "alice@example.com",
    admin: true,    // 在运行时被静默丢弃 - 不在 fillable 里
}).await?;
```

### Guarded 拒绝列表

`guarded` 是反过来的 - 除了被 guarded 的那些字段之外，每一个字段都是 fillable 的。和 `fillable` 互斥；两者同时使用，会是这个宏产出的一个编译期错误。

```rust
#[model(
    table = "posts",
    guarded = ["id", "user_id"],   // 其他一切都是 fillable 的
)]
pub struct Post { /* ... */ }
```

### 默认策略

当 `fillable` 和 `guarded` 都没设置时，默认策略是 `guarded = ["id"]`（或者 `primary_key = "..."` 解析出来的那个值）- 除了主键之外，每一个字段都是 fillable 的。这和 Laravel 那个“除主键外所有字段都可填充”的默认值一致。

### `unguarded(closure)` 脱围机制

`unguarded(closure)` 会对一个代码块关闭这个过滤器：

```rust
use suprnova::eloquent::unguarded;

// 为一个一次性的数据迁移脚本绕开这个过滤器：
unguarded(|| async {
    User::create(attrs! {
        name: "Bootstrap",
        email: "boot@example.com",
        admin: true,    // 在这个闭包内部可赋值
    }).await
}).await?;
```

实现方式：一个 `tokio::task_local!` 布尔值，`Fillable::apply` 过滤器会在运行前检查它。任务本地意味着，并发的请求不会受到另一个任务的 `unguarded` 作用域的影响。

## 转换

转换运行在存储（列值）和运行时（模型字段）之间的边界上。每一种转换类型都实现了 `Cast` trait。内置转换覆盖了 Laravel 的完整集合；用户可以经由这个 trait 注册自定义转换。本节是速查索引；完整的逐转换契约 - 基元、时间性、结构化、枚举、加密、哈希，外加 `casts!` 这个运行时覆盖宏 - 都在[Eloquent 转换、访问器和修改器](eloquent-mutators.md)。

### 仅限显式

转换是在 `#[model(casts = { ... })]` 里声明的 - 不会从字段类型自动检测。一个 `prefs: Json` 字段不会隐式变成 `AsJson`；您要写 `casts = { prefs = AsJson }`。理由：您应该能读一读这个模型，就确切知道在存储边界上运行的是什么。没有魔法。

### 示例

```rust
use suprnova::{model, AsArray, AsBool, AsCollection, AsDate, AsDateTime,
    AsEncrypted, AsEnum, AsObject, AsTimestamp};

#[model(
    table = "users",
    casts = {
        active        = AsBool,
        preferences   = AsArray<String>,
        options       = AsObject<UserOptions>,
        profile       = AsCollection<ProfileField>,
        birthday      = AsDate,
        last_seen_at  = AsDateTime,
        role          = AsEnum<UserRole>,
        api_token     = AsEncrypted,
    },
)]
pub struct User { /* ... */ }
```

### 完整的 Laravel 转换列表与 Suprnova 对应关系

| Laravel 转换 | Suprnova 转换 | 运行时类型 |
|--------------|---------------|--------------|
| `bool`, `boolean` | `AsBool` | `bool` |
| `int`, `integer` | `AsInt<I>` | `I: PrimInt` |
| `float`, `double`, `real` | `AsFloat` | `f64` |
| `decimal:N` | `AsDecimal<N>` | `rust_decimal::Decimal` |
| `string` | `AsString` | `String` |
| `array` | `AsArray<T>` | `Vec<T>`（JSON 编码） |
| `object` | `AsObject<T>` | `T: Serialize + DeserializeOwned` |
| `collection` | `AsCollection<T>` | `Collection<T>` |
| `json` | `AsJson<T>` | `T`（原始 JSON 列） |
| `date`, `date:format` | `AsDate` | `chrono::NaiveDate` |
| `datetime`, `datetime:format` | `AsDateTime` | `chrono::DateTime<Utc>` |
| `immutable_date` | `AsImmutableDate` | `chrono::NaiveDate` |
| `immutable_datetime` | `AsImmutableDateTime` | `chrono::DateTime<Utc>` |
| `timestamp` | `AsTimestamp` | `i64`（unix 纪元） |
| `encrypted` | `AsEncrypted` | `String`（经由 `Crypt` 加密） |
| `encrypted:array` | `AsEncryptedArray<T>` | `Vec<T>`（JSON + 加密） |
| `encrypted:object` | `AsEncryptedObject<T>` | `T`（JSON + 加密） |
| `encrypted:collection` | `AsEncryptedCollection<T>` | `Collection<T>` |
| `EnumClass::class` | `AsEnum<E>` | `E: EnumString + AsRefStr` |
| `AsArrayObject::class` | `AsArrayObject<T>` | `IndexMap<String, T>` |
| `hashed` | `AsHashed` | `String`（写入时 `Hash::make`；永不解密） |

总共 22 种转换。大多数和 Laravel 一一对应；`AsOptionalDateTime`（被 `soft_deletes` 使用）会在软删除列是 `Option<DateTime<Utc>>` 时，由这个宏自动注入。

### 加密转换的失败模式

四个 `AsEncrypted*` 转换，会把每一次加密/解密都路由经过 `Crypt` 门面（用 `APP_KEY` 作为密钥）。当解密失败时 - 密钥错误、密文被截断、字节被篡改、AEAD 标签不匹配 - 这个转换会从 `Cast::from_storage` 里暴露出一个清晰的 `FrameworkError::Internal`。没有静默回落到乱码这种事：

- 经由 `Model::find` / `Model::query()` 加载一行，会传播这个解密错误，并且（按这个宏生成的 `From<inner::Model>`）带着 `cast from_storage failed - corrupt data in database column` panic。运维人员会立刻在日志里看到这个失败；这个模型永远不会携带一个看似合理、实则错误的明文。
- `AsHashed` 这个转换是单向的；它永不解密，所以这种失败模式不适用。

这和 Laravel 的 `encrypted` 转换一致：对一个已有的加密列用错误的 `APP_KEY`，是一个硬错误，绝不会是一个悄悄的 `null`/空字符串。

### 轮换 `APP_KEY`

Suprnova 通过一个密钥*环*支持零停机的密钥轮换：当前的 `APP_KEY` 用于加密；一个可选的 `APP_KEY_PREVIOUS` 环境变量（逗号分隔，从最旧到最新），为那些在旧密钥下写入的数据，提供解密回退。加密**永远**用当前密钥 - 旧密钥只参与解密。

每一次回落到某个旧密钥的解密，都会发出一行
`tracing::warn!`，其中带着这个旧密钥的索引。这条日志负载故意排除了明文和密文；只有轮换这个事实，加上一条可操作的重新加密提示。

**轮换流程**（零停机，生产环境安全）：

1. 生成一个新密钥：`suprnova key:generate`（写到 stdout）。
2. 把旧密钥挪到 `APP_KEY_PREVIOUS`，把 `APP_KEY` 设成新的值：
   ```
   APP_KEY_PREVIOUS=<old_key>
   APP_KEY=<new_key>
   ```
3. 部署。新的写入会用这个新密钥；已有的行会继续经由旧密钥回退来解密。日志里的警告会标出仍然依赖 `APP_KEY_PREVIOUS` 的那些列。
4. 运行一次重新加密的扫尾。对每一个带加密转换的模型：
   ```rust
   for chunk in User::query().chunk(500).await? {
       for user in chunk {
           // Touch + save 会用当前密钥重写每一个转换列。
           // `Cast::to_storage` 永远会伸手去拿
           // 这个环里当前的那个条目。
           user.save().await?;
       }
   }
   ```
   这是幂等的 - 已经用上新密钥的行，只会是一次空操作。
5. 一旦日志里不再显示 `APP_KEY_PREVIOUS` 警告了（给这次批处理，以及任何软删除/存档数据留一个宽裕的窗口），就从环境里移除 `APP_KEY_PREVIOUS`，重新部署。

**多步轮换。** 如果您在完成上一次扫尾之前又轮换了一次，就追加：`APP_KEY_PREVIOUS=<oldest>,<previous>`。这个环会按顺序尝试每一个旧密钥。这个列表的上限是 8 项 - 一条现实的链条是 1 到 3 项（一次进行中的轮换，也许还有一次卡住的之前的轮换），更长的列表几乎总是一次配置模板化的事故；超出这个上限会带着一个可操作的诊断信息让启动失败，而不是静默丢掉一个运维人员可能仍然依赖的密钥。

**限制。**

- `APP_KEY_PREVIOUS` 里的一个格式错误的条目，会让启动明确地失败（和一个格式错误的 `APP_KEY` 一样）- 一个转了一半的密钥，永远不应该静默退化。
- `APP_KEY_PREVIOUS` 里超过 8 项，会让启动明确地失败 -
  参见 [`suprnova::crypto::MAX_PREVIOUS_KEYS`]。
- 列表里的空条目（例如模板化配置产生的尾随逗号），会被容忍为“这个位置没有密钥”- 不是一个错误。
- 这个传输格式和轮换前的单密钥布局相比没有变化：密文里不嵌入任何密钥标识符。这个环会按顺序试解密每一个密钥，直到有一个成功。

### 运行时转换覆盖 - `with_casts`

```rust
let users = User::query()
    .with_casts(suprnova::casts! { birthdate = AsDateTime })
    .get()
    .await?;
```

`with_casts` 会在单次查询期间，覆盖这个模型已声明的转换 - 当一个原始列是从一个 join / view / `select_raw` 里回来的，需要一种和模型默认值不同的类型强转时很有用。

### 自定义转换

自定义转换要实现 `Cast`：

```rust
use suprnova::eloquent::casts::Cast;
use suprnova::FrameworkError;

pub struct AsAesGcmJson<T>(std::marker::PhantomData<T>);

impl<T: serde::Serialize + serde::de::DeserializeOwned + Send + Sync> Cast
    for AsAesGcmJson<T>
{
    type Runtime = T;
    type Storage = String;
    fn to_storage(value: &T) -> Result<String, FrameworkError> { /* ... */ }
    fn from_storage(stored: &String) -> Result<T, FrameworkError> { /* ... */ }
}

#[model(casts = { secret = AsAesGcmJson<SecretBundle> })]
pub struct Vault { /* ... */ }
```

`Cast` trait 是和基元转换一起发布的。自定义转换可以用 `String` 存储（做 JSON 编码时），也可以用任何 SeaORM 支持的标量类型（`i64`、`f64`、`bool`、`Vec<u8>`）。

## 访问器与修改器

### 访问器

```rust
#[model(
    table = "users",
    appends = ["full_name"],
)]
pub struct User {
    pub id: i64,
    pub first_name: String,
    pub last_name: String,
    // ...
}

impl User {
    #[accessor]
    pub fn full_name(&self) -> String {
        format!("{} {}", self.first_name, self.last_name)
    }
}
```

当 `user.to_array()` 运行时（或者 `user.to_json()`，它会委托给 `to_array()`），`full_name` 这个访问器会被调用，它的返回值会被插入到 JSON 输出里。从 Rust 里调用 `user.full_name()`，就是一次普通的方法调用。

### 修改器

修改器会在存储之前运行：

```rust
#[model(
    table = "users",
    fillable = ["first_name", "last_name", "password"],
    mutators = ["password"],
)]
pub struct User { /* ... */ }

impl User {
    #[mutator]
    pub fn set_password(
        &mut self,
        value: serde_json::Value,
    ) -> Result<(), suprnova::FrameworkError> {
        let raw: String = serde_json::from_value(value).map_err(|e| {
            suprnova::FrameworkError::validation("password", format!("{e}"))
        })?;
        self.password = hash::make(&raw);
        Ok(())
    }
}
```

调用 `user.password = "secret".into()`，会直接赋值这个原始值，不会运行这个修改器。要走修改器这条路径，就调用 `user.set_password(json!("secret"))`，或者用 JSON 路径（`user.fill(attrs!{password: "secret"})`）- 因为 `"password"` 被列进了 `mutators = [...]`，它会自动路由经过这个修改器。

### 路由是怎么工作的

- **序列化（`to_array` → `Value`，`to_json` → `String`）** 会运行访问器。每一个列在 `appends = [...]` 里的字段名，都会变成对 `self.<name>()` 的一次调用；返回值会被插入到 JSON 输出里。`to_json()` 是一层薄封装：`serde_json::to_string(&self.to_array())`。
- **Fill 风格的写入（`fill`、`create`、`update`）** 会路由经过修改器。每一个列在 `mutators = [...]` 里的字段名，都会变成对 `self.set_<field>(value)` 的一次调用，而不是直接赋值。

函数级的 `#[accessor]` 和 `#[mutator]` 宏，会生成注册表条目，供这个宏的序列化/填充路径遍历。

### 格式错误的值是错误，不是默认值

一个没法解码成对应字段类型的值，会让这次写入失败，并指名这个字段：

```rust
let err = user.fill(attrs! { age: "not a number" }).unwrap_err();
// ValidationError { field: "age", message: "could not decode the
// supplied value: invalid type: string \"not a number\", expected i32" }
```

这个模型会被留在原样不动 - 一次被拒绝的 `fill` 什么都不会应用。

有两个相近的场景，行为却不同，这是故意的：

- 一个**未知列**仍然会被静默跳过，和 Laravel 的 `$model->fill()` 一致。不知道某一列存在，和拿到了一个您明明知道的列的坏值，不是一回事。
- 一列被 `fillable` / `guarded` 排除掉，会在解码*之前*就被这个批量赋值过滤器丢弃，所以一个调用者本来就不该设置的字段上的格式错误值，也会是静默的。在那里报错，会告诉一个未获授权的调用者哪些列是存在的。

数值的放宽转换不是一个类型错误：一个 JSON 整数，正常就能解码进一个 `f64` 字段。

> 在 v0.8.0 之前，一个格式错误的值会被静默替换成这个字段的 `Default`，并且这次调用会返回 `Ok` - `fill(attrs!{ age: "abc" })` 会把 `age` 设成 `0`，并报告成功。如果您依赖过这种强转，请在调用 `fill` 之前先做验证或转换。

### Hidden / visible

```rust
#[model(
    table = "users",
    hidden = ["password", "remember_token"],
)]
pub struct User { /* ... */ }
```

`hidden = [...]` 是一份拒绝列表 - 除了列出的那些列之外，每一列都会被序列化。`visible = [...]` 是允许列表形式 - 只有列出的那些列才会被序列化。两者在编译期互斥。

## 时间戳

当 `created_at` 和 `updated_at` 两列都存在时，这个宏会自动检测到它们，并启用时间戳追踪：

- 对新行，`created_at` 会在 `save()` 时被设成 `Utc::now()`。
- `updated_at` 会在每一次 `save()` 时被设成 `Utc::now()`。

这个自动检测是保守的：如果这个结构体只有两列中的一列，这个宏会报错，这样一个打错的字（`craeted_at`）就不会静默地关掉时间戳。设置 `timestamps = false` 可以完全退出。

### 禁用自动时间戳

```rust
#[model(table = "audit_logs", timestamps = false)]
pub struct AuditLog {
    pub id: i64,
    pub event: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    // 没有 updated_at 字段 - 但 timestamps = false 同时也会
    // 让这个宏的“只找到一列”错误保持静默。
}
```

### `touch()` - 推进 updated_at，不做其他改动

```rust
user.touch().await?;
```

`touch()` 会发出 `UPDATE table SET updated_at = ? WHERE pk = ?` - 是原子的，没有读改写。这个宏会在每一个带时间戳的模型上生成一个 `Touchable` 实现。

### 父级 touch

```rust
#[model(
    table = "comments",
    touches = ["post"],
    relations = {
        post: BelongsTo<Post> { fk = "post_id" },
    },
)]
pub struct Comment {
    pub id: i64,
    pub post_id: i64,
    // ...
}
```

在创建、保存、更新或删除 comment 后，它的 post 的 `updated_at` 会提升 - 一条 `UPDATE posts SET updated_at = ? WHERE id = ?`，无 `SELECT`。这正是一个挂在 `post.updated_at` 上的缓存键在只有子项变化时保持诚实所需的行为。

`touches` 中的每个名称，都必须是同一个 `relations = { ... }` 块中声明的 `BelongsTo` 关系。无法解析的名称，或者解析为其他关系种类的名称，都会是编译错误，而不是第一次保存时的意外。多态（`MorphTo`）所有者尚不可 touch。

模型使用 `timestamps = false` 的所有者会被**跳过**：不报错、不写入，子项的保存仍会返回 `Ok`。经由 `NULL` 外键到达的所有者同样如此，软删除的所有者也是如此。

touch 在触发它的写入使用的同一执行器上运行，所以在 `DB::transaction` 闭包内，它会加入那笔事务，而回滚会撤销它。

### 为什么 Suprnova 有所不同

Laravel 的 `touchOwners` 会加载每个父模型并递归，所以一次 comment 保存也会提升 post 自己的所有者，并触发每个父项的 `saved` 事件。Suprnova 通过关系注册表解析父项并直接写入该列 - 每个被 touch 的关系一条语句，不水合。因此级联只有一层深，且不会触发父项事件。这是一次保存不为每个被 touch 的关系发出一条 `SELECT` 所作的取舍。需要提升祖父项或需要事件时，请使用观察者。

对一个软删除子项的 `restore()` 不会 touch 其所有者。Laravel 的 `restore` 会经过 `save`；Suprnova 的则是直接的 `UPDATE deleted_at = NULL`。

### 格式

始终是带 UTC 的 ISO 8601。没有 `Model::$timestampsFormat` 覆盖项（按照与 Eloquent 分歧点的那张表 - 前端互操作性优先；locale 格式化的事，属于 i18n 层）。

## 观察者与生命周期事件

每一个模型，在经过 `create` / `save` / `update` / `delete` / `restore` / `replicate` / 构造器查询路径的过程中，都会走过一套固定的 16 个事件的生命周期。监听器可以挂上每一个事件，去记日志、审计、产生副作用、验证，或者取消这个正在进行的操作。

### 16 个生命周期事件

这些事件按能不能取消，分成两组：

**可取消的（5 个）** - 在数据库写入**之前**触发。一个监听器返回 `EventResult::cancel("reason")`，就会带着 `FrameworkError::bad_request(reason)` 中止这个操作。

| 事件       | 时机                                      | 负载                                                 |
|-------------|-------------------------------------------|---------------------------------------------------------|
| `Saving`    | 在 `create` 和 `save` 之前都会触发           | `Arc<Mutex<Attrs>>` + `is_creating: bool`               |
| `Creating`  | 在 `create` 之前                           | `Arc<Mutex<Attrs>>`                                     |
| `Updating`  | 在对已有行的 `save` / `update` 之前  | 更新前的模型快照 + `Arc<Mutex<Attrs>>`         |
| `Deleting`  | 在 `delete` 之前（软删除或硬删除）            | 模型 + `is_force: bool`（对软删除模型做强制删除）  |
| `Restoring` | 在对软删除模型的 `restore` 之前     | 模型                                                   |

**不可取消的（11 个）** - 在操作**之后**触发。监听器的错误会传播，但没法阻止一次已经落地的写入。

| 事件           | 时机                                              | 负载                          |
|-----------------|---------------------------------------------------|----------------------------------|
| `Retrieving`    | 每一次构造器查询各一次，在数据库调用之前        | 无                             |
| `Retrieved`     | 一次构造器查询返回的每一行各一次          | 模型                            |
| `Created`       | 在 `create` 成功之后                         | 模型                            |
| `Updated`       | 在 `save` / `update` 成功之后                | 更新前 + 更新后的快照     |
| `Saved`         | 在 `create` 和 `save` 之后都会触发                    | 模型                            |
| `Deleted`       | 在 `delete` 成功之后                         | 模型 + `is_force: bool`         |
| `Trashed`       | 在软删除之后（**不是**强制删除）              | 模型                            |
| `Restored`      | 在 `restore` 成功之后                        | 模型                            |
| `Replicating`   | 在 `replicate` / `replicate_except` 期间，返回之前（**不包括** `replicate_into` - 按源类型触发） | 源 + `Arc<Mutex<replica>>`（可变） |
| `ForceDeleting` | 在对软删除模型的 `force_delete` 之前        | 模型                            |
| `ForceDeleted`  | 在 `force_delete` 成功之后                   | 模型                            |

可取消/不可取消的这个划分，和 Laravel 的 `creating` 对 `created` 那对钩子相呼应。`Saving` 在插入和更新时都会触发 - 当两条路径上的行为完全一致时，就重写这一个，再用 `is_creating` 来区分。

`Replicating` 是唯一一个会交出一个可变引用的不可取消钩子（这个副本是 `Arc<Mutex<M>>`）。可以用它在这个克隆被返回给调用者之前，清空时间戳、重新生成 UUID、重置自增值等等。

### 观察者 vs 原始监听器

有两种方式可以挂上生命周期事件：

1. **原始监听器** - 对您想要的每一个事件调用
   `EventFacade::listen::<Created, _>(Arc::new(MyListener))`，一个事件一个实现。这是底层机制；观察者是叠在它上面的。

2. **观察者** - 把全部 16 个钩子打包进一个 trait。这个宏会看用户重写了哪些方法，只注册那些方法。对任何不算简单的钩子集合，这是推荐的路径。

```rust
use async_trait::async_trait;
use suprnova::eloquent::attrs::Attrs;
use suprnova::eloquent::events::EventResult;
use suprnova::eloquent::observers::Observer;
use suprnova::FrameworkError;

pub struct AuditObserver;

#[suprnova::observer(User)]   // <- 必须写在 #[async_trait] 前面
#[async_trait]
impl Observer<User> for AuditObserver {
    async fn creating(&self, attrs: &mut Attrs) -> EventResult {
        if attrs.get("email").is_none() {
            return EventResult::cancel("email is required");
        }
        EventResult::ok()
    }

    async fn created(&self, user: &User) -> Result<(), FrameworkError> {
        tracing::info!(user.id = user.id, "user created");
        Ok(())
    }
}
```

每一个 trait 方法都有一个默认的空操作，所以这个 impl 块里只包含您关心的那些事件。这个宏是靠名字匹配这个封闭的 16 方法集合来识别重写的；您没有重写的方法，不会注册任何监听器。

### 必须遵守的属性顺序

`#[suprnova::observer(M)]` 必须写在 `#[async_trait]` **上面**：

```rust
#[suprnova::observer(User)]   // 外层 - 先运行，看到的是原始的 async fn
#[async_trait]                // 内层 - 会重写 async fn 的签名
impl Observer<User> for AuditObserver { /* ... */ }
```

属性宏是由外向内展开的。`async_trait` 会把每一个 `async fn` 重写成一个脱糖之后的 `Pin<Box<dyn Future>>` poll 函数形态；如果 `#[async_trait]` 先运行，这个观察者宏针对 16 个 trait 方法名做的名字匹配就什么都找不到，会静默地生成零个监听器。

### 四条注册路径

| 路径                                         | 何时使用                                         |
|----------------------------------------------|-----------------------------------------------------|
| `#[suprnova::observer(M)]`（inventory）       | 编译期就已知的静态观察者。会在启动时自动安装。 |
| `#[model(observers = [Foo, Bar])]`           | 文档说明 + 编译期验证列出的类型都能解析。它本身**不会**注册。 |
| `Model::observe(MyObs).await`                | 运行时注册。手动驱动；当注册依赖配置时很有用。 |
| `EventFacade::listen::<events::Created, _>(...)` | 最底层 - 一次一个事件。当一个观察者显得太重时使用。 |

`#[model]` 上的 `observers = [...]` 属性，是一个文档标记。它会编译成一个 `const _: fn() = || { let _ = ::std::any::type_name::<T>; ... };` 块，证明列出的每一个类型都能解析成一个真实的 Rust 项；打错的字会在模型声明处暴露出来。实际的安装，是经由这条 inventory 通路 - `Foo` 上的 `#[observer(M)]` 属性，才是让 `Foo` 被登记进自动安装的那个东西。

### 应用启动

在启动时调用一次 `bootstrap_observers()`，把这个 inventory 排空，并安装每一个通过 `#[observer(M)]` 注册的观察者：

```rust
suprnova::eloquent::observers::bootstrap_observers().await?;
```

对这条 inventory 通路来说，这次排空是幂等的 - 每一个观察者的安装闭包，都被一个逐类型的 `AtomicBool`（T2b 那个宏生成的）挡着，所以调用两次 `bootstrap_observers()` 不会重复注册。

运行时的 `Model::observe(MyObs)` 这个薄封装**没有**被这样挡着。调用两次会注册两套监听器，这和 Laravel 手动的 `Model::observe(MyObs::class)` 语义一致。如果一个手动驱动的观察者同时也带着 `#[observer]`，这个 inventory 适配器会在那些手动安装的之外额外触发。

### 从一个观察者内部取消

这五个可取消的钩子都返回 `EventResult`。要中止这个操作，就返回 `EventResult::cancel("reason")`：

```rust
#[suprnova::observer(Subscription)]
#[async_trait]
impl Observer<Subscription> for PolicyObserver {
    async fn creating(&self, attrs: &mut Attrs) -> EventResult {
        if let Some(plan) = attrs.get("plan") {
            if plan == "blocked" {
                return EventResult::cancel("plan is blocked");
            }
        }
        EventResult::ok()
    }
}
```

这个取消原因，会从 `Subscription::create` 里以 `FrameworkError::bad_request(reason)` 的形式暴露出来。这一行永远不会落进数据库 - 取消是一次真正的中止，不是一次“事后删除”。

同一个模型上，可以有多个观察者注册可取消的钩子；其中任何一个返回 `Cancel`，都会停止这个操作。顺序是这个 inventory 的登记顺序（实践中就是链接顺序）。

### 一个模型上的多个观察者

多个 `Observer<M>` 实现，会针对同一个事件全部触发 - EventFacade 的分发是扇出给每一个已注册的监听器，而不是只挑一个：

```rust
#[suprnova::observer(Comment)]
#[async_trait]
impl Observer<Comment> for AuditObserver { /* ... */ }

#[suprnova::observer(Comment)]
#[async_trait]
impl Observer<Comment> for NotifyObserver { /* ... */ }

// Comment::create(...) 会触发 AuditObserver::created **和** NotifyObserver::created。
```

这和 Laravel 的扇出语义一致，也是“按关注点拆分钩子”这个模式背后那个承重的特性：一个 `AuditObserver` 只知道审计，一个 `NotifyObserver` 只知道通知，而这个模型声明不关心挂了多少个观察者。

### 手动的 `Model::observe()`

每一个 `#[suprnova::model]` 结构体，都会得到一个逐模型的 `observe<O>()` 薄封装。在启动时调用它，就能做动态注册：

```rust
#[derive(Clone)]
struct MyObs;

#[async_trait]
impl Observer<User> for MyObs { /* ... */ }

// 在运行时：
User::observe(MyObs).await;
```

这个薄封装的 `O: Clone + 'static` 约束，正是让框架能把一个全新的观察者克隆，分发给 16 个内部适配器监听器当中每一个的原因。每一次调用都会安装全部 16 个监听器适配器 - trait 的默认实现，让没被重写的方法成为廉价的空操作。

### 约束

- **宏版本要求这个 impl 块使用匹配这个 trait 16 个钩子的朴素方法名。**
  改了名字的方法、被 `#[allow]` 压制的默认实现，以及被 `#[cfg]` 挡住的方法体，都落在这个名字匹配之外，不会注册监听器。

- 在 v1 里，**这个宏检视的观察者结构体必须是零大小的**（没有字段）。这个宏是在每一个适配器内部，经由 `let obs =
  MyObserver;` 这样构造这个观察者的。带状态的观察者（携带着
  `Arc<Inner>`）需要走运行时的 `Model::observe()` 路径，它会按值接收这个观察者，把它克隆进每一个适配器。

- **测试隔离：每个场景用独一无二的模型类型。** 这个进程全局的
  EventDispatcher 意味着，为 `User` 安装的监听器，对同一个二进制里的每一个测试都是可见的。逐测试独一无二的模型类型（`T2Comment`、
  `T2Subscription`、……），能让跨测试的渗漏不影响这些计数断言。
  `eloquent_observers.rs` 这些集成测试练习的就是这个模式。

## 可修剪

Laravel 提供了一个 `Prunable` trait，让一个模型能声明一个要按计划删除的行范围。Suprnova 用两个 trait 和一个控制台命令，对应了这个设计。

### 声明一个修剪器

```rust
use async_trait::async_trait;
use chrono::{Duration, Utc};
use suprnova::eloquent::Prunable;

#[suprnova::prunable]
#[async_trait]
impl Prunable for ExpiredSession {
    fn prunable() -> suprnova::Builder<Self> {
        Self::query().filter_op(
            "expires_at",
            "<",
            (Utc::now() - Duration::days(30)).to_rfc3339(),
        )
    }
}
```

### `MassPrunable` - 批量删除变体

对高吞吐量的表（审计日志、请求日志、过期的缓存条目），`MassPrunable` 会跳过逐行事件，运行单一一条 `DELETE WHERE …` 语句：

```rust
use suprnova::eloquent::MassPrunable;

#[suprnova::prunable]
#[async_trait]
impl MassPrunable for AuditLog {
    fn prunable() -> suprnova::Builder<Self> {
        Self::query().filter_op(
            "created_at",
            "<",
            (Utc::now() - Duration::days(365)).to_rfc3339(),
        )
    }
}
```

### 触发修剪

通过逐项目的控制台运行（`app/cmd/main.rs` 会为它调用 `suprnova::console::dispatch_argv`，在 `db:seed` 和其他内置命令之后）：

```bash
suprnova model:prune                          # 修剪每一个已注册的类型
suprnova model:prune --model=ExpiredSession   # 只过滤出一个模型
suprnova model:prune --pretend                # 空跑；把会删除的内容记进日志
```

以编程方式调用时，这些运行器位于 `suprnova::eloquent::{prune_all, prune_all_dry, prune_one}`。

### 修剪钩子

`Prunable::pruning(&self)` 会在每一次行删除之前触发，这样用户就能运行一些副作用（清理关联文件、扇出事件等等）。默认实现是空的。`MassPrunable` 按定义会跳过这个钩子 - 批量删除不会逐行枚举。

### 级联行为

**修剪不会自动级联到相关行。** 一个针对 `User` 的 `Prunable` 或 `MassPrunable` 实现，删除的是 user 行；它们的 `posts`、`role_user` 中间表条目、多态的 `comments` 等等，会被**留成孤儿**，外键列指向那个现在已经被删除的用户。

这和 Laravel 的契约一致：清理关系是用户自己的事。有两种干净的处理方式：

1. **数据库层面的外键级联** - 在写迁移的时候，在外键约束里声明
   `ON DELETE CASCADE`（或者 `ON DELETE SET NULL`）。数据库引擎会免费处理这个级联，不需要任何逐行的 Rust 代码。

2. **逐行钩子** - 实现 `Prunable::pruning(&self)`，在这个父级行被丢弃之前删除子级。这个钩子会在和父级删除同一个逻辑操作内部触发，所以顺序是有保证的、一致的：

   ```rust
   #[async_trait]
   impl Prunable for User {
       fn prunable() -> Builder<Self> {
           Self::query().filter_op("deleted_at", "<", thirty_days_ago())
       }

       async fn pruning(&self) -> Result<(), FrameworkError> {
           // 删除 posts。
           Post::query().filter("user_id", self.id).get().await?
               .into_iter()
               .map(|p| p.delete());
           // 解除 role 中间表关联。
           self.roles().sync(Vec::<i64>::new()).await?;
           Ok(())
       }
   }
   ```

`MassPrunable` 是基于集合的 - `pruning()` 不会触发。任何需要级联的时候，就用朴素的 `Prunable`。当您选用 `MassPrunable` 时，框架不会静默地发出逐行 DELETE；这个权衡是被醒目地写进文档的。

### 注册表机制

修剪器的注册，用的是和观察者、命令、监督程序一样的 inventory 模式。`impl Prunable for T { ... }` 块上的 `#[suprnova::prunable]` 属性，会在编译期经由 `inventory::submit!` 自动注册。没有中心化的配置文件；添加一个新的可修剪类型，只需要一个属性。

## 多连接路由

生产应用经常需要不止一个数据库连接 - 典型的场景是给分析用的一个读副本，加上给写入用的主库，但这套表面能泛化到任何具名连接（报表数据库、存档数据库、逐租户的分片）。

### 注册一个连接

在启动时，为您的应用要对话的每一个非默认连接，调用 `DB::register_named(name, config)`：

```rust
DB::register_named(
    "reporting",
    DatabaseConfig {
        url: env::var("REPORTING_DATABASE_URL")?,
        max_connections: Some(20),
        ..Default::default()
    },
).await?;
```

有两个名字是保留的：`__primary__` 会让这个注册表短路到 `DB::connection()`；`__read_replica__` 会让这个连接选择加入自动的读写分离路由 - 见下文。

### 逐查询选择加入：`Model::on(name)`

`Model::on("reporting")` 会返回一个预先设置好、会路由经过这个具名连接的 `Builder<M>`：

```rust
let totals = Order::on("reporting")
    .order_by_desc("total")
    .limit(100)
    .get()
    .await?;
```

`on(...)` 是请求作用域的 - 它只影响这条链上的构造器。下一次朴素的 `Order::query()` 调用，会经由默认值解析。

### 逐模型默认值：`#[model(connection = "...")]`

当一个模型总是活在一个连接上时，就在这个属性上声明这个默认值：

```rust
#[model(table = "events", connection = "events_db")]
pub struct Event { /* ... */ }
```

每一次 `Event::query()` / `Event::create()` / `Event::find()` 调用，都会路由经过 `events_db`，不需要逐查询的 `.on(...)` 覆盖。一个构造器上显式的 `.on(...)`，仍然会赢。

### 读写分离

在保留名字 `__read_replica__` 下注册一个连接，会让每一个模型都选择加入自动路由：读方法（`first` / `get` / `find` / `count` / `paginate` / `chunk` / 那些闭包驱动的遍历器）会流经这个副本；写操作（`save` / `create` / `update` / `delete` / `force_delete` / `replicate` / `attach` / `detach` / `sync` / `increment` / `decrement`）会流经主库。

`Model::on_write_connection()` 会让单个构造器**退出**这个副本 - 当“读到自己刚写的东西”这种一致性很重要时有用（例如在一次 `save` 之后立刻读取，在复制赶上之前）。

### 路由优先级

这条分发链，会把每一个操作都经过 `ExecutorChoice::resolve_read` 或 `resolve_write`。顺序是：

1. **活跃的事务绝对优先。** 在 `DB::transaction` 内部，每一次读**和**
   每一次写，都会用这个 tx 连接。在一个事务内部，`on(name)` 会被
   **忽略** - 这个 tx 绑定的是一个具体的物理连接。SeaORM 没法在一个连接上开始一个事务，却在另一个连接上运行语句。
2. **逐构造器的 `on(name)`。** 经由 `Model::on(name)` /
   `Builder::on(name)` 设置。会赢过模型默认值和读写分离。
3. **`Model::on_write_connection()`。** 强制使用主库，即便这个操作原本会路由到这个副本上。
4. **逐模型的 `#[model(connection = "...")]` 默认值。** 对这个模型自己的查询来说，会赢过读写分离。
5. **读写分离。** 当 `__read_replica__` 被注册时，读方法会路由到那里；写操作会路由到主库。
6. **默认值。** `DB::connection()` - 主库，也就是 `DB::init()`
   设置好的那一个。

### 注意事项

- 活跃的事务会**忽略** `on(name)`（见上面第 1 条）。如果您需要在事务中途对一个不同的连接做写入，那做不到 - 这个 tx 绑定的是一个连接。
- 保留名字 `__primary__` 和 `__read_replica__`，不能用作用户的连接名。`DB::register_named` 在发生冲突时会返回一个错误。
- 副本延迟是**您自己**的问题。当这个副本数据陈旧时，Suprnova 不会在读取时重试，也不会回落到主库；如果您在保存之后需要“读到自己刚写的东西”，就显式使用 `Model::on_write_connection()`。

## 复制

`Model::replicate()` 会返回这个模型的一个未保存的副本，主键会被重置成它的默认值。适合“复制这条记录”这种用户想从一个已有行开始的 UX 场景。

```rust
let template: User = User::find_or_fail(42).await?;
let mut copy = template.replicate().await?;  // id 被重置成默认值
copy.email = "fresh@example.com".into();
copy.save().await?;  // 是 INSERT，不是 UPDATE
```

在 Suprnova 里，`replicate` 是**异步**的（这是和 Laravel 的一个分歧点），因为它会触发 `Replicating` 事件 - `Saving` / `Created` 等等的监听器，可以在这个副本被返回之前修改它。监听器修改的契约，请参见[Replicating 事件](#replicating-事件)。

### `replicate_except`

从这个副本里丢弃指名的字段：

```rust
let copy = order.replicate_except(["payment_token", "stripe_id"]).await?;
```

列出的字段会回落到这个模型的 `Default` 实现 - `String` 会变成 `""`，`Option` 会变成 `None`，等等。用于那些复制出来的行不应该继续携带的敏感列。

### 跨类型的 `replicate_into::<T>`

这是 Suprnova 的分歧点 - Laravel 做不到，因为 PHP 没有类型。`replicate_into::<T>()` 经由 `serde_json`，桥接到一个同级类型：

```rust
let order: Order = Order::find_or_fail(42).await?;
let invoice: Invoice = order.replicate_into::<Invoice>().await?;
invoice.save().await?;
```

名字匹配、类型又和 serde 兼容的字段会被带过去；两边任何一边不匹配的字段，都会被静默丢弃。`T` 必须实现 `Default`，这样没被填充的字段才有一个值。跨类型复制**不会**触发 `Replicating`（这个事件携带的是一个 `&mut Self` - 没法经由它指向 `T`）。如果您需要事件驱动的修改，就先做同类型复制，再从结果里物化出 `T`。

## 调试 - dump 与 dd

每一个 `Builder<M>` 上都有两个交互式调试帮手：

```rust
// 经由 tracing::info! 记录 SQL + 绑定参数，返回 self。
let users = User::query()
    .filter("active", true)
    .dump()                       // → 一行日志，构造器继续
    .order_by_desc("created_at")
    .get()
    .await?;

// 用 tracing::error! 记录，然后带着消息里的 SQL panic。
User::query().filter("id", 1).dd();  // - !
```

`dump` 是可链式调用的；`dd` 返回 `!`（永不返回 - panic 就是这个契约）。两者都精确地对应着 Laravel 的 `Builder::dump()` / `Builder::dd()`。

当没有绑定一个存活的数据库连接时，这两个帮手都会回落到 SQLite 方言（和 `to_sql_with_bindings` 的回落一致），这样它们在 REPL 里，或者在一个没有 `TestDatabase` 的测试里，仍然有用。

这个 panic 消息用的是字面前缀 `eloquent dd:`，这样测试就能针对它做断言：

```rust
#[test]
#[should_panic(expected = "eloquent dd")]
fn dd_panics_with_sql_in_message() {
    User::query().filter("id", 1).dd();
}
```

**永远不要把 `dd()` 提交进一条生产代码路径。** 它是一个交互式调试帮手；退出时的那个 panic 就是它存在的全部意义。`dump()` 更安全（只是记日志），但在热路径里刷屏式地用它，会把您的日志灌满 - 推送之前请把它删掉。

如果您想要这个 SQL，但不想要这些副作用，就伸手去用那些不记日志的帮手：

- `Builder::to_sql()` - 把渲染好的 SQL 以 `String` 的形式返回。
- `Builder::to_sql_with_bindings()` - 返回 `(String, Vec<SeaValue>)`。
- `Builder::to_sql_for(backend)` - 针对一个显式的方言渲染（跨后端调试）。

## 测试模型

测试是经由 `TestDatabase` 实例化一个真实的数据库的，它会把这个连接注册进逐测试的容器里，这样被测系统（SUT）内部任何调用 `DB::connection()` 的地方，都会解析到这个测试数据库。

### 两个入口点

- **`TestDatabase::fresh::<MyMigrator>().await`** - 运行生产迁移器会运行的每一次迁移。用于那些您想让测试架构精确匹配 `suprnova migrate` 产出结果的应用级 dogfood 测试。
- **`TestDatabase::sqlite_memory().await`** - 打开一个内存中的 SQLite 数据库，**不**应用任何迁移。用于那些您想通过逐测试的 `db.execute_unprepared("CREATE TABLE …")` 精确控制列形态的框架级单元测试。

### 应用级 dogfood 模式

```rust
use app::migrations::Migrator;
use app::models::users::User;
use suprnova::testing::TestDatabase;
use suprnova::{attrs, Model};

#[tokio::test]
async fn user_lifecycle() {
    let _db = TestDatabase::fresh::<Migrator>().await.unwrap();

    let alice = User::create(attrs! {
        name: "Alice",
        email: "alice@example.com",
        password: "hashed",
    }).await.unwrap();

    assert!(alice.id > 0);

    alice.delete().await.unwrap();
    assert!(User::find(alice.id).await.unwrap().is_none(),
        "default scope hides soft-deleted rows");
}
```

这个 `_db` 绑定，在整个测试期间都持有着这个 `TestDatabase` - 丢弃它会把这个容器拆掉，释放这个内存中的 SQLite 连接。不要把它遮蔽成 `_`，否则这个连接会在 SUT 运行之前就消失。

### 框架级形态模式

```rust
use suprnova::testing::TestDatabase;
use suprnova::{attrs, model, Model};

#[model(table = "t_users", timestamps = false)]
pub struct TUser { pub id: i64, pub name: String }

#[tokio::test]
async fn shape_test() {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    db.execute_unprepared(
        "CREATE TABLE t_users (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT)"
    ).await.unwrap();

    let u = TUser::create(attrs! { name: "Alice" }).await.unwrap();
    assert_eq!(u.name, "Alice");
}
```

### 关键模式

- 用生产架构做应用级测试时，用 `TestDatabase::fresh::<MyMigrator>()`。单元级的形态测试，用 `TestDatabase::sqlite_memory()`。
- 对测试要改动的任何单例，请用 `TestContainer::bind`（**不是** `App::bind`）- 全局注册表的覆盖，在并行运行时会产生竞态。`TestDatabase` 的构造函数会替您处理这个数据库绑定。
- 把模型声明留在模块作用域里，不要放进测试函数内部。这个宏会生成一个内部 `mod`，它的 `use super::*;` 只能看到这个文件顶层的那些导入 - 在一个测试函数内部声明模型，会弄坏 SeaORM 的类型解析。

## 落到 SeaORM

三个脱围机制，让 SeaORM 在 Eloquent 层内部仍然可以触达：

1. **内部模块** - `user::Entity`、`user::Column`、`user::ActiveModel`、
   `user::Model`。这个宏会为每一个模型生成这些；它们是您可以直接使用的 SeaORM 类型。完整的布局，以及该在什么时候伸手去用，请参见[模型模块布局](#模型模块布局)。
2. **`From` 转换** - `From<user::Model> for User` 和
   `From<User> for user::Model`，会在 SeaORM 形态的行（存储类型的列）和 Eloquent 形态的行（运行时类型的列）之间搭桥。当您想发出一次 SeaORM 查询，再把结果转换成 Eloquent 形态时（或者反过来）很有用。
3. **由 Suprnova 起别名的 SeaORM 类型** - 每一个消费者会接触到的
   SeaORM 类型，都在 `suprnova::*` 下面被重新导出了。您在应用代码里应该不需要 `use sea_orm::*`。

```rust
use suprnova::sea_orm::{ColumnTrait, EntityTrait};

// 在查询中途落到 SeaORM - Eloquent 没有对应这个的方法，
// 但 SeaORM 有：
let db = suprnova::DB::connection()?;
let users = user::Entity::find()
    .filter(user::Column::Email.like("%@example.com"))
    .all(db.inner())
    .await?;

// 转换成 Eloquent 形态：
let eloquent: Vec<User> = users.into_iter().map(User::from).collect();
```

三个脱围机制，加上这个 From 桥接，意味着 Eloquent 层永远不会挡着您去触达底层的 ORM。

## 从 `database::Model` 迁移

较老的代码，可能在一个手写的 SeaORM entity 上带着 `impl suprnova::database::Model for Entity {}`。这个 trait 被改名成了 `EntityExt`，为新的 `Model` trait 腾出位置 - 新的这个 trait 是坐落在面向用户的结构体上，不是坐落在 SeaORM entity 上。

推荐的迁移路径，是把这个类型切换成 `#[suprnova::model]`，这样您就能得到完整的 Eloquent 表面，外加那个改了名字的 `EntityExt` trait 作为额外收获。对少数您想保留旧的 SeaORM Entity 扩展形态的场景，`EntityExt` / `EntityExtMut` 这两个 trait 名字，在 `suprnova::database::*` 下面仍然可用。它们的行为和旧的 `database::Model` 完全一样。

## `DB` 门面 - 无模型查询

有些表不适合放在一个 `#[suprnova::model]` 结构体上：短生命周期的审计日志、临时的报表 join、仪表盘聚合。对这些场景，就伸手去用 `DB` 门面。它下面有两个表面：

### `DB::table(name)` - 可链式调用的查询构造器

`DbTableBuilder` 镜照了 `Builder<M>` 的 where / order / limit 形态，但返回的行是 `DynamicRow`（一个套在 `serde_json::Map<String, Value>` 上的类型化访问器 newtype）：

```rust
use suprnova::DB;

let rows = DB::table("audit_log")
    .filter("actor_id", 42)
    .filter_op("created_at", ">=", "2026-01-01")
    .order_by_desc("id")
    .limit(50)
    .get()
    .await?;

for row in rows.iter() {
    let event: String = row.get_string("event")?;
    let actor_id: i64 = row.get_int("actor_id")?;
    println!("{actor_id}: {event}");
}
```

完整的表面：

| 方法 | 返回类型 | 用途 |
|--------|---------|---------|
| `.select(["id", "event"])` | `DbTableBuilder` | 限制列（默认 `*`） |
| `.filter(col, val)` | `DbTableBuilder` | `WHERE col = ?` |
| `.filter_op(col, op, val)` | `DbTableBuilder` | `WHERE col <op> ?` |
| `.order_by_asc(col) / _desc(col)` | `DbTableBuilder` | 排序 |
| `.limit(n) / .offset(n)` | `DbTableBuilder` | 窗口 |
| `.get()` | `Collection<DynamicRow>` | 所有匹配的行 |
| `.first()` | `Option<DynamicRow>` | 第一行，或者 `None` |
| `.count()` | `u64` | `SELECT COUNT(*) ...` |
| `.insert(attrs)` | `i64` | 新行的 `id` |
| `.update(attrs)` | `u64` | 受影响的行数 |
| `.delete()` | `u64` | 受影响的行数 |

**标识符的信任边界。** 表名、列名、SQL 运算符，以及 ORDER BY 的方向，都会被原样内插进这条 SQL 字符串里 - 它们**不会**被绑定为参数。这些参数只应该传入可信的、编译期的字面量。值（`filter` / `filter_op` 的右手边）**会**被绑定，从请求数据里传过去是安全的。

**`update` / `delete` 上的空 WHERE，会作用于每一行。** `DB::table("audit_log").delete().await?` 会按设计截断这张表 - 如果您不是这个意思，就加一个 `filter`。

**插入操作的后端分歧。** `RETURNING id` 用在 Postgres 和 SQLite 上；MySQL 会运行这条 INSERT，再发出 `SELECT LAST_INSERT_ID() as id` 来取回这个自增值。

### `DynamicRow` - JSON 映射上的类型化访问器

`DynamicRow` 包装了一个 `serde_json::Map<String, Value>`，并暴露出类型化的取值方法。每一个都返回 `Result<T, FrameworkError>`，并在键缺失或类型不匹配时附上一条清晰的错误消息：

```rust
let event: String     = row.get_string("event")?;
let actor_id: i64     = row.get_int("actor_id")?;
let active: bool      = row.get_bool("active")?;
let prefs: Prefs      = row.get_as("prefs")?;  // 任何 DeserializeOwned
let raw: serde_json::Value = row.get_value("meta")?;
```

对于可空的列，请用 `get_optional_*`。它们会区分“列缺失”（错误 - 架构不匹配）和“列存在，值为 null”（`Ok(None)`）：

```rust
let score: Option<i64>      = row.get_optional_int("score")?;
let title: Option<String>   = row.get_optional_string("title")?;
```

`DynamicRow` 会解引用到 `Map<String, Value>`，所以迭代和键存在性检查都能直接使用：

```rust
for (key, value) in row.iter() {
    println!("{key} = {value}");
}
```

### 原始 SQL 脱围机制

当这个构造器不够用时 - 窗口函数、递归 CTE、后端专属的 DDL - 就落到一个原始字符串。占位符要匹配活跃的后端（Postgres 用 `$1, $2, ...`，MySQL + SQLite 用 `?`）：

```rust
// 原始 SELECT，物化成 DynamicRow。
let rows = DB::select(
    "SELECT u.name, COUNT(p.id) as post_count
     FROM users u LEFT JOIN posts p ON p.user_id = u.id
     GROUP BY u.id
     HAVING post_count > ?",
    vec![5i64.into()],
).await?;

// 原始 UPDATE / DELETE - 返回受影响的行数。
let updated = DB::update(
    "UPDATE users SET verified_at = NOW() WHERE id = ANY($1)",
    vec![ids.into()],
).await?;

let deleted = DB::delete(
    "DELETE FROM stale_sessions WHERE expires_at < ?",
    vec![now.into()],
).await?;

// 原始 DDL 或者不带绑定参数的语句。
DB::statement("CREATE INDEX CONCURRENTLY idx_users_email ON users(email)")
    .await?;

// 通用的生效语句 - 用于 INSERT ... ON CONFLICT 等等。
let rows = DB::affecting_statement(
    "INSERT INTO counters (k, n) VALUES ($1, 1) ON CONFLICT (k) DO UPDATE SET n = counters.n + 1",
    vec!["page_views".into()],
).await?;
```

请节制地使用这些脱围机制 - 类型化的构造器能在编译期抓住更多错误，在业务逻辑里读起来也更干净。但当您需要它们时，它们就在这里。

**聚合列的一个坑。** 像 `SELECT COUNT(*) AS n FROM t` 这样无类型的聚合，经由构造器的 `.count()` 帮助函数能正常工作，但在 SQLite 上，可能会从原始的 `DB::select` 行里被静默丢弃 - 底层的 `JsonValue::from_query_result` 会遍历 sqlx 逐列的类型信息，而一个裸的聚合不带任何类型信息。如果您需要带聚合的原始 select 路径，就给这个表达式一个类型化的上下文：可以用一个 `CAST(... AS BIGINT)` 包装，或者用一个底层用了 `query_one` + `try_get` 的类型化 `DB::table(...).count()` / `.max(...)` 帮助函数来读这一列。

## 关系存在性 + 轻量捷径

Suprnova 提供了一套与 Laravel 的关系存在性查询家族相对应的接口。这里的每一个方法，都把 Laravel 形态的名字和一个符合 Rust 惯用法的别名配成一对（这是 Suprnova 一贯的双 API 约定）。

### 关系存在性过滤（`has` / `where_has` / `where_belongs_to`）

这个关联的 `EXISTS (...)` 家族，会按相关行的存在（或不存在，或数量）来约束父查询，而不必把这个关系联结进外层的 SELECT。

```rust
use suprnova::Model;

// 至少有一篇 post 的用户。
let users = User::query().has("posts").get().await?;

// 一篇 post 都没有的用户。
let empty = User::query().doesnt_have("posts").get().await?;

// 有 >= 3 篇 post 的用户（Laravel 的 `has("posts", ">=", 3)`）。
let prolific = User::query().has_count("posts", ">=", 3).get().await?;

// 通过闭包施加的内层约束 - 限制 EXISTS 子查询的主体。
let recent = User::query()
    .where_has::<Post, _>("posts", |q| q.filter_op("created_at", ">=", "2026-01-01"))
    .get()
    .await?;

// 单列捷径 - 等价于带一个极小闭包的 `where_has`。
let with_pub = User::query()
    .where_relation("posts", "published", true)
    .get()
    .await?;

// belongs-to 的直接联结（没有 EXISTS - 外键就在这张表上）。
let posts = Post::query().where_belongs_to("author", author.id).get().await?;
```

所有变体都能和 `or_*` 以及 `*_doesnt_have` 这些同伴组合起来：

- `has` / `or_has` / `has_count` / `doesnt_have` / `or_doesnt_have`
- `where_has` / `or_where_has` / `where_doesnt_have` / `or_where_doesnt_have`
- `where_relation` / `where_relation_op` / `or_where_relation`
- `where_belongs_to`

这个引擎会从宏生成的 `RelationEntry` inventory 里读取关系元数据：联结列、中间表、morph 判别符全都会自动流转过去。它会渲染出三种子查询形态：

- **Has** - `EXISTS (SELECT 1 FROM child WHERE child.fk = parent.pk)`
- **Pivot** - `EXISTS (SELECT 1 FROM pivot INNER JOIN target ON ... WHERE pivot.parent_fk = parent.pk)`
- **Morph** - has / 中间表的形态，再加上 `AND target.<morph>_type = '<value>'`

不认识的关系名字会渲染成安全失败的形式（`EXISTS (SELECT 1 WHERE 1 = 0)`），它求值为 `FALSE`，返回零行。一个拼写错误永远不会泄漏成一次全表扫描。

### `MorphTo` 的分歧

Laravel 的 `MorphTo` 反向查询（`whereMorphedTo`、`whereHasMorph`）会走过多张目标表，因为 morph 子表带着一个 `*_type` 判别符，用来从 N 个可能的父级里挑一个。Suprnova 的 `MorphTo` 在宏展开时会下降成一个按族划分的枚举 - 目标类型静态地是一个 `<Family>Morph { Variant1(...), ... }`，而不是单独一张 SQL 表。存在性引擎没法为这种情况渲染一个固定的 `EXISTS (SELECT 1 FROM <table>)`，因为根本不存在单独的一张表。

推荐的迁移方式：改在 morph 子表这一层做存在性检查。Laravel 这么写：

```php
Comment::whereHasMorph('commentable', [Post::class], fn ($q) => $q->where('published', true))
```

Suprnova 则这么写：

```rust
Comment::query()
    .filter("commentable_type", "post")
    .where_has::<Post, _>("commentable_post", |q| q.filter("published", true))
    .get()
    .await?;
```

这种类型更窄的写法，能在内层构造器上给出完整的 IDE 补全，而类型松散的 `whereHasMorph` 做不到。

### 轻量的构造器捷径

```rust
// 主键过滤。
User::query().where_key(7).first().await?;        // filter("id", 7) 的语法糖
User::query().where_key_not(7).get().await?;      // filter_op("id", "!=", 7) 的语法糖
User::query().filter("name", n).or_where_key(7).get().await?;      // ... OR id = 7
User::query().filter("name", n).or_where_key_not(7).get().await?;  // ... OR id != 7
// 符合 Rust 惯用法的别名：filter_key / filter_key_not /
// or_filter_key / or_filter_key_not。

// 按 created_at 排序。
Post::query().latest().get().await?;              // ORDER BY created_at DESC
Post::query().oldest().get().await?;              // ORDER BY created_at ASC
Post::query().latest_by("published_at").get().await?;  // 具名的列

// 恰好匹配一行。
let one = User::query().filter("email", e).sole().await?;          // 0 行或 >1 行时报错
let val: i64 = User::query().filter("id", 1).sole_value("views").await?;
let v: i64 = User::query().filter("name", "x").value_or_fail("views").await?;

// 预加载的退出选项。
User::query().with(["posts","tags"]).without(["tags"]).get().await?;
User::query().with_only(["posts"]).get().await?;   // 先把这个计划清空

// 完全限定的列（用于联结）。
Builder::<User>::qualify_column("name");           // -> "users.name"
Builder::<User>::qualify_columns(["name", "id"]);  // -> ["users.name", "users.id"]
```

### 批量变更 - `update_all` / `delete_all` / `upsert` / `*_each`

这些方法会用单条语句直接打到数据库，**不会**触发逐行的模型事件。当缩小作用域就够用、而您又不需要生命周期钩子时，就用它们；需要逐行钩子的话，请用 `.get()` 迭代，然后逐行调用 `.update()` / `.delete()`。`delete_all` 总是以模型静态的 `M::TABLE` 为目标；运行时的表名不会被当作可执行 SQL 接受。显式的空值属性会作为 SQL `NULL` 发出，所以在 PostgreSQL 上，可为空的 bigint、integer、boolean、timestamp 以及其他非文本列都会保留它们的数据库类型。每一个非空属性仍然是参数绑定的。upsert 的各行必须有相同的列集合；缺少或多出的键会被拒绝，而不会被解释成空值。

```rust
// 批量 UPDATE。
let n = User::query()
    .filter("active", false)
    .update_all(attrs! { archived_at: Utc::now() })
    .await?;

// 批量 DELETE。
let n = Session::query()
    .filter_op("expires_at", "<", cutoff)
    .delete_all()
    .await?;

// INSERT ... ON CONFLICT (Postgres / SQLite) / ON DUPLICATE KEY UPDATE (MySQL).
let n = Counter::query()
    .upsert(
        vec![attrs! { key: "page_views", n: 1 }, attrs! { key: "signups", n: 1 }],
        vec!["key"],                  // 冲突目标
        Some(vec!["n"]),              // 要更新的列；None = 每一个非唯一列
    )
    .await?;

// 针对一个作用域的原子递增/递减。
User::query()
    .filter("id", 7)
    .increment_each(vec![("views", 1), ("likes", 1)])
    .await?;

User::query()
    .filter("id", 7)
    .decrement_each(vec![("balance", 100)])
    .await?;
```

### `Model` 的静态辅助函数

```rust
// 按一组主键批量销毁。逐行的事件会触发（每一行都会走 .delete()，
// 所以软删除的墓碑语义 + Deleting/Deleted 的分发
// 都会被遵守）。
let removed: u64 = User::destroy(vec![1i64, 2, 3]).await?;
let removed: u64 = User::force_destroy(vec![1i64, 2, 3]).await?;

// 按主键做身份比较。
assert!(alice.is(&also_alice));
assert!(alice.is_not(&bob));
```

### `*Quietly` 变体 - 压制生命周期事件

这是 `seed::without_events` 之上的语法糖。在这个作用域内部，五个静态生命周期事件（`Saving`/`Creating`/`Updating`/`Deleting`/`Restoring`）以及那些不可取消的事后事件，都会短路。

```rust
user.save_quietly().await?;            // 不会有 Saving / Updated / Saved
user.update_quietly(attrs).await?;
user.delete_quietly().await?;
user.force_delete_quietly().await?;
```

### `*_or_fail` 变体

在找不到的情况下给出明确的错误。在那些“行缺失就是缺陷”的不变量检查代码路径里很有用。

```rust
let user = user.update_or_fail(attrs).await?;   // 这一行若在中途被删除就是 not_found
user.delete_or_fail().await?;
```

### 过滤式序列化 - `to_array_except` / `to_array_only`

这是 Suprnova 对 Laravel 逐实例 `makeHidden` / `makeVisible` 的 Rust 原生替代。Eloquent 结构体不携带运行时的属性包，所以列清单是在调用点给出的：

```rust
return Json::ok(user.to_array_except(&["password_hash", "remember_token"]));
return Json::ok(user.to_array_only(&["id", "name", "email"]));
```

**分歧说明。** Laravel 逐实例的 `makeHidden` 修改的是一份状态，当这个模型被嵌套进父级的 `toArray()` 调用时，这份状态会传播下去。Suprnova 的过滤是终结性的 - 它产出一个 `serde_json::Value`，不会影响 `self` 以后的序列化。要做声明式且永久的可见性控制，请使用 `#[model(hidden = [...])]` / `#[model(visible = [...])]` 属性。

### UUID / ULID 主键 - `#[model(unique_id = "...")]`

这是 Suprnova 对应 Laravel `HasUuids` / `HasUlids` / `HasVersion4Uuids` 这一族 trait 的东西。设置这个属性，把主键的类型写成 `String`，这个宏就会在 INSERT 之前自动填好这个 ID。

```rust
#[model(
    table = "users",
    primary_key = "id",
    key_type = "String",
    auto_increment = false,
    unique_id = "uuid",      // 或者 "uuid_v4"、"ulid"
)]
pub struct User {
    pub id: String,
    pub email: String,
}

// 自动填充：
let u = User::create(attrs! { email: "a@b.com" }).await?;
// u.id 是一个全新的 UUID v7。

// 调用方提供的 ID 仍然胜出（与 Laravel 的 HasUuids 行为一致）。
let u = User::create(attrs! { id: "...", email: "..." }).await?;
```

支持的策略：

- `"uuid"` / `"uuid_v7"` - UUID v7（按时间戳排序，推荐；与 Laravel 11+ 默认的 `Str::uuid7()` 一致）
- `"uuid_v4"` - 随机 UUID（对应 `HasVersion4Uuids`）
- `"ulid"` - 小写的 26 字符 Crockford-base32 ULID

这个宏会发出一个 `impl HasUniqueId for YourStruct` 块，暴露出 `UNIQUE_ID_KIND` 和一个 `new_unique_id()` 钩子，您可以在类型上覆盖它来接入自定义生成器（比如 `usr_<uuid>` 这样带前缀的 ID）。

### `find_or` / `find_or_new` / `create_or_first`

补齐 `FirstOrCreate` trait 的表面。

```rust
// 按主键查找；找不到就运行这个兜底。
let user = User::find_or(id, || async {
    User::create(attrs! { id, name: "guest" }).await
}).await?;

// 按主键查找；找不到就用默认值构建一个未保存的实例。
let user = User::find_or_new(id, attrs! { name: "draft" }).await?;
// 这里 user.id == 0 - 这个实例只存在于内存里。

// 竞态安全的插入：先试着创建，冲突时回退到获取。
let user = User::create_or_first(
    attrs! { email: "race@x.com" },
    attrs! { name: "race winner" },
).await?;
```

### `without_touching` 作用域

这是 Suprnova 对应 Laravel `Model::withoutTouching` 的东西。在这个作用域内部，每一次 `model.touch().await` 调用都会短路 - 在运行数据迁移、或者那些通过别的路径修改时间戳的批处理作业时，这很有用。

```rust
use suprnova::eloquent::without_touching;

without_touching(async {
    // 这里的 .touch() 调用都是空操作。
    for post in posts {
        post.touch().await?;
    }
}).await;
```

这个作用域由 `tokio::task_local` 支撑，所以其他任务上的并发请求，仍然只遵从它们自己的作用域（或者它的缺席）。
`without_touching` 也会抑制[父级 touch 级联](#父级-touch) - 在该作用域内保存的子项，不会 touch 其 `touches` 列表中命名的任何所有者。

`without_touching_on::<Post, _, _>(fut)` 是按类型的形式 - Laravel 的 `Model::withoutTouchingOn([Post::class], $cb)`。在它里面，`post.touch()` 和任何原本会提升 `Post` 的级联都会静默，而其他每种类型的所有者仍会提升：

```rust
use suprnova::eloquent::without_touching_on;

without_touching_on::<Post, _, _>(async {
    // 此处的 Comment 保存不会 touch 它们的 Post 所有者；同一个 comment
    // 上的 Video 所有者仍会提升。
    comment.save().await
}).await?;
```

作用域可嵌套，两者都由 `tokio::task_local` 支撑。

## 下一步

- [Eloquent 关系](eloquent-relationships.md) - 深入探讨每一种关系类型、多态注册表，以及多态枚举到底层表示的映射
- [Eloquent 集合](eloquent-collections.md) - 完整的 `Collection<T>`
  表面、通用与感知模型两种表面的区分，以及 `LazyCollection<M>`
  流式处理
- [Eloquent 转换、访问器和修改器](eloquent-mutators.md) - 22 种内置转换，外加 `casts!` 运行时覆盖
- [Eloquent 序列化](eloquent-serialization.md) - `to_array`、
  `to_json`、hidden / visible / appends、过滤后的终结方法
- [Eloquent 工厂](eloquent-factories.md) - 供测试和填充器使用的随机化模型实例
