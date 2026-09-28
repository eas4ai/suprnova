# コンテキスト

`Context` は、Suprnovaのリクエストごとのキー/値バッグです。同じリクエスト内のあらゆる下流の呼び出し元に見えてほしいデータ - リクエストID、テナントのスラッグ、ユーザーのロール、監査証跡など - を、あらゆる関数シグネチャに値を通すことなく、しまっておく場所です。これは、Laravelの `Context` ファサードに相当する、Suprnovaの仕組みです。

```rust
use suprnova::Context;

Context::add("tenant_id", "acme");
Context::push("breadcrumbs", "checkout/start");
Context::hidden_add("api_key", secret);

let tenant: Option<String> = Context::get("tenant_id");
let page: Option<String> = Context::query_param("page");
```

次のようなときに使ってください。

- ログ行、キューに入れたジョブ、あるいはブロードキャストメッセージが、リクエストにスコープされたメタデータ（テナントID、相関ID、ユーザーのロール）を必要とするとき
- 深くネストしたヘルパーが、ハンドラがすでに持っている値を必要としているが、呼び出しチェーンのあらゆる層にパラメータを持ち回りたくないとき
- ハンドラではないコードから、現在のリクエストのクエリ文字列（`?page=3`、`?cursor=…`）を読み取りたいとき

`Context` は、リクエストをまたぐ状態のためのものでは**ありません**。現在のTokioタスクに束縛されており、リクエストが終わると消えます。リクエストより長生きするものには、[サービス コンテナ](container.md)か[キャッシュ](cache.md)を使ってください。

## 2つのバッグ

アクティブな `Context` スコープはそれぞれ、2つのキー/値マップと、1つの追加スロットを運びます。

| バッグ | 読み取り方法 | `Context::all()` に現れるか |
|---|---|---|
| **可視** | `Context::get` | はい |
| **隠し** | `Context::hidden_get` | いいえ |
| **クエリ** | `Context::query_param` | いいえ（URLの `?key=value` ペアの別スナップショット） |

可視と隠しの分割こそが、2つのバッグを持つことの意味そのものです - `Context::all()` を構造化出力へダンプするログのシリアライザーは、あなたが意図的に隠したデータを漏らすことがありません。監査メタデータは可視バッグへ、APIキー、OAuthのベアラートークン、そしてログに残したくないPIIは隠しバッグへ入れてください。

クエリバッグは、フレームワークのリクエストミドルウェアによって、URLのクエリ文字列から自動的に埋められます（下記の[ページネーションはクエリパラメータを読み取る](#ページネーションはクエリパラメータを読み取る)を参照）。通常は、これを読むだけで、書き込むことはありません。

## アクティブなスコープ

`Context` スコープは、あらゆる受信HTTPリクエストに対して、フレームワークによってインストールされます。ハンドラ、ミドルウェア、モデルオブザーバー、イベントリスナー、あるいはリクエストタスクから到達できるその他のあらゆる場所の内側では、スコープは有効であり、`Context::*` の読み書きは何の儀式もなく機能します。

スコープの外側 - 起動初期のコード、コンテキストを継承しない素の `tokio::spawn`、スコープをインストールしないユニットテストなど - では、あらゆる変更操作が**サイレントなno-op**になり、あらゆる読み取りが `None` を返します。契約は次のとおりです。どこから呼び出そうとも、決してパニックしません。

```rust
// ハンドラの中 - スコープはアクティブで、すべてが機能します:
Context::add("user_id", 42i64);
let id: Option<i64> = Context::get("user_id");
assert_eq!(id, Some(42));

// スコープの外側 - サイレントなno-op + None:
Context::add("user_id", 42i64);            // 捨てられます
let id: Option<i64> = Context::get("user_id");
assert_eq!(id, None);
```

パニックしないという契約は、意図的なものです。`Context` に触れるライブラリコード（カスタムのログサブスクライバー、SDKの拡張など）は、自分がリクエストの内側で動いているのか起動時なのかを知る必要がないはずです - ただ `Context::get` を呼び出し、`None` を「今は利用できない」として扱えばよいのです。

### サイレントな操作の可観測性

本当にサイレントなno-opだと、バグが隠れてしまいます（ミドルウェアの順序違い、spawnされたタスクへコンテキストが伝播していない、うっかり起動時に読み取ってしまった、など）。フレームワークの変更操作は、パニックしないという性質を保ちつつも、値を捨てるたびに `suprnova::context` ターゲットへ `tracing::trace!` イベントを発します。

```text
TRACE suprnova::context: Context mutation discarded: no active scope on this task op="add"
TRACE suprnova::context: Context mutation discarded: value failed to serialize op="push" key="bad"
TRACE suprnova::context: Context read returned None: value present but did not deserialize op="get" key="user_id" expected="String"
```

3つのクラスのイベントがあります。

| イベント | いつ発火するか |
|---|---|
| `mutation discarded: no active scope` | `add`、`push`、`hidden_add`、`forget` がスコープの外側で呼び出された |
| `mutation discarded: value failed to serialize` | `add`/`push`/`hidden_add` の値の `Serialize` 実装がエラーになった |
| `read returned None: value present but did not deserialize` | `get`/`hidden_get` はキーを見つけたが、保存されているJSONが要求された `T` と一致しなかった |

単なる不在 - 一度も設定されなかったキーへの `get` - はサイレントのままです。「これは設定されているか？」という探りがログを溢れさせないようにするためです。伝播のバグを疑うときは、`RUST_LOG=suprnova::context=trace` を有効にしてください - 本番コードの振る舞いを変えることなく、サイレントなno-opの経路が見えるようになります。

## 値を追加する

### `Context::add` - キーの値を置き換える

```rust
use suprnova::Context;

Context::add("user_id", 42i64);
Context::add("tenant", "acme");
Context::add("plan", PlanTier::Pro);     // 任意の Serialize な値
```

キーは `Into<String>` です。値は任意の `Serialize` 型です。値は書き込み時に一度だけ `serde_json::Value` へ変換され、その形で保存されます。同じキーへの以後の `add` は、値を置き換えます。

### `Context::push` - スタックへ追加する

```rust
Context::push("trail", "home");
Context::push("trail", "settings");
Context::push("trail", "billing");

let trail: Vec<String> = Context::get("trail").unwrap();
assert_eq!(trail, vec!["home", "settings", "billing"]);
```

`push` は、最初の呼び出しで空の配列を初期化し、それ以降の呼び出しでは追加していきます。すでにそのキーにスカラー値が存在する場合は、`[scalar, new_value]` という配列へ変換されます - `push` は、同じキーに対する以前の `add` に寛容です。

### `Context::hidden_add` - 隠しバッグへ書き込む

```rust
Context::hidden_add("api_key", os_env_secret);
Context::hidden_add("oauth_bearer", token);

// 可視バッグのダンプ（例えばJSONのログエミッタ）には、これらは見えません:
let all = Context::all();
assert!(!all.contains_key("api_key"));

// ですが、意図的に読み取ることはできます:
let key: Option<String> = Context::hidden_get("api_key");
```

隠しバッグは、可視バッグとは独立してキー管理されます - `hidden_add("user_id", 99)` と `add("user_id", "alice")` は、衝突することなく共存します。`Context::forget(key)` は、1回の呼び出しで両方のバッグから削除します。

## 値を読み取る

### `Context::get` - 可視バッグからの型付きの読み取り

```rust
use suprnova::Context;

let user_id: Option<i64>       = Context::get("user_id");
let tenant:  Option<String>    = Context::get("tenant");
let trail:   Option<Vec<String>> = Context::get("trail");
```

`get` は `T: DeserializeOwned` に対してジェネリックです。保存されているJSONの値は、読み取りのたびにデシリアライズされます。次の場合に `None` を返します。

- キーが設定されていない
- 現在のタスクでスコープがアクティブでない
- 保存されている値が `T` にデシリアライズできない（例えば `i64` を保存したのに `String` を求めた）

最後のケースは `tracing::trace!` を発するため、型違いのバグを観測できます - `Context::get` が、実際には「値の形が間違っている」だけなのに「値が設定されていない」ように見えてしまうのは、それを指し示すログ行がなければ、見つけるのに1時間かかってしまう類のバグです。

### `Context::hidden_get` - 隠しバッグからの型付きの読み取り

`get` と同じ形で、隠しバッグを読み取ります。型違いのときのtracingの挙動も同じです。

### `Context::has` - 可視バッグの存在チェック

```rust
if Context::has("user_id") {
    // …
}
```

`has` は可視バッグのみをチェックします（隠しバッグを調べる必要がある場合は `hidden_get(...).is_some()` を使ってください）。

### `Context::all` - 可視バッグのスナップショット

```rust
let snapshot: HashMap<String, serde_json::Value> = Context::all();
```

スコープの外側では、空の `HashMap` を返します。これは、リクエストにスコープされたフィールドをあらゆるログ行へ注入するために、JSONのログエミッタが呼び出すべきものです - そして、隠しバッグが別に存在する理由でもあります。

### `Context::forget` - 両方のバッグからキーを削除する

```rust
Context::forget("trail");          // 可視と隠しの両方から削除します
```

両方のバッグから削除するのは、意図的な設計です。関連するデータを両方のバッグに保存していた場合（例えば `user_id` を可視バッグに、`user_email` を隠しバッグに）、1回の `forget` で両方をきれいにできます。

## クエリパラメータを読み取る

`Context::query_param` は、リクエストの入り口で捕捉された、URLの `?key=value` ペアから読み取ります。リクエストミドルウェアは、クエリ文字列を一度だけスコープのクエリバッグへパースし、その後は、あらゆる下流の呼び出し元が、再パースすることなく名前で個々のパラメータを読み取れます。

```rust
use suprnova::Context;

let page: Option<String>   = Context::query_param("page");
let cursor: Option<String> = Context::query_param("cursor");
let sort: Option<String>   = Context::query_param("sort");
```

パラメータが欠けているか、スコープがアクティブでない場合は `None` を返します。重複したキーは、Laravelの「後勝ち」セマンティクスに従います - リクエストのパース済みクエリマップから得られるのと、同じ値です。

### ページネーションはクエリパラメータを読み取る

これこそが、クエリバッグが存在する理由です。Eloquentのページネーターは、`?page=` と `?cursor=` を `Context::query_param` から直接読み取るため、ページネーターを返すハンドラは、ページ番号を手動で配線する必要がありません。

```rust
use suprnova::{json_response, Request, Response};
use crate::models::Post;

pub async fn index(_req: Request) -> Response {
    // Context::query_param を介して、リクエストのURLから ?page=N を読み取ります
    // - req.query() の定型コードも、パラメータの受け渡しも必要ありません。
    let posts = Post::query()
        .order_by_desc("created_at")
        .paginate(15)
        .await?;

    json_response!(posts)
}
```

3つのページネーターのエントリポイントが、これを使います。

- `Builder::paginate(per_page)` - `?page=` を読み取ります
- `Builder::simple_paginate(per_page)` - `?page=` を読み取ります
- `Builder::cursor_paginate(per_page)` - `?cursor=` を読み取ります

表面全体については、[ページネーション](pagination.md)を参照してください。

## spawnされたタスクへ伝播させる

`tokio::spawn` は、まっさらなタスクローカル環境で子タスクを開始します - 親の `Context` スコープは流れ込ん**でいきません**。リクエストの中の素の `tokio::spawn` は、空の `Context` を見ることになり、あらゆる読み取りは `None` を返します。

スコープをspawnへ運ぶには、`Context::current()` でスナップショットを取り、子タスクの内側で `Context::scope` を使って再度入ってください。

```rust
use suprnova::context::Context;

// リクエストハンドラの中:
if let Some(store) = Context::current() {
    tokio::spawn(Context::scope(store, async move {
        // これで `Context::get`、`Context::query_param` などは、
        // 親リクエストのバッグを見られます。
        let request_id: Option<String> = Context::get("_request_id");
        do_background_work(request_id).await;
    }));
}
```

`Context::current()` が返すストアは、親の背後にあるマップを `Arc` 経由で共有します - 子が複製を保持している限り、子からの書き込みは親からも見えます。これはまさに、監査やロギングのspawnが望むものです - 子は追加のキー（`Context::add("audit.completed", true)`）を刻印でき、親の最後のログ行はそれを見ることができます。

分離されたスナップショットが必要な場合（子の書き込みを漏らしたくない場合）は、新しい `ContextStore` を組み立て、必要なキーだけをコピーしてください。

### 素の `spawn` が伝播しない理由

Tokioのタスクローカル（`tokio::task_local!`）は、意図的にタスクスコープです。spawnをまたいで自動的に継承するということは、次を意味してしまいます。

- 長生きするバックグラウンドタスクが、親のコンテキストマップを永遠に固定してしまう
- 子タスクでのパニックが、親の状態をポイズニングしてしまいうる
- ランタイムが、あらゆるタスクローカルの読み取りのたびに、親へのポインタ連鎖をたどらなければならなくなる

`Context::current()` + `Context::scope` という明示的な一手間は、伝播を隠れたデフォルトではなく、意図的な決定にします。

## テスト

`#[tokio::test]` や `#[suprnova_test]` の中では、デフォルトでは `Context` スコープはインストールされません。`Context` に触れるほとんどのコードは、「スコープなし」のケースを優雅に扱います（サイレントなno-op + `None` の読み取り）。そのため、普通のユニットテストには、何のセットアップも必要ありません。

テストに助けが必要になる状況が、2つあります。

### テスト対象のコードが `query_param` を呼び出す場合

ページネーションのヘルパーは、`Context::query_param` を介して `?page=` を読み取ります。「3ページ目は正しいオフセットを返す」というユニットテストには、`query_param` が `Some("3")` を返す必要があります。2つの方法があります。

**`test_query_guard`（推奨）:**

```rust
use suprnova::Context;

#[tokio::test]
async fn paginate_reads_page_from_query() {
    let _q = Context::test_query_guard("page", "3");

    // テスト対象のコードは、これで ?page=3 を見ます
    assert_eq!(Context::query_param("page"), Some("3".into()));

    let posts = Post::query().paginate(15).await?;
    assert_eq!(posts.current_page(), 3);
}
// `_q` はスコープの終わりでドロップされます - スレッドローカルのオーバーライドは消去されます。
```

`test_query_guard` は、RAIIガードを返します。テスト本体がパニックしたとしても、`Drop` が実行され、OSのスレッドが再利用される前に、スレッドローカルのオーバーライドをクリアします。このガードは `#[must_use]` です - `_` へ束縛すると即座にクリアされてしまい、それはほとんどの場合、あなたが望むものではありません。

**素の `test_set_query` + `test_clear_query`:**

```rust
#[tokio::test]
async fn manual_pair() {
    Context::test_clear_query();        // 兄弟テストからの漏れを消去する
    Context::test_set_query("page", "5");

    // … アサーション …

    Context::test_clear_query();
}
```

ガードの形を使ってください。手動のペアは、複数のオーバーライドを独立して設定・解除する必要があるケースのために存在しますが、`#[must_use]` のガードのほうが誤用しにくくなっています。

どちらのAPIも、`#[cfg(any(test, feature = "testing"))]` によってゲートされています - テストバイナリと、統合テストハーネス用に `testing` フィーチャーをオプトインしたリリースビルドにのみコンパイルされます。普通のリリースビルドには存在しません。

### テスト対象のコードが `Context` スコープから読み書きする場合

`Context::scope` を介して、明示的にインストールしてください。

```rust
use suprnova::context::{Context, ContextStore};

#[tokio::test]
async fn handler_reads_tenant_id() {
    Context::scope(ContextStore::default(), async {
        Context::add("tenant_id", "acme");

        let resolved = my_helper_that_reads_tenant().await;
        assert_eq!(resolved, "acme");
    })
    .await;
}
```

あるいは、スコープの作成時にクエリバッグをシードすることもできます。

```rust
use std::collections::HashMap;
use suprnova::context::{Context, ContextStore};

#[tokio::test]
async fn handler_reads_query_from_scope() {
    let mut q = HashMap::new();
    q.insert("page".into(), "3".into());
    q.insert("sort".into(), "name".into());

    Context::scope(ContextStore::with_query(q), async {
        assert_eq!(Context::query_param("page"), Some("3".into()));
        assert_eq!(Context::query_param("sort"), Some("name".into()));
    })
    .await;
}
```

`ContextStore::with_query(HashMap)` は、リクエストミドルウェアが使うのと同じコンストラクタです。そのため、本番と同じコードパスを通すテストは、同じ形のクエリバッグを見ることになります。

### スレッドローカルのオーバーライドが存在する理由

クエリパラメータのオーバーライドは、タスクローカルではなく `thread_local!` です。これは意図的なものです - テストが、**あらゆるアサーションを `Context::scope` の呼び出しでラップすることなく**、クエリパラメータをインストールできるようにするためです。その組み合わせは、次のとおりです。

1. 読み取りは、まずスレッドローカルのオーバーライドをチェックします
2. オーバーライドがなければ、タスクローカルの `CONTEXT` スコープのクエリバッグを読みます
3. スコープすらなければ、`None` を返します

スレッドローカルの検索は、本番環境では事実上コストがかかりません（テストビルドの外では、オーバーライドは常に空です）。そして、テストの作者を、ページネーション関連のあらゆるアサーションの周りに定型的な `Context::scope(...)` ラッパーを書く手間から解放します。

## よくあるパターン

### あらゆるログにリクエストIDを刻印する

フレームワークはすでにこれを行っています。リクエストミドルウェアが `_request_id` を可視バッグへシードするため、下流のジョブ、ブロードキャスト、そして `Context::all()` のログダンプは、名前でこのIDを読み取れます。同じミドルウェアは、IDをスパンフィールドとして運ぶ `tracing` スパンも開きます - これが、リクエストの内側で発せられるあらゆるログ行にIDが現れる理由です。サブスクライバー側については[ロギング](logging.md)を参照してください。値を文字列として必要とするとき（例えば、送信するHTTPリクエストへ相関ヘッダーとして配線するとき）は、`Context` からIDを読み取るのが正しい経路です。

```rust
let request_id: Option<String> = Context::get("_request_id");
```

### キューに入れたジョブへテナントのコンテキストを運ぶ

`Context` は、キューのシリアライズ/デシリアライズの境界を越えて自動的には伝播しません - ワーカーはディスパッチャーとは別のプロセスで、しばしば別のマシンで動いています。必要なものは、ジョブのペイロードに渡してください。

```rust
use suprnova::{Context, FrameworkError, Queue};

// ハンドラの中:
let tenant_id: String = Context::get("tenant_id")
    .ok_or_else(|| FrameworkError::param("tenant_id missing"))?;

Queue::push(SendInvoice { tenant_id, invoice_id }).await?;
```

ワーカーが `SendInvoice` を処理するときは、`Job::handle` の先頭で新しい `Context` スコープをインストールし、ジョブのペイロードから必要なキーを再びシードしてください - 本体を `Context::scope(ContextStore::default(), async { ... })` でラップする形です。そうすれば、ジョブが呼び出すあらゆるロギングや深くネストしたヘルパーは、リクエストの内側にいるときと同じテナントIDを見ることになります。

ここは、`hidden_add` が真価を発揮する場所でもあります - ジョブはスコープの入り口で一度だけAPIキーを取得して保管でき、ジョブの内側にあるあらゆる下流のHTTP呼び出しは、再取得することなく `Context::hidden_get` を介してそれを読み取れます。`Job` トレイトの形については、[キュー](queues.md)を参照してください。

### リクエスト全体にわたる監査証跡

```rust
Context::push("audit.steps", "validated_input");
// … 他の処理 …
Context::push("audit.steps", "charged_card");
// … 他の処理 …
Context::push("audit.steps", "sent_receipt");

// レスポンス時のミドルウェアで:
let steps: Vec<String> = Context::get("audit.steps").unwrap_or_default();
tracing::info!(?steps, "request audit trail");
```

ハンドラの後に実行されるレスポンス時のミドルウェアは、リクエストログのあちこちに散らばった、各ステップの個別のデバッグ行の代わりに、監査証跡を1つのログ行にまとめてダンプできます。

### SDK拡張の認証情報のための隠しバッグ

```rust
// リクエストの入り口で、認証の後に:
Context::hidden_add("sdk.api_key", load_api_key_for(user_id));

// SDK呼び出しの奥深くで:
let key = Context::hidden_get::<String>("sdk.api_key")
    .ok_or_else(|| FrameworkError::param("api key not stashed"))?;
```

`Context::all()` をダンプするログには、このキーは現れません。隠しバッグは、ハンドラがログの表面にさらすことなく呼び出しスタックの奥深くへ渡す必要がある、あらゆる認証情報にとって正しい置き場所です。

## Suprnovaが異なる設計を選んだ理由

Laravelの `Context` ファサード（Laravel 11で導入されました）がインスピレーションの元です - 同じメソッド名、同じ可視/隠しの分割、同じ「リクエストの外側ではサイレント」という契約です。Rustのランタイムに由来する違いが、2つあります。

**非同期の伝播は、魔法ではなく明示的です。** Laravelの `Context` がキューに入れたジョブを自動的に流れていくのは、Laravelがディスパッチ時にコンテキストバッグをジョブのペイロードへシリアライズするからです。Rustの非同期モデルには、スレッドローカルが流れ込む単一の「現在のリクエスト」というものがありません - `tokio::spawn` はまっさらな状態で始まり、キューの境界はプロセスをまたいだシリアライズを伴います。Suprnovaは伝播のためのプリミティブ（`Context::current()` + `Context::scope`）を公開し、タスクが実際には継承していないコンテキストを継承しているふりをする代わりに、その境界で明示的にオプトインできるようにしています。

**型違いの読み取りは、観測可能です。** 別の型で保存された値への `get::<T>` は、Laravelでは黙って `None` を返します（PHPなので、そもそも書き込み時に型が強制されていません）。Suprnovaでは、この読み取りが `tracing::trace!` を発します。型違いのケースは、本物のバグを示しているからです - 値はどこかに書き込まれているが、あなたが読んでいる型とは違う、というだけなのです。このトレースがあれば、パニックしないという契約を変えることなく、計装された実行の中でそれを見つけられます。

3つ目の相違は、機構的なものです。Suprnovaの `Context` は `tokio::task_local!` の上に構築されているため、その寿命はグローバルな状態ではなく、Tokioのタスクに束縛されています。スレッドをまたいだ読み取りが見るのは、**そのスレッドで現在実行されているタスク**のスコープであり、最後にインストールされたスコープではありません。これによって、同じ `Context` ファサードを、スレッドプール、アクター、あるいは `spawn_blocking` の本体から呼び出しても安全になります - ただし、スコープをspawnへ伝播させている場合に限ります。

## 実装場所

| トピック | ファイル |
|---|---|
| `Context` ファサード + `ContextStore` | `framework/src/context/mod.rs` |
| HTTPリクエストでのスコープのインストール | `framework/src/logging/request_id.rs` |
| `Context::query_param` の呼び出し元（ページネーション） | `framework/src/eloquent/builder.rs` |
| 再エクスポート | `framework/src/lib.rs`(`pub use context::{Context, ContextStore}`) |

## 次のステップ

- [リクエスト ライフサイクル](lifecycle.md) - あらゆるリクエストで `Context` スコープがインストールされる場所
- [サービス コンテナ](container.md) - 単一のタスクより長生きする、リクエストをまたぐ状態のために
- [ロギング](logging.md) - `Context::all()` が構造化ログ行にどう現れるか
- [ページネーション](pagination.md) - `Context::query_param` の主な下流の読み取り手
- [テスト](testing.md) - ユニットテストのための `test_query_guard` と `Context::scope` のパターン
