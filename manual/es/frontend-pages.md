# Componentes de página

Una página es la unidad que Inertia envía por la red. El controlador
en Rust elige un nombre de componente y un struct de props tipado; el
frontend empaquetado con Vite resuelve ese nombre a un archivo en
`frontend/src/pages/` y lo renderiza con los props como argumentos. El
framework es agnóstico de framework - Suprnova incluye starters de
primera clase para Svelte 5, React 19 y Vue 3.5, y el contrato de
página tiene la misma forma en los tres.

## El contrato

Un controlador devuelve una respuesta de Inertia que nombra un
componente:

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

La cadena `"Home"` se resuelve contra `frontend/src/pages/Home.<ext>`.
La extensión depende de qué starter hayas generado con andamiaje:

| Starter | Extensión | ¿Por defecto? |
|---|---|---|
| Svelte 5 | `.svelte` | sí |
| React 19 | `.tsx` | - |
| Vue 3.5 | `.vue` | - |

La macro valida en tiempo de compilación que el archivo existe, así
que un error tipográfico o una página eliminada hace fallar
`cargo check` en lugar de devolver un 500 en producción.

## Layout de directorios

Sea cual sea el framework que hayas elegido, las páginas viven bajo
`frontend/src/pages/` y el nombre de componente en `inertia_response!`
es la ruta del archivo relativa a ese directorio, sin la extensión.
Las barras diagonales funcionan igual en todas las plataformas.

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

La convención es `Index` para páginas de colección, `Show` / `Edit` /
`Create` para páginas de un solo elemento, y un subdirectorio en
minúsculas como `auth/` para páginas de una feature agrupada. Las
mayúsculas y minúsculas en el nombre del componente deben coincidir
exactamente con el nombre del archivo - `import.meta.glob` de Vite
distingue mayúsculas de minúsculas.

## Generación de una página

El generador `make:inertia` de la CLI coloca un componente de partida
en el lugar correcto y usa la sintaxis del frontend que el proyecto
esté usando:

```bash
suprnova make:inertia Dashboard
```

El generador lee `SUPRNOVA_FRONTEND` de tu `.env` (usando Svelte por
defecto), elige la extensión correspondiente, y añade `Page` al
nombre del componente si todavía no lo tiene. Así que el comando
anterior crea uno de estos:

- `frontend/src/pages/DashboardPage.svelte`
- `frontend/src/pages/DashboardPage.tsx`
- `frontend/src/pages/DashboardPage.vue`

La salida de la consola imprime la llamada `inertia_response!`
correspondiente que deberías pegar en tu controlador.

Para omitir el sufijo y quedarte con el nombre tal cual, pasa el
nombre completo:

```bash
suprnova make:inertia DashboardPage   # crea DashboardPage.<ext>
```

Para generar en su lugar un struct de props tipado del lado de Rust,
pasa `--data`:

```bash
suprnova make:inertia Dashboard --data
# Crea app/src/props/dashboard.rs con #[derive(Data, Validate)]
```

## Una página en cada starter

El mismo `inertia_response!(&req, "Home", HomeProps { ... })` del lado
del backend se corresponde con uno de estos archivos de página en el
frontend. Los props llegan como argumentos tipados a través de los
tipos generados en `inertia-props.ts`.

### Svelte 5

Con runes activadas. Los props llegan vía `$props()`:

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

Componente de función estándar. Los props llegan como el primer
argumento:

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

`<script setup lang="ts">` con `defineProps`. Los props se acceden
directamente en el template:

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

## Navegación entre páginas

Cada starter incluye el adaptador de Inertia v3 para su framework. Las
exportaciones son las mismas: `Link` para navegación declarativa,
`router` para navegación programática, `usePage` (o `page`) para props
compartidos, `Form` y `useForm` para el manejo de formularios.

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

El objeto `router` también expone `router.post(url, data)`,
`router.put(url, data)`, `router.patch(url, data)`,
`router.delete(url)`, y `router.reload()` - la misma forma en los tres
adaptadores.

## Formularios

Inertia v3 incluye un componente declarativo `<Form>` y el ayudante
imperativo `useForm` (o `createForm` en Svelte). Ambos hacen POST de
vuelta a tu controlador en Rust; los errores de validación emergen
como un prop `errors` estructurado.

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

### Callbacks de formulario

`form.post(url, options)` - y los correspondientes `.put` / `.patch` /
`.delete` - aceptan los callbacks de visita estándar (`onStart`,
`onSuccess`, `onError`, `onFinish`). Los errores de validación que
devuelve tu handler en Rust aterrizan en `form.errors`
automáticamente; los callbacks son para efectos secundarios:

```ts
form.post('/posts', {
  onSuccess: async () => { await refreshDrafts() },  // esperado (await) desde Inertia 3.4
  onError: (errors) => console.warn(errors),
  onFinish: () => form.reset('content'),
})
```

Desde Inertia 3.4, un `onSuccess` asíncrono se espera (`await`) antes
de que el envío se complete, así que `form.processing` sigue siendo
`true` hasta que tu callback se resuelve - útil cuando un envío
exitoso dispara trabajo de seguimiento que no quieres que la UI
adelante.

## Sondeo

Para una página que debería refrescarse en un intervalo - un panel en
vivo, el estado de un job, una insignia de no leídos - el hook
`usePoll` reemite una recarga parcial con un temporizador. Impórtalo
desde tu adaptador:

```ts
import { usePoll } from '@inertiajs/svelte' // or '@inertiajs/react' / '@inertiajs/vue3'
```

Combínalo con `only` para que cada tick obtenga solo los props que
cambian - el servidor entonces resuelve solo esas claves (consulta
[recargas parciales](frontend-inertia-responses.md#partial-reloads)):

```ts
const { stop, start } = usePoll(5000, { only: ['stats', 'jobs'] })
```

`usePoll(interval, requestOptions, options)`:

- **`interval`** - milisegundos entre recargas.
- **`requestOptions`** - un objeto de opciones de recarga (`only`,
  `except`, `data`, `onSuccess`, …) **o una función que devuelve
  uno**, para que la solicitud pueda depender del estado actual (por
  ejemplo, un cursor que avanza en cada tick).
- **`options.mode`** - cómo se maneja un tick que se dispara mientras
  la solicitud anterior todavía está en vuelo: `'overlap'` (por
  defecto - disparar igualmente), `'cancel'` (abortar la solicitud en
  vuelo), o `'rest'` (omitir este tick).
- **`options.keepAlive`** - sigue sondeando mientras la pestaña está
  en segundo plano (por defecto `false`: el sondeo se pausa en una
  pestaña oculta).
- **`options.autoStart`** - empieza inmediatamente (por defecto
  `true`); pasa `false` y llama al `start()` devuelto cuando estés
  listo.

El hook devuelve `{ stop, start }` para control manual. Fuera de un
componente, `router.poll(...)` de `@inertiajs/core` es la misma
llamada.

Como cada tick es una recarga parcial ordinaria, los props bajo `only`
fluyen a través de los mismos resolvers Lazy / Optional / Defer que
cualquier otra solicitud - y esos resolvers se ejecutan
concurrentemente (limitados por `max_concurrent_resolvers`), así que
un panel que sondea seis widgets emite seis consultas paralelas por
tick en lugar de seis secuenciales.

## Props compartidos

Cualquier cosa que registres como prop compartido en el arranque -
típicamente el usuario actual, los mensajes flash, y el token CSRF
global - está disponible en cada página a través de `usePage()`
(React, Vue) o el store reactivo `page` (Svelte). Los props de página
sobrescriben los props compartidos en caso de colisión de clave.

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

Un layout es solo un componente ordinario que toma un slot / children
/ contenido de template. No hay ninguna API especial de Suprnova -
importas un layout y renderizas el contenido de tu página dentro de
él.

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

## Por qué Suprnova diverge

La integración de Inertia de Laravel incluye un solo frontend a la
vez - eliges React, Vue o Svelte al instalar, con un único starter kit
por proyecto. Suprnova mantiene la misma regla de uno por proyecto (no
se mezclan), pero la CLI genera andamiaje para los tres de forma
idiomática desde la misma llamada `inertia_response!`. El lado de Rust
nunca sabe qué frontend se está ejecutando; el generador y el resolver
de Vite eligen la extensión correcta en disco.

La otra divergencia es la validación de componentes en tiempo de
compilación. Laravel resuelve el nombre del componente en tiempo de
ejecución, así que un error tipográfico en `Inertia::render('Dahsboard')`
se convierte en un error de producción. La macro `inertia_response!`
de Suprnova recorre `frontend/src/pages/` en tiempo de expansión y
hace fallar `cargo check` con una sugerencia "Did you mean 'Dashboard'?".
La historia completa de tipos de TypeScript (generada a partir de
`#[derive(InertiaProps)]` en el struct de Rust) significa que los
props del componente también están tipados de punta a punta.

## Siguiente

- [Respuestas de Inertia](frontend-inertia-responses.md) - la macro
  `inertia_response!`, recargas parciales, props diferidos
- [Tipos de TypeScript](frontend-typescript-types.md) -
  `suprnova generate-types` y el pipeline de props tipados
- [Descripción general de Frontend](frontend.md) - cómo se ensambla el
  puente de Inertia
- [Tutorial de CRUD con Inertia](tutorial-inertia-crud.md) - un
  recurso Posts completo de principio a fin
- [Autenticación](authentication.md) - conectar las páginas de auth
  que el starter genera con andamiaje por ti
