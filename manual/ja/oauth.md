# OAuth、Apple、マジックリンクによるログイン

Suprnovaは、フレームワーク所有の `Auth` ファサードを通じて、OAuth、Sign in with Apple、パスワードレスのマジックリンクを公開します。このファサードの背後にあるクレデンシャル、セレモニー、アイデンティティ、要素ゲート、セッションの各エンジンはMagnetarが提供します。

公開エントリポイントは次のとおりです:

- OAuthとAppleには `Auth::oauth(provider)`。
- パスワードレスのメールログインには `Auth::magic_link()`。

Suprnovaはこれらのフローのルートをインストールしません。アプリケーションが開始ハンドラとコールバックハンドラを小さく用意し、マジックリンクのメールをどのように配信するかを決めます。

## Magnetarの初期化

`DB::init` の後、かつ `APP_KEY` によって `Crypt` が初期化された後に、デフォルトのパスワード、パスキー、セッション、ロックアウト、二要素の各エンジンを初期化します:

```rust
use suprnova::{DB, MagnetarConfig, PasskeyConfig, init_magnetar};

pub async fn register_auth() -> Result<(), suprnova::FrameworkError> {
    let database = DB::connection()?;
    let config = MagnetarConfig::from_sea_orm(database.inner().clone())
        .passkey_config(PasskeyConfig {
            rp_id: "app.example.com".to_string(),
            rp_origin: "https://app.example.com".to_string(),
        });

    init_magnetar(config).await
}
```

`MagnetarConfig` はアプリケーションのSeaORM接続を使用します。デフォルトエンジンは `apply_migrations` が有効（デフォルト）ならスキーマを作成します。デプロイ時に同じスキーマ設定を別途実行する場合にだけ `.apply_migrations(false)` を設定してください。

`init_magnetar` はパスワード/セッションアダプターとパスキーアダプターをアトミックにインストールします。2回目のインストールはエンジンを置き換えて認証状態を分割する代わりに、エラーを返します。

## OAuthエンジンのインストール

OAuthサポートは、フレームワークのデフォルトの `magnetar-oauth` 機能によってコンパイルされますが、プロバイダーの登録は常に明示的な実行時ステップです。`--no-default-features` ビルドでは、`magnetar-oauth` を明示的に有効にしてください。`init_magnetar` は内部の具体的なホストエンジンを返すことも公開することもないため、以下の例は自身で `MagnetarHostEngine` を構築して保持するアプリケーションにのみ当てはまり、前述のデフォルト初期化例に追加することはできません。現在の公開APIには、`MagnetarConfig` を通じてすでにインストールしたエンジンへOAuthレジストリを追加するためのコンビニエンスメソッドはありません。

```rust,ignore
use std::sync::Arc;
use suprnova::magnetar_integration::install_magnetar_oauth_engine;

let oauth = host_engine.oauth_service(oauth_host_config)?;
install_magnetar_oauth_engine(Arc::new(oauth))?;
```

`MagnetarOAuthHostConfig` は、`MagnetarOAuthProviderConfig` の明示的なリスト、HTTPトランスポート、濫用リミッター、認可ポリシー、自動リンクポリシーを受け取ります。インストールされるとプロバイダーレジストリが権威あるものになります。不明なプロバイダーは別の認証実装へフォールスルーせず、フェイルクローズします。

プロバイダー実装とクライアント認証の設定一式は `suprnova-magnetar` クレートから提供されます。OAuthエンジンを構築するアプリケーションは、利用するプロバイダー機能を有効にしたうえで、このクレートを直接の依存関係として追加する必要があります。フレームワークはOAuthクライアントIDやシークレットを環境変数から推測しません。アプリケーション設定またはシークレットマネージャーを通じて読み取り、ブートストラップ中にプロバイダーレジストリを構築してください。

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
- OAuthのインストール:
  `suprnova::magnetar_integration::install_magnetar_oauth_engine` と、
  `suprnova::magnetar_integration::engine` の設定型。
- 移行ライブラリ: `suprnova-magnetar` クレートの `magnetar::migration`。
- Bearer認証: `BearerTokenMiddleware`。

## 次のステップ

- [認証](authentication.md)では、パスワード、パスキー、ガード、フレームワークセッション、エンジンの初期化を扱います。
- [認証フロー](auth-flows.md)では、メール検証、パスワードリセット、ロックアウト、二要素認証を扱います。
- [メール](mail.md)では、アプリケーションが所有するマジックリンク配信を扱います。
- [セッション](session.md)では、OAuthとパスキーのセレモニーをバインドするブラウザーセッションを扱います。
