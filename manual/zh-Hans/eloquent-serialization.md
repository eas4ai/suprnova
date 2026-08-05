# Eloquent 序列化

Eloquent 模型是怎么变成 JSON 的。本章覆盖 `to_array()` 和 `to_json()`、`hidden` / `visible` / `appends` 这条过滤管道、两个终结帮助函数 `to_array_except` / `to_array_only`、appends 把访问器接进输出的方式，以及两处会让人栽跟头的与 Laravel 的分歧：serde 绕过陷阱，以及预加载的关系不会自动折进 JSON 主体这一事实。

如果您已经读过 [Eloquent API](eloquent.md)，这里的大多数名字应该都不陌生 - 属性参考就在那一章里。这一页要讲的是*序列化契约*本身：哪些字段会出现、过滤器按什么顺序生效，以及忘了什么会造成泄漏。

## 目录

- [契约](#契约)
- [`to_array` 和 `to_json`](#to-array-和-to-json)
- [隐藏字段 - `hidden = [...]`](#隐藏字段-hidden)
- [允许字段 - `visible = [...]`](#允许字段-visible)
- [追加访问器 - `appends = [...]`](#追加访问器-appends)
- [过滤管道的顺序](#过滤管道的顺序)
- [逐次调用的过滤 - `to_array_except` / `to_array_only`](#逐次调用的过滤-to-array-except-to-array-only)
- [按查看者进行条件隐藏](#按查看者进行条件隐藏)
- [serde 绕过陷阱](#serde-绕过陷阱)
- [序列化集合](#序列化集合)
- [预加载的关系与序列化](#预加载的关系与序列化)
- [JSON:API 呢？](#json-api-呢)
- [每一部分位于何处](#每一部分位于何处)
- [下一步](#下一步)

## 契约

每一个 `#[suprnova::model]` 结构体都会从 `Model` trait 上拿到两个序列化方法：

```rust
fn to_array(&self) -> serde_json::Value;
fn to_json(&self) -> String;
```

`to_array` 产出一个供处理程序响应和测试使用的 `serde_json::Value`。`to_json` 是一个薄包装 - `serde_json::to_string(&self.to_array())` - 所以同一条过滤管道拥有这两种形态。

输出是一个以结构体字段名（或者您应用过的任何 serde 改名）为键的 JSON 对象，会经过在 `#[model(...)]` 上声明的三个可选开关过滤：

- `hidden = [...]` - 列的拒绝列表
- `visible = [...]` - 列的允许列表（与 `hidden` 互斥）
- `appends = [...]` - 要注入到具名键下的访问器方法

当模型一个都没声明时，会运行 trait 的默认实现：通过 `serde_json::to_value(self)` 序列化 `self`，剥掉两个框架内部的暂存字段（`__eager` 和 `__pivot` - 参见[预加载的关系](#预加载的关系与序列化)），返回结果。当模型声明了其中任意一个时，宏会发出一个覆盖实现，运行这个[过滤管道](#过滤管道的顺序)。

## `to_array` 和 `to_json`

一个最小可用的例子 - 把一行数据以 JSON 的形式发出去：

```rust
use suprnova::{json_response, model, Model, Request, Response};
use chrono::{DateTime, Utc};

#[model(table = "users")]
pub struct User {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub async fn show(req: Request) -> Response {
    let id: i64 = req.param("id")?.parse()
        .map_err(|_| suprnova::FrameworkError::param_parse("id", "i64"))?;
    let user = User::find_or_fail(id).await?;
    json_response!(user.to_array())
}
```

`json_response!` 接受任何 `serde_json::Value`；`user.to_array()` 恰好产出一个。字符串形态的对应物是 `user.to_json()` - 相同的主体，相同的过滤器，只多了一次 `to_string`。

您也可以直接伸手去用 `serde_json::to_value(&user)`。**对任何面向用户的场景都不要这样做。** 它会彻底绕开这条过滤管道 - 原因请参见本章后面的[serde 绕过陷阱](#serde-绕过陷阱)。

## 隐藏字段 - `hidden = [...]`

拒绝列表的形态。除了列出的那些列之外，每一列都会被序列化：

```rust
use chrono::{DateTime, Utc};
use suprnova::{model, Model};

#[model(
    table = "users",
    fillable = ["name", "email", "password"],
    hidden = ["password", "remember_token"],
)]
pub struct User {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub password: String,
    pub remember_token: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

这个模型面向用户的 JSON，永远不会包含 `password` 或 `remember_token`：

```json
{
    "id": 42,
    "name": "Alice",
    "email": "alice@example.com",
    "created_at": "2026-05-30T11:14:22Z",
    "updated_at": "2026-05-30T11:14:22Z"
}
```

当**大多数字段都要发给客户端**，而您只需要减去一小部分密钥、内部标志或者仅供认证使用的数据时，`hidden` 就是正确的工具。

## 允许字段 - `visible = [...]`

允许列表的形态。只有列出的那些列会被序列化：

```rust
#[model(
    table = "users",
    visible = ["id", "name", "avatar_url"],
)]
pub struct PublicUserView { /* ... */ }
```

对于那种专门存在、只是为了充当一个精简的公开投影的模型（想想 Laravel 的 `Profile` / `PublicUser` 这类类型），这很有用。当一张表持有几十个内部列，而只有少数几个需要发给客户端时，`visible` 同样是正确的工具 - 列出要保留的那一小撮，比列出要剔除的一大堆更短。

`hidden` 和 `visible` 在**编译期是互斥的**。如果您两个都设置了，宏会报错：

```text
error: cannot specify both `hidden` and `visible` on the same model
 --> src/models/user.rs:7:1
  |
7 | #[model(table = "users", hidden = ["x"], visible = ["y"])]
  | ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
```

这两者是策略上的两个极端 - 选择意图与您模型的形状相符的那一个，而不是两个都用。

## 追加访问器 - `appends = [...]`

`appends` 把计算出来的值注入到 JSON 输出里。每一项都指名模型上一个标了 `#[accessor]` 的方法；宏会在 `to_array()` 期间调用它，并把返回值存到同名的键下。

```rust
use suprnova::{accessor, model, Model};

#[model(
    table = "users",
    fillable = ["first_name", "last_name"],
    appends = ["full_name", "initials"],
)]
pub struct User {
    pub id: i64,
    pub first_name: String,
    pub last_name: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl User {
    #[accessor]
    pub fn full_name(&self) -> String {
        format!("{} {}", self.first_name, self.last_name)
    }

    #[accessor]
    pub fn initials(&self) -> String {
        let f = self.first_name.chars().next().unwrap_or(' ');
        let l = self.last_name.chars().next().unwrap_or(' ');
        format!("{f}{l}")
    }
}
```

序列化之后的 user 现在带着这两个计算出来的键：

```json
{
    "id": 7,
    "first_name": "Alice",
    "last_name": "Pond",
    "created_at": "...",
    "updated_at": "...",
    "full_name": "Alice Pond",
    "initials": "AP"
}
```

宏会在编译期校验 `appends` 里的每一项：

- 每个名字都必须能解析成一个 Rust 标识符（`"full-name"` 会失败 - 它不是一个合法的 ident）。
- 如果指名的方法在模型的 `impl` 块上不存在，编译器会指向宏生成的分发器，报出一条清楚的 `no method named 'full_name' found` 错误。

从 Rust 里直接调用 `user.full_name()`，其表现和任何其他方法完全一样 - `appends` 只控制**JSON 分发表**。访问器仍然是普通的方法。

## 过滤管道的顺序

当一个模型声明了 `hidden`、`visible` 或 `appends` 中的任意一个时，宏会发出一个 `to_array` 覆盖实现，按以下顺序运行四个步骤：

1. 通过 `serde_json::to_value` 把 `self` 序列化成一个 `serde_json::Map`。
2. 无条件剥掉框架内部的 `__eager` 和 `__pivot` 键（关于它们的更多内容，见[关系那一节](#预加载的关系与序列化)）。
3. 当 `visible` 非空时，把它当作一份**允许列表**应用：任何不在这份列表里的键都会被移除。
4. 把 `hidden` 当作一份**拒绝列表**应用：任何列在其中、且在允许列表那一步存活下来的键都会被移除。
5. 注入 `appends`：对每一项，调用注册的访问器，把它的结果插入到这一项的名字下。

### 为什么 Suprnova 有所不同

Laravel 运行的是同样的 `hidden` → `visible` → `appends` 顺序。分歧在第 5 步：在 Suprnova 里，appends 是在隐藏拒绝列表**之后**运行的，并且它们总会出现 - 即便它们的名字也列在 `hidden` 里。这个理由和 Laravel 的一样：如果您同时声明了 `$appends = ['full_name']` 和 `$hidden = ['full_name']`，意图就是“算出来、发出去” - `appends` 是更具体的那个信号。当一个访问器的键和某一列的名字冲突时（例如一个访问器覆盖了存储的 `display_name` 列的值），这个顺序就很关键；发给客户端的会是访问器的结果。

## 逐次调用的过滤 - `to_array_except` / `to_array_only`

对于列声明不合适的一次性场景，有两个终结帮助函数会先运行完整的 `to_array` 管道，再按名字修剪结果：

```rust
use suprnova::{json_response, Model};

pub async fn admin_show(user: User) -> suprnova::Response {
    // 给一个需要这一行大部分数据、但不需要这几个字段的
    // 管理端点，去掉几个多余的字段：
    json_response!(
        user.to_array_except(&["password_hash", "remember_token", "internal_notes"])
    ))
}

pub async fn directory_show(user: User) -> suprnova::Response {
    // 公开目录 - 只发布我们想发布的这些列：
    json_response!(
        user.to_array_only(&["id", "name", "avatar_url"])
    ))
}
```

两者都产出一个 `serde_json::Value` - 它们不会改变 `self`，也不会改变同一行未来的序列化结果。它们会先运行完整的 `hidden` / `visible` / `appends` 管道，然后在此之上再应用自己的修剪。`to_array_only` 返回一个*全新的* JSON 对象，只包含指名的那些键；`to_array_except` 返回完整对象减去指名的那些键。

### 为什么 Suprnova 有所不同

Laravel 的 `$user->makeHidden(['x'])` 和 `$user->makeVisible(['x'])` 会**改变**模型实例 - 之后每一次 `toArray()` 调用，包括这个模型被嵌套在某个父级序列化里面时发生的调用，看到的都是变化后的状态。Suprnova 的这些帮助函数是**终结性的**。它们产出一个 `Value` 就停下来。如果您需要这个改动能传播出去，就把它声明在 `#[model(hidden = [...])]` / `#[model(visible = [...])]` 上，让*类型*来表达这条策略，而不是在实例上做一次隐藏的改动。

从 Rust 的形状来看，原因是：Suprnova 里的一个 Eloquent 结构体，就是一个没有运行时属性包的朴素 Rust 结构体。一个实例侧的可见性标志，如果不引入环境式的隐藏状态，就无处安放 - 而这正是框架有意要避开的那一类陷阱。

## 按查看者进行条件隐藏

当可见性取决于查看者时，惯用的模式是在调用点做一次匹配，分支进正确的那个逐次调用过滤：

```rust
use suprnova::{Auth, json_response, Model, Request, Response};

pub async fn show(req: Request) -> Response {
    let id: i64 = req.param("id")?.parse()
        .map_err(|_| suprnova::FrameworkError::param_parse("id", "i64"))?;
    let user = User::find_or_fail(id).await?;
    let viewer = Auth::user_as::<User>().await?;
    let viewing_self = viewer.as_ref().map(|v| v.id) == Some(user.id);

    let body = if viewing_self {
        user.to_array()
    } else {
        user.to_array_except(&["email", "phone", "stripe_customer_id"])
    };

    json_response!(body)
}
```

对于更精细的逐查看者形态 - 给管理员、试用用户、付费用户分别不同的属性 - 正确的工具是带着 `Maybe<T>` / `MissingValue<T>` 字段的**JSON:API 资源层**。声明式的写法请参见 [JSON:API 资源](eloquent-resources.md#conditional-attributes--maybet--missingvaluet)。

## serde 绕过陷阱

这是关于 Suprnova 里 Eloquent 序列化，最重要的一件事。

**`hidden` / `visible` / `appends` 这些过滤器只会在经过 `to_array()` 和 `to_json()` 时运行。** 派生出来的 `Serialize` 实现*不会*强制它们。通过任何其他 serde 路径返回这个结构体，都会彻底绕开这些过滤器。

这意味着**下面这些全都会泄漏 `password`**：

```rust
// 直接用 serde - 绕开了 to_array，hidden 不起作用：
let raw = serde_json::to_value(&user).unwrap();

// json_response! 配上一个结构体字段 - 同样的问题：
json_response!({ "user": user }))

// 嵌套在另一个可序列化的容器里面 - 同样的问题：
#[derive(Serialize)]
struct EnvelopeWithUser { ok: bool, user: User }
let env = EnvelopeWithUser { ok: true, user };
json_response!(env))

// 通过 serde 返回一个 Vec<User> - 同样的问题：
json_response!(users))   // 这里 users: Vec<User>
```

只有下面这些会经过这条过滤管道：

```rust
json_response!(user.to_array()))
json_response!(users_collection.to_array()))  // Collection<User>
json_response!(user.to_array_except(&["secret"])))
json_response!(user.to_array_only(&["id", "name"])))
```

### 为什么会这样

serde 那个对 `Vec<T>`（以及任何其他容器）的兜底 `Serialize` 实现，会直接调用 `T::serialize`。Suprnova 的过滤管道活在 `Model::to_array` 这个 trait 方法里，不在 `Serialize` 里。除非您调用它，这个 trait 方法不会被触发。

框架会防范这个*内部*陷阱（`__eager` / `__pivot` 这两个暂存字段被标了 `#[serde(skip)]`，所以它们不会从任何一条路径泄漏出去），但这个宏刻意**不会**在隐藏字段上发出 `#[serde(skip_serializing)]` - 这样做会破坏那些合理地在内部 SeaORM 模型上使用 serde、且调用者想要完整一行的场景（例如内部 RPC、持久化层、诊断、测试）。

### 规则

对任何要跨越信任边界、回传给客户端的值，都要走一遍 `to_array()` 或者它那些经过过滤的同族函数。这份能换来安全的四行契约是：

| 想要 | 用 | 结果 |
|---|---|---|
| 序列化一个模型 | `user.to_array()` | 过滤后的 JSON 对象 |
| 序列化一个集合 | `collection.to_array()` | 过滤后的 JSON 数组 |
| 减去几个字段 | `user.to_array_except(&["x"])` | 过滤后再减去 |
| 只保留几个字段 | `user.to_array_only(&["x"])` | 只有列出的那些键 |

针对模型值，用一个 linter，或者在 PR 阶段人工审查 `json_response!\({.*: [a-z_]+ ?})` 和 `serde_json::to_value\(&\w+\)` 这两个模式，是坚持这条规则的一种低成本办法。框架自身针对 `Model` 序列化的测试覆盖了这两条路径。

## 序列化集合

`Collection<M>` - 由 `Builder::get()`、`Model::all()`，以及关系访问器返回 - 有它自己的 `to_array()` 和 `to_json()`，会遍历底层的 `Vec<M>`，**逐行**调用 `to_array()`。结果是一个由过滤后的对象组成的 JSON 数组：

```rust
use suprnova::{json_response, Model};

pub async fn list() -> suprnova::Response {
    let users = User::all().await?;
    json_response!(users.to_array())
}
```

这是在一个多行结果上拿到逐行过滤的唯一地方。`serde_json::to_value(&users)` 会通过 serde 的兜底实现发出一个 Vec，一次性绕开每一行上的过滤器 - 集合层面的这个帮助函数存在的意义，正是为了补上这个缺口。

```rust
// Collection<M> 的覆盖实现：
pub fn to_array(&self) -> Value {
    Value::Array(self.0.iter().map(|m| m.to_array()).collect())
}
```

对于一个分页器，被包装的数据活在 `LengthAwarePaginator::data` / `CursorPaginator::data` 里，是一个 `Vec<M>` - 在组装分页器响应之前，对每一项调用 `.to_array()`，或者使用 [JSON:API 分页形态](eloquent-resources.md#pagination)，它会把逐行过滤当作资源管道的一部分来处理。

## 预加载的关系与序列化

这是需要吃透的第二处分歧。

当您在一个构造器上调用 `.with(["posts"])` 时，框架会加载这些 posts，并把它们存进一个逐行的 `EagerLoadCache`（自动注入的 `__eager` 字段）。读取它们的那个访问器 - `user.posts_loaded()` - 就是从这个缓存里取数据。

**这个缓存是 `#[serde(skip)]` 的，`to_array()` 会无条件剥掉它。** 预加载的关系不会自动折进 JSON 输出。对一个预加载了 posts 的 user 调用 `to_array()`，看起来和对一个没预加载的 user 调用 `to_array()` 一模一样。

### 为什么 Suprnova 有所不同

Laravel 的 `toArray()` 会遍历 `$model->getRelations()`，把每一个已加载的关系都折进输出里。PHP 那个数组形状的模型包，让这件事显得很自然 - 一个关系不过是模型上另一个带键的条目。

Rust 那些类型化的 Eloquent 结构体没有这个包。一个 `User` 结构体持有的是类型化的列，不是一个“无论加载了什么关系”的异质映射。把 `posts` 折进去，需要的要么是对一个类型化结构体做运行时字段注入（一种 serde 绕过机制），要么是一条并行的序列化路径，在跑完列的序列化器之后再去查阅那个缓存。这两个选项都会把每个模型的 JSON 形态，和某个特定调用者预加载了哪些关系耦合起来 - 这在 PHP 里是一个承重的契约，因为客户端学会了依赖它；而 Suprnova 明确拒绝提供这样的契约，因为它会让 JSON 的形态依赖于调用方那一侧的查询构造方式。

### 两种发布关系数据的方式

**1. 显式的访问器 + appends。** 定义一个从 `<rel>_loaded()` 取数据的方法，把它注册进 `appends`。这个关系就会出现在您指名的那个键下面。当这个关系在读取路径上*总是*被预加载时，这个办法可行：

```rust
use suprnova::{accessor, model};
use serde_json::Value;

#[model(
    table = "users",
    appends = ["posts"],
)]
pub struct User { /* ... */ }

impl User {
    #[accessor]
    pub fn posts(&self) -> Value {
        // 如果读取路径上没调用过 .with(["posts"])，posts_loaded()
        // 就会 panic。这个访问器必须在预加载之后运行。
        let posts = self.posts_loaded();
        serde_json::to_value(posts).unwrap_or(Value::Null)
    }
}

// 读取路径必须预加载：
let users = User::query()
    .with(["posts"])
    .get()
    .await?;
let body = users.to_array();   // 每个 user 的 "posts" 键都有值
```

这份契约很醒目：忘了 `.with(["posts"])`，访问器就会在第一行的 `posts_loaded()` 调用上 panic（按设计，这个预加载缓存在关系没被加载时，读取就会 panic - 一个静默的空数组会把这个 bug 藏起来）。对于可选的预加载，用返回 `Option<&T>` 的 HasOne 形态，给您一次 `match` 的机会：

```rust
impl User {
    #[accessor]
    pub fn profile(&self) -> Value {
        match self.profile_loaded() {
            Some(profile) => serde_json::to_value(profile).unwrap_or(Value::Null),
            None => Value::Null,
        }
    }
}
```

**2. JSON:API 资源层。** 当关系的形态和包含策略应该由传输格式而不是模型来决定时，就用一个带 `#[derive(Data)] #[json_resource]` 的结构体，在关系字段上标注 `#[data(allow_include)]`。客户端通过 `?include=posts.comments` 选择加入，框架会走遍这棵 include 树，用去重后的资源对象填充 `included`。这是正确答案的场景包括：

- 关系的形态是一个传输格式层面的关切（稀疏字段集、条件性包含、跨链接的元数据）。
- 不同的端点想要不同的默认包含项。
- 同一个模型出现在不同的信封下面（一个端点发布 `posts`，另一个发布 `subscriptions`）。

完整的模式请参见 [JSON:API 资源](eloquent-resources.md#compound-documents--include-chains)。

## JSON:API 呢？

`to_array()` 这条管道和 `Resource` / `JsonApi` 门面是两个层次，服务于不同的工作：

| 关切 | `Model::to_array` | `Resource::single` / `JsonApi::single` |
|---|---|---|
| **形态** | 扁平对象 - 列名直接映射成键 | JSON:API 信封（`data`、`included`、`meta`、`links`、`jsonapi`） |
| **逐属性的把控** | `#[model]` 上的 `hidden` / `visible` / `appends` | `#[data(input_only)]`、`Maybe<T>`，以及通过 `?fields[type]=` 实现的稀疏字段集 |
| **关系** | 手动（访问器 + appends，见上文） | 通过 `#[data(allow_include)]` + `?include=` 一等公民地支持 |
| **分页** | 手动包装一个 `Vec<Value>` | `Resource::paginated(p)` 会处理 links + meta |
| **错误** | 通过 `FrameworkError` 渲染 | `into_json_api_response()` 产出 JSON:API 的 `errors` 信封 |
| **什么时候该用它** | 简单的端点、内部工具、临时性的形态 | 公开 API、第三方消费者、了解 JSON:API 的客户端 |

`to_array()` 是更底层的那一层 - 大多数内部处理程序、管理页面、Inertia props（通过 serde）和测试调用的都是它。JSON:API 这一层是在它之上组合出来的：它不会取代 `to_array`，而是围绕那些过于丰富、不适合活在模型本身上的逐资源属性/关系逻辑，加上一层信封。

对于类型化的 Inertia props，您几乎总是想要资源层，或者一个带着显式字段的专用 `#[derive(Serialize)]` DTO，而不是直接把模型顺着 serde 管道传下去。Inertia 的返回值和其他任何东西一样，会受到同样的 serde 绕过待遇 - 安全的做法是“构造一个 DTO，从 `to_array()` 里填充它，返回这个 DTO”。

## 每一部分位于何处

| 关切 | 文件 |
|---|---|
| `Model::to_array` / `to_json` trait 默认实现 | `framework/src/eloquent/model.rs` |
| `Model::to_array_except` / `to_array_only` | `framework/src/eloquent/model.rs` |
| `Model::__append_accessor` trait 默认实现 | `framework/src/eloquent/model.rs` |
| 宏发出的 `to_array` 覆盖实现（过滤管道） | `suprnova-macros/src/model/serialization.rs` |
| 宏发出的 `__append_accessor` 分发器 | `suprnova-macros/src/model/serialization.rs` |
| `Collection<M>::to_array` / `to_json` | `framework/src/eloquent/collection.rs` |
| `EagerLoadCache`（`__eager` 字段） | `framework/src/eloquent/relations/eager_cache.rs` |
| `hidden` / `visible` / `appends` 宏解析 | `suprnova-macros/src/model/parse.rs` |
| 函数级的 `#[accessor]` 宏 | `suprnova-macros/src/lib.rs` |

## 下一步

- [Eloquent API](eloquent.md) - 完整的模型表面、属性参考，以及 `#[accessor]` / `#[mutator]` 定义在哪里
- [JSON:API 资源](eloquent-resources.md) - 那个声明式的资源层，用于更丰富的逐查看者形态、稀疏字段集，以及复合的 `?include=` 文档
- [验证](validation.md) - 请求输入是如何在模型层看到它之前，变成一个类型化结构体的
- [响应](responses.md) - `HttpResponse` 构造器、请求头和 cookie；`json_response!` 最终产出的就是这个表面
- [错误模型](error-model.md) - 一个错误是如何变成一个 JSON 主体的，带着和成功路径相同的 `request_id` 关联
