# Componentes de página

Uma página é a unidade que o Inertia envia para o cliente. O controlador
Rust escolhe um nome de componente e um struct de props tipado; o
frontend empacotado pelo Vite resolve esse nome para um arquivo em
`frontend/src/pages/` e o renderiza com as props como argumentos. O
framework é agnóstico quanto ao frontend - o Suprnova traz starters de
primeira classe para Svelte 5, React 19 e Vue 3.5, e o contrato de
página tem o mesmo formato nos três.

## O contrato

Um controlador retorna uma resposta Inertia nomeando um componente:

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

A string `"Home"` é resolvida contra `frontend/src/pages/Home.<ext>`. A
extensão depende de qual starter você criou com scaffold:

| Starter | Extensão | Padrão? |
|---|---|---|
| Svelte 5 | `.svelte` | sim |
| React 19 | `.tsx` | - |
| Vue 3.5 | `.vue` | - |

A macro valida em tempo de compilação que o arquivo existe, então um erro
de digitação ou uma página apagada faz `cargo check` falhar em vez de
retornar 500 em produção.

## Layout de arquivos

Qualquer que seja o framework que você escolheu, as páginas vivem em
`frontend/src/pages/` e o nome do componente em `inertia_response!` é o
caminho do arquivo relativo a esse diretório, sem a extensão. Barras
normais funcionam do mesmo jeito em todas as plataformas.

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

A convenção é `Index` para páginas de coleção, `Show` / `Edit` / `Create`
para páginas de item único, e um subdiretório em minúsculas como `auth/`
para páginas de feature agrupadas. A capitalização no nome do componente
precisa corresponder exatamente ao nome do arquivo - o
`import.meta.glob` do Vite é sensível a maiúsculas e minúsculas.

## Gerando uma página

O gerador `make:inertia` da CLI cria um componente inicial no lugar
certo e usa a sintaxe de qualquer frontend que o projeto esteja usando:

```bash
suprnova make:inertia Dashboard
```

O gerador lê `SUPRNOVA_FRONTEND` do seu `.env` (o padrão é Svelte),
escolhe a extensão correspondente, e acrescenta `Page` ao nome do
componente se ele ainda não estiver lá. Então o comando acima cria um
destes:

- `frontend/src/pages/DashboardPage.svelte`
- `frontend/src/pages/DashboardPage.tsx`
- `frontend/src/pages/DashboardPage.vue`

A saída do console imprime a chamada `inertia_response!` correspondente
que você deve colar no seu controlador.

Para pular o sufixo e ficar com o nome que quiser, passe o nome
completo:

```bash
suprnova make:inertia DashboardPage   # cria DashboardPage.<ext>
```

Para gerar um struct de props tipado no lado Rust em vez disso, passe
`--data`:

```bash
suprnova make:inertia Dashboard --data
# Cria app/src/props/dashboard.rs com #[derive(Data, Validate)]
```

## Uma página em cada starter

O mesmo `inertia_response!(&req, "Home", HomeProps { ... })` no backend
mapeia para um destes arquivos de página no frontend. As props chegam
como argumentos tipados via os tipos gerados de `inertia-props.ts`.

### Svelte 5

Runes ativadas. As props chegam via `$props()`:

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

Componente de função padrão. As props chegam como o primeiro argumento:

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

`<script setup lang="ts">` com `defineProps`. As props são acessadas
diretamente no template:

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

## Navegação entre páginas

Cada starter traz o adaptador Inertia v3 para seu framework. As
exportações são as mesmas: `Link` para navegação declarativa, `router`
para navegação programática, `usePage` (ou `page`) para props
compartilhadas, `Form` e `useForm` para manipulação de formulário.

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

O objeto `router` também expõe `router.post(url, data)`,
`router.put(url, data)`, `router.patch(url, data)`, `router.delete(url)`,
e `router.reload()` - mesmo formato nos três adaptadores.

## Formulários

O Inertia v3 traz um componente declarativo `<Form>` e o helper
imperativo `useForm` (ou `createForm` no Svelte). Ambos fazem POST de
volta para seu controlador Rust; erros de validação aparecem como uma
prop `errors` estruturada.

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

### Callbacks de formulário

`form.post(url, options)` - e os equivalentes `.put` / `.patch` /
`.delete` - aceitam os callbacks de visit padrão (`onStart`,
`onSuccess`, `onError`, `onFinish`). Erros de validação retornados pelo
seu handler Rust caem em `form.errors` automaticamente; os callbacks
são para efeitos colaterais:

```ts
form.post('/posts', {
  onSuccess: async () => { await refreshDrafts() },  // aguardado desde o Inertia 3.4
  onError: (errors) => console.warn(errors),
  onFinish: () => form.reset('content'),
})
```

A partir do Inertia 3.4, um `onSuccess` assíncrono é aguardado antes de
a submissão se resolver, então `form.processing` permanece `true` até
seu callback ser resolvido - útil quando uma submissão bem-sucedida
dispara um trabalho de acompanhamento que você não quer que a UI
adiante.

## Polling

Para uma página que deve se atualizar em um intervalo - um dashboard em
tempo real, o status de um job, um badge de não lidos - o hook `usePoll`
reemite um reload parcial em um timer. Importe-o do seu adaptador:

```ts
import { usePoll } from '@inertiajs/svelte' // ou '@inertiajs/react' / '@inertiajs/vue3'
```

Combine-o com `only` para que cada tick busque só as props que mudam - o
servidor então resolve somente essas chaves (veja
[reloads parciais](frontend-inertia-responses.md#partial-reloads)):

```ts
const { stop, start } = usePoll(5000, { only: ['stats', 'jobs'] })
```

`usePoll(interval, requestOptions, options)`:

- **`interval`** - milissegundos entre reloads.
- **`requestOptions`** - um objeto de opções de reload (`only`,
  `except`, `data`, `onSuccess`, …) **ou uma função que retorna um**,
  para que a solicitação possa depender do estado atual (por exemplo, um
  cursor que avança a cada tick).
- **`options.mode`** - como um tick que dispara enquanto a solicitação
  anterior ainda está em voo é tratado: `'overlap'` (padrão - dispara
  assim mesmo), `'cancel'` (aborta a solicitação em voo), ou `'rest'`
  (pula esse tick).
- **`options.keepAlive`** - continua fazendo polling enquanto a aba está
  em segundo plano (padrão `false`: o polling pausa em uma aba oculta).
- **`options.autoStart`** - começa imediatamente (padrão `true`); passe
  `false` e chame o `start()` retornado quando estiver pronto.

O hook retorna `{ stop, start }` para controle manual. Fora de um
componente, `router.poll(...)` de `@inertiajs/core` é a mesma chamada.

Como todo tick é um reload parcial comum, as props sob `only` passam
pelos mesmos resolvers Lazy / Optional / Defer que qualquer outra
solicitação - e esses resolvers rodam concorrentemente (limitados por
`max_concurrent_resolvers`), então um dashboard fazendo polling de seis
widgets emite seis queries paralelas por tick em vez de seis seriais.

## Props compartilhadas

Qualquer coisa que você registre como prop compartilhada na
inicialização - tipicamente o usuário atual, mensagens flash, e o token
CSRF global - fica disponível em toda página através de `usePage()`
(React, Vue) ou da store reativa `page` (Svelte). Props de página
sobrescrevem props compartilhadas em caso de colisão de chave.

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

## Layouts

Um layout é apenas um componente comum que recebe um slot / children /
conteúdo de template. Não há nenhuma API especial do Suprnova - você
importa um layout e renderiza o conteúdo da sua página dentro dele.

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

## Por que Suprnova diverge

A integração Inertia do Laravel traz um frontend por vez - você escolhe
React, Vue, ou Svelte na instalação com um starter kit por projeto. O
Suprnova mantém a mesma regra de um-por-projeto (você não mistura), mas
a CLI faz scaffold para os três de forma idiomática a partir da mesma
chamada `inertia_response!`. O lado Rust nunca sabe qual frontend está
em execução; o gerador e o resolver do Vite escolhem a extensão certa no
disco.

A outra divergência é a validação de componente em tempo de compilação.
O Laravel resolve o nome do componente em tempo de execução, então um
erro de digitação em `Inertia::render('Dahsboard')` se torna um erro de
produção. A macro `inertia_response!` do Suprnova percorre
`frontend/src/pages/` no momento da expansão e faz `cargo check` falhar
com uma sugestão "Você quis dizer 'Dashboard'?". A história completa de
tipos TypeScript (gerada a partir de `#[derive(InertiaProps)]` no struct
Rust) significa que as props do componente também são tipadas de ponta a
ponta.

## Próximos passos

- [Respostas Inertia](frontend-inertia-responses.md) - a macro
  `inertia_response!`, reloads parciais, props deferred
- [Tipos TypeScript](frontend-typescript-types.md) - `suprnova
  generate-types` e o pipeline de props tipadas
- [Visão geral do Frontend](frontend.md) - como a ponte Inertia se
  encaixa
- [Tutorial CRUD com Inertia](tutorial-inertia-crud.md) - um recurso
  Posts completo de ponta a ponta
- [Autenticação](authentication.md) - conectando as páginas de
  autenticação que o starter traz com scaffold para você
