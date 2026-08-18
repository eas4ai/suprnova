# 请求

Suprnova 的处理程序接收的要么是一个 `Request` - 也就是网络层面上的那个 HTTP 请求 - 要么是一个类型化的表单请求结构体，它会在您的代码运行之前解析、验证并授权请求体。两条路径都挂在同一个 `#[handler]` 宏上；具体用哪一种形态，由您逐路由地选择。本章两者都会讲到，另外还有 multipart 上传的提取器，以及您在中间件里会用到的那些原始访问器。

## 类型化表单请求

`#[request]` 属性把一个结构体标记为 `FormRequest`。这个宏会加上 `serde::Deserialize` 和 `validator::Validate` 两个 derive，并生成一个 `impl FormRequest`，这样 `#[handler]` 宏就知道要在请求进来的路上把它提取出来并做验证：

```rust
use suprnova::request;

#[request]
pub struct CreateUserRequest {
    #[validate(email(message = "Please provide a valid email address"))]
    pub email: String,

    #[validate(length(min = 8, message = "Password must be at least 8 characters"))]
    pub password: String,

    #[validate(length(min = 1, max = 100, message = "Name is required"))]
    pub name: String,
}
```

一个把这个类型写作参数的处理程序，拿到的是一个已经验证过的值：

```rust
use suprnova::{handler, json_response, Response};
use crate::requests::CreateUserRequest;

#[handler]
pub async fn store(form: CreateUserRequest) -> Response {
    // `form` 已经过验证 - 只有每一条规则都通过了，这段代码才会运行。
    json_response!({ "email": form.email, "name": form.name })
}
```

而一个把 `Request` 写作参数的处理程序，拿到的是原样传过来的原始请求：

```rust
use suprnova::{handler, json_response, Request, Response};

#[handler]
pub async fn index(req: Request) -> Response {
    json_response!({ "path": req.path() })
}
```

两者都是提取器 - `#[handler]` 宏会为每一个参数类型去查找 `FromRequest::from_request`，而任何实现了 `FormRequest` 的结构体都会免费得到一个通用的 `FromRequest` 实现。

## 验证规则

验证是通过 `validator` crate 来跑的。常见的规则有：

### 字符串验证

```rust
#[request]
pub struct ExampleRequest {
    // 必填（非空）
    #[validate(length(min = 1, message = "This field is required"))]
    pub name: String,

    // 邮箱格式
    #[validate(email(message = "Invalid email address"))]
    pub email: String,

    // URL 格式
    #[validate(url(message = "Invalid URL"))]
    pub website: String,

    // 长度约束
    #[validate(length(min = 8, max = 100))]
    pub password: String,

    // 正则模式 - PHONE_REGEX 必须是一个 `static` 或 `const`，
    // 并且在验证器的展开点可见。只需声明一次，
    // 通常就放在同一个模块里：
    #[validate(regex(path = "PHONE_REGEX", message = "Invalid phone number"))]
    pub phone: String,
}

use std::sync::LazyLock;
use regex::Regex;

// validator 0.20 为 `std::sync::LazyLock<Regex>` 实现了 `AsRegex`，
// 但没有为 `once_cell::sync::Lazy<Regex>` 实现 - 请使用 std 的这个类型，
// 这样 derive 展开出来的 `#[validate(regex(path = "..."))]` 才能通过类型检查。
static PHONE_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\+?[0-9\s\-()]{7,20}$").unwrap());
```

### 数值验证

```rust
#[request]
pub struct ProductRequest {
    // 范围验证 - 字面量必须与字段类型匹配。`f64`
    // 接受 `0.0` / `10000.0`，而不是整数字面量 `0` / `10000`。
    #[validate(range(min = 0.0, max = 10000.0, message = "Price must be between 0 and 10000"))]
    pub price: f64,

    // 最小值
    #[validate(range(min = 1))]
    pub quantity: i32,

    // 最大值
    #[validate(range(max = 100))]
    pub discount_percent: i32,
}
```

### 嵌套与集合验证

```rust
use serde::Deserialize;

#[derive(Deserialize, Validate)]
pub struct Address {
    #[validate(length(min = 1))]
    pub street: String,

    #[validate(length(min = 1))]
    pub city: String,
}

#[request]
pub struct OrderRequest {
    // 嵌套结构体验证
    #[validate(nested)]
    pub shipping_address: Address,

    // 集合长度
    #[validate(length(min = 1, message = "At least one item required"))]
    pub items: Vec<String>,
}
```

### 常用的验证属性

| 属性 | 说明 | 示例 |
|-----------|-------------|---------|
| `email` | 合法的邮箱格式 | `#[validate(email)]` |
| `url` | 合法的 URL 格式 | `#[validate(url)]` |
| `length` | 字符串 / 集合的长度 | `#[validate(length(min = 1, max = 100))]` |
| `range` | 数值范围 | `#[validate(range(min = 0, max = 100))]` |
| `regex` | 正则模式匹配 | `#[validate(regex(path = "PATTERN"))]` |
| `contains` | 字符串包含某个子串 | `#[validate(contains(pattern = "@"))]` |
| `does_not_contain` | 字符串不包含某个子串 | `#[validate(does_not_contain(pattern = "admin"))]` |
| `nested` | 验证嵌套的结构体 | `#[validate(nested)]` |

## 验证错误响应

当验证失败时，Suprnova 会返回一个 422 响应，带着与 Laravel / Inertia 兼容的错误包：

```json
HTTP 422 Unprocessable Entity

{
    "message": "The given data was invalid.",
    "errors": {
        "email": ["Please provide a valid email address"],
        "password": ["Password must be at least 8 characters"]
    }
}
```

`errors` 的形状，正好就是 `@inertiajs/*` 客户端从 `usePage().props.errors` 直接读到的那个形状。

### 嵌套字段

一次 `#[validate(nested)]` 失败，会被报告在一个点分的键下面，这个键点明了完整路径，用的是和 Laravel 一样的记法。一个嵌套结构体贡献 `parent.field`；一个被验证的 `Vec<T>` 的某个元素贡献 `parent.<index>.field`：

```json
{
    "message": "The given data was invalid.",
    "errors": {
        "shipping_address.street": ["Validation failed for field 'shipping_address.street'"],
        "items.1.name": ["Validation failed for field 'items.1.name'"]
    }
}
```

下标 `1` 指的是第二个元素 - 第一个元素通过了验证，所以不在这个包里。在客户端可以直接把这个键原样绑定过去：`form.errors['items.1.name']`。

## 完整示例

一个用户注册端点，从头到尾。

**定义这个请求：**

```rust
// src/requests/create_user.rs
use suprnova::request;

#[request]
pub struct CreateUserRequest {
    #[validate(email(message = "Please provide a valid email address"))]
    pub email: String,

    #[validate(length(min = 8, message = "Password must be at least 8 characters"))]
    pub password: String,

    #[validate(length(min = 2, max = 50, message = "Name must be between 2 and 50 characters"))]
    pub name: String,
}
```

**创建控制器：**

```rust
// src/controllers/user.rs
use suprnova::{handler, json_response, Request, Response, ResponseExt};
use crate::requests::CreateUserRequest;

#[handler]
pub async fn index(_req: Request) -> Response {
    json_response!({ "users": [] })
}

#[handler]
pub async fn store(form: CreateUserRequest) -> Response {
    // 验证已通过 - 创建这个用户
    // 在真实的应用里，您会在这里写进数据库

    json_response!({
        "user": {
            "email": form.email,
            "name": form.name
        },
        "message": "User created successfully"
    })
    .status(201)
}
```

**注册路由：**

```rust
// src/routes.rs
use suprnova::{get, post, routes};
use crate::controllers;

routes! {
    get!("/users", controllers::user::index).name("users.index"),
    post!("/users", controllers::user::store).name("users.store"),
}
```

## 授权与跨字段钩子

`FormRequest` trait 暴露了三个生命周期钩子：`authorize`、`after_validation` 和 `after_validation_async`。`#[request]` 属性和 `#[derive(FormRequestDerive)]` 这两种写法都会为您生成一个默认的 `impl FormRequest`。要覆盖其中任何一个钩子，请加上 `#[form_request(custom_hooks)]` 这个选择退出标记来抑制默认实现，然后写您自己的。（这与 `#[multipart(custom_hooks)]` 的模式是一致的。）

```rust
use suprnova::{FormRequest, FormRequestDerive, Request};
use serde::Deserialize;
use validator::Validate;

#[derive(Deserialize, Validate, FormRequestDerive)]
#[form_request(custom_hooks)]
pub struct DeleteUserRequest {
    pub user_id: i64,
}

impl FormRequest for DeleteUserRequest {
    fn authorize(req: &Request) -> bool {
        // 返回 false 就会在请求体被读取之前，
        // 用一个 403 Forbidden 短路掉。
        req.header("X-Admin-Token").is_some()
    }
}
```

这个选择退出标记在 `#[request]` 属性的写法下同样有效 - 当您既想要这个属性带来的自动 derive、又需要覆盖钩子时，它很有用：

```rust
use suprnova::{FormRequest, Request, request};

#[request]
#[form_request(custom_hooks)]
pub struct DeleteUserRequestAttr {
    pub user_id: i64,
}

impl FormRequest for DeleteUserRequestAttr {
    fn authorize(req: &Request) -> bool {
        req.header("X-Admin-Token").is_some()
    }
}
```

当 `authorize` 返回 `false` 时，提取会返回 `FrameworkError::Unauthorized`，并渲染出：

```json
HTTP 403 Forbidden

{ "message": "This action is unauthorized." }
```

`after_validation` 是同步的跨字段钩子 - 用它来表达“密码与确认密码必须一致”这类规则。`after_validation_async` 是它的异步对应版本，也是那些依赖数据库的规则（比如内置的 `Unique`）参与自动验证的地方。两者都在逐字段的 `validator` 规则通过之后才触发；`extract` 会在第一个失败的阶段就退出。

```rust
use suprnova::{FormRequest, FormRequestDerive, ValidationErrors};
use serde::Deserialize;
use validator::Validate;

#[derive(Deserialize, Validate, FormRequestDerive)]
#[form_request(custom_hooks)]
pub struct UpdatePasswordRequest {
    #[validate(length(min = 8))]
    pub new_password: String,
    pub confirmation: String,
}

impl FormRequest for UpdatePasswordRequest {
    fn after_validation(&self) -> Result<(), ValidationErrors> {
        if self.new_password != self.confirmation {
            let mut errs = ValidationErrors::new();
            errs.add("confirmation", "passwords do not match");
            return Err(errs);
        }
        Ok(())
    }
}
```

### 请求体大小上限

逐结构体的 `#[form_request(max_body_bytes = N)]` 属性会在单个 FormRequest 上覆盖进程级全局的 8 MiB 上限：

```rust
use suprnova::FormRequestDerive;
use serde::Deserialize;
use validator::Validate;

#[derive(Deserialize, Validate, FormRequestDerive)]
#[form_request(max_body_bytes = 64 * 1024 * 1024)] // 64 MiB
pub struct ImportPayload {
    pub rows: Vec<Row>,
}

#[derive(Deserialize, Validate)]
pub struct Row { /* ... */ }
```

`Content-Length` 会被预先解析，当声明的大小超过上限时，请求会在读取任何一个请求体字节*之前*就被以 HTTP 413 拒绝；而那些在 `Content-Length` 上撒谎的客户端，仍然会在读取过程中触发流式的字节计数器。

## 内容类型检测

`FormRequest::extract` 只看 `Content-Type` 请求头：

- `application/x-www-form-urlencoded` → 通过 `serde_urlencoded` 解析
- `application/json`，或者任何 `application/*+json` 后缀 → 通过 `serde_json` 解析
- 其他任何东西（包括请求头缺失） → 在读取请求体之前就以 HTTP 415 Unsupported Media Type 拒绝

关于 multipart 请求体（`multipart/form-data`），请参见下面的[文件上传](#文件上传-multipartrequest)。

## 直接读取请求体

对于一次性的端点，或者不想要一个完整 `FormRequest` 的中间件，`Request` 类型本身提供了三种读取请求体的方式 - 每一种都会消费 `self`，因为请求体最多只能被读取一次：

```rust
use serde::Deserialize;
use suprnova::{handler, json_response, Request, Response};

#[derive(Deserialize)]
struct LoginForm { username: String, password: String }

#[handler]
pub async fn login(req: Request) -> Response {
    // 显式挑选解析器。
    let form: LoginForm = req.form().await?;
    json_response!({ "user": form.username })
}

#[handler]
pub async fn webhook(req: Request) -> Response {
    // 同样的形态，只不过传输的是 JSON。
    let payload: serde_json::Value = req.json().await?;
    json_response!({ "received": payload })
}

#[handler]
pub async fn ingest(req: Request) -> Response {
    // 根据 Content-Type 自动挑选 - 除非显式写明了
    // `application/x-www-form-urlencoded`，否则按 JSON 处理。
    let value: serde_json::Value = req.input().await?;
    json_response!({ "value": value })
}
```

要做原始访问，`req.body_bytes().await` 会返回缓冲好的 `Bytes` 以及 `RequestParts` 元数据（路由参数和内容类型）。用 `body_bytes_with_cap(n)` 可以逐个场景地覆盖全局的 8 MiB 上限。

## 与表单一起解析服务

已验证的表单请求可以和[服务容器](container.md)组合使用。在处理程序内部使用 `App::resolve::<T>()`（或者 `App::get::<T>()`）：

```rust
use suprnova::{handler, json_response, Response, App};
use crate::requests::CreateUserRequest;
use crate::services::UserService;

#[handler]
pub async fn store(form: CreateUserRequest) -> Response {
    let user_service = App::resolve::<UserService>()?;
    let user = user_service.create_user(&form.email, &form.name).await?;
    json_response!({ "user": user })
}
```

## 文件上传（`MultipartRequest`）

`multipart/form-data` 有它自己的提取器 - `#[derive(MultipartRequest)]` 会一段一段地流式读取请求体，把超过配置阈值的大文件分段溢写到临时文件里，这样一次 200 MiB 的上传永远不会整个待在内存里。每个字段都带一个 `#[field("name")]` 注解，用来指明它在传输中的字段名；文件字段使用 `UploadedFile<V>`，其中 `V` 是来自 `suprnova::http::upload::validators` 的一个验证器（或者一组验证器构成的元组）。

```rust
use suprnova::{handler, json_response, MultipartRequest, Response};
use suprnova::http::upload::UploadedFile;
use suprnova::http::upload::validators::{Image, MaxSize};

#[derive(MultipartRequest)]
pub struct AvatarUpload {
    #[field("avatar")]
    pub avatar: UploadedFile<(Image, MaxSize<5_242_880>)>, // 5 MiB 上限
    #[field("caption")]
    pub caption: Option<String>,
}

#[handler]
pub async fn upload_avatar(form: AvatarUpload) -> Response {
    // `avatar` 视大小而定，可能在内存里，也可能在一个临时文件里。
    // `.bytes()` 两种都能读；`.store_as(...)` 会流式写入一个磁盘。
    let bytes = form.avatar.bytes().await?;
    json_response!({ "size": bytes.len(), "caption": form.caption })
}
```

字段形态：

| 声明 | 传输中的形态 |
|---|---|
| `UploadedFile<V>` | 必填的文件 |
| `Option<UploadedFile<V>>` | 可选的文件 |
| `Vec<UploadedFile<V>>` | 数组上传（`photos[]`） |
| `String` / `u32` / 任何 `FromStr` | 文本字段（必填） |
| `Option<String>` / `Option<T: FromStr>` | 可选的文本字段 |
| `Vec<String>` / `Vec<T: FromStr>` | 重复出现的文本字段 |

`suprnova::http::upload::validators` 里的内置验证器：

- `MaxSize<N>` - 当累计总量超过 `N` 字节时，就在那个字节边界上短路（HTTP 413）。
- `Image` - 拒绝那些魔数字节没有声称自己是 `image/*` 的分段。
- `MimeType<L>` - 接受一份由您自己的 `MimeAllowlist` 类型提供的固定允许列表。
- `()` - 空操作；`UploadedFile<()>` 接受任意字节。

验证器以元组的形式组合起来：`(Image, MaxSize<5_242_880>)` 会把两个都跑一遍，并在第一个失败处短路。

### 逐字段上限与数组数量上限

针对整个请求体的字节上限是全局的（multipart 默认 8 MiB，可以通过 `suprnova::http::upload::set_global_max_multipart_body_bytes` 配置）。逐字段的上限用来防止这样一种滥用：一个由许多小分段组成的请求体，在字节预算之内把 `Vec<UploadedFile<_>>` 撑到没有边界：

```rust
#[derive(MultipartRequest)]
pub struct Gallery {
    #[field("photos", max_count = 8)]
    pub photos: Vec<UploadedFile<MaxSize<1_048_576>>>,
}
```

同名的第（`max_count` + 1）个分段会在分配之前就返回 HTTP 422，所以多出来的那个分段根本走不到 `Vec` 扩容那一步。

### 授权钩子与验证后钩子

`MultipartRequest` 通过 `MultipartRequestHooks` trait 镜照了 `FormRequest` 的那些钩子。默认情况下这个 derive 会生成一个空实现；用 `#[multipart(custom_hooks)]` 来选择启用您自己的实现：

```rust
use suprnova::{MultipartRequest, Request, ValidationErrors};
use suprnova::http::upload::{MultipartRequestHooks, UploadedFile};

#[derive(MultipartRequest)]
#[multipart(custom_hooks)]
pub struct GuardedUpload {
    #[field("file")]
    pub file: UploadedFile,
}

impl MultipartRequestHooks for GuardedUpload {
    fn authorize(req: &Request) -> bool {
        req.header("X-Admin-Token").is_some()
    }

    fn after_validation(&self) -> Result<(), ValidationErrors> {
        if self.file.size == 0 {
            let mut errs = ValidationErrors::new();
            errs.add("file", "empty file");
            return Err(errs);
        }
        Ok(())
    }
}
```

### 流式写入存储

`UploadedFile::store_as` 会把这个分段写入一个已注册的存储磁盘。对于落在磁盘上的分段，整条路径都是完全流式的（通过 `opendal::Operator::writer` 以 64 KiB 为一块）；在内存里的分段则用一次写调用完成。当存储路径是按内容寻址的时候，请使用从内容推导出来的扩展名 - 文件名那个请求头是不可信的：

```rust
use suprnova::Storage;

let disk = Storage::disk("avatars")?;
let path = format!("{}.{}", user.id, form.avatar.extension_from_magic());
form.avatar.store_as(&disk, &path).await?;
```

存储磁盘的注册表请参见[文件系统和存储](filesystem.md)。

## 文件组织

请求的标准结构是这样的：

```
src/
├── requests/
│   ├── mod.rs                 # 重导出所有请求
│   ├── create_user.rs         # CreateUserRequest
│   ├── update_user.rs         # UpdateUserRequest
│   └── create_post.rs         # CreatePostRequest
├── controllers/
│   └── user.rs                # 使用 CreateUserRequest
└── routes.rs
```

**src/requests/mod.rs：**
```rust
pub mod create_user;
pub mod update_user;

pub use create_user::CreateUserRequest;
pub use update_user::UpdateUserRequest;
```

## 与 Inertia 的端到端类型安全

请求还可以 derive `InertiaProps` 来生成 TypeScript 类型，从而实现从您的 Rust 后端到 React 前端的端到端类型安全。

### 为请求生成 TypeScript 类型

在 `#[request]` 旁边加上 `InertiaProps` derive：

```rust
use suprnova::{request, InertiaProps};

#[request]
#[derive(InertiaProps)]
pub struct CreateTodoRequest {
    #[validate(length(min = 1, message = "Title is required"))]
    pub title: String,

    #[validate(length(max = 500))]
    pub description: Option<String>,
}
```

运行类型生成：

```bash
suprnova generate-types
```

这会在 `frontend/src/types/inertia-props.ts` 里生成 TypeScript 类型：

```typescript
export interface CreateTodoRequest {
  title: string
  description: string | null
}
```

### 用 Inertia 编写类型安全的表单

用 Inertia 的 `<Form>` 组件可以得到最干净的表单处理方式：

```tsx
import { Form, usePage } from '@inertiajs/react'

export default function CreateTodo() {
  const { errors } = usePage().props

  return (
    <Form action="/todos" method="post">
      <input
        type="text"
        name="title"
        placeholder="Todo title"
      />
      {errors?.title && <span className="error">{errors.title}</span>}

      <textarea
        name="description"
        placeholder="Description (optional)"
      />

      <button type="submit">Create Todo</button>
    </Form>
  )
}
```

如果想要更多控制，可以把 `<Form>` 与 `useForm` 钩子以及您生成出来的类型结合起来：

```tsx
import { Form, useForm } from '@inertiajs/react'
import type { CreateTodoRequest } from '../types/inertia-props'

export default function CreateTodo() {
  const { data, setData, errors, processing } = useForm<CreateTodoRequest>({
    title: '',
    description: null,
  })

  return (
    <Form action="/todos" method="post">
      {({ processing }) => (
        <>
          <input
            type="text"
            name="title"
            value={data.title}
            onChange={(e) => setData('title', e.target.value)}
            placeholder="Todo title"
          />
          {errors.title && <span className="error">{errors.title}</span>}

          <textarea
            name="description"
            value={data.description || ''}
            onChange={(e) => setData('description', e.target.value || null)}
            placeholder="Description (optional)"
          />

          <button type="submit" disabled={processing}>
            Create Todo
          </button>
        </>
      )}
    </Form>
  )
}
```

### 这个 derive 给您带来了什么

- TypeScript 会在编译期捕获字段名的拼写错误和类型不匹配。
- IDE 的自动补全直接读取生成出来的 `.ts`。
- 在 Rust 里重命名一个字段，重新运行 `suprnova generate-types`，TypeScript 那一侧的接口就会跟着走。

完整的生成流程请参见 [TypeScript 类型](frontend-typescript-types.md)。

## 请求访问器

在上面那种“已验证表单”的模式之外，`Request` 类型还带有一套 Laravel 风格的访问器，用来检视网络层面上的那个请求 - URL、请求头、查询字符串、内容协商、路由元数据以及客户端 IP。它们在中间件里很有用，在那些既想要原始访问、又同时用着 `FormRequest` 的处理程序里也很有用，在任何“经过验证的解析”并不是合适工具的地方同样有用。

### URL 与路径

| 方法 | 返回值 | 说明 |
|--------|---------|-------|
| `req.path()` | `&str` | 原始的 URI 路径。 |
| `req.decoded_path()` | `String` | 解开了百分号转义之后的路径。 |
| `req.segments()` | `Vec<String>` | 按 `/` 切分后的路径，空的段会被丢掉。 |
| `req.segment(index, default)` | `Option<String>` | 从 1 开始计数的段访问。 |
| `req.url()` | `String` | 协议 + 主机 + 路径（不含查询字符串）。 |
| `req.full_url()` | `String` | URL + 查询字符串。 |
| `req.full_url_with_query(&[("k","v")])` | `String` | 追加或覆盖查询键。 |
| `req.full_url_without_query(&["k"])` | `String` | 去掉指定的查询键。 |

```rust
use suprnova::{handler, json_response, Request, Response};

#[handler]
pub async fn show(req: Request) -> Response {
    if req.is(&["admin/*"]) {
        // 路径匹配 admin/* 这个通配符
    }
    json_response!({ "url": req.full_url() })
}
```

### 主机、协议、IP

| 方法 | 返回值 | 来源顺序 |
|--------|---------|--------------|
| `req.host()` | `Option<String>` | `X-Forwarded-Host` → `Host` → URI 里的主机部分。 |
| `req.http_host()` | `Option<String>` | 主机，端口非默认时再带上端口。 |
| `req.scheme_and_http_host()` | `Option<String>` | `scheme://host:port`。 |
| `req.scheme()` | `&'static str` | [`secure`] 为 true 时是 `"https"`，否则是 `"http"`。 |
| `req.secure()` | `bool` | URI 的协议 → `X-Forwarded-Proto` → `X-Forwarded-Ssl: on`。 |
| `req.ip()` | `Option<String>` | `X-Forwarded-For[0]` → `X-Real-IP` → 对端地址。 |
| `req.ips()` | `Vec<String>` | 完整的链条：先是代理请求头，然后是对端地址。 |
| `req.user_agent()` | `Option<&str>` | `User-Agent` 请求头。 |
| `req.port()` | `Option<u16>` | Host 请求头里的端口 → `X-Forwarded-Port` → URI 里的端口。 |

### 请求头与方法

| 方法 | 返回值 |
|--------|---------|
| `req.has_header("X-Foo")` | `bool` |
| `req.bearer_token()` | `Option<String>`（最后一段 `Bearer ` 子串，已去掉逗号） |
| `req.is_method("POST")` | `bool`（大小写不敏感） |
| `req.ajax()` | `X-Requested-With: XMLHttpRequest` |
| `req.pjax()` | 取值为真的 `X-PJAX` 请求头 |
| `req.prefetch()` | `X-Moz`、`Purpose` 或 `Sec-Purpose` = `prefetch` |

### 内容协商

```rust
if req.is_json() { /* Content-Type 里带有 /json 或 +json */ }
if req.expects_json() { /* AJAX 且 Accept 没有收窄，或者 Accept 更偏好 JSON */ }
if req.wants_json() { /* Accept 请求头里排在最前的是 JSON */ }
if req.accepts_html() { /* Accept 允许 text/html */ }

let preferred = req.prefers(&["application/json", "text/html"]);
let acceptable = req.acceptable_content_types();
```

`accepts(&[ty])` 既匹配裸的类型，也匹配 `application/<vendor>+json` 这种后缀形式。当没有 Accept 请求头、或者排在最前的偏好是 `*/*` 时，`accepts_any_content_type()` 返回 true。

### 查询字符串

```rust
let id: Option<String> = req.query_param("id");
let present: bool = req.has_query("id");
let map = req.query_params(); // HashMap<String, String>

// 通过 serde 做类型化的查询解析
#[derive(serde::Deserialize)]
struct SearchQuery { page: u32, q: String }
let q: SearchQuery = req.query_into()?;
```

### 路由元数据

在路由器分派完一个请求之后，匹配到的模式会被记录在这个请求上：

```rust
if req.route_is(&["users.show", "users.*"]) {
    // 我们正处在 users.show 或 users.* 这条路由里
}

let pattern = req.route_pattern(); // Some("/users/{id}")
let name = req.route_name();       // Some("users.show")
```

`route_is(&[...])` 接受 `*` 通配符（Laravel 的 `Str::is` 语义）。

## 提前中止

如果想要提前退出式的错误处理，又不想套上完整的 `Response` 外壳，`abort_with` / `abort_if` / `abort_unless` 这几个辅助函数会返回一个 `FrameworkError`，它会经由标准的 `From<FrameworkError> for HttpResponse` 流程被渲染出来。它们可以直接和 `?` 组合起来用：

```rust
use suprnova::{abort_if, abort_unless, abort_with, handler, json_response, Request, Response};

#[handler]
pub async fn show(req: Request) -> Response {
    let id = req.param("id")?;

    // 资源缺失时返回 404。
    abort_if(id == "0", 404, "User not found")?;

    // 调用方未认证时返回 403。
    abort_unless(req.has_header("Authorization"), 403, "Login required")?;

    // 或者无条件地抛出一个状态码：
    if some_condition() {
        return Err(abort_with(418, "I'm a teapot").unwrap_err().into());
    }

    json_response!({ "id": id })
}
```

当条件为 false 时，`abort_if` / `abort_unless` 会返回 `Ok(())`，所以 `?` 会照常往下走。

## 为什么 Suprnova 有所不同

Laravel 暴露了一个同步的、合并过的输入包 - `$req->input('field')`、`$req->all()`、`$req->only(['a','b'])`、`$req->boolean('flag')` - 内容同时取自查询字符串和解析后的请求体。Suprnova 没有提供这套接口。原因是：

- Suprnova 的请求体是一次性消费的，而且是异步的。一个同步的 `all()` 会要求预先把每一个请求体都缓冲下来，只为了满足一个大多数处理程序从来不会调用的方法 - 这里的内存开销和 DoS 暴露面，与 PHP 那种每请求一个进程的生命周期是不一样的。
- 类型化的替代方案（`#[request]` + `FormRequest`）给出了编译期的字段名、验证，以及能感知 content-type 的解析 - 这正是那个无类型的输入包所缺少的安全网。

要检视查询串 / 请求头 / 路由，请使用 `query_param`、`query_into`、`has_query`、`bearer_token`，以及上面那些请求头读取方法。要访问请求体那一侧，请定义一个 `#[request]` 结构体，或者一个 `#[derive(MultipartRequest)]` 提取器。

## 下一步

- [验证](validation.md) - `#[validate(...)]` 背后的规则库，以及那个 422 错误包的形状
- [响应](responses.md) - 从您的处理程序里反向构建出 `HttpResponse` 值，包括流式响应和重定向
- [错误](errors.md) - 建立在“`Response` 就是 `Result<HttpResponse, HttpResponse>`”之上的处理程序模式
- [路由](routing.md) - 注册路由，以及 `req.param("id")` 所读取的那些 `{id}` 参数
- [认证](authentication.md) - `Auth::user_as`、`Auth::attempt`，以及那些从请求中解析出当前用户的认证守卫
- [文件系统和存储](filesystem.md) - 注册那些 `UploadedFile::store_as` 会写入的存储磁盘
