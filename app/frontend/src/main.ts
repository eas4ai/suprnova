import './app.css'
import { createInertiaApp, type ResolvedComponent } from '@inertiajs/svelte'
import { hydrate, mount } from 'svelte'

// No hand-rolled CSRF header here, on purpose. The Inertia client this app
// locks (`@inertiajs/core` 3.1.1) sends its visits over XMLHttpRequest and,
// whenever the `XSRF-TOKEN` cookie the Suprnova CSRF middleware sets is
// present, echoes it back in the `X-XSRF-TOKEN` header itself, per request.
// Reading `<meta name="csrf-token">` once at module load and pinning it into
// a header instead captures a token that logging in rotates, and the next
// visit - the logout - is refused with a 419. Server-side verification is
// unchanged: the CSRF middleware still checks the token on every
// state-changing request.

createInertiaApp({
  resolve: (name) => {
    const pages = import.meta.glob<ResolvedComponent>('./pages/**/*.svelte', {
      eager: true,
    })
    return pages[`./pages/${name}.svelte`]
  },
  setup({ el, App, props }) {
    if (el?.hasAttribute('data-server-rendered')) {
      hydrate(App, { target: el, props })
    } else {
      mount(App, { target: el!, props })
    }
  },
})
