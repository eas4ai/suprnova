# キャッシュ

Suprnovaは、2つのドライバー - インメモリまたはRedis - のどちらかに支えられた、Laravel形の `Cache` ファサードを出荷します。ドライバーは、`CACHE_DRIVER` を通じて起動時に明示的に選ばれます。ファサードは `CacheStore` トレイトの上に被さる薄い層であるため、独自のバックエンドも組み込みのものと同じやり方で差し込めます。

## ファサード

```rust
use suprnova::Cache;
use std::time::Duration;

Cache::put("user:1", &user, Some(Duration::from_secs(3600))).await?;

let cached: Option<User> = Cache::get("user:1").await?;

if Cache::has("user:1").await? {
    // ヒット
}

Cache::forget("user:1").await?;
```

あらゆるメソッドは、ファサードの境界で `serde_json` を通じてシリアライズされるため、`T: Serialize + DeserializeOwned` であればどんな型でも往復できます。ファサードの下にあるトレイト（`CacheStore`）が目にするのは、不透明なJSON文字列だけです。

## ブートストラップ

キャッシュは、`Server::run()` のドライバーブートストラップの手順の中でバインドされます（[リクエスト ライフサイクル](lifecycle.md)を参照）。`Cache::bootstrap` は、設定済みの `CacheConfig`（あるいは環境変数から構築したもの）を読み取り、`CacheConfig::driver` に応じてディスパッチします:

- `Memory` - 設定されたプレフィックスとデフォルトTTLを持つ `InMemoryCache` をバインドします。常に成功します。
- `Redis` - `REDIS_URL` に接続し、結果として得られる `RedisCache` をバインドします。URLに到達できない場合は**フェイルクローズします**。メモリへの無言のダウングレードはありません。

ワーカー（`queue:work`、`schedule:run`、`workflow:work`）も同じブートストラップを経由するため、`Cache::get` を使うジョブは、HTTPハンドラと同じバックエンドを目にします。

### Suprnovaが異なる設計を選んだ理由

Laravelの `cache.php` 設定はデフォルトのストアを選びますが、一部のコードパスでは、設定を誤ったバックエンドが失敗すると、Laravelは無言で `array`（プロセス内）へ切り替えてしまいます。これは `php artisan tinker` にとっては生産的なデフォルトですが、本番環境ではフットガンになります - たった1回のRedisのミスが、アプリ内のあらゆるタグの消去とロック取得の保証を、無言のうちに変えてしまうのです。

Suprnovaは、その正反対のデフォルトを選びます。`CACHE_DRIVER=memory` は明示的なもので（`cargo run` のデフォルトでもあります）、`CACHE_DRIVER=redis` を到達不能なRedisに対して使うと、`Server::from_config` からエラーが返ります。バイナリは対処方法のメッセージとともに非ゼロで終了し、supervisord/systemdは、半端に動くアプリではなく、起動失敗を目にすることになります。

## 設定

| 環境変数 | 意味 | デフォルト |
|---|---|---|
| `CACHE_DRIVER` | `memory` または `redis` | `memory` |
| `REDIS_URL` | Redis URL（`driver=redis` のときだけ参照される） | `redis://127.0.0.1:6379` |
| `REDIS_PREFIX` | ストアへのあらゆる操作に適用されるキーのプレフィックス | `suprnova_cache:` |
| `CACHE_DEFAULT_TTL` | `Cache::put(None)` のデフォルトTTL（秒単位）。`0` はデフォルトなしを意味する | `3600` |

`CACHE_DRIVER` が未設定であれば `Memory` としてパースされます。`memory`/`in-memory`/`inmemory`/`redis` のいずれでもない値（大文字小文字を区別せず、トリムした上で判定されます）は、起動時にエラーを返します。

環境変数のパースを望まない場合は、設定をプログラムから組み立てることもできます:

```rust
use suprnova::{Config, CacheConfig, cache::CacheDriver};

Config::register(
    CacheConfig::builder()
        .driver(CacheDriver::Redis)
        .url("redis://cache.internal:6379")
        .prefix("myapp:")
        .default_ttl(7200)
        .build(),
);
```

`CacheConfigBuilder::build` は決定的です - 未設定のフィールドは、環境変数を読み直すのではなく、`CacheConfig::default()` にフォールバックします。

### `forever` の契約は、バックエンドをまたいで保たれる

`Cache::forever` と `Cache::remember_forever` は `CACHE_DEFAULT_TTL` を完全にバイパスします。設定されたデフォルトにかかわらず、値は決して期限切れになりません。`Cache::put(key, value, None)` はデフォルトを適用します - デフォルトを持つことの意味はそこにあります。

デフォルトTTLの解決は、ファサード層で行われます。どちらの `CacheStore` バックエンドも、ストアの境界では `None` を文字どおりに扱います（有効期限なし）。だからこそ `forever` は、メモリでもRedisでも、本当に永遠を意味するのです。

## 読み取り、書き込み、削除

```rust
use suprnova::Cache;
use std::time::Duration;

// 明示的なTTLを指定して書き込む
Cache::put("session:42", &session, Some(Duration::from_secs(1800))).await?;

// 永遠に書き込む - CACHE_DEFAULT_TTLをバイパスする
Cache::forever("config:features", &features).await?;

// 読み取る（ミス、または期限切れならNone）
let session: Option<Session> = Cache::get("session:42").await?;

// 存在確認 - trueは、存在していて期限切れでないことを意味する
if Cache::has("session:42").await? { /* … */ }

// Laravel流の否定形
if Cache::missing("session:42").await? { /* ウォームする */ }

// 1回の呼び出しで読み取りと削除を行う
let one_shot: Option<String> = Cache::pull("notice:welcome:42").await?;

// キーが存在して削除された場合はtrueを返す
Cache::forget("session:42").await?;

// すべて消去する（両方のバックエンドでプレフィックスの範囲に限られる）
Cache::flush().await?;
```

`Cache::pull` はアトミックでは**ありません** - `get` の後に `forget` が続くだけであり、Laravelの `Repository::pull` と同じ形です。アトミックな取り出しには `Cache::lock` を使ってください（下記参照）。

### 書き換えずにTTLを更新する

```rust
let refreshed = Cache::touch("session:42", Duration::from_secs(1800)).await?;
```

`touch` は、キーが存在してTTLが延長された場合は `true` を、そうでなければ `false` を返します。保存されている値自体には触れません。

## Add - 存在しなければ書き込む（アトミック）

```rust
let won = Cache::add(
    "daily:winner",
    &user_id,
    Some(Duration::from_secs(86_400)),
).await?;
if won {
    send_winner_email(user_id).await?;
}
```

`Cache::add` は、キーが空である（あるいは期限切れである）場合にだけ書き込みます。書き込めれば `true`、競合すれば `false` を返します。組み込みの両バックエンドで**アトミック**です:

- `InMemoryCache` は、存在確認と挿入をまたいで書き込みロックを保持します
- `RedisCache` は `SET key value NX EX ttl`（または `EX` なしの `NX`）を使います

`add_raw` をオーバーライドしない独自の `CacheStore` 実装は、非アトミックなcheck-then-putへフォールバックします。これは、ネイティブな `add` を持たないストアに対するLaravelの `Repository::add` のフォールバックと同じです。

## Remember - 取得または計算する

```rust
let user = Cache::remember(
    "user:1",
    Some(Duration::from_secs(3600)),
    || async { User::find(1).await },
).await?;

let cfg = Cache::remember_forever("config:app", || async {
    load_config_from_db().await
}).await?;
```

`remember` は、ミスしたときにだけあなたのクロージャを呼び出し、その結果をストアに保存します。クロージャは `Result<T, FrameworkError>` を返すため、ドメインの失敗はキャッシュをポイズニングするのではなく、`?` を通じて伝播します。

`Cache::sear(key, default)` は、`remember_forever` のLaravel流のエイリアスです。本体もセマンティクスも同じです - 移行してきたコードが同じように読めるよう、両方の名前で出荷されています。

### Rememberはスタンピードに対して安全ではない

`remember` は、非アトミックな `get` と `put` のペアです。同じコールドなキーに対してN個の並行したミスが起きれば、クロージャがN回実行され、N個の結果が書き込まれます。これはLaravelの `Repository::remember` と正確に一致しており、よくあるケース（クロージャがべき等で、書き込みが同一である場合）では問題になりません。

問題になるのは、次のような場合です:

- クロージャのコストが高い場合（計算に1秒以上かかる、あるいは遅い上流にアクセスする）
- キーが十分に人気で、コールドキャッシュのイベントが背後のストアへ一斉にN件のリクエストを送ってしまう場合
- クロージャが、値を計算する以上の副作用を持つ場合

そのような場合は、`Cache::lock` でラップしてください:

```rust
use suprnova::Cache;
use std::time::Duration;

let key = "rebuild:user:1";

if let Some(guard) = Cache::lock(key, Duration::from_secs(10)).await? {
    let user = Cache::remember(
        "user:1",
        Some(Duration::from_secs(3600)),
        || async { User::find(1).await },
    ).await?;
    guard.release().await?;
    return Ok(user);
}

// 競争に負けた - 勝者が計算中である。勝者が書き込んだものを読み取るか、
// 陳腐化した値にフォールバックする。
let user = Cache::get::<User>("user:1").await?
    .ok_or_else(|| FrameworkError::internal("cache miss after losing rebuild lock"))?;
```

## ロック

`Cache::lock` は、所有権トークンを保持する `LockGuard` を返します。ロックはアドバイザリー（強制力のない）であり、Redisに支えられている場合はプロセスをまたぎます。

```rust
use suprnova::Cache;
use std::time::Duration;

if let Some(guard) = Cache::lock("job:42", Duration::from_secs(30)).await? {
    do_exclusive_work().await?;
    guard.release().await?;
}
// Some(guard) は、自分たちが所有していることを意味する。Noneは、別の保持者が先に取得したことを意味する。
```

ガードが公開するものです:

| メソッド | 用途 |
|---|---|
| `guard.token()` | 所有権トークンを読み取る（Rust側の名前） |
| `guard.owner()` | 同じ値を返す、Laravel流のエイリアス |
| `guard.refresh(ttl)` | TTLを延長する - もはやロックを所有していなければ `false` を返す |
| `guard.release()` | まだロックを所有していれば解放する - トークンがもう一致しなければ `false` を返す |

**`Drop` による自動解放は、意図的にありません。** Redisのロックは、プロセスの境界をまたいで確認されなければなりません。ドロップ時の自動解放は、（誤って）横取りされたロックを無言で取り戻してしまうか、あるいは（さらに悪いことに）解放の失敗をデストラクタのパニックの中に隠してしまうかのどちらかになります。解放が明示的であるからこそ、エラーが伝播します。

`refresh` を使えば、長時間実行されるジョブが自分自身のロックを延長し、自業自得のタイムアウトを避けられます - ツリー内での利用例については[べき等性](idempotency.md)を参照してください。

## アトミックカウンター

```rust
// 存在しなければ0で初期化してから増分する。新しい値を返す。
let visits = Cache::increment("page:visits", 1).await?;

// マイナス方向のステップも同じ形
let remaining = Cache::decrement("quota:remaining", 1).await?;

// 任意の増分量
let total = Cache::increment("stats:downloads", 10).await?;
```

組み込みの両バックエンドでアトミックです: `InMemoryCache` は書き込みロックされた `HashMap::entry` を使い、`RedisCache` は `INCRBY`/`DECRBY` を使います。保存される値はJSONエンコードされた整数なので、`Cache::get::<i64>("page:visits")` は同じキーで往復します。

## タグ付きキャッシュ

タグを使えば、1回の呼び出しで、関連するエントリのファミリー全体を無効化できます。典型的なユースケースは、リソースが変化したときに一緒に消去しなければならない、リソースごとのキャッシュです。

```rust
use suprnova::Cache;
use std::time::Duration;

// 1つ以上のタグの下に保存する
Cache::tags_put(
    &["users", "user:1"],
    "user:1:profile",
    &profile,
    Some(Duration::from_secs(3600)),
).await?;

Cache::tags_put(
    &["users", "user:1"],
    "user:1:posts",
    &posts,
    Some(Duration::from_secs(600)),
).await?;

// 更新経路: `user:1` というタグが付いたキーをすべて消去する
Cache::flush_tags(&["user:1"]).await?;
```

タグの所属は**エントリ単位**です: タグ付きの書き込みはそれぞれ、その書き込みのタグ集合をエントリの正とする情報としてインストールし、以前のタグを置き換えます。知っておく価値のある帰結が2つあります:

- 以前タグが付いていたキーに対する、タグなしの `Cache::put` は、そのエントリのタグを**消去します**。その後に古いタグへ `flush_tags` をかけても、生きている、タグなしの値は削除されません。
- `tags_put(&["a"], …)` を `tags_put(&["b"], …)` で上書きすると、そのエントリは `flush_tags(&["b"])` にだけ応答するようになります。

陳腐化した前方インデックスの参照は、消去の走査の間、そして `flush()` の際に取り除かれます。そのため、書き込まれても決して消去されないタグについて、無限に積み上がっていくことはありません。

## 2つのバックエンド

| 特徴 | `InMemoryCache` | `RedisCache` |
|---|---|---|
| プロセス間で共有 | いいえ | はい |
| 永続化 | いいえ | Redisがそのように設定されていればはい |
| アトミックな `add` | はい（書き込みロック） | はい（`SET NX`） |
| アトミックな `increment`/`decrement` | はい（書き込みロック） | はい（`INCRBY`/`DECRBY`） |
| タグ付きキャッシュ | はい | はい |
| ロック | はい | はい（プロセス間） |
| サブ秒のTTL | はい（`tokio::time::Instant`） | はい（`PX`/`PEXPIRE`） |
| 選択方法 | `CACHE_DRIVER=memory`（デフォルト） | `CACHE_DRIVER=redis` |

Databaseキャッシュドライバーはありません - フレームワークが出荷しているのは、上記の2つのバックエンドだけです。独自のバックエンドは `CacheStore` を実装し、コンテナへ直接バインドできます - 下のテスト注入のパターンを参照してください。

### インメモリの有効期限

`InMemoryCache` は、期限切れのエントリを**読み取り時に遅延して**追い出します: `get_raw`、`has`、`add_raw` は、あるエントリが期限切れであると初めて観測した時点で、それを取り除きます。再びアクセスされるキーには、死骸が積み上がることはありません。

カーディナリティの高い短命なキー集合を書き込むだけで、二度と読み返さないワークロードには、そのようなトリガーがありません。その場合は、定期タスクから `InMemoryCache::purge_expired()` を呼んでください - 削除されたエントリの件数を返します。Redisは自身の期限切れをサーバー側で処理するため、そちら側では同等のものは必要ありません。

### Redis TTLの精度

あらゆるRedisのTTLは、`EX` / `EXPIRE` ではなく `PX` / `PEXPIRE` を経由します。これにより、2つの落とし穴を避けられます:

- サブ秒の `Duration` は、`EX` の下では `0秒` に切り詰められてしまいます。Redisはこれを拒否する（`SET … EX 0`）か、さらに悪いことに「キーを削除する」（`EXPIRE key 0`）と解釈します。
- `Duration::ZERO` は、呼び出しの前に1ミリ秒へクランプされるため、どちらの拒否経路もユーザーコードから到達することはありません。

## テスト

`InMemoryCache` を `TestContainer` にバインドすれば、ファサードは他のストアと同じようにそれを解決します:

```rust
use std::sync::Arc;
use suprnova::{Cache, CacheStore, InMemoryCache};
use suprnova::container::testing::TestContainer;

#[tokio::test]
async fn cache_round_trips() {
    let _guard = TestContainer::fake();
    TestContainer::bind::<dyn CacheStore>(Arc::new(InMemoryCache::new()));

    Cache::put("k", &"v", None).await.unwrap();

    let v: Option<String> = Cache::get("k").await.unwrap();
    assert_eq!(v.as_deref(), Some("v"));
}
```

`TestContainer::bind` はスレッドローカルなスコープへ書き込むため、並列に実行されるテストは、互いにキャッシュの状態を漏らし合いません。3層のルックアップモデルについては、[サービス コンテナ](container.md)の章を参照してください。

## パターン

名指す価値のある、いくつかの繰り返し現れる形です:

```rust
// 階層的な、コロン区切りのキー - Laravelが使うのと同じ規約
Cache::put("users:1:profile", &profile, None).await?;
Cache::put("posts:123:comments:count", &count, None).await?;

// データの変動しやすさに応じたTTL
Cache::put("stats:active", &count, Some(Duration::from_secs(60))).await?;
Cache::put("config:features", &features, Some(Duration::from_secs(3600))).await?;
Cache::forever("translations:en", &translations).await?;

// 書き込みの周りでの、タグによるキャッシュ無効化
async fn update_user(id: i64, data: UserUpdate) -> Result<User, FrameworkError> {
    let user = User::update(id, data).await?;
    Cache::flush_tags(&[&format!("user:{}", id)]).await?;
    Ok(user)
}
```

## 次のステップ

- [設定](configuration.md) - `Config::register` と環境変数がどのように組み合わさるか
- [レート リミット](rate-limiting.md) - Laravel形の `RateLimiter`
  ファサードは `Cache` の上に構築されている
- [べき等性](idempotency.md) - リクエストの重複排除ミドルウェアは、
  `Cache::lock` をエンドツーエンドで使っている
- [サービス コンテナ](container.md) - `CacheStore` がどのようにバインドされ、解決されるか
- [エラー モデル](error-model.md) - リクエストの途中でRedisに到達できないとき、
  `Cache::*` が何を返すか
