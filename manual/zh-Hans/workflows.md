# 工作流

工作流是持久化的、长时间运行的异步函数，它们的中间状态能在崩溃、重启和 panic 之后存活下来。当一个工作单元跨越多个步骤 - 每一步都可能缓慢、可能失败、可能有副作用 - 而您承受不起在中途丢失进度时，就该伸手去拿工作流。一个工作流的函数体只运行一次；每一步的输出都会被持久化；一次重试会从第一个还没完成的步骤恢复。当这份工作是一次性作业时，搭配[`Queue`](queues.md)；当这份工作在请求任务里同步运行时，搭配[`Bus`](bus.md)。

## 快速上手

一个工作流是一个返回 `Result<T, FrameworkError>` 的异步函数；它的函数体会调用一个或多个 `#[workflow_step]` 函数；您通过 `start_workflow!` 宏把它入队，一个工作进程会把它排空。

```rust
use suprnova::{workflow, workflow_step, start_workflow, FrameworkError};

#[workflow_step]
async fn fetch_user(user_id: i64) -> Result<String, FrameworkError> {
    Ok(format!("user:{}", user_id))
}

#[workflow_step]
async fn send_welcome_email(user: String) -> Result<(), FrameworkError> {
    // …实际发送这封邮件
    Ok(())
}

#[workflow]
async fn welcome_flow(user_id: i64) -> Result<(), FrameworkError> {
    let user = fetch_user(user_id).await?;
    send_welcome_email(user).await?;
    Ok(())
}

// 从一个处理程序或者任何异步上下文里：
let handle = start_workflow!(welcome_flow, 123).await?;
```

这个宏会把参数序列化成 JSON，在 `workflows` 表里插入一行，并返回一个标识这个已入队实例的 [`WorkflowHandle`](#等待结果)。一个独立的工作进程会取走这一行，运行函数体，并随着执行把每一步的输出持久化下来。

`#[workflow]` 会把这个函数以它的完全限定路径（`module_path::fn_name`）收进工作流的 inventory。在同一个名字下重复注册，会通过 `registry::assert_no_duplicates` 中止工作进程的启动 - 静默的遮蔽会让人没法排查，所以框架选择明确地失败。

## 架构

工作流会持久化进两张表：`workflows`（每个实例一行）和 `workflow_steps`（每次步骤调用一行，以 `(workflow_id, step_index)` 为键）。这份架构由框架拥有；您来选择什么时候应用它。

有两种接入这些迁移的方式。

### 生成的迁移文件

CLI 会把框架迁移的副本脚手架进您的应用：

```bash
suprnova workflow:install
suprnova migrate
```

`workflow:install` 会在 `src/migrations/` 下写出 `m_create_workflows_table.rs` 和 `m_create_workflow_steps_table.rs`，然后把它们注册进您的 `Migrator`。当您想让这份架构和您应用的其他迁移一起被版本化时，就用这条路径。

### 编程式注册

或者，也可以直接注册框架自带的那些迁移结构体：

```rust
use sea_orm_migration::MigratorTrait;
use suprnova::workflow::migrations::{
    CreateWorkflowsTable, CreateWorkflowStepsTable,
};

pub struct Migrator;

impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
        vec![
            Box::new(CreateWorkflowsTable),
            Box::new(CreateWorkflowStepsTable),
        ]
    }
}
```

两条路径产出的 SQL 是完全一样的。[`features::migrations`](feature-flags.md) 和 [`payments::migrations`](payments.md) 用的是同一个约定。

## 运行工作进程

在一个脚手架出来的应用里，工作进程由这个二进制文件的 `workflow:work` 子命令启动：

```bash
suprnova workflow:work
```

这个工作进程走的是和您的 HTTP 服务器一样的 bootstrap，所以在 `bootstrap()` 里注册的观察者、监听器和容器绑定，工作流步骤都能看到。在 `SIGINT` / `SIGTERM` 上，工作进程会停止拉取新的认领，并在退出之前等待每一个飞行中的工作流完成 - 一次干净的关闭里，不会有工作流在某个步骤中途变成孤儿。

这条认领路径（`claim_next_workflow`）对 `workflows` 表使用 `FOR UPDATE SKIP LOCKED`，所以这个工作进程**要求使用 Postgres**。SQLite 和 MySQL 对测试、以及入队/持久化路径来说是可以工作的，但如果连接不是 Postgres，这个工作进程守护进程会在第一次认领时报错退出。

## 配置

五个环境变量用来调优这个工作进程。超出范围的值会被夹到安全的最小值，并带一条 `tracing::warn!`，这样 `.env` 里的一个拼写错误就不会彻底损坏这个守护进程。

| 变量 | 默认值 | 说明 |
|---|---|---|
| `WORKFLOW_POLL_INTERVAL_MS` | `1000` | 两轮空认领之间的休眠时长 |
| `WORKFLOW_CONCURRENCY` | `4` | 每个工作进程运行的工作流数量上限（最小 1） |
| `WORKFLOW_LOCK_TIMEOUT_SECS` | `30` | 另一个工作进程可以重新认领之前的租期时长 |
| `WORKFLOW_MAX_ATTEMPTS` | `3` | 每个工作流的尝试次数预算（最小 1） |
| `WORKFLOW_RETRY_BACKOFF_SECS` | `5` | 线性退避：`attempts * value`（最小 0） |

对于程序化的配置（在代码里构建，而不是从环境变量解析），在构造一个 `WorkflowWorker` 之前，调用 `WorkflowConfig::validate()` 来针对同样的这些不变量快速失败。

## 崩溃恢复

三层保护让工作流不会因为工作进程故障而卡死。

**Panic 边界。** 工作流的函数体运行在 `AssertUnwindSafe(...).catch_unwind()` 内部。任何一步里的一次 panic 都会被捕获，它的载荷会被捕获进错误列，这一行会经过和一次返回的 `Err` 一样的重试/失败结算流程。如果没有这道边界，一次 panic 就会跳过这条结算路径，让这一行永远停留在 `status='running'` 上。

**租约心跳。** 一个运行时间超过 `WORKFLOW_LOCK_TIMEOUT_SECS` 的长时间步骤，否则可能会在它自己还在运行的时候，租约就已经到期。工作进程会 spawn 出一个心跳任务，以锁超时时长的一半为间隔刷新 `locked_until`，直到函数体完成。这个心跳任务会在 `Drop` 时中止，所以一次返回的 `?` 不会让一个续约任务泄漏出去，冻住一个没有人在运行的工作流的租约。

**过期租约的重新认领。** 当一个工作进程在从未释放自己的锁的情况下死掉时（硬杀、主机崩溃、内核 OOM），这一行会停留在 `status='running'`，直到 `locked_until` 过去。这条认领查询会显式地把这样的行也纳入范围：任何租约已经过期的 `running` 工作流，都会在下一轮被另一个工作进程认领，`attempts` 也会随之递增。崩溃恢复是自动的 - 没有什么需要编写脚本，也没有需要记住的管理命令。

## 投递语义 - 至少一次

步骤体是以**至少一次**的语义运行的。一个步骤可能在两种情况下运行超过一次：

1. **返回了 `Err`** - 这个工作流会被重新入队；重试时，失败的那一步会再运行一次，而更早的每一步都会从缓存里重放。
2. **在副作用发生之后、`mark_step_succeeded` 提交之前崩溃** - 租约到期，另一个工作进程重新认领，在那个步骤索引上没看到已缓存的输出，于是再次运行这个函数体。

框架会持久化步骤的**输出**，但它没法观察到副作用本身。让步骤体保持幂等，是您自己的责任。几乎每种情形都能靠两种模式解决。

**条件写入。** 使用 `INSERT ... ON CONFLICT DO NOTHING`、幂等键列，或者 `seen_event_id` 标记。从已经在作用域内的数据里，派生出一个稳定的、逐步骤的键：工作流的输入参数，加上一个字面的步骤标签（`("wf-charge", customer_id)`），就足够了，因为同样的参数，在多次重试之间会映射到同一行工作流记录上。

**外部幂等键。** 大多数第三方 API（Stripe、SES、SQS）都接受一个 `Idempotency-Key` 请求头。传一个由工作流的输入加上一个步骤本地标签派生出的键（`format!("wf-charge-{}", customer_id)`），这样被重试的请求就能在提供方那一侧去重。

**不要**假定一个返回了 `Ok` 的步骤就不会再运行第二次 - 一次崩溃可能会把这第二次运行落在任何后续的工作进程上，包括在另一台主机上重启之后。关于 `Idempotency::once`、`Idempotency::commit_on_success` 和 `Idempotency::remember` - 这些都是包在一个步骤体外面的有效包装器 - 请参见[幂等性](idempotency.md)一章。

## 确定性契约

工作流必须在多次重放之间保持确定性。每一步都以 `(step_name, step_index)` 为键，框架会把它序列化后的输入和输出一起缓存下来。当同一个索引上的一个步骤，用一份不同的序列化输入被重放时，框架会返回一个错误，而不是靠返回那份旧输入对应的缓存输出来掩盖这次损坏。

在实践中这意味着：

- 不要在 `#[workflow_step]` 之外，根据 `Utc::now()`、`rand::random()`，或者其他非确定性的来源来分支。步骤体内部可以自由调用它们 - 它们的结果会被捕获进步骤输出缓存。
- 不要有条件地插入步骤。如果一次重试在到达某个给定索引之前遇到了不同数量的步骤，您会得到一个步骤名不匹配的错误。把分支逻辑放进一个步骤内部。
- 不要在两次部署之间改变步骤的参数形状，除非同时重命名这个步骤。重命名会改变 `step_name`，这会让这个步骤的缓存从头开始。

## 等待结果

`WorkflowHandle` 让调用方可以轮询这一行、等待它完成，或者取回序列化后的输出。

```rust
use std::time::Duration;
use suprnova::{FrameworkError, WorkflowStatus};

let handle = start_workflow!(welcome_flow, 123).await?;

match handle.wait_with_timeout(Duration::from_secs(30)).await {
    Ok(WorkflowStatus::Succeeded) => { /* 完成 */ }
    Ok(WorkflowStatus::Failed) => { /* 已持久化的错误列 */ }
    Ok(_) => unreachable!("wait_* only returns terminal status"),
    Err(FrameworkError::Internal { message }) if message.contains("Timed out") => {
        // 工作流仍在运行中；回退到异步 UX 路径。
    }
    Err(other) => return Err(other),
}
```

`wait()` 会无限期地轮询 - 只在测试里，或者永远阻塞是可以接受的短生命周期脚本里使用它。对于 HTTP 请求路径，`wait_with_timeout(Duration)` 总是会赢过内部的轮询循环，即便底层的状态查询卡住了也一样。一次超时错误**不会**取消这个工作流 - 工作进程会继续运行，`handle.status().await` 之后会返回实时的状态。

当默认值不合适时，`wait_with_options(Some(poll), Some(deadline))` 会把这两个旋钮都暴露出来。

对于类型化的输出，在这个工作流上定义一个 `T: Serialize + DeserializeOwned` 的返回类型，并调用 `handle.output::<T>().await?`。原始的 JSON 可以通过 `output_raw()` 拿到。

## 步骤缓存详解

步骤缓存以**步骤名 + 步骤索引**为键。一个步骤的第一次调用会持久化它的输入 JSON，运行函数体，并在成功时持久化输出 JSON。在同一个索引上的一次重放：

- 如果这个步骤是 `succeeded`，并且被重放的输入与缓存的输入匹配，就返回缓存的输出。
- 如果输入不同，就返回一个错误（这是确定性守卫）。
- 如果这个步骤是 `running` 或 `failed`（没有可返回的缓存输出），就重新运行函数体。

步骤索引由每个工作流上下文里的一个 `AtomicI32` 分配，所以顺序是由您工作流函数体所做的那些调用决定的。如果分支在一次重试中，在同一个索引上产生了一个不同的步骤，这会表现为一个步骤名不匹配的错误，而不是静默地损坏下游的步骤。

输出和输入都以 JSON 文本的形式存储，所以每一个步骤的返回类型和参数都必须是 `Serialize + DeserializeOwned`。

## 从一个辅助函数里探测工作流上下文

`WorkflowContext::is_active()` 返回当前任务是否运行在一个工作流之下。在那些需要在工作进程内部和外部表现不同的辅助函数里用它 - 比如说，一个只在工作流存在时才附上工作流标签的日志记录器：

```rust
use suprnova::workflow::WorkflowContext;

fn maybe_workflow_tagged(message: &str) -> String {
    if WorkflowContext::is_active() {
        format!("[workflow] {message}")
    } else {
        message.to_string()
    }
}
```

在一个工作流之外（直接从一个测试或处理程序里调用），一个 `#[workflow_step]` 函数仍然会运行 - `WorkflowContext::current()` 只是会返回 `None`，函数体不带持久化地执行，这一步会完全绕过缓存。这是故意的：它让步骤函数无需拉起一个工作进程就能被单独测试。

### 为什么 Suprnova 有所不同

Laravel 没有一个头等的工作流原语 - 作业是最接近的邻居，但它们的重试方式是重新运行整个作业体，而不是从最后一个成功的步骤恢复。Suprnova 把工作流作为一个独立的构造发布出来，因为 Tokio 让“在一个缓慢的异步函数里挂靠一个小时”这种模式变得成本低廉，也因为对于任何多步骤的外部交互（为一个客户开通服务、跨两个支付提供商运行一次 Saga、生成一份涉及好几个上游 API 的报表），逐步骤的持久化都是正确的抽象。

这个设计更接近 [DBOS](https://www.dbos.dev/) 和 Cadence/Temporal，而不是一个队列：持久化的状态、确定性的重放、显式的步骤边界。和 Temporal 的区别在于运维上的重量 - 这里没有一个需要单独运行的工作流服务；这个工作进程就是针对您应用数据库运行的 `suprnova workflow:work`。

## 说明

- 步骤体可以返回任何 `Serialize + DeserializeOwned` 类型。对于那些只为了自己的副作用而存在的步骤，`()` 单元类型就够用。
- 在一个工作流上下文之外调用的 `#[workflow_step]` 函数会内联运行 - 没有缓存，没有重放。测试正是这样直接练习步骤体的。
- 步骤缓存以 `(step_name, step_index)` 为键；重命名一个步骤（或者重新排列调用顺序），这个步骤的缓存就会在下一次重放时重置。
- `start_workflow!` 接受任何一个由可序列化参数组成的元组。元组会保留参数顺序，所以重命名位置参数是安全的；改变参数类型则会给任何飞行中的工作流带来一次架构破坏。
- 框架的[可观测性](observability.md)层会在每一条结算路径上，捕获工作进程的结构化日志（`worker_id`、`workflow_id`、`attempts`、`max_attempts`），这样您就能在生产环境里审计重试预算，而不需要在您的步骤里加插桩。

## 下一步

- [队列](queues.md) - 使用 sync/redis/database 驱动程序的一次性后台作业
- [幂等性](idempotency.md) - 面向至少一次投递的包装器
- [总线](bus.md) - 带类型化结果的同步命令分发
- [监督程序](supervisors.md) - 带 panic 捕获自动重启的长期存活任务监督
- [错误模型](error-model.md) - `FrameworkError`、panic 边界，以及为什么结算要通过 `?` 来运行
