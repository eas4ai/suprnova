# 認証

Suprnovaは、Laravelの形をした認証システムを出荷します: 静的な `Auth` ファサード、`AuthManager` を通じて解決される名前付きの認証ガード、差し替え可能なユーザープロバイダー、あなたのUserモデルに対する `Authenticatable` トレイト、そしてルートをゲートするミドルウェアです。スキャフォルドされたプロジェクトは、あなたの型付きの `User` に対してすでに配線されたセッションの認証ガード（`web`）とトークンの認証ガード（`api`）を持って起動するため、ログイン、登録、保護されたルートは、`suprnova new` を実行した日から機能します。

## 構成要素

| 型 | 役割 |
|---|---|
| `Auth` | 認証ガードに加え、Magnetarに支えられたパスワード、マジックリンク、パスキー、OAuth操作のためのフレームワークファサード |
| `MagnetarConfig` / `init_magnetar` | デフォルトのパスワード、セッション、ロックアウト、パスキー、および要素エンジンを合成して原子的にインストールする |
| `Authenticatable` | アプリケーションモデルが実装するトレイト。`get_auth_identifier() -> String` とパスワードハッシュを表面化する |
| `UserProvider` | アプリケーションユーザーを取得するトレイト。`EloquentUserProvider<M>` と `DatabaseUserProvider` が標準で出荷される |
| `AuthManager` | `AuthConfig` と登録済みプロバイダーを保持し、要求に応じて名前付き認証ガードを解決する |
| `SessionGuard` / `TokenGuard` | フレームワークのステートフルおよびステートレスな認証ガード契約 |
| `BearerTokenMiddleware` | Magnetarのベアラーセッションをフレームワークのリクエスト認証状態へ解決する |
| `AuthMiddleware` / `GuestMiddleware` / `BasicAuthMiddleware` | ルートを保護するミドルウェア |
| `Credentials` | JSON形の認証情報マップ。典型的には `{ "email", "password" }` |

フレームワークのガード/プロバイダーコードは `framework/src/auth/` に存在します。Magnetarホストアダプターとファサードは `framework/src/magnetar_integration/` に、エンジンクレートは `crates/suprnova-magnetar/` に存在します。より高レベルのメール確認、パスワードリセット、ロックアウト、およびTOTPフローは `framework/src/auth_flows/` にあり、[認証フロー](auth-flows.md)で扱います。OAuth、Apple、マジックリンクログインは[OAuthとパスワードレスログイン](oauth.md)で扱います。

## 識別子モデル

認証済みユーザーのidは、Suprnovaを端から端まで `String` として流れます - セッションストレージ、[`UserProvider::retrieve_by_id`]、remember-meのテーブル、すべての認証イベントです。正式な表面は `Authenticatable::get_auth_identifier() -> String`（Laravelの `getAuthIdentifier`）です。数値の主キーは自明に文字列化されます。UUID、ULID、そして不透明なOAuthプロバイダーのidは、変更なくそのまま流れます。

```rust
use std::any::Any;
use suprnova::Authenticatable;

impl Authenticatable for User {
    fn get_auth_identifier(&self) -> String {
        self.id.to_string()
    }

    fn get_auth_password(&self) -> Option<&str> {
        Some(&self.password)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
```

`get_auth_password` は、組み込みのプロバイダーが `hashing::verify_async` を介して平文のパスワードを照合する対象です。OAuth、パスキー、マジックリンクなど、別の手段で認証するユーザーには `None` を返してください。`auth_identifier_name() -> &'static str` メソッド（デフォルトは `"id"`）は、idが存在するカラムに名前を付けます。便利メソッドの `auth_identifier() -> i64` は、文字列のidをデフォルトでパースし、数値でないidに対しては `0` にフォールバックします - Suprnova自身は決してこれを呼び出しません。パースを省略したい整数キーのモデルに対してのみ、これをオーバーライドしてください。

### Suprnovaが異なる設計を選んだ理由

Laravelの `getAuthIdentifier()` は `mixed` を返します。PHPは、idがint、UUIDの文字列、あるいはレガシーなテーブルからの文字列型の主キーであるかを気にしません。Rustは、セッション、プロバイダー、イベントのすべてが合意する単一の具体的な型を必要とします。`String` は、フレームワークにあなたのアプリがどのidの形を使っているかを知らせることなく、あらゆるidの形に対応できる唯一の選択です。`auth_identifier()` という整数の便利メソッドは、あなたのカラムが `BIGINT` であるという一般的なケースのために存在しますが、フレームワークはそれに決して依存しません - 明日あなたの `User` をULIDに切り替えても、認証のスタックの中で何も気づきません。

## 起動時に認証を配線する

`config/auth.php` に相当するRustのものは、コンテナに `AuthManager` のシングルトンとして登録される `AuthConfig` と、名前の下に登録される `UserProvider` です。`bootstrap.rs` は、通常この両方を2行で行います:

```rust
use std::sync::Arc;
use suprnova::{App, Auth, AuthConfig, AuthManager, EloquentUserProvider};

use crate::models::user::User;

pub async fn bootstrap() -> Result<(), suprnova::FrameworkError> {
    // … DB::init、SessionMiddlewareのインストールなど。

    App::singleton(AuthManager::new(AuthConfig::from_env()));
    Auth::register_provider("users", Arc::new(EloquentUserProvider::<User>::new()))
        .expect("register users provider");

    Ok(())
}
```

`AuthConfig::from_env()` は、デフォルトの認証ガードを `AUTH_GUARD`（デフォルトは `"web"`）から読み取り、標準で2つの名前付き認証ガードを出荷します: `web` のセッション認証ガードと、`api` のトークン認証ガードで、どちらも `"users"` プロバイダーに支えられています。より多くの認証ガードを必要とするアプリ（別個の `admins` プロバイダー、ステートフルとステートレスを分けた認証ガードなど）は、設定を明示的に組み立てます:

```rust
use suprnova::{AuthConfig, GuardConfig};

let config = AuthConfig::new("web")
    .guard("web", GuardConfig::session("users"))
    .guard("admin", GuardConfig::session("admins"))
    .guard("api", GuardConfig::token("users"));
```

## Magnetarエンジンを初期化する

APIスターターは、データベースと `APP_KEY` の準備後にMagnetarを初期化します:

```rust
use suprnova::{DB, MagnetarConfig, PasskeyConfig, init_magnetar};

pub async fn register_auth() -> Result<(), suprnova::FrameworkError> {
    let database = DB::connection()?;
    let magnetar = MagnetarConfig::from_sea_orm(database.inner().clone())
        .passkey_config(PasskeyConfig {
            rp_id: "app.example.com".to_string(),
            rp_origin: "https://app.example.com".to_string(),
        });

    init_magnetar(magnetar).await
}
```

デフォルトエンジンはアプリケーションのSeaORM接続を共有し、`.apply_migrations(false)` を選択しない限りそのスキーマを作成します。パスワード/セッションおよびパスキーアダプターを原子的にインストールします。再初期化は、別のリクエストが古いストアを使い続けている間に一方のアダプターを置換するのではなく、エラーを返します。

`MagnetarConfig` は、セッション、ロックアウト、および二要素ポリシー値も受け入れます:

```rust,ignore
let magnetar = MagnetarConfig::from_sea_orm(database)
    .session_config(session_policy)
    .lockout_config(lockout_policy)
    .two_factor_config(factor_policy)
    .passkey_config(passkey_policy);
```

デフォルトのホスト束縛は、`i64` のアプリケーションIDを持つ正規の `app_users` テーブルを使用します。Magnetarの公開 `UserId` はファサード境界では不透明のままであり、デフォルト束縛はアプリケーションテーブルへ渡る箇所でのみ保存済み識別子をパースします。

### Magnetarに支えられたファサードメソッド

インストール済みエンジンは、次のフレームワーク所有メソッドを動かします:

- `Auth::password().register(...)`。
- `Auth::password().authenticate(...)`。
- `Auth::magic_link().send(...)` と `.consume(...)`。
- `Auth::passkey().begin_registration(...)` と `.finish_registration(...)`。
- `Auth::passkey().begin_authentication(...)` と `.finish_authentication(...)`。
- OAuthデリゲートがインストールされている場合の `Auth::oauth(provider)`。
- Remember-meの発行、ローテーション、および失効。
- `BearerTokenMiddleware` によるベアラーセッションのルックアップ。
- `suprnova::magnetar_integration` の `list_sessions`、`revoke_session`、および `revoke_all_sessions`。

サインイン成功時には、フレームワークのセッションIDとCSRFトークンをローテーションし、アプリケーションユーザーIDを保存し、不透明なMagnetarウェブ束縛を記録します。フレームワークは引き続きHTTPミドルウェア、クッキー、メール、イベント、およびガード/プロバイダー契約を所有します。

### パスワード認証

統合されたクレデンシャル、ロックアウト、要素ゲート、およびセッションパスをアプリケーションが望む場合は、Magnetarのパスワードファサードを使用します:

```rust,ignore
let user = Auth::password()
    .register("alice@example.com", password)
    .await?;

let (user, session) = Auth::password()
    .authenticate(
        "alice@example.com",
        password,
        request.header("User-Agent").map(str::to_string),
        request.peer_ip().map(str::to_string),
    )
    .await?;
```

`authenticate` は、無効なクレデンシャル、ロックアウト、または必須の第二要素に対してHTTP 401エラーを返します。ストレージとエンジンの失敗はサーバーエラーのままです。このメソッドがパスワード材料を返すことはありません。

### パスキー

使い捨てセレモニーセレクターはフレームワークセッションに保存されるため、パスキーの開始および完了呼び出しには `SessionMiddleware` が必要です:

```rust,ignore
let challenge = Auth::passkey()
    .begin_authentication("alice@example.com")
    .await?;

let (user, session) = Auth::passkey()
    .finish_authentication("alice@example.com", browser_credential)
    .await?;
```

登録では対応する `begin_registration` と `finish_registration` の組を使用します。既存アカウントへの登録には、確認済みのリクエストアクターとプラグインパスを通じた最近の再認証が必要です。レガシーセッション内の裸のユーザーIDがクレデンシャルアクターへ昇格されることはありません。

### 初回メール証明と認証エポック

Magnetarは、未確認アカウントで最初に成功したメールボックス証明を原子的なクレデンシャル境界として扱います。パスワードリセット、マジックリンクの消費、およびOAuthの確認済みメール完了がこの境界を獲得できます。

このトランザクションはアカウントの認証エポックを進め、古いセッションとrememberクレデンシャルを失効させ、メールボックス所有者が到着する前に占有者が登録できた暫定クレデンシャルを削除します。パスワード、パスキー、リンク済みアカウント、および二要素の書き込みはアクタースナップショットを持ち、操作中にアカウントエポックが変化すると失敗します。

すでに確認済みのアカウントでは、パスワードリセットは正当なパスキー、リンク済みアカウント、および二要素登録を保持しつつ、パスワードをローテーションしてセッションを無効化します。OAuthは、未確認の既存アカウントをプロバイダーのメールだけで自動リンクしません。ホストポリシーに従って確認済みメール完了または明示的なリンクを要求します。

### Magnetarクレートを直接使う表面

ほとんどのアプリケーションはフレームワークファサードに留まります。カスタムアイデンティティホストを構築するアプリケーションは、次のために `suprnova-magnetar` へ直接依存できます:

- フレームワーク中立のプラグインルートと効果ハンドラー。
- パスワードおよびパスワード管理プラグイン。
- パスキーおよび二要素エンジン。
- OAuth認可、グラント、プロバイダープラグイン、デバイス認可、およびトークンブローカーサービス。
- 不透明、JWT、remember、およびグラントのセッションエンジン。
- カスタムストレージ束縛とデフォルトSeaORMスキーマ。
- 形状を認識する認証データマイグレーション。

直接使用しても、HTTPまたはアプリケーションユーザーの所有権がMagnetarへ移るわけではありません。ホストは、ワイヤーリクエスト、メール効果、アプリケーションID、レート制限ドライバー、およびセッション束縛を引き続き自身のフレームワークへ対応付けます。

## `Auth` ファサード

静的な `Auth` ファサードは、コントローラーやミドルウェアから呼び出す、Laravelの形をした表面です。認証情報とユーザーに基づくメソッドは、**デフォルトの認証ガード**（`AuthConfig::default_guard` が指す先、デフォルトは `"web"`）へ委譲します。同期の `check`/`guest`/`id` の読み取りは、セッションに支えられた高速パスであり、マネージャーを必要としません。

```rust
use suprnova::{Auth, Credentials};

// 認証情報を検証し、ユーザーをログインさせます。Attempting → (Login +
// Authenticated) を発火し、remember-meを尊重します。解決されたユーザー、
// あるいは認証情報が不正な場合はNoneを返します。
if let Some(user) = Auth::attempt(&Credentials::password(&email, &password), remember).await? {
    println!("Welcome, user {}", user.get_auth_identifier());
}

// 既知のユーザーを直接ログインさせます。
Auth::login(user, remember).await?;

// 認証情報を再チェックせずにidでログインします（例えば、登録が完了した直後など）。
Auth::login_using_id(&id, remember).await?;

// セッションを永続化せずに認証情報を検証します（パスワード確認ダイアログ向け）。
let ok: bool = Auth::validate(&Credentials::password(&email, &password)).await?;

// このリクエストだけ認証します - セッションへの書き込みはありません。Laravelの `once` です。
let ok: bool = Auth::once(&Credentials::password(&email, &password)).await?;
Auth::once_using_id(&id).await?;

// セッションに支えられた高速パス（AuthManagerは不要）。
if Auth::check()    { /* 認証済み */ }
if Auth::guest()    { /* 未認証 */ }
if let Some(id) = Auth::id() { /* 文字列のid */ }

// 現在のユーザーが、このリクエストでremember-meクッキーによって認証
// されたかどうか。Laravelの `viaRemember()` です。
if Auth::via_remember() { /* … */ }

// 現在のユーザーを解決します（登録済みのプロバイダーを介して）。
if let Some(user) = Auth::user().await? {
    println!("user id: {}", user.get_auth_identifier());
}
if let Some(user) = Auth::user_as::<User>().await? {
    println!("Welcome, {}!", user.name);
}

// 認証を解体し、remember-meを失効させ、CSRFをローテーションし、Logoutを発火します。
Auth::logout().await?;

// セッションを完全に破棄します（idの再生成 + 消去 + remember-meの失効 + Logoutの発火）。
Auth::logout_and_invalidate().await?;
```

`Auth::attempt` は、成功時に素の `bool` ではなく解決されたユーザーを返します - LaravelのAPIより豊かであり、後続の `Auth::user()` の呼び出しを省けます。`Ok(None)` は、認証情報がユーザーを解決しなかったことを意味します。`Err` は、伝播させる必要のあるデータベース / ハッシング / 設定の失敗を意味します。

すでに自分でユーザーのアイデンティティを検証済みで、セッションを確立することだけを望む場合 - 例えばOAuthのコールバックが完了した後など - 同期のプリミティブに手を伸ばしてください:

```rust
// Sync、プロバイダーなし、AuthManagerなし、イベントなし。リクエストスコープの
// 外側から呼ばれた場合（SessionMiddlewareがインストールされていない場合）は
// Errを返すため、サイレントに失敗したログインが成功に見えることは決してありません。
Auth::login_id(user.id.to_string())?;
```

`login_id` はセッションidを再生成し（セッション固定化を防ぎます）、CSRFトークンをローテーションし、それからidをセッションへ書き込みます。これは意図的に、はっきりと失敗するようになっています: 以前のバージョンは、セッションスコープの外側でサイレントに何もしませんでした。監査でそれを修正しました - 一度も届かなかった「ログイン成功」は、他の何も捕まえられない類のバグです。

## `Auth::user()` と `user_as<T>`

`Auth::user()` は、トレイトの背後にあるユーザーを返します:

```rust
if let Some(user) = Auth::user().await? {
    println!("user id: {}", user.get_auth_identifier());
}
```

そのトレイトオブジェクトは、`Authenticatable` を実装する誰でもをカバーします。あなたの具体的な `User` を取り戻すには、`user_as::<T>()` を通じてダウンキャストしてください:

```rust
use suprnova::Auth;
use crate::models::user::User;

if let Some(user) = Auth::user_as::<User>().await? {
    // モデルへの直接のフィールドアクセスです。
    println!("Welcome, {}!", user.name);
}
```

`user_as` は、ユーザーが認証されていない場合*と*、解決されたユーザーが `T` でない場合（例えば、スタックの別の場所で行われた異なる型の `Auth::set_user(...)`）の両方で `Ok(None)` を返します。リクエストの内部では、ユーザーはリクエストごとにキャッシュされるため、`Auth::user()` を繰り返し呼び出しても、プロバイダーに当たるのは1回だけです。

## 名前付き認証ガード

素の `Auth::*` メソッドは、デフォルトの認証ガードに話しかけます。特定の認証ガードに対して操作するには、名前でそれを解決してください:

```rust
use suprnova::Auth;

// 読み取り専用の操作は、どのドライバーでも機能します。
if Auth::guard("api")?.check().await? { /* … */ }

// Login/logout/attemptはステートフルな認証ガードを必要とします。トークンの認証ガードは、ここでははっきりと失敗します。
let user = Auth::stateful_guard("web")?
    .attempt(&credentials, false)
    .await?;
```

`Auth::guard("name")` は `Arc<dyn Guard>`（読み取り専用の契約）を返し、`Auth::stateful_guard("name")` は `Arc<dyn StatefulGuard>`（`attempt`/`login`/`logout` を追加したもの）を返します。トークンの認証ガードに対してステートフルな契約を求めると、APIをサイレントに制限するのではなく、対処方法を示すメッセージを伴うエラーが返ります。

## ユーザープロバイダー

`UserProvider` は、認証のスタックに対して、ユーザーをどのように取得し検証するかを伝えます。2つのプロバイダーが標準で出荷されるため、よくあるケースではカスタムの実装は不要です:

- **`EloquentUserProvider<M>`** - `Authenticatable` でもある型付きの `#[suprnova::model]` の `User` を通じて解決します。idについては主キーで、認証情報については（デフォルトで）`email` でルックアップします。
- **`DatabaseUserProvider`** - 生のテーブルを名前で `GenericUser`（id + 属性マップ）へ解決します。型付きのモデルを持っていない、あるいは望まないときに使ってください。

どちらも、認証情報のルックアップを許可リスト（デフォルトは `["email"]`）に対してフィルタリングします - 悪意のある認証情報マップが、余分な `WHERE` 述語を注入することはできません。許可リストは `.credential_columns([...])` で、ルックアップのカラムは `.identifier_column("uuid")` で、idの束縛戦略は `.with_id_parser(...)` でカスタマイズできます。

カスタムのソース（LDAP、外部API）を差し込むには、`UserProvider` を直接実装してください。`retrieve_by_id` は、識別子を `&str` として受け取ります:

```rust
use async_trait::async_trait;
use std::sync::Arc;
use suprnova::{Authenticatable, FrameworkError, UserProvider};

struct LdapProvider;

#[async_trait]
impl UserProvider for LdapProvider {
    async fn retrieve_by_id(
        &self,
        id: &str,
    ) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
        // … LDAPから取得し、Arc<dyn Authenticatable> として返します
        Ok(None)
    }

    // retrieve_by_credentials + validate_credentials には、None / falseを
    // 返すトレイトのデフォルトがあります。あなたのソースに対して `Auth::attempt`
    // と `Auth::validate` をサポートするには、これらをオーバーライドしてください。
}
```

マネージャーにそれを登録してください:

```rust
Auth::register_provider("ldap", Arc::new(LdapProvider))?;
```

## ルートを保護する

### `AuthMiddleware`

認証済みのみのルートをゲートします。未認証のリクエストはログインページへリダイレクトされるか、`401` を受け取ります:

```rust
use suprnova::{AuthMiddleware, Router};

pub fn routes() -> Router {
    Router::new()
        .get("/dashboard", controllers::dashboard::index)
        .post("/logout", controllers::auth::logout)
        .middleware(AuthMiddleware::redirect_to("/login"))
}
```

`AuthMiddleware::new()` は代わりに `401 Unauthorized` を返します - JSON APIに最適です。`AuthMiddleware::redirect_to("/login")` は、通常のリクエストには `302` を、Inertiaのリクエストには `409 X-Inertia-Location` を発行します（Inertiaのクライアントはこれをフルページの遷移に変換します）。特定の認証ガードに対してゲートするには、`for_guard` をチェーンしてください:

```rust
// apiの認証ガードが認証されていない限り401。
.middleware(AuthMiddleware::new().for_guard("api"))
```

トークンの認証ガード（`for_guard("api")`）は、チェーンのより前で実行される何らかのベアラートークンミドルウェアが、リクエストの認証idを埋めることに依存しています。それがなければ、その認証ガードは常に未認証だと報告します。

### `GuestMiddleware`

その逆です - 認証済みのユーザーが目にすべきではない、ログインページや登録ページのためのものです:

```rust
use suprnova::{GuestMiddleware, Router};

pub fn routes() -> Router {
    Router::new()
        .get("/login", controllers::auth::show_login)
        .post("/login", controllers::auth::login)
        .get("/register", controllers::auth::show_register)
        .post("/register", controllers::auth::register)
        .middleware(GuestMiddleware::redirect_to("/dashboard"))
}
```

`GuestMiddleware::for_guard("name")` は `AuthMiddleware::for_guard` と同じように機能します。

### `BasicAuthMiddleware`

`Authorization: Basic` ヘッダーからの、認証ガードのプロバイダーに対するHTTP Basic認証です:

```rust
use suprnova::BasicAuthMiddleware;

// ステートフル - 成功時にユーザーをセッションへログインさせます（Laravelの `basic`）。
.middleware(BasicAuthMiddleware::new())

// ステートレス - このリクエストだけ認証します（Laravelの `onceBasic`）。
.middleware(BasicAuthMiddleware::once())
```

デコードされたユーザー名は、`field` の認証情報（デフォルトは `"email"`）と照合されます。ヘッダーが欠けている、形式が不正、あるいは無効な場合は、`WWW-Authenticate: Basic realm="..."` というチャレンジを伴う `401` を返します。`.field(...)`、`.realm(...)`、`.for_guard(...)` で設定してください。

## ライフサイクルイベント

認証ガードは、5つのライフサイクルイベントをディスパッチします。[`EventFacade`](events.md)を介して、それらをリスンしてください:

| イベント | いつ |
|---|---|
| `Attempting` | 認証情報の試行が始まったとき（`attempt`/`once`） |
| `Authenticated` | このリクエストでユーザーが能動的に認証されたとき（`login`/`once`/`once_using_id`） |
| `Login` | ユーザーがセッションに永続化されたとき（`login` / 成功した `attempt`） |
| `Logout` | ユーザーがログアウトしたとき |
| `Failed` | 認証情報の試行が失敗したとき（間違ったパスワードまたは未知のid） |

あらゆるイベントは、認証ガード名と文字列のユーザーidを運びます - 平文のパスワードや、生の認証情報マップは決して運びません。`Authenticated` は、ユーザーが能動的に確立されたときにのみ発火し、既存のセッションからの受動的な `Auth::user()` の解決では発火しません。そのため、リスナーは、認証済みのリクエストのたびに重複のストリームを受け取ることはありません。

## スキャフォルドされたログインフロー

`suprnova new` は、登録済みのプロバイダーに対して `Auth::attempt` を使う認証コントローラーを生成します。`FormRequest` と `Validate` は `{ message, errors }` のバリデーションエンベロープを生成します。Inertiaリクエストでは、インストール済みのバリデーションリダイレクトミドルウェアがその失敗をHTTP `303 See Other` の元のページへのリダイレクトに変換し、エラーをフラッシュします。InertiaでないクライアントはHTTP `422 Unprocessable Entity` のJSONエンベロープを受け取ります:

```rust
use serde::Deserialize;
use suprnova::{
    handler, inertia_response, redirect, serde_json, Auth, Credentials,
    FormRequest, InertiaProps, Request, Response, Validate, ValidationErrors,
};

#[derive(InertiaProps)]
pub struct LoginProps {
    pub errors: Option<serde_json::Value>,
}

#[handler]
pub async fn show_login(req: Request) -> Response {
    inertia_response!(&req, "auth/Login", LoginProps { errors: None })
}

#[derive(Deserialize, Validate)]
pub struct LoginRequest {
    #[validate(email(message = "Please enter a valid email address"))]
    pub email: String,
    #[validate(length(min = 1, message = "Password is required"))]
    pub password: String,
    #[serde(default)]
    pub remember: bool,
}

impl FormRequest for LoginRequest {}

fn invalid_credentials() -> suprnova::FrameworkError {
    let mut errs = ValidationErrors::new();
    errs.add("email", "These credentials do not match our records.");
    suprnova::FrameworkError::Validation(errs)
}

#[handler]
pub async fn login(form: LoginRequest) -> Response {
    match Auth::attempt(
        &Credentials::password(&form.email, &form.password),
        form.remember,
    )
    .await?
    {
        Some(_user) => redirect!("/dashboard").into(),
        None => Err(invalid_credentials().into()),
    }
}

#[handler]
pub async fn logout(_req: Request) -> Response {
    Auth::logout().await?;
    redirect!("/").into()
}
```

登録も同じ形に従います: フォームを検証し、ユーザーを作成し、それから `Auth::login(Arc::new(user), false).await?` が、作られたばかりのユーザーをセッションへログインさせ、`Login` イベントを発火します。

## スキャフォルドされた `User` モデル

生成された `User` は `Authenticatable` を実装する `#[suprnova::model]` です。また `email_verified_at: Option<DateTime<Utc>>` を含み、`MustVerifyEmail` と `CanResetPassword` を実装します。これらの橋渡しにより、`EloquentUserProvider<User>` はメール確認を記録し、パスワードリセットのアイデンティティデータを提供できます。以下の抜粋はガードログインのフィールドとヘルパーだけを示します。完全な認証フローの実装には生成済みモデルテンプレートを使ってください。パスワードヘルパーは [`hashing`](hashing.md) モジュールを使用します:

```rust
use chrono::{DateTime, Utc};
use suprnova::{attrs, hashing, model, Authenticatable, FrameworkError};

#[model(
    table = "users",
    fillable = ["name", "email", "password"],
    hidden = ["password", "remember_token"],
    timestamps,
)]
pub struct User {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub password: String,
    pub remember_token: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl User {
    pub async fn find_by_email(email: &str) -> Result<Option<Self>, FrameworkError> {
        <Self as suprnova::eloquent::Model>::query()
            .filter("email", email)
            .first()
            .await
    }

    pub fn verify_password(&self, password: &str) -> Result<bool, FrameworkError> {
        hashing::verify(password, &self.password)
    }

    pub async fn create(
        name: impl Into<String>,
        email: impl Into<String>,
        password: &str,
    ) -> Result<Self, FrameworkError> {
        let hashed = hashing::hash(password)?;
        <Self as suprnova::eloquent::Model>::create(attrs! {
            name: name.into(),
            email: email.into(),
            password: hashed,
        })
        .await
    }
}
```

`hidden = ["password", "remember_token"]` という属性は、モデルがJSONへシリアライズしてレスポンスとして送る際に、それらのカラムをスキップさせます - それらは構造体の上には存在しますが、Inertiaのレスポンスを通じて漏れることは決してありません。

## Remember-me

Magnetarエンジンがインストールされている場合、`Auth::attempt(credentials, true)` と `Auth::issue_remember_cookie` は目的束縛されたMagnetarのrememberクレデンシャルを発行します。ブラウザーは引き続きフレームワークの暗号化された `remember_me` クッキーを受け取りますが、検証子ストレージ、認証エポックの確認、使い捨てローテーション、異常処理、および失効はMagnetarが所有します。

有効なフレームワークログインのないリクエストでは、`SessionMiddleware` はインストール済みエンジンを通じてクッキーを消費し、rememberクレデンシャルをローテーションし、新しいMagnetarセッションを発行し、両方のセッション層を束縛します。古い認証エポック、失効済みアカウントセッション、不正なクレデンシャル、またはリプレイでリクエストが認証されることはありません。

`Auth::revoke_remember_tokens()` は、現在のユーザーのすべてのrememberクレデンシャルを無効化します。ストレージ操作が失敗した場合にもブラウザーがクレデンシャルを破棄するよう、バックエンド失効より前に消去クッキーをキューへ入れます。

Magnetarエンジンがインストールされていない場合、フレームワークは互換性のためにレガシーな `remember_tokens` フォールバックを維持します。新しいアプリケーションは、そのフォールバックに依存せずMagnetarを初期化すべきです。

## セキュリティの保証

認証のスタックが確立する不変条件の、短いリストです:

- **`Auth::login_id` は、リクエストスコープの外側では、はっきりと失敗します。** 以前のバージョンは、セッションへの書き込みをサイレントに落としていました。一度も届かなかった「ログイン成功」は、他の何も捕まえられない類のバグです。
- **セッションidとCSRFトークンは、ログインのたびに再生成されます。** `login_id` と、認証ガードに支えられた `login`/`attempt` のどちらも、セッション固定化を防ぐためにそれらをローテーションします。
- **ログアウトは、remember-meを失効させる前に認証状態をクリアします。** DBでの失効が失敗しても、セッションはすでにログアウト済みの状態にあるため、古い認証のスロットが、部分的なログアウトを生き延びることはありません。remember-meを消去するクッキーは、DBの削除より*前に*キューへ入れられるため、行の削除が失敗した場合でも、ブラウザはそのクッキーを落とします（後で刈り取りの一掃が片付けます）。
- **認証情報の許可リストが、インジェクションを阻止します。** 組み込みのプロバイダーはどちらも、`retrieve_by_credentials` を `credential_columns` に対してフィルタリングするため、攻撃者に影響された認証情報マップの中の余分なキーが、余分な `WHERE` 述語になることはできません。
- **クレデンシャル書き込みはアクターで柵囲いされます。** パスワード、パスキー、リンク済みアカウント、二要素、セッション、およびrememberの変更は、確認済み認証によって確立されたユーザーIDと認証エポックを伴います。失効または初回証明によるエポック変更は、進行中の古い書き込みを失敗させます。
- **最初のメールボックス証明は原子的です。** 未確認アカウントで、パスワードリセット、マジックリンク消費、またはOAuthの確認済みメール完了は、同じトランザクションで認証エポックを進め、暫定クレデンシャルを削除します。並行する占有者の書き込みは、コミット後にアクセスを復元できません。
- **メール確認はアクターに束縛されます。** フレームワーク確認ファサードは、IDがトークン所有者と一致する認証済みユーザーを必要とします。他のアカウントのトークンは消費せずに拒否されます。
- **OAuthのメールはアカウント所有権ではありません。** 未確認の既存アカウントがプロバイダーのメールだけで自動リンクされることはありません。確認済みアカウントには明示的なリンクが、未確認アカウントには初回メール証明完了パスが必要です。
- **認証イベントは、平文を決して運びません。** 認証ガード名 + 文字列のユーザーid、それ以外は何もありません。失敗した試行の追跡（emailをキーにしたロックアウト）は、ライフサイクルイベントではなく、[認証フロー](auth-flows.md)の `BruteForce` に属します。

[セッション](session.md)の章は、セッションに支えられた認証ガードが引き継ぐクッキーの設定（`SESSION_LIFETIME`、`SESSION_COOKIE`、`SESSION_SECURE`、`SESSION_SAME_SITE`、および `SESSION_COOKIE_PREFIX`）を扱っています。

## 次のステップ

- [認証フロー](auth-flows.md) - メール確認、パスワードリセット、Magnetarに支えられたアカウントロックアウト、フレームワークTOTPの2FA、および認証フローイベント
- [OAuthとパスワードレスログイン](oauth.md) - MagnetarのOAuth、Apple、マジックリンク、プロバイダーポリシー、および認証データ移行
- [認可](authorization.md) - `Gate`、ポリシー、および `Authorizable`
- [セッション](session.md) - ブラウザーセッションとクッキー層
- [CSRF保護](csrf.md) - 状態変更リクエストの保護
- [ハッシング](hashing.md) - bcryptおよびArgon2のヘルパー
