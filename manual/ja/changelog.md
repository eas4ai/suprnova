# 変更履歴

Suprnovaで何が変わったかを、バージョンごとに読みやすくまとめたログです。各バージョンのセクションは、そのバージョンのリリース記録です。バージョンは、バージョンコミットと対応する`v<version>`タグがアトミックにプッシュされたときにリリースされます。新しい順に並んでいます。

## 1.3.4 - 2026-08-25

### 追加

- **リードスルーディスクが `copy` フラグを取り、フォールバックをまたいで `copy` / `rename` を解決します。** `ReadThroughConfig` に `copy: false` を設定すると、フォールバックでのヒットを書き抜くことなく提供します。これはディスクを透過的なオーバーレイに変え、各取得を、あなたが要求した範囲へ絞り込みます。`copy` と `rename` は、フォールバックにしか存在しないソースを、プライマリの行き先までストリームで渡すようになりました。`rename` はフォールバック側のソースも削除するため、後の読み取りが、移動したオブジェクトを蘇らせることはできません。条件はそのストリーミングの経路をまたいで運ばれます: `if_not_exists` は既存の行き先を変わらず拒否し、コピーのソースバージョンは、フォールバックがどのオブジェクトを引き渡すかを選び、コピーの `if_match` は、黙って落とされるのではなく `Unsupported` で拒否されます。途中で失敗した転送は、自分が作った行き先だけを取り除くため、もともとそこにあったオブジェクトを壊すことはできません。
- **デバウンスされるジョブと、デバウンスされる、キューに入れられたリスナー。** `Job::debounce_for()` は、ディスパッチのバーストを、直近のディスパッチからウィンドウ1つ分の後に走る1回の実行へ畳み込み、最新のペイロードを運びます。これは、最初のディスパッチを残して残りを抑制する `push_unique` の鏡像です。`Job::max_debounce_wait()` は、途切れないバーストが作業を永遠に先送りしてしまうのを止め、`Job::debounce_id(&self)` はウィンドウをエンティティごとにスコープするため、1つの注文への20回の更新は、別の注文のものに触れることなく畳み込まれます。`Queue::push_debounced(job, DebounceOptions)` は呼び出し箇所でウィンドウを設定し、`DebouncedListener::new(window, build).keyed_by(...)` は、イベントから導出したキーでイベントリスナーをデバウンスします - 素の `QueuedListener` は、ジョブ自身が宣言したウィンドウをすでに尊重します。あらゆるディスパッチは、それでもenqueueされます。畳み込みはワーカーで決着し、ワーカーは追い越されたエンベロープをackして `JobDebounced` を発行します。デバウンスはフェイルオープンします: 期限切れになった、あるいは追い出されたウィンドウは、ジョブを落とすのではなく実行します。実際の実行はそのたびに新しい最大待ちのウィンドウを始めるため、バーストは常に、前のバーストのものを引き継ぐのではなく、自分自身の最初のディスパッチから最大待ち時間を測ります。1つのジョブが `debounce_for` と `unique_id` の両方を宣言することはできず、チェーンとバッチはデバウンスされるジョブを拒否します - 追い越されたリンクは、そのチェーンの残りを取り残すことになり、追い越されたバッチのジョブは、バッチの保留カウントを永遠にゼロより上に残すことになるからです。エンベロープはこのために2つの追加のみのフィールドを運びますが、デバウンスされないすべてのプッシュにとって、ワイヤの上ではバイト単位で同一のままです。
- **`Storage::register_read_through` が、2つのディスクをリードスルーディスクへ合成します。** 読み取りとメタデータは、まずプライマリに対して解決され、2つ目のディスクへフォールバックします。フォールバックで見つかったものはプライマリへ書き抜かれるため、ストアの移行が実トラフィックの下で完了します。書き込みと一覧はプライマリに留まり、削除は両方のディスクからオブジェクトを取り除きます。昇格の失敗が、フォールバックの読み取りへ劣化するのではなく表面化しなければならないときは、`throw_on_promotion_failure` を設定してください。昇格はアトミックに公開されるため、書きかけのオブジェクトが読み手に見えることはなく、フォールバックのオブジェクトのコンテンツタイプ、キャッシュ制御、コンテンツディスポジション、コンテンツエンコーディング、そしてユーザーメタデータを一緒に運びます。バージョン付き、あるいは条件付きの読み取りは、その条件を保ったまま通され、昇格されることなく提供されます。
- **`Queue::forward` が、キュー全体を名前でリダイレクトします。** `Queue::route` がジョブの型でキー付けされるのに対し、`Queue::forward("default", "high")` はキュー名でキー付けされます - プールを引退させる、バックログを吸収する、これから落とそうとしているプールから作業を移す、といったことを、ジョブもルートも1つとして触ることなく行うためのレバーです。これは両側に適用されます: `default` に解決された新しいプッシュは `high` に着地し、*そして* `--queue=default` で起動したワーカーが `high` をドレインするため、行き先が、誰も確保しない作業を集めてしまうことはありません。`default` を転送すると、キューを名指ししなかったジョブが捕まります。転送は1回の引き当てであって決して連鎖ではないため、入れ替え（`a -> b` に加えて `b -> a` も登録されている状態）やより長いローテーションは、ループではなく、筋の通ったプールの交換になります - リゾルバーがこれと同じ1回の引き当てであるLaravelと、まったく同じです。一時停止は、今もワーカーが起動時に与えられた名前の上で評価されるため、`Queue::pause(&connection, "default")` は、`default` が転送されている間もそのワーカーを止めます。`Queue::forward_on(from, to, connection)` は、転送を1つのコネクション名に制限します。これは、ジョブが宣言したコネクションではなく、このプロセスのコネクション名と比較されるため、リダイレクトの両側が同じ値でゲートされます。`Queue::forward_for(from)` は転送を読み戻し、`Queue::try_forward` は失敗し得る兄弟です。検査の呼び出し（`Queue::pending_jobs` とその兄弟たち）は、意図的に転送に追随しません。そのため、転送されたキューに取り残されたバックログは、見えたままになります。
- **読み取り形のRedisコマンドが、一時的な失敗を表面化させるのではなくリトライします。** コネクションマネージャーは既にバックグラウンドで再接続していましたが、死んだソケットに当たったコマンドは、それでもあなたの呼び出しを失敗させていました。`GET`、`EXISTS`、`Cache::flush` / `Cache::flush_tags` の背後にある `SCAN` と `SSCAN` のページ、キューのドライバーの `XLEN` / `ZCARD` / `XPENDING` の読み取り、そしてレートリミッターの `Retry-After` の計算は、短い休止の後に一度リトライするようになりました。`REDIS_COMMAND_RETRIES` はその上にさらにリトライを積み、10でクランプされます。リトライの見積もりは、ミリ秒ではなく秒の単位で立ててください: 2回目の試行は代わりのコネクションを待つため、ドライバーのコネクトとレスポンスの予算の全体を費やしますし、タイムアウトしたコマンドも、切断されたコマンドと同じく一時的なものと数えられます。書き込みは、どの設定であっても決してリトライしません: 一時的なエラーが意味するのは、コネクションが失敗したということであって、サーバーがそのコマンドを拒んだということではないため、`SET`、`INCR`、ロックの獲得、レートリミットのヒット、あるいはキューのポップを繰り返せば、それが2回実行されうるからです。エラーメッセージは変わっていないため、それに対してマッチしているものは引き続き動作します。
- **停止されたワーカーが、自分は停止していると告げるようになりました。** `queue:work` は遷移ごとに1行を出力し - `2026-08-25 14:03:11 Queue billing PAUSED`、そして復帰するときには `RESUMED` です - ワーカーは `WorkerQueuePaused` / `WorkerQueueResumed` を発行するため、同じシグナルをあなた自身のアラートへルーティングできます。これらはワーカー側の対です。既存の `QueuePaused` / `QueueResumed` は、`queue:pause` を実行したプロセスがどれであれ、そのプロセスで発火し、それがワーカーであることは決してないため、これまでは、誰かが自分のキューを停止したせいで静かになったワーカーは、ハングしたワーカーと見分けがつきませんでした。各イベントは、ポーリングごとにではなく、遷移ごとに一度発火します。それらの `queue` フィールドは省略可能です: `--queue` なしで起動したワーカーはすべてをドレインし、`pause_all` のもとで報告すべきキュー名を持たないため、リスナーがマッチできてしまう名前をでっち上げるのではなく、`None` を報告します。
- **`?include=` のパスが5セグメントで上限を定められ、`max_relationship_depth` がその天井を動かします。** 循環するリレーションのグラフは、`?include=author.posts.author.posts...` を、クエリ文字列だけを境界とする、クライアントが制御するファンアウトへ変えてしまいます。パスは、パースされる途中で切り詰められるようになりました。上限を変えるには `bootstrap::register()` で `suprnova::max_relationship_depth(n)` を呼び、includeを切るには `0` を渡してください。
- **`Gt`、`Gte`、`Lt`、`Lte` が、あるフィールドを数値と、あるいは別のフィールドと比較します。** `CompareWith` が、オペランドと尺度を1つの値の中で名指しします: リテラルには `Number`、数値の兄弟フィールドには `NumericField`、文字数で比較される兄弟フィールドには `LengthField` です。ルールが測れないオペランドは、パニックするのではなく、そのフィールドを失敗させます。
- **3つのメンバーシップのルールが組み込みの集合に加わりました: `InArray`、`Contains`、`DoesntContain` です。** `InArray` は、ある値を別のフィールドのリストに対して検査します。フィールドをルールの文字列で名指しするのではなく、リストを直接渡します。`Contains` と `DoesntContain` はJSONの配列の上で走り、パラメータを文字列の要素とだけ照合するため、`1` と `"1"` は別のままです。
- **データベースのプールが、生存性のノブを持つようになりました。** `DB_IDLE_TIMEOUT`、`DB_MAX_LIFETIME`、`DB_ACQUIRE_TIMEOUT`、`DB_TEST_BEFORE_ACQUIRE`、`DB_PING_AFTER_IDLE` が、プールがコネクションを閉じる、作り直す、pingを打つタイミングを制御し、対応する `DatabaseConfig::builder()` のセッターがあります。どれもデフォルトでは未設定であるため、既存のデプロイのプールは、これまでとまったく同じように振る舞います。NATゲートウェイやファイアウォールがアイドルのコネクションを落とすときに使ってください: sqlxはlibpqの `keepalives_*` に相当するものを公開していないため、プールの作り直しがその仕組みです。
- **`db:seed <Class>` が、その進捗を報告します。** 対象を絞った実行は、シーダーの前に `RUNNING` の行を、その後に経過ミリ秒を伴う `DONE` の行を出力します。素の `db:seed` は静かなままです。そのフォーマッター `suprnova::two_column_detail` は、あなた自身の `#[command]` のハンドラからも使えます。
- **多対多のリレーションが、ピボットのカラムでフィルタできるようになりました。** `where_pivot`、`where_pivot_op`、`where_pivot_in`、`where_pivot_not_in`、`where_pivot_null`、`where_pivot_not_null`、`where_pivot_between`、`where_pivot_not_between`、`where_pivot_group`、そしてそれらの `or_` の双子が、`BelongsToMany`、`MorphToMany`、`MorphedByMany` の上の `get`、`first`、`count` を制約します。`where_pivot_group` はクロージャを取り、括弧でくくられた1つのグループを描画するため、続く `or_where_pivot` の内側でもアトミックなまま保たれます。ピボットのフィルターは読み取りにのみ適用されます: `attach`、`attach_with`、`detach`、`sync` は、フィルターが1つでも設定されている間はエラーを返し、イーガーロードはそれらを運びません。
- **`where_binary` が、カラムの値をバイト単位で比較します。** この一族（`where_binary`、`or_where_binary`、`where_not_binary`、`or_where_not_binary`）は `Builder<M>` の上に出荷され、`where_binary` と `where_not_binary` は `DB::table(...)` の上にも出荷されます。MySQLとMariaDBは `= binary` を発します。PostgresとSQLiteは、照合順序に依存したマッチへフォールバックするのではなく、クエリがレンダリングされる時点でエラーを返します。
- **`Builder::try_to_sql_with_bindings_for` が、パニックすることなく、あるダイアレクト向けのSQLを描画します。** これは `to_sql_with_bindings_for` の失敗し得る兄弟であり、ビルダーがあるバックエンド向けに正当に描画できないケースのためのものです。
- **`Model::refresh_for_update` が、`FOR UPDATE` のロックの下で行を再読み込みします。** 行の現在の状態と排他ロックを1つの文で必要とするときに、トランザクションの内側で呼び出してください。SQLiteには行レベルのロックがないため、そこではロック句はno-opです。
- **`Builder::or_where_key` と `Builder::or_where_key_not` が、主キーのフィルタを論理和として加えます。** どちらも、`or_where` と同じやり方で直前の `WHERE` 句へ畳み込まれ、どちらも `or_filter_key` と `or_filter_key_not` のエイリアスを出荷します。
- **`Builder::in_order_of` が、行を明示的な順序へ並べます。** カラムと、あなたが望む順序に並べた値を渡してください。値がリストにない行は、最後に並びます。値はパラメータとしてバインドされるため、リクエストのデータから取っても安全です。

### 修正

- **メンテナンスのバイパスクッキーが、サーバー側で期限切れになるようになりました。** 12時間のTTLは、ブラウザーが強制する `max-age` だったため、盗まれたクッキーは、あなたがシークレットをローテーションするまで効き続けていました。暗号化されたペイロードが期限を運ぶようになり、リクエストごとにそれを再検査します。
- **`suprnova serve` が、フロントエンドのないプロジェクトを走らせます。** `suprnova new --api` でスキャフォルドされたプロジェクトには `frontend/` ディレクトリがなく、`--backend-only` を渡さないかぎり、`serve` はそれを「No frontend directory found. Are you in a Suprnova project directory?」として拒否していました。今は、Viteのペインと、それに材料を供給するTypeScriptの生成をスキップし、バックエンドを配信します。そのようなプロジェクトでは `--frontend-only` は依然として失敗しますが、その理由を告げるメッセージを伴います。

### アップグレード

- **このリリースより前に発行されたバイパスクッキーは、機能しなくなります。** クッキーのペイロードは、素のシークレットから、封じられた `{ secret, expires_at }` のオブジェクトへ変わり、期限を持たないペイロードは拒否されます。アップグレードの後、新しいクッキーを得るために、シークレットのURLを一度訪れてください。ほかには何も変わりません: `down`、`up`、`--secret`、`--with-secret` は、すべて以前と同じように振る舞います。
- **5セグメントより長いincludeのパスは、そのすべてではなく、最初の5つのリレーションを返すようになりました。** リソースの許可リストの外側にあるものが到達可能だったことは一度もないため、レスポンスがデータを得ることはありません。深いパスは、その末尾を失います。それに伴ってステータスコードが1つ変わります: 深すぎる末尾が、そのリソースが許していないリレーションを名指ししているパスは、何かがバリデーションされるより前に切り詰められるため、完全なパスが以前は `400` を返していたところで、生き延びたセグメントとともに `200` を返すようになりました - その拒否をアサートしているクライアントやテストがあれば、調整してください。あなたのAPIがそれより長いパスをドキュメント化しているのなら、`suprnova::max_relationship_depth(n)` で天井を上げてください。
- **`DatabaseConfig` が、5つの公開フィールドを得ました。** 構造体リテラルでそれを組み立てているコードは、もうコンパイルできません。`DatabaseConfig::from_env()` か `DatabaseConfig::builder()` を使ってください。どちらも、今日のプールの振る舞いを保つデフォルトで、新しいフィールドを埋めます。

## 1.3.3 - 2026-08-25

### 追加

- **フェイルオーバーのキュー接続。** `FailoverQueueDriver` は、順序付きの接続リストをラップします。最初の接続が拒否したプッシュは次で再試行され、以下同様にリストを落ちていきます。環境変数から `QUEUE_DRIVER=failover` に加えて `QUEUE_FAILOVER_CONNECTIONS=redis,database` で配線するか（各エントリはそれ自身のドライバーの変数を読むため、`database` のエントリは今も先に `DB::init()` を必要とし、今も自身の失敗ジョブのストアを連れてきます）、`FailoverQueueDriver::new(vec![(label, driver), ...])` で直接組み立ててください。落ちていくのは書き込みだけです: `push` と `bulk_push` はリストを歩きますが、`pop`、`pop_from`、`ack`、`nack`、`release`、`settle`、`clear`、4つのカウンターすべてと3つの検査の一覧はすべて、最初の接続だけに委譲します。予約のトークンは、それを発行したドライバーにとってしか意味を持たないからです。その運用上の帰結は、覆い隠すのではなく文書化されています: フェイルオーバー接続の上のワーカーはプライマリだけをドレインするため、フォールバックへフェイルオーバーしたものには、それ自身のワーカーが必要です。`bulk_push` はバッチを転送するのではなく各エンベロープを個別にプッシュします。これは、各エンベロープ自身の `available_at` を保つと同時に（Laravel #60950）、プライマリが半分だけ受け入れたバッチが、まるごとフォールバックへ再プッシュされるのを防ぎます。拒否は `queue::events::QueueFailedOver { connection, job_name, exception }` をディスパッチします。これはエッジトリガーです: 接続は失敗状態に入るときに一度だけ自身を報告し、後のプッシュがそこで成功して再び武装させるまで静かなままであるため、障害は、ディスパッチごとに1つではなく、全体で1つのアラートを生みます。すべての接続が拒否したときは、プッシュは最後の接続のエラーを返します。空の接続リスト、`QUEUE_FAILOVER_CONNECTIONS` が欠けているか空白であること、ネストした `failover` のエントリ、存在しないドライバーを指すエントリは、いずれも起動エラーです - 警告してメモリへフォールバックする振る舞いは `QUEUE_DRIVER` 自体に留まります。そこでは、タイプミスが揮発性のバックエンドを永続的な連なりへ継ぎ足すことはできません。
- **キューの検査API。** `Queue::pending_jobs(queue)` / `delayed_jobs` / `reserved_jobs` は、既存の `pending_size`/`delayed_size`/`reserved_size` のカウンターの背後にある実際のエンベロープを、`InspectedJob` のDTO（`id`、`queue`、`name`、`attempts`、`payload`、`created_at`）として一覧します - Laravelの `InspectedJob` を映しています。1つの `Option<&str>` のキューのフィルターが、Laravelの `pendingJobs($queue)` / `allPendingJobs()` の組（および `delayedJobs`/`reservedJobs` の相当物）を、それぞれ1回の呼び出しへ畳み込みます。`QueueDriver` トレイトのデフォルトは誠実な `Err` です - Laravelの Beanstalkd/SQS の空コレクションのデフォルトは、明らかにキューがあるときでも「何も入っていない」と読めてしまいます - そのため、検査を実装していないドライバーはそうだと言います。`sync`/`null` は `Ok(vec![])` で上書きします。それらにとっては、それが本当に真実だからです。memory、database、Redisのドライバーはいずれも、完全な一覧を実装します: memoryドライバーの遅延ストアは、素の `DelayQueue<Envelope>`（イテレートできません）から `DelayQueue<Uuid>` と、idをキーとするマップへ移りました。databaseドライバーは、サイズのカウンターとまったく同じ述語に `ORDER BY available_at` を加えて再利用し、`envelope_json` のデコードに失敗した行も落とさずに一覧します（`id: None`、`payload: {"unparseable": true}`）。そのため、1つのポイズン行がキューの残りをオペレーターから見えなくすることはできません。Redisの `reserved_jobs` は、このコンシューマーのプロセス内の予約に限定されます（文書化済み）。`pending_jobs` は `XRANGE` でストリームをバッチごとに走査します。`Queue::fake()` は、対応する `pending_jobs()`/`delayed_jobs()` のヘルパーを得ました。記録されたプッシュを、`attempts` は常に `0`、`created_at` は常に `None` として射影します。
- **コミット後のディスパッチ。** `Job::after_commit()` は、周囲の `DB::transaction` がコミットするまでプッシュを保留します。そのため、別プロセスのワーカーが、トランザクションがまだ永続化していない行を記述したエンベロープをpopすることは決してありません。待つのはドライバーへの書き込みだけではなく、プッシュ全体です: エンベロープの構築、`JobQueueing`、`JobQueued` はすべてコミットの時点で起こるため、ロールバックが捨てることになるジョブについて、リスナーが知らされることはありません。ロールバックはプッシュをまるごと捨てます。トランザクションの外側ではプッシュは即座に起こり、これが、すべてのディスパッチ箇所が自身のコードの経路がトランザクション的かどうかを知らなくても、ジョブ型がこのオプトインを宣言できる理由です。ディスパッチごとには、`EnvelopeOverrides::after_commit` がジョブより優先されます: `Some(true)`（短縮形の `Queue::push_after_commit(job)` を伴います）はオプトインしていないジョブを先送りし、`Some(false)` はLaravelの `beforeCommit()` です。先送りされた `Queue::push` は `Job::delay()` をプッシュではなくコミットに対して再解決しますが、`Queue::push_later` / `later` / `later_with` は呼び出し側の絶対的なタイムスタンプをそのまま運びます。`Queue::push_unique` は、エンベロープが先送りされる場合でも重複排除のロックを即座に取るため、同じトランザクションの内側の重複はやはり抑制され、ロールバックはそのロックを所有者スコープで解放します。`Queue::bulk` は1つの単位として先送りします。`Queue::fake()` は、Laravelの `Bus::fake` と同じく、先送りも含めてプッシュを即座に記録します。手動の `DB::begin_transaction` は決して先送りしません - アンビエントなトランザクションをインストールしないため、コールバックをぶら下げるコミットが存在しないからです。コミットが着地しないまま終わるすべての終わり方は、データベースが拒否した `COMMIT` や、コミットを妨げる漏れた `TxHandle` も含めて、同じように補償します。そして `Transaction::rollback_to` は、それが巻き戻すスコープにとってのその1つとして数えられます: セーブポイントの内側で先送りされたプッシュは、そのセーブポイントがロールバックされたときに捨てられ、そのロックはその場で解放されます。一方、セーブポイントより前に登録されたものは手つかずです。キューに入れられたメール、通知、バッチ、チェーンは、まだ先送りしません。
- **処理開始まで一意なジョブ。** `Job::unique_until_processing()` は、処理が始まるとき - ジョブのミドルウェアの通過の後、ハンドラが走る直前 - に一意性のロックを解放します。`unique_for` の窓の全体にわたって保持するのではありません。これは、ロックが実行を直列化するためではなく、キューに入った重複をまとめるために存在するときに、あなたが望むものです。ミドルウェアがキューへ戻したジョブは、まだ処理を始めていないため、そのロックを保ちます。ミドルウェアが削除した、あるいはデッドレターに送ったジョブは、そのロックを手放します。解放は所有者スコープです: `Queue::push_unique` はキャッシュのロックの所有者トークンをエンベロープに記録し（`Envelope::unique_lock_owner`。追加のみのフィールドであり、一意でないすべてのプッシュにとって、凍結されたワイヤ形式はバイト単位で同一のままです）、ワーカーはそのトークンで解放するため、再配送された試行が、より新しいディスパッチが今保持しているロックを強制解放することは決してできません。それを支えるべき等性の表面も公開されています: `Idempotency::commit_on_success_owned` は本体にロックの所有者を手渡してそれを返し、`Idempotency::release_owned(key, owner)` は所有者スコープで解放し、ロックが存在しない場合やほかの誰かが保持している場合は、エラーではなく `Ok(false)` を報告します。素の `unique_id` のジョブは変わらず、今も `unique_for` のTTLを重複排除の窓にできます。
- **`Gate::default_denial_response` が、素の拒否のデフォルトの形をカスタマイズします。** Laravelの `Gate::defaultDenialResponse($response)` を映しています。一度だけ設定すれば - 通常は `bootstrap::register()` で - ちょうど2つの結果を作り変えます: 素の `false`（boolのゲート - `Gate::define` / `Gate::define_async`。`bool` を返す `#[policy]` のメソッドを含みます - あるいは `false` と判定した `before`/`after` フック）と、ほかに何も判定しなかった評価（フックの意見もない未定義の権限）です。これらはすべて、以前は素の `Response::deny()`（403）へ収束していました。今は、そのデフォルトが運ぶ `Response` が何であれ、それとして表面化します。例えば `Response::deny_as_not_found()` なら、ゲートごとにではなくアプリケーション全体でリソースの存在を隠す404です。デフォルトが適用されるのは素の `false` だけです - `define_with` / `define_async_with` で登録されたゲートは、望む `Response` をすでに返しており、それは常に `Gate::inspect` を手つかずで通過します。これは、デフォルトが返された `Response` オブジェクトの代わりになることは決してないという、Laravel自身の規則と一致します。allowの形をした `Response::allow()` をデフォルトにすると、boolのゲートをすべて黙って許可へ反転させるのではなく、拒否されます（ログに記録され、無視されます） - これが意図的にLaravelから分岐する唯一の点については、`Gate::default_denial_response` のドキュメントコメントを参照してください。Laravelにはそうした保護機構がありません。
- **`Password` のバリデーションルールのファミリーが、Have I Been Pwned の `uncompromised()` チェックを含めて出荷されます。** `Password::min(n)` と強度のビルダー（`.max()`、`.letters()`、`.mixed_case()`、`.numbers()`、`.symbols()`）は、Laravelの `Password` ルールの正規表現をそのまま移植しています - 素の空白は `.symbols()` を満たし、Laravelの `\p{Z}` の区切り文字クラスと一致します。`.uncompromised()`（または `.uncompromised_with_threshold(n)`）は、Have I Been Pwned のk匿名性のレンジAPIに対してパスワードを照合します: プロセスの外へ出るのはパスワードのSHA-1ハッシュの最初の5文字だけであり、ネットワークの失敗、タイムアウト、2xx以外のレスポンスは、Laravelの `NotPwnedVerifier` とまったく同じく、サインアップを止めるのではなくフェイルオープンします。このチェックはHTTPの往復であるため、`Password` は `Rule`（強度のみ。同期の `validate!` の行のため）と `AsyncRule`（強度に続いてHIBPのチェック。`after_validation_async` のため）の両方を実装する唯一の組み込みルールです - `uncompromised()` が設定された `Password` に対して同期の経路を呼び出すことは、黙って飛ばされるのではなく、はっきりとした開発者向けのエラーになります。`Password::defaults_with(...)` は、`Password::defaults()` が返すプロセス全体のデフォルトを設定します。新しい `HIBP_TIMEOUT_SECS` 環境変数（デフォルトは30秒）。`Http::fake_response_text(...)` は、HIBPのような `text/plain` の上流APIに対するテストのための、`fake_response(...)` の生ボディ版の兄弟です。
- **スケジュールされたタスクが、自身のcron式を読むタイムゾーンを名指しできるようになり、`schedule:list` がスケジュール全体を任意のゾーンでレンダリングできるようになりました。** `.timezone(chrono_tz::Tz)` は1つのタスクを固定し、`.try_timezone("Area/City")` は実行時にしか存在しないゾーン名のための失敗し得る兄弟であり、`Schedule::timezone(tz)` はそれ以降に登録されるすべてのタスクのデフォルトを設定します。ゾーンを固定しないタスクについては何も変わりません: 今もプロセスのローカルゾーンに対して評価されます。固定されたゾーンが影響するのは実行予定かどうかだけです - スケジューラーは今もプロセスの1分に一度ティックし、同一分の重複排除ゲートは手つかずです。なお、夏時間を採用するゾーンでは、壁時計上のある分が2回起こったり、まったく起こらなかったりするため、そうした分に固定されたタスクは2回走ったり、飛ばされたりすることがあります。スケジューリングの章が、完全な警告を載せています。`schedule:list` は `--timezone` オプションと2つのカラムを得ました: 出力される式が書かれているゾーンと、そのタスクが次に発火する分です。固定されたタスクの式は一覧のゾーンへ書き換えられ、そこで真夜中をまたぐ場合は複数行に分かれます。そして、忠実な書き換えが不可能なとき - 夏時間の切り替わりをまたぐとき、日付のまたぎが、制限された日と制限された曜日を一緒に動かさなければならなくなるとき、あるいは2月の長さを決めなければならなくなるとき - は、書かれたとおりに残されます。`chrono_tz::Tz` はクレートルートから再エクスポートされているため、消費するアプリが自身の `Cargo.toml` に `chrono-tz` を加えることはありません。
- **Laravelの形をした画像サブシステム。デフォルトで有効な `media` フィーチャーの背後、`suprnova::media` にあります。** `Image::from_bytes/from_path/from_disk/from_upload/from_stream` がレイジーなパイプラインを組み立て - `resize`、`scale`、`crop`、`cover`、`contain`、任意の角度の `rotate`、`flip_vertically`/`flip_horizontally`、`blur`、`sharpen`、`grayscale`、`to_format`、`quality` - `to_bytes`、`to_response`、`save`、`store`、`dimensions`、`mime_type`、`dominant_color` で仕上げます。PNG、JPEG、WebP、GIF、BMPを読み書きします。AVIFの出力は、自社製のAV1エンコーダーが公開されるまで先送りされており、その時点で新しい `OutputFormat` のバリアントが1つ増えるだけで、ほかには何も変わりません。Laravelの `gd`/`imagick` の分かれ方と同じく、2つのドライバーがあります: `IMAGE_DRIVER=oxideav`（デフォルト）は、ネイティブライブラリもインストールするものもない純粋なRustの [OxideAV](https://github.com/OxideAV) のコーデックファミリーの上で走り、`IMAGE_DRIVER=magick` は、HEICを含むより広い入力サポートのために、ホストにインストールされたImageMagick 7へシェルアウトします。デコードの上限（`IMAGE_MAX_DIMENSION`、`IMAGE_MAX_ALLOC_BYTES`）は、何かが割り当てられる前に入力自身のヘッダーに対して検査されます - 拡張WebPの内側のビットストリームも含みます。その助言的なキャンバスのサイズを使って、より大きなフレームを密輸してゲートを通すことはできません - そして、ピクセルの作業はすべてブロッキングスレッドの上で走ります。`magick` ドライバーは、ImageMagickにバイト列からコーダーを選ばせるのではなく、入力のコーダーを名前で固定し、すべての起動を `IMAGE_MAGICK_TIMEOUT_SECS` で境界付けます。`ImageDriver` が、それ以外のあらゆるもののためのトレイトの境界です。モジュールが `media` という名前なのは、OxideAVに支えられた音声と動画の表面が、その隣に置かれることになるからです。[画像](../images.md)
- **WebPのゲートは、1つの固定された、設定できない境界を運びます。** WebPは、実際のデコード後のサイズを最も内側のビットストリームのチャンクの中で宣言するため、フレームワークはそれを見つけるためにコンテナを歩きます。その歩みはレベルあたり最大4096チャンクを訪れ、2レベルのネストをたどり、そのどちらかを超えるファイルは測られるのではなく拒否されます。終えられなかった歩みから数字を報告することは、十分な量の詰め物のチャンクを積めば回り込めるゲートになってしまいます。どの `IMAGE_MAX_*` 変数もそれに影響せず、エラーメッセージもそう述べます。300フレームのアニメーションは影響を受けず、4100フレームのものは拒否されます。[画像](../images.md#one-bound-is-not-configurable)

- **OAuthを、アプリケーションの既存のパスワードとセッションの権限元を置き換えることなくインストールできるようになりました。** `MagnetarOAuthOnlyConfig` と `init_magnetar_oauth_only` は、デフォルトのセレモニーとプロバイダーのエンジンをインストールし、パスワードとパスキーのスロットは空のままにします。既存の `users` テーブルを持つアプリケーションは、`verify_oauth_identity` を呼び、検証済みのプロバイダー subject を自分で対応付け、通常のフレームワークセッションを確立できます。

### 変更

- **`DB::transaction` が、コミットの成功後に `Err` を返せるようになりました。** コミット後のコールバックが失敗した場合です: メッセージは `after-commit callback failed (the transaction itself committed): …` と読め、クロージャの戻り値は失われますが、その書き込みは失われません。`DB::transaction_with_attempts` は、コールバック自身のメッセージがどれほどデッドロックらしく読めても、そのエラーをリトライすることは決してありません - すでに永続化された書き込みを持つクロージャを再実行すれば、それらを二重に適用してしまうからです。
- **新しいバリデーションのカタログキー: `validation-password-unverifiable`。** `Err` を返すカスタムの `UncompromisedVerifier` は、自身のエラーのテキストを422のボディへそのまま入れることがなくなりました。そのテキストは代わりに `error` でログに記録され、レスポンスはこのキーを運び、「The { $field } could not be checked against known data leaks. Please try again.」としてレンダリングされます - チェックが走らなかったことは、パスワードが悪いことと同じではなく、インフラの詳細はクライアントへのレスポンスに属しません。自身のバリデーションカタログを出荷しているアプリは、このキーを追加しなければなりません。さもなければ、そのユーザーには組み込みの英語のフォールバックが見えます。
- **`Image` のアップロードのバリデーターが `ImageFile` になりました。** `suprnova::Image` は新しい画像加工のパイプラインの型であり、`Illuminate\Image\Image` に対応します。そして、マジックバイトによるアップロードのルールは、Laravelが同じルールのクラスに与えている名前 `Illuminate\Validation\Rules\ImageFile` を取ります。移行は使用箇所ごとに1行です: `UploadedFile<(Image, MaxSize<N>)>` が `UploadedFile<(ImageFile, MaxSize<N>)>` になります。1.0以前の変動は、gitタグによる配布モデルが吸収します。

### 削除

- **使われていなかった直接の `image` 依存が消えました。** これはワークスペースのどこにも使用箇所がないベースの依存であり、JPEG、PNG、WebP、GIFのコーデックを、何の役にも立たないまま引き込んでいました。これを落としたことで、`gif`、`image-webp`、`zune-jpeg`、`color_quant`、`weezl` がツリーから消えます。このクレート自体は、`totp-rs` のQRコードのレンダリングの背後に、その `png` フィーチャーだけを伴って推移的に現れます。新しい画像サブシステムは、代わりに `media` フィーチャーの背後のOxideAVのクレートの上に構築されています。

### 修正

- **OAuthをインストールしても、プロバイダーに支えられたアプリケーションがMagnetarのウェブ束縛の検証を強いられることがなくなりました。** 完全な `init_magnetar` の経路は、原子的なまま変わりません。OAuth専用の経路は、構築の間にエンジンのスロットを予約し、OAuthだけを公開し、2つの認証の権限元を混在させるのではなく失敗します。

### アップグレード

- **`Image` は今や別の型であり、アップロードのバリデーターは `ImageFile` です。** マジックバイトによるアップロードのルールを使っている人にとっては、ソース互換性が壊れます。すべての使用箇所で改名してください: `UploadedFile<(Image, MaxSize<N>)>` が `UploadedFile<(ImageFile, MaxSize<N>)>` になります。`suprnova::Image` は今も解決できますが、それは今や画像加工のパイプラインの型であるため、改名し忘れれば、黙って振る舞いが変わるのではなくコンパイルに失敗します。
- **`EnvelopeOverrides` が、公開の `after_commit: Option<bool>` フィールドを得ました。** このリポジトリとスキャフォルドされたテンプレートの中のすべての構築は `..Default::default()` を使っており、変更は不要です。網羅的な構造体リテラルで `EnvelopeOverrides` を組み立てているコードは、新しいフィールドを名指しする必要があります。`after_commit: None` は今日の振る舞い、すなわち `Job::after_commit()` に委ねる振る舞いを保ちます。ほかには何も変わりません: `after_commit()` のデフォルトは `false` であるため、既存のジョブが、これまで待っていなかったコミットを待ち始めることはありません。
- **`Envelope` が、公開の `unique_lock_owner: Option<String>` フィールドを得ました。** ワイヤ形式は変わりません - このフィールドは `#[serde(default)]` で、`None` のときはスキップされるため、エンベロープは双方向でバイト単位に同一に往復し、`schema_version` は2のままです - が、構造体リテラルで `Envelope` を組み立てているコードは、これを名指しする必要があります。プッシュをまたいで一意性のロックを意図的に持ち越すのでなければ、`unique_lock_owner: None` を加えてください。エンベロープを読むだけのコードや、`Queue::push` とその兄弟を通じて組み立てるコードには、変更は不要です。

- アプリケーションがユーザー、パスワード、フレームワークセッション、remember-me の状態をすでに所有している場合は、`init_magnetar` の代わりに `init_magnetar_oauth_only` を使ってください。OAuth専用のコールバックは `verify_oauth_identity` を使い、完全なMagnetarのアプリケーションは引き続き `complete` を使います。

## 1.3.2 - 2026-08-25

> The v1.3.2 release notes are intentionally kept in English to preserve the complete normative record.

### 追加

- **OAuth providers can now be registered through `MagnetarConfig::oauth`.** Suprnova re-exports the `OAuthProvider` contract, all five first-party provider and configuration types, and the HTTP, revocation, abuse-limiter, authorization, and auto-link types an application needs. Custom providers no longer require a direct `suprnova-magnetar` dependency or a hand-retained `MagnetarHostEngine`.

- **A production OAuth transport and framework limiter adapter now ship at the crate root.** `ReqwestOAuthTransport` implements token, userinfo, and revocation I/O with redirects disabled by default, a 30-second timeout, a default `User-Agent`, and a 1 MiB response cap. `FrameworkAbuseLimiter` reuses the configured `RateLimiterDriver`; apps no longer hand-write either adapter.

### 修正

- **`init_magnetar` now publishes OAuth with password and passkey services as one reserved installation.** The OAuth service is built before publication, and all three engine slots remain hidden while the reservation is active. A failed or duplicate OAuth configuration cannot leave password and passkey state visible without the configured OAuth registry.

- **Custom providers can supply userinfo headers.** `OAuthProvider::userinfo_headers` is merged with the host-owned bearer header, enabling requirements such as GitHub's `User-Agent` and media-type `Accept` headers without allowing a provider to replace `Authorization`.

### アップグレード

- **The Magnetar cutover in `4faaa933` removed Torii's OAuth installation path without wiring its replacement into the default initializer.** The old workaround required constructing a custom host engine, calling `oauth_service`, and installing the adapter separately. Replace that workaround with `MagnetarConfig::from_sea_orm(database).oauth(oauth_config)` and one `init_magnetar` call.

- **GitHub community providers must handle verified email explicitly.** GitHub `/user` usually omits non-public email, while the verified primary address requires `/user/emails`. Return `email: None` to use the email-completion ceremony, or point `userinfo_endpoint` at a host adapter that combines both responses; never treat a public but unverified address as ownership.

## 1.3.1 - 2026-08-24

> The v1.3.1 release notes are intentionally kept in English to preserve the complete normative record.

### 修正

- **Provider-backed applications can reset verified users again.** When no Magnetar engine is installed, `PasswordReset` uses an explicitly reset-capable `UserProvider` and framework `auth_flow_tokens` for already verified accounts. `EloquentUserProvider<M>` opts in when `M` implements `MustVerifyEmail + CanResetPassword`; no `app_users` migration is required.
- **The published framework line now contains both post-release repair sets.** The translated 1.3.0 changelog layout and headings, CJK wrapping, localized anchors, glossary terms, and prose punctuation are reconciled instead of split across divergent local and remote branches.
- **Post-tag CLI and Magnetar hardening is included.** Development-process cleanup uses the completed process-group fallback, and the local qualification contracts cover the released refs and plugin-SDK SQLite lanes.

### セキュリティ

- **The provider fallback never treats password reset as first mailbox proof.** Unknown and unverified addresses receive the same no-mail response. Install Magnetar when an unverified account must prove mailbox ownership through reset so credential cleanup, auth-epoch advancement, and revocation remain atomic. Provider fallback completion reports framework session and remember revocation failures through `PasswordResetOutcome`.

### アップグレード

- **Move every `v1.3.0` Git dependency to `v1.3.1`.** Applications with their own `users` table keep their configured `UserProvider`; they do not initialize the default `app_users` engine merely to reset an already verified account. Applications that use Magnetar credentials or unverified-account first proof continue to initialize Magnetar.

## 1.3.0 - 2026-08-24

### セキュリティ

- **Magnetar は、認証情報およびセッションの変更を認証済みアクターとアカウント認証エポックに限定するようになりました。** パスワード、passkey、リンク済みアカウント、二要素、不透明セッション、JWT、remember、OAuth、デバイス認可の書き込みは、古いアクターまたは失効したアクターを拒否します。未確認アカウントで最初に成功したパスワードリセット、マジックリンク、または OAuth の確認済みメール証明はエポックを進め、仮の認証情報、セッション、remember 状態、および占有者による TOTP 登録を原子的に削除します。確認済みアカウントでは、パスワードリセット中も正当な認証情報が維持されます。メール確認には認証済みトークンの所有者が必要であり、OAuth はメールアドレスだけから未確認の既存アカウントを自動リンクしません。
- **プロトコル相対の `_previous.url` は、書き込み側でも読み取り側でも、もはや `Redirect::back()` を通じてオリジン外へのオープンリダイレクトを生み出せません。** `SessionMiddleware` はプロトコル相対の現在 URL を永続化しなくなりました。書き込みは `InertiaValidationRedirectMiddleware` が `Referer` の検査に使うものと同じサニタイザーを通り、`//host` の形のリクエストパス（または ASCII 制御バイトを含むもの）は決して記録されません。これがなければ、アプリの `fallback!` ルート（未一致のパスにすべて `200` を返す、標準的な Inertia/SPA のアプリシェルの形）は `GET //evil.test/anything` によりそのパスをそのまま永続化し得ました。`SessionData::previous_url()` もすべての**読み取り**で同じ検査を適用するため、この修正より前のリリースから残った、現在のプロセスが決して書き出していない未サニタイズ値を持つセッションクッキーは、信頼されず「何も記録されていない」状態へ自己修復します。これにより、古い汚染済みクッキーも新しい悪意あるリクエストも、`Redirect::back()`、`Redirect::refresh()`、`url::previous()` にオリジン外の `Location` を渡せません。どちらかの検査に失敗した値は合成値で置換されず欠落として扱われるため、本当に正しい以前の URL が上書きされることもありません。
- **Inertia のバリデーションリダイレクトブリッジの `Referer` 検査は、さらに 2 つの同一オリジン回避を閉じました。** `InertiaValidationRedirectMiddleware` の `303` 宛先は、文字どおりの `//` または `/\` 接頭辞で始まる `Referer` だけを拒否していました。`Referer: /<TAB>/evil.test` のような値は通過しましたが、WHATWG URL パーサーはオリジン比較前に文字列全体から ASCII タブと改行を除去するため、ブラウザはそれを `//evil.test` と読み、`303` をオリジン外へ追従します。検査は、2 つの名前付き接頭辞だけでなく候補のどこにあっても ASCII 制御バイト（C0 または DEL）を拒否するようになりました。さらに、`Referer` もセッションの以前の URL も使えないときに使う、失敗したリクエスト自身のパスという最後の手段はサニタイズされていませんでした。オリジン形式の HTTP リクエストターゲットは構文上 `//` で始められるため、生のクライアントまたは正規化しないプロキシが「安全な最後の手段」をオリジン外リダイレクトに変えられました。両方の経路は同じルート相対検査を共有し、リクエスト自身のパスさえ失敗した場合は `/` にフォールバックします。
- **Cookie 暗号文は、コンテキスト付き v2 AAD により論理的なクッキー名へ束縛されるようになりました。** `Cookie::encrypted` / `Cookie::read_encrypted_for` は、あるクッキースロットで発行された値が別のスロットで復号されることを止めます。また論理名への束縛により、後からの `__Host-` / `__Secure-` レスポンス接頭辞の変更も安全に保たれます。バージョンなしの互換ウィンドウは、キーリング全体で v2 を試した後にキーリング全体で v1 を試すため、既存クッキーはロールアウトを生き残ります。v1 フォールバックは、予定された 1.4.0 の削除までは古いリプレイ弱点を残します。
- **セッションおよび remember-me クッキーの接頭辞は、起動時に検証され、レンダリング時に強制されます。** `SESSION_COOKIE_PREFIX=__Host-` には `Secure`、`Path=/`、`Domain` がないことが必要で、`__Secure-` には `Secure` が必要です。無効な起動時の組み合わせは配信開始前に失敗し、レンダラーは無効な接頭辞付きヘッダーを、ブラウザに黙って破棄させる代わりに書き換えます。

### 追加

- **Suprnova の認証は、内部 Magnetar エンジン上で動作するようになりました。** フレームワーク所有の `Auth` ファサードは、Torii 依存関係を取り除きつつ、既存のパスワード、マジックリンク、passkey、OAuth、bearer、ロックアウト、セッション、二要素の呼び出し箇所を維持します。デフォルトエンジンはパスワード/セッションおよび passkey アダプターを原子的に導入し、ライフサイクル配信リースをアプリケーションデータベースに保存し、アプリケーションの正規 `i64` `app_users` ID を共有します。
- **形状を認識する認証移行ランナーが Torii、Suprnova web、Suprnova API のソースを対象にします。** ドライランは、安定したプラン ID を永続的な行およびスキーマのフィンガープリント、さらに宛先 ID の決定に結び付けます。適用ではトランザクションインポート、リトライ台帳、形状所有のクリーンアップ、衝突拒否を使います。MySQL は、書き込みバリアで保護されたシャドウスワップに、事前コピーの仕訳、行およびスキーマのパリティ、再開可能なリネーム、クリーンアップを維持する復元を組み合わせます。
- **`MAIL_DRIVER=file` は、メッセージごとに 1 つの RFC 5322 `.eml` を `MAIL_FILE_PATH` に書き込みます。** 既定値は `storage_path("mail")` で、相対値はプロセスの CWD ではなくアプリケーションのベースディレクトリを基準にします。これによりローカルメールをログ行から読む代わりにメールクライアントで開けます。ファイルは `X-Priority`、`Importance`、`X-Tag`、`X-Metadata-*`、`Return-Path` を含む SMTP と同じヘッダーの上位集合を持ちます。`log` や `memory` と同様、これは配送を行わないため、本番の起動は `MAIL_ALLOW_NON_DELIVERING_IN_PRODUCTION=true` がなければ拒否します。
- **`FrameworkError::External` はラップしたエラーを保持します。** `FrameworkError::from_external(e)` と `FrameworkError::from_external_with("saving user", e)` は元のエラーを文字列に溶かさず、`std::error::Error` の source として到達可能にします。`FrameworkError::external_source()` はダウンキャストのためにそれを返します。共有 `Arc` ハンドルを返す `source()` ではなく、こちらを使ってください。両方のコンストラクターは HTTP 500 に対応付けられます。
- **5xx ログは完全なエラー source チェーンをレンダリングするようになりました。** `render_error_chain` は `source()` をたどり、フレームワークエラーのログ行、`ErrorOccurred` イベントのペイロード、`APP_DEBUG=true` で出力される `debug_message` フィールドへ接続されています。クライアント向けレスポンス本文は変わらず、5xx 本文はサニタイズされたままです。
- **`InertiaResponse::scroll_wrapped` / `scroll_with_wrapped` / `try_scroll_wrapped`。** スクロール prop のマージ指示を裸のキーではなく `<key>.<wrap_key>` の下にネストします。すなわち、値自体がエンベロープ（`{ data: [...], meta: {...} }`）の場合、`mergeProps: ["users"]` ではなく `["users.data"]` です。Laravel の `ScrollProp` は無条件に `"data"` の下へラップしますが、Suprnova の組み込み paginator は裸の行配列を返すため、すべての呼び出し元が回避しなければならない既定値ではなくオプトインです。新しい `ProvidesScrollMetadata` トレイト（`page_name` / `previous_page` / `next_page` / `current_page` と既定の `scroll_metadata()`）は、このクレートが知らない paginator のために同名の Laravel インターフェースを写します。`LengthAwarePaginator`、`Paginator`、`CursorPaginator` は今や手で `ScrollMetadata` を組み立てる代わりにこれを実装します。スクロール prop の `.match_on(...)` フィールドも `matchPropsOn` へ出力され、Laravel の `resolveMergeMatchingKeys`（`Response.php:641-652`）に一致します。これはほかのマージ prop と同じく `ScrollProp` の `matchesOn()` を畳み込み、マッチ項目は prop が実際にマージされる場所、ラップなしの `<key>` または `.scroll_wrap(...)` 下の `<key>.<wrap_key>` をキーにします。
- **`Prop::merge_with_path`、複数フィールドの `match_on`、リゾルバーを伴うマージ prop。** `Prop::merge_with_path(path)` は prop 値全体ではなく値の中のネスト済みフィールドをマージします。`Prop::eager(v).merge().merge_with_path("data")` は `mergeProps: ["<key>.data"]` を出力し、パスマージする prop がそのルートも同時にマージすることはありません。`.deep_merge()` はすでにすべてのフィールドへ再帰するため、これを無視します。`Prop::match_on` は、既存の `match_on("id").match_on("slug")` という `Prop` のチェーン合成に加え、1 回の呼び出しで 1 つまたは複数のフィールド（`match_on(["id", "slug"])`）を受け取ります。`InertiaResponse::merge_lazy` / `merge_lazy_with` は `.merge` / `.merge_with` のリゾルバー付き兄弟であり、Laravel の `Inertia::merge(fn () => ...)` に対応します。
- **部分リロードの `only`/`except` がドット記法を理解します。** `X-Inertia-Partial-Data: user.name` は値全体または何も返さないのではなく `user` prop を `{ name: ... }` へ絞り、`X-Inertia-Partial-Except: user.email` は `user` の残りを残してそのフィールドだけを削ります。両ヘッダーが同じパスを指定したときは `except` が勝ち、裸の項目は依然として prop 全体を意味し、未知または型不一致のネストパスは兄弟を触らずに黙って削除されます。`Always` prop は影響されず、常に全体が送られます。
- **ドットキー prop のネスト化。** `.with("user.name", value)`（および eager か resolved かを問わない、ほかのあらゆる prop 付加メソッド）は、リテラルな `"user.name"` キーを送る代わりに `props.user` へネストします。これは Laravel の `Arr::set` ベースの `resolveArrayableProperties` の展開に一致します。接頭辞を共有する 2 回の呼び出し、`.with("user.name", …)` の後の `.with("user.age", …)` は 1 つのオブジェクトへ蓄積され、ドットのないキーは影響されません。`App::inertia_share*` の共有レジストリキーもレスポンス上で同様にネストします。展開はトップレベル prop の**キー**だけに触れ、prop の値へ再帰しないため、バリデーションの `errors` バッグは内部に含むドット付きフィールド名を保ちます。
- **`App::inertia_shared(key)` / `App::flush_inertia_shared()`。** Laravel の `Inertia::getShared` / `Inertia::flushShared`、すなわち静的共有レジストリ（`App::inertia_share` / `_lazy` / `_once`）の読み取りと消去です。`inertia_shared` は読み取り側で `inertia_share` と同じドット記法をサポートし、lazy または once の共有（リクエストに対して解決するものがないため）および未登録キーには `None` を返します。`flush_inertia_shared` が消去するのは静的レジストリだけで、`App::register_inertia_shared` によって登録されたトレイトプロバイダーは Laravel と同様に影響されません。そこには消去すべきリクエストごとの状態がないからです。
- **`InertiaResponse::always_with(key, resolver)`。** `.always(key, value)` の非同期リゾルバー兄弟です。遅延解決する価値があるほど高コストでも、常に含める prop のためのもので、Laravel の `Inertia::always(fn () => …)`（`AlwaysProp` はクロージャを含む任意の値を受け取る）に対応します。
- **`InertiaSharedData::share` がページコンポーネント名を受け取るようになりました。** これによりプロバイダーはページごとに出力を変えられます。Laravel の `RenderContext` に対応します。アップグレードを参照してください。
- **Inertia prop の合成。** `Prop` は 9 つの閉じた variant の一つである代わりに直交するフラグを持つようになり、Inertia 3 プロトコルが想定し、閉じた enum では表せなかった、deferred かつ mergeable、mergeable かつ cached、optional かつ cached のような組み合わせを 1 つの prop で表せます。`Prop::eager` / `Prop::lazy` / `Prop::from_resolver` / `Prop::absent` で作り、`.always()`、`.optional()`、`.defer()`、`.group()`、`.rescue()`、`.merge()`、`.prepend()`、`.deep_merge()`、`.match_on()`、`.once()`、`.as_key()`、`.until()`、`.fresh()`、`.scroll()` をチェーンし、新しい `InertiaResponse::prop(key, prop)` で付加します。`defer().merge()` prop は最初のレンダーで `deferredProps` の下に通知され、後続リクエストで `mergeProps` の下に届きます。新しい `MergeMode` と `Visibility` 型がフラグを表し、既存のすべてのビルダーショートカット（`.with`、`.always`、`.lazy`、`.optional`、`.defer`、`.merge*`、`.once*`）は変わりません。
- **キューの一時停止 / 再開。** `Queue::pause(connection, queue)` / `resume` / `pause_all()` / `resume_all()` / `is_paused(connection, queue)` / `paused_queues(connection, &queues)` は再起動シグナルと同じく `Cache` を基盤にします。Laravel と同様、`resume_all` はキューごとの一時停止を消しません。ワーカーの claim ゲートは各 pop の直前に置かれるため、実行中ジョブは常に完了します。グローバル一時停止は Laravel の `pausedQueues` と同様に `--queue=...` のフィルターを短絡し、キューごとの一時停止は明示的な `--queue=...` リストで開始されたワーカーだけに効きます。新しい CLI コマンドは `queue:pause [queue] [--all]` / `queue:resume [queue] [--all]`（エイリアス `queue:continue`）で、オペレーターが機能を無効にする `QUEUE_PAUSABLE=false` も加わりました。停止不可ワーカーは一時停止シグナルを無視し、`queue:pause` 自体も実行を拒否します。新しいイベントは `QueuePaused` / `QueueResumed` / `QueuesPaused` / `QueuesResumed` です。
- **`suprnova::testing::TestResponse`。** すべての HTTP テストハーネスがすでに生成する `(status, headers, body)` トリプルを、流れるような Laravel の `TestResponse` 形ラッパーで包みます。`assert_status`、`assert_ok`、`assert_redirect`、`assert_json`、`assert_json_path`、`assert_json_count`、`assert_see`、`assert_header`、`assert_cookie`、および（`.with_session_store(...)` があれば）`assert_session_has` を提供します。すべてのアサーションは `&Self` を返し、`expect!` と同じ契約で失敗時に panic します。リクエストをテストがどう駆動するかは一切変える必要がありません。
- **`suprnova new` が SSR エントリをスキャフォルドします。** 各スターター（Svelte、React、Vue）は `frontend/src/ssr.{ts,tsx}` と `build:ssr` npm スクリプト（`vite build --ssr`）を出荷し、SSR バンドルが `public/assets/` 内のクライアントビルドと衝突しないよう、専用出力ディレクトリ（`frontend/bootstrap/ssr/`）へ配線されます。
- **`InertiaConfig::ssr_bundle_path(path)` / `.ssr_ensure_bundle_exists(bool)`。** SSR ゲートウェイはレンダーをディスパッチする前にビルド済みバンドルがディスクに存在するか検査でき、Laravel の `ensure_bundle_exists` 設定に対応します。開始されていないワーカーや未ビルドのバンドルは、決して成功しない接続に `ssr_timeout` を費やす代わりに速やかに失敗します。`.ssr_bundle_path(...)` でオプトインしてください。Laravel の `BundleDetector` と異なりパスは決して自動検出されないため、設定しない既存 SSR 設定（およびテスト）は影響されません。
- **Inertia visit のバリデーション失敗は `422` JSON を返す代わりにリダイレクトバックします。** `Inertia::install` は 4 つ目のミドルウェア `InertiaValidationRedirectMiddleware` を登録し、`X-Inertia` リクエスト上のバリデーション `422` を、エラーを flash したフォームページへの `303` に変えます。そのためハンドラのコードなしに `useForm().errors` が埋まります。Inertia クライアントは `X-Inertia` ヘッダーのないレスポンスを非 Inertia として扱いエラーモーダルを表示するため、旧来の `422` は `form.errors` に到達できませんでした。非 Inertia リクエストは `422` エンベロープを維持し、Precognition のドライランは影響されず、`X-Inertia-Error-Bag` が flash されたバッグをスコープします。リダイレクト先は同一オリジンの `Referer`、次にセッションの以前の URL、次に同じサニタイザーを通したリクエスト自身のパスであり、それさえ失敗すれば `/` へフォールバックします。決してそのまま信頼されません。
- **`InertiaConfig::with_all_errors(bool)`。** フィールドごとに最初のバリデーションメッセージへ畳む代わりに、すべてを維持します。Laravel の `Inertia\Middleware::$withAllErrors` に対応します。
- **`suprnova::testing::AssertableInertia`。** `X-Inertia` JSON レスポンスまたはハードナビゲーション HTML シェルに埋め込まれた `<script data-page="app">` 要素から解析した Inertia ページオブジェクトに対する、流れるような Laravel の `AssertableInertia` 形アサーションです。`component`、`url`、`version`、`prop`、`has`、`missing`、`where_`、`count`、`has_flash` を提供します。`HttpResponse` からは `AssertableInertia::from_response` で、`TestResponse` からは新しい `TestResponse::assert_inertia()` で作れます。`reload_only`、`reload_except`、`load_deferred_props` は、呼び出し元指定の `with_reload(...)` クロージャに対して部分リロードを再実行します。Suprnova の HTTP テストは実ソケットをまたぐため、ハードコードできる単一のプロセス内テストクライアントはありません。
- **`Cookie::queue`/`queued`/`unqueue`/`expire`。** タスクローカルのクッキージャー、Laravel の `CookieJar` が、イベントリスナー、コンテナにバインドされたサービス、ハンドラより前のミドルウェアなど、どのコードにも付加先の `HttpResponse` を持たずに次のアウトバウンドレスポンス用クッキーをキューさせます。`Auth::login_remember` が remember-me クッキーをハンドラ境界の先へ運ぶためにすでに使うリクエストごとのスロットを基盤にし、`SessionMiddleware` がセッションクッキーの隣でレスポンスへ排出します。`Cookie::expire(name, path, domain)` は `Cookie::forget_with` で作った削除クッキーをキューします。ルートのミドルウェアチェーンに `SessionMiddleware` が必要であり、その外部では 4 つの呼び出しはすべて `App::flash` の flash スコープ外での動作と同じく黙った no-op です。
- **`HttpResponse::event_stream(stream, end)` および `HttpResponse::stream_json(stream)`。** Laravel の `ResponseFactory::eventStream` / `streamJson` と、`@laravel/stream-{react,vue,svelte}` の `useEventStream` / `useJsonStream` が期待する正確なレスポンス形状です。`event_stream` は `Stream<Item = sse::StreamedEvent>` を、項目が独自イベント名を指定しない限り項目ごとに `event: update` としてフレーム化し、文字列以外のペイロードを JSON エンコードし、構成可能な終端フレームを付加します（`EndSignal::default()` は `data: </stream>`、`EndSignal::None` は省略）。`stream_json` は `Stream<Item = impl Serialize>` を増分フラッシュする 1 つの JSON 配列としてストリーミングします。どちらも既存の `sse`/`stream_bytes` ボディパイプライン上に構築され、残りのフレームワークとキャンセルおよび panic 分離の動作を共有します。
- **`suprnova serve` はクラッシュした開発プロセスでセッション全体を終了する代わりに再起動します。** 試行間は指数バックオフで、200ms から連続クラッシュごとに倍になり 5s で上限となり、プロセスが 30s 稼働すると下限へ戻ります。`--no-restart` はオプトアウトして従来動作を復元します。`--restart-tries <N>`（既定 `5`、Laravel の `--restart-tries=5` と一致）は、永遠に再試行する代わりに連続クラッシュがその回数に達したプロセスの再試行を断念し、対処可能なメッセージを表示し、ほかのプロセスとセッション自体を稼働させ続けます。`--timestamps` は転送する各行へ `HH:MM:SS` を付けます。新しい `Suprnova.toml` の `[[serve.process]]` 配列は、プロジェクトがバックエンドおよびフロントエンドと並行して独自の開発プロセス（Laravel の `DevCommands::register`）を、それぞれの `[name]` 接頭辞と任意の色で宣言できるようにします。エントリ内の未知キーまたは空の `name`/`command` は、黙って無視されたり後で不透明な spawn 失敗になったりする代わりにハードなパースエラーです。`--json` は stdout へ代わりに 1 行ごとの JSON オブジェクト（NDJSON）を出力します。プロセス開始、出力、終了、再起動予定、再起動成功、断念、型再生成、終了イベントに、ファイルウォッチャー自身の再生成通知と `Ctrl+C` ハンドラの終了通知も含みます。これら 2 つも `--json` では stdout から外れます。スクリプトおよびログパイプライン用であり、各イベントは固有の timestamp を持つため、`--timestamps` と組み合わせても無害ですが冗長です。
- **`RequestBuilder::retry_when(predicate)`。** 組み込みポリシー（`.retry(...)` / `.retry_non_idempotent(...)`）が本来行う各リトライ前に参照される predicate で、`RetryContext { attempt, method, url, outcome: RetryOutcome::TransportError | Status(u16) }` を受け取ります。ポリシーを置換せず合成します。`false` はポリシーが行うはずのリトライを拒否できますが、`max_attempts` を越えるものや、ポリシーが行わないもの（4xx ステータス、または `retry_non_idempotent` なしの非べき等メソッド）を強制することは決してできません。
- **`#[model(touches = [...])]` が実際に touch するようになりました。** 子が作成、保存、更新、削除された後、リストに名付けられた各 `BelongsTo` owner は、トリガーとなった書き込みと同じ executor 上で 1 回の `UPDATE <owner> SET updated_at = ? WHERE <key> = ?` を受けます。したがって `DB::transaction` の内部では touch はそのトランザクションに参加し、一緒にロールバックされます。モデルの `timestamps = false` を持つ owner は書き込まれずエラーにもならずスキップされます（Laravel 13.25 も同じ不足を閉じました）。`NULL` 外部キーを介して到達する owner と論理削除済み owner もスキップされます。宣言済み `BelongsTo` リレーションを名付けない `touches` 項目は今やコンパイルエラーで、polymorphic owner はまだ未対応です。
- **`without_touching_on::<M, _, _>(fut)`。** Laravel の `Model::withoutTouchingOn([M::class], $cb)` です。`m.touch()` と `M` を対象とするあらゆる owner cascade の両方を抑制し、ほかの型の owner は引き続き更新します。スコープはネストでき、既存の `without_touching` も直接の `touch()` 呼び出しだけでなく owner cascade を抑制するようになりました。
- **`Model::touch_owners()` / `touch_owners_with_tx(tx)`。** フレームワークが所有しないパスを通じて子行を書いたときのための Laravel の `touchOwners()` です。
- **値形バリデーションルール `ArrayKeys` と `Distinct`。** 新しい `ValueRule` トレイト（`passes(&self, value: &serde_json::Value)`）は `Rule` と並び、同じキー付きメッセージ契約を共有します。`rules::ArrayKeys(&[...])` は許可リスト外のキーを持つ JSON オブジェクトを拒否し（Laravel の `array:keys`、#60918）、`rules::Distinct { ignore_case, strict }` は重複要素のある JSON 配列を拒否します（Laravel の `distinct`）。`validate!` 行は同じフィールドリストでどちらの規則種別も受け取り、ディスパッチは新しい行構文ではなく、規則が実装するトレイトによって自動選択されます。
- **`Job::delay()`。** ジョブは既定の遅延（`fn delay() -> Option<Duration>`、既定は `None`）を宣言でき、`Queue::push` および `Queue::bulk` が尊重します。`available_at` は `now` ではなく `now + delay` になります。明示的な呼び出し箇所の遅延が依然として勝ちます。`Queue::push_later(job, at)` と `Queue::later(delay, job)` は呼び出し元の timestamp をそのまま使い、`Job::delay()` を決して参照しません。
- **`Notification::{queue, timeout, fail_on_timeout, max_tries, backoff}`。** キューされた通知（`Notify::queue`）は、`Mail::on_queue` が使う `EnvelopeOverrides` primitive を通じて、チャネルごとの `SendNotificationJob` push すべてに固有のキューチューニング既定値を運びます。`fail_on_timeout(&self) == true` は Laravel の `#[FailOnTimeout]` 通知属性（#61072）と同じく、リトライする代わりに最初の timeout で dead-letter になります。5 つすべての既定値は `SendNotificationJob` の既存 `Job` 既定値であり、何も上書きしない通知は影響されません。
- **`Mail::on_queue` / `Mail::on_connection` と `Queue::push_with`/`later_with`。** キューされた mailable は今や `Mail::to(..).on_queue("emails").queue(mailable)` で自らをルーティングでき、`Mailable::queue(&self)` で既定を設定できます。どちらもジョブのために登録された `Queue::route` とジョブ自身の `Job::queue()`/`Job::connection()` より優先されます。それらの背後にある新しい `EnvelopeOverrides` primitive（`Queue::push_with(job, overrides)` / `Queue::later_with(delay, job, overrides)`）は、1 回の push に対する timeout、fail-on-timeout、max-tries、backoff も対象にします。`MailFake` のキュー済みスナップショットは解決済み `queue` を運ぶようになり、`queued_on(...)` / `assert_queued_on(name, queue)` でアサートできます。
- **`Application::http_bootstrap(f)`。** HTTP 専用の boot hook です。`bootstrap` の後に、`serve` / `web:run` パスだけで実行されるため、キュー、スケジュール、ワークフローワーカーとコンソールバイナリは決して実行しません。ワーカーおよびコンソールのコンテナイメージは起動にビルド済みフロントエンド manifest を必要としなくなりました。`Inertia::install` は本番で manifest がない場合に fail closed しますが、検査は実際に HTTP を提供するプロセスでのみ実行されます。
- **`Router::inertia(path, component, props)`。** ハンドラが 1 行になる静的ページのための Laravel の `Route::inertia` です。`GET` を登録し（`HEAD` はそこへフォールスルーします）、`RouteBuilder` を返すため、ルートに名前とミドルウェアを与えられます。`Router::view` はエイリアスとして維持されます。
- **SES v2 送信オプション。** SES transport は今や `SendEmail` に `TenantName`、`ConfigurationSetName`、`ListManagementOptions` を出力します。各々には transport レベルの既定値（`SesMailTransport::tenant_name` / `configuration_set_name` / `list_management`）とメッセージごとのヘッダー上書き（`X-SES-TENANT-NAME`、`X-SES-CONFIGURATION-SET`、`X-SES-LIST-MANAGEMENT-OPTIONS`）があり、ヘッダーが勝ちます。ヘッダーはリクエスト構築時に消費され、メッセージへレンダリングされません。
- **すべてのレスポンスビルダーでの `without_cookies`。** `HttpResponse`、`Response`（`ResponseExt` 経由）、`Redirect`、`RedirectRouteBuilder` はすべて 1 回の呼び出しでクッキーのリストを失効させ、`Redirect` / `RedirectRouteBuilder` には不足していた単一名の `without_cookie` が加わりました。新しい `Cookie::forget_with(name, path, domain)` は元のクッキーが設定された path と domain にスコープされた削除クッキーを構築します。単純な `forget` は `/` の外で設定されたクッキーを決して消しません。
- **`Queue::fake()` はキャプチャした各 push に envelope id を刻印します。** `pushed_with_id::<J>()` は `(job, id)` の組を返し、fake はその id を運ぶ、本物の driver push と同じ `JobQueueing` / `JobQueued` の組をディスパッチするようになりました。そのためテストは、キャプチャした push とリスナーが見たものを相関できます。既存の fake ヘルパーは変わりません。
- **`UniqueJobSkipped` キューイベント。** `Queue::push_unique` が重複を抑制したとき、`queue::events::UniqueJobSkipped { job_name, unique_id, connection }` をディスパッチするようになりました。これにより dedupe は黙ったものではなく観測可能です。呼び出しの戻り値は変わりません（`Ok(false)`）。
- **クエリビルダーおよびコレクションの `model_keys()`。** `User::query().model_keys().await?` は単一のモデルも hydrate せずに一致するすべての行の主キーを返し、結合に耐えるようテーブル修飾キー（`users.id`）を投影します。`Collection::model_keys()` はすでに hydrate 済みの対応物です。`#[suprnova::model]` はキーの Rust 型も `EloquentModel::Key` として宣言するため、どちらも呼び出し元が選ぶ turbofish ではなく型の `key_type` 名を返します。

### 修正

- **PostgreSQL の論理削除はバックエンド対応のプレースホルダーを使うようになり、生成された timestamp 書き込みは宣言済み cast を尊重します。** `delete()` と `restore()` は MySQL および SQLite の `?` プレースホルダーではなく PostgreSQL の序数プレースホルダーをレンダリングします。生成された create、update、save、touch、論理削除の書き込みも timestamp を各フィールドの宣言済み `Cast` ストレージ型で変換するため、ネイティブの `TIMESTAMPTZ` 列がテキスト値を受け取ることはありません。両方の欠陥を報告し、[@i-am-v-alexander-v](https://github.com/i-am-v-alexander-v) が [PR #3](https://github.com/eas4ai/suprnova/pull/3) で修正を提出したことに感謝します。
- **既定の workspace および Magnetar gate の実行に、稼働中の PostgreSQL または MySQL サービスが不要になりました。** バックエンド固有の動作スイートは、構成されたデータベースなしに意図して呼び出した場合には引き続き失敗する、明示的に ignored な適格性テストです。到達可能性だけを確認するテストと恒久的な gate 環境要件を取り除いたため、無関係な変更が毎回の検証実行で外部データベース設定のコストを払うことはありません。
- **`PartialFilter::narrow` は現在 `pub` です。** 4 つの兄弟 predicate（`should_include`、`should_include_eager`、`should_include_optional`、および型自身）はすでに公開されていましたが、`should_include_eager` の `true` の回答を正しくする絞り込み pass、すなわち `only`/`except` 項目が実際に要求したドットパスへ解決済み値を切り詰める pass は `pub(crate)` でした。`PartialFilter` 上に独自の部分リロード処理を構築する呼び出し元にはその絞り込みを再現する公開手段がなく、`should_include_eager` がキーを含むと報告しても、ドット付き `only` 項目の下へ値全体を出荷してしまいました。
- **`MailFake` の `QueuedSnapshot` は `.on_connection(...)` をアサートできるようになりました。** `Queue::fake()` は Wave 3 で `assert_pushed_on_queue` とともに `assert_pushed_on_connection` を得ましたが、`Mail::fake()` はキュー側だけを得ていました。そのため connection override を伴ってキューされた mailable は実際の dispatch では解決および適用されたのに、fake 経由ではアサートできませんでした。新しい `QueuedSnapshot::connection`、`MailFake::queued_on_connection`、`MailFake::assert_queued_on_connection` が、`assert_queued_on` の形を写してその差を埋めます。
- **ドット付き共有 prop は裸の `only` 項目から到達不能でした。** `App::inertia_share("auth.user", …)` の後の `router.reload({ only: ['auth'] })` は `props: {"errors":{}}` を返し、その share は完全に消えていました。レジストリは `auth.user` を 1 つのリテラルキーとして保存し、`Arr::set` の展開 pass はすべての prop が解決された後にだけネストするため、部分リロードゲートはまだ平坦なキーを見て `auth` にもほかの何にも一致させませんでした。`only`/`except` 項目は現在対称です。項目は prop のキーを正確に、内部のパス（絞り込む `user.name`）、またはその**祖先**（`auth.user` キーに対する `auth`、呼び出し元がルート全体を求めたため prop 全体を出荷する）として指定できます。裸の `except: ['auth']` は Laravel のすでにネストされたバッグで `Arr::forget` がサブツリー全体を落とすのと同様に、その下の全 prop キーを落とします。接頭辞はセグメント境界で終わらなければならないため、無関係な `authAgent.user` prop はどちらのリストからも影響されません。Laravel は `Inertia::share` が share 時に `Arr::set` を実行するためこれに遭遇しませんが、Suprnova のレジストリは lazy share にリクエスト解決までネストする値がないため実行できません。
- **`#[data(lazy(deferred))]` フィールドが `?include=` allowlist を迂回していました。** `resolve_props` の owner-tagged 解決パスは `Prop::is_lazy()` で prop を選びましたが、これはフラグを持つものには false で、deferred フィールドは `Visibility::Deferred` です。そのためフィールドは include-set 検査のない通常 prop パスで解決され、deferred follow-up を送った任意クライアントへ、リクエストがフィールドを opt in したかにかかわらず出荷されていました。`Prop::resolve_with_owner` は今やフラグの有無を問わずあらゆるリゾルバー付き owner-tagged prop をゲートし、`resolve_props` はほかのすべての block より前にそのゲートを実行します。`?include=` 外のフィールドは全体が落とされ（値も `deferredProps` 通知もなし）、`?include=` に名付けられたが DTO の allowlist 外のフィールドは、`X-Inertia-Partial-Data` が吸収する前に `400` を発生させます。回帰ではありません。Wave-4 前のコードは `Prop::Lazy` enum variant によってゲートしており、`Prop::Defer` も失敗していましたが、いずれにせよ実際の穴でした。
- **一致した部分リロードで `deferredProps` が再通知されていました。** 1 つの deferred キーを指定した partial が、ほかのすべての deferred キーをクライアントへ再び通知し、クライアントはそれらを再度取得し、次の partial でも再度取得していました。Laravel の `resolveDeferredProps` は 1 つの prop も調べる前にリクエストが partial になった時点で `[]` を返します（`Response.php:661-663`）。現在は一致したどの partial でも block 全体が落とされます。別コンポーネントを狙う部分リロードは、このゲートでもほかのすべてと同様に標準 visit であり、その通知は影響されません。
- **`errors` バッグのフィルターはエラーの出所によって異なっていました。** セッション flash バッグは resolve loop より先に seed され、部分リロードフィルターは到達できませんでした。一方ハンドラ自身の `.with("errors", …)` は通常ゲートを通ったため、`only: ['errors.email']` は seed 済みバッグ全体を出荷する一方で 1 フィールドのハンドラバッグを出荷し、`only: ['users']` はキーを残す代わりにハンドラバッグを seed 済みバッグで置換しました。両パスは今や `errors` を常に可視として扱い、`Inertia::always(...)` として共有し、`only`/`except` 再構築後に raw 値を `resolveAlways` で再注入する Laravel のミドルウェアに一致します。クライアントが部分レスポンスを `{...current.props, ...response.props}` で折り畳むため、これは必要な形です。空の `errors` オブジェクトは画面上のメッセージを消しますが、未フィルターのものは正しく保ちます。キーの明示的な可視性フラグは依然として勝つため、`.prop("errors", Prop::eager(…).optional())` は optional に振る舞います。
- **`Queue::fake()` は push ごとの `EnvelopeOverrides` を観測できるようになりました。** `Queue::push_with`/`Queue::later_with` を通じて push されたジョブは fake 下で通常の `Queue::push` と区別できませんでした。`FakePush` は payload と `available_at` だけを持ち、override はファサードから出ず、テストは正しいキューまたは connection へ dispatch したかを何もアサートできませんでした。新しい `queue::testing::pushed_with_overrides::<J>() -> Vec<(J, EnvelopeOverrides)>` はキャプチャした各 push と宣言されたものの組を返します。`assert_pushed_on_queue::<J>(queue)` と `assert_pushed_on_connection::<J>(connection)` は、`MailFake::assert_queued_on` に対応するよくある単一フィールドの場合を対象にします。ほかのすべてのエントリポイント（`push`、`push_later`、`bulk`、`push_unique`、chain/batch dispatcher）は引き続き override を取らず、`EnvelopeOverrides::default()` を記録するため、通常 push は fake 下で正確に「override 未宣言」として読めます。
- **レスポンス本文の途中で停止した SSR worker がレンダーを永遠にハングさせることがありました。** `SsrConfig::timeout` はレスポンスヘッダーまでの待機だけを制限し、ヘッダー到着後の本文読み取りには固有の timeout がありませんでした。そのため接続を受け入れ、ヘッダーを送り、その後データを止める worker は、CSR へフォールバックする（または `ssr_throw_on_error` 下でエラーにする）代わりに構成済み timeout を超えてリクエストをハングさせました。両フェーズは現在 1 つの deadline を共有するため、構成済み timeout は自身のドキュメントが約束していたとおり SSR 呼び出し全体を制限します。
- **`Auth::login_remember` が設定する remember-me クッキーを含むキュー済みクッキーが、`SessionMiddleware` の 3 つの内部 fail-closed パスで黙って落とされていました。** セッション読み取り失敗、セッション書き込み失敗、セッションクッキー暗号化失敗はそれぞれ、`handle` の終端で走る pending-cookie drain を迂回して合成された `500` を直接返していました。そのリクエストで `Cookie::queue` を通じてキューされたもの、データベースにすでに commit された remember-me トークン行を含め、すべてが `Set-Cookie` ヘッダーとしてクライアントに届きませんでした。現在は 3 パスとも返す前に pending cookies を排出し、ハンドラが返したエラーまたはリダイレクトと同じです。Laravel 自身のキュー済みクッキーが未捕捉 panic では失われるのと同じく、これは未捕捉 panic を対象にしません。
- **`Queue::push_unique` は `Queue::push`、`Queue::push_with`、`Queue::bulk` と一致して `Job::delay()` を尊重するようになりました。** 以前は `available_at` を `Utc::now()` から直接算出していたため、既定の遅延（`fn delay() -> Option<Duration>`）を宣言するジョブは `push_unique` を通じると、その遅延後ではなく即時 dispatch されました。`Queue::push_unique_later` と `Queue::later_unique` は影響されません。すでに呼び出し元から明示的な timestamp または遅延を取っており、`push_later`/`later` と同じく `Job::delay()` を決して参照しないためです。

### 変更

- **現在の開発ブランチは SeaORM 2.0 を使い、Rust 1.94.0 を必要とします。** Suprnova は Eloquent、`#[model]`、migration、database-facade のソース形状を維持します。SeaORM を直接呼ぶアプリケーションは、SeaQuery の式メソッドのために `ExprTrait` を import し、事前構築した `Statement` 値には明示的な `*_raw` connection メソッドを使う必要があります。SeaQuery は現在 1.0 で、直接の MariaDB vector driver は SQLx 0.9 を使います。既存データベースにはアプリケーションデータ migration は不要で、新規 PostgreSQL スキーマは serial を基盤にする primary key を維持します。
- **さらに 3 つの未使用依存関係を削除しました。** `pretty_assertions` と `qrcode` は framework crate から去ります（`totp-rs` はすでに `qr` feature を運ぶため、二要素登録の QR provisioning は影響されません）。`notify-debouncer-mini` は CLI から去ります（`notify` 自体は残ります。`serve` と `generate-types` の watcher が直接使うためです）。3 つすべては doc test を対象にする `cargo-udeps` とソース全体の検索で未使用と確認されました。
- **`suprnova-macros` は `serde` または `serde_derive_internals` に依存しなくなりました。** いずれも未使用でした。マクロが出力する `::serde::Serialize` パスはマクロ crate 自身ではなく downstream crate 内で解決されます。生成コードへの影響はありません。
- **`MergeStrategy` の `match_on` は複数のフィールド名を運ぶようになりました。** `Append`、`Prepend`、`Deep` はそれぞれ `match_on: Option<String>` から `match_on: Option<Vec<String>>` へ広がります。そのため `InertiaResponse::merge_with` / `merge_lazy_with` は、`.prop(key, Prop::eager(v).match_on([...]))` がすでにできたのと同じく、複数フィールドで dedupe できます。これ以前は response-builder のショートカットは `Prop` を直接作るより厳密に表現力が低いものでした。アップグレードを参照してください。
- **スクロール prop は Laravel と同一の `reset` およびマージ意味論を出力するようになりました。** `scrollProps[key].reset` はクライアントが `X-Inertia-Reset` で `key` を指定したときだけ `true` で、Laravel の `resolveScrollProps` に一致します。以前のように `X-Inertia-Infinite-Scroll-Merge-Intent` ヘッダーのないすべての visit で `true` にはなりません。スクロール prop は現在、既定で append のマージメタデータも無条件に運びます。ヘッダーがまったくない新規 visit は、以前の `reset: true` とマージメタデータなしではなく、`reset: false` と `mergeProps` 項目を出力します。`X-Inertia-Reset` にあるキーは、そのレスポンスの `mergeProps` / `prependProps` から除外され、これは通常のマージ prop がすでに持つ除外と同じです。
- **`ssr:check` は、何かが TCP 接続を受理したことだけでなく、SSR worker の `GET /health` ルートが 2xx を返すことを検証します。** すべての `@inertiajs/{vue3,react,svelte}/server` worker は `/health` をすぐに返すため、worker 側の変更は不要でした。Laravel の `Inertia\Ssr\HttpGateway::isHealthy()` に一致します。
- **Inertia の `errors` prop は配列ではなくフィールドごとに 1 つの文字列を運ぶようになりました。** セッション flash のバリデーションバッグは `{ email: ["The email field is required."] }` ではなく `{ email: "The email field is required." }` としてレンダリングされ、Laravel の既定と Inertia 自身の `ErrorValue = string` に一致します。`InertiaConfig::with_all_errors(true)` は配列形状を復元します。ハンドラが自身で設定する `errors` prop はそのまま通り、セッション flash（`Redirect::with_errors`、`session.pull_errors_flash()`）は配列を引き続き保存します。変わるのはレンダリングされたページ prop だけです。
- **`Model::TOUCHES` は inherent const から `EloquentModel` へ移動しました。** 親 touch cascade は `Model` トレイトの既定にあり、トレイト既定は inherent const を読めません。`Comment::TOUCHES` は依然として解決されますが、現在はスコープ内に `use suprnova::EloquentModel;` が必要です。`touches` 属性を持たないモデルはトレイトの空の既定値を得ます。
- **`RelationEntry` に `related_updated_at_column` が加わりました。** 手で `RelationEntry` を構築するものには追加フィールドが必要で、ツリー内にはそのようなものはありません。マクロがすべてを出力します。
- **`Router::view` は JSON object ではない props を拒否するようになりました。** 以前は診断なしに無視し、空の prop バッグをレンダリングするルートを登録していました。`null` は「props なし」として引き続き受け付けられ、`Router::try_inertia` は失敗可能な形式です。
- **Inertia asset version はリテラルの `"1.0"` ではなく Vite build manifest の hash を既定にします。** そのため誰かが文字列を bump するのを覚えなくても、deploy は長期接続クライアントを無効化します。`InertiaConfig::manifest_path(...)` はそれとともに resolver を再指定し、明示的な `.version(...)` / `.version_with(...)` は引き続き勝ちます。ディスクに manifest がないローカル開発では version はすべてのアプリが以前見ていた `"1.0"` にフォールバックするため、ビルドするまで何も変わりません。新しい `VersionResolver::from_manifest(path)` は resolver を直接公開します。

### 非推奨

- **`Cookie::read_encrypted` は現在 v1 専用のレガシー reader です。** `Cookie::encrypted` で発行し `read_encrypted` で読むコードは、このリリース後に最初に書かれた値で実行時に失敗します。`read_encrypted_for(name, wire)` へ切り替えてください。コンテキストを持たない `CryptPurpose::Cookie` エントリポイントも置き換えられます。両方の削除は 1.4.0 に予定されています。

### アップグレード

- **Cookie 復号警告には、現在 2 つの独立した軸があります。** `KeyOrigin::Previous(index)` 警告は、現在の `APP_KEY` の下で値を再暗号化し、rotation tail がなくなった後にだけその previous key を除くべきことを意味します。`AadVersion::Legacy` 警告は、1.4.0 の fallback 削除前に、名前に束縛された API を通じてクッキーを再発行すべきことを意味します。値は両方を報告できます。
- **`SESSION_COOKIE_PREFIX` はオプトインです。** `__Host-` は HTTPS、`SESSION_SECURE=true`、`SESSION_PATH=/`、`SESSION_DOMAIN` なしでのみ deploy してください。ローカル HTTP scaffold では空のままにします。`CsrfMiddleware` の `with_session_config` はリテラルの `XSRF-TOKEN` 名を保ちます。クライアントがその別名に構成されている場合は `.xsrf_cookie_name("__Host-XSRF-TOKEN")` を使ってください。
- **`DecryptOrigin` は 2 軸の `#[non_exhaustive]` struct になりました。** その `key` と `aad` フィールドを独立に読み、`KeyOrigin` / `AadVersion` enum に対して wildcard 互換の match 戦略を維持してください。
- **`SessionConfig` と `CookieOptions` は現在 `#[non_exhaustive]` です。** アプリケーションコード内の struct literal と functional record update は、`Type::default()` の後に public field の代入または builder method を使う形へ移す必要があります。
- **`FrameworkError` は現在 `#[non_exhaustive]` です。** 自分のコードでそれを `match` するには wildcard arm が必要です。variant の追加が破壊的変更であった最後のリリースです。
- **`MergeStrategy::Append`/`Prepend`/`Deep` の `match_on` フィールドは `Option<String>` ではなく現在 `Option<Vec<String>>` です。** struct-literal 形式を直接構築する呼び出し箇所、`MergeStrategy::Append { match_on: Some("id".into()) }` はコンパイルされなくなります。フィールド名を `Vec` で包んでください。`Some(vec!["id".into()])`。`match_on: None` は影響されず変更不要です。
- **一致した部分リロードは、もはや `deferredProps` を出力しません。** カスタム deferred-loading コンポーネント、テストスナップショット、エンドツーエンドアサーションで、部分リロードレスポンスの `page.deferredProps` を読んでいたコードは、リクエストが名付けなかった deferred props のリストがあった場所で現在はキーがないことを見ます。通知は Laravel が置き、公式クライアントが読む最初の（非 partial）visit から読んでください。
- **裸の `except` 項目は現在、その下のドット付き prop キーを落とします。** `X-Inertia-Partial-Except: auth` は、キー全体を比較していたため、以前は `auth.user` に登録された prop をレスポンスに残していました。現在は落ちます。ページが裸の `except` 項目は正確なキーだけを削ることに依存していた場合、正確なキー（`except: ['auth.user']`）を指定するか、代わりにドット付きパスで絞ってください。
- **`errors` は `only`/`except` を無視します。** ハンドラが供給する `.with("errors", …)` prop を部分リロードが除外したり、ドット付き項目で絞ったりしていた場合、現在は全体を出荷します。部分リロードで切り出された、または空の `errors` object をアサートするテストは更新が必要です。意図してバッグをレスポンスから除くには、部分リロードリストに依存せず `.prop("errors", Prop::eager(…).optional())` のように flag を付けてください。
- **`Prop::resolve_with_owner` は現在 flag 付き prop もゲートします。** 以前は `Prop::is_lazy()` ではない、eager 値**または** flag を運ぶ resolver のいずれも include set を参照せず解決していました。現在はあらゆる resolver-backed prop をゲートし、すでに materialize 済みの値だけを ungated で通します。その結果 `#[data(lazy(deferred))]` フィールドは、ほかのすべての lazy 種別と同じく、解決または通知される前にリクエストの `?include=` リストへ `?include=<field>` が必要です。フィールドをリクエストの `?include=` リストへ追加するか、そもそも opt-in にする意図がなかったなら `lazy(...)` 属性を取り除いてください。
- **スクロール prop の `reset` は、もはや merge-intent header に従いません。** カスタム infinite-scroll コンポーネントまたはテストスナップショットで `page.scrollProps[key].reset` を直接読むコードは、以前 `reset: true` でマージメタデータなしだった通常 revisit に、現在は `reset: false`（および `mergeProps` 項目）を見ます。公式の `<InfiniteScroll>` コンポーネントが異なるのは通常 revisit 上だけです。明示的な `router.reload()` だけでなく、すべての `router` `success` イベントで `reset` を listen するためです。したがって通常 revisit は、サーバーが実際に `X-Inertia-Reset` でキーを名付けない限り累積状態をクリアしなくなり、Laravel と一致します。古い「append/prepend ではない任意 visit は reset」動作に依存していた場所では、`X-Inertia-Reset: <key>` を明示的に送信してください。
- **`Prop::match_on` は `impl Into<String>` ではなく `impl MatchOnFields` を取ります。** 新しい bound により 1 回の呼び出しで複数フィールド（`match_on(["id", "slug"])`）を名付けられ、その impl list は意図して閉じています。`&str`、`String`、`[T; N]`、`Vec<T>` のみです。`IntoIterator` に対する blanket impl は使えません。`&str` および `String` の impl と coherence が衝突し、これらの型が後から `IntoIterator` impl を得ることを止めるものがないためです。以前コンパイルされた 3 つの引数型、`&String`、`Cow<'_, str>`、`Box<str>` は現在はコンパイルされません。呼び出し箇所では `&str` を渡してください。`&String` には `match_on(name.as_str())`、`Cow<'_, str>` には `match_on(name.as_ref())`、`Box<str>` には `match_on(&*name)` です。
- **ドット付き `only`/`except` 項目は、現在トップレベル prop を完全に除外する代わりに絞り込みます。** この修正より前の `X-Inertia-Partial-Data: user.name` は `should_include_eager` に正確な `"user"` 項目を探させ、見つからず prop 全体を黙って落としていました。`user` の 1 フィールドを求めるクライアントは何も得ませんでした。この穴に依存し、ドット付き `router.reload({ only: [...] })` をキー省略と同じものとして扱っていたフロントエンドのページコンポーネントは、現在代わりに `{ user: { name: ... } }` を受け取ります。コード変更は不要です。これは Inertia v3 プロトコルがすでにリクエスト/レスポンス契約に意味させているものです。同じ修正は `should_include_optional` にも適用され、その影響は運用上より大きいものです。ドット付き `only` 項目（`permissions.read`）は、以前は裸の項目（`permissions`）がなければまったく trigger しなかった `Optional` または `Defer` prop のトップレベルキーへの明示的リクエストとして現在は数えられます。以前その prop の resolver を完全に skip していたリクエストが今は実行します。resolver がデータベースまたは外部サービスへ到達するなら、ドット付き部分リロードトラフィックをすでに送っているクライアントは、以前にはなかった作業をリクエスト上で発行し始めます。ドット付き部分リロードトラフィックを持つ `Optional`/`Defer` prop があれば、アップグレード後に resolver 呼び出し量を監視してください。
- **`InertiaSharedData::share` は現在ページコンポーネント名を取ります。** `req` の後に `component: &str` パラメーターを加えてください。
  ```diff
  -async fn share(&self, req: &dyn InertiaRequestExt) -> Result<IndexMap<String, Prop>, FrameworkError>
  +async fn share(&self, req: &dyn InertiaRequestExt, component: &str) -> Result<IndexMap<String, Prop>, FrameworkError>
  ```
  プロバイダーがページごとに変える必要がなければ無視してください（`_component`）。Laravel の `RenderContext` は `ProvidesInertiaProperties::toInertiaProperties` に同じ組（`component`、`request`）を運びます。
- **`Prop` は enum ではなく struct です。** その variant はなくなりました。prop はメソッドを通じて構築および読み取りしてください。
  - `Prop::Eager(v)` -> `Prop::eager(v)`
  - `Prop::EagerNone` -> `Prop::absent()`
  - `Prop::Always(v)` -> `Prop::eager(v).always()`
  - `Prop::Lazy(r)` -> `Prop::from_resolver(r)` （`Prop::lazy(closure)` は変わりません）
  - `Prop::Optional(r)` -> `Prop::from_resolver(r).optional()`
  - `match prop { Prop::Eager(v) => … }` -> `prop.as_value()`
  - `matches!(prop, Prop::Lazy(_))` -> `prop.is_lazy()`; `matches!(prop, Prop::EagerNone)` -> `prop.is_absent()`
  `DeferConfig`、`MergeConfig`、`OnceConfig`、`ScrollConfig` の payload struct は削除され、そのフィールドは現在 `Prop` 上の flag です。`Prop::is_deferred()` は、それが常に意味していたものに合わせて `Prop::has_resolver()` に改名されました。`DeferOptions`、`OnceOptions`、`MergeStrategy`、`ScrollMetadata`、すべての `InertiaResponse` builder method は変わらないため、response builder だけを使うアプリは編集不要です。通常は `InertiaSharedData` 実装である、手で prop を作るアプリには上記の改名が必要です。
- **この修正は、ここから先のリクエストだけでなく、すでにあるセッションも保護します。** アップグレードだけで十分です。以前のリリースが書いたセッションクッキーには、一度もサニタイズされなかった `_previous.url` があり得ます。`SessionData::previous_url()` は現在、そのセッションがアップグレード後に最初に使われる読み取り時に、すでに保存済みだからと信頼する代わりにそれを破棄します。既存セッションを無効化したり、セッションテーブルを移行したり、再ログインを強制したりする必要はありません。プロトコル相対に見えるパス（`//host`）のリクエストも、今後は記録される以前の URL を更新しません。アプリの `fallback!` ルート（または通常でないパスで到達できる、`200` を返す任意ルート）が、そのようなパスを `Redirect::back()` の宛先にすることへ正当に依存していた場合、それはもうできません。いずれにせよ、セッション内の以前の安全な値は代わりにそのまま残ります（安全な値が何も記録されなかったなら `Redirect::back(fallback)` 自身の fallback が勝ちます）。すでにオープンリダイレクトリスクだった、この修正が閉じる正確な edge case に依存していた場合を除き、コード変更は不要です。
- **ページ内のすべての `errors.<field>` binding から `[0]` を外してください。** 新しい既定形では `errors.email` は文字列のため、`errors.email[0]` はメッセージではなく最初の文字をレンダリングします。同時に TypeScript 型を `string[]` から `string` へ変えてください。ページに触れたくなければ、`Inertia::install` に渡す config で `InertiaConfig::with_all_errors(true)` を設定し、`@inertiajs/core` の `errorValueType: string[]` module augmentation を追加してください。スターター frontend は新しい形を出荷します。
- **バリデーション失敗後のリダイレクトバックを手書きしていたハンドラは、それを削除できます。** ブリッジは現在自動であり、ハンドラが自身でリダイレクトしても動作を続けます。ミドルウェアは、値の入った `errors` object を運ぶ `422` にだけ作用するためです。
- **クラッシュした `suprnova serve` の子は、セッションを終える代わりに現在は再 spawn されます。** クラッシュで `suprnova serve` が即時終了すること（CI smoke check、終了を「何かがおかしい」と扱うスクリプト）に依存していた場合、正確に復元するには `--no-restart` を渡してください。リトライも既定で上限があります。連続で 5 回クラッシュしたプロセスは再試行されなくなります（`--restart-tries` で上限を上げるか、元の 1 クラッシュで終了する動作には `--no-restart` を使います）。
- **`Model::TOUCHES` はもはや inherent const ではありません。** `Comment::TOUCHES` を直接読んでいたコードには、親 touch cascade が読むために const がそこへ移ったので、スコープ内に `use suprnova::EloquentModel;`（または `suprnova::eloquent::EloquentModel`）が必要です。アプリで `grep -rn TOUCHES` を実行すればすべての呼び出し箇所が見つかります。const は以前 runtime で何もしなかったため、ほとんどのアプリにはありません。
- **`RelationEntry` にフィールドが加わりました。** 手で `RelationEntry` を構築するコードだけが変更を必要とし、literal へ `related_updated_at_column` を追加してください。フレームワークが出荷するマクロ生成リレーション登録はすでにそれを出力するため、`#[suprnova::model]` によるリレーション宣言だけをする通常のアプリは影響されません。
- **object ではない props を伴う `Router::view` は、現在起動時に panic します。** 以前は空の prop バッグで黙って登録していました。`view` は object（または `null`）を必要とする `Router::inertia` へ委譲し、そうでなければ panic します。`view` 呼び出しが object ではない props を運び得るなら、`Router::try_inertia` へ切り替えて `Err` を処理してください。それ以外には何も変わりません。
- **Inertia version manifest の既定は、ビルドが存在する瞬間に version string を変え得ます。** `X-Inertia-Version: 1.0` をハードコードするアプリまたはテストは、Vite manifest がディスク上に現れるまでだけ動作します。一度現れると version は manifest hash になります。古い定数が必要なら `VersionResolver::from_manifest(path)` から自分で読むか、`.version(...)` を明示的に pin してください。アップグレード後の最初の deploy は、すでに接続したクライアントに完全ページリロードを 1 サイクル強制すると見込んでください。これは 1 回だけであり、この変更の目的です。manifest がない場合の fallback 値は `suprnova::MANIFEST_VERSION_FALLBACK` として export されるため、`"1.0"` を二度とハードコードする必要はありません。
- **`Inertia::install` と `global_middleware!` の登録を `bootstrap::register` の外へ移してください。** 新しい関数に置き、代わりに `.http_bootstrap(...)` へ渡します。scaffold の新しい形は、`.http_bootstrap(|| async { bootstrap::register_http_stack() })` として呼ぶ同期の `register_http_stack()` です。これを省略するアプリは、フロントエンド manifest がない場合の worker-boot failure を含め、現在の動作を保ちます。

## 1.2.4 - 2026-08-18

### セキュリティ

- **メンテナンスモードのバイパス用シークレットが、定数時間で比較されるようになりました。**`MaintenanceMiddleware`は、素の文字列比較でシークレットのURLを照合しており、これは最初に異なるバイトで戻ります。シークレットはリクエストのパスに載って運ばれるベアラー認証情報であるため、そのタイミングの差が、どれだけの長さのプレフィックスを正しく推測できたかを攻撃者に教えていました。比較は今では`subtle::ConstantTimeEq`を介してバイト長の全体にわたって走り、長さの不一致のときにだけショートサーキットします - すぐ隣にあるバイパスクッキーの比較と同じ形です。

- **`rules::Url`が、スクリプトURIを拒否するようになりました。**このルールは、`url::Url`がパースできるあらゆるスキームを受け付けており、そこには`javascript:`と`vbscript:`も含まれていたため、検証済みのURLが、`href`へレンダリングされたときに依然としてスクリプト実行のシンクになり得ました。今では、Laravelの`url`ルールの形（`Illuminate\Support\Str::isUrl`の`^(PROTOCOLS)://HOST`パターン）を適用します: スキームはLaravelの許可リストに載っていなければならず、`://`が続かなければならず、**さらに**空でないホストが続かなければなりません - Laravelのホストのグループには`?`がないため、リストに載ったスキームであっても、ホストが存在しないか空である場合は決してマッチしません。スキームのリストと、`://`に加えてホストという要件は、Laravelそのままです。ホスト自体は、Laravelの正規表現ではなく`url`クレートによってパースされるため、いくつかのエッジケースは依然として異なります - 範囲外のポートは、ここでは拒否され、あちらでは受け付けられますし、IDNホストの正規化も異なります。新しい`Url::protocols(&[...])`は、Laravelの`url:http,https`をミラーします。`HttpUrl`は今ではその文字どおりのシュガーであり、独自のメッセージを保ちます。**挙動の変更:** これまで通っていた、リストにないスキームのURLは、今では失敗します - それを受け付けるつもりだったのなら、`Url::protocols(&["myapp"])`でそのスキームを名指ししてください。挙動の変更はさらに2つあります。`mailto:`、`data:`、`tel:`は、名前としてはLaravelの許可リストに載っていますが、authority成分を運ばないため、今では失敗します。そして`file:///etc/passwd`形式のパス - 最後の2つのスラッシュの間に何もない`scheme://` - も、空文字列もまたホストではないため、今では同様に失敗します。どちらも、Laravel自身の`://`に加えてホストという規則から導かれます。

- **Inertiaのレスポンスが、あらゆる場所で`Vary: X-Inertia`を広告するようになりました。**このヘッダーは、ページオブジェクトのレスポンス自体にしか設定されていませんでした。リダイレクト、404、422、そして静的なレスポンスはどれも運んでいなかったため、URLだけをキーにした共有キャッシュが、ハードなブラウザナビゲーションに対してJSONのページオブジェクトを、あるいはInertiaのXHRに対してHTMLシェルを配信してしまう可能性がありました。新しい`InertiaHeadersMiddleware` - `Inertia::install`によって、3つのうち最も外側として登録されます - は、それをすべてのレスポンスに設定し、Inertiaの訪問での空の`200`を、クライアントが非Inertiaとして拒否するレスポンスではなく`303`の戻りへ変えます。`InertiaVersionMiddleware`は今では、自分の`409`の前にセッションを再フラッシュするため、フラッシュされたエラーは、クライアントの追いかけのページ全体のGETを生き延びます。

- **Inertiaのレスポンスに関する3つの修正。**`InertiaResponse::location_for(&req, url)`は、InertiaのXHRには`409` + `X-Inertia-Location`を、ハードナビゲーションには素の`302` + `Location`を返すため、SPAの外側で始まったOAuthやSSOの跳ね返しが、ボディのない`409`で行き止まりになることはもうありません。既存の`location(url)`は、常に`409`という形を保ちます。新しい`App::clear_history()`は、履歴クリアのフラグをセッションへフラッシュするため、それはログアウトのリダイレクトを生き延び、実際にレンダリングされるページへ着地します - レスポンスごとの`.clear_history()`は、ブラウザが捨てるリダイレクトにしか印を付けておらず、直前のセッションの暗号化された履歴を復号可能なまま残していました。そして`once`のプロップは、今では完全なInertiaの訪問でのみスキップされます: 明示的な`router.reload({ only: ['stats'] })`は、何も返さないのではなく、それを再解決します。

- **SESのトランスポートが、カスタムのメッセージヘッダーを送るようになりました。**`Mail::to(..).header("List-Unsubscribe", ...)`と`Mailable::headers()`は、`MAIL_DRIVER=ses`の下でサイレントに捨てられていました: `Content.Simple`のリクエストボディには`Headers`フィールドがなく、生のMIMEのビルダーは`OutgoingMessage::headers`を一度も読んでいませんでした - 他のあらゆるトランスポートはそれらを転送しているにもかかわらずです。SESの両方の経路が、今ではそれらを運びます - `Headers`はSES v2の`{Name, Value}`のリストとして、生のMIMEは実際のヘッダー行として - そのため、購読解除のリンク、スレッド化のヘッダー、ルーティングのヒントは、ドライバーの差し替えを生き延びます。ヘッダー名は、両方の経路で先に検証されます - CR、LF、NUL（Mailgunのトランスポートがすでに拒否しているのと同じ、注入用のバイトです）と、有効なRFC 5322のフィールド名でないもの（空白、コロン、非ASCII）です - そのため、ファイルを添付することが、メッセージが受け付けられるかどうかを変えることは決してありません。

### 修正

- **入れ子になったバリデーションの失敗が、422のボディへ届くようになりました。**入れ子になった構造体や、バリデーションされる`Vec<T>`の要素に対する`#[validate(nested)]`の失敗は、バリデーターとレスポンスの間で落とされていました: リクエストは正しく422で拒否されていたものの、`errors`のマップは空で返ってきていたため、メッセージは何もレンダリングされず、クライアントはどのフィールドに問題があったのかを知ることができませんでした。入れ子の失敗は今では、トップレベルのものと並んで、Laravelのドット区切りの記法 - `address.street`、`items.1.name`、`order.items.2.sku` - へ平坦化されます。

- **Inertiaのページオブジェクトの`url`が、クエリ文字列を保つようになりました。**`page.url`はリクエストのパスだけだったため、`/users?page=2&sort=name`への訪問に対して、クライアントは`/users`を記録していました。その結果、あらゆる戻る/進むのナビゲーションと、あらゆる`router.reload()`が、ページネーションのカーソル、ソート、フィルタなしでそのページを再生していました。今ではパスにクエリを加えたものになります - `InertiaVersionMiddleware`が`X-Inertia-Location`のためにすでに使っていたのと同じ導出であるため、デフォルトではこの2つはバイト単位で一致します。新しい`InertiaConfig::url_resolver(...)`は、*ページオブジェクト*がそのページをどう名指しするかを上書きします（Laravelの`Inertia::resolveUrlUsing`です）。バージョンの跳ね返しは、到着したURLを名指しし続けます。それが、ブラウザの取得しなければならないURLだからです。

- **`Inertia::install`が、その設定をすべてのレスポンスへ適用するようになりました。**`Inertia::install`へ渡された設定は、3つのフィールドについて読まれた後、捨てられていました。そのため、明示的な`.with_config(...)`なしで構築されたすべての`InertiaResponse`は、`InertiaConfig::default()`からレンダリングされていました。`--frontend react`でスキャフォルドされたアプリは、環境に`SUPRNOVA_FRONTEND`が設定されていない限り、Svelteのエントリポイントを配信し、Reactのrefreshのプリアンブルを出しませんでした。設定の上で有効にしたSSRは、レスポンスへ一度も届きませんでした。そして、ページオブジェクトのアセットバージョンは、バージョンのミドルウェアのリゾルバとは別の設定から来ていました。インストールされた設定は今では、コンテナのInertiaレジストリ上に保持され、`InertiaResponse::new`はそこから出発します。レスポンスごとの`.with_config(...)`は引き続き上書きし、`Inertia::install`を一度も呼ばないアプリは変わらず、（フェイルクローズで）失敗したインストールは何も保持しません。副次的な効果として、本番のViteのマニフェストは今では、レスポンスごとではなくプロセスごとに一度だけパースされます。

- **スキャフォルドされたアプリが、Inertiaプロトコルのミドルウェアをインストールするようになりました。**`suprnova new`が書き出す`bootstrap.rs`は、セッション、ロケール、CSRF、includeの各ミドルウェアを登録していましたが、`Inertia::install`を一度も呼んでいませんでした。そのため、生成されたアプリは`InertiaVersionMiddleware`も`Inertia303Middleware`も持たず、直前のバンドルをまだ走らせているブラウザは、デプロイの後にリロードするよう決して伝えられず、リダイレクトする`PUT`/`PATCH`/`DELETE`は、クライアントが元の動詞で追いかけてしまう`302`のままでした。この呼び出しは今では`SessionMiddleware`の後に - バージョンのミドルウェアのセッションの再フラッシュが機能する場所に - 着地し、アセットが変わったときに上げるための名前付きの`INERTIA_VERSION`定数を伴い、プロジェクトが生成されたときのフロントエンドをピン留めします（`--frontend react`なら`.frontend(Frontend::React)`）。そのため、HTMLシェルは、Svelteのものへフォールバックするのではなく、そのフレームワークのViteのエントリポイントをロードします。生成される`.env`は今では、それに合わせて`SUPRNOVA_FRONTEND`を設定します。`--api`のスターターは変わりません。フロントエンドを持たないからです。

- **`Queue::push_unique`が、キューに載ったジョブをスキップされたと報告しなくなりました。**戻り値は`matches!(outcome, Idempotent::Fresh(()))`で計算されており、これは`Idempotent::FreshUnfenced`を`false`へ畳み込んでいました - エンベロープは*プッシュされた*が、重複排除のリースがプッシュの途中で失われた、という結果です。その真偽値で分岐する呼び出し元は、これから走ろうとしているジョブが、重複として抑制されたと伝えられていました。3つの結果は今では、すべて網羅的にマッチされます: リースを失った場合は、ジョブとその一意キーを名指しする`warn`とともに`true`を返し、本物の重複だけが`false`を返します。`push_unique_later`と`later_unique`は、同じ経路を共有しており、一緒に修正されています。

### 変更

- **パリティの基準が、Laravel 13.25.0へ移りました。**13.23.0、13.24.0、13.25.0のリリースノートを、項目ごとにフレームワーク自身の表面まで追跡しました。Suprnovaのコード経路に届いたものはすべて、このリリースで修正されているか、[`parity.md`](../parity.md)の中に`not yet`または`by design no`と印の付いた行を持っています。

### アップグレード

2つの変更が、あなたの側でのコード変更なしに、動作中のアプリを変えうるものです。

- **`Inertia::install`へ渡す設定の項目が、効くようになりました。**それらは3つのフィールドについて読まれ、捨てられていました。あなたのインストール用の設定が`.ssr(...)`を設定している場合、SSRは今ではオンです: デプロイの前にワーカーを起動する（`suprnova ssr:start`）か、`.ssr(...)`の呼び出しを外してください。そこで設定した`.entry_point`、`.assets_base_url`、`.default_title`、`.encrypt_history(...)`も、今ではページへ届きます。

- **`rules::Url`が、より多くを拒否します。**これまで通っていて、もう通らなくなる値は次のとおりです。Laravelの許可リストの外にあるあらゆるスキーム（`javascript:`と`vbscript:`もその中に含まれます）。許可リストには載っているものの`://`のホストを運ばない`mailto:`、`data:`、`tel:`。そして`file:///path`のような、ホストが空の`scheme://`です。あるスキームを受け付けるつもりだったのなら、それを名指ししてください: `Url::protocols(&["myapp"])`。

## 1.2.3 - 2026-08-16

### 修正

- **日時キャストがデータベースネイティブの`CURRENT_TIMESTAMP`テキストを読み取れるようになりました。** `AsDateTime`、`AsImmutableDateTime`、`AsOptionalDateTime`は引き続き正規化されたRFC-3339を書き込み、読み取りではタイムゾーン付きのPostgreSQLテキストと、タイムゾーンを持たないSQLite/MySQL値も受け付けます。タイムゾーンを持たない値はUTCとして解釈されます。

## 1.2.2 - 2026-08-14

### 修正

- **属性ベースの書き込み全体で、nullableな非テキスト値をPostgreSQL上で扱えるようになりました。** 型付きの`Builder::update_all`と`Builder::upsert`、モデルを使わない`DB::table().insert/update`、多対多ピボットの追加属性は、明示的なJSON nullをSQLの`NULL`として出力し、nullでないすべての値は引き続きバインドします。これにより、PostgreSQLがbigint、integer、boolean、timestamp、およびその他の非テキストカラムに対して拒否する、テキスト型付きnullパラメータを送る代わりに、対象カラムの型が保持されます。複数行upsertは、形が不正な行の欠落または余分なカラムを黙ってnullに変換せず、拒否するようになりました。多対多ピボットの自動タイムスタンプは、テキストではなく型付きUTC日時としてバインドされます。

### セキュリティ

- **リリースゲートは、ワークスペース全体で休眠中のlockfileメタデータとコンパイル対象の依存関係を区別するようになりました。** Cargoはrust_decimalの未使用のオプション依存関係であるrkyv 0.7互換依存関係を`Cargo.lock`に記録します。ゲートは、rkyvもそのderive crateも、ワークスペースのどのメンバー、feature、target、依存関係エッジからも到達可能でないことを証明するようになりました。対応するRustSec例外は管理対象となっており、2026-11-14に期限切れになります。rust_decimalがこのレガシーなオプション依存関係を記録しなくなった時点で削除する必要があります。

## 1.2.1 - 2026-08-09

### 変更

- **SuprnovaはGitHubの`entrepeneur4lyf` organizationから`eas4ai`へ移動しました。** パッケージメタデータ、ドキュメント、依存関係の例、scaffoldテンプレートにあるリポジトリURLは、`github.com/eas4ai`を使うようになりました。新しいプロジェクトでは、監視対象の作者メールアドレス`shawn@eas4ai.com`も使われます。このリリースによるruntime動作の変更はありません。

## 1.2.0 - 2026-08-05

### 追加

- **マニュアルが7言語で提供されるようになりました。** `manual/es/`、`manual/fr/`、
  `manual/de/`、`manual/pt-BR/`、`manual/ja/`、`manual/zh-Hans/` のそれぞれが、全104章のマニュアル - すべての章、目次、そしてこの変更履歴 - を英語のソースから翻訳して収めています。英語は引き続き正典です: 章の構成、コードブロック、識別子、
  CLIコマンド、環境変数はソースとバイト単位で同一に保たれているため、翻訳された章がフレームワークの動作について英語と食い違うことはあり得ません - 読者の言語で語り直すだけです。

  翻訳は suprnova.app のために作成・レビューされました。同サイトはこのマニュアルを
  `/docs` としてレンダリングしています。各セクションはそこでレビュー台帳を持ちます:
  評決は英語と翻訳の両方のコンテンツハッシュに対して記録され、セクションが承認と数えられるには、2人の独立したレビュアーが正確に同じバイト列を承認しなければなりません。また、言語ごとの用語集が用語の裁定 - どの用語を英語のまま残し、どれを母語の語にするのか、そしてその理由 - を固定します。修正はどちらのリポジトリでも歓迎です - ここでの修正は、次回の同期でサイトに届きます。

## 1.1.0 - 2026-08-02

### 追加

- **ロケール単位のフォールバックチェーン。**`LocalizationConfig`に`parents`が追加されました（`APP_LOCALE_PARENTS`という、カンマ区切りの`child=parent`ペア、またはチェーン可能な`.parent(child, parent)`ビルダーです）。ロケールは、グローバルな`fallback_locale`へさらにフォールバックする前に、設定済みの兄弟ロケールを継承できます - `pt-BR`からの`pt-PT`、`en-GB`からの`en-AU`、というように、推移的に続きます。`Lang::get`/`try_get`/`get_with`/`try_get_with`/`has`はすべて、現在のロケールを先頭にしてこのチェーンをたどるため、これはバンドル済みのものだけでなく、あらゆる`Translator`ドライバーで機能します。不正な形式のペア、無効なロケール、二重に名付けられた子、あるいは循環（ロケールが自分自身を親として名付ける場合を含む）は、リクエスト時に劣化する代わりに、設定読み込み時にはっきりと失敗します。

  配信されるカタログは、事前にチェーンをフラット化した状態を保ちます: `FluentTranslator`は、各ロケールの`/_suprnova/lang/<locale>.ftl`カタログを畳み込みとして構築するようになりました - `en`/`en-*`ロケール向けの埋め込みフレームワークカタログを一番下に置き、続いてそのロケールの設定済み親チェーン、そして最後に自身の`*.ftl`ファイルという順です - そのため、チェーンされたロケールであっても、ブラウザが一度だけフェッチする自己完結型の単一ファイルのままであり、クライアント側でチェーンを意識する必要はありません。フラット化がカバーするのは設定済みの親のみです。末端の`fallback_locale`は、依然として`Lang`ファサードレベルのフォールバックであり、配信されるバイト列には焼き込まれません。

  これにより、差分形式のカタログが実用的になります: `lang/pt-PT/`ディレクトリは、`lang/pt-BR/`から実際に異なるわずかな文字列だけを保持でき、カタログ全体を複製する必要はありません。それを可能にするマージは、Fluent ASTレベルで動作します - 子の値が親の値を置き換え、アトリビュートは名前でマージされ（アトリビュートに言及しないオーバーライドが、そのアトリビュートを失うことはもうありません）、選択式は丸ごと置き換わり（CLDRの複数形カテゴリはロケール依存のため、バリアント単位のマージは筋が通りません）、子だけが持つエントリは追加されます。完全な契約については、`manual/localization.md`の新しい「フォールバックチェーン」セクションを参照してください。

### 変更

- **`LocalizationConfig`に`parents`フィールドが追加されました。**`from_env()`とビルダーは影響を受けません。リテラルな構造体コンストラクタ（`LocalizationConfig`を手作りで組み立てるテストなど）は、フィールドがもう1つ必要になります。
- **配信されるカタログのテキストは、すべてのロケールについてシリアライザで正規化されるようになりました。**ロケール内の複数ファイルマージ（1つのロケールディレクトリ内に複数の`.ftl`ファイルがある場合）も、単純なバンドルの上書きではなく、親チェーンと同じASTレベルのマージを通るようになりました。解決される翻訳結果は、以下の2つの厳密な改善点を除いて変わりません。ただし裏側のバイト列はいずれにせよ入れ替わります - `ETag`/`?v=<hash>`はアップグレード時に一度だけローテートします。改善点は次のとおりです: オーバーライドが、言及していないアトリビュートをサイレントに失うことはもうありません。また、アトリビュートのみのオーバーライドが、メッセージ自身の値を消してしまうこともなくなりました（以前はエラーになるか、フォールバック解決になっていました。今では、より前のオーバーライドの値に解決されます）。

## 1.0.0 - 2026-08-02

### 追加

- **ローカライゼーション。**`lang/<locale>/*.ftl`内のメッセージカタログ（[Fluent](https://projectfluent.org)）、`__!("key", name: value)`マクロを備えた`Lang`ファサード、リクエストごとのロケール検出（`LocaleMiddleware`: セッション → クッキー → `Accept-Language` → `APP_LOCALE`）、そしてICU4Xを介した数値、通貨、日付、時刻、リスト、相対時間のロケール対応フォーマットです。`manual/localization.md`がその章です。

  組み込みのバリデーションルールは、英語をハードコードしなくなりました。それぞれが、キー付きメッセージ（`validation-min`とその引数、そして英語のフォールバック）を返し、シリアライズ境界で一度だけ翻訳されます - そのため、スペイン語のアプリは`lang/es/validation.ftl`を投入するだけでスペイン語のバリデーションエラーを得られ、ルールのラップもフレームワークのメッセージのフォーク版も不要です。フィールド名は、`field-<name>`のルックアップを通じて人間可読な形になります。`Rule::passes`（および`ContextualRule`/`AsyncRule`）は、`Result<(), ValidationMessage>`を返すようになりました。カスタムルールの`Err("…".into())`という本体は、引き続きコンパイルが通り、そのままの形でレンダリングされますが、あなたの`impl`のシグネチャは新しい型を必要とします。

  ブラウザは、サーバーが解決したものと同じバイト列を受け取ります: マージ済みのカタログは、ETagとイミュータブルな`?v=<hash>`形式を伴って`/_suprnova/lang/<locale>.ftl`で配信され、3つのスターターキットはそれを`@fluent/bundle`でパースし、`suprnova generate-types`は`MessageKey`のユニオン型を出力するため、メッセージをリネームするとTypeScriptコンパイラがすべての呼び出し箇所を指し示します。

  Laravel流のPHP配列ではなくFluentを選んだのは、1つのフォーマットがサーバーとブラウザの両方に対応しなければならないからであり、またロシア語、ポーランド語、アラビア語を正しく扱えるのはCLDRの複数形カテゴリだからです - `trans_choice`の整数レンジではそれができません。だからこそ、ここには`trans_choice`はありません。デフォルトで有効な`localization`フィーチャーの裏にあります。`--no-default-features`でも、埋め込み済みの英語フォールバックを使って、引き続きコンパイルが通り、引き続きバリデーションも動作します。

- **`Paginator`向けの`IntoInertiaScroll`。**このトレイトは`LengthAwarePaginator`と`CursorPaginator`には実装されていましたが、シンプルなページネーターには実装されておらず、そのため`simple_paginate`の結果は`Inertia::paginate`にまったく渡せませんでした - `simple.rs`自身のモジュールドキュメントが、それをURL生成の経路として指し示しているにもかかわらずです。そのせいで、オフセットページネーションのInertiaコレクションは、リクエストごとの`COUNT(*)`と、スクロールメタデータの手組みとの間で選択を迫られていました。`next_page`は、計算された最終ページではなく、`LIMIT n+1`のオーバーフロー探査から得られます。合計値がないため、そこから計算するものがないからです。

### 修正

- **`suprnova generate-types`が、実行のたびに異なるファイルを出力していました。**トポロジカルソートは、`HashMap`をイテレートして作業キューの種を作っていました。Rustはプロセスごとにハッシュのイテレーション順序をランダム化するため、連続した実行が同じインターフェースを違う順序に並べていました。出力はコミットされる成果物であるため、実行のたびに差分が生まれていました - そして、理由もなく変動する生成ファイルは、人々が再生成をやめてしまうファイルであり、そうなると、それが説明しているはずのRustを静かに描写しなくなります。ディレクトリの走査もソートされるようになったため、出力はファイルシステムの順序にも依存しなくなりました。同じソースからの2回の実行は、今ではバイト単位で同一になります。

- **`topological_sort`は、自身のドキュメントコメントと正反対の動作をしていました。**依存先より先に依存元を出力していたのです。実害はありません - TypeScriptのインターフェースは、同じファイル内で後から宣言されるものを参照できるからです - そのため、順序ではなくコメントの方を修正しました。順序を直すと、何の得もないままコミット済みファイルをかき乱すことになります。

## 0.9.1 - 2026-08-01

3件の不具合です。いずれも、コードを読んで見つかったのではなく、コンテナ化されたハーネスの下でドッグフードアプリを走らせて見つかりました。そのすべてが、本番環境がプロセスを止めるような形でプロセスを止めることのないテストスイートには見えません。

それらは特定の順序で積み重なります: ローリングデプロイが、ジョブの途中でワーカーをSIGKILLし（1つ目）、そのジョブは、試行回数を一度もカウントしなかった再取得パスをたどります（2つ目）。

### 修正

- **`schedule:work`、`queue:work`、`workflow:work`が、SIGTERMを無視していました。**それぞれが`tokio::signal::ctrl_c()`だけをセレクトしており、これはSIGINTハンドラをインストールします - そのため、プロセス内のどこにもSIGTERMのハンドラが存在せず、しかもSIGTERMこそ、`docker stop`、Coolify、systemd、Kubernetesが送ってくるものです。3つとも、その`select!`の裏にはすでに慎重な有界ドレインを備えていました。ただし、それがスーパーバイザーの下で実行されたことは一度もありませんでした。修正前に計測したところ: `queue:work`コンテナへの`docker stop`は、40秒の猶予ウィンドウをまるごと使い切り、進行中のジョブを破壊したまま終了コード137で終了しました。PID 1として - コンテナが実行するのはこれです - カーネルはハンドルされていないSIGTERMをまるごと捨ててしまうため、プロセスは不格好に死んだのではなく、SIGKILLが来るまでまったく死にませんでした。`Server::run`はすでに両方のシグナルを正しく扱っており、そのリスナーは今では共有されているため、これはスケジューラーのループにおける、シグナルを取りこぼす窓も閉じます。

- **ワーカーを道連れにするジョブは、決してデッドレターに送られませんでした。***ハンドラ*が失敗するジョブはNACKされ、その試行がカウントされるため、`max_tries`の後にデッドレターに送られます。*ワーカーを道連れにする*ジョブ - OOM、アボート、セグフォルト、あるいは上記のSIGKILL - は何も決着しません。その予約は失効するだけで、どのドライバーも、かつてはそれをバイト単位で同一のまま再配送していました。そのようなジョブは不死身です: それを掴んだワーカーを次々に道連れにし、変わらぬ姿で戻ってきては次のワーカーを道連れにする、ということを、何かがワーカーを再起動し続ける限り繰り返します。3つのドライバーはすべて、ワーカーが死んだと判明した時点で試行をカウントするようになりました。`QUEUE_DRIVER`を切り替えても、ポイズンジョブを止められるかどうかが変わってはならないからです。`attempts`は今や、「ハンドラの失敗」ではなく「ワーカーへの配送」を意味します - `manual/queues.md`にドキュメント化されています。無関係な理由で失われたワーカーもまた、試行を1つ消費するからです。

- **…そして、使い果たされたジョブは今、ディスパッチされる前にデッドレターに送られます。**試行をカウントするだけでは、必要ではあっても十分ではありませんでした。あらゆるデッドレターの判定は、ハンドラが復帰することを前提とするワーカーの決着パスの中にありました - そのため、まさに復帰できなかったジョブに対してだけ、それが一度も実行されなかったのです。ドライバーの修正だけでは、カウンターは上昇するものの（計測値: 殺されたワーカー3台にわたって0 → 1 → 2）、それに対して何のアクションも起きませんでした。予算は今、ハンドラが走る前に使い切られます。最初の修正が正しく見えた後、コンテナ実験を再実行して初めて捕まりました。

- **デーモンには、tracingサブスクライバーがありませんでした。**`serve`は`init_telemetry`からそれを受け取ります。一方、`queue:work`、`schedule:work`、`schedule:run`、`workflow:work`は別の起動パスを通るため何も受け取っておらず、それらが発する`tracing::`の行はすべてどこにも届かず、`LOG_LEVEL`はそれらに対して無効でした。それこそが、これらが伝えるべきことのほとんどです - ジョブをデッドレターに送るワーカー、取りこぼしたティックをスキップするスケジューラー、解放できなかったロック。コンテナの中では、目に見える出力は起動時のバナーだけであり、プロセスはそのすべてを行いながら、アイドル状態に見えていました。このリリースにあった不具合のうち2件は、これが修正されるまで見えませんでした。

- **失敗ジョブストアが束縛されていない状態でのデッドレターは、サイレントな削除でした。**永続化ステップは`if let Some(store) = ..`の内側にあったため、ストアがないとこのアームはマッチせず、実行はACKへと素通りしていました - すぐ上にある失敗パスより静かで、そちらは少なくとも予約をそのまま残します。ストアが存在しないことは、壊れたストアより成功として扱われていたのです。今では、完全なエンベロープをERRORレベルでログ出力するようになりました。それこそが、`queue:retry`が再投入する対象だからです: 手作業で復旧できる作業と、消えてなくなった作業との違いです。

- **`QUEUE_DRIVER=database`は、失敗ジョブストアを束縛するようになりました。**`failed_jobs`は、そのドライバーの契約の一部です - `queue:retry`がそれを読み、`Queue::retry_failed`はそれなしには動作できません - しかし`bootstrap_from_env`はドライバーを配線する一方でストアを未設定のままにしていたため、アプリが手作業でストアを束縛しない限り、データベースバックエンドのキューは何もない場所へデッドレターしていました。`QUEUE_FAILED_DB_TABLE`経由で設定可能です。このドライバー限定です: `memory`は構造上一時的であり、`redis`には書き込む先のテーブルがありません。

- **Redisの再取得レイテンシは、`--visibility-timeout`に従うようになりました。**このフラグはXAUTOCLAIMのアイドル閾値を設定しますが、コンシューマーがどれくらいの頻度で確認するかは別のクロックが支配しており、ドライバーはそれをsea-streamerの30秒というデフォルトのままにしていました - そのため、`--visibility-timeout 5`は実際には「最大35秒」を意味していました。この間隔は今では設定済みのタイムアウトに追従し、1秒から30秒の範囲にクランプされるため、短いタイムアウトがXAUTOCLAIMストームになることはなく、長いタイムアウトは以前より再取得を速くする以外の結果にはなりません。

### 追加

- **`TaskBuilder::on_one_server()` / `on_one_server_for(ttl)`** - レプリカ全体で、期限が来たティックごとにスケジュールタスクをちょうど1回だけ実行します。これがなければ、そのティックのリーダーを誰も選出しません: 各`schedule:work`プロセスは独立にスケジュールを評価するため、3つのレプリカが、期限の来たすべてのタスクを毎分3回ずつ、ばらつきなく実行することが計測されました。3レプリカ上の毎晩の課金ジョブは、顧客ごとに3回ずつ課金していました。

  `without_overlapping()`はこれをカバーしませんし、できません: そのロックはタスクにキーが振られ、ハンドラが復帰したときに解放されるため、速いタスクは2つ目のレプリカが確認する前にロックを解放してしまいます。`on_one_server`は、タスク*とティック*にキーを振り、ハンドラを超えてロックを保持し続け、TTLで失効させます。2つは組み合わせられます。

  オプトインで、Laravelと一致します。フェイルクローズする点でLaravelと異なります: 選出は、その裏にあるキャッシュがどれだけ共有されているかにしか依存しないため、`CACHE_DRIVER=memory`かつシングルサーバー向けタスクがある本番環境の起動は、問題のタスクの名前を挙げて拒否されます。本当に単一のスケジューラーしか動かさないデプロイのためには、`SCHEDULE_ALLOW_MEMORY_LOCK_IN_PRODUCTION=true`があります。

### 変更

- `manual/deployment.md`は、「`schedule:work`プロセスをちょうど1つだけ動かす」ことをもはや唯一の選択肢としては述べなくなり、新たに**「クリーンに停止する」**セクションを得ました。このセクションは、サブシステムごとのドレインウィンドウ、それらを上回るようプラットフォームの終了猶予をどう見積もるか、そしてPID 1がシグナルハンドラの欠落を見た目以上に悪化させる理由をカバーします。

## 0.9.0 - 2026-07-31

### セキュリティ

- **認証の発行は、呼び出し元単位でしかスロットルできず、受信者単位ではできませんでした。**アドレスをキーにした制限は「1つのクライアントがうるさいか」には答えられますが、「1つのメールボックスが溢れさせられているか」には答えられません。ボットネットや単一のIPv6`/64`に分散した攻撃者は、あらゆるIP単位の予算を下回ったまま、1人の被害者の受信箱をパスワードリセットメールで埋め尽くすことができ、それを止めたはずの制限をフレームワークの中で表現するものは何もありませんでした - キー関数はパス、ヘッダー、クエリ文字列は読めても、フォームエンコードされたボディは読めなかったため、アドレスは、まさにそれを運んでいるルート上で見えなくなっていたのです。

  `identity_key`は、操作対象のアカウントにバケットのキーを振ります。クエリ文字列を先に読み、続いてバッファ済みのフォームボディを読むため、1つのキー関数が両方の形をカバーします。値はトリムされ小文字化されます。`Alice@Example.com`は`alice@example.com`と同じメールボックスに届きますし、シフトキーを押しっぱなしにするだけで回避できる制限は制限とは言えないからです。そしてハッシュ化もされます。レート制限のバックエンドは、多くの場合、プライマリデータベースよりアクセス制御が弱い共有Redisだからです。

  それを支えるのは、2つの新しいミドルウェアビルダーです。`key_reads_body(cap)`は、キーを振る前にボディをバッファします - オプトインです。バッファリングは、未認証の呼び出し元があなたにやらせることのできる作業であり、上限を超えるボディはキーなしで通過させるのではなく413で拒否されるからです。`only_when(pred)`は、自分に関係のないリクエストに対してはリミッターを丸ごとスキップします。これが、積み重ねられた受信者単位の予算が、受信者を指定しないルート上でサイレントに拘束力のある制限になってしまうのを防いでいます。

  ドッグフードアプリは今、発行グループの上に両方を積み重ねています: アドレスあたり5分に10回、受信者あたり15分に3回です。

Toriiのセッション、パスワード、OAuth、パスキーの各パスをレビューしたところ、8件の不具合が見つかり、すべてピン留めされたフォーク（`suprnova-torii-rs` `968b0be`）で修正されました。

- **期限切れのセッションが、リフレッシュによって息を吹き返すことがありました。**SeaORMのセッションリポジトリの`refresh`には有効期限の判定がなく、無条件に`expires_at`を延長していました。また`OpaqueSessionProvider::refresh_session`は、`get_session`が行う`is_expired()`チェックをスキップしていました。有効期限を過ぎて保持されたトークンが、無期限に更新され得たのです。両方の層で修正済みです。Suprnova自身の表面からは到達できません - `Torii`もフレームワークもセッションリフレッシュを公開していないからです - しかし、両クレートの公開APIではあります。
- **ログインフォームは、タイミングによってどのアカウントが存在するかを漏らしていました。**認証は、メールアドレスが一致しなかった時点でただちにリターンしており、Argon2をまるごとスキップしていました: 未知のアドレスで54µs、誤ったパスワードで719msと計測され、ネットワーク越しに読み取れる約13,000倍の差でした。どちらの失敗パスも、今ではダミーハッシュに対して検証を行うため、コストが同じになります。これは、Suprnovaのパスワードログインを通じて実際に到達*可能でした*。
- **JWTの`iss`クレームは、書き込まれてはいたものの検証されたことがありませんでした。**アルゴリズムのピン留めはすでに正しく行われていました - `alg: none`やHS/RSの混同は決して起こり得ませんでした - しかしissuerは飾りに過ぎず、署名鍵を共有する2つのサービスが互いのセッションを受け入れてしまう可能性がありました。issuerが設定されている場合、今では強制されます。
- **1回限りのはずのPKCEベリファイアが、2回クレームされることがありました。**消費は読み取りに続く削除という形だったため、同じ`csrf_state`に対する2つのOAuthコールバックが、どちらの削除も着地する前に、両方とも読み取れてしまうことがありました。今では1つの操作でクレームされます - Postgresでは`DELETE ... RETURNING`、SeaORMでは、影響を受けた行数で勝者を決めるプライマリキー削除です。
- **期限切れのセッションが、アクティブとして一覧表示されていました。**`find_by_user_id`には有効期限のフィルタがなく、期限切れの行はクリーンアップが走るまで残り続けるため、「サインイン中のデバイス」画面は、生きているセッションについては何も語らないまま、ユーザーに死んだセッションの失効操作を提供していました。
- **あるパスキーのルックアップが、`authenticate`と名付けられていました。**Toriiの`PasskeyService::authenticate_credential`はクレデンシャルIDを受け取り、それを所有するユーザーを返していました。そして`PasskeyAuth::authenticate`は、そこからセッションを発行していました。Toriiはパスキーを保存するだけです - WebAuthnへの依存を一切持たず、アサーションを検証できません。そのため、これらの呼び出しが証明できるのは、呼び出し元がクレデンシャルIDを知っていたということだけでした: ブラウザが平文で送信し、`allowCredentials`がセレモニーを開始できる誰にでも渡す値です。`find_user_by_credential`と`create_session_for_verified_credential`にリネームされ、どちらも検証が呼び出し元の仕事であることを文書化しています。Suprnovaを通じては到達できません。Suprnovaは`webauthn-rs`自体を自ら駆動し（`torii_integration::passkey`を参照）、Toriiにはクレデンシャルの保存のためにしか到達しないからです。
- **WebAuthnのチャレンジは、そのTTLの間ずっとリプレイ可能でした。**どちらのバックエンドも、読み取り時にチャレンジを消費しておらず、SeaORMの`get_challenge`は`expires_at`もまるごと無視して、期限切れのチャレンジを生きているものとして返していました。読み取りは今では両バックエンドで期限切れの行を除外し、新しい`take_challenge`が、ちょうど1回だけチャレンジをクレームします - PKCEの修正と同じ、削除が勝者を決める形です。

### 破壊的変更

- **Azure Blob StorageとGoogle Cloud Storageは、新しい`filesystem-azure`と`filesystem-gcs`フィーチャーの裏に移動しました。**`Storage::register_azblob`、`register_azblob_with`、`register_gcs`、`register_gcs_with`、`AzBlobConfig`、`GcsConfig`は、対応するフィーチャーを有効にしない限り、もう存在しません。どちらかのバックエンドを使っている場合は、依存関係に追加してください:

  ```toml
  suprnova = { git = "…", tag = "v…", features = ["filesystem-gcs"] }
  ```

  得られるのは、実行時の失敗ではなく、欠けている項目の名前を挙げるコンパイルエラーです。

  どちらのopendalサービスクレートも`rsa`を引き込みます。これはRUSTSEC-2023-0071（Marvinタイミング攻撃）を抱えており、上流に修正済みリリースがありません。これらは、`reqsign-core`のオプションの`rsa`が裏にある機能である`reqsign-core/jwt`を有効化する唯一のクレートだったため、それらをゲートすることで、3つのopendal経路すべてがそこへ至る道を一度に断ち切ります。`rsa`は今では*回避可能*です: `--no-default-features --features filesystem,database-postgres`は、それなしで解決でき、それでいてストレージサブシステムは維持されます。以前は、ストレージを何であれ維持したままそれを手放せるフィーチャーの組み合わせは存在しませんでした。

  標準のデフォルトビルドは、依然として`rsa`を抱えています - `database-mysql`はデフォルトフィーチャーであり、`sqlx-mysql 0.8.6`がそれに非オプションで依存しているためです - そのため、この監査上の例外は開いたままです。S3は意図的にゲートされて**いません**: `reqsign-aws-v4`は`jwt`なしで`reqsign-core`を使うため、S3ドライバーはそこへの経路に一度も関与しておらず、それをゲートすることは、何も取り除かないまま最も使われているクラウドバックエンドを壊すことになります。

### 追加

- **`suprnova --version`**、`-v`もclapのデフォルトである`-V`と同様に使えます。CLIに対して、他のあらゆるCLIが使うフラグでバージョンを尋ねたときに、使用方法のエラーが表示されるべきではありません。

### 修正

- **2つのRedis操作に、上限がありませんでした。**キャッシュのタグフラッシュは、タグのメンバー集合全体を`SMEMBERS`で読み取り、キーを1つずつ削除していました。そのため、メンバー数の多いタグはコネクションを詰まらせ、読み取りと削除の間に並行書き込みが失われることもあり得ました。タグは今では世代ベースになり、アトミックにフラッシュされ、上限付きの`SSCAN`でスキャンされます。遅延キューの昇格パスは、期限が来たすべてのジョブを1つの無制限な`ZRANGEBYSCORE`で移動させていたため、まとめて期限が来た滞留は、1つの巨大なスクリプトを生んでいました。今ではバッチ単位で昇格します。
- **2つのシャットダウンドレインが、永遠に待ち続けていました。**Ctrl-C時の`schedule:work`と、キャンセル後のワークフローワーカーは、どちらも期限なしにすべての進行中タスクをawaitしていたため、決して復帰しない1つのタスクが、`SIGKILL`が来るまでプロセスを開いたまま保持していました - オペレーターの目には、「止まらない」デーモンとして映ります。どちらも今では、有界の猶予を待ってから残りをアボートし、その件数を報告します。
- **リリースのバージョンピン留めスイープは、2つあるピン留め構文のうち片方しか認識していませんでした。**そのため、`cargo install --tag vX.Y.Z`という行を持ちながら依存関係のスニペットを持たないファイルは、一度も発見されませんでした。`suprnova-cli/README.md`は、3リリースにわたって読者にv0.6.0のインストールを案内し続けていました。`manual/cli.md`と`manual/cli-new.md`はv0.7.2のまま止まっていました。`manual/installation.md`は両方の形式を抱えており、片方だけが上がって、もう片方は凍りついていました。発見と書き換えは今では1つのパターンテーブルから読み取るようになり、ファイルのルールはその内容から導出されます。
- **`cargo doc`は、`filesystem`はあるが`testing`はないビルドすべてで失敗していました。**7つの`Storage::fake`イントラドキュメントリンクが解決できず、`lib.rs`は壊れたリンクを禁止しているためです。`testing`はデフォルトフィーチャーであるため、その組み合わせをビルドするゲートステップは一度も存在しませんでした。`check-feature-matrix.sh`は今ではビルドします。
- **Toriiのマイグレーションは、自分自身のスキーマの上で再生できませんでした。**そのため、`torii_migrations`という追跡テーブルを持たないままそれを保持しているデータベース - それをスキップしたダンプから復元されたものであれ、手作業でマイグレーションされたものであれ - は、管理下に置くことができませんでした。すべての`Table::create()`は`.if_not_exists()`を伴っていましたが、19個の`Index::create()`呼び出しはどれも伴っておらず、`ADD COLUMN locked_at`のalterも同様だったため、再生はテーブルを通り抜けた末に、最初の`CREATE INDEX`で息絶えていました。`IF NOT EXISTS`ではなく`has_index`/`has_column`を介して、ピン留めされたフォーク（`suprnova-torii-rs` `a0f956d`）で修正されました。sea-queryはそれをMySQL向けにサイレントに落としてしまうため、構文だけの修正では、デフォルトフィーチャーのビルドは壊れたままだったはずです。
- **失敗したToriiのマイグレーションは、エラーを返す代わりにプロセスをアボートしていました。**`SeaORMStorage::migrate`はマイグレーターをunwrapし、無条件に`Ok(())`を返していたため、失敗を`FrameworkError`へマッピングする`init_torii`側の処理は、到達不能なコードになっていました。
- **アプリ自身の`users`テーブルが、Toriiのものをサイレントに抑え込んでしまうことがありました。**`.if_not_exists()`は、「すでに自分のもの」と「すでに他の誰かのもの」を区別できないためです。マイグレーションは成功を報告し、認証は後になってカラム不足で失敗していました - これが、`--api`スターターがそのテーブルを`app_users`と名付けている理由です。Toriiのマイグレーションは今では、既存の`users`テーブルに必要なカラムが欠けている場合、マイグレーション時にそのカラムと対処法を挙げて警告します。既存のデプロイが起動し続けられるよう、ハードな失敗ではなく警告のままにしてあります。
- **RailwayとDigitalOceanのデプロイガイドは、プラットフォームのヘルスチェックを、Postgresをプローブし得るパスに向けていました。**どちらのプラットフォームも、そのチェックが失敗するとコンテナを再起動するため、そのアドバイスに従うと、データベースの瞬断がすべてのレプリカにまたがる再起動ループへと変わってしまいました。どちらも今では`/_suprnova/health/live`を使い、データベースはコンソールから手作業でプローブします。旧来のパスは引き続き解決します。すでにデプロイ済みのものに変更は必要ありません。

## 0.8.0 - 2026-07-30

外部のレッドチーム監査に対する是正対応です。監査は19件のP1指摘と、1.0に対するNO-GO判定を返しました。このリリースは、**19件すべて**を閉じます。加えて、それらを修正する過程で見つかった、監査が名指ししていなかった不具合もいくつか閉じます。

いくつかの修正は、サイレントな設定ミスを、意図的に起動拒否へと変えます。デプロイする前に**アップグレード**を読んでください - 問題なく動いていた本番アプリが、起動しなくなるかもしれません。

### アップグレード

以前は警告付きで（あるいはサイレントに）起動していた3つの設定が、今では本番環境でフェイルクローズします。それぞれのエラーは、それを解除する変数の名前を挙げ、リスクが本当に存在しないデプロイのためには、それぞれに明示的なオーバーライドが用意されています。

- **配信しないメールドライバー。**`MAIL_DRIVER`が未設定、`log`、`memory`、あるいは未知の値のいずれであっても、メールをレンダリングして破棄するトランスポートに解決されていました - そのため、パスワードリセットは、何も送信されないまま成功を報告していました。オーバーライド: `MAIL_ALLOW_NON_DELIVERING_IN_PRODUCTION=true`。
- **平文のSMTP。**4通りの認証情報の組み合わせのうち3つが、暗号化されていないトランスポートに帰着しており、両方とも未設定のケースは警告をログに出しながらもとにかく送信していました。オーバーライド: `MAIL_ALLOW_INSECURE_SMTP_IN_PRODUCTION=true`。
- **インメモリのレート制限。**そのバケットは1つのプロセスのヒープ上に存在するため、N台のレプリカの裏では、あらゆるクォータが実質N倍になり、デプロイのたびにリセットされます。`RATE_LIMIT_DRIVER`を`redis`に向けるか、本当に単一プロセスしか動かさないのであれば`RATE_LIMIT_ALLOW_MEMORY_IN_PRODUCTION=true`を設定してください。*未知の*ドライバー値も同じ理由で失敗します。メモリへフォールバックしていたからです - `RATE_LIMIT_DRIVER=Redis`のように大文字始まりのものは、設定されているように見えるため、本番環境に到達してしまう可能性が最も高いケースです。

開発、テスト、ステージングは、この3つのケースすべてにおいて変わりません。ステージングは意図的にゲートされていません: そこをハードに失敗させると、チームはオーバーライドをグローバルに設定するようになり、それは肝心なところでチェックを無力化してしまいます。

起動失敗ではない、2つの挙動変更です:

- **`fill`と`first_or_new`は、不正な形式の値を拒否します。**フィールドの型へデコードできない値は、以前はそのフィールドの`Default`になった上で`Ok`を返していました - `fill(attrs!{ age: "abc" })`は`age = 0`をセットし、成功を報告していました。今では、そのフィールドの名前を挙げた`ValidationError`を返し、モデルには触れません。未知のカラムは引き続きサイレントにスキップされ（Laravel互換）、数値の拡幅も引き続き機能します。
- **`/_suprnova/health?db=true`は、もうドライバーのエラーを返しません。**詳細はログへ移動しました。ボディは`"database": "error"`を保持し続けます。デバッグビルドには引き続き含まれます。`status`/`database`をパースするダッシュボードへの影響はありません。
- **`url::signature_has_not_expired`は、有効な署名を要求するようになり**、非推奨になりました。以前は、偽造されたURLに対しても`true`を返していました - 不正な署名は「期限切れ」ではありません。取り逃す期限そのものを一度も持っていなかったからです - そのため、それだけをガードにしていたハンドラは、偽造を受け入れていました。今では`has_valid_signature`と同一です。*期限切れ*と*無効*を区別するためにこれを使っていた場合（403の代わりに「新しいリンクをリクエストしてください」を描画するためなど）、3つの状態すべてを返す`url::signature_verdict`に切り替えてください。これはLaravelの`URL::signatureHasNotExpired`とは、意図的に異なります。

オプトインした場合にのみ、あなたの側で何かが必要になる、2つの追加機能です:

- **`QueueDriver`に`settle`と`release`が追加されました**。どちらもデフォルト実装を持つため、既存のドライバー実装は変更なくコンパイルが通り続けます。あなたのバックエンドが、後続の書き込みとACKを1つのトランザクションでコミットできるなら`settle`を実装してください。予約済みのメッセージをその場でリキューできるなら`release`を実装してください。
- **バッチの集計を、永続化できるようになりました。**`DatabaseBatchRepository`は、`job_batches`と`job_batch_settlements`という2つの新しいテーブルを必要とします - `jobs`や`failed_jobs`と同様に、あなたのマイグレーションに追加してください。スキーマは`manual/queues.md`にあります。`MemoryBatchRepository`のままであれば、何も変わりません。

### セキュリティ

- **Slowloris（SEC-07）。**hyperのヘッダー読み取りタイムアウトは、30秒とドキュメント化されていましたが実際には無効でした - コネクションビルダーにタイマーがインストールされている場合にのみ作動するところ、インストールされていなかったのです。クライアントは、コネクションと`SERVER_MAX_CONNECTIONS`のパーミットを、無期限に保持できました。今では作動し、`SERVER_HEADER_READ_TIMEOUT`経由で設定可能です。
- **マルチパートアップロード（SEC-05）。**上限は個々のパートのペイロードには適用されていましたが、生のストリームには適用されておらず、そのためボディは合計で上限を超えることがありました。今ではストリームで上限が課されます。
- **空のキーを使ったWebhook HMAC（SEC-08）。**どちらの支払いアダプターも空のシークレットを受け入れており、これは何でも検証を通してしまいます。両方で拒否するようになりました。
- **Paddleの署名パース（P2-11）。**奇数長、あるいは非16進の`paddle-signature`が、ピン留めされたSDKに到達し、その内部でパニックしていました。今では先に検証されます: 不正な形式の署名は401になります。
- **パスキーの登録とリセットトークン（SEC-01、SEC-02）。**既存のメールアドレスに対する匿名の登録、非所有者による登録、そして直近の再認証を伴わない所有者による登録は、それぞれ別個のステータスで拒否されます。パスワードログインは今では、再認証ウィンドウのタイムスタンプを刻むようになりました。
- **`dev:tls`（SEC-10）。**プロジェクトが、このコマンドが信頼するCAを選べてしまっていました。
- **生成されるDocker Compose（P2-12）。**このリポジトリにコミットされた認証情報のまま、PostgresとRedisをすべてのインターフェースに公開していました。今ではループバックに束縛され、スキャフォルドごとに生成されるパスワード、0600で書き込まれる`.env`、そしてシンボリックリンクされたターゲットの拒否が備わっています。
- **ヘルスエンドポイント（P2-01、CI-05）。**データベースへクエリを投げるかどうかを、`query.contains("db=true")`という部分文字列テストで決めていたため、`?nodb=true`でもプローブが走っていました。今では正しくパースされます。503は、ホスト、ポート、スキーマ、バージョンを名指ししていたドライバーのエラーを、もう埋め込みません。
- **認証情報発行のスロットリング（P2-02）。**リファレンスアプリの4つの認証発行ルートには、レート制限がまったくありませんでした。唯一レート制限を持っていたルートも、生の`x-forwarded-for`ヘッダーにバケットのキーを振っていました - これはどんなクライアントでも、リクエストごとに変えて新しいバケットを得ることができます。両方とも修正されました。発行の予算は4つのルート間で共有されるため、それらを切り替えても予算が倍増することはありません。
- **再配送されたチェーンのステップが、新しいidの下で後続を再プッシュしていました（DATA-02b、部分的）。**決着処理は、ACKする*前に*次のチェーンのリンクをプッシュします。これは意図的なものです: 先にACKすると、そのウィンドウでのクラッシュがチェーンを永久に失わせてしまいますが、重複はサイレントな消失とは違って回復可能だからです。しかし、後続のエンベロープはプッシュのたびに新しい`Uuid::new_v4()`を得ていたため、そのトレードオフによって生じた重複は、正当な新しいステップと区別がつきませんでした - ドライバーにとっても、アウトボックスにとっても、ハンドラにとっても。

  最後のそれこそが、本当のコストです。フレームワークの配送契約はat-least-onceであり、重複に対する答えは「ハンドラはべき等でなければならない」です - しかし、受け取る唯一の識別子である`env.id`にキーを振ったハンドラは、チェーンされたジョブについてはその契約を満たせませんでした。重複がそのたびに新しいidの下で届いていたからです。その契約は、構造上満たしようがなかったのです。

  後続のidは今では、先行のidから導出されたUUIDv5になりました。これは、その先行自身の再配送をまたいで安定しています。再配送されたステップは、以前にプッシュしたidを再プッシュします。スキーマの変更も、新しいフィールドも、新しい依存関係もありません。

  これにより、重複は**検出可能**になります。これが、DATA-02bの残りの部分に欠けていたプリミティブです。これは、プッシュをACKとアトミックにするわけではなく（それにはアウトボックスが必要です）、入ってくる重複を拒否するものもまだありません。どちらも未解決のままです。
- **署名付きURLは、あるURLを検証しながら別のURLを実行していました（SEC-04）。**正規化された形式は、クエリのペアをマップへと畳み込んでいたため、繰り返されたキーは**最後**の値だけを保持していました - 一方、`Request::query_param`は**最初**の値を返していました。そのため、正当に署名された`?user=victim`は、元の署名をそのままに`?user=attacker&user=victim`としてリプレイできてしまいました: 検証は`victim`で正規化されて通過し、ハンドラは`attacker`に対して動作していました。

  正規化された形式は今では、`(key, value)`でソートされたすべてのペアを保持するため、署名はパラメータの正確なマルチセットをカバーします - どの値を追加、削除、置換してもHMACが壊れます。繰り返された`signature`や`expires`は、まるごと拒否されます。どちらであれ2つあると、どちらが支配するかについて恣意的でない答えが存在しなくなるからです。

  `Request::query_param`は今では、繰り返されたキーを最後の値に解決するようになり、`query_params`や`Context::query_param`と一致します。3つのうち、食い違っていたのはこれだけであり、その食い違いこそが不具合のもう半分でした。**既存の署名付きリンクは、引き続き機能します** - 繰り返しキーがなければペイロードのバイト列は変わらず、これはテストで固定されています。未解決のあらゆるパスワードリセットリンクをサイレントに無効化してしまう正規化形式の変更は、この不具合そのものより悪いことになるからです。

  6件のリグレッションテストがあり、両方の攻撃順序、正当に繰り返されても署名・検証できなければならないキー、そして並べ替えの保証をカバーしています。変更*されていない*もの: `signature_has_not_expired`は、依然として偽造された署名を「期限切れではない」と報告します。これはLaravelの挙動であり、ドキュメント上の修正として意図的に据え置かれたもので、善意の「修正」に抗してそれを固定する専用のテストを持っています。
- **Postgres下でのRBAC。**SQLiteだけでなく、実際のPostgresに対して検証されました。
- **4件のRustSecアドバイザリーが、更新ではなく根絶されました。**Pineconeドライバーは、PineconeのREST APIに対して書き直され、`pinecone-sdk 0.1.2`を切り離しました - その最新リリースは2024-09-06付けです - それに伴い、`tonic 0.11 → rustls 0.22 → rustls-webpki 0.102`と、RUSTSEC-2026-0049 / -0098 / -0099 / -0104も切り離されました。この4件はすべて、`rustls-webpki >= 0.103.13`で上流で修正済みであり、このワークスペースは他のTLS利用箇所についてはすでにそちらに解決していました。1つの放棄されたクレートが、ツリーを脆弱な系列に留めていたのです。`.cargo/audit.toml`は、5件のignoreから1件へ減りました。このドライバーのAPIにとってこれが何を意味するかは、**変更**を参照してください。
- **監査の例外に、有効期限が設定されるようになりました。**`.cargo/audit.toml`のすべてのエントリが`OWNER`と`EXPIRES`日付を持つようになり、`scripts/check-audit.sh`は、オーナーの欠落、日付の欠落あるいはパース不能、または期限切れのいずれかがあると、リリースゲートを失敗させます。`cargo audit`には、期限付きignoreという概念がないため、「一時的に」追加されたものが、誰かがそのファイルを読み直すまで残り続けていました。残っているエントリ（RUSTSEC-2023-0071、`rsa`。これはそもそも修正済みリリースがまったくありません）には、オーナーと日付が設定されています。
- **到達可能性の主張は、宣言ではなく検証されます。**`scripts/check-feature-matrix.sh`は、実際の依存関係ツリーを解決し、`cargo audit`が実際に読み取る対象である`--all-features`を含め、どのビルドも`pinecone-sdk`、`rustls-webpki 0.102.x`、`tonic 0.11.x`を含まないことをアサートします。何も検証しないコメントによって正当化された例外は、誰かが依存関係を1つ追加した瞬間に真実でなくなります。

### 修正

- **データベースバックエンドのキューにおける、あらゆるリリースが、サイレントに何もしていませんでした。**`JobOutcome::Released`（ビジーな`WithoutOverlapping`ロック、レートリミッターのバックオフなど）は、「コピーをプッシュしてから、元のものをACKする」という形で実装されていました。エンベロープのidは`jobs`テーブルのプライマリキーであるため、コピーは、現在の予約を保持したままの行と衝突し、プッシュは`UNIQUE constraint failed: jobs.id`で失敗していました。ワーカーはその後、正しくACKを拒みました。そのため、要求された遅延は一度も適用されず、`JobReleased`イベントも発火せず、ジョブはただ、可視性の失効が再配送するまで留め置かれていました。リリースは今では、その場で完結する1回のドライバー呼び出しになりました。
- **部分的なバッチディスパッチが、すでにキューに入れていたジョブを孤児にしていました（DATA-02）。**`driver.push`がループの途中で失敗すると、`PendingBatch::dispatch`はバッチの行を削除していました - しかし、すでにキューに入っていたエンベロープには、そのバッチidが刻印されたままだったため、それぞれが、もはや存在しないバッチに対して決着しようとし、配送のたびに永遠に`Err(batch not found)`を返していました。バッチは今では、代わりに決着させられます: ディスパッチされなかったジョブは失敗として記録され、バッチはキャンセルされます。そのため、キューに入っていたものは正常に決着し、終端コールバックも引き続き発火します。
- **`url::has_valid_signature`が偽造されたURLを拒否することを、何もテストしていませんでした。**SEC-04の修正を検証している最中に見つかりました: 主要な署名付きURLガードを、あらゆる署名を受け入れるよう書き換えても、フレームワークのテストスイート全体が通過してしまったのです。
- **スキャフォルドされたアプリは、データベースをマイグレーションすることも、イメージをビルドすることもできませんでした（REL-01b）。**どちらのスキャフォルドも`default-run`を宣言していなかったため、`cargo run`をシェルアウトする9つのCLIラッパーすべてが、まっさらなプロジェクトで失敗していました。生成されるDockerfileには、5つの独立した不具合がありました - ロックファイルのCOPY漏れ、ロックなしの`npm ci`、宣言済みの2つのバイナリのうち1つだけをスタブするキャッシュステージ、viteが一度も作らないパスからコピーされるフロントエンドビルド、そして`inertia_response!`がコンパイル時に検証する`frontend/src/pages`のコピー漏れです。標準のスキャフォルドのイメージは、ビルドできませんでした。
- **`docker:init`は、あらゆるプロジェクト種別に対して同じDockerfileを出力していました。**`--api`プロジェクトでは、最初の命令である`COPY frontend/package.json`がまるごと失敗していました。APIプロジェクトは今では、フロントエンドを含まないDockerfileを受け取ります。
- **SQLのプレースホルダー（DATA-01）。**単一の方言を前提とするのではなく、バックエンドごとにレンダリングされるようになりました。
- **キューの決着（DATA-02a、P2-06c）。**後続処理は、予約がACKされる前に決着するようになり、ロック解放のエラーが、すでに成功したジョブをリトライに変えてしまうこともなくなりました。
- **キャンセルされたバッチは、`Then`ではなく`Catch`を発火していました。**
- **`Builder::clone`が、eager-loadの計画をサイレントに落としていました（P2-09a）。**`User::query().with("posts")`は、ページネーション、`count()`、クローンを行うあらゆるスコープなど、どこでクローンしても、リレーションを持たない行をエラーなしで返していました。
- **プレゼンスの名簿がメンバーを失っていました（P2-08）。**名簿は購読の前にスナップショットされていたため、そのウィンドウの間に参加した人は、どちらの名簿にも永久に現れませんでした。
- **Pineconeは、すべてのインデックス取得を直列化していました（P2-14）。**書き込みロックは、2回のネットワークラウンドトリップにまたがって保持されており、`tokio`の公平な`RwLock`のせいで、1つの冷えたインデックスが、あらゆる温まったインデックスを止めてしまっていました。
- **型ウォッチャーが、バーストを捨てていました（P2-13）。**リーディングエッジのデバウンスは、バーストの最初のファイルで再生成し、末尾の実行なしに残りを捨てていたため、最後の保存が反映されることは決してありませんでした。
- **`ssr:check`はハングすることがあり、アドレスを1つしか試しませんでした（P2-13）。**DNSは、タイムアウトの外側でまるごと実行されており、解決されたアドレスのうち最初の1つしか試されていませんでした - そのため、AAAAレコードを持ちながらIPv6経路を持たないホストは、v4でリッスンしているにもかかわらず、ワーカーがダウンしていると報告していました。
- **`suprnova serve`は、`cargo-watch`をピン留めせずにインストールしていました（P2-13）。**今ではメジャーバージョンの範囲付きで`--locked`になりました。
- **リリースのバンパーは、5つのREADMEだけを書き換え、他は何もしていませんでした。**4つのマニュアルの章と1つの公開ドキュメントコメントが、どのリリースでも一度も更新されないタグをピン留めしたままになっていました - そのドキュメントコメントは、2リリース分古くなっていました。発見は今では手作業で保守されていたリストに置き換わり、スモークテストは、バンパー自身のverifyステップを信頼するのではなく、更新後のツリーを独立してgrepします。
- **`db:sync`は、データベースのスキーマを信頼できる入力として扱っていました（CLI-01）。**
- **`migrate:fresh`は、`--force`と入力による確認の両方の裏にゲートされるようになりました（CLI-02）**。CLIだけでなく、アプリのバイナリでも同様です。
- **`log`メールドライバーは、Laravelと同じように、メッセージ全体をログに出力するようになりました**。そして本番環境では、bearerリンクをログに書き込まなくなりました。

### 追加

- **アトミックな終端決着（`QueueDriver::settle`、DATA-02）。**チェーンの後続とACKは今では、`DatabaseQueueDriver`上で一緒にコミットされ、その間でのクラッシュがチェーンの残りを失わせたり、次のステップを2回実行させたりするウィンドウを閉じます。予約をキーにした削除は、フェンスとしても機能します: 実行途中で可視性が失効したワーカーは何もコミットせず、`Settled::Stale`を報告するため、別のコンシューマーが今では所有しているメッセージに対して作業をエンキューすることができません。これができないドライバーは`Settled::Unsupported`を返し、ドキュメント化されたプッシュ・ビフォア・ACKの順序を維持します。
- **`DatabaseBatchRepository`（DATA-02）。**バッチの集計は再起動を生き延びます。`pending_jobs`/`failed_jobs`は、保存してデクリメントするのではなく、`(batch_id, job_id)`をキーにした決着行から導出されます - そのため、再配送されたジョブが、他のジョブがまだ実行中であるにもかかわらずバッチを「完了」に押し進めてしまうことはなく、この保護は1プロセスの中だけでなく、プロセスをまたいで保たれます。
- **`/_suprnova/health/live`と`/_suprnova/health/ready`。**liveness（生存確認）は何にも触れません。readiness（準備確認）は依存関係をプローブします。livenessプローブにデータベースチェックを組み込むと、データベースの瞬断が、すべてのレプリカのローリング再起動に変わってしまいます。これは、以前の単一のエンドポイントが誘発していたことです。`/_suprnova/health`は、ドキュメントどおりに引き続き機能します。
- **`SERVER_HEALTH_READINESS_TOKEN`。**readinessプローブ向けのオプションの共有シークレットで、一定時間で比較されます。これがない場合、readinessは404を返します - ルーティングされていないパスと見分けがつきません。それは実際にルーター自身の404そのものだからです。既存のプローブが引き続き機能するよう、デフォルトでは未設定です。
- **`MAIL_SMTP_ENCRYPTION`** - `starttls` | `tls` | `none`で、`ssl`と`null`はLaravel互換のエイリアスとして受け入れられます。未設定の場合は認証情報から導出され、以前の挙動を正確に再現します。これにより、ポート465の暗黙的TLSにも到達できるようになります: トランスポートはそれをサポートしていましたが、どんな環境変数の組み合わせを使ってもそれを選択することはできませんでした。
- **`SERVER_MAX_CONNECTIONS`と`SERVER_HEADER_READ_TIMEOUT`**が、まるごと欠けていた`manual/env-vars.md`にドキュメント化されました。

### 変更

監査自身の結論は、ゲートが470秒で通過し、19件のP1のうちどれも捕まえなかった、というものでした。このリリースのテスト作業の大半は、そこに狙いを定めています。

- **Postgresがゲートの中で実行されるようになりました。**6つのファイルにまたがる12件のテストが、一度も実行されたことがありませんでした。そのうち2件は、デフォルトで`localhost:5432`上にあるどんなPostgresに対してであれ`DROP TABLE`を向けてしまうことが判明し、どちらも`Crypt`を一度も初期化していなかったため、初めて実行されたときにどちらも失敗しました。
- **スキャフォルドのアサーションは、テンプレートのソースではなく、置換後にユーザーが受け取るバイト列を読むようになりました。**データベースを文字どおり`{package_name}`と名付けたドキュメントコメントを出荷しているAPIプロジェクトや、フレームワークが一度も読まない5つのメールキーを謳う`.env.example`を発見しました。
- **キューの障害注入。**ACKの喪失、再配送、リースの失効、部分的なディスパッチは、指定された呼び出しで指定された操作を失敗させるデコレーターによって駆動されるようになり、あらゆるケースが、スリープを使った競合ではなく決定的になりました。
- **支払いアダプターに、ネガティブテストが追加されました。**Stripeの`verify()`は、*有効な*署名で一度も演習されたことがなく、HMAC比較への到達に依存するあらゆる拒否パスが、実証されていませんでした。
- **Pineconeドライバーは、RESTで話すようになりました。***デフォルトでオフの`vector-pinecone`フィーチャーの裏にある、破壊的変更です。*動機は**セキュリティ**の下にあります。表面上の変更は次のとおりです:
  - `client()`はなくなりました - `PineconeClient`はもう存在しません。代わりとなるのは`control_plane_get`、`control_plane_post`、`data_plane_post`で、これらは、ドライバーの認証済みでホスト解決済みのトランスポート上で、あなた自身のリクエスト型とレスポンス型を使って、*あらゆる*Pineconeエンドポイントに到達できます。これは、以前の抜け道が持っていたよりも厳密に広い到達範囲です。
  - `json_to_metadata` → `metadata_from_json`となり、メタデータは今では`prost_types::Struct`ではなく`serde_json::Map`です。`decode_match_fields` → `decode_match`となり、`PineconeMatch`を受け取ります。`namespace()`は`&str`を返します。
  - 新規: `with_control_plane`、`with_api_version`、`with_index_host`（既知のホストを固定し、コントロールプレーンへのラウンドトリップをスキップします）、`index_host`、そして`PineconeVector`/`PineconeMatch`という通信用の型です。
  - `from_env`は引き続き`PINECONE_API_KEY`と`PINECONE_CONTROLLER_HOST`を読み取り、今では`PINECONE_API_VERSION`も読み取ります。
  - REST APIのバージョンは、浮動ではなく固定されています - `2025-04`、つまりドライバーのリクエストとレスポンスの形がそれに対して書かれたバージョンです。
  - もはや何も直列化されません。旧ドライバーは、`pinecone-sdk`が`&mut self`の裏でしかそれを公開していなかったため、名前ごとに1つの`Index`を`tokio::Mutex`の裏にキャッシュしていました。新しいものは、ホスト文字列をキャッシュし、`reqwest`のコネクションプールを共有します。
  - コントロールプレーンから知らされたホストは、レスポンスがどんなスキームを運んでいようと、常に`https`経由で連絡されます。
  - `Debug`は、APIキーを伏せ字にした形で手動実装されているため、ドライバーを保持する構造体に対する`#[derive(Debug)]`は、それを出力できません。
- **Pineconeの通信契約テスト。**実際に稼働している統合テストは`PINECONE_API_KEY`を必要とするため、ゲートの中では実行できません - そのせいで、RESTでの書き直しにおけるフィールド名（`topK`、`includeMetadata`、`vectorCount`）は、何にも支えられないまま残っていました。13件のテストが今では、ローカルの`wiremock`フェイクに対してドライバーを駆動し、それが実際に送信する正確なメソッド、パス、ヘッダー、JSONボディをアサートします。さらに、2xx以外がレスポンスとしてデコードされることは決してないこと、そしてエラーメッセージがAPIキーを運ぶことは決してないこともアサートします。これらは、ドライバーをPineconeの*ドキュメント化された*契約に固定します。ドキュメントが実際のサービスと一致していることを確認できるのは、`#[ignore]`されたテストだけです。

## 0.7.2 - 2026-07-28

### 修正

- **`generate-types`は、deriveを持たないネストしたprop構造体も解決します。**0.7.1のジェネレーターは、`InertiaProps`/`Data`をderiveしていない型を持つあらゆるpropフィールドを`unknown`へ劣化させていました - そのため、コミット済みの型ファイルを持つプロジェクトに対してジェネレーター（あるいは`suprnova serve`のウォッチャー）を再実行すると、`Array<AdminArticleRow>`のような実在のインターフェースが`unknown`に置き換わり、アプリ全体の型チェックが壊れていました。`src/`のどこかで定義されたプレーンな構造体は、今ではpropのルートから推移的に、実在のインターフェースへ解決されます。`unknown`（警告付き）は、プロジェクトが本当に定義していない型 - 外部クレートの型、enum、タプル構造体 - のために予約されています。

### 変更

- **`routes.ts`の生成は、オプトインになりました。**`generate-types`は、聞かれもせずに`frontend/src/types/routes.ts`をあらゆるプロジェクトに落とすことはもうありません。生成するには`--routes`を渡してください。

- **フロントエンドのスターター依存関係が更新されました。**`suprnova new`による新しいスキャフォルドは、今では現行バージョンを固定します: Vite ^8.1.5、Tailwind CSS ^4.3.3、Svelte ^5.56.8（vite-plugin-svelte ^7.2.0、svelte-check ^4.7.4）、React ^19.2.8（plugin-react ^6.0.4）、Vue ^3.5.40（plugin-vue ^6.0.8、vue-tsc ^3.3.8）、そして`@types/node` ^24（Node 24 LTSの型系列）です。TypeScriptは意図的に^6.0.3のままです: これは最新の6.x系であり、svelte-checkのpeer範囲（`^5 || ^6`）は、まだTypeScript 7を受け入れないためです。3つのスターターすべてが、更新後のセットに対してエンドツーエンドで検証されました（`npm install` + `npm run build`）。

## 0.7.1 - 2026-07-27

0.7.0のキュールーティングに対する、不具合修正のパスです。リリース後の全面レビューから生まれました。

### 修正

- **チェーンされたジョブは、もう宣言済みのキューを失いません。**`ChainLink`は、チェーン構築時にジョブの`max_tries`、`timeout`、`backoff`をキャプチャしていましたが、`Job::queue()`はキャプチャしていませんでした。そのため、直接プッシュされれば宣言済みのキューに届くジョブが、チェーンの一部としてディスパッチされると`default`に届いてしまっていました - route → job → defaultという解決順序のうち「job」の階層が、チェーンに対してはサイレントに消えていたのです。宣言済みのキューは今ではリンクにキャプチャされ、直接プッシュとまったく同じように解決されます。このリリースより前に書かれたチェーンのペイロードは、変更なくデコードでき（`serde(default)`）、宣言済みのキューを持たないリンクは、0.7.0が書き出したものとバイト単位で同一に直列化されます。
- **失敗ジョブのレコードは、そのジョブが死んだキューを運ぶようになりました。**ワーカーのデッドレターパスは、あらゆる`FailedJob`レコードに`queue = "default"`をハードコードしていたため、ルーティングされたジョブの失敗は、失敗ストアをそれを所有するプールでフィルタしているオペレーターには見えませんでした。レコードは今では、エンベロープのキュー（ルーティングされていないジョブには`default`）を運びます。
- **0.7.0のアップグレードノートは、`jobs`マイグレーションの重要性を過小に述べていました。**そこには「フィルタしないワーカーは影響を受けず、マイグレーションは不要」と書かれていましたが、`DatabaseQueueDriver::push`は、ジョブがルーティングされているかどうかにかかわらず、`INSERT`の中で`queue`カラムを名指しします - マイグレーションされていないテーブルに対する0.7.0のバイナリは、フィルタの有無を問わず**あらゆるプッシュ**を失敗させます。以下の0.7.0のセクションと`manual/queues.md`は修正されています: データベースドライバー上では、`ALTER TABLE`があらゆるデプロイにおいて必須であり、バイナリがロールする前に実行されなければなりません（古いバイナリは自身のカラムを明示的に列挙し、新しいカラムを無視するため、先にマイグレーションする順序は安全です）。

- **READMEは、もう`#[job]`マクロを謳いません。**そのようなマクロは存在しません - ジョブは`Job`トレイトを実装します。キューの行は今では、0.7.0のキュールーティングを含む、実際の表面を説明します。

### 変更

- **リリースパスは、今ではREADMEのバージョン参照を更新します。**`bump-workspace-version.py`は、READMEのピン留めされたインストールタグ、配布モデルの例、MSRVの行を、マニフェストとアトミックに書き換えます。パターンにマッチしなくなるほど文言が変わったREADMEは、リリースをはっきりと失敗させます。READMEは、v0.7.0が出荷されて以降、リリースパスの中の何もそれに触れていなかったため、v0.6.0を謳い続けていました。
- **コネクションルーティングは、名前解決のみであるとドキュメント化されました。**`Job::connection()`と`Queue::route`のコネクションフィールドは、`JobQueueing`/`JobQueued`のライフサイクルイベントが運ぶコネクション*名*を解決します。単一のプロセスグローバルなドライバーが引き続きすべてのプッシュを受け取るため、それらは別のドライバーを選択するわけではありません。rustdocと`manual/queues.md`は、以前は存在しないドライバー選択をほのめかしていました。キューの次元には影響がありません - こちらはエンドツーエンドで尊重されます。コネクションごとのドライバーは、今後の課題のままです。
- `ChainLink`に公開の`queue: Option<String>`フィールドが追加され、これはチェーンリンクの構造体リテラル構築を壊します。`ChainLink::from_job`（通常の経路）を通じて構築されるリンクへの影響はありません。

### アップグレード

データベースキュードライバー上で0.6.x以下から移行する場合は、バイナリをロールする**前**に、以下の0.7.0のマイグレーションを適用してください。これは、そのドライバー上のあらゆるデプロイに必要であり、`--queue`を使うものだけではありません。0.7.1自体にはマイグレーションは必要ありません。

## 0.7.0 - 2026-07-26

### セキュリティ

- **`ammonia`を4.1.4へアップグレード（RUSTSEC-2026-0213）。**4.1.3までのバージョンは、SVGの`animate`と`set`アニメーションタグを介したXSSを許してしまいます。`ammonia`は、Suprnovaのmarkdownパイプライン（`comrak` → `syntect` → `ammonia`）の末端にあるサニタイザーであるため、`content`を通じてユーザー入力のMarkdownをレンダリングするあらゆるアプリが、影響にさらされていました。このアドバイザリーは2026-07-21に公開されました - v0.6.5の出荷後です - そのため、**v0.6.5までのすべてのリリースが影響を受けます**。フレームワークをアップグレードすることが修正であり、アプリケーションコードの変更は必要ありません。

### 追加

- **キュールーティング。**ジョブは特定のキューとコネクションへディスパッチでき、ワーカーは特定のキューに専念させられます - Laravel 13の`Queue::route(...)`の表面を、型付きにしたものです。ジョブは`Job::queue()`/`Job::connection()`で自分自身の居場所を宣言します。オペレーターは、ジョブを編集することなく、`bootstrap::register()`の中の`Queue::route::<SendInvoice>(Some("redis"), Some("billing"))`で、それを一元的にオーバーライドできます。解決順序はroute、job、グローバルデフォルトの順で、routeの中の`None`フィールドは、クリアではなく先送りを意味します。`queue:work --queue=billing,default`は、それらのキューだけをドレインします。ルーティングされていないジョブは`default`に属するため、決して取り残されません。チェーンされたジョブは、チェーンのリンクが自身のジョブを型消去して保存するため、ルートを名前で解決します。
- **`QueueDriver::pop_from`。**フィルタリングされたpopで、尊重できないフィルタに対しては、あらゆるキューをサイレントにドレインするのではなく**拒否する**デフォルト実装を持ちます - `billing`をドレインするよう指示されたワーカーが、静かにすべてをドレインしてしまうと、間違ったプールが間違ったジョブを食い尽くすまで、正常に動いているデプロイと見分けがつきません。メモリドライバーとデータベースドライバーは、ネイティブにフィルタします。カスタムドライバーは、コンパイルが通り続け、このはっきりしたデフォルトを継承します。
- **`jobs`テーブルのスキーマをドキュメント化しました。**`manual/queues.md`は今では、`DatabaseQueueDriver`が実際に期待するDDLを掲載しています。これは、以前はドライバーのSQLを読むことでしか発見できませんでした。
- **Inertiaの`serverHead`オプションをドキュメント化しました。**サーバー主導の`<head>`要素（Inertia 3.5.0）は、フレームワークのサポートを一切必要としません: クライアントはそれらを普通のpropから読み取るため、どのハンドラもすでにそれらを提供できます。`manual/frontend-inertia-responses.md`を参照してください。

### 変更

- `Envelope`に`queue: Option<String>`フィールドが追加されました。これは`serde(default)`であり、存在しない場合はスキップされるため、ルーティングされていないエンベロープは、以前のバージョンが書き出したものとバイト単位で同一に直列化されます - 通信上の形式を凍結したテストは変更なく通過し、`schema_version`のバンプもなく、ローリングアップグレードの最中もバージョンの混在したフリートが相互運用できます。
- `WorkerConfig`に`queues: Vec<String>`フィールドが追加されました（空の場合はすべてをドレインする、以前の挙動のままです）。
- `ROADMAP.md`を削除しました。その設計原則は`manual/introduction.md`に、作業の取り決めは`manual/contributions.md`に、デプロイとスケールアウトの資料は`manual/deployment.md`に、それぞれ住んでいます。出荷済み/計画中のチェックリストは、古くなっていました。`README.md`が「上流との関係」のために指し示していたリンクは、すでに宙に浮いていました - その帰属表示は`LICENSE`に住んでいます。
- スキャフォルドのフロントエンドは今では、`@inertiajs/{svelte,react,vue3}`を（`^3.4.0`から）`^3.6.1`に固定します。3.4.0 → 3.6.1の範囲はクライアントサイドのみです - 上流のchangelogと`packages/core/src/types.ts`の`Page`契約に照らして監査したところ、3.6.1のクライアントが送るあらゆる`X-Inertia-*`ヘッダーは、すでに処理済みでした。
- `scripts/release.sh`は今では、そのバージョンの`CHANGELOG.md`セクションから取られたノートを添えて、GitHubリリース自体を公開します。以前はこれが、スキップされがちな手作業の「次のステップ」だったため、v0.5.10とv0.6.1–v0.6.3はタグのみで、Releasesページは古いバージョンのまま止まっていました。プリフライトはゲートの前に実行されるため、`gh`やchangelogセクションの欠落は数秒で失敗し、`origin`がGitHubでない限り、公開は自動的にスキップされます。

### アップグレード

データベースキュードライバー上の既存の`jobs`テーブルは、新しいカラムを追加**しなければなりません** - `push`は、ジョブがルーティングされているかどうかにかかわらず、`INSERT`の中でそれを名指しするため、マイグレーションされていないテーブルはあらゆるプッシュを失敗させます。先にマイグレーションしてから、バイナリをロールしてください（古いバイナリは自身のカラムを明示的に列挙し、新しいカラムを無視するため、その順序は安全です）:

```sql
ALTER TABLE jobs ADD COLUMN queue TEXT NULL;
CREATE INDEX idx_jobs_queue ON jobs(queue);
```

*（0.7.1で修正 - このノートは元々、フィルタしないデプロイにはマイグレーションが不要だと主張していました。）*

## 0.6.5 - 2026-07-21

### 追加

- **Stripeアダプターにおける、ホスト型のワンオフCheckout。**`SessionMode::OneOff`と空でない`price_refs`を伴う`Checkout::start_session`は、今ではホスト型のCheckout Session（`mode=payment`、price refごとに1つの明細項目、`allow_promotion_codes=true`）を作成し、`SessionPayload::StripeCheckoutRedirect`を返します。`amount_hint`のみのElementsの経路は変わりません。2つの形は、リクエストごとに選ばれます。
- **Stripe Managed Payments（Merchant of Record）のサポート。**`StripeProvider::with_managed_payments(true)` - あるいは`from_env()`での`STRIPE_MANAGED_PAYMENTS=true` - は、ホスト型のワンオフセッション作成時に`managed_payments[enabled]=true`を送信します。デフォルトではオフです。このフィールドはまるごと省略されるため、登録していないアカウントへの影響はありません。
- **`Checkout::session_status`。**新しいトレイトメソッド（デフォルト: `PaymentError::NotSupported`）で、セッションのプロバイダー側の状態を、新しい中立な`CheckoutSessionState`（`Open`/`Complete { paid, payment_ref, amount_total }`/`Expired`）として報告します。Stripeの実装は`GET /v1/checkout/sessions/{id}`をマッピングします。`payment_ref`は、ミラーテーブルとの突き合わせのために、セッションのPaymentIntent idを運びます。これは、リダイレクト復帰ページと消込スイープのための、サーバーサイド検証のプリミティブです。
- **`Promotions`ケイパビリティトレイト。**`create_promotion_code`は、事前作成されたクーポンから、顧客限定で、任意に期限を持ち、償還回数の上限が付いたコードを発行します。新しい`PaymentProvider::as_promotions()`（デフォルトは`None`）経由でクエリされます。Stripe（`POST /v1/promotion_codes`）とモックに実装されています。
- **上記に対応するための、`MockPaymentProvider`の拡張。**あらゆる`start_session`リクエストを記録し（`recorded_sessions()`）、セッションidごとに`session_status`をスクリプト化し（`script_session_status()` - スクリプト化されていない既知のセッションは`Open`を、未知のidは`NotFound`を報告します）、記録済みリクエストを伴う`Promotions`を実装します（`recorded_promotion_requests()`）。

## 0.6.4 - 2026-07-17

### 修正

- **Eloquentの集計は、データベースバックエンドをまたいで一貫してデコードされます。**生成される`count`、`sum`、`avg`、`min`、`max`の式は、今では1つの安定した内部結果エイリアスを使います。PostgreSQLは、そのドライバーが集計カラムをSQLiteとは異なる形でラベル付けすることによる偽のゼロや`None`をもう返しません。カラムの欠落や型の非互換によるエラーは、今ではサイレントにデフォルト値化されるのではなく、伝播するようになりました。
- **一括削除は、呼び出し元が指定するテーブル式を使えません。**実行可能な削除SQLは、常にモデルの検証済み静的な`M::TABLE`からターゲットを導出します。従来の公開レンダラー引数はソース互換のまま残りますが、削除ターゲットをリダイレクトしたり注入したりすることはできません。

## 0.6.3 - 2026-07-15

### 追加

- **型付きの生の読み取りが、トランザクションに固定されたコネクション上に留まれるようになりました。**`Transaction::backend()`はアクティブなバックエンドを公開し、`Transaction::query_all(Statement)`は、`QueryExecuted`の計装を保ったまま、型付きの集計やカスタムSQLをトランザクションを通じて実行します。ロックスコープの判定が計算済みの結果カラムに依存する場合でも、アプリケーションはもう、プールレベルのクエリや非公開のエグゼキューターへのアクセスを必要としません。

## 0.6.2 - 2026-07-15

### 修正

- **バインドされた生の述語は、バックエンドに中立です。**Eloquentの`filter_raw`と`where_raw`は、今ではあらゆるデータベースバックエンドで、可搬な`?`バインドマーカーを受け付けます。PostgreSQLでのレンダリングは、先行する述語、リレーションシップのサブクエリ、HAVING句、UNIONの各項をまたいで、それらを単調増加する`$N`の位置へ振り直します。既存の番号付きPostgreSQLフラグメントは、そのローカルなマーカー順によって正規化される一方、スタイルの混在やバインド数の不一致は、I/Oの前にバリデーションで失敗します。SQLを意識したスキャナーは、クォートされた文字列、識別子、コメント、ドル記号でクォートされた本体の内側にある疑問符を保持します。`??`は、バインドされた生のフラグメントの中で、リテラルな疑問符演算子を出力します。

## 0.6.1 - 2026-07-15

### 追加

- **監督下にある、観測可能なセッションクリーンアップ。**`SessionMiddleware::install`は、設定可能な`SESSION_GC_INTERVAL`の周期（デフォルトは1時間）を使い、`session_gc_metrics()`は、保護された運用系サーフェス向けに、プロセスローカルな実行回数、成功、失敗、削除された行数、最終結果のタイムスタンプを公開します。
- **上限付きのスライディングセッションのタッチ。**`SESSION_TOUCH_INTERVAL`は、アクティビティ書き込みの最小周期（デフォルトは5分）を制御し、アクティブなセッションがタッチの間に失効しないよう、セッション寿命の半分を上限としてクランプされます。

### 修正

- **状態を持たないリクエストは、もう永続的なセッションを作成しません。**有効なセッションクッキーを持たないリクエストは、セッションストアの読み書きを一切行わず、ハンドリングが状態を作らない限りセッションクッキーも受け取りません。既存のクリーンなセッションは、無条件のupsertとクッキーの入れ替わりを避け、旧来のクッキーは次のリクエストで移行し、裏側の行が失効しているクッキーは、空のセッションを再作成することなくクリアされます。

## 0.6.0 - 2026-07-10

### 追加

- **後方互換のデフォルトを保ったまま、オプトインになったフレームワークのサブシステム。**ファイルシステムストレージ、SQLite/Postgres/MySQLのデータベースドライバー、MariaDBのベクトルドライバー、そしてWeb Pushは、今では明示的なCargoフィーチャーを持ちます。既存のデフォルトビルドはこれらすべての機能を保持し続け、`default-features = false`の利用者は、ドライバーをゼロ個選ぶことも、使用するストレージ/データベース/ベクトル/プッシュの表面だけを選ぶこともできます。実行可能なフィーチャーマトリクスは、ドライバーゼロ、個別ドライバー、Nation X最小構成、デフォルト、全フィーチャーの各プロファイルを検証します。
- **生のP-256 VAPID秘密鍵インポート。**`VapidKey::from_bytes`は、既存のPKCS#8 PEMインポート/エクスポート経路に加えて、検証済みの32バイトビッグエンディアンP-256スカラーを受け付けます。

### 変更

- **VAPIDのJWTは、P-256で直接署名されるようになりました。**Web Pushは今では、RFC 8292のES256ヘッダー/クレームを直列化し、`p256`で署名します。生成される鍵、PEMのラウンドトリップ、公開鍵エンコーディング、24時間の寿命上限は保ったまま、汎用のJWT依存を取り除きました。
- **セキュリティ関連の依存関係の更新。**bcryptやammoniaを含む、脆弱なフレームワークの依存関係を更新し、シンタックスハイライトは維持したまま、Comrakの有効フィーチャーを絞り込みました。
- **Rust 1.91.1が、このリリースのMSRVです。**ワークスペースのすべてのパッケージが同じ`rust-version`を宣言し、生成されるDockerfileは対応するビルダーイメージを固定し、フルのリリースゲートは、まさにRust 1.91.1のツールチェーンでサポート対象のファイルシステムプロファイルをコンパイルします。
- **OpenDAL 0.58のセキュリティピン留め。**ファイルシステムフィーチャーは、公式のApache OpenDALコミット`ae99a3b016e354a1b2bb2baf0c70f9f9e134970a`にちょうど基づく最小限のフォークである、`eas4ai/opendal`のコミット`88717391eb72c9839d3f8e79fccad9f22fc3a1b4`をピン留めします。このフォークは、下流の利用者が公式のApache Reqsignコミット`b49cd2996b9d2d9944e84481f8835ff55b188b97`と`quick-xml` 0.41.0を解決できるよう、OpenDALコアとS3、GCS、Azure Blobが使うReqsignの宣言だけを変更しています。依存リポジトリのルートのCargoパッチは利用者へ伝播しないため、フォークが必要です - そうしなければ、公開される依存グラフが、脆弱な`quick-xml` 0.38/0.40を復活させてしまいかねません。

### 修正

- **アトミックなリリースバージョンメタデータ。**リリースのバンプは今では、`workspace.package.version`とバージョン付けされたすべての内部パス依存を、1つの検証済み操作で更新し、影響を受けるすべてのマニフェストをステージし、リリース前に`cargo check --workspace`で一時的な`0.6.0`ワークスペースを検証します。リリースバージョンは、プレリリースの数値に先頭ゼロを許さないルールを含め、厳密なSemVer 2.0として検証されます。バージョンに依存しない使い捨てのベアリモートスモークテストは、現在のソースとすでに`0.6.0`であるソースの両方から、後続のパッチリリースを導出し、ゲートの前にステージ済み/未ステージ/未追跡のリリースツリーを拒否し、タグが拒否されたときにコミット/タグの公開がアトミックに両方のrefをロールバックすることを証明し、実際のリモートに触れることなく通常のリリース手順を証明します。リリースバージョンは、プレリリースの遷移を含め、SemVerの優先順位に従って増加しなければなりません。スモークビルドの成果物は、呼び出し元の`CARGO_TARGET_DIR`が何であれ無視して、常に一時的なワークスペースの内側に留まります。
- **Rustdocが、サポート対象のあらゆるフィーチャー境界をカバーします。**OAuthモジュールは、公開の`OAuthAuth::complete`にリンクし、実行可能なマトリクスは、依存関係なしで、ドライバーゼロ、デフォルト、全フィーチャーのrustdocをビルドします。
- **ファイルシステムのストリームバリデーションが、セッションスコープになりました。**ローカルファイルシステムのライター、リスター、コピアーは、チャンクや項目ごとに1回ではなく、最初のI/Oの前に一度だけパスを解決し閉じ込めます。一方、アクティベートされたクローズ/アボート操作は、クリーンアップのために常にバックエンドへ到達します。既存のトラバーサルとシンボリックリンクの閉じ込めは、信頼できるファイルシステムに対しては引き続き強制されます。canonicalize-then-openのチェックは、ツリーを並行して変更するプリンシパルに対する競合を排除しません。

### セキュリティ

- **リリースゲートは、フェイルクローズします。**`release.sh`は、マニフェストを編集したりコミット/タグを作成したりする前に、正規のフルゲートへ委譲します。そのゲートは常に`cargo audit`を実行し、`cargo-audit`バイナリの欠落をエラーとして扱い、監査の失敗があれば必ず停止します。また、隔離された下流のファイルシステム利用者をビルドし監査して、正確なOpenDAL/Reqsignのソースリビジョンと、0.41未満の`quick-xml`が存在しないことをアサートします。新しいアドバイザリーのignoreは追加されていません。

## 0.5.10 - 2026-07-03

### 修正

- **`generate-types`は、もう自己参照する構造体を落としません。**自分自身の型を参照するフィールドを持つ構造体（`children: Vec<Self>`を持つツリーノード、たとえばスレッド型コメントのビューなど）は、型の依存グラフに自己エッジを作り、その入次数をゼロより上に固定していたため、Kahnのトポロジカルソートはそれを一度も出力しませんでした - それを参照するあらゆるインターフェースに、`svelte-check`/`tsc`を失敗させる宙に浮いた型名を残していたのです。自己エッジは今ではソートの前に取り除かれ、参照の循環（相互再帰）に捕らわれた構造体も、TSのインターフェースは宣言順に関係なく互いを参照できるため、落とされるのではなく任意の順序で出力されます。

## 0.5.9 - 2026-07-01

### 追加

- **`MAIL_FROM_NAME` - 認証フローのメールにおける、オプションの表示名。**メール確認、パスワードリセット、パスワード変更の各メーラブルは、`MAIL_FROM_NAME`が設定されている場合、今では`From`ヘッダーを`"Name <address>"`としてレンダリングします（送信時に読み取られるため、キューのserdeラウンドトリップを生き延びます）。`MAIL_FROM`は素のアドレスのままです。`MAIL_FROM_NAME`を未設定または空のままにしておくと、以前の素アドレスの挙動が保たれます。呼び出し箇所への変更はありません - メーラブル自身が環境変数を読み取ります。

## 0.5.8 - 2026-06-30

### 修正

- **`generate-types`のルートヘルパーは、常に有効なTypeScriptです。**あるモジュール内の複数のルートが1つのハンドラを共有する場合（たとえば、多数のfavicon/アセットURLをマッピングする`static_files::serve`の許可リストなど）、最初のものはハンドラ名を保持し、残りはルートパスから導出されたキーを得ていました - しかし、そのパスは部分的にしかサニタイズされておらず（`/ { } -` → `_`）、ファイル拡張子がキーに`.`を漏らしてしまっていました: `favicon_16x16.png: (...) => ...`。これはプロパティ名ではなくメンバーアクセスであるため、`tsc`/`svelte-check`は生成された`routes.ts`を拒否していました。導出されたキーは今では正当な識別子へサニタイズされます - 英数字以外の文字はすべて`_`になり、先頭が数字の場合は接頭辞が付きます - そのため`favicon-16x16.png` → `favicon_16x16_png`、`2fa.json` → `_2fa_json`となります。一意なハンドラ名には変更がありません。

## 0.5.7 - 2026-06-30

### 修正

- **`generate-types`は、もう宙に浮いた型参照を出力しません。**`InertiaProps`/`Data`をderiveしていない構造体（あるいはジェネレーターから見えない外部の型）を型に持つpropフィールドは、素の識別子として出力されていました - たとえば`user: UserInfo`のように - そのインターフェースが決して書き出されないため、`tsc`/`svelte-check`を失敗させるTypeScriptを生んでいました。そのような参照は今では`unknown`へ劣化するため（`user: unknown`、`Vec<T>` → `Array<unknown>`、`Option<T>` → `unknown | null`）、生成される出力は常に型チェックを通り、`generate-types`は、未解決の型とそれを参照しているフィールドの名前を挙げ、修正方法（`InertiaProps`/`Data`をそれにderiveすること）を添えた警告を出力します。ジェネリックパラメータと、解決済みのネストしたInertiaProps/Dataの型には影響がありません。

## 0.5.6 - 2026-06-29

### 変更

- **Sign in with Apple: RS256 JWKS検証。**`suprnova-apple-rs`をv0.3.1にバンプしました - AppleのIDトークンは今では、構造的に信頼されるのではなく、Appleが公開しているJWKS（RS256）に照らして検証されます。

## 0.5.5 - 2026-06-28

### 追加

- **`MagicLink`トークンパーパス。**パスワードレスのマジックリンクサインイントークン向けに、認証フローの`TokenPurpose`enumへ新しい`MagicLink`バリアントを追加しました。

## 0.5.4 - 2026-06-28

### 変更

- **組み立て可能なOAuth完了処理。**汎用のOAuth完了処理を、`verify_oauth_identity`（検証してアイデンティティを解決する）と、薄い`complete`に分割しました。これにより、アプリは、セッション完了の副作用をフルに引き起こすことなく、OAuthのアイデンティティを検証できます。

## 0.5.3 - 2026-06-28

### 修正

- **ワークスペースのバージョンメタデータを修正。**v0.5.2は、その`Cargo.toml`のバージョンバンプがステージされる前にタグ付けされプッシュされてしまったため、プッシュされたv0.5.2タグは依然として`version = "0.5.1"`のままでした。v0.5.3は、正しいワークスペースバージョンでリリースを切り直します - コードの変更はありません（v0.5.2のOAuth分割への影響はありません）。

## 0.5.2 - 2026-06-28

### 変更

- **組み立て可能なApple完了処理。**Apple Sign-Inの完了処理を、汎用のOAuth分割を反映する形で、`verify_apple_identity` + 薄い`complete_apple`に分割しました。（注: プッシュされたv0.5.2タグは、古い`0.5.1`のバージョンフィールドを抱えています - v0.5.3で修正されました。）

## 0.5.1 - 2026-06-28

### 変更

- **Appleクレートをリネーム。**Apple依存関係を、リネームされた`suprnova-apple-rs`リポジトリへ向け直しました。

## 0.5.0 - 2026-06-28

### 追加

- **Sign in with Apple。**AppleのためのOAuthトークン交換 + IDトークン検証 + ユーザーupsert。Appleのwell-knownエンドポイントと`form_post`レスポンスモード。`OAuthProviderConfig`上のApple固有のフィールド。アプリが`apple`への直接依存なしにApple Sign-Inを設定できるよう、`AppleKeyPair`が再エクスポートされました。

### 修正

- Appleの認可URLからPKCEパラメータを省くようにしました（存在するとAppleがリクエストを拒否するため）。

### 依存関係

- `torii`のマジック認証修正を取り込み、`apple-rs` v0.3.0を追加しました。

## 0.4.1 - 2026-06-26

### パフォーマンス

- リクエストごとの`Vec`再割り当てをなくすため、`MiddlewareChain`のサイズを事前に確保するようにしました。

### 修正

- 並行テスト実行下でも衝突しないよう、メンテナンスのdownファイルのパスを堅牢にしました。

### 文書

- フレームワークのドキュメント例をコンパイルチェックするようにし（`ignore` → `no_run`）、配布に関するノートをタグ付けされたGitHub Releasesと整合させ、`docs/`ツリー全体を無視するようにしました。

## 0.4.0 - 2026-06-22

### 変更

- **配布はgitで追跡されます。タグにピン留めする必要はありません。**スキャフォルドされたアプリは`suprnova = { git = "…/suprnova.git" }`に依存し、デフォルトブランチを追跡します。更新は`cargo update -p suprnova`で取得してください。バージョンはchangelogのために、タグ付けされたGitHub Releases（`v0.4.0`など）として公開されますが、`Cargo.lock`はすでに正確に解決されたコミットをピン留めしています - そのため、`tag`や`rev`を手作業でピン留めしなくても、ビルドは再現可能なままです。インストールのドキュメントは、もうコミットのピン留めを更新方法として提示しません。

## 0.3.0 - 2026-06-21

### 追加

- **Eloquentの読み取りに対するクエリ計装** - `Builder::get`、`Model::find`、`find_many`、`all`は、今では`QueryExecuted`を発火するため、モデルのSELECTとeager-loadのクエリが、書き込みや生クエリと並んで`DB::listen`とインメモリのクエリログに現れます。計装された`ExecutorChoice::statement_all`という読み取り終端を追加します。
- **リソースルートの認可** - `ResourceRoutes::authorize_resource::<U, R>()`は、慣例的な権限チェックを、生成されるすべてのリソースルートへルートごとのミドルウェアとして取り付けます（Laravelの`authorizeResource`互換）。アクション→権限のマッピングは、`index`/`show` → `view`、`create`/`store` → `create`、`edit`/`update` → `update`、`destroy` → `delete`です。あらゆるコントローラー本体が`Gate::authorize`を覚えていることに頼るのではなく、1回の呼び出しで7つのアクション全体の表面をゲートします。
- **アトミックなレート制限ヒット** - `RateLimiter::hit_and_check(key, max, decay)`は、固定ウィンドウを1回のラウンドトリップでインクリメントし検査して、バケットが今その上限を超えているかどうかを返します（`i64::MAX`は無制限を意味します）。
- **一定時間比較ヘルパー** - Webhook署名検証のための`constant_time_eq(a, b)`（subtleクレートに支えられています）です。`WebhookHandler::verify`のドキュメントは今では、一定時間でのダイジェスト比較を義務付けています。
- **Inertiaクライアントを3.4.0へ** - Svelte/React/Vueのスキャフォルドは今では、`@inertiajs/{svelte,react,vue3}`を（`3.1.1`から）`^3.4.0`に固定し、`router.poll`モード、動的な`usePoll`、`Inertia.once`、InfiniteScrollのキャンセル修正、awaitされるFormの`onSuccess`を取り込みます。サーバーはすでに、3.4.0のページオブジェクトとヘッダーの表面全体（once-props、prepend/deep-mergeのスクロール系列、`matchPropsOn`、rescued/sharedのprops）を発しているため、これはプロトコルの変更を伴わないクライアントの追随バンプです。
- **オプションのコネクション上限** - `SERVER_MAX_CONNECTIONS`（およびプログラム的な`Server::max_connections(n)`）は、acceptループにセマフォを設けて、同時にアクティブなコネクションを制限し、TCPレベルでバックプレッシャーをかけます。未設定 - あるいは`0` - の場合はコネクションを無制限のままにします（デフォルトで、変更なしです）。リバースプロキシや`LimitNOFILE`と組み合わせるための最後の砦であり、上流のレート制限の代替ではありません。
- **リダイレクト追従のオプトアウト** - `RequestBuilder::no_redirects()`は、リクエストを追従しないHTTPクライアントへ通し、`3xx`を追いかけるのではなくそのまま返します。リクエストURLが信頼できない入力の影響を受ける場合に使い、リダイレクトを介したSSRFのベクトル（悪意のあるエンドポイントが内部やクラウドメタデータのホストへリダイレクトすること）を塞いでください。デフォルトのクライアントは、一般的なクライアントの慣例に従って、引き続きリダイレクトに追従します。

### セキュリティ

- **リソースルート**は、認可レジストリの型消去ダウンキャストに対してパニックする代わりにフェイルクローズするようになり、`authorize_resource`の拒否/未認証のリクエストは、ハンドラが実行される前に拒否されます。
- **レートリミッター**は、アトミックにインクリメントして比較すること（`hit_and_check`）によって、固定ウィンドウのcheck-then-hit競合を塞ぎます。
- **キューの`RateLimited`ミドルウェア**は、今では、別個の`too_many_attempts` + `hit`のペアではなく、そのアトミックな`hit_and_check`を通じてジョブを通すため、並行するワーカーがどれもインクリメントする前に全員が予算チェックを通過して`max_attempts`を超えて過剰に通してしまうことはもうありません。
- **アップロードバリデーター**（`mimetypes`/`mime`）は、クライアントが提供する`Content-Type`を信頼するのではなく、アップロードされたバイト列をコンテンツスニッフィングします。
- **ファイルシステムのパスガード**は、以前の字句的な`../`/絶対パス/UNCチェックに加えて、パスを正規化することで、ストレージルートの外へのシンボリックリンクトラバーサルを捕捉します。
- **認証**は、パスワードレスログインのタイミングオラクルを塞ぎます - マッチしたもののパスワードを持たないアカウントにパスワードが与えられた場合、Eloquentとデータベースのどちらのユーザープロバイダーでも固定コストの検証を実行するようになりました - そして`dummy_verify`は設定済みのハッシャーを駆動するため、マッチしないユーザーの経路は一定時間になります。
- **Eloquent**は、`pluck`/`value`/`pluck_keyed`/`sole_value`と`sum`/`avg`/`min`/`max`の射影経路で、カラム識別子を検証します。
- **支払い** - モックプロバイダーのベリファイアは、開発環境の外ではフェイルクローズし、Webhookの送信元IPは、生の`X-Forwarded-For`ヘッダーではなく`TrustedProxiesConfig`（`req.ip()`）を通じて解決されるようになりました。
- **ファイルシステムのパスガード**は、書き込みターゲットがまだ存在しない場合、今では最も近い*実在する*祖先まで辿るようになり、直近の親が欠けた中間シンボリックリンクを仕込むことでガードをすり抜けるシンボリックリンクエスケープを塞ぎます。
- **`DB::init_with`**は、接続する前に環境を検証するようになり（`DB::init`と一致します）、そのエントリーポイントを通じて開発用のSQLiteフォールバックが本番環境でサイレントに起動することはもうありません。
- **静的ファイル配信**は、`.`/`..`トラバーサルだけでなく、ドットファイル（`.env`、`.git/config`、`.htpasswd`、先頭が`.`のあらゆるセグメント）も拒否します。
- **支払いのWebhook**は、同じ未処理イベントの並行リトライを、`FOR UPDATE`ロック + 再チェックで直列化し、ミラーテーブルの一意制約違反を、無害な適用済みとして扱います。`payments_subscription_items`に`UNIQUE(subscription_id, provider_item_id)`が追加されました。
- **RBAC**は、モデルの判別子を完全修飾型名にデフォルトするようになり、末端の名前を共有する2つの認証可能な型が、互いのロール/権限を継承してしまうことはもうありません。
- **`invalidate_session()`**は、（単にフラッシュするだけでなく）セッションidをローテートし、セッション固定の隙間を塞ぎます。キューの`WithoutOverlapping`ミドルウェアは、ジョブがパニックしたときでもキャッシュロックを解放します。
- **メールプロバイダー**は、web-pushクライアントと同様に、エラーレスポンスのボディ読み取りに上限（8 KiB）を課すため、悪意のあるエンドポイントが送信側のメモリを圧迫することはできません。
- **Web push**は、デフォルトのクライアントでHTTPリダイレクト追従を無効化するため、攻撃者の影響を受けたpushエンドポイントが、通知POSTを内部やクラウドメタデータのホストへ`3xx`でリダイレクトすること（SSRF）はもうできません。リダイレクトは今では、サイレントに追従されるリクエストではなく、拒否されたpushとして表面化します。
- **Stripeアダプター**の`Debug`は、Webhook署名シークレットを伏せ字にし、*さらに*（認証ヘッダーにAPIシークレットキーを運ぶ）`stripe::Client`にはプレースホルダーを出力するため、上流クライアント自身の`Debug`がどうであれ、どちらのシークレットも`StripeProvider`の`{:?}`を通じてログに届くことはありません。
- **Stripeアダプター**の`from_env`は、存在はするが空である認証情報を拒否するようになり、空の（つまり偽造可能な）Webhook HMACシークレットを持つクライアントを構築するのではなく、フェイルクローズします。
- **OAuthのメール検証**は、未知のプロバイダーに対してフェイルクローズします: `email`は運ぶが`email_verified`フラグを運ばないuserinfoペイロードは、もう検証済みとして扱われません。未知のプロバイダーは、今では`email_verified: true`をアサートするか、検証済みメールのエンドポイントを公開しなければならず、これは、アカウントをメールでキーにするアプリに対する、アカウント連携/乗っ取りのベクトルを塞ぎます。Google（明示的な`true`のみ）とGitHub（`/user`契約による検証）には変更がありません。

### 修正

- **ネストしたeager loading**（`with(["posts.comments"])`）は、今では定数個のクエリになります - 末尾のセグメントは、親ごとに1クエリ（N+1）ではなく、すべての親をまたぐ1つのバッチ化されたINクエリでロードされます。
- **`where_has`/`where_doesnt_have`**は、クロージャのカラムをターゲットテーブルで修飾するようになり、pivotとターゲットの両方に存在するカラムが、多対多のリレーションでambiguous-columnエラーを生むことはもうありません。
- **ソフトデリートの`delete`/`force_delete`/`touch`とファクトリーの`persist`**は、プライマリプールへフォールバックするのではなく、モデルの`#[model(connection = "…")]`ルーティングを尊重するようになりました（`restore`や他の書き込み経路と一致します）。
- **JSON:APIの`Maybe::Missing`**は、衝突しない番兵値を通信に使うようになったため、`{"__missing__": true}`という形をしたユーザーデータが、サイレントに取り除かれることはもうありません。
- **キューに入れられた通知**は、ワーカー上で再チェックされる`should_send`（チャネルごとの拒否権）と`after_sending`を尊重するようになりました - 以前は同期経路だけがそうしていました。
- **リリースされたジョブ**は、元のものをACKする前にリトライ用のコピーをプッシュするようになり、一時的なドライバーのプッシュエラーがジョブを失わせることはもうありません。
- **Paddleのadjustment（返金）Webhook**は、adjustment idの下にゼロ金額の行を挿入するのではなく、参照されているトランザクションidにミラーの更新をキーづけし、金額は`data.totals`から読み取るようになりました。
- **クエリ文字列を伴うSQLite URL**（`sqlite://db.sqlite?mode=rwc`）は、有効な単一クエリのコネクションURLと、クリーンなディスク上のファイル名を構築するようになりました。
- **HTTP**は`Accept`の`q`値を`[0,1]`にクランプし、ボディが事前にバッファされていた場合でも`FormRequest`の`max_body_bytes`を強制するようになりました。**WebSocket**の設定は、`max_missed_pings < 2`を拒否するようになりました（1では、最初のpingであらゆるコネクションが閉じられていました）。
- **Cron**は、日と曜日の両方が制限されている場合にOR意味論を使うようになりました（Vixie/POSIX互換）。Markdownの`plain_text`/抜粋は、意図的なスペース入り句読点を保持します。`CachedEvaluator`はキャッシュの増加に上限を設けます。`SupervisorRegistry::start_all`は、2回目の呼び出しで二重にspawnすることはもうありません。テストコンテナは、ポイズニングされたロックからその場で回復します。
- **スーパーバイザーの再起動バックオフ**は、少なくとも60秒の上限だけ稼働し続けた実行の後、100msの下限にリセットされるようになりました。そのため、長い期間健全に動いていたデーモンが終了した場合、以前の失敗の連発の間に積み上がったバックオフを引き継ぐのではなく、速やかに再起動します。実行が一度もその閾値に達しないクラッシュループは、依然として上限まで上り詰めるため、このリセットが不安定なスーパーバイザーを覆い隠してしまうことはありません。
- `filter_op`（演算子は許可リストで検証されます）、署名付きURL（Laravelのデフォルトの絶対署名とバイト互換ではありません）、`UniqueIdKind::is_valid`（呼び出し元向けのヘルパーであり、`find`に自動配線されているわけではありません）、識別子の長さ上限（64ではなく128）に関する、古くなったドキュメントを修正しました。

### ドキュメント

- リソースルートの認可（`authorize_resource`）をルーティングと認可の章にドキュメント化し、アトミックな`hit_and_check`カウンターをレート制限の章にドキュメント化しました。

## 0.2.0 - 2026-06-21

ロールベースのアクセス制御、Markdownコンテンツ/ドキュメントレンダリングパイプライン、そしてネイティブな静的ファイル配信を追加します。

### 追加

- **Tier-2 RBAC** - `HasRoles`トレイト。`role_has_permissions`ジョインによるロール+権限。`PermissionMiddleware`/`RoleMiddleware`（どちらもフェイルクローズ/デフォルト拒否）。`CreateRbacTables`マイグレーション。そして`create_role`/`create_permission`/`give_permission_to_role`ヘルパーです。
- **コンテンツレンダリング** - Markdownレンダリングとドキュメントビルドパイプラインです: `MarkdownRenderer`、`build_docs`、`DocsCatalog`/`DocsChapter`、見出し抽出、`slugify_heading`。レンダリングされたHTMLはサニタイズされます（comrak + syntect + ammonia）。
- **ネイティブな静的ファイル配信** - Webルートで`public/`ディレクトリを配信するための`StaticFiles::public()`フォールバックハンドラです。アプリ内で手作りされていたアセットごとの許可リストコントローラーを置き換えます。

### 修正

- 新しく生成されたアプリは、フレームワークレベルの`time = 0.3.47`互換ピン留めを継承するようになり、まっさらなスキャフォルドの依存関係解決における`time 0.3.48`由来のRust 1.96コヒーレンス衝突を避けます。

### ドキュメント

- 出荷済みの2つのスターターキット - **Nebula**（Breezeクラスの認証）と**Pulsar**（プロダクトサイト + コミュニティ） - を、マニュアル、README、ロードマップ全体にドキュメント化し、出荷済みの表面を軸にロードマップを再構成し、ドキュメント全体のバージョン参照を整合させました。

## 0.1.0 - 2026-06-10

Suprnovaの最初のリリースです。Suprnovaは、Rust向けのLaravelに着想を得たWebフレームワークで、Kitからフォークされ、独自の方向へ進んでいます。現時点での互換目標はLaravel 13.xです。

このリリースは、gitによる配布モデルを使います: フレームワークの利用者は`suprnova = { git = "https://github.com/eas4ai/suprnova.git" }`に依存し、CLIは`cargo install --git`でインストールします。

### 追加

#### HTTP、ルーティング、ミドルウェア

- ルートグループ、プレフィックス、パラメータ制約、名前付きルートを備えた`Router`
- `routes!`マクロによる、コンパイル時検証されるルート登録
- 7つの標準ルートを生成する、リソースルーティング（`Router::resource`）
- 署名付きURL（`url::signed_route`/`url::temporary_signed_route`フリー関数、および`Redirect::signed_route`/`Redirect::temporary_signed_route`）
- リダイレクトヘルパー - `Redirect::to`、`Redirect::back`、`Redirect::route`、`Redirect::with_input`、`Redirect::with_errors`、`with_flash`
- グローバル、グループ、ルートごとの層を持つミドルウェアトレイト
- 組み込みミドルウェア - CORS、CSRF、セッション、リクエストタイムアウト、リクエストID、throttle/ログインthrottle、署名付きURL検証、認証済み、メール確認済み、ブルートフォース対策
- アボートヘルパー（`abort`、`abort_unless`、`abort_if`）
- `suprnova::handle_request(...)` - ルーター + ミドルウェアチェーンに対して単一のhyperリクエストを処理する、公開アダプター

#### Inertia.jsフロントエンドブリッジ

- TypeScriptの型出力を伴う`#[derive(InertiaProps)]`
- コンパイル時のコンポーネント検証を伴う`inertia_response!`マクロ
- 3つのファーストクラスのスターターフロントエンド - **Svelte 5**（runes有効）、**React 19**、**Vue 3.5** - いずれもInertia 3.1.1 + Vite 8 + Tailwind v4の上に構築
- 部分リロード（`only`/`except`）、遅延プロパティ、永続レイアウト、暗号化された履歴、スクロール位置の保持
- ページネーター → Inertiaのprop配線のための`Inertia::paginate(component, key, paginator)`

#### Eloquent風ORM（SeaORMベース）

- SeaORMエンティティとユーザー向けのEloquent構造体を一度に生成する`#[suprnova::model]`アトリビュートマクロ
- フルセットの`Model`トレイト - `create`、`find`、`find_or_fail`、`find_many`、`all`、`query`、`save`、`update`、`delete`、`force_delete`、`refresh`、`fresh`、`replicate`、`replicate_into`、`increment`/`decrement`、`destroy`、`is`/`is_not`、`to_array`/`to_json`
- `Attrs`エンベロープによる、fillable/guardedなマスアサインメント
- 22種類のアトリビュートキャスト - 真偽値、整数、浮動小数点数、日付、enum、ハッシュ化、暗号化、JSON、コレクション、金額、タイムゾーン付き日時
- `#[suprnova::model]`によるアクセサ/ミューテータ
- 自動タイムスタンプ（`created_at`、`updated_at`）
- `force_delete`、`restore`、`trashed`、`only_trashed`、`with_trashed`を伴うソフトデリート（`deleted_at`）
- 11種類のリレーション - `HasOne`、`HasMany`、`BelongsTo`、`BelongsToMany`、`HasOneThrough`、`HasManyThrough`、`MorphOne`、`MorphMany`、`MorphTo`、`MorphToMany`、`MorphedByMany`
- ファミリーごとのmorph enumと、`APP_KEY_PREVIOUS`ローテーションを伴うmorphレジストリ
- `.with(...)`、`.with_count(...)`、`.load_missing(...)`によるeager loading
- `has`/`where_has`のための、相関EXISTSエンジン
- 16種類のライフサイクルイベント（retrieving、retrieved、creating、created、updating、updated、saving、saved、deleting、deleted、restoring、restored、force-deleting、force-deleted、replicating、trashed）
- inventoryによるメソッドごとの自動登録を伴う`Observer<M>`トレイト
- `#[scopes(M)]`によるローカルスコープ、`GlobalScope`によるグローバルスコープ
- `Collection<M>`のLaravel互換表面 - `pluck`、`key_by`、`group_by`、`where_in`、`first_where`、`contains_where`、`partition`など
- 3種類のページネーター - `paginate`（length-aware）、`simple_paginate`、`cursor_paginate` - いずれもLaravel形式のJSONへシリアライズ
- OOMを起こさない一括行イテレーションのための`chunk`/`lazy`/`cursor`
- `lock_for_update`/`shared_lock`による行レベルロック
- アドホックなクエリのための、`DynamicRow`を伴う`DB::table(...)`クエリビルダー
- セーブポイント、デッドロック時リトライ、複数コネクションでの読み書き分割を伴う`DB::transaction(...)`
- `DB::listen(...)` + `QueryExecuted`/`TransactionBegan`/`TransactionCommitted`/`TransactionRolledBack`イベント
- `Prunable`トレイト + `model:prune`コンソールコマンド
- `dump`/`dd`クエリヘルパーメソッド
- UUID/ULIDのプライマリキーのための`#[model(unique_id="...")]`

#### 認証

- `Authenticatable`トレイト + `EloquentUserProvider<M>`
- `Auth::attempt`、`Auth::login`、`Auth::user`、`Auth::user_or_fail`、`Auth::user_as<T>`、`Auth::logout`、`Auth::check`
- 複数の名前付き認証ガード（Webセッション、APIトークン）
- メール確認フロー - `EmailVerification`、`EnsureEmailVerifiedMiddleware`、署名付き確認URL、`EmailVerificationMail`
- パスワードリセットフロー - `PasswordReset`、throttleされたトークン、`PasswordChangedMail`、`PasswordResetLinkSent`イベント
- 二要素TOTP - 登録、検証、リカバリーコード、リプレイ保護
- ブルートフォース対策/ログインthrottle - IP + 識別子でキー化、`LoginThrottleMiddleware`
- 安定した不透明トークンによるremember-meクッキー
- 6種類の認証イベント - `LoginAttempted`、`LoggedIn`、`Authenticated`、`LoggedOut`、`PasswordResetLinkSent`、`EmailVerified`
- `github.com/eas4ai/suprnova-torii-rs`のToriiフォークに支えられたブラウザセッション

#### 認可

- `Gate`ファサード - `define`、`allows`、`denies`、`authorize`、`any`、`none`、`check`（同期・非同期の両バリアント）
- ポリシー登録のための`#[policy(Model)]`マクロ
- リソースルートの自動認可

#### 支払い

- プロバイダーに依存しない5トレイトの表面 - `Checkout`、`Payment`、`Subscription`、`CustomerStore`、`WebhookHandler`
- `PaymentProvider`という上位トレイト + `as_payment()`によるケイパビリティ照会
- DBミラー - `customers`、`subscriptions`、`subscription_items`、`payments`、`refunds`、`payment_webhook_events`（べき等性のためのUNIQUE）
- フロータグ付きの`SessionPayload`enum（ワンショット対サブスクリプション）
- ワークスペースクレートとしての、2つのリファレンスアダプター - `suprnova-payments-stripe`（ゲートウェイ、フルの`Payment`実装）、`suprnova-payments-paddle`（Merchant of Record、`Payment`実装なし）
- テスト用のモックプロバイダー

#### キュー、ジョブ、バッチ、チェーン

- `Job`トレイト - `handle`、`max_tries`、`backoff`、`timeout`、`fail_on_timeout`
- `Queue::push`、`Queue::push_later`、`Queue::push_unique`、`Queue::push_unique_later`
- ドライバー - `sync`、`null`、`redis`、`database`
- `JobMiddleware`トレイト - 6種類の組み込みミドルウェア
- バッチとチェーン - `Queue::batch(jobs).dispatch()`、流暢なチェーンビルダー、キャンセル、進捗追跡
- リプレイ機能付きの失敗ジョブストア
- グレースフルシャットダウン、設定可能な並行数、`catch_unwind`によるパニック回復、決着メトリクスを備えたワーカー
- キューイング、処理、失敗、リリース、ワーカーのライフサイクルをカバーする、12種類のキューイベント

#### ブロードキャストとWebSocket

- 型付きWebSocketエンドポイントのための`ws!()`マクロ + `Router::ws`
- `WsSocket`のSink/Stream分割
- `Supervisor`トレイトによる自動再起動スーパーバイザー
- `Channel`、`Private`、`Presence`の各チャネルを備えた`BroadcastHub`
- JSONエンベロープのプロトコル、presenceのjoin/leave/here、クラッシュ復旧を伴う設定可能なpresence TTL
- `EventDispatcher`への`Broadcastable`ブリッジ
- 設定可能なWS_TASKSドレインを伴う、pong不在時クローズのハートビート
- ルートごとのWebSocketミドルウェア
- 1 MiB/64 KiBのより安全なデフォルト + `WsConfig::generous()`ファクトリー
- オリジンポリシー + プロトコル違反時1011クローズ

#### 通知とメール

- `Notification`トレイト + `Notify::send(recipient, notification).await`
- メーラブル + Markdownテンプレートレンダリング
- データベース/メール/ブロードキャスト/web-pushの各チャネル
- VAPID署名 + RFC 8291 ECEペイロード暗号化（`suprnova-web-push`経由）
- VAPIDのsubject検証、retry-afterのパース、8 KiBの拒否ボディ上限
- 受信者の型付けのためのNotifiableトレイト

#### イベント

- 型付きイベントディスパッチャー - `EventFacade::dispatch`、`EventFacade::listen<E, L>`、`EventFacade::forget`
- キャンセル可能なsaving/updatingイベント（`EventResult::cancel`を返す）
- キュー可能なリスナー

#### ファイルシステム

- OpenDAL経由のマルチドライバーサポートを伴う`Storage::disk("name")` - local、S3、Azure、GCS
- move、copy、exists、size、mime、last-modified、prepend/append
- ストリーミングアップロードとダウンロード

#### キャッシュ

- `Cache::store("name")` + ドライバー登録
- ドライバー - memory、redis（上限付きconnect-timeout）、database、file
- `remember`、`forever`、`tags`、アトミックなincrement/decrement、ロック

#### ベクトルDB

- 4種類のドライバーを持つ`VectorDriver`トレイト - インメモリ、Qdrant（UUID-5 IDマッピング）、Pinecone（ネイティブな文字列ID）、MariaDBネイティブの`VECTOR(N)` + HNSWインデックス（11.7以降）
- コサイン/内積/ユークリッド距離

#### コンソールバイナリとCLI

- プロジェクトごとの`console`バイナリ - `php artisan`のRust版で、`#[suprnova::console::command]`経由でユーザー定義のコマンドを実行
- 型付き引数のための`#[derive(Command)]`
- `suprnova` CLI - `new`、`serve`、`migrate`、`db:sync`、`generate-types`、`key:generate`、`make:{controller,middleware,action,error,inertia,migration,task,command}`、`db:seed`、`model:prune`
- `--version`フラグ
- 3つのフロントエンドにまたがる、バックエンド + APIスターター向けのスキャフォルドテンプレート

#### フィーチャーフラグ

- スナップショット読み込みを伴う`DatabaseEvaluator`
- TTLを伴う`CachedEvaluator`
- `FeatureMiddleware`エクストラクター
- 管理用CRUD表面
- プロセス間でのサブ秒の伝播のための`FeatureSync`トレイト

#### スケジュール

- Cron式パーサー
- 組み立て可能な述語を伴う`Schedule::task(...)`
- シングルサーバーロック、重複実行の防止、ディスパッチ追跡
- `schedule:run`コンソールコマンド

#### バリデーション

- `validator` 0.20との統合
- `#[request]` + `#[derive(FormRequest)]`マクロ
- フォームごとのサイズ上限のための`#[form_request(max_body_bytes = N)]`
- ユーザーが書く`impl FormRequest`のためのオプトアウトである`#[form_request(custom_hooks)]`
- ライフサイクルフック - `authorize`、`after_validation`、`after_validation_async`

#### データベースドライバー

- SeaORMに支えられた、SQLite、Postgres、MySQL、MariaDBのサポート
- URLベースのドライバー検出
- マイグレーションシステム + `migrate`、`migrate:rollback`、`migrate:status`、`migrate:fresh`、`migrate:refresh`

#### HTTPクライアント

- `Http`ファサード - `RequestBuilder`を返す`get`/`post`/`put`/`patch`/`delete`。`.send().await`は`ClientResponse`を生成
- rustls TLS、30秒のデフォルトタイムアウト、`suprnova/<version>`のuser-agent
- `json`/`form`/`body`/`header`/`bearer_token`/`basic_auth`/`timeout`のチェーン可能なメソッド
- `RequestBuilder::retry(max_attempts, base_backoff)` - 一時的な失敗と5xxに対する指数バックオフ。`Retry-After`を尊重
- `fake_response(method, url_substring, status, body)` + `assert_sent`/`assert_not_sent`を伴う`Http::fake(|| async { ... }).await`テストガード

#### 暗号化

- `Crypt`静的ファサード + `EncryptionKey`（`crypto::*`）。12バイトのランダムノンスを伴うAES-256-GCM
- `encrypt_string`/`decrypt_string`/`encrypt<T>`/`decrypt<T>`
- クロスプロトコルのリプレイを防ぐ`CryptPurpose`のAADバインディング
- `APP_KEY_PREVIOUS`ローテーション
- 新しいキーを発行するための`suprnova key:generate` CLIコマンド

#### テスト

- `#[suprnova_test]`非同期テストマクロ
- 並行実行に安全なインスタンスを伴う`TestDatabase::fresh::<Migrator>()`
- テストごとのモックのための`TestContainer::bind`
- HTTPテストヘルパー - `Test::get`、`Test::post`、JSON/form/multipart
- キュー/メール/通知/イベントのフェイク
- `assert_emitted`、`assert_dispatched`、`assert_dispatched_times`

### 変更

- 認証確認とパスワードリセットのフローは、今ではTorii内部ではなく、設定済みのユーザープロバイダーを通じて動作するようになりました。
- 生成されるアプリは、`get_auth_password`を実装しなければなりません。スキャフォルドされたサンプルは、ログインを常にサイレントに失敗させるままにするのではなく、今ではっきりと失敗するようになりました。
- ローカルのリリースゲートは`scripts/release.sh`に配線されており、このリポジトリには、fmt、clippy、テスト、ドキュメント、フィーチャービルドのための、強制されるpre-pushフックが含まれています。
- スキャフォルドされた開発用ポートのドキュメントは、`dev:tls`と`--with-portless`のドキュメント化と共に、現行のバックエンド/フロントエンドのデフォルト（`8765`/`5765`）へ移されました。
- `MAIL_FROM`は、確認やリセットのトークンが発行される前に検証されるようになり、メール設定が無効な場合に認証フローの行が孤児になることを避けます。

### 修正

- Reactのスキャフォルドテンプレートが、リリースされたスターターからずれていた問題を修正しました。
- ルートのルートグループが、もう重複した`//`パスを生成しないようにしました。
- リテラルパスのリダイレクトが、今では意図したルーティング経路を通じてディスパッチされるようにしました。
- ブロードキャストのファンアウトテストが、今では`track`/`untrack`の結果を扱うようにしました。
- メールログドライバーは、レンダリングされたテキスト本文を出力するようになり、確認とパスワードリセットのリンクが、ローカル開発のログに現れるようにしました。
- パスワードリセットのテストカバレッジが、セッションとremember-meの失効挙動を固定するようにしました。

### 補足

- **配布モデル**: エンドツーエンドでgitベースです。`suprnova = { git = "https://github.com/eas4ai/suprnova.git" }`。CLIは`cargo install --git`経由です。crates.ioには何も公開されていません。
