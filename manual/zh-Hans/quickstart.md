# 快速上手

我们将构建一个小型的“链接”应用程序 - 一个单一页面，列出带标题的 URL，加上一个用来发布新链接的 API 端点。它涉及路由、控制器、Eloquent 模型、迁移和 Inertia 页面。如果您能构建这个，您就能构建 Suprnova 可以做的任何事情。

这假设您已经按照 [安装](installation.md) 章节操作，并且 `suprnova` CLI 在您的 `PATH` 中。

## 1. 脚手架

```bash
suprnova new links --frontend svelte --no-interaction
cd links
suprnova migrate
npm install
suprnova serve
```

打开 `http://127.0.0.1:8765`。您应该看到欢迎页面。停止服务器（`Ctrl+C`）- 我们将添加一个功能。

## 2. 创建模型和迁移

没有专用的 `make:model` 命令 - 一旦迁移运行，模型由 `db:sync --regenerate-models` 从架构中重新生成。从迁移开始：

```bash
suprnova make:migration create_links_table
```

打开 `src/migrations/` 下的新迁移文件：

```rust
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.create_table(
            Table::create()
                .table(Alias::new("links"))
                .if_not_exists()
                .col(ColumnDef::new(Alias::new("id"))
                    .big_integer().primary_key().auto_increment().not_null())
                .col(ColumnDef::new(Alias::new("title")).string().not_null())
                .col(ColumnDef::new(Alias::new("url")).string().not_null())
                .col(ColumnDef::new(Alias::new("created_at"))
                    .timestamp_with_time_zone().not_null().default(Expr::current_timestamp()))
                .col(ColumnDef::new(Alias::new("updated_at"))
                    .timestamp_with_time_zone().not_null().default(Expr::current_timestamp()))
                .to_owned()
        ).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(Alias::new("links")).to_owned()).await
    }
}
```

在 `src/models/link.rs` 处手工创建模型：

```rust
use chrono::{DateTime, Utc};
use suprnova::{model, Model};

#[model(table = "links")]
pub struct Link {
    pub id: i64,
    pub title: String,
    pub url: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

将 `pub mod link;` 添加到 `src/models/mod.rs`，使新模块可访问，然后应用迁移并重新生成实体：

```bash
suprnova db:sync
```

`db:sync` 运行待处理的迁移并重新生成 SeaORM 实体。组合步骤是开发循环的默认值；在生产中，您使用简单的 `suprnova migrate`。

## 3. 添加控制器

`src/controllers/link.rs`：

```rust
use suprnova::{
    Data, InertiaProps, Model, Request, Response,
    handler, inertia_response, json_response,
};
use validator::Validate;
use crate::models::Link;

#[derive(InertiaProps)]
pub struct IndexProps {
    pub links: Vec<Link>,
}

pub async fn index(req: Request) -> Response {
    let links = Link::query().order_by_desc("created_at").get().await?;
    inertia_response!(&req, "Links/Index", IndexProps { links: links.into_vec() })
}

#[derive(Data, Validate)]
pub struct CreateLink {
    #[validate(length(min = 1, max = 200))]
    pub title: String,
    #[validate(url)]
    pub url: String,
}

#[handler]
pub async fn store(input: CreateLink) -> Response {
    let link = Link::create(suprnova::attrs! {
        title: input.title,
        url: input.url,
    }).await?;
    json_response!({ "link": link })
}
```

在 `src/controllers/mod.rs` 中注册控制器模块：

```rust
pub mod link;
```

## 4. 连接路由

`src/routes.rs`：

```rust
use suprnova::{get, post, routes};
use crate::controllers;

routes! {
    get!("/", controllers::home::index).name("home"),

    // Links
    get!("/links", controllers::link::index).name("links.index"),
    post!("/links", controllers::link::store).name("links.store"),
}
```

## 5. 构建 Inertia 页面

创建 `frontend/src/pages/Links/Index.svelte`（用于 Svelte 起步模板）：

```svelte
<script lang="ts">
    import { router } from '@inertiajs/svelte';

    let { links } = $props<{
        links: { id: number; title: string; url: string }[]
    }>();

    let title = $state('');
    let url = $state('');

    function submit(e: SubmitEvent) {
        e.preventDefault();
        router.post('/links', { title, url }, {
            onSuccess: () => { title = ''; url = ''; },
        });
    }
</script>

<div class="mx-auto max-w-2xl p-8">
    <h1 class="text-2xl font-bold">Links</h1>

    <form onsubmit={submit} class="mt-4 flex gap-2">
        <input bind:value={title} placeholder="Title"
               class="flex-1 rounded border p-2" />
        <input bind:value={url} placeholder="https://..."
               class="flex-1 rounded border p-2" />
        <button class="rounded bg-blue-600 px-4 py-2 text-white">Add</button>
    </form>

    <ul class="mt-8 space-y-2">
        {#each links as link}
            <li class="rounded border p-3">
                <a href={link.url} target="_blank"
                   class="text-blue-600 hover:underline">
                    {link.title}
                </a>
                <p class="text-sm text-gray-500">{link.url}</p>
            </li>
        {/each}
    </ul>
</div>
```

（等效的 React 和 Vue 起步模板为您提供相同的结构，使用它们自己的模板 - Inertia 桥接是相同的。）

## 6. 查看其工作

```bash
suprnova serve
```

访问 `http://127.0.0.1:8765/links`。通过表单添加几个链接。它们发送到 `/links`，控制器写入 `links` 表，Inertia 请求重新获取索引 props。没有 JSON 编组粘合剂 - `InertiaProps` 为您推导了线路格式。

## 刚刚发生了什么

您修改了八个文件。以下是它们的实际含义：

| 文件 | 层 | 角色 |
|---|---|---|
| `src/migrations/m_create_links_table.rs` | 架构 | 定义 `links` 表 |
| `src/models/link.rs` | 领域 | 一个结构体，四行，完整的 Eloquent 模型 |
| `src/controllers/link.rs` | HTTP | 两个处理程序：`index`（页面）和 `store`（创建） |
| `src/routes.rs` | 路由器 | 通过 `routes!` 连接 URL 和处理程序 |
| `src/controllers/mod.rs` | 连接 | 重新导出新的控制器模块 |
| `frontend/src/pages/Links/Index.svelte` | 前端 | Inertia 呈现的页面 |
| （现有）`bootstrap.rs` | 启动 | 您在此注册观察器/服务的地方 |
| （现有）`.env` | 配置 | 数据库 URL、端口、秘密 |

这是标准的节奏：迁移 → 模型 → 控制器 → 路由 → 前端页面。每个功能，无论多大，都分解为这些步骤。

## 接下来阅读什么

您已经完成了一个完整的垂直切片。接下来您将需要的东西：

- [路由](routing.md) - 分组、中间件、命名路由、签名 URL、资源路由
- [验证](validation.md) - `#[derive(Validate)]` 能为您提供什么
- [Eloquent](eloquent.md) - 关系、作用域、观察器、软删除、完整查询构造器表面
- [Inertia + 前端](frontend.md) - 部分重新加载、类型化 props、TypeScript 类型生成
- [认证](authentication.md) - 起步模板附带的认证脚手架
- [控制台](console.md) - `cargo run --bin console <subcommand>` 和编写您自己的命令

或者浏览 [`documentation.md`](documentation.md) 了解完整的目录。
