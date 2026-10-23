# 操作

Suprnova 里的操作是一个只做一件事的结构体：把单一一块业务逻辑封装在一个方法背后。它是 Laravel 单方法可调用控制器的 Rust 对应物 - `RegisterUser`、`PublishPost`、`ChargeInvoice`。这个操作住在 `src/actions/` 里，带着 `#[injectable]` 属性以便容器能够解析它，并暴露一个供控制器（以及作业和其他操作）调用的 `execute(...)` 方法。这里没有 `#[action]` 宏，框架侧也不会强制“一个方法”这条规则 - 这个形态是一种约定，而 `#[injectable]` 就是让这条约定变得毫不费力的机制。

```rust
use suprnova::{injectable, FrameworkError};

#[injectable]
pub struct RegisterUserAction {
    // 把依赖作为字段注入 - 参见下面的“依赖”一节
}

impl RegisterUserAction {
    pub async fn execute(&self, email: &str) -> Result<String, FrameworkError> {
        tracing::info!(action = "RegisterUser", email, "executed");
        Ok(format!("registered: {email}"))
    }
}
```

从一个处理程序里用 `App::resolve::<RegisterUserAction>()?` 解析它，您就把领域逻辑从 HTTP 层里拆分出来了，而不需要发明一个服务层基类。这就是这个模式的全部。

## 生成一个操作

```bash
suprnova make:action RegisterUser
```

CLI 会把名字规范化为 PascalCase，如果缺少 `Action` 后缀就补上，然后把文件名转成 snake_case。所以：

| `make:action <Name>` | 结构体名 | 文件 |
|---|---|---|
| `RegisterUser` | `RegisterUserAction` | `src/actions/register_user_action.rs` |
| `SendNotification` | `SendNotificationAction` | `src/actions/send_notification_action.rs` |
| `ProcessPayment` | `ProcessPaymentAction` | `src/actions/process_payment_action.rs` |
| `ChargeInvoiceAction` | `ChargeInvoiceAction` | `src/actions/charge_invoice_action.rs` |

生成器会写入这个文件，并在 `src/actions/mod.rs` 里添加一行 `pub mod register_user_action;`。生成出来的骨架代码可以立即编译：

```rust
//! register_user_action action

use suprnova::{injectable, FrameworkError};

/// RegisterUserAction
///
/// Single-responsibility command resolved from the container. Inject any
/// dependencies as fields and the `#[injectable]` macro wires them at
/// resolve time.
#[injectable]
pub struct RegisterUserAction {
    // Add injected dependencies as fields here, e.g.
    // db: suprnova::DbConnection,
}

impl RegisterUserAction {
    /// Execute the action.
    pub async fn execute(&self) -> Result<String, FrameworkError> {
        Ok("RegisterUserAction executed".to_string())
    }
}
```

这个签名 - `async fn execute(&self) -> Result<_, FrameworkError>` - 是生产安全的形态：异步的，返回一个 `Result`，通过 `?` 在调用点直接转换成 `HttpResponse`。方法体只是一个占位符；把它换成真正的工作流程。

## `#[injectable]` 属性

`#[injectable]` 是这个操作模式所依赖的唯一一块框架机制。它展开为三件事：

1. 在结构体上加一个 `#[derive(Clone)]`（当没有 `#[inject]` 字段时还会加上 `Default`）。
2. 一条 `inventory::submit!` 条目，这样启动过程就能发现这个类型。
3. 一个自动注册闭包，`App::singleton_if_absent` 会在 `boot_services()` 期间运行它一次。

这个宏的契约：

| 结构体形态 | 行为 |
|---|---|
| 单元结构体（`pub struct Foo;`） | 派生 `Default + Clone`，注册 `Default::default()` |
| 具名字段，没有 `#[inject]` | 派生 `Default + Clone`，注册 `Default::default()` |
| 带 `#[inject]` 的具名字段 | 只派生 `Clone`；每个 `#[inject]` 字段在启动时从容器解析，非注入字段用默认值 |
| 元组结构体 | 在编译期被拒绝 - “请改用具名字段” |

一个被解析出来的操作，是那个存储着的单例的一次克隆。代价是每次 `App::resolve::<Action>()?` 调用一次 `Clone`，对于一个单元结构体，或者一个由 `Arc` 包裹的服务构成的结构体来说，这不过是几次引用计数的递增。繁重的状态应该放在操作注入的 `Arc<dyn …>` 服务背后，而不是操作自身内部。

### `#[inject]` 发生在启动时，而不是每次调用时

当框架启动时，`App::boot_services()` 会遍历每一条 `#[injectable]` 注册，并在一个不动点重试循环里运行它们。每一条都会尝试从容器里解析它的 `#[inject]` 字段。如果某个依赖还没有被注册，这一条就会推迟到下一轮迭代。这个循环会一直运行，直到每一条都成功，或者不再有任何进展为止 - 一旦失败，框架就会返回一个结构化的错误，指出那个无法解析的类型或者那个循环依赖。

实际的后果是：**`App::resolve::<MyAction>()` 克隆的是那个已经构造好的单例**。它不会在每次调用时都重新运行一次 `#[inject]` 解析。任何一个操作所依赖的可注入对象，本身都必须先于这个操作被注册 - 要么通过它自己的 `#[injectable]` 属性，要么在您的 `bootstrap()` 函数里手动调用 `App::bind` / `App::singleton`。这个重试循环会为您处理 inventory 的顺序问题；它不会凭空生出缺失的服务。

## 从控制器里使用一个操作

标准的处理程序形态：解析、执行、渲染。

```rust
use suprnova::{App, Request, Response, ResponseExt, json_response};

use crate::actions::register_user_action::RegisterUserAction;

pub async fn store(_req: Request) -> Response {
    let action = App::resolve::<RegisterUserAction>()?;
    let result = action.execute("alice@example.com").await?;

    json_response!({ "ok": true, "result": result }).status(201)
}
```

两处 `?` 都能工作，因为两种错误类型都通过 `From` 实现转换成了 `HttpResponse` - `App::resolve` 返回 `Result<T, FrameworkError>`，框架的错误转换器负责处理剩下的部分。缺失的服务注册会表现为一个 500，服务名会记在结构化日志里，而不是一次 panic。完整的图景请参见[错误模型](error-model.md)。

如果您更愿意在 resolve 上避免用 `?` - 比如在一条应该在启动时就硬性失败的路径上 - `App::get::<RegisterUserAction>()` 返回 `Option<T>`，您可以用 `.expect("registered at boot")`，如果接线接错了就明确地失败。

## 触碰数据库的异步操作

这是大多数操作实际会走的路径 - 通过一个 Eloquent 模型进行加载或写入。把方法体从您的领域里搬过来就行；表面是一样的。

```rust
use suprnova::{attrs, injectable, FrameworkError, Model};

use crate::models::todos::Todo;

#[injectable]
pub struct CreateRandomTodoAction;

impl CreateRandomTodoAction {
    pub async fn execute(&self) -> Result<Todo, FrameworkError> {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
            % 10000;

        Todo::create(attrs! {
            title: format!("Todo #{}", n),
            description: format!("created at {}", n),
            done: false,
        })
        .await
    }
}

#[injectable]
pub struct ListTodosAction;

impl ListTodosAction {
    pub async fn execute(&self) -> Result<Vec<Todo>, FrameworkError> {
        Ok(<Todo as suprnova::eloquent::Model>::all().await?.into_vec())
    }
}
```

`Todo::create(attrs!{...})` 和 `Todo::all()` 都来自 `#[suprnova::model]` 宏。模型的完整表面请参见 [Eloquent](eloquent.md)。注意 `Model::all()` 返回的是一个 `Collection<Todo>` - 这个示例调用 `.into_vec()`，把一个朴素的 `Vec` 交给控制器；您也可以直接返回这个 `Collection`，让序列化器去渲染它。

把它们接进一个控制器：

```rust
use suprnova::{App, Request, Response, ResponseExt, json_response};

use crate::actions::todo_action::{CreateRandomTodoAction, ListTodosAction};

pub async fn create_random(_req: Request) -> Response {
    let action = App::resolve::<CreateRandomTodoAction>()?;
    let todo = action.execute().await?;
    json_response!({ "ok": true, "todo": todo }).status(201)
}

pub async fn list(_req: Request) -> Response {
    let action = App::resolve::<ListTodosAction>()?;
    let todos = action.execute().await?;
    json_response!({ "ok": true, "todos": todos })
}
```

每个处理程序两个 `?`；控制器始终是 HTTP 和领域之间的一层薄适配器。

## 通过 `#[inject]` 注入依赖

当一个操作需要协作者时 - 一个邮件发送器、一个日志记录器、一个领域服务 - 把它们声明成字段，并给每一个都打上 `#[inject]` 标签：

```rust
use suprnova::{injectable, FrameworkError};

use crate::services::{MailerService, LoggerService};

#[injectable]
pub struct SendWelcomeEmailAction {
    #[inject]
    mailer: MailerService,
    #[inject]
    logger: LoggerService,
}

impl SendWelcomeEmailAction {
    pub async fn execute(&self, to: &str) -> Result<(), FrameworkError> {
        self.logger.info(&format!("welcome → {to}"));
        self.mailer.send_welcome(to).await
    }
}
```

`MailerService` 和 `LoggerService` 自身都必须在这个操作启动之前完成容器注册 - 可以用它们自己的 `#[injectable]` 属性，也可以通过一次 `bootstrap()` 调用：

```rust
// 在 src/bootstrap.rs 中
App::singleton(MailerService::from_env()?);
App::singleton(LoggerService::default());
```

如果任何一个依赖在启动运行那个不动点循环时还缺失，启动就会返回一个指出那个未解析类型的错误，框架会以非零状态退出，而不是带着一个接线接了一半的容器启动起来。

非 `#[inject]` 字段会回退到 `Default::default()`，所以您可以把注入的依赖和普通状态混在一起，而不需要写一个构造函数。

## 什么时候该用一个操作

经验法则是：当同一块工作会（或可能会）从不止一个入口点被触发时，一个操作就有了存在的理由。一个既会从一条 HTTP 路由运行、又会从一个排队作业运行的注册流程，就属于 `RegisterUserAction`。一个一次性的“渲染这个索引页面”处理程序不需要一个操作 - 把它留在控制器里就好。

| 适合的场景 | 示例 |
|---|---|
| 多步骤的业务操作 | `RegisterUserAction`、`CheckoutAction` |
| HTTP + 队列共享的工作 | `IssueRefundAction`（两种方式都会被分发） |
| 值得脱离请求单独测试的逻辑 | `CalculateTotalsAction` |
| 外部集成 | `SendEmailAction`、`SyncInventoryAction` |
| 任何控制器本来会内联并重复的东西 | 三次法则触发 |

与控制器相比，一个操作是可复用的，没有 `Request` 绑定，并且从测试里调用它毫不费力（`App::resolve` + `await`）。控制器则始终是一个知道如何把一个操作的结果转换成 `Response` 的、感知 HTTP 的边界。

| 控制器 | 操作 |
|---|---|
| 处理一条路由 | 可在多条路由、作业、计划任务之间复用 |
| 了解 `Request` / `Response` | 了解您的领域类型 |
| 返回 `Response` | 返回 `Result<T, FrameworkError>` |
| 调用操作 | 被控制器（以及其他东西）调用 |

## 操作、总线与队列

操作并不是业务逻辑唯一能安身的地方 - [总线](bus.md)负责处理带类型化输出的、被分发的命令，而[队列](queues.md)负责处理那些应该在工作进程上运行的工作。按工作被调用的方式来选择：

| 您想要的是… | 该伸手去拿的是 |
|---|---|
| 同步的业务逻辑，可从一个控制器或一个作业里调用 | **操作**（`#[injectable]` + `execute`） |
| 一个带注册处理程序的类型化命令，可通过 `Bus::dispatch` 调用 | [总线](bus.md) |
| 持久化的、会重试的、脱离当前任务的工作 | [队列](queues.md) |

混用是可以的：一个 `BusHandler` 或一个 `Job` 常常只是解析出一个操作，然后调用它的 `execute`。操作持有领域逻辑；总线或队列持有分发所需的元数据。

## 文件布局

`make:action` 会生成什么，以及分组所需的空间：

```
src/
├── actions/
│   ├── mod.rs                          // pub mod register_user_action;
│   ├── register_user_action.rs
│   ├── send_welcome_email_action.rs
│   └── billing/                        // 当这个目录变大时，按域分组
│       ├── mod.rs
│       ├── charge_invoice_action.rs
│       └── issue_refund_action.rs
├── controllers/
└── main.rs
```

框架里没有任何东西强制要求这种布局；生成器写入 `src/actions/`，只是因为那是约定。把一个操作挪到 `src/billing/actions/`，它照样能工作 - `#[injectable]` 是不关心位置的。

## 测试一个操作

因为一个操作只是一个带 `async` 方法、可从容器解析的结构体，测试的接口就是 `App::resolve` + `await`。别处用到的那个同样的 `TestDatabase` 测试夹具在这里也能用：

```rust
use suprnova::{describe, expect, test, App};
use suprnova::testing::TestDatabase;

use crate::actions::todo_action::ListTodosAction;
use crate::models::todos::Todo;

describe!("ListTodosAction", {
    test!("returns all todos", async fn(_db: TestDatabase) {
        Todo::create(suprnova::attrs! { title: "Test", description: "", done: false })
            .await
            .unwrap();

        let action = App::resolve::<ListTodosAction>().unwrap();
        let todos = action.execute().await.unwrap();

        expect!(todos).to_have_length(1);
    });
});
```

完整的 `describe!` / `test!` / `expect!` 接口请参见[测试](testing.md)，需要向一个被测试的操作里注入一个假邮件发送器或假网关时，也请参见那里的 `TestContainer::fake`。

## 为什么 Suprnova 有所不同

Laravel 的单方法控制器 - 那些在 `App\Actions\` 里带一个 `__invoke` 方法的类 - 是逐请求构造的。容器解析这个类，运行构造函数注入，然后这个实例会在响应发出之后被丢弃。PHP 的每请求一个进程模型让这几乎是零成本的。

Suprnova 的操作是容器常驻的单例：在启动时构造一次，`#[inject]` 字段也在那时解析好，此后每次 `App::resolve` 都只是克隆出来。这个模式适合 Rust，因为克隆一个由 `Arc` 包裹的服务构成的结构体，代价只是几次引用计数的递增，而在每个请求上都构造并丢弃一个结构体，会强迫每一个字段都走一遍分配。这个 Laravel 形状的约定 - 一个结构体，一个方法，以操作命名 - 完好无损地保留了下来；它底下的接线是为 Tokio 的形状打造的。

另一处有意为之的拆分：控制器始终是自由函数（参见[控制器](controllers.md)），所以 HTTP 层是一次纯粹的、请求到响应的转换，自己没有任何 DI 接口。构造函数式的注入发生在 `#[injectable]` 这条边界上，也就是操作内部 - 它本该待的地方。

## 下一步

- [控制器](controllers.md) - 那些解析并调用操作的、面向 HTTP 的自由函数
- [服务容器](container.md) - `App::resolve`、`App::singleton`，以及那三层查找实际做的事
- [总线](bus.md) - 当您想要一个已注册的处理程序、而不是一个被解析出来的操作时，用来做类型化命令分发
- [测试](testing.md) - `App::resolve` + `TestContainer::fake`，用于隔离的操作测试
- [错误模型](error-model.md) - `App::resolve::<Action>()?` 和 `action.execute().await?` 上的 `?`，如何收拢成一个干净的响应
