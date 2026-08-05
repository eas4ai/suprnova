# JSON:API 资源

Suprnova 为类型化的 REST API 提供了一个 JSON:API 资源层。给一个 `#[derive(Data)]` 结构体标注上 `#[json_resource("type")]`，框架就会发出一个 `IntoJsonResource` 实现，通过同一条代码路径处理单个信封、集合、分页集合、稀疏字段集（`?fields[type]=...`）、复合的 `included` 文档，以及多级的 `?include=a.b.c` 链。两个门面 - `Resource` 和 `JsonApi` - 是同一个类型的两个名字；哪种更符合您的代码风格，就用哪种。

## 定义一个资源

```rust
use suprnova::Data;

#[derive(Debug, Clone, Data)]
#[json_resource("users")]
pub struct UserResource {
    pub id: i64,
    pub email: String,

    // `input_only` 让 `password` 在表单请求一侧仍然可用，
    // 但会把它从 API 输出里去掉。
    #[data(input_only)]
    pub password: String,

    // 把一个字段标记为*关系*：它永远不会落进 `attributes`，
    // 而是产出一个 JSON:API 关系对象，并且有资格参与
    // `?include=`。这个字段的类型必须实现 `IntoJsonResource`
    // （直接实现，或者通过 `Vec<T>` / `Option<T>`）。
    #[data(allow_include)]
    pub posts: Vec<PostResource>,
}
```

`id_field` 关键字用来改名提供 JSON:API `id` 的那个字段：

```rust
#[derive(Data)]
#[json_resource("orders", id_field = "uuid")]
pub struct OrderResource {
    pub uuid: String,
    pub total_cents: i64,
}
```

## 渲染响应

从一个处理程序里构造一个待定的响应，再调用 `.render().await`：

```rust
use suprnova::{LengthAwarePaginator, Resource};

#[handler]
async fn show_user(id: i64) -> Result<HttpResponse, FrameworkError> {
    let user: UserResource = User::find_or_fail(id).await?.into();
    Resource::single(user).render().await
}

#[handler]
async fn list_users() -> Result<HttpResponse, FrameworkError> {
    let users: Vec<UserResource> = User::all().await?.into_iter().map(Into::into).collect();
    Resource::collection(users).render().await
}

#[handler]
async fn paginate_users() -> Result<HttpResponse, FrameworkError> {
    // `paginate(per_page)` 会自动从当前请求读取 `?page=`。
    let page = User::query().paginate(10).await?;
    // 把模型分页器逐字段转换成资源分页器 -
    // `data` 是 `pub` 的，其余的计数/链接原样带过来。
    let page = LengthAwarePaginator::new(
        page.data.into_iter().map(UserResource::from).collect(),
        page.total,
        page.per_page,
        page.current_page,
    )
    .with_base_url("/api/users");
    Resource::paginated(page).render().await
}
```

如果您更喜欢 Laravel 的写法，`JsonApi::single` / `JsonApi::collection` / `JsonApi::paginated` 是完全相同的别名入口。

## 可链式的修改方法

`JsonApiResponse` 是一个待定对象。在调用 `.render().await` 之前，先定制这个信封。每一个修改方法都是 `self` → `Self`，所以它们可以组合起来：

```rust
use suprnova::{Resource, JsonApiInfo};
use serde_json::json;

let info = JsonApiInfo::new()
    .with_version("1.1")
    .with_ext("https://jsonapi.org/ext/atomic")
    .with_meta("copyright", json!("2026 Acme Inc."));

Resource::single(user)
    .status(201)                                  // 覆盖 HTTP 状态码
    .with_meta("trace_id", json!("req-7"))        // 顶层的 meta 键值对
    .with_link("self", "/api/users/1")            // 顶层链接
    .with_jsonapi(info)                           // 顶层的 `jsonapi`
    .additional(json!({ "api_version": "2.0" }).as_object().unwrap().clone())
    .render()
    .await
```

| 修改方法 | Laravel 对应物 | 效果 |
|---|---|---|
| `.status(code)` | `ResourceResponse::calculateStatus` | 覆盖 HTTP 状态码。 |
| `.created()` | `wasRecentlyCreated → 201` | `.status(201)` 的简写。 |
| `.with_meta(k, v)` / `.meta(k, v)` | `with($request)` | 顶层的 `meta` 键值对。 |
| `.with_meta_map(m)` | 批量的 `with($request)` | 把一个映射合并进顶层的 `meta`。 |
| `.with_link(rel, href)` / `.link(rel, href)` | `with($request)['links']` | 顶层的 `links` 键值对。 |
| `.with_link_value(rel, v)` | 链接对象形态 | 顶层链接写成 `{href, meta}`。 |
| `.with_additional(k, v)` | `additional($data)` | 与 `data` 并列的根级键。 |
| `.additional(map)` | `additional($data)` | 批量的附加键。 |
| `.with_jsonapi(info)` | `JsonApiResource::configure(...)` | 顶层的 `jsonapi` 成员。 |

规范成员（`data`、`included`、`links`、`meta`、`jsonapi`、`errors`）永远不会被 `.additional(...)` 覆盖。

## 逐资源的 `links` 和 `meta`

覆盖 `IntoJsonResource::resource_links` 和 `IntoJsonResource::resource_meta` 的默认实现，把链接/元数据挂到*资源对象*上，而不是文档的根上：

```rust
use suprnova::resources::IntoJsonResource;
use serde_json::{Map, Value};

impl IntoJsonResource for MyHandRolledPost {
    // ...

    fn resource_links(&self) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("self".into(), Value::String(format!("/api/posts/{}", self.id)));
        m
    }

    fn resource_meta(&self) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("kind".into(), Value::String("blog".into()));
        m
    }
}
```

对宏派生出来的资源来说，两者都默认是一个空的 `Map`，所以 JSON:API 渲染器在没用到这两个键时就会省略它们。覆盖 `resource_top_level_meta`，可以把逐资源的元数据提升到信封顶层的 `meta` 成员里。

## 条件属性 - `Maybe<T>` / `MissingValue<T>`

用 `Maybe`，可以根据一个运行时条件，把一个字段从渲染出来的 `attributes` 对象里省去。这是 Suprnova 对 Laravel 的 `MissingValue` 以及 `when()` / `whenLoaded()` / `unless()` 这一族的对应物。

```rust
use suprnova::{Maybe, MissingValue};

// 两个名字指向同一个类型。
let m1: Maybe<&str> = Maybe::present("email@example.com");
let m2: MissingValue<&str> = MissingValue::missing();
let m3 = Maybe::when(user.is_verified, &user.verified_at);
let m4 = Maybe::unless(user.is_admin, &user.public_handle);
let m5 = Maybe::when_with(expensive_check(), || compute_value()); // 惰性
```

对宏派生出来的结构体，把一个字段声明成 `Maybe<T>`，渲染器就会在它是 `Missing` 时自动丢弃它。对手写的 `resource_attributes`，请用 `insert_maybe(map, key, maybe)` 这个帮助函数：

```rust
use suprnova::resources::{insert_maybe, Maybe};

fn resource_attributes(&self, _fs: Option<&[&str]>) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    insert_maybe(&mut map, "email", Maybe::present(&self.email));
    insert_maybe(
        &mut map,
        "phone",
        if self.show_phone { Maybe::present(&self.phone) } else { Maybe::missing() },
    );
    serde_json::Value::Object(map)
}
```

渲染器还会对整个属性对象调用 `strip_missing_values(&mut value)`，所以嵌套在任意 serde 派生结构深处的 `Maybe::Missing` 值都会被递归丢弃 - 当一个深层嵌套的转换器想省去某些子字段时，这很有用。

## 稀疏字段集

框架的 `IncludeMiddleware` 会解析 `?fields[type]=email,name` 这样形态的查询参数，并把它们绑定到一个任务本地。宏生成的 `resource_attributes` 会查阅这个字段集，只发出被请求的属性。不需要任何处理程序端的工作 - 装上这个中间件，资源层就会自动遵从它。

```rust
// 请求：GET /api/users/7?fields[users]=email
// 响应：{ "data": { "type": "users", "id": "7", "attributes": { "email": "alice@example.com" } } }
```

## 复合文档 - `?include=` 链

用 `#[data(allow_include)]` 声明关系字段。框架会从 `?include=author.posts.tags,comments` 构建出一棵 `IncludeTree`，走遍每一个节点，把彻底解析好的资源对象推进 `included` 里。去重是在推入时通过 `IncludedSink` 完成的，键是 `(type, id)`，遵循 JSON:API 规范第 8 节 - 所以一个 1,000 项的集合，即便每一项都共享同一个作者，这个作者也只会被解析一次。峰值内存和 CPU 只与不同的被包含资源数量成正比，与关系的扇入无关。

```rust
#[derive(Data)]
#[json_resource("posts")]
pub struct PostResource {
    pub id: i64,
    pub title: String,

    #[data(allow_include)]
    pub author: Option<AuthorResource>,

    #[data(allow_include)]
    pub tags: Vec<TagResource>,
}
```

一个指名了某个不在该资源允许列表上的 include 路径的请求，会得到一个 JSON:API 400 的错误信封。

### 为什么 Suprnova 有所不同

与 Laravel 的 `JsonApiResource` 相比，有两处可见的分歧：

1. **对 `?include=` 采用严格的默认拒绝。** Laravel 的资源层会静默忽略解析不出来的 include 路径。Suprnova 会用一个带着 JSON:API 错误信封的 `400 Bad Request` 拒绝它们。规范第 5.2.2 节的默认拒绝立场，正是客户端可以据以编程的契约；静默忽略会掩盖客户端的 bug，并破坏复合文档的完整性。

2. **显式的 `.status(code)` / `.created()`，而不是自动 201。** Laravel 会从底层 Eloquent 模型的 `wasRecentlyCreated` 自动设成 `201`。Suprnova 把资源 DTO 与任何具体的持久化生命周期解耦，所以状态码是设在响应对象本身上的 - 想表达这一点就用 `.created()`，响应为空就用 `.status(204)`，依此类推。在任何流程下，单一的修改方法都能保持诚实。

## 分页

`Resource::paginated(p)` 能配合任何实现了 `Paginated<T>` trait 的分页器工作 - `suprnova::pagination` 里的 `LengthAwarePaginator<T>` 和 `CursorPaginator<T>` 都实现了这个 trait。渲染器会自动附上 `links.{self,first,prev,next,last}` 和一个 `meta.pagination` 块。

```rust
use suprnova::{LengthAwarePaginator, Resource};

let page = LengthAwarePaginator::new(items, total, per_page, current_page)
    .with_base_url("/api/users");
Resource::paginated(page).render().await
```

## 错误信封

每一个 `FrameworkError` 都知道如何通过 `into_json_api_response()` 把自己渲染成一个 JSON:API 的 `{"errors": [...]}` 信封。这个帮助函数之所以被公开出来，是因为 `FrameworkError` 携带着一个状态码、一个字段名来源指针（用于 `ValidationError`），以及一个放在 `meta.request_id` 下的请求 id 关联令牌。5xx 响应会被清理：原始消息永远不会抵达客户端，除非当前环境设置了 `APP_DEBUG=true`，此时它会出现在 `meta.debug_message` 下。

```rust
let response = FrameworkError::validation("email", "email is invalid")
    .into_json_api_response();
// {
//   "errors": [{
//     "status": "422",
//     "title": "Validation failed",
//     "detail": "email is invalid",
//     "source": { "pointer": "/data/attributes/email" },
//     "meta": { "request_id": "..." }
//   }]
// }
```

## 表面小结

| Suprnova 表面 | Laravel 13 对应物 |
|---|---|
| `Resource` / `JsonApi` 门面 | `JsonResource::make`、`JsonApiResource` |
| `JsonApiResponse` | `ResourceResponse`、`JsonApiResource::toResponse` |
| `JsonApiBuilder` | （`ResourceResponse` 的内部构造器） |
| `IntoJsonResource` trait | `JsonResource::toArray`、`toAttributes`、`toRelationships`、`toLinks`、`toMeta`、`with` |
| `RelationshipValue` / `ResourceIdentifier` | `toRelationships` 内部的数组形态 |
| `IncludeTree` | 从 `JsonApiRequest` 解析出的 `?include=` |
| `RequestFieldsetSet` | 从 `JsonApiRequest` 解析出的 `?fields[type]=` |
| `Maybe<T>` / `MissingValue<T>` | `MissingValue` + `whenLoaded` / `when` / `unless` |
| `JsonApiInfo` | `JsonApiResource::$jsonApiInformation` |
| `JsonApiResponse::status(code)` / `.created()` | `ResourceResponse::calculateStatus` |
| `JsonApiResponse::additional(map)` / `.with_additional(k, v)` | `JsonResource::additional($data)` |
| `JsonApiResponse::with_meta(k, v)` / `.meta(k, v)` | `JsonResource::with($request)['meta']` |
| `JsonApiResponse::with_link(rel, href)` / `.link(rel, href)` | `JsonResource::with($request)['links']` |
| `JsonApiResponse::with_jsonapi(info)` | `JsonApiResource::configure(...)` |
| `current_fieldset()` / `scope_fieldset(...)` | 由 `IncludeMiddleware` 设置的任务本地字段集 |
| `IncludeResolutionError` → 400 信封 | 严格模式下的 `?include=` 解析器 |

`suprnova::` 之下的顶层重导出：`Resource`、`JsonApi`、`JsonApiResponse`、`JsonApiBuilder`、`JsonApiInfo`、`IncludedSink`、`IntoJsonResource`、`RelationshipValue`、`ResourceIdentifier`、`IncludeTree`、`RequestFieldsetSet`、`Maybe`、`MissingValue`、`insert_maybe`、`strip_missing_values`、`AsRelationshipValue`、`PushIncluded`、`IncludeResolutionError`、`current_fieldset`、`scope_fieldset`。

## 下一步

- [Eloquent 序列化](eloquent-serialization.md) - `#[derive(Data)]`、隐藏/可见字段，以及喂给资源属性的那个 `toArray` 对应物
- [Eloquent 关系](eloquent-relationships.md) - `#[data(allow_include)]` 消费的是什么；支撑复合文档的那些类型化关系
- [分页](pagination.md) - `LengthAwarePaginator`、`CursorPaginator`，以及 `Resource::paginated` 所消费的 `Paginated<T>` trait
- [数据对象](data.md) - 与 Inertia 共享的 `#[derive(Data)]` 宏、`?include=`/`?fields[type]=` 中间件，以及 `Maybe<T>` 这类模式
- [错误模型](error-model.md) - `FrameworkError::into_json_api_response` 如何契合这套转换契约
