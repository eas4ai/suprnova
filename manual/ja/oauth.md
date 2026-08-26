# OAuth、Apple、マジックリンクによるログイン

Suprnovaは、フレームワーク所有の `Auth` ファサードを通じて、OAuth、Sign in with Apple、パスワードレスのマジックリンクを公開します。このファサードの背後にあるクレデンシャル、セレモニー、アイデンティティ、要素ゲート、セッションの各エンジンはMagnetarが提供します。

公開エントリポイントは次のとおりです:

- OAuthとAppleには `Auth::oauth(provider)`。
- パスワードレスのメールログインには `Auth::magic_link()`。

Suprnovaはこれらのフローのルートをインストールしません。アプリケーションが開始ハンドラとコールバックハンドラを小さく用意し、マジックリンクのメールをどのように配信するかを決めます。

## OAuth で Magnetar を初期化する

パスワード、パスキー、セッション、ロックアウト、二要素認証サービスを初期化する同じ `MagnetarConfig` で OAuth を設定します。プロバイダーレジストリはこれらのサービスと原子的に公開されます。いずれかのサービスを構築できない場合、どれも可視になりません。

```rust,no_run
use std::sync::Arc;

use suprnova::{
    AbuseLimiter, App, AutoLinkPolicy, DB, DatabaseConnection, EndpointOverrides,
    FrameworkAbuseLimiter, GoogleOAuthProvider, GoogleProviderConfig, MagnetarConfig,
    MagnetarOAuthHostConfig, MagnetarOAuthProviderConfig, OAuthAuthorizationConfig,
    OAuthHttpTransport, PasskeyConfig, RateLimiterDriver, ReqwestOAuthTransport,
    RevocationTransport, SecretString, init_magnetar,
};

fn auth_config(
    database: DatabaseConnection,
    transport: Arc<dyn OAuthHttpTransport>,
    revocation: Arc<dyn RevocationTransport>,
    limiter: Arc<dyn AbuseLimiter>,
) -> MagnetarConfig {
    let provider = Arc::new(GoogleOAuthProvider::new(
        GoogleProviderConfig {
            client_id: "google-client".to_owned(),
            client_secret: SecretString::from("google-secret".to_owned()),
            redirect_uri: Some("https://app.example.com/auth/google/callback".to_owned()),
            scopes: vec!["openid".to_owned(), "email".to_owned()],
            endpoints: EndpointOverrides::default(),
        },
        revocation,
    ));
    let oauth = MagnetarOAuthHostConfig::new(
        vec![MagnetarOAuthProviderConfig {
            provider,
            redirect_uri: "https://app.example.com/auth/google/callback".to_owned(),
            scopes: vec!["openid".to_owned(), "email".to_owned()],
        }],
        transport,
        limiter,
        OAuthAuthorizationConfig::default(),
        AutoLinkPolicy::default(),
    )
    .expect("valid OAuth host configuration");

    MagnetarConfig::from_sea_orm(database)
        .passkey_config(PasskeyConfig {
            rp_id: "app.example.com".to_owned(),
            rp_origin: "https://app.example.com".to_owned(),
        })
        .oauth(oauth)
}

pub async fn register_auth() -> Result<(), suprnova::FrameworkError> {
    let database = DB::connection()?;
    let transport = Arc::new(ReqwestOAuthTransport::try_default()?);
    let limiter = Arc::new(FrameworkAbuseLimiter::new(
        App::resolve_make::<dyn RateLimiterDriver>()?,
    ));
    init_magnetar(auth_config(
        database.inner().clone(),
        transport.clone(),
        transport,
        limiter,
    ))
    .await
}
```

このフレームワークは、`OAuthProvider` コントラクト、5 つのファーストパーティプロバイダーと設定型、そしてカスタムプロバイダーの実装に必要なすべての型を再エクスポートします。`ReqwestOAuthTransport` は、本番環境向けのトークン、userinfo、失効処理の I/O を提供します。`FrameworkAbuseLimiter` は、アプリケーションで設定された `RateLimiterDriver` を使用します。アプリケーションには、直接の `suprnova-magnetar` 依存関係も、手書きのトランスポートおよびリミッターアダプターも必要ありません。

`MagnetarConfig` は `apply_migrations` が有効な場合にスキーマを作成します。これはデフォルトです。デプロイが同じスキーマを別途準備する場合にのみ `.apply_migrations(false)` を使用してください。2 回目の初期化は、インストール済みエンジンを置き換えるのではなくエラーを返します。

### 既存のユーザーとセッションの仕組みを維持する

アプリケーションは、Magnetar をパスワード、パスキー、フレームワークセッション、
remember-me 状態の権限元にせず、OAuth セレモニーとプロバイダー証明だけに利用できます。
同じ `MagnetarOAuthHostConfig` を構築し、OAuth 専用の初期化関数でインストールします。

```rust,no_run
use suprnova::{
    MagnetarOAuthOnlyConfig, init_magnetar_oauth_only,
};

let database = DB::connection()?;
init_magnetar_oauth_only(
    MagnetarOAuthOnlyConfig::from_sea_orm(
        database.inner().clone(),
        oauth,
    ),
)
.await?;
```

セレモニーは通常どおり `Auth::oauth(provider).begin()` で開始します。コールバックでは
`verify_oauth_identity(code, state)` を呼び、検証済みのプロバイダー subject を
アプリケーション自身のユーザーテーブルへ対応付け、`Auth::login` で既存の
フレームワークセッションを確立します。このモードでは `complete` を呼び出さないで
ください。`complete` は Magnetar の既定のアカウントおよびセッション対応付けを
適用しますが、OAuth 専用初期化の目的は、それらの判断をアプリケーションに残すことです。

OAuth 専用初期化と完全な既定初期化は代替関係です。2 つ目の初期化関数は、異なる
セッション権限を混在させる代わりに失敗します。

### GitHub プロバイダーの要件

GitHub の REST ユーザーエンドポイントには `User-Agent` が必要です。コミュニティプロバイダーは、必要な任意のメディアタイプの `Accept` 値とともに、`OAuthProvider::userinfo_headers` を通じてそれを追加します。Suprnova は bearer `Authorization` ヘッダーを別途追加し、プロバイダーがそれを上書きしようとする試みを拒否します。

GitHub の `/user` レスポンスにメールアドレスが含まれるのは、ユーザーがそれを公開している場合のみです。検証済みのプライマリアドレスには 2 回目の `/user/emails` リクエストが必要ですが、`resolve_identity` は意図的に I/O を実行せず、1 つの userinfo レスポンスを受け取ります。GitHub プロバイダーは `email: None` を返して Suprnova のメール補完セレモニーを使用するか、`userinfo_endpoint` を `/user` と検証済みのプライマリメールを組み合わせるホストアダプターに向けることができます。未検証または単に公開されているだけのアドレスを、アカウント所有権として扱わないでください。

## セッションのバインディング

OAuthの開始には `SessionMiddleware` が必要です。Magnetarは開始元フレームワークセッションのダイジェストにセレモニーをバインドするため、コールバックを別のブラウザーセッションへ移すことはできません。

パスワード、マジックリンク、パスキー、OAuthによるサインインに成功すると、フレームワークのセッションIDとCSRFトークンがローテーションされ、アプリケーションのユーザーIDが記録され、非透過的なMagnetarのWebバインディングが保存されます。Remember-meのハイドレーションでは、Magnetarのクレデンシャルとフレームワークのセッションバインディングの両方がローテーションされます。

## OAuthフローの開始

プロバイダーの開始ハンドラで `begin` を使います:

```rust,ignore
use suprnova::Auth;

let kickoff = Auth::oauth("google").begin().await?;
// `kickoff.authorization_url` へHTTPリダイレクトを返します。
```

返される `OAuthKickoff` には次が含まれます:

- `authorization_url` - ブラウザーへ送るURL。
- `state` - 開始元セッションにバインドされた単一使用のセレクター。

状態の生成、PKCEポリシー、セレモニーの永続化、プロバイダーとの交換、アイデンティティの検証、濫用制限はMagnetarが所有します。HTTPリダイレクトとコールバックルートはホストコントローラーが所有します。

## コールバックの検証または完了

コールバックには2つのエントリポイントがあります:

| メソッド | 結果 | 副作用 |
|---|---|---|
| `verify_oauth_identity(code, state)` | `OAuthIdentity` | プロバイダーの証明を検証し、アプリケーションセッションを作成せずに、プロバイダー、subject、検証済みメール、表示名を返します。 |
| `complete(code, state)` | `(User, Session)` | インストール済みホストエンジンを通じてアイデンティティを解決し、アカウントリンクポリシーと要素ゲートを適用し、フレームワークセッションをローテーションして、フレームワーク所有のユーザーとMagnetarセッションの値を返します。 |

```rust,ignore
let identity = Auth::oauth("google")
    .verify_oauth_identity(&code, &state)
    .await?;

let (user, session) = Auth::oauth("google")
    .complete(&code, &state)
    .await?;
```

`OAuthIdentity.email` は、プロバイダーが検証済みメールを提供した場合だけ存在します。安定した外部アイデンティティとしてプロバイダーとsubjectを永続化してください。メールは安定したプロバイダー識別子ではありません。

## アカウントリンクポリシー

OAuthの完了では、検証されていないメール文字列を持っていることを、既存のアプリケーションアカウントを所有している証明とはみなしません。

完了結果はセッションを発行する代わりに、追加の作業を要求することがあります:

- **メール完了が必要** - プロバイダーのアイデンティティに、検証済みメールの別セレモニーが必要な場合はHTTP 409を返します。
- **明示的なリンクが必要** - 既存の検証済みアカウントがリンクを認可しなければならない場合はHTTP 409を返します。
- **要素が必要** - アカウントポリシーがセッション発行前の第2要素を要求する場合はHTTP 401を返します。

最初のメール証明境界を勝ち取った検証済みメールの完了は、検証されていない状態で占有されたアカウントをアトミックに取り戻します。トランザクションは認証エポックを進め、仮のクレデンシャルを削除し、古いセッションとrememberクレデンシャルを取り消し、検証済みプロバイダーアカウントを接続します。検証済みアカウントがメールだけで自動リンクされることはありません。

## Sign in with Apple

Appleは同じ `Auth::oauth("apple")` ファサードを使いますが、コールバックでは通常 `response_mode=form_post` を使います。コールバックを `POST` ルートとして登録し、オプションのApple `user` フォームフィールドをApple固有のメソッドへ渡します:

```rust,ignore
let identity = Auth::oauth("apple")
    .verify_apple_identity(&code, &state, form_post_user.clone())
    .await?;

let (user, session) = Auth::oauth("apple")
    .complete_with_apple_form_post(&code, &state, form_post_user)
    .await?;
```

`AppleIdentity` には安定したsubject、オプションの検証済みメール、`email_verified`、`is_private_email` が含まれます。安定したキーとしてsubjectを永続化してください。Appleは最初の認可時にだけ表示名を提供することがあるため、プロバイダーアダプターは最初の `form_post` の値を保持する必要があります。

Appleのトークンとアイデンティティの検証は、インストールされたプロバイダー実装が担います。現在のMagnetarプロバイダーは、IDトークンのデコード済みJSONを信頼するのではなく、署名、issuer、audience、有効期限、nonceを検査します。

## マジックリンクログイン

マジックリンクログインは、インストール済みのMagnetarパスワード/セッションエンジンを使います。フレームワークは単一使用のトークンを平文で返し、メールの作成とURLの形はアプリケーションが所有します:

```rust,ignore
use suprnova::{Auth, Mail};

let token = Auth::magic_link()
    .send("alice@example.com", "https://app.example.com/auth/magic")
    .await?;

let url = format!("https://app.example.com/auth/magic?token={token}");
Mail::to("alice@example.com")
    .send(MagicLinkMail { url })
    .await?;

let (user, session) = Auth::magic_link().consume(&token).await?;
```

`send` はトークン発行前に認証の濫用予算を適用します。`consume` は単一使用であり、要素ゲートを適用し、結果のセッションをフレームワークのリクエストセッションにバインドして、ユーザーとMagnetarセッションを返します。

検証されていない既存アカウントの場合、マジックリンクの消費に成功することが最初のメール証明になります。トランザクションはアカウントを取り戻し、仮のパスワード、パスキー、リンク済みアカウント、二要素、セッション、rememberの状態を削除するため、以前の占有者がアクセスを保持できません。

## 追加するルート

一般的なアプリケーションでは次のルートを追加します:

```rust,ignore
get!("/auth/oauth/{provider}/start", controllers::oauth::start),
get!("/auth/oauth/{provider}/callback", controllers::oauth::callback),
post!("/auth/apple/callback", controllers::oauth::apple_callback),
post!("/auth/magic", controllers::magic_link::send),
get!("/auth/magic/callback", controllers::magic_link::consume),
```

すべてのOAuthおよびパスキーの開始/コールバックルートに `SessionMiddleware` を適用してください。セッションがセレモニーのセレクターを保持し、往復をそれを開始したブラウザーにバインドします。

## 認証の移行

`suprnova-magnetar` クレートには、Torii、Suprnova web、Suprnova API、既存のMagnetarスキーマ向けの形状認識型移行エンジンが含まれます。これはライブラリの表面と例であり、`suprnova` CLIサブコマンドではありません。

`migration` 機能とソースデータベースドライバーを有効にし、適用前にドライランのプランを実行します。PostgreSQLの場合:

```text
cargo run -p suprnova-magnetar \
  --features migration,seaorm-postgres \
  --example migrate -- \
  --source-shape torii \
  --database-url "$SOURCE_DATABASE_URL" \
  --app-database-url "$DATABASE_URL"
```

ソースとアプリケーションのデータベースドライバーに応じて、代わりに `seaorm-mysql` または `seaorm-sqlite` を使います。

レビュー済みのプランを適用するには `--apply` を追加します。ランナーはインポート前にソースとスキーマのフィンガープリントを再検査し、リトライ状態を記録し、アイデンティティの衝突を拒否し、トランザクション型インポートを使います。同一データベースのMySQL移行では、書き込みバリアで保護されたシャドースワップと、再開可能なリストアおよび中断パスを使います。

生成されたプランとレポートをデプロイ記録に保管してください。レビュー後にソースのフィンガープリントが変わったプランは適用しないでください。

## リファレンス

- デフォルトのブート: `MagnetarConfig`、`PasskeyConfig`、`init_magnetar`。
- ファサード: `Auth::oauth(provider)` と `Auth::magic_link()`。
- OAuth インストール: `MagnetarConfig::oauth`、`ReqwestOAuthTransport`、`FrameworkAbuseLimiter`。
- 移行ライブラリ: `suprnova-magnetar` クレートの `magnetar::migration`。
- Bearer認証: `BearerTokenMiddleware`。

## 次のステップ

- [認証](authentication.md)では、パスワード、パスキー、ガード、フレームワークセッション、エンジンの初期化を扱います。
- [認証フロー](auth-flows.md)では、メール検証、パスワードリセット、ロックアウト、二要素認証を扱います。
- [メール](mail.md)では、アプリケーションが所有するマジックリンク配信を扱います。
- [セッション](session.md)では、OAuthとパスキーのセレモニーをバインドするブラウザーセッションを扱います。
