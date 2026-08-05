# suprnova serve

`suprnova serve` 会把您的后端和 Vite 开发服务器一起运行起来，两边都带热重载，外加每次您改动一个 `#[derive(InertiaProps)]` 结构体时的自动 TypeScript 类型再生成。这是您在构建时会一直开着的那一条命令。

```bash
suprnova serve
```

两个进程都会把自己的 stdout 流进同一个终端，带着彩色的 `[backend]` 和 `[frontend]` 前缀，这样您就能分清是谁在说话。`Ctrl+C` 会把它们俩都干净地关掉。

## 用法

```bash
suprnova serve [OPTIONS]
```

| 选项 | 默认值 | 描述 |
|---|---|---|
| `-p, --port <PORT>` | `8765`（CLI）/ `$SERVER_PORT`（环境变量） | 后端 HTTP 端口 |
| `--frontend-port <PORT>` | `5765`（CLI）/ `$VITE_PORT`（环境变量） | Vite 开发服务器端口 |
| `--backend-only` | `false` | 跳过 Vite 开发服务器 |
| `--frontend-only` | `false` | 跳过后端，只运行 Vite |
| `--skip-types` | `false` | 不在 Rust 变更时重新生成 TypeScript 类型 |

CLI 标志优先于环境变量，环境变量又优先于内置的默认值。脚手架生成的 `.env` 自带 `SERVER_PORT=8765` 和 `VITE_PORT=5765`；除非您用 `--port` 覆盖，否则您会看到用的就是这些值。

## 示例

### 默认 - 两个服务器都启动

```bash
suprnova serve
```

输出：

```
Backend  http://127.0.0.1:8765
Frontend http://127.0.0.1:5765
[backend] Compiling my-app v0.1.0 ...
[frontend] VITE v6.3.0  ready in 312 ms
```

在浏览器里访问 `http://127.0.0.1:8765`。后端会提供 Inertia 的 HTML 外壳，并把资产请求代理转发给 Vite，所以您不需要直接访问 Vite 的 URL。

### 自定义端口

```bash
suprnova serve --port 3000 --frontend-port 3001
```

或者把它们设在 `.env` 里，不带标志直接运行：

```env
SERVER_PORT=3000
VITE_PORT=3001
```

### 仅后端

```bash
suprnova serve --backend-only
```

适合只做纯 API 项目的开发，或者您的前端已经在另一个终端里跑着的时候（也可能是另一台机器，或者一个已部署的预览环境）。

### 仅前端

```bash
suprnova serve --frontend-only
```

适合专心做 UI，而不用为每次保存都付出一次 Rust 重新编译的代价，或者后端已经在另一个 shell 里跑着（或者跑在 Docker 里）的时候。

### 跳过类型生成

```bash
suprnova serve --skip-types
```

关掉 TypeScript 再生成监视器。当您是手工维护 `frontend/src/types/inertia-props.ts`，或者您正在做的事离任何 Inertia 代码都很远、想要更安静的输出时，就用这个。

## 它实际做了什么

当您运行 `suprnova serve` 时，CLI 会：

1. 从当前目录加载 `.env`。
2. 解析后端和前端的端口（CLI 标志 → 环境变量 → 默认值）。
3. 验证您确实在一个 Suprnova 项目里 - `Cargo.toml` 必须存在（除非传了 `--frontend-only`），`frontend/` 目录也必须存在（除非传了 `--backend-only`）。
4. 从它在 `src/` 里找到的任何 `#[derive(InertiaProps)]` 结构体重新生成 TypeScript 类型，写入 `frontend/src/types/inertia-props.ts`。
5. 如果 `cargo-watch` 还不在 PATH 上，就通过 `cargo install --locked --version "^8.5" cargo-watch` 安装它（只做一次，带一条「Installing...」提示）。在 `--frontend-only` 下会被跳过。这个版本号之所以被限定，是因为 `serve` 驱动的是 `cargo watch -x`，它的含义在一次大版本跳跃之间并不保证不变；`--locked` 会构建 cargo-watch 发布时的那份依赖树，而不是在安装时重新解析它。一条会作为启动开发服务器的副作用去安装软件的命令，不该同时还替您挑版本。
6. 如果 `node_modules` 还不存在，就在 `frontend/` 里运行 `npm install`。在 `--backend-only` 下会被跳过。
7. 为后端 spawn 一个 `cargo watch -x 'run --bin <package-name>'`。每当一个 `.rs` 文件发生变化，`cargo-watch` 就会重新运行这个二进制文件。
8. 为 Vite 在 `frontend/` 里 spawn 一个 `npm run dev`，这会给您 Svelte/React/Vue 组件和 Tailwind 类的 HMR。
9. 在 `src/` 上启动一个文件监视器，每当一个 `.rs` 文件变化，并且这一连串保存已经安静了 500 毫秒之后，就重新运行这个类型生成器。这个防抖是后沿触发的，所以一连串的变更 - `cargo fmt`、跨多个文件的保存时格式化、一次分支切换 - 会合并成恰好一次再生成，在最后一次写入*之后*运行，而不是在第一个文件上就触发、错过剩下的文件。
10. 把两个子进程的 stdout/stderr 都转发到您的终端，带着 `[backend]` 和 `[frontend]` 前缀。

`Ctrl+C` 会通知这个管理器去设置它的关闭标志、杀掉两个子进程，然后退出。如果任何一个进程自己退出了 - 通常是因为一个 `cargo watch` 没法恢复的、太严重的 Rust 编译错误，或者一次端口冲突 - 这个管理器就会把它当作一次关闭信号，把另一个也拆掉。

### 为什么 Suprnova 有所不同

Laravel 用户通常会用 `php artisan serve` 跑后端，在另一个终端里跑 `npm run dev`，大多数团队会用一个 `Procfile` 加 `foreman`/`overmind` 来掩盖这种两个终端的割裂。Suprnova 把这个多路复用器当作一个一等的 CLI 命令来发布。您得到的是一个终端、一次 `Ctrl+C`、自动的工具链引导（`cargo-watch`、`npm install`），以及一座类型化的 Inertia 桥，它会随时再生成 `frontend/src/types/inertia-props.ts`，这样您的 Svelte/React/Vue 组件永远能看到当前的 prop 形态，不需要手动同步类型。

## 热重载

**后端。** `cargo watch -x 'run --bin <package>'` 就是这个循环。项目里每一次 `.rs` 变化，它都会重新编译并重启服务器。改动一个很重的 crate 之后的冷编译可能要花几秒钟；单个文件内的增量变化通常不到一秒。

**前端。** Vite 的 HMR 会就地注入组件变更，不需要整页重载，还会保留组件状态。Tailwind 类会通过 Tailwind v4 的监视器实时更新。

**TypeScript 类型。** 每当一个 `.rs` 文件变化，类型监视器就会重新运行这个生成器。如果出现了新的 `#[derive(InertiaProps)]` 结构体（或者已有的结构体改变了形态），重新生成的 `frontend/src/types/inertia-props.ts` 就会为导入它们的组件触发 Vite 的 HMR。

## 故障排查

### 端口已被占用

```text
[backend] Error: Address already in use (os error 98)
```

找到并杀掉这个进程，或者换一个端口：

```bash
lsof -i :8765
kill -9 <pid>

# 或者
suprnova serve --port 8081
```

### `cargo-watch` 安装失败

如果 `cargo-watch` 还不在 PATH 上，CLI 会运行 `cargo install cargo-watch`。如果这次安装失败了（没有网络、受限的环境），就手动安装一次：

```bash
cargo install cargo-watch
```

在那之后，`suprnova serve` 就能找到它，不会再尝试安装了。

### 前端依赖卡住

如果 `npm install` 在引导过程中失败了，先修好根源（npm 仓库能不能连通、磁盘空间、lockfile 是否完好），然后手动运行一次：

```bash
cd frontend && npm install
```

然后重新运行 `suprnova serve`。CLI 只会在 `node_modules` 缺失时自动运行 `npm install`，所以一次成功的手动安装能让它跳过这一步。

### 类型重生成没有捕捉到变更

这个监视器每 2 秒轮询一次（用带轮询间隔的 `notify` - 这么选是为了跨平台的可靠性，而不是被 inotify 的怪癖绊住），并把再生成防抖到每 500 毫秒一次。如果一次变更没有反映出来：

- 确认这个文件在 `src/` 之下（这个监视器不会递归进 `crates/`、`cmd/` 或 `migrations/`）。
- 确认这个结构体确实带着 `#[derive(InertiaProps)]`。
- 重启 `suprnova serve`，留意那条 `Generated N type(s)` 的启动消息 - 如果您看到的是 `No InertiaProps structs found`，说明这个扫描器没找到任何东西可以生成。

### 后端启动后立刻悄悄退出

当任何一个子进程退出时，这个管理器也会把另一个关掉。如果后端是带着一个编译错误死掉的，紧挨在「Servers stopped.」这条消息上方的 `[backend]` 那些行，会显示来自 rustc 的 `error[E…]`。修好这个编译错误，然后重新运行。

## 下一步

- [安装](installation.md) - 让 CLI 在您的机器上跑起来
- [快速上手](quickstart.md) - 一次完整的第一个应用走一遍
- [目录结构](structure.md) - `suprnova new` 脚手架出了什么
- [生成器](cli-generators.md) - `make:controller`、`make:action` 等等
- [控制台](console.md) - 逐项目的 `cargo run --bin console` 二进制文件
