# 安装

本章将帮助您从“机器上没有 Suprnova”到一个运行中的脚手架项目。如果您已经有了，可以跳转到 [快速上手](quickstart.md)。

## 要求

- **Rust 1.91.1+**（工作空间使用 2024 版本）。通过 [rustup](https://rustup.rs/) 安装：
  ```bash
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  ```
- **Node.js 20+** 和 **npm**（或 pnpm/yarn/bun）用于前端工具链。Suprnova 使用 Vite 8，您的起步模板配备 TypeScript + Tailwind v4。通过 [nodejs.org](https://nodejs.org/) 或您的包管理器安装。
- **数据库客户端库**，与您要使用的驱动程序匹配：
  - SQLite - 无需额外配置；sqlite 已包含
  - PostgreSQL - 大多数系统上需要 `libpq`（通常预装）
  - MySQL 或 MariaDB - 大多数系统上需要 `libmariadb` / `libmysqlclient`

您现在不必选择数据库。默认脚手架选择 SQLite，因此新应用无需设置即可运行。

## 安装 CLI

Suprnova 作为 Cargo 项目分发，CLI 安装程序从 git 拉取框架（不是从 crates.io - 参见下面的 [预启动说明](#pre-launch-note)）：

```bash
cargo install --git https://github.com/eas4ai/suprnova.git --tag v1.2.4 suprnova-cli
```

这将编译 `suprnova` 二进制文件并将其放入 `~/.cargo/bin`。确认它有效：

```bash
suprnova --version
```

您应该看到 `suprnova 0.x.x`。

如果找不到 `suprnova`，您的 `~/.cargo/bin` 不在 `PATH` 中。添加这一行到您的 shell 配置：

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

如果您想跳过提示（CI、脚本化设置），传递 `--no-interaction` 并显式选择前端：

```bash
suprnova new my-app --frontend svelte --no-interaction
```

`--no-interaction` 接受描述（“A web application built with Suprnova”）和作者（空）的默认值。要设置这些，请在脚手架后编辑生成的 `Cargo.toml`。

三个前端选择各自都有自己的 Runes-on/Svelte-5、React-19 或 Vue-3.5 起步模板。所有三个都使用 Inertia v3 + Vite 8 + Tailwind v4，并预配置带有会话认证的 Login/Register/Dashboard 流程。

Suprnova 还提供一个更简洁的 **API 起步** 用于无 SPA 的服务后端：

```bash
suprnova new my-api --api
```

API 起步具有相同的后端栈，但没有前端、没有 Inertia，并使用基于令牌的认证而不是会话 cookie。

## 首次运行

```bash
cd my-app

# 运行迁移（users, sessions, etc.）
suprnova migrate

# 安装前端依赖
npm install              # 在项目根目录中

# 一起启动后端和 Vite
suprnova serve
```

`suprnova serve` 在 `http://127.0.0.1:8765` 运行后端，在 `http://127.0.0.1:5765` 运行 Vite。点击后端 URL - Vite 被代理，所以您不需要直接访问它。

您应该看到欢迎页面。然后访问 `/register` 创建账户，`/login` 登录。

## 生成的目录结构

```
my-app/
├── Cargo.toml          # crate 清单，两个 [[bin]] 目标
├── .env                # 本地配置（数据库 URL、应用密钥、端口）
├── .env.example        # 用于运维/CI 的模板
├── .gitignore
├── cmd/
│   └── main.rs         # 二进制入口点；调用 Application::new().run()
├── src/
│   ├── lib.rs          # 模块连接
│   ├── bootstrap.rs    # 服务注册（Suprnova 的 providers 对等物）
│   ├── routes.rs       # routes! 宏树
│   ├── bin/
│   │   └── console.rs  # `cargo run --bin console <subcommand>`
│   ├── actions/        # 单方法可调用控制器
│   ├── commands/       # `#[command]` 注解处理程序
│   ├── config/         # 类型化配置部分（数据库、邮件）
│   ├── controllers/    # 主页、认证、仪表板
│   ├── middleware/     # 日志、认证
│   ├── migrations/     # SeaORM 迁移工具（用户、会话等）
│   └── models/         # `#[suprnova::model]` 结构（user）
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
    └── assets/         # Vite 生产构建输出
```

完整的目录详览在 [目录结构](structure.md) 中。

## 更新 CLI

CLI 位于您的 `~/.cargo/bin` 中。要更新到最新版本：

```bash
cargo install --force --git https://github.com/eas4ai/suprnova.git --tag v1.2.4 suprnova-cli
```

`--force` 使 Cargo 覆盖现有的二进制文件。

## 更新应用的框架版本

脚手架应用通过 `Cargo.toml` 中的 git 依赖依赖于 `suprnova` 框架 crate：

```toml
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.2.4" }
```

要拉取最新框架变更：

```bash
cargo update -p suprnova
```

git 依赖跟踪命名的发布标签。更新 `Cargo.toml` 中的标签，然后运行 `cargo update -p suprnova`；您的 `Cargo.lock` 记录了它解析的精确提交，因此构建在更新之间保持可重现 - 无需在 `Cargo.toml` 中手动固定 `rev`。

## 分发模型

Suprnova 通过 git 分发，不是 crates.io - 框架和 CLI 都从 GitHub 安装。每个版本都作为带标签的 GitHub 版本发布（例如 `v0.7.2`）用于变更日志，但您不依赖该标签：git 依赖跟踪默认分支，`Cargo.lock` 固定应用解析的精确提交，因此构建在 `cargo update` 运行之间是可重现的 - 无需手动固定 `tag` 或 `rev`。

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
