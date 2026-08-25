import './app.css'
import { createInertiaApp, router, type ResolvedComponent } from '@inertiajs/svelte'
import { hydrate, mount } from 'svelte'
import { initLang } from './lib/lang.svelte'

// Forward the per-session CSRF token (rendered into <meta name="csrf-token">
// by the Suprnova CSRF middleware) on every Inertia visit. Inertia 3 uses
// the native fetch API and sets X-Inertia automatically, so no axios.
const csrfToken = document
  .querySelector('meta[name="csrf-token"]')
  ?.getAttribute('content')
if (csrfToken) {
  router.on('before', (event) => {
    event.detail.visit.headers['X-CSRF-TOKEN'] = csrfToken
  })
}

createInertiaApp({
  resolve: (name) => {
    const pages = import.meta.glob<ResolvedComponent>('./pages/**/*.svelte', {
      eager: true,
    })
    return pages[`./pages/${name}.svelte`]
  },
  async setup({ el, App, props }) {
    // `el` is `null` when `setup` runs server-side - @inertiajs/svelte's
    // `createInertiaApp` reuses this same callback for both the browser
    // bootstrap and an SSR render pass (see `ssr.ts`, which calls this
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

    if (el?.hasAttribute('data-server-rendered')) {
      hydrate(App, { target: el, props })
    } else {
      mount(App, { target: el!, props })
    }
  },
})
