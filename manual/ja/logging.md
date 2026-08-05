# ロギング

Suprnovaは、[`tracing`](https://docs.rs/tracing) を通じてログを記録します - あらゆるログ行は、フォーマット済みの文字列ではなく、フィールドを持つ構造化されたイベントです。起動時にインストールされるサブスクライバーは、環境から `LOG_LEVEL` と `LOG_FORMAT` を読み取り、開発環境では見やすい複数行の出力を、本番環境では1行につき1つのJSONオブジェクトを出力し、リクエストごとのIDを、ハンドラが発するあらゆるイベントへと伝播させます。

この章が扱うのは、ログの表面そのものです - サブスクライバー、フォーマット、レベル、そして本番環境のログを検索可能にするリクエストIDによる突き合わせです。OpenTelemetryのブリッジとクエリロギングについては[可観測性](observability.md)を、発信元がIDと並んで読める、リクエストの `Context` バッグについては[コンテキスト](context.md)を参照してください。

## 何がどこに記録されるか

デフォルトでは、2つの出力先があります。

| 出力先 | フォーマット | いつ |
|---|---|---|
| `stdout` | `LogFormat::Pretty` - 複数行、色付き、人間に読みやすい | 開発環境（`APP_ENV` が `local`、`dev`、`testing`、…） |
| `stdout` | `LogFormat::Json` - 1行につき1つのJSONオブジェクト | 本番環境（`APP_ENV=production` / `prod`） |

開発/本番のデフォルトは、`Environment::detect()` を介して `APP_ENV` から計算されます。明示的にどちらかを強制するには、`LOG_FORMAT=pretty` または `LOG_FORMAT=json` で上書きしてください。

```env
# .env（開発）
LOG_LEVEL=info,sqlx=warn
LOG_FORMAT=pretty   # 任意。これが開発のデフォルトです

# .env.production
LOG_LEVEL=info,sqlx=warn,suprnova::queue=debug
LOG_FORMAT=json     # 任意。これが本番のデフォルトです
```

フレームワークは `stdout` にのみ書き込みます。本番環境では、コンテナランタイム、systemdジャーナル、あるいはログアグリゲータをそこに向けてください（`docker logs`、`kubectl logs`、`journalctl -u my-app`、Loki/Vectorエージェントなど）。ローテーションするファイルアペンダーはありません - ログの永続化は、プラットフォームに任せてください。

## イベントを発火する

ハンドラ、ジョブ、ミドルウェア、どこであれ、`tracing` のマクロを使ってください。

```rust
use suprnova::{json_response, session, Request, Response};
use tracing::{debug, info, warn, error, instrument};

pub async fn checkout(_req: Request) -> Response {
    let user_id: i64 = session()
        .and_then(|s| s.get::<i64>("user_id"))
        .unwrap_or(0);

    info!(user_id, "checkout starting");

    let order = place_order(user_id).await.map_err(|e| {
        error!(user_id, error = %e, "checkout failed");
        e
    })?;

    info!(user_id, order_id = order.id, total = order.total_cents, "checkout succeeded");

    json_response!(order)
}
```

各フィールドは、JSON出力ではトップレベルのキーになり、pretty出力では色付きの `field=value` というペアになります。文字列への埋め込みよりもフィールドを優先してください - JSONログの中で検索可能になりますし、フォーマッタが型を意識したレンダリングを行ってくれます。

関数をスパンでラップし、その中のあらゆるイベントに共有フィールドを刻印するには、`#[instrument]` を使ってください。

```rust
#[instrument(skip(db), fields(user_id = %user_id))]
pub async fn load_dashboard(
    db: &suprnova::DatabaseConnection,
    user_id: i64,
) -> Result<Dashboard, FrameworkError> {
    info!("loading"); // スパンから自動的に user_id を運びます
    // … クエリ …
}
```

同じ `#[instrument]` は、`otel` フィーチャーが有効になっているとき、OpenTelemetryのスパンになります - [可観測性](observability.md#opentelemetry)を参照してください。

## ログレベル

`LOG_LEVEL` は、単一のレベルではなく、[`tracing-subscriber` のenv-filterディレクティブ](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html)です。文法は、カンマ区切りの `target=level` のペアであり、裸の値はデフォルトを設定します。

```env
LOG_LEVEL=info                                  # info以上のすべて
LOG_LEVEL=debug                                 # debug以上のすべて
LOG_LEVEL=info,sqlx=warn                        # infoがデフォルト、sqlxは静かめ
LOG_LEVEL=warn,suprnova::queue=debug,my_app=info  # warnがデフォルト、2つのターゲットは饒舌
```

ターゲットは、通常、発信元のクレートまたはモジュールパスです（`suprnova::queue`、`hyper::server`、`my_app::services::checkout`）。ターゲットを見つけるには、JSONのログ行を読んでください - あらゆるイベントの `target` フィールドが、そのフィルタキーです。

詳細さが増していく順に並べると、`error` < `warn` < `info`（デフォルト）< `debug` < `trace` です。クライアントに返るエラーレスポンスは、レベルに関わらず常に `{"message": "Internal Server Error"}` にサニタイズされます - 詳細は構造化ログにのみ送られます。

### 不正なディレクティブは起動を止めない

不正な形の `LOG_LEVEL`（例えば `LOG_LEVEL=app=notalevel`）は `"info"` へフォールバックし、`stderr` へ1行の警告を書き出します。

```text
suprnova: invalid LOG_LEVEL directive "app=notalevel" (...); falling back to "info". Fix LOG_LEVEL to silence this.
```

これが `tracing::warn!` ではなく `stderr` である理由は、その時点ではまだサブスクライバーがインストールされていないからです - `warn!` は黙って捨てられてしまいます。ディレクティブを修正すれば、警告は消えます。

## PrettyとJSONの出力

同じ `info!(user_id = 42, "saved")` が、フォーマットごとに異なる形でレンダリングされます。

**Pretty（開発）:**

```text
  2026-05-30T22:14:08.221341Z  INFO request{request_id=78a9...} my_app::handlers::checkout: saved
    at src/handlers/checkout.rs:48
    in checkout
    in request with request_id: 78a9..., method: POST, path: /checkout
```

**JSON（本番）:**

```json
{
  "timestamp": "2026-05-30T22:14:08.221341Z",
  "level": "INFO",
  "fields": { "message": "saved", "user_id": 42 },
  "target": "my_app::handlers::checkout",
  "span": { "name": "checkout" },
  "spans": [
    { "name": "request", "request_id": "78a9...", "method": "POST", "path": "/checkout" }
  ]
}
```

このJSONの形は、本番環境のアグリゲータ（Datadog、Loki、Honeycomb、CloudWatch、…）が、そのまま解析できるものです。`span.request_id` が突き合わせ用のキーです - 詳しくは下記を参照してください。

## リクエストごとのIDによる突き合わせ

あらゆるHTTPリクエストは、あらゆるチェーンの最も外側のミドルウェアである `RequestIdMiddleware` から `RequestId` を受け取ります。このIDは次のような性質を持ちます。

- 安全な、受信した `X-Request-Id` ヘッダー（英数字と `- _ . :`、最大128バイト）から**再利用**されるか、それが不在／安全でない場合はUUID v4として**新たに発行**されます。
- レスポンスに `X-Request-Id` として**エコーバック**されます（2xxと5xx、どちらのバリアントでも）。
- `request` という `tracing` スパンへ**スコープ**されるため、あらゆるミドルウェア、ハンドラ、下流のライブラリからのイベントは、その `spans` 配列に `request_id` を自動的に運びます。
- リクエストの `Context` バッグへ `_request_id` として**シード**されるため、裸の文字列が欲しい発信元（ジョブ、ブロードキャストのペイロード、エラーレポート）は、名前でそれを読み取れます。

コードの中でこれを読むには、`current_request_id()` を使ってください。

```rust
use suprnova::current_request_id;
use tracing::info;

if let Some(id) = current_request_id() {
    info!(request_id = %id, "checkpoint reached");
}
```

`current_request_id()` が `Option<RequestId>` を返すのは、バックグラウンドの作業（ジョブ、スケジュールされたタスク、ミドルウェアをインストールしなかったテスト）が、あらゆるリクエストスコープの外側で実行されるからです。

### バックグラウンドタスク: IDを添えてspawnする

`tokio::spawn` は、空のタスクローカルを持つ新しいタスクを開始します - 副作用のある作業をspawnするハンドラは `current_request_id()` を失い、そのログイベントは孤立してしまいます。代わりに `spawn_with_request_id` を使ってください。

```rust
use suprnova::spawn_with_request_id;
use tracing::info;

pub async fn checkout(req: suprnova::Request) -> suprnova::Response {
    let order = place_order().await?;

    spawn_with_request_id(async move {
        // このタスクは、それでも current_request_id() を観測できます。
        // このタスクのログイベントは、ハンドラと同じ request_id を運びます。
        info!(order_id = order.id, "post-checkout fanout running");
        send_receipt(order.id).await;
        update_analytics(order.id).await;
    });

    suprnova::Response::ok().json(&order)
}
```

このヘルパーは、`RequestId` のタスクローカルと現在の `tracing::Span` の両方を伝播させるため、spawnされたfutureのイベントは、ログの中で同じ `request` スパンの下にネストされます。アクティブなリクエストスコープの外側では、素の `tokio::spawn` へとフォールスルーします - 無条件に使っても安全です。

タスクに付いていくのは、リクエストIDとtracingのスパンだけです - リクエストの `Context` バッグは、意図的に付いていきません。バックグラウンドの作業は、発信元のHTTPリクエストに応答しているわけではないからです。

## サブスクライバー

フレームワークは、起動時に `Server::run()` からグローバルな `tracing` サブスクライバーをインストールします。これを自分で呼び出すことは、ほとんどありません - ドキュメント化されているのは、テスト、組み込み先、そして通常とは異なるエントリポイントが、ときにこれを必要とするからです。

```rust
use suprnova::{LogConfig, init_subscriber};

// 環境から LOG_LEVEL / LOG_FORMAT を読み取ります:
init_subscriber(LogConfig::from_env());

// あるいはプログラムから:
init_subscriber(LogConfig {
    level: "info,sqlx=warn".to_string(),
    format: suprnova::LogFormat::Json,
});
```

`init_subscriber` は**べき等**です。2回目の呼び出しは、既存のサブスクライバーをそのままにし、新しい `LogConfig` が適用されなかったことをオペレーターが把握できるよう、`tracing::warn!` を発行します。これによって、それぞれが `init_subscriber` を呼び出すテスト同士が競合しなくなります - 最初の呼び出しが勝ち、残りは何もしません。

OTelを意識したバリアント（同じ `LogConfig` に加えて、分散トレーシングのエクスポートを行うもの）には、[`init_telemetry`](observability.md#opentelemetry) を使ってください。

### デーモン

`queue:work`、`schedule:work`、`schedule:run`、`workflow:work` は、あなたのアプリのバイナリのサブコマンドであり、`Server::run()` を経由して起動するわけではないため、起動の途中で自分自身のサブスクライバーをインストールします。これらは、サーバーと同じ `LOG_LEVEL` と `LOG_FORMAT` を読み取り、あなたが何かを呼び出す必要はありません。

```bash
LOG_LEVEL=info,suprnova::queue=debug cargo run --bin my-app -- queue:work

# …あるいは、コンテナの中で、ビルド済みのバイナリに対して:
LOG_LEVEL=info my-app queue:work
```

0.9.1より前は、その経路は何もインストールしていませんでした。デーモンが発するあらゆる `tracing::` の行はどこにも届かず、`LOG_LEVEL` はデーモンに対して何の効果もありませんでした。コンテナの中では、起動時のバナーだけが唯一の出力として残ることになります - ジョブをデッドレターにしているワーカーも、リーダー選出に敗れてティックをスキップしているスケジューラーも、解放できずにいるロックも、すべてアイドル状態のプロセスと見分けがつきませんでした。0.9.1より古いピン留めされたビルドを動かしていて、なぜワーカーが何も言わないのか疑問に思っているなら、それが理由です - そして直し方は、設定の変更ではなくアップグレードです。

ワーカーが言うべきことのほとんどは、`warn!` と `error!` で言われます - 試行回数を使い果たしたジョブ、永続化できなかったデッドレター、解放できなかったロックです - そのため、デフォルトの `info` レベルで、問題を見るには十分です。より静かな判断まで必要なときは、`debug` まで下げてください。

## テスト

テストは、サブスクライバーをインストールする必要はありません - `#[suprnova_test]` アトリビュートと `TestContainer::fake` が、ハンドラのイベントが流れるのに十分な仕組みをセットアップしてくれます。ログの出力についてアサーションしたい場合は、`tracing-subscriber` の [`tracing_subscriber::fmt::TestWriter`](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/fmt/struct.TestWriter.html) やカスタムレイヤーを介してキャプチャしてください。フレームワークが意図的に「このテストのあらゆるログをキャプチャする」フェイクを出荷していないのは、標準の `tracing-subscriber` のテストパターンがきれいに機能するからです。

## Suprnovaが異なる設計を選んだ理由

Laravelは[Monolog](https://github.com/Seldaek/monolog)を使います - オプションのコンテキスト配列を伴うメッセージ文字列、ログチャネル、そしてチャネルごとのハンドラ（ファイル、syslog、Slack、…）です。PHPのリクエストごとにプロセスを立てるモデルでは、単一のグローバルな静的ロガーは安全です - 各リクエストが、自分専用のプロセスと自分専用のコンテキストを持つからです。

Rustのプロセスモデルはその逆です - 1つのプロセスが、多数のスレッド上で多数の並行リクエストを処理します。グローバルな文字列フォーマッタでは、コンテキストにおいて競合が起き、あらゆる呼び出し箇所を通じて `request_id` を明示的に配線する必要が生じてしまいます。`tracing` は、構造化されたフィールドとタスクローカルなスパンによって、その両方を解決します - 配線は不要で、フィールドは型を保ったままであり、チェーンが発するあらゆるイベントに対してリクエストのスパンがスコープ内にあるため、突き合わせは自動的に行われます。

`stdout` のみへの出力もまた、意図的なものです。コンテナ化されたデプロイメント（Suprnovaが出荷する唯一の方法です）では、ログの永続化を担うのはアプリではなくランタイムです - ファイルのローテーション、保持、そして転送は、すべてプラットフォームに属します。

## 次のステップ

- [可観測性](observability.md) - OpenTelemetry、クエリログ、オペレーター向けの表面の全体
- [コンテキスト](context.md) - `_request_id` やその他のコンテキストのフィールドが存在する、リクエストごとのバッグ
- [エラーハンドリング](errors.md) - フレームワークのパニック境界と5xxの経路が、自身の構造化されたイベントを発する仕組み
- [環境変数](env-vars.md) - `LOG_LEVEL`、`LOG_FORMAT` のリファレンス
