import { createInertiaApp } from '@inertiajs/vue3'
import { createApp, createSSRApp, h } from 'vue'
import AppLayout from './layouts/AppLayout.vue'
import GuestLayout from './layouts/GuestLayout.vue'

// Exception text can include copied URLs or credentials even without request context.
function redactBrowserText(text: string): string {
  const bounded = text.slice(0, 16_384)
  const urls = bounded.replace(/https?:\/\/[^\s)]+/gu, (value) => {
    try {
      const url = new URL(value)
      return `${url.origin}${url.pathname}`
    } catch {
      return '[URL]'
    }
  })
  const fields = /(^|[\s?&#;,{"'])([^\s?&#;=,:{}"'<>`]+)(["']?\s*[:=]\s*)/gu
  let redacted = ''
  let cursor = 0
  let match: RegExpExecArray | null
  while ((match = fields.exec(urls)) !== null) {
    const name = match[2] ?? ''
    // Decode valid escapes even beside an invalid percent sign.
    const decoded = new URLSearchParams(`${name}=`).keys().next().value ?? name
    if (!/(?:password|token|secret|key|signature|authorization|cookie|[cx]srf|session)/iu.test(decoded)) continue
    // Error text has no trustworthy value boundary. Discard the rest of a
    // credential line, including every cookie and quoted or malformed value.
    const lineEnd = urls.slice(fields.lastIndex).search(/[\r\n]/u)
    redacted += `${urls.slice(cursor, fields.lastIndex)}[REDACTED]`
    cursor = lineEnd === -1 ? urls.length : fields.lastIndex + lineEnd
    fields.lastIndex = cursor
  }
  return (redacted + urls.slice(cursor)).slice(0, 16_384)
}

type Sentry = typeof import('@sentry/vue')

// The SDK loads in its own chunk while the application mounts, so the first
// page never waits for it. Errors raised before it is ready, a hydration
// error included, wait in a short queue and are sent once it is. An unset
// public DSN installs nothing and never imports the SDK.
const reportingDsn = import.meta.env.VITE_REPORTING_DSN
const earlyReports: Array<(sentry: Sentry) => void> = []
// Work that must run once the SDK is ready, kept apart from the bounded
// error queue so a burst of early errors cannot crowd it out.
const onReporterReady: Array<(sentry: Sentry) => void> = []
// `undefined` while the SDK loads, `null` once it could not.
let reporter: Sentry | null | undefined = reportingDsn ? undefined : null

function report(send: (sentry: Sentry) => void) {
  if (reporter) send(reporter)
  else if (reporter === undefined && earlyReports.length < 32) earlyReports.push(send)
}

function reportEarlyError(event: ErrorEvent) {
  report((sentry) => sentry.captureException(event.error ?? event.message))
}

function reportEarlyRejection(event: PromiseRejectionEvent) {
  report((sentry) => sentry.captureException(event.reason))
}

function stopEarlyReporting() {
  window.removeEventListener('error', reportEarlyError)
  window.removeEventListener('unhandledrejection', reportEarlyRejection)
}

if (reportingDsn) {
  window.addEventListener('error', reportEarlyError)
  window.addEventListener('unhandledrejection', reportEarlyRejection)
  // Vue recovers from a hydration mismatch after logging it, so the log
  // line is the only signal. Watch for it from the first render.
  const original = console.error
  console.error = (...args: unknown[]) => {
    const message = args.filter((value): value is string => typeof value === 'string').join(' ')
    if (/hydrat/i.test(message)) report((sentry) => sentry.captureException(new Error('Vue hydration mismatch')))
    original.apply(console, args)
  }
}

async function loadReporting() {
  if (!reportingDsn) return
  try {
    const sentry = await import('@sentry/vue')
    sentry.init({
      dsn: reportingDsn,
      sendDefaultPii: false,
      sendClientReports: false,
      // Native global handlers and framework hooks own errors. Callback wrappers
      // can temporarily suppress an independent rejection while deduplicating.
      integrations: (defaults) => [
        ...defaults.filter((integration) => !['Breadcrumbs', 'HttpContext', 'BrowserSession', 'Vue', 'BrowserApiErrors'].includes(integration.name)),
      ],
      beforeSend(event) {
        // Vue and Svelte emit recoverable hydration failures through the console.
        // Other console logs are not error events.
        if (event.logger === 'console' && !/hydrat/i.test(event.message ?? '')) return null
        if (event.message) event.message = redactBrowserText(event.message)
        for (const exception of event.exception?.values ?? []) {
          if (exception.type) exception.type = redactBrowserText(exception.type)
          if (exception.value) exception.value = redactBrowserText(exception.value)
          for (const frame of exception.stacktrace?.frames ?? []) {
            if (frame.function) frame.function = redactBrowserText(frame.function)
            if (frame.filename) frame.filename = redactBrowserText(frame.filename)
            delete frame.vars
            delete frame.pre_context
            delete frame.context_line
            delete frame.post_context
          }
        }
        // Browser state, component props, cookies and URLs are never error context.
        delete event.request
        delete event.user
        delete event.extra
        delete event.breadcrumbs
        return event
      },
    })
    // The SDK's own global handlers take over from here.
    stopEarlyReporting()
    reporter = sentry
    for (const ready of onReporterReady.splice(0)) ready(sentry)
    for (const send of earlyReports.splice(0)) send(sentry)
  } catch (error) {
    // A blocked SDK chunk must leave the application usable.
    stopEarlyReporting()
    reporter = null
    earlyReports.length = 0
    onReporterReady.length = 0
    console.error('Browser reporting could not initialize', error)
  }
}
void loadReporting()

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
    if (reportingDsn) {
      // Until the SDK's Vue integration wraps this handler, it reports a
      // component error itself; afterwards the integration reports it.
      app.config.errorHandler = (error) => {
        if (!reporter) report((sentry) => sentry.captureException(error))
        console.error(error)
      }
      const attach = (sentry: Sentry) =>
        sentry.addIntegration(sentry.vueIntegration({ app, attachProps: false }))
      if (reporter) attach(reporter)
      else if (reporter === undefined) onReporterReady.push(attach)
    }
    app.use(plugin)

    app.mount(el)
  },
}).catch((error: unknown) => {
  report((sentry) => sentry.captureException(error))
  console.error('Application bootstrap failed', error)
})
