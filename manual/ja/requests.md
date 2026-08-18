# リクエスト

Suprnovaのハンドラは、`Request` - 通信レベルのHTTPリクエスト - を受け取るか、あるいは、あなたのコードが動き出す前にボディをパースし、バリデーションし、認可する型付きのフォームリクエスト構造体を受け取ります。どちらの経路も同じ `#[handler]` マクロの上に乗っており、ルートごとにどちらの形にするかを選びます。この章では両方を扱い、加えてマルチパートアップロードのエクストラクターと、ミドルウェアで手を伸ばすことになる生のアクセッサーも扱います。

## 型付きフォームリクエスト

`#[request]` アトリビュートは、構造体を `FormRequest` としてマークします。このマクロは `serde::Deserialize` と `validator::Validate` のderiveを追加し、`impl FormRequest` を出力します。これによって `#[handler]` マクロは、入ってくる途中でそれを抽出しバリデーションすればよいと分かります:

```rust
use suprnova::request;

#[request]
pub struct CreateUserRequest {
    #[validate(email(message = "Please provide a valid email address"))]
    pub email: String,

    #[validate(length(min = 8, message = "Password must be at least 8 characters"))]
    pub password: String,

    #[validate(length(min = 1, max = 100, message = "Name is required"))]
    pub name: String,
}
```

この型をパラメータとして名指ししたハンドラには、すでにバリデーション済みの値が手渡されます:

```rust
use suprnova::{handler, json_response, Response};
use crate::requests::CreateUserRequest;

#[handler]
pub async fn store(form: CreateUserRequest) -> Response {
    // `form` はバリデーション済みです - このコードは、すべてのルールを通過した場合にのみ実行されます。
    json_response!({ "email": form.email, "name": form.name })
}
```

代わりに `Request` を名指ししたハンドラには、生のリクエストがそのまま渡ってきます:

```rust
use suprnova::{handler, json_response, Request, Response};

#[handler]
pub async fn index(req: Request) -> Response {
    json_response!({ "path": req.path() })
}
```

どちらもエクストラクターです - `#[handler]` マクロはすべてのパラメータの型について `FromRequest::from_request` を探しに行き、`FormRequest` を実装した構造体には、包括的な `FromRequest` の実装が無償で付いてきます。

## バリデーションルール

バリデーションは `validator` クレートを通じて実行されます。よく使うルールは次のとおりです。

### 文字列のバリデーション

```rust
#[request]
pub struct ExampleRequest {
    // 必須（空文字列は不可）
    #[validate(length(min = 1, message = "This field is required"))]
    pub name: String,

    // メールアドレスの形式
    #[validate(email(message = "Invalid email address"))]
    pub email: String,

    // URL の形式
    #[validate(url(message = "Invalid URL"))]
    pub website: String,

    // 長さに関する制約
    #[validate(length(min = 8, max = 100))]
    pub password: String,

    // 正規表現パターン - PHONE_REGEX は、バリデータの展開地点から見える
    // `static` または `const` でなければなりません。通常は同じモジュールの中で、
    // 一度だけ宣言します:
    #[validate(regex(path = "PHONE_REGEX", message = "Invalid phone number"))]
    pub phone: String,
}

use std::sync::LazyLock;
use regex::Regex;

// validator 0.20 は `std::sync::LazyLock<Regex>` に対して `AsRegex` を実装しますが、
// `once_cell::sync::Lazy<Regex>` に対しては実装しません - derive の
// `#[validate(regex(path = "..."))]` の展開が型検査を通るよう、std の型を使ってください。
static PHONE_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\+?[0-9\s\-()]{7,20}$").unwrap());
```

### 数値のバリデーション

```rust
#[request]
pub struct ProductRequest {
    // 範囲のバリデーション - リテラルはフィールドの型と一致していなければなりません。
    // `f64` は `0.0` / `10000.0` を取り、整数リテラルの `0` / `10000` は取りません。
    #[validate(range(min = 0.0, max = 10000.0, message = "Price must be between 0 and 10000"))]
    pub price: f64,

    // 最小値のバリデーション
    #[validate(range(min = 1))]
    pub quantity: i32,

    // 最大値のバリデーション
    #[validate(range(max = 100))]
    pub discount_percent: i32,
}
```

### ネストとコレクションのバリデーション

```rust
use serde::Deserialize;

#[derive(Deserialize, Validate)]
pub struct Address {
    #[validate(length(min = 1))]
    pub street: String,

    #[validate(length(min = 1))]
    pub city: String,
}

#[request]
pub struct OrderRequest {
    // 入れ子になった構造体のバリデーション
    #[validate(nested)]
    pub shipping_address: Address,

    // コレクションの長さ
    #[validate(length(min = 1, message = "At least one item required"))]
    pub items: Vec<String>,
}
```

### よく使うバリデーション用アトリビュート

| アトリビュート | 説明 | 例 |
|-----------|-------------|---------|
| `email` | 正しいメールアドレスの形式 | `#[validate(email)]` |
| `url` | 正しいURLの形式 | `#[validate(url)]` |
| `length` | 文字列・コレクションの長さ | `#[validate(length(min = 1, max = 100))]` |
| `range` | 数値の範囲 | `#[validate(range(min = 0, max = 100))]` |
| `regex` | 正規表現パターンとの一致 | `#[validate(regex(path = "PATTERN"))]` |
| `contains` | 文字列が部分文字列を含むこと | `#[validate(contains(pattern = "@"))]` |
| `does_not_contain` | 文字列が部分文字列を含まないこと | `#[validate(does_not_contain(pattern = "admin"))]` |
| `nested` | ネストした構造体をバリデーションする | `#[validate(nested)]` |

## バリデーションエラーのレスポンス

バリデーションが失敗すると、Suprnovaは、Laravel / Inertia互換のエラーバッグを伴う422のレスポンスを返します:

```json
HTTP 422 Unprocessable Entity

{
    "message": "The given data was invalid.",
    "errors": {
        "email": ["Please provide a valid email address"],
        "password": ["Password must be at least 8 characters"]
    }
}
```

この `errors` の形は、`@inertiajs/*` のクライアントが `usePage().props.errors` から直接読み取るものと一致します。

### 入れ子になったフィールド

`#[validate(nested)]` の失敗は、完全なパスを名指しするドット区切りのキーの下で報告されます。これはLaravelが使うのと同じ記法です。入れ子になった構造体は `parent.field` を、バリデーションされる `Vec<T>` の要素は `parent.<index>.field` を与えます:

```json
{
    "message": "The given data was invalid.",
    "errors": {
        "shipping_address.street": ["Validation failed for field 'shipping_address.street'"],
        "items.1.name": ["Validation failed for field 'items.1.name'"]
    }
}
```

インデックスの `1` は2番目の要素です - 最初の要素は通過したため、バッグには存在しません。クライアント側では、そのキーをそのままバインドしてください: `form.errors['items.1.name']`。

## 完全な例

ユーザー登録のエンドポイントを、端から端まで通して見てみましょう。

**リクエストを定義します:**

```rust
// src/requests/create_user.rs
use suprnova::request;

#[request]
pub struct CreateUserRequest {
    #[validate(email(message = "Please provide a valid email address"))]
    pub email: String,

    #[validate(length(min = 8, message = "Password must be at least 8 characters"))]
    pub password: String,

    #[validate(length(min = 2, max = 50, message = "Name must be between 2 and 50 characters"))]
    pub name: String,
}
```

**コントローラーを作ります:**

```rust
// src/controllers/user.rs
use suprnova::{handler, json_response, Request, Response, ResponseExt};
use crate::requests::CreateUserRequest;

#[handler]
pub async fn index(_req: Request) -> Response {
    json_response!({ "users": [] })
}

#[handler]
pub async fn store(form: CreateUserRequest) -> Response {
    // バリデーションを通過 - ユーザーを作成します
    // 実際のアプリでは、ここでデータベースに保存することになります

    json_response!({
        "user": {
            "email": form.email,
            "name": form.name
        },
        "message": "User created successfully"
    })
    .status(201)
}
```

**ルートを登録します:**

```rust
// src/routes.rs
use suprnova::{get, post, routes};
use crate::controllers;

routes! {
    get!("/users", controllers::user::index).name("users.index"),
    post!("/users", controllers::user::store).name("users.store"),
}
```

## 認可とフィールド横断のフック

`FormRequest` トレイトは、3つのライフサイクルフックを公開しています: `authorize`、`after_validation`、`after_validation_async` です。`#[request]` アトリビュートも `#[derive(FormRequestDerive)]` の形も、どちらもデフォルトの `impl FormRequest` を出力してくれます。いずれかのフックをオーバーライドするには、`#[form_request(custom_hooks)]` というオプトアウトを付けてデフォルトの実装を抑止し、その上で自分の実装を書いてください。（これは `#[multipart(custom_hooks)]` のパターンを反映したものです。）

```rust
use suprnova::{FormRequest, FormRequestDerive, Request};
use serde::Deserialize;
use validator::Validate;

#[derive(Deserialize, Validate, FormRequestDerive)]
#[form_request(custom_hooks)]
pub struct DeleteUserRequest {
    pub user_id: i64,
}

impl FormRequest for DeleteUserRequest {
    fn authorize(req: &Request) -> bool {
        // ボディが読まれる前に 403 Forbidden でショートサーキットさせるには、
        // false を返してください。
        req.header("X-Admin-Token").is_some()
    }
}
```

このオプトアウトは、`#[request]` アトリビュートの形の下でも機能します - アトリビュートによる自動のderiveは欲しいけれど、フックはオーバーライドしたい、というときに便利です:

```rust
use suprnova::{FormRequest, Request, request};

#[request]
#[form_request(custom_hooks)]
pub struct DeleteUserRequestAttr {
    pub user_id: i64,
}

impl FormRequest for DeleteUserRequestAttr {
    fn authorize(req: &Request) -> bool {
        req.header("X-Admin-Token").is_some()
    }
}
```

`authorize` が `false` を返した場合、抽出は `FrameworkError::Unauthorized` を返し、次のように描画されます:

```json
HTTP 403 Forbidden

{ "message": "This action is unauthorized." }
```

`after_validation` は、同期的にフィールドを横断するフックです - 「パスワードと確認用の入力が一致していなければならない」といったルールに使ってください。`after_validation_async` はその非同期版であり、データベースを裏付けとするルール（組み込みの `Unique` など）が自動バリデーションに参加するのはここです。どちらも、フィールドごとの `validator` のルールを通過した後に発火します。`extract` は、最初に失敗した段階で処理を打ち切ります。

```rust
use suprnova::{FormRequest, FormRequestDerive, ValidationErrors};
use serde::Deserialize;
use validator::Validate;

#[derive(Deserialize, Validate, FormRequestDerive)]
#[form_request(custom_hooks)]
pub struct UpdatePasswordRequest {
    #[validate(length(min = 8))]
    pub new_password: String,
    pub confirmation: String,
}

impl FormRequest for UpdatePasswordRequest {
    fn after_validation(&self) -> Result<(), ValidationErrors> {
        if self.new_password != self.confirmation {
            let mut errs = ValidationErrors::new();
            errs.add("confirmation", "passwords do not match");
            return Err(errs);
        }
        Ok(())
    }
}
```

### ボディサイズの上限

構造体ごとの `#[form_request(max_body_bytes = N)]` アトリビュートは、プロセス全体で共通の8 MiBという上限を、1つのFormRequestについてだけ上書きします:

```rust
use suprnova::FormRequestDerive;
use serde::Deserialize;
use validator::Validate;

#[derive(Deserialize, Validate, FormRequestDerive)]
#[form_request(max_body_bytes = 64 * 1024 * 1024)] // 64 MiB
pub struct ImportPayload {
    pub rows: Vec<Row>,
}

#[derive(Deserialize, Validate)]
pub struct Row { /* ... */ }
```

`Content-Length` は最初にパースされ、申告されたサイズが上限を超えている場合、ボディを1バイトも読む*前に*リクエストはHTTP 413で拒否されます。`Content-Length` について嘘をつくクライアントも、読み取りの最中にストリーミングのバイトカウンタに引っかかります。

## コンテンツタイプの検出

`FormRequest::extract` が見るのは、`Content-Type` ヘッダーだけです:

- `application/x-www-form-urlencoded` → `serde_urlencoded` でパースされます
- `application/json` または `application/*+json` というサフィックスを持つもの → `serde_json` でパースされます
- それ以外のすべて（ヘッダーが欠けている場合も含む） → ボディが読まれる前に、HTTP 415 Unsupported Media Type で拒否されます

マルチパートのボディ（`multipart/form-data`）については、後述の[ファイルアップロード](#ファイルアップロード-multipartrequest)を参照してください。

## ボディを直接読む

単発のエンドポイントや、完全な `FormRequest` までは必要としないミドルウェアのために、`Request` 型そのものが3つの流儀でボディを読み取ります - ボディは高々一度しか読めないため、どれも `self` を消費します:

```rust
use serde::Deserialize;
use suprnova::{handler, json_response, Request, Response};

#[derive(Deserialize)]
struct LoginForm { username: String, password: String }

#[handler]
pub async fn login(req: Request) -> Response {
    // パース方法を明示的に選びます。
    let form: LoginForm = req.form().await?;
    json_response!({ "user": form.username })
}

#[handler]
pub async fn webhook(req: Request) -> Response {
    // 同じ形で、通信上は JSON。
    let payload: serde_json::Value = req.json().await?;
    json_response!({ "received": payload })
}

#[handler]
pub async fn ingest(req: Request) -> Response {
    // Content-Type に基づいて自動で選択します - `application/x-www-form-urlencoded`
    // が明示されていない限り JSON です。
    let value: serde_json::Value = req.input().await?;
    json_response!({ "value": value })
}
```

生のアクセスが必要なら、`req.body_bytes().await` がバッファリング済みの `Bytes` と、`RequestParts` のメタデータ（ルートパラメータとコンテンツタイプ）を返します。グローバルな8 MiBの上限を場合ごとに上書きするには、`body_bytes_with_cap(n)` を使ってください。

## フォームと並べてサービスを解決する

バリデーション済みのフォームリクエストは、[サービス コンテナ](container.md)と組み合わせられます。ハンドラの内部で `App::resolve::<T>()`（または `App::get::<T>()`）を使ってください:

```rust
use suprnova::{handler, json_response, Response, App};
use crate::requests::CreateUserRequest;
use crate::services::UserService;

#[handler]
pub async fn store(form: CreateUserRequest) -> Response {
    let user_service = App::resolve::<UserService>()?;
    let user = user_service.create_user(&form.email, &form.name).await?;
    json_response!({ "user": user })
}
```

## ファイルアップロード（`MultipartRequest`）

`multipart/form-data` には専用のエクストラクターがあります - `#[derive(MultipartRequest)]` はボディをパートごとにストリーミングし、設定された閾値を超える大きなファイルのパートを一時ファイルへ退避させるため、200 MiBのアップロードがRAMに丸ごと居座ることはありません。各フィールドには、通信上のフィールド名を指定する `#[field("name")]` という注釈を付けます。ファイルのフィールドには `UploadedFile<V>` を使い、`V` は `suprnova::http::upload::validators` にあるバリデータ（またはバリデータのタプル）です。

```rust
use suprnova::{handler, json_response, MultipartRequest, Response};
use suprnova::http::upload::UploadedFile;
use suprnova::http::upload::validators::{Image, MaxSize};

#[derive(MultipartRequest)]
pub struct AvatarUpload {
    #[field("avatar")]
    pub avatar: UploadedFile<(Image, MaxSize<5_242_880>)>, // 5 MiB の上限
    #[field("caption")]
    pub caption: Option<String>,
}

#[handler]
pub async fn upload_avatar(form: AvatarUpload) -> Response {
    // `avatar` は、サイズに応じてメモリ上か一時ファイル上に置かれます。
    // `.bytes()` はどちらも読み取り、`.store_as(...)` はディスクへストリーミングします。
    let bytes = form.avatar.bytes().await?;
    json_response!({ "size": bytes.len(), "caption": form.caption })
}
```

フィールドの形:

| 宣言 | 通信上の形 |
|---|---|
| `UploadedFile<V>` | 必須のファイル |
| `Option<UploadedFile<V>>` | 省略可能なファイル |
| `Vec<UploadedFile<V>>` | 配列でのアップロード（`photos[]`） |
| `String` / `u32` / `FromStr` を実装するあらゆる型 | テキストフィールド（必須） |
| `Option<String>` / `Option<T: FromStr>` | 省略可能なテキストフィールド |
| `Vec<String>` / `Vec<T: FromStr>` | 繰り返されるテキストフィールド |

`suprnova::http::upload::validators` にある組み込みのバリデータ:

- `MaxSize<N>` - 累積の合計が `N` バイトを超えた時点で、そのバイト境界でショートサーキットします（HTTP 413）。
- `Image` - マジックバイトが `image/*` を名乗っていないパートを拒否します。
- `MimeType<L>` - あなた自身の `MimeAllowlist` 型が提供する、固定の許可リストを受け付けます。
- `()` - 何もしません。`UploadedFile<()>` はどんなバイト列でも受け付けます。

バリデータはタプルとして組み合わせられます: `(Image, MaxSize<5_242_880>)` は両方を実行し、最初の失敗でショートサーキットします。

### フィールドごとの上限と配列の境界

ボディ全体に対するバイト数の上限はグローバルです（マルチパートではデフォルトで8 MiB、`suprnova::http::upload::set_global_max_multipart_body_bytes` で設定できます）。フィールドごとの上限は、小さなパートを大量に詰めたボディが、バイト数の予算に収まったまま `Vec<UploadedFile<_>>` を無制限に成長させる、という悪用を防ぎます:

```rust
#[derive(MultipartRequest)]
pub struct Gallery {
    #[field("photos", max_count = 8)]
    pub photos: Vec<UploadedFile<MaxSize<1_048_576>>>,
}
```

その名前を持つ（`max_count` + 1）番目のパートは、メモリを確保する前にHTTP 422を返します。そのため、余分なパートが `Vec` の成長に到達することはありません。

### 認可とバリデーション後のフック

`MultipartRequest` は、`MultipartRequestHooks` トレイトを通じて `FormRequest` のフックを反映します。deriveがデフォルトで出力するのは空の実装です。自分の実装を使うには、`#[multipart(custom_hooks)]` でオプトインしてください:

```rust
use suprnova::{MultipartRequest, Request, ValidationErrors};
use suprnova::http::upload::{MultipartRequestHooks, UploadedFile};

#[derive(MultipartRequest)]
#[multipart(custom_hooks)]
pub struct GuardedUpload {
    #[field("file")]
    pub file: UploadedFile,
}

impl MultipartRequestHooks for GuardedUpload {
    fn authorize(req: &Request) -> bool {
        req.header("X-Admin-Token").is_some()
    }

    fn after_validation(&self) -> Result<(), ValidationErrors> {
        if self.file.size == 0 {
            let mut errs = ValidationErrors::new();
            errs.add("file", "empty file");
            return Err(errs);
        }
        Ok(())
    }
}
```

### ストレージへのストリーミング

`UploadedFile::store_as` は、そのパートを登録済みのストレージディスクへ書き込みます。ディスクを裏付けとするパートについては、経路は完全にストリーミングです（`opendal::Operator::writer` による64 KiBのチャンク）。メモリ上のパートは、1回の書き込み呼び出しで済ませます。ストレージのパスがコンテンツアドレス方式である場合は、内容から導かれる拡張子を使ってください - ファイル名のヘッダーは信頼できません:

```rust
use suprnova::Storage;

let disk = Storage::disk("avatars")?;
let path = format!("{}.{}", user.id, form.avatar.extension_from_magic());
form.avatar.store_as(&disk, &path).await?;
```

ストレージディスクのレジストリについては、[ファイルシステムとストレージ](filesystem.md)を参照してください。

## ファイルの構成

リクエストの標準的な構成:

```
src/
├── requests/
│   ├── mod.rs                 # すべてのリクエストを再エクスポート
│   ├── create_user.rs         # CreateUserRequest
│   ├── update_user.rs         # UpdateUserRequest
│   └── create_post.rs         # CreatePostRequest
├── controllers/
│   └── user.rs                # CreateUserRequest を使用
└── routes.rs
```

**src/requests/mod.rs:**
```rust
pub mod create_user;
pub mod update_user;

pub use create_user::CreateUserRequest;
pub use update_user::UpdateUserRequest;
```

## Inertiaによるエンドツーエンドの型安全性

リクエストは `InertiaProps` をderiveしてTypeScriptの型を生成することもでき、これによってRustのバックエンドからReactのフロントエンドまで、エンドツーエンドの型安全性が得られます。

### リクエストのTypeScript型を生成する

`#[request]` と並べて `InertiaProps` のderiveを追加します:

```rust
use suprnova::{request, InertiaProps};

#[request]
#[derive(InertiaProps)]
pub struct CreateTodoRequest {
    #[validate(length(min = 1, message = "Title is required"))]
    pub title: String,

    #[validate(length(max = 500))]
    pub description: Option<String>,
}
```

型生成を実行します:

```bash
suprnova generate-types
```

これによって、`frontend/src/types/inertia-props.ts` にTypeScriptの型が生成されます:

```typescript
export interface CreateTodoRequest {
  title: string
  description: string | null
}
```

### Inertiaによる型安全なフォーム

最もすっきりとフォームを扱えるのは、Inertiaの `<Form>` コンポーネントです:

```tsx
import { Form, usePage } from '@inertiajs/react'

export default function CreateTodo() {
  const { errors } = usePage().props

  return (
    <Form action="/todos" method="post">
      <input
        type="text"
        name="title"
        placeholder="Todo title"
      />
      {errors?.title && <span className="error">{errors.title}</span>}

      <textarea
        name="description"
        placeholder="Description (optional)"
      />

      <button type="submit">Create Todo</button>
    </Form>
  )
}
```

より細かく制御したい場合は、`<Form>` を `useForm` フックおよび生成された型と組み合わせてください:

```tsx
import { Form, useForm } from '@inertiajs/react'
import type { CreateTodoRequest } from '../types/inertia-props'

export default function CreateTodo() {
  const { data, setData, errors, processing } = useForm<CreateTodoRequest>({
    title: '',
    description: null,
  })

  return (
    <Form action="/todos" method="post">
      {({ processing }) => (
        <>
          <input
            type="text"
            name="title"
            value={data.title}
            onChange={(e) => setData('title', e.target.value)}
            placeholder="Todo title"
          />
          {errors.title && <span className="error">{errors.title}</span>}

          <textarea
            name="description"
            value={data.description || ''}
            onChange={(e) => setData('description', e.target.value || null)}
            placeholder="Description (optional)"
          />

          <button type="submit" disabled={processing}>
            Create Todo
          </button>
        </>
      )}
    </Form>
  )
}
```

### このderiveが与えてくれるもの

- TypeScriptが、フィールド名のタイプミスと型の不一致をコンパイル時に捕まえます。
- IDEの補完は、生成された `.ts` を直接読み取ります。
- Rust側でフィールドの名前を変えて `suprnova generate-types` を再実行すれば、TypeScript側の表面もそれに追従します。

生成パイプラインの全体像については、[TypeScript 型](frontend-typescript-types.md)を参照してください。

## リクエストのアクセッサー

上のバリデーション済みフォームのパターンに加えて、`Request` 型は通信レベルのリクエストを調べるためのLaravel風のアクセッサーを備えています - URL、ヘッダー、クエリ文字列、コンテンツネゴシエーション、ルートのメタデータ、クライアントのIPです。これらは、ミドルウェアの中、`FormRequest` と並べて生のアクセスも欲しいハンドラの中、そしてバリデーション付きのパースが適切な道具ではないあらゆる場面で役に立ちます。

### URLとパス

| メソッド | 戻り値 | 備考 |
|--------|---------|-------|
| `req.path()` | `&str` | 生のURIのパス。 |
| `req.decoded_path()` | `String` | パーセントエスケープを解決したパス。 |
| `req.segments()` | `Vec<String>` | `/` で分割したパス。空のセグメントは落とされます。 |
| `req.segment(index, default)` | `Option<String>` | 1始まりのセグメントアクセス。 |
| `req.url()` | `String` | スキーム + ホスト + パス（クエリ文字列は含みません）。 |
| `req.full_url()` | `String` | URL + クエリ文字列。 |
| `req.full_url_with_query(&[("k","v")])` | `String` | クエリのキーを追加、または上書きします。 |
| `req.full_url_without_query(&["k"])` | `String` | クエリのキーを取り除きます。 |

```rust
use suprnova::{handler, json_response, Request, Response};

#[handler]
pub async fn show(req: Request) -> Response {
    if req.is(&["admin/*"]) {
        // パスが admin/* のワイルドカードにマッチします
    }
    json_response!({ "url": req.full_url() })
}
```

### ホスト、スキーム、IP

| メソッド | 戻り値 | 取得元の順序 |
|--------|---------|--------------|
| `req.host()` | `Option<String>` | `X-Forwarded-Host` → `Host` → URIのオーソリティ。 |
| `req.http_host()` | `Option<String>` | ホストと、既定でないときはポートも。 |
| `req.scheme_and_http_host()` | `Option<String>` | `scheme://host:port`。 |
| `req.scheme()` | `&'static str` | [`secure`] が真なら `"https"`、そうでなければ `"http"`。 |
| `req.secure()` | `bool` | URIのスキーム → `X-Forwarded-Proto` → `X-Forwarded-Ssl: on`。 |
| `req.ip()` | `Option<String>` | `X-Forwarded-For[0]` → `X-Real-IP` → ピアのアドレス。 |
| `req.ips()` | `Vec<String>` | 連鎖の全体: プロキシのヘッダー、続いてピアのアドレス。 |
| `req.user_agent()` | `Option<&str>` | `User-Agent` ヘッダー。 |
| `req.port()` | `Option<u16>` | Hostヘッダーのポート → `X-Forwarded-Port` → URIのポート。 |

### ヘッダーとメソッド

| メソッド | 戻り値 |
|--------|---------|
| `req.has_header("X-Foo")` | `bool` |
| `req.bearer_token()` | `Option<String>`（最後の `Bearer ` 部分文字列。カンマは取り除かれます） |
| `req.is_method("POST")` | `bool`（大文字小文字を区別しません） |
| `req.ajax()` | `X-Requested-With: XMLHttpRequest` |
| `req.pjax()` | 真とみなせる `X-PJAX` ヘッダー |
| `req.prefetch()` | `X-Moz`、`Purpose`、`Sec-Purpose` のいずれかが `prefetch` |

### コンテンツネゴシエーション

```rust
if req.is_json() { /* Content-Type carries /json or +json */ }
if req.expects_json() { /* AJAX without Accept narrowing, or Accept prefers JSON */ }
if req.wants_json() { /* Accept header tops with JSON */ }
if req.accepts_html() { /* Accept allows text/html */ }

let preferred = req.prefers(&["application/json", "text/html"]);
let acceptable = req.acceptable_content_types();
```

`accepts(&[ty])` は、素の型と `application/<vendor>+json` 形式のサフィックスの両方にマッチします。`accepts_any_content_type()` は、Acceptヘッダーが存在しないか、最も優先度の高いものが `*/*` である場合に真を返します。

### クエリ文字列

```rust
let id: Option<String> = req.query_param("id");
let present: bool = req.has_query("id");
let map = req.query_params(); // HashMap<String, String>

// serde を介した型付きのクエリ解析
#[derive(serde::Deserialize)]
struct SearchQuery { page: u32, q: String }
let q: SearchQuery = req.query_into()?;
```

### ルートのメタデータ

ルーターがリクエストをディスパッチした後、マッチしたパターンがそのリクエストに記録されます:

```rust
if req.route_is(&["users.show", "users.*"]) {
    // users.show あるいは users.* のルートの中にいます
}

let pattern = req.route_pattern(); // Some("/users/{id}")
let name = req.route_name();       // Some("users.show")
```

`route_is(&[...])` は `*` のワイルドカードを受け付けます（Laravelの `Str::is` のセマンティクスです）。

## 早期に中断する

`Response` を丸ごと組み立てずに早期離脱でエラーを扱いたい場合、`abort_with` / `abort_if` / `abort_unless` というヘルパーが、標準の `From<FrameworkError> for HttpResponse` のパイプラインを通じて描画される `FrameworkError` を返します。これらは `?` と直接組み合わせられます:

```rust
use suprnova::{abort_if, abort_unless, abort_with, handler, json_response, Request, Response};

#[handler]
pub async fn show(req: Request) -> Response {
    let id = req.param("id")?;

    // リソースが見つからないときは404。
    abort_if(id == "0", 404, "User not found")?;

    // 呼び出し元が未認証のときは403。
    abort_unless(req.has_header("Authorization"), 403, "Login required")?;

    // あるいは、無条件にステータスを起こします:
    if some_condition() {
        return Err(abort_with(418, "I'm a teapot").unwrap_err().into());
    }

    json_response!({ "id": id })
}
```

`abort_if` / `abort_unless` は、条件が偽のときには `Ok(())` を返すため、`?` はそのまま先へ進みます。

## Suprnovaが異なる設計を選んだ理由

Laravelは、同期的で統合された入力バッグ - `$req->input('field')`、`$req->all()`、`$req->only(['a','b'])`、`$req->boolean('flag')` - を公開しており、これはクエリ文字列とパース済みのボディの両方からまとめて値を引いてきます。Suprnovaは、その表面を出荷していません。理由は次のとおりです:

- Suprnovaのボディは一度しか読めず、しかも非同期です。同期的な `all()` を用意するには、ほとんどのハンドラが決して呼ばないメソッドを満たすためだけに、あらゆるボディをあらかじめバッファリングしなければなりません - メモリとDoSの攻撃面は、PHPのリクエストごとにプロセスが立ち上がるライフサイクルとは事情が異なります。
- 型付きの代替手段（`#[request]` + `FormRequest`）は、コンパイル時に検査されるフィールド名、バリデーション、そしてコンテンツタイプを踏まえたパースを与えてくれます - まさに、型のないバッグに欠けている安全網です。

クエリ / ヘッダー / ルートを調べるには、`query_param`、`query_into`、`has_query`、`bearer_token`、そして上に挙げたヘッダーの読み取りメソッドに手を伸ばしてください。ボディ側にアクセスするには、`#[request]` 構造体か `#[derive(MultipartRequest)]` のエクストラクターを定義します。

## 次のステップ

- [バリデーション](validation.md) - `#[validate(...)]` の背後にあるルールライブラリと、422のエラーバッグの形
- [レスポンス](responses.md) - ハンドラから `HttpResponse` の値を組み立てて返すこと。ストリーミングとリダイレクトも含みます
- [エラーハンドリング](errors.md) - `Response` が `Result<HttpResponse, HttpResponse>` であることの上に組み立てられた、ハンドラのパターン
- [ルーティング](routing.md) - ルートの登録と、`req.param("id")` が読み取る `{id}` パラメータ
- [認証](authentication.md) - `Auth::user_as`、`Auth::attempt`、そしてリクエストから現在のユーザーを解決する認証ガード
- [ファイルシステムとストレージ](filesystem.md) - `UploadedFile::store_as` が書き込むストレージディスクを登録すること
