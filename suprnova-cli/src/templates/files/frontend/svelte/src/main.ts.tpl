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
    // `el` is `null` when `setup` runs server-side — @inertiajs/svelte's
    // `createInertiaApp` reuses this same callback for both the browser
    // bootstrap and an SSR render pass. `initLang` does a `fetch()`,
    // which has no business running on the server (no absolute URL to
    // fetch, no reason to block a render on a network round trip
    // there), so it's skipped entirely there; `t()`'s documented
    // raw-key fallback covers whatever renders during that pass. This
    // scaffold doesn't ship an SSR build target today (no `vite build
    // --ssr` wiring), so `el` is always a real element in practice —
    // this guard is defense-in-depth for the day one is added.
    //
    // Caution if SSR is added later: awaiting `initLang` here, before
    // mount/hydrate, means a hydrating client's first paint would carry
    // real translations while the server-rendered markup it hydrates
    // against still has `t()`'s untranslated fallback — a hydration
    // content mismatch. This scaffold accepts that trade-off (translate
    // before first paint, for the common CSR-only case) rather than
    // deferring the catalog load until after hydration.
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
