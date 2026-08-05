# 可観測性

フレームワークには、オペレーターに見える3つの層の信号が出荷されています：構造化ログ（常時オン）、リクエストごとのIDによる突き合わせ（常時オンで、spawnされたタスクへも伝播します）、そしてあらゆる `tracing` のスパンをエクスポートされるOTelのスパンへ変換するオプトインのOpenTelemetryブリッジです。ローカルログのために書くのと同じ `#[tracing::instrument]` が、OTelフィーチャーが有効なときには分散トレースのスパンになります - 2つ目の計装APIは不要です。

```rust
use suprnova::telemetry::{init_telemetry, OtelConfig};
use suprnova::logging::LogConfig;

#[suprnova::main]
async fn main() {
    let guard = init_telemetry(LogConfig::from_env(), OtelConfig::from_env());

    // ... アプリを実行する ...

    // 終了する前に、バッファリングされたテレメトリを書き出します。OTelのバッチ
    // プロセッサーはスパン/メトリクス/ログをメモリ内に保持しているため、
    // `shutdown` を呼ばずにガードをドロップすると、まだエクスポートされていない
    // ものが失われます。
    guard.shutdown().await;
}
```

スキャフォルドされたアプリの `Server` は、すでにあなたの代わりに `init_telemetry` を呼び出し、シャットダウン信号でガードをフラッシュします - 自分の手でこれを配線するのは、Suprnovaを自分自身のランタイムに組み込むときだけです。

## 3つの層

| 層 | 常時オン | 得られるもの |
|---|---|---|
| 構造化ロギング（`tracing`） | はい | `pretty`（開発環境）または `json`（本番環境）形式の、環境を意識したStdoutログ |
| リクエストIDの突き合わせ | はい | `tokio::task_local!` にスコープされたリクエストごとのIDで、`X-Request-Id` にエコーされ、`spawn_with_request_id` のタスクへ伝播します |
| OpenTelemetryのエクスポート | `otel` フィーチャー + コレクターのエンドポイント | トレース、メトリクス、ログのOTLP HTTP/protoエクスポート。W3Cの `traceparent` を双方向で伝播します |

OTel層は**コンパイル時のオプトイン**であるため、デフォルトのビルドはOpenTelemetryへの依存を一切持ち込まず、[`Metrics`](#メトリクス)ファサードは無害なno-opにコンパイルされます。このフィーチャーがオフの場合、「トレース」と「メトリクスのエクスポート」は静かにno-opになります - あなたのログはそれでも動作します。

### Suprnovaが異なる設計を選んだ理由

Laravelの可観測性の物語は、フレームワーク内のイベント（`QueryExecuted`、`MessageSent`、`JobProcessed`）と、FPM層に差し込まれるPHP拡張（OpenTelemetry、Sentry、New Relic）へ委ねられるランタイムの関心事の間で分かれています。イベントの表面は豊富ですが、ランタイムの表面は「あなたのAPMベンダーが必要とする拡張をインストールする」というものです。

Suprnovaは単一の非同期プロセスであるため、その両方を自分で所有します。イベントの表面は同等です（同じ `QueryExecuted` / `NotificationSent` / `ErrorOccurred` の形です）。そしてランタイムの表面は、フレームワーク内部の `tracing` → OpenTelemetryブリッジです。あなたは拡張をインストールするのではなく、フィーチャーフラグを切り替えるだけで、すでに発しているのと同じスパンがOTelへエクスポートされるようになります。

## 構造化ロギング

`LogConfig::from_env()` は、2つの環境変数を読み取ります。

| 変数 | デフォルト | 注記 |
|---|---|---|
| `LOG_LEVEL` | `"info"` | `tracing-subscriber` のenv-filter文法です（例: `"debug,sqlx=warn,hyper=warn"`） |
| `LOG_FORMAT` | 環境を意識する | 本番環境では `"json"`、それ以外のすべてでは `"pretty"`。明示的な値は常に優先されます |

フォーマットのデフォルトは、`Environment::detect()` を介して `APP_ENV` から検出されます。本番環境のデプロイでは、デフォルトでログアグリゲータ向けの1行1JSONオブジェクトの出力になり、ローカル/開発環境での実行では、人間に読みやすい複数行の出力になります。本番環境で生のstdoutが欲しい場合は、明示的な `LOG_FORMAT=pretty` が本番環境のデフォルトを上書きします。

```bash
# ローカル開発 - 明示的な上書きが優先されます
LOG_LEVEL=debug,sqlx=warn,hyper=warn LOG_FORMAT=pretty cargo run

# 本番環境 - APP_ENV=production がフォーマットのデフォルトをjsonへ切り替えます
APP_ENV=production LOG_LEVEL=info cargo run --release
```

不正な形の `LOG_LEVEL` ディレクティブは起動をクラッシュさせません - `"info"` へフォールバックし、stderrへ1行の警告を出力するため、設定ミスがオペレーターに見える形になります。

### あらゆる行に含まれるスパンコンテキスト

ルーティングされたあらゆるHTTPリクエストは、フレームワークの最も外側のミドルウェアが作る `request` スパンの内側で実行されます。このスパンは3つのフィールド - `request_id`、`method`、`path` - を運び、JSONフォーマッタは、リクエストの内側で発せられるあらゆるイベントの上で、これらを `span` の下にネストします。あなたのアプリケーションコードは、あらゆる行でこのIDを読み取ったり記録したりする必要はありません - スパンが暗黙的にそれを運びます。

```rust
use tracing::info;

pub async fn show(req: suprnova::Request) -> suprnova::Response {
    info!(user_id = 42, "loaded dashboard");
    // JSONの行は、呼び出し箇所が何かを配線する必要なく、
    // span.request_id / span.method / span.path を運びます。
    Ok(suprnova::json_response!({ "ok": true }))
}
```

## リクエストIDによる突き合わせ

あらゆるリクエストは、`tokio::task_local!` にスコープされた、36文字の小文字UUID v4のIDを受け取ります。ミドルウェアは、ヘッダーの値が厳格な安全性チェック（ASCII英数字と `-_.:`、最大128バイト）を通過する場合、受信した `X-Request-Id` を再利用します。その文字集合の外にあるものはすべて拒否され、新しいUUIDに置き換えられます - これにより、攻撃者がログ出力に制御文字を注入したり、下流のパイプラインを肥大化させたりすることはできません。

同じIDは、成功・エラー・パニックリカバリのいずれであっても、**あらゆる**レスポンスで `X-Request-Id` ヘッダーとしてエコーされます。そのため、フロントエンドやアップストリームのサービスはそれをバグレポートに含めることができ、オペレーターは構造化ログの中でそれをgrepできます。

### IDを読み取る

```rust
use suprnova::{current_request_id, spawn_with_request_id};

pub async fn checkout(req: suprnova::Request) -> suprnova::Response {
    // リクエストの内側では、IDは常に存在します。
    let id = current_request_id().expect("inside a request");
    tracing::info!(request_id = %id, "checkout starting");

    // ハンドラからspawnされたバックグラウンドの作業です。`tokio::spawn` は
    // 空のタスクローカルを持つタスクを開始するため、助けがなければ
    // spawnされたfutureはリクエストIDを失ってしまいます。
    // `spawn_with_request_id` は呼び出し元のIDを捕捉してspawnされた
    // futureに対して再スコープし、現在の `tracing` スパンも取り付けるため、
    // タスクのイベントは、リクエスト内のイベントと同じ形で
    // `request_id` を引き継ぎます。
    spawn_with_request_id(async move {
        // このログ行は、発信元のリクエストのIDを運びます。
        tracing::info!("post-checkout fanout running");
    });

    Ok(suprnova::ok!())
}
```

`current_request_id()` はリクエストの外側では `None` を返します - バックグラウンドのジョブ、スケジュールされたタスク、そしてミドルウェアなしのテストはIDを見ることがなく、このヘルパーがIDを発明することもありません。リクエストスコープの外側での `spawn_with_request_id` は、まさに `tokio::spawn` そのものです - 何も特別なことは起きません。

### IDが他に手に入る場所

| 表面 | 方法 |
|---|---|
| `tracing` のイベント | リクエストの内側のあらゆる行における `span.request_id` |
| レスポンスヘッダー | 成功・エラー・パニックリカバリ済みのレスポンスにおける `X-Request-Id` |
| `Context` バッグ | `Context::get("_request_id")` - `Context` を参照するオブザーバー、リスナー、ジョブから読み取れます |
| spawnされたタスク | `spawn_with_request_id` の後の `current_request_id()` |

## 可観測性のための組み込みイベント

フレームワークは、オペレーターが通常計装したいと思う地点で、型付きのイベントをディスパッチします。それぞれは `suprnova::Event` であり、`EventFacade::listen::<E, _>(...)` を介して `listen` し、Sentry、Datadog、Slack、あるいはあなたのメトリクスパイプラインへ送ることができます。これらはすべて `dispatch_best_effort` を通じて実行されるため、失敗したリスナーが、それを引き起こしたリクエストを壊すことはありません。

| イベント | いつ発火するか | 運ぶもの |
|---|---|---|
| `ErrorOccurred` | あらゆる `FrameworkError` → 5xxへの変換（パニックリカバリを含む） | エラーコンテキスト + リクエストID |
| `QueryExecuted` | 計装済みのエグゼキューターヘルパーを経由するすべてのクエリ | sql、バインドパラメータ、所要時間、コネクション、読み取り/書き込みの分類、結果 |
| `ConnectionEstablished` | `DbConnection::connect` が成功したとき | コネクション名 |
| `TransactionBeginning` / `TransactionCommitted` / `TransactionRolledBack` | クロージャ形式の `DB::transaction` + 手動のハンドル | コネクション名 |
| `NotificationSending` / `NotificationSent` / `NotificationFailed` | `Notification::send` のチャネルごとの前/後/エラー | 通知 + チャネル + 受信者 |

`ErrorOccurred` は5xxの例外を送るためのフックであり、`QueryExecuted` はスロークエリのアラートのためのフックであり、通知の3つ組は配信ダッシュボードのためのフックです。リスナーAPIについては[イベント](events.md)を、各イベントがリクエストパスのどこで発火するかについては[リクエスト ライフサイクル](lifecycle.md)を参照してください。

### DBクエリを直接観測する

`DB::listen` は、`QueryExecuted` に特化した、もう1つの同期的なフックです。これはエグゼキューターの内側でインラインに発火するため、遅いリスナーはクエリそのものを遅くします - 軽量に保ってください。ディスパッチャー経由の経路（`EventFacade::listen::<QueryExecuted, _>`）は全件実行のベストエフォートで、エラーを許容します - 失敗する可能性のあるものには、これを優先してください。

```rust
use suprnova::DB;

// bootstrap.rs の中で:
DB::listen(|q| {
    if q.time > std::time::Duration::from_millis(100) {
        tracing::warn!(
            sql = %q.sql,
            ms = q.time.as_millis(),
            "slow query"
        );
    }
})?;
```

自分自身がデータベースクエリを発するリスナーは、その入れ子になった呼び出しに対して `QueryExecuted` を再発火**しません** - タスクローカルな再入防止ガードが、「DBリスナーへのログ記録 → イベントを発する → DBへのログ記録 → ...」というループを防ぎます。

### テスト/デバッグ用にクエリログをキャプチャする

テストのアサーションのため、あるいは「このブロックの間に何が実行されたか」を一度だけ調べるデバッグのために使えます。

```rust
use suprnova::DB;

DB::enable_query_log()?;
// ... 調べたいコードを実行する ...
let queries = DB::get_query_log()?;
for q in &queries {
    println!("{:>4}ms  {}", q.time.as_millis(), q.to_raw_sql());
}
DB::disable_query_log()?;
DB::flush_query_log()?;
```

このバッファは**無制限**です - キャプチャされたクエリのひとつひとつが、それを大きくしていきます。テストやその場限りの調査に使い、本番環境でこれを有効にしたままにする場合は定期的にフラッシュしてください。

## 分散トレーシング（OTel）

オプトインするには、`otel` フィーチャーを追加してください。

```toml
[dependencies]
suprnova = { git = "...", features = ["otel"] }
```

標準のOTel環境変数を介して設定します。

```bash
# 最低限必要なもの: コレクターがどこにあるか。
OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4318
OTEL_SERVICE_NAME=my-app          # デフォルトは "suprnova"
OTEL_SERVICE_VERSION=1.4.2        # デフォルトはあなたのクレートのバージョン
```

テレメトリが**有効化**されるのは、`OTEL_EXPORTER_OTLP_ENDPOINT` が設定されており、**かつ**キルスイッチである `OTEL_SDK_DISABLED` がオンでない場合だけです。エンドポイントがなければ、ロギング層だけが単独で動作し、返されるガードはプロバイダーを何も保持しないため、`shutdown()` を呼ばずにこれをドロップしても無音です（あらゆるテストプロセスで「バッファリングされたテレメトリが失われる可能性がある」という余計な警告は出ません）。

### トレースコンテキストの自動結合

**インバウンド。** リクエストがW3Cの[`traceparent`](https://www.w3.org/TR/trace-context/)ヘッダーを伴って到着した場合 - つまり、それが別のトレース対象サービスによって作られたものである場合 - ミドルウェアはそのコンテキストを取り出し、リクエストスパンを呼び出し元のスパンの子として再親付けします。あなたのサーバースパンは、新しいルートとしてではなく、*同じ*分散トレースの中の子として現れます。`traceparent` を伴わないリクエスト（ブラウザからの直接のヒット）は、まっさらなルートスパンのままです。

**アウトバウンド。** フレームワークのHTTPクライアント（[`Http`](http-client.md)）は、あらゆるアウトバウンドの呼び出しに対して、アクティブなトレースコンテキストを `traceparent` として注入するため、下流のサービスは同じトレースを続けます。

まとめると、`アップストリームのサービス → あなたのハンドラ → ダウンストリームのサービス` は1つの連結されたトレースであり、あなたのハンドラの中で手作業によるスパンの配線は不要です。

**エラーステータス。** ハンドラが5xxを返すと、リクエストスパンはエラー済みとしてマークされ、OTelバックエンドは `Status::Error` を表示します。（ハンドラの*パニック*は捕捉され、エラーレベルのログと `ErrorOccurred` イベントを伴う500へと変換されますが、その経路ではOTelのスパンステータスは設定されません - パニックは、そのマーカーが実行される前に、スパンのfutureを巻き戻してしまいます。）

### 独自のスパンを追加する

このブリッジがあらゆる `tracing` のスパンをOTelのスパンへと変換するため、あなたは素の `tracing` で計装します - あなたのコードにOTel固有のAPIは不要です。

```rust
use suprnova::DatabaseConnection;

#[tracing::instrument(skip(db))]
async fn load_dashboard(db: &DatabaseConnection, user_id: i64) -> anyhow::Result<()> {
    // このスパンは自動的にリクエストスパンの下にネストされ、
    // `otel` フィーチャーが有効なときはあなたのコレクターへエクスポートされます。
    Ok(())
}
```

### Suprnovaが読み取る環境変数

| 変数 | 効果 |
|---|---|
| `OTEL_EXPORTER_OTLP_ENDPOINT` | コレクターのベースURL。未設定 → テレメトリは無効。 |
| `OTEL_SERVICE_NAME` | `service.name` リソース属性（デフォルトは `"suprnova"`）。 |
| `OTEL_SERVICE_VERSION` | `service.version` リソース属性（デフォルト: クレートのバージョン）。 |
| `OTEL_SDK_DISABLED` | キルスイッチ。大文字小文字を区別しない `true` または `1` は、エンドポイントが設定されていてもエクスポートを無効にします。 |

残りの標準的なOTLPのノブは、SDK自身によって読み取られるため、通常の方法で設定してください。

| 変数 | 読み取る側 |
|---|---|
| `OTEL_EXPORTER_OTLP_HEADERS` | エクスポーター（コレクターの認証、例: `Authorization=Bearer ...`） |
| `OTEL_EXPORTER_OTLP_PROTOCOL` | エクスポーター（`http/protobuf` など） |
| `OTEL_EXPORTER_OTLP_TIMEOUT` | エクスポーター |
| `OTEL_EXPORTER_OTLP_COMPRESSION` | エクスポーター |

信号ごとのエンドポイント上書き（`OTEL_EXPORTER_OTLP_TRACES_ENDPOINT`、`_METRICS_ENDPOINT`、`_LOGS_ENDPOINT`）は、現在のところベースのエンドポイントによって覆い隠されます - 3つの信号すべてが `OTEL_EXPORTER_OTLP_ENDPOINT` へ向かいます。信号を別々のコレクターへ振り分ける必要がある場合は、それらへルーティングするローカルのコレクターを立ててください。

## メトリクス

`Metrics` は、カウンター、ヒストグラム、ゲージのためのファサードです。ハンドルは複製が安価で、構築のたびにグローバルなメーターを解決します。

```rust
use suprnova::telemetry::Metrics;

// カウンター - 単調増加。
let signups = Metrics::counter("user.signups");
signups.inc();                                  // +1
signups.inc_by(3);                              // +3
signups.inc_with(&[("plan", "pro")]);           // ラベル付きで +1

// ヒストグラム - 分布（レイテンシ、サイズ）。
let latency = Metrics::histogram("request.latency_ms");
latency.record(42.0);
latency.record_with(42.0, &[("route", "/checkout")]);

// ゲージ - ある時点の値。
let queue_depth = Metrics::gauge("jobs.pending");
queue_depth.set(17.0);
queue_depth.set_with(17.0, &[("queue", "emails")]);
```

`otel` フィーチャーがなければ、上のあらゆる呼び出しはゼロアロケーションのno-opです - ホットパスの中に計装を残しておいても、デフォルトのビルドでは何のコストも払いません。

メトリクスのハンドルは、その裏側の計装が最初に解決されたときにアクティブだったメータープロバイダーへ結び付きます。ハンドルは `init_telemetry` の実行**後**に作成してください（あるいは初回使用時に遅延して作成してください） - 初期化の前に構築されたハンドルは、no-opプロバイダーに対して解決され、そのまま不活性になります。慣用的なパターンは、起動のかなり後、初回の発行時に解決される `once_cell` / `LazyLock` ハンドルです。

属性の値は文字列型です（`&[(&'static str, &str)]`）。数値やブール値の属性は計画中の拡張であり、今のところは呼び出し箇所で文字列にフォーマットしてください。

命名: 安定していて、ASCIIで、ドット区切りです（例: `"http.requests.total"`、`"http.request.duration"`）。標準のOTelセマンティックコンベンションは、`opentelemetry-semantic-conventions::metric::*` にあります。

## シャットダウンの契約

`init_telemetry` は、SDKのプロバイダーハンドルを所有する `TelemetryGuard` を返します。OTelのバッチプロセッサーは、スパン / メトリクス / ログをメモリ内でバッファリングし、非同期にフラッシュするため、プロセスが終了する前に `guard.shutdown().await` を呼ばなければ、まだバッファリングされているものを失います。

- `shutdown()` を呼ぶとフラッシュされ、1度だけ呼ぶのは安全です（`self` を取ります）。
- `shutdown()` を呼ば**ずに**ガードをドロップすると警告がログに記録されます - ただし、それはガードが実際にプロバイダーを保持している場合だけです。テレメトリが無効な実行（エンドポイントなし、`OTEL_SDK_DISABLED`、あるいは `otel` を使わないビルド）は、プロバイダーを持たないガードを返し、そのドロップは無音です - そのため、コレクターのない開発環境やテストの実行がスパムされることはありません。

## まとめ

| タスク | API |
|---|---|
| OTelを有効にする | `features = ["otel"]` + `OTEL_EXPORTER_OTLP_ENDPOINT` |
| 初期化する | `init_telemetry(LogConfig::from_env(), OtelConfig::from_env())` |
| 終了時にフラッシュする | `guard.shutdown().await` |
| 実行時に無効化する | `OTEL_SDK_DISABLED=true` |
| 独自のスパン | `#[tracing::instrument]`（自動的にOTelへブリッジされます） |
| カウンター / ヒストグラム / ゲージ | `Metrics::counter/histogram/gauge(name)` |
| 分散トレースの結合 | 自動 - インバウンドの `traceparent` を取り出し、アウトバウンドへ注入します |
| 現在のリクエストIDを読む | `current_request_id()` |
| spawnへIDを伝播する | `spawn_with_request_id(future)` |
| 同期的なクエリオブザーバー | `DB::listen(|q| { ... })` |
| ベストエフォートのクエリオブザーバー | `EventFacade::listen::<QueryExecuted, _>(...)` |
| テスト用にクエリをキャプチャする | `DB::enable_query_log()` → `DB::get_query_log()` |

## 次のステップ

- [イベント](events.md) - リスナーAPI、ディスパッチのモード、テスト用の `EventFacade::fake()`
- [リクエスト ライフサイクル](lifecycle.md) - 各イベントがリクエストパスのどこで発火し、リクエストスパンがどこで構築されるか
- [エラーハンドリング](errors.md) - `ErrorOccurred`、`HttpError`、サニタイズされた5xxのボディ
- [データベース](database.md) - `QueryExecuted`、`DB::transaction`、イベントを発するエグゼキューターヘルパー
- [HTTP クライアント](http-client.md) - 分散トレースのループを閉じる、アウトバウンドの `traceparent` 注入
