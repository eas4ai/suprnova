# マイグレーション

マイグレーションは、あなたのスキーマがどのように進化するかを記述します - 各ファイルは、フレームワークがタイムスタンプの順序で実行する、`up()` と `down()` メソッドを持つ小さなRust構造体です。テーブル、カラム、インデックス、外部キーを変更するときは、いつでもこれを使ってください。その変更は、各場所で同じmigrateコマンドを実行することで、あなたのラップトップからステージング、そして本番へと移動していきます。

Suprnovaのマイグレーションは、その内部ではSeaORMのマイグレーションです。CLIがそれらを生成し、`Migrator` がそれらを集約し、`Application::migrations::<Migrator>()` がそれらをあなたのアプリの起動へ差し込みます。コマンドごとの完全なリファレンス（フラグ、出力サンプル、終了コード）については、[CLI マイグレーション](cli-migrations.md)を参照してください。この章は、ファイルの*内側*に何を置くかをカバーします。

## マイグレーションを作成する

新しいマイグレーションファイルを生成します:

```bash
suprnova make:migration create_users_table
```

ジェネレーターは、タイムスタンプ付きのファイルを `src/migrations/` の下に書き込み（初回はディレクトリも作成します）、それを `Migrator` に登録します:

```
src/migrations/
├── mod.rs                              ← Migrator（CLI管理）
└── m20240115_120000_create_users_table.rs
```

ファイル名は `m{YYYYMMDD}_{HHMMSS}_<name>.rs` です。順序はファイル名によって決まるため、タイムスタンプのプレフィックスこそが、決定的な適用順序を強制するものです。

### ジェネレーターが出力するもの

`make:migration create_users_table` は、この骨格を生成します:

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

ジェネレーターは、マイグレーション名からテーブル名を推測します（`create_X_table` → `X`、`add_Y_to_X` → `X`、`drop_X_table` → `X`）。それ以外はすべて、そのままのリテラルな名前になります。

### Migrator

`src/migrations/mod.rs` は、あらゆるマイグレーションを単一の `Migrator` へ集約し、`MigratorTrait` がそれを巡回します。CLIは、あなたが `make:migration` を実行するたびにこのファイルを保守するため、手で触ることはほとんどありません:

```rust
pub use sea_orm_migration::prelude::*;

mod m20240115_120000_create_users_table;
mod m20240115_130000_create_posts_table;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20240115_120000_create_users_table::Migration),
            Box::new(m20240115_130000_create_posts_table::Migration),
        ]
    }
}
```

マイグレーターをあなたのアプリの `main.rs` へ配線してください。そうすれば、`serve`、`migrate`、`migrate:status`、`migrate:rollback`、`migrate:fresh` はすべて、同じリストを見るようになります:

```rust
use suprnova::Application;

#[suprnova::main]
async fn main() {
    Application::new()
        .config(my_app::config::register)
        .bootstrap(my_app::bootstrap::bootstrap)
        .routes(my_app::routes::register)
        .migrations::<my_app::migrations::Migrator>()
        .run()
        .await
}
```

スキャフォルダーは、`suprnova new` の際にこれを代わりに書いてくれます。

### Suprnovaが異なる設計を選んだ理由

フレームワークの大部分は、意図的にSeaORMを隠します - あなたが書くのは `#[suprnova::model]` と `User::query().db_where(...)` であり、`Entity::find().filter(...)` ではありません。マイグレーションは、私たちが `sea_orm_migration::prelude::*` を見える状態のまま残す、唯一の場所です。理由は2つあります。

第一に、スキーマビルダーのDSLは本当に良く、その中のすべての名前（`Table`、`ColumnDef`、`Index`、`ForeignKey`、`Expr`、`ForeignKeyAction`、`DeriveIden`、…）を再エイリアスしても、より長いimport行が手に入るだけで、それ以外には何も得られません。第二に、マイグレーションファイルは純粋なRustです - あなたのCIのコンパイラがそれらを検証します - そして、それはどんなDSLの再エイリアスよりも多くのタイプミスを捕まえます。私たちはマイグレーションをスキーマ・アズ・コードとして扱っており、正規のSeaORMの名前こそが、スキーマの語彙*そのもの*です。

フレームワークが再エクスポートしていないSeaORMの型がどうしても必要になった場合の逃げ道は、`use suprnova::sea_orm;` です。それが必要になることは、ほとんどありません。

## マイグレーションの構造

すべてのマイグレーションは、2つのメソッドを持ちます:

```rust
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    // 変更を適用する
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> { /* ... */ }

    // 変更を元に戻す
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> { /* ... */ }
}
```

どちらの分岐も `Result<(), DbErr>` を返します - `?` でエラーを伝播させてください。フレームワークは、失敗したマイグレーションを非ゼロの終了コードへ変換するため、デプロイパイプラインは中断します。

## スキーマ操作

### テーブルを作成する

```rust
use sea_orm_migration::prelude::*;

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
                .col(ColumnDef::new(Users::Email).string().not_null().unique_key())
                .col(ColumnDef::new(Users::Name).string().not_null())
                .col(ColumnDef::new(Users::PasswordHash).string().not_null())
                .col(ColumnDef::new(Users::CreatedAt).timestamp().not_null())
                .col(ColumnDef::new(Users::UpdatedAt).timestamp().not_null())
                .to_owned(),
        )
        .await
}

// テーブルとカラムの識別子を定義する
#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
    Email,
    Name,
    PasswordHash,
    CreatedAt,
    UpdatedAt,
}
```

### テーブルを削除する

```rust
async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
        .drop_table(Table::drop().table(Users::Table).to_owned())
        .await
}
```

### カラムの型

| メソッド | データベースの型 | 備考 |
|--------|---------------|-------|
| `integer()` | INTEGER | 32ビット整数 |
| `big_integer()` | BIGINT | 64ビット整数 |
| `small_integer()` | SMALLINT | 16ビット整数 |
| `float()` | FLOAT | 浮動小数点数 |
| `double()` | DOUBLE | 倍精度浮動小数点数 |
| `decimal()` | DECIMAL | 固定小数点数 |
| `string()` | VARCHAR(255) | 可変長文字列 |
| `string_len(n)` | VARCHAR(n) | カスタム長の文字列 |
| `text()` | TEXT | 長いテキスト |
| `boolean()` | BOOLEAN | 真偽値 |
| `timestamp()` | TIMESTAMP | 日時 |
| `date()` | DATE | 日付のみ |
| `time()` | TIME | 時刻のみ |
| `blob()` | BLOB | バイナリデータ |
| `json()` | JSON | JSONデータ |
| `uuid()` | UUID | UUID型 |

### カラムの修飾子

```rust
ColumnDef::new(Column::Name)
    .string()
    .not_null()                                // NOT NULL 制約
    .null()                                    // NULLを許可する（デフォルト）
    .default("value")                          // デフォルト値
    .default(Expr::current_timestamp())        // 関数によるデフォルト（例: NOW()）
    .unique_key()                              // UNIQUE 制約
    .primary_key()                             // PRIMARY KEY
    .auto_increment()                          // AUTO_INCREMENT
```

代用の主キーには、本物のテーブルでは `big_integer().auto_increment().primary_key()` を優先してください - `INTEGER`（32ビット）は小さなルックアップテーブルには十分ですが、スキャフォルドされた `users`、`sessions`、そして類似のテーブルはすべて `BIGINT` を使います。4バイトのカウンターは、3年後に後悔する類の制約だからです。

## カラムを追加する

```rust
async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
        .alter_table(
            Table::alter()
                .table(Users::Table)
                .add_column(
                    ColumnDef::new(Users::PhoneNumber)
                        .string()
                        .null()
                )
                .to_owned(),
        )
        .await
}

async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
        .alter_table(
            Table::alter()
                .table(Users::Table)
                .drop_column(Users::PhoneNumber)
                .to_owned(),
        )
        .await
}
```

## カラムを変更する

```rust
async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
        .alter_table(
            Table::alter()
                .table(Users::Table)
                .modify_column(
                    ColumnDef::new(Users::Name)
                        .string_len(500)  // VARCHAR(255)をVARCHAR(500)へ変更する
                        .not_null()
                )
                .to_owned(),
        )
        .await
}
```

## カラムをリネームする

```rust
async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
        .alter_table(
            Table::alter()
                .table(Users::Table)
                .rename_column(Users::Name, Users::FullName)
                .to_owned(),
        )
        .await
}
```

## インデックス

### インデックスを作成する

```rust
async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
        .create_index(
            Index::create()
                .name("idx_users_email")
                .table(Users::Table)
                .col(Users::Email)
                .unique()  // オプション: ユニークにする
                .to_owned(),
        )
        .await
}
```

### 複合インデックス

```rust
manager
    .create_index(
        Index::create()
            .name("idx_posts_user_created")
            .table(Posts::Table)
            .col(Posts::UserId)
            .col(Posts::CreatedAt)
            .to_owned(),
    )
    .await
```

### インデックスを削除する

```rust
async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
        .drop_index(Index::drop().name("idx_users_email").to_owned())
        .await
}
```

## 外部キー

### 外部キーを追加する

```rust
async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
        .create_table(
            Table::create()
                .table(Posts::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(Posts::Id)
                        .integer()
                        .not_null()
                        .auto_increment()
                        .primary_key(),
                )
                .col(ColumnDef::new(Posts::UserId).integer().not_null())
                .col(ColumnDef::new(Posts::Title).string().not_null())
                .col(ColumnDef::new(Posts::Content).text().not_null())
                .foreign_key(
                    ForeignKey::create()
                        .name("fk_posts_user")
                        .from(Posts::Table, Posts::UserId)
                        .to(Users::Table, Users::Id)
                        .on_delete(ForeignKeyAction::Cascade)
                        .on_update(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await
}
```

### 外部キーのアクション

| アクション | 説明 |
|--------|-------------|
| `Cascade` | 子の行を自動的に削除/更新する |
| `SetNull` | 外部キーをNULLに設定する |
| `SetDefault` | 外部キーをデフォルト値に設定する |
| `Restrict` | 参照されている場合は削除/更新を防ぐ |
| `NoAction` | Restrictと似ている |

## マイグレーションのワークフロー

典型的な変更は、4つのステップを経ます:

```bash
# 1. ファイルを生成する（src/migrations/m{ts}_create_posts_table.rsを作成し、
#    src/migrations/mod.rsを更新する）。
suprnova make:migration create_posts_table

# 2. src/migrations/m{ts}_create_posts_table.rsを編集して、スキーマを定義する。

# 3. マイグレーションを適用する。
suprnova migrate

# 4. 実際のスキーマからSeaORMのエンティティファイルを再生成し、モデルが
#    新しい形に対してコンパイルされるようにする。`db:sync`は、先に
#    保留中のマイグレーションも実行する（そのステップをスキップするには
#    --skip-migrationsを使う）。
suprnova db:sync
```

`db:sync` は、自動生成されるエンティティのグルーコードを `src/models/entities/<table>.rs` へ、ユーザーが編集可能なスタブを `src/models/<table>.rs` へ書き込みます。再実行するとエンティティファイルが更新されます - `--regenerate-models` を渡さない限り、あなたのユーザースタブはそのまま残されます（これを渡すとそれらは上書きされます - カスタムメソッドは別の場所に置くか、実行前にバージョン管理してください）。

### serve時の自動マイグレーション

`suprnova serve` と `suprnova web:run` は、HTTPソケットを開く前に、保留中のマイグレーションを適用します。デフォルトのポリシーは**フェイルクローズ**です: `up()` がエラーになると、プロセスはbindの前に非ゼロで中止します。そのため、壊れたマイグレーションがトラフィックに到達することは決してありません。

2つの逃げ道があります:

| フラグ / 環境変数 | 効果 |
|---|---|
| `--no-migrate`（`serve` / `web:run` に対して） | 自動マイグレーションのステップを完全にスキップする。マイグレーションが別個のデプロイステップから実行される場合に有用 |
| `SUPRNOVA_AUTO_MIGRATE_BEST_EFFORT=true` | 従来の「ログを出して続行する」振る舞いへオプトバックする。プロセスは、マイグレーションのエラーが起きても起動を続ける。本番環境では非推奨 |

バックグラウンドワーカー（`queue:work`、`workflow:work`、`schedule:run`）は自動マイグレーションを*行いません* - N個のワーカーから並行してマイグレーションを実行すると競合してしまうため、起動時にはスキーマがすでに整っていることを前提とします。

### テストの中でマイグレーションを実行する

`TestDatabase::fresh::<Migrator>()` は、分離されたインメモリのSQLiteデータベースを立ち上げ、すべてのマイグレーションを実行し、その接続をテストコンテナへ束縛します。そのため、`DB::connection()` と `#[inject]` はそれを解決します:

```rust
use suprnova::testing::TestDatabase;
use crate::migrations::Migrator;

#[tokio::test]
async fn users_table_is_created() {
    let db = TestDatabase::fresh::<Migrator>().await.unwrap();
    // `db`はテストの終わりにドロップし、コンテナをクリアする。
}
```

完全なパターン（ファクトリー、並列に対する安全性、インメモリSQLiteの代わりに本物のドライバーを選ぶこと）については、[データベース テスト](database-testing.md)を参照してください。

## ベストプラクティス

### 常に `down()` を実装する

ロールバックを可能にするために、常に `down()` を実装してください:

```rust
// 良い例: 元に戻せるマイグレーション
async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager.create_table(/* ... */).await
}

async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager.drop_table(/* ... */).await
}
```

### 説明的な名前を使う

```bash
# 良い例: 変更を説明している
suprnova make:migration add_email_verified_to_users
suprnova make:migration create_order_items_table
suprnova make:migration add_index_to_posts_slug

# 悪い例: あいまいな名前
suprnova make:migration update_users
suprnova make:migration change_table
```

### マイグレーションごとに1つの変更

マイグレーションは、単一の変更に集中させてください:

```bash
# 良い例: 分離されたマイグレーション
suprnova make:migration create_categories_table
suprnova make:migration add_category_id_to_posts

# 避けるべき例: 1つのマイグレーションに無関係な複数の変更を入れる
```

### マイグレーションを両方向でテストする

コミットする前に、両方向が機能することを確認してください:

```bash
suprnova migrate           # 適用
suprnova migrate:rollback  # ロールバック
suprnova migrate           # 再び適用
```

## CLIコマンド一覧

| コマンド | 説明 |
|---------|-------------|
| `suprnova make:migration <name>` | 新しいマイグレーションを作成する |
| `suprnova migrate` | 保留中のマイグレーションをすべて実行する |
| `suprnova migrate:status` | マイグレーションの状態を表示する |
| `suprnova migrate:rollback` | 直前のマイグレーションをロールバックする |
| `suprnova migrate:rollback --step 3` | 直前の3件のマイグレーションをロールバックする |
| `suprnova migrate:fresh` | すべてのテーブルを削除し、すべてのマイグレーションを再実行する |
| `suprnova db:sync` | マイグレーションを実行し、エンティティファイルを再生成する |
| `suprnova db:sync --skip-migrations` | マイグレーションを適用せずにエンティティファイルを再生成する |
| `suprnova db:sync --regenerate-models` | ユーザーが編集可能なモデルスタブも上書きする |

コマンドごとの完全なリファレンス（フラグ、出力サンプル、終了コード）については、[CLI マイグレーション](cli-migrations.md)を参照してください。

## 次のステップ

- [CLI マイグレーション](cli-migrations.md) - `migrate*` と `db:sync` のフラグごとのリファレンス
- [データベース](database.md) - 接続の設定、トランザクション、読み取り/書き込みの分割
- [Eloquent](eloquent.md) - あなたのマイグレーションが供給するモデル層
- [シーディング](seeding.md) - スキーマが存在するようになったテーブルにデータを投入する
- [データベース テスト](database-testing.md) - `TestDatabase::fresh::<Migrator>()` と並列に対して安全なパターン
