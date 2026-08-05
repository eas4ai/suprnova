# 控制台

每个 Suprnova 项目都自带一个 `console` 二进制文件 - 这是面向一切需要用到应用编译期类型的东西的运行时命令分发器：数据库填充器、修剪器、一次性的维护任务，以及任何您会用 Laravel 的 `php artisan` 来构建的东西。命令要么是 `#[derive(Command)]`（构建在 `clap::Parser` 之上）的类型化结构体，要么是标注了 `#[command]` 的异步函数；框架在链接时通过 `inventory` 收集它们，所以添加一个新命令只需要一个文件，不需要编辑任何中心化的注册表。这是 `php artisan` 的 Suprnova 对应物 - 同一个脚本，同一个进程，同一个地址空间，处理程序返回时就退出。

## 快速上手

推荐的形态是用 `#[derive(clap::Parser, Command)]` 来处理类型化的参数：

```rust
use async_trait::async_trait;
use clap::Parser;
use suprnova::{Command, FrameworkError, TypedCommand};

#[derive(Parser, Command, Debug)]
#[console(name = "greet", description = "Print a friendly greeting")]
pub struct Greet {
    #[arg(short, long, default_value = "world")]
    pub name: String,

    #[arg(long, default_value_t = false)]
    pub loud: bool,
}

#[async_trait]
impl TypedCommand for Greet {
    async fn run(self) -> Result<(), FrameworkError> {
        let prefix = if self.loud { "HELLO" } else { "Hello" };
        println!("{prefix}, {}!", self.name);
        Ok(())
    }
}
```

把它放进 `src/commands/greet.rs`，在 `src/commands/mod.rs` 里加上 `pub mod greet;`，然后运行它：

```bash
cargo run --bin console -- greet
# Hello, world!
cargo run --bin console -- greet --name Alice --loud
# HELLO, Alice!
cargo run --bin console -- greet --help
# （clap 自动生成的逐命令帮助，包括那些类型化的标志）
```

没有需要编辑的中心化注册表。`#[derive(Command)]` 通过 inventory 提交一个 `CommandEntry { name, description, clap_builder, handler }`；console 二进制文件调用 `suprnova::console::dispatch_argv_with_init(argv, init)`，它会从每一条已注册的条目构建出一棵 clap 解析器树，只有当一个真实的子命令匹配时才运行 bootstrap 的 `init` 闭包，并把解析出来的 `ArgMatches` 路由到正确的处理程序。

### 更简单的路径：原始的 `Vec<String>`

对于那些不需要类型化参数的简单命令，标注在一个异步函数上的 `#[command]` 属性同样能用：

```rust
use suprnova::{command, FrameworkError};

#[command(name = "ping", description = "Smoke test")]
pub async fn ping(_args: Vec<String>) -> Result<(), FrameworkError> {
    println!("pong");
    Ok(())
}
```

两条路径在底层都落进同一个 `CommandEntry` 注册表；原始形态只是用一个带 `trailing_var_arg` 的 clap 子命令，把 argv 捕获进 `Vec<String>`。对于任何带参数的命令，都优先选用类型化的形态 - 您能免费得到逐命令的 `--help`、值解析、默认值，以及短/长标志配对，而不需要手写一个解析器。

## 控制台二进制文件

`suprnova new` 会给每个新项目脚手架出两个二进制文件：

- **`<project>`**（`cmd/main.rs` 或 `src/main.rs`） - HTTP 服务器，由 `cargo run` 或 `suprnova serve` 启动。长期存活；一直服务到被杀死为止。
- **`console`**（`src/bin/console.rs`） - 运行时命令分发器。一次性的；处理程序返回时就退出。

console 二进制文件的 `main` 短小而且行为可预测：

```rust
use std::process::ExitCode;

#[suprnova::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    // 通过 `--version` / `--help` 把这个项目的版本亮出来。
    // env! 解析出的是用户应用的版本，不是框架的版本。
    suprnova::console::set_version(env!("CARGO_PKG_VERSION"));

    let argv: Vec<String> = std::env::args().collect();
    let result = suprnova::console::dispatch_argv_with_init(argv, || async {
        my_app::config::register_all();
        my_app::bootstrap::register().await;
    })
    .await;

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}
```

Tokio 以 `current_thread` 这个 flavor 运行 - 在一个一次性的命令里没有什么工作值得跨核心并行化，多线程运行时的工作线程池只会是纯粹的开销。

有两件事值得注意：

- **Bootstrap 是惰性的。** 传给 `dispatch_argv_with_init` 的那个闭包，只有当 clap 匹配到一个真实的、已注册的子命令时才会运行。`console --help`、`console --version`、缺失子命令，以及解析错误这些路径，全都会跳过它 - 所以 `console --help` 在一个还没设置 `DATABASE_URL` 的全新检出上也能正常工作。
- **`main` 不打印错误。** `dispatch_argv_with_init` 拥有全部面向用户的 stderr - 它会把处理程序的错误消息 eprintln 出来（除非这个错误是静默的，比如一次 clap 已经自己打印过的解析失败），并打印 clap 自己的 help / version / 解析错误输出。`main` 是纯粹的 `Result → ExitCode` 转换；再加一个多余的 `eprintln!` 只会导致重复打印。

如果您想让某个特定命令完全跳过一个昂贵的 bootstrap 步骤，请把这个步骤本身用一个环境变量控制起来，而不是在框架里穿一条“惰性 bootstrap”标志线过去。

## 内置命令

框架自己会注册一小组命令。把框架链接进一个项目，就会自动把它们带进来。

| 命令 | 作用 |
|---|---|
| `db:seed` | 按顺序运行每一个已注册的 `Seeder`。接受 `--class=<Name>`（或者一个裸的位置参数）来运行单个具名的填充器，与 `php artisan db:seed --class=UserSeeder` 对应。 |
| `model:prune` | 遍历 `PrunerEntry` 注册表，强制删除每一个已注册的 `Prunable` / `MassPrunable` 作用域返回的每一行。`--model=<Name>` 把范围限制到一个类型；`--pretend` 只报告行数，不修改任何行。 |
| `--help` / `-h` | 列出可用的命令；逐子命令的 `--help` 由 clap 根据类型化的参数构建出来。 |
| `--version` | 打印由 `set_version` 注册的版本（通常是您应用的 `CARGO_PKG_VERSION`）。如果从未调用过 `set_version`，就完全省略。 |

`db:seed` 会运行您在 `bootstrap::register()` 里通过 `suprnova::seed::register::<MySeeder>()` 注册过的任何东西。在一个空的注册表上，它会打印一条警告并返回 `Ok(())` - 在注册填充器之前调用 `db:seed`，是一个无害的用户失误，不是一个程序员的错误。

> 那些工作守护进程（`queue:work`、`schedule:run`、`schedule:work`、`schedule:list`、`workflow:work`）**不在** console 二进制文件上。它们生活在 app/server 二进制文件的 clap 解析器上（也就是那个提供 HTTP 服务的二进制文件）。全局的 `suprnova` CLI 会为它们 shell 进 `cargo run --quiet -- <name>`。参见下面的[不对称性一节](#与-suprnova-migrate-的不对称性)。

## 定义命令

两个宏，一个注册表。挑一个适合这个命令形态的就好。

### `#[derive(Command)]` - 类型化参数（推荐）

叠加在 `#[derive(clap::Parser)]` 之上。结构体字段就是这个命令的参数；clap 把 argv 解析进这个结构体；框架调用您的 `TypedCommand::run(self)`。

```rust
use async_trait::async_trait;
use clap::Parser;
use suprnova::{Command, FrameworkError, TypedCommand};

#[derive(Parser, Command, Debug)]
#[console(name = "users:purge", description = "Purge users older than N days")]
pub struct UsersPurge {
    #[arg(long)]
    pub older_than_days: u32,

    #[arg(long, default_value_t = false)]
    pub dry_run: bool,
}

#[async_trait]
impl TypedCommand for UsersPurge {
    async fn run(self) -> Result<(), FrameworkError> {
        // self.older_than_days、self.dry_run - 类型化的，由 clap 验证过
        Ok(())
    }
}
```

属性：

| 属性 | 是否必需 | 用途 |
|---|---|---|
| `#[console(name = "...")]` | 是 | 在 CLI 上的调用名（`"users:purge"`、`"mail:send"`、`"greet"`）。 |
| `#[console(description = "...")]` | 否 | 在顶层帮助里显示的一行描述。 |
| `#[arg(...)]`（clap） | 不适用 | clap 自己的字段属性，用于短/长标志、默认值、值解析器等等。 |

您还能免费得到 clap 自动生成的逐命令帮助（`console users:purge --help`）。

### `#[command]` - 原始的 `Vec<String>`（简单场景）

对于那些不接受参数、或者只把位置参数当作一个列表来消费的命令，标注在一个异步函数上的这个属性就足够了：

```rust
use suprnova::{command, FrameworkError};

#[command(name = "cache:clear", description = "Drop every entry from the cache")]
pub async fn cache_clear(_args: Vec<String>) -> Result<(), FrameworkError> {
    suprnova::Cache::flush().await
}
```

被标注的函数必须是 `async fn(Vec<String>) -> Result<(), FrameworkError>`。这个宏会保留原始的函数，所以您也可以直接从 Rust 里调用它 - 这对那些不想把 argv 字符串穿过分发器的单元测试很有用。

两种形态里的名字都支持 Laravel 风格的命名空间划分：`mail:send`、`queue:work`、`db:fresh`。冒号纯粹是装饰性的 - 它只是分发器用来匹配 `argv[1]` 的一个字符串。

## `suprnova make:command`

CLI 生成器会放下一个可运行的骨架。生成出来的文件使用**类型化的形态**（`#[derive(Parser, Command)]` + `impl TypedCommand`） - 这是推荐的默认选择，而且能免费给您逐命令的 `--help`：

```bash
suprnova make:command cache:clear
# → src/commands/cache_clear.rs（带有 #[console(name = "cache:clear")] 的 pub struct CacheClear）
# → src/commands/mod.rs 被追加上 `pub mod cache_clear;`（如果不存在就创建）
```

这个骨架可以原样运行 - `cargo run --bin console -- cache:clear` 会打印 `cache:clear: not yet implemented`，并返回 `Ok(())`，这样您就可以把它接好、慢慢迭代。为类型化参数填充结构体上的字段，并替换掉 `TypedCommand::run` 的方法体。

名字规范化：

| 输入 | 文件 | 命令名 |
|---|---|---|
| `greet` | `greet.rs` | `greet` |
| `CleanCache` | `clean_cache.rs` | `clean-cache` |
| `clean-cache` | `clean_cache.rs` | `clean-cache` |
| `mail:send` | `mail_send.rs` | `mail:send` |

如果输入包含 `:`，这个冒号命名空间会被原样保留。否则 Rust 函数名会是 snake_case，命令名会是 kebab-case。

请确保 `src/lib.rs` 里声明了 `pub mod commands;`，这样这条 inventory 提交才能从 console 二进制文件那边被链接到。生成器会为新项目脚手架好这一行，并在它缺失时发出一条醒目的警告；如果您把它删掉了，新文件里的 `inventory::submit!` 块依然能编译，但永远不会进入那个注册表。

### 为什么 Suprnova 有所不同

框架故意**不**为像 `db:seed` 这样的运行时任务做一个全局的 `suprnova` CLI 命令。一个全局二进制文件没法静态加载您应用的填充器、工厂，或者 `#[command]` 异步函数，除非：

- shell 出去调用 `cargo run --bin app -- ...`（慢 - 每次调用都要完整编译一次，这就违背了初衷），或者
- 动态加载（对 v1 来说复杂度太高）

所以，用户的项目会产出一个 `console` 二进制文件。直接运行它：

```bash
./target/debug/console db:seed
./target/release/console greet Alice
cargo run --bin console -- mail:send
```

Laravel 用 `php artisan` 解决了同样的问题 - 一个逐项目的脚本，启动框架并分发到用户定义的命令。PHP 能动态地做到这一点，是因为框架代码在运行时就在用户代码旁边。Rust 的编译加链接模型排除了这条路，所以我们把这个分发器作为一个库来发布（`suprnova::console::*`），让每个项目去链接它自己那一行的 `console` 二进制文件。

### 与 `suprnova migrate` 的不对称性

在一个 Suprnova 项目里，有三条截然不同的命令调用路径，这种不对称是**结构性的** - 不要试图去统一它们：

| 命令表面 | 调用方式 | 原因 |
|---|---|---|
| `suprnova new`、`suprnova make:*`、`suprnova serve`、`suprnova key:generate`，…… | 全局 CLI 二进制文件（通过 `cargo install --git` 安装） | 只生成文件的生成器和脚手架工具；不需要用户代码。 |
| `suprnova migrate`、`suprnova migrate:status`、`suprnova schedule:run`、`suprnova schedule:work`、`suprnova schedule:list`、`suprnova workflow:work` | 全局 CLI 针对 app/server 二进制文件 shell 进 `cargo run --quiet -- <name>` | 长期运行的守护进程，以及那个由同一个 `Application::run` 的 clap 解析器所拥有的模式相关工作。服务器二进制文件的 `queue:work` 也生活在这里 - `cargo run --bin <app> -- queue:work`。 |
| `console db:seed`、`console model:prune`、`console <your-command>` | 每个项目自己的 `console` 二进制文件（`src/bin/console.rs`） | 需要把用户类型（填充器、命令、可修剪的模型）编译进用户 crate 的一次性命令。 |

这种拆分是有意为之的。服务器二进制文件本来就需要一个 clap 解析器，来在 `serve`、`migrate`、`queue:work` 等之间做选择；与它共享生命周期的守护进程就住在那里。console 二进制文件的存在，是为了容纳其余的一切 - 短命的、用户定义的、类型丰富的东西。新的运行时命令，应该归属于由项目的 `console` 二进制文件分发的 `#[command]` / `#[derive(Command)]`。

## 最佳实践

### 让处理程序保持精简；通过容器去拿共享服务

一个 `#[command]` 是 CLI 形状的包装器；业务逻辑应该住在一个操作、一个服务，或者一个模型的方法里。处理程序解析参数，从容器里解析出服务，然后转发。这让同一份逻辑能够从一个单元测试、一条 HTTP 路由，以及 console 里，都得到测试。

```rust
#[command(name = "users:purge")]
pub async fn users_purge(args: Vec<String>) -> Result<(), FrameworkError> {
    let action = App::resolve::<PurgeStaleUsers>()?;
    action.execute(parse(args)?).await
}
```

`App::resolve` 返回 `Result<T, FrameworkError::ServiceUnresolved(_)>` - 是 `App::get`（返回 `Option`）的 `?` 风味版本。完整的接口请参见[服务容器](container.md)。

### 给相关的命令用命名空间

用 `:` 来分组：`mail:send`、`mail:retry`、`mail:queue:work`。分发器把它当作不透明的字符串来对待，但人眼扫视 `mail:*` 会比 `send-mail`、`retry-mail`、`mail-queue-work` 更轻松。

### 不要打印结构化数据 - 返回它

Console 处理程序把面向人类阅读的输出打印到 stdout。如果下游有工具需要消费这份输出，就写一个 `console <name> --json` 变体，把机器可读的 JSON 发到 stdout，把一行状态发到 stderr。不要让那条面向人类的路径同时对两种受众负责。

### 把退出码当作契约来对待

`FrameworkError` → `ExitCode::FAILURE` 是唯一的失败路径。不要在处理程序内部调用 `std::process::exit(custom_code)` - 返回 `Err(...)`，让二进制文件的 `main` 去做转换。未来的工具（CI 门禁、受监督的工作进程）只需要读这个退出码就够了。

## 参考

| 符号 | 用途 |
|---|---|
| `suprnova::Command`（derive） | 把一个派生自 `clap::Parser` 的结构体注册为一个类型化的 console 命令。与 `TypedCommand` 配对使用。 |
| `suprnova::TypedCommand`（trait） | 带有 `async fn run(self) -> Result<(), FrameworkError>` 的 trait - 一个类型化命令的方法体。 |
| `suprnova::command`（属性） | 把一个接受 `Vec<String>` 的异步函数注册为一个原始参数的 console 命令。 |
| `suprnova::console::dispatch_argv(argv)` | 从每一条已注册的条目构建出 clap 解析器树，解析 argv，路由到处理程序。没有惰性初始化 - 便于测试和程序化调用者使用。 |
| `suprnova::console::dispatch_argv_with_init(argv, init)` | 和 `dispatch_argv` 一样，但会在 clap 解析完 argv 和匹配到的处理程序之间运行那个 `init` 闭包。这个 init 只有在一个真实的子命令匹配时才会触发 - `--help` / `--version` / 解析错误路径都会跳过它。脚手架出来的 `console` 二进制文件用的就是这个。 |
| `suprnova::console::set_version(&'static str)` | 注册通过 `--version` 和 `--help` 亮出来的版本字符串。在 `main` 开头调用一次。第一次注册的胜出。 |
| `suprnova::console::find(name)` | 按精确的名字查找一个已注册的命令。 |
| `suprnova::console::list()` | 全部已注册的命令，按名字排序。 |
| `suprnova::CommandEntry` | Inventory 记录：`{ name, description, clap_builder, handler }`。由两个宏共同提交。 |
| `suprnova::CommandHandler` | 处理程序的函数指针类型：`fn(&clap::ArgMatches) -> Pin<Box<dyn Future<...>>>`。 |
| `FrameworkError::silent()` / `.is_silent()` | 构造 / 检测一个分发器**不会**打印到 stderr 的错误。内部用它来抑制重复打印 - 当 clap 已经把一次解析错误写到终端时。 |

## 下一步

- [应用启动](bootstrap.md) - `dispatch_argv_with_init` 闭包内部运行的是什么
- [服务容器](container.md) - `App::resolve` 与 `App::get` 的区别，以及一个处理程序如何触达共享服务
- [数据填充](seeding.md) - `db:seed` 实际调用的是什么
- [Eloquent](eloquent.md) - `Prunable`、`MassPrunable`，以及 `model:prune` 如何遍历这个注册表
- [任务调度](scheduling.md) - 这种不对称性：调度器守护进程生活在 app 二进制文件上，而不是 console 上
