# スケジューリング コマンド

分単位のタスクスケジューラーのためのCLI表面です。3つの `schedule:*` サブコマンドはすべて、あなたのアプリケーションバイナリの `Application::run()` ディスパッチへ委譲するため、リクエストハンドラが見るのと同じconfig、サービス、オブザーバー、リスナーを見ます。スケジューラーの全体モデル - `Task` トレイト、フルーエントなcron API、`without_overlapping`、`run_in_background` - は[タスク スケジューリング](scheduling.md)にあります。この章は、コマンド自体のための運用者向けリファレンスです。

## コマンドの実行方法

`suprnova schedule:run`、`suprnova schedule:work`、`suprnova schedule:list` は、現在のディレクトリのプロジェクトに対して `cargo run -- schedule:<subcommand>` を呼び出す、薄いシェルです。同じサブコマンドは、本番環境ではアプリケーションバイナリ上で直接到達することもできます:

```bash
# 開発環境では（プロジェクトルートから、ソースビルドで）:
suprnova schedule:run

# 本番環境では（PATH上のバイナリで）:
/usr/local/bin/myapp schedule:run
```

ランタイムドライバー（Cache、Queue、RateLimit、Mail）とあなたの `bootstrap_fn` は、どのタスクが実行される前にも起動されるため、スケジュールされたタスクは、コントローラーと全く同じようにコンテナからサービスを解決できます - [アプリケーション ブートストラップ](bootstrap.md)を参照してください。

サブコマンドがタスクを見つけられるようにするには、スケジューラーをアプリケーションビルダーに配線しなければなりません:

```rust
// cmd/main.rs（バックエンドスターター）または src/main.rs（APIスターター）
Application::new()
    .config(my_app::config::register)
    .bootstrap(my_app::bootstrap::bootstrap)
    .routes(my_app::routes::register)
    .schedule(my_app::schedule::register)   // <-- スケジューラーのフック
    .migrations::<my_app::migrations::Migrator>()
    .run()
    .await
```

`suprnova make:task <Name>` は、これを自動的に配線します。チェーンを手で組み立てる場合は、`.schedule(...)` の呼び出しを自分で追加してください。

## schedule:run

登録済みのタスクをすべて一度だけ評価し、cron式が現在の分に一致するものを実行します。システムcronによって毎分呼び出されるよう設計されています。いずれかのタスクが失敗すれば非ゼロで終了し、この分に何も実行予定がなければ（`No tasks were due.` とともに）ゼロで終了します。

```bash
suprnova schedule:run
```

### 出力例

```
Running due scheduled tasks...
  ✓ cleanup:logs
  ✓ send:reminders
```

タスクがエラーを返すと、その行には `✗` が前置され、エラーメッセージが追記されます:

```
Running due scheduled tasks...
  ✓ cleanup:logs
  ✗ backup:database: connection refused
```

この分に実行予定のタスクが何もない場合:

```
Running due scheduled tasks...
No tasks were due.
```

### crontabのエントリ

1つのエントリが、毎分スケジューラーを実行します。アプリケーションバイナリ自身が実行予定のタスクをすべて評価するため、本番環境のホストが必要とするcrontabの行は、これだけです:

```cron
* * * * * cd /path/to/your/project && /usr/local/bin/myapp schedule:run >> /var/log/myapp/schedule.log 2>&1
```

2台以上のホストのシステムcronから `schedule:run` を実行している場合（あるいは `schedule:work` デーモンと並行して実行している場合）、`.without_overlapping()` でマークされたタスクは、プロセスをまたいで協調するために、設定済みのCacheバックエンド（`CACHE_DRIVER=redis` が本番環境向けの選択です）を必要とします - ロックのセマンティクスについては、[重複を防ぐ](scheduling.md#preventing-overlapping)を参照してください。

## schedule:work

スケジューラーを、長寿命のデーモンとして実行します。最初のティックは次の分の境界に合わせられ、その後ループは `SIGINT`（Ctrl-C）または `SIGTERM` を受け取るまで、1分に一度、実行予定のタスクを評価します。シャットダウン時には、まだ実行中の `run_in_background` タスクが、書き込みの途中で取り壊されないよう、終了前に待たれます。

```bash
suprnova schedule:work
```

### 出力例

```
Starting scheduler daemon...
Press Ctrl+C to stop

==============================================
  suprnova Scheduler Daemon
==============================================
  3 task(s) registered. Press Ctrl+C to stop.
==============================================
```

各ティックは静かです - 失敗だけがログに記録されます。シャットダウン時:

```
suprnova: scheduler shutting down.
suprnova: waiting for 1 background task(s) to finish…

Scheduler daemon stopped.
```

### ユースケース

- **開発。** crontabは不要です - ターミナルでデーモンを起動し、ティックするのを見てください。
- **Docker。** 1つのイメージにスケジューラーの役割を担わせたい場合、コンテナのメインプロセスとして使ってください。
- **Systemd。** 長時間実行のユニットとして管理してください（下記の[systemdユニット](#systemdユニット)を参照）。

### systemdユニット

```ini
# /etc/systemd/system/myapp-scheduler.service
[Unit]
Description=MyApp Scheduler
After=network.target

[Service]
Type=simple
User=www-data
WorkingDirectory=/path/to/your/project
ExecStart=/usr/local/bin/myapp schedule:work
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable myapp-scheduler
sudo systemctl start myapp-scheduler
```

`Restart=always` は、デーモンがクラッシュした場合にそれを復帰させます。`RestartSec=5` はクラッシュループをデバウンスします。フレームワークのパニック境界がパニックするタスクを捕まえて `FrameworkError` に変換するため、1つの不良なタスクがデーモンをクラッシュさせるべきではありません - `Restart=always` は、稀なプロセス全体の障害（OOM、親プロセスによるkill）のためのものです。

## schedule:list

登録されているすべてのタスクを、そのcron式、次回の実行時刻、説明とともに出力します。

```bash
suprnova schedule:list
suprnova schedule:list --timezone=Asia/Tokyo
```

### 出力例

```
Registered scheduled tasks:
  cleanup:logs [0 3 * * *] next: 2026-05-29 03:00 UTC
  send:reminders [0 9 * * *] next: 2026-05-28 09:00 UTC
  heartbeat [* * * * *] next: 2026-05-28 12:01 UTC
  report:generate [0 6 * * *] (UTC) next: 2026-05-29 06:00 UTC
```

ビルダーに `.description(...)` をチェーンしたタスクは、次回の実行時刻のあとに説明を含みます。説明のないタスクは、cronと次回の実行だけを表示します。

`next:` は、式が一致する現在時刻より後の最初の分です。決して一致し得ない式は `next: never` と出力します。時刻は、`--timezone` が別のIANAゾーンを指定しない限りUTCで表示され、未知のゾーン名は何も出力される前にエラーで終了します。

`.timezone(...)` で自身のゾーンを固定したタスクは、その式が一覧のゾーンへ書き換えられ、そのゾーンのラベルが付きます - 上記の `report:generate` は `02:00 America/New_York` を要求していました。ゾーンを固定していないタスクは書かれたとおりに出力され、ラベルは付きません。書き換えが拒否される場合や、1つのタスクが複数行を占め得る場合を含む、タイムゾーンの規則の全容は[タスク スケジューリング](scheduling.md)を参照してください。

何も登録されていないとき（`.schedule(...)` のビルダー呼び出しが欠けている、あるいは `schedule::register` が何もしない場合）:

```
No scheduled tasks registered.
Define tasks in src/schedule.rs and wire it with `Application::schedule(schedule::register)`.
```

## タスクを生成する

フレームワークは、タスクを作成し、プロジェクトに配線し、スケジューラーの呼び出しを `main.rs` に追加する、ジェネレーターを出荷します:

```bash
suprnova make:task CleanupLogs
```

これは次を行います:

1. `src/tasks/cleanup_logs_task.rs`（自分自身の所要時間をログに記録する、動作する `Task` のスタブ）を作成する
2. `src/tasks/mod.rs`（`CleanupLogsTask` を再エクスポートする）が、まだ存在しなければ作成する
3. `src/schedule.rs`（`register(&mut Schedule)` 関数を持つ）が、まだ存在しなければ作成する
4. `src/lib.rs` に `pub mod schedule;` と `pub mod tasks;` を宣言する
5. `cmd/main.rs`（あるいはAPIスターターでは `src/main.rs`）の中の `Application` チェーンに `.schedule(<crate>::schedule::register)` を追加する

ステップ2〜5はべき等であるため、`make:task` を再実行すれば、手動で取り除かれた配線を修復できます。より広い `make:*` ファミリーについては、[コード ジェネレーター](cli-generators.md)を参照してください。

生成した後、`src/schedule.rs` の中でタスクを登録してください:

```rust
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

フルーエントなビルダーAPI（`.daily()`、`.cron(...)`、`.without_overlapping()`、`.run_in_background()`、曜日指定の修飾子）は、[タスク スケジューリング](scheduling.md)で完全にカバーされています。

## 終了コード

| コマンド | ゼロで終了 | 非ゼロで終了 |
|---|---|---|
| `schedule:run` | 実行予定のタスクがすべて `Ok(())` を返した、あるいは実行予定のタスクが何もなかった | 少なくとも1つのタスクが `Err(_)` を返したか、パニックした |
| `schedule:work` | `SIGINT` / `SIGTERM` 経由のきれいなシャットダウン（ラッパーは終了コード130をきれいなCtrl-Cとして扱う） | ブートストラップの失敗、あるいはデーモンプロセスが中止した |
| `schedule:list` | 一覧の表示が成功した（「タスクが登録されていません」というメッセージを含む） | アプリケーションが起動に失敗した |

`schedule:work` の内側でのバックグラウンドタスクの失敗は、stderrにログ記録されますが、デーモンを終了させません - `JoinSet` の `catch_unwind` 境界がそれらを `FrameworkError` として表面化させ、ティックループは継続します。

### Suprnovaが異なる設計を選んだ理由

Laravelの `schedule:run` が唯一のファーストクラスなエントリーポイントであり、デーモン形式（`schedule:work`）は、crontabのないホストのためのバックポートです。PHPには長寿命のプロセスがないため、毎分がまっさらなランタイムであり、フレームワーク、コンテナ、そしてすべてのサービスバインディングを再起動しなければなりません。

Suprnovaでは、デーモンはファーストクラスです。`schedule:work` は、HTTPを提供するのと同じTokioランタイムの内側で実行されます。そのため:

- **バックグラウンドタスクはティックループと合成されます。** `.run_in_background()` タスクは `JoinSet` へ生成され、ループは次のティックの前に完了したものをポーリングし、シャットダウン時に残りをドレインします。Laravelは、バックグラウンドタスクごとに子プロセスを生成します。
- **グレースフルシャットダウンは、実行中の作業をドレインします。** Ctrl-C / SIGTERMは、インラインのタスクに現在の呼び出しを完了させ、終了前にすべてのバックグラウンドの生成を待ちます。Laravelは、cronの子プロセスをkillするのをOSに委ねます。
- **起動コストは一度だけ払われます。** コンテナ、ドライバー、そしてあなたの `bootstrap_fn` は、デーモンの起動時に起動し、ティックごとではありません。`schedule:run` は、それでも呼び出しごとに起動コストを払います（それは単発のサブコマンドです）が、デーモンの経路こそが、ランタイムモデルが報われる場所です。

`schedule:run` は、それでも機能します（そして、システムcronが既にオペレーターの信頼できる情報源であるなら、正しい選択です）。あなたのデプロイの形に合う方を選んでください - どちらも同じタスク定義を共有します。

## 次のステップ

- [タスク スケジューリング](scheduling.md) - `Task` トレイト、フルーエントなcron API、`without_overlapping`、`run_in_background`、そして同一分の重複排除
- [コード ジェネレーター](cli-generators.md) - `make:task` を含む、完全な `make:*` ファミリー
- [コンソール](console.md) - スケジュールに乗らない、ワンショットの運用者向けタスクのための `#[command]` アノテーション
- [キュー](queues.md) - 時計でティックするのではなく、ワーカーが拾い上げるべき作業のために
- [アプリケーション ブートストラップ](bootstrap.md) - `.schedule(...)` がビルダーにどう組み込まれ、タスクがコンテナから何を解決できるか
