# 支払い - Stripe アダプター

`suprnova-payments-stripe` は、Suprnovaのプロバイダーニュートラルな支払いサーフェスの参照アダプターです。`async-stripe` 1.0.0-rc.5経由でStripe APIに対して、5つの支払いトレイトすべて（`Checkout`、`Payment`、`Subscription`、`CustomerStore`、`WebhookHandler`）を実装します。あるメソッドが正確にどのStripeエンドポイントを呼ぶのか、webhookの署名形式がどう検証されるのか、PaymentIntentsが `ChargeResult` を通じてどう流れるのか、どのイベントタイプがニュートラルなイベント列挙型にマッピングされるのかを知る必要があるときは、この章に手を伸ばしてください。

トレイトの形そのもの、環境変数の設定、ブートストラップのパターンについては、まず[支払い](payments.md)を読んでください。この章は、Stripe固有の深掘りです。

## ゲートウェイであり、Merchant of Recordではない

Stripeはデフォルトでは**決済ゲートウェイ**です：資金は直接あなた自身の銀行口座に入り、あなたは税の徴収と納付、請求書発行、支払いの督促、そしてチャージバック対応の責任を負います。Paddle（[支払い - Paddle](payments-paddle.md)）とは対照的です - Paddleは Merchant of Record であり、資金を回収し、税を申告し、手数料を差し引いた分をあなたに支払います。

この章にとっての実務上の帰結：`StripeProvider` は `Payment` を実装します（サーバー上でカードを承認し、確定し、返金し、取り消すことができます）。`PaddleProvider` はそうしません。このトレイトの分割が存在するのは、2つのフローが本当に異なるからです - 時間が足りなかったからではありません。

### Stripe Managed Payments（オプトインのMerchant of Record）

Stripeの**Managed Payments**プログラムは、対象となる取引について、Stripeを Merchant of Record の座に移します - Stripeが正式な販売者になり、売上税 / VAT / GSTを計算し、徴収し、申告し、納付し、チャージバックを引き受けます。このプログラムには、厳しい統合上の制約があります：

- **ホスト型Checkoutのみ。** セッションは、Stripeのホスト型ページ上で実行されなければなりません。Elements / カスタムフローは除外されます - これが、アダプターのホスト型の一度限りの経路（下記）だけが、これと組み合わされる唯一の `OneOff` の形である理由です。
- **対象となる税コードを持つ、事前定義された価格。** 明細項目は、StripeダッシュボードでManaged-Payments対象のラベルが付いた税コードを持つ商品を参照する `price_…` オブジェクトを参照しなければなりません。その場限りの金額は拒否されます。
- **アカウントの登録。** Stripeアカウントは、このプログラムへオンボーディングされていなければなりません。登録されていないアカウントで、このフラグを持つセッションは失敗します。

`.with_managed_payments(true)` または `STRIPE_MANAGED_PAYMENTS=true` で、プロバイダーごとにこれを有効にしてください - すると、アダプターは、ホスト型の一度限りのセッションを作成するときに `managed_payments[enabled]=true` を送ります。オフの場合（デフォルト）、このフィールドは完全に省かれます。

### Suprnovaが異なる設計を選んだ理由

Laravelは、コアのドキュメントの中で、Cashierを第一級のStripe統合として出荷しています。それは便利ですが、Stripe専用です - 2番目のプロバイダーを追加するには、Cashierをフォークするか、並行するサーフェスを構築するしかありません。

Suprnovaは、Stripeを一定の距離を置いて扱います。Stripeアダプターは、他のどのプロバイダーも実装する同じ5つのトレイトに対して、自分自身を登録する1つのクレートです。あなたのドメインコードは `StripeProvider` の名前を決して呼びません - レジストリから解決された `Arc<dyn PaymentProvider>` に対して `provider.charge(...)` を呼ぶだけであり、Stripeの振る舞いは、Paddleの振る舞いから一度の切り替えで変わります。後になってMollieを追加したり、まだ存在しない地域のゲートウェイを配線したりするときも、あなたは同じ5つのトレイトを実装するだけで、アプリの残りは動きません。

## 構築

```rust
use suprnova_payments_stripe::StripeProvider;
use std::sync::Arc;
use suprnova::payments::PaymentProviderRegistry;

// 本番環境：環境変数から読む。
let stripe = StripeProvider::from_env()
    .expect("STRIPE_SECRET_KEY / PUBLISHABLE_KEY / WEBHOOK_SIGNING_SECRET");

// テスト / 明示的な設定：
let stripe = StripeProvider::new(
    "sk_test_...",
    "pk_test_...",
    "whsec_...",
);

PaymentProviderRegistry::bind("stripe", Arc::new(stripe));
```

`StripeProvider` は `Clone` です（安価です - 背後の `stripe::Client` は `Arc` に支えられています）。これは、次の値を保持します：

| フィールド | ソース | 用途 |
|---|---|---|
| `secret_key` | `sk_live_…` / `sk_test_…` | すべてのAPI呼び出しでのHTTP `Authorization: Bearer …` |
| `publishable_key` | `pk_live_…` / `pk_test_…` | `SessionPayload::StripeElements` の内部に表面化し、フロントエンドが別の設定ルックアップなしでStripe.jsをマウントできるようにする |
| `webhook_signing_secret` | `whsec_…` | `Stripe-Signature` ヘッダーのHMAC-SHA256検証 |
| `managed_payments` | `STRIPE_MANAGED_PAYMENTS`（`true`/`1`）または `.with_managed_payments(bool)` | ホスト型の一度限りのセッション作成時に `managed_payments[enabled]=true` を送る（[Managed Payments](#stripe-managed-payments-オプトインのmerchant-of-record)を参照） |

`from_env()` は `Result<Self, String>` を返します - エラーメッセージは、欠けている必須の変数の名前を示します（`STRIPE_MANAGED_PAYMENTS` はオプションです。存在しない場合はオフを意味します）。起動時にパニックする経路はありません。

## チェックアウトセッション

`Checkout::start_session` は、リクエストからStripeのサーフェスを選びます：

| リクエストの形 | Stripeオブジェクト | `SessionPayload` バリアント |
|---|---|---|
| `OneOff` + 空でない `price_refs` | ホスト型Checkoutセッション、`mode=payment` | `StripeCheckoutRedirect { url, provider_session_id: "cs_…" }` |
| `OneOff` + 空の `price_refs` + `amount_hint` | PaymentIntent | `StripeElements { client_secret, publishable_key, provider_session_id: "pi_…" }` |
| `Subscription` + `price_refs` | ホスト型Checkoutセッション、`mode=subscription` | `StripeCheckoutRedirect` |

ホスト型の一度限りの経路は、`allow_promotion_codes=true`（カスタマーはStripeのページ上でプロモーションコードを入力できます - 下記の `Promotions` トレイトと組み合わせてください）と、プロバイダーがそのように設定されている場合はManaged Paymentsフラグを送ります。あなたの `success_return_url` に、Stripeの `{CHECKOUT_SESSION_ID}` テンプレートリテラルを入れてください - リダイレクト時に、Stripeが本物の `cs_…` idに置き換え、あなたの復帰ページはそれを `session_status` に渡します。

`Checkout::session_status` は `GET /v1/checkout/sessions/{id}` を、ニュートラルな `CheckoutSessionState` にマッピングします：

| Stripeの `status` / `payment_status` | `CheckoutSessionState` |
|---|---|
| `open` | `Open` |
| `expired` | `Expired` |
| `complete` + `paid` または `no_payment_required` | `Complete { paid: true, payment_ref, amount_total }` |
| `complete` + `unpaid`（確定処理が遅延） | `Complete { paid: false, … }` |

`payment_ref` は、セッションのPaymentIntent id（`pi_…`）を運ぶため、復帰ページと突き合わせ処理は、そのセッションを `Payment` の操作と `payments_transactions` ミラーに突き合わせられます。`amount_total` は、プロバイダー側の割引とManaged Paymentsの税がすでに織り込まれた、確定済みの合計です。

## プロモーションコード

`StripeProvider` は、オプションの `Promotions` トレイトを実装します（`provider.as_promotions()` は `Some` を返します）。`create_promotion_code` は `POST /v1/promotion_codes` にマッピングされます：これは、事前に作成されたクーポン（`coupon_ref`）から、1人のカスタマー（`customer_ref`）に制限されたコードを、オプションの有効期限と引き換え上限を伴って発行します。制限は、引き換え時にStripeによって強制されます - カスタマーAのために発行されたコードは、カスタマーBがそれを入力すると拒否され、期限切れのコードは拒否され、`max_redemptions: Some(1)` はそのコードを単回限りにします。キャンペーンのパターンについては、[支払い](payments.md)の `Promotions` の節を参照してください。

## PaymentIntentのライフサイクル

Stripeは、単一の支払い試行を**PaymentIntent**として表現します。このintentは複数のステータスを経て移動します。Suprnovaの `Payment` トレイトが、その遷移を駆動します。すべての `StripeProvider` の `Payment` メソッドは、1つの `/v1/payment_intents/...` エンドポイントにマッピングされます：

| `Payment` メソッド | Stripeエンドポイント | 何をするか |
|---|---|---|
| `charge` | `POST /v1/payment_intents` | 保存済みの支払い方法に対して、1回の呼び出しで作成 + 確認する。`capture_method: "manual"` であるため、intentは `succeeded` では**なく** `requires_capture` へ移る。 |
| `capture` | `POST /v1/payment_intents/{id}/capture` | 以前に承認されたintentを確定する。ステータスは `requires_capture` → `succeeded`。 |
| `refund` | `POST /v1/refunds` | 確定済みのintentを、全額または部分的に取り消す。 |
| `void` | `POST /v1/payment_intents/{id}/cancel` | 確定前に承認を取り消す。ステータスは `requires_capture` → `canceled`。 |
| `status` | `GET /v1/payment_intents/{id}` | 現在のステータスを取得する（`PaymentStatus` を返す）。 |

### 先に承認し、後で確定する

`StripeProvider::charge` は、資金を即座に確定**しません**。これは `capture_method=manual` + `confirm=true` を送り、カードを承認して資金を確保し、それから明示的な `capture` 呼び出しを待ちます。これが、標準的な2ステップのフローです：

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
    idempotency_key: Some("order-12345".into()),  // 下記の「べき等性」を参照
    metadata: None,
}).await?;

match result {
    ChargeResult::Completed { provider_transaction_id, status, .. }
        if status == PaymentStatus::Pending => {
        // 承認済み - 注文が発送されるときに確定する。
        let settled = payment.capture(&provider_transaction_id).await?;
        assert!(matches!(
            settled,
            ChargeResult::Completed { status: PaymentStatus::Succeeded, .. }
        ));
    }
    ChargeResult::RequiresClientAction { client_secret, .. } => {
        // 3DSのステップアップが必要 - 下記の「3DSとSCA」を参照。
    }
    other => panic!("unexpected charge result: {other:?}"),
}
```

**即時**の確定が欲しい場合 - よくあるEコマースのワンショット - は、代わりに `SessionMode::OneOff` を伴う `Checkout::start_session` を使ってください。その経路は、`automatic_payment_methods` を有効にしたPaymentIntentを作成し、クライアントシークレットをフロントエンドに渡すため、カスタマーのブラウザがその場でintentを確認します。`Payment::charge` は、あなたがすでにカスタマーの保存済みの支払い方法を持っていて、明示的な承認してから確定するという制御を望むサーバー主導のフロー（マーケットプレイス、遅延履行のSaaS、分割発送のコマースで典型的です）のためのものです。

### ステータスのマッピング

Stripeのステータスは、Suprnovaの `PaymentStatus` 列挙型に折り込まれます：

| `PaymentIntentStatus` | `PaymentStatus` |
|---|---|
| `Succeeded` | `Succeeded` |
| `Processing` | `Pending` |
| `RequiresCapture` | `Pending`（承認済み、確定待ち） |
| `RequiresAction` | `Pending`（`charge` から `RequiresClientAction` として返される） |
| `RequiresConfirmation` | `Pending` |
| `RequiresPaymentMethod` | `Pending` |
| `Canceled` | `Canceled` |
| _新しいStripeのステータス（列挙型は `#[non_exhaustive]`）_ | `Failed` |

`non_exhaustive` のフォールバックは意図的なものです。Stripeは時折、状態を追加します（例：新しい支払い方法のタイプを導入するとき）。それらを `Failed` として表面化させるのが、保守的なデフォルトです - あなたのアプリは、アダプターをアップグレードするまで、その支払いを未確定のまま扱います。

### 3DSとSCA

欧州の強力な顧客認証（Strong Customer Authentication）、インドのRBIの規則、そしていくつかの他の規制当局は、カード保有者が、別のブラウザコンテキストでその支払いを認証することを要求します。Stripeは、これを `next_action` ブロックを伴う `requires_action` として表面化させます。

`StripeProvider::charge` は、これを2つの `ChargeResult` バリアントのどちらかに翻訳します：

```rust
ChargeResult::RequiresClientAction {
    provider_transaction_id,   // pi_xxx - これを保持しておく
    action_kind: "stripe_3ds", // Stripe固有のタグ
    client_secret,             // Stripe.jsへ渡す
    publishable_key,           // Stripe.jsへ渡す
}
```

intentの `next_action` がリダイレクトURLを含んでいるとき（一部の認証フローは、その場のモーダルではなくURLリダイレクトです）、結果は次のように書き換えられます：

```rust
ChargeResult::RedirectRequired {
    provider_transaction_id,
    url,                       // ブラウザをここへリダイレクトする
    return_to: None,
}
```

あなたのコントローラーは、`RequiresClientAction` のペイロードをInertiaページに渡します。フロントエンドは `stripe.confirmCardPayment(client_secret, ...)` を呼び出し、カスタマーが3DSを完了させます。確認が成功すると、Stripeは `payment_intent.succeeded` を発火させ、webhookルートがミラー行を書き込みます。Svelte / React / Vueのスニペットについては、[支払い - フロントエンド 統合](payments-frontend.md)を参照してください。

### 取り消しと返金

`void` は確定**前**に承認を取り消します。`refund` は確定済みの支払いを取り消します。確定済みのintentに対して `void` を呼ぶと失敗します - Stripeは `"already succeeded"` または `"You cannot cancel"` を含むメッセージで拒否し、アダプターはそれを `PaymentError::Validation` として表面化させるため、あなたのハンドラは、回復可能なユーザーエラー（代わりに `refund` を使う）と、本物のプロバイダー障害とを区別できます。それ以外の失敗はすべて `PaymentError::Provider` です。

```rust
let voided = payment.void("pi_3PNzj...").await;
match voided {
    Ok(()) => { /* 承認が取り消された */ }
    Err(suprnova::payments::PaymentError::Validation(msg)) => {
        // すでに確定済み - 代わりにrefundを呼ぶ。
        let refund = payment.refund(RefundRequest {
            provider_transaction_id: "pi_3PNzj...".into(),
            amount: None,           // 全額返金
            reason: Some("requested_by_customer".into()),
            idempotency_key: None,  // refund()はこれを転送しない - 「べき等性」を参照
        }).await?;
    }
    Err(e) => return Err(e.into()),
}
```

## カスタマー

`StripeProvider` は、`/v1/customers` に対して `CustomerStore` を実装します。アダプターは、返された `Customer` をニュートラルな `CustomerRef` にマッピングし、emailとあなたのアプリケーションの `user_id` を保持します：

```rust
use suprnova::payments::CreateCustomerRequest;

let customer = provider.create_customer(CreateCustomerRequest {
    user_id: "user-42".into(),       // あなたのアプリのユーザーid
    email: "alice@example.com".into(),
    name: Some("Alice Example".into()),
    metadata: None,
}).await?;

// customer.provider_customer_id == "cus_NffrFeUfNV2Hib"
// これを、あなたのUser行と並べて永続化しておくと、以降の
// 請求、サブスクリプション、webhookが正しく解決される。
```

`update_customer`、`get_customer`、そして `delete_customer` は、それぞれ `POST /v1/customers/{id}`、`GET /v1/customers/{id}`、`DELETE /v1/customers/{id}` を叩きます。Stripeの削除は `DeletedCustomer` エンベロープを返しますが、アダプターはこれを捨てます - その呼び出しの成功 / 失敗だけが伝播します。

## サブスクリプション

`StripeProvider::subscribe` は、カスタマーの参照、`items[]` の配列、そしてオプションの `trial_period_days` を伴って `/v1/subscriptions` にPOSTします：

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

### 期間の境界

Stripeは、`current_period_start` / `current_period_end` のタイムスタンプを、APIバージョン `2023-08-16` で、親のSubscriptionから各 `SubscriptionItem` へ移しました。複数明細のサブスクリプションは、理論上は明細ごとに異なる期間を持てますが、実際には、単一のサブスクリプション上のすべての明細が、親の請求サイクルを共有します。アダプターは、返される `SubscriptionResult` の中で、**最初の明細**の期間を親の期間として採用します。本当に明細ごとの期間が必要な場合は、`sub.items[n]` からそれらを読んでください - スナップショット上に保持されています。

### 期間の終わりにキャンセルする、対、即時にキャンセルする

```rust
// ソフトキャンセル - current_period_endまでアクセスを保持する：
let sub = provider.cancel("sub_1234", /* at_period_end */ true).await?;
// sub.cancel_at_period_end == true
// sub.status == Active

// 即時キャンセル - Stripeの DELETE /v1/subscriptions/{id}：
let sub = provider.cancel("sub_1234", /* at_period_end */ false).await?;
// sub.status == Canceled
```

この2つの経路は、異なるStripeエンドポイントを叩きます。ソフトキャンセルは、`cancel_at_period_end=true` を伴う `POST /v1/subscriptions/{id}` です - サブスクリプションは請求期間の終わりまでアクティブなままで、その後Stripeがそれを確定させます。即時キャンセルは、`prorate=false` と `invoice_now=false` を伴う `DELETE /v1/subscriptions/{id}` です。

### `update()` は意図的に制限されている

`UpdateSubscriptionRequest` は、アダプターが作用する2つのフィールドを持ちます：`cancel_at_period_end` と `new_price_refs` です。前者はサポートされています。後者は `PaymentError::NotSupported` を返します：

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

これは、`NotSupported` が、後回しではなく誠実な答えである数少ない場所の1つです。Stripeの価格セットの置き換えには、サブスクリプションの明細項目を削除して再作成する必要があります - その形はプロバイダーごとに異なり（プロレーション、請求サイクルの起点、トライアルの保持挙動）、これを単一のニュートラルなAPIに畳み込むことは、助けになる以上に多くを隠してしまいます。推奨される経路は、既存のサブスクリプションをキャンセルし、必要であれば自分自身のプロレーションのポリシーを適用しながら、新しい価格セットで再度 `subscribe` することです。

## Webhook

Stripeは、次の形式でHMAC-SHA256署名されたwebhookを送ります：

```
Stripe-Signature: t=1717000000,v1=5257a869e7ecebeda32affa62cdca3fa51cad7e77a0e56ff536d0ce8e108d8bd
```

`StripeProvider::verify` は、そのヘッダーをパースし、webhook署名シークレットを使って `"{timestamp}.{raw_body}"` に対するHMAC-SHA256を再計算し、ヘッダー内のすべての `v1=` の値に対して**定数時間**の比較を行います。署名シークレットのローテーション中は、複数の `v1=` の値が存在します - Stripeは一定期間、古いシークレットと新しいシークレットを重ねるため、フラグデーの切り替えなしに再署名してデプロイできます。

```
Stripe-Signature: t=1717000000,v1=<old_sig>,v1=<new_sig>
```

アダプターは、**いずれかの** `v1=` の値が一致すればリクエストを受け入れます。`t=` が欠けているヘッダーや、`v1=` の値が1つもないヘッダーは、`PaymentError::WebhookSignature` として拒否されます。ヘッダー内のどこかにある非ASCIIバイトも拒否されます - Stripeはそれらを決して送らないため、置換文字に差し替えるより、無効として扱う方が安全です。

`verify` を直接呼ぶことは決してありません。フレームワークの `webhook_routes(db.clone())` が `POST /webhooks/payments/{provider}` を登録し、そこに届くすべてのリクエストに対して、アダプターの `verify` + `parse_event` + ペイロード抽出器を呼び出します。リトライを意識した監査の振る舞いについては、[べき等性](idempotency.md)を参照してください - プロバイダーがリトライしたときに、以前に失敗したイベントが更新処理を再試行するというルールも含まれます。

### イベント → ニュートラルのマッピング

Stripeのイベントタイプは、`stripe_event_to_neutral` 関数を介して、Suprnovaの `NeutralEventKind` にマッピングされます。マッピング表：

| Stripeのイベントタイプ | `NeutralEventKind` |
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
| _それ以外すべて_ | `None` |

`None` にマッピングされるイベント（Radarの不正シグナル、payout、balance transfer、`created` より後のチャージバックのライフサイクルイベント）は、それでも `payments_webhook_events` の監査テーブルに永続化されます - それらはミラーテーブルを駆動しないだけです。それらが必要な場合は、カスタムハンドラの中で `event.raw_payload` を直接読んでください。

このマッピングは、webhookルートの外でも使えるように、クレートのルートでも再エクスポートされています：

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

### ペイロードの抽出

`verify` と `parse_event` が成功した後、フレームワークは `extract_payload_ids`、`extract_payment_snapshot`、そして `extract_customer_snapshot` を呼び出して、ミラーテーブルを駆動するフィールドを取り出します（背後にある、自分自身のDBから読むというパターンについては、[Eloquent API](eloquent.md)を参照してください）。Stripeは構造的に一貫しています：すべてのwebhookは、関連するエンティティを `data.object` に置き、`id` をその主キーとします。

抽出器は、4つのイベントの系列を扱います：

- **サブスクリプションイベント** - `data.object.id`（サブスクリプションid）と `data.object.customer` を取り出す。
- **カスタマーイベント** - `data.object.id`（カスタマーid）を取り出す。
- **PaymentIntent / Chargeイベント** - `data.object.id`、`data.object.amount`、`data.object.currency`、`data.object.customer`、そして（`payment_intent.succeeded` の場合のみ）`data.object.created` を `paid_at` として取り出す。
- **Invoiceイベント** - `data.object.id`、カスタマーへのポインタ、`data.object.subscription`（継続的な請求のみ）、`amount_paid`（`amount_due` にフォールバック）、`tax`、`currency`、そして `data.object.status_transitions.paid_at` を取り出す。

それ以外は、スナップショット抽出器から `None` が返ります。それでも監査行は書き込まれます。

## ミラーテーブル

6つのテーブルが、あなたのアプリケーションのデータベースの中で、支払いサーフェスを支えます。フレームワークのマイグレーションを、あなた自身のマイグレーションと一緒に適用してください：

```rust
use sea_orm_migration::{MigrationTrait, MigratorTrait};
use suprnova::payments::migrations::CreatePaymentsTables;

pub struct Migrator;

impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            // ... あなたのマイグレーション ...
            Box::new(CreatePaymentsTables),
        ]
    }
}
```

作成されるテーブルは、`payments_customers`、`payments_payment_methods`、`payments_subscriptions`、`payments_subscription_items`、`payments_transactions`、そして `payments_webhook_events` です。webhookルートは、イベントごとに単一のDBトランザクションの中でそれらを更新します - 部分的な状態が観測されることは決してなく、監査行は、リトライを通じて `process_error` を運ぶため、失敗は運用者に見え続けます。

## べき等性

Stripe API呼び出しに対する送信側のべき等性と、webhook配信に対する受信側のべき等性は、2つの別々の物語です。そのように読んでください。

### 送信側：メソッドごとの対応

Stripeは、`Idempotency-Key` HTTPリクエストヘッダーを介して、リクエストのべき等性をサポートします - 同じ本文を伴う同じキーは、24時間のリプレイウィンドウの間、同じレスポンスオブジェクトを返します。本文が一致しない場合はエラーが返ります。SuprnovaのStripeアダプターは、今日のところ、DTOの `idempotency_key` フィールドをそのヘッダーへ一様に通していません。この文章を書いている時点での実際の振る舞いは：

| メソッド | DTOのフィールド | アダプターが行うこと |
|---|---|---|
| `Payment::charge` | `ChargeRequest::idempotency_key` | （HTTPヘッダーではなく）POST本文に `idempotency_key=...` として転送される。StripeのAPIは本文形式のべき等キーを読ま**ない**ため、これは、アダプターがリクエストヘッダーの経路に移行するまでは、効果がないものとして扱うのが最善である。 |
| `Payment::refund` | `RefundRequest::idempotency_key` | サイレントに捨てられる - このフィールドは転送されない。 |
| `Checkout::start_session` | `StartSessionRequest::idempotency_key` | サイレントに捨てられる。 |
| `Subscription::subscribe` / `update` | `*Request::idempotency_key` | サイレントに捨てられる。 |

今日、Stripeに対する請求 / 返金のリトライについて、最大1回の実行という保証に依存する場合は、アダプターがそのヘッダーを配線するまで、あなた自身の呼び出し箇所でそのリトライをゲートしてください（あなたのDBに永続化された決定的なドメインキーと、2度目の挿入を防ぐユニークインデックス）。DTOのフィールドはAPI上では受け付けられますが、現在のところワイヤまで完全には尊重されていません - そのギャップを明示するために、テストと本番のコードではそれらを `None` に設定し、Stripeがあなたのリトライを重複排除してくれると仮定しないでください。

これはv1のアダプターにおける既知のギャップであり、次のリリースの修正候補です。配線が到着しても、サーフェスの形は同じままです。

### 受信側：webhookの重複排除

webhookのべき等性は、受信側でフレームワークによって処理されており、完全に配線されています。すべてのイベントは、`(provider, provider_event_id)` に対するUNIQUEインデックスを伴って `payments_webhook_events` に入ります。すでに処理されたイベントの重複配信は、更新処理を再実行せずに、即座にStripeへ200を返します。以前に**失敗した**イベントの重複は、更新処理を再試行するため、プロバイダーのリトライがあなたの復旧の仕組みになります。完全な監査 + リトライの契約については、[べき等性](idempotency.md)を参照してください。

## テスト

このアダプターは、hyperに支えられ、rustlsで前面を固められています。`StripeProvider` を構築するテストは、登録済みの暗号プロバイダーを必要とします。私たちは `#[cfg(test)]` の中で、`ring` をちょうど一度だけインストールします：

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
        let event = /* raw_payloadを伴うWebhookEventを構築する */;
        let ids = p.extract_payload_ids(&event);
        assert_eq!(ids.subscription_id.as_deref(), Some("sub_abc"));
    }
}
```

本物のStripeサンドボックスを叩く統合テストには、あなたのテスト用の環境に `STRIPE_SECRET_KEY` などを設定してください。あなた自身のコントローラーの単体テストには、フレームワークの `MockPaymentProvider` を使うことを好んでください - これは、予測可能な戻り値とゼロのネットワークで、5つのトレイトすべてを実装します。

## 次のステップ

- [支払い](payments.md) - トレイトのサーフェス、レジストリ、ブートストラップのパターン、そしてflowタグ付きの `SessionPayload`。
- [支払い - Paddle アダプター](payments-paddle.md) - Merchant-of-Recordの対応物。同じ5つのトレイト、異なる責任分担。
- [支払い プロバイダー アダプター の作成](payments-provider-guide.md) - Suprnovaが出荷していないゲートウェイのためのアダプターを書く方法。
- [支払い - フロントエンド 統合](payments-frontend.md) - `SessionPayload.flow` に対するSvelte / React / Vueのディスパッチ、Stripe.jsのconfirm-card-paymentループを含む。
- [べき等性](idempotency.md) - 少なくとも1回の配信のもとでwebhookの処理を安全にする、監査 + リトライの契約。
- [Eloquent API](eloquent.md) - あなた自身のモデルと並べてミラーテーブルにクエリする。すべてはただのSeaORMエンティティである。
