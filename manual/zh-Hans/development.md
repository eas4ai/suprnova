# 开发

日常 Suprnova 工作流是一条命令：`suprnova serve`。它在单个进程中运行 Rust 后端、Vite 前端和一个 TypeScript 类型再生器，每个都监视相关的文件。本章介绍开发服务器、热重载部分如何配合工作，以及您日常需要使用的命令。第一次设置请参阅 [安装](installation.md)；了解目录结构请参阅 [目录结构](structure.md)。

## 开发服务器

从搭建的项目的根目录：

```bash
suprnova serve
```

CLI 会打印两个 URL，然后是来自每个子进程的连续前缀输出流：

```
Backend  http://127.0.0.1:8765
Frontend http://127.0.0.1:5765

[backend]  Compiling links v0.1.0
[backend]  Finished `dev` profile [unoptimized + debuginfo] target(s) in 4.21s
[backend]  Running `target/debug/links`
[frontend] VITE v6.0.1  ready in 312 ms
[frontend]   ➜  Local:   http://localhost:5765/
[types]    Watching for Rust file changes to regenerate types
```

访问后端 URL（`127.0.0.1:8765`）。Vite 通过 Inertia 的开发集成提供您的 JS/CSS - 不要直接访问 `:5765`。按一次 `Ctrl+C`，CLI 会干净地关闭两个子进程。

### 标志

| 标志 | 默认值 | 功能 |
|---|---|---|
| `-p`, `--port <N>` | `8765` | 后端端口 |
| `--frontend-port <N>` | `5765` | Vite 端口 |
| `--backend-only` | off | 跳过 Vite 子进程（仅做 API 工作时）|
| `--frontend-only` | off | 跳过后端子进程（针对在别处运行的后端做组件开发时）|
| `--skip-types` | off | 跳过 TypeScript 类型生成器及其监视器 |

相同的端口可以在 `.env` 中通过 `SERVER_PORT` 和 `VITE_PORT` 设置。命令行上的标志优先于 `.env`。

### 预检查

在生成任何东西之前，`suprnova serve`：

1. **检查您是否在一个项目中。** 如果没有 `Cargo.toml`（或运行前端时没有 `frontend/`）则中止并返回清晰的错误。
2. **生成一次 TypeScript 类型。** 扫描 `src/` 中的 `#[derive(InertiaProps)]` 并写入 `frontend/src/types/inertia-props.ts`。通过 `--skip-types` 或 `--frontend-only` 跳过。
3. **如果缺少则安装 `cargo-watch`。** 在新机器上首次运行会为您运行 `cargo install cargo-watch`，然后继续。
4. **如果 `frontend/node_modules` 缺失则运行 `npm install`。** 在新克隆上不需要手动安装步骤。

## 热重载

三个监视器在 `suprnova serve` 内并发运行：

- **`cargo watch -x 'run --bin <pkg>'`** 驱动后端。项目下的任何 `.rs` 变化会触发重新编译和进程内重启。编译错误会打印到 `[backend]` 流，前一个二进制文件会一直运行，直到下一次成功构建。
- **Vite** 驱动前端。组件、样式和资产编辑会热模块替换到打开的浏览器标签中，无需完整重载。
- **基于 `notify` 的类型监视器** 在 `.rs` 文件变化时重新运行 InertiaProps 扫描器。它在 500ms 处防抖，因此一连串的保存会再生成一次 `inertia-props.ts`。输出显示在 `[types]` 前缀下。

第三个是您不必考虑的部分：在 `#[derive(InertiaProps)]` 结构体上重命名一个字段，匹配的 TypeScript 接口在下一次保存时会跟随。Svelte/React/Vue 页面会立即获取新类型。在正常开发期间不需要 `suprnova generate-types` 调用。

### 为什么 Suprnova 有所不同

大多数 Rust Web 堆栈让热重载成为您的问题 - 选择您自己的文件监视器、编写您自己的重启包装器、在单独的终端中运行 Vite。大多数 Laravel 堆栈让 TypeScript 类型成为您的问题 - 在两个地方（PHP 和 TS）声明它们并保持同步。`suprnova serve` 运行两个监视器，加上保持您的前端类型诚实的类型生成器，作为一个受监控的进程。Tokio 运行时使“一次做很多事”便宜到足以让开发循环自由花费。

## 日常命令

您每小时会运行的少数几个：

```bash
suprnova serve                    # 启动开发环境（后端 + Vite + 类型监视器）
suprnova make:controller orders   # 生成控制器脚手架
suprnova make:migration add_idx   # 生成迁移脚手架
suprnova db:sync                  # 运行迁移，重新生成 SeaORM 实体
suprnova migrate:status           # 查看哪些已经应用
suprnova migrate:fresh            # 删除表 + 从头重新运行
suprnova key:generate --show      # 轮换 APP_KEY
cargo run --bin console <cmd>     # 任何带 `#[command]` 标注的控制台处理程序
cargo test                        # 运行测试套件
```

`db:sync` 是一个开发快捷方式，用于“在一步中迁移 + 实体再生”。在生产中您使用普通 `suprnova migrate`，因为您不希望再生成在发布机器上发生。完整的生成器表面位于 [代码生成器](cli-generators.md)，迁移动词位于 [迁移](migrations.md)。

## 调试

### 日志

Suprnova 端到端使用 `tracing`。使用 `LOG_LEVEL` 过滤打印的内容（与 `tracing-subscriber` 的 `EnvFilter` 相同的语法）：

```bash
# 详细的框架输出
LOG_LEVEL=debug suprnova serve

# 让 hyper 保持安静，但您自己的 crate 输出详细
LOG_LEVEL=info,my_app=debug,hyper=warn suprnova serve
```

输出格式由 `LOG_FORMAT` 控制（`pretty` 为人类可读，`json` 为机器可解析）。开发默认值是 `pretty`。完整的日志表面请参阅 [可观测性](observability.md)。

### SQL 查询

使用一个环境变量启用每查询日志：

```env
DB_LOGGING=true
```

这通过 `tracing` 在 `info` 级别路由每个 SeaORM 查询，这样您可以看到完全执行的是什么。除非您追踪特定的慢查询，在生产中关闭它 - 日志量很快变得嘈杂。

### 回溯

标准 Rust：

```bash
RUST_BACKTRACE=1 suprnova serve
```

处理程序中的恐慌被捕获并转换为结构化 500 响应；回溯不会将服务器关闭就落地在您的日志中。如何处理该契约的信息请参阅 [错误模型](error-model.md)。

## 循环中的测试

```bash
cargo test                        # 整个工作空间
cargo test -p my_app              # 只测您的应用 crate
cargo test some_test_name         # 按名称过滤
cargo test -- --nocapture         # 显示 println!/tracing 的输出
```

测试执行是普通 Cargo。框架端的辅助工具（`#[suprnova_test]`、`TestDatabase`、`expect!`、Mail/Queue/Storage/等 的伪造）在 [测试](testing.md) 和 [数据库测试](database-testing.md) 中有文档。它们在您已经知道的相同 `cargo test` 下运行。

## 使用 SSR 工作进程

如果您的应用使用 Inertia 服务器端渲染，您会希望 SSR 工作进程在开发期间与 `suprnova serve` 一起：

```bash
# 终端 1
suprnova serve

# 终端 2
suprnova ssr:start
```

`ssr:start` 在 Node、Bun 或 Deno（`--runtime`）下运行捆绑的 SSR 工作进程。`ssr:check` 验证一个运行中的工作进程是否可到达。两者都在前端章节下有文档 - 参阅 [前端](frontend.md)。

## 当事情看起来不对时

一个针对最常见开发循环打嗝的简短分类列表：

- **端口已在使用。** 另一个 `suprnova serve` 仍在运行，或先前的后端卡住。`lsof -i :8765` 来查找它，或只需传递 `--port 8001`。
- **`cargo-watch` 不断重新编译。** 某些编辑器在保存时重写文件（格式化程序、有自动修复的 linters）。禁用项目的保存时格式，或使用 `CARGO_WATCH_IGNORE` 模式限定您的监视器。
- **TypeScript 类型不更新。** 要么 `--skip-types` 被传递了，要么监视器被 `.rs` 解析错误绊倒了。查看 `[types]` 行 - 它打印警告并继续而不是使整个 serve 失败。
- **Vite 出错但后端没问题。** 在 `frontend/` 中运行一次 `npm install`（CLI 首次 serve 时会这样做，但如果您清除了 `node_modules` 它在新启动之前不会再做，直到该目录再次缺失）。

其他任何东西，[错误](errors.md) 章节涵盖了更深层的分类模式。

## 下一步

- [安装](installation.md) - CLI 和项目的首次设置
- [快速上手](quickstart.md) - 从开始到结束构建一个小应用
- [目录结构](structure.md) - 每个目录包含什么
- [代码生成器](cli-generators.md) - 每个 `make:*` 命令
- [测试](testing.md) - `#[suprnova_test]`、伪造和测试数据库
