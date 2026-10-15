# レスポンス

Suprnovaのハンドラは、いずれも `Response` を返します。これは `Result<HttpResponse, HttpResponse>` のエイリアスです。`Ok` 側は成功したレスポンスを運び、`Err` 側はすでに描画済みのエラーレスポンスを運びます。そして `?` 演算子は、その途中で `HttpResponse` への `From` を持つあらゆるエラー型を畳み込みます。この章は、`Ok` 側を組み立てるための実践的なリファレンスです - `HttpResponse` のビルダー、`Redirect` のビルダー、クッキーのAPI、そして `abort_*` によるショートサーキットを扱います。エラー側の話については、[エラー モデル](error-model.md)と[エラーハンドリング](errors.md)を参照してください。

## `HttpResponse` のビルダー

`HttpResponse` は、通信レベルの形をしたレスポンス型です。コンストラクタが妥当なデフォルトを設定し、チェーンできるセッターがそれを上書きします。

### ボディのコンストラクタ

```rust
use suprnova::{HttpResponse, Response};
use serde_json::json;

pub async fn examples() -> Response {
    // text/plain
    let _ = HttpResponse::text("OK");

    // application/json（任意の serde_json::Value）
    let _ = HttpResponse::json(json!({ "ok": true }));

    // text/html; charset=utf-8
    let _ = HttpResponse::html("<h1>Hello</h1>");

    // コンテンツタイプを明示した生のバイト列 - JSON:API のシリアライズや、
    // JSON以外のあらゆるバイトボディに使われます。
    let _ = HttpResponse::bytes_body(b"PNG...".to_vec(), "image/png");

    Ok(HttpResponse::text("done"))
}
```

長く生き続けるレスポンスのために、ストリーミング用のコンストラクタが2つあります:

- `HttpResponse::sse(stream)` - Server-Sent イベントです。`SseEvent` の値の `Stream` を包み、必須の4つのヘッダー（`Content-Type: text/event-stream`、`Cache-Control: no-cache`、`Connection: keep-alive`、`X-Accel-Buffering: no`）を設定し、生成側のストリームが終わるまでコネクションを開いたままにします。[Server-Sent イベント](sse.md)を参照してください。
- `HttpResponse::stream_bytes(stream)` - 汎用のチャンク転送レスポンスです。`Stream<Item = Result<Bytes, Infallible>>` を受け取ります。エラー型が `Infallible` なのは意図的な設計です: レスポンスの途中でトランスポートレベルのエラーをクライアントへ伝える手段はないため、フレームワーク内のどの生成側も、ストリームが終わる前に自分のエラーをストリームの終端メッセージへと変換します。

- `HttpResponse::event_stream(stream, end)` - Laravelの `ResponseFactory::eventStream` です。`sse::StreamedEvent` の `Stream` を包み、各イベントを `event: update`（または独自の名前）と、構成可能な終端フレームとしてフレーム化します。[Server-Sent イベント](sse.md)を参照してください。
- `HttpResponse::stream_json(stream)` - Laravelの `ResponseFactory::streamJson` です。任意の `Serialize` 値の `Stream` を包み、コレクション全体を先にバッファリングする代わりに、増分構築される1つのJSON配列としてフラッシュします。[Server-Sent イベント](sse.md#event-stream-and-stream-json)を参照してください。

### ステータス、ヘッダー、クッキー

どのビルダーも `Self` を返すため、自由にチェーンできます:

```rust
use suprnova::{Cookie, HttpResponse, Response};
use serde_json::json;

pub async fn created() -> Response {
    Ok(HttpResponse::json(json!({ "id": 42 }))
        .status(201)
        .header("X-Resource-Id", "42")
        .cookie(Cookie::new("last_id", "42")))
}
```

| メソッド | 挙動 |
|---|---|
| `.status(code)` | HTTPのステータスを設定します。`100..=599` の範囲外のコードは、通信境界で警告ログとともに500へ格下げされます。 |
| `.header(name, value)` | ヘッダーを1つ追加します。重複は許されます（`Set-Cookie` のセマンティクスに合わせています）。 |
| `.replace_header(name, value)` | それまでの出現をすべて捨てて、1つだけ設定します。 |
| `.with_headers([(k, v), ...])` | 一度に複数を追加します。あらゆる `IntoIterator<Item = (K, V)>` を受け付けます。 |
| `.without_header(name)` | すべての出現を取り除きます（大文字小文字を区別しません）。 |
| `.header_value(name)` | 最初に設定された値を読み返します。テストで役に立ちます。 |
| `.cookie(Cookie)` | クッキーを1つ、`Set-Cookie` として添付します。 |
| `.with_cookies([Cookie, ...])` | 複数のクッキーを添付します。 |
| `.without_cookie(name)` | 削除を予約します（`Cookie::forget(name)` と等価です）。 |

同じチェーンできるセッターは、`ResponseExt` トレイトを通じて `Response`（つまり `Result`）の上でも使えます。そのため、マクロは扱いやすいままです:

```rust
use suprnova::{json_response, Cookie, Response, ResponseExt};

pub async fn list() -> Response {
    json_response!({ "ok": true })
        .status(200)
        .header("X-Total-Count", "42")
        .cookie(Cookie::new("last_query", "list"))
}
```

`ResponseExt` が公開しているのは、`.status`、`.header`、`.with_headers`、`.without_header`、`.cookie`、`.with_cookies`、`.without_cookie` です。

### 通信境界でのバリデーション

`HttpResponse::into_hyper` は、レスポンスをhyperへ引き渡す前に、2つの安全フィルタを実行します:

- **ステータスの範囲。** `100..=599` の範囲外のものは、`tracing::warn!` とともに500へ格下げされます。これによって `AppError::status(700)` のようなタイプミスは境界で捕まり、規格に沿わないコードがそのまま通信に乗ることはありません。
- **ヘッダーのCRLFインジェクション。** ヘッダーの名前と値は、すべてhyper自身の `HeaderName::try_from` / `HeaderValue::try_from` を通じて検証されます。拒否されたヘッダーは警告ログとともに捨てられ、レスポンスはそれを含まない形で組み立てられます。攻撃者が制御する値がヘッダーへ反映される場面（CORSのallow-headers、`X-Forwarded-*`、独自のデバッグ用ヘッダー）でも、レスポンスを分割することはできません。

どちらのフィルタも、成功する経路では何も言いません - 何かがすり抜けようとしたときにだけ、ログに現れます。

## レスポンスのマクロ

よくあるケースのために、`Response` の形をしたマクロが2つあります:

```rust
use suprnova::{json_response, text_response, Response};

pub async fn json_handler() -> Response {
    json_response!({ "users": [{ "id": 1, "name": "Alice" }] })
}

pub async fn text_handler() -> Response {
    text_response!("OK")
}
```

どちらも `Ok(HttpResponse::...)` へ展開されます。ステータス、ヘッダー、クッキーを調整するには、どちらの結果にも `ResponseExt` のセッターをチェーンしてください。

## クッキー

`Cookie::new(name, value)` は、安全なデフォルト - `HttpOnly`、`Secure`、`SameSite=Lax`、`Path=/` - を備えたクッキーを生み出します。クッキーごとに上書きできます:

```rust
use suprnova::Cookie;
use std::time::Duration;

let session = Cookie::new("session_id", "abc123")
    .http_only(true)
    .secure(true)
    .same_site(suprnova::SameSite::Strict)
    .path("/")
    .domain("example.com")
    .max_age(Duration::from_secs(3600))
    .partitioned(true);
```

よくあるパターンは、4つの便利なコンストラクタでカバーされます:

- `Cookie::forget(name)` - 空の値、`Max-Age=0`、パス `/`、ドメインなしです。ログアウト時にこれを使って、そのクッキーを捨てるようブラウザに指示してください。
- `Cookie::forget_with(name, path, domain)` - スコープ付きの形です。ブラウザは、削除クッキーの `Path` と `Domain` が設定時のものと一致した場合にのみクッキーを削除するため、`Path=/admin` または `Domain=.example.com` で設定されたクッキーは、単純な `forget` では残ります。どちらかの引数に `None` を渡せばデフォルトを維持できます。
- `Cookie::forever(name, value)` - 5年間の `Max-Age` です。
- `Cookie::encrypted(name, plaintext)` - 論理的なクッキー名に束縛されたAADを持つAES-256-GCM暗号文を書き込みます。同じ名前を使って `Cookie::read_encrypted_for(name, wire)` で読み取ってください。`Cookie::read_encrypted(wire)` は非コンテキストの非推奨v1リーダーであり、現在の `Cookie::encrypted` 出力を復号できません。これはv1フォールバックとともに1.4.0で削除される予定です。起動時に `APP_KEY` が設定されている必要があります。[暗号化](encryption.md)を参照してください。

ヘッダーへのシリアライズは、RFC 6265でいう正当なcookie-octetではないバイトを、すべての制御文字も含めてパーセントエンコードします。クッキーの名前や値に含まれるCRLFは、伝播せずにエンコードされます - クッキーを経由したヘッダーインジェクションは、シリアライザの段階で塞がれています。

複数のクッキーを一度に削除する - 通常のログアウトの形 - には、`without_cookies` を使います。これは `HttpResponse`、`Response` の `ResponseExt`、そして両方のリダイレクトビルダーで利用できます:

```rust
use suprnova::{HttpResponse, Redirect};

let _ = HttpResponse::text("bye").without_cookies(["session", "remember"]);
let _: suprnova::Response = Redirect::to("/login")
    .without_cookies(["session", "remember"])
    .into();
```

リダイレクトでは、削除は遷移先ではなく302自体に乗るため、ブラウザは `Location` を追うまでにすでに削除を完了しています。

### 後で使うクッキーをキューに入れる

レスポンスを組み立てていないコードでも、クッキーを設定する必要が生じることがあります - イベントに反応するリスナー、ハンドラより前に実行されるミドルウェア、スコープ内に `HttpResponse` がない `App::bind` サービスなどです。`Cookie::queue` はLaravelの `Cookie::queue()` です。リクエスト単位のジャーへクッキーを退避し、`SessionMiddleware` がセッション用クッキーの直後に、送信されるレスポンスへ排出します。

```rust
use suprnova::Cookie;

Cookie::queue(Cookie::new("theme", "dark"));

// キューに入っているものを調べる。
let queued = Cookie::queued("theme");

// レスポンスが出ていく前に取り除く。
Cookie::unqueue("theme");

// 値の代わりに削除をキューに入れる - `forget_with` と組み合わせられる。
Cookie::expire("theme", Some("/app"), None);
```

ジャーはタスクローカルで、リクエストごとに新しく空になります - あるリクエストでキューに入れたものが次で見えることはなく、キューに入れられた値が排出されない場合（ルートのチェーンに `SessionMiddleware` がない場合）もパニックせず破棄されます。キューに入れたクッキーはハンドラが返すものなら何にでも付き、リダイレクトも含まれます: クッキーをキューに入れてから `Redirect::to(...)` を返すハンドラでも、3xxレスポンスに `Set-Cookie` ヘッダーが残ります。また、リクエスト途中の内部失敗に対して `SessionMiddleware` 自身が組み立てる500にも付きます - 読み取れない既存セッション、失敗したセッション書き込み、セッションクッキーの暗号化失敗などです - なぜなら、キューに入れたクッキーは、すでに別の場所でコミットされた副作用（例えば、すでに書き込まれたremember-meトークンの行）を表すことがあり、そのため失敗を報告するレスポンスにもそれを載せるからです。パニックを越えて生き残ることは**ありません** - `SessionMiddleware` の排出コードはハンドラが正常に返った後に実行され、捕捉されたパニックはミドルウェアチェーン全体の外側、Laravel自身のキュー済みクッキーが捕捉されない例外で失われるのと同じ地点で、500に変換されます。

### Suprnovaが異なる設計を選んだ理由

Laravelの `CookieJar` は名前*と*パスでキューをキー付けするため、異なるパスにある同名の2つのクッキーを独立してキューに入れられます。Suprnovaはジャーを名前だけでキー付けします。すでにキューに入っている名前の下で2つ目のクッキーをキューに入れると、2つ目の `Set-Cookie` 行を追加するのではなく、1つ目を置き換えます。これは一般的なケース - 1つの呼び出し箇所が特定のクッキー名を所有するケース - を、Laravelの実装が必要とするパスキーの追加ルックアップなしでカバーします。


## リダイレクト

`Redirect` は、Laravelのリダイレクタの表面をすべてカバーしています。どのバリアントも `From<Redirect> for Response` を実装しているため、慣用的な書き方は `Redirect::...().into()` です。

### リダイレクト先

```rust
use suprnova::{Redirect, redirect_to};

// 明示的な URL またはパス
let _ = Redirect::to("/dashboard");

// 同じもの。少しだけ短いフリー関数
let _ = redirect_to("/dashboard");

// 名前付きルート（RedirectRouteBuilder を返します）
let _ = Redirect::route("users.show").with("id", "42");

// 明示的な外部 URL - `to` と同じですが、この名前は、オープンリダイレクトの
// 監査に向けて「これはサイト外へ出る」と示します
let _ = Redirect::away("https://external.example.com");

// ページを再読み込みします（直前のURLをセッションから読み取り、セッションの
// スコープがアクティブでなければ "/" に落ちます）
let _ = Redirect::refresh();

// 同じですが、スコープがアクティブでないときに明示的な Request を受け取ります
// let _ = Redirect::refresh_for(&request);

// セッションの previous_url。セッションがスコープにないときのフォールバック付き
let _ = Redirect::back("/login");

// セッションに保存された intended URL。読み取り時に消費され、フォールバック付き
let _ = Redirect::intended("/home");

// ゲストリダイレクト: 現在のリクエストURLを "intended" として退避し、
// ユーザーをログインページへ送ります
// let _ = Redirect::guest(&request, "/login");
```

`Redirect::back`、`Redirect::intended`、`Redirect::guest`、`Redirect::refresh` は、いずれもセッションと連携します。セッションのスコープがない場合、これらは黙ってそれぞれのデフォルトへ落ちます - 部分的なテストのセットアップでは便利です。[セッション](session.md)を参照してください。
`Redirect::back` のターゲットであるセッションに記録された以前のURLは、決してそのまま信頼されません。セッションミドルウェアは、そもそもルート相対の同一オリジンURLだけを記録します（`//` または `/\` で始まるパスや、どこかにASCII制御バイトを含むパスは決して保存されません）。同じチェックが読み取りのたびに再び実行されるため、`back` は、通常とは異なるパスでアプリへ到達したリクエストや、このガードが存在する前に書かれたセッションクッキーによっても、オリジン外へ向けられません。完全なルールについては、[セッション](session.md#other-operations)を参照してください。


### 名前付きルートのバリデーション

`redirect!` という手続きマクロは、ルート名をコンパイル時にバリデーションし、`Redirect::route(name)` へ展開されます:

```rust
use suprnova::{redirect, Response};

pub async fn store() -> Response {
    // "users.index" が登録済みのルート名でなければコンパイルが失敗します。
    // エラーメッセージは、利用可能なルートを列挙し、近い候補を提案します。
    redirect!("users.index").into()
}
```

### ステータスコード

```rust
use suprnova::Redirect;

let _ = Redirect::to("/x").permanent();      // 301
let _ = Redirect::to("/x").status(303);      // 303, 307, 308, ...
```

デフォルトは302です。

### フラッシュデータ

Redirectのビルダーは、自分専用のフラッシュバッグを持っています。`Response` へ変換される時点で、そのバッグの中身は現在のセッションへ流し込まれ、ちょうどもう1回のリクエストだけを生き延びます:

```rust
use suprnova::Redirect;

let _ = Redirect::back("/users/new")
    .with("status", "User created")            // 単一のキー/値のペア
    .with_input([                              // フォームを再投入する
        ("email", "shawn@example.com"),
        ("name", "Shawn"),
    ])
    .with_errors([                             // デフォルトのエラーバッグ
        ("email", "Must be unique"),
    ])
    .with_errors_bag("login", [                // 名前付きのエラーバッグ
        ("password", "Required"),
    ]);
```

受け取り側のページは、これらを `session.get(...)`（`with` の分）、`session.get_old_input(...)`（`with_input` の分）、そして `session.pull_errors_flash()` が汲み出すバッグのマップ（`with_errors` / `with_errors_bag` の分）を通じて読み返します。Inertiaの層は、このエラーのフラッシュを自動的に消費します - どのInertiaレスポンスでも `errors` プロップはセッションから種を与えられるため、`Redirect::back().with_errors(...)` は追加の配線なしに、遷移先でメッセージを表に出します。複数のフォームを持つページでは、`X-Inertia-Error-Bag` リクエストヘッダーが、そのプロップを名前付きのバッグの下にスコープします。

注意すべき点として、`RedirectRouteBuilder`（`Redirect::route` と `redirect!` が返すもの）では、`.with(key, value)` が設定するのはフラッシュのエントリではなく**ルートパラメータ**です - そちらでは `.flash(key, value)` を使ってください:

```rust
use suprnova::redirect;

let _ = redirect!("users.show")
    .with("id", "42")                          // ルートパラメータを設定
    .flash("status", "Updated");               // セッションのフラッシュ
```

### クッキー、ヘッダー、フラグメント

```rust
use suprnova::{Cookie, Redirect};

let _ = Redirect::route("billing.show")
    .with_cookies([Cookie::new("welcome", "yes")])
    .with_headers([("X-Trace", "abc")])
    .with_fragment("invoices")                 // #invoices を付け足す
    .without_fragment();                       // あるいは以前のフラグメントを取り除く
```

`with_fragment` は、先頭に `#` が付いていてもいなくても、フラグメントを受け付けます。`without_fragment` の後に `with_fragment` を呼べば、再び付け直されます。

### リダイレクトをまたいでフラグメントを保つ

遷移先が*元の*URLのハッシュを保つべきInertiaアプリでは、`preserve_fragment` を使ってください:

```rust
use suprnova::Redirect;

let _ = Redirect::route("dashboard.index").preserve_fragment();
```

変換の時点で、これは `_inertia.preserve_fragment = true` をセッションへフラッシュします。次のInertiaレスポンスがそのフラグを読み取り、自身のページオブジェクトに `preserveFragment: true` を出力します。セッションのスコープがなければ、フラグは黙って捨てられます。

### 署名付きリダイレクト

名前付きルートへの使い捨てのリダイレクト（パスワードリセット、メール認証、ダウンロードリンク）のために、2つのビルダーがURL署名の表面を包んでいます:

```rust
use suprnova::Redirect;

let r = Redirect::signed_route("downloads.show", &[("id", "42")])?;
let r = Redirect::temporary_signed_route(
    "downloads.show",
    &[("id", "42")],
    1_700_000_000, // expires_at_epoch_seconds
)?;
```

どちらも `Result<Redirect, FrameworkError>` を返します - `Redirect` は `Response` へきれいに変換されるため、エラーは `?` で伝播させてください。署名まわりの表面については[URL 生成](urls.md)を参照してください。

### 意図された遷移先URLを保存する

`Redirect::set_intended_url` は、リダイレクトを実行せずに、セッションの意図された遷移先を書き込みます - 典型的には `/login` へリダイレクトする前の認証ミドルウェアから呼び出され、後から `Redirect::intended` が本来リクエストされていたURLを取り戻せるようにします:

```rust
suprnova::Redirect::set_intended_url("/admin/users");
```

## ハンドラから中断する

3つのフリー関数が、指定したステータスでハンドラをショートサーキットさせます。これらは `Result<(), FrameworkError>` を返すため、`?` と組み合わせてください:

```rust
use suprnova::{abort_if, abort_unless, abort_with, json_response, Request, Response};

pub async fn show(req: Request) -> Response {
    abort_unless(Auth::user().await?.is_some(), 401, "must be logged in")?;
    abort_if(req.param("id")? == "0", 404, "User not found")?;
    abort_with(503, "scheduled maintenance")?;
    json_response!({ "ok": true })
}
```

背後にあるエラーは `FrameworkError::Domain { message, status_code }` であるため、他のあらゆるエラー経路と同じJSONのボディ形状と5xxのサニタイズ規則を通じて描画されます。範囲外のステータスコードは、レスポンスレンダラーによって500へ補正されます。変換の契約の全体像については、[エラー モデル](error-model.md)を参照してください。

## エラーを直接返す

`Response` は `Result<HttpResponse, HttpResponse>` であるため、`Err` 側を直接返せます - レスポンスの形がすでに特定のJSONボディとして決まっていて、それをそのまま通信に乗せたい場合に便利です:

```rust
use suprnova::{HttpResponse, Response};
use serde_json::json;

pub async fn legacy_lookup() -> Response {
    Err(HttpResponse::json(json!({
        "error": "deprecated endpoint",
    })).status(410))
}
```

それより豊かなもの - 型付きのドメインエラー、バリデーション、可観測性 - が必要なら、[エラー モデル](error-model.md)の表面（`AppError`、`FrameworkError`、`#[domain_error]`）を使ってください。

## クイックリファレンス

| やりたいこと | 使うもの |
|---|---|
| JSONのレスポンス | `HttpResponse::json(v)` または `json_response!({...})` |
| テキストのレスポンス | `HttpResponse::text(s)` または `text_response!(s)` |
| HTMLのレスポンス | `HttpResponse::html(s)` |
| 生のバイト列 + content-type | `HttpResponse::bytes_body(b, "image/png")` |
| Server-Sent イベント | `HttpResponse::sse(stream)` - [SSE](sse.md)を参照 |
| チャンク転送のストリーム | `HttpResponse::stream_bytes(stream)` |
| ステータスを設定する | `.status(code)` |
| ヘッダーを追加する | `.header(k, v)` / `.with_headers([...])` |
| ヘッダーを取り除く | `.without_header(name)` |
| クッキーを添付する | `.cookie(c)` / `.with_cookies([...])` |
| クッキーを忘れさせる | `.without_cookie(name)` / `.without_cookies([...])` |
| パス/ドメインのスコープ付きクッキーを忘れさせる | `Cookie::forget_with(name, Some("/admin"), Some("example.com"))` |
| 次のレスポンス用にクッキーをキューに入れる | `Cookie::queue(c)` |
| キューに入れたクッキーを検索する | `Cookie::queued(name)` |
| キューからクッキーを取り除く | `Cookie::unqueue(name)` |
| 削除クッキーをキューに入れる | `Cookie::expire(name, path, domain)` |
| 単純なリダイレクト | `Redirect::to(path).into()` または `redirect_to(path).into()` |
| 名前付きルートへのリダイレクト | `redirect!("name").into()` または `Redirect::route("name")` |
| 直前のページへのリダイレクト | `Redirect::back(fallback)` |
| 意図された遷移先へのリダイレクト | `Redirect::intended(default)` |
| ゲスト向けリダイレクト（意図された遷移先を退避します） | `Redirect::guest(&req, login)` |
| 意図された遷移先を設定する | `Redirect::set_intended_url(url)` |
| 外部のURL | `Redirect::away(url)` |
| 現在のページを読み込み直す | `Redirect::refresh()` / `Redirect::refresh_for(&req)` |
| 署名付きルートへのリダイレクト | `Redirect::signed_route(name, &[(k, v)])?` |
| リダイレクトのルートパラメータ | `.with("key", "value")` |
| リダイレクトのクエリパラメータ | `.query("key", "value")` |
| フラッシュデータ | `.with(key, value)`（`RedirectRouteBuilder` では `.flash`） |
| フラッシュされる入力 | `.with_input([(k, v), ...])` |
| フラッシュされるエラー | `.with_errors([(k, msg), ...])` |
| 名前付きのエラーバッグ | `.with_errors_bag(bag, [(k, msg)])` |
| フラグメントを付け足す | `.with_fragment("section")` |
| フラグメントを取り除く | `.without_fragment()` |
| フラグメントを保つ（Inertia） | `.preserve_fragment()` |
| 恒久的なリダイレクト | `.permanent()`（301） |
| リダイレクトのステータスを指定する | `.status(303)` |
| 早期に中断する | `abort_with(code, msg)?`、`abort_if(cond, code, msg)?`、`abort_unless(cond, code, msg)?` |

## 次のステップ

- [エラー モデル](error-model.md) - `FrameworkError`、`AppError`、`HttpError`、そしてあらゆるエラーを `HttpResponse` へ描画する唯一の変換
- [エラーハンドリング](errors.md) - `?`、`AppError`、独自のドメインエラーのための、実践的なハンドラのパターン
- [Server-Sent イベント](sse.md) - `sse(...)` のレスポンスを組み立て、それを消費すること
- [URL 生成](urls.md) - 署名付きURL、名前付きルートの解決、`Redirect::signed_route` の背後にある表面
- [セッション](session.md) - フラッシュデータ、意図された遷移先URL、`Redirect::with`/`with_input`/`with_errors` が書き込むバッグ
