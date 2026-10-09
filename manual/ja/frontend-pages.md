# ページ コンポーネント

ページとは、Inertiaがレスポンスとして送り出す単位です。Rustのコントローラーは、コンポーネント名と型付きのプロップ構造体を選びます。Viteでバンドルされたフロントエンドは、その名前を`frontend/src/pages/`の中のファイルへ解決し、プロップを引数としてそれをレンダリングします。フレームワークはフレームワーク不問です - Suprnovaは、Svelte 5、React 19、Vue 3.5のためのファーストクラスのスターターを出荷しており、ページの契約は3つすべてで同じ形をしています。

## 契約

コントローラーは、コンポーネントを名指しするInertiaレスポンスを返します。

```rust
use suprnova::{InertiaProps, Request, Response, inertia_response};

#[derive(InertiaProps)]
pub struct HomeProps {
    pub title: String,
    pub message: String,
}

pub async fn index(req: Request) -> Response {
    inertia_response!(&req, "Home", HomeProps {
        title: "Welcome".to_string(),
        message: "Hello from Suprnova!".to_string(),
    })
}
```

文字列`"Home"`は、`frontend/src/pages/Home.<ext>`に対して解決されます。拡張子は、どのスターターをスキャフォルドしたかによって決まります。

| スターター | 拡張子 | デフォルト？ |
|---|---|---|
| Svelte 5 | `.svelte` | はい |
| React 19 | `.tsx` | - |
| Vue 3.5 | `.vue` | - |

このマクロは、ファイルが存在することをコンパイル時に検証します。そのため、タイプミスや削除されたページは、本番環境で500になるのではなく、`cargo check`で失敗します。

## ディレクトリレイアウト

どのフレームワークを選んでも、ページは`frontend/src/pages/`の下に存在し、`inertia_response!`の中のコンポーネント名は、そのディレクトリからの相対パスであり、拡張子を除いたものです。フォワードスラッシュは、どのプラットフォームでも同じように機能します。

```
frontend/src/pages/
├── Home.svelte                 # inertia_response!(&req, "Home", ...)
├── About.svelte                # inertia_response!(&req, "About", ...)
├── Users/
│   ├── Index.svelte            # inertia_response!(&req, "Users/Index", ...)
│   ├── Show.svelte             # inertia_response!(&req, "Users/Show", ...)
│   └── Edit.svelte             # inertia_response!(&req, "Users/Edit", ...)
├── Posts/
│   ├── Index.svelte            # inertia_response!(&req, "Posts/Index", ...)
│   └── Show.svelte             # inertia_response!(&req, "Posts/Show", ...)
└── auth/
    ├── Login.svelte            # inertia_response!(&req, "auth/Login", ...)
    └── Register.svelte         # inertia_response!(&req, "auth/Register", ...)
```

慣例では、コレクションのページには`Index`を、単一項目のページには`Show` / `Edit` / `Create`を使い、まとまった機能のページには`auth/`のような小文字のサブディレクトリを使います。コンポーネント名の大文字・小文字は、ファイル名と正確に一致していなければなりません - Viteの`import.meta.glob`は大文字・小文字を区別します。

## ページを生成する

CLIの`make:inertia`ジェネレーターは、正しい場所にスターターコンポーネントを配置し、そのプロジェクトが使っているフロントエンドの構文を使います。

```bash
suprnova make:inertia Dashboard
```

ジェネレーターは、あなたの`.env`から`SUPRNOVA_FRONTEND`を読み取り（デフォルトはSvelte）、対応する拡張子を選び、コンポーネント名の末尾にまだ付いていなければ`Page`を付け足します。そのため、上のコマンドは、次のいずれかを作成します。

- `frontend/src/pages/DashboardPage.svelte`
- `frontend/src/pages/DashboardPage.tsx`
- `frontend/src/pages/DashboardPage.vue`

コンソールの出力は、コントローラーへ貼り付けるべき、対応する`inertia_response!`の呼び出しを表示します。

その接尾辞を省いて、名前を自分で決めるには、完全な名前を渡してください。

```bash
suprnova make:inertia DashboardPage   # DashboardPage.<ext>を作成する
```

代わりにRust側で型付きのプロップ構造体を生成するには、`--data`を渡してください。

```bash
suprnova make:inertia Dashboard --data
# `#[derive(Data, Validate)]`を伴う app/src/props/dashboard.rs を作成する
```

## 各スターターにおけるページ

バックエンド側の同じ`inertia_response!(&req, "Home", HomeProps { ... })`は、フロントエンド側のこれらのページファイルのいずれかに対応します。プロップは、生成される`inertia-props.ts`の型を介して、型付きの引数として届きます。

### Svelte 5

runes-onです。プロップは`$props()`を介して届きます。

```svelte
<!-- frontend/src/pages/Home.svelte -->
<script lang="ts">
  import type { HomeProps } from '../types/inertia-props'

  let { title, message }: HomeProps = $props()
</script>

<div class="font-sans p-8 max-w-xl mx-auto">
  <h1 class="text-3xl font-bold">{title}</h1>
  <p class="mt-2">{message}</p>
</div>
```

### React 19

標準的な関数コンポーネントです。プロップは第一引数として届きます。

```tsx
// frontend/src/pages/Home.tsx
import type { HomeProps } from '../types/inertia-props'

export default function Home({ title, message }: HomeProps) {
  return (
    <div className="font-sans p-8 max-w-xl mx-auto">
      <h1 className="text-3xl font-bold">{title}</h1>
      <p className="mt-2">{message}</p>
    </div>
  )
}
```

### Vue 3.5

`defineProps`を伴う`<script setup lang="ts">`です。プロップはテンプレートの中で直接アクセスされます。

```vue
<!-- frontend/src/pages/Home.vue -->
<script setup lang="ts">
import type { HomeProps } from '../types/inertia-props'

defineProps<HomeProps>()
</script>

<template>
  <div class="font-sans p-8 max-w-xl mx-auto">
    <h1 class="text-3xl font-bold">{{ title }}</h1>
    <p class="mt-2">{{ message }}</p>
  </div>
</template>
```

## ページ間のナビゲーション

各スターターは、それぞれのフレームワーク向けのInertia v3アダプターを出荷します。エクスポートは同じです - 宣言的なナビゲーションのための`Link`、プログラムによるナビゲーションのための`router`、共有プロップのための`usePage`（あるいは`page`）、フォーム処理のための`Form`と`useForm`です。

### Svelte 5

```svelte
<script lang="ts">
  import { Link, router } from '@inertiajs/svelte'

  function gotoPosts() {
    router.visit('/posts')
  }
</script>

<Link href="/posts">All posts</Link>
<Link href="/posts/42" method="delete" as="button">Delete</Link>

<button onclick={gotoPosts}>Visit programmatically</button>
```

### React 19

```tsx
import { Link, router } from '@inertiajs/react'

<Link href="/posts">All posts</Link>
<Link href="/posts/42" method="delete" as="button">Delete</Link>

<button onClick={() => router.visit('/posts')}>Visit programmatically</button>
```

### Vue 3.5

```vue
<script setup lang="ts">
import { Link, router } from '@inertiajs/vue3'
</script>

<template>
  <Link href="/posts">All posts</Link>
  <Link href="/posts/42" method="delete" as="button">Delete</Link>
  <button @click="router.visit('/posts')">Visit programmatically</button>
</template>
```

`router`オブジェクトは、`router.post(url, data)`、`router.put(url, data)`、`router.patch(url, data)`、`router.delete(url)`、`router.reload()`も公開しています - 3つのアダプターすべてで同じ形です。

## フォーム

Inertia v3は、宣言的な`<Form>`コンポーネントと、命令的な`useForm`（Svelteでは`createForm`）ヘルパーを出荷します。どちらも、あなたのRustコントローラーへPOSTで送り返します。バリデーションエラーは、構造化された`errors`プロップとして表に出てきます。

### Svelte 5

```svelte
<!-- frontend/src/pages/Posts/Create.svelte -->
<script lang="ts">
  import { useForm } from '@inertiajs/svelte'

  const form = useForm({
    title: '',
    content: '',
  })

  function submit(e: SubmitEvent) {
    e.preventDefault()
    form.post('/posts')
  }
</script>

<form onsubmit={submit} class="space-y-4">
  <input type="text" bind:value={form.title} placeholder="Title" />
  {#if form.errors.title}
    <p class="text-red-500">{form.errors.title}</p>
  {/if}

  <textarea bind:value={form.content} rows={6}></textarea>

  <button type="submit" disabled={form.processing}>
    {form.processing ? 'Saving…' : 'Create'}
  </button>
</form>
```

### React 19

```tsx
// frontend/src/pages/Posts/Create.tsx
import { useForm } from '@inertiajs/react'

export default function PostCreate() {
  const { data, setData, post, processing, errors } = useForm({
    title: '',
    content: '',
  })

  const submit = (e: React.FormEvent) => {
    e.preventDefault()
    post('/posts')
  }

  return (
    <form onSubmit={submit} className="space-y-4">
      <input
        type="text"
        value={data.title}
        onChange={(e) => setData('title', e.target.value)}
        placeholder="Title"
      />
      {errors.title && <p className="text-red-500">{errors.title}</p>}

      <textarea
        value={data.content}
        onChange={(e) => setData('content', e.target.value)}
        rows={6}
      />

      <button type="submit" disabled={processing}>
        {processing ? 'Saving…' : 'Create'}
      </button>
    </form>
  )
}
```

### Vue 3.5

```vue
<!-- frontend/src/pages/Posts/Create.vue -->
<script setup lang="ts">
import { useForm } from '@inertiajs/vue3'

const form = useForm({
  title: '',
  content: '',
})

function submit() {
  form.post('/posts')
}
</script>

<template>
  <form @submit.prevent="submit" class="space-y-4">
    <input type="text" v-model="form.title" placeholder="Title" />
    <p v-if="form.errors.title" class="text-red-500">{{ form.errors.title }}</p>

    <textarea v-model="form.content" rows="6" />

    <button type="submit" :disabled="form.processing">
      {{ form.processing ? 'Saving…' : 'Create' }}
    </button>
  </form>
</template>
```

### フォームのコールバック

`form.post(url, options)` - そして対応する`.put` / `.patch` / `.delete` - は、標準的な訪問コールバック（`onStart`、`onSuccess`、`onError`、`onFinish`）を受け付けます。あなたのRustハンドラが返すバリデーションエラーは、自動的に`form.errors`に収まります。コールバックは、副作用のためのものです。

```ts
form.post('/posts', {
  onSuccess: async () => { await refreshDrafts() },  // Inertia 3.4以降、awaitされる
  onError: (errors) => console.warn(errors),
  onFinish: () => form.reset('content'),
})
```

Inertia 3.4以降、非同期の`onSuccess`は、送信が決着する前にawaitされます。そのため、コールバックが解決するまで`form.processing`は`true`のままです - 成功した送信がフォローアップ作業を引き起こし、UIにそれを追い越されたくない場合に役立ちます。

## ポーリング

一定間隔で更新されるべきページ - ライブダッシュボード、ジョブのステータス、未読バッジなど - のために、`usePoll`フックはタイマーで部分的なリロードを再発行します。アダプターからそれをインポートします。

```ts
import { usePoll } from '@inertiajs/svelte' // あるいは '@inertiajs/react' / '@inertiajs/vue3'
```

`only`と組み合わせれば、各ティックは変化するプロップだけを取得します - サーバーは、それらのキーだけを解決します（[部分的なリロード](frontend-inertia-responses.md#partial-reloads)を参照）。

```ts
const { stop, start } = usePoll(5000, { only: ['stats', 'jobs'] })
```

`usePoll(interval, requestOptions, options)`:

- **`interval`** - リロードの間隔をミリ秒で指定します。
- **`requestOptions`** - リロードのオプションオブジェクト（`only`、`except`、`data`、`onSuccess`、…）、**あるいはそれを返す関数**です。そのため、リクエストは現在の状態に依存できます（たとえば、ティックごとに進むカーソルなど）。
- **`options.mode`** - 前のリクエストがまだ処理中の間に発火したティックを、どう扱うかです - `'overlap'`（デフォルト - とにかく発火する）、`'cancel'`（処理中のリクエストを中断する）、`'rest'`（このティックをスキップする）。
- **`options.keepAlive`** - タブがバックグラウンドにある間もポーリングを続けます（デフォルトは`false`です - 隠れたタブではポーリングが一時停止します）。
- **`options.autoStart`** - 即座に開始します（デフォルトは`true`です）。`false`を渡し、準備ができたら返り値の`start()`を呼んでください。

このフックは、手動での制御のために`{ stop, start }`を返します。コンポーネントの外では、`@inertiajs/core`の`router.poll(...)`が同じ呼び出しです。

どのティックも普通の部分的なリロードであるため、`only`の下にあるプロップは、他のどのリクエストとも同じLazy / Optional / Deferのリゾルバを通じて流れます - そして、これらのリゾルバは並行して実行されます（`max_concurrent_resolvers`で上限が定められます）。そのため、6つのウィジェットをポーリングするダッシュボードは、ティックごとに6つの直列クエリではなく、6つの並列クエリを発行します。

## 共有プロップ

起動時に共有プロップとして登録したものは何であれ - 典型的には現在のユーザー、フラッシュメッセージ、グローバルなCSRFトークンです - `usePage()`（React、Vue）や、リアクティブな`page`ストア（Svelte）を通じて、すべてのページで利用できます。キーが衝突した場合、ページのプロップが共有プロップを上書きします。

### Svelte 5

```svelte
<script lang="ts">
  import { page } from '@inertiajs/svelte'

  let auth = $derived($page.props.auth as { user?: { name: string } })
</script>

{#if auth.user}
  <span>Welcome, {auth.user.name}</span>
{:else}
  <a href="/login">Log in</a>
{/if}
```

### React 19

```tsx
import { usePage } from '@inertiajs/react'

function Header() {
  const { auth } = usePage<{ auth: { user?: { name: string } } }>().props
  return auth.user ? <span>Welcome, {auth.user.name}</span> : <a href="/login">Log in</a>
}
```

### Vue 3.5

```vue
<script setup lang="ts">
import { usePage } from '@inertiajs/vue3'

const page = usePage<{ auth: { user?: { name: string } } }>()
</script>

<template>
  <span v-if="page.props.auth.user">Welcome, {{ page.props.auth.user.name }}</span>
  <a v-else href="/login">Log in</a>
</template>
```

## レイアウト

レイアウトは、slot / children / テンプレートコンテンツを受け取る、ただの普通のコンポーネントです。Suprnova独自のAPIは特にありません - レイアウトをインポートし、その中にページのコンテンツをレンダリングするだけです。

### Svelte 5

```svelte
<!-- frontend/src/layouts/AppLayout.svelte -->
<script lang="ts">
  import { Link } from '@inertiajs/svelte'
  let { children } = $props()
</script>

<div class="min-h-screen bg-gray-100">
  <nav class="bg-white shadow p-4">
    <Link href="/">Home</Link>
    <Link href="/posts">Posts</Link>
  </nav>
  <main class="max-w-6xl mx-auto py-8">
    {@render children?.()}
  </main>
</div>
```

```svelte
<!-- frontend/src/pages/Posts/Index.svelte -->
<script lang="ts">
  import AppLayout from '../../layouts/AppLayout.svelte'
  import type { PostsIndexProps } from '../../types/inertia-props'

  let { posts }: PostsIndexProps = $props()
</script>

<AppLayout>
  <h1 class="text-2xl font-bold">Posts</h1>
  <ul>
    {#each posts as post (post.id)}
      <li>{post.title}</li>
    {/each}
  </ul>
</AppLayout>
```

### React 19

```tsx
// frontend/src/layouts/AppLayout.tsx
import { Link } from '@inertiajs/react'

export default function AppLayout({ children }: { children: React.ReactNode }) {
  return (
    <div className="min-h-screen bg-gray-100">
      <nav className="bg-white shadow p-4">
        <Link href="/">Home</Link>
        <Link href="/posts">Posts</Link>
      </nav>
      <main className="max-w-6xl mx-auto py-8">{children}</main>
    </div>
  )
}
```

### Vue 3.5

```vue
<!-- frontend/src/layouts/AppLayout.vue -->
<script setup lang="ts">
import { Link } from '@inertiajs/vue3'
</script>

<template>
  <div class="min-h-screen bg-gray-100">
    <nav class="bg-white shadow p-4">
      <Link href="/">Home</Link>
      <Link href="/posts">Posts</Link>
    </nav>
    <main class="max-w-6xl mx-auto py-8">
      <slot />
    </main>
  </div>
</template>
```

## Suprnovaが異なる設計を選んだ理由

LaravelのInertia統合は、一度に1つのフロントエンドを出荷します - インストール時に、1プロジェクトにつき1つのスターターキットとして、React、Vue、Svelteのいずれかを選びます。Suprnovaも、同じ「1プロジェクトにつき1つ」というルールを保っています（混在させません）が、CLIは、同じ`inertia_response!`呼び出しから、3つすべてへ慣用的にスキャフォルドします。Rust側は、どのフロントエンドが動いているかを一切知りません - ジェネレーターとViteのリゾルバが、ディスク上で正しい拡張子を選びます。

もう1つの相違点は、コンパイル時のコンポーネント検証です。Laravelはコンポーネント名を実行時に解決するため、`Inertia::render('Dahsboard')`のようなタイプミスは、本番環境のエラーになります。Suprnovaの`inertia_response!`マクロは、展開時に`frontend/src/pages/`を走査し、"Did you mean 'Dashboard'?"という提案とともに`cargo check`を失敗させます。完全なTypeScriptの型のストーリー（Rustの構造体の`#[derive(InertiaProps)]`から生成されます）は、コンポーネントのプロップもエンドツーエンドで型付けされていることを意味します。

## 次のステップ

- [Inertia レスポンス](frontend-inertia-responses.md) - `inertia_response!`マクロ、部分的なリロード、ディファードプロップ
- [TypeScript 型](frontend-typescript-types.md) - `suprnova generate-types`と型付きプロップのパイプライン
- [フロントエンド 概要](frontend.md) - Inertiaブリッジがどのように組み合わさっているか
- [Inertia CRUD チュートリアル](tutorial-inertia-crud.md) - Postsリソースのエンドツーエンドの全体
- [認証](authentication.md) - スターターがあなたのために生成する認証ページを配線すること
