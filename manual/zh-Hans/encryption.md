# 加密

Suprnova 以一个进程全局的门面 `Crypt` 的形式，提供应用层加密。它会用 AES-256-GCM 加密字符串或者任何 `Serialize` 值，密钥来自您的 `APP_KEY`。当您需要把某些敏感的东西放进一个您不完全信任的存储里 - 一个列、一个 cookie、一个分页游标 - 并且之后还需要把它原样读回来时，就伸手去用它。

```rust
use suprnova::{Crypt, CryptPurpose};

let wire = Crypt::encrypt_string(CryptPurpose::Cast, "ssn-123-45-6789")?;
let plain = Crypt::decrypt_string(CryptPurpose::Cast, &wire)?;
assert_eq!(plain, "ssn-123-45-6789");
```

框架自身在加密 cookie、加密的分页游标、2FA 密钥、恢复码，以及
`AsEncrypted*` 这些 Eloquent 转换器上都在用 `Crypt`。一旦配置好了
`APP_KEY`，同一个门面无需任何额外接线，就能在您的代码里直接使用（参见 [configuration.md](configuration.md#the-env-file)）。

## 传输格式

`encrypt_string` 和 `encrypt` 都返回 URL-safe base64（无填充），编码的对象是 `nonce || ciphertext_with_tag`：

```
base64url( [12 字节随机数] || [密文] || [16 字节 GCM 标签] )
```

每一次调用都会从操作系统的 RNG 里取一个新的 12 字节随机数，所以同一把密钥对同一段明文做两次加密，会产出不同的密文。这里没有填充预言机会泄露超出明文本身的长度信息。

这个输出可以安全地放进 URL 查询字符串、JSON 正文、请求头和
cookie 里，不需要再做额外编码。一个合法的传输值最少是 28 字节（12 字节随机数 + 16 字节标签） - 更短的会被直接拒绝。

## `APP_KEY` - 唯一要紧的秘密

Suprnova 从 `APP_KEY` 环境变量里读取一个单一的 32 字节对称密钥。期望的格式是 URL-safe base64、无填充，解码后正好是 32 字节（43 个 base64 字符）：

```env
APP_KEY=hQ7rW0X9_NkSi8Cw5fF8j6V_K6JzgB3y2Hq9LpL9-Wo
```

用 CLI 生成一个：

```bash
suprnova key:generate
# Generated a new APP_KEY (AES-256, base64 URL-safe, no padding):
#
#     hQ7rW0X9_NkSi8Cw5fF8j6V_K6JzgB3y2Hq9LpL9-Wo
#
# Add it to your .env (or your secrets manager):
#
#     APP_KEY=hQ7rW0X9_NkSi8Cw5fF8j6V_K6JzgB3y2Hq9LpL9-Wo
```

或者直接用管道把它送进环境变量：

```bash
echo "APP_KEY=$(suprnova key:generate --show)" >> .env
```

### 启动时校验 - 失败关闭

`Server::from_config` 会在**每一次启动时**都校验 `APP_KEY`，不仅仅是第一次。规则如下：

| 环境 | `APP_KEY` 未设置 | `APP_KEY` 格式错误 |
|---|---|---|
| `local`、`development`、`testing` | 生成一个临时密钥，在日志里给出警告 | 硬错误 - 启动失败 |
| `staging`、`production`，以及其他任何环境 | 硬错误 - 启动失败 | 硬错误 - 启动失败 |

一个格式错误的密钥**永远**是一个硬错误，即便在 `local` 里也一样 - 让启动失败，好过掩盖一个手误。一个框架不认识的 `Custom` 环境值（比如 `APP_ENV=k8s`）会被当作生产环境类似的情形来处理：没有 `APP_KEY`，就不启动。

这条诊断信息会直接指向修复办法：

```
APP_KEY is required when APP_ENV=production. Generate one with
`suprnova key:generate` and set it in your environment (e.g. .env
or your secrets manager). Suprnova refuses to boot without an
encryption key outside of local/development/testing because session
cookies and pagination cursors would otherwise be unsigned and
forgeable.
```

## `CryptPurpose` - 通过 AAD 做域分离

每一次 `Crypt::*` 调用都会带上一个 `CryptPurpose`。这个变体会映射到一个稳定的字节标签，绑定进 AES-GCM 认证标签里，作为关联数据（AAD）：

```rust
pub enum CryptPurpose {
    Cookie,            // suprnova:cookie:v1
    Cursor,            // suprnova:cursor:v1
    TwoFactorSecret,   // suprnova:2fa:secret:v1
    TwoFactorRecovery, // suprnova:2fa:recovery:v1
    Cast,              // suprnova:cast:v1
}
```

这个标签**不会**存进传输值里。GCM 会把 AAD 混进认证标签里，而不把它包含在密文中，所以：

- 传输格式不变 - 仍然是 `base64(nonce || ciphertext || tag)`。
- 在 `CryptPurpose::Cookie` 下产出的一个传输值，会被任何提供了不同用途的解密调用**拒绝**。GCM 的标签检查会在任何解密后的解析运行之前就失败。
- 添加一个新的表面（未来的队列载荷加密、一个加密的文件头），意味着添加一个新的变体 - 不是改变传输格式。

```rust
use suprnova::{Crypt, CryptPurpose};

let wire = Crypt::encrypt_string(CryptPurpose::Cookie, "session-id")?;

// 同一把密钥，同一份传输值，不同的用途 - 会失败。
let result = Crypt::decrypt_string(CryptPurpose::Cursor, &wire);
assert!(result.is_err());

// 相同的用途 - 会成功。
let plain = Crypt::decrypt_string(CryptPurpose::Cookie, &wire)?;
```

### 为什么 Suprnova 有所不同

Laravel 的 `Crypt::encryptString` 不带用途参数。同一个 `APP_KEY`
在 cookie、签名 URL、签名的过期令牌，以及任何用户对
`Crypt::encrypt` 的调用之间被反复重用，在加密层没有任何域分离。如果两个表面碰巧都接受相同明文形状的密文，一个为某个表面铸造出来的值，就可以被重放到另一个表面上。

Suprnova 出于同样的理由重用同一个 `APP_KEY` - 运维人员只需要管理一份密钥 - 但把每个表面都绑定到了它自己的 AAD 标签上。跨表面的密文重放会在 GCM 标签检查这一步就被拒绝，早于任何解析的运行。调用方付出的代价是多传一个枚举参数；换来的收益，是一个单靠传输格式本身永远打不破的性质。

每个标签上的 `:v1` 后缀，是为将来按表面单独轮换而保留的：把
`suprnova:cookie:v1` 升到 `suprnova:cookie:v2`，**只会**让旧的
cookie 密文失效 - 游标、2FA 密钥和转换器列都不受影响。

## 两对加密 / 解密函数

有两种形态，对应两种使用场景。

### 字符串 - `encrypt_string` / `decrypt_string`

面向 UTF-8 字符串：

```rust
use suprnova::{Crypt, CryptPurpose};

let wire: String =
    Crypt::encrypt_string(CryptPurpose::Cast, "alice@example.com")?;

let plain: String =
    Crypt::decrypt_string(CryptPurpose::Cast, &wire)?;
```

解密路径返回一个 `String` - 非 UTF-8 字节（一次正常的加密运行不会产出这种东西，但一个损坏的、或者攻击者提供的传输值可能会）会以一个明确的 `FrameworkError::Internal` 的形式暴露出来。

### 任何 `Serialize` 类型 - `encrypt` / `decrypt`

面向结构化的值，一次调用完成“先 JSON 编码、再加密”：

```rust
use serde::{Serialize, Deserialize};
use suprnova::{Crypt, CryptPurpose};

#[derive(Serialize, Deserialize)]
struct Secret {
    api_key: String,
    last_rotated_at: chrono::DateTime<chrono::Utc>,
}

let value = Secret {
    api_key: "sk_live_…".into(),
    last_rotated_at: chrono::Utc::now(),
};

let wire = Crypt::encrypt(CryptPurpose::Cast, &value)?;
let round_trip: Secret = Crypt::decrypt(CryptPurpose::Cast, &wire)?;
```

传输格式是一样的 - 都是 `nonce || ciphertext || tag` 之上的
base64 - 唯一的区别是，这里的明文是 `value` 的 `serde_json` 字节，而不是一个字符串的 UTF-8 字节。把它用在任何记录形状上：一个配置数据块、一个会话载荷、一个队列参数元组。

### `appears_encrypted` - 形状检查，不是防篡改检查

对于那种需要在出站这一遍跳过已经加密过的值的中间件（匹配
Laravel `EncryptCookies` 的行为），`Crypt::appears_encrypted` 会做一次低成本的启发式检查：

```rust
if Crypt::appears_encrypted(cookie_value) {
    // 直接放过 - 已经包好了
} else {
    // 发送前先加密
}
```

当输入能被解码成 URL-safe base64、且解码后的长度至少是 28 字节（随机数 + 标签）时，它返回 `true`。它从不会调进 AES-GCM，所以它**没法**把一个合法的密文，和形状正确的随机字节区分开。需要认证的调用方，必须调用 `decrypt_string` / `decrypt`，并处理这个错误。

## 密钥轮换 - 密钥环

Suprnova 通过一个密钥*环*支持零停机的轮换：一把当前密钥（用于每一次新的加密），加上一份有序的旧密钥列表（在解密时依次作为回退尝试）。您可以滚动更新 `APP_KEY`，而不需要同步地把每一列都重新加密一遍。

把 `APP_KEY_PREVIOUS` 设置成一份逗号分隔的 base64 密钥列表，从最旧到最新：

```env
APP_KEY=<new key>
APP_KEY_PREVIOUS=<old key>
# 或者用于多步轮换（从旧到新）：
APP_KEY_PREVIOUS=<oldest>,<middle>,<previous>
```

加密**永远**用当前密钥。解密会先尝试当前密钥；如果失败，就按顺序依次尝试每一把旧密钥。命中一把旧密钥时，`Crypt` 会发出一条
`tracing::warn!`：

```
WARN previous_index=0 Crypt decrypted a value with APP_KEY_PREVIOUS[0];
re-encrypt (load + save) this row under the current APP_KEY and remove
the corresponding APP_KEY_PREVIOUS entry once the rotation completes.
```

这条日志行故意排除了明文和密文两者 - 只传递“发生了轮换”这个事实，加上一条可以采取行动的提示。运维人员搜索日志里的
`APP_KEY_PREVIOUS`，就能落到每一个仍然依赖旧密钥的列上。

### 上限 - `MAX_PREVIOUS_KEYS = 8`

`APP_KEY_PREVIOUS` 的上限是 8 个条目。一条现实的轮换链是 1 到 3
个条目（一次正在进行中的滚动更新，也许还有一次运维人员没清理掉的、卡住的旧滚动更新）；8 留出了相当宽裕的余量。超过这个上限，启动会**明确地失败**，并给出一条同时点名条目数和上限的诊断：

```
APP_KEY_PREVIOUS holds 12 keys; the maximum is 8. A realistic
rotation chain is 1-3 entries - a longer list is almost always a
config-templating accident. Trim the list to the keys still needed
for in-flight rotation; once a re-encrypt job has migrated every
row off an old key, drop that entry.
```

悄悄截断会丢掉一把运维人员可能仍然依赖的密钥，让某些列变得无法解密，却没有任何诊断信息。这个硬上限是故意设的。

空的条目是被容忍的：`APP_KEY_PREVIOUS=,,,old1,,,old2,,,` 会被解析成两把真正的密钥。一个格式错误的条目（手误、长度不对、错误的
base64）是一个硬错误 - 半轮换状态的密钥会让启动失败，而不是悄悄丢掉一个回退项。

### 轮换流程

```bash
# 1. 铸造一把新密钥。
NEW=$(suprnova key:generate --show)

# 2. 把当前密钥移到 APP_KEY_PREVIOUS，装上新的这把。
#    编辑您的 .env 或者密钥管理器：
#
#      APP_KEY_PREVIOUS=<old_value_of_APP_KEY>
#      APP_KEY=<NEW>

# 3. 部署。新的写入会用新密钥；既有的行继续
#    通过旧密钥回退来解密。日志会标出
#    哪些列仍然停留在旧密钥上。

# 4. 跑一遍重新加密。对每一个带加密转换器的模型：
#
#      User::query().chunk(500, |batch| async {
#          for mut row in batch { row.save().await?; }
#          Ok(())
#      }).await?;
#
#    `Cast::to_storage` 永远用当前密钥，所以一次
#    空操作式的“读取再保存”就能迁移这一行。

# 5. 一旦日志里不再出现警告，就去掉 APP_KEY_PREVIOUS，
#    再部署一次。
```

整个流程都是在线完成的 - 没有任何一个时间窗口，新请求会失败。

### 观察这个密钥环

面向运维仪表盘或者健康检查：

```rust
use suprnova::Crypt;

if Crypt::has_previous_keys() {
    let n = Crypt::previous_key_count();
    tracing::info!(previous_keys = n, "APP_KEY rotation in progress");
}
```

密钥的字节本身永远无法通过公开 API 访问到。`EncryptionKey` 的
`Debug` 实现打印的是 `"[REDACTED]"`，并且没有任何访问器，能把一个原始密钥暴露到这个 crate 之外。

## Eloquent 整合 - `AsEncrypted*` 转换器

应用层加密最有用的地方，是在列这个边界上。`AsEncrypted*` 这一族转换器包装了 `Crypt::encrypt_string`，让您模型上的字段在运行时保持类型化的明文，落到存储时则是密文：

```rust
use suprnova::{model, Model};
use suprnova::eloquent::casts::{
    AsEncrypted, AsEncryptedArray, AsEncryptedObject, AsEncryptedCollection,
};
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, Clone)]
pub struct ApiKey {
    pub provider: String,
    pub secret: String,
}

#[model(table = "users", casts = {
    api_token     = AsEncrypted,
    api_keys      = AsEncryptedArray<ApiKey>,
    billing       = AsEncryptedObject<BillingDetails>,
    ssh_keys      = AsEncryptedCollection<String>,
})]
pub struct User {
    pub id: i64,
    pub api_token: String,
    pub api_keys: Vec<ApiKey>,
    pub billing: BillingDetails,
    pub ssh_keys: suprnova::eloquent::Collection<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
```

| 转换器 | 运行时类型 | 存储形态 |
|---|---|---|
| `AsEncrypted` | `String` | 加密后的字符串 |
| `AsEncryptedArray<T>` | `Vec<T>` | JSON → 加密后的字符串 |
| `AsEncryptedObject<T>` | `T` | JSON → 加密后的字符串 |
| `AsEncryptedCollection<T>` | `Collection<T>` | JSON → 加密后的字符串 |

这四个全都经过 `CryptPurpose::Cast`。一个由加密转换器铸造出来的传输值，会被任何试图把它当作 cookie 或游标来解密的代码拒绝 - 即便 `APP_KEY` 是同一个，AAD 标签也是不同的。

完整的转换器表面、失败模式对照表，以及重新加密的做法，参见
[eloquent.md](eloquent.md)。加密机制和上面这个门面是一样的 - 这个转换器只是在存储边界上跑一次
`Crypt::encrypt_string(CryptPurpose::Cast, …)` 的语法糖。

### 加密对比哈希 - 选对工具

`AsEncrypted` 是**可逆的**。用 `APP_KEY` 可以把明文恢复出来。把它用在您的应用需要读回来的数据上：您在设置页面展示的 API 令牌、您转发给上游服务的第三方密钥、您用来发货的收货地址。

对于您的应用只需要*验证*的数据 - 密码、您拿来和传入令牌比对的
API 密钥前缀 - 请改用哈希。哈希是单向的：即便 `APP_KEY` 被攻破，也没有明文可以泄露。参见 [hashing.md](hashing.md) 了解 Bcrypt /
Argon2id 门面和 `AsHashed` 转换器。

## `Crypt` 在框架内部还被用在哪里

您不需要做任何事情来接入这些功能 - 一旦 `APP_KEY` 配置好了，它们就会自动接好线。

- **加密的 cookie** - `Cookie::encrypted(...)` /
  `Cookie::read_encrypted(...)` 用的是 `CryptPurpose::Cookie`。会话 cookie、记住我 cookie，以及维护模式绕行 cookie，全都搭这条路。参见 [responses.md](responses.md) 和 [session.md](session.md)。
- **游标分页** - `CursorPaginator` 会在 `CryptPurpose::Cursor` 下编码这个游标，让传输中的 `?cursor=…` 值无法被跨表面伪造或重放。参见 [eloquent.md](eloquent.md#cursor-pagination)。
- **2FA 密钥** - `two_factor_authentications.secret` 上那个加密的 base32 TOTP 密钥，用的是 `CryptPurpose::TwoFactorSecret`；恢复码用的是 `CryptPurpose::TwoFactorRecovery`。不同的用途，防止了同一行内跨列的密文重放。参见 [auth-flows.md](auth-flows.md)。
- **HMAC 派生签名** - 签名 URL 和密码重置令牌，是从 `APP_KEY`
  派生出一个 HMAC 密钥，而不是在它之下做加密。原始的密钥字节不会被导出；这个派生过程活在框架内部。参见
  [routing.md](routing.md#signed-urls)。

## 用 `Crypt` 做测试

`Crypt` 这个门面是由 `OnceLock` 支撑的，所以一个测试二进制文件里第一个安装它的调用会生效。测试辅助函数替您处理了这些样板代码：

```rust
use suprnova::testing::install_test_encryption_key;

#[tokio::test]
async fn encrypts_and_round_trips() {
    install_test_encryption_key(); // 幂等 - 可以安全地在每个测试里调用

    let wire = suprnova::Crypt::encrypt_string(
        suprnova::CryptPurpose::Cast,
        "hello",
    ).unwrap();

    let plain = suprnova::Crypt::decrypt_string(
        suprnova::CryptPurpose::Cast,
        &wire,
    ).unwrap();

    assert_eq!(plain, "hello");
}
```

这个测试密钥是一把确定性的、全零的 32 字节密钥，让密文行为在多次运行之间可以重现（随机数依然是随机的，所以密文在多次调用之间还是不同的 - 但密钥是固定的，所以任何需要跨运行比较传输值的测试，都可以在一把稳定的密钥下做到这一点）。

对于轮换测试，直接安装一个密钥环，并用 `_test_encrypt_with` 铸造历史密文：

```rust
use suprnova::testing::install_test_encryption_keyring;
use suprnova::EncryptionKey;

let current = EncryptionKey::generate();
let old = EncryptionKey::generate();

install_test_encryption_keyring(current, vec![old.clone()]);

// 模拟一个在 `old` 还是当前密钥时写入的值。
let legacy_wire = suprnova::crypto::_test_encrypt_with(
    &old,
    suprnova::CryptPurpose::Cast,
    "legacy",
).unwrap();

// 当前的密钥环会通过旧密钥回退来解密它，
// 并发出那条轮换警告日志。
let plain = suprnova::Crypt::decrypt_string(
    suprnova::CryptPurpose::Cast,
    &legacy_wire,
).unwrap();

assert_eq!(plain, "legacy");
```

当 `testing` 这个 feature 被禁用时（`default-features = false`），这两个辅助函数都不会被编译进生产二进制文件。

## 失败模式 - 错误长什么样

每一个可能失败的 `Crypt::*` 调用都返回 `Result<_, FrameworkError>`。您可能看到的五种错误：

| 原因 | 在哪里 | 表现形式 |
|---|---|---|
| `Crypt` 尚未初始化 | 启动之前的任何调用 | `FrameworkError::Internal("Crypt is not initialized - set APP_KEY before serving")` |
| 传输值不是合法的 base64 | `decrypt_string`、`decrypt` | `FrameworkError::Internal("Crypt base64 decode failed: …")` |
| 传输值太短（< 28 字节） | `decrypt_string`、`decrypt` | `FrameworkError::Internal("AEAD wire too short …")` |
| 标签检查失败 - 密钥不对、AAD 不对、字节被篡改 | `decrypt_string`、`decrypt` | `FrameworkError::Internal("AEAD decrypt failed: …")` |
| JSON 编码 / 解码失败 | `encrypt`、`decrypt` | `FrameworkError::Internal("Crypt JSON {encode,decode} failed: …")` |

没有悄悄地回退到垃圾数据这种情况。一把错误的密钥对上一段既有的密文，永远是一个硬错误，无论是在门面这一层，还是在转换器这一层。这和 Laravel 的 `Encrypter` 行为是一致的，也正是这个性质让轮换变得安全：一个被漏掉的列会立刻暴露出来，而不是返回一个看似合理、实际错误的明文。

当一把旧密钥成功解密了一个传输值时，这次调用仍然返回 `Ok(...)` - 但那条 `tracing::warn!` 会随之一起触发，所以基于日志的告警能在 `APP_KEY_PREVIOUS` 被移除之前，抓到这条轮换的尾巴。

## 下一步

- [configuration.md](configuration.md) - `APP_KEY`、`APP_ENV`，以及启动环境里的其余部分。
- [eloquent.md](eloquent.md) - `AsEncrypted*` 转换器、完整的转换器对照表，以及模型列的轮换流程。
- [hashing.md](hashing.md) - 当您需要*验证*而不是*恢复*时的单向替代方案；bcrypt 和 Argon2id 门面，加上 `AsHashed`。
- [auth-flows.md](auth-flows.md) - 2FA 密钥和恢复码的存储，它们各自在自己的用途下搭 `Crypt` 这条路。
- [session.md](session.md) - 会话 cookie，通过 `CryptPurpose::Cookie` 由 `Crypt` 加密并签名。
