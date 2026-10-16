import './app.css'
import { createInertiaApp } from '@inertiajs/react'
import type { ErrorInfo } from 'react'
import { createRoot, hydrateRoot } from 'react-dom/client'
import { layout, title } from './lib/app'

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

type Sentry = typeof import('@sentry/react')

// The SDK loads in its own chunk while the application mounts, so the first
// page never waits for it. Errors raised before it is ready, a hydration
// error included, wait in a short queue and are sent once it is. An unset
// public DSN installs nothing and never imports the SDK.
const reportingDsn = import.meta.env.VITE_REPORTING_DSN
const earlyReports: Array<(sentry: Sentry) => void> = []
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
}

async function loadReporting() {
  if (!reportingDsn) return
  try {
    const sentry = await import('@sentry/react')
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
    for (const send of earlyReports.splice(0)) send(sentry)
  } catch (error) {
    // A blocked SDK chunk must leave the application usable.
    stopEarlyReporting()
    reporter = null
    earlyReports.length = 0
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

    const errorOptions = reportingDsn ? {
      onUncaughtError: (error: unknown, info: ErrorInfo) => report((sentry) => sentry.reactErrorHandler()(error, info)),
      onCaughtError: (error: unknown, info: ErrorInfo) => report((sentry) => sentry.reactErrorHandler()(error, info)),
      onRecoverableError: (error: unknown, info: ErrorInfo) => report((sentry) => sentry.reactErrorHandler()(error, info)),
    } : {}
    const app = <App {...props} />
    if (el.hasAttribute('data-server-rendered')) {
      hydrateRoot(el, app, errorOptions)
    } else {
      createRoot(el, errorOptions).render(app)
    }
  },
}).catch((error: unknown) => {
  report((sentry) => sentry.captureException(error))
  console.error('Application bootstrap failed', error)
})
