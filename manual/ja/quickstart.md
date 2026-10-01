# クイックスタート

小さな「links」アプリを構築します。URL とタイトルを一覧表示する単一ページと、新しいものをポストするための API エンドポイントを備えています。ルーティング、コントローラー、Eloquent モデル、マイグレーション、Inertia ページを使用します。これを構築できれば、Suprnova が提供するすべてを構築できます。

[インストール](installation.md)に従い、`suprnova` CLI が`PATH`に存在することを前提としています。

## 1. スキャフォルド

```bash
suprnova new links --frontend svelte --no-interaction
cd links
suprnova migrate
npm install
suprnova serve
```

`http://127.0.0.1:8765`を開きます。ウェルカムページが表示されるはずです。サーバーを停止します（`Ctrl+C`）。機能を追加する予定です。

## 2. モデルとマイグレーションを作成する

専用の`make:model`コマンドはありません。マイグレーションが実行されると、モデルはスキーマから`db:sync --regenerate-models`によって再生成されます。マイグレーションから始めます。

```bash
suprnova make:migration create_links_table
```

`src/migrations/`の下の新しいマイグレーションファイルを開きます。

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

`src/models/link.rs`でモデルを手動で作成します。

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

`pub mod link;`を`src/models/mod.rs`に追加して、新しいモジュールに到達できるようにします。次にマイグレーションを適用し、エンティティを再生成します。

```bash
suprnova db:sync
```

`db:sync`は保留中のマイグレーションを実行し、SeaORM エンティティを再生成します。この統合ステップは開発ループのデフォルトです。本番環境では、通常の`suprnova migrate`を使用します。

## 3. コントローラーを追加する

`src/controllers/link.rs`

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

`src/controllers/mod.rs`でコントローラーモジュールを登録します。

```rust
pub mod link;
```

## 4. ルートを接続する

`src/routes.rs`

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

## 5. Inertia ページを構築する

`frontend/src/pages/Links/Index.svelte`を作成します（Svelte スターター用）。

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

（同等の React および Vue スターターは、独自のテンプレートで同じ形状を提供します。Inertia ブリッジは同じです。）

## 6. 動作を確認する

```bash
suprnova serve
```

`http://127.0.0.1:8765/links`にアクセスします。フォームから数個のリンクを追加します。それらは`/links`にポストされ、コントローラーが`links`テーブルに書き込み、Inertia リクエストがインデックスプロップを再取得します。JSON マーシャリングのグルーはありません。`InertiaProps`がレスポンス形式を導出してくれています。

## 何が起きたのか

8 つのファイルに触れました。それぞれが実際に何を意味するかは以下の通りです。

| ファイル | レイヤー | 役割 |
|---|---|---|
| `src/migrations/m_create_links_table.rs` | スキーマ | `links`テーブルを定義 |
| `src/models/link.rs` | ドメイン | 1 つの構造体、4 行、完全な Eloquent モデル |
| `src/controllers/link.rs` | HTTP | 2 つのハンドラ：`index`（ページ）および`store`（作成） |
| `src/routes.rs` | ルーター | `routes!`経由で URL をハンドラに接続 |
| `src/controllers/mod.rs` | ワイリング | 新しいコントローラーモジュールを再エクスポート |
| `frontend/src/pages/Links/Index.svelte` | フロントエンド | Inertia がレンダリングするページ |
| （既存）`bootstrap.rs` | ブート | この機能のオブザーバー/サービスを登録する場所 |
| （既存）`.env` | 設定 | DB URL、ポート、シークレット |

これは標準的なリズムです。マイグレーション → モデル → コントローラー → ルート → フロントエンドページ。すべての機能は、大きさに関わらず、これらのステップに分解されます。

## 次に読むべきもの

完全なバーティカルスライスを完成させました。次に必要になるもの：

- [ルーティング](routing.md) - グループ化、ミドルウェア、名前付きルート、署名付き URL、リソースルーティング
- [バリデーション](validation.md) - `#[derive(Validate)]`が提供するもの
- [Eloquent](eloquent.md) - リレーションシップ、スコープ、オブザーバー、ソフトデリート、クエリビルダー表面全体
- [Inertia + フロントエンド](frontend.md) - 部分的なリロード、型付きプロップ、TypeScript 型生成
- [認証](authentication.md) - スターターが付属していた認証スキャフォルディング
- [コンソール](console.md) - `cargo run --bin console <subcommand>`と独自のコマンドの作成

または[`documentation.md`](documentation.md)を参照して、完全な TOC を確認してください。
