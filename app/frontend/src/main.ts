import './app.css'
import { createInertiaApp } from '@inertiajs/svelte'
import { hydrate, mount } from 'svelte'

// No hand-rolled CSRF header here, on purpose. The Inertia client this app
// locks (`@inertiajs/core` 3.8.0) sends its visits over XMLHttpRequest and,
// whenever the `XSRF-TOKEN` cookie the Suprnova CSRF middleware sets is
// present, echoes it back in the `X-XSRF-TOKEN` header itself, per request
// (`inertia-3.8.0/packages/core/src/xhrHttpClient.ts:84-85`). Reading
// `<meta name="csrf-token">` once at module load and pinning it into a
// header instead captures a token that logging in rotates, and the next
// visit - the logout - is refused with a 419. Server-side verification is
// unchanged: the CSRF middleware still checks the token on every
// state-changing request.

createInertiaApp({
  // The `@inertiajs/vite` plugin in `vite.config.ts` resolves `name` to
  // `./pages/${name}.svelte`.
  pages: './pages',
  setup({ el, App, props }) {
    if (el?.hasAttribute('data-server-rendered')) {
      hydrate(App, { target: el, props })
    } else {
      mount(App, { target: el!, props })
    }
  },
})
