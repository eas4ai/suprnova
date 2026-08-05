# ルーティング

ルーティングとは、Suprnovaが受信したHTTPリクエストをハンドラ呼び出しへと変換する仕組みです。ルートは `routes!` マクロを使って `src/routes.rs` に宣言します（あるいは `Router` を手で組み立てます）。そして `Server::from_config` がそのルーターを受け取り、プロセスが生きている間ずっとそれを実行し続けます。Laravelの `routes/web.php` と同じ形ですが、ファサードの代わりにRustの型を使います。

```rust
// src/routes.rs
use suprnova::{routes, get, post, put, delete};
use crate::controllers;

routes! {
    get!("/", controllers::home::index).name("home"),
    get!("/users", controllers::users::index).name("users.index"),
    get!("/users/{id}", controllers::users::show).name("users.show"),
    post!("/users", controllers::users::store).name("users.store"),
    put!("/users/{id}", controllers::users::update).name("users.update"),
    delete!("/users/{id}", controllers::users::destroy).name("users.destroy"),
}
```

このマクロは `pub fn register() -> Router { ... }` に展開されます。あなたの `bootstrap` からこれを呼び出し、その結果をサーバーに渡してください。

## HTTP メソッド

メソッドごとに1つのマクロがあります。7つすべてが「パス、ハンドラ」のペアを受け取り、`.name(...)` や `.middleware(...)` をチェーンできるビルダーを返します。

| マクロ | メソッド | 用途 |
|---|---|---|
| `get!`     | GET     | 読み取りエンドポイント、静的ページ |
| `post!`    | POST    | リソースの作成 |
| `put!`     | PUT     | 全体を置き換える更新 |
| `patch!`   | PATCH   | 部分更新（RFC 5789） |
| `delete!`  | DELETE  | 削除 |
| `head!`    | HEAD    | ヘッダーのみのプローブ（明示的に登録されていない場合、HEADはRFC 9110 § 9.3.2に従ってGETのレジストリにフォールバックします） |
| `options!` | OPTIONS | 機能検出、`Accept-Patch`。CORSプリフライトはルーターに到達する前に `CorsMiddleware` が応答するため、通常はこれを使う必要はありません |

```rust
use suprnova::{routes, get, post, patch, delete};

routes! {
    get!("/articles", controllers::articles::index),
    post!("/articles", controllers::articles::store),
    patch!("/articles/{id}", controllers::articles::update),
    delete!("/articles/{id}", controllers::articles::destroy),
}
```

どのマクロも、パスが `/` で始まることをコンパイル時にチェックします - 先頭のスラッシュが欠けていると、リクエストではなくビルドが失敗します。

### 複数メソッドと `any!`

`any!` は、1つのハンドラを一般的な7つのHTTPメソッドすべてに対して登録します。HTTPが送ってくるものは何でも受け入れる必要があるWebhook受信エンドポイントなどに使ってください。

```rust
use suprnova::{routes, any};

routes! {
    any!("/webhooks/inbound", controllers::webhooks::inbound)
        .name("webhooks.inbound")
        .middleware(SignatureCheck),
}
```

複数あるメソッドのうち一部だけを1つのハンドラで共有したい場合は、ビルダーAPIと `Router::methods` を使ってください:

```rust
use suprnova::Router;
use hyper::Method;

let router = Router::new()
    .methods(&[Method::PUT, Method::PATCH], "/posts/{id}", update_post)
    .name("posts.update")
    .middleware(AuthMiddleware);
```

`.name(...)` と `.middleware(...)` は、そのルートが登録されているすべてのメソッドに展開されます。そのため、呼び出し側がどのメソッドで逆引きしても、同じURLが得られます。

### WebSocket ルート

`ws!` は、長時間生存するアップグレードハンドラを登録します。このマクロは同じ `routes!` 本体の一部です - 詳細は[WebSocket](websockets.md)で扱います。

## ルートパラメータ

動的セグメントには波括弧（`{id}`）を使います。馴染みやすさのため、SuprnovaはExpress/Rails形式のコロン（`:id`）も受け付け、パターンを `matchit` に渡す前に波括弧へ正規化します。

```rust
routes! {
    get!("/users/{id}", controllers::users::show),       // matchit ネイティブ
    get!("/users/:id", controllers::users::show),        // Express/Rails - 同じもの
    get!("/posts/{post_id}/comments/{comment_id}", controllers::comments::show),
}
```

コロンは、パスセグメントの先頭にある場合にのみパラメータの開始として扱われます。そのため、セグメントの途中にあるリテラルなコロンはそのまま残ります（`/files/note:draft` はリテラルなルートのままで、`/files/{draft}` にはなりません）。

ハンドラの内部でリクエストからパラメータを読み取ります:

```rust
use suprnova::{Request, Response, HttpResponse};

pub async fn show(req: Request) -> Response {
    let user_id = req.param("id").unwrap_or("0");
    Ok(HttpResponse::text(format!("User ID: {}", user_id)))
}
```

`unwrap_or` を書かずに済む型付き抽出については、後述のルートモデル結合、または[コントローラー](controllers.md)の `#[handler]` を参照してください。

## ルートモデル結合

ハンドラのパラメータがSeaORMの `*::Model` 型である場合、`#[handler]` はマッチしたパスパラメータを抽出し、主キーの型としてパースした上で、データベースから該当する行を取得します。行が見つからない場合は404を、パラメータが主キーの型としてパースできない場合は400を返します。

```rust
use suprnova::{handler, json_response, Response};
use crate::models::users;

// ルート: GET /users/{user}
#[handler]
pub async fn show(user: users::Model) -> Response {
    json_response!({ "name": user.name, "email": user.email })
}
```

パラメータ名（`user`）は、`#[handler]` がマッチしたルートのパラメータの中から探し出すものです - そのためプレースホルダーは一致していなければなりません（`/users/{id}` ではなく `/users/{user}`）。

1つのシグネチャに複数のモデルがある場合も同じように動作し、フォームリクエストやプリミティブ型、`Request` と組み合わせて使えます:

```rust
// ルート: PUT /posts/{post}/comments/{comment}
#[handler]
pub async fn update(
    post: posts::Model,
    comment: comments::Model,
    form: UpdateCommentRequest,
) -> Response {
    // post と comment はすでに取得済みで、form はバリデーション済みです。
    json_response!({ "post_id": post.id, "comment_id": comment.id })
}
```

### 要件

結合は、`Entity` が `suprnova::database::EntityExt` を実装し、かつ主キーの型が `FromStr` を実装している、あらゆるSeaORMモデルに対して自動的に行われます。`EntityExt` の付随トレイトは、`Entity::find_by_pk(id)`、`::all()`、`::first()` などを提供します。ルートモデル結合は、パスパラメータによって駆動される単なる `find_by_pk` です。

```rust
// src/models/users.rs（従来のSeaORM形式のレイアウト）
pub use super::entities::users::*;
use sea_orm::entity::prelude::*;

impl ActiveModelBehavior for ActiveModel {}

// ルートモデル結合（およびLaravel形の読み取り表面）を有効にします。
impl suprnova::database::EntityExt for Entity {}
impl suprnova::database::EntityExtMut for Entity {}
```

モデルが `#[suprnova::model]` マクロ（[Eloquent](eloquent.md)にあるEloquent表面）で宣言されている場合は、これを直接使えます: `User::find_by_pk(id).await?`。`#[handler]` によるルートモデル結合は、それでもなお `*::Model` の形を期待します - ラッパー構造体ではなく、SeaORMのモデル型を渡してください。

### 結合は身元確認であり、認可ではありません

ルートモデル結合が答えるのは「この行は存在するか？」であり、「現在のユーザーはこの行を見ることを許可されているか？」には**答えません**。結合をむき出しにしたハンドラは、認証済みの任意のユーザーに、`/posts/N` を推測するだけであらゆる投稿を閲覧させてしまいます。結合されたモデルに対する認可は、`Gate::authorize` または `#[policy]` マクロを使って行ってください - [認可](authorization.md)を参照してください。

### 使用しない場合

`*::Model` パラメータ型を使わず、IDを抽出して手動でクエリを行います:

```rust
use suprnova::{handler, json_response, Response, FrameworkError};
use crate::models::users;
use suprnova::database::EntityExt;

#[handler]
pub async fn show(id: i32) -> Response {
    let user = users::Entity::find_by_pk(id)
        .await?
        .ok_or(FrameworkError::not_found("User"))?;
    json_response!({ "id": user.id, "name": user.name })
}
```

## 名前付きルート

名前は、URL生成のための安定した識別子を与えてくれます。`.name(...)` で名前を付けます:

```rust
routes! {
    get!("/", controllers::home::index).name("home"),
    get!("/users", controllers::users::index).name("users.index"),
    get!("/users/{id}", controllers::users::show).name("users.show"),
    post!("/users", controllers::users::store).name("users.store"),
}
```

名前はLaravelの規約 `<resource>.<action>` に従います - `users.show`、`posts.destroy`、`admin.dashboard` のように。トップレベルの `route(name, &[...])` ヘルパーで検索できます:

```rust
use suprnova::route;

let home = route("home", &[]);
//   Some("/")

let profile = route("users.show", &[("id", "123")]);
//   Some("/users/123")
```

`route` は `Option<String>` を返し、パラメータの値をパスとして安全な形にパーセントエンコードします（そのため `("slug", "a/b")` は `/posts/a%2Fb` になります - matchitにとって安全であり、`req.param("slug")` を通じて往復できます）。リダイレクト先やメールのリンクには、厳格な兄弟である `suprnova::routing::try_route` を使ってください。こちらは `Result<String, RouteUrlError>` を返し、埋まっていない `{placeholder}` セグメントを含むURLを出力することを拒否します。URL表面全体（署名付きURL、絶対URL、`Redirect::route`）については[URL 生成](urls.md)を参照してください。

ルート名はグローバルに一意であり、プロセス全体で共有されます。同じ名前を2つの異なるパスに登録すると起動時にパニックします - 見えない形で上書きされてしまうと、どちらの登録が勝つかによってリダイレクトの行き先が変わってしまうという、セキュリティ上の欠陥になっていたからです。失敗しうるバリアントには `RouteBuilder::try_name`（または `suprnova::routing::try_register_route_name`）を使ってください。

## ルートごとのミドルウェア

任意のルートビルダーに `.middleware(M)` をチェーンします:

```rust
use suprnova::{routes, get, post};
use crate::middleware::{AuthMiddleware, AdminMiddleware};

routes! {
    // 公開されたルート
    get!("/", controllers::home::index).name("home"),

    // 保護されたルート
    get!("/dashboard", controllers::dashboard::index)
        .name("dashboard")
        .middleware(AuthMiddleware),

    // 複数のミドルウェアは左から右へ合成されます（最も外側が先）
    get!("/admin", controllers::admin::index)
        .middleware(AuthMiddleware)
        .middleware(AdminMiddleware),
}
```

ルートローカルのミドルウェアは、あらゆるグローバルミドルウェア（`Server::with_middleware`）と、そのルートを包むあらゆるグループミドルウェアの後に実行されます。ミドルウェアのマップは `(method, path)` をキーにしているため、`POST /api/posts` に認証ミドルウェアを付けても、同じパスの公開された `GET /api/posts` に影響が漏れ出すことはありません。ミドルウェアの契約と、自分のミドルウェアを書く方法については[ミドルウェア](middleware.md)を参照してください。

## ルートグループ

`group!` は、共有のパスプレフィックスや共有ミドルウェアをまとめて切り出します:

```rust
use suprnova::{routes, get, post, group};
use crate::middleware::{AuthMiddleware, ApiMiddleware};

routes! {
    get!("/", controllers::home::index).name("home"),

    // 共有の /api プレフィックス + ミドルウェア
    group!("/api", {
        get!("/users", controllers::api::users::index).name("api.users.index"),
        post!("/users", controllers::api::users::store).name("api.users.store"),
        get!("/users/{id}", controllers::api::users::show).name("api.users.show"),
    }).middleware(ApiMiddleware),

    // 管理者向けエリア
    group!("/admin", {
        get!("/dashboard", controllers::admin::dashboard).name("admin.dashboard"),
        get!("/settings", controllers::admin::settings).name("admin.settings"),
    }).middleware(AuthMiddleware),
}
```

グループのプレフィックスは、それぞれのルートパスと連結されます。グループの内側にある `/` のルートは、グループのプレフィックスそのものに解決されます（`group!("/users", { get!("/", index) })` → `GET /users`）。

### ネストしたグループ

グループは、どんな深さにでもネストできます。プレフィックスは連結され、ミドルウェアは親から子へと継承されます:

```rust
routes! {
    group!("/api", {
        get!("/health", controllers::api::health),

        group!("/v1", {
            get!("/users", controllers::api::v1::users),

            group!("/admin", {
                get!("/stats", controllers::admin::stats),
            }).middleware(AdminMiddleware),
        }),
    }).middleware(AuthMiddleware),
}
```

| ルート | 実効パス | ミドルウェアチェーン |
|---|---|---|
| `/api/health` | `/api/health` | `AuthMiddleware` |
| `/api/v1/users` | `/api/v1/users` | `AuthMiddleware` |
| `/api/v1/admin/stats` | `/api/v1/admin/stats` | `AuthMiddleware` → `AdminMiddleware` |

ネストしたグループの内側にある単一のルートについて、実行順序は**最も外側のミドルウェアが最初**です: 親グループ → 子グループ → ルートローカル。ルートごとの `.middleware(...)` が最も内側で実行されます。

## フォールバックルート

`fallback!` は、他のどのルートにもマッチしなかったときに実行されるハンドラを登録します。独自の404ページに使ってください。

```rust
use suprnova::{routes, get, fallback};

routes! {
    get!("/", controllers::home::index),

    fallback!(controllers::errors::not_found),
}
```

```rust
// src/controllers/errors.rs
use suprnova::{Request, Response, HttpResponse};

pub async fn not_found(req: Request) -> Response {
    Ok(HttpResponse::text(format!("Page not found: {}", req.path()))
        .status(404))
}
```

フォールバックは、独自のミドルウェアチェーンをサポートします（`fallback!(handler).middleware(M)`）。フォールバックが登録されていない場合、フレームワークはプレーンテキストの `404 Not Found` を返します。

## リソースルーティング

標準的な7アクションのREST表面には、`ResourceController` を実装し、`Router` ビルダーを通じてそのリソースを登録します。`Route::resource()` と `Route::apiResource()` に対するLaravelパリティです。

```rust
use suprnova::{Router, ResourceController, ResourceAction, Request, Response, HttpResponse};
use std::pin::Pin;
use std::future::Future;

struct PostsCtl;

impl ResourceController for PostsCtl {
    fn index(&self, _req: Request) -> Pin<Box<dyn Future<Output = Response> + Send>> {
        Box::pin(async { Ok(HttpResponse::text("list")) })
    }
    fn show(&self, _req: Request) -> Pin<Box<dyn Future<Output = Response> + Send>> {
        Box::pin(async { Ok(HttpResponse::text("one")) })
    }
    // store / update / destroy / create / edit はデフォルトで404になります。
}

let router: Router = Router::new()
    .resource("posts", PostsCtl)
    .into();
```

オーバーライドしなかったメソッドは404を返します。フォームを描画するためだけに存在する2つのルート、`create` と `edit` を除きたい場合は `api_resource` を使ってください。

### デフォルトのルートと名前

| メソッド | パス | トレイトのメソッド | 名前 |
|---|---|---|---|
| GET    | `/posts`             | `index`   | `posts.index`   |
| GET    | `/posts/create`      | `create`  | `posts.create`  |
| POST   | `/posts`             | `store`   | `posts.store`   |
| GET    | `/posts/{post}`      | `show`    | `posts.show`    |
| GET    | `/posts/{post}/edit` | `edit`    | `posts.edit`    |
| PUT    | `/posts/{post}`      | `update`  | `posts.update`  |
| DELETE | `/posts/{post}`      | `destroy` | `posts.destroy` |

パスパラメータのデフォルトは、リソース名の単数形です - `posts` → `{post}`、`categories` → `{category}`。不規則な複数形は、そのまま最後のセグメントとして扱われます。`.parameter(...)` で上書きしてください。

### 制限と名前の変更

```rust
use suprnova::{Router, ResourceAction};

Router::new()
    .resource("posts", PostsCtl)
    .only(&[ResourceAction::Index, ResourceAction::Show])      // 2つの動作に絞り込む
    .names([("index", "posts.list")])                          // デフォルト名を変更
    .parameter("post_id")                                      // {post} → {post_id}
    .into();
```

一部の呼び出し箇所ではこちらのほうが読みやすい、Rust側のエイリアスです: `.only(...)` に対する `.keep(...)`、`.except(...)` に対する `.drop(...)`、`.names(...)` に対する `.rename(...)`。

### 一括登録

```rust
Router::new()
    .resources([
        ("posts",    Box::new(PostsCtl)    as Box<dyn ResourceController>),
        ("comments", Box::new(CommentsCtl) as Box<dyn ResourceController>),
    ])
    .api_resources([("authors", Box::new(AuthorsCtl) as Box<dyn ResourceController>)]);
```

### リソース全体を認可する

`authorize_resource::<U, R>()` は、生成されるすべてのルートに、慣例的な権限チェックをルートごとのミドルウェアとして付与します - Laravelの `authorizeResource` パリティです。これを使わない場合、すべてのコントローラー本体が `Gate::authorize` の呼び出しを忘れずに行わない限り、リソース表面はゲートされないままになります。`destroy` を1つ忘れるだけで、ゲートされていない削除が出荷されてしまいます。

```rust
use suprnova::{Router, Gate};

// アビリティは (ability, user type, resource marker type) をキーとします。
Gate::define::<User, Post>("view",   |u, _p| u.is_member);
Gate::define::<User, Post>("create", |u, _p| u.is_author);
Gate::define::<User, Post>("update", |u, _p| u.is_author);
Gate::define::<User, Post>("delete", |u, _p| u.is_admin);

let router: Router = Router::new()
    .resource("posts", PostsCtl)
    .authorize_resource::<User, Post>()
    .into();
```

アクション → 権限のマッピングは、Laravelを反映しています:

| アクション | 権限 |
|---|---|
| `index`, `show`     | `view`   |
| `create`, `store`   | `create` |
| `edit`, `update`    | `update` |
| `destroy`           | `delete` |

`PATCH` は `update` アクションを共有するため、`PUT` と全く同じようにゲートされます。権限が拒否されると、ハンドラが実行される前に `403` でショートサーキットし、未認証のリクエストはフェイルクローズします。リソースマーカーである `R` に必要なのは `Default` だけです - ゲートはLaravelがモデルクラスで判別するのと同じように、その*型*で判別します。権限そのものを定義する方法については、[認可の章](authorization.md)を参照してください。

## ルーターレベルのリダイレクトとビュー

`Router` にある3つのシュガーメソッドは、ハンドラ関数を必要としないルート宣言をカバーします:

```rust
use suprnova::Router;
use serde_json::json;

let router = Router::new()
    // 静的リダイレクト: GET /old-pricing → 302 /pricing
    .redirect("/old-pricing", "/pricing", 302)
    // 301 を返す兄弟
    .permanent_redirect("/legacy", "/new")
    // Inertia の静的ページ: GET /about は、定数プロップとともに About コンポーネントを描画します
    .view("/about", "About", json!({ "team_size": 4 }));
```

`Router::view` は、Laravelの `Route::view($uri, $view, $data)` に相当するSuprnovaの仕組みです。LaravelはBladeテンプレートを描画しますが、SuprnovaはフレームワークのテンプレートシステムがInertiaであり、Bladeではないため、Inertiaコンポーネントを描画します。

（ルート宣言ではなく）リダイレクトの*レスポンス* - `Redirect::route`、`Redirect::back`、`Redirect::intended`、署名付きリダイレクト - については、[URL 生成](urls.md)と[レスポンス](responses.md)を参照してください。

## 署名付きURL

HMACで署名されたルートはルーティングに隣接する話です（名前付きルートに対してURLを発行し、受信したリクエストで署名を検証します）。詳しくは[URL 生成](urls.md)で扱っていますが、手短に言うと:

```rust
use suprnova::url;

let reset = url::signed_route("password.reset", &[("user", "42")])?;
// /password/reset/42?signature=...

let expires_at = chrono::Utc::now().timestamp() + 3600;
let verify = url::temporary_signed_route("verify.email", &[("user", "42")], expires_at)?;
// /verify/email/42?expires=1748803600&signature=...
```

ハンドラの内部では、`url::has_valid_signature(&request)`（真偽値）または `url::signature_verdict(&request)`（`Valid`/`Expired`/`Invalid` の三択で、一般的な403の代わりに「新しいリンクをリクエストする」ページを描画できます）で検証します。

## 失敗しうる登録

ルートの登録は起動時に一度だけ実行されるため、重複や不正な形式のルートはプログラマーのエラーとして扱われます。そのため、素朴なヘルパー（`Router::get`、`post`、`put`、`delete`、`ws`、`RouteBuilder::name`、`GroupBuilder` → `Router` の `From` 変換）は**パニック**して、起動時にはっきりと失敗します。これは、ソースコードで宣言されるルートにとって正しいデフォルトです。

パターンや名前が失敗しうる出所 - 動的な設定、プラグインシステム、意図的に競合するルートを登録するテストなど - から来る場合は、`try_*` の兄弟を使ってください。これらはパニックする代わりに、`Result<_, FrameworkError>`（問題のあるメソッド、パス、または競合する名前を含む）を返します:

| パニックする版 | 失敗しうる兄弟 | 返す値 |
|---|---|---|
| `Router::get` / `post` / `put` / `patch` / `delete` / `head` / `options` | `try_get` / `try_post` / `try_put` / `try_patch` / `try_delete` / `try_head` / `try_options` | `Result<RouteBuilder, FrameworkError>` |
| `Router::ws`（および全ての `ws_*` バリアント） | `try_ws`（および全ての `try_ws_*`） | `Result<Router, FrameworkError>` |
| `RouteBuilder::name` | `try_name` | `Result<Router, FrameworkError>` |
| `GroupBuilder` → `Router`（`.into()` 経由） | `GroupBuilder::try_finalize` | `Result<Router, FrameworkError>` |
| `ResourceRoutes::register` | `try_register` | `Result<Router, FrameworkError>` |

```rust
use suprnova::{FrameworkError, Router};

// `path` は動的な設定から来ます。不正な形式や重複したパターンは
// 復旧可能であり、起動時のパニックではありません。
fn register_dynamic(router: Router, path: &str) -> Result<Router, FrameworkError> {
    Ok(router.try_get(path, health)?.into())
}
```

重複したグループのルートも同じように回復可能です - `From` は失敗しうる形にはできないため、`.into()` に対応する失敗しうる版は、固有メソッドである `try_finalize` です:

```rust
let router: Router = Router::new()
    .group("/api", |r| r.get("/users", list).post("/users", create))
    .try_finalize()?;
```

パニックするヘルパーは、便宜的な逃げ道として残されています。`try_*` の兄弟は、あくまで純粋な追加です。

## Suprnovaが異なる設計を選んだ理由

**2つのパスパラメータ構文が併存しています。** Laravelは `{param}` を、Expressは `:param` を使います。Suprnovaは両方を受け付け、パスが `matchit` に届く前に `:param` を `{param}` に正規化します。どちらのスタイルも、グループ、モデル結合、署名付きURLなど、他のあらゆるものと組み合わせられます。理由は優柔不断さではありません。あなたがどのような背景を持ってやってくるかを予測できず、ルーティングの構文は、人に学び直させるにはあまりに高頻度で摩擦を生む部分だからです。

**マクロとビルダー、2つの対等なAPIがあります。** Laravelは1つのDSL（`Route::get(...)`）を出荷します。Suprnovaは、宣言的な `routes! { ... }` マクロと、チェーン可能な `Router::new().get(...).name(...)` ビルダーの両方を出荷します。どちらも同一の登録を生成します。マクロはトップレベルのルート表で読みやすく、ビルダーはルーターを動的に組み立てるとき（プラグイン、生成されたルート、テスト）に読みやすくなります。呼び出し箇所に合うほうを選んでください - どちらの形も第一級であるため、規範となる唯一の答えはありません。

**サイレントな上書きではなく、起動時のパニックです。** 重複したルート名やパターンの衝突は、起動時にパニックします。Laravelの配列キーのレジストリは、後の登録が黙って勝つことを許します。これは、あなたのルートファイルが唯一の登録者である限りは問題ありませんが、プラグインや生成されたルートが登場すると安全ではなくなります。`try_*` の兄弟は、あえて失敗しうる形が必要なときの逃げ道です。

## 次のステップ

- [コントローラー](controllers.md) - `#[handler]`、フォームリクエスト、JSON/Inertiaを返す
- [ミドルウェア](middleware.md) - `Middleware` トレイト、順序、自分のものを組み立てる
- [URL 生成](urls.md) - 名前付きルートのURL、署名付きURL、リダイレクト、`RouteUrlError`
- [認可](authorization.md) - 結合されたモデルに対するゲートとポリシー
- [WebSocket](websockets.md) - `ws!`、`WebSocketHandler` トレイト、ルートごとの設定
