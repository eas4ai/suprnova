# 支払い - Paddle アダプター

Paddleアダプター（`suprnova-payments-paddle`）は、PaddleをSuprnovaの汎用的な支払いサーフェスに組み込みます。売上税、VAT、GST、支払いの督促、請求書発行、そして返金まで、あなたに代わって処理してくれる支払いプロバイダーが欲しいときは、これに手を伸ばしてください - Paddleは Merchant of Record（MoR）であり、これは、あなたの顧客に対する正式な販売者であること、そしてStripeのような直接確定型のゲートウェイがあなたに残すコンプライアンスの負担を、Paddleが吸収することを意味します。

その選択は、メンタルモデルを変えます。あなたのドメインコードは、サブスクリプションを*所有*しません - Paddleが所有します。あなたはチェックアウトを開き、顧客がそれを完了させ、`SubscriptionCreated` webhookが、サブスクリプションが今存在することを伝えてきます。APIを介してサブスクリプションを作成することはできず、後からその価格セットを入れ替えることもできません。キャンセルはできます、状態の読み取りもできます、請求メタデータの更新もできます。それ以外はPaddleのものです。

この章は、汎用的な5トレイトサーフェスについて[支払い](payments.md)を読んでいることを前提としています。ここでは、Paddleに*限って*成り立つことを扱います。

## Paddleを選ぶべきとき

次のうち1つ以上が当てはまるなら、Paddleを選んでください：

- あなたはデジタル商品をグローバルに販売しており、税務コンプライアンス（VAT、GST、米国の売上税）が、ロードマップ上の実質的なコストになっている。
- 失敗した支払いのリトライ、支払いの督促メール、あるいは受領書の発行を、自分で管理したくない。
- 会計のために、単一の正式な販売者からの単一のインボイスが欲しい。
- あなたのビジネスモデルはサブスクリプション優先であり、プロバイダーがサブスクリプションのライフサイクルを駆動することを受け入れられる。

チャージの確定を直接コントロールしたい、自分で税務を処理する、あるいは自分自身のコードパスからサーバーサイドの `charge`/`capture`/`refund` 呼び出しが必要な場合は、代わりに[Stripe](payments.md#stripe)を選んでください。

## 設定

クレートを追加してください：

```bash
cargo add suprnova-payments-paddle
```

4つの環境変数を設定してください：

```env
PADDLE_API_KEY=pdl_sdbx_apikey_...
PADDLE_WEBHOOK_KEY=pdl_ntfset_...
PADDLE_CLIENT_TOKEN=test_...
PADDLE_ENVIRONMENT=sandbox
```

| 変数 | それが何であるか | どこから来るか |
|---|---|---|
| `PADDLE_API_KEY` | サーバーサイドのAPIキー（`pdl_live_apikey_…` / `pdl_sdbx_apikey_…`） | Paddleダッシュボード → Developer Tools → Authentication |
| `PADDLE_WEBHOOK_KEY` | 通知先のシークレット（`pdl_ntfset_…`） | Paddleダッシュボード → Developer Tools → Notifications → あなたのエンドポイント |
| `PADDLE_CLIENT_TOKEN` | ブラウザで安全に使えるクライアントトークン（`live_…` / `test_…`） | Paddleダッシュボード → Developer Tools → Authentication → Client-side tokens |
| `PADDLE_ENVIRONMENT` | `sandbox`（デフォルト）または `production` | あなたの判断次第 |

起動時にプロバイダーを登録してください。どちらの形式も有効です：

```rust
use std::sync::Arc;
use suprnova::payments::PaymentProviderRegistry;
use suprnova_payments_paddle::{PaddleEnvironment, PaddleProvider};

pub async fn bootstrap() {
    // 環境変数から（推奨）：
    let paddle = PaddleProvider::from_env()
        .expect("Paddle env vars not set");

    // または直接構築する：
    let paddle = PaddleProvider::new(
        "pdl_sdbx_apikey_...",
        "pdl_ntfset_...",
        "test_...",
        PaddleEnvironment::Sandbox,
    ).expect("Paddle client init failed");

    PaymentProviderRegistry::bind("paddle", Arc::new(paddle));
}
```

webhookの受信ルートは、フレームワークの `webhook_routes(db.clone())` ヘルパーによって登録されます - [支払い](payments.md#webhook-handling)を参照してください。`from_env()` と `new()` はどちらも `Result` を返します。なぜなら、内部で使われている `paddle_rust_sdk::Paddle::new` が、構築時にAPIキーの形とエンドポイントURLを検証するからです。

## MoRのメンタルモデル

Stripeのユーザーを驚かせる形はこうです：

```
Stripe（ゲートウェイ）：
    あなたのアプリ  ─────────►  Stripe  ──►  カードネットワーク
       │                            ▲
       └──────── webhook ───────────┘
    あなたはDB内でサブスクリプションの状態を所有し、Stripeは実行者です

Paddle（Merchant of Record）：
    あなたのアプリ  ─►  チェックアウトリンク  ─►  顧客  ──►  Paddle  ──►  カードネットワーク
                                                                │
       ◄──────────────────────  webhook  ──────────────────────┘
    Paddleがサブスクリプションの状態を所有し、あなたのDBはミラーです
```

コードの中では、この違いは3つの点に現れます：

1. **APIを介してサブスクリプションを作成することはできません。** 定期価格を指定して `Checkout::start_session` を呼び出し、顧客がPaddleのウィジェットを完了させると、`SubscriptionCreated` webhookがあなたのミラーを更新します。
2. **APIを介してサブスクリプションの価格セットを入れ替えることはできません。** Paddleは、プラン変更を自身のダッシュボードか、自身が所有する移行フローのために予約しています。
3. **顧客を削除することはできません。** updateを介したアーカイブ化が、サポートされている回避策です。

Suprnovaは、これらの制約を取り繕うのではなく、`PaymentError::NotSupported` として表面化させます - 下の[能力マトリクス](#能力マトリクス)を参照してください。

## チェックアウトフロー

`Checkout::start_session` が、Paddleで支払いを開始する唯一の方法です。フロントエンドは、起動時に設定した `client_token` を使って、結果として得られる `transaction_id` をpaddle.jsで開きます：

```rust
use std::sync::Arc;
use suprnova::payments::*;

pub async fn start_checkout(
    user_id: String,
    email: String,
) -> PaymentResult<SessionPayload> {
    let provider = PaymentProviderRegistry::get("paddle")
        .expect("paddle provider not registered");

    // 1. Paddleに顧客を作成する（あるいは既存のものを再利用する）。
    let cus = provider.create_customer(CreateCustomerRequest {
        user_id: user_id.clone(),
        email,
        name: None,
        metadata: None,
    }).await?;

    // 2. チェックアウトセッションを開く。Paddleは、以下のSessionModeフィールドではなく、
    //    *価格の種類*に基づいて、一度限りかサブスクリプションかを振り分ける。
    let session = provider.start_session(StartSessionRequest {
        mode: SessionMode::Subscription,           // Paddleでは無視される（下の注記を参照）
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

返ってくる `SessionPayload::PaddleInline` は、フロントエンドが必要とするすべてを運びます：

```json
{
  "flow": "paddle_inline",
  "transaction_id": "txn_01h...",
  "customer_token": "ctm_01h...",
  "client_token": "test_..."
}
```

Svelte / React / Vueでのpaddle.jsマウントコードについては、[支払い - フロントエンド 統合](payments-frontend.md)を参照してください。

### Paddleは `SessionMode` ではなく価格の種類で振り分ける

本物のPaddle固有の落とし穴です：`StartSessionRequest` の `SessionMode::OneOff` / `SessionMode::Subscription` フィールドは、**Paddleアダプターによって無視されます**。PaddleのAPIには単一の `transaction_create` エンドポイントしかなく、プロバイダーは、フローを推測するために渡された価格IDを調べます - 定期価格はサブスクリプションを開始し、一度限りの価格は単発のチャージを開始します。Stripeでは、そのフィールドがフローを駆動しますが、Paddleでは*価格*がそれを行います。アダプターをそれらに向ける前に、正しい価格の種類でPaddleのカタログをセットアップしておいてください。

## サブスクリプションはwebhook経由で届く

Paddleがサブスクリプションのライフサイクルを所有しているため、あなたのドメインコードは、Paddleが伝えてきたときにだけ、サブスクリプションについて*知る*ことになります。そのフロー：

```
あなたのアプリ                       Paddle                     顧客
   │                              │                          │
   │  start_session(price=pri_…)  │                          │
   ├─────────────────────────────►│                          │
   │  PaddleInline { txn_id, … }  │                          │
   │◄─────────────────────────────┤                          │
   │                              │       paddle.js          │
   │                              │◄─────────────────────────┤
   │                              │   チェックアウトを完了する  │
   │                              ├─────────────────────────►│
   │                              │                          │
   │   subscription.created webhook                          │
   │◄─────────────────────────────┤                          │
   │                              │                          │
   ▼                              │                          │
 ミラーテーブルが更新される、        │                          │
 payments_subscriptions 行に      │                          │
 provider_subscription_id が設定される │                     │
```

フレームワークの `webhook_routes(db)` ハンドラが、あなたに代わって更新処理を行います：`WebhookHandler::extract_payload_ids` を呼んで `subscription_id` を見つけ、`Subscription::get(id)` を呼んで正式な状態を読み取り、単一のトランザクションの中で `payments_subscriptions` + `payments_subscription_items` をupsertします。webhookが200を返す時点で、あなたのミラーはPaddleと整合しています。

顧客がウィジェットを完了させてからwebhookが到着するまでの間に短いウィンドウがあり、その間、`payments_subscriptions` には新しいサブスクリプションの行がありません。2つのパターンがこれをカバーします：

- **即時のUXにはリダイレクトURLを使う。** `success_return_url` は、Paddleがトランザクションを確認した瞬間にクライアントサイドで発火するため、サーバーサイドのwebhookを待たずに「サブスクリプションが有効」と表示できます。
- **ポーリングしてレンダリングする。** リダイレクトの後、短い遅延を置いてページを再読み込みし、Inertiaコントローラーが、更新済みになったミラーを読み取れるようにします。

## 能力マトリクス

すべてのトレイトのすべてのメソッドが、Stripe版と同じことをするわけではありません。下の表が真実です。`subscribe()` と、`new_price_refs.is_some()` を伴う `update()` だけが、*常に*失敗するメソッドです。残りは、注記された注意点はありますが動作します。

| トレイトメソッド | 振る舞い |
|---|---|
| `Checkout::start_session` | 動作する。`SessionMode` ではなく価格の種類で、一度限りかサブスクリプションかを振り分ける。 |
| `Subscription::subscribe` | 常に `NotSupported`。サブスクリプションは、チェックアウトの完了 + webhookから生まれる。 |
| `Subscription::update(cancel_at_period_end: Some(true), new_price_refs: None)` | 動作する。デフォルトの `EffectiveFrom::NextBillingPeriod` を伴う `subscription_cancel` に配線される。 |
| `Subscription::update(new_price_refs: Some(...))` | v1では `NotSupported`。Paddleは、価格セットの置き換えを自身の移行フローのために予約している。 |
| `Subscription::update`（no-op） | 動作する。`subscription_get` を介して現在の状態を再取得する。 |
| `Subscription::cancel` | 動作するが、`at_period_end` は**無視される** - 常に次の請求サイクルへスケジュールされる。[下記](#キャンセルは常にスケジュールされる)を参照。 |
| `Subscription::get` | 動作する。 |
| `CustomerStore::create_customer` | 動作する。 |
| `CustomerStore::update_customer` | 動作する。 |
| `CustomerStore::get_customer` | 動作する。 |
| `CustomerStore::delete_customer` | `NotSupported`。必要なら `archived` ステータスで `update_customer` を使う。 |
| `Payment::*` | トレイトは実装されていない。`provider.as_payment()` は `None` を返す。 |
| `WebhookHandler::*` | 動作する。 |

`Payment` が実装されていないこと、`subscribe`/`delete_customer` が `NotSupported` を返すこと、そしてwebhookの署名拒否という不変条件は、`crates/suprnova-payments-paddle/tests/integration.rs` の中の常時実行されるテストによって固定されているため、上のマトリクスがサイレントにドリフトすることはありません。

### キャンセルは常にスケジュールされる

`Subscription::cancel(id, at_period_end)` は、トレイトの互換性のためにboolを受け取りますが、**常にスケジュールされたキャンセルとして振る舞います** - Paddleの `EffectiveFrom` 列挙型は `paddle_rust_sdk` 0.18の中でprivateであるため、v1では即時キャンセルは実現できません。ユーザーは、現在の請求サイクルが終わるまでアクセスを保持し、その時点でPaddleが `subscription.canceled` を発火させ、ミラーは `status` を `Canceled` に切り替えます。

アプリへのアクセスを即座に取り消しつつ、Paddleにバックグラウンドで請求を巻き戻させる、UXレベルの「今すぐキャンセル」が欲しい場合は、あなた自身の `subscription.status != Canceled && subscription.cancel_at_period_end == false` フラグでアクセスをゲートし、`cancel()` が返った直後にUIを更新してください - 次のwebhookがそれを確認します。

### 顧客の削除は「updateを介したアーカイブ化」である

`delete_customer` が `PaymentError::NotSupported` を返すのは、Paddleの公開APIが削除エンドポイントを一切公開していないからです。Paddleの中で顧客レコードを抑制する必要がある場合は、`archived` ステータスで `update_customer` を呼んでください。フレームワークのアダプターはこれを直接ラップしていません - metadataフィールドがエスケープハッチです：

```rust
provider.update_customer(UpdateCustomerRequest {
    provider_customer_id: customer_id,
    email: None,
    name: None,
    metadata: Some(serde_json::json!({ "status": "archived" })),
}).await?;
```

これを出荷するときは、あなたのPaddle APIバージョンに対して正確なフィールドパスを確認してください - SDKは現時点で `status` 列挙型を直接モデル化していません。

## Webhookの署名検証

Paddleは、すべてのwebhookをHMACで署名します。`Paddle-Signature` ヘッダーは `ts=1716000000,h1=abcdef…` のような形をしています。アダプターは、SDKの `Paddle::unmarshal` に検証を委任します。これは：

- ヘッダーをパースする
- あなたの `PADDLE_WEBHOOK_KEY` を使ってHMACを再計算する
- タイムスタンプが `MaximumVariance::default()` の範囲外にある署名を拒否する（これを書いている時点では5秒 - それより古いリプレイは捨てられる）

フレームワークの `webhook_routes` ハンドラは、他の何かをする前に `verify` を呼び出します。失敗すると、ボディの漏洩なしに `401 invalid-signature` を返します。このコードを自分で書くことはありませんが、この検証がHMAC + タイムスタンプの許容範囲であり、静的なシークレットの比較ではないことは知っておく価値があります。

## Webhookペイロードの形

アダプターの `extract_payload_ids`、`extract_payment_snapshot`、`extract_customer_snapshot` メソッドは、Paddleのペイロードの形を知っているため、フレームワークはミラーテーブルを更新できます。簡単なマッピング：

| webhookのevent_type | `NeutralEventKind` | ミラーへの効果 |
|---|---|---|
| `transaction.completed`、`transaction.paid` | `PaymentSucceeded` | `payments_transactions` をupsert |
| `transaction.payment_failed` | `PaymentFailed` | `payments_transactions` をupsert（失敗） |
| `transaction.billed` | `InvoicePaid` | `provider_subscription_id` を紐付けた `payments_transactions` をupsert |
| `adjustment.created`、`adjustment.updated` | `PaymentRefunded` | `payments_transactions` をupsert（返金） |
| `subscription.created` | `SubscriptionCreated` | `Subscription::get` → `payments_subscriptions` + 明細項目をupsert |
| `subscription.updated`、`.activated`、`.paused`、`.resumed`、`.trialing` | `SubscriptionUpdated` | 上と同じ |
| `subscription.canceled` | `SubscriptionCanceled` | 同じ。`canceled_at` を設定し、statusを切り替える |
| `customer.created` | `CustomerCreated` | 更新専用：ミラー行が存在する場合、`email`/`metadata` を再取得する |
| `customer.updated` | `CustomerUpdated` | 同じ |
| その他すべて | `None`（マッピングされない） | 監査行のみ - ミラーへの変更なし |

Paddleは、エンティティオブジェクトを（Stripeのような `data.object` ではなく）`data` の直下に置きます。金額は10進数ではなく、**最小単位の文字列**として届きます（`"1234"` = 主要単位で12.34）- アダプターは、前方互換性のために、文字列と数値の両方の形をパースします。通貨は `currency_code` として小文字で届き、スナップショットはそれを大文字化します。

### 税込みの金額

Paddleは、トランザクションの金額を**税込みで**報告します。フレームワークの `payments_transactions` ミラーは、これを分割します：

- `amount_total_minor` - 顧客が支払った全額（税込み）
- `amount_tax_minor` - 税の構成部分

税抜きの額は `amount_total_minor - amount_tax_minor` です。これは、（`amount_tax_minor = 0` で税抜きを報告する）Stripeとは異なります。両方のプロバイダーをまたいで収益を合計するコードは、税を意識する必要があります：

```rust
let net_revenue_minor = txn.amount_total_minor - txn.amount_tax_minor;
```

## 顧客の作成

`CreateCustomerRequest` は、Paddleの `customer_create` に直接マッピングされます：

```rust
let cus = provider.create_customer(CreateCustomerRequest {
    user_id: "user_42".into(),       // あなたのアプリのユーザーid
    email: "alice@example.com".into(),
    name: Some("Alice".into()),
    metadata: None,                  // v1ではPaddleに転送されない
}).await?;
// cus.provider_customer_id == "ctm_01h..."
```

`cus.provider_customer_id` を、あなたのユーザーレコードと並べて保存してください。以降のすべての呼び出し（チェックアウトの開始、サブスクリプションのルックアップなど）は、アプリのユーザーIDではなく、Paddleのカスタマーidを取ります。ミラーテーブル `payments_customers` は両方のカラムを運ぶため、単一のインデックスルックアップで、どちらの方向にも到達できます。

`update_customer` と `get_customer` は、対応するSDKメソッドへそのまま渡されます。`update_customer` は `email` / `name` の更新を受け取り、更新された `CustomerRef` を返します。`get_customer` は、（ミラーからではなく）Paddleからスナップショットを取得します - Paddleダッシュボードでの範囲外の変更の後に、新鮮な読み取りが必要なときにこれを使ってください。

## 意図的な `NotSupported` の形

コードベースに不慣れな読者は、`subscribe()` と `delete_customer()` における `PaymentError::NotSupported` が、後回しにされたTODOだと思うかもしれません。そうではありません。この制約はPaddleのプロダクトサーフェスの一部であり、Suprnovaは、プロバイダーが決して尊重しないローカルなミューテーションをエミュレートするのではなく、それをそのままコード化します。

それぞれの `NotSupported` エラーメッセージは、サポートされているワークフローを指し示します：

- `subscribe`：「`SessionMode::Subscription` で `Checkout::start_session` を使い、`SubscriptionCreated` webhookを待つ」
- `new_price_refs` を伴う `update`：「既存のサブスクリプションに対するPaddleの価格セット置き換えはv1にない」
- `delete_customer`：「`archived` ステータスで `UpdateCustomer` を使う」

プロバイダーに依存しないドメインコードを書くときは、このエラーで明示的に分岐してください：

```rust
match provider.delete_customer(&cus_id).await {
    Ok(()) => { /* Stripeの経路 */ }
    Err(PaymentError::NotSupported(_)) => {
        // Paddleの経路 - 代わりにupdateを介してアーカイブ化する
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

### Suprnovaが異なる設計を選んだ理由

Laravel CashierはStripe専用であり、サブスクリプションをアプリ所有としてモデル化します：`$user->newSubscription('default', 'pri_pro')->create()` は、あたかもアプリケーションがサブスクリプションを開始しているかのような形をしています。直接確定型のゲートウェイであれば、それは正確です。MoRでは、それは嘘です - 実行者はプロバイダーであり、あなたのアプリではありません。

Suprnovaの支払いサーフェスはプロバイダーニュートラルであるため、どちらの側にも立ちません。トレイトサーフェス（`subscribe`、`update`、`cancel`、`get`）が汎用的な形であり、各アダプターはそのプロバイダーが公開しているものを実装し、プロバイダーのプロダクトモデルが異なる場合は `NotSupported` を返します。Stripeアダプターは `subscribe` を実装します。Paddleアダプターはそうしません、なぜならPaddleがそれを許さないからです。この違いを、偽のローカルな「create」の裏に隠すことは、アダプターにあなたへ嘘をつかせることになります - Suprnovaは、エラー文字列の中に移行メッセージを持つ、型付きの `NotSupported` を好みます。

同じ分岐が `Payment`（サーバーサイド確定）にも当てはまります。Stripeはこれを実装し、Paddleは実装せず、`provider.as_payment()` は `None` を返します。charge/capture/refundを必要とするコードは、盲目的に呼び出すのではなく、`as_payment().is_some()` をチェックしなければなりません - [支払い](payments.md#payment--optional-server-side-capture)を参照してください。

## 統合をテストする

このクレートには、常時実行される不変条件のテスト（ネットワークアクセス不要）に加えて、Paddleのサンドボックスapiに対する、環境変数でゲートされた統合テストが含まれています：

```bash
# 常時実行される不変条件（署名拒否、NotSupportedの形）：
cargo test -p suprnova-payments-paddle

# さらにサンドボックス統合（PADDLE_API_KEYなどが必要）：
PADDLE_API_KEY=pdl_sdbx_apikey_... \
PADDLE_WEBHOOK_KEY=pdl_ntfset_... \
PADDLE_CLIENT_TOKEN=test_... \
PADDLE_ENVIRONMENT=sandbox \
  cargo test -p suprnova-payments-paddle
```

アダプター固有の抽象化を構築するなら、不変条件のテストが、あなた自身のコードで手本にするべきものです。コピーする価値のある3つのテストの形：

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
    let p = /* ...上と同じ... */;
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
    let p = /* ...上と同じ... */;
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

Paddleに一切触れない、ローカルなエンドツーエンドのテストのために、フレームワークは `MockPaymentProvider` を出荷しています。Paddleと同様に、このモックの `as_payment()` は `None` を返すため（サーバーサイド確定なし）、`as_payment().is_some()` で分岐するコードは、モックの下でもPaddleの下と同じ経路をたどります。このモックの `subscribe()` は（Paddleと異なり）`Ok` を返すため、`NotSupported` の分岐をアサートする必要があるテストは、本物の `PaddleProvider` を使うべきです。テストでは、本物のプロバイダーの代わりにモックをバインドしてください：

```rust
use std::sync::Arc;
use suprnova::payments::{MockPaymentProvider, PaymentProviderRegistry};

#[suprnova_test]
async fn checkout_flow() {
    PaymentProviderRegistry::bind("paddle", Arc::new(MockPaymentProvider::new()));
    // ...モックに対してあなたのコントローラーを動かす...
}
```

## 本番デプロイのチェックリスト

`PADDLE_ENVIRONMENT=production` に切り替える前に：

- [ ] 4つの環境変数すべてが、コミットされてはいない本番のシークレットに設定されている
- [ ] webhookエンドポイントのURLが、Paddleダッシュボードの*Notifications*設定に登録されており、そこで生成した宛先シークレットが `PADDLE_WEBHOOK_KEY` と一致している
- [ ] カタログにライブの（サンドボックスではない）価格IDがあり、`price_refs` で参照するIDがライブカタログに存在する
- [ ] `success_return_url` と `cancel_return_url` が、HTTPSのエンドポイントを指している（Paddleは本番環境でHTTPを拒否する）
- [ ] `subscribe()`、`delete_customer()`、あるいは `update(price_refs)` が `NotSupported` を返したときに、あなたのアプリがどう応答するかを決めている - コードで分岐するか、それらのフローがMoR専用であることを文書化するか
- [ ] キャンセルのUXをストレステストしている：キャンセルは常にスケジュールされるため、あなたのUIが示すべきメッセージは「キャンセルしましたが、DATEまでアクセスできます」である
- [ ] サブスクリプション到着のwebhookをストレステストしている：顧客が支払いを終えているのにミラーにまだ行がないウィンドウが存在する
- [ ] 収益を正しく集計している：Paddleの金額は税込み、Stripeの金額は税抜きである

## 次のステップ

- [支払い](payments.md) - 汎用的な5トレイトサーフェスと、webhookハンドラのミラー更新契約
- [支払い - フロントエンド 統合](payments-frontend.md) - Svelte / React / Vueでのpaddle.jsインラインチェックアウト
- [支払い プロバイダー アダプター の作成](payments-provider-guide.md) - あなた自身のアダプタークレートをエンドツーエンドで書く
- [設定](configuration.md) - Paddleの環境変数が差し込まれる、型付き設定の登録
- [アプリケーション ブートストラップ](bootstrap.md) - `PaymentProviderRegistry::bind` が実際にあなたのアプリのどこにあるか
