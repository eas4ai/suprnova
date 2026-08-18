# 支付

Suprnova 的支付表面是提供商中立的。您选择一个适配器 crate - Stripe、Paddle，或者您自己编写的 - 在启动时注册它，然后您的领域代码调用同样的四个核心 trait（外加一个用于服务端扣款的可选第五个），不管背后是哪个提供商。您数据库里的镜像表由 webhook 保持同步，所以您的领域代码是从自己的数据库读取的，而不需要为每一次查询都去打提供商的 API。

没有任何功能被锁定在单一提供商上。Stripe 的直接扣款模式和 Paddle 的记录商户（Merchant-of-Record）模式，都能装进同一个 trait 约定里。唯一有区别的表面是 `Payment`（服务端扣款），它是可选的 - Paddle 不需要它，所以 Paddle 没有实现它。提供商通过重写 `PaymentProvider::as_payment()` 来宣告自己的能力，让它返回 `Some(&dyn Payment)`；调用方在运行时查询。

## 为什么 Suprnova 有所不同

Laravel 在核心文档里把 Cashier 作为一个第一方的 Stripe 集成来发布。它很方便，但只支持 Stripe - 添加第二个提供商，意味着要么派生 Cashier，要么另建一套并行的表面。Suprnova 对待支付提供商的方式，和它对待缓存、存储驱动程序的方式一样：一套通用的 trait 集合，可替换的适配器。您的领域代码从不指名 `StripeProvider` 或者 `PaddleProvider`；它调用的是针对一个从注册表里解析出来的 `Arc<dyn PaymentProvider>` 的 `provider.subscribe(...)`，而背后的提供商，只需要改一行 bootstrap 代码就能变成别的东西。

## 快速上手

加上适配器 crate。在 Suprnova 发布它的 v0.1 之前，框架及其适配器 crate 都是通过 git 而不是 crates.io 引入的：

```toml
# Cargo.toml
[dependencies]
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.2.4" }
suprnova-payments-stripe = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.2.4" }
```

在启动时注册这个提供商和 webhook 路由器。这个 webhook 路由器就是一个普通的 `Router`，您把它组合进自己的 `routes::register()` 里：

```rust,ignore
// src/bootstrap.rs
use std::sync::Arc;
use suprnova::payments::PaymentProviderRegistry;
use suprnova_payments_stripe::StripeProvider;

pub async fn register() {
    let stripe = StripeProvider::from_env().expect("Stripe env vars not set");
    PaymentProviderRegistry::bind("stripe", Arc::new(stripe));
}
```

```rust,ignore
// src/routes.rs
use std::sync::Arc;
use suprnova::payments::webhook_routes;
use suprnova::container::App;
use suprnova::Router;
use sea_orm::DatabaseConnection;

/// `Application::routes(routes::register)` 在启动时调用它一次。
/// 我们从这个支付 webhook 路由器出发，再用普通的 `.get(...)` /
/// `.post(...)` 调用，把应用其余的路由叠加上去。
pub fn register() -> Router {
    let db: Arc<DatabaseConnection> = App::get().expect("db not bound");

    webhook_routes(db)
        .get("/", crate::controllers::home::index)
        .post("/login", crate::controllers::auth::login)
        // ……您其余的那些路由……
        .into()
}
```

`webhook_routes(db)` 返回一个只包含 `POST /webhooks/payments/{provider}` 的 `Router`。因为 `Router::get` 和 `Router::post` 各自返回一个 `RouteBuilder`，而它能通过 `.into()` 转回 `Router`，所以在这个支付路由器之上继续链式调用，是最直接的组合方式。如果您本来就用 `routes!{}` 宏来写普通路由，把这个 webhook 的 POST 丢进同一个块里就行 - `webhook_routes` 只是围着一次 `Router::new().post(...)` 调用的便捷包装。

在您的控制器里，查出这个提供商，创建一位客户，然后打开一个结账会话：

```rust,ignore
// src/controllers/billing.rs
use std::sync::Arc;
use suprnova::payments::*;

pub async fn start_checkout(
    user_id: String,
    email: String,
) -> PaymentResult<SessionPayload> {
    let provider = PaymentProviderRegistry::get("stripe")
        .ok_or_else(|| PaymentError::Internal("stripe not registered".into()))?;

    let customer = provider.create_customer(CreateCustomerRequest {
        user_id,
        email,
        name: None,
        metadata: None,
    }).await?;

    provider.start_session(StartSessionRequest {
        mode: SessionMode::Subscription,
        customer_ref: customer.provider_customer_id,
        price_refs: vec!["price_pro_monthly".into()],
        success_return_url: "https://app.example/billing/success".into(),
        cancel_return_url: "https://app.example/billing/cancel".into(),
        amount_hint: None,
        idempotency_key: None,
        metadata: None,
    }).await
}
```

这个 `SessionPayload` 会进到您的 Inertia 页面 props 里。前端根据 `payload.flow` 来分发，渲染出正确的小部件 - 参见[支付 - 前端集成](payments-frontend.md)。

## 挑一个适配器

### Stripe

```toml
# Cargo.toml
suprnova-payments-stripe = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.2.4" }
```

必需的环境变量：

| 变量 | 描述 |
|---|---|
| `STRIPE_SECRET_KEY` | 私密密钥（`sk_live_…` / `sk_test_…`） |
| `STRIPE_PUBLISHABLE_KEY` | 可公开密钥（`pk_live_…` / `pk_test_…`） |
| `STRIPE_WEBHOOK_SIGNING_SECRET` | Webhook 端点的签名密钥（`whsec_…`） |

```rust,ignore
use suprnova_payments_stripe::StripeProvider;
use std::sync::Arc;
use suprnova::payments::PaymentProviderRegistry;

// 从环境变量读取（生产环境推荐）：
let stripe = StripeProvider::from_env().expect("Stripe env vars not set");

// 或者直接构造：
let stripe = StripeProvider::new("sk_test_...", "pk_test_...", "whsec_...");

PaymentProviderRegistry::bind("stripe", Arc::new(stripe));
```

Stripe 实现了每一个 trait，包括可选的 `Payment`（通过 PaymentIntents 做服务端扣款）和 `Promotions`（通过 `/v1/promotion_codes` 铸造促销码）。`provider.as_payment()` 和 `provider.as_promotions()` 两者都返回 `Some`。

### Paddle

```toml
# Cargo.toml
suprnova-payments-paddle = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.2.4" }
```

必需的环境变量：

| 变量 | 描述 |
|---|---|
| `PADDLE_API_KEY` | API 密钥（`pdl_live_apikey_…` / `pdl_sdbx_apikey_…`） |
| `PADDLE_WEBHOOK_KEY` | 通知目的地的密钥（`pdl_ntfset_…`） |
| `PADDLE_CLIENT_TOKEN` | 客户端令牌（`live_…` / `test_…`） |
| `PADDLE_ENVIRONMENT` | 可选，默认为 `"sandbox"` |

```rust,ignore
use suprnova_payments_paddle::{PaddleProvider, PaddleEnvironment};
use std::sync::Arc;
use suprnova::payments::PaymentProviderRegistry;

// 从环境变量读取：
let paddle = PaddleProvider::from_env().expect("Paddle env vars not set");

// 或者直接构造：
let paddle = PaddleProvider::new(
    "pdl_sdbx_apikey_...",
    "pdl_ntfset_...",
    "test_...",
    PaddleEnvironment::Sandbox,
).expect("Paddle client init failed");

PaymentProviderRegistry::bind("paddle", Arc::new(paddle));
```

Paddle 是一个记录商户（Merchant of Record） - 它负责税务、催缴，以及完整的订阅生命周期。它不暴露服务端扣款，所以没有实现 `Payment`。调用 `provider.as_payment()` 会返回 `None`。订阅是间接创建的：调用 `Checkout::start_session`，走完 Paddle 的小部件，然后 `SubscriptionCreated` webhook 会到达，确认这个订阅 ID。

## trait 的拆分

`PaymentProvider` 是一个总括性的 trait，捆绑了四个每个适配器都会实现的通用 trait - `Checkout`、`Subscription`、`CustomerStore`、`WebhookHandler`。还有两个进一步的可选 trait：`Payment`（服务端扣款只对像 Stripe 这样的网关才有意义）和 `Promotions`（生成促销代码）。适配器通过重写 `PaymentProvider::as_payment()` / `PaymentProvider::as_promotions()` 来选择加入。

```rust,ignore
pub trait PaymentProvider: Checkout + Subscription + CustomerStore + WebhookHandler {
    fn name(&self) -> &'static str;

    /// 如果这个提供商也实现了 `Payment`（服务端扣款），就返回 `Some`。
    /// 默认返回 `None`。
    fn as_payment(&self) -> Option<&dyn Payment> {
        None
    }

    /// 如果这个提供商也实现了 `Promotions`
    /// （生成促销代码），就返回 `Some`。默认返回 `None`。
    fn as_promotions(&self) -> Option<&dyn Promotions> {
        None
    }
}
```

### `Checkout` - 通用，打开客户端小部件

每一个提供商都实现了 `Checkout`。调用 `start_session` 来获得一个带 flow 标记的 `SessionPayload`，供您的前端渲染。`session_status`（默认：`NotSupported`；由那些会话可以被查询的提供商重写，例如 Stripe）报告的是您之前发起的一个会话，在提供商那一侧的权威状态。

```rust,ignore
#[async_trait]
pub trait Checkout: Send + Sync {
    async fn start_session(&self, req: StartSessionRequest) -> PaymentResult<SessionPayload>;

    async fn session_status(&self, provider_session_id: &str)
        -> PaymentResult<CheckoutSessionState>;
}
```

`StartSessionRequest` 的字段：

| 字段 | 类型 | 说明 |
|---|---|---|
| `mode` | `SessionMode` | `OneOff` 或者 `Subscription` |
| `customer_ref` | `String` | 来自 `CustomerStore::create_customer` 的提供商客户 ID |
| `price_refs` | `Vec<String>` | 提供商的价格/产品 ID |
| `success_return_url` | `String` | 支付完成后把用户带去哪里 |
| `cancel_return_url` | `String` | 用户放弃时把他们带去哪里 |
| `amount_hint` | `Option<Money>` | 为一次性金额提供的覆盖值或提示 |
| `idempotency_key` | `Option<String>` | 用于安全重试 |

`session_status` 是重定向流程的服务端验证原语。当客户回到您的返回页面时，不要相信他们浏览器带回来的查询参数 - 传入您在 `start_session` 时记录下的 `provider_session_id`，然后基于结果分支处理：

```rust,ignore
match provider.session_status(&order.provider_session_id).await? {
    CheckoutSessionState::Complete { paid: true, payment_ref, amount_total } => {
        // 履行这个订单。`payment_ref`（例如 Stripe 的 `pi_…`）
        // 和 `Payment` 操作以及 payments_transactions 镜像相关联。
    }
    CheckoutSessionState::Complete { paid: false, .. } => { /* 结算待定 */ }
    CheckoutSessionState::Open => { /* 客户还没有完成支付 */ }
    CheckoutSessionState::Expired => { /* 会话已过期 - 关闭这个订单 */ }
}
```

同一个调用也支撑着对账扫描：重新轮询您数据库里仍然是打开状态的订单，并履行那些在客户关掉标签页之后，会话才完成的订单。

### `Payment` - 可选，服务端扣款

只有暴露了服务端扣款能力的提供商，才会实现 `Payment`。Stripe 实现了；Paddle 没有。要在运行时检查：

```rust,ignore
let provider = PaymentProviderRegistry::get("stripe").unwrap();
if let Some(payment) = provider.as_payment() {
    let result = payment.charge(ChargeRequest {
        customer_ref: "cus_...".into(),
        payment_method_ref: "pm_...".into(),
        amount: Money::from_minor_units(2999, Currency::USD),
        description: Some("Pro plan one-off".into()),
        idempotency_key: Some("charge_user42_order99".into()),
        metadata: None,
    }).await?;
}
```

完整的 `Payment` 接口：

```rust,ignore
#[async_trait]
pub trait Payment: Send + Sync {
    async fn charge(&self, req: ChargeRequest) -> PaymentResult<ChargeResult>;
    async fn capture(&self, provider_transaction_id: &str) -> PaymentResult<ChargeResult>;
    async fn refund(&self, req: RefundRequest) -> PaymentResult<RefundResult>;
    async fn void(&self, provider_transaction_id: &str) -> PaymentResult<()>;
    async fn status(&self, provider_transaction_id: &str) -> PaymentResult<PaymentStatus>;
}
```

`ChargeResult` 是一个用 `kind` 打标签的枚举 - 参见[金额与 ChargeResult](#chargeresult)一节。

### `Promotions` - 可选，生成促销代码

拥有促销代码表面的提供商会实现 `Promotions`。折扣对象本身（一张打折或减免固定金额的优惠券）是提前创建好的 - 通常只创建一次，在提供商的仪表盘里 - 而这个 trait 会基于它生成*代码*，每一个都被限定给一个客户，并且有一个兑换窗口。这正是唤回流失客户和追加销售活动所需要的形态：每一个收件人都拿到一个专属代码，其他人用不了，窗口关闭之后就失效。

```rust,ignore
let provider = PaymentProviderRegistry::get("stripe").unwrap();
if let Some(promotions) = provider.as_promotions() {
    let minted = promotions.create_promotion_code(CreatePromotionCodeRequest {
        coupon_ref: "coupon_15off".into(),          // 预先创建好的优惠券
        customer_ref: "cus_...".into(),             // 只有这个客户能兑换
        expires_at: Some(chrono::Utc::now() + chrono::Duration::days(7)),
        max_redemptions: Some(1),                   // 单次使用
    }).await?;
    // 把 `minted.code` 通过邮件发给客户；他们在结账时输入它，
    // 提供商会强制执行每一项限制。
}
```

`MockPaymentProvider` 实现了 `Promotions`（代码生成为 `PROMO_MOCK_n`）并且会记录每一次请求 - 在测试里对 `recorded_promotion_requests()` 做断言。

### `Subscription` - 订阅、更新、取消、获取

```rust,ignore
#[async_trait]
pub trait Subscription: Send + Sync {
    async fn subscribe(&self, req: SubscribeRequest) -> PaymentResult<SubscriptionResult>;
    async fn update(&self, req: UpdateSubscriptionRequest) -> PaymentResult<SubscriptionResult>;
    async fn cancel(&self, provider_subscription_id: &str, at_period_end: bool) -> PaymentResult<SubscriptionResult>;
    async fn get(&self, provider_subscription_id: &str) -> PaymentResult<SubscriptionResult>;
}
```

在计费周期结束时取消（在此之前保留访问权限）：

```rust,ignore
let sub = provider.cancel(&sub_id, true).await?;
// sub.cancel_at_period_end == true, sub.status == Active

// 立即取消：
let sub = provider.cancel(&sub_id, false).await?;
// sub.status == Canceled
```

注意：`Paddle::subscribe` 会返回 `PaymentError::NotSupported` - Paddle 是通过结账完成来创建订阅的，而不是直接的 API 调用。请使用 `Checkout::start_session`，然后等待 `SubscriptionCreated` 这个 webhook。

### `CustomerStore` - 创建、更新、获取、删除

```rust,ignore
#[async_trait]
pub trait CustomerStore: Send + Sync {
    async fn create_customer(&self, req: CreateCustomerRequest) -> PaymentResult<CustomerRef>;
    async fn update_customer(&self, req: UpdateCustomerRequest) -> PaymentResult<CustomerRef>;
    async fn get_customer(&self, provider_customer_id: &str) -> PaymentResult<CustomerRef>;
    async fn delete_customer(&self, provider_customer_id: &str) -> PaymentResult<()>;
}
```

`CreateCustomerRequest` 接受 `user_id`、`email`、`name: Option<String>` 和 `metadata: Option<Value>`。返回的 `CustomerRef` 带着 `provider_customer_id` - 把它和您的用户记录存在一起，供后续调用使用。

### `WebhookHandler` - 验证、解析、提取

```rust,ignore
#[async_trait]
pub trait WebhookHandler: Send + Sync {
    fn verify(&self, ctx: &WebhookContext<'_>) -> PaymentResult<()>;
    fn parse_event(&self, body: &[u8]) -> PaymentResult<WebhookEvent>;

    /// 把实体 ID 从原始载荷里取出来，这样框架才知道要水合
    /// 哪些镜像行。默认返回一个空的 `PayloadIds`。
    fn extract_payload_ids(&self, event: &WebhookEvent) -> PayloadIds;

    /// 从一个支付/账单事件构建一个 `PaymentSnapshot`。默认
    /// 返回 `None`，这会跳过 `payments_transactions` 的 upsert。
    fn extract_payment_snapshot(&self, event: &WebhookEvent) -> Option<PaymentSnapshot>;

    /// 从一个客户事件构建一个 `CustomerSnapshot`。默认返回
    /// `None`，这会跳过对已有行的邮箱/元数据刷新。
    fn extract_customer_snapshot(&self, event: &WebhookEvent) -> Option<CustomerSnapshot>;
}
```

实践中您从来不会直接调用这些方法中的任何一个 - `webhook_routes` 会为每一个进来的 webhook 调用它们。它们被放在这个 trait 上，是为了让适配器 crate 可以用一种可测试的方式，实现提供商特有的签名验证、事件解析和载荷提取。这些 `extract_*` 方法都有合理的默认实现；框架自带的 Stripe 和 Paddle 适配器，用能感知各自提供商形态的实现重写了它们（Stripe 深入到 `data.object.*`，Paddle 深入到 `data.*`）。

## 带 flow 标记的 Inertia 载荷

`start_session` 返回一个 `SessionPayload` 枚举，它会序列化成带有一个 `flow` 判别字段的 JSON。您的前端根据 `flow` 来切换，渲染出正确的小部件：

```rust,ignore
#[serde(tag = "flow", rename_all = "snake_case")]
pub enum SessionPayload {
    StripeElements {
        client_secret: String,
        publishable_key: String,
        provider_session_id: String,
    },
    StripeCheckoutRedirect {
        url: String,
        provider_session_id: String,
    },
    PaddleInline {
        transaction_id: String,
        customer_token: Option<String>,
        client_token: String,
    },
    /// Mobile Money 流程 - 没有重定向也没有嵌入。前端会展示一条
    /// 面向用户的消息，告诉客户在他们的手机上确认
    /// （USSD 提示或者运营商 App），然后通过
    /// `provider_transaction_id` 轮询提供商获取状态更新。
    MobileMoneyPrompt {
        provider_transaction_id: String,
        message: String,
        operator: MobileMoneyOperator,
    },
    Redirect {
        url: String,
        provider_session_id: String,
    },
}
```

`StripeElements` 载荷的序列化形式：

```json
{
  "flow": "stripe_elements",
  "client_secret": "pi_..._secret_...",
  "publishable_key": "pk_live_...",
  "provider_session_id": "pi_..."
}
```

一个 `MobileMoneyPrompt` 载荷长这样 - 没有 URL，因为客户从不会离开您的页面；前端渲染 `message` 并开始轮询：

```json
{
  "flow": "mobile_money_prompt",
  "provider_transaction_id": "ch_mm_...",
  "message": "Check your phone for the MTN MoMo prompt.",
  "operator": { "kind": "mtn_momo" }
}
```

从您的控制器里，把提供商产出的那个变体原样作为 Inertia props 返回。前端集成的做法请参见[支付 - 前端集成](payments-frontend.md)。

## 镜像表

框架的迁移会创建六张表。引入这个公开别名，并把它包含进您应用的迁移器：

```rust,ignore
use sea_orm_migration::{MigrationTrait, MigratorTrait};
use suprnova::payments::migrations::CreatePaymentsTables;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            // ……您的其他迁移……
            Box::new(CreatePaymentsTables),
        ]
    }
}
```

同一个模块还导出了一个助手函数 `pub fn migrations() -> Vec<Box<dyn MigrationTrait>>`，如果您更想调用它，把结果展开进您自己的列表里的话。

### 表概览

| 表 | 用途 |
|---|---|
| `payments_customers` | 每个 `(provider, user_id)` 对一行 |
| `payments_payment_methods` | 每个客户存储的支付方式 |
| `payments_subscriptions` | 订阅生命周期状态 |
| `payments_subscription_items` | 一个订阅内部的明细项 |
| `payments_transactions` | 一次性扣款和订阅账单 |
| `payments_webhook_events` | 审计日志与幂等性防护 |

每张表都有一个 `provider_metadata` 的 JSON 列。当框架的中立表示没有覆盖某个提供商特有的字段时，就从那里读取它。

### 交易表

`payments_transactions` 把金额拆成 `amount_total_minor` 和 `amount_tax_minor`。Stripe 报告的是不含税的金额 - 交易行上的税是零，任何税务数据都存在 `provider_metadata` 里。Paddle 报告的是含税的金额，并把 `amount_tax_minor` 设成税额部分。两种表示方式都能用；用 `amount_total_minor - amount_tax_minor` 算出净额。

### Webhook 事件表

`payments_webhook_events` 有一个 `UNIQUE(provider, provider_event_id)` 索引。每一个进来的 webhook 在处理之前都会先对照这个索引检查 - 重复的会直接返回 200 OK，不会重新处理。这一点是承重的：Stripe、Paddle，以及大多数提供商都会很激进地重试失败的 webhook。

### 注意事项

领域代码是从镜像表读取的，不是直接读提供商的 API。变更操作（创建订阅、取消等等）会发到提供商那里；随之而来的 webhook 会把镜像表同步回来。这意味着在一次变更和 webhook 到达之间，存在一个短暂的窗口，您的镜像表会在这段时间里滞后。请把这一点设计进您的用户体验里（展示“处理中”状态，靠提供商的重定向 URL 来获得即时确认）。

## Webhook 处理

在启动时挂载一次这个 webhook 入口路由 - 组合模式请参见[快速上手](#快速上手)里的路由示例。`webhook_routes(db)` 返回一个 `Router`，携带着框架内置的那个单一的 `POST /webhooks/payments/{provider}` 处理程序。您可以把自己的路由链接在它上面（或者直接在您自己的 `routes!{}` 块里调用这个路由底层的原语）。

框架的处理程序对每一个请求都会做这些事：

1. 在 `PaymentProviderRegistry` 里查找这个具名的提供商。
2. 调用 `WebhookHandler::verify` 来检查签名。失败时返回 401。
3. 调用 `WebhookHandler::parse_event` 来构建一个 `WebhookEvent`。解析失败时返回 400。
4. 检查 `payments_webhook_events` 里是否已经存在一行带有相同 `(provider, provider_event_id)` 的记录。如果找到了，立即返回 200 - 这就是幂等性防护。
5. 插入这条审计行。

### WebhookEvent 结构

```rust,ignore
pub struct WebhookEvent {
    pub provider: String,
    pub provider_event_id: String,
    pub provider_event_type: String,        // 提供商原始的字符串，例如 "customer.subscription.created"
    pub neutral: Option<NeutralEventKind>,  // 映射到框架的分类体系，未映射的提供商特有事件则为 None
    pub raw_payload: Value,                 // 完整的 JSON 请求体，供兜底处理使用
}
```

`NeutralEventKind` 覆盖的是常见路径：

```rust,ignore
pub enum NeutralEventKind {
    PaymentSucceeded,
    PaymentFailed,
    PaymentRefunded,
    PaymentDisputed,
    SubscriptionCreated,
    SubscriptionUpdated,
    SubscriptionCanceled,
    InvoicePaid,
    InvoiceFailed,
    CustomerCreated,
    CustomerUpdated,
}
```

当 `neutral` 是 `None` 时，这个事件就是提供商特有的。读取 `provider_event_type` 和 `raw_payload` 来获取完整数据。

### 镜像表的水合

在这条审计行被持久化之后，框架会根据 `neutral` 把这个事件分发给相关的镜像表。**同一个事件的所有镜像写入，都和 `mark_processed` 一起发生在单个数据库事务内** - 局部的镜像状态永远不会被观察到。要么全部一起提交，要么全部回滚。

| `NeutralEventKind` | 镜像效果 |
|----------------------------------|-----------------------------------------------------------------------------------------------------|
| `SubscriptionCreated/Updated` | 调用提供商的 `Subscription::get(id)`，对 `payments_subscriptions` 做 upsert，并同步各个明细项。 |
| `SubscriptionCanceled` | 和上面一样；还会在已有行上设置 `canceled_at`，并把 `status` 翻转成 `canceled`。 |
| `PaymentSucceeded / Failed / Refunded / Disputed` | 根据提供商从 `raw_payload` 产出的快照，对 `payments_transactions` 做 upsert。 |
| `InvoicePaid / InvoiceFailed` | 对 `payments_transactions` 做 upsert，并关联上 `provider_subscription_id`。 |
| `CustomerCreated / CustomerUpdated` | 根据提供商的 `CustomerSnapshot`，更新已有 `payments_customers` 行的 `email`/`provider_metadata`。**永不插入。** |
| `None`（未映射） | 只有审计行 - 没有镜像变更。 |

客户镜像在 webhook 这条路径上，是刻意做成只更新的。`user_id` 是 `NOT NULL` 的，而只有您的应用才知道一个提供商客户属于哪个用户（这个关联是您的代码在调用完 `CustomerStore::create_customer` 之后立刻创建的）。带外客户 - 比如说在 Stripe 仪表盘里创建的那些 - 会被记录下来，但永远不会被合成进镜像里。

### 故障恢复契约

这个处理程序把提供商的重试当作恢复机制：

- **水合成功：** 事务提交，`processed_at` 被设置，`process_error` 被清空。响应：`200 ok`。
- **水合失败：** 事务回滚（没有局部镜像状态），审计行保持 `processed_at = NULL`，`process_error` 记录下这次失败。响应：`503 hydration-failed` - 提供商会带着退避策略重试。
- **提供商重试这个失败的事件：** 幂等性检查会看到这条已有的审计行，但 `processed_at IS NULL`，所以水合会再跑一次。这次重试会用当前这次尝试的结果，替换掉那个陈旧的 `process_error`。
- **提供商重试一个已经成功的事件：** 幂等性检查会看到 `processed_at IS NOT NULL`，立即返回 `200 duplicate`。不会重新水合。

一个订阅/客户事件，如果载荷里缺失了 `subscription_id`/`customer_id`，会被当作一个 `Validation` 错误处理（同样是 503 + 记录 `process_error`）。对一个格式错误的载荷默默返回成功，会让镜像陈旧下去，而运维人员却看不到。

在提供商那一侧从一个订阅里移除的项（例如用户去掉了一个席位附加项），会在下一个 `subscription.updated` webhook 到达时，从 `payments_subscription_items` 里被移除。每一次同步时，提供商的 `Subscription::get(id)` 响应都是事实来源。

## 卡以外的支付方式

`PaymentMethod` 是框架用于 `payments_payment_methods` 里存储的方式、以及任何暴露方式元数据的提供商的枚举。它覆盖了显而易见的那些情况 - 卡、银行转账、电子钱包 - 再加上在很多市场里都是头等公民的区域性支付方式：

```rust,ignore
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PaymentMethod {
    Card { brand: String, last4: String, exp_month: u8, exp_year: u16 },
    BankTransfer { bank_name: String, last4: String },
    EWallet { provider: String, identifier: String },
    /// 付款人由手机号 + 运营商 + 国家来标识。
    MobileMoney {
        operator: MobileMoneyOperator,
        phone: PhoneNumber,
        country: CountryCode,
    },
    /// 锚定加密货币 - 对大多数提供商来说等同于现金。
    Stablecoin { asset: StablecoinAsset, network: Option<String> },
    /// 非锚定的加密货币。
    Crypto { network: String, address: String },
    /// 给尚未建模的区域性/提供商特有方式用的脱围机制。
    Custom { kind: String, descriptor: String },
}

#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MobileMoneyOperator {
    MtnMomo,
    Mpesa,
    AirtelMoney,
    OrangeMoney,
    Lipila,
    Custom { identifier: String },
}

#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StablecoinAsset {
    Usdc,
    Usdt,
    Dai,
    Custom { ticker: String },
}
```

这些具名的运营商和资产，是我们已经列举过的那些。每一个上面的 `Custom { ... }` 变体，覆盖的是我们还没有钉死的区域性运营商和稳定币，所以给其中一个添加支持，并不需要框架发一个新版本。

`PhoneNumber` 和 `CountryCode` 是 `suprnova::payments` 里经过验证的 DTO - 它们会在构造时就拒绝格式错误的输入，而这正是您想要这次失败发生的地方，而不是在调用提供商的时候。

## 金额

金额用 `Money` 来表示 - 一个 `i64` 的最小货币单位计数，加上一个 `Currency`。不涉及任何 `f64`。

```rust,ignore
use suprnova::payments::{Money, Currency};
use rust_decimal::Decimal;
use std::str::FromStr;

// 来自最小货币单位（分、便士、日元等等）
let price = Money::from_minor_units(1999, Currency::USD);  // $19.99

// 来自一个十进制字符串
let price = Money::from_decimal(Decimal::from_str("19.99").unwrap(), Currency::USD);

// 零小数位货币 - 1234 个最小单位 = 1234 JPY（不需要换算）
let yen = Money::from_minor_units(1234, Currency::JPY);

// 算术运算 - 货币不匹配时会 panic
let total = price + Money::from_minor_units(100, Currency::USD);  // $20.99

// 负值代表退款或者贷记
let refund = Money::from_minor_units(-500, Currency::USD);  // -$5.00

// 读回来
println!("{} minor units in {:?}", price.minor_units(), price.currency());
```

`Add` 和 `Sub` 在货币不匹配、以及 `i64` 溢出时都会 panic。请使用这种会 panic 的算术运算来保证正确性 - 悄无声息的跨货币加法是一个 bug，不是一个特性。

## ChargeResult

`Payment::charge` 返回一个 `ChargeResult` 枚举。不是每一次扣款都会立即完成 - 3DS 强化验证和非会话场景下的卡，都可能需要一次重定向或者一次客户端操作：

```rust,ignore
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChargeResult {
    Completed {
        provider_transaction_id: String,
        amount: Money,
        status: PaymentStatus,
        provider_metadata: Value,
    },
    RedirectRequired {
        provider_transaction_id: String,
        url: String,
        return_to: Option<String>,
    },
    RequiresClientAction {
        provider_transaction_id: String,
        action_kind: String,
        client_secret: Option<String>,
        publishable_key: Option<String>,
    },
}
```

处理 `RequiresClientAction` 的办法，是把这个载荷返回给您的前端。前端用 `client_secret` + `publishable_key` 来渲染这个 3DS 质询。前端的分发代码请参见[支付 - 前端集成](payments-frontend.md)。

## 幂等键

每一个会产生变更的 DTO，都有一个可选的 `idempotency_key: Option<String>`。在可重试的网络调用上设置一个：

```rust,ignore
provider.start_session(StartSessionRequest {
    // ...
    idempotency_key: Some(format!("checkout_{}_{}", user_id, order_id)),
    // ...
}).await?;

provider.subscribe(SubscribeRequest {
    // ...
    idempotency_key: Some(format!("sub_{}_{}", user_id, plan_id)),
    // ...
}).await?;
```

Stripe 通过 `Idempotency-Key` 这个 HTTP 请求头来遵守幂等键。Paddle 也有一套等效的机制。如果一个请求在半路上失败了，而您用同一个键重试，提供商会返回原始的响应，而不是创建一次重复的扣款或者订阅。

## 判别模式

每一个声称实现了 `PaymentProvider` 的适配器，都必须通过同样的端到端流程：

```
create_customer → start_session → subscribe → get → cancel(at_period_end) → cancel(immediate) → assert as_payment invariant
```

框架自带的 `MockPaymentProvider` 通过了这个流程：

```rust,ignore
use suprnova::payments::*;

#[tokio::test]
async fn discriminator_flow() {
    let provider = MockPaymentProvider::new();

    let cus = provider.create_customer(CreateCustomerRequest {
        user_id: "user_42".into(),
        email: "alice@example.com".into(),
        name: Some("Alice".into()),
        metadata: None,
    }).await.unwrap();

    let session = provider.start_session(StartSessionRequest {
        mode: SessionMode::Subscription,
        customer_ref: cus.provider_customer_id.clone(),
        price_refs: vec!["price_pro_monthly".into()],
        success_return_url: "https://app.example/billing/success".into(),
        cancel_return_url: "https://app.example/billing/cancel".into(),
        amount_hint: None,
        idempotency_key: Some("idem_1".into()),
        metadata: None,
    }).await.unwrap();
    assert!(matches!(session, SessionPayload::Redirect { .. }));

    let sub = provider.subscribe(SubscribeRequest {
        customer_ref: cus.provider_customer_id.clone(),
        price_refs: vec!["price_pro_monthly".into()],
        trial_days: None,
        idempotency_key: Some("idem_2".into()),
        metadata: None,
    }).await.unwrap();
    assert_eq!(sub.status, SubscriptionStatus::Active);

    // 在计费周期结束时取消
    let s = provider.cancel(&sub.provider_subscription_id, true).await.unwrap();
    assert!(s.cancel_at_period_end);

    // 立即取消
    let s = provider.cancel(&sub.provider_subscription_id, false).await.unwrap();
    assert_eq!(s.status, SubscriptionStatus::Canceled);

    // MockPaymentProvider 故意省略了 Payment（Paddle 风格的可选项）
    let p: &dyn PaymentProvider = &provider;
    assert!(p.as_payment().is_none());
}
```

`MockPaymentProvider` 没有实现 `Payment` - 这演练的是和 Paddle 一样的不变量。`StripeProvider` 和 `PaddleProvider` 在集成测试里，都会针对真实的 API，通过同样的流程。

## 多提供商应用

在启动时注册两个适配器，然后根据每个客户的记录是在哪里创建的来分发：

```rust,ignore
PaymentProviderRegistry::bind("stripe", Arc::new(stripe_provider));
PaymentProviderRegistry::bind("paddle", Arc::new(paddle_provider));

// 之后，逐请求：
let provider_name = user.payment_provider.as_str(); // "stripe" 或者 "paddle"
let provider = PaymentProviderRegistry::get(provider_name).expect("unknown provider");
let sub = provider.cancel(&sub_id, true).await?;
```

常见用法：把欧盟客户路由给 Paddle（为了它的 MoR 税务处理），把美国客户路由给 Stripe；在提供商之间 A/B 测试结账转化率；用一个提供商处理订阅，用另一个提供商处理一次性扣款。

## 从 Laravel Cashier 迁移

Cashier 在设计上就只支持 Stripe。Suprnova 开箱即支持多提供商。速查映射：

| Laravel Cashier | Suprnova |
|---|---|
| `$user->newSubscription('default', 'price_pro')->create()` | `provider.subscribe(SubscribeRequest { ... }).await` |
| `$user->subscription('default')->cancel()` | `provider.cancel(&sub_id, true).await` |
| `Cashier::webhookHandler` | `webhook_routes(db.clone())` |
| `$user->createAsStripeCustomer()` | `provider.create_customer(CreateCustomerRequest { ... }).await` |
| `$user->charge(1999, 'pm_...')` | `payment.charge(ChargeRequest { ... }).await`（如果提供商支持的话） |
| `$invoice->download()` | 没有内置；从交易镜像表里读取 `provider_metadata["invoice_pdf_url"]` |

## 下一步

- [支付 - Stripe 适配器](payments-stripe.md) - 网关流程的细节：PaymentIntents、webhook 签名格式、事件类型映射
- [支付 - Paddle 适配器](payments-paddle.md) - MoR 流程的细节：由结账驱动的订阅创建、税务处理、通知验证
- [支付 - 前端集成](payments-frontend.md) - Svelte 5、React 19 和 Vue 3.5 的按 flow 分发示例
- [编写支付提供商适配器](payments-provider-guide.md) - 从头到尾构建您自己的适配器 crate
- [数据库](database.md) - 镜像表所依托的那层 SeaORM
