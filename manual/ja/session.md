# セッション

セッションとは、同じブラウザからの複数のリクエストをまたいで生き延びる、ユーザーごとのキー/値のバッグです。Suprnovaは、データベースを裏付けとするドライバーを最初から出荷し、それを `SessionMiddleware` で配線し、現在のセッションを2つのフリー関数 - 読み取りには `session()`、書き込みには `session_mut()` - を通じて公開します。ある値が1回のリクエストより長く生きるべきだけれども、URLやJWTが運ぶべきものではない、というときにはいつでも使ってください。

## リクエストからセッションがどう見えるか

`SessionMiddleware` はすべてのリクエストで実行され、次の5つのことを順に行います:

1. `suprnova_session` クッキー（AES-256-GCMで暗号化されています）から、セッションIDと、最後に成功したアクティビティ更新のタイムスタンプを読み取ります。改ざんされたクッキー、復号できないクッキー、形式が不正なクッキーは、存在しないものとして扱われます。
2. ストアから `SessionData` を読み込むのは、有効なクッキーがセッションを名指ししている場合だけです。クッキーを持たないリクエストは、まっさらなメモリ上のセッションから始まり、確実に空振りするデータベースアクセスを発行することはありません。対応する行がもう存在しないクッキーは、空の行を作り直すことなく消去されます。ストアの読み取りエラーは `warn!` を記録し、状態を持たないリクエストはそのまま続行させますが、その後にハンドラが変更を加えようとすると、未知の保存済み状態を上書きするのではなく、安全側に倒れて失敗します。
3. フラッシュデータを1つ古くします: `_flash.old.*` は捨てられ、`_flash.new.*` は `_flash.old.*` へ改名されます。この手順の後では、直前のリクエストがフラッシュしたものが読み取れます。このリクエストがフラッシュするものは、次回に読み取れます。
4. ハンドラが動いている間、セッションをタスクローカルのスロットへ束縛します。`session()` と `session_mut()` は、そのスロットを引きます。
5. ハンドラが戻った後、変更のあったセッションの状態、または頻度を抑えたスライディング有効期限の更新を永続化し、書き込みが成功した場合にのみ差し替え用の暗号化クッキーを添付し、保留されている帯域外のクッキー（例えば、ローテーションされたばかりのremember-meクッキー）を吐き出します。何も変更のないクッキーなしのリクエストは、セッションストアへのI/Oを一切行わず、セッションクッキーも受け取りません。

手順5には、取り出しておく価値のある安全上の保証が1つあります: **このリクエストでセッションが変更され、なおかつストアへの書き込みが失敗した場合、レスポンスは500に置き換えられます。** ハンドラの成功をそのまま返すことは、データベースが一度も記録しなかった状態に対するクッキーをクライアントへ手渡すことを意味します - 次のリクエストは空のセッションを読み込み、その変更（ログイン、CSRFのローテーション、フラッシュ）は黙って消えてしまいます。読み取りしか行わないリクエストで、期限の来た `last_activity` の更新だけが失敗した場合は、`warn!` を記録し、既存のクッキーを保ったまま通過します。

## セッションを読む

```rust
use suprnova::session::session;

if let Some(s) = session() {
    let user_id: Option<String> = s.get("preferred_username");
    if s.has("cart") {
        // ...
    }
    if s.missing("locale") {
        // はじめての訪問
    }
}
```

`session()` は、現在の `SessionData` を複製します。リクエストのスコープの外（ミドルウェアをインストールしていないユニットテスト、CLIのサブコマンドなど）では `None` を返します。型付きの値が欲しい場合、`get::<T>` は背後のJSONからデシリアライズします。キーがない場合や型が合わない場合に返るのは `None` であって、パニックではありません。

## セッションに書き込む

`session_mut` は、`&mut SessionData` を受け取るクロージャを取ります:

```rust
use suprnova::session::session_mut;

session_mut(|s| {
    s.put("locale", "en");
    s.put("preferences", serde_json::json!({
        "theme": "dark",
        "notifications": true,
    }));
    s.forget("legacy_key");
});
```

このクロージャは同期です - 背後のロックのガードは、どの `.await` よりも前に破棄されるため、これは非同期ハンドラの内部でも、中断をまたいでロックを保持することなく組み合わせられます。シリアライズするものは `Serialize` を実装していなければならず、`get` でのデシリアライズには `DeserializeOwned` が必要です。

（ガードを返すのではなく）クロージャの形にしているのは意図的です。Tokioのfutureは、開始したのとは別のワーカースレッドで再開されうるため、セッションは `task_local!` のスロットに置かれ、スコープに縛られたクリティカルセクションを通じて借用されなければなりません。`|s|` という形はその境界を明示的にし、`.await` をまたいでミューテックスのガードを保持してしまう事故を防ぎます。

## フラッシュデータ

フラッシュの値は、続く**1回**のリクエストでだけ見え、その後は消えます。よくあるパターンはこうです: コントローラーがフラッシュを書き込み、リダイレクトを返し、次のページがそのフラッシュを描画します。

```rust
use suprnova::session::session_mut;

session_mut(|s| s.flash("status", "Profile updated."));
```

次のリクエストでは:

```rust
use suprnova::session::session_mut;

let status: Option<String> = session_mut(|s| s.get_flash("status"));
```

`get_flash` は、値を返すのと同時にそれを取り除きます。消費せずに読むバリアントが欲しい場合は `get::<String>("_flash.old.status")` を使ってください。ただし、コントローラーが普通に欲しいのは、消費する側の形です。

Laravel由来のフラッシュの表面は、すべて揃っています:

- `flash(key, value)` - 次のリクエストのために書き込みます
- `now(key, value)` - 現在のリクエストにだけ書き込みます
- `reflash()` - 今見えているものをすべて、もう1回分だけ再フラッシュします
- `keep(&["k1", "k2"])` - 特定の一部だけを再フラッシュします
- `flash_input(map)` / `old_input()` / `get_old_input(key)` - `Redirect::with_input` / `old()` のヘルパーが使う、フォーム入力のバッグです

## 再生成と無効化

認証情報が変わった後（ログイン、パスワードリセット、2FAの通過）には、セッションIDをローテーションして、変更より前に固定されたIDが有効でなくなるようにします:

```rust
use suprnova::session::{regenerate_session_id, regenerate_csrf_token};

regenerate_session_id();        // 新しいID、データは同じ
regenerate_csrf_token();        // 新しいCSRFトークン、IDとデータは同じ
```

セッションを丸ごと消去するには（ログアウト）:

```rust
use suprnova::session::invalidate_session;

invalidate_session();           // データを消去し、新しいCSRFトークンを発行
```

あるユーザーのすべてのセッションを失効させる必要のあるセキュリティ上の事象（別の場所でのパスワードリセット、アカウントの復旧、管理者による強制ログアウト）には:

```rust
use suprnova::session::destroy_all_for_user;

let rows = destroy_all_for_user("user-42").await?;
tracing::info!(revoked = rows, "all sessions destroyed");
```

これは、フレームワークのデフォルトである `DatabaseSessionDriver` に対する `SessionStore::destroy_for_user` を包んだものです。独自のストアを束縛している場合は、そちらの `destroy_for_user` を直接呼んでください。

## 認証のヘルパー

`auth_user_id()` は、現在認証されているユーザーのIDを返します（まずリクエストスコープの認証状態を参照し、なければ永続化されたセッションのフィールドへフォールバックします）:

```rust
use suprnova::session::{auth_user_id, is_authenticated};

if is_authenticated() {
    let uid = auth_user_id().expect("just checked");
    // ...
}
```

通常、認証は [Auth](authentication.md) ファサード - `Auth::login`、`Auth::logout`、`Auth::user()` - を通じて動かします。セッションのヘルパーは、それらのファサードが乗っている低レベルの層です。生のセッションを調べる必要があるときや、自分で認証ガードを実装するときに、これらへ手を伸ばしてください。

## その他の操作

`SessionData` のAPIは、Laravelの `Store` の表面を反映しています:

| メソッド | 何をするか |
|---|---|
| `get::<T>(key)` | 型付きの読み取り |
| `put(key, value)` | 型付きの書き込み |
| `forget(key)` | キーを1つ取り除く |
| `forget_many(&[..])` | 複数のキーを取り除く |
| `flush()` | データをすべて消去する（IDは保つ） |
| `has(key)` / `missing(key)` | 存在するかどうかの確認 |
| `has_any(&[..])` / `has_all(&[..])` | まとめて存在を確認 |
| `all()` | 背後のマップを借用する |
| `only(&[..])` / `except(&[..])` | 絞り込んだ複製 |
| `pull::<T>(key)` | 取得と削除を一度に行う |
| `push(key, value)` | 配列の値へ追加する |
| `increment(key, n)` / `decrement(key, n)` | 整数のカウンタ |
| `remember::<T>(key, \|\| default())` | 取得し、なければ計算して書き込む |
| `replace(&[(k, v), ..])` | 全消去してからまとめて書き込む |
| `put_many(&[(k, v), ..])` | まとめて書き込んでマージする |
| `previous_url()` / `set_previous_url(url)` | `Redirect::back` が読むもの |
| `password_confirmed()` / `password_confirmed_at()` | 「ユーザーがたった今パスワードを確認した」というタイムスタンプ |

変更を伴う操作では `session_mut` の内部で、読み取りでは `session()` で、これらに手を伸ばしてください。`previous_url` のスロットは、成功したGETのHTMLレスポンスに対してミドルウェアが自動的に埋めます。ミドルウェアは、ルート相対かつ同一オリジンのURLだけを記録します。`//` または `/\` で始まるリクエストパス（どちらもブラウザーではプロトコル相対として読まれます）、または任意の位置にASCII制御バイトを持つパス（`TAB` や改行は、ブラウザーのURLパーサーが取り除くとルート相対に見える値をその2形態のどちらかへ変えられます）は、決して保存されません。`previous_url()` も読み取りごとに同じ規則を再確認するため、この書き込み時ガードより前の古いリリースが書いた値は、信頼されず欠落として読み返されます。どちらの場合も、このスロットが保持した値から `Redirect::back()`、`Redirect::refresh()`、および `url::previous()` がアプリケーション外の `Location` へ解決されることはありません。

## 設定

セッションの設定は環境変数で行います - `SessionConfig::from_env` が起動時にそれらを読み取ります:

```env
# 分単位の有効期間。行のTTLとクッキーの Max-Age の両方を駆動します。
SESSION_LIFETIME=120

# スライディング有効期限の書き込み間隔の最小秒数（デフォルトは5分）。
# 実行時の強制により、この値はセッションの有効期間より下に抑えられます。
SESSION_TOUCH_INTERVAL=300

# 監督付きで期限切れ行を回収する間隔の秒数（デフォルトは1時間）。
SESSION_GC_INTERVAL=3600

# クライアント側でのクッキー名。
SESSION_COOKIE=suprnova_session

# クッキーの属性
SESSION_SECURE=true          # HTTPS を必須にします。デフォルトは true
SESSION_PATH=/
SESSION_DOMAIN=.example.com  # 任意。未設定ならホスト限定
SESSION_SAME_SITE=Lax        # Lax | Strict | None
SESSION_COOKIE_PREFIX=       # 空 | __Secure- | __Host-
SESSION_PARTITIONED=false    # CHIPS へのオプトイン
SESSION_EXPIRE_ON_CLOSE=false # true にすると Max-Age を省き、ブラウザは閉じたときに破棄します

# セッションストア用の名前付きDB接続（任意）
SESSION_CONNECTION=sessions

# リメンバーミーのトークン / クッキーの有効期間、分単位（デフォルトは30日）
REMEMBER_LIFETIME=43200
```

取り上げておく価値のあるデフォルトが、いくつかあります:

- **`SESSION_SECURE` のデフォルトは `true` です。** 素のHTTPで送られるセッションは認証情報の漏洩につながる危険があるため、secureのフラグはデフォルトで有効になっています。HTTPでのローカル開発では、手元の `.env` に `SESSION_SECURE=false` を設定してください。
- **`HttpOnly` は常に有効です。** これを無効にするつまみはありません - セッションクッキーをJavaScriptへ晒すことは、XSSに対する第一の防御を放棄することであり、今どきそれを望む正当な理由はありません。
- **`SameSite` のデフォルトは `Lax` です。** `Strict` は、クロスサイトのGETによる遷移のほとんど（メールからの戻りリンクも含みます）でセッションを遮断します。普通に正しい答えは `Lax` のほうです。


### クッキー名プレフィックスの強化

`SESSION_COOKIE_PREFIX=__Host-` は、ブラウザーにセッションおよびremember-meクッキーをホストにロックさせます。`__Host-` クッキーは `Secure`、`Path=/`、および `Domain` の省略が必要であり、`__Secure-` クッキーは `Secure` が必要です。Suprnovaは最終クッキー名からレンダリング時にこれらの規則を強制するため、ビルダー順序とキュー済みクッキーも同じ保護を受けます。

`Config::init` は起動時にプレフィックス、`SESSION_DOMAIN`、および `SESSION_PATH` を検証し、組み合わせが不正なら提供開始前に失敗します。レンダリング時の強制は両プレフィックスに `Secure` を引き続き強制し、`__Host-` のパスを `/` に書き換えます。`__Host-` の `Domain` は要求されたスコープを狭めるため警告を記録して削除します。不正なプレフィックス付きクッキーはブラウザーにサイレントに破棄されるため、デプロイ前に起動診断を確認してください。

ローカルHTTP開発ではプレフィックスを空のままにし、ローカル環境だけで `SESSION_SECURE=false` を設定してください。本番ではHTTPSをデプロイし、`SESSION_SECURE=true` を保ち、`SESSION_COOKIE_PREFIX=__Host-` を使い、`SESSION_PATH=/` を保ち、`SESSION_DOMAIN` は設定しないでください。

デプロイチェックリスト:

1. ヘルスチェックと最初のリダイレクトを含め、公開オリジンがHTTPSであることを確認します。
2. `SESSION_COOKIE_PREFIX=__Host-`、`SESSION_SECURE=true`、`SESSION_PATH=/` を設定します。
3. `SESSION_DOMAIN` を削除します。起動バリデーターは `__Host-` と組み合わせると拒否します。
4. 最初の `Set-Cookie` レスポンスに `__Host-suprnova_session`、`Secure`、`Path=/` があり、`Domain` がないことを確認します。

### Suprnovaが異なる設計を選んだ理由

Laravelは、セッション設定でファーストクラスのクッキープレフィックスのつまみを公開していません。Suprnovaは、失敗時にブラウザーがサイレントに動作するため、起動時検証を伴う設定値としてプレフィックスを公開します。不正なクッキーは、アプリケーションコードがセッション失敗を報告できる前に破棄されます。
プログラムから設定する場合は、フルーエントなビルダーを使ってください:

```rust
use std::time::Duration;
use suprnova::SessionConfig;

let config = SessionConfig::new()
    .lifetime(Duration::from_secs(60 * 60))      // 1時間
    .touch_interval(Duration::from_secs(5 * 60))
    .gc_interval(Duration::from_secs(60 * 60))
    .cookie_name("myapp_session")
    .secure(true)
    .domain(".example.com")
    .remember_lifetime(Duration::from_secs(30 * 24 * 60 * 60));
```

`SessionConfig` は `#[non_exhaustive]` です。プログラムによる設定でプレフィックスが必要な場合は、デフォルトを使い公開フィールドを代入してください:

```rust
use suprnova::{CookiePrefix, SessionConfig};

let mut config = SessionConfig::default();
config.cookie_prefix = CookiePrefix::Host;
```

## 配線する

`SessionMiddleware` は、アプリケーションのbootstrapでグローバルミドルウェアとしてインストールされます。ミドルウェアの順序は重要です: CSRFはセッションごとのトークンを読み取るため、セッションは [CSRF](csrf.md) より前に来なければなりません。

```rust
use std::sync::Arc;
use suprnova::{global_middleware, CsrfMiddleware, SessionConfig, SessionMiddleware};

pub async fn bootstrap() {
    let config = SessionConfig::from_env();

    // `install` は、設定されたGCスーパーバイザーも併せて登録します。
    // GCのスケジューリングを `Schedule` で自分で行いたい場合は、
    // `SessionMiddleware::new(config)` を使ってください。
    global_middleware!(SessionMiddleware::install(config).await);

    global_middleware!(CsrfMiddleware::new());
}
```

`SessionMiddleware::install` は、`SESSION_GC_INTERVAL`（デフォルトでは1時間に1回）ごとに `gc()` を呼ぶ、[スーパーバイザー管理下の](supervisors.md)gcタスクを登録します。バリアントの `install_with_gc(config, interval).await` は独自の間隔を受け取ります。`new(config)` はgcタスクを省きます（`gc()` を [Schedule](scheduling.md) のエントリから呼びたい場合に便利です）。スーパーバイザー管理下のタスクはフレームワークのシャットダウン時のドレインに参加するため、gcのループは強制的に中断されるのではなく、`Ctrl-C` / `SIGTERM` できれいに終了します。

保護された運用向けのエンドポイントは、sessionsテーブルに問い合わせることなく、回収タスクの状態を公開できます:

```rust
use suprnova::session::session_gc_metrics;

let metrics = session_gc_metrics();
tracing::info!(
    runs = metrics.runs,
    failures = metrics.failures,
    removed_rows = metrics.removed_rows,
    last_success = metrics.last_success_unix_seconds,
    "session collector status"
);
```

データベース以外のストアを使うには - テストのため、あるいは自分で書くRedisを裏付けとするドライバーのため - `SessionStore` を実装し、`with_store` を通じてそれを渡してください:

```rust
use std::sync::Arc;
use suprnova::{SessionConfig, SessionMiddleware, SessionStore};

let store: Arc<dyn SessionStore> = Arc::new(MyRedisStore::new());
let mw = SessionMiddleware::with_store(SessionConfig::from_env(), store);
```

`SessionMiddleware::new` または `with_store` で登録された `SessionStore` は、`destroy_all_for_user` によって解決されます。セッションストアが登録されていない場合（テストやミドルウェアを一度も構築しなかった埋め込みシステムなど）のみ、新しい `DatabaseSessionDriver` にフォールバックします。

## sessionsテーブル

デフォルトのドライバーは、次の形の `sessions` テーブルを期待します（`framework/src/session/driver/database.rs` にあるSeaORMのエンティティが、正となる定義です）:

| カラム | 型 | 備考 |
|---|---|---|
| `id` | VARCHAR PK | 40文字の小文字英数字によるセッションID |
| `user_id` | VARCHAR NULL | 認証済みユーザーのID（文字列。不透明なIDにも対応します） |
| `payload` | TEXT | JSONへシリアライズされたセッションデータのマップ |
| `csrf_token` | VARCHAR | セッションごとのCSRFトークン |
| `last_activity` | TIMESTAMP | 最終アクセス。有効期限とGCを駆動します |

このテーブルには、2つのインデックスが付いてきます: `idx_sessions_user_id`（`destroy_for_user` のため）と `idx_sessions_last_activity`（`gc()` のため）です。

スキャフォルドされたアプリケーションには、この形に一致する `create_sessions_table` マイグレーションが含まれています。自分でマイグレーションを持ち込む場合は、カラム名を厳密に写し取ってください - SeaORMはそれらを位置で解決するため、名前を変えたカラムは一致しません。

### Suprnovaが異なる設計を選んだ理由

LaravelがPHPの形に沿った選択をしたところを、Tokioのおかげで別の形にできた箇所が2つあります:

**ガベージコレクション。** Laravelは、リクエストごとに100分の2の抽選を回します: 各リクエストは2%の確率で、その場でセッションのGCを引き起こします。PHPでこれが成り立つのは、どのみちリクエストごとに新しいプロセスが立ち上がるからです。Tokioでは長く生きるワーカーがいるため、`SessionMiddleware::install` は、一定の間隔で `gc()` を呼ぶ[スーパーバイザー管理下の](supervisors.md)タスクを1つ登録します。リクエストごとのオーバーヘッドはなく、確率による驚きもありません - 抽選ではなく明示的なスケジューリングであり、スーパーバイザーの再起動ループがパニックを受け止めるため、1回の不出来なgcがデーモンを道連れにすることはありません。

**クロージャの形をした `session_mut`。** Laravelは `$request->session()` を手渡し、その上でメソッドを呼ばせます。Suprnovaはそうしません。ハンドラがfutureであり、futureは開始したのとは別のワーカースレッドで再開されうるからです。セッションはTokioの `task_local!` のスロットに置かれるため、借用によるアクセスはスコープの内側で行われなければなりません。クロージャの形はそのスコープを明示的にし、`.await` をまたいでミューテックスのガードを保持するという誤りを、静的に防ぎます。

**変更のある書き込みでは安全側に倒れます。** 頻度を抑えたアクティビティ更新が失敗した場合は `warn!` を記録し、既存のクッキーのままリクエストを通します（ユーザーから見える状態は損なわれていません）。*変更のあった*セッションの書き込み - ログイン、フラッシュ、CSRFのローテーション - が失敗した場合は500を返します。ストアが一度も記録しなかった状態に対するクッキーを黙ってクライアントへ手渡せば、「成功した」ログインが次のリクエストで消えてしまいます。それよりは、はっきりと失敗するほうがましです。

## 次のステップ

- [認証](authentication.md) - `Auth::login`、認証ガード、ユーザープロバイダーの連鎖
- [認証フロー](auth-flows.md) - パスワードリセット、2FA、ブルートフォースのスロットリング、remember-me
- [CSRF](csrf.md) - 書き込みの際に、セッションのCSRFトークンがどう検査されるか
- [ミドルウェア](middleware.md) - セッションを読み書きする独自のミドルウェアを書くこと
- [リクエスト ライフサイクル](lifecycle.md) - チェーンの中で `SessionMiddleware` がどこに座っているか
