# 验证

Suprnova 在两条互补的轨道上验证请求输入：

1. **derive 验证** - `FormRequest` 结构体上的 `#[validate(...)]` 属性，由 `extract()` 自动运行。这是日常使用的路径，在[请求](requests.md)一章里有介绍。它以声明式的方式处理逐字段的规则（`email`、`length`、`range`、…）。
2. **规则对象 + `validate!` 宏** - 实现了 [`Rule`](#规则对象) / `ContextualRule` / `AsyncRule` 的普通值，以命令式的方式组合起来。当您需要跨字段逻辑、需要访问数据库的规则，或者需要把规则存起来四处传递时，就用这一套。

两条轨道会累积进同一个 [`ValidationErrors`](error-model.md) 错误包，并渲染出同一种 Laravel/Inertia 的 `{ "message", "errors": { field: [...] } }` 形状（HTTP 422）。

## 规则对象

规则就是一个实现了以下三个 trait 之一的值：

| Trait | 形态 | 用途 |
|-------|-------|-----|
| `Rule` | `passes(&self, value: &str)` | 对单个值做纯粹的检查 |
| `ContextualRule` | `passes(&self, value, ctx)` | 需要读取兄弟字段的检查 |
| `AsyncRule` | `async passes(&self, value)` | 需要 `.await` 的检查（数据库、HTTP） |

内置的 `Rule`：`Required`、`Email`、`Min`、`Max`、`Between`、`In`、`NotIn`、`Integer`、`Numeric`、`Boolean`、`Alpha`、`AlphaNum`、`Url`、`HttpUrl`、`Uuid`。内置的 `ContextualRule`：`RequiredIf`、`RequiredWith`、`RequiredUnless`、`Same`、`Different`、`Confirmed`。内置的 `AsyncRule`：[`Unique`](#unique-规则)。

```rust
use suprnova::{Rule, rules::Email};

Email.passes("user@example.com")?; // Ok(())
```

> **注意：** `Numeric` 只接受**有限**的数字 - `NaN`、`inf`，以及那些溢出成无穷大的量级都会被拒绝，尽管 Rust 的解析器本身会接受这些字符串。对于回调 / webhook / 头像这类输入，请使用 `HttpUrl`（而不是 `Url`）：`Url` 会解析 `url::Url` 所接受的任何协议（`file:`、`javascript:`、自定义 URI），而 `HttpUrl` 要求 `http`/`https`。

### 编写您自己的规则

一条自定义规则就是一个单元结构体（或者携带数据的结构体），配上一个 impl。这个 trait 免费给了您一个 `check()` - 它会把任何失败消息按指定的字段名推入一个 `ValidationErrors` 错误包 - 所以这条规则可以原封不动地接入 `validate!` 和 `after_validation` 钩子：

```rust
use suprnova::{Rule, ValidationMessage};

pub struct StartsWith(pub &'static str);

impl Rule for StartsWith {
    fn passes(&self, value: &str) -> Result<(), ValidationMessage> {
        if value.starts_with(self.0) {
            Ok(())
        } else {
            Err(format!("must start with {}", self.0).into())
        }
    }
}

// 现在到处都能用了：
StartsWith("acct_").passes("acct_1234")?;
// 或者，写成 validate! 里的一行：
//   stripe_id => Required, StartsWith("acct_");
```

一个 `String` 会转换成一条逐字渲染的 `ValidationMessage`，对单语言应用来说这就够了。若想让消息按语言区域被翻译，请改为返回一条*带键的*消息 - `ValidationMessage::keyed("validation-starts-with").arg("prefix", self.0).fallback(…)` - 并在 `lang/<locale>/validation.ftl` 里定义这个 id。请参见[本地化](localization.md)，那一章还讲了如何覆盖内置规则的消息，以及 `field-<name>` 命名约定。

如果需要跨字段逻辑，请改为实现 [`ContextualRule`] - 它的 `passes` 方法除了拿到被测试的值之外，还会拿到一个 `&FormContext`（一个由兄弟字段的值构成的 `HashMap<String, String>`）。如果检查需要访问数据库，请实现 [`AsyncRule`]，并从 `after_validation_async` 里使用它。

## `validate!` 宏

`validate!` 会在一个结构体的各个字段上运行一串规则，把每一次失败都累积进同一个 `ValidationErrors`。它是同步的跨字段钩子 [`after_validation`](#跨字段钩子) 最地道的落脚处。

```rust
use suprnova::{validate, ValidationErrors, rules::{Required, Email, Min, Max, RequiredIf}};

fn after_validation(&self) -> Result<(), ValidationErrors> {
    // 上下文规则会从您自己构建的 `FormContext` 里读取兄弟字段的值
    // - 一个从字段名到其字符串值的映射。
    let mut ctx = std::collections::HashMap::new();
    ctx.insert("billing_type".to_string(), self.billing_type.clone());
    validate! { self =>
        email       => Required, Email;          // 必填形态的行
        bio         ?: Min(10), Max(500);        // 可选：只在是 Some 时才验证
        card_number ?=> RequiredIf {             // 条件性存在（见下文）
            other: "billing_type",
            value: "card",
        } => with ctx;
    }
}
```

每一行都是以下三种形态之一：

- **`field => Rule1, Rule2;`** - 必填形态。规则直接在 `&self.field` 上运行（适用于 `String`、`i64`，或者任何能解引用成规则所期望的那个借用的东西）。
- **`field ?: Rule1, Rule2;`** - 可选。这个字段是 `Option<T>`；规则只在它是 `Some` 时才运行，而在 `None` 上**会被完全跳过**。这就是 Laravel 的“存在才验证”（`sometimes`）语义。
- **`field ?=> Rule1, Rule2;`** - 条件性存在。同样用于 `Option<String>` 字段，但规则**即使在 `None` 时也会运行**（缺失会被当作空字符串处理）。像 `RequiredIf` 这类依赖存在与否的规则要用这种行，因为它们必须能够*让一个缺失的字段失败* - 而这正是 `?:` 表达不了的情形，因为它在 `None` 上会跳过。

上下文规则后面要跟上 `=> with $ctx`（一个由兄弟字段的值构成的 `&HashMap<String, String>`）。这个宏是**同步的** - 异步规则请使用下面的[钩子](#请求中的异步规则)。

> **警告：** 一个常见的陷阱：写成 `card_number ?: RequiredIf {...} => with ctx;`。在 `?:` 的行上，`None` 会跳过所有规则，所以 `RequiredIf` 永远无法让一个缺失的字段失败。任何必须在缺失时触发的规则，都要用 `?=>`。

## 跨字段钩子

在 derive 出来的逐字段规则之后，`FormRequest` 会运行两个跨字段钩子，在普通流程和 Precognition 流程里都是如此。`extract()` 会按顺序运行这些阶段 - 先是 derive 出来的 `validate()`，然后是 `after_validation`，再然后是 `after_validation_async` - 并且**会在第一个失败的阶段就退出**。

```rust
use suprnova::{FormRequest, ValidationErrors};
use serde::Deserialize;
use validator::Validate;

#[derive(Deserialize, Validate)]
pub struct UpdatePassword {
    #[validate(length(min = 8))]
    pub new_password: String,
    pub confirmation: String,
}

impl FormRequest for UpdatePassword {
    fn after_validation(&self) -> Result<(), ValidationErrors> {
        let mut errs = ValidationErrors::new();
        if self.new_password != self.confirmation {
            errs.add("confirmation", "passwords do not match");
        }
        errs.into_result()
    }
}
```

> **注意：** 覆盖钩子需要一个手写的 `impl FormRequest` - `#[request]` 属性和 `#[derive(FormRequest)]` 会各自生成它们自己的（空）实现，所以它们只适用于常见的那种不做覆盖的情形。

### 请求中的异步规则

`validate!` 宏没法把 `.await` 编织进去，所以依赖数据库的规则要放在 `after_validation_async` 里运行 - 那是验证的最后一个阶段，`extract()` 会自动调用它。[`Unique`](#unique-规则) 和任何自定义的 `AsyncRule` 都是在这里参与到自动的请求验证中的；不需要在每个处理程序里手工接线。

```rust
use suprnova::{FormRequest, ValidationErrors, Unique, async_trait};
use serde::Deserialize;
use validator::Validate;

#[derive(Deserialize, Validate)]
pub struct CreateUser {
    #[validate(email)]
    pub email: String,
}

#[async_trait]
impl FormRequest for CreateUser {
    async fn after_validation_async(&self) -> Result<(), ValidationErrors> {
        let mut errs = ValidationErrors::new();
        Unique::new("users", "email")
            .check_async(&self.email, &mut errs, "email")
            .await;
        errs.into_result()
    }
}
```

因为异步阶段只在同步阶段通过之后才运行，所以一个格式错误的值（一个语法上无效的邮箱）永远走不到数据库上的 `Unique` 查询。

## `Unique` 规则

`Unique` 检查某个值在一张表里尚不存在。用 `Unique::new(table, column)` 构建它，再用流式 API 细化：

```rust
use suprnova::Unique;

// email 必须唯一，但忽略当前正在编辑的那一行
Unique::new("users", "email").ignore(current_user_id)

// email *按租户*唯一，并且比较时不区分大小写
Unique::new("users", "email")
    .where_eq("tenant_id", tenant_id)
    .case_insensitive()
```

| 构建器方法 | 效果 |
|----------------|--------|
| `.ignore(id)` | 排除 `id` 等于 `id` 的那一行（编辑自身的情形） |
| `.ignore_with_column(col, id)` | 在一个非 `id` 的键列上做排除 |
| `.where_eq(col, value)` | 把检查限定在 `col = value` 的那些行上；多次调用之间是 AND 关系 |
| `.case_insensitive()` | 用 `LOWER(col) = LOWER(?)` 来比较 |

表名、列名、排除用的键，以及每一个 `where_eq` 的列名，在进入 SQL 字符串之前都会先对照一份标识符允许列表做校验；被测试的值和所有限定范围的值都是绑定参数。

### Unique 是建议性的 - 数据库约束才是保证

`Unique` 会在写入**之前**运行一次 `SELECT COUNT(*)`，所以它带着一个无法避免的“检查时刻 / 使用时刻”竞态：两个并发请求可以双双通过检查，然后双双插入。Laravel 的 `unique` 规则也有完全相同的性质。**唯一**真正的保证，是在您的迁移里给这一列加上 `UNIQUE` 约束（或者唯一索引）。

把这三者一起用：

1. **建议性的规则** - 在提交之前给出一条快速、友好的“这个邮箱已被占用”消息（也让 Precognition 能验证这个字段）。
2. **`UNIQUE` 约束** - 针对这个竞态的权威把关。
3. **`FrameworkError::from_unique_violation`** - 在写入的地方，把竞态中落败的那一方收到的约束冲突映射回同样干净的 422，而不是泄漏出一个 500：

```rust
use suprnova::FrameworkError;

// `users.email` 在迁移里有一个 UNIQUE 约束。
let user = new_user
    .insert(db)
    .await
    .map_err(|e| FrameworkError::from_unique_violation(
        "email",
        "That email address is already registered.",
        e,
    ))?;
```

当数据库错误是一次唯一约束冲突时，`from_unique_violation` 会返回一个 422 的 `Validation` 错误；其他任何错误都会原样透传（MySQL、Postgres 和 SQLite 都能被识别）。

## 异步授权

`FormRequest::authorize(&Request) -> bool` 在请求体被解析**之前**运行，所以它可以在不读取载荷的情况下拒绝未授权的请求。它被设计成同步的：在那个时刻请求仍然持有着流式的请求体，所以这个钩子无法 `.await`。那些需要访问数据库或者异步策略的授权，属于下面这几个地方，而不属于 `authorize`：

- **中间件** - 在 `extract()` 之前运行，是 `async` 的，并且通过返回 `Err(response)` 来短路（参见[中间件](middleware.md)）。这里是回答“这个用户究竟允不允许到达这条路由”的正确位置。
- **Gate** - 一旦在处理程序里拿到了已认证的用户和资源，就调用 `Gate::allows_async` / `Gate::authorize_async`（参见[授权](authorization.md)）。
- **`after_validation_async`** - 如果一次授权检查依赖于解析之后的请求体，就把它和您其他的异步规则一起放在这个异步钩子里运行。

## 设计说明

- **部分验证。** `FormRequest` 会在验证运行之前反序列化成一个类型化的结构体，所以这个结构体*就是*架构：一个可能缺失的字段必须是 `Option<T>`。这也正是 Precognition 能够验证部分载荷的原因 - 把草稿可以省略的那些字段设为可选。
- **规则消息。** 内置规则返回的是带键的消息（`validation-min` 加上它的参数和一个英文兜底），在序列化边界上通过 Fluent 消息目录解析出来。想翻译或者改写其中任何一条，只需在 `lang/<locale>/validation.ftl` 里定义同一个 id - 不需要包装规则。请参见[本地化](localization.md)。
- **`Min` / `Max` / `Between`** 是字符串长度规则（按 Unicode 标量值计数）。要做数值上下界，请用 derive 上的 `#[validate(range(...))]` 或者一条自定义规则来验证 - 长度规则不是值的比较。

## 总结

| 任务 | API |
|------|-----|
| 逐字段规则 | `FormRequest` 上的 `#[validate(...)]`（参见“请求”一章） |
| 组合规则 / 跨字段规则 | `validate! { self => ... }` |
| 可选的“存在才验证” | `field ?: Rule;` |
| 有条件必填的可选字段 | `field ?=> Rule => with ctx;` |
| 异步 / 依赖数据库的规则 | `after_validation_async` + `AsyncRule::check_async` |
| 唯一性 | `Unique::new(t, c)` + `UNIQUE` 约束 + `from_unique_violation` |
| 异步授权 | 中间件 / `Gate::*_async` / `after_validation_async` |

## 下一步

- [请求](requests.md) - `#[request]` / `#[derive(FormRequest)]` 这套表面，也就是日常使用的 derive 验证路径
- [数据对象](data.md) - `#[derive(Data, Validate)]`，让同一个结构体既是入站的请求又是出站的 DTO
- [错误模型](error-model.md) - `ValidationErrors` 如何变成那个 422 JSON 响应体，以及其他每一条错误路径
- [本地化](localization.md) - 翻译规则消息、`field-<name>` 约定，以及带键的 `ValidationMessage`
- [授权](authorization.md) - `Gate`、`Policy`，以及授权相对于验证应该放在哪里
- [中间件](middleware.md) - 那些需要 `.await` 的“这个请求究竟允不允许通过”检查的正确位置
