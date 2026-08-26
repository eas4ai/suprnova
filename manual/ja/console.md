# コンソール

Suprnovaの各プロジェクトには `console` バイナリが同梱されています - アプリのコンパイル済み型を必要とするあらゆるもの、つまりデータベースシーダー、プルーナー、ワンショットのメンテナンスタスクなど、Laravelの `php artisan` で組み立てるようなものすべてのための、実行時コマンドディスパッチャーです。コマンドは、`#[derive(Command)]`（`clap::Parser` の上に構築されています）する型付き構造体か、`#[command]` をアノテーションした非同期fnのどちらかです。フレームワークはリンク時に `inventory` を通じてそれらを収集するため、新しいコマンドを追加するのは、編集すべき中央レジストリのない、1つのファイルで済みます。これはLaravelの `php artisan` に相当するSuprnovaの仕組みです - 同じスクリプト、同じプロセス、同じアドレス空間で、ハンドラが戻ると終了します。

## クイックスタート

推奨される形は、型付き引数のために `#[derive(clap::Parser, Command)]` を使います:

```rust
use async_trait::async_trait;
use clap::Parser;
use suprnova::{Command, FrameworkError, TypedCommand};

#[derive(Parser, Command, Debug)]
#[console(name = "greet", description = "Print a friendly greeting")]
pub struct Greet {
    #[arg(short, long, default_value = "world")]
    pub name: String,

    #[arg(long, default_value_t = false)]
    pub loud: bool,
}

#[async_trait]
impl TypedCommand for Greet {
    async fn run(self) -> Result<(), FrameworkError> {
        let prefix = if self.loud { "HELLO" } else { "Hello" };
        println!("{prefix}, {}!", self.name);
        Ok(())
    }
}
```

これを `src/commands/greet.rs` に置き、`src/commands/mod.rs` に `pub mod greet;` を追加して、実行します:

```bash
cargo run --bin console -- greet
# Hello, world!
cargo run --bin console -- greet --name Alice --loud
# HELLO, Alice!
cargo run --bin console -- greet --help
# （clapが生成するコマンドごとのヘルプで、型付きフラグも含む）
```

編集すべき中央レジストリはありません。`#[derive(Command)]` はinventoryを介して `CommandEntry { name, description, clap_builder, handler }` を提出します。コンソールバイナリは `suprnova::console::dispatch_argv_with_init(argv, init)` を呼び出し、これは登録済みのすべてのエントリから1本のclapパーサーツリーを構築し、実際のサブコマンドがマッチしたときにだけ起動用の `init` クロージャを実行し、パース済みの `ArgMatches` を適切なハンドラへとディスパッチします。

### よりシンプルな方法: 生の `Vec<String>`

型付き引数を必要としない、些細なコマンドについては、非同期fnに付けた `#[command]` アトリビュートでも機能します:

```rust
use suprnova::{command, FrameworkError};

#[command(name = "ping", description = "Smoke test")]
pub async fn ping(_args: Vec<String>) -> Result<(), FrameworkError> {
    println!("pong");
    Ok(())
}
```

内部では、どちらの経路も同じ `CommandEntry` レジストリに行き着きます。生の形は、argvを `Vec<String>` へ取り込むために `trailing_var_arg` を伴うclapのサブコマンドを使っているだけです。引数を持つコマンドには型付きの形を優先してください - パーサーを手書きすることなく、コマンドごとの `--help`、値のパース、デフォルト値、short/longのフラグのペアが手に入ります。

## コンソールバイナリ

`suprnova new` は、新しいプロジェクトごとに2つのバイナリをスキャフォルドします:

- **`<project>`**（`cmd/main.rs` または `src/main.rs`） - `cargo run` または `suprnova serve` によって起動されるHTTPサーバーです。長時間実行され、killされるまで動作し続けます。
- **`console`**（`src/bin/console.rs`） - 実行時コマンドディスパッチャーです。ワンショットであり、ハンドラが戻ると終了します。

コンソールバイナリの `main` は小さく、予測可能です:

```rust
use std::process::ExitCode;

#[suprnova::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    // このプロジェクトのバージョンを `--version` / `--help` で表に出す
    // env! はフレームワークのものではなく、ユーザーのアプリのバージョンに解決される
    suprnova::console::set_version(env!("CARGO_PKG_VERSION"));

    let argv: Vec<String> = std::env::args().collect();
    let result = suprnova::console::dispatch_argv_with_init(argv, || async {
        my_app::config::register_all();
        my_app::bootstrap::register().await;
    })
    .await;

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}
```

Tokioは `current_thread` フレーバーで動作します - ワンショットのコマンドの中には、コアをまたいで並列化すべき作業がなく、マルチスレッドランタイムのワーカープールは、単なるオーバーヘッドにしかならないからです。

気に留めておくべきことが2つあります:

- **ブートストラップは遅延します。** `dispatch_argv_with_init` に渡されたクロージャは、clapが実際に登録済みのサブコマンドにマッチしたときにだけ実行されます。`console --help`、`console --version`、サブコマンド欠落、パースエラーの各経路はいずれもそれをスキップします - そのため `console --help` は、`DATABASE_URL` がまだ設定されていない、まっさらなチェックアウトの上でも機能します。
- **`main` はエラーを出力しません。** ユーザー向けのstderrはすべて `dispatch_argv_with_init` が所有します - （clapがすでに出力したパースエラーのように、エラーがサイレントでない限り）ハンドラのエラーメッセージをeprintlnし、clap自身のhelp / version / パースエラーの出力を表示します。`main` は純粋な `Result → ExitCode` の変換であり、冗長な `eprintln!` を追加すれば二重出力になってしまいます。

特定のコマンドで、コストの高いブートストラップの手順をまるごとスキップさせたい場合は、「遅延ブートストラップ」のフラグをフレームワーク全体に通すのではなく、その手順自体を環境変数でゲートしてください。

## 組み込みコマンド

フレームワーク自身が、小さなコマンド集合を登録します。フレームワークをプロジェクトにリンクすれば、それらは自動的に持ち込まれます。

| コマンド | 内容 |
|---------------|-------------------------------------------|
| `db:seed` | 登録済みの `Seeder` を順番にすべて実行します。`--class=<Name>`（あるいは裸の位置引数）を受け付け、`php artisan db:seed --class=UserSeeder` に対応する形で、名前を指定した1つのシーダーだけを実行できます。 |
| `model:prune` | `PrunerEntry` レジストリを走査し、登録済みの各 `Prunable` / `MassPrunable` スコープが返す行をすべて強制削除します。`--model=<Name>` は1つの型に絞り込み、`--pretend` は行を一切変更せずに件数だけを報告します。 |
| `--help` / `-h` | 利用可能なコマンドを一覧表示します。サブコマンドごとの `--help` は、型付き引数からclapが構築します。 |
| `--version` | `set_version` によって登録されたバージョン（通常はあなたのアプリの `CARGO_PKG_VERSION`）を出力します。`set_version` が一度も呼ばれていなければ、完全に省略されます。 |

`db:seed` は、`suprnova::seed::register::<MySeeder>()` で `bootstrap::register()` の中に登録したものを、何であれ実行します。レジストリが空であれば警告を出力して `Ok(())` を返します - シーダーを登録する前に `db:seed` を呼び出すのは、罪のないユーザーの誤りであって、プログラマーの誤りではありません。

`db:seed` は、対象を絞った実行の進捗を `suprnova::two_column_detail` を使って報告します。これは、名前、ドットリーダー、そしてステータスを、80カラムの1行として描画します。あなた自身のコマンドからも、同じ見た目を得るためにこれを呼び出せます。

> ワーカーデーモン（`queue:work`、`schedule:run`、`schedule:work`、`schedule:list`、`workflow:work`）は、コンソールバイナリの上には**ありません**。それらは、app/serverバイナリのclapパーサー上に存在します（HTTPを提供するのと同じバイナリです）。グローバルな `suprnova` CLIは、それらについては `cargo run --quiet -- <name>` へシェルアウトします。下の[非対称性のセクション](#suprnova-migrate-との非対称性)を参照してください。

## コマンドを定義する

2つのマクロ、1つのレジストリです。コマンドの形に合う方を選んでください。

### `#[derive(Command)]` - 型付き引数（推奨）

`#[derive(clap::Parser)]` の上に重ねます。構造体のフィールドがコマンドの引数になり、clapがargvを構造体へパースし、フレームワークがあなたの `TypedCommand::run(self)` を呼び出します。

```rust
use async_trait::async_trait;
use clap::Parser;
use suprnova::{Command, FrameworkError, TypedCommand};

#[derive(Parser, Command, Debug)]
#[console(name = "users:purge", description = "Purge users older than N days")]
pub struct UsersPurge {
    #[arg(long)]
    pub older_than_days: u32,

    #[arg(long, default_value_t = false)]
    pub dry_run: bool,
}

#[async_trait]
impl TypedCommand for UsersPurge {
    async fn run(self) -> Result<(), FrameworkError> {
        // self.older_than_days, self.dry_run - 型付きで、clapによってバリデーション済み
        Ok(())
    }
}
```

アトリビュート:

| アトリビュート | 必須 | 目的 |
|--------------|----------|-----------------------------------------------|
| `#[console(name = "...")]` | yes | CLI上での呼び出し名（`"users:purge"`、`"mail:send"`、`"greet"`）。 |
| `#[console(description = "...")]` | no | トップレベルのヘルプに表示される1行の説明。 |
| `#[arg(...)]`（clap） | n/a | short/longのフラグ、デフォルト値、値パーサーなどのための、clap自身のフィールドアトリビュート。 |

clapが自動生成する、コマンドごとのヘルプ（`console users:purge --help`）も無償で手に入ります。

### `#[command]` - 生の `Vec<String>`（単純なケース）

引数を取らない、あるいは位置引数をリストとして消費するだけのコマンドには、非同期fnへのアトリビュートで十分です:

```rust
use suprnova::{command, FrameworkError};

#[command(name = "cache:clear", description = "Drop every entry from the cache")]
pub async fn cache_clear(_args: Vec<String>) -> Result<(), FrameworkError> {
    suprnova::Cache::flush().await
}
```

アノテーションを付けた関数は `async fn(Vec<String>) -> Result<(), FrameworkError>` でなければなりません。マクロは元の関数をそのまま保持するため、Rustから直接呼び出すこともできます - argvの文字列をディスパッチャー経由で通したくない単体テストで便利です。

どちらの形の名前も、Laravel流のネームスペースをサポートします: `mail:send`、`queue:work`、`db:fresh`。コロンは純粋に見た目のためのものです - ディスパッチャーが `argv[1]` に対して照合する、ただの文字列にすぎません。

## `suprnova make:command`

CLIジェネレーターは、実行可能なスタブを配置します。生成されるファイルは**型付きの形**（`#[derive(Parser, Command)]` + `impl TypedCommand`）を使います - これが推奨されるデフォルトであり、コマンドごとの `--help` を無償で得られます:

```bash
suprnova make:command cache:clear
# → src/commands/cache_clear.rs（#[console(name = "cache:clear")] を持つ pub struct CacheClear）
# → src/commands/mod.rs に `pub mod cache_clear;` が追加される（存在しなければ新規作成）
```

スタブはそのまま実行できます - `cargo run --bin console -- cache:clear` は `cache:clear: not yet implemented` を出力して `Ok(())` を返すため、配線してから反復開発できます。型付き引数のためのフィールドを構造体に書き足し、`TypedCommand::run` の本体を差し替えてください。

名前の正規化:

| 入力          | ファイル              | コマンド名   |
|----------------|-------------------|----------------|
| `greet`        | `greet.rs`        | `greet`        |
| `CleanCache`   | `clean_cache.rs`  | `clean-cache`  |
| `clean-cache`  | `clean_cache.rs`  | `clean-cache`  |
| `mail:send`    | `mail_send.rs`    | `mail:send`    |

入力に `:` が含まれる場合、コロンのネームスペースはそのまま保たれます。それ以外では、Rustの関数名はsnake_caseに、コマンド名はkebab-caseになります。

`pub mod commands;` が `src/lib.rs` に宣言されていることを確認してください。そうしないと、inventoryへの提出がコンソールバイナリからリンク到達可能になりません。ジェネレーターは新しいプロジェクトに対してこれをスキャフォルドし、欠けていれば大きな警告を出します。あなたがそれを取り除いてしまった場合、新しいファイルの `inventory::submit!` ブロックはコンパイルは通るものの、レジストリには決して行き着きません。

### Suprnovaが異なる設計を選んだ理由

フレームワークは、`db:seed` のような実行時タスクのために、グローバルな `suprnova` CLIコマンドを作ることを意図的に**しません**。グローバルなバイナリは、次のいずれかなしには、あなたのアプリのシーダー、ファクトリー、`#[command]` の非同期fnを静的にロードできません:

- `cargo run --bin app -- ...` へシェルアウトする（遅い - 呼び出しごとにフルコンパイルが走り、そもそもの目的を損なう）、または
- 動的ロード（v1にしては複雑すぎる）

そのため、ユーザーのプロジェクトは `console` バイナリを生成します。それを直接実行してください:

```bash
./target/debug/console db:seed
./target/release/console greet Alice
cargo run --bin console -- mail:send
```

Laravelは同じ問題を `php artisan` で解決しています - フレームワークを起動し、ユーザー定義のコマンドへディスパッチする、プロジェクトごとのスクリプトです。フレームワークのコードがランタイム上でユーザーのコードのすぐ隣に存在するため、PHPはこれを動的に行えます。Rustのコンパイル・リンクモデルはそれを許さないため、私たちはディスパッチャーをライブラリ（`suprnova::console::*`）として出荷し、各プロジェクトに、自分専用の1行の `console` バイナリをリンクさせています。

### `suprnova migrate` との非対称性

Suprnovaのプロジェクトには、コマンド起動の経路が3つ明確に存在し、その非対称性は**構造的なもの**です - それらを統一しようとしないでください:

| コマンドの表面                                   | 起動方法                                              | 理由                                                 |
|---------------------------------------------------|---------------------------------------------------------|-----------------------------------------------------|
| `suprnova new`、`suprnova make:*`、`suprnova serve`、`suprnova key:generate`、… | グローバルCLIバイナリ（`cargo install --git` でインストール） | ファイルのみを扱うジェネレーターやスキャフォルダーであり、ユーザーコードを必要としない。 |
| `suprnova migrate`、`suprnova migrate:status`、`suprnova schedule:run`、`suprnova schedule:work`、`suprnova schedule:list`、`suprnova workflow:work` | グローバルCLIが、app/serverバイナリに対して `cargo run --quiet -- <name>` へシェルする | 同じ `Application::run` のclapパーサーが所有する、長時間実行のデーモンとスキーマ作業。サーバーバイナリの `queue:work` もここに属する - `cargo run --bin <app> -- queue:work`。 |
| `console db:seed`、`console model:prune`、`console <your-command>` | プロジェクトごとの `console` バイナリ（`src/bin/console.rs`） | ユーザーのクレートにコンパイルされたユーザー型（シーダー、コマンド、プルーン可能なモデル）を必要とする、ワンショットのコマンド。 |

この分割は意図的なものです。サーバーバイナリは、`serve`、`migrate`、`queue:work` などを選ぶために、すでにclapパーサーを必要としています。そのライフサイクルを共有するデーモンは、そこに存在します。コンソールバイナリは、それ以外のすべて - 短命で、ユーザー定義で、型が豊かなもの - のために存在します。新しい実行時コマンドは、プロジェクトの `console` バイナリがディスパッチする `#[command]` / `#[derive(Command)]` に属します。

## ベストプラクティス

### ハンドラは小さく保ち、共有サービスにはコンテナ経由でアクセスする

`#[command]` はCLIの形をしたラッパーであり、ビジネスロジックは `Action`、サービス、あるいはモデル上のメソッドに置くべきです。ハンドラは引数をパースし、コンテナからサービスを解決し、それに委ねます。そうしておけば、同じロジックを、単体テストからも、HTTPルートからも、コンソールからも、同じようにテストできます。

```rust
#[command(name = "users:purge")]
pub async fn users_purge(args: Vec<String>) -> Result<(), FrameworkError> {
    let action = App::resolve::<PurgeStaleUsers>()?;
    action.execute(parse(args)?).await
}
```

`App::resolve` は `Result<T, FrameworkError::ServiceUnresolved(_)>` を返します - `App::get`（こちらは `Option` を返します）の `?` 版です。表面の全体像については[サービス コンテナ](container.md)を参照してください。

### 関連するコマンドにはネームスペースを使う

`:` でグループ化してください: `mail:send`、`mail:retry`、`mail:queue:work`。ディスパッチャーはそれを不透明な文字列として扱いますが、人間は `send-mail`、`retry-mail`、`mail-queue-work` よりも `mail:*` の方が見渡しやすいものです。

### 構造化データは出力せず、返す

コンソールのハンドラは、人間が読める出力のためにstdoutへ出力します。下流のツールがその出力を消費する必要がある場合は、機械可読なJSONをstdoutへ、ステータス行をstderrへ出力する `console <name> --json` のバリアントを書いてください。人間が読める経路に、両方の対象読者の責任を負わせないでください。

### 終了コードを契約として扱う

`FrameworkError` → `ExitCode::FAILURE` が、唯一の失敗経路です。ハンドラの内部から `std::process::exit(custom_code)` を呼ばないでください - `Err(...)` を返し、バイナリの `main` に変換させてください。将来のツール群（CIのゲート、監督下のワーカー）は、終了コードを読むだけで済みます。

## リファレンス

| シンボル                                    | 目的                                       |
|-------------------------------------------|-----------------------------------------------|
| `suprnova::Command`（derive）              | `clap::Parser` を導出した構造体を、型付きのコンソールコマンドとして登録する。`TypedCommand` と対になる。 |
| `suprnova::TypedCommand`（トレイト）          | `async fn run(self) -> Result<(), FrameworkError>` を持つトレイト - 型付きコマンドの本体。 |
| `suprnova::command`（アトリビュート）           | `Vec<String>` を受け取る非同期fnを、生の引数を扱うコンソールコマンドとして登録する。 |
| `suprnova::console::dispatch_argv(argv)`  | 登録済みのすべてのエントリから1本のclapパーサーツリーを構築し、argvをパースして、ハンドラへルーティングする。遅延初期化なし - テストやプログラムからの呼び出しに便利。 |
| `suprnova::console::dispatch_argv_with_init(argv, init)` | `dispatch_argv` と同じだが、clapのargvパースとマッチしたハンドラの間で `init` クロージャを実行する。initが発火するのは、実際のサブコマンドがマッチしたときだけ - `--help` / `--version` / パースエラーの各経路はそれをスキップする。スキャフォルドされた `console` バイナリが使っているのはこちら。 |
| `suprnova::console::set_version(&'static str)` | `--version` と `--help` の中で表に出るバージョン文字列を登録する。`main` の先頭で一度だけ呼ぶ。最初の登録が勝つ。 |
| `suprnova::console::find(name)`           | 正確な名前で、登録済みのコマンドを引く。   |
| `suprnova::two_column_detail(left, right)` | 名前、ドットリーダー、そしてステータスの語を、80カラムの1行の進捗行として描画する。Laravelの `$this->components->twoColumnDetail(...)` を映している。 |
| `suprnova::console::list()`               | 登録済みのすべてのコマンドを、名前順に並べたもの。      |
| `suprnova::CommandEntry`                  | インベントリのレコード: `{ name, description, clap_builder, handler }`。両方のマクロによって提出される。 |
| `suprnova::CommandHandler`                | ハンドラの関数ポインタ型: `fn(&clap::ArgMatches) -> Pin<Box<dyn Future<...>>>`。 |
| `FrameworkError::silent()` / `.is_silent()` | ディスパッチャーがstderrへ出力**しない**エラーを構築 / 検出する。clapがすでにパースエラーを端末へ書き込んでいる場合に、二重出力を抑えるため内部的に使われる。 |

## 次のステップ

- [アプリケーション ブートストラップ](bootstrap.md) - `dispatch_argv_with_init` クロージャの内部で何が実行されるか
- [サービス コンテナ](container.md) - `App::resolve` と `App::get` の違い、そしてハンドラがどのように共有サービスへ到達するか
- [シーディング](seeding.md) - `db:seed` が実際に何を呼び出すか
- [Eloquent](eloquent.md) - `Prunable`、`MassPrunable`、そして `model:prune` がどのようにレジストリを走査するか
- [タスク スケジューリング](scheduling.md) - その非対称性: スケジューラーのデーモンはコンソールではなくアプリバイナリ上に存在する
