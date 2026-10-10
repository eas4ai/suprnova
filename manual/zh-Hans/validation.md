# 验证

Suprnova 在两条互补的轨道上验证请求输入：

1. **derive 验证** - `FormRequest` 结构体上的 `#[validate(...)]` 属性，由 `extract()` 自动运行。这是日常使用的路径，在[请求](requests.md)一章里有介绍。它以声明式的方式处理逐字段的规则（`email`、`length`、`range`、…）。
2. **规则对象 + `validate!` 宏** - 实现 [`Rule`](#规则对象) / `ValueRule` / `ContextualRule` / `AsyncRule` 的普通值，以命令式的方式组合起来。当您需要跨字段逻辑、需要访问数据库的规则，或者需要把规则存起来四处传递时，就用这一套。

两条轨道会累积进同一个 [`ValidationErrors`](error-model.md) 错误包，并渲染出同一种 Laravel/Inertia 的 `{ "message", "errors": { field: [...] } }` 形状（HTTP 422）。

## 规则对象

一条规则，就是一个实现了下面四个 trait 之一的值：

| Trait | 形态 | 用途 |
|-------|-------|-----|
| `Rule` | `passes(&self, value: &str)` | 对单个值的纯粹检查 |
| `ValueRule` | `passes(&self, value: &serde_json::Value)` | 对 JSON 形状的值（数组/对象）进行检查 |
| `ContextualRule` | `passes(&self, value, ctx)` | 会读取兄弟字段的检查 |
| `AsyncRule` | `async passes(&self, value)` | 会 `.await` 的检查（数据库、HTTP） |

内置的 `Rule`：`Required`、`Email`、`Min`、`Max`、`Between`、`In`、`NotIn`、`InArray`、`Integer`、`Numeric`、`Boolean`、`Alpha`、`AlphaNum`、`AlphaDash`、`Url`、`UrlProtocols`、`HttpUrl`、`Uuid`、[`Password`](#密码强度)（只做强度检查）。内置的 `ValueRule`：`ArrayKeys`、`Distinct`、`Contains`、`DoesntContain`。内置的 `ContextualRule`：`RequiredIf`、`RequiredWith`、`RequiredUnless`、`Same`、`Different`、`Confirmed`、`Gt`、`Gte`、`Lt`、`Lte`。内置的 `AsyncRule`：[`Unique`](#unique-规则) 和 [`Password`](#密码强度)（强度，外加它那项 `uncompromised()` HIBP 检查 - 唯一一个同时实现了 `Rule` 和 `AsyncRule` 的内置规则）。

```rust
use suprnova::{Rule, rules::Email};

Email.passes("user@example.com")?; // Ok(())
```

> **注意：** `Numeric` 接受的是一个**有限**的数 - `NaN`、`inf`，以及那些溢出成无穷大的量值都会被拒绝，尽管 Rust 的解析器会接受这些字符串。

### URL 协议方案

`Url` 接受的值，必须能被解析成一个 URL，它的协议方案在 Laravel 的允许列表上 - 也就是 `Illuminate\Support\Str::isUrl` 用的那份列表 - 后面跟着 `://`，**并且**再往后跟着一个非空的主机，在形态上与 Laravel 的 `^(PROTOCOLS)://HOST` 模式一致（Laravel 的主机分组没有 `?` - 缺失或为空的主机永远不会匹配）。协议方案列表以及“`://` 加主机”这条要求都逐字取自 Laravel；主机由 `url` crate 解析，而不是由 Laravel 的正则解析，所以一个超出范围的端口在这里会被拒绝，而 Laravel 会接受它。这三条都必须成立：`mailto:`、`tel:` 和 `data:` 按名字在允许列表上，却根本不携带 authority 组成部分，所以 `Url` 拒绝它们；而 `file:///etc/passwd` 是因为第三条而失败的 - 它有 `://`，但第三个和第四个 `/` 之间什么都没有，而“什么都没有”不是一个主机。`javascript:` 和 `vbscript:` 会被直接拒绝；它们压根就不在允许列表上。

`ftp://host/x` 和 `ssh://host` - 真实的主机，只是不是 Web 协议方案 - 仍然会通过，所以 `Url` 不是一个“这是一个网页”的检查，它也完全没有说这个 URL 会解析到哪里去。拒绝 `javascript:` 让一个通过验证的值可以安全地放进 `href`，但不代表可以安全地去获取它。一个 webhook 或者回调目标仍然需要 `HttpUrl`（或者您自己的协议方案 + SSRF 检查）；光靠 `Url` 覆盖不了那件事。

想要一个更窄的集合，就把您想要的协议方案点出来：

```rust
use suprnova::{Rule, rules::Url};

// Laravel 的 `url:http,https`
Url::protocols(&["https"]).passes("https://example.com")?;   // Ok
Url::protocols(&["https"]).passes("http://example.com");     // Err

// 同一件事，换个名字
use suprnova::rules::HttpUrl;
HttpUrl.passes("https://example.com")?;
```

`Url::protocols(...)` 是**替换**这份允许列表，而不是收窄它，所以一个应用可以接受自己的深链协议方案（`myapp://…`），而框架对此不持任何意见 - “`://` 加主机”这条要求对那个自定义协议方案同样适用。对于回调、webhook 和头像这类输入，请用 `HttpUrl`（或者 `Url::protocols(&["https"])`） - 一个解析到 `ftp://internal-host/` 的 webhook 目标，仍然能被解析成一个 `Url`，而一个 `ftp:` 目标不是一个 webhook 目标。

### 密码强度

`Password` 会检查长度和字符类别的强度，外加一项可选的 Have I Been Pwned `uncompromised()` 检查 - 它是 Laravel 那个 `Password` 规则对象的移植。用 `Password::min(n)` 把它构建出来，再链上那些强度构建器：

```rust
use suprnova::{Password, Rule};

let rule = Password::min(8).letters().mixed_case().numbers().symbols();
Rule::passes(&rule, "Str0ng! Pass")?; // Ok(())
Rule::passes(&rule, "weak");          // Err - 太短，没有数字，也没有符号
```

| 构建器 | 要求 | Laravel 的正则 |
|---|---|---|
| `.min(n)`（通过 `Password::min`） | 至少 `n` 个字符（下限为 1） | 长度检查 |
| `.max(n)` | 至多 `n` 个字符 | 长度检查 |
| `.letters()` | 至少一个 Unicode 字母 | `/\pL/u` |
| `.mixed_case()` | 一个大写字母和一个小写字母，先后不限 | `/(\p{Ll}+.*\p{Lu})\|(\p{Lu}+.*\p{Ll})/u` |
| `.numbers()` | 至少一个 Unicode 数字 | `/\pN/u` |
| `.symbols()` | 至少一个分隔符、符号或标点字符 - **一个普通空格也算** | `/\p{Z}\|\p{S}\|\p{P}/u` |

`Password::defaults_with(|| Password::min(12).letters().mixed_case().numbers())` 从 `bootstrap::register()` 里调用一次，就设定了 `Password::defaults()` 在别处返回的那个进程级默认值 - 对应 Laravel 的 `Password::defaults(fn () => ...)`。第二次调用会被忽略（并附带一条 `tracing::warn!`），而不是悄悄替换掉这个应用第一次选定的那份策略。

#### `uncompromised()` - 因为光有强度还不够

`.uncompromised()`（或者 `.uncompromised_with_threshold(n)`）会加上一项针对 Have I Been Pwned 泄露语料库的检查，用的是它的 k-匿名 range API：只有密码那份大写 SHA-1 哈希的**前 5 个字符**会离开进程 - `GET https://api.pwnedpasswords.com/range/{prefix}` - 而与完整哈希的比对是在本地做的，比对的是这个 API 为那个前缀返回的那些 `SUFFIX:COUNT` 行。这项服务从来看不到密码，甚至看不到它的完整哈希。阈值比较是严格的（`count > threshold`），所以默认的 `uncompromised()`（阈值 `0`）只要它出现过一次就会失败；而一次网络故障、超时或者非 2xx 响应会**失败开放** - 这个密码会被当作干净的，而不是在 Have I Been Pwned 故障期间把每一次注册都挡下来。这与 Laravel 的 `NotPwnedVerifier` 完全一致。

因为那项检查是一次 HTTP 往返，`uncompromised()` 需要的是 `AsyncRule`，而不是只做强度检查时就够用的那个同步 `Rule`。请通过 `after_validation_async` 把它接起来，配方和 [`Unique`](#unique-规则) 用的那份一样：

```rust
use suprnova::{AsyncRule, FormRequest, Password, ValidationErrors, async_trait};
use serde::Deserialize;
use validator::Validate;

#[derive(Deserialize, Validate)]
pub struct Register {
    pub password: String,
}

#[async_trait]
impl FormRequest for Register {
    async fn after_validation_async(&self) -> Result<(), ValidationErrors> {
        let mut errs = ValidationErrors::new();
        Password::defaults()
            .uncompromised()
            .check_async(&self.password, &mut errs, "password")
            .await;
        errs.into_result()
    }
}
```

在一个设了 `uncompromised()` 的 `Password` 上调用同步的 `Rule::passes`，会是一个**醒目的错误**，而不是一次悄无声息的跳过 - 一项悄悄什么都不做的安全检查，比一项从未存在过的更糟。这条错误消息会点名 `after_validation_async` 作为修复办法。

`HIBP_TIMEOUT_SECS`（默认 `30`）控制的是请求超时 - 参见[环境变量](env-vars.md)。

一个返回 `Err` 的自定义校验器，和一次失败的检查是两回事：它的错误文本会以 `error` 级别记进日志，绝不会到达客户端，而响应携带的是 `validation-password-unverifiable` 这个语料表键（“The { $field } could not be checked against known data leaks. Please try again.”）。如果您自带一份校验语料表，请把这个键加上。

### 为什么 Suprnova 有所不同：Password

- Laravel 的 `Password` 会把每一项失败的强度检查都收进同一个数组里。Suprnova 的 `Rule` 契约返回的是单条 `ValidationMessage`，所以 `Rule::passes` 报告的是**第一项**失败的检查，顺序是 min、max、大小写混合、字母、符号、数字 - 一次修一个，而不是一上来就看到整张清单。
- Laravel 的同步校验器可以直接调用 `uncompromised()`；一个 PHP 请求本来就处在一个能容忍阻塞式 HTTP 调用的事件循环里。Suprnova 的 `Rule::passes` 按契约是同步的，所以没有哪个安全的地方能从它里面去发那个 HIBP 请求。与其悄悄跳过这项检查 - 对一条与安全相关的规则来说，那是唯一不可接受的结局 - Suprnova 的 `Rule::passes` 会返回一个醒目的、面向开发者的错误，点名 `after_validation_async` 作为修复办法。
- `Password::defaults_with` 接受的是一个朴素的 `fn` 指针，而不是一个闭包，这样被配置的那个默认值就保持 `Copy`、也不需要堆分配 - 这是相对 Laravel 的 `Closure` 的一次刻意收窄。

### 编写您自己的规则

一条自定义规则，就是一个带着一份 impl 的单元结构体（或者携带数据的结构体）。这个 trait 免费给了您 `check()` - 它会把任何失败消息压进一个 `ValidationErrors` 包里、放在被点名的那个字段下 - 所以这条规则可以原封不动地接进 `validate!` 和 `after_validation` 钩子：

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
// 或者，写在一行 validate! 里：
//   stripe_id => Required, StartsWith("acct_");
```

一个 `String` 会转换成一个原样渲染的 `ValidationMessage`，对一个单语言应用来说这就够了。要让这条消息按语言区域被翻译，请改为返回一条*带键*的消息 - `ValidationMessage::keyed("validation-starts-with").arg("prefix", self.0).fallback(…)` - 并在 `lang/<locale>/validation.ftl` 里定义这个 id。参见[本地化](localization.md)，那一章还讲了怎样覆盖内置规则的消息，以及 `field-<name>` 这个命名约定。

要写跨字段逻辑，请改为实现 [`ContextualRule`] - 它的 `passes` 方法除了拿到被检查的值，还会拿到一个 `&FormContext`（一个装着兄弟字段值的 `HashMap<String, String>`）。要做由数据库支撑的检查，请实现 [`AsyncRule`]，并从 `after_validation_async` 里使用它。

### 值形状规则

`Rule` 只会看到 `&str`。两个内置规则需要字符串以外的更多结构，因此它们通过 `&serde_json::Value` 改为实现 `ValueRule`：

```rust
use suprnova::{ValueRule, rules::{ArrayKeys, Distinct}};

// Laravel 的 array:keys - 拒绝允许集合之外的键。列出的键不一定都要出现；
// 空允许列表是编程错误，会报出无键消息。
ArrayKeys(&["name", "email"]).passes(&serde_json::json!({"name": "Ada"}))?;

// Laravel 的 distinct / distinct:ignore_case / distinct:strict。
Distinct { ignore_case: false, strict: false }
    .passes(&serde_json::json!(["a", "b", "c"]))?;
```

由 `ValueRule` 验证的字段必须直接持有 `serde_json::Value`（对于 `?:`/`?=>` 行则是 `Option<serde_json::Value>`）- 通常是直接从 JSON 请求体提取的请求字段。`validate!` 行接受同一字段列表中的 `Rule` 和 `ValueRule`；运行哪个 trait 由规则类型实现的 trait 决定，而不是由您在行中写入的内容决定。

### 成员资格规则

有三条规则回答“这个值在那个列表里吗？”，每一条都作用在它所需要的那种形状上：

```rust
use suprnova::{Rule, ValueRule, rules::{Contains, DoesntContain, InArray}};

// Laravel 的 in_array:allowed_roles.* - 这个值必须出现在另一个字段的列表里。
// 请把那个列表本身传进来：一个 Vec<String> 字段和一个 &[&str] 字面量都可以。
InArray(&form.allowed_roles).passes(&form.role)?;

// Laravel 的 contains:rust,web - 这个数组必须持有列出的每一个值。
Contains(&["rust", "web"]).passes(&form.tags)?;

// Laravel 的 doesnt_contain:banned - 这个数组必须一个都不持有。
DoesntContain(&["banned"]).passes(&form.tags)?;
```

每一次比较都是精确的。`InArray` 用 `==` 比较字符串，而 `Contains` 和 `DoesntContain` 只把一个参数与 JSON 的字符串元素相匹配，所以 `["1"]` 含有 `"1"`，而 `[1]` 没有。一个不是数组的值会直接让 `Contains` 和 `DoesntContain` 失败。

`Contains` 和 `DoesntContain` 会把一个空的参数列表当作一个无键的构造错误拒绝掉，和 `ArrayKeys` 的做法一样 - 一个什么都没装的列表约束不了任何东西。一个空的 `InArray` 待查列表则是另一回事：一个兄弟字段在运行时完全可能理所当然地为空，所以这个值就是单纯地失败。

`InArray` 的失败消息不点任何值的名，因为它那个列表是从请求里来的，而一条验证消息是要被渲染进一个响应体里的。

### 比较规则

`Gt`、`Gte`、`Lt` 和 `Lte` 把一个字段与一个数字、或者与另一个字段做比较。`CompareWith` 把操作数和度量方式一并点了出来：

```rust
use suprnova::{ContextualRule, FormContext, rules::{CompareWith, Gt, Lte}};

let mut ctx = FormContext::new();
ctx.insert("max_price".to_string(), form.max_price.clone());

// Laravel 的 gt:0 - 一个字面操作数，按数值比较。
Gt(CompareWith::Number(0.0)).passes(&form.price, &ctx)?;

// Laravel 的 lte:max_price - 一个兄弟字段，按数值比较。
Lte(CompareWith::NumericField("max_price")).passes(&form.price, &ctx)?;

// Laravel 在两个字符串字段上的 gt:summary - 按字符数比较。
Gt(CompareWith::LengthField("summary")).passes(&form.body, &ctx)?;
```

这四条都会读取兄弟字段，所以它们是 `ContextualRule`，而且每一个 `validate!` 行都要带上 `=> with ctx` - 包括那种唯一的操作数是一个字面量、上下文根本不会被读的行。那种情况请传一个空的 `FormContext` 进去。

任何这条规则量不出来的东西，都会让这个字段失败：一个在数值比较之下并不是有限数的值、一个表单从未发送过的兄弟字段、一个不是数字的兄弟字段，或者一个像 `f64::NAN` 这样的非有限字面量。这里面没有一样会 panic，也没有一样会通过。

### 为什么 Suprnova 有所不同

Laravel 的 `distinct:strict` 依赖 PHP 会进行强制转换的 `==`。JSON 值已具有类型，因此 Suprnova 的 `strict` 仅改变内部表示不同的两个*数字*（`1` 与 `1.0`）是否计作相等 - 它在两种模式下都不会让字符串和数字“相同”。

Laravel 是把另一个字段写进一个规则字符串里的 - `in_array:allowed_roles.*` - 再由验证器在运行时从请求数据里把它 glob 出来。Suprnova 没有规则字符串解析器：您直接把那个列表交给 `InArray`，而编译器会检查这个字段确实存在。

Laravel 13.27 把 `in`、`in_array` 和 `doesnt_contain` 收紧成了严格比较，因为 PHP 的 `==` 会把 `"1abc"`、`true` 和 `"0x1"` 都变成匹配。Suprnova 从来没有过那个漏洞 - `In` 和 `NotIn` 是用 `==` 比较 `&str` 的 - 而这几条新规则是逐个变体地匹配 JSON 值的。Laravel 的 `contains` 仍然是松散的；Suprnova 的不是。代价是这几条规则没法检查一个数值数组：`Contains(&["1"])` 不会匹配上 `[1]`。

Laravel 的 `gt` 家族是在运行时挑它的度量方式的：数值就用数字本身，数组用 `count()`，文件用千字节，其余一切用字符长度，而那条数值分支还取决于这个字段是否同时带着 `numeric` 或 `integer`。Suprnova 改为把度量方式写进规则里，因为这里的一条规则看不见它那个字段上的其他规则，而嗅探值的形态，正是这几条规则存在的意义所要避开的那种强制转换习惯。Laravel 那四种度量方式里有两种在这里根本没有对应物：一条规则永远只会收到一个字符串，所以一个数组值的兄弟字段读不出来；而上传永远到不了规则这个表面 - multipart 解析器早在处理程序看到它们之前，就已经给它们的大小设了上限。

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

- **`field => Rule1, Rule2;`** - 必填形态。规则直接在 `&self.field` 上运行（适用于 `String`、`i64`，或者任何能解引用成规则所期望的借用的东西）- 或者对于 `ValueRule`，直接在 `serde_json::Value` 字段上运行。每条规则使用哪个 trait 会自动推断。
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

## Inertia 表单提交

验证失败会对两类受众给出不同回答。REST 客户端会得到携带 `{ message, errors }` 的 `422`。Inertia 访问会获得重定向回表单页的 `303`，错误会以 flash 写入会话，因为 Inertia 客户端会对任何未识别为 Inertia 响应的响应显示错误模态框 - `422` 永远不会填充 `form.errors`。

处理程序无需变动。在目标页，每个字段都将第一个消息作为字符串携带：

```svelte
{#if errors?.email}
  <p class="text-red-600">{errors.email}</p>
{/if}
```

有关错误包、`with_all_errors` 和重定向指向的位置，请参见 [Inertia 响应](frontend-inertia-responses.md#validation-failures)。

## 设计说明

- **部分验证。** `FormRequest` 会在验证运行之前反序列化成一个类型化的结构体，所以这个结构体*就是*架构：一个可能缺失的字段必须是 `Option<T>`。这也正是 Precognition 能够验证部分载荷的原因 - 把草稿可以省略的那些字段设为可选。
- **规则消息。** 内置规则返回的是带键的消息（`validation-min` 加上它的参数和一个英文兜底），在序列化边界上通过 Fluent 消息目录解析出来。想翻译或者改写其中任何一条，只需在 `lang/<locale>/validation.ftl` 里定义同一个 id - 不需要包装规则。请参见[本地化](localization.md)。
- **`Min` / `Max` / `Between`** 是字符串长度规则（按 Unicode 标量值计数）。要做数值上下界，请用 derive 上的 `#[validate(range(...))]` 或者一条自定义规则来验证 - 长度规则不是值的比较。

## 总结

| 任务 | API |
|------|-----|
| 逐字段规则 | `FormRequest` 上的 `#[validate(...)]`（参见“请求”一章） |
| 组合规则 / 跨字段规则 | `validate! { self => ... }` |
| JSON 形状规则（数组/对象） | `field => ArrayKeys(&[...]);` / `field => Distinct { .. };` |
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
