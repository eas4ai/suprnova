# Eloquent 关系

[Eloquent](eloquent.md) 覆盖的是日常会用到的关系表面 - 声明语法、选项表、逐类型的基本链式调用。本章是关系专属的深入探讨：一次 `user.posts()` 调用究竟是怎么解析成 SQL 的，预加载器是怎么避开 N+1 的，存在性引擎（`has` / `where_has` / `where_belongs_to`）是怎么渲染出关联的 `EXISTS` 子查询的，多态是怎么在 Rust 缺少后期静态绑定的情况下存活下来的，以及当全部十一种关系类型都必须共存在一个 trait 上时，类型系统会漏出些什么。

如果您刚开始接触 Suprnova 上的 Eloquent，请先读 [Eloquent](eloquent.md#relationships) - 那一页教的是声明语法。这一页假定您已经有一个带着 `relations = { ... }` 块的模型，想弄明白底下发生了什么。

## 十一种关系类型

[`RelationKind`][relations] 里的每一种关系类型，都是下面之一：

| 类型                  | 侧       | 基数 | 跨族 | 中间表 |
|-----------------------|------------|-------------|-----------------|-------|
| `HasOne<R>`           | 父级     | 一         | 否              | - |
| `HasMany<R>`          | 父级     | 多        | 否              | - |
| `BelongsTo<R>`        | 子级     | 一         | 否              | - |
| `BelongsToMany<R, P>` | 任一方     | 多        | 否              | 有   |
| `HasOneThrough<B, R>` | 父级     | 一         | 否              | - |
| `HasManyThrough<B, R>`| 父级     | 多        | 否              | - |
| `MorphOne<R>`         | 父级     | 一         | 是              | - |
| `MorphMany<R>`        | 父级     | 多        | 是              | - |
| `MorphTo`             | 子级     | 一         | 是（n 个目标） | - |
| `MorphToMany<R, P>`   | 父级     | 多        | 是              | 有   |
| `MorphedByMany<R, P>` | 多对多伙伴方| 多        | 是（反向）   | 有   |

“跨族”指的是被关联行的*类型*会变化 - 一个 `Comment` 可能属于一个 `Post`，也可能属于一个 `Video`，不是固定指向单一的父表。这就是多态，Suprnova 通过[多态注册表](#多态注册表)外加一个按族的枚举来处理它。

[relations]: https://docs.rs/suprnova

### 这个宏会发出什么

当您这样写时：

```rust
use suprnova::model;

#[model(table = "users", relations = {
    posts: HasMany<Post>,
})]
pub struct User {
    pub id: i64,
    pub name: String,
}
```

`#[suprnova::model]` 会为 `posts` 展开出五样东西：

1. **关系方法** - `fn posts(&self) -> HasMany<Self, Post>`。返回一个惰性包装，携带着 `self.id` 加上外键元数据；此时还没有任何 SQL 运行。
2. **已加载访问器** - `fn posts_loaded(&self) -> &[Post]`。在 `User::with(["posts"])` 之后，从预加载缓存里读取。如果没有运行过预加载，就是一个空切片。
3. **计数访问器** - `fn posts_count(&self) -> u64`。在 `User::with_count(["posts"])` 之后，从同一个缓存里读取。
4. **分发器分支** - 模型的 `__eager_load` 固有方法里的一个 match 分支。预加载器会查找 `"posts"`，运行那条 `IN` 查询。
5. **清单条目** - 一次 `inventory::submit!(RelationEntry { ... })`，这样这个关系在运行时就是可枚举的（管理工具、存在性引擎、多态分发器都会遍历它）。

您永远看不到 (4) 或 (5)。它们撑起了本章剩下的内容。

## 惰性决议：`user.posts()` 是怎么变成 SQL 的

`user.posts()` 返回的是一个 `HasMany<User, Post>` 包装，不是一个查询结果。这个包装持有父级的主键值，加上外键列名，以及一个已经预先过滤好、应用了 `WHERE posts.user_id = ?` 的 `Builder<Post>`。此时还没有任何东西碰过数据库。

```rust
use suprnova::Direction;

// 没有 SQL 执行。
let posts_q = user.posts();

// SQL: SELECT * FROM posts WHERE user_id = ? ORDER BY id DESC LIMIT 5
let recent = user.posts()
    .order_by("id", Direction::Desc)
    .limit(5)
    .get()
    .await?;

// SQL: SELECT COUNT(*) FROM posts WHERE user_id = ?
let n = user.posts().count().await?;
```

双 API 表面（[Eloquent → 命名说明](eloquent.md#naming-note-dual-api)）在这个包装上同样受到尊重：`.filter("col", v)` 和 `.db_where("col", v)` 两者都能用，效果完全一样。`HasOne` / `HasMany` / `MorphOne` / `MorphMany` 上的可链式表面覆盖 `filter` / `db_where` / `order_by` / `latest` / `oldest` / `limit` / `take`。穿透关系和多态多对多关系只暴露它们的终结方法 - 它们走的是手写的 SQL 拼接，不是一个 `Builder<R>`，所以没法和标准链组合起来。参见下文的[穿透关系](#hasonethrough-和-hasmanythrough)和[多态多对多](#morphtomany-和-morphedbymany)。

### 软删除会贯穿始终

当被关联的类型实现了 [`SoftDeletes`](eloquent.md#soft-deletes-flag) 时，这个关系包装会继承它的全局作用域。`user.posts().get()` 隐藏被丢弃的 posts 的方式，和 `Post::query().get()` 一样。有三个转发方法能穿透这层限制：

```rust
let alive = user.posts().get().await?;                 // 默认：只有存活的
let all = user.posts().with_trashed().get().await?;    // 存活的 + 被丢弃的
let dead = user.posts().only_trashed().get().await?;   // 只有被丢弃的
```

`with_trashed` / `only_trashed` 存在于 `HasOne`、`HasMany`、`MorphOne`、`MorphMany`、`BelongsToMany`、`MorphToMany`、`MorphedByMany`，以及 `BelongsTo` 上。它们故意没有出现在 `HasOneThrough` 和 `HasManyThrough` 上 - 参见下文的[穿透关系的软删除缺口](#穿透关系的软删除-v1)。

## 一对一：`HasOne` 和 `BelongsTo`

`HasOne` 是父级在说“这个子级有一列指向我”。`BelongsTo` 是子级在说“我有一列指向父级”。两者都只运行一次 `WHERE fk = ? LIMIT 1`，返回 `Option<R>`。

```rust
// HasOne - 父级 → 子级
let profile: Option<Profile> = user.profile().first().await?;

// BelongsTo - 子级 → 父级
let owner: Option<User> = profile.user().first().await?;
```

`BelongsTo` 加了一个别的关系类型不需要的、Laravel 形状的功能：`with_default`。当子级的外键是 null，或者父级那一行已经被删除时，`first()` 会返回这个闭包给出的替身，而不是 `None`：

```rust
#[model(table = "comments", relations = {
    author: BelongsTo<User> {
        with_default = || User { id: 0, name: "Guest".into(), .. },
    },
})]
pub struct Comment { /* ... */ }

// 永远返回 Some(User) - 要么是真正的作者，要么是 Guest 替身。
let display: Option<User> = comment.author().first().await?;
```

预加载的分发器也遵从同样的回退 - 惰性路径和预加载路径共享这个默认行为，所以打印 `comment.author_loaded()[0].name` 的模板代码不需要分支判断。

## 一对多：`HasMany`

`HasMany` 是父级一侧、多基数的关系。终结方法 `.get()` 返回一个 [`Collection<R>`](eloquent.md#collections) - 那个围着 `Vec<R>` 的 Laravel 形状包装 - 所以感知模型的表面能组合起来：

```rust
let titles = user.posts()
    .order_by("created_at", Direction::Desc)
    .limit(10)
    .get()
    .await?
    .pluck::<String>("title");
```

`latest()` 和 `oldest()` 分别是 `order_by("created_at", Direction::Desc)` 和 `Asc` 的糖 - 它们只对声明了 `created_at` 列的模型才能解析，而只要时间戳是开着的（默认就是开着），`#[suprnova::model]` 宏就会自动加上这一列。

## 多对多：`BelongsToMany<R, P>` 与作为一等公民的中间表

`BelongsToMany` 是通过一张联结表实现的多对多。Suprnova 的中间表本身就是一个 `#[suprnova::model]` 结构体，有自己的迁移、自己的访问器、自己的事件。这就是那处分歧 - 参见[下文](#为什么-suprnova-有所不同-中间表是一个真正的模型)。

```rust
#[model(table = "users", relations = {
    roles: BelongsToMany<Role, RoleUser> {
        with_pivot = ["assigned_at"],
        with_timestamps,
    },
})]
pub struct User { /* ... */ }

#[model(table = "role_user", primary_key = "id")]
pub struct RoleUser {
    pub id: i64,
    pub user_id: i64,
    pub role_id: i64,
    pub assigned_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
```

下面这些操作会作用在中间表的行上：

```rust
use suprnova::attrs;

user.roles().attach(role.id).await?;
user.roles().attach_with(role.id, attrs! { assigned_at: now }).await?;
user.roles().detach(role.id).await?;
user.roles().sync([role_a.id, role_b.id, role_c.id]).await?;
```

`sync` 会读取当前的中间表集合，算出 `attach_set = ids - current` 和 `detach_set = current - ids`，在一个事务内部运行这些差量。输入集合里的重复项，会按它们的 JSON 字符串形态折叠起来，所以 `sync([1, 1, 2])` 会做您想要的事。

读取走的是两次查询的策略：

```rust
// 查询 1：通过 INNER JOIN 选出 roles.*、role_user.*，按 user_id 限定范围。
// 查询 2：为同一个联结选出 role_user.*，给每一行盖上 __pivot 戳。
let roles = user.roles().get().await?;

// 每一个 role 都带着宏让它可访问的那份中间表上下文：
for r in &roles {
    let pivot = r.pivot::<RoleUser>();
    println!("{} assigned at {:?}", r.name, pivot.assigned_at);
}
```

### 按中间表的列过滤

`where_pivot` 和它这一家子约束的是*中间*表，不是关联表。当联结行上带着您想拿来过滤的状态时 - 一个 `active` 标志、一个过期时间戳、一个作用域列 - 就该用它们。下面的例子假定上面那个 `RoleUser` 中间表还声明了 `active`、`pinned` 和 `note` 这几个列：

```rust
// 中间表行仍然处于 active 的那些角色。
let active = user.roles().where_pivot("active", 1i64).get().await?;

// 在某个窗口内被指派的角色，或者被显式置顶的角色。
let visible = user
    .roles()
    .where_pivot_between("assigned_at", start..=end)
    .or_where_pivot("pinned", 1i64)
    .get()
    .await?;

// 一个嵌套的分组：(active = 1 AND note IS NOT NULL) OR pinned = 1。
let complex = user
    .roles()
    .where_pivot_group(|q| q.filter("active", 1i64).filter_not_null("note"))
    .or_where_pivot("pinned", 1i64)
    .get()
    .await?;
```

这一整家子：

| 方法 | SQL |
|---|---|
| `where_pivot(col, val)` | `col = ?` |
| `where_pivot_op(col, op, val)` | `col <op> ?` |
| `where_pivot_in(col, vals)` | `col IN (...)` |
| `where_pivot_not_in(col, vals)` | `col NOT IN (...)` |
| `where_pivot_null(col)` | `col IS NULL` |
| `where_pivot_not_null(col)` | `col IS NOT NULL` |
| `where_pivot_between(col, low..=high)` | `col BETWEEN ? AND ?` |
| `where_pivot_not_between(col, low..=high)` | `col NOT BETWEEN ? AND ?` |
| `where_pivot_group(\|q\| ...)` | `(... AND ...)` |

每一个方法都有一个 `or_` 的孪生方法，它会和它前面那一项折叠成一个“或”的关系，就像 `Builder` 上的 `or_where` 那样。一个闭包分组在那个“或”的关系里仍然保持为一个整体，所以 `.where_pivot_null("note").or_where_pivot_group(|q| ...)` 读起来是 `note IS NULL OR (...)`，而不是一条被摊平的链。

列名会作为原始 SQL 标识符插进中间表的语句里，契约和 `Builder::filter` 一样。绝不要从请求数据里取列名。值是作为参数绑定的，所以那些从请求数据里取是安全的。

闭包形式跑在同一条语句上，所以它内部的一个 `where_raw` 或者一个 `where_has`，会逐字落进中间表的 SQL 里 - 标识符允许列表按设计就跳过那道原始 SQL 的脱围机制。请像对待 `Builder::where_raw` 那样对待这个闭包：绝不要用不受信任的输入去拼它的片段。

`MorphToMany` 和 `MorphedByMany` 上有同样的一家子。

### 为什么 Suprnova 有所不同：中间表过滤器是只读的

两条边界，两条都是刻意划下的。

**一个中间表过滤器绝不会收窄一次写入。** Laravel 会把 `wherePivot` 的约束折进 `detach()` 里，所以 `->wherePivot('active', 1)->detach()` 只会删掉那些 active 的联结行。Suprnova 的中间表 `DELETE` 是手写出来的，而一个读谓词到底有没有触及一次删除、却在调用点看不出来，这样的差别是您没法看见的。所以只要设了任何过滤器，`attach`、`attach_with`、`detach` 和 `sync` 都会返回一个错误。请把这两种意图拆开：

```rust
// 先读出匹配的东西，再显式地对它动手。
let stale = user.roles().where_pivot("active", 0i64).get().await?;
for role in &stale {
    user.roles().detach(role.id).await?;
}
```

**预加载不会把中间表过滤器带上。** `user_query.with(["roles"])` 走的是这个关系生成出来的预加载路径，它会一次性为整批父行扫描中间表，并把一个 `with_where` 闭包施加到*关联*表上。那条路径上没有留给中间表谓词的位置。当您需要一次带过滤的多对多读取时，请逐个父行去调用关系访问器（`user.roles().where_pivot(...).get()`），而不是预加载。

### 为什么 Suprnova 有所不同：中间表是一个真正的模型

Laravel 的中间表是一个不透明的逐属性包（`$role->pivot->note`）。Suprnova 要求您声明这个中间表结构体，因为 Rust 的类型系统需要在编译期就知道这些列 - 而一旦您为这份声明付出了代价，这个中间表就会得到和任何其他表一样的 `#[suprnova::model]` 待遇：迁移、事件、观察者、工厂、软删除。`r.pivot::<RoleUser>()` 返回一个类型化的引用；没有字符串键的属性查找，一列名字打错了也不会在运行时给您惊喜。

代价是每张中间表多一个结构体。好处是这个中间表能携带行为 - 领域逻辑、验证规则、审计列 - 而不需要逃逸到原始 SQL 里。
## `HasOneThrough` 和 `HasManyThrough`

两跳关系：`A → B → C`，其中 `B` 是一个中间模型，它的外键指向 `A`；`C` 是最终目标，它的外键指向 `B`。经典例子：`Country` 有多个 `User`；`User` 有多个 `Post`；`Country::posts()` 会在一次 SQL 往返里跳过这两跳。

```rust
#[model(table = "countries", relations = {
    posts: HasManyThrough<User, Post>,
})]
pub struct Country { /* ... */ }

// 单次 INNER JOIN：SELECT posts.* FROM posts
//   INNER JOIN users ON posts.user_id = users.id
//   WHERE users.country_id = ?
let posts: Collection<Post> = country.posts().get().await?;
```

`HasOneThrough` 的形态相同，但 `.get()` 返回的是 `Option<C>`（匹配单基数的语义），`.first()` 是它的别名。

穿透关系的包装只暴露它们的终结方法 - `get` / `first` / `count`，外加这些键设置方法（`first_key` / `second_key` / `local_key` / `second_local_key`）。它们不会流经一个 `Builder<C>`，所以没法链接 `.filter(...)` 或 `.order_by(...)`。如果您需要跨联结过滤，就退回到两次显式的关系跳转。

### 穿透关系的软删除（v1）

穿透关系用的是原始的 `INNER JOIN` SQL，而不是 `Builder<C>` 那条管道，所以 `C::query()` 会安装的那个全局软删除作用域（`WHERE c.deleted_at IS NULL`）**不会**被应用。被丢弃的中间者和被丢弃的目标，都会参与这个 JOIN。

这与 Laravel 不同：当模型声明了 `SoftDeletes` 时，Laravel 的 `hasManyThrough` 会用 `deleted_at IS NULL` 同时过滤 `B` 和 `C`。在这个修复落地之前，需要限定作用域的穿透读取，应该显式链接这两个关系：

```rust
// 别用 country.posts().get()，改成这样：
let users = country.users().get().await?;
let user_ids: Vec<i64> = users.iter().map(|u| u.id).collect();
let posts = Post::query().filter_in("user_id", user_ids).get().await?;
// User 和 Post 的软删除作用域都会生效。
```

## 多态关系

一个多态外键是一对列：`<name>_id`（那一行的主键）加上 `<name>_type`（一个字符串，指明这个 id 活在*哪张表*里）。一行 `Comment` 可以指向一个 `Post` 或者一个 `Video`，而不需要加一列 `post_id` 或者 `video_id`。

Suprnova 提供了四种多态类型：`MorphOne`、`MorphMany`、`MorphTo`，以及多对多这一对 `MorphToMany` / `MorphedByMany`。它们全都共享同一套基础设施：[多态注册表](#多态注册表)。

### `MorphOne<R>` 和 `MorphMany<R>` - 父级一侧

`MorphOne` 和 `MorphMany` 对应着 `HasOne` 和 `HasMany`，但在上面叠加了 `<name>_type` 这个鉴别器。内部的构造器预先用 `WHERE <name>_id = ? AND <name>_type = ?` 过滤好了，所以指向*其他*族的多态子级永远不会出现在结果里。

```rust
#[model(table = "posts", morph_type = "post", relations = {
    comments: MorphMany<Comment> { name = "commentable" },
})]
pub struct Post { /* ... */ }

#[model(table = "videos", morph_type = "video", relations = {
    comments: MorphMany<Comment> { name = "commentable" },
})]
pub struct Video { /* ... */ }

let post_comments = post.comments().get().await?;     // 只有 commentable_type = 'post'
let video_comments = video.comments().get().await?;   // 只有 commentable_type = 'video'
```

`morph_type = "post"` 是父级注册进子级 `commentable_type` 列里的那个字符串。默认值是结构体名的 snake_case 形态，但对任何您要发布的模型来说，覆盖它才是正确的做法 - 改表名的重构不应该弄坏这个多态键。

### `MorphTo` 与按族的枚举

`MorphTo` 活在多态表那一侧。用户要预先声明*目标列表*：

```rust
#[model(table = "comments", relations = {
    commentable: MorphTo { name = "commentable", targets = [Post, Video] },
})]
pub struct Comment {
    pub id: i64,
    pub commentable_id: i64,
    pub commentable_type: String,
    pub body: String,
}
```

宏会在声明的地方，发出一个按族的枚举：

```rust
// 由宏发出 - 您不需要写这个。
pub enum CommentableMorph {
    Post(Post),
    Video(Video),
    Unknown(String, i64),     // 对未注册的 <name>_type 的兜底
}
```

而 `comment.commentable()` 会返回一个取数帮助函数，它的 `.get()` 会解析成这个枚举：

```rust
match comment.commentable().get().await? {
    CommentableMorph::Post(post) => println!("on post: {}", post.title),
    CommentableMorph::Video(video) => println!("on video: {}", video.url),
    CommentableMorph::Unknown(t, id) => {
        eprintln!("orphaned commentable_type={t} id={id}");
    }
}
```

### 为什么 Suprnova 有所不同：按族的枚举

Laravel 的 `morphTo` 返回 `mixed` - PHP 的动态分发会在运行时解析这个方法。Rust 没有后期静态绑定，所以 Suprnova 把这个族变得显式。好处超过了多打这些字的代价：

- **穷举式的 `match`** - 当一个新的多态目标落地、而您忘了处理它时，编译器会告诉您。
- **`Unknown(String, id)` 是类型安全的** - 来自一个已被移除的父模型类的孤儿行，会作为一个变体浮现出来，而不是引发 panic。
- **目标列表记录了这份架构** - 读一读 `MorphTo` 的声明，就能知道另一端可能坐着哪些类型。枚举它们不需要任何数据库查询。

### v1 的限制：`MorphTo` 只支持 `i64`

`MorphTo::morph_id` 被硬编码成了 `i64`。因此多态目标必须使用 `i64` 主键，多态表的 `<name>_id` 列也必须是 `i64`。主键是 `String`、或者经字符串表示的 `Uuid` 的模型，在 v1 里不能当 `MorphTo` 的目标。v2 会把这个多态 ID 类型参数化，接受完整的主键格局（`i64` / `String` / `Uuid`）。

这只是一条针对多态反向端的限制。`MorphOne` / `MorphMany` / `MorphToMany` / `MorphedByMany` 对任何主键形态都能正常工作 - 它们直接读取父级那个已经类型化的 `id`。

### `MorphToMany` 和 `MorphedByMany`

通过单张中间表实现的多态多对多。一侧是“多态的”一方（`Post.tags()`、`Video.tags()` - 两者都经过同一张 `taggables` 中间表）。另一侧是共享的多对多伙伴（`Tag.posts()`、`Tag.videos()` - 同一张中间表，反过来扫描）。

```rust
#[model(table = "tags", relations = {
    posts: MorphedByMany<Post, Taggable> {
        name = "taggable",
        target_morph_type = "post",
    },
    videos: MorphedByMany<Video, Taggable> {
        name = "taggable",
        target_morph_type = "video",
    },
})]
pub struct Tag { /* ... */ }

#[model(table = "posts", morph_type = "post", relations = {
    tags: MorphToMany<Tag, Taggable> { name = "taggable" },
})]
pub struct Post { /* ... */ }

#[model(table = "taggables", primary_key = "id", timestamps = false)]
pub struct Taggable {
    pub id: i64,
    pub tag_id: i64,
    pub taggable_id: i64,
    pub taggable_type: String,
}
```

`MorphToMany` 是变更的那一侧 - `attach` / `attach_with` / `detach` / `sync` 全都活在那里。`MorphedByMany` 是只读的：每一次 `tag.posts()` 调用只返回 `Post` 类型的 taggable，每一次 `tag.videos()` 只返回 `Video` 类型的 taggable，不会在一个集合里混在一起。

从多态的那一侧做变更：

```rust
post.tags().attach(rust_tag.id).await?;
post.tags().sync([rust_tag.id, async_tag.id]).await?;
```

从任一侧读取：

```rust
let tags_on_post: Collection<Tag> = post.tags().get().await?;
let posts_with_rust_tag: Collection<Post> = rust_tag.posts().get().await?;
```

## 多态注册表

每一个标注了 `#[suprnova::model(morph_type = "...")]` 的结构体，都会在编译期通过 `inventory::submit!` 发出一个 [`MorphTypeEntry`][morph]。这个注册表撑起三件事：

1. **按族的枚举分发** - `MorphTo.get()` 会读取子级那一行的 `<name>_type` 字符串，查找它，找到正确的枚举变体。
2. **`MorphedByMany` 的目标过滤** - `target_morph_type = "post"` 会经由这个注册表解析，以确保这个类型字符串是真实存在的。
3. **健全性检查** - 如果没有任何模型用这个字符串注册过，`find_morph_type("post")` 就会返回 `None`，从而把“故意没有注册”和“打错字”区分开来。

```rust
use suprnova::{morph_types, find_morph_type, find_morph_type_by_id};
use std::any::TypeId;

for entry in morph_types() {
    println!("{} -> {}", entry.morph_type, entry.type_name);
}

if let Some(e) = find_morph_type("post") {
    assert_eq!(e.table, "posts");
}

let by_id = find_morph_type_by_id(TypeId::of::<Post>());
```

[morph]: https://docs.rs/suprnova

没有 `morph_type = "..."` 属性的模型，故意不会去注册 - 这个注册表是选择性加入的。一个非多态的 `User` 模型，对它毫无贡献，这正是 `find_morph_type("user")` 返回 `None` 会成为一个有用信号的原因。

## 按关系是否存在来查询

`has` / `where_has` / `doesnt_have` / `where_relation` / `where_belongs_to` 组成了 Suprnova 的关系存在性引擎。它们全都会渲染成针对**父级自身 SELECT** 的关联 `EXISTS (...)` 子查询 - 没有 JOIN，没有重复的父级行，没有 GROUP BY。

```rust
// 至少有一篇 post 的 user。
let with_posts = User::query().has("posts").get().await?;

// 至少有三篇 post 的 user。
let prolific = User::query().has_count("posts", ">=", 3).get().await?;

// 至少有一篇已发布 post 的 user。
let published_authors = User::query()
    .where_has::<Post, _>("posts", |q| q.filter("published", true))
    .get()
    .await?;

// 没有任何 post 的 user。
let empty_users = User::query().doesnt_have("posts").get().await?;

// 没有任何草稿 post 的 user（可能仍然有已发布的）。
let clean = User::query()
    .where_doesnt_have::<Post, _>("posts", |q| q.filter("published", false))
    .get()
    .await?;

// 简写：where_has + 单一列 == 匹配。
let same = User::query()
    .where_relation("posts", "published", true)
    .get()
    .await?;

// where_belongs_to - 直接在这张表上 FK = ?（不需要 EXISTS，
// 因为外键就在子级这一行上）。
let mine = Post::query()
    .where_belongs_to("author", user.id)
    .get()
    .await?;
```

### 工作原理

这个引擎会在构建查询时遍历关系清单。对每一个具名的关系，它会取出 `RelationEntry`，按类型渲染出恰当的 SQL 形态：

- `HasOne` / `HasMany` / `MorphOne` / `MorphMany` → `EXISTS (SELECT 1 FROM child WHERE child.<fk> = parent.<pk>)`。多态类型会加上 `AND child.<name>_type = '<parent_morph_type>'`。
- `BelongsTo` → `EXISTS (SELECT 1 FROM parent WHERE parent.<pk> = child.<fk>)`。
- `BelongsToMany` / `MorphToMany` → 经过中间表联结：`EXISTS (SELECT 1 FROM pivot WHERE pivot.<parent_fk> = parent.<pk> ...)`。
- 穿透关系 → 经过中间者联结。

闭包形态（`where_has::<R, _>(rel, |q| ...)`）会构造一个内部的 `Builder<R>`；这个构造器产出的任何 WHERE 条件，都会落进这个子查询的主体里。占位符编号在整条语句里是单调的，所以这个引擎能配合 `$1` 这种形态的 Postgres 参数正确工作。

`where_belongs_to` 是唯一不渲染 EXISTS 的例外。这个从属关系的外键活在父级*自己*那一行上，所以一个直接的 `WHERE child.<fk> = ?` 正是恰当的 SQL - 不需要子查询。如果这个关系名在父级的清单里是未知的，这个引擎就会发出 `WHERE 1 = 0`，让这次查询安全地什么都不返回。

### 为什么这比 LEFT JOIN 更好

Laravel 更早期的 `has` / `whereHas` 引擎，过去会发出 JOIN，并重复父级的行；关联 EXISTS 的重写在 Laravel 9.落地。Suprnova 从第一天起就提供 EXISTS。好处是：结果集里没有重复项，聚合不需要 GROUP BY 变通方案，不需要 `DISTINCT`，而且数据库的优化器看到的是一个真正的子查询，不是一个它没法把谓词下推过去的 JOIN。对 `has_count(rel, ">=", n)`，这个引擎会直接渲染出 `(SELECT COUNT(*) FROM child WHERE ...) >= n` - 一条查询，一份执行计划。

## 预加载 - `with`、`with_count`、`with_*` 聚合

惰性的 `user.posts().get()` 会为每一个父级做一次查询。当您有很多 user 时，这就是 N+1：

```rust
// 糟糕：1 次查询给 users + 100 次查询给 posts。
let users = User::query().limit(100).get().await?;
for u in &users {
    let posts = u.posts().get().await?;
    /* ... */
}
```

`with(["posts"])` 会把这压缩成总共两次查询 - 无论父级有多少个：

```rust
// 好：1 次查询给 users + 1 次 IN 查询给所有 posts。
let users = User::query()
    .with(["posts"])
    .limit(100)
    .get()
    .await?;

for u in &users {
    for post in u.posts_loaded() {       // 从缓存读取，没有 SQL
        println!("{}: {}", u.name, post.title);
    }
}
```

嵌套路径也能用 - 用点号分隔的关系名会递归：

```rust
let users = User::query()
    .with(["posts.comments.author"])
    .get()
    .await?;
// 4 次查询：users，posts IN users.id，comments IN posts.id，authors IN comments.user_id。
```

### `with_count` 与聚合

`with_count` 会加上一个逐关系的 `COUNT(*) GROUP BY parent_fk` 聚合，和父级一起被加载 - 每个关系多一次查询：

```rust
let users = User::query().with_count(["posts"]).get().await?;
for u in &users {
    println!("{} has {} posts", u.name, u.posts_count());
}
```

四个聚合变体可以叠加：`with_sum`、`with_avg`、`with_min`、`with_max`。缓存键的形态是 `<rel>_<kind>_<col>`，所以在同一个关系上叠加多个聚合不会冲突：

```rust
let users = User::query()
    .with_count(["posts"])
    .with_sum(("posts", "views"))
    .with_avg(("posts", "views"))
    .get()
    .await?;

for u in &users {
    println!(
        "{}: {} posts, {} views total, {} avg",
        u.name,
        u.posts_count(),
        u.posts_sum_of("views").unwrap_or(0.0),
        u.posts_avg_of("views").unwrap_or(0.0),
    );
}
```

完整的存储契约请参见 [Eloquent → 预加载 → 缓存布局](eloquent.md#cache-layout)。

### 带约束的预加载 - `with_where`

`with_where` 会过滤哪些子级行落进预加载缓存，同时不会丢掉那些没有匹配子级的父级：

```rust
use suprnova::Builder;

let users = User::query()
    .with_where(("posts", |q: Builder<Post>| q.filter("published", true)))
    .get()
    .await?;
// 每一个 u.posts_loaded() 只包含已发布的 posts。
// 已发布 posts 数为零的 user 仍然会出现在结果集里 -
// 他们的 posts_loaded() 返回一个空切片。
```

`with_where` 和 `where_has` 的意图不同：`where_has` 过滤的是父级集合（“至少有一篇已发布 post 的 user”）；`with_where` 过滤的是预加载缓存（“对所有 user，只加载他们已发布的 posts”）。当您想要两种效果时，就把两者一起用。

这个判定条件是一个 `Fn`，不是 `FnOnce`，所以携带它的构造器可以被克隆，运行超过一次。想要消费一个捕获值的闭包，应该在内部克隆它：

```rust
let wanted = vec!["rust".to_string(), "web".to_string()];
let users = User::query()
    // 是内部的 `wanted.clone()`，不是 `move` 走 `wanted` 本身 -
    // 这个闭包可能对构造器的每一份克隆都运行一次。
    .with_where(("posts", move |q: Builder<Post>| q.filter_in("tag", wanted.clone())))
    .get()
    .await?;
```

### 克隆一个查询会保留它的预加载计划

`Builder` 是 `Clone` 的，克隆出来的那份会带着这个预加载计划一起走，所以“构建一个基础查询，从它派生出几个”这种模式是可行的：

```rust
let base = User::query().with(["posts"]).filter("active", true);

let first_page = base.clone().limit(20).get().await?;
let total = base.count().await?;
// first_page 里的行，posts_loaded() 都是有值的。
```

### 为什么 Suprnova 有所不同

Laravel 的 `$query->with(...)` 可以随意克隆，因为 PHP 的数组在赋值时会复制。Rust 必须说清楚，对一个类型擦除后的闭包，克隆意味着什么；在 v0.7.2 及之前，Suprnova 给出的答案是丢掉这个计划 - 克隆成功了，查询也成功了，只是关系干脆就不见了。通过一个 `Arc` 共享这个判定条件，能让克隆变得完整，代价就是上面那个 `Fn` 约束。

在 `chunk` / `chunk_by_id` / `lazy` 内部做预加载，仍然是一个醒目的错误，而不是每个分块里悄无声息的 N+1。如果您想要预加载，就在每个分块的闭包内部重新应用一次 `.with(...)`。

### 在已经取到的集合上加载

当您取到一个没有预加载计划的 `Collection<M>` 时，可以事后再给它挂上一个：

```rust
let mut users = User::query().get().await?;

users.load(["posts"]).await?;                 // 无条件
users.load_missing(["posts.comments"]).await?; // 跳过已经加载过的
```

`load_missing` 会遍历每一个父级的 `__eager` 缓存，只对还没加载过这个关系的行触发那条 IN 查询。在这样的循环里很有用：有些父级在这次请求里更早就被预加载过了，有些没有。

### 退出 - `without`

`without` 会把指名的关系从预加载计划里移除，在一个基础作用域加上了您这次调用不想要的默认值时很有用：

```rust
let users = User::query()
    .with(["profile", "posts", "team"])
    .without(["team"])     // 把 team 从这份计划里去掉
    .get()
    .await?;
```

## 触碰所有者

子模型可以声明：写入它时应刷新其所有者的
`updated_at`：

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
    pub body: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
```

只有 `BelongsTo` 关系可以被触碰 - 被触碰的行必须能通过子模型上的列识别，这正是所有者一侧提供的条件。框架通过关系注册表解析所有者，因此触碰只需一次 `UPDATE`，无需 `SELECT`。

不声明时间戳的所有者（`#[model(timestamps = false)]`）、通过 `NULL` 外键到达的所有者，或软删除的所有者，都会被静默跳过。使用 `without_touching`（所有所有者）或 `without_touching_on::<Post, _, _>`（一种类型）来抑制一段工作中的级联。完整语义见
[Eloquent - 父级触碰](eloquent.md#parent-touching)。

## 脱围机制

当一个关系不适合十一种类型中的任何一种时 - 递归树、非 id 键上的多态穿透、三方中间表，任何量身定制的东西 - 就手写这个方法。这个宏不会阻止您这样做；您只是拿不到那个已加载访问器，或者那个关系的预加载分发器分支。

```rust
impl User {
    /// 自定义：不管外键形态如何，取最新的一篇 post。
    pub async fn latest_post(&self) -> Result<Option<Post>, FrameworkError> {
        Post::query()
            .filter("user_id", self.id)
            .latest()
            .first()
            .await
    }
}
```

这个权衡是明确的：手写的方法不会出现在 `relations()` 清单里，存在性引擎不知道它们，预加载器也没法把它们收进一份计划里。对一次性的场景，这没问题。对任何您想要 `with(["..."])` 的东西，请把它声明成一种正规的关系类型，即便您得动用宏的选项才能把它掰成那个形状。

## 下一步

- [Eloquent](eloquent.md) - 日常会用到的模型表面；关系的声明语法就活在那里。
- [数据库](database.md) - 连接、事务、多驱动程序，所有东西都坐落在的那个更底层。
- [迁移](migrations.md) - 这些关系所需要的那些外键列，在架构那一侧的样子。
- [查询构造器](eloquent.md#query-builder-dual-api) - 关系包装转发进去的那个双 API 表面。
- [Eloquent 资源](eloquent-resources.md) - 把已加载的关系变成发给客户端的 JSON:API 载荷。
