# Seiten-Komponenten

Eine Seite ist die Einheit, die Inertia an den Client schickt. Der
Rust-Controller wählt einen Komponentennamen und eine typisierte
Prop-Struktur; das Vite-gebündelte Frontend löst diesen Namen zu
einer Datei in `frontend/src/pages/` auf und rendert sie mit den
Props als Argumenten. Das Framework ist framework-agnostisch -
Suprnova liefert erstklassige Starter für Svelte 5, React 19 und Vue
3.5, und der Seiten-Vertrag hat in allen drei dieselbe Form.

## Der Vertrag

Ein Controller liefert eine Inertia-Response, die eine Komponente
benennt:

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

Der String `"Home"` wird gegen `frontend/src/pages/Home.<ext>`
aufgelöst. Die Erweiterung hängt davon ab, welchen Starter Sie
gescaffoldet haben:

| Starter | Erweiterung | Standard? |
|---|---|---|
| Svelte 5 | `.svelte` | ja |
| React 19 | `.tsx` | - |
| Vue 3.5 | `.vue` | - |

Das Makro validiert zur Compile-Zeit, dass die Datei existiert,
sodass ein Tippfehler oder eine gelöschte Seite `cargo check`
fehlschlagen lässt, statt in Produktion einen 500 auszulösen.

## Verzeichnislayout

Unabhängig davon, welches Framework Sie gewählt haben, liegen Seiten
unter `frontend/src/pages/`, und der Komponentenname in
`inertia_response!` ist der Dateipfad relativ zu diesem Verzeichnis,
ohne die Erweiterung. Schrägstriche funktionieren auf allen
Plattformen gleich.

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

Die Konvention ist `Index` für Sammlungsseiten, `Show` / `Edit` /
`Create` für Einzelelement-Seiten und ein Kleinbuchstaben-
Unterverzeichnis wie `auth/` für gruppierte Feature-Seiten. Die
Groß-/Kleinschreibung im Komponentennamen muss exakt zum Dateinamen
passen - Vites `import.meta.glob` unterscheidet zwischen Groß- und
Kleinschreibung.

## Eine Seite generieren

Der `make:inertia`-Generator der CLI legt eine Starter-Komponente am
richtigen Ort ab und verwendet die Syntax für das Frontend, das das
Projekt gerade nutzt:

```bash
suprnova make:inertia Dashboard
```

Der Generator liest `SUPRNOVA_FRONTEND` aus Ihrer `.env` (Standard:
Svelte), wählt die passende Erweiterung und hängt `Page` an den
Komponentennamen an, falls es dort noch nicht steht. Der obige
Befehl erstellt also eine von:

- `frontend/src/pages/DashboardPage.svelte`
- `frontend/src/pages/DashboardPage.tsx`
- `frontend/src/pages/DashboardPage.vue`

Die Konsolenausgabe druckt den passenden
`inertia_response!`-Aufruf, den Sie in Ihren Controller einfügen
sollten.

Um das Suffix zu überspringen und den Namen selbst zu bestimmen,
übergeben Sie den vollen Namen:

```bash
suprnova make:inertia DashboardPage   # erstellt DashboardPage.<ext>
```

Um stattdessen eine typisierte Prop-Struktur auf der Rust-Seite zu
generieren, übergeben Sie `--data`:

```bash
suprnova make:inertia Dashboard --data
# Erstellt app/src/props/dashboard.rs mit #[derive(Data, Validate)]
```

## Eine Seite in jedem Starter

Dasselbe `inertia_response!(&req, "Home", HomeProps { ... })` auf dem
Backend bildet auf eine dieser Seiten-Dateien im Frontend ab. Props
kommen als typisierte Argumente über die generierten
`inertia-props.ts`-Typen an.

### Svelte 5

Runes aktiviert. Props kommen über `$props()` an:

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

Standard-Funktionskomponente. Props kommen als erstes Argument an:

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

`<script setup lang="ts">` mit `defineProps`. Props werden direkt im
Template angesprochen:

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

## Navigation zwischen Seiten

Jeder Starter liefert den Inertia-v3-Adapter für sein Framework. Die
Exporte sind dieselben: `Link` für deklarative Navigation, `router`
für programmatische Navigation, `usePage` (oder `page`) für
gemeinsame Props, `Form` und `useForm` für Formularverarbeitung.

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

Das `router`-Objekt stellt außerdem `router.post(url, data)`,
`router.put(url, data)`, `router.patch(url, data)`,
`router.delete(url)` und `router.reload()` bereit - dieselbe Form
über alle drei Adapter.

## Formulare

Inertia v3 liefert eine deklarative `<Form>`-Komponente und den
imperativen `useForm`-Helfer (`createForm` in Svelte). Beide senden
per POST zurück an Ihren Rust-Controller; Validierungsfehler
erscheinen als strukturierte `errors`-Prop.

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

### Formular-Callbacks

`form.post(url, options)` - und die passenden `.put` / `.patch` /
`.delete` - akzeptieren die Standard-Besuchs-Callbacks (`onStart`,
`onSuccess`, `onError`, `onFinish`). Validierungsfehler, die Ihr
Rust-Handler zurückgibt, landen automatisch in `form.errors`; die
Callbacks sind für Nebeneffekte:

```ts
form.post('/posts', {
  onSuccess: async () => { await refreshDrafts() },  // seit Inertia 3.4 awaited
  onError: (errors) => console.warn(errors),
  onFinish: () => form.reset('content'),
})
```

Seit Inertia 3.4 wird ein asynchrones `onSuccess` awaited, bevor die
Übermittlung abschließt, sodass `form.processing` `true` bleibt, bis
Ihr Callback aufgelöst ist - praktisch, wenn ein erfolgreiches
Abschicken Folgearbeit anstößt, die die UI nicht überholen soll.

## Polling

Für eine Seite, die sich in einem Intervall aktualisieren soll - ein
Live-Dashboard, ein Job-Status, ein Ungelesen-Badge - stößt der
`usePoll`-Hook per Timer erneut einen Partial Reload an. Importieren
Sie ihn aus Ihrem Adapter:

```ts
import { usePoll } from '@inertiajs/svelte' // oder '@inertiajs/react' / '@inertiajs/vue3'
```

Kombinieren Sie ihn mit `only`, sodass jeder Tick nur die Props holt,
die sich ändern - der Server löst dann nur diese Schlüssel auf
(siehe [Partial Reloads](frontend-inertia-responses.md#partial-reloads)):

```ts
const { stop, start } = usePoll(5000, { only: ['stats', 'jobs'] })
```

`usePoll(interval, requestOptions, options)`:

- **`interval`** - Millisekunden zwischen Reloads.
- **`requestOptions`** - ein Reload-Options-Objekt (`only`, `except`,
  `data`, `onSuccess`, …) **oder eine Funktion, die eines
  zurückgibt**, sodass die Anfrage vom aktuellen Zustand abhängen
  kann (z. B. ein Cursor, der bei jedem Tick vorrückt).
- **`options.mode`** - wie ein Tick behandelt wird, der feuert,
  während die vorherige Anfrage noch unterwegs ist: `'overlap'`
  (Standard - trotzdem feuern), `'cancel'` (die laufende Anfrage
  abbrechen) oder `'rest'` (diesen Tick auslassen).
- **`options.keepAlive`** - weiter pollen, während der Tab im
  Hintergrund ist (Standard `false`: Polling pausiert auf einem
  verborgenen Tab).
- **`options.autoStart`** - sofort beginnen (Standard `true`);
  übergeben Sie `false` und rufen Sie das zurückgegebene `start()`
  auf, wenn Sie bereit sind.

Der Hook liefert `{ stop, start }` für manuelle Steuerung. Außerhalb
einer Komponente ist `router.poll(...)` aus `@inertiajs/core`
derselbe Aufruf.

Weil jeder Tick ein gewöhnlicher Partial Reload ist, laufen die
Props unter `only` durch dieselben Lazy-/Optional-/Defer-Resolver
wie jede andere Anfrage - und diese Resolver laufen gleichzeitig
(begrenzt durch `max_concurrent_resolvers`), sodass ein Dashboard,
das sechs Widgets pollt, sechs parallele Queries pro Tick ausgibt
statt sechs serieller.

## Gemeinsame Props

Alles, was Sie beim Boot als gemeinsame Prop registrieren -
typischerweise der aktuelle Benutzer, Flash-Nachrichten und das
globale CSRF-Token - ist auf jeder Seite über `usePage()` (React,
Vue) oder den reaktiven `page`-Store (Svelte) verfügbar. Seiten-Props
überschreiben gemeinsame Props bei einer Schlüsselkollision.

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

Ein Layout ist einfach eine gewöhnliche Komponente, die einen Slot /
Children / Template-Inhalt entgegennimmt. Es gibt keine spezielle
Suprnova-API - Sie importieren ein Layout und rendern Ihren
Seiteninhalt darin.

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

## Warum Suprnova abweicht

Laravels Inertia-Integration liefert immer nur ein Frontend
gleichzeitig - Sie wählen React, Vue oder Svelte bei der Installation
mit einem einzigen Starter-Kit pro Projekt. Suprnova behält dieselbe
Eins-pro-Projekt-Regel (Sie mischen nicht), aber die CLI scaffoldet
idiomatisch zu allen drei aus demselben `inertia_response!`-Aufruf.
Die Rust-Seite weiß nie, welches Frontend läuft; der Generator und
der Vite-Resolver wählen die richtige Erweiterung auf der Platte.

Die andere Abweichung ist die Komponentenvalidierung zur
Compile-Zeit. Laravel löst den Komponentennamen zur Laufzeit auf,
sodass ein Tippfehler in `Inertia::render('Dahsboard')` zu einem
Produktionsfehler wird. Suprnovas `inertia_response!`-Makro
durchläuft `frontend/src/pages/` zur Expansionszeit und lässt `cargo
check` mit einem Vorschlag „Did you mean 'Dashboard'?“ fehlschlagen.
Die vollständige TypeScript-Typ-Geschichte (generiert aus
`#[derive(InertiaProps)]` auf der Rust-Struktur) bedeutet, dass auch
die Props der Komponente end-to-end typisiert sind.

## Nächste Schritte

- [Inertia Responses](frontend-inertia-responses.md) - das
  `inertia_response!`-Makro, Partial Reloads, Deferred Props
- [TypeScript Types](frontend-typescript-types.md) - `suprnova
  generate-types` und die Typed-Props-Pipeline
- [Frontend - Übersicht](frontend.md) - wie die Inertia-Brücke
  zusammenpasst
- [Inertia-CRUD-Tutorial](tutorial-inertia-crud.md) - eine
  vollständige Posts-Ressource end-to-end
- [Authentifizierung](authentication.md) - die Auth-Seiten
  verdrahten, die der Starter für Sie scaffoldet
