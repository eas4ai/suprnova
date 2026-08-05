# OAuth、Apple とマジックリンク ログイン

Suprnovaは、`Auth` ファサードの裏に、toriiに支えられた3つのログイン方式を出荷します: **汎用OAuth**（GitHub、Google、あるいは任意のOIDC/OAuth2プロバイダー）、**Sign in with Apple**、そして**パスワードレスのマジックリンク**です。これらは1つの前提条件（`init_torii` とセレモニーのマイグレーション）と、同じファサードの形 - `Auth::oauth(provider)` / `Auth::magic_link()` - を共有し、いずれもルートを出荷しません: 薄いコントローラー（start + callback）を自分で追加すれば、フレームワークがCSRFのstate、PKCE、トークン交換、アイデンティティの検証、ユーザーのアップサート、セッションの発行を行います。

この表面全体は `framework/src/torii_integration/` に存在します。これについては、フレームワークによる環境変数の契約が**一切ありません** - あらゆる認証情報はプログラムから渡されます（自分で環境から取り出してください）。この章の例が `std::env::var(...)` を使っているのは、あなたのシークレットがどこへ流れるかを示すためだけです。

## 前提条件

1. **起動時に一度だけtoriiを初期化する** - これがユーザーのアップサートとセッションの作成を裏付けます:

   ```rust
   use suprnova::{init_torii, ToriiConfig};

   // bootstrap::register() の内部、DB::init() の後で
   init_torii(ToriiConfig::from_sea_orm(db_conn)).await?;
   ```

2. **セレモニーのマイグレーションを実行する。** OAuthとAppleは、短命な（10分間の）CSRFの `state` + PKCEのセレモニーを `auth_ceremony_tokens` テーブルに一時保存します。マイグレーション `m20251209_000000_create_auth_ceremony_tokens_table` を、あなたの `Migrator` に登録してください（スターターキットにはすでに含まれています）。任意で、古くなった行をGCするために `suprnova::torii_integration::ceremony::prune_expired()` をスケジュールしてください。

3. **OAuthの *start* ルートに `SessionMiddleware` を。** `begin()` は `state` をセッションに書き込みます。セッションのない呼び出しは500で失敗します。

マジックリンクが必要とするのは手順1だけです。

## 汎用OAuth（GitHub、Google、カスタム）

### プロバイダーを設定する

各プロバイダーを、起動時に一度だけ登録してください。レジストリはプロセスグローバルかつべき等であるため、同じプロバイダーを再登録すると、単に設定が置き換わるだけです:

```rust
use suprnova::Auth;
use suprnova::torii_integration::oauth::OAuthProviderConfig;

Auth::oauth("github").configure(OAuthProviderConfig {
    client_id: std::env::var("GITHUB_CLIENT_ID")?,
    client_secret: std::env::var("GITHUB_CLIENT_SECRET")?,
    redirect_url: "https://app.example.com/auth/oauth/github/callback".into(),
    scopes: vec!["user:email".into()],
    endpoints_override: None,   // None → 組み込みの既知のテーブル
    apple_key_pair: None,       // Appleのみ。GitHub/GoogleではNoneのままにしてください
    apple_team_id: None,        // Appleのみ
});
```

既知のauthorize/token/userinfoのエンドポイントは、`github`、`google`、`apple` に対して組み込まれています。それ以外のプロバイダー - あるいは自前でホストするサーバーやテストサーバー - については、自分で用意してください:

```rust
use suprnova::torii_integration::oauth::EndpointOverrides;

Auth::oauth("gitlab").configure(OAuthProviderConfig {
    client_id: /* … */,
    client_secret: /* … */,
    redirect_url: /* … */,
    scopes: vec!["read_user".into()],
    endpoints_override: Some(EndpointOverrides {
        authorize: "https://gitlab.com/oauth/authorize".into(),
        token: "https://gitlab.com/oauth/token".into(),
        userinfo: "https://gitlab.com/api/v4/user".into(),
        emails: None,   // プライベートなプライマリのための、GitHubスタイルの /emails フォールバック
    }),
    apple_key_pair: None,
    apple_team_id: None,
});
```

### フローを開始する（authorize URL）

```rust
// GET /auth/oauth/github/start （ルートは必ず SessionMiddleware を運ぶこと）
let kickoff = Auth::oauth("github").begin().await?;
// kickoff.authorization_url - ブラウザをここへリダイレクトする
// kickoff.state - CSRFのstate。すでにセッションに保存済み
```

`begin()` はCSRFの `state`（UUID v4）と、RFC 7636のPKCEベリファイア/S256チャレンジを発行し、セレモニーを記録し（TTLは10分）、プロバイダーのauthorize URLを返します。ユーザーを `authorization_url` へリダイレクトしてください。

### フローを完了する - `verify` 対 `complete`

コールバックでは、2つのエントリーポイントがあります（0.5.4で分割されました）。あなたの `users` テーブルがtoriiのスキーマ**である**かどうかで選んでください:

| メソッド | 戻り値 | 副作用 | 使うべきとき |
|---|---|---|---|
| `verify_oauth_identity(code, state)` | `OAuthIdentity { provider, subject, email, name }` | **なし** - セレモニーを検証し、コードを交換し、userinfoを取得し、検証済みのメールと安定した `subject` を抽出します。ユーザーもセッションもありません。 | あなたのアプリが自分の `users` テーブルを所有し、ユーザーの検索・作成を自分で行いたい場合。 |
| `complete(code, state)` | `(User, Session)` | ユーザーをtoriiへアップサートし（`get_or_create_user`）、セッションを発行します。 | あなたの `users` テーブルがtoriiのスキーマである場合。 |

```rust
// カスタムのusersテーブル:
let id = Auth::oauth("github").verify_oauth_identity(&code, &state).await?;
// id.subject は安定したプロバイダーID。id.email は検証済みかNoneのいずれか。
let user = my_users::upsert(id.provider, id.subject, id.email, id.name).await?;

// …あるいは、toriiに支えられた形:
let (user, session) = Auth::oauth("github").complete(&code, &state).await?;
```

`verify` が返す `email` は、常に*検証済み*のアドレスです（OIDCの `email_verified`、検証済みとして扱われるGitHub、あるいは `/emails` フォールバック）。未検証または存在しないメールは `None` として返ってきて、繰り返しのログインは `subject` で解決されます。

### あなたが追加するルート

フレームワークはOAuthのルートを一切提供しません - 2つの薄いハンドラを配線してください（スターターキットにある既存の `auth_verify` / `auth_reset` コントローラーの形を反映させます）:

```rust
// start - プロバイダーへリダイレクトする
get!("/auth/oauth/{provider}/start", controllers::oauth::start),
// callback - GitHub/Googleは GET ?code&state を使う
get!("/auth/oauth/{provider}/callback", controllers::oauth::callback),
```

（少なくとも）`/start` ルートを `SessionMiddleware` の後ろに置いてください。

## Sign in with Apple

Appleも同じファサードです - `Auth::oauth("apple")` - ただし、いくつかのApple特有のルールが組み込まれています:

- **コールバックは `POST` です。** Appleは `response_mode=form_post` を使うため、リダイレクトは `code` + `state` をクエリパラメータではなくフォームボディで届けます。Appleのコールバックは `post!` ルートとして登録し、フィールドをフォームから読み取ってください。
- **PKCEはありません。** Appleは `code_challenge` を拒否するため、authorize URLはそれを省略します（代わりにクライアントシークレットは署名済みのJWTになります）。
- **`client_secret` は使われません** - `String::new()` のままにしてください。Suprnovaは、トークン交換のたびに、あなたの `.p8` キーから短命なJWTクライアントシークレットを発行します。
- **IDトークンはAppleのJWKS（RS256）に対して検証されます** - 0.5.6以降、構造的に信頼されるのではなく。

### あなたのApple keyを渡す - `AppleKeyPair`

`AppleKeyPair` は、アプリのために再公開されている唯一のApple型です（そのため、`apple` への直接の依存は不要です）。あなたの `.p8` 署名キーからこれを構築してください:

```rust
use suprnova::torii_integration::oauth::AppleKeyPair;

let key = AppleKeyPair::from_file(
    &std::env::var("APPLE_KEY_ID")?,   // Apple の *Key ID*（Team IDではない）
    &std::env::var("APPLE_P8_PATH")?,  // AuthKey_XXXXXX.p8 へのパス
)?;
// または: AppleKeyPair::from_base64(key_id, b64)  /  from_pem_bytes(key_id, bytes)
```

### Appleを設定する

```rust
use suprnova::torii_integration::oauth::OAuthProviderConfig;

Auth::oauth("apple").configure(OAuthProviderConfig {
    client_id: std::env::var("APPLE_CLIENT_ID")?,  // あなたのServices ID
    client_secret: String::new(),                  // 使われない - キーから発行される
    redirect_url: "https://app.example.com/auth/apple/callback".into(),
    scopes: vec!["email".into(), "name".into()],
    endpoints_override: None,
    apple_key_pair: Some(key),
    apple_team_id: Some(std::env::var("APPLE_TEAM_ID")?),  // 10文字のTeam ID
});
```

### Appleのフローを完了する

汎用OAuthと同じ分割です。`complete` はアップサート + セッションを行い、verifyの経路はカスタムのusersテーブルのために `AppleIdentity` を返します:

```rust
// POST /auth/apple/callback - code + state をFORMボディから読み取る
let (user, session) = Auth::oauth("apple").complete(&code, &state).await?;

// …あるいは、カスタムのusersテーブル:
let id = Auth::oauth("apple").verify_apple_identity(&code, &state).await?;
// id: AppleIdentity { provider, subject, email, email_verified, is_private_email }
```

`AppleIdentity.email` は、Appleがそれを検証済みだと主張する場合にのみ `Some(_)` になります。未検証のメールは、アイデンティティが構築される前に（401で）拒否されます。`is_private_email` は、ユーザーがAppleのプライベートリレーアドレスを選んだ場合に設定されます - リレーアドレスが手に入る唯一のメールであるため、安定したキーとして `subject` を永続化してください。

## マジックリンク ログイン

パスワードレスのメールログインで、toriiに支えられ、`Auth::magic_link()` を通じて行われます。フレームワークはトークンを発行・検証しますが、リンクをメールで送るのは**あなた**です（フレームワーク自身は決してメールを送信しません）。これは[メール](mail.md)の章ときれいに組み合わさります。

```rust
use suprnova::Auth;

// POST /auth/magic - リンクをリクエストする
let token = Auth::magic_link()
    .send("alice@example.com", "https://app.example.com/auth/magic")
    .await?;
// リンクを構築し、自分でメールしてください:
Mail::to("alice@example.com")
    .send(MagicLink { url: format!("https://app.example.com/auth/magic?token={token}") })
    .await?;

// GET /auth/magic?token=… - それを消費する（使い捨て。2回目の呼び出しは失敗する）
let (user, session) = Auth::magic_link().consume(&token).await?;
```

ユーザーは初回の使用時に自動作成されます。`send` は**平文**のトークンを返すため、URLの形と配送はあなたが制御します。

> **注意 - `TokenPurpose::MagicLink`。** `auth_flows` の `TokenPurpose` enumには `MagicLink` バリアント（0.5.5で追加）がありますが、これは汎用の `TokenStore` のための*予約済みの判別子*です - 組み込みのフローがそれを消費することはありません。実際に動作し、サポートされているマジックリンクの経路は、上記の `Auth::magic_link()` です。`TokenPurpose::MagicLink` に手を伸ばすのは、`auth_flow_tokens` テーブルの上で独自のフローを自作している場合だけにしてください。

## 設定についての注記

これらのメソッドのどれも、フレームワークの環境変数を読み取りません - プロバイダーID、シークレット、リダイレクトURL、Appleのキーは、すべてプログラムから `configure(...)` に渡されます。好きな方法で読み込み（`std::env::var`、型付きの設定構造体、シークレットマネージャー）、`bootstrap` の中で一度だけプロバイダーを登録してください。これによって、固定された環境変数の命名規則を強制する代わりに、マルチテナント / デプロイ単位のプロバイダー設定を第一級のものにしています。

## リファレンス

- ファサードのエントリーポイント: `Auth::oauth(provider)`、`Auth::magic_link()`（`suprnova::Auth`）
- 設定: `suprnova::torii_integration::oauth::{OAuthProviderConfig, EndpointOverrides, AppleKeyPair}`
- OAuthの結果: `OAuthKickoff { authorization_url, state }`、`OAuthIdentity { provider, subject, email, name }`、`AppleIdentity { provider, subject, email, email_verified, is_private_email }`
- Bootstrap: `suprnova::{init_torii, ToriiConfig}`
- セレモニーストア: `auth_ceremony_tokens` テーブル + `suprnova::torii_integration::ceremony::prune_expired()`

## 次のステップ

- [認証](authentication.md) - 認証ガード、プロバイダー、そしてこれらのフローがセッションを作成する対象となる `Authenticatable` ユーザーモデル
- [認証フロー](auth-flows.md) - メール確認、パスワードリセット、2FA
- [メール](mail.md) - マジックリンクのメールを送ること（そして送信元の設定である `MAIL_FROM` / `MAIL_FROM_NAME`）
- [セッション](session.md) - 返される `Session` が何であり、それがどのように永続化されるか
