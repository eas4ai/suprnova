# イベント

イベントは、Suprnovaの型付きのプロセス内pub/subです。コントローラーが `UserRegistered { user_id }` を発火させます。1つのリスナーがユーザーにメールを送り、別のリスナーが監査の行を書き込み、3つ目がブロードキャストを発行します。3つとも同じペイロードを見て、登録順に実行され、互いについてのコンパイル時の知識を持ちません。

ユーザー向けの表面は `EventFacade` 構造体です（`suprnova::EventFacade` として再エクスポートされています）。このクレートは、`Event` *トレイト*も `suprnova::Event` として再エクスポートしています - Laravelのファサードと同じ名前ですが、Rustでは、このトレイトは、あらゆるペイロードが実装する型付きの契約です。ファサードの背後には、単一のプロセスグローバルな `EventDispatcher`（`OnceLock` に保持されています）があります: 登録済みのリスナーは、それを登録したリクエストより長生きし、ディスパッチはインラインで実行されるか、上限付きでリトライするタスクセットへspawnされます。

## 基本

```rust
use suprnova::{EventFacade, Event, Listener, FrameworkError, async_trait};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct UserRegistered {
    pub user_id: i64,
}

impl Event for UserRegistered {
    fn event_name() -> &'static str {
        "UserRegistered"
    }
}

pub struct SendWelcomeEmail;

#[async_trait]
impl Listener<UserRegistered> for SendWelcomeEmail {
    async fn handle(&self, e: &UserRegistered) -> Result<(), FrameworkError> {
        // メールを送信する…
        let _ = e.user_id;
        Ok(())
    }
}

// bootstrap.rs の中で:
EventFacade::listen::<UserRegistered, SendWelcomeEmail>(Arc::new(SendWelcomeEmail)).await;

// コントローラーの中で:
EventFacade::dispatch(UserRegistered { user_id: 42 }).await?;
```

`Event` は `Send + Sync + Clone + 'static + Debug` を要求します。これは、ペイロードがタスクの境界を越えられるようにし（キューに入れられたリスナー）、ディスパッチャーがそれをログに記録できるようにするためです。`Listener<E>` は `Send + Sync + 'static` であり、登録の呼び出しより長生きできます。`#[derive(Event)]` はありません - このトレイトは2つのメソッド（`event_name` と、デフォルト実装のある `queued`）しか持たないため、手書きの実装は2行で済みます。

## ディスパッチのモード

| メソッド | セマンティクス |
|---|---|
| `EventFacade::dispatch(event)` | 同期的、フェイルファスト - 最初のリスナーの `Err` がチェーンを中断する |
| `EventFacade::dispatch_best_effort(event)` | 同期的、全実行 - すべてのリスナーが実行された後に、最初の `Err` を返す |
| `Event::queued() = true` のときの `EventFacade::dispatch(event)` | 各リスナーが、上限付きでリトライするタスクとしてspawnされる。呼び出しはspawn後に戻る |

下流の副作用が、成功した上流を確実に観測しなければならないときは、`dispatch`（フェイルファスト）を使ってください - ほとんどのモデルのライフサイクルフックがこれに該当し、保存を拒否権で止めるオブザーバーがショートサーキットできます。1つの失敗したリスナーが残りを黙らせるべきでないファンアウトには、`dispatch_best_effort` を使ってください - ほとんどの可観測性のイベントがこれに該当します。

キューに入れられた配信をオプトインするには、トレイトのメソッドをオーバーライドしてください:

```rust
impl Event for ExpensiveAuditTrail {
    fn event_name() -> &'static str { "ExpensiveAuditTrail" }
    fn queued() -> bool { true }
}
```

キューに入れられたリスナーは、プロセス全体のセマフォによって上限が定められます。デフォルトの上限は256の並行タスクです。ディスパッチャーごとに `EventDispatcher::with_concurrency(n)` で、あるいはグローバルに `EVENT_MAX_CONCURRENCY` 環境変数で上書きしてください。各タスクは、諦める前に、100ms→2sのジッター付きバックオフで最大3回まで試行をリトライします - これらは、永続的なキューの分単位のスケジュールではなく、プロセス内での一時的な障害に対するリトライです。

## Subscriber - 関連する登録をまとめる

複数のリスナーが1つの機能に属するときは、`Subscriber` がそれらを1つの単位として登録します。Laravelの `EventServiceProvider` のサブスクライバーパターンを反映しています。

```rust
use suprnova::{EventFacade, EventDispatcher, Subscriber, async_trait};
use std::sync::Arc;

pub struct UserEventSubscriber {
    db: Arc<crate::Db>,
}

#[async_trait]
impl Subscriber for UserEventSubscriber {
    async fn subscribe(self: Arc<Self>, d: &EventDispatcher) {
        let db = self.db.clone();
        d.listen::<UserRegistered, _>(Arc::new(SendWelcomeEmail::new(db.clone()))).await;
        d.listen::<UserDeleted, _>(Arc::new(CleanupUserData::new(db.clone()))).await;
        d.listen::<UserPromoted, _>(Arc::new(NotifyAdmins::new(db))).await;
    }
}

// bootstrap.rs の中で - リスナーごとに3行の代わりに、サブスクライバーごとに1行:
EventFacade::subscribe(Arc::new(UserEventSubscriber { db: db.clone() })).await;
```

`subscribe` は `Arc<S>` を取るため、サブスクライバーと状態を共有する必要のあるリスナーは、その `Arc` をクローンしてキャプチャできます。

## リスナーの検査と削除

```rust
if EventFacade::has_listeners::<UserRegistered>() {
    EventFacade::dispatch(UserRegistered { user_id: 42 }).await?;
}

let removed: usize = EventFacade::forget::<UserRegistered>();
```

`has_listeners::<E>()` は、Laravelの `Event::hasListeners($eventName)` を反映しています。`forget::<E>()` は、そのイベント型に登録されているすべてのリスナーを捨て、削除した数を返します。本番のコードが `forget` を必要とすることはほとんどありません - リスナーの登録は通常、起動時に一度だけだからです - ですが、ホットスワップやテストコードはこれに手を伸ばします。

どちらのメソッドも、リスナーレジストリのロックがポイズニングされている場合、安全なデフォルト（それぞれ `false` と `0`）を返し、その失敗が観測可能であるように `tracing::error!` を記録します。

## pushとflush

`push` は、発火させることなく、イベントごとの名前のバケットにイベントをキャプチャします。`flush::<E>()` は、そのバケットをドレインし、キャプチャされた順にすべてをディスパッチします。Laravelの `Event::push` / `Event::flush` のペアを反映しています。

```rust
// 2つのフェーズで作業を行うハンドラの中で:
EventFacade::push(UserRegistered { user_id: 42 }).await;
// … レンダリング、バリデーション、その他の作業 …
EventFacade::flush::<UserRegistered>().await?;
```

pushされたイベントは `defer` のスコープを無視します - それらはすでに明示的に遅延されているからです。`forget_pushed()` は、ディスパッチすることなく、pushされたすべてのイベントを捨て、捨てた数を返します。`Event::forgetPushed()` を反映しています。

## defer - コールバックの内側のすべてのディスパッチをバッファする

`defer(only, async { … })` は、タスクローカルなバッファをスコープに入れて、コールバックを実行します。コールバックの内側で行われるすべての `dispatch` / `dispatch_best_effort` の呼び出しはキャプチャされ、コールバックが戻った後に再生されます。Laravelの `Event::defer($callback, ?$events)` を反映しています。

```rust
let ((), flush_err) = EventFacade::defer::<_, ()>(None, async {
    do_work_part_one().await?;
    EventFacade::dispatch(WorkStarted).await?; // バッファされる
    do_work_part_two().await?;
    EventFacade::dispatch(WorkFinished).await?; // バッファされる
    Ok(())
})
.await?;
// この時点で、WorkStartedとWorkFinishedは両方、順番に発火している。
// `flush_err` は、再生からの最初のディスパッチエラーを運ぶ（あれば）。
```

それらのイベント名だけを遅延させるには、`Some(&["EventOne", "EventTwo"])` を渡してください。他のすべては、通常どおりインラインでディスパッチされます。コールバックのエラーはショートサーキットします - バッファされたイベントは捨てられ、エラーが伝播します。

deferのバッファはTokioタスクごとであるため、2つの並行する `defer` の呼び出しは、互いの状態を踏みつぶし合いません。

## キューに入れられたリスナー - プロセス内 vs 永続

2つの異なる「キューに入れられた」層があり、命名が重要です:

| 必要なもの | 手を伸ばすもの |
|---|---|
| リスナーはタスクの外で走るべきで、クラッシュ時に失われても構わない | イベントトレイト上の `Event::queued() = true` |
| リスナーの作業が、クラッシュ + 再起動を確実に生き延びなければならない | `QueuedListener<E, J>`（イベント → 永続的なジョブを橋渡しする） |

`Event::queued() = true` は、ディスパッチャーに、各リスナーをそれ専用のTokioタスクとしてspawnさせます。プロセスのセマフォによって上限が定められ、上限付きのリトライ（3回の試行、ジッター付きバックオフ）を伴います。この作業はこのプロセス上で走ります。クラッシュは、実行中のリスナーを失います。[シャットダウン時のドレイン](#シャットダウン時のドレイン)は、期限まで、実行中のタスクを待ちます。

`QueuedListener<E, J>` は、各イベントから[`Job`](queues.md)を構築し、それを永続的なキューへ投入する、既製のリスナーです。イベントはそれでも同期的に発火します。リスナーはenqueueするだけです - これは速いため、リクエストのレイテンシは低いままです。ジョブ自体は、キューが永続的であるため、クラッシュを生き延びます。

```rust
use suprnova::{EventFacade, QueuedListener};
use std::sync::Arc;

EventFacade::listen::<UserRegistered, _>(Arc::new(
    QueuedListener::<UserRegistered, SendWelcomeEmailJob>::new(|e| SendWelcomeEmailJob {
        user_id: e.user_id,
    }),
))
.await;
```

`QueuedListener` は、イベントが通常の同期的なイベントであることだけを必要とします - 永続性は、ディスパッチャーではなく、キューの中に存在します。

## シャットダウン時のドレイン

キューに入れられたプロセス内のリスナーは、ディスパッチャーが追跡する `JoinSet` へspawnされます。サーバーのグレースフルシャットダウンのシーケンスは、それらを待つために `EventFacade::drain_queued(timeout)` を呼び出します:

```rust
let still_running = EventFacade::drain_queued(Duration::from_secs(30)).await;
if still_running > 0 {
    tracing::warn!(still_running, "queued listeners abandoned at shutdown");
}
```

ドレインは、期限が経過した時点でまだ実行中だった数を返します（`0` = 完全にドレインされた）。期限を過ぎた残りのものは中断されるため、シャットダウンがハングすることはありません。

## イベントをブロードキャストへ橋渡しする

`EventFacade::broadcast::<E>(hub)` は、ディスパッチされたイベントから `BroadcastHub` への、1行の橋渡しを配線します。`Broadcastable` と `Event` を実装する任意の型は、この方法でブロードキャストできます。リスナーは型付きのペイロードを受け取り、名前付きのチャネルの購読者は、ブロードキャストのエンベロープを受け取ります。

```rust
use suprnova::EventFacade;
use std::sync::Arc;

let hub: Arc<dyn suprnova::BroadcastHub> = Arc::new(broadcast_hub);
EventFacade::broadcast::<OrderShipped>(hub).await;

// それ以降のあらゆるディスパッチも、OrderShipped::broadcast_on() が
// 宣言するチャネルへ発行される:
EventFacade::dispatch(OrderShipped { order_id: 42, user_id: 99 }).await?;
```

チャネルのモデル（パブリック / プライベート / プレゼンス）と `Broadcastable` トレイトについては、[ブロードキャスト](broadcasting.md)を参照してください。

## 組み込みのイベント

フレームワークは、自分自身のサブシステムから、固定されたイベントの集合をディスパッチします。リスナーを登録することでオプトインします。リスナーが登録されていなければ、イベントはno-opです。

| サブシステム | イベント | ディスパッチ元 |
|---|---|---|
| エラーハンドリング | `ErrorOccurred` | すべての5xxレスポンス（返された `FrameworkError`、または回復されたパニック） |
| 認証（ガード） | `Auth\\Attempting`、`Auth\\Authenticated`、`Auth\\Login`、`Auth\\Logout`、`Auth\\Failed` | `StatefulGuard::attempt` / `login` / `logout` / `once` |
| 認証フロー | `EmailVerified`、`PasswordResetLinkSent`、`PasswordResetCompleted`、`AccountLocked`、`AccountUnlocked`、`TwoFactorEnrolled`、`TwoFactorChallenged`、`TwoFactorChallengeFailed`、`TwoFactorDisabled` | `auth_flows::{EmailVerification, PasswordReset, BruteForce, TwoFactor}` |
| データベース | `Database\\ConnectionEstablished`、`Database\\QueryExecuted`、`Database\\TransactionBeginning`、`Database\\TransactionCommitted`、`Database\\TransactionRolledBack`、`Database\\DatabaseBusy` | `DbConnection::connect`、`ExecutorChoice` ヘルパー、`DB::transaction` |
| メール | `Suprnova\\Mail\\MessageSending`、`Suprnova\\Mail\\MessageSent` | `MailBuilder::send` のトランスポートの前後 |
| 通知 | `Suprnova::Notifications::Sending`、`Suprnova::Notifications::Sent`、`Suprnova::Notifications::Failed` | 各チャネルの配信 |
| キュー（ワーカー） | `queue::JobQueueing`、`JobQueued`、`JobProcessing`、`JobProcessed`、`JobAttempted`、`JobExceptionOccurred`、`JobFailed`、`JobReleased`、`JobReleasedAfterException`、`JobTimedOut`、`Looping`、`WorkerStarting`、`WorkerStopping`、`WorkerInterrupted`、`UniqueJobSkipped`、`QueuePaused`、`QueueResumed`、`QueuesPaused`、`QueuesResumed` | `Queue::push` / `Queue::push_unique` / `run_worker` / `Queue::pause` / `resume` / `pause_all` / `resume_all` |
| フィーチャー | `FeatureUpdated`、`FeatureDeleted` | `features::admin` のCRUD |
| Eloquent（モデルごと） | 16個のライフサイクルイベント - `Retrieved`、`Saving`、`Saved`、`Creating`、`Created`、`Updating`、`Updated`、`Deleting`、`Deleted`、`Restoring`、`Restored`、`ForceDeleting`、`ForceDeleted`、`Replicating`、`Pruning`、`Pruned` - 各モデルの `events::` サブモジュールの下で発される | `#[suprnova::model]` マクロが、これらをsave/update/deleteに配線する |

`ErrorOccurred` は、5xxの例外をSentry、Datadog、Slackなどへ出荷するための専用のフックです。このディスパッチはベストエフォートでspawnされるため、壊れたSentryのリスナーが残りを黙らせることはなく、レスポンスの変換がそれでブロックされることも決してありません。パニックリカバリと変換の契約の全体像については、[エラー モデル](error-model.md)を参照してください。

モデルのライフサイクルイベントは、フェイルファストに発火します: `EventResult::Cancel` を返す（`CancellableListener` トレイトを介した）`Saving` のリスナーは、保存を中断させます。[Eloquentのオブザーバーとライフサイクルイベント](eloquent.md)を参照してください。

## DB::listen - クエリを観測する

クエリごとの可観測性のためには、ディスパッチャーを介して型付きの `Listener<QueryExecuted>` を登録することもできますが、より一般的には、Laravelの `DB::listen(function ($q) { ... })` のシグネチャを反映した `DB::listen` のコールバックを登録します:

```rust
use suprnova::DB;
use std::sync::Arc;

DB::listen(Arc::new(|q| {
    tracing::debug!(
        sql = %q.sql,
        time_ms = q.time.as_millis(),
        connection = %q.connection_name,
        "query"
    );
}));
```

このコールバックは、SQL、バインディング、実時間の所要時間、コネクション名、読み取り/書き込みの分類、そして最終的な `Result`（失敗したクエリも観測可能であるように）を運ぶ `QueryExecuted` を受け取ります。`QueryExecuted::to_raw_sql()` は、ログの便宜のためにバインディングをインライン化します - デバッグ形式であり、SQLセーフでは**ありません**。

2つの、再入とコストに関する保証です:

- **再入のガード。** 自分自身がクエリを発行するリスナーは、その入れ子になったクエリから `QueryExecuted` を再発火させることはありません - ディスパッチャーは、リスナーが実行されている間、タスクローカルなフラグを立て、エグゼキューターはそのスコープの内側で発行をスキップします。DBへログを書き込むリスナーがループすることはありません。
- **誰もリスニングしていないときのオーバーヘッドはゼロ。** エグゼキューターは、イベントのペイロードを構築する前に、統合された `query_observation_active()`（何らかの直接のリスナー、何らかの登録済みの `Listener<QueryExecuted>`、あるいはクエリログの有効化のいずれか）をチェックします。3つすべてがオフのとき、発行の経路全体がショートサーキットされます。

## テスト - `EventFacade::fake()`

`EventFacade::fake()` は、グローバルなディスパッチャーをレコーダーへ差し替えます。ディスパッチされたイベントは、リスナーを実行する代わりに、その記録へ入ります。このフェイクは、ガードの生存期間の間、プロセス全体のシリアライザーを保持するため、これを使う並行する `#[tokio::test]` は1つずつ実行されます - テストは、もはや自分専用の `serial_test` のmutexを必要としません。

```rust
use suprnova::events::{
    EventFacade, assert_dispatched, assert_dispatched_once, assert_dispatched_times,
    assert_nothing_dispatched, has_dispatched, dispatched, dispatched_events,
};

#[tokio::test]
async fn registration_dispatches_welcome_event() {
    let _guard = EventFacade::fake();

    register_user("ada@example.com").await.unwrap();

    assert_dispatched_once::<UserRegistered>();
    assert_dispatched::<UserRegistered>(|e| e.email == "ada@example.com");
}
```

| ヘルパー | アサートすること |
|---|---|
| `assert_dispatched::<E>(pred)` | `pred` に一致する `E` が少なくとも1つディスパッチされた |
| `assert_dispatched_once::<E>()` | `E` がちょうど1つディスパッチされた |
| `assert_dispatched_times::<E>(n)` | `E` がちょうど `n` 個ディスパッチされた |
| `assert_not_dispatched::<E>(pred)` | `pred` に一致する `E` がディスパッチされなかった |
| `assert_nothing_dispatched()` | どの型のイベントもディスパッチされ**なかった** |
| `assert_listening::<E, L>()` | リスナー `L` が `E` に対して登録されていた |
| `has_dispatched::<E>()` | bool: 何らかの `E` が記録されている |
| `dispatched::<E>(pred)` | 一致するイベントの `Vec<E>` のクローン |
| `dispatched_count::<E>(pred)` | 一致するイベントの数 |
| `dispatched_events()` | すべてのディスパッチの `HashMap<&'static str, usize>` |

### 選択的なフェイク

```rust
// これらのイベントだけをフェイクする。他のすべては通常どおりディスパッチされる。
let _guard = EventFacade::fake_only(&["UserRegistered", "UserDeleted"]);

// これら以外のすべてのイベントをフェイクする。
let _guard = EventFacade::fake_except(&["TelemetryEvent"]);
```

Laravelの `Event::fake([…])` と `EventFake::except($events)` を反映しています。

### Mute - 記録せずにイベントを捨てる

`EventFacade::muted(async { … })` は、タスクローカルな「サイレントディスパッチャー」フラグを立てて、コールバックを実行します。その内側でディスパッチされるすべてのイベントは、記録されることも、リスナーを呼び出すこともなく、捨てられます。Laravelの `NullDispatcher` に相当するSuprnovaの概念で、コールバックにスコープされています。

```rust
EventFacade::muted(async {
    // リスナーは発火せず、イベントも記録されない。
    run_bulk_import().await;
})
.await;
```

`fake()` とは異なり、`muted` はプロセスのシリアライザーを取得**しません** - 2つのmutedスコープは並行に実行できます。

### `assert_listening` - リスナーが配線されていることを検証する

イベントを発火させずに、bootstrapの配線をテストするために使ってください:

```rust
#[tokio::test]
async fn bootstrap_wires_welcome_listener() {
    let _guard = EventFacade::fake();
    bootstrap::register_listeners().await;
    suprnova::events::assert_listening::<UserRegistered, SendWelcomeEmail>();
}
```

このフェイクは、ディスパッチャーの `listen` メソッドを介して登録を観測します。そのため、登録はフェイクのスコープの**内側**で起きなければなりません - `EventFacade::fake()` の前に登録されたリスナーは、`assert_listening` からは見え**ません**。

## Laravel 対応リファレンス

型付きのRustの等価物を持つ、あらゆるLaravel 13の `Event` ファサードと `EventFake` のメソッドは、最も近い名前の下で出荷されています。型付きのRustに合わないLaravelが公開しているメソッドは、短い注記とともに省略されています。

| Laravel | Suprnova |
|---|---|
| `Event::dispatch($event)` | `EventFacade::dispatch(event).await` |
| `Event::dispatch($event)`（halt引数） | `dispatch` を使う（`Err` でフェイルファスト） |
| `Event::until($event)` | `dispatch`（型付き: 最初の `Err` が停止させる） |
| `Event::listen($event, $listener)` | `EventFacade::listen::<E, L>(Arc::new(L))` |
| `Event::hasListeners($name)` | `EventFacade::has_listeners::<E>()` |
| `Event::forget($event)` | `EventFacade::forget::<E>()` |
| `Event::push($event)` | `EventFacade::push(event).await` |
| `Event::flush($event)` | `EventFacade::flush::<E>().await` |
| `Event::forgetPushed()` | `EventFacade::forget_pushed().await` |
| `Event::defer($callback, ?$events)` | `EventFacade::defer(only, async {…}).await` |
| `Event::subscribe($subscriber)` | `EventFacade::subscribe(Arc::new(S)).await` |
| `Event::fake()` | `EventFacade::fake()`（ガード） |
| `Event::fake([$names])` | `EventFacade::fake_only(&["…"])` |
| `EventFake::except($names)` | `EventFacade::fake_except(&["…"])` |
| `EventFake::assertDispatched` | `assert_dispatched` |
| `EventFake::assertDispatchedOnce` | `assert_dispatched_once` |
| `EventFake::assertDispatchedTimes` | `assert_dispatched_times` |
| `EventFake::assertNotDispatched` | `assert_not_dispatched` |
| `EventFake::assertNothingDispatched` | `assert_nothing_dispatched` |
| `EventFake::assertListening` | `assert_listening` |
| `EventFake::hasDispatched` | `has_dispatched` |
| `EventFake::dispatched` | `dispatched`（`Vec<E>` を返す） |
| `EventFake::dispatchedEvents` | `dispatched_events`（名前 → 件数のマップ） |
| `NullDispatcher` | `EventFacade::muted(async {…}).await` |
| `Event::wildcards`（`User.*` パターン） | 出荷されていない - 型付きのリスナー、あるいはモデルごとのライフサイクルフックのための `Observer<M>` トレイトを使う |
| `Event::subscribe`（文字列のサブスクライバー） | 型付きの `Subscriber` トレイトを使う |
| `DB::listen(function ($q) {…})` | `DB::listen(Arc::new(|q| {…}))` - 同じ形で、`&QueryExecuted` を取る |

### Suprnovaが異なる設計を選んだ理由

Laravelのディスパッチャーは、PHPの文字列型ランタイムに頼っています: イベントは文字列として渡されるクラス名であり、リスナーはコンテナを介してルックアップされるクラス名であり、`Event::listen('User.*', ...)` が動作するのは、クラス名の文字列に対するワイルドカードがPHPでは意味を持つからです。Rustでは、「このリスナーは `User.*` を処理する」の等価物は、「このリスナーは `E: UserEvent` に対してジェネリックである」です - 文字列マッチではなく、トレイトです。そのため、Suprnovaは型システムを優先してワイルドカードを捨てており、その結果、壊れたリファクタリングは、実行時の誤ったルーティングではなく、コンパイルエラーになります。

もう1つの分岐点は `defer` です: Laravelのdeferは、遅延のスコープを区切るために、リクエストごとにプロセスを立てるモデルに頼っています。Suprnovaは、1つのプロセスの中で多くの並行するリクエストを処理するため、遅延のバッファはタスクローカルです。2つの並行する `defer` の呼び出しは、それぞれ自分専用のバッファを得ます。呼び出しは互いを踏みつぶすことができず、漏れ出す隠れたグローバル状態もありません。

## 各要素の実装場所

| 要素 | ファイル |
|---|---|
| `Event` トレイト、`Listener<E>`、`Subscriber` | `framework/src/events/mod.rs` |
| `EventDispatcher`、`EventFacade`（ファサード構造体） | `framework/src/events/dispatcher.rs` |
| `ErrorOccurred` | `framework/src/events/builtins.rs` |
| `QueuedListener<E, J>` | `framework/src/events/queued_listener.rs` |
| `assert_dispatched*`、`EventFakeGuard`、`muted` | `framework/src/events/testing.rs` |
| 組み込みのイベントペイロード | `framework/src/{database,auth,auth_flows,mail,notifications,queue,features}/events.rs` |
| モデルごとのライフサイクルイベント | マクロによって各モデルの `events::` サブモジュールへ生成される |

## 次のステップ

- [エラー モデル](error-model.md) - `ErrorOccurred` と、5xxの変換経路
- [キュー](queues.md) - 永続的なジョブ、クラッシュ耐性のある層。`QueuedListener` はこれへ橋渡しする
- [ブロードキャスト](broadcasting.md) - `EventFacade::broadcast::<E>(hub)` を介して、ディスパッチされたイベントをWebSocketのチャネルへ配線する
- [Eloquent](eloquent.md) - モデルのライフサイクルイベントと `Observer<M>` トレイト
- [データベース](database.md) - `DB::listen` と `Database\\QueryExecuted` イベント
