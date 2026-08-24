# 認証フロー

`suprnova::auth_flows` は、[認証](authentication.md)の上に乗るライフサイクル層です。`auth::*` が「このリクエストは誰か」に答えるのに対し、`auth_flows::*` はメールボックスの証明、パスワード復旧、アカウントのロックアウト、およびフレームワークのTOTPチャレンジを扱います。

この名前空間には、5つの表面があります:

- `EmailVerification` はフレームワークの `auth_flow_tokens` を発行して消費し、[`Mail`](mail.md) ファサードを通じてメールを送信し、設定済みの `UserProvider` を通じて認証済みトークン所有者を確認済みにします。
- `PasswordReset` は、利用可能な場合はインストール済みの Magnetar エンジンを使用します。Magnetar がない場合、検証済みアカウントは、設定済みの `UserProvider` とフレームワークの `auth_flow_tokens` を介してパスワードをリセットできます。未検証アカウントはフェイルクローズで拒否されます。これは、汎用プロバイダーでは、最初のメール証明に関する Magnetar のアトミックポリシーを実行できないためです。
- `BruteForce` と `LoginThrottleMiddleware` は、アカウントロックアウトの状態をインストール済みのMagnetarエンジンへ委譲します。
- `TwoFactor` は、`two_factor_credentials` を対象とするフレームワーク所有のTOTPファサードです。登録、確認、検証、リカバリーコード、シークレットのローテーション、チャレンジ昇格、およびタイムステップのリプレイ保護を提供します。
- `remember_me` は、名前空間互換性のためにレガシーなフレームワークのrememberモジュールを再公開します。Magnetarをインストールすると、通常の `Auth` および `SessionMiddleware` のrememberフローは代わりにMagnetarのクレデンシャルを使用します。

同じ名前空間には、2つのルートゲート用ミドルウェアが出荷されています:

- `EnsureEmailVerifiedMiddleware` は `AuthMiddleware` の後に合成され、`email_verified_at` に基づいてルートをゲートします。
- `TwoFactorChallengeMiddleware` は `AuthMiddleware` の前に合成され、保留中のフレームワークTOTPチャレンジを持つセッションをチャレンジフォームへリダイレクトします。

トランザクションメッセージは常にフレームワークの [`Mail`](mail.md) ファサードを使用します。Magnetarはセキュリティエンジンとストレージ契約を提供しますが、2つ目のアプリケーションメールトランスポートをインストールしません。

### 状態がどこに存在するか

メール確認トークンはフレームワークの `auth_flow_tokens` テーブルに存在し、確認済みタイムスタンプは設定済みの `UserProvider` を通じて書き込まれます。確認はアクターに束縛されています。現在認証されているユーザーがトークンを所有していなければなりません。

パスワードリセットトークン、パスワードクレデンシャル、ロックアウト行、不透明セッション、rememberクレデンシャル、パスキーのセレモニー、OAuthのセレモニー、および認証エポックは、インストール済みのMagnetarホストエンジンに属します。パスワードリセット、マジックリンク、およびOAuthの確認済みメール完了は、未確認アカウントを回収するためのMagnetarの原子的な初回メール証明境界を共有します。

この章の公開 `TwoFactor` ファサードは、フレームワーク所有の `two_factor_credentials` スキーマを維持します。Magnetarにも、統合されたパスワード、マジックリンク、パスキー、OAuth、およびセッションフローが使う要素エンジンがあります。2つのストアを交換可能だと想定しないでください。アプリケーションごとに一貫して1つの登録表面を使用してください。

Suprnovaは引き続きHTTPミドルウェア、クッキー、送信メール、イベント、および `UserProvider` ブリッジを所有します。アプリケーションコードはストレージエンジンを直接呼び出すのではなく、フレームワークのファサードを使用します。

## フローをまたぐ失敗のセマンティクス

あらゆるファサードは、1つの順序のルールに従います: 永続的な状態変更が先にコミットされ、それから通知の副作用が発火します。ミューテーションの後のリスナーのパニック、一時的なメールトランスポートの失敗、あるいはディスパッチャーのエラーは、そのミューテーションを巻き戻すことができません。

- `EmailVerification::verify` は認証済みトークン所有者を必要とし、`EmailVerified` を発火する前にトークンを消費してユーザーを確認済みにします。
- `PasswordReset::complete` は、利用可能な場合はインストール済みの Magnetar エンジンを介してコミットし、初回証明ポリシー、認証エポックの更新、アトミックな失効を行います。プロバイダーへのフォールバックは検証済みアカウント専用です。フレームワークトークンを消費し、プロバイダーのパスワードをローテーションしてから、フレームワークのセッションとログイン状態保持の失効結果を報告します。その後、メールとイベントが処理されます。
- `BruteForce::unlock_account` は、`AccountUnlocked` を発火する前にロック解除をコミットします。
- `TwoFactor::confirm` は、`TwoFactorEnrolled` を発火する前に `confirmed_at` を打刻します。`TwoFactor::disable` は、`TwoFactorDisabled` を発火する前に行を削除します。`TwoFactor::complete_challenge` は、標準の `auth::Login` + `auth::Authenticated` の組を送出し、続いて `TwoFactorChallenged` を送出する前に、pendingをauthedへ昇格させます。

永続性を必要とするリスナーは、自分の作業をバッファリングすべきです（リスナー本体からジョブをキューへ入れます）。ファサード自身は、決してリトライしません。

## ブートストラップ

`DB::init` の後、かつ `APP_KEY` が `Crypt` を初期化した後にMagnetarを初期化します:

```rust
use suprnova::{DB, MagnetarConfig, PasskeyConfig, init_magnetar};

pub async fn register() -> Result<(), suprnova::FrameworkError> {
    let database = DB::connection()?;
    let config = MagnetarConfig::from_sea_orm(database.inner().clone())
        .passkey_config(PasskeyConfig {
            rp_id: "app.example.com".to_string(),
            rp_origin: "https://app.example.com".to_string(),
        });

    init_magnetar(config).await
}
```

`init_magnetar` は、マイグレーションが無効でない限りデフォルトの認証スキーマを作成し、それからパスワード/セッションおよびパスキーアダプターを原子的にインストールします。2度目の呼び出しはエラーを返します。プロセスグローバルなインストールを必要とするテストは、インストール済みのエンジンを置換できないため、専用の統合テストバイナリーを使用すべきです。

### メール確認

メール確認には次が必要です:

1. メールでユーザーを取得し、確認タイムスタンプを打刻できる、登録済みの `UserProvider`。
2. アプリケーションのユーザー型に対する `MustVerifyEmail`。
3. null許容の `email_verified_at` カラム。
4. フレームワークの `auth_flow_tokens` テーブル。

```rust
use chrono::{DateTime, Utc};
use suprnova::MustVerifyEmail;

impl MustVerifyEmail for User {
    fn email(&self) -> &str {
        &self.email
    }

    fn email_verified_at(&self) -> Option<DateTime<Utc>> {
        self.email_verified_at
    }

    fn set_email_verified_at(&mut self, value: Option<DateTime<Utc>>) {
        self.email_verified_at = value;
    }
}
```

確認ハンドラは認証済みセッションのスコープ内で実行しなければなりません。他のユーザーに対する有効なトークンは、消費せずに拒否されます。

### パスワードリセットとロックアウト

`BruteForce` には、インストール済みの Magnetar パスワードエンジンが必要です。パスワードリセットではこのエンジンが優先されますが、`M` が `MustVerifyEmail + CanResetPassword` を実装している場合、`EloquentUserProvider<M>` は検証済みユーザーのリセットをサポートします。未検証ユーザーには、プロバイダー経由のリセットリンクは送信されません。リセットを最初のメールボックスのアトミックな証明として使用するには、Magnetar をインストールしてください。

パスワードリセットは、濫用リミッター、メール設定、エンジン、ストレージのチェックが成功した後に限り、未知のアドレスを `Ok(())` に正規化します。既知および未知のアカウントの経路は、失敗と実行時間が依然として異なる場合があります。完了では原子的な初回メール証明ストアを使用し、明示的なセッションまたはremember失効状態を必要とする呼び出し元に `PasswordResetOutcome` を返します。

### 2FAのマイグレーションを登録する

フレームワークはスキーマを出荷します。あなたのアプリは、両方のマイグレーションを自分のマイグレーターに一覧することでオプトインします:

```rust
use sea_orm_migration::prelude::*;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            // … あなた自身のマイグレーション …

            // `two_factor_credentials` を作成します。
            Box::new(suprnova::auth_flows::two_factor::migration::Migration),
            // TOTPのリプレイ保護のために `last_used_timestep` を追加します。
            Box::new(suprnova::auth_flows::two_factor::migration_replay::Migration),
        ]
    }
}
```

どちらも、すでに適用済みのデータベースに対してべき等です（v1は `CREATE TABLE IF NOT EXISTS` を使い、v2はカラムの追加です）。すでにスキーマを持っている本番データベースに対して `suprnova migrate` を再実行しても、何も起きません。

### 環境変数

トランザクションメールは、送信時に2つの環境変数を読み取ります:

| 変数 | デフォルト | 用途 |
|---|---|---|
| `APP_NAME` | `"Suprnova"` | 件名のブランディングと、認証アプリが表示する `otpauth://` の発行者ラベル。 |
| `MAIL_FROM` | なし - **未設定だとエラーになります** | あらゆる送信メッセージのエンベロープ `From`。検証済みの送信ドメインに設定してください。 |

`MAIL_FROM` は、意図的にデフォルトを持ちません。`noreply@example.com` のようなプレースホルダーにデフォルトさせてしまうと、本番でDMARC / SPFをサイレントに壊し、運用者が制御していないドメインから送信することになってしまいます。そのため、ファサードは代わりにフェイルクローズします。`EmailVerification::send_link` と `PasswordReset::send_link` は、エラーを `Err` として表面化させます。`PasswordReset::complete` は `tracing::warn!` でログに記録し、続行します（パスワードの変更はすでにコミットされているため、通知の経路がそれを巻き戻すことはできません）。

アプリはさらに `APP_URL` を設定するため、コントローラーは `send_link` の呼び出しで使うベースURLを導出できます。フレームワークのファサード自身は、ベースURLをパラメータとして受け取ります。

メールのドライバーは、`MAIL_DRIVER` を介して別に設定します - [メール](mail.md)のドキュメントを参照してください。

## メール確認

`EmailVerification` は、`auth_flow_tokens` テーブルに対して確認トークンを発行し、確認し、消費し、設定済みのプロバイダーを通じてユーザーを確認済みにします。4つの操作がライフサイクルをカバーします:

| メソッド | 署名 | 備考 |
|---|---|---|
| `send_link` | `send_link<U: MustVerifyEmail>(user: &U, base_url: &str) -> Result<()>` | すでに手元にあるユーザーに対して、発行 + メール送信します。 |
| `resend` | `resend(email: &str, base_url: &str) -> Result<()>` | 未知のプロバイダー結果を `Ok(())` に正規化します。トークンストレージとメールの失敗は `Err` のままで、実行時間は等しくされません。 |
| `check` | `check(token: &str) -> Result<bool>` | 消費しません - ランディングページで呼んでも安全です。 |
| `verify` | `verify(token: &str) -> Result<String>` | アクター束縛かつ使い捨て: 認証済みユーザーがトークンを所有していなければなりません。成功時はトークンを消費し、ユーザーを確認済みにし、そのユーザーIDを返します。 |

```rust
use suprnova::auth_flows::EmailVerification;

// 新規登録の直後、作られたばかりのユーザーが手元にある状態で:
EmailVerification::send_link(&user, "https://app.example.com/verify-email").await?;

// 任意のランディングページチェック - 消費しないため、ページを
// リフレッシュしてもトークンが失われません。
let valid: bool = EmailVerification::check(&token_str).await?;

// クリックスルーのハンドラは認証の背後で実行されます。`verify` は
// `Auth::id()` が所有者と一致するときだけトークンを消費します。
let user_id: String = EmailVerification::verify(&token_str).await?;
```

`verify` は成功時に `EmailVerified` を発火します - リスナーは、確認ハンドラに結合させることなく、追加の機能（ウェルカムメール、デフォルトのフォロー、「プロフィールを完成させましょう」というCTA）を解き放つのに正しい場所です。このイベントは、プロバイダーのユーザーidを運びます。

### resendエンドポイント（列挙攻撃対策）

`resend` はemailだけを受け取ります - ファサードは有効なプロバイダーを通じてユーザーをルックアップし、アカウントが存在する場合にはトークンを発行してメールを送ります。未知のプロバイダー結果は `Ok(())` に正規化されますが、トークンストレージとメール配信の失敗は `Err` のままであり、実行時間は等しくされません:

```rust
use std::collections::HashMap;
use suprnova::auth_flows::EmailVerification;
use suprnova::{FrameworkError, HttpResponse, Request, Response};

pub async fn resend(req: Request) -> Response {
    resend_inner(req).await.map_err(HttpResponse::from)
}

async fn resend_inner(req: Request) -> Result<HttpResponse, FrameworkError> {
    let raw = req.query().unwrap_or("");
    let params: HashMap<String, String> =
        url::form_urlencoded::parse(raw.as_bytes()).into_owned().collect();
    let email = params
        .get("email")
        .ok_or_else(|| FrameworkError::bad_request("missing email"))?;

    let base = format!(
        "{}/auth/verify",
        std::env::var("APP_URL").unwrap_or_else(|_| "http://localhost:8765".into()),
    );
    // `resend` は、ルックアップ + 列挙攻撃対策を内部で行います。
    EmailVerification::resend(email, &base).await?;

    Ok(HttpResponse::text(
        "If this email is on file, a verification link has been sent.",
    ))
}
```

`send_link` と `resend` はどちらも、URLを `{base_url}?token={plaintext_token}` として組み立てます。`base_url` の末尾のスラッシュは、クエリ文字列が追加される前に取り除かれるため、`https://app.example.com/verify/` と `https://app.example.com/verify` は、どちらもきれいなURLを生成します。

クリックスルーのハンドラは `AuthMiddleware` の背後で実行しなければなりません。クエリ文字列からトークンを取り出し、`verify` を呼び出します:

```rust
async fn verify_inner(req: Request) -> Result<HttpResponse, FrameworkError> {
    let raw = req.query().unwrap_or("");
    let params: HashMap<String, String> =
        url::form_urlencoded::parse(raw.as_bytes()).into_owned().collect();
    let token = params
        .get("token")
        .ok_or_else(|| FrameworkError::bad_request("missing token"))?;

    let _user_id = EmailVerification::verify(token).await?;

    Ok(HttpResponse::new().status(302).header("Location", "/"))
}
```

`verify` は消費前に `Auth::id()` をトークン所有者と照合します。他のアカウントに属するトークンは同じ無効トークン応答を返し、未使用のままです。成功時には、プロバイダーが認証済み所有者を確認済みにし、ファサードが `EmailVerified` を発火します。

### 確認済み限定のルート: `EnsureEmailVerifiedMiddleware`

`EnsureEmailVerifiedMiddleware` は、認証済みユーザーの `email_verified_at` に基づいてルートをゲートします。`AuthMiddleware` の後に合成すると、チェーンは、確認のステップをまだ完了していないユーザーのリクエストをすべてブロックします。

**403 JSON** と **302のHTMLリダイレクト** の選択は、コンストラクタを介してルート登録の時点で行われます - リクエストの内容を覗き見ることはなく、`AuthMiddleware::new` / `AuthMiddleware::redirect_to` が定めたパターンと一致します:

```rust
use suprnova::{AuthMiddleware, EnsureEmailVerifiedMiddleware, group, get};

// APIの表面 - JSONボディを伴う403。
group!("/api")
    .middleware(AuthMiddleware::new())
    .middleware(EnsureEmailVerifiedMiddleware::new())
    .routes([
        get!("/me", profile::show),
    ]);

// Webの表面 - 302（Inertiaの遷移では409 + X-Inertia-Location）。
group!("/dashboard")
    .middleware(AuthMiddleware::redirect_to("/login"))
    .middleware(EnsureEmailVerifiedMiddleware::redirect_to("/email/verify"))
    .routes([
        get!("/", dashboard::index),
    ]);
```

ユーザーが認証されていない場合、ミドルウェアは「認証済みだが未確認」と同じレスポンスの分岐に落ち込みます - Laravelの `! $request->user() || ! hasVerifiedEmail()` という形と一致します。未認証のリクエストに対して別個の `401` を望む場合は、`AuthMiddleware` を先に合成してください。

ハンドラ内部での分岐のためには（例えば、リダイレクトせずに条件付きで「確認してください」というCTAを描画するなど）、セッションの認証ガードを通じて型付きのユーザーを読み込み、トレイトのメソッドを読んでください:

```rust
use suprnova::{Auth, MustVerifyEmail};
use crate::models::users::User;

if let Some(user) = Auth::user_as::<User>().await? {
    let verified: bool = user.is_email_verified();
    // これで分岐する
}
```

## パスワードリセット

`PasswordReset` には4つの操作があります:

| メソッド | 署名 | 備考 |
|---|---|---|
| `send_link` | `send_link(email: &str, base_url: &str) -> Result<()>` | 濫用リミッター、メール設定、エンジン、ストレージのチェックが成功した後、未知のアドレスに対して `Ok(())` を返します。その他の失敗は `Err` のままです。 |
| `check` | `check(token: &str) -> Result<bool>` | インストール済みのMagnetarエンジンによる、消費しない検証です。 |
| `complete` | `complete(token: &str, new_password: &str) -> Result<String>` | トークンを原子的に消費し、初回証明ポリシーを適用し、クレデンシャルをローテーションして、セッションとremember状態を失効させ、ユーザーIDを返します。 |
| `complete_with_outcome` | `complete_with_outcome(token, new_password) -> Result<PasswordResetOutcome>` | 同じトランザクションを実行し、コミット済みの失効件数を返します。 |

```rust
use suprnova::auth_flows::PasswordReset;

// 「パスワードを忘れた」フォームからのものです。未知のアドレスは、前提チェックが
// 成功した後に `Ok(())` を返ります。設定とバックエンドのエラーは引き続き表面化します。
PasswordReset::send_link(&email, "https://app.example.com/reset").await?;

// 新しいパスワードのフォームを描画する前の、任意のランディングページチェックです。
let valid: bool = PasswordReset::check(&token).await?;

// クリックスルーのハンドラです。ユーザーが新しいパスワードを送信した後、
// トークンを消費してパスワードをローテーションし、ユーザーidを返します。
let user_id: String = PasswordReset::complete(&token, &new_password).await?;
```

`complete` は平文パスワードを `SecretString` を通じて渡します。Magnetarはクレデンシャルエンジン内でそれをハッシュ化します。事前にハッシュ化しないでください。空または空白のみのパスワードは、エンジンが呼び出される前にHTTP 400を返します。

### 限定された列挙防止の振る舞い

`PasswordReset::send_link` は、濫用リミッター、メール設定、エンジン、ストレージのチェックが成功した後に限り、未知のアドレスへ `Ok(())` を返します。設定、リミッター、ストレージ、メールの失敗は引き続き `Err` を返します。ドッグフードのコントローラーは、成功した既知および未知のアカウントのリクエストに同じHTTPステータスとボディを与えますが、実装はそれらの実行時間を等しくしません。

### `complete` の副作用

Magnetarは、1つのトランザクションでパスワードリセットをコミットします:

1. 使い捨てリセットトークンを消費します。
2. アカウントがまだ未確認である場合、初回メール証明ポリシーを適用します。
3. パスワードをハッシュ化して置き換えます。
4. 認証エポックを進めます。
5. 古い不透明セッションとrememberクレデンシャルを失効させます。
6. このリセットがアカウントの最初のメールボックス証明である場合、暫定クレデンシャルを削除します。

コミット後、フレームワークは `PasswordChangedMail` を送信し、`PasswordResetCompleted` をディスパッチします。メールまたはリスナーの失敗でリセットをロールバックすることはできません。

すでに確認済みのアカウントでは、リセットは正当なパスキー、リンク済みアカウント、および確認済み二要素登録を保持します。未確認の乗っ取られたアカウントでは、初回証明が暫定クレデンシャルを削除するため、以前の登録者はアクセスを保持できません。

## ブルートフォース対策

ブルートフォースの層には2つの部分があります: ロックアウトの状態を記録し問い合わせる `BruteForce` ファサードと、ハンドラが呼び出される前にHTTPの層でショートサーキットする `LoginThrottleMiddleware` です。

### `BruteForce` ファサード

あなたのログインハンドラの、認証失敗の分岐からは `record_failed_attempt` を、成功の分岐からは `reset_attempts` を呼び出してください:

```rust
use suprnova::auth_flows::BruteForce;

// 認証失敗の経路では:
let status = BruteForce::record_failed_attempt(&email, Some(&peer_ip)).await?;
if status.is_locked {
    // 任意でカスタムのレスポンスを表面化させます。ミドルウェアは
    // *次の*リクエストでこれを代わりに行います - 下記を参照してください。
}

// 成功の経路では:
BruteForce::reset_attempts(&email).await?;
```

`record_failed_attempt` は、更新された `LockoutStatus`（`is_locked`、`failed_attempts`、そしてロックされている場合は `locked_until`）を返します。監査ログのために、任意の `ip` を渡してください。あなたのトランスポートがクライアントのIPをきれいに表面化させない場合は、`None` を渡してください。

さらに2つの操作があります:

```rust
// 読み取り専用 - 履歴のないemailに対しても安全です。
let status = BruteForce::get_lockout_status(&email).await?;
let locked: bool = BruteForce::is_locked(&email).await?;

// 管理者による / 強制的なロック解除です。実際の状態遷移が起きた場合にのみ
// `AccountUnlocked` を発火します（すでにロック解除されているアカウントに対する
// no-opのロック解除は発火しません）。
let was_locked: bool = BruteForce::unlock_account(&email).await?;
```

`unlock_account` は、呼び出しの時点でアカウントがロックされていた場合に `true` を、それ以外の場合に `false` を返します。`AccountUnlocked` イベントは `true` のときにのみ発火します - `false` の戻り値は、あるがままのno-opであり、監査イベントではありません。

### `LoginThrottleMiddleware`

このミドルウェアは、リクエストが対象としているどのemailについても、ロックアウトの状態を読み取り、アカウントがロックされている場合は `429 Too Many Requests` でショートサーキットします。ログインハンドラは決して呼び出されないため、ロックされたアカウントは、認証情報のチェックを試みることさえできません:

```rust
use suprnova::auth_flows::LoginThrottleMiddleware;
use suprnova::Router;

// emailの抽出器は `&Request` に対する同期のクロージャです。JSON/フォームの
// ボディを読むのはasyncであり `Request` を消費するため、このクロージャは
// ボディを読めません - 代わりにヘッダー、クエリ文字列、あるいはルートの
// パラメータから取り出してください。
let throttle = LoginThrottleMiddleware::new(|req| {
    req.header("X-Login-Email").map(str::to_string)
});

let router = Router::new()
    .post("/login", login_handler)
    .middleware(throttle);
```

実用的な抽出の表面です:

- ヘッダー（`X-Login-Email`）で、前段のプリプロセッサによって設定されるもの - ドッグフードアプリで使われているパターンです。
- クエリ文字列パラメータ（`?email=…`）。
- ルートパラメータ（`/login/{email}`）。

抽出器から `None` を返すことは、「チェックすべきものが何もない」という明示的な信号です - ミドルウェアはリクエストを変更せずに通します。これにより、ミドルウェアは、時折匿名のトラフィックを目にするルート（例えば、emailなしの「パスワードリセットをリクエストする」というサブアクションも処理する、同じ `POST /login` エンドポイント）に対してもインストールしても安全です。

ロックされている場合、ミドルウェアは次を返します:

- ステータス `429 Too Many Requests`。
- `Retry-After` ヘッダー - 秒数で、ロックアウトの `locked_until` から `LockoutStatus::retry_after_seconds` を介して計算されます。タイムスタンプが何らかの理由で存在しない場合は、`900`（15分、Magnetarのデフォルトのロックアウト期間）にフォールバックします。
- ボディ: `"Account locked due to too many failed login attempts. Try again later."`

### バックエンドエラー（デフォルトはフェイルクローズ）

`get_lockout_status` がエラーを返す場合、`LoginThrottleMiddleware` はその失敗をログに記録し、デフォルトではログインハンドラを呼び出さずに、`Retry-After: 1` を伴うHTTP `503 Service Unavailable` を返します。ロックアウトバックエンドの停止中もログインを利用可能にするには、`.on_backend_error(BackendErrorPolicy::FailOpen)` で明示的にオプトインしてください。そのポリシーだけがリクエストをハンドラへ通します。

### `RateLimitMiddleware` と重ねる

`LoginThrottleMiddleware` はアカウント単位です - しきい値を超えたときに、単一のemailをゲートします。IP単位のクォータには、[`RateLimitMiddleware`](rate-limiting.md) と重ねてください。この2つは自然に合成できます:

```rust
let router = Router::new()
    .post("/login", login_handler)
    .middleware(LoginThrottleMiddleware::new(|req| { /* ... */ }))
    .middleware(RateLimitMiddleware::ip_based(20, std::time::Duration::from_secs(60)));
```

両者を組み合わせると、クレデンシャルスタッフィングの現実的な形をカバーします: 分散型（1つのemail × 多数のIP）はレートリミットの仕事であり、集中型（多数の試行 × 1つのemail）はスロットルミドルウェアの仕事です。

### 設定

`MagnetarConfig` は `LockoutConfig` を受け入れます。デフォルトは、失敗5回、15分のカウント期間およびロックアウト期間、7日間の試行保持、および `BackendErrorPolicy::FailClosed` です:

```rust,ignore
let config = MagnetarConfig::from_sea_orm(database)
    .lockout_config(lockout_policy);
```

他のフェイルクローズのアイデンティティ制御がアカウントロックアウトを置き換える場合にのみ、`LockoutConfig::disabled()` を使用してください。

## 2FA（TOTP）

`TwoFactor` は、TOTPベースの2FAをカバーします - 標準に準拠したあらゆる認証アプリ（Google Authenticator、1Password、Bitwarden、Authy）と組み合わせられる種類のものです。フローは、登録 → 確認 → 継続的な検証で、それに加えて、ユーザーがデバイスを失ったときのための使い捨てのリカバリーコード、そしてすべてをログインのライフサイクルへ縫い込むチャレンジフローがあります。

### `TwoFactorUser` トレイト

フレームワークは、あなたのアプリケーションのユーザーストレージに手を伸ばすことができないため、呼び出し側は、自分のユーザーモデルから2FAファサードへ橋渡しする小さなトレイトを実装します:

```rust
use suprnova::auth_flows::TwoFactorUser;

pub trait TwoFactorUser: Send + Sync {
    fn user_id(&self) -> &str;
    fn email(&self) -> &str;
}
```

`user_id` は不透明なストレージキーです。テキストとして表した数値のアプリケーションID、UUID、またはMagnetarの `UserId` にできます。フレームワークのTOTPテーブルは、アプリケーションユーザーテーブルへの外部キーを持ちません。

`email` は `otpauth://` URLの `account_name` セグメントへ折り込まれるため、認証アプリは認識できるアカウントラベルを表示します。

よくあるパターンは、あなたのユーザーモデルをラップする小さなニュータイプです:

```rust
use suprnova::auth_flows::TwoFactorUser;

struct AppUser2fa<'a> {
    user: &'a User,
}

impl TwoFactorUser for AppUser2fa<'_> {
    fn user_id(&self) -> &str {
        &self.user.auth_id
    }

    fn email(&self) -> &str {
        &self.user.email
    }
}
```

### ストレージ

2FAの状態は、フレームワークが所有する `two_factor_credentials` テーブルに存在します。シークレットとリカバリーコードは、`crate::crypto::Crypt::encrypt_string` によって保存時に暗号化され、これはプロセスグローバルな `EncryptionKey` を必要とします。アプリは、両方のマイグレーションを `Migrator::migrations()` に一覧することで、このスキーマにオプトインします - [ブートストラップ](#ブートストラップ)を参照してください。

### 登録、確認、検証

```rust
use suprnova::auth_flows::{TwoFactor, EnrollmentResponse};

// 1. 登録: 新しいシークレット + 10個のリカバリーコードを生成し、
//    それらを暗号化して永続化し、QRコードを描画するために必要な
//    すべてを返します。
let response: EnrollmentResponse = TwoFactor::enroll(&user_2fa).await?;
// response.otpauth_url - `otpauth://totp/...` というディープリンク
// response.qr_code_svg - base64のPNGをラップする<svg>。インラインで埋め込む
// response.recovery_codes - Vec<String>、10個の平文のコード - 表示は1回だけ

// 2. 確認: ユーザーが認証アプリを開き、6桁のコードを入力します。
//    `confirm` はそれを検証し、`confirmed_at` に打刻します。
TwoFactor::confirm(&user_2fa, &user_typed_code).await?;
// `TwoFactorEnrolled` を発火する

// 3. 以降のログインでは、`verify` でセッションをゲートします:
let ok: bool = TwoFactor::verify(&user_2fa, &code_from_login_form).await?;
if !ok {
    return Err(suprnova::FrameworkError::domain("invalid 2FA code", 401));
}
```

`enroll` は、平文のリカバリーコードを**ちょうど1回だけ**返します。後でそれらを取得するAPIはありません - 暗号化されたカラムは、この時点から一方向になります。それらを登録成功のページに表示し、ユーザーに保存を促し、平文をどこにも他には保存しないでください。

`enroll` は、**確認済み**の登録を上書きすることを拒否します - これは `409` を返し、呼び出し元を `re_enroll`（所持の証明を必要とします）へ押し向けます。未確認の（保留中の）行に対する再登録は許可されています: 以前の登録は、一度も正式なものになっていないからです。

### リプレイ保護

`verify` は成功時に、現在のTOTPのタイムステップを `last_used_timestep` へ書き込みます。以降の検証で `current_timestep <= last_used_timestep` となるものは、コード自体が構造的に有効であっても拒否され、30秒のウィンドウの内側での盗まれたコードのリプレイを打ち破ります。

タイムステップの主張はアトミックです。打刻は、条件付きの `UPDATE … WHERE last_used_timestep IS NULL OR last_used_timestep < :current` を介して行われ、検証は、その文がちょうど1行に影響を与えたときにのみ成功します。同じタイムステップにおける2つの並行した検証は、両方が勝つことはできません: 最初のものがカラムを反転させ、2番目のものの述語はもはやマッチせず、2番目のものはリプレイとして扱われます。素朴なread-modify-writeであれば、TOCTOUの競合になってしまいます - 両方の検証が打刻前の行を読み、両方が同じコードを検証し、両方が打刻し、両方が成功してしまいます。並行する競合者もまた失敗した試行として数えられるため、ブルートフォースのカウンターがそれらを記録します。

### リカバリーコード

```rust
let consumed: bool = TwoFactor::consume_recovery_code(&user_2fa, &code).await?;
```

使い捨て: マッチしたコードは、呼び出しが返るより前に行から取り除かれるため、同じコードに対する2回目の試みは `false` を返します。コードは、`NNNNNN-NNNNNN` の形をした12桁の10進数です（それぞれ約40ビットのエントロピーで、Laravel Fortifyの形式と一致します）。

`consume_recovery_code` は、2FAが完全に確認されている場合にのみコードを受け付けます - `confirmed_at` がNULLである間は `Ok(false)` にショートサーキットします。このゲートがなければ、被害者のアカウントで登録を引き起こした攻撃者（あるいは、確認せずに行を作成するあらゆるフロー）は、新しいリカバリーコードだけを使って認証できてしまい、TOTPを完全に迂回できてしまいます。この契約は、`verify` の「確認済みの登録のみ」という制約と対をなしています。

### リカバリーコードとシークレットをローテーションする

ユーザーがリカバリーコードを使い果たしたとき、あるいは侵害が疑われた後にそれらをローテーションしたいとき:

```rust
let fresh: Vec<String> = TwoFactor::regenerate_recovery_codes(&user_2fa, &proof).await?;
```

`proof` は、現在のTOTPコードか、未使用のリカバリーコードのいずれかとして検証されなければなりません。この証明のチェックがなければ、セッションを乗っ取った攻撃者が、正当なユーザーのリカバリーコードをサイレントに吹き飛ばしてしまえます（アカウントリカバリーに対するサービス拒否）。新しいコードは、永続化されている集合を置き換えます。既存のシークレットと `confirmed_at` は保たれるため、ユーザーの認証アプリは、再ペアリングせずに機能し続けます。エラー:

- `400` - 確認済みの登録が存在しません。先に `enroll`/`confirm` を呼び出してください。
- `401` - `proof` が、TOTPコードとしても未使用のリカバリーコードとしても検証されません。
- `429` - アカウントがブルートフォースのスロットリングによってロックされています。

先に2FAを無効化せずに**シークレット**をローテーションする（新しいデバイスに再ペアリングする）には:

```rust
let response = TwoFactor::re_enroll(&user_2fa, &proof).await?;
```

`regenerate_recovery_codes` と同じ証明モデルです。この行は、新しいシークレット + 10個の新しいリカバリーコードで書き直されます。`confirmed_at` はNULLへリセットされるため、2FAが再び有効になる前に、ユーザーは新しい認証アプリからのコードで `confirm` しなければなりません。

### 無効化する

```rust
TwoFactor::disable(&user_2fa).await?;
// 行が削除された場合にのみ `TwoFactorDisabled` を発火する
```

べき等です: 一度も登録していないユーザーに対する無効化はエラーではありません。`TwoFactorDisabled` イベントは、実際の状態遷移が起きた場合にのみ発火するため、監査のリスナーは、no-opのボタンをクリックするたびに1件ではなく、実際の無効化のたびに1件を目にします。

### チャレンジフロー（第二要素でログインをゲートする）

enroll / confirm / verifyのプリミティブが構成要素であり、**チャレンジフロー**が、それらをログインのライフサイクルへ縫い込むことで、2FAが有効なユーザーが、パスワードだけで保護されたページに到達できないようにします。

フローは次のとおりです:

1. パスワードログインがユーザーを解決します。
2. `TwoFactor::is_enabled_by_id(&user_id)` が `true` を返す場合、ログインハンドラは `TwoFactor::start_challenge(user_id, remember)` を呼び出します - これは、ユーザーidを**pending**としてセッションに一時保存し、完全に認証済みのスロットをクリアし、`Auth::attempt` によって発行されたremember-meクッキーを失効させ、チャレンジが完了した後にクッキーを再発行できるよう、ユーザーがremember-meを選んだかどうかを記憶します。`Auth::id()` は、この時点からチャレンジが完了するまで `None` を返します。
3. ハンドラは、コードのフォームを表示する `/two-factor-challenge` ルートへリダイレクトします。
4. チャレンジのPOSTハンドラは `TwoFactor::complete_challenge(code)` を呼び出します - コードを検証し（TOTP **または** 未使用のリカバリーコード。Fortifyのチャレンジコントローラーと一致します）、pendingをauthedへ格上げし、セッションid（セッション固定化を打ち破ります）とCSRFトークンをローテーションし、ユーザーが選んでいた場合はremember-meクッキーを再発行し、標準の `auth::Login` + `auth::Authenticated` のライフサイクルイベントと、2FA特有の `TwoFactorChallenged` をディスパッチします。

```rust
use suprnova::auth_flows::TwoFactor;
use suprnova::{Auth, Authenticatable, Credentials, redirect};

pub async fn login(form: LoginRequest) -> Response {
    match Auth::attempt(&Credentials::password(&form.email, &form.password), form.remember).await? {
        Some(user) => {
            let user_id = user.get_auth_identifier();
            if TwoFactor::is_enabled_by_id(&user_id).await? {
                // 「pending」へ降格させます: 認証スロットはクリアされ、pendingが設定され、
                // remember-meクッキーは失効します。`complete_challenge` が成功時にクッキーを
                // 再発行できるよう、フォームのrememberフラグを通してください。
                TwoFactor::start_challenge(user_id, form.remember).await?;
                redirect!("/two-factor-challenge").into()
            } else {
                redirect!("/dashboard").into()
            }
        }
        None => Err(invalid_credentials().into()),
    }
}

pub async fn complete(form: TwoFactorChallengeRequest) -> Response {
    let _user = TwoFactor::complete_challenge(&form.code).await?;
    // セッションid + CSRFはローテーションされました。元のログインフォームが
    // 設定していた場合、remember-meは再発行されています。`auth::Login` /
    // `auth::Authenticated` にフックするリスナーには、通常のログインとして見えます。
    redirect!("/dashboard").into()
}
```

`complete_challenge` は、authedへの格上げの一部として、セッションidとCSRFトークンをローテーションします。これによって、攻撃者が被害者にログインより前に既知のセッションidを植え付ける、古典的なセッション固定化攻撃を締め出します - ローテーションの後、植え付けられたidは死んでおり、新しく生成されたidだけが認証済みの状態を運びます。この契約は `Auth::login_id` / `Auth::login_using_id` と一致するため、セッションの状態とリスナーの可観測性という点で、2FAのログインは2FAなしのログインと見分けがつきません。

保護されたすべてのルートグループを、`AuthMiddleware` の**前に** `TwoFactorChallengeMiddleware` でゲートしてください。これにより、pending状態のセッションは、ログインページではなくチャレンジページへリダイレクトされます:

```rust
use suprnova::{AuthMiddleware, TwoFactorChallengeMiddleware, group, get};

group!("/dashboard")
    .middleware(TwoFactorChallengeMiddleware::redirect_to("/two-factor-challenge"))
    .middleware(AuthMiddleware::redirect_to("/login"))
    .routes([
        get!("/", dashboard::index),
    ]);
```

チャレンジページ自身（フォームを描画するGETと、`complete_challenge` を呼び出すPOST）は、`TwoFactorChallengeMiddleware` をインストールしては**なりません** - それが目的地だからです。POSTハンドラは通常、`TwoFactor::pending_user_id().is_some()` も事前にチェックし、古くなったリンクが空のセッションで検証ロジックに到達しないようにします。

`TwoFactor::cancel_challenge()` は、誰も認証することなく、両方のpendingスロットをクリアします - チャレンジページの「ログインに戻る」リンクにこれを配線してください。

**リカバリーコードのフォールバック。** `complete_challenge(code)` は、まずTOTPの経路を試し、リカバリーコードを消費するほうへフォールバックします。そのため、認証アプリを失ったユーザーでも、それでもログインできます。各リカバリーコードは使い捨てです。

**ブルートフォースとの連携。** 失敗したチャレンジのコードは、素の `TwoFactor::verify` と同じやり方で、`BruteForce::record_failed_attempt` を通じてアカウント単位のブルートフォースカウンターに供給されます。チャレンジフォームを何度も試す攻撃者は、設定済みのしきい値の後で `AccountLocked` を引き起こします。`complete_challenge` は内部でTOTPとリカバリーコードの両方の経路を試しますが、1回の不正な送信は**1回**の失敗した試行として数えられます - サイレントな検証のコアはブルートフォースカウンターをスキップするため、外側の層が正規の試行をちょうど1回だけ記録します。

**ロックアウトゲート。** `complete_challenge` は事前に `BruteForce::is_locked` をチェックし、アカウントがすでにロックされている場合は `429 Too Many Requests` を返します - 送信されたコードが正しい場合でもです。このメソッド内のゲートがなければ、ロックアウトを引き起こした攻撃者が、次のリクエストで正しいコードを送信することで、それでもログインできてしまいます: ブルートフォースのカウンターはユーザーのemailをキーにしていますが、`verify` 自身はそれを参照しません。パスワードの経路の `LoginThrottleMiddleware` は、同じ制約をルートの層で強制します。それをチャレンジのPOSTルートの手前に合成しても問題ありません - どちらのゲートもべき等です。

**失敗イベント。** `complete_challenge` は、不正なコード（あるいはロックされたアカウント）に対して `TwoFactorChallengeFailed { user_id }` をディスパッチします。これは、パスワードの経路の `auth::Failed` とは別物です。「ユーザーが2FAを試して失敗した」を見張るリスナーは、新しいイベントを購読します。「パスワードが認証しなかった」を見張るリスナーは、`auth::Failed` に留まります。この2つの表面は分けて保たれるため、2FAのタイプミスが、監査のパイプラインにとってパスワードの失敗のように見えることはありません。

### Suprnovaが異なる設計を選んだ理由

フレームワークTOTPの `user_id` は `String` です。固定された `i64`、UUID、またはMagnetar識別子型なら、この再利用可能なファサードを1つのアプリケーションスキーマに結び付けてしまいます。文字列境界により、アプリは呼び出し箇所で1回変換するだけで任意の安定した識別子を選べます。

Magnetarの統合要素ゲートは、この維持されるファサードとは別です。この分離は `two_factor_credentials` を使うアプリケーションの互換性を保ちますが、アプリケーションは同じアカウントを両方のストアから登録すべきではありません。

## Remember-me

`suprnova::auth_flows::remember_me` は、互換性のためにレガシーな `suprnova::auth::remember` モジュールを再公開します。

Magnetarをインストールすると、通常の `Auth::attempt(..., true)`、`Auth::issue_remember_cookie`、および `SessionMiddleware` のハイドレーションは、Magnetarの目的束縛されたrememberクレデンシャルを使用します。Magnetarは検証子ダイジェストを保存し、認証エポックを確認し、使用成功時にクレデンシャルをローテーションし、ユーザーセッションと共に失効させ、シークレットを露出せずにリプレイまたは不正なクレデンシャルの異常を報告します。

ブラウザー向けクッキーは引き続きフレームワークが所有します。論理名 `remember_me` で暗号化され、`SESSION_COOKIE_PREFIX` に従い、ストレージ障害でブラウザーが古いクレデンシャルを送信し続けないよう、バックエンドの失効より前に消去されます。

Magnetarエンジンがインストールされていない場合、レガシーなデータベース行実装を引き続き利用できます。新しいアプリケーションはMagnetarを初期化し、レガシーな再公開を移行用の表面として扱うべきです。

## イベント

9つのイベントが、フローをまたいで発火します。セキュリティ状態の遷移1つにつき1つです:

| イベント | 発火元 | 運ぶもの |
|---|---|---|
| `EmailVerified` | `EmailVerification::verify` の成功時 | `user_id: String` |
| `PasswordResetLinkSent` | `PasswordReset::send_link` の成功時 - 存在しないemailに対しては列挙攻撃対策でサイレント | `user_id: String`、`email: String` |
| `PasswordResetCompleted` | `PasswordReset::complete` の成功時 | `user_id: String` |
| `AccountLocked` | `BruteForce::record_failed_attempt` の、unlocked → lockedの遷移時 | `email: String`、`failed_attempts: u32` |
| `AccountUnlocked` | `BruteForce::unlock_account` の、実際にロック解除が起きたとき | `email: String` |
| `TwoFactorEnrolled` | `TwoFactor::confirm` の成功時 | `user_id: String` |
| `TwoFactorChallenged` | `TwoFactor::complete_challenge` が pending → authed へ格上げしたとき | `user_id: String` |
| `TwoFactorChallengeFailed` | `TwoFactor::complete_challenge` が不正なコードを拒否した、あるいはロックされたアカウントを拒んだとき | `user_id: String` |
| `TwoFactorDisabled` | `TwoFactor::disable` の、行が実際に削除されたとき | `user_id: String` |

あらゆるイベントは `Debug + Clone + 'static` であり、機密なデータ（平文のトークンなし、IPなし）を運ばず、文字列的な識別子を使います。そのため、リスナーは、ユーザーストレージのバックエンドから型情報を漏らすことなく、タスクの境界をまたいでそれらをシリアライズできます。

### リスンする

標準のイベントAPIを介して購読してください - 他のあらゆるプロセス内イベントと同じ表面です:

```rust
use std::sync::Arc;
use suprnova::async_trait;
use suprnova::auth_flows::events::AccountLocked;
use suprnova::{EventFacade, FrameworkError, Listener};

pub struct PageOpsOnLockout;

#[async_trait]
impl Listener<AccountLocked> for PageOpsOnLockout {
    async fn handle(&self, event: &AccountLocked) -> Result<(), FrameworkError> {
        tracing::warn!(
            email = %event.email,
            failed_attempts = event.failed_attempts,
            "account locked - paging ops",
        );
        // … Slack通知、監査テーブルへの追記など。
        Ok(())
    }
}

// bootstrap.rs にて:
EventFacade::listen::<AccountLocked, _>(Arc::new(PageOpsOnLockout)).await;
```

リスナーはTokioのランタイム上で実行され、登録順にディスパッチされます。完全な表面については、[イベント](events.md)の章を参照してください。

## テスト

3つのフェイクが認証フローの表面をカバーし、これらは組み合わせられます。

### `Mail::fake()`

プロセスローカルな捕捉用トランスポートをインストールします。ガードの生存期間中のあらゆる送信は、外へ出て行く代わりに、インメモリのバッファに収まります:

```rust
use suprnova::mail::Mail;

#[tokio::test]
async fn send_link_dispatches_email() {
    let fake = Mail::fake();
    // … フローを駆動する …
    EmailVerification::send_link(&user, "https://app.example.com/verify")
        .await
        .unwrap();
    fake.assert_sent(|m| {
        m.to.iter().any(|a| a.email == "alice@example.com")
            && m.subject.contains("Verify")
    });
    fake.assert_sent_count(1);
}
```

`MailFake` は、`assert_sent`、`assert_not_sent`、`assert_sent_count`、そして生の `captured()` と `count()` のアクセサを公開します。ガードがドロップすると、以前に束縛されていたトランスポートが復元されます - フェイクと明示的なトランスポートの束縛を織り交ぜるテストが、状態を漏らすことはありません。

### `EventFacade::fake()`

同じ形ですが、イベント向けです:

```rust
use suprnova::auth_flows::events::EmailVerified;
use suprnova::events::testing::assert_dispatched;
use suprnova::EventFacade;

#[tokio::test]
async fn verify_fires_email_verified_event() {
    let _guard = EventFacade::fake();
    // … フローを駆動する …
    EmailVerification::verify(&token).await.unwrap();
    assert_dispatched::<EmailVerified>(|e| !e.user_id.is_empty());
}
```

このフェイクは、リスナーを呼び出すことなくディスパッチされたイベントを記録するため、外部のサービスと話すリスナーは、テストの間に発火しません。対になる `assert_not_dispatched::<E>(pred)` は否定を主張し、`dispatched_count::<E>(pred)` は、より細かいアサーションのために生のカウントを返します。

### メール確認とパスワードリセットの統合テスト

メール確認テストでは `auth_flow_tokens` を作成し、`UserProvider` を登録し、認証済みトークン所有者を確立し、`MAIL_FROM` を設定して、`Mail::fake()` の下でファサードを駆動します。

パスワードリセットテストでは `MagnetarPasswordAuthEngine` のテストアダプターをインストールし、発行、消費しないチェック、原子的完了、セッション失効、および使い捨ての挙動をアサートします。

正規のソース例:

- アクター束縛された確認と使い捨てトークンについては `framework/tests/email_verify.rs`。
- Magnetarへの委譲と完了結果については `framework/tests/password_reset.rs`。
- 実際のデフォルトエンジン設定については `framework/tests/magnetar_default_engine.rs`。
- ロックアウトのライフサイクルについては `framework/tests/brute_force.rs`。
- 維持されるフレームワークTOTPチャレンジフローについては `framework/tests/two_factor_challenge_flow.rs`。
- rememberのローテーションと二重セッション束縛については `framework/tests/magnetar_remember_middleware.rs`。

プロセスグローバルなMagnetarインストールは、意図的に一度限りです。異なるエンジンを必要とするテストは別々の統合テストバイナリーに置くか、テストアダプターをバイナリー全体で一度だけインストールしてください。

## リファレンス

| シンボル | 目的 |
|---|---|
| `suprnova::auth_flows::EmailVerification` | `send_link`、`resend`、`check`、およびアクター束縛された `verify`。`verify` はユーザーIDを返します。 |
| `suprnova::auth_flows::EnsureEmailVerifiedMiddleware` | 403 JSONには `new()`、ブラウザーまたはInertiaリダイレクトには `redirect_to(path)`。 |
| `suprnova::auth_flows::PasswordReset` | Magnetar を優先し、フレームワークの `auth_flow_tokens` を介して検証済みアカウント用の `UserProvider` にフォールバックするリセット。 |
| `suprnova::MustVerifyEmail` | フレームワークの確認ファサードのためのアプリケーションユーザー契約。 |
| `suprnova::auth_flows::token_store::create_auth_flow_tokens_table` | フレームワークの確認トークンのためのSeaORMテーブル定義。 |
| `suprnova::auth_flows::BruteForce` | Magnetarに支えられたアカウントロックアウトファサード。 |
| `suprnova::auth_flows::LoginThrottleMiddleware` | アカウントがロックされている場合、ログインハンドラの前に429を返すHTTPミドルウェア。 |
| `suprnova::auth_flows::TwoFactor` | 維持されるフレームワークTOTPの登録、検証、回復、およびチャレンジのファサード。 |
| `suprnova::auth_flows::TwoFactorUser` | フレームワークTOTPファサードのためのアプリケーションユーザーブリッジ。 |
| `suprnova::auth_flows::TwoFactorChallengeMiddleware` | フレームワークTOTPチャレンジを待つセッションのためのゲート。 |
| `suprnova::auth_flows::remember_me` | レガシーなフレームワークrememberモジュールの互換再公開。 |
| `suprnova::MagnetarConfig` / `suprnova::init_magnetar` | デフォルトMagnetarエンジンの設定と一度限りのインストール。 |
| `suprnova::auth_flows::events::*` | 認証ライフサイクルイベント。 |

## 次のステップ

- [認証](authentication.md) - 認証ガード、プロバイダー、`Auth` ファサード、`AuthMiddleware`。
- [メール](mail.md) - `send_link` の呼び出しがディスパッチを通す、トランスポートの層。
- [イベント](events.md) - 9つの認証フローイベントに対するリスナーを登録すること。
- [レート リミット](rate-limiting.md) - 重層的な防御のために、`RateLimitMiddleware::ip_based` と `LoginThrottleMiddleware` を組にすること。
- [セッション](session.md) - `start_challenge` / `complete_challenge` がセッションidをローテーションするときに、何に触れるか。
