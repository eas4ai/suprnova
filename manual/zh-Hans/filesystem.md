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

每一个磁盘都在启动时通过 `Storage::register_*` 注册一次，然后通过 `Storage::disk(name)` 按名字查找。这里不存在一个让其他后端劣化过去的“默认后端” - 每个驱动程序都是平级的。

| 构造函数                          | 后端                       | Feature             |
|--------------------------------------|-------------------------------|---------------------|
| `Storage::register_fs(name, root)`   | 本地文件系统              | `filesystem`        |
| `Storage::register_memory(name)`     | 进程内内存（测试用）     | `filesystem`        |
| `Storage::register_s3(name, cfg)`    | Amazon S3 或 S3 兼容服务    | `filesystem`        |
| `Storage::register_azblob(name, cfg)`| Azure Blob Storage            | `filesystem-azure`  |
| `Storage::register_gcs(name, cfg)`   | Google Cloud Storage          | `filesystem-gcs`    |
| `Storage::register_read_through(name, cfg)` | 读穿透复合磁盘 | `filesystem` |

`filesystem` 默认开启；Azure 和 GCS 这两个 feature 则不是。请在您的 `Cargo.toml` 里打开其中之一：

```toml
[dependencies]
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.3.7", features = ["filesystem-gcs"] }
```

不开这个 feature，`register_azblob` / `register_gcs` 以及它们的配置结构体就都不存在 - 您得到的是一个点名缺失项的编译错误，而不是一次运行时失败。

每一个构造函数都有一个 `_with` 变体，它会在 `suprnova::opendal::Operator` 落进注册表之前把它交给您，这样您就能在它外面装上重试/超时/日志这些层：

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

云端的那几个构造函数（`register_s3`、`register_azblob`、`register_gcs`）默认会装上一个 `RetryLayer`（3 次尝试），因为在对象存储上，短暂的限流 / 5xx 错误是家常便饭。需要完全掌控时，请用 `_with` 变体。

Suprnova 接好的 opendal 层的完整集合是：`RetryLayer`、`TimeoutLayer`、`LoggingLayer`、`TracingLayer`（当框架的 `otel` feature 开启时，通过 `tracing-opentelemetry` 桥接到 OTel），以及 `PrometheusClientLayer`（把直方图和计数器导出到一个由您持有的 `prometheus_client::registry::Registry` 里）。层的顺序很要紧 - 最外层的那个会包住它里面的一切 - 惯用的栈是 `RetryLayer → TimeoutLayer → LoggingLayer`，这样一次超时的尝试仍然会被记录，而重试则覆盖了传输失败。

用同一个名字重新注册，会替换掉之前那个 operator，并发出一条 `warn!` 日志 - 磁盘本该在启动时注册一次，而一次意外的重复注册，可能会把一个生产磁盘换成一个内存磁盘。替换仍然会发生；这条警告只是让这次替换变得能被听见。

### 为什么 Suprnova 有所不同

Laravel 的 `config/filesystems.php` 会列出每一个磁盘驱动程序，您在运行时挑一个；什么都不会被编译掉。Suprnova 把 Azure 和 GCS 挡在 feature 后面，是因为在 Rust 里这个选择带有依赖成本，而且这一次还带有安全维度：这两个 opendal 服务 crate 都会拉入 `rsa`，它携带着 [RUSTSEC-2023-0071](https://rustsec.org/advisories/RUSTSEC-2023-0071)（Marvin 计时攻击），上游还没有修复版本。把它们做成可选启用，就意味着一个把文件存在本地或 S3 上的应用，永远不会携带那个 crate。

S3 是刻意**没有**被挡在 feature 后面的 - 它的签名器从来没有依赖过 `rsa`，所以把它挡起来只会破坏用得最多的那个云后端，却什么都清除不了。

### 本地原子写入

在本地磁盘上，每一个会在某条路径上发布字节的操作，都是一步把它们发布出去的。`disk.write(...)`、`disk.writer(...)` 和 `disk.copy(...)` 全都先落在 `<root>/.suprnova-atomic/` 里，在那里被刷写并同步到磁盘上，然后再被 rename 到目标上；`disk.rename(...)` 本来就已经是单独一步了。因此一个并发的读取方看到的，要么是先前那个对象，要么是写完的那个新对象，绝不会是一个写了一半的长度；而一个写到一半就死掉的进程，会让目标原封不动，而不是在活着的那条路径上留下一个被截断的对象。

`append` 是唯一一个原地进行的操作，因为要给一次 append 做暂存，就得先把整个对象复制一遍。这一点对那次*创建出*对象的 append 同样成立，就跟对它之后的每一次 append 一样，所以两个向同一个新对象追加内容的写入方都会落地。原地进行也正是一次 append 让您付出的代价：一次失败或者被中止的 append 会把这个对象留在那里，可能是空的，也可能是没写全的，这与向一个已经存在的对象追加内容时一直以来的情形完全一样。

一次条件写入是用 `link(2)` 而不是用一次 rename 发布出去的，这让它保持为一次真正的独占创建，而不是“先检查、再覆盖”：

```rust,ignore
// 在任意多个相互竞态的调用方当中，恰好有一个会在这里拿到 Ok。其余每一个
// 都会拿到一个 `ErrorKind::ConditionNotMatch` 错误，并且什么都不写。
disk.write_with("locks/import.json", body).if_not_exists(true).await?;
```

这次发布需要一个支持硬链接的文件系统。在 FAT、exFAT 以及某些网络文件系统上，`link(2)` 是不受支持的，而一次条件写入在那里会干脆失败，而不是悄悄劣化成“先检查、再覆盖” - 那样会递给您一个并不成立的独占性保证。别的每一个操作都不受影响。

以 rename 发布会替换掉这个对象的 inode。因此一次重写不会保留原先那个文件的权限模式、属主或者硬链接，而一个持有已打开描述符的读取方，会继续读到旧内容，而不是看到新字节。这是原子发布通常要付的代价，但如果您原先依赖这两者中的任何一个，那它就是一次变更。

如果一条路径是经由一条这道防护解析不了的符号链接够到这个磁盘的 - 也就是一条悬空的链接，它的目标并不存在 - 那么它会被拒绝，而不是被当成一个可以随手创建的空闲名字。通过这样一条链接去创建，创建出来的会是这条链接的目标，位置可以是宿主机上的任何地方，所以这道防护分辨不出一条无害的悬空链接和一次逃逸，于是两者都拒绝。

`.suprnova-atomic` 这个名字在每一个本地磁盘的根目录下都是保留的。任何第一段是这个名字的路径都会被以一个权限错误拒绝，任何通过符号链接*解析*进这个目录的路径也一样，所以您没法读另一个写入方的暂存文件、没法往这个目录里写，也没法把它删掉。这个条目会从 `files`、`directories`、`all_files` 和 `all_directories` 里被过滤掉，所以它绝不会作为一个对象露面。这个名字以 `suprnova::ATOMIC_STAGING_DIR` 导出，因为备份和同步工具需要它：请像排除一个锁目录那样把这个目录排除掉。它装着正在飞行中的临时文件，加上一个发布到一半就死掉的进程留下来的那些东西，而没有任何东西会去清扫它们，所以一台处在崩溃循环里的宿主机会让它一直长大，直到有人把它清空 - 在没有任何东西正在写的时候这么做是安全的。

### 路径穿越防护

本地文件系统磁盘会在任何用户提供的层之前，先装上一个 `PathGuardLayer`。像 `disk.write("../escaped.txt", ..)` 这样的请求，在到达操作系统之前就会被拒绝 - 任何 `..` 片段或绝对路径前缀都逃不出磁盘根目录。对象存储和内存后端不会得到这道防护（在那些后端上，`../foo` 这样的键只是一串普通的键字符）。

在拒绝了 `..` 和绝对路径片段之后，这道防护会把本地磁盘根目录和请求的磁盘上目标都规范化。对已存在的目标，会解析每一个符号链接片段；对一个还不存在的路径，这道防护会向上走到最近的那个已存在的祖先并将其规范化。如果解析出来的路径落在规范根目录之外，这次操作就会被拒绝，所以一个在校验期间观察到的根目录内符号链接，没法把一次读取、写入、列举、复制或重命名重定向到磁盘之外。

这是一道“先规范化再操作”的防护，不是基于描述符的文件系统禁闭。它假定磁盘根目录及其内容在并发修改面前是可信的：一个能在校验之后、后端打开这个路径之前替换目录或符号链接的攻击者，可能会赢下一场“检查时到使用时”的竞态。当其他主体可以并发修改这棵存储树时，请使用操作系统层面的隔离，或者一个专用的文件系统。

流式的写入器、列举器和复制器，会在它们第一次后端 I/O 之前紧接着做一次这项已解析路径检查。此后校验结果在那次流式会话里就固定下来，这样每一个数据块或条目就不会卡在文件系统规范化上。复制器和写入器的中止，总是会把清理工作转发给它们的后端，即使是在激活之前，或者在校验已经无法完成的时候。

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

## 读穿透磁盘

一个读穿透磁盘把一个快的*主*磁盘和一个慢的*后备*磁盘配成一对，并在对象被读到的时候，把它们从后者搬到前者。把主磁盘指向您要迁往的那个存储、把后备磁盘指向您要迁离的那个，工作集就会在真实流量之下自己迁过去 - 不需要维护窗口，也不需要把谁都没要过的对象整批复制一遍。

```rust,ignore
use suprnova::{ReadThroughConfig, S3Config, Storage};

Storage::register_s3("new-store", S3Config { bucket: "assets-2".into(), ..Default::default() })?;
Storage::register_s3("legacy-store", S3Config { bucket: "assets-1".into(), ..Default::default() })?;

Storage::register_read_through(
    "assets",
    ReadThroughConfig {
        primary: "new-store".into(),
        fallback: "legacy-store".into(),
        ..Default::default()
    },
)?;

let assets = Storage::disk("assets")?;
// 从 `legacy-store` 读出 `logo.png`，并在出去的路上把它写进 `new-store`。
// 之后每一次读取都由 `new-store` 来应答。
let bytes = assets.read("logo.png").await?;
```

`Storage::disk("assets")` 返回的是一个普通的 `Operator`，所以它上面的每一个方法、以及每一个 `DiskExt` 便捷方法，都原封不动地照常工作。

### 哪个磁盘应答哪个操作

| 操作 | 磁盘 |
|---|---|
| `read` | 主磁盘持有这个对象时用主磁盘，否则用后备磁盘 - 而且，除非 `copy` 是 `false`，命中后备磁盘的那个对象会被提升 |
| `exists`、`size`、`last_modified`、`mime_type`、`stat` | 主磁盘持有这个对象时用主磁盘，否则用后备磁盘 |
| `write`、`make_directory` | 只用主磁盘 |
| `files`、`directories`、`list` | 只用主磁盘 - 后备磁盘上的条目对一次列举是不可见的 |
| `delete` | 两个都删，先删后备磁盘 |
| `copy`、`rename` / `move_to` | 主磁盘持有源对象时用主磁盘，否则从后备磁盘那边流式传过来；一次 `rename` 还会删掉后备磁盘上的源对象 |
| `temporary_url` | 主磁盘持有这个对象时用主磁盘，否则用后备磁盘 |
| `temporary_upload_url` | 只用主磁盘 - 一次上传必须落在写入所落的地方 |

列举只走主磁盘，这是设计如此。一次并集式的列举，将不得不在两个后端之间调和分页和排序，而且它会报告出一些对象，它们一旦被提升，后来的列举就不再返回了。当您需要列举后备磁盘上还剩下什么时，请直接用 `Storage::disk("legacy-store")`。

删除会把这个对象从两个磁盘上都移走。如果它只移走主磁盘上那一份，下一次读取又会把后备磁盘上那一份直接提升回来。由此带来的后果是：一个建在只读后备磁盘之上的读穿透磁盘没法删除：后备磁盘上的删除会失败，而这个错误会传到您手上。

### 当一次提升失败时

默认情况下，一次提升失败会以 `warn` 记进日志，然后被吞掉。您仍然会收到您要的那些字节；这个磁盘只是退化成每一次都去读后备磁盘，直到主磁盘重新可写为止。当一次悄无声息的提升丢失，会遮住一个您需要看见的故障时 - 比方说一次您正想收尾的迁移 - 请设置 `throw_on_promotion_failure: true`：

```rust,ignore
Storage::register_read_through(
    "assets",
    ReadThroughConfig {
        primary: "new-store".into(),
        fallback: "legacy-store".into(),
        throw_on_promotion_failure: true,
        ..Default::default()
    },
)?;
```

注册会拒绝一份没法工作的配置：一个空的 `primary` 或 `fallback`、一对两次点了同一个磁盘的组合、一个点了自己的磁盘，或者一个并未注册过的名字。每一种都会返回一个点名了这个问题的 `FrameworkError`，并且不会注册任何磁盘。

### 只读取，不提升

设置 `copy: false`，就能在应答命中后备磁盘的读取时，不把它们写穿过去：

```rust,ignore
Storage::register_read_through(
    "assets",
    ReadThroughConfig {
        primary: "cache-store".into(),
        fallback: "origin-store".into(),
        copy: false,
        ..Default::default()
    },
)?;
```

这个磁盘于是读起来像一层透明的覆盖层：主磁盘应答它持有的东西，后备磁盘应答其余的一切，而两者之间什么都不会移动。当主磁盘是一个您不希望被一次一次性读取填满的小缓存时，或者当后备磁盘才是权威、而主磁盘只持有您有意放上去的对象时，就用它。

这个标志管的只是读取时刻的提升，别的什么都不管。写入、删除、元数据、列举，以及 `copy` 和 `rename` 的目的地，行为都和提升开着时完全一样 - 所以一个 `copy: false` 的磁盘，仍然会把复制或移动过来的对象落在主磁盘上。因为什么都不会被写回去，所以一次 `copy: false` 的读取只会取回您所要的那个范围，而不是整个对象。

### 跨后备磁盘的复制与移动

`copy` 和 `rename` 会先对着主磁盘去解析源对象。当只有后备磁盘持有它时，这个对象会以 64 KiB 的数据块流式传过来，而目的地落在主磁盘上：

```rust,ignore
let assets = Storage::disk("assets")?;

// `logo.png` 只存在于 `legacy-store` 上。这次复制会把它流式传过来，
// 并把 `branding/logo.png` 写进 `new-store`；遗留的那个对象原地不动。
assets.copy("logo.png", "branding/logo.png").await?;

// 一次移动做同样的事，然后再删掉遗留的源对象。
assets.rename("logo.png", "branding/logo.png").await?;
```

一次移动在两条路径上都会删掉后备磁盘上的源对象 - 无论主磁盘有没有持有这个源对象。没有这一步，下一次读取就会把后备磁盘上那一份提升回来，把这次移动撤销掉。

这两条路径的区别在于什么时候删它，而这个区别，正是一次失败的移动会留下什么：

- 主磁盘持有源对象。后备磁盘上那一份先走，在这次 rename 之前。只要主磁盘持有这个路径，后备磁盘上那一份通过这个磁盘就是够不着的，所以先把它移走，并不会改变任何您能观察到的东西 - 而且如果这次删除失败了，什么都还没有移动过。重试这次移动就是了。反过来，如果这次删除成功了、而随后的 rename 失败了，那么后备磁盘上就这个路径而言什么都没有了，目的地也没有被写，主磁盘则仍然持有源对象 - 所以一次重试会走同样这条路径，再 rename 一次。这次失败付出的代价只有那份冷副本，别的什么都没有。
- 只有后备磁盘持有它。删除只能在目的地就位之后才发生，所以一次在删除上失败的移动，会留下已经写好的目的地，以及仍然在后备磁盘上的源对象。重试这次移动就是了；源对象现在在主磁盘上，所以这次重试会走第一条路径。

不管走哪条路，一次失败的移动都可以安全重试，而您最后拿到的那个目的地，就是这次移动一开始所依据的那个对象。

在流式的这条路径上，那些条件也会跟着操作一起走。`if_not_exists` 会变成一次条件写入，所以一次带防护的复制或移动，仍然会拒绝一个已经存在的目的地，而不是把它踩掉；而一次点名了源版本的复制，会从后备磁盘上取出那个版本。一次复制的 `if_match` 是唯一的例外：它是一个由后端在它自己的复制内部施加的条件，而那次调用正是这条路径做不到的，所以它会带着一个点名了这个条件的 `Unsupported` 错误被拒绝，而不是被悄悄忽略。

这就使得条件成了唯一一处、会让“哪个磁盘持有源对象”显露出来的地方。一个本地目录会宣告它支持 `copy` 和 `rename`，却两者的条件形式都不支持，所以当只有后备磁盘持有 `a` 时，`copy_with(a, b).if_not_exists(true)` 会成功（它变成了一次条件写入），而当主磁盘持有它时，则会被以 `Unsupported` 拒绝。请对着主磁盘的驱动程序去核对您需要的那个条件，而不要假定它对这个磁盘上的每一个对象都成立。

一次主磁盘本来就会拒绝的移动，会在任何东西被删掉之前就被拒绝。一个根本没有 `rename` 的主磁盘、一次落到没有条件 `rename` 的主磁盘上的带防护移动，以及一次落到一个已经存在的目的地上的带防护移动，全都会在后备磁盘上的源对象仍然在原处的情况下失败 - 一次从未发生过的移动，不该让您付出那份冷副本的代价。

如果这个流在中途失败了，写入器会被中止，而这次传输所创建出来的目的地会在错误传到您手上之前被删掉，所以一次失败的传输不会被观察成一个被截断的对象。一个本来就在那里的目的地会被原样留下 - 一次失败的复制，绝不该成为那个毁掉一个它从未写过的对象的东西。一个本地文件系统的主磁盘同样遵守这一点，因为它会把这次传输暂存在 `.suprnova-atomic/` 底下，只有在成功时才 rename；中止这个写入器会把暂存文件移走，所以一次失败的传输既不会留下一个写了一半的目的地，也不会留下一个残余的临时文件。

### 带版本的读取与条件读取

一次带着版本、或者带着 `If-Match`、`If-None-Match`、`If-Modified-Since`、`If-Unmodified-Since` 条件的读取，会带着那个条件原封不动地被传递下去，所以它给出的答案，意思就是您让它问的那个意思。这样的读取会被应答，但绝不会被提升：把一个旧版本、或者一个被校验器匹配上的响应体写进主磁盘，就等于把它当作那个活着的对象发布出去，而之后每一次朴素的读取都会拿到它。

由哪个磁盘来应答这样一次读取，还是按老规矩定的。第一次探测是一次普通的存在性检查，所以只要主磁盘持有这个路径，读穿透磁盘就会把一次带版本的或者条件的读取委托给主磁盘；只有当主磁盘没有时，它才会去够后备磁盘。

主磁盘还决定了一个读穿透磁盘到底接受这里面的哪些，因为主磁盘的读取器是先打开的。一次针对主磁盘是本地目录的读穿透磁盘的带版本读取，会在够到后备磁盘之前就被拒绝，因为一个本地目录没有版本。

### 为什么 Suprnova 有所不同

Laravel 是从一条 `config/filesystems.php` 条目构建出一个读穿透磁盘的，那条条目的 `primary` 和 `fallback` 键，既接受一个磁盘名字，也接受一份内联的驱动程序配置。Suprnova 只接受磁盘名字，因为这里的磁盘是由类型化的构造函数注册出来的，而不是用数组描述出来的 - 请先把里面那个磁盘注册好，然后点它的名字。

Laravel 的提升会在读完后备磁盘之后重新检查一遍主磁盘，这让一个并发的写入方能够胜出。Suprnova 保留了这项检查，并且把提升原子地发布出去，而 Laravel 没有这么做。在一个本地文件系统的主磁盘上，字节会被暂存到一个临时的兄弟文件里，再被 rename 到位；直接把它们写到目标上，会让一个正在长大的、写了一半的文件在整个写入期间都可见，而一个读穿透磁盘恰恰是靠那次存在性检查来给读取方选路的。在一个没有 rename 的主磁盘上 - 内存、S3、Azure Blob、GCS - 一次写入本身就已经是一次单一而不可分割的发布，所以这次提升会直接写目标，条件是这个对象尚不存在，这样两个并发的读取方就不会都去提升。

那个条件正是一次暂存式的提升不可能拥有的：暂存路径是唯一的，所以在它上面加一个不许覆盖的条件是空洞的，而目标是由一次会覆盖的 rename 发布出去的。因此，一个建在本地文件系统主磁盘上的读穿透磁盘，把它交换掉了 - 一次在这次提升的最后一次存在性检查和它的 rename 之间落到主磁盘上的写入，会被提升出来的那一份覆盖掉。在一个没有 rename 的主磁盘上，这个条件是成立的，也就不存在这样的窗口期。

那个暂存对象在它存在的那段时间里，是主磁盘上一个真实的条目，所以一次在提升进行到一半时做的列举，可能会显示出一个 `.suprnova-promote-<id>.tmp` 兄弟文件。一次读取无论是完成了、失败了还是放弃了，都会尝试把它自己的那个兄弟文件移走，而如果那次删除失败了，它会记一条警告，而不是让这次读取失败。没有任何东西会去清扫一个因删除失败、因进程崩溃，或者因一个读取 future 在提升中途被取消而留下来的兄弟文件：那些必须手工移走。

一次从后备磁盘解析出来的读取，会把这个对象一直放在内存里，直到提升的写入完成为止，因为提升需要整个对象。这适合读穿透磁盘所面向的那种分层场景。对于非常大的冷对象，请改为直接读后备磁盘，或者使用 [`copy_between_disks`](#跨磁盘流式复制)。

`copy` 为 `false` 时，Laravel 会把后备磁盘自己的流交回来；为 `true` 时，则通过 `php://temp` 做缓冲。Suprnova 则是在 `copy` 为 `false` 时，把对后备磁盘的取回收窄到所请求的那个范围，并且只在那条无论如何都需要整个对象的提升路径上做缓冲。

Laravel 跨后备磁盘的 `copy` 和 `move`，同样会把源对象通过 `php://temp` 缓冲一遍。Suprnova 改为以 64 KiB 的数据块把它流式传过去，因为后备磁盘正是那些又大又很少被碰的对象所在的地方，而且它会在把错误返回之前，先把一个写了一半的目的地删掉。还有两处差别是从 OpenDAL 那里带出来的。删除一个并不存在的路径算作成功，所以一次移动会直接清掉后备磁盘上的源对象，而不先检查它是否存在。还有，OpenDAL 在 `copy` 和 `rename` 上带有一些条件，而 Flysystem 没有对应物，所以 Suprnova 不得不决定：当源对象只在后备磁盘上时，每一个条件各自意味着什么 - `if_not_exists` 和一次复制的源版本会被遵从，而一次复制的 `if_match` 会被拒绝，而不是被丢掉。

Laravel 在两条路径上都是在移动之后才删掉后备磁盘上的源对象。当主磁盘持有源对象时，Suprnova 先删它，因为这两种顺序在一次重试之下并不相同：不管哪种顺序，源对象通过这个磁盘都是够不着的，但后删意味着，一次因为一个瞬时故障而丢掉了自己那次删除的移动，回来时会变成一次源对象如今只在后备磁盘上的移动，于是它会把后备磁盘上那份陈旧的副本，流式覆盖到第一次尝试已经正确写好的那个目的地上。
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
