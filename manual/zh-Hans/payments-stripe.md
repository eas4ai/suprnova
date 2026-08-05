# 支付 - Stripe 适配器

`suprnova-payments-stripe` 是 Suprnova 那个提供商中立的支付表面的参考适配器。它通过 `async-stripe` 1.0.0-rc.5，针对 Stripe 的 API，实现了全部五个支付 trait（`Checkout`、`Payment`、`Subscription`、`CustomerStore`、`WebhookHandler`）。当您需要确切知道一个方法调用的是哪个 Stripe 端点、webhook 签名格式是怎么被验证的、PaymentIntent 是怎么流经 `ChargeResult` 的，或者哪些事件类型映射到了这个中立的事件枚举上时，就来查这一章。

trait 本身的形态、环境变量设置，以及 bootstrap 模式，请先读[支付](payments.md)。本章是 Stripe 特有内容的深入探讨。

## 网关，而不是记录商户

Stripe 默认是一个**支付网关**：您直接把资金收进自己的银行账户，税款的征收和缴纳、开票、催缴，以及拒付处理，都由您自己负责。这和 Paddle（[支付 - Paddle](payments-paddle.md)）形成对比：在那里，Paddle 才是记录商户 - 他们收款，申报税务，再把扣除费用之后的净额付给您。

这对本章来说的实际后果是：`StripeProvider` 实现了 `Payment`（您可以在服务端对一张卡进行授权、扣款、退款和撤销）。`PaddleProvider` 没有。这个 trait 上的拆分是存在的，因为这两种流程真的不一样 - 不是因为我们时间不够。

### Stripe Managed Payments（可选启用的记录商户模式）

Stripe 的**Managed Payments** 项目，会针对符合条件的交易，把 Stripe 挪进记录商户的位置 - Stripe 成为法定卖方，计算、征收、申报并缴纳销售税/VAT/GST，并且承担争议处理。这个项目有一些硬性的集成约束：

- **只支持托管结账。** 会话必须运行在 Stripe 的托管页面上。Elements/自定义流程都被排除在外 - 这就是为什么这个适配器的托管一次性路径（见下文）是唯一能和它组合的 `OneOff` 形态。
- **预定义的、带有合规税码的 Price。** 明细项必须引用 `price_…` 对象，而这些对象的产品，在 Stripe 仪表盘里必须带有一个标记为 Managed-Payments-eligible 的税码。临时金额会被拒绝。
- **账户开通。** Stripe 账户必须先接入这个项目；在一个还没开通的账户上带着这个标志的会话，会失败。

用 `.with_managed_payments(true)` 或者 `STRIPE_MANAGED_PAYMENTS=true`，逐提供商启用它 - 这个适配器随后会在创建托管一次性会话时，发送 `managed_payments[enabled]=true`。关闭时（默认情况），这个字段会被完全省略。

### 为什么 Suprnova 有所不同

Laravel 在核心文档里把 Cashier 作为一个第一方的 Stripe 集成来发布。它很方便，但只支持 Stripe - 添加第二个提供商，意味着要么派生 Cashier，要么另建一套并行的表面。

Suprnova 没有特别优待 Stripe。这个 Stripe 适配器只是一个 crate，用任何其他提供商都要实现的那同样五个 trait 来注册自己。您的领域代码从不指名 `StripeProvider`；它调用的是针对一个从注册表里解析出来的 `Arc<dyn PaymentProvider>` 的 `provider.charge(...)`，而 Stripe 的行为，只需要换一下，就能变成 Paddle 的行为。等您以后添加 Mollie，或者接上一个还不存在的区域性网关时，您实现同样的五个 trait，而您应用剩下的部分都不需要动。

## 构造

```rust
use suprnova_payments_stripe::StripeProvider;
use std::sync::Arc;
use suprnova::payments::PaymentProviderRegistry;

// 生产环境：从环境变量读取。
let stripe = StripeProvider::from_env()
    .expect("STRIPE_SECRET_KEY / PUBLISHABLE_KEY / WEBHOOK_SIGNING_SECRET");

// 测试/显式配置：
let stripe = StripeProvider::new(
    "sk_test_...",
    "pk_test_...",
    "whsec_...",
);

PaymentProviderRegistry::bind("stripe", Arc::new(stripe));
```

`StripeProvider` 是 `Clone` 的（代价很低 - 底层的 `stripe::Client` 是由 `Arc` 支撑的），它持有这些值：

| 字段 | 来源 | 用途 |
|---|---|---|
| `secret_key` | `sk_live_…` / `sk_test_…` | 每一次 API 调用上的 HTTP `Authorization: Bearer …` |
| `publishable_key` | `pk_live_…` / `pk_test_…` | 暴露在 `SessionPayload::StripeElements` 里面，这样前端就能挂载 Stripe.js，而不需要单独查一次配置 |
| `webhook_signing_secret` | `whsec_…` | 对 `Stripe-Signature` 请求头做 HMAC-SHA256 验证 |
| `managed_payments` | `STRIPE_MANAGED_PAYMENTS`（`true`/`1`）或者 `.with_managed_payments(bool)` | 在创建托管一次性会话时，发送 `managed_payments[enabled]=true`（参见[Managed Payments](#stripe-managed-payments-可选启用的记录商户模式)） |

`from_env()` 返回 `Result<Self, String>` - 错误消息会指出缺失的那个必需变量（`STRIPE_MANAGED_PAYMENTS` 是可选的；缺失就意味着关闭）。启动时没有 panic 路径。

## 结账会话

`Checkout::start_session` 会根据请求，选择它要用的那个 Stripe 表面：

| 请求形态 | Stripe 对象 | `SessionPayload` 变体 |
|---|---|---|
| `OneOff` + 非空的 `price_refs` | 托管 Checkout Session，`mode=payment` | `StripeCheckoutRedirect { url, provider_session_id: "cs_…" }` |
| `OneOff` + 空的 `price_refs` + `amount_hint` | PaymentIntent | `StripeElements { client_secret, publishable_key, provider_session_id: "pi_…" }` |
| `Subscription` + `price_refs` | 托管 Checkout Session，`mode=subscription` | `StripeCheckoutRedirect` |

这个托管的一次性路径会发送 `allow_promotion_codes=true`（客户可以在 Stripe 的页面上输入促销代码 - 和下面的 `Promotions` trait 搭配使用），并且，当这个提供商配置了它时，还会发送 Managed Payments 标志。把 Stripe 的 `{CHECKOUT_SESSION_ID}` 模板字面量放进您的 `success_return_url` 里 - Stripe 会在重定向时把真实的 `cs_…` id 替换进去，然后您的返回页面会把它喂给 `session_status`。

`Checkout::session_status` 把 `GET /v1/checkout/sessions/{id}` 映射到中立的 `CheckoutSessionState` 上：

| Stripe 的 `status` / `payment_status` | `CheckoutSessionState` |
|---|---|
| `open` | `Open` |
| `expired` | `Expired` |
| `complete` + `paid` 或者 `no_payment_required` | `Complete { paid: true, payment_ref, amount_total }` |
| `complete` + `unpaid`（延迟结算） | `Complete { paid: false, … }` |

`payment_ref` 带着这个会话的 PaymentIntent id（`pi_…`），这样返回页面和扫描任务，就能把这个会话和 `Payment` 操作以及 `payments_transactions` 镜像关联起来。`amount_total` 是已经把提供商侧折扣和 Managed Payments 税费都折算进去的结算总额。

## 促销代码

`StripeProvider` 实现了可选的 `Promotions` trait（`provider.as_promotions()` 返回 `Some`）。`create_promotion_code` 映射到 `POST /v1/promotion_codes`：它基于一张预先创建好的优惠券（`coupon_ref`）生成一个代码，限定给一个客户（`customer_ref`），并带有一个可选的过期时间和兑换上限。限制是由 Stripe 在兑换时强制执行的 - 一个为客户 A 生成的代码，客户 B 输入时会被拒绝，过期的代码会被拒绝，而 `max_redemptions: Some(1)` 会让这个代码变成单次使用。活动模式请参见[支付](payments.md)的 `Promotions` 一节。

## PaymentIntent 的生命周期

Stripe 把一次扣款尝试表示成一个**PaymentIntent**。这个 intent 会经历几种状态；Suprnova 的 `Payment` trait 驱动着这些转换。每一个 `StripeProvider` 的 `Payment` 方法，都映射到一个 `/v1/payment_intents/...` 端点：

| `Payment` 方法 | Stripe 端点 | 它做什么 |
|---|---|---|
| `charge` | `POST /v1/payment_intents` | 针对一个已保存的支付方式，一次调用里同时创建并确认。`capture_method: "manual"`，所以这个 intent 会转移到 `requires_capture`，**不是** `succeeded`。 |
| `capture` | `POST /v1/payment_intents/{id}/capture` | 结算一个之前已经被授权的 intent。状态从 `requires_capture` → `succeeded`。 |
| `refund` | `POST /v1/refunds` | 完全或部分地撤销一个已扣款的 intent。 |
| `void` | `POST /v1/payment_intents/{id}/cancel` | 在扣款之前释放一次授权。状态从 `requires_capture` → `canceled`。 |
| `status` | `GET /v1/payment_intents/{id}` | 获取当前状态（返回 `PaymentStatus`）。 |

### 先授权，再扣款

`StripeProvider::charge` **不会**立即结算资金。它发送 `capture_method=manual` + `confirm=true`，这会对这张卡进行授权并预留资金，然后等待一次显式的 `capture` 调用。这是那个典型的两步流程：

```rust
use suprnova::payments::{
    PaymentProviderRegistry, ChargeRequest, ChargeResult,
    Money, Currency, PaymentStatus,
};

let provider = PaymentProviderRegistry::get("stripe").unwrap();
let payment = provider.as_payment()
    .expect("Stripe implements Payment");

let result = payment.charge(ChargeRequest {
    customer_ref: "cus_NffrFeUfNV2Hib".into(),
    payment_method_ref: "pm_card_visa".into(),
    amount: Money::from_minor_units(2999, Currency::USD),
    description: Some("Pro plan, manual capture".into()),
    idempotency_key: Some("order-12345".into()),  // 见下文的“幂等性”一节
    metadata: None,
}).await?;

match result {
    ChargeResult::Completed { provider_transaction_id, status, .. }
        if status == PaymentStatus::Pending => {
        // 已授权 - 订单发货时再结算。
        let settled = payment.capture(&provider_transaction_id).await?;
        assert!(matches!(
            settled,
            ChargeResult::Completed { status: PaymentStatus::Succeeded, .. }
        ));
    }
    ChargeResult::RequiresClientAction { client_secret, .. } => {
        // 需要 3DS 强化验证 - 见下文的“3DS 与 SCA”一节。
    }
    other => panic!("unexpected charge result: {other:?}"),
}
```

如果您想要**立即**扣款 - 常见的电商一次性场景 - 请改用带 `SessionMode::OneOff` 的 `Checkout::start_session`。那条路径会创建一个启用了 `automatic_payment_methods` 的 PaymentIntent，并把客户端密钥交给前端，让客户的浏览器就地确认这个 intent。`Payment::charge` 是给服务端驱动的流程用的，那种流程下您已经持有客户已保存的支付方式，并且想要显式的先授权再扣款控制（典型场景是市场平台、延迟履约的 SaaS，或者分批发货的电商）。

### 状态映射

Stripe 的状态会折叠进 Suprnova 的 `PaymentStatus` 枚举：

| `PaymentIntentStatus` | `PaymentStatus` |
|---|---|
| `Succeeded` | `Succeeded` |
| `Processing` | `Pending` |
| `RequiresCapture` | `Pending`（已授权，等待扣款） |
| `RequiresAction` | `Pending`（从 `charge` 返回为 `RequiresClientAction`） |
| `RequiresConfirmation` | `Pending` |
| `RequiresPaymentMethod` | `Pending` |
| `Canceled` | `Canceled` |
| _新的 Stripe 状态（这个枚举是 `#[non_exhaustive]` 的）_ | `Failed` |

这个 `non_exhaustive` 回退是刻意的。Stripe 偶尔会新增状态（例如在引入新的支付方式类型时）。把它们呈现成 `Failed` 是一个保守的默认值 - 在您升级这个适配器之前，您的应用会把这次扣款当作尚未确认来处理。

### 3DS 与 SCA

欧洲的强客户认证（Strong Customer Authentication）、印度 RBI 的规定，以及其他几个监管机构，都要求持卡人在一个独立的浏览器上下文里，对这次扣款做认证。Stripe 把这呈现为带有一个 `next_action` 块的 `requires_action`。

`StripeProvider::charge` 会把这个转换成两个 `ChargeResult` 变体之一：

```rust
ChargeResult::RequiresClientAction {
    provider_transaction_id,   // pi_xxx - 把它留存起来
    action_kind: "stripe_3ds", // Stripe 特有的标签
    client_secret,             // 交给 Stripe.js
    publishable_key,           // 交给 Stripe.js
}
```

当这个 intent 的 `next_action` 包含一个重定向 URL 时（有些认证流程是 URL 重定向而不是就地弹窗），这个结果会被重写为：

```rust
ChargeResult::RedirectRequired {
    provider_transaction_id,
    url,                       // 把浏览器重定向到这里
    return_to: None,
}
```

您的控制器把这个 `RequiresClientAction` 载荷交给 Inertia 页面；前端调用 `stripe.confirmCardPayment(client_secret, ...)`，客户完成 3DS。确认成功之后，Stripe 会触发 `payment_intent.succeeded`，webhook 路由会写入这个镜像行。Svelte / React / Vue 的代码片段，请参见[支付 - 前端集成](payments-frontend.md)。

### 撤销与退款

`void` 会在扣款**之前**释放一次授权；`refund` 撤销的是一次已经扣款的支付。对一个已经扣款的 intent 调用 `void` 会失败 - Stripe 会用一条包含 `"already succeeded"` 或者 `"You cannot cancel"` 的消息来拒绝，而这个适配器会把它呈现成 `PaymentError::Validation`，这样您的处理程序就能把一个可恢复的用户错误（改用 `refund`）和一次真正的提供商故障区分开来。任何其他失败都是 `PaymentError::Provider`。

```rust
let voided = payment.void("pi_3PNzj...").await;
match voided {
    Ok(()) => { /* 授权已释放 */ }
    Err(suprnova::payments::PaymentError::Validation(msg)) => {
        // 已经扣过款了 - 改调用 refund。
        let refund = payment.refund(RefundRequest {
            provider_transaction_id: "pi_3PNzj...".into(),
            amount: None,           // 全额退款
            reason: Some("requested_by_customer".into()),
            idempotency_key: None,  // refund() 不会转发这个字段 - 见下文的“幂等性”一节
        }).await?;
    }
    Err(e) => return Err(e.into()),
}
```

## 客户

`StripeProvider` 针对 `/v1/customers` 实现了 `CustomerStore`。这个适配器把返回的 `Customer` 映射成中立的 `CustomerRef`，保留邮箱和您应用的 `user_id`：

```rust
use suprnova::payments::CreateCustomerRequest;

let customer = provider.create_customer(CreateCustomerRequest {
    user_id: "user-42".into(),       // 您应用的用户 id
    email: "alice@example.com".into(),
    name: Some("Alice Example".into()),
    metadata: None,
}).await?;

// customer.provider_customer_id == "cus_NffrFeUfNV2Hib"
// 把它和您的 User 行存在一起，这样后续的
// 扣款、订阅和 webhook 就能解析回来。
```

`update_customer`、`get_customer` 和 `delete_customer`，分别打向 `POST /v1/customers/{id}`、`GET /v1/customers/{id}` 和 `DELETE /v1/customers/{id}`。Stripe 的删除操作会返回一个 `DeletedCustomer` 信封，这个适配器会把它丢弃 - 只有这次调用的成功/失败会被传播出去。

## 订阅

`StripeProvider::subscribe` 会带着客户 ref、一个 `items[]` 数组，以及一个可选的 `trial_period_days`，POST 到 `/v1/subscriptions`：

```rust
use suprnova::payments::{SubscribeRequest, SubscriptionStatus};

let sub = provider.subscribe(SubscribeRequest {
    customer_ref: "cus_NffrFeUfNV2Hib".into(),
    price_refs: vec!["price_pro_monthly".into()],
    trial_days: Some(14),
    idempotency_key: None,
    metadata: None,
}).await?;

assert!(matches!(
    sub.status,
    SubscriptionStatus::Trialing | SubscriptionStatus::Active
));

println!("Period ends at {}", sub.current_period_end);
for item in &sub.items {
    println!(
        "  {} × {} @ {:?}",
        item.quantity, item.provider_price_id, item.unit_amount,
    );
}
```

### 周期边界

Stripe 在 API 版本 `2023-08-16` 里，把 `current_period_start` / `current_period_end` 这两个时间戳，从父级 Subscription 移到了每一个 `SubscriptionItem` 上。多明细项的订阅理论上可以有不一致的明细项周期，但实践中，同一个订阅上的每一个明细项，都共享父级的计费周期。这个适配器在返回的 `SubscriptionResult` 里，把**第一个明细项**的周期当作父级周期。如果您确实需要逐明细项的周期，就从 `sub.items[n]` 里读取它们 - 它们被保留在这份快照上。

### 在周期结束时取消，还是立即取消

```rust
// 软取消 - 保留访问权限，直到 current_period_end：
let sub = provider.cancel("sub_1234", /* at_period_end */ true).await?;
// sub.cancel_at_period_end == true
// sub.status == Active

// 立即取消 - Stripe 的 DELETE /v1/subscriptions/{id}：
let sub = provider.cancel("sub_1234", /* at_period_end */ false).await?;
// sub.status == Canceled
```

这两条路径打向不同的 Stripe 端点。软取消是带着 `cancel_at_period_end=true` 的 `POST /v1/subscriptions/{id}` - 这个订阅会保持活跃，直到计费周期结束，然后 Stripe 会把它敲定下来。立即取消是带着 `prorate=false` 和 `invoice_now=false` 的 `DELETE /v1/subscriptions/{id}`。

### `update()` 是刻意受限的

`UpdateSubscriptionRequest` 有两个这个适配器会处理的字段：`cancel_at_period_end` 和 `new_price_refs`。第一个是支持的；第二个会返回 `PaymentError::NotSupported`：

```rust
provider.update(UpdateSubscriptionRequest {
    provider_subscription_id: "sub_1234".into(),
    new_price_refs: Some(vec!["price_team_yearly".into()]),
    cancel_at_period_end: None,
    idempotency_key: None,
}).await
// → Err(PaymentError::NotSupported(
//      "Stripe price-set replacement on existing subscription not in v1. \
//       Cancel the subscription and create a new one with the new price set."
//   ))
```

这是少数几个 `NotSupported` 是诚实的答案、而不是一种拖延的地方。Stripe 的 price-set 替换需要删除并重新创建订阅明细项 - 这个形态因提供商而异（按比例分摊、计费周期锚定、保留试用行为），把它硬塞进一个单一的中立 API 里，隐藏掉的东西会比它帮上的忙更多。推荐的路径是取消现有的订阅，然后用新的 price 集合再 `subscribe` 一次，如果您需要按比例分摊策略，就自己应用一个。

## Webhook

Stripe 发送的 webhook，是用 HMAC-SHA256 按下面这种格式签名的：

```
Stripe-Signature: t=1717000000,v1=5257a869e7ecebeda32affa62cdca3fa51cad7e77a0e56ff536d0ce8e108d8bd
```

`StripeProvider::verify` 会解析这个请求头，用这个 webhook 签名密钥，对 `"{timestamp}.{raw_body}"` 重新计算 HMAC-SHA256，然后对这个请求头里的每一个 `v1=` 值做**常量时间**比较。在签名密钥轮换期间，会存在多个 `v1=` 值 - Stripe 会在一个窗口期内让新旧密钥重叠，这样您就能重新签名并部署，而不需要一次一刀切的切换。

```
Stripe-Signature: t=1717000000,v1=<old_sig>,v1=<new_sig>
```

只要**任意一个** `v1=` 值匹配，这个适配器就会接受这个请求。缺失 `t=`，或者没有任何 `v1=` 值的请求头，会被当作 `PaymentError::WebhookSignature` 拒绝。请求头里任何地方出现非 ASCII 字节，同样会被拒绝 - Stripe 从不会发送它们，把它们当作无效处理，比用一个替换字符去顶替它们更安全。

您从不会直接调用 `verify`。框架的 `webhook_routes(db.clone())` 会注册 `POST /webhooks/payments/{provider}`，并且对每一个落到那里的请求，调用这个适配器的 `verify` + `parse_event` + 载荷提取器。带重试感知的审计行为，请参见[幂等性](idempotency.md) - 包括那条规则：之前失败过的事件，会在提供商重试时重新尝试水合。

### 事件 → 中立映射

Stripe 的事件类型，通过 `stripe_event_to_neutral` 函数，映射到 Suprnova 的 `NeutralEventKind` 上。映射表如下：

| Stripe 事件类型 | `NeutralEventKind` |
|---|---|
| `payment_intent.succeeded` | `PaymentSucceeded` |
| `payment_intent.payment_failed` | `PaymentFailed` |
| `charge.refunded` | `PaymentRefunded` |
| `charge.dispute.created` | `PaymentDisputed` |
| `customer.subscription.created` | `SubscriptionCreated` |
| `customer.subscription.updated` | `SubscriptionUpdated` |
| `customer.subscription.deleted` | `SubscriptionCanceled` |
| `customer.subscription.paused` | `SubscriptionUpdated` |
| `customer.subscription.resumed` | `SubscriptionUpdated` |
| `customer.subscription.trial_will_end` | `SubscriptionUpdated` |
| `invoice.payment_succeeded` / `invoice.paid` | `InvoicePaid` |
| `invoice.payment_failed` | `InvoiceFailed` |
| `customer.created` | `CustomerCreated` |
| `customer.updated` | `CustomerUpdated` |
| _其他任何情况_ | `None` |

映射到 `None` 的事件（Radar 的欺诈信号、打款、余额转账、`created` 之后的争议生命周期事件），仍然会被持久化到 `payments_webhook_events` 这张审计表里 - 它们只是不会驱动镜像表。如果您需要它们，就在一个自定义处理程序里，直接读取 `event.raw_payload`。

这份映射也在 crate 的根模块被重新导出，这样您就能在 webhook 路由之外使用它：

```rust
use suprnova_payments_stripe::stripe_event_to_neutral;
use suprnova::payments::NeutralEventKind;

assert_eq!(
    stripe_event_to_neutral("payment_intent.succeeded"),
    Some(NeutralEventKind::PaymentSucceeded),
);
assert_eq!(
    stripe_event_to_neutral("radar.early_fraud_warning.created"),
    None,
);
```

### 载荷提取

在 `verify` 和 `parse_event` 成功之后，框架会调用 `extract_payload_ids`、`extract_payment_snapshot` 和 `extract_customer_snapshot`，取出驱动镜像表的那些字段（底层的“从您自己的数据库读取”模式，请参见[Eloquent](eloquent.md)）。Stripe 在结构上是一致的：每一个 webhook 都把相关的实体放在 `data.object` 里，用 `id` 作为它的主键。

这些提取器处理四类事件家族：

- **订阅事件** - 取出 `data.object.id`（这个订阅的 id）和 `data.object.customer`。
- **客户事件** - 取出 `data.object.id`（这个客户的 id）。
- **PaymentIntent / Charge 事件** - 取出 `data.object.id`、`data.object.amount`、`data.object.currency`、`data.object.customer`，以及（只对 `payment_intent.succeeded`）把 `data.object.created` 当作 `paid_at`。
- **账单事件** - 取出 `data.object.id`、客户指针、`data.object.subscription`（只对循环扣款）、`amount_paid`（回退到 `amount_due`）、`tax`、`currency`，以及 `data.object.status_transitions.paid_at`。

其他任何情况，这些快照提取器都会返回 `None`；审计行仍然会落地。

## 镜像表

六张表在您应用的数据库里，支撑着这个支付表面。把框架的迁移和您自己的迁移一起应用：

```rust
use sea_orm_migration::{MigrationTrait, MigratorTrait};
use suprnova::payments::migrations::CreatePaymentsTables;

pub struct Migrator;

impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            // ……您的迁移……
            Box::new(CreatePaymentsTables),
        ]
    }
}
```

被创建出来的这些表是 `payments_customers`、`payments_payment_methods`、`payments_subscriptions`、`payments_subscription_items`、`payments_transactions` 和 `payments_webhook_events`。webhook 路由会在每个事件一个数据库事务内，对它们做水合 - 局部状态永远不会被观察到，而这条审计行会在多次重试之间携带着 `process_error`，这样失败对运维人员来说始终是可见的。

## 幂等性

Stripe API 调用上的出站幂等性，和 webhook 投递上的入站幂等性，是两个不同的故事。请把它们当作两件事来读。

### 出站：逐方法覆盖情况

Stripe 通过 `Idempotency-Key` 这个 HTTP 请求头来支持请求幂等性 - 同一个密钥配上同一个请求体，在一个 24 小时的重放窗口内，会返回同一个响应对象；请求体不匹配则会返回一个错误。Suprnova 的 Stripe 适配器目前**并没有**统一地把这个 DTO 的 `idempotency_key` 字段，穿到那个请求头上。写这段话的时候，实际的行为是这样的：

| 方法 | DTO 字段 | 适配器做了什么 |
|---|---|---|
| `Payment::charge` | `ChargeRequest::idempotency_key` | 被转发进 POST 请求体，作为 `idempotency_key=...`（不是 HTTP 请求头）。Stripe 的 API **不会**读取表单形式的幂等键，所以最好把它当作无效的，直到这个适配器迁移到请求头路径为止。 |
| `Payment::refund` | `RefundRequest::idempotency_key` | 默默地被丢弃 - 这个字段没有被转发。 |
| `Checkout::start_session` | `StartSessionRequest::idempotency_key` | 默默地被丢弃。 |
| `Subscription::subscribe` / `update` | `*Request::idempotency_key` | 默默地被丢弃。 |

如果您现在依赖针对 Stripe 的扣款/退款重试的至多一次语义，就在您自己的调用点上把关这次重试（一个持久化在您数据库里的确定性领域键，配上一个防止第二次插入的唯一索引），直到这个适配器把这个请求头接通为止。这些 DTO 字段在 API 上是被接受的，但目前并没有被一路兑现到网络上 - 在测试和生产代码里把它们设成 `None`，让这个缺口保持显式，不要假设 Stripe 会替您给这些重试去重。

这是 v1 适配器里一个已知的缺口，也是下一个版本的候选修复项；一旦这条线路接通，这个表面的形态不会变。

### 入站：webhook 去重

webhook 的幂等性由框架在入口这一侧处理，而且已经完全接通了。每一个事件都会落进 `payments_webhook_events`，上面有一个 `(provider, provider_event_id)` 的唯一索引。一个已经被处理过的事件，其重复投递会立即向 Stripe 返回 200，不会重新跑一次水合；而一个之前**失败**过的事件，其重复投递会重新尝试水合，所以提供商的重试就是您的恢复机制。完整的审计 + 重试契约，请参见[幂等性](idempotency.md)。

## 测试

这个适配器是由 hyper 支撑、rustls 打头的。构造一个 `StripeProvider` 的测试，需要一个已注册的加密提供者；我们在 `#[cfg(test)]` 里精确安装一次 `ring`：

```rust
#[cfg(test)]
mod tests {
    use suprnova_payments_stripe::StripeProvider;
    use std::sync::OnceLock;

    fn install_crypto_provider() {
        static ONCE: OnceLock<()> = OnceLock::new();
        ONCE.get_or_init(|| {
            let _ = rustls::crypto::ring::default_provider().install_default();
        });
    }

    fn provider() -> StripeProvider {
        install_crypto_provider();
        StripeProvider::new("sk_test_dummy", "pk_test_dummy", "whsec_dummy")
    }

    #[test]
    fn parses_subscription_webhook_ids() {
        let p = provider();
        let event = /* 构造一个带着 raw_payload 的 WebhookEvent */;
        let ids = p.extract_payload_ids(&event);
        assert_eq!(ids.subscription_id.as_deref(), Some("sub_abc"));
    }
}
```

对于会打到真实 Stripe 沙盒的集成测试，请在您的测试环境里设置 `STRIPE_SECRET_KEY` 以及相关的那几个变量。对于您自己控制器的单元测试，优先使用框架提供的 `MockPaymentProvider` - 它实现了全部五个 trait，返回值可预测，而且零网络访问。

## 下一步

- [支付](payments.md) - trait 表面、注册表、bootstrap 模式，以及带 flow 标记的 `SessionPayload`。
- [支付 - Paddle](payments-paddle.md) - 记录商户这一侧的对应篇；同样的五个 trait，不同的职责拆分。
- [支付 - 提供商指南](payments-provider-guide.md) - 如何为一个 Suprnova 没有自带的网关，编写一个适配器。
- [支付 - 前端集成](payments-frontend.md) - Svelte / React / Vue 根据 `SessionPayload.flow` 做的分发，包括 Stripe.js 的 confirm-card-payment 循环。
- [幂等性](idempotency.md) - 让 webhook 处理在至少一次投递下保持安全的那份审计 + 重试契约。
- [Eloquent](eloquent.md) - 把镜像表和您自己的模型放在一起查询；一切都只是一个 SeaORM 实体。
