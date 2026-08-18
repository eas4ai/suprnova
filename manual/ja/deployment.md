# デプロイメント 概要

Suprnova アプリは単一の自己完結型バイナリにコンパイルされ、Web サーバー、マイグレーションランナー、スケジューラー、およびキューワーカーを所有しています。デプロイは「バイナリをコピーして、4 つの環境変数を設定して、実行する」という流れです。このチャプターでは、その 4 つの変数が何であるか、バイナリのサブコマンドが本番環境で何をするか、組み込みのヘルスエンドポイントがプラットフォームのライブネスプローブとどのように統合されるかについて説明します。プラットフォーム固有のウォークスルーは [Railway](deployment-railway.md)、[Digital Ocean](deployment-digital-ocean.md)、および [Hetzner](deployment-hetzner.md) に続きます。

## 単一バイナリ

アプリは clap サブコマンドサーフェスを持つ 1 つのバイナリにビルドされます：

```bash
./app                       # serve（デフォルト） - 自動マイグレーションののち HTTP
./app serve                 # 明示的な serve、自動マイグレーションあり
./app serve --no-migrate    # マイグレーションを実行せずに serve
./app web:run               # serve のエイリアス

./app migrate               # 保留中のマイグレーションを適用して終了
./app migrate:status        # マイグレーションの状態を表示
./app migrate:rollback [N]  # 直近 N 件のマイグレーションをロールバック（デフォルトは 1）
./app migrate:fresh         # 全テーブルを削除して再マイグレーション - 本番環境では
                            # --force と、対話端末でタイプ入力する確認の両方が必要です。
                            # cli-migrations.md を参照してください

./app schedule:work         # スケジューラーデーモン - 毎分目を覚まします
./app schedule:run          # 期限の来たタスクを一度実行して終了
./app schedule:list         # 登録済みのタスクをすべて出力
./app queue:work            # キューワーカーデーモン
./app workflow:work         # ワークフローワーカーデーモン

./app down [--secret …] [--retry …] [--except …] [--message …]
./app up                    # メンテナンスモードを抜ける
```

1 つのバイナリは 1 つの Docker イメージ、1 つの CI アーティファクト、検証する 1 つのデプロイを意味します。同じイメージが Web サービス、スケジューラー、キューワーカー、およびワークフローワーカーを実行します。それぞれに対して異なるサブコマンドを開始します。

## 4 つの本番環境変数

Suprnova は本番環境が誤設定されている場合、ブート時に失敗状態になります。デプロイするための最小限の環境変数セット：

| 変数 | 機能 | 失敗モード |
|---|---|---|
| `APP_ENV` | 環境を選択します（`production`、`staging` など）。 | 未設定の場合は `local` がデフォルトです - アプリが本番環境で開発モードで実行されます。 |
| `APP_KEY` | `Crypt`、セッション、クッキー、およびペジネーションカーソル用の 32 バイト AES-256 base64 キー。 | `APP_ENV` が local/dev/test 以外であり、`APP_KEY` が不正または形式が不正な場合、ブートは型指定エラーを返し、ゼロ以外で終了します。 |
| `APP_URL` | アプリの正規絶対 URL（`https://app.example.com`）。 | デフォルトは `http://localhost:8765` です。署名付き URL、リダイレクト、メールリンク、および絶対 Inertia URL はすべてこれを使用します。 |
| `DATABASE_URL` | リレーショナルデータベースの接続 URL。 | `APP_ENV` が `production` または `staging` であり、`DATABASE_URL` が未設定の場合、ブート開始を拒否します - 開発用 SQLite フォールバックは明示的に拒否されます。 |

CLI で `APP_KEY` を一度生成します：

```bash
suprnova key:generate           # APP_KEY=… を ./.env に書き込みます
suprnova key:generate --show    # $(…) 向けにキーだけを表示します
```

キーのローテーションについては [暗号化](encryption.md) を参照してください。`APP_KEY_PREVIOUS`（または Laravel 互換の `APP_PREVIOUS_KEYS`）は、復号化のみのフォールバック用に、古いキーのコンマ区切りリストを取得します。

4 つの必須変数を超えて、一般的な本番環境ノブ：

| 変数 | デフォルト | ノート |
|---|---|---|
| `SERVER_HOST` | `127.0.0.1` | コンテナでは `0.0.0.0` を使用します。 |
| `SERVER_PORT` | `8765` | プラットフォームの期待されるポートに一致させます。 |
| `APP_DEBUG` | env-derived | 本番環境/ステージング/カスタム環境では `false`。ステージングで大きなエラーが必要な場合は明示的に設定します。 |
| `SERVER_MAX_BODY_SIZE` | ハンドラごとのデフォルト | プロセス全体のリクエストボディキャップ。 |
| `SERVER_MAX_CONNECTIONS` | 未設定（無制限）| 同時アクティブ TCP 接続のキャップ。下記を参照してください。 |
| `SERVER_HEALTH_READINESS_TOKEN` | 未設定（レディネスは公開）| レディネスプローブに到達するために必要な共有シークレット。[ヘルスチェック](#ヘルスチェック)を参照してください。 |
| `DB_MAX_CONNECTIONS` | `10` | プールサイズ。 |
| `REDIS_URL` | unset | Redis キャッシュ/キュー/セッションドライバーを設定した場合に必要です。 |

完全なテーブルは [環境変数](env-vars.md) にあります。

## 推奨データベース：MariaDB

Suprnova は SQLite、PostgreSQL、MySQL、および MariaDB をファーストクラスのリレーショナルバックエンドとしてサポートしています。推奨事項は環境固有です：

- **開発環境。** SQLite。スキャフォルダーは `DATABASE_URL=sqlite://./database.db` を書き込むため、`suprnova serve` はデータベースセットアップなしで動作します。
- **本番環境。** MariaDB。これにより、3 つの別々のサービス（リレーショナル + ベクトル + KV キャッシュ）を 1 つのエンジンに折りたたみます。必要に応じて監査用のシステムバージョン管理されたテーブルがあります。

```bash
# .env.production
DATABASE_URL=mysql://app_user:secret@db.internal:3306/app_production
```

`mysql://` スキームを使用します。SeaORM の MySQL ドライバーは MariaDB をネイティブに処理し、Suprnova の `MariaDbVectorDriver`（`VECTOR(N)` + HNSW）はベクトル作業負荷に直接フックします。

他のリレーショナルバックエンドもファーストクラスです：

```bash
# PostgreSQL
DATABASE_URL=postgres://app_user:secret@db.internal:5432/app_production

# MySQL
DATABASE_URL=mysql://app_user:secret@db.internal:3306/app_production

# SQLite（ごく小さな単一インスタンスのデプロイ向け）
DATABASE_URL=sqlite:///var/lib/myapp/data.db
```

### Suprnovaが異なる設計を選んだ理由

Laravel のデフォルトは、PHP + PostgreSQL が実績のあるパスであるため、新しいプロジェクトを PostgreSQL に向かわせます。Suprnova は、Rust アプリに対して最もクリーンな単一エンジンの本番環境設定を提供するデータベースを選択します。MariaDB の `VECTOR(N)`（11.7+）、Dynamic Columns、およびシステムバージョン管理されたテーブルは、中小製品が Redis、OpenSearch、または pgvector を組み込むことなく、検索、KV、および監査を出荷できることを意味します。PostgreSQL は完全にサポートされています - フレームワークのテストマトリックスは 3 つのリレーショナルバックエンドすべてに対して実行されます - しかし、デプロイメントドキュメントは移動部分を最小限にするエンジンでリードしています。バックエンド固有の表面については、[ベクトルストレージ](vector.md) および [データベース](database.md) を参照してください。

## 本番イメージの構築

スキャフォルダーはマルチステージ Dockerfile のジェネレーターを提供します：

```bash
suprnova docker:init
```

これにより、3 つのステージを持つ `Dockerfile` が書き込まれます：

1. **フロントエンドビルド** - `node:20-alpine`、`frontend/` Inertia アプリに対して `npm ci && npm run build` を実行します（スキャフォルド選択に応じて Svelte 5、React 19、または Vue 3.5）。
2. **バックエンドビルド** - `rust:1.91.1-slim-bookworm`、依存関係キャッシング付きでクレートをリリースモードでコンパイルします。
3. **ランタイム** - `debian:bookworm-slim`、コンパイルされたバイナリと Vite 出力をコピーし、non-root `appuser` として実行され、ポート 8765 をエクスポーズし、`CMD ["./app"]`（自動マイグレーションサーバー）を実行します。

プッシュする前にローカルでビルドして実行して確認します：

```bash
docker build -t myapp .

# env ファイルを使う場合
docker run --rm -p 8765:8765 --env-file .env.production myapp

# あるいは変数を明示する場合（必須の 4 つ）
docker run --rm -p 8765:8765 \
  -e APP_ENV=production \
  -e APP_KEY=$APP_KEY \
  -e APP_URL=https://app.example.com \
  -e DATABASE_URL=mysql://user:pass@host:3306/app \
  myapp
```

`.env.production`（または `APP_KEY` または `DATABASE_URL` を含むファイル）をリポジトリにコミットしないでください。プラットフォームのシークレットストアを使用し、デプロイ時に値を読み込んでください。

## ブート時のマイグレーション

デフォルトの `./app`（および明示的な `./app serve`）コマンドは、ソケットをバインドする前に保留中のマイグレーションを適用します。2 つの実際の影響：

- **複数インスタンスで安全。** SeaORM のマイグレーションランナーはデータベースレベルのアドバイザリロックを使用します。最も遅いポッドは待機し、他のポッドは完了後に進みます。ルーチン リリース ロールには、個別の「migrate-then-deploy」ステップは不要です。
- **マイグレーション失敗 = デプロイ失敗。** マイグレーションがエラーになった場合、プロセスはサーバーをバインドする前にゼロ以外で終了します。プラットフォームのヘルスプローブ（下記を参照）はポッドを不健康と報告し、ロールアウトが停止します。次のリリースで修正マイグレーションを送信して修正を進めます。

ポッドがトラフィックを受け入れる前に成功したマイグレーション上でデプロイをゲートできたい CI パイプラインの場合は、マイグレーションをワンショットで実行します：

```bash
docker run --rm myapp ./app migrate
# … その後、実際のデプロイを進めます
docker run myapp ./app serve --no-migrate
```

`--no-migrate` は自動マイグレーションフェーズをスキップしますが、それでもサーバーは正常にブートします。

## 個別のサービスとしてのワーカー

スケジューラー、キュー、およびワークフローシステムはそれぞれ独自のデーモンサブコマンドを持ちます。本番環境では、同じイメージに対して個別のプロセスとして実行し、同じ環境を共有します：

```bash
docker run myapp ./app schedule:work    # インスタンスは 1 つ - 下記を参照
docker run myapp ./app queue:work       # N インスタンスまでスケール
docker run myapp ./app workflow:work    # N インスタンスまでスケール
```

内面化する 2 つのルール：

- **正確に 1 つの `schedule:work` プロセスを実行するか、タスクを `.on_one_server()` でマークします。** スケジューラーレプリカはデフォルトで調整しません。それぞれがスケジュールを独立して評価するため、3 つのレプリカは期限切れのすべてのタスクを 3 回実行します。`replicas: 1` がシンプルな答えです。`.on_one_server()` は共有キャッシュに対してティックごとに 1 つのレプリカを選出し、スケジューラーが高可用性でなければならない場合は必要なものです。[スケジューリング](scheduling.md#running-on-one-server) を参照してください。
- **キューおよびワークフローワーカーは水平にスケーリングします。** どちらも共有ストアから作業を引き出し、可視性タイムアウトまたは行レベルのロックを使用して調整します。ポッドを追加するとスループットが追加されます。`./app queue:work --max-jobs N` はワーカーを N ジョブ後に終了させるため、スーパーバイザーはプロセスをローテーションできます - リリース時のデプロイに役立ちます。

サブシステムごとの詳細については、[キュー](queues.md)、[スケジューリング](scheduling.md)、および [ワークフロー](workflows.md) を参照してください。

## クリーンに停止する

すべての長時間実行の Suprnova プロセス（サーバーおよび 3 つのデーモン）は、SIGINT と同様に **SIGTERM** でドレインされます。SIGTERM は `docker stop`、Coolify、systemd、および Kubernetes が送信するものです。SIGINT は Ctrl-C が送信するものです。どちらも同じパスをたどります：新しい作業の受け入れを停止し、設定された猶予期間内に進行中のものを終了し、`0` で終了します。

猶予期間はサブシステムごとで、意図的に設定されています。1 つのスロークライアントまたは 1 つの長時間タスクは、プロセスを無期限に生かしておくことができません。

| プロセス | 待機対象 | 猶予 |
|---|---|---|
| `serve` | 進行中の HTTP 接続 | 5s |
| `queue:work` | 進行中のジョブが解決されるまで | ジョブが返されるまで |
| `schedule:work` | `.run_in_background()` タスク | 30s |
| `workflow:work` | 進行中のワークフローステップ | ステップが返されるまで |

**これらの上にプラットフォームの終了猶予をサイズします。** Docker のデフォルトは 10 秒、Kubernetes は 30 秒です。プラットフォームのウィンドウが作業時間より短い場合は、SIGKILL を送信し、進行中のジョブを失うことに戻ります。

```yaml
# docker compose
services:
  worker:
    command: ["app", "queue:work"]
    stop_grace_period: 60s
```

```yaml
# kubernetes
spec:
  terminationGracePeriodSeconds: 60
```

**進行中に強制終了されたジョブは失われませんが、1 回の試行にはかかります。** その予約は失効し、別のワーカーがそれを再び要求します。信頼できるワーカーを強制終了するジョブは、永遠にサイクルするのではなく、デッドレターとしてそのまま使用できます。[キュー](queues.md#what-counts-as-an-attempt) を参照してください。

**PID 1 は実際の制約です。** コンテナエントリポイントは PID 1 として実行され、カーネルは PID 1 にデフォルト信号処理を適用しません。SIGTERM ハンドラなしのプロセスは SIGTERM では死なず、プラットフォームがあきらめて SIGKILL を送信するまで無視します。Suprnova はハンドラをインストールするため、`CMD ["app", "queue:work"]` は書き込まれたままで問題なく、`tini` シムは必要ありません。

## ヘルスチェック

Suprnova は 3 つの組み込みヘルスパスを公開します。`_suprnova/` プレフィックスは予約されており、独自のルートがそれらと衝突することはありません。

| パス | タッチ | 使用目的 |
|---|---|---|
| `/_suprnova/health/live` | なし | ライブネス。プロセスがリクエストを処理できる限り 200 で応答します。 |
| `/_suprnova/health/ready` | データベース | レディネス。依存関係に到達できない場合は 503。 |
| `/_suprnova/health` | なし、または `?db=true` のデータベース | 元のエンドポイント。上記のいずれかの動作をします。 |

```bash
curl http://localhost:8765/_suprnova/health/live
# 200 {"status":"ok","timestamp":"2026-05-30T12:34:56+00:00"}

curl http://localhost:8765/_suprnova/health/ready
# 正常: 200 {"status":"ok","timestamp":"…","database":"connected"}
# 劣化: 503 {"status":"degraded","timestamp":"…","database":"error"}
```

`/_suprnova/health` および `/_suprnova/health?db=true` は以前と同じように機能し、既にデプロイしたものは何も変更する必要はありません。[Hetzner ガイド](deployment-hetzner.md)は 1 回限りのチェックのためにそれらの名前を付けたままであり、独自の仕様も同様かもしれません。名前付きパスはより明確なため、新しい設定ではそれらを優先します。[Railway](deployment-railway.md)、[DigitalOcean](deployment-digital-ocean.md)、および [Docker](cli-docker.md) ガイドはそれらを使用しています。

### 適切な質問に対して適切なプローブを使用する

ライブネスを `/live` に、レディネスを `/ready` に指定します。違いはそれが見た目以上に重要です。失敗した**ライブネス**プローブはポッドを再起動しますが、失敗した**レディネス**プローブはロードバランサーから外すだけです。データベースチェックをライブネスに組み込むと、データベースが一瞬詰まっただけで手持ちのレプリカが軒並み再起動します。しかもそれは、再接続が一斉に殺到する事態にデータベースが最も耐えられない、まさにその瞬間です。

```yaml
livenessProbe:
  httpGet:
    path: /_suprnova/health/live
    port: 8765
readinessProbe:
  httpGet:
    path: /_suprnova/health/ready
    port: 8765
```

エンドポイントはミドルウェアチェーンの前にショートサーキットするため、ミドルウェアがデッドロックしたり、リクエスト ID ミドルウェアがトラフィックを拒否していても応答性を保ちます。

### デグレード応答はドライバー詳細を含まない

503 本体は `"database":"error"` と何も報告しません。ドライバー自身のメッセージ（ホスト、ポート、データベース、スキーマ名、サーバーバージョン、および一部の設定エラーの接続 URL を命名）は `error!` レベルのログに移動し、オペレーターはそれを読むことができ、知らない人は読めません。デバッグビルドでは、`database_error` として本体にも含まれるため、ローカルデバッグは影響を受けません。

### レディネスを閉じる

レディネスは、要求する者に対してデータベースラウンドトリップを実行します。エンドポイントがインターネット到達可能な場合は、共有シークレットを設定します：

```bash
SERVER_HEALTH_READINESS_TOKEN=<a long random string>
```

その後、プローブはそれをヘッダーとして送信する必要があります：

```bash
curl -H "X-Suprnova-Health-Token: $SERVER_HEALTH_READINESS_TOKEN" \
  http://localhost:8765/_suprnova/health/ready
```

```yaml
readinessProbe:
  httpGet:
    path: /_suprnova/health/ready
    port: 8765
    httpHeaders:
      - name: X-Suprnova-Health-Token
        value: <the same value>
```

ヘッダーがない場合、レディネスは **404** で応答します。存在しないパスと同じ応答であるため、エンドポイントは単に閉じられているのではなく見えません。ライブネスはどちらの方法でも公開のままなので、再起動時信号を保つために秘密をあらゆるマニフェストに入れる必要はありません。

未設定がデフォルトであり、レディネスは公開されています。それは意図的です。このマニュアルとスキャフォルダーが生成する設定はすべてヘッダーなしで `?db=true` を呼び出し、デフォルトが閉じられるとそれらは破壊されます。

## メンテナンスモード

破壊的なマイグレーションを流したり、インシデントのためにトラフィックを静止させたりするには：

```bash
./app down --secret abc123 \
           --retry 60 \
           --message "Deploying - back in a few minutes" \
           --except /webhooks/stripe

./app up
```

`down` は、ミドルウェアがすべてのリクエストで読み取るメンテナンスマーカーを書き込みます。リクエストは、指定されたメッセージとともに 503（`--status` 経由で設定可能）を受け取ります。ただし、`--except` のパスと、シークレットを含むリクエストは除きます。`up` はマーカーを削除します。

シークレットはベアラー認証情報です。`/<secret>` を訪れた者には誰でも、12 時間のバイパスクッキーが発行されます。URL のマッチもクッキーのマッチもどちらも定数時間比較であるため、レスポンスのタイミングが、どれだけの長さのプレフィックスを正しく推測したかをプローブする者に教えることはありません。`--secret` に覚えやすい文字列を選ぶよりも、あなたの代わりに 1 つを鋳造し（ランダムな 16 バイト、16 進で 32 文字）バイパス URL を出力する `--with-secret` を優先してください - そして、それをインシデントノートの中の他のあらゆる認証情報と同じように扱ってください。

## スケーリング

### Web

水平スケーリングはデフォルトのストーリーです。すべてのポッドが `./app` を実行し、`DATABASE_URL` を共有し、同じ Redis に接続します（Redis バックアップキャッシュ/キュー/セッションを設定した場合）。上記のアドバイザリロックのため、自動マイグレーションは安全です。スティッキーセッションは必要ありません。セッション状態はセッションドライバー（データベースまたは Redis）に存在し、プロセスメモリには存在しません。

### ワーカー

- **スケジューラー。** 常に正確に 1 つのインスタンス。
- **キュー。** 水平にスケーリングします。複数の名前付きキューに作業を分割した場合は、キューごとにワーカーを実行します（またはドライバー固有のキューフィルターを渡します - [キュー](queues.md) を参照）。
- **ワークフロー。** 水平にスケーリングします。行レベルのクレーム/ハートビートがワーカーを調整します。

## 接続キャップ（`SERVER_MAX_CONNECTIONS`）

デフォルトでは、サーバーは無制限の数の同時 TCP 接続を受け入れます。ほとんどのデプロイでは、リバースプロキシ（nginx、Caddy、Traefik）またはプラットフォームのロードバランサーが最初の防御線を提供します。単一の不正な動作クライアントプールがファイルディスクリプタを枯渇させるのを防ぐために、プロセス自体内で強固な最終手段が必要な場合は、`SERVER_MAX_CONNECTIONS` を設定します：

```bash
# .env.production - 同時接続を 1024 に制限
SERVER_MAX_CONNECTIONS=1024
```

キャップに達すると、**アクセプトループはブロック**され（TCP レベルのバックプレッシャー）、既存の接続が閉じるまで待ちます。保留中のハンドシェイクはカーネルのアクセプトバックログに残ります。許可証は各接続の全期間保持され、接続が終了した時点で解放されるため、スロットはすぐにターンオーバーします。

経験則：

- **未設定（デフォルト = 無制限）。** リバースプロキシが独自の接続制限を適用している場合、または同時実行を管理する PaaS の背後で実行している場合は正確です。
- **具体的な値に設定**プロセスがインターネット上で直接実行されている場合、またはプロキシ設定に関係なく多層防御が必要な場合。一般的な開始点は、予想ピーク同時ユーザーの 2 倍で、長期接続（WebSocket、SSE）に対して上方調整されます。
- **`LimitNOFILE`（systemd）または `ulimit -n` とペアにします。** OS ファイルディスクリプタ制限が予期しないキャップにならないようにします。各 HTTP 接続は 1 つのファイルディスクリプタがかかります。データベースプールサイズと OS ハウスキーピング用にそれに数十を追加します。
- **これはアップストリームレート制限の代替ではなく、バックストップです。** `SERVER_MAX_CONNECTIONS` は暴走蓄積を停止します。リバースプロキシまたは `rate_limit` ミドルウェアは、クライアントごとまたは IP ごとのスロットリングを処理する必要があります。

空白、解析不可、またはゼロの値は、タイプミスがサーバーの起動を防がないように、暗黙的に未設定として扱われます。

## プラットフォーム別ウォークスルー

上記のレシピは、すべての最新の PaaS または VPS に移植されます。次の 3 つのチャプターでは、詳細な説明が行われます：

| プラットフォーム | スタイル | ウォークスルー |
|---|---|---|
| Railway | git からの自動デプロイ付き PaaS | [Railway にデプロイ](deployment-railway.md) |
| Digital Ocean | App Platform（PaaS）または Droplets（VPS） | [Digital Ocean にデプロイ](deployment-digital-ocean.md) |
| Hetzner | systemd + Caddy 付き VPS | [Hetzner VPS にデプロイ](deployment-hetzner.md) |

## 次のステップ

- [環境変数](env-vars.md) - フレームワークが読み取るすべての環境変数
- [暗号化](encryption.md) - `APP_KEY`、キーローテーション、暗号化される内容
- [設定](configuration.md) - 環境の上に構築された型付き設定セクション
- [データベース](database.md) - ドライバー選択、プール調整、複数接続分割
- [キュー](queues.md) - ワーカースケーリングとキュードライバー
