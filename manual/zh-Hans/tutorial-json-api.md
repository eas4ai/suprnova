# 构建待办事项 JSON:API

一次端到端的 API 路径演练：迁移、模型、经过验证的表单请求、路由模型绑定、JSON:API 资源信封、稀疏字段集、分页。到最后，您会拥有一个有五个端点的待办事项服务，它会产出符合规范的 [JSON:API](https://jsonapi.org/) 响应，`?include=` 和 `?fields[todos]=...` 都会被自动处理。

您将构建的东西：

| 方法     | 路由                  | 操作     |
|----------|----------------------|---------|
| `GET`    | `/api/todos`         | 列表（分页） |
| `GET`    | `/api/todos/{todo}`  | 详情 |
| `POST`   | `/api/todos`         | 创建 |
| `PUT`    | `/api/todos/{todo}`  | 更新 |
| `DELETE` | `/api/todos/{todo}`  | 删除 |

## 前提条件

一个已脚手架好的项目：

```bash
suprnova new todo-api
cd todo-api
```

## 步骤 1：迁移

```bash
suprnova make:migration create_todos_table
```

这会写出 `src/migrations/m<timestamp>_create_todos_table.rs`。把方法体替换成 `todos` 的架构：

```rust
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Todos::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Todos::Id)
                            .big_integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Todos::Title).string().not_null())
                    .col(ColumnDef::new(Todos::Description).text().null())
                    .col(
                        ColumnDef::new(Todos::Done)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .col(
                        ColumnDef::new(Todos::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .col(
                        ColumnDef::new(Todos::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Todos::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum Todos {
    Table,
    Id,
    Title,
    Description,
    Done,
    CreatedAt,
    UpdatedAt,
}
```

运行它：

```bash
suprnova migrate
```

这个 `down` 方法体让 `migrate:rollback` 之后能撤销这个变更。

## 步骤 2：模型

一个 `#[suprnova::model]` 结构体*就是*这个 Eloquent 模型 - 这个宏会在一个内部模块里发出 SeaORM 的 `Entity`、`Column` 和 `ActiveModel`，并给这个结构体装上查询接口（`Todo::query()`、`Todo::find`、`Todo::create`、`model.update`、`model.delete`，自动管理的时间戳，生命周期事件）。创建 `src/models/todo.rs`：

```rust
use chrono::{DateTime, Utc};
use suprnova::model;

#[model(
    table = "todos",
    fillable = ["title", "description", "done"],
    timestamps,
)]
pub struct Todo {
    pub id: i64,
    pub title: String,
    pub description: Option<String>,
    pub done: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// 重新导出这个宏在内部的 `todo` 模块里发出的那些 SeaORM 类型，
// 这样调用处就能直接拿到它们，不用去碰这个宏内部的东西。
pub use todo::{ActiveModel, Column, Entity};
```

把这个模块接到 `src/models/mod.rs` 里：

```rust
pub mod todo;
```

这个 `fillable` 列表就是批量赋值的允许列表 - 只有列在其中的字段，才能通过 `Todo::create(attrs!{...})` 和 `model.update(attrs!{...})` 来设置。列表之外的字段，会被防护起来，不会被请求输入意外写入。

## 步骤 3：表单请求

验证活在一个 `#[request]` 结构体上。`extract()` 会在处理程序方法体看到这个值之前运行验证器；一次失败会短路到一个带着 Laravel/Inertia 错误包的 422。创建 `src/requests.rs`：

```rust
use suprnova::request;

#[request]
pub struct CreateTodoRequest {
    #[validate(length(min = 1, max = 255, message = "title is required"))]
    pub title: String,

    #[validate(length(max = 1000))]
    pub description: Option<String>,
}

#[request]
pub struct UpdateTodoRequest {
    #[validate(length(min = 1, max = 255))]
    pub title: Option<String>,

    #[validate(length(max = 1000))]
    pub description: Option<String>,

    pub done: Option<bool>,
}
```

并在 `src/lib.rs` 里注册它：

```rust
pub mod requests;
```

这个 `#[request]` 属性会展开成相当于 `#[derive(serde::Deserialize, validator::Validate)] + impl FormRequest` 的东西，所以这个结构体的字段同时也是输入架构。可选字段（`Option<T>`）对局部更新来说是正确的形态：JSON 请求体里缺失的一个键，会反序列化成 `None`，而处理程序会把 `None` 当作“不要改动这一列”。

## 步骤 4：JSON:API 资源

一个资源就是一个带着 `#[json_resource("type")]` 的 `#[derive(Data)]` 结构体。这个宏会发出 `Resource::single`、`Resource::collection` 和 `Resource::paginated` 会消费的那个 `IntoJsonResource` 实现。这个资源的字段会变成 JSON:API 的 `attributes` 对象 - 每一次稀疏字段集过滤和 `?include=` 链都会通过这个类型来分发。创建 `src/resources/todo_resource.rs`：

```rust
use crate::models::todo::Todo;
use suprnova::Data;
use validator::Validate;

#[derive(Debug, Clone, Data, Validate)]
#[json_resource("todos")]
pub struct TodoResource {
    pub id: i64,
    pub title: String,
    pub description: Option<String>,
    pub done: bool,
    pub created_at: String,
    pub updated_at: String,
}

impl From<Todo> for TodoResource {
    fn from(t: Todo) -> Self {
        Self {
            id: t.id,
            title: t.title,
            description: t.description,
            done: t.done,
            created_at: t.created_at.to_rfc3339(),
            updated_at: t.updated_at.to_rfc3339(),
        }
    }
}
```

把它接到 `src/resources/mod.rs` 里：

```rust
pub mod todo_resource;
```

并在 `src/lib.rs` 里重新声明这个模块：

```rust
pub mod resources;
```

这个 `id` 字段提供了 JSON:API 的 `id` 成员（按规范转成字符串）；其它每一个字段都落在 `attributes` 里，并受稀疏字段集过滤的约束 - 一个指名 `?fields[todos]=title,done` 的请求，只会拿回这两个属性，不需要任何处理程序端的工作。

## 步骤 5：控制器

这个 `#[handler]` 属性会给每个参数分类，并生成与之匹配的提取器：

- `i64` - `FromParam` 会解析同名的那个路由参数。错误的输入（`/api/todos/abc`）会短路到 400。
- `CreateTodoRequest` / `UpdateTodoRequest` - `FromRequest` 会反序列化请求体、运行验证，并在失败时给出 422。
- `Request` - 原样传递过去。

加载这一行要经过 Eloquent 接口：当没有行匹配时，`Todo::find_or_fail(id)` 会返回一个 404。

创建 `src/controllers/todos.rs`：

```rust
use crate::models::todo::Todo;
use crate::requests::{CreateTodoRequest, UpdateTodoRequest};
use crate::resources::todo_resource::TodoResource;
use suprnova::{
    attrs, handler, LengthAwarePaginator, Model, Resource, Response,
};

// GET /api/todos?page=2
#[handler]
pub async fn index() -> Response {
    let page = Todo::query()
        .order_by_desc("created_at")
        .paginate(20)
        .await?;
    // 把这个分页器重新包装到 `TodoResource` 周围，这样这个 JSON:API
    // 渲染器看到的就是资源对象，而不是裸模型。分页窗口
    // （`total`、`per_page`、`current_page`）被保留了下来。
    let total = page.total;
    let per_page = page.per_page;
    let current_page = page.current_page;
    let resources: Vec<TodoResource> =
        page.data.into_iter().map(TodoResource::from).collect();
    let paginator = LengthAwarePaginator::new(resources, total, per_page, current_page)
        .with_path("/api/todos");
    Resource::paginated(paginator).render().await
}

// GET /api/todos/{todo}
#[handler]
pub async fn show(todo: i64) -> Response {
    let todo = Todo::find_or_fail(todo).await?;
    Resource::single(TodoResource::from(todo)).render().await
}

// POST /api/todos
#[handler]
pub async fn store(form: CreateTodoRequest) -> Response {
    let todo = Todo::create(attrs! {
        title: form.title,
        description: form.description,
        done: false,
    })
    .await?;
    Resource::single(TodoResource::from(todo))
        .created()           // 201
        .render()
        .await
}

// PUT /api/todos/{todo}
#[handler]
pub async fn update(todo: i64, form: UpdateTodoRequest) -> Response {
    let row = Todo::find_or_fail(todo).await?;

    let mut changes = attrs!();
    if let Some(title) = form.title {
        changes.insert("title", title.into());
    }
    if let Some(description) = form.description {
        changes.insert("description", description.into());
    }
    if let Some(done) = form.done {
        changes.insert("done", done.into());
    }
    let updated = row.update(changes).await?;
    Resource::single(TodoResource::from(updated)).render().await
}

// DELETE /api/todos/{todo}
#[handler]
pub async fn destroy(todo: i64) -> Response {
    Todo::find_or_fail(todo).await?.delete().await?;
    suprnova::json_response!({ "deleted": true })
}
```

把它接到 `src/controllers/mod.rs` 里：

```rust
pub mod todos;
```

这个参数名必须与路由占位符一致 - `{todo}` 对应 `todo: i64`。这个宏会通过 `FromParam` 解析这个路径段，然后处理程序方法体驱动 Eloquent 接口去加载、更新和删除这一行。

## 步骤 6：路由

`src/routes.rs`：

```rust
use crate::controllers::todos;
use suprnova::{delete, get, post, put, routes};

routes! {
    get!("/api/todos",           todos::index   ).name("todos.index"),
    get!("/api/todos/{todo}",    todos::show    ).name("todos.show"),
    post!("/api/todos",          todos::store   ).name("todos.store"),
    put!("/api/todos/{todo}",    todos::update  ).name("todos.update"),
    delete!("/api/todos/{todo}", todos::destroy ).name("todos.destroy"),
}
```

这个 `routes!` 宏会返回一个配置好的 `Router`，供 `Application::routes(...)` 在启动时消费。

## 步骤 7：运行它

```bash
suprnova serve --backend-only
```

### 创建

```bash
curl -X POST http://localhost:8765/api/todos \
  -H "Content-Type: application/json" \
  -d '{"title": "Read JSON:API spec", "description": "All of it"}'
```

```json
{
  "data": {
    "type": "todos",
    "id": "1",
    "attributes": {
      "title": "Read JSON:API spec",
      "description": "All of it",
      "done": false,
      "created_at": "2026-05-30T12:00:00+00:00",
      "updated_at": "2026-05-30T12:00:00+00:00"
    }
  }
}
```

### 列表（分页）

```bash
curl http://localhost:8765/api/todos
```

```json
{
  "data": [
    { "type": "todos", "id": "1", "attributes": { … } }
  ],
  "meta": {
    "pagination": {
      "total": 1,
      "per_page": 20,
      "current_page": 1,
      "last_page": 1
    }
  },
  "links": {
    "first": "?page=1",
    "last":  "?page=1",
    "prev":  null,
    "next":  null
  }
}
```

### 稀疏字段集

```bash
curl 'http://localhost:8765/api/todos/1?fields[todos]=title,done'
```

```json
{
  "data": {
    "type": "todos",
    "id": "1",
    "attributes": { "title": "Read JSON:API spec", "done": false }
  }
}
```

`IncludeMiddleware` 会解析 `?fields[type]=...`，把这个过滤器绑定到一个任务本地变量上，而 `Resource::single` 会在渲染期间读取它 - 处理程序完全看不到这个查询参数。

### 更新

```bash
curl -X PUT http://localhost:8765/api/todos/1 \
  -H "Content-Type: application/json" \
  -d '{"done": true}'
```

一个局部的请求体之所以能用，是因为 `UpdateTodoRequest` 里的每一个字段都是 `Option<T>` - 处理程序只会写入那些实际到达的键。

### 删除

```bash
curl -X DELETE http://localhost:8765/api/todos/1
# {"deleted": true}
```

### 验证失败

```bash
curl -X POST http://localhost:8765/api/todos \
  -H "Content-Type: application/json" \
  -d '{"title": ""}'
```

```json
{
  "message": "The given data was invalid.",
  "errors": { "title": ["title is required"] },
  "request_id": "8f9e1a2b-…"
}
```

422，带着 Laravel/Inertia 错误包 - 处理程序方法体根本没有运行。

## 每一部分位于何处

| 文件 | 角色 |
|------|------|
| `src/migrations/m*_create_todos_table.rs` | 架构 |
| `src/models/todo.rs` | `#[suprnova::model]` 结构体 |
| `src/requests.rs` | `#[request]` 表单请求，由 `extract()` 验证 |
| `src/resources/todo_resource.rs` | `#[derive(Data)]` + `#[json_resource("todos")]` |
| `src/controllers/todos.rs` | `#[handler]` 函数 |
| `src/routes.rs` | `routes!` 注册 |

## 下一步

- [Eloquent](eloquent.md) - 完整的 Model 接口、查询构造器、`attrs!`、生命周期事件、软删除、关系
- [验证](validation.md) - `#[request]`、`validate!`、`Unique`、异步钩子、跨字段规则
- [JSON:API 资源](eloquent-resources.md) - `?include=` 链、逐资源的 links/meta、`Maybe<T>` 条件属性
- [表单请求](requests.md) - `FormRequest` trait、内容类型分发、`authorize(&Request)`
- [控制器](controllers.md) - `#[handler]` 提取了什么，以及路由模型绑定在底层是如何工作的
