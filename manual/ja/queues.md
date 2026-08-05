# キュー

`Queue` ファサードは、バックグラウンドの作業をドライバーへディスパッチし、別個のワーカープロセスにそれをドレインさせます: HTTPハンドラは速く戻り、重い処理は舞台裏で走ります。リクエストが、後回しにできる何か - メール送信、webhookの呼び出し、レポート生成など - でブロックされてしまうときは、これに手を伸ばしてください。作業を現在のタスクの中で*今すぐ*実行し、型付きの結果を受け取りたいときは[`Bus`](bus.md)と組み合わせ、1つの信号を多くのリスナーへファンアウトさせたいときは[`Events`](events.md)と組み合わせてください。

## クイックスタート

ジョブを定義し、起動時に一度だけ登録し、pushします。

```rust
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use suprnova::{error::FrameworkError, queue::{Job, Queue}};

#[derive(Serialize, Deserialize)]
struct SendWelcomeEmail { user_id: i64 }

#[async_trait]
impl Job for SendWelcomeEmail {
    fn job_name() -> &'static str { "SendWelcomeEmail" }

    async fn handle(self) -> Result<(), FrameworkError> {
        // … 実際にメールを送る
        Ok(())
    }
}

// 起動時に一度だけ（ワーカープロセスとディスパッチ元のプロセスの両方がこれを必要とする）。
Queue::set_driver(std::sync::Arc::new(suprnova::queue::MemoryQueueDriver::new()));
suprnova::queue::worker::register_job::<SendWelcomeEmail>();

// ハンドラから投入する:
Queue::push(SendWelcomeEmail { user_id: 42 }).await?;
```

ワーカープロセスは、キャンセルされるまで、設定済みのドライバーをドレインします。

```rust
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use suprnova::queue::{Queue, worker::{WorkerConfig, run_worker}};

let driver = Queue::driver()?;
let cfg = WorkerConfig {
    visibility_timeout: Duration::from_secs(60),
    poll_interval: Duration::from_millis(100),
    max_jobs: None,
};
let shutdown = CancellationToken::new();
run_worker(driver, cfg, shutdown).await;
```

スキャフォルドされたアプリでは、ワーカーはバイナリの `queue:work` サブコマンド - `cargo run -- queue:work` - によって開始され、これはあなたのHTTPサーバーと同じブートストラップを実行します。そのため、`bootstrap()` に登録されたオブザーバーとリスナーは、キューハンドラからの挿入に対しても同一に発火します。

## ドライバー

5つのドライバーがツリー内に出荷されています。`QUEUE_DRIVER` 環境変数を介して、あるいは `Queue::set_driver(...)` をプログラムから呼び出して設定してください。

| ドライバー | 用途 | 強み |
| --- | --- | --- |
| `MemoryQueueDriver` | テスト、単一プロセスのアプリ | `available_at` のための `tokio::time::DelayQueue`、仮想クロック対応 |
| `RedisQueueDriver` | 本番運用のファンアウト | コンシューマーグループ + `XAUTOCLAIM` + ZSETに支えられた遅延ジョブ |
| `DatabaseQueueDriver` | 単一DBのアプリ | Postgres/MySQLでの `FOR UPDATE SKIP LOCKED`、SQLiteでの `BEGIN` による直列化 |
| `SyncQueueDriver` | 開発、CI | `push` の時点でハンドラをインラインで実行する。ワーカー不要 |
| `NullQueueDriver` | テスト用のラッパー | 実行せずにすべてのpushを捨てる |

`Queue::bootstrap_from_env()` は `QUEUE_DRIVER` を読み取り、一致するドライバーを配線します。`Queue::bootstrap_default()` は常にメモリドライバーを配線します。サーバーの起動経路は、あなたに代わってこれらのどちらかを呼び出します - ほとんどのアプリは、環境変数を介して設定するだけです。

### 環境変数による設定

```bash
QUEUE_DRIVER=redis
QUEUE_REDIS_URL=redis://127.0.0.1:6379
QUEUE_REDIS_STREAM=suprnova-queue
QUEUE_REDIS_GROUP=default
QUEUE_REDIS_CONSUMER=consumer-1
QUEUE_VISIBILITY_TIMEOUT_SECS=60

# データベースドライバー - 先に DB::init() を実行する必要がある
QUEUE_DRIVER=database
QUEUE_DB_TABLE=jobs
```

データベースドライバーは、構築時に `QUEUE_DB_TABLE` をSQL識別子として検証するため、不正な形式の環境変数値は、SQLの組み立てに到達する前に起動を失敗させます。Redisは、内部で `AutoCommit::Disabled` を伴う sea-streamer-redis を使います。可視性タイムアウトは、コンシューマーグループの構築時に固定されるため、pop単位の `visibility_timeout` 引数はRedis上では無視されます（Redis Streamsが課す、トレイトの契約からの、文書化された分岐です）。

### Suprnovaが異なる設計を選んだ理由

Laravelは、あらゆるqueueable（キュー可能なもの）をBusを通じてルーティングし、ディスパッチ時に `ShouldQueue` ジョブを区別します。Suprnovaはこの2つを分離しています: 型付きの結果を返す同期的な作業には `Bus`、プロセスのクラッシュを生き延びる非同期の作業には `Queue` です。PHPが暗黙のルーティングを必要とするのは、そのリクエストごとにプロセスを立てるモデルが、「これを後で、別のプロセスで行う」ことを、そうでなければモデル化しにくくしているからです。Tokioはそうではありません - 明示的な `Bus::dispatch` と `Queue::push` の方が明快で速く、永続性の選択を呼び出し箇所で表面化させます。並べての比較は[`bus.md`](bus.md)を参照してください。

## pushのバリエーション

すべてのpushのバリエーションは、型付きの `J: Job` の値を受け取り、エンベロープがドライバーにコミットされた時点で戻ります - ハンドラが実行された時点ではありません。

| メソッド | 振る舞い |
| --- | --- |
| `Queue::push(job)` | 即座にキューへ投入する |
| `Queue::push_later(job, at)` | 特定の `DateTime<Utc>` に利用可能になる |
| `Queue::later(delay, job)` | 今から `delay` 後に利用可能になる |
| `Queue::push_unique(job)` | `J::unique_for` の間、`J::unique_id` による重複排除を行う。新規なら `Ok(true)`、重複なら `Ok(false)` を返す |
| `Queue::push_unique_later(job, at)` | 一意 + スケジュール済み |
| `Queue::later_unique(delay, job)` | 一意 + 遅延 |
| `Queue::bulk(vec![job1, job2, ...])` | すべてのジョブを投入する（ドライバーはネイティブな一括経路を使うことがある） |

`push_unique` は、キャッシュ層がブートストラップされていることを要求します - 重複排除ロックは、[`Cache`](cache.md)の中に、[`Idempotency::commit_on_success`](idempotency.md)を介して存在します。失敗したpushは、呼び出し元がリトライできるよう重複排除キーを解放します。成功したpushは、それを `J::unique_for` 秒間保持します。ジョブは `Job::unique_id(&self)` をオーバーライドして `Some(id)` を返さなければなりません - `None` は内部エラーを返します。

## ジョブの設定

実装ごとに振る舞いを調整するには、`Job` の関連関数をオーバーライドしてください。

```rust
use std::time::Duration;
use suprnova::queue::{BackoffSchedule, JobMiddleware};

#[async_trait]
impl Job for SendWelcomeEmail {
    fn job_name() -> &'static str { "SendWelcomeEmail" }

    async fn handle(self) -> Result<(), FrameworkError> { /* … */ Ok(()) }

    fn max_tries() -> u32 { 5 }                            // デフォルト: 3
    fn timeout() -> Option<Duration> { Some(Duration::from_secs(30)) }
    fn fail_on_timeout() -> bool { false }                 // デフォルト: false（タイムアウトはリトライする）
    fn backoff() -> BackoffSchedule {
        BackoffSchedule::Sequence { secs: vec![5, 15, 60, 300] }
    }
    fn unique_id(&self) -> Option<String> {
        Some(format!("welcome:{}", self.user_id))
    }
    fn unique_for() -> Duration { Duration::from_secs(600) }  // デフォルト: 5分
    fn middleware() -> Vec<std::sync::Arc<dyn JobMiddleware>> {
        vec![/* 下の「ジョブミドルウェア」を参照 */]
    }
}
```

## キューのルーティング

デフォルトでは、すべてのジョブが1つのキューへ行き、すべてのワーカーがそのすべてをドレインします。一部のジョブが他より遅い、あるいは重要になってきたら、専用のワーカープールが欲しくなります: 長時間実行のエクスポートが、1000件のウェルカムメールの後ろに並んでいるべきではありません。

ジョブは、自分がどこに属するかを宣言できます。

```rust
#[async_trait]
impl Job for GenerateExport {
    fn job_name() -> &'static str { "GenerateExport" }
    async fn handle(self) -> Result<(), FrameworkError> { Ok(()) }

    fn queue() -> Option<&'static str> { Some("exports") }
    fn connection() -> Option<&'static str> { None }   // デフォルトのコネクション
}
```

…そして、運用者は、ジョブに触れることなく、それを中央から上書きできます。

```rust
// bootstrap::register()
use suprnova::Queue;

Queue::route::<GenerateExport>(None, Some("heavy"));
Queue::route::<SendInvoice>(Some("redis"), Some("billing"));
```

解決は、優先度が最も高いものから順に実行されます。

1. `Queue::route` で登録されたルート
2. ジョブ自身の `Job::queue` / `Job::connection`
3. ドライバー / グローバルなデフォルト

あるフィールドに `None` を渡すと、その軸には触れません。そのため、ジョブのコネクションをルーティングしても、そのジョブがすでに宣言しているキューを乱すことはありません。

この2つの軸は、現在のところ異なる深さで動いています。**キュー**はエンドツーエンドで尊重されます - エンベロープに刻印され、ドライバーに保存され、`--queue` でフィルタされます。**コネクション**は、`JobQueueing` / `JobQueued` のライフサイクルイベントに運ばれるコネクション*名*を解決するだけで、これはリスナーやダッシュボードが目にするものです。1つのプロセスグローバルなドライバーが、それでもすべてのpushを受け取るため、ジョブのコネクションをルーティングしても、まだ別のドライバーを選ぶことはありません。今、コネクションを宣言することは、コネクションごとのドライバーが実現したときのための前方互換性であり、振る舞いを変えるものではありません。

続いて、それのためにワーカーを専用化します。

```bash
./app queue:work --queue=billing
./app queue:work --queue=exports,heavy
./app queue:work                       # 以前と同様、すべてのキューをドレインする
```

ルートを持たないジョブは `default` に属するため、`--queue=default` は、ルーティングされていない作業を見捨てるのではなく、それをドレインします。

### Suprnovaが異なる設計を選んだ理由

Laravelの `Queue::route(...)` はクラス文字列を取りますが、Suprnovaはジョブを型パラメータとして取るため、リネームまたは削除されたジョブは、サイレントに一致しなくなるルートではなく、コンパイルエラーになります。

より大きな分岐点は、ドライバーがフィルタできない場合に何が起きるかです。`QueueDriver::pop_from` は、尊重できないキューフィルタを、すべてをドレインすることへフォールバックする代わりに、**拒否します**。`billing` だけをドレインするよう指示されたワーカーが、静かにすべてのキューをドレインしてしまう場合、間違ったプールが間違ったジョブを消費するまでは、動作しているデプロイと見分けがつきません - そのため、設定ミスは最初のポーリングではっきりと表面化させられます。メモリドライバーとデータベースドライバーはネイティブにフィルタします。フィルタしないドライバー - 単一のストリームコンシューマーグループにはキューごとのストレージがないため、Redisドライバーがそれにあたります - は、誤解させるのではなく、エラーになります。

### `jobs` テーブル

`DatabaseQueueDriver` は、このスキーマを想定しています。`queue` カラムが、`--queue` によるフィルタリングを可能にしているものです。

```sql
CREATE TABLE jobs (
    id              TEXT PRIMARY KEY,
    job_name        TEXT NOT NULL,
    queue           TEXT NULL,
    envelope_json   TEXT NOT NULL,
    available_at    BIGINT NOT NULL,
    reserved_until  BIGINT NULL,
    reserved_token  TEXT NULL,
    attempts        INTEGER NOT NULL DEFAULT 0,
    created_at      BIGINT NOT NULL
);
CREATE INDEX idx_jobs_available_at ON jobs(available_at);
CREATE INDEX idx_jobs_queue ON jobs(queue);
```

`queue` はnull許容であり、ルーティングされていないジョブは `'default'` ではなく `NULL` を保存します。これは意図的なものです: 古いバイナリによって書き込まれた行は、新しいバイナリによって書き込まれたルーティングされていない行と区別できないため、バージョンが混在したフリートは、ローリングアップグレードの間も同じ作業をドレインします。

既存のテーブルへこのカラムを追加することは、フィルタリングのためだけでなく**必須**です: `push` は、ジョブがルーティングされているかどうかにかかわらず、その `INSERT` の中で `queue` カラムを名指しします。そのため、0.7.0以降のバイナリは、それを欠いたテーブルに対するすべてのpushを失敗させます。先にマイグレーションを実行し、その後でバイナリをロールしてください - 古いバイナリは自分のカラムを明示的にリストし、新しいものを無視するため、その順序は安全です。

```sql
ALTER TABLE jobs ADD COLUMN queue TEXT NULL;
CREATE INDEX idx_jobs_queue ON jobs(queue);
```

### バックオフスケジュール

| バリアント | 振る舞い |
| --- | --- |
| `Fixed { secs }` | 試行ごとに一定の遅延 |
| `Exponential { base_secs, cap_secs, jitter_ratio }` | `min(base * 2^(attempts-1), cap)` × `[1±jitter]` の範囲の乱数 |
| `Sequence { secs }` | 試行ごとに1エントリ。使い果たすと最後のエントリが繰り返される |

デフォルトは `Exponential { base_secs: 2, cap_secs: 300, jitter_ratio: 0.25 }` です - ±25%のジッターを伴う、2秒から5分までです。

## ジョブミドルウェア

6つのミドルウェアがツリー内に出荷されており、いずれも `Illuminate\Queue\Middleware\*` を反映しています。

| ミドルウェア | 振る舞い |
| --- | --- |
| `WithoutOverlapping` | 期間中 `Cache::lock` を保持する。競合時は遅延を伴って解放する |
| `RateLimited` | `RateLimiter` の予算でゲートする。ウィンドウがリセットされるまで解放する |
| `ThrottlesExceptions` | リクエストではなく、連続する*失敗*に対してレート制限する |
| `Skip::when(cond)` / `Skip::unless(cond)` | 条件が満たされたときにジョブを捨てる |
| `FailOnException` | 一致するエラーを永続的な失敗へ昇格させる（リトライなし） |
| `SkipIfBatchCancelled` | 所属するバッチがキャンセルされていたら、ジョブを捨てる |

これらは `Job` の実装に配線します。

```rust
use std::sync::Arc;
use std::time::Duration;
use suprnova::queue::{JobMiddleware, RateLimited, WithoutOverlapping};

fn middleware() -> Vec<Arc<dyn JobMiddleware>> {
    vec![
        Arc::new(
            WithoutOverlapping::new("user-42")
                .expire_after(Duration::from_secs(120))
        ),
        Arc::new(
            RateLimited::new(10, Duration::from_secs(60))
                .by("send-mail")
        ),
    ]
}
```

`WithoutOverlapping` と `RateLimited` は、キャッシュのサブシステムが起動していることを必要とします（起動時の `Cache::init` または `App::bind::<dyn CacheStore>(...)`）。

### 解放されないロックは、ジョブを失敗させない

`WithoutOverlapping` が、ハンドラの実行後にロックを解放できない場合 - キャッシュバックエンドが一瞬不調になった、コネクションが切れた、など - は、`warn` でログを記録し、それでもハンドラ自身の結果をそのまま返します。ロックはその後、`expire_after` で失効します。

これは意図的なものです。解放が実行される時点で、ハンドラはすでにその副作用をコミットしています: 行の書き込み、メールの送信、課金の実行などです。解放の失敗をジョブの失敗として報告してしまうと、ワーカーはリトライし、そのすべてを2度目にも行ってしまいます。これは、TTL分だけロックキーが保持され続けるよりも悪い結果です。本当に失敗したハンドラは、それでも自分の失敗を報告します - 解放のエラーを抑えることは、ハンドラのエラーを抑えることではありません。

### 試行回数を消費しない解放の契約

ミドルウェアは `Result<()>` ではなく `JobOutcome` を返します。4つのバリアントがあります。

- `JobOutcome::Completed` - ハンドラが実行された。ACK。
- `JobOutcome::Released { delay }` - `attempts` を増やす**ことなく**、`delay` の後に再投入する。`WithoutOverlapping`、`RateLimited` が使う。ワーカーは操作全体を `QueueDriver::release` に委ね、ツリー内のすべてのドライバーは、自分が保存しているコピーをその場で再投入するため、メッセージが予約済みと可視の両方に同時になることも、両方でもなくなることも、決してありません。試行回数は、ドライバーが不一致を起こしうる算術をワーカー側に持たせることなく保たれます - 保存されているコピーは、この実行のために増やされたことが一度もないからです。
- `JobOutcome::Failed { reason }` - 今すぐデッドレターにし、失敗ジョブストアへ永続化し、リトライしない。
- `JobOutcome::Deleted` - デッドレターにせずに予約を捨てる。`Skip` が使う。ジョブがバッチに属していた場合、コールバックが発火できるよう、バッチの `pending_jobs` はそれでも減算される。

この契約こそが、リトライの会計、メトリクス、ライフサイクルイベントにおいて、「バケットが満杯だったためスロットルされた」を「ハンドラがエラーになったため失敗した」とは異なる感触にしているものです。

### 何が1回の試行として数えられるか

ジョブが完了せずにワーカーを離れる方法は2つあり、どちらも1回の試行を消費します。

- **ハンドラが失敗した** - `Err` を返した、あるいはフレームワークの境界へパニックした場合。ワーカーはnackし、ドライバーは `attempts + 1` で再投入します。
- **ワーカーが死んだ** - OOM kill、`abort()`、segfault、`docker kill`、あるいは停止がタイムアウトしたときにスーパーバイザーが送るSIGKILLなど。何も決着しません。予約は単純に失効します。そのジョブを再取得したワーカーが、その時点で試行を課金します。

2番目のケースは、以前はタダでした。それは親切さではなく、穴でした: 自分のワーカーを確実に殺すジョブは、決して `max_tries` を使い果たすことがなく、そのため決してデッドレターにされることがありませんでした。それを確保したワーカーを次々に殺し、バイト単位で同一のまま戻ってきて、次のワーカーを殺す - 何かがワーカーを再起動させ続ける限り、それが続いていたのです。

ツリー内の3つのドライバーはすべて、これを課金します。なぜなら、`QUEUE_DRIVER` を切り替えても、ポイズンジョブを止められるかどうかが変わってはならないからです。`database` は失効した `reserved_until` を検知します。`memory` は、リーパーが予約を可視状態へ戻すときにそれを課金します。`redis` はエントリの配信回数を `XPENDING` から読み取ります。なぜなら、Redisストリームのエントリは不変であり、それ自身のカウンターだけが唯一の記録だからです。

`JobOutcome::Released` は意図的な例外です - 上の契約を参照してください。`RateLimited` によってスロットルされたジョブは一度も実行されていないため、何も負っていません。

**Redis上では、再取得には2つの時計があります。** `--visibility-timeout` は、あるエントリが再取得の対象になるまでにACKされずにどれだけ座っていられるかを設定します。もう1つの間隔は、コンシューマーがどのくらいの頻度で確認するかを支配します。ドライバーは後者を前者に結びつけるため、失われたジョブは、設定されたタイムアウトに固定の30秒を足したものではなく、そのタイムアウトのおおよそ2倍以内に戻ってきます。

**予算は、決着の時点だけでなく、ハンドラが実行される前にもチェックされます。** 他のあらゆるデッドレターの判断は、ハンドラが戻った後に起きますが、それはハンドラが戻ることを前提としています。自分のワーカーを殺すジョブは、そのチェックに到達できません。そのため、ワーカーは、試行回数がすでに使い果たされているジョブのディスパッチも拒否します - 別のワーカーを道連れにする前に、代わりにそれをデッドレターにします。これがなければ、試行を数えることは、ジョブが循環し続ける間、数字を上げるだけになってしまいます。

**これがあなたにとって意味すること。** `attempts` は、*ハンドラの失敗*ではなく、*ワーカーへの配信*を数えます。ジョブと無関係な理由で失われたワーカー - ホストの再起動、うるさい隣人が原因のOOMなど - も、そのジョブの予算から1回の試行を消費します。Laravelも同じように振る舞います。そのことを踏まえて `max_tries` の大きさを決め、べき等なハンドラを優先してください: 少なくとも1回の配信は、もともとの契約でした。これは、再配信の経路を、サイレントにではなく正直に数えさせるだけのことです。

## ライフサイクルイベント

ワーカーは、[`Event`](events.md)ファサードを通じて、Laravel形のライフサイクルイベントを発します。リスナーが受け取るのは、型付きのジョブインスタンスではなく、エンベロープの識別情報（`id`、`job_name`、`attempts`、`max_tries`、`connection`）です - ワーカーは、JSONのペイロードに対して型消去されています。`FrameworkError` は `Clone` を導出しないため、エラーは `String` として運ばれます。

| イベント | 発火するタイミング |
| --- | --- |
| `JobQueueing` | エンベロープがドライバーに届く前 |
| `JobQueued` | ドライバーが受け入れた後 |
| `JobProcessing` | ワーカーがpopし、ディスパッチしようとしているとき |
| `JobProcessed` | ハンドラが `Ok` を返したとき |
| `JobAttempted` | あらゆる終端の決着（成功、失敗、タイムアウト） |
| `JobExceptionOccurred` | ハンドラが `Err` を返し、リトライする |
| `JobReleasedAfterException` | エラー後のリトライによる再投入が起きたとき |
| `JobReleased` | ミドルウェア主導の解放（失敗ではない） |
| `JobFailed` | デッドレターにされたとき |
| `JobTimedOut` | 試行ごとのタイムアウトを超えたとき |
| `Looping` | ループの反復ごと（popの前） |
| `WorkerStarting` / `WorkerStopping` | ワーカーの生存期間ごとに一度 |
| `WorkerInterrupted` | `Queue::restart()` のシグナルが観測されたとき |

通常の `Event::listen` APIで購読してください。イベントはベストエフォートです - リスナーがいない状態での `Event::dispatch` はno-opの `Ok(())` であるため、`Event::init()` のないデプロイのワーカーは何も代償を払いません。

## 失敗したジョブのストレージ

デッドレターにされたジョブは、設定済みの `FailedJobStore` に行き着きます。

```rust
use std::sync::Arc;
use suprnova::queue::{Queue, MemoryFailedJobStore};

Queue::set_failed_store(Arc::new(MemoryFailedJobStore::new()));

// 管理ツールの中で:
let store = Queue::failed_store().unwrap();
for record in store.all().await? {
    println!("{} failed: {}", record.job_name, record.exception);
}
store.forget(some_id).await?;
store.flush(None).await?;
```

3つのバックエンドがあります。

- `MemoryFailedJobStore` - プロセス内の `Vec`。再起動で失われる。
- `DatabaseFailedJobStore` - SeaORMを介して `failed_jobs` テーブルへ永続化する。
- `NullFailedJobStore` - すべてのレコードを捨てる。Laravelの `NullFailedJobProvider` を反映している。

### ストアがレコードを拒否したとき

設定済みのストアがエラーを返した場合、ワーカーは `error` でログを記録し、ACKする代わりに**予約をそのまま残します**。ジョブは可視性の期限切れで戻ってきてリトライされます - サイレントに捨てられることはありません。

これは意図的なものです。代わりに、とにかくACKしてしまうという選択肢は、すでに試行を使い果たし、*かつ*どこにも記録されなかったジョブを捨ててしまうことになり、これは回復不能です。何度も戻ってくるジョブは回復可能です: ストアを修正すれば、次の配信は届きます。

実務上のケースは、マイグレーションされていない `failed_jobs` テーブルを指す `DatabaseFailedJobStore` です。マイグレーションするまで、デッドレターにされるジョブは、可視性タイムアウトごとに1回の再配信で循環し続け、そのたびにストアのエラーをログに記録します。本当に失敗を捨てたいのであれば、`NullFailedJobStore` を設定してください - それは成功するため、ジョブはACKされて消えます。

### リトライする

```rust
use uuid::Uuid;

// 単一のレコード - idがストアになければfalse。
Queue::retry_failed(some_id).await?;

// 一括 - 任意の締切（`before` より古いレコードだけをリトライする）。
let count = Queue::retry_all_failed(None).await?;
```

`retry_failed` はエンベロープを読み込み、`attempts`、`available_at`、`idempotency_key` をリセットし、設定済みのドライバーを通じて投入し、その後で失敗ジョブのレコードを削除します。`php artisan queue:retry <id>` に加えて `queue:flush` のセマンティクスを反映しています（リトライされた各エンベロープは、投入され、*かつ*ストアから取り除かれます）。

### `failed_jobs` スキーマ

`DatabaseFailedJobStore` は、このテーブル（あなたのマイグレーションによって管理されます）を想定しています。

```sql
CREATE TABLE failed_jobs (
    id              TEXT PRIMARY KEY,
    connection      TEXT NOT NULL,
    queue           TEXT NOT NULL,
    job_name        TEXT NOT NULL,
    envelope_json   TEXT NOT NULL,
    exception       TEXT NOT NULL,
    failed_at       BIGINT NOT NULL
);
CREATE INDEX idx_failed_jobs_failed_at ON failed_jobs(failed_at);
```

`DatabaseFailedJobStore::new` への `table` 引数は、構築時にSQL識別子として検証されます。

## キューに入れたバッチ

進捗の追跡と完了コールバックを伴って、ジョブのグループをディスパッチします。

```rust
use std::sync::Arc;
use suprnova::queue::{Queue, MemoryBatchRepository, batch::register_callback};

Queue::set_batch_repository(Arc::new(MemoryBatchRepository::new()));

// 起動時に、名前付きコールバックを登録する。
register_callback(Arc::new(SendSummary));
register_callback(Arc::new(PageOnFail));

let id = Queue::batch()
    .name("import-users")
    .add(ImportUser { id: 1 })
    .add(ImportUser { id: 2 })
    .add(ImportUser { id: 3 })
    .then("send-summary-email")
    .catch("page-on-fail")
    .finally("cleanup-temp-tables")
    .dispatch()
    .await?;

// 後で進捗を調べる:
let repo = Queue::batch_repository().unwrap();
let snap = repo.find(&id).await?.unwrap();
println!("{}/{} jobs done ({}%)", snap.processed_jobs(), snap.total_jobs, snap.progress());
```

各ワーカーは、自分のジョブをバッチに対して決着させ、`pending_jobs` がゼロに達すると、ワーカーは登録済みの `then`/`catch`/`finally` コールバックを発火させます。デフォルトでは、最初の失敗がバッチをキャンセルします。`.allow_failures()` は、残りのジョブを進み続けさせます。

### 永続的なバッチ

`MemoryBatchRepository` は再起動で失われ、進行中のすべてのバッチを見捨てます: そのカウンターは消え、`pending_jobs` は二度とゼロに達することができず、コールバックは決して発火しません。本番環境では `DatabaseBatchRepository` を使ってください。

```rust
use std::sync::Arc;
use suprnova::queue::{Queue, DatabaseBatchRepository};

Queue::set_batch_repository(Arc::new(DatabaseBatchRepository::new(db.clone())));
```

フレームワークが作成しない、2つのテーブルです - `jobs` や `failed_jobs` と同じように、あなたのマイグレーションへ追加してください。

```sql
CREATE TABLE job_batches (
    id            TEXT PRIMARY KEY,
    name          TEXT NOT NULL,
    total_jobs    INTEGER NOT NULL,
    options_json  TEXT NOT NULL,
    created_at    INTEGER NOT NULL,
    cancelled_at  INTEGER NULL,
    finished_at   INTEGER NULL
);

CREATE TABLE job_batch_settlements (
    batch_id   TEXT NOT NULL,
    job_id     TEXT NOT NULL,
    failed     INTEGER NOT NULL,
    settled_at INTEGER NOT NULL,
    PRIMARY KEY (batch_id, job_id)
);
```

`DatabaseBatchRepository::with_tables(db, batches, settlements)` を使えば、それらの名前を自分で指定できます。どちらの名前も、構築時にSQL識別子として検証されます。

`pending_jobs` と `failed_jobs` は**カラムではありません**。それらは、読み取りのたびに決着の行から導出されます -

```text
pending_jobs = max(0, total_jobs - COUNT(settlements))
failed_jobs  = COUNT(settlements WHERE failed)
```
 -
なぜならキューは少なくとも1回であり、再配信が起きるたび、ACKが重複するたび、あるいはワーカーが作業を終えてから記録する間に死ぬたびに、同じジョブが複数回決着してしまうからです。決着ごとに減算するカウンターは、そのたびにずれていき、そのずれは見た目だけの問題ではありません: `pending_jobs` はコールバックをゲートするため、早すぎるゼロが、バッチの他のジョブがまだ実行中であるうちに `then` を発火させてしまいます。カウントが導出されたものであり、主キーが `(batch_id, job_id)` にあることで、繰り返しの決着は何も挿入せず、間違えるべきカウンターがそもそも存在しません - 1つのプロセスの中だけでなく、プロセスをまたいでもです。

### ディスパッチが途中で失敗したとき

`dispatch()` の途中で `driver.push` が失敗した場合、すでにキューへ到達していたジョブは本物であり、すでにバッチidを刻印されています。そのため、バッチは削除されるのではなく決着されます: pushされ*なかった*エンベロープはすべて失敗したジョブとして記録され、バッチはキャンセルされます。

`total_jobs` は、それでもあなたが求めた数を数えます。`failed_job_ids` は、届かなかったジョブを正確に名指しします。すでにキューに入っていたものは通常どおり決着し、`SkipIfBatchCancelled` は残りを捨てます - そのため、`pending_jobs` はそれでもゼロに達し、あなたの `catch`/`finally` コールバックはそれでも実行されます。何も一切pushされなかった場合は、それを行うべきワーカーが残っていないため、`dispatch` 自身がそれらを発火させます。どちらの場合も、元のpushエラーは返ってきます。

### バッチのオプション

| オプション | ビルダーメソッド | 効果 |
| --- | --- | --- |
| 失敗を許容する | `.allow_failures()` | ジョブが失敗した後もスケジューリングを続ける |
| Thenコールバック | `.then(name)` | すべてのジョブが成功したときに実行される |
| Catchコールバック | `.catch(name)` | 最初の失敗のときに実行される |
| Finallyコールバック | `.finally(name)` | どちらにせよバッチが決着した後に実行される |
| キャンセルをスキップする | ジョブに付けた `SkipIfBatchCancelled` ミドルウェア | バッチがキャンセルされたら残りのジョブを捨てる |

### `BatchCallback` の実装

```rust
use async_trait::async_trait;
use suprnova::queue::{Batch, BatchCallback};
use suprnova::error::FrameworkError;

pub struct SendSummary;

#[async_trait]
impl BatchCallback for SendSummary {
    fn name(&self) -> &'static str { "send-summary-email" }

    async fn handle(&self, batch: Batch, error: Option<String>) -> Result<(), FrameworkError> {
        let subject = match error {
            Some(_) => format!("Batch {} failed", batch.name),
            None    => format!("Batch {} done - {} jobs", batch.name, batch.total_jobs),
        };
        // … メールを送る
        Ok(())
    }
}
```

起動時に `batch::register_callback(Arc::new(SendSummary))` で登録します。コールバックは `name()` をキーとします - バッチのオプションはコールバックの名前を保存するため、プロセスの再起動は、クロージャをデシリアライズしようとする代わりに、ルックアップによって登録済みのコールバックを拾い上げます（Rustのクロージャはシリアライズできません）。

## キューに入れたチェーン

各リンクが、前のもののハンドラがACKした後にだけ実行される、逐次的なワークフローです。

```rust
Queue::chain()
    .add(GenerateReport { id: 99 })?
    .add(UploadToBucket { id: 99 })?
    .add(NotifyOwner { id: 99 })?
    .dispatch()
    .await?;
```

最初のエンベロープは即座に投入されます。残りは、その `chain_remaining` ペイロードフィールドの上を運ばれます。成功した決着のたびに、ワーカーは次のエントリを取り出してディスパッチします。失敗はチェーンを断ち切ります - それ以降のリンクは決して投入されません。

### 終端の決着

チェーンされたジョブを終わらせることは、2つのことを意味します: 後継を投入することと、今終わったジョブを解放することです。2つの別個の操作として行えば、安全な順序はありません。先にACKすれば、その間隙でのクラッシュはチェーンの残りを永久に失います - リトライの元になるものがキューに何も残っていません。先にpushすれば、同じクラッシュが終わったジョブを再配信してしまい、そのハンドラが再び実行され、後継が2回投入されてしまいます。

そこでワーカーは、`QueueDriver::settle(token, follow_ups)` を介して、両方を一度にドライバーへ渡します。

| 結果 | 意味 |
| --- | --- |
| `Settled::Atomically` | 後継が投入され、予約が1つのトランザクションの中で捨てられた |
| `Settled::Stale` | 予約は別のコンシューマーに再取得されていた。投入されたものも捨てられたものも**何もない** |
| `Settled::Unsupported` | このドライバーはトランザクショナルに決着できない |

`DatabaseQueueDriver` はこれを実装しています: 両方の効果は1つのトランザクションであり、予約をキーとする `DELETE` はフェンスも兼ねます。ハンドラの実行中に可視性タイムアウトが失効し、別のワーカーがそのジョブを拾ってしまった場合、そのdeleteは何にも一致せず、トランザクションはロールバックし、あなたは何も投入していない状態で `Stale` を受け取ります。2ステップの決着では、これを表現することがまったくできません: あなたのpushは成功し、新しい所有者のpushも成功し、チェーンは分岐してしまいます。

Redisとインメモリドライバーは `Unsupported` を返し、push-before-ackの順序を保ちます。これは、永久な損失を、少なくとも1回の重複と引き換えにします。これはフレームワークが文書化している契約であり、それが、チェーンされたエンベロープのidがランダムではなく前者から導出される理由です - 再配信されたステップは、以前にpushしたidを再びpushするため、その重複は同じ論理的なステップとして認識可能です。

後続の書き込みとACKが1つのトランザクション領域を共有するドライバーを書くのであれば、`settle` を実装してください。そのデフォルトは `Unsupported` を返すため、これが存在する前に書かれたドライバーは、変更なく動作し続けます。

## イントロスペクション

```rust
Queue::size().await?;            // 合計
Queue::pending_size().await?;    // available_at <= 現在時刻、かつ未予約
Queue::delayed_size().await?;    // available_at > 現在時刻
Queue::reserved_size().await?;   // popされているが、まだACKされていない
Queue::clear().await?;           // すべてのエンベロープを捨て、件数を返す
Queue::driver_name()?;           // ログ/管理用の、設定済みドライバー名
```

`QueueDriver` トレイトは、`size` / `pending_size` / `reserved_size` / `delayed_size` / `clear` のデフォルトを宣言しています。`MemoryQueueDriver` と `DatabaseQueueDriver` はこれらをネイティブに実装します。`RedisQueueDriver` は `size` / `clear` に対して「unsupported」エラーを返します - それらには管理用のredis-cliを使ってください。

## ワーカーの再起動シグナル

`php artisan queue:restart` に相当するのは次です。

```rust
Queue::restart().await?;
```

この信号は、ミリ秒単位のタイムスタンプとして `Cache` の中に存在します。ワーカーはループごとに一度ポーリングし、タイムスタンプが自分の開始時刻より新しければきれいに終了します。新しいワーカーが、前のワーカーが止まった場所から引き継げるよう、スーパーバイザー（systemd、Kubernetes、`supervisor` モジュールなど）と組み合わせてください。

## グレースフルシャットダウン

ワーカーの `CancellationToken` は、次のpopの境界で発火し、ディスパッチの途中では決して発火しません。すでにpopされていたハンドラは、ワーカーが終了する前に、（設定されていれば自分自身の `Job::timeout()` に縛られて）完了まで走ります。つまり、進行中の副作用が途中で引き裂かれることはありませんが、SIGTERMは、ドレインするのに、ジョブごとのタイムアウトの分だけ時間がかかることがあります。長寿命のワーカーに対して定期的な再起動の戦略を取るには、`WorkerConfig::max_jobs` を設定してください。ワーカーは、結果にかかわらず、その回数だけ決着した後にきれいに終了します。

## 決着のメトリクス

ワーカーは、ACK/NACKが失敗するたびに、[`Metrics`](observability.md)を介して `queue.settlement.failures` カウンターを発します。属性: `operation`（`"ack"` | `"nack"`）、`driver`（設定済みドライバーの名前）、`job`（job_name）、`outcome`（`"success"`、`"dead_letter"`、`"retry"`、`"deleted"`、`"timeout_dead_letter"`、`"timeout_retry"`、`"released"`）。

ここでの非ゼロの率は、少なくとも1回の配信が、成功した副作用を再配信してしまうか、試行の会計を失ってしまう可能性があることを意味します - これについては明示的にアラートを設定してください。

## 型付きエラー

`MaxAttemptsExceeded`、`TimeoutExceeded`、`ManuallyFailed` は、Laravelの `MaxAttemptsExceededException` / `TimeoutExceededException` / `ManuallyFailedException` を反映しています。ワーカーは、関連する原因をデッドレターの `JobFailed` イベントに添付するため、リスナーは、エラーメッセージを部分文字列検索する代わりに、パターンマッチできます。

## コネクション名

ワーカーは、あらゆるライフサイクルイベントに、コネクション名でタグを付けます。デフォルトでは、これはドライバーの `name()`（例えば `"memory"`、`"redis"`、`"database"`）です。複数のコネクションを同時に実行するアプリは、上書きできます。

```rust
Queue::set_connection_name("orders-redis");
```

## テスト

`Queue::fake()` のセマンティクスは `queue::testing` にあります。

```rust
let _guard = suprnova::queue::testing::install_fake();
my_code_that_dispatches_jobs().await;

suprnova::queue::testing::assert_pushed::<SendWelcomeEmail>(|j| j.user_id == 42);

// 遅延ディスパッチについては、スケジュールされたタイムスタンプを固定する:
suprnova::queue::testing::assert_pushed_later::<SendWelcomeEmail>(|j, at| {
    j.user_id == 42 && at > chrono::Utc::now()
});
```

フェイクのガードは、プロセス全体のmutexを介して並行テストを直列化します。pushごとに `(payload, available_at)` をキャプチャし、`Drop` 時にクリアします。フェイクモードでは、`push_unique` は常にpushを新規として記録します - ドライバーが配線されていないときは、重複排除は無関係です。

## べき等性は、ワーカーとあなたの間の契約

Redisに支えられたキューのドライバーは、`nack` をアトミックにできません - `XADD` と `XACK` は別個のコマンドです。その間でのクラッシュは、`XAUTOCLAIM` を介してメッセージを再配信します。インメモリとデータベースのドライバーは、試行ごとに厳密に1回ですが、ワーカーループはドライバーを区別しないため、**本番環境のデプロイにおけるすべてのジョブハンドラは、べき等でなければなりません**。

典型的なコマンド風のジョブでは、安定した操作ごとのキー（エンティティid、呼び出し元が渡すリクエストidなど）をキーとして、ハンドラの本体を[`Idempotency::once`](idempotency.md)や[`Idempotency::commit_on_success`](idempotency.md)でラップしてください。リトライが、再実行をスキップするのではなく*元の*結果を返さなければならない場合は、成功した値を記録し、後の配信でそれを再生する `Idempotency::remember` を使ってください。

## 次のステップ

- [バス](bus.md) - 型付きの結果を伴う同期的なディスパッチャー
- [イベント](events.md) - pub/subのファンアウト
- [べき等性](idempotency.md) - 少なくとも1回の配信のために、ハンドラが守るべき契約
- [キャッシュ](cache.md) - `push_unique`、`WithoutOverlapping`、`RateLimited` を支える
- [モックとフェイク](mocking.md) - `Queue::fake` を含む、あらゆるフェイクのガード
