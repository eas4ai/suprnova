# Web プッシュ

Web プッシュは、あなたのサイトが閉じられていても、ブラウザへ短いメッセージを届けます - Service Workerが起動し、ペイロードを復号し、OSレベルの通知を表示します。Suprnovaは、このプロトコルをエンドツーエンドで出荷しています: VAPIDキーの生成、AES128GCMによるペイロード暗号化、HTTPトランスポート、そして、メールやデータベースへ送るのと同じ `Notification` がプッシュとしても届くようにする、通知サブシステムに組み込まれる `WebPushChannel` です。

開いたWebSocketなしでリアルタイムにユーザーへ知らせたいときは、これに手を伸ばしてください - 注文の発送、フレンドリクエスト、メンション、残高の更新などです。ユーザーがサイトを閉じたデスクトップブラウザにいる場合、web プッシュだけが彼らに届く唯一の仕組みです。ユーザーがサイトにいる場合は、通常[ブロードキャスト](broadcasting.md)がより良い選択です。

このAPIは、デフォルトで有効になっている `web-push` というCargoのフィーチャーの裏にあります。`default-features = false` を使うアプリケーションは、`web-push` を明示的に有効にしなければなりません。

## 4つの部品

Web プッシュは、メールやデータベースよりも可動部分が多くなっています。それは、仕様（[RFC 8030](https://datatracker.ietf.org/doc/html/rfc8030) + [RFC 8291](https://datatracker.ietf.org/doc/html/rfc8291) + [RFC 8292](https://datatracker.ietf.org/doc/html/rfc8292)）が、アイデンティティ、暗号化、トランスポートを3つの契約に分けているからです:

| 部品 | それが何か |
|---|---|
| `VapidKey` / `VapidSigner` | あなたのサーバーが名乗る通りの存在であることを証明するJWTに署名するための、P-256 ECDSAの鍵ペア |
| `WebPushClient` | ペイロードを暗号化し、VAPID JWTに署名し、購読のエンドポイントへPOSTするHTTPクライアント |
| `WebPushChannel` | `Notification` を `WebPushClient::send` の呼び出しへ変換する、通知サブシステムのアダプタ |
| `SubscriptionInfo` | ユーザーが購読したときにブラウザが渡してくる、不透明な（`endpoint`、`p256dh`、`auth`）の三つ組 - あなたはそれを保存するだけで、生成することはない |

下位の3つの層 - `VapidKey`、`WebPushClient`、暗号化されたPOST - は `suprnova::web_push` から再エクスポートされているため、アプリケーションが下層の `suprnova-web-push` クレートに直接依存する必要は決してありません。

## VAPIDの鍵ペアを生成する

Web プッシュは、プッシュサービスが不正な送信者をレート制限したり連絡したりできるようにするために、VAPID（Voluntary Application Server Identification）を使います。アプリケーションごとに1つのP-256鍵ペアが必要です。公開鍵はあなたのフロントエンドに入り、ブラウザがあなたのサーバーへ購読を固定できるようにします。秘密鍵はサーバー側に留まり、JWTに署名します。

一度だけ生成し、永続化し、それをずっと使い続けてください:

```rust
use suprnova::VapidKey;

let key = VapidKey::generate();

// PEMを、耐久性のある場所に保存する - シークレットマネージャー、デプロイ
// パイプラインがマウントするファイル、env-vars-as-filesのボリュームなど。
// 既存のすべての購読を無効化せずに、これを再生成することは決してできない。
let pem = key.to_pem()?;
std::fs::write("vapid_private.pem", &pem)?;

// フロントエンドは、base64url-no-paddingの非圧縮公開鍵を必要とする。
// これをあなたのJSへ渡し、`pushManager.subscribe()` が `applicationServerKey`
// として使えるようにする。
println!("PUBLIC_VAPID_KEY={}", key.public_key_uncompressed_b64url());
```

起動時に、保存済みのPEMを読み込みます:

```rust
use suprnova::{VapidKey, VapidSigner};

let pem = std::fs::read_to_string("vapid_private.pem")?;
let key = VapidKey::from_pem(&pem)?;
let signer = VapidSigner::new(key);
```

`VapidSigner` はJWTを生成しますが、何も送信しません - これは純粋に署名のプリミティブです。次の層がこれをラップします。

## WebPushClientを構築する

`WebPushClient` は、HTTP側のプリミティブです: シグナーと連絡先URI（「不正な振る舞いをした場合に、プッシュサービスがあなたに連絡できる方法」）を渡すと、`send` メソッドがペイロードを暗号化し、JWTに署名し、購読のエンドポイントへPOSTするオブジェクトが返ってきます。

```rust
use std::sync::Arc;
use suprnova::{VapidKey, VapidSigner, WebPushClient};

let signer = VapidSigner::new(VapidKey::from_pem(&pem)?);

// subjectは、RFC 8292 §2.1に従い、mailto: のURIかhttps: のURLでなければ
// ならない。それ以外は構築時に拒否されるため、設定を誤ったデプロイは、
// 最初の失敗したディスパッチの後にサイレントに失敗するのではなく、起動時にフェイルファストする。
let client = WebPushClient::new(signer, "mailto:ops@example.org")?;

let client = Arc::new(client);
```

なぜ `Arc<WebPushClient>` なのか。`WebPushClient` は `VapidSigner` をラップし、それは秘密の `ES256KeyPair` をラップしています。これらのどれも `Clone` ではありません - 秘密鍵はカジュアルに複製されるべきではないからです - そして、チャネルの登録ごとに新しいシグナーを構築してしまうと、同じアプリケーションに対してN個の独立したVAPIDアイデンティティを持つことになってしまいます。`Arc` でラップすることで、1つの署名済みアイデンティティが、あらゆる登録とあらゆる並行の配信を支えられるようになります。

### エンドポイントポリシー

購読のエンドポイントは、ユーザー由来のデータです: ユーザーが購読すると、ブラウザは遠隔のプッシュサービスからそのURLを受け取り、あなたのサーバーは、ブラウザが返してきたものをそのまま保存します。悪意を持って保存された購読は、HTTP POSTを到達可能な任意の場所へ向けることができ、プッシュの送信者をSSRFの踏み台にしてしまいます。

`WebPushClient` は、デフォルトで `EndpointPolicy::Strict` になります:

- スキームは `https` でなければならない
- ホストは、IPリテラルではなく、名前付きのドメインでなければならない
- クラウドメタデータのホスト名と、RFC 2606で予約されたTLD（`.localhost`、`.local`、`.internal`、`.test`、`.example`、`.invalid`）は拒否される

これは、実際のプッシュサービス（FCM、Mozilla Autopush、Appleの `web.push.apple.com`）を壊すことなく、明白なSSRFの探索をブロックします。

`wiremock` のモックサーバーに対するローカルな統合テストのためには、オプトアウトする必要があります:

```rust
use suprnova::{EndpointPolicy, WebPushClient};

let client = WebPushClient::new(signer, "mailto:test@example.org")?
    .with_endpoint_policy(EndpointPolicy::AllowAny);
```

本番環境で `AllowAny` を使わないでください。この厳格なチェックは、改ざんされた購読テーブルが武器化されるのを防ぐために存在しています。

### カスタムトランスポート

`WebPushClient::new` は、リクエストごとに30秒のタイムアウトを適用します。異なるトランスポートポリシーが必要な場合 - 企業のプロキシ、ピン留めされたTLS、より短いタイムアウトなど - は、`reqwest::Client` を構築し、`WebPushClient::with_client` を使ってください:

```rust
use reqwest::Client;
use std::time::Duration;
use suprnova::WebPushClient;

let http = Client::builder()
    .timeout(Duration::from_secs(10))
    .build()?;

let client = WebPushClient::with_client(http, signer, "mailto:ops@example.org")?;
```

## WebPushChannelを通知に配線する

生の `WebPushClient::send` は動作します - しかし、Suprnovaで実際にプッシュ通知を送る方法は、[通知](notifications.md)サブシステムを経由することです。`Notification` は、自分の `channels()` の中で `vec!["webpush"]` を宣言し、`Notifiable` な受信者は `route_for("webpush")` からJSONエンコードされた `SubscriptionInfo` を返し、束縛済みの `NotificationDispatcher` がファンアウトを行います。

```rust
use std::sync::Arc;
use suprnova::{
    NotificationDispatcher, WebPushChannel, WebPushClient,
    notifications::set_dispatcher,
};

let client: Arc<WebPushClient> = Arc::new(
    WebPushClient::new(signer, "mailto:ops@example.org")?
);

// ttl_secs: プッシュサービスが未配信のメッセージをどれだけ保持するか。
// 緊急でない通知には、86_400（24時間）が妥当なデフォルト。「今すぐ動け」
// というアラートで、古びたメッセージがメッセージなしより悪い場合は、60に落とす。
let webpush = Arc::new(WebPushChannel::new(client, 86_400));

let dispatcher = NotificationDispatcher::new()
    .register_channel(webpush);

set_dispatcher(Arc::new(dispatcher))?;
```

`register_channel` は、チャネルの `name()` に対して最後の書き込みが勝ちます。そのため、テストは、本番のバインディングに影響を与えることなく、スタブを差し替えられます。

## 通知を定義する

プッシュ向けの通知は、他のあらゆるSuprnovaの通知と同じ形です - `channels()` の中で `"webpush"` を宣言し、届けたいJSONを何であれ `data()` に入れてください:

```rust
use serde::{Deserialize, Serialize};
use suprnova::Notification;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct OrderShipped {
    pub order_id: i64,
    pub tracking_url: String,
}

impl Notification for OrderShipped {
    fn notification_name() -> &'static str {
        "OrderShipped"
    }

    fn channels(&self) -> Vec<&'static str> {
        vec!["webpush"]
    }

    fn data(&self) -> serde_json::Value {
        serde_json::json!({
            "title":   "Your order has shipped",
            "body":    format!("Track order #{}", self.order_id),
            "url":     self.tracking_url,
        })
    }
}
```

`data()` のJSONは、あなたのService Workerが受け取るものです。安定した形を選び、フロントエンドのためにそれを文書化してください - Suprnovaはそれを強制しません。通知のUIはフロントエンドの関心事だからです。

## 受信者をルーティングする

`Notifiable` は、自分がサポートする各チャネルについて、ルートを返します。Web プッシュの場合、そのルートはJSONエンコードされた `SubscriptionInfo` です - ブラウザが `PushSubscription.toJSON()` を介して生成したものそのものであり、そのまま保存されます:

```rust
use suprnova::Notifiable;

pub struct User {
    pub id: i64,
    pub push_subscription_json: Option<String>,
}

impl Notifiable for User {
    fn route_for(&self, channel: &str) -> Option<String> {
        match channel {
            "webpush" => self.push_subscription_json.clone(),
            _ => None,
        }
    }
}
```

`None` を返すと、ディスパッチャーはそのチャネルを無音でスキップします - プッシュを購読していないが、それでもメールを受け取るユーザーに便利です。

## 送信する

同期的に:

```rust
use suprnova::Notify;

let user = User::find(42).await?.unwrap();
Notify::send(&user, &OrderShipped {
    order_id: 1234,
    tracking_url: "https://ship.example.org/o/1234".into(),
}).await?;
```

キューに入れる - キューに入れる時点で購読のルートを事前に解決するため、ワーカーはユーザーを再読み込みする必要がありません:

```rust
Notify::queue(&user, OrderShipped {
    order_id: 1234,
    tracking_url: "https://ship.example.org/o/1234".into(),
}).await?;
```

`Notify::queue` が動作するためには、ワーカーがJSONペイロードを型付きの通知へ再構築できるよう、起動時に通知のファクトリーを登録してください:

```rust
suprnova::notifications::register_notification_factory::<OrderShipped>()?;
suprnova::queue::worker::register_job::<suprnova::SendNotificationJob>();
```

裏側では、キューに入れられたディスパッチは、`(notification_name, payload, per_channel_routes,channels)` を運ぶ `SendNotificationJob` を構築します。ワーカーは通知を再構成し、束縛済みのディスパッチャー上で名前によって `WebPushChannel` をルックアップし、`deliver(route,&notification)` を呼び出します - 同期的な `Notify::send` と同じコードパスです。

## ブラウザ側

SuprnovaはJavaScriptのSDKを出荷していません - ブラウザ側は、プレーンなWeb Push APIです。あなたのフロントエンドが実装する必要のあるフローは次のとおりです:

1. Service Workerを登録する。
2. ユーザーに許可を求める。
3. `pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: <your VAPID public key> })` を介して購読する。
4. `subscription.toJSON()` を、それをユーザーの行に保存するSuprnovaのエンドポイントへPOSTする。

```js
// Service Workerの登録（アプリのエントリポイントのどこかで）
const registration = await navigator.serviceWorker.register('/sw.js');

if (Notification.permission === 'default') {
    await Notification.requestPermission();
}

if (Notification.permission === 'granted') {
    const subscription = await registration.pushManager.subscribe({
        userVisibleOnly: true,
        applicationServerKey: window.PUBLIC_VAPID_KEY,
    });

    await fetch('/api/push/subscribe', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(subscription.toJSON()),
    });
}
```

あなたのSuprnovaのエンドポイントはそのJSONを受け取り、形を検証し、ユーザーに保存します - その文字列はあなたのサーバーにとって不透明ですが、ブラウザが生成したものと完全に同じJSONでなければなりません（`SubscriptionInfo` 型は、後でそれをパースするために `Deserialize` を使います）:

```rust
use suprnova::{Auth, Request, Response, SubscriptionInfo, attrs, json_response};

pub async fn subscribe(req: Request) -> Response {
    let user_id = Auth::id().expect("auth middleware");

    let (_parts, bytes) = match req.body_bytes().await {
        Ok(b) => b,
        Err(e) => return json_response!({ "error": e.to_string() }).map(|r| r.status(400)),
    };
    let raw = match std::str::from_utf8(&bytes) {
        Ok(s) => s.to_string(),
        Err(_) => return json_response!({ "error": "body not utf-8" }).map(|r| r.status(400)),
    };

    // 形を検証するためにパースする - endpoint、keys.p256dh、keys.auth。
    // パースに失敗した場合、ブラウザは不正な形のものを渡してきている。
    let sub: SubscriptionInfo = match serde_json::from_str(&raw) {
        Ok(s) => s,
        Err(e) => return json_response!({ "error": e.to_string() }).map(|r| r.status(400)),
    };

    // `raw` をそのまま永続化する - それが、ディスパッチ時にWebPushChannelが
    // serde_json::from_str へ渡す、まさにその文字列である。
    User::query()
        .db_where_op("id", "=", user_id)
        .update_all(attrs! { push_subscription_json: raw })
        .await
        .unwrap();

    json_response!({ "ok": true, "endpoint": sub.endpoint })
}
```

Service Workerは、プッシュのペイロードを復号し、通知をレンダリングします:

```js
// /sw.js
self.addEventListener('push', (event) => {
    const data = event.data.json();
    event.waitUntil(
        self.registration.showNotification(data.title, {
            body: data.body,
            data: { url: data.url },
        }),
    );
});

self.addEventListener('notificationclick', (event) => {
    event.notification.close();
    event.waitUntil(clients.openWindow(event.notification.data.url));
});
```

## ペイロードの上限

Web プッシュの仕様は、暗号化された各ペイロードを合計4096バイトに制限しています。Suprnovaは、暗号化の時点で3992バイト（上限から、AES128GCMの暗号化オーバーヘッドである約85バイトを引いたもの）より大きい平文を拒否します。そのため、失敗はプッシュサービスからの413ではなく、あなたのコードの中で表面化します。シリアライズされた `data()` がその上限を超える `Notification` は、チャネルの `deliver` から `WebPushError::Encryption` を返します。

それより大きいもの - 長いメッセージ本文、サムネイルなど - については、クリック時にService WorkerがfetchするURLを運ぶ、短い通知を送ってください。これは、より速く（数KBのペイロードに暗号化がかからない）、より柔軟です（fetchは、あなたが望む任意の形を返せます）。

## 失効した購読

プッシュサービスが404または410を返したとき、その購読は失効しています - ユーザーがブラウザをアンインストールした、許可を取り消した、あるいはストレージをクリアした、といった場合です。`WebPushChannel` は、これを致命的でない警告として扱います:

```text
WARN webpush subscription gone (404/410); caller should remove
     channel=webpush endpoint=https://fcm.googleapis.com/fcm/send/abc
```

ディスパッチは `Ok(())` を返します。なぜなら、その通知は終端状態に達したからです - リトライすべき受信者がもういません。あなたのアプリケーションは、その警告に対して行動することが期待されています: ログから `endpoint` をパースするか、あるいは `WebPushError` を介して分類する `NotificationFailed` のリスナーをフックし、購読の行を削除してください。Suprnovaはその警告を出荷しますが、あなたの代わりに購読テーブルを自動で刈り取ることはしません。

## リトライとRetry-After

プッシュサービスが一時的な5xx、408、または429を返したとき、下層の `WebPushError::PushServiceRejected` は、パース済みの `Retry-After` のヒントを運びます（delta-seconds形式のみ - HTTP-date形式は `None` を返します）:

```rust
use suprnova::WebPushError;

match client.send(&sub, payload, ContentEncoding::Aes128Gcm, 60).await {
    Ok(_) => (),
    Err(e) if e.is_retryable() => {
        let wait = e.retry_after().unwrap_or(Duration::from_secs(30));
        tokio::time::sleep(wait).await;
        // ...再試行するか、遅延を伴ってキューへ戻す
    }
    Err(WebPushError::SubscriptionGone) => {
        // 購読を削除する
    }
    Err(e) => return Err(e.into()),
}
```

`Retry-After` のヒントは24時間に制限されているため、悪意のあるサーバーが、ワーカーを何年にもわたるsleepに追い込むことはできません。

`Notify::queue` を使う場合、キュー自身のリトライ / バックオフが適用されます - `WebPushChannel::deliver` から伝播する `WebPushError` は、ジョブのエラーとして表面化し、エンベロープは、そのジョブのバックオフポリシーに従って再キューを処理します。`Retry-After` のヒントはログに記録されますが、（まだ）キューの遅延計算にはフィードバックされません。それが必要な場合は、ヒントされた遅延で再キューする `NotificationFailed` のリスナーをフックしてください。

## テレメトリ

通知のディスパッチャーは、そのファンアウトを、通知名とチャネル数がタグ付けされた `notification.dispatch` というinfoスパンでラップします。成功した配信はそれぞれ `NotificationSent` イベントを発します。失敗は、チャネル名、ルート、エラー文字列を運ぶ `NotificationFailed` を発します。これらのいずれも、他のフレームワークのイベントを配線するのと同じ方法で、あなたのメトリクス / ログのパイプラインへ配線してください - [イベント](events.md)を参照してください。

失効した購読は、`channel="webpush"`、エンドポイント、通知名を伴う構造化された `WARN` を発します。それが、自動化された購読クリーンアップジョブのためにスクレイプすべき信号です。

### Suprnovaが異なる設計を選んだ理由

Laravelの `WebPush` ドライバーは、コミュニティパッケージ（`laravel-notification-channels/webpush`）です - コアには含まれず、別個にバージョン管理され、ORMについて独自の意見を持っています。Suprnovaは、Web プッシュをフレームワークに焼き込んでいます。なぜなら、このプロトコルは十分に定義されており、暗号化されたHTTP POSTは、サードパーティの抽象化でラップするには小さすぎる契約だからです。通知サブシステムは、この表面を均一に保ちます: メールやデータベースへ送るのと同じ `Notification` が、プッシュとしても届き、ドライバーのマトリクスも、別個の設定ツリーもありません。

私たちは、厳格なエンドポイントポリシーもデフォルトで表面化させています。Laravelのコミュニティパッケージは、SSRF対策をアプリケーションに委ねています。私たちは、「エンドポイントはユーザーのデータから来ている」ことが、あらゆるWeb プッシュの購読の形であるという立場を取り、安全なデフォルトは、あなたのコードではなく、フレームワークに属すると考えています。

リトライの分類（`is_retryable`、`retry_after`）は、キュー層のマジックな定数テーブルとしてではなく、`WebPushError` 上の型付きメソッドとして公開されています。キューは、それでもリトライポリシーを所有します - エラーは、リトライが成功しうるかどうかと、どれだけ待つべきかを伝え、キューは、再度デキューするかどうか、そしていつ行うかを決めます。この2つを分離することで、あなたのカスタムのリトライ戦略（指数バックオフ、ジッター付き、上限付き）は、Web プッシュを特別扱いする必要がなくなります。

## テスト

`wiremock` サーバーを立て、`EndpointPolicy::AllowAny` で `WebPushClient` をそれへ向け、それが受け取るリクエストに対してアサートしてください:

```rust
use std::sync::Arc;
use suprnova::{
    EndpointPolicy, NotificationDispatcher, Notify, VapidKey, VapidSigner,
    WebPushChannel, WebPushClient,
    notifications::set_dispatcher,
};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn order_shipped_pushes() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/push"))
        .respond_with(ResponseTemplate::new(201))
        .mount(&server)
        .await;

    let signer = VapidSigner::new(VapidKey::generate());
    let client = Arc::new(
        WebPushClient::new(signer, "mailto:test@example.org")
            .unwrap()
            .with_endpoint_policy(EndpointPolicy::AllowAny),
    );
    let channel = Arc::new(WebPushChannel::new(client, 60));

    let dispatcher = NotificationDispatcher::new().register_channel(channel);
    set_dispatcher(Arc::new(dispatcher)).unwrap();

    let user = test_user_with_subscription(&server.uri()).await;
    Notify::send(&user, &OrderShipped {
        order_id: 1,
        tracking_url: "https://ship.example.org/o/1".into(),
    }).await.unwrap();
    // server.received_requests() には、これで暗号化されたPOSTが含まれている。
}
```

暗号化されたバイトを気にしないエンドツーエンドのテストには、`Notify::fake()`（[通知](notifications.md)で扱っています）が、チャネルを実行せずにディスパッチをキャプチャします - より速く、モックサーバーも、暗号化のラウンドトリップも不要です。

## リファレンス

- プリミティブ: `suprnova::VapidKey`、`suprnova::VapidSigner`、`suprnova::VapidClaims`
- クライアント: `suprnova::WebPushClient`、`suprnova::EndpointPolicy`、`suprnova::PushResponse`、`suprnova::SubscriptionInfo`
- エラー: `suprnova::WebPushError` - `.is_retryable()`、`.retry_after()`、`WebPushError::SubscriptionGone`
- エンコーディング: `suprnova::ContentEncoding`（Aes128Gcm。3992バイトの平文上限）
- チャネル: `suprnova::WebPushChannel`
- ファサード: `suprnova::Notify`
- キュージョブ: `suprnova::SendNotificationJob`
- ファクトリー登録: `suprnova::notifications::register_notification_factory`

## 次のステップ

- [通知](notifications.md) - `WebPushChannel` が組み込まれる、マルチチャネルのディスパッチャー
- [メール](mail.md) - プッシュを持たないユーザーのための、メールチャネルの対応物
- [ブロードキャスト](broadcasting.md) - サイトにいるユーザーのための、リアルタイム配信
- [キュー](queues.md) - `Notify::queue` が `SendNotificationJob` をどう支えているか
- [イベント](events.md) - 失効した購読のクリーンアップを駆動するための、`NotificationSent` / `NotificationFailed` のリスニング
