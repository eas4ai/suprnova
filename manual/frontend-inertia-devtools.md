# Inertia DevTools

The Inertia DevTools browser extension shows each request an Inertia
application answered: what kind of visit it was, which component rendered,
each prop with its kind and where it came from, and the request and
response themselves. The extension reads that from the server, so the
server has to record it. Suprnova records each request as an entry, as
inertia-laravel 3.5.1's `DevTools` does, and serves the entries to the
extension.

## Enable DevTools

DevTools is on when `APP_ENV` names the `local` environment and off
everywhere else, an unset `APP_ENV` included. The `.env` file of a new
project sets `APP_ENV=local`, so its development server records without
any setup, while a test run with no `APP_ENV` records nothing. To decide
outright, set `INERTIA_DEVTOOLS_ENABLED`, or pass a `DevToolsConfig` to
`InertiaConfig::devtools`:

```rust
use suprnova::{DevToolsConfig, Inertia, InertiaConfig};

pub fn register_http_stack() {
    let devtools = DevToolsConfig::new()
        .enabled(true)                        // record on staging too
        .gate("viewInertiaDevtools")          // who may read entries there
        .except(["_inertia/devtools*", "_suprnova/*", "admin/*"])
        .limit(50);                           // entries kept per tab

    Inertia::install(&InertiaConfig::new().devtools(devtools))
        .expect("Inertia install failed");
}
```

`Inertia::install` and `Inertia::middleware` put `DevToolsMiddleware`
outermost in the Inertia stack when DevTools is enabled, inside the
session middleware you registered before them. Requests outside the stack,
such as an API group without it, are not recorded.

Every setting reads the variable Laravel's `config/inertia.php` reads, and
a builder call wins over the variable:

| Setting | Variable | Default |
|---|---|---|
| `enabled(bool)` | `INERTIA_DEVTOOLS_ENABLED` | unset: only when `APP_ENV` names `local` |
| `ttl_hours(hours)` | `INERTIA_DEVTOOLS_TTL_HOURS` | `24` |
| `prune_interval_secs(seconds)` | `INERTIA_DEVTOOLS_PRUNE_INTERVAL_SECONDS` | `300` |
| `limit(entries)` | `INERTIA_DEVTOOLS_LIMIT` | `100`, `0` keeps every entry |
| `gate(ability)` | `INERTIA_DEVTOOLS_GATE` | none |
| `storage_path(path)` | - | `storage_path("inertia-devtools")` |
| `except(patterns)` | - | `_inertia/devtools*`, `_suprnova/*` |
| `redact_keys(keys)` | - | `password`, `password_confirmation`, `current_password`, `token`, `_token`, `access_token`, `refresh_token`, `secret`, `client_secret`, `api_key` |
| `redact_headers(names)` | - | `cookie`, `set-cookie`, `authorization`, `proxy-authorization`, `x-xsrf-token`, `x-csrf-token` |

`INERTIA_DEVTOOLS_ENABLED` reads `true`, `1`, `on` and `yes` as on, and
`false`, `0`, `off` and `no` as off; an empty value leaves the decision to
the environment.

An `except` pattern is matched against the request path as
`Request::is` matches it: `*` matches any characters, and a leading slash
is ignored. The default skips the extension's own endpoints and the
framework's tooling routes, such as `/_suprnova/health`.

## What an entry holds

Each recorded request is one entry. Its JSON is Laravel's, key for key,
because the extension reads it:

- `__meta` - the entry `id` (a ULID), `tabUuid`, `batchId` and `visitId`
  from the extension's `X-Inertia-Devtools-Tab`, `-Parent` and `-Visit`
  headers (`batchId` for an Inertia visit only), the UTC `timestamp` and
  `utime`, the `method` and full `url`, the `component`, the
  `requestType`, the `status`, the `redirectLocation` (`X-Inertia-Location`,
  else `Location` on a `3xx`), and `serverTimingMs`.
- `http` - the request and response headers, and the request and response
  bodies.
- `props` and `propValues` - each prop of a rendered page with its kind,
  and the value the client received. See
  [Props and where they come from](#props-and-where-they-come-from).
- `route` - the route's `name`, its pattern as `uri`, the `method` for a
  rendered page, and the handler's type name as `action`
  (`app::controllers::users::index`).
- `renderSource` - the file and line of the call that built the page.
- `componentPath` - the page file under `InertiaConfig::pages_dir`, when
  there is one.

The `requestType` is decided in this order:

1. `precognition` for a request with a `Precognition` header.
2. For a request that is not an Inertia visit, `initial` when it rendered
   a page and `http` when it did not.
3. `deferred` with the extension's `X-Inertia-Devtools-Deferred` header.
4. `poll` with its `X-Inertia-Devtools-Poll` header.
5. `partial` for a partial reload (`X-Inertia-Partial-Component`).
6. `prefetch` for a prefetch (`Purpose` or `Sec-Purpose: prefetch`).
7. `navigate` for any other Inertia visit.

The request body is recorded only for an Inertia visit when the request
writes (`POST`, `PUT`, `PATCH`, `DELETE`); any other write records
`{"status": "omitted", "reason": "non-inertia-request"}`. A JSON body is
recorded as JSON. Otherwise the query and form input are recorded, with a
multipart upload summarized as its `name`, `size` and `mimeType`, never
its bytes; else the raw text, `empty` when there is none, and `binary`
when it is not UTF-8. A body is read before the handler only when its
length is declared and at most 256,000 bytes; the handler then reads the
copy kept on the request. A longer body records `too-large`, and one of
unknown length `streamed`, and both are left for the handler untouched.

The response body of a rendered page is its page object. Any other
response records its text when its `Content-Type` is textual (JSON,
`text/*`, XML or JavaScript), decoded when it is JSON, up to 256,000
bytes; otherwise the reason is `non-textual`, `streamed` or `too-large`.

## Headers and the id tag

Every recorded response carries:

- `X-Inertia-Devtools-Id` - the entry's id, which the extension fetches.
- `X-Inertia-Devtools-Parent-Out` - the incoming `X-Inertia-Devtools-Parent`
  of an Inertia visit, else the entry's own id. A prefetch is always its
  own parent. The extension sends the value back on the next request of
  the same batch, so the follow-ups of a visit, a redirect included,
  attach to it.
- `X-Inertia-Devtools-Base-Path` - the public root, when the application
  is not served at the host's root.

The first visit of a page has no response headers the extension can see
once it attaches, so a `200` HTML document that rendered a page for a
request that is not an Inertia visit gets the id in the document, before
its last `</body>`:

```html
<script data-inertia-devtools-id type="application/json">"01JA2B7Q9C3M4N5P6R7S8T9V0W"</script>
```

Under a public root the tag also carries
`data-inertia-devtools-base-path="/billing"`. An Inertia visit's JSON and a
plain HTML page get no tag.

## Props and where they come from

Each prop of a rendered page is listed under `props` with `shared` and
`inertiaType`, and with each of these that applies:

- `inertiaType` - `always`, `defer`, `optional`, `merge`, `scroll` or
  `once`, read in that order from the prop's flags, so a prop built with
  `.merge().once()` is `merge`. A deferred prop is `defer`, with its
  `deferGroup`, only on the visit the extension marks as loading deferred
  props; reloaded by hand it is a plain prop.
- `reset` - the client named the prop in `X-Inertia-Reset`.
- `once` - the client keeps the prop across visits.
- `mergeDirection` - `append` or `prepend`, for a prop that merges.
- `deepMerge` - the prop merges deeply, or matches items with `match_on`.
- `rescued` - the prop's deferred resolver failed and was rescued.
- `shareSource` - for a shared prop, the file and line of the
  `Inertia::share`, `Inertia::share_many`, `Inertia::share_data`,
  `App::inertia_share`, `App::inertia_share_lazy` or
  `App::inertia_share_once` call that shared it.
- `renderSource` - for any other prop, the line that names its key below
  the render call: `"users":` in `inertia_response!`, or `("users",` in a
  builder call.

A prop your middleware hooks share has the hooks' type name as its
`shareSource` file, at line `0`: Rust has no reflection that finds the
`share` method's line. Keys from `share_provider` and
`register_inertia_shared` providers are marked `shared` with no source.
`errors` is listed as an `always` prop every page shares.

Every top-level key is listed. A dotted key that nests is listed under its
path only when it carries something besides being a prop. `propValues`
holds the value of each listed path as the client received it, after
partial-reload filtering, so a prop the visit withheld has no value.

## The entry endpoints

The extension reads entries from two endpoints, which `DevToolsMiddleware`
answers before any route:

- `GET /_inertia/devtools/entries` - every entry's `__meta`, newest first.
  `component=Home` keeps one component's entries, `type=navigate,partial`
  keeps those request types, `exclude=poll` drops these, then `offset` and
  `limit` (at least 1) page through the rest.
- `GET /_inertia/devtools/entries/{id}` - the stored entry, or
  `404 {"message": "Not found."}` for an id that is not a ULID or names no
  entry.

In the `local` environment every request is admitted, so a broken gate
never locks you out of your own tools. Anywhere else a request is admitted
only when the configured gate ability allows the signed-in user, else the
answer is `403 {"message": "Forbidden."}`. The gate is asked about the
user and `()` as the resource; a guest is asked about as `()`:

```rust
use suprnova::Gate;

// Admins only. A guest has no gate of this name, which denies.
Gate::define::<User, ()>("viewInertiaDevtools", |user, _| user.is_admin);
```

An endpoint request reflashes the session, so the flash data a `POST`
left for the page after its redirect still reaches that page when the
extension fetches the entry first. It never becomes the session's
previous URL, so a later `back()` does not send the visitor to it. It is
never recorded, whatever `except` says. With
`InertiaConfig::register_globally(false)`, the stack on your route groups
records their routes and `Inertia::install` registers a global
`DevToolsMiddleware` that answers only the endpoints.

## Storage and redaction

Each entry is one JSON file, `<storage_path>/<id>.json`, written to a
temporary file and renamed into place. `_meta.json` lists every entry's
`__meta`, newest first, and is rewritten under a file lock, so several
processes can record into one directory; when it is missing or unreadable
it is rebuilt from the entry files. A `.gitignore` in the directory keeps
it out of your repository.

After a request, entries older than `ttl_hours` are pruned when the last
prune, noted in `_last_prune`, is at least `prune_interval_secs` old. A
browser tab keeps its newest `limit` entries.

Before an entry is written, the value of every key named by
`redact_keys` is replaced by `[REDACTED]` at any depth: in the request
and response bodies, in prop values, and in the page object. So are the
query parameters of the same names in the entry's URLs, and the values of
the `redact_headers` headers. Names are compared without case, so
`Password` is caught by `password`. A header value or multipart field that
is not text is stored as `[UNSERIALIZABLE]`, and the rest of the entry is
kept.

## Recording never breaks a response

Recording observes the response and changes nothing of it beyond the
DevTools headers and the id tag. A failure while recording drops that
entry. A failure to write the store is logged at `warn` once and pauses
recording into that directory for 30 seconds; it is logged again only
after a write has succeeded. The response goes out with its headers
either way.

### Why Suprnova diverges

- **Sources come from the compiler.** Laravel finds a render or share call
  by walking a backtrace and reflects on a route's controller. Here the
  calls take `#[track_caller]`, so a source is exact; the cost is that a
  hook-shared prop names its hooks' type at line `0`, a provider's keys
  have no source, and `route` has no `actionSource`.
- **`_suprnova/*` replaces `telescope*` and `horizon*`** in the default
  `except` list: those are Laravel packages, and `_suprnova/` is where the
  framework's own tooling routes live.
- **There is no `devtools.middleware` setting.** The endpoints run inside
  the Inertia stack, inside the session your bootstrap registered above
  it, which is what Laravel's default `web` group gives them.
- **A request body has a size limit.** Symfony keeps every body in
  memory; a Rust request streams its body to the handler. DevTools reads
  a body first only when its length is declared and at most 256,000
  bytes, and records `too-large` or `streamed` otherwise. Multipart field
  names are kept as sent, not nested.
- **The gate takes a resource.** Suprnova's gate is typed by user and
  resource, so the ability is defined for `(User, ())`, and a guest is
  `()` where Laravel passes `null`.
- **A prop value is never `[UNSERIALIZABLE]`.** A Suprnova prop is a JSON
  value once it resolves, so the marker appears only for header values and
  multipart text that are not UTF-8.
- **The write breaker is per directory**, where Laravel's is one static
  per process: a failing directory does not pause recording into another.
- **DevTools reads `APP_ENV` itself.** Laravel's unset `APP_ENV` is
  `production`, while Suprnova's `Environment::detect` reads an unset one
  as `local`. So DevTools reads the variable itself and records only when
  it is set to `local`, or with `enabled(true)`. That is why a test run
  with no `APP_ENV` records nothing, and a scaffolded project, whose `.env`
  sets it, does.

## Next

- [Inertia Responses](frontend-inertia-responses.md) - the middleware
  stack DevTools sits in, and the prop flags it classifies
- [Authorization](authorization.md) - defining the gate ability
- [HTTP Tests](http-tests.md) - `TestClient`, whose requests DevTools
  records like any other, so a test can read the entry a page left
