import './app.css'
import { createInertiaApp, type ResolvedComponent } from '@inertiajs/react'
import { createRoot, hydrateRoot } from 'react-dom/client'
import { initLang, LangProvider } from './lib/lang'

// No hand-rolled CSRF header here, on purpose. The Inertia client this
// scaffold installs (`@inertiajs/*`, pinned `^3.6.1`) sends its visits over
// XMLHttpRequest and, whenever the `XSRF-TOKEN` cookie the Suprnova CSRF
// middleware sets is present, echoes it back in the `X-XSRF-TOKEN` header
// itself, per request. Reading `<meta name="csrf-token">` once at module
// load and pinning it into a header instead captures a token that logging
// in rotates, and the next visit - the logout - is refused with a 419.
// Server-side verification is unchanged: the CSRF middleware still checks
// the token on every state-changing request.

createInertiaApp({
  resolve: (name) => {
    const pages = import.meta.glob<ResolvedComponent>('./pages/**/*.tsx', {
      eager: true,
      import: 'default',
    })
    return pages[`./pages/${name}.tsx`]
  },
  async setup({ el, App, props }) {
    // `el` is `null` when `setup` runs server-side - @inertiajs/react's
    // `createInertiaApp` reuses this same callback for both the browser
    // bootstrap and an SSR render pass (see `ssr.tsx`, which calls this
    // same `createInertiaApp` shape from inside `createServer`).
    // `initLang` does a `fetch()`, which has no business running on the
    // server (no absolute URL to fetch, no reason to block a render on
    // a network round trip there), so it's skipped entirely there;
    // `t()`'s documented raw-key fallback covers whatever renders
    // during that pass.
    //
    // Caution: awaiting `initLang` here, before mount/hydrate, means a
    // hydrating client's first paint carries real translations while
    // the server-rendered markup it hydrates against still has `t()`'s
    // untranslated fallback (SSR always skips `initLang` - see above) -
    // a hydration content mismatch on any translated string. This
    // scaffold accepts that trade-off (translate before first paint,
    // for the common case) rather than deferring the catalog load until
    // after hydration.
    if (el) {
      await initLang(props.initialPage)
    }

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
  },
})
