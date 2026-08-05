# 支付 - Paddle 适配器

Paddle 适配器（`suprnova-payments-paddle`）把 Paddle 接进 Suprnova 那个通用的支付表面。当您想要一个能替您处理销售税、VAT、GST、催缴、开票和退款的支付提供商时，就用它 - Paddle 是一个记录商户（Merchant of Record，MoR），这意味着它是您客户眼中的法定卖方，并且吸收了那些像 Stripe 这样的直接扣款网关会留给您自己处理的合规表面。

这个选择改变了心智模型。您的领域代码并不*拥有*这个订阅 - Paddle 拥有。您打开一个结账，客户完成它，然后 `SubscriptionCreated` 这个 webhook 会告诉您这个订阅现在存在了。您无法通过 API 创建一个订阅，事后也不能替换它的价格集合。您可以取消，可以读取状态，可以更新计费元数据。剩下的都是 Paddle 的事。

本章假定您已经读过[支付](payments.md)里那个通用的五 trait 表面。这里我们涵盖的是*只*对 Paddle 才成立的东西。

## 什么时候选 Paddle

当以下任何一条成立时，选 Paddle：

- 您在全球销售数字产品，而税务合规（VAT、GST、美国销售税）是您路线图上一项真实的成本。
- 您不想自己去管理失败扣款的重试、催缴邮件，或者开具收据。
- 出于记账的原因，您想要来自单一法定卖方的单一账单。
- 您的商业模式以订阅为先，并且您接受由提供商来驱动订阅的生命周期。

如果您想要对扣款有直接的控制，自己处理税务，或者需要从您自己的代码路径里发出服务端的 `charge`/`capture`/`refund` 调用，那就改选[Stripe](payments.md#stripe)。

## 设置

添加这个 crate：

```bash
cargo add suprnova-payments-paddle
```

设置这四个环境变量：

```env
PADDLE_API_KEY=pdl_sdbx_apikey_...
PADDLE_WEBHOOK_KEY=pdl_ntfset_...
PADDLE_CLIENT_TOKEN=test_...
PADDLE_ENVIRONMENT=sandbox
```

| 变量 | 它是什么 | 它从哪里来 |
|---|---|---|
| `PADDLE_API_KEY` | 服务端 API 密钥（`pdl_live_apikey_…` / `pdl_sdbx_apikey_…`） | Paddle 仪表盘 → Developer Tools → Authentication |
| `PADDLE_WEBHOOK_KEY` | 通知目标密钥（`pdl_ntfset_…`） | Paddle 仪表盘 → Developer Tools → Notifications → 您的端点 |
| `PADDLE_CLIENT_TOKEN` | 浏览器安全的客户端令牌（`live_…` / `test_…`） | Paddle 仪表盘 → Developer Tools → Authentication → Client-side tokens |
| `PADDLE_ENVIRONMENT` | `sandbox`（默认）或者 `production` | 您自己决定 |

在 bootstrap 里注册这个提供商。两种写法都有效：

```rust
use std::sync::Arc;
use suprnova::payments::PaymentProviderRegistry;
use suprnova_payments_paddle::{PaddleEnvironment, PaddleProvider};

pub async fn bootstrap() {
    // 从环境变量（推荐）：
    let paddle = PaddleProvider::from_env()
        .expect("Paddle env vars not set");

    // 或者直接构造：
    let paddle = PaddleProvider::new(
        "pdl_sdbx_apikey_...",
        "pdl_ntfset_...",
        "test_...",
        PaddleEnvironment::Sandbox,
    ).expect("Paddle client init failed");

    PaymentProviderRegistry::bind("paddle", Arc::new(paddle));
}
```

这个 webhook 入口路由，由框架的 `webhook_routes(db.clone())` 助手函数注册 - 参见[支付](payments.md#webhook-handling)。`from_env()` 和 `new()` 都返回 `Result`，因为底层的 `paddle_rust_sdk::Paddle::new`，会在构造时校验 API 密钥的形态和端点 URL。

## MoR 的心智模型

这个形态，会让 Stripe 用户感到意外：

```
Stripe（网关）：
    您的应用  ─────────►  Stripe  ──►  银行卡网络
       │                    ▲
       └────── webhook ─────┘
    您在自己的数据库里拥有订阅状态；Stripe 是执行者

Paddle（记录商户）：
    您的应用  ─►  结账链接  ─►  客户  ──►  Paddle  ──►  银行卡网络
                                                  │
       ◄──────────────────  webhook  ──────────────────┘
    Paddle 拥有订阅状态；您的数据库是镜像
```

在代码里，这个区别体现在三个地方：

1. **您无法通过 API 创建一个订阅。** 用一个循环价格调用 `Checkout::start_session`；客户完成 Paddle 的小部件；`SubscriptionCreated` 这个 webhook 会对您的镜像做水合。
2. **您无法通过 API 替换一个订阅的价格集合。** Paddle 把套餐变更保留给它自己的仪表盘，或者它自己拥有的迁移流程。
3. **您无法删除一个客户。** 通过更新来归档，是受支持的变通做法。

Suprnova 把这些约束呈现成 `PaymentError::NotSupported`，而不是粉饰过去 - 参见下文的[能力矩阵](#能力矩阵)。

## 结账流程

`Checkout::start_session` 是用 Paddle 开始一次支付的唯一方式。前端会用您在 bootstrap 时设置的那个 `client_token`，通过 paddle.js 打开返回的那个 `transaction_id`：

```rust
use std::sync::Arc;
use suprnova::payments::*;

pub async fn start_checkout(
    user_id: String,
    email: String,
) -> PaymentResult<SessionPayload> {
    let provider = PaymentProviderRegistry::get("paddle")
        .expect("paddle provider not registered");

    // 1. 在 Paddle 里创建这个客户（或者重用一个已有的）。
    let cus = provider.create_customer(CreateCustomerRequest {
        user_id: user_id.clone(),
        email,
        name: None,
        metadata: None,
    }).await?;

    // 2. 打开一个结账会话。Paddle 是根据*价格类型*来分发
    //    一次性还是订阅的，不是根据下面的 SessionMode 字段。
    let session = provider.start_session(StartSessionRequest {
        mode: SessionMode::Subscription,           // 被 Paddle 忽略（见下面的说明）
        customer_ref: cus.provider_customer_id,
        price_refs: vec!["pri_pro_monthly".into()],
        success_return_url: "https://app.example/billing/success".into(),
        cancel_return_url: "https://app.example/billing/cancel".into(),
        amount_hint: None,
        idempotency_key: Some(format!("checkout_{user_id}")),
        metadata: None,
    }).await?;

    Ok(session)
}
```

返回的 `SessionPayload::PaddleInline` 携带着前端需要的一切：

```json
{
  "flow": "paddle_inline",
  "transaction_id": "txn_01h...",
  "customer_token": "ctm_01h...",
  "client_token": "test_..."
}
```

Svelte / React / Vue 里 paddle.js 的挂载代码，请参见[支付 - 前端集成](payments-frontend.md)。

### Paddle 是根据价格类型分发的，不是根据 `SessionMode`

一个真正的 Paddle 特有陷阱：`StartSessionRequest` 上的 `SessionMode::OneOff` / `SessionMode::Subscription` 字段，**被 Paddle 适配器忽略**。Paddle 的 API 只有一个 `transaction_create` 端点，这个提供商会检查提供的价格 ID，来推断这个流程 - 一个循环价格会启动一个订阅，一个一次性价格会启动一次单独的扣款。用 Stripe 时，是这个字段驱动流程；用 Paddle 时，是*价格*驱动流程。在把适配器指向它们之前，请用正确的价格类型来设置您的 Paddle 商品目录。

## 订阅是通过 webhook 到达的

因为 Paddle 拥有这个订阅的生命周期，您的领域代码只会在 Paddle 告诉您的时候，才*得知*一个订阅的存在。这个流程是：

```
您的应用                         Paddle                     客户
   │                              │                          │
   │  start_session(price=pri_…)  │                          │
   ├─────────────────────────────►│                          │
   │  PaddleInline { txn_id, … }  │                          │
   │◄─────────────────────────────┤                          │
   │                              │       paddle.js          │
   │                              │◄─────────────────────────┤
   │                              │        完成结账           │
   │                              ├─────────────────────────►│
   │                              │                          │
   │   subscription.created webhook                          │
   │◄─────────────────────────────┤                          │
   │                              │                          │
   ▼                              │                          │
 镜像表完成水合；                  │                          │
 payments_subscriptions 行        │                          │
 带有 provider_subscription_id    │                          │
```

框架的 `webhook_routes(db)` 处理程序，替您完成这次水合：它调用 `WebhookHandler::extract_payload_ids` 来找到这个 `subscription_id`，调用 `Subscription::get(id)` 来读取权威状态，然后在一个事务内对 `payments_subscriptions` + `payments_subscription_items` 做 upsert。等这个 webhook 返回 200 的时候，您的镜像已经和 Paddle 保持一致了。

在客户完成这个小部件和这个 webhook 到达之间，存在一个短暂的窗口，在这段时间里，`payments_subscriptions` 里还没有这个新订阅的行。两种模式能覆盖它：

- **用重定向 URL 来获得即时的用户体验。** 一旦 Paddle 确认了这次交易，`success_return_url` 就会在客户端触发，所以您可以展示“订阅已激活”，而不需要等待服务端的 webhook。
- **轮询后渲染。** 重定向之后，延迟一小段时间刷新页面，这样 Inertia 控制器就能读到这个此时已经水合完的镜像。

## 能力矩阵

不是每一个 trait 上的每一个方法，都会做它 Stripe 对应物所做的事。下面这张表就是事实。`subscribe()`，以及带着 `new_price_refs.is_some()` 的 `update()`，是唯二*总是*失败的方法；剩下的都能用，带着标注出来的那些注意事项。

| Trait 方法 | 行为 |
|---|---|
| `Checkout::start_session` | 能用。根据价格类型分发一次性还是订阅，不是根据 `SessionMode`。 |
| `Subscription::subscribe` | 总是 `NotSupported`。订阅是从结账完成 + webhook 里诞生的。 |
| `Subscription::update(cancel_at_period_end: Some(true), new_price_refs: None)` | 能用。接到带着默认 `EffectiveFrom::NextBillingPeriod` 的 `subscription_cancel` 上。 |
| `Subscription::update(new_price_refs: Some(...))` | 在 v1 里是 `NotSupported`。Paddle 把价格集合替换保留给它自己的迁移流程。 |
| `Subscription::update`（无操作） | 能用。通过 `subscription_get` 重新获取当前状态。 |
| `Subscription::cancel` | 能用，但 `at_period_end` 会**被忽略** - 总是被安排到下一个计费周期。参见[下文](#取消总是被安排生效)。 |
| `Subscription::get` | 能用。 |
| `CustomerStore::create_customer` | 能用。 |
| `CustomerStore::update_customer` | 能用。 |
| `CustomerStore::get_customer` | 能用。 |
| `CustomerStore::delete_customer` | `NotSupported`。如果需要，就用带着 `archived` 状态的 `update_customer`。 |
| `Payment::*` | 这个 trait 没有被实现。`provider.as_payment()` 返回 `None`。 |
| `WebhookHandler::*` | 能用。 |

`Payment` 没有被实现、`subscribe`/`delete_customer` 返回 `NotSupported`，以及 webhook 签名拒绝，这些不变量都被 `crates/suprnova-payments-paddle/tests/integration.rs` 里的常驻测试钉住了，所以上面这张表不会悄悄地漂移。

### 取消总是被安排生效

`Subscription::cancel(id, at_period_end)` 接受这个布尔值是为了 trait 兼容性，但**总是表现为已排期的取消** - Paddle 的 `EffectiveFrom` 枚举在 `paddle_rust_sdk` 0.18 里是私有的，所以立即取消在 v1 里行不通。用户会保留访问权限，直到当前的计费周期结束，到那时 Paddle 会触发 `subscription.canceled`，镜像会把 `status` 翻转成 `Canceled`。

如果您想要一个用户体验层面的“立即取消”，让应用访问权限立即被撤销，同时让 Paddle 在后台慢慢结束计费，就用您自己的 `subscription.status != Canceled && subscription.cancel_at_period_end == false` 标志来把关访问权限，并在 `cancel()` 返回之后立刻更新 UI - 下一个 webhook 会确认这一点。

### 客户删除是“通过更新来归档”

`delete_customer` 会返回 `PaymentError::NotSupported`，因为 Paddle 的公开 API 根本没有暴露一个删除端点。如果您需要在 Paddle 里隐去一条客户记录，就调用带着 `archived` 状态的 `update_customer`。框架的适配器没有直接封装这个 - metadata 字段就是那个脱围机制：

```rust
provider.update_customer(UpdateCustomerRequest {
    provider_customer_id: customer_id,
    email: None,
    name: None,
    metadata: Some(serde_json::json!({ "status": "archived" })),
}).await?;
```

在把这个发布出去之前，请对照您的 Paddle API 版本，确认这个确切的字段路径 - 这个 SDK 目前还没有直接建模这个 `status` 枚举。

## Webhook 签名验证

Paddle 用 HMAC 给每一个 webhook 签名。这个 `Paddle-Signature` 请求头看起来像 `ts=1716000000,h1=abcdef…`。这个适配器把验证工作委托给了 SDK 里的 `Paddle::unmarshal`，它会：

- 解析这个请求头
- 用您的 `PADDLE_WEBHOOK_KEY` 重新计算这个 HMAC
- 拒绝那些时间戳超出 `MaximumVariance::default()` 的签名（写这段话的时候是 5 秒 - 比这更旧的重放会被丢弃）

框架的 `webhook_routes` 处理程序，会在做任何其他事情之前先调用 `verify`；失败会返回 `401 invalid-signature`，不会泄漏请求体。您自己不需要写这些代码，但值得了解的是，这个验证是 HMAC + 时间戳容差，不是一次静态密钥比较。

## Webhook 载荷形态

这个适配器的 `extract_payload_ids`、`extract_payment_snapshot` 和 `extract_customer_snapshot` 方法，知道 Paddle 的载荷形态，这样框架就能对镜像表做水合。速查映射：

| Webhook 的 event_type | `NeutralEventKind` | 镜像效果 |
|---|---|---|
| `transaction.completed`, `transaction.paid` | `PaymentSucceeded` | 对 `payments_transactions` 做 upsert |
| `transaction.payment_failed` | `PaymentFailed` | 对 `payments_transactions` 做 upsert（失败） |
| `transaction.billed` | `InvoicePaid` | 对 `payments_transactions` 做 upsert，并关联上 `provider_subscription_id` |
| `adjustment.created`, `adjustment.updated` | `PaymentRefunded` | 对 `payments_transactions` 做 upsert（已退款） |
| `subscription.created` | `SubscriptionCreated` | `Subscription::get` → 对 `payments_subscriptions` + 明细项做 upsert |
| `subscription.updated`, `.activated`, `.paused`, `.resumed`, `.trialing` | `SubscriptionUpdated` | 和上面一样 |
| `subscription.canceled` | `SubscriptionCanceled` | 一样；设置 `canceled_at`，翻转状态 |
| `customer.created` | `CustomerCreated` | 只更新：如果镜像行存在，就刷新 `email`/`metadata` |
| `customer.updated` | `CustomerUpdated` | 一样 |
| 其他任何情况 | `None`（未映射） | 只有审计行 - 没有镜像变更 |

Paddle 把这个实体对象直接放在 `data` 下面（不像 Stripe 那样放在 `data.object` 下面）。金额是以**最小单位的字符串**形式到达的（`"1234"` = 主单位里的 12.34），不是小数 - 为了向前兼容，这个适配器会解析字符串和数字这两种形态。货币是以小写的 `currency_code` 形式到达的，而这份快照会把它转成大写。

### 含税金额

Paddle 报告的交易金额是**含税的**。框架的 `payments_transactions` 镜像会把它拆开：

- `amount_total_minor` - 客户支付的全额（含税）
- `amount_tax_minor` - 税额部分

扣除税后的净额是 `amount_total_minor - amount_tax_minor`。这和 Stripe 不一样（Stripe 报告的是不含税的金额，`amount_tax_minor = 0`）。跨两个提供商汇总收入的代码，需要意识到这一点：

```rust
let net_revenue_minor = txn.amount_total_minor - txn.amount_tax_minor;
```

## 创建客户

`CreateCustomerRequest` 直接映射到 Paddle 的 `customer_create`：

```rust
let cus = provider.create_customer(CreateCustomerRequest {
    user_id: "user_42".into(),       // 您应用的用户 id
    email: "alice@example.com".into(),
    name: Some("Alice".into()),
    metadata: None,                  // 在 v1 里不会转发给 Paddle
}).await?;
// cus.provider_customer_id == "ctm_01h..."
```

把 `cus.provider_customer_id` 和您的用户记录存在一起。每一次后续调用（开始一次结账、查找一个订阅等等）用的都是这个 Paddle 客户 ID，不是应用的用户 ID。镜像表 `payments_customers` 携带着这两个列，所以一次索引查找就能拿到任一个方向。

`update_customer` 和 `get_customer` 会直通到对应的 SDK 方法。`update_customer` 接受 `email`/`name` 更新，返回刷新后的 `CustomerRef`。`get_customer` 从 Paddle（不是从镜像）获取一份快照 - 当您在 Paddle 仪表盘里做了一次带外变更、需要一次全新读取时，就用它。

## 刻意设计成 `NotSupported` 的形态

一个不熟悉这份代码库的读者，可能会以为 `subscribe()` 和 `delete_customer()` 上的 `PaymentError::NotSupported`，是一个被拖延的 TODO。不是的。这些约束是 Paddle 产品表面的一部分，而 Suprnova 把它们编码了下来，而不是去模拟一些这个提供商永远不会兑现的本地变更。

每一条 `NotSupported` 错误消息，都指向那条受支持的工作流：

- `subscribe`："use `Checkout::start_session` with `SessionMode::Subscription` and await the `SubscriptionCreated` webhook"
- `update` 带 `new_price_refs`："Paddle price-set replacement on existing subscription not in v1"
- `delete_customer`："use `UpdateCustomer` with `archived` status"

当您在编写提供商无关的领域代码时，请显式地对这个错误分支处理：

```rust
match provider.delete_customer(&cus_id).await {
    Ok(()) => { /* Stripe 路径 */ }
    Err(PaymentError::NotSupported(_)) => {
        // Paddle 路径 - 改用更新来归档
        provider.update_customer(UpdateCustomerRequest {
            provider_customer_id: cus_id,
            email: None,
            name: None,
            metadata: Some(serde_json::json!({ "status": "archived" })),
        }).await?;
    }
    Err(e) => return Err(e),
}
```

### 为什么 Suprnova 有所不同

Laravel Cashier 只支持 Stripe，并且把订阅建模成应用拥有的：`$user->newSubscription('default', 'pri_pro')->create()` 的形态，就好像是应用在发起这个订阅。对一个直接扣款网关来说，这是准确的。对一个 MoR 来说，这是一句谎言 - 提供商才是那个行动者，不是您的应用。

Suprnova 的支付表面是提供商中立的，所以它不站队。这个 trait 表面（`subscribe`、`update`、`cancel`、`get`）是通用的形态；每一个适配器实现它的提供商暴露出来的东西，并在提供商的产品模型有差异的地方，返回 `NotSupported`。Stripe 适配器实现了 `subscribe`。Paddle 适配器没有，因为 Paddle 不允许它这么做。把这个差异藏在一个假的本地“create”背后，会让这个适配器对您说谎 - Suprnova 更喜欢那个带类型的 `NotSupported`，错误字符串里带着一条迁移提示。

同样的分歧也适用于 `Payment`（服务端扣款）。Stripe 实现了它；Paddle 没有，`provider.as_payment()` 会返回 `None`。需要 charge/capture/refund 的代码，必须检查 `as_payment().is_some()`，而不能盲目调用 - 参见[支付](payments.md#payment--optional-server-side-capture)。

## 测试您的集成

这个 crate 包含了常驻的不变量测试（不需要网络访问），外加一个用环境变量把关的、针对 Paddle 沙盒 API 的集成测试：

```bash
# 常驻不变量（签名拒绝、NotSupported 形态）：
cargo test -p suprnova-payments-paddle

# 外加沙盒集成（需要 PADDLE_API_KEY 等等）：
PADDLE_API_KEY=pdl_sdbx_apikey_... \
PADDLE_WEBHOOK_KEY=pdl_ntfset_... \
PADDLE_CLIENT_TOKEN=test_... \
PADDLE_ENVIRONMENT=sandbox \
  cargo test -p suprnova-payments-paddle
```

如果您要构建适配器特有的抽象，这些不变量测试就是您应该在自己代码里照着做的那些。三种值得抄的测试形态：

```rust
use suprnova::payments::*;
use suprnova_payments_paddle::{PaddleEnvironment, PaddleProvider};

#[test]
fn paddle_does_not_implement_payment_trait() {
    let p = PaddleProvider::new(
        "pdl_sdbx_apikey_test",
        "pdl_ntfset_test",
        "test_client",
        PaddleEnvironment::Sandbox,
    ).expect("provider construction");
    assert!(p.as_payment().is_none());
}

#[tokio::test]
async fn paddle_subscribe_returns_not_supported() {
    let p = /* ……和上面一样…… */;
    let err = p.subscribe(SubscribeRequest {
        customer_ref: "ctm_test".into(),
        price_refs: vec!["pri_test".into()],
        trial_days: None,
        idempotency_key: None,
        metadata: None,
    }).await.unwrap_err();
    assert!(matches!(err, PaymentError::NotSupported(_)));
}

#[test]
fn webhook_verify_rejects_bad_signature() {
    let p = /* ……和上面一样…… */;
    let mut headers = http::HeaderMap::new();
    headers.insert("paddle-signature", "ts=1234,h1=deadbeef".parse().unwrap());
    let ctx = WebhookContext {
        body: b"{}",
        headers: &headers,
        remote_addr: None,
    };
    assert!(matches!(p.verify(&ctx).unwrap_err(), PaymentError::WebhookSignature(_)));
}
```

对于完全不去碰 Paddle 的本地端到端测试，框架自带了 `MockPaymentProvider`。和 Paddle 一样，这个 mock 的 `as_payment()` 也返回 `None`（没有服务端扣款），所以根据 `as_payment().is_some()` 分支的代码，在 mock 下和在 Paddle 下走的是同一条路径。这个 mock 的 `subscribe()` 会返回 `Ok`（和 Paddle 不一样），所以需要断言 `NotSupported` 分支的测试，应该使用真正的 `PaddleProvider`。在测试里绑定这个 mock，而不是真正的提供商：

```rust
use std::sync::Arc;
use suprnova::payments::{MockPaymentProvider, PaymentProviderRegistry};

#[suprnova_test]
async fn checkout_flow() {
    PaymentProviderRegistry::bind("paddle", Arc::new(MockPaymentProvider::new()));
    // ……针对这个 mock 演练您的控制器……
}
```

## 生产环境检查清单

在把 `PADDLE_ENVIRONMENT` 翻转成 `production` 之前：

- [ ] 全部四个环境变量都设置在生产环境的密钥里，而不是提交进代码库
- [ ] 这个 webhook 端点 URL，已经在 Paddle 仪表盘的*Notifications*设置里注册好了，并且您在那里生成的目标密钥，和 `PADDLE_WEBHOOK_KEY` 是匹配的
- [ ] 这个商品目录里有正式的（不是沙盒的）价格 ID，并且您在 `price_refs` 里引用的这些 ID，在正式商品目录里存在
- [ ] 您的 `success_return_url` 和 `cancel_return_url` 指向的是 HTTPS 端点（Paddle 在生产环境会拒绝 HTTP）
- [ ] 您已经决定好，当 `subscribe()`、`delete_customer()` 或者 `update(price_refs)` 返回 `NotSupported` 时，您的应用要怎么响应 - 要么在代码里分支处理，要么记录下这些流程只对 MoR 有效
- [ ] 您已经对取消的用户体验做过压力测试：取消总是被排期的，所以“您已取消，但在 DATE 之前仍保留访问权限”，才是您的 UI 应该展示的消息
- [ ] 您已经对订阅到达的这个 webhook 做过压力测试：存在一个窗口，客户已经付款了，但镜像里还没有这一行
- [ ] 您在正确地汇总收入：Paddle 的金额是含税的，Stripe 的金额是不含税的

## 下一步

- [支付](payments.md) - 那个通用的五 trait 表面，以及 webhook 处理程序的镜像水合契约
- [支付 - 前端集成](payments-frontend.md) - Svelte / React / Vue 里的 paddle.js 内嵌结账
- [支付 - 提供商指南](payments-provider-guide.md) - 从头到尾编写您自己的适配器 crate
- [配置](configuration.md) - Paddle 环境变量所接入的那套类型化配置注册
- [应用启动](bootstrap.md) - `PaymentProviderRegistry::bind` 在您应用里实际所处的位置
