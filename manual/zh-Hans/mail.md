# 邮件

Suprnova 的邮件子系统，在 Tokio 上镜照 Laravel 的 `Mail::to(...)->send(...)` API。一个 `Mail` 门面，八种传输层（面向开发/测试的 log 和内存、SMTP，以及五个 HTTP 提供商 - Postmark、SES、SendGrid、Mailgun、Resend），用 Tera 渲染模板、以 Mailable 序列化后的字段作为上下文，构建在持久的至少一次信封之上的队列 + 延迟投递，还有一个和 `Bus::fake()`、`Cache::fake()` 出自同一块布料的 `Mail::fake()` 测试守卫。

## 快速上手

```rust
use serde::{Deserialize, Serialize};
use suprnova::async_trait;
use suprnova::mail::{Address, Mail, Mailable};

#[derive(Serialize, Deserialize)]
struct Welcome {
    name: String,
}

#[async_trait]
impl Mailable for Welcome {
    fn mailable_name() -> &'static str { "Welcome" }
    fn subject(&self) -> String { format!("Welcome, {}", self.name) }
    fn text_template_source(&self) -> Option<String> {
        Some("Hi {{ name }}, welcome aboard.".into())
    }
    fn from(&self) -> Option<Address> {
        Some(Address::new("hello@example.com").with_name("Suprnova"))
    }
}

async fn greet(name: String) -> Result<(), suprnova::FrameworkError> {
    Mail::to("alice@example.org")
        .send(Welcome { name })
        .await
}
```

这个 Mailable 会序列化成 JSON，成为这个模板的 Tera 上下文；每一个 `pub` 字段都能以 `{{ field_name }}` 的形式触达。

## 配置

`Server::serve` 会在启动时调用一次 `suprnova::mail::boot::bootstrap_from_env()`。它会读取 `MAIL_DRIVER`，并绑定匹配的传输层。未设置时默认为 `log` 驱动程序。

| `MAIL_DRIVER` | 行为 |
|---------------|----------|
| `log`         | 像 Laravel 那样，每次发送都发出一条 `tracing::info!` - 信封和完整正文 - 然后丢弃。生产环境之外的默认值。 |
| `memory`      | 在进程内捕获每一条消息。参见 `suprnova::mail::boot::captured_in_memory()`。 |
| `smtp`        | 连接到一个 SMTP 服务器（设置了凭据时用 STARTTLS，否则用裸 TCP）。 |
| `postmark`    | 向 Postmark 的 `/email` 端点 POST JSON。 |
| `ses`         | 向 Amazon SES 的 `SendEmail` POST SigV4 签名的请求。 |
| `sendgrid`    | 向 SendGrid 的 `/v3/mail/send` POST JSON。 |
| `mailgun`     | 向 Mailgun 的 `/v3/{domain}/messages` POST `application/x-www-form-urlencoded`（有附件时则是 `multipart/form-data`）。 |
| `resend`      | 向 Resend 的 `/emails` POST JSON。 |

### 生产环境对一个会丢弃邮件的驱动程序失败关闭

`log` 和 `memory` 会渲染一条消息，然后丢弃它。在 `APP_ENV=production` 下，启动在这两者上都会**拒绝**：在一个未设置的 `MAIL_DRIVER`，或者一个构建无法识别的值上，也同样会拒绝，因为两者都会落到同一个 `log` 传输层上：

```
refusing to boot in production: MAIL_DRIVER is unset, which defaults to the `log`
transport. Password resets and email verifications would report success while
nothing is delivered. Set MAIL_DRIVER to a delivering driver (smtp | postmark |
ses | sendgrid | mailgun | resend), or set
MAIL_ALLOW_NON_DELIVERING_IN_PRODUCTION=true to acknowledge that outgoing mail is
intentionally discarded.
```

这防止的是一种静默的失败：在旧的默认值下，一次忘了设置 `MAIL_DRIVER` - 或者把大小写写错成 `MAIL_DRIVER=SMTP` - 的部署，会把每一次密码重置都报告成已发送，而实际上什么都没有离开这个进程，直到某个用户被锁在外面才会有人发现。

如果一次生产部署真的想要没有出站邮件（一个只读镜像、一次暗启动），就明确地承认它：

```env
MAIL_ALLOW_NON_DELIVERING_IN_PRODUCTION=true
```

只有 `1`、`true`、`yes` 或者 `on` 才算数为同意 - `=false` 或者一个笔误，都会让这道防护保持戒备。设置了这个覆盖项之后，每一次启动都会警告：出站邮件将不会被投递。

生产环境之外什么都不会变：`local`、`development`、`testing` 和 `staging` 会保留 `log` 这个默认值，并为未知的驱动程序保留那套警告并回退的行为。

### 生产环境对一条未加密的 SMTP 连接失败关闭

同一条规则，应用到连接是如何被保护的，而不是它是否投递。生产环境里的 `MAIL_DRIVER=smtp` 必须解析到一条加密的传输层，否则启动失败。

`MAIL_SMTP_ENCRYPTION` 接受 `starttls`、`tls` 或者 `none`（`ssl` 和 `null` 会被当作与 Laravel 兼容的别名接受）。未设置时，它会从凭据推导：

| `MAIL_SMTP_USER` / `MAIL_SMTP_PASS` | 解析为 | 因为 |
|---|---|---|
| 两者都设置了 | `starttls` | 凭据意味着提交端口上有一个真实的中继。 |
| 两者都没设置 | `none` | 本地捕获路径。Mailpit、MailHog 和 maildev 在 1025 上不认证地监听，也不讲 TLS。 |

所以一份全新的脚手架，零配置就能继续工作；而一次从未接上凭据的生产部署，会停下来，而不是悄悄地明文发送。如果一个中继期望在 465 上使用隐式 TLS，就设置 `MAIL_SMTP_ENCRYPTION=tls` - 这是这个传输层一直支持、但此前任何环境变量组合都触达不到的一种模式。

一个无法识别的值，会让启动在**每一个**环境里都失败，不只是生产环境。`MAIL_SMTP_ENCRYPTION=tsl` 是一个会加密的模式的字母换位，所以把它静默地当作“没有加密”处理，正是这个变量存在要防止的那个确切失败 - 在开发者的机器上失败，好过在部署里失败。

这个脱围机制，镜照的是上面那一个：

```env
MAIL_ALLOW_INSECURE_SMTP_IN_PRODUCTION=true
```

只有在这个中继只能通过一个私有网络到达时才是可以辩护的 - 一个 sidecar，或者 VPC 里的一个 Postfix。在其他任何情形下，明文 SMTP 都会把凭据和每一个密码重置链接暴露在网络传输过程里，并且对任何在这条路径上窃听的人保持暴露。

### `log` 驱动程序会记录整条消息

和 Laravel 的 `log` 邮件发送器一样：信封*以及*渲染后的正文。

```
mail (log driver): would send from=noreply@app.test to=["alice@example.org"]
  subject=Reset your password
  text=Reset your password: https://app.test/password/reset?token=9f3a…&signature=…
  html=<a href="https://app.test/password/reset?token=9f3a…&signature=…">Reset</a>
```

那个链接才是重点。在开发环境里，控制台正是您用来读取应用刚“发送”的验证或者密码重置链接的地方，而一个把它藏起来的驱动程序，是一个没人能用的驱动程序。

它在这里是安全的，因为这个驱动程序没法触达生产环境 - 在 `APP_ENV=production` 下，启动会在 `MAIL_DRIVER=log` 上拒绝启动（见上文）。这些正文只会存在于开发者自己的机器上。

如果您设置了 `MAIL_ALLOW_NON_DELIVERING_IN_PRODUCTION=true`，在一个已部署的环境里运行 `log` 驱动程序，您就是在选择把一次性的 bearer 链接放进您的日志里。任何能读到那些文件的人 - 运维人员、日志转发工具、留存存储桶、聚合器 - 都能使用它们，而链接过期帮不上忙，因为日志转运比一个人去读自己的收件箱要快。请按这个风险来设定您的留存和访问策略，或者用一个不打印的驱动程序：

```env
# 进程内捕获 - suprnova::mail::boot::captured_in_memory()，或者测试里的 Mail::fake()
MAIL_DRIVER=memory

# 或者一个本地捕获器（mailpit / maildev / mailhog），它会在一个 UI 里渲染出真实的邮件
MAIL_DRIVER=smtp
MAIL_SMTP_HOST=127.0.0.1
MAIL_SMTP_PORT=1025
```

### 逐驱动程序的环境变量

```env
# SMTP
MAIL_DRIVER=smtp
MAIL_SMTP_HOST=smtp.mailtrap.io
MAIL_SMTP_PORT=587
MAIL_SMTP_USER=...
MAIL_SMTP_PASS=...
MAIL_SMTP_ENCRYPTION=starttls   # 或者 `tls` 表示 465 上的隐式 TLS，或者 `none`

# Postmark
MAIL_DRIVER=postmark
MAIL_POSTMARK_TOKEN=...

# Amazon SES
MAIL_DRIVER=ses
MAIL_SES_ACCESS_KEY=...
MAIL_SES_SECRET_KEY=...
MAIL_SES_REGION=us-east-1

# SendGrid
MAIL_DRIVER=sendgrid
MAIL_SENDGRID_API_KEY=...

# Mailgun
MAIL_DRIVER=mailgun
MAIL_MAILGUN_API_KEY=...
MAIL_MAILGUN_DOMAIN=mg.example.com

# Resend
MAIL_DRIVER=resend
MAIL_RESEND_API_KEY=...
```

每一个 HTTP 提供商，也都遵循一个对应的 `MAIL_<PROVIDER>_ENDPOINT` 覆盖项，指向一个区域性 URL 或者一个模拟服务器（对针对 `wiremock` 的集成测试很有用）。

### 认证流程的发件人：`MAIL_FROM` 和 `MAIL_FROM_NAME`

内置的认证流程 mailable - 邮箱验证、密码重置，以及密码已修改通知 - 会从环境变量里解析它们信封上的 `From`，而不是一个硬编码的 `from()`：

```env
MAIL_FROM=no-reply@example.com        # 裸地址（认证流程要求它；未设置时失败关闭）
MAIL_FROM_NAME=Acme Support           # 可选的显示名字（自 0.5.9 起）
```

- `MAIL_FROM` **必须是一个裸地址。** 它会被原样提到这条消息的 `From` 里，所以一个 `"Name <addr>"` 形式的值，会被整体当作地址处理，并被传输层拒绝。
- `MAIL_FROM_NAME`（可选，在 **0.5.9** 中加入）会附上一个显示名字，这样这个请求头就会渲染成 `Acme Support <no-reply@example.com>`。未设置或者留空，会保留之前那种裸地址行为。它是在发送时读取的，所以也适用于已入队的认证流程邮件。

这两个变量只影响框架自己的认证流程 mailable。您自己的 `Mailable` 通过 `from()`（或者全局的 `always_from` 默认值）来设置它们的发件人 - 见下文。

## `Mailable` trait

Mailable 是知道如何渲染自己的、可序列化的结构体。这个 trait 的默认实现，针对这个 mailable 序列化后的字段，用 `tera::Tera::one_off` 来渲染：

```rust
use suprnova::async_trait;
use suprnova::mail::{Address, Attachment, Mailable};

#[async_trait]
impl Mailable for OrderShipped {
    fn mailable_name() -> &'static str { "OrderShipped" }
    fn subject(&self) -> String {
        format!("Order #{} shipped", self.order_id)
    }
    fn html_template_source(&self) -> Option<String> {
        Some("<p>Tracking: <code>{{ tracking }}</code></p>".into())
    }
    fn text_template_source(&self) -> Option<String> {
        Some("Tracking: {{ tracking }}".into())
    }
    fn from(&self) -> Option<Address> {
        Some(Address::new("orders@example.com").with_name("Acme Orders"))
    }
    fn attachments(&self) -> Vec<Attachment> {
        vec![Attachment::new("invoice.pdf", self.invoice_bytes.clone(), "application/pdf")]
    }
}
```

| 方法 | 是否必需？ | 用途 |
|--------|-----------|---------|
| `mailable_name()` | 是 | 持久化在队列信封里的稳定名字 - 改名会破坏正在途中的已入队邮件。 |
| `subject(&self)` | 是 | 计算出的主题。当 `subject_template_source` 返回 `None` 时会被原样使用。 |
| `subject_template_source(&self)` | 可选 | 主题用的 Tera 模板 - 为 `Some` 时，优先于 `subject()`，并以 `self` 为上下文渲染。语义和正文模板源相同。 |
| `html_template_source(&self)` | 可选 | HTML 正文的 Tera 模板。返回 `None` 以跳过 HTML。 |
| `text_template_source(&self)` | 可选 | 纯文本正文的 Tera 模板。返回 `None` 以跳过文本。 |
| `from(&self)` | 可选 | 覆盖全局默认值 `noreply@localhost`。 |
| `attachments(&self)` | 可选 | 要附加的文件。每一个都是 名字 + 字节 + mime。 |
| `render_subject(&self)` / `render_html(&self)` / `render_text(&self)` | 可选 | 如果您想绕开 Tera（Markdown → HTML、预渲染的内容、自定义的主题逻辑等等），就覆盖它们。 |

`html_template_source` 或者 `text_template_source` 里必须至少有一个返回 `Some`（或者 `render_html`/`render_text` 必须产出内容）。一个空正文的 mailable，在分发时（`Mail::send`）和入队时（`Mail::queue`）都会被拒绝。

### Tera 自动转义

自动转义是**关闭**的，因为邮件正文通常是手写的 HTML，而 Tera 的 `<>&` 转义会过度转义。如果您的字面正文里，出于非模板的原因包含了 `{{`（例如营销文案里引用了 Mustache 语法），就转义它：`{% raw %}{{ literal }}{% endraw %}`。

## 构建消息

`Mail::to(...)` 这个构建器，把收件人、抄送/密送、回复地址，以及一个逐消息的发件人覆盖，都编织进这次分发里：

```rust
Mail::to("alice@example.org")
    .cc("manager@example.com")
    .bcc("audit@example.com")
    .reply_to("support@example.com")
    .from(("Operations", "ops@example.com"))   // （显示名字，邮箱）
    .send(OrderShipped { order_id: 42, /* ... */ })
    .await?;
```

`Address` 接受 `&str`、`String`，以及 `(name, email)` 元组；`Mail::to(...)` 接受任何 `Into<Address>`。

## 附件

```rust
use suprnova::mail::Attachment;

let attachment = Attachment::new(
    "report.csv",
    csv_bytes,
    "text/csv",
);
```

附件通过 `Mailable::attachments` 方法搭车。全部五个 HTTP 提供商都能处理它们 - Postmark/SendGrid/Resend 走 JSON（base64 编码）、SES 走原始 MIME（因为 `Content.Simple` 不支持附件）、Mailgun 走 `multipart/form-data`（没有附件时则走表单编码路径）。

## 排队

`Mail::queue(...)` 会构建一个 `SendMailJob`，并把它推上框架队列。工作进程会从注册好的工厂重建这个 mailable，并通过绑定的传输层分发它：

```rust
// 一次性：注册工作进程会见到的每一个 Mailable 类型。
suprnova::mail::register_mailable_factory::<Welcome>()?;

// 在发送时：
Mail::to("alice@example.org").queue(Welcome { name: "Alice".into() }).await?;

// 延迟发送：
use std::time::Duration;
Mail::to("alice@example.org")
    .later(Duration::from_secs(60), Welcome { name: "Alice".into() })
    .await?;
```

同一道空正文防护，在队列路径上也会运行，所以一个配置错误的 Mailable，会在推送时就被拒绝，而不会等到任何信封被创建之后。

## 遥测

每一次发送，都会经过 `suprnova::mail::dispatch_with_telemetry`，它会打开一个带着以下字段的 `mail.send` `tracing::info_span!`：

- `transport` - 传输层名字（`"postmark"`、`"smtp"`、`"in-memory"`，……）
- `to_count`、`cc_count`、`bcc_count` - 收件人计数
- `has_html`、`has_text` - 正文形状
- `attachment_count` - 附件数量
- `tag_count`、`metadata_count` - 提供商提示的计数
- `priority` - `1..=5`，未设置时为 `0`

完成时，这个 span 会发出 `mail sent`（info）或者 `mail send failed`（warn），带着 `duration_ms`。同一个包装器覆盖了 `Mail::send`、`SendMailJob` 队列工作进程，以及通知 `MailChannel`，所以不管消息是怎么产生的，这个 span 的模式都是一样的。

## 用 `Mail::fake()` 做测试

`Mail::fake()` 会在返回的这个 RAII 守卫的生存期内，安装一个内存捕获传输层。镜照的是 `Bus::fake()` / `Queue::fake()` / `Cache::fake()`：

```rust
use suprnova::mail::Mail;

#[tokio::test]
async fn welcome_mail_is_sent_on_signup() {
    let fake = Mail::fake();

    sign_up("alice@example.org").await.unwrap();

    fake.assert_sent_count(1);
    fake.assert_sent(|m| m.to.iter().any(|a| a.email == "alice@example.org"));
    fake.assert_sent(|m| m.subject.starts_with("Welcome"));
    fake.assert_not_sent(|m| m.subject.contains("Password reset"));
}
```

当这个守卫被丢弃时，之前绑定的传输层（如果有的话）会被恢复。那些把 `Mail::fake()` 和显式的传输层绑定交错使用的测试，不会泄漏状态。

`Mail::fake()` 是 `Send + Sync` 的；按需要跨 await 或者跨线程共享它。

## 自定义传输层

`MailTransport` trait 是这个集成点：

```rust
use suprnova::async_trait;
use suprnova::mail::{MailTransport, OutgoingMessage};
use suprnova::FrameworkError;

pub struct StdoutTransport;

#[async_trait]
impl MailTransport for StdoutTransport {
    async fn send(&self, msg: &OutgoingMessage) -> Result<(), FrameworkError> {
        println!("--- mail ---\n{}\n--- end ---", msg.subject);
        Ok(())
    }
    fn name(&self) -> &'static str { "stdout" }
}

// At boot:
use std::sync::Arc;
suprnova::mail::Mail::set_transport(Arc::new(StdoutTransport))?;
```

传输层运行在 Tokio 的运行时上 - 异步 IO、连接池，以及并发发送都是一等公民。没有逐请求的 fork 代价。

### 为什么 Suprnova 有所不同

Laravel 的 Mailable 层构建在 Symfony Mailer 之上，后者在请求生命周期内部同步运行。Suprnova 的 `MailTransport` 从头到尾都是 `async fn send(&self, msg: &OutgoingMessage)`：HTTP 提供商用 `reqwest`，SMTP 路径用一个异步的 lettre 适配器，而 `dispatch_with_telemetry` 会把每一次发送都包进一个 Tokio `tracing` span。远程提供商不会阻塞处理程序线程，连接池能跨请求存活，而在一个处理程序里并发发送也轻而易举 - `tokio::try_join!(Mail::to(a).send(m), Mail::to(b).send(n))`的行为正是您所期望的那样。

另一处分歧是事件取消。Laravel 建模了一个能返回 `false` 并抑制发送的 `MessageSending` 监听器（`events->until()`）。Suprnova 的分发器不暴露一条短路返回通道 - `MessageSending` 只是可观测的。要给一次发送加门，请在 Mailable 层拒绝（覆写 `render_html` / `render_text` 让它返回一个错误），或者用您自己的防护包住 `MailBuilder::send` 调用。这个权衡是真实的：我们放弃了一个 Laravel 钩子，来保持这个分发器契约的简单。

另一个较小的分歧是刻意的加固。Laravel 满足于让 `MAIL_MAILER=log` 在生产环境里运行；Suprnova 没有一次明确的确认就拒绝在那里启动，因为一个报告成功、却什么都不投递的邮件子系统，正是那种好几周都没人注意到的中断。`log` 驱动程序本身的行为和 Laravel 的一模一样 - 包括完整的消息、正文和链接 - 这正是它在开发环境里有用的原因，而生产环境的拒绝，正是保持这一点安全的原因（见[`log` 驱动程序会记录整条消息](#log-驱动程序会记录整条消息)）。

## 最佳实践

### 在启动时注册工厂，而不是逐请求注册

`Mail::queue` 和 `Mail::later` 会推送一个携带着这个 mailable 名字和 JSON 载荷的 `SendMailJob` - 工作进程会通过 `mailable_registry` 重建出具体的类型。请在 `Server::serve` 时，把每一个可入队的 `Mailable` 都注册一次：

```rust
// bootstrap.rs
pub fn register() -> Result<(), suprnova::FrameworkError> {
    suprnova::mail::register_mailable_factory::<WelcomeEmail>()?;
    suprnova::mail::register_mailable_factory::<PasswordReset>()?;
    suprnova::mail::register_mailable_factory::<InvoiceShipped>()?;
    Ok(())
}
```

一个针对未注册 mailable 的 `Mail::queue`，会落到队列上，运行一次，命中“未知的 mailable”，按信封的退避策略重试，然后被死信处理 - 这会耗费掉一段可观测性排查时间，而如果这个工厂在启动时就绑定好了，您本来不需要花这个时间。

### 对任何缓慢或者不可靠的渲染都用队列

在一个请求处理程序里发送邮件，会把用户的响应延迟和您的 SMTP 服务器（或者不管哪个提供商的 HTTP API）耦合在一起。对超出一次同步的本地开发渲染的任何东西，都用 `Mail::queue`；当您想要延后这次分发时 - 引导后续跟进、提醒邮件、定时摘要 - 就用 `Mail::later`。

```rust
// 差：把响应时间和这个邮件提供商绑在了一起
Mail::to(&user.email).send(Welcome { ... }).await?;
return json_response!({ "ok": true });

// 好：200 OK 立即返回；由工作进程去投递这封邮件。
Mail::to(&user.email).queue(Welcome { ... }).await?;
return json_response!({ "ok": true });
```

### 总是在一个 Mailable 上设置 `from`

框架的默认发件人是 `noreply@localhost` - 在开发环境里，这对捕捉缺失的发件人很有用，但不是任何提供商会在生产环境里接受的发件人。覆写 `Mailable::from(&self)`（或者在一个 `NotificationMailable` 的 `#[mail(...)]` 属性里设置 `from = "..."`），这样每一条被分发的消息都有一个真实的发件人身份：

```rust
fn from(&self) -> Option<Address> {
    Some(Address::new("orders@example.com").with_name("Acme Orders"))
}
```

`MailBuilder` 上那个逐消息的覆盖（`.from(("Operations", "ops@example.com"))`），会优先于这个 mailable 的默认值 - 这对一次性的事务性发送很有用。

### 用队列来实现至少一次投递，而不是直接路径

`MailBuilder::send` 是至多一次的：如果传输层在分发给两个提供商的过程中失败了一半，您没法在不冒重复发送风险的情况下重试。`MailBuilder::queue` 搭乘的是那个持久的队列信封，它支持幂等键和工作进程级别的重试。对于任何您既不能丢失、又不能重复发送的邮件，请带着一个绑定到源头事件的稳定幂等键去排队。

## 一次性消息：`Mail::raw` 和 `Mail::html`

当这封邮件是一次单一的事务性提示、不值得用一个完整的 `Mailable` 结构体时，两个快捷方式能跳过这些样板代码：

```rust
use suprnova::mail::Mail;

// 纯文本
Mail::raw("Your code is 12345", |b| {
    b.to("alice@example.org")
        .subject("Verification code")
        .from("auth@example.com")
}).await?;

// HTML
Mail::html("<p>Hello, <b>world</b></p>", |b| {
    b.to("alice@example.org")
        .subject("Hi")
        .from("hello@example.com")
}).await?;
```

这个闭包收到一个预装了正文的 [`MailBuilder`]，让您可以在上面叠加收件人、主题、发件人、标签、metadata、优先级，以及任何其他 [`MailBuilder`] 的流式方法。这些路径完全绕开了 `Mailable` trait - 对一次性的测试提示和简短的事务性说明很有用。

## 全局默认值：`always_from`、`always_reply_to`、`always_to`、`always_return_path`

镜照 Laravel 的 `Mailer::alwaysFrom` / `alwaysReplyTo` / `alwaysTo` / `alwaysReturnPath`，这个 Mail 门面暴露了四个全局设置函数：

```rust
use suprnova::mail::{Address, Mail};

// 在启动时：
Mail::always_from(Address::new("noreply@example.com").with_name("Acme"))?;
Mail::always_reply_to(Address::new("support@example.com"))?;
Mail::always_return_path(Address::new("bounce@example.com"))?;

// 本地开发的“单一收件箱” - 把所有邮件都路由到一个地址，丢弃抄送/密送：
Mail::always_to(Address::new("dev-inbox@example.com"))?;

// 把一切都回滚（测试通常在收尾时调用这个）：
Mail::forget_always()?;
```

优先级是保守的 - 只有当分发的消息缺少一个显式值时，默认值才会生效：

| 字段 | 默认值何时生效 |
|-------|---------------------|
| `always_from` | 消息的 `from` 是框架默认值 `noreply@localhost` |
| `always_reply_to` | 消息没有显式的 `reply_to` |
| `always_to` | 总是 - 把每一条消息都路由到这个地址，并清空抄送/密送 |
| `always_return_path` | 消息没有显式的 `return_path` |

同样的优先级也适用于队列路径：已入队的 mailable，会在工作进程分发时经过 `apply_always_defaults`，所以直接发送和已入队发送，会收敛到相同的信封形状上。

## 标签、Metadata、优先级、请求头、退信地址

每一条被分发的消息，都可以携带 Laravel 风格的提供商提示 - 标签、metadata 键/值、RFC-2076 优先级、自定义 MIME 请求头，以及一个发件人 / 退信地址。它们会转发给 HTTP 提供商各自原生的字段（Postmark 的 `Tag` / `Metadata` / `Headers`，SES 的 `EmailTags`，SendGrid 的 `categories` / `custom_args` / `headers`，Mailgun 的 `o:tag` / `v:` / `h:`，Resend 的 `tags` / `headers`），并转发给 SMTP 作为 RFC 5322 请求头。

有两种方式可以附加它们 - 在 Mailable 层面设置逐类型的默认值，或者在构建器上逐消息设置：

```rust
use suprnova::async_trait;
use suprnova::mail::{Mailable, PRIORITY_HIGH};
use std::collections::BTreeMap;

#[async_trait]
impl Mailable for OrderShipped {
    fn mailable_name() -> &'static str { "OrderShipped" }
    fn subject(&self) -> String { format!("Order #{} shipped", self.order_id) }
    fn text_template_source(&self) -> Option<String> { Some("...".into()) }

    fn tags(&self) -> Vec<String> { vec!["transactional".into(), "order".into()] }
    fn metadata(&self) -> BTreeMap<String, String> {
        let mut m = BTreeMap::new();
        m.insert("order_id".into(), self.order_id.to_string());
        m
    }
    fn priority(&self) -> Option<u8> { Some(PRIORITY_HIGH) }
    fn headers(&self) -> Vec<(String, String)> {
        vec![("X-Origin".into(), "warehouse".into())]
    }
}
```

```rust
// 逐消息地设置在构建器上。metadata 键冲突时构建器胜出；tags 和 headers 取并集。
Mail::to(&user.email)
    .tag("campaign-spring")
    .metadata("ab_variant", "B")
    .priority(1)
    .header("X-Source", "promo-feed")
    .return_path("bounce@example.com")
    .send(WelcomeEmail { name: user.name.clone() })
    .await?;
```

五个优先级等级的常量，活在 `suprnova::mail::{PRIORITY_HIGHEST, PRIORITY_HIGH, PRIORITY_NORMAL, PRIORITY_LOW, PRIORITY_LOWEST}` - 和 Laravel 用的是同一套 `1..=5` 整数刻度。

## 检视已捕获的消息

`OutgoingMessage` 携带着 Laravel 风格的检视辅助函数 - 对测试断言和运行时审计日志都很有用：

```rust
fn audit_outgoing(m: &suprnova::mail::OutgoingMessage) {
    if m.has_tag("transactional") && m.has_to("alice@example.org") { /* ... */ }
    if m.has_metadata("order_id") { /* ... */ }
    if m.has_subject("Welcome") { /* ... */ }
    if m.has_attachment("invoice.pdf") { /* ... */ }
    if m.has_header("X-Source", "promo-feed") { /* ... */ }
}
```

收件人检查在邮箱上是大小写不敏感的；metadata、标签、主题和附件文件名的检查则是精确匹配的。

## 测试用的伪造实现：扩展表面

`Mail::fake()` 同时覆盖发送和入队这两条轨道。已发送的邮件（通过 `MailBuilder::send`）会落进这个内存传输层；已入队的邮件（通过 `.queue` / `.later`）会落进这个伪造实现的队列缓冲区。

```rust
use suprnova::mail::Mail;

#[tokio::test]
async fn boot_dispatches_welcome() {
    let fake = Mail::fake();

    onboard_user("alice@example.org").await.unwrap();

    // 已发送这一侧
    fake.assert_sent_count(1);
    fake.assert_sent(|m| m.has_to("alice@example.org") && m.subject.starts_with("Welcome"));
    fake.assert_sent_to("alice@example.org");
    fake.assert_not_sent(|m| m.subject.contains("Password reset"));

    // 已入队这一侧（用于延迟邮件）
    fake.assert_queued("WelcomeFollowup");
    fake.assert_queued_to("alice@example.org");
    fake.assert_queued_count(1);

    // 复合
    fake.assert_outgoing_count(2);   // 已发送 + 已入队
    fake.assert_not_outgoing("PasswordReset");
}
```

其他辅助函数：

| 辅助函数 | 用途 |
|--------|---------|
| `fake.captured()` | 所有已发送的消息 |
| `fake.count()` | 已发送计数 |
| `fake.queued()` | 所有已入队的 `QueuedSnapshot` |
| `fake.queued_count()` | 已入队计数 |
| `fake.outgoing_count()` | 已发送 + 已入队 |
| `fake.sent(predicate)` | 按谓词过滤已发送的 |
| `fake.sent_to(email)` | 按收件人过滤已发送的 |
| `fake.queued_named(name)` | 给定名字的已入队 mailable |
| `fake.queued_to(email)` | 发往某收件人的已入队 mailable |
| `fake.assert_sent_count(n)` | 精确的已发送计数 |
| `fake.assert_queued_count(n)` | 精确的已入队计数 |
| `fake.assert_outgoing_count(n)` | 精确的总数 |
| `fake.assert_nothing_sent()` | 已发送缓冲区为空 |
| `fake.assert_nothing_queued()` | 已入队缓冲区为空 |
| `fake.assert_nothing_outgoing()` | 两者都为空 |
| `fake.assert_sent_to(email)` | 至少有一条发给该收件人 |
| `fake.assert_not_sent_to(email)` | 没有发给该收件人的 |
| `fake.assert_queued(name)` | 至少有一条给定名字的已入队 |
| `fake.assert_queued_with(name, fn)` | 至少有一条给定名字、且匹配谓词的已入队 |
| `fake.assert_queued_to(email)` | 至少有一条发往该收件人的已入队 |
| `fake.assert_not_queued(name)` | 没有给定名字的已入队 |

`QueuedSnapshot::decode::<M>()` 会把这个载荷反序列化回具体的 `M`，所以类型检查过的谓词不需要定制的解码样板代码就能工作。

## 事件：`MessageSending` 和 `MessageSent`

每一次成功的分发，都会触发两个框架事件：

- `MessageSending` - 就在传输层调用**之前**。监听器能观察到这条消息的形状（收件人、主题、标签、正文形状标志）。
- `MessageSent` - 就在一次成功的传输层调用**之后**。监听器能观察到同样的形状；失败的发送不会触发这个事件。

```rust
use std::sync::Arc;
use suprnova::events::EventFacade;
use suprnova::mail::MessageSent;

EventFacade::listen::<MessageSent, _>(Arc::new(MyAuditListener)).await;
```

这两个事件都只是可观测的 - 这个分发器不建模一条 Laravel 风格的取消通道。给一次发送加门的变通做法，请参见上文的[为什么 Suprnova 有所不同](#为什么-suprnova-有所不同)。

## 多收件人便捷方法：`Mail::cc` 和 `Mail::bcc`

这个 Mail 门面暴露了三个入口点 - `to`、`cc`、`bcc` - 它们都会返回一个全新的 `MailBuilder`。挑一个匹配主要路由意图的：

```rust
// Start with a cc / bcc when the message is primarily an audit copy.
Mail::cc("manager@example.com")
    .to("alice@example.org")
    .send(OrderShipped { /* ... */ })
    .await?;
```

不管您从哪个入口点开始，同一套流式表面都适用。

### 针对 `Mail::fake()` 测试，而不是针对已绑定的传输层

`Mail::fake()` 会在这个 RAII 守卫的生存期内，安装一个进程本地的捕获传输层，并在结束时恢复之前绑定的东西。使用它的测试不需要在每次进入/退出时清理全局状态 - drop 语义会处理这一点。对那些会改动传输层全局状态的测试，请把 `#[serial_test::serial]` 和 `Mail::fake()` 结合起来使用；否则并发的测试会互相破坏。

## 下一步

- [通知](notifications.md) - `Notify::send` 会在邮件、数据库和 webpush 通道之间扇出；`#[derive(NotificationMailable)]` 是 `Mailable` trait 之上那个由宏驱动的捷径
- [队列](queues.md) - `Mail::queue` 和 `Mail::later` 所搭乘的那个持久信封
- [事件](events.md) - 监听 `MessageSending` / `MessageSent`，以及更广的分发器模型
- [测试](testing.md) - `Mail::fake()`，以及其他那些 `*::fake()` 守卫
- [配置](configuration.md) - 服务凭据的类型化配置注册

## 参考

- Trait：`suprnova::mail::Mailable`
- 门面：`suprnova::mail::Mail`
- 启动引导：`suprnova::mail::boot::bootstrap_from_env()`
- 传输层：`LogMailTransport`、`InMemoryMailTransport`、`SmtpMailTransport`、`PostmarkMailTransport`、`SesMailTransport`、`SendGridMailTransport`、`MailgunMailTransport`、`ResendMailTransport`
- 队列作业：`suprnova::mail::SendMailJob`
- 测试守卫：`suprnova::mail::MailFake`
- 遥测辅助函数：`suprnova::mail::dispatch_with_telemetry`
