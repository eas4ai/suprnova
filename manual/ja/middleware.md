# ミドルウェア

ミドルウェアはリクエストハンドラをラップします。ハンドラがリクエストを目にする前に一度実行され、ハンドラがレスポンスを返した後にもう一度実行されます。そのため、横断的な処理 - 認証、ロギング、CORS、スロットリング、計測、リクエストやレスポンスの変換 - を置く場所になります。Suprnovaの表面は、Laravelのユーザーがすでに知っているものと同じです: リクエストを転送するか、ショートサーキットするか、レスポンスを変更するかを決める `handle(request, next)` メソッドです。

## トレイト

ミドルウェアは、`Middleware` を実装する構造体です:

```rust
use suprnova::{async_trait, HttpResponse, Middleware, Next, Request, Response};

pub struct LoggingMiddleware;

#[async_trait]
impl Middleware for LoggingMiddleware {
    async fn handle(&self, request: Request, next: Next) -> Response {
        // 前処理: ハンドラの前に実行されます。
        println!("--> {} {}", request.method(), request.path());

        // 次のミドルウェアへ転送します（これが最後の層であれば
        // ハンドラへ転送します）。
        let response = next(request).await;

        // 後処理: ハンドラが返った後に実行されます。
        println!("<-- complete");

        response
    }
}
```

`handle` がすべきことは3つあり、どのリクエストでもそのうちの1つだけを行えばよいことになっています:

- **転送する。** `next(request).await` を呼び出して、次の層に制御を渡します。返される `Response` は、それより上にあるすべての層が目にするものです。
- **ショートサーキットする。** `next` を呼ばずに `Err(HttpResponse::...)` を返します。フレームワークは `Response`（`Result<HttpResponse, HttpResponse>`）の両方の分岐を1つのレスポンスへと収束させます - `Err` はクラッシュではなく、レスポンスです。[エラー モデル](error-model.md)を参照してください。
- **変更する。** 転送する前にリクエストを、あるいは後でレスポンスを変更します。

`Next` は `Arc<dyn Fn(Request) -> MiddlewareFuture + Send + Sync>` です - `Request` から `Response` への非同期関数として扱ってください。

## スタブを生成する

CLIは、動作するミドルウェアファイルをスキャフォルドします:

```bash
suprnova make:middleware Auth         # → src/middleware/auth.rs (AuthMiddleware)
suprnova make:middleware RateLimit    # → src/middleware/rate_limit.rs
suprnova make:middleware CorsMiddleware  # "Middleware" というサフィックスを付けても同じ結果です
```

生成されるファイルはTODOのスタブではありません - ラップされたリクエストの時間を計測し、`RequestIdMiddleware` によってインストールされたリクエストごとのIDとともに、入出力のイベントをログに記録する、本物のミドルウェアです。本文を、実際に必要なものへと置き換えてください。

## ミドルウェアの登録

スコープに応じて、インストールする場所は3つあります:

### グローバル

あらゆるリクエストで、登録順に実行されます。`bootstrap()` の内部で `global_middleware!` マクロを使ってください:

```rust
// src/bootstrap.rs
use suprnova::{global_middleware, FrameworkError};
use crate::middleware;

pub async fn bootstrap() -> Result<(), FrameworkError> {
    global_middleware!(middleware::LoggingMiddleware);
    global_middleware!(middleware::CorsMiddleware);
    Ok(())
}
```

`global_middleware!(M)` は `register_global_middleware(M)` に展開されます。登録は**具象型ごとにべき等**です - 同じ構造体を2回登録しても、最初の登録が保たれ、デバッグログが出力されるだけです。これにより、起動をやり直すこと（テスト、ホットリロード、1つのプロセス内で複数の `Server` インスタンスを持つこと）が安全になります。同じ振る舞いを異なる設定で複数コピーしたい場合は、それぞれを別個のニュータイプでラップしてください。

### ルートごと

`routes!` マクロから作られるルート定義に、`.middleware(M)` をチェーンします:

```rust
// src/routes.rs
use suprnova::{routes, get};
use crate::{controllers, middleware::AuthMiddleware};

routes! {
    get!("/", controllers::home::index).name("home"),
    get!("/public", controllers::home::public),

    get!("/protected", controllers::dashboard::index)
        .middleware(AuthMiddleware),
    get!("/admin", controllers::admin::index)
        .middleware(AuthMiddleware),
}
```

### グループごと

`group(...)` ブロックの中のすべてのルートにミドルウェアを適用します:

```rust
use suprnova::Router;
use crate::middleware::{ApiMiddleware, AuthMiddleware};
use crate::controllers::{user, admin};

Router::new()
    // 公開ルート - ミドルウェアなし。
    .get("/", home_handler)
    .get("/login", login_handler)

    // /api 配下のすべてのルートが ApiMiddleware を運びます。
    .group("/api", |r| {
        r.get("/users", user::index)
         .post("/users", user::store)
         .get("/users/{id}", user::show)
    })
    .middleware(ApiMiddleware)

    // 管理者ルートは認証を共有します。
    .group("/admin", |r| {
        r.get("/dashboard", admin::dashboard)
         .get("/settings", admin::settings)
    })
    .middleware(AuthMiddleware);
```

## 実行順序

実行時には、チェーンは外側から内側へと実行されます:

```
Request  →  RequestId  →  グローバル  →  グループミドルウェア  →  ルートミドルウェア  →  ハンドラ
                                                                                             │
Response ←  RequestId  ←  グローバル  ←  グループミドルウェア  ←  ルートミドルウェア  ←  ハンドラ
```

最初に追加されたミドルウェアが最初に実行されます。戻ってくる際には順序が逆転します - `MiddlewareChain::execute` は、各層の後処理を、その前の層の内側にネストします。

ミドルウェアが `Err(response)` でショートサーキットすると、チェーンは直ちに巻き戻ります: ショートサーキットより上にあるすべての層は戻ってくる際にそのレスポンスを目にしますが、下にある層（ハンドラに近い側)は実行されません。

### グループミドルウェアは平坦化され、積み重なりません

これは重要なので、はっきりと述べておく価値があります。**ルートグループのミドルウェアは、別個のランタイム層ではありません。** `GroupBuilder::try_finalize` が実行されると、グループのミドルウェアは、グループ化された各ルートの `(method, pattern)` ミドルウェアリストにコピーされます。実行時までには、グループミドルウェアは、ルートに直接付けられたミドルウェアと見分けがつかなくなります。

これには2つの帰結があります:

- 実行時の順序は正しいままです（グループミドルウェアは先に登録されるため、ルートミドルウェアより前に実行されます）が、**イントロスペクションでは、グループ由来のミドルウェアとルート由来のミドルウェアを区別できません**。
- ミドルウェアは、生のパス（`/posts/42`）ではなく、マッチしたパターン（`"/posts/{id}"`）をキーにしているため、パラメータ付きルートに対するグループミドルウェアが確実に発火します。

平坦化のパスについては `framework/src/routing/group.rs` を、実行ループについては `framework/src/middleware/chain.rs` を参照してください。

## ショートサーキット

ハンドラに到達する前にリクエストをブロックするため、早期にリターンします:

```rust
use suprnova::{async_trait, HttpResponse, Middleware, Next, Request, Response};

pub struct RequireApiKey;

#[async_trait]
impl Middleware for RequireApiKey {
    async fn handle(&self, request: Request, next: Next) -> Response {
        if request.header("X-Api-Key").is_none() {
            return Err(HttpResponse::text("Unauthorized").status(401));
        }
        next(request).await
    }
}
```

チェーンは `Result<HttpResponse, HttpResponse>` を1つのレスポンスに収束させるため、`Err(...)` は、異なる役割を持つだけの、ただのレスポンスです。このミドルウェアより上にある層は、戻ってくる際にそれを観測し、後処理を行えます。

## パニック安全性

`MiddlewareChain::execute` はパニックを**捕捉しません** - あらゆるミドルウェアやハンドラでのパニックは、他のあらゆる非同期関数と同じように、そのまま外側へ巻き戻ります。リクエストパスの安全網は、1つ上の層、サーバー境界にある `execute_chain_safely` に存在し、これがチェーンを `catch_unwind` でラップして、パニックをリクエストIDを添えたサニタイズ済みの500へと変換し、可観測性リスナーのために `ErrorOccurred` をディスパッチします。パニックリカバリの全体像については[リクエスト ライフサイクル](lifecycle.md)を参照してください。

この分割は意図的なものです: 標準化されたパニック処理は、レイヤーに依存しないプリミティブの中で重複させるのではなく、リクエストライフサイクルがそれを所有する場所でちょうど一度だけ行われます。その境界の外でチェーンを駆動する利用者は、自分自身の `catch_unwind` に責任を持ちます。

## 組み込みのミドルウェア

網羅的ではない地図です。それぞれ、インストールできる状態で出荷されます - ほとんどは設定用の構造体を必要とし、どれもスキャフォルドを必要としません。

| ミドルウェア | 目的 |
|---|---|
| `RequestIdMiddleware` | 常に最も外側の層。リクエストごとにUUIDを割り当て、ログと `X-Request-Id` を通じてそれをタグ付けする |
| `TimeoutMiddleware` | レスポンスまでの時間を制限する。超過したときは503を返す（下記を参照） |
| `CorsMiddleware` | CORSのプリフライトを処理し、クロスオリジンのレスポンスを装飾する（下記を参照） |
| `CsrfMiddleware` | 設定可能な `OriginPolicy` を備えた、クッキーの二重送信によるCSRF保護 |
| `RateLimitMiddleware` / `ThrottleRequestsMiddleware` | トークンバケットとスライディングウィンドウのスロットリング。[レート リミット](rate-limiting.md)を参照 |
| `SessionMiddleware` | クッキー越しにセッションをロード/永続化する。`req.session()` を支える |
| `AuthMiddleware` / `GuestMiddleware` / `BearerTokenMiddleware` | 認証ガードの所属チェック。[認証](authentication.md)を参照 |
| `LoginThrottleMiddleware` / `EnsureEmailVerifiedMiddleware` / `TwoFactorChallengeMiddleware` | 認証フローのゲート。[認証フロー](auth-flows.md)を参照 |
| `MaintenanceMiddleware` | キャッシュまたはファイルシステムのメンテナンスフラグが設定されているとき、503を返す |
| `InertiaHeadersMiddleware` / `InertiaVersionMiddleware` / `Inertia303Middleware` / `EncryptHistoryMiddleware` | Inertiaプロトコル。すべてのレスポンスへの `Vary: X-Inertia` と、空の200のリダイレクトの戻し、アセットバージョンの409の跳ね返し、GET以外のリダイレクトでの302→303、履歴の暗号化です。最初の3つは `Inertia::install` によって登録されます。[Inertia レスポンス](frontend-inertia-responses.md#bootstrap-inertia-install)を参照 |
| `IncludeMiddleware` | `#[derive(Data)]` の部分的なリロードのための、フィールドごとのincludeの集合 |

### リクエストのタイムアウト

`TimeoutMiddleware` は、ハンドラがレスポンスを*生成する*のにかけてよい時間を制限します。そうしなければ、遅いハンドラやハングしたデータベースクエリが、コネクションを無期限に開いたままにしかねません。タイムアウトは、期限を超過した時点で `503 Service Unavailable` を返します。

```rust
// src/bootstrap.rs - すべてのHTTPルートに30秒の上限。
use suprnova::{global_middleware, TimeoutMiddleware};

global_middleware!(TimeoutMiddleware::default()); // DEFAULT_TIMEOUT = 30s
```

```rust
// 単一のエンドポイントを5秒へ絞り込む。
use suprnova::{Router, TimeoutMiddleware};

Router::new()
    .get("/report", heavy_report_handler)
    .middleware(TimeoutMiddleware::seconds(5));
```

`TimeoutMiddleware::new(Duration)` は任意の期間を受け付け、`TimeoutMiddleware::seconds(n)` は整数秒の短縮形です。

グローバルミドルウェアは、ルートのミドルウェアの**外側**で走ります。そのため、グローバルなタイムアウトは外側の上限であり、ルートごとのタイムアウトは、特定のルートをより*厳しく*することしかできません - 短いほうの期限が先に発火します。1つのルートをグローバルなデフォルトより長く走らせたい場合は、グローバルな値を上げるか、そのエンドポイントを除外したルートグループへグローバルミドルウェアをスコープしてください。

ストリーミングのレスポンス（`HttpResponse::sse(...)`、`HttpResponse::stream_bytes(...)`）は、当然ながら適用外です: ハンドラは、ミドルウェアチェーンの完了後にhyperがドレインするレイジーなボディとともに、ただちに返るからです。WebSocketのアップグレードも、明示的にスキップされます。キャンセル安全性のセマンティクスについては、[リクエスト タイムアウト](timeout.md)を参照してください。

### CORS

`CorsMiddleware` は、クロスオリジンのページがあなたのレスポンスを読めるようにするためにブラウザが必要とする `Access-Control-*` のヘッダーを追加し、単純でないクロスオリジンの呼び出しの前にブラウザが送るプリフライトの `OPTIONS` リクエストに応答します。同一オリジンのアプリ（デフォルトのInertiaのセットアップ）には必要ありません - これが問題になるのは、*異なる*オリジンのブラウザがあなたのAPIを呼ぶときだけです。

プリフライトが到達できるよう、CORSは**グローバルに**インストールしなければなりません（プリフライトはルートに一致することが決してないため、ルートごとのCORSミドルウェアは、それを目にすることが決してありません）。緩いデフォルトは意図的に存在しません - オリジンのポリシーは明示的に選んでください:

```rust
// src/bootstrap.rs
use suprnova::{global_middleware, CorsConfig, CorsMiddleware};

global_middleware!(CorsMiddleware::new(
    CorsConfig::allow_origins(["https://app.example", "https://admin.example"])
        .allow_credentials(true)
        .max_age(std::time::Duration::from_secs(600)),
));
```

`CorsConfig::any_origin()` は、`Access-Control-Allow-Origin: *` を明示的にオプトインします。ビルダーのメソッドは、`.methods([...])`、`.allow_headers([...])` / `.allow_any_headers()`、`.expose_headers([...])`、`.paths([...])`（CORSをURLのパターンへスコープする）、`.allow_origin_patterns([regex...])`、`.skip_when(|req| bool)`、`.allow_credentials(bool)`、`.max_age(Duration)` です。Laravelの名前のエイリアスも一緒に出荷されるため（例えば `.supports_credentials`、`.allowed_methods`）、Laravelの設定はそのまま対応づけられます。

`Access-Control-Allow-Origin: *` は、認証情報と一緒には無効です - ブラウザがそれを拒否します。`.allow_credentials(true)` が設定されているとき、ミドルウェアは常に `*` ではなく具体的なリクエストの `Origin` をエコーするため、無効な組み合わせが出力されることは決してありません。ワイルドカードでないレスポンスには `Vary: Origin` も付くため、共有キャッシュは正しいままです。[CORS](cors.md)を参照してください。

## パイプライン - Laravelの `Illuminate\Pipeline\Pipeline`

`Pipeline` は、Laravelのパイプラインクラスに相当するSuprnovaの仕組みです - `MiddlewareChain` の上に構築されたフルーエントなビルダーで、Laravelのユーザーがすでに知っている `send / through / pipe / then / then_return / finally_with` の形を反映しています。リクエストライフサイクルの外側でミドルウェアチェーンを組み立てたいとき（ジョブ、CLIコマンド、単発の統合テストなど）に便利です:

```rust
use suprnova::{Pipeline, Request};

let response = Pipeline::new()
    .send(request)
    .through([AuthMiddleware, LoggingMiddleware])
    .pipe(CorsMiddleware::new(cors_config))
    .finally_with(|| tracing::info!("pipeline complete"))
    .then(|req| async move { handler(req).await })
    .await;
```

Rust側のエイリアスは、Laravelの名前と一緒に出荷されます: `send` に対する `with_request`、`through` に対する `with_middleware`、`pipe` に対する `push`、`finally_with` に対する `on_finally`、`then` に対する `execute`。あなたのコードベースで読みやすいほうを使ってください。

| Pipelineのメソッド | Laravel | Rustのエイリアス | 目的 |
|---|---|---|---|
| `send(request)` | `send($passable)` | `with_request(request)` | 通されるリクエストを設定する |
| `through(iter)` | `through($pipes)` | `with_middleware(iter)` | パイプのリストを置き換える |
| `through_boxed(iter)` | - | - | 事前にボックス化されたミドルウェアでパイプのリストを置き換える |
| `pipe(M)` | `pipe($pipes)` | `push(M)` | 単一のミドルウェアを追加する |
| `pipe_boxed(M)` | - | - | 事前にボックス化されたミドルウェアを追加する |
| `then(destination)` | `then($destination)` | `execute(destination)` | 目的地のハンドラでチェーンを実行する |
| `then_with(req, dst)` | - | - | 通されるものをその場で上書きする |
| `then_return()` | `thenReturn()` | - | チェーンを実行し、204 No Contentを返す |
| `finally_with(F)` | `finally($callback)` | `on_finally(F)` | 目的地が解決した後に実行する |

## 終了処理ミドルウェア - レスポンス後のフック

終了処理ミドルウェアは、レスポンスがクライアントに送信された*後に*実行されます。レスポンスをブロックする必要のない、遅いIOに使ってください: セッションの永続化、監査ログ、メトリクスのフラッシュなどです。

Suprnovaは、これを `Middleware` とは別の、専用の `Terminable` トレイトとして出荷します。そのため、リクエストパスと終了処理パスは、はっきりと型付けされたまま保たれます。ある型は、どちらか一方、あるいは両方を実装できます:

```rust
use suprnova::{Terminable, TerminationSnapshot, register_terminable, async_trait};

pub struct AuditLogTerminator;

#[async_trait]
impl Terminable for AuditLogTerminator {
    async fn terminate(&self, snapshot: &TerminationSnapshot) {
        tracing::info!(
            method = %snapshot.method,
            path = %snapshot.path,
            status = snapshot.status,
            "request handled",
        );
    }
}

// bootstrap.rs にて
register_terminable(AuditLogTerminator);
```

サーバーは、あらゆるレスポンス（4xxと5xxも含む）の後、登録順に登録済みの終了処理を反復処理し、それぞれをawaitします。エラーは `tracing::error!` でログに記録された上で握りつぶされます - レスポンスはすでに建物を出てしまっているため、それを表面化させる相手はもう誰もいません。

登録は具象型ごとにべき等です。`registered_terminables()`、`terminable_count()`、`has_terminable::<T>()` は、テストや起動時の診断のためのイントロスペクションを提供します。

## 名前付きエイリアスとグループ

文字列キーのミドルウェア（Laravelの `middlewareAliases` / `middlewareGroups`）を好む利用者のために、Suprnovaはプロセスグローバルなエイリアス + グループのレジストリを出荷します:

```rust
use suprnova::middleware::{
    register_middleware_alias, register_middleware_group,
    resolve_middleware_group,
};

// エイリアスはファクトリーのクロージャです - 解決のたびに新しく呼び出されるため、
// ルート登録ごとに独立したミドルウェアのインスタンスが生まれます。
register_middleware_alias("auth", || AuthMiddleware::new());
register_middleware_alias("throttle", || ThrottleRequestsMiddleware::default());

// グループはエイリアスをまとめます。入れ子のグループもサポートされています。
register_middleware_group("api", ["auth".into(), "throttle".into()]);
register_middleware_group("web", ["session".into(), "auth".into()]);

// 起動時、あるいはルートごとに Vec<BoxedMiddleware> へ解決します。
let api_mws = resolve_middleware_group("api")?;
```

`resolve_middleware_group` は、以下の場合に `Err(MiddlewareResolveError)` を返します:

- `UnknownGroup(name)` - 名前付きグループが一度も登録されていない
- `UnknownAlias { group, missing }` - グループのエントリが既知のエイリアスではない
- `UnknownNestedGroup { group, missing }` - ネストしたグループの参照が解決に失敗する
- `CycleDetected { group }` - グループの定義が再帰している

エイリアスやグループの登録は、同じ名前に対しては**後勝ち**です。Laravelの再代入可能なカーネル配列を反映しています。

## ミドルウェアの優先順位

`prepend_middleware_priority::<M>()` / `append_middleware_priority::<M>()` は、プロセスグローバルな優先順位リストに `TypeId` を登録します - Laravelの `Kernel::$middlewarePriority` に相当するSuprnovaの仕組みです。型がリストの中でより前に現れるミドルウェアは、登録順にかかわらずチェーンの先頭側にソートされます:

```rust
use suprnova::{append_middleware_priority};

// SessionMiddleware は、登録された順序にかかわらず、常に AuthMiddleware
// より先に実行されます。
append_middleware_priority::<SessionMiddleware>();
append_middleware_priority::<AuthMiddleware>();
```

`middleware_priority()` は、診断のため、あるいは独自のソーターを駆動したい組み込み先のために、現在の `Vec<TypeId>` のスナップショットを返します。

## レジストリのイントロスペクション

`register_global_middleware` に加えて、レジストリは以下を公開します:

| 表面 | Laravel | 目的 |
|---|---|---|
| `prepend_global_middleware(M)` | `prependMiddleware` | チェーンの先頭に挿入する |
| `has_global_middleware::<M>()` | `hasMiddleware` | 型 `M` が登録されているかどうか |
| `global_middleware_count()` | - | 現在登録されているグローバルの数 |
| `MiddlewareRegistry::from_global()` | - | グローバルレジストリを、サーバーごとのレジストリにスナップショットする |
| `MiddlewareRegistry::prepend(M)` | - | レジストリインスタンスに対するビルダー形式の先頭挿入 |
| `MiddlewareRegistry::append_boxed(M)` | - | 事前にボックス化されたミドルウェアを追加する |
| `MiddlewareRegistry::prepend_boxed(M)` | - | 事前にボックス化されたミドルウェアを先頭に挿入する |
| `MiddlewareRegistry::len()` / `is_empty()` | - | ビルダーのイントロスペクション |

`MiddlewareRegistry::from_global()` は、呼び出された時点でグローバルレジストリをスナップショットします。サーバーを構築する**前に**、すべてのグローバルミドルウェアを登録してください - サーバーが構築された**後**に行われた `global_middleware!` の呼び出しは、遡って適用されることはありません。そのため、稼働中のサーバーのミドルウェアスタックが、その足元で変化することはありません。

## ファイル構成

いくつかミドルウェアを持つようになった時点での、典型的な構成です:

```
src/
├── middleware/
│   ├── mod.rs          # mod + pub use
│   ├── auth.rs         # AuthMiddleware
│   ├── logging.rs      # LoggingMiddleware
│   └── audit.rs        # AuditLogTerminator
├── bootstrap.rs        # global_middleware! + register_terminable
├── routes.rs           # ルートごとの .middleware(M)
└── main.rs
```

`make:middleware` は `src/middleware/mod.rs` を同期させ続けます - ファイルが生成されると、新しい `mod foo;` 宣言と、対応する `pub use foo::FooMiddleware;` の再エクスポートを追記します。

## Suprnovaが異なる設計を選んだ理由

Laravelは、ミドルウェアのクラスを `app/Http/Kernel.php` に登録し、コンテナを通じてそれらを解決します。コンテナは、依存性を注入するために、コンストラクタの型ヒントに対するリフレクションを行います。PHPのリクエスト単位プロセスモデルでは、カーネルはリクエストごとに再構築されるため、リフレクティブな解決のコストはリクエストごとに支払われ、リクエストとリクエストの間には残りません。

Suprnovaのプロセスモデルは、1つのバイナリが多数のスレッドにまたがって多数の並行リクエストを処理するというものです。リクエストごとに新しいチェーンを構築すると、グローバルミドルウェアリストに対する同期点が強制され、あらゆるリクエストのあらゆる層で `Arc<dyn Middleware>` を再割り当てすることになってしまいます。そこで代わりに:

- グローバルミドルウェアは、起動時に `OnceLock<RwLock<Vec<...>>>` へと登録され、べき等な登録のために `TypeId` でキー付けされます。
- `MiddlewareRegistry::from_global()` は、サーバー構築時に一度だけグローバルリストをスナップショットします。リクエストごとのチェーンは、そのスナップショットを再利用します。
- チェーンそのものは、`Arc<dyn Fn>` のクロージャをネストすることで構成されます。そのため、リクエストごとの作業は、新たな割り当てではなく、層ごとに1回の `Arc::clone` になります。

利用者に見える表面 - `handle(request, next)`、`global_middleware!` マクロ、名前付きエイリアス、優先順位リスト、終了処理フック - は、Laravelの開発者が手を伸ばすものと同じです。その下にある機構は、PHPのリクエストごとの再構築を、Rustらしい起動時スナップショットモデルへと置き換え、フレームワークがレジストリを奪い合うことなく並行リクエストを処理できるようにしています。

## 次のステップ

- [リクエスト ライフサイクル](lifecycle.md) - チェーンがどこで実行され、パニックがサーバー境界でどのように捕捉されるか
- [エラー モデル](error-model.md) - `Result<HttpResponse, HttpResponse>` が実際に何を意味し、ショートサーキットがどのように収束するか
- [リクエスト タイムアウト](timeout.md) - `TimeoutMiddleware` のキャンセル安全性の詳細
- [CORS](cors.md) - プリフライトの処理、オリジンパターン、パスのスコープ
- [レート リミット](rate-limiting.md) - `RateLimitMiddleware` / `ThrottleRequestsMiddleware` と `BackendErrorPolicy`
- [ルーティング](routing.md) - `routes!`、`Router`、`group(...)` が何に展開されるか
