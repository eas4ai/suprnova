# CLI 概要

Suprnovaは、異なる役割を持つ2つのバイナリを出荷します。グローバルな `suprnova` - 一度だけ `~/.cargo/bin` にインストールされます - は、新しいプロジェクトをスキャフォルドし、コードを生成し、開発サーバーを起動し、マイグレーションを実行します。各アプリの `src/bin/console.rs` からビルドされる、プロジェクトごとの `console` は、アプリのコンパイル済み型を必要とする実行時コマンド（シーダー、プルーナー、あなた自身の `#[command]` ハンドラ）を実行します。この章は地図です。各サブコマンドは、[次のステップ](#次のステップ)に一覧されている姉妹チャプターで、それぞれ深く掘り下げられています。

## インストール

CLIは `cargo install --git` 経由で配布されます。Suprnovaはまだcrates.ioにはありません - その理由については、[インストールのプリローンチノート](installation.md#pre-launch-note)を参照してください。

```bash
cargo install --git https://github.com/eas4ai/suprnova.git --tag v1.2.4 suprnova-cli
suprnova --version
```

後で更新するには、`--force` を渡してください:

```bash
cargo install --force --git https://github.com/eas4ai/suprnova.git --tag v1.2.4 suprnova-cli
```

## 2つのバイナリ

| バイナリ | ビルド元 | 用途 |
|---|---|---|
| `suprnova` | `suprnova-cli/`（このクレート） | スキャフォルド（`new`）、ジェネレーター（`make:*`）、開発ランナー（`serve`）、マイグレーション（`migrate*`、`db:sync`）、Docker設定（`docker:*`）、SSRワーカー（`ssr:*`）、キーの生成（`key:generate`）、型生成（`generate-types`） |
| `console` | プロジェクトの中の `src/bin/console.rs` | あなたのアプリの型をリンクする実行時コマンド - 組み込みの `db:seed` と `model:prune`、それにあなたが定義するすべての `#[command]` / `#[derive(Command)]` |

ワーカーデーモン（`schedule:run`、`schedule:work`、`schedule:list`、`workflow:work`、`queue:work`）は、3つ目の表面 - HTTPを提供するのと同じバイナリである、あなたの*app*バイナリ自身のclapパーサー - の上に存在します。グローバルな `suprnova` は、それらについては `cargo run --quiet -- <name>` へシェルするため、既に開いているCLIから起動できます。3方向の分割の全体像については、[コンソール](console.md)を参照してください。

### Suprnovaが異なる設計を選んだ理由

Laravelはこれを、プロジェクトごとの単一のスクリプト - `php artisan` - で解決しています。PHPが実行時にフレームワークとユーザーコードを一緒にロードするからです。Rustはコンパイル時にバイナリをリンクするため、グローバルな `suprnova` バイナリは、あなたのシーダー、ファクトリー、`#[command]` ハンドラを静的に見ることができません。実用的な分割は次のとおりです:

- ファイルのみを扱う作業（スキャフォルド、ジェネレーター、運用）は、グローバルな `suprnova` バイナリの上にある
- コンパイル済みの型を必要とする実行時の作業は、プロジェクトごとの `console` バイナリの上にある
- デーモンは、あなたのapp/serverバイナリの上にあり、`serve` と同じ起動経路を共有する

静的リンクという幻想を持たずに、`php artisan` の使い勝手（`cargo run --bin console -- db:seed`、あるいは直接 `console <name>`）が手に入ります。

## コマンド一覧

`suprnova --help` が出力するのと同じリストを、同じようにグループ化したものです。

### 作成

| コマンド | 説明 |
|---|---|
| `suprnova new [name]` | 新しいプロジェクトをスキャフォルドする。[`suprnova new`](cli-new.md)を参照。 |
| `suprnova serve` | バックエンド + Viteを、ホットリロードとともに一緒に起動する。[`suprnova serve`](cli-serve.md)を参照。 |
| `suprnova dev:tls` | portlessのCAを信頼し、`https://<name>.localhost` の開発URLを登録する。[HTTPS 開発 URL](dev-tls.md)を参照。 |
| `suprnova web:run` | appバイナリを直接実行する（Viteなし、再ビルドループなし）。本番環境の形をしたローカル実行。 |

### 生成

| コマンド | 説明 |
|---|---|
| `suprnova make:controller <name>` | `src/controllers/` にコントローラーをスキャフォルドする。 |
| `suprnova make:action <name>` | `src/actions/` に呼び出し可能なアクションをスキャフォルドする。 |
| `suprnova make:middleware <name>` | `src/middleware/` にミドルウェアをスキャフォルドする。 |
| `suprnova make:migration <name>` | `src/migrations/` にSeaORMのマイグレーションをスキャフォルドする。 |
| `suprnova make:inertia <name>` | `frontend/src/pages/` にInertiaページをスキャフォルドする。代わりに `src/props/` の中の `#[derive(Data, Validate)]` propsの構造体にするには `--data` を渡す。 |
| `suprnova make:error <name>` | `src/errors/` にドメインエラーをスキャフォルドする。 |
| `suprnova make:task <name>` | `src/tasks/` にスケジュールされたタスクをスキャフォルドする。 |
| `suprnova make:command <name>` | `src/commands/` に `#[derive(Command)]` のコンソールコマンドをスキャフォルドする。 |
| `suprnova generate-types` | あらゆる `#[derive(InertiaProps)]` 構造体からTypeScriptの型を出力する。出力先を上書きするには `-o <path>`、ウォッチして再生成するには `-w`。 |

完全なスキャフォルドの詳細と、各生成ファイルの見た目については、[コード ジェネレーター](cli-generators.md)を参照してください。

### データベース

| コマンド | 説明 |
|---|---|
| `suprnova migrate` | 保留中のマイグレーションをすべて実行する。 |
| `suprnova migrate:status` | どのマイグレーションが適用済みで、どれが保留中かを表示する。 |
| `suprnova migrate:rollback [--step N]` | 直近N件のマイグレーションをロールバックする（デフォルトは1）。 |
| `suprnova migrate:fresh [--force]` | 全テーブルを削除し、すべてのマイグレーションを再実行する。**破壊的。** 本番環境では `--force` に加えて、対話端末でのタイプ入力による確認が必要。 |
| `suprnova db:sync [--skip-migrations] [--regenerate-models]` | マイグレーションを実行し、稼働中のスキーマからSeaORMエンティティを再生成する。`--regenerate-models` は `src/models/` 内のカスタムモデルファイルも上書きする。 |

`db:seed` はここには**ありません** - シーダーのレジストリがあなたのクレートにコンパイルされるため、プロジェクトごとの `console` バイナリの上にあります。`cargo run --bin console -- db:seed` あるいは `./target/debug/console db:seed` 経由で実行してください。登録パターンについては、[コンソール](console.md)を参照してください。

完全なマイグレーションのワークフローについては、[CLI マイグレーション](cli-migrations.md)を参照してください。

### スケジュール

| コマンド | 説明 |
|---|---|
| `suprnova schedule:run` | 実行予定のタスクをすべて一度だけ実行する。cronに適した形。 |
| `suprnova schedule:work` | 毎分チェックして実行予定のタスクを実行する、フォアグラウンドのデーモン。 |
| `suprnova schedule:list` | 登録済みのタスクをすべて、そのcron式とともに出力する。 |

これらはそれぞれ、あなたのapp/serverバイナリ - HTTPを提供するのと同じバイナリ - に対して `cargo run --quiet -- <name>` へシェルするため、登録済みのタスクとブートストラップされたサービスが見えます。[スケジューリング コマンド](cli-scheduling.md)と[タスク スケジューリング](scheduling.md)チャプターを参照してください。

### ワークフロー

| コマンド | 説明 |
|---|---|
| `suprnova workflow:work` | ワークフローワーカーデーモンを起動する。レジストリからワークフローのステップを取り出し、HTTPハンドラと同じパニック境界で実行する。 |
| `suprnova workflow:install` | workflow + workflow_stepsのマイグレーションを `src/migrations/` に配置する。新規スキャフォルドには既に存在する。 |

[ワークフロー](workflows.md)を参照してください。

### SSR

| コマンド | 説明 |
|---|---|
| `suprnova ssr:start [--runtime node\|bun\|deno] [--bundle <path>]` | Inertia SSRワーカーをフォアグラウンドで起動する。`SUPRNOVA_SSR_RUNTIME` env、次に `node` へフォールバックする。バンドルは `SUPRNOVA_SSR_BUNDLE`、次に `frontend/bootstrap/ssr/ssr.js` へフォールバックする。 |
| `suprnova ssr:check [--url <url>] [--timeout-ms N]` | SSRワーカーをプローブする。`SUPRNOVA_SSR_URL`、次に `http://127.0.0.1:13714` へフォールバックする。タイムアウトのデフォルトは2000ms。 |

本番環境のセットアップについては、[Inertia SSR](frontend.md)を参照してください。

### デプロイ

| コマンド | 説明 |
|---|---|
| `suprnova docker:init` | 本番用のマルチステージ `Dockerfile` + `.dockerignore` を出力する。 |
| `suprnova docker:compose [--with-mailpit] [--with-minio]` | ローカル開発用の `docker-compose.yml` を出力する。Postgres + Redisは常に含まれ、MailpitとMinIOはオプトイン。 |

[Docker](cli-docker.md)と[デプロイメント 概要](deployment.md)チャプターを参照してください。

### セキュリティ

| コマンド | 説明 |
|---|---|
| `suprnova key:generate [--show]` | 32バイトのAES-256キーを生成する。base64のURLセーフでパディングなし（`EncryptionKey::to_base64` が生成するのと同じ通信上の形式）。`--show` は `APP_KEY=$(suprnova key:generate --show)` のために、キーだけを出力する。 |

`APP_KEY` が何を保護しているか、そして `APP_KEY_PREVIOUS` 経由のローテーションがどう機能するかについては、[暗号化](encryption.md)を参照してください。

## クイックスタート

「何もインストールされていない」から「動作するアプリ」までの、最も一般的な道筋:

```bash
# 1. CLIをインストールする
cargo install --git https://github.com/eas4ai/suprnova.git --tag v1.2.4 suprnova-cli

# 2. プロジェクトをスキャフォルドする（インタラクティブ - デフォルトでSvelteを選ぶ）
suprnova new my-app

# 3. 起動する
cd my-app
suprnova migrate
npm install
suprnova serve
```

非インタラクティブなスキャフォルド（CI、スクリプト化されたセットアップ）:

```bash
suprnova new my-app \
  --frontend svelte \
  --no-interaction \
  --no-git
```

APIのみのスキャフォルド（Inertiaなし、SPAなし）:

```bash
suprnova new my-api --api
```

既存のプロジェクトでコードを生成する:

```bash
suprnova make:controller Posts
suprnova make:migration create_posts_table
suprnova make:command reports:daily   # プロジェクトごとのconsoleバイナリの下に登録される
suprnova migrate
```

## ヘルプを表示する

`--help`（あるいは `-h`）は、どのサブコマンドでも機能します。トップレベルのヘルプは手動でフォーマットされており（`ui::print_help`）、コマンドをセクションごとにグループ化します。サブコマンドごとのヘルプはclapから来ており、すべてのフラグをそのデフォルト値とともに表示します:

```bash
suprnova --help
suprnova new --help
suprnova serve --help
suprnova make:inertia --help
```

プロジェクトごとの `console` バイナリについては:

```bash
cargo run --bin console -- --help
cargo run --bin console -- db:seed --help
cargo run --bin console -- <your-command> --help
```

`--version` は、バージョンを単独の行に出力します。これは、バグを報告するときや、インストールが成功したかを確認するときに欲しいものです:

```bash
suprnova --version
# suprnova 1.2.4
```

`-v` と `-V` の両方が受け付けられます。clapが生成するフラグは `-V` だけを提供しますが、こちらは手動で宣言されているため、小文字のつづり - 多くの人が最初に試すもの - も機能します。バージョンは `--help` のバナーにも現れます。これは、そのフラグが存在する前からバージョンが置かれていた場所です。

## 次のステップ

- [`suprnova new`](cli-new.md) - スキャフォルダーが受け付けるすべてのフラグと、生成されるディレクトリレイアウト
- [`suprnova serve`](cli-serve.md) - 開発ランナー: バックエンド + Vite + 型生成
- [コード ジェネレーター](cli-generators.md) - 出力テンプレートを備えた、完全な `make:*` ファミリー
- [CLI マイグレーション](cli-migrations.md) - `migrate`、`migrate:fresh`、`db:sync`、そしてSeaORMのワークフロー
- [コンソール](console.md) - プロジェクトごとの `console` バイナリ、`#[command]`、`#[derive(Command)]`、そして3バイナリの非対称性
