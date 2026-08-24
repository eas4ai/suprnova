# 安装

本章将帮助您从“机器上没有 Suprnova”到一个运行中的脚手架项目。如果您已经有了，可以跳转到[快速上手](quickstart.md)。

## 要求

- 当前 `main` 需要 **Rust 1.94.0+**（workspace 使用 2024 edition）。带标签的 v1.3.0 版本同样以 Rust 1.94.0 为最低版本。通过 [rustup](https://rustup.rs/) 安装：
  ```bash
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  ```
- **Node.js 20+** 和 **npm**（或 pnpm/yarn/bun）用于前端工具链。Suprnova 使用 Vite 8，您的起步模板配备 TypeScript +
  Tailwind v4。通过 [nodejs.org](https://nodejs.org/) 或您的包管理器安装。
- **数据库客户端库**，与您要使用的驱动程序匹配：
  - SQLite - 无需额外配置；sqlite 已包含
  - PostgreSQL - 大多数系统上需要 `libpq`（通常预装）
  - MySQL 或 MariaDB - 大多数系统上需要 `libmariadb` / `libmysqlclient`

您现在不必选择数据库。默认脚手架选择 SQLite，因此新应用无需设置即可运行。


当前 `main` 使用 SeaORM 2.0、SeaQuery 1.0 和 SQLx 0.9。直接调用 SeaORM 的应用程序必须导入 `ExprTrait` 以使用 SeaQuery 表达式方法，并对预构建的 `Statement` 值使用显式 `*_raw` 连接方法。此次依赖项升级不需要迁移应用程序数据。

## 安装 CLI

Suprnova 以一个 Cargo 项目的形式分发，CLI 安装程序会从 git 拉取框架（不是从 crates.io - 原因见下面的[发布前说明](#pre-launch-note)）：

```bash
cargo install --git https://github.com/eas4ai/suprnova.git --tag v1.3.0 suprnova-cli
```

这会编译出 `suprnova` 二进制文件，并把它放进 `~/.cargo/bin`。确认它生效了：

```bash
suprnova --version
```

您应该看到 `suprnova 0.x.x`。

如果找不到 `suprnova`，说明您的 `~/.cargo/bin` 不在 `PATH` 上。请把这一行加进您的 shell 配置：

```bash
export PATH="$HOME/.cargo/bin:$PATH"
```

## 创建项目

`suprnova new` 脚手架完整的项目 - 后端 + 选择的前端 + Vite 配置 + 认证迁移 + 示例路由。默认是交互式的：

```bash
suprnova new my-app
```

向导按顺序询问：

1. **项目名称** - 当您作为参数传递时跳过（`my-app`）
2. **描述** - 用于 `Cargo.toml`
3. **作者** - 用于 `Cargo.toml`；默认为您的 git `user.name`
4. **前端框架** - `svelte`（默认）、`react` 或 `vue` 之一

如果您想跳过提示（CI、脚本化设置），传递
`--no-interaction` 并显式选择前端：

```bash
suprnova new my-app --frontend svelte --no-interaction
```

`--no-interaction` 接受描述（“A web application built with Suprnova”）和作者（空）的默认值。要设置这些，请在脚手架后编辑生成的 `Cargo.toml`。

三个前端选择各自都有自己的 Runes-on/Svelte-5、React-19 或 Vue-3.5 起步模板。所有三个都使用 Inertia v3 + Vite 8 + Tailwind v4，并预配置带有会话认证的 Login/Register/Dashboard 流程。

Suprnova 还提供一个更简洁的 **API 起步** 用于无 SPA 的服务后端：

```bash
suprnova new my-api --api
```

API 起步没有前端或 Inertia 层。它会在应用数据库上初始化 Magnetar，安装
`BearerTokenMiddleware`，并针对 `app_users` 脚手架密码注册和登录。

## 首次运行

```bash
cd my-app

# Run migrations (users, sessions, etc.)
suprnova migrate

# Install frontend dependencies
npm install              # in the project root

# Start the backend + Vite together
suprnova serve
```

`suprnova serve` 在 `http://127.0.0.1:8765` 运行后端，在 `http://127.0.0.1:5765` 运行 Vite。点击后端 URL - Vite 被代理，所以您不需要直接访问它。

您应该看到欢迎页面。然后访问 `/register` 创建账户，`/login` 登录。

## 生成了什么

```
my-app/
├── Cargo.toml          # crate manifest, two [[bin]] targets
├── .env                # local config (DB URL, app key, ports)
├── .env.example        # template for ops/CI
├── .gitignore
├── cmd/
│   └── main.rs         # the binary entry; calls Application::new().run()
├── src/
│   ├── lib.rs          # module wiring
│   ├── bootstrap.rs    # service registration (the Suprnova analogue of providers)
│   ├── routes.rs       # the routes! macro tree
│   ├── bin/
│   │   └── console.rs  # `cargo run --bin console <subcommand>`
│   ├── actions/        # single-method invokable controllers
│   ├── commands/       # `#[command]`-annotated handlers
│   ├── config/         # typed config sections (database, mail)
│   ├── controllers/    # home, auth, dashboard
│   ├── middleware/     # logging, authenticate
│   ├── migrations/     # SeaORM migrators (users, sessions, etc.)
│   └── models/         # `#[suprnova::model]` structs (user)
├── frontend/
│   ├── package.json
│   ├── vite.config.ts
│   ├── tsconfig.json
│   ├── index.html
│   └── src/
│       ├── main.{tsx,ts}
│       ├── app.css
│       ├── pages/
│       │   ├── Home, Dashboard
│       │   └── auth/{Login,Register}
│       └── types/
│           └── inertia-props.ts
└── public/
    └── assets/         # Vite production build output
```

完整的目录详览在[目录结构](structure.md)中。

## 更新 CLI

CLI 就住在您的 `~/.cargo/bin` 里。要更新到最新版：

```bash
cargo install --force --git https://github.com/eas4ai/suprnova.git --tag v1.3.0 suprnova-cli
```

`--force` 会让 Cargo 覆盖已有的那个二进制文件。

## 更新您应用的框架版本

一个脚手架出来的应用，是通过 `Cargo.toml` 里的一条 git 依赖来依赖 `suprnova` 框架 crate 的：

```toml
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.3.0" }
```

要拉取最新的框架变更：

```bash
cargo update -p suprnova
```

这条 git 依赖跟踪的是被点名的那个发布标签。请在 `Cargo.toml` 里更新这个标签，然后运行 `cargo update -p suprnova`；您的 `Cargo.lock` 会记下它解析到的那个精确提交，所以两次更新之间构建保持可复现 - 不需要在 `Cargo.toml` 里手工钉一个 `rev`。

## 分发模型

Suprnova 是通过 git 分发的，不是 crates.io - 框架和 CLI 都从 GitHub 安装。每个版本都会作为一个带标签的 GitHub Release（比如 `v1.2.4`）发布出来，而您的应用依赖的正是这个标签：一个脚手架出来的 `Cargo.toml` 会钉住 `tag = "v1.3.0"`，而 `Cargo.lock` 会记下这个标签解析到的那个精确提交，所以在您主动选择挪动之前，构建都是可复现的。更新是刻意为之的，绝不会顺带发生 - 递增这个标签，然后运行 `cargo update -p suprnova`；关于更新您应用框架版本的那一节会带您走一遍。

## 编辑器设置

一些 VS Code 扩展使体验更顺利：

- **rust-analyzer** - Rust 语言服务器
- **Svelte for VS Code**（如果您选择了 React/Vue 则选择那个）
- **Tailwind CSS IntelliSense**
- **Even Better TOML**

`rust-analyzer` 将在首次打开项目时索引该项目；第一次预计 1-2 分钟，然后增量。

## 下一步

- [快速上手](quickstart.md) - 5 分钟内构建一个小应用
- [目录结构](structure.md) - 脚手架生成的每个文件中有什么
- [配置](configuration.md) - `.env` 和类型化配置方案
- [路由](routing.md) - 添加您的第一个路由
