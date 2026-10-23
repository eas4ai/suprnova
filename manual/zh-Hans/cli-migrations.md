# CLI 迁移

`suprnova` 这个开发者 CLI 会 shell 进您的应用二进制文件，去驱动 SeaORM 的迁移运行器，所以不管您是从一个开发者终端、从 CI，还是隐式地在服务器启动时运行它，执行的都是同一套迁移。用这些命令来编写迁移文件、应用它们、回滚，以及让您生成的 SeaORM 实体和架构保持同步。

架构编写 API（列类型、索引、外键，以及完整的 `MigrationTrait`）请参见[迁移](migrations.md)。架构落地之后插入测试数据，请参见[数据填充](seeding.md)。

## make:migration

在 `src/migrations/` 下生成一个新的迁移文件，并把它接入 `src/migrations/mod.rs` 里的 `Migrator`。

```bash
suprnova make:migration <name>
```

`<name>` 会被规范化成 snake_case。这个生成器能识别标准的命名模式，并用它们来挑选 `DeriveIden` 枚举：

- `create_<table>_table` - 脚手架出一个 `create_table` 的主体
- `add_<column>_to_<table>` - 脚手架出一个 `alter_table` 的骨架
- `drop_<table>_table` - 脚手架出一个 `drop_table` 的主体
- 其他任何形式 - 把这个名字当作表标识符来用

### 示例

```bash
suprnova make:migration create_users_table
suprnova make:migration add_email_to_users
suprnova make:migration drop_legacy_sessions_table
```

### 生成的文件

这个文件会被写入 `src/migrations/m{YYYYMMDD}_{HHMMSS}_<name>.rs`（例如 `m20260530_142301_create_users_table.rs`），并被加进 `Migrator::migrations()` 这个 vec 里。

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

编辑这个生成出来的文件，去声明您的列、索引和约束。完整的架构构造器表面请参见[迁移](migrations.md)。

## migrate

运行 `src/migrations/` 里每一条待处理的迁移。

```bash
suprnova migrate
```

这个 CLI 会 shell 出去调用 `cargo run -- migrate`，这样实际做事的就是您应用的 `Application` 运行器 - 同一个二进制文件、同一个 `Migrator`、和 `serve` 会用的同一个数据库连接。

```
Running migrations...
Migrations completed successfully!
```

serve / web:run 这条路径，会在绑定套接字之前自动运行 `migrate`，除非您用 `--no-migrate` 选择退出，或者设置 `SUPRNOVA_AUTO_MIGRATE_BEST_EFFORT=true` 来让它在一次失败之后继续走下去。自动迁移期间的一次迁移错误，会在服务器启动之前以非零状态退出；失败关闭这份契约请参见 `framework/src/app/mod.rs`。

## migrate:status

打印每一条迁移的已应用/待处理状态。

```bash
suprnova migrate:status
```

```
Migration status:
...此处是 SeaORM 格式化的已应用/待处理迁移表格...
```

这份报告的主体来自 SeaORM 的 `MigratorTrait::status`，所以确切的格式会跟着您应用所依赖的 SeaORM 版本走。

## migrate:rollback

回滚最后应用的那条迁移（或者最后 `N` 条）。

```bash
suprnova migrate:rollback [--step <N>]
```

| 选项 | 默认值 | 描述 |
|---|---|---|
| `--step <N>` | `1` | 要回滚的迁移条数 |

```bash
# 回滚一条迁移
suprnova migrate:rollback

# 回滚最后三条
suprnova migrate:rollback --step 3
```

```
Rolling back 3 migration(s)...
Rollback completed successfully!
```

每一条迁移的 `down()` 都按应用顺序的反顺序运行。一个失败的 `down()` 会以非零状态退出，链条里剩下的部分则原样不动 - 不会再尝试任何后续操作。

## migrate:fresh

删除数据库里的每一张表，从头重新运行每一条迁移。

```bash
suprnova migrate:fresh
```

```
WARNING: Dropping all tables and re-running migrations...
Database refreshed successfully!
```

这会摧毁所连接数据库里的全部数据。它是给本地开发和测试环境搭建用的，不是给任何数据有意义的环境用的。

### 生产环境防护

在生产环境之外，它会立刻运行，不带任何提示 - 丢弃一个本地数据库是家常便饭，而一个您每次都用同样方式回答的确认，只会训练您不再去读它。

当 `APP_ENV` 解析为生产环境时，它会要求两种不同的证明：

```bash
suprnova migrate:fresh --force   # ……然后在被要求时输入环境名
```

1. **`--force`** 证明的是您输入这条命令那一刻的意图。
2. **在一个交互式终端上敲入的确认**，证明的是有一个人在场。

这个终端要求正是第二条的意义所在。没有它，部署脚本里的 `echo production | suprnova migrate:fresh --force` 就会自动回答这个提示，那这个确认就只是另一个标志而已。所以，即便带着 `--force`，一个非交互式的 stdin 也会被拒绝。

除了那个精确的环境名之外，任何输入都会在删除第一张表之前就中止。

同样这道防护，也适用于您应用二进制文件自己的子命令（`./app migrate:fresh --force`），而这才是一次生产部署真正会运行的那个。

## db:sync

从当前的数据库架构重新生成 `src/models/entities/` 里的 SeaORM 实体文件，并且（当存在一个 `src/bin/migrate.rs` 时）先运行待处理的迁移。

```bash
suprnova db:sync [--skip-migrations] [--regenerate-models]
```

| 选项 | 描述 |
|---|---|
| `--skip-migrations` | 跳过迁移这一遍，只重新生成实体 |
| `--regenerate-models` | 也覆盖 `src/models/<table>.rs` 文件，不只是 `src/models/entities/<table>.rs` |

### 它做了什么

1. （可选）运行待处理的迁移。默认的脚手架不自带 `src/bin/migrate.rs`，所以这一步是个空操作，会打印 `Migration binary not found, skipping migrations`。在一个默认项目里，请先运行 `suprnova migrate`，再运行 `suprnova db:sync --skip-migrations`。
2. 连接到 `DATABASE_URL`，反射每一张用户表（跳过 `seaql_migrations`，以及任何以 `_` 开头的名字），把每张表的一个实体文件写入 `src/models/entities/<table>.rs`。
3. 在 `src/models/<table>.rs` 写入一份薄薄的、面向用户的模型文件 - 但只有在这个文件还不存在的时候才写，这样您手写的访问器、作用域和观察者钩子才能保留下来。
4. `--regenerate-models` 会越过第 3 步的这层保护，覆盖那些用户文件。当您还没有定制过它们，或者您有一份备份时，就用它。

### 典型工作流

```bash
# 1. 编写一条迁移
suprnova make:migration create_posts_table
# （编辑 src/migrations/m..._create_posts_table.rs）

# 2. 应用它
suprnova migrate

# 3. 重新生成实体，让这张新表能从代码里访问到
suprnova db:sync --skip-migrations
```

### 为什么 Suprnova 有所不同

Laravel 有一个全局的 `artisan`，拥有每一个框架命令，包括 `db:seed`。Suprnova 把这个拆成了两半：

- `suprnova` 这个开发者 CLI（本章讲的就是它）拥有项目脚手架、生成器和迁移命令。它通过 `cargo install` 在每台开发者机器上装一次，需要用到应用 `Migrator` 的工作，就 shell 进您的应用二进制文件去做。
- 一个逐项目的 `console` 二进制文件，由您项目的 `src/bin/console.rs` 构建而成，拥有 `db:seed`、您那些标注了 `#[command]` 的处理程序、`queue:work`、`schedule:run`、`workflow:work`，以及其他需要用到您应用的 bootstrap、容器绑定和已注册观察者的一次性任务。

迁移命令活在开发者 CLI 上，因为它们有一个不依赖您的 bootstrap 的、确定性的形态。任何需要用到您服务容器，或者您已注册填充器的东西，都活在逐项目的 console 二进制文件上。完整的 console 表面请参见[控制台](console.md)。

## db:seed

不是一个 `suprnova` CLI 命令。通过逐项目的 console 二进制文件来运行填充器：

```bash
cargo run --bin console -- db:seed
cargo run --bin console -- db:seed --class=UsersSeeder
```

填充器注册表、排序规则，以及 `--class` 的匹配方式，都在[数据填充](seeding.md)里讲了。这个框架把 `db:seed` 作为一个内置的 console 命令来发布 - 您的脚手架不需要您接任何线就能拿到它，但您确实要通过 `console` 来调用它，而不是通过 `suprnova`。

## 总结

| 命令 | 它做了什么 |
|---|---|
| `suprnova make:migration <name>` | 脚手架出一个新的迁移文件，并把它注册进 `Migrator` |
| `suprnova migrate` | 运行待处理的迁移 |
| `suprnova migrate:status` | 显示已应用/待处理的状态 |
| `suprnova migrate:rollback [--step N]` | 回滚最后 `N` 条迁移（默认 1） |
| `suprnova migrate:fresh` | 删除所有表，重新运行每一条迁移 |
| `suprnova db:sync [--skip-migrations] [--regenerate-models]` | 从存活的架构重新生成 SeaORM 实体 |
| `cargo run --bin console -- db:seed` | 运行已注册的填充器（逐项目的 console，不是 `suprnova` CLI） |

## 下一步

- [迁移](migrations.md) - 架构构造器 API：表、列、索引、外键
- [数据填充](seeding.md) - 编写填充器，以及 `db:seed` 这个 console 命令
- [控制台](console.md) - 逐项目的 `console` 二进制文件和 `#[command]` 处理程序
- [数据库](database.md) - 连接、驱动程序、事务、查询构造器
- [CLI 概览](cli.md) - 一览每一个 `suprnova` 子命令
