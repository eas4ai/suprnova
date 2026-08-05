# 設定

Suprnova は環境変数から設定を読み込み（開発時は `.env` から、本番環境ではプロセス環境から）、コードに 2 つの形式で公開します。

1. **直接的な環境変数アクセス** - `env::env`、`env_required`、`env_optional` を使用した単発の読み込み
2. **型付き設定構造体** - `Config::register` / `Config::get` を使用し、複数回読み込む任意の値に対して強い型付けを行う

フレームワーク自体が少数の環境変数を読み込み（`APP_KEY`、
`APP_ENV`、`DATABASE_URL` など）、その他はアプリケーション側で使用します。

## `.env` ファイル

`suprnova new` はアプリケーションの起動に必要な値を含むスターター `.env` を書き込みます。

```env
APP_NAME="my-app"
APP_ENV=local                # local, development, staging, production, testing, …
APP_DEBUG=true               # detailed error pages + verbose logs
APP_URL=http://localhost:8765

# 32-byte AES-256 key (URL-safe base64, no padding). Encrypts session
# cookies, pagination cursors, and anything via `suprnova::Crypt`.
# Generated at scaffold time. Rotate with `suprnova key:generate`.
APP_KEY=<32-byte base64>

SERVER_HOST=127.0.0.1
SERVER_PORT=8765
VITE_PORT=5765

# Database - SQLite by default; swap to postgres://user:pass@host/db
DATABASE_URL=sqlite://./database.db
DB_MAX_CONNECTIONS=10
DB_MIN_CONNECTIONS=1
DB_CONNECT_TIMEOUT=30
DB_LOGGING=false

# Session
SESSION_LIFETIME=120         # minutes
SESSION_COOKIE=suprnova_session
SESSION_SECURE=false         # set true in production (HTTPS only)
SESSION_PATH=/
SESSION_SAME_SITE=Lax

# Mail - defaults to `log` driver (writes outgoing mail to the
# tracing log, good for dev). Set MAIL_DRIVER to one of
# smtp / ses / mailgun / postmark / sendgrid / resend / log / memory
# for production.
MAIL_DRIVER=log
# SMTP credentials (only read when MAIL_DRIVER=smtp):
MAIL_SMTP_HOST=127.0.0.1
MAIL_SMTP_PORT=587
MAIL_SMTP_USER=
MAIL_SMTP_PASS=
# starttls | tls | none. Left blank it derives from the credentials
# above - starttls with them, none without. Production refuses to boot
# unencrypted; see the Mail chapter.
MAIL_SMTP_ENCRYPTION=
```

兄弟ファイル `.env.example` には、プレースホルダー値を持つ同じキーが含まれています。これをコミットしてください。`.env` はコミットしないでください。デフォルトの `.gitignore` は既に `.env` を除外しています。

## `.env` の読み込みの仕組み

起動時、フレームワークは以下を行います。

1. `APP_ENV` から環境を検出します（大文字小文字を区別しません。
   `prod`/`dev`/`stage`/`stg`/`test` も認識されます）。
2. プロジェクトルートから `.env` を読み込みます。
3. 環境別ファイル（`.env.staging`、`.env.production` など）が存在する場合、それを読み込みます。その値は `.env` の値をオーバーライドします。
4. 実際のプロセス環境変数は両方をオーバーライドします（これはコンテナオーケストレーションが依存する動作です）。

1 行の優先順位は次の通りです。**プロセス環境変数 > `.env.<environment>` > `.env`**。

```rust
use suprnova::Config;

let env = Config::environment();           // Environment::Local
let is_prod = Config::is_production();     // false
```

`APP_ENV=testing` での CI 実行では、フレームワークは `.env.testing` を `.env` の上に読み込むため、開発用 `.env` を変更することなく DB URL をオーバーライドしたりメールドライバーを無効にできます。

## 直接的な環境変数アクセス

文字列、数値、ブール値など、`std::str::FromStr` を実装する任意の型の単発読み込みには、
`env::*` ファミリーを使用してください。

```rust
use suprnova::config::{env, env_required, env_optional};

let port: u16 = env("SERVER_PORT", 8765);                    // デフォルト値を伴う
let url: String = env_required("APP_URL");                   // 見つからなければパニック - 起動時専用
let smtp_host: Option<String> = env_optional("MAIL_HOST");   // 見つからなければ None
```

- `env(key, default)` - デフォルト値を伴う型変換読み込み
- `env_required(key)` - キーが見つからないか解析に失敗した場合にパニックします。起動時（`bootstrap()` または `config::register()` 内）でのみ使用してください。ここで必要な値が見つからない場合、プロセスは直ちに終了する必要があります
- `env_optional(key)` - `Option<T>` を返します。見つからないか解析できない場合は `None` です

各ユニークなキーは初回読み込み時に 1 回ログに記録されるため、アプリケーションが触れている環境変数を正確に監査できます。

## 型付き設定構造体

アプリケーションが複数回読み込む任意の値に対しては、型付き構造体を定義して登録してください。パターンは以下の通りです。

```rust
// src/config/database.rs
use suprnova::Config;
use suprnova::config::{env, env_required, env_optional};

#[derive(Clone, Debug)]
pub struct DatabaseConfig {
    pub url: String,
    pub max_connections: u32,
    pub min_connections: u32,
    pub connect_timeout_secs: u32,
    pub logging: bool,
}

pub fn register() {
    Config::register(DatabaseConfig {
        url: env_required("DATABASE_URL"),
        max_connections: env("DB_MAX_CONNECTIONS", 10),
        min_connections: env("DB_MIN_CONNECTIONS", 1),
        connect_timeout_secs: env("DB_CONNECT_TIMEOUT", 30),
        logging: env("DB_LOGGING", false),
    });
}
```

その後、どこからでも 1 行で読み込めます。

```rust
let db = Config::get::<DatabaseConfig>().expect("DB config registered at boot");
println!("Pool size: {}", db.max_connections);
```

レジストリは `TypeId` でキー付けされるため、各構造体は 1 回だけ保存されます。同じ型で再び `Config::register` を呼び出すと、前のエントリが置き換わります。これはテストに便利です。

### アプリケーションに登録を配線する

スキャフォルドの `cmd/main.rs` には、fluent ブートパイプラインに `.config(…)` ステップが含まれています。

```rust
use suprnova::Application;

#[suprnova::main]
async fn main() {
    Application::new()
        .config(my_app::config::register)   // ← ここであなたの登録関数が呼ばれます
        .bootstrap(my_app::bootstrap::register)
        .routes(my_app::routes::register)
        .migrations::<my_app::migrations::Migrator>()
        .run()
        .await
}
```

`my_app::config::register` は通常、各セクションモジュールに委譲します。

```rust
// src/config/mod.rs
pub mod database;
pub mod mail;

pub fn register() {
    database::register();
    mail::register();
}
```

### 環境変数から構造体全体をデシリアライズする

大規模な設定の場合、`serde` 経由で環境変数から直接デシリアライズできます。Suprnova は 2 つのヘルパーを公開しています。

```rust
use suprnova::Config;

#[derive(Clone, Debug, serde::Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

// 環境変数から SERVER_HOST / SERVER_PORT を読み込みます
let cfg = Config::resolve_prefixed::<ServerConfig>("SERVER_")?;
```

- `Config::resolve::<T>()` - すべてのプロセス環境変数からデシリアライズします
- `Config::resolve_prefixed::<T>("PREFIX_")` - 指定されたプレフィックスを持つ環境変数のみデシリアライズします（デシリアライズ前にプレフィックスがストリップされます）

どちらも `Result<T, FrameworkError>` を返すため、必須フィールドが見つからない場合、パニックではなく `FrameworkError::Internal` が返されます。この例外には envy の診断情報が含まれています。

## 環境固有の設定

`Environment` 列挙型は標準的なセットをカバーしています。

| バリアント | 認識される `APP_ENV` の値 |
|---|---|
| `Local` | `local` |
| `Development` | `development`, `dev` |
| `Staging` | `staging`, `stage`, `stg` |
| `Production` | `production`, `prod` |
| `Testing` | `testing`, `test` |
| `Custom(String)` | その他（大文字小文字を保持し、`.env.<custom>` のルックアップに使用）|

一般的な分岐：

```rust
use suprnova::{Config, Environment};

if Config::is_production() {
    // 厳格なクッキー、実際のメールドライバーなど
}

if Config::is_debug() {
    // 詳細なエラーページ、クエリのログ出力
}

match Config::environment() {
    Environment::Production => { /* … */ },
    Environment::Staging    => { /* … */ },
    _ => { /* dev/test path */ },
}
```

`is_debug()` は `APP_DEBUG=true` が明示的に設定されている場合、または `APP_DEBUG` が未設定で検出された環境が `Local`、`Development`、または `Testing` である場合に `true` を返します。本番環境、ステージング環境、および認識されないカスタム環境はデフォルトで `false` です。本番環境ではこれをオフにしておいてください。エラーページの詳細度と内部のいくつかのデフォルト値を制御します。

### `APP_KEY` は非開発環境で必須です

本番環境（`local`/`development`/`testing` 以外の `APP_ENV`）では、Suprnova は `APP_KEY` を有効な 32 バイトの URL セーフ base64 文字列に設定する必要があります。これなしで起動すると、説明的なエラーメッセージと共に失敗します。サイレントフォールバックはありません。

まだ `APP_KEY` がない場合は、

```bash
suprnova key:generate          # キーと、.env に追加するよう促すヒントを表示します
suprnova key:generate --show   # キーのみを表示します。`APP_KEY=$(suprnova key:generate --show)` 向けです
```

どちらの形式も `.env` を自動的に編集しません。表示されたキーを `.env`（またはシークレッツマネージャー）に手動でコピーしてください。

キー rotate（古い暗号化データが移行期間中もデシリアライズできる必要がある場合）については、[暗号化](encryption.md#key-rotation) を参照してください。

## テスト内での設定

テストでは、`.env` に頼るのではなく、テストセットアップ内に設定を登録してください。

```rust
use suprnova::suprnova_test;

#[suprnova_test]
async fn test_with_custom_db() {
    suprnova::Config::register(DatabaseConfig {
        url: "sqlite::memory:".to_string(),
        max_connections: 1,
        min_connections: 1,
        connect_timeout_secs: 5,
        logging: false,
    });

    // … ここにテストを書きます
}
```

`#[suprnova_test]` 属性は、並行テストが互いのバインディングを見ないよう、分離されたコンテナ状態をセットアップします。詳細は [テスト](testing.md) を参照してください。

## Suprnova が読み込む一般的な環境変数

完全ではないリストです。これらはフレームワーク自体が参照する変数です。アプリケーション側でさらに多くの環境変数を読み込みます。

| 変数 | デフォルト | 説明 |
|---|---|---|
| `APP_NAME` | `"app"` | 起動時にログに記録され、いくつかのデフォルトエラーメッセージで使用されます |
| `APP_ENV` | `local` | `Environment::detect` と `.env.<suffix>` のルックアップを駆動します |
| `APP_DEBUG` | 環境依存（本番環境では `false`） | 詳細なエラーページと追加ログ |
| `APP_URL` | `http://localhost:8765` | 絶対 URL 生成、署名付き URL のベース URL |
| `APP_KEY` | なし（本番環境では必須） | `Crypt`、セッション、カーソル用の AES-256 キー |
| `APP_KEY_PREVIOUS` | なし | ローテーション用のカンマ区切りの前のキー（最大 8 個） |
| `SERVER_HOST` | `127.0.0.1` | バインドアドレス |
| `SERVER_PORT` | `8765` | バインドポート |
| `DATABASE_URL` | なし | アプリケーションがデータベースを使用する場合は必須 |
| `DB_MAX_CONNECTIONS` | `10` | sqlx プール最大値 |
| `DB_MIN_CONNECTIONS` | `1` | sqlx プール最小値 |
| `DB_CONNECT_TIMEOUT` | `30`（秒） | sqlx プール接続タイムアウト |
| `SESSION_LIFETIME` | `120`（分） | セッション有効期限 |
| `SESSION_TOUCH_INTERVAL` | `300`（秒） | 最小スライディング有効期限書き込み間隔 |
| `SESSION_GC_INTERVAL` | `3600`（秒） | 監視対象期限切れセッションクリーンアップ間隔 |
| `SESSION_COOKIE` | `suprnova_session` | Cookie 名 |
| `SESSION_SECURE` | `true` | `Secure` cookie フラグを設定します。ローカル HTTP 開発の場合は `false` にオーバーライドしてください。 |
| `SESSION_SAME_SITE` | `Lax` | `Strict`、`Lax`、または `None` |
| `MAIL_DRIVER` | `log` | `smtp`、`ses`、`mailgun`、`postmark`、`sendgrid`、`resend`、`log`、`memory` のいずれか |
| `CACHE_DRIVER` | `memory` | `memory`、`redis`、`database` のいずれか |
| `QUEUE_DRIVER` | `memory` | `memory`、`redis`、`database` のいずれか（未知の値は警告と共に `memory` にフォールバック） |
| `RATE_LIMIT_DRIVER` | `memory` | `memory`、`redis` のいずれか |
| `LOG_FORMAT` | 環境依存（dev/local では `pretty`、本番環境では `json`） | `pretty` または `json` |
| `LOG_LEVEL` | `info` | `error`、`warn`、`info`、`debug`、`trace` のいずれか |

完全な監査済みリストは [環境変数](env-vars.md) にあります。

## 次のステップ

- [アプリケーション ブートストラップ](bootstrap.md) - 型付き設定登録が呼び出される場所
- [サービス コンテナ](container.md) - 登録された設定がバインドされたサービスと共にどのように読み込まれるか
- [環境変数](env-vars.md) - 完全なリファレンスリスト
- [デプロイメント](deployment.md) - 本番環境のセットアップ
