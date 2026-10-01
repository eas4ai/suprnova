# アクション

Suprnovaにおけるアクションとは、1つの仕事だけを持つ構造体です - 1つのメソッドの背後に、単一のビジネスロジックを保持します。これはLaravelの単一アクション呼び出し可能コントローラー - `RegisterUser`、`PublishPost`、`ChargeInvoice` - に相当するRustの仕組みです。アクションは `src/actions/` に置かれ、コンテナが解決できるよう `#[injectable]` アトリビュートを持ち、コントローラー（そしてジョブや他のアクション）が呼び出す `execute(...)` メソッドを公開します。`#[action]` マクロは存在せず、「1つのメソッド」というルールをフレームワーク側で強制することもありません - この形は規約であり、`#[injectable]` はその規約を苦労なく成り立たせる仕組みです。

```rust
use suprnova::{injectable, FrameworkError};

#[injectable]
pub struct RegisterUserAction {
    // 依存関係をフィールドとして注入する - 下の「依存関係」を参照
}

impl RegisterUserAction {
    pub async fn execute(&self, email: &str) -> Result<String, FrameworkError> {
        tracing::info!(action = "RegisterUser", email, "executed");
        Ok(format!("registered: {email}"))
    }
}
```

ハンドラから `App::resolve::<RegisterUserAction>()?` で解決すれば、サービス層の基底クラスを新たに考案することなく、ドメインロジックをHTTP層から切り離せます。パターンはそれで全てです。

## アクションを生成する

```bash
suprnova make:action RegisterUser
```

CLIは名前をPascalCaseへ正規化し、`Action` サフィックスが欠けていれば付け足したうえで、ファイル名をsnake_caseにします。つまり:

| `make:action <Name>` | 構造体名 | ファイル |
|---|---|---|
| `RegisterUser` | `RegisterUserAction` | `src/actions/register_user_action.rs` |
| `SendNotification` | `SendNotificationAction` | `src/actions/send_notification_action.rs` |
| `ProcessPayment` | `ProcessPaymentAction` | `src/actions/process_payment_action.rs` |
| `ChargeInvoiceAction` | `ChargeInvoiceAction` | `src/actions/charge_invoice_action.rs` |

ジェネレーターはファイルを書き出し、`src/actions/mod.rs` に `pub mod register_user_action;` の行を追加します。出力されるスタブは、そのままコンパイルが通ります:

```rust
//! register_user_action action

use suprnova::{injectable, FrameworkError};

/// RegisterUserAction
///
/// Single-responsibility command resolved from the container. Inject any
/// dependencies as fields and the `#[injectable]` macro wires them at
/// resolve time.
#[injectable]
pub struct RegisterUserAction {
    // Add injected dependencies as fields here, e.g.
    // db: suprnova::DbConnection,
}

impl RegisterUserAction {
    /// Execute the action.
    pub async fn execute(&self) -> Result<String, FrameworkError> {
        Ok("RegisterUserAction executed".to_string())
    }
}
```

シグネチャ - `async fn execute(&self) -> Result<_, FrameworkError>` - は、本番投入に耐える形です: 非同期であり、呼び出し元で `?` を通じてそのまま `HttpResponse` へ変換される `Result` を返します。本体はプレースホルダーなので、実際のワークフローに差し替えてください。

## `#[injectable]` アトリビュート

`#[injectable]` は、アクションパターンが依存しているフレームワーク機構の唯一の部品です。これは3つのものへ展開されます。

1. 構造体への `#[derive(Clone)]`（`#[inject]` フィールドが1つもない場合は `Default` も）。
2. 起動処理が型を発見できるようにする `inventory::submit!` エントリ。
3. `boot_services()` の間に一度だけ `App::singleton_if_absent` が実行する自動登録クロージャ。

このマクロの契約は次のとおりです。

| 構造体の形 | 振る舞い |
|---|---|
| ユニット構造体（`pub struct Foo;`） | `Default + Clone` を導出し、`Default::default()` を登録する |
| 名前付きフィールド、`#[inject]` なし | `Default + Clone` を導出し、`Default::default()` を登録する |
| `#[inject]` 付きの名前付きフィールド | `Clone` のみを導出する。`#[inject]` の付いた各フィールドは起動時にコンテナから解決され、injectでないフィールドはデフォルト値になる |
| タプル構造体 | コンパイル時に拒否される - 「名前付きフィールドを使ってください」 |

解決されたアクションは、保存されたシングルトンの複製です。コストは `App::resolve::<Action>()?` の呼び出し1回につき `Clone` が1回であり、ユニット構造体や `Arc` でラップされたサービスの構造体であれば、わずかな参照カウントの増減で済みます。重い状態は、アクション自体の中ではなく、アクションが注入する `Arc<dyn …>` サービスの背後に置いてください。

### `#[inject]` は起動時に行われ、呼び出しのたびには行われない

フレームワークが起動すると、`App::boot_services()` はすべての `#[injectable]` 登録を巡回し、それらを不動点に達するまでのリトライループの中で実行します。各エントリは、自分の `#[inject]` フィールドをコンテナから解決しようと試みます。依存関係がまだ登録されていない場合、そのエントリは次の反復に持ち越されます。このループは、すべてのエントリが成功するか、それ以上進展がなくなるまで続きます - 失敗した場合、フレームワークは解決不能な型または循環の名前を含む構造化されたエラーを返します。

実務上の帰結はこうです: **`App::resolve::<MyAction>()` は、すでに構築済みのシングルトンを複製します。** 呼び出しのたびに `#[inject]` の解決を実行するわけではありません。あるアクションが依存しているinjectable対象は、そのアクションより前に - 自身の `#[injectable]` アトリビュートを通じて、あるいは `bootstrap()` 関数内での手動の `App::bind` / `App::singleton` を通じて - 登録されていなければなりません。リトライループはインベントリの順序を処理してくれますが、存在しないサービスを作り出してはくれません。

## コントローラーからアクションを使う

標準的なハンドラの形はこうです: 解決し、実行し、描画する。

```rust
use suprnova::{App, Request, Response, ResponseExt, json_response};

use crate::actions::register_user_action::RegisterUserAction;

pub async fn store(_req: Request) -> Response {
    let action = App::resolve::<RegisterUserAction>()?;
    let result = action.execute("alice@example.com").await?;

    json_response!({ "ok": true, "result": result }).status(201)
}
```

どちらの `?` も機能します。どちらのエラー型も `From` の実装を通じて `HttpResponse` へ変換されるからです - `App::resolve` は `Result<T, FrameworkError>` を返し、フレームワークのエラーコンバータが残りを処理します。サービス登録の欠落は、パニックではなく、構造化ログにサービス名を伴う500として表面化します。全体像については[エラー モデル](error-model.md)を参照してください。

resolveに `?` を使いたくない場合 - たとえば起動時にハードに失敗すべき経路であれば - `App::get::<RegisterUserAction>()` は `Option<T>` を返すので、配線を間違えたときにはっきりと失敗させたいなら `.expect("registered at boot")` を使えます。

## データベースに触れる非同期アクション

これは、実際のところほとんどのアクションが辿る経路です - Eloquentモデルを介して読み込むか、書き込みます。ボディをあなたのドメインから持ち上げてください。表面は同じです。

```rust
use suprnova::{attrs, injectable, FrameworkError, Model};

use crate::models::todos::Todo;

#[injectable]
pub struct CreateRandomTodoAction;

impl CreateRandomTodoAction {
    pub async fn execute(&self) -> Result<Todo, FrameworkError> {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
            % 10000;

        Todo::create(attrs! {
            title: format!("Todo #{}", n),
            description: format!("created at {}", n),
            done: false,
        })
        .await
    }
}

#[injectable]
pub struct ListTodosAction;

impl ListTodosAction {
    pub async fn execute(&self) -> Result<Vec<Todo>, FrameworkError> {
        Ok(<Todo as suprnova::eloquent::Model>::all().await?.into_vec())
    }
}
```

`Todo::create(attrs!{...})` と `Todo::all()` は、`#[suprnova::model]` マクロに由来します。モデルの表面については[Eloquent](eloquent.md)を参照してください。`Model::all()` が `Collection<Todo>` を返す点に注意してください - この例ではコントローラーに素の `Vec` を渡すために `.into_vec()` を呼んでいますが、`Collection` をそのまま返してシリアライザに描画させることもできます。

これらをコントローラーへ配線します:

```rust
use suprnova::{App, Request, Response, ResponseExt, json_response};

use crate::actions::todo_action::{CreateRandomTodoAction, ListTodosAction};

pub async fn create_random(_req: Request) -> Response {
    let action = App::resolve::<CreateRandomTodoAction>()?;
    let todo = action.execute().await?;
    json_response!({ "ok": true, "todo": todo }).status(201)
}

pub async fn list(_req: Request) -> Response {
    let action = App::resolve::<ListTodosAction>()?;
    let todos = action.execute().await?;
    json_response!({ "ok": true, "todos": todos })
}
```

ハンドラごとに `?` が2回。コントローラーは、HTTPとドメインの間の薄いアダプターであり続けます。

## `#[inject]` による依存関係

アクションが協力者 - メーラー、ロガー、ドメインサービス - を必要とする場合は、それらをフィールドとして宣言し、それぞれに `#[inject]` のタグを付けます:

```rust
use suprnova::{injectable, FrameworkError};

use crate::services::{MailerService, LoggerService};

#[injectable]
pub struct SendWelcomeEmailAction {
    #[inject]
    mailer: MailerService,
    #[inject]
    logger: LoggerService,
}

impl SendWelcomeEmailAction {
    pub async fn execute(&self, to: &str) -> Result<(), FrameworkError> {
        self.logger.info(&format!("welcome → {to}"));
        self.mailer.send_welcome(to).await
    }
}
```

`MailerService` と `LoggerService` はどちらも、このアクションが起動する前に、それ自体がコンテナ登録済みでなければなりません - 自身の `#[injectable]` アトリビュートによってか、あるいは `bootstrap()` 呼び出しによってです:

```rust
// src/bootstrap.rs 内で
App::singleton(MailerService::from_env()?);
App::singleton(LoggerService::default());
```

起動が不動点ループを実行する時点でどちらかの依存関係が欠けていれば、起動は未解決の型を名指ししたエラーを返し、フレームワークは半端に配線されたコンテナのまま起動するのではなく、非ゼロで終了します。

`#[inject]` の付かないフィールドは `Default::default()` にフォールバックするため、コンストラクタを書くことなく、注入された依存関係と素の状態を混在させられます。

## アクションを使うべきとき

経験則はこうです: アクションが存在するべきなのは、同じ作業が（あるいは将来）複数のエントリポイントから起動される場合です。HTTPルートとキュージョブの両方から実行される登録フローは、`RegisterUserAction` に属します。「このインデックスページを描画するだけ」の一回限りのハンドラにアクションは要りません - コントローラーに留めておいてください。

| 適する場合 | 例 |
|---|---|
| 複数ステップにわたるビジネス操作 | `RegisterUserAction`、`CheckoutAction` |
| HTTPとキューの両方で共有される作業 | `IssueRefundAction`（両方の経路からディスパッチされる） |
| リクエストなしでテストする価値のあるロジック | `CalculateTotalsAction` |
| 外部インテグレーション | `SendEmailAction`、`SyncInventoryAction` |
| さもなければコントローラーにインライン化して複製されてしまうもの全般 | 「3回目」ルールの発動 |

コントローラーと比べて、アクションは再利用可能で、`Request` に縛られておらず、テストから呼び出すのも簡単です（`App::resolve` + `await`）。コントローラーは、アクションの結果を `Response` へ変換する方法を知っている、HTTPを意識した境界であり続けます。

| コントローラー | アクション |
|---|---|
| 1つのルートを扱う | ルート、ジョブ、スケジュールをまたいで再利用可能 |
| `Request` / `Response` を知っている | あなたのドメイン型を知っている |
| `Response` を返す | `Result<T, FrameworkError>` を返す |
| アクションを呼び出す | コントローラー（や他のもの）から呼び出される |

## アクション、バス、キュー

ビジネスロジックが置ける場所はアクションだけではありません - [バス](bus.md)は型付きの出力を持つディスパッチされたコマンドを扱い、[キュー](queues.md)はワーカー上で実行されるべき作業を扱います。どちらを使うかは、作業がどう起動されるかで選んでください:

| したいこと… | 手を伸ばすもの |
|---|---|
| コントローラーやジョブから呼び出せる同期的なビジネスロジック | **アクション**（`#[injectable]` + `execute`） |
| 登録済みハンドラを介して `Bus::dispatch` で呼び出せる、型付きコマンド | [バス](bus.md) |
| 永続的で、リトライされる、タスク外の作業 | [キュー](queues.md) |

混在させても構いません - `BusHandler` や `Job` は、しばしば単にアクションを解決してその `execute` を呼ぶだけです。アクションはドメインロジックを保持し、バスやキューはディスパッチのメタデータを保持します。

## ファイル構成

`make:action` が出力するもの、そしてグループ化のための余地です:

```
src/
├── actions/
│   ├── mod.rs                          // pub mod register_user_action;
│   ├── register_user_action.rs
│   ├── send_welcome_email_action.rs
│   └── billing/                        // ディレクトリが成長したらドメインごとにグループ化
│       ├── mod.rs
│       ├── charge_invoice_action.rs
│       └── issue_refund_action.rs
├── controllers/
└── main.rs
```

フレームワークの側は、この配置を何ら要求していません。ジェネレーターが `src/actions/` に書き出すのは、それが規約だからです。アクションを `src/billing/actions/` へ移動しても動き続けます - `#[injectable]` は場所を問いません。

## アクションのテスト

アクションは、`async` メソッドを持つ、単なるコンテナ解決可能な構造体にすぎないため、テストの表面は `App::resolve` + `await` です。他の場所でも使われている同じ `TestDatabase` テストフィクスチャが、ここでも機能します:

```rust
use suprnova::{describe, expect, test, App};
use suprnova::testing::TestDatabase;

use crate::actions::todo_action::ListTodosAction;
use crate::models::todos::Todo;

describe!("ListTodosAction", {
    test!("returns all todos", async fn(_db: TestDatabase) {
        Todo::create(suprnova::attrs! { title: "Test", description: "", done: false })
            .await
            .unwrap();

        let action = App::resolve::<ListTodosAction>().unwrap();
        let todos = action.execute().await.unwrap();

        expect!(todos).to_have_length(1);
    });
});
```

`describe!` / `test!` / `expect!` の表面全体、そしてテスト対象のアクションへフェイクのメーラーやフェイクのゲートウェイを注入したいときの `TestContainer::fake` については、[テスト](testing.md)を参照してください。

## Suprnovaが異なる設計を選んだ理由

Laravelの単一アクションコントローラー - `App\Actions\` の中にある `__invoke` メソッドを持つクラス - は、リクエストごとに構築されます。コンテナがそのクラスを解決し、コンストラクタインジェクションを実行し、レスポンスが送出されるとインスタンスは破棄されます。PHPのリクエストごとにプロセスを立てるモデルは、これを実質タダにしています。

Suprnovaのアクションは、コンテナに常駐するシングルトンです: 起動時に一度だけ構築され、その時点で `#[inject]` フィールドが解決され、`App::resolve` のたびに複製されて渡されます。このパターンがRustに適合するのは、`Arc` でラップされたサービスの構造体を複製するコストがわずかな参照カウントの増減で済む一方、リクエストのたびに構造体を構築しては捨てることになれば、あらゆるフィールドがアロケーションを経由することを強いられるからです。Laravel由来の規約 - 1つの構造体、1つのメソッド、操作にちなんだ名前 - はそのまま生き残り、その下の配線がTokio向けの形をしています。

もう1つの意図的な分割はこうです: コントローラーはフリー関数のままであり（[コントローラー](controllers.md)を参照）、そのためHTTP層は、それ自体のDI表面を持たない、純粋なリクエストからレスポンスへの変換であり続けます。コンストラクタ形式の注入は `#[injectable]` の境界で、つまりそれがふさわしい場所であるアクションの内部で行われます。

## 次のステップ

- [コントローラー](controllers.md) - アクションを解決して呼び出す、HTTP向けのフリー関数
- [サービス コンテナ](container.md) - `App::resolve`、`App::singleton`、そして3層のルックアップが実際に何をしているか
- [バス](bus.md) - 解決されたアクションではなく、登録済みハンドラが欲しいときの型付きコマンドディスパッチ
- [テスト](testing.md) - 隔離されたアクションテストのための `App::resolve` + `TestContainer::fake`
- [エラー モデル](error-model.md) - `App::resolve::<Action>()?` と `action.execute().await?` の `?` が、どのようにしてきれいなレスポンスへ収束するか
