import { createInertiaApp, type ResolvedComponent } from '@inertiajs/svelte'
import createServer from '@inertiajs/svelte/server'
import { render } from 'svelte/server'

// `suprnova ssr:start` runs this bundle under Node — `npm run build:ssr`
// (`vite build --ssr src/ssr.ts`) produces it. `createServer` (from
// `@inertiajs/svelte/server`, re-exporting `@inertiajs/core/server`)
// opens the HTTP worker the framework's `SsrConfig` posts `POST /render`
// to, and answers `GET /health` itself — no extra code needed here for
// `suprnova ssr:check`.
//
// `svelte/server`'s `render()` returns `{ body, head }` directly, which
// is the exact shape Inertia's `setup()` contract expects for SSR — no
// `render:` option to pass at the `createInertiaApp` level, unlike
// React/Vue. `lib/lang.svelte.ts`'s `t()`/`initLang()` are plain module
// state with no context requirement, so no wrapper is needed here (see
// `main.ts`'s `setup()` — same story: `initLang` never runs server-side,
// so `t()` falls back to raw-key rendering for the SSR pass).
createServer((page) =>
  createInertiaApp({
    page,
    resolve: (name) => {
      const pages = import.meta.glob<ResolvedComponent>('./pages/**/*.svelte', { eager: true })
      return pages[`./pages/${name}.svelte`]
    },
    setup({ App, props }) {
      return render(App, { props })
    },
  }),
)
