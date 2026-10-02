# 迁移

迁移描述的是您的架构如何演化 - 每个文件都是一个小小的 Rust 结构体，带着 `up()` 和 `down()` 方法，框架会按时间戳顺序运行它们。当您改动表、列、索引或外键时就用它们；同一条迁移命令在每个环境里各运行一次，这个改动就会从您的笔记本电脑，一路走到预发布环境，再走到生产环境。

Suprnova 的迁移，底层是 SeaORM 迁移。CLI 生成它们，`Migrator` 聚合它们，`Application::migrations::<Migrator>()` 把它们接进您应用的启动流程。完整的逐命令参考（标志、输出样例、退出码）请参见 [CLI 迁移参考](cli-migrations.md)；这一章讲的是该往文件*里面*放什么。

## 创建迁移

生成一个新的迁移文件：

```bash
suprnova make:migration create_users_table
```

生成器会在 `src/migrations/` 下写入一个带时间戳的文件（第一次会创建这个目录），并把它注册进 `Migrator`：

```
src/migrations/
├── mod.rs                              ← 迁移器（由 CLI 管理）
└── m20240115_120000_create_users_table.rs
```

文件名是 `m{YYYYMMDD}_{HHMMSS}_<name>.rs`；顺序按文件名排；正是这个时间戳前缀，强制了一个确定性的应用顺序。

### 生成器会产出什么

`make:migration create_users_table` 会产出这份骨架：

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

生成器会从这个迁移名推断出表名（`create_X_table` → `X`，`add_Y_to_X` → `X`，`drop_X_table` → `X`）。其他任何形式都会变成字面意思上的名字。

### 迁移器

`src/migrations/mod.rs` 把每一条迁移收拢进一个单一的 `Migrator`，供 `MigratorTrait` 遍历。您 `make:migration` 时，CLI 会维护这个文件，所以您很少需要手工去碰它：

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

把这个迁移器接进您应用的 `main.rs`，这样 `serve`、`migrate`、`migrate:status`、`migrate:rollback` 和 `migrate:fresh` 看到的就都是同一份列表：

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

脚手架工具会在 `suprnova new` 时替您写好这一段。

### 为什么 Suprnova 有所不同

框架的大部分都刻意把 SeaORM 藏起来 - 您写的是 `#[suprnova::model]` 和 `User::query().db_where(...)`，不是 `Entity::find().filter(...)`。迁移是我们留下 `sea_orm_migration::prelude::*` 可见的唯一一处。原因有两个。

第一，这套架构构造器 DSL 本身就很好用，把它里面的每一个名字（`Table`、`ColumnDef`、`Index`、`ForeignKey`、`Expr`、`ForeignKeyAction`、`DeriveIden`，……）都重新起个别名，只会换来一行更长的导入，别无所获。第二，迁移文件是纯粹的 Rust - 您的 CI 编译器会校验它们 - 这比任何 DSL 重新起别名都能抓出更多的笔误。我们把迁移当作代码即架构来对待，而那些标准的 SeaORM 名字，*正是*架构的词汇表。

如果您确实需要一个框架没有重新导出的 SeaORM 类型，脱围机制是 `use suprnova::sea_orm;`。您几乎用不上它。

## 迁移结构

每一条迁移都有两个方法：

```rust
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    // 应用这次变更
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> { /* ... */ }

    // 撤销这次变更
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> { /* ... */ }
}
```

两个分支都返回 `Result<(), DbErr>` - 用 `?` 把错误往上冒泡，框架会把一次失败的迁移变成一个非零的退出码，这样部署流水线就会中止。

## 架构操作

### 创建表

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

// 定义表和列的标识符
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

### 删除表

```rust
async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
        .drop_table(Table::drop().table(Users::Table).to_owned())
        .await
}
```

### 列类型

| 方法 | 数据库类型 | 备注 |
|--------|---------------|-------|
| `integer()` | INTEGER | 32 位整数 |
| `big_integer()` | BIGINT | 64 位整数 |
| `small_integer()` | SMALLINT | 16 位整数 |
| `float()` | FLOAT | 浮点数 |
| `double()` | DOUBLE | 双精度浮点数 |
| `decimal()` | DECIMAL | 定点数 |
| `string()` | VARCHAR(255) | 可变长度字符串 |
| `string_len(n)` | VARCHAR(n) | 自定义长度的字符串 |
| `text()` | TEXT | 长文本 |
| `boolean()` | BOOLEAN | 真/假 |
| `timestamp()` | TIMESTAMP | 日期和时间 |
| `date()` | DATE | 仅日期 |
| `time()` | TIME | 仅时间 |
| `blob()` | BLOB | 二进制数据 |
| `json()` | JSON | JSON 数据 |
| `uuid()` | UUID | UUID 类型 |

### 列修饰符

```rust
ColumnDef::new(Column::Name)
    .string()
    .not_null()                                // NOT NULL 约束
    .null()                                    // 允许 NULL（默认）
    .default("value")                          // 默认值
    .default(Expr::current_timestamp())        // 函数式默认值（例如 NOW()）
    .unique_key()                              // UNIQUE 约束
    .primary_key()                             // PRIMARY KEY
    .auto_increment()                          // AUTO_INCREMENT
```

对于代理主键，在真实的表上优先选用 `big_integer().auto_increment().primary_key()` - 对小巧的查找表来说 `INTEGER`（32 位）足够了，但脚手架出来的 `users`、`sessions` 之类的表全都用 `BIGINT`，因为一个 4 字节的计数器正是那种三年后您会后悔的约束。

## 新增列

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

## 修改列

```rust
async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
        .alter_table(
            Table::alter()
                .table(Users::Table)
                .modify_column(
                    ColumnDef::new(Users::Name)
                        .string_len(500)  // 把 VARCHAR(255) 改成 VARCHAR(500)
                        .not_null()
                )
                .to_owned(),
        )
        .await
}
```

## 重命名列

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

## 索引

### 创建索引

```rust
async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
        .create_index(
            Index::create()
                .name("idx_users_email")
                .table(Users::Table)
                .col(Users::Email)
                .unique()  // 可选：让它唯一
                .to_owned(),
        )
        .await
}
```

### 复合索引

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

### 删除索引

```rust
async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
        .drop_index(Index::drop().name("idx_users_email").to_owned())
        .await
}
```

## 外键

### 新增外键

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

### 外键动作

| 动作 | 描述 |
|--------|-------------|
| `Cascade` | 自动删除/更新子行 |
| `SetNull` | 把外键设为 NULL |
| `SetDefault` | 把外键设为默认值 |
| `Restrict` | 被引用时阻止删除/更新 |
| `NoAction` | 与 Restrict 类似 |

## 迁移工作流

一次典型的改动要走完四个步骤：

```bash
# 1. 生成这个文件（会创建 src/migrations/m{ts}_create_posts_table.rs，
#    并更新 src/migrations/mod.rs）。
suprnova make:migration create_posts_table

# 2. 编辑 src/migrations/m{ts}_create_posts_table.rs 来定义您的架构。

# 3. 应用这次迁移。
suprnova migrate

# 4. 从存活的架构重新生成 SeaORM 实体文件，让模型能针对这个新形态
#    编译。`db:sync` 还会先运行任何待处理的迁移
#    （用 --skip-migrations 跳过这一步）。
suprnova db:sync
```

`db:sync` 会把自动生成的实体粘合代码写到 `src/models/entities/<table>.rs`，把一份用户可编辑的骨架写到 `src/models/<table>.rs`。再次运行它会更新这些实体文件；您那些用户骨架会被放着不动，除非您传了 `--regenerate-models`（它会覆盖它们 - 在运行它之前，把自定义方法迁到别处，或者纳入版本控制）。

### serve 时自动迁移

`suprnova serve` 和 `suprnova web:run` 会在打开 HTTP 套接字之前，应用任何待处理的迁移。默认策略是**失败关闭**：如果 `up()` 报错，这个进程会在绑定之前以非零状态中止，这样一次损坏的迁移永远碰不到流量。

两个脱围机制：

| 标志 / 环境变量 | 效果 |
|---|---|
| `--no-migrate`（在 `serve` / `web:run` 上） | 完全跳过这次自动迁移步骤。当迁移是从一个单独的部署步骤运行时有用。 |
| `SUPRNOVA_AUTO_MIGRATE_BEST_EFFORT=true` | 重新选择接入旧版的“记录日志并继续”行为。这个进程会在一次迁移错误上照常启动。生产环境不推荐。 |

后台工作进程（`queue:work`、`workflow:work`、`schedule:run`）*不会*自动迁移 - 它们启动时假定架构已经就位，因为让 N 个工作进程并发地运行迁移会产生竞态。

### 在测试里运行迁移

`TestDatabase::fresh::<Migrator>()` 会启动一个隔离的内存 SQLite 数据库，运行每一条迁移，并把这个连接绑定进测试容器，这样 `DB::connection()` 和 `#[inject]` 就会解析到它：

```rust
use suprnova::testing::TestDatabase;
use crate::migrations::Migrator;

#[tokio::test]
async fn users_table_is_created() {
    let db = TestDatabase::fresh::<Migrator>().await.unwrap();
    // `db` 在这个测试结束时被丢弃，清空这个容器。
}
```

完整的模式（工厂、并行安全、挑一个真实驱动程序而不是内存 SQLite）请参见[数据库测试](database-testing.md)。

## 最佳实践

### 始终编写向下迁移

始终实现 `down()`，让回滚成为可能：

```rust
// 推荐：可逆的迁移
async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager.create_table(/* ... */).await
}

async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager.drop_table(/* ... */).await
}
```

### 使用有描述性的名字

```bash
# 推荐：描述了这次变更
suprnova make:migration add_email_verified_to_users
suprnova make:migration create_order_items_table
suprnova make:migration add_index_to_posts_slug

# 不推荐：含糊的名字
suprnova make:migration update_users
suprnova make:migration change_table
```

### 一次迁移只做一个改动

让每一条迁移都只专注于单一一个改动：

```bash
# 推荐：分开的迁移
suprnova make:migration create_categories_table
suprnova make:migration add_category_id_to_posts

# 避免：在一条迁移里塞进多个不相关的改动
```

### 双向测试迁移

提交之前，验证两个方向都能跑：

```bash
suprnova migrate           # 应用
suprnova migrate:rollback  # 回滚
suprnova migrate           # 再次应用
```

## CLI 命令一览

| 命令 | 描述 |
|---------|-------------|
| `suprnova make:migration <name>` | 创建一条新迁移 |
| `suprnova migrate` | 运行所有待处理的迁移 |
| `suprnova migrate:status` | 显示迁移状态 |
| `suprnova migrate:rollback` | 回滚最后一条迁移 |
| `suprnova migrate:rollback --step 3` | 回滚最后 3 条迁移 |
| `suprnova migrate:fresh` | 删除所有表并重新运行每一条迁移 |
| `suprnova db:sync` | 运行迁移并重新生成实体文件 |
| `suprnova db:sync --skip-migrations` | 不应用迁移，只重新生成实体文件 |
| `suprnova db:sync --regenerate-models` | 同时覆盖用户可编辑的模型骨架 |

完整的逐命令参考（标志、输出样例、退出码）请参见 [CLI 迁移参考](cli-migrations.md)。

## 下一步

- [CLI 迁移参考](cli-migrations.md) - `migrate*` 和 `db:sync` 逐标志的参考
- [数据库](database.md) - 连接配置、事务、读/写分离
- [Eloquent](eloquent.md) - 您的迁移所供给的那个模型层
- [数据填充](seeding.md) - 在架构存在之后填充表
- [数据库测试](database-testing.md) - `TestDatabase::fresh::<Migrator>()` 和并行安全的模式
