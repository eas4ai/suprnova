# ブロードキャスト

ブロードキャストは、Suprnovaの[WebSocketプリミティブ](websockets.md)の上に構築された、サーバーからクライアントへの通知層です。あなたは、`EventFacade` を介して `Broadcastable` なイベントをディスパッチします。フレームワークは、そのイベントのJSONエンベロープを、そのイベントが名指しするチャネル上のすべてのWebSocket購読者へファンアウトします。あなたは個々のコネクションを管理することは決してありません - チャネルの購読を管理するだけで、hubが残りをやってくれます。

`BroadcastHub` はバスです。デフォルトの `InMemoryBroadcastHub` は、完全にプロセス内で動作します - 単一レプリカのデプロイやテストスイートに最適です。`broadcasting-fanout` というCargoのフィーチャーの裏では、`SeaStreamerBroadcastHub` が同じイベントをストリームブローカー（Redis Streams、Kafka、file、stdio）経由でルーティングします。そのため、1つのプロセスでの発行が、他のすべてのプロセスの購読者に届きます。

[WebSocket](websockets.md)の章のすべてが、それでも適用されます - ハートビートのping、`max_missed_pings`、`WsConfig`、ルートごとのミドルウェア、パスパラメータです。ブロードキャストは、その上に、通信プロトコルとチャネルのレジストリを追加するだけです。

## クイックスタート

4つのファイルで、ブラウザはイベントを目にします。

`src/channels/order_updates.rs`:

```rust
use async_trait::async_trait;
use suprnova::broadcasting::Channel;

pub struct OrderUpdates;

#[async_trait]
impl Channel for OrderUpdates {
    fn name(&self) -> &'static str { "order.updates" }
}
```

`src/events/order_placed.rs`:

```rust
use serde::{Deserialize, Serialize};
use suprnova::Event;
use suprnova::broadcasting::Broadcastable;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderPlaced {
    pub order_id: i64,
    pub user_id: i64,
}

impl Event for OrderPlaced {
    fn event_name() -> &'static str { "OrderPlaced" }
}

impl Broadcastable for OrderPlaced {
    fn broadcast_on(&self) -> Vec<String> {
        vec!["order.updates".into()]
    }
}
```

`src/bootstrap.rs`:

```rust
use std::sync::Arc;
use suprnova::broadcasting::{BroadcastHub, ChannelRegistry, InMemoryBroadcastHub};
use suprnova::container::App;
use suprnova::events::EventFacade;

pub async fn register() {
    // 1. トレイトの背後でhubを束縛する - ハンドラは、これを均一に解決する。
    let hub: Arc<dyn BroadcastHub> = Arc::new(InMemoryBroadcastHub::new());
    App::bind::<dyn BroadcastHub>(Arc::clone(&hub));

    // 2. すべてのチャネルを先に登録する。WSハンドラは名前で解決する。
    let mut registry = ChannelRegistry::new();
    registry.register(OrderUpdates);
    App::singleton(Arc::new(registry));

    // 3. Broadcastable の型ごとに一度、イベント → hub のブリッジを配線する。
    EventFacade::broadcast::<OrderPlaced>(Arc::clone(&hub)).await;
}
```

`src/routes.rs` - ブートストラップされたhubとレジストリをコンテナから解決して、ルートごとに `BroadcastingWsHandler` を構築します:

```rust
use std::sync::Arc;
use suprnova::broadcasting::{
    BroadcastHub, BroadcastingWsHandler, ChannelRegistry, InMemoryBroadcastHub,
};
use suprnova::container::App;
use suprnova::{routes, ws, AuthMiddleware};

fn broadcasting_handler() -> BroadcastingWsHandler {
    // コンテナ優先。bootstrapなしでルーターを組み立てる単体テストもそれでも
    // 動作するよう、新しいプロセス内hub + 空のレジストリへフォールバックする。
    let hub: Arc<dyn BroadcastHub> = App::make::<dyn BroadcastHub>()
        .unwrap_or_else(|| Arc::new(InMemoryBroadcastHub::new()));
    let registry: Arc<ChannelRegistry> = App::get::<Arc<ChannelRegistry>>()
        .unwrap_or_else(|| Arc::new(ChannelRegistry::new()));
    BroadcastingWsHandler::new(hub, registry)
}

routes! {
    ws!("/ws/broadcast", broadcasting_handler())
        .middleware(AuthMiddleware::new()),
}
```

接続して観察します:

```bash
wscat -c ws://localhost:3000/ws/broadcast
> {"action":"connected","socket_id":"6f1a3c2e-…"}
> {"action":"subscribe","channel":"order.updates","data":{}}
< {"action":"subscribed","channel":"order.updates"}
```

任意のコントローラー、ワーカー、またはスケジュールされたタスクからディスパッチします:

```rust
EventFacade::dispatch(OrderPlaced { order_id: 99, user_id: 42 }).await?;
```

```
< {"action":"event","channel":"order.updates","event":"OrderPlaced","data":{"order_id":99,"user_id":42}}
```

## チャネル

チャネルとは、名前付きの購読対象です。クライアントは名前で購読し、hubはその名前上のすべてのアクティブな購読者へイベントを配信します。`Channel` トレイトは、書き込みではフェイルクローズし、読み取りではフェイルオープンする、非対称なデフォルトを持っています - 下記の[Suprnovaが異なる設計を選んだ理由](#suprnovaが異なる設計を選んだ理由)を参照してください。

### パブリックチャネル

デフォルトです。任意のクライアントが購読できます。

```rust
use async_trait::async_trait;
use suprnova::broadcasting::Channel;

pub struct OrderUpdates;

#[async_trait]
impl Channel for OrderUpdates {
    fn name(&self) -> &'static str { "order.updates" }
    // authorize() はデフォルトでtrue - すべての購読者に開かれている。
}
```

### プライベートチャネル

購読をゲートするには `authorize` をオーバーライドしてください。拒否された購読は、`reason: "unauthorized"` を伴う `error` フレームを生成します。`subscribed` フレームは送信されません。

```rust
use async_trait::async_trait;
use serde_json::Value;
use suprnova::broadcasting::{Channel, ChannelParams, PrivateChannel};
use suprnova::http::Request;

pub struct PrivateChat;

#[async_trait]
impl Channel for PrivateChat {
    fn name(&self) -> &'static str { "chat.private" }

    async fn authorize(
        &self,
        _req: &Request,
        _params: &ChannelParams,
        data: &Value,
    ) -> bool {
        data["token"].as_str().map(|t| t == "valid").unwrap_or(false)
    }
}

impl PrivateChannel for PrivateChat {}
```

`data` は、クライアントが購読フレームの `data` フィールドに送ってきたものなら何でもです - bearerトークン、署名されたチャネルバインド、アプリケーション定義の何でも、です。`Request` は元のHTTPアップグレードリクエストです（ヘッダーとクッキーは直接読み取れます）。`params` は、パラメータ化された名前からキャプチャされた値を運び、固定名では空です。

`PrivateChannel` はマーカートレイトです。フレームワークは、実行時にこれをチェックしません - これは、そのチャネルが `authorize` をオーバーライドしているという型レベルの信号であり、将来のツール（clippyのリント、監査）のために意図されています。

### パラメータ化されたチャネル

`name()` に `{param}` のセグメントを埋め込むと、1回の登録が、そのパターンに一致するすべての具体的な購読に応えます - Laravelの `Broadcast::channel('orders.{id}', …)` と同じモデルです。キャプチャされた値は、`ChannelParams` のマップとして、あらゆるフックに届きます。

```rust
use async_trait::async_trait;
use serde_json::Value;
use suprnova::broadcasting::{Channel, ChannelParams, PrivateChannel};
use suprnova::http::Request;

pub struct OrderChannel;

#[async_trait]
impl Channel for OrderChannel {
    fn name(&self) -> &'static str { "orders.{id}" }

    async fn authorize(
        &self,
        _req: &Request,
        params: &ChannelParams,
        _data: &Value,
    ) -> bool {
        let order_id = params.get("id").unwrap_or_default();
        // キャプチャされたidでゲートする - セッションのユーザーはこの注文を所有しているか？
        !order_id.is_empty()
    }
}

impl PrivateChannel for OrderChannel {}

// 1回の登録が orders.42、orders.99、orders.featured、… に応える
registry.register(OrderChannel);
```

各 `{param}` は、正確に1つのドットセグメントにバインドします: `orders.{id}` は `orders.42` にマッチしますが、`orders` や `orders.42.line` にはマッチしません。解決は、まず正確な固定名の登録をあらゆるパターンより優先し（`orders.featured` は、その1つの名前についてなら `orders.{id}` に勝ちます）、次に最も具体的なパターン（最もリテラルなセグメントを持つもの）を優先し、決定的なタイブレークとして辞書順で最小のパターンを使います。

### プレゼンスチャネル

プレゼンスチャネルは、メンバーシップを追跡します。クライアントが購読すると、hubはそのクライアントへ `presence.here` のスナップショットを配信し、他のすべての購読者へ `presence.joined` をブロードキャストします。クライアントが離れると、hubは `presence.left` をブロードキャストします。

この2部構成の契約は、半分だけ実装してしまいがちです: あなたは、`Some(self)` を返すよう `Channel::presence_info` をオーバーライドすることと、`PresenceChannel::member_info` を実装することの、両方をしなければなりません。`presence_info` を忘れると、そのチャネルは非プレゼンスとして配線されます - 購読は動作しますが、`presence.joined` / `presence.here` / `presence.left` は決して発火しません。

```rust
use async_trait::async_trait;
use serde_json::{json, Value};
use suprnova::FrameworkError;
use suprnova::broadcasting::{Channel, ChannelParams, PresenceChannel};
use suprnova::http::Request;

pub struct PresenceLobby;

#[async_trait]
impl Channel for PresenceLobby {
    fn name(&self) -> &'static str { "presence.lobby" }

    // 必須 - このオーバーライドがなければ、PresenceChannel は配線されるが不活性のままになる。
    fn presence_info(&self) -> Option<&dyn PresenceChannel> {
        Some(self)
    }
}

#[async_trait]
impl PresenceChannel for PresenceLobby {
    async fn member_info(
        &self,
        _req: &Request,
        _params: &ChannelParams,
    ) -> Result<Value, FrameworkError> {
        // 他の購読者がこのメンバーを識別するために必要とするものを返す -
        // 典型的にはユーザーidである。秘密や非公開のPIIを含めては決してならない。
        Ok(json!({ "user_id": 42, "display_name": "Alice" }))
    }
}
```

完全なイベントフローと自己参加のエコーについては、[プレゼンス](#プレゼンス)を参照してください。

### 予約された名前

`__` で始まる名前は、フレームワークのメタチャネル用に予約されています（`__presence__` は、クロスプロセスのプレゼンスのレプリケーションを運びます）。`__` が前置された名前に対して `registry.register(channel)` を呼ぶと、その間違いが実行時ではなく起動時に捕まえられるよう、登録時にパニックします。

### Suprnovaが異なる設計を選んだ理由

Laravelは、チャネルの認可を `$user` コールバックパラメータにひも付けます。なぜなら、PHPは現在認証されているユーザーを暗黙的に注入するからです。Suprnovaの `authorize` は、代わりに、生の `Request`、キャプチャされた `ChannelParams`、そして任意の `data: Value` を取ります - 3つの直交する入力であり、すべてが利用可能で、暗黙のコンテキストはありません。あなたは、`Request` からセッションクッキーやbearerトークンを読み、`ChannelParams` からルーティング形式のパラメータを読みます。`data` のペイロードは、クライアントが購読時に提供するトークンのための自由な枠です。

`Channel` トレイトのデフォルトは、**意図的に非対称です**: `authorize` はデフォルトで `true` です（購読はデフォルトで公開されています）。`authorize_publish` はデフォルトで `false` です（クライアント起点の発行はデフォルトで拒否されます）。危険な操作はフェイルクローズし、安全な操作はフェイルオープンします。迷ったときは、両方をそのままにしておいてください。

## `Broadcastable` トレイト

`Broadcastable: Event + Serialize` - あらゆる `Broadcastable` は `Event` でもあります。`EventFacade::dispatch(event)` を介したディスパッチは、あらゆるプロセス内のリスナーを実行し、かつ、そのイベントが名指しするチャネル上のすべてのWebSocket購読者へ、JSONシリアライズされたペイロードをプッシュします。

```rust
use serde::{Deserialize, Serialize};
use suprnova::Event;
use suprnova::broadcasting::Broadcastable;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderPlaced {
    pub order_id: i64,
    pub user_id: i64,
}

impl Event for OrderPlaced {
    fn event_name() -> &'static str { "OrderPlaced" }
}

impl Broadcastable for OrderPlaced {
    fn broadcast_on(&self) -> Vec<String> {
        // 1つのイベント、複数のチャネル。各チャネルの各購読者が、
        // 同じエンベロープを受け取る。
        vec![
            format!("user.{}.orders", self.user_id),
            "orders.global".into(),
        ]
    }
}
```

起動時に、Broadcastableの型ごとに一度、このブリッジを配線します:

```rust
EventFacade::broadcast::<OrderPlaced>(Arc::clone(&hub)).await;
```

それ以降は、`EventFacade::dispatch(event).await?` が送信側の全体です - 別個の `publish` の呼び出しはありません。

デフォルトでは、イベントは `serde_json::to_value(&event)` を介してシリアライズされ、すべての購読者へプッシュされます。購読者がゼロのチャネルは、プロセス内のhubでは無音でスキップされます。クロスプロセスのhubは、他のプロセスが配信する機会を得られるよう、それでもそれらを発行します。

4つの省略可能なメソッドが、デフォルトを洗練させます:

**`broadcast_event_name(&self) -> &'static str`** - 通信上のイベント名をオーバーライドします。デフォルトは `Self::event_name()` です。プロセス内のイベントのアイデンティティを、通信上の名前から分離するために使ってください。

**`broadcast_with(&self) -> Option<Value>`** - 完全なイベントのシリアライズの代わりに、精選されたペイロードをプッシュするには、`Some(value)` を返してください（Laravelの `broadcastWith()`）。イベントの型を変えずに、秘密を省いたり、クライアント向けに形を変えたりできます:

```rust
impl Broadcastable for AccountFunded {
    fn broadcast_on(&self) -> Vec<String> {
        vec![format!("account.{}", self.account_id)]
    }
    fn broadcast_with(&self) -> Option<serde_json::Value> {
        // 残高を通信に乗せては決してならない - 公開されているidだけにする。
        Some(serde_json::json!({ "account_id": self.account_id }))
    }
}
```

**`broadcast_when(&self) -> bool`** - イベントをプロセス内のリスナーへディスパッチしつつ、WebSocketへのプッシュをスキップするには、`false` を返してください（Laravelの `broadcastWhen()`）。ゲートされるのはブロードキャストだけです。イベントパイプラインの残りは変わらずに走ります:

```rust
impl Broadcastable for DraftSaved {
    fn broadcast_on(&self) -> Vec<String> { vec![format!("doc.{}", self.doc_id)] }
    fn broadcast_when(&self) -> bool { self.publish } // publish時にのみブロードキャストする
}
```

**`broadcast_to_others(&self) -> bool`** - ブロードキャストを引き起こしたコネクションを除外するには、`true` を返してください（Laravelの `toOthers()`）。フレームワークは、接続時に、ブロードキャストする各コネクションに `socket_id` を割り当てます（`connected` フレームで送られます）。ブラウザは、それをHTTPリクエスト上の `X-Socket-ID` ヘッダーとして返します。そのリクエストを処理している間にディスパッチされた `broadcast_to_others` イベントは、発生元のコネクションをスキップします。リクエストの外（ワーカーやジョブ）や、`X-Socket-ID` が存在しない場合は、全員へのブロードキャストへ退化します:

```rust
impl Broadcastable for MessagePosted {
    fn broadcast_on(&self) -> Vec<String> { vec![format!("chat.{}", self.room)] }
    fn broadcast_to_others(&self) -> bool { true } // 送信者はすでにそれを持っている
}
```

これは、イベントの型ごとの選択です。ディスパッチごとの除外のためには、直接発行してください:

```rust
use suprnova::broadcasting::BroadcastEnvelope;

hub.publish(
    BroadcastEnvelope::new(channel, event, data).with_except(socket_id),
).await?;
```

### 兄弟リスナーとのディスパッチ順序

`EventFacade::dispatch` は**フェイルファスト**です: hubのpublishが `Err` を返した場合（クロスプロセスのhubでのブローカーの切断など）、`BroadcastListener` は `Err` を返し、それより**後に**登録された兄弟リスナーは実行されません。これに対処する方法は2つあります:

- ブロードキャストの結果にかかわらず副作用（DBへの書き込み、ログの発行）が実行されなければならない、プロセス内のリスナーの**後に**、ブロードキャストのブリッジを登録する。
- 1つが `Err` を返しても関係なく、すべてのリスナーが実行されなければならないときは、`EventFacade::dispatch_best_effort(event)` に切り替える。

インメモリのhubは、`Err` を返すことは決してありません - ブローカーの失敗を表面化させるのは、クロスプロセスのバリアントだけです。

## 通信プロトコル

ブロードキャストのルート上のすべてのメッセージは、UTF-8のJSONフレームです。2つの形があります: `ClientFrame`（クライアント → サーバー）と `ServerFrame`（サーバー → クライアント）です。

### クライアントフレーム

| `action` | 必須フィールド | 省略可能なフィールド | 意味 |
|----------|-----------------|-----------------|---------|
| `subscribe` | `channel` | `data` | `channel` を購読する。`data` は `Channel::authorize` へ転送される。 |
| `unsubscribe` | `channel` | | `channel` から離脱する。 |
| `publish` | `channel`、`event`、`data` | | `channel` 上のすべての購読者へイベントをプッシュする。`Channel::authorize_publish` によってゲートされ、かつ生きた購読を必要とする。 |

クライアント起点の `publish` は、**2つ**のチェックによってゲートされます: コネクションは、対象のチャネルへの認可された購読を保持していなければ**なりません**。かつ、`Channel::authorize_publish` は `true` を返さなければなりません（デフォルトは `false` です）。これは、Pusherのクライアントイベントの契約を反映しています - クライアントの発行を望むチャネルは、そのフックをオーバーライドすることで明示的にオプトインします。ほとんどのサーバー側のブロードキャストチャネルは、クライアント起点のイベントを決して望まないため、デフォルトで拒否する形は、その意図に一致します。

```json
{"action":"subscribe","channel":"chat.42","data":{"token":"abc"}}
{"action":"unsubscribe","channel":"chat.42"}
{"action":"publish","channel":"chat.42","event":"MessagePosted","data":{"text":"hi"}}
```

### サーバーフレーム

| `action` | フィールド | 意味 |
|----------|--------|---------|
| `connected` | `socket_id` | 最初に一度だけ送られる。サーバー側の `broadcast_to_others` がこのコネクションを除外できるよう、`socket_id` を `X-Socket-ID` のHTTPヘッダーとしてエコーすること。 |
| `subscribed` | `channel` | 購読が受け入れられた。 |
| `unsubscribed` | `channel` | 購読解除が確認された。 |
| `event` | `channel`、`event`、`data` | `channel` 上でイベントがブロードキャストされた。 |
| `lagged` | `channel`、`skipped` | 購読者がサーバーのチャネルごとのリングバッファに遅れを取り、`skipped` 個のエンベロープがこのコネクション上で捨てられた。`channel` に関するクライアントのローカルな状態は古い - それ以降のイベントを処理する前に再取得すること。 |
| `error` | `channel`（null許容）、`reason` | 直前の操作が失敗した。チャネルにひも付かないエンベロープレベルのエラーでは `channel` は `null` になる。 |

```json
{"action":"connected","socket_id":"6f1a3c2e-…"}
{"action":"subscribed","channel":"chat.42"}
{"action":"unsubscribed","channel":"chat.42"}
{"action":"event","channel":"chat.42","event":"MessagePosted","data":{"text":"hi"}}
{"action":"lagged","channel":"chat.42","skipped":42}
{"action":"error","channel":"chat.42","reason":"unauthorized"}
{"action":"error","channel":null,"reason":"malformed envelope: …"}
```

#### `lagged` について

各チャネルは、プロセスごとのリングバッファ（256エンベロープ）を持ちます。十分な速さでドレインしない購読者 - 遅いクライアント、詰まったフォワーダー - は遅れを取り、バッファは最も古いイベントを上書きします。それが起きると、サーバーは、そのチャネルを名指しし、捨てられたイベントの数を伴う `lagged` フレームを1つ送り、その後は通常どおり後続のフレームを配信し続けます。このギャップは、サーバー側からは**決して**回復できません - クライアントは、そのチャネル上のそれ以降のイベントを処理する前に、再取得または再同期しなければなりません。イベントを無音で捨ててしまうと、バグが「クライアントの状態がサーバーの状態から分岐した」ではなく「1ティックを失った」として隠れてしまいます。

#### 発行の失敗

クライアント起点の `publish` が `authorize_publish` に受け入れられたものの、hubのpublish自体が失敗した場合（クロスプロセスのhubでのブローカーの切断など）、発生元のクライアントは、そのイベントが他のプロセスに届かなかったことを知れるよう、`reason: "publish failed: …"` を伴う `error` フレームを受け取ります。他の購読者には通知されません。

### セッションの例

```
S → C  {"action":"connected","socket_id":"6f1a3c2e-…"}
C → S  {"action":"subscribe","channel":"order.updates","data":{}}
S → C  {"action":"subscribed","channel":"order.updates"}

# サーバーがOrderPlacedをディスパッチする:
S → C  {"action":"event","channel":"order.updates","event":"OrderPlaced","data":{"order_id":99,"user_id":42}}

C → S  {"action":"subscribe","channel":"chat.private","data":{"token":"bad"}}
S → C  {"action":"error","channel":"chat.private","reason":"unauthorized"}

C → S  {"action":"unsubscribe","channel":"order.updates"}
S → C  {"action":"unsubscribed","channel":"order.updates"}
```

## ルートごとのミドルウェア

ブロードキャストのルートは、素のWebSocketルートと同じ `.middleware(M)` のチェーンをサポートします:

```rust
ws!("/ws/broadcast", broadcasting_handler())
    .middleware(AuthMiddleware::new()),
```

いずれかのミドルウェアからの非2xxレスポンスは、アップグレードをショートサーキットします - クライアントはHTTPのエラーレスポンスを受け取り、WebSocketのハンドシェイクは起きません。これは、あらゆるチャネルの `authorize` の内側でチェックを重複させることなく、トランスポートレベルの認証（セッションの有効性、オリジンのチェック、接続時のレートリミット）を強制するための正しい場所です。

複数のミドルウェアは左から右へ合成されます:

```rust
ws!("/ws/broadcast", broadcasting_handler())
    .middleware(AuthMiddleware::new())
    .middleware(RateLimitMiddleware::connections_per_ip(100)),
```

この分割は意図的なものです: **トランスポートレベル**（そもそも誰がコネクションを開けるか）はミドルウェアに存在し、**チャネルレベル**（誰がどのチャネルを購読できるか）は `Channel::authorize` に存在します。

### ルートごとの `WsConfig`

プロセス全体のWebSocketのデフォルトを、ルートごとに上書きします。ハンドラの後に `.config(WsConfig { ... })` をチェーンしてください - `.middleware(M)` の前でも後でも構いません（順序は関係ありません）:

```rust
use std::time::Duration;
use suprnova::ws::WsConfig;

ws!("/ws/chat", broadcasting_handler())
    .config(WsConfig {
        ping_interval: Duration::from_secs(5),
        max_missed_pings: 1,
        ..Default::default()
    })
    .middleware(AuthMiddleware::new())
```

設定可能な5つのフィールドと、それぞれが重要になる場面:

| フィールド | デフォルト | 使用場面 |
|-------|---------|----------|
| `ping_interval` | 30秒 | チャット / プレゼンス: 死んだモバイルコネクションを素早く検知するため、5〜10秒に短縮する。大量データのストリーミング: オーバーヘッドを減らすため、長くする。 |
| `max_missed_pings` | 2 | 1回の見逃したPongで即座に閉じるべきチャットには `1` を設定する。不安定なモバイルネットワークには `3以上` を設定する。no-pong時のクローズを無効化するには `usize::MAX` を設定する。 |
| `max_message_size` | 1 MiB | 公開エンドポイントで安全なデフォルト。信頼済みの内部フィードには `WsConfig::generous()`（64 MiB）を出発点にする。 |
| `max_frame_size` | 64 KiB | 余裕を持たせた、チャット / 通知フレーム向けの大きさ。断片化されない大きなフレームには `WsConfig::generous()`（16 MiB）を出発点にする。 |
| `origin_policy` | `SameOrigin` | デフォルトはクロスオリジンのアップグレードを拒否する - ブラウザのWSハンドシェイクが持つ唯一のCSRF対策。明示的なクロスオリジンのフロントエンドには `AllowList(vec![...])` を、ブラウザ以外のエンドポイントにのみ `AllowAny` を使う。 |

`.config(...)` が与えられない場合、そのルートは `WsConfig::default()` を継承します。明示的なルートごとの設定は、常にデフォルトに勝ちます。

信頼済みの内部フィード（サーバー間のファンアウト、大きなバイナリ転送）を扱うルートについては、信頼済みフィード用のファクトリーを出発点にし、必要に応じて調整してください:

```rust
use suprnova::ws::WsConfig;
use std::time::Duration;

ws!("/ws/internal/firehose", FirehoseHandler::new())
    .config(WsConfig {
        ping_interval: Duration::from_secs(10),
        ..WsConfig::generous() // 64 MiBのメッセージ / 16 MiBのフレーム
    })
```

## プレゼンス

クライアントがプレゼンスチャネルの購読に成功すると、hubは:

1. 参加するメンバーのデータを収集するために、アップグレードの `Request` とキャプチャされた `ChannelParams` を伴って `PresenceChannel::member_info` を呼び出す。
2. 新しい購読者へ、`data: { "members": [...] }` を伴う `presence.here` のイベントフレームを送る - 現在追跡されているすべてのメンバーのスナップショットである（新しく参加した本人は除く）。
3. `data: <member_info>` を伴う `presence.joined` のイベントをそのチャネルへ発行する。すべての購読者 - 自身のフォワーダーを介した新しい購読者も含む - がそれを受け取る。クライアントは、参加したメンバーのアイデンティティを自分自身のものと比較することで、自己参加をフィルタする。

購読者が切断するか、購読解除フレームを送ると:

4. hubは、離脱するメンバーのデータを伴う `presence.left` のイベントを発行する。残っているすべての購読者がそれを受け取る。

3つのフレームはすべて、予約された `event` 名を伴う `event` アクションのフレームとして届きます:

```json
{"action":"event","channel":"presence.lobby","event":"presence.here","data":{"members":[{"user_id":1},{"user_id":2}]}}
{"action":"event","channel":"presence.lobby","event":"presence.joined","data":{"user_id":3}}
{"action":"event","channel":"presence.lobby","event":"presence.left","data":{"user_id":3}}
```

プロセスをまたいで、プレゼンスの状態は、予約された `__presence__` メタチャネルを介してレプリケートされます（[クロスプロセスのファンアウト](#クロスプロセスのファンアウト)を参照）。どのプロセス上でのtrackとuntrackの操作も、すべての購読者へ伝播します。`list_members` は、統合されたビュー（ローカル + リモート）を返します。`untrack_member` が一度も発火しなかった、死んだプロセスのメンバーは、TTL経由で刈り取られます - デフォルトは60秒です。

## クロスプロセスのファンアウト

デフォルトの `InMemoryBroadcastHub` は、現在のプロセス上の購読者へのみファンアウトします。マルチレプリカのデプロイのためには、`broadcasting-fanout` というCargoのフィーチャーを有効にし、`SeaStreamerBroadcastHub` に差し替えてください:

`Cargo.toml`:

```toml
suprnova = { git = "https://github.com/entrepeneur4lyf/suprnova.git", tag = "v1.2.0", features = ["broadcasting-fanout"] }
```

`src/bootstrap.rs`:

```rust
use std::sync::Arc;
use suprnova::broadcasting::{BroadcastHub, ChannelRegistry};
use suprnova::broadcasting::fanout::SeaStreamerBroadcastHub;
use suprnova::container::App;

pub async fn register() {
    let hub: Arc<dyn BroadcastHub> = Arc::new(
        SeaStreamerBroadcastHub::new(
            "redis://broker:6379",   // ストリーマーのURI（スキームからバックエンドが選ばれる）
            "suprnova-broadcast",    // ストリームキー（クラスタ内のすべてのプロセスで共有される）
        )
        .await
        .expect("connect"),
    );
    App::bind::<dyn BroadcastHub>(Arc::clone(&hub));
    // ... bootstrapの残りは変わらない
}
```

このコンストラクタは、2つの引数を取ります: ストリーマーのURI（実行時にスキームでバックエンドを選ぶ）と、ストリームキー（クラスタ内のすべてのプロセスで共有されるトピック名）です。すべてのレプリカで同じストリームキーを使ってください。そうしなければ、互いのイベントが見えません。

`new_with_presence_ttl(uri, key, ttl)` は、デフォルトの60秒のプレゼンスTTLを上書きします - クラッシュリカバリの経路を素早く動かす必要があるテストに便利です。`new_loopback(uri, key)` は、単一プロセスの統合テストのためにstdioのループバックを有効にします。重複防止ガードは、各アプリのイベントが、それでもローカルでちょうど1回だけ配信されることを保証します。

### バックエンド

バックエンドは、実行時にURIのスキームから選ばれます:

| URIのスキーム | バックエンド | 本番運用可能 | 注記 |
|------------|---------|------------------|-------|
| `redis://`、`rediss://` | Redis Streams | **はい** | デフォルトの推奨。`rediss://` はTLSを使う。デフォルトのビルドで有効。 |
| `kafka://`、`kafka+ssl://` | Kafka | **はい** | `sea-streamer` のフィーチャーセット（`framework/Cargo.toml`）に `kafka` が必要。 |
| `stdio://` | stdin/stdoutのパイプ | いいえ - テスト専用 | 単一プロセスのループバック。 |
| `file://` | ローカルファイル | いいえ - 単一ホスト | `sea-streamer` のフィーチャーセットに `file` が必要。 |

デフォルトのSuprnovaのビルドは、`stdio` + `redis` + `socket` を有効にします。Kafkaやfileを有効にするには、`framework/Cargo.toml` を編集し、対応する `sea-streamer` のフィーチャーを追加してください。

### アーキテクチャ

各 `publish(envelope)` は、2つのことを並行して行います:

1. **ローカルなファンアウト** - 内側の `InMemoryBroadcastHub` が、このプロセス上の購読者へ即座に配信します。ローカルな購読者は、ネットワークを待つことは決してありません。
2. **ストリームへの書き込み** - 同じエンベロープがシリアライズされ、sea-streamerのストリームへプッシュされます。そのため、他のすべてのプロセスのコンシューマーポンプがそれを拾い上げ、ローカルに配信します。

重複配信ガードは、各アプリデータのイベントを2回目にしないようにします: hubのインスタンスはランダムなUUIDを持ち、それが生成するあらゆるエンベロープはそのUUIDを運び、コンシューマーポンプは、インスタンスidがローカルなhub自身のものと一致する受信エンベロープをスキップします。プレゼンスのメタチャネルのメッセージは例外です - 読み取りパスが統一されるよう、各hubは、クロスプロセスのビューの中に自分自身のイベントを必要とします。

バックエンドのディスパッチは、トレイトオブジェクトではなくenumベースです: hubは、sea-streamerのソケットアダプタからの具体的な `SeaProducer` / `SeaConsumer` を保持し、これは、コンパイルされたすべてのバックエンドにわたるenumです。publishの呼び出し箇所に `dyn` のオーバーヘッドはありません。

### クロスプロセスのプレゼンス

`SeaStreamerBroadcastHub` は、プレゼンスの状態をプロセスをまたいで自動的にレプリケートします。各インスタンスは、構築時にUUIDの `instance_id` を持ちます。`track_member` / `untrack_member` は、予約された `__presence__` メタチャネルへ `PresenceEvent` を発行します。各プロセスは、自分のコンシューマータスクによって更新される `cross_process_view` を保持します。`list_members` は、統合されたビュー（ローカルとリモートを均一に）を返します。

生存確認: 各プロセスは、ハートビートとして、`ttl / 6`（デフォルトの60秒のTTLでは10秒）ごとに自分のメンバーを再発行します。古びたエントリ - `last_seen` がTTLを超えたメンバー - は、`ttl / 2` ごとに刈り取られます。これは、`MemberRemoved` を発行できなかったプロセスのクラッシュを処理します。

## no-pong時のクローズ

ブロードキャストのルートは、素の `ws!` ルートと同じWebSocketのハートビートに参加します。フレームワークは、`WsConfig::ping_interval`（デフォルト30秒）ごとにPingを送信します。コネクションが `max_missed_pings` 回連続する間隔（デフォルト2）の内にPongで応答しなかった場合、フレームワークはコード1011でクローズします。

```rust
use std::time::Duration;
use suprnova::ws::WsConfig;

let config = WsConfig {
    ping_interval: Duration::from_secs(15),
    max_missed_pings: 3,
    ..WsConfig::default()
};
```

`ping_interval` を下げると、より高いベースラインのトラフィックというコストを払って、死んだコネクションをより速く検知します。`max_missed_pings: 1` は、最初の見逃したPongの後に閉じます - ネットワークの不調が稀で、可能な限り速い、死んだコネクションのクリーンアップが欲しいときにのみこれを使ってください。`max_missed_pings: usize::MAX` は、no-pong時のクローズを完全に無効化します。

## 本番デプロイ

ブロードキャストのルートは、あなたのHTTPルートと同じhyperのリスナー上でアップグレードされたHTTPコネクションです。TLSの終端は、[WebSocketの章](websockets.md#production-deployment)で説明されているのとまったく同じように、上流で行われます。その章のnginxとCaddyの設定は、変わらずに適用されます - `/ws/broadcast` のパスをカバーするよう、それらを拡張してください。

アクティブなWebSocketハンドラのタスク（ブロードキャストのコネクションを含む）は、フレームワークの `WS_TASKS` のセットの中で追跡され、グレースフルシャットダウンの際にドレインされます。そのため、実行中のイベント配信は、プロセスが終了する前に完了します。

## ブロードキャストのテスト

`RecordingBroadcastHub` は、Laravelの `Broadcast::fake()` に相当するSuprnovaの概念です - 生きた購読者への配信を続けたまま、発行されたすべてのエンベロープを記録する `BroadcastHub` です。テストの中で `InMemoryBroadcastHub` の代わりにこれを束縛し、先に購読することなく、何がブロードキャストされたかをアサートしてください:

```rust
use std::sync::Arc;
use suprnova::broadcasting::{BroadcastHub, RecordingBroadcastHub};
use suprnova::container::App;

#[tokio::test]
async fn shipping_an_order_broadcasts_to_the_user_channel() {
    let hub = Arc::new(RecordingBroadcastHub::new());
    App::bind::<dyn BroadcastHub>(Arc::clone(&hub) as Arc<dyn BroadcastHub>);

    // ... 発行するコードを実行する（直接、あるいはディスパッチされた Broadcastable を介して）...

    hub.assert_broadcast("orders.42", "OrderShipped");
    assert_eq!(hub.count(), 1);
}
```

| ヘルパー                         | アサートすること                                              |
|--------------------------------|----------------------------------------------------------|
| `assert_broadcast(ch, ev)`     | `ch` 上に、イベント名 `ev` を持つエンベロープが少なくとも1つある       |
| `assert_nothing_broadcast()`   | 何も発行されなかった                                    |
| `broadcasts()`                 | `Vec<BroadcastEnvelope>` - 記録されたすべてのエンベロープ       |
| `count()`                      | 記録されたエンベロープの総数                                 |

（通信に到達したものではなく）`Broadcastable` な*イベント*がそもそもディスパッチされたかをアサートするには、`EventFacade::fake()` がそのイベント自体を記録します - [イベント](events.md#testing--eventfacadefake)を参照してください。

## Laravel 対応リファレンス

| Laravel | Suprnova |
|---------|----------|
| `Broadcast::channel('name', fn(...))` | `Channel` トレイトのimpl + `registry.register(...)` |
| `Broadcast::channel('orders.{id}', ...)` | `fn name() -> "orders.{id}"`、`ChannelParams` の中のパラメータ |
| `PrivateChannel`（インターフェース） | `PrivateChannel` マーカートレイト + `authorize` をオーバーライド |
| `PresenceChannel`（インターフェース） | `PresenceChannel` + `Channel::presence_info` をオーバーライド |
| `ShouldBroadcast`（インターフェース） | `Broadcastable` トレイト |
| `broadcastOn()` | `broadcast_on(&self) -> Vec<String>` |
| `broadcastAs()` | `broadcast_event_name(&self) -> &'static str` |
| `broadcastWith()` | `broadcast_with(&self) -> Option<Value>` |
| `broadcastWhen()` | `broadcast_when(&self) -> bool` |
| `toOthers()` | `broadcast_to_others(&self) -> bool` |
| `Broadcast::fake()` | `dyn BroadcastHub` として束縛された `RecordingBroadcastHub` |
| `assertBroadcasted` | `RecordingBroadcastHub::assert_broadcast(channel, event)` |
| Pusher / Reverb / Ably ドライバー | `InMemoryBroadcastHub`（単一プロセス）または `SeaStreamerBroadcastHub`（クロスプロセス: Redis / Kafka / file / stdio） |
| Echoクライアントライブラリ | 出荷されていない - 今のところ、JSONエンベロープのプロトコルをブラウザから手作業で配線する |

## リファレンス

| シンボル | 目的 |
|--------|---------|
| `suprnova::broadcasting::Channel` | Channelトレイト。`name()`（必須）、`authorize`、`authorize_publish`、`presence_info` をオーバーライドする。 |
| `suprnova::broadcasting::ChannelParams` | パラメータ化された `name()` からキャプチャされた値。`get(key) -> Option<&str>`。固定名では空。 |
| `suprnova::broadcasting::PrivateChannel` | `authorize` をオーバーライドする `Channel` へのマーカートレイト。必須のメソッドはない。 |
| `suprnova::broadcasting::PresenceChannel` | `async fn member_info(req, params) -> Result<Value, FrameworkError>`。`Channel::presence_info` のオーバーライドを要求する。 |
| `suprnova::broadcasting::ChannelRegistry` | 登録されているすべてのチャネルを保持する。コンテナの中で `Arc<ChannelRegistry>` として束縛され、`BroadcastingWsHandler` によって解決される。 |
| `suprnova::broadcasting::Broadcastable` | `Event + Serialize` に対するトレイト。必須: `broadcast_on()`。省略可能: `broadcast_event_name`、`broadcast_with`、`broadcast_when`、`broadcast_to_others`。 |
| `suprnova::broadcasting::BroadcastHub` | Hubトレイト。`subscribe`、`publish`、`subscriber_count`、プレゼンスのtrack/untrack/list。 |
| `suprnova::broadcasting::InMemoryBroadcastHub` | デフォルトのプロセス内hub。外部依存なし。Publishは無条件に `Ok` を返す。 |
| `suprnova::broadcasting::RecordingBroadcastHub` | テスト用のダブル。あらゆるpublishを記録する。生きた購読者への配信は継続する。 |
| `suprnova::broadcasting::BroadcastEnvelope` | 発行された1つのイベント: `channel`、`event`、`data`、`except`。`new(ch, ev, data)` ビルダー。ディスパッチごとの除外のための `.with_except(socket_id)`。 |
| `suprnova::broadcasting::ClientFrame` / `ServerFrame` | JSONエンベロープの通信上の型。`ServerFrame::Lagged { channel, skipped }` は、チャネルごとのリングバッファのオーバーフローを表面化させる。 |
| `suprnova::broadcasting::BroadcastingWsHandler` | フレームワークの再利用可能な `WebSocketHandler`。コンストラクタ: `BroadcastingWsHandler::new(hub, registry)`。`ws!()` へ渡す。 |
| `suprnova::broadcasting::fanout::SeaStreamerBroadcastHub` | `broadcasting-fanout` の裏にあるクロスプロセスのhub。`new(uri, stream_key)`、`new_with_presence_ttl(uri, key, ttl)`、`new_loopback(uri, key)`。 |
| `EventFacade::broadcast::<E>(hub)` | `E` のためのイベント → hub のブリッジを登録する。起動時に、`Broadcastable` ごとに一度呼ぶ。 |
| `EventFacade::dispatch(event)` | プロセス内のリスナーを発火させ、かつ、`E::broadcast_on()` が返すすべてのチャネルでhubへ発行する。 |
| `WsRouteDef::config(WsConfig)` | ルートごとのWS設定の上書き。`.middleware(M)` とどちらの順序でも合成できる。 |
| `WsRouteDef::middleware(M)` | ルートごとのミドルウェアチェーン。非2xxレスポンスはアップグレードをショートサーキットする。 |
| `WsConfig::generous()` | 信頼済みフィード用のファクトリー: 64 MiBのメッセージ / 16 MiBのフレーム、他のフィールドは変わらない。公開ルートで使っては**ならない**。 |

## 次のステップ

- [WebSocket](websockets.md) - 下層のプリミティブ、`WsSocket`、`OriginPolicy`
- [イベント](events.md) - `EventFacade`、フェイルファスト対ベストエフォートのディスパッチ
- [Server-Sent イベント](sse.md) - Upgradeのハンドシェイクを伴わない一方向プッシュ
- [通知](notifications.md) - `BroadcastChannel` の通知ドライバー
- [Web プッシュ](web-push.md) - オフラインのユーザーへの、サーバー起点のプッシュ通知
