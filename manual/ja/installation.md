# インストール

このチャプターは、「このマシンに Suprnova がない」から「実行中のスキャフォルドされたプロジェクト」への状態遷移です。既に完了している場合は、[クイックスタート](quickstart.md)にジャンプしてください。

## 要件

- **Rust 1.94.0+** が現在の `main` に必要です（workspace は 2024 edition を使用します）。タグ付きの v1.3.5 リリースでも、最低要件は同じく Rust 1.94.0 です。[rustup](https://rustup.rs/) でインストールしてください：
  ```bash
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  ```
- **Node.js 20 以上**および**npm**（または pnpm/yarn/bun）をフロントエンドツールチェーン用に。Suprnova は Vite 8 を使用し、スターターは TypeScript + Tailwind v4 が付属しています。[nodejs.org](https://nodejs.org/) またはパッケージマネージャー経由でインストールします。
- **使用したいドライバーに対応したデータベースクライアントライブラリ**：
  - SQLite - 追加は不要です。sqlite は同梱されています
  - PostgreSQL - ほとんどのシステムで `libpq`（通常はプリインストール）
  - MySQL または MariaDB - ほとんどのシステムで `libmariadb` / `libmysqlclient`

データベースを今選ぶ必要はありません。デフォルトスキャフォルダーは SQLite を選択するため、新しいアプリはセットアップなしで実行できます。


現在の `main` は SeaORM 2.0、SeaQuery 1.0、SQLx 0.9 を使用します。SeaORM を直接呼び出すアプリケーションでは、SeaQuery の式メソッド用に `ExprTrait` をインポートし、事前構築済みの `Statement` 値には明示的な `*_raw` 接続メソッドを使用する必要があります。この依存関係のアップグレードに伴うアプリケーションデータの移行は不要です。

## CLI をインストールする

Suprnova は Cargo プロジェクトとして配布されており、CLI のインストーラーはフレームワークを git から取得します（crates.io からではありません - 下記の[プレローンチの注記](#pre-launch-note)を参照してください）：

```bash
cargo install --git https://github.com/eas4ai/suprnova.git --tag v1.3.5 suprnova-cli
```

これは `suprnova` バイナリをコンパイルし、`~/.cargo/bin` に配置します。うまくいったか確認してください：

```bash
suprnova --version
```

`suprnova 0.x.x` と表示されるはずです。

`suprnova` が見つからない場合、`~/.cargo/bin` が `PATH` に載っていません。シェルの設定に次を追加してください：

```bash
export PATH="$HOME/.cargo/bin:$PATH"
```

## プロジェクトを作成する

`suprnova new` は完全なプロジェクト - バックエンド + 選択したフロントエンド + Vite 設定 + 認証マイグレーション + サンプルルート - をスキャフォルドします。デフォルトではインタラクティブです：

```bash
suprnova new my-app
```

ウィザードは以下を順番に質問します：

1. **プロジェクト名** - 引数として渡すときはスキップされます（`my-app`）
2. **説明** - `Cargo.toml` で使用されます
3. **作成者** - `Cargo.toml` で使用されます。デフォルトは git の `user.name`
4. **フロントエンドフレームワーク** - `svelte`（デフォルト）、`react`、`vue` のいずれか

プロンプトをスキップしたい場合（CI、スクリプト化されたセットアップ）、`--no-interaction` を渡してフロントエンドを明示的に選択します：

```bash
suprnova new my-app --frontend svelte --no-interaction
```

`--no-interaction` は説明（「Suprnova で構築された Web アプリケーション」）と作成者（空）のデフォルトを受け入れます。これらを設定するには、スキャフォルド後に生成された `Cargo.toml` を編集します。

3 つのフロントエンド選択肢はそれぞれ独自の Svelte-5、React-19、または Vue-3.5 スターターが付属しています。すべて Inertia v3 + Vite 8 + Tailwind v4 を使用し、セッションベースの認証を備えた Login/Register/Dashboard フローをプリワイアします。

Suprnova はさらに、SPA のない**スリムな API スターター**も出荷しています：

```bash
suprnova new my-api --api
```

APIスターターにはフロントエンドもInertiaレイヤーもありません。アプリケーションデータベースでMagnetarを初期化し、`BearerTokenMiddleware` を導入して、`app_users` に対するパスワード登録とログインをスキャフォルドします。

## 最初の実行

```bash
cd my-app

# マイグレーションを実行（users、sessions など）
suprnova migrate

# フロントエンドの依存関係をインストール
npm install              # プロジェクトルートで

# バックエンドと Vite をまとめて起動
suprnova serve
```

`suprnova serve` はバックエンドを `http://127.0.0.1:8765` で、Vite を `http://127.0.0.1:5765` で実行します。バックエンド URL にアクセスしてください - Vite はプロキシされているため、直接訪問する必要はありません。

ウェルカムページが表示されるはずです。その後、`/register` にアクセスしてアカウントを作成し、`/login` でログインしてください。

## スキャフォルド内容

```
my-app/
├── Cargo.toml          # crate マニフェスト、2 つの [[bin]] ターゲット
├── .env                # ローカル設定（DB URL、アプリキー、ポート）
├── .env.example        # ops/CI 用のテンプレート
├── .gitignore
├── cmd/
│   └── main.rs         # バイナリエントリーポイント；Application::new().run() を呼び出す
├── src/
│   ├── lib.rs          # モジュールの配線まわり
│   ├── bootstrap.rs    # サービスの登録（Suprnova におけるプロバイダー相当）
│   ├── routes.rs       # routes! マクロツリー
│   ├── bin/
│   │   └── console.rs  # `cargo run --bin console <subcommand>`
│   ├── actions/        # シングルメソッドの呼び出し可能なコントローラー
│   ├── commands/       # `#[command]` アノテーション付きハンドラ
│   ├── config/         # 型付き設定セクション（database、mail）
│   ├── controllers/    # home, auth, dashboard
│   ├── middleware/     # logging, authenticate
│   ├── migrations/     # SeaORM マイグレーター（users、sessions など）
│   └── models/         # `#[suprnova::model]` 構造体（user）
├── frontend/
│   ├── package.json
│   ├── vite.config.ts
│   ├── tsconfig.json
│   ├── index.html
│   └── src/
│       ├── main.{tsx,ts}
│       ├── app.css
│       ├── pages/
│       │   ├── Home, Dashboard
│       │   └── auth/{Login,Register}
│       └── types/
│           └── inertia-props.ts
└── public/
    └── assets/         # Vite 本番ビルド出力
```

完全なディレクトリツアーは [ディレクトリ構成](structure.md) にあります。

## CLI を更新する

CLI はあなたの `~/.cargo/bin` にあります。最新版に更新するには：

```bash
cargo install --force --git https://github.com/eas4ai/suprnova.git --tag v1.3.5 suprnova-cli
```

`--force` は Cargo に既存のバイナリを上書きさせます。

## アプリのフレームワークバージョンを更新する

スキャフォルドされたアプリは、`Cargo.toml` の git 依存性を経由して `suprnova` フレームワーククレートに依存します：

```toml
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.3.5" }
```

最新のフレームワークの変更を取り込むには：

```bash
cargo update -p suprnova
```

git 依存性は、名前を指定されたリリースタグを追跡します。`Cargo.toml` のタグを更新してから `cargo update -p suprnova` を実行してください。`Cargo.lock` が解決した正確なコミットを記録するため、更新の合間もビルドは再現可能なままです - `Cargo.toml` に `rev` をハンドピンする必要はありません。

## 配布モデル

Suprnova は crates.io ではなく git を通じて配布されます - フレームワークと CLI の両方が GitHub からインストールされます。各バージョンはタグ付きの GitHub Release（例えば `v1.2.4`）として公開されており、あなたのアプリが依存するのはそのタグです：スキャフォルドされた `Cargo.toml` は `tag = "v1.3.5"` をピン留めし、`Cargo.lock` はそのタグが解決した正確なコミットを記録するため、あなたが移ると決めるまで、ビルドは再現可能なままです。更新は意図的な操作であり、偶発的に起こることは決してありません - タグを上げて `cargo update -p suprnova` を実行してください。アプリのフレームワークバージョンを更新するセクションが、その手順を説明しています。

## エディタセットアップ

いくつかの VS Code 拡張機能でエクスペリエンスがスムーズになります：

- **rust-analyzer** - Rust 言語サーバー
- **Svelte for VS Code**（React/Vue を選択した場合はそれら）
- **Tailwind CSS IntelliSense**
- **Even Better TOML**

`rust-analyzer` は初回オープン時にプロジェクトをインデックス化します。初回は 1-2 分かかり、その後は増分です。

## 次のステップ

- [クイックスタート](quickstart.md) - 5 分でティニーアプリを構築する
- [ディレクトリ構成](structure.md) - スキャフォルダーが生成した各ファイルの内容
- [設定](configuration.md) - `.env` と型付き設定のストーリー
- [ルーティング](routing.md) - 最初のルートを追加する
