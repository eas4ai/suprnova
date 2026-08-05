# 支払い - フロントエンド 統合

サーバーは、あなたのInertiaページのpropsの一部として `SessionPayload` を返します。このペイロードは、フロントエンドがどのウィジェットをマウントすべきかを伝える `flow` フィールドを運びます。あなたのフロントエンドは `flow` にディスパッチし、特定のプロバイダーの名前を決して出しません。この章は、Svelte 5、React 19、Vue 3.5のディスパッチループをカバーします。Stripe Elementsのconfirm-card-paymentサイクルと、オフセッションの3DSステップアップハンドラを含みます。

`flow` の5つの取りうる値と、それに紐づくフィールド：

| `flow` | フィールド | ウィジェット |
|---|---|---|
| `stripe_elements` | `client_secret`、`publishable_key`、`provider_session_id` | Stripe Elements（埋め込みカードフォーム） |
| `stripe_checkout_redirect` | `url`、`provider_session_id` | Stripeホスト型チェックアウトへのリダイレクト |
| `paddle_inline` | `transaction_id`、`client_token`、`customer_token?` | Paddle.jsのインラインオーバーレイ |
| `mobile_money_prompt` | `provider_transaction_id`、`message`、`operator` | USSD / 通信事業者アプリのプロンプト + ポーリング |
| `redirect` | `url`、`provider_session_id` | 汎用リダイレクト（Mollie、モックなど） |

バックエンドのコントローラーは `Checkout::start_session` を呼び出し、その結果をInertiaのpropsとして返します - フロントエンドの視点では、どのアダプターが動いていても、APIは同じです。

## プロバイダーではなく `flow` にディスパッチする

あなたのチェックアウトページは、`flow` フィールドを一度読み取り、対応するウィジェットをマウントします。「Stripe」や「Paddle」の名前を出すことは決してありません - どのアダプターが選ばれたかを知っているのは、それを選んだブートストラップだけです。これが、この章の残りが組み立てられる契約です。

### Suprnovaが異なる設計を選んだ理由

Laravel Cashierは、Stripe Checkout用のBladeビュー、SCA用のpartialsの経路、そしてPaddle用の別個のSDK規約を出荷します。StripeとPaddleの経路は、フロントエンドの契約を共有しません - それぞれのプロバイダーのウィジェットは、異なるコントローラーアクションと異なるテンプレートツリーに配線されています。

Suprnovaはそれを反転させます：バックエンドは常に同じ `SessionPayload` 列挙型を返し、フロントエンドは常に `flow` で分岐します。新しいプロバイダーを追加することは、サーバーサイドで1つのバリアントを、クライアントサイドで1つの `case` を追加することを意味します - あなたのチェックアウトページの残りは動きません。モバイルマネーのバリアントがその証拠です - これはウィジェットを一切生成せず（カスタマーは自分の電話で確認します）、ディスパッチャーは、呼び出し側のコンポーネントに特別扱いを入れることなく、それを吸収します。

## Svelte 5

```svelte
<!-- src/pages/Billing/Checkout.svelte -->
<script lang="ts">
  import { page } from "@inertiajs/svelte";

  // SessionPayloadはInertiaのページpropsに届く
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
    // Stripe.jsが読み込まれていなければならない - index.htmlに追加する:
    // <script src="https://js.stripe.com/v3/"></script>
    const stripe = (window as any).Stripe(s.publishable_key);
    const elements = stripe.elements({ clientSecret: s.client_secret });

    const card = elements.create("card");
    card.mount("#card-element");

    // フォームの送信を配線する:
    const form = document.getElementById("payment-form") as HTMLFormElement;
    form?.addEventListener("submit", async (e) => {
      e.preventDefault();
      const { error, paymentIntent } = await stripe.confirmCardPayment(s.client_secret, {
        payment_method: { card },
      });
      if (error) {
        // ユーザーにエラーを表示する
        console.error(error.message);
      } else if (paymentIntent?.status === "succeeded") {
        // 支払い完了 - 遷移するか確認を表示する
        window.location.href = "/billing/success";
      }
    });
  }

  function mountPaddleInline(s: Extract<SessionPayload, { flow: "paddle_inline" }>) {
    // Paddle.jsが読み込まれていなければならない - index.htmlに追加する:
    // <script src="https://cdn.paddle.com/paddle/v2/paddle.js"></script>
    const Paddle = (window as any).Paddle;
    Paddle.Initialize({ token: s.client_token });
    Paddle.Checkout.open({
      transactionId: s.transaction_id,
      customerToken: s.customer_token,
    });
  }

  async function pollMobileMoney(txId: string) {
    // 自分自身のバックエンドをポーリングする。それはミラーのtransactionsテーブルを読む。
    // プロバイダーが通知してくると、webhookハンドラがその行を更新する。
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
  <!-- stripe_elementsのときだけレンダリングされる。それ以外は隠す -->
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

`mountedRef` のガードは、React 19のStrictModeによる開発時の二重レンダーの下で、二重マウントを防ぎます。

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

## Payment SDKを読み込む

関連するスクリプトを、あなたの `index.html`（または同等のエントリーポイント）に追加してください。あなたが選んだプロバイダーが必要とするものだけを含めてください：

```html
<!-- Stripe（stripe_elementsまたはstripe_checkout_redirectを使う場合は追加する） -->
<script src="https://js.stripe.com/v3/" crossorigin="anonymous"></script>

<!-- Paddle（paddle_inlineを使う場合は追加する） -->
<script src="https://cdn.paddle.com/paddle/v2/paddle.js" crossorigin="anonymous"></script>
```

両方のスクリプトは、ブラウザによって非同期に読み込まれます。コード分割を伴うViteを使っているなら、動的な `import()` を介してこれらを読み込むか、プロバイダーのSDKを自分でバンドルしてしまわないよう、`vite.config.ts` の中でexternalsとして含めてください。

StripeとPaddleは、どちらも、それぞれ自身のCDNからSDKを読み込むことを要求します - StripeはこれをPCI準拠の条件にしており、PaddleはライブなURL書き換えのためにこれに依存しています。Subresource Integrity（`integrity="sha384-..."`）は、どちらのスクリプトでも使えません。両ベンダーは継続的に出荷しており、安定したハッシュを公開しないためです - 信頼の境界は、HTTPS接続とベンダーのCDNです。あなたの脅威モデルが、埋め込むものすべてにSRIを要求するなら、それは、支払いUIのすべてを、あなた自身のページの中ではなく、ベンダーがホストするチェックアウト（`stripe_checkout_redirect`、あるいはサーバー発行のリダイレクトから呼び出されるPaddleのホスト型オーバーレイ）に留めておくべきだという信号です。

## TypeScript 型

上記の各例で示した `SessionPayload` 型は、Rustの列挙型のシリアライズされた形に一致する判別可能なユニオン型です。あなたの `SessionPayload` が `#[derive(InertiaProps)]` ラッパーを介して公開されているなら、`suprnova generate-types` で自動的に生成できます。あるいは、示したように手動で定義してください。

## モバイルマネーのポーリング

`mobile_money_prompt` は、プロンプトが届いた後、カスタマーがあなたのページに一切触れない唯一のflowです。カスタマーは自分の電話で確認し（USSDメニューまたは通信事業者アプリのプッシュ通知）、プロバイダーはあなたのwebhookハンドラに通知し、あなたのフロントエンドは、そのトランザクションが確定したことを見つけ出さなければなりません。

`provider_transaction_id` によってミラーの `payments_transactions` テーブルを読む、小さなステータスエンドポイントを配線してください。`webhook_routes(db)` によってインストールされたwebhookハンドラが、その行のstatusカラムを最新に保ちます - あなたのエンドポイントは、それをそのまま反映するだけです：

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

上記の各例で示したフロントエンドの `pollMobileMoney` ヘルパーは、5分の上限を伴って、3秒ごとにそのエンドポイントを叩きます。ステータス文字列は `PaymentStatus` 列挙型から来て、snake_caseでシリアライズされます：`created`、`requires_action`、`pending`、`processing`、`authorized`、`expired`、`succeeded`、`failed`、`canceled`、`refunded`、`partially_refunded`、`disputed`。

## エラー処理 - `RequiresClientAction`

`Payment::charge`（サーバーサイド確定）が `ChargeResult::RequiresClientAction` を返すとき、バックエンドはその結果をJSONへシリアライズし、フロントエンドへ返します。これは、カード発行会社が追加の認証を要求する、オフセッションの3DSステップアップフローで発生します。

JSONはこのようになります：

```json
{
  "kind": "requires_client_action",
  "provider_transaction_id": "pi_...",
  "action_kind": "stripe_3ds",
  "client_secret": "pi_..._secret_...",
  "publishable_key": "pk_live_..."
}
```

`client_secret` と `publishable_key` は、Rust側では `Option<String>` であり、アクションがそれらを必要としないときは、JSONから欠落します。プロバイダーのSDKに渡す前に、必ず両方をnullチェックしてください。そして、ディスパッチを駆動させるのは `action_kind` にしてください - このフィールドは常に存在します。

あなたのバックエンドコントローラーは、これを検出し、個別のInertiaのpropとして、あるいはフロントエンドが読み取るHTTPレスポンスとして返すべきです。コントローラーパターンの例：

```rust,ignore
use suprnova::payments::ChargeResult;

let result = payment.charge(req).await?;
match result {
    ChargeResult::Completed { .. } => {
        // 成功ページへリダイレクトする
    }
    ChargeResult::RequiresClientAction { action_kind, client_secret, publishable_key, .. } => {
        return inertia.render("Billing/ThreeDSChallenge", json!({
            "action_kind": action_kind,
            "client_secret": client_secret,
            "publishable_key": publishable_key,
        }));
    }
    ChargeResult::RedirectRequired { url, .. } => {
        // ブラウザをリダイレクトする
    }
}
```

フロントエンドでは、`action_kind` にディスパッチします：

**Svelte 5:**

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
      // 3DS失敗のメッセージを表示する
    } else if (paymentIntent?.status === "succeeded") {
      window.location.href = "/billing/success";
    }
  }
</script>
```

**React 19:**

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

**Vue 3.5:**

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

`action_kind` フィールドは、プロバイダー固有の文字列です。現在、出荷されているStripeアダプターが生成する値は `"stripe_3ds"` だけです。追加のアダプターがクライアントアクションを必要とするようになれば、同じパターンに従って、それぞれ自身の `action_kind` の値を追加します - デフォルトの分岐（`console.warn("Unknown action_kind:", k)`）を書いておいてください。そうすれば、認識されない値は、支払いをサイレントに落とすのではなく、はっきりと失敗します。

## 次のステップ

- [支払い](payments.md) - 5つのトレイトのサーフェス、レジストリ、そして `SessionPayload` を生成するブートストラップのパターン
- [支払い - Stripe](payments-stripe.md) - `stripe_elements`、`stripe_checkout_redirect`、`stripe_3ds` の各flowのためのサーバーサイド設定
- [支払い - Paddle](payments-paddle.md) - `paddle_inline` flowのためのサーバーサイド設定と、Merchant of Recordの責務分担
- [支払い - プロバイダーガイド](payments-provider-guide.md) - Suprnovaが出荷していないゲートウェイのためにアダプターを書くとき、新しい `SessionPayload` バリアントを追加する
- [フロントエンド 概要](frontend.md) - Inertiaのページ設定、propの型付け、そして `usePage` がSvelte / React / Vueのスターターにどう組み込まれるか
