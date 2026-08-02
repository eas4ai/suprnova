import './app.css'
import { createInertiaApp, router } from '@inertiajs/vue3'
import { createApp, createSSRApp, h, type DefineComponent } from 'vue'
import { initLang } from './lib/lang'

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
    const pages = import.meta.glob<DefineComponent>('./pages/**/*.vue', {
      eager: true,
    })
    return pages[`./pages/${name}.vue`]
  },
  async setup({ el, App, props, plugin }) {
    // `el` is `null` when `setup` runs server-side — @inertiajs/vue3's
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

    // SSR-aware: when the server pre-rendered, the mount node carries
    // `data-server-rendered="true"` and we must hydrate. Without this
    // check, SSR markup gets destroyed and re-rendered on the client.
    if (el.hasAttribute('data-server-rendered')) {
      createSSRApp({ render: () => h(App, props) })
        .use(plugin)
        .mount(el)
    } else {
      createApp({ render: () => h(App, props) })
        .use(plugin)
        .mount(el)
    }
  },
})
