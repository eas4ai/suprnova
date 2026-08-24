# モックとフェイク

Suprnovaのあらゆる外部表面は、あなたのコードが送ったはずのもの - メール、通知、キューに入れられたジョブ、ディスパッチされたコマンド、発火したイベント、書き込まれたファイル、アウトバウンドのHTTP呼び出し - をキャプチャするプロセス内のフェイクと、事後に実行する対応するアサーションの集まりを伴って出荷されます。その形は常に同じです - フェイクをインストールし、テスト対象のコードを実行し、キャプチャされたものをアサートします。この章は集約された概観です - 各サブシステムの章（[メール](mail.md)、[通知](notifications.md)、[キュー](queues.md)、[バス](bus.md)、[イベント](events.md)、[ファイルシステムとストレージ](filesystem.md)、[HTTP クライアント](http-client.md)）が、それぞれのフェイクを深く扱っています。

## 7つのフェイク

| 表面 | エントリポイント | アサーションのスタイル | 並列に対する安全性 | 章 |
|-----------------|---------------------------------------------------|---------------------------------------|----------------------------------------------------|--------------------------------------|
| メール | `Mail::fake()` → `MailFake` ガード | ガードの上のメソッド | `#[serial]` が必要 - グローバルなトランスポートで、シリアライザーなし | [mail.md](mail.md) |
| 通知 | `Notify::fake()` → `NotifyFakeGuard` | `notifications::testing` の中のフリー関数 | ガードがプロセス全体のシリアライザーを保持する | [notifications.md](notifications.md) |
| キュー | `suprnova::queue::testing::install_fake()` | `queue::testing` の中のフリー関数 | ガードがプロセス全体のシリアライザーを保持する | [queues.md](queues.md) |
| バス | `suprnova::bus::testing::install_fake()` | `bus::testing` の中のフリー関数 | ガードがプロセス全体のシリアライザーを保持する | [bus.md](bus.md) |
| イベント | `EventFacade::fake()` → `EventFakeGuard` | `events` の中のフリー関数 | ガードがプロセス全体のシリアライザーを保持する | [events.md](events.md) |
| ストレージ | `Storage::fake()` → `StorageFakeGuard` | ディスクの上の `DiskAssertExt` のメソッド | ガードがプロセス全体のシリアライザーを保持する | [filesystem.md](filesystem.md) |
| HTTPクライアント | `Http::fake(\|\| async { … }).await` | `assert_sent` / `assert_not_sent` | タスクローカル - テストをまたいで本当に並行 | [http-client.md](http-client.md) |

7つすべてを通じて成り立つ、いくつかの不変条件があります。

- **フェイクは記録し、本物のバックエンドは動きません。** メールは送信されず、ジョブはドライバーへプッシュされず、ハンドラは実行されず、イベントはそのリスナーをスキップし、HTTPはネットワークに触れず、ファイルの書き込みはメモリディスクへ向かいます。キャプチャされた側は、何が起きたはずかをアサートするのに十分な情報を運びます。
- **ガードはRAIIです。** ガードをドロップすると、それまでにあったもの（以前のメールトランスポート、クリーンなストレージレジストリ、イベントの記録なし、など）が復元されます。テストはティアダウンの手順を必要としません。
- **フェイクはエラーについて嘘をつきません。** あなたのコードが未登録のコマンドに対して `Bus::dispatch` を呼ぶと、フェイクはそれでも `Err(_)` を返します - キャプチャされるのは成功したディスパッチだけです。

## それぞれの形と、なぜ違うのか

3つのパターンが繰り返し現れます。どのパターンをフェイクが使っているかを知れば、フリー関数をインポートすべきか、ガードの上のメソッドを呼ぶべきか、テスト本体をクロージャでラップすべきかが分かります。

### ガード + メソッド（Mail）

`Mail::fake()` は `MailFake` を返し、その自身のメソッドがアサーションになります。アサートする側が*その*フェイク自身であるとき - すでにそれをローカル変数へバインドしているとき - これは便利ですが、この形をしているフェイクはこれだけです。

```rust,ignore
let fake = Mail::fake();
Mail::to("alice@example.org")
    .send(WelcomeEmail { name: "Alice".into() })
    .await?;
fake.assert_sent_count(1);
fake.assert_sent(|m| m.has_to("alice@example.org"));
```

### ガード + フリー関数（Notify、Queue、Bus、Events）

このガードは、フェイクをインストールされたままにしておくことだけが仕事の、何もしないトークンです - アサーションは、フェイクの内部の隣にある `testing` サブモジュールの中に存在します。必要なものをインポートしてください。

```rust,ignore
use suprnova::queue::testing::{install_fake, assert_pushed, pushed};

let _guard = install_fake();
schedule_welcome_email(user_id).await?;
assert_pushed::<WelcomeJob>(|j| j.user_id == user_id);
```

これは最も一般的な形です。型をまたいできれいに一般化できるからです - あらゆるアサーションは、ガードの型に焼き込まれるのではなく、`J: Job` / `C: Command` / `E: Event` に対して汎用的です。トレードオフは、追加で1つインポートが必要になることです。

キャプチャされたすべてのプッシュには、フェイクが割り当てたエンベロープIDが含まれるため、テストはキャプチャしたものをリスナーが見たものへ結び付けられます:

```rust,ignore
use suprnova::events::{EventFacade, dispatched};
use suprnova::queue::events::JobQueued;
use suprnova::queue::testing::{install_fake, pushed_with_id};

let _queue = install_fake();
let _events = EventFacade::fake();

Queue::push(SendInvoice { order_id: 7 }).await?;

let (job, id) = pushed_with_id::<SendInvoice>().remove(0);
assert_eq!(job.order_id, 7);
assert_eq!(dispatched::<JobQueued>(|_| true)[0].id, id);
```

フェイクの下にはドライバーがないため、フェイク自身が、記録したIDを使って本物のプッシュと同じ `JobQueueing` / `JobQueued` の組を発します。実経路でどちらのイベントも発しない `bulk` と `push_unique` については、フェイクも発しません。

### スコープ + クロージャ（HTTP）

`Http::fake` は、そこだけ毛色が違います。アウトバウンドのHTTPは、たまたま生きているどのTokioタスクの上でも実行されるため、フェイクの状態は `tokio::task_local!` の中に存在します。一度インストールしてそのまま乗っていく、ということはできません - クライアントを呼ぶ本体の方をラップしなければなりません。

```rust,ignore
use suprnova::{Http, fake_response, assert_sent};

Http::fake(|| async {
    fake_response("POST", "/api/users", 201, serde_json::json!({"id": 1}));

    let resp = Http::post("https://example.com/api/users")
        .json(&serde_json::json!({"name": "Ada"}))
        .send()
        .await?;

    assert_eq!(resp.status(), 201);
    assert_sent(|r| r.method == "POST" && r.url.contains("/api/users"));
})
.await;
```

その見返りはこうです。他のあらゆるフェイクはプロセス全体のシリアライザーを保持するため、並列テストは1つずつ順番に実行されますが、`Http::fake` は本当に並行です - あらゆるテストが自分専用のタスクローカルなレコーダーを手にし、それらは決して衝突しません。

### Storageの拡張トレイト

`Storage::fake()` はガード*と*デフォルトのインメモリディスクを返しますが、そのアサーションは、`DiskAssertExt` 拡張トレイトを通じて、ディスクそのものにぶら下がっています。

```rust,ignore
use suprnova::{Storage, DiskExt};
use suprnova::filesystem::testing::DiskAssertExt;

let _guard = Storage::fake();
let disk = Storage::disk("default")?;

disk.put("invoices/42.pdf", b"...").await?;
disk.assert_exists("invoices/42.pdf").await;
disk.assert_count("invoices/", 1, false).await;
```

この拡張トレイトは `#[cfg(any(test, feature = "testing"))]` でゲートされているため、本番環境のコードが誤って `disk.assert_exists(…)` を呼んでしまうことはありません。

## 並列に対する安全性を、1段落で

7つのフェイクのうち6つは、プロセスグローバルなstaticをガードしています。それぞれのガードは、構築時に専用の `FAKE_SERIAL` という `std::sync::Mutex` を取り、ドロップまでそれを保持します。その結果、同じフェイクをインストールする任意の2つの `#[tokio::test]` は、1つのプロセスの下で直列化されて実行されます - [serial_test](https://crates.io/crates/serial_test) クレートの `#[serial]` は不要です。**Mailは例外です**。`MailFake` ガードは、シリアライザーを取ることなくグローバルな `TRANSPORT` を差し替えるため、並行する `Mail::fake()` のテストは互いを壊してしまう*可能性があります*。これらには `#[serial]` を付けてください。**`Http::fake` もまた例外です**。これはプロセスグローバルではなくタスクローカルであるため、テストは本当に並列に実行され、`#[serial]` は決して必要ありません。

1つのテストバイナリの中で、同じ表面に対する本物のディスパッチとフェイクのディスパッチを織り交ぜる場合、本物の経路はシリアライザーを取らないため、並列に動くフェイクのテストと競合する可能性があります。その場合は、本物のディスパッチを行うテストに `#[serial]` を付けてください - それが当てはまる箇所については、章ごとのドキュメントがそれを指摘しています（典型例については[バス](bus.md)を参照してください）。

## メール - `Mail::fake()`

```rust,ignore
use serial_test::serial;
use suprnova::mail::{Mail, Address};

#[tokio::test]
#[serial]
async fn welcome_email_is_sent() {
    let fake = Mail::fake();

    register_user("alice@example.org").await.unwrap();

    fake.assert_sent_count(1);
    fake.assert_sent(|m| m.has_to("alice@example.org"));
    fake.assert_sent(|m| m.subject.starts_with("Welcome"));
    fake.assert_not_sent_to("eve@example.org");
}
```

| アサーション | 検証すること… |
|--------------------------------------------|-----------------------------------------------------|
| `fake.assert_sent(\|m\| pred)` | 少なくとも1つのキャプチャされたメッセージが一致する |
| `fake.assert_sent_to("…")` | 少なくとも1つのキャプチャされたメッセージが、そのメールへルーティングされた |
| `fake.assert_not_sent(\|m\| pred)` | 一致するキャプチャされたメッセージがない |
| `fake.assert_not_sent_to("…")` | そのメールへ向かったキャプチャされたメッセージがない |
| `fake.assert_sent_count(n)` | ちょうど `n` 個のキャプチャされたメッセージがある |
| `fake.assert_nothing_sent()` | 何もキャプチャされなかった |
| `fake.assert_queued("MailableName")` | この名前のキューに入れられたmailableが少なくとも1つある |
| `fake.assert_queued_with(name, \|q\| …)` | キューに入れられたmailableが、その述語に一致する |
| `fake.assert_queued_to("…")` | キューに入れられたmailableが、そのメールへルーティングされた |
| `fake.assert_not_queued("MailableName")` | この名前のキューに入れられたmailableがない |
| `fake.assert_queued_count(n)` | ちょうど `n` 個のキューに入れられたmailableがある |
| `fake.queued_on("…")`                      | キューに入れられたmailableがキューへルーティングされた |
| `fake.assert_queued_on(name, "…")`         | 指定した名前のキュー投入済みmailableがキューへルーティングされた |
| `fake.queued_on_connection("…")`          | キューに入れられたmailableが接続へルーティングされた |
| `fake.assert_queued_on_connection(name, "…")` | 指定した名前のキュー投入済みmailableが接続へルーティングされた |
| `fake.assert_nothing_queued()` | 何もキューに入れられなかった |
| `fake.assert_outgoing_count(n)` | 送信済み + キュー入りの合計が `n` |
| `fake.assert_nothing_outgoing()` | 何も送信されず、何もキューに入れられなかった |

`fake.captured()`、`fake.queued()`、`fake.sent(pred)`、`fake.sent_to(…)`、`fake.queued_named(…)`、`fake.queued_to(…)` は、一致するデータを返すため、独自のアサーションを組み立てられます。`Queue::fake` がインストールされていないときでも `Mail::queue` がフェイクへどのように反映されるかを含む、完全な表面については[メール](mail.md)を参照してください。

`queued_on_connection` / `assert_queued_on_connection` は `QueuedSnapshot::connection` を読み取ります - `.on_connection(...)` のオーバーライドがあればその値です - これは、下記の通常ジョブ経路で `Queue::fake` の `assert_pushed_on_connection` が読む同じフィールドであり、2つのフェイクは対称性を保ちます。


## 通知 - `Notify::fake()`

```rust,ignore
use suprnova::notifications::{Notify, testing};

#[tokio::test]
async fn order_shipped_notifies_customer() {
    let _guard = Notify::fake();

    ship_order(order_id).await.unwrap();

    testing::assert_sent_to("alice@example.org", "OrderShipped");
    testing::assert_sent_to_on("alice@example.org", "mail", "OrderShipped");
    testing::assert_sent_times("OrderShipped", 1);
}
```

| アサーション | 検証すること… |
|------------------------------------------------------|---------------------------------------------------|
| `assert_sent(\|r\| pred)` | 少なくとも1つのディスパッチされた通知が一致する |
| `assert_sent_to(route, "Name")` | 名前付きの通知が、このチャネルごとのルートへ向かった |
| `assert_sent_to_on(route, channel, "Name")` | このチャネルの上で、このルートへディスパッチされた |
| `assert_sent_named("Name")` | 名前付きの通知が、いずれかのチャネルの上でディスパッチされた |
| `assert_sent_times("Name", n)` | 名前付きの通知がちょうど `n` 個 |
| `assert_nothing_sent()` | 通知がディスパッチされなかった |
| `assert_count(n)` | あらゆる型とチャネルを合わせてちょうど `n` 個 |
| `assert_nothing_sent_to(route)` | このルートへ何もディスパッチされなかった |

`testing::recorded()` は、より細かいアサーションのために、あらゆる `FakeRecord`（通知名、チャネル、ルート、JSONデータ）を返します。通知の受信者は、チャネルごとの `route_for` の値でキー付けされているため、`assert_sent_to` はルートの文字列を取ります（`"mail"` に対してはメールアドレス、`"database"` に対しては文字列としてのidなど）- ルーティングのモデルについては[通知](notifications.md)を参照してください。

## キュー - `queue::testing::install_fake()`

```rust,ignore
use suprnova::Queue;
use suprnova::queue::testing::{
    install_fake, assert_pushed, assert_pushed_later, pushed,
};

#[tokio::test]
async fn order_placed_enqueues_charge() {
    let _guard = install_fake();

    place_order(42).await.unwrap();

    assert_pushed::<ChargeCustomerJob>(|j| j.order_id == 42);
}
```

| アサーション | 検証すること… |
|------------------------------------------------|----------------------------------------------------------------|
| `assert_pushed::<J>(\|j\| pred)` | `J` のプッシュのうち少なくとも1つが一致する |
| `assert_pushed_later::<J>(\|j, at\| pred)` | `J` のプッシュが `at` にスケジュールされた（遅延ディスパッチ） |
| `assert_pushed_on_queue::<J>(queue)`           | [`EnvelopeOverrides`](queues.md#per-push-overrides-with-envelopeoverrides) により `queue` を宣言した `J` のプッシュ |
| `assert_pushed_on_connection::<J>(connection)` | `EnvelopeOverrides` により `connection` を宣言した `J` のプッシュ |

データ側は、型付きのジョブそのものを返します。

- `pushed::<J>() -> Vec<J>` - `J` のキャプチャされたあらゆるプッシュ
- `pushed_with_available_at::<J>() -> Vec<(J, DateTime<Utc>)>` - 同じですが、各ジョブのスケジュールされたタイムスタンプも伴います
- `pushed_with_overrides::<J>() -> Vec<(J, EnvelopeOverrides)>` - 同じですが、ジョブが宣言したプッシュごとのオーバーライドも伴います

あらゆる `Queue::push`、`Queue::push_later`、`Queue::later`、`Queue::push_unique*`、そしてチェーン/バッチのディスパッチャーは、すべて同じレコーダーへ流れ込みます。フェイクの下での `push_unique` の意味（常に記録され「pushed」として報告されます）については、[キュー](queues.md)を参照してください。
`Queue::push_with` と `Queue::later_with` だけが `EnvelopeOverrides` を持つため、`pushed_with_overrides` は他のすべてのエントリポイントについて `EnvelopeOverrides::default()` を記録します - 通常の `Queue::push` はフェイクの下では「オーバーライドが宣言されていない」とまったく同じように読み取られ、`entries[0].1 == EnvelopeOverrides::default()` をアサートした場合と同じです。`assert_pushed_on_queue` / `assert_pushed_on_connection` が調べるのは解決済みのキュー名や接続名ではなく、*宣言された*オーバーライドです。`Queue::route` や `Job::queue` / `Job::connection` の解決はフェイクの下では実行されません（解決すべきドライバーへのプッシュがないため）。そのため、本番環境ならルートやジョブレベルのデフォルトへフォールスルーするジョブは、ここではオーバーライドなしで現れます。オーバーレイが運ぶその他のもの - `timeout`、`fail_on_timeout`、`max_tries`、`backoff` - をアサートするには、`pushed_with_overrides` を直接使ってください。

## バス - `bus::testing::install_fake()`

```rust,ignore
use suprnova::Bus;
use suprnova::bus::testing::{
    install_fake, assert_dispatched, assert_dispatched_times,
    assert_not_dispatched, assert_nothing_dispatched,
};

#[tokio::test]
async fn order_placed_dispatches_charge() {
    let _guard = install_fake();

    place_order(42).await.unwrap();

    assert_dispatched::<ChargeCustomer>(|c| c.customer_id == 42);
    assert_dispatched_times::<ChargeCustomer>(|_| true, 1);
    assert_not_dispatched::<RefundCustomer>(|_| true);
}
```

| アサーション | 検証すること… |
|-----------------------------------------------------|-----------------------------------------------------------------|
| `assert_dispatched::<C>(\|c\| pred)` | `C` のディスパッチされたコマンドのうち少なくとも1つが一致する |
| `assert_not_dispatched::<C>(\|c\| pred)` | 一致する `C` のディスパッチされたコマンドがない |
| `assert_dispatched_times::<C>(\|c\| pred, n)` | `C` のディスパッチされたコマンドのうち、ちょうど `n` 個が一致する |
| `assert_nothing_dispatched()` | アクティブなフェイクの下で、いずれの型のコマンドもディスパッチされていない |

フェイクの下では、`Bus::dispatch` はハンドラを実行する代わりに `Ok(Dispatched::Captured)` を返します。本物の失敗 - エンコード/デコードのエラー、フェイクがインストールされる前に登録されたハンドラがない、といったもの - は、それでも `Err(_)` として表面化します。[バス](bus.md)を参照してください。

## イベント - `EventFacade::fake()`

```rust,ignore
use suprnova::EventFacade;
use suprnova::events::{
    assert_dispatched, assert_dispatched_once, assert_dispatched_times,
    assert_not_dispatched, assert_nothing_dispatched, dispatched,
    dispatched_count, dispatched_events, has_dispatched,
};

#[tokio::test]
async fn registration_dispatches_welcome_event() {
    let _guard = EventFacade::fake();

    register_user("ada@example.com").await.unwrap();

    assert_dispatched_once::<UserRegistered>();
    assert_dispatched::<UserRegistered>(|e| e.email == "ada@example.com");
}
```

| アサーション | 検証すること… |
|----------------------------------------|-----------------------------------------------------|
| `assert_dispatched::<E>(\|e\| pred)` | ディスパッチされた `E` のうち少なくとも1つが一致する |
| `assert_dispatched_once::<E>()` | ちょうど1つの `E` がディスパッチされた |
| `assert_dispatched_times::<E>(n)` | `E` がちょうど `n` 個ディスパッチされた |
| `assert_not_dispatched::<E>(\|e\| ..)` | 一致する `E` がディスパッチされなかった |
| `assert_nothing_dispatched()` | いずれの型のイベントもディスパッチされなかった |
| `assert_listening::<E, L>()` | リスナー `L` が `E` に対して登録されている |
| `has_dispatched::<E>()` | `bool`: 記録された `E` があるか |
| `dispatched::<E>(\|e\| pred)` | 一致するイベントの `Vec<E>` クローン |
| `dispatched_count::<E>(\|e\| pred)` | 一致するイベントの個数 |
| `dispatched_events()` | あらゆるディスパッチの `HashMap<&'static str, usize>` |

2つのバリアントが、何がフェイクされるかを絞り込みます。

```rust,ignore
// これらのイベントだけをフェイクする。他のすべては通常どおりディスパッチされる。
let _guard = EventFacade::fake_only(&["UserRegistered", "UserDeleted"]);

// これら以外のすべてのイベントをフェイクする。
let _guard = EventFacade::fake_except(&["TelemetryEvent"]);
```

そして、記録せずに抑制する1つのバリアントもあります。

```rust,ignore
EventFacade::muted(async {
    // リスナーは何も発火せず、イベントは何も記録されない。
    run_bulk_import().await;
})
.await;
```

`muted` はシリアライザーを取得**しない**ため、mutedのスコープは並列に実行できます。フェイクのスコープの*内側*でだけ起きるリスナーの登録を観測する `assert_listening` を含む、完全な仕組みについては[イベント](events.md)を参照してください。

## ストレージ - `Storage::fake()`

```rust,ignore
use suprnova::{Storage, DiskExt};
use suprnova::filesystem::testing::DiskAssertExt;

#[tokio::test]
async fn invoice_upload_persists() {
    let _guard = Storage::fake();
    let disk = Storage::disk("default").unwrap();

    upload_invoice(b"%PDF-1.7 …").await.unwrap();

    disk.assert_exists("invoices/2026/05/30/inv-00042.pdf").await;
    disk.assert_contents("invoices/2026/05/30/inv-00042.pdf", b"%PDF-1.7 …").await;
}
```

このガードは、`"default"` というインメモリディスクを事前に登録するため、簡単なテストはディスクのセットアップを何も必要としません。テスト対象のコードがデフォルト以外のディスクに手を伸ばす場合は、テストの内側から `Storage::register_memory("audit_logs")` で、カスタムの名前の下に追加のディスクを登録してください。

| アサーション | 検証すること… |
|--------------------------------------------------|---------------------------------------------------|
| `disk.assert_exists(path).await` | そのパスが存在する |
| `disk.assert_contents(path, &expected).await` | そのファイルが `expected` とバイト単位で一致する |
| `disk.assert_missing(path).await` | そのパスが存在しない |
| `disk.assert_count(dir, n, recursive).await` | `dir` がちょうど `n` 個のエントリを含む |
| `disk.assert_directory_empty(dir).await` | `dir` にエントリがない（再帰的に） |

5つすべては、不一致のときにディスクのパスをメッセージに含めてパニックします。`Storage` ファサード自体と、ドライバーの話（memory / fs / s3 / azblob / gcs）については、[ファイルシステムとストレージ](filesystem.md)を参照してください。

## HTTPクライアント - `Http::fake`

```rust,ignore
use suprnova::{Http, fake_response, assert_sent, assert_not_sent};

#[tokio::test]
async fn payment_webhook_is_acked() {
    Http::fake(|| async {
        fake_response("POST", "/v1/charges", 201, serde_json::json!({
            "id": "ch_42",
            "status": "succeeded",
        }));

        let result = charge_card(amount_cents).await;

        assert!(result.is_ok());
        assert_sent(|r| r.method == "POST" && r.url.contains("/v1/charges"));
        assert_not_sent(|r| r.method == "DELETE");
    })
    .await;
}
```

`fake_response(method, url_substring, status, body)` は、1つの用意されたレスポンスをキューに入れます。メソッド `"*"` はあらゆるメソッドに一致します。用意された各エントリは、最初に一致したリクエストで消費されます - それ以降の一致するリクエストは、次の用意されたエントリへ流れ落ちるか、空の `200 {}` を返します。

| ヘルパー | 用途 |
|----------------------------------------------|-----------------------------------------------------------|
| `Http::fake(\|\| async { … }).await` | タスクローカルなフェイクのスコープをインストールする |
| `fake_response(method, url_substring, …)` | 用意されたレスポンスをキューに入れる |
| `assert_sent(\|r\| pred)` | 記録されたリクエストのうち少なくとも1つが一致することをアサートする |
| `assert_not_sent(\|r\| pred)` | 一致する記録されたリクエストがないことをアサートする |

### spawnされたタスクは、デフォルトではフェイクを継承しない

`tokio::spawn` は、spawnされたfutureへタスクローカルを運びません。そのため、親のタスクから逃れる作業は、フェイクからも逃れてしまいます。これに対処する2つの道具があります。

```rust,ignore
// 二重の安全策: フェイクされなかったアウトバウンドの呼び出しをすべて、ハードなエラーにする。
let _guard = suprnova::FailOnRealCallsGuard::install();

Http::fake(|| async {
    fake_response("GET", "/child", 204, serde_json::json!({}));

    // 明示的なオプトイン: この子タスクは、親のフェイクの状態を見る。
    let handle = Http::spawn_with_fake_inheritance(async {
        Http::get("https://child.test").send().await
    });

    let response = handle.await.unwrap().unwrap();
    assert_eq!(response.status(), 204);
})
.await;
```

`FailOnRealCallsGuard` はRAIIです - テストの先頭でこれをインストールすれば、アクティブなフェイクに当たらないあらゆるアウトバウンドの呼び出しは、ネットワークに触れる代わりにエラーになります。`Http::spawn_with_fake_inheritance` は、親のフェイクの状態を共有すべきタスクのための、明示的なオプトインです。完全な議論については[HTTP クライアント](http-client.md)を参照してください。

## ブロードキャスト

WebSocketのブロードキャストには並列なテストフィクスチャがありますが、その形は十分に異なるため、自分専用の章の中に存在します - `RecordingBroadcastHub` は本物の `BroadcastHub` であり、生きている購読者への配信を続けながら、発行されたあらゆるエンベロープを記録します。`InMemoryBroadcastHub` の代わりにこれをバインドし、`hub.broadcasts()` / `hub.assert_broadcast(channel, event)` を呼んでください。ブロードキャストのモデルと、記録用ハブの使い方については[ブロードキャスト](broadcasting.md)を参照してください。

## 各フェイクの実装場所

| 表面 | ソース | ファサードの再エクスポート |
|---------------|----------------------------------------|----------------------------------------------|
| メール | `framework/src/mail/mod.rs` | `suprnova::{Mail, MailFake}` |
| 通知 | `framework/src/notifications/testing.rs` | `suprnova::{Notify, NotifyFakeGuard}` + `suprnova::notifications::testing::*` |
| キュー | `framework/src/queue/testing.rs` | `suprnova::queue::testing::*` |
| バス | `framework/src/bus/testing.rs` | `suprnova::bus::testing::*` |
| イベント | `framework/src/events/testing.rs` | `suprnova::{EventFacade, EventFakeGuard}` + `suprnova::events::*` |
| ストレージ | `framework/src/filesystem/testing.rs` | `suprnova::{Storage, DiskExt}` + `suprnova::filesystem::testing::DiskAssertExt` |
| HTTP | `framework/src/http_client/fake.rs` | `suprnova::{Http, fake_response, assert_sent, assert_not_sent, FailOnRealCallsGuard, RecordedRequest}` |

`testing` と `fake` のモジュールは、`testing` という名前のCargoフィーチャーの背後にゲートされています。これはデフォルトのフィーチャーセットに含まれているため、`suprnova` に依存するあらゆるテストは、これらのヘルパーを無償で手にします。フックそのものは、アプリケーションコードから誤って到達しうる場所では `#[doc(hidden)]` です - 本当に重要な安全策は `Server::from_config` の `APP_KEY` バリデーションであり、これはどのテストヘルパーがコンパイルされているかにかかわらず、あらゆる起動のたびに実行されます。本番環境のビルドの話については[テスト](testing.md)を参照してください。

## なぜこれらの形で、1つの形ではないのか

単一の統一された形は、ページの上ではきれいに見えても、実践においてはより悪いものになるでしょう。それぞれの形が存在するのは、その裏にある状態が、異なる並行性の意味を持っているからです。

- **Mail**のトランスポートは、ガードによって差し替えられるグローバルな `Arc<dyn MailTransport>` です。返されたガードの上のメソッドによるアサーションは、アサートする側を特定のインストールへ結び付けるため、フェイクがアクティブでないときにアサーションを呼ぶことは不可能になります。
- **Notify / Queue / Bus / Events** は、異種の型付きペイロードに対してアサートします - あらゆるアサーションは、イベント/ジョブ/コマンドの型に対して汎用的です。`testing` モジュールの中のフリー関数は、ガードの上に手書きされたメソッド集合よりも、型パラメータときれいに組み合わさります。
- **Storage** のアサーションは、フェイクごとではなくディスクごとです - 同じ `disk.assert_exists(…)` が、統合テストスイートの中の、フェイクされたメモリディスクに対しても、本物の `s3` ディスクに対しても機能します。拡張トレイトを介してそれらをディスクの上に置くことで、その対称性が保たれます。
- **HTTP** は、呼び出し側のスタックではなくタスクに追従しなければなりません。`Http::fake` は、そのスコープをガードとして表現できない唯一のフェイクです - spawnのセマンティクスが、クロージャを強制します。

存在しないヘルパーに手を伸ばしていることに気付いたら、該当する章を読んでください - 公開されているテストの表面は、サブシステムごとに網羅的に文書化されています。

## 次のステップ

- [テスト](testing.md) - `#[suprnova_test]` マクロ、`TestDatabase`、`expect!`、そして `TestContainer::fake`
- [HTTP テスト](http-tests.md) - ソケットを開かずに `handle_request` を直接駆動する
- [データベース テスト](database-testing.md) - テストごとのインメモリデータベースの話
- [サービス コンテナ](container.md) - 注入されたサービスを差し替えるための `TestContainer::fake`
