# CORS

`CorsMiddleware` は、プリフライトの `OPTIONS` リクエストに応答し、通常のクロスオリジンレスポンスに `Access-Control-Allow-*` ヘッダーを付与します。別のオリジンにいるブラウザがあなたのAPIを呼び出すとき - 公開API、別ドメインでホストされているSPA、モバイルのwebview、別途ホストされているドキュメントサイトなど - に、`bootstrap()` で一度だけインストールします。同一オリジンのアプリ（バックエンドと同じホストから配信されるInertia、これがSuprnovaのデフォルトです）には、CORSはまったく必要ありません。このミドルウェアはLaravelの `HandleCors` と `config/cors.php` を反映したものですが、`CorsConfig` 上の型付きビルダーとして提供されます。

## グローバルにインストールする

```rust,ignore
use std::time::Duration;
use suprnova::{global_middleware, CorsConfig, CorsMiddleware};

pub fn register() {
    global_middleware!(CorsMiddleware::new(
        CorsConfig::allow_origins(["https://app.example"])
            .allow_credentials(true)
            .max_age(Duration::from_secs(600)),
    ));
}
```

プリフライトとは、`Access-Control-Request-Method` ヘッダーを伴う `OPTIONS` リクエストのことです。ルーターには `OPTIONS` のルートが存在しないため、プリフライトがルートに*マッチする*ことは決してありません - しかしSuprnovaのサーバーは、マッチしなかったリクエストに対してもグローバルミドルウェアチェーンを実行します（最後は404で終わります）。そのため、グローバルにインストールされた `CorsMiddleware` はプリフライトを目にし、404が生成されるより前に `204` でショートサーキットします。**これが、CORSをルートごとではなくグローバルにインストールしなければならない理由です。**

## オリジンポリシーを選ぶ

`CorsConfig` には、意図的に `Default` が用意されていません。反射的に何でも許可してしまうポリシーはセキュリティ上の地雷なので、あなた自身が選ばなければなりません:

| ビルダー | 振る舞い |
| --- | --- |
| `CorsConfig::allow_origins([...])` | 固定の許可リスト。オリジンがいずれかのエントリと完全に一致したときにだけ、そのオリジンがエコーされます。 |
| `CorsConfig::any_origin()` | ワイルドカードの `*`。クレデンシャルが有効になっている場合、ミドルウェアは `*` ではなくリクエストの具体的なオリジンをエコーします（`*` とクレデンシャルの組み合わせは、Fetch仕様上は不正です）。 |
| `.allow_origin_patterns([...])` | リテラルのリストに上乗せして追加される正規表現パターン。動的なサブドメインに便利です。 |

```rust,ignore
CorsConfig::allow_origins(["https://app.example"])
    .allow_origin_patterns([r"^https://[a-z0-9-]+\.staging\.example$"])
```

パターンは自動的にアンカーされます - `^` と `$` が欠けていれば先頭と末尾に補われるため、`https://evil.com/?u=https://app.example` のようなリダイレクトURLに対する部分一致がすり抜けることはありません。

不正な正規表現は、リクエスト時ではなく設定時（起動時）にパニックします - 黙ってフェイルオープンするのではなく、設定のバグを目立つ形で表に出すためです。

`allowed_origins_patterns`（Laravel風の名前のエイリアス）も利用できます。

## CORSを適用するパスを絞り込む

Laravelの `cors.php` 設定には `paths` 配列（`['api/*', 'sanctum/csrf-cookie']`）があり、CORSの適用先を特定のURLパターンに限定します。Suprnovaもこれを反映しています:

```rust,ignore
CorsConfig::allow_origins(["https://app.example"])
    .paths(["api/*", "sanctum/csrf-cookie"])
```

`paths` を設定しない場合、CORSはすべてのリクエストで実行されます（これがSuprnovaのデフォルトです - ミドルウェアは登録によるオプトインだからです）。パターンを1つ以上設定した場合、CORSの処理を受けるのはマッチしたリクエストだけになり（プリフライト**と**実レスポンスの装飾の両方）、それ以外はすべて手を触れられずに通り抜けます。

パターンはLaravelの `Str::is` のセマンティクスに従います: `*` は `/` をまたいで貪欲にマッチする、複数セグメントのワイルドカードです。先頭の `/` は正規化されるため、`"api/*"` と `"/api/*"` は等価です。

```rust,ignore
"api/*"             // /api/users、/api/users/42 にマッチ
"api/*/posts"       // /api/v2/posts、/api/v1/posts にマッチ
"sanctum/csrf-cookie" // 完全一致のリテラル
"*"                 // すべてのパスにマッチ
```

## 述語によるスキップ

パスパターンには収まらない、リクエストの形に基づく述語（ヘッダーを見てスキップする、本番環境でのみCORSを実行する、ヘルスチェックの間はスキップする）には、`skip_when` を使ってください:

```rust,ignore
CorsConfig::any_origin()
    .skip_when(|req| req.header("X-Internal-Call").is_some())
    .skip_when(|req| req.path() == "/healthz")
```

Laravelの `HandleCors::skipWhen(Closure)` を反映していますが、グローバルな可変状態としてではなく、ポリシーの上に置かれています。`skip_when` のコールバックは複数登録でき、そのうち1つでも `true` を返せばCORSはスキップされます。

## メソッド、ヘッダー、公開ヘッダー

```rust,ignore
CorsConfig::allow_origins(["https://app.example"])
    .methods(["GET", "POST", "DELETE"])           // デフォルト = GET/POST/PUT/PATCH/DELETE/OPTIONS/HEAD
    .allow_headers(["Content-Type", "X-CSRF-TOKEN"])  // 制限する。デフォルトはリクエストされたものをそのまま返す
    .allow_any_headers()                          // 「リクエストされたものを何であれそのまま返す」を明示
    .expose_headers(["X-Total-Count", "Link"])    // JS がレスポンス上で読めるヘッダー
```

Laravel風の名前のエイリアス（`cors.php` のユーザーが期待どおりのものを見つけられるように）:

- `allowed_methods(...)` ≡ `methods(...)`
- `allowed_headers(...)` ≡ `allow_headers(...)`
- `exposed_headers(...)` ≡ `expose_headers(...)`
- `allowed_origins_patterns(...)` ≡ `allow_origin_patterns(...)`
- `supports_credentials(...)` ≡ `allow_credentials(...)`

## クレデンシャルと `*`

Fetch仕様によれば、`Access-Control-Allow-Origin: *` はクレデンシャルと一緒に使うと不正です - ブラウザがそのレスポンスを拒否します。明示的なオリジンのリスト（`allow_origins([...])`）に `allow_credentials(true)` を組み合わせた場合、ミドルウェアは `*` ではなくリクエストの具体的な `Origin` をエコーするため、ポリシーは期待どおりに動作します。

**`any_origin() + allow_credentials(true)` はビルド時にパニックします。** この組み合わせは、オリジンの許可リストを完全に迂回してしまいます: 攻撃者のページであれば何であれ、クレデンシャル付きのクロスオリジンリクエストを行い、そのレスポンスを読み取れてしまうのです。実行時に間違ったヘッダーを出力するのではなく、ポリシーのコンストラクタがはっきりと失敗するため、この設定ミスが動作中のデプロイに届くことは決してありません。代わりに、明示的な許可リストを使ってください:

```rust,ignore
// 正しい例 - 明示的な許可リストとクレデンシャルの併用。
CorsConfig::allow_origins(["https://app.example"]).allow_credentials(true)
// → Origin: https://app.example のリクエストに対して
// → レスポンス: Access-Control-Allow-Origin: https://app.example
//               Access-Control-Allow-Credentials: true

// ビルド時に拒否されます - 対処方法を示すメッセージとともにパニックします。
// CorsConfig::any_origin().allow_credentials(true)
```

## Max-age

```rust,ignore
.max_age(Duration::from_secs(600))   // 型付き
.max_age_secs(600)                   // Laravel 風の整数秒
```

`Access-Control-Max-Age` は、プリフライトの結果をどれだけの間キャッシュしてよいかをブラウザに伝えます。値が大きいほどプリフライトの往復は減りますが、ポリシーの変更が伝わるのは遅くなります。

## ミドルウェアが実際に出力するもの

### プリフライト（`OPTIONS` + `Access-Control-Request-Method`）

オリジンが許可されている場合:

```
HTTP/1.1 204 No Content
Access-Control-Allow-Origin: <origin>
Access-Control-Allow-Credentials: true        // クレデンシャルが有効なとき
Access-Control-Allow-Methods: GET, POST, ...
Access-Control-Allow-Headers: <reflected or fixed>
Access-Control-Max-Age: 600                   // 設定されているとき
Vary: Origin, Access-Control-Request-Method, Access-Control-Request-Headers
```

オリジンが許可されていない場合: 素の `204` と `Vary` だけです（`Access-Control-*` は付きません）。ブラウザ側のヘッダー欠落チェックがCORSエラーを生み出します - `tower-http` の慣例と一致する挙動です。

### 実際のクロスオリジンレスポンス

リクエストが `Origin` ヘッダーを持ち、かつそのオリジンが許可されている場合:

```
Access-Control-Allow-Origin: <origin or *>
Access-Control-Allow-Credentials: true        // 有効になっているとき
Access-Control-Expose-Headers: X-Total, Link  // 設定されているとき
Vary: Origin                                  // "*" ではないときだけ
```

`*` のACAOはどのオリジンに対しても同一なので、`Vary` は必要ありません。一方、具体的なオリジンはオリジンごとに変わるため、共有キャッシュはそれをキーに含めなければなりません。

## CORSハンドラのテスト

CORSはブラウザ側で強制されるものです - オリジンが許可されていない場合でも、サーバーはハンドラを実行します。ただレスポンスを装飾しないだけです。テストできるのは、この振る舞いです:

```rust,ignore
let (status, headers, body) = request_with_origin(
    "/api/data",
    "https://app.example",
).await;
assert_eq!(status, 200);
assert_eq!(
    headers.get("access-control-allow-origin"),
    Some(&"https://app.example".to_string()),
);
```

許可されていないオリジンの場合も、ハンドラは実行されてボディは返ってきますが、`Access-Control-Allow-Origin` が存在しないことによって、ブラウザはそれを読み取れなくなります。

## Laravel パリティ表

| Laravel の `cors.php` | Suprnova のビルダー |
| --- | --- |
| `paths` | `.paths([...])` |
| `allowed_methods` | `.methods([...])` / `.allowed_methods([...])` |
| `allowed_origins` | `CorsConfig::allow_origins([...])` |
| `allowed_origins_patterns` | `.allow_origin_patterns([...])` / `.allowed_origins_patterns([...])` |
| `allowed_headers` | `.allow_headers([...])` / `.allowed_headers([...])` |
| `exposed_headers` | `.expose_headers([...])` / `.exposed_headers([...])` |
| `max_age` | `.max_age(Duration)` / `.max_age_secs(u64)` |
| `supports_credentials` | `.allow_credentials(bool)` / `.supports_credentials(bool)` |
| `HandleCors::skipWhen(closure)` | `.skip_when(\|req\| ...)` |

このミドルウェアは、Laravel流の「`paths` に対して自動的にインストールされる」やり方ではなく、グローバルに登録します - Suprnovaのミドルウェアチェーンは明示的です。設計については[ミドルウェア](middleware.md)を参照してください。

### Suprnovaが異なる設計を選んだ理由

Laravelの `HandleCors` はカーネルに自動的に取り付けられ、ポリシーを `config/cors.php` から読み取ります。この形がPHPでうまく機能するのは、リクエストごとにプロセスが立ち上がるフレームワークにとって、設定配列こそが、リクエストごとに評価し直すことなく設定を共有できる唯一の場所だからです。Suprnovaは同じオプションを、`global_middleware!` で明示的に登録する型付きの `CorsConfig` ビルダーとして公開します。これにより、ミドルウェアチェーンは `bootstrap()` の中で目に見えるまま保たれ、許可リストとワイルドカードのどちらを選ぶのかをコンパイラが強制できます（`CorsConfig` には `Default` がないため、設定値の記入を忘れたせいでうっかり `Access-Control-Allow-Origin: *` を出荷してしまう、ということが起こりません）。

もう1つの相違点は、ルーティングされていないパスであってもプリフライトがミドルウェアに到達することです。Laravelは `OPTIONS` をルーター経由でルーティングするため、プリフライトは（RESTルートごとに自動登録される）`OPTIONS` ルートにマッチします。Suprnovaのルーターには `OPTIONS` のルートがありません。代わりにサーバーが、マッチしなかったリクエストに対して404を返す前にグローバルミドルウェアチェーンを実行するため、グローバルにインストールされた `CorsMiddleware` は、not-foundの経路がたどられる前にプリフライトを `204` でショートサーキットします。これが、CORSをグローバルにインストールし*なければならない*理由です - ルートごとの登録では、プリフライトを目にすることは決してないのです。

## 次のステップ

- [ミドルウェア](middleware.md) - トレイト、チェーン、グローバル登録とルートごとの登録、終了処理フック
- [CSRF](csrf.md) - ほとんどのアプリがCORSと並べてインストールする、もう1つのグローバルミドルウェア
- [ルーティング](routing.md) - ルートがどのようにマッチするのか（そしてなぜプリフライトはマッチしないのか）、そしてグローバルチェーンが走るフォールバックなしの経路
- [リクエスト ライフサイクル](lifecycle.md) - セッション、CSRF、ハンドラとの関係で、CORSがチェーンのどこに位置するか
- [設定](configuration.md) - 環境変数駆動の設定を必要とするミドルウェアのための、型付き設定パターン
