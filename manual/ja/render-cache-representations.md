# RenderCache の表現

キャッシュ対象のルートが保存するのは「ページ」ではありません。保存するのは
**表現**です。1 つの具体的なレスポンスが、1 つのルックアップキーのもとに、1 つ
以上のストレージ層へ、条件付きリクエストに答えるのに足りるだけの、そしてあとから
それがまだ最新だと証明するのに足りるだけのメタデータを添えて保存されます。
2 人の訪問者が同じ保存済みバイト列を受け取るのは、両者が導出したキーが同じキー
であるときだけです。そしてそのキーは、ルートが宣言した内容から導出されます。
ハンドラーがたまたま何をしたかから導出されることは決してありません。

この章は、その保存されるものについてです。表現が取りうる形（`Complete` と
`Composite`）、キーに入るもの、書き込まれる層、配信されたヒットが運ぶ `ETag`、
`Cache-Control`、`Vary`、`Age`、`Warning`、表現が取りうる 4 つの鮮度状態、
`If-None-Match` と `HEAD` への答え方、そして `PrivateCached` と
`PublicShellStitched` が実際に何を保存するのかを扱います。表現が新鮮な帯から
出ていく*理由*（書き込み、エポックの前進）は次章の主題です。ここでは、その帯が
存在すること、そして 1 つの表現がそのいずれかの中にいることがわかれば十分です。
以下の例はすべて、このリポジトリのドッグフードアプリケーション
（`app/src/live/mod.rs`）のルートであり、`app/tests/live_render_cache.rs` の
名前付きテストによって証明されています。

## エントリの 2 つの形

保存されるエントリは、2 つの種類のいずれかです。

- **`Complete`** は完成した答えです。ステータス、再生可能なヘッダーの集合、
  そして 1 つの本文バッファからなります。これを配信しても、何もコピーせず、
  何も実行しません。`PublicShared` と `PrivateCached` のルートは、すべてこの形を
  保存します。
- **`Composite`** は、型付きの穴が開けられた共有**シェル**と、それぞれの穴に何が
  戻るのかを述べるセグメントグラフです。この形を保存するのは
  `RepresentationClass::PublicShellStitched` だけであり、これを生み出すのは Live
  ドキュメントだけです。

ポリシーで宣言したクラスが、そもそもどちらの形に到達できるのかを決めます。
`/live/public` と `/live/todos` はどちらも `PublicShared` を宣言しています。
`the_database_profile_serves_a_hit_through_the_sql_stores` は `/live/todos` の
公開済みエントリをストアから読み戻し、それが `EntryKind::Complete` であることを
表明します。`the_public_document_is_a_hit_whose_seed_still_promotes` は
`/live/public` のそれを `RenderCache::inspect_route_for_test` を通じて読み戻し、
どのクラスのもとで保存されたのかを表明します。これが重要なのは、「保存された」と
「黙って却下された」が同じレスポンスを生むからです。主張はエントリに対して
行わなければならず、訪問者に見えているものに対して行ってはなりません。

## ルックアップキー

リクエストが導出するキーは、ルートパターン、そのパスパラメータ、ポリシーが宣言
したクエリパラメータ、宣言された各バリエーションディメンションの解決済みの値、
アプリケーションのビルド識別子（`APP_BUILD_ID`）、そして現在の権威エポックから
組み立てられます。それ以外は何も入りません。リクエストとともに届いたのに
`QueryPolicy::declared` が名前を挙げていないクエリパラメータは、キーから黙って
落とされるのではなく、そのリクエストについてキャッシュをバイパスします。落として
しまえば、それを送った相手に間違ったページを配信することになるからです。

キーは、運用担当者が手に持てるテキストです。
`the_operator_commands_inspect_without_a_body_and_advance_the_epoch` の中の
`RenderCache::key_for_route_for_test` は、それが `rk1.` で始まることを表明し、
`render-cache:inspect` はまさにそのテキストを受け取ります。

エポックがキーの一部であるため、エポックの前進は何かを探して削除する必要が
ありません。以前に保存されたエントリは、次のリクエストの時点で、通常の
ルックアップからは単に到達できなくなります。それが、
[運用](render-cache-operations.md)の章の緊急時の無効化が拠って立つ仕組みです。

## ポリシーがどの層に書き込むか

ストレージ層は 2 つあります。**L0** はプロセス内メモリで、
`RENDER_CACHE_L0_ENTRIES` と `RENDER_CACHE_L0_BYTES` によって上限が定められて
います。**L1** はデプロイプロファイルが設定するもの（ファイルのディレクトリ、
データベースのテーブル、または Redis）であり、そこを指しているすべての
プロセスで共有されます。

ポリシービルダーは、指示しない限り **L0 のみ**に保存します。
`StorageLayers::l0_only()` が既定です。共有階層に置く価値のあるルートは、
それを宣言します:

```rust
use suprnova::render_cache::{
    FreshnessPolicy, RenderCachePolicy, RepresentationClass, StorageLayers,
};

let router = router.try_render_cache(
    "/live/todos",
    RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .freshness(FreshnessPolicy::new(300_000, 60_000, 300_000)?)
        .layers(StorageLayers::l0_and_l1())
        .build()?,
)?;
```

これが `app/src/live/mod.rs` にある `/live/todos` の宣言です。このアプリケーション
の中で、そのバイト列をすべてのノードが共有できる唯一のドキュメントなので、
`l0_and_l1()` を宣言しているのもこれだけです。`RENDER_CACHE_L1_DIR` が
ディレクトリを指さない限り L1 が無効な組み込みプロファイルのもとでは、この層を
宣言しても何も変わりません。データベースプロファイルのもとでは、エントリは
`suprnova_render_entries` に着地し、2 つ目のプロセスがそこでそれを見つけます。

`the_database_profile_serves_a_hit_through_the_sql_stores` がその証明です。
これはデータベースプロファイルのプロバイダーでアプリケーションを起動し、
ミドルウェアが導出したまさにそのキーのもとで、公開済みエントリを L1 から直接
読み取り、それから L0 を空にしてもう一度尋ねます。すると 2 回目のリクエストも、
ハンドラーを走らせずに答えられます。プロセス内メモリからのヒットはクライアント側
からは同じに見えるので、このテストはストアに手を伸ばしているのです。

層はグローバルにではなく、ルートごとに選んでください。L1 はミス時に、L0 だけなら
払わずに済むラウンドトリップの費用がかかりますし、1 つのノードしか要求しない
エントリを、すべてのノードから見える場所に置く価値はありません。

## 配信されたヒットが運ぶメタデータ

配信された表現を説明するレスポンスフィールドは 5 つあり、それらが定義されるのは
ここです。ほかの章は、それらを繰り返し説明せずに使います。

| フィールド | 何を述べるか |
|---|---|
| `ETag` | 送られたバイト列そのものに対する強いバリデータ。クライアントはこれを `If-None-Match` として送り返せます。 |
| `Cache-Control` | 既定ではどのクラスでも `private` です。`SharedCachePolicy::SMaxAge` を設定した `PublicShared` ルートは `public` と `s-maxage` も得ます。共有プロキシがそのバイト列を保持するよう招かれる方法は、これだけです。アイランドを少なくとも 1 つ持つ、組み立て済みの `Composite` ドキュメントは `private, no-store` です。 |
| `Vary` | 宣言されたバリエーションディメンションのうち、リクエストヘッダーを含意するものから導出されます。`Locale` は `Accept-Language` を、`Media` は `Accept` を、`Encoding` は `Accept-Encoding` を含意します。何も含意しないディメンションは、何も加えません。名前は、ディメンションを宣言した順ではなく、ヘッダー名でソートして出力されます。 |
| `Age` | その表現が公開されてから経過した整数秒。これが存在すること自体が、レスポンスがストアから出てきたことのいちばん簡単なローカルの証拠です。 |
| `Warning` | `110 - "Response is Stale"`。新鮮な区間を過ぎて配信されたレスポンスにだけ付きます。 |

ディメンションからヘッダーへの対応づけは、
`crates/suprnova-live/src/render_cache/variance.rs` の
`VarianceDimension::vary_header` です。2 つのエンジンテストが、その `Locale` と
`Encoding` の半分ずつと、連結されたヘッダー値を証明しています。
`a_descriptor_orders_dimensions_and_bounds_values`
（`crates/suprnova-live/tests/render_cache_variance.rs`）は、両方を運ぶ
ディスクリプターが `["Accept-Encoding", "Accept-Language"]` を報告することを
表明し、`cache_control_and_vary_agree_with_class_variance_and_seed_deadline`
（`crates/suprnova-live/tests/render_cache_coherence.rs`）は、同じ組が
`Accept-Encoding, Accept-Language` を出力すること、そしてヘッダーを含意する
ディメンションを持たないディスクリプターは `Vary` をまったく出力しないことを
表明します。`Media` が `Accept` を含意することはコードから文書化されたもので、
ここにそれを組にするテストはありません。

レスポンスの値のうち 3 つは、動作中のアプリケーションに対して表明されています。
`the_public_document_is_a_hit_whose_seed_still_promotes` は `/live/public` から
`private, max-age=300` を読み取り、2 回目のリクエストに `Age` ヘッダーがあること
を要求します。`the_private_document_is_cached_per_principal_and_never_crosses`
は `/live/me` から `private, max-age=60` を読み取ります。
`the_dashboard_is_stitched_per_principal_from_one_shared_shell` は、組み立て済み
のダッシュボードから `private, no-store` を読み取ります。これは、コンポジット
レスポンダー以外の何も書かない値です。

## 4 つの鮮度状態

すべてのヒットは、何かが配信される前に、ちょうど 4 つの状態のいずれか 1 つに
解決されます。それらを設定するのが
`FreshnessPolicy::new(fresh_ms, stale_servable_ms, stale_on_error_ms)` です。
**2 つの古さのウィンドウは、どちらも新鮮な区間の終わりから測られるのであって、
一方の後ろにもう一方が積み重なるのではありません。**ここが人のつまずくところ
です:

| 状態 | 公開からの経過時間 | 訪問者が受け取るもの |
|---|---|---|
| 新鮮 | `fresh_ms` 未満 | 保存済みバイト列。`Warning` なし |
| 古くても配信可 | `fresh_ms` を超えているが、超過分が `stale_servable_ms` 未満 | 保存済みバイト列を即座に、`Warning` 付きで。リクエストの背後で境界付きの再構築が起動される |
| エラー時に配信 | `fresh_ms` の超過分が `stale_servable_ms` 以上、かつ `stale_on_error_ms` 未満 | フォアグラウンドでの再構築。その再構築自体が失敗したときにだけ、保存済みバイト列を `Warning` 付きで |
| 死亡 | `fresh_ms` の超過分が 2 つのウィンドウの大きいほう以上 | 何もなし。リクエストはレンダリングする |

`/live/todos` は `FreshnessPolicy::new(300_000, 60_000, 300_000)` を宣言して
いるので、5 分間は新鮮、6 分目は古くても配信可、10 分までがエラー時に配信、
そのあとは死亡です。

この帯を上書きする規則が 2 つあります。`PrivateCached` の表現は**決して**古い
まま配信されません。新鮮な区間を過ぎればそれは死亡です。`/live/me` が
`FreshnessPolicy::new(60_000, 0, 0)` を宣言しているのはそのためで、そこに古さの
帯があれば、このキャッシュが守らない約束として読まれてしまいます。そして、昇格
期限を過ぎた公開シードのドキュメントは、区間が何と言っていようと死亡です。期限を
過ぎたシードは二度と昇格できないからです。

`stale_service_is_marked_and_rebuilt_in_the_background` は、制御されたクロックの
上で `/live/todos` を最初の境界の向こうへ進め、配信された本文、
`Warning: 110 - "Response is Stale"`、そして `Age: 300` を表明します。表現が
新鮮な帯から早く出ていく*原因*（書き込み、エポックの前進）は
[RenderCache の世代](render-cache-generations.md)の主題です。

## 条件付きリクエストと HEAD

配信された `ETag` を `If-None-Match` として送り返したクライアントは、本文のない
`304` を受け取り、`HEAD` は本文なしでヘッダーを受け取ります。どちらもあなたの
ハンドラーには届きません:

```
GET  /live/todos                          -> 200, ETag: "..."
GET  /live/todos  If-None-Match: "..."    -> 304, empty body
HEAD /live/todos                          -> 200, same ETag, empty body
```

`conditional_and_head_requests_are_answered_from_the_stored_entry` は、動作中の
アプリケーションに対してこの 3 つすべてを表明します。あとの 2 つをまたいでも
レンダリング回数が動かないことも含めてです。

例外が 1 つあり、それは意図的なものです。`Composite` のレスポンスは決して `304`
を返しません。組み立てのたびに別個の表現になるからです。アイランドの識別子は
新しくなり、ドキュメントがブートストラップのノンスを持つならそれも新しくなり
ます。したがって `304` は、すでに手元にある本文を、このリクエストのために鋳造
されたヘッダーと組み合わせるようクライアントに告げてしまいます。組み立てられた
レスポンスの `ETag` は、送られたバイト列そのものに対する強いバリデータのままで、
ただ、あとのリクエストと一致することが決してないだけです。
`the_dashboard_is_stitched_per_principal_from_one_shared_shell` の手順 7 は、
配信されたバリデータをそのまま送り返し、異なる `ETag` を伴う `200` を表明します。

## 1 人のものである表現

`RepresentationClass::PrivateCached` は、サインイン済みの訪問者ごとに表現を
1 つ保存します。ポリシーが `Principal` または `Tenant` のバリエーションも宣言
していない限り、ビルド時に拒否されるため、この組み合わせが偶然ずれることは
ありません:

```rust
use suprnova::render_cache::{
    FreshnessPolicy, RenderCachePolicy, RepresentationClass, VarianceDimension,
};

let router = router.try_render_cache(
    "/live/me",
    RenderCachePolicy::builder(RepresentationClass::PrivateCached)
        .freshness(FreshnessPolicy::new(60_000, 0, 0)?)
        .vary(VarianceDimension::Principal)
        .build()?,
)?;
```

その背後にあるハンドラーは、ごく普通のものです。サインイン済みの訪問者を解決し、
その名前をレンダリングします:

```rust
pub async fn me(_request: Request) -> Response {
    let result: Result<HttpResponse, FrameworkError> = async {
        let user = Auth::user_as::<User>()
            .await?
            .ok_or_else(|| FrameworkError::internal("Live account document without a principal"))?;
        html(&MeView { name: user.name })
    }
    .await;
    result.map_err(failed)
}
```

これをキャッシュさせるために、追加で配線されているものは何もありません。この
ルートは、ダッシュボードと同じ `AuthMiddleware::redirect_to("/login")` を運ぶので、
匿名の訪問者はハンドラーが走る前にリダイレクトされ、プリンシパル自体は
レンダリングの内側で解決されます。サインイン済みの訪問者のアイデンティティを
セッションから読み取ることは、セッションの読み取りではなく**アイデンティティの
読み取り**として分類されるため、レンダリングは `PrivateCached` に狭まり、キーは
プリンシパルごとの不透明な素材を運び、両者は一致します。

`the_private_document_is_cached_per_principal_and_never_crosses` は 2 人の訪問者
でサインインし、それぞれ 2 回ずつヒットさせてレンダリングが起きないことを確かめ、
それぞれの本文が相手ではなく自分自身の名前を挙げていることを表明します。3 人目の
訪問者はレンダリングします。どちらとも何も共有していないからです。配信される
`Cache-Control` は `private, max-age=60` なので、共有プロキシにバイト列が
差し出されることは決してありません。同じテストは、この取り引きのもう半分も
示しています。レンダリングは `users` テーブルを読むプロバイダーを通じて
プリンシパルを解決するため、3 人目の訪問者をシードすると、保存済みの
`/live/me` エントリはすべて無効化され、それぞれの次のリクエストが再構築します。
それが、[世代](render-cache-generations.md)が説明するとおりのテーブル粒度の
無効化です。

この分類の帰結が 2 つあり、このクラスを宣言する前に知っておく価値があります:

- `Principal` バリエーションを持つ `PrivateCached` ルートへの**匿名**リクエスト
  は、`Anonymous` キーのもとでキャッシュされます。レンダリングはアイデンティティ
  を解決しなかったので、プリンシパル素材は観測されず、キーは `Anonymous` と
  述べ、両者は一致します。サインイン済みの訪問者は `Private` キーを導出するので、
  そのエントリに届くことは決してありません。これが当てはまるのは、そうした
  リクエストが実際に `200` をレンダリングするときですが、`/live/me` はそれを
  決してしません。ログインへのリダイレクトが `302` を返し、`302` はここに
  たどり着く前に適格性によって拒否されるからです。実際にそこへ届く
  フレームワークのテストは、`framework/tests/render_cache/middleware.rs` の
  `an_anonymous_render_resolving_identity_through_the_session_caches_anonymously`
  です。
- **名前付きガード**の識別子は、既定のガードのものとまったく同じようにプリンシパル
  素材です。それを読み取ると、プリンシパルの読み取りが記録され、id があるときは
  その値も記録されます。

そして、動いていない規則が 1 つあります。`Principal` バリエーションを宣言*せずに*
プリンシパルを読み取るルートは、保存を却下されます。そうしたエントリを訪問者ごとに
キー付けする方法がないため、共有されるのではなく、決して保存されないのです。
セッションのそれ以外の値は、依然として `Uncacheable` を強制します。
[RenderCache](render-cache.md) の分類の一覧を参照してください。

## 穴の開いたシェル

`RepresentationClass::PublicShellStitched` は、枠は誰にとっても同じでアイランド
はそうではない Live ドキュメントのためのものです。保存されるエントリが持つのは
シェルだけです。アイデンティティ結合アイランドのマークアップも、署名付き
スナップショットも、保存されたバイト列の中に入ることは決してありません。ヒットの
たびに、要求してきた相手のために、そのリクエストのために導出された権限のもとで、
すべてのアイランドが再マウントされます。

このリポジトリのダッシュボードが、そのルートです:

```rust
router.try_render_cache(
    "/live",
    RenderCachePolicy::builder(RepresentationClass::PublicShellStitched)
        .freshness(FreshnessPolicy::new(300_000, 60_000, 300_000)?)
        .build()?,
)
```

`the_dashboard_is_stitched_per_principal_from_one_shared_shell` は、それが何を
もたらし、何を代償にするのかを表明します。保存されるエントリは、
アイデンティティ結合アイランドごとに 1 つ、合計 3 つのスロットを持つ
`EntryKind::Composite` です。2 人目のプリンシパルもそのシェルから応答され、
2 つのドキュメントはアイランドのタグ**だけ**が異なります。このテストは、
それぞれから 3 つのアイランドタグを取り除き、残りをバイト単位で比較します。
ルート自身のログインリダイレクトは、ヒットのたびに変わらず走ります。ステッチ
されたヒットは、何かが配信される前にルートのミドルウェアチェーン全体を通して
転送されるため、匿名の訪問者は組み立て済みドキュメントではなく、リダイレクトを
受け取ります。

スロットを少なくとも 1 つ持つ組み立て済みのドキュメントには、
`Cache-Control: private, no-store` が付いて送られます。それは 1 人のプリンシパル
のアイランドを、1 つのリクエストのために導出し直された権限のもとで保持して
おり、`max-age` があれば、共有のブラウザープロファイルが次に座った相手にそれを
再生できてしまうからです。スロットが 0 個の `Composite` は、プリンシパルごとの
バイト列をまったく運ばず、リクエストごとのノンスだけを運ぶので、ほかの私的な
表現と同じように、このクラスの私的な `max-age` を保ちます。
`framework/tests/render_cache/stitch.rs` の
`a_zero_slot_composite_is_assembled_with_a_fresh_nonce_on_every_hit` が、それを
表明します。いずれにせよ、このクラスはポリシーのビルド時に
`SharedCachePolicy::SMaxAge` を拒否するので、共有プロキシにバイト列が差し出される
ことは決してありません。

知っておくべき制限が 2 つあります。このクラスに意味があるのは、チェーンの末尾が
Live の完了ミドルウェアであるルートの上だけなので、`LiveDocument::render` と
ともに使い、それ以外とは使わないでください。そして、ステッチされたエントリが
エラー時のフォールバックで配信されることは決してなく、バックグラウンドの再構築を
引き起こすこともありません。2 つ目が実際に何を意味するかは、
[世代](render-cache-generations.md)の章が述べます。

### Suprnovaが異なる設計を選んだ理由

Laravel には、サーバー側の表現モデルがそもそもありません。そのレスポンス
キャッシュのパッケージは、自分で組み立てたキー（たいていは URL、ときには URL に
ログイン済みユーザー用の手書きの接尾辞を足したもの）のもとにルートの
レンダリング出力を保存し、次のリクエストでそれを返します。保存されるものの形は
1 つきりで、それは常に完成した本文であり、2 人の訪問者がそれを共有するかどうかは
あなたが組み立てた文字列の性質です。

Suprnova は、キーを宣言にし、形をその帰結にします。あなたはクラスと
バリエーションディメンションを名指しし、フレームワークがキーを導出し、分割する
ディメンションのない `PrivateCached` を拒否し、レンダリングが実際に観測した
ものとキーが実際に述べたものを突き合わせ、両者が食い違うときはそのレンダリングの
保存を却下します。そして `PublicShellStitched` があるおかげで、95 パーセントが
共有で 5 パーセントが私的なページは、何もキャッシュしないことと、してはいけない
ものをキャッシュすることのどちらかを選ぶ必要がありません。共有部分は一度だけ
保存され、私的な部分はリクエストごとに再レンダリングされ、私的なバイト列が
ストアに入ることはありません。

## 次のステップ

- [RenderCache の世代](render-cache-generations.md) - 保存された表現は
  どのように最新でなくなり、そのあと何が起きるのか
- [RenderCache](render-cache.md) - ポリシーとバリエーションの宣言、そして
  レンダリングが決して保存されない理由
- [Live](live.md) - ステッチされたシェルが穴を空けている先のアイランド
