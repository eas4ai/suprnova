import { createInertiaApp } from '@inertiajs/react'
import { layout, title } from './lib/app'

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
createInertiaApp({
  pages: './pages',
  title,
  layout,
  serverHead: true,
  setup: ({ App, props }) => <App {...props} />,
})
