# レート リミット

Suprnovaは、互いを補い合う2つのレートリミットの表面を出荷します:

| 表面 | 使うとき... | バックエンド |
|---------|-------------|---------|
| `RateLimiterDriver` + `RateLimitMiddleware` | 任意のストレージ（Redis ZSET、インメモリのdeque）に対する、厳密なスライディングウィンドウの強制が欲しいとき | `dyn RateLimiterDriver` |
| `RateLimiter` + `ThrottleRequestsMiddleware` | Laravelの形をした名前付きリミッター、`attempt()` ワークフローのコールバック、あるいは `X-RateLimit-*` レスポンスヘッダーが欲しいとき | `Cache` ストア（メモリまたはRedis） |

スライディングウィンドウのドライバーは、Suprnovaにネイティブな形です - リクエストごとに1スロット、別建てのタイマーキーなし、Redis上ではLuaによるアトミックな評価です。Laravelファサードは、移行してきたアプリが手を伸ばすものであり、名前付きリミッター／レスポンスコールバックのパターンが求めるものです。この2つは設計上共存し、1つのルートが両方を重ねられます。

## スライディングウィンドウ ドライバーSPI

`RateLimiterDriver` は、スライディングウィンドウアルゴリズムのためのストレージSPIです。各キーは、ヒットしたタイムスタンプのdequeを追跡します。`try_acquire` のたびに、`now - window` より古いエントリが削除されます。残りの件数が `max_requests` を下回っていれば `now` が追加され、呼び出しは受理されます。そうでなければ拒否されます。

```rust
use std::sync::Arc;
use std::time::Duration;
use suprnova::rate_limit::memory::InMemoryRateLimiter;
use suprnova::rate_limit::{RateLimiterDriver, SlidingWindowConfig};

let limiter: Arc<dyn RateLimiterDriver> = Arc::new(InMemoryRateLimiter::new());
let cfg = SlidingWindowConfig {
    max_requests: 60,
    window: Duration::from_secs(60),
};
let ok = limiter.try_acquire("user:42", &cfg).await?;
if !ok {
    let wait = limiter.retry_after("user:42", &cfg).await?;
    // wait は、バケット内で最も古いスロットが失効するまでの
    // Option<Duration> です。
}
```

### 組み込みドライバー

| ドライバー | ストレージ | 選択方法 |
|--------|---------|--------------|
| `InMemoryRateLimiter` | プロセスごとの `HashMap<String, Bucket>`。`tokio::time::Instant` を使うため、`start_paused` テストが時計を駆動できます | `RATE_LIMIT_DRIVER=memory`（デフォルト） |
| `RedisRateLimiter` | Redis ZSET + Luaによるアトミックなcheck-and-record | `RATE_LIMIT_DRIVER=redis` + `RATE_LIMIT_REDIS_URL` |

`bootstrap_from_env()` は、一致するドライバーをコンテナへ配線します。本番環境の外では、未知のドライバー値は `warn!` ログと共にメモリへフォールバックします。

### 本番環境は、インメモリドライバーに対してフェイルクローズします

本番環境では、インメモリのリミッターに解決されることは、起動の失敗です:

```
refusing to boot in production: RATE_LIMIT_DRIVER is unset, which defaults
to the in-memory limiter. Per-process buckets mean every configured quota
is multiplied by your replica count and reset by every deploy...
```

インメモリドライバーは、1つのプロセスのヒープにバケットを保持します。Nレプリカの背後では、それぞれが自分自身のカウントを保持するため、「15分あたり5回の試行」というパスワードリセットのスロットルは、実質的に5Nになり、デプロイのたびにすべてがゼロへリセットされます。あなたが設定した上限は、あなたが実際に得る上限ではありません - そして、それを教えてくれるものは何もありません。リクエストは成功するからであり、それは外から見れば、正しく動いているスロットルそのものに見えるからです。それはエラーとしてではなく、クレデンシャルスタッフィングやアカウント列挙のインシデントとして表面化します。

**認識されない**ドライバー値も、同じ理由で失敗します: メモリへフォールバックするからです。`RATE_LIMIT_DRIVER=Redis` - 大文字始まり - は、そうでなければ起動時に一度警告するだけで、マルチレプリカのデプロイをプロセスごとのスロットリングのまま静かに放置してしまいます。それは、設定済みに見えるがゆえに、本番環境へ最も届きやすいケースです。

Redisを指すようにするか:

```env
RATE_LIMIT_DRIVER=redis
RATE_LIMIT_REDIS_URL=redis://cache.internal:6379
```

あるいは、本当に単一プロセスで実行しているなら、そう明言してください:

```env
RATE_LIMIT_ALLOW_MEMORY_IN_PRODUCTION=true
```

開発、テスト、そして**ステージング**は影響を受けません。ステージングが意図的にゲートされていないのは、メールガードと同じ理由によるものです: ハードに失敗させると、チームはオーバーライドをグローバルに設定するようになり、それはまさにチェックが重要になる場面で、そのチェックを無力化してしまいます。

### `RateLimitMiddleware`

ドライバーを包むHTTPラッパーです。リクエストごとのバケット選択を駆動するために、`key_fn` クロージャで構築します:

```rust
use std::sync::Arc;
use std::time::Duration;
use suprnova::container::App;
use suprnova::rate_limit::{
    BackendErrorPolicy, RateLimitMiddleware, RateLimiterDriver, SlidingWindowConfig,
};

let limiter: Arc<dyn RateLimiterDriver> =
    App::resolve_make::<dyn RateLimiterDriver>().unwrap();

let mw = RateLimitMiddleware::new(
    limiter,
    SlidingWindowConfig {
        max_requests: 100,
        window: Duration::from_secs(60),
    },
    |req| format!("route:{}", req.path()),
)
.on_backend_error(BackendErrorPolicy::FailClosed);
```

拒否時（クォータ超過時）は、`Retry-After` ヘッダーを伴うHTTP 429を返します。

### 呼び出し元単位だけでなく、受信者単位で制限する

アドレスをキーにした上限は、*1つのクライアントがリクエストを送りすぎていないか*には答えられます。しかし、*1つのメールボックスが溢れさせられていないか*には答えられません。ボットネット、プロキシプール、あるいは単一のIPv6 `/64` に分散した攻撃者は、1人の被害者に何千通ものパスワードリセットメールを送りつけながら、あらゆるIP単位の予算の下に留まり続けます - 使い果たされているリソースは受信箱であり、それらのリクエストが共有している唯一のものは被害者のアドレスです。逆もまた痛手です: キャリアグレードNATやオフィスのゲートウェイの背後では、IP単位の上限は、1人のメンバーの振る舞いのせいで、大勢を罰してしまいます。

`identity_key` は、*操作の対象になっている*アカウントでバケットをキー付けします:

```rust
use suprnova::rate_limit::{identity_key, names_identity};

let per_recipient = RateLimitMiddleware::new(
    limiter.clone(),
    SlidingWindowConfig { max_requests: 3, window: Duration::from_secs(900) },
    |req| identity_key(req, "email", "auth-issuance"),
)
.key_reads_body(4096)
.only_when(|req| names_identity(req, "email"))
.on_backend_error(BackendErrorPolicy::FailClosed);
```

どちらかで置き換えるのではなく、IP単位のリミッターと*並べて*重ねてください。それぞれが、もう一方にはできないことを捕まえます: IP単位は、1つのホストが多数のアドレスを列挙するのを止め、受信者単位は、多数のホストが1つのアドレスを狙うのを止めます。

3つの詳細が、セキュリティを支えています:

- **`key_reads_body`** は、キーが計算される前に（指定された上限まで）ボディをバッファリングします。そのため、クエリ文字列だけでなく、form-encodedなPOSTからもフィールドを読み取れます。バッファリングは、未認証の呼び出し元があなたにやらせられる作業であるため、これはオプトインです。上限がそれを制限します。上限を超えるボディは、キー付けされないまま通過させるのではなく413で拒否されます - そうしなければ、ボディにパディングを詰めることが、上限を逃れる手段になってしまいます。
- **`only_when`** は、誰も名指ししないリクエストに対してリミッターをスキップします。これがなければ、そうしたリクエストは `identity_key` のアドレスフォールバックに落ち、*この*リミッターのクォータに対してカウントされてしまいます - そして、受信者単位の予算は通常この2つのうちより厳しい方であるため、誰も名指ししないすべてのルートに対して、それが黙って拘束力のある上限になってしまいます。
- **値は正規化され、ハッシュ化されます。** `Alice@Example.com` と `alice@example.com` は同じメールボックスに届くため、バケットを共有しなければなりません。そうしなければ、大文字小文字を変えるだけで上限を回避できてしまいます。結果がハッシュ化されるのは、レートリミットのバックエンドが、プライマリのデータベースよりアクセス制御の弱い共有Redisであることが多いためであり、キーのダンプが、誰がパスワードをリセットしているかの一覧として読めてしまってはならないからです。

### バックエンドエラーのポリシー

`BackendErrorPolicy` は、リミッターの*バックエンド*自体がエラーになったとき - 例えばRedisに到達できないとき - に何が起きるかを支配します。これは、リクエストが正当にクォータを超過する場合とは別物です。バックエンドは判断を下せないため、ミドルウェアは可用性と上限の保証のどちらかを選ばなければなりません。

| ポリシー | 振る舞い | 使いどころ |
|--------|-----------|-------------|
| `FailOpen`（デフォルト） | リクエストを通過させる。`warn` でログ出力 | ほとんどの公開API - リミッターの障害がトラフィックを落とすべきではない場合 |
| `FailClosed` | HTTP 503 + `Retry-After: 1` で拒否する。`error` でログ出力 | センシティブなルート（ログイン、パスワードリセット、支払い）で、バックエンド障害時の無制限のトラフィックが、一時的な拒否より悪い場合 |

ミドルウェア上の `.on_backend_error(BackendErrorPolicy::FailClosed)` で選択します。クォータを使い果たしたリクエストは、ポリシーにかかわらず常に429です - ポリシーが影響するのは、バックエンドエラー時のフォールスルーだけです。

## Cacheベースの、Laravelの形をしたファサード

`RateLimiter`（構造体）は `Illuminate\Cache\RateLimiter` を反映しています。Suprnovaの[`Cache`](cache.md)ファサードの上に構築された、固定ウィンドウのカウンターです。名前付きリミッター、`attempt()` ワークフロー、あるいはLaravelアプリが期待する `X-RateLimit-*` ヘッダーが欲しいときはいつでも、これを使ってください。

### ストレージのレイアウト

減衰が `D` 秒である、試行回数カウンターのキー `K` について:

- `K` - `hit` のたびにインクリメントされるi64カウンターです。初期シードは（`Cache::add` を介して）0です。
- `K:timer` - ウィンドウが終わるときの、i64のunix-seconds-since-epochです。`Cache::add` を介して設定されるため、ウィンドウ内で最初の呼び出し元だけが期限を固定します。

両方のキーは同じTTLを運ぶため、ウィンドウが終わるとキャッシュが自動的にそれらを片付けます。カウンターが `max_attempts` に達しているが `:timer` が消えている場合、`too_many_attempts` はカウンターをリセットします - これが、クォータを使い果たした期間のあとにウィンドウを前へスライドさせているものです。

### カウンターAPI

```rust
use suprnova::RateLimiter;

// 試行を1回消費する。ウィンドウがなければシードする。
let n = RateLimiter::hit("login:1.2.3.4", 60).await?;

// 試行を1回消費し、かつ単一のアトミックなラウンドトリップで上限をテストする。
// このヒットがバケットを `max` 超過へ押し上げた場合は `true`（リクエストを
// 拒否する）、受理された場合は `false` を返す。別々の
// `too_many_attempts` + `hit` のペアの代わりにこれを使うこと: チェックしてから
// ヒットするという2回の呼び出しでは、並行するリクエストが上限をすり抜けて
// しまう（check-then-actレース）。
// maxとしての `i64::MAX` は「無制限」を意味する - 常に受理するが、カウントはする。
let over_limit = RateLimiter::hit_and_check("login:1.2.3.4", 5, 60).await?;
if over_limit { /* 429を返す */ }

// Nだけインクリメントする。「コスト加重」の上限（各リクエストが
// 1回より多くの試行を消費する）に便利。
let n = RateLimiter::increment("api:user:1", 60, 5).await?;

// 現在のカウントを読み取る（一度もヒットしていないか期限切れなら0）。
let attempts = RateLimiter::attempts("login:1.2.3.4").await?;

// ウィンドウが再び開くまでの秒数（ウィンドウが開いていなければ0）。
let secs = RateLimiter::available_in("login:1.2.3.4").await?;

// 上限に達するまでの残り試行回数。
let remaining = RateLimiter::remaining("login:1.2.3.4", 5).await?;
// retries_left は、remaining のLaravel流の綴りのエイリアス。
let remaining = RateLimiter::retries_left("login:1.2.3.4", 5).await?;

// バケットは、今この瞬間（ウィンドウがまだ開いた状態で）上限を超えているか?
let over = RateLimiter::too_many_attempts("login:1.2.3.4", 5).await?;

// カウンターだけを落とす（タイマーは残る - ウィンドウはまだ固定されている）。
RateLimiter::reset_attempts("login:1.2.3.4").await?;

// カウンターとタイマーの両方を落とす。
RateLimiter::clear("login:1.2.3.4").await?;
```

### `attempt()` ワークフロー

バケットがクォータの範囲内にあるときだけコールバックを実行します。ヒットは、コールバックが実行されたときだけ消費されます:

```rust
let result = RateLimiter::attempt(
    "login:1.2.3.4",
    5,
    || async { do_login_work().await },
    60,
).await?;
match result {
    Some(value) => { /* コールバックが実行され、試行がカウントされた */ }
    None => { /* 上限超過、コールバックは実行されなかった */ }
}
```

これは、ログインフォームにとって正しい形です - 作業が実際にコールバックへ到達しない限り、試行を消費しません。

### 名前付きリミッター

起動時に登録し、リクエスト時に解決します。Laravel側の名前 `for` はRustの予約キーワードであるため、Rust側の主たる名前は `define` です。Laravelそのままのエイリアスは `r#for` 経由で公開されています。

```rust
use suprnova::{Limit, RateLimiter};

// 起動時 - `define` がRust側の主たる名前。
RateLimiter::define("api", |req| {
    // 生の `X-Forwarded-For` ヘッダーではなく `req.ip()` - 下記参照。
    let key = req.ip().unwrap_or_else(|| "anon".into());
    Limit::per_minute(60).by(format!("ip:{key}")).into()
});

// Laravel側のエイリアス - キーワードエスケープの綴りでの同じもの。
RateLimiter::r#for("uploads", |_req| Limit::per_hour(100).into());

// 解決する。
let cb = RateLimiter::limiter("api").unwrap();
let limit_result = cb(&request);
```

名前付きリミッターのコールバックは [`LimitResult`] を返します。これは、次のものから構築できます:

- 単一の `Limit` - この上限を適用します。
- `Vec<Limit>` - すべての上限を適用します。最初に達したものが勝ちます。
- `HttpResponse` - このレスポンスで即座にショートサーキットします（`Limit::none()` を介した「管理者は無制限アクセスを得る」や、リクエストを完全に拒否するために使われます）。

### キーのサニタイズ

`RateLimiter::clean_rate_limiter_key(key)` は、キーから `&abc;` 形式のHTMLエンティティのマーカーを取り除きます - Laravelは、`htmlentities` を往復するユーザー入力の文字列に対してこれを使います。Suprnovaは、取り除く段階を正確に再現しますが、（非UTF-8の入力にのみ関係し、Rustの `String` には無関係な）`htmlentities` のエンコードを前段に付け加えることは*しません*。この関数はSuprnova内部で決定的かつべき等です。PHPのサービスとバイト単位で同一のハッシュが必要な利用者は、入力に対して独自の `htmlentities` の前処理を実行してください。

```rust
assert_eq!(RateLimiter::clean_rate_limiter_key("a&amp;b"), "aab");
```

## `Limit` ビルダー

名前付きリミッターのコールバックが返すデータ型です。省略コンストラクタは、Laravelの `Limit::per*` を反映しています:

```rust
use suprnova::Limit;
use std::time::Duration;

Limit::per_second(10, 1);           // 1秒あたり10回（max_attempts、decay_seconds）
Limit::per_minute(60);              // 1分あたり60回
Limit::per_minutes(5, 100);         // 5分あたり100回（decay優先、Laravelのシグネチャ）
Limit::per_hour(1_000);             // 1000/時
Limit::per_hours(6, 5_000);         // 6時間あたり5000回
Limit::per_day(10_000);             // 10000/日
Limit::per_days(7, 50_000);         // 7日あたり50000回
Limit::new(123, Duration::from_secs(45));  // 素のコンストラクタ

// ビルダーチェーン。
let l = Limit::per_minute(5)
    .by("user:42")
    .response(|req| {
        suprnova::HttpResponse::text("blocked").status(429)
    })
    .after(|response| response.status_code() >= 400);
```

- `.by(key)` - バケットのキーを設定します。空のキーは「グローバル」です（すべての呼び出し元が1つのバケットを共有します）。
- `.response(callback)` - 上限に達したときのカスタムレスポンスを生成します。デフォルトは、素の429「Too Many Attempts.」です。
- `.after(callback)` - `callback(response)` が true を返したときだけ試行を消費します。典型的な用途: 失敗したログインだけをカウントすること（`after(|r| r.status_code() >= 400)`）。

`Limit::none()` は `Unlimited`（`max_attempts = i64::MAX` の `GlobalLimit`）を返します。名前付きリミッターからこれを返すのは、バイパスのためのLaravelパターンです。`GlobalLimit` 自体は、`Illuminate\Cache\RateLimiting\GlobalLimit` とのパリティのために保たれている、空のキーを持つ `Limit` の薄いラッパーです。

## `ThrottleRequestsMiddleware`

Cacheベースのファサードを包むHTTPラッパーです。`Illuminate\Routing\Middleware\ThrottleRequests` を反映しています。3つのコンストラクタがあります:

```rust
use suprnova::{Limit, ThrottleRequestsMiddleware};

// 名前付きリミッター - RateLimiter::limiter(name) を介してリクエスト時に解決される。
ThrottleRequestsMiddleware::by_name("api");

// インラインのmax/decay/prefix - Laravelの `throttle:60,1` そのままの形。
ThrottleRequestsMiddleware::with(60, 1, "myroute");

// Limitの明示的なリスト - 最初に上限に達したものが勝つ。最もRustらしい。
ThrottleRequestsMiddleware::with_limits(vec![
    Limit::per_hour(5_000).by("user:1"),
    Limit::per_minute(60).by("user:1"),
]);
```

ルートグループへ配線します:

```rust
use suprnova::{Limit, RateLimiter, Router, ThrottleRequestsMiddleware};

RateLimiter::define("api", |req| {
    Limit::per_minute(60)
        .by(req.ip().unwrap_or_else(|| "anon".into()))
        .into()
});

let router = Router::new()
    .get("/api/items", list_items)
    .post("/api/items", create_item)
    .middleware(ThrottleRequestsMiddleware::by_name("api"));
```

### キーには `req.ip()` を使い、ヘッダーは使わない

`X-Forwarded-For` は呼び出し元が指定するものです。生のヘッダーでキー付けされたリミッターは、リクエストごとに異なる値を送ることで無効化されます - 攻撃者は自分自身のバケットを選べるため、クォータはクライアントごとではなくリクエストごとになってしまいます。

`Request::ip()` は安全な読み取りです。**TCPのピアが `APP_TRUSTED_PROXIES` に列挙されている場合にのみ** `X-Forwarded-For` / `X-Real-IP` を返し、そうでなければピアのアドレスを返します。そのため、あなた自身のプロキシ以外からのヘッダーは無視されます。

この系も同じくらい重要です: その変数が未設定 - デフォルト - の場合、終端プロキシの背後にある `req.ip()` は、あらゆるリクエストで*プロキシの*アドレスを返し、アプリ内のあらゆるIP単位の上限が、1つの共有バケットへ潰れてしまいます。すると `ThrottleRequestsMiddleware::with(20, 1, "login")` は、全ユーザーを合わせて1分あたり20回の試行を意味するようになり、それは、誰か1人の呼び出し元が全員を締め出すために使い切れてしまいます。nginx、Traefik、ALB、Cloudflareの背後にデプロイすることは、[`APP_TRUSTED_PROXIES`](env-vars.md#behind-a-reverse-proxy-set-app_trusted_proxies)を設定することを意味します。

### レスポンスヘッダー

ラップされたすべてのレスポンスは、次を運びます:

- `X-RateLimit-Limit` - 設定された `max_attempts` です。
- `X-RateLimit-Remaining` - このバケットに残っている再試行回数です。

429のレスポンスは、さらに次を運びます:

- `Retry-After` - ウィンドウが再び開くまでの秒数です。
- `X-RateLimit-Reset` - バケットが再び開くときの、unix-seconds-since-epochです。

これは、Laravelの `ThrottleRequests::getHeaders` の形と正確に一致します。

### 名前付きリミッターが見つからない

ルートが `by_name("X")` へ配線されているのに、`X` の下にリミッターが登録されていない場合、ミドルウェアは、見つからないリミッター名を運ぶボディと共にHTTP 503を返します。Laravelは `MissingRateLimiterException` を投げます。私たちは、設定ミスの起動がワーカースレッドをパニックさせないよう、これをHTTPレスポンスとして表面化させます。

### ドライバーとファサードの合成

2つのミドルウェアは、1つのルーター上で共存できます。低レベルの公平性のためにスライディングウィンドウのドライバーを重ね、その上にエンドポイントごとの名前付き上限のためにCacheベースのスロットルを重ねてください:

```rust
let router = Router::new()
    .get("/api/items", list_items)
    .middleware(RateLimitMiddleware::new(limiter_driver, cfg, key_fn))
    .middleware(ThrottleRequestsMiddleware::by_name("api"));
```

## 設定

ドライバーSPIは環境変数を介して設定されます。Cacheベースのファサードは、あなたの[`Cache`](cache.md)ストアが設定されている場所であればどこでも設定されます（メモリまたはRedis）。

| 変数 | 使用箇所 | デフォルト |
|----------|---------|---------|
| `RATE_LIMIT_DRIVER` | ドライバーSPIのbootstrap | `memory`（本番環境では拒否されます - 上記参照） |
| `RATE_LIMIT_ALLOW_MEMORY_IN_PRODUCTION` | 本番環境のフェイルクローズのオーバーライド | 未設定 |
| `RATE_LIMIT_REDIS_URL` | Redisドライバー | `redis://127.0.0.1:6379` |
| `RATE_LIMIT_PREFIX` | Redisのキープレフィックス | `suprnova:` |
| `CACHE_DRIVER` / `REDIS_URL` / `CACHE_DEFAULT_TTL` / `REDIS_PREFIX` | Cacheベースの `RateLimiter` ファサード（[`Cache`](cache.md)を参照） | 様々 |

## Laravelからの移行

| Laravel | Suprnova |
|---------|----------|
| `RateLimiter::for('api', fn ($req) => Limit::perMinute(60))` | `RateLimiter::define("api", \|req\| Limit::per_minute(60).into())` または `RateLimiter::r#for(...)` |
| `RateLimiter::hit($key, $decay)` | `RateLimiter::hit(key, decay).await?` |
| `RateLimiter::tooManyAttempts($key, $max)` | `RateLimiter::too_many_attempts(key, max).await?` |
| `RateLimiter::availableIn($key)` | `RateLimiter::available_in(key).await?` |
| `RateLimiter::attempt($key, $max, $cb, $decay)` | `RateLimiter::attempt(key, max, \|\| async { ... }, decay).await?` |
| `RateLimiter::retriesLeft($key, $max)` | `RateLimiter::retries_left(key, max).await?` |
| `RateLimiter::cleanRateLimiterKey($key)` | `RateLimiter::clean_rate_limiter_key(key)` |
| `Limit::perMinute(60)->by($ip)->response(fn () => abort(429))` | `Limit::per_minute(60).by(ip).response(\|_\| HttpResponse::text("...").status(429))` |
| `Limit::perMinutes(3, 100)` | `Limit::per_minutes(3, 100)` |
| `Limit::none()` | `Limit::none()` |
| `throttle:api` ミドルウェア | `ThrottleRequestsMiddleware::by_name("api")` |
| `throttle:60,1` ミドルウェア | `ThrottleRequestsMiddleware::with(60, 1, "")` |
| `X-RateLimit-Limit/Remaining/Reset` + `Retry-After` ヘッダー | 同じヘッダー、同じ形 |

### Suprnovaが異なる設計を選んだ理由

Laravelは1つの形を出荷します: `Illuminate\Cache\RateLimiter`（Cacheベースの固定ウィンドウカウンター）と、そのHTTPラッパーとしての `Illuminate\Routing\Middleware\ThrottleRequests` です。Suprnovaは、その形*と*、ネイティブなスライディングウィンドウのドライバーSPIの両方を出荷します。なぜなら、2つの本物の問いには、2つの本物の答えが必要だからです。

Cacheベースのカウンターは、「名前付きリミッター、レスポンスコールバック、失敗ログインだけをカウントするためのafterコールバックがあり、Laravelのマイグレーションとソース互換でいたい」という問いに対する正しい答えです。それは、「Redis ZSETに対する、アトミックなLua評価を伴い、別建てのタイマーキーを持たない、厳密な1リクエスト1スロットのスライディングウィンドウの強制が必要だ」という問いに対しては間違った答えです。この2つ目の問いこそが、Tokioの並行性の上限にぶつかっているほとんどのRustサービスが実際に抱えているものです。だからこそ `RateLimiterDriver` + `RateLimitMiddleware` は、フィーチャーフラグの裏に隠れることなく、並んで存在しているのです。

バックエンドエラーのポリシーも、Suprnovaによる追加です。Laravelのミドルウェアは、「リミッターが壊れている」という判断を決して表面化させません。PHPのリクエストごとのライフサイクルがそれを隠してしまうからです - 次のリクエストは新しいプロセスを得ます。Redisを10秒間失った、長寿命のTokioワーカーは、そのウィンドウの間に到着するリクエストをどう扱うか決めなければなりません。`BackendErrorPolicy::FailOpen`（デフォルト）対 `FailClosed` は、その判断を明示的に露出させたものです。

## 次のステップ

- [ミドルウェア](middleware.md) - ミドルウェアがどのように合成され、実行され、リクエストチェーンの中でショートサーキットするか
- [キャッシュ](cache.md) - Laravelの形をした `RateLimiter` ファサードが構築されている、そのストア
- [設定](configuration.md) - キャッシュとRedisバックエンドのための型付き設定
- [認証フロー](auth-flows.md) - `LoginThrottleMiddleware` とブルートフォースのロックアウトパターンは、この表面の上に構築されています
- [エラー モデル](error-model.md) - なぜ `Result<HttpResponse, HttpResponse>` が、ミドルウェアをきれいにショートサーキットさせてくれるのか
