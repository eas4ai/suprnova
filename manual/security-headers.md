# Security Headers

`SecurityHeaders` adds browser security policy to responses through Suprnova's
own middleware chain. New full-stack projects register the balanced preset.
Existing applications opt in by registering the middleware; upgrading alone
does not change their responses.

## Register the middleware

Register it first in the HTTP bootstrap, before logging, `Inertia::install`,
error pages and RenderCache. The framework also sorts `SecurityHeaders` to the
outermost application position regardless of registration order or middleware priority.
Use typed registration: `global_middleware!`, `MiddlewareRegistry::append` or
`prepend`, or `Server::middleware`. Those APIs retain the type that ordering
and the server error boundary need; a manually erased middleware closure
does not retain that information. The built-in request-id scope stays outside
the application stack.
Route and group security overrides run immediately inside it, before RenderCache
lookup. The response returns through security headers last, so cache hits and
error pages retain their policy.

```rust,ignore
use suprnova::{FrameworkError, SecurityHeaders, global_middleware};

pub fn try_register_http_stack() -> Result<(), FrameworkError> {
    global_middleware!(SecurityHeaders::from_env()?);
    // Register the remaining HTTP middleware here.
    Ok(())
}
```

The scaffold keeps a `register_http_stack()` wrapper for the application's
HTTP bootstrap hook. It calls `try_register_http_stack()` and fails at boot if
configuration is invalid. The fallible sibling also returns Inertia install errors. A server-substituted error page keeps the other
security headers and supplies its own CSP, which allows no script.

## Choose a preset

Use `SecurityHeaders::strict()`, `SecurityHeaders::balanced()` or
`SecurityHeaders::relaxed()` for programmatic configuration. `from_env()` reads
the variables below and defaults to balanced. `SecurityHeaders::off()` supplies
no headers.

| Header | Strict | Balanced | Relaxed |
|---|---|---|---|
| `Content-Security-Policy` | Strict policy below | Balanced policy below | Omitted |
| `Strict-Transport-Security` | `max-age=63072000; includeSubDomains` | `max-age=31536000` | `max-age=31536000` |
| `X-Frame-Options` | `DENY` | `SAMEORIGIN` | `SAMEORIGIN` |
| `X-Content-Type-Options` | `nosniff` | `nosniff` | `nosniff` |
| `Referrer-Policy` | `no-referrer` | `strict-origin-when-cross-origin` | `strict-origin-when-cross-origin` |
| `Permissions-Policy` | `camera=(), microphone=(), geolocation=(), payment=(), usb=(), accelerometer=(), gyroscope=(), magnetometer=(), browsing-topics=(), display-capture=()` | Strict's list without `payment=()` | Omitted |
| `Cross-Origin-Opener-Policy` | `same-origin` | `same-origin-allow-popups` | Omitted |
| `Cross-Origin-Embedder-Policy` | `require-corp` | Omitted | Omitted |
| `Cross-Origin-Resource-Policy` | `same-origin` | `same-site` | Omitted |

Strict's CSP is:

```text
default-src 'self'; script-src 'nonce-N' 'strict-dynamic'; style-src 'self' 'nonce-N'; img-src 'self' data:; font-src 'self'; connect-src 'self'; object-src 'none'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'
```

Balanced's CSP is:

```text
default-src 'self'; script-src 'self' 'nonce-N'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob: https:; font-src 'self' data: https:; connect-src 'self'; object-src 'none'; base-uri 'self'; form-action 'self'; frame-ancestors 'self'
```

`N` represents the fresh request nonce. Balanced runs script files from the
application's own origin, such as the Live components' scripts under
`/suprnova-ui/`, and any script that carries the nonce. An inline script an
attacker injects carries no nonce and is blocked. Strict adds
`'strict-dynamic'` and drops `'self'`, so under strict every script tag needs
the nonce, the Live components' tags included. Balanced permits inline style
attributes because components and kits use them. Adding a nonce to `style-src`
would disable that permission. Strict requires authorized style elements and
does not permit inline style attributes.

In `APP_ENV=local` or `development`, the framework also permits the configured
Vite dev server in `script-src` and its origin and HMR websocket in
`connect-src`. It reads the same configuration as the Inertia Vite integration,
so `suprnova serve` works with the balanced policy.

## Configure headers

The `header` builder changes or removes an individual header. Header names and
values are validated and errors return `FrameworkError`:

```rust,ignore
use suprnova::{FrameworkError, SecurityHeaders};

fn policy() -> Result<SecurityHeaders, FrameworkError> {
    SecurityHeaders::balanced()
        .header("referrer-policy", Some("same-origin"))?
        .header("cross-origin-opener-policy", None)
}
```

Use `.content_security_policy(Some(policy))` to set a typed
`ContentSecurityPolicy`, or `.content_security_policy(None)` to remove it.
`ContentSecurityPolicy::strict()` and `::balanced()` provide the built-in
policies; `ContentSecurityPolicy::new(template)?` validates a custom policy.
The `{nonce}` placeholder expands to the request nonce:

```rust,ignore
use suprnova::{ContentSecurityPolicy, FrameworkError, SecurityHeaders};

fn custom_policy() -> Result<SecurityHeaders, FrameworkError> {
    let csp = ContentSecurityPolicy::new(
        "default-src 'self'; script-src 'nonce-{nonce}' 'strict-dynamic'; object-src 'none'; base-uri 'self'; frame-ancestors 'self'",
    )?;
    Ok(SecurityHeaders::balanced().content_security_policy(Some(csp)))
}
```

A header already set on the response wins. This preserves the policy on
Live's transport and upload responses and any policy a handler supplies.

## Environment and reporting

| Variable | Default | Values |
|---|---|---|
| `SECURITY_HEADERS` | `balanced` | `strict`, `balanced`, `relaxed`, `off` |
| `SECURITY_HEADERS_CSP_REPORT_ONLY` | `false` | Boolean |
| `SECURITY_HEADERS_CSP_REPORT_TO` | Unset | A reporting endpoint URL |
| `SECURITY_HEADERS_HSTS_PRELOAD` | `false` | Boolean |

`SecurityHeaders::from_env()` returns an error naming a variable when its
value is invalid. See [Environment Variables](env-vars.md).

Report-only mode sends `Content-Security-Policy-Report-Only` in place of
`Content-Security-Policy`. It reports violations without enforcing that CSP.
Enable it while checking a policy against your application:

```env
SECURITY_HEADERS=balanced
SECURITY_HEADERS_CSP_REPORT_ONLY=true
SECURITY_HEADERS_CSP_REPORT_TO=https://reports.example.com/csp
SECURITY_HEADERS_HSTS_PRELOAD=false
```

A reporting URL adds a CSP `report-to` directive and a `Reporting-Endpoints`
header identifying its destination. Your application or reporting service
must accept the browser's violation reports. Programmatic equivalents are
`.report_only(true)` and `.report_to(Some("https://reports.example.com/csp"))?`.
Pass `None` to `report_to` to remove the destination.

## HSTS and HTTPS

`Strict-Transport-Security` is sent only when `Request::secure()` is true and
never in the local environment. Behind a TLS-terminating proxy, configure
`APP_TRUSTED_PROXIES` so the framework can trust its forwarded scheme. An
untrusted client's forwarded header cannot turn on HSTS.

The strict preset includes subdomains; balanced and relaxed do not. Preload is
opt-in through `SECURITY_HEADERS_HSTS_PRELOAD=true` or `.hsts_preload(true)`.
Choose it only when the domain and its subdomains meet the browser preload
requirements. Setting the token does not submit the domain to a preload list.

## Read and use the nonce

For a nonce policy, the middleware generates 128 random bits encoded as
unpadded base64url before it calls the handler. Each request gets a different
value. Read it through `request.csp_nonce()` or `CspNonce::current()`:

```rust,ignore
use suprnova::{CspNonce, Request};

fn request_nonce(request: &Request) -> Option<String> {
    request.csp_nonce().or_else(CspNonce::current)
}
```

Tokio does not inherit task-local values automatically. Capture the nonce scope
before spawning work that renders request assets:

```rust,ignore
let task = tokio::spawn(suprnova::CspNonce::propagate(async {
    suprnova::CspNonce::current()
}));
let nonce = task.await?;
```

The request task's context makes `CspNonce::current()` available to view and
head rendering code that holds no request. Without a nonce policy it returns
`None`. A JSON or other non-HTML response gets the other configured security
headers and no nonce-bearing CSP.

The framework stamps its executable scripts and script or style links:

- Inertia's default document and application root template `head` part include
  `<meta property="csp-nonce" nonce="...">`. Vite reads this element; each
  scaffold entry passes its nonce to `createInertiaApp` for injected styles.
- Vite's development preamble, scripts, modulepreload links and stylesheets
  carry it, including `Vite::tags` and `Vite::to_html` outside Inertia.
- `Head` stamps scripts and `preload`, `modulepreload` and `stylesheet` links.
  Preconnect, DNS-prefetch and prefetch hints need no nonce.
- Live's bootstrap uses it when the caller has not selected a nonce with
  `LiveBootstrapOptions::with_nonce`. Its scripts, modulepreload, preload and
  stylesheet links carry it.

Inertia page data and Live configuration use non-executing JSON elements and
need no nonce. Without a nonce policy, framework markup keeps its previous
bytes.

### Add an application script

An [Inertia root template](frontend-inertia-responses.md) receives `nonce` as
`Option<&str>`. Match it before writing a script. Askama escapes the attribute
value:

```html
{% if let Some(nonce) = nonce %}
<script nonce="{{ nonce }}" src="https://js.stripe.com/v3/"></script>
{% endif %}
```

The same shape authorizes an analytics tag or a script you host yourself. An
Askama view outside Inertia can receive `CspNonce::current()` as a field. Copy
the nonce only from the current request.

The nonce lets the script run. What the script then loads still meets the
rest of the policy, and the balanced policy's `default-src 'self'` blocks
another origin's frames, API calls and the scripts it loads itself. Stripe.js,
for example, also needs its own origin, its frames and its API. Add them to a
custom policy that keeps the nonce:

```rust,ignore
let csp = suprnova::ContentSecurityPolicy::new(
    "default-src 'self'; script-src 'self' 'nonce-{nonce}' https://js.stripe.com; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob: https:; font-src 'self' data: https:; connect-src 'self' https://api.stripe.com; frame-src https://js.stripe.com https://hooks.stripe.com; object-src 'none'; base-uri 'self'; form-action 'self'; frame-ancestors 'self'",
)?;
let headers = suprnova::SecurityHeaders::balanced().content_security_policy(Some(csp));
```

Balanced leaves `payment` out of `Permissions-Policy`, so Stripe's Apple Pay
and Google Pay buttons work, and sends `Cross-Origin-Opener-Policy:
same-origin-allow-popups`, so a popup sign-in or checkout window can report
back to the page. Strict turns both off.

To permit another origin's scripts without a nonce on each tag, add the origin
to `script-src` in a custom policy. Under strict this does not work:
`'strict-dynamic'` makes supporting browsers ignore origin allowlists, so give
the tag the nonce instead. For example:

```rust,ignore
let csp = suprnova::ContentSecurityPolicy::new(
    "default-src 'self'; script-src 'self' 'nonce-{nonce}' https://analytics.example.com; object-src 'none'; base-uri 'self'; frame-ancestors 'self'",
)?;
let headers = suprnova::SecurityHeaders::balanced().content_security_policy(Some(csp));
```

A nonce authorizes a script to execute. It does not make its contents trusted.
Choose external scripts with the same care as application dependencies.

## Override a route or group

Attach `SecurityHeadersOverride` as ordinary route or group middleware. It
records changes for the outer global middleware to apply:

```rust,ignore
use suprnova::{FrameworkError, Router, SecurityHeadersOverride, get, group, routes};

fn router() -> Result<Router, FrameworkError> {
    let route_policy = SecurityHeadersOverride::new()
        .header("referrer-policy", Some("no-referrer"))?;
    let group_policy = SecurityHeadersOverride::new()
        .header("cross-origin-resource-policy", None)?;
    Ok(routes! {
        get!("/report", handlers::report).middleware(route_policy),
        group!("/shared", {
            get!("/image", handlers::image),
        }).middleware(group_policy),
    })
}
```

Use `.content_security_policy(None)` on the override to turn off CSP for that
route or group. A route override can change or turn off any middleware header;
a header the handler already set still wins. Review frame permissions together:
`X-Frame-Options` and CSP's `frame-ancestors` should describe the same access.

## Cache nonce pages

A Complete page under a nonce CSP renders on every request. RenderCache never
stores that complete rendering, since replaying it would reuse a nonce. A
stitched Live document can be cached: each hit fills its nonce holes and
nonce-bearing headers with the current request's nonce, so body and CSP agree.
See [RenderCache](render-cache.md).

### Why Suprnova diverges

Laravel ships `FrameGuard` for `X-Frame-Options` and `Vite::useCspNonce` for asset
tags, but no CSP, HSTS or policy middleware. Applications use packages such as
`spatie/laravel-csp`. Suprnova includes `SecurityHeaders` so its own Inertia,
Live and RenderCache writers share one per-request nonce and its scaffolds
start with a browser policy.

## Next

- [Middleware](middleware.md) covers global order and route middleware.
- [Inertia Responses](frontend-inertia-responses.md) covers application root documents.
- [Environment Variables](env-vars.md) lists every configuration setting.
