# HTTP クライアント

`Http` ファサードは、HTTPのアウトバウンド側です - Laravelの `Http::` ヘルパーに相当するRust版です。あなたのハンドラ、ジョブ、あるいはスケジュールされたタスクが、決済ゲートウェイ、ジオコーダー、webhookの宛先、Slackへのメッセージといった、誰か他者のAPIを呼ぶ必要があるときに、これに手を伸ばしてください。流れるようなビルダー、JSONの送受信、ジッターを伴うリトライ、そしてあなたが送ったものを記録する決定的なテストフェイクです。Laravelで使っていたのと同じ表面を、並列テストが互いのフェイクを見ないようにするタスクローカルな分離を伴って使えます。

```rust
use suprnova::Http;
use serde_json::json;

let resp = Http::post("https://api.stripe.com/v1/charges")
    .bearer_token(secret_key)
    .json(&json!({ "amount": 1000, "currency": "usd" }))
    .send()
    .await?;

let body: serde_json::Value = resp.json().await?;
```

その形はこうです。`Http::<verb>(url)` が `RequestBuilder` を返し、あなたはそこへ設定をチェーンしていき、`.send().await` が `ClientResponse` を返します。裏にあるクライアントは、rustlsのTLS、30秒のデフォルトタイムアウト、そして `suprnova/<version>` というユーザーエージェントを持つ、共有された1つの `reqwest::Client` です - 最初の呼び出しで遅延構築されます。

## 動詞

```rust
Http::get("https://api.example.com/users/42")
Http::post("https://api.example.com/users")
Http::put("https://api.example.com/users/42")
Http::patch("https://api.example.com/users/42")
Http::delete("https://api.example.com/users/42")
```

あらゆる動詞は `RequestBuilder` を返します。URLは、`&str`、`String`、`Cow<str>` など、あらゆる `impl Into<String>` にできます。URLを組み立てるヘルパーはこのファサードには出荷されていません - URLは自分でフォーマットするか、クエリ文字列用のクレートに手を伸ばしてください。

## ボディ

ボディを添付する方法は3つあります。それぞれが、以前に設定されたボディを置き換えます。

### JSON

```rust
use serde::Serialize;

#[derive(Serialize)]
struct CreateUser {
    name: String,
    email: String,
}

Http::post("https://api.example.com/users")
    .json(&CreateUser {
        name: "Ada".into(),
        email: "ada@example.com".into(),
    })
    .send()
    .await?;
```

`.json(&value)` は、`serde::Serialize` を実装するあらゆるものを受け付けます。実際に送信される `Content-Type` は、自動的に `application/json` になります。シリアライズが失敗した場合（例えば文字列でないキーを持つマップ）、ビルダーはそのエラーを記録し、`send()` は、無音で `null` のボディを送るのではなく、それを表面化させます。

### フォーム

```rust
Http::post("https://login.example.com/oauth/token")
    .form(&serde_json::json!({
        "grant_type": "client_credentials",
        "client_id": id,
        "client_secret": secret,
    }))
    .send()
    .await?;
```

`.form(&value)` は、その値を `application/x-www-form-urlencoded` としてシリアライズします。値はJSONオブジェクトへシリアライズされなければならず、そのキーがフォームのフィールドになります。`.json` と同じボディエラーの意味論です - シリアライズの失敗は `send().await?` を通じて表面化し、無音の空のボディになることは決してありません。

### 生バイト

```rust
use bytes::Bytes;

let payload: Bytes = compress(report)?;
Http::post("https://collector.example.com/ingest")
    .header("Content-Type", "application/octet-stream")
    .body(payload)
    .send()
    .await?;
```

`.body(bytes)` は、あらゆる `impl Into<Bytes>` を取ります。`Content-Type` ヘッダーはあなたの責任です - `.body` はそれを設定しません。

## ヘッダーと認証

```rust
Http::get("https://api.example.com/private")
    .header("X-Request-Id", request_id)
    .header("Accept", "application/vnd.api+json")
    .bearer_token(api_key)
    .send()
    .await?;
```

`.header(name, value)` は追加していきます - フレームワークは重複を排除しないため、同じ名前で2回呼ぶと2つのヘッダーが送られ、reqwestがHTTPの意味論に従ってそれらを結合します。よくある認証方式のための、2つのショートカットがあります。

- `.bearer_token(token)` - `Authorization: Bearer <token>` を設定します
- `.basic_auth(user, password)` - `Authorization: Basic <b64>` を設定します。`password` は `Option<&str>` であるため、`.basic_auth("api-key", None)` は、一部のプロバイダーが求める `api-key:` という形をエンコードします

## タイムアウト

共有クライアントは、30秒のデフォルトタイムアウトを持ちます。必要なときは、リクエストごとに上書きしてください。

```rust
use std::time::Duration;

Http::get("https://slow.example.com/report")
    .timeout(Duration::from_secs(120))
    .send()
    .await?;
```

`.timeout(dur)` は、この1回の呼び出しについて、接続と合計のリクエストタイムアウトの両方を上書きします。ビルダーには、個別の `connect_timeout` というノブはありません - 裏にあるreqwestクライアントは、1つの結合されたタイムアウトを使います。

## リダイレクト

共有クライアントは、デフォルトでリダイレクトに従います（reqwestの上限である10回まで）- これは、信頼できるエンドポイントを呼んでいて、それが `http → https` で応答したり、CDNのURLを渡してきたりする場合には正しい振る舞いです。

リクエストのURLが信頼できない入力の影響を受ける場合、そのデフォルトはサーバーサイドリクエストフォージェリ（SSRF）のベクタになります - 悪意のあるエンドポイントは、`Location` が内部のサービスやクラウドのメタデータアドレス（`http://169.254.169.254/…`）を指す `3xx` で応答でき、追従するクライアントはそれを追いかけてしまいます。そのようなリクエストに対しては、`.no_redirects()` でリダイレクトの追従を無効にしてください。

```rust
let resp = Http::get(user_supplied_url)
    .no_redirects()
    .send()
    .await?;

// 3xxは追従されるのではなく、そのまま返される。それを調べて、
// クライアントにLocationヘッダーを追いかけさせるのではなく拒否する。
if (300..400).contains(&resp.status()) {
    return Err(AppError::bad_request("refusing to follow a redirect"));
}
```

`.no_redirects()` は、リクエストを、追従しない別個のクライアントを経由させます - デフォルトのクライアント、そしてこれを呼ばないあらゆるリクエストは、変わりません。これは、web-pushの送信側が攻撃者に制御されたプッシュのエンドポイントに対してすでに適用している、リダイレクトの締め付けの、汎用クライアント版です。

## リトライ

`Http` は、フルジッターを伴う指数バックオフのリトライを出荷しています - AWSのレシピであり、Laravelが使っているのと同じものです。2つのバリアントがあり、べき等でないメソッドを再生する意思があるかどうかで区別されます。

### `.retry(max_attempts, base_backoff)` - べき等な場合のみ

```rust
use std::time::Duration;

let resp = Http::get("https://flaky.example.com/health")
    .retry(4, Duration::from_millis(200))
    .send()
    .await?;
```

`max_attempts` は最初の試行を含むため、`retry(4, ...)` は最初の試行の後、最大3回リトライします。試行 `n+1` の前の遅延は、`[0, base_backoff * 2^(n-1)]` の範囲の均一なランダム時間で、30秒でキャップされます。フルジッターであり、指数バックオフ + 固定スリープではないため、同じ障害をリトライする多数のワーカーが、サンダリングハードへ同期することはありません。

リクエストがリトライされるのは、次のときです。

- 送信が、レスポンスが到着する前に失敗する場合（connect / DNS / タイムアウト）、あるいは
- レスポンスのステータスが5xxである場合

4xxと2xx/3xxのレスポンスは、そのまま返されます。リトライを使い切った後は、最後のレスポンス（あるいは最後のエラー）が呼び出し元へ返されます。

`.retry()` の形は、`POST` や `PATCH` のリトライを拒否します - それらのメソッドはべき等ではなく、サーバーが書き込みをすでにコミットしていたのに、レスポンスが戻る途中で失われた場合、盲目的な再生は副作用を重複させてしまいます。POST/PATCHに対して `.retry()` を呼ぶことは、それでも機能します - それは単に「リクエストがサーバーに到達する前のコネクションエラーでリトライする」ことを意味するだけです。一度5xxが返ってくれば、1回の試行の後に呼び出し元へ返されます。

### `.retry_non_idempotent(...)` - POST/PATCHのためのオプトイン

```rust
Http::post("https://api.example.com/charges")
    .header("Idempotency-Key", idem_key)
    .retry_non_idempotent(3, Duration::from_millis(200))
    .send()
    .await?;
```

アップストリームが尊重するべき等性キーを渡していたり、あるいはそれ以外の方法でリクエストを再生しても安全にしていたりする場合は、`.retry_non_idempotent(...)` に切り替えて、POSTとPATCHを同じリトライの挙動へオプトインさせてください。リトライのルールは同一です - コネクションエラーと5xxのレスポンスはリトライされ、4xxと2xx/3xxはそのまま通過します。

### 503では`Retry-After`が尊重される

`503 Service Unavailable` に対して、フレームワークは `Retry-After` ヘッダーを尊重します - デルタ秒（`Retry-After: 30`）またはHTTP-date（`Retry-After: Tue, 15 Nov 1994 08:12:31 GMT`）のどちらの形式でもです。実際の待機時間は、ジッターの入ったバックオフと `Retry-After` のヒントのうち大きい方であり、それでも30秒でキャップされます。悪意のある、あるいは設定を誤ったサーバーが `Retry-After: 86400` を返しても、あなたのタスクを1日パークさせることはありません。

## レスポンスを読み取る

`ClientResponse` は、ステータス、ヘッダー、そして3つのボディ読み取りメソッドを公開します。各ボディメソッドはレスポンスを消費します。

```rust
let resp = Http::get("https://api.example.com/users/42").send().await?;

let status: u16 = resp.status();
let etag: Option<String> = resp.header("ETag");

// いずれか1つを選ぶ - どちらもレスポンスを消費する。
let user: User = resp.json().await?;
// let text: String = resp.text().await?;
// let bytes: Bytes = resp.bytes().await?;
```

`.header(name)` は大文字小文字を区別しません。`.json::<T>()` は `Result<T, FrameworkError>` を返し、デコードには `serde_json` を使います。`.text()` はUTF-8を強制し、ボディが有効なUTF-8でない場合は `FrameworkError` を表面化させます。

### レスポンスボディの上限

遅い、あるいは悪意のあるアップストリームは、そうでなければ、無制限のボディをメモリへストリーミングできてしまいます。それを防ぐため、バッファリングされるボディの読み取りはすべて上限を持ちます - デフォルトは25 MiBです。起動時にグローバルに上書きできます。

```rust
use suprnova::Http;

// bootstrapの中で、一度だけ。
Http::set_max_response_bytes(100 * 1024 * 1024); // 100 MiB
```

あるいは、1回の呼び出しが正当により大きなペイロードを扱う場合は、リクエストごとに。

```rust
let bytes = Http::get("https://example.com/big-export.json")
    .max_response_bytes(500 * 1024 * 1024) // 500 MiB
    .send()
    .await?
    .bytes()
    .await?;
```

上限を超える `Content-Length` を宣言するレスポンスは、ボディが何も読まれる前に拒否されます - ストリーミングのループも、`Content-Length` が欠けているか嘘をついている場合に備えて、実際のバイト数に対してもその上限を強制します。

## 逃げ道 - 生のreqwest

フレームワークは、よくあるケースをカバーします。私たちが公開していない何か - ストリーミングのボディ、マルチパートのアップロード、リダイレクトポリシーの検査、WebSocketのアップグレード - が必要なときは、`.into_inner()` を呼んで、裏にある `reqwest::Response` をアンラップしてください。

```rust
let resp = Http::get("https://example.com/big-stream").send().await?;
let raw: reqwest::Response = resp.into_inner()?;
let mut stream = raw.bytes_stream();
while let Some(chunk) = stream.next().await {
    process(chunk?);
}
```

`into_inner()` は、フェイクのレスポンスに対して呼ばれた場合 `Err(FrameworkError::internal(...))` を返します - その場合、裏にある `reqwest::Response` は存在しないからです。生のレスポンスを取り出した後は、レスポンスボディの上限もそれ以上適用されません - そこから先の読み取りは、あなた自身が管理します。

今のところ、送信側のマルチパートアップロードには、同じ逃げ道を通じて `reqwest::Client` へ直接落ちてください。需要のパターンが形になったら、将来のリリースで `.multipart(...)` ビルダーが追加されるかもしれません。

## `Http::fake` でテストする

これは、あなたが毎日使う部分です。`Http::fake` は、あらゆるアウトバウンドの呼び出しが横取りされ、キャプチャされ、あなたがキューに入れたものによって応答される `tokio::task_local!` のスコープの内側で、あなたのテスト本体を実行します。

```rust
use suprnova::{Http, fake_response, assert_sent};

#[tokio::test]
async fn creates_a_user_via_api() {
    Http::fake(|| async {
        fake_response(
            "POST",
            "/api/users",
            201,
            serde_json::json!({ "id": 42, "name": "Ada" }),
        );

        let resp = Http::post("https://example.com/api/users")
            .json(&serde_json::json!({ "name": "Ada" }))
            .send()
            .await
            .unwrap();

        assert_eq!(resp.status(), 201);
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["id"], 42);

        assert_sent(|r| r.method == "POST" && r.url.contains("/api/users"));
    })
    .await;
}
```

### 用意されたレスポンスにマッチさせる

`fake_response(method, url_substring, status, body)` は、用意されたレスポンスをキューに入れます。メソッドが一致し（大文字小文字を区別しません）、URLが `url_substring` を含む最初のアウトバウンドのリクエストが、その用意されたエントリを消費し、そのレスポンスを返します。あらゆるメソッドに一致させるには、メソッド `"*"` を使ってください。

それ以降の一致するリクエストは、同じ形の次の用意されたエントリへ流れ落ちるか、一致するものがなければ、空の `200 {}` を返します。期待する呼び出しごとに、1つの用意されたレスポンスをキューに入れてください。

```rust
fake_response("GET", "/v1/customer", 200, json!({ "id": "cus_1" }));
fake_response("GET", "/v1/customer", 200, json!({ "id": "cus_2" }));
// /v1/customer への2回のGETはそれぞれ別のレスポンスを得て、3回目は200 {}を得る。
```

### アサーション

```rust
// 記録されたリクエストのうち少なくとも1つが一致すればパスする。
assert_sent(|r| r.method == "POST" && r.url.contains("/charges"));

// 一致する記録されたリクエストがなければパスする。
assert_not_sent(|r| r.url.contains("/refunds"));
```

`RecordedRequest` は、`method: String`、`url: String`、`headers: Vec<(String, String)>`、`body: Option<Vec<u8>>` を公開します。述語は、記録されたあらゆるリクエストに対して実行されます - アサーションが失敗すると、ヘッダーの値とボディを伏せ字にした記録済みのリストが出力されます（`Content-Type`、`Accept`、`User-Agent` の小さな許可リストは全文が表示され、それ以外はすべて `<redacted>` です）。これにより、アサーションが吹き飛んだときでも、ベアラートークンとwebhookのペイロードがCIのログから守られます。

### テストは安全に並列実行される

フェイクの状態は `tokio::task_local!` の中に存在します - あらゆるフェイクのスコープは、プロセスではなく、テストを実行するタスクにスコープされます。異なるタスクの上で並行に実行される2つのテストは、それぞれ自分専用の記録済みリクエストのvecと、自分専用の用意されたレスポンスのキューを手にします。共有されるmutexも、テストの順序も、`#[serial]` もありません。

```rust
#[tokio::test]
async fn first_test() {
    Http::fake(|| async {
        fake_response("GET", "/a", 200, json!({"who": "first"}));
        let _ = Http::get("https://x.test/a").send().await.unwrap();
        assert_sent(|r| r.url.contains("/a"));
        // 兄弟テストによる /b へのリクエストは、ここでは見えない。
    })
    .await;
}

#[tokio::test]
async fn second_test() {
    Http::fake(|| async {
        fake_response("GET", "/b", 200, json!({"who": "second"}));
        let _ = Http::get("https://x.test/b").send().await.unwrap();
        assert_sent(|r| r.url.contains("/b"));
    })
    .await;
}
```

## spawnされたタスクの落とし穴

`tokio::task_local!` は、現在のタスクにスコープされます。`tokio::spawn` を経由する作業は、新しいタスクの上に着地し、フェイクを継承**しません** - デフォルトでは、spawnされたfutureからのアウトバウンドの呼び出しは、本物のネットワークに当たります。これに対処する2つのヘルパーがあります。

### `Http::fail_on_real_calls()` と `FailOnRealCallsGuard`

一致しなかったあらゆるアウトバウンドの呼び出しを、ネットワークに当てるのではなく `FrameworkError::internal(...)` へ変えるプロセスグローバルなフラグを切り替えます。これは、Laravelの `Http::preventStrayRequests()` のSuprnova版です - この落とし穴が生み出すまさにそのバグを捕まえます。

パニックが起きた場合でも、テストが終わるときにこのフラグがリセットされるよう、RAIIガードを使ってください。

```rust
use suprnova::FailOnRealCallsGuard;

#[tokio::test]
async fn no_test_makes_a_real_call() {
    let _guard = FailOnRealCallsGuard::install();

    // このテストの内側のどこから来たものであれ - `tokio::spawn` された
    // タスクからも含めて - フェイクされていないアウトバウンドのHTTP呼び出しは、
    // URLを名指しするメッセージでエラーになる。実際のネットワークI/Oは起きない。
}
```

入れ子になったガードは正しく組み合わさります - 内側のガードの `Drop` は、無条件に「許可」するのではなく、直前の状態を復元します。そのため、外側のガードされたスコープの内側で自分自身のガードをインストールする内側のテストヘルパーは、外へ出るときに外側のガードを解除してしまうことはありません。

このフラグは、設計上プロセスグローバルです。要点は、`tokio::spawn` されたfutureが静かにフェイクのスコープから逃れ、CIから本物のサードパーティへ通信してしまうことを捕まえることです。タスクごとのフラグでは、それを見逃してしまいます。

### `Http::spawn_with_fake_inheritance(future)`

テスト対象のコードが正当にタスクをspawnする場合 - キューワーカー、バックグラウンドの同期処理、サブタスク - に、そのアウトバウンドの呼び出しを親のフェイクを通過させたいときは、`tokio::spawn` を `Http::spawn_with_fake_inheritance` に差し替えてください。

```rust
Http::fake(|| async {
    fake_response("GET", "/child", 204, json!({}));

    let handle = Http::spawn_with_fake_inheritance(async {
        // 新しいタスクの上で実行されるが、親のフェイクの状態は
        // このタスクのタスクローカルなスコープの中へ再インストールされる。
        // 送信は横取りされ、レスポンスは上の204になる。
        Http::get("https://child.example.com/child").send().await
    });

    let response = handle.await.unwrap().unwrap();
    assert_eq!(response.status(), 204);

    // 子タスクからの記録されたリクエストはここに現れる -
    // Arc<Mutex<FakeState>> は共有されており、スナップショットではない。
    assert_sent(|r| r.url.contains("/child"));
})
.await;
```

`spawn_with_fake_inheritance` を呼ぶときにアクティブなフェイクのスコープがなければ、それは `tokio::spawn` と等価です - 子タスクは、フェイクのコンテキストを何も持たずに実行されます。そのため、`Http::fake` でテストされることもあれば、されないこともあるコードの中で、これを無条件に使えます。

### テストセットアップにおける二重の安全策

この2つは組み合わせられます。目立つ形で安全でありたいテストは、これらを組にします。

```rust
#[tokio::test]
async fn pays_the_invoice() {
    let _guard = FailOnRealCallsGuard::install();

    Http::fake(|| async {
        fake_response("POST", "/v1/charges", 200, json!({ "id": "ch_1" }));

        // URLやメソッドのタイプミスがフェイクからずれてしまった場合、
        // リクエストはガードへ流れ落ち、URLを名指しするメッセージで
        // エラーになる - 不一致を隠してしまう空の200を無音で返すのではない。
        pay_invoice(&invoice).await.unwrap();

        assert_sent(|r| r.url.contains("/v1/charges"));
    })
    .await;
}
```

ガードがなければ、フェイクからずれたURLやメソッドは無音でデフォルトの `200 {}` へ流れ落ち、本番環境のコードが別のエンドポイントを呼んでいるにもかかわらず、あなたのテストは通ってしまいます。ガードがあれば、最初の不一致ではっきりと失敗します。

## OpenTelemetryのトレース伝播

フレームワークが `otel` フィーチャーでビルドされ、W3CのTraceContextプロパゲーターがインストールされている場合、あらゆるアウトバウンドの `Http::*` リクエストは、`traceparent`（そして空でない場合は `tracestate`）をそのヘッダーへ注入します - そのため、下流のサービスはトレースを続けられます。呼び出し箇所での設定は不要です - プロパゲーターは、送信時に `opentelemetry::Context::current()` を読み取ります。

アクティブなOTelコンテキストがなければ、ヘッダーは何も注入されず、アウトバウンドのリクエストは以前と全く同じに見えます。プロパゲーターのセットアップについては[可観測性](observability.md)を参照してください。

## Suprnovaが異なる設計を選んだ理由

Laravelの `Http::` ファサードからの、2つの小さな分岐が指摘に値します。どちらもランタイムモデルによって強いられたものです。

**プロセスグローバルなモックストアではなく、タスクローカルなフェイク。** Laravelの `Http::fake()` は、プロセス全体のレジストリを変更します - テストはそれに対して直列化されるか、あるいは並列なランナーが競合することを受け入れます。Suprnovaの `Http::fake` は `tokio::task_local!` を使うため、2つのタスクの上の2つのテストは、それぞれ自分専用のフェイクを見ます - テストの順序も、共有されるmutexもありません。その代償は、`tokio::spawn` された作業がデフォルトではフェイクを継承しないことであり、それが `Http::spawn_with_fake_inheritance` と `FailOnRealCallsGuard` が存在する理由です。この2つが合わさることで、Laravelの `Http::preventStrayRequests()` と同じ「誤って本番環境に当たることはない」という保証を、より厳格なスコープで手にできます。

**リトライは、デフォルトでPOST/PATCHを拒否します。** LaravelのHTTPクライアントは、デフォルトでどのメソッドもリトライします。Suprnovaの `.retry(...)` はべき等な場合のみであり、べき等でないメソッドには明示的な `.retry_non_idempotent(...)` によるオプトインが必要です。その理由は、書き込みエンドポイントからの5xxのレスポンスが、しばしば「書き込みはコミットしたが、その後レスポンスが失われた」を意味するからです - それを盲目的に再生すると、課金、返金、ファンアウトが重複してしまいます。私たちは、呼び出し元に決めさせます。アップストリームが尊重するべき等性キーを渡していますか? もしそうならPOST/PATCHをリトライへオプトインさせてください。そうでなければ、5xxを受け入れてください。

## エッジケースと細かい注意点

- **`Http::*` はv1に対して閉じています。** 裏にある `reqwest::Client` を、私たちは意図的に公開していません。表面を広げるには、`reqwest` へ直接手を伸ばすのではなく、ファサードへメソッドを加えてください - 本物のレスポンスの上に文書化された `into_inner()` という逃げ道を使う場合を除きます。
- **共有クライアントは、一度だけ構築され、永久に生き続けます。** あらゆる `Http::*` の動詞への最初の呼び出しで遅延構築され、`OnceLock` の中に保たれます。rustlsのTLSスタックと30秒のデフォルトタイムアウトは、焼き込まれています。
- **JSON/フォームのシリアライズの失敗は、はっきりと失敗します。** `.json(&unserializable)` のビルダーはエラーを記録し、`send()` はそれを `FrameworkError::internal(...)` として返します。リクエストは決して送信されません - 私たちは `null` のボディへ格下げすることはしません。
- **30秒のリトライ上限は固定です。** バックオフの計算は30秒でキャップされ、`Retry-After` の解釈も30秒でキャップされます - 単一のリトライのスリープが、それより長くタスクをパークすることはありません。
- **プロセスグローバルな上限は、ワンショットです。** `Http::set_max_response_bytes` は、プロセスグローバルなatomicへの書き込みです - 起動時に一度だけ設定し、必要に応じてリクエストごとに上書きしてください。「デフォルトへリセットする」呼び出しはありません。

## 次のステップ

- [メール](mail.md) - アウトバウンドのメールで、テストのために似たフェイク / ドライバーのパターンを使います
- [通知](notifications.md) - web pushを含む通知のチャネルで、すべて同じテストフェイクの哲学を共有します
- [キュー](queues.md) - アウトバウンドのHTTP呼び出しを行うジョブと、ワーカーをテストするための `spawn_with_fake_inheritance` パターン
- [テスト](testing.md) - `#[suprnova_test]`、`TestContainer`、そしてフェイクの表面の残り
- [可観測性](observability.md) - `traceparent` 注入を機能させる、OTelプロパゲーターのセットアップ
