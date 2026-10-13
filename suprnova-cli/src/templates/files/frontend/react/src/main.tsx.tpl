import './app.css'
import { createInertiaApp } from '@inertiajs/react'
import { createRoot, hydrateRoot } from 'react-dom/client'
import { layout, title } from './lib/app'
import { initLang, LangProvider } from './lib/lang'

// No hand-rolled CSRF header here, on purpose. The Inertia client this
// scaffold installs (`@inertiajs/*`, pinned `^3.8.0`) sends its visits over
// XMLHttpRequest and, whenever the `XSRF-TOKEN` cookie the Suprnova CSRF
// middleware sets is present, echoes it back in the `X-XSRF-TOKEN` header
// itself, per request. Reading `<meta name="csrf-token">` once at module
// load and pinning it into a header instead captures a token that logging
// in rotates, and the next visit - the logout - is refused with a 419.
// Server-side verification is unchanged: the CSRF middleware still checks
// the token on every state-changing request.

// The request's CSP nonce, when the server runs a nonce policy: the
// document's head then carries `<meta property="csp-nonce" nonce="...">`.
// The Inertia client stamps it on the progress bar and error dialog styles
// it injects. Read through the `nonce` property: the browser hides the
// attribute's value from `getAttribute` once the document has loaded.
const nonce =
  document.querySelector<HTMLMetaElement>('meta[property="csp-nonce"]')?.nonce || undefined

createInertiaApp({
  // The Inertia Vite plugin replaces `pages` with a resolver that loads
  // `./pages/<name>.tsx`, each page in its own chunk.
  pages: './pages',
  title,
  layout,
  // Head tags a handler sets from Rust arrive as the `head` prop and are
  // kept in the document's head across visits.
  serverHead: true,
  nonce,
  setup({ el, App, props }) {
    // `el` is `null` only when `setup` runs in a server render. This
    // entry runs in the browser; `ssr.tsx` renders on the server.
    if (!el) {
      return
    }

    // `initLang` does a `fetch()`, which has no business running on the
    // server (no absolute URL to fetch, no reason to block a render on a
    // network round trip there), so `ssr.tsx` never calls it; `t()`'s
    // documented raw-key fallback covers whatever renders during that pass.
    //
    // Caution: loading the catalog here, before mount/hydrate, means a
    // hydrating client's first paint carries real translations while the
    // server-rendered markup it hydrates against still has `t()`'s
    // untranslated fallback (SSR always skips `initLang` - see above) - a
    // hydration content mismatch on any translated string. This scaffold
    // accepts that trade-off (translate before first paint, for the common
    // case) rather than deferring the catalog load until after hydration.
    // `initLang` never rejects: a failed fetch degrades to the fallback.
    void initLang(props.initialPage).then(() => {
      const app = (
        <LangProvider>
          <App {...props} />
        </LangProvider>
      )

      if (el.hasAttribute('data-server-rendered')) {
        hydrateRoot(el, app)
      } else {
        createRoot(el).render(app)
      }
    })
  },
})
