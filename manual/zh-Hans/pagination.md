# 分页

Suprnova 发布了三个逐行对齐 Laravel 表面的分页器：长度感知型（知道总数）、简单型（每页一次查询），以及游标型（不透明键集）。三者都派生了 `Serialize`，产出的是 Inertia 和 JSON:API 消费者早已理解的那种 Laravel 形状的 JSON - 您获取一页，把它返回；不需要别的任何东西。

```rust
use crate::models::User;

let page = User::query()
    .filter("active", true)
    .order_by_desc("created_at")
    .paginate(20)
    .await?;
```

那一次调用就跑完了 `COUNT(*)` 和 `LIMIT/OFFSET` 这次页面获取，从活跃的请求里解析出 `?page=N`，返回一个可以直接发布的 `LengthAwarePaginator<User>`。它的两个姊妹方法 - `simple_paginate(20)` 和 `cursor_paginate(20)` - 返回同样形态的值，但权衡不同。这一章剩下的部分，讲的是该伸手去用哪一个、每一个各自的代价，以及这份 JSON 是怎么送到手上的。

## 挑选一个分页器

最快的选择方式，是看这张权衡表：

| 方法 | 类型 | 每页查询次数 | 知道总数？ | 何时使用 |
|---|---|---|---|---|
| `paginate(n)` | `LengthAwarePaginator<M>` | 2（`COUNT(*)` + 页面查询） | 是 | UI 显示数字页码，或者“第 3 页，共 17 页” |
| `simple_paginate(n)` | `Paginator<M>` | 1（`LIMIT n+1`） | 否 | 大表；一个“下一页”按钮就够了 |
| `cursor_paginate(n)` | `CursorPaginator<M>` | 1（`LIMIT n+1`） | 否 | 无限滚动；热表上的深层分页 |

一旦您的表变大，这个代价差异就会显现出来。对一亿行做一次 `COUNT(*)`，是您请求预算里最昂贵的那条查询。`simple_paginate` 省下了这次计数。`cursor_paginate` 不但省下了这次计数，*还*避开了那个会咬住大表上每一次深层分页请求的 `OFFSET N` 线性扫描 - 只要有合适的索引，一次游标定位大致是 `O(1)` 的，不管用户身处结果集的哪个位置。

### 为什么 Suprnova 有所不同

Laravel 的分页器带着构建 URL 的助手方法 - `nextPageUrl()`、`previousPageUrl()`，以及那个由 Blade 渲染的、由 `{url, label, page, active}` 描述符构成的 `links` 数组。Suprnova 原始的 `Serialize` 实现只发出数据切片加计数器；URL 的构建活在那些已经掌握着 URL 上下文的响应形态构造器上：[`Inertia::paginate`](frontend-inertia-responses.md) 附上 Inertia 的滚动元数据（页面标识符，不是绝对 URL）；[`Resource::paginated`](eloquent-resources.md) 按照 JSON:API 的推荐做法，附上 JSON:API 的 `links.{self,first,last,prev,next}`。

这个拆分有两个原因。第一，客户端应该看到的 URL，取决于是哪个协议表面在渲染它 - Inertia 靠的是页面标识符，JSON:API 想要的是绝对的 href。第二，这个分页器默认并不知道这次请求的基础 URL；而那些知道它的助手方法，能在它们该待的地方，一次性把这些 URL 附上去。如果您确实需要在裸的分页器上拿到 URL（自定义 JSON 信封、遥测载荷、测试断言），就调用 `with_path(...)`，再用 `url_for_page(n)` - 覆盖在下面的[URL 生成与路径](#url-生成与路径)一节里。

## `paginate` - 长度感知

```rust
use suprnova::LengthAwarePaginator;
use crate::models::User;

pub async fn index(_req: suprnova::Request) -> suprnova::Response {
    let page: LengthAwarePaginator<User> = User::query()
        .filter("active", true)
        .order_by_desc("created_at")
        .paginate(20)
        .await?;

    Ok(suprnova::json_response!(page))
}
```

这个结构体的公开字段：

```rust
pub struct LengthAwarePaginator<T> {
    pub data: Vec<T>,           // 这一页上的行
    pub current_page: u64,       // 从 1 开始
    pub last_page: u64,          // 从 1 开始；total == 0 时为 0
    pub per_page: u64,
    pub total: u64,              // 跨越所有页的每一行
    pub from: Option<u64>,       // 这一页上第一行的下标，从 1 开始
    pub to: Option<u64>,         // 这一页上最后一行的下标，从 1 开始
    pub path: Option<String>,    // url_for_page 用的基础 URL（可选）
}
```

这个派生出来的 `Serialize` 发出的 JSON：

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

`path` 未设置时会从这份 JSON 里省略；当这一页是空的时（这一页上没有行，或者请求的页码超过了最后一页），`from` 和 `to` 会是 `null`。

### 自动读取 `?page=N`

`paginate(n)` 会通过 `Context::query_param`，从活跃请求的 `?page=N` 上读取当前页码。缺失、空、非数字，以及零值都会被夹到 `1`。没有什么需要接线 - 只要有一个请求在作用域内，这个参数就会被读取。

### 一页上的多个分页器

当一个页面渲染的分页列表不止一个时，就用 `paginate_using` 给每一个都配上自己的查询字符串键：

```rust
let posts = Post::query()
    .order_by_desc("created_at")
    .paginate_using("posts_page", 10)
    .await?;

let comments = Comment::query()
    .order_by_desc("created_at")
    .paginate_using("comments_page", 25)
    .await?;
```

`paginate_using` 还会在返回的分页器上设置 `page_name`，这样 `url_for_page` 构建 URL 时，用的就是同一个键：

```rust
posts.url_for_page(2);     // "/posts?posts_page=2"（当 path 被设置时）
comments.url_for_page(3);  // "/posts?comments_page=3"
```

### 页面位置断言

完整的 Laravel `AbstractPaginator` 断言集合都实现了：

```rust
page.has_more_pages();   // current_page < last_page
page.on_first_page();    // current_page <= 1
page.on_last_page();     // !has_more_pages()
page.has_pages();        // 不在第 1 页，或者还有更多页存在
page.is_empty();         // data.is_empty()
page.is_not_empty();     // !is_empty()
page.count();            // data.len() - 这一页的切片长度，不是总数
```

`count()` 是这个切片的大小，不是总数 - 这是 Laravel `Countable` 的形态；要拿总数，请直接用 `total` 字段。

## `simple_paginate` - 一次查询，不计数

```rust
use suprnova::Paginator;
use crate::models::User;

let page: Paginator<User> = User::query()
    .order_by_desc("id")
    .simple_paginate(20)
    .await?;
```

```rust
pub struct Paginator<T> {
    pub data: Vec<T>,
    pub current_page: u64,
    pub per_page: u64,
    pub has_more: bool,          // 在 per_page 之后是否还多出了一行？
    pub path: Option<String>,
}
```

JSON：

```json
{
  "data": [...],
  "current_page": 1,
  "per_page": 10,
  "has_more": true,
  "path": "/api/users"
}
```

诀窍在这条 SQL 里。`simple_paginate(20)` 会发出 `LIMIT 21`，检查第 21 行是否被返回了，据此设置 `has_more`，再把 `data` 截回 20 行。每页一次查询；没有 `COUNT(*)`。

您放弃了 `total`、`last_page`、`from` 和 `to`。作为交换，您能对那些每次加载页面都跑一次 `COUNT(*)` 太昂贵的表做分页。它的 UI 表面是“下一页” / “上一页”按钮，不是“第 7 页，共 142 页”。

和长度感知分页器一样的那套断言集合，这里也实现了：`has_more_pages()`、`on_first_page()`、`on_last_page()`、`has_pages()`、`is_empty()`、`is_not_empty()`、`count()`。

## `cursor_paginate` - 不透明键集

```rust
use suprnova::CursorPaginator;
use crate::models::User;

let page: CursorPaginator<User> = User::query()
    .cursor_paginate(20)
    .await?;
```

```rust
pub struct CursorPaginator<T> {
    pub data: Vec<T>,
    pub per_page: u64,
    pub next_cursor: Option<String>,  // 在最后一页上为 None
    pub prev_cursor: Option<String>,  // 在第一页上为 None
    pub path: Option<String>,
}
```

JSON：

```json
{
  "data": [...],
  "per_page": 10,
  "next_cursor": "...",
  "prev_cursor": null,
  "path": "/api/users"
}
```

`next_cursor` 和 `prev_cursor` 作为 JSON 键永远存在（缺失时为 `null`），这样客户端的模式就能依赖字段的存在性；`path` 未设置时会被省略。

### 游标在线路上是怎么工作的

客户端通过 `?cursor=<opaque>` 传递上一页的游标：

```
GET /api/users?cursor=eyJ0IjoiQmlnSW50IiwidiI6MTAwLCJkIjoibmV4dCJ9...
```

`cursor_paginate` 会解码这个游标，走一遍这个键集过滤器（`next` 用 `pk > boundary ASC`；`prev` 用 `pk < boundary DESC`，再反转回 ASC），取回 `LIMIT n+1` 行，并根据这一页的邻居是否存在，重新发出 `next_cursor` / `prev_cursor`。它是双向的 - 客户端能前后走动，而不丢失自己的位置。

游标分页会**取代**构造器上任何已有的 `ORDER BY`。这个键集过滤器要能确定性地切分这张表，需要主键上有一个稳定的全序；一个任意的 `ORDER BY random_score()` 游标会跳过一些行，又重复另一些行。如果您需要一个非主键的排序，就换用 `paginate` / `simple_paginate`。

### 游标是加密并认证过的

Suprnova 的游标**不是**Laravel 那种 base64-JSON 明文。这个线路上的游标，是那个键集边界（一个类型化的 `sea_orm::Value` - `Int`、`BigInt`、`Uuid`、日期时间、小数、字符串、字节）加上一个方向标签，先 JSON 编码，再通过框架的 `Crypt` 密钥环用 AES-256-GCM 封起来（绑定在 `CryptPurpose::Cursor` 下，所以一份游标密文永远不能被重放进任何其他表面 - cookie、2FA 密钥、转换器）。

这在实践中意味着三件事：

1. **无法被篡改。** 一个在 `?cursor=` 里翻转比特位的客户端，得到的是一个 400 `Invalid pagination cursor`，不是另一页数据。
2. **没有信息泄漏。** 这个边界值（通常是一个主键，有时是一个时间戳）被密封在这个游标里面 - 客户端没法靠编辑它来枚举范围。
3. **类型化的边界值能无损地往返。** 这个线路信封给 SeaORM 的变体打上了标签（`"BigInt"`、`"Uuid"`，等等），所以在解码时，这个值会用原来那一列所发出的同一个 SQL 类型重新绑定。跨 Postgres / MySQL / SQLite 都不会有字符串强制转换的 bug。

这里没有明文回退。如果 `Crypt` 没有被初始化 - 在 `Server::from_config` 之后，这应该是不可能发生的 - 编码会报错，而不是发出一个可以被伪造的游标。

### 为什么 Suprnova 有所不同

Laravel 的游标分页器默认是只能向前的，线路上的游标是一个 base64 编码的 JSON 数据块 - 可读、可编辑、可重放。Suprnova 的游标是双向的（对应的是 Laravel 后来加上的那个 `cursorPaginate()` 表面），并且是端到端认证过的，客户端没法构造或者改动它。Rust 生态里已经有 AES-GCM 这个基元了；用它，框架只需要多付出一个 trait 实现的代价，就能让每一个游标都拿到一份明文 base64 载荷给不了的安全属性。

## 门面 - `Pagination::length_aware` / `Pagination::cursor`

这本手册的大多数章节，都是通过 Eloquent 构造器来展示分页的，因为那是常见的路径。如果您在直接构建一个 SeaORM `Select<E>` - 比方说，为一份报表联结到一个不带模型的查询上 - `Pagination` 门面就是那个等价的表面：

```rust
use suprnova::{Pagination, LengthAwarePaginator};
use sea_orm::EntityTrait;

let select = User::find()  // 或者任何 SeaORM 的 Select<E>
    .filter(user::Column::Active.eq(true));

let page: LengthAwarePaginator<user::Model> =
    Pagination::length_aware(select, 20, 1).await?;
```

这个门面还提供了 `length_aware_on(conn, ...)` 和 `cursor_on(conn, ...)`，用于路由到一个特定的具名连接，以及一个类型化的 `cursor(query, cursor, per_page, order_col)` 形式，显式接受这个键集列 - 用在游标排序依据不是主键的时候。

路由规则和 Eloquent 构造器一致。一个环境中的 `DB::transaction` 会被尊重（COUNT 和这次页面查询都会在这个事务的连接上运行），一个已注册的 `__read_replica__` 连接会被自动用于读取。当您想绕过这个副本时，`__primary__` 这个哨兵值会选中默认连接池。

## 验证 - `per_page == 0`

这三个方法都会拒绝 `per_page == 0`：

```rust
let result = User::query().paginate(0).await;
assert!(matches!(
    result,
    Err(FrameworkError::ParamError { ref param_name }) if param_name == "per_page",
));
```

这个错误会渲染成一个带标准错误体的 HTTP 400。这里没有悄悄的“空页面” - 一个零的页面大小永远是错的，会在调用点被拒绝，这和 Eloquent 构造器与 `Pagination` 门面是一致的。同样的验证活在 `cursor_paginate`、`simple_paginate`、`Pagination::length_aware`、`Pagination::length_aware_on`、`Pagination::cursor`，和 `Pagination::cursor_on` 上 - 一条规则，六个入口点。

`current_page` 这个值是被**夹住**的，不是被验证的：`0` 会变成 `1`，来自一个有防御性的前端的负数不可能发生（这个解析器是 `u64`），而任何大于 `last_page` 的 `?page=N`，都会返回一个 `data` 为空、`from`/`to` 都是 `None` 的分页器。走过头是客户端的失误，不是一个错误。

## 错误形态

| 条件 | 变体 | HTTP |
|---|---|---|
| `per_page == 0` | `FrameworkError::ParamError { param_name: "per_page" }` | 400 |
| 被篡改 / 无效的游标 | `FrameworkError::Domain`（`"Invalid pagination cursor"`） | 400 |
| 解码游标时 `Crypt` 未初始化 | `FrameworkError::Internal` | 500 |
| `decode_cursor` 上的游标变体不匹配 | `FrameworkError::Internal` | 500 |
| 底层数据库故障 | `FrameworkError::Database` | 500 |

被篡改的游标这种情形，是值得记住的那一个。游标是直接从线路上读出来的 - `?cursor=…` 这个查询字符串，按定义就是攻击者的输入，比特位被翻转的 base64 和被重放的密文，都是预期之内的失败模式，不是服务器的 bug。这次解密步骤会降级成一个 400 `Invalid pagination cursor`，这样客户端能触发的失败就不会污染 500 这条遥测通道。这条静态消息不会给客户端留下任何可以探测的东西。

解密之后的失败（JSON 解析、变体标签分发、方向解析）仍然是 500 - 任何撑过了 AEAD 认证的字节序列，都是*我们自己*生成的，所以到了这个地步还有一个格式错误的载荷，就是一个值得举报的框架 bug。

## URL 生成与路径

这个原始的分页器带着一个可选的 `path` 字段。设置了它之后，`url_for_page(n)` 和游标链接的生成都会用它来构建查询字符串：

```rust
let page = User::query()
    .paginate(20)
    .await?
    .with_path("/api/users");

page.url_for_page(1);    // "/api/users?page=1"
page.url_for_page(2);    // "/api/users?page=2"
```

当这个基础路径已经带着一个查询字符串时，这个分隔符会切换成 `&`，让这个 URL 保持格式良好：

```rust
let page = User::query()
    .paginate(20)
    .await?
    .with_path("/users?sort=name");

page.url_for_page(2);    // "/users?sort=name&page=2"
```

如果 `path` 未设置，`url_for_page` 会回退到一个裸的相对查询：`?page=2`。这个页面参数的名字来自 `with_page_name(...)`（默认是 `"page"`）；`paginate_using(name, n)` 会自动设置它，这样生成出来的 URL，用的就是驱动这个分页器时所用的同一个键。这个参数名是经过表单 URL 编码的，所以即便一个名字带着保留字符，也没法破坏这个 URL。

游标分页器有着同样的形态：`with_path(...)` 设置基础路径，`with_cursor_name(...)` 覆盖这个查询键（默认是 `"cursor"`），JSON:API 的链接构建器会自动把它们捡起来。

大多数应用不会直接调用 `url_for_page` - 它们把这个分页器交给下面两个集成表面之一，由它们按各自协议的正确方式去构建这些 URL。

## Inertia 集成 - 无限滚动 props

对于 Inertia 前端，`Inertia::paginate(component, key, paginator)` 这个助手方法会把这个分页器附加成一个滚动 prop：

```rust
use suprnova::Inertia;

pub async fn index(_req: suprnova::Request) -> suprnova::Response {
    let users = User::query()
        .order_by_desc("created_at")
        .cursor_paginate(20)
        .await?;

    Ok(Inertia::paginate("Users/Index", "users", users).into())
}
```

三个分页器在这里都能用 - `LengthAwarePaginator`、`Paginator`，和 `CursorPaginator`。这份元数据的页面名字来自分页器自身：两个偏移量分页器是 `"page"`，`CursorPaginator` 是 `"cursor"`。客户端会在选定的这个 prop 键下拿到这些行，再加上一个带着 `current_page`、`next_page`、`previous_page` 的 `ScrollMetadata` 描述符（对偏移量分页器是页面标识符；对游标分页器是游标字符串） - `useInfiniteScroll` / `WhenVisible` 这两个 Inertia 助手方法会消费它来做无限滚动。每个分页器都会通过 `ProvidesScrollMetadata` 构建这份描述符 - 这是 Laravel 的分页器适配器所满足的同一个接口（`ProvidesScrollMetadata::getPageName` / `getPreviousPage` / `getNextPage` / `getCurrentPage`）。这个 crate 不认识的分页器 - 第三方 crate 的游标类型、手写的 repository 结果 - 可以实现这四个方法，并用同样的方式把一个 `ScrollMetadata` 交给框架：参见[Inertia 响应](frontend-inertia-responses.md#merge-strategies-and-infinite-scroll)。

`simple_paginate` 值得单独点出来，因为一个针对足够大的表的列表 - 大到让 `COUNT(*)` 成为这次请求的主要开销 - 正是一个 Inertia 集合页面会感到疼痛的地方：

```rust
let users = User::query()
    .order_by_asc("id")
    .simple_paginate(20)     // 没有 COUNT，一次查询
    .await?;

Ok(Inertia::paginate("Users/Index", "users", users).into())
```

它的 `next_page` 来自那次 `LIMIT n+1` 的溢出探测，而不是来自一个计算出来的最后一页，因为没有总数可以拿来算它。客户端拿到的是“还有另一页”，不是“总共有 4,812 页” - 而这正是一个无限滚动 UI 唯一会读的东西。

### 在行送出之前投影它们

分页器没有 `map` / `through`（Laravel 的有）。请改为从公开字段重新构建 - 这些计数器和游标描述的是这次*查询*，所以换一种行类型时，它们能原样带过去：

```rust
let page = User::query().cursor_paginate(20).await?;

let page = suprnova::CursorPaginator::new(
    page.data.into_iter().map(PublicUser::from).collect(),
    page.per_page,
    page.next_cursor,
    page.prev_cursor,
);
```

只要这条路由是未经认证的，而这个模型带着任何调用方不该看到的东西，就值得这样做，而不是直接序列化这个模型。一个针对用户表的游标，一次只发出一页，但它终究会把每一页都发出去。

如果您想把一个分页器和其他 props 混在一起，同样的助手方法，也作为 `InertiaResponse::paginate(key, paginator)` 上的一个可链式方法存在：

```rust
inertia_response!("Dashboard")
    .with("stats", &stats)
    .paginate("recent_users", users)
    .into()
```

更宽泛的 prop 模型请参见 [Inertia 响应](frontend-inertia-responses.md)。

## JSON:API 集成 - `Resource::paginated`

对于 JSON:API 消费者，`Resource::paginated(paginator)` 会构建出完整的信封：

```rust
use suprnova::Resource;

pub async fn index(_req: suprnova::Request) -> suprnova::Response {
    let users = User::query()
        .paginate(20)
        .await?
        .with_path("/api/users");

    Ok(Resource::paginated(users).into())
}
```

这份响应带着：

- `data` - 每一行都经过这个模型的 `IntoJsonResource` 渲染。
- `meta.pagination` - 长度感知型是 `{ total, per_page, current_page, last_page }`；游标型是 `{ next_cursor, prev_cursor }`。
- `links.{self,first,last,prev,next}` - 长度感知分页器是绝对的 href（从 `path` 构建出来的）；游标分页器是 `links.{prev,next}`。

两种分页器类型都实现了 `Resource::paginated` 所消费的那个 `Paginated<T>` trait - 长度感知型和游标型之间没有单独的代码路径。如果您构建一个实现了 `Paginated<T>` 的、类似分页器的自定义类型，它会用同样的方式组合起来。

资源模型请参见 [JSON:API 资源](eloquent-resources.md)。

## 自定义 JSON 信封

如果 Inertia 和 JSON:API 都不适合您的客户端，就直接通过 `json_response!` 发布这个分页器：

```rust
let page = User::query().paginate(20).await?;
Ok(suprnova::json_response!({
    "users": page.data,
    "pagination": {
        "current_page": page.current_page,
        "last_page": page.last_page,
        "per_page": page.per_page,
        "total": page.total,
    }
}))
```

或者干脆把整个分页器都递过去 - 这个派生出来的 `Serialize` 实现，发出的就是上面记录的那个形态：

```rust
Ok(suprnova::json_response!(User::query().paginate(20).await?))
```

这些字段是公开的；按您契约的要求去重塑它们。

## 跨连接路由

分页遵循的是和 Eloquent 构造器一样的多连接路由。在一个 `DB::transaction(...)` 内部，COUNT 和这次页面查询都会在这个事务的连接上运行 - 它们永远不会分裂到不同的连接上，所以这个计数永远不会和它所描述的那一页产生分歧。在一个事务之外，一个已注册的 `__read_replica__` 会被自动用于读取。要把一个分页器钉在一个特定的具名连接上，请用 `Pagination` 门面上的 `_on(connection, ...)` 变体，或者从 Eloquent 那一侧用 `Builder::on("replica_b").paginate(20)`。

路由契约请参见 [Eloquent - 多连接路由](eloquent.md)。

## 该在什么时候伸手去用哪一个

一份粗略的决策树：

- **数字页码 UI 是设计的一部分** → `paginate`。您需要 `last_page` 才能渲染出“第 3 页，共 17 页”，而这个 COUNT 的代价在您表的大小上是可以接受的。
- **只有“下一页” / “上一页”按钮，大表** → `simple_paginate`。每页一次查询；您放弃了 `total` 和 `last_page`，但页面加载时间减半。
- **无限滚动** → `cursor_paginate`。双向游标意味着客户端能一直滚动过第 1000 页，而不需要先让 OFFSET 扫描几千行。
- **一个热门的、只追加的信息流的尾部** → `cursor_paginate`。按主键排序的键集是并发安全的：新行会落在这个游标之外，永远不会落在它里面。基于 OFFSET 的分页，在插入发生时会跳过一些行。
- **在一个 Eloquent 模型之外构建一个 `Select<E>`** → `Pagination::length_aware` / `Pagination::cursor`。同样的权衡；这个门面就是那个不带模型的等价物。

拿不准的时候，就从 `paginate` 开始。当 `COUNT(*)` 出现在您的慢查询日志里时，转向 `simple_paginate`。当深层分页开始主导请求耗时，或者当这个 UI 是无限滚动时，转向 `cursor_paginate`。

## 每一部分位于何处

| 部分 | 文件 |
|---|---|
| `Pagination` 门面、`Paginated<T>` trait | `framework/src/pagination/mod.rs` |
| `LengthAwarePaginator<T>` | `framework/src/pagination/length_aware.rs` |
| `Paginator<T>`（简单型） | `framework/src/pagination/simple.rs` |
| `CursorPaginator<T>`、`CursorDirection`、`encode_value`、`decode_value` | `framework/src/pagination/cursor.rs` |
| `IntoInertiaScroll` 桥接 | `framework/src/pagination/inertia.rs` |
| `Builder::paginate` / `simple_paginate` / `cursor_paginate` | `framework/src/eloquent/builder.rs` |
| `Inertia::paginate`、`InertiaResponse::paginate` | `framework/src/inertia/facade.rs`、`framework/src/inertia/response.rs` |
| `Resource::paginated`、`JsonApi::paginated` | `framework/src/resources/response.rs` |

## 下一步

- [Eloquent API](eloquent.md) - 驱动着每一个从 `Builder::paginate*` 返回的分页器的那个模型层
- [查询构造器](queries.md) - 与 `Pagination::length_aware` 和 `Pagination::cursor` 组合使用的那些不带模型的查询
- [Inertia 响应](frontend-inertia-responses.md) - 滚动 props 是如何把分页器附加到 Inertia 页面上的
- [JSON:API 资源](eloquent-resources.md) - `Resource::paginated`、链接、元数据，以及 `Paginated<T>` trait
- [错误模型](error-model.md) - `FrameworkError::param` 这条验证规则，以及游标篡改的降级处理
