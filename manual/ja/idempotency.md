# べき等性

クライアントがPOSTをリトライするとき、2回目の呼び出しが安全であってほしいはずです。ネットワークは信頼できず、クライアントはリトライします - しかし `POST /charges` はカードに二重に請求してはならず、`POST /orders` は1回のクリックに対して2つの注文を生み出してはなりません。べき等キーは、「同じキーをもう一度見たら、最初の答えを返してくれ。作業をやり直すな。」と告げる契約です。

Suprnovaの `Idempotency` は、`Cache::lock` の上の薄いファサードであり、段階的に強くなる3つの保証を与えます: 重複排除のみ、失敗時リトライ付きの重複排除、そしてStripeスタイルの結果リプレイです。3つとも、本体が実行されている間はロックのリースを生かし続けるため、遅い本体がロックを期限切れにさせ、重複をすり抜けさせることは決してありません。

```rust
use std::time::Duration;
use suprnova::{Idempotency, Idempotent};

let outcome: Idempotent<OrderId> = Idempotency::once(
    "create-order:user-42:client-key-abc",
    Duration::from_secs(86_400),
    || async {
        // キーごとに、24時間のウィンドウ内で正確に1回だけ実行される。
        place_order(&user, &cart).await
    },
)
.await?;

match outcome {
    Idempotent::Fresh(id) => /* 最初の呼び出し - id は新しい注文 */ {},
    Idempotent::FreshUnfenced(id) => {
        // 注文は作成されたが、ロックのリースは途中で
        // 失われたため、別の呼び出し元も1つ作成したかもしれない。整合を取るか
        // アラートを出すこと - 下記の「排他性が失われたとき」を参照。
    },
    Idempotent::Duplicate => /* 同じキーが既に使われた */ {},
}
```

## 3つのプリミティブ

| メソッド | 本体の実行 | 重複が見るもの | 失敗時にロックを解放するか? | 使うとき |
|---|---|---|---|---|
| `Idempotency::once` | ウィンドウごとに正確に1回 | `Duplicate` マーカー | いいえ | 副作用が絶対に繰り返されてはならないとき（メール送信、請求の試行） |
| `Idempotency::commit_on_success` | ウィンドウごとに成功1回 | `Duplicate` マーカー | はい | 一時的な失敗はリトライ可能であるべきだが、成功は保持されるべきとき |
| `Idempotency::remember` | ウィンドウごとに成功1回 | 元の戻り値 | はい | 重複が、マーカーではなく元のペイロードを受け取らなければならないとき |

3つとも `suprnova::idempotency` の下に存在し、クレートルートから `Idempotency`、`Idempotent`、`Replay` として再エクスポートされています。これらは、同じキーのハッシュ化、リースの更新、そしてロックのセマンティクスを共有しています - 異なるのは成功/失敗のポリシーだけです。

### `Idempotency::once` - 最大1回

最も厳格な契約です。TTLウィンドウ内の最初の呼び出し元が本体を実行し、`Fresh(value)` を得ます。ウィンドウ内のそれ以降のすべての呼び出し元は `Duplicate` を得て、本体は再び実行され*ません* - たとえ最初の呼び出し元の本体が `Err` を返していたとしてもです。TTLこそが重複排除のウィンドウです。

```rust
use std::time::Duration;
use suprnova::{Idempotency, Idempotent};

// サインアップのコールバックが何回リトライされようと、サインアップごとに
// ウェルカムメールを正確に1回だけ送信する。
let result = Idempotency::once(
    &format!("welcome-mail:{}", user.id),
    Duration::from_secs(7 * 24 * 3600),
    || async {
        Mail::to(&user.email).send(WelcomeMail { user: user.clone() }).await
    },
)
.await?;
```

副作用が、「試した。副作用のあとでエラーになったとしても、二度と試すな」という種類のものであるときは `once` に手を伸ばしてください - メールの送信、独自のべき等キーを尊重しない外部APIへの投稿、二重書き込みが下流の分析を壊してしまう監査ログエントリの記録などです。

### `Idempotency::commit_on_success` - 成功時は少なくとも1回、失敗時はリトライ

`once` と似ていますが、本体が `Err` を返した場合、重複排除ロックは解放され、TTLウィンドウ内の次の呼び出し元がリトライできるようになります。成功した本体は、ウィンドウの残りの間ロックを保持し続けます。

```rust
use std::time::Duration;
use suprnova::{Idempotency, Idempotent};

let outcome = Idempotency::commit_on_success(
    &format!("publish-post:{}", post.id),
    Duration::from_secs(300),
    || async {
        // 上流のサービスへメッセージを投稿する。ネットワークエラーは
        // 一時的なものであり - 実際には何も起きていないのに「もう完了した」と
        // 告げるのではなく、次のリトライは再び入るべきである。
        social_media_client.post(&post).await
    },
)
.await?;
```

本体がリトライ可能な失敗モード（一時的なネットワークエラー、上流のレートリミット、リフレッシュで直る期限切れの認証情報）を持ち、成功に対しては少なくとも1回を望みつつ、失敗時にはリトライが再び入れるようロックを明け渡してほしいときは `commit_on_success` を使ってください。

### `Idempotency::remember` - Stripeスタイルの結果リプレイ

HTTPの `Idempotency-Key` ヘッダーが発明された、まさにその契約です。最初の呼び出し元は本体を実行し、成功値を保存して `Replay::Fresh` を得ます。ウィンドウ内の後続の呼び出し元は `Replay::Replayed(<元の値>)` を得ます - マーカーではなく、記録された戻り値です。最初の呼び出しが*まだ*実行中である間に到着した並行する呼び出し元は、`Replay::InProgress` を得ます。

```rust
use std::time::Duration;
use suprnova::{
    handler, Auth, FrameworkError, HttpResponse, Idempotency, Replay, Request, Response,
};

#[handler]
pub async fn create_charge(req: Request) -> Response {
    // 本体のために `req` を消費する前に、ヘッダーを所有権を持つStringへ抽出する。
    let key = req
        .header("Idempotency-Key")
        .ok_or_else(|| FrameworkError::bad_request("Idempotency-Key header required"))?
        .to_string();

    let user = Auth::user_as::<User>()
        .await?
        .ok_or_else(|| FrameworkError::unauthorized("login required"))?;

    let form: ChargeForm = req.json().await?;

    let outcome = Idempotency::remember(
        &format!("charge:{}:{}", user.id, key),
        Duration::from_secs(24 * 3600),
        || async {
            let charge = StripeClient::charge(&form).await?;
            Ok(ChargeResponse {
                id: charge.id,
                amount: charge.amount,
                status: charge.status,
            })
        },
    )
    .await?;

    match outcome {
        Replay::Fresh(body) | Replay::Replayed(body) => {
            let json = serde_json::to_value(&body)
                .map_err(|e| FrameworkError::internal(format!("serialize: {e}")))?;
            Ok(HttpResponse::json(json))
        }
        Replay::FreshUnfenced(body) => {
            // クライアントへのレスポンスは同じだが、メトリクスに残す価値がある: 排他性が
            // 本体全体を通じて保持されなかった。
            tracing::warn!("idempotent body completed unfenced");
            let json = serde_json::to_value(&body)
                .map_err(|e| FrameworkError::internal(format!("serialize: {e}")))?;
            Ok(HttpResponse::json(json))
        }
        Replay::InProgress => Ok(HttpResponse::text("retry")
            .status(409)
            .header("Retry-After", "1")),
    }
}
```

`Fresh` と `Replayed` が、クライアント向けのレスポンスにおいて同一に扱われていることに注目してください - `remember` の要点はまさに、2番目の呼び出し元が、自分が本体を実行した側だったのか、それとも記録された結果を受け取った側だったのかを見分けられないという点にあります。

`InProgress` は、考える価値のあるケースです: 最初の呼び出し元の本体がまだ実行中の間に重複が到着したため、まだ返すべき記録された結果がありません。`Retry-After: 1` ヘッダーを伴う `409 Conflict` が、正典となる答えです - クライアントは短く待ってからリトライし、2回目の試行は、`Cache::get` のショートサーキットで元の呼び出しと競争するか、`Replayed` に行き当たるかのどちらかになります。

## キーの素材

3つのメソッドはすべて、キーとして任意の `&str` を受け付けます。キャッシュバックエンドに触れる前に、キーはSHA-256で64文字の16進ダイジェストへハッシュ化されます。これにより、次の3つが得られます:

1. **バックエンドのキー長が有界であること。** 10 KBの `Idempotency-Key` ヘッダーをPOSTするクライアントであっても、生成されるのは64バイトのキャッシュキーです。
2. **生の識別子がキャッシュツールへ漏れないこと。** キーがメールアドレス、セッションid、あるいは内部のユーザーidを含んでいても、それらは `redis-cli KEYS idem:*` には現れません。
3. **文字クラスの衝突がないこと。** キャッシュバックエンドが特別に解釈するもの（コロン、globの文字、制御バイト）は、すでに取り除かれています - ハッシュは16進数のみです。

ハッシュの対象は、キャッシュキーのプレフィックスではなく、ユーザーが指定したキーです - 同じプロセス内の2つの異なる呼び出し箇所からの `Idempotency::once("k", …)` と `Idempotency::once("k", …)` は、意図的に衝突します。それを望まないなら、自分自身でキーを名前空間分けしてください:

```rust
Idempotency::once(
    &format!("billing:charge:{}:{}", tenant_id, client_key),
    Duration::from_secs(86_400),
    || async { /* … */ },
)
.await?;
```

## リースの更新 - 遅い本体の問題

素朴なロック + TTLの組み合わせには、ウィンドウのバグがあります: 本体がTTLより長く実行されると、本体がまだ実行中であるにもかかわらずロックが期限切れになり、2番目の呼び出し元が新しいロックを取得して、本体を並行して再び実行できてしまいます。重複排除の契約は、まさにそれを必要とするほど遅い操作に対して破綻するのです。

Suprnovaはこれを、本体の実行時間全体にわたって、TTLの3分の1ごとに（下限50 msで）ロックをリフレッシュするバックグラウンドタスクを起動することで解決します。`biased` 順序を伴う `tokio::select!` は、本体の分岐だけが決してfutureを解決することを保証します。

リフレッシュの*エラー*は、リースが失われたものとしては扱われません。それが意味するのは、バックエンドに問い合わせできなかったということであり、誰か他の人がロックを奪ったということではありません。そのため、更新は次の間隔でリトライし、何回か連続で失敗したあとにだけ諦めます。最初の一時的な不調で諦めていたら、バックエンドが数ミリ秒後に回復したとしても、リースは必ず失効してしまっていたでしょう。

### 排他性が失われたとき

それでも、更新は本当に失敗することがあります: ロックが期限切れになり、誰か他の人がそれを取得したために、トークンが一致しなくなるのです。その瞬間、2つの呼び出し元が同じ本体を実行している可能性があります。

本体は**キャンセルされません**。リースが失われた時点で、すでにカードへの請求やメッセージの送信が済んでいるかもしれず、キャンセルしてしまうと、それを記録するものが何もないまま中途半端な状態で取り残してしまいます。本体は完了まで実行され、喪失は報告されます:

| 結果 | 意味 |
|---|---|
| `Fresh(v)` / `Replay::Fresh(v)` | 本体が実行され、排他性が最後まで保持された |
| `FreshUnfenced(v)` | 本体が実行されて `v` を生成したが、別の呼び出し元が並行して実行していた可能性がある |

`FreshUnfenced` が `Fresh` 上のフラグではなく別のバリアントになっているのは、まさに、網羅的な `match` がうっかりそれを見逃せないようにするためです。それをどう扱うかはあなた次第です - 整合を取る、アラートを出す、埋め合わせをする、など。しかし、それを `Fresh` として扱ってしまうと、保証が保たれなかったことを知らせてくれる唯一のシグナルを捨ててしまうことになります。

リースを失うには、バックエンドが複数回分のリフレッシュ間隔にわたって到達不能であるか、TTLより長いストップ・ザ・ワールドの一時停止が必要です。稀です。不可能ではありませんし、かつては目に見えないものでした。

実務上の結論はこうです: TTLは、本体の最悪実行時間ではなく、あなたの重複排除ウィンドウ（`重複したリクエストは、どれくらいの期間重複排除されるべきか?`）に基づいて選んでください。1分のTTLを持つ30分の本体は問題ありません - ロックは、本体の実行中に約90回リフレッシュされます。

これを検証するテスト: 500 msブロックする本体を持つ200 msのTTLで、2番目の呼び出し元が400 msの時点で到着します。更新がなければ、2番目の呼び出し元は本体を再実行してしまうでしょう。更新があれば、`Duplicate` を見ます。ロックは持ちこたえます。

## 共有バックエンド

プロセスをまたぐ重複排除には、プロセスをまたぐキャッシュが必要です。インメモリのバックエンドは、プロセスごとの `HashMap` にロックを保持するため、同じマシン上の2つの `cargo run` インスタンスは、互いのべき等キーを見ることができません。これらのいずれかが問題になる本番のデプロイ - 複数のアプリプロセス、水平スケーリング、トラフィックのウィンドウが重なるブルーグリーンデプロイ - では、`CACHE_DRIVER=redis` を設定し、到達可能な `REDIS_URL` を用意しなければなりません。

bootstrapはフェイルクローズです: `CACHE_DRIVER=redis` でRedisに到達できない場合、アプリは、プロセスごとのメモリへ黙って格下げするのではなく、起動を拒否します。キャッシュバックエンドの契約全体については、[cache.md](cache.md)を参照してください。

## エラー処理

本体の `FrameworkError` は、`Idempotency` を通じて変わらずに伝播します。ロック取得の失敗（リクエストの途中でRedisがダウンする、バックエンドがエラーを返す）は、キャッシュ層からの `FrameworkError` として伝播します - サイレントなフォールバックはありません。エラー型はフレームワーク標準の `FrameworkError` であるため、ハンドラはそれを `?` でコントローラーのエラーコンバータへ通せます:

```rust
use std::time::Duration;
use suprnova::{handler, FrameworkError, HttpResponse, Idempotency, Replay, Response};

#[handler]
pub async fn handler(order_id: i64) -> Response {
    let outcome: Replay<MyDto> = Idempotency::remember(
        &format!("order:{order_id}"),
        Duration::from_secs(60),
        || async move {
            let row = MyRow::find(order_id)
                .await?
                .ok_or_else(|| FrameworkError::not_found("missing"))?;
            Ok(MyDto::from(row))
        },
    )
    .await?;

    match outcome {
        Replay::Fresh(dto) | Replay::Replayed(dto) | Replay::FreshUnfenced(dto) => {
            let json = serde_json::to_value(&dto)
                .map_err(|e| FrameworkError::internal(format!("serialize: {e}")))?;
            Ok(HttpResponse::json(json))
        }
        Replay::InProgress => Ok(HttpResponse::text("retry")
            .status(409)
            .header("Retry-After", "1")),
    }
}
```

`commit_on_success` や `remember` の `Err` 経路における解放の失敗は、**ログに記録されるだけで、決して返されません** - その経路で呼び出し元が目にする唯一のエラーは、本体のエラーです。解放の失敗は、ロックがTTLの失効まで保持され続けることを意味します。ウィンドウ内でのリトライは、それまでの間 `Duplicate` または `InProgress` を目にします。ログにはハッシュ化されたキーが含まれます（生のキー素材は含まれません）。そのため、運用者はPIIを漏らすことなく突き合わせができます。

## キャンセル

呼び出し元が、本体が完了する前に `Idempotency::remember` のfutureをドロップした場合、本体は他の `tokio::select!` の分岐と同じようにキャンセルされます - ロックは解放**されず**、TTLが失効する前に到着した重複は `InProgress` を見ます（そしてTTLのあとは、再び `Fresh` を見ます）。これが安全なデフォルトです: 効果がわからない中途半端な本体は、リトライしても安全だと決めつけるべきではありません。本体をキャンセル不能にする必要がある場合は、管理されていない副作用を持つ本体を `tokio::spawn` で包み、ハンドルをjoinしてください。

## キュー統合

キュー層は、`Queue::push_unique` を実装するために、内部で `Idempotency::commit_on_success` を使います。`Job::unique_id(&self)` ごとに、`Job::unique_for()` のウィンドウあたり最大1回だけジョブがエンキューされてほしい場合、あなた自身で `Idempotency::*` を呼び出す必要はありません:

```rust
use suprnova::{Job, Queue};

let was_pushed = Queue::push_unique(SendReceipt { order_id: 42 }).await?;
if was_pushed {
    // 競争に勝った。ジョブはキューに乗っている。
} else {
    // 別の呼び出し元がすでにこれをエンキュー済み。成功として扱う。
}
```

ジョブの一意性の契約全体については、[queues.md](queues.md)を参照してください。

## 支払いWebhookの受信

支払いのwebhookハンドラは `Idempotency::*` を使い*ません*。webhookの受信には、より厳格な要件があります - すべてのイベントは、最初の配信であっても監査可能でなければなりません。そのため、監査行が正となる情報源であり、重複排除キーはデータベースの `UNIQUE(provider, provider_event_id)` 制約です。`Idempotency::remember` はレスポンスのペイロードをキャッシュに保存しますが、webhookハンドラは、*イベントエンベロープ全体と処理結果*を `payments_webhook_events` に保存します。これは、運用者がそのテーブルを読むことで、オフラインでイベントをリプレイしたり再処理したりできることを意味します。

この2つのパターンは互いを補い合います。TTLでスコープされた重複排除を伴う、クライアント起因のキーには `Idempotency::*` を使ってください。キャッシュのTTLを超えて監査可能性を必要とする、プロバイダー起因のwebhook受信には、`UNIQUE` でインデックスされた監査テーブルを使ってください。webhookの契約については、[payments.md](payments.md)を参照してください。

### Suprnovaが異なる設計を選んだ理由

Laravelの `Cache::lock` はプリミティブです。Stripeスタイルのべき等性の契約（結果を記録し、それをリプレイし、進行中と重複を区別する）は、ユーザーランドのレシピとして残されています。それを必要とするあらゆるLaravelプロジェクトは、同じロックとキャッシュの一連の手順を書く羽目になり、たいていは次の3つのバグのうちどれかを抱えます:

1. **リースの更新がない。** TTLより長生きする本体は、重複した呼び出し元の中で並行して再実行されてしまいます。ロックはそこにありました。ただ、間違ったタイミングで期限切れになっただけです。
2. **成功経路での解放。** 本体が成功したときにロックを解放すると、`body() -> Ok` と、次の呼び出し元が新しいロックを取得するまでの間にウィンドウが開いてしまいます - まさに、重複排除が閉じるはずだったそのウィンドウです。
3. **キャッシュバックエンドの生のキー。** クライアントが指定した `Idempotency-Key` ヘッダーがそのままRedisのキーへ入り、運用ツールへPIIを漏らし、無制限のキーサイズを生み出してしまいます。

Suprnovaは、このレシピをファーストクラスのプリミティブとして出荷します。そのため、すべての呼び出し元が、同じリースの更新、同じフェイルクローズの解放セマンティクス、同じハッシュ化されたキーの安全性を得ます。3つのメソッド（`once`、`commit_on_success`、`remember`）は、あなたが実際に選ばなければならない3つのポリシーに名前を付けたものです - あなたの本体の失敗モデルに合うものを選んで、先へ進んでください。

## テスト

`Idempotency` は、コンテナを通じて自身の `CacheStore` を解決します。そのため、`InMemoryCache` を束縛するテストは、テストごとに新しく隔離されたキャッシュを得ます:

```rust
use std::sync::Arc;
use std::time::Duration;
use suprnova::cache::InMemoryCache;
use suprnova::cache::store::CacheStore;
use suprnova::container::testing::TestContainer;
use suprnova::idempotency::{Idempotency, Replay};

#[tokio::test]
async fn duplicate_remember_replays_the_first_result() {
    let _guard = TestContainer::fake();
    let store: Arc<dyn CacheStore> = Arc::new(InMemoryCache::with_prefix("idem:"));
    TestContainer::bind::<dyn CacheStore>(store);

    let r1: Replay<i32> = Idempotency::remember(
        "k",
        Duration::from_secs(60),
        || async { Ok(7) },
    )
    .await
    .unwrap();
    assert_eq!(r1, Replay::Fresh(7));

    let r2: Replay<i32> = Idempotency::remember(
        "k",
        Duration::from_secs(60),
        || async { Ok(999) },
    )
    .await
    .unwrap();
    assert_eq!(r2, Replay::Replayed(7));
}
```

フレームワーク自身の `framework/tests/idempotency.rs` は、契約の表面をカバーしています: 重複の抑制、TTLの期限切れ、エラー対成功の解放ポリシー、TTLより長生きする本体の実行時間にまたがるリースの更新、`InProgress` のレース、そしてキャッシュの `release_lock` 自体がエラーになるケースです。頼りにできる正確な振る舞いを見たいなら、それらのテストを読んでください。

## 落とし穴

- **`Idempotency::once` は、エラー時にもウィンドウを消費します。** 失敗した最初の呼び出し元も、TTLが失効するまでロックを保持し続けます。ウィンドウ内でのリトライが欲しいなら `commit_on_success` を使ってください。
- **`Idempotency::remember` は `T` をキャッシュバックエンドに保存します。** キーはハッシュ化されますが、*ペイロード*はserdeでシリアライズされてバックエンドへ書き込まれます。あなたのキャッシュストアに現れてはならないシークレットを、リプレイされる値に入れないでください。
- **2つのプロセスには共有キャッシュが必要です。** インメモリの重複排除は、プロセスごとです。プロセスをまたぐ正しさには、`CACHE_DRIVER=redis`（あるいは他のクロスプロセスなストア）が必要です。
- **150 ms未満のTTLは、リースのテスト対象外です。** 更新の下限は50 msであるため、100 msのTTLはおよそ50 msごとにリフレッシュされます - 契約としては問題ありませんが、フレームワークのリースのテストは `ttl >= 1s` で実行されます。現実的な重複排除のウィンドウを使ってください。ミリ秒単位で計測されるべき等性のウィンドウは、たいてい、この契約がふさわしい道具ではないことを意味します。
- **本体のキャンセルは、ロックを解放しません。** キャンセルされた本体は、TTLが失効するまでロックを保持したままにします。これはフェイルクローズの選択です。キャンセルが、重複した呼び出し元に見えてほしいものと一致するよう、タイムアウトを調整してください。

## 次のステップ

- [cache.md](cache.md) - 基盤となるロックのプリミティブと `CACHE_DRIVER` の選択。
- [queues.md](queues.md) - `Queue::push_unique` が、ジョブレベルの重複排除のために `Idempotency::commit_on_success` の上にどう構築されているか。
- [payments.md](payments.md) - キーでキャッシュされた重複排除の代わりにデータベース行のべき等性を使うwebhookの受信、そしてどちらへ手を伸ばすべきとき。
- [rate-limiting.md](rate-limiting.md) - スライディングウィンドウの強制のために、同じ `Cache` バックエンドを使う、隣接するミドルウェア。
- [middleware.md](middleware.md) - べき等キーの抽出を、あなたのPOST/PUTルートに対する再利用可能なミドルウェアへ切り出す方法。
