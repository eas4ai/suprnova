# CLI 概览

Suprnova 发布两个各有分工的二进制文件。全局的 `suprnova` - 只需要安装一次到 `~/.cargo/bin` - 负责脚手架新项目、生成代码、引导开发服务器，以及运行迁移。逐项目的 `console`，由每个应用自己的 `src/bin/console.rs` 构建而成，运行那些需要用到应用编译期类型的运行时命令（填充器、修剪器，还有您自己的 `#[command]` 处理程序）。这一章是一份地图；每个子命令都在[下一步](#下一步)下列出的相邻章节里有自己的深入介绍。

## 安装

CLI 通过 `cargo install --git` 分发。Suprnova 目前还不在 crates.io 上 - 原因请参见[安装章节里的发布前说明](installation.md#pre-launch-note)。

```bash
cargo install --git https://github.com/eas4ai/suprnova.git --tag v1.3.7 suprnova-cli
suprnova --version
```

以后要升级时，请传 `--force`：

```bash
cargo install --force --git https://github.com/eas4ai/suprnova.git --tag v1.3.7 suprnova-cli
```

## 两个二进制文件

| 二进制文件 | 构建自 | 用于 |
|---|---|---|
| `suprnova` | `suprnova-cli/`（这个 crate） | 脚手架（`new`）、生成器（`make:*`）、开发运行器（`serve`）、迁移（`migrate*`、`db:sync`）、Docker 配置（`docker:*`）、SSR 工作进程（`ssr:*`）、密钥铸造（`key:generate`）、类型生成（`generate-types`） |
| `console` | 您项目里的 `src/bin/console.rs` | 需要链接您应用类型的运行时命令 - 内置的 `db:seed` 和 `model:prune`，加上您定义的每一个 `#[command]` / `#[derive(Command)]` |

工作守护进程（`schedule:run`、`schedule:work`、`schedule:list`、`workflow:work`、`queue:work`）活在第三个表面上：您*应用*自己的二进制文件自己的 clap 解析器，也就是那个提供 HTTP 服务的同一个二进制文件。全局的 `suprnova` 会为它们 shell 进 `cargo run --quiet -- <name>`，这样您就能从已经开着的那个 CLI 里启动它们。完整的三方拆分请参见[控制台](console.md)。

### 为什么 Suprnova 有所不同

Laravel 用一个逐项目的单一脚本解决了这个问题 - `php artisan` - 因为 PHP 会在运行时把框架代码和用户代码一起加载进来。Rust 是在编译期链接二进制文件的，所以一个全局的 `suprnova` 二进制文件没法静态地看到您的填充器、工厂，或者 `#[command]` 处理程序。这个务实的拆分是：

- 只涉及文件的工作（脚手架、生成器、运维操作）活在全局的 `suprnova` 二进制文件上
- 需要用到您编译期类型的运行时工作，活在逐项目的 `console` 二进制文件上
- 守护进程活在您的 app/server 二进制文件上，这样它们就能和 `serve` 共享同一条启动路径

您得到的是 `php artisan` 那种人体工学（`cargo run --bin console -- db:seed`，或者直接 `console <name>`），却不必吞下那个静态链接的谎言。

## 命令一览

和 `suprnova --help` 打印的是同一份列表，分组方式也相同。

### 创建

| 命令 | 描述 |
|---|---|
| `suprnova new [name]` | 脚手架出一个新项目。参见 [`suprnova new`](cli-new.md)。 |
| `suprnova serve` | 把后端 + Vite 一起引导起来，带热重载。参见 [`suprnova serve`](cli-serve.md)。 |
| `suprnova dev:tls` | 信任 portless 的 CA，注册一个 `https://<name>.localhost` 的开发 URL。参见 [HTTPS 开发 URL](dev-tls.md)。 |
| `suprnova web:run` | 直接运行这个应用二进制文件（没有 Vite，没有重建循环）。一次生产形态的本地运行。 |

### 生成

| 命令 | 描述 |
|---|---|
| `suprnova make:controller <name>` | 在 `src/controllers/` 里脚手架出一个控制器。 |
| `suprnova make:action <name>` | 在 `src/actions/` 里脚手架出一个可调用的操作。 |
| `suprnova make:middleware <name>` | 在 `src/middleware/` 里脚手架出一个中间件。 |
| `suprnova make:migration <name>` | 在 `src/migrations/` 里脚手架出一条 SeaORM 迁移。 |
| `suprnova make:inertia <name>` | 在 `frontend/src/pages/` 里脚手架出一个 Inertia 页面。改传 `--data`，则会在 `src/props/` 里生成一个 `#[derive(Data, Validate)]` 的 props 结构体。 |
| `suprnova make:error <name>` | 在 `src/errors/` 里脚手架出一个领域错误。 |
| `suprnova make:task <name>` | 在 `src/tasks/` 里脚手架出一个计划任务。 |
| `suprnova make:command <name>` | 在 `src/commands/` 里脚手架出一个 `#[derive(Command)]` 的 console 命令。 |
| `suprnova generate-types` | 为每一个 `#[derive(InertiaProps)]` 结构体生成 TypeScript 类型。用 `-o <path>` 覆盖输出路径，用 `-w` 监视并再生成。 |

完整的脚手架细节，以及每个生成文件的样子，请参见[生成器](cli-generators.md)。

### 数据库

| 命令 | 描述 |
|---|---|
| `suprnova migrate` | 运行所有待处理的迁移。 |
| `suprnova migrate:status` | 显示哪些迁移已经应用、哪些还待处理。 |
| `suprnova migrate:rollback [--step N]` | 回滚最后 N 条迁移（默认 1）。 |
| `suprnova migrate:fresh [--force]` | 删除每一张表，重新运行所有迁移。**破坏性操作。** 在生产环境中，它需要 `--force`，还需要在交互式终端上完成一次类型确认。 |
| `suprnova db:sync [--skip-migrations] [--regenerate-models]` | 运行迁移，并从存活的架构重新生成 SeaORM 实体。`--regenerate-models` 会覆盖 `src/models/` 里自定义的模型文件。 |

`db:seed` **不**在这里 - 它活在逐项目的 `console` 二进制文件上，因为这个填充器注册表是被编译进您的 crate 里的。通过 `cargo run --bin console -- db:seed` 或者 `./target/debug/console db:seed` 来运行它。注册模式请参见[控制台](console.md)。

完整的迁移工作流请参见[迁移一章](cli-migrations.md)。

### 调度

| 命令 | 描述 |
|---|---|
| `suprnova schedule:run` | 把每一个到期的任务都运行一次。这是对 cron 友好的形式。 |
| `suprnova schedule:work` | 前台守护进程，每分钟检查一次，运行到期的任务。 |
| `suprnova schedule:list` | 打印每一个已注册的任务及其 cron 表达式。 |

这里的每一条，都会针对您的 app/server 二进制文件 shell 进 `cargo run --quiet -- <name>` - 也就是那个提供 HTTP 服务的同一个二进制文件 - 所以已注册的任务和已引导的服务都是可见的。参见[调度命令](cli-scheduling.md)，以及[任务调度](scheduling.md)一章。

### 工作流

| 命令 | 描述 |
|---|---|
| `suprnova workflow:work` | 启动这个工作流工作进程守护进程。从注册表里取出工作流步骤，用和 HTTP 处理程序一样的 Panic 边界来运行它们。 |
| `suprnova workflow:install` | 把 workflow + workflow_steps 这两条迁移放进 `src/migrations/`。全新的脚手架里已经带着它们了。 |

参见[工作流](workflows.md)。

### SSR

| 命令 | 描述 |
|---|---|
| `suprnova ssr:start [--runtime node\|bun\|deno] [--bundle <path>]` | 在前台启动这个 Inertia SSR 工作进程。回退到 `SUPRNOVA_SSR_RUNTIME` 环境变量，再回退到 `node`；bundle 回退到 `SUPRNOVA_SSR_BUNDLE`，再回退到 `frontend/bootstrap/ssr/ssr.js`。 |
| `suprnova ssr:check [--url <url>] [--timeout-ms N]` | 验证 SSR 工作进程的 `GET /health` 路由是否返回 2xx。回退到 `SUPRNOVA_SSR_URL`，再回退到 `http://127.0.0.1:13714`。超时默认 2000 毫秒。 |

生产环境的设置请参见[Inertia SSR](frontend.md)。

### 部署

| 命令 | 描述 |
|---|---|
| `suprnova docker:init` | 生成一个多阶段的生产 `Dockerfile` + `.dockerignore`。 |
| `suprnova docker:compose [--with-mailpit] [--with-minio]` | 为本地开发生成一份 `docker-compose.yml`。Postgres + Redis 始终包含在内；Mailpit 和 MinIO 是可选启用的。 |

参见[Docker](cli-docker.md)，以及[部署概览](deployment.md)一章。

### 安全

| 命令 | 描述 |
|---|---|
| `suprnova key:generate [--show]` | 铸造一把 32 字节的 AES-256 密钥，base64 URL 安全、无填充（和 `EncryptionKey::to_base64` 产出的是同一种传输格式）。`--show` 只打印这把密钥，方便 `APP_KEY=$(suprnova key:generate --show)`。 |

`APP_KEY` 保护的是什么、`APP_KEY_PREVIOUS` 又是怎么做轮换的，请参见[加密](encryption.md)。

## 快速上手

从“什么都没装”到“应用跑起来”，最常见的路径是：

```bash
# 1. 安装 CLI
cargo install --git https://github.com/eas4ai/suprnova.git --tag v1.3.7 suprnova-cli

# 2. 脚手架出一个项目（交互式 - 默认选 Svelte）
suprnova new my-app

# 3. 把它启动起来
cd my-app
suprnova migrate
npm install
suprnova serve
```

非交互式的脚手架（CI、脚本化的搭建）：

```bash
suprnova new my-app \
  --frontend svelte \
  --no-interaction \
  --no-git
```

仅 API 的脚手架（没有 Inertia，没有 SPA）：

```bash
suprnova new my-api --api
```

在一个已有项目里生成代码：

```bash
suprnova make:controller Posts
suprnova make:migration create_posts_table
suprnova make:command reports:daily   # 注册在逐项目的 console 二进制文件下
suprnova migrate
```

## 获取帮助

`--help`（或者 `-h`）在任何子命令上都能用。顶层帮助是手工排版的（`ui::print_help`），会按小节给命令分组；逐子命令的帮助来自 clap，会列出每一个标志及其默认值：

```bash
suprnova --help
suprnova new --help
suprnova serve --help
suprnova make:inertia --help
```

对于逐项目的 `console` 二进制文件：

```bash
cargo run --bin console -- --help
cargo run --bin console -- db:seed --help
cargo run --bin console -- <your-command> --help
```

`--version` 会把版本单独打印成一行，这正是您在报告缺陷、或者确认一次安装有没有生效时想要的：

```bash
suprnova --version
# suprnova 1.3.7
```

`-v` 和 `-V` 都会被接受。clap 生成的标志只提供 `-V`；这一个是手工声明的，所以小写写法 - 也就是大多数人第一个会试的那个 - 同样管用。版本号也会出现在 `--help` 的横幅里，在这个标志存在之前，它就住在那儿。

## 下一步

- [`suprnova new`](cli-new.md) - 脚手架工具接受的每一个标志，以及它产出的目录布局
- [`suprnova serve`](cli-serve.md) - 这个开发运行器：后端 + Vite + 类型生成
- [生成器](cli-generators.md) - 完整的 `make:*` 家族及其输出模板
- [迁移 CLI](cli-migrations.md) - `migrate`、`migrate:fresh`、`db:sync`，以及这套 SeaORM 工作流
- [控制台](console.md) - 逐项目的 `console` 二进制文件、`#[command]`、`#[derive(Command)]`，以及三个二进制文件之间的不对称性
