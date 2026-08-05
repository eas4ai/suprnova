# 数据对象

Suprnova 的 `#[derive(Data)]` 让您能在**一个结构体**里，同时描述一个入站请求的形状、一个出站响应的形状，以及一份 TypeScript 导出。

## 快速上手

```rust
use suprnova::Data;
use suprnova::data::Field;
use validator::Validate;

#[derive(Data, Validate)]
pub struct UserDto {
    pub id: i64,

    #[validate(email)]
    pub email: String,

    pub name: String,

    #[data(input_only)]
    #[validate(length(min = 8))]
    pub password: String,

    #[data(output_only)]
    pub display_handle: String,

    pub bio: Field<String>,
}
```

`#[derive(Data)]` 会生成：
- `Serialize`（跳过 `#[data(input_only)]` 字段）
- `Deserialize`（拒绝载荷里的 `#[data(output_only)]` 字段，把它们默认置为 `T::default()`）
- 默认带 `authorize: true` 的 `FormRequest` - 处理程序可以直接把这个类型当作一个提取器来用
- `IntoInertiaData`（`Inertia::data(component, dto)` 这条分发路径）
- 为任何 `#[data(allow_include)]` 字段注册一条 `inventory::submit!`

请单独加上 `#[derive(Validate)]`，这样 `#[validate(...)]` 属性才会在字段调用点保持可见。

## 字段属性

| 属性 | 效果 |
|---|---|
| `#[data(input_only)]` | 在 Deserialize 上被接受，在 Serialize 里被省略 |
| `#[data(output_only)]` | 在 Deserialize 上被拒绝（422），在 Serialize 里被包含 |
| `#[data(allow_include)]` | 这个字段有资格被 `?include=` 选中。**默认拒绝**：任何 `foo` 不在允许列表上的 `?include=foo` 请求都返回 400 |
| `#[data(lazy)]` | 这个字段是一个针对请求的 include 集合来解析的 `Prop`；会自动注册为 `allow_include` |
| `#[data(lazy(inertia))]` | 与 `lazy` 相同，为 Inertia 的部分重新加载协议打上标签 |
| `#[data(lazy(deferred))]` | 为 Inertia 的 deferred-props 协议打上标签 |
| `#[data(lazy(closure))]` | 在首次访问时总是被解析；在部分重新加载时是惰性的 |
| `#[data(lazy(when_loaded))]` | 只有当源实体已经预加载了这个关系时才会被解析 |
| `#[data(from_route_param)]` | 字段值来自一次路径捕获（例如 `/users/{id}`）。默认的键 = 字段名；传入 `#[data(from_route_param("id"))]` 可以覆盖它 |

## 结构体属性

| 属性 | 效果 |
|---|---|
| `#[data(auto_lazy)]` | 每一个 `Prop` 类型的字段都隐式地是 `#[data(lazy)]` |
| `#[data(authorize = "path::to::fn")]` | 把生成出来的 `FormRequest::authorize` 路由到一个签名为 `fn(req: &Request) -> bool` 的自由函数。请求体解析器、验证器、Precognition 支持，以及路由参数注入，仍然来自这个 derive |
| `#[data(allow_unknown_fields)]` | 接受不匹配任何结构体字段的载荷键。默认是**严格的**：一个未识别的键会让反序列化失败，报出 `serde::de::Error::unknown_field(..)`，并通过 `FormRequest` 表现为一个 422。只为那些读取向前兼容的第三方载荷的响应 DTO 选择启用宽松模式 |

早先的 `#[data(custom_authorize)]` 标志 - 它会抑制整个 `FormRequest` 实现，并强迫您手工重新实现请求体解析、验证和 Precognition - 已经不存在了。如果您试图使用它，这个宏会报出一个迁移错误。请改用 `#[data(authorize = "fn")]`。

## `Field<T>` - Absent / Null / Value

对于那些“载荷中缺失”必须和“显式的 null”区分开来的 PATCH 端点：

```rust
use suprnova::data::Field;

match dto.bio {
    Field::Absent  => { /* 不要动这一列 */ },
    Field::Null    => { /* 清空这一列 */ },
    Field::Value(text) => { /* 设置为 text */ },
}
```

当在调用点配上 `#[serde(default, skip_serializing_if = "Field::is_absent")]` 时，`Field::Absent`（默认值）能来回还原成“从 JSON 里省略”。没有 `skip_serializing_if` 的话，`Absent` 会序列化成 JSON 的 `null`。

对于三态的数据库 upsert：`dto.bio.into_option_or_null() -> Option<Option<T>>` 把 `Absent` 映射为 `None`，`Null` 映射为 `Some(None)`，`Value(v)` 映射为 `Some(Some(v))`。当下游需要把“不要动”和“设置为 NULL”区分开来时，就用这个。

> **注意事项：** `Field<Option<T>>` 是有损的 - `Value(None)` 和 `Null` 都会序列化成 JSON 的 `null`，并反序列化回 `Null`。对于可空的内部类型，优先选用一个扁平的 `Field<T>`，让 `Null` 来携带“清空它”这个信号。

## `?include=` 查询字符串

`IncludeMiddleware` 会把请求的查询字符串解析成一个逐请求的 `RequestIncludeSet`：

- `?include=foo,bar` - 解析惰性字段 `foo` 和 `bar`。
- `?include[]=foo&include[]=bar` - 数组形式，结果相同。
- `?exclude=`、`?only=`、`?except=` - 与 Laravel-Data 的 API 对等。

与 `X-Inertia-Partial-Data`（Inertia 的部分重新加载请求头）的组合方式：对于打了归属标签的惰性字段，include 集合 + 逐 DTO 的允许列表会**先**运行，所以即使部分数据本来会把一个不被允许的字段过滤掉，对它的请求仍然会返回 400。部分数据是**之后**才被应用的，作为对已解析出来的 props 的最后一道“only”过滤。

全局地注册 `IncludeMiddleware` - 通常放在中间件栈里会话和授权之间：

```text
SessionMiddleware → IncludeMiddleware → AuthMiddleware → 处理程序
```

### 编程式的 include/exclude/only/except

`RequestIncludeSet` 用可链式调用的构建器方法，镜照了 Laravel-Data 的 `IncludeableData` 契约。处理程序、测试和中间件都可以构造或覆盖一个集合，而不需要直接去碰公开字段：

```rust
use suprnova::data::RequestIncludeSet;

let set = RequestIncludeSet::default()
    .include(["author", "comments"])
    .exclude(["password"])
    .only(["id", "name"])
    .except(["secret"]);

assert!(set.is_visible("name"));   // 在 `only` 上，不在 `except` 里
assert!(!set.is_visible("secret"));// `except` 总是获胜
assert!(set.includes("author"));   // 对 `author` 这个关系的请求
```

| 方法 | 效果 | Laravel 对应物 |
|---|---|---|
| `.include(fields)` | 追加到 include 列表（要解析的惰性字段） | `Data::include(...$fields)` |
| `.exclude(fields)` | 追加到 exclude 列表（要丢弃的字段） | `Data::exclude(...$fields)` |
| `.only(fields)` | 初始化或扩展 `only` 允许列表 | `Data::only(...$fields)` |
| `.except(fields)` | 追加到 except 列表（永远丢弃） | `Data::except(...$fields)` |
| `.include_when(cond, fields)` | 只在 `cond == true` 时追加 | `Data::includeWhen($field, $condition)` |
| `.exclude_when(cond, fields)` | 只在 `cond == true` 时追加 | `Data::excludeWhen($field, $condition)` |
| `.only_when(cond, fields)` | 只在 `cond == true` 时扩展 `only` | `Data::onlyWhen($field, $condition)` |
| `.except_when(cond, fields)` | 只在 `cond == true` 时追加 | `Data::exceptWhen($field, $condition)` |
| `.merge(other)` | 合并两个集合（就地分层覆盖） | PHP 里手工的 `array_merge` |
| `.includes(field)` | `field`（或 `field.path`）在 include 列表里吗？ | `relationLoaded()` 的对应物 |
| `.is_excluded(field)` | `field` 在 exclude 列表里吗？ | 读取 exclude 分区 |
| `.is_excepted(field)` | `field` 在 except 列表里吗？ | 读取 except 分区 |
| `.is_only_listed(field)` | `field` 被 `only` 允许吗（或者 `only` 未设置）？ | 读取 only 分区 |
| `.is_visible(field)` | 完整的 Laravel 决议顺序：except → exclude → only | `resolveResource` 的决策 |

这些构建器方法接受任何 `IntoIterator<Item = impl Into<String>>`，所以数组、vec，以及 `&str`/`String` 的切片全都能用。字符串会被去除首尾空白；空条目会被丢弃（与 `from_query` 保持一致）。

任何列表里的点号路径，在被裸名字探测时都会匹配它的根段 - `include=["author.posts"]` 会报出 `set.includes("author") == true`，与 Laravel-Data 的路径解析方式一致。嵌套的 `posts` 段，会被 `IncludeTree::from_include_set` 为 JSON:API 复合文档消费掉。

### 处理程序端的覆盖：`with_include_overrides`

要在请求的查询字符串已经声明的东西之上，再叠加一层编程式的覆盖（同时不丢失请求自己的集合），请使用 `with_include_overrides`：

```rust
use suprnova::data::with_include_overrides;

async fn show_album(req: Request, user: User) -> Response {
    with_include_overrides(
        |set| set
            .include_when(user.is_admin(), ["audit_log"])
            .exclude_when(!user.is_admin(), ["price_cost"]),
        async move {
            // 在这个作用域内部，惰性 prop 解析器和 JSON:API
            // include 解析器看到的都是合并后的集合。
            Inertia::data("Album/Show", album_dto).into_response()
        },
    ).await
}
```

这个闭包运行时针对的是当前已绑定集合的一个克隆（如果没有中间件绑定过，就是那个空的默认值）。在这个 future 完成之后，原来的集合会被恢复 - 这是一次作用域内的覆盖，不是一次变更。

对于测试，请优先选用 `scope_include_set(set, future)`，来安装一个全新的集合，而不继承任何环境状态。

## 泛型结构体

```rust
use serde::{Serialize, Deserialize};

#[derive(suprnova::Data)]
pub struct Paginated<T>
where
    T: Serialize + for<'de> Deserialize<'de>,
{
    pub items: Vec<T>,
    pub total: usize,

    #[data(allow_include)]
    pub meta: Option<serde_json::Value>,
}
```

这个 TypeScript 提取器会生成 `export interface Paginated<T>`，这样前端代码就能在不同的实例化之间复用这个泛型。

`?include=` 的允许列表是按完全限定的类型路径（`concat!(module_path!(), "::", stringify!(Paginated))`）来加键的，不是按类型参数的实例化来加键的。在同一个模块里声明的 `Paginated<UserDto>` 和 `Paginated<ArticleDto>` 共享一份允许列表 - `allow_include` 命名的是一个字段，而字段名不依赖于类型参数。两个分别在不同模块里、都叫 `Paginated` 的不同 DTO，各自会得到自己的允许列表；它们的键不会冲突。

注意：对泛型结构体，`FormRequest` 会被抑制，因为它的 trait 约束（`DeserializeOwned + Validate + Send`）在不知道具体的类型参数的情况下无法被验证。如果您需要从一个请求里提取一个泛型的 Data 结构体，请提供您自己的实现。

## 路由参数字段注入

```rust
use suprnova::Data;
use validator::Validate;

#[derive(Data, Validate)]
pub struct UpdateUser {
    #[data(from_route_param("id"))]
    pub id: i64,

    #[validate(length(min = 1))]
    pub name: String,
}
```

对于带着请求体 `{"name": "Ada"}` 的 `PATCH /users/{id}`，路由捕获到的那个 `id` 会被合并进已验证的载荷。**路径永远胜过请求体提供的值**（防止通过篡改请求体来实现 IDOR）。

裸的 `#[data(from_route_param)]` 默认使用字段名。这个宏在编译期对字段类型路径的最后一段进行分类，并分发到一个匹配的解析器上。只有下面列出的这些精确名字才会被识别；其他的一切（包括 `i8`/`i16`/`isize`、`Uuid`、`DateTime`、自定义的 newtype）都会落到 `pass_string` 上，交给字段自己的 `Deserialize` 去处理。

| 字段类型 | 解析器 |
|---|---|
| `i64` | `parse_i64` |
| `u64` | `parse_u64` |
| `i32` | `parse_i32` |
| `u32` | `parse_u32` |
| `i128` | `parse_i128`（先验证，然后把原始字符串传下去；由字段的 `Deserialize` 解析） |
| `u128` | `parse_u128`（同样的字符串传递模式） |
| `f64` | `parse_f64`（拒绝非有限的值） |
| `f32` | `parse_f32`（拒绝非有限的值） |
| `bool` | `parse_bool`（只接受 `"true"` / `"false"`） |
| 其他任何东西 | `pass_string` - 把原始字符串交给字段自己的 `Deserialize` |
| 上述任意一种的 `Option<T>` 或 `Field<T>` | 与 `T` 相同的解析器；缺失的路由参数会让这个字段处于缺失状态 |

## Lazy props

```rust
use suprnova::Data;
use suprnova::inertia::Prop;

#[derive(Data)]
#[data(auto_lazy)]
pub struct AlbumDto {
    pub id: i64,
    pub songs: Prop,    // 自动注册为 ?include=songs
    pub artist: Prop,   // 自动注册为 ?include=artist
}
```

逐字段显式指定风味：

```rust
#[derive(Data)]
pub struct AlbumDto {
    pub id: i64,

    #[data(lazy(inertia))]
    pub songs: Prop,

    #[data(lazy(deferred))]
    pub lyrics: Prop,

    #[data(lazy(closure))]
    pub artist: Prop,
}
```

使用 `Inertia::data(component, dto)` 来渲染 - 这个 derive 会生成一个查询 include 集合和允许列表的 `IntoInertiaData` 实现：

```rust
return Inertia::data("Album/Show", album_dto);
```

注意：带惰性字段的结构体会抑制 `Serialize`、`Deserialize` 和 `FormRequest`，因为 `Prop` 没有实现它们。如果单个端点同时需要入站解析和惰性的出站，请使用两个 DTO：一个入站的（朴素的 `#[derive(Data, Validate)]`），一个出站的（带惰性字段的 `#[derive(Data)]`）。

## `when_loaded!` - 基于关系是否已加载的条件式惰性

镜照了 Laravel-Data 的 `#[AutoWhenLoadedLazy]`。用户的 `From<Entity>` 实现决定这个关系是否已被预加载：

```rust
use suprnova::data::{when_loaded, IsRelationLoaded};

impl From<&AlbumEntity> for AlbumDto {
    fn from(album: &AlbumEntity) -> Self {
        Self {
            id: album.id,
            songs: when_loaded!(album, "songs", || async {
                serde_json::json!(album.songs_relation()
                    .iter()
                    .map(SongDto::from)
                    .collect::<Vec<_>>())
            }),
            artist: Prop::eager(serde_json::json!(album.artist_name())),
            lyrics: Prop::lazy(|| async { /* ... */ }),
        }
    }
}
```

如果这个实体没有预加载这个具名的关系（根据 `IsRelationLoaded::is_relation_loaded`），`when_loaded!` 就会返回 `Prop::EagerNone`，这个字段就会在响应里缺失。

SeaORM 实体需要一个查询它们自己已加载关系状态的自定义 `IsRelationLoaded` 实现 - 框架不提供一个统一的兜底实现，因为 SeaORM 的 `ModelTrait` 不携带逐实例的关系已加载状态（已加载的关系活在查询结果上，不在模型结构体本身上）。

## TypeScript 导出

`suprnova generate-types` 会为每一个 `#[derive(Data)]`（以及遗留的 `#[derive(InertiaProps)]`）结构体生成 TypeScript 定义。行为如下：

- `Field<T>` → `field?: T | null`
- `Prop` → `field?: T`（惰性的“可能缺失”语义；`?` 携带了这一层含义，类型本身是朴素的）
- `#[data(input_only)]` → 从输出类型里排除
- `#[data(output_only)]` → 从输入类型里排除
- 泛型结构体 → TypeScript 泛型接口（`export interface Paginated<T>`）
- 当**任何**字段带有 `input_only` / `output_only` / `lazy` 时，会生成两个接口：`<Name>`（输出）和 `<Name>Input`（输入）

生成出来的类型永远不会泄漏仅供 Rust 使用的类型（`Prop<...>` 不会出现在输出的 `.d.ts` 里）。

## 脚手架

```bash
suprnova make:inertia UserDto --data
```

生成一个 `#[derive(Data, Validate)]` 骨架，而不是遗留的 `#[derive(InertiaProps)]` 模板。

## 下一步

- [验证](validation.md) - `#[derive(Validate)]`、异步验证器，以及 `FormRequest` 如何调用它们
- [请求](requests.md) - `FormRequest` 所接入的那个请求提取器接口
- [Inertia 响应](frontend-inertia-responses.md) - `Inertia::data` 这条路径，以及惰性 props 如何变得有资格参与部分重新加载
- [JSON:API 资源](eloquent-resources.md) - 用于 JSON:API 输出的 `#[derive(Resource)]`（在仅用于序列化的载荷场景中，是 `Data` 的姊妹机制）
- [错误模型](error-model.md) - `unknown_field` 的拒绝如何变成一个 422，以及 `FormRequest` 的失败如何以 `ValidationErrors` 的形式传回来
