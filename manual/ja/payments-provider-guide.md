# 支払い プロバイダー アダプターの作成

このガイドは、Suprnovaのプロバイダーニュートラルな支払いサーフェスに接続する、サードパーティのアダプタークレート - `suprnova-payments-mollie` - の構築を順に説明します。最後まで終えると、自分自身を登録し、判別フローに合格し、1回の `cargo add` であらゆるSuprnovaアプリに組み込めるクレートが手に入ります。

同じ構造は、あらゆるプロバイダーに当てはまります：Square、Braintree、Adyen、あるいはHTTP APIを持つ他の何にでもです。

### Suprnovaが異なる設計を選んだ理由

Laravelは、Cashierを第一級のStripe統合として出荷しています。Stripeの経路においては優れていますが、1つのプロバイダーの語彙をフレームワークに固定化してしまいます - 2番目のプロバイダーを追加するには、Cashierをフォークするか、その横に並行するサーフェスを構築するしかありません。

Suprnovaは、すべてのプロバイダーを同じ5トレイトの契約の上に保ちます：`Checkout`、`Subscription`、`CustomerStore`、`WebhookHandler`、そしてサーバーサイド確定に対応するプロバイダー向けのオプションの `Payment` です。ドメインコードは、レジストリから得た `Arc<dyn PaymentProvider>` だけを持ちます。StripeをPaddleに（あるいは、あなたがこれから書くMollieアダプターに）差し替えることは、ブートストラップの変更であり、コードの変更ではありません。`crates/suprnova-payments-stripe/` と `crates/suprnova-payments-paddle/` にある参照アダプターは、直接確定型のゲートウェイとMerchant of Recordという、2つのまったく異なる商業モデルに対して、このトレイト契約が成り立つことを証明しています - あなたのアダプターも、同じ形に収まります。

## 1. ワークスペースメンバークレートを作成する

リポジトリのルートから：

```bash
cargo new --lib crates/suprnova-payments-mollie
```

あなたのルートの `Cargo.toml` に追加してください：

```toml
[workspace]
members = [
    "framework",
    "app",
    "suprnova-cli",
    "suprnova-macros",
    "crates/suprnova-payments-mollie",  # この行を追加する
]
```

（参照アダプター - `crates/suprnova-payments-stripe` と `crates/suprnova-payments-paddle` - は、この同じ `crates/` ディレクトリの中にあり、このガイドと並べて読むのに適したテンプレートです。）

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
# あなたのMollie SDK：
mollie-rs = "0.1"
hmac = "0.12"   # webhookのHMAC検証のため
sha2 = "0.10"
hex = "0.4"

[dev-dependencies]
tokio = { version = "1", features = ["full"] }
```

## 2. ソースファイルを配置する

出荷されているアダプターが使う構造を反映してください：

```
crates/suprnova-payments-mollie/src/
├── lib.rs          # MollieProvider構造体、PaymentProvider実装、from_env
├── checkout.rs     # Checkout実装
├── customer.rs     # CustomerStore実装
├── subscription.rs # Subscription実装
├── webhook.rs      # WebhookHandler実装
├── event_map.rs    # プロバイダーのイベント文字列 → NeutralEventKind
└── payment.rs      # Payment実装（Mollieがサーバーサイド確定に対応していれば）
```

## 3. `lib.rs` - プロバイダー構造体

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

/// Suprnovaのプロバイダーニュートラルな支払いサーフェスのためのMollieアダプター。
#[derive(Clone, Debug)]
pub struct MollieProvider {
    /// Mollie APIキー（`test_…` / `live_…`）。
    api_key: String,
    /// webhookの署名シークレット - HMAC検証で使う。
    webhook_secret: String,
    /// HTTPクライアント - リクエスト間で共有する。
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

    /// 環境変数から構築する。
    ///
    /// 読み取るもの：
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

    // `Payment`（サーバーサイド確定）も実装している場合だけ `as_payment()` をオーバーライドする。
    // `PaymentProvider` のデフォルト実装は `None` を返す - Mollieがチェックアウトのみ /
    // MoR的なものなら、このオーバーライドを完全に省いてよい。
    fn as_payment(&self) -> Option<&dyn Payment> {
        Some(self)
    }
}
```

`PaymentProvider` は傘トレイトです - スーパートレイトの句は `Checkout + Subscription + CustomerStore + WebhookHandler` であるため、あなたのプロバイダーが4つすべてを実装するまで、コンパイラはそれを束縛することを拒みます。5番目のトレイトである `Payment` は**オプション**です - サーバーサイド確定を公開するプロバイダーだけがこれを実装し、`as_payment()` がその結果をフレームワークへ報告します。デフォルトの `as_payment()` は `None` を返すため、あなたのプロバイダーがサーバーサイド確定を行わないなら、オーバーライドを完全に省いてください。

## 4. 4つの必須トレイトを実装する

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
        // Mollie APIを呼び出し、支払いまたは注文を作成する。
        // レスポンスをSessionPayloadのいずれかのバリアントにマッピングする。
        // Mollieはホスト型のチェックアウトページを使うため、Redirectが自然な形になる。
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
        // ここでMollie SDKの呼び出しを配線する。
        // ホスト型のチェックアウトURLを返す。
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
        // Mollieへ POST /v2/customers
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
            // キャンセル日を期間の終わりに設定する
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

あなたのプロバイダーがあるメソッドに対応していない場合は、`PaymentError::NotSupported` を返してください：

```rust,ignore
Err(PaymentError::NotSupported(
    "Mollie creates subscriptions via checkout - use start_session instead".into()
))
```

### `payment.rs` - サーバーサイド確定（オプション）

あなたのプロバイダーが、保存された支払い方法に対する直接のサーバーサイド請求に対応している場合だけ、これを実装してください。これを省く場合は、`lib.rs` の中の `as_payment()` オーバーライドを削除してください。

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

## 5. プロバイダーのイベントを `NeutralEventKind` にマッピングする

**`event_map.rs`：**

```rust,ignore
use suprnova::payments::NeutralEventKind;

/// Mollieのwebhookイベントタイプ文字列を、フレームワークのニュートラルな分類法にマッピングする。
/// ニュートラルな対応物を持たないプロバイダー固有のイベントには `None` を返す。
pub fn mollie_event_to_neutral(event_type: &str) -> Option<NeutralEventKind> {
    match event_type {
        // Mollieの支払い
        "payment.paid"          => Some(NeutralEventKind::PaymentSucceeded),
        "payment.failed"        => Some(NeutralEventKind::PaymentFailed),
        "payment.expired"       => Some(NeutralEventKind::PaymentFailed),
        "refund.created"        => Some(NeutralEventKind::PaymentRefunded),
        "chargeback.created"    => Some(NeutralEventKind::PaymentDisputed),
        // Mollieのサブスクリプション
        "subscription.created"  => Some(NeutralEventKind::SubscriptionCreated),
        "subscription.updated"  => Some(NeutralEventKind::SubscriptionUpdated),
        "subscription.canceled" => Some(NeutralEventKind::SubscriptionCanceled),
        // Mollieの注文 / インボイス
        "order.paid"            => Some(NeutralEventKind::InvoicePaid),
        // カスタマーイベント
        "customer.created"      => Some(NeutralEventKind::CustomerCreated),
        "customer.updated"      => Some(NeutralEventKind::CustomerUpdated),
        // プロバイダー固有 - raw_payloadへフォールスルーする
        _                       => None,
    }
}
```

少なくとも上に挙げたイベントはカバーしてください。ニュートラルな分類法にないイベントについては `None` を返してください - それでも `provider_event_type` + `raw_payload` の下で `payments_webhook_events` に永続化されるため、ドメインコードはそれを読み取れます。

## 6. Webhook署名検証を実装する

**`webhook.rs`：**

Mollieは、HMAC-SHA256を使ってwebhookのペイロードに署名します。タイミング攻撃を防ぐため、署名の比較は常に定数時間で行ってください。

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
        // Mollieが送ってくる署名ヘッダーを読む。
        // 正確なヘッダー名と署名スキームは、あなたのバージョンのMollieのドキュメントを確認する。
        let signature = ctx
            .headers
            .get("X-Mollie-Signature")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| PaymentError::WebhookSignature(
                "missing X-Mollie-Signature header".into()
            ))?;

        // 生のボディに対する、期待されるHMAC-SHA256を計算する。
        let mut mac = HmacSha256::new_from_slice(self.webhook_secret.as_bytes())
            .map_err(|e| PaymentError::Internal(format!("HMAC init: {e}")))?;
        mac.update(ctx.body);

        // 16進エンコードされた、受信した署名をデコードする。
        let received = hex::decode(signature)
            .map_err(|_| PaymentError::WebhookSignature("non-hex signature".into()))?;

        // 定数時間での比較。
        mac.verify_slice(&received)
            .map_err(|_| PaymentError::WebhookSignature("signature mismatch".into()))
    }

    fn parse_event(&self, body: &[u8]) -> PaymentResult<WebhookEvent> {
        // MollieはJSONを送ってくる - それをパースする。
        let raw: serde_json::Value = serde_json::from_slice(body)
            .map_err(|e| PaymentError::Validation(format!("invalid mollie webhook body: {e}")))?;

        let event_id = raw["id"].as_str()
            .ok_or_else(|| PaymentError::Validation("missing event id".into()))?
            .to_string();

        // 一部のwebhookの形では、Mollieはイベントタイプ文字列ではなくリソースタイプを使う。
        // あなたのSDKバージョンが送ってくるものに合わせて調整する。
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

要点：

- `PaymentError::WebhookSignature(String)` は、あらゆる署名の失敗 - ヘッダーの欠落、不正なエンコード、不一致 - に対する単一のバリアントです。フレームワークのwebhookルートは、あらゆる `WebhookSignature(_)` を401として扱います。
- パースできないボディには `PaymentError::Validation(String)` を使ってください。webhookルートは、パースの失敗に対して400を返します。
- フレームワークの `webhook_routes` ハンドラは、`parse_event` の前に `verify` を呼び出し、その後、DBトランザクションの中で更新処理を行います。更新処理の失敗は503を返すため、プロバイダーがリトライします。
- 生のシークレットや、受信した署名を、決してログに出さないでください。

### ミラーテーブルの更新：`extract_payload_ids` + `extract_payment_snapshot` + `extract_customer_snapshot`

`parse_event` が `WebhookEvent` を返した後、フレームワークのwebhookルートはミラーテーブルを更新します。3つのオプションのトレイトメソッドがそれを駆動します - すべて安全なデフォルトのno-op実装を持つため、アダプターはそれらなしで出荷しても、監査層は問題なく通過します：

```rust,ignore
fn extract_payload_ids(&self, event: &WebhookEvent) -> PayloadIds;
fn extract_payment_snapshot(&self, event: &WebhookEvent) -> Option<PaymentSnapshot>;
fn extract_customer_snapshot(&self, event: &WebhookEvent) -> Option<CustomerSnapshot>;
```

`PayloadIds` は、パースされたイベントとフレームワークのミラーロジックとの間の橋渡しです。フレームワークが正しいエンティティを見つけられるように、これを実装してください：

```rust,ignore
pub struct PayloadIds {
    pub subscription_id: Option<String>,
    pub customer_id: Option<String>,
    pub transaction_id: Option<String>,
}
```

それぞれの `neutral` の値について、プロバイダーのペイロードが公開しているIDを埋めてください。サブスクリプションイベントは `subscription_id` を設定するべきです。そうすれば、フレームワークは `Subscription::get(id)` を呼び出し、正式な状態からミラーを更新できます。カスタマーイベントは `customer_id` を設定します。payment / invoiceイベントは `transaction_id` を設定し、それが定期的な請求である場合は `subscription_id` も設定します。

`PaymentSnapshot` は、webhookのペイロードから直接構築されます - `Payment::get` のようなコールバックはありません。payment / invoiceのニュートラルイベントに対して、これを実装してください：

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
    pub provider_metadata: Value,   // 通常は、ペイロードの中のエンティティオブジェクト
}
```

Stripeの参照実装は、`PaymentIntent`/`Charge` イベントに対して `data.object.{id,amount,currency,customer}` を読み、`Invoice` イベントに対して `data.object.{id,amount_paid,tax,currency,customer,subscription,status_transitions.paid_at}` を読みます。Paddleの実装は `data.{id,customer_id,currency_code,details.totals.{total,tax},billed_at,subscription_id}` を読みます。あなたのプロバイダーのペイロードの形に合った慣習を反映してください - フレームワークは、あなたがどう抽出するかを気にしません。スナップショットが正しいことだけが問題です。

`extract_payment_snapshot` から `None` を返した場合、監査行はそれでも書き込まれますが、`payments_transactions` は触れられません。それは、サブスクリプション / カスタマーイベントに対する正しい戻り値であり、あるいは、ペイロードが行を埋めるのに十分な情報を運んでいない、あらゆるpaymentイベントに対する正しい戻り値でもあります。

`CustomerSnapshot` は、カスタマーミラーの同期をプロバイダー主導のまま保ちます（フレームワークにハードコードされたJSONパスはありません）：

```rust,ignore
pub struct CustomerSnapshot {
    pub provider_customer_id: String,
    pub email: Option<String>,
    pub provider_metadata: Value,
}
```

フレームワークは、スナップショットが1つ提供している場合にだけ `email = Set(snapshot.email)` とします。`provider_metadata` は常に、そのカスタマーに対するプロバイダーの見解で置き換えられます（`updated_at` も、いずれにせよ更新されます）。カスタマーミラーの行は、常に**更新**されるだけで - 決して挿入されません - `user_id` は `NOT NULL` であり、アプリが `CustomerStore::create_customer` を介して、ユーザーとカスタマーのリンクを所有しているためです。

### 失敗時の挙動

サブスクリプションイベントに対して `extract_payload_ids` が `subscription_id` に `None` を返した場合（あるいはカスタマーイベントに対して `customer_id` に `None` を返した場合）、フレームワークはそれを `Validation` エラーとして扱います：更新処理のトランザクションはロールバックされ、監査行の `process_error` が設定され、HTTPレスポンスは**503 hydration-failed**となり、プロバイダーがリトライします。不正なペイロードでサイレントに成功すると、運用者からの可視性なしにミラーが古びたままになってしまいます - プロバイダーのリトライが復旧の仕組みです。

この契約が意味するのは、アダプターの抽出器が関連するIDを誠実に埋めなければならないということです。`None` を返すことは、あなたのプロバイダーがまったく翻訳できないイベント（例えば、ペイロードにcharge IDがまったくないpaymentイベント）のために予約されており、「これはパースするのが面倒だった」ためのものではありません。

## 7. アプリの起動時に登録する

2つの仕組みが用意されています - どちらか一方を選んでください：

### 実行時登録（環境変数による設定を行うアプリに推奨）

```rust,ignore
use std::sync::Arc;
use suprnova::payments::PaymentProviderRegistry;
use suprnova_payments_mollie::MollieProvider;

let mollie = MollieProvider::from_env().expect("Mollie env vars not set");
PaymentProviderRegistry::bind("mollie", Arc::new(mollie));
```

### `inventory` によるコンパイル時登録

ゼロコンフィグの登録を望むアダプタークレートのために - 消費者がブート時の配線を一切行わずに `cargo add` するだけのライブラリを出荷するときに有用です：

```rust,ignore
use suprnova::payments::{PaymentProviderEntry, PaymentProviderRegistry};
use inventory;

// lib.rsの中、静的初期化子の中で：
inventory::submit!(PaymentProviderEntry {
    name: "mollie",
    factory: || Arc::new(MollieProvider::from_env().expect("Mollie env not set")),
});
```

`inventory::submit!` は `main` の前に実行されます。ファクトリークロージャは、レジストリが最初にアクセスされたときに一度だけ呼び出されます。

## 8. 判別テストに合格する

すべてのアダプタークレートは、トレイト契約がエンドツーエンドで正しいことを証明する統合テストを含めるべきです。これはその健全性の証明です - このテストが通れば、そのプロバイダーは、驚きなしに、あらゆるSuprnovaアプリに組み込めます。

```rust,ignore
// tests/discriminator.rs（crates/suprnova-payments-mollie/ の内側）

use suprnova::payments::*;
use suprnova_payments_mollie::MollieProvider;

/// MOLLIE_API_KEY と MOLLIE_WEBHOOK_SECRET が設定されていることを要求する。
/// 実行するには：cargo test --test discriminator -- --ignored
#[tokio::test]
#[ignore = "requires live Mollie sandbox credentials"]
async fn discriminator_flow() {
    let provider = MollieProvider::from_env().expect("Mollie env vars not set");

    // 1. カスタマーを作成する
    let cus = provider.create_customer(CreateCustomerRequest {
        user_id: "test_user_1".into(),
        email: "test@example.com".into(),
        name: Some("Test User".into()),
        metadata: None,
    }).await.expect("create_customer failed");
    assert!(!cus.provider_customer_id.is_empty());

    // 2. チェックアウトセッションを開始する
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

    // 3. 直接サブスクライブする（あなたのプロバイダーが対応していれば。Mollieはチェックアウトを要求する場合がある）
    let sub = provider.subscribe(SubscribeRequest {
        customer_ref: cus.provider_customer_id.clone(),
        price_refs: vec!["your_mollie_plan_id".into()],
        trial_days: None,
        idempotency_key: Some("discriminator_test_sub".into()),
        metadata: None,
    }).await.expect("subscribe failed");
    assert_eq!(sub.status, SubscriptionStatus::Active);

    // 4. 読み返す
    let fetched = provider.get(&sub.provider_subscription_id).await.expect("get failed");
    assert_eq!(fetched.provider_subscription_id, sub.provider_subscription_id);

    // 5. 期間の終わりにキャンセルする
    let s = provider.cancel(&sub.provider_subscription_id, true).await.expect("cancel failed");
    assert!(s.cancel_at_period_end);

    // 6. 即時にキャンセルする
    let s = provider.cancel(&sub.provider_subscription_id, false).await.expect("cancel failed");
    assert_eq!(s.status, SubscriptionStatus::Canceled);

    // 7. as_payment() の不変条件を検証する
    let p: &dyn PaymentProvider = &provider;
    // Paymentを実装している場合：assert!(p.as_payment().is_some())
    // Paymentを実装していない場合：assert!(p.as_payment().is_none())
    let _ = p.as_payment();
}
```

`cargo test` がCIで認証情報なしに通るように、ライブの統合テストは `#[ignore]` でゲートしてください。サンドボックスのアカウントに対して、`-- --ignored` で明示的に実行してください。

## 9. `PaymentError` バリアントのリファレンス

完全な列挙型は `framework/src/payments/error.rs` にあります。実際に何が起きたかに合ったバリアントを選んでください：

| バリアント | 使うべき場面 |
|---|---|
| `Provider(String)` | プロバイダーのAPIがエラーを返し、それをこれ以上翻訳する必要がない場合 |
| `Validation(String)` | リクエストのフィールドが不正である、あるいはwebhookのボディがパースできない場合 |
| `NotSupported(String)` | このプロバイダーにそのメソッドが当てはまらない場合（例：Paddleの `subscribe`） |
| `Declined { reason, decline_code }` | カードが拒否された - プロバイダーが `decline_code` を提供する場合はそれを通す |
| `Authentication(String)` | プロバイダーがあなたのAPIキーや認証情報を拒否した場合 |
| `NotFound(String)` | カスタマー、サブスクリプション、あるいはトランザクションIDが存在しない場合 |
| `WebhookSignature(String)` | あらゆる署名の失敗 - ヘッダーの欠落、不正なエンコード、あるいは不一致 |
| `InvalidPhoneNumber(String)` | モバイルマネーのフローで、E.164の検証が失敗した場合 |
| `InvalidCountryCode(String)` | ISO-3166-1 alpha-2の検証が失敗した場合 |
| `Internal(String)` | 予期しないSDKのエラー、ネットワークの障害、HMAC初期化の失敗、あるいはその他のフレームワーク側の問題 |

webhookルートは、これらをステータスコードにマッピングします：`WebhookSignature(_)` → 401、`parse_event` からの `Validation(_)` → 400、更新処理からのそれ以外 → 503（プロバイダーがリトライするように）。

あなたのアダプターがコンパイルされ、判別テストに合格したら：

- `cargo add suprnova-payments-mollie --path ./crates/suprnova-payments-mollie` で、あなたのアプリの `Cargo.toml` にクレートを追加してください。
- ステップ7で示したように、ブートストラップ時に登録してください。
- アプリの起動時に一度、`webhook_routes(db.clone())` をマウントしてください - 同じハンドラが、名前によって登録済みのすべてのプロバイダーへディスパッチするため、1回のマウントがStripe、Paddle、そしてあなたの新しいアダプターに対応します。

## 次のステップ

- [支払い](payments.md) - プロバイダーニュートラルなサーフェスとクイックスタート
- [支払い - Stripe アダプター](payments-stripe.md) - ゲートウェイアダプターの完全なテンプレート
- [支払い - Paddle アダプター](payments-paddle.md) - Merchant of Recordアダプターの完全なテンプレート
- [支払い - フロントエンド 統合](payments-frontend.md) - あなたのアダプターが返す `SessionPayload` をレンダリングする方法
- [エラー モデル](error-model.md) - `PaymentError` が `HttpResponse` としてどのように現れるか
