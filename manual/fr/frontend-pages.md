# Composants de page

Une page est l'unité qu'Inertia expédie sur le réseau. Le contrôleur
Rust choisit un nom de composant et une struct de props typée ; le
frontend empaqueté par Vite résout ce nom en un fichier dans
`frontend/src/pages/` et le rend avec les props comme arguments. Le
framework est agnostique du framework frontend - Suprnova fournit des
starters de première classe pour Svelte 5, React 19 et Vue 3.5, et le
contrat de page a la même forme dans les trois.

## Le contrat

Un contrôleur retourne une réponse Inertia qui nomme un composant :

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

La chaîne `"Home"` est résolue contre
`frontend/src/pages/Home.<ext>`. L'extension dépend du starter que
vous avez scaffoldé :

| Starter | Extension | Par défaut ? |
|---|---|---|
| Svelte 5 | `.svelte` | oui |
| React 19 | `.tsx` | - |
| Vue 3.5 | `.vue` | - |

La macro valide à la compilation que le fichier existe, si bien qu'une
faute de frappe ou une page supprimée fait échouer `cargo check` au
lieu de partir en 500 en production.

## Disposition des répertoires

Quel que soit le framework choisi, les pages vivent sous
`frontend/src/pages/` et le nom de composant dans `inertia_response!`
est le chemin de fichier relatif à ce répertoire, sans l'extension.
Les slashs avant fonctionnent de la même façon sur toutes les
plateformes.

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

La convention est `Index` pour les pages de collection, `Show` /
`Edit` / `Create` pour les pages d'élément unique, et un
sous-répertoire en minuscules comme `auth/` pour les pages de
fonctionnalité groupées. La casse dans le nom de composant doit
correspondre exactement au nom de fichier - `import.meta.glob` de Vite
est sensible à la casse.

## Générer une page

Le générateur `make:inertia` de la CLI dépose un composant de
démarrage au bon endroit et utilise la syntaxe du frontend qu'utilise
le projet :

```bash
suprnova make:inertia Dashboard
```

Le générateur lit `SUPRNOVA_FRONTEND` depuis votre `.env` (Svelte par
défaut), choisit l'extension correspondante, et ajoute `Page` au nom
du composant s'il n'y est pas déjà. La commande ci-dessus crée donc
l'un de ces fichiers :

- `frontend/src/pages/DashboardPage.svelte`
- `frontend/src/pages/DashboardPage.tsx`
- `frontend/src/pages/DashboardPage.vue`

La sortie console affiche l'appel `inertia_response!` correspondant
que vous devriez coller dans votre contrôleur.

Pour sauter le suffixe et garder la main sur le nom, passez le nom
complet :

```bash
suprnova make:inertia DashboardPage   # crée DashboardPage.<ext>
```

Pour générer plutôt une struct de props typée côté Rust, passez
`--data` :

```bash
suprnova make:inertia Dashboard --data
# Crée app/src/props/dashboard.rs avec #[derive(Data, Validate)]
```

## Une page dans chaque starter

Le même `inertia_response!(&req, "Home", HomeProps { ... })` côté
backend correspond à l'un de ces fichiers de page côté frontend. Les
props arrivent comme arguments typés via les types générés
`inertia-props.ts`.

### Svelte 5

Runes-on. Les props arrivent via `$props()` :

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

Composant fonction standard. Les props arrivent comme premier
argument :

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

`<script setup lang="ts">` avec `defineProps`. Les props sont accédées
directement dans le template :

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

## Navigation entre les pages

Chaque starter fournit l'adaptateur Inertia v3 de son framework. Les
exports sont les mêmes : `Link` pour la navigation déclarative,
`router` pour la navigation programmatique, `usePage` (ou `page`) pour
les props partagées, `Form` et `useForm` pour la gestion de
formulaire.

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

L'objet `router` expose aussi `router.post(url, data)`,
`router.put(url, data)`, `router.patch(url, data)`, `router.delete(url)`
et `router.reload()` - même forme dans les trois adaptateurs.

## Formulaires

Inertia v3 fournit un composant déclaratif `<Form>` et le helper
impératif `useForm` (ou `createForm` en Svelte). Les deux font un POST
vers votre contrôleur Rust ; les erreurs de validation surgissent sous
forme d'une prop `errors` structurée.

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

### Callbacks de formulaire

`form.post(url, options)` - et les `.put` / `.patch` / `.delete`
correspondants - acceptent les callbacks de visite standard
(`onStart`, `onSuccess`, `onError`, `onFinish`). Les erreurs de
validation retournées par votre handler Rust atterrissent
automatiquement dans `form.errors` ; les callbacks servent aux effets
de bord :

```ts
form.post('/posts', {
  onSuccess: async () => { await refreshDrafts() },  // awaité depuis Inertia 3.4
  onError: (errors) => console.warn(errors),
  onFinish: () => form.reset('content'),
})
```

Depuis Inertia 3.4, un `onSuccess` async est awaité avant que la
soumission ne se clôture, si bien que `form.processing` reste `true`
jusqu'à ce que votre callback se résolve - pratique quand une
soumission réussie déclenche un travail de suivi que vous ne voulez
pas voir l'UI devancer.

## Polling

Pour une page qui doit se rafraîchir à intervalle régulier - un
tableau de bord en direct, un statut de job, un badge de non-lus - le
hook `usePoll` relance un rechargement partiel sur une minuterie.
Importez-le depuis votre adaptateur :

```ts
import { usePoll } from '@inertiajs/svelte' // ou '@inertiajs/react' / '@inertiajs/vue3'
```

Associez-le à `only` pour que chaque tick ne récupère que les props
qui changent - le serveur ne résout alors que ces clés (voir les
[rechargements partiels](frontend-inertia-responses.md#partial-reloads)) :

```ts
const { stop, start } = usePoll(5000, { only: ['stats', 'jobs'] })
```

`usePoll(interval, requestOptions, options)` :

- **`interval`** - millisecondes entre les rechargements.
- **`requestOptions`** - un objet d'options de rechargement (`only`,
  `except`, `data`, `onSuccess`, …) **ou une fonction qui en retourne
  un**, pour que la requête puisse dépendre de l'état courant (par
  exemple un curseur qui avance à chaque tick).
- **`options.mode`** - comment un tick qui se déclenche pendant que la
  requête précédente est encore en vol est géré : `'overlap'`
  (défaut - déclenche quand même), `'cancel'` (abandonne la requête en
  vol), ou `'rest'` (saute ce tick).
- **`options.keepAlive`** - continue le polling pendant que l'onglet
  est en arrière-plan (défaut `false` : le polling se met en pause sur
  un onglet masqué).
- **`options.autoStart`** - démarre immédiatement (défaut `true`) ;
  passez `false` et appelez le `start()` retourné quand vous êtes
  prêt.

Le hook retourne `{ stop, start }` pour un contrôle manuel. Hors d'un
composant, `router.poll(...)` depuis `@inertiajs/core` est le même
appel.

Comme chaque tick est un rechargement partiel ordinaire, les props
sous `only` passent par les mêmes résolveurs Lazy / Optional / Defer
que toute autre requête - et ces résolveurs s'exécutent en concurrence
(plafonnés par `max_concurrent_resolvers`), si bien qu'un tableau de
bord qui sonde six widgets émet six requêtes parallèles par tick au
lieu de six séquentielles.

## Props partagées

Tout ce que vous enregistrez comme prop partagée à l'amorçage -
typiquement l'utilisateur courant, les messages flash et le token
CSRF global - est disponible sur chaque page via `usePage()` (React,
Vue) ou le store réactif `page` (Svelte). Les props de page l'emportent
sur les props partagées en cas de collision de clé.

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

Un layout est simplement un composant ordinaire qui prend un
slot / des children / du contenu de template. Il n'y a pas d'API
Suprnova spéciale - vous importez un layout et vous rendez le contenu
de votre page à l'intérieur.

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

## Pourquoi Suprnova diverge

L'intégration Inertia de Laravel fournit un seul frontend à la
fois - vous choisissez React, Vue ou Svelte à l'installation avec un
seul starter kit par projet. Suprnova garde la même règle
d'un-par-projet (vous ne mélangez pas), mais la CLI scaffolde vers les
trois de façon idiomatique depuis le même appel `inertia_response!`.
Le côté Rust ne sait jamais quel frontend s'exécute ; le générateur et
le résolveur Vite choisissent la bonne extension sur le disque.

L'autre divergence est la validation de composant à la compilation.
Laravel résout le nom du composant à l'exécution, si bien qu'une
faute de frappe dans `Inertia::render('Dahsboard')` devient une erreur
de production. La macro `inertia_response!` de Suprnova parcourt
`frontend/src/pages/` au moment de l'expansion et fait échouer
`cargo check` avec une suggestion « Vouliez-vous dire 'Dashboard' ? ».
L'histoire complète des types TypeScript (générés depuis
`#[derive(InertiaProps)]` sur la struct Rust) signifie que les props
du composant sont aussi typées de bout en bout.

## Suivant

- [Réponses Inertia](frontend-inertia-responses.md) - la macro
  `inertia_response!`, rechargements partiels, props deferred
- [Types TypeScript](frontend-typescript-types.md) -
  `suprnova generate-types` et le pipeline de props typées
- [Présentation du frontend](frontend.md) - comment le pont Inertia
  s'articule
- [Tutoriel CRUD Inertia](tutorial-inertia-crud.md) - une ressource
  Posts complète de bout en bout
- [Authentification](authentication.md) - câbler les pages d'auth que
  le starter scaffolde pour vous
