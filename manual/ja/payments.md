# 支払い

Suprnovaの支払いサーフェスはプロバイダーニュートラルです。アダプタークレート - Stripe、Paddle、あるいは自分で書いたもの - を選び、起動時に登録すれば、あなたのドメインコードは、背後にいるプロバイダーが何であっても、同じ4つのコアトレイト（サーバーサイド確定のためのオプションの5番目付き）を呼び出します。データベース内のミラーテーブルはwebhookによって同期が保たれるため、あなたのドメインコードは、クエリのたびにプロバイダーAPIを叩くのではなく、自分自身のDBから読み取ります。

どの機能も、単一のプロバイダーに限定されていません。Stripeの直接確定モデルとPaddleのMerchant of Recordモデルは、どちらも同じトレイト契約に収まります。異なるサーフェスは `Payment`（サーバーサイド確定）だけであり、これはオプションです - Paddleはこれを必要としないため、Paddleはこれを実装しません。プロバイダーは、`PaymentProvider::as_payment()` をオーバーライドして `Some(&dyn Payment)` を返すことで、自分の能力を宣言します。呼び出し側は実行時に問い合わせます。

## Suprnovaが異なる設計を選んだ理由

Laravelは、コアのドキュメントの中で、Cashierを第一級のStripe統合として出荷しています。それは便利ですが、Stripe専用です - 2番目のプロバイダーを追加するには、Cashierをフォークするか、並行するサーフェスを構築するしかありません。Suprnovaは、支払いプロバイダーを、キャッシュやストレージのドライバーと同じように扱います：1つの汎用的なトレイト集合と、差し替え可能なアダプターです。あなたのドメインコードは、`StripeProvider` や `PaddleProvider` の名前を決して呼びません - レジストリから解決された `Arc<dyn PaymentProvider>` に対して `provider.subscribe(...)` を呼び出すだけであり、その背後にあるプロバイダーは、一度のブートストラップの変更で別のものに切り替わります。

## クイックスタート

アダプタークレートを追加してください。Suprnovaがv0.1リリースを出荷するまでは、フレームワークとそのアダプタークレートは、crates.ioではなくgit経由で取り込まれます：

```toml
# Cargo.toml
[dependencies]
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.2.3" }
suprnova-payments-stripe = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.2.3" }
```

起動時に、プロバイダーとwebhookルーターを登録してください。webhookルーターは、あなたの `routes::register()` に組み込む通常の `Router` です：

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

/// `Application::routes(routes::register)` は、起動時にこれを一度だけ呼び出します。
/// 私たちは支払いwebhookルーターから始め、通常の `.get(...)` / `.post(...)` 呼び出しで、
/// アプリの残りのルートをその上に重ねます。
pub fn register() -> Router {
    let db: Arc<DatabaseConnection> = App::get().expect("db not bound");

    webhook_routes(db)
        .get("/", crate::controllers::home::index)
        .post("/login", crate::controllers::auth::login)
        // ... 残りのルート ...
        .into()
}
```

`webhook_routes(db)` は、`POST /webhooks/payments/{provider}` だけを含む `Router` を返します。`Router::get` と `Router::post` はそれぞれ `.into()` を介して `Router` に戻る `RouteBuilder` を返すため、支払いルーターの上にチェーンすることが、合成する最も直接的な方法です。すでに通常のルートに `routes!{}` マクロを使っているなら、webhookのPOSTを同じブロックに入れてください - `webhook_routes` は、1回の `Router::new().post(...)` 呼び出しをまとめる便利なラッパーです。

あなたのコントローラーの中で、プロバイダーをルックアップし、カスタマーを作成し、チェックアウトセッションを開いてください：

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

その `SessionPayload` は、あなたのInertiaページのpropsに入ります。フロントエンドは `payload.flow` にディスパッチして、正しいウィジェットをレンダリングします - [支払い - フロントエンド 統合](payments-frontend.md)を参照してください。

## アダプターを選ぶ

### Stripe

```toml
# Cargo.toml
suprnova-payments-stripe = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.2.3" }
```

必須の環境変数：

| 変数 | 説明 |
|---|---|
| `STRIPE_SECRET_KEY` | シークレットキー（`sk_live_…` / `sk_test_…`） |
| `STRIPE_PUBLISHABLE_KEY` | 公開可能キー（`pk_live_…` / `pk_test_…`） |
| `STRIPE_WEBHOOK_SIGNING_SECRET` | webhookエンドポイントの署名シークレット（`whsec_…`） |

```rust,ignore
use suprnova_payments_stripe::StripeProvider;
use std::sync::Arc;
use suprnova::payments::PaymentProviderRegistry;

// 環境変数から（本番環境で推奨）：
let stripe = StripeProvider::from_env().expect("Stripe env vars not set");

// または直接構築する：
let stripe = StripeProvider::new("sk_test_...", "pk_test_...", "whsec_...");

PaymentProviderRegistry::bind("stripe", Arc::new(stripe));
```

Stripeは、オプションの `Payment`（PaymentIntents経由のサーバーサイド確定）と `Promotions`（`/v1/promotion_codes` 経由のプロモーションコード発行）を含む、すべてのトレイトを実装します。`provider.as_payment()` と `provider.as_promotions()` はどちらも `Some` を返します。

### Paddle

```toml
# Cargo.toml
suprnova-payments-paddle = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.2.3" }
```

必須の環境変数：

| 変数 | 説明 |
|---|---|
| `PADDLE_API_KEY` | APIキー（`pdl_live_apikey_…` / `pdl_sdbx_apikey_…`） |
| `PADDLE_WEBHOOK_KEY` | 通知先のシークレット（`pdl_ntfset_…`） |
| `PADDLE_CLIENT_TOKEN` | クライアントサイドのトークン（`live_…` / `test_…`） |
| `PADDLE_ENVIRONMENT` | オプション、デフォルトは `"sandbox"` |

```rust,ignore
use suprnova_payments_paddle::{PaddleProvider, PaddleEnvironment};
use std::sync::Arc;
use suprnova::payments::PaymentProviderRegistry;

// 環境変数から：
let paddle = PaddleProvider::from_env().expect("Paddle env vars not set");

// または直接構築する：
let paddle = PaddleProvider::new(
    "pdl_sdbx_apikey_...",
    "pdl_ntfset_...",
    "test_...",
    PaddleEnvironment::Sandbox,
).expect("Paddle client init failed");

PaymentProviderRegistry::bind("paddle", Arc::new(paddle));
```

Paddleは Merchant of Record です - 税務、支払いの督促、そしてサブスクリプションのライフサイクル全体を管理します。サーバーサイド確定を公開しないため、`Payment` は実装されません。`provider.as_payment()` を呼ぶと `None` が返ります。サブスクリプションは間接的に作成されます：`Checkout::start_session` を呼び出し、Paddleのウィジェットを完了させると、`SubscriptionCreated` webhookが到着してサブスクリプションIDを確定させます。

## トレイトの分割

`PaymentProvider` は、あらゆるアダプターが実装する4つの汎用トレイト - `Checkout`、`Subscription`、`CustomerStore`、`WebhookHandler` - をまとめた傘トレイトです。さらに2つのトレイトがオプションです：`Payment`（サーバーサイド確定はStripeのようなゲートウェイにのみ意味があります）と `Promotions`（プロモーションコード発行）。アダプターは、`PaymentProvider::as_payment()` / `PaymentProvider::as_promotions()` をオーバーライドすることでオプトインします。

```rust,ignore
pub trait PaymentProvider: Checkout + Subscription + CustomerStore + WebhookHandler {
    fn name(&self) -> &'static str;

    /// このプロバイダーが `Payment`（サーバーサイド確定）も実装していれば `Some` を返す。
    /// デフォルトは `None` を返す。
    fn as_payment(&self) -> Option<&dyn Payment> {
        None
    }

    /// このプロバイダーが `Promotions` も実装していれば `Some` を返す
    /// （プロモーションコード発行）。デフォルトは `None` を返す。
    fn as_promotions(&self) -> Option<&dyn Promotions> {
        None
    }
}
```

### `Checkout` - 汎用、クライアントウィジェットを開く

すべてのプロバイダーが `Checkout` を実装します。`start_session` を呼び出すと、あなたのフロントエンドがレンダリングする、flowタグ付きの `SessionPayload` が得られます。`session_status`（デフォルト：`NotSupported`。セッションを問い合わせ可能なプロバイダー、たとえばStripeでは上書きされます）は、以前に開始したセッションの、プロバイダー側の正式な状態を報告します。

```rust,ignore
#[async_trait]
pub trait Checkout: Send + Sync {
    async fn start_session(&self, req: StartSessionRequest) -> PaymentResult<SessionPayload>;

    async fn session_status(&self, provider_session_id: &str)
        -> PaymentResult<CheckoutSessionState>;
}
```

`StartSessionRequest` のフィールド：

| フィールド | 型 | 説明 |
|---|---|---|
| `mode` | `SessionMode` | `OneOff` または `Subscription` |
| `customer_ref` | `String` | `CustomerStore::create_customer` からのプロバイダーカスタマーID |
| `price_refs` | `Vec<String>` | プロバイダーの価格 / 商品ID |
| `success_return_url` | `String` | 支払い後にユーザーを送る先 |
| `cancel_return_url` | `String` | ユーザーが中断した場合に送る先 |
| `amount_hint` | `Option<Money>` | 一度限りの金額に対する上書きまたはヒント |
| `idempotency_key` | `Option<String>` | 安全なリトライのため |

`session_status` は、リダイレクトフローのためのサーバーサイド検証プリミティブです。カスタマーがあなたの復帰ページに戻ってきたとき、そのブラウザが運んできたクエリパラメータを信頼しては**いけません** - `start_session` の時点で記録した `provider_session_id` を渡し、結果で分岐してください：

```rust,ignore
match provider.session_status(&order.provider_session_id).await? {
    CheckoutSessionState::Complete { paid: true, payment_ref, amount_total } => {
        // 注文を履行する。`payment_ref`（例：Stripeの `pi_…`）は、
        // `Payment` の操作と `payments_transactions` ミラーに突き合わせられる。
    }
    CheckoutSessionState::Complete { paid: false, .. } => { /* 確定処理待ち */ }
    CheckoutSessionState::Open => { /* カスタマーがまだ支払いを終えていない */ }
    CheckoutSessionState::Expired => { /* セッションが失効した - 注文をクローズする */ }
}
```

同じ呼び出しは、突き合わせ処理も支えます：あなたのデータベースの中でまだopenな注文を再ポーリングし、カスタマーがタブを閉じた後にセッションが完了したものを履行してください。

### `Payment` - オプション、サーバーサイド確定

サーバーサイド確定を公開するプロバイダーだけが `Payment` を実装します。Stripeは実装し、Paddleは実装しません。実行時にチェックするには：

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

完全な `Payment` インターフェース：

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

`ChargeResult` は、`kind` でタグ付けされた列挙型です - [MoneyとChargeResult](#chargeresult)の節を参照してください。

### `Promotions` - オプション、プロモーションコードを発行する

プロモーションコードのサーフェスを持つプロバイダーは `Promotions` を実装します。割引そのもの（パーセントオフまたは金額オフのクーポン）は前もって作成されます - 通常は一度、プロバイダーのダッシュボードで - そして、このトレイトはそこから*コード*を発行し、それぞれを1人のカスタマーと1回の引き換えウィンドウに制限します。それは、リテンション施策やアップセル施策が必要とする形です：受け取った人ごとに個人用のコードが渡され、他の誰にも使えず、ウィンドウが閉じると失効します。

```rust,ignore
let provider = PaymentProviderRegistry::get("stripe").unwrap();
if let Some(promotions) = provider.as_promotions() {
    let minted = promotions.create_promotion_code(CreatePromotionCodeRequest {
        coupon_ref: "coupon_15off".into(),          // 事前に作成されたクーポン
        customer_ref: "cus_...".into(),             // このカスタマーだけが引き換え可能
        expires_at: Some(chrono::Utc::now() + chrono::Duration::days(7)),
        max_redemptions: Some(1),                   // 単回限り
    }).await?;
    // `minted.code` をカスタマーにメールで送る。カスタマーはそれをチェックアウトで
    // 入力し、プロバイダーがすべての制限を強制する。
}
```

`MockPaymentProvider` は `Promotions` を実装しており（コードは `PROMO_MOCK_n` として発行されます）、すべてのリクエストを記録します - テストでは `recorded_promotion_requests()` をアサートしてください。

### `Subscription` - サブスクライブ、更新、キャンセル、取得

```rust,ignore
#[async_trait]
pub trait Subscription: Send + Sync {
    async fn subscribe(&self, req: SubscribeRequest) -> PaymentResult<SubscriptionResult>;
    async fn update(&self, req: UpdateSubscriptionRequest) -> PaymentResult<SubscriptionResult>;
    async fn cancel(&self, provider_subscription_id: &str, at_period_end: bool) -> PaymentResult<SubscriptionResult>;
    async fn get(&self, provider_subscription_id: &str) -> PaymentResult<SubscriptionResult>;
}
```

期間の終わりにキャンセルする（請求サイクルが終わるまでアクセスを保持する）：

```rust,ignore
let sub = provider.cancel(&sub_id, true).await?;
// sub.cancel_at_period_end == true, sub.status == Active

// 即時にキャンセルする：
let sub = provider.cancel(&sub_id, false).await?;
// sub.status == Canceled
```

注：`Paddle::subscribe` は `PaymentError::NotSupported` を返します - Paddleは直接のAPI呼び出しではなく、チェックアウトの完了を通じてサブスクリプションを作成します。`Checkout::start_session` を使い、`SubscriptionCreated` webhookを待ってください。

### `CustomerStore` - 作成、更新、取得、削除

```rust,ignore
#[async_trait]
pub trait CustomerStore: Send + Sync {
    async fn create_customer(&self, req: CreateCustomerRequest) -> PaymentResult<CustomerRef>;
    async fn update_customer(&self, req: UpdateCustomerRequest) -> PaymentResult<CustomerRef>;
    async fn get_customer(&self, provider_customer_id: &str) -> PaymentResult<CustomerRef>;
    async fn delete_customer(&self, provider_customer_id: &str) -> PaymentResult<()>;
}
```

`CreateCustomerRequest` は、`user_id`、`email`、`name: Option<String>`、そして `metadata: Option<Value>` を取ります。`CustomerRef` は `provider_customer_id` を伴って返ってきます - 以降の呼び出しで使うために、あなたのユーザーレコードと並べてそれを保存してください。

### `WebhookHandler` - 検証、パース、抽出

```rust,ignore
#[async_trait]
pub trait WebhookHandler: Send + Sync {
    fn verify(&self, ctx: &WebhookContext<'_>) -> PaymentResult<()>;
    fn parse_event(&self, body: &[u8]) -> PaymentResult<WebhookEvent>;

    /// 生のペイロードからエンティティIDを取り出し、フレームワークがどの
    /// ミラー行を更新すべきかを知れるようにする。デフォルトは空の `PayloadIds` を返す。
    fn extract_payload_ids(&self, event: &WebhookEvent) -> PayloadIds;

    /// payment / invoiceイベントから `PaymentSnapshot` を構築する。デフォルトは
    /// `None` を返し、その場合 `payments_transactions` のupsertはスキップされる。
    fn extract_payment_snapshot(&self, event: &WebhookEvent) -> Option<PaymentSnapshot>;

    /// customerイベントから `CustomerSnapshot` を構築する。デフォルトは `None` を
    /// 返し、その場合、既存の行に対するemail / metadataの更新はスキップされる。
    fn extract_customer_snapshot(&self, event: &WebhookEvent) -> Option<CustomerSnapshot>;
}
```

実際には、これらのどれも直接呼び出すことはありません - `webhook_routes` が、そこに届くすべてのリクエストに対してこれらを呼び出します。これらがトレイトの上にあるのは、アダプタークレートが、プロバイダー固有の署名検証、イベントのパース、ペイロードの抽出を、テスト可能な形で実装できるようにするためです。`extract_*` メソッドはすべて、まともなデフォルトを持ちます - 出荷されているStripeとPaddleのアダプターは、プロバイダーの形を意識した実装でそれらを上書きします（Stripeは `data.object.*` へ、Paddleは `data.*` へ入っていきます）。

## flowタグ付きのInertiaペイロード

`start_session` は、`flow` の判別フィールドを持つJSONへシリアライズされる `SessionPayload` 列挙型を返します。あなたのフロントエンドは、`flow` で分岐して正しいウィジェットをレンダリングします：

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
    /// モバイルマネーのフロー - リダイレクトも埋め込みもない。フロントエンドは、
    /// カスタマーに電話で確認するよう伝えるユーザー向けメッセージを表示し
    /// （USSDプロンプトまたは通信事業者アプリ）、それから
    /// `provider_transaction_id` でプロバイダーをポーリングして状態を更新する。
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

`StripeElements` ペイロードのシリアライズ形式：

```json
{
  "flow": "stripe_elements",
  "client_secret": "pi_..._secret_...",
  "publishable_key": "pk_live_...",
  "provider_session_id": "pi_..."
}
```

`MobileMoneyPrompt` ペイロードはこのようになります - カスタマーがあなたのページを離れることはないため、URLはありません。フロントエンドは `message` をレンダリングし、ポーリングを開始します：

```json
{
  "flow": "mobile_money_prompt",
  "provider_transaction_id": "ch_mm_...",
  "message": "Check your phone for the MTN MoMo prompt.",
  "operator": { "kind": "mtn_momo" }
}
```

プロバイダーが生成した方のバリアントを、あなたのコントローラーからInertiaのpropsとして返してください。フロントエンドの統合については、[支払い - フロントエンド 統合](payments-frontend.md)で説明されています。

## ミラーテーブル

6つのテーブルがフレームワークのマイグレーションによって作成されます。公開エイリアスを取り込み、あなたのアプリのマイグレーターに含めてください：

```rust,ignore
use sea_orm_migration::{MigrationTrait, MigratorTrait};
use suprnova::payments::migrations::CreatePaymentsTables;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            // ... あなたの他のマイグレーション ...
            Box::new(CreatePaymentsTables),
        ]
    }
}
```

同じモジュールは、ヘルパー `pub fn migrations() -> Vec<Box<dyn MigrationTrait>>` も公開しています。その代わりにこれを呼んで、結果を自分のリストへ展開したい場合はどうぞ。

### テーブル概観

| テーブル | 目的 |
|---|---|
| `payments_customers` | `(provider, user_id)` の組ごとに1行 |
| `payments_payment_methods` | カスタマーごとに保存された支払い方法 |
| `payments_subscriptions` | サブスクリプションのライフサイクル状態 |
| `payments_subscription_items` | サブスクリプション内の明細項目 |
| `payments_transactions` | 一度限りの請求とサブスクリプションのインボイス |
| `payments_webhook_events` | 監査ログとべき等性ガード |

すべてのテーブルは `provider_metadata` というJSONカラムを持ちます。フレームワークのニュートラルな表現がプロバイダー固有のフィールドをカバーしていないときは、そこから読み取ってください。

### トランザクションテーブル

`payments_transactions` は、金額を `amount_total_minor` と `amount_tax_minor` に分割します。Stripeは税抜きの金額を報告します - トランザクション行では税額はゼロになり、税に関するデータは `provider_metadata` に入ります。Paddleは税込みの金額を報告し、`amount_tax_minor` に税の構成部分を設定します。どちらの表現も動作します。純額を得るには `amount_total_minor - amount_tax_minor` を足してください。

### Webhookイベントテーブル

`payments_webhook_events` は、`UNIQUE(provider, provider_event_id)` インデックスを持ちます。すべての受信webhookは、処理される前にこれと照合されます - 重複は、再処理することなく200 OKを返します。これは要となる仕組みです：Stripe、Paddle、そしてほとんどのプロバイダーは、失敗したwebhookを積極的にリトライします。

### 注意点

ドメインコードは、プロバイダーAPIから直接ではなく、ミラーテーブルから読み取ります。ミューテーション（サブスクリプションの作成、キャンセルなど）はプロバイダーへ向かい、結果として生じるwebhookがミラーテーブルを同期して戻します。つまり、ミューテーションとwebhookの到着との間には短いウィンドウがあり、その間、あなたのミラーテーブルは遅れをとります。あなたのUXは、これを考慮して設計してください（「処理中」のような状態を表示する、即時の確認にはプロバイダーのリダイレクトURLに頼る）。

## Webhookの処理

webhookの受信ルートは、ブートストラップ時に一度だけマウントしてください - 合成のパターンについては[クイックスタート](#クイックスタート)のルートの例を参照してください。`webhook_routes(db)` は、フレームワークに組み込まれた、単一の `POST /webhooks/payments/{provider}` ハンドラを運ぶ `Router` を返します。あなたは自分のルートをその上にチェーンします（あるいは、あなた自身の `routes!{}` ブロックの中で、ルートの内部のプリミティブを直接呼び出すこともできます）。

フレームワークのハンドラは、各リクエストに対して次のことを行います：

1. `PaymentProviderRegistry` の中で、名前付きのプロバイダーをルックアップする。
2. `WebhookHandler::verify` を呼び出して署名をチェックする。失敗すると401を返す。
3. `WebhookHandler::parse_event` を呼び出して `WebhookEvent` を構築する。パースに失敗すると400を返す。
4. 同じ `(provider, provider_event_id)` を持つ既存の行を、`payments_webhook_events` の中で確認する。見つかった場合は、即座に200を返す - これがべき等性ガードである。
5. 監査行を挿入する。

### WebhookEventの構造

```rust,ignore
pub struct WebhookEvent {
    pub provider: String,
    pub provider_event_id: String,
    pub provider_event_type: String,        // 生のプロバイダー文字列、例："customer.subscription.created"
    pub neutral: Option<NeutralEventKind>,  // フレームワークの分類法にマッピングされたもの、あるいはプロバイダー固有のイベントの場合はNone
    pub raw_payload: Value,                 // フォールスルーのための完全なJSONボディ
}
```

`NeutralEventKind` は、共通の経路をカバーします：

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

`neutral` が `None` のとき、そのイベントはプロバイダー固有です。完全なデータについては、`provider_event_type` と `raw_payload` を読んでください。

### ミラーテーブルの更新

監査行が永続化された後、フレームワークは `neutral` に基づいて、そのイベントを関連するミラーテーブルへディスパッチします。**1つのイベントに対するすべてのミラー書き込みは、`mark_processed` と一緒に、単一のDBトランザクションの中で起こります** - 部分的なミラーの状態が観測されることは決してありません。すべてが一緒にコミットされるか、すべてがロールバックされるかのどちらかです。

| `NeutralEventKind`               | ミラーへの効果                                                                                       |
|----------------------------------|-----------------------------------------------------------------------------------------------------|
| `SubscriptionCreated/Updated`    | プロバイダーに対して `Subscription::get(id)` を呼び、`payments_subscriptions` をupsertし、明細項目を同期する。 |
| `SubscriptionCanceled`           | 上と同じ。加えて、既存の行に対して `canceled_at` を設定し、`status` を `canceled` に切り替える。        |
| `PaymentSucceeded / Failed / Refunded / Disputed` | プロバイダーが `raw_payload` から生成するスナップショットから、`payments_transactions` をupsertする。        |
| `InvoicePaid / InvoiceFailed`    | `provider_subscription_id` を紐付けた `payments_transactions` をupsertする。                              |
| `CustomerCreated / CustomerUpdated` | プロバイダーの `CustomerSnapshot` から、既存の `payments_customers` 行の `email` / `provider_metadata` を更新する。**挿入は決してしない。**   |
| `None`（マッピングされない場合）                | 監査行のみ - ミラーへの変更なし。                                                                   |

カスタマーミラーは、webhookの経路上で意図的に更新専用になっています。`user_id` は `NOT NULL` であり、どのプロバイダーカスタマーがどのユーザーに属するかを知っているのはアプリだけです（そのリンクは、`CustomerStore::create_customer` の直後にあなたのコードによって作られます）。範囲外のカスタマー - たとえばStripeのダッシュボードで作成されたもの - はログに記録されますが、ミラーへ合成されることは決してありません。

### 障害復旧の契約

このハンドラは、プロバイダーのリトライを復旧の仕組みとして扱います：

- **更新の成功：** トランザクションがコミットされ、`processed_at` が設定され、`process_error` がクリアされる。レスポンス：`200 ok`。
- **更新の失敗：** トランザクションがロールバックされ（部分的なミラーの状態はない）、監査行は `processed_at = NULL` を保持し、`process_error` が失敗を記録する。レスポンス：`503 hydration-failed` - プロバイダーはバックオフしながらリトライする。
- **プロバイダーが失敗したイベントをリトライする：** べき等性チェックは既存の監査行を見つけるが `processed_at IS NULL` であるため、更新処理が再度実行される。そのリトライは、古い `process_error` を、今回の試行の結果で置き換える。
- **プロバイダーが成功したイベントをリトライする：** べき等性チェックは `processed_at IS NOT NULL` を見つけ、即座に `200 duplicate` を返す。再更新はしない。

ペイロードの中で `subscription_id` / `customer_id` が欠けているサブスクリプション / カスタマーイベントは、`Validation` エラーとして扱われます（同じく503 + `process_error` の記録）。不正なペイロードでサイレントに成功すると、運用者からの可視性なしにミラーが古びたままになってしまいます。

プロバイダー側でサブスクリプションから削除された明細項目（例：ユーザーがシートのアドオンを1つ外した場合）は、次の `subscription.updated` webhookが到着したときに、`payments_subscription_items` から削除されます。プロバイダーの `Subscription::get(id)` のレスポンスが、あらゆる同期における正式な情報源です。

## カード以外の支払い方法

`PaymentMethod` は、`payments_payment_methods` に保存する方法のため、そしてメソッドのメタデータを公開するあらゆるプロバイダーのために、フレームワークが使う列挙型です。これは、明白なケース - カード、銀行振込、電子ウォレット - に加えて、多くの市場で第一級とされている地域的な方法をカバーします：

```rust,ignore
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PaymentMethod {
    Card { brand: String, last4: String, exp_month: u8, exp_year: u16 },
    BankTransfer { bank_name: String, last4: String },
    EWallet { provider: String, identifier: String },
    /// 電話番号 + 通信事業者 + 国で識別される支払人。
    MobileMoney {
        operator: MobileMoneyOperator,
        phone: PhoneNumber,
        country: CountryCode,
    },
    /// ペグされた暗号資産 - ほとんどのプロバイダーにとって現金と等価。
    Stablecoin { asset: StablecoinAsset, network: Option<String> },
    /// ペグされていない暗号資産。
    Crypto { network: String, address: String },
    /// まだモデル化されていない地域固有 / プロバイダー固有の方法のためのエスケープハッチ。
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

名前の付いた通信事業者と資産は、私たちが列挙してきたものです。それぞれの `Custom { ... }` バリアントは、まだ固めていない地域の通信事業者やステーブルコインをカバーするため、1つへの対応を追加してもフレームワークのリリースを強いることはありません。

`PhoneNumber` と `CountryCode` は、`suprnova::payments` の中で検証されるDTOです - これらは、構築時に不正な入力を拒否します。それは、プロバイダー呼び出しの時点ではなく、失敗してほしい場所です。

## Money

金額は `Money` として表現されます - `i64` の最小単位のカウントと `Currency` です。`f64` は一切関与しません。

```rust,ignore
use suprnova::payments::{Money, Currency};
use rust_decimal::Decimal;
use std::str::FromStr;

// 最小単位から（セント、ペンス、円など）
let price = Money::from_minor_units(1999, Currency::USD);  // $19.99

// 10進数の文字列から
let price = Money::from_decimal(Decimal::from_str("19.99").unwrap(), Currency::USD);

// ゼロ小数の通貨 - 最小単位1234 = 1234 JPY（変換なし）
let yen = Money::from_minor_units(1234, Currency::JPY);

// 算術 - 通貨が一致しないとパニックする
let total = price + Money::from_minor_units(100, Currency::USD);  // $20.99

// 負の値は返金またはクレジットを表す
let refund = Money::from_minor_units(-500, Currency::USD);  // -$5.00

// 読み取り
println!("{} minor units in {:?}", price.minor_units(), price.currency());
```

`Add` と `Sub` は、通貨が一致しない場合と `i64` がオーバーフローする場合にパニックします。正しさのためにパニックする算術を使ってください - サイレントな異なる通貨同士の加算は、機能ではなくバグです。

## ChargeResult

`Payment::charge` は `ChargeResult` 列挙型を返します。すべての支払いが即座に完了するわけではありません - 3DSのステップアップやオフセッションのカードは、リダイレクトやクライアントサイドのアクションを必要とすることがあります：

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

`RequiresClientAction` は、そのペイロードをあなたのフロントエンドへ返すことで処理してください。フロントエンドは、`client_secret` + `publishable_key` を使って3DSチャレンジをレンダリングします。フロントエンドのディスパッチコードについては、[支払い - フロントエンド 統合](payments-frontend.md)を参照してください。

## べき等キー

すべてのミューテーションを行うDTOは、オプションの `idempotency_key: Option<String>` を持ちます。リトライ可能なネットワーク呼び出しにはこれを設定してください：

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

Stripeは、`Idempotency-Key` HTTPヘッダーを介してべき等キーを尊重します - 同じ本文を伴う同じキーは、24時間のリプレイウィンドウの間、同じレスポンスオブジェクトを返します。本文が一致しない場合はエラーが返ります。Paddleにも同等の仕組みがあります。リクエストが途中で失敗し、同じキーでリトライした場合、プロバイダーは、重複した請求やサブスクリプションを作る代わりに、元のレスポンスを返します。

## 判別パターン

`PaymentProvider` の実装を主張するすべてのアダプターは、同じE2Eフローに合格しなければなりません：

```
create_customer → start_session → subscribe → get → cancel(at_period_end) → cancel(immediate) → assert as_payment invariant
```

フレームワークに含まれる `MockPaymentProvider` は、これに合格します：

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

    // 期間の終わりにキャンセルする
    let s = provider.cancel(&sub.provider_subscription_id, true).await.unwrap();
    assert!(s.cancel_at_period_end);

    // 即時にキャンセルする
    let s = provider.cancel(&sub.provider_subscription_id, false).await.unwrap();
    assert_eq!(s.status, SubscriptionStatus::Canceled);

    // MockPaymentProviderは意図的にPaymentを省いている（Paddle風のオプション）
    let p: &dyn PaymentProvider = &provider;
    assert!(p.as_payment().is_none());
}
```

`MockPaymentProvider` は `Payment` を実装していません - これは、Paddleと同じ不変条件を検証します。`StripeProvider` と `PaddleProvider` は、どちらも同じフローを、統合テストの中で本物のAPIに対して合格させます。

## 複数プロバイダーアプリ

起動時に両方のアダプターを登録し、それぞれのカスタマーのレコードがどこで作成されたかに基づいてディスパッチしてください：

```rust,ignore
PaymentProviderRegistry::bind("stripe", Arc::new(stripe_provider));
PaymentProviderRegistry::bind("paddle", Arc::new(paddle_provider));

// 後で、リクエストごとに：
let provider_name = user.payment_provider.as_str(); // "stripe" または "paddle"
let provider = PaymentProviderRegistry::get(provider_name).expect("unknown provider");
let sub = provider.cancel(&sub_id, true).await?;
```

よくある使い方：EUのカスタマーはPaddle経由（MoRの税務処理のため）、USのカスタマーはStripe経由でルーティングする。プロバイダー間でチェックアウトのコンバージョンをA/Bテストする。サブスクリプションには1つのプロバイダーを、一度限りの請求には別のプロバイダーを使う。

## Laravel Cashierからの移行

Cashierは、設計上Stripe専用です。Suprnovaは標準でマルチプロバイダーを出荷します。簡単な対応表：

| Laravel Cashier | Suprnova |
|---|---|
| `$user->newSubscription('default', 'price_pro')->create()` | `provider.subscribe(SubscribeRequest { ... }).await` |
| `$user->subscription('default')->cancel()` | `provider.cancel(&sub_id, true).await` |
| `Cashier::webhookHandler` | `webhook_routes(db.clone())` |
| `$user->createAsStripeCustomer()` | `provider.create_customer(CreateCustomerRequest { ... }).await` |
| `$user->charge(1999, 'pm_...')` | `payment.charge(ChargeRequest { ... }).await`（プロバイダーが対応していれば） |
| `$invoice->download()` | 組み込みではない。トランザクションミラーテーブルから `provider_metadata["invoice_pdf_url"]` を読む |

## 次のステップ

- [支払い - Stripe アダプター](payments-stripe.md) - ゲートウェイのフローの詳細：PaymentIntents、webhookの署名形式、イベントタイプのマッピング
- [支払い - Paddle アダプター](payments-paddle.md) - MoRのフローの詳細：チェックアウト主導のサブスクリプション作成、税務処理、通知の検証
- [支払い - フロントエンド 統合](payments-frontend.md) - Svelte 5、React 19、Vue 3.5のflowディスパッチの例
- [支払い プロバイダー アダプター の作成](payments-provider-guide.md) - あなた自身のアダプタークレートをエンドツーエンドで構築する
- [データベース](database.md) - ミラーテーブルが載っているSeaORM層
