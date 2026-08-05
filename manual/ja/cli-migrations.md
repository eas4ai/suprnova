# CLI マイグレーション

`suprnova` 開発者CLIは、SeaORMのマイグレーションランナーを駆動するために、あなたのアプリケーションバイナリへシェルします。そのため、開発者のターミナルから実行しても、CIから実行しても、あるいはサーバー起動時に暗黙的に実行されても、同じマイグレーションの集合が実行されます。マイグレーションファイルを作成し、適用し、ロールバックし、生成されたSeaORMエンティティをスキーマと同期させ続けるために、これらのコマンドを使ってください。

スキーマを執筆するためのAPI（カラムの型、インデックス、外部キー、完全な `MigrationTrait`）については、[マイグレーション](migrations.md)を参照してください。スキーマが定着した後にテストデータを挿入するには、[シーディング](seeding.md)を参照してください。

## make:migration

`src/migrations/` の下に新しいマイグレーションファイルを生成し、`src/migrations/mod.rs` の中の `Migrator` に配線します。

```bash
suprnova make:migration <name>
```

`<name>` はsnake_caseに正規化されます。ジェネレーターは標準的な命名パターンを認識し、それを使って `DeriveIden` のenumを選びます:

- `create_<table>_table` - `create_table` の本体をスキャフォルドする
- `add_<column>_to_<table>` - `alter_table` のスタブをスキャフォルドする
- `drop_<table>_table` - `drop_table` の本体をスキャフォルドする
- それ以外 - 名前をテーブル識別子として使う

### 例

```bash
suprnova make:migration create_users_table
suprnova make:migration add_email_to_users
suprnova make:migration drop_legacy_sessions_table
```

### 生成されるファイル

ファイルは `src/migrations/m{YYYYMMDD}_{HHMMSS}_<name>.rs`（例えば `m20260530_142301_create_users_table.rs`）に書き込まれ、`Migrator::migrations()` のvecに追加されます。

```rust
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Users::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Users::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(Users::CreatedAt)
                            .timestamp()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .col(
                        ColumnDef::new(Users::UpdatedAt)
                            .timestamp()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Users::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
    CreatedAt,
    UpdatedAt,
}
```

生成されたファイルを編集して、カラム、インデックス、制約を宣言してください。完全なスキーマビルダーの表面については、[マイグレーション](migrations.md)を参照してください。

## migrate

`src/migrations/` の中の保留中のマイグレーションをすべて実行します。

```bash
suprnova migrate
```

CLIは `cargo run -- migrate` へシェルアウトするため、あなたのアプリの `Application` ランナーが作業を行います - `serve` が使うのと同じバイナリ、同じ `Migrator`、同じデータベース接続です。

```
Running migrations...
Migrations completed successfully!
```

serve / web:runの経路は、`--no-migrate` でオプトアウトしない限り、あるいは失敗を越えて続行するよう `SUPRNOVA_AUTO_MIGRATE_BEST_EFFORT=true` を設定しない限り、ソケットをバインドする前に自動で `migrate` を実行します。自動マイグレーション中のマイグレーションエラーは、サーバーが起動する前に非ゼロで終了します。フェイルクローズの契約については、`framework/src/app/mod.rs` を参照してください。

## migrate:status

すべてのマイグレーションの適用済み/保留中の状態を出力します。

```bash
suprnova migrate:status
```

```
Migration status:
...SeaORMがフォーマットした、適用済み/保留中のマイグレーションの表...
```

レポートの本体はSeaORMの `MigratorTrait::status` から来るため、正確なフォーマットは、あなたのアプリが依存するSeaORMのバージョンに追随します。

## migrate:rollback

直近に適用されたマイグレーション（あるいは直近の `N` 件）をロールバックします。

```bash
suprnova migrate:rollback [--step <N>]
```

| オプション | デフォルト | 説明 |
|---|---|---|
| `--step <N>` | `1` | ロールバックするマイグレーションの数 |

```bash
# 1件のマイグレーションをロールバックする
suprnova migrate:rollback

# 直近の3件をロールバックする
suprnova migrate:rollback --step 3
```

```
Rolling back 3 migration(s)...
Rollback completed successfully!
```

各マイグレーションの `down()` は、適用の逆順で実行されます。失敗した `down()` は非ゼロで終了し、チェーンの残りには触れません - それ以上は何も試みられません。

## migrate:fresh

データベース内のすべてのテーブルを削除し、すべてのマイグレーションをゼロから再実行します。

```bash
suprnova migrate:fresh
```

```
WARNING: Dropping all tables and re-running migrations...
Database refreshed successfully!
```

これは、接続されているデータベース内のすべてのデータを破壊します。ローカル開発とテストのセットアップのためのものであり、データが重要な意味を持つ環境のためではありません。

### 本番環境の保護

本番環境の外では、プロンプトなしで即座に実行されます - ローカルのデータベースを削除するのは日常的なことであり、常に同じように答える確認は、それを読まなくなるよう仕向けてしまいます。

`APP_ENV` が本番環境に解決されると、2種類の異なる証明を要求します:

```bash
suprnova migrate:fresh --force   # …その後、尋ねられたら環境名をタイプ入力する
```

1. **`--force`** は、コマンドをタイプ入力した瞬間の意図を証明します。
2. **対話端末でのタイプ入力による確認**は、人間がそこにいることを証明します。

端末の要件こそが、2つ目の要点です。それがなければ、デプロイスクリプト内の `echo production | suprnova migrate:fresh --force` がプロンプトに自動で答えてしまい、確認はもう1つのフラグに過ぎなくなってしまいます。そのため、非インタラクティブなstdinは、`--force` があっても拒否されます。

正確な環境名以外の何かを入力すると、1つのテーブルすら削除される前に中止されます。

同じゲートは、あなたのアプリケーションバイナリ自身のサブコマンド（`./app migrate:fresh --force`）にも適用されます。これは、本番デプロイが実際に実行するものです。

## db:sync

現在のデータベーススキーマから `src/models/entities/` のSeaORMエンティティファイルを再生成し、（`src/bin/migrate.rs` が存在する場合は）先に保留中のマイグレーションを実行します。

```bash
suprnova db:sync [--skip-migrations] [--regenerate-models]
```

| オプション | 説明 |
|---|---|
| `--skip-migrations` | マイグレーションのパスをスキップし、エンティティだけを再生成する |
| `--regenerate-models` | `src/models/entities/<table>.rs` だけでなく、`src/models/<table>.rs` ファイルも上書きする |

### 実際に行われること

1. （オプション）保留中のマイグレーションを実行する。デフォルトのスキャフォルドは `src/bin/migrate.rs` を同梱しないため、このステップはno-opであり、`Migration binary not found, skipping migrations` を出力する。デフォルトのプロジェクトでは、まず `suprnova migrate` を実行し、その後 `suprnova db:sync --skip-migrations` を実行する。
2. `DATABASE_URL` に接続し、すべてのユーザーテーブルをイントロスペクトし（`seaql_migrations` と `_` で始まる名前はスキップする）、テーブルごとに1つのエンティティファイルを `src/models/entities/<table>.rs` に書き込む。
3. `src/models/<table>.rs` に、薄いユーザー向けのモデルファイルを書き込む - ただし、そのファイルがまだ存在しない場合だけであり、あなたが手で書いたアクセサ、スコープ、オブザーバーフックが生き残るようにするためです。
4. `--regenerate-models` はステップ3の保護を上書きし、それらのユーザーファイルを上書きします。まだカスタマイズしていないとき、あるいはバックアップがあるときに使ってください。

### 典型的なワークフロー

```bash
# 1. マイグレーションを作成する
suprnova make:migration create_posts_table
# （src/migrations/m..._create_posts_table.rs を編集する）

# 2. それを適用する
suprnova migrate

# 3. 新しいテーブルがコードから到達可能になるよう、エンティティを再生成する
suprnova db:sync --skip-migrations
```

### Suprnovaが異なる設計を選んだ理由

Laravelには、`db:seed` を含む、あらゆるフレームワークのコマンドを所有する、1つのグローバルな `artisan` があります。Suprnovaは、これを2つに分割します:

- `suprnova` 開発者CLI（この章）は、プロジェクトのスキャフォルド、ジェネレーター、そしてマイグレーションコマンドを所有します。開発者のマシンごとに `cargo install` 経由で一度だけインストールされ、アプリの `Migrator` を必要とする作業を行うために、あなたのアプリバイナリへシェルします。
- あなたのプロジェクトの `src/bin/console.rs` からビルドされる、プロジェクトごとの `console` バイナリは、`db:seed`、あなたの `#[command]` アノテーション付きハンドラ、`queue:work`、`schedule:run`、`workflow:work`、そしてあなたのアプリのブートストラップ、コンテナのバインディング、登録済みのオブザーバーを必要とする、その他のワンショットのタスクを所有します。

マイグレーションコマンドが開発者CLIの上にあるのは、それらがあなたのブートストラップに依存しない、決定的な形を持つからです。あなたのサービスコンテナや登録済みのシーダーを必要とするものはすべて、プロジェクトごとのconsoleバイナリの上にあります。完全なconsoleの表面については、[コンソール](console.md)を参照してください。

## db:seed

`suprnova` CLIのコマンドではありません。プロジェクトごとのconsoleバイナリ経由で、シーダーを実行してください:

```bash
cargo run --bin console -- db:seed
cargo run --bin console -- db:seed --class=UsersSeeder
```

シーダーのレジストリ、順序のルール、そして `--class` のマッチングは、[シーディング](seeding.md)で扱われています。フレームワークは `db:seed` を組み込みのconsoleコマンドとして出荷します - あなたのスキャフォルドは、あなた側の配線なしにそれを手に入れますが、それを呼び出すのは `console` 経由であり、`suprnova` 経由ではありません。

## まとめ

| コマンド | 内容 |
|---|---|
| `suprnova make:migration <name>` | 新しいマイグレーションファイルをスキャフォルドし、`Migrator` に登録する |
| `suprnova migrate` | 保留中のマイグレーションを実行する |
| `suprnova migrate:status` | 適用済み/保留中の状態を表示する |
| `suprnova migrate:rollback [--step N]` | 直近の `N` 件のマイグレーションをロールバックする（デフォルトは1） |
| `suprnova migrate:fresh` | すべてのテーブルを削除し、すべてのマイグレーションを再実行する |
| `suprnova db:sync [--skip-migrations] [--regenerate-models]` | 稼働中のスキーマからSeaORMエンティティを再生成する |
| `cargo run --bin console -- db:seed` | 登録済みのシーダーを実行する（プロジェクトごとのconsole、`suprnova` CLIではない） |

## 次のステップ

- [マイグレーション](migrations.md) - スキーマビルダーAPI: テーブル、カラム、インデックス、外部キー
- [シーディング](seeding.md) - シーダーの作成と `db:seed` のconsoleコマンド
- [コンソール](console.md) - プロジェクトごとの `console` バイナリと `#[command]` ハンドラ
- [データベース](database.md) - コネクション、ドライバー、トランザクション、クエリビルダー
- [CLI 概要](cli.md) - すべての `suprnova` サブコマンドの一覧
