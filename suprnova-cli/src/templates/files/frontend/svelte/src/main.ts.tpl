import './app.css'
import { createInertiaApp } from '@inertiajs/svelte'
import { hydrate, mount } from 'svelte'
import AppLayout from './layouts/AppLayout.svelte'
import GuestLayout from './layouts/GuestLayout.svelte'
import { pageTitle } from './lib/title'

// No hand-rolled CSRF header here, on purpose. The Inertia client this
// scaffold installs (`@inertiajs/*`, pinned `^3.8.0`) sends its visits over
// XMLHttpRequest and, whenever the `XSRF-TOKEN` cookie the Suprnova CSRF
// middleware sets is present, echoes it back in the `X-XSRF-TOKEN` header
// itself, per request. Reading `<meta name="csrf-token">` once at module
// load and pinning it into a header instead captures a token that logging
// in rotates, and the next visit - the logout - is refused with a 419.
// Server-side verification is unchanged: the CSRF middleware still checks
// the token on every state-changing request.

// Under a nonce-based Content Security Policy the server writes the
// request's nonce on `<meta property="csp-nonce">`. The Inertia client
// stamps the `nonce` given here on the styles it injects (the progress bar,
// the error dialog); without a policy the element is absent and the nonce
// stays `undefined`. Browsers hide a `nonce` attribute's value, so it is
// read from the element's `nonce` property.
const nonce =
  document.querySelector<HTMLMetaElement>('meta[property="csp-nonce"]')?.nonce || undefined

createInertiaApp({
  // `@inertiajs/vite` turns this into the page resolver: `Notes/Index`
  // loads `./pages/Notes/Index.svelte`, each page in its own chunk.
  pages: './pages',
  // Pages under `auth/` sit in the guest frame and every other page in the
  // application frame. A layout given here stays mounted across visits
  // between the pages that share it.
  layout: (name) => (name.startsWith('auth/') ? GuestLayout : AppLayout),
  // The Svelte adapter (3.8.0) does not read `title`; `components/Head.svelte`
  // applies the same `pageTitle` through `<svelte:head>`. It is passed so
  // the options read as they do in the other adapters.
  title: pageTitle,
  // Head tags a handler sets from Rust arrive as the `head` page prop; the
  // client keeps them in the document across visits.
  serverHead: true,
  nonce,
  setup({ el, App, props }) {
    // This entry runs in the browser; `ssr.ts` renders on the server.
    if (!el) {
      return
    }
    if (el.hasAttribute('data-server-rendered')) {
      hydrate(App, { target: el, props })
    } else {
      mount(App, { target: el, props })
    }
  },
})
