# suprnova new

`suprnova new` 会脚手架出一个 Suprnova 项目 - 一个全新的 Cargo crate，带有控制器、路由、迁移、一个 Inertia SPA，以及一套已经接好的可用认证流程。每个应用只需要运行一次，此后您的日常工作就都在 `suprnova serve` 里进行。

## 用法

```bash
suprnova new [name] [options]
```

如果省略了 `name`，交互式向导会提示您输入。这个名字会成为项目目录、Cargo 包名（经过 snake-case 转换之后），以及 `.env` 里默认的 `APP_NAME`。名字必须是 ASCII 字母/数字/`-`/`_`，以字母开头，不含路径分隔符或 `..`，并且不超过 64 个字符。

## 选项

| 选项 | 描述 |
|---|---|
| `--frontend <svelte\|react\|vue>` | 非交互式地选择 SPA 框架。与 `--api` 冲突。 |
| `--api` | 脚手架出一个只有 JSON:API 的项目（没有 Inertia，没有 SPA，用 token 认证取代会话）。 |
| `--no-interaction` | 跳过所有提示，使用默认值（名字 `my-suprnova-app`，前端 `svelte`，作者/描述为空）。 |
| `--no-git` | 跳过在新项目里执行 `git init`。 |
| `--with-portless` | 生成一份 `portless.json`，这样 [`suprnova dev:tls`](dev-tls.md) 就能把应用发布在 `https://<name>.localhost` 上。是可选启用的；不会改变其他任何东西。 |

## 交互模式

```bash
suprnova new my-app
```

向导会按下面这个顺序问四个问题：

1. **项目名称** - 默认为目录参数（`my-app`）
2. **描述** - 用作 Cargo 包描述
3. **作者** - 用作 Cargo 包作者；如果设置了，默认为您的 `git config user.name <name@email>`
4. **前端框架** - `Svelte (recommended)`、`React` 或 `Vue`

确认之后，脚手架工具会写入项目，运行 `git init`（除非传了 `--no-git`），并打印后续步骤：

```
Backend  http://localhost:8765
Frontend http://localhost:5765
```

## 非交互模式

对于 CI、dotfiles，或者脚本化的设置，传入 `--no-interaction`，再加上您想覆盖的那些标志：

```bash
suprnova new my-app --frontend svelte --no-interaction
```

`--no-interaction` 下的默认值：

- 前端：`svelte`
- 描述：`"A web application built with Suprnova"`
- 作者：空
- Git：已初始化

没有 `--description` 或 `--author` 标志；这些值只能通过交互式提示来设置，否则就接受它们的默认值。

## 纯 API 项目

对于没有 SPA 的服务后端，使用 `--api`：

```bash
suprnova new my-api --api
```

API 起步明显更小：没有 `frontend/` 目录，没有 Inertia，没有认证视图，是单 crate 的 `src/main.rs` 布局（而不是 SPA 起步那种 `cmd/main.rs` 工作空间），基于 token 的认证，还带一个示例性的 `users` 控制器，加上一个 `UserResource` JSON 序列化器。API 起步在它的 `.env` 里绑定到 8765 端口。

`--api` 与 `--frontend` 互斥；两者都传会报错。在 `--api` 之下，只会提示项目名称 - 描述/作者/前端的提示都会被跳过。

## 脚手架出的内容

完整的目录导览见 [目录结构](structure.md)；简版是：

- `cmd/main.rs` - 二进制入口；调用 `Application::new()…run()`
- `src/` - 控制器、操作、命令、配置、中间件、模型、迁移，加上 `bootstrap.rs` 和 `routes.rs`
- `src/bin/console.rs` - 逐项目的 `php artisan` 对应物
- `frontend/` - Vite 8 + Tailwind v4 + 您选择的框架，Home / Dashboard / Login / Register 页面已经通过 Inertia 接好
- `src/migrations/` - `users`、`sessions` 和 `remember_tokens` 表已经就位
- `.env` - 默认是 SQLite 数据库，带一个刚生成好的 `APP_KEY`，这样应用无需运维干预就能启动
- `.gitignore`、`Cargo.toml`

### 为什么 Suprnova 有所不同

Laravel 随 Blade 一起发布，之后再通过 Breeze/Jetstream 把一个前端拉进来。Suprnova 走的是反过来的路：`suprnova new` 总是会脚手架出一个真正的 SPA（基于 Inertia 的 Svelte/React/Vue），或者一个真正的 JSON:API 项目。这里没有一个以模板引擎为先的起步方案 - 如果您想要服务端渲染的 HTML，Tera 是可用的，但它不是默认形态，也没有哪条脚手架路径会把视图放在您应用的最前面。

默认前端是 **Svelte 5**（runes-on），而不是 React。我们选它，是因为它在运行时是三者中最轻量的，也最贴近这个框架「编译期胜过运行时花招」的哲学。React 和 Vue 同样都是一等公民 - 选您团队熟悉的那个就好。

## 分发

CLI 本身通过 git 分发，而不是 crates.io（预发布阶段）：

```bash
cargo install --git https://github.com/entrepeneur4lyf/suprnova.git --tag v1.2.0 suprnova-cli
```

对同一条命令追加 `--force`，会更新一个已有的安装。脚手架出来的项目，依赖框架 crate 的方式也是一样的 - 在它们的 `Cargo.toml` 里是一个 git 依赖，固定在当前的发布标签上。完整的工具链前提条件请参见[安装](installation.md)。

## 下一步

- [安装](installation.md) - Rust/Node/DB 前提条件与工具链设置
- [目录结构](structure.md) - 每个脚手架文件的作用
- [快速上手](quickstart.md) - `suprnova new` 之后的头 5 分钟
- [suprnova serve](cli-serve.md) - 您接下来会用到的开发运行器
- [控制台](console.md) - `cargo run --bin console` 和 `#[command]` 系统
