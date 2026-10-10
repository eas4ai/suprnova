# タスク スケジューリング

スケジュールされたタスクとは、フレームワークがcron式に従って実行する非同期関数です - 毎分、毎時、毎日、毎週、あるいは任意のカスタムな5フィールドのcronです。タスクはあなたのアプリケーションバイナリの中に存在します。`schedule:run` は実行予定のタスクを一度だけ評価し（システムcronから呼び出します）、`schedule:work` は同じ評価器を長寿命のデーモンとして実行します。

## タスクを生成する

新しいスケジュールされたタスクを作る最も速い方法は、suprnova CLIを使うことです。

```bash
suprnova make:task CleanupLogs
```

このコマンドは次を行います。
1. 動作するタスクのスタブを持つ `src/tasks/cleanup_logs_task.rs` を作成する
2. `src/tasks/mod.rs` が存在しなければ作成し、そのタスクを再エクスポートする
3. タスクを登録するための `src/schedule.rs` を、存在しなければ作成する
4. `src/lib.rs` に `pub mod schedule;` と `pub mod tasks;` を宣言する
5. `cmd/main.rs`（APIスターターでは `src/main.rs`）の中の、あなたのアプリケーションビルダーに `.schedule(<crate>::schedule::register)` を配線する

ステップ2〜5はべき等であるため、`make:task` を再実行すれば、手動で取り除かれた配線を修復できます。スケジューラーはあなたのアプリケーションバイナリの中で実行されます - ビルドやデプロイが必要な、別個のスケジューラー実行ファイルはありません。

```bash Examples
# src/tasks/cleanup_logs_task.rs に CleanupLogsTask を作成する
suprnova make:task CleanupLogs

# src/tasks/send_reminders_task.rs に SendRemindersTask を作成する
suprnova make:task SendReminders

# 「Task」サフィックスを含めることもできる（結果は同じ）
suprnova make:task BackupDatabaseTask
```

```rust Generated File
//! CleanupLogsTask スケジュールされたタスク
//!
//! `suprnova make:task cleanup_logs_task` で作成された。

use std::time::Instant;

use async_trait::async_trait;
use suprnova::{Task, TaskResult};

/// CleanupLogsTask - スケジュールされたタスク。
///
/// フルーエントなAPIで `src/schedule.rs` にタスクを登録する。以下の
/// スケルトンは自分自身の実行時間を計測し、呼び出しのたびに構造化された
/// ログ行を出力するため、配線した最初の瞬間からエンドツーエンドで動作する。
pub struct CleanupLogsTask;

impl CleanupLogsTask {
    /// このタスクの新しいインスタンスを作成する。
    pub fn new() -> Self {
        Self
    }
}

impl Default for CleanupLogsTask {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Task for CleanupLogsTask {
    async fn handle(&self) -> TaskResult {
        let started_at = Instant::now();
        println!("[CleanupLogsTask] task started");

        // これを実際のジョブに置き換える。スケルトンは、実装が
        // 書き込まれる前でもタスクをスケジュールして観測できるよう、
        // no-opの成功として出荷されている。

        println!(
            "[CleanupLogsTask] task finished in {} ms",
            started_at.elapsed().as_millis(),
        );
        Ok(())
    }
}
```

## スケジュールを定義する

suprnovaは、スケジュールされたタスクを定義するための2つのアプローチをサポートします。

### 1. トレイトベースのタスク（推奨）

依存関係や再利用可能なロジックを必要とする複雑なタスクには、`Task` トレイトを実装し、登録の際にスケジュールを設定してください。

```rust
// src/tasks/cleanup_logs_task.rs
use async_trait::async_trait;
use chrono::{Duration, Utc};
use suprnova::{Task, TaskResult};
use crate::models::Log;

pub struct CleanupLogsTask;

impl CleanupLogsTask {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Task for CleanupLogsTask {
    async fn handle(&self) -> TaskResult {
        // Eloquentは、コントローラーの内部とまったく同じように動作する。タスクは、
        // リクエストハンドラが見るのと同じコンテナのバインディング
        // （`DB::connection()`、`App::get::<T>()`）を見る - 下のアプリケーション
        // ブートストラップを参照。
        let cutoff = Utc::now() - Duration::days(30);
        Log::query()
            .filter_op("created_at", "<", cutoff)
            .delete_all()
            .await?;

        println!("Old logs cleaned up successfully");
        Ok(())
    }
}
```

続いて、`src/schedule.rs` の中で、フルーエントなスケジューリングAPIを使って登録します。

```rust
// src/schedule.rs
use suprnova::Schedule;
use crate::tasks::CleanupLogsTask;

pub fn register(schedule: &mut Schedule) {
    schedule.add(
        schedule.task(CleanupLogsTask::new())
            .daily()
            .at("03:00")
            .name("cleanup:logs")
            .description("Removes logs older than 30 days")
    );
}
```

### 2. クロージャベースのタスク

別個のファイルを必要としない、手軽でインラインなタスクには次のようにします。

```rust
// src/schedule.rs
use suprnova::Schedule;

pub fn register(schedule: &mut Schedule) {
    // シンプルなクロージャタスク
    schedule.add(
        schedule.call(|| async {
            println!("Ping! Running every minute");
            Ok(())
        })
        .every_minute()
        .name("heartbeat")
    );

    // 設定されたクロージャタスク
    schedule.add(
        schedule.call(|| async {
            // あなたのタスクのロジック
            Ok(())
        })
        .daily()
        .at("09:00")
        .name("morning-report")
        .description("Sends daily morning report")
    );
}
```

## タスクを登録する

`src/schedule.rs` の中で、あなたのタスクを登録します。

```rust
// src/schedule.rs
use suprnova::Schedule;
use crate::tasks;

pub fn register(schedule: &mut Schedule) {
    // フルーエントなスケジュール設定を伴う、トレイトベースのタスク
    schedule.add(
        schedule.task(tasks::CleanupLogsTask::new())
            .daily()
            .at("03:00")
            .name("cleanup:logs")
            .description("Removes logs older than 30 days")
    );

    schedule.add(
        schedule.task(tasks::SendRemindersTask::new())
            .daily()
            .at("09:00")
            .name("send:reminders")
            .description("Sends daily reminder emails")
    );

    schedule.add(
        schedule.task(tasks::BackupDatabaseTask::new())
            .weekly()
            .at("00:00")
            .name("backup:database")
            .description("Weekly database backup")
            .without_overlapping()
    );

    // クロージャベースのタスク
    schedule.add(
        schedule.call(|| async {
            println!("Quick task!");
            Ok(())
        })
        .hourly()
        .name("quick-task")
    );
}
```

## スケジュールの頻度オプション

suprnovaは、タスクをいつ実行すべきかを定義するための、フルーエントなAPIを提供します:

### よくある間隔

| メソッド | 説明 |
|--------|-------------|
| `.every_minute()` | 毎分実行します |
| `.every_two_minutes()` | 2分ごとに実行します |
| `.every_five_minutes()` | 5分ごとに実行します |
| `.every_ten_minutes()` | 10分ごとに実行します |
| `.every_fifteen_minutes()` | 15分ごとに実行します |
| `.every_thirty_minutes()` | 30分ごとに実行します |
| `.hourly()` | 毎時0分に実行します |
| `.hourly_at(30)` | 毎時30分に実行します |
| `.every_two_hours()` / `.every_three_hours()` / `.every_four_hours()` / `.every_six_hours()` | N時間ごとに、毎正時に実行します |
| `.daily()` | 毎日0時に実行します |
| `.daily_at("03:00")` | 毎日午前3時に実行します |
| `.twice_daily(1, 13)` | 1日に2回実行します（例: 午前1時と午後1時） |
| `.weekly()` | 毎週日曜日の0時に実行します |
| `.monthly()` | 毎月1日の0時に実行します |
| `.monthly_on(15)` | 毎月、指定した日に実行します |
| `.quarterly()` | 1月/4月/7月/10月の1日の0時に実行します |
| `.yearly()` | 1月1日の0時に実行します |

### 曜日を指定したスケジュール

```rust
use suprnova::DayOfWeek;

// 特定の曜日に実行する
.weekly_on(DayOfWeek::Monday)
.weekly_on(DayOfWeek::Friday)

// 曜日の短縮メソッド
.sundays()
.mondays()
.tuesdays()
.wednesdays()
.thursdays()
.fridays()
.saturdays()

// 複数の曜日
.days(&[DayOfWeek::Monday, DayOfWeek::Wednesday, DayOfWeek::Friday])

// 平日/週末
.weekdays()  // 月曜日から金曜日
.weekends()  // 土曜日と日曜日
```

### 時刻の修飾子

具体的な時刻を設定するには、任意のスケジュールに `.at()` をチェーンしてください:

```rust
.daily().at("14:30")           // 毎日14:30に
.weekly().at("09:00")          // 毎週9:00に
.mondays().at("08:00")         // 毎週月曜日の8:00に
.monthly().at("00:00")         // 毎月1日の0時に
```

### タイムゾーン

デフォルトでは、スケジューラーはすべてのcron式を、コンテナが起動されたときの `TZ` が何であれ、プロセスのローカルゾーンに対して読みます。タスクのスケジュールがサーバーではなく場所に属するときは、そのタスクを名前付きのIANAゾーンへ固定してください:

```rust
use suprnova::chrono_tz;

schedule.add(
    schedule.task(GenerateReportTask::new())
        .daily()
        .at("02:00")
        .timezone(chrono_tz::America::New_York)
        .name("report:generate")
);
```

`timezone` は型付きの `chrono_tz::Tz` を受け取るため、綴りを誤ったゾーンは、間違った時刻に静かに走るタスクではなくコンパイルエラーになります。ゾーンの定数は `suprnova::chrono_tz` の下にあり（`chrono_tz::Asia::Tokyo`、`chrono_tz::Europe::Berlin` など）、再エクスポートされているので、あなた自身の `Cargo.toml` に `chrono-tz` は必要ありません。

ゾーン名が実行時にしか存在しないとき - 設定の値や、テナントのカラムなど - は、失敗し得る兄弟を使ってください:

```rust
schedule.add(
    schedule.task(GenerateReportTask::new())
        .daily()
        .at("02:00")
        .try_timezone(&tenant.timezone)?   // 未知のゾーンでは Err(String)
        .name("report:generate")
);
```

固定されたゾーンが変えるのは、ちょうど1つのことだけです: 5つのcronのフィールドが、どの壁時計に対して読まれるかです。スケジューラーは今もプロセスの1分に一度ティックし、同一分の重複排除ゲートは影響を受けません。

#### スケジュール全体のデフォルト

タスクの大半が1つのビジネス上のゾーンに属するのであれば、すべてのタスクで繰り返すのではなく、スケジュールに一度だけ設定してください:

```rust
pub fn register(schedule: &mut Schedule) {
    schedule.timezone(chrono_tz::America::Chicago);

    // 02:00 America/Chicago として読まれます
    let nightly = schedule
        .call(|| async { Ok(()) })
        .daily()
        .at("02:00")
        .name("nightly");
    schedule.add(nightly);

    // タスクごとの明示的なゾーンが常に勝ちます
    let tokyo = schedule
        .call(|| async { Ok(()) })
        .daily()
        .at("09:00")
        .timezone(chrono_tz::Asia::Tokyo)
        .name("tokyo-open");
    schedule.add(tokyo);
}
```

デフォルトはタスクが追加されるときに適用されるため、その呼び出しより後に登録されたタスクを覆い、それより前のタスクは手つかずのまま残します。

#### 夏時間

夏時間を採用しているゾーンがあります。時計が変わるとき、そうしたゾーンへ固定されたタスクは、2回走ったり、まったく走らなかったりすることがあります:

- 時計を戻すとき、壁時計上の1時間は2回起こります。`01:30` のタスクは、その両方の通過に一致します。それらは実時間としては異なる2つの分であるため、同一分の重複排除ゲートはそれらを併合せず、タスクは2回走ります。
- 時計を進めるとき、壁時計上の1時間はまったく起こりません。`02:30` のタスクは、その日は完全にスキップされます。

可能な限りタイムゾーンによるスケジューリングは避け、ちょうど一度だけ走らなければならないものには、夏時間のないゾーン（`chrono_tz::UTC`）を選んでください。

#### 一覧を別のゾーンで読む

`schedule:list` は `--timezone` を受け取り、cron式と次回の実行時刻の両方を、そのゾーンで読んだとおりに表示します。実際の出力については[タスクを一覧表示する](#タスクを一覧表示する)を参照してください。

### Suprnovaが異なる設計を選んだ理由: タイムゾーン

Laravelの `timezone()` は文字列を受け取り、スケジュール全体のデフォルトは `app.schedule_timezone` という設定キーから来ます。Suprnovaは型付きの `chrono_tz::Tz` を受け取り、設定キーを持ちません: あなたの `schedule::register` 関数の中の `Schedule::timezone` が、デフォルトが設定される唯一の場所であり、スケジュールは参照すべき2つ目のファイルなしに上から下へ読めます。

何も固定されていないときのSuprnovaのデフォルトは、設定されたアプリケーションのタイムゾーンではなく、プロセスのローカルゾーンです。それはスケジューラーがずっと持ってきた挙動であり、この機能を追加してもそれを使わないスケジュールには何も変わらないよう、デフォルトのままにしてあります。

### カスタムのcron式

完全に制御したい場合は、cronの構文を使ってください:

```rust
// 標準のcronの形式: 分 時 日 月 曜日
.cron("0 */2 * * *")    // 2時間ごと
.cron("30 4 * * 1-5")   // 平日の午前4時30分
.cron("0 0 1,15 * *")   // 毎月1日と15日
```

式が不正な場合（フィールド数の誤り、パースできないステップ/範囲/リスト）、`.cron(...)` は**パニックします**。式が実行時に与えられ（設定やユーザー入力）、パースのエラーを伝播させたい場合は `.try_cron(expr)` を使ってください:

```rust
schedule.add(
    schedule.task(MyTask::new())
        .try_cron(env_expr)?   // 不正な式では Err(String) を返します
        .name("from-config")
);
```

同じ `panic` / `try_*` の組は、数値の範囲を取るすべてのビルダーメソッドに存在します: `try_hourly_at`、`try_daily_at`、`try_twice_daily`、`try_monthly_on` です。失敗しないほうのバリアントは、範囲外の数値（例えば `daily_at("25:00")` や `monthly_on(40)`）でパニックします。失敗し得る兄弟は `Err(String)` を返します。

## タスクの設定

### 重複を防ぐ

同じタスクの前回の実行がまだ進行中である場合、そのティックをスキップします。

```rust
schedule.add(
    schedule.task(LongRunningTask::new())
        .daily()
        .name("long-task")
        .without_overlapping()
);
```

**ロックがどう機能するか。** このフラグが設定されると、suprnovaは、設定済みの[`Cache`](cache.md)バックエンドを介して分散mutex（`schedule:lock:<task-name>`）を取得しようとします。取得に成功すればタスクを実行してロックを解放します。取得が競合した場合は、成功したスキップとして報告されます - `Ok(())` であり、タスクのスキップカウンターが増加するため、`schedule:run` の終了コードを汚すことなく、可観測性の表面がそれを見ることができます。

**プロセスをまたぐ保護には、Cacheが必須です。** 同じタスクをスケジュールする複数のプロセスを実行している場合（例えば、システムcronから `suprnova schedule:run` を呼び出す複数のマシン、あるいはロードバランサーの背後にある複数の `schedule:work` デーモン）、それらを協調させるのがCacheバックエンドです。**設定されたCacheがなければ、`without_overlapping()` はサイレントに、プロセスごとの `AtomicBool` へ縮退します** - 2つの別個のプロセスは互いのロックを見ることができません。このフォールバックが最初に発火したとき、オペレーターがその弱い保証に気づけるよう、フレームワークは一度だけ `WARN`（`suprnova::schedule`）を発します。

> `without_overlapping() falling back to in-process AtomicBool protection - Cache is not bootstrapped. Multi-process deployments will NOT see each other's locks. Configure Cache (CACHE_DRIVER=memory|redis) before relying on cross-process overlap protection.`

**カスタムのロックTTL。** ロックTTLのデフォルトは30分です - ほとんどのタスクが終わるには十分な長さであり、ロックを持ったままクラッシュしたタスクが、オペレーターの介入なしに次のティックのブロックを解くには十分な短さです。`.without_overlapping_for(Duration)` で、タスクごとに上書きしてください。`Duration::ZERO` は、キャッシュバックエンドをまたいで未定義です（Redisはエラーになり、インメモリは即座に期限切れになり、Memcachedは「決して期限切れにならない」と解釈します）。そのため、ビルダーはそれを30分のデフォルトへ補正し、オペレーターが呼び出し箇所を修正できるよう、一度だけ `WARN` を発します。

```rust
use std::time::Duration;

schedule.add(
    schedule.task(SlowBackupTask::new())
        .daily()
        .name("backup:full")
        // このジョブは、正当な理由で30分のデフォルトより長く実行される。
        // 遅い実行が次のティックに横取りされないよう、
        // ロックに2時間のTTLを与える。
        .without_overlapping_for(Duration::from_secs(2 * 3600))
);
```

### 単一サーバーで実行する

スケジューラーを実行しているレプリカがいくつあっても、実行予定のティックごとに、タスクをちょうど1回だけ実行します。

```rust
schedule.add(
    schedule.task(NightlyBillingTask::new())
        .daily()
        .at("02:00")
        .name("billing:nightly")
        .on_one_server()
);
```

**これがないと何が問題になるか。** `schedule:work` を実行しているすべてのレプリカは、スケジュールを独立に評価するため、それらすべてが同じティックを自分のものだと判断してしまうのを止めるものが何もありません。3つのレプリカで測定した結果、ばらつきなく、毎分同じタスクが3回実行されました。夜間の課金ジョブであれば、それはすべての顧客が3回課金されることを意味します。

**なぜ `without_overlapping()` はこれをカバーしないのか。** この2つは似ているように見えますが、異なる問題を解決します。

| | ロックキー | 保持される期間 | 防ぐもの |
|---|---|---|---|
| `without_overlapping()` | タスク | タスクの所要時間 | 遅い実行が、自分自身の次のティックと重なること |
| `on_one_server()` | タスク**+そのティック** | ティックのウィンドウ | 2番目のレプリカが同じティックを実行すること |

重要な違いは、ロックがいつ解放されるかです。`without_overlapping()` は、ハンドラが戻った瞬間に解放します - 速いタスクであれば、2番目のレプリカがまだ見に来る前であり、そのためN個すべてがそれでも実行されてしまいます。`on_one_server()` は、意図的にハンドラの後もロックを保持し続け、TTLで期限切れにさせます。なぜなら、同じティックに後から到着したレプリカは、それが取られていることを見つけなければならないからです。

これらは組み合わせられます。単一サーバーでもなければならない、長時間実行のタスクは、両方を取ります。

**共有キャッシュが必要です。** この選出は[`Cache`](cache.md)ロックであるため、「単一サーバー」とは「キャッシュバックエンドを共有するプロセスのうちの1つ」を意味します。`CACHE_DRIVER=memory` の下では、ロックは単一プロセスのヒープの中に存在し、すべてのレプリカが自分自身の選出に勝ってしまい、その保証はサイレントに失われます。

本番環境では、これは警告ではなく**起動失敗**です。

> `refusing to boot in production: 1 task(s) request single-server execution (billing:nightly) but CACHE_DRIVER is memory or unset, so the election lock lives in this process's heap. Every replica would win its own election and run the task, which is what on_one_server() exists to prevent. Set CACHE_DRIVER=redis with REDIS_URL, or set SCHEDULE_ALLOW_MEMORY_LOCK_IN_PRODUCTION=true to acknowledge per-process locking - which is only accurate if you run exactly one scheduler.`

あなたのデプロイが本当に単一のスケジューラーだけを実行しているのであれば、`SCHEDULE_ALLOW_MEMORY_LOCK_IN_PRODUCTION=true` を設定してください。本番環境の外では、メモリドライバーは使用可能なままであり、フレームワークは代わりに一度だけ警告します。

**カスタムのロックTTL。** デフォルトは60秒 - 1分に揃えられたティックです。両方の極端が問題になります: 短すぎると、ティックが数秒遅れて到着したレプリカがロックの消失を見つけ、タスクを再び実行してしまいます。長すぎるとロックがそのティックより長生きしてしまい、*次に*実行予定の実行はそれが保持されていることを見つけ、完全にスキップされてしまいます。より粗いスケジュールには `.on_one_server_for(Duration)` を使ってください。

```rust
use std::time::Duration;

schedule.add(
    schedule.task(HourlyRollupTask::new())
        .hourly()
        .name("rollup:hourly")
        // 1時間ごとのタスクは、レプリカがまだこのティックを実行予定と
        // 呼びうるウィンドウより、ロックが長生きしさえすればよい。
        .on_one_server_for(Duration::from_secs(300))
);
```

**キャッシュに到達できない場合**は、ティックは実行されるのではなくスキップされます。協調を失っている瞬間は、すべてのレプリカを通してしまうのに最悪のタイミングです: スキップされたティックは次のティックで回復可能ですが、重複した副作用は一般的にそうではありません。

### Suprnovaが異なる設計を選んだ理由

Laravelの `onOneServer()` は同じオプトインであり、Suprnovaはそれを保っています: サーバーごとのタスク - ログローテーション、ローカルキャッシュのウォーミングなど - は正当なものであり、表現可能なままです。

違ってくるのは失敗のモードです。Laravelは、協調できないキャッシュドライバーに対しても、何の問題もなく `onOneServer()` を実行してしまいます。Suprnovaは、代わりに本番環境での起動を拒否します。インメモリのレート制限装置と同じ理屈です: 主張しているよりもずっと少ないことしかサイレントに行わない制御は、目に見えて存在しない制御よりも悪いのです。

### バックグラウンドで実行する

他の実行予定のタスクが開始をブロックされないよう、タスクをティックごとのクリティカルパスから切り離します。

```rust
schedule.add(
    schedule.task(BackgroundTask::new())
        .hourly()
        .name("background-task")
        .run_in_background()
);
```

**パニックの分離。** バックグラウンドタスクは `catch_unwind` を伴う `tokio::task::JoinSet` の内側で実行されるため、パニックするタスクは、スケジューラーを引き倒すのではなく、そのタスク名に対して記録された `FrameworkError` として表面化します。`schedule:work` デーモンは、シャットダウン時（Ctrl-C / SIGTERM）にJoinSetをドレインするため、進行中のバックグラウンドタスクは終了前に完了します。

**`without_overlapping` との組み合わせ。** この2つのフラグは組み合わせられます - `without_overlapping()` を持つバックグラウンドタスクは、JoinSetへ生成され、生成されたフューチャーの内側から重複ロックを取得するため、上で説明したロックのセマンティクスがそれでも適用されます。

### 同一分の重複排除

Cronの分解能は分単位であり、suprnovaはそれを強制します: 同じタスクが、単一プロセスの中で、同じ実時計の分の内側に2回実行するよう求められた場合、2回目の呼び出しはno-opのスキップになります - `Ok(())` であり、タスクのスキップカウンターが増加します。これは、デーモンループや、間隔の詰まった `schedule:run` の呼び出しが、`.every_minute()` タスクを同じ分の中で複数回実行してしまうという、一種のバグを塞ぎます。

このプロセス内のゲートは、`without_overlapping` とは無関係に、**常に有効です**。これはプロセスをまたぎません（各プロセスは、タスクごとの状態を自分自身で持ちます）。プロセスをまたぐ同一分の協調が必要な場合は、`without_overlapping` を重ねてください + 設定済みのCacheバックエンドを使ってください - 両方を合わせれば、両方の方向をカバーします。

## スケジューラーを実行する

suprnovaは、スケジュールされたタスクを実行するためのCLIコマンドを提供します:

### 一度だけ実行する

実行予定のタスクをすべて一度だけ実行します（通常は、cronから毎分呼び出されます）:

```bash
suprnova schedule:run
```

### デーモンモード

継続的に実行し、毎分、実行予定のタスクを確認します:

```bash
suprnova schedule:work
```

これは、開発時や、systemdのようなプロセスマネージャーを使うときに理想的です。

### タスクを一覧表示する

登録されているスケジュール済みのタスクをすべて表示します:

```bash
suprnova schedule:list
```

出力:
```
Registered scheduled tasks:
  cleanup:logs [0 3 * * *] next: 2026-05-29 03:00 UTC
  send:reminders [0 9 * * *] next: 2026-05-28 09:00 UTC
  report:generate [0 6 * * *] (UTC) next: 2026-05-29 06:00 UTC
```

各行は、タスク名、cron式、省略可能なゾーンのラベル、そのタスクが次に発火する時刻、そして説明があればタスクの説明です。

`next:` は、式が一致する現在時刻より後の最初の分であり、そのタスクが評価されるゾーンで計算されたうえで、一覧のゾーンで表示されます。決して一致し得ない式（`0 0 30 2 *` は存在しない日付を指しています）は `next: never` と出力します。

一覧のゾーンは、`--timezone` を渡さない限りUTCです。上記の `cleanup:logs` と `send:reminders` はゾーンを固定していないため、その式は書かれたとおりに出力され - スケジューラーはそれらをプロセスのローカルゾーンに対して読みますが、そこには変換元にできるIANA名がありません - ゾーンのラベルも付きません。`report:generate` は `America/New_York` を固定して `02:00` を要求したため、その式は一覧のゾーンへ書き換えられ、そのラベルが付きます。

```bash
suprnova schedule:list --timezone=Asia/Tokyo
```

```
Registered scheduled tasks:
  cleanup:logs [0 3 * * *] next: 2026-05-29 12:00 JST
  send:reminders [0 9 * * *] next: 2026-05-28 18:00 JST
  report:generate [0 15 * * *] (Asia/Tokyo) next: 2026-05-29 15:00 JST
```

1つのタスクが複数の行を占めることがあります。一覧のゾーンで真夜中をまたぐ式は、片側ごとに1つのcronの行を必要とします。5フィールドの式1つでは、その両方を記述できないからです:

```
  monday-digest [0 23 * * 1] (Asia/Tokyo) next: 2026-06-01 23:00 JST
  monday-digest [0 5 * * 2] (Asia/Tokyo) next: 2026-06-01 23:00 JST
```

`next:` は行にではなくタスクに属するため、繰り返されます: どちらの行も、同じタスクと同じ次回の実行を記述しています。

一部の変換は、近似されるのではなく拒否され、拒否された式はそのタスク自身のゾーンのラベルとともに、書かれたとおりに出力されます。変換が拒否されるのは、次の2回の実行のあいだに夏時間の切り替わりが入るとき（どちらの側でも正しい式が1つも存在しません）、日付のまたぎが、制限された日と制限された曜日を一緒に動かさなければならなくなるとき（cronはこの2つのフィールドをORで結ぶため、両方をずらすとどの日が一致するかが変わってしまいます）、あるいはまたぎが2月の長さを決めなければならなくなるときです。

## 本番環境のセットアップ

### Cronを使う

スケジューラーを毎分実行する、単一のcronエントリを追加します。

```bash
* * * * * cd /path/to/your/project && suprnova schedule:run >> /dev/null 2>&1
```

**プロセスをまたぐ協調。** 複数のホスト上のシステムcronから `schedule:run` を実行している場合（あるいは `schedule:work` デーモンと並行して実行している場合）、`.without_overlapping()` を持つタスクは、プロセスをまたいで協調するために、設定済みの**Cache**バックエンド（本番環境では `CACHE_DRIVER=redis` が推奨されます）を必要とします。それがなければ、重複フラグはプロセスごとの保護へ縮退し、同じタスクが同じ分の中で複数のホスト上で実行されてしまう可能性があります。ロックのセマンティクスの全体像については、上の[重複を防ぐ](#重複を防ぐ)を参照してください。

### Systemdを使う

スケジューラーデーモンのためのsystemdサービスを作成します。

```ini
# /etc/systemd/system/myapp-scheduler.service
[Unit]
Description=MyApp Scheduler
After=network.target

[Service]
Type=simple
User=www-data
WorkingDirectory=/path/to/your/project
ExecStart=/path/to/suprnova schedule:work
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl enable myapp-scheduler
sudo systemctl start myapp-scheduler
```

## アプリケーションコンテキストへアクセスする

スケジュールされたタスクは、コントローラーと同様に、アプリケーションコンテキストへの完全なアクセスを持ちます。

```rust
use async_trait::async_trait;
use suprnova::{App, Task, TaskResult};
use crate::actions::SendEmailAction;
use crate::models::User;

pub struct SendRemindersTask;

#[async_trait]
impl Task for SendRemindersTask {
    async fn handle(&self) -> TaskResult {
        // Eloquent: `.get()` は、反復可能な `Collection<User>` を返す。
        let users = User::query()
            .filter("reminder_enabled", true)
            .get()
            .await?;

        // `bootstrap.rs` でバインドされたものは、ここからも到達できる。
        let send_email = App::get::<SendEmailAction>()
            .expect("SendEmailAction bound in bootstrap()");

        for user in users.iter() {
            send_email.execute(&user.email, "Daily Reminder").await?;
        }

        Ok(())
    }
}
```

## ファイル構成

スケジュールされたタスクのための、推奨されるファイル構造です。

```
src/
├── tasks/
│   ├── mod.rs              # すべてのタスクを再エクスポートする（make:taskが自動更新する）
│   ├── cleanup_logs_task.rs
│   ├── send_reminders_task.rs
│   └── backup_database_task.rs
├── schedule.rs             # タスクを登録する（schedule:* コマンドから実行される）
├── bootstrap.rs
├── routes.rs
└── lib.rs                  # `pub mod schedule;` + `pub mod tasks;` を宣言する
cmd/
└── main.rs                 # `.schedule(<crate>::schedule::register)` を呼び出す
```

**src/tasks/mod.rs:**
```rust
pub mod cleanup_logs_task;
pub mod send_reminders_task;
pub mod backup_database_task;

pub use cleanup_logs_task::CleanupLogsTask;
pub use send_reminders_task::SendRemindersTask;
pub use backup_database_task::BackupDatabaseTask;
```

## スケジューラーをアプリケーションに配線する

`make:task` は、`.schedule(<crate>::schedule::register)` を、あなたの `Application` ビルダーへ自動的に配線します。チェーンを手で組み立てる場合、関係する呼び出しは `Application` の上にあります。

```rust
// cmd/main.rs（apiスターターでは src/main.rs）
Application::new()
    .config(my_app::config::register)
    .bootstrap(my_app::bootstrap::bootstrap)
    .routes(my_app::routes::register)
    .schedule(my_app::schedule::register)        // <- この行
    .migrations::<my_app::migrations::Migrator>()
    .run()
    .await;
```

`.schedule(...)` がなければ、`schedule:*` サブコマンドはすべて、タスクが登録されていないと報告します。`schedule:work` と `schedule:run` も、HTTPサーバーと同じランタイムドライバーと `bootstrap_fn` を実行するため、起動時に登録されたオブザーバー、リスナー、コンテナのバインディングは、コントローラーに対してと全く同じように、あなたのタスクハンドラからも見えます（[アプリケーション ブートストラップ](bootstrap.md)を参照）。

### Suprnovaが異なる設計を選んだ理由

Laravelのスケジューラーは、それ自体がPHP-cronが毎分トリガーする単一のArtisanコマンド（`schedule:run`）です。PHPのランタイムが立ち上がり、実行予定のタスクを評価し、プロセス内で実行するかシェルアウトし、そしてランタイムを取り壊します。PHPには長寿命のプロセスがないため、デーモン形式（`schedule:work`）はLumenによってバックポートされ、crontabへのアクセスがないサイトのための回避策として、Laravel自身にも出荷されています。

Suprnovaでは、デーモンは第一級です。`schedule:work` は、すでに長寿命であるTokioランタイムの内側で実行されます。そのため:

- **バックグラウンドタスク（`run_in_background`）はティックループと合成されます。** Laravelはバックグラウンドタスクごとに子プロセスを生成しますが、私たちは `JoinSet` へ生成し、次のティックかシャットダウン時に完了を表面化させます。
- **グレースフルシャットダウンは `tokio::select!` の1本のアームです。** Ctrl-C / SIGTERMは、終了前に進行中のバックグラウンドタスクをドレインします。プロセス内のタスクは、現在の呼び出しを完了させます。
- **同一分の重複排除は、プロセス内の状態です。** タスクごとの `last_run_minute` アトミックは、ループが速くティックしても、単一プロセスが分に揃えられたタスクを二重発火できないことを保証します。PHPはこれができません - cronのティックのたびに新しいプロセスになるからです - そしてそれが、Laravelがファイルシステムロックを唯一の防衛線として使っている理由です。

`Cache::lock` に支えられた `without_overlapping` は、複数プロセスのケース（複数ホスト上のシステムcron、ロードバランサーの背後にある複数の `schedule:work` デーモン）のために、それでも存在します。これは同じ仕組みですが、スケジューラーが常には必要としない層にあるだけです。

## まとめ

| 機能 | 使い方 |
|---------|-------|
| タスクを作成する | `suprnova make:task TaskName` |
| トレイトベース | `Task` トレイトを実装し、登録の際にスケジュールを設定する |
| クロージャベース | `schedule.call(\|\| async { ... })` |
| タスクを登録する | `schedule.add(schedule.task(...).daily().name("..."))` |
| アプリに配線する | `Application::new().schedule(schedule::register)` |
| 一度だけ実行する | `suprnova schedule:run` |
| デーモンとして実行する | `suprnova schedule:work` |
| タスクを一覧表示する | `suprnova schedule:list` |
| 重複を防ぐ | `.without_overlapping()`（Cacheバックエンド経由のデフォルト30分ロックTTL） |
| カスタムの重複TTL | `.without_overlapping_for(Duration)` |
| バックグラウンド | `.run_in_background()`（JoinSetによるパニック分離） |
| 同一分の重複排除 | プロセスごとに常に有効。スキップされた実行は `Ok(())` を返す |
| 実行時に検証されるcron | `.try_cron(expr)` / `.try_daily_at(s)` / `.try_hourly_at(n)` |

## 次のステップ

- [スケジューリング コマンド](cli-scheduling.md) - `schedule:run` / `schedule:work` / `schedule:list` のCLIリファレンス
- [キュー](queues.md) - 時計でティックするのではなく、ワーカーが拾い上げるべき作業のために
- [コンソール](console.md) - スケジュールに乗らない、ワンショットの運用者向けタスクのための `#[command]`
- [キャッシュ](cache.md) - プロセスをまたぐ `without_overlapping` を支えるバックエンド
- [アプリケーション ブートストラップ](bootstrap.md) - `.schedule(...)` がビルダーにどう組み込まれ、タスクがコンテナから何を解決できるか
