# 哈希

`suprnova::hashing` 模块是框架的密码哈希表面，提供三个头等驱动程序 - **bcrypt**（默认，与 Laravel 一致）、**Argon2i**（内存困难型，抗侧信道）和 **Argon2id**（OWASP 2024 推荐）。在存储用户密码、对记住我验证令牌做哈希，或者任何单向函数才是正确原语的场合，都用它。驱动程序的选择由环境变量驱动，而这个门面从头到尾都是算法感知的（`info`、`is_hashed`、`needs_rehash`、`verify`），所以即使您把 `HASH_DRIVER=argon2id` 打开，一个已存储的 bcrypt 哈希依然能通过验证。

## 概览

```rust
use suprnova::hashing;

// 异步（在 Tokio 请求处理程序内部优先使用 - 会把这个 CPU 密集型的
// 哈希运算放到 spawn_blocking 上运行，好让工作线程保持空闲）：
let hashed = hashing::hash_async("my_password").await?;
let valid = hashing::verify_async("my_password", &hashed).await?;

// 同步（测试、CLI 工具、非异步上下文）：
let hashed = hashing::hash("my_password")?;
let valid = hashing::verify("my_password", &hashed)?;
```

这个自由函数门面会从 `HASH_DRIVER` 读取当前活跃的驱动程序（读不到就回退到 bcrypt）。对于要显式指定驱动程序的调用，直接构造驱动程序类型，再把它传给 `hash_with` / `verify_with` / `needs_rehash_with`。

## 配置

| 变量 | 说明 | 默认值 | 范围 |
|----------|-------------|---------|-------|
| `HASH_DRIVER` | 活跃算法 | `bcrypt` | `bcrypt` \| `argon` \| `argon2i` \| `argon2id` |
| `HASH_ROUNDS` | Bcrypt 成本因子 | `12` | `4..=31`（仅 bcrypt） |
| `HASH_MEMORY` | Argon 内存成本，以 KiB 计 | `65536`（64 MiB） | `>= 8`（仅 argon） |
| `HASH_TIME` | Argon 时间迭代次数 | `4` | `>= 1`（仅 argon） |
| `HASH_THREADS` | Argon 并行度 / 通道数 | `1` | `>= 1`（仅 argon） |
| `HASH_VERIFY` | 为 true 时，`verify()` 会拒绝跨算法的哈希 | `false` | `true` / `false` |

配置错误（值不对、参数超出范围）会在第一次调用 `hash` / `verify` / `needs_rehash` 时以 `FrameworkError::param` 的形式暴露出来 - 而不是悄悄回退到一个默认值。

### argon2id 的 `.env` 示例

```env
HASH_DRIVER=argon2id
HASH_MEMORY=65536
HASH_TIME=4
HASH_THREADS=1
```

### 为什么 Suprnova 的 Argon2 默认值比 Laravel 的更强

| 参数 | Laravel 默认值 | Suprnova 默认值 | 来源 |
|-------|-----------------|------------------|--------|
| 内存 | 1 024 KiB（1 MiB） | 65 536 KiB（64 MiB） | OWASP 2024 |
| 时间 | 2 次迭代 | 4 次迭代 | OWASP 2024 |
| 线程 | 2 | 1 | OWASP 2024 / 与 libsodium 对齐 |

Laravel 的默认值假定的是 PHP 那种每进程一个请求的模型 - 一个工作进程在每一次密码哈希上能花的时间是有限的，超了这个箱子就装不下了。Tokio 的 `spawn_blocking` 让 Suprnova 可以把这次哈希运算甩给一个阻塞线程池，而不冻住请求循环，所以 OWASP 2024 给出的这些数字，在真实的生产硬件上是可行的。

## 驱动程序

### Bcrypt（默认）

```rust
use suprnova::hashing::{BcryptHasher, BcryptOptions, hash_with, verify_with};

let driver = BcryptHasher::new(BcryptOptions { rounds: 14 });
let hashed = hash_with(&driver, "my_password")?;
assert!(verify_with(&driver, "my_password", &hashed)?);
```

Bcrypt 对密码输入设有一个 **72 字节的块大小上限** - 底层原语会静默截断更长的输入，这意味着两个不同的密码短语，只要前 72 个字节相同，就会哈希出同一个值。Suprnova 会提前拒绝（框架的 bcrypt 路径会在 `hash()` 上报错，并对超长密码在 `verify()` 上返回 `Ok(false)`，让认证流程的“invalid credentials”响应保持统一的样子）。Argon2 没有这样的上限。

这个 bcrypt 上限以 `suprnova::hashing::MAX_BCRYPT_PASSWORD_BYTES` 的形式暴露出来（71 - 也就是 bcrypt 的空终止符之后实际可用的上限）。

### Argon2id（OWASP 2024 推荐）

```rust
use suprnova::hashing::{Argon2idHasher, Argon2Options, hash_with, verify_with};

let driver = Argon2idHasher::new(Argon2Options {
    memory: 65_536,  // 64 MiB
    time: 4,
    threads: 1,
})?;

let hashed = hash_with(&driver, "my_password")?;
assert!(verify_with(&driver, "my_password", &hashed)?);

// Argon2 接受任意长度的密码短语 - bcrypt 那 72 字节的
// 上限不适用于它。
let long = "x".repeat(500);
let h = hash_with(&driver, &long)?;
assert!(verify_with(&driver, &long, &h)?);
```

### Argon2i

形态和 Argon2id 相同；`Argon2iHasher::new(opts)`。新项目请用 Argon2id - Argon2i 的存在是为了对等支持，但 Argon2id 才是现代推荐。

## 带显式成本的 Bcrypt（`hash_with_cost`）

`hash_with_cost(password, cost)` 和 `hash_with_cost_async(password, cost)` 会以调用方提供的成本因子铸造一个 bcrypt 哈希，无论 `HASH_DRIVER` 是什么。当策略或者按租户的配置需要把成本值下推到调用点，而不是下推到进程环境时，就用这两个函数 - 举例来说，一个高安全账户类别用成本 14，而应用其余部分跑在默认的成本 12 上。

```rust
use suprnova::hashing::{hash_with_cost, hash_with_cost_async};

// 同步 - 测试、CLI 工具。
let h = hash_with_cost("my_password", 14)?;

// 异步 - 在 Tokio 请求处理程序内部。
let h = hash_with_cost_async("my_password", 14).await?;
```

这两个入口都会以 `FrameworkError::param` 拒绝落在 `MIN_BCRYPT_COST..=MAX_BCRYPT_COST`（`4..=31`）之外的 `cost`，和环境变量那一侧的 `HASH_ROUNDS` 校验镜照：

```rust
use suprnova::hashing::{hash_with_cost, MIN_BCRYPT_COST, MAX_BCRYPT_COST};

assert!(hash_with_cost("pw", MIN_BCRYPT_COST - 1).is_err()); // < 4
assert!(hash_with_cost("pw", MAX_BCRYPT_COST + 1).is_err()); // > 31
```

这个边界检查很要紧，因为每提高一级成本，CPU 时间就翻一倍。在成本 31 上，单次 bcrypt 哈希在普通硬件上要跑上几个小时 - 框架内部的这道边界检查，防止一个策略或配置上的手误，意外地把一个工作线程钉死一整天。异步版本会经过 `spawn_blocking`，所以即使是一个合理地设得很高的成本，也不会冻住请求循环。

## 算法感知的 needs_rehash

`needs_rehash` 会在已存储的哈希应当在当前活跃驱动程序下重新哈希时返回 `true`。它涵盖三种情形：

1. **算法不匹配** - 存的是 bcrypt 哈希，而 `HASH_DRIVER=argon2id`（或者反过来）。会在下一次成功验证时触发一次轮换。
2. **参数偏弱** - bcrypt 成本低于 `HASH_ROUNDS`，或者 argon 的 `m`/`t`/`p` 低于 `HASH_MEMORY`/`HASH_TIME`/`HASH_THREADS`。
3. **Bcrypt 遗留变体** - `$2a$`、`$2x$`、`$2y$` 即便是在配置好的成本下，也会轮换到规范的 `$2b$`。

```rust
if hashing::needs_rehash(&stored_hash) {
    let fresh = hashing::hash_async("plaintext_at_login").await?;
    // 持久化 `fresh`。标准的 Laravel“登录成功时重新哈希”
    // 模式；跨算法都能用。
}
```

格式错误的输入会返回 `true` - 调用方自然会把它解析不了的任何东西都拿去轮换。

## 哈希检视（`info` + `is_hashed`）

```rust
use suprnova::hashing::{info, is_hashed};

let h = hashing::hash_async("my_password").await?;
let i = info(&h);
println!("algo: {}", i.algo.as_str());
println!("bcrypt cost: {:?}", i.rounds);
println!("argon memory KiB: {:?}", i.memory);

// 对任何可识别的算法哈希都为 true；对明文 / 垃圾数据为 false。
assert!(is_hashed(&h));
assert!(!is_hashed("plaintext"));
```

`info().algo` 是下面之一：`Bcrypt`、`Argon2i`、`Argon2id`、`Argon2d`（可识别，但从不会被铸造出来）、`Unknown`。

`AsHashed` 这个 eloquent 转换器正是靠 `is_hashed` 来跳过对一个已经哈希过的列重新哈希 - 三个驱动程序全都适用，所以在项目中途切换 `HASH_DRIVER`，也不会在下一次保存时触发哈希叠哈希的循环。

## 跨算法验证门（`HASH_VERIFY`）

默认情况下，`verify()` 会拿密码去对照哈希做检查，而不管这个哈希是哪种算法产出的 - 这正是为什么即便您把 `HASH_DRIVER=argon2id` 打开，遗留的 bcrypt 哈希依然能通过验证（这样您就可以在登录时把它们轮换掉）。一旦每个用户都完成了轮换，就设置 `HASH_VERIFY=true`，严格执行当前活跃的算法：

```env
HASH_VERIFY=true
```

打开这道门之后，`verify()` 会对任何算法与活跃驱动程序不同的哈希返回 `Ok(false)` - 形态上和 Laravel 的 `RuntimeException` 一样，但 Suprnova 返回的是 false 而不是抛出异常，因为认证流程里的调用方无论如何都指望着一个 `Result<bool>`。

## 异步对比同步

成本 12 下的 bcrypt（约 250 毫秒）和内存 = 64 MiB 下的 Argon2id（约 80 毫秒）都是故意做成 CPU 密集型的 - 这正是慢哈希存在的全部意义。直接从一个 Tokio 请求处理程序里调用同步的 `hash` / `verify`，会在整个哈希运算期间阻塞这个工作线程，饿死同一个工作线程上的其他请求。

请在 `async fn` 处理程序内部使用对应的 `*_async` 版本。它们把这个 CPU 密集型调用包进 `tokio::task::spawn_blocking`，让工作线程保持空闲，可以去处理别的请求：

```rust
// 好 - 在一个异步处理程序内部
let hashed = hashing::hash_async(&form.password).await?;

// 差 - 阻塞工作线程约 250 毫秒
let hashed = hashing::hash(&form.password)?;
```

同步版本是给测试、CLI 工具，以及其他阻塞无妨的非异步上下文用的。

## Eloquent 整合：`AsHashed` 转换器

`#[cast(AsHashed)]` 这个 eloquent 转换器会在写入时用当前活跃的驱动程序对一个明文字段做哈希，并且**在所有驱动程序之间都是幂等的** - 保存一个 `password` 列已经包含一个可识别哈希（bcrypt 或 argon）的模型时，这个值会原样通过，不受影响。没有这层防护，`User::find(id).await?.save().await?` 就会在每次保存时把现有的哈希再哈希一遍，把认证弄坏。

```rust
use suprnova::eloquent::casts::AsHashed;

#[suprnova::model]
struct User {
    #[cast(AsHashed)]
    pub password: String,
    // ...
}
```

这个幂等性检查用的是 `hashing::is_hashed`，所以在项目中途切换 `HASH_DRIVER` 是安全的 - 遗留的 bcrypt 哈希和新鲜的 argon2id 哈希都能被识别出来，并在重新保存时被跳过。

## 与 `Auth::attempt` 搭配使用

`Auth::attempt(&credentials)` 会调用 `UserProvider::validate_credentials`，后者又会拿用户已存储的哈希去调用 `hashing::verify_async`。验证是按*已存储*哈希的算法来分派的，不是按配置的驱动程序 - 所以在您把 `HASH_DRIVER=argon2id` 打开之后，每一个既有的 bcrypt 哈希依然能通过验证，而 `needs_rehash` 会返回 `true`，于是标准的登录时轮换模式会一次一个登录地，把整个用户群体带到新算法上。

## 在测试中覆盖驱动程序

`set_default_driver(Box<dyn Hasher>)` 会为测试和内嵌的 CLI 工具以编程方式安装一个驱动程序，供那些不经过 `HASH_DRIVER` 就构造驱动程序的场景使用。它是一次性的 - 第一次调用生效，第二次调用会返回 `FrameworkError::internal`，而不会在进程运行中途替换掉这个驱动程序。请在测试套件启动时、在任何代码路径解析出默认值之前使用它。

## 下一步

- [认证](authentication.md) - `Auth::attempt`、用户提供者 trait，以及哈希如何与登录整合
- [认证流程](auth-flows.md) - `PasswordReset::complete` 会把已存储的密码哈希通过当前活跃的驱动程序轮换一遍；记住我令牌在存储之前会经由 `hash_async` 做哈希
- [Eloquent](eloquent.md) - `#[cast(AsHashed)]` 参考，以及更广的转换器表面
- [加密](encryption.md) - 面向静态数据的双向认证加密；是单向哈希的互补
- [错误模型](error-model.md) - 当一个哈希配置值被拒绝时，`FrameworkError::param` 长什么样
