import { createInertiaApp } from '@inertiajs/svelte'
import AppLayout from './layouts/AppLayout.svelte'
import GuestLayout from './layouts/GuestLayout.svelte'
import { pageTitle } from './lib/title'

// `suprnova ssr:start` runs this entry's bundle under Node, and
// `npm run build:ssr` (`vite build --ssr src/ssr.ts`) produces it at
// `bootstrap/ssr/ssr.js`. The call below is the whole entry: on an SSR
// build `@inertiajs/vite` wraps this top-level `createInertiaApp(...)`
// statement in the server bootstrap (`packages/vite/src/ssrTransform.ts`
// and `frameworks/svelte.ts`). It imports `createServer` from
// `@inertiajs/svelte/server` and `render` from `svelte/server`, renders
// each page the framework's `SsrConfig` posts to `POST /render`, answers
// `GET /health` for `suprnova ssr:check`, and starts that server in a
// production build only. Under `npm run dev` the plugin serves the same
// render at `/__inertia_ssr` instead.
//
// The options match `main.ts`, so the markup the server renders is the
// markup the browser hydrates: the same pages, the same layout, the same
// title. `initLang` never runs on the server (see `main.ts`'s `setup`), so
// `t()` falls back to raw-key rendering for this pass; `lib/lang.svelte.ts`
// is plain module state with no context requirement, so nothing wraps the
// app here.
createInertiaApp({
  pages: './pages',
  layout: (name) => (name.startsWith('auth/') ? GuestLayout : AppLayout),
  title: pageTitle,
  serverHead: true,
})
