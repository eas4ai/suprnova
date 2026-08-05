# バス

バスは、Suprnovaの**同期的な**コマンドディスパッチャーです。型付きの `Command`（`{ 入力, Output型 }`）を定義し、起動時にそれ用の `Handler` を登録すれば、プロセス内のどのコードからでも `Bus::dispatch(cmd).await` を呼び出し、ハンドラの型付きの結果を運ぶ `Dispatched<T>` を受け取れます。

バスは、非同期の兄弟である[`Queue`](queues.md)と対をなします。これらは意図的に分離された2つのファサードであり、1つの振り分けディスパッチャーではありません。

| したいことが…                                          | 使うもの       |
|-------------------------------------------------------|----------------|
| このタスクの中で*今すぐ*作業を実行し、結果を受け取りたい | `Bus`          |
| ワーカーへ作業を送り出し、失敗時にリトライし、永続化させたい | `Queue`        |

呼び出し元が明示的に選びます。Suprnovaは `ShouldQueue` マーカーを出荷していません - Tokio上ではどちらの経路もノンブロッキングであるため、明示的な選択は暗黙のルーティングよりも明快で高速です。

## クイックスタート

コマンドからディスパッチまで、わずか10行です。

```rust
use serde::{Deserialize, Serialize};
use suprnova::async_trait;
use suprnova::bus::command::{Command, Handler};
use suprnova::bus::Bus;
use suprnova::error::FrameworkError;

#[derive(Serialize, Deserialize)]
pub struct ChargeCustomer { pub customer_id: i64, pub cents: i64 }

#[async_trait]
impl Command for ChargeCustomer {
    type Output = String; // 受け取った課金ID
    fn command_name() -> &'static str { "ChargeCustomer" }
}

pub struct ChargeCustomerHandler;

#[async_trait]
impl Handler<ChargeCustomer> for ChargeCustomerHandler {
    async fn handle(&self, cmd: ChargeCustomer) -> Result<String, FrameworkError> {
        Ok(format!("charge-{}-{}", cmd.customer_id, cmd.cents))
    }
}

// 起動時に（一度だけ）:
Bus::register::<ChargeCustomer, _>(ChargeCustomerHandler);

// リクエストハンドラの中で:
let charge_id = Bus::dispatch(ChargeCustomer { customer_id: 42, cents: 1999 })
    .await?
    .unwrap_executed();
```

## コマンドを定義する

`Command` とは、関連付けられた `Output` 型と一意な `command_name()` を持つ、シリアライズ可能な任意の構造体です。

```rust
#[async_trait]
pub trait Command: Serialize + DeserializeOwned + Send + Sync + 'static {
    type Output: Send + 'static;
    fn command_name() -> &'static str;
}
```

`Output` は、ハンドラが返すものです。これは `Send + 'static` でありさえすればよく - 実際のディスパッチ経路は、serdeの往復を伴わない `Box<dyn Any>` を介して、値をネイティブなまま保持します。つまり、`Bytes`、不透明なハンドル、`Arc<Mutex<…>>` のようなserde非対応の出力も、生きた値のまま呼び出し元へ往復します。`Command` 自体に付いている `Serialize + DeserializeOwned` の制約は、フェイクキャプチャの経路のためのものです: `Bus::fake()` は、ディスパッチされた各コマンドを `serde_json::Value` として記録するため、述語ベースのアサーション（`assert_dispatched`、`assert_dispatched_times`）がそれらをデコードして調べられます。

`command_name()` は、具体的な `Command` の実装ごとに一意で安定した文字列であるべきです。これは、`assert_dispatched` / `assert_dispatched_times` の失敗メッセージや、ハンドラが登録されていない場合のエラー戻り値の中に現れます。

## ハンドラを登録する

`Handler<C>` は、コマンドを受け取り `Result<C::Output, FrameworkError>` を返す、型付きの非同期関数です。

```rust
#[async_trait]
pub trait Handler<C: Command>: Send + Sync + 'static {
    async fn handle(&self, cmd: C) -> Result<C::Output, FrameworkError>;
}
```

起動時に、コマンドの型ごとに一度 `Bus::register::<C, H>(handler)` を呼び出します。レジストリはグローバルです。同じ `C` を再登録すると、以前のハンドラが上書きされ（テストは実装を入れ替えるためにこれに依存しています）、2つの起動時サービス登録による重複バインディングがログで見えるよう、`tracing::warn!` が発されます。

```rust
Bus::register::<ChargeCustomer, _>(ChargeCustomerHandler);
Bus::register::<RefundCustomer, _>(RefundCustomerHandler);
```

## ディスパッチする

`Bus::dispatch::<C>(cmd)` は、登録済みのハンドラをプロセス内で実行し、`Dispatched<C::Output>` という列挙型を返します。

```rust
pub enum Dispatched<T> {
    Executed(T),  // ハンドラが実行された。これが結果
    Captured,    // Bus::fake() が有効だった。ハンドラは実行されなかった
}
```

`Dispatched<T>` には4つのヘルパーがあります。

- `.unwrap_executed()` - 値を返す。`Captured` ならパニックする
- `.executed() -> Option<T>` - `Option` へ変換する
- `.is_executed()` - bool の述語
- `.is_captured()` - bool の述語

実モードの呼び出し箇所では、`.unwrap_executed()` がイディオマティックな形です。

### `Bus::chain` - 逐次実行

`Bus::chain(Vec<C>)` は、コマンドを1つずつ実行し、最初のエラーで（それを含めて）停止します。すべてのコマンドは同じ型でなければなりません。試行されたコマンドごとに1エントリを持つ `Vec<Result<Dispatched<C::Output>, FrameworkError>>` を返します。

```rust
let results = Bus::chain(vec![
    ChargeCustomer { customer_id: 1, cents: 100 },
    ChargeCustomer { customer_id: 2, cents: 200 },
    ChargeCustomer { customer_id: 3, cents: 300 },
]).await;

// 最初の失敗までの、成功した課金IDを収集する:
let charge_ids: Vec<String> = results
    .into_iter()
    .filter_map(|r| r.ok().and_then(|d| d.executed()))
    .collect();
```

`Bus::chain` は、設計上、同種の型のみを扱います - ディスパッチャーは `Dispatched<C::Output>` を返しますが、これはすべての入力が1つの `Output` を共有している場合にだけ正しく型付けされます。Laravel流の異種混合のチェーン（複数のジョブ型が混在し、各ステップが次を起動する形）には、[`Queue::chain`](queues.md)を使ってください - キューは各ジョブを型付きのエンベロープへ詰め込むため、同じ制約を持ちません。

### `Bus::batch` - 並行実行

`Bus::batch(Vec<C>)` は、`futures::join_all` を介してコマンドを並行に実行し、結果を入力順に収集します。`chain` と同じ、同種の型のみという制約があります。

```rust
let results = Bus::batch(vec![
    SendWelcomeEmail { user_id: 1 },
    SendWelcomeEmail { user_id: 2 },
    SendWelcomeEmail { user_id: 3 },
]).await;
```

`Bus::batch` は、`chain` と同じ理由で同種の型のみを扱います。進捗コールバック、ライフサイクルイベント、`BatchRepository` を伴う、異種混合の永続化されたバッチには、[`Queue::batch`](queues.md)を使ってください。

## テスト

テストの先頭でフェイクをインストールしてください。`install_fake()` は、ガードの寿命の間、プロセス全体の `FAKE_SERIAL` mutexを取得するため、並行する2つの `Bus::fake()` テストが互いのキャプチャストアを壊し合うことはありません - 2番目のテストは、最初のガードがドロップされるまでブロックされます。それでも、同じバイナリ内の兄弟テストが本物の `Bus::dispatch` を呼び出す場合は、そのテストに `#[serial]` を付けてください: 本物のディスパッチを行う呼び出し元は `FAKE_SERIAL` を取得しないため、`#[serial]` がなければ、並行するフェイクテストと競合し、`is_active() == true` を観測してしまう可能性があります。`FAKE_SERIAL` はフェイク同士の危険を取り除き、`#[serial]` は本物とフェイクの間の危険を取り除きます。

```rust
use serial_test::serial;
use suprnova::bus::Bus;
use suprnova::bus::testing::{
    assert_dispatched,
    assert_dispatched_times,
    assert_not_dispatched,
    assert_nothing_dispatched,
    install_fake,
};

#[tokio::test]
#[serial]
async fn order_placed_dispatches_charge() {
    let _guard = install_fake();

    place_order(/* … */).await.unwrap();

    assert_dispatched::<ChargeCustomer>(|c| c.customer_id == 42);
    assert_dispatched_times::<ChargeCustomer>(|_| true, 1);
    assert_not_dispatched::<RefundCustomer>(|_| true);
}
```

フェイクは、ハンドラを実行することなく、ディスパッチされたコマンドをキャプチャします。`Bus::dispatch` の呼び出しは、`Executed` ではなく `Ok(Dispatched::Captured)`（ハンドラの出力なし）を返します。本物のエラー - エンコード/デコードの失敗、フェイクがインストールされる前に登録されたハンドラが見つからない、といったもの - は、それでも `Err(_)` として表面化します。

`install_fake()` は `BusFakeGuard` を返します。これをドロップすると（RAIIです）、フェイクがクリアされ、`FAKE_SERIAL` mutexが解放されます。典型的なイディオムは、テストの先頭で `let _guard = install_fake();` とすることです。

### アサーションの表面

| アサーション                                          | 検証すること…                                              |
|------------------------------------------------------|------------------------------------------------------------|
| `assert_dispatched::<C>(pred)`                       | `pred` に一致する型 `C` のコマンドが少なくとも1つある       |
| `assert_not_dispatched::<C>(pred)`                   | `pred` に一致する型 `C` のコマンドが0個である               |
| `assert_dispatched_times::<C>(pred, count)`          | `pred` に一致する型 `C` のコマンドがちょうど `count` 個である |
| `assert_nothing_dispatched()`                        | 有効なフェイクの下で、どの型のコマンドも0個ディスパッチされている |

フェイクがインストールされていない場合、4つすべてが `Bus::fake() must be active` でパニックします。型にスコープされたものは、個数が一致しないときに `expected … dispatched <command_name> …` でパニックします。`assert_nothing_dispatched` は `expected no dispatched commands but found <n>` でパニックします。

## 代わりに `Queue` を使うべきとき

次のいずれかが欲しいときは、[`Queue`](queues.md)に手を伸ばしてください。

- **再起動をまたぐ永続性。** ドライバーが `database` または `redis` であれば、キューに入れられたジョブはプロセスのクラッシュを生き延びます。
- **バックオフ付きのリトライ。** キューワーカーは、失敗のたびに `Job::max_tries` + `Job::backoff`（exponential / fixed / sequence）を適用します。
- **ジョブごとのタイムアウト。** `Job::timeout` + `Job::fail_on_timeout` は、ワーカーループによって尊重されます。
- **遅延実行。** `Queue::later(duration, job)` または `Queue::push_later(job, at)`。
- **重複排除 / べき等性。** `Job::unique_id` + `Queue::push_unique` が、設定可能なTTLの間、再提出をゲートします。
- **呼び出し元をワーカーから切り離す。** `cargo run --bin app -- queue:work` ワーカーの、別個のフリート上でジョブを実行します。

次のいずれかが欲しいときは、`Bus` に手を伸ばしてください。

- **プロセス内で、今すぐ実行。** プロセスをまたぐシリアライズがありません。
- **呼び出し元への型付きの結果。** `Dispatched<C::Output>` が、ハンドラの型付きの戻り値を呼び出し箇所まで運びます。
- **同期的な合成。** 作業をより小さな `Command` 呼び出しへ分解し、各結果を順番に読み取るリクエストハンドラ。

典型的なアプリは両方を使います: 同期的なリクエスト経路は、結果を返す操作を `Bus` を通じてディスパッチし、「投げて忘れる」/永続的な作業は `Queue` を通じて送り出します。

## 次のステップ

- [キュー](queues.md) - 非同期の兄弟、ドライバー、ワーカー、リトライポリシー、異種混合のチェーンとバッチ
- [イベント](events.md) - pub/subディスパッチャー（1つのイベント → 複数のリスナー）
- [ワークフロー](workflows.md) - チェーンでは足りないときの、再起動を生き延びる長時間実行のステートフルな作業
- [テスト](testing.md) - `#[suprnova_test]`、コンテナのフェイク、そして `Bus::fake()` が使う、プロセス全体のシリアライザーパターン
