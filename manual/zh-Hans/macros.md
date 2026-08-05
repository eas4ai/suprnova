# 宏

Suprnova 提供了大约三十多个宏，全都从 `suprnova::*` 重新导出。它们是框架与您的代码彼此衔接的地方 - `routes!` 构建路由器，`#[handler]` 把一个函数适配成处理程序，`#[suprnova::model]` 把一个结构体变成一个 Eloquent 模型，`#[derive(Data)]` 产生一个类型化的 Inertia 负载。本章是索引：每个宏都会有一段描述、一个最小示例，以及一个指向实际使用它的那一章的指针。

有几条原则贯穿整个宏的表面：

- **宏会输出完全限定路径。** 生成的代码会写 `::suprnova::…`，所以无论您是否已经导入了底层类型，这些宏都能正常工作。
- **大量使用 `inventory::submit!`。** 模型、命令、策略、观察者、支付提供商等等，都会在编译期自行注册，框架会在启动时清空这个注册表。您几乎从不需要手动接入注册逻辑。
- **在划算的地方做编译期校验。** `inertia_response!` 会检查具名的组件文件是否存在。`redirect!` 会检查具名的路由是否存在。`routes!` 会拒绝不以 `/` 开头的路径。凡是能在构建期捕获的错误，都会在构建期被捕获。

## 路由

| 宏 | 返回值 | 作用 |
|---|---|---|
| `routes!` | `pub fn register() -> Router` | 顶层的路由列表 - 导出一个供您的 `app.rs` 调用的 `register()` |
| `get!` / `post!` / `put!` / `delete!` / `patch!` / `head!` / `options!` / `any!` | `RouteDefBuilder<H>` | 一条 HTTP 路由 - 可链式调用 `.name(...)` / `.middleware(...)` |
| `group!` | `GroupDef` | 应用到一组子路由上的前缀 + 中间件 |
| `fallback!` | `FallbackDefBuilder<H>` | 没有路由匹配时的自定义 404 处理程序 |
| `ws!` | `WsRouteDef` | 一条 WebSocket 路由 - 可链式调用 `.middleware(...)` / `.config(...)` |

```rust
use suprnova::{routes, get, post, ws, group};
use crate::{controllers, middleware::AuthMiddleware, ws::ChatHandler};

routes! {
    get!("/", controllers::home::index).name("home"),
    get!("/users/{id}", controllers::user::show).name("users.show"),
    post!("/users", controllers::user::store).name("users.store"),

    group!("/admin", {
        get!("/dashboard", controllers::admin::dashboard),
    }).middleware(AuthMiddleware),

    ws!("/ws/chat", ChatHandler),
}
```

路由路径字符串会在编译期被检查 - `validate_route_path` 会拒绝任何不以 `/` 开头的路径。通过 `.name("…")` 注册的路由名称，也会在启动时通过 `register_route_name` 检查唯一性。完整的展开过程参见[路由](routing.md)，`ws!` 参见 [WebSocket](websockets.md)。

## 处理程序与请求

### `#[handler]`

改写一个控制器函数，让它能直接从传入的请求中提取类型化的参数（通过 `FromRequest`）- 您不再需要手动从 `Request` 上取出各个字段，而是声明处理程序需要什么，宏会负责把它接好。

```rust
use suprnova::{handler, Response, json_response, request};

#[request]
pub struct CreateUserRequest {
    #[validate(email)]
    pub email: String,

    #[validate(length(min = 8))]
    pub password: String,
}

#[handler]
pub async fn store(form: CreateUserRequest) -> Response {
    // `form` 已经通过验证 - 失败时会自动返回 422
    json_response!({ "email": form.email })
}
```

一个 `Request` 形状的首个参数仍然被接受，作为恒等情形。参见[控制器](controllers.md)。

### `#[request]` 和 `#[derive(FormRequest)]`

`#[request]` 是声明一个经过验证的请求类型的推荐方式。它会自动派生 `Deserialize`、`Validate` 和 `FormRequest`，所以这个结构体既能处理 `application/json`，也能处理 `application/x-www-form-urlencoded` 的请求体。

如果您想不使用这个属性宏，`#[derive(FormRequestDerive)]` 是底层的派生宏（您需要自己派生 `Deserialize` 和 `Validate`）。我们推荐使用属性宏；派生宏是为边缘情形而存在的。参见[请求](requests.md)和[验证](validation.md)。

### `#[derive(MultipartRequest)]`

针对 `multipart/form-data` 的强类型提取器 - 在一个结构体里绑定文本字段和上传的文件，并为每个字段提供类型级别的校验器。

```rust
use suprnova::{MultipartRequest};
use suprnova::http::upload::{Image, MaxSize, UploadedFile};

#[derive(MultipartRequest)]
pub struct AvatarUpload {
    #[field("avatar")]
    pub avatar: UploadedFile<(Image, MaxSize<5_242_880>)>,

    #[field("caption")]
    pub caption: Option<String>,
}
```

内置的校验器（`Image`、`MimeAllowlist<…>`、`MaxSize<…>`、`MimeType<…>`）通过元组组合。参见[请求](requests.md)。

## 响应

### `json_response!` 和 `text_response!`

这两个简写形式的响应宏，都会把 `HttpResponse::*` 包裹进 `Ok(...)` 里，所以可以直接放进处理程序的返回位置：

```rust
use suprnova::{handler, json_response, text_response, Response};

#[handler]
pub async fn health() -> Response {
    json_response!({ "status": "ok" })
}

#[handler]
pub async fn robots() -> Response {
    text_response!("User-agent: *\nDisallow:")
}
```

参见[响应](responses.md)。

### `inertia_response!`

构建一个 Inertia 页面响应，并在编译期校验具名的组件文件（`.svelte` / `.tsx` / `.jsx` / `.vue`）是否存在于 `frontend/src/pages/` 里。如果您拼错了组件名，构建会失败并给出建议：

```rust
use suprnova::{handler, inertia_response, InertiaProps, Request, Response};

#[derive(InertiaProps)]
struct HomeProps {
    title: String,
    user_count: i64,
}

#[handler]
pub async fn index(req: Request) -> Response {
    inertia_response!(&req, "Home", HomeProps {
        title: "Welcome".into(),
        user_count: 42,
    })
}
```

`#[derive(InertiaProps)]` 会生成响应形状所需要的 `Serialize` 实现。参见 [Inertia 响应](frontend-inertia-responses.md)。

### `redirect!`

类型安全地重定向到一个具名路由 - 路由名会在编译期与通过 `routes!` 注册的名称进行核对：

```rust
use suprnova::redirect;

// 只有当 "users.show" 是一个已注册的路由名时才能编译通过
let resp = redirect!("users.show").with("id", "42").into();
```

参见 [URL 生成](urls.md)。

## Eloquent

### `#[suprnova::model]`

把一个普通的结构体变成一个完整的 Eloquent 模型：生成 SeaORM 的 `Entity`、`Model`、`ActiveModel`、`Column`、`Relation` 存根，以及 Eloquent 所需要的全部 trait 实现。还会 `inventory::submit!` 一个 `ModelEntry`，让框架能在启动时枚举出每一个模型。

```rust
use suprnova::model;

#[model(table = "users")]
pub struct User {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
```

属性键包括 `table`、`primary_key`、`key_type`、`auto_increment`、`connection`、`fillable`、`guarded`、`casts`、`timestamps`、`soft_deletes`、`appends`、`hidden`、`visible`、`mutators`、`touches`，以及（用于 UUID/ULID 主键的）`unique_id`。参见 [Eloquent](eloquent.md)。

### `#[suprnova::scopes(Model)]`

遍历一个 `impl Model { … }` 块，把每一个签名匹配 `fn name(query: Builder<Self>[, args…]) -> Builder<Self>` 的方法都变成一个 scope - 同时生成 `Model::scope_name(args)`，以及 `Builder<Model>` 上一个可链式调用的 `.scope_name(args)`。

```rust
use suprnova::{scopes, Builder};

#[suprnova::scopes(User)]
impl User {
    pub fn active(query: Builder<Self>) -> Builder<Self> {
        query.filter("active", true)
    }

    pub fn popular(query: Builder<Self>, threshold: i64) -> Builder<Self> {
        query.filter_op("followers_count", ">", threshold)
    }

    // 不是 scope - 原样透传
    pub fn display_name(&self) -> String { self.name.clone() }
}

// 两种调用方式都能编译通过：
// User::active().popular(500).get().await?;
// User::query().filter_op("id", ">", 0).active().get().await?;
```

从另一个模块调用时，可链式的形式需要生成的 trait `HasScope_<scope>_<Model>` 在作用域内。参见 [Eloquent](eloquent.md)。

### `#[suprnova::observer(Model)]`

把一个 `impl Observer<M>` 块接入生命周期事件系统 - 16 个被重写的方法里的每一个，都会变成一个已注册的监听器，提交到 inventory 里，并在启动时被清空。

```rust
use async_trait::async_trait;
use suprnova::eloquent::observers::Observer;
use suprnova::eloquent::events::EventResult;
use suprnova::eloquent::attrs::Attrs;
use suprnova::FrameworkError;

pub struct AuditObserver;

#[suprnova::observer(User)]
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

**必须遵守的属性顺序：`#[suprnova::observer(M)]` 必须写在 `#[async_trait]` 之前。** 属性宏是由外向内展开的 - 如果 `async_trait` 先运行，它会把每一个 `async fn` 都重写成一个脱糖后的形状，而 observer 宏按 16 个 trait 方法名做的名称匹配就会悄无声息地一无所获。参见[事件](events.md)。

### `#[suprnova::accessor]` 和 `#[suprnova::mutator]`

在 `impl Model { … }` 的方法上做函数级别的标记，接入模型的 `to_json()` / `fill()` 路径。在 `#[model(appends = […])]`（accessor）或 `#[model(mutators = […])]`（mutator）里引用字段名，宏就会把它们接好。

```rust
#[suprnova::model(appends = ["full_name"], mutators = ["password"])]
pub struct User {
    pub id: i64,
    pub first_name: String,
    pub last_name: String,
    pub password: String,
}

impl User {
    #[suprnova::accessor]
    pub fn full_name(&self) -> String {
        format!("{} {}", self.first_name, self.last_name)
    }

    #[suprnova::mutator]
    pub fn set_password(
        &mut self,
        value: serde_json::Value,
    ) -> Result<(), suprnova::FrameworkError> {
        let raw: String = serde_json::from_value(value)
            .map_err(|e| suprnova::FrameworkError::validation("password", format!("{e}")))?;
        self.password = bcrypt(raw);
        Ok(())
    }
}
```

参见[修改器与转换](eloquent-mutators.md)。

### `#[suprnova::prunable]`

包裹一个 `Prunable`（或 `MassPrunable`）实现，并把一个 `PrunerEntry` 提交进 `model:prune` 在运行时会遍历的那个注册表：

```rust
use async_trait::async_trait;
use chrono::{Duration, Utc};
use suprnova::eloquent::Prunable;

#[suprnova::prunable]
#[async_trait]
impl Prunable for Session {
    fn prunable() -> suprnova::Builder<Self> {
        Self::query().filter_op(
            "expires_at",
            "<",
            (Utc::now() - Duration::days(30)).to_rfc3339(),
        )
    }
}
```

参见 [Eloquent](eloquent.md)。

### `attrs!`

为 `Model::create` / `Model::update` / `Model::fill` 构建一个有序的 `Attrs` 映射（`IndexMap<&'static str, serde_json::Value>`）：

```rust
use suprnova::attrs;

let user = User::create(attrs! {
    name: "Alice",
    email: "alice@example.com",
    age: 32,
}).await?;
```

参见 [Eloquent](eloquent.md)。

### `casts!`

构建一个可以传给 `Builder::with_casts` 的、按查询生效的转换映射：

```rust
use suprnova::{casts, AsDate, AsJson};

let map = casts! {
    birthday = AsDate,
    metadata = AsJson<serde_json::Value>,
};
let rows = User::query().with_casts(map).get().await?;
```

参见[修改器与转换](eloquent-mutators.md)。

### `route_binding!`

为一个手写的 SeaORM 实体实现 `RouteBinding`，让它能从路由参数里自动解析出来。用 `#[suprnova::model]` 定义的模型会自动注册，不需要这个宏；当您是手写的实体时，才需要用到 `route_binding!`：

```rust
use suprnova::route_binding;

route_binding!(crate::entities::user::Entity, User, "user");
```

在那之后，`get!("/users/{user}", controllers::user::show)` 就会把一个完全加载好的 `User` 传给您的处理程序。参见[路由](routing.md)。

## 数据与 Inertia

### `#[derive(Data)]`

面向类型化负载的复合派生宏。生成一个尊重 `#[data(input_only)]` 字段的 `Serialize` 实现，以及一个会拒绝任何试图设置 `#[data(output_only)]` 字段的负载的 `Deserialize` 实现。搭配 `#[json_resource("type")]`，可以通过 `Resource` 一章获得 JSON:API 输出。

```rust
use suprnova::{Data, Validate};

#[derive(Data, Validate)]
struct UserDto {
    pub id: i64,
    pub name: String,

    #[data(input_only)]
    #[validate(length(min = 8))]
    pub password: String,

    #[data(output_only)]
    pub computed_handle: String,

    #[data(allow_include)]
    pub posts: Vec<PostDto>,
}
```

`#[data(allow_include)]` 会通过 `inventory::submit!` 把这个字段注册进部分重载的 include 允许列表里。参见[数据对象](data.md)和 [API 资源](eloquent-resources.md)。

### `#[derive(InertiaProps)]`

生成 `inertia_response!` 所需要的 `Serialize` 实现。是一个单纯的标记派生宏 - 大多数应用会转而使用 `#[derive(Data)]`，因为它能免费给您带来部分重载的 include。

```rust
use suprnova::InertiaProps;

#[derive(InertiaProps)]
struct DashboardProps {
    title: String,
    user: User,
}
```

参见 [Inertia 响应](frontend-inertia-responses.md)。

### `when_loaded!`

只有当一个具名的关系已经在实体上被预加载时，才会发出一个 `Prop::lazy(…)`；否则会发出 `Prop::EagerNone`，让这个 prop 完全从响应中跳过：

```rust
use suprnova::when_loaded;

let songs_prop = when_loaded!(&artist, "songs", || async {
    serde_json::to_value(&artist.songs).unwrap()
});
```

参见[数据对象](data.md)。

## 依赖注入

### `#[service]`

给一个 trait 加上 `Send + Sync + 'static`，让它可以放进容器：

```rust
use suprnova::service;

#[service]
pub trait HttpClient {
    async fn get(&self, url: &str) -> Result<String, FrameworkError>;
}

// App::bind::<dyn HttpClient>(Arc::new(RealHttpClient::new()));
// let client = App::make::<dyn HttpClient>()?;
```

参见[服务容器](container.md)。

### `#[injectable]`

自动把一个具体类型注册为单例。派生 `Default` + `Clone`，并提交一个会在启动时运行的注册：

```rust
use suprnova::injectable;

#[injectable]
pub struct AppState {
    pub counter: u32,
}

// let state: AppState = App::get().unwrap();
```

参见[服务容器](container.md)。

## 错误

### `#[domain_error]`

定义一个领域错误，让它实现 `Display`、`Error`、`HttpError`，以及 `From<T> for FrameworkError` - 这样它就能通过 `?` 让处理程序短路：

```rust
use suprnova::domain_error;

#[domain_error(status = 404, message = "User not found")]
pub struct UserNotFoundError {
    pub user_id: i32,
}

pub async fn get_user(id: i32) -> Result<User, FrameworkError> {
    let user = User::find(id).await?
        .ok_or_else(|| UserNotFoundError { user_id: id })?;
    Ok(user)
}
```

参见[错误处理](errors.md)。

## 控制台与后台工作

### `#[command]`

把一个 `async fn(Vec<String>) -> Result<(), FrameworkError>` 标记为一个控制台命令。提交一个 `CommandEntry`，这样当每个项目自带的控制台二进制文件运行时，`dispatch_argv` 就能找到它：

```rust
use suprnova::{command, FrameworkError};

#[command(name = "db:seed", description = "Run all registered seeders")]
async fn db_seed(_args: Vec<String>) -> Result<(), FrameworkError> {
    suprnova::seed::run_all().await
}
```

参见[控制台](console.md)。

### `#[derive(Command)]`

类型化参数的替代方案。叠加在 `#[derive(clap::Parser)]` 之上，从 `#[console(...)]` 里读取元数据，并生成调用您的 `TypedCommand::run` 的运行器：

```rust
use async_trait::async_trait;
use suprnova::{Command, FrameworkError, TypedCommand};

#[derive(clap::Parser, Command)]
#[console(name = "greet", description = "Greet someone")]
pub struct Greet {
    #[arg(short, long)]
    name: Option<String>,
    #[arg(long)]
    loud: bool,
}

#[async_trait]
impl TypedCommand for Greet {
    async fn run(self) -> Result<(), FrameworkError> {
        let target = self.name.unwrap_or_else(|| "world".into());
        println!("{}", if self.loud { format!("HELLO {target}!") } else { format!("Hello {target}") });
        Ok(())
    }
}
```

参见[控制台](console.md)。

### `#[workflow]` 和 `#[workflow_step]`

`#[workflow]` 把一个异步函数注册为一个持久化的工作流 - 可运行的状态、可重试的步骤、持久化的历史记录。函数体内的每一个 `#[workflow_step]` 都是一个检查点，运行时可以在崩溃或重启后从那里恢复。

```rust
use suprnova::{workflow, workflow_step, FrameworkError};

#[workflow]
async fn onboard_user(user_id: i64) -> Result<(), FrameworkError> {
    send_welcome_email(user_id).await?;
    enable_default_features(user_id).await?;
    Ok(())
}

#[workflow_step]
async fn send_welcome_email(user_id: i64) -> Result<(), FrameworkError> {
    // …
    Ok(())
}
```

### `start_workflow!`

按路径启动一个工作流，把参数序列化进工作流运行时的信封格式里：

```rust
use suprnova::start_workflow;

let handle = start_workflow!(crate::workflows::onboard_user, 42).await?;
```

参见[工作流](workflows.md)。

### `schedule_task!`

围绕 `TaskBuilder::from_async` 的语法糖，让一个闭包能和基于 trait 的 `Task` 实现一起干净地调度：

```rust
use suprnova::{schedule_task, FrameworkError};

let task = schedule_task!(|| async {
    println!("ticking");
    Ok::<(), FrameworkError>(())
})
    .every_minute()
    .name("tick");
```

参见[任务调度](scheduling.md)。

## 授权

### `#[policy(UserType, ResourceType)]`

包裹一个 `impl Policy` 块，把每一个方法都注册为一个具名的 gate 动作。gate 名称由方法名和小写的资源类型组合而成 - `Comment` 上的 `fn view(...)` 会变成 `"view-comment"`：

```rust
use suprnova::policy;

struct CommentPolicy;

#[policy(User, Comment)]
impl CommentPolicy {
    fn view(_user: &User, _comment: &Comment) -> bool { true }
    fn update(user: &User, comment: &Comment) -> bool {
        comment.author_id == user.id
    }
}
```

`Server::run` 会自动调用 `authorization::init_policies()`。参见[授权](authorization.md)。

## 通知与邮件

### `#[derive(NotificationMailable)]`

从一个 `#[mail(...)]` 属性自动生成 `to_mail` - 主题、HTML 正文和文本正文可以用内联的 Tera 模板，也可以用文件里的 Tera 模板。编译期检查：subject 必须存在，至少要有一种正文存在，html/html_template 二选一，`from_name` 需要 `from`：

```rust
use serde::{Serialize, Deserialize};
use suprnova::NotificationMailable;

#[derive(Serialize, Deserialize, NotificationMailable)]
#[mail(
    subject = "Your order shipped - tracking {{ tracking }}",
    html    = "<p>Tracking: <code>{{ tracking }}</code></p>",
    text    = "Tracking: {{ tracking }}",
    from    = "orders@suprnova.dev",
)]
pub struct OrderShipped { pub tracking: String }
```

通知 trait 本身是手写实现的 - 没有 `#[derive(Notification)]`。参见[通知](notifications.md)和[邮件](mail.md)。

## 验证

### `validate!`

同步的、声明式的验证入口。每一行把一个字段名和一个或多个 `Rule`（或 `ContextualRule`）值配对，用 `?:` 表示“仅当存在时才验证”，用 `?=>` 表示条件必填的可选字段：

```rust
use suprnova::{validate, ValidationErrors};
use suprnova::validation::rules::*;

fn validate_form(self_ref: &SignupForm) -> Result<(), ValidationErrors> {
    validate! { self_ref =>
        email   => Required, Email;
        password => Required, Min(8);
        bio     ?: Max(500);
        card_number ?=> RequiredIf { other: "billing_type", value: "card" } => with ctx;
    }
}
```

`Validate` 是从 `validator` crate 重新导出的 - `#[validate(...)]` 属性（例如 `#[validate(email)]`）来自 `validator`，并通过 `FormRequest` 的同步路径运行。当您需要上下文相关/跨字段的规则、异步规则，或者 `suprnova::validation::rules` 里的规则时，使用 `validate!`。参见[验证](validation.md)。

## 工厂

### `#[derive(Factory)]`

生成一个同级的 `<Model>Factory` 标记，以及一个通过 `fake::Faker` 产生模型的 `Factory` 实现。这个模型必须实现 `fake::Dummy<fake::Faker>` - 通常通过 `#[derive(Dummy)]`：

```rust
use suprnova::{Dummy, Factory};

#[derive(Dummy, Factory)]
pub struct User {
    pub id: i32,
    pub name: String,
    pub email: String,
}

// UserFactory 已经存在：
let users = UserFactory::new().count(10).make_many();
```

参见[工厂](eloquent-factories.md)。

## 测试

### `#[suprnova_test]`

用一个内存 SQLite 数据库（默认运行 `crate::migrations::Migrator`）包裹一个 `async fn` 测试，调用 `App::init()` 和 `App::boot_services()`，并在 `#[tokio::test]` 下运行函数体。并行测试通过容器的逐线程层保持彼此隔离 - 通过 `TestContainer::fake`（而不是 `App::bind`）绑定测试专属的服务，这样每个线程都能看到自己的伪造实现：

```rust
use suprnova::suprnova_test;
use suprnova::testing::TestDatabase;

#[suprnova_test]
async fn creates_a_user(db: TestDatabase) {
    let user = User::create(attrs! { name: "A", email: "a@x.com" }).await.unwrap();
    assert!(user.id > 0);
}
```

自定义迁移器通过 `#[suprnova_test(migrator = MyMigrator)]` 使用。参见[测试](testing.md)。

### `test_database!`

供那些不通过 `#[suprnova_test]` 获取 `db` 参数的测试使用的单行 `TestDatabase` 构造器：

```rust
let db = test_database!();
let db = test_database!(my_crate::CustomMigrator);
```

### `describe!`、`test!`、`expect!`

Jest 风格的分组 + 流式断言。`describe!` 是一个模块，`test!` 产生一个 `#[test]`（同步或异步，带或不带 `TestDatabase` 参数），`expect!` 把一个值包裹起来，以便进行链式断言，失败时带有文件/行号上下文：

```rust
use suprnova::{describe, test, expect};

describe!("CreateUserAction", {
    test!("creates a user", async fn(db: TestDatabase) {
        let user = CreateUserAction::new()
            .execute("test@example.com").await.unwrap();
        expect!(user.email).to_equal("test@example.com".to_string());
    });
});
```

参见[测试](testing.md)。

## 中间件

### `global_middleware!`

注册一个在每个请求上都会运行的中间件，按注册顺序排列，先于任何路由特定的中间件运行。按类型幂等：

```rust
use suprnova::global_middleware;
use crate::middleware;

pub fn register() {
    global_middleware!(middleware::LoggingMiddleware);
    global_middleware!(middleware::CorsMiddleware);
}
```

必须在 `Server::from_config` / `Server::new` 之前运行 - 服务器会在构建时对全局注册表拍一次快照。参见[中间件](middleware.md)。

## 陷阱

一份简短的清单，列出那些容易踩中、也容易修复的失败模式。

### 属性顺序 - `#[observer]` 必须写在 `#[async_trait]` 之前

```rust
// 正确
#[suprnova::observer(User)]
#[async_trait]
impl Observer<User> for AuditObserver { … }

// 错误 - 会静默地产生零个监听器
#[async_trait]
#[suprnova::observer(User)]
impl Observer<User> for AuditObserver { … }
```

属性宏是由外向内展开的。`async_trait` 会把每一个 `async fn` 都重写成一个脱糖后的 `Pin<Box<dyn Future>>` 形状。如果它先运行，observer 宏就再也无法按方法名匹配，什么都不会产生。每当您叠加多个宏时，同样的由外向内规则都适用 - 拿不准的时候，把 Suprnova 的属性放在最外层。

### 固有实现陷阱

一个固有 `impl` 方法**无法**通过 trait 分发去遮蔽一个 trait 的默认方法。如果您写了一个宏（或者手写代码），把 `fn save(&self)` 定义成模型上的一个固有方法，那些经过 `Model` trait 分发的调用（`some_model.save()`，其中调用点只知道它是 `&dyn Model`）会选中 trait 的默认实现 - 而不是您的固有重载。

修复方式：当生成的行为必须参与 trait 分发时，就生成一个 trait 方法重载，而不是一个固有方法。这就是为什么框架的宏（尤其是 `#[suprnova::model]`）会写到 trait 实现里。如果您在手写 Eloquent 扩展，也请这样做。

### `global_middleware!` 只在 `Server::from_config` 之前生效

服务器会在构建时对全局注册表拍一次快照。在 `Server::from_config(...)` 之后调用 `global_middleware!(M)`，不会追溯性地应用到那个服务器上。请在 `bootstrap()` 里注册每一个全局中间件，在 `Application::run()` 到达 serve 步骤之前完成。

### `redirect!` 和 `inertia_response!` 是编译期检查

如果具名的目标不存在，这两个宏都会拒绝编译 - 这正是它们的意义所在。如果一次重构移除了一个路由名或组件名，每一个提到它的调用点都会让构建失败，这正是您想要的效果。如果构建错误让您感到意外，在“修复”宏调用之前，先去您的 `routes!` 块 / pages 目录里搜索那个字符串字面量。

### `?:` 在 `None` 时跳过；`?=>` 即使 `None` 也会运行

在 `validate!` 的行里，`?:` 只有当字段是 `Some` 时才会运行规则。因此，像 `RequiredIf` 这样一个依赖存在性的规则，放在一个 `?:` 行上，永远无法让一个缺失的字段验证失败。对于“当 X 时必填”这种情形，请使用 `?=>`（它会把缺失当作 `""` 处理）。

### `#[derive(Validate)]` 来自 `validator` crate，不是 Suprnova

Suprnova 重新导出了 `validator::Validate`，这样您就不需要直接依赖 `validator`。`#[validate(...)]` 属性来自 `validator`。Suprnova 自己的 `validate!` 宏，则是运行时跨字段/上下文相关的入口；两者相辅相成，但活在不同的命名空间里。

## 为什么 Suprnova 有所不同

Laravel 在运行时发现路由、命令、邮件模板、模型类、工厂、观察者和策略 - 通过反射、文件系统扫描和基于字符串的分发。PHP 让这一切变得廉价（自动加载 + opcache 摊薄了成本），开发者体验也很出色：把一个文件放进正确的目录，它就会出现。

这个模型不适合 Rust。我们没有针对 trait 实现的运行时反射，运行时是一个单一的静态链接二进制文件，而在一个每个二进制文件要服务数百万请求的进程模型里，启动时的文件系统扫描是更糟糕的选择。

所以 Suprnova 在编译期做同样的事情。路由会被校验，组件名会对照 pages 目录进行检查，邮件模板通过 `include_str!` 被嵌入，路由名通过 inventory 检查唯一性，模型在一个框架会在启动时清空的 inventory 里自行注册，命令也是如此。开发者体验是相似的 - 放一个文件，加一个 `#[command]` 或 `#[suprnova::model]`，运行二进制文件 - 只是接线发生在 `main` 之前，而不是第一个请求到达时。

代价是，拼写错误、缺失的组件和失效的引用会变成构建期错误，而不是运行时错误，并且每个请求零反射开销。

## 下一步

- [路由](routing.md) - 完整的 `routes!` 展开、命名、模型绑定
- [控制器](controllers.md) - `#[handler]` 和 `#[request]` 一起使用
- [Eloquent](eloquent.md) - `#[suprnova::model]` 及其相关内容
- [验证](validation.md) - `validate!`、上下文相关规则、异步规则
- [控制台](console.md) - `#[command]` 和 `#[derive(Command)]` 从头到尾
- [测试](testing.md) - `#[suprnova_test]`、`expect!`、伪造实现
