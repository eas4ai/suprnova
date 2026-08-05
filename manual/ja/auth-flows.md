# 認証フロー

`suprnova::auth_flows` は、[セッション認証](authentication.md)の上に乗るライフサイクルの層です。`auth::*` が「このリクエストは誰か」に答えるのに対して、`auth_flows::*` は、その問いの周りにあるすべてに答えます - メールアドレスが本物であることを証明し、パスワードを失ったときにそれを復旧し、クレデンシャルスタッフィングからそれを守り、第二要素でそれを保護することです。5つのフローが、1つの名前空間の下に出荷されます:

- `EmailVerification` - 使い捨ての確認トークンを発行し、確認し、消費します。`send_link` / `resend` は、[`Mail`](mail.md) ファサードを介して確認メールを発送し、`verify` は、設定済みのユーザープロバイダーを通じてユーザーを確認済みにします。
- `PasswordReset` - 列挙攻撃対策された `send_link`、消費しない `check`、そして `complete` です。`complete` は、設定済みのユーザープロバイダーを通じてパスワードをローテーションし、そのユーザーのすべてのセッションとremember-meの行を失効させ、`PasswordChangedMail` というセキュリティ通知を送ります。
- `BruteForce` + `LoginThrottleMiddleware` - toriiに支えられたロックアウトの状態と、ログインハンドラが呼び出される前に `429 Too Many Requests` でショートサーキットするHTTPミドルウェアです。
- `TwoFactor` - TOTPの登録、確認、検証、リカバリーコード、シークレットのローテーション、パスワードログインを第二要素でゲートする完全なチャレンジフロー、そして30秒のタイムステップ粒度でのリプレイ保護です。
- `remember_me` - 名前空間の一貫性のための、`crate::auth::remember`（DBの行 + bcrypt + 使い捨てローテーションの永続クッキー）の再公開です。

同じ名前空間には、2つのルートゲート用ミドルウェアが出荷されています:

- `EnsureEmailVerifiedMiddleware` - `AuthMiddleware` の後に合成され、`email_verified_at` に基づいてルートをゲートします。
- `TwoFactorChallengeMiddleware` - `AuthMiddleware` の手前に合成され、保留中の2FAチャレンジを持つセッションを、ログインページではなくチャレンジフォームへリダイレクトします。

あらゆるトランザクションメッセージは、[`Mail`](mail.md) ファサードを通じて配送されます。toriiの任意の `mailer` フィーチャーは、`framework/Cargo.toml` で意図的に無効化されています: torii の内部で2つ目のメールスタックを走らせると、テレメトリが分裂し、トランスポートの設定の表面が二重になり、アプリに2つの「from」アドレスを配線させることになってしまいます。

### 状態がどこに存在するか

メール確認とパスワードリセットは**プロバイダーに依存しません**。確認とリセットのトークンは、フレームワーク自身の `auth_flow_tokens` テーブル（使い捨て、SHA-256でハッシュ化）に存在し、ユーザーのルックアップ + 変更は、アプリが登録したどの [`UserProvider`](authentication.md) を通じても行われます - `Auth::user` が解決する対象と同じプロバイダーです。これら2つのフローのために初期化すべきグローバルな認証インスタンスはありません: 新しくスキャフォルドされたアプリには、すでに `EloquentUserProvider<User>` が束縛されており、`EmailVerification` と `PasswordReset` が必要とするのはそれだけです。

toriiは、それに本当に依存しているフロー - アカウント単位のブルートフォースのロックアウトカウンター、OAuth / パスキー / WebAuthnのセレモニー、そしてセッションプール - のセキュリティ状態を、引き続き所有します。Suprnovaは、すべてのフローをまたぐ横断的関心事 - 送信メール、イベントのディスパッチ、2FAのTOTPテーブル、remember-meのクッキー、そしてHTTPミドルウェア - を所有します。アプリケーションコードが触れるのは、常に `suprnova::auth_flows::*` だけです。Laravelは、同等の表面をFortifyへ折り込みます。Suprnovaは、モデルのトレイト（`MustVerifyEmail` / `CanResetPassword`）とトークンストアをフレームワークの中に保つため、フローはどのユーザーバックエンドに対しても機能します。

## フローをまたぐ失敗のセマンティクス

あらゆるファサードは、1つの順序のルールに従います: 永続的な状態変更が先にコミットされ、それから通知の副作用が発火します。ミューテーションの後のリスナーのパニック、一時的なメールトランスポートの失敗、あるいはディスパッチャーのエラーは、そのミューテーションを巻き戻すことができません。

- `EmailVerification::verify` は、`EmailVerified` を発火する前に、トークンを消費し、プロバイダーを通じてユーザーを確認済みにします。
- `PasswordReset::complete` は、まずトークンを消費し、プロバイダーを通じてパスワードをローテーションし、それからそのユーザーのすべてのセッションとremember-meの行を失効させ（失敗時はログに記録されるだけで表面化しません）、それから結果を待たずに `PasswordChangedMail` を送出し、それから `PasswordResetCompleted` を発火します。
- `BruteForce::unlock_account` は、`AccountUnlocked` を発火する前に、ロック解除をコミットします。
- `TwoFactor::confirm` は、`TwoFactorEnrolled` を発火する前に `confirmed_at` を打刻します。`TwoFactor::disable` は、`TwoFactorDisabled` を発火する前に行を削除します。`TwoFactor::complete_challenge` は、標準の `auth::Login` + `auth::Authenticated` の組を発行し、続けて `TwoFactorChallenged` を発行する前に、pendingをauthedへ格上げします。

永続性を必要とするリスナーは、自分の作業をバッファリングすべきです（リスナー本体からジョブをキューへ入れます）。ファサード自身は、決してリトライしません。

## ブートストラップ

メール確認とパスワードリセットはプロバイダーに支えられており、**toriiを一切必要としません**。ブルートフォース対策と2FAは、それでもtoriiを必要とします。あなたが使うフローが必要とするものだけを配線してください - それらは独立しています。

### メール確認 + パスワードリセット

3つのものがあり、スキャフォルドされたアプリはすべてすでに持っています:

1. **認証フローの表面を実装するユーザープロバイダー。** `bootstrap.rs::register()` の中で、`EloquentUserProvider<User>`（`Auth::user` が解決する対象と同じプロバイダー）を `dyn UserProvider` の束縛として登録してください。どちらのファサードも、有効なプロバイダーを内部で解決します。呼び出し箇所でインスタンスが渡されることはありません。

   ```rust
   use suprnova::{bind, EloquentUserProvider};
   use suprnova::auth::UserProvider;
   use crate::models::users::User;

   bind!(dyn UserProvider, EloquentUserProvider::<User>::new());
   ```

2. **あなたの `User` に対する、2つのモデルトレイト。** `EloquentUserProvider<User>` が認証フローのメソッド（`retrieve_by_email` / `mark_email_verified` / `set_password` / `is_email_verified`）を実装するのは、`User` が `MustVerifyEmail` と `CanResetPassword` の両方を実装している場合だけです - これらは、Laravelの `MustVerifyEmail` / `CanResetPassword` の契約に相当するSuprnovaの仕組みです:

   ```rust
   use chrono::{DateTime, Utc};
   use suprnova::{Authenticatable, CanResetPassword, MustVerifyEmail};

   impl MustVerifyEmail for User {
       fn email(&self) -> &str {
           &self.email
       }
       fn email_verified_at(&self) -> Option<DateTime<Utc>> {
           self.email_verified_at
       }
       fn set_email_verified_at(&mut self, v: Option<DateTime<Utc>>) {
           self.email_verified_at = v;
       }
       fn name(&self) -> Option<&str> {
           Some(&self.name)
       }
   }

   impl CanResetPassword for User {
       fn email_for_reset(&self) -> &str {
           &self.email
       }
       fn set_password_hash(&mut self, hash: &str) {
           // 値はすでにハッシュ化された状態で届きます - そのまま保存してください。
           self.password = hash.to_string();
       }
   }
   ```

   `is_email_verified()` には、タイムスタンプを追跡するデフォルト（`email_verified_at().is_some()`）があり、`name()` はデフォルトで `None` です - メールの中でユーザーを名前で呼びかけたい場合は、これをオーバーライドしてください。

3. **あなたのマイグレーターにある、2つのカラム / テーブル。** `users` テーブルは、null許容の `email_verified_at` タイムスタンプを必要とします（プロバイダーは `is_email_verified` でそれを読み取り、`mark_email_verified` でそれに打刻します）。そして、フレームワークの使い捨ての `auth_flow_tokens` テーブルが、確認 / リセットのトークンを保持します。フレームワークはトークンテーブルの `CREATE` を出荷します。あなたのマイグレーターにそれを一覧してください:

   ```rust
   use sea_orm_migration::prelude::*;

   #[async_trait::async_trait]
   impl MigrationTrait for AuthFlowTokens {
       async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
           manager
               .create_table(
                   suprnova::auth_flows::token_store::create_auth_flow_tokens_table(),
               )
               .await
       }

       async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
           manager
               .drop_table(Table::drop().table(Alias::new("auth_flow_tokens")).to_owned())
               .await
       }
   }
   ```

   `email_verified_at` を、あなた自身のカラムマイグレーションで `users` に追加してください（null許容の `timestamp_with_time_zone`）。`NULL` は未確認を意味するため、既存の行は正しくバックフィルされます。

トークンは使い捨てであり、保存時にはSHA-256でハッシュ化されています - データベースのダンプが、使える平文のトークンを生み出すことは決してありません。デフォルトのTTLは、メール確認については**24時間**、パスワードリセットについては**15分**です。

### ブルートフォース + 2FA: toriiを配線する

`BruteForce` / `LoginThrottleMiddleware` と `TwoFactor` はtoriiに支えられています - これらは、`bootstrap.rs::register()` の中で、`DB::init` の後に初期化されるグローバルなtoriiインスタンスを必要とします。（OAuth、パスキー、WebAuthnのセレモニーも同じインスタンスを経由します - [認証](authentication.md)を参照してください。）

```rust
use suprnova::torii_integration::{init_torii, ToriiConfig};
use suprnova::DB;

pub async fn register() -> Result<(), suprnova::FrameworkError> {
    DB::init().await?;

    let conn = DB::connection()?.inner().clone();
    init_torii(ToriiConfig::from_sea_orm(conn)).await?;

    Ok(())
}
```

`init_torii` はべき等です。`OnceLock` による保護のおかげで、2回目の呼び出しは何もしないため、フィクスチャごとに `register()` に再度入るテストハーネスが、二重にマイグレーションすることはありません。テストのためには、`ToriiConfig::sqlite_in_memory()` に差し替えてください - これは、ランタイムをまたいで生き残る、共有キャッシュのインメモリデータベースを立ち上げます:

```rust
let config = ToriiConfig::sqlite_in_memory()
    .await?
    .apply_migrations(true);
init_torii(config).await?;
```

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
| `resend` | `resend(email: &str, base_url: &str) -> Result<()>` | 列挙攻撃対策: emailでユーザーをルックアップします。未知のアドレスはサイレントな `Ok(())` になります。 |
| `check` | `check(token: &str) -> Result<bool>` | 消費しません - ランディングページで呼んでも安全です。 |
| `verify` | `verify(token: &str) -> Result<String>` | 使い捨て: トークンを消費し、ユーザーを確認済みにし、ユーザーidを返します。 |

```rust
use suprnova::auth_flows::EmailVerification;

// 新規登録の直後、作られたばかりのユーザーが手元にある状態で:
EmailVerification::send_link(&user, "https://app.example.com/verify-email").await?;

// 任意のランディングページチェック - 消費しないため、ページを
// リフレッシュしてもトークンが失われません。
let valid: bool = EmailVerification::check(&token_str).await?;

// クリックスルーのハンドラは、トークンを消費してユーザーに打刻し、
// 確認済みユーザーのidを返します。
let user_id: String = EmailVerification::verify(&token_str).await?;
```

`verify` は成功時に `EmailVerified` を発火します - リスナーは、確認ハンドラに結合させることなく、追加の機能（ウェルカムメール、デフォルトのフォロー、「プロフィールを完成させましょう」というCTA）を解き放つのに正しい場所です。このイベントは、プロバイダーのユーザーidを運びます。

### resendエンドポイント（列挙攻撃対策）

`resend` はemailだけを受け取ります - ファサードは有効なプロバイダーを通じてユーザーをルックアップし、アカウントが存在する場合にはトークンを発行してメールを送ります。未知のemailはサイレントなno-opであり、それでも `Ok(())` を返します。ハンドラは、存在そのものによって分岐することは決してないため、探りを入れる呼び出し元は「送信した」と「そんなアカウントはない」を見分けることができません:

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

クリックスルーのハンドラは、クエリ文字列からトークンを取り出し、`verify` を呼び出します:

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

ハンドラは、ユーザーをルックアップする必要がありません - `verify` がトークンを消費し、プロバイダーを通じてユーザーを確認済みにし、ユーザーidを返し、`EmailVerified` を発火します。使い捨て: 同じトークンに対する2回目の `verify` はエラーを返します。

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

`PasswordReset` には3つの操作があります:

| メソッド | 署名 | 備考 |
|---|---|---|
| `send_link` | `send_link(email: &str, base_url: &str) -> Result<()>` | 列挙攻撃対策: emailでユーザーをルックアップします。未知のアドレスはサイレントな `Ok(())` になります。 |
| `check` | `check(token: &str) -> Result<bool>` | 消費しません - 新しいパスワードのフォームを描画する前に、トークンを確認してください。 |
| `complete` | `complete(token: &str, new_password: &str) -> Result<String>` | 使い捨て: トークンを消費し、パスワードをローテーションし、セッション + remember-meを失効させ、変更通知を送り、ユーザーidを返します。 |

```rust
use suprnova::auth_flows::PasswordReset;

// 「パスワードを忘れた」フォームからのものです。常にOk(())です - ファサードは
// ユーザーをルックアップし、アカウントが存在する場合にのみ送信します。
PasswordReset::send_link(&email, "https://app.example.com/reset").await?;

// 新しいパスワードのフォームを描画する前の、任意のランディングページチェックです。
let valid: bool = PasswordReset::check(&token).await?;

// クリックスルーのハンドラです。ユーザーが新しいパスワードを送信した後、
// トークンを消費してパスワードをローテーションし、ユーザーidを返します。
let user_id: String = PasswordReset::complete(&token, &new_password).await?;
```

`complete` は、プロバイダーへ渡す前に `new_password` をハッシュ化します - 事前にハッシュ化した値ではなく、平文を渡してください。空白のみ、または空のパスワードは、事前に `400` で拒否されます。

### 列挙攻撃対策

`send_link` は、レスポンスの形が、そのemailアドレスがアカウントを持っているかどうかを決して漏らさないように組み立てられています:

- これは常に `Ok(())` を返します。emailが存在しない場合、トークンは発行されず、メールも発送されず、`PasswordResetLinkSent` イベントも発火しません - しかし、その不在は戻り値の型を通じても表面化しないため、呼び出し元（そしてネットワークの観測者）は、「そんなアカウントはない」と「リンクを送信した」を見分けることができません。
- ドッグフードのコントローラーは、`send_link` を固定の200レスポンスボディと組み合わせているため、探りを入れる呼び出し元は、ステータスコード、レスポンスボディ、あるいはレスポンスのタイミングを通じて見分けることができません。

### `complete` の副作用

`complete` は、4つのステップを順番に実行します:

1. トークンを消費し（使い捨て）、設定済みのプロバイダーを通じてパスワードハッシュをローテーションします（この呼び出しを失敗させうる唯一のステップです）。
2. `crate::session::destroy_all_for_user` を介して、そのユーザーのすべてのセッション行を失効させます（ベストエフォート: 失敗は `tracing::warn!`）。
3. `crate::auth::remember::revoke_all_for_user` を介して、すべてのremember-me行を失効させます（ベストエフォート）。
4. 結果を待たずに `PasswordChangedMail` を送出し、それから `PasswordResetCompleted` を発火します。

盗まれたセッションと、捕捉されたremember-meクッキーは、それらが依存していた認証情報より長生きしてはなりません。失効は、ユーザー起点のものだけでなく、成功したリセットのたびに起きるため、セキュリティチームによる強制リセットも、活動中の攻撃者を蹴り出します。

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
- `Retry-After` ヘッダー - 秒数で、ロックアウトの `locked_until` から `LockoutStatus::retry_after_seconds` を介して計算されます。タイムスタンプが何らかの理由で存在しない場合は、`900`（15分 - toriiのデフォルトのロックアウト期間）にフォールバックします。
- ボディ: `"Account locked due to too many failed login attempts. Try again later."`

### バックエンドのエラーに対してフェイルオープンする

`get_lockout_status` が `Err`（一時的なデータベースの不調）を返す場合、ミドルウェアはリクエストを通します。下流のログインハンドラは、それから自分でその呼び出しを行い、フェイルクローズするかフェイルオープンするかを決められます。このミドルウェアは、可用性を優先する側に誤ります: 認証データベースに不調があるたびにログインのエンドポイントを落とすことは、ハンドラに直接その呼び出しをさせるよりも悪いことです。

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

toriiの `BruteForceProtectionConfig` は、デフォルトで**ロックアウトまでの失敗5回**と**15分のロックアウト期間**です。これらは、今日 `init_torii` が配線するものです。アプリごとの値を設定するには、toriiそれ自身の設定の表面に手を伸ばす必要があり、Suprnovaの `ToriiConfig` ビルダーからは公開されていません。このデフォルトは意図的に保守的です - それを緩めることを決める前に、「タイプミス5回で15分ロックされる」という状態を選んでみてください。

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

`user_id` は不透明なストレージキーです - 典型的には `torii::UserId.as_str()` ですが、安定したユーザー単位の識別子であれば何でも機能します。2FAのテーブルはこれにインデックスを張ります。あなたのユーザーテーブルへの外部キーはありません。

`email` は `otpauth://` URLの `account_name` セグメントへ折り込まれるため、認証アプリは、人間に読める形のラベル（例えば「MyCorp (alice@example.com)」）で、その行を描画します。

よくあるパターンは、あなたのユーザーモデルをラップする小さなニュータイプです:

```rust
use suprnova::auth_flows::TwoFactorUser;
use suprnova::torii_integration::User as ToriiUser;

struct AppUser2FA<'a> { user: &'a ToriiUser }

impl<'a> TwoFactorUser for AppUser2FA<'a> {
    fn user_id(&self) -> &str { self.user.id.as_str() }
    fn email(&self)   -> &str { &self.user.email }
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

2FAの `user_id` は、意図的に `String` です。もしそれが `i64`、`Uuid`、あるいは `torii::UserId` として型付けされていたら、2FAのテーブルは、フレームワークが最初に選んだ形へ永久に縛られてしまいます - 異なる形でユーザーを保存するアプリ（UUID対自動増分の整数、あるいはtoriiを一切使わないが2FAモジュールを望むアプリ）は締め出されてしまうでしょう。文字列的な `user_id` は、各アプリに、好きな安定したユーザー単位の識別子を選ばせます。その代償は、呼び出し箇所での1回の `.to_string()` です。LaravelのFortifyは、同等のカラムをEloquentの `User::id` に結び付けます - Suprnovaはそれを分離するため、`TwoFactor` は、Userの形をしたアクセサリーではなく、再利用可能なライフサイクルのプリミティブです。

## Remember-me

`suprnova::auth_flows::remember_me` は `suprnova::auth::remember` を再公開します - これは、セッション認証と並んですでに出荷されていた、永続クッキーのモジュールです。この再公開は、純粋に構成上のものです: 認証フローの形をしたものはすべて `auth_flows::*` の下に存在します。実装がこの名前空間より前からあった場合でもです。

出荷される設計です:

- **DBの行 + bcryptハッシュ** - 発行されたトークンはそれぞれ、`remember_tokens` テーブルに行を持ち、bcryptハッシュだけを保存します。平文は決して保存しません。データベースのダンプが、再認証できる認証情報を生み出すことはできません。
- **使い捨てのローテーション** - 成功した検証は、マッチした行をDELETEし、新しい行を発行します。捕捉されたクッキーは再利用できません。攻撃者と被害者がそれを使おうと競合した場合、負けたほうは行が消えているのを見て、認証に失敗します。
- **失効** - `revoke_all_for_user` は、1回のDELETEで、あるユーザーのすべての行を消し去ります。`Auth::logout` はこれを連鎖させるため、本当のログアウトは実際に永続的な状態をクリアします。`PasswordReset::complete` も同じことを行うため、パスワードリセットは、既存のすべての永続クッキーを無効にします。
- **刈り取り** - `prune_expired` は、期限切れの行をスケジュールに従って片付けます。

実際には、フレームワークのセッションミドルウェアが重い処理を行います。典型的なアプリは、`remember_me` モジュールを直接呼び出しません。[認証](authentication.md)のドキュメントが、ユーザー向けの表面 - `Auth::login` の `remember` フラグ、クッキー名、有効期間のつまみ - を扱っています。

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

### メール確認 + パスワードリセットの統合テスト

確認 / リセットのテストはtoriiを必要としません - インメモリのデータベースに `auth_flow_tokens` テーブルを用意し、プロバイダーを登録し、`MAIL_FROM` を設定し、`Mail::fake()` の下でファサードを駆動してください。フレームワーク自身のテストは、`create_auth_flow_tokens_table()` から直接テーブルを発行します:

```rust
use sea_orm::ConnectionTrait;
use suprnova::auth_flows::token_store::create_auth_flow_tokens_table;
use suprnova::mail::Mail;
use suprnova::testing::TestDatabase;

#[tokio::test]
#[serial_test::serial]
async fn send_link_mails_a_token_link() {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    let conn = db.conn();
    let stmt = create_auth_flow_tokens_table();
    conn.execute(conn.get_database_backend().build(&stmt))
        .await
        .unwrap();

    // ファサードは MAIL_FROM を読み取ります（フェイルクローズ）。テストのためにそれを設定します。
    // SAFETY: `#[serial]` によって直列化されています - 並行する観測者はいません。
    unsafe { std::env::set_var("MAIL_FROM", "test-mailer@example.com"); }

    let fake = Mail::fake();
    // … EmailVerification::send_link(&user, base) を駆動する …
    fake.assert_sent_to("ada@example.com");
}
```

プロバイダーに支えられた経路（`resend` / `verify` / `complete`）は、さらに `dyn UserProvider` の束縛を登録するため、ルックアップ + 変更が解決されます - `framework/tests/email_verify.rs` と `framework/tests/password_reset.rs` を参照してください。

### ブルートフォース + 2FAのテストのための `ToriiConfig::sqlite_in_memory()`

ブルートフォースと2FAのテストは、インメモリのSQLiteデータベース上に新しいtoriiを立ち上げます。`framework/tests/` にあるテストファイルの例は、共有ランタイム + `once_cell::sync::Lazy<()>` というパターンを使って、テストをまたいでコストを償却し、さらに `#[serial]` を使って、`Mail::fake()` を織り交ぜるテストの間で、プロセスグローバルなメールトランスポートを安定させます:

```rust
use once_cell::sync::Lazy;
use serial_test::serial;
use tokio::runtime::Runtime;
use suprnova::torii_integration::{init_torii, ToriiConfig};

static RT: Lazy<Runtime> = Lazy::new(|| Runtime::new().expect("tokio runtime"));

static SETUP: Lazy<()> = Lazy::new(|| {
    RT.block_on(async {
        let config = ToriiConfig::sqlite_in_memory()
            .await
            .expect("sqlite in-memory connection")
            .apply_migrations(true);
        init_torii(config).await.expect("init_torii");
    });
});

#[test]
#[serial]
fn my_test() {
    Lazy::force(&SETUP);
    RT.block_on(async {
        // … ここで Mail::fake() / EventFacade::fake() を使う …
    });
}
```

正規の例です - あなた自身のものを書くときは、これらからコピーしてください:

- `framework/tests/email_verify.rs` - verifyトークンの往復、`send_link` の末尾スラッシュの切り取り、件名/HTMLに対する `Mail::fake()` のアサーション。
- `framework/tests/password_reset.rs` - 新しいパスワードでの認証を伴うリセットの往復、未知のemailに対する列挙攻撃対策、`complete` が再利用されたトークンを拒否すること。
- `framework/tests/brute_force.rs` - ロックアウトの完全なライフサイクル、`AccountLocked` は遷移ごとに1回発火すること、`unlock_account` が `was_locked` を返すこと。
- `framework/tests/two_factor.rs` - otpauth URLから計算された実際のTOTPコードによる、enroll → confirm → verifyの完全な流れ、リカバリーコードの使い捨て、再登録がシークレットを上書きすること、2つの並行した検証をまたぐリプレイの拒否。
- `framework/tests/two_factor_challenge_flow.rs` - セッションのローテーション、remember-meの再発行、イベントのディスパッチを伴う、エンドツーエンドのチャレンジフロー。
- `framework/tests/email_verified_middleware.rs` と `two_factor_challenge_middleware.rs` - ミドルウェアのレスポンスの形（403 JSON対302対409 + X-Inertia-Location）。

## リファレンス

| シンボル | 目的 |
|---|---|
| `suprnova::auth_flows::EmailVerification` | `send_link`、`resend`、`check`、`verify` - プロバイダーに支えられている。`verify` はユーザーidを返す。 |
| `suprnova::auth_flows::EnsureEmailVerifiedMiddleware` | 403 JSONには `new()`、302 / 409 + X-Inertia-Locationには `redirect_to(path)`。設定済みのプロバイダーの `is_email_verified` をチェックする（フェイルクローズ）。 |
| `suprnova::auth_flows::PasswordReset` | `send_link`、`check`、`complete` - プロバイダーに支えられている。`complete` はユーザーidを返す。 |
| `suprnova::MustVerifyEmail` / `suprnova::CanResetPassword` | `EloquentUserProvider` の背後にあるユーザーが実装するモデルトレイト。これにより、確認 / リセットのファサードは、そのemailを読み、確認のタイムスタンプ / パスワードハッシュを書ける。 |
| `suprnova::auth_flows::token_store::create_auth_flow_tokens_table` | `auth_flow_tokens` のためのSeaORM `CREATE TABLE`。あなたのマイグレーターに一覧する。 |
| `suprnova::auth_flows::BruteForce` | `record_failed_attempt`、`reset_attempts`、`get_lockout_status`、`is_locked`、`unlock_account`。 |
| `suprnova::auth_flows::LoginThrottleMiddleware` | 対象のアカウントがロックされている場合、ハンドラより前に429にするHTTPミドルウェア。 |
| `suprnova::auth_flows::TwoFactor` | `enroll`、`re_enroll`、`confirm`、`verify`、`consume_recovery_code`、`regenerate_recovery_codes`、`is_enabled`、`is_enabled_by_id`、`start_challenge`、`pending_user_id`、`cancel_challenge`、`complete_challenge`、`disable`。 |
| `suprnova::auth_flows::TwoFactorUser` | アプリのユーザーモデルを2FAファサードへ橋渡しするトレイト。 |
| `suprnova::auth_flows::EnrollmentResponse` | `TwoFactor::enroll` の戻り値 - `otpauth_url`、`qr_code_svg`、`recovery_codes`。 |
| `suprnova::auth_flows::TwoFactorChallengeMiddleware` | 403 JSONには `new()`、302 / 409 + X-Inertia-Locationには `redirect_to(path)`。`AuthMiddleware` の手前に合成する。 |
| `suprnova::auth_flows::two_factor::migration::Migration` | `two_factor_credentials` のためのSeaORMマイグレーション。あなたの `Migrator::migrations()` に一覧する。 |
| `suprnova::auth_flows::two_factor::migration_replay::Migration` | `last_used_timestep`（TOTPのリプレイ保護）のためのカラム追加。create-tableのマイグレーションの後に一覧する。 |
| `suprnova::auth_flows::remember_me` | `suprnova::auth::remember` の再公開。 |
| `suprnova::auth_flows::events::*` | 9つのイベント - [イベント](#イベント)を参照。 |
| `suprnova::auth_flows::EmailVerificationMail` | トランザクションのMailable。件名は `"Verify your email for {APP_NAME}"`。 |
| `suprnova::auth_flows::PasswordResetMail` | トランザクションのMailable。件名は `"Reset your {APP_NAME} password"`。 |
| `suprnova::auth_flows::PasswordChangedMail` | セキュリティ通知のMailable。件名は `"Your {APP_NAME} password was changed"`。 |
| `suprnova::torii_integration::ToriiConfig` | Toriiのブートストラップ設定。本番には `from_sea_orm(conn)`、テストには `sqlite_in_memory()`。 |
| `suprnova::torii_integration::init_torii` | べき等なグローバル初期化。`bootstrap.rs::register()` から一度だけ呼び出す。 |

## 次のステップ

- [認証](authentication.md) - 認証ガード、プロバイダー、`Auth` ファサード、`AuthMiddleware`。
- [メール](mail.md) - `send_link` の呼び出しがディスパッチを通す、トランスポートの層。
- [イベント](events.md) - 9つの認証フローイベントに対するリスナーを登録すること。
- [レート リミット](rate-limiting.md) - 重層的な防御のために、`RateLimitMiddleware::ip_based` と `LoginThrottleMiddleware` を組にすること。
- [セッション](session.md) - `start_challenge` / `complete_challenge` がセッションidをローテーションするときに、何に触れるか。
