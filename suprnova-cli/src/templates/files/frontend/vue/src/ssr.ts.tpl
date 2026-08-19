import { createInertiaApp } from '@inertiajs/vue3'
import createServer from '@inertiajs/vue3/server'
import { createSSRApp, h, type DefineComponent } from 'vue'
import { renderToString } from 'vue/server-renderer'

// `suprnova ssr:start` runs this bundle under Node — `npm run build:ssr`
// (`vite build --ssr src/ssr.ts`) produces it. `createServer` (from
// `@inertiajs/vue3/server`, re-exporting `@inertiajs/core/server`) opens
// the HTTP worker the framework's `SsrConfig` posts `POST /render` to,
// and answers `GET /health` itself — no extra code needed here for
// `suprnova ssr:check`.
//
// `initLang` is skipped here, same as `main.ts`'s `setup()`: there is no
// absolute URL to `fetch()` from inside the SSR worker, so the catalog
// never loads server-side and `t()` falls back to its raw-key rendering
// for the SSR pass. `lib/lang.ts`'s `useLang()` is a module-level
// composable with no context requirement, unlike React's, so no
// provider wrapper is needed here.
createServer((page) =>
  createInertiaApp({
    page,
    render: renderToString,
    resolve: (name) => {
      const pages = import.meta.glob<DefineComponent>('./pages/**/*.vue', { eager: true })
      return pages[`./pages/${name}.vue`]
    },
    setup({ App, props, plugin }) {
      return createSSRApp({ render: () => h(App, props) }).use(plugin)
    },
  }),
)
