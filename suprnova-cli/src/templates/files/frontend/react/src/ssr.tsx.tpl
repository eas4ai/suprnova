import { createInertiaApp } from '@inertiajs/react'
import { layout, title } from './lib/app'
import { LangProvider } from './lib/lang'

// `suprnova ssr:start` runs the bundle `npm run build:ssr` builds from this
// file under Node. The Inertia Vite plugin wraps this `createInertiaApp`
// call: a production build passes its render function to `createServer`
// from `@inertiajs/react/server`, which opens the HTTP worker the
// framework's `SsrConfig` posts `POST /render` to and answers `GET /health`
// itself for `suprnova ssr:check`; in development the Vite dev server calls
// it at `/__inertia_ssr` (`packages/vite/src/ssrTransform.ts` and
// `frameworks/react.ts` in Inertia 3.8.0).
//
// `pages`, `title` and `layout` match `main.tsx`, so the markup rendered
// here is the markup the browser hydrates. No `nonce` here: the server has
// no document to read one from.
//
// `initLang` is skipped here, same as `main.tsx`'s `setup()` on a server
// render: there is no absolute URL to `fetch()` from inside the SSR worker,
// so the catalog never loads server-side and `t()` falls back to its
// raw-key rendering for the SSR pass. `<LangProvider>` still has to wrap the
// tree - the scaffolded `Home.tsx` calls `useLang()`, which throws outside a
// provider, and that would fail SSR for every page rather than degrade
// gracefully.
createInertiaApp({
  pages: './pages',
  title,
  layout,
  serverHead: true,
  setup: ({ App, props }) => (
    <LangProvider>
      <App {...props} />
    </LangProvider>
  ),
})
