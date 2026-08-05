# Digital Ocean にデプロイ

Digital Oceanには、Suprnovaアプリに適した2つの本番環境ターゲットがあります：**App Platform**（マネージドDocker PaaS - プッシュして後は任せるだけ）と**Droplet**（あなた自身のVPSで、すべてを自分で管理します）です。このチャプターでは、両方を扱います。マネージドデータベース、自動デプロイ、そしてSSLの処理を任せたいならApp Platformを使ってください。フルコントロールが欲しい、既にそのマシン上で他のサービスを動かしている、あるいはトラフィックにかかわらず料金を一定に保ちたいなら、Dropletを使ってください。

## 前提条件

- [Digital Oceanアカウント](https://www.digitalocean.com)
- Dockerfileを持つSuprnovaプロジェクト - 以下で生成します：
  ```bash
  suprnova docker:init
  ```
- 本番環境用の`APP_KEY`。一つ生成し、安全なところに保管してください：
  ```bash
  suprnova key:generate --show
  ```
  `APP_ENV`が`local` / `development` / `testing`以外で、`APP_KEY`が未設定の場合、Suprnovaはブート時に失敗状態になります。
- gitリポジトリ（GitHubまたはGitLab） - App Platformには必須です。Dropletの場合は、ビルド済みのイメージをレジストリにプッシュすることもできます。

## App Platform

App Platformは、あなたのDockerfileをビルドし、単一のSuprnovaバイナリを実行し、望むならマネージドPostgresを提供します。

### 1. アプリを作成する

1. [Digital Ocean Apps](https://cloud.digitalocean.com/apps)に移動します。
2. **Create App**をクリックし、GitHub/GitLabを接続して、リポジトリとブランチを選びます。
3. App Platformは、リポジトリのルートにある`Dockerfile`を自動検出します。

### 2. Webサービスを設定する

| 設定 | 値 |
|---|---|
| リソースタイプ | Web Service |
| HTTPポート | `8765` |
| 実行コマンド | 空のままにします - Dockerfileの`CMD`が`./app`を実行します |
| ヘルスチェック（HTTPパス） | `/_suprnova/health/live` |

デフォルトのSuprnovaバイナリは、自動マイグレーション付きで`serve`を実行するため、コンテナは起動時にマイグレーションを実行してから、リスナーをバインドします。

### 3. マネージドPostgresを追加する

1. **Add Resource** -> **Database** -> **PostgreSQL**を選択します。
2. プランを選びます（テスト用にはDev Database、実際のトラフィックにはProductionプラン）。

App Platformは、`${db.DATABASE_URL}`バインディングを介して、`DATABASE_URL`をすべてのコンポーネントへ自動的に注入します。

### 4. 環境変数

Webコンポーネントの**Environment Variables**セクションで、以下を設定します：

| 変数 | 値 | ノート |
|---|---|---|
| `APP_ENV` | `production` | フェイルクローズの`APP_KEY`チェックを発動させます |
| `APP_KEY` | `suprnova key:generate --show`の出力 | **encrypted**としてマークします |
| `SERVER_HOST` | `0.0.0.0` | すべてのインターフェースにバインドします |
| `SERVER_PORT` | `8765` | Dockerfileの`EXPOSE`に一致します |
| `APP_URL` | `https://your-app.ondigitalocean.app` | Inertiaと署名付きURLで使われます |

`DATABASE_URL`は、マネージドデータベースのバインディングによって自動的に提供されます。手動で設定しないでください。

キャッシュやセッションにRedisを使う場合は、マネージドRedisクラスターを追加し、`REDIS_URL`をそのバインディング値（`${redis.REDIS_URL}`）に設定してください。

### 5. デプロイする

**Create Resources**をクリックします。最初のビルドは数分かかります（Rustのリリースビルド + フロントエンドのビルド）。それ以降のビルドは、Dockerfileのレイヤーキャッシュを使うため、ずっと高速です。

### スケジューラーワーカーを追加する

スケジュールされたタスク（`Schedule::call`を介して登録される`#[derive(Task)]`ハンドラ）には、長時間生存するプロセスが必要です。同じイメージを異なるコマンドで実行する、Workerコンポーネントを追加します：

1. **Create** -> **Add Resource** -> **Detect from source code**を選び、同じリポジトリを選択します。
2. リソースタイプを**Worker**に設定します。
3. **Run command**：
   ```bash
   ./app schedule:work
   ```
4. ワーカーは、`DATABASE_URL`や`APP_KEY`を含む、アプリからの環境変数を引き継ぎます。

ワーカーはHTTPトラフィックを受け取りません。ワーカーインスタンスを正確に**1つ**実行してください - 複数のスケジューラーは、各タスクを複数回実行してしまいます。

キューワーカー（`./app queue:work`）についても、パターンは同一です。キュードライバーがどのワーカーがどのジョブを取るかを調整するため、通常は複数のキューワーカーを安全に実行できます。[キュー](queues.md)を参照してください。

### アプリ仕様（Infrastructure as Code）

再現可能なデプロイのために、`.do/app.yaml`をコミットします：

```yaml
name: my-suprnova-app

services:
  - name: web
    dockerfile_path: Dockerfile
    github:
      repo: your-username/your-repo
      branch: main
      deploy_on_push: true
    http_port: 8765
    instance_count: 1
    instance_size_slug: basic-xxs
    health_check:
      # ライブネス専用です - App Platformは、これが失敗するとコンテナを
      # 再起動するため、Postgresに依存してはいけません。トラブルシューティ
      # ングのヘルスチェックに関する注記を参照してください。
      http_path: /_suprnova/health/live
    envs:
      - key: APP_ENV
        value: production
      - key: APP_KEY
        scope: RUN_TIME
        type: SECRET
        value: ${APP_KEY}
      - key: SERVER_HOST
        value: 0.0.0.0
      - key: SERVER_PORT
        value: "8765"
      - key: APP_URL
        value: https://your-app.ondigitalocean.app
      - key: DATABASE_URL
        scope: RUN_TIME
        value: ${db.DATABASE_URL}

workers:
  - name: scheduler
    dockerfile_path: Dockerfile
    github:
      repo: your-username/your-repo
      branch: main
      deploy_on_push: true
    instance_count: 1
    instance_size_slug: basic-xxs
    run_command: ./app schedule:work
    envs:
      - key: APP_ENV
        value: production
      - key: APP_KEY
        scope: RUN_TIME
        type: SECRET
        value: ${APP_KEY}
      - key: DATABASE_URL
        scope: RUN_TIME
        value: ${db.DATABASE_URL}

databases:
  - name: db
    engine: PG
    version: "16"
    size: db-s-dev-database
```

`doctl` CLIでデプロイします：

```bash
doctl apps create --spec .do/app.yaml
```

シークレットの`APP_KEY`は、Apps UI経由で別途設定するか、あるいは：

```bash
doctl apps update <app-id> --spec .do/app.yaml \
  --set-env "APP_KEY=$(suprnova key:generate --show)"
```

### カスタムドメイン

**Settings** -> **Domains** -> **Add Domain**で、あなたのドメインを入力し、DNSの指示に従ってください。App Platformは、Let's Encrypt証明書を自動的に発行し、更新します。

ドメインが有効になったら、それに合わせて`APP_URL`を更新してください - InertiaはそれをX-Inertia-Locationヘッダーに使い、署名付きURLはそれをハッシュの入力に使います。

### スケーリング

- **水平：** Webサービスの**Instance Count**を上げます。各インスタンスはマネージドPostgresを共有します。複数のインスタンスが起動時に自動マイグレーションを実行しても安全です - SuprnovaはSeaORMのアドバイザリロック方式のマイグレーターを使っています。
- **垂直：** **Instance Size**を変更します。Rustのバイナリは、トラフィックの少ないアプリには最小のslugで十分機能します。WebSocketや長時間生存する接続を大規模に処理し始めたら、上げてください。

スケジューラーワーカーは、インスタンス数を**1**に保ってください。

## Droplet（VPS）

Dropletは、あなた自身のVPS上でSuprnovaを動かしたい場合の選択肢です。その仕組みは、他のLinux VPSと同一です - systemdサービス、Caddyリバースプロキシ、マネージドまたはセルフホストのPostgresです。[Hetzner VPS](deployment-hetzner.md)のチャプターが、そのパターンの決定版のウォークスルーです。そこに書かれていることはすべて、Dropletにもそのまま当てはまります。呼び出す価値のある違いは、以下だけです：

- **イメージ：** Dropletのコンソールで、**Ubuntu 24.04**または**Debian 12**を選びます。
- **データベース：** Postgres / MySQL / Redisを、Droplet上で自分で動かす代わりに、Digital Oceanの**Managed Databases**を使うこともできます - `DATABASE_URL` / `REDIS_URL`の話は同じです。マネージドのエンドポイントを指すようにすれば、Suprnovaはその違いに気づきません。
- **バックアップ：** DOのコンソールで、Dropletのスナップショットとマネージド DBの日次バックアップを有効にします。
- **ネットワーキング：** DOの**VPC**を使って、Dropletとマネージドデータベースをプライベートネットワーク上に保ちます。リスナーを`127.0.0.1`にバインドし、TLSのためにCaddyを前段に置きます。

Droplet上でDocker（システムバイナリの代わりに）を使いたい場合は、[Docker](cli-docker.md)のdocker-composeパターンがそのまま綺麗に収まります - セルフホストのPostgresをマネージドデータベースに差し替えれば完了です。

### Suprnovaが異なる設計を選んだ理由

Laravelの典型的なPHPデプロイには、PHP-FPM + opcache + キューランナー + スケジューラーのcronエントリが必要です - 少なくとも3つの可動部分があり、それぞれが独自の再起動のセマンティクスを持ちます。Suprnovaのデプロイは、単一のバイナリとオプションのワーカープロセスです。バイナリはマイグレーションを実行し、HTTPを提供し、WebSocketを処理し、リバースプロキシの背後で動きます。`./app schedule:work`や`./app queue:work`で呼び出される同じバイナリが、あなたのスケジューラーやキューワーカーになります。App Platformの「1つのイメージ、複数のコンポーネント」というモデルは、これに自然に適合します - すべてのコンポーネントで同じDockerfileを使い、ロールごとに異なる`run_command`を使うだけです。

## トラブルシューティング

### ビルドが失敗する

最初に確認すべきことは、Dockerfileがローカルでビルドできるかどうかです：

```bash
docker build -t myapp .
```

ローカルのビルドは動くのに、App Platformのビルドは動かない場合の、よくある原因：

- **ビルドコンテキストファイルの不足：** `.dockerignore`が`Cargo.lock`や`migrations/`ディレクトリを除外していないか確認してください。
- **cargoビルド中のメモリ不足：** App Settings -> Resources -> Buildで、ビルドインスタンスのサイズを上げてください。Rustのリリースビルドは、メモリを大量に消費します。

### アプリはブートするが、起動時にクラッシュする

**Runtime Logs**タブで、ランタイムログを確認してください。最もよくある2つのSuprnovaのブート失敗は：

- **`APP_KEY is required when APP_ENV=production`** - `suprnova key:generate --show`で1つ生成し、暗号化された環境変数として追加してください。
- **`SERVER_HOST=…`の値が不正** - App Platformでは`0.0.0.0`でなければならず、`127.0.0.1`ではいけません（ロードバランサーからループバックには到達できません）。

### ヘルスチェックが失敗する

プラットフォームは`/_suprnova/health/live`にpingを送り、設定されたタイムアウト内での200を期待します。失敗している場合：

- パスが正確に`/_suprnova/health/live`であることを確認してください（`/health`ではありません）。より古い`/_suprnova/health`も、あなたの仕様が既にその名前を使っている場合は引き続き動作します。
- ポートが`8765`であり、`SERVER_PORT`に一致することを確認してください。
- 「バインドできない」のか「Postgresに到達できない」のかを見分けるには、ヘルスチェックからではなく、コンソールから**手動で**データベースをプローブしてください：

  ```bash
  curl http://localhost:8765/_suprnova/health/ready
  # 正常:  200 {"status":"ok","database":"connected"}
  # 劣化: 503 {"status":"degraded","database":"error"}
  ```

  劣化した応答は、アプリはバインドできたがPostgresに到達できないことを意味します - `DATABASE_URL`のバインディングを確認してください。`-f`は渡さないでください：これは、まさにあなたが読み取ろうとしているケースである503で、curlを黙って終了させてしまいます。

データベースのプローブを、アプリ仕様の`health_check`に入れないでください。App Platformは、そのチェックが失敗するとコンテナを再起動するため、データベースの一時的な不調がアプリを道連れにしてしまいます - この障害モードは、まさにアプリに生き延びてほしいインシデントの最中に、再起動ループを引き起こします。[適切な質問に対して適切なプローブを使用する](deployment.md#use-the-right-probe-for-the-right-question)を参照してください。

### マイグレーションが実行されない

マイグレーションは、デフォルトの`./app`のブートの一部として自動的に実行されます。実行されていない場合は、ランタイムログでSeaORMのエラーを確認してください。App Platformのコンソールから手動で実行するには：

1. Webコンポーネントの**Console**タブを開きます。
2. `./app migrate`を実行します。

マイグレーションをブートパスから外しておきたい場合は、実行コマンドを`./app serve --no-migrate`に設定し、デプロイ前に`./app migrate`を実行するワンショットの**Job**を、アプリ仕様に追加してください。

## 次のステップ

- [デプロイメント 概要](deployment.md) - クロスプラットフォームなデプロイの入門（バイナリ、マイグレーション、スケジューラー、ヘルス）
- [Docker](cli-docker.md) - `suprnova docker:init`と`docker:compose`が何を生成するか
- [設定](configuration.md) - Suprnovaが読み込むすべての環境変数
- [環境変数](env-vars.md) - 本番環境で必須のものを含む、完全なリファレンス
- [Hetzner VPS にデプロイ](deployment-hetzner.md) - ここでのDropletのウォークスルーは、そのままあてはまります
