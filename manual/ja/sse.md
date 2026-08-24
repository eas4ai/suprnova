# Server-Sent イベント

Server-Sent イベント（SSE）は、サーバーからブラウザへの最小限の一方向プッシュチャネルです。ブラウザは `EventSource(url)` を開き、サーバーは `text/event-stream` レスポンスを開いたままにして、イベントが起きるたびにフレーム化されたイベントをプッシュします。WebSocketのハンドシェイクも、permessage-deflateも、フレーミング用のライブラリも不要です - 必要なのは、[WHATWG `EventSource`](https://html.spec.whatwg.org/multipage/server-sent-events.html)仕様に従った、空行で終端される `data:`、`event:`、`id:`、`retry:` の行だけです。

SuprnovaのSSEプリミティブは、ストリーミングボディの経路に組み込まれています: `Stream<Item = SseEvent>` を構築し、それを `HttpResponse::sse(...)` に渡せば、コネクション管理、フレーミング、ヘッダー、パニックの分離をフレームワークが引き受けます。コネクションは、生成側のストリームが終わるか、クライアントが切断するまで開いたままになります。

## SSEとWebSocketのどちらを使うべきか

| 特性 | SSE | WebSocket |
|----------|-----|------------|
| 方向 | サーバー → ブラウザ | 双方向 |
| トランスポート | プレーンなHTTP/1.1またはHTTP/2 | アップグレード限定 |
| 再接続 | `retry:` と `Last-Event-ID` による自動 | 手動 |
| プロキシ / CDN | 長時間のHTTPレスポンスを許すものなら何でも動く | 明示的なUpgradeサポートが必要になることが多い |
| ブラウザAPI | `EventSource`（組み込み） | `WebSocket`（組み込み） |
| バイナリフレーム | テキストのみ（UTF-8） | テキストまたはバイナリ |
| タブごとのコネクション上限 | 6（HTTP/1.1） / 無制限（HTTP/2） | 無制限 |

サーバーからクライアントへのプッシュだけが必要なとき（アクティビティフィード、通知、ログの末尾表示、AIのストリーミングなど）はSSEに手を伸ばしてください。双方向の通信やバイナリフレームが必要なときは[WebSocket](websockets.md)に手を伸ばしてください。

## クイックスタート

```rust
use futures::StreamExt;
use suprnova::{HttpResponse, Request, Response, sse::SseEvent};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

pub async fn stream_ticks(_req: Request) -> Response {
    let (tx, rx) = mpsc::channel::<SseEvent>(16);
    tokio::spawn(async move {
        for i in 0..10 {
            let evt = SseEvent::data(format!("tick {i}"))
                .with_event("tick")
                .with_id(i.to_string());
            if tx.send(evt).await.is_err() {
                break; // クライアントが切断した
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    });
    Ok(HttpResponse::sse(ReceiverStream::new(rx)))
}
```

1件のtickに対する通信上の出力:

```text
event: tick
id: 0
data: tick 0

```

ブラウザはこれをパースし、`evt.data === "tick 0"` かつ `evt.lastEventId === "0"` を伴う `tick` イベントを発火します。

## `SseEvent` API

`SseEvent` は、ストリームへプッシュする型です。これには2つの種類があります。

* **Frame** - 省略可能な `event` / `id` / `retry` と複数行の `data` ペイロードを持つ、通常のイベントです。[`SseEvent::data`](#コンストラクタ)、`SseEvent::json`、`SseEvent::error` を介して構築します。
* **Comment** - レスポンス上にしか存在しないキープアライブです（`:\n\n` または `: <text>\n\n`）。`SseEvent::comment(text)` または `SseEvent::keep_alive()` を介して構築します。ブラウザは仕様によりコメントを無視します。コネクションを通過するバイト列こそが、アイドル状態のプロキシやロードバランサーにコネクションを閉じさせないようにしているものです。

### コンストラクタ

| コンストラクタ | 生成するもの | 用途 |
|-------------|----------|-----|
| `SseEvent::data(text)` | `data:` 行だけを持つFrame | 最小限のイベント |
| `SseEvent::json(event, &payload)` | `event:` + JSONの `data:` を持つFrame | 95%のケース - クライアント側の `JSON.parse(evt.data)` |
| `SseEvent::error(message)` | `event: error` を持つFrame | ドメインレベルのエラーイベント。ブラウザがトランスポート障害時に発する接続レベルの `error` とは別物 |
| `SseEvent::comment(text)` | Comment | オペレーターがログの中で見つけられるマーカー付きのキープアライブ |
| `SseEvent::keep_alive()` | 空のComment（`:\n\n`） | 最小バイト数の正規のハートビート |

### ビルダー

| ビルダー | 効果 | `Comment` の場合 |
|---------|--------|--------------|
| `.with_event(name)` | `event:` フィールドを設定する | 無音で何もしない |
| `.with_id(id)` | `id:` フィールドを設定する - 再開のセマンティクスに必要 | 無音で何もしない |
| `.with_retry(Duration)` | `retry:` フィールド（ms）を設定する。仕様上、`Duration::ZERO` は「即座に再接続する」ことを意味する | 無音で何もしない |
| `.try_with_event(name)` | 失敗しうるバリアント - [セキュリティ契約](#セキュリティ契約)を参照 | `Ok(self)` のまま変わらない |
| `.try_with_id(id)` | `with_id` の失敗しうるバリアント | `Ok(self)` のまま変わらない |

`Comment` に対するビルダーが意図的に無音で何もしないのは、通信上の形式には「イベント名付きのコメント」を表現する手段がないからです。誤用は、生成側を驚かせてイベントをFrameへ変換してしまうのではなく、無音のままにされます。

### アクセッサー

| メソッド | 戻り値 |
|--------|---------|
| `.event()` | `Option<&str>` - 設定されていればイベント名 |
| `.id()` | `Option<&str>` - 設定されていればlast-event-id |
| `.retry()` | `Option<Duration>` - 設定されていれば再接続の遅延 |
| `.payload()` | `&str` - `data:` ペイロード（`Comment` の場合は `""`） |
| `.is_comment()` | `bool` |
| `.comment_text()` | `Option<&str>` - Commentであれば、そのコメントテキスト |

### 通信上の形式へのエンコード

`SseEvent::to_wire()` は、ボディストリームに渡せる状態の `Bytes` へとイベントをシリアライズします:

**Frame:**

```text
event: <event>\n   （Someの場合のみ）
id: <id>\n         （Someの場合のみ）
retry: <ms>\n      （Someの場合のみ）
data: <line>\n     （ペイロード中の1行につき1つ、\r/\r\n の正規化後）
\n                 （終端 - 仕様により必須）
```

**Comment:**

```text
: <line>\n         （コメントテキスト中の1行につき1つ。空行の場合は `:\n`）
\n                 （書き出しの境界）
```

## セキュリティ契約

SSEの通信上の形式は、CR / LF / NULをフィールドの終端記号として使い、エスケープ機構を持ちません。サニタイズせずにユーザー入力を `event:` や `id:` に到達させる生成側は、フィールドインジェクションの脆弱性を露呈させてしまいます - `"legit\ndata: injected"` という値は、通信上に2つの `data:` フィールドを生成してしまい、`"legit\n\nevent: spoofed"` は現在のイベントを終端し、新しいイベントを開始してしまいます。

Suprnovaの `to_wire()` は、2つの層で防御します。

* **`event:` と `id:` のフィールド値** - すべてのCR / LF / NULは、シリアライズ時に取り除かれます。取り除きが起きるたびに、構造化された `WARN` が発火します: `target: "suprnova::sse"`、`field = "event"|"id"`。この警告は値そのものをログに記録することは決してありません - それらのバイトは、構造上、攻撃者に制御されているからです。
* **`data:` とコメントテキスト** - `\r\n` と単独の `\r` は、分割の前に `\n` へ正規化されるため、ペイロードに `\r` を埋め込む生成側であっても、受信側のパーサーがパース時に `data:` / `event:` / `id:` フィールドを合成してしまうことはありません。NULは、対応する `WARN` を伴って、コメントテキストから取り除かれます。

サイレントに取り除くのではなく、不正な入力に対して**フェイルファスト**したい場合は、`try_with_*` の兄弟に手を伸ばしてください:

```rust
use suprnova::{Response, sse::SseEvent};

let evt = SseEvent::data("hello")
    .try_with_event(&user_supplied_event)?     // returns Err on CR/LF/NUL
    .try_with_id(&user_supplied_id)?;
```

返される `FrameworkError::validation(field, ...)` はフィールド名を示しますが、値をエコーバックすることは**ありません** - そのため、クライアントに表面化する400は、安全にログへ記録できます。

## キープアライブとプロキシのアイドルタイムアウト

長寿命のSSEコネクションは、デフォルトでは無音です。ほとんどの本番デプロイは、リソースを解放するためにアイドル状態のコネクションを閉じる、プロキシ / ロードバランサー / CDNの背後に置かれています。

* nginxのデフォルト: 60秒
* AWS ALBのデフォルト: 60秒
* Cloudflareのデフォルト: 100秒

15〜30秒ごとの `keep_alive()` コメントは、ブラウザへ `message` イベントをディスパッチすることなく、これらすべてを通してコネクションを生かし続けます。最小バイト数の形（`:\n\n`）は、ペイロードを何も送らずにプロキシの書き込みバッファを書き出すのに十分です。

```rust
use std::time::Duration;
use futures::StreamExt;
use suprnova::sse::SseEvent;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

let (tx, rx) = mpsc::channel::<SseEvent>(16);

// ハートビートタスク - イベントの生成側とは独立している。
let hb_tx = tx.clone();
tokio::spawn(async move {
    let mut ticker = tokio::time::interval(Duration::from_secs(20));
    loop {
        ticker.tick().await;
        if hb_tx.send(SseEvent::keep_alive()).await.is_err() {
            break; // クライアントがいなくなった
        }
    }
});

// イベントの生成側 ... 発生するたびにフレームを `tx` へ送る。
```

## 切断後の再開（`Last-Event-ID`）

ブラウザの `EventSource` がコネクションを切断すると、それは自動的に再接続し、それまでに見た最新の `id:` を、新しいリクエストの `Last-Event-ID` ヘッダーとして送ります。各イベントに `.with_id(...)` でタグを付け、再開リクエストでそのヘッダーを読み取ってください:

```rust
use futures::StreamExt;
use suprnova::{HttpResponse, Request, Response, sse::{self, SseEvent}};

pub async fn stream_from_resume(req: Request) -> Response {
    let resume_from: u64 = sse::last_event_id(&req)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    // `resume_from + 1` 以降から生成側のストリームを構築する。クロージャは
    // 自前の実行中カウンターを所有しているため、変更はストリームの内側にとどまる。
    let stream = futures::stream::iter(events_since(resume_from))
        .scan(resume_from + 1, |next_id, payload| {
            let id = *next_id;
            *next_id += 1;
            futures::future::ready(Some((id, payload)))
        })
        .map(|(id, payload)| {
            SseEvent::json("activity", &payload)
                .expect("payload is a Serialize value")
                .with_id(id.to_string())
        });

    Ok(HttpResponse::sse(stream))
}
```

`sse::last_event_id(&Request) -> Option<String>` は、ヘッダーが存在しない場合、またはその値にNULバイトが含まれる場合に `None` を返します（WHATWG仕様により、NULはlast-event-idを無効にし、ブラウザのパーサーはそれを捨てます）。返される `String` は、それ以外の場合は不透明なユーザー入力です - 使用する前に、あなた自身のカーソル / シーケンス / オフセットとしてパースしてください。

## ドメインレベルのエラー

`SseEvent::error("...")` は、慣例的な `event: error\ndata: <msg>\n\n` の形を生成します。購読者は、ブラウザがトランスポート障害時に発する接続レベルの `error` とは別に、これに対するリスナーを登録できます:

```js
const es = new EventSource("/stream");

// 接続 / トランスポートのエラー（`data` なし）。
es.onerror = (evt) => console.warn("transport error", evt);

// SseEvent::error(...) が発するドメインレベルのエラー。
es.addEventListener("error", (evt) => console.error("server-side:", evt.data));
```

`Stream<Item = Result<T, E>>` を `Stream<Item = SseEvent>` へマッピングするときのイディオマティックなパターンは、`map(|r| match r { Ok(x) => SseEvent::json(...), Err(e) => SseEvent::error(...) })` です - コンシューマー側のエラーマッピングは生成側の手に留まり、フレームワークがデフォルトの形を発明する必要は決してありません。

## 1つのストリームを多くの購読者へブロードキャストする

多くのSSE購読者へのファンアウトは、すでに[ブロードキャストのサブシステム](broadcasting.md)がカバーしています: `BroadcastHub` のチャネルを購読し、`tokio_stream::wrappers::BroadcastStream` + `.map(...)` を使って `broadcast::Receiver` を `SseEvent` のストリームへ適応させてください。コネクションごとに専用のレシーバーが与えられます。低速コンシューマーへのポリシー（購読者が遅れたときの `Lagged(n)` エラー）はハブが処理し、それをクライアントへどう表面化させるかはあなたが決めます。

実際に動作するドッグフード例が `app/src/controllers/sse_example.rs` にあり、これを約25行で実装しています:

```rust
use futures::StreamExt;
use std::sync::Arc;
use suprnova::broadcasting::BroadcastHub;
use suprnova::container::App;
use suprnova::{HttpResponse, Request, Response, sse::SseEvent};
use tokio_stream::wrappers::BroadcastStream;

pub async fn stream(_req: Request) -> Response {
    let hub: Arc<dyn BroadcastHub> = App::make::<dyn BroadcastHub>()
        .expect("BroadcastHub not bootstrapped");
    let rx = hub.subscribe("user_registered");

    let stream = BroadcastStream::new(rx).map(|result| match result {
        Ok(envelope) => SseEvent::json("user.registered", &envelope.data)
            .unwrap_or_else(|_| {
                SseEvent::data(envelope.data.to_string())
                    .with_event("user.registered")
            }),
        Err(tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(n)) => {
            SseEvent::data(n.to_string()).with_event("lagged")
        }
    });

    Ok(HttpResponse::sse(stream))
}
```

`lagged` イベントは、クライアントが完全な再取得と再開をトリガーできるようにします - コネクションは、遅延の間も開いたままです。

## `event_stream` と `stream_json`

`HttpResponse::sse` はフレーミングを完全に制御します。すべての `SseEvent` を自分で構築します。二つの高水準の兄弟が一般的な形をカバーします:

```rust
use suprnova::sse::{EndSignal, StreamedEvent};
use suprnova::{HttpResponse, Request, Response};
use tokio::sync::mpsc;

pub async fn progress(_req: Request) -> Response {
    let (tx, rx) = mpsc::channel::<StreamedEvent>(16);
    tokio::spawn(async move {
        for pct in [25, 50, 75, 100] {
            let evt = StreamedEvent::message(pct).unwrap();
            if tx.send(evt).await.is_err() {
                break; // client disconnected
            }
        }
    });
    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    Ok(HttpResponse::event_stream(stream, EndSignal::default()))
}
```

`StreamedEvent::message(data)` は `event` のデフォルトを `"update"` にします。これは `useEventStream` がそのままでリッスンする名前です。`StreamedEvent::named(event, data)` は、同じ接続で複数の論理チャネルをファンアウトするproducer用にそれを上書きします。`data` は、素の文字列では引用符なし、それ以外はJSONエンコードされてワイヤへ届きます。`event_stream` の `end: EndSignal` 引数は、ストリーム終了後に送られる終端フレームを制御します。`EndSignal::default()` は `event: update\ndata: </stream>\n\n` を送ります（Laravel自身のデフォルトで、`useEventStream` の `endSignal` オプションが検査するものです）。`EndSignal::None` は省略し、`EndSignal::text(...)` / `EndSignal::Event(...)` はカスタマイズします。これはSuprnovaにおける `ResponseFactory::eventStream($callback, $headers, $endStreamWith)` です。

`HttpResponse::stream_json(stream)` - Laravelの `ResponseFactory::streamJson` / `StreamedJsonResponse` - は任意の `Stream<Item = impl Serialize>` を受け取り、コレクション全体を先にバッファリングする代わりに、1つの増分構築されたJSON配列（`Content-Type: application/json`）としてflushします。ワイヤ上のバイトは正確に `[item,item,...]` です。レスポンス全体を連結すれば、任意のJSONパーサーでデシリアライズできます。

## React / Vue / Svelteから消費する

[`@laravel/stream-{react,vue,svelte}`](https://github.com/laravel/stream) パッケージがこのワイヤ契約のクライアント側を所有します。Suprnovaは独自のものを出荷せず、それらを対象にします:

| Hook | 通信先 | Suprnovaビルダー |
|---|---|---|
| `useEventStream(url, options)` | `EventSource`（GET、ブラウザ管理の再接続） | `HttpResponse::event_stream` |
| `useStream(url, options)` | `fetch`（POST、手動の `ReadableStream` 読み取りループ） | `HttpResponse::stream_bytes` |
| `useJsonStream(url, options)` | `useStream` と同じで、完全にバッファされた結果を `JSON.parse` する | `HttpResponse::stream_json` |

```tsx
import { useEventStream, useJsonStream } from "@laravel/stream-react";

const { message } = useEventStream("/progress");          // against an event_stream endpoint
const { data, send } = useJsonStream<Order[]>("/export"); // against a stream_json endpoint
```

`useStream`/`useJsonStream` は、Suprnovaが他のリクエストヘッダーと同じように読む二つのヘッダーを伴ってPOSTします。`X-STREAM-ID`（hookがクライアント側で生成する、認証を行わない素の相関ID）と、[CSRF保護](csrf.md)がすでに期待するのと同じく `<meta name="csrf-token">` から読む `X-CSRF-TOKEN` です。`useEventStream` はどちらも送りません。`EventSource` はカスタムリクエストヘッダーをまったく設定できず、素のブラウザGETだからです。

## 本番環境のセットアップ

### レスポンスヘッダー

`HttpResponse::sse(...)` は、必要なヘッダーを代わりに設定します。

| ヘッダー | 値 | 理由 |
|--------|-------|-----|
| `Content-Type` | `text/event-stream` | 仕様で定義されている。ブラウザの `EventSource` がこれを要求する |
| `Cache-Control` | `no-cache` | 中間者がストリームをキャッシュすることを止める |
| `Connection` | `keep-alive` | HTTP/1.1の長寿命レスポンス |
| `X-Accel-Buffering` | `no` | nginxのプロキシバッファリングを無効化する - イベントは即座に書き出される。nginx以外ではno-op |

### 再接続のチューニング

ブラウザのデフォルトの再接続遅延は3秒です。それを上書きするには、ストリームの先頭で一度だけ `retry:` フィールドを送ってください:

```rust
let preamble = SseEvent::data("ready").with_retry(Duration::from_secs(5));
```

`Duration::ZERO` は仕様上有効であり（「即座に再接続する」ことを意味します）、補正されることなく、そのまま送出されます。本番のストリームでは、5〜15秒のリトライが、速い復旧と、リージョン障害の間にサーバーを叩き過ぎないことの間で、バランスを取ります。

### Suprnovaが異なる設計を選んだ理由

Laravelは、SSEを `Response` 上の場当たり的なヘルパーとして出荷しています: `Response::eventStream(fn () => ...)` はジェネレーターをyieldするクロージャを受け取り、yieldされた各値を `data:` の行としてフレーム化します。これは `event:` / `id:` / `retry:` をファーストクラスのフィールドとしてモデル化せず、組み込みのキープアライブプリミティブを持たず、通信上に余分なフィールドをインジェクトしてしまうであろう値をサニタイズしません。

Suprnovaは、SSEを場当たり的なヘルパーではなく、本物のサブシステムとして扱います。

- `SseEvent` は、失敗しうる（`try_with_*`）ビルダーと失敗しない（`with_*`）ビルダー、明確に区別された `Frame` と `Comment` の種類、そしてすべての単一行フィールドに対する文書化されたサニタイズ契約を持つ、型付きの値です。
- `HttpResponse::sse(stream)` は、他のあらゆる長寿命レスポンスが使うのと同じ `stream_bytes` のボディパイプラインに組み込まれるため、SSEはフレームワークの残りの部分と、1つのキャンセル、ヘッダー、パニック分離の経路を共有します。
- 生成側は、任意の `Stream<Item = SseEvent>` を組み合わせられます - `tokio::sync::mpsc`、`tokio::sync::broadcast`、`futures::stream::iter`、あるいは[BroadcastHub](broadcasting.md)のファンアウトアダプタです。これらのどれも、フレームワークのエスケープハッチを必要としません。
- `Last-Event-ID` のリーダー（`sse::last_event_id`）とWHATWGのNUL-dropルールは、標準で用意されています。そのため、切断後の再開は、アプリごとの独自のヘッダーユーティリティではなく、1回のパース呼び出しの先にあります。

## リファレンス

| シンボル | 目的 |
|--------|---------|
| `suprnova::sse::SseEvent` | SSEストリームの、送出可能な1つの要素です。2つの種類があります - `Frame`（省略可能な `event` / `id` / `retry` + `data` を持つイベント）と `Comment`（キープアライブ）です。 |
| `SseEvent::data(text)` | `data:` の行だけを持つFrameを構築します。 |
| `SseEvent::json(event, &payload)` | ペイロードが `serde_json` でシリアライズされた `payload` であるFrameを構築します。`event:` を `event` に設定します。`Result<Self, serde_json::Error>` を返します。 |
| `SseEvent::error(message)` | `event: error` と、渡されたメッセージを `data` として持つFrameを構築します。 |
| `SseEvent::comment(text)` | コメントのみのイベント（`: <text>\n\n`）を構築します。ブラウザには見えません。プロキシを起こしたままにします。 |
| `SseEvent::keep_alive()` | 空のコメント `:\n\n` の省略形です。最小バイト数のハートビートです。 |
| `.with_event(name)` / `.with_id(id)` / `.with_retry(Duration)` | `Frame` に対する失敗しないビルダーです。`Comment` に対しては無音で何もしません。`to_wire()` の時点で、構造化された `WARN` を伴ってCR / LF / NULを取り除きます。 |
| `.try_with_event(name)` / `.try_with_id(id)` | 失敗しうる兄弟です - CR / LF / NULがあれば `Err(FrameworkError::validation(...))` を返します。値がユーザー入力から流れてくる場合で、無音での取り除きではなく4xxが欲しいときに使ってください。 |
| `.event()` / `.id()` / `.retry()` / `.payload()` / `.is_comment()` / `.comment_text()` | アクセッサーです。`payload()` という名前は、`data` コンストラクタとの衝突を避けるために付けられています。 |
| `SseEvent::to_wire()` | SSEの通信上の形式で `Bytes` へシリアライズします。テストやアダプタがレスポンスビルダーを経由せずにエンコードできるよう、公開されています。 |
| `suprnova::sse::last_event_id(&Request) -> Option<String>` | `Last-Event-ID` ヘッダーを読み取ります。存在しない場合、または値にNULバイトが含まれる場合は `None` を返します（WHATWGは無効なidを捨てます）。 |
| `suprnova::sse::last_event_id_from_value(Option<&str>)` | 同じバリデーション契約を公開する、純粋なヘルパーです - `Request` を構築せずに単体テストできます。 |
| `HttpResponse::sse(stream)` | 任意の `Stream<Item = SseEvent> + Send + Sync + 'static` から、ストリーミングレスポンスを構築します。`Content-Type`、`Cache-Control`、`Connection`、`X-Accel-Buffering` を設定します。 |
| `suprnova::sse::StreamedEvent` | `event_stream` へpushされる1項目です - `{ event: String, data: serde_json::Value }`。 |
| `StreamedEvent::message(data)` / `StreamedEvent::named(event, data)` | デフォルトの `"update"` 名、または明示した名前で構築します。どちらも `Result<Self, serde_json::Error>` を返します。 |
| `suprnova::sse::EndSignal` | producer終了後に `event_stream` が送る終端フレームです - `None` / `Message(String)` / `Event(StreamedEvent)`。`Default` は `text("</stream>")` です。 |
| `HttpResponse::event_stream(stream, end)` | 任意の `Stream<Item = StreamedEvent> + Send + Sync + 'static` から `event_stream` レスポンスを構築します。`sse` 上に構築されます。 |
| `HttpResponse::stream_json(stream)` | 任意の `Stream<Item = impl Serialize> + Send + Sync + 'static` から `stream_json` レスポンスを構築します。`stream_bytes` 上に構築されます。 |

## 次のステップ

- [WebSocket](websockets.md) - 双方向やバイナリフレームが必要なときのための、もう1つの長寿命コネクションです。
- [ブロードキャスト](broadcasting.md) - WebSocketの購読者と共有される `BroadcastHub` のファンアウトです。
- [通知](notifications.md) - ストリーミングでないプッシュ配信（メール、データベース、ブロードキャスト）のためのチャネルドライバーです。
- [Web プッシュ](web-push.md) - `EventSource` が開いていないときにクライアントへ届く、サーバー起点のプッシュ通知です。
- [レスポンス](responses.md) - `HttpResponse` ビルダーの残りの表面です。
