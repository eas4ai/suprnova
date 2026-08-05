# マクロ

Suprnovaは、およそ三十数個のマクロを同梱しており、そのすべてが `suprnova::*` から再エクスポートされています。これらは、フレームワークとあなたのコードが交わる関節にあたります - `routes!` はルーターを構築し、`#[handler]` は関数をハンドラへと適応させ、`#[suprnova::model]` は構造体をEloquentモデルへと変え、`#[derive(Data)]` は型付きのInertiaペイロードを生成します。この章はその索引です。各マクロについて、1段落の説明と、最小限の例、そして実際の作業でそれを使っている章へのポインタを載せています。

表面全体を通じて成り立つ、いくつかの原則があります。

- **マクロは完全修飾パスを出力します。** 生成されたコードは `::suprnova::…` と書くため、背後の型をインポートしているかどうかに関わらず、マクロは機能します。
- **`inventory::submit!` を多用します。** モデル、コマンド、ポリシー、オブザーバー、支払いプロバイダーなどは、コンパイル時に自分自身を登録し、フレームワークは起動時にそのレジストリを流し込みます。手作業で登録を配線することは、ほとんどありません。
- **見合うところではコンパイル時の検証を行います。** `inertia_response!` は、指定されたコンポーネントファイルが存在するかを確認します。`redirect!` は、指定されたルートが存在するかを確認します。`routes!` は、`/` で始まらないパスを拒否します。ビルド時に捕捉できるエラーは、そこで捕捉されます。

## ルーティング

| マクロ | 戻り値 | 何を行うか |
|---|---|---|
| `routes!` | `pub fn register() -> Router` | ルートのトップレベルの一覧です - あなたの `app.rs` が呼び出す `register()` をエクスポートします |
| `get!` / `post!` / `put!` / `delete!` / `patch!` / `head!` / `options!` / `any!` | `RouteDefBuilder<H>` | 1つのHTTPルートです - `.name(...)` / `.middleware(...)` をチェーンできます |
| `group!` | `GroupDef` | プレフィックスとミドルウェアを、子となるルートの一覧に適用します |
| `fallback!` | `FallbackDefBuilder<H>` | どのルートにもマッチしなかった場合のカスタム404ハンドラです |
| `ws!` | `WsRouteDef` | 1つのWebSocketルートです - `.middleware(...)` / `.config(...)` をチェーンできます |

```rust
use suprnova::{routes, get, post, ws, group};
use crate::{controllers, middleware::AuthMiddleware, ws::ChatHandler};

routes! {
    get!("/", controllers::home::index).name("home"),
    get!("/users/{id}", controllers::user::show).name("users.show"),
    post!("/users", controllers::user::store).name("users.store"),

    group!("/admin", {
        get!("/dashboard", controllers::admin::dashboard),
    }).middleware(AuthMiddleware),

    ws!("/ws/chat", ChatHandler),
}
```

ルートパスの文字列は、コンパイル時にチェックされます - `validate_route_path` は、`/` で始まらないものをすべて拒否します。`.name("…")` を介して登録されたルート名も、`register_route_name` を通じて、起動時に一意性がチェックされます。完全な展開については[ルーティング](routing.md)を、`ws!` については[WebSocket](websockets.md)を参照してください。

## ハンドラとリクエスト

### `#[handler]`

コントローラー関数を書き換え、（`FromRequest` を介して）型付きのパラメータを、受信したリクエストから直接抽出できるようにします - `Request` から手作業でフィールドを取り出す代わりに、ハンドラが必要とするものを宣言すれば、マクロがその配線を行います。

```rust
use suprnova::{handler, Response, json_response, request};

#[request]
pub struct CreateUserRequest {
    #[validate(email)]
    pub email: String,

    #[validate(length(min = 8))]
    pub password: String,
}

#[handler]
pub async fn store(form: CreateUserRequest) -> Response {
    // `form` はすでにバリデーション済みです - 失敗した場合は自動的に422が返ります
    json_response!({ "email": form.email })
}
```

`Request` 形の第一引数も、恒等的なケースとして引き続き受け付けられます。[コントローラー](controllers.md)を参照してください。

### `#[request]` と `#[derive(FormRequest)]`

`#[request]` は、バリデーション済みのリクエスト型を宣言するための推奨される方法です。`Deserialize`、`Validate`、`FormRequest` を自動的に導出するため、この構造体は `application/json` と `application/x-www-form-urlencoded` の両方のボディで機能します。

このアトリビュートを使わずに済ませたい場合の、背後にあるderiveが `#[derive(FormRequestDerive)]` です（その場合、`Deserialize` と `Validate` は自分で導出する必要があります）。私たちが推奨するのはこのアトリビュートであり、deriveはエッジケースのために存在しています。[リクエスト](requests.md)と[バリデーション](validation.md)を参照してください。

### `#[derive(MultipartRequest)]`

`multipart/form-data` 向けの、強く型付けされたエクストラクタです - テキストフィールドとアップロードされたファイルを1つの構造体にまとめて束ね、フィールドごとに型レベルのバリデータを付けられます。

```rust
use suprnova::{MultipartRequest};
use suprnova::http::upload::{Image, MaxSize, UploadedFile};

#[derive(MultipartRequest)]
pub struct AvatarUpload {
    #[field("avatar")]
    pub avatar: UploadedFile<(Image, MaxSize<5_242_880>)>,

    #[field("caption")]
    pub caption: Option<String>,
}
```

組み込みのバリデータ（`Image`、`MimeAllowlist<…>`、`MaxSize<…>`、`MimeType<…>`）は、タプルによって合成できます。[リクエスト](requests.md)を参照してください。

## レスポンス

### `json_response!` と `text_response!`

2つの短縮形のレスポンスマクロです。どちらも `HttpResponse::*` を `Ok(...)` でラップするため、ハンドラの戻り値の位置にそのまま収まります。

```rust
use suprnova::{handler, json_response, text_response, Response};

#[handler]
pub async fn health() -> Response {
    json_response!({ "status": "ok" })
}

#[handler]
pub async fn robots() -> Response {
    text_response!("User-agent: *\nDisallow:")
}
```

[レスポンス](responses.md)を参照してください。

### `inertia_response!`

Inertiaのページレスポンスを構築し、指定されたコンポーネントファイル（`.svelte` / `.tsx` / `.jsx` / `.vue`）が `frontend/src/pages/` に存在することを、コンパイル時に検証します。コンポーネント名を打ち間違えた場合、ビルドは候補の提案付きで失敗します。

```rust
use suprnova::{handler, inertia_response, InertiaProps, Request, Response};

#[derive(InertiaProps)]
struct HomeProps {
    title: String,
    user_count: i64,
}

#[handler]
pub async fn index(req: Request) -> Response {
    inertia_response!(&req, "Home", HomeProps {
        title: "Welcome".into(),
        user_count: 42,
    })
}
```

`#[derive(InertiaProps)]` は、レスポンスの形状が必要とする `Serialize` の実装を生成します。[Inertia レスポンス](frontend-inertia-responses.md)を参照してください。

### `redirect!`

名前付きルートへの、型安全なリダイレクトです - ルート名は、`routes!` を通じて登録された名前に対して、コンパイル時に検証されます。

```rust
use suprnova::redirect;

// "users.show" が登録済みのルート名である場合にのみコンパイルが通ります
let resp = redirect!("users.show").with("id", "42").into();
```

[URL 生成](urls.md)を参照してください。

## Eloquent

### `#[suprnova::model]`

プレーンな構造体を、完全なEloquentモデルへと変えます。SeaORMの `Entity`、`Model`、`ActiveModel`、`Column`、`Relation` のスタブを生成し、さらにEloquentが必要とするトレイト実装をすべて生成します。また、`ModelEntry` を `inventory::submit!` するため、フレームワークは起動時にすべてのモデルを列挙できます。

```rust
use suprnova::model;

#[model(table = "users")]
pub struct User {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
```

アトリビュートのキーには、`table`、`primary_key`、`key_type`、`auto_increment`、`connection`、`fillable`、`guarded`、`casts`、`timestamps`、`soft_deletes`、`appends`、`hidden`、`visible`、`mutators`、`touches`、そして（UUID/ULIDの主キー向けの）`unique_id` があります。[Eloquent](eloquent.md)を参照してください。

### `#[suprnova::scopes(Model)]`

`impl Model { … }` ブロックを走査し、シグネチャが `fn name(query: Builder<Self>[, args…]) -> Builder<Self>` に一致するすべてのメソッドをスコープへと変えます - `Model::scope_name(args)` と、`Builder<Model>` 上でチェーンできる `.scope_name(args)` の両方を生成します。

```rust
use suprnova::{scopes, Builder};

#[suprnova::scopes(User)]
impl User {
    pub fn active(query: Builder<Self>) -> Builder<Self> {
        query.filter("active", true)
    }

    pub fn popular(query: Builder<Self>, threshold: i64) -> Builder<Self> {
        query.filter_op("followers_count", ">", threshold)
    }

    // これはスコープではありません - そのまま素通りします
    pub fn display_name(&self) -> String { self.name.clone() }
}

// どちらの呼び出し方もコンパイルできます:
// User::active().popular(500).get().await?;
// User::query().filter_op("id", ">", 0).active().get().await?;
```

チェーン形式は、別のモジュールから呼び出す場合、生成されたトレイト `HasScope_<scope>_<Model>` がスコープ内にあることを必要とします。[Eloquent](eloquent.md)を参照してください。

### `#[suprnova::observer(Model)]`

`impl Observer<M>` ブロックを、ライフサイクルイベントの仕組みへと配線します - オーバーライドされた16個のメソッドそれぞれが、登録済みのリスナーとなり、インベントリに提出され、起動時に流し込まれます。

```rust
use async_trait::async_trait;
use suprnova::eloquent::observers::Observer;
use suprnova::eloquent::events::EventResult;
use suprnova::eloquent::attrs::Attrs;
use suprnova::FrameworkError;

pub struct AuditObserver;

#[suprnova::observer(User)]
#[async_trait]
impl Observer<User> for AuditObserver {
    async fn creating(&self, attrs: &mut Attrs) -> EventResult {
        if attrs.get("email").is_none() {
            return EventResult::cancel("email is required");
        }
        EventResult::ok()
    }

    async fn created(&self, user: &User) -> Result<(), FrameworkError> {
        tracing::info!(user.id = user.id, "user created");
        Ok(())
    }
}
```

**必須のアトリビュート順序: `#[suprnova::observer(M)]` は `#[async_trait]` より前に置かなければなりません。** アトリビュートマクロは外側から内側へと展開されます - もし `async_trait` が先に実行されると、あらゆる `async fn` を脱糖された形へと書き換えてしまい、observerマクロによる16個のトレイトメソッド名とのマッチングは、何も見つけられずに沈黙します。[イベント](events.md)を参照してください。

### `#[suprnova::accessor]` と `#[suprnova::mutator]`

`impl Model { … }` のメソッドに付ける関数レベルのマーカーで、モデルの `to_json()` / `fill()` の経路にフックします。マクロがそれらを配線できるよう、`#[model(appends = […])]`（アクセッサー）または `#[model(mutators = […])]`（ミューテータ）の中で、フィールド名を参照させてください。

```rust
#[suprnova::model(appends = ["full_name"], mutators = ["password"])]
pub struct User {
    pub id: i64,
    pub first_name: String,
    pub last_name: String,
    pub password: String,
}

impl User {
    #[suprnova::accessor]
    pub fn full_name(&self) -> String {
        format!("{} {}", self.first_name, self.last_name)
    }

    #[suprnova::mutator]
    pub fn set_password(
        &mut self,
        value: serde_json::Value,
    ) -> Result<(), suprnova::FrameworkError> {
        let raw: String = serde_json::from_value(value)
            .map_err(|e| suprnova::FrameworkError::validation("password", format!("{e}")))?;
        self.password = bcrypt(raw);
        Ok(())
    }
}
```

[ミューテータとキャスト](eloquent-mutators.md)を参照してください。

### `#[suprnova::prunable]`

`Prunable`（または `MassPrunable`）の実装をラップし、実行時に `model:prune` が走査するレジストリへと `PrunerEntry` を提出します。

```rust
use async_trait::async_trait;
use chrono::{Duration, Utc};
use suprnova::eloquent::Prunable;

#[suprnova::prunable]
#[async_trait]
impl Prunable for Session {
    fn prunable() -> suprnova::Builder<Self> {
        Self::query().filter_op(
            "expires_at",
            "<",
            (Utc::now() - Duration::days(30)).to_rfc3339(),
        )
    }
}
```

[Eloquent](eloquent.md)を参照してください。

### `attrs!`

`Model::create` / `Model::update` / `Model::fill` のために、順序付きの `Attrs` マップ（`IndexMap<&'static str, serde_json::Value>`）を構築します。

```rust
use suprnova::attrs;

let user = User::create(attrs! {
    name: "Alice",
    email: "alice@example.com",
    age: 32,
}).await?;
```

[Eloquent](eloquent.md)を参照してください。

### `casts!`

`Builder::with_casts` に渡せる、クエリごとのキャストマップを構築します。

```rust
use suprnova::{casts, AsDate, AsJson};

let map = casts! {
    birthday = AsDate,
    metadata = AsJson<serde_json::Value>,
};
let rows = User::query().with_casts(map).get().await?;
```

[ミューテータとキャスト](eloquent-mutators.md)を参照してください。

### `route_binding!`

手作りのSeaORMエンティティに対して `RouteBinding` を実装し、ルートパラメータから自動的に解決されるようにします。`#[suprnova::model]` で定義されたモデルは自動的に登録されるため、これは不要です - エンティティを手で書いた場合に、`route_binding!` を使ってください。

```rust
use suprnova::route_binding;

route_binding!(crate::entities::user::Entity, User, "user");
```

これを行うと、`get!("/users/{user}", controllers::user::show)` は、完全に読み込まれた `User` をハンドラに渡します。[ルーティング](routing.md)を参照してください。

## データとInertia

### `#[derive(Data)]`

型付きペイロードのための複合deriveです。`#[data(input_only)]` フィールドを尊重する `Serialize` の実装と、`#[data(output_only)]` フィールドを設定しようとするペイロードを拒否する `Deserialize` の実装を生成します。JSON:API出力のためには、`Resource` の章に沿って `#[json_resource("type")]` と組み合わせてください。

```rust
use suprnova::{Data, Validate};

#[derive(Data, Validate)]
struct UserDto {
    pub id: i64,
    pub name: String,

    #[data(input_only)]
    #[validate(length(min = 8))]
    pub password: String,

    #[data(output_only)]
    pub computed_handle: String,

    #[data(allow_include)]
    pub posts: Vec<PostDto>,
}
```

`#[data(allow_include)]` は、`inventory::submit!` を介して、部分リロードのインクルード許可リストにそのフィールドを登録します。[データ オブジェクト](data.md)と[APIリソース](eloquent-resources.md)を参照してください。

### `#[derive(InertiaProps)]`

`inertia_response!` が必要とする `Serialize` の実装を生成します。単純なマーカーderiveです - ほとんどのアプリは、代わりに `#[derive(Data)]` を使います。そちらであれば、部分リロードのインクルードが標準で手に入るためです。

```rust
use suprnova::InertiaProps;

#[derive(InertiaProps)]
struct DashboardProps {
    title: String,
    user: User,
}
```

[Inertia レスポンス](frontend-inertia-responses.md)を参照してください。

### `when_loaded!`

指定されたリレーションがエンティティ上でイーガーロードされている場合にのみ `Prop::lazy(…)` を発行し、そうでない場合は `Prop::EagerNone` を発行して、そのpropをレスポンスから完全に除外します。

```rust
use suprnova::when_loaded;

let songs_prop = when_loaded!(&artist, "songs", || async {
    serde_json::to_value(&artist.songs).unwrap()
});
```

[データ オブジェクト](data.md)を参照してください。

## 依存性の注入

### `#[service]`

トレイトに `Send + Sync + 'static` を追加し、コンテナに収まるようにします。

```rust
use suprnova::service;

#[service]
pub trait HttpClient {
    async fn get(&self, url: &str) -> Result<String, FrameworkError>;
}

// App::bind::<dyn HttpClient>(Arc::new(RealHttpClient::new()));
// let client = App::make::<dyn HttpClient>()?;
```

[サービス コンテナ](container.md)を参照してください。

### `#[injectable]`

具体型をシングルトンとして自動登録します。`Default` + `Clone` を導出し、起動時に実行される登録を提出します。

```rust
use suprnova::injectable;

#[injectable]
pub struct AppState {
    pub counter: u32,
}

// let state: AppState = App::get().unwrap();
```

[サービス コンテナ](container.md)を参照してください。

## エラー

### `#[domain_error]`

`Display`、`Error`、`HttpError`、そして `From<T> for FrameworkError` を実装するドメインエラーを定義します - これにより、`?` を介してハンドラをショートサーキットさせられます。

```rust
use suprnova::domain_error;

#[domain_error(status = 404, message = "User not found")]
pub struct UserNotFoundError {
    pub user_id: i32,
}

pub async fn get_user(id: i32) -> Result<User, FrameworkError> {
    let user = User::find(id).await?
        .ok_or_else(|| UserNotFoundError { user_id: id })?;
    Ok(user)
}
```

[エラーハンドリング](errors.md)を参照してください。

## コンソールとバックグラウンド作業

### `#[command]`

`async fn(Vec<String>) -> Result<(), FrameworkError>` を、コンソールコマンドとしてマークします。`CommandEntry` を提出するため、プロジェクトごとのコンソールバイナリが実行されたときに、`dispatch_argv` がそれを見つけられます。

```rust
use suprnova::{command, FrameworkError};

#[command(name = "db:seed", description = "Run all registered seeders")]
async fn db_seed(_args: Vec<String>) -> Result<(), FrameworkError> {
    suprnova::seed::run_all().await
}
```

[コンソール](console.md)を参照してください。

### `#[derive(Command)]`

型付き引数を使う場合の代替手段です。`#[derive(clap::Parser)]` の上に重ね、メタデータのために `#[console(...)]` を読み取り、あなたの `TypedCommand::run` を呼び出すランナーを生成します。

```rust
use async_trait::async_trait;
use suprnova::{Command, FrameworkError, TypedCommand};

#[derive(clap::Parser, Command)]
#[console(name = "greet", description = "Greet someone")]
pub struct Greet {
    #[arg(short, long)]
    name: Option<String>,
    #[arg(long)]
    loud: bool,
}

#[async_trait]
impl TypedCommand for Greet {
    async fn run(self) -> Result<(), FrameworkError> {
        let target = self.name.unwrap_or_else(|| "world".into());
        println!("{}", if self.loud { format!("HELLO {target}!") } else { format!("Hello {target}") });
        Ok(())
    }
}
```

[コンソール](console.md)を参照してください。

### `#[workflow]` と `#[workflow_step]`

`#[workflow]` は、async関数を永続的なワークフローとして登録します - 実行可能な状態、リトライ可能なステップ、永続化された履歴を持ちます。本体の中にある各 `#[workflow_step]` は、クラッシュや再起動の後にランタイムが再開できるチェックポイントです。

```rust
use suprnova::{workflow, workflow_step, FrameworkError};

#[workflow]
async fn onboard_user(user_id: i64) -> Result<(), FrameworkError> {
    send_welcome_email(user_id).await?;
    enable_default_features(user_id).await?;
    Ok(())
}

#[workflow_step]
async fn send_welcome_email(user_id: i64) -> Result<(), FrameworkError> {
    // …
    Ok(())
}
```

### `start_workflow!`

パスを指定してワークフローを開始し、引数をワークフローランタイムのエンベロープ形式へとシリアライズします。

```rust
use suprnova::start_workflow;

let handle = start_workflow!(crate::workflows::onboard_user, 42).await?;
```

[ワークフロー](workflows.md)を参照してください。

### `schedule_task!`

`TaskBuilder::from_async` を包むシンタックスシュガーであり、クロージャが、トレイトベースの `Task` の実装と並んで、きれいにスケジュールされるようにします。

```rust
use suprnova::{schedule_task, FrameworkError};

let task = schedule_task!(|| async {
    println!("ticking");
    Ok::<(), FrameworkError>(())
})
    .every_minute()
    .name("tick");
```

[タスク スケジューリング](scheduling.md)を参照してください。

## 認可

### `#[policy(UserType, ResourceType)]`

`impl Policy` ブロックをラップし、各メソッドを名前付きのゲートアクションとして登録します。ゲート名は、メソッド名と小文字化されたリソース型を組み合わせたものです - `Comment` に対する `fn view(...)` は `"view-comment"` になります。

```rust
use suprnova::policy;

struct CommentPolicy;

#[policy(User, Comment)]
impl CommentPolicy {
    fn view(_user: &User, _comment: &Comment) -> bool { true }
    fn update(user: &User, comment: &Comment) -> bool {
        comment.author_id == user.id
    }
}
```

`Server::run` は、`authorization::init_policies()` を自動的に呼び出します。[認可](authorization.md)を参照してください。

## 通知とメール

### `#[derive(NotificationMailable)]`

`#[mail(...)]` アトリビュートから `to_mail` を自動生成します - 件名、HTML本文、テキスト本文のための、インラインまたはファイルに保存されたTeraテンプレートです。コンパイル時のチェックには、件名が必須であること、少なくとも1つの本文が存在すること、html/html_templateが排他的であること、`from_name` が `from` を要求することが含まれます。

```rust
use serde::{Serialize, Deserialize};
use suprnova::NotificationMailable;

#[derive(Serialize, Deserialize, NotificationMailable)]
#[mail(
    subject = "Your order shipped - tracking {{ tracking }}",
    html    = "<p>Tracking: <code>{{ tracking }}</code></p>",
    text    = "Tracking: {{ tracking }}",
    from    = "orders@suprnova.dev",
)]
pub struct OrderShipped { pub tracking: String }
```

通知トレイト自体は手で実装するものであり、`#[derive(Notification)]` は存在しません。[通知](notifications.md)と[メール](mail.md)を参照してください。

## バリデーション

### `validate!`

同期的で宣言的なバリデーションのエントリーポイントです。各行はフィールド名を1つ以上の `Rule`（または `ContextualRule`）の値と対にし、「存在する場合のみバリデーションする」ための `?:` と、条件付きで必須になるオプションフィールドのための `?=>` を持ちます。

```rust
use suprnova::{validate, ValidationErrors};
use suprnova::validation::rules::*;

fn validate_form(self_ref: &SignupForm) -> Result<(), ValidationErrors> {
    validate! { self_ref =>
        email   => Required, Email;
        password => Required, Min(8);
        bio     ?: Max(500);
        card_number ?=> RequiredIf { other: "billing_type", value: "card" } => with ctx;
    }
}
```

`Validate` は `validator` クレートから再エクスポートされています - `#[validate(...)]` アトリビュート（例えば `#[validate(email)]`）は `validator` に由来し、`FormRequest` の同期経路を通じて実行されます。コンテキストに応じた/複数フィールドにまたがるルール、非同期ルール、あるいは `suprnova::validation::rules` パレットのルールが必要な場合は、`validate!` を使ってください。[バリデーション](validation.md)を参照してください。

## ファクトリー

### `#[derive(Factory)]`

兄弟にあたる `<Model>Factory` マーカーと、`fake::Faker` を介してモデルを生成する `Factory` の実装を生成します。モデルは `fake::Dummy<fake::Faker>` を実装している必要があります - 通常は `#[derive(Dummy)]` を介してです。

```rust
use suprnova::{Dummy, Factory};

#[derive(Dummy, Factory)]
pub struct User {
    pub id: i32,
    pub name: String,
    pub email: String,
}

// UserFactory が存在します:
let users = UserFactory::new().count(10).make_many();
```

[ファクトリー](eloquent-factories.md)を参照してください。

## テスト

### `#[suprnova_test]`

`async fn` のテストを、インメモリのSQLiteデータベース（デフォルトでは `crate::migrations::Migrator` を実行します）でラップし、`App::init()` と `App::boot_services()` を呼び出し、`#[tokio::test]` の下で本体を実行します。並列に実行されるテストは、コンテナのスレッドごとの層を通じて、互いに影響し合わない状態を保ちます - テスト固有のサービスは、（`App::bind` ではなく）`TestContainer::fake` を通じてバインドし、各スレッドが自分自身のフェイクだけを見るようにしてください。

```rust
use suprnova::suprnova_test;
use suprnova::testing::TestDatabase;

#[suprnova_test]
async fn creates_a_user(db: TestDatabase) {
    let user = User::create(attrs! { name: "A", email: "a@x.com" }).await.unwrap();
    assert!(user.id > 0);
}
```

カスタムのマイグレータは、`#[suprnova_test(migrator = MyMigrator)]` を介して指定します。[テスト](testing.md)を参照してください。

### `test_database!`

`#[suprnova_test]` を介して `db` パラメータを受け取らないテストのための、1行で書ける `TestDatabase` のコンストラクタです。

```rust
let db = test_database!();
let db = test_database!(my_crate::CustomMigrator);
```

### `describe!`, `test!`, `expect!`

Jestスタイルのグルーピングと、流れるようなアサーションです。`describe!` はモジュールであり、`test!` は（同期・非同期を問わず、`TestDatabase` パラメータの有無も問わず）`#[test]` を生成し、`expect!` は値をラップして、失敗時にファイル/行のコンテキスト付きで連鎖するアサーションを行えるようにします。

```rust
use suprnova::{describe, test, expect};

describe!("CreateUserAction", {
    test!("creates a user", async fn(db: TestDatabase) {
        let user = CreateUserAction::new()
            .execute("test@example.com").await.unwrap();
        expect!(user.email).to_equal("test@example.com".to_string());
    });
});
```

[テスト](testing.md)を参照してください。

## ミドルウェア

### `global_middleware!`

あらゆるリクエストで、ルート固有のミドルウェアより前に、登録順で実行されるミドルウェアを登録します。型ごとにべき等です。

```rust
use suprnova::global_middleware;
use crate::middleware;

pub fn register() {
    global_middleware!(middleware::LoggingMiddleware);
    global_middleware!(middleware::CorsMiddleware);
}
```

`Server::from_config` / `Server::new` より前に実行する必要があります - サーバーは、構築時にグローバルレジストリのスナップショットを取ります。[ミドルウェア](middleware.md)を参照してください。

## 落とし穴

すぐに引っかかりやすく、すぐに直せる失敗のパターンを、いくつか挙げます。

### アトリビュートの順序 - `#[observer]` は `#[async_trait]` より前に置く

```rust
// 正しい書き方
#[suprnova::observer(User)]
#[async_trait]
impl Observer<User> for AuditObserver { … }

// 誤った書き方 - 静かにリスナーを1つも生成しません
#[async_trait]
#[suprnova::observer(User)]
impl Observer<User> for AuditObserver { … }
```

アトリビュートマクロは、外側から内側へと展開されます。`async_trait` は、あらゆる `async fn` を、脱糖された `Pin<Box<dyn Future>>` の形へと書き換えます。これが先に実行されると、observerマクロはもはやメソッド名でマッチさせることができず、何も生成しません。同じ外側から内側への規則は、複数のマクロを重ねるときには常に当てはまります - 迷ったら、Suprnovaのアトリビュートを最も外側に置いてください。

### 固有implの罠

固有 `impl` のメソッドは、トレイトディスパッチを通じて、トレイトのデフォルトメソッドを覆い隠すことが**できません**。あるモデルに `fn save(&self)` を固有メソッドとして定義するマクロ（あるいは手書きのコード）を書いた場合、`Model` トレイトを経由する呼び出し（呼び出し側がそれを `&dyn Model` としてしか知らない `some_model.save()`）は、あなたの固有のオーバーライドではなく、トレイトのデフォルトを選びます。

対処法: 生成される振る舞いがトレイトディスパッチに参加する必要がある場合は、固有メソッドではなく、必ずトレイトメソッドのオーバーライドを生成してください。フレームワークのマクロ（特に `#[suprnova::model]`）がトレイトのimplに書き込むのは、これが理由です。Eloquentの拡張を手作りする場合も、同じようにしてください。

### `global_middleware!` は `Server::from_config` より前でのみ効果を持つ

サーバーは、構築される際にグローバルレジストリのスナップショットを取ります。`Server::from_config(...)` の後に `global_middleware!(M)` を呼び出しても、そのサーバーに遡って適用されることはありません。あらゆるグローバルミドルウェアは、`Application::run()` がserveのステップに到達する前に、`bootstrap()` の中で登録してください。

### `redirect!` と `inertia_response!` はビルド時のチェックです

どちらのマクロも、指定されたターゲットが存在しない場合はコンパイルを拒否します - それこそが狙いです。リファクタリングによってルートやコンポーネント名が削除されると、それに言及しているすべての呼び出し箇所でビルドが壊れます。これはまさに望ましい挙動です。ビルドエラーに驚いたときは、マクロの呼び出しを「修正」する前に、`routes!` ブロックやページのディレクトリの中で、その文字列リテラルを検索してください。

### `?:` は `None` でスキップし、`?=>` は `None` でも実行される

`validate!` の行において、`?:` はフィールドが `Some` のときにのみルールを実行します。そのため、`?:` の行にある `RequiredIf` のような、存在を条件とするルールは、値が存在しないフィールドを決して失敗させることができません。「Xのときに必須」というケースには、（値が存在しない場合を `""` として扱う）`?=>` を使ってください。

### `#[derive(Validate)]` はSuprnovaではなく `validator` クレートに由来する

Suprnovaは `validator::Validate` を再エクスポートしているため、`validator` に直接依存する必要はありません。`#[validate(...)]` のアトリビュートは `validator` に由来します。Suprnova自身の `validate!` マクロは、実行時における複数フィールドにまたがる/コンテキストに応じたエントリーポイントです - この2つは互いを補完し合いますが、異なる名前空間に属しています。

## Suprnovaが異なる設計を選んだ理由

Laravelは、ルート、コマンド、メールテンプレート、モデルクラス、ファクトリー、オブザーバー、ポリシーを、実行時に - リフレクション、ファイルシステムのスキャン、文字列ベースのディスパッチを通じて発見します。PHPはこれを安価にしており（オートロードとopcacheがそのコストを償却します）、開発体験は優れています。正しいディレクトリにファイルを置けば、それが現れます。

このモデルは、Rustには合いません。私たちには、トレイト実装に対する実行時のリフレクションがなく、ランタイムは単一の静的リンクバイナリであり、起動時のファイルシステムスキャンは、1つのバイナリが数百万のリクエストをさばくプロセスモデルには、より不向きです。

そこでSuprnovaは、同じ仕事をコンパイル時に行います。ルートは検証され、コンポーネント名はページディレクトリと突き合わせてチェックされ、メールテンプレートは `include_str!` を介して埋め込まれ、ルート名はインベントリを通じて一意性がチェックされ、モデルは自分自身をインベントリに登録してフレームワークが起動時にそれを流し込み、コマンドも同様です。開発体験はよく似ています - ファイルを置き、`#[command]` や `#[suprnova::model]` を追加し、バイナリを実行します - ただし配線が行われるのは、最初のリクエストの時点ではなく、`main` の前です。

その代償として、綴りの誤り、コンポーネントの欠落、壊れた参照は、実行時エラーではなくビルドエラーになり、リクエストごとのリフレクションのコストはゼロになります。

## 次のステップ

- [ルーティング](routing.md) - `routes!` の完全な展開、名前付け、モデルバインディング
- [コントローラー](controllers.md) - `#[handler]` と `#[request]` を組み合わせる
- [Eloquent](eloquent.md) - `#[suprnova::model]` とその仲間たちを、実際の文脈の中で
- [バリデーション](validation.md) - `validate!`、コンテキストに応じたルール、非同期ルール
- [コンソール](console.md) - `#[command]` と `#[derive(Command)]` を一通り
- [テスト](testing.md) - `#[suprnova_test]`、`expect!`、フェイク
