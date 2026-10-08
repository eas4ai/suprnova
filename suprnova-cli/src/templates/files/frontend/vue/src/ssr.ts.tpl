import { createInertiaApp } from '@inertiajs/vue3'
import AppLayout from './layouts/AppLayout.vue'
import GuestLayout from './layouts/GuestLayout.vue'

// `suprnova ssr:start` runs this bundle under Node - `npm run build:ssr`
// (`vite build --ssr src/ssr.ts`) produces it. The Vite plugin
// (`@inertiajs/vite`) wraps the `createInertiaApp(..)` statement below
// with the server bootstrap (Inertia 3.8.0,
// `packages/vite/src/frameworks/vue.ts`): `createServer` from
// `@inertiajs/vue3/server` (re-exporting `@inertiajs/core/server`) opens
// the HTTP worker the framework's `SsrConfig` posts `POST /render` to,
// answers `GET /health` itself for `suprnova ssr:check`, and renders each
// page with `renderToString`. That is why this file imports neither: the
// wrapped module would declare both twice. In development the same plugin
// serves this entry at the dev server's `/__inertia_ssr`.
//
// Called on the server with no page, `createInertiaApp` returns the render
// function the bootstrap calls per request
// (`packages/vue3/src/createInertiaApp.ts`), and with no `setup` it mounts
// the page in a `createSSRApp` with the Inertia plugin.
//
// `initLang` is skipped here, same as `main.ts`'s `setup()`: there is no
// absolute URL to `fetch()` from inside the SSR worker, so the catalog
// never loads server-side and `t()` falls back to its raw-key rendering
// for the SSR pass. `lib/lang.ts`'s `useLang()` is a module-level
// composable with no context requirement, unlike React's, so no
// provider wrapper is needed here.
//
// Keep the options below in step with `main.ts`: the browser hydrates the
// markup this renders, and a layout or title that differs between the two
// is a hydration mismatch. There is no `nonce` here: the server has no
// document to read one from, and the client passes its own.
const appName = '{project_title}'

createInertiaApp({
  pages: './pages',
  layout: (name) => (name.startsWith('auth/') ? GuestLayout : AppLayout),
  title: (title) => (title ? `${title} - ${appName}` : appName),
  serverHead: true,
})
