# 文件系统和存储

Suprnova 的存储门面在本地文件系统、内存后端，以及主流的对象存储（S3、Azure Blob、Google Cloud Storage）之上，提供了一套单一的具名磁盘 API。在底层，它构建于 [`opendal`](https://docs.rs/opendal) 之上 - 但对外的接口经过塑形，以匹配 Laravel 的 `Storage::disk(...)` 调用，因此 PHP 的肌肉记忆能直接平移过来。

```rust,no_run
use suprnova::{DiskExt, Storage};

# async fn doc() -> Result<(), suprnova::FrameworkError> {
Storage::register_fs("local", "./storage")?;
let disk = Storage::disk("local")?;

disk.put("notes/hello.txt", b"hello world".to_vec()).await?;
let bytes = disk.get("notes/hello.txt").await?;
assert_eq!(bytes, b"hello world");
# Ok(())
# }
```

## 注册磁盘

每一个磁盘都在启动时通过 `Storage::register_*` 注册一次，并通过 `Storage::disk(name)` 按名字查找。这里没有一个供其它后端退化成的“默认后端” - 每个驱动程序都是平等的一员。

| 构造函数 | 后端 | Cargo feature |
|--------------------------------------|-------------------------------|---------------------|
| `Storage::register_fs(name, root)`   | 本地文件系统              | `filesystem`        |
| `Storage::register_memory(name)`     | 进程内内存（测试用）     | `filesystem`        |
| `Storage::register_s3(name, cfg)`    | Amazon S3 或兼容 S3 的服务    | `filesystem`        |
| `Storage::register_azblob(name, cfg)`| Azure Blob 存储            | `filesystem-azure`  |
| `Storage::register_gcs(name, cfg)`   | Google Cloud 存储          | `filesystem-gcs`    |

`filesystem` 默认是开启的；Azure 和 GCS 这两个 feature 不是。请在您的 `Cargo.toml` 里开启其中一个：

```toml
[dependencies]
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.2.0", features = ["filesystem-gcs"] }
```

没有这个 feature，`register_azblob` / `register_gcs` 以及它们的配置结构体根本不存在 - 您得到的是一个指名缺失项的编译错误，而不是一次运行时失败。

每一个构造函数都有一个 `_with` 变体，会在这个 `suprnova::opendal::Operator` 落入注册表之前，把它交到您手上，这样您就可以在它周围安装重试 / 超时 / 日志层：

```rust,ignore
use std::time::Duration;
use suprnova::opendal::layers::{LoggingLayer, RetryLayer, TimeoutLayer};
use suprnova::Storage;

Storage::register_fs_with("local", "./storage", |op| {
    op.layer(RetryLayer::new().with_max_times(3))
      .layer(TimeoutLayer::new().with_timeout(Duration::from_secs(30)))
      .layer(LoggingLayer::default())
})?;
```

云端构造函数（`register_s3`、`register_azblob`、`register_gcs`）默认会应用一个 `RetryLayer`（3 次尝试），因为瞬时的限流 / 5xx 错误在对象存储上是司空见惯的。当您需要完全的控制权时，请使用 `_with` 变体。

Suprnova 接好的完整一套 opendal 层是 `RetryLayer`、`TimeoutLayer`、`LoggingLayer`、`TracingLayer`（当框架的 `otel` feature 开启时，通过 `tracing-opentelemetry` 桥接到 OTel），以及 `PrometheusClientLayer`（把直方图和计数器导出到您自己拥有的一个 `prometheus_client::registry::Registry` 里）。层的顺序很重要 - 最外层的层会包住它内部的一切 - 而地道的堆叠方式是 `RetryLayer → TimeoutLayer → LoggingLayer`，这样一次超时的尝试仍然会被记入日志，而一次重试能覆盖传输层面的失败。

用同一个名字重新注册，会替换掉之前的那个 operator，并发出一条 `warn!` 日志 - 磁盘本应只在启动时注册一次，一次意外的重复注册可能会把一个生产磁盘换成一个内存磁盘。这次替换依然会发生；这条警告只是让这次替换变得醒目。

### 为什么 Suprnova 有所不同

Laravel 的 `config/filesystems.php` 列出了每一种磁盘驱动程序，您在运行时挑一个；没有任何东西被编译剔除。Suprnova 把 Azure 和 GCS 挡在 feature 后面，因为在 Rust 里这个选择带着依赖成本，而这一个还带着一个安全维度：这两个 opendal 服务 crate 都会拉进 `rsa`，它带着 [RUSTSEC-2023-0071](https://rustsec.org/advisories/RUSTSEC-2023-0071)（Marvin 时序攻击），且上游没有已修复的发布版本。把它们做成可选启用，意味着一个把文件存在本地或者 S3 上的应用永远不会带上这个 crate。

S3 是故意*没有*被挡住的 - 它的签名器从来不依赖 `rsa`，所以把它挡住只会破坏最常用的那个云后端，却什么都没有省下来。

### 路径遍历防护

本地文件系统磁盘会在任何用户提供的层之前，应用一个 `PathGuardLayer`。像 `disk.write("../escaped.txt", ..)` 这样的请求，会在到达操作系统之前就被拒绝 - 没有任何 `..` 组成部分或绝对路径前缀能逃出这个磁盘的根目录。对象存储和内存后端不会获得这层防护（在这些后端上，像 `../foo` 这样的键，只是一个普通的键字符）。

在拒绝了 `..` 和绝对路径这两类组成部分之后，这层防护会对本地磁盘的根目录，以及请求所指向的磁盘内目标，做规范化。已经存在的目标会解析每一个符号链接组成部分；对于一个尚不存在的路径，这层防护会一路向上，规范化到最近的一个已存在的祖先目录。如果解析出来的路径落在规范化后的根目录之外，这次操作就会被拒绝，所以一个在校验期间被观察到、身处根目录内的符号链接，无法把一次读取、写入、列出、复制或重命名重定向到磁盘之外。

这是一种“先规范化、再操作”的防护，不是基于描述符的文件系统封闭。它假定磁盘根目录及其内容，在面对并发修改时是可信的：一个能在校验之后、但在后端打开这个路径之前替换目录或符号链接的攻击者，可能会赢下一次“检查时刻 / 使用时刻”的竞态。当其他主体可能并发修改这棵存储树时，请使用操作系统级别的隔离，或者一个专用的文件系统。

流式的写入器、列出器和复制器只会在第一次触达后端 I/O 之前，执行一次这个已解析路径的检查。此后，这次校验就对那个流会话固定下来了，所以每一个分块或条目都不需要在文件系统规范化上阻塞。复制器和写入器的中止，总是会把清理工作转交给它们的后端 - 即便是在激活之前，或者校验已经无法完成的时候。

## Laravel 形状的磁盘接口

`Storage::disk(name)` 直接返回一个 `suprnova::opendal::Operator`，这样您就可以用上它完整的流式接口（`writer`、`reader`、`presign_read`、`list`、`stat`，……）。在此之上，[`DiskExt`] 这个 trait - 在 `Operator` 上有一份覆盖实现，并重导出为 `suprnova::DiskExt` - 加上了您在 Laravel 里会通过 `Storage::disk('local')->...` 伸手去拿的每一个便捷方法。

用 `use suprnova::DiskExt;` 把它引入作用域。

### 存在性检查

```rust,ignore
disk.exists("a.txt").await?;        // 原始 opendal 方法
disk.missing("a.txt").await?;       // 取反
disk.file_exists("a.txt").await?;   // 只匹配文件（不是目录）
disk.file_missing("a.txt").await?;
disk.directory_exists("dir/").await?;
disk.directory_missing("dir/").await?;
```

### 读取与写入

| Laravel 名称 | Rust 原生对应 | 说明 |
|--------------|------------------------|------|
| `get(path)`  | `read(path)`           | `get` 返回 `Vec<u8>`；`read` 返回 opendal 的 `Buffer`。 |
| `put(path, contents)` | `write(path, contents)` | 两者都接受任何 `Into<Bytes>`。 |
| `json::<T>(path)` | - | 通过 serde_json 读取并反序列化。 |
| `put_json(path, &value)` | - | 通过 serde_json 美化打印。 |
| `prepend(path, data)` | - | 用 `\n` 拼接。要自定义拼接方式，请用 `prepend_with_separator`。 |
| `append(path, data)`  | - | 用 `\n` 拼接。要自定义拼接方式，请用 `append_with_separator`。 |

如果文件还不存在，`prepend` 和 `append` 会创建它，所以把它们当作对一个日志文件的第一次写入是安全的。

### 元数据

```rust,ignore
let bytes  = disk.size("a.bin").await?;          // u64
let when   = disk.last_modified("a.bin").await?; // Option<DateTime<Utc>>
let mime   = disk.mime_type("a.bin").await?;     // Option<String>
let digest = disk.checksum("a.bin", ChecksumAlgorithm::Sha256).await?;
```

`mime_type` 首先会去问后端 - S3、Azure 和 GCS 会把存储的那个 `Content-Type` 原样传回来。如果后端没有这个信息，它会通过 `infer` 这个 crate 嗅探前 16 KiB。`Ok(None)` 是留给那些无法识别的二进制数据块的。

`checksum` 通过 [`ChecksumAlgorithm`] 支持 `Md5`、`Sha1` 和 `Sha256`。收录 MD5 和 SHA-1，是为了和 Laravel 以及对象存储的 ETag 对等；对任何新的完整性检查，请选择 SHA-256。

### 列出内容

```rust,ignore
let files = disk.files("docs", false).await?;     // 顶层文件
let all   = disk.all_files("docs").await?;        // 递归
let dirs  = disk.directories("docs", false).await?;
let all   = disk.all_directories("docs").await?;
```

这四个方法都返回已排序的 `Vec<String>`，这样调用方就能在各个后端之间依赖一份稳定的顺序。目录会从 `files` 里被过滤掉，反过来也一样。目录路径在返回时**不带**结尾的斜杠（`"docs/sub"`），以匹配 Laravel 的 `Storage::directories()` 输出 - opendal 底层的 `list` 报告的是 `"docs/sub/"`，但为了对等，我们会把这个斜杠去掉。

### 修改目录和文件

| Laravel 名称           | opendal 原生方法        |
|------------------------|-----------------------|
| `make_directory(path)` | `create_dir(path)`    |
| `delete_directory(p)`  | `delete_with(p).recursive(true)` |
| `move_to(from, to)`    | `rename(from, to)`    |

如果后端不支持 rename，`move_to` 会回退到 `copy + delete`；如果连 copy 也不支持，就再回退到 `read + write + delete` - 所以它无论是对着测试里用的那个内存驱动程序，还是对着生产后端，都同样能用。

### 预签名 URL

```rust,ignore
let read_url   = disk.temporary_url("uploads/a.pdf", Duration::from_secs(900)).await?;
let upload_url = disk.temporary_upload_url("uploads/new.pdf", Duration::from_secs(900)).await?;
```

为了和 Laravel 对等，`temporary_url` 和 `temporary_upload_url` 把这个 URL 作为 `String` 返回。它们背后是 `Operator::presign_read` / `presign_write`，所以在那些没有实现预签名的后端上，它们会带着一条 `Unsupported` 消息报错（内存驱动程序和本地文件系统驱动程序就属于这一类；S3、Azure Blob 和 GCS 都支持它）。

## 跨磁盘流式复制

`copy_between_disks(src, src_path, dest, dest_path)` 会以 64 KiB 为一块，把源对象流式地送进目标 - 不管这对后端是什么组合。源和目标背后可以是*任何* opendal 驱动程序 - 本地文件系统到 S3、S3 到 Azure Blob、内存到 GCS，等等。

```rust,ignore
use suprnova::filesystem::streaming::copy_between_disks;

Storage::register_fs("local", "./storage")?;
Storage::register_memory("scratch");
let bytes = copy_between_disks("local", "uploads/big.bin", "scratch", "big.bin").await?;
```

如果复制过程中任何一步失败，这个不完整的目标对象会在原始错误往外传播之前被中止并删除 - 一次失败的复制永远不会表现为一个被截断的目标文件。

## 注册表维护

```rust,ignore
let removed = Storage::forget("local");  // bool：它之前是否存在？
Storage::purge();                        // 丢弃每一个磁盘
let names = Storage::disks();            // Vec<String>，已排序
```

这些方法对应着 Laravel 的 `FilesystemManager::forgetDisk` / `purge`，在配置重载和管理后台里很有用。它们并不是只给测试用的：生产代码偶尔也需要在运行时丢弃并重新注册一个磁盘（例如在一次密钥轮换之后）。

## 测试

`Storage::fake()` 返回一个守卫，它会：

1. 获取一个进程全局的 mutex，这样并发的 `#[tokio::test]` 用例就不会在共享的注册表上产生竞态；并且
2. 在构造和 drop 时都重置这个注册表，让接下来无论运行哪个测试，这个套件都处在一个干净的状态。

为了方便起见，一个 `"default"` 内存磁盘会被预先注册好。

```rust,ignore
use suprnova::filesystem::testing::DiskAssertExt;
use suprnova::{DiskExt, Storage};

#[tokio::test]
async fn stores_and_asserts() {
    let _guard = Storage::fake();
    Storage::register_memory("uploads");
    let disk = Storage::disk("uploads").unwrap();

    disk.put("a.txt", b"hello".to_vec()).await.unwrap();

    disk.assert_exists("a.txt").await;
    disk.assert_contents("a.txt", b"hello").await;
    disk.assert_missing("not-here.txt").await;
    disk.assert_count("", 1, false).await;
    disk.assert_directory_empty("docs/").await;
}
```

这五个断言辅助函数 - `assert_exists`、`assert_contents`、`assert_missing`、`assert_count`、`assert_directory_empty` - 是通过 [`DiskAssertExt`] 这个 trait 暴露出来的，并被 `#[cfg(any(test, feature = "testing"))]` 挡住，这样生产代码就没法伸手去拿它们。

## 对等速查表

| Laravel `Storage::disk(...)->...`     | Suprnova                                                 |
|---------------------------------------|------------------------------------------------------------|
| `exists($path)`                       | `disk.exists(path)`                                      |
| `missing($path)`                      | `disk.missing(path)`                                     |
| `fileExists($path)` / `fileMissing`   | `disk.file_exists(path)` / `file_missing(path)`          |
| `directoryExists($p)` / `directoryMissing` | `disk.directory_exists(p)` / `directory_missing(p)` |
| `get($path)`                          | `disk.get(path)` (`Vec<u8>`)                             |
| `json($path)`                         | `disk.json::<T>(path)`                                   |
| `put($path, $contents)`               | `disk.put(path, bytes)`                                  |
| `prepend($path, $data)`               | `disk.prepend(path, data)`                               |
| `append($path, $data)`                | `disk.append(path, data)`                                |
| `size($path)`                         | `disk.size(path)`                                        |
| `lastModified($path)`                 | `disk.last_modified(path)`                               |
| `mimeType($path)`                     | `disk.mime_type(path)`                                   |
| `checksum($path, ['checksum_algo' => 'sha256'])` | `disk.checksum(path, ChecksumAlgorithm::Sha256)` |
| `files($dir, $recursive)`             | `disk.files(dir, recursive)`                             |
| `allFiles($dir)`                      | `disk.all_files(dir)`                                    |
| `directories($dir, $recursive)`       | `disk.directories(dir, recursive)`                       |
| `allDirectories($dir)`                | `disk.all_directories(dir)`                              |
| `makeDirectory($path)`                | `disk.make_directory(path)`                              |
| `deleteDirectory($path)`              | `disk.delete_directory(path)`                            |
| `move($from, $to)`                    | `disk.move_to(from, to)`（或者 opendal 原生的 `rename`）    |
| `copy($from, $to)`                    | `disk.copy(from, to)`（opendal 原生）                   |
| `delete($path)`                       | `disk.delete(path)`（opendal 原生）                     |
| `temporaryUrl($path, $expiry)`        | `disk.temporary_url(path, expire)`（或者 opendal 原生的 `presign_read`） |
| `temporaryUploadUrl($path, $expiry)`  | `disk.temporary_upload_url(path, expire)`（或者 opendal 原生的 `presign_write`） |
| `Storage::fake()`                     | `Storage::fake()`                                        |
| `Storage::disk()->assertExists()`     | `disk.assert_exists(path).await`                         |
| `FilesystemManager::forgetDisk($n)`   | `Storage::forget(name)`                                  |
| `FilesystemManager::purge()`          | `Storage::purge()`                                       |

## 配置

存储配置完全活在 Rust 代码里，不在 `.env` 里。磁盘是在 `bootstrap()` 里通过 `Storage::register_*` 按名字注册的，并在调用处按名字寻址（`Storage::disk("public")`）。这里没有一个框架会去读取的 `FILESYSTEM_DISK` 环境变量，也没有隐含的默认磁盘 - 每个驱动程序都是平等的一员。应用自己决定某一次上传或下载的目标是哪个磁盘名字，并把所选驱动程序需要的任何 URL / 密钥 / 凭据，作为它们自己的环境变量传进去。

关于框架从环境里读取什么、又在哪些地方期望代码侧注册这条更宽的规则，请参见[配置](configuration.md)。

## 下一步

- [配置](configuration.md) - 框架从 `.env` 里读取什么（以及为什么存储不在这份清单上）
- [请求](requests.md) - 文件上传是如何通过 `UploadedFile::store_as` 落到一个磁盘上的
- [响应](responses.md) - 如何把字节从一个磁盘流式地送出去
- [缓存](cache.md) - 另一个具名驱动程序注册表，形状相同
- [测试](testing.md) - 那套更宽的“伪造一切”测试接口

[`DiskExt`]: https://docs.rs/suprnova/latest/suprnova/trait.DiskExt.html
[`DiskAssertExt`]: https://docs.rs/suprnova/latest/suprnova/filesystem/testing/trait.DiskAssertExt.html
[`ChecksumAlgorithm`]: https://docs.rs/suprnova/latest/suprnova/enum.ChecksumAlgorithm.html
