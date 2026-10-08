# メール

Suprnovaのメールサブシステムは、Laravelの `Mail::to(...)->send(...)` APIをTokio上に反映します。1つの `Mail` ファサード、9つのトランスポート（log、in-memory、開発/テスト向けの`.eml`ファイルプレビュー、SMTP、そして5つのHTTPプロバイダー - Postmark、SES、SendGrid、Mailgun、Resend）、MailableのシリアライズされたフィールドをコンテキストとするTeraでレンダリングされるテンプレート、永続的な少なくとも1回のエンベロープの上に乗るキュー + 遅延配信、そして `Bus::fake()` や `Cache::fake()` と同じ生地から仕立てられた `Mail::fake()` のテストガードです。

## クイックスタート

```rust
use serde::{Deserialize, Serialize};
use suprnova::async_trait;
use suprnova::mail::{Address, Mail, Mailable};

#[derive(Serialize, Deserialize)]
struct Welcome {
    name: String,
}

#[async_trait]
impl Mailable for Welcome {
    fn mailable_name() -> &'static str { "Welcome" }
    fn subject(&self) -> String { format!("Welcome, {}", self.name) }
    fn text_template_source(&self) -> Option<String> {
        Some("Hi {{ name }}, welcome aboard.".into())
    }
    fn from(&self) -> Option<Address> {
        Some(Address::new("hello@example.com").with_name("Suprnova"))
    }
}

async fn greet(name: String) -> Result<(), suprnova::FrameworkError> {
    Mail::to("alice@example.org")
        .send(Welcome { name })
        .await
}
```

Mailableは、JSONへシリアライズされ、それがテンプレートのTeraコンテキストになります。すべての `pub` フィールドは `{{ field_name }}` として到達可能です。

## 設定

`Server::serve` は、起動時に一度 `suprnova::mail::boot::bootstrap_from_env()` を呼びます。これは `MAIL_DRIVER` を読み取り、一致するトランスポートを束縛します。未設定の場合は `log` ドライバーがデフォルトです。

| `MAIL_DRIVER` | 振る舞い |
|---------------|----------|
| `log`         | 送信ごとに `tracing::info!` を発する - Laravelと同様、エンベロープと本文全体を - そして破棄します。本番環境の外ではデフォルトです。 |
| `memory`      | すべてのメッセージをプロセス内でキャプチャします。`suprnova::mail::boot::captured_in_memory()` を参照してください。 |
| `file`        | 送信ごとに1つのRFC 5322 `.eml` を `MAIL_FILE_PATH`（デフォルトは `storage/mail`）へ書き込み、その後破棄します。メールクライアントでファイルを開けば、レンダリング、ヘッダー、添付ファイルを確認できます。 |
| `smtp`        | SMTPサーバーへ接続します（認証情報が設定されていればSTARTTLS、そうでなければ平文のTCP）。 |
| `postmark`    | PostmarkのJSONを `/email` エンドポイントへPOSTします。 |
| `ses`         | SigV4で署名されたリクエストをAmazon SESの `SendEmail` へPOSTします。 |
| `sendgrid`    | SendGridのJSONを `/v3/mail/send` へPOSTします。 |
| `mailgun`     | Mailgunの `/v3/{domain}/messages` へ、`application/x-www-form-urlencoded`（添付ファイルがあれば `multipart/form-data`）をPOSTします。 |
| `resend`      | ResendのJSONを `/emails` へPOSTします。 |

### 本番環境は、メールを破棄するドライバーに対してフェイルクローズします

`log`、`memory`、`file` はメッセージをレンダリングして捨てます。`APP_ENV=production` の下では、起動はこれらのいずれに対しても**拒否**されます - そして、`MAIL_DRIVER` が未設定であるか、ビルドが認識しない値である場合も同様に拒否されます。どちらも、同じ `log` トランスポートへ行き着くからです:

```
refusing to boot in production: MAIL_DRIVER is unset, which defaults to the `log`
transport. Password resets and email verifications would report success while
nothing is delivered. Set MAIL_DRIVER to a delivering driver (smtp | postmark |
ses | sendgrid | mailgun | resend), or set
MAIL_ALLOW_NON_DELIVERING_IN_PRODUCTION=true to acknowledge that outgoing mail is
intentionally discarded.
```

これが防ぐ失敗は、サイレントなものです: 以前のデフォルトでは、`MAIL_DRIVER` を書き忘れた - あるいは大文字小文字を間違えて `MAIL_DRIVER=SMTP` と書いた - デプロイは、プロセスから何一つ出ていかないまま、すべてのパスワードリセットを送信済みとして報告し、ユーザーがロックアウトされるまで誰も気づきませんでした。

本番のデプロイが本当に、送信メールなしを望むなら（読み取り専用のミラー、ダークローンチ）、それを明示的に了承してください:

```env
MAIL_ALLOW_NON_DELIVERING_IN_PRODUCTION=true
```

`1`、`true`、`yes`、`on` だけが同意として数えられます - `=false` やタイプミスは、ガードを作動させたままにします。オーバーライドが設定されている場合、起動のたびに、送信メールが配信されないという警告が出ます。

本番環境の外では何も変わりません: `local`、`development`、`testing`、`staging` は、`log` のデフォルトを保ち、未知のドライバーに対する警告してフォールバックする振る舞いも保ちます。

### 本番環境は、暗号化されていないSMTP接続に対してフェイルクローズします

配信するかどうかではなく、接続がどう保護されるかに適用される、同じルールです。本番環境の `MAIL_DRIVER=smtp` は、暗号化されたトランスポートへ解決されなければならず、そうでなければ起動は失敗します。

`MAIL_SMTP_ENCRYPTION` は `starttls`、`tls`、`none` のいずれかを取ります（`ssl` と `null` は、Laravel互換のエイリアスとして受け付けられます）。未設定のままなら、認証情報から導かれます:

| `MAIL_SMTP_USER` / `MAIL_SMTP_PASS` | 解決される先 | 理由 |
|---|---|---|
| 両方とも設定 | `starttls` | 認証情報は、submissionポート上の本物のリレーを示唆します。 |
| どちらも未設定 | `none` | ローカルキャッチャーの経路です。Mailpit、MailHog、maildevは、1025で未認証のまま待ち受け、TLSを話しません。 |

そのため、新しくスキャフォルドされたものは、設定ゼロのまま動き続けます。そして、認証情報を一度も配線しなかった本番のデプロイは、黙って平文で送信するのではなく、停止します。465での暗黙のTLSを期待するリレーには `MAIL_SMTP_ENCRYPTION=tls` を設定してください - これは、トランスポートが常にサポートしてきたものの、以前は環境変数のどんな組み合わせでも到達できなかったモードです。

認識されない値は、本番環境だけでなく*あらゆる*環境で起動を失敗させます。`MAIL_SMTP_ENCRYPTION=tsl` は、暗号化するモードの文字の入れ替わりです。そのため、それを黙って「暗号化なし」として扱ってしまうことは、この変数がまさに防ぐために存在している、その失敗そのものです - デプロイでではなく、開発者のマシン上で失敗するほうがましです。

エスケープハッチは、上記のものを反映しています:

```env
MAIL_ALLOW_INSECURE_SMTP_IN_PRODUCTION=true
```

正当化できるのは、リレーがプライベートネットワーク経由でのみ到達可能な場合だけです - サイドカーや、VPC内のPostfixなど。それ以外のものでは、平文のSMTPは、認証情報とすべてのパスワードリセットリンクをネットワーク上に晒し、経路を盗聴している誰にとっても、それは残り続けます。

### `log` ドライバーはメッセージ全体をログに出力します

Laravelの `log` メーラーと同じです: エンベロープ*と*、レンダリングされた本文の両方です。

```
mail (log driver): would send from=noreply@app.test to=["alice@example.org"]
  subject=Reset your password
  text=Reset your password: https://app.test/password/reset?token=9f3a…&signature=…
  html=<a href="https://app.test/password/reset?token=9f3a…&signature=…">Reset</a>
```

そのリンクこそが要点です。開発中は、アプリがちょうど「送信した」確認リンクやパスワードリセットリンクを読む場所がコンソールです。それを隠すドライバーは、誰も使えないドライバーです。

ここで安全なのは、このドライバーが本番環境に到達できないからです - `APP_ENV=production` の下では、`MAIL_DRIVER=log` での起動は拒否されます（上記参照）。本文は、開発者のマシン上にしか存在しません。

デプロイされた環境で `log` ドライバーを動かすために `MAIL_ALLOW_NON_DELIVERING_IN_PRODUCTION=true` を設定するなら、あなたは1回限りのbearerリンクをログに置くことを選んでいます。それらのファイルを読める者 - 運用担当者、ログシッパー、保持用バケット、アグリゲーター - は誰でもそれらを使えます。リンクの有効期限は助けになりません。ログの転送は、人が受信箱を読むより速いからです。それを踏まえて保持期間とアクセスポリシーを決めるか、出力しないドライバーを使ってください:

```env
# プロセス内でのキャプチャ - suprnova::mail::boot::captured_in_memory()、
# またはテストでは Mail::fake()
MAIL_DRIVER=memory

# ログ行の代わりに、送信ごとに .eml を1つ書き出す - これが引き換えにする
# アクセス制御については、下の「`.eml`ファイルとしてメールをプレビューする」を参照
MAIL_DRIVER=file
MAIL_FILE_PATH=storage/mail

# あるいはローカルのメールキャッチャー（mailpit / maildev / mailhog）。
# 本物のメールをUIでレンダリングする
MAIL_DRIVER=smtp
MAIL_SMTP_HOST=127.0.0.1
MAIL_SMTP_PORT=1025
```

### ドライバーごとの環境変数

```env
# SMTP
MAIL_DRIVER=smtp
MAIL_SMTP_HOST=smtp.mailtrap.io
MAIL_SMTP_PORT=587
MAIL_SMTP_USER=...
MAIL_SMTP_PASS=...
MAIL_SMTP_ENCRYPTION=starttls   # あるいは、465での暗黙のTLSには `tls`、もしくは `none`

# Postmark
MAIL_DRIVER=postmark
MAIL_POSTMARK_TOKEN=...

# Amazon SES
MAIL_DRIVER=ses
MAIL_SES_ACCESS_KEY=...
MAIL_SES_SECRET_KEY=...
MAIL_SES_REGION=us-east-1

# SendGrid
MAIL_DRIVER=sendgrid
MAIL_SENDGRID_API_KEY=...

# Mailgun
MAIL_DRIVER=mailgun
MAIL_MAILGUN_API_KEY=...
MAIL_MAILGUN_DOMAIN=mg.example.com

# Resend
MAIL_DRIVER=resend
MAIL_RESEND_API_KEY=...
```

各HTTPプロバイダーは、リージョンのURLやモックサーバーを指す、対応する `MAIL_<PROVIDER>_ENDPOINT` のオーバーライドも尊重します（`wiremock` に対する統合テストに便利です）。

### 認証フローの送信者: `MAIL_FROM` と `MAIL_FROM_NAME`

組み込みの認証フローのMailable - メール確認、パスワードリセット、パスワード変更通知 - は、ハードコードされた `from()` ではなく、環境変数からエンベロープの `From` を解決します:

```env
MAIL_FROM=no-reply@example.com        # 素のアドレス（認証フローに必須。未設定ならフェイルクローズ）
MAIL_FROM_NAME=Acme Support           # 任意の表示名（0.5.9以降）
```

- `MAIL_FROM` は**素のアドレスでなければなりません。** そのままメッセージの `From` へ持ち上げられるため、`"Name <addr>"` という値は、アドレス全体として扱われてしまい、トランスポートに拒否されます。
- `MAIL_FROM_NAME`（任意、**0.5.9** で追加）は表示名を付加します。そのため、ヘッダーは `Acme Support <no-reply@example.com>` としてレンダリングされます。未設定または空欄なら、これまでどおりの素のアドレスの振る舞いを保ちます。送信時に読み取られるため、キューに入れられた認証フローのメールにも適用されます。

この2つの変数は、フレームワーク自身の認証フローのMailableにのみ影響します。あなた自身の `Mailable` は、`from()`（あるいはグローバルな `always_from` のデフォルト）を通じて送信者を設定します - 下記を参照してください。

## `.eml`ファイルとしてメールをプレビューする

`MAIL_DRIVER=log` はレンダリングされた本文をコンソールに出力します。プレーンテキストのメッセージには機能しますが、それ以外にはうまくいきません。`file` ドライバーは、SMTPが通信上に書き込むはずだったバイト列を書き込みます:

```
MAIL_DRIVER=file
MAIL_FILE_PATH=storage/mail
```

送信ごとに、そのディレクトリへ1つの `<millis>-<seq>.eml` が生成されます。任意のメールクライアント（Thunderbird、Apple Mail、`mutt -f`）で開けば、受信者から見たメッセージを確認できます - 2つの代替本文、すべての添付ファイル、そして `X-Priority`、`X-Tag`、`X-Metadata-*`、`Return-Path` を含む完全なヘッダーセットです。

ディレクトリは最初の送信時に作成されます。`MAIL_FILE_PATH` が未設定の場合、メールは `storage_path("mail")` に置かれます - 他の `storage/` 利用者が使うのと同じパス系統なので、サービスマネージャーが別の場所からプロセスを起動しても、ディレクトリはアプリケーションのベース内に留まります。絶対パスの `MAIL_FILE_PATH` は指定どおりに使われ、相対パスはアプリケーションのベースディレクトリ（`APP_BASE_PATH` で上書き可能な `base_path`）を基準にします。

### Suprnovaが異なる設計を選んだ理由

Laravelにはファイルメーラーがありません。その `log` メーラーは生のMIMEをログチャネルへ書き込むため、添付ファイルを再構成するには、MIME境界を探すためにログファイルをgrepすることになります。メッセージごとに実際の `.eml` を書き込めば、再構成ではなく成果物を開けます。トレードオフは、メールがディスク上に蓄積することです - このドライバーは決してプルーニングしないため、`MAIL_FILE_PATH` はスクラッチ領域として扱ってください。

### 各 `.eml` ファイルは使用可能な資格情報であり、それ自体では期限切れにならない

パスワードリセットとメール確認のメールには1回限りのbearerリンクが含まれ、`file` ドライバーはSMTPが送信するはずだったものとまったく同じように書き出します - ファイルを開ける誰もが読めます。`log` ドライバーのストリームと違い、これは永続ストレージです: `MAIL_FILE_PATH` をプルーニングするものはないため、1日目に書かれたトークンは100日目にもそこに残り、有効期限までは有効です。リセットリンクを保持するログファイルと同じアクセス扱いをディレクトリに与えてください - バージョン管理の対象外にし、デプロイファイルシステムを読める人を制限し、`file` が実トラフィックの近くで動く場合はスケジュールに従って消去します。

## Mailableトレイト

Mailableは、自分自身をレンダリングする方法を知っている、シリアライズ可能な構造体です。トレイトのデフォルトは、mailableのシリアライズされたフィールドに対して `tera::Tera::one_off` でレンダリングします:

```rust
use suprnova::async_trait;
use suprnova::mail::{Address, Attachment, Mailable};

#[async_trait]
impl Mailable for OrderShipped {
    fn mailable_name() -> &'static str { "OrderShipped" }
    fn subject(&self) -> String {
        format!("Order #{} shipped", self.order_id)
    }
    fn html_template_source(&self) -> Option<String> {
        Some("<p>Tracking: <code>{{ tracking }}</code></p>".into())
    }
    fn text_template_source(&self) -> Option<String> {
        Some("Tracking: {{ tracking }}".into())
    }
    fn from(&self) -> Option<Address> {
        Some(Address::new("orders@example.com").with_name("Acme Orders"))
    }
    fn attachments(&self) -> Vec<Attachment> {
        vec![Attachment::new("invoice.pdf", self.invoice_bytes.clone(), "application/pdf")]
    }
}
```

| メソッド | 必須? | 用途 |
|--------|-----------|---------|
| `mailable_name()` | はい | キューのエンベロープに永続化される安定した名前 - リネームすると、処理中のキュー投入済みメールが壊れます。 |
| `subject(&self)` | はい | 計算される件名。`subject_template_source` が `None` を返すとき、そのまま使われます。 |
| `subject_template_source(&self)` | 任意 | 件名のためのTeraテンプレート - `Some` のとき、`subject()` より優先され、`self` をコンテキストとしてレンダリングされます。本文のテンプレートソースと同じセマンティクスです。 |
| `html_template_source(&self)` | 任意 | HTML本文のTeraテンプレート。HTMLをスキップするなら `None` を返します。 |
| `text_template_source(&self)` | 任意 | プレーンテキスト本文のTeraテンプレート。テキストをスキップするなら `None` を返します。 |
| `from(&self)` | 任意 | グローバルなデフォルトの `noreply@localhost` を上書きします。 |
| `attachments(&self)` | 任意 | 添付するファイル。それぞれ `name + bytes + mime` です。 |
| `render_subject(&self)` / `render_html(&self)` / `render_text(&self)` | 任意 | Teraを迂回したいときに上書きします（Markdown → HTML、事前レンダリング済みのコンテンツ、カスタムの件名ロジックなど）。 |

`html_template_source` または `text_template_source` の少なくとも一方は `Some` を返さなければなりません（あるいは `render_html`/`render_text` がコンテンツを生成しなければなりません）。本文が空のmailableは、ディスパッチ時（`Mail::send`）とエンキュー時（`Mail::queue`）のどちらでも拒否されます。

### Teraのオートエスケープ

オートエスケープは**オフ**です。メールの本文は、典型的には手書きのHTMLであり、Teraの `<>&` エスケープは過剰エスケープになってしまうからです。テンプレート以外の理由で本文の文字列に `{{` が含まれる場合（例えば、Mustache構文を引用するマーケティングコピーなど）は、それをエスケープしてください: `{% raw %}{{ literal }}{% endraw %}`。

## メッセージを組み立てる

`Mail::to(...)` ビルダーは、受信者、CC/BCC、reply-to、そしてメッセージごとの送信者オーバーライドを、ディスパッチへ通します:

```rust
Mail::to("alice@example.org")
    .cc("manager@example.com")
    .bcc("audit@example.com")
    .reply_to("support@example.com")
    .from(("Operations", "ops@example.com"))   // （表示名、メールアドレス）
    .send(OrderShipped { order_id: 42, /* ... */ })
    .await?;
```

`Address` は `&str`、`String`、そして `(name, email)` のタプルを受け付けます。`Mail::to(...)` は、`Into<Address>` を実装するものなら何でも受け付けます。

## 添付ファイル

```rust
use suprnova::mail::Attachment;

let attachment = Attachment::new(
    "report.csv",
    csv_bytes,
    "text/csv",
);
```

添付ファイルは `Mailable::attachments` メソッドを介して運ばれます。5つのHTTPプロバイダーすべてがそれらを扱います - Postmark/SendGrid/ResendはJSON経由（base64エンコード）、SESはRaw MIME経由（`Content.Simple` は添付ファイルをサポートしないため）、そしてMailgunは `multipart/form-data` 経由です（添付ファイルがない場合はform-encodedの経路が使われます）。

## キューイング

`Mail::queue(...)` は `SendMailJob` を構築し、フレームワークのキューへプッシュします。ワーカーは、登録されたファクトリーからmailableを再構築し、束縛されたトランスポートを通じてディスパッチします:

```rust
// 1回だけ: ワーカーが目にするすべてのMailable型を登録する。
suprnova::mail::register_mailable_factory::<Welcome>()?;

// 送信時:
Mail::to("alice@example.org").queue(Welcome { name: "Alice".into() }).await?;

// 遅延:
use std::time::Duration;
Mail::to("alice@example.org")
    .later(Duration::from_secs(60), Welcome { name: "Alice".into() })
    .await?;
```

Mailのディスパッチを特定のキューまたは接続へルーティングするには `.on_queue(...)` / `.on_connection(...)` を使います。または、`Mailable::queue(&self)` を通じて、`Mailable` 自体にデフォルトを与えます:

```rust
Mail::to("alice@example.org")
    .on_queue("emails")
    .queue(Welcome { name: "Alice".into() })
    .await?;
```

`.on_queue(...)` は、`Mailable::queue()` と、メールディスパッチジョブに登録された `Queue::route` の両方より優先されます - `Queue::push_with` がどこでも適用するのと同じ「プッシュごとのオーバーライドが勝つ」ルールです。[キュー](queues.md#queue-routing)を参照してください。


同じ空本文のガードがキューの経路でも実行されるため、設定ミスのMailableは、エンベロープが作られる前の、プッシュ時点で拒否されます。

## テレメトリ

すべての送信は `suprnova::mail::dispatch_with_telemetry` を経由します。これは、次を運ぶ `mail.send` の `tracing::info_span!` を開きます:

- `transport` - ドライバー名（`"postmark"`、`"smtp"`、`"in-memory"`、…）
- `to_count`、`cc_count`、`bcc_count` - 受信者数
- `has_html`、`has_text` - 本文の形
- `attachment_count` - 添付ファイルの数
- `tag_count`、`metadata_count` - プロバイダーヒントの数
- `priority` - `1..=5`、未設定なら `0`

完了時、スパンは `duration_ms` とともに `mail sent`（info）または `mail send failed`（warn）を発します。同じラッパーが `Mail::send`、`SendMailJob` のキューワーカー、そして通知の `MailChannel` をカバーするため、メッセージがどう生成されたかにかかわらず、スパンのスキーマは同一です。

## `Mail::fake()` でテストする

`Mail::fake()` は、返されるRAIIガードの生存期間の間、インメモリのキャプチャ用トランスポートをインストールします。`Bus::fake()` / `Queue::fake()` / `Cache::fake()` を反映しています:

```rust
use suprnova::mail::Mail;

#[tokio::test]
async fn welcome_mail_is_sent_on_signup() {
    let fake = Mail::fake();

    sign_up("alice@example.org").await.unwrap();

    fake.assert_sent_count(1);
    fake.assert_sent(|m| m.to.iter().any(|a| a.email == "alice@example.org"));
    fake.assert_sent(|m| m.subject.starts_with("Welcome"));
    fake.assert_not_sent(|m| m.subject.contains("Password reset"));
}
```

ガードがドロップされると、以前に束縛されていたトランスポート（もしあれば）が復元されます。`Mail::fake()` と明示的なトランスポートの束縛を混在させるテストも、状態を漏らしません。

`Mail::fake()` は `Send + Sync` です。必要に応じて、awaitやスレッドをまたいで共有してください。

## カスタムトランスポート

`MailTransport` トレイトが統合ポイントです:

```rust
use suprnova::async_trait;
use suprnova::mail::{MailTransport, OutgoingMessage};
use suprnova::FrameworkError;

pub struct StdoutTransport;

#[async_trait]
impl MailTransport for StdoutTransport {
    async fn send(&self, msg: &OutgoingMessage) -> Result<(), FrameworkError> {
        println!("--- mail ---\n{}\n--- end ---", msg.subject);
        Ok(())
    }
    fn name(&self) -> &'static str { "stdout" }
}

// 起動時:
use std::sync::Arc;
suprnova::mail::Mail::set_transport(Arc::new(StdoutTransport))?;
```

トランスポートはTokioのランタイム上で実行されます - 非同期IO、コネクションプーリング、そして並行送信がファーストクラスです。リクエストごとのforkのペナルティはありません。

### Suprnovaが異なる設計を選んだ理由

LaravelのMailable層は、リクエストのライフサイクルの内側で同期的に実行されるSymfony Mailerの上に構築されています。Suprnovaの `MailTransport` は、`async fn send(&self, msg: &OutgoingMessage)` としてエンドツーエンドです: HTTPプロバイダーは `reqwest` を使い、SMTPの経路は非同期のlettreアダプターを使い、`dispatch_with_telemetry` はすべての送信をTokioの `tracing` スパンで包みます。長距離のプロバイダーはハンドラのスレッドをブロックせず、コネクションプールはリクエストをまたいで生き残り、1つのハンドラの中での並行送信は自明です - `tokio::try_join!(Mail::to(a).send(m), Mail::to(b).send(n))` は、期待どおりに動きます。

もう1つの相違点は、イベントのキャンセルです。Laravelは、`false` を返して送信を抑制できる `MessageSending` リスナーをモデル化しています（`events->until()`）。Suprnovaのディスパッチャーは、ショートサーキットの戻り経路を公開しません - `MessageSending` は観測専用です。送信をゲートするには、Mailable層で拒否する（`render_html` / `render_text` を上書きしてエラーを返す）か、`MailBuilder::send` の呼び出しをあなた自身のガードで包んでください。トレードオフは本物です: ディスパッチャーの契約をシンプルに保つために、Laravelのフックを1つ失っています。

もう1つの小さな相違点は、意図的な堅牢化です。Laravelは、本番環境で `MAIL_MAILER=log` を動かし続けることを甘んじて受け入れますが、Suprnovaは、明示的な了承なしにはそこで起動を拒否します。成功を報告しながら何も配信しないメールサブシステムは、誰も何週間も気づかない類の障害だからです。`log` ドライバー自体は、Laravelのものとまったく同じように振る舞います - メッセージ全体、本文とリンクを含めて - これが、開発において有用にしている理由であり、本番環境での起動拒否こそが、それを安全に保っているものです（[`log` ドライバーはメッセージ全体をログに出力します](#log-ドライバーはメッセージ全体をログに出力します)を参照）。

## ベストプラクティス

### ファクトリーは起動時に登録する。リクエストごとではなく

`Mail::queue` と `Mail::later` は、mailableの名前とJSONペイロードを運ぶ `SendMailJob` をプッシュします - ワーカーは `mailable_registry` を介して具体的な型を再構築します。キューに入れられるすべての `Mailable` を、`Server::serve` の時点で一度だけ登録してください:

```rust
// bootstrap.rs
pub fn register() -> Result<(), suprnova::FrameworkError> {
    suprnova::mail::register_mailable_factory::<WelcomeEmail>()?;
    suprnova::mail::register_mailable_factory::<PasswordReset>()?;
    suprnova::mail::register_mailable_factory::<InvoiceShipped>()?;
    Ok(())
}
```

登録されていないmailableに対する `Mail::queue` はキューに乗り、1回実行され、「unknown mailable」に行き当たり、エンベロープのバックオフポリシーに従ってリトライし、デッドレターになります - ファクトリーが起動時に束縛されていれば費やす必要のなかった、可観測性の時間を消費してしまいます。

### 遅い、または信頼できないレンダリングは、メールをキューに入れる

リクエストハンドラの中でメールを送信すると、ユーザーのレスポンスのレイテンシが、あなたのSMTPサーバー（あるいはどのプロバイダーのHTTP APIであれ）と結合してしまいます。同期的なローカル開発用のレンダリングを超えるものには `Mail::queue` を使い、ディスパッチを遅らせたいとき - オンボーディングのフォローアップ、リマインダーメール、スケジュールされたダイジェストなど - には `Mail::later` を使ってください。

```rust
// 悪い例: レスポンスタイムをメールプロバイダーに結びつけてしまう
Mail::to(&user.email).send(Welcome { ... }).await?;
return json_response!({ "ok": true });

// 良い例: 200 OKが即座に返る。メールはワーカーが配信する。
Mail::to(&user.email).queue(Welcome { ... }).await?;
return json_response!({ "ok": true });
```

### Mailableには必ず `from` を設定する

フレームワークのデフォルトの送信者は `noreply@localhost` です - 開発中に送信者の設定漏れを捕まえるのには便利ですが、本番環境でどのプロバイダーも受け入れる送信者ではありません。ディスパッチされるすべてのメッセージが本物の送信者アイデンティティを持つよう、`Mailable::from(&self)` を上書きするか（あるいは `NotificationMailable` の `#[mail(...)]` アトリビュートで `from = "..."` を設定してください）:

```rust
fn from(&self) -> Option<Address> {
    Some(Address::new("orders@example.com").with_name("Acme Orders"))
}
```

`MailBuilder` 上のメッセージごとのオーバーライド（`.from(("Operations", "ops@example.com"))`）は、mailableのデフォルトより優先されます - 単発のトランザクショナルな送信に便利です。

### 少なくとも1回の配信にはキューを使う。直接の経路ではなく

`MailBuilder::send` は最大1回です: トランスポートが2つのプロバイダーへのディスパッチの途中で失敗した場合、二重送信のリスクなしにリトライすることはできません。`MailBuilder::queue` は耐久性のある少なくとも1回の配信を使い、キューとコネクションのルーティングを公開しますが、べき等性キーを受け取りません。再配信されたメールジョブは2回送信することがあります。メッセージの重複を排除しなければならない場合は、`MailBuilder` がキーを受け取ると主張するのではなく、カスタムのキュージョブ内でアプリケーションレベルのべき等性ガードまたはプロバイダーがサポートするべき等性の仕組みを使ってください。

## 単発のメッセージ: `Mail::raw` と `Mail::html`

メールが、完全な `Mailable` 構造体を正当化するほどではない、単発のトランザクショナルなpingであるとき、2つのショートカットがボイラープレートを省きます:

```rust
use suprnova::mail::Mail;

// プレーンテキスト
Mail::raw("Your code is 12345", |b| {
    b.to("alice@example.org")
        .subject("Verification code")
        .from("auth@example.com")
}).await?;

// HTML
Mail::html("<p>Hello, <b>world</b></p>", |b| {
    b.to("alice@example.org")
        .subject("こんにちは")
        .from("hello@example.com")
}).await?;
```

クロージャは、本文があらかじめロードされた [`MailBuilder`] を受け取り、その上に受信者、件名、送信者、タグ、メタデータ、優先度、そしてその他あらゆる [`MailBuilder`] のフルーエントメソッドを重ねられるようにします。これらの経路は、`Mailable` トレイトを完全に迂回します - ワンショットのテストpingや、短いトランザクショナルなメモに便利です。

## グローバルデフォルト: `always_from`、`always_reply_to`、`always_to`、`always_return_path`

Laravelの `Mailer::alwaysFrom` / `alwaysReplyTo` / `alwaysTo` / `alwaysReturnPath` を反映して、Mailファサードは4つのグローバルなセッターを公開します:

```rust
use suprnova::mail::{Address, Mail};

// 起動時:
Mail::always_from(Address::new("noreply@example.com").with_name("Acme"))?;
Mail::always_reply_to(Address::new("support@example.com"))?;
Mail::always_return_path(Address::new("bounce@example.com"))?;

// ローカル開発の「単一受信箱」 - すべてのメールを1つのアドレスへルーティングし、CC/BCCを落とす:
Mail::always_to(Address::new("dev-inbox@example.com"))?;

// すべてを元に戻す（テストは典型的にはこれをteardownで呼ぶ）:
Mail::forget_always()?;
```

優先順位は控えめです - デフォルトは、ディスパッチされるメッセージに明示的な値がないときにだけ適用されます:

| フィールド | デフォルトが適用されるとき |
|-------|---------------------|
| `always_from` | メッセージの `from` がフレームワークのデフォルトである `noreply@localhost` のとき |
| `always_reply_to` | メッセージに明示的な `reply_to` がないとき |
| `always_to` | 常に - すべてのメッセージをこのアドレスへルーティングし、CC/BCCをクリアします |
| `always_return_path` | メッセージに明示的な `return_path` がないとき |

同じ優先順位は、キューの経路にも適用されます: キューに入れられたmailableは、ワーカーのディスパッチ時に `apply_always_defaults` を通るため、直接の送信とキュー経由の送信は、同一のエンベロープの形に収束します。

## タグ、メタデータ、優先度、ヘッダー、Return-Path

送信されるすべてのメッセージは、Laravel形のプロバイダー向けヒントを運べます - タグ、メタデータのキー/値、RFC-2076の優先度、カスタムのMIMEヘッダー、そしてSender / bounce-toのアドレスです。これらは、HTTPプロバイダーのネイティブなフィールド（Postmarkの `Tag` / `Metadata` / `Headers`、SESの `EmailTags` と `Content.Simple.Headers`、SendGridの `categories` / `custom_args` / `headers`、Mailgunの `o:tag` / `v:` / `h:`、Resendの `tags` / `headers`）へ転送され、SMTPへはRFC 5322のヘッダーとして転送されます。

SESに限っては、ヘッダーはメッセージが使うほうの内容の形に乗ります: 素のメッセージなら `Content.Simple.Headers`、添付ファイルのあるメッセージ（SESはこれを生のMIMEとしてしか受け付けません）なら実際のMIMEのヘッダー行です。ヘッダー名は、メッセージが最終的にどちらの形を使うことになっても、同じ方法で検証されます - CR、LF、NULは拒否され（呼び出し元が与えた文字列が2つ目のヘッダーに化けるのは、まさにそれが理由です）、空の名前、76バイトを超える名前、非ASCIIのバイト、名前の中の `:` や空白も同様に拒否されます。これは、生のMIMEのビルダー自身が要求するものと一致します。2回以上繰り返されたヘッダー名は、素のメッセージの経路ではすべての値を保ちますが、添付ファイルの経路では最後の値だけを保ちます - SMTPが持つのと同じ制限です。

それらを付ける方法は2つあります - 型ごとのデフォルトのためのMailableのレベルか、ビルダー上のメッセージごとかです:

```rust
use suprnova::async_trait;
use suprnova::mail::{Mailable, PRIORITY_HIGH};
use std::collections::BTreeMap;

#[async_trait]
impl Mailable for OrderShipped {
    fn mailable_name() -> &'static str { "OrderShipped" }
    fn subject(&self) -> String { format!("Order #{} shipped", self.order_id) }
    fn text_template_source(&self) -> Option<String> { Some("...".into()) }

    fn tags(&self) -> Vec<String> { vec!["transactional".into(), "order".into()] }
    fn metadata(&self) -> BTreeMap<String, String> {
        let mut m = BTreeMap::new();
        m.insert("order_id".into(), self.order_id.to_string());
        m
    }
    fn priority(&self) -> Option<u8> { Some(PRIORITY_HIGH) }
    fn headers(&self) -> Vec<(String, String)> {
        vec![("X-Origin".into(), "warehouse".into())]
    }
}
```

```rust
// ビルダー上のメッセージごと。メタデータのキーが衝突したときはビルダーが勝つ。タグとヘッダーは和集合。
Mail::to(&user.email)
    .tag("campaign-spring")
    .metadata("ab_variant", "B")
    .priority(1)
    .header("X-Source", "promo-feed")
    .return_path("bounce@example.com")
    .send(WelcomeEmail { name: user.name.clone() })
    .await?;
```

5つの優先度レベルの定数は、`suprnova::mail::{PRIORITY_HIGHEST, PRIORITY_HIGH, PRIORITY_NORMAL, PRIORITY_LOW, PRIORITY_LOWEST}` にあります - Laravelが使うのと同じ `1..=5` の整数のスケールです。

### SES送信オプション

Amazon SES v2の `SendEmail` は、メッセージ自体に加えて3つのオプションを受け取ります。これらをトランスポートに固定するか、ヘッダーでメッセージごとに上書きしてください:

```rust
use suprnova::mail::ses::SesMailTransport;

let transport = SesMailTransport::new(key, secret, "us-east-1")
    .tenant_name("acme")                                  // TenantName
    .configuration_set_name("transactional")              // ConfigurationSetName
    .list_management("newsletter", Some("weekly"));       // ListManagementOptions
```

| メッセージ上のヘッダー | SESフィールド | 形 |
|---|---|---|
| `X-SES-TENANT-NAME` | `TenantName` | テナント名 |
| `X-SES-CONFIGURATION-SET` | `ConfigurationSetName` | コンフィギュレーションセット名 |
| `X-SES-LIST-MANAGEMENT-OPTIONS` | `ListManagementOptions` | `my-list`、`contactListName=my-list`、または `my-list; topicName=weekly` |

ヘッダーは常にトランスポートのデフォルトに勝つため、1つのマルチテナントトランスポートとメッセージごとのヘッダーで一般的なケースをカバーできます:

```rust
Mail::to(&user.email)
    .header("X-SES-TENANT-NAME", &tenant.slug)
    .send(WelcomeMail { name: user.name.clone() })
    .await?;
```

これらのヘッダーはメッセージの内容ではなくトランスポートへの指示です: リクエストの構築時に消費され、受信者へ届くMIMEへレンダリングされることはありません。

### Suprnovaが異なる設計を選んだ理由

Laravelはメッセージから `X-SES-TENANT-NAME` と `X-SES-LIST-MANAGEMENT-OPTIONS` を読み取りますが、`ConfigurationSetName` はトランスポートのオプション配列を通じてのみ公開します。そのため、メッセージごとにコンフィギュレーションセットを切り替えるには2つ目のトランスポートが必要です。Suprnovaは3つすべてに同じ2つのソースを与え、`X-SES-CONFIGURATION-SET` ヘッダーを追加します。ヘッダーがトランスポートに勝つ優先順位は、メッセージ由来のオプションが設定済みのものへマージされるLaravelの挙動と一致します。

## キャプチャされたメッセージを検査する

`OutgoingMessage` は、Laravelスタイルの検査用ヘルパーを運びます - テストのアサーションと、実行時の監査ログの両方に便利です:

```rust
fn audit_outgoing(m: &suprnova::mail::OutgoingMessage) {
    if m.has_tag("transactional") && m.has_to("alice@example.org") { /* ... */ }
    if m.has_metadata("order_id") { /* ... */ }
    if m.has_subject("Welcome") { /* ... */ }
    if m.has_attachment("invoice.pdf") { /* ... */ }
    if m.has_header("X-Source", "promo-feed") { /* ... */ }
}
```

受信者のチェックは、メールアドレスについて大文字小文字を区別しません。メタデータ、タグ、件名、添付ファイル名のチェックは、完全一致です。

## テストフェイク: 拡張された表面

`Mail::fake()` は、送信済みとキュー投入済みの*両方の*経路をカバーします。（`MailBuilder::send` 経由の）送信済みメールはインメモリのトランスポートに乗り、（`.queue` / `.later` 経由の）キュー投入済みメールはフェイクのキューバッファに乗ります。

```rust
use suprnova::mail::Mail;

#[tokio::test]
async fn boot_dispatches_welcome() {
    let fake = Mail::fake();

    onboard_user("alice@example.org").await.unwrap();

    // 送信側
    fake.assert_sent_count(1);
    fake.assert_sent(|m| m.has_to("alice@example.org") && m.subject.starts_with("Welcome"));
    fake.assert_sent_to("alice@example.org");
    fake.assert_not_sent(|m| m.subject.contains("Password reset"));

    // キュー投入側（遅延メール用）
    fake.assert_queued("WelcomeFollowup");
    fake.assert_queued_to("alice@example.org");
    fake.assert_queued_count(1);

    // 複合
    fake.assert_outgoing_count(2);   // 送信済み + キュー投入済み
    fake.assert_not_outgoing("PasswordReset");
}
```

追加のヘルパー:

| ヘルパー | 用途 |
|--------|---------|
| `fake.captured()` | すべての送信済みメッセージ |
| `fake.count()` | 送信済みの件数 |
| `fake.queued()` | すべてのキュー投入済み `QueuedSnapshot` |
| `fake.queued_count()` | キュー投入済みの件数 |
| `fake.outgoing_count()` | 送信済み + キュー投入済み |
| `fake.sent(predicate)` | 述語で送信済みを絞り込む |
| `fake.sent_to(email)` | 受信者で送信済みを絞り込む |
| `fake.queued_named(name)` | 指定した名前のキュー投入済みmailable |
| `fake.queued_to(email)` | 受信者へのキュー投入済みmailable |
| `fake.assert_sent_count(n)` | 送信済みの件数の厳密一致 |
| `fake.assert_queued_count(n)` | キュー投入済みの件数の厳密一致 |
| `fake.assert_outgoing_count(n)` | 合計の厳密一致 |
| `fake.assert_nothing_sent()` | 送信済みバッファが空 |
| `fake.assert_nothing_queued()` | キュー投入済みバッファが空 |
| `fake.assert_nothing_outgoing()` | 両方とも空 |
| `fake.assert_sent_to(email)` | 受信者への送信済みが少なくとも1件 |
| `fake.assert_not_sent_to(email)` | 受信者への送信済みが1件もない |
| `fake.assert_queued(name)` | 指定した名前のキュー投入済みが少なくとも1件 |
| `fake.assert_queued_with(name, fn)` | 述語に一致する、指定した名前のキュー投入済みが少なくとも1件 |
| `fake.assert_queued_to(email)` | 受信者へのキュー投入済みが少なくとも1件 |
| `fake.assert_not_queued(name)` | 指定した名前のキュー投入済みが1件もない |

`QueuedSnapshot::decode::<M>()` は、ペイロードを具体的な `M` へ逆シリアライズして戻すため、型チェックされた述語が、あつらえのデコード用ボイラープレートなしに機能します。

## イベント: `MessageSending` と `MessageSent`

成功したすべてのディスパッチは、2つのフレームワークイベントを発します:

- `MessageSending` - トランスポートの呼び出しの*直前*です。リスナーは、メッセージの形（受信者、件名、タグ、本文の形のフラグ）を観測します。
- `MessageSent` - トランスポートの呼び出しが成功した*直後*です。リスナーは同じ形を観測します。失敗した送信は、このイベントを発しません。

```rust
use std::sync::Arc;
use suprnova::events::EventFacade;
use suprnova::mail::MessageSent;

EventFacade::listen::<MessageSent, _>(Arc::new(MyAuditListener)).await;
```

どちらのイベントも観測専用です - ディスパッチャーは、Laravelスタイルのキャンセルチャネルをモデル化していません。ゲーティングの回避策については、上記の[Suprnovaが異なる設計を選んだ理由](#suprnovaが異なる設計を選んだ理由)を参照してください。

## 複数受信者向けの便利機能: `Mail::cc` と `Mail::bcc`

Mailファサードは、すべて新しい `MailBuilder` を返す3つのエントリーポイント - `to`、`cc`、`bcc` - を公開します。主なルーティングの意図に一致するものを使ってください:

```rust
// メッセージが主に監査用のコピーであるときは、cc / bcc から始める。
Mail::cc("manager@example.com")
    .to("alice@example.org")
    .send(OrderShipped { /* ... */ })
    .await?;
```

どのエントリーポイントから始めても、同じフルーエントな表面が適用されます。

### 束縛されたトランスポートではなく、`Mail::fake()` に対してテストする

`Mail::fake()` は、RAIIガードの生存期間の間、プロセスローカルなキャプチャ用トランスポートをインストールし、以前に束縛されていたものを復元します。それを使うテストは、出入りのたびにグローバルをクリアする必要はありません - drop のセマンティクスがそれを扱います。トランスポートのグローバルを変更するテストには、`#[serial_test::serial]` と `Mail::fake()` を組み合わせてください。そうしなければ、並行実行されるテストが互いを踏みつぶしてしまいます。

## 次のステップ

- [通知](notifications.md) - `Notify::send` は、メール、データベース、webpushの各チャネルへファンアウトします。`#[derive(NotificationMailable)]` は、`Mailable` トレイトの上のマクロ駆動のショートカットです
- [キュー](queues.md) - `Mail::queue` と `Mail::later` が乗る、永続的なエンベロープ
- [イベント](events.md) - `MessageSending` / `MessageSent` のリスンと、より広いディスパッチャーモデル
- [テスト](testing.md) - 他の `*::fake()` ガードと並ぶ `Mail::fake()`
- [設定](configuration.md) - サービスの認証情報のための、型付き設定の登録

## リファレンス

- トレイト: `suprnova::mail::Mailable`
- ファサード: `suprnova::mail::Mail`
- Bootstrap: `suprnova::mail::boot::bootstrap_from_env()`
- トランスポート: `LogMailTransport`、`InMemoryMailTransport`、`FileMailTransport`、`SmtpMailTransport`、`PostmarkMailTransport`、`SesMailTransport`、`SendGridMailTransport`、`MailgunMailTransport`、`ResendMailTransport`
- キュージョブ: `suprnova::mail::SendMailJob`
- テストガード: `suprnova::mail::MailFake`
- テレメトリヘルパー: `suprnova::mail::dispatch_with_telemetry`
