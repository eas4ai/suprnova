# コントローラー

Suprnovaのコントローラーは、単なる非同期関数です。リクエストから必要なもの - 型付きのパスパラメータ、読み込み済みのモデル、バリデーション済みのフォーム - を受け取り、`Response` を返します。コントローラーの基底クラスはありません。サービスロケーターの配線ファイルもありません。単位となるのは関数であり、`#[handler]` アトリビュートがそれをルーティングマクロへと結びつけます。

```rust
use suprnova::{handler, json_response, Response};
use crate::models::user;

// GET /users/{user}
#[handler]
pub async fn show(user: user::Model) -> Response {
    json_response!({
        "id": user.id,
        "name": user.name,
        "email": user.email,
    })
}
```

このハンドラのシグネチャは、3つのことを同時に行っています: ルートパラメータ（`user`）を宣言し、データベースから行を取り出し、その行が存在しなければ404を返す、ということです。そのどれ一つとして手で書かれてはいません。`#[handler]` が引数の型を読み取り、抽出処理を生成します。

## コントローラーを生成する

```bash
suprnova make:controller User
```

これは、`invoke` のスタブを1つだけ持つ `src/controllers/user.rs` を書き出し、`src/controllers/mod.rs` に `pub mod user;` を追加します。このスタブは、最小限で動作するハンドラです:

```rust
//! User controller

use suprnova::{handler, json_response, Request, Response};

#[handler]
pub async fn invoke(_req: Request) -> Response {
    json_response!({
        "controller": "User"
    })
}
```

このファイルには、好きなだけ関数を追加してください - Suprnovaはコントローラーの「クラス」ではなく、関数だけを追跡します。多くのアプリはリソースごとに分割しますが（`controllers::user::{index, show, store, update, destroy}`）、フレームワークの中にそれを強制するものは何もありません。

名前は、ファイル名のために `snake_case` へ変換されます: `OrderItem` は `order_item.rs` になります。

## `#[handler]` アトリビュート

このマクロは、各パラメータの型を分類し、それに対応するエクストラクターを生成します。カテゴリは4つあります:

| パラメータの型 | 抽出の方法 | 失敗したときの挙動 |
|---|---|---|
| `Request` | リクエストをそのまま素通りさせます | - |
| `i32`, `i64`, `u32`, `u64`, `usize`, `String` | `FromParam` - 同じ名前のルートパラメータをパースします | パース失敗で400、欠落で400 |
| `T: AutoRouteBinding`（あらゆるEloquentの `Model`） | パラメータをモデルの主キーとしてパースし、その行を読み込みます | パース失敗で400、見つからなければ404 |
| それ以外のすべて（`T: FromRequest`） | `T::from_request(req)` を呼び出します - 通常は `#[derive(FormRequest)]` のバリデータです | `from_request` が返すもの。バリデーションエラーであれば422 |

マクロは宣言された順に抽出を実行するため、関数の本体は完全に型付けされた値を目にします。いずれかの抽出が失敗した場合、エラーは `?` によってショートサーキットし、ハンドラの本体が実行されることはありません。

### パスパラメータ

```rust
// ルート: get!("/users/{id}", controllers::user::show)
#[handler]
pub async fn show(id: i64) -> Response {
    json_response!({ "user_id": id })
}

// ルート: get!("/posts/{post_id}/comments/{comment_id}", show_comment)
#[handler]
pub async fn show_comment(post_id: i64, comment_id: i64) -> Response {
    json_response!({
        "post_id": post_id,
        "comment_id": comment_id,
    })
}
```

引数の名前は、ルートのプレースホルダーと一致していなければなりません: `{id}` には `id: …` が必要です。引数の型は `FromParam` を通じてパースされます。不正な入力（`id: i64` に対する `/users/abc`）は、パラメータ名と変換先の型を示すメッセージとともに400を返します。

### ルートモデル結合

`Eloquent` のモデルは、`AutoRouteBinding` を自動的に実装します。モデルを引数として宣言すれば、フレームワークがそれを読み込みます:

```rust
use suprnova::{handler, json_response, Response};
use crate::models::user;

// ルート: get!("/users/{user}", controllers::user::show)
#[handler]
pub async fn show(user: user::Model) -> Response {
    json_response!({
        "id": user.id,
        "name": user.name,
        "email": user.email,
    })
}
```

ルートのプレースホルダー名（`{user}`）と引数名（`user`）は一致していなければなりません。フレームワークはパラメータの文字列をモデルの主キーの型としてパースし、`Entity::find_by_pk` を呼び出して、その行が欠けていれば404を返します。`#[suprnova::model]` を付けた構造体は、すべて自動的に結合されます。`#[suprnova::model]` を使わずに手書きしたSeaORMのエンティティのために、`route_binding!` マクロも引き続き利用できます - [マクロ](macros.md#route_binding)を参照してください。

### フォームリクエスト

`FromRequest` を実装するものであれば、何でも同じように差し込めます。よくあるのは、リクエストボディをバリデーションし、失敗したときにフィールドをキーとするエラーとともに422を返す、`#[derive(FormRequest)]` 付きの構造体です:

```rust
use suprnova::{attrs, handler, json_response, Response};
use crate::models::user;
use crate::requests::UpdateUserRequest;

// ルート: put!("/users/{user}", controllers::user::update)
#[handler]
pub async fn update(user: user::Model, form: UpdateUserRequest) -> Response {
    let id = user.id;
    user.update(attrs! { name: form.name, email: form.email }).await?;
    json_response!({ "updated": id })
}
```

バリデータのderiveと、バリデーションパイプライン全体については、[フォームリクエスト](requests.md)を参照してください。

### 生の `Request` が欲しいとき

自分の手で取り出したい場合 - あるいはヘッダー、クッキー、クエリ文字列が必要な場合 - は、`Request` を直接受け取ってください:

```rust
use suprnova::{handler, json_response, Request, Response};

#[handler]
pub async fn show(req: Request) -> Response {
    let id = req.param("id")?;             // ルートパラメータ、見つからなければ400
    let ua = req.header("User-Agent");      // Option<&str>
    let page: u32 = req.query_param("page") // Option<String>
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);

    json_response!({ "id": id, "ua": ua, "page": page })
}
```

混ぜて使うこともできます: `pub async fn nested(category_id: i64, product: product::Model, req: Request)` は有効なシグネチャです。マクロは、それぞれの引数をそれぞれのルールに従って抽出します。

## `Response` の契約

`Response` は `Result<HttpResponse, HttpResponse>` のエイリアスです。どちらの分岐も同じペイロードの型を運ぶため、`?` がどこでも機能します。ミドルウェアチェーンは、境界にあるこの1行で結果を収束させます:

```rust
result.unwrap_or_else(|e| e)
```

これは、あらゆる `?` の伝播地点が依拠しているのと同じ契約です。エラーは、チェーンに届く前に `From<FrameworkError> for HttpResponse` を通じて変換されます - 全体像については[エラー モデル](error-model.md)を参照してください。

ハンドラの本体は上から下へ読み下せる形になっており、抜けるときには `?` を使います:

```rust
use suprnova::{handler, json_response, Response};
use crate::models::user;

#[handler]
pub async fn show(id: i64) -> Response {
    let user = user::Model::find_or_fail(id).await?;
    let invoices = user.invoices().get().await?;
    json_response!({ "user": user, "invoices": invoices })
}
```

`find_or_fail` が `Err` を返した場合、関数は404で終了します。`invoices().get()` がエラーになれば、返ってくるのは500です。`match` 文も、例外ハンドラもありません。

## レスポンスを作る

3つのマクロと1つのビルダーが、よくあるケースをカバーします:

```rust
use suprnova::{handler, json_response, text_response, HttpResponse, Response, ResponseExt};

#[handler]
pub async fn json_handler() -> Response {
    json_response!({
        "users": [
            {"id": 1, "name": "John"},
            {"id": 2, "name": "Jane"},
        ]
    })
}

#[handler]
pub async fn health() -> Response {
    text_response!("OK")
}

#[handler]
pub async fn store() -> Response {
    // ResponseExt により、ステータスやヘッダーを組み込みでチェーンできます。
    json_response!({ "id": 1, "created": true }).status(201)
}

#[handler]
pub async fn page() -> Response {
    Ok(HttpResponse::html("<h1>Hello</h1>"))
}
```

`json_response!`、`text_response!`、`HttpResponse::*` は、いずれも同じ `Response` 型を生み出します。`ResponseExt` トレイトが `.status(...)`、`.header(...)`、`.cookie(...)`、`.with_headers(...)` を追加するため、マクロの結果に設定をチェーンできます。

それ以外のすべて - ファイルのダウンロード、ストリーミングするボディ、Inertiaのレスポンス、リダイレクト - については、[レスポンス](responses.md)を参照してください。

## リダイレクト

`redirect!("route.name")` は、そのルートが存在することをコンパイル時に検証し、設定をチェーンできるビルダーを返します:

```rust
use suprnova::{handler, redirect, Response};

#[handler]
pub async fn store() -> Response {
    // ユーザーを作成…
    redirect!("users.index").into()
}

#[handler]
pub async fn update(id: i64) -> Response {
    redirect!("users.show")
        .with("id", id.to_string())
        .into()
}

#[handler]
pub async fn search() -> Response {
    redirect!("users.index")
        .query("page", "1")
        .query("sort", "name")
        .into()
}
```

`.with(key, value)` はルートのプレースホルダーを埋めます。`.query(key, value)` はクエリ文字列のパラメータを追加します。`.flash(key, value)` は、次のリクエストのためにセッションのフラッシュバッグへ書き込みます。`.into()` は、ビルダーを `Response` へ変換します。

名前付きルートが存在しない場合、マクロは利用可能なルート名の一覧とともにコンパイルを失敗させます - タイプミスは、ステージングに届く前に表に出ます。

## コンテナから注入されるサービス

コンテナからサービスを解決するには、`App::resolve`（具象型）または `App::resolve_make`（トレイトオブジェクト）を使います。どちらも `Result<_, FrameworkError>` を返すため、`?` と組み合わせられます:

```rust
use suprnova::{handler, json_response, App, Response};
use crate::services::UserService;

#[handler]
pub async fn index() -> Response {
    let user_service = App::resolve::<UserService>()?;
    let users = user_service.list_all().await?;
    json_response!({ "users": users })
}
```

`#[injectable]` でアクションを束縛している場合、コントローラーからそれを呼び出すのがこの形です。アクションの形については[アクション](actions.md)を、コンテナの表面全体 - 束縛、ファクトリー、task-local / thread-local / グローバルという探索のカスケード - については[サービス コンテナ](container.md)を参照してください。

## RESTfulなコントローラーの実例

```rust
// src/controllers/user.rs
use suprnova::{attrs, handler, json_response, redirect, Response, ResponseExt};
use crate::models::user;
use crate::requests::{StoreUserRequest, UpdateUserRequest};

// GET /users
#[handler]
pub async fn index() -> Response {
    let users = user::Model::all().await?;
    json_response!({ "users": users })
}

// GET /users/{user}
#[handler]
pub async fn show(user: user::Model) -> Response {
    json_response!({ "user": user })
}

// POST /users
#[handler]
pub async fn store(form: StoreUserRequest) -> Response {
    let user = user::Model::create(attrs! {
        name: form.name,
        email: form.email,
    }).await?;
    json_response!({ "user": user }).status(201)
}

// PUT /users/{user}
#[handler]
pub async fn update(user: user::Model, form: UpdateUserRequest) -> Response {
    let id = user.id;
    user.update(attrs! {
        name: form.name,
        email: form.email,
    }).await?;
    json_response!({ "updated": id })
}

// DELETE /users/{user}
#[handler]
pub async fn destroy(user: user::Model) -> Response {
    user.delete().await?;
    redirect!("users.index").into()
}
```

これらを `routes!` マクロで登録します:

```rust
// src/routes.rs
use suprnova::{delete, get, post, put, routes};
use crate::controllers;

routes! {
    get!("/users",           controllers::user::index   ).name("users.index"),
    get!("/users/{user}",    controllers::user::show    ).name("users.show"),
    post!("/users",          controllers::user::store   ).name("users.store"),
    put!("/users/{user}",    controllers::user::update  ).name("users.update"),
    delete!("/users/{user}", controllers::user::destroy ).name("users.destroy"),
}
```

ルートのプレースホルダー `{user}` は引数名 `user: user::Model` と対応しており、これによってフレームワークは、どのパスセグメントがモデルを読み込むのかを知ります。

## `Request` API

`Request` を直接受け取るときに、最もよく手を伸ばすことになるメソッドです:

| メソッド | 戻り値 | 備考 |
|---|---|---|
| `method()` | `&hyper::Method` | HTTPメソッド |
| `path()` | `&str` | URLのパス |
| `param(name)` | `Result<&str, ParamError>` | ルートパラメータ。抜けるには `?` を使います |
| `params()` | `&HashMap<String, String>` | すべてのルートパラメータ |
| `query()` | `Option<&str>` | 生のクエリ文字列 |
| `query_param(key)` | `Option<String>` | クエリ文字列の単一の値 |
| `query_params()` | `HashMap<String, String>` | すべてのクエリパラメータ |
| `query_into::<T>()` | `Result<T, FrameworkError>` | 型付きのデシリアライズ |
| `header(name)` | `Option<&str>` | 単一のヘッダー |
| `headers()` | `&hyper::HeaderMap` | ヘッダーマップ全体 |
| `has_header(name)` | `bool` | 存在チェック |
| `bearer_token()` | `Option<String>` | パース済みの `Authorization: Bearer …` |
| `cookie(name)` | `Option<String>` | 単一のクッキーの値 |
| `cookies()` | `HashMap<String, String>` | すべてのクッキー |
| `ip()` | `Option<String>` | ピアのIP。X-Forwarded-Forを考慮します |
| `secure()` | `bool` | HTTPSの検出（プロキシ経由も含む） |
| `is_method(m)` | `bool` | 大文字小文字を区別しません |
| `is_inertia()` | `bool` | InertiaのXHRヘッダー |
| `ajax()` | `bool` | `X-Requested-With: XMLHttpRequest` |
| `expects_json()` / `wants_json()` | `bool` | Acceptヘッダーの検査 |
| `route_name()` | `Option<String>` | マッチしたルートの `.name(...)` |
| `json::<T>()` | `Result<T, FrameworkError>` | ボディをJSONとしてパースします（消費します） |
| `form::<T>()` | `Result<T, FrameworkError>` | form-urlencodedとしてパースします |
| `input::<T>()` | `Result<T, FrameworkError>` | Content-Typeで振り分けるパース |

これはLaravelの形をした表面です - ここにあるメソッドはすべて、Laravelの `Request` クラスのメソッドを反映しています。

## ファイル構成

慣例は次のとおりです:

```
src/
├── controllers/
│   ├── mod.rs          # pub mod home; pub mod user; ...
│   ├── home.rs
│   ├── user.rs
│   └── api/
│       ├── mod.rs
│       └── user.rs
├── routes.rs           # routes! { ... }
└── main.rs
```

フレームワークの中に、この構成を強制するものは何もありません - コントローラーは `routes.rs` から到達できる場所であれば、どこにでも置けます。この慣例が存在するのは、スキャフォルドが出力するのがこの形だからであり、またルートとコントローラーが自然な対をなすからです。

## Suprnovaが異なる設計を選んだ理由

Laravelのコントローラーは、`Illuminate\Routing\Controller` を継承したクラスです。メソッドは、コンテナがリクエストごとに解決するインスタンスの上で呼び出され、コンストラクタ注入はそこで行われます。このパターンはPHPでは問題ありません - レスポンスの後にプロセス全体が破棄されるのであれば、リクエストごとの `new` は安いのです。

Rustで同じパターンを採ると、(a) リクエストごとにコントローラーの構造体を確保して、必要のない `Arc` のクローン分のコストを払うか、(b) 割に合わない基底クラスの階層を通じて依存性注入を作り直すか、そのどちらかになってしまいます。

Suprnovaは、より単純なモデルを選びます: コントローラーはクラスに属さない非同期関数であり、「依存」はコンテナからの解決（`App::resolve::<Service>()?`）か、抽出によって型付けされた引数（`form: UpdateUserRequest`）のどちらかです。コンストラクタ注入は、それが本来属する場所である[アクション](actions.md)の `#[injectable]` 境界で行われます。ハンドラは、リクエストからレスポンスへの純粋な関数のままです。そのおかげで、単独でのテストがごく簡単になります: `Request` を組み立て、関数を呼び、結果をアサートするだけです。

## 次のステップ

- [ルーティング](routing.md) - `routes!`、`get!`、`post!`、`.name()` が何に展開されるか
- [フォームリクエスト](requests.md) - `#[derive(FormRequest)]` による型付きバリデーション
- [レスポンス](responses.md) - JSON、HTML、ファイル、ストリーム、Inertiaのページ、リダイレクト
- [サービス コンテナ](container.md) - `App::resolve` が実際に行っていること
- [アクション](actions.md) - コントローラーの外側でビジネスロジックが暮らす場所
- [エラー モデル](error-model.md) - `?` が `FrameworkError` をレスポンスへ変える仕組み
