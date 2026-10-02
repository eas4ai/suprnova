# 编写支付提供商适配器

本指南会带您走一遍构建一个第三方适配器 crate 的过程 - `suprnova-payments-mollie` - 它插入到 Suprnova 那个提供商中立的支付表面里。读完之后，您会有一个能自己注册、能通过判别流程、并且只需一次 `cargo add` 就能塞进任何 Suprnova 应用的 crate。

同样的结构适用于任何提供商：Square、Braintree、Adyen，或者任何其他带 HTTP API 的东西。

### 为什么 Suprnova 有所不同

Laravel 把 Cashier 作为一个第一方的 Stripe 集成来发布。它对 Stripe 这条路径来说很出色，但它把一个提供商的词汇固化进了框架里 - 添加第二个提供商，意味着要么派生 Cashier，要么在它旁边另建一套并行的表面。

Suprnova 让每一个提供商都遵守同样的五个 trait 约定：`Checkout`、`Subscription`、`CustomerStore`、`WebhookHandler`，以及给服务端扣款提供商用的可选 `Payment`。领域代码手上始终只有从注册表拿到的 `Arc<dyn PaymentProvider>`。把 Stripe 换成 Paddle（或者换成您正要编写的这个 Mollie 适配器），是一次 bootstrap 变更，不是一次代码变更。位于 `crates/suprnova-payments-stripe/` 和 `crates/suprnova-payments-paddle/` 的参考适配器，证明了这个 trait 约定对两种截然不同的商业模式都成立 - 直接扣款网关，以及记录商户 - 而您的适配器，也会嵌进同样的形态里。

## 1. 创建工作空间成员 crate

在仓库根目录下：

```bash
cargo new --lib crates/suprnova-payments-mollie
```

把它添加进您根目录的 `Cargo.toml`：

```toml
[workspace]
members = [
    "framework",
    "app",
    "suprnova-cli",
    "suprnova-macros",
    "crates/suprnova-payments-mollie",  # 添加这一行
]
```

（这两个参考适配器 - `crates/suprnova-payments-stripe` 和 `crates/suprnova-payments-paddle` - 就住在同一个 `crates/` 目录里，是配合本指南阅读的好模板。）

**`crates/suprnova-payments-mollie/Cargo.toml`：**

```toml
[package]
name = "suprnova-payments-mollie"
version.workspace = true
edition.workspace = true
license.workspace = true
description = "Mollie payment adapter for Suprnova"

[dependencies]
suprnova = { path = "../../framework" }
async-trait = "0.1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
inventory = "0.3"
tracing = "0.1"
tokio = { version = "1", features = ["macros", "rt"] }
# 您的 Mollie SDK：
mollie-rs = "0.1"
hmac = "0.12"   # 用于 webhook 的 HMAC 验证
sha2 = "0.10"
hex = "0.4"

[dev-dependencies]
tokio = { version = "1", features = ["full"] }
```

## 2. 源文件布局

照着框架自带适配器用的结构来做：

```
crates/suprnova-payments-mollie/src/
├── lib.rs          # MollieProvider 结构体，PaymentProvider 实现，from_env
├── checkout.rs     # Checkout 实现
├── customer.rs     # CustomerStore 实现
├── subscription.rs # Subscription 实现
├── webhook.rs      # WebhookHandler 实现
├── event_map.rs    # 提供商事件字符串 → NeutralEventKind
└── payment.rs      # Payment 实现（如果 Mollie 支持服务端扣款）
```

## 3. `lib.rs` - 这个提供商结构体

```rust,ignore
use async_trait::async_trait;
use suprnova::payments::{Payment, PaymentProvider};

mod checkout;
mod customer;
mod event_map;
mod payment;
mod subscription;
mod webhook;

pub use event_map::mollie_event_to_neutral;

/// Suprnova 那个提供商中立的支付表面的 Mollie 适配器。
#[derive(Clone, Debug)]
pub struct MollieProvider {
    /// Mollie 的 API 密钥（`test_…` / `live_…`）。
    api_key: String,
    /// Webhook 签名密钥 - 用于 HMAC 验证。
    webhook_secret: String,
    /// HTTP 客户端 - 在各个请求之间共享。
    client: reqwest::Client,
}

impl MollieProvider {
    pub fn new(api_key: impl Into<String>, webhook_secret: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            webhook_secret: webhook_secret.into(),
            client: reqwest::Client::new(),
        }
    }

    /// 从环境变量构造。
    ///
    /// 读取：
    /// - `MOLLIE_API_KEY`
    /// - `MOLLIE_WEBHOOK_SECRET`
    pub fn from_env() -> Result<Self, String> {
        let api_key = std::env::var("MOLLIE_API_KEY")
            .map_err(|_| "MOLLIE_API_KEY not set".to_string())?;
        let webhook_secret = std::env::var("MOLLIE_WEBHOOK_SECRET")
            .map_err(|_| "MOLLIE_WEBHOOK_SECRET not set".to_string())?;
        Ok(Self::new(api_key, webhook_secret))
    }
}

impl PaymentProvider for MollieProvider {
    fn name(&self) -> &'static str {
        "mollie"
    }

    // 只有当您也实现了 `Payment`（服务端扣款）时，才重写 `as_payment()`。
    // `PaymentProvider` 上的默认实现会返回 `None` - 如果 Mollie
    // 只支持结账/是 MoR 风格的，就完全不要写这个重写。
    fn as_payment(&self) -> Option<&dyn Payment> {
        Some(self)
    }
}
```

`PaymentProvider` 是那个总括 trait - 它的 supertrait 子句是 `Checkout + Subscription + CustomerStore + WebhookHandler`，所以在这四个都被实现之前，编译器会拒绝绑定您的提供商。第五个 trait，`Payment`，是**可选**的 - 只有暴露了服务端扣款的提供商才实现它，`as_payment()` 会把结果报告给框架。默认的 `as_payment()` 返回 `None`，所以如果您的提供商不做服务端扣款，就完全不要写这个重写。

## 4. 实现这四个必需的 trait

### `checkout.rs`

```rust,ignore
use async_trait::async_trait;
use suprnova::payments::{
    Checkout, PaymentError, PaymentResult, SessionMode, SessionPayload, StartSessionRequest,
};

use crate::MollieProvider;

#[async_trait]
impl Checkout for MollieProvider {
    async fn start_session(&self, req: StartSessionRequest) -> PaymentResult<SessionPayload> {
        // 调用 Mollie 的 API 来创建一个支付或者订单。
        // 把响应映射成 SessionPayload 的某一个变体。
        // Mollie 用的是托管结账页面，所以 Redirect 是天然合适的。
        let checkout_url = self.create_mollie_payment(&req).await
            .map_err(|e| PaymentError::Internal(format!("Mollie checkout error: {e}")))?;

        Ok(SessionPayload::Redirect {
            url: checkout_url,
            provider_session_id: "mollie_session_id_here".into(),
        })
    }
}

impl MollieProvider {
    async fn create_mollie_payment(&self, req: &StartSessionRequest) -> Result<String, mollie_rs::Error> {
        // 在这里接上 Mollie SDK 的调用。
        // 返回这个托管结账 URL。
        todo!("Mollie payment creation")
    }
}
```

### `customer.rs`

```rust,ignore
use async_trait::async_trait;
use suprnova::payments::{
    CreateCustomerRequest, CustomerRef, CustomerStore, PaymentError, PaymentResult,
    UpdateCustomerRequest,
};

use crate::MollieProvider;

#[async_trait]
impl CustomerStore for MollieProvider {
    async fn create_customer(&self, req: CreateCustomerRequest) -> PaymentResult<CustomerRef> {
        // 向 Mollie 发送 POST /v2/customers
        Err(PaymentError::Internal("not yet implemented".into()))
    }

    async fn update_customer(&self, req: UpdateCustomerRequest) -> PaymentResult<CustomerRef> {
        // PATCH /v2/customers/{id}
        Err(PaymentError::Internal("not yet implemented".into()))
    }

    async fn get_customer(&self, provider_customer_id: &str) -> PaymentResult<CustomerRef> {
        // GET /v2/customers/{id}
        Err(PaymentError::Internal("not yet implemented".into()))
    }

    async fn delete_customer(&self, provider_customer_id: &str) -> PaymentResult<()> {
        // DELETE /v2/customers/{id}
        Err(PaymentError::Internal("not yet implemented".into()))
    }
}
```

### `subscription.rs`

```rust,ignore
use async_trait::async_trait;
use suprnova::payments::{
    PaymentError, PaymentResult, SubscribeRequest, Subscription, SubscriptionResult,
    UpdateSubscriptionRequest,
};

use crate::MollieProvider;

#[async_trait]
impl Subscription for MollieProvider {
    async fn subscribe(&self, req: SubscribeRequest) -> PaymentResult<SubscriptionResult> {
        // POST /v2/customers/{id}/subscriptions
        Err(PaymentError::Internal("not yet implemented".into()))
    }

    async fn update(&self, req: UpdateSubscriptionRequest) -> PaymentResult<SubscriptionResult> {
        // PATCH /v2/customers/{id}/subscriptions/{sub_id}
        Err(PaymentError::Internal("not yet implemented".into()))
    }

    async fn cancel(
        &self,
        provider_subscription_id: &str,
        at_period_end: bool,
    ) -> PaymentResult<SubscriptionResult> {
        if at_period_end {
            // 把取消日期设成周期结束
        } else {
            // DELETE /v2/customers/{id}/subscriptions/{sub_id}
        }
        Err(PaymentError::Internal("not yet implemented".into()))
    }

    async fn get(&self, provider_subscription_id: &str) -> PaymentResult<SubscriptionResult> {
        // GET /v2/customers/{id}/subscriptions/{sub_id}
        Err(PaymentError::Internal("not yet implemented".into()))
    }
}
```

如果您的提供商不支持某个方法，就返回 `PaymentError::NotSupported`：

```rust,ignore
Err(PaymentError::NotSupported(
    "Mollie creates subscriptions via checkout - use start_session instead".into()
))
```

### `payment.rs` - 服务端扣款（可选）

只有当您的提供商支持针对一个已保存支付方式的直接服务端扣款时，才实现这个。如果您跳过这个，就把 `lib.rs` 里的 `as_payment()` 重写去掉。

```rust,ignore
use async_trait::async_trait;
use suprnova::payments::{
    ChargeRequest, ChargeResult, Payment, PaymentError, PaymentResult, PaymentStatus,
    RefundRequest, RefundResult,
};

use crate::MollieProvider;

#[async_trait]
impl Payment for MollieProvider {
    async fn charge(&self, req: ChargeRequest) -> PaymentResult<ChargeResult> {
        Err(PaymentError::Internal("not yet implemented".into()))
    }

    async fn capture(&self, provider_transaction_id: &str) -> PaymentResult<ChargeResult> {
        Err(PaymentError::Internal("not yet implemented".into()))
    }

    async fn refund(&self, req: RefundRequest) -> PaymentResult<RefundResult> {
        // POST /v2/payments/{id}/refunds
        Err(PaymentError::Internal("not yet implemented".into()))
    }

    async fn void(&self, provider_transaction_id: &str) -> PaymentResult<()> {
        Err(PaymentError::Internal("not yet implemented".into()))
    }

    async fn status(&self, provider_transaction_id: &str) -> PaymentResult<PaymentStatus> {
        Err(PaymentError::Internal("not yet implemented".into()))
    }
}
```

## 5. 把提供商事件映射到 `NeutralEventKind`

**`event_map.rs`：**

```rust,ignore
use suprnova::payments::NeutralEventKind;

/// 把一个 Mollie webhook 事件类型字符串，映射到框架的中立分类体系上。
/// 对没有中立对应物的提供商特有事件，返回 `None`。
pub fn mollie_event_to_neutral(event_type: &str) -> Option<NeutralEventKind> {
    match event_type {
        // Mollie 的支付
        "payment.paid"          => Some(NeutralEventKind::PaymentSucceeded),
        "payment.failed"        => Some(NeutralEventKind::PaymentFailed),
        "payment.expired"       => Some(NeutralEventKind::PaymentFailed),
        "refund.created"        => Some(NeutralEventKind::PaymentRefunded),
        "chargeback.created"    => Some(NeutralEventKind::PaymentDisputed),
        // Mollie 的订阅
        "subscription.created"  => Some(NeutralEventKind::SubscriptionCreated),
        "subscription.updated"  => Some(NeutralEventKind::SubscriptionUpdated),
        "subscription.canceled" => Some(NeutralEventKind::SubscriptionCanceled),
        // Mollie 的订单/账单
        "order.paid"            => Some(NeutralEventKind::InvoicePaid),
        // 客户事件
        "customer.created"      => Some(NeutralEventKind::CustomerCreated),
        "customer.updated"      => Some(NeutralEventKind::CustomerUpdated),
        // 提供商特有 - 落进 raw_payload 里
        _                       => None,
    }
}
```

至少要覆盖上面列出的这些事件。对任何不在这个中立分类体系里的事件，返回 `None` - 它仍然会以 `provider_event_type` + `raw_payload` 的形式，被持久化进 `payments_webhook_events`，这样领域代码就能读取它。

## 6. 实现 webhook 签名验证

**`webhook.rs`：**

Mollie 用 HMAC-SHA256 给 webhook 载荷签名。请始终用常量时间比较签名，以防范时序攻击。

```rust,ignore
use async_trait::async_trait;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use suprnova::payments::{
    NeutralEventKind, PaymentError, PaymentResult, WebhookContext, WebhookEvent, WebhookHandler,
};

use crate::{MollieProvider, event_map::mollie_event_to_neutral};

type HmacSha256 = Hmac<Sha256>;

#[async_trait]
impl WebhookHandler for MollieProvider {
    fn verify(&self, ctx: &WebhookContext<'_>) -> PaymentResult<()> {
        // 读取 Mollie 发送的这个签名请求头。
        // 确切的请求头名字和签名方案 - 请查阅您那个版本的 Mollie 文档。
        let signature = ctx
            .headers
            .get("X-Mollie-Signature")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| PaymentError::WebhookSignature(
                "missing X-Mollie-Signature header".into()
            ))?;

        // 对原始请求体计算出期望的 HMAC-SHA256。
        let mut mac = HmacSha256::new_from_slice(self.webhook_secret.as_bytes())
            .map_err(|e| PaymentError::Internal(format!("HMAC init: {e}")))?;
        mac.update(ctx.body);

        // 解码这个十六进制编码的、收到的签名。
        let received = hex::decode(signature)
            .map_err(|_| PaymentError::WebhookSignature("non-hex signature".into()))?;

        // 常量时间比较。
        mac.verify_slice(&received)
            .map_err(|_| PaymentError::WebhookSignature("signature mismatch".into()))
    }

    fn parse_event(&self, body: &[u8]) -> PaymentResult<WebhookEvent> {
        // Mollie 发送的是 JSON - 解析它。
        let raw: serde_json::Value = serde_json::from_slice(body)
            .map_err(|e| PaymentError::Validation(format!("invalid mollie webhook body: {e}")))?;

        let event_id = raw["id"].as_str()
            .ok_or_else(|| PaymentError::Validation("missing event id".into()))?
            .to_string();

        // 在某些 webhook 形态里，Mollie 用的是资源类型，而不是事件类型字符串。
        // 请适配您那个 SDK 版本实际发送的东西。
        let event_type = raw["resource"].as_str()
            .unwrap_or("unknown")
            .to_string();

        let neutral = mollie_event_to_neutral(&event_type);

        Ok(WebhookEvent {
            provider: "mollie".into(),
            provider_event_id: event_id,
            provider_event_type: event_type,
            neutral,
            raw_payload: raw,
        })
    }
}
```

关键点：

- `PaymentError::WebhookSignature(String)` 是任何签名失败情形共用的单一变体 - 缺失请求头、编码格式错误、不匹配。框架的 webhook 路由会把每一个 `WebhookSignature(_)` 都当作 401 处理。
- 对无法解析的请求体，使用 `PaymentError::Validation(String)`。webhook 路由会在任何解析失败时返回 400。
- 框架的 `webhook_routes` 处理程序，会在 `parse_event` 之前调用 `verify`，然后在一个数据库事务内做水合。水合失败会返回 503，这样提供商就会重试。
- 永远不要记录原始密钥或者收到的签名。

### 镜像表的水合：`extract_payload_ids` + `extract_payment_snapshot` + `extract_customer_snapshot`

在 `parse_event` 返回一个 `WebhookEvent` 之后，框架的 webhook 路由会对镜像表做水合。三个可选的 trait 方法驱动着这件事 - 它们都有安全的、什么都不做的默认实现，所以一个适配器可以不实现它们就发布出去，仍然能通过审计层：

```rust,ignore
fn extract_payload_ids(&self, event: &WebhookEvent) -> PayloadIds;
fn extract_payment_snapshot(&self, event: &WebhookEvent) -> Option<PaymentSnapshot>;
fn extract_customer_snapshot(&self, event: &WebhookEvent) -> Option<CustomerSnapshot>;
```

`PayloadIds` 是解析出来的事件和框架的镜像逻辑之间的桥梁。实现它，这样框架才能找到正确的实体：

```rust,ignore
pub struct PayloadIds {
    pub subscription_id: Option<String>,
    pub customer_id: Option<String>,
    pub transaction_id: Option<String>,
}
```

对每一个 `neutral` 值，填充提供商载荷里暴露出来的那些 ID。订阅事件应该设置 `subscription_id`，这样框架就能调用 `Subscription::get(id)`，从权威状态刷新镜像。客户事件设置 `customer_id`。支付/账单事件设置 `transaction_id`，如果是一次循环扣款，再加上 `subscription_id`。

`PaymentSnapshot` 是直接从 webhook 载荷构建的 - 没有 `Payment::get` 回调。为支付/账单这两类中立事件实现它：

```rust,ignore
pub struct PaymentSnapshot {
    pub provider_transaction_id: String,
    pub provider_customer_id: String,
    pub provider_subscription_id: Option<String>,
    pub amount_total_minor: i64,
    pub amount_tax_minor: i64,
    pub currency: String,
    pub status: String,             // "succeeded" | "failed" | "refunded" | "disputed"
    pub paid_at: Option<DateTime<Utc>>,
    pub provider_metadata: Value,   // 通常是载荷里的那个实体对象
}
```

Stripe 的参考实现，对 `PaymentIntent`/`Charge` 事件读取 `data.object.{id,amount,currency,customer}`，对 `Invoice` 事件读取 `data.object.{id,amount_paid,tax,currency,customer,subscription,status_transitions.paid_at}`。Paddle 的读取的是 `data.{id,customer_id,currency_code,details.totals.{total,tax},billed_at,subscription_id}`。照着匹配您提供商载荷形态的那套约定来做 - 框架不关心您怎么提取，只关心这份快照是不是正确的。

如果您从 `extract_payment_snapshot` 返回 `None`，审计行仍然会被写入，但 `payments_transactions` 不会被动。对订阅/客户事件，或者对任何载荷里没带足够信息去填充一行的支付事件，这就是正确的返回值。

`CustomerSnapshot` 让客户镜像的同步由提供商驱动（框架里没有硬编码的 JSON 路径）：

```rust,ignore
pub struct CustomerSnapshot {
    pub provider_customer_id: String,
    pub email: Option<String>,
    pub provider_metadata: Value,
}
```

框架只会在这份快照提供了邮箱时，才执行 `email = Set(snapshot.email)`；`provider_metadata` 总是会被替换成提供商那一侧看到的客户视图（`updated_at` 也总会被推进）。客户镜像行永远只会被**更新** - 永不插入 - 因为 `user_id` 是 `NOT NULL` 的，而应用是通过 `CustomerStore::create_customer` 来拥有这个用户 ↔ 客户关联的。

### 失败语义

如果 `extract_payload_ids` 在一个订阅事件上对 `subscription_id` 返回 `None`（或者在一个客户事件上对 `customer_id` 返回 `None`），框架会把它当作一个 `Validation` 错误：水合事务会回滚，审计行的 `process_error` 会被设置，HTTP 响应是**503 hydration-failed**，这样提供商就会重试。对一个格式错误的载荷默默返回成功，会让镜像陈旧下去，而运维人员却看不到 - 提供商的重试就是恢复机制。

这份契约意味着，一个适配器的提取器必须老老实实地填充相关的 ID。返回 `None` 是留给那些您的提供商完全无法转换的事件的（例如一个载荷里没有扣款 ID 的支付事件），不是留给“我懒得解析这一个”的。

## 7. 在应用启动时注册

有两种机制可用 - 选一个：

### 运行时注册（推荐给用环境变量做配置的应用）

```rust,ignore
use std::sync::Arc;
use suprnova::payments::PaymentProviderRegistry;
use suprnova_payments_mollie::MollieProvider;

let mollie = MollieProvider::from_env().expect("Mollie env vars not set");
PaymentProviderRegistry::bind("mollie", Arc::new(mollie));
```

### 通过 `inventory` 做编译期注册

给那些想要零配置注册的适配器 crate 用的 - 当您发布一个库、让使用者只需要 `cargo add` 就行、不需要任何启动时接线时很有用：

```rust,ignore
use suprnova::payments::{PaymentProviderEntry, PaymentProviderRegistry};
use inventory;

// 在 lib.rs 里，在一个静态初始化器里：
inventory::submit!(PaymentProviderEntry {
    name: "mollie",
    factory: || Arc::new(MollieProvider::from_env().expect("Mollie env not set")),
});
```

`inventory::submit!` 会在 `main` 之前运行。这个工厂闭包，会在这个注册表第一次被访问时调用一次。

## 8. 通过判别测试

每一个适配器 crate 都应该包含一个集成测试，从头到尾证明这个 trait 约定是正确的。这就是那份健全性证明 - 如果这个测试通过了，这个提供商就能插进任何 Suprnova 应用，而不会有意外。

```rust,ignore
// tests/discriminator.rs（位于 crates/suprnova-payments-mollie/ 内部）

use suprnova::payments::*;
use suprnova_payments_mollie::MollieProvider;

/// 需要设置 MOLLIE_API_KEY 和 MOLLIE_WEBHOOK_SECRET。
/// 运行方式：cargo test --test discriminator -- --ignored
#[tokio::test]
#[ignore = "requires live Mollie sandbox credentials"]
async fn discriminator_flow() {
    let provider = MollieProvider::from_env().expect("Mollie env vars not set");

    // 1. 创建客户
    let cus = provider.create_customer(CreateCustomerRequest {
        user_id: "test_user_1".into(),
        email: "test@example.com".into(),
        name: Some("Test User".into()),
        metadata: None,
    }).await.expect("create_customer failed");
    assert!(!cus.provider_customer_id.is_empty());

    // 2. 开始一个结账会话
    let session = provider.start_session(StartSessionRequest {
        mode: SessionMode::Subscription,
        customer_ref: cus.provider_customer_id.clone(),
        price_refs: vec!["your_mollie_plan_id".into()],
        success_return_url: "https://app.example/billing/success".into(),
        cancel_return_url: "https://app.example/billing/cancel".into(),
        amount_hint: None,
        idempotency_key: Some("discriminator_test_checkout".into()),
        metadata: None,
    }).await.expect("start_session failed");
    assert!(matches!(session, SessionPayload::Redirect { .. }));

    // 3. 直接订阅（如果您的提供商支持；Mollie 可能需要走结账）
    let sub = provider.subscribe(SubscribeRequest {
        customer_ref: cus.provider_customer_id.clone(),
        price_refs: vec!["your_mollie_plan_id".into()],
        trial_days: None,
        idempotency_key: Some("discriminator_test_sub".into()),
        metadata: None,
    }).await.expect("subscribe failed");
    assert_eq!(sub.status, SubscriptionStatus::Active);

    // 4. 读回来
    let fetched = provider.get(&sub.provider_subscription_id).await.expect("get failed");
    assert_eq!(fetched.provider_subscription_id, sub.provider_subscription_id);

    // 5. 在周期结束时取消
    let s = provider.cancel(&sub.provider_subscription_id, true).await.expect("cancel failed");
    assert!(s.cancel_at_period_end);

    // 6. 立即取消
    let s = provider.cancel(&sub.provider_subscription_id, false).await.expect("cancel failed");
    assert_eq!(s.status, SubscriptionStatus::Canceled);

    // 7. 验证 as_payment() 的不变量
    let p: &dyn PaymentProvider = &provider;
    // 如果您实现了 Payment：assert!(p.as_payment().is_some())
    // 如果您没有实现 Payment：assert!(p.as_payment().is_none())
    let _ = p.as_payment();
}
```

用 `#[ignore]` 把关这些真实的集成测试，这样 `cargo test` 在 CI 里没有凭据也能通过。针对一个沙盒账户，用 `-- --ignored` 显式运行它们。

## 9. `PaymentError` 变体参考

这个完整的枚举位于 `framework/src/payments/error.rs`。挑选与实际出错情况相匹配的那个变体：

| 变体 | 什么时候用 |
|---|---|
| `Provider(String)` | 提供商的 API 返回了一个您不需要再进一步转换的错误 |
| `Validation(String)` | 请求字段无效，或者一个 webhook 请求体解析不了 |
| `NotSupported(String)` | 这个方法对这个提供商不适用（例如 Paddle 的 `subscribe`） |
| `Declined { reason, decline_code }` | 卡被拒绝 - 当提供商给出一个 `decline_code` 时，把它转发过去 |
| `Authentication(String)` | 提供商拒绝了您的 API 密钥或者凭据 |
| `NotFound(String)` | 客户、订阅或者交易 ID 不存在 |
| `WebhookSignature(String)` | 任何签名失败 - 缺失请求头、编码格式错误，或者不匹配 |
| `InvalidPhoneNumber(String)` | 在 mobile-money 流程里，E.164 校验失败 |
| `InvalidCountryCode(String)` | ISO-3166-1 alpha-2 校验失败 |
| `Internal(String)` | 意外的 SDK 错误、网络故障、HMAC 初始化失败，或者任何其他框架侧的问题 |

webhook 路由会把这些映射到状态码：`WebhookSignature(_)` → 401，来自 `parse_event` 的 `Validation(_)` → 400，来自水合的其他任何情况 → 503（这样提供商就会重试）。

一旦您的适配器编译通过、判别测试也通过之后：

- 用 `cargo add suprnova-payments-mollie --path ./crates/suprnova-payments-mollie`，把您的 crate 添加进您应用的 `Cargo.toml`。
- 像第 7 步展示的那样，在 bootstrap 里注册它。
- 在应用启动时挂载一次 `webhook_routes(db.clone())` - 同一个处理程序会按名字分发给每一个已注册的提供商，所以挂载一次，就能同时服务 Stripe、Paddle，以及您这个新的适配器。

## 下一步

- [支付](payments.md) - 那个提供商中立的表面，以及快速上手
- [支付 - Stripe 适配器](payments-stripe.md) - 一个网关适配器的完整模板
- [支付 - Paddle 适配器](payments-paddle.md) - 一个记录商户适配器的完整模板
- [支付前端](payments-frontend.md) - 怎么渲染您的适配器返回的这个 `SessionPayload`
- [错误模型](error-model.md) - `PaymentError` 是怎么落地成一个 `HttpResponse` 的
