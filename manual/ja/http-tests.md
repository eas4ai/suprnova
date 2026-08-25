# HTTP テスト

この章は、フレームワークのリクエストパイプラインを `suprnova::handle_request` を通じて駆動することで、あなたのHTTP表面 - ルート、ミドルウェア、認証フロー、エラーレスポンス - をテストする方法を示します。`$this->get('/users')` でLaravelのフィーチャーテストを書き、`$response->status()` をアサートした経験があるなら、これがSuprnovaの相当物です - 本番環境でマウントするのと同じ `Router` がテストの中で実行され、あらゆるミドルウェアが発火し、パニック境界はそれでも捕捉し、レスポンスは本物のクライアントが見るものとバイト単位で一致します。

## テストの表面

ちょうど3つの構成要素があります。

| 要素 | 役割 |
|---|---|
| `Router` | テスト対象のルート - 本番環境と同じ方法で構築されます |
| `MiddlewareRegistry` | グローバルなミドルウェアスタック - これも同じ方法で構築されます |
| `handle_request(router, registry, req) -> hyper::Response<…>` | プロセス内のドライバー - 1つのリクエストをエンドツーエンドで実行します |

`handle_request` は、`Server::run` がリクエストごとに呼ぶのと同じ関数であり、テストと組み込み先のために公開されています。本番環境で機能するものは何であれ、ここでも機能します - パニックリカバリのラッパー、リクエストIDのスコープ、Inertiaのフラッシュバッグのスコープ、認証のリクエスト状態スコープ、HEADのボディの除去、レスポンス後の終了処理です。より静かなパイプラインに差し替える「テストモード」はありません。

`handle_request_with_peer` は、接続するピアのための明示的な `Option<std::net::IpAddr>` を伴う、同じ呼び出しです - プロキシヘッダーをセットアップすることなく `Request::ip()` の解決をアサートしたいときに便利です。

## hyperのボディ問題

前もって知っておく価値のある、1つのひねりがあります。`handle_request` は `hyper::Request<hyper::body::Incoming>` を取ります。`Incoming` はhyperの内部的なストリーミングボディ型であり、`Full::new(bytes)` やその他のインメモリのボディ型で構築することはできません。これは、hyperのコネクションから出てくるものだけです。

これを回避する、きれいな方法が2つあります。

1. **TCPループバック** - `127.0.0.1:0` のリスナーをバインドし、`service_fn` の中で1つのacceptを処理し、hyperクライアントを通じてリクエストを送り、サーバー側で `Incoming` が自然に生成されるようにします。フレームワークのあらゆる統合テストが、すでにこれを行っています。
2. **プロセス内でのRequestの構築** - ルーティングを経由せずに `Request` のアクセッサー（ヘッダー、ルートパラメータ、IP、JSONのパース）を調べるだけで済むテストには、同じTCPループバックのキャプチャパターンを使いますが、それを実行する代わりに `Request` を `oneshot::channel` へ取り出すサービスを使います。`framework/tests/http_request_accessors.rs` ファイルには、この `build_request()` ヘルパーがそのまま入っています。

どちらのパターンも、本物の `Incoming` ボディを生成します。このループバックはローカルであり、テストの実時間の観点では同期的（マイクロ秒単位）であり、`lo` の外のネットワークには決して触れません。この契約を保ったまま、これより遅い、あるいはこれより単純な方法はありません。

### Suprnovaが異なる設計を選んだ理由

Laravelの `$this->get('/users')` が機能するのは、PHPのリクエストライフサイクルが「`Request` オブジェクトを組み立て、それをカーネルを通じてディスパッチする」というものだからです。カーネルはインメモリのオブジェクトを直接取り、トランスポートを強制するボディ型はありません。Suprnovaのサーバーはhyperの上に構築されており、hyperのボディ型は、正当な理由（ストリーミング、バックプレッシャー、ゼロコピー）から意見を持っています。テストの表面は、その制約を引き継ぎます。

その制約と引き換えに得られるのは忠実さです。本番環境のリクエストパスのあらゆる詳細 - ヘッダーのパース、ボディの上限、コネクションのアップグレード - は、テストの中でも同じように実行されます。テストハーネスが、本物のサーバーが実行する層をスキップしたためにテストが通ってしまう、ということは決してありません。

## 最初のエンドツーエンドテスト

ここに、1つのルートをマウントし、それに対してGETを送り、ステータスとボディをアサートする、完全に動作するテストがあります。

```rust
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;

use suprnova::http::text;
use suprnova::{MiddlewareRegistry, Request, Router, handle_request};

async fn spawn_server(
    router: Router,
    middleware: MiddlewareRegistry,
    accepts: usize,
) -> SocketAddr {
    let router = Arc::new(router);
    let middleware = Arc::new(middleware);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral listener");
    let addr = listener.local_addr().expect("local_addr");

    tokio::spawn(async move {
        for _ in 0..accepts {
            let Ok((stream, _)) = listener.accept().await else { return };
            let io = TokioIo::new(stream);
            let router = router.clone();
            let middleware = middleware.clone();
            tokio::spawn(async move {
                let svc = service_fn(move |req: hyper::Request<Incoming>| {
                    let router = router.clone();
                    let middleware = middleware.clone();
                    async move {
                        Ok::<_, Infallible>(handle_request(router, middleware, req).await)
                    }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(io, svc)
                    .await;
            });
        }
    });

    addr
}

async fn send_get(addr: SocketAddr, path: &str) -> (u16, Bytes) {
    let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let io = TokioIo::new(stream);
    let (mut sender, conn) =
        hyper::client::conn::http1::handshake::<_, Full<Bytes>>(io).await.unwrap();
    tokio::spawn(async move { let _ = conn.await; });

    let req = hyper::Request::builder()
        .method("GET")
        .uri(path)
        .header("Host", "localhost")
        .header("Content-Length", "0")
        .body(Full::new(Bytes::new()))
        .unwrap();

    let resp = tokio::time::timeout(Duration::from_secs(5), sender.send_request(req))
        .await
        .expect("send_get timeout")
        .expect("hyper send_request");
    let (parts, body) = resp.into_parts();
    let bytes = body.collect().await.unwrap().to_bytes();
    (parts.status.as_u16(), bytes)
}

#[tokio::test]
async fn get_root_returns_hello() {
    let router = Router::new().get("/", |_req: Request| async { text("hello") });
    let addr = spawn_server(router, MiddlewareRegistry::new(), 1).await;

    let (status, body) = send_get(addr, "/").await;
    assert_eq!(status, 200);
    assert_eq!(&body[..], b"hello");
}
```

これが全体の形です。この2つのヘルパーをクレートごとにコピーし、スイートに合わせて調整してください（複数のaccept、ヘッダーのキャプチャ、ボディのキャプチャ）。フレームワーク自体は、`framework/tests/cors_middleware.rs`、`framework/tests/middleware_panic_safety.rs`、`framework/tests/email_verified_middleware.rs` の中で、ほぼ同一のヘルパーを使っています。

`accepts` という引数は、acceptループが終了するまでに処理するコネクションの数を制限します。単一のリクエストには1で十分です - テストがパニック後のリカバリを行使する場合は、2以上に増やしてください（[パニック境界をテストする](#パニック境界をテストする)を参照してください）。

## リクエストを組み立てる

`send_get` の内側で、あなたはこれを見ました。

```rust
let req = hyper::Request::builder()
    .method("GET")
    .uri("/users/42")
    .header("Host", "localhost")
    .header("Content-Length", "0")
    .body(Full::new(Bytes::new()))
    .unwrap();
```

これが標準の形です。知っておく価値のあることがいくつかあります。

- **`Host` ヘッダー**。Hyperは、これを持たないHTTP/1.1のリクエストを拒否します。常にこれを含めてください - あなたのハンドラがこれをキーにしない限り、値は問題になりません。
- **`Content-Length: 0`**。ボディに一致させてください。Hyperは `Full::new(Bytes::new())` を使うとこれを計算してくれますが、テストの中では明示する方が読みやすくなります。
- **ボディの型**。クライアント側は `Full<Bytes>` を送ります。サーバー側は `Incoming` を受け取ります。テストの中で構築するのは常に `Full<Bytes>` のリクエストだけです - フレームワークは、hyperのコネクションごとの変換の後、それらを `Incoming` として受け取ります。

JSONボディを伴うPOSTです。

```rust
let body_bytes = serde_json::to_vec(&serde_json::json!({
    "name": "Alice",
    "email": "alice@example.com"
})).unwrap();

let req = hyper::Request::builder()
    .method("POST")
    .uri("/users")
    .header("Host", "localhost")
    .header("content-type", "application/json")
    .header("content-length", body_bytes.len())
    .body(Full::new(Bytes::from(body_bytes)))
    .unwrap();
```

## レスポンスをアサートする

`handle_request` から返ってくるレスポンスは、`hyper::Response<BoxBody<Bytes, Infallible>>` です。そこから読み取ることになる3つのことがあります。

```rust
let (parts, body) = resp.into_parts();

// 1. ステータス。
assert_eq!(parts.status.as_u16(), 200);

// 2. ヘッダー - 大文字小文字を区別しないルックアップ。
let location = parts.headers.get("location").and_then(|v| v.to_str().ok());
assert_eq!(location, Some("/login"));

// 3. ボディ - バイトへ集約してから、パースする。
use http_body_util::BodyExt;
let bytes = body.collect().await.unwrap().to_bytes();

// テキストとして:
let text = String::from_utf8_lossy(&bytes);

// JSONとして:
let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
assert_eq!(value["message"], "ok");
```

共通のレンダラーに到達する通常のエラーレスポンスでは、[エラー モデル](error-model.md)に文書化されたボディ形状に `message`、任意の `errors`、`request_id`、任意の `debug_message` が含まれます。`request_id` はリクエストスコープ外では `null` です。3つの特別なバリアントはrequest-idの注入前に返ります: `PrecognitionSuccess` はボディなしの204、`PrecognitionFailure` はバリデーションボディにPrecognitionヘッダーを加えたもの、誤ってHTTPレンダリングされた `AlreadyReported` の番兵は `message` だけを含む汎用的な500です。request-idミドルウェアが実行されたことをアサートするときは、通常のエラーレスポンスを使ってください。

## TestResponseによる流暢なレスポンスアサーション

上で行ったように `(status, headers, body)` のトリプルを手で組み立て、1つずつアサートすることは、このクレートのすべてのハーネスが使う基礎です。`suprnova::testing::TestResponse` は同じトリプルをLaravel風の流暢なAPIで包むため、テストはヘッダーのルックアップではなくアサーションのように読めます:

```rust
use suprnova::testing::TestResponse;

let (parts, body) = resp.into_parts();
let bytes = body.collect().await.unwrap().to_bytes();
let headers = parts.headers.iter().map(|(k, v)| {
    (k.as_str().to_string(), v.to_str().unwrap_or_default().to_string())
});

TestResponse::new(parts.status.as_u16(), headers, bytes)
    .assert_ok()
    .assert_header("content-type", "application/json")
    .assert_json(serde_json::json!({ "message": "ok" }));
```

`new()` は `(String, String)` ヘッダーペアとして反復可能なものなら何でも受け付けます - いくつかの既存ハーネスがすでに収集している `HashMap<String, String>`、`Vec<(String, String)>`、または `HeaderMap::iter()` を所有文字列へマップしたものです。そのため、リクエストの駆動方法を変更する必要はありません。

すべてのアサーションは `&Self` を返すため、チェーンできます: `assert_status`、`assert_ok`、`assert_redirect(target: Option<&str>)`、`assert_json`（部分一致 - ボディ内の追加キーは問題ありません）、`assert_json_path`（ドット記法で、数値セグメントは配列をインデックス指定）、`assert_json_count`、`assert_see`、`assert_header`、`assert_cookie` です。アサーション失敗は `expect!`（[テスト](testing.md)）と同じ契約で、期待値/実際の値の抜粋とともにパニックします - これはテスト用の表面であり、ライブラリコードではないため、パニック禁止の原則は適用されません。

### `assert_session_has` にはセッションストアが必要

他のすべてのアサーションはレスポンスレベルのレスポンスだけを読みます。`assert_session_has` はそうできません。サーバー側のセッション状態はレスポンスではなく `SessionStore` に存在し、ループバックソケットからレスポンスが返る時点では、読み取れるプロセス内セッションはもうありません。テストの `SessionMiddleware` を構築したときと同じストアを、そのCookie名とともに取り付けてください。するとアサーションがレスポンスのセッションCookieを復号して、行自体を見つけます:

```rust
let response = TestResponse::new(status, headers, body)
    .with_session_store(middleware.store(), "suprnova_session");

response
    .assert_session_has("flash.success", serde_json::json!("Saved!"))
    .await;
```

これは唯一の `async` アサーションです。I/Oを行う唯一のアサーションだからです。それでも `&Self` を返すので、`.await` をインラインに置き、後続のチェーンを続けられます。

### Suprnovaが異なる設計を選んだ理由

Laravelの `TestResponse` はテスト対象と同じPHPプロセスに存在するため、`assertSessionHas` は `$this->session()` を直接読みます - 越えるべきレスポンス境界がありません。Suprnovaのテストは実際のhyper接続を駆動するため、セッションは実際のブラウザーと同様に、テストにとって不透明なCookieです。`assert_session_has` は、プロセス内のショートカットが存在するふりをする代わりに、明示的なストアハンドルでその正直さを取り戻します。

## Inertiaレスポンスのテスト

`suprnova::testing::AssertableInertia` は、`X-Inertia` JSONボディとして返ったか、ハードナビゲーションのHTMLシェルに埋め込まれたかにかかわらず、Inertiaページオブジェクトを `TestResponse` と同じ流暢で失敗時にパニックするスタイルで包みます。Laravelの `Inertia\Testing\AssertableInertia` に相当します。

取得方法は2つあります。実際の `X-Inertia: true` 訪問を通った `TestResponse` から取得する方法:

```rust
use suprnova::testing::TestResponse;

let response = TestResponse::new(status, headers, body);
response
    .assert_inertia()
    .component("Users/Index")
    .url("/users")
    .has("users")
    .where_("users.0.name", "Ada")
    .count("users", 1)
    .missing("admin_only_field");
```

または、`InertiaResponse::resolve` が返す `HttpResponse` から直接取得する方法です。ソケットなしでレスポンスパイプラインを駆動するテストに使います。この形は両方の形状を扱います: `X-Inertia` JSONボディ、またはHTMLシェルに埋め込まれた `<script data-page="app">` 要素です:

```rust
use suprnova::testing::AssertableInertia;

let response = InertiaResponse::new("Users/Index")
    .with("users", users_json)
    .resolve(&req)
    .await?;

AssertableInertia::from_response(&response)
    .component("Users/Index")
    .where_("users.0.name", "Ada");
```

`version()` はページのアセットバージョンを検査します。デフォルトのリゾルバーはViteマニフェストをハッシュし、マニフェストがまだ存在しない場合は `MANIFEST_VERSION_FALLBACK` にフォールバックします。フロントエンドをビルドしていないテストでは、ハードコードした `"1.0"` ではなく、この定数に対してアサートしてください:

```rust
use suprnova::MANIFEST_VERSION_FALLBACK;

response.assert_inertia().version(MANIFEST_VERSION_FALLBACK);
```

`has_flash(key, expected)` は、`has` / `where_` がプロップを読むのと同じドットパスでページのフラッシュデータを読みます。`expected` は `Option` なので、存在だけを検査するには `None::<serde_json::Value>` を渡します:

```rust
response.assert_inertia().has_flash("toast.message", Some(serde_json::json!("Saved!")));
response.assert_inertia().has_flash("toast", None::<serde_json::Value>);
```

### 部分的なリロードとディファードプロップのアサーション用に再読み込みする

`reload_only`、`reload_except`、`load_deferred_props` は、初回訪問の後にInertiaクライアントが行うことを再現します。同じページを部分的なリロードとして再送信し、返ってきたものを確認します。SuprnovaのHTTPテストは実際のソケットを通り、各テストファイルが独自のハーネスを所有するため（下記の[各要素の実装場所](#各要素の実装場所)を参照）、これらのメソッドには組み込みのトランスポートがありません。`ReloadRequest`（送るURL、コンポーネント、バージョン、部分リロードのキー）から、リロードされた `AssertableInertia` を返すfutureを作るクロージャーを `with_reload` で取り付けてください:

```rust
use suprnova::testing::TestResponse;

let assertable = TestResponse::new(status, headers, body)
    .assert_inertia()
    .with_reload(move |reload| {
        async move {
            let header_pairs = reload.headers();
            let headers: Vec<(&str, &str)> = header_pairs
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();
            let (status, headers, body) = request(addr, "GET", &reload.url, &headers).await;
            TestResponse::new(status, headers, body).assert_inertia()
        }
    });

// `users` だけを要求し、同じコンポーネント/URL/バージョンに
// リロードされて `users` が返ったことをアサートします。
assertable.reload_only(["users"]).await;

// `stats` 以外をすべて要求し、`stats` がないことをアサートします。
assertable.reload_except(["stats"]).await;

// 元のページから `deferredProps` を読み、ディファードなキーを
// すべて1回の部分リロードで要求し、すべて返ったことをアサートします。
assertable.load_deferred_props().await;
```

先に `with_reload` を付けずに3つのいずれかを呼ぶと、その指示を示してパニックします。リロードの結果は同じリローダーを引き継ぐため、そこからの2回目の `.reload_only(...).await` は再接続せずに動作します。

### Suprnovaが異なる設計を選んだ理由

Laravelの `ReloadRequest` は、元のテストが使った同じプロセス内PHPカーネルを通じてリクエストを再送します - 常に利用できる1つのテストクライアントです。SuprnovaのHTTPテストは実際のhyper/TCPループバックを駆動し、各テストファイルが独自の `spawn_server` / `request` ペアを定義するため（下記の[各要素の実装場所](#各要素の実装場所)を参照）、`AssertableInertia` が利用できる単一のクライアントはありません。`with_reload` は、形状の異なるテストファイルでは使えないハーネスをハードコードする代わりに、そのことを明示します。`component()` もLaravelのページコンポーネントファイル存在検査（`view-finder`）を省略します。`Router::inertia` または手作りの `InertiaResponse::new(name)` を通ったコンポーネントは、検査対象のファイルがないランタイム文字列だからです。Suprnovaのコンパイル時に相当するものは `inertia_response!` マクロです（[Inertiaレスポンス](frontend-inertia-responses.md)を参照）。メソッド名も `TestResponse` のものとは異なります: `component`、`has`、`missing`、`where_`、`count`、`has_flash` は `assert_` プレフィックスを完全に省き、Laravelの `Inertia\Testing\AssertableInertia` に合わせています。対応するメソッドが同じように裸であるためです。失敗時にパニックする契約はどちらでも同一であり、`assert_` という見た目の手がかりは必要ありません。

## ミドルウェアをテストする

ミドルウェアのテストは、ルートのテストと同じに見えます - 違いは、spawnする前にレジストリへ何を `.append()` するかだけです。

### グローバルミドルウェアをテストする

そのミドルウェアを `MiddlewareRegistry::new().append(...)` へ渡し、そのレジストリを使ってください - 複数のミドルウェアはappendした順に実行され、`prepend` は新しいものを先頭に置きます。

```rust
use suprnova::{CorsConfig, CorsMiddleware, MiddlewareRegistry};

fn cors_registry() -> MiddlewareRegistry {
    MiddlewareRegistry::new().append(CorsMiddleware::new(
        CorsConfig::allow_origins(["https://app.example"])
            .allow_credentials(true)
            .max_age(std::time::Duration::from_secs(600)),
    ))
}

#[tokio::test]
async fn cors_preflight_returns_204_with_headers() {
    let router = Router::new();
    // spawn_serverの3引数の形を使うと、空でないMiddlewareRegistryを
// 配線できる - framework/tests/cors_middleware.rs からこのヘルパーを
// コピーする（~30行）。
let addr = spawn_server(router, cors_registry(), 1).await;

    let (status, headers, _) = options(
        addr,
        "/anything",
        &[
            ("Origin", "https://app.example"),
            ("Access-Control-Request-Method", "POST"),
        ],
    ).await;

    assert_eq!(status, 204);
    assert_eq!(
        headers.get("access-control-allow-origin").map(String::as_str),
        Some("https://app.example"),
    );
}
```

このテストが証明しているのは、CORSのロジック自体だけではありません - グローバルミドルウェアが**ルーティングされない**リクエストに対しても実行されることを証明しています。これは、フレームワークが保証する契約です（そうでなければ、決してルートにマッチしないOPTIONSのプリフライトは、CORSをスキップしてしまいます）。完全なスイートについては `framework/tests/cors_middleware.rs` を参照してください。

### ルート固有のミドルウェアをテストする

本番環境と全く同じように、ルートビルダーの上で `.middleware(...)` を使って取り付けてください。それから、通常どおりそのルートをテストしてください - ミドルウェアチェーンは、同じ登録から組み立てられます。

```rust
let router = Router::new()
    .get("/admin/dashboard", |_req| async { text("admin") })
    .middleware(RequireRole::new("admin"));

let (status, _) = send_get(addr, "/admin/dashboard").await;
assert_eq!(status, 403); // 未認証のリクエスト
```

### 認証済みユーザーをスタブする

本物の認証フローのテストには、ログイン済みのユーザーが必要です。最もきれいなパターンは、テスト対象のミドルウェアより前に `Auth::set_user` を呼ぶ、小さな一回限りのミドルウェアです。フレームワーク自身の `framework/tests/email_verified_middleware.rs` が、これを使っています。

```rust
use std::any::Any;
use std::sync::Arc;
use suprnova::{Auth, Authenticatable, Middleware, Next, Request, Response};

struct UserById(String);

impl Authenticatable for UserById {
    fn get_auth_identifier(&self) -> String { self.0.clone() }
    fn as_any(&self) -> &dyn Any { self }
}

struct LoginAs(String);

#[async_trait::async_trait]
impl Middleware for LoginAs {
    async fn handle(&self, request: Request, next: Next) -> Response {
        Auth::set_user(Arc::new(UserById(self.0.clone())));
        next(request).await
    }
}
```

そして、テストの中では次のようになります。

```rust
let registry = MiddlewareRegistry::new()
    .append(LoginAs("user-id-123".to_string()))
    .append(EnsureEmailVerifiedMiddleware::new());
```

`LoginAs` が最初に実行され、そのユーザーをリクエストごとの認証状態へインストールします。そしてテスト対象のミドルウェアは、本物のログインを一度も発行することなく `Auth::id() == Some(...)` を見ることになります。認証状態のスコープは、`handle_request` 自身によって - 本番環境で実行されるのと同じものによって - セットアップされるため、そのユーザーは、それより後のあらゆるミドルウェアとハンドラから見えます。

## ルートモデルバインディングをテストする

`RouteParam<User>` はハンドラのエクストラクターチェーンを通じて型付きの `User` をハイドレートするため、テストはそのエクストラクターを `#[handler]` 関数へ渡さなければなりません:

```rust
use suprnova::{RouteParam, Response, handler};

#[suprnova::model(table = "users")]
pub struct User {
    pub id: i64,
    pub email: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[handler]
async fn show(RouteParam(user): RouteParam<User>) -> Response {
    suprnova::http::json(serde_json::json!({ "email": user.email }))
}

#[tokio::test]
async fn show_user_binds_from_route_param() {
    // テスト用ユーザーをモデル経由で挿入する。データベースのセットアップは省略 -
    // `TestDatabase` のパターンについてはテストの章を参照。
    let user = User::create(suprnova::attrs! {
        email: "bound@example.com"
    }).await.unwrap();

    // 分解したRouteParamは現在、ハンドラマクロのルートパラメーター名として
    // `param` を使います。
    let router: Router = Router::new()
        .get("/users/{param}", show)
        .into();

    let addr = spawn_server(router, MiddlewareRegistry::new(), 1).await;
    let (status, body) = send_get(addr, &format!("/users/{}", user.id)).await;

    assert_eq!(status, 200);
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["email"], "bound@example.com");
}
```

代わりに `{user}` というルートパラメーターを使うには、分解せずに `user: RouteParam<User>` を受け取ってください。`RouteParam` はフィールドアクセスのために `User` へデリファレンスします。`req.param(...).parse()` を呼んでから `User::find_or_fail(...)` を呼ぶのは、パラメーターのパースとモデルのルックアップをテストするものであり、ルートモデルバインディングではありません。

バインディングを分離してテストするには、`<RouteParam<User> as AutoRouteBinding>::from_route_param(...)` を直接呼び出してください。これはルーターなしでバインディング実装を確認しますが、`#[handler]` のエクストラクターチェーンは行使しません。

## 認証フローをエンドツーエンドでテストする

ログインセッションをエンドツーエンドでテストするには、`SessionMiddleware` を含むレジストリをループバックサーバーに渡し、`AuthMiddleware` またはアプリケーションのweb認証ミドルウェアで `/dashboard` を保護してください。最初にクッキーなしのリクエストをルートが拒否することを示し、ログインし、返されたセッションクッキーを再送して、保護されたルートが成功することを示します:

```rust
#[tokio::test]
async fn login_flow_issues_session_cookie() {
    // 1. ブートストラップ: ユーザーを作成する。
    Auth::password()
        .register("alice@example.com", "longpassword123")
        .await.expect("register");

    // 2. 保護されたルートとステートフルなセッションミドルウェアをマウントする。
    let router: Router = Router::new()
        .post("/login", login_handler)
        .get("/dashboard", |_req: Request| async { text("dashboard") })
        .middleware(AuthMiddleware::new())
        .into();
    let registry = MiddlewareRegistry::new()
        .append(SessionMiddleware::new(SessionConfig::from_env()));
    let addr = spawn_server(router, registry, 3).await;

    // 3. 認証前にルートが保護されていることを示す。
    let (guest_status, _) = send_get(addr, "/dashboard").await;
    assert_eq!(guest_status, 401);

    // 4. ログインを駆動し、Set-Cookieヘッダーをキャプチャする。
    let login = post_json(addr, "/login", serde_json::json!({
        "email": "alice@example.com",
        "password": "longpassword123",
    })).await;
    assert_eq!(login.status, 200);
    let cookie = extract_session_cookie(&login.headers);

    // 5. 保護されたルートに対してクッキーを再送する。
    let (status, body) = get_with_cookie(addr, "/dashboard", &cookie).await;
    assert_eq!(status, 200);
    assert_eq!(&body[..], b"dashboard");
}
```

これらのミドルウェアなしの簡略化したルーターは、クッキーの配線だけを示すものであり、認証フローのテストではありません。`framework/tests/auth_http_middleware.rs` は明示的なレジストリで認証ミドルウェアの振る舞いをテストしますが、本物の `SessionMiddleware` はインストールしません。ステートフルなログインフローのテストは、上で示した通り、セッションミドルウェアと認証ゲートの両方をインストールしなければなりません。

## パニック境界をテストする

ハンドラの内側のパニックは、サーバーをクラッシュさせてはなりません。パニックリカバリのラッパー（`execute_chain_safely`）がそれを捕捉し、返されたエラーが流れるのと同じ経路を通じて500へ変換します。これは、特別なテストインフラなしで検証できます - リスナーがパニックを生き延びられるよう、`accepts >= 2` を設定してください。

```rust
#[tokio::test]
async fn panicking_handler_yields_500_and_server_survives() {
    let router = Router::new()
        .get("/panic", |_req: Request| async {
            panic!("intentional test panic");
            #[allow(unreachable_code)] text("unreachable")
        })
        .get("/ok", |_req: Request| async { text("ok") });

    let addr = spawn_server(router.into(), MiddlewareRegistry::new(), 4).await;

    // 1つ目: パニックはサニタイズされた500に変換される。
    let (s1, body) = send_get(addr, "/panic").await;
    assert_eq!(s1, 500);
    let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(parsed["message"], "Internal Server Error");
    assert!(parsed.get("request_id").is_some());

    // 2つ目: リスナーは生き延びる。次のリクエストは正常である。
    let (s2, body2) = send_get(addr, "/ok").await;
    assert_eq!(s2, 200);
    assert_eq!(&body2[..], b"ok");
}
```

## ルーティングを経由せずにアクセッサーをテストする

ルーターを立ち上げることなく、`Request` のアクセッサー（`bearer_token`、`is_method`、`ip`、`is_json` など）をテストしたいことがあります。その手口は、`Request` を構築して `tokio::sync::oneshot::channel` を通じて送り返すことだけが仕事の、小さなhyperサービスを実行するハーネスです。

```rust
let (req_tx, req_rx) = tokio::sync::oneshot::channel::<suprnova::Request>();
// ... service_fnが次を行う、ループバックのhyperサービス:
//     let req = suprnova::Request::new(hyper_req);
//     let _  = req_tx.send(req);
//     空のボディを伴う200を返す
let req = req_rx.await.unwrap();
```

`framework/tests/http_request_accessors.rs` には、完全な `build_request(builder, body) -> Request` ヘルパーがあります。これをクレートごとに一度コピーすれば、あらゆるアクセッサーのテストはきれいに読めます。

```rust
#[tokio::test]
async fn bearer_token_extracts_simple_token() {
    let req = build_request(
        hyper::Request::builder()
            .method("GET")
            .uri("/api/users")
            .header("Authorization", "Bearer secret-token-123"),
        "",
    ).await;
    assert_eq!(req.bearer_token().as_deref(), Some("secret-token-123"));
}
```

この `Request` は本物です（本物の通信のやり取りからhyperによって生成されます）が、ルーティングもミドルウェアも実行されていません - テスト対象の単位がアクセッサー自身であるときに、まさに求めているものです。

## `Request` のビルダーフック

手元に `Request` があり、ルーティング層の一部をフェイクする必要があるときは、3つのビルダーメソッドが役立ちます。

```rust
impl Request {
    pub fn with_params(mut self, params: HashMap<String, String>) -> Self;
    pub fn with_route_pattern(mut self, pattern: String) -> Self;
    pub fn with_peer_addr(mut self, addr: std::net::IpAddr) -> Self;
}
```

これらは、サーバーがマッチしたルートをディスパッチするときに呼ぶのと同じメソッドです - `Router` は `matchit` が返った後に `with_params` を呼び、`req.route_pattern()` が解決できるように `with_route_pattern` を呼び、受理されたTCPソケットのIPが分かった時点で `with_peer_addr` を呼びます。テストの中では、同じセットアップをショートサーキットするために、これらを自分で呼びます。

```rust
let req = Request::new(hyper_req)
    .with_params(HashMap::from([("id".into(), "42".into())]))
    .with_route_pattern("/users/{id}".into())
    .with_peer_addr("192.168.1.10".parse().unwrap());

assert_eq!(req.param("id").unwrap(), "42");
assert_eq!(req.ip(), Some("192.168.1.10".parse().unwrap()));
```

## 知っておくべきこと

初めて書く人を捕まえる、フットガンの短いリストです。

- **`Incoming` はサーバーサイド専用です。** あなたのテストの中でこれを構築することはできません。TCPループバック（あるいはプロセス内のサービスキャプチャ）だけが唯一の道です - 「`Vec<u8>` のボディから `Request` を構築する」というコンストラクタはありません。
- **テスト間で状態を共有しないでください。** 各 `#[tokio::test]` は自分専用のランタイムを手にします - テストをまたいだ汚染は、通常グローバル（`once_cell`、`lazy_static`、環境変数）を共有していることを意味します。DBの状態については、[テスト](testing.md)の中の `TestDatabase` を参照してください。
- **クッキーには本物のクライアントが必要です。** 自動的なクッキージャーはありません - あるレスポンスの `Set-Cookie` を、次の `Cookie` へ手作業で通してください。パターンについては `framework/tests/auth_http_middleware.rs` を参照してください。
- **レスポンス後の終了処理のspawnは、ブロックしません。** `Terminable` を介して実行される副作用をアサートしたい場合は、それをポーリングしてください - レスポンスは、そのフックが実行される前にクライアントへ返ります。

## 各要素の実装場所

| 要素 | ファイル |
|---|---|
| `handle_request`、`handle_request_with_peer` | `framework/src/server.rs` |
| `Request::new`、`with_params`、`with_route_pattern`、`with_peer_addr` | `framework/src/http/request.rs` |
| `MiddlewareRegistry::new`、`append`、`prepend` | `framework/src/middleware/registry.rs` |
| ループバックのテストハーネス（標準） | `framework/tests/cors_middleware.rs` |
| `TestResponse`（上記トリプルに対する流暢なアサーション） | `framework/src/testing/response.rs` |
| `AssertableInertia`、`ReloadRequest`（流暢なInertiaページオブジェクトアサーション） | `framework/src/testing/inertia.rs` |
| プロセス内の `Request` キャプチャハーネス | `framework/tests/http_request_accessors.rs` |
| パニック境界のテストパターン | `framework/tests/middleware_panic_safety.rs` |
| 認証 + ミドルウェアのエンドツーエンドパターン | `framework/tests/email_verified_middleware.rs` |

## 次のステップ

- [テスト](testing.md) - `#[suprnova_test]`、`TestDatabase`、`describe!`/`test!`/`expect!` マクロ、そしてユニットレベルの表面
- [エラー モデル](error-model.md) - あらゆるエラーレスポンスが使うJSONの形、5xxのサニタイズ規則、そしてテスト本体の中で `request_id` が何を意味するか
- [ミドルウェア](middleware.md) - ここでテストするミドルウェアを書くこと、そしてグローバル対ルートのライフサイクル
- [ルーティング](routing.md) - 本番環境とテストの両方でマウントする `Router`、ルートパラメータ、ルート名、署名付きURL
- [認証](authentication.md) - `Auth` ファサード、`Authenticatable`、ガード、そして `Auth::set_user` が `handle_request` がインストールするリクエストスコープとどのように相互作用するか
