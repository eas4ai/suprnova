# WebSocket

SuprnovaのWebSocketルートは、同じルーター内でHTTPルートと並んで存在します。パスとハンドラを登録すると、フレームワークはそのパスへの `Upgrade: websocket` リクエストを検知し、そのパスへのHTTP GETが走らせるのと同じミドルウェアチェーンを走らせ、RFC 6455のハンドシェイクを完了させ、型付きの `WsSocket` と元の `Request` を伴ってあなたのハンドラを呼び出します。別個のWebSocketサーバーは存在しません - コネクションは、あなたのHTTPトラフィックを処理しているのと同じhyperのリスナーからアップグレードされます。フレームワークは、spawnされたすべてのハンドラを、サーバーごとの `JoinSet` の中でも追跡します。そのため、グレースフルシャットダウンは、リスナーが終了する前に、実行中のコネクションをドレインします。

## クイックスタート

`EchoHandler` を追加し、`routes!` の中に登録します。

`src/ws/echo.rs`:

```rust
use async_trait::async_trait;
use suprnova::{FrameworkError, http::Request, ws::{WebSocketHandler, WsSocket}};

pub struct EchoHandler;

#[async_trait]
impl WebSocketHandler for EchoHandler {
    async fn handle(&self, mut socket: WsSocket, _req: Request) -> Result<(), FrameworkError> {
        while let Some(text) = socket.recv_text().await? {
            socket.send_text(format!("echo: {text}")).await?;
        }
        Ok(())
    }
}
```

`src/routes.rs`（`routes! { ... }` の中）:

```rust
ws!("/ws/echo", app_ws::echo::EchoHandler),
```

アプリを起動し、`wscat` で接続します:

```bash
cargo run --bin app
```

```text
$ wscat -c ws://localhost:3000/ws/echo
Connected (press CTRL+C to quit)
> hello
< echo: hello
> suprnova
< echo: suprnova
```

`recv_text()` が `Ok(None)` を返したときは、ピアがコネクションを閉じたということです。ループは終了し、ハンドラは `Ok(())` を返し、フレームワークはクリーンなClose(1000)フレームを送信します。

## アップグレードのライフサイクル

WebSocketのハンドシェイクは、`Upgrade: websocket` を伴うHTTP GETです。フレームがどれも流れる前に、フレームワークはそれに対して完全なリクエストパイプラインを走らせます:

1. **ルートマッチ。** ルーターはWSルートテーブルの中でパスを検索します。マッチしない場合、リクエストはHTTPのフォールバックへフォールスルーします。
2. **オリジンポリシー。** 設定済みの[`OriginPolicy`](#オリジンポリシー)が強制されます。違反はアップグレードなしでHTTP 403を返します。
3. **サブプロトコルのネゴシエーション。** ルートが `accepted_protocols` を持つ場合、重なり合う中でクライアントが最初に提示したトークンが、101レスポンスでエコーされます。
4. **ミドルウェアチェーン。** `RequestIdMiddleware` が最も外側で実行され、続いてグローバルに登録されたすべてのミドルウェア、続いてそのルートのルートごとのミドルウェアが実行されます。いずれかのミドルウェアからの非2xxレスポンスは、アップグレードをショートサーキットします - ピアはHTTPエラーを受け取り、WebSocketのフューチャーはきれいにドロップされます。
5. **ハンドシェイク。** `hyper_tungstenite::upgrade` は、`WebSocketStream` へと解決するフューチャーを生成します。
6. **ハンドラのディスパッチ。** （ミドルウェアによって書き換えられている可能性のある）`Request` と、新しく構築された `WsSocket` が、`WebSocketHandler::handle` へ渡されます。
7. **ハートビート + ハンドラ。** フレームワークは、コネクションごとのハートビートタスクをspawnし、リクエストIDを運ぶ `ws.connection` の `tracing` スパンの下で、ハンドラのフューチャーをawaitします。
8. **クローズのハンドシェイク。** `Ok(())` のときフレームワークはClose(1000)を送信します。`Err(_)` のときはClose(1011 "internal error")を送信します。コネクションの追跡されたタスクが完了として報告される前に、クローズフレームが通信上に書き出されるよう、フォワーダーがawaitされます。

戻り値のセマンティクスは、HTTPとは逆になっています: ボディが存在しません。`Ok(())` はクリーンな切断を意味します。`Err(_)` はログに記録され、ピアはClose(1011)を目にします。どちらにせよ、コネクションは終了します。

## `WsSocket` API

`WsSocket` は、フレームワークがあなたのハンドラへ渡す双方向のハンドルです。内部的には、下層のtungstenite ストリームは Sink + Stream の2つの半身へ分割されています: フォワーダータスクがシンクを所有し、mpscをドレインします。ハンドラ向けのsendメソッドは、そのmpscへenqueueします。ハンドラは、ストリーム側の半身から直接読み取ります。この分割により、フレームワークは、ハンドラのsend経路と競合することなく、フレーム（ハートビートのping、ブロードキャスターのファンアウト）をプッシュすることもできます。

### `send_text`

```rust
socket.send_text("hello").await?;
socket.send_text(format!("user {id} joined")).await?;
```

UTF-8のテキストフレームをenqueueします。コネクションがすでに閉じている場合にのみ `Err` を返します。

### `send_binary`

```rust
socket.send_binary(bytes).await?;
```

バイナリフレームをenqueueします。`Into<Vec<u8>>` であれば何でも受け付けます。`send_text` と同じエラーのセマンティクスです。

### `recv_text`

```rust
while let Some(text) = socket.recv_text().await? {
    // text: String
}
// Ok(None) はピアが閉じたことを意味する。
```

次のテキストメッセージを返します。テキストのみを扱うハンドラが気にする必要のないフレームの種類は、無音で捨てられます:

- `Message::Binary` - ピアのバイナリペイロード
- `Message::Ping` - ピア起点のping（tungstenite が自動的にpongを処理する）
- `Message::Pong` - フレームワークのハートビートに対するピアのpong応答（副作用として、未応答pingカウンターがゼロにリセットされる）
- `Message::Frame` - サーバー側のコンテキストから来る生のフレームバリアント。この層では想定されない

取り込まれて捨てられたフレームは失われます - 後から遡って見る方法はありません。ハンドラがバイナリフレームやクローズコードを観測する必要がある場合は、最初の読み取りから[`recv`](#recv)を使ってください。

### `recv`

```rust
use tokio_tungstenite::tungstenite::Message;

while let Some(msg) = socket.recv().await? {
    match msg {
        Message::Text(t)   => { /* ... */ }
        Message::Binary(b) => { /* ... */ }
        Message::Close(_)  => break,
        _                  => {}
    }
}
```

Binary、Ping、Pong、Closeを含む、あらゆる種類の次のメッセージを返します。`Pong` は、返される前に、副作用として未応答pingカウンターをリセットします。`Ok(None)` は、下層のストリームが終了したことを意味します。

### `close`

```rust
socket.close(1008, "policy violation").await?;
return Ok(());
```

クローズフレームをenqueueして戻ります。フォワーダーはそのフレームをシンクへ書き込み、シンクの `close()` を呼び出し、終了します。同じソケットへの後続のsendは、フォワーダーが失われているため `Err` を返します。`close` を呼び出した直後には、常に `Ok(())` を返してください。

`close` は、その引数を事前に、RFC 6455 §7.4 + §5.5.1に対して検証します:

- `code` は `CloseCode::is_allowed()` を満たさなければなりません。予約済みまたは無効なコード（1004、1005、1006、1015、1000未満のあらゆる値、4999超のあらゆる値）は `Err` で拒否され、**フレームは送信されません** - コネクションは開いたままとなり、呼び出し元は有効なコードで再試行できます。通常のクローズには1000を、定義済みの理由には1001-1013を、IANA登録済みのコードには3000-3999を、アプリケーション専用のコードには4000-4999を使ってください。
- `reason` は123バイトに制限されます（125バイトのコントロールフレーム上限から、2バイトのコード分を引いたもの）。それより長い理由は、何もenqueueせずに拒否されます。

### Suprnovaが異なる設計を選んだ理由

PHPのフレームワークは、WebSocketサポートを別個のプロセス（ratchet、soketi、pusher）として後付けします。SuprnovaのWebSocketルートは、あなたのHTTPルートと同じ `routes! { ... }` の中に存在し、同じhyperのリスナーによって処理され、同じグレースフルシャットダウンの経路でドレインされます。バイナリは1つ、設定は1つ、デプロイは1つです。長寿命のコネクションは、Tokioがそれを安価にするため、ファーストクラスです - フレームワークは、それについて弁明する必要がありません。

## パスパラメータ

WebSocketのルートは、HTTPルートと同じ `{param}` キャプチャ構文をサポートします。キャプチャされた値は、ハンドラへ渡される `Request` 上で利用できます。

```rust
// routes! の中で:
ws!("/ws/rooms/{id}", RoomHandler),
```

```rust
use async_trait::async_trait;
use suprnova::{FrameworkError, http::Request, ws::{WebSocketHandler, WsSocket}};

pub struct RoomHandler;

#[async_trait]
impl WebSocketHandler for RoomHandler {
    async fn handle(&self, mut socket: WsSocket, req: Request) -> Result<(), FrameworkError> {
        let room_id = req.param("id")?;
        socket.send_text(format!("joined room {room_id}")).await?;
        while let Some(text) = socket.recv_text().await? {
            socket.send_text(format!("[{room_id}] {text}")).await?;
        }
        Ok(())
    }
}
```

`req.param("id")` は `Result<&str, ParamError>` を返します。セグメントが欠けている場合、`?` は `FrameworkError::ParamError` を伝播させ、これによりハンドラは `Err` を返し、フレームワークはClose(1011)を送信します。実際には、ルートがマッチした時点でキャプチャは常に存在します - このエラー経路は、パラメータ名の打ち間違いに対する安全網です。

Express形式の `:id` セグメントも受け付けられ（`ws!("/ws/rooms/:id", h)`）、内部的にはmatchit形式へ変換されます。

完全な `Request` API - ヘッダー、クッキー、クエリ文字列、ピアのアドレス - については、[リクエストのドキュメント](requests.md)を参照してください。

## ルートごとのミドルウェア

`ws!` のエントリに `.middleware(M)` をチェーンしてください。複数のミドルウェアは左から右へ合成され、同じパスへのHTTPリクエストが走るのと同じ固定順序で実行されます: 最も外側が `RequestIdMiddleware`、続いてグローバルに登録されたすべてのミドルウェア、続いてルートごとのチェーン、そしてハンドラです。

```rust
ws!("/ws/private", PrivateHandler)
    .middleware(AuthMiddleware::new())
    .middleware(RateLimitMiddleware::connections_per_ip(100)),
```

いずれかのミドルウェアからの非2xxレスポンスは、アップグレードをショートサーキットします。ピアは、`X-Request-Id` が設定された拒否（401、403など）を受け取り、まだ起動していないWebSocketのフューチャーはきれいにドロップされ、ハンドラは決して呼び出されません。これは、トランスポートレベルのチェックのための正しい層です: そもそも誰がコネクションを開けるのか、コネクションはどこから来ているのか、アイデンティティごとに何本の同時コネクションが許されるのか。

ミドルウェアは、`next(modified_req)` を呼び出すことで、変更済みの `Request` に差し替えられます。終端は、チェーンが最終的に通したものを取り込み、それがハンドラの `Request` 引数として見えるものになります。アイデンティティを解決するミドルウェア（セッションのルックアップ、トークンのチェック）は、`Request` の拡張を介して結果を添付できます。ハンドラは、HTTPコントローラーと同じ方法でそれを読み返します。

`Router` 上に直接書くバリアント（`Router::ws`、`Router::ws_with_middleware`、`Router::ws_with_config`、`Router::ws_with_middleware_and_config`）は、マクロの外で `Router` を構築するコードのために、同じ表面をカバーします。それぞれに、重複または不正な形のパターンに対してパニックする代わりに `Err(FrameworkError)` を返す、失敗しうる `try_*` の兄弟があります。

### Suprnovaが異なる設計を選んだ理由

多くのエコシステムは、WebSocketのアップグレードでミドルウェアを省略する（Nodeの慣習）か、「WebSocketミドルウェア」のための別個の登録の儀式を強制する（.NET / Springの慣習）かのどちらかです。Suprnovaは、アップグレードを、それが実際にそうであるところのHTTP GETとして扱います: 同じチェーンが、同じ順序で、同じショートサーキットのセマンティクスで実行されます。学ぶべき第二の概念はありません - `AuthMiddleware`、`RateLimitMiddleware`、`RequestIdMiddleware`、`CorsMiddleware` はWSルート上でも動作します。なぜなら、それらはどんなルートでも動作するからです。オリジンの強制だけが唯一の追加のひねりであり、それは別個のミドルウェアではなく、`WsConfig` の特性です。

## 接続時の認証

ハンドラは、ミドルウェアによって書き換えられた `Request` を受け取ります。フレームワークの残りの部分との統合の度合いが高くなる順に、3つのパターンがうまく機能します:

**パターン1 - ハンドラ内でのインラインbearerトークン。** 最も単純です。認証ミドルウェアなしで動作します。`wscat`、ブラウザクライアント、ロードバランサーは、いずれもヘッダーをきれいに通します。

```rust
use async_trait::async_trait;
use suprnova::{FrameworkError, http::Request, ws::{WebSocketHandler, WsSocket}};

pub struct PrivateChatHandler;

#[async_trait]
impl WebSocketHandler for PrivateChatHandler {
    async fn handle(&self, mut socket: WsSocket, req: Request) -> Result<(), FrameworkError> {
        let Some(token) = req.header("authorization")
            .and_then(|v| v.strip_prefix("Bearer "))
        else {
            socket.close(1008, "missing bearer token").await?;
            return Ok(());
        };
        let Some(user_id) = verify_token(token).await else {
            socket.close(1008, "invalid bearer token").await?;
            return Ok(());
        };
        while let Some(text) = socket.recv_text().await? {
            socket.send_text(format!("[user {user_id}] {text}")).await?;
        }
        Ok(())
    }
}

async fn verify_token(_token: &str) -> Option<i64> { Some(42) }
```

**パターン2 - ルートミドルウェアでアップグレードをゲートする。** フレームが流れる前に、認可されていないオープンを拒否します。関心の分離がよりクリーンです。ハンドラは、認証済みのコネクションしか目にしません。

```rust
ws!("/ws/private", PrivateChatHandler)
    .middleware(AuthMiddleware::new()),
```

`AuthMiddleware` は、未認証のリクエストに対して401を返します。アップグレードは拒否レスポンスとともに中断され、ハンドラは決して呼び出されません。

**パターン3 - ミドルウェアによるゲート + ハンドラでの再読み取り。** ミドルウェアは、認可されていないオープンをショートサーキットします。続いてハンドラは、今まさに存在すると分かっている同じ資格情報（トークン、クッキーなど）を再読み取りし、どのユーザーが今接続したのかを識別します:

```rust
async fn handle(&self, mut socket: WsSocket, req: Request) -> Result<(), FrameworkError> {
    // ミドルウェアはすでにbearerを検査済みであり、有効だった場合にのみここへ到達する。
    let token = req.bearer_token().expect("auth middleware vetted bearer presence");
    let user_id = lookup_user_by_token(&token).await?;
    // ...
}
```

**パターン4 - ミドルウェアに認証させ、その結果を読み取る。** 認証ミドルウェアがすでにアップグレード時に実行されている場合に好まれます。それが解決したアイデンティティは、リクエスト自体に運ばれます:

```rust
async fn handle(&self, mut socket: WsSocket, req: Request) -> Result<(), FrameworkError> {
    let Some(user_id) = req.auth_user_id() else {
        socket.close(1008, "unauthenticated").await?;
        return Ok(());
    };
    // `user_id` はセッション/トークンのミドルウェアから来ており、クライアントが
    // フレームで送ってきたものからは来ていない。
    socket.send_text(format!("welcome, {user_id}")).await?;
    Ok(())
}
```

これが、プライベートなブロードキャストチャネルの `authorize` フックを意味のあるものにしている理由です: それは同じ `Request` を受け取るため、クライアントが選んだ値ではなく、サーバー側で導出されたアイデンティティに基づいてゲートできます。`auth_user_id` が存在する前は、チャネルには頼れるものが何もなく、明白な代替策 - 「正しく見えるトークンを購読フレームに載せている購読者は誰でも受け入れる」 - は、そもそもゲートになっていませんでした。

HTTPコントローラーの中で動作するスレッドローカルなアクセッサー - `session()`、`Auth::user()`、リクエストごとの `Context` バッグ - は、WebSocketハンドラの内側では、それでも**満たされません**。ミドルウェアチェーンのタスクローカルなスコープは、チェーンが戻る時点で巻き戻ります。ハンドラは、リクエストIDと解決済みの認証idだけを継承する、新しくspawnされたタスクの中で動きます。ハンドラが必要とするそれ以外のすべては、`Request` から直接読み取ってください（ヘッダー、`req.cookie("...")` によるクッキー、キャプチャされたパラメータ、`req.bearer_token()` によるbearerトークン）- これらは、ハンドラのタスクへも引き継がれます。

### Suprnovaが異なる設計を選んだ理由

Laravelは、ブロードキャストチャネルを別個のHTTPエンドポイント（`/broadcasting/auth`）経由で認可します。そのため、チャネルのコールバックは、完全なセッションが利用できる通常のリクエストの中で実行されます。Suprnovaは、代わりにアップグレードの最中にプロセス内で認可します - 1本のコネクション、2度目のラウンドトリップなし - つまり、アイデンティティは、再度ルックアップされるのではなく、spawnの境界を越えて明示的に運ばれなければなりません。

## `WsConfig`

`WsConfig` は、コネクションごとの振る舞いを制御します。デフォルトは、公開されたブラウザ向けのエンドポイントを念頭に置いています - アクティブなコネクションはそれぞれ、`max_message_size` の大きさのtungstenite バッファを確保するため、フレームワークはデフォルトを小さくしておき、より多くを必要とするルートには、明示的に上限を上げさせます。

| フィールド | デフォルト | 型 | 効果 |
|-----------------------|----------------|-----------------|--------|
| `ping_interval`       | 30s            | `Duration`      | フレームワークがコネクションを生かし続けるためにPingフレームを送る頻度。 |
| `max_message_size`    | 1 MiB          | `usize`         | 再構成後のメッセージサイズの最大値（バイト単位）。これを超えるメッセージはtungstenite によって拒否される。 |
| `max_frame_size`      | 64 KiB         | `usize`         | 単一のWebSocketフレームサイズの最大値（バイト単位）。 |
| `max_missed_pings`    | 2              | `usize`         | ハートビートがコード1011でコネクションを閉じるまでの、連続した未応答Pongの数。`usize::MAX` は強制を無効化する。 |
| `origin_policy`       | `SameOrigin`   | `OriginPolicy`  | アップグレード時に強制されるOriginヘッダーのチェック。[オリジンポリシー](#オリジンポリシー)を参照。 |
| `accepted_protocols`  | `vec![]`       | `Vec<String>`   | サーバーが受け入れる `Sec-WebSocket-Protocol` トークン。空はネゴシエーションなしを意味する。[サブプロトコル](#サブプロトコル)を参照。 |

用途別の推奨される上書き:

- **チャット / 通知 / カーソル位置** - デフォルトのままで問題ありません。あなたのLBのアイドルタイムアウトが厳しい場合は、`ping_interval` を5〜10秒に下げてください。
- **信頼済みの内部フィード**（サーバー間のファンアウト、一括エクスポート、大きなバイナリ転送） - `WsConfig::generous()` を出発点にしてください。これは、他のデフォルトを保ったまま、`max_message_size` を64 MiBへ、`max_frame_size` を16 MiBへ上げます。
- **特定の過大なペイロード**（256 MiBのオーディオファイルをアップロードする1つのルート） - フィールドを直接設定してください。それを必要としないルートに、大きい上限を適用しないでください。

この設定用の構造体は `Default` で構築可能であり、すべてのフィールドが公開されています:

```rust
use std::time::Duration;
use suprnova::ws::WsConfig;

let chat = WsConfig {
    ping_interval: Duration::from_secs(5),
    max_missed_pings: 1,
    ..Default::default()
};

let trusted = WsConfig::generous();
assert_eq!(trusted.max_message_size, 64 * 1024 * 1024);
assert_eq!(trusted.max_frame_size, 16 * 1024 * 1024);
```

この上書きは、`ws!` のエントリ上か、`Router::ws_with_config` 上のいずれかで、ルートごとに適用してください:

```rust
ws!("/ws/chat", ChatHandler).config(chat),
```

`WsConfig` は、ルート登録時に検証されます。ゼロの `ping_interval` やゼロの `max_missed_pings` はハートビートタスクを壊してしまうため、どちらも、最初のコネクションでパニックするのではなく、起動時に拒否されます。

### ハートビートとno-pong時のクローズ

アップグレードされた各コネクションについて、フレームワークは、`ping_interval` ごとに `Ping(b"")` を送信するハートビートタスクをspawnします。tickごとに、未応答pingカウンターが増加します。ピアのPongごとに、それはゼロにリセットされます。カウンターが `max_missed_pings` に達すると、ハートビートはClose(1011 "no pong response")を送信し、コネクションは終了します。強制を無効化するには、`max_missed_pings` を `usize::MAX` に設定してください（pingは流れ続けますが、コネクションはpongの欠落を理由に閉じられることは決してありません）。

最初のtickはタスク開始時に消費されるため、ピアは、最初のpingの前に、少なくとも1回分の完全な猶予期間を得られます。

## オリジンポリシー

ブラウザは、WebSocketのハンドシェイクにおいて、常に `Origin` ヘッダーを送信します。`fetch()` / `XMLHttpRequest` とは異なり、WebSocketのアップグレードはCSRFトークンのミドルウェアによって保護されません（ハンドシェイクはトークンを運びません）。そのため、同一オリジンの `Origin` チェックだけが、悪意のあるページと、ログイン済みユーザーのセッション上の特権的なWSエンドポイントとの間に立ちます。フレームワークは、`hyper_tungstenite::upgrade` が呼ばれる前に、設定済みのポリシーを強制します。違反は、アップグレードなしでHTTP 403を返します。

```rust
use suprnova::ws::{OriginPolicy, WsConfig};

let cfg = WsConfig {
    origin_policy: OriginPolicy::AllowList(vec![
        "https://app.example.com".into(),
        "https://admin.example.com".into(),
    ]),
    ..Default::default()
};
```

| バリアント | 振る舞い |
|--------------|----------|
| `SameOrigin`（デフォルト） | `Origin` のホスト（と、存在すればポート）が、リクエストの `Host` ヘッダーと一致するときにのみ許可する。`Origin` が欠けている場合は拒否される。スキームは比較されない（TLSは上流で終端するため、サーバーは公開されたスキームがhttpsかhttpかを確実には知りえない）。 |
| `AllowAny`   | チェックを省略する。ブラウザ以外のエンドポイント（サーバー間、ネイティブアプリ、テストモック）にのみ使うこと。 |
| `AllowList(Vec<String>)` | `Origin` が、渡されたオリジンのいずれかと（大文字小文字を区別せずに）完全に一致するときにのみ許可する。各エントリは、ブラウザが送るであろう完全な `scheme://host[:port]` の形。 |

ブラウザ以外のクライアント（CLIツール、サーバー、ネイティブアプリ）は、通常 `Origin` ヘッダーを送信しません。そのようなクライアントだけを扱うルートは `AllowAny` を使うべきであり、両方を扱うルートは、本番のフロントエンドのオリジンをすべて列挙した `AllowList` を使うべきです。

## サブプロトコル

WebSocketのサブプロトコルは、クライアントとサーバーがハンドシェイクの間に合意する、アプリケーションレベルのトークンです（例: `graphql-transport-ws`、`jsonrpc-2.0`）。参加するには `accepted_protocols` を設定してください:

```rust
use suprnova::ws::WsConfig;

let cfg = WsConfig {
    accepted_protocols: vec![
        "graphql-transport-ws".into(),
        "graphql-ws".into(),
    ],
    ..Default::default()
};
```

クライアントが `Sec-WebSocket-Protocol` を提示すると、フレームワークは、`accepted_protocols` と重なり合う、クライアントが提示した最初のトークン（RFC 6455 §4.2.2に従ったクライアントの優先順）を、大文字小文字を区別せずに選び、101レスポンスでそれをエコーします。クライアントがプロトコルを提示したのにどれも一致しなかった場合でも、アップグレードは `Sec-WebSocket-Protocol` ヘッダーなしで成功します - RFC 6455はその場合、ブラウザにクライアント側でコネクションを失敗させることを要求します。これは正しい振る舞いです（そのまま進んでしまうサーバーは、無音のまま間違ったプロトコルを話してしまうことになります）。

`accepted_protocols` が空の場合、ネゴシエーションは完全にスキップされます - アップグレードのレスポンスは `Sec-WebSocket-Protocol` を省略し、クライアントはデフォルトのプロトコル処理へフォールバックします。

## 本番デプロイ

フレームワークは、ハンドシェイクとフレームのI/Oを処理します。本番環境のために、フレームワーク側で追加の設定を行う必要はありません。

**TLSの終端は上流で行われます。** クライアントは、nginx、Caddy、またはクラウドのロードバランサー上の `wss://` へ接続します。プロキシがTLSを剥ぎ取り、プレーンな `ws://` をフレームワークへ転送します。フレームワークは、`rustls` のフィーチャーやTLS証明書を必要としません。

### nginx

```nginx
location /ws/ {
    proxy_pass http://127.0.0.1:3000;
    proxy_http_version 1.1;
    proxy_set_header Upgrade $http_upgrade;
    proxy_set_header Connection "Upgrade";
    proxy_set_header Host $host;
    proxy_set_header X-Real-IP $remote_addr;
    proxy_read_timeout 3600s;
    proxy_send_timeout 3600s;
}
```

`proxy_read_timeout` と `proxy_send_timeout` は、ハートビートの間のアイドルな間隔をカバーできるだけ十分長くなければなりません。デフォルトの30秒の `ping_interval` であれば、3600秒は余裕のある上限です。

### Caddy

```caddy
reverse_proxy /ws/* localhost:3000 {
    header_up Upgrade {http.request.header.Upgrade}
    header_up Connection "Upgrade"
}
```

Caddyは、プロキシする際に `Upgrade` / `Connection` を自動的に処理します。上記の明示的な `header_up` ディレクティブは、分かりやすさのためのものです。

### クラウドロードバランサー（AWS ALB、GCP GLB）

リスナールール上でWebSocketサポートを有効にしてください（AWS ALBは、ターゲットグループのプロトコルがHTTP/1.1で、スティッキーセッションがオフのとき、これを自動的に行います）。ロードバランサーのアイドルタイムアウトが、少なくとも `ping_interval` と同じ長さであることを確認してください。フレームワークのハートビートは通信を活性状態に保ちますが、LBは、その視点からアイドルに見えるコネクションを切断します。

## グレースフルシャットダウン

spawnされたすべてのWebSocketハンドラは、サーバーの `WS_TASKS` の `JoinSet` の中で追跡されます。`Ctrl-C` または外部からのシャットダウン信号を受けると、リスナーは新しいコネクションの受け入れを停止し、`Server::run` は、プロセスが終了する前にそのセットをドレインします。ハンドラのフューチャーは、クローズのハンドシェイクが書き出されるまで解決しません: ユーザーの `handle` が戻った後、フレームワークはフォワーダーをawaitし、コネクションのタスクが完了として報告される前に、最終的なClose(1000)またはClose(1011)フレームが通信上に書き出されるようにします。クリーンなシャットダウンでは、ピアはTCPリセットではなく、通常のクローズを目にします。

完了したハンドルは、サーバーの生存期間の間、機会があるたびに回収されます。そのため、長時間の運用の下でも `JoinSet` が無制限に増大することはありません。

## リファレンス

| シンボル | 目的 |
|---|---|
| `suprnova::ws::WebSocketHandler` | トレイト: `async fn handle(&self, socket: WsSocket, request: Request) -> Result<(), FrameworkError>`。`Send + Sync + 'static`。 |
| `suprnova::ws::WsSocket` | 双方向のハンドル。メソッド: `send_text`、`send_binary`、`recv_text`、`recv`、`close`。`close` は、コード + 理由の長さを事前に検証する。 |
| `suprnova::ws::WsConfig` | コネクションごとの設定。フィールド: `ping_interval`、`max_message_size`、`max_frame_size`、`max_missed_pings`、`origin_policy`、`accepted_protocols`。`Default` + `generous()` コンストラクタ。登録時に検証される。 |
| `suprnova::ws::OriginPolicy` | `SameOrigin`（デフォルト）、`AllowAny`、`AllowList(Vec<String>)`。アップグレード時に強制される。 |
| `ws!(path, Handler)` | `routes! { ... }` 用のマクロ形式。`.config(WsConfig)` と `.middleware(M)` をどちらの順序でもサポートする `WsRouteDef` を返す。 |
| `Router::ws(path, handler)` | 直接の登録。`Router` を返す。 |
| `Router::ws_with_config(path, handler, cfg)` | ルートごとの `WsConfig` の上書き。 |
| `Router::ws_with_middleware(path, handler, mws)` | ルートごとのミドルウェアリスト。 |
| `Router::ws_with_middleware_and_config(...)` | 両方。 |
| `Router::try_ws*` ファミリー | 失敗しうる兄弟 - パニックする代わりに、重複または不正な形のパターンに対して `Err(FrameworkError)` を返す。 |

## 次のステップ

- [ブロードキャスト](broadcasting.md) - チャネル、プレゼンス、`ws!` の上に構築された通信プロトコル
- [Server-Sent イベント](sse.md) - 厳格なプロキシの背後にあるブラウザのための一方向プッシュ
- [ルーティング](routing.md) - `routes!` と `ws!` が実際に何に展開されるか
- [ミドルウェア](middleware.md) - HTTPとWSを均一にゲートするミドルウェアを書くこと
- [リクエスト](requests.md) - あなたのハンドラが受け取る `Request` 上のヘッダー、クッキー、クエリ、拡張
