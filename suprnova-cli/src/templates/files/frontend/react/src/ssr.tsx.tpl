import { createInertiaApp, type ResolvedComponent } from '@inertiajs/react'
import createServer from '@inertiajs/react/server'
import ReactDOMServer from 'react-dom/server'
import { LangProvider } from './lib/lang'

// `suprnova ssr:start` runs this bundle under Node — `npm run build:ssr`
// (`vite build --ssr src/ssr.tsx`) produces it. `createServer` (from
// `@inertiajs/react/server`, re-exporting `@inertiajs/core/server`) opens
// the HTTP worker the framework's `SsrConfig` posts `POST /render` to,
// and answers `GET /health` itself — no extra code needed here for
// `suprnova ssr:check`.
//
// `initLang` is skipped here, same as `main.tsx`'s `setup()`: there is
// no absolute URL to `fetch()` from inside the SSR worker, so the
// catalog never loads server-side and `t()` falls back to its raw-key
// rendering for the SSR pass. `<LangProvider>` still has to wrap the
// tree — the scaffolded `Home.tsx` calls `useLang()`, which throws
// outside a provider, and that would fail SSR for every page rather
// than degrade gracefully.
createServer((page) =>
  createInertiaApp({
    page,
    render: ReactDOMServer.renderToString,
    resolve: (name) => {
      const pages = import.meta.glob<ResolvedComponent>('./pages/**/*.tsx', {
        eager: true,
        import: 'default',
      })
      return pages[`./pages/${name}.tsx`]
    },
    setup: ({ App, props }) => (
      <LangProvider>
        <App {...props} />
      </LangProvider>
    ),
  }),
)
