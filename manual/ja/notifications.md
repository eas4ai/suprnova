# 通知

通知とは、1つの呼び出し箇所から、1つ以上のチャネル - メール、アプリ内のインボックス、ブラウザプッシュ、リアルタイムのWebSocket - を通じて、ユーザー（あるいは「メールアドレスを持つ誰でも」）に届けたい小さなメッセージです。あなたは `Notify::send(&user, &OrderShipped { … })` と書きます。ディスパッチャーは、その1つの通知を、その通知が宣言したすべてのチャネルへファンアウトし、それぞれを受信者を通じて宛先指定します。

*何が*（注文が発送された、請求書が支払われた）起きたかが、*どうやって*（どのトランスポートが結局配信したか）よりもあなたのコードにとって重要なときに、通知を使ってください。生のトランスポートへのアクセス - カスタムのメール本文を組み立てる、特定のブロードキャストチャネルへ発行する、一度限りのweb プッシュを送る、といったことのためには、[メール](mail.md)、[ブロードキャスト](broadcasting.md)、または[web プッシュ](web-push.md)を直接経由してください。

## クイックスタート

```rust
use serde::{Deserialize, Serialize};
use suprnova::FrameworkError;
use suprnova::NotificationMailable;          // deriveマクロ
use suprnova::notifications::channels::mail::MailRendering;
use suprnova::{Notifiable, Notification, Notify};

#[derive(Serialize, Deserialize, NotificationMailable)]
#[mail(
    subject = "Order shipped - tracking {{ tracking }}",
    html    = "<p>Your order is on its way.</p><p>Tracking: <code>{{ tracking }}</code></p>",
    text    = "Tracking: {{ tracking }}",
    from    = "orders@example.com",
    from_name = "Acme Orders",
)]
pub struct OrderShipped {
    pub tracking: String,
}

impl Notification for OrderShipped {
    fn notification_name() -> &'static str { "OrderShipped" }
    fn channels(&self) -> Vec<&'static str> { vec!["mail", "database"] }
    fn data(&self) -> serde_json::Value {
        serde_json::json!({ "tracking": self.tracking })
    }
}

struct User { id: i64, email: String }
impl Notifiable for User {
    fn route_for(&self, channel: &str) -> Option<String> {
        match channel {
            "mail"     => Some(self.email.clone()),
            "database" => Some(self.id.to_string()),
            _          => None,
        }
    }
}

async fn ship(user: &User, tracking: String) -> Result<(), FrameworkError> {
    Notify::send(user, &OrderShipped { tracking }).await
}
```

`Notify::send` は、1回の呼び出しで、メールチャネルとデータベースチャネルの両方へディスパッチします。受信者は、`route_for` から `None` を返すことでチャネルを辞退します - 「メールのみ」や「プッシュのみ」のユーザーに便利です。

## 3つのトレイト

| トレイト | 何を表すか | 実装するもの |
|---|---|---|
| `Notification` | 型付きのメッセージ + それがディスパッチするチャネル | あなたの通知の構造体 |
| `Notifiable` | 受信者 - チャネルごとの `route_for` を公開する | あなたの `User`、`Order`、あて先指定可能な何でも |
| `Channel` | トランスポート - ルートへの配信方法を知っている | 組み込み: `MailChannel`、`DatabaseChannel`、`BroadcastChannel`、`WebPushChannel` |

### `Notifiable`

```rust
pub trait Notifiable: Send + Sync {
    fn route_for(&self, channel: &str) -> Option<String>;
}
```

受信者は、チャネルごとの宛先指定を所有します。`route_for("mail")` はメールアドレスを返します。`route_for("database")` はエンティティのidを文字列として返します。`route_for("webpush")` はシリアライズされた `SubscriptionInfo` のJSONを返します。`route_for("broadcast")` はブロードキャストのチャネル名を返します。この受信者に対してチャネルをスキップするには `None` を返してください。

### `Notification`

```rust
pub trait Notification: Serialize + DeserializeOwned + Send + Sync + 'static {
    fn notification_name() -> &'static str where Self: Sized;
    fn channels(&self) -> Vec<&'static str>;
    fn data(&self) -> serde_json::Value;

    fn should_send(&self, _channel: &str) -> bool { true }
    fn after_sending(&self, _channel: &str) -> Result<(), FrameworkError> { Ok(()) }
    fn queue(&self) -> Option<&'static str> { None }
    fn timeout(&self) -> Option<std::time::Duration> { None }
    fn fail_on_timeout(&self) -> bool { false }
    fn max_tries(&self) -> u32 { 3 }
    fn backoff(&self) -> BackoffSchedule { BackoffSchedule::default() }
}
```

| メソッド | 目的 |
|---|---|
| `notification_name()` | データベースチャネルによって永続化され、キューのエンベロープのキーとして使われ、メールレンダラーレジストリのルックアップキーとなる、安定した識別子。 |
| `channels(&self)` | この通知がディスパッチするチャネル名。順序は反復順。 |
| `data(&self)` | チャネルが配信/永続化する、JSONシリアライズ可能なペイロード。通常は、チャネルが必要とするフィールドの部分集合の `serde_json::to_value(self)`。 |
| `should_send(&self, channel)` | 同期パスとキューに入れられたパスの両方で参照される、チャネルごとの拒否権。`false` を返すと、このディスパッチに対してそのチャネルをスキップする。デフォルト: 常に送る。 |
| `after_sending(&self, channel)` | 同期パスとキューに入れられたパスの両方で、完了した各チャネルについて一度呼び出される、成功後のフック。`Err` を返すと、チャネルのエラーと同じ方法で伝播する。デフォルト: no-op。 |
| `queue(&self)` | この通知の `Notify::queue` ディスパッチが解決するキュー。デフォルト: `None`（ドライバーのデフォルト、または登録済みなら `Queue::route`）。[キュー調整](#キュー調整)を参照してください。 |
| `timeout(&self)` | この通知のキューに入れられたジョブに対する試行ごとのタイムアウト。デフォルト: `None`（タイムアウトなし）。 |
| `fail_on_timeout(&self)` | `true` の場合、タイムアウトは永続的な失敗です（デッドレター、リトライなし）。デフォルト: `false`。 |
| `max_tries(&self)` | この通知のキューに入れられたジョブの最大試行回数。デフォルト: `3`。 |
| `backoff(&self)` | この通知のキューに入れられたジョブのバックオフスケジュール。デフォルト: フレームワークのデフォルト。 |

`should_send` と `after_sending` は、**両方**のパスで尊重されます。`Notify::send` はディスパッチャーの中でこれらを参照します。`Notify::queue` は、チャネルごとのジョブをそれぞれenqueueする前に `should_send` をチェックし、ワーカーは配信の前に `should_send` を再チェックし（状態はenqueueと実行の間に変わりうるため）、送信が成功した後に `after_sending` を実行します。3つのライフサイクル*イベント*（`NotificationSending` / `NotificationSent` / `NotificationFailed`）は、それでも同期パスでのみ発火します。

## チャネル

### メール

メールチャネルは、束縛済みのメールトランスポートを介して配信します（[メール](mail.md)を参照）。通知は、`NotificationMailable` を実装することでオプトインします:

```rust
pub trait NotificationMailable: Notification {
    fn to_mail(&self) -> Result<MailRendering, FrameworkError>;
}
```

`MailRendering` は、レンダリングのエンベロープです - `subject`（必須）、`html` および/または `text`（少なくとも1つが必須）、省略可能な `from`、`cc`、`bcc`、`reply_to`、`attachments` です。メールチャネルは、このレンダリングと受信者の `route_for("mail")` から送信メッセージを組み立て、設定済みの送信者デフォルト（`Mail::always_from(...)`、`always_to(...)` など）を適用し、`Mail::current_transport` を介してディスパッチします。

レンダラーが `html` も `text` も持たないレンダリングを返した場合、配信はフェイルファストします - 空の通知メールが無音で送られることは決してありません。

#### `#[derive(NotificationMailable)]`

このderiveは、通知ごとの `to_mail` の `impl` を、1つの `#[mail(...)]` アトリビュートへ折り畳みます。テンプレートは[Tera](https://keats.github.io/tera/)を使います。`self` のシリアライズされたフィールドがコンテキストになります。

```rust
#[derive(Serialize, Deserialize, NotificationMailable)]
#[mail(
    subject = "Welcome {{ name }}",
    html_template = "templates/welcome.html",
    text_template = "templates/welcome.txt",
    from = "hello@example.com",
    from_name = "Acme",
    cc = "ops@example.com, support@example.com",
)]
pub struct Welcome { pub name: String }
```

サポートされているキー:

| キー | 必須？ | 目的 |
|---|---|---|
| `subject` | はい | Teraテンプレート - `self` をコンテキストにしてレンダリングされる。 |
| `html` | dagger | インラインのHTML本文のTeraテンプレート。 |
| `html_template` | dagger | HTML本文のTeraテンプレートへのパス（`include_str!` を介して埋め込まれる）。 |
| `text` | dagger | インラインのプレーンテキスト本文のTeraテンプレート。 |
| `text_template` | dagger | プレーンテキスト本文のTeraテンプレートへのパス（`include_str!` を介して埋め込まれる）。 |
| `from` | いいえ | 送信者のメール - デフォルトの `noreply@localhost` を上書きする。 |
| `from_name` | いいえ | 表示名。`from` を要求する。 |
| `cc` | いいえ | カンマ区切りのCCリスト。空白と末尾のカンマは無視される。 |
| `bcc` | いいえ | カンマ区切りのBCCリスト。 |
| `reply_to` | いいえ | カンマ区切りのReply-Toリスト。 |

（dagger）少なくとも1つの本文のバリアントが存在しなければなりません。`html` と `html_template` は互いに排他的です。`text` と `text_template` も同様です。

あらゆる不変条件は、コンパイル時に強制されます - `subject` の欠落、空の本文、競合するバリアント、`from` を伴わない `from_name`、あるいは未知のキーは、ディスパッチ時に失敗するのではなく、ビルドを失敗させます。

添付ファイル（バイナリのペイロード）や、インスタンスごとの動的な受信者のためには、`NotificationMailable` を手作業で実装し、`MailRendering` を直接構築してください。

### データベース

データベースチャネルは、各通知を `notifications` テーブルの1行として永続化します:

```rust
use std::sync::Arc;
use suprnova::{DatabaseChannel, NotificationDispatcher};

let dispatcher = NotificationDispatcher::new()
    .register_channel(Arc::new(DatabaseChannel::new(db, "users")));
```

2番目の引数は、受信者の多態的な型タグです（後でインボックスの行をクエリで取り戻せるように、`notifiable_type` に保存するものです）。受信者の `route_for("database")` が `notifiable_id` になります。このマイグレーションはフレームワークに同梱されています（`framework/migrations/20260516_create_notifications_table.sql`）。`suprnova migrate` を実行すれば、テーブルが現れます。

#### インボックスを読む

読み取り側のヘルパーは、`(notifiable_type, notifiable_id)` に対するフリー関数として、`suprnova::notifications` に存在します:

```rust
use suprnova::notifications::{
    all_for, unread_for, read_for,
    mark_as_read, mark_as_unread, mark_all_as_read,
    delete_for, StoredNotification,
};

let unread: Vec<StoredNotification> = unread_for(&db, "users", "42").await?;
let count = mark_all_as_read(&db, "users", "42").await?;
let removed = delete_for(&db, "users", "42").await?;
```

`StoredNotification` は、`id`、`type_name`（`Notification::notification_name`）、`notifiable_type`、`notifiable_id`、デコードされたJSONの `data`、`read_at`、`created_at`、`updated_at` を運びます。`mark_as_read` / `mark_as_unread` はべき等です（Laravelの契約と一致しています）。

### Web プッシュ

web プッシュチャネルは、ペイロードを暗号化し、フレームワークのVAPID署名クライアントを介して、保存済みのブラウザのプッシュ購読のエンドポイントへPOSTします:

```rust
use std::sync::Arc;
use suprnova::WebPushChannel;
use suprnova::web_push::{VapidKey, WebPushClient};

let client = WebPushClient::new(
    VapidKey::from_pem(b"-----BEGIN PRIVATE KEY-----\n…")?,
    "mailto:ops@example.com",
)?;
let push_channel = WebPushChannel::new(Arc::new(client), 86_400 /* TTL秒 */);
```

受信者の `route_for("webpush")` は、シリアライズされた `SubscriptionInfo` のJSONを返します（ブラウザが `PushSubscription.toJSON()` から返してくるのと同じ形です - そのまま保存し、そのまま返してください）。TTLはプッシュサービスへ転送されます。

プッシュサービスがチャネルに購読が失効したと伝えたとき（HTTP 404/410）、チャネルは構造化された `WARN` を記録し、成功を返します - その通知は、リトライすべき受信者がない終端状態に達したということです。オペレーターはログを見て、失効した購読を削除します。配信はエラーになりません。

完全なクライアントについては、[Web プッシュ](web-push.md)を参照してください。

### ブロードキャスト

ブロードキャストチャネルは、各通知をアプリケーションの `BroadcastHub` へ発行します。そのため、WebSocketの購読者はそれをリアルタイムで受け取ります。受信者の `route_for("broadcast")` がチャネル名であり、通知の型がイベントであり、`data()` がペイロードです:

```rust
use std::sync::Arc;
use suprnova::BroadcastChannel;
use suprnova::broadcasting::BroadcastHub;
use suprnova::container::App;

// 起動時に - あらゆるブロードキャストのディスパッチの前に、hubを束縛する。
App::bind::<dyn BroadcastHub>(Arc::clone(&hub));

let dispatcher = suprnova::NotificationDispatcher::new()
    .register_channel(Arc::new(BroadcastChannel::new()));
```

このチャネルは、配信の時点でコンテナからhubを解決します。通知が `"broadcast"` を宣言しているのに `BroadcastHub` が束縛されていない場合、チャネルはエラーを返します - 設定を誤ったアプリケーションは、メッセージを無音で落とすのではなく、その問題を表面化させます。生きた購読者がゼロのチャネルへ発行することはエラーではありません。

hubのセットアップとWebSocketの配線については、[ブロードキャスト](broadcasting.md)を参照してください。

## オンデマンドの通知

*データベースにいない誰か*に通知したいときがあります - メールアドレスへの一度限りの運用アラート、webhookの受信者、どのユーザーも所有していないブロードキャストチャネルなどです。`AnonymousNotifiable` は「行を持たないユーザー」です:

```rust
use suprnova::Notify;

let recipient = Notify::route("mail", "ops@example.com")?;
Notify::send(&recipient, &IncidentNotification { id: 7 }).await?;

// 1つのビルダーで複数のチャネル:
let recipient = Notify::routes([
    ("mail", "ops@example.com"),
    ("broadcast", "ops-channel"),
])?;
Notify::send(&recipient, &IncidentNotification { id: 7 }).await?;
```

`Notify::route("database", …)` と `Notify::routes([..., ("database", …)])` は `Err` を返します - データベースチャネルは、`(notifiable_type, notifiable_id)` のペアを永続化しますが、匿名の受信者はこれを与えられないからです。

## ディスパッチャー

`NotificationDispatcher` は、チャネルのレジストリを保持します。起動時に一度構築し、グローバルに束縛してください:

```rust
use std::sync::Arc;
use suprnova::{DatabaseChannel, MailChannel, NotificationDispatcher, WebPushChannel};
use suprnova::notifications::set_dispatcher;

let dispatcher = NotificationDispatcher::new()
    .register_channel(Arc::new(MailChannel::new()))
    .register_channel(Arc::new(DatabaseChannel::new(db, "users")))
    .register_channel(Arc::new(WebPushChannel::new(push_client, 86_400)));

set_dispatcher(Arc::new(dispatcher))?;
```

`register_channel` は、チャネル名に対して最後の書き込みが勝ちます - `"mail"` という名前の2つのチャネルを登録すると、無音で最初のものが置き換えられます。これにより、テストのセットアップが快適になります。

ディスパッチャーが登録していないチャネルを宣言する通知は、`WARN`（"no channel registered;skipping"）を記録し、次のチャネルへ進みます - ディスパッチは、未知のチャネル名でエラーにはなりません。

`set_dispatcher` は `Result<(), FrameworkError>` を返します。これは、ディスパッチャーのレジストリが `RwLock` の背後に存在するためです。エラー経路は、そのロックがポイズニングされている場合（以前の書き込み側がパニックした場合）にのみ発生します。実務上、起動時の呼び出し箇所は `?` を使います。

### ライフサイクルイベント

3つのイベントが、あらゆる同期的なチャネル配信を取り囲みます:

| イベント | いつ | リスナーのエラーの振る舞い |
|---|---|---|
| `NotificationSending` | チャネルが実行される直前 | リスナーの `Err` が、このディスパッチに対してそのチャネルに**拒否権を行使する** |
| `NotificationSent` | 配信が成功した後 | ベストエフォートのディスパッチ - リスナーのエラーは伝播しない |
| `NotificationFailed` | チャネルがエラーを返したとき | ベストエフォートのディスパッチ。下層のチャネルのエラーは、最初の失敗で停止する契約に従って、それでも伝播する |

3つとも `(notification, channel, route, data)` を運びます。`Failed` は、文字列化された `error` を追加します。`EventFacade::listen::<E, L>` でリスナーを登録してください - [イベント](events.md)を参照してください。

これらのイベントは、同期的な `Notify::send` のパスでのみ発火します。キューに入れられたワーカーは、イベントをディスパッチすることなく、チャネルを直接配信します。

### テレメトリ

`NotificationDispatcher::notify` は、そのファンアウトを `notification.dispatch` という `tracing` のスパンでラップします:

- `notification` - `Notification::notification_name()`
- `channel_count` - 宣言されたチャネルの数
- `duration_ms` - 完了時のファンアウトのレイテンシ
- 終端のログ: `notification dispatched`（info）または `notification dispatch failed`（warn）

メールチャネルは、その内側に自分専用の `mail.send` スパンをネストします。

### 最初の失敗で停止する契約

`Notify::send` は、最初のチャネルのエラーで戻ります。すでに成功したチャネルはロールバックされません。まだ実行されていないチャネルは試行されません。同じ契約が、キューに入れられたワーカーにも適用されます。

複数のチャネルにわたる少なくとも1回の配信のためには、各チャネルを、それ専用の `Notify::queue` の呼び出しを通じてディスパッチしてください - キューのエンベロープのべき等性キーが、リトライ時の二重送信から守ります。

## キューに入れられた配信

`Notify::send` はプロセス内で実行されます。`Notify::queue` は、`SendNotificationJob` を[キュー](queues.md)へ投入し、実行時にワーカーが `Notifiable` のハンドルを必要としないよう、チャネルごとのルートを受信者から事前に解決します:

```rust
use suprnova::notifications::register_notification_factory;
use suprnova::Notify;

// 起動時に - Notify::queue を介して到達可能な、具体的な通知ごとに一度。
register_notification_factory::<OrderShipped>()?;

// どこでも:
Notify::queue(&user, OrderShipped { tracking }).await?;
```

ディスパッチの時点で、ワーカーは:

1. `notification_name` によって通知ファクトリーをルックアップする
2. JSONペイロードから型付きの通知を再構築する
3. キューに入れた時点で記録されたチャネルを反復する
4. それぞれについて、`should_send(channel)` を再チェックし（拒否権を行使されたチャネルはスキップする）、束縛済みのディスパッチャー上でそのチャネルをルックアップし、`deliver(route, &notification)` を呼び出し、その後 `after_sending(channel)` を実行する

キューに入れた時点で宣言されていたが、ワーカーが実行される時点で登録されていないチャネルは、`WARN` を記録し、スキップされます - 同期パスと同じ契約です。事前に解決されたルートを持たないチャネルは、無音でスキップされます（受信者がキューに入れた時点で `None` を返していたということです）。

`Notify::queue` は、enqueueの時点でも `should_send` を評価します。そのため、拒否権を行使されたチャネルは、そもそもenqueueされません。ワーカーの再チェックは、enqueueと実行の間に変化する状態をカバーします。キューに入れられたパスは、3つのライフサイクルイベント（`NotificationSending` / `NotificationSent` / `NotificationFailed`）を**発火させません** - それらは同期パス専用のままです。これらのイベントに依存する場合は、`Notify::send` を通じて送ってください。

### キュー調整

さらに5つの `Notification` メソッドが、`Job` 自身の調整メソッドを反映して、通知ごとのキューポリシーを `Notify::queue` のディスパッチへ運びます:

| メソッド | デフォルト | 対応するもの |
|---|---|---|
| `queue(&self)` | `None` - ドライバーのデフォルト、または登録済みなら `Queue::route` | `Job::queue()` |
| `timeout(&self)` | `None` - 試行ごとのタイムアウトなし | `Job::timeout()` |
| `fail_on_timeout(&self)` | `false` - タイムアウトは他の失敗と同様にリトライする | `Job::fail_on_timeout()` |
| `max_tries(&self)` | `3` | `Job::max_tries()` |
| `backoff(&self)` | 指数、2秒ベース、5分上限、±25%ジッター | `Job::backoff()` |

`Notify::queue` は、通知インスタンスから一度これらを読み取り、チャネルごとのすべての `SendNotificationJob` pushへ運びます。5つのいずれもオーバーライドしない通知は、素の `Notify::queue` 呼び出しが常に生成していたものと正確に同じエンベロープを得ます。

```rust
struct WelcomeDigest;

impl Notification for WelcomeDigest {
    fn notification_name() -> &'static str { "WelcomeDigest" }
    fn channels(&self) -> Vec<&'static str> { vec!["mail"] }
    fn data(&self) -> serde_json::Value { serde_json::Value::Null }

    fn queue(&self) -> Option<&'static str> { Some("digests") }
    fn timeout(&self) -> Option<std::time::Duration> { Some(std::time::Duration::from_secs(10)) }
    fn fail_on_timeout(&self) -> bool { true }
}
```

タイムアウトが一時的なものではなく配信不能を意味する場合、`fail_on_timeout(&self)` を `true` にしてください。ワーカーは `max_tries` までリトライせず、最初のタイムアウトでデッドレターに入れます。

この5つのメソッドが適用されるのは `Notify::queue` のみです。`Notify::send` はプロセス内で実行され、調整するキューエンベロープを持ちません。

### Suprnovaが異なる設計を選んだ理由

Laravelは、キューに入れられた通知を `ShouldQueue` マーカーインターフェースにひも付けています - 同じ `Notification::send($user, $notification)` の呼び出しが、通知が `ShouldQueue` を実装していればキューに入れ、実装していなければインラインで送信します。その振る舞いは、通知の側にある型レベルのフラグに依存しており、それは呼び出し箇所からは見えません。

Suprnovaは、その選択をあらゆる呼び出しで明示的にします: `Notify::send` は常に同期的であり、`Notify::queue` は常にキューに入れられます。隠れたモード切り替えはありません。（`send_now` が存在しないのもそのためです - `send` がすでに同期的なものだからです。）

受信者側も分岐しています。Laravelの `Notifiable` トレイトは、インボックスのリレーションシップ、`routeNotificationFor*` メソッド、多態的な主キーを持ち込む、ミックスインです。Suprnovaの `Notifiable` は、意図的に最小限です - 単に `route_for(channel) -> Option<String>` だけです - Rustのトレイトはミックスインによって合成されないからです。Laravelに相当する読み取り側は、`(notifiable_type, notifiable_id)` に対するフリー関数（`unread_for`、`mark_as_read`、…）として出荷されます。そのため、素のままの構造体が、ORMのリレーションシップを継承することなく、通知対象になれます。

## テスト

異なる問いに答える、2つのフェイクの表面です。

### `Notify::fake()` - 「通知はディスパッチされたか？」

```rust
use suprnova::Notify;
use suprnova::notifications::{
    assert_count, assert_nothing_sent, assert_sent_named,
    assert_sent_times, assert_sent_to, assert_sent_to_on,
    recorded_notifications,
};

#[tokio::test]
async fn ship_dispatches_order_shipped() {
    let _fake = Notify::fake();

    Notify::send(
        &User { id: 1, email: "alice@example.org".into() },
        &OrderShipped { tracking: "1Z…".into() },
    ).await.unwrap();

    assert_sent_named("OrderShipped");
    assert_sent_to("alice@example.org", "OrderShipped");
    assert_sent_to_on("alice@example.org", "mail", "OrderShipped");
    assert_sent_times("OrderShipped", 1);
    assert_count(2); // メール + データベース
}
```

フェイクのガードが生きている間、`Notify::send` と `Notify::queue` はどちらも、チャネルを実行したりジョブをenqueueしたりする代わりに、そのディスパッチを記録します - どのチャネルも実行されず、どのキューの行も書き込まれません。このフェイクは、プロセス全体のシリアライゼーションmutexを保持するため、並行するテストがキャプチャを入り交ぜることはできません - テストの終わりに `_fake` ガードをドロップさせ、レコーダーをクリアしてください。

キャプチャされたデータを完全に管理するには、`recorded_notifications()` を使ってください:

```rust
let records = recorded_notifications();
assert_eq!(records[0].notification, "OrderShipped");
assert_eq!(records[0].channel, "mail");
assert_eq!(records[0].data["tracking"], "1Z…");
```

### `Mail::fake()` + 実際の `MailChannel` - 「通知は正しく*レンダリング*されたか？」

`Notify::fake()` は、チャネルの前でショートサーキットします。メール本文が実際にあなたの期待どおりにレンダリングされたことをアサートするには、`Mail::fake()` の下で実際のチャネルを駆動してください:

```rust
use serial_test::serial;
use std::sync::Arc;
use suprnova::mail::Mail;
use suprnova::notifications::{set_dispatcher, NotificationDispatcher};
use suprnova::{MailChannel, Notify, register_mail_renderer};

#[tokio::test]
#[serial]
async fn ordershipped_renders_tracking_in_subject() {
    let fake = Mail::fake();
    register_mail_renderer::<OrderShipped>().unwrap();
    set_dispatcher(Arc::new(
        NotificationDispatcher::new()
            .register_channel(Arc::new(MailChannel::new())),
    )).unwrap();

    Notify::send(
        &User { id: 1, email: "alice@example.org".into() },
        &OrderShipped { tracking: "1Z…".into() },
    ).await.unwrap();

    fake.assert_sent_count(1);
    fake.assert_sent(|m| m.subject.contains("1Z…"));
}
```

ディスパッチャー、レンダラー、またはトランスポートのグローバル変数に触れるテストは、`#[serial_test::serial]` でなければなりません - それらはプロセスグローバルな静的変数だからです。

## ベストプラクティス

### 起動時にすべてのファクトリーとレンダラーを登録する

`Notify::queue` は、ワーカーでファクトリーレジストリを介して通知を再構築し、`MailChannel` は `register_mail_renderer` を介してレンダリングします。キューに入れられる/メール送信可能な通知はすべて、あらかじめ登録してください:

```rust
// bootstrap.rs
use suprnova::notifications::register_notification_factory;
use suprnova::register_mail_renderer;

pub fn register() -> Result<(), FrameworkError> {
    // 通知ファクトリー（Notify::queue を介して到達可能な通知ごとに1つ）。
    register_notification_factory::<OrderShipped>()?;
    register_notification_factory::<InvoicePaid>()?;

    // メールレンダラー（NotificationMailable ごとに1つ）。
    register_mail_renderer::<OrderShipped>()?;
    register_mail_renderer::<InvoicePaid>()?;
    Ok(())
}
```

キュー上の未登録の通知は、ワーカーの実行時に `unknown notification: {name}` として表面化し、デッドレターの経路を通じてリトライされます。未登録のレンダラーに対する `MailChannel` のディスパッチは、同じように `register via suprnova::register_mail_renderer::<N>()` というエラーを表面化させます。

### マルチチャネルのファンアウトにはキューを使う

同期的なディスパッチャーは、チャネルを順番に訪れ、最初のエラーで戻ります。チャネル#2での失敗は、チャネル#1をコミット済みのままにし、チャネル#3以降を未試行のままにします。複数のチャネルにわたるあらゆる通知については、ワーカーがバックオフ付きのリトライを処理し、ディスパッチがプロセスのクラッシュを生き延びられるよう、`Notify::queue` を優先してください。

### チャネルの配信をべき等にする

ワーカーのリトライは、同じ `SendNotificationJob` が2回以上実行されうるということを意味します。組み込みのチャネルは、べき等性に優しく作られています: `MailChannel` は、通常メッセージidで重複排除するプロバイダーへ転送します。`DatabaseChannel` は、実行ごとに新しいUUIDを挿入します（これは監査の行にとって正しい振る舞いです）。`WebPushChannel` は、重複を吸収するプロバイダーへPOSTします。カスタムのチャネルは、べき等な操作を目指すべきです - 安定したクライアント側の重複排除キーを伴うHTTP POST、盲目的な挿入ではなくupsert、配信パス上での「カウンターを増やす」ような副作用を持たないこと、などです。

### ディスパッチャーを1箇所で束縛する

`register_channel` は最後の書き込みが勝つため、テストはセットアップの中で実際のチャネルをスタブに差し替えられます。本番のバインディングは `bootstrap.rs` に留め、テストには、必要な任意のスタブで自分専用のディスパッチャーを構築させてください。リクエストハンドラの内側で `register_channel` を遅延的に呼ばないでください - グローバルなロックへの書き込みと、最後の書き込みが勝つセマンティクスの組み合わせは、並行負荷の下で驚くような結果になります。

## リファレンス

| シンボル | パス |
|---|---|
| `Notifiable`、`Notification`、`Channel`、`DynNotification` | `suprnova::` |
| `Notify`（ファサード）、`NotifyFakeGuard` | `suprnova::` |
| `NotificationDispatcher`、`NotificationFactory` | `suprnova::` |
| `AnonymousNotifiable` | `suprnova::` |
| `MailChannel`、`MailRendering`、`NotificationMailable` | `suprnova::` |
| `register_mail_renderer::<N>()` | `suprnova::` |
| `DatabaseChannel`、`StoredNotification` | `suprnova::` |
| `WebPushChannel` | `suprnova::` |
| `BroadcastChannel` | `suprnova::` |
| `SendNotificationJob` | `suprnova::` |
| `NotificationSending`、`NotificationSent`、`NotificationFailed` | `suprnova::` |
| `set_dispatcher`、`register_notification_factory` | `suprnova::notifications::` |
| `all_for`、`unread_for`、`read_for`、`mark_as_read`、`mark_as_unread`、`mark_all_as_read`、`delete_for` | `suprnova::notifications::` |
| `assert_sent`、`assert_sent_named`、`assert_sent_times`、`assert_sent_to`、`assert_sent_to_on`、`assert_nothing_sent`、`assert_nothing_sent_to`、`assert_count`、`recorded_notifications` | `suprnova::notifications::` |
| `#[derive(NotificationMailable)]` | `suprnova::` |

## 次のステップ

- [メール](mail.md) - メールチャネルが乗っている、トランスポートと `Mailable` の表面
- [ブロードキャスト](broadcasting.md) - ブロードキャストチャネルが発行する先の `BroadcastHub`
- [Web プッシュ](web-push.md) - VAPID、暗号化、購読の保存
- [イベント](events.md) - `NotificationSending` / `Sent` / `Failed` のリスニング
- [キュー](queues.md) - `Notify::queue` を駆動するワーカー
- [テスト](testing.md) - フェイクの表面とserial-testのパターン
