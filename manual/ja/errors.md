# エラーハンドリング

これは、Suprnovaのハンドラ、サービス、ミドルウェアで失敗しうるコードを書くための、日々のパターンガイドです。その基盤となるモデル - 変換の契約、パニック境界、5xxのサニタイズ規則、可観測性のフック - については[エラー モデル](error-model.md)を参照してください。この章が示すのは、実際に何を書けばよいか、という点です。

覚えておくべき形は、次のとおりです。

- ハンドラは `Response = Result<HttpResponse, HttpResponse>` を返します。
- `?` はハンドラのエラー型への、単一の直接的な `From<E>` 変換を実行します。Rustは `DbErr -> FrameworkError -> HttpResponse` を連鎖させません。`Response` ハンドラではSeaORMエラーを明示的に変換してください。すでに `Result<_, FrameworkError>` を返すコードは `.await?` を直接使えます。
- 3つのフリーヘルパー（`abort_with`、`abort_if`、`abort_unless`）を使えば、エラー型を名指しすることなく、ステータスコードでショートサーキットできます。

```rust
use sea_orm::EntityTrait;
use suprnova::{DB, FrameworkError, Request, Response, json_response};

pub async fn show(req: Request) -> Response {
    let id: i64 = req.param("id")?.parse()
        .map_err(|_| FrameworkError::param_parse("id", "i64"))?;
    let user = users::Entity::find_by_id(id)
        .one(&*DB::get()?)
        .await
        .map_err(FrameworkError::from)?
        .ok_or_else(|| FrameworkError::not_found("User"))?;
    json_response!({ "user": user })
}
```

この章の残りの部分は、エラーを生み出すものたちのカタログです - 何を組み立てるか、それがどのステータスを返すか、クライアントにはどんな形が見えるか、です。

## `?` は変換です

ハンドラ本体のあらゆる `?` は、単一の直接的な `From<E> for HttpResponse` 変換を実行します。フレームワークはハンドラ向けのエラー型に対する直接的な変換を提供しますが、Rustは複数の `From` 実装を連鎖させません。中間エラーが `HttpResponse` への直接的な変換を持たない場合は、明示的に変換してください。

```rust
use suprnova::{DB, FrameworkError, Request, Response, json_response};
use sea_orm::EntityTrait;

pub async fn show(req: Request) -> Response {
    let id: i64 = req.param("id")?.parse()
        .map_err(|_| FrameworkError::param_parse("id", "i64"))?;

    let user = users::Entity::find_by_id(id)
        .one(&*DB::get()?)
        .await
        .map_err(FrameworkError::from)?
        .ok_or_else(|| FrameworkError::not_found("User"))?;

    json_response!({ "user": user })
}
```

このスニペットでは4つの変換が起きています:

1. `req.param("id")?` は `ParamError` を直接 `HttpResponse`（400）へ変換します。
2. パースエラーは明示的に `FrameworkError::ParamError` へマップされ、`?` がそれを直接 `HttpResponse`（400）へ変換します。
3. SeaORMエラーは `DbErr` から `FrameworkError::Database` へ明示的にマップされ、`?` がその `FrameworkError` を直接 `HttpResponse`（500。ワイヤ上ではサニタイズされます）へ変換します。
4. `.ok_or_else(...)?` は `None` を `FrameworkError::ModelNotFound` に変え、それが `HttpResponse`（404）へ変換されます。

各 `?` は1つの直接的な変換を使います。`Response` ではなく `Result<_, FrameworkError>` を返すコードは、`DbErr` が直接 `FrameworkError` へ変換されるため、SeaORM呼び出しで `.await?` を使えます。

これらの変換はどれも、最後はフレームワークのJSONのエラーボディ - 対応するステータスでの `{ "message": …, "request_id": … }` - に行き着きます。APIクライアントにとってはそれが正解ですが、ページを必要とするInertiaの訪問にとっては不正解です。[エラーページ](frontend-inertia-responses.md#error-pages)を名指ししておけば、Inertiaアプリはこれらのエラーを本物のページとして描画し、APIクライアントはJSONをそのまま受け取り続けます。

## `AppError` - インラインのドメインエラー

専用の型を用意するほどではない、一回限りのエラーには `AppError` を使ってください。コンストラクタは、Laravelの `abort($status, $msg)` という形に対応しています。

| コンストラクタ | ステータス |
|---|---|
| `AppError::new(msg)` | 500 |
| `AppError::bad_request(msg)` | 400 |
| `AppError::unauthorized(msg)` | 401 |
| `AppError::forbidden(msg)` | 403 |
| `AppError::not_found(msg)` | 404 |
| `AppError::conflict(msg)` | 409 |
| `AppError::unprocessable(msg)` | 422 |
| `AppError::new(msg).status(code)` | 任意 |

`AppError` は `FrameworkError` への `From` を備えているため、`?` は何の儀式もなく機能します。

```rust
use suprnova::{AppError, Request, Response, json_response};

pub async fn transfer(req: Request) -> Response {
    let amount: i64 = req.param("amount")?.parse()
        .map_err(|_| AppError::bad_request("amount must be a number"))?;

    if amount <= 0 {
        return Err(AppError::unprocessable("amount must be positive").into());
    }

    if amount > balance() {
        return Err(AppError::forbidden("amount exceeds daily limit").into());
    }

    json_response!({ "transferred": amount })
}
```

この非対称性に注意してください。`AppError::unauthorized` は**401**（認証情報が欠けている）である一方、`FrameworkError::Unauthorized` は**403**（認証済みのユーザーがポリシーによって拒否された）です。両者は異なる意味を持つため、実際の失敗内容に合う方を選んでください。

## `FrameworkError` - 正規の列挙型

内部のエクストラクタ、コンテナ、ルートバインディング、バリデーション、データベース層、そしてストレージはすべて `FrameworkError` を生み出します。通常は、便利なコンストラクタを通じて1つを組み立て、`?` にルーティングを任せます。

```rust
use suprnova::FrameworkError;

FrameworkError::not_found("User");                    // 404
FrameworkError::bad_request("Bad input");             // 400
FrameworkError::param("user_id");                     // 400
FrameworkError::param_parse("user_id", "i64");        // 400
FrameworkError::validation("email", "required");      // 422
FrameworkError::domain("Conflict", 409);              // 409（任意のコード）
FrameworkError::internal("disk full");                // 500
FrameworkError::database("timeout");                  // 500
FrameworkError::service_not_found::<MyService>();     // 500
FrameworkError::model_not_found("Post");              // 404
```

レスポンスの形への影響を含めた、バリアントの全体集合は[エラー モデル](error-model.md)にあります。上記のコンストラクタは、あらゆる一般的なケースをカバーします - バリアントを直接使うのは、受け取ったエラーに対してマッチさせるときだけです。

### 自動変換

`FrameworkError` は、依存先が発するさまざまな方言を、すでに解します。次の2つの `?` は、どちらも自動的に変換されます。

```rust
use suprnova::{DB, FrameworkError};
use sea_orm::ActiveModelTrait;

pub async fn create_user(new_user: users::ActiveModel)
    -> Result<users::Model, FrameworkError>
{
    // DB::get は Result<_, FrameworkError> を返します。
    // .insert は Result<_, DbErr> を返しますが、From<DbErr> for FrameworkError があります。
    let user = new_user.insert(&*DB::get()?).await?;
    Ok(user)
}
```

フレームワークはまた、ストレージ操作のための `From<opendal::Error>` と、パスパラメータ抽出のための `From<ParamError>` も実装しています。

### コンテキストを添えて再送出する

ステータスコードを失うことなく、エラーがどこから来たのかを注釈したいときは、`.context()` を使ってください。

```rust
db.insert(user).await
    .map_err(FrameworkError::from)
    .map_err(|e| e.context("creating new user"))?;
```

メッセージは `"creating new user: <original>"` という形になります。構造化されたバリアント（`Validation`、`ValidationError`、`ModelNotFound`、`ParamParse`、`PrecognitionFailure`、`PrecognitionSuccess`、`Unauthorized`、`UnsupportedMediaType`、`AlreadyReported`、`RateLimited`、`External`）は自身のバリアントを保つため、レスポンスレンダラーは引き続き正しい形状を出力します（`External` ではラップされたsourceも存続します）。単なるメッセージを運ぶだけのバリアント（`Internal`、`Database`、`Domain`）は、プレフィックス付きのメッセージと元のステータスを保ったまま、`Domain` へと平坦化されます。

### 重複キーエラーを422に変える

`Unique` バリデーションルールは、書き込みの前に `SELECT COUNT(*)` を実行するため、助言的なものです - 2つの並行リクエストがどちらも通過し、その後どちらも挿入を試みる可能性があります。レースに負けたリクエストは、データベースの一意制約違反を受け取りますが、そのままでは500として漏れてしまいます。`from_unique_violation` は、これを助言的なルールが生成していたであろう422へと変換します。

```rust
use suprnova::FrameworkError;

let user = new_user.insert(db).await.map_err(|e| {
    FrameworkError::from_unique_violation(
        "email",
        "That email address is already registered.",
        e,
    )
})?;
```

背後の `DbErr` が一意制約違反でない場合は、500クラスの `Database` エラーとして変更されずに通過します。バックエンドの対応範囲は、SeaORMの `DbErr::sql_err` が認識するものすべてです - Postgres、MySQL/MariaDB、SQLiteは、いずれも重複キーのエラーをマッピングします。

### 外部エラーをラップする

他のすべてのバリアントは、ラップしたものを文字列化します。`from_external_with` は元のエラーを到達可能なまま保つため、ログが完全なチェーンをレンダリングでき、コードも実際に何が失敗したかを調べられます:

```rust
use suprnova::FrameworkError;

let row = sqlx_like_query()
    .await
    .map_err(|e| FrameworkError::from_external_with("verify query failed", e))?;
```

`from_external(e)` は、エラー自身の `Display` をメッセージにした同じものです。どちらもHTTP 500へマップされます。

元のものを調べるには、`source()` ではなく `external_source()` を使用してください:

```rust
if let Some(src) = err.external_source() {
    if let Some(db) = src.downcast_ref::<sea_orm::DbErr>() {
        // これを再試行する価値があるかを決定する
    }
}
```

`std::error::Error::source()` はラップされたエラーではなく共有 `Arc` ハンドルを返すため、それを通じたdowncastは `None` を返します。`external_source()` は先にハンドルをdereferenceします。

フレームワークは完全なチェーンを5xxログ行と、`APP_DEBUG=true` のときに追加する `debug_message` フィールドへレンダリングするため、ラップされたエラーのテキストが失われることはありません。

### レート制限ヒントを保持する

下流サービスがリクエストをスロットルして `Retry-After` ヒントを返すとき、失敗を `internal(...)` でラップすると、期間は説明文の中に埋もれてしまいます。`rate_limited` は、その期間を構造化したまま保持します:

```rust
use std::time::Duration;
use suprnova::FrameworkError;

let err = FrameworkError::rate_limited(
    Some(Duration::from_secs(30)),
    "push provider rejected the batch",
);

assert_eq!(err.retry_after(), Some(Duration::from_secs(30)));
assert_eq!(err.status_code(), 429);
```

キューのリトライポリシー、ジッタースケジューリング、HTTPの `Retry-After` レスポンスヘッダーはすべて、他のバリアントとヒントなしのスロットルでは `None` を返す `retry_after()` を通じてヒントを読み戻します。`.context(...)` はバリアントを保持するため、操作コンテキストの追加により期間が取り除かれることはありません。

## カスタムドメインエラー

エラーがどれだけ再利用可能である必要があるかに応じて、3つの階層があります。

### 型付きの場合の `#[domain_error]`

再利用可能なエラーの多くは、名前、固定のステータス、そして固定のメッセージテンプレートを求めます - 呼び出しごとのメッセージは必要ありません。`#[domain_error]` アトリビュートマクロは、`Display`、`std::error::Error`、`HttpError`、そして `FrameworkError` への `From` を一度に生成します。

```rust
use suprnova::domain_error;

#[domain_error(status = 404, message = "User not found")]
pub struct UserNotFound;

#[domain_error(status = 402, message = "Insufficient funds")]
pub struct InsufficientFunds {
    pub available: i64,
    pub requested: i64,
}
```

呼び出し箇所では、`?` と一緒に使ってください。

```rust
use crate::errors::user_not_found::UserNotFound;

pub async fn show(req: Request) -> Response {
    let id: i64 = req.param("id")?.parse()
        .map_err(|_| FrameworkError::param_parse("id", "i64"))?;

    let user = find_user(id).await
        .ok_or_else(|| FrameworkError::from(UserNotFound))?;

    json_response!({ "user": user })
}
```

このマクロは、不正な形のアトリビュートを明確なコンパイルエラーとして拒否します - オーバーフローしたステータスコード（`status = 70_000`）、間違ったリテラル型（`message = 42`）、未知のキーなどです - そのため、タイプミスのせいで、気づかないうちに間違ったステータスになってしまうことはありません。

#### CLIでスキャフォルドする

```bash
suprnova make:error UserNotFound
```

`src/errors/user_not_found.rs` を、デフォルトの `status = 500` と、推測された文頭大文字のメッセージとともに書き出し、`src/errors/mod.rs` を更新してそれを再エクスポートします。`status` と `message` はお好みで編集してください。

### 手作りの場合の `HttpError`

ドメインエラーが、メッセージの中に実行時の状態（例えば、失敗に関わるIDなど）を必要とする場合は、`HttpError` を直接実装してください。このトレイトには、妥当なデフォルトを持つ2つのメソッドがあります。

```rust
use suprnova::HttpError;

#[derive(Debug)]
pub struct InsufficientFunds {
    pub available: i64,
    pub requested: i64,
}

impl std::fmt::Display for InsufficientFunds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Insufficient funds: have {}, need {}",
            self.available, self.requested)
    }
}

impl std::error::Error for InsufficientFunds {}

impl HttpError for InsufficientFunds {
    fn status_code(&self) -> u16 { 402 }
    fn error_message(&self) -> String {
        format!("Need {} units, only {} available.",
            self.requested, self.available)
    }
}
```

手作りの `HttpError` を `?` へ橋渡しするには、`FrameworkError::from_http_error` を呼び出してください。包括的な `From<T: HttpError> for FrameworkError` は、既存の `From<AppError>` のimplと衝突してしまうため、この橋渡しは明示的なコンストラクタになっています。

```rust
account.withdraw(amount)
    .map_err(FrameworkError::from_http_error)?;
```

### 1つのモジュールの失敗のためのエラー列挙型

サービスが複数の関連する失敗を持つ場合は、それらを1つの列挙型にまとめ、その列挙型全体に対して1つの `From` を書いてください。

```rust
use suprnova::FrameworkError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum OrderError {
    #[error("Order {0} not found")]
    NotFound(i64),

    #[error("Insufficient stock for product {product_id}")]
    InsufficientStock { product_id: i64 },

    #[error("Payment failed: {0}")]
    PaymentFailed(String),

    #[error("Order already shipped")]
    AlreadyShipped,
}

impl From<OrderError> for FrameworkError {
    fn from(err: OrderError) -> Self {
        let status = match &err {
            OrderError::NotFound(_) => 404,
            OrderError::InsufficientStock { .. } => 422,
            OrderError::PaymentFailed(_) => 402,
            OrderError::AlreadyShipped => 409,
        };
        FrameworkError::Domain {
            message: err.to_string(),
            status_code: status,
        }
    }
}
```

`From` が存在すれば、その列挙型は他のあらゆるエラー型と同じように `?` を通り抜けます。

## `abort_with` / `abort_if` / `abort_unless`

3つのヘルパーが、ハンドラをあるステータスでショートサーキットさせます。これらは、Laravelの `abort` / `abort_if` / `abort_unless` を反映したものです。（フリー関数が `abort` ではなく `abort_with` としてエクスポートされているのは、`abort` をユーザー型のメソッド名として使えるように残しておくためです。）

```rust
use suprnova::{abort_if, abort_unless, abort_with, Request, Response, json_response};

pub async fn show(req: Request) -> Response {
    abort_unless(Auth::user().await?.is_some(), 401, "must be logged in")?;
    abort_if(req.param("id")? == "0", 404, "User not found")?;
    abort_with(503, "scheduled maintenance")?;

    json_response!({ "ok": true })
}
```

それぞれが `Result<(), FrameworkError>` を返すため、`?` が仕事をしてくれます。背後にあるエラーは `FrameworkError::Domain { message, status_code }` であり、他のあらゆるエラーと同じボディ形状でレンダリングされます。範囲外のステータスコードは、レスポンスレンダラーによって500に補正されるため、呼び出し箇所で不正な入力を防御する必要はありません。

## `ValidationErrors` - Laravel形式のエラーバッグ

バリデーションが失敗すると - `#[derive(Validate)]` の時点でも `after_validation` の本体でも - フレームワークは、Laravel/Inertiaのフロントエンドが期待するJSONの形を出力します。

```json
{
    "message": "The given data was invalid.",
    "errors": {
        "email": ["The email field must be a valid email address."],
        "password": ["The password field must be at least 8 characters."]
    },
    "request_id": "8f9e1a2b-c3d4-..."
}
```

たいていの場合、これを直接組み立てることはありません - `#[derive(Validate)]` が実行され、フレームワークが `validator::ValidationErrors` を変換してくれます。エラーを命令的に追加する必要がある場合（フィールド横断のルール、`Unique` を補完する非同期の一意性チェックなど）は、`ValidationErrors` を組み立てて返してください。

```rust
use suprnova::{FrameworkError, ValidationErrors};

pub async fn after_validation(payload: &Signup) -> Result<(), FrameworkError> {
    let mut errs = ValidationErrors::new();

    if payload.email.ends_with("@example.com") {
        errs.add("email", "example.com addresses are not allowed");
    }
    if payload.password == payload.email {
        errs.add("password", "password must not match email");
    }

    errs.into_result().map_err(FrameworkError::Validation)
}
```

`add_to_bag` は、バッグ名を `.` 区切りで先頭に付けることで、名前付きバッグ（Laravelの `withErrors($errors, 'profile')` という形式）の下にフィールドをスコープします。1つのレスポンスが、フラットな名前空間を共有できない複数のサブフォームからのエラーを運ぶときに便利です。

```rust
let mut errs = ValidationErrors::new();
errs.add_to_bag("profile", "bio", "must be under 280 characters");
errs.add_to_bag("billing", "card", "expired");
// errors マップ: { "profile.bio": [...], "billing.card": [...] }
```

`from_validator(ve)` は `validator::ValidationErrors` を変換します。`retain_fields(&keep)` は、指定されたエントリだけを含むコピーを返します（内部的にPrecognitionの `Precognition-Validate-Only` ヘッダーで使われています）。

## `ErrorOccurred` で可観測性をフックする

あらゆる5xxレスポンスは `ErrorOccurred` イベントを発火します - パニックから合成されたものも含みます。他のあらゆるイベントを購読するのと同じ方法で、これを購読してください。

```rust
use std::sync::Arc;
use suprnova::{ErrorOccurred, EventFacade, FrameworkError, Listener};

pub struct SentryReporter;

#[suprnova::async_trait]
impl Listener<ErrorOccurred> for SentryReporter {
    async fn handle(&self, evt: &ErrorOccurred) -> Result<(), FrameworkError> {
        sentry::capture_message(&evt.error_message, sentry::Level::Error);
        Ok(())
    }
}

// bootstrap.rs にて:
// `listen` は、リスナーの型から両方のジェネリクスを推論します。戻り値は
// `()` です（登録は失敗しえないため）、そのため `?` もResultも必要ありません。
EventFacade::listen::<ErrorOccurred, SentryReporter>(Arc::new(SentryReporter)).await;
```

このイベントは、生のエラーメッセージ（レスポンスボディは、それでもサニタイズされたままです - [エラー モデル](error-model.md)を参照）、ステータス、そして突き合わせ可能なリクエストIDを運びます。これは、例外ハンドラにおけるLaravelの `report()` コールバックに相当する、Suprnovaの仕組みです。

## よく書くことになるパターン

### パスパラメータを型付きの値としてパースする

```rust
let id: i64 = req.param("id")?.parse()
    .map_err(|_| FrameworkError::param_parse("id", "i64"))?;
```

`ParamError` はすでに400へと変換されます。`param_parse` はパース失敗版の相当物であり、同じ形状でレンダリングされます。

### IDで検索し、存在しなければ404にする

```rust
let user = users::Entity::find_by_id(id)
    .one(&*DB::get()?)
    .await
    .map_err(FrameworkError::from)?
    .ok_or_else(|| FrameworkError::not_found("User"))?;
```

`map_err(FrameworkError::from)?` は、SeaORMの `DbErr` を `From<DbErr> for FrameworkError` を経由し、続いて `From<FrameworkError> for HttpResponse` を経由して橋渡しします。Rustは2ホップにまたがって `From` のimplを自動連鎖させないため、明示的な `.map_err` が必要です。

あるいは、Eloquent層を使う場合です（すでにSeaORMをラップしており、直接 `Result<_, FrameworkError>` を返します）。

```rust
use suprnova::Model;

let user = User::find_or_fail(id).await?;
```

`find_or_fail` は、`find(id).ok_or(ModelNotFound)` をひとまとめにしたものです。

### アクションを認可する

```rust
let user = Auth::user().await?
    .ok_or_else(|| AppError::unauthorized("login required"))?;
abort_unless(post.owner_id == user.id() || user.is_admin(), 403,
    "you don't own this post")?;
```

`abort_unless` は `Result<(), FrameworkError>` を返します。`?` はそれを、ハンドラのエラー側へと畳み込みます。

### 型付きエラーを返すサービス

```rust
use suprnova::{App, FrameworkError, injectable};

#[injectable]
pub struct UserService;

impl UserService {
    pub async fn find_by_email(&self, email: &str)
        -> Result<users::Model, FrameworkError>
    {
        users::Entity::find()
            .filter(users::Column::Email.eq(email))
            .one(&*DB::get()?)
            .await?
            .ok_or_else(|| FrameworkError::not_found("User"))
    }
}

// 呼び出し箇所:
pub async fn show(req: Request) -> Response {
    let email = req.param("email")?;
    let user = App::resolve::<UserService>()?
        .find_by_email(email)
        .await?;
    json_response!({ "user": user })
}
```

`App::resolve::<UserService>()?` は `Result<Arc<UserService>, FrameworkError>` を返します。連鎖した `?` は、resolveの失敗と検索の失敗の両方を、1つのレスポンスへと畳み込みます。

## 早見表

| やりたいこと… | 使うもの |
|---|---|
| ステータス付きのインラインエラー | `AppError::bad_request("…")` とその仲間 |
| 型付きの再利用可能なエラー | `#[domain_error(status = …, message = "…")]` |
| 生成されたスキャフォルド | `suprnova make:error UserNotFound` |
| 実行時状態を持つ手作りのエラー | `impl HttpError for MyError` |
| 手作りのエラーを `?` へ橋渡しする | `FrameworkError::from_http_error(e)` |
| ステータスでショートサーキットする | `abort_with` / `abort_if` / `abort_unless` |
| モデルが見つからないときの404 | `FrameworkError::not_found("User")` / `Model::find_or_fail` |
| パスパラメータのパース失敗 | `FrameworkError::param_parse("id", "i64")` |
| フィールドレベルのバリデーションエラー | `FrameworkError::validation("email", "…")` |
| 複数フィールドのエラーバッグ | `ValidationErrors::new().add(…)` + `Validation(errs)` |
| 重複キー違反 → 422 | `FrameworkError::from_unique_violation(field, msg, e)` |
| 既存のエラーに注釈を付ける | `err.context("creating user")` |
| あらゆる5xxを観測する | `ErrorOccurred` を購読する |
| エラーをInertiaのページとして描画する | `InertiaConfig::error_page("Error")` |

## 次のステップ

- [エラー モデル](error-model.md) - バリアント、変換の契約、5xxのサニタイズ、パニック境界
- [バリデーション](validation.md) - `#[derive(Validate)]`、フォームリクエスト、そして `after_validation`
- [レスポンス](responses.md) - `HttpResponse` のビルダー、ステータス、ヘッダー
- [イベント](events.md) - `ErrorOccurred` やその他の組み込みイベントを購読する
- [リクエスト ライフサイクル](lifecycle.md) - リクエストフローのどこでエラー変換が実行されるか
