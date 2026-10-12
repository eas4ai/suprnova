# 認可

認証は_「あなたは誰か？」_に答え、認可は_「これを行うことを許されているか？」_に答えます。Suprnovaは、Laravelの形をした `Gate` ファサードと、リソース指向の配線のための `#[policy]` マクロを出荷しており、あらゆるチェックにsyncとasyncの両方のバリアントがあります。そのため、あなたのポリシー本体がDBへのアクセスを必要とする場合でも、単純な構造体フィールドの比較で済む場合でも、同じ表面が機能します。

## クイックスタート

```rust
use suprnova::{Authorizable, Gate};

#[derive(Debug)]
struct User { id: i64, is_admin: bool }
#[derive(Debug)]
struct Post { id: i64, author_id: i64, is_public: bool }

// ユーザーが `user.can(action, &resource)` という使い勝手を選び取れるようにします。
impl Authorizable for User {}

// 1つの権限を配線します:
Gate::define::<User, Post>("update", |user, post| {
    user.is_admin || post.author_id == user.id
});

let alice = User { id: 1, is_admin: false };
let own_post = Post { id: 10, author_id: 1, is_public: false };
let foreign_post = Post { id: 11, author_id: 99, is_public: false };

assert!(alice.can("update", &own_post));
assert!(alice.cannot("update", &foreign_post));

// ハンドラから直接403を返します:
alice.authorize("update", &foreign_post)?;
```

## `Gate` の表面

### 権限を定義する

```rust
// Syncのクロージャ - 直接呼び出され、boxed futureはありません。
Gate::define::<User, Post>("view", |user, post| post.is_public || user.id == post.author_id);

// Asyncのクロージャ - futureは所有されていなければなりません（クロージャの戻り以降に借用は残せません）。
Gate::define_async::<User, Post, _, _>("publish", |user, post| {
    let user_is_admin = user.is_admin;
    let post_id = post.id;
    async move {
        // …DBの検索、RPC呼び出しなど。
        user_is_admin || check_publish_permission(post_id).await
    }
});
```

内部では型消去されており、レジストリは `(action, TypeId<U>, TypeId<R>)` をキーとします。`User` のアクションゲートと `Comment` のアクションゲートは、同じ名前であっても独立して存在します - `Gate::has::<User, Post>("publish")` と `Gate::has::<User, Comment>("publish")` は、それぞれ別々に答えます。

### 権限を確認する

| メソッド | 戻り値 | 用途 |
|---|---|---|
| `Gate::allows(action, &user, &resource)` | `bool` | すばやい分岐 |
| `Gate::denies(action, &user, &resource)` | `bool` | 逆 |
| `Gate::authorize(action, &user, &resource)` | `Result<(), FrameworkError>` | 素の拒否には403。詳細な拒否は自身のステータス/メッセージを運びます（[詳細な判定](#詳細な判定-response-inspect-raw)を参照） - `?` でハンドラをショートサーキットします |
| `Gate::inspect(action, &user, &resource)` | `Response` | 完全な判定: `allowed` + `message` + `code` + HTTPの `status` |
| `Gate::raw(action, &user, &resource)` | `Option<Response>` | `inspect` に似ていますが、`None` は「ルールが定義されていない」という意味です（明示的な拒否とは異なります） |
| `Gate::any(&[...], &user, &resource)` | `bool` | 1つでも許可すればtrue |
| `Gate::none(&[...], &user, &resource)` | `bool` | 1つも許可しなければtrue |
| `Gate::check(&[...], &user, &resource)` | `bool` | すべて許可すればtrue |

すべてのメソッドには `_async` の兄弟があり、syncとasync両方の方法で登録されたゲートに対して機能します。そのため、ハンドラは、そのアクションの裏にあるのがどちらの種類のクロージャなのかを知る必要がありません。

### イントロスペクション

```rust
// 権限が定義されているか?
Gate::has::<User, Post>("publish");  // bool

// どんな権限が存在するか?（アクション名でソートされ、重複排除される）
let all: Vec<String> = Gate::abilities();
```

`abilities()` はリソースの型をまたいで重複を排除します: `User`-on-`Post` と `User`-on-`Comment` の両方に対して `"view"` を登録しても、`"view"` というエントリは1つだけになります。管理者向けのピッカーやInertiaの共有データに便利です。

### 未定義のゲートのセマンティクス

一度も登録されていないアクションに対して `allows` / `denies` / `authorize` を呼び出すと、**デフォルトで拒否されます**。async登録されたゲートに対してsyncのAPIを呼び出した場合も同様です（syncの経路はawaitできません - デフォルトの拒否によって、サイレントに通過させるのではなく `tracing::warn!` を通じてログにバグを表面化させます）。async登録されたゲートは、`_async` の経路からは正しく応答します。

## `#[policy]` によるポリシー

1つのリソース型がいくつもの権限を持つときは、それらをポリシー構造体へまとめ、`#[policy]` に各メソッドをゲートとして登録させてください:

```rust
use suprnova::policy;
use suprnova::authorization::Response;

struct User { id: i64, is_admin: bool }
struct Post { id: i64, author_id: i64, is_public: bool }
struct PostPolicy;

#[policy(User, Post)]
impl PostPolicy {
    // `-> bool` を返すメソッドは、素のallow/denyゲートです。
    fn view_any(_user: &User, _post: &Post) -> bool {
        true // 誰でも投稿を一覧できます
    }
    fn view(user: &User, post: &Post) -> bool {
        post.is_public || post.author_id == user.id || user.is_admin
    }

    // `-> Response` を返すメソッドは、拒否時にメッセージ + HTTPステータスを運べます。
    fn update(user: &User, post: &Post) -> Response {
        if post.author_id == user.id || user.is_admin {
            Response::allow()
        } else {
            Response::deny_with("You may only edit your own posts.")
        }
    }
    fn delete(user: &User, post: &Post) -> Response {
        if user.is_admin {
            Response::allow()
        } else {
            Response::deny_as_not_found() // 管理者以外から投稿を隠す
        }
    }
}
```

各メソッドは1つの `inventory::submit!` になります。`Server::serve` は、起動時に `init_policies()` を通じてインベントリを流し込むため、最初のリクエストが到着する時点では、すべてのアクションが登録済みです（これがブートシーケンスのどこに位置するかは[ブートストラップ](bootstrap.md)を参照してください）。`init_policies()` は `suprnova::authorization::init_policies` にあり、べき等です - サーバーを立ち上げずにポリシーの登録を検証するテストでは、これを手動で呼び出してください。

ポリシーのメソッドは、`(user, resource)` を取るステートレスな関連関数です - Laravelの `update(User $user, Post $post)` と同じ形であり、そこでは `$this` がステートレスなポリシーオブジェクトです。すべてのメソッドが両方の引数を取るのは、ゲートの署名を一様にするためです。`view_any` / `create` は、単にリソース（`_post`）を無視します。書かなかったメソッドは登録されず、未登録のアクションはデフォルトで拒否されます。

### メソッド名 → アクションのマッピング

メソッド名は、そのままアクションの動詞部分として使われ、リソースはkebab-caseにされて末尾に付加されます:

| メソッド | アクション |
|---|---|
| `Post` の `view` | `"view-post"` |
| `Post` の `view_any` | `"view_any-post"` |
| `UserProfile` の `force_delete` | `"force_delete-user-profile"` |

これは、Rustの表面をイディオマティックに保つため、LaravelのcamelCaseなアクション名（`viewAny`、`forceDelete`）から意図的に外れています - どのアクション文字列も、エディタで補完されるメソッド識別子をそのまま映しています。

### 戻り値の型: `bool` か `Response`

ポリシーのメソッドの戻り値の型が、どのように登録されるか - そして拒否が何を運べるか - を選びます:

| 戻り値の型 | 登録の経路 | 拒否がどう表面化するか |
|---|---|---|
| `bool` | `Gate::define` | 素の `403`（`This action is unauthorized.`） |
| `Response` | `Gate::define_with` | `Response` が運ぶメッセージ、コード、HTTPステータス |

単純なyes/noには `bool` を返してください。拒否が理由や403以外のステータスを運ぶべきときは、（`suprnova::authorization::Response` からインポートする）`Response` を返してください - メッセージには `Response::deny_with("…")` を、`404` を返してリソースの存在を隠すには `Response::deny_as_not_found()` を使います。どちらも同じ型消去されたゲートへコンパイルされます（`bool` は素のallow/denyへとラップされます）。それ以外の戻り値の型 - あるいは戻り値がない場合 - はコンパイルエラーです。

## `Authorizable` トレイト

`Gate` の呼び出しに対する、差し替え可能なユーザー側のシュガーです:

```rust
use suprnova::Authorizable;

impl Authorizable for User {}

// Syncのシュガー
if alice.can("update", &post)    { /* ... */ }
if alice.cannot("delete", &post) { /* ... */ }
alice.authorize("update", &post)?;  // 403 on deny

// Asyncのシュガー
if alice.can_async("publish", &post).await    { /* ... */ }
alice.authorize_async("publish", &post).await?;
```

すべてのメソッドには、対応する `Gate` メソッドへ委譲するデフォルト本体があるため、（本体のない）`impl Authorizable for User {}` だけで十分です。ブランケット実装ではなくオプトインなのは、`Gate::allows` に渡せるすべての型が `.can` の主語になることを意図しているわけではないからです - たいていは、あなたのアプリケーションの `User` です。

## 合成パターン

### ルートグループをゲートする

```rust
use suprnova::{group, get, Auth, AuthMiddleware, FrameworkError, Request, Response};

// ミドルウェアが認証済みユーザーを確認し、ハンドラがアクションを認可します。
group!("/posts")
    .middleware(AuthMiddleware::new())
    .routes([
        get!("/{id}/edit", edit_form),
    ]);

async fn edit_form(req: Request) -> Response {
    let user: User = Auth::user_as::<User>()
        .await?
        .ok_or(FrameworkError::Unauthorized)?;
    let id: i64 = req.param("id")?.parse()
        .map_err(|_| FrameworkError::param_parse("id", "i64"))?;
    let post = Post::find(id).await?
        .ok_or_else(|| FrameworkError::not_found("Post"))?;
    user.authorize("update", &post)?;
    // ... 編集フォームを描画する
}
```

### 複数アクションのチェック

「このユーザーがこのリソースに対して行えることをすべて一覧する」ページです:

```rust
let actions = ["view", "update", "delete", "restore", "force_delete"];
let mut allowed = Vec::new();
for action in &actions {
    if user.can(action, &post) {
        allowed.push(*action);
    }
}
// あるいはショートサーキットします:
let can_do_anything = Gate::any(&actions, &user, &post);
let is_locked_out   = Gate::none(&actions, &user, &post);
```

### 複数ゲートによる認可

```rust
// ユーザーがこのリソースに対してこれらのアクションのすべてを行える場合にのみ許可します。
Gate::authorize_async("publish", &user, &post).await?;
if Gate::check_async(&["update", "view"], &user, &post).await {
    // チェックを組み合わせます。
}
```

### リソースルートをゲートする

`Router::resource` の表面が存在するとき、`authorize_resource::<U, R>()` は、慣例的な権限チェックを7つのルートすべてに一度に配線するため、すべてのコントローラーメソッドが認可を忘れずに行うことに依存しなくて済みます:

```rust
Gate::define::<User, Post>("view",   |u, _p| u.is_member);
Gate::define::<User, Post>("create", |u, _p| u.is_author);
Gate::define::<User, Post>("update", |u, _p| u.is_author);
Gate::define::<User, Post>("delete", |u, _p| u.is_admin);

let router: Router = Router::new()
    .resource("posts", PostsCtl)
    .authorize_resource::<User, Post>()   // index/show→view、store→create、…
    .into();
```

権限が拒否されると、ハンドラが実行される前に `403` を返します。未認証のリクエストはフェイルクローズします。アクション → 権限の完全な表は、[ルーティングの章](routing.md)にあります。

## Asyncのセマンティクス

`Gate::define_async` のクロージャは、**所有された** futureを返さなければなりません - 型消去されたレジストリは、`&user` や `&resource` の参照がクロージャの戻りを超えて生き残ることを許しません。必要なフィールドは、それを返す前に `async move {}` ブロックの内部でコピーまたはクローンしてください:

```rust
Gate::define_async::<User, Post, _, _>("publish", |user, post| {
    let user_id = user.id;        // プリミティブをコピー
    let post_id = post.id;
    let admin   = user.is_admin;
    async move {
        // ここには `user` / `post` の参照はありません - キャプチャされたコピーだけです。
        admin || check_can_publish(user_id, post_id).await
    }
});
```

Syncのゲートは、asyncの経路から透過的に機能します（`Gate::allows_async` は `.await` なしでそれらをディスパッチします）。そのため、コードベースは今日はsyncのゲートを登録し、後で個々の権限を呼び出し箇所を変えることなくasyncへ移行できます。

## ロックのポイズニングに対する構え

`Gate` のレジストリは、内部で `RwLock` を使います。ロックがポイズニングされた場合（書き込みガードを保持している間にスレッドがパニックした場合）、レジストリは**安全側に拒否します** - それ以降のすべての `authorize` 呼び出しは、パニックするのではなく `Unauthorized` を返します。登録の呼び出しは `tracing::error!` に記録し、継続します。これは、より広いフレームワークのポリシーと一致します: ポイズニングされたロックは、決してプロセスを中断させません。

## 詳細な判定: `Response`、`inspect`、`raw`

素の `bool` のゲートは、allow/denyしか答えられません。*メッセージ*、機械可読の*コード*、あるいは403以外のHTTP*ステータス*を運ぶ拒否のためには、`define_with`（または `define_async_with`）でゲートを登録し、`Response` を返してください:

```rust
use suprnova::authorization::Response;  // クレートルートでは `GateResponse` として再エクスポートされています

Gate::define_with::<User, Post>("update", |user, post| {
    if post.author_id == user.id {
        Response::allow()
    } else {
        Response::deny_with("You do not own this post.")
    }
});

// リソースの存在を認めるのではなく、隠します:
Gate::define_with::<User, Secret>("view", |user, secret| {
    if user.can_see(secret) {
        Response::allow()
    } else {
        Response::deny_as_not_found()  // 403ではなく404
    }
});
```

`Gate::inspect`（sync）/ `Gate::inspect_async` で、完全な判定を調べられます:

```rust
let decision = Gate::inspect("update", &user, &post);
decision.allowed();   // bool
decision.message();   // Option<&str> - Some("You do not own this post.")
decision.status();    // Option<u16> - ここではNone。deny_as_not_found の後は Some(404)
```

`Response` のコンストラクターはLaravelを映しています: `allow()`、`deny()`、`deny_with(msg)`、`deny_with_status(status, msg)`、`deny_as_not_found()`、それに `with_message` / `with_code` / `with_status` / `as_not_found` のビルダーです。

### 拒否がどのようにエラーになるか

`Gate::authorize` は、`Response::authorize()` を通じて判定を畳み込みます:

| 判定 | `authorize` の結果 |
|---|---|
| 許可 | `Ok(())` |
| 素の `deny()`（メッセージ/コード/ステータスなし） - 設定されていないデフォルトの拒否レスポンスがフォールバックする先 | `FrameworkError::Unauthorized`（403、`"This action is unauthorized."`） |
| 詳細な拒否（メッセージやステータス、あるいはその両方が設定されている） - それを運ぶデフォルトの拒否レスポンスが設定されている場合も含みます | `FrameworkError::Domain { message, status_code }` |

したがって `deny_as_not_found()` は404として、`deny_with_status(422, "…")` は422として、`deny_with("…")` はあなたのメッセージを運ぶ403として表面化します。`code` はinspectした `Response` からは読めますが、`authorize` を通じては**伝わりません** - `FrameworkError` にコードのフィールドはないため、必要なら `inspect()` から読んでください。

拒否がどのステータスに着地するとしても、それはフレームワークのJSONのエラーボディとしてクライアントへ届きます。Inertiaアプリは、[エラーページ](frontend-inertia-responses.md#error-pages)も名指ししておくべきです - それがないと、Inertiaクライアントはそのボディを非Inertiaのレスポンスとして扱い、何も描画せずに全画面のエラーモーダルを表示します。つまり、ロールが合っていないユーザーには「その操作はできません」ではなく、クラッシュが見えることになります。

### `raw`: 「拒否」と「未定義」

`Gate::raw`（および `raw_async`）は `Option<Response>` を返します: `None` は*どのルールも適用されなかった*ことを意味し - `before` フックは発火せず、ゲートは登録されておらず、`after` フックも埋めていない - 明示的な `Some(deny)` とは区別されます。`inspect` はその `None` を、設定されたデフォルトの拒否レスポンス（`Gate::default_denial_response` が別のものを設定していなければ素の拒否）へ正規化します。`raw` は診断のために `None` を保ちます（「このアクションはそもそも統制されているのか?」）。

### デフォルトの拒否レスポンス

Laravelの `Gate::defaultDenialResponse($response)` は、*判定されなかった*拒否の見え方を作り変えます - すべての拒否ではなく、そうでなければ素の `Response::deny()` へフォールバックしていたものだけです。通常は `bootstrap::register()` で、一度だけ設定します:

```rust
use suprnova::authorization::Response;
use suprnova::Gate;

Gate::default_denial_response(Response::deny_as_not_found());
```

この呼び出しの後、2種類の結果が新しい形を受け取ります: 素の `false` - boolのゲート（`define`/`define_async`。`bool` を返す `#[policy]` のメソッドを含みます）から、あるいは `false` と判定した `before`/`after` フックから - と、ほかに何も判定しなかった評価、すなわちフックの意見もない未定義の権限です。これらはすべて、以前は素の `Response::deny()`（403）として表面化していました。今は `default_denial_response` に与えられたもの - 上の例では404 - として表面化します。これは「見てよいとは限らないユーザーからリソースの存在を隠す」という標準的な手（本章の前半の `Secret` の例を参照）を、ゲートごとにではなくアプリケーション全体へ一度だけ適用するものです。

デフォルトが適用されるのは**素の `false` だけ**です。`define_with`（または `define_async_with`）で登録されたゲートは、望む `Response` - `Response::deny_with("…")`、`Response::deny_as_not_found()`、明示的な素の `Response::deny()` さえ - をすでに返しており、そのいずれもが `inspect` を手つかずで通過します。これはLaravel自身の規則を映しています: `Gate::inspect` がデフォルトで置き換えるのは、本当にfalsyなコールバックの結果だけであり、コールバック自身が組み立てた `Response` オブジェクトを置き換えることは決してありません。

## `before` / `after` フック

`Gate::before` は、どのゲートよりも*前*に走るチェックを登録します。最初に `Some(decision)` を返したフックが、すべてをショートサーキットします。典型的な用途はグローバルな上書きです:

```rust
// 管理者は何でもできます。
Gate::before::<User>(|user, _action| user.is_admin.then_some(true));
```

`Gate::after` はゲートの*後*に走ります。Laravelの `??=` のセマンティクスに従い、afterフックは未判定の結果（どのゲートにも一致せず、beforeフックも発火しなかった場合）を**埋める**ことしかできません - すでに生じたallow/denyを上書きすることは決してできません。afterフックはすべて必ず走るため、監査ログの継ぎ目としても働きます:

```rust
Gate::after::<User>(|user, action, decided| {
    audit_log(user.id, action, decided);   // すべての評価を観測します
    None                                    // 記録のみ。結果は変えません
});
```

フックは、リソースではなく**ユーザー型** `U` をキーとします - フックはすべての `(action, U, R)` に対して発火します。リソース固有のロジックはゲートに置いてください。フックはsyncの述語であり、asyncの評価経路にも適用されます。asyncな認可ロジックには、`define_async` / `define_async_with` を使ってください。

### Suprnovaが異なる設計を選んだ理由

Laravelの `Gate::forUser($user)->allows(...)` は、ゲートの*暗黙の*現在ユーザーのリゾルバーを差し替え、次のチェックをそのユーザーとして評価させます。Suprnovaのゲートは呼び出しのたびにユーザーを**明示的に**受け取るため、「別のユーザーとしてチェックする」は単に `Gate::allows(action, &other_user, &resource)` です。差し替えるべき暗黙のリゾルバーは存在しません - 明示的なAPIのほうが厳密に一般的であり、そのため `forUser` は欠けているのではなく冗長なのです。

同じ理屈が、Laravelのクラス名によるポリシーの自動検出にも当てはまります。Suprnovaは登録の時点で、ポリシーのメソッドを型消去された `(action, U, R)` のキーへ結び付けます。そのため、同じメソッド名を持つ `Post` のポリシーと `Comment` のポリシーは、命名規約も検出のためのスキャンもなしに、2つの異なるゲートとして登録されます。

`Gate::default_denial_response` も、1点でLaravelとは異なります: allowの形をした `Response::allow()` を渡すと、受け入れられるのではなくログに記録されて無視されます。Laravelの `defaultDenialResponse` にそうした保護機構はありませんが、これは*拒否*のデフォルトです - allowの形をしたものを受け入れれば、素の `false` を返すすべてのゲートの結果が黙って許可へ反転してしまいます。この表面において、唯一のフェイルオープン方向です。

## 次のステップ

- [認証](authentication.md) - ユーザー側の半分: 認証ガード、`Auth::user()`、`Auth::user_as::<T>()`
- [ブートストラップ](bootstrap.md) - ブートシーケンスのどこで `init_policies()` が実行されるか、そしてbefore/afterフックを登録する方法
- [ミドルウェア](middleware.md) - `AuthMiddleware` とルートレベルの認可を組み合わせること
- [エラー モデル](error-model.md) - ゲートの拒否が、どのように403、404、あるいはカスタムステータスの `FrameworkError::Domain` へと収束するか
- [イベント](events.md) - 監査ログのために `Gate::after` を通じてポリシーの結果をリスンすること
