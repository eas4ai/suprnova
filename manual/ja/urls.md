# URL 生成

URLは、アプリケーションが自分自身を参照するための手段です - あらゆるリダイレクト、あらゆるメールのリンク、あらゆるInertiaの `<Link>` のhref、あらゆる署名付きダウンロードは、どこかから生まれてこなければなりません。パスをハードコードすると、リファクタリングは苦痛になり、ルート名の変更は危険になります。Suprnovaは、小さな `url::` 名前空間と、その兄弟である `route()` ヘルパーを出荷しています。これらは名前とパラメータを受け取って文字列を返し、パーセントエンコードを引き受け、署名の発行にも対応し、Laravelの通信上の形式とバイト単位で一致する検証を備えています。

この章は、URL生成の表面のリファレンスです。ルートを宣言して名前を付ける方法は[ルーティング](routing.md)の章が扱います。こちらの章が扱うのは、その名前をその後どう使うかです。

```rust
use suprnova::{route, url};

// 名前から URL を引く
let profile = route("users.show", &[("id", "42")]).unwrap();
//   "/users/42"

// APP_URL を基準にした絶対 URL
let absolute = url::to("/dashboard");
//   "https://app.test/dashboard"

// パスワードリセット用の署名付きリンク
let link = url::signed_route("password.reset", &[("token", reset_token)])?;
//   "/password/reset/xyz?signature=ab12..."

// 受信したリクエストで検証します
if url::has_valid_signature(&request)? {
    // それに応じて動きます
}
```

この章に出てくるものはすべて `suprnova::url::*` と `suprnova::route` の下に再エクスポートされているため、利用側のコードがルーティングのモジュールへ直接手を伸ばす必要はありません。

## 名前付きルート

名前とは、登録の時点でルートに付けられる文字列のラベルです。名前がひとたび存在すれば、`route(name, params)` がそれをURLのパターンへ解決し、パラメータを埋め込みます。名前は、プロセス全体で1つのグローバルなレジストリに置かれます - `name → path` の表は、実行中のバイナリごとに1つであって、`Router` ごとに1つではありません。

```rust
use suprnova::{routes, get, post};

routes! {
    get!("/", controllers::home::index).name("home"),
    get!("/users/{id}", controllers::users::show).name("users.show"),
    post!("/users", controllers::users::store).name("users.store"),
}
```

`.name(...)` の呼び出しは、`"users.show" → "/users/{id}"` を登録します。それ以降は、プロセス内のどこからでもその名前を解決できます:

```rust
use suprnova::route;

let url = route("users.show", &[("id", "42")]);
// Some("/users/42")

let missing = route("does.not.exist", &[]);
// None
```

同じ `(name, path)` の組を再登録することはべき等です - 起動中にルートの登録が複数回走る場合に役立ちます。*別の*パスの下に同じ名前を登録するとパニックします。この衝突がセキュリティに関わる欠陥なのは、`Redirect::route` のようなヘルパーが、競争に勝ったほうを黙って行き先にしてしまうからです。

### 名前を引くヘルパー

| 関数 | 戻り値 | ルートが見つからないとき |
|---|---|---|
| `route(name, params)` | `Option<String>` | `None` |
| `route_with_params(name, params_map)` | `Option<String>` | `None` |
| `try_route(name, params)` | `Result<String, RouteUrlError>` | `Err(NameNotFound)` |
| `try_route_with_params(name, params_map)` | `Result<String, RouteUrlError>` | `Err(NameNotFound)` |

寛容な `route` / `route_with_params` の組は、埋まらなかった `{placeholder}` のセグメントを、そのままの形で出力に残します - デバッグログには問題ありませんが、ブラウザへ送り出すには危険です。厳格な `try_route` / `try_route_with_params` の組は、埋まらなかったプレースホルダーを並べた `RouteUrlError::MissingParams { name, missing }` を返すため、呼び出し側はユーザーを `/users/{id}` へリダイレクトする代わりに、はっきりと失敗できます。

```rust
use suprnova::routing::{try_route, RouteUrlError};

match try_route("users.show", &[]) {
    Ok(url) => /* safe to redirect */,
    Err(RouteUrlError::MissingParams { name, missing }) => {
        // missing == vec!["id"]
        return Err(FrameworkError::internal(
            format!("cannot build URL for {name}: missing {missing:?}"),
        ));
    }
    Err(RouteUrlError::NameNotFound(name)) => {
        return Err(FrameworkError::internal(format!("unknown route: {name}")));
    }
}
```

`Redirect::route` が内部で `try_route_with_params` を使っているのは、まさにこの理由からです - `Location` ヘッダーに生の `{id}` が入ったリダイレクトは、失敗するよりもたちが悪いからです。

### パーセントエンコードは自動です

パラメータの値は、埋め込まれる前にRFC 3986のパスセグメントの規則に従ってエンコードされます。対象になるのは、gen-delimsとsub-delims（`/ ? # [ ] @ ! $ & ' ( ) * + , ; =`）、制御文字、空白、そして `%` 自身です。予約されていない文字（`A-Z a-z 0-9 - _ . ~`）は、そのまま通り抜けます。

```rust
use suprnova::route;

// スラッシュを含むスラッグも、1つのセグメントに収まります:
route("posts.show", &[("slug", "hello/world")]);
// Some("/posts/hello%2Fworld")

// パストラバーサルの試みも、セグメントの外へは出られません:
route("users.show", &[("id", "../../etc/passwd")]);
// Some("/users/..%2F..%2Fetc%2Fpasswd")

// 本物のUnicodeは、手を触れられずに通り抜けます:
route("users.show", &[("id", "user-é-42")]);
// Some("/users/user-%C3%A9-42")
```

マッチする側もこの往復を保ちます - `/posts/hello%2Fworld` へのリクエストは `/posts/{slug}` のルートにマッチし、`req.param("slug")` を読むハンドラには、デコードされた `"hello/world"` が見えます。境界でエンコードし、境界でデコードします。ハンドラのコードが生のバイト列を目にすることは決してありません。

### 逆引き

マッチしたルートのパターンを持っていて、登録されている名前が欲しいとき - 例えばログのため、あるいは `Request::route_is("users.show")` の判定のため - には、`route_name_for_pattern` を使ってください:

```rust
use suprnova::routing::route_name_for_pattern;

let name = route_name_for_pattern("/users/{id}");
// Some("users.show")
```

これは、名前のレジストリを走査するO(n)のスキャンです。nは登録されている名前の数です。ルート数が4桁になっても、そのコストは周囲のリクエストライフサイクルに比べれば無視できます。この関数はツールとミドルウェアのために公開されています - ハンドラの中で名前付きルートと比較する場合には、`Request::route_is` がすでにあなたの代わりにこれを呼んでいます。

## 絶対URL

それ以外のすべて - メールの組み立て、URLの共有、Open Graphのメタデータの送出 - では、正しいスキームとホストを備えた絶対URLが欲しくなります。`url::to` は、パスを `APP_URL` へ連結します:

```rust
use suprnova::url;

// 環境変数: APP_URL=https://app.example.com
let url = url::to("/about");
// "https://app.example.com/about"

// すでに絶対URLであるものは、手を触れられずに通り抜けます:
let cdn = url::to("https://cdn.example/asset.js");
// "https://cdn.example/asset.js"

let proto_relative = url::to("//cdn.example/asset.js");
// "//cdn.example/asset.js"
```

ホスト、スキーム、ポートは、いずれも `APP_URL` から来ます。`APP_URL` が `http://localhost:8765` であれば、`url::to("/foo")` は `"http://localhost:8765/foo"` を返します。`APP_URL` の末尾のスラッシュは正規化によって取り除かれるため、`https://host//path` になってしまうことはありません。

### HTTPSを強制する

`url::secure(path)` は同じ絶対URLを組み立てますが、`APP_URL` が `http://` であってもスキームを `https://` へ引き上げます:

```rust
use suprnova::url;

// 環境変数: APP_URL=http://app.example.com
url::secure("/login");
// "https://app.example.com/login"
```

本番環境では、通常 `APP_URL` にHTTPSのホストを一度だけ設定し、`secure` を直接呼ぶことはありません - この引き上げは、ローカルの開発がHTTPで動いていながら、特定のリンクだけはHTTPSでなければならない環境（例えば、決済セッションに埋め込まれるコールバックURL）のためのものです。

### 現在のURLを読む

ハンドラの内部では、リクエストそのものが拠り所になります:

```rust
use suprnova::url;

async fn breadcrumbs(req: Request) -> Response {
    let here = url::current(&req);       // "/posts/42?expand=author"
    let full = url::full(&req);          // "https://app.test/posts/42?expand=author"
    let back = url::previous("/");        // セッションに記録された直前のURL
    // ...
}
```

| ヘルパー | 戻り値 | 取得元 |
|---|---|---|
| `url::current(&req)` | このリクエストのパス + クエリ | 現在の `Request` |
| `url::full(&req)` | このリクエストの絶対URL | `APP_URL` + `current(&req)` |
| `url::previous(fallback)` | セッションミドルウェアが記録した直前のURL | セッション内の `_previous.url`、なければ `fallback` |

`Redirect::back` を支えているのが `previous` です - セッションミドルウェアは、成功したHTMLのGETについてそのURLを記録するため、フォームの `POST` は送信元のページへ跳ね返れます。Inertiaの部分更新、JSON-APIのリクエスト（`text/html` を伴わない `Accept: application/json`）、2xx/3xx以外のレスポンスは記録されません。そのため、ユーザーが一度も目にしていない中間のエンドポイントへ跳ね返ることはありません。ミドルウェアは、ルート相対かつ同一オリジンでないURLも記録を拒否します。`//host` や `/\host`（どちらもブラウザではパスではなくプロトコル相対として解釈される）の形をしたリクエストパス、またはどこかにASCII制御バイトを含むパス（ブラウザのURLパーサーがオリジン比較の前に除去する `TAB` や改行が、見た目には安全なパスを上記二つの形式のいずれかに変えてしまう）は、決して保存されません。同じ検査はすべての読み取りで再度実行されるため、古いリリースが保存した値も、すでにセッション内にあるという理由だけで信頼されず、引き続き失敗します。いずれにせよ、過去または現在、アプリケーションへ届く異常なリクエストパスによって `previous` と `Redirect::back` がオリジン外へ誘導されることはありません。

## 署名付きURL

署名付きURLを使うと、URLをどこにも保存することなく、それが自分のサーバーから来たことを証明できるURLを発行できます。署名は、`APP_KEY` を使い、URLの正規形に対して計算したHMAC-SHA256です。サーバーは受信したリクエストでHMACを再計算し、一致する署名だけを受け入れます。

署名付きURLに手を伸ばすのは、次のような場面です:

- **メールで届けられるリンク** - パスワードリセット、メール認証、メールによる招待、マジックリンクによるログインです。URLは、不透明な状態として保存しておけないまま、受信箱を経由する往復を生き延びなければなりません。
- **一時的なダウンロード** - 「CSVのエクスポートができました」という24時間で期限切れになるリンクや、URLを自分のドメイン上に留めておきたい場合の、署名付きS3の代替です。
- **自分のところへ戻ってくるWebhook** - サードパーティからのコールバックで、リクエストごとのデータベース検索を必要とせずに、偽造された呼び出しを拒否したいものです。

```rust
use suprnova::url;
use chrono::Utc;

// 恒久的な署名付きURL - 期限切れになりません。
let link = url::signed_route(
    "password.reset",
    &[("user", user_id), ("token", token)],
)?;
// "/password/reset/42/xyz?signature=ab12cd34..."

// 一時的な署名付きURL - 今から1時間後に期限切れになります。
let expires_at = Utc::now().timestamp() + 3600;
let link = url::temporary_signed_route(
    "verify.email",
    &[("user", user_id)],
    expires_at,
)?;
// "/verify/email/42?expires=1748803600&signature=def012..."
```

`expires_at_epoch_seconds` は継続時間ではなく、**絶対的なUNIXタイムスタンプ**であることに注意してください。呼び出し側で計算します:

```rust
let one_hour_from_now = chrono::Utc::now().timestamp() + 3600;
let one_day_from_now  = chrono::Utc::now().timestamp() + 86_400;
```

こうすることでヘルパーのシグネチャは小さく保たれ、「今から相対」の期限にも「明示的に絶対」の期限にも、同じ関数を使い回せます。

### 検証する

受信側では、実際のリクエストに対して署名を検証します:

```rust
use suprnova::{url, FrameworkError, Request, Response, HttpResponse};

pub async fn reset(req: Request) -> Response {
    reset_inner(req).await.map_err(HttpResponse::from)
}

async fn reset_inner(req: Request) -> Result<HttpResponse, FrameworkError> {
    if !url::has_valid_signature(&req)? {
        return Err(FrameworkError::forbidden("Invalid or expired link"));
    }
    // 署名は正しく、期限切れでもありません - 先へ進みます。
    let user_id = req.param("user").unwrap();
    // ...
    Ok(HttpResponse::text("ok"))
}
```

`has_valid_signature` が `true` を返すのは、HMACが一致し、なおかつURLが期限切れでない場合だけです。*無効*、*期限切れ*、*有効*の三択で区別したい場合は、`signature_verdict` を使ってください:

```rust
use suprnova::{url, FrameworkError, HttpResponse, Request, Response};
use suprnova::routing::SignatureVerdict;

pub async fn reset(req: Request) -> Response {
    reset_inner(req).await.map_err(HttpResponse::from)
}

async fn reset_inner(req: Request) -> Result<HttpResponse, FrameworkError> {
    match url::signature_verdict(&req)? {
        SignatureVerdict::Valid => {
            // 先へ進みます。
        }
        SignatureVerdict::Expired => {
            // リンクが期限切れであることを説明し、新しいものを送ると申し出る
            // ページへ、ユーザーを送り返します。
            return Ok(HttpResponse::new()
                .status(302)
                .header("Location", "/password/reset-expired"));
        }
        SignatureVerdict::Invalid => {
            // 汎用的な403を描画します - 署名が不正な形式だったのか、欠けていたのか、
            // それとも単に間違っていたのかを漏らしてはいけません。
            return Err(FrameworkError::forbidden("Invalid link"));
        }
    }
    // ...
    Ok(HttpResponse::text("ok"))
}
```

`signature_has_not_expired(&req)` は非推奨であり、今では `has_valid_signature` とまったく同じことを答えます。代わりに、上の `signature_verdict` に手を伸ばしてください。`expires` クエリパラメータを持たないURLは、Laravelと同じくSuprnovaでも、定義上「決して期限切れにならない」ものです。

### Suprnovaが異なる設計を選んだ理由

Laravelの `URL::signatureHasNotExpired($request)` は文字どおり「期限切れではない」を意味するため、**偽造された**署名でも `true` が返ってきます - そもそも、切れるべき期限を持っていなかったからです。Suprnovaのものも、かつてはそれに合わせていました。今は違います: このヘルパーは、まず有効な署名であることを要求します。

理由は、HMACがそうではないと言うまで `expires` は攻撃者が与えた値であり、署名が確認できるまで、そこから導かれた答えには何の意味もないからです - そして、名前が防御用のチェックのように読める関数が、それだけを呼び出しているあらゆる箇所で、偽造されたURLをすべて素通りさせていました。

有効であることを要求すると、この関数は `has_valid_signature` へと畳み込まれます。だからこそ、挙動を切り替えるフラグではなく非推奨という扱いになっているのです。この畳み込みは失われたものではありません: 三値の判定のもとでは、1つの `bool` が正直に報告できる「期限切れではない」は、`Valid` 以外に存在しないからです。*期限切れ*と*無効*を言い分けたい - 「禁止」ではなく「新しいリンクを要求してください」と伝えたい - のであれば、そのためにあるのが `signature_verdict` であり、それは型でそう述べています。

### 任意のURLに署名する

署名したいURLが、登録された名前付きルートから来たものでない場合 - サードパーティから渡されたコールバックURL、実行時に動的に組み立てられたパスなど - には、`signed_url` を直接使ってください:

```rust
use suprnova::url;

let callback = url::signed_url(
    "/webhooks/stripe/callback?order=42",
    Some(chrono::Utc::now().timestamp() + 600),  // 10分間の有効期限
)?;
```

有効期限に `None` を渡すと、恒久的な署名を発行します。検証側は同じです - `has_valid_signature(&req)` は、そのURLが名前付きルートから発行されたのか、生のパスから発行されたのかを気にしません。

### 通信上の形式

クエリパラメータの順序だけが違う2つのURLは、同じ署名を生みます。正規形が、ハッシュを取る前にクエリの組を辞書順に並べ替えるからです。これが重要なのは、クライアントが転送の途中でクエリパラメータの順序を入れ替えることがあり（プロキシ、リンクのプレビュー生成、モバイルのメールアプリなど）、並べ替えで壊れてしまう署名付きURLは使い物にならないからです。

| 構成要素 | 値 |
|---|---|
| アルゴリズム | HMAC-SHA256 |
| キー | 現在有効な `APP_KEY` の生のバイト列 |
| ペイロード | `path?<sorted-query>`（パラメータがなければ `?` は省きます） |
| 並べ替えの順序 | `(key, value)` - 繰り返しも含めた、すべての組 |
| エンコード | 16進エンコードされた64文字のダイジェスト |
| 比較 | `subtle::ConstantTimeEq` による定数時間の比較 |
| 予約されたキー | `signature`、`expires` |

**繰り返されたキーは、まとめられるのではなく署名の対象になります。** `?tag=a&tag=b` は両方の値をペイロードへ運ぶため、署名を壊さずにどちらかを追加したり、取り除いたり、差し替えたりすることはできません。キーだけでなく `(key, value)` で並べ替えていることが、その順序を全順序に保っています。そのため、あるキーが複数回現れる場合でも、上の並べ替えに関する保証は成り立ちます。

これをわざわざ述べておくのは、そうでない実装が手ひどく噛みつくからです。以前のバージョンはマップへ正規化しており、繰り返されたキーについては最後の値だけを残していました。`Request::query_param` が返していたのは*最初*の値です。そのため、正当に署名された `?user=victim` は、元の署名を付けたまま `?user=attacker&user=victim` としてリプレイできてしまいました: 検証は `victim` を見て通し、ハンドラは `attacker` に対して動いたのです。署名されたURLと実行されたURLが別物でした。現在は3つのクエリアクセッサー - `query_param`、`query_params`、`Context::query_param` - がいずれも、繰り返されたキーを最後の値へ解決し、正規形は何も失いません。

`signature` や `expires` が繰り返されている場合は、その場で拒否されます。これらは制御用のパラメータであり、どちらかが2つあれば「どちらが優先されるのか」に恣意的でない答えは残りません。そして、当て推量をする役目を負うべきなのは検証側ではありません。

HMACのペイロードは、すでに付いている `signature` クエリパラメータを除外し（そのため、署名の上にさらに署名しても何も起きません）、呼び出しの引数から新しい `expires` の値を出し直します。`expires` を取り除いたり書き換えたりするクライアントは署名を壊し、`signature` を取り除くクライアントは `Invalid` として失敗します。どちらも安全側に倒れて失敗します。

フラグメント（`#section`）は正規形から取り除かれます。ブラウザがフラグメントをサーバーへ送り返すことは決してないからです。フラグメントまで署名の対象にすると、クライアントがアンカーを付け足した瞬間にすべてのリンクが無効になってしまいます - `?signature=...#docs` は、サーバー側で検証を通りません。

### 予約されたクエリパラメータ

`signature` と `expires` は、予約されたクエリパラメータ名です。`signature` や `expires` という名前のクエリパラメータを正当に期待するルートは、署名付きURLの仕組みと衝突し、検証側はその値を取り違えてしまいます。パラメータの名前を変えるか、そのルートが受け取るパラメータを別の名前空間の下に包んでください。

```rust
// 悪い例 - `signature` は予約された名前と衝突します。
get!("/api/check", check)  // ?signature=hash を受け取ります

// 良い例 - 名前空間を付けます。
get!("/api/check", check)  // ?body_signature=hash を受け取ります
```

定数は、Laravelの通信上の形式との対称性のために公開されています:

```rust
use suprnova::routing::{SIGNATURE_KEY, EXPIRES_KEY};
// SIGNATURE_KEY == "signature"
// EXPIRES_KEY   == "expires"
```

### キーローテーション

署名付きURLは、`Crypt::encrypt` とセッションクッキーの完全性を支えているのと同じ `APP_KEY` を使います。`APP_KEY` をローテーションすると、それまでに発行され、まだ使われていない署名はすべて無効になります - 配送済みのパスワードリセットのメールは、ユーザーが次にクリックした時点で403になります。

たいていのアプリケーションでは、それが正しい挙動です。重なりを持たせた緩やかなローテーション（古いリンクがデプロイの期間中も動き続けるようにすること）が必要なら、`APP_KEY_PREVIOUS` を使って前のキーを持ち越してください。キーリングは、検証の際にインストールされているすべてのキーを試します。キーリングの全体像については、[ハッシング](hashing.md)の章を参照してください。

## エラーとエッジケース

知っておく価値のある失敗のしかたが、いくつかあります:

- **`route(name, ...)` が `None` を返す**のは、その名前が登録されていない場合です。これは寛容な表面であり、黙って失敗するのは意図的です - 呼び出し側のコードがデフォルトへフォールバックできるようにするためです。はっきりと失敗させたいなら `try_route` を使ってください。
- **`try_route` は、未知の名前に対して `Err(NameNotFound)` を返し**、必要な `{placeholder}` に対応する値がない場合は `Err(MissingParams { name, missing })` を返します。
- **`url::signed_route` とその仲間は `FrameworkError` を返します** - 暗号化キーがインストールされていない場合（例えば `.env` の `APP_KEY` を忘れた場合）です。本番環境では、`Crypt::init` が `Server::from_config` の途中で走るため、これは起動時に失敗します。ここにあるエラー経路は、検証できないリンクを作り出す代わりに、設定の誤りを目立つ形で表に出すために存在しています。
- **`has_valid_signature` は、無効あるいは期限切れの署名に対して、`Err` ではなく `Ok(false)` を返します。** `FrameworkError` のバリアントは、「サーバーが検査すらできない」という失敗（キーがない場合）のために取ってあります。
- **`expires` が改ざんされた署名付きURL**は、`Expired` ではなく `Invalid` として検証されます。HMACのペイロードには `expires` の値が含まれているため、それを変更すると先に署名が壊れます。

```rust
use suprnova::{routing::SignatureVerdict, url};

// これらはいずれも Expired ではなく Invalid です:
url::signature_verdict(&req)?;  // signature クエリパラメータが欠けている
url::signature_verdict(&req)?;  // signature が16進数でないゴミ
url::signature_verdict(&req)?;  // パスが改ざんされた (/orders/1 → /orders/2)
url::signature_verdict(&req)?;  // いずれかのクエリパラメータの値が改ざんされた
url::signature_verdict(&req)?;  // expires の値が改ざんされた

// こちらは Expired です:
url::signature_verdict(&req)?;  // HMACは有効だが、現在時刻 > expires
```

## Suprnovaが異なる設計を選んだ理由

Laravelの `URL` ファサードは、`asset()`、`secureAsset()`、`assetFrom()`、`action()` を備えています。Suprnovaはそのいずれも出荷していません - 意図的な理由があります。

**アセット。** Suprnovaにおけるフロントエンドのやり方は、独立したアセット用のヘルパーではなく、Viteとファイルシステムのディスク（[ファイルシステムとストレージ](filesystem.md)）です。Viteの `@vite('resources/app.ts')` ディレクティブ（あるいはInertiaアダプターの同等物）は、本番環境では正しいハッシュ付きのURLを、開発環境では開発サーバーのURLを出力します。これと並行する `URL::asset()` という経路を作れば、アセットの扱いは、ハッシュ、バージョン管理、そしてどのマニフェストが権威を持つのかについて合意しなければならない2つのシステムに分かれてしまいます。その責任は、すでにVite側が勝ち取っています。

**アクションによるルーティング。** Laravelの `action('UserController@show', ['id' => 1])` は、PHPのクラス名文字列によるルーティングに依拠しています - コントローラーはメソッドを持つクラスであり、フレームワークは `action` の文字列を逆引きできます。Rustのハンドラはフリー関数です。最も近い対応物は名前付きルートであり、`route("users.show", &[("id", "1")])` はすでに正しいインターフェースです。Rustのハンドラの型の上にアクション文字列によるルーティングを持ち込み直しても、名前付きルートに対して実質的なものは何も足されません。

**`URL::forceScheme()` / `URL::forceRootUrl()`。** Laravelがこれらを公開しているのは、テストのため、そして `X-Forwarded-Proto` を渡さないリバースプロキシの背後にあるサイトのためです。Suprnovaは、どちらの場合も設定で扱います: `APP_URL` が正規のホストとスキームを運び、プロキシ環境では、信頼済みプロキシのミドルウェア（[ミドルウェア](middleware.md)）が `X-Forwarded-*` ヘッダーを読み取り、リクエストがあなたのハンドラへ届く前にそのURLを更新します。`forceScheme` が上書きすべきものは何もありません - スキームが何であるかは、すでに `APP_URL` が述べています。

ここに着地しているのは、利用者が実際に手を伸ばすユーザー向けの形であり、きれいに移せるところではLaravelと同じ形の名前を保っています。削ぎ落としは意図的なものであって、見落としではありません。

## 次のステップ

- [ルーティング](routing.md) - ルートの宣言、名前付け、ルートグループ、リソースルーティング、そしてメソッドごとのマッチング表面の全体
- [レスポンス](responses.md) - `Redirect::route`、`Redirect::signed_route`、`Redirect::back`、そしてURL生成を利用するリダイレクト用ヘルパーの残り
- [ハッシング](hashing.md) - `APP_KEY` のライフサイクル、キーローテーション、そして暗号化と並んでURLの署名を支える共有のキーリング
- [認証フロー](auth-flows.md) - 署名付きURLの実運用での使い手: パスワードリセット、メール認証、remember-meのクッキー
- [リクエスト](requests.md) - `Request::path`、`Request::query`、`Request::route_is`、そしてこの章のあらゆるヘルパーの裏側
