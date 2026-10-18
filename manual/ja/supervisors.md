# スーパーバイザー

スーパーバイザーは、フレームワークが起動時に開始し、終了すると自動的に再起動する、長寿命なTokioタスクです。スーパーバイザーは「常時稼働」の作業のためのものです - バックグラウンドのハートビート、メトリクスコレクター、コネクションウォーマー、定期的なクリーンアップ処理、あるいは決して止まってはならない非同期ループなどです。これらは、キューから個別の `Job` アイテムを消費する[キューワーカー](queues.md)とは異なります。スーパーバイザーはジョブキューを持ちません - 自分自身のループを所有し、いつスリープし、待ち、行動するかを自分で決めます。

`SupervisorRegistry` は、登録済みのすべてのスーパーバイザーを分離されたTokioタスクとして開始し、各タスクの `JoinHandle` を監視し、それが終了したとき - `Err` を返すか、`Ok` を返すか、あるいはパニックするかを問わず - その `RestartPolicy` に従って再起動します。再起動は、100msから始まり60秒で上限に達する指数バックオフによって間隔を空けられるため、クラッシュするスーパーバイザーが高速に再起動を繰り返してログを溢れさせることはありません。

## クイックスタート

スーパーバイザーを定義し、`inventory::submit!` を介して登録し、ブートストラップ時に `SupervisorRegistry::start_all()` を呼び出します。

**`src/supervisors/heartbeat.rs`:**

```rust
use async_trait::async_trait;
use std::time::Duration;
use suprnova::supervisor::{RestartPolicy, Supervisor};
use suprnova::{FrameworkError, SupervisorEntry};
use tokio_util::sync::CancellationToken;

pub struct LogHeartbeat;

#[async_trait]
impl Supervisor for LogHeartbeat {
    fn name(&self) -> &'static str { "heartbeat" }

    async fn run(&self, cancel: CancellationToken) -> Result<(), FrameworkError> {
        loop {
            tokio::select! {
                _ = cancel.cancelled() => return Ok(()),
                _ = tokio::time::sleep(Duration::from_secs(60)) => {
                    tracing::info!("supervisor heartbeat tick");
                }
            }
        }
    }

    fn restart_policy(&self) -> RestartPolicy { RestartPolicy::Always }
}

// スキャフォルドされたアプリが `inventory` を直接の依存関係として
// 追加する必要がないよう、再エクスポートされた `suprnova::inventory` を使う。
suprnova::inventory::submit!(SupervisorEntry {
    factory: || Box::new(LogHeartbeat),
});
```

**`src/bootstrap.rs`:**

```rust
use suprnova::supervisor::SupervisorRegistry;

pub async fn register() {
    SupervisorRegistry::start_all().await;
}
```

これで設定は完了です。`LogHeartbeat` スーパーバイザーは起動時に開始し、60秒おきにログを記録し、そして - `RestartPolicy::Always` が `Ok` と `Err` どちらの終了でも再起動するため - ループが何らかの理由で終了した場合には即座に再起動されます。

## 再起動ポリシー

各スーパーバイザーは、トレイトメソッドを介して自身の `RestartPolicy` を宣言します。デフォルトは `OnError` です。

| ポリシー | 再起動するタイミング… | 用途 |
|--------|-----------------|----------|
| `RestartPolicy::OnError` | `run()` が `Err` を返す、またはパニックする | 成功時には完了まで走りきるべきタスク（例: スーパーバイザーとしてラップされた一度限りの初期化ジョブ）。 |
| `RestartPolicy::Always` | `run()` が `Ok` または `Err` のいずれかを返す、またはパニックする | 本物のデーモン - 決して戻ってはならないループです。ループが何らかの理由で終了した場合、それはバグであり、再起動が正当化されます。 |
| `RestartPolicy::Never` | （決してしない） | 結果にかかわらず、一度だけ実行され、再起動されるべきではないワンショットのタスク。 |

```rust
fn restart_policy(&self) -> RestartPolicy { RestartPolicy::OnError }   // デフォルト
fn restart_policy(&self) -> RestartPolicy { RestartPolicy::Always }    // デーモンループ
fn restart_policy(&self) -> RestartPolicy { RestartPolicy::Never }     // ワンショット
```

**`Always` と `OnError` のどちらを選ぶか。** 無限ループのスーパーバイザー（`loop { ... }`）は `Always` を使うべきです - ループが `Ok(())` を返すようなことが起きれば、それは想定外の事態であり、再起動が正しい対応です。有限の作業を行い、成功時に `Ok` を返すスーパーバイザー（例えば、キャッシュを一度だけリフレッシュするもの）は `OnError` を使うべきです。そうすれば、きれいな終了が再起動を引き起こすことはありません。

**ワンショットの作業には `Never`。** スケジュールに従って動く作業には、[キューワーカー](queues.md)や[スケジュールされたタスク](scheduling.md)を優先してください。起動時に一度だけ実行され、二度と実行されてはならない何かに対して、スーパーバイザーというパターンが都合よく使える場合には `RestartPolicy::Never` を使ってください。

## パニック処理

`run()` の内部で起きたパニックはレジストリによって捕捉され、エラーとして扱われます - パニックしたスーパーバイザーは、プロセス全体を道連れにすることなく、バックオフを伴って再起動されます。レジストリは各スーパーバイザーの `JoinHandle` を監視し、標準のTokio joinの仕組みを通じてパニックを検知します。

再起動ポリシーの観点からは、パニックは常に、ポリシーにかかわらず `Err` 終了として扱われます。

- `OnError` - パニックの後に再起動します（パニックはエラーとして数えられます）。
- `Always` - パニックの後に再起動します（他のあらゆる終了と同じです）。
- `Never` - パニックの後に再起動しません（他のあらゆる終了と同じです）。

パニックは、再起動のバックオフが始まる前に、スーパーバイザー名を伴って `error!` レベルでログに記録されます。

## バックオフ

スーパーバイザーが終了し、そのポリシーが再起動を指示する場合、レジストリは代わりのタスクを生成する前に待機します。

| 連続再起動回数 | 遅延 |
|---------|-------|
| 1回目 | 100ms |
| 2回目 | 200ms |
| 3回目 | 400ms |
| 4回目 | 800ms |
| … | 毎回2倍になる |
| 上限 | 60秒 |

バックオフは、正常な実行の後にリセットされます。遅延は*連続する*再起動のたびに、60秒の上限まで倍増していきますが、少なくとも60秒間（上限と同じ長さ）動き続けた実行は正常とみなされます。次の再起動は、以前の失敗の連続で積み上がったバックオフを引き継ぐのではなく、100msの下限まで戻ります。そのため、何時間も正常に動いていたデーモンが一時的な不調に陥っても、はるか昔に積み上がった60秒の待機の後ではなく、すぐに再起動します。

このリセットは生存時間に基づくもので、意図的に保守的です。*可能な最大バックオフより長く生き延びた*実行だけが正常とみなされます。その閾値に達する前に終了した実行は、現在のバックオフをそのまま引き継ぐため、本当にフラッピングしているスーパーバイザー - 実行が一度もその閾値に届かないもの - は、それでも60秒の上限まで上がりきり、そこに留まり続けます。このリセットは、クラッシュループしているスーパーバイザーを決して覆い隠しません。

60秒の上限は、永続的に壊れたスーパーバイザーが無期限にスリープしたり、リトライのたびに外部の依存先を叩き続けたりすることを防ぎます。スーパーバイザーが高バックオフの帯域に入ったときにアラートを出すには、`error!` レベルのロギングと組み合わせてください。

## グレースフルシャットダウン

スーパーバイザーは、`run()` のパラメータとして `CancellationToken` を受け取ります。フレームワークは、`Server::run` のシャットダウンシーケンスの一部として、Ctrl-C / SIGTERM でこのトークンをキャンセルします。状態を書き出したい、進行中の作業を終わらせたい、あるいはそれ以外の形できれいに終了したいスーパーバイザーは、`cancel.cancelled()` に対して `tokio::select!` を使うべきです。

```rust
async fn run(&self, cancel: CancellationToken) -> Result<(), FrameworkError> {
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return Ok(()),
            _ = tokio::time::sleep(Duration::from_secs(60)) => {
                tracing::info!("supervisor heartbeat tick");
            }
        }
    }
}
```

フレームワークは、キャンセル後5秒間の猶予期間を設けて、スーパーバイザーの JoinSet をドレインします。その猶予期間内にトークンに応じなかったスーパーバイザーは、`JoinSet::abort_all` を介して中断されます。このドレインは、WebSocketハンドラのドレインの後（そのためWSコネクションが先にクリーンアップされます）、そしてテレメトリバッファの書き出しの前に実行されます。

トークンをまったく無視するスーパーバイザーは、5秒のウィンドウが切れるまで動き続け、その後で強制的に中断されます。スーパーバイザーが、書き出しを必要とするリソース（開いたファイルハンドル、進行中のHTTPリクエスト、書き込みが半端に終わったレコードなど）を保持している場合は、必ず `cancel.cancelled()` を select し、戻る前に後始末をしてください。

### 組み込み先と統合テスト

`Server::run` は、あなたに代わって `SupervisorRegistry::shutdown(...)` を呼び出します。`Server::run` の外側で `SupervisorRegistry::start_all()` を呼び出すコード（カスタムバイナリからフレームワークを駆動する組み込み先、あるいはスーパーバイザーを直接立ち上げる統合テスト）は、後始末の際に `SupervisorRegistry::shutdown(timeout)` も呼び出さなければなりません。そうしないと、スーパーバイザーのタスクがテストの寿命を超えて漏れ出してしまいます。

```rust
use std::time::Duration;
use suprnova::SupervisorRegistry;

// テストのセットアップ
SupervisorRegistry::start_all().await;

// … スーパーバイザーを動かす …

// テストの後始末 - 共有トークンをキャンセルし、`timeout` まで
// JoinSetをドレインし、それでも残っているものには `abort_all` を行う。
SupervisorRegistry::shutdown(Duration::from_secs(1)).await;
```

`start_all` が一度も呼ばれていなければ `shutdown` は no-op であるため、後始末から無条件に呼び出しても安全です。

## 可観測性

エラー経路での再起動はすべて、構造化フィールドを伴う `error!` レベルのログエントリを発します。

- `supervisor` - `Supervisor::name()` から得られるもの。
- `error` - `run()` の `Err` 戻り値からのエラーメッセージ、捕捉されたパニックに対する `"panic: <payload>"`、あるいは通常でないjoin失敗に対する `"join error: <detail>"`。
- `backoff_ms` - 次の生成までのバックオフ遅延（ミリ秒単位）。

パニックは同じエラーログを通じて報告されます - 別個の「panicked」というメッセージはありません。

```
ERROR suprnova::supervisor: supervisor errored; restarting after backoff supervisor=heartbeat error=connection refused backoff_ms=400
ERROR suprnova::supervisor: supervisor errored; restarting after backoff supervisor=heartbeat error="panic: \"deliberate test panic\"" backoff_ms=800
```

`RestartPolicy::Always` が `Ok(())` を返すと、同じ `supervisor` / `backoff_ms` フィールドと共に（`error!` ではなく）`warn!` が発され、メッセージは "supervisor returned Ok under Always policy; restarting" となります - 終了してはならないのにきれいに終了してしまったデーモンループを見つけるのに役立ちます。

スーパーバイザーは、`run()` の周りに自動的な tracing スパンを得ることはありません - レジストリはライフサイクル（開始、再起動）にスパンを張りますが、タスクの内部にはスパンを張りません。スーパーバイザーの内部で行われる作業にスパンのコンテキストを持たせたい場合は、自分自身で `info_span!` を発するか、ループ本体を `instrument` してください。

```rust
async fn run(&self, cancel: CancellationToken) -> Result<(), FrameworkError> {
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return Ok(()),
            _ = async {
                let span = tracing::info_span!("heartbeat.tick");
                let _guard = span.enter();
                do_work().await.ok();
                tokio::time::sleep(Duration::from_secs(60)).await;
            } => {}
        }
    }
}
```

### Suprnovaが異なる設計を選んだ理由

Laravelには直接の相当物がありません。PHPのリクエストごとにプロセスを立てるモデルは、プロセス内での常時稼働デーモンを不可能にします - 長寿命の作業は、リクエストのライフサイクルの外側で、典型的には `supervisord` が管理するワーカープロセスとして、キューを消費するかcronでスケジュールされたコマンドとして存在しなければなりません。Laravelのキューワーカー（`php artisan queue:work`）が最も近い相当物ですが、それでも外部のスーパーバイザーが再起動する、ワンショットのCLIプロセスにすぎません。

Suprnovaは、単一の長寿命プロセスの中でTokio上で動きます。常時稼働のバックグラウンドタスクは、HTTPサーバーと並んで、監督下にあるTokioタスクとして自然に収まります - 余分なプロセス境界も、外部のスーパーバイザーも、状態のための別個のIPCチャネルも必要ありません。`Supervisor` トレイトは、フレームワーク自身のタスクツリーにスコープされた、プロセス内版の `supervisord` であり、同じ「終了時に再起動 + バックオフ」という保証を持ちます。

（Laravelにもある）`Queue` ワーカーは、個別のジョブ作業のために、引き続き出荷されています - [キュー](queues.md)を参照してください。スーパーバイザーは、Laravelがフレームワークの境界の外へ完全に押し出してしまう「常時稼働」のケースをカバーします。

## v1のスコープ外

以下の項目は、意図的に先送りされています。

- **スーパーバイザーツリー（親子関係）。** 階層はありません - すべてのスーパーバイザーは、単一の `SupervisorRegistry` の下でのピアです。構造化された監督（1つのスーパーバイザーが子スーパーバイザーを所有し、再起動する）は、オーケストレーターの領分です。

- **リソース制限（cgroup、メモリ、CPU）。** リソースの制約は、systemdのユニットファイル（`MemoryMax=`、`CPUQuota=`）や、ポッドレベルのKubernetesリソースリクエスト/リミットを通じて適用してください。フレームワークは、個々のスーパーバイザータスクに対してプロセス内部のリソース制限を課しません。

- **複数マシンにわたる監督。** スーパーバイザーは、単一マシン上の単一プロセスの中で動きます。マシンをまたいで監督の判断を分散させることは、オーケストレーターの領分です（Kubernetes、Nomad、複数ホスト上のsystemdなど）。

## リファレンス

4つの主要な型 - `Supervisor`、`RestartPolicy`、`SupervisorEntry`、`SupervisorRegistry` - は、より長い `suprnova::supervisor::*` パスに加えて、クレートのルート（`suprnova::Supervisor` など）でも再エクスポートされています。2つのフリーアクセサーは `suprnova::supervisor::*` の下に留まります。

| シンボル | 目的 |
|--------|---------|
| `Supervisor` | あなたのスーパーバイザー構造体に実装するトレイト。必須メソッド: `name() -> &'static str`、`async fn run(&self, cancel: CancellationToken) -> Result<(), FrameworkError>`。任意: `restart_policy() -> RestartPolicy`（デフォルトは `OnError`）。`cancel` トークンはプロセスのシャットダウン時に通知されます - 5秒の中断ウィンドウが終わる前にきれいに終了するには、`cancel.cancelled()` を select してください。 |
| `RestartPolicy` | `OnError`、`Always`、`Never` というバリアントを持つ列挙型。レジストリが代わりのタスクをいつ生成するかを制御します。 |
| `SupervisorEntry` | インベントリのアイテム。`factory: fn() -> Box<dyn Supervisor>` を宣言します。`suprnova::inventory::submit!(SupervisorEntry { factory: || Box::new(MySupervisor) })` を介して、スーパーバイザーごとに1つのエントリを提出します。 |
| `SupervisorRegistry::start_all()` | 非同期fn。提出済みのすべての `SupervisorEntry` の値を巡回し、各スーパーバイザーを分離されたTokioタスクとしてプロセスごとのJoinSetへ生成し、再起動の監視を開始します。べき等です - プロセスごとの静的変数は `OnceLock` です。ブートストラップの `register()` から一度だけ呼び出してください。 |
| `SupervisorRegistry::shutdown(timeout)` | 非同期fn。共有のキャンセルトークンをキャンセルし、`cancel.cancelled()` を監視しているすべてのスーパーバイザーが終了するようにし、`timeout` まで JoinSet をドレインし、それでも残っているものには `abort_all` を行います。`Server::run` はこれを自身のシャットダウンシーケンスの一部として呼び出します - `Server::run` の外側で `start_all` を呼び出す組み込み先や統合テストは、タスクを漏らさないよう、これを自分で呼び出さなければなりません。`start_all` が一度も呼ばれていなければ no-op です。 |
| `suprnova::supervisor::supervisor_tasks()` / `supervisor_cancel_token()` | 背後にある JoinSet とキャンセルトークンへの `Option<&'static …>` を返すアクセサー。`Server::run` のシャットダウンシーケンスで使われます - カスタムバイナリからフレームワークを駆動する組み込み先が統合できるよう、`pub` として公開されています。アプリケーションのコードがこれらを必要とすることはないはずです。 |

## 次のステップ

- [キュー](queues.md) - スーパーバイザーとキューワーカーのどちらを選ぶかという判断と、個別ジョブという代替手段
- [スケジューリング](scheduling.md) - 長寿命のループを必要としない周期的な作業のために
- [ワークフロー](workflows.md) - 永続的な再開を必要とする、ステートフルで長時間実行される作業のために
- [ブロードキャスト](broadcasting.md) - 同じシャットダウンシーケンス（ドレインの順序）を使う
- [リクエスト ライフサイクル](lifecycle.md) - `Server::run` とシャットダウンのドレインがどこに収まるか
