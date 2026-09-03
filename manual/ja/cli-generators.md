# コード ジェネレーター

`suprnova make:*` ファミリーは、プロジェクトの各部分 - コントローラー、アクション、ミドルウェア、コンソールコマンド、ドメインエラー、スケジュールされたタスク、Inertiaページあるいはpropsの構造体、データベースマイグレーション - のための、慣習に沿ったファイルをスキャフォルドし、新しいモジュールをその親の `mod.rs`（そして必要なら `src/lib.rs` と `cmd/main.rs`）に配線します。同じボイラープレート + `pub mod x;` というインポート行を打ち直すことになる場面 - たいていの場合がそうです - では、これらに手を伸ばしてください。

## make:controller

コントローラーをスキャフォルドします - `invoke` という名前の単一の `#[handler]` 非同期fnを持つ、`src/controllers/` 内のファイルです。

```bash
suprnova make:controller User
suprnova make:controller order_item
```

名前は、ファイル名のために `snake_case` に正規化され、レスポンス内の `controller:` のエコーにはそのまま使われます。ASCIIの文字、数字、`_` だけが受け付けられます - `api/User` のようなパスは拒否されます。

### 生成されるファイル

```rust
// src/controllers/user.rs
use suprnova::{handler, json_response, Request, Response};

#[handler]
pub async fn invoke(_req: Request) -> Response {
    json_response!({
        "controller": "User"
    })
}
```

### 配線されるもの

1. `#[handler]` fnを持つ `src/controllers/<name>.rs` を書き込む。
2. `src/controllers/mod.rs` に `pub mod <name>;` を追加する（存在しなければファイルを作成する）。
3. `src/routes.rs` にルートを追加するためのヒントを出力する: `.get("/<name>", controllers::<name>::invoke)`。

ハンドラの契約、エクストラクター、そして `routes!` マクロについては、[コントローラー](controllers.md)を参照してください。

---

## make:action

単一責任のアクションをスキャフォルドします - コンテナから解決可能な構造体であり、本体を埋める前にスケルトンがコンパイルできるよう、非同期の `execute` メソッドが `Result<String, FrameworkError>` を返します。

```bash
suprnova make:action CreateUser
suprnova make:action SendNotification
```

名前はPascalCaseになります。`Action` サフィックスが欠けていれば付け足され、ファイルはsnake-caseにした構造体名になります。

### 生成されるファイル

```rust
// src/actions/create_user_action.rs
use suprnova::{injectable, FrameworkError};

#[injectable]
pub struct CreateUserAction {
    // Add injected dependencies as fields here, e.g.
    // db: suprnova::DbConnection,
}

impl CreateUserAction {
    pub async fn execute(&self) -> Result<String, FrameworkError> {
        Ok("CreateUserAction executed".to_string())
    }
}
```

### 配線されるもの

1. `src/actions/<snake>.rs` を書き込む。
2. `src/actions/mod.rs` に `pub mod <snake>;` を追加する。
3. `#[injectable]` は、リンク時にアクションをコンテナへ登録するため、どのコントローラーも `App::get::<CreateUserAction>()` 経由でそれを解決し、`action.execute().await?` を呼び出せる。

解決してから呼び出すパターンと、アクションがコンテナとどのように組み合わさるかについては、[アクション](actions.md)を参照してください。

---

## make:middleware

ミドルウェアをスキャフォルドします - `suprnova::Middleware` を実装するユニット構造体です。デフォルトの本体は、内側のハンドラの時間を計測し、リクエストごとのidとともにインバウンド + アウトバウンドのイベントをログに記録するため、最初の実行からエンドツーエンドで動作します。

```bash
suprnova make:middleware Auth
suprnova make:middleware RateLimit
```

名前はPascalCaseになります。`Middleware` サフィックスが欠けていれば付け足されます。ファイルは（サフィックスを除いた）snake-caseの基本名を使います。例えば `Auth` → `src/middleware/auth.rs`、構造体 `AuthMiddleware` です。

### 生成されるファイル

```rust
// src/middleware/auth.rs
use std::time::Instant;

use suprnova::{async_trait, current_request_id, Middleware, Next, Request, Response};

pub struct AuthMiddleware;

#[async_trait]
impl Middleware for AuthMiddleware {
    async fn handle(&self, request: Request, next: Next) -> Response {
        let method = request.method().to_string();
        let path = request.path().to_string();
        let request_id = current_request_id()
            .map(|id| id.as_str().to_string())
            .unwrap_or_default();
        let started_at = Instant::now();

        println!(
            "[AuthMiddleware] --> {} {} (request_id={})",
            method, path, request_id,
        );

        let response = next(request).await;

        println!(
            "[AuthMiddleware] <-- {} {} ({} ms, request_id={})",
            method, path, started_at.elapsed().as_millis(), request_id,
        );

        response
    }
}
```

### 配線されるもの

1. `src/middleware/<snake>.rs` を書き込む。
2. `src/middleware/mod.rs` に `mod <snake>;` + `pub use <snake>::<StructName>;` を追加する（必要なら作成する）。
3. ルートごとの形（`.get("/path", handler).middleware(AuthMiddleware)`）と、グローバルな形（`bootstrap.rs` の中の `global_middleware!(middleware::AuthMiddleware)`）の両方を出力する。

完全なチェーンのセマンティクス、順序、そしてグローバルとルートごとの違いについては、[ミドルウェア](middleware.md)を参照してください。

---

## make:command

コンソールコマンドをスキャフォルドします - リンク時に、プロジェクトごとの `console` バイナリが `inventory` 経由で拾い上げる `#[derive(clap::Parser, Command)]` 構造体です。デフォルトの本体は `println!("…: not yet implemented")` であるため、コマンドはすぐに実行できます。

```bash
suprnova make:command CleanCache
suprnova make:command mail:send
suprnova make:command clean-cache
```

命名は3つのルールに従います:

- `:` を含む入力は、登録されるコマンド名としてそのまま使われる（Laravelのネームスペース形式: `db:seed`、`mail:send`）。
- それ以外の場合、snake-caseにしたfn名がkebab化されて登録名になる（`CleanCache` → コマンド `clean-cache`）。
- Rustのファイルと構造体は、常に同じ識別子のsnake-case / PascalCase形式になる。

### 生成されるファイル

```rust
// src/commands/clean_cache.rs
use async_trait::async_trait;
use clap::Parser;
use suprnova::{Command, FrameworkError, TypedCommand};

#[derive(Parser, Command, Debug)]
#[console(name = "clean-cache", description = "TODO: describe what clean-cache does")]
pub struct CleanCache {
    // clap-deriveの引数をここに追加する。
}

#[async_trait]
impl TypedCommand for CleanCache {
    async fn run(self) -> Result<(), FrameworkError> {
        println!("clean-cache: not yet implemented");
        Ok(())
    }
}
```

### 配線されるもの

1. `src/commands/<snake>.rs` を書き込む。
2. `src/commands/mod.rs` に `pub mod <snake>;` を追加する（必要なら作成する）。
3. `src/lib.rs` に `pub mod commands;` が欠けていれば、大きな警告を出す - それがなければ、コマンドはconsoleバイナリにリンクされない。
4. 実行コマンドを出力する: `cargo run --bin console -- clean-cache`。

完全な型付きコマンドの表面、argvのみのハンドラのための `#[command]` の省略形、そしてプロジェクトごとのconsoleバイナリの役割については、[コンソール](console.md)を参照してください。

---

## live:make

Live コンポーネントを生成します。サーバーが所有するアイランドで、型付きアクションは
Live プロトコルで到着し、再レンダリングされたビューは同梱のブラウザーランタイムが
その場でモーフします。

```bash
suprnova live:make Counter
suprnova live:make todo-list
suprnova live:make Counter --dry-run
```

名前は `Counter`、`TodoList`、`todo-list`、`todo_list` のいずれかの形の単純な ASCII
識別子でなければなりません。ファイルとモジュールは snake_case、構造体は PascalCase
になり、登録されるコンポーネント名は `<package>.<kebab>` です（`demo-app` という
パッケージなら `demo-app.counter`）。Rust のキーワード、区切り文字、ドット、非 ASCII
入力は、何かを書く前に拒否されます。

### 生成されるファイル

```rust
// src/live/counter.rs
use suprnova::live::{LiveComponent, live};

/// A counter island rendered by `live/counter.html`.
#[derive(LiveComponent)]
#[live(name = "demo-app.counter", view = "live/counter.html")]
pub struct Counter {
    /// Current count, exposed to the view.
    #[public]
    count: u64,
}

#[live]
impl Counter {
    /// Increments the counter in response to `live:click="increment"`.
    #[action]
    pub fn increment(&mut self) {
        self.count += 1;
    }
}
```

```html
<!-- templates/live/counter.html -->
<div>
<p>Count: {{ count }}</p>
<button type="button" live:click="increment">Increment</button>
</div>
```

### 配線されるもの

1. まずすべての対象パスを検証し、トラバーサルとシンボリックリンクを拒否します。
   コンポーネントファイルまたはビューがすでに存在する場合は警告し、何も書きません。
2. `src/live/<snake>.rs` と `templates/live/<snake>.html` をアトミックに書きます。
   いずれかの書き込みが失敗すると、その実行で作成または変更したすべてのファイルを
   ロールバックします。
3. `src/live/mod.rs` の `registry()` ビルダーに `pub mod <snake>;` と
   `.register::<snake::Pascal>()?` を挿入します。`suprnova new` で作成したすべての
   プロジェクトには、空のレジストリ、ガード付きの予約済み Live ルートをインストールする
   `routes()` 関数、レジストリをバインドするブートストラップを備えたこのモジュールが
   含まれます。古いプロジェクトでは、初回使用時に同じモジュールが作成されます。
4. `src/lib.rs` に `pub mod live;` がなければ追加します。
5. レジストリをバインドするブートストラップの行を表示し、続いて確認コマンド
   `suprnova live:check` を表示します。

Live モジュールより前のプロジェクトでは、ブートストラップ中にレジストリをバインドし、
`cmd/main.rs` からルートを手動でインストールします:

```rust
suprnova::App::singleton(crate::live::registry().expect("Live registry"));
```

```rust
.try_routes(|| live::routes(routes::register()))
```

---

## make:error

ドメインエラーをスキャフォルドします - `#[domain_error]` がアノテーションされたユニット構造体であり、そのままでHTTPステータス、`Display` のメッセージ、そして `From<…> for FrameworkError` のimplを運びます。

```bash
suprnova make:error UserNotFound
suprnova make:error PaymentFailed
```

名前は構造体のためにPascalCaseになり、ファイルのためにsnake-caseになります。デフォルトのステータスは500であり、メッセージは文頭大文字にした構造体名です - 状況に合わせて、生成されたファイル内の両方のアトリビュートを変更してください。

### 生成されるファイル

```rust
// src/errors/user_not_found.rs
use suprnova::domain_error;

#[domain_error(status = 500, message = "User not found")]
pub struct UserNotFound;
```

`status = 500` を、状況に合うものに変更してください - not-foundには `404`、payment-requiredには `402`、forbiddenには `403` です - そしてメッセージ文字列を編集してください。より豊かなペイロードのためには、構造体に名前付きフィールドを追加し、手書きの `Display` impl の中で補間経由でメッセージ内から参照してください（その時点で `#[domain_error]` マクロは外してください）。

### 配線されるもの

1. `src/errors/<snake>.rs` を書き込む。
2. `src/errors/mod.rs` に `pub mod <snake>;` を追加する（必要なら作成する）。
3. `errors/` ディレクトリが新規に作成された場合、`src/lib.rs` の中で `mod errors;` を宣言することについて警告する。

### 使用例

`Response` を返すハンドラの内側で、`?` がきれいにショートサーキットできるよう、ドメイン型を `FrameworkError` へ持ち上げてください:

```rust
use crate::errors::user_not_found::UserNotFound;
use suprnova::FrameworkError;

#[handler]
pub async fn show(req: Request) -> Response {
    let id = req.param("id")?;
    let user = find_user(id).await
        .ok_or_else(|| FrameworkError::from(UserNotFound))?;
    json_response!({ "user": user })
}
```

[エラーハンドリング](errors.md)チャプターは、`#[domain_error]` と `AppError::bad_request(…)` と手書きの `HttpError` impl をいつ使うべきかを含む、完全なカスタムエラーのストーリーをカバーしています。

---

## make:task

スケジュールされたタスクをスキャフォルドします - `suprnova::Task` を実装するユニット構造体であり、あなたが実際の本体を埋める前から、そのスキャフォルドが進行状況をログに記録できるよう、構造化された開始/終了の行を出力します。

```bash
suprnova make:task CleanupLogs
suprnova make:task SendReminders
```

名前はPascalCaseになります。`Task` サフィックスが欠けていれば付け足されます。ファイルは、snake-caseにした構造体名です。例えば `CleanupLogs` → `src/tasks/cleanup_logs_task.rs` です。

### 生成されるファイル

```rust
// src/tasks/cleanup_logs_task.rs
use std::time::Instant;

use async_trait::async_trait;
use suprnova::{Task, TaskResult};

pub struct CleanupLogsTask;

impl CleanupLogsTask {
    pub fn new() -> Self {
        Self
    }
}

impl Default for CleanupLogsTask {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Task for CleanupLogsTask {
    async fn handle(&self) -> TaskResult {
        let started_at = Instant::now();
        println!("[CleanupLogsTask] task started");

        // これを実際のジョブに置き換える。

        println!(
            "[CleanupLogsTask] task finished in {} ms",
            started_at.elapsed().as_millis(),
        );
        Ok(())
    }
}
```

### 配線されるもの

最初の `make:task` の呼び出しは、他のジェネレーターより重い配線を行います - プロジェクト内にスケジューラーの表面をゼロから作り出します:

1. `src/tasks/` と `src/tasks/mod.rs` が欠けていれば作成する。
2. `src/schedule.rs`（`register(schedule: &mut Schedule)` エントリーポイント）が欠けていれば作成する。
3. `src/lib.rs` に `pub mod schedule;` と `pub mod tasks;` を宣言する。
4. `cmd/main.rs` あるいは `src/main.rs` の中の `Application::new()` チェーンに、`.run()` の直前に `.schedule(<crate>::schedule::register)` を挿入する。
5. `src/tasks/<snake>.rs` を書き込み、`src/tasks/mod.rs` に追加する。

以降の呼び出しは、既に実行済みのステップをスキップします。

### タスクを登録する

`src/schedule.rs` を開き、フルーエントなスケジュールAPIで登録の呼び出しを追加してください:

```rust
use suprnova::Schedule;
use crate::tasks::CleanupLogsTask;

pub fn register(schedule: &mut Schedule) {
    schedule.add(
        schedule.task(CleanupLogsTask::new())
            .daily()
            .at("03:00")
            .name("cleanup:logs")
            .description("Removes old log files daily"),
    );
}
```

その後、スケジューラーを実行してください:

```bash
suprnova schedule:work   # デーモン - 毎分チェックする
suprnova schedule:run    # ワンショット - 通常はcronから呼び出される
suprnova schedule:list   # 登録済みのタスクをすべて表示する
```

完全なタスクの表面（`hourly`、`weekly`、`cron(...)`、`between`、`when`、`without_overlapping`、タイムゾーンの扱い）については[タスク スケジューリング](scheduling.md)を、cronとして実行するかデーモンとして実行するかのトレードオフについては[スケジューリング コマンド](cli-scheduling.md)を参照してください。

---

## make:inertia

フラグに応じて、Inertiaのページコンポーネント（デフォルト）か、型付きのData構造体（`--data`）のどちらかをスキャフォルドします。ページジェネレーターは、`.env` からフロントエンドフレームワーク（Svelte 5、React 19、Vue 3.5）を検出し、対応するファイル拡張子を出力します。

### ページモード（デフォルト）

```bash
suprnova make:inertia About
suprnova make:inertia UserProfile
```

名前はPascalCaseになり、`Page` サフィックスが欠けていれば付け足されるため、`About` → `AboutPage` になります。ファイルは `frontend/src/pages/` に、フロントエンドごとの拡張子で置かれます: Svelteなら `AboutPage.svelte`、Reactなら `AboutPage.tsx`、Vueなら `AboutPage.vue` です。

例（Svelte）:

```svelte
<!-- frontend/src/pages/AboutPage.svelte -->
<div class="font-sans p-8 max-w-xl mx-auto">
  <h1 class="text-3xl font-bold">AboutPage</h1>
  <p class="mt-2">
    Edit <code class="bg-gray-100 px-1 rounded">frontend/src/pages/AboutPage.svelte</code> to get started.
  </p>
</div>
```

コントローラーからレンダリングする:

```rust
inertia_response!(&req, "AboutPage", props)
```

コントローラーとページの間の橋渡し、部分リロード、そして共有propsについては、[ページ コンポーネント](frontend-pages.md)と[Inertia レスポンス](frontend-inertia-responses.md)を参照してください。

### Data構造体モード（`--data`）

```bash
suprnova make:inertia UserProps --data
```

`app/src/props/`（`src/props/` ではない - ファイルがワークスペースの例/ホストアプリに置かれるよう、`app/` というプレフィックスがハードコードされています）に、`#[derive(Data, Validate)]` 構造体を出力します:

```rust
// app/src/props/user_props.rs
use suprnova::Data;
use validator::Validate;

#[derive(Data, Validate)]
pub struct UserProps {
    pub id: i64,
    // ここにフィールドを追加する。
    //
    // 利用可能なフィールドアトリビュート:
    //   #[data(input_only)] - Deserializeでは受け付けられ、Serializeからは除かれる
    //   #[data(output_only)] - Deserializeでは拒否され、Serializeには含まれる
    //   #[data(allow_include)] - ?include=の対象として登録される（デフォルト拒否）
    //
    // PATCHエンドポイントでは、absentとnullを区別するために suprnova::data::Field<T> を使う。
    // レイジーな出力フィールドには suprnova::inertia::Prop<T> を使う。
}
```

リクエストボディを検証するために、コントローラーの中で使ってください:

```rust
let dto: UserProps = req.validate_json().await?;
```

---

## make:migration

タイムスタンプ付きのSeaORMマイグレーションファイルをスキャフォルドします。`migrate` / `migrate:rollback` / `migrate:status` / `migrate:fresh` / `db:sync` コマンドも一通り解説する、[CLI マイグレーション](cli-migrations.md)で詳しく扱われています。短縮形:

```bash
suprnova make:migration create_users_table
```

マイグレーション名はそのまま保たれ、ファイルが時系列で並ぶよう `YYYYMMDDHHMMSS_` というスタンプが前置されます。生成されたファイルは `migrations/` に置かれます。

スキーマビルダーの表面については[マイグレーション](migrations.md)を、テストごとに分離されたデータベースに対してマイグレーションを実行する `TestDatabase::fresh` パターンについては[データベース テスト](database-testing.md)を参照してください。

---

## generate-types

`#[derive(InertiaProps)]` がアノテーションされたすべてのRust構造体から、TypeScriptのインターフェースを出力します。開発サーバーはこれを自動的に実行します。スタンドアロンのコマンドは、CIのチェックとワンショットの再生成のためのものです。

```bash
suprnova generate-types [--output <PATH>] [--watch]
```

| オプション | デフォルト | 説明 |
|---|---|---|
| `-o, --output <PATH>` | `frontend/src/types/inertia-props.ts` | 出力ファイルのパス |
| `-w, --watch` | off | ソースファイルをウォッチし、変更時に再生成する |

```bash
# ワンショット
suprnova generate-types

# ウォッチモード（完全な開発サーバーを実行したくないときに便利）
suprnova generate-types --watch

# カスタムの出力パス
suprnova generate-types --output frontend/src/types/props.ts
```

左側のRustの形は、右側のTypeScriptインターフェースを生成します:

```rust
#[derive(InertiaProps)]
pub struct UserPageProps {
    pub user: User,
    pub posts: Vec<Post>,
}
```

```typescript
export interface UserPageProps {
    user: User;
    posts: Post[];
}
```

完全なマッピング表（enum、option、日付、ネストした構造体）とオーバーライドのフックについては、[TypeScript 型](frontend-typescript-types.md)を参照してください。

---

### Suprnovaが異なる設計を選んだ理由

Laravelの `php artisan make:*` は、正しいディレクトリにファイルを置くだけです - PSR-4の自動読み込みが、フレームワークが次に起動するときに新しいクラスを拾い上げます。Rustには、それに相当するものがありません。`src/foo/bar.rs` にあるファイルは、`src/foo/mod.rs` が `pub mod bar;` を宣言するまでクレートにコンパイルされず、親ディレクトリも同じ方法で `src/lib.rs` の中で配線されなければなりません。

そのため、あらゆる `suprnova make:*` ジェネレーターは、1つではなく2つのことを行います: 新しいファイルを書き込み、*そして*最も近い `mod.rs`（`make:task` と `make:command` については `src/lib.rs` と `cmd/main.rs` も）を編集します。だからこそ、どのジェネレーターも `Created src/.../mod.rs` あるいは `Updated src/.../mod.rs` という行を出力します - 配線は作業の一部であり、自分で覚えておくべき後続のステップではありません。

---

## まとめ

| コマンド | 作成するもの | 配線先 |
|---|---|---|
| `make:controller <name>` | `src/controllers/<snake>.rs` | `controllers/mod.rs` |
| `make:action <Name>` | `src/actions/<snake>_action.rs` | `actions/mod.rs` |
| `make:middleware <Name>` | `src/middleware/<snake>.rs` | `middleware/mod.rs` |
| `make:command <name>` | `src/commands/<snake>.rs` | `commands/mod.rs`（+ `lib.rs` について警告する） |
| `make:error <Name>` | `src/errors/<snake>.rs` | `errors/mod.rs` |
| `make:task <Name>` | `src/tasks/<snake>_task.rs` | `tasks/mod.rs`、`schedule.rs`、`lib.rs`、`main.rs` |
| `make:inertia <Name>` | `frontend/src/pages/<Name>Page.<ext>` | （モジュールの配線なし） |
| `make:inertia <Name> --data` | `app/src/props/<snake>.rs` | （モジュールの配線なし） |
| `make:migration <name>` | `migrations/YYYYMMDDHHMMSS_<name>.rs` | （モジュールの配線なし） |
| `generate-types` | `frontend/src/types/inertia-props.ts` | n/a |

## 次のステップ

- [CLI 概要](cli.md) - 完全なサブコマンドの表
- [コンソール](console.md) - `make:command` が供給する、プロジェクトごとのconsoleバイナリ
- [コントローラー](controllers.md) - `make:controller` がスキャフォルドするハンドラの契約
- [タスク スケジューリング](scheduling.md) - `make:task` が生成したタスクを登録するために使う、フルーエントなスケジュールAPI
- [CLI マイグレーション](cli-migrations.md) - `make:migration` と対になる、migrate / db:sync コマンド
