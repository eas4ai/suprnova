# Railway にデプロイ

[Railway](https://railway.app)は、あなたのDockerfileをビルドし、マネージドインフラストラクチャ上で実行する、Git駆動のPaaSです。RailwayのマネージドPostgresとRedisを組み合わせれば、面倒を見るべきサーバーが一つもない、完全なSuprnova本番スタックが手に入ります。このレシピは、`suprnova new`で作られたばかりのスキャフォルドアプリを、実際に動くURLへと導きます。

## 前提条件

- [Railwayアカウント](https://railway.app)
- GitHub、GitLab、あるいはBitbucketにプッシュされたSuprnovaプロジェクト
- リポジトリのルートにある`Dockerfile`と`.dockerignore`。以下で生成されます：
  ```bash
  suprnova docker:init
  ```
- Railwayの変数に貼り付けられる、生成済みの`APP_KEY`：
  ```bash
  suprnova key:generate --show
  ```

`suprnova`はローカルでのみ必要です - RailwayはDockerfileを自分自身でビルドします。フレームワークのクレートは、ビルド中に通常のcargo依存関係としてgitから取得されます。

## プロジェクトをプロビジョニングする

1. [Railwayダッシュボード](https://railway.app/dashboard)を開き、**New Project**をクリックして、**Deploy from GitHub repo**を選択します。
2. リポジトリを選びます。Railwayは`Dockerfile`を検出し、自動的に最初のビルドを開始します。
3. ビルド中に、データベースを追加します：**New** → **Database** → **Add PostgreSQL**。Railwayは`DATABASE_URL`を、プロジェクト上の参照変数として公開します。
4. アプリがRedisのキャッシュ、セッション、キュー、あるいはレートリミットのドライバーを使う場合は、同じ方法でRedisを追加することもできます（**New** → **Database** → **Redis**）。Railwayは接続URLを`REDIS_URL`として公開します。

## 変数を配線する

Webサービスを開き、**Variables**に移動して、本番環境の設定を追加します。ローテーションのたびに貼り直さなくて済むように、データベースサービスからURLを引き込むRailwayの`${{ }}`参照構文を使ってください。

```env
APP_ENV=production
APP_KEY=<paste the output of `suprnova key:generate --show`>
SERVER_HOST=0.0.0.0
SERVER_PORT=8765
DATABASE_URL=${{ Postgres.DATABASE_URL }}
REDIS_URL=${{ Redis.REDIS_URL }}
```

知っておく価値のあることがいくつかあります：

- **`APP_KEY`は、非開発環境では必須です。** `APP_ENV`が`local`/`dev`/`test`以外であり、`APP_KEY`が欠落しているか不正な形式である場合、Suprnovaはブート時に失敗状態になります。サーバーは対処方法を示すメッセージをログに記録し、ゼロ以外で終了します - Railwayはデプロイを失敗としてマークします。`suprnova key:generate --show`でキーを生成してください。
- **`SERVER_HOST=0.0.0.0`が必要です。** Railwayはコンテナのネットワークインターフェースを通じてトラフィックをルーティングします。`127.0.0.1`（ローカルのデフォルト）にバインドすると、接続拒否のように見えます。
- **`SERVER_PORT`はDockerfileの`EXPOSE`に一致します。** 生成されたDockerfileは8765を公開します。Railwayはそれを自動的に公開URLへとマッピングします。

## ビルドとデプロイ

Railwayは、接続されたブランチへのすべてのプッシュでビルドします。`docker:init`によって生成されたDockerfileは、以下を行います：

1. **ステージ1 - フロントエンド。** `frontend/`の中で`npm ci`と`npm run build`を実行します。Viteの出力は`frontend/dist/`に置かれます。
2. **ステージ2 - バックエンド。** あなたのワークスペースに対して`cargo build --release`を実行します。キャッシュされた依存関係レイヤーが、反復的なビルドを高速に保ちます。
3. **ステージ3 - ランタイム。** `ca-certificates`と`libssl3`を備えた`debian:bookworm-slim`イメージで、non-rootの`appuser`と、コンパイル済みの`./app`バイナリを持ちます。デフォルトの`CMD`は`./app`で、自動マイグレーション付きで`serve`を実行します。

最初のビルドは通常、数分かかります（コールドなRustキャッシュ）。それ以降のビルドは、Dockerのレイヤーキャッシュのおかげでずっと高速です。

## スケジューラーサービスを追加する

あなたのアプリが`#[derive(Task)]`のスケジュールを使っている場合、スケジューラーには専用の長時間実行プロセスが必要です。同じリポジトリから、2つ目のサービスを追加します：

1. **New** → **GitHub Repo** → 同じリポジトリを選びます。
2. ダッシュボードで見つけやすいように、`scheduler`と名付けます。
3. **Settings** → **Deploy**の下で、**Custom Start Command**を以下に設定します：
   ```bash
   ./app schedule:work
   ```
4. ワーカーがWebサービスと同じ設定を読み込むように、同じ変数（特に`APP_KEY`とデータベースの参照）をコピーします。

`schedule:work`はデーモンループです - 1分に1回目を覚まし、期限の来たタスクをスケジュールに問い合わせ、HTTPサーバーと同じブートストラップを通じてそれらを実行します。契約については、[コンソール](console.md)とスケジューラーのチャプターを参照してください。

スケジューラーインスタンスを正確に1つ実行してください。複数の`schedule:work`プロセスは、キャッシュに支えられたロックを介して調整しますが、デフォルトの想定は単一のワーカーです。

### Suprnovaが異なる設計を選んだ理由

ForgeやVapor上でのLaravelのデプロイは、通常、Webサーバー（php-fpm + nginx）、キューワーカー（`php artisan queue:work`）、そして毎分`schedule:run`を呼び出すcronエントリを配線します。3つのコンポーネント、3つのデプロイサーフェスです。

Suprnovaは、すべてのロールを同じバイナリにコンパイルします。Railwayのサービス仕様は、Webロールには`./app`、スケジューラーには`./app schedule:work`です - 同じイメージ、同じブートストラップ、異なるargvです。別個のphp-fpmコンテナも、別個のワーカーイメージも、ホストのcronもありません。キューに入れるジョブがあるなら、3つ目のサービスとして`./app queue:work`を追加すれば、1つのDockerfileから、3つのRailwayサービスの中に完全なLaravelのトポロジーが手に入ります。

## ヘルスチェックと`railway.json`

デプロイをより細かく制御するには、`railway.json`をリポジトリのルートにコミットしてください。Railwayはそれを自動的に読み込みます。

```json
{
  "$schema": "https://railway.app/railway.schema.json",
  "build": {
    "builder": "DOCKERFILE",
    "dockerfilePath": "Dockerfile"
  },
  "deploy": {
    "startCommand": "./app",
    "healthcheckPath": "/_suprnova/health/live",
    "healthcheckTimeout": 300,
    "restartPolicyType": "ON_FAILURE",
    "restartPolicyMaxRetries": 10
  }
}
```

Suprnovaは、ミドルウェアチェーンの前でショートサーキットする組み込みのヘルスエンドポイントを提供します - authやCSRF、レートリミットを通ることなく、200のJSONステータスを返します。`/_suprnova/`プレフィックスは予約されているため、あなた自身のルートと衝突することは決してありません。

上記の`healthcheckPath`は、何にも触れない`/_suprnova/health/live`を指しています。この組み合わせは意図的なものです。このサービスは`"restartPolicyType": "ON_FAILURE"`に設定されているため、ヘルスチェックが何をプローブするにせよ、それは再起動のトリガーになります。データベースを指す - `/_suprnova/health/ready`や、より古い`/_suprnova/health?db=true`を介して - ということは、データベースが再接続の殺到に最も耐えられない瞬間に、データベースの一時的な不調がすべてのレプリカを再起動させることを意味します。プロセスを再起動させるパスからではなく、別個のレディネスチェックやあなたの監視から、データベースをプローブしてください。[適切な質問に対して適切なプローブを使用する](deployment.md#use-the-right-probe-for-the-right-question)を参照してください。

古い方のパスもどちらも引き続き機能するため、既存のRailwayサービスに変更は不要です。名前付きのパスは、単により明確であるというだけです。

## カスタムドメインとTLS

1. Webサービスの中で、**Settings** → **Networking**を開きます。
2. `*.up.railway.app`のサブドメインには**Generate Domain**を、独自のホスト名をサービスに向けるには**Custom Domain**をクリックします。
3. Railwayの指示どおりにDNSを更新します（サブドメインには`CNAME`、apexドメインにはANAME/ALIAS）。

Railwayは、生成されたドメインとカスタムドメインの両方について、Let's Encrypt証明書をプロビジョニングし、更新します。

## CI/CDでのマイグレーション

デフォルトの`CMD ["./app"]`は、ブート時にマイグレーションを実行します。これは単一インスタンスのデプロイには問題ありません。複数レプリカの構成では、マイグレーションのステップを切り離します：

1. 新しいレプリカが起動する前に、本番データベースに対して`./app migrate`を実行する、ワンショットの**プリデプロイフック**を追加します。
2. レプリカ同士が競合しないように、ランタイムの起動コマンドを`./app serve --no-migrate`に変更します。

マイグレーションランナーはべき等です - ステップを分割しなくても、すべてのブートでマイグレーションを実行することは、レプリカ間で安全です。この分割が存在するのは、ロールアウトを開いたままにすることなく、不正なマイグレーションでデプロイを早期に失敗させられるようにするためです。

## ログ、メトリクス、ロールバック

Webサービスのタブは、以下を公開します：

- **Deployments** - 時系列順のすべてのビルドです。以前の成功したデプロイの三点リーダーメニューが、ワンクリックでのロールバック手段です。
- **Logs** - コンテナからの`tracing`出力で、構造化ログのフィールド（`request_id`、`route`、`status`）はログビューアーのフィルターですぐに使えます。
- **Metrics** - CPU、メモリ、ネットワークIOです。インスタンスをスケールアップ・ダウンする際のサイズ決定に役立ちます。

## トラブルシューティング

**`cargo build --release`でビルドが失敗する。** `docker build -t myapp .`でローカルに再現してください。最も多い原因は、あなたのマシンではコンパイルできるのにリポジトリに含まれていないワークスペースメンバーです - Dockerfileは最初に`Cargo.toml`と`Cargo.lock`をコピーするため、不足しているクレートははっきりと失敗します。

**アプリが「connection refused」を返す。** サービスで`SERVER_HOST=0.0.0.0`が設定されているか確認してください。デフォルトは`127.0.0.1`で、Railwayはそこにルーティングできません。

**アプリがブートした後、キーエラーで終了する。** `APP_KEY`が未設定か、不正な形式です。フレームワークは、それなしでは本番環境でのブートを拒否します。`suprnova key:generate --show`の出力を、サービスの変数に貼り直してください。

**マイグレーションがブート時に失敗する。** 根本的なSQLエラーについて、ログを確認してください。よくある原因は、未設定の`DATABASE_URL`（`${{ Postgres.DATABASE_URL }}`の参照が解決されたか確認してください）か、古いベースラインに対して実行されたマイグレーションです（`./app migrate:status`は、どこに何が適用されているかを報告します）。

**スケジューラーが一向に発火しない。** 起動コマンドが正確に`./app schedule:work`であることを確認してください（`schedule:run`ではありません - こちらは期限の来たタスクを一度実行して終了します）。ワンショットのデプロイからの`schedule:list`が、あなたのタスクが登録されていることを確認します。

## 次のステップ

- [デプロイメント 概要](deployment.md) - あなたのRailwayサービスが実行する、統合バイナリのモデル
- [Docker CLI](cli-docker.md) - `docker:init`と`docker:compose`が実際に何を生成するか
- [設定](configuration.md) - `.env`の読み込み、型付き設定、必須のキー
- [コンソール](console.md) - `schedule:work`、`queue:work`、`workflow:work`、そして統合CLIの残り
- [Digital Ocean にデプロイ](deployment-digital-ocean.md) - 別のPaaS上での、同じレシピ
