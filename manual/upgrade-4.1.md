# Upgrading to 4.1

This chapter moves an application from Suprnova 4.0.4 to 4.1.0. Each section
matches an entry in the 4.1.0 [changelog](../CHANGELOG.md).

## Who this chapter is for

- A person upgrading an existing application from 4.0 to 4.1.
- A maintainer looking up the exact bold lead sentence of a changelog entry.

## How to use this chapter

Find the changelog entry below, then read **Before**, **Now** and **Do**.
Existing applications keep their current response policy until they register
the new middleware. New full-stack scaffolds register it automatically.

## Upgrade in this order

1. Point application dependencies and the CLI at `v4.1.0`:

   ```toml
   suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v4.3.0" }
   ```

   ```bash
   cargo update -p suprnova
   cargo install --force --git https://github.com/eas4ai/suprnova.git --tag v4.3.0 suprnova-cli
   cargo check --all-targets
   ```

2. Decide whether to install browser security headers and error reporting, using the sections below.
3. Run your tests and exercise a browser first visit, error page and Live page.

## Added entries that change behaviour

### Error reporting through Sentry-compatible trackers.

**Before:** An application supplied its own Sentry SDK and `ErrorOccurred`
listener. Background failures did not all reach that listener.

**Now:** The built-in `sentry` driver sends errors to Rustrak, Sentry or
GlitchTip. It is inert without a DSN. A bounded background queue, shared
scrubbing and shutdown flushing cover requests and background work.

**Do:** A scaffolded application turns the framework's default features off
and lists the ones it keeps. Add `reporting-sentry` to that list in
`Cargo.toml`, or the `sentry` driver is not compiled in:

```toml
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v4.3.0", default-features = false, features = [
    # ...the features already listed...
    "reporting-sentry",
] }
```

Remove the hand-written tracker listener and direct SDK capture for errors
the framework reports, then set the project's DSN:

```dotenv
REPORTING_DRIVER=sentry
REPORTING_DSN=http://your-project-key@127.0.0.1:8080/1
```

Use the DSN your tracker supplies. Existing `Exceptions::reportable`
callbacks still run first; `.stop()` prevents tracker delivery. `log` and
`null` drivers are also available. Logs and traces remain off until you set
`REPORTING_LOGS` or `REPORTING_TRACES_SAMPLE_RATE`.

For browser reporting, copy the current starter entry-point integration and
install its Sentry package for your frontend. Set `VITE_REPORTING_DSN` for an
Inertia kit; the build reads it, and `SecurityHeaders` reads the same value
to allow the tracker in CSP. Live needs only `REPORTING_PUBLIC_DSN`; its
reporter is built into the runtime. See [Error Reporting](error-reporting.md) for
all settings, the helpers and the local Rustrak commands.

### Security headers with per-request CSP nonces.

**Before:** Applications supplied their own response security middleware.

**Now:** `SecurityHeaders` supplies strict, balanced and relaxed presets, CSP
nonces, report-only mode, reporting and route overrides. New full-stack
projects register it first. Existing applications retain their current policy.

**Do:** To opt in, register the middleware first in your HTTP bootstrap, before
logging, `Inertia::install`, error pages and RenderCache:

```rust,ignore
use suprnova::{FrameworkError, SecurityHeaders, global_middleware};

pub fn try_register_http_stack() -> Result<(), FrameworkError> {
    global_middleware!(SecurityHeaders::from_env()?);
    // Keep your other middleware registrations after this one.
    Ok(())
}

pub fn register_http_stack() {
    try_register_http_stack().expect("Invalid HTTP middleware configuration");
}
```

Set `SECURITY_HEADERS=balanced` and leave
`SECURITY_HEADERS_HSTS_PRELOAD=false` unless you have chosen preload. Use
`SECURITY_HEADERS_CSP_REPORT_ONLY=true` while observing your application's
violations, then set it to `false` to enforce the policy. A reporting endpoint
is optional: `SECURITY_HEADERS_CSP_REPORT_TO=https://reports.example.com/csp`.
An invalid value returns an error naming its variable.

Add the request nonce to scripts your templates own. Inertia root templates
already receive `nonce`; views outside Inertia can receive
`suprnova::CspNonce::current()`. See [Security Headers](security-headers.md) for
the full policy and application-script examples. JSON responses receive no
nonce policy, and HSTS applies only to secure requests outside local development.

## Fixed entries that change behaviour

### Framework scripts and asset links share the request's CSP nonce.

**Before:** Framework asset tags did not consistently carry an application
nonce, and Inertia's root template nonce part was always absent.

**Now:** Under a nonce policy, Inertia and Vite, the Head facade and Live's
bootstrap use the request nonce. Inertia's head exposes a `csp-nonce` meta
element for Vite and the client. Complete nonce pages render on every request;
stitched Live cache hits use the current request nonce in body and headers.

**Do:** Keep `{{ head }}` in your Inertia root document. If you customized the
frontend entry, read the meta element and pass its nonce to `createInertiaApp`,
as current scaffolds do:

```typescript
const nonce = document.querySelector<HTMLMetaElement>('meta[property="csp-nonce"]')?.nonce;
createInertiaApp({ nonce /* keep your other existing options */ });
```

A Live bootstrap that supplies no nonce inherits the current request nonce.
Avoid supplying a different nonce. Applications without a nonce policy keep
their existing markup.

### The Live starter's Name boxes show the right name.

**Before:** In an application scaffolded with `--frontend live`, the register
form's Name box started with the word "name", and the profile form showed
"name" instead of the user's display name. Saving the profile form unchanged
renamed the user "name".

**Now:** New Live applications read the component's own `name` field.

**Do:** In an existing Live application, change the Name input in
`templates/live/components/register_form.html` and
`templates/live/components/profile_name.html`. Inside the `field` macro's call
block, `name` is the macro's own parameter, so read the field through
`component`:

```diff
-{% call input::input("name", required=true, autocomplete="name", value=name) %}
+{% call input::input("name", required=true, autocomplete="name", value=component.name) %}
```

A user already renamed "name" sets their name again on the profile page.

### Live component libraries cannot change the application's error handling.

**Before:** A third-party Live component library could call
`Exceptions::reportable`, `dont_retry` and `dont_retry_when`, and so change
how the application handles errors.

**Now:** `live:add` and `live:check` refuse a library that names those,
`dont_report`, `dont_report_when`, `dont_report_duplicates` or
`Request::csp_nonce`. A library still reports an error with
`Exceptions::report` or `try_report`.

**Do:** If you publish a component library that configures error handling,
move that setup into the application that installs it, and run
`suprnova live:check` before you publish.

### Why Suprnova diverges

Laravel provides `FrameGuard` and `Vite::useCspNonce`; its applications add
policy packages such as `spatie/laravel-csp`. Suprnova owns the middleware and
nonce plumbing so Inertia, Live and RenderCache agree on each request.

## Next

- [Error Reporting](error-reporting.md) documents drivers, scrubbing and kits.
- [Security Headers](security-headers.md) documents the policy and overrides.
- [Upgrading to 4.0](upgrade-4.0.md) covers the preceding release.
