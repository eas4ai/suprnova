# Hetzner VPS にデプロイ

このガイドは、Hetzner Cloudを使ってSuprnovaアプリケーションをVPSにデプロイする方法を扱います。同じ原則は、Linode、Vultr、AWS EC2、あるいは既に所有している専用サーバーなど、単一マシンのホスト全般にあてはまります。マシンをフルコントロールしたい、月額コストを予測可能にしたい、そして同じマシンにPostgres / Redisを同居させたいなら、この方法を選んでください。

このガイド全体を通じて、プロジェクト名には`myapp`、ドメインには`myapp.com`を使います - あなた自身のものに置き換えてください。

## 前提条件

- Ubuntu 22.04またはDebian 12を実行しているVPS
- サーバーへのSSHアクセス
- サーバーのIPアドレスを指しているドメイン名
- Suprnovaプロジェクト - 動作するソースツリー、あるいは`suprnova docker:init`で生成したDockerfileのどちらか（[Docker](cli-docker.md)を参照）

## サーバーのセットアップ

### 1. VPSを作成する

1. [Hetzner Cloud Console](https://console.hetzner.cloud)に移動します
2. 新しいプロジェクトを作成し、サーバーを追加します
3. イメージとして**Ubuntu 22.04**を選びます
4. サーバーサイズを選びます（小さなアプリにはCX11で十分です）
5. 安全なアクセスのために、SSHキーを追加します

### 2. 初期サーバー設定

サーバーにSSH接続し、初期セットアップを実行します：

```bash
# パッケージを更新します
apt update && apt upgrade -y

# アプリ用のnon-rootユーザーを作成します
useradd -m -s /bin/bash app
mkdir -p /opt/myapp
chown app:app /opt/myapp

# 必要なパッケージをインストールします
apt install -y curl postgresql redis-server
```

### 3. PostgreSQLを設定する

```bash
# データベースとユーザーを作成します
sudo -u postgres psql << EOF
CREATE USER myapp WITH PASSWORD 'your_secure_password';
CREATE DATABASE myapp_production OWNER myapp;
GRANT ALL PRIVILEGES ON DATABASE myapp_production TO myapp;
EOF
```

> **ヒント：**
>
> 本番環境では、信頼性とバックアップを向上させるために、Hetznerの近日公開予定のマネージドPostgreSQLや、Neon、Supabase、AWS RDSのようなサービスなど、マネージドデータベースサービスの利用を検討してください。


## デプロイオプション

以下のデプロイ方法のうち、どれか1つを選んでください。どの方法も、最終的には`/opt/myapp/app`に置かれた`app`という名前のバイナリ（あるいはコンテナ）になり、下記のsystemdユニットはそれの実行の仕方を知っています。

### オプションA：ローカルでビルドする

あなたのマシンでビルドし、バイナリをアップロードします。`myapp`を、あなたの実際のプロジェクト名に置き換えてください - `cargo build`は、`Cargo.toml`の`[package].name`にちなんでバイナリに名前を付けます：

```bash
# あなたのローカルマシンで - Linux向けにクロスコンパイルします（macOSの場合）
cargo build --release --target x86_64-unknown-linux-gnu

# あるいは、Linux向けにDockerでビルドします（Dockerfileがバイナリを`app`にリネームします）
docker build -t myapp .
docker create --name temp myapp
docker cp temp:/app/app ./app-linux
docker rm temp

# サーバーにアップロードし、到着時に`app`にリネームします
scp target/x86_64-unknown-linux-gnu/release/myapp root@your-server:/opt/myapp/app
# あるいは、Dockerルートを選んだ場合：
scp ./app-linux root@your-server:/opt/myapp/app
```

### オプションB：サーバー上でビルドする

Rust 1.91.1+をインストールし（Suprnovaは2024エディションを使います）、サーバー上で直接ビルドします：

```bash
# Rustをインストールします
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env

# クローンし、ビルドし、バイナリを標準のパスに置きます
cd /opt/myapp
git clone https://github.com/your-username/your-repo.git .
cargo build --release
cp target/release/myapp ./app   # systemdのExecStart=/opt/myapp/appが見つけられるようにリネームします
```

### オプションC：Dockerを使う

あなたのアプリをDockerコンテナの中で実行します - スキャフォルドされたDockerfileは、既にランタイムバイナリを`app`と名付けています（[Docker](cli-docker.md)を参照）：

```bash
# Dockerをインストールします
curl -fsSL https://get.docker.com | sh

# イメージをプルして実行します
docker run -d \
  --name myapp \
  --restart unless-stopped \
  -p 8765:8765 \
  --env-file /opt/myapp/.env.production \
  your-registry/myapp:latest
```

Dockerを選んだ場合は、systemdのセクションを飛ばして[Caddyリバースプロキシ](#caddyリバースプロキシ)へ進んでください - プロセスの監督はDockerが行います。

## 環境設定

まず、サーバー上で（あるいはローカルで - 大事なのは値そのものです）、本番用の`APP_KEY`を生成します。`APP_KEY`は、セッションクッキーと署名付きURLのために`suprnova::Crypt`が使う、32バイトのAES-256キーです。`APP_ENV`が`local`/`dev`/`test`のいずれでもなく、`APP_KEY`が未設定の場合、Suprnovaは**ブート時に失敗状態になります** - そのため、本番環境ではこれはオプションではありません：

```bash
suprnova key:generate --show
# -> APP_KEY=base64-url-safe-32-bytes
```

次に、envファイルを書き込みます：

```bash
cat > /opt/myapp/.env.production << 'EOF'
APP_NAME="My App"
APP_ENV=production
APP_DEBUG=false
APP_URL=https://myapp.com
APP_KEY=paste-the-generated-key-here

SERVER_HOST=127.0.0.1
SERVER_PORT=8765

# データベース - DBが同じマシン上にある場合は、localhostにバインドします
DATABASE_URL=postgres://myapp:your_secure_password@localhost:5432/myapp_production
DB_MAX_CONNECTIONS=10
DB_MIN_CONNECTIONS=1

# セッション
SESSION_SECURE=true
SESSION_SAME_SITE=Lax

# Redis（オプション - キャッシュ、キュー、ブロードキャストのドライバーが使います）
REDIS_URL=redis://127.0.0.1:6379

# メール
MAIL_DRIVER=smtp
MAIL_HOST=your-smtp-host
MAIL_PORT=587
MAIL_USERNAME=
MAIL_PASSWORD=
MAIL_FROM_ADDRESS=hello@myapp.com
MAIL_FROM_NAME="My App"
EOF

# ファイルを保護します - appユーザーだけが読み取れるようにします
chmod 600 /opt/myapp/.env.production
chown app:app /opt/myapp/.env.production
```

完全な環境変数のサーフェスと、それがどのように型付き設定になるかについては、[設定](configuration.md)を参照してください。

## systemdサービス

Suprnovaのバイナリは、複数のコマンドをサポートします - `./app`（自動マイグレーション付きのserve）、`./app schedule:work`（スケジューラーデーモン）、`./app queue:work`（キューワーカー）、`./app workflow:work`（ワークフローランナー）です。それぞれの長時間実行プロセスは、同じバイナリとenvファイルを使う、独自のsystemdユニットを持ちます。

### Webサーバーサービス

`/etc/systemd/system/myapp.service`を作成します：

```ini
[Unit]
Description=Suprnova Application
After=network.target postgresql.service redis.service
Requires=postgresql.service

[Service]
Type=simple
User=app
Group=app
WorkingDirectory=/opt/myapp
ExecStart=/opt/myapp/app
Restart=always
RestartSec=5

# 環境
EnvironmentFile=/opt/myapp/.env.production

# セキュリティ強化
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ReadWritePaths=/opt/myapp

[Install]
WantedBy=multi-user.target
```

デフォルトの`ExecStart=/opt/myapp/app`は、自動マイグレーション付きで`serve`を実行します。マイグレーションを別個のデプロイステップにしたい場合は、`ExecStart=/opt/myapp/app serve --no-migrate`を使い、バイナリを切り替える前に、デプロイスクリプトから`./app migrate`を実行してください。

### スケジューラーサービス

あなたのアプリが`Schedule::call(...)`を介して登録されたタスクを持つ場合（[スケジューリング](cli-scheduling.md)のチャプターを参照）、重複したタスク実行を避けるために、スケジューラープロセスを**正確に1つ**実行してください。`/etc/systemd/system/myapp-scheduler.service`を作成します：

```ini
[Unit]
Description=Suprnova Scheduler
After=network.target myapp.service
Requires=myapp.service

[Service]
Type=simple
User=app
Group=app
WorkingDirectory=/opt/myapp
ExecStart=/opt/myapp/app schedule:work
Restart=always
RestartSec=5

# 環境
EnvironmentFile=/opt/myapp/.env.production

# セキュリティ強化
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ReadWritePaths=/opt/myapp

[Install]
WantedBy=multi-user.target
```

### キューワーカー（オプション）

ジョブをキューにディスパッチする場合は、`/etc/systemd/system/myapp-queue.service`を追加します：

```ini
[Unit]
Description=Suprnova Queue Worker
After=network.target myapp.service
Requires=myapp.service

[Service]
Type=simple
User=app
Group=app
WorkingDirectory=/opt/myapp
ExecStart=/opt/myapp/app queue:work
Restart=always
RestartSec=5

EnvironmentFile=/opt/myapp/.env.production

NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ReadWritePaths=/opt/myapp

[Install]
WantedBy=multi-user.target
```

キューワーカーは水平にスケールできます - 同じマシン上でも、異なるマシン上でも、複数の`myapp-queue.service`インスタンスは安全です。

### サービスを有効化して起動する

```bash
# ユニットファイルを書いた後、systemdをリロードします
systemctl daemon-reload

# ブート時に起動するように、サービスを有効化します
systemctl enable myapp
systemctl enable myapp-scheduler
systemctl enable myapp-queue        # キューワーカーを追加した場合

# 今すぐ起動します
systemctl start myapp
systemctl start myapp-scheduler
systemctl start myapp-queue

# 確認します
systemctl status myapp
systemctl status myapp-scheduler
systemctl status myapp-queue
```

## Caddyリバースプロキシ

Caddyは、Let's Encryptを使って、HTTPS証明書を自動的に処理します。

### Caddyをインストールする

```bash
apt install -y debian-keyring debian-archive-keyring apt-transport-https curl
curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/gpg.key' | gpg --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg
curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt' | tee /etc/apt/sources.list.d/caddy-stable.list
apt update
apt install caddy
```

### Caddyを設定する

`/etc/caddy/Caddyfile`を編集します：

```
myapp.com {
    reverse_proxy localhost:8765

    # 圧縮を有効化します
    encode gzip

    # ロギング
    log {
        output file /var/log/caddy/myapp.log
    }
}
```

`myapp.com`を、あなたの実際のドメインに置き換えてください。

### Caddyを起動する

```bash
systemctl enable caddy
systemctl start caddy
```

Caddyは、SSL証明書を自動的に取得し、更新します。

## ヘルスチェック

Suprnovaは、ミドルウェアチェーンの前でショートサーキットし、あなたのルートと決して衝突しない、組み込みの`/_suprnova/health`エンドポイントを提供します：

```bash
curl https://myapp.com/_suprnova/health
```

```json
{
  "status": "ok",
  "timestamp": "2026-05-30T10:30:00Z"
}
```

### データベースの接続性を確認する

データベースも検証するには、`?db=true`を追加します：

```bash
curl https://myapp.com/_suprnova/health?db=true
```

正常な応答（HTTP 200）：

```json
{
  "status": "ok",
  "timestamp": "2026-05-30T10:30:00Z",
  "database": "connected"
}
```

データベースチェックが失敗すると、エンドポイントは`"status": "degraded"`と`"database_error"`フィールドを伴う、HTTP **503**に切り替わります - ロードバランサーが不健全なインスタンスをローテーションから外せるように、これを`livenessProbe` / `readinessProbe`スタイルのヘルスチェックに配線してください。

### 外部モニタリング

モニタリングサービスと共に、ヘルスエンドポイントを使ってください：

- **UptimeRobot：** `https://myapp.com/_suprnova/health`用のHTTPモニターを追加します
- **Better Stack**（旧Better Uptime）：503トリガー付きで、ヘルスチェックエンドポイントを設定します
- **Prometheus / Grafana：** `status`と`database`フィールドについて、JSONボディをスクレイプします

## デプロイスクリプト

アトミックな更新のために、デプロイスクリプトを作成します。`myapp`を、あなたのプロジェクト名（`Cargo.toml`の`[package].name`）に置き換えてください - それが、`cargo build`が出力バイナリに付ける名前です：

```bash
#!/bin/bash
# deploy.sh - あなたのローカルマシンで実行します

set -e

PROJECT="myapp"               # Cargoパッケージ名
SERVER="root@your-server"
APP_PATH="/opt/myapp"
BIN="target/x86_64-unknown-linux-gnu/release/$PROJECT"

echo "アプリケーションをビルドしています..."
cargo build --release --target x86_64-unknown-linux-gnu

echo "バイナリをアップロードしています..."
scp "$BIN" "$SERVER:$APP_PATH/app.new"

echo "デプロイしています..."
ssh "$SERVER" << 'EOF'
    set -e
    cd /opt/myapp

    # 長時間実行のサービスを停止します（初回デプロイでの失敗は無視します）
    systemctl stop myapp-queue || true
    systemctl stop myapp-scheduler || true
    systemctl stop myapp

    # アトミックな入れ替え - 同じファイルシステム上では、リネームは単一のシステムコールです
    mv app.new app
    chmod +x app

    # マイグレーションを明示的に実行します（ユニットも自動マイグレーションを
    # 行いますが、ここで実行することで、トラフィックを再開する前に失敗を
    # 表面化させます）
    sudo -u app ./app migrate

    # サービスを起動します
    systemctl start myapp
    systemctl start myapp-scheduler || true
    systemctl start myapp-queue || true

    # ヘルスを確認します（サーバーがバインドするまで少し待ちます）
    sleep 2
    curl -fsS http://localhost:8765/_suprnova/health?db=true > /dev/null || exit 1

    echo "デプロイが完了しました！"
EOF
```

実行可能にします：

```bash
chmod +x deploy.sh
./deploy.sh
```

## ログとモニタリング

### ログを見る

```bash
# Webサーバーのログ
journalctl -u myapp -f

# スケジューラーのログ
journalctl -u myapp-scheduler -f

# Caddyのアクセスログ
tail -f /var/log/caddy/myapp.log
```

### ログローテーション

systemdのjournaldは、ログローテーションを自動的に処理します。長期保存には、以下を検討してください：

- **Loki + Grafana：** セルフホストのログ集約
- **Papertrail：** クラウドベースのロギングサービス
- **Logtail：** シンプルなログ管理

## ファイアウォール設定

UFWで、あなたのサーバーを保護します：

```bash
# SSHを許可します
ufw allow 22/tcp

# HTTP/HTTPSを許可します（Caddy）
ufw allow 80/tcp
ufw allow 443/tcp

# ファイアウォールを有効化します
ufw enable
```

> **警告：**
>
> ポート8765を直接公開しないでください。SSLとセキュリティヘッダーの処理には、常にCaddyをリバースプロキシとして使ってください。


## スケーリング

単一のSuprnovaバイナリは非常に効率的です - スケールアウトが必要になるまでに、小さなVPSでも驚くほどの量のトラフィックを処理できます。その時が来たら：

### 垂直スケーリング

より多くのCPU/メモリのために、VPSをより大きなインスタンスにアップグレードします。バイナリ、envファイル、systemdユニットは、変更なしにそのまま持っていけます。

### 水平スケーリング

複数のアプリケーションインスタンスのためには：

1. ロードバランサーを設定します（Hetzner Load Balancer、HAProxy、あるいは専用ノード上のCaddy）
2. アプリのマシンをステートレスに保てるよう、Postgresをマネージドサービスか専用ノードに移します
3. どのアプリインスタンスでもどのリクエストにも応答できるよう、セッション、キャッシュ、ブロードキャストをRedisに移します
4. 複数のアプリインスタンスをデプロイします。それぞれが、ブート時に自身の自動マイグレーションを安全に実行します（マイグレーションランナーがロックを取るため、同時に起動しても衝突しません）
5. フリート全体を通じて、スケジューラー（`schedule:work`）を**1つ**だけ稼働させ続けます - キューワーカーは並行して実行しても安全ですが、スケジューラーはそうではありません

### Suprnovaが異なる設計を選んだ理由

Laravelは通常、nginxの背後でPHP-FPMを実行し、cronが1分ごとに`schedule:run`をトリガーし、Horizon（またはsupervisord）がキューワーカーを管理します。Suprnovaは、これを1つのバイナリとサブコマンドへと折りたたみます。`./app`は長時間生存するTokioプロセスです - 前段にプロセスプールを必要とせず、別個のcronも必要とせず、リクエストをまたいでウォームな状態を保ちます。systemdは、Webプロセスとワーカーの両方のスーパーバイザーであり、Caddyは、nginxが避けられなかったこと、つまりTLSの終端とプロキシだけを行います。

## サイジング

マーケティング上のティア名ではなく、ワークロードに基づいてVPSを選んでください。Hetznerのラインナップは定期的に変わりますが、サイジングのロジックは変わりません：

| ワークロード | おおよその目安 |
|---|---|
| 小規模サイト、低トラフィック、SQLiteまたは共有DB | 最小の共有vCPUインスタンス（1 vCPU / 2 GB） |
| 同じマシン上のPostgres + Redisを伴う、中程度のトラフィック | 2 vCPU / 4 GB |
| より重いAPI + スケジューラー + キューワーカー + Postgres | 2–4 vCPU / 8 GB |
| 大規模な本番環境 | 専用CPUインスタンス、あるいはDBを独自のノードへ分割 |

最新のカタログについては、Hetznerの[現在の価格](https://www.hetzner.com/cloud)を確認してください。Suprnovaのアイドル時のメモリフットプリントは小さく（一桁MB台）、そのためRAMのほとんどは、データベースのワーキングセットとあなたのドメインコードが占めます。

## トラブルシューティング

### サービスが起動しない

エラーについて、ログを確認してください：

```bash
journalctl -u myapp -n 50
```

よくある問題：
- 環境変数の不足
- データベース接続の失敗
- ポートが既に使用中

### Caddyの証明書エラー

以下を確認してください：
- ドメインのDNSが、あなたのサーバーを指している
- ポート80と443が開いている
- 他のサービスがポート80を使用していない

```bash
caddy validate --config /etc/caddy/Caddyfile
```

### データベース接続の問題

手動で接続をテストします：

```bash
sudo -u app psql $DATABASE_URL -c "SELECT 1"
```

### ヘルスチェックが失敗する

```bash
# アプリが実行中か確認します
systemctl status myapp

# ヘルスエンドポイントを直接テストします
curl http://localhost:8765/_suprnova/health

# データベースありで確認します
curl http://localhost:8765/_suprnova/health?db=true
```

`"status": "degraded"`を伴う`503`応答は、アプリは起動しているものの、データベースのヘルスチェックが失敗したことを意味します - ボディの中の`database_error`を調べ、`DATABASE_URL`、Postgresのログ、そして接続数の上限を確認してください。

## 次のステップ

- [デプロイメント 概要](deployment.md) - 単一バイナリのデプロイに関する、プラットフォームに依存しない話
- [Docker](cli-docker.md) - `docker:init`と`docker:compose`の詳細
- [設定](configuration.md) - 完全な環境変数のサーフェスと型付き設定
- [Railway にデプロイ](deployment-railway.md) - 自動ビルド付きのPaaSという選択肢
- [Digital Ocean にデプロイ](deployment-digital-ocean.md) - マネージドインフラストラクチャを備えたApp Platform
