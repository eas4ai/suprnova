# 支払い - NOWPayments

`suprnova-payments-nowpayments` アダプターは、ホスト型インボイスを作成し、支払い通知を検証し、マーチャントAPIキーを使って支払いステータスを読み取ります。通常の支払いプロバイダーレジストリには `nowpayments` として登録されます。

## インストールと設定

両方の依存関係について、このアダプターを含むSuprnovaのリビジョンを使用してください。Suprnovaのソースチェックアウトの隣にあるローカルアプリケーションの場合：

```toml
[dependencies]
suprnova = { path = "../suprnova/framework" }
suprnova-payments-nowpayments = { path = "../suprnova/crates/suprnova-payments-nowpayments" }
serde_json = "1"
```

アプリケーションの保護された環境に、次の値を設定してください：

```dotenv
NOWPAYMENTS_ENVIRONMENT=sandbox
NOWPAYMENTS_API_KEY=your-sandbox-api-key
NOWPAYMENTS_IPN_SECRET=your-sandbox-ipn-secret
NOWPAYMENTS_IPN_CALLBACK_URL=https://app.example/webhooks/payments/nowpayments
```

環境は `sandbox` または `production` を受け付け、既定値は `sandbox` です。未知の環境や空の環境は、設定を失敗させます。空の認証情報は、HTTPリクエストの前に失敗します。環境ごとに別々の認証情報を使用してください。

起動時にプロバイダーを登録し、設定エラーを伝播させてください：

```rust,no_run
use std::sync::Arc;
use suprnova::payments::{PaymentProviderRegistry, PaymentResult};
use suprnova_payments_nowpayments::NowPaymentsProvider;

fn register_nowpayments() -> PaymentResult<Arc<NowPaymentsProvider>> {
    let provider = Arc::new(NowPaymentsProvider::from_env()?);
    PaymentProviderRegistry::bind("nowpayments", provider.clone());
    Ok(provider)
}
```

[支払いの概要](payments.md) に示されているとおり、支払いのマイグレーションを追加し、`webhook_routes(db)` をアプリケーションのルーターに組み込んでください。エンドポイントは `POST /webhooks/payments/nowpayments` です。このプロバイダー認証されたルートは、ブラウザセッションのCSRFミドルウェアの外側に置いてください。コールバックには、正確な公開HTTPS URLを設定してください。リクエストボディは、変更されないままアダプターに届く必要があります。

## ホスト型インボイスを開始する

リクエストを行う前に、一意のマーチャント注文参照を持つチェックアウト試行を永続化してください。金額は、アプリケーションが信頼する注文から取得します。ブラウザから供給された合計は決して使いません。

```rust,no_run
use suprnova::payments::{Money, SessionMode, StartSessionRequest};
use suprnova_payments_nowpayments::{InvoiceCreationError, NowPaymentsInvoice, NowPaymentsProvider};
use serde_json::json;

async fn create_order_invoice(
    provider: &NowPaymentsProvider,
    order_id: String,
    amount: Money,
) -> Result<NowPaymentsInvoice, InvoiceCreationError> {
    provider.create_invoice(StartSessionRequest {
        mode: SessionMode::OneOff,
        customer_ref: String::new(),
        price_refs: Vec::new(),
        amount_hint: Some(amount),
        success_return_url: "https://app.example/billing/return".into(),
        cancel_return_url: "https://app.example/billing/cancel".into(),
        idempotency_key: None,
        metadata: Some(json!({"order_id": order_id})),
    }).await
}
```

`Checkout::start_session` は同じリクエストを受け付け、汎用の `SessionPayload::Redirect` を返します。その `provider_session_id` を **インボイスID** として保存し、検証済みのURLへリダイレクトしてください。具象メソッドの `create_invoice` は、マーチャント注文参照も返し、結果が不確実な作成エラーを区別します。

インボイスアダプターは、一回限りモード、空の顧客参照と価格参照、そして法定通貨の指数が定義された正の `Money` 金額を要求します。主要単位の正確な小数金額と、小文字の通貨コードを送信します。`pay_currency` が指定されない限り、顧客はホスト型ページで暗号通貨を選択します。

メタデータは、次のフィールドを持つ厳格なオブジェクトです：

| フィールド | 意味 |
| --- | --- |
| `order_id` | 必須の空でないマーチャント参照、最大128バイト |
| `order_description` | 任意の説明、最大500バイト |
| `pay_currency` | 任意の小文字のプロバイダーティッカー。たとえば `btc` |
| `is_fixed_rate` | 任意のプロバイダー為替レート設定 |
| `is_fee_paid_by_user` | 任意のプロバイダー手数料設定 |

未知のメタデータキーは拒否されます。任意のアプリケーションデータや顧客データを、このオブジェクトに入れないでください。リターンURLは、コールバックと同じHTTPSオリジンを使用し、認証情報とフラグメントを含んではなりません。インボイスのリダイレクトは、選択されたNOWPayments環境を指し、返されたインボイスIDを含んでいる必要があります。

## 作成の失敗とリトライ

NOWPaymentsのインボイスエンドポイントは、冪等性キーの保証を文書化していません。アダプターは `None` ではない `idempotency_key` を拒否します。`order_id` は相関データであり、プロバイダー側の重複排除ではありません。HTTPリダイレクトも自動リトライも有効になっていません。リクエストには30秒の期限と64 KiBのレスポンス上限があります。

`InvoiceCreationError::Rejected` は、入力検証、またはAPIによる明確な拒否を表します。`Unknown` は、インボイスがすでに存在する可能性を意味します。たとえば、リクエストがタイムアウトした、プロバイダーがサーバーエラーを返した、あるいはその成功レスポンスが不正だった場合です。その試行を不確実なものとして記録し、別のインボイスを作成する前に、プロバイダーのダッシュボードで注文を照合してください。インボイスの作成を、汎用のリトライループの中に置かないでください。`Checkout::start_session` を通した場合、不明な結果は、この回復手順を伴う `PaymentError::Provider` になります。

## 支払いを検証して照合する

インボイスは、別個の支払いIDを生成することがあります。`payment_status(payment_id)` は認証済みの支払いエンドポイントを呼び出し、異なるIDを持つレスポンスを拒否します。`Checkout::session_status(invoice_id)` は `NotSupported` を返します。インボイスIDを支払い照会エンドポイントに代入することはできません。このアダプターは、インボイス単位で支払いを一覧するために、ダッシュボードのメールアドレスとパスワードの認証情報を使用しません。

```rust,no_run
use suprnova::payments::{Money, PaymentResult};
use suprnova_payments_nowpayments::{NowPaymentsProvider, NowPaymentsStatus};

async fn payment_matches_order(
    provider: &NowPaymentsProvider,
    verified_payment_id: &str,
    expected_invoice_id: &str,
    expected_order_id: &str,
    expected_price: Money,
) -> PaymentResult<bool> {
    let payment = provider.payment_status(verified_payment_id).await?;
    Ok(payment.status == NowPaymentsStatus::Finished
        && payment.invoice_id.as_deref() == Some(expected_invoice_id)
        && payment.order_id.as_deref() == Some(expected_order_id)
        && payment.price == expected_price)
}
```

支払いIDは、検証済みのIPN、または他の信頼できるサーバーサイドの記録から得なければなりません。ブラウザの戻りは、支払いの証拠にはなりません。照会が成功したことも、認可のチェックにはなりません。アクセスを変更する前に、保存された注文、インボイス、金額、通貨と突き合わせてください。`price` は要求された法定通貨の価格であり、受け取った暗号通貨の数量ではありません。マーチャントの部分支払いの受け入れ設定を確認してください。このアダプターは `partially_paid` を成功に変換することは決してありません。

| プロバイダーの状態 | 中立イベント |
| --- | --- |
| `finished` | `PaymentSucceeded` |
| `failed`, `expired`, `cancelled`, `canceled` | `PaymentFailed` |
| `refunded` | `PaymentRefunded` |
| `waiting`, `confirming`, `confirmed`, `sending`, `partially_paid`、および未知の値 | 中立な分類なし |

IPNの署名は、再帰的にソートされたJSONとHMAC SHA-512を使用します。検証には、一定時間のMAC比較を使用します。署名の欠落、重複、不正、JSONキーの重複、過大なペイロード、必須フィールドの不正な形式は、いずれも拒否されます。プロバイダーは独立した配信タイムスタンプに署名しないため、でっち上げたタイムスタンプのリプレイ窓は存在しません。リプレイ保護には、永続化されたレシートを使用します。

NOWPaymentsのIPNには、独立したイベントIDがありません。アダプターは支払いIDとステータスでレシートを識別するため、`updated_at` が異なる再配信が、同じ終端イベントを繰り返すことはありません。フレームワークは検証済みイベントを `payments_webhook_events` に保存し、失敗した処理をリトライします。遅れて届いた保留中のイベントが、確定済みのトランザクションミラーを変更することはありません。

### 顧客のいないインボイスとアプリケーションの状態

インボイスAPIは、Suprnovaのトランザクションミラーが要求する顧客の識別情報を提供しません。このアダプターは `WebhookHandler::mirrors_payment_transactions` から `false` を返します。返金を含むすべての検証済みイベントは監査ログに残りますが、顧客ミラーもトランザクションミラーも作成しません。StripeとPaddleは、既定のミラー動作を維持します。

汎用のwebhookルートは、プロバイダーのイベントを記録して処理しますが、アプリケーションの注文を履行はしません。アプリケーションの照合ジョブは、検証済みのレシートを消費し、現在の支払いステータスを読み取り、冪等なトランザクションの中で自分の注文を更新するべきです。リトライや順序の入れ替わった配信が二重にアクセスを付与できないよう、プロバイダーと支払い、または注文ごとに一意の履行キーを保存してください。ジョブ自身の永続的な完了状態は、フレームワークの `processed_at` とは別に管理してください。遅れて届いたイベントには現在の認証済み状態を使用し、返金はアプリケーションのポリシーに従って処理してください。

## 能力の限界

`CustomerStore` と `Subscription` の操作は `NotSupported` を返します。`as_payment()` と `as_promotions()` は `None` を返します。このアダプターは、カストディ残高、継続課金、入金アドレスによる直接チェックアウト、送金、返金の開始、プロバイダー側の顧客管理を実装しません。NOWPaymentsは、これらの操作の一部について別個の製品を提供しています。このインボイスアダプターは、それらを実装しているふりをしません。

### Suprnovaが異なる設計を選んだ理由

共通のプロバイダートレイトは引き続き利用できますが、サポートされない操作は明示的に失敗します。プロバイダー側の顧客が存在しないインボイスは、でっち上げた顧客レコードやサブスクリプションではなく、監査された通知とアプリケーションが所有する注文を使用します。

## 検証

ソースチェックアウトから、アダプターのテストと、共有の支払いリグレッションを実行してください：

```sh
CARGO_INCREMENTAL=0 cargo nextest run -p suprnova-payments-nowpayments
CARGO_INCREMENTAL=0 cargo nextest run -p suprnova --test payments
```

ローカルのスイートは、偽のHTTPレスポンス、独立して生成されたJavaScriptの署名フィクスチャ、そしてSQLiteを用いた本物のフレームワークのイングレスを使用します。これは、成功するチェックアウト、認証の失敗、不正または大きなレスポンス、タイムアウト、信頼できないリダイレクト、不正な署名、ステータスのマッピング、重複配信、そしてデータベース障害からの復旧を対象とします。ローカルのフィクスチャは、プロバイダーとの相互運用性を立証するものではありません。

本番を有効化する前に、NOWPaymentsのサンドボックスアカウントと、到達可能なHTTPSコールバックを使用してください。インボイスを作成し、プロバイダーが提供する成功、部分、失敗の各ケースを試し、その署名が受け入れられることを確認し、支払いIDを保存されたインボイスおよび注文と照合してください。配信をリプレイし、アプリケーションの履行キーが二度目の付与を防ぐことを確認してください。ローカルのテストは、実アカウントでのサンドボックス実行や本番実行を含意しません。

プロバイダーの参考資料：[APIエンドポイント](https://nowpayments.zendesk.com/hc/en-us/articles/21345824322717-API-and-endpoint-description)、[IPN認証](https://nowpayments.zendesk.com/hc/en-us/articles/21395546303389-IPN-and-how-to-setup)、そして [公式SDK](https://github.com/NowPaymentsIO/nowpayments-sdk-nodejs)。

## 次のステップ

リダイレクトペイロードのレンダリングについては [フロントエンド 統合](payments-frontend.md) を、共有される契約については [プロバイダー ガイド](payments-provider-guide.md) を参照してください。
