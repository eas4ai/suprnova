# エラー モデル

この章では、Suprnovaのエラー処理を支える仕組み - 型、変換の契約、そしてフレームワークが標準で提供する安全性の保証について解説します。日々のハンドラでの実践的なパターン（`?`、エラーを返すこと、独自のドメインエラーを組み立てること）については[エラーハンドリング](errors.md)を参照してください。この章が説明するのは、それらのパターンが*なぜ*そのように機能するのか、という点です。

このページから1つだけ覚えておくとすれば、次の点です。**Suprnovaにおけるエラーは、例外ではなく値です**。あらゆるエラーは最終的に、単一かつ網羅的な変換を通じて `HttpResponse` になります。グローバルな例外ハンドラが存在しないのは、そもそもグローバルな例外という概念が存在しないからです。

## 全体像

Suprnovaのエラーモデルは、5つの可動部分から構成されています。

| 型 | 役割 |
|---|---|
| `Response = Result<HttpResponse, HttpResponse>` | すべてのハンドラが満たす契約です - どちらの分岐も、すでにレスポンスになっています |
| `FrameworkError` | フレームワークの正規のエラー列挙型です。内部のあらゆるエラー経路が、これを生成します |
| `AppError` | 専用の型を用意せず、インラインで使うための即席のドメインエラーです |
| `HttpError`（トレイト） | 独自の型付きドメインエラーが実装することで、ステータスとメッセージを得られるようにするものです |
| `ValidationErrors` | フィールドごとの失敗を表す、Laravel/Inertia形式のエラーバッグです |

`FrameworkError` とフレームワークの具体的なエラー型は `From` 実装を使います。手書きの `HttpError` は `?` の前に `FrameworkError::from_http_error` でマップしなければなりません。包括的な `From<T: HttpError>` 実装は存在しません。ミドルウェアチェーンはリクエスト境界でエラーを変換し、パニックハンドラは巻き戻りを変換します。通常のエラーはその後、共通のボディレンダラーと5xxのサニタイズ規則を共有します。

## `Response` は `Result<HttpResponse, HttpResponse>` です

すべてのハンドラは、これを返します。

```rust
pub type Response = Result<HttpResponse, HttpResponse>;
```

どちらの分岐も同じペイロード型を運びます。これこそが肝心な点です。ミドルウェアチェーンがハンドラの実行を終えると、次の1行で結果を1つにまとめます。

```rust
result.unwrap_or_else(|e| e)
```

フレームワーク側は、ハンドラが「成功」したのか「失敗」したのかを知る必要がありません - どちらの分岐も、すでにレンダリング済みのHTTPレスポンスだからです。この区別が存在するのは、ひとえに `?` がその役割を果たせるようにするためです。

```rust
use suprnova::{Request, Response, json_response};

pub async fn show(req: Request) -> Response {
    // `?` は Err でショートサーキットします。以下の各変換は From 実装を経由して
    // HttpResponse を生み出し、チェーンが両方の分岐を1つに畳み込みます。
    let id: i64 = req.param("id")?.parse().map_err(|_| {
        suprnova::FrameworkError::param_parse("id", "i64")
    })?;
    let user = User::find_or_fail(id).await?;  // 見つからなければ404
    Ok(json_response!({ "user": user }))
}
```

この単一の契約 - あらゆるエラー経路が `From` を通じて `HttpResponse` を生成すること - こそが、このモデルの核心です。この章の残りの部分はすべて、さまざまな `From` の実装が実際に何を行っているかについて説明しています。

### Suprnovaが異なる設計を選んだ理由

Laravelは例外をスローし、`app/Exceptions/Handler.php` に登録されたグローバルな `Handler` クラスを通じてそれをルーティングします。フレームワークがすべてを捕捉し、ハンドラに「何をレンダリングすべきか」を問い合わせ、レスポンスを送出します。PHPの巻き戻し式の例外モデルは、これを自然な形にしています。

Rustのユーザーコードには、巻き戻し式の例外がありません。Suprnovaにおけるそれに相当する仕組みが、`From<FrameworkError> for HttpResponse` の実装と `ErrorOccurred` イベントです。変換がレンダラーであり、イベントは可観測性（Sentry、PagerDuty、構造化ログの転送先など）を組み込むためのフックです。ハンドラクラスを登録するのではなく、変換は1つの関数であり、`ErrorOccurred` を購読することが拡張ポイントになります。表面は同じでも、仕組みは異なります。

## `FrameworkError` - 正規の列挙型

フレームワーク内部のあらゆるエラー経路 - エクストラクタ、ルートバインディング、コンテナ、バリデーション、データベース層、ストレージ - は、`FrameworkError` を生成します。これは16個のバリアントを持つ列挙型で、それぞれにHTTPステータスがタグ付けされています。

```rust
pub enum FrameworkError {
    ServiceNotFound { type_name: &'static str },        // 500
    ParamError { param_name: String },                   // 400
    ValidationError { field: String, message: String },  // 422
    Database(String),                                    // 500
    Internal { message: String },                        // 500
    Domain { message: String, status_code: u16 },        // *
    Validation(ValidationErrors),                        // 422
    Unauthorized,                                        // 403
    ModelNotFound { model_name: String },                // 404
    ParamParse { param: String, expected_type: &'static str }, // 400
    UnsupportedMediaType,                                // 415
    PrecognitionSuccess,                                 // 204
    PrecognitionFailure(ValidationErrors),               // 422
    AlreadyReported,                                     // CLI 専用
    RateLimited { retry_after: Option<Duration>, message: String }, // 429
    External { message: String, source: Arc<dyn Error + Send + Sync> }, // 500
}
```

バリアントに対してマッチさせることは、めったにありません。便利なコンストラクタを通じて1つを組み立て、残りは `?` に任せます。

```rust
use suprnova::FrameworkError;

// これらはいずれも、正しいステータスを持つ FrameworkError を生み出します:
FrameworkError::not_found("User");                    // → ModelNotFound, 404
FrameworkError::bad_request("Bad input");             // → Domain, 400
FrameworkError::param("user_id");                     // → ParamError, 400
FrameworkError::param_parse("user_id", "i64");        // → ParamParse, 400
FrameworkError::validation("email", "required");      // → ValidationError, 422
FrameworkError::domain("Conflict", 409);              // → Domain, 409
FrameworkError::internal("disk full");                // → Internal, 500
FrameworkError::database("timeout");                  // → Database, 500
```

`FrameworkError` には `unauthorized()` や `forbidden()` といったコンストラクタはありません - `Unauthorized` は固定のバリアントであり、Laravelの「This action is unauthorized.」というメッセージを403として運びます。401のケースは、（次のセクションで扱う）`AppError::unauthorized` を経由します。なお、このバリアントは `Unauthorized` という名前を持ちますが、ステータスが403であるのは、HTTP認証ではなくLaravelの認可拒否をモデル化しているためです。

### 自動変換

`FrameworkError` は `From<sea_orm::DbErr>` と `From<opendal::Error>` を実装しているため、データベースとストレージのエラーは、ラップすることなく `?` を通過します。

```rust
use suprnova::{DB, FrameworkError};
use sea_orm::ActiveModelTrait;

pub async fn create_user(new_user: ActiveModel) -> Result<Model, FrameworkError> {
    // ここの2つの `?` は、いずれも自動的に FrameworkError へ変換されます:
    // - DB::get は Result<_, FrameworkError> を返します
    // - insert は Result<_, DbErr> を返し、これには From<DbErr> for FrameworkError があります
    let user = new_user.insert(&*DB::get()?).await?;
    Ok(user)
}
```

あなたのコードが `Result<_, FrameworkError>` を返すのであれば、依存先が生成するあらゆる一般的なエラーは、すでに正しい言語で語られていることになります。コントローラーの `?` が行う仕事は、ある1つのエラー型を別のエラー型へと変換すること以上のものではありません。

### コンテキストの付与

操作のコンテキストを添えてエラーを再送出したい場合は、`.context()` を使います。

```rust
db.insert(user).await
    .map_err(FrameworkError::from)
    .map_err(|e| e.context("creating new user"))?;
```

メッセージは `"creating new user: <original>"` という形になります。バリアントは、それが意味を持つ箇所では保持されます - `Validation`、`ValidationError`、`PrecognitionFailure`、`PrecognitionSuccess`、`Unauthorized`、`ModelNotFound`、`ParamParse`、`UnsupportedMediaType`、`AlreadyReported`、`RateLimited`、`External` は自身の構造を保つため、レスポンスレンダラーは引き続き正しい形状を出力します（`External` ではラップしたsourceも存続します）。単なるメッセージを運ぶだけのバリアント（`Internal`、`Database`、`Domain`）は、プレフィックス付きのメッセージを持つ `Domain` へと平坦化されます。

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

`RateLimited` は、下流の `Retry-After` ヒントがメッセージテキストへ潰れず、`Duration` としてエラーシステムを通過できるようにするために存在します:

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

`retry_after()` は、他のすべてのバリアント、およびヒントなしで到着したスロットルに対して `None` を返します。このバリアントはHTTP 429としてレンダリングされ、`.context(...)` は `Domain` へ平坦化せず保持するため、操作コンテキストの追加により期間が取り除かれることはありません。

## `AppError` - 即席のドメインエラー

専用の型を定義したくない一回限りのエラーには、`AppError` を使います。これは `HttpError` を実装しており、`FrameworkError` への `From` も備えているため、`?` がそのまま機能します。

```rust
use suprnova::{AppError, Request, Response, json_response};

pub async fn transfer(req: Request) -> Response {
    let amount: i64 = req.param("amount")?.parse()
        .map_err(|_| AppError::bad_request("amount must be a number"))?;

    if amount <= 0 {
        return Err(AppError::unprocessable("amount must be positive").into());
    }

    if amount > 1_000_000 {
        return Err(AppError::forbidden("amount exceeds daily limit").into());
    }

    Ok(json_response!({ "transferred": amount }))
}
```

これらのコンストラクタは、Laravelの `abort($status, $msg)` という形にきれいに対応しています。

| `AppError::*` | ステータス |
|---|---|
| `bad_request(msg)` | 400 |
| `unauthorized(msg)` | 401 |
| `forbidden(msg)` | 403 |
| `not_found(msg)` | 404 |
| `conflict(msg)` | 409 |
| `unprocessable(msg)` | 422 |
| `new(msg)` | 500 |
| `.status(code)` | 任意 |

`AppError::unauthorized` は**401**（HTTP認証が欠けている状態）であるのに対し、`FrameworkError::Unauthorized` は**403**（認可の拒否。Laravelのポリシー拒否に対応）である点に注意してください。両者は異なる意味を持つため、実際の失敗内容に合う方を選んでください。

## `HttpError` - カスタム型付きエラー

同じドメインエラーが多くの場所に現れる場合は、それを型としてモデル化してください。`HttpError` を実装すれば、変換は自分の手の中にあります。

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

`HttpError` には2つのメソッドがあり、どちらにもデフォルト実装があります。

```rust
pub trait HttpError: std::error::Error + Send + Sync + 'static {
    fn status_code(&self) -> u16 { 500 }
    fn error_message(&self) -> String { self.to_string() }
}
```

### `?` への橋渡し

素朴に `impl<T: HttpError> From<T> for FrameworkError` と書いてしまうと、既存の `From<AppError>` の実装と衝突します（`AppError` 自身が `HttpError` を実装しているためです）。Suprnovaは、この孤児ルールの問題を、明示的な橋渡し用コンストラクタで解決します。`HttpError` を実装するカスタム型を扱う場合は、常に `FrameworkError::from_http_error` を明示的に呼び出してください。

```rust
use suprnova::{FrameworkError, HttpError};

pub async fn debit(account: &mut Account, amount: i64) -> Result<(), FrameworkError> {
    account.withdraw(amount)
        .map_err(FrameworkError::from_http_error)?;
    Ok(())
}
```

ステータスコードとメッセージは `HttpError::status_code` と `HttpError::error_message` から取り出され、`FrameworkError::Domain` バリアントに格納されます。その後、レスポンスレンダラーは通常の `Domain` の経路をたどります。

### `#[domain_error]` - ボイラープレート不要の型のために

`Display`、`Error`、`HttpError` の実装を手で書くことなく、型付きエラーのパターンを使いたい場合は、`#[domain_error]` アトリビュートマクロを使います。

```rust
use suprnova::domain_error;

#[domain_error(status = 404, message = "User not found")]
pub struct UserNotFoundError;

#[domain_error(status = 402, message = "Insufficient funds")]
pub struct InsufficientFundsError {
    pub available: i64,
    pub requested: i64,
}
```

`#[domain_error]` は、`From<YourError> for FrameworkError` を*含む*完全な実装一式を生成するため、橋渡し用の呼び出しなしに `?` がそのまま機能します。

```rust
pub async fn show(req: Request) -> Response {
    let id: i64 = req.param("id")?.parse()
        .map_err(|_| FrameworkError::param_parse("id", "i64"))?;
    let user = User::find(id).await?
        .ok_or_else(|| FrameworkError::from(UserNotFoundError))?;
    Ok(json_response!({ "user": user }))
}
```

カスタムエラーの作り方には3段階あります - インラインで使う `AppError`、マクロで型付けする `#[domain_error]`、そして完全に制御できる手作りの `HttpError` です。これにより、求める作り込みの度合いに応じて、そのつど適切な道具を選べます。

## `ValidationErrors` - Laravel形式のエラーバッグ

リクエストがバリデーションに失敗すると、SuprnovaはLaravelとInertiaのフロントエンドが期待するのと同じJSON形状を出力します。

```json
{
    "message": "The given data was invalid.",
    "errors": {
        "email": ["The email field must be a valid email address."],
        "password": ["The password must be at least 8 characters."]
    },
    "request_id": "8f9e1a2b-c3d4-..."
}
```

通常、これを手で組み立てることはありません - フォームリクエストに付けた `#[derive(Validate)]` と、その背後にある `validator` クレートが `validator::ValidationErrors` を生成し、それをSuprnovaが `ValidationErrors::from_validator` を介して変換します。ですが、必要なときのためにこの型は公開されています。

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

`add_to_bag` は、バッグ名を `.` 区切りで先頭に付けることで、名前付きバッグ（Laravelの `withErrors($errors, 'profile')` という形式）の下にエラーをスコープします。

```rust
let mut errs = ValidationErrors::new();
errs.add_to_bag("profile", "bio", "must be under 280 characters");
errs.add_to_bag("billing", "card", "expired");
// errors マップ: { "profile.bio": [...], "billing.card": [...] }
```

`retain_fields` は、指定されたエントリだけを残します - これはPrecognitionの `Precognition-Validate-Only` ヘッダーの内部で使われており、サーバーは完全なバリデーションを実行しつつ、クライアントが尋ねたフィールドについてのみエラーを報告します。

## 変換の契約

`FrameworkError` がHTTP境界に到達すると、`From<FrameworkError> for HttpResponse` を通過します。次の3つのことが、この順序で起こります。

1. **ステータスのルーティング**。バリアントの `status_code()` が1度だけ読み取られます。
2. **ロギングと可観測性**。5xxは `tracing::error!` を発火させ、`ErrorOccurred` をディスパッチします。4xxは `tracing::warn!` を発火させます。どちらも、スコープ内にリクエストIDがあれば、それを運びます。
3. **ボディのレンダリング**。Laravel形式のJSONボディが生成され、5xxの場合はサニタイズされます。

### 通常のボディの形状

共通のレンダラーに到達する通常のエラーレスポンスは、次のJSONの骨格に従います。

```json
{
    "message": "<human readable>",
    "errors": { "field": ["msg", ...] },
    "request_id": "<uuid>" | null,
    "debug_message": "<dev only>"
}
```

- `message` はこれらの通常のレスポンスに常に存在します。
- `errors` はバリデーション系のエラー（`Validation`、`ValidationError`）でのみ現れます - どちらも同じ形状でレンダリングされるため、利用側は1つの経路だけを解析します。
- `request_id` はこれらの通常のレスポンスに現れます（起動の初期段階やリクエストコンテキストを持たないテストなど、リクエストスコープ外では `null` になります）。
- `debug_message` は、`APP_DEBUG=true` のときに通常の5xxに対してのみ現れます。これは純粋に付加的なものです - 本番環境のクライアントは、これに依存してはいけません。

3つの特別なバリアントはrequest-idの注入前に返ります:

- `PrecognitionSuccess` はボディなしの204レスポンスです。
- `PrecognitionFailure` はバリデーションボディにPrecognitionヘッダーを加えたものです。
- 誤ってHTTPレンダリングされた `AlreadyReported` の番兵は、`message` だけを含む汎用的な500レスポンスです。

### 5xxのサニタイズ規則

これは、覚えておく価値のある安全性の保証です。共通のレンダラーに到達するステータスが500以上のあらゆるエラーについて、JSONボディの `message` は、次のリテラル文字列に置き換えられます。

```json
{ "message": "Internal Server Error", "request_id": "..." }
```

生のエラー詳細が、レスポンスボディに漏れることは**ありません**。詳細が向かう先は次の通りです。

- リクエストIDとステータスを伴う `tracing::error!` のログエントリ
- 任意のリスナーが受け取れる `ErrorOccurred` イベント

`APP_DEBUG=true` の場合（`local`/`dev`/`test` 以外ではデフォルトで `false` です）、レスポンスには生の詳細を持つ `debug_message` フィールドも付与されます - ですが `message` はどちらのモードでも汎用的なままなので、フロントエンドやクライアントが、開発専用のデータに誤って依存してしまうことはありません。

この契約があるからこそ、`FrameworkError::internal("db connection refused: password mismatch on user 'app_rw'")` のように呼び出しても、パスワードがレスポンスに漏れることはありません。あなたが渡す `message` はログを読むオペレーター向けのものであり、クライアントが目にする `message` は `"Internal Server Error"` です。

4xxのエラーについては、呼び出し元向けのメッセージがそのまま保たれます - `404 User not found`、`400 Missing required parameter: user_id` のようにです。これらは内部的な失敗ではなく、クライアントが対処すべきドメインエラーです。

### 契約の置き場所

変換の全体は、1つの関数です - `framework/src/http/response.rs` にある `impl From<FrameworkError> for HttpResponse` です。これを一度読めば、Suprnovaのエラーレンダリング表面のすべてを読んだことになります。それ以外の経路はありません。

## パニック境界

ミドルウェアやハンドラでのパニックは、そうでなければ、コネクションごとのタスクを伝って上へ伝播し、レスポンスの途中でhyperサービスを崩壊させ、クライアントにはTCPリセットが残り、HTTPレスポンスは一切返りません。Suprnovaはこれを捕捉します。

`framework/src/server.rs` の `execute_chain_safely` は、ミドルウェアチェーンを `AssertUnwindSafe(...).catch_unwind().await` でラップします。パニックが発生すると、次のことを行います。

1. パニックのペイロードを取り出します（`&'static str` と `String` のペイロードを扱い、それ以外は `"panic with non-string payload"` として表面化します）。
2. リクエストのメソッド、パス、IDとともに `tracing::error!` でログを記録します。
3. `FrameworkError::internal(format!("request handler panicked: {msg}"))` を構築し、他のあらゆる5xxが使うのと*同じ* `From<FrameworkError> for HttpResponse` の変換を通します。
4. リクエストIDを `X-Request-Id` として返します。

パニックのペイロードはログエントリにとどまり、クライアントが受け取るのはサニタイズされた `{"message": "Internal Server Error"}` というボディです。返された5xxエラーに対して `ErrorOccurred` で発火する可観測性リスナーは、パニックに対しても同様に発火します - 配線すべき別個のパニックイベント表面はありません。

同じパニックリカバリのパターンは、次のものでも使われています。

- WebSocketハンドラ（`framework/src/server.rs`）
- スケジュールされたタスク（`framework/src/schedule/mod.rs`）
- ワークフロー（`framework/src/workflow/mod.rs`）
- `Supervisor` トレイト（ブロードキャスト）

これらのサブシステムのいずれかでパニックが発生した場合、それはログに記録され、エラー状態への変換か自動再起動のいずれかが行われます。ワーカータスクを道連れにすることはありません。

## `ErrorOccurred` で可観測性をフックする

`ErrorOccurred` は、フレームワークがあらゆる5xxレスポンス（パニックから合成されたものも含む）で発火する組み込みイベントです。

```rust
pub struct ErrorOccurred {
    pub error_message: String,
    pub status_code: u16,
    pub request_id: Option<String>,
}
```

他のイベントを購読するのと同じ方法で、これを購読します。

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
EventFacade::listen::<ErrorOccurred, _>(Arc::new(SentryReporter)).await;
```

これは、グローバル例外ハンドラにおけるLaravelの `report()` コールバックに相当する、Suprnovaの仕組みです。このイベントには、サニタイズされる前の元の `error_message`（クライアントが目にするボディは、引き続きサニタイズされます）、ステータスコード、そして突き合わせ可能なリクエストIDが渡されます。

### 完全なチェーンをレンダリングする: `render_error_chain`

`thiserror` が生成する `Display` はエラー自身のメッセージだけを出力するため、`FrameworkError::External` のラップされた `source` は、何かがチェーンをたどらない限り見えません。`render_error_chain` はこの走査を行い、`.context()` が使うのと同じ区切り文字 `": "` で結果を結合します。フレームワークは上記の `error_message` を構築する前と、対応する5xxログ行の前にこれを呼び出すため、ラップされたエラーはどちらの場所でも原因を失いません。

リスナーやログシンクが同じ完全チェーンのレンダリングを必要とする場合、たとえばフラットな文字列しか受け取らないシンクへ転送する前に `error_message` を再ラップする場合は、自分でこれを使用してください:

```rust
use suprnova::render_error_chain;

let chain = render_error_chain(&err);
// "loading users: connection refused (os error 111)"
```

## 中断ヘルパー

3つのフリー関数が、指定したステータスでハンドラをショートサーキットさせます。これらは、Laravelの `abort` / `abort_if` / `abort_unless` をそのまま反映したものです。

```rust
use suprnova::{abort_with, abort_if, abort_unless, Auth, Request, Response, json_response};

pub async fn show(req: Request) -> Response {
    abort_unless(Auth::check(), 401, "must be logged in")?;
    abort_if(req.param("id")? == "0", 404, "User not found")?;
    abort_with(503, "scheduled maintenance")?;
    Ok(json_response!({ "ok": true }))
}
```

それぞれが `Result<(), FrameworkError>` を返します。`?` と組み合わせて使ってください。背後にあるエラーは `FrameworkError::Domain { message, status_code }` であるため、他のあらゆるエラーと同じボディ形状・サニタイズ規則を通じてレンダリングされます。範囲外のステータスコードは、レスポンスレンダラーのステータス検証によって500に補正されるため、呼び出し側で不正な入力を防御する必要はありません。

## CLIの番兵: `AlreadyReported`

`FrameworkError` の1つのバリアントには、HTTP上の意味がありません。`AlreadyReported` は `FrameworkError::silent()` を介して構築され、clapがすでに自身の引数解析エラーを整形して出力し終えている場合に、コンソールディスパッチャーによって使われます。バイナリの `main` は、この番兵を `eprintln` なしで非ゼロの終了コードへと変換するため、ユーザーが同じ失敗に対して2つのエラーメッセージを目にすることはありません。

`AlreadyReported` がHTTPレスポンスコンバータに到達してしまった場合、それはリクエストハンドラが誤って `silent()` を返したことを示しています。コンバータは、この漏れを特定する大きな `tracing::error!` のログを記録し、`{"message": "Internal Server Error"}` だけを含む汎用的な500を返します。このバリアントはリクエスト経路には本来関係がなく、大きなログによって、このバグは沈黙したままにならず、観測可能になります。

通常、このバリアントを目にすることはありません。ここで文書化しているのは、この列挙型が「HTTP寄り」の性格を持つため、説明のないこのバリアントが、ソースを読む人を戸惑わせてしまうからです。

## 安全性の保証 - まとめ

Suprnovaが提供する契約は、次の通りです。

- **網羅的な変換**。あらゆる `FrameworkError` は `HttpResponse` を生成します。サーバーをクラッシュさせたり、コネクションを黙って落としたりするエラー経路はありません。
- **サニタイズされた5xx**。共通のレンダラーは、あらゆる5xxのワイヤ上の `message` を `Internal Server Error` に置き換えます。生の詳細はログと `ErrorOccurred` へ流れます。誤ってHTTPレンダリングされた `AlreadyReported` の番兵は、`request_id` なしで同じ汎用メッセージを返します。
- **任意のデバッグ可視性**。`APP_DEBUG=true` は通常の5xxレスポンスに `debug_message` フィールドを追加しますが、`message` には決して追加しません。本番環境のクライアントが、開発専用のデータに誤って依存することはありません。
- **突き合わせ可能なリクエストID**。共通のレンダラーに到達する通常のあらゆるエラーボディはリクエストID（リクエストのスコープが存在しない場合は `null`）を運び、同じIDがログ行と `ErrorOccurred` イベントの両方に現れます。上で説明した3つの早期返却バリアントはこのフィールドを迂回します。
- **パニックリカバリ**。ハンドラとミドルウェアでのパニックは捕捉され、ログに記録され、返されたエラーと同じ `From` の実装を通じてルーティングされます。コネクションが落ちることも、可観測性の空白が生じることもありません。
- **通常のエラーに共通する1つの形状**。共通のレンダラーに到達するバリデーションエラー、パラメーターエラー、パニック、カスタムドメインエラー、ストレージ障害は、同じJSONの骨格を使います。上で文書化した3つの特別なバリアントは異なるワイヤ形状を持ちます。

## 各要素の実装場所

| 要素 | ファイル |
|---|---|
| `FrameworkError`、`AppError`、`HttpError`、`ValidationErrors` | `framework/src/error.rs` |
| `render_error_chain` | `framework/src/error.rs` |
| `From<FrameworkError> for HttpResponse`（変換とサニタイズ） | `framework/src/http/response.rs` |
| `abort`、`abort_if`、`abort_unless` | `framework/src/http/abort.rs` |
| `execute_chain_safely`（パニック境界） | `framework/src/server.rs` |
| `ErrorOccurred` イベント | `framework/src/events/builtins.rs` |
| `#[domain_error]` マクロ | `suprnova-macros/src/domain_error.rs` |

## 次のステップ

- [エラーハンドリング](errors.md) - このモデルを使う、実践的なハンドラパターン
- [リクエスト ライフサイクル](lifecycle.md) - リクエストフローのどこでエラー変換が実行されるか
- [バリデーション](validation.md) - `#[derive(Validate)]`、フォームリクエスト、そして `ValidationErrors` がどのように埋められるか
- [レスポンス](responses.md) - `HttpResponse` ビルダー、ヘッダー、クッキー、ストリーミング
- [イベント](events.md) - `ErrorOccurred` やその他の組み込みイベントを購読する
