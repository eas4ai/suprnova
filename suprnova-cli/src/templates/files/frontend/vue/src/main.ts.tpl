import './app.css'
import { createInertiaApp } from '@inertiajs/vue3'
import { createApp, createSSRApp, h } from 'vue'
import AppLayout from './layouts/AppLayout.vue'
import GuestLayout from './layouts/GuestLayout.vue'
import { initLang } from './lib/lang'

// No hand-rolled CSRF header here, on purpose. The Inertia client this
// scaffold installs (`@inertiajs/*`, pinned `^3.8.0`) sends its visits over
// XMLHttpRequest and, whenever the `XSRF-TOKEN` cookie the Suprnova CSRF
// middleware sets is present, echoes it back in the `X-XSRF-TOKEN` header
// itself, per request. Reading `<meta name="csrf-token">` once at module
// load and pinning it into a header instead captures a token that logging
// in rotates, and the next visit - the logout - is refused with a 419.
// Server-side verification is unchanged: the CSRF middleware still checks
// the token on every state-changing request.

// The tab title reads `Page - App` for a page that sets one with `<Head>`,
// and the application's name for one that does not. `ssr.ts` titles the
// server render the same way, and the server's `default_title` in
// `src/bootstrap.rs` names the application the same.
const appName = '{project_title}'

// Under a nonce-based Content Security Policy the server writes the
// request's nonce into `<meta property="csp-nonce" nonce="...">`, the
// element Vite's runtime reads too. The browser hides a nonce attribute's
// value, so it is read from the element's `nonce` property. The Inertia
// client stamps it on the progress bar and dialog styles it injects.
const nonce =
  document.querySelector<HTMLMetaElement>('meta[property="csp-nonce"]')?.nonce || undefined

createInertiaApp({
  // The Vite plugin (`@inertiajs/vite`) turns this into a resolver over
  // `./pages/**/*.vue`, one code-split chunk per page.
  pages: './pages',
  // Pages under `auth/` render inside the guest layout, every other page
  // inside the application layout. A layout stays mounted across visits
  // between two pages that share it, so its state survives navigation.
  layout: (name) => (name.startsWith('auth/') ? GuestLayout : AppLayout),
  title: (title) => (title ? `${title} - ${appName}` : appName),
  // The `head` prop's tags, set from Rust, join the document head.
  serverHead: true,
  nonce,
  setup({ el, App, props, plugin }) {
    // This entry only runs in the browser, so `el` is always the mount
    // node; `ssr.ts` renders on the server without a `setup`.
    if (!el) {
      return
    }

    // SSR-aware: when the server pre-rendered, the mount node carries
    // `data-server-rendered="true"` and we must hydrate. Without this
    // check, SSR markup gets destroyed and re-rendered on the client.
    const app = el.hasAttribute('data-server-rendered')
      ? createSSRApp({ render: () => h(App, props) })
      : createApp({ render: () => h(App, props) })
    app.use(plugin)

    // The catalog loads before the first mount, so the first paint
    // carries real translations. `initLang` does a `fetch()`, which
    // `ssr.ts` never runs (no absolute URL to fetch, no reason to block a
    // render on a network round trip there); `t()`'s documented raw-key
    // fallback covers whatever renders during that pass.
    //
    // Caution: loading the catalog before mount/hydrate means a hydrating
    // client's first paint carries real translations while the
    // server-rendered markup it hydrates against still has `t()`'s
    // untranslated fallback - a hydration content mismatch on any
    // translated string. This scaffold accepts that trade-off (translate
    // before first paint, for the common case) rather than deferring the
    // catalog load until after hydration. `initLang` never rejects: a
    // failed load leaves `t()` on its raw-key fallback.
    void initLang(props.initialPage).then(() => app.mount(el))
  },
})
