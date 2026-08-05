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

async fn spawn_server(router: Router, accepts: usize) -> SocketAddr {
    let router = Arc::new(router);
    let middleware = Arc::new(MiddlewareRegistry::new());

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
    let addr = spawn_server(router, 1).await;

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

エラーレスポンスについては、ボディの形は固定されており、[エラー モデル](error-model.md)に文書化されています - `message`、`errors`、`request_id`、そして任意の `debug_message` です。`request_id` キーは常に存在します（リクエストスコープの外側では `null` の場合があります）- これが、request-idミドルウェアが実行されたことを確認するときにアサートすべきものです。

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

ルートモデルバインディングは、`/users/{id}` を型付きの `User` 引数へ変えます。このバインディングはハンドラのエクストラクターチェーンの一部として実行されるため、通常のエンドツーエンドテストが、これを無償で行使します。

```rust
#[suprnova::model(table = "users")]
pub struct User {
    pub id: i64,
    pub email: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[tokio::test]
async fn show_user_binds_from_route_param() {
    // テスト用ユーザーをモデル経由で挿入する。データベースのセットアップは省略 -
    // `TestDatabase` のパターンについてはテストの章を参照。
    let user = User::create(suprnova::attrs! {
        email: "bound@example.com"
    }).await.unwrap();

    let router = Router::new().get("/users/{id}", |req: Request| async move {
        let id: i64 = req.param("id")?.parse()
            .map_err(|_| suprnova::FrameworkError::param_parse("id", "i64"))?;
        let user = User::find_or_fail(id).await?;
        suprnova::http::json(serde_json::json!({ "email": user.email }))
    });

    let addr = spawn_server(router, 1).await;
    let (status, body) = send_get(addr, &format!("/users/{}", user.id)).await;

    assert_eq!(status, 200);
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["email"], "bound@example.com");
}
```

ルーターもTCPループもない、バインディングを分離してテストするには、[`Request` のビルダーフック](#request-のビルダーフック)（下記）にある `Request::with_params(...)` で、ルートパラメータを自分自身で合成してください。これは、`framework/tests/data_route_params.rs` が、合成されたパラメータに対して `#[derive(Data)]` のエクストラクターをテストするのに使っているパターンです。

## 認証フローをエンドツーエンドでテストする

本物の認証フローのテストは、ユーザーを登録し、ログインルートを駆動し、レスポンスからセッションクッキーを取り出し、それを保護されたルートに対して再送信します。4つのステップで、すべて通信レベルです。

```rust
#[tokio::test]
async fn login_flow_issues_session_cookie() {
    // 1. ブートストラップ: ユーザーを作成する。
    Auth::password()
        .register("alice@example.com", "longpassword123")
        .await.expect("register");

    // 2. ルートをマウントする。
    let router = Router::new()
        .post("/login", login_handler)
        .get("/dashboard", |_req: Request| async { text("dashboard") });
    let addr = spawn_server(router, 2).await;

    // 3. ログインを駆動する。Set-Cookieヘッダーをキャプチャする。
    let login = post_json(addr, "/login", serde_json::json!({
        "email": "alice@example.com",
        "password": "longpassword123",
    })).await;
    assert_eq!(login.status, 200);
    let cookie = extract_session_cookie(&login.headers);

    // 4. 保護されたルートに対して、そのクッキーを再生する。
    let (status, body) = get_with_cookie(addr, "/dashboard", &cookie).await;
    assert_eq!(status, 200);
    assert_eq!(&body[..], b"dashboard");
}
```

`extract_session_cookie` と `get_with_cookie` は、単純なヘッダーとクッキーの配線です - 完全な実装は `framework/tests/auth_http_middleware.rs` にあります。要点はこうです。フロー全体が、本物の `SessionMiddleware`、`Auth` の本物の認証ガード、本物の `Authenticatable` の解決を通じて実行されます。このテストが検証しているのは、実際の通信契約であり、それを模したモックではありません。

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

    let addr = spawn_server(router, 4).await;

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
| プロセス内の `Request` キャプチャハーネス | `framework/tests/http_request_accessors.rs` |
| パニック境界のテストパターン | `framework/tests/middleware_panic_safety.rs` |
| 認証 + ミドルウェアのエンドツーエンドパターン | `framework/tests/email_verified_middleware.rs` |

## 次のステップ

- [テスト](testing.md) - `#[suprnova_test]`、`TestDatabase`、`describe!`/`test!`/`expect!` マクロ、そしてユニットレベルの表面
- [エラー モデル](error-model.md) - あらゆるエラーレスポンスが使うJSONの形、5xxのサニタイズ規則、そしてテスト本体の中で `request_id` が何を意味するか
- [ミドルウェア](middleware.md) - ここでテストするミドルウェアを書くこと、そしてグローバル対ルートのライフサイクル
- [ルーティング](routing.md) - 本番環境とテストの両方でマウントする `Router`、ルートパラメータ、ルート名、署名付きURL
- [認証](authentication.md) - `Auth` ファサード、`Authenticatable`、ガード、そして `Auth::set_user` が `handle_request` がインストールするリクエストスコープとどのように相互作用するか
