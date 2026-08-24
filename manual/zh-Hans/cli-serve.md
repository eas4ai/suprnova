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
| `--no-restart` | `false` | 不重新 spawn 崩溃的开发进程 - 而是拆掉整个会话（旧行为） |
| `--restart-tries <N>` | `5` | 在连续崩溃达到此次数后放弃重试进程。与 `--no-restart` 一起使用时忽略，后者已会在第一次崩溃时结束会话。 |
| `--timestamps` | `false` | 为每个输出行添加 `HH:MM:SS` 时钟时间前缀 |
| `--json` | `false` | 在 stdout 上输出每行一个 JSON 对象（NDJSON），而不是带前缀的文本 - 参见[JSON 输出](#json-output)。与 `--timestamps` 组合不是错误；`--timestamps` 不会有额外效果，因为每个事件已携带自己的时间戳。 |

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
9. spawn 项目 `Suprnova.toml` 中声明的每个额外进程（见下方[额外开发进程](#extra-dev-processes)），每个都有自己的 `[name]` 前缀 - 队列工作进程、日志 tailer，或任何您否则要在另一个终端中调度的东西。
10. 在 `src/` 上启动一个文件监视器，每当一个 `.rs` 文件变化，并且这一连串保存已经安静了 500 毫秒之后，就重新运行这个类型生成器。这个防抖是后沿触发的，所以一连串的变更 - `cargo fmt`、跨多个文件的保存时格式化、一次分支切换 - 会合并成恰好一次再生成，在最后一次写入*之后*运行，而不是在第一个文件上就触发、错过剩下的文件。
11. 把每个子进程的 stdout/stderr 都转发到您的终端，带着 `[name]` 前缀（`[backend]`、`[frontend]`，或进程的配置名称），可选择用 `--timestamps` 加上时间戳 - 或者使用 `--json` 时改为 NDJSON 事件（见下方[JSON 输出](#json-output)）。

`Ctrl+C` 会通知这个管理器去设置它的关闭标志、杀掉每个子进程，然后退出。如果一个子进程自己退出了 - 一次 `cargo watch` 无法恢复的严重 Rust 编译错误、崩溃的 Vite 进程、失败的 `Suprnova.toml` 进程 - 它会在短暂退避后重新 spawn（200ms，每次连续崩溃翻倍，上限为 5s；持续运行 30s 的进程会重置该爬升），而不是拆掉会话。传递 `--no-restart` 可恢复旧行为：任一子进程退出会立即关闭整个会话。

持续崩溃的进程不会永远重试：`--restart-tries`（默认 `5`）会限制 `serve` 在放弃该一个进程之前重试的连续崩溃次数 - 持续运行 30 秒会重置计数，与退避延迟相同。放弃时会打印可操作的消息，并**只**停止重试该进程；其他进程（以及会话本身）继续运行，这与 Laravel 自己的 `concurrently --restart-tries=5` 默认值相匹配。参见[故障排查](#a-process-keeps-crash-looping)。

### 为什么 Suprnova 有所不同

Laravel 用户通常会用 `php artisan serve` 跑后端，在另一个终端里跑 `npm run dev`，大多数团队会用一个 `Procfile` 加 `foreman`/`overmind` 来掩盖这种两个终端的割裂。Suprnova 把这个多路复用器当作一个一等的 CLI 命令来发布。您得到的是一个终端、一次 `Ctrl+C`、自动的工具链引导（`cargo-watch`、`npm install`），以及一座类型化的 Inertia 桥，它会随时再生成 `frontend/src/types/inertia-props.ts`，这样您的 Svelte/React/Vue 组件永远能看到当前的 prop 形态，不需要手动同步类型。

Laravel 的 `dev` 命令也提供 `--tabs` 和 `--stream` 模式，两者都通过一个小型 Node TUI（`@laravel/multiplex`）渲染输出。Suprnova 不提供该 TUI：带前缀的单终端输出是 Rust 开发工具生态（`cargo watch`、`bacon`、`just`）的常态，带彩色前缀的进程注册表已提供 TUI 所提供的“哪个进程说了这句话”信号。`--stream` 的底层工作 - 一个可脚本化的实时事件流 - 作为 `--json` 提供（见[JSON 输出](#json-output)）；`--tabs` 的多窗格 TUI 是刻意不做，不是缺口 - 对本页已经解决的问题，第二种交互模型以及第二个需要跨终端保持可用的库是不值得的。参见[兼容性](parity.md#what-we-wont-ship-and-why)中的相应行。

## 热重载

**后端。** `cargo watch -x 'run --bin <package>'` 就是这个循环。项目里每一次 `.rs` 变化，它都会重新编译并重启服务器。改动一个很重的 crate 之后的冷编译可能要花几秒钟；单个文件内的增量变化通常不到一秒。

**前端。** Vite 的 HMR 会就地注入组件变更，不需要整页重载，还会保留组件状态。Tailwind 类会通过 Tailwind v4 的监视器实时更新。

**TypeScript 类型。** 每当一个 `.rs` 文件变化，类型监视器就会重新运行这个生成器。如果出现了新的 `#[derive(InertiaProps)]` 结构体（或者已有的结构体改变了形态），重新生成的 `frontend/src/types/inertia-props.ts` 就会为导入它们的组件触发 Vite 的 HMR。

## 额外开发进程

`suprnova serve` 总会运行后端和 Vite，但大多数项目有不止两件要保持运行的事 - 队列工作进程、日志 tailer、邮件捕获器。请在项目根目录的 `Suprnova.toml` 中声明它们，`serve` 会在后端和前端旁边 spawn、加前缀并自动重启：

```toml
[[serve.process]]
name = "queue"
command = "cargo"
args = ["run", "--bin", "console", "--", "queue:work"]
color = "yellow"

[[serve.process]]
name = "logs"
command = "tail"
args = ["-f", "storage/logs/app.log"]
```

每个条目都需要 `name` 和 `command`；`args` 默认没有，`color` 默认按声明顺序分配 green/yellow/blue/white 之一（或者选择八个命名 `console` 颜色之一 - black、red、green、yellow、blue、magenta、cyan、white）。名称必须唯一。`Suprnova.toml` 完全是可选的；没有它的项目会完全按以前的方式运行。

### 为什么 Suprnova 有所不同

Laravel 会从 PHP 中注册额外 `dev` 进程 - `DevCommands::register($command, $name)`，通常在服务提供者的 `boot()` 中 - 因为 `php artisan dev` 从已经启动应用的同一进程内部执行一个多路复用器。`suprnova serve` 是独立于您的应用的二进制文件；它从不链接或运行您的 Rust 代码，只会 shell out 到 `cargo watch` 和 `npm`。没有可挂接的应用启动过程，所以注册必须是 CLI 读取的数据，而不是您的代码进行的调用 - 因此使用 `Suprnova.toml`，而非 `DevProcesses::register()` API。

## JSON 输出

传递 `--json` 后，`suprnova serve` 会在 stdout 输出每行一个 JSON 对象（NDJSON），而不是带彩色 `[name]` 前缀的文本 - 激活期间没有其他内容写入 stdout，因此您可以直接管道给 `jq` 或其他面向行的 JSON 消费者。每一行都有一个 `type` 字段：

| `type` | 字段 | 含义 |
|---|---|---|
| `started` | `ts`、`name`、`pid` | 一个进程（后端、前端或 `Suprnova.toml` 条目）首次被 spawn。 |
| `output` | `ts`、`name`、`stream`（`"stdout"` 或 `"stderr"`）、`line` | 子进程的一行输出，作为字段携带而不是原样透传。 |
| `exited` | `ts`、`name`、`code`（nullable） | 一个进程退出。若由信号杀死而不是返回状态，则 `code` 是 `null`。 |
| `restart_scheduled` | `ts`、`name`、`delay_ms` | 崩溃进程会在 `delay_ms` 后重新 spawn（见上文退避计划）。 |
| `restart_succeeded` | `ts`、`name`、`pid` | 计划的重新 spawn 成功；进程在新的 PID 下再次运行。 |
| `gave_up` | `ts`、`name`、`tries` | 进程连续崩溃 `tries` 次（`--restart-tries`），`serve` 已停止重试它。会话及每个其他进程继续运行。 |
| `types_regenerated` | `ts`、`artifact`（`"inertia_props"` 或 `"lang_keys"`）、`count` | 文件监视器响应 `.rs`/`.ftl` 变更，重新生成了 TypeScript 产物。 |
| `shutdown` | `ts` | 会话正在关闭。始终是最后一行。 |

例如，一次 Vite 崩溃及其重新 spawn 如下：

```json
{"type":"exited","ts":"2026-08-18T10:15:23.456-07:00","name":"frontend","code":1}
{"type":"restart_scheduled","ts":"2026-08-18T10:15:23.456-07:00","name":"frontend","delay_ms":200}
{"type":"restart_succeeded","ts":"2026-08-18T10:15:23.657-07:00","name":"frontend","pid":48391}
```

`--json` 与 `--timestamps` 可组合而非相互冲突：组合它们不是错误，但 `--timestamps` 没有额外效果，因为每个事件已携带自己的 `ts` 字段。

这是其他工具会解析的机器可读输出 - 未经 changelog 注明，字段名称和 `type` 值不会改名或删除。请将无法识别的 `type` 或意外的额外字段视为要忽略的内容，而不是错误，使未来版本可以扩展模式而不破坏您的消费者。

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

### 进程持续崩溃循环

若一个子进程 - 后端、前端或 `Suprnova.toml` 条目 - 无法启动（代码错误、缺少二进制文件、端口冲突），它会按上述退避计划重新 spawn，而不是停止。请查看每个“将在 …ms 后重新 spawn”通知之前的 `[name]` 行，了解真正的错误（rustc 的 `error[E…]`、ENOENT，或子进程打印的任何内容）。修正原因；下一次重新 spawn 尝试会自动拾取它。若要停止重试并仅看一次失败，请以 `--no-restart` 重新运行 - 会话随后会在第一次崩溃时关闭，与此功能出现前 `suprnova serve` 的行为相同。

连续崩溃达到 `--restart-tries`（默认 `5`）后，`serve` 自行停止重试该进程，并打印命名它的消息：

```text
gave up restarting `backend` after 5 attempts; fix the error and run `suprnova serve` again
```

其他进程和会话本身会继续运行 - 请修正原因并重新运行 `suprnova serve` 来带回已放弃的进程；不需要为了它重启整个会话。

## 下一步

- [安装](installation.md) - 让 CLI 在您的机器上跑起来
- [快速上手](quickstart.md) - 一次完整的第一个应用走一遍
- [目录结构](structure.md) - `suprnova new` 脚手架出了什么
- [生成器](cli-generators.md) - `make:controller`、`make:action` 等等
- [控制台](console.md) - 逐项目的 `cargo run --bin console` 二进制文件
