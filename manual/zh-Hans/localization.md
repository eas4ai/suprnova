# 本地化

Suprnova 里的本地化是一个模块，有四张脸：服务器上的消息语料表、已经翻译好才抵达的验证错误、发给浏览器的*同一份*语料表字节，以及能感知语言区域的数字、日期和列表格式化。消息格式是 [Fluent](https://projectfluent.org) - Mozilla 的 `.ftl`，Firefox 用的那种 - 整个子系统默认在 `localization` feature 之后开启。

最短的一次导览。写一份语料表：

```ftl
# lang/en/app.ftl
welcome = Welcome to { $app }!
```

```ftl
# lang/es/app.ftl
welcome = ¡Bienvenido a { $app }!
```

从一个处理程序里使用它：

```rust
use suprnova::{__, handler, HttpResponse, Request, Response};

#[handler]
pub async fn greet(_req: Request) -> Response {
    Ok(HttpResponse::text(__!("welcome", app: "Suprnova")))
}
```

一个带着 `Accept-Language: es` 的请求会拿到西班牙语字符串，因为 `LocaleMiddleware` 在您的处理程序运行之前就解析好了语言区域。处理程序里的其他一切都不变 - 没有语言区域参数被穿线传入，签名里也没有 `&Translator`。

## 为什么要本地化

这是一件框架关心的事、而不是一个您自己挑的 crate，理由有三个：

- **验证消息是框架的字符串，不是您的。**“The email field is required.” 是在 `Rule::passes` 内部深处发出的，离您拥有的任何代码都很远。除非框架自带一道翻译接缝，否则一个西班牙语应用发出的就是英语验证错误 - 要么就得您自己手工包装每一条规则。Suprnova 的内置规则返回*带键*的消息；您通过丢进去一个 `.ftl` 文件来翻译它们，完全不用碰这些规则。
- **浏览器需要同样的字符串。** 一个 Inertia 应用，一半文本在 Rust 里渲染，一半在 Svelte/React/Vue 里渲染。两套翻译系统意味着两种文件格式、两套评审流程，以及同一句话走样两次的机会。Suprnova 会把服务器从 `/_suprnova/lang/<locale>.ftl` 解析出来的那份语料表原样送出，起步套件用 `@fluent/bundle` 解析它 - 一套文件，一个真相来源。
- **复数和格式是 CLDR 数据，不是字符串拼接。** 英语有两个复数类别，俄语和波兰语有四个，阿拉伯语有六个。同一个数字在 `en-US` 里是 `1,234.56`，在 `de-DE` 里是 `1.234,56`。Fluent 依据 CLDR 复数类别做选择，格式化交给 ICU4X 做，所以两者都不是您需要逐语言区域手搓的东西。

关掉这个 feature（`--no-default-features`）是支持的：本地化模块不会被编译进来，验证会渲染它内嵌的英语兜底字符串。其他一切形态不变。

## 文件布局

语料表活在 `lang/` 下面，一个语言区域一个目录：

```
myapp/
├── lang/
│   ├── en/
│   │   ├── app.ftl
│   │   └── validation.ftl
│   └── es/
│       ├── app.ftl
│       └── validation.ftl
├── src/
└── frontend/
```

规则如下：

- **目录名是一个 BCP-47 语言区域** - `en`、`en-GB`、`pt-BR`、`zh-Hans`。一个名字解析不了的目录会被跳过，并带一条 `warn!`，而不会让启动失败。
- **一个语言区域目录里的每一个 `.ftl` 都会合并进同一份语料表**，按文件名排序顺序合并。按功能拆分（`auth.ftl`、`billing.ftl`、`emails.ftl`）随您喜欢 - 消息 id 在这个语言区域内是全局的，所以 `auth.ftl` 和 `billing.ftl` 不能定义同一个 id。
- **框架自己的英语验证语料表最先加载**，加载进每一个语言区域的 bundle 里。您的文件在它之上加载，后加载的定义生效。这就是整个覆盖机制：在 `lang/es/validation.ftl` 里定义 `validation-min`，西班牙语 bundle 就会用您的版本。
- **根路径是 `lang_path()`** - `<APP_BASE_PATH>/lang`。当这个二进制文件运行在项目根目录以外的某个地方时（一个 systemd 单元、一个 `WorkingDirectory` 不同的容器），设置 `APP_BASE_PATH`；或者调用 `use_lang_path("…")` 来只移动 `lang` 目录。参见[环境变量](env-vars.md)。
- **缺失的 `lang/` 目录不是一个错误。** 一个全新的应用必须能启动，所以翻译器会带着内嵌的英语语料表启动，仅此而已。*格式错误*的 `.ftl` 是另一回事：解析错误会让启动失败，并指名是哪个文件、解析器反对的是什么，因为一份悄悄半加载的语料表，比一个被停下的进程更糟。
- **在 `local` 和 `development` 里，语料表会热重载。** 每个请求都会对 `lang/` 做一次 stat，只有在确实有变化时才重新解析，所以编辑一个 `.ftl` 会在下一次刷新时体现出来。生产环境从不重新 stat；语料表只在启动时读取一次。

## 五分钟学会 FTL

Fluent 是一种小巧的格式。这一节涵盖了一个典型应用需要的一切。

**消息** 是 `id = value` 这样的配对。按惯例 id 用 kebab-case（框架自己的就是这样），value 一直延伸到行尾，缩进的续行会被拼接起来：

```ftl
# 一条注释。附着在它下面的那条消息上。
sign-in = 登录
password-hint =
    请至少使用 12 个字符。几个普通单词组成的口令短语，
    胜过一串短小的符号。
```

**参数** 是 `{ $name }` 这样的占位符。您在调用时提供它们；缺失的参数是一个错误，不是一个空字符串（`Lang::get` 随后会沿它的链走下去 - 参见 [`Lang` 门面](#lang-门面)）：

```ftl
greeting = 你好，{ $name }！
invoice-line = { $qty } × { $item }
```

**术语** 以 `-` 开头，只对语料表内部可见，存在的意义是让一个品牌名或者一个反复出现的短语只住在一个地方：

```ftl
-product-name = Suprnova
about = 关于 { -product-name }
footer = © 2026 { -product-name }。保留所有权利。
```

**选择器** 是 Fluent 的条件语句。选择器的值会拿去和各个变体的键匹配；恰好一个变体会被标上 `*` 作为默认值：

```ftl
cart-summary =
    { $count ->
        [0] 您的购物车是空的。
        [one] 购物车里有一件商品。
       *[other] 购物车里有 { $count } 件商品。
    }
```

`[0]` 匹配字面上的数字零。`[one]` 和 `[other]` 是 **CLDR 复数类别**，针对这个 bundle 的语言区域来解析 - 这正是 Fluent 派上用场的地方。英语有两个类别；俄语有四个，一位俄语译者可以把全部四个都写出来，而您不需要改一行 Rust 代码：

```ftl
# lang/ru/app.ftl
unread-messages =
    { $count ->
        [one] У вас { $count } непрочитанное сообщение.
        [few] У вас { $count } непрочитанных сообщения.
        [many] У вас { $count } непрочитанных сообщений.
       *[other] У вас { $count } непрочитанного сообщения.
    }
```

CLDR 把 `1`、`21`、`31` 分到 `one`；把 `2`-`4`、`22`-`24` 分到 `few`；把 `0`、`5`-`20`、`25`-`30` 分到 `many`；把分数分到 `other`。同一次 `__!("unread-messages", count: 22)` 调用，在英语、俄语、波兰语和阿拉伯语里都能正确渲染，因为类别的选择是数据，不是代码。

**永远把 `*` 放在 `other` 上。** 这是 CLDR 为每一个语言区域都定义的那一个类别，所以它是唯一保证存在的变体 - 而默认值正是一个不匹配的选择器值最终落到的地方，包括任何非整数的计数。把 `*[many]`（或者任何其他类别）标记为默认值，会把分数送去给为整数写的文本。

> **把计数当作数字传递。** `__!("unread-messages", count: 3)` 发送的是一个 JSON 数字，会选中一个复数类别。`count: "3"` 发送的是一个字符串，它只能匹配一个字面量的变体键 - 会落到您的 `*[other]` 默认值上。这是唯一一个值得牢记的 FTL 陷阱。

**函数** 在占位符内部调用。有两个已注册：`NUMBER()`（Fluent 的内置函数）和 `DATETIME()`（Suprnova 的）：

```ftl
score = 您的得分是 { NUMBER($points) }，满分 { NUMBER($total) }。
published = 发布于 { DATETIME($when, dateStyle: "medium") }
```

两者都请参见[能感知语言区域的格式化](#能感知语言区域的格式化)。

**一处刻意为之的限制：** Suprnova 只解析扁平的消息*值*。Fluent 的属性语法（`login .placeholder = …`）能解析，但没法经由 `Lang::get` 寻址，所以请给每个字符串保留一个 id：用 `login-placeholder`，不要用 `login.placeholder`。id 在每个语言区域内是一个扁平的命名空间 - 给它们加前缀（`auth-login-title`、`billing-invoice-due`），而不要去够一个解析器根本没有的层级结构。

## `Lang` 门面

`Lang` 是服务器端的入口点。每一个方法都会读取**当前语言区域**，也就是中间件为这个请求绑定的那个。

| 方法 | 返回值 | 备注 |
|---|---|---|
| `Lang::get(key)` | `String` | 不会失败。走完回退顺序之后，返回这个键本身 |
| `Lang::get_with(key, args)` | `String` | 同上，带参数 |
| `Lang::try_get(key)` | `Result<String, FrameworkError>` | 报错，而不是退化 |
| `Lang::try_get_with(key, args)` | `Result<String, FrameworkError>` | 同上，带参数 |
| `Lang::has(key)` | `bool` | 这个键是否能为当前语言区域解析出来，或者在它的回退顺序上的任何一环解析出来 |
| `Lang::locale()` | `Locale` | 当前语言区域 |
| `Lang::set_locale(locale)` | `()` | 把它改成这个请求剩余部分要用的值 |
| `Lang::available_locales()` | `Vec<Locale>` | 每一个已加载语料表的语言区域 |

```rust
use suprnova::{Lang, Locale, TranslateArgs};

let subject = Lang::get("password-reset-subject");

let mut args = TranslateArgs::new();
args.insert("name".into(), serde_json::json!("Ada"));
args.insert("count".into(), serde_json::json!(3));
let body = Lang::get_with("unread-messages", args);

if Lang::has("beta-banner") {
    // 只有部分语言区域提供了这份横幅文案。
}

let locales: Vec<String> = Lang::available_locales()
    .iter()
    .map(Locale::as_str)
    .collect();
```

`TranslateArgs` 是一个从 `String` 到 `serde_json::Value` 的有序映射，两者都从 crate 根部重新导出。Fluent 参数是字符串和数字；其他 JSON 形态会被字符串化。

### 回退顺序

`Lang::get` 从不失败，也从不返回一个空字符串。按顺序：

1. **当前语言区域**的语料表。
2. 它**已配置的回退父级**（参见[回退链](#回退链)），传递地遍历下去，如果配置了的话 - `pt-PT` 先于 `pt-BR`，`pt-BR` 先于它自己指名的父级，依此类推。
3. **回退语言区域**的语料表（`APP_FALLBACK_LOCALE`，默认是 `en`），除非它已经在这条链里更早出现过。
4. **键本身**，外加每一对缺失的 `(locale, key)` 一条 `tracing::warn!` - 只发一次，不是每个请求发一次，所以热路径上的一个缺失键不会把您的日志灌满。

第 4 步就是为什么一个缺失的翻译会在按钮里渲染出 `checkout-submit`、而不是一个空白按钮：一个明显错误的字符串是一份等着被提的 bug 报告，而一个空字符串则是一个谜。

当您想要知道、而不是想要退化时，用那些 `try_*` 对应函数。它们运行第 1 到第 3 步，然后返回 `Err`，而不是去做第 4 步：

```rust
use suprnova::Lang;

// 这里缺失一个键，意味着一封邮件坏掉了 - 让这个作业失败，不要发一条
// 主题行里带着裸键的消息。
let subject = Lang::try_get("invoice-paid-subject")?;
```

### `__!` 宏

`__!` 是照顾 Laravel 肌肉记忆的简写形式。不带参数时，它调用 `Lang::get`；带命名参数时，它构建一个 `TranslateArgs` 并调用 `Lang::get_with`：

```rust
use suprnova::__;

let plain = __!("welcome-back");
let greeted = __!("greeting", name: "Ada");
let counted = __!("unread-messages", name: "Ada", count: 3);
```

参数值可以是任何能转换成 `serde_json::Value` 的东西 - `&str`、`String`、整数、浮点数、`bool`。这个宏在 crate 根部导出，所以当您不想把 `__` 引入作用域时，`suprnova::__!("welcome-back")` 不需要那条 import 也能用。

## 回退链

`APP_FALLBACK_LOCALE` 是每一个语言区域之下的那一张全局兜底网。有时候这还不够：欧洲葡萄牙语和巴西葡萄牙语几乎共享一切，只在少数几个词上有分歧（`ficheiro`/`arquivo`、`utilizador`/`usuário`、`tu`/`você`），而维护两份完整的语料表，意味着每一个新字符串都得写两遍。一个**回退父级**让 `pt-PT` 能先继承自 `pt-BR`，然后 `pt-BR` 才进一步回退到全局的 `fallback_locale` - 所以 `lang/pt-PT/` 只需要保留那些确实不同的字符串。

### 配置父级

一个环境变量，逗号分隔的 `child=parent` 对：

```env
APP_LOCALE_PARENTS=pt-PT=pt-BR
```

或者用构建器，一对一次调用，可以链式调用：

```rust
use suprnova::{Config, Locale, LocalizationConfig};

pub fn register_all() {
    let localization = LocalizationConfig::from_env()
        .expect("APP_LOCALE / APP_FALLBACK_LOCALE must be valid BCP-47")
        .parent(
            Locale::parse("pt-PT").expect("valid locale"),
            Locale::parse("pt-BR").expect("valid locale"),
        );

    Config::register(localization);
}
```

两条路径喂给的是同一张映射表（`LocalizationConfig::parents`），并且都是在启动时被校验的，而不是在请求时：

- 一对没有 `=`、或者子级/父级为空的配对，是一条格式错误的 `APP_LOCALE_PARENTS` 条目 - 启动会失败，并指名那个坏掉的片段。
- 配对任意一侧的语言区域，如果作为 BCP-47 无效，同样会失败。
- 给同一个子级指名两次是含糊的配置，不是后者生效 - 启动会失败，并指名那个重复的子级。
- **一个环会让启动失败。** 这个错误会把这个环拼出来：两个语言区域互相指名对方（`pt-PT=pt-BR,pt-BR=pt-PT`）会产出 `` `pt-PT` -> `pt-BR` -> `pt-PT` ``。一个语言区域把自己指名为自己的父级（`pt-PT=pt-PT`）是同一种情形的缩小版 - `` `pt-PT` -> `pt-PT` ``。（有两条代码路径会抛出这个错误：解析 `APP_LOCALE_PARENTS` - 所以任何配置经由 `LocalizationConfig::from_env()` 的应用，都会在配置加载时失败 - 以及 `FluentTranslator` 的语料表加载，它会捕获一张用 `.parent(...)` 以编程方式构建出来的、有环的映射表。只有一个完全手写配置、*并且*在 `bootstrap_fn` 里绑定了自己的自定义 `Translator` 的应用，才会两者都跳过；`Lang` 的这次遍历是独立设防的，在那里仍然会安全终止，只是不会得到那个明确的启动期错误。）

构建器的 `.parent(child, parent)`，对一个重复的子级来说是后写的生效 - 一次后来的调用覆盖一次更早的调用，只是一次更晚的覆盖，不是 `APP_LOCALE_PARENTS` 所防范的那种含糊输入情形。

### 解析顺序

一条链可以不止一跳长：`pt-PT` 指名 `pt-BR` 作为它的父级，而 `pt-BR` 又可以转而指名它自己的父级。`Lang::get` / `try_get` / `get_with` / `try_get_with` / `has` 都会遍历整条链，当前语言区域优先：

1. **当前语言区域**的语料表。
2. 它**已配置的父级**，然后是*那个*语言区域已配置的父级，传递地遍历下去，直到遇到一个没有配置父级的语言区域为止。
3. 全局的 **`fallback_locale`**（`APP_FALLBACK_LOCALE`），除非它已经在这条链里更早出现过 - 包括那种常见情形：它就是当前语言区域本身（`en`/`en` 这个默认值）。

如果链上什么都没能解析出这个键，`Lang::get` / `Lang::get_with` 会落到键本身，正如[回退顺序](#回退顺序)所描述的那样；`Lang::try_get` / `Lang::try_get_with` 会返回 `Err`，`Lang::has` 会返回 `false`。这次遍历运行在 `Lang` 门面自己内部，所以它对**任何** `Translator` 都有效 - 无论是内置的 `FluentTranslator`，还是您自己写的驱动程序。

### 一个可运行的例子

```
myapp/
├── lang/
│   ├── pt-BR/
│   │   ├── app.ftl
│   │   └── validation.ftl
│   └── pt-PT/
│       └── app.ftl
├── src/
└── frontend/
```

```ftl
# lang/pt-BR/app.ftl
welcome = Bem-vindo ao { $app }!
file-label = Arquivo
```

```ftl
# lang/pt-PT/app.ftl
file-label = Ficheiro
```

```rust
use suprnova::__;

// 一个解析成 `pt-PT` 的请求。
assert_eq!(__!("file-label"), "Ficheiro");                    // pt-PT 自己的覆盖
assert_eq!(
    __!("welcome", app: "Suprnova"),
    "Bem-vindo ao Suprnova!"                                  // 继承自 pt-BR
);
```

`lang/pt-PT/` 从不定义 `welcome` - 它不需要定义。`file-label` 是这两份语料表之间一处真正的、一个词的差异，所以它是唯一一个拥有自己文件的 id。

### 已提供服务的语料表是扁平化的

`/_suprnova/lang/pt-PT.ftl` 这个端点（参见[语料表端点](#语料表端点)）从不要求浏览器知道 `pt-BR` 的存在。`FluentTranslator` 会在加载时把整条链预先合并成每个语言区域一份资源 - 最底下是 `en`/`en-*` 语言区域用的、内嵌的框架语料表，然后是已配置的父级链，然后是这个语言区域自己的文件 - 并提供*那份*已经扁平化的资源。抓取 `pt-PT.ftl`，响应里会同时携带 `welcome` 和 `file-label`，在一次请求里，没有任何客户端的链式逻辑。`?v=<hash>` 依然指名一份不可变的资源；只是现在这个哈希也覆盖了从 `pt-BR` 拉进来的那些字符串。

**扁平化只覆盖已配置的父级** - 它从不会越过它们去够 `fallback_locale`。`pt-PT` 提供服务的语料表包含 `pt-BR` 的字符串，是因为 `pt-BR` 是一个*已配置的父级*；它不会仅仅因为 `en` 恰好是全局兜底，就包含 `en` 的字符串。`LocaleShare` 的 `fallback` 字段总是指名终端的那个 `fallback_locale`，不受这一切影响 - 它告诉前端的是 `Lang` 门面层面的这次遍历最终会落到哪里，而不是它刚刚抓到的文件里已经有什么。

### Delta 文件的合并规则

一份子级语料表会**在 Fluent AST 这一层**合并到它的父级之上，不是靠文本拼接，也不是靠整条消息的遮蔽。覆盖的单位是*模式（pattern）*，所以：

- **一个子级的值会替换掉父级的值**，落在这个文件里父级原来的那个位置上。
- **一个带属性但没有值的子级条目，会保留父级的值。** 重新翻译 `.placeholder` 不需要重复这条消息自己的文本。
- **属性按名字合并。** 一个同名的子级属性会原地替换掉父级的；一个只存在于子级的属性，会追加在父级自己的属性之后。**子级没有提到的属性，会从父级存活下来** - 覆盖一条消息的值，从不会悄悄丢掉它的 `.placeholder` 或者 `.aria-label`。
- **选择表达式是整体替换，从不逐变体替换。** 一个选择器的各个变体，是按某一个语言区域的 CLDR 复数类别来定键的；因为这些类别是依语言区域而定的，把父级的一个变体和子级的另一个变体拼在一起，可能会产出一个背后没有任何单一语言区域语法支撑的选择器。一个子级如果要覆盖一个选择器，就必须提供它想要的每一个变体。
- **一条被覆盖条目上的注释，保留父级的。** 这条注释记录的是这个 id，而覆盖的单位是模式，不是注释。
- **只存在于子级的条目会追加在末尾**，按子级自己的顺序，注释也包括在内 - 一个 `pt-BR` 从未定义过的 id，不是对任何东西的“覆盖”。

术语（`-brand`）遵循一模一样的规则，只有一处收窄：在 Fluent 语法里，一个术语的值从来都不是可选的，所以上面那条“有属性没有值就保留父级的值”的情形只适用于消息 - 一个子级术语总会提供一个值，并且那个值总是生效。按名字合并属性、对值做整体模式替换、注释归父级所有，这些规则对术语的适用方式和对消息完全一样。术语被追踪在它们自己的命名空间里 - 覆盖 `-brand` 永远不会遮蔽一条同样叫 `brand` 的消息。

### 为什么 Suprnova 有所不同

Laravel 13 只有一层回退：单一的全局 `fallback_locale` 配置值，在当前语言区域的数组缺失一个键时会被查阅。它没有“一个语言区域继承自一个兄弟语言区域”这种概念 - `pt_PT.php` 和 `pt_BR.php` 是两个不相关的数组，一个 `pt_PT` 应用要么把 `pt_BR` 已经翻译好的一切都重复一遍，要么就没有它。

Suprnova 的父级链是 Rust 这一侧的扩展：在“这个语言区域”和“全局兜底”之间加了一个中间步骤，逐语言区域配置，而不是只配置一次全局的。我们不想做的那个取舍，是把这份复杂性推给浏览器 - 一个能感知链的前端，得先抓取 `pt-PT.ftl`，发现它不完整，再去抓取 `pt-BR.ftl`，然后在客户端用 JavaScript 把两者合并起来，用的规则还必须和服务器的一模一样。改成在加载时就扁平化，就意味着提供服务的语料表永远是一份完整的、自包含的文件 - 和前端在父级链存在之前就已经有的那份契约完全一样，所以 `@fluent/bundle` 和起步套件的包装器完全不需要改动就能支持这个功能。

## 语言区域检测

`LocaleMiddleware` 为每个请求解析出一个语言区域，并在处理程序运行期间绑定它。这条链是由配置驱动的，并且**先命中者生效**：

1. **会话** - 会话里的 `locale` 键，前提是[会话中间件](session.md)运行过、并且这个值指名了一个可用的语言区域。“用户在设置里选了 Español”就活在这里。
2. **Cookie** - `locale` cookie。在登出之后依然存活，所以一次在登录之前做出的语言选择不会丢失。
3. **`Accept-Language`** - 用 `fluent-langneg` 对照 `available_locales()` 协商，尊重 q 值。`fr-CH, es;q=0.8, en;q=0.5` 对照 `en` + `es` 这两份语料表会解析成 `es`。
4. **`APP_LOCALE`** - 当以上都没命中时，那个已配置的默认值。

一个解析不了的候选值，或者一个指名了没有语料表的语言区域的候选值，会被**跳过，而不是拒绝**。一个带着过期 `locale=zz` cookie 的用户，看到的是默认语言，而不是一个 500。一个格式错误的 `Accept-Language` 请求头也是同样处理。攻击者可控的输入，在每一个请求上都会抵达这条链；它绝不能被允许做出比“挑一种语言”更多的事。

请在 `bootstrap.rs` 里把它接上，接在会话中间件**之后**，因为第 1 步要读会话：

```rust
use std::sync::Arc;
use suprnova::{
    global_middleware, App, LocaleMiddleware, LocaleShare, SessionConfig, SessionMiddleware,
};

pub async fn register() {
    global_middleware!(SessionMiddleware::install(SessionConfig::from_env()).await);

    // 解析这个语言区域，并为这个请求绑定它。
    global_middleware!(LocaleMiddleware::from_env().expect("locale config"));

    // 在每一个 Inertia 页面上，把语言区域 + 语料表 URL 交给前端。
    App::register_inertia_shared(Arc::new(LocaleShare));
}
```

`LocaleMiddleware::from_env()` 读取 `LocalizationConfig::from_env()`；`LocaleMiddleware::new(config)` 接受一个您自己构建的。一个脚手架生成的应用已经带着这两行了。

### 在请求进行到一半时改变语言区域

`Lang::set_locale` 就是 Laravel 的 `App::setLocale` - 它从那一刻起，重写当前请求的语言区域：

```rust
use suprnova::session::session_mut;
use suprnova::{FrameworkError, Lang, Locale};

/// 用户刚刚在一个设置表单里切换了语言。
pub fn switch_language(choice: &str) -> Result<(), FrameworkError> {
    let locale = Locale::parse(choice)?;
    Lang::set_locale(locale);                       // 这一次请求
    session_mut(|s| s.put("locale", choice));       // 之后的每一次请求
    Ok(())
}
```

请注意这两半：`set_locale` 影响的是*这一次*请求（所以那次重定向的 flash 消息已经是西班牙语了），而写入会话的部分，才是检测链在*下一次*请求时会读取的东西。

### 在请求之外

Console 命令、队列工作进程和计划任务，没有请求，也没有中间件。在那里，`Lang::set_locale` 写入的是一个进程全局的覆盖值，`Lang::locale()` 会在回退到 `APP_LOCALE` 之前查阅它：

```rust
use suprnova::{command, FrameworkError, Lang, Locale, Mail};

use crate::mail::Digest;
use crate::models::user::User;

#[command(name = "mail:digest", description = "Send the weekly digest")]
pub async fn send_digest(_args: Vec<String>) -> Result<(), FrameworkError> {
    for user in User::query().get().await? {
        // 每个用户存下来的偏好，在他们那封邮件的整个处理期间生效。
        Lang::set_locale(Locale::parse(&user.locale)?);
        Mail::to(&user.email).send(Digest::for_user(&user)).await?;
    }
    Ok(())
}
```

因为这个覆盖值是进程级的、而不是任务本地的，所以请像上面这样，在每个工作单元的最开头设置它 - 不要指望它在一次 `.await` 期间保持不变，因为另一个任务可能会和它交错执行。

## 配置

三个环境变量。`APP_LOCALE` 和 `APP_FALLBACK_LOCALE` 都默认为 `en`；`APP_LOCALE_PARENTS` 默认为空 - 没有逐语言区域的覆盖，只有 `fallback_locale` 生效：

```env
APP_LOCALE=en
APP_FALLBACK_LOCALE=en
# APP_LOCALE_PARENTS=pt-PT=pt-BR
```

其他一切都是代码，架在 `LocalizationConfig` 上。它像每一个其他有类型的配置一样注册 - 在您的 `config::register_all` 里，它在启动之前运行：

```rust
// src/config/mod.rs
use suprnova::{Config, Detect, Locale, LocalizationConfig};

pub fn register_all() {
    let localization = LocalizationConfig::from_env()
        .expect("APP_LOCALE / APP_FALLBACK_LOCALE must be valid BCP-47")
        .default_locale(Locale::parse("es").expect("valid locale"))
        .use_isolating(true)                                // 参见分歧说明
        .detection(vec![Detect::Session, Detect::Header])   // 忽略这个 cookie
        .session_key("preferred_locale")
        .cookie_name("lang")
        .parent(                                            // 参见回退链
            Locale::parse("pt-PT").expect("valid locale"),
            Locale::parse("pt-BR").expect("valid locale"),
        );

    Config::register(localization);
}
```

- `default_locale` / `fallback_locale` - 从代码里覆盖 `APP_LOCALE` 和 `APP_FALLBACK_LOCALE`。任何一处的格式错误值，都会让启动失败，而不是悄悄变成 `en`。
- `use_isolating` - 插值周围的 Unicode 隔离标记。默认关闭；当您发布一个 RTL 语言区域时打开它。
- `detection` - 这条链，按顺序。去掉 `Detect::Cookie` 意味着一次语言选择只活在会话里；去掉 `Detect::Header` 意味着浏览器的偏好被完全忽略。
- `session_key` / `cookie_name` - 给这两处查找改名。
- `parents` - 逐语言区域的回退父级（`child -> parent`），在一个键从子级的语料表里缺失时，会在 `fallback_locale` 之前被遍历；形态和 `APP_LOCALE_PARENTS` 一样。用 `.parent(child, parent)` 添加一个 - 可以链式调用，对一个重复的子级是后写的生效。完整的契约（启动期校验、解析顺序、已提供服务的语料表的扁平化）请参见[回退链](#回退链)。

启动会在容器里绑定一个 `Arc<dyn Translator>`。如果您的应用已经绑定了一个，框架会随它去 - 这就是您无需 fork 任何东西、就能替换成自己的翻译器的办法：

```rust
// src/bootstrap.rs
use std::sync::Arc;
use suprnova::{App, FluentTranslator, LocalizationConfig, Translator};

pub async fn register() {
    let config = LocalizationConfig::from_env().expect("locale config");
    let translator =
        FluentTranslator::from_dir("./catalogs", &config).expect("load catalogs");
    App::bind::<dyn Translator>(Arc::new(translator));
}
```

`Translator` 是这里的扩展接缝：`translate`、`has`、`available_locales`、`catalog`、`reload`。有一个驱动程序已实现（`FluentTranslator`），一个新的后端就是一个新的驱动程序 - 不需要 fork 这个表面。

## 翻译过的验证消息

每一条内置规则都返回一条**带键**的消息：一个语料表键、这条消息需要的参数，以及一个英语兜底。翻译只发生一次，在序列化边界上 - `ValidationErrors::to_json` 和 Inertia 的错误包 - 从不发生在规则内部。规则保持纯粹，整个子系统也能被编译出去。

这些键遵循一条约定：

| 形状 | 例子 | 用于 |
|---|---|---|
| `validation-<rule>` | `validation-min`、`validation-required-if` | 每条内置规则一个，kebab-case |
| `field-<name>` | `field-email` | 一个字段的人类可读名字 |
| `validation-invalid-data` | - | 顶层的“The given data was invalid.”横幅 |

要翻译它们，请在目标语言区域下的任意一个 `.ftl` 文件里，定义您关心的那些 id：

```ftl
# lang/es/validation.ftl
validation-invalid-data = Los datos proporcionados no son válidos.
validation-required = El campo { $field } es obligatorio.
validation-email = El campo { $field } debe ser una dirección de correo válida.
validation-min = El campo { $field } debe tener al menos { $min } caracteres.
validation-confirmed = La confirmación del campo { $field } no coincide.
```

`$field` 永远可用。每条规则自己的参数，会以它们在框架的英语语料表里携带的名字传入 - `$min`、`$max`、`$other`、`$value` - 而 `framework/src/localization/catalogs/en/validation.ftl` 就是那份权威的 id 和参数清单。从里面拷贝出您需要的那些 id 就行；您从不需要把它们全部覆盖一遍。

覆盖是逐语言区域、逐键生效的。在 `lang/en/validation.ftl` 里定义 `validation-min`，会替换掉框架针对那一条规则的英语措辞，其余的保持不变。

### 字段名

对一个原始列名做插值，会产出“The email_address field is required.”。`field-<name>` 这条约定修复了这一点：

```ftl
# lang/en/validation.ftl
field-email_address = email address
field-dob = date of birth
```

在渲染之前，翻译器会为当前语言区域查找 `field-<name>`。命中时会作为 `$field` 传入；未命中则回退到把下划线换成空格之后的那个字段名。所以上面这个文件，只对那些人性化效果不好的名字才是必需的。

### 自定义规则

`Rule::passes` 返回 `Result<(), ValidationMessage>`。一条带键的消息会参与翻译：

```rust
use suprnova::{Rule, ValidationMessage};

pub struct StartsWith(pub &'static str);

impl Rule for StartsWith {
    fn passes(&self, value: &str) -> Result<(), ValidationMessage> {
        if value.starts_with(self.0) {
            Ok(())
        } else {
            Err(ValidationMessage::keyed("validation-starts-with")
                .arg("prefix", self.0)
                .fallback(format!("must start with {}", self.0)))
        }
    }
}
```

```ftl
# lang/en/validation.ftl
validation-starts-with = The { $field } field must start with { $prefix }.
```

一个纯字符串依然能用，对一条只会存在于一种语言里的消息来说，这就是正确答案：

```rust
Err("must start with acct_".into())   // 无键：逐字渲染
```

无键的消息完全跳过翻译，这正是既有自定义规则能保持编译通过、行为和以前一模一样的原因。

### 派生流程

`#[derive(Validate)]` 的错误也是带键的。`validator` crate 的错误码，会变成把下划线换成短横线之后的 `validation-<code>`，而验证器附上的每一个参数，都会变成一个消息参数 - 有两个保留的例外，`value` 和 `other`，它们总是会被丢弃。两者携带的都是一个字段的实际*值*，而不是关于这条规则的元数据：`value` 是被测试的那份回显输入，`other`（由 `must_match` 设置，也就是那条典型的密码确认规则）是那个兄弟字段的值。两者都从不会被交给语料表，所以无论一份 `.ftl` 覆盖把 `validation-must-match` 怎么措辞，都不可能把一个提交上来的秘密插值进一个 422 响应体里。所以一次 `#[validate(email)]` 失败，会像手写规则那样解析出 `validation-email`，而一个翻译了其中一个的语言区域，两个都会翻译。

## 前端

浏览器拿到的是服务器解析出来的同一份字节。没有任何东西被重新翻译、重新导出，或者靠手工保持同步。

### 语料表端点

```
GET /_suprnova/lang/es.ftl              → 200 text/plain, ETag: "<hash>"
GET /_suprnova/lang/es.ftl?v=<hash>     → 200 + Cache-Control: public,
                                          max-age=31536000, immutable
GET /_suprnova/lang/es.ftl              → If-None-Match 匹配时为 304
GET /_suprnova/lang/zz.ftl              → 404（没有这份语料表）
```

响应体是那个语言区域的合并语料表 - 先是框架消息，然后是它已配置的回退父级链（如果有的话，参见[回退链](#回退链)），然后是您的文件，按加载顺序。`ETag` 是内容哈希。带着一个具体的哈希用 `?v=` 请求，响应就会被永久地不可变缓存，因为那个 URL 只可能意味着一件事；不带它请求，得到的就是重新校验。和 `/_suprnova/health` 一样，这条路径豁免于中间件链之外：它必须在一个语言区域被解析出来之前就能应答，并且它不携带任何用户数据。

### 共享 prop

`LocaleShare` 是框架提供的一个 `InertiaSharedData`。在 `bootstrap.rs` 里注册之后（参见[语言区域检测](#语言区域检测)），它会给每一个 Inertia 页面加上一个 prop：

```json
{
  "lang": {
    "locale": "es",
    "fallback": "en",
    "catalog": {
      "url": "/_suprnova/lang/es.ftl?v=9f2c1ae4",
      "hash": "9f2c1ae4"
    }
  }
}
```

当没有翻译器被绑定时，`catalog` 是 `null` - 这个共享 prop 从不会让页面渲染失败。

### 起步套件的包装器

每个起步套件都提供一个约 100 行的包装器，读取那个 prop，取一次语料表，构建一个 `@fluent/bundle` bundle，并暴露 `t()`。在您的 Inertia 入口点里调用一次 `initLang`（脚手架生成的应用已经这样做了）：

```ts
// frontend/src/main.ts
import { createInertiaApp } from '@inertiajs/svelte'
import { mount } from 'svelte'
import { initLang } from './lib/lang.svelte'

createInertiaApp({
  resolve: (name) => { /* …不变… */ },
  async setup({ el, App, props }) {
    await initLang(props.initialPage)
    mount(App, { target: el!, props })
  },
})
```

然后，在组件里：

```svelte
<!-- Svelte 5 -->
<script lang="ts">
  import { t, currentLocale } from '../lib/lang.svelte'
</script>

<h1>{t('welcome', { app: 'Suprnova' })}</h1>
<p>{currentLocale()}</p>
```

```tsx
// React 19
import { useLang } from '../lib/lang'

export default function Home() {
  const { t, locale } = useLang()
  return <h1>{t('welcome', { app: 'Suprnova' })}</h1>
}
```

```vue
<!-- Vue 3.5 -->
<script setup lang="ts">
import { useLang } from '../lib/lang'
const { t, locale } = useLang()
</script>

<template>
  <h1>{{ t('welcome', { app: 'Suprnova' }) }}</h1>
</template>
```

客户端上的数字和日期格式化，用的是浏览器内置的 `Intl` - 没有任何 ICU 数据会被发给浏览器。

### 有类型的消息键

`suprnova generate-types` 解析 `lang/<默认语言区域>/*.ftl`，并连同 page-props 类型一起，生成一个涵盖每一个消息 id 的联合类型：

```ts
// frontend/src/types/lang-keys.ts
// Generated by `suprnova generate-types` - do not edit.
export type MessageKey =
  | "validation-min"
  | "welcome"
```

这些包装器给 `t(key: MessageKey, …)` 标了类型，所以这和 [`inertia-props.ts`](frontend-typescript-types.md) 是同一份承诺：在 Rust 里重命名一条消息，重新生成，TypeScript 编译器就会指出每一个还在用旧 id 的调用点。`suprnova serve` 会和 `src/` 一起监视 `lang/`，所以当您编辑语料表时，这个文件会重新生成。

一个没有 `lang/` 目录、也没有消息 id 的项目，得到的是**没有文件** - 一个没有本地化的应用，不会看到任何新产物出现。

## 能感知语言区域的格式化

`Lang` 上的七个函数，全部由 ICU4X 支撑，全部读取当前语言区域，全部都有返回 `Result<String, FrameworkError>`、而不是退化的 `try_*` 对应函数：

```rust
use suprnova::chrono::NaiveDate;
use suprnova::{DateStyle, Lang, ListStyle, RelativeUnit, TimeStyle};

let dt = NaiveDate::from_ymd_opt(2026, 8, 1)
    .and_then(|d| d.and_hms_opt(14, 30, 0))
    .expect("valid datetime");

Lang::number(1_234_567.89);                          // en-US → 1,234,567.89
                                                     // de-DE → 1.234.567,89
Lang::currency(19.99, "USD");                        // en-US → $19.99
Lang::date(&dt, DateStyle::Long);                    // en-US → August 1, 2026
Lang::time(&dt, TimeStyle::Short);                   // en-US → 2:30 PM
Lang::datetime(&dt, DateStyle::Medium, TimeStyle::Short);
Lang::list(&["Ada", "Grace", "Alan"], ListStyle::And); // → Ada, Grace, and Alan
Lang::relative(-3, RelativeUnit::Day);               // → 3 days ago
```

这些风格枚举：`DateStyle { Full, Long, Medium, Short }`、`TimeStyle { Medium, Short }`、`ListStyle { And, Or, Unit }`、`RelativeUnit { Second, Minute, Hour, Day, Week, Month, Year }`。`Lang::relative` 接受一个带符号的量 - 负数是过去（“3 天前”），正数是未来（“3 天后”）。

> 精确的输出来自烤进 ICU4X 里的 CLDR 数据，会随着 ICU 升级而变化，日期和货币尤其如此。在您自己的测试里，请针对形状和语言区域可区分性做断言（`de != en`、包含 `2026`），而不要针对精确的字节。

### 在一条消息内部格式化

有两个函数可以从 FTL 里调用：

```ftl
order-total = 您的总额是 { NUMBER($amount, maximumFractionDigits: 2) }。
published = 发布于 { DATETIME($when, dateStyle: "medium", timeStyle: "short") }
```

```rust
use suprnova::__;

let line = __!("published", when: "2026-08-01T14:30:00");
```

`NUMBER()` 是 Fluent 的内置函数，被显式注册，在消息内部给您小数位数的控制权。`DATETIME()` 是 Suprnova 的：`$value` 接受一个 ISO-8601 字符串或者纪元毫秒数，`dateStyle` / `timeStyle` 用的是和 Rust 枚举一样的名字，小写。一个它没法解析的值，会带着一条 `warn!` 原样透传 - 一个 Fluent 函数没法返回一个错误，而一个带着一处看起来古怪的日期的渲染页面，也好过一个 500。

当您想要 ICU4X 的完整格式化能力、而不是一个 Fluent 函数所暴露的那些时，就在 Rust 里格式化，再把成品字符串传进去：

```rust
use suprnova::{__, Lang};

let total = __!("order-total-text", amount: Lang::currency(19.99, "USD"));
```

## 测试您的翻译

两个辅助函数完成这项工作：`use_lang_path` 把加载器指向一个 fixture 目录，`scope_locale` 在一个 future 的整个生命周期内钉住当前语言区域。

那种密封的形式 - 在一个 fixture 目录之上构建一个翻译器，并把它绑定进一个测试范围的容器 - 是框架自己的测试所使用的，因为它不触碰任何进程全局状态，并且能在并行测试执行下存活：

```rust
use std::sync::Arc;
use suprnova::testing::TestContainer;
use suprnova::{scope_locale, FluentTranslator, Lang, Locale, LocalizationConfig, Translator};

#[tokio::test]
async fn spanish_greeting_comes_from_the_catalog() {
    let _guard = TestContainer::fake();

    let config = LocalizationConfig::from_env().expect("locale config");
    let translator = FluentTranslator::from_dir("tests/fixtures/lang", &config)
        .expect("load catalogs");
    TestContainer::bind::<dyn Translator>(Arc::new(translator));

    scope_locale(Locale::parse("es").expect("locale"), async {
        assert_eq!(Lang::get("welcome"), "¡Bienvenido!");
        assert_eq!(Lang::locale().as_str(), "es");
    })
    .await;
}
```

当一个测试启动的是真实的应用、并且您想让*整个*应用都指向 fixture 时，`use_lang_path` 就是正确的工具：

```rust
use suprnova::use_lang_path;

#[tokio::test]
async fn app_boots_against_fixture_catalogs() {
    use_lang_path("tests/fixtures/lang");
    // …启动这个应用；`lang_path("")` 现在会解析到这个 fixture 目录。
}
```

它写入的是一个进程全局的路径覆盖，所以请把它当作一个逐二进制文件的设置，而不是两个并行测试可能起冲突的东西。

检测本身 - session/cookie/`Accept-Language` 这条链 - 值得经由真实的流水线来测试，而不是直接调用这个中间件，因为有意思的场景，是关于请求头解析、以及哪个来源会赢的。挂载一条处理程序返回 `__!("welcome")` 的路由，把 `LocaleMiddleware` 注册进 `MiddlewareRegistry`，然后用来自 [HTTP 测试](http-tests.md)的环回测试装置来驱动它，发送 `Accept-Language: fr, es;q=0.8`，并对西班牙语响应体做断言。值得钉住的场景：一个请求头会被协商、一个 cookie 会赢过一个请求头、一个不可用的语言区域会被跳过而不是报错，以及一个格式错误的请求头依然返回 200。

当您的测试跑在一个多线程运行时上时，`TestContainer::scope` 请参见[测试](testing.md) - 上面那个线程本地的 `fake()` 守卫，撑不过一个 future 在工作线程之间的迁移。

### 为什么 Suprnova 有所不同

**是 FTL 文件，不是 PHP 数组。** Laravel 有两种格式 - `lang/en/messages.php` 里的嵌套数组，加上 `lang/en.json` 里针对字符串键翻译的扁平 JSON - 两者都不能被浏览器加载，也都没有在文件里表达复数选择：那活在 `trans_choice` 那种字符串内部的管道加范围约定里。Fluent 给了我们服务器和客户端都能解析的同一种格式，这正是让“前端展示的字符串和验证器产出的一样”成为设计的一项属性、而不是一条您需要自己维护的约定的原因。它的代价是要学一套新语法（这一章大部分内容都在讲它）外加一次工具链的变动：Poedit 编辑不了 `.ftl`，而 Crowdin、Weblate、Lokalise 和 Pontoon 可以。它还要付出点号命名空间的代价 - `trans('messages.welcome')` 没有对应物，因为 id 在每个语言区域内是一个扁平的命名空间。改用前缀代替。

**没有 `trans_choice`。** Laravel 用管道分隔的字符串和显式的范围来选择一个复数形式：

```php
// Laravel
trans_choice('{1} plik|[2,4] pliki|[5,*] plików', $count);
```

现在用波兰语数到 22。CLDR 把 22 归到 `few` 类别 - `22 pliki` - 但 `[5,*]` 把它吞掉了，产出的是 `22 plików`。同样的断裂发生在 32、42、102，以及俄语、阿拉伯语、捷克语、立陶宛语和威尔士语里，每种语言各有各的断裂点。整数范围没法表达复数规则，因为复数规则关心的不是范围；它们关心的是最后一位数字、最后两位数字，在某些语言里，还关心这个值到底是不是一个整数。Fluent 直接依据 CLDR 类别做选择，所以 `$count` 就是一个普通的参数，而*译者* - 那个懂这门语言的人 - 会把波兰语的全部四个类别都写出来：

```ftl
files =
    { $count ->
        [one] { $count } plik
        [few] { $count } pliki
        [many] { $count } plików
       *[other] { $count } pliku
    }
```

`one` 是 1；`few` 是 2-4、22-24、32-34、102-104；`many` 是 0、5-21、25-31；`other` 接住那些分数（`1,5 pliku`），并按上面那条规则携带默认标记。

Laravel 那种无范围的形式（`plik|pliki|plików`）做得更好一些 - 它查阅一份逐语言的索引，挑出第 *n* 段 - 但那份索引是一张手工维护的表，不是 CLDR 数据，它给波兰语提供三段，而 CLDR 定义了四个类别，这些段是按位置排的，没有类别名字可供审阅，并且它永远只能依据计数来选择。

这就是白得的第二个好处：一个 Fluent 选择器可以依据*任何*参数切换，不只是一个计数。性别、方案档位、连接状态都可以用同样的方式来选择，没有一个需要一个新的门面方法。

**隔离标记默认是关闭的。** Fluent 通常会把每一处插值都包进 U+2068（FIRST STRONG ISOLATE）和 U+2069（POP DIRECTIONAL ISOLATE）里，好让一个嵌在从左到右句子里的从右到左的值，能按正确的顺序渲染。这是对的 - 但也是不可见的，这意味着一个纯英语应用里的每一个 `assert_eq!("Hello Ada", …)`，都会因为 diff 里两个谁也看不见的字符而失败。我们默认把它们关闭，并让打开它们只需要一次调用：

```rust
let config = LocalizationConfig::from_env()?.use_isolating(true);
```

**当您发布一个 RTL 语言区域时打开它们** - 阿拉伯语、希伯来语、波斯语、乌尔都语 - 或者任何用户提供的值会在一句话内部混用不同文字系统的语言区域。然后把您的断言更新成去比较带着这些标记的字符串，或者在断言辅助函数里把它们剥掉。默认值是为常见情形优化的；正确的情形只差一行，而这一段就是提醒您去补上这一行的地方。

## 下一步

- [验证](validation.md) - 规则、`validate!` 宏，以及 `ValidationMessage` 是从哪里来的
- [TypeScript 类型](frontend-typescript-types.md) - `generate-types`、`inertia-props.ts` 和 `lang-keys.ts`
- [中间件](middleware.md) - 把 `LocaleMiddleware` 相对全局链其余部分排序
- [会话](session.md) - 第一个检测步骤读取的那个存储
- [环境变量](env-vars.md) - `APP_LOCALE`、`APP_FALLBACK_LOCALE`、`APP_LOCALE_PARENTS`、`APP_BASE_PATH`
- [测试](testing.md) - `TestContainer`、`#[suprnova_test]`，以及密封的 DI 覆盖
