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
    queues: Vec::new(),
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

`FailoverQueueDriver` は6つ目のバックエンドではありません。上記のドライバーの順序付きリストをラップし、ある接続が拒否したプッシュが次へ落ちていくようにします。[フェイルオーバー接続](#フェイルオーバー接続)を参照してください。

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

## フェイルオーバー接続

`FailoverQueueDriver` は、順序付きの接続リストをラップします。最初の接続が拒否したプッシュは次で再試行され、以下同様にリストを落ちていくため、Redisの障害があらゆるディスパッチを失われたジョブに変えてしまうことはありません。

環境変数から設定します:

```bash
QUEUE_DRIVER=failover
QUEUE_FAILOVER_CONNECTIONS=redis,database

# 各接続は、それ自身が単独で QUEUE_DRIVER であった場合とまったく同じように、
# 自身の変数を読みます。
QUEUE_REDIS_URL=redis://127.0.0.1:6379
QUEUE_DB_TABLE=jobs
```

あるいは、接続が環境変数では表現できない実行時の設定を必要とするときは、自分で配線してください:

```rust
use std::sync::Arc;
use std::time::Duration;
use suprnova::queue::{
    DatabaseQueueDriver, FailoverQueueDriver, Queue, QueueDriver, RedisQueueDriver,
};
use suprnova::{DB, FrameworkError};

pub async fn register() -> Result<(), FrameworkError> {
    let redis = RedisQueueDriver::connect(
        "redis://127.0.0.1:6379",
        "suprnova-queue",
        "default",
        "consumer-1",
        Duration::from_secs(60),
    )
    .await?;
    let database =
        DatabaseQueueDriver::new(DB::connection()?.inner().clone(), "jobs".to_string())?;

    let failover = FailoverQueueDriver::new(vec![
        ("redis".to_string(), Arc::new(redis) as Arc<dyn QueueDriver>),
        ("database".to_string(), Arc::new(database) as Arc<dyn QueueDriver>),
    ])?;
    Queue::set_driver(Arc::new(failover));
    Ok(())
}
```

各エントリの `String` は、`QueueFailedOver` イベントで報告される接続のラベルです。2つの接続が同じドライバーで動くこともあるため、これはドライバーの型からは導かれません。

`QUEUE_FAILOVER_CONNECTIONS` は `QUEUE_DRIVER=failover` のときに必須であり、そのリストは `failover` 自身を含められません。存在しないドライバーを指すエントリは、`QUEUE_DRIVER` が自分自身に適用する「警告してメモリを使う」というフォールバックではなく、起動エラーになります: フェイルオーバーの連なりの内側では、タイプミスが黙ってインメモリの接続になってしまえば、永続的なリストの中に揮発性のバックエンドを置くことになるからです。

### 書き込みはフェイルオーバーし、読み取りはしない

接続のリストを歩くのは `push` と `bulk_push` だけです。それ以外のすべての操作 - `pop`、`ack`、`nack`、`release`、`settle`、`clear`、4つのカウンター、3つの検査の一覧 - は、**最初の**接続へ行き、ほかへは行きません。

その非対称性は、抜け落ちではなく契約です。予約のトークンは、それを発行したドライバーにとってしか意味を持たないため、別の接続に対してackしても何も決着させず、両方を壊してしまいます。カウンターと一覧も同じ規則に従います。そうすることで、あなたが検査するものが、どのワーカーの視界とも一致しないバックエンドをまたいだ合計ではなく、この接続のワーカーがドレインするものと一致するからです。

**フェイルオーバー接続の上のワーカーは、プライマリだけをドレインします。** フォールバックへフェイルオーバーしたジョブには、そのフォールバックの接続に対して直接走るワーカーが必要です:

```bash
# フェイルオーバーの連なりのプライマリをドレインします。
QUEUE_DRIVER=failover QUEUE_FAILOVER_CONNECTIONS=redis,database ./app queue:work

# データベースへフェイルオーバーしたものをドレインします。これも走らせてください。
QUEUE_DRIVER=database ./app queue:work
```

Laravelのドキュメントも、同じ理由で同じ警告を載せています。

これはチェーンにも及びますが、通るのは1つの扉だけです。ワーカーは1回の呼び出し、`settle` で、ジョブを決着させると同時に[キューに入れたチェーン](#キューに入れたチェーン)の次の環をエンキューし、デコレーターはその呼び出しをプライマリだけに委譲します。そのため、databaseドライバーのようなトランザクション的なプライマリでは、プライマリが落ちていれば決着が失敗し、何もフェイルオーバーしません: ワーカーは予約をそのまま残し、可視性の失効がジョブを再配送します。フェイルオーバーが起こるのは、プライマリが `Settled::Unsupported` と答えたときです。memoryとRedisのドライバーがそう答えます。というのも、そのときワーカーは次の環を、ほかのどのプッシュとも同じようにバインドされたドライバーを通じてプッシュし - そのプッシュがフェイルオーバーするからです。そのチェーンの残りは、そこでフォールバック接続のワーカーを待つことになります。それがなければ、チェーンは止まります - 環は永続化されていて何も失われませんが、それを走らせるものもありません。

### `QueueFailedOver` イベント

プッシュを拒否した各接続は、`queue::events::QueueFailedOver { connection, job_name, exception }` をディスパッチしますが、それはその接続を失敗状態*へ*移すプッシュのときだけです。すでに失敗していると分かっている接続は、後のプッシュがそこで成功して再び武装するまで、静かなままです。4時間の障害は、ディスパッチごとに1つではなく1つのイベントを生みます。これが、それをアラートとして使えるものにしています。

`connection` は、ジョブを受け入れた接続ではなく、失敗した接続のラベルです。

すべての接続がプッシュを拒否したとき、そのプッシュは最後の接続のエラーを返します。`bulk_push` は各エンベロープを個別にプッシュするため、それぞれが自分だけで落ちていきます: プライマリが半分だけ受け入れたバッチが、まるごとフォールバックへ再プッシュされることは決してなく、各エンベロープは構築されたときの `available_at` を保ちます。バッチは原子的ではありません。1つのエンベロープがすべての接続に拒否された場合、`bulk_push` は、それより前のエンベロープをすでにエンキューしたうえで、そのエンベロープのエラーを返します。

フェイルオーバーは重複排除ではありません。デコレーターは、ある接続が受け入れたエンベロープを再試行することは決してありませんが、エンベロープを書き込んで*その後*に失敗を報告する接続は、次の接続で重複を生みます。「書き込んだが確認応答を失った」ことは、「そもそも受け取らなかった」ことと区別できないからです。どちらのコピーも同じジョブidを運びます。それがこのフレームワークの少なくとも1回の配信の契約であり、ほかのあらゆる場所でハンドラのべき等性を要件にしているのと同じものです - [べき等性は、ワーカーとあなたの間の契約](#べき等性は-ワーカーとあなたの間の契約)を参照してください。

### Suprnovaが異なる設計を選んだ理由

Laravelのフェイルオーバー接続は `config/queue.php` の `connections` の配列であり、接続のレジストリを通じて解決されます。Suprnovaには接続ごとのドライバーのレジストリがなく - 1つのドライバーがプロセス全体にバインドされます - そのため、ラベルは `QUEUE_FAILOVER_CONNECTIONS`（あるいは `FailoverQueueDriver::new` に渡す `String`）から来て、読み取りは名前付きの接続にではなく最初の*ドライバー*へ委譲されます。

Laravelの `FailoverQueue::bulk` は、それぞれの遅延が生き残るようジョブを個別にループします。Suprnovaは、どのドライバーがそれを見るよりも前に遅延をエンベロープの上で解決するため、エンベロープごとのループはそれを無償で保ちます - ただし、半分だけ着地したバッチが二重にプッシュされるのを防いでいるのはやはりそのループなので、ループは残ります。

## プッシュの変種

どのプッシュの変種も、型付きの `J: Job` の値を取り、エンベロープがドライバーへコミットされた時点で返ります - ハンドラが走る時点ではありません。

| メソッド | 振る舞い |
| --- | --- |
| `Queue::push(job)` | ただちにエンキューする |
| `Queue::push_later(job, at)` | 特定の `DateTime<Utc>` に利用可能になる |
| `Queue::later(delay, job)` | 今から `delay` の後に利用可能になる |
| `Queue::push_with(job, overrides)` | 1回のプッシュごとの `EnvelopeOverrides` でただちにエンキューする |
| `Queue::push_after_commit(job)` | 周囲の `DB::transaction` がコミットしたときにエンキューする |
| `Queue::later_with(delay, job, overrides)` | `delay` 後に利用可能。1回のプッシュごとの `EnvelopeOverrides` を伴う |
| `Queue::push_unique(job)` | `J::unique_for` の間、`J::unique_id` で重複排除する。エンベロープがプッシュされたときは `Ok(true)`、生きている重複排除キーがそれを抑制したときは `Ok(false)` を返す |
| `Queue::push_unique_later(job, at)` | 一意 + スケジュール |
| `Queue::later_unique(delay, job)` | 一意 + 遅延 |
| `Queue::bulk(vec![job1, job2, ...])` | すべてのジョブをプッシュする（ドライバーはネイティブの一括経路を使うことがある） |

`push_unique` は、キャッシュ層がブートストラップされていることを必要とします - 重複排除のロックは[`Cache`](cache.md)の中に、[`Idempotency::commit_on_success`](idempotency.md)を介して存在します。プッシュが失敗すると重複排除キーは解放されるため、呼び出し元はリトライできます。プッシュが成功すると、`J::unique_for` 秒の間それを保持します。ジョブは、`Some(id)` を返すよう `Job::unique_id(&self)` をオーバーライドしなければなりません - `None` は内部エラーを返します。

この真偽値が答えるのは1つの問い - 「このジョブはキューに載ったか?」 - であり、その裏には3つ目のケースがあります。プッシュの実行中に重複排除のロックのリースが失われた場合でも、プッシュは完了し（べき等性の層は、すでに効果を及ぼしたかもしれない本体を決してキャンセルしません）、あなたはやはり `Ok(true)` を受け取り、ジョブとその一意キーを名指しする `warn` レベルのログが出ます。ジョブはキューに載っています。証明されていないのは、他の誰も同じものを並行してキューに載せなかった、ということのほうです。あなたのハンドラは、すでに再配信に耐えなければならないため、これに追加の対処は必要ありません - しかし、それが大量に出るということは、重複排除のロックを支えるキャッシュが苦しんでいるということなので、ログはそこにあります。

### 処理が始まるまでの一意性

一意性のロックは通常、ジョブが走り終えた後も含め、`unique_for` の窓の全体にわたって持続します。そのロックが実行を直列化するためではなく、*キューに入った*重複をまとめるために存在するのなら、処理が始まった瞬間にロックを解放することへオプトインしてください:

```rust
use std::time::Duration;
use suprnova::{FrameworkError, Job, async_trait};

#[derive(serde::Serialize, serde::Deserialize)]
struct RebuildSearchIndex {
    index: String,
}

#[async_trait]
impl Job for RebuildSearchIndex {
    fn job_name() -> &'static str { "rebuild-search-index" }
    fn unique_id(&self) -> Option<String> { Some(self.index.clone()) }
    fn unique_until_processing() -> bool { true }
    fn unique_for() -> Duration { Duration::from_secs(3600) }

    async fn handle(self) -> Result<(), FrameworkError> {
        // 20分走る再構築が、2分目に届いた再ディスパッチを
        // 飲み込んでしまうことは、もうありません。
        Ok(())
    }
}
```

ワーカーは、ジョブのミドルウェアの通過の後、ハンドラが走る直前にロックを解放します。そこから4つの帰結が出てきます:

- ミドルウェアがキューへ戻したジョブは、そのロックを保ちます。まだ処理を始めていないため、重複にとっては何も変わっていないからです。
- ミドルウェアがそれ以外の方法でショートサーキットしたジョブは、そのロックを手放します。そもそも処理されることがないからです。これには、ジョブを削除すること、デッドレターに送ること、そしてハンドラを一度も呼ばずに完了と報告することが含まれます。
- 失敗したジョブはロックを解放し、それでもリトライされます。処理が始まった瞬間にロックは去っているため、失敗した試行がバックオフを待つあいだに重複がエンキューでき、同じ一意idに対して2つのエンベロープを抱えることになります。これが、このオプトインが行う取引です。リトライがその枠を保持し続けなければならないのなら、`unique_until_processing` はオフのままにして、試行の連なり全体を `unique_for` のTTLに任せてください。
- 解放は所有者スコープです。`push_unique` はロックの所有者トークンをエンベロープに記録し、ワーカーはそのトークンで解放するため、再配送された試行が、その後により新しいディスパッチが獲得したロックを解放することは決してありません。

`unique_until_processing` が必要とするのは、`push_unique` が必要とするのと同じ2つです: `Some(id)` を返す `unique_id` と、ブートストラップ済みのキャッシュ層です。

`sync` ドライバーの下では、ハンドラはロックを取った `push_unique` の呼び出しの内側でインラインに走るため、ジョブは、自身の呼び出し元が名目上まだ保持しているロックを解放することになります。そのハンドラが `unique_for` の3分の1より長く走ると、重複排除のリースを更新する側はロックが消えていることに気づいてリース喪失の警告をログに記録し、その上に `push_unique` 自身の「排他性を証明できなかった」という警告が重なります。ここではどちらも、障害ではなく想定どおりです: ジョブは走り、プッシュは `Ok(true)` を返し、ロックはジョブ自身が解放したから消えているのです。

### Suprnovaが異なる設計を選んだ理由

Laravelは、*普通の*一意ジョブのロックを、ハンドラが返った時点で解放します。Suprnovaは代わりに、そのロックを `unique_for` のTTLとともに失効させます。これは、ワーカーがジョブの途中で死んだときにも重複排除の窓を誠実に保ちます: あなたが設定した窓が、ハンドラが返ったかどうかにかかわらず、あなたが得る窓です。`unique_until_processing` の振る舞いは、両方のフレームワークで同じです。

またSuprnovaは、一意性のロックを強制的に解放することも決してありません。Laravelは、所有者トークンを運ばない最初の試行のために、強制解放へフォールバックします。Suprnovaのワーカーにトークンなしで届くエンベロープは、そのトークンが存在するより前にキューへ入れられたエンベロープだけであり、それらは、より新しいディスパッチのロックを削除する危険を冒すのではなく、TTLによる失効を保ちます。

### デバウンス - 最初ではなく、最後のディスパッチを残す

`push_unique` は重複を抑え、**最初の**ディスパッチを残します。デバウンスはその逆で、**最後の**ものを残します。「この注文が変わった」という20個のイベントのバーストは、20回目からウィンドウ1つ分の後に走る、最新のペイロードを運ぶ1回のリインデックスになります。

```rust
use std::time::Duration;
use suprnova::{FrameworkError, Job, async_trait};

#[derive(serde::Serialize, serde::Deserialize)]
struct ReindexOrder {
    order_id: u32,
}

#[async_trait]
impl Job for ReindexOrder {
    fn job_name() -> &'static str { "reindex-order" }
    fn debounce_for() -> Option<Duration> { Some(Duration::from_secs(30)) }
    fn max_debounce_wait() -> Option<Duration> { Some(Duration::from_secs(300)) }
    fn debounce_id(&self) -> Option<String> { Some(self.order_id.to_string()) }

    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
}
```

- `debounce_for` がウィンドウです: ディスパッチのたびにリセットされるため、実行は*直近の*ディスパッチの30秒後に起こります。
- `max_debounce_wait` は、途切れないバーストが作業を永遠に先送りしてしまうのを止めます。バーストが5分間先送りし続けたなら、次のディスパッチは遅延なしでキューへ入ります。そのあとウィンドウは改めて始まるため、各バーストは、自分自身の最初のディスパッチから最大待ち時間を測ります。
- `debounce_id` はウィンドウをスコープします。注文7に対する20回の更新は1回の実行になり、注文8に対する更新は、それらの影響を受けません。これを省くと、そのジョブのあらゆるディスパッチが1つのウィンドウを共有します。

あらゆるディスパッチは、それでもenqueueされます。畳み込みはワーカーで決着します: プッシュのたびにキャッシュのトークンが上書きされ、ワーカーは、そのトークンをより新しいディスパッチに置き換えられたエンベロープをすべて落とし、それをackして `JobDebounced` を発行します。生き残った実行が、最も古いものではなく最新のペイロードを運ぶのは、これによってです。トークンが期限切れになっていたり追い出されていたりした場合、そのジョブは実行されます - デバウンスはフェイルオープンします。トークンが失われたことは、ほかの誰かがそのウィンドウを所有している証拠ではないからです。

[`sync` ドライバー](#ドライバー)にはワーカーがないため、あらゆるディスパッチをインラインで実行し、何ひとつ畳み込まれることはありません。Laravelのsyncドライバーも同じように振る舞います。`Queue::bulk` はドライバーのレベルでプッシュし、ウィンドウを開始することもないため、bulkでプッシュされたデバウンス対象のジョブは、すべてのコピーが実行されます。Laravelの `Queue::bulk` も、同じ理由で自身のデバウンスの獲得をスキップします。

ウィンドウが呼び出し元に属するときは、代わりに呼び出し箇所で設定してください:

```rust
use suprnova::queue::DebounceOptions;

Queue::push_debounced(
    ReindexOrder { order_id: 7 },
    DebounceOptions::new(Duration::from_secs(30))
        .max_wait(Duration::from_secs(300))
        .id("7"),
)
.await?;
```

1つのジョブが `debounce_for` と `unique_id` の両方を宣言することはできません: 一意性はバーストの最初のディスパッチを残し、デバウンスは最後のものを残すため、プッシュは両方を名指しするエラーを返します。チェーンとバッチがデバウンス対象のジョブを拒否するのも、関連する理由からです - 追い越されたリンクは落とされ、それはチェーンの残りを取り残すことになり、そして落とされたバッチのジョブは、バッチの保留カウントをゼロより上に残すため、そのコールバックが決して発火しなくなるからです。

### `EnvelopeOverrides` によるプッシュごとの上書き

`Queue::push_with` と `Queue::later_with` は、ジョブ自身の既定値とは異なるキュー、接続、タイムアウト、リトライ動作が必要な1回のディスパッチのために、ジョブと並べて `EnvelopeOverrides` を受け取ります:

```rust
use std::time::Duration;
use suprnova::queue::{EnvelopeOverrides, Queue};

let overrides = EnvelopeOverrides {
    queue: Some("priority".into()),
    timeout: Some(Duration::from_secs(10)),
    max_tries: Some(1),
    ..Default::default()
};

Queue::push_with(SendWelcomeEmail { user_id: 42 }, overrides.clone()).await?;

// 遅延させる対応物です。`Queue::later` と `Queue::push` の関係を映しています。
Queue::later_with(Duration::from_secs(60), SendWelcomeEmail { user_id: 42 }, overrides).await?;
```

各フィールドのデフォルトは `None` で、通常の `Queue::push` が行う解決に委ねます。`Some` のフィールドはこのプッシュについてそのすべてに優先し、[`Queue::route`](#キューのルーティング) に登録したルートと、ジョブ自身のそのフィールドの `Job::*` 宣言の両方を上回ります:

| フィールド | 上回るもの |
| --- | --- |
| `queue` | `Queue::route`、`Job::queue()` |
| `connection` | `Queue::route`、`Job::connection()` |
| `timeout` | `Job::timeout()` |
| `fail_on_timeout` | `Job::fail_on_timeout()` |
| `max_tries` | `Job::max_tries()` |
| `backoff` | `Job::backoff()` |
| `after_commit` | `Job::after_commit()` |

`EnvelopeOverrides` は `Mail::on_queue` / `.on_connection()` と `Notify::queue` の通知ごとのキュー調整の両方が構築されるプリミティブです。[メール](mail.md#queueing)と[通知](notifications.md)を参照してください。

### ジョブが宣言する遅延

すべての呼び出し箇所で `Queue::later(Duration::from_secs(60), job)` を繰り返す代わりに、ジョブは自身の既定遅延を持てます:

```rust
impl Job for SendDigest {
    // ...
    fn delay() -> Option<Duration> { Some(Duration::from_secs(60)) }
}
```

`Queue::push(job)`、`Queue::push_with(job, overrides)`、`Queue::push_unique(job)`、`Queue::bulk(vec![job1, job2])` はすべてこれを尊重します。`available_at` は `now` ではなく `now + J::delay()` になります。`Queue::bulk` は呼び出しごとに遅延を1回解決します。ベクター内のすべてのジョブは同じ具体的な `J` を共有し、そのため同じ `Job::delay()` を持つからです。

明示的な呼び出し箇所の遅延は常に優先されます: `Queue::push_later(job, at)`、`Queue::later(delay, job)`、`Queue::later_with(delay, job, overrides)`、`Queue::push_unique_later(job, at)`、`Queue::later_unique(delay, job)` はすべて、呼び出し側が渡したタイムスタンプまたは遅延をそのまま使い、`Job::delay()` は参照しません。ジョブ型のすべてのディスパッチを既定で遅延させるならトレイトメソッドを、型が宣言していない特定のディスパッチだけに遅延が必要なら `later` / `push_later` の変種を使ってください。

バッチとチェーンもこれを参照しません: `Queue::batch()...add(job)` と `Queue::chain()...add(job)?` はどちらも、`add` を呼んだ瞬間を `available_at` に設定してエンベロープを構築します。そのため、`Job::delay()` を宣言したジョブは、同じジョブを素の `Queue::push(job)` で投入すれば待つ場合でも、バッチまたはチェーンの一部として即座にディスパッチされます。バッチまたはチェーンのステップに遅延が必要なら、ジョブ自身のフィールドを `handle()` で適用するなど、別の方法で明示的な遅延を与えてください。

### Suprnovaが異なる設計を選んだ理由

Laravelの `$job->delay` はインスタンスプロパティであり、ディスパッチごとに設定されます（`SendDigest::dispatch($user)->delay(60)`）。そのため同じクラスの2つのディスパッチが異なる遅延を持てます。ここでの `Job::delay()` は `Job::queue()` や `Job::max_tries()` のような、クラスレベルの既定値です。自身のデータから遅延を計算するディスパッチには `Queue::later` / `push_later` を使います。これはすでに宣言済みの既定値より優先されます。

### コミット後のディスパッチ

[`DB::transaction`](database.md#transactions)の内側でプッシュされたジョブは、そのトランザクションと競争しています。別のプロセスのワーカーがエンベロープをpopし、トランザクションがまだ開いたまま保持している行を探して失敗する - あるいはもっと悪いことに、トランザクションがロールバックし、ジョブがもう存在しないデータに対して走る - ということが起こり得ます。

そのジョブを、コミットを待つようオプトインさせてください:

```rust
use suprnova::{DB, FrameworkError, Job, Queue, async_trait};

#[derive(serde::Serialize, serde::Deserialize)]
struct SendReceipt {
    order_id: i64,
}

#[async_trait]
impl Job for SendReceipt {
    fn job_name() -> &'static str { "send-receipt" }
    fn after_commit() -> bool { true }

    async fn handle(self) -> Result<(), FrameworkError> {
        // これが走る時点で、注文の行が永続化されていることが保証されます。
        Ok(())
    }
}

DB::transaction(|_tx| {
    Box::pin(async move {
        let order = Order::create(suprnova::attrs! { total: 4999i64 }).await?;
        // ここではドライバーには何も届きません。
        Queue::push(SendReceipt { order_id: order.id }).await?;
        Ok::<(), FrameworkError>(())
    })
})
.await?;
// エンベロープは今キューにあります。そして、今になってはじめてです。
```

3つの規則が、すべてのケースをカバーします:

- **トランザクションの内側では、プッシュ全体がコミットを待ちます。** ドライバーへの書き込みだけではありません: エンベロープの構築も、`JobQueueing` イベントも、`JobQueued` イベントも、すべてコミットの時点で起こります。そのため、ロールバックが捨てることになるジョブについて、リスナーが知らされることは決してありません。
- **ロールバックはそれを捨てます。** プッシュは、単に起こりません。一意性のロックを取っていたなら、ロールバックがそのロックを返します。
- **トランザクションの外側では、プッシュは即座に起こります。** これが、このオプトインをジョブ型の上で宣言しても安全である理由です: ディスパッチする側は、自分が乗っているコードの経路がトランザクション的かどうかを知らなくてよいのです。

[セーブポイント](database.md#savepoints)のロールバックは、その内側に登録されたすべてにとってのロールバックとして数えられます。`tx.rollback_to("name")` は、`tx.savepoint("name")` 以降に先送りされたプッシュを捨て、それらが取ったロックをその場で解放するため、同じトランザクションの内側での再ディスパッチが、そのキーを再び獲得できます。セーブポイントより前に行われたプッシュは手つかずで、決してロールバックしなかったセーブポイントは、その内側に登録されたものをすべて保ちます。

ジョブ型ごとではなくディスパッチごとに指定するには、`EnvelopeOverrides::after_commit` を使ってください。`Some(true)` はLaravelの `afterCommit()` であり、`Queue::push_after_commit(job)` という短縮形があります。`Some(false)` はLaravelの `beforeCommit()` で、コミットが着地する前にワーカーから見えていなければならない、唯一のディスパッチのためのものです:

```rust
use suprnova::queue::{EnvelopeOverrides, Queue};

// 型がオプトインしていないジョブを先送りします。
Queue::push_after_commit(SendWelcomeEmail { user_id: 42 }).await?;

// ジョブ型がオプトインしていても、即座にプッシュします。
Queue::push_with(
    SendReceipt { order_id: 7 },
    EnvelopeOverrides { after_commit: Some(false), ..Default::default() },
)
.await?;
```

先送りされた `Queue::push` は、[`Job::delay()`](#ジョブが宣言する遅延)を、プッシュに対してではなくコミットに対して再解決します。遅延は「ディスパッチからこれだけ待つ」という意味であり、先送りされたジョブにとって、ディスパッチとは*コミットそのもの*だからです。明示的なタイムスタンプは、ある時点についての呼び出し側の意図であるため、`Queue::push_later`、`Queue::later`、`Queue::later_with` は、それぞれのタイムスタンプを先送りを通じてそのまま運びます。

`Queue::push_unique` は、1つの意図的な非対称性を伴って先送りします: 重複排除のロックは即座に取られるため、同じトランザクションの内側での、同じ一意idに対する2回目の `push_unique` は、やはり抑制され、やはり `Ok(false)` を報告します。待つのはエンベロープだけです。勝った側は、そのプッシュが保留中であっても `Ok(true)` を報告します。そのプッシュはこれから起こるからです。ロールバックは、それが取ったロックを所有者スコープで解放するため、`unique_for` の窓が、起こらなかったディスパッチによって塞がれることは決してありません - 拒否された `COMMIT` を含め、コミットが着地しないほかのどの終わり方でも同じです。この保証の唯一の境界はTTLそのものです: `unique_for` より長く開いたままのトランザクションは、そのロックが飛行中に失効して別のディスパッチに取られ得るため、重複排除が重要なら、いちばん長いトランザクションより余裕を持たせて `unique_for` を設定してください。`push_unique*` のファミリーは `EnvelopeOverrides` を取らないため、一意のプッシュが先送りされるかどうかを決めるのは `Job::after_commit()` だけです - それに対するプッシュごとの上書きはありません。

バッチとチェーンは、`Job::delay()` を参照しないのと同じように、先送りもしません: `Queue::batch()` と `Queue::chain()` は、それぞれのエンベロープを直接構築してプッシュします。バッチがコミットを待たなければならないのなら、`.dispatch()` の呼び出しを、トランザクションが返った後に走るように包んでください。

キューに入れられた[メール](mail.md#queueing)と[通知](notifications.md)も、先送りしません。それぞれが1つの共有されたジョブ型（`SendMailJob` / `SendNotificationJob`）に乗っており、`Mailable` や `Notification` に `ShouldQueueAfterCommit` 相当はまだないため、トランザクションの内側での `Mail::queue` や `Notify::queue` の呼び出しは、即座にドライバーへ到達します。それらは、トランザクションが返った後に送ってください。

`Queue::fake()` の下では、プッシュは先送りも含めて即座に記録されるため、テストは何もコミットせずにそれをアサートできます。これはLaravelの `Bus::fake` と一致し、トランザクション的なハンドラを1つ走らせて、そのディスパッチを同じ流れの中でアサートできるようにしているものです。

### Suprnovaが異なる設計を選んだ理由

`Queue::bulk` は単相です - すべての要素が1つの具体的な `J` を共有します - そのため、コミット後についての分割は、その呼び出しにとって全部か無かです。Laravelは異種の配列を、先送りする側と即座に送る側へ分割しますが、ここには分割すべきものがありません。

先送りはクロージャの形式に結び付いています。手動の [`DB::begin_transaction`](database.md#manual-form) の内側でのプッシュは、**即座に**起こります。手動モードはアンビエントなトランザクションをインストールせず、したがってコールバックをぶら下げるコミットを持たないからです。そこで先送りすれば、誰も走らせないコールバックをキューに積むことになり、黙って消えるディスパッチは、早すぎるディスパッチより悪いものです。ディスパッチがコミットを待たなければならないときは、`DB::transaction` に手を伸ばしてください。

Laravelはさらに、優先順位の連なりの最後のフォールバックとして、接続レベルの `after_commit` という設定キーも読みます。Suprnovaは、プッシュごとの上書きと、その次のジョブ自身の `Job::after_commit()` で止まります: ここでのキューの接続は、自身のディスパッチのポリシーを持ちません。

## ジョブの設定

実装ごとに振る舞いを調整するには、`Job` の関連関数をオーバーライドしてください。

```rust
use std::time::Duration;
use suprnova::queue::{BackoffSchedule, JobMiddleware};

#[async_trait]
impl Job for SendWelcomeEmail {
    fn job_name() -> &'static str { "SendWelcomeEmail" }

    async fn handle(self) -> Result<(), FrameworkError> { /* … */ Ok(()) }
    fn delay() -> Option<Duration> { None }                // デフォルト: 遅延なし

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
    fn unique_until_processing() -> bool { true }          // デフォルト: false（TTLが窓になる）
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
1. `Queue::push_with` / `Queue::later_with` に渡されたプッシュごとの上書き（[`EnvelopeOverrides` によるプッシュごとの上書き](#envelopeoverrides-によるプッシュごとの上書き)を参照）
2. `Queue::route` で登録されたルート
3. ジョブ自身の `Job::queue` / `Job::connection`
4. ドライバー / グローバルなデフォルト

あるフィールドに `None` を渡すと、その軸には触れません。そのため、ジョブのコネクションをルーティングしても、そのジョブがすでに宣言しているキューを乱すことはありません。

この2つの軸は、現在のところ異なる深さで動いています。**キュー**はエンドツーエンドで尊重されます - エンベロープに刻印され、ドライバーに保存され、`--queue` でフィルタされます。**コネクション**は、`JobQueueing` / `JobQueued` のライフサイクルイベントに運ばれるコネクション*名*を解決するだけで、これはリスナーやダッシュボードが目にするものです。1つのプロセスグローバルなドライバーが、それでもすべてのpushを受け取るため、ジョブのコネクションをルーティングしても、まだ別のドライバーを選ぶことはありません。今、コネクションを宣言することは、コネクションごとのドライバーが実現したときのための前方互換性であり、振る舞いを変えるものではありません。

続いて、それのためにワーカーを専用化します。

```bash
./app queue:work --queue=billing
./app queue:work --queue=exports,heavy
./app queue:work                       # 以前と同様、すべてのキューをドレインする
```

ルートを持たないジョブは `default` に属するため、`--queue=default` は、ルーティングされていない作業を見捨てるのではなく、それをドレインします。

### キュー全体を転送する

`Queue::route` はジョブの型でキー付けされます。あるプールを別のプールを通じてドレインしたいとき - キューを引退させる、バックログを吸収する、これから落とそうとしているプールから作業を移す - は、代わりにリダイレクトをキュー名でキー付けしてください:

```rust
// bootstrap::register()
use suprnova::Queue;

Queue::forward("default", "high");
Queue::forward_on("exports", "heavy", "redis");   // `redis` コネクションでのみ
```

`forward_on` のコネクションはゲートであり、このプロセスのコネクション名 - 設定していれば `Queue::set_connection_name`、していなければドライバー自身の名前 - と比較されます。ジョブの `Job::connection`、`Queue::route` のコネクション、プッシュごとの `EnvelopeOverrides` のコネクションと比較されるのではありません: それらは、ライフサイクルイベントが報告するものを名指しするのであり、ワーカーが自分の確保リストをゲートするために持っているのは、プロセス名だけだからです。リダイレクトの両側が、その1つの値でゲートされます。そのため、転送が、確保を動かさないままプッシュだけを動かすことは決してありません。

リダイレクトは両側に適用されます。それが、作業を取り残さないようにしているものです:

- **プッシュ側では**、ルーティングとジョブ自身の `Job::queue` が名前を決めた後 - そして、渡したのであればプッシュごとの `EnvelopeOverrides` のキューの後 - に、名前が書き換えられます。
- **pop側では**、`--queue=default` で起動したワーカーが `high` をドレインします。この半分がなければ、行き先のキューは、どのワーカーも確保しないジョブを集めてしまうことになります。

`--queue` をまったく付けずに起動したワーカーは、すでにすべてをドレインしているため、転送はそれにとって何も変えません。`default` を転送すると、キューを名指ししなかったジョブが捕まります。ルーティングされていないジョブは `default` に属するからです。

転送は1回の引き当てであって、決して連鎖ではありません。`a -> b` と `b -> c` が登録されているとき、`a` に解決されたプッシュは `b` に着地します。したがって、すでにある `a -> b` の上に `b -> a` を登録することは、ループではなく、筋の通ったプールの入れ替えです: `a` へのプッシュは変わらず `b` に着地し、`b` へのプッシュは今度は `a` に着地し、どちらの名前で起動したワーカーも、もう一方を確保します - 何も連鎖しないため、何も取り残されません。より多くのキュー名にまたがる、より長いローテーションも、独立した1ホップずつ、同じように解決されます。Laravelの `Queue::forward` にもサイクルの検査はありません。理由は同じで、そのリゾルバーも、これと同じ1回の引き当てだからです。キューを自分自身の名前へ転送することは恒等であり - リダイレクトはまったく起こりません - これが、すでに登録した転送を無効化する方法です。

動くのは、これから先のプッシュだけです。ソースのキューにすでに載っているエンベロープはそこに留まり、それらをドレインしていたワーカーは今や行き先を確保しているため、転送する前にソースのプールをドレインしてください。同じことが `queue:retry` にも当てはまります: 失敗したジョブは、それが失敗したキューへ再エンキューされます。

一時停止は、リダイレクトより前に、ワーカーが起動時に与えられた名前の上で評価されます。`Queue::pause(&connection, "default")` は、`default` が `high` へ転送されている間も、`--queue=default` で起動したワーカーを止めます。その逆も成り立ちます: 転送の*行き先*を一時停止しても - `Queue::pause(&connection, "high")` - `--queue=default` で起動したワーカーは止まりません。そのワーカーへは、書き換えられた後の名前ではなく、自身のソースの名前を通じて到達するからです。この遷移が発生させる `WorkerQueuePaused` イベントは、設定された名前である `queue: default` を運び、`high` を運ぶことは決してありません - Laravelも、同じ順序で評価し、同じように報告します。

検査の呼び出しは、意図的に転送されません: `Queue::pending_jobs(Some("default"))` は、`high` の上にあるものではなく、文字どおり `default` の上にあるものを一覧します。これが、今しがた転送したソースのキューに取り残されたバックログを見る方法です。Laravelはそこでも転送を解決します。下の分岐の注記を参照してください。

登録した転送は `Queue::forward_for("default")` で読み戻せます。行き先を `queue` に、コネクションのゲートを `connection` に返します。

### Suprnovaが異なる設計を選んだ理由

Laravelの `Queue::route(...)` はクラス文字列を取りますが、Suprnovaはジョブを型パラメータとして取るため、リネームまたは削除されたジョブは、サイレントに一致しなくなるルートではなく、コンパイルエラーになります。

より大きな分岐点は、ドライバーがフィルタできない場合に何が起きるかです。`QueueDriver::pop_from` は、尊重できないキューフィルタを、すべてをドレインすることへフォールバックする代わりに、**拒否します**。`billing` だけをドレインするよう指示されたワーカーが、静かにすべてのキューをドレインしてしまう場合、間違ったプールが間違ったジョブを消費するまでは、動作しているデプロイと見分けがつきません - そのため、設定ミスは最初のポーリングではっきりと表面化させられます。メモリドライバーとデータベースドライバーはネイティブにフィルタします。フィルタしないドライバー - 単一のストリームコンシューマーグループにはキューごとのストレージがないため、Redisドライバーがそれにあたります - は、誤解させるのではなく、エラーになります。

`Queue::forward` は、Laravelの `Queue::forward` のキューからキューへの半分を完全に移植しており、移植しているのはその半分だけです。Laravelの3番目の引数は、転送されたキューを別の*コネクション*へ移せます。そのキューマネージャーが、コネクション名ごとにドライバーを解決するからです。Suprnovaはプロセスグローバルなドライバーを1つしか持たず、コネクション名はライフサイクルイベントにラベルを付けるだけです。そのため `Queue::forward_on(from, to, connection)` は、コネクションを**ゲート**として - キュー名のリダイレクトが適用されるかどうかを決めるものとして - 扱い、行き先として扱うことは決してありません。同じ理由で、Laravelでは省略可能な `to` が、ここでは必須です: Laravelで `to` を省略することは「コネクションだけを移す」という意味であり、それはまさにSuprnovaが尊重できない軸であるため、`forward(from, None)` は、設定の変更を装ったno-opになってしまいます。

Laravelの検査の呼び出しは転送に追随します。`pendingJobs($queue)` とその兄弟たちが、プッシュとpopが通るのと同じ、ドライバーレベルの `getQueue()` を通るからです。対してSuprnovaの `Queue::pending_jobs` / `delayed_jobs` / `reserved_jobs` は、あなたが名指しした文字どおりのキューを報告します。プロセスグローバルなドライバーが1つしかない以上、文字どおりに見せることだけが、今しがた転送して手放したキューに残されたエンベロープ - この節が最初にドレインするよう言っているバックログ - を見る方法です。新しい作業がどこに着地しているかを見るには、行き先のキューを名前で尋ねてください。

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
| `UniqueJobSkipped` | `push_unique` が `unique_for` の期間内の重複を抑制したとき |
| `JobDebounced` | より新しいデバウンスのディスパッチに追い越されたエンベロープを、ワーカーが落としたとき |
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
| `QueuePaused` | `Queue::pause` が1つのキュー自身のスイッチを設定したとき |
| `QueueResumed` | `Queue::resume` が1つのキュー自身のスイッチを解除したとき |
| `QueuesPaused` | `Queue::pause_all` がグローバルスイッチを設定したとき |
| `QueuesResumed` | `Queue::resume_all` がグローバルスイッチを解除したとき |
| `WorkerQueuePaused` | 実行中のワーカーが、あるキューが停止されているのを初めて観測したとき |
| `WorkerQueueResumed` | 実行中のワーカーが、停止されていたキューが再び確保できるようになったのを見たとき |

通常の `Event::listen` APIで購読してください。イベントはベストエフォートです - リスナーがいない状態での `Event::dispatch` はno-opの `Ok(())` であるため、`Event::init()` のないデプロイのワーカーは何も代償を払いません。

`UniqueJobSkipped` は、ワーカー側ではなく*プッシュ側*で発火する唯一のイベントであり、失敗でないことを報告する唯一のイベントです。`job_name`、`unique_id`、`connection` を持ちます。重複排除の決定はエンベロープが存在する前に行われるため、報告するエンベロープIDはありません。プッシュはなお `Ok(false)` を返します。このイベントが、そうでなければ見えない抑制を可観測にします。

`QueuePaused` / `QueueResumed` / `QueuesPaused` / `QueuesResumed` も同じく、ワーカーループからではなく `Queue::pause` / `resume` / `pause_all` / `resume_all` 自体から発火します。これらにもエンベロープ識別情報はありません。完全な契約については下の「キューの停止」を参照してください。

`WorkerQueuePaused` / `WorkerQueueResumed` はワーカー側の対であり、*なぜ特定のワーカーが静かになったのか*を教えてくれるのはこれらです。遷移ごとに一度、ワーカーループの内側から発火し、そのワーカーがドレインしているコネクションと、キュー名を運びます - あるいは、フィルタなしのワーカーがグローバルな停止のもとでアイドルしていて、報告すべきキュー名を持たないときは、`None` を運びます。

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

`QueueDriver` トレイトは、`size` / `pending_size` / `reserved_size` / `delayed_size` / `clear` のデフォルトを宣言しています。`MemoryQueueDriver`、`DatabaseQueueDriver`、`RedisQueueDriver` は、いずれもこれらをネイティブに実装します。

### キューを検査する

件数は、どれだけキューに載っているかを教えてくれます。ときには実際のエンベロープを見る必要があります - 管理ダッシュボード、デバッグの作業、「正確に何が詰まっているのか」という問いです。`Queue::pending_jobs` / `delayed_jobs` / `reserved_jobs` は、サイズのカウンターが数えているのと同じ情報を、`InspectedJob` のDTOの一覧として返します:

```rust
use suprnova::queue::{InspectedJob, Queue};

let pending: Vec<InspectedJob> = Queue::pending_jobs(None).await?;
let billing_only: Vec<InspectedJob> = Queue::pending_jobs(Some("billing")).await?;
let delayed = Queue::delayed_jobs(None).await?;
let reserved = Queue::reserved_jobs(None).await?;

for job in &pending {
    println!(
        "{} attempts={} queue={:?} payload={}",
        job.name, job.attempts, job.queue, job.payload
    );
}
```

`InspectedJob` は `id`、`queue`、`name`、`attempts`、`payload`、`created_at` を運びます。`id` と `created_at` は `Option` です: databaseドライバーの一覧は、`envelope_json` のデコードに失敗した行も - `id: None` と `payload: {"unparseable": true}` として - 落とさずに報告し、ポイズンジョブを見ている人から隠しません。`Queue::fake()` の射影は、ディスパッチのタイムスタンプを `available_at` とは別に記録することが決してないため、そこでの `created_at` は常に `None` です。

memoryドライバーでは、`delayed_size()` は遅延ストアの長さを直接読みますが、`delayed_jobs()` と `pending_jobs()` は、`available_at` がすでに過ぎているエントリを先に昇格させます。ジョブが実行可能になってから、バックグラウンドの回収係の次の50msのティックまでの狭い窓では、`delayed_jobs()` がすでに `pending_jobs()` へ昇格させたジョブを `delayed_size()` がまだ数えていることがあります - 一覧のほうが、より現在に近い眺めです。そこでの食い違いは、バグではなく想定どおりです。

可視性タイムアウトが失効した予約は、`pop` かバックグラウンドの回収係がそれを再要求するまで、`reserved_jobs()` に現れ続けます。再要求するのはその2つだけであり、再要求こそが試行を1回消費するものであるため、一覧を取る呼び出しは、どれだけ呼んでもジョブの試行回数を変えることはありません。

#### Suprnovaが異なる設計を選んだ理由

- **一覧ごとに1組ではなく、`Option<&str>` を取る1つのメソッドです。** Laravelは `pendingJobs($queue)` と並べて、別の `allPendingJobs()` を出荷しています。ここでは `queue: None` が、その2つを1回の呼び出しへ畳み込みます。`delayedJobs`/`allDelayedJobs` と `reservedJobs`/`allReservedJobs` も同じ形です。
- **トレイトのデフォルトは、空のコレクションではなく誠実な `Err` です。** LaravelのBeanstalkdとSQSのドライバーは、明らかにジョブがあるキューについてさえ、これらのメソッドから `[]` を返します - サードパーティのドライバーの作者が気づかずに真似しかねない、不作為の嘘です。検査を実装していないSuprnovaのドライバーは、そうだと言います。`sync` と `null` は `Ok(vec![])` で上書きします。それらにとって「一覧にすべきものは決してない」ことは、未実装のメソッドではなく文字どおりの真実だからです。
- **Redisの `reserved_jobs` は、コンシューマーごとです。** このドライバーが知っているのは、自身がプロセス内で手渡した予約だけです。別のコンシューマーの飛行中のエントリは、この呼び出しからではなく、Redis自身の `XPENDING` を通じてしか見えません。
- **Redisの `pending_jobs` は、「このグループのどのコンシューマーにも一度も配信されていない」という意味です。** これは、ストリーム全体ではなく `XRANGE (<last-delivered-id> +` - グループの配信カーソル（`XINFO GROUPS`）より後のすべて - を走査します。`ack` はエントリを `XACK` するだけであり（このドライバーはストリームを `XDEL`/`XTRIM` することが決してありません）、そのため、1つのコンシューマーのメモリ内の予約を除外しただけの走査は、ackされたすべてのジョブを永遠にpendingとして報告してしまうからです。解放されたジョブやnackされたジョブは、カーソルより上の新しいidの下で再発行されるため、そのリトライが有効になれば再び現れます。`pending_size` と同じ「上限」の位置づけです: カーソルは一度だけ読まれるため、その読み取りと走査のあいだに、並行する `pop` がエントリを要求し得ます。実際には、走っているコンシューマーのバックグラウンドの先読みタスクが、プッシュされたばかりのエントリを、アプリケーションが `pop` を呼ぶよりずっと前、プッシュから数ミリ秒のうちに要求する傾向があります - そのため `pending_jobs` は、たいていの場合「まだ誰も明示的にpopしていない任意のエンベロープ」ではなく、そのストリームのコンシューマーが誰も能動的にポーリングしていないあいだにプッシュされた作業を映します。

## ワーカーの再起動シグナル

`php artisan queue:restart` に相当するのは次です。

```rust
Queue::restart().await?;
```

この信号は、ミリ秒単位のタイムスタンプとして `Cache` の中に存在します。ワーカーはループごとに一度ポーリングし、タイムスタンプが自分の開始時刻より新しければきれいに終了します。新しいワーカーが、前のワーカーが止まった場所から引き継げるよう、スーパーバイザー（systemd、Kubernetes、`supervisor` モジュールなど）と組み合わせてください。

## キューの停止

`php artisan queue:pause` / `queue:resume` に相当するものは次のとおりです:

```rust
Queue::pause(&connection, "billing").await?;
Queue::resume(&connection, "billing").await?;
Queue::pause_all().await?;
Queue::resume_all().await?;
```

またはCLIから:

```bash
./app queue:pause billing
./app queue:pause --all
./app queue:resume billing
./app queue:resume --all      # alias: queue:continue
```

停止されたワーカーは、すでにpopしたものを完了します - 停止は実行中のジョブを中断しません - その後、再開されるまで新しい作業の取得を止めます。`pause_all` / `resume_all` はグローバルスイッチです。名前付きキューの停止（または再開）は、そのキューだけに影響します。**`resume_all` はキューごとの停止を解除しません** - 個別に停止したキューはグローバル再開後も停止したままです。Laravelと同じ動作であり、`Queue::resume(&connection, "billing")` で明示的に解除してください。

停止されたワーカーも、そうであることを口に出します。`queue:work` は、遷移ごとに1行を出力します:

```text
  2026-08-25 14:03:11 Queue billing PAUSED
  2026-08-25 14:07:44 Queue billing RESUMED
```

`--queue` なしで起動したワーカーには報告すべきキュー名がないため、グローバルな停止では代わりに `All queues PAUSED` を出力します。どちらの行も `WorkerQueuePaused` / `WorkerQueueResumed` のイベントから来ているので、あなた自身でそれらを購読し、あなたのアラートが置かれている場所へルーティングできます。

両方のシグナルは、上にある再起動シグナルの隣の `Cache` に存在します:

| キー | 意味 |
| --- | --- |
| `suprnova:queues:paused` | `pause_all` が設定するグローバルスイッチ |
| `suprnova:queue:paused:{connection}:{queue}` | `pause` が設定する1つのキューのスイッチ |

`Queue::is_paused(&connection, "billing").await?`（どちらかのキーが設定されていればtrue）で状態を確認するか、`Queue::paused_queues(&connection, &queues).await?`（`queues` のうち現在停止しているもの）を使います。

### キューごとの停止には名前付き `--queue` が必要

`--queue=billing,exports` で起動したワーカーはこの2つのキューからだけ取得するため、`billing` を停止すると、停止が続く間はリストが `exports` に狭まります。`--queue` なしで起動したワーカーは、ドライバーが保持するすべてのキューを排出します。そのワーカーに対して「`billing` だけを停止」と尋ねる方法はありません。`QueueDriver::pop_from` はどのキュー名が存在するかを返さないため、キューごとの停止キーに対して確認するものがないからです。`pause_all` はフィルターされていないワーカーを完全に止めますが、名前付きのキューごとの停止は、そのワーカーのキューも名前付きになって初めて有効になります。

### 停止ポーリングの無効化

`QUEUE_PAUSABLE=false` を設定すると、そのプロセスのすべてのワーカーが停止シグナルを完全に無視し、ループごとの追加のキャッシュ読み取りコストもなくなります。`queue:pause`（`queue:resume` ではない）も実行を拒否して非ゼロで終了するため、停止を無効にした運用者は、何も起きない停止を静かに発行するのではなく、すぐに知ることができます。Laravelの `Worker::$pausable` に対応します。

### Suprnovaが異なる設計を選んだ理由

到達不能なキャッシュは**フェイルオープン**です。停止キーを読み取れないワーカーは「停止していない」として動作し、排出を続けます。これは上のワーカー再起動シグナルがすでに使うフェイルオープン契約と同じです。一時的なキャッシュ停止によってワーカーフリートが「停止を無視する」状態になるのは許容しますが、「すべてのワーカーが静かに凍結する」状態にはしません。停止状態は明示的なオプトインシグナルであり、その利用不能が隠れたキルスイッチになってはいけないからです。

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

フェイクのガードは、プロセス全体のmutexを介して並行テストを直列化します。pushごとに `(payload, available_at, overrides)` をキャプチャし、`Drop` 時にクリアします。`overrides` フィールドは `push_with` / `later_with` 以外のすべてのエントリポイントでは `EnvelopeOverrides::default()` です。これと、そのアサーションである `assert_pushed_on_queue` / `assert_pushed_on_connection`、`pushed_with_overrides` については[モックとフェイク](mocking.md#queue---queuetestinginstall_fake)を参照してください。フェイクモードでは、`push_unique` は常にpushを新規として記録します - ドライバーが配線されていないときは、重複排除は無関係です。

デバウンスされたプッシュも同じように振る舞います: フェイクはキャッシュへ何も書かないため、ウィンドウは開始されず、記録される `available_at` はデバウンスの遅延を運びません。`assert_pushed_later` からは、遅延なしのものとして見えます。それでもフェイクが捕まえるのは、`debounce_for` と `unique_id` の両方を宣言したジョブです - この組み合わせは環境が何であれ成り立たないため、`Queue::fake()` の下でも、本番環境とまったく同じようにプッシュがエラーを返します。

## べき等性は、ワーカーとあなたの間の契約

Redisに支えられたキューのドライバーは、`nack` をアトミックにできません - `XADD` と `XACK` は別個のコマンドです。その間でのクラッシュは、`XAUTOCLAIM` を介してメッセージを再配信します。インメモリとデータベースのドライバーは、試行ごとに厳密に1回ですが、ワーカーループはドライバーを区別しないため、**本番環境のデプロイにおけるすべてのジョブハンドラは、べき等でなければなりません**。

典型的なコマンド風のジョブでは、安定した操作ごとのキー（エンティティid、呼び出し元が渡すリクエストidなど）をキーとして、ハンドラの本体を[`Idempotency::once`](idempotency.md)や[`Idempotency::commit_on_success`](idempotency.md)でラップしてください。リトライが、再実行をスキップするのではなく*元の*結果を返さなければならない場合は、成功した値を記録し、後の配信でそれを再生する `Idempotency::remember` を使ってください。

## 次のステップ

- [バス](bus.md) - 型付きの結果を伴う同期的なディスパッチャー
- [イベント](events.md) - pub/subのファンアウト
- [べき等性](idempotency.md) - 少なくとも1回の配信のために、ハンドラが守るべき契約
- [キャッシュ](cache.md) - `push_unique`、`WithoutOverlapping`、`RateLimited` を支える
- [モックとフェイク](mocking.md) - `Queue::fake` を含む、あらゆるフェイクのガード
