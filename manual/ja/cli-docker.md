# Docker

Suprnovaは、そのまま採用するか変更できるDockerアーティファクトを生成する、2つのCLIコマンドを出荷します。`docker:init` は、本番用のマルチステージ `Dockerfile` + `.dockerignore` を書き込みます。`docker:compose` は、ローカル開発用サービス（データベース、キャッシュ、オプションでMailpit + MinIO）のための `docker-compose.yml` を書き込みます。両方のコマンドは、現在のプロジェクトルートに書き込みます。どちらも、あなたのコンテナランタイムを操縦しようとはしません。

## docker:init

対応する `.dockerignore` と一緒に、本番用のDockerfileを生成します。

```bash
suprnova docker:init
```

このコマンドは、既存の `Dockerfile` の上書きを拒みます。再生成したい場合は、まず既存のファイルを削除してください。

### 書き込まれる内容

| ファイル | 目的 |
|------|---------|
| `Dockerfile` | 3段階のビルド: フロントエンドのアセット、Rustのリリースバイナリ、ランタイムイメージ |
| `.dockerignore` | `target/`、`node_modules/`、`.env*`、既存のビルドアーティファクト、そしてDockerファイル自身を除外する |

### Dockerfileの形

生成されるDockerfileは3段階を使うため、ランタイムイメージは、コンパイル済みのバイナリと、それが必要とする共有ライブラリだけを運びます:

1. **`frontend-builder`** - `node:20-alpine`。npmの依存関係をインストールし、`npm run build` を実行して `frontend/dist` を生成する。
2. **`backend-builder`** - `rust:1.91.1-slim-bookworm`。`Cargo.toml` + `Cargo.lock` を依存関係のレイヤーとしてキャッシュし、その後 `cmd/`、`src/`、そしてビルド済みの `frontend/dist`（`public/assets` として）をコピーし、`cargo build --release` を実行する。
3. **`runtime`** - `ca-certificates` と `libssl3` を備えた `debian:bookworm-slim`。non-rootの `appuser` として実行される。バイナリを `./app` としてコピーし、その隣に `public/` ディレクトリを置く。ポート8765をエクスポーズする。

最終イメージのデフォルトの `CMD` は `["./app"]` であり、これは統合バイナリの `serve` サブコマンド（起動時に自動マイグレーションを行うWebサーバー）を実行します。別のサブコマンドを実行するには、`docker run` の時点でコマンドを上書きしてください:

```bash
# Webサーバー（デフォルト）
docker run -p 8765:8765 --env-file .env.production my-app

# マイグレーションだけを実行して終了する
docker run --env-file .env.production my-app ./app migrate

# スケジューラーデーモンを実行する
docker run --env-file .env.production my-app ./app schedule:work

# キューワーカーを実行する
docker run --env-file .env.production my-app ./app queue:work
```

本番環境の設定は、`--env-file .env.production` あるいは個別の `-e` フラグ経由で渡してください。`.env.production` は決してコミットすべきではありません - 既に `.dockerignore` でカバーされています。

### Rustツールチェーンを上げる

Dockerfileは、ビルドステージのために `rust:1.91.1-slim-bookworm` にピン留めしています。新しく生成されたイメージが再現可能であり、Suprnova 0.6が宣言するMSRVと一致するようにするためです。カスタムのDockerfileは、同じか、それより新しいツールチェーンを使うべきです:

```dockerfile
FROM rust:1.91.1-slim-bookworm AS backend-builder
```

（もしあれば）`rust-toolchain.toml` か、あなたのローカルの `rustc --version` が報告するものに一致する、どのツールチェーンバージョンにでもピン留めしてください。

### Suprnovaが異なる設計を選んだ理由

Laravelのデプロイは通常、**コンテナあるいはホストごとに複数のプロセス**を実行します: Web用のphp-fpm、キューワーカー、スケジューラー、時にはHorizonダッシュボード、時にはOctaneランナーです。それぞれが自分自身のサービス定義です。

Suprnovaは、フレームワークが出荷するあらゆるサブコマンド - `serve`、`migrate`、`queue:work`、`schedule:work`、`workflow:work`、`ssr:start` - を知っている、**1つの静的リンクされたバイナリ**にコンパイルされます。同じDockerイメージがすべての役割を実行し、変わるのはコマンドだけです。そのため、「web + worker + scheduler」は、オーケストレーター内では同じイメージタグを指す3つのサービスになります - アプリ全体を前進させるビルドは1回だけです。

## docker:compose

ローカル開発用サービスを立ち上げる `docker-compose.yml` を生成します。

```bash
suprnova docker:compose [OPTIONS]
```

`docker:init` と同様に、これも既存の `docker-compose.yml` の上書きを拒みます。また、（`.gitignore` が存在する場合）`docker-compose.override.yml` を `.gitignore` に追記するため、開発者ごとの上書きをコミットせずにローカルに保つことができます。

### オプション

| オプション | 説明 |
|--------|-------------|
| `--with-mailpit` | Mailpitのメールテストサービスを含める |
| `--with-minio` | MinIO（S3互換のオブジェクトストレージ）を含める |

どちらのフラグも渡さない場合、コマンドは両方についてインタラクティブに尋ねます。いずれかのフラグを渡すと、プロンプトはスキップされ、渡したフラグの値が使われます。

### 常に手に入るもの

PostgreSQLとRedisは、生成されるすべてのcomposeファイルに書き込まれます:

| サービス | デフォルトポート | イメージ |
|---------|-------------:|-------|
| PostgreSQL | 5432 | `postgres:16-alpine` |
| Redis | 6379 | `redis:7-alpine` |

両方のサービスは、ヘルスチェック、永続的な名前付きボリュームを持ち、プロジェクトスコープのネットワーク（`<project>_network`）上で動きます。Postgresのユーザー、パスワード、データベースのデフォルトは `suprnova` / `suprnova_secret` / `suprnova_db` です。

### オプションのサービス

オプトインすると:

| サービス | デフォルトポート | イメージ |
|---------|--------------:|-------|
| Mailpit | 1025（SMTP）、8025（UI） | `axllent/mailpit:latest` |
| MinIO | 9000（S3 API）、9001（Console） | `minio/minio:latest` |

Mailpitは、開発中に認証情報を設定する必要がないよう、デフォルトでどんなSMTP認証でも受け入れます。`http://localhost:8025` のWeb UIは、あなたのアプリが送信するすべてのメールを表示します。MinIOのデフォルトの認証情報は `minioadmin` / `minioadmin` です。

### スタックを実行する

```bash
# すべてをバックグラウンドで立ち上げる
docker compose up -d

# ログを追う
docker compose logs -f

# コンテナを停止して削除する（ボリュームは残る）
docker compose down

# ボリュームも削除する（ローカルデータベースを消し去る）
docker compose down -v
```

### `.env` をcomposeに配線する

composeファイルは、あらゆる場所で `${VAR:-default}` という構文を使っているため、`.env` やシェルで設定することで、何でも上書きできます。デフォルトのスタックのための典型的な `.env`:

```env
DATABASE_URL=postgres://suprnova:suprnova_secret@localhost:5432/suprnova_db
REDIS_URL=redis://localhost:6379

# Mailpit（有効な場合）
MAIL_DRIVER=smtp
MAIL_HOST=localhost
MAIL_PORT=1025

# MinIO（有効な場合）
FILESYSTEM_DISK=s3
S3_ENDPOINT=http://localhost:9000
S3_ACCESS_KEY=minioadmin
S3_SECRET_KEY=minioadmin
S3_BUCKET=local
S3_REGION=us-east-1
```

ポートを上書きするには（例えば5432が既に使用中である場合）、スタックを立ち上げる前に、対応する環境変数を設定してください:

```bash
DB_PORT=5433 docker compose up -d
```

上書き可能なポートの完全な一覧:

| 変数 | サービス | デフォルト |
|----------|---------|--------:|
| `DB_PORT` | PostgreSQL | 5432 |
| `REDIS_PORT` | Redis | 6379 |
| `MAILPIT_SMTP_PORT` | Mailpit SMTP | 1025 |
| `MAILPIT_UI_PORT` | Mailpit UI | 8025 |
| `MINIO_API_PORT` | MinIO S3 | 9000 |
| `MINIO_CONSOLE_PORT` | MinIO Console | 9001 |

### composeファイルをカスタマイズする

`docker-compose.yml` は、生成後はあなたが編集するためのものです - Suprnovaは、後でそれを再生成したり読んだりしません。よくあるパッチ:

- それらのドライバーのどちらかを好むなら、`postgres:16-alpine` を `mysql:8` か `mariadb:11` に差し替えてください。どちらもSuprnovaではファーストクラスです
- ワンショットのコンテナ内でマイグレーションを実行したい場合は、`migrations/` ディレクトリをマウントする `volumes:` エントリを追加してください
- 同じ方法で、追加のサービス（Qdrant、Elasticsearch、Nats）を追加してください

## 本番デプロイ

実際のデプロイでは、`docker:init` を実行し、生成された `Dockerfile` をビルド入力として扱ってください。ほとんどのオーケストレーター（Railway、Fly、Digital Ocean App Platform、Kubernetes）が必要とするのは、次の3つだけです:

1. この `Dockerfile` からビルドされたイメージタグ
2. `DATABASE_URL`、`APP_KEY`、そしてドライバー固有のキーを持つenvファイル
3. `GET /_suprnova/health/live` を指すヘルスチェック（そして、プラットフォームが両者を区別するなら、`/_suprnova/health/ready` へのレディネスチェック）

単一バイナリの形は、すべての役割が同じイメージを使うことを意味します。`./app` を実行する「web」サービスと、`./app schedule:work`（あるいは `./app queue:work`）を実行する「scheduler」あるいは「worker」サービスを宣言します。両方が同じ環境変数を読むため、デプロイのたびに歩調を合わせたままになります。

プラットフォームに依存しないチェックリストについては[デプロイメント 概要](deployment.md)を、詳しく解説された例については、プラットフォームガイド - [Railway にデプロイ](deployment-railway.md)、[Digital Ocean にデプロイ](deployment-digital-ocean.md)、[Hetzner VPS にデプロイ](deployment-hetzner.md) - を参照してください。

## まとめ

| コマンド | 書き込むもの | 使うタイミング |
|---------|--------|-------------|
| `suprnova docker:init` | `Dockerfile`、`.dockerignore` | 本番用イメージをビルドするとき |
| `suprnova docker:compose` | `docker-compose.yml` | ローカルのPostgres/Redis/Mailpit/MinIOを立ち上げるとき |

## 次のステップ

- [デプロイメント 概要](deployment.md) - プラットフォームに依存しないデプロイのチェックリスト
- [Railway にデプロイ](deployment-railway.md) - gitからビルドするマネージドPaaS
- [Digital Ocean にデプロイ](deployment-digital-ocean.md) - App Platformへのデプロイ
- [Hetzner VPS にデプロイ](deployment-hetzner.md) - systemd + Caddyを使ったベアメタル
- [環境変数](env-vars.md) - フレームワークが読み取るすべてのキー
