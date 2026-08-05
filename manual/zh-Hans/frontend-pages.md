# 页面组件

页面就是 Inertia 在网络上传输的那个单位。Rust 控制器挑选一个组件名和一个类型化的 props 结构体；由 Vite 打包的前端，会把这个名字解析到 `frontend/src/pages/` 里的一个文件，并把 props 当作参数来渲染它。这个框架是不挑前端框架的 - Suprnova 为 Svelte 5、React 19 和 Vue 3.5 都提供了一等的起步套件，而这份页面契约在三者之间的形态是一样的。

## 契约

一个控制器返回一个具名了某个组件的 Inertia 响应：

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

这个字符串 `"Home"` 会被解析到 `frontend/src/pages/Home.<ext>`。这个扩展名取决于您脚手架化的是哪一个起步套件：

| 起步套件 | 扩展名 | 默认？ |
|---|---|---|
| Svelte 5 | `.svelte` | 是 |
| React 19 | `.tsx` | - |
| Vue 3.5 | `.vue` | - |

这个宏会在编译期校验这个文件是否存在，所以一次拼写错误或者一个被删掉的页面，会让 `cargo check` 失败，而不是在生产环境里变成一个 500。

## 目录布局

不管您选的是哪个框架，页面都放在 `frontend/src/pages/` 下面，而 `inertia_response!` 里的这个组件名，就是相对于那个目录的文件路径，不带扩展名。正斜杠在所有平台上的行为都是一样的。

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

约定是集合页面用 `Index`，单条目页面用 `Show` / `Edit` / `Create`，而像 `auth/` 这样的小写子目录用于分组的功能页面。组件名里的大小写必须和文件名精确一致 - Vite 的 `import.meta.glob` 是大小写敏感的。

## 生成一个页面

CLI 的 `make:inertia` 生成器会把一个起步组件放到正确的位置，并使用这个项目所用前端对应的语法：

```bash
suprnova make:inertia Dashboard
```

这个生成器会从您的 `.env` 里读取 `SUPRNOVA_FRONTEND`（默认是 Svelte），挑选匹配的扩展名，并在这个组件名后面追加 `Page`（如果它还没有的话）。所以上面这条命令会创建下面这些之一：

- `frontend/src/pages/DashboardPage.svelte`
- `frontend/src/pages/DashboardPage.tsx`
- `frontend/src/pages/DashboardPage.vue`

控制台的输出会打印出您应该粘贴进控制器里的那个匹配的 `inertia_response!` 调用。

要跳过这个后缀，自己掌控这个名字，就传入完整的名字：

```bash
suprnova make:inertia DashboardPage   # 创建 DashboardPage.<ext>
```

如果想在 Rust 那一侧生成一个类型化的 props 结构体，就传入 `--data`：

```bash
suprnova make:inertia Dashboard --data
# 创建 app/src/props/dashboard.rs，带着 #[derive(Data, Validate)]
```

## 每个起步套件里的一个页面

后端同样的这一次 `inertia_response!(&req, "Home", HomeProps { ... })`，会映射到前端这些页面文件之一。Props 会通过生成出来的那个 `inertia-props.ts` 类型，作为类型化的参数到达。

### Svelte 5

已启用 runes 模式。Props 通过 `$props()` 到达：

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

标准的函数组件。Props 作为第一个参数到达：

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

`<script setup lang="ts">` 配合 `defineProps`。Props 在模板里直接访问：

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

## 页面之间的导航

每个起步套件都为它自己的框架配备了 Inertia v3 的适配器。它们导出的东西是一样的：`Link` 用于声明式导航，`router` 用于编程式导航，`usePage`（或者 `page`）用于共享 props，`Form` 和 `useForm` 用于表单处理。

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

这个 `router` 对象还暴露了 `router.post(url, data)`、`router.put(url, data)`、`router.patch(url, data)`、`router.delete(url)` 和 `router.reload()` - 在三个适配器之间形态是一样的。

## 表单

Inertia v3 提供了一个声明式的 `<Form>` 组件，以及一个命令式的 `useForm`（在 Svelte 里是 `createForm`）辅助函数。两者都会向您的 Rust 控制器发出 POST；验证错误会以一个结构化的 `errors` prop 的形式浮现出来。

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

### 表单回调

`form.post(url, options)` - 以及与之对应的 `.put` / `.patch` / `.delete` - 都接受标准的这些访问回调（`onStart`、`onSuccess`、`onError`、`onFinish`）。您 Rust 处理程序返回的验证错误，会自动落到 `form.errors` 里；这些回调是给副作用用的：

```ts
form.post('/posts', {
  onSuccess: async () => { await refreshDrafts() },  // 自 Inertia 3.4 起会被 await
  onError: (errors) => console.warn(errors),
  onFinish: () => form.reset('content'),
})
```

自 Inertia 3.4 起，一个异步的 `onSuccess` 会在这次提交完成之前被 await，所以 `form.processing` 会一直保持 `true`，直到您的回调完成 - 当一次成功的提交会引发您不想让 UI 抢跑过去的后续工作时，这就很好用。

## 轮询

对于一个应该按固定间隔刷新的页面 - 一个实时仪表盘、一个作业状态、一个未读徽标 - `usePoll` 这个钩子会按一个计时器，重新发出一次部分重新加载。从您的适配器里导入它：

```ts
import { usePoll } from '@inertiajs/svelte' // 或者 '@inertiajs/react' / '@inertiajs/vue3'
```

把它和 `only` 搭配起来，这样每一次滴答只会拿取会变化的那些 props - 服务器接下来就只解析那些键（参见[部分重新加载](frontend-inertia-responses.md#partial-reloads)）：

```ts
const { stop, start } = usePoll(5000, { only: ['stats', 'jobs'] })
```

`usePoll(interval, requestOptions, options)`：

- **`interval`** - 两次重新加载之间的毫秒数。
- **`requestOptions`** - 一个重新加载选项对象（`only`、`except`、`data`、`onSuccess`，……），**或者一个返回这样一个对象的函数**，这样这次请求就能依赖当前的状态（比如一个每次滴答都会前进的游标）。
- **`options.mode`** - 当上一次请求仍然在途时，如何处理这一次触发的滴答：`'overlap'`（默认 - 照样发出），`'cancel'`（中止那个在途的请求），或者 `'rest'`（跳过这一次滴答）。
- **`options.keepAlive`** - 在标签页被切到后台时，依然保持轮询（默认 `false`：轮询会在标签页隐藏时暂停）。
- **`options.autoStart`** - 立即开始（默认 `true`）；传入 `false`，并在您准备好的时候，调用返回的那个 `start()`。

这个钩子会返回 `{ stop, start }`，供手动控制。在一个组件之外，来自 `@inertiajs/core` 的 `router.poll(...)` 是同一个调用。

因为每一次滴答都是一次普通的部分重新加载，`only` 下面的这些 props，会像任何其它请求一样，流经同一套 Lazy / Optional / Defer 解析器 - 而这些解析器是并发运行的（受 `max_concurrent_resolvers` 上限约束），所以一个轮询六个组件的仪表盘，每次滴答发出的是六次并行查询，而不是六次串行查询。

## 共享 props

任何您在启动时注册为共享 prop 的东西 - 通常是当前用户、flash 消息，以及全局 CSRF 令牌 - 都可以在每个页面上，通过 `usePage()`（React、Vue）或者那个响应式的 `page` store（Svelte）拿到。在键冲突时，页面 props 会覆盖共享 props。

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

## 布局

一个布局就是一个普通的组件，接受一个 slot / children / template 内容。这里没有什么特殊的 Suprnova API - 您导入一个布局，并把您的页面内容渲染在它里面。

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

## 为什么 Suprnova 有所不同

Laravel 的 Inertia 集成一次只发布一个前端 - 您在安装时选择 React、Vue 或者 Svelte，每个项目一个起步套件。Suprnova 保留了同样的“每个项目一个”规则（您不会混用它们），但 CLI 会从同一个 `inertia_response!` 调用出发，地道地为这三者都生成脚手架。Rust 那一侧永远不知道运行的是哪个前端；生成器和 Vite 解析器会在磁盘上挑出正确的扩展名。

另一处分歧是编译期的组件校验。Laravel 在运行时解析组件名，所以 `Inertia::render('Dahsboard')` 里的一个拼写错误，会变成一个生产环境的错误。Suprnova 的 `inertia_response!` 宏会在展开时遍历 `frontend/src/pages/`，并在 `cargo check` 上失败，给出一条 `Did you mean 'Dashboard'?` 的建议。完整的 TypeScript 类型故事（从 Rust 结构体上的 `#[derive(InertiaProps)]` 生成而来），也意味着这个组件的 props 是端到端类型化的。

## 下一步

- [Inertia 响应](frontend-inertia-responses.md) - `inertia_response!` 宏、部分重新加载、deferred props
- [TypeScript 类型](frontend-typescript-types.md) - `suprnova generate-types` 和这条类型化 props 的流水线
- [前端概览](frontend.md) - Inertia 桥梁是如何整合在一起的
- [Inertia CRUD 教程](tutorial-inertia-crud.md) - 一个端到端的完整 Posts 资源
- [认证](authentication.md) - 接好起步套件为您脚手架出来的那些认证页面
