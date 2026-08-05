# 支付 - 前端集成

服务端会把一个 `SessionPayload` 作为您 Inertia 页面 props 的一部分返回。这个载荷带着一个 `flow` 字段，告诉前端要挂载哪个小部件；您的前端根据 `flow` 来分发，从不指名一个具体的提供商。本章涵盖 Svelte 5、React 19 和 Vue 3.5 的分发循环，包括 Stripe Elements 的 confirm-card-payment 循环，以及非会话场景下的 3DS 强化验证处理程序。

五个可能的 `flow` 值，以及它们各自关联的字段：

| `flow` | 字段 | 小部件 |
|---|---|---|
| `stripe_elements` | `client_secret`, `publishable_key`, `provider_session_id` | Stripe Elements（嵌入式的卡片表单） |
| `stripe_checkout_redirect` | `url`, `provider_session_id` | 重定向到 Stripe 托管的结账页面 |
| `paddle_inline` | `transaction_id`, `client_token`, `customer_token?` | Paddle.js 的内嵌浮层 |
| `mobile_money_prompt` | `provider_transaction_id`, `message`, `operator` | USSD/运营商 App 提示 + 轮询 |
| `redirect` | `url`, `provider_session_id` | 通用重定向（Mollie、mock 等等） |

后端控制器调用 `Checkout::start_session`，把结果作为 Inertia props 返回 - 从前端的角度看，不管背后跑的是哪个适配器，这个 API 都是一样的。

## 根据 `flow` 分发，而不是根据提供商

您的结账页面读取一次 `flow` 字段，然后挂载匹配的小部件。它从不指名“Stripe”或者“Paddle”；只有选择了这个适配器的那个 bootstrap 才知道。这就是本章余下部分建立在其上的那份契约。

### 为什么 Suprnova 有所不同

Laravel Cashier 为 Stripe Checkout 发布了一个 Blade 视图，为 SCA 发布了一条 partial 路径，为 Paddle 发布了一套独立的 SDK 约定。Stripe 和 Paddle 这两条路径并不共享一份前端契约 - 每个提供商的小部件，都接到了一个不同的控制器动作，和一棵不同的模板树上。

Suprnova 把这个反过来：后端始终返回同一个 `SessionPayload` 枚举，前端始终根据 `flow` 来切换。添加一个新的提供商，意味着在服务端加一个变体，在客户端加一个 `case`；您结账页面剩下的部分都不需要动。Mobile Money 这个变体就是证明 - 它完全不产出任何小部件（客户在自己手机上确认），而这个分发器会把它吸收掉，调用它的那个组件里不需要任何特殊处理。

## Svelte 5

```svelte
<!-- src/pages/Billing/Checkout.svelte -->
<script lang="ts">
  import { page } from "@inertiajs/svelte";

  // SessionPayload 会出现在 Inertia 的页面 props 里
  let session = $derived($page.props.session as SessionPayload);

  type MobileMoneyOperator =
    | { kind: "mtn_momo" }
    | { kind: "mpesa" }
    | { kind: "airtel_money" }
    | { kind: "orange_money" }
    | { kind: "lipila" }
    | { kind: "custom"; identifier: string };

  type SessionPayload =
    | { flow: "stripe_elements"; client_secret: string; publishable_key: string; provider_session_id: string }
    | { flow: "stripe_checkout_redirect"; url: string; provider_session_id: string }
    | { flow: "paddle_inline"; transaction_id: string; client_token: string; customer_token?: string }
    | { flow: "mobile_money_prompt"; provider_transaction_id: string; message: string; operator: MobileMoneyOperator }
    | { flow: "redirect"; url: string; provider_session_id: string };

  let mobileMessage = $state("");

  $effect(() => {
    if (!session) return;
    switch (session.flow) {
      case "stripe_elements":
        mountStripeElements(session);
        break;
      case "stripe_checkout_redirect":
        window.location.href = session.url;
        break;
      case "paddle_inline":
        mountPaddleInline(session);
        break;
      case "mobile_money_prompt":
        mobileMessage = session.message;
        pollMobileMoney(session.provider_transaction_id);
        break;
      case "redirect":
        window.location.href = session.url;
        break;
    }
  });

  async function mountStripeElements(s: Extract<SessionPayload, { flow: "stripe_elements" }>) {
    // Stripe.js 必须先被加载 - 把它加进 index.html：
    // <script src="https://js.stripe.com/v3/"></script>
    const stripe = (window as any).Stripe(s.publishable_key);
    const elements = stripe.elements({ clientSecret: s.client_secret });

    const card = elements.create("card");
    card.mount("#card-element");

    // 接上表单提交：
    const form = document.getElementById("payment-form") as HTMLFormElement;
    form?.addEventListener("submit", async (e) => {
      e.preventDefault();
      const { error, paymentIntent } = await stripe.confirmCardPayment(s.client_secret, {
        payment_method: { card },
      });
      if (error) {
        // 向用户展示错误
        console.error(error.message);
      } else if (paymentIntent?.status === "succeeded") {
        // 支付完成 - 导航过去，或者展示一个确认
        window.location.href = "/billing/success";
      }
    });
  }

  function mountPaddleInline(s: Extract<SessionPayload, { flow: "paddle_inline" }>) {
    // Paddle.js 必须先被加载 - 把它加进 index.html：
    // <script src="https://cdn.paddle.com/paddle/v2/paddle.js"></script>
    const Paddle = (window as any).Paddle;
    Paddle.Initialize({ token: s.client_token });
    Paddle.Checkout.open({
      transactionId: s.transaction_id,
      customerToken: s.customer_token,
    });
  }

  async function pollMobileMoney(txId: string) {
    // 轮询您自己的后端，它读取的是交易镜像表。
    // webhook 处理程序会在提供商通知我们时，更新这一行。
    const deadline = Date.now() + 5 * 60_000;
    while (Date.now() < deadline) {
      await new Promise((r) => setTimeout(r, 3000));
      const res = await fetch(`/billing/status?transaction_id=${encodeURIComponent(txId)}`);
      const { status } = await res.json();
      if (status === "succeeded") {
        window.location.href = "/billing/success";
        return;
      }
      if (status === "failed" || status === "canceled" || status === "expired") {
        window.location.href = "/billing/failed";
        return;
      }
    }
  }
</script>

<div id="payment-form">
  <div id="card-element"></div>
  <!-- 只在 stripe_elements 时渲染；其他情况隐藏 -->
  {#if session?.flow === "stripe_elements"}
    <button type="submit">Pay now</button>
  {/if}
  {#if session?.flow === "mobile_money_prompt"}
    <p>{mobileMessage}</p>
    <p>Waiting for confirmation…</p>
  {/if}
</div>
```

## React 19

```tsx
// src/pages/Billing/Checkout.tsx
import { useEffect, useRef, useState } from "react";
import { usePage } from "@inertiajs/react";

type MobileMoneyOperator =
  | { kind: "mtn_momo" }
  | { kind: "mpesa" }
  | { kind: "airtel_money" }
  | { kind: "orange_money" }
  | { kind: "lipila" }
  | { kind: "custom"; identifier: string };

type SessionPayload =
  | { flow: "stripe_elements"; client_secret: string; publishable_key: string; provider_session_id: string }
  | { flow: "stripe_checkout_redirect"; url: string; provider_session_id: string }
  | { flow: "paddle_inline"; transaction_id: string; client_token: string; customer_token?: string }
  | { flow: "mobile_money_prompt"; provider_transaction_id: string; message: string; operator: MobileMoneyOperator }
  | { flow: "redirect"; url: string; provider_session_id: string };

export default function Checkout() {
  const { session } = usePage<{ session: SessionPayload }>().props;
  const mountedRef = useRef(false);
  const [mobileMessage, setMobileMessage] = useState("");

  useEffect(() => {
    if (!session || mountedRef.current) return;
    mountedRef.current = true;

    switch (session.flow) {
      case "stripe_elements":
        mountStripeElements(session);
        break;
      case "stripe_checkout_redirect":
        window.location.href = session.url;
        break;
      case "paddle_inline":
        mountPaddleInline(session);
        break;
      case "mobile_money_prompt":
        setMobileMessage(session.message);
        pollMobileMoney(session.provider_transaction_id);
        break;
      case "redirect":
        window.location.href = session.url;
        break;
    }
  }, [session]);

  async function mountStripeElements(
    s: Extract<SessionPayload, { flow: "stripe_elements" }>
  ) {
    const stripe = (window as any).Stripe(s.publishable_key);
    const elements = stripe.elements({ clientSecret: s.client_secret });
    const card = elements.create("card");
    card.mount("#card-element");

    const form = document.getElementById("payment-form") as HTMLFormElement;
    form?.addEventListener("submit", async (e) => {
      e.preventDefault();
      const { error, paymentIntent } = await stripe.confirmCardPayment(s.client_secret, {
        payment_method: { card },
      });
      if (error) {
        console.error(error.message);
      } else if (paymentIntent?.status === "succeeded") {
        window.location.href = "/billing/success";
      }
    });
  }

  function mountPaddleInline(
    s: Extract<SessionPayload, { flow: "paddle_inline" }>
  ) {
    const Paddle = (window as any).Paddle;
    Paddle.Initialize({ token: s.client_token });
    Paddle.Checkout.open({
      transactionId: s.transaction_id,
      customerToken: s.customer_token,
    });
  }

  async function pollMobileMoney(txId: string) {
    const deadline = Date.now() + 5 * 60_000;
    while (Date.now() < deadline) {
      await new Promise((r) => setTimeout(r, 3000));
      const res = await fetch(`/billing/status?transaction_id=${encodeURIComponent(txId)}`);
      const { status } = await res.json();
      if (status === "succeeded") {
        window.location.href = "/billing/success";
        return;
      }
      if (status === "failed" || status === "canceled" || status === "expired") {
        window.location.href = "/billing/failed";
        return;
      }
    }
  }

  return (
    <form id="payment-form">
      <div id="card-element" />
      {session?.flow === "stripe_elements" && (
        <button type="submit">Pay now</button>
      )}
      {session?.flow === "mobile_money_prompt" && (
        <div>
          <p>{mobileMessage}</p>
          <p>Waiting for confirmation…</p>
        </div>
      )}
    </form>
  );
}
```

`mountedRef` 这道防护，防止了在 React 19 的 StrictMode 开发期双重渲染下的重复挂载。

## Vue 3.5

```vue
<!-- src/pages/Billing/Checkout.vue -->
<script setup lang="ts">
import { onMounted, ref } from "vue";
import { usePage } from "@inertiajs/vue3";

type MobileMoneyOperator =
  | { kind: "mtn_momo" }
  | { kind: "mpesa" }
  | { kind: "airtel_money" }
  | { kind: "orange_money" }
  | { kind: "lipila" }
  | { kind: "custom"; identifier: string };

type SessionPayload =
  | { flow: "stripe_elements"; client_secret: string; publishable_key: string; provider_session_id: string }
  | { flow: "stripe_checkout_redirect"; url: string; provider_session_id: string }
  | { flow: "paddle_inline"; transaction_id: string; client_token: string; customer_token?: string }
  | { flow: "mobile_money_prompt"; provider_transaction_id: string; message: string; operator: MobileMoneyOperator }
  | { flow: "redirect"; url: string; provider_session_id: string };

const page = usePage<{ session: SessionPayload }>();
const session = page.props.session;
const isStripeElements = ref(session?.flow === "stripe_elements");
const isMobileMoney = ref(session?.flow === "mobile_money_prompt");
const mobileMessage = ref(
  session?.flow === "mobile_money_prompt" ? session.message : ""
);

onMounted(() => {
  if (!session) return;
  switch (session.flow) {
    case "stripe_elements":
      mountStripeElements(session);
      break;
    case "stripe_checkout_redirect":
      window.location.href = session.url;
      break;
    case "paddle_inline":
      mountPaddleInline(session);
      break;
    case "mobile_money_prompt":
      pollMobileMoney(session.provider_transaction_id);
      break;
    case "redirect":
      window.location.href = session.url;
      break;
  }
});

async function mountStripeElements(
  s: Extract<SessionPayload, { flow: "stripe_elements" }>
) {
  const stripe = (window as any).Stripe(s.publishable_key);
  const elements = stripe.elements({ clientSecret: s.client_secret });
  const card = elements.create("card");
  card.mount("#card-element");

  const form = document.getElementById("payment-form") as HTMLFormElement;
  form?.addEventListener("submit", async (e) => {
    e.preventDefault();
    const { error, paymentIntent } = await stripe.confirmCardPayment(s.client_secret, {
      payment_method: { card },
    });
    if (error) {
      console.error(error.message);
    } else if (paymentIntent?.status === "succeeded") {
      window.location.href = "/billing/success";
    }
  });
}

function mountPaddleInline(
  s: Extract<SessionPayload, { flow: "paddle_inline" }>
) {
  const Paddle = (window as any).Paddle;
  Paddle.Initialize({ token: s.client_token });
  Paddle.Checkout.open({
    transactionId: s.transaction_id,
    customerToken: s.customer_token,
  });
}

async function pollMobileMoney(txId: string) {
  const deadline = Date.now() + 5 * 60_000;
  while (Date.now() < deadline) {
    await new Promise((r) => setTimeout(r, 3000));
    const res = await fetch(`/billing/status?transaction_id=${encodeURIComponent(txId)}`);
    const { status } = await res.json();
    if (status === "succeeded") {
      window.location.href = "/billing/success";
      return;
    }
    if (status === "failed" || status === "canceled" || status === "expired") {
      window.location.href = "/billing/failed";
      return;
    }
  }
}
</script>

<template>
  <form id="payment-form">
    <div id="card-element" />
    <button v-if="isStripeElements" type="submit">Pay now</button>
    <div v-if="isMobileMoney">
      <p>{{ mobileMessage }}</p>
      <p>Waiting for confirmation…</p>
    </div>
  </form>
</template>
```

## 加载支付 SDK

把相关的脚本添加进您的 `index.html`（或者等效的入口点）。只包含您选定的提供商所需要的那些：

```html
<!-- Stripe（如果使用 stripe_elements 或者 stripe_checkout_redirect，就添加它） -->
<script src="https://js.stripe.com/v3/" crossorigin="anonymous"></script>

<!-- Paddle（如果使用 paddle_inline，就添加它） -->
<script src="https://cdn.paddle.com/paddle/v2/paddle.js" crossorigin="anonymous"></script>
```

这两个脚本都是被浏览器异步加载的。如果您在用 Vite 的代码拆分，就通过动态的 `import()` 来加载它们，或者把它们作为外部依赖写进您的 `vite.config.ts`，这样您就不需要自己去打包这些提供商的 SDK。

Stripe 和 Paddle 都要求您从它们自己的 CDN 加载这个 SDK - Stripe 把这作为一个 PCI 合规条件，Paddle 依赖它来做实时的 URL 重写。子资源完整性（Subresource Integrity，`integrity="sha384-..."`）在这两个脚本上都用不了，因为两家供应商都在持续发布，不会公开稳定的哈希值；这里的信任边界是 HTTPS 连接，加上供应商的 CDN。如果您的威胁模型要求对您嵌入的每一样东西都做 SRI，那就是一个信号，说明应该把所有的支付 UI 都留在供应商托管的结账页面上（`stripe_checkout_redirect`，或者由服务端发起重定向调用的 Paddle 托管浮层），而不是放在您自己的页面里。

## TypeScript 类型

上面每一个例子里展示的这个 `SessionPayload` 类型，是一个匹配这个 Rust 枚举序列化形式的可判别联合类型。如果您的 `SessionPayload` 是通过一个 `#[derive(InertiaProps)]` 封装暴露出来的，就可以用 `suprnova generate-types` 自动生成它，或者像示例那样手动定义它。

## Mobile Money 轮询

`mobile_money_prompt` 是唯一一个这样的流程：提示一出现，客户就再也不会碰您的页面了。他们在手机上确认（USSD 菜单，或者运营商 App 的推送），提供商会通知您的 webhook 处理程序，而您的前端得自己发现这次交易已经结算。

接一个小小的状态端点，通过 `provider_transaction_id` 读取镜像 `payments_transactions` 表。由 `webhook_routes(db)` 安装的那个 webhook 处理程序，会让这一行的 status 列保持最新；您的端点只是把它原样反映回去：

```rust,ignore
use suprnova::{Json, Query, json_response};
use suprnova::payments::entities::transaction;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

#[derive(serde::Deserialize)]
pub struct StatusQuery {
    pub transaction_id: String,
}

pub async fn status(Query(q): Query<StatusQuery>) -> Json<serde_json::Value> {
    let db = suprnova::db().await;
    let row = transaction::Entity::find()
        .filter(transaction::Column::ProviderTransactionId.eq(q.transaction_id))
        .one(&db)
        .await
        .unwrap();
    let status = row.map(|r| r.status).unwrap_or_else(|| "pending".into());
    Json(serde_json::json!({ "status": status }))
}
```

上面每个例子里展示的那个前端 `pollMobileMoney` 助手函数，每三秒打一次那个端点，上限是五分钟。状态字符串来自 `PaymentStatus` 这个枚举，并以 snake_case 序列化：`created`、`requires_action`、`pending`、`processing`、`authorized`、`expired`、`succeeded`、`failed`、`canceled`、`refunded`、`partially_refunded`、`disputed`。

## 错误处理 - `RequiresClientAction`

当 `Payment::charge`（服务端扣款）返回 `ChargeResult::RequiresClientAction` 时，后端会把这个结果序列化成 JSON，返回给前端。这会发生在非会话场景下的 3DS 强化验证流程里，也就是发卡行要求额外认证的时候。

这个 JSON 长这样：

```json
{
  "kind": "requires_client_action",
  "provider_transaction_id": "pi_...",
  "action_kind": "stripe_3ds",
  "client_secret": "pi_..._secret_...",
  "publishable_key": "pk_live_..."
}
```

`client_secret` 和 `publishable_key` 在 Rust 那一侧是 `Option<String>`，当一个操作不需要它们时，它们在 JSON 里就会缺失。在把它们传给一个提供商 SDK 之前，请始终对两者做空值检查，并让 `action_kind` 来驱动分发 - 这个字段总是存在的。

您的后端控制器应该检测到这一点，把它作为一个独立的 Inertia prop，或者作为一个前端会去读取的 HTTP 响应返回。示例的控制器模式：

```rust,ignore
use suprnova::payments::ChargeResult;

let result = payment.charge(req).await?;
match result {
    ChargeResult::Completed { .. } => {
        // 重定向到成功页面
    }
    ChargeResult::RequiresClientAction { action_kind, client_secret, publishable_key, .. } => {
        return inertia.render("Billing/ThreeDSChallenge", json!({
            "action_kind": action_kind,
            "client_secret": client_secret,
            "publishable_key": publishable_key,
        }));
    }
    ChargeResult::RedirectRequired { url, .. } => {
        // 重定向浏览器
    }
}
```

在前端，根据 `action_kind` 来分发：

**Svelte 5：**

```svelte
<script lang="ts">
  import { page } from "@inertiajs/svelte";

  let props = $derived($page.props as {
    action_kind: string;
    client_secret?: string;
    publishable_key?: string;
  });

  $effect(() => {
    if (!props.action_kind) return;
    switch (props.action_kind) {
      case "stripe_3ds":
        handleStripe3DS(props.client_secret!, props.publishable_key!);
        break;
      default:
        console.warn("Unknown action_kind:", props.action_kind);
    }
  });

  async function handleStripe3DS(clientSecret: string, publishableKey: string) {
    const stripe = (window as any).Stripe(publishableKey);
    const { error, paymentIntent } = await stripe.handleNextAction({ clientSecret });
    if (error) {
      // 展示 3DS 失败的消息
    } else if (paymentIntent?.status === "succeeded") {
      window.location.href = "/billing/success";
    }
  }
</script>
```

**React 19：**

```tsx
import { usePage } from "@inertiajs/react";
import { useEffect } from "react";

export default function ThreeDSChallenge() {
  const { action_kind, client_secret, publishable_key } = usePage<{
    action_kind: string;
    client_secret?: string;
    publishable_key?: string;
  }>().props;

  useEffect(() => {
    if (!action_kind) return;
    if (action_kind === "stripe_3ds" && client_secret && publishable_key) {
      const stripe = (window as any).Stripe(publishable_key);
      stripe.handleNextAction({ clientSecret: client_secret }).then(
        ({ error, paymentIntent }: any) => {
          if (!error && paymentIntent?.status === "succeeded") {
            window.location.href = "/billing/success";
          }
        }
      );
    }
  }, [action_kind]);

  return <div>Completing payment authentication...</div>;
}
```

**Vue 3.5：**

```vue
<script setup lang="ts">
import { onMounted } from "vue";
import { usePage } from "@inertiajs/vue3";

const { action_kind, client_secret, publishable_key } = usePage<{
  action_kind: string;
  client_secret?: string;
  publishable_key?: string;
}>().props;

onMounted(async () => {
  if (action_kind === "stripe_3ds" && client_secret && publishable_key) {
    const stripe = (window as any).Stripe(publishable_key);
    const { error, paymentIntent } = await stripe.handleNextAction({
      clientSecret: client_secret,
    });
    if (!error && paymentIntent?.status === "succeeded") {
      window.location.href = "/billing/success";
    }
  }
});
</script>

<template>
  <p>Completing payment authentication...</p>
</template>
```

`action_kind` 字段是一个提供商特有的字符串。目前，`"stripe_3ds"` 是框架自带的 Stripe 适配器唯一会产出的值。当更多的适配器需要客户端操作时，它们会照着同样的模式，添加自己的 `action_kind` 值 - 写一个默认分支（`console.warn("Unknown action_kind:", k)`），这样一个未识别的值会明确地失败，而不是默默地把这次支付丢掉。

## 下一步

- [支付](payments.md) - 那个五个 trait 的表面、注册表，以及产出 `SessionPayload` 的 bootstrap 模式。
- [支付 - Stripe](payments-stripe.md) - `stripe_elements`、`stripe_checkout_redirect` 和 `stripe_3ds` 这些流程的服务端配置。
- [支付 - Paddle](payments-paddle.md) - `paddle_inline` 流程的服务端配置，以及记录商户的职责拆分。
- [支付 - 提供商指南](payments-provider-guide.md) - 当您为一个 Suprnova 没有自带的网关编写适配器时，添加一个新的 `SessionPayload` 变体。
- [前端](frontend.md) - Inertia 页面设置、prop 类型化，以及 `usePage` 是怎么接入您的 Svelte / React / Vue 起步模板的。
