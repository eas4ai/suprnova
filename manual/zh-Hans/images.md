# 图像

Suprnova 提供了一条 Laravel 形状的图像管道：在一个处理程序里把它构建起来，链上您想要的那些操作，最后用一个终结方法收尾 - 它会把字节、一个响应，或者一个已存储的文件交到您手上。

```rust
use suprnova::{Image, OutputFormat, Response, handler};

#[handler]
pub async fn thumbnail() -> Response {
    Ok(Image::from_path("storage/photos/hero.jpg")
        .cover(320, 320)
        .to_format(OutputFormat::WebP)
        .quality(80)
        .to_response()
        .await?)
}
```

那个处理程序会解码这个 JPEG，填满一个 320x320 的框，从中心裁掉溢出的部分，编码成 WebP，并返回一个带 `Content-Type: image/webp` 的 `200`。

这个子系统住在 `suprnova::media` 里，位于默认开启的 `media` feature 之后。您平常会伸手去拿的一切 - `Image`、`OutputFormat`、`ImageDriver`、`ImageConfig` - 都在 crate 根部被平铺重导出，所以 `use suprnova::Image;` 就是您要的那一行 import。这个模块名在精神上取成复数，是有意为之：将来由 OxideAV 支撑的音频和视频表面也会住在这里。

如果您正在升级，请注意：过去那个叫 `Image` 的上传校验器现在叫 `ImageFile`，这就把这个朴素的名字腾给了这个管道类型。这与 Laravel 一致 - 在那边，校验规则是 `ImageFile`，而操作图像的类型是 `Image`。校验器请参见[请求](requests.md)。

## 管道是惰性的

构造一个 `Image` 什么都不读，也什么都不解码。操作只是把自己记录下来；只有当一个终结方法运行时，源才会被打开。所以这是免费的：

```rust
use suprnova::Image;

let pipeline = Image::from_disk("uploads", "avatars/42.png").resize(64, 64);
```

到这里为止还没有任何东西碰过磁盘。`Image` 实现了 `Clone`，而一个克隆体会从它自己的源重新跑一遍这条管道，而不是共享一份结果。

有两个构造函数不得不是急切的，它们的文档里也这么说：`from_upload`（一次上传的临时文件活不过这个请求）和 `from_stream`（一个流只能被消费一次）。

## 构造

| 构造函数 | 源 | 急切？ |
|---|---|---|
| `Image::from_bytes(bytes)` | 任何满足 `Into<Bytes>` 的东西 | 否 |
| `Image::from_path(path)` | 文件系统 | 否 |
| `Image::from_disk(disk, path)` | 一个 `Storage` 磁盘 | 否 |
| `Image::from_upload(&file).await?` | 一个 `UploadedFile` | 是 |
| `Image::from_stream(stream).await?` | 一个 `Stream<Item = io::Result<Bytes>>` | 是 |

`from_stream` 会**在**收集的过程中就强制执行 `IMAGE_MAX_ALLOC_BYTES`，所以一个没有尽头的流会被就地切断，而不是等它已经把内存填满之后才被发现。

## 操作

| 方法 | 效果 |
|---|---|
| `resize(w, h)` | 精确的尺寸，忽略宽高比 |
| `resize_width(w)` / `resize_height(h)` | 只给一个维度，另一个由宽高比推出来 |
| `scale(w, h)` | 装进这个框里，保持宽高比。**从不放大** |
| `scale_width(w)` / `scale_height(h)` | 缩小到至多一个维度那么大。从不放大 |
| `crop(w, h, x, y)` | 切出一个矩形。如果它落在图像之外就报错 |
| `cover(w, h)` | 精确填满这个框，从中心裁掉溢出的部分 |
| `contain(w, h)` | 装进这个框里，保持宽高比。不做填充 |
| `rotate(degrees)` | 按任意角度顺时针旋转，画布会跟着长大以容纳下它 |
| `flip_vertically()` / `flip_horizontally()` | Laravel 的 `flip` 和 `flop` |
| `blur(amount)` | 高斯模糊，`0..=100`。`0` 是空操作 |
| `sharpen(amount)` | 非锐化掩模，`0..=100`。`0` 是空操作。`50` 是经典的强度 |
| `grayscale()` | 去饱和。拼写沿用 Laravel 的写法 |
| `to_format(format)` | 选择输出容器 |
| `quality(q)` | 编码质量，夹紧到 `1..=100`，默认 `70` |

那些说不通的取值会被夹紧，而不是被拒绝：`blur(500)` 记下的是 `100`，`quality(0)` 记下的是 `1`。一个落在图像之外的裁剪则是一个真正的错误，而不是一次夹紧，因为悄悄挪动别人的裁剪框，比直接告诉他们更糟。

`rotate` 接受任意角度。90 度的整数倍会走一条精确的、轴对齐的路径，不做重采样；除此之外都是双线性的，而且画布会长大，所以不会有像素被切掉。在输出格式带 alpha 通道时，露出来的那几个角是透明的。

## 终结方法

每一个终结方法都是 `async` 的，会消费掉这个 `Image`，并且在一个阻塞线程上运行解码、变换和编码的工作，所以它从不会把运行时拖住。源的 I/O 发生在那次跳转之前，所以一块慢磁盘从不会占住一个阻塞工作线程。

| 终结方法 | 返回 |
|---|---|
| `to_bytes()` | 编码后文件的 `Vec<u8>` |
| `to_response()` | 一个带着正确 `Content-Type` 的 `HttpResponse` |
| `save(path)` | 写到文件系统 |
| `store(disk, path)` | 写到一个 `Storage` 磁盘 |
| `dimensions()` | **处理后**图像的 `(width, height)` |
| `mime_type()` | **处理后**图像的媒体类型 |
| `dominant_color()` | 平均颜色，形如 `#rrggbb` |

`dimensions()`、`mime_type()` 和 `dominant_color()` 描述的全都是成品图像，而不是源 - 和 Laravel 的契约一样。哪怕只是问一下 mime 类型，也仍然会把这条管道跑完，因为为一张根本产不出来的图像报出一个类型，是一句调用方只会在更晚的时候才发现的谎话。

```rust
use suprnova::{FrameworkError, Image, OutputFormat};

async fn describe() -> Result<(), FrameworkError> {
    let banner = Image::from_path("hero.png").resize(1200, 400);

    // 读到的是 (1200, 400)，而不是源的尺寸。
    let (width, height) = banner.clone().dimensions().await?;
    println!("{width}x{height}");

    let accent = banner.to_format(OutputFormat::Jpeg).dominant_color().await?;
    println!("{accent}");

    Ok(())
}
```

## 格式

今天有五种格式可读也可写：**PNG、JPEG、WebP、GIF 和 BMP**。

| 格式 | 可读 | 可写 | 质量旋钮 |
|---|---|---|---|
| PNG | 是 | 是 | 忽略（无损） |
| JPEG | 是 | 是 | 生效 |
| WebP | 是 | 是（无损） | 今天没有效果 |
| GIF | 是 | 是 | 忽略（调色板） |
| BMP | 是 | 是 | 忽略（无损） |

AVIF 目前既读不了也写不了。它依赖的那个自研 AV1 编码器还没有发布，而发布一个总是失败的 `OutputFormat::Avif`，会是一个框架兑现不了的承诺。它会随着那次发布一起到来，形式是一个新的枚举变体，除此之外别无他物。

GIF 输出在编码之前会被调色板量化到至多 256 色，并带上 Floyd-Steinberg 抖动，所以一张摄影源图会干干净净地转换过去，而不是报错。

WebP 是以无损方式写出的，所以 `quality()` 目前对 WebP 输出没有效果。当您需要一个体积/质量的旋钮时，请用 JPEG。

## 存储

`from_disk` 和 `store` 对任何一个已注册的 `Storage` 磁盘都能用，所以一次“缩放再存回去”的往返，从不需要碰本地路径：

```rust
use suprnova::{FrameworkError, Image};

async fn make_web_copy() -> Result<(), FrameworkError> {
    Image::from_disk("uploads", "originals/42.png")
        .scale(1024, 1024)
        .store("uploads", "web/42.png")
        .await
}
```

注册磁盘请参见[文件存储](filesystem.md)。

## 解码限制

解码正是敌意输入下手的地方：几 KB 的数据就能声明一张 40000x40000 的画布，要求一台服务器为它分配六个 GB。Suprnova 会在分配任何东西之前就拒绝掉它。

| 变量 | 默认值 | 用途 |
|---|---|---|
| `IMAGE_MAX_DIMENSION` | `16384` | 宽和高的像素上限 |
| `IMAGE_MAX_ALLOC_BYTES` | `268435456`（256 MiB） | 解码后 RGBA 占用的上限，以及源文件自身大小的上限 |
| `IMAGE_MAGICK_TIMEOUT_SECS` | `30` | 一次 ImageMagick 调用的挂钟时间上限（仅 `magick` 驱动程序） |

框架会解析输入自己的文件头 - 几十个字节，不做分配 - 读出声明的尺寸，并在一个解码器被构造出来之前就拒绝掉超大的输入。同一组上限也适用于缩放目标，因为不管这些数字来自攻击者还是一个笔误，`resize(50_000, 50_000)` 要分配的内存都一样多。

命中一个限制会是一个 4xx 形状的 `FrameworkError::param`，因为超大的输入是客户端的问题，不是服务端的过错。

超出范围的配置会带一条警告被夹紧，而不是让启动失败：`IMAGE_MAX_DIMENSION=0` 会拒绝掉这个应用里的每一张图像，那不会是任何人本来想配置出来的东西。

### 有一个界限是不可配置的

一个 WebP 会把它真正的解码尺寸声明在最内层的比特流 chunk 里，而不是画布头里，所以框架会走一遍这个容器去把它找出来。这次遍历在**每层 4096 个 chunk** 之后就停下，并且只往下跟进**两层嵌套**，超出其中任何一条的文件都会被直接拒绝，而不是被测量。

它是被拒绝而不是被测量，这是有意的。从一次没有走到文件末尾的遍历里报出一个数字，会造就一道只要堆上足够多的填充 chunk 就能绕过去的关卡，所以一次走不完的遍历没有答案可给。

这两个数字都不可调，也没有任何 `IMAGE_MAX_*` 变量会影响它们 - 错误信息里就是这么说的，而不是说“已配置”，正是为了不让任何人花一个下午去调高 `IMAGE_MAX_ALLOC_BYTES`，然后眼睁睁看着什么都没变。实际上只有刻意敌意的文件才会接近它：一段 300 帧的动画能轻松通过，一段 4100 帧的则不能。

## 后端

和 Laravel 一样，图像这块表面是两个驱动程序，用 `IMAGE_DRIVER` 来选。

| 驱动程序 | 值 | 需要 | 可读 |
|---|---|---|---|
| OxideAV | `oxideav`（默认） | 什么都不需要 | PNG、JPEG、WebP、GIF、BMP |
| ImageMagick | `magick` | 宿主上的 ImageMagick 7 | 宿主的委托提供什么就读什么 |

### `IMAGE_DRIVER=oxideav`

默认值。纯 Rust，构建在 [OxideAV](https://github.com/OxideAV) 这个编解码器家族之上：没有原生库，没有东西要装，也没有东西要配。对几乎每一个应用来说它都是正确的选择，也是一个由脚手架生成出来的应用会拿到的东西。

### `IMAGE_DRIVER=magick`

需要主动选用。它会运行一个宿主上安装好的 ImageMagick 7 二进制文件，把图像经由 stdin 灌进去，再经由 stdout 把结果读回来 - 不用临时文件。二进制文件的名字来自 `IMAGE_MAGICK_BINARY`，默认是 `magick`；二进制文件缺失会在第一次使用时给出一个明确的错误，而不是一次悄无声息的回退。

当您需要那个纯 Rust 驱动程序不携带的输入格式时，就选它 - HEIC 是最常见的那一个。代价是一项宿主依赖：运营者安装 ImageMagick 和它的委托，并对它们的许可负责。无论选哪一边，框架都不链接、也不编译任何原生代码。

参数永远是一个固定的数组，直接递给这个进程，绝不是一个 shell 字符串，而且每一个数值参数都是从一个已经校验过的字段格式化出来的。没有任何一个参数位置是用户输入能够到达的。

当框架认得出这个输入时，解码器的名字会写在命令行上 - 是 `png:-` 而不是一个光秃秃的 `-`。这很要紧：给它一个光秃秃的 `-`，ImageMagick 就会从递给它的那些字节里挑一个编解码器，于是一个魔数说自己是 MVG 或 MSL 的文件，就会被当成一个**脚本**来读，不管您的应用以为自己在接受的是什么。把编解码器钉死，会让一个贴错标签的文件失败，而不是变成别的东西。

**框架叫不出名字的输入，依然依赖您的 `policy.xml`。** 能读这些格式正是这个驱动程序存在的全部理由，所以那条路径没法钉死一个编解码器。如果您在 `IMAGE_DRIVER=magick` 下接受任意上传，请加固宿主的 ImageMagick 策略 - 至少要禁用 `MVG`、`MSL`、`URL`、`HTTPS`、`EPHEMERAL` 和 `TEXT` 这几个编解码器。

在这个驱动程序下，解码限制会被强制执行两次。对于框架能解析的那五种格式，上面那道文件头检查会在进程被拉起之前运行。对于其他一切，预解析是不可能的，所以每一次调用都会带上从同一份配置推导出来的、ImageMagick 自己的 `-limit` 标志，其中包括一个挂钟时间的 `-limit time`。

那个标志不是故事的全部，因为 ImageMagick 是用它自己的资源监视器来执行它的，而一个在那个监视器介入之前就卡死在某个委托里的进程，永远不会碰到它。所以 Suprnova 还持有它自己的截止时间：过了 `IMAGE_MAGICK_TIMEOUT_SECS`（外加几秒宽限，好让 IM 自己的限制先开火）之后，它会杀掉整个进程组 - 委托也包括在内，而不只是它启动的那个进程 - 并停止在那些管道上等待。因此一个卡住的委托没法钉住一个工作线程。留在进程组里的委托会跟着一起死；一个离开了进程组的委托，或者一台没有 `kill` 二进制文件的宿主，可能活得比这个请求更久 - 那点残留正是宿主的进程监管要负责的东西。

一次击杀会以一个 5xx 的 `FrameworkError::internal` 浮现，而不是 4xx，哪怕它是被一个请求触发的。有东西把图像这条路径卡到了需要动手杀掉的程度，这属于服务端错误监控的范畴，运营者会在那里看到它 - 把它归类成客户端错误，等于把这里唯一值得呼叫值班的状况给归档了事。

## 自定义驱动程序

`ImageDriver` 就是那个扩展点：`&[u8]` 进，`Vec<u8>` 出，没有任何编解码器类型跨过这条边界。

```rust
use suprnova::{FrameworkError, ImageDriver, ImagePipeline};

struct MyDriver;

impl ImageDriver for MyDriver {
    fn process(
        &self,
        contents: &[u8],
        pipeline: &ImagePipeline,
    ) -> Result<Vec<u8>, FrameworkError> {
        // 解码 `contents`，重放 `pipeline.transformations`，然后按
        // `pipeline.quality` 编码成 `pipeline.format`。
        todo!()
    }

    fn dimensions(&self, contents: &[u8]) -> Result<(u32, u32), FrameworkError> {
        todo!()
    }

    fn dominant_color(&self, contents: &[u8]) -> Result<String, FrameworkError> {
        todo!()
    }

    fn name(&self) -> &'static str {
        "mine"
    }
}
```

在 `bootstrap()` 期间安装它，赶在第一张图像被处理之前：

```rust
use suprnova::FrameworkError;

pub fn register() -> Result<(), FrameworkError> {
    suprnova::media::set_default_driver(Box::new(MyDriver))
}
```

一个合规的驱动程序，会在为一次解码分配内存之前，就强制执行已配置的 `ImageConfig` 限制。框架没法代替一个驱动程序去做这件事，因为它从来见不到解码后的缓冲区。

### 触及更多的格式

如果内置的这五种还不够，有三条路，大致按您要承担多少来排：

1. **内置的 `magick` 驱动程序。** 设 `IMAGE_DRIVER=magick`。格式的广度来自宿主的 ImageMagick 委托，而且没有构建依赖要打理。
2. **一个围着 libvips 的自定义驱动程序**，例如经由 [libvips-rust-bindings](https://github.com/olxgroup-oss/libvips-rust-bindings) 这个 crate（MIT）。libvips 是 Node 的 `sharp` 背后的那个引擎，格式覆盖面非常宽 - JPEG、JPEG XL、TIFF、PNG、WebP、HEIC、AVIF、PDF、SVG、GIF 等等，外加对 ImageMagick 的委派 - 并且流式性能很强。它绑定的是 libvips 这个 C 库，所以您的应用在构建期和运行期都要装上 libvips，并且要对那项依赖负责，而这恰恰就是它属于 trait 之后、而不属于框架之内的原因。一条实用的说明：这个绑定的 `VipsImage` 不是线程安全的，而“一次 `process()` 调用处理一张图像”的驱动程序形状已经容纳了这一点。
3. **任何 CLI 工具**，按 `magick` 驱动程序那样包起来：一个固定的参数数组递给 `std::process::Command`，图像字节经 stdin 进、经 stdout 出，绝不用 shell 字符串。

Suprnova 背书的是这条 trait 边界，而不是它背后任何特定的依赖。坐在那后面的是什么，由您决定，它的许可也一样。

## 测试

这个子系统不需要磁盘上的任何 fixture - 一旦解码和编码能往返，它自己就是自己的 fixture 工厂：

```rust
use suprnova::{FrameworkError, Image, OutputFormat};

/// 把一个 1x1 的字节字面量 fixture 长成一个测试所需要的任何尺寸。
async fn fixture(source: &[u8]) -> Result<Vec<u8>, FrameworkError> {
    Image::from_bytes(source.to_vec())
        .resize(4, 2)
        .to_format(OutputFormat::Png)
        .to_bytes()
        .await
}
```

那些会收紧解码限制的测试必须串行化：这些限制是进程全局的，所以一个并行的兄弟测试会在被收紧的上限之下解码。

### 为什么 Suprnova 有所不同

**默认驱动程序里没有 HEIC，原因是专利。** HEIC 里面的那个编解码器 HEVC 是带专利负担的 - Access Advance 专利池就是其中之一。Suprnova 不安装任何原生库，所以一个内置的解码器就只能是纯 Rust 的，也就会直接背上那份风险敞口；而唯一一个可信的纯 Rust 解码器是 AGPL-3.0 / 商业双许可的，那是一项逐应用的法律义务，而不是一个 MIT 框架有资格把任何人默认卷进去的东西。

两个框架都把 HEIC 当成一件宿主供给的事情；Suprnova 这个版本只是少了一个活动部件。Laravel 的默认驱动程序 GD 根本读不了 HEIC，而它的 Imagick 路径需要把 libheif 委托编译进系统的 ImageMagick 二进制文件**和** PHP 的 `imagick` 扩展这**两边**。在 Suprnova 里，默认驱动程序不读 HEIC，而只要宿主的 ImageMagick 带着 libheif 委托，`IMAGE_DRIVER=magick` 就能读它 - 中间没有扩展这一层。所以 HEIC 的接入今天就能用：通过您的包管理器装上带 libheif 的 ImageMagick，再把那个环境变量翻过去。许可落在它该落的地方，也就是宿主那边。

当 `oxideav` 驱动程序遇到一个 HEIC 文件时，它会点名说出来，指向本章，并把两条出路都说清楚，而不是返回一个泛泛的“不支持的格式”。

**AVIF 是待定，不是跳过。** 它是免版税的，也是我们想要的那个现代格式答案；只是那个自研的 AV1 编码器还没有发布而已。在此期间，WebP 就是那条通往现代格式的路。

**没有 base64 或 URL 构造函数。** Laravel 的 `ImageManager` 有 `->read($base64)` 和 `->read($url)`。`from_bytes` 能和产出这些字节的任何东西组合，包括 [HTTP 客户端](http-client.md)；而把一次 URL 抓取挡在图像子系统之外，能让它的超时、重试和 SSRF 策略待在一个地方，而不是两个。

**`from_stream` 是急切的，并且带一个上限。** Laravel 的内容是一个惰性闭包。一个流没法重放，所以这一个会在构造时就被排空，一边走一边把字节数记到 `IMAGE_MAX_ALLOC_BYTES` 头上。

**`contain` 不做填充。** 它把图像装进这个框里就到此为止；它不会把图像铺到一块背景上去做黑边。如果您需要一块背景，请自己把它组合上去。

**缩放用的是双线性重采样。** 后端的滤波器集合提供了最近邻和双线性；对自然图像来说，双线性是它文档里写明的默认值。

**图像永远不可序列化。** Laravel 在 `__serialize` 上抛异常，而 Suprnova 干脆就不实现它。请存下路径或者磁盘上的键，然后把这条管道重建出来。

## 下一步

- [文件存储](filesystem.md) - `from_disk` 和 `store` 读写的那些磁盘。
- [HTTP 响应](responses.md) - `to_response()` 交回来的是什么。
- [环境变量](env-vars.md) - 图像设置的完整列表。
