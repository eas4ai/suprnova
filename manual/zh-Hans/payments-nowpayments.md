# 支付 - NOWPayments

`suprnova-payments-nowpayments` 适配器创建托管发票、验证支付通知，并使用商户 API 密钥读取支付状态。它在常规的支付提供商注册表里注册为 `nowpayments`。

## 安装与配置

两个依赖都要使用包含该适配器的 Suprnova 版本。如果应用位于 Suprnova 源码检出目录旁边：

```toml
[dependencies]
suprnova = { path = "../suprnova/framework" }
suprnova-payments-nowpayments = { path = "../suprnova/crates/suprnova-payments-nowpayments" }
serde_json = "1"
```

在应用受保护的环境中配置这些值：

```dotenv
NOWPAYMENTS_ENVIRONMENT=sandbox
NOWPAYMENTS_API_KEY=your-sandbox-api-key
NOWPAYMENTS_IPN_SECRET=your-sandbox-ipn-secret
NOWPAYMENTS_IPN_CALLBACK_URL=https://app.example/webhooks/payments/nowpayments
```

环境接受 `sandbox` 或 `production`，默认是 `sandbox`。未知或空白的环境会让配置失败。空白的凭据会在发出 HTTP 请求之前就失败。请为每个环境使用各自独立的凭据。

在启动时注册提供商，并把配置错误向上传播：

```rust,no_run
use std::sync::Arc;
use suprnova::payments::{PaymentProviderRegistry, PaymentResult};
use suprnova_payments_nowpayments::NowPaymentsProvider;

fn register_nowpayments() -> PaymentResult<Arc<NowPaymentsProvider>> {
    let provider = Arc::new(NowPaymentsProvider::from_env()?);
    PaymentProviderRegistry::bind("nowpayments", provider.clone());
    Ok(provider)
}
```

按照[支付概览](payments.md)所示，添加支付相关的迁移，并把 `webhook_routes(db)` 组合进应用的路由器。端点是 `POST /webhooks/payments/nowpayments`。请把这条由提供商认证的路由放在浏览器会话的 CSRF 中间件之外。用确切的公开 HTTPS URL 配置回调；请求体必须原封不动地到达适配器。

## 发起一张托管发票

在发出请求之前，先持久化一次结账尝试，并带上唯一的商户订单引用。金额来自应用自己可信的订单，绝不来自浏览器提供的总额。

```rust,no_run
use suprnova::payments::{Money, SessionMode, StartSessionRequest};
use suprnova_payments_nowpayments::{InvoiceCreationError, NowPaymentsInvoice, NowPaymentsProvider};
use serde_json::json;

async fn create_order_invoice(
    provider: &NowPaymentsProvider,
    order_id: String,
    amount: Money,
) -> Result<NowPaymentsInvoice, InvoiceCreationError> {
    provider.create_invoice(StartSessionRequest {
        mode: SessionMode::OneOff,
        customer_ref: String::new(),
        price_refs: Vec::new(),
        amount_hint: Some(amount),
        success_return_url: "https://app.example/billing/return".into(),
        cancel_return_url: "https://app.example/billing/cancel".into(),
        idempotency_key: None,
        metadata: Some(json!({"order_id": order_id})),
    }).await
}
```

`Checkout::start_session` 接受同样的请求，并返回通用的 `SessionPayload::Redirect`。把它的 `provider_session_id` 当作**发票 ID** 存起来，然后重定向到它那个已校验的 URL。具体的 `create_invoice` 方法还会返回商户订单引用，并区分那些结果不确定的创建错误。

发票适配器要求一次性模式、空的客户引用和价格引用，以及一个法币货币指数已定义的正 `Money` 金额。它发送主单位的精确十进制金额和小写的货币代码。除非提供了 `pay_currency`，否则客户会在托管页面上自行选择加密货币。

元数据是一个严格的对象，包含这些字段：

| 字段 | 含义 |
| --- | --- |
| `order_id` | 必填且非空的商户引用，最多 128 字节 |
| `order_description` | 可选的描述，最多 500 字节 |
| `pay_currency` | 可选的小写提供商代号，例如 `btc` |
| `is_fixed_rate` | 可选的提供商汇率设置 |
| `is_fee_paid_by_user` | 可选的提供商手续费设置 |

未知的元数据键会被拒绝。不要把任意的应用数据或客户数据塞进这个对象。返回 URL 必须使用与回调相同的 HTTPS 源，并且不能带凭据或片段。发票重定向必须指向所选的 NOWPayments 环境，并包含返回的发票 ID。

## 创建失败与重试

NOWPayments 的发票端点没有文档化任何幂等键保证。适配器会拒绝不为 `None` 的 `idempotency_key`；`order_id` 是关联数据，不是提供商侧的去重。HTTP 重定向和自动重试都没有启用。请求有 30 秒的期限和 64 KiB 的响应上限。

`InvoiceCreationError::Rejected` 表示输入校验失败，或者 API 明确拒绝。`Unknown` 表示发票可能已经存在：例如请求超时、提供商返回了服务端错误，或者它的成功响应无效。把那次尝试标记为不确定，并在创建另一张发票之前，先到提供商的控制台里对账该订单。不要把发票创建放进通用的重试循环里。通过 `Checkout::start_session` 调用时，未知结果会是一个带有该恢复指引的 `PaymentError::Provider`。

## 验证并对账支付

一张发票可能产生一个独立的支付 ID。`payment_status(payment_id)` 会调用经过认证的支付端点，并拒绝 ID 不一致的响应。`Checkout::session_status(invoice_id)` 返回 `NotSupported`；发票 ID 不能拿去代入支付查询端点。这个适配器不会用控制台的邮箱和密码凭据去按发票列出支付。

```rust,no_run
use suprnova::payments::{Money, PaymentResult};
use suprnova_payments_nowpayments::{NowPaymentsProvider, NowPaymentsStatus};

async fn payment_matches_order(
    provider: &NowPaymentsProvider,
    verified_payment_id: &str,
    expected_invoice_id: &str,
    expected_order_id: &str,
    expected_price: Money,
) -> PaymentResult<bool> {
    let payment = provider.payment_status(verified_payment_id).await?;
    Ok(payment.status == NowPaymentsStatus::Finished
        && payment.invoice_id.as_deref() == Some(expected_invoice_id)
        && payment.order_id.as_deref() == Some(expected_order_id)
        && payment.price == expected_price)
}
```

支付 ID 必须来自一个已验证的 IPN，或者其他可信的服务端记录。浏览器返回并不是支付的证据。查询成功同样不是一次授权检查：在改变访问权限之前，请把它与已存储的订单、发票、金额和货币对上。`price` 是请求的法币价格，不是收到的加密货币数量。请检查商户关于接受部分支付的设置；这个适配器绝不会把 `partially_paid` 转成成功。

| 提供商状态 | 中立事件 |
| --- | --- |
| `finished` | `PaymentSucceeded` |
| `failed`, `expired`, `cancelled`, `canceled` | `PaymentFailed` |
| `refunded` | `PaymentRefunded` |
| `waiting`、`confirming`、`confirmed`、`sending`、`partially_paid` 以及未知值 | 没有中立分类 |

IPN 签名使用递归排序的 JSON 和 HMAC SHA-512。验证使用常数时间的 MAC 比较。缺失、重复或无效的签名，重复的 JSON 键，超大的载荷，以及格式错误的必填字段，都会被拒绝。提供商不会对任何独立的投递时间戳签名，所以这里没有一个凭空发明的时间戳重放窗口。重放保护依靠持久化的回执。

NOWPayments 的 IPN 没有独立的事件 ID。适配器按支付 ID 和状态来标识一份回执，所以带着不同 `updated_at` 的重复投递，不会把同一个终态事件再走一遍。框架把已验证的事件存放在 `payments_webhook_events` 里，并对失败的处理进行重试。迟到的待处理事件不会修改一条已经结清的交易镜像。

### 没有客户的发票与应用状态

发票 API 并不提供 Suprnova 的交易镜像所需的客户身份。这个适配器从 `WebhookHandler::mirrors_payment_transactions` 返回 `false`：所有已验证的事件，包括退款，都会留在审计日志里，但它不会创建任何客户镜像或交易镜像。Stripe 和 Paddle 保持默认的镜像行为。

通用的 webhook 路由会记录并处理提供商事件；它并不履行应用的订单。应用的对账作业应当消费已验证的回执，读取当前的支付状态，并在一个幂等事务中更新自己的订单。请存一个按提供商与支付、或者按订单唯一的履约键，这样重试和乱序投递就无法两次授予访问权限。请把作业自身持久的完成状态，与框架的 `processed_at` 分开跟踪。对迟到的事件使用当前已认证的状态，并按应用的政策处理退款。

## 能力边界

`CustomerStore` 和 `Subscription` 的操作返回 `NotSupported`。`as_payment()` 和 `as_promotions()` 返回 `None`。这个适配器没有实现托管余额、周期性计费、直接用充值地址结账、出款、发起退款，或者提供商侧的客户管理。NOWPayments 为其中一些操作提供了单独的产品；这个发票适配器并不假装实现了它们。

### 为什么 Suprnova 有所不同

通用的提供商 trait 依然可用，但不受支持的操作会明确地失败。没有提供商客户的发票，依靠的是经过审计的通知和归应用所有的订单，而不是伪造出来的客户记录或订阅。

## 验证

从源码检出目录运行适配器测试和共享的支付回归测试：

```sh
CARGO_INCREMENTAL=0 cargo nextest run -p suprnova-payments-nowpayments
CARGO_INCREMENTAL=0 cargo nextest run -p suprnova --test payments
```

本地这套测试使用伪造的 HTTP 响应、一份独立生成的 JavaScript 签名固件，以及配合 SQLite 的真实框架入口。它覆盖了成功的结账、认证失败、格式错误或过大的响应、超时、不可信的重定向、无效的签名、状态映射、重复投递，以及数据库故障之后的恢复。本地固件并不能确立与提供商之间的互操作性。

在启用生产环境之前，请使用一个 NOWPayments 沙盒账户和一个可达的 HTTPS 回调。创建一张发票，把提供商提供的成功、部分和失败情形都走一遍，确认它的签名会被接受，并把支付 ID 与已存储的发票和订单对上。重放一次投递，确认应用的履约键能阻止第二次授予。本地测试并不意味着做过任何真实账户的沙盒或生产运行。

提供商参考资料：[API 端点](https://nowpayments.zendesk.com/hc/en-us/articles/21345824322717-API-and-endpoint-description)、[IPN 认证](https://nowpayments.zendesk.com/hc/en-us/articles/21395546303389-IPN-and-how-to-setup)，以及[官方 SDK](https://github.com/NowPaymentsIO/nowpayments-sdk-nodejs)。

## 下一步

关于渲染重定向载荷，请看[前端集成](payments-frontend.md)；关于共享的约定，请看[提供商指南](payments-provider-guide.md)。
