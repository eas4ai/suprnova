# Inertia Responses

Inertia responses are how a Suprnova handler ships state to a Svelte / React /
Vue page component. Every handler that renders an Inertia page returns one,
built either through the [`inertia_response!`](#the-inertia_response-macro)
macro (for typed, compile-time-checked eager props) or the
[`InertiaResponse`](#the-inertiaresponse-builder) builder (for everything
else - lazy props, deferred props, merge, once, scroll, flash). This
chapter covers the response surface end-to-end: the macro, the builder, the
v3 protocol features (partial reloads, history encryption, version
detection), shared data via `App::inertia_share*`, and the flash bag carried
across redirects.

If you haven't picked a frontend yet, [Frontend Overview](frontend.md) and
[Page Components](frontend-pages.md) come first; this chapter assumes the
SPA bridge is wired and focuses on what your handler returns.

## The `inertia_response!` macro

The macro is the shortest path from a handler to a typed eager page. It
takes the current request, a component name, and a props expression:

```rust
use suprnova::{Request, Response, inertia_response, InertiaProps};

#[derive(InertiaProps)]
pub struct HomeProps {
    pub title: String,
    pub message: String,
}

pub async fn index(req: Request) -> Response {
    inertia_response!(&req, "Home", HomeProps {
        title: "Welcome".into(),
        message: "Hello from Suprnova!".into(),
    })
}
```

Three things to know:

- **The leading `&req` is required.** The macro reads `X-Inertia` headers,
  the URL, and the partial-reload filtering headers off the request, so it
  needs the request value (or a reference). Without it, partial reloads
  would silently break.
- **Component existence is checked at compile time.** The macro looks for
  `frontend/src/pages/<Component>.{svelte,tsx,jsx,vue}`; if no file
  matches, the build fails with a "did you mean…?" suggestion sourced from
  the actual filenames on disk. Nested paths work the same way -
  `inertia_response!(&req, "Admin/Dashboard", …)` resolves
  `frontend/src/pages/Admin/Dashboard.svelte` (or your frontend's
  extension).
- **The macro expands to an `await`ed `Result`.** Your handler must
  return [`Response`](error-model.md) (which is
  `Result<HttpResponse, HttpResponse>`) or another type that absorbs
  `FrameworkError` through `?` / `From`. Failures during prop
  serialization or response building are returned as `Err`, not panics.

For a page with no logic at all - about, terms, privacy - skip the
handler entirely and declare the route:

```rust
use suprnova::Router;
use serde_json::json;

let router = Router::new().inertia("/about", "About", json!({ "team_size": 4 }));
```

See [Routing](routing.md#router-level-redirects-and-views). The
component there is a runtime string, so it doesn't get this macro's
compile-time existence check - that's the trade for not writing the
handler.

### JSON-style props

For prototyping and tiny pages you can skip the typed struct:

```rust
inertia_response!(&req, "Dashboard", {
    "user": { "name": "John" },
    "stats": { "visits": 1234 }
})
```

The macro still validates the component file. The trade-off is that you
lose the typed-prop chain - no `#[derive(InertiaProps)]`, no automatic
TypeScript generation, no compile-time check that the frontend's
expected shape matches.

### Optional config override

The macro accepts an optional trailing `InertiaConfig` for per-response
overrides (different SSR settings, a custom default title for one page):

```rust
let cfg = InertiaConfig::new().default_title("Reports");
inertia_response!(&req, "Reports/Index", props, cfg)
```

Most apps register a single config at boot via [`Inertia::install`](#bootstrap-inertia-install)
and never touch this argument - the installed config is already what
every response starts from. Pass one here only to override the installed
config for a single page.

## `#[derive(InertiaProps)]`

`InertiaProps` emits a `Serialize` impl whose key names match your field
names. It exists so the typed-props path stays terse and so the
TypeScript generator (`suprnova generate-types`) has a marker to find:

```rust
use suprnova::InertiaProps;

#[derive(InertiaProps)]
pub struct UserProps {
    pub name: String,
    pub email: String,
    pub role: String,
    pub is_active: bool,
}
```

Nested types compose normally - fields can be `Vec<T>`, `Option<T>`,
nested structs, anything `Serialize`-able. The nested types themselves
don't have to derive `InertiaProps`; they just need `Serialize`. Use
`#[derive(InertiaProps)]` on the *top-level* props struct and you get
the automatic TypeScript surface (see [TypeScript Types](frontend-typescript-types.md))
for the whole tree.

## The `InertiaResponse` builder

The macro covers eager typed props. Anything else - lazy, optional, deferred,
mergeable, cached-on-client, flash, history-encryption overrides - uses
the builder directly:

```rust
use suprnova::{InertiaResponse, Request, Response, FrameworkError, HttpResponse};

pub async fn show(req: Request) -> Response {
    let resp = InertiaResponse::new("Posts/Show")
        .with("title", "Welcome")
        .with("post", load_post(42).await?)
        // Lazy: closure runs only when the prop will actually be sent
        // (initial visit, or partial reload that requests this key).
        .lazy("recent_activity", || async {
            Ok::<_, FrameworkError>(load_activity().await?)
        })
        // Optional: never sent on initial visits; the client must
        // explicitly ask for the key via X-Inertia-Partial-Data.
        .optional("permissions", || async {
            Ok::<_, FrameworkError>(load_permissions().await?)
        })
        // Defer: skipped on the initial render; the client issues a
        // follow-up XHR and the closure runs then.
        .defer("notifications", || async {
            Ok::<_, FrameworkError>(load_notifications().await?)
        })
        // Merge: append-into-existing on partial reloads ("load more").
        .merge("rows", next_page().await?)
        // Once: cached client-side across navigations; resolver skipped
        // on subsequent visits unless server forces refresh.
        .once("plans", || async {
            Ok::<_, FrameworkError>(load_plan_catalog().await?)
        })
        // Flash: one-shot toast; appears under `page.flash`, not `props`.
        .flash("toast", serde_json::json!({"type":"info","msg":"Saved"}))
        .resolve(&req)
        .await
        .map_err(HttpResponse::from)?;
    Ok(resp)
}
```

| Method | Purpose | Maps to Laravel |
|---|---|---|
| `.with(k, v)` | Eager prop, honours partial-reload filtering | typed prop |
| `.always(k, v)` | Eager prop, ignores partial-reload filters | `Inertia::always(…)` |
| `.always_with(k, ‖)` | Async resolver, ignores partial-reload filters | `Inertia::always(fn () => …)` |
| `.lazy(k, ‖)` | Resolver runs only when prop will be sent | `fn () => …` closure |
| `.optional(k, ‖)` | Never on initial visit; must be requested explicitly | `Inertia::optional(…)` |
| `.defer(k, ‖)` / `.defer_with(...)` | Initial-visit-skipped; follow-up XHR triggers resolution | `Inertia::defer(…)` |
| `.merge` / `.merge_prepend` / `.deep_merge` / `.merge_with` | Combine with existing client state on partial reloads | `Inertia::merge` / `deepMerge` |
| `.once(k, ‖)` / `.once_with(…)` | Client caches across navigations | `Inertia::once(…)` |
| `.scroll` / `.scroll_with` / `.scroll_wrapped` / `.scroll_with_wrapped` / `.paginate` (via `Inertia::paginate`) | Infinite-scroll pagination | `Inertia::scroll(…)` |
| `.flash(k, v)` | One-shot value under `page.flash` (not `props`) | `session()->flash(…)` |
| `.title(…)` | Default `<title>` for the HTML shell | `Inertia::render(…)->title(…)` |
| `.encrypt_history(bool)` | Per-response history encryption | `Inertia::encryptHistory(…)` |
| `.clear_history()` | Force history key rotation on **this** page | `Inertia::clearHistory()` |
| `.preserve_fragment(bool)` | Keep `#fragment` after Inertia visit | `Inertia::preserveFragment()` |

Eager builder methods have `try_*` siblings (`try_with`, `try_always`,
`try_merge_with`, `try_scroll`, `try_scroll_wrapped`, `try_flash`) that return
`Result<Self, FrameworkError>` when a value's `Serialize` impl might
fail at runtime - the infallible methods convert the panic into a 500
via [the panic boundary](error-model.md), so reach for `try_*` when
you'd rather handle the failure explicitly.

`.clear_history()` marks the response you are building. A logout handler
redirects, and the browser discards the redirect's response - so the login
page, not the logout response, is the one that has to carry the flag.
`App::clear_history()` is the fix for that case - it's a free function, not
a builder method, so it isn't in the table above. It flashes a one-shot
session flag that the next Inertia page object turns into
`clearHistory: true`. It needs a session scope, and it survives exactly
one hop.

Call it **after** `Auth::logout()` / `Auth::logout_and_invalidate()`, not
before - invalidation flushes the whole session, and the flag lives in
that session, so flashing it first only gets erased by the flush:

```rust
use suprnova::{App, Auth, Redirect, Response};

pub async fn logout() -> Response {
    Auth::logout_and_invalidate().await?;
    App::clear_history();
    Redirect::to("/login").into()
}
```

### Composing flags on one prop

The methods above each set one flag. A prop can carry several, and some
combinations are how the Inertia protocol expects real pages to work: a
deferred list that appends into what the client already rendered, a
merge prop the client caches across navigations, an optional prop with
its own cache key. Build the prop with `Prop`, then attach it with
`.prop(key, prop)`:

```rust
use suprnova::{InertiaResponse, Prop};
use serde_json::json;

InertiaResponse::new("Feed/Index").prop(
    "posts",
    Prop::lazy(|| async { json!([{ "id": 1 }]) })
        .defer()
        .merge()
        .match_on("id"),
)
```

That prop is skipped on the first render and announced under
`deferredProps`. The client issues its follow-up request, the resolver
runs, and the value arrives with a `mergeProps` instruction, so it
appends to the list already on screen instead of replacing it.

The flags fall into five groups:

| Group | Methods | Effect |
|---|---|---|
| Visibility | `.always()`, `.optional()`, `.defer()` | Mutually exclusive; the last call wins |
| Defer detail | `.group(name)`, `.rescue()` | Read only when the prop is deferred |
| Merge | `.merge()`, `.prepend()`, `.deep_merge()`, `.match_on(fields)`, `.merge_with_path(path)` | How the client folds the value in, and at which path |
| Client cache | `.once()`, `.as_key(key)`, `.until(ms)`, `.fresh()` | Whether the client keeps the value across navigations |
| Scroll | `.scroll(metadata)`, `.scroll_wrap(key)` | Infinite-scroll `scrollProps` entry plus unconditional merge metadata; `.scroll_wrap` read only when `.scroll` is set |

Sources are `Prop::eager(value)`, `Prop::lazy(closure)`,
`Prop::from_resolver(resolver)` for a resolver you built yourself, and
`Prop::absent()` for a prop that never reaches the response - what
`when_loaded!` returns for an unloaded relation.

Two rules are worth knowing before you compose:

- **Visibility is one setting, not three flags.** `.always().optional()`
  is an optional prop, and `.optional().always()` is an always prop.
  Neither is an error; the earlier call is erased.
- **Metadata follows the partial-reload lists, not the value.** A prop's
  `mergeProps`, `onceProps`, and `scrollProps` entries are emitted
  whenever the key passes `X-Inertia-Partial-Data` and
  `X-Inertia-Partial-Except`, even on a visit where the value itself is
  withheld. That is what carries the merge instruction across a deferred
  prop's two requests. Two consequences follow:
  - An `.always().merge()` prop outside the requested set still sends its
    value and does not send its merge instruction, so the client replaces
    rather than appends.
  - `scrollProps` has one extra condition on top of the lists: a
    `.scroll().defer()` prop announces its merge instruction on a
    non-partial visit but ships no cursor there, because nothing is on
    screen yet for a cursor to describe. Every matched partial reload
    gets the cursor, whether or not that request also resolves the
    value.
  - `deferredProps` is the one block the lists never govern. It is
    dropped whole on any matched partial reload, no matter what the
    lists say - Laravel's `resolveDeferredProps` returns `[]` the
    moment the request is partial. A partial reload is the client
    working through announcements it already holds, so re-announcing
    the keys it left out of this round would send it back for them
    again. A partial reload aimed at a *different* component is a
    standard visit for every gate, announcements included.

`.group(name)` and `.rescue()` are stored on any prop but only read when
the prop is deferred, so `.rescue().defer()` and `.defer().rescue()`
mean the same thing. A scroll prop takes its merge direction from the
client's `X-Inertia-Infinite-Scroll-Merge-Intent` header, so `.merge()`
and `.prepend()` on a scroll prop are redundant and not read.
`.deep_merge()` is the exception: it routes the prop into
`deepMergeProps` instead of `mergeProps`, the same way Laravel's
`ScrollProp` does.

### Merge strategies and infinite scroll

`.merge` (append), `.merge_prepend`, and `.deep_merge` cover the common
"load more" cases. To diff-merge - update rows the client already holds
instead of duplicating them - reach for `.merge_with` with an explicit
`MergeStrategy` carrying a `match_on` key:

```rust
use suprnova::{InertiaResponse, MergeStrategy};

InertiaResponse::new("Feed/Index")
    .merge_with(
        "posts",
        next_page,                                     // the new page slice
        MergeStrategy::Append { match_on: Some(vec!["id".into()]) },
    )
```

`match_on` names the field(s) the client dedupes on (emitted to the page
object as `matchPropsOn`) - one field or several, the same as
`Prop::match_on` (below) - so a refetch that overlaps the current window
replaces matching rows in place rather than appending copies. `Prepend`
and `Deep` take the same `match_on`.

`MergeStrategy` is the one-call form. `Prop::merge()` / `.prepend()` /
`.deep_merge()` / `.match_on(field)` are the same settings as separate
flags, for when the prop also needs a visibility or cache flag - see
[Composing flags on one prop](#composing-flags-on-one-prop).

`.match_on` takes one field or several in one call -
`.match_on(["id", "slug"])` and `.match_on("id").match_on("slug")` emit
the same `matchPropsOn`.

To merge only part of a prop's value instead of the whole thing, name
the nested field with `.merge_with_path`:

```rust
use suprnova::{InertiaResponse, Prop};
use serde_json::json;

InertiaResponse::new("Feed/Index").prop(
    "posts",
    Prop::eager(json!({ "data": next_page, "meta": meta }))
        .merge()
        .merge_with_path("data")
        .match_on("data.id"),
)
```

`mergeProps` now carries `"posts.data"` instead of `"posts"`, so only
`props.posts.data` folds into what the client already holds -
`props.posts.meta` is replaced outright, like any non-merge prop. Calls
accumulate, so a prop with two mergeable fields can name each
independently. Naming a path turns off root-level merging for that prop
entirely - a path-merging prop never also merges its whole value.
`match_on` composes with a path by including the path in the field name
(`"data.id"`, not `"id"`); the framework doesn't infer it for you.
`.deep_merge()` ignores `.merge_with_path` - a deep merge already
recurses into every nested field, so there's nothing a path narrows.

A merge prop's value can come from a resolver too, via `.merge_lazy` /
`.merge_lazy_with` - the resolver sibling of `.merge` / `.merge_with`:

```rust
InertiaResponse::new("Feed/Index").merge_lazy("posts", || async {
    Ok::<_, FrameworkError>(load_next_page().await?)
})
```

The resolver runs only when the merge prop will actually be sent -
skipped by partial-reload filtering and by `.defer()` like any other
resolver-backed prop.

Infinite scroll is the same machinery with pagination metadata attached.
`.scroll` / `.scroll_with` - or `.paginate`, which adapts a
`LengthAwarePaginator` or `CursorPaginator` directly - emit `scrollProps`
next to the data, and the client's `<InfiniteScroll>` component drives the
next/previous fetches:

```rust
// `posts` is a CursorPaginator from the query builder.
InertiaResponse::new("Feed/Index").paginate("posts", posts)
```

A scroll prop always carries merge metadata, not just on a follow-up
fetch: it defaults to append, and switches to prepend only when the
client's `X-Inertia-Infinite-Scroll-Merge-Intent` header says so (`append`
when scrolling down, `prepend` when scrolling up). `reset` is independent
of that header - it's `true` exactly when the client named the key in
`X-Inertia-Reset`, the same header a regular merge prop reads. A fresh,
unfiltered visit sends neither header, so it gets `reset: false` and an
append instruction, matching Laravel.

`.merge_with_path` has no effect on a scroll prop - the scroll block that
computes its merge instruction reads `Prop::scroll_wrap`'s single wrap
key, not `.merge_with_path`'s accumulated path list, so
`.scroll(metadata).merge_with_path("data")` stores a path nothing reads.
`.scroll_wrap` - reached directly through `.prop(...)`, or through the
`.scroll_wrapped` response shortcut below - is the nesting equivalent for
a scroll prop.

A scroll prop also honors `.match_on(...)`, the same as any other merge
prop - reach it through `.prop(...)`, since neither `.scroll` nor
`.match_on` has a combined response-level shortcut:

```rust
InertiaResponse::new("Users/Index").prop(
    "users",
    Prop::eager(rows)
        .scroll(ScrollMetadata::new("page").current(1).next(2))
        .match_on("id"),
)
```

The match field keys off wherever the prop actually merges: the bare key
when unwrapped (`matchPropsOn: ["users.id"]`), or `key.wrap_key` under
`.scroll_wrap(...)` (`matchPropsOn: ["posts.data.id"]` for a prop wrapped
under `"data"`) - so the entry always lines up with the merge path the
client folds, instead of silently never matching.

When the prop's value is itself a wrapped structure - `{ data: [...],
meta: {...} }`, the shape a hand-built API resource typically returns -
merging the whole object would clobber `meta` on every fetch. Point the
merge at the array field instead with `.scroll_wrapped`:

```rust
InertiaResponse::new("Feed/Index").scroll_wrapped(
    "posts",
    "data",
    ScrollMetadata::new("page").current(2).next(3),
    serde_json::json!({ "data": rows, "meta": { "total": total } }),
)
```

`mergeProps` then names `posts.data`, so the client folds new rows into
the nested array and leaves `meta` to be replaced wholesale each time.
`.scroll_with_wrapped` and `try_scroll_wrapped` are the resolver-based and
fallible siblings, matching `.scroll_with` / `try_scroll`.

A type outside this crate's `pagination` module - a third-party
paginator, a hand-rolled cursor - can describe itself to `.scroll`
by implementing `ProvidesScrollMetadata` instead of building
`ScrollMetadata` field by field:

```rust
use suprnova::{ProvidesScrollMetadata, ScrollMetadata};

impl ProvidesScrollMetadata for MyCursorPage {
    fn page_name(&self) -> String { "cursor".to_string() }
    fn previous_page(&self) -> Option<serde_json::Value> { self.prev.clone().map(Into::into) }
    fn next_page(&self) -> Option<serde_json::Value> { self.next.clone().map(Into::into) }
    fn current_page(&self) -> Option<serde_json::Value> { Some(self.current.clone().into()) }
}

InertiaResponse::new("Feed/Index").scroll("posts", page.scroll_metadata(), page.rows)
```

`LengthAwarePaginator`, `Paginator`, and `CursorPaginator` implement it too - see [Pagination](pagination.md#inertia-integration-infinite-scroll-props).

### Dot-notation nesting

A key containing `.` nests into the response instead of shipping as a
literal string key - Laravel's `Arr::set`-backed dot notation
(`Inertia::share('user.name', …)`, `resolveArrayableProperties`):

```rust
InertiaResponse::new("Dashboard")
    .with("user.name", "Todd")
    .with("user.locale", "es")
```

ships as:

```json
{ "user": { "name": "Todd", "locale": "es" } }
```

not two literal `"user.name"` / `"user.locale"` keys. Two calls sharing a
prefix accumulate into one object; a key with no dot is unaffected. This
applies to every prop-attaching method - `.with`, `.always`, `.lazy`,
shared-registry keys - and to nothing else: it never recurses into a
prop's *value*, so a validation `errors` object keeps whatever dotted
field names it carries internally. There is no escape hatch for a key
that must keep a literal dot (`.with("config.json", …)` still nests) -
this matches Laravel, where `Arr::set` has no escaping mechanism either.

## Partial reloads

The Inertia 3 client can request a subset of a page's props (or a
superset by including an Optional or Defer key). The protocol uses
three request headers:

| Header | Meaning |
|---|---|
| `X-Inertia-Partial-Component` | The component being partial-reloaded - must match the response's component for filtering to apply. |
| `X-Inertia-Partial-Data` | Whitelist: comma-separated prop keys to include. |
| `X-Inertia-Partial-Except` | Blacklist: comma-separated prop keys to exclude. Wins over `Partial-Data` on key collision. |

Filtering reads one thing: the prop's visibility, set by `.always()`,
`.optional()`, or `.defer()`. A prop with none of those has the default
visibility.

- Default-visibility props follow whitelist / blacklist semantics.
- `.always()` props are sent regardless.
- `.optional()` and `.defer()` props never ship on a standard visit, and
  only appear on a matching partial reload that explicitly lists the key.

The merge and scroll flags do not enter into it: they decide how the
client folds a value it receives, not whether it receives one, so a
`.defer().merge()` prop filters exactly like a plain `.defer()` one.
`.once()` doesn't enter into it either, though it isn't purely a folding
instruction - on a full visit where the client reports the value already
cached, the server skips the resolver and sends no value, as the note
below describes. What all three change is which metadata blocks ride
along - see [Composing flags on one prop](#composing-flags-on-one-prop).

The handler doesn't have to do anything special - register every prop
through the builder, and the framework consults the headers when
serializing the page object.

A `once` prop's client-side cache is honoured only on a **full** Inertia
visit. On a partial reload that names the key
(`router.reload({ only: ['stats'] })`), the resolver runs and the value is
sent - the client asked precisely because it wants a fresh one, and
honouring its stale-cache claim there would return nothing at all for the
key it asked for.

### Nested only/except (dot notation)

`X-Inertia-Partial-Data` and `X-Inertia-Partial-Except` entries can name a
path inside a prop's value, not just the prop's own key. A client calling
`router.reload({ only: ['user.name'] })` sends
`X-Inertia-Partial-Data: user.name`, and the response narrows the `user`
prop down to just that field:

```json
{ "props": { "user": { "name": "Ada" } } }
```

`except` prunes the same way instead of narrowing - `router.reload({
except: ['user.email'] })` leaves every other field of `user` in place.

Rules:

- A bare entry (`user`) still means the whole prop. If `only` names both
  `user` and `user.name`, the whole value ships - the bare entry wins.
- An entry can also name an *ancestor* of a dotted prop key. A prop
  registered under `auth.user` - by `.with("auth.user", …)` or
  `App::inertia_share("auth.user", …)` - participates in
  `only: ['auth']`, and ships whole, because the caller asked for the
  whole `auth` root. A bare `except: ['auth']` drops it for the same
  reason. The prefix has to end on a segment boundary, so an unrelated
  `authAgent.user` prop is untouched by either.
- `except` wins on a path both headers name, the same way it wins at the
  top level.
- A path that doesn't resolve against the value - an unknown field, or one
  that drills through a scalar or an array instead of an object -
  contributes nothing for that path, without dropping the sibling fields
  requested alongside it.
- `Always` props ignore `only`/`except` entirely, dot notation included -
  they always ship whole.
- `Optional` and `Defer` props still need the explicit request to resolve
  at all. A dotted entry (`permissions.read`) counts as that request for
  the top-level key, and the resolved value narrows the same way an
  `Eager` prop's does.
- A dotted `only` against a prop whose current value isn't an object -
  a string, a number, an array - narrows to `{}`, not to the original
  value. The client's reconciliation only deep-merges when *both* the
  cached value and the incoming one are objects
  (`inertia-3.6.1/packages/core/src/response.ts` `nestedTopKeys`); an
  empty object fails that check against a non-object cache the same way
  a populated one would, so the empty object replaces the cached scalar
  outright instead of merging onto it. Avoid sending a dotted request
  against a prop that isn't shaped as an object.
- A dotted `except` doesn't delete the field on the client - it stops the
  field from refreshing on this response, and the client's merge restores
  it from whatever it already had cached. `deepMergeObjects` builds the
  merged object by cloning the cached value first and then only
  overwriting the keys the server actually sent; a key the server pruned
  is never touched, so it survives with its old value. On a
  client's first-ever load of that prop (nothing cached yet) the pruned
  field is genuinely absent, since there's no cache to fall back to - the
  "restores from cache" behavior only applies to a page the client has
  already seen.

## Shared data via `App::inertia_share*`

Some props are the same on every Inertia page - auth state, the CSRF
token, the current locale, app-wide flags. Register them once at
bootstrap and they merge into every response:

```rust
use suprnova::App;
use std::sync::Arc;

pub fn register() {
    // Sync, materialized once at boot.
    App::inertia_share("appName", "Suprnova");
    App::inertia_share("appVersion", env!("CARGO_PKG_VERSION"));

    // Async, resolved per response (skipped by partial reloads that
    // exclude the key).
    App::inertia_share_lazy("locale", || async {
        Ok::<_, suprnova::FrameworkError>(detect_locale().await)
    });

    // Cached on the client across navigations - `share_once` runs on
    // the first page that needs it, then the client skips re-resolution
    // via `X-Inertia-Except-Once-Props` until the cache key changes.
    App::inertia_share_once("plans", || async {
        Ok::<_, suprnova::FrameworkError>(load_plan_catalog().await?)
    });
}
```

Shared keys nest on dots the same way `.with` does - two static shares
under `"user.name"` / `"user.age"` land in one `user` object on the wire.
Read a shared value back, or clear the static registry entirely, with
`App::inertia_shared` / `App::flush_inertia_shared` - Laravel's
`Inertia::getShared` / `Inertia::flushShared`:

```rust
use suprnova::App;

App::inertia_share("user.name", "Todd");
assert_eq!(App::inertia_shared("user.name"), Some(serde_json::json!("Todd")));

App::flush_inertia_shared();
assert_eq!(App::inertia_shared("user.name"), None);
```

`inertia_shared` reads the static registry only - it returns `None` for a
key registered via `inertia_share_lazy` / `inertia_share_once` (there's no
request to resolve one against, mirroring Laravel's `getShared`, which
returns the raw closure rather than invoking it) and for a per-request
trait-provider share. `flush_inertia_shared` clears only the static
registry too; a provider registered via `register_inertia_shared` has no
per-request state to flush.

For per-request shared data (the authenticated user, request-scoped
flags), implement [`InertiaSharedData`](#per-request-shared-data) and
register the singleton - the framework calls `share(&req, component)` on
every Inertia response and merges the result. `component` is the page
being rendered, so a provider can vary its output by page - see below.

### Precedence on key collision

When the same key appears in more than one layer, later writes win:

1. Static registry (`App::inertia_share` / `App::inertia_share_lazy`)
2. Per-request trait provider (`InertiaSharedData::share`)
3. Per-response builder methods (`.with`, `.lazy`, etc.)

This lets a handler override a globally-shared default for one page
without having to unregister anything.

### Per-request shared data

The trait runs once per Inertia response with access to the request
**and** the page component name - Laravel's `RenderContext` (`component`,
`request`), passed as a plain parameter rather than a wrapper struct
since the request already covers the other half. Implementations need
`async_trait` (re-exported as `suprnova::__async_trait`) and `IndexMap`
(re-exported as `suprnova::indexmap`):

```rust
use suprnova::{
    App, Auth, FrameworkError, InertiaRequestExt, InertiaSharedData, Prop,
    indexmap::IndexMap,
};
use std::sync::Arc;

pub struct AuthShare;

#[suprnova::__async_trait]
impl InertiaSharedData for AuthShare {
    async fn share(
        &self,
        _req: &dyn InertiaRequestExt,
        component: &str,
    ) -> Result<IndexMap<String, Prop>, FrameworkError> {
        let mut out = IndexMap::new();
        if let Some(user) = Auth::user().await? {
            out.insert(
                "auth".into(),
                Prop::eager(serde_json::json!({
                    "id": user.get_auth_identifier(),
                })),
            );
        }
        // Vary by page: only the admin dashboard needs the nav counts.
        if component == "Admin/Dashboard" {
            out.insert("pendingReviews".into(), Prop::eager(serde_json::json!(12)));
        }
        Ok(out)
    }
}

// In bootstrap:
App::register_inertia_shared(Arc::new(AuthShare));
```

Ignore `component` (`_component`) if your provider doesn't need to vary by page.

## Flash and redirects

Flash data is one-shot state that should appear on the next render and
disappear after - toast messages, "just created" IDs, validation summaries.
Suprnova surfaces it under `page.flash` on every Inertia response. There
are three writers:

```rust
// 1. Push into the current request's flash bag.
App::flash("toast", "Saved");

// 2. Attach to a specific response (same effect on this response only).
InertiaResponse::new("Posts/Show").flash("toast", "Saved")

// 3. Carry across a redirect via the Redirect facade.
use suprnova::Redirect;

Redirect::to("/posts").with("toast", "Created")
```

The `Redirect::with(key, value)` form is the cross-handler path: the
value lands in the session under `_flash.new.*`, the next request's
[`SessionMiddleware`](csrf.md) ages it into `_flash.old.*`, and the
destination's `InertiaResponse` surfaces it under `page.flash`.

Same-request flash (the task-local bag) wins over inherited session
flash on key collision, so a destination handler can override an
inbound value just by re-flashing the key.

Internal session keys (anything prefixed `_`) are filtered out of
`page.flash` - `_old_input` for form repopulation and `_inertia.*`
protocol flags don't leak to the client.

### Redirect helpers

`Redirect` is the full Laravel surface:

```rust
Redirect::to("/dashboard")                       // 302 to a path
Redirect::route("posts.show").with("id", "42")   // named route, route params
Redirect::back("/")                              // session-recorded previous URL
Redirect::refresh()                              // same URL, fresh GET
Redirect::guest(&req, "/login")                  // stashes intended URL
Redirect::intended("/dashboard")                 // pops the stashed URL
Redirect::signed_route("downloads.show", &[("id","42")])?  // signed URL
Redirect::to("/posts/42").preserve_fragment()    // keep #frag across visit
```

All `Redirect` variants accept `.with(k, v)`, `.with_input(map)`,
`.with_errors(map)`, `.with_errors_bag(name, map)`, `.cookie(c)`,
`.header(k, v)`, `.permanent()`, `.status(303)`, etc. The full chain
mirrors Laravel's `RedirectResponse`.

For non-GET Inertia visits, the framework auto-converts the response to
`303 See Other` when [`Inertia303Middleware`](#bootstrap-inertia-install)
is installed, so the browser issues a clean follow-up GET instead of
re-submitting the original PUT/PATCH/DELETE to the redirect target.

### Validation failures

When a handler fails validation on an Inertia visit, the framework
answers `303 See Other` back to the form page with the errors flashed,
instead of the `422` JSON a REST client gets. That is not cosmetic: the
Inertia client treats any response without an `X-Inertia` header as
non-Inertia and renders it in the full-screen error modal, so a `422`
never reaches `form.errors`. Nothing in the handler changes - the bridge
is one of the middlewares `Inertia::install` registers.

The destination is the request's `Referer` when it is same-origin, then
the session's recorded previous URL, then the failing request's own URL.
A cross-origin `Referer` is ignored rather than followed, and so is one
that only looks same-origin: a leading `//` or `/\` (a browser reads
either as protocol-relative once it folds a backslash into a slash) and
any ASCII control byte anywhere in the value (the URL parser strips tab
and newline from the whole string before it compares origins, so a
control byte can turn what looks like a safe path into a different
origin by the time a browser navigates it) both fall back the same way.
The same check applies to the final URL fallback too, so even an
unusual request path can't become an off-origin redirect.

A field's value is its **first** message, a plain string - the shape
Inertia's own `ErrorValue` type describes and what
`$page.props.errors.email` binds to. Set
`InertiaConfig::with_all_errors(true)` to get every message as an array
instead; the client-side type then needs the matching augmentation:

```ts
// global.d.ts
import '@inertiajs/core'

declare module '@inertiajs/core' {
  export interface InertiaConfig {
    errorValueType: string[]
  }
}
```

Multiple forms on one page stay isolated: send
`X-Inertia-Error-Bag: <name>` with the visit and the errors are flashed
under that bag and read back under it, arriving as `errors.<name>.<field>`.

The `errors` prop is always-visible by default, so a partial reload
never filters or narrows it. `only: ['users']` still ships the bag, and
so does `except: ['errors']`; `only: ['errors.email']` ships the whole
bag rather than just that field. This is Laravel's shape - its
middleware shares the bag as `Inertia::always(...)`, and `resolveAlways`
re-injects the raw value after the `only`/`except` rebuild. It matters
because the client folds a partial response in with
`{...current.props, ...response.props}`: an empty `errors` object would
wipe the messages already on screen, where an unfiltered one leaves them
correct. The rule covers both sources - the session-flashed bag and a
handler's own `.with("errors", …)`. An explicit visibility flag still
wins, so `.prop("errors", Prop::eager(…).optional())` behaves optionally.

Two things this does not do. It does not re-flash old input - the request
body is already consumed by the time the bridge runs, and an Inertia
`useForm` keeps its own state across a failed submit, so there is nothing
to repopulate. And it never touches a Precognition response: a dry-run
`422` is exactly what the client asked for.

To send the visitor **out** of the Inertia app - a payment provider, an
OAuth authorize endpoint, a hosted billing portal - use `location_for`:

```rust
use suprnova::{InertiaResponse, Request, Response};

pub async fn checkout(req: Request) -> Response {
    Ok(InertiaResponse::location_for(&req, "https://billing.example/checkout"))
}
```

An Inertia XHR gets `409` + `X-Inertia-Location` (the client runs
`window.location = url`); a hard navigation gets a plain `302` + `Location`.
The bare `InertiaResponse::location(url)` always returns the 409 form - use
it only where the request is already known to be an Inertia visit, because
a browser that follows a `409` with no `Location` header has nowhere to go.

## Version detection

Inertia versions the asset manifest so a long-lived client doesn't try
to mount a page from yesterday's bundle against today's server. When
the client's `X-Inertia-Version` header doesn't match the server's
configured version, [`InertiaVersionMiddleware`](#bootstrap-inertia-install)
responds with `409 Conflict` and an `X-Inertia-Location` header naming
the new URL - the Inertia client picks that up and does a full page
reload, picking up the new bundle.

The bounce re-flashes the session first. The client answers a 409 with a
full-page GET, and that GET is a fresh request - without the re-flash, a
validation error or success message flashed by the previous request is aged
away before the destination page can read it, and the user loses their error
message purely because a deploy landed mid-submit. This needs
`SessionMiddleware` registered ahead of the version middleware.

By default you set nothing: `InertiaConfig` hashes your Vite build
manifest (`manifest_path`, default `public/assets/.vite/manifest.json`)
and uses the first 16 bytes of its SHA-256, hex-encoded. The manifest is
the one file that changes on every build and on no other occasion, so
the version bumps itself. When there is no manifest to read - local
development, where Vite serves from memory - it falls back to the static
string `"1.0"` and logs at `debug`.

Override it when you want something else:

```rust
use suprnova::{InertiaConfig, VersionResolver};

// Default - hash the build manifest. Nothing to write.
let cfg = InertiaConfig::new();

// A different manifest location; the version follows it.
let cfg = InertiaConfig::new().manifest_path("dist/.vite/manifest.json");

// Static - bake in a build-time identifier. Survives a later
// `.manifest_path(...)` call: an explicit version is deliberate.
let cfg = InertiaConfig::new().version(env!("CARGO_PKG_VERSION"));

// Dynamic - a container deployment id, anything. The closure runs on
// every version check; cache inside if it isn't cheap.
let cfg = InertiaConfig::new().version_with(|| deployment_id());
```

The manifest is read on every version check, which is what Laravel's
`hash_file` does too - a few KB out of the page cache, and a rebuild is
picked up immediately. If you have measured that and want it gone,
resolve once at boot:

```rust
use suprnova::{InertiaConfig, VersionResolver};

let version = VersionResolver::from_manifest("public/assets/.vite/manifest.json").resolve();
let cfg = InertiaConfig::new().version(version);
```

For async or fallible version resolution (e.g. read a manifest hash
from S3), do the read once at boot and pass the cached `String` to
`.version(...)`.

## Bootstrap: `Inertia::install`

Most apps install the four protocol middlewares in one call, from
`register_http_stack` - the HTTP-only bootstrap hook, which the server
path runs and the queue, schedule, workflow, and console binaries skip
(see [Bootstrap](bootstrap.md)):

```rust
use suprnova::{Inertia, InertiaConfig};

pub fn register_http_stack() {
    let cfg = InertiaConfig::new()
        .version(env!("CARGO_PKG_VERSION"))
        .default_title("My App");

    Inertia::install(&cfg)
        .expect("Inertia install failed (production needs a built frontend manifest)");
    // …the rest of your global middleware, in the order you want it to run
}
```

Anything the Inertia layer depends on - `SessionMiddleware` - and
anything an error page needs to read - `LocaleMiddleware` - goes *above*
this call. See [the ordering rules below](#bootstrap-inertia-install).

```rust
// cmd/main.rs
Application::new()
    .bootstrap(bootstrap::register)
    .http_bootstrap(|| async { bootstrap::register_http_stack() })
```

Keep it out of `bootstrap::register`. `Inertia::install` fails closed in
production when the built frontend manifest is missing, which is exactly
the state of a worker or console image that ships no `public/assets` -
so installing it from the process-wide hook takes those binaries down
with it.

`Inertia::install` returns `Result` and, in order:

1. Fails closed if `cfg` resolves to production mode (`development ==
   false` - the default whenever `APP_ENV=production`) but no Vite
   manifest can be loaded from `cfg.manifest_path`. This is the CFG-01
   guard: a production boot with an unbuilt frontend errors loudly
   instead of silently falling back to a legacy hardcoded asset path.
2. Registers `InertiaHeadersMiddleware` - sets `Vary: X-Inertia` on every
   response and turns an empty `200` on an Inertia visit into a `303` back.
3. Registers `InertiaVersionMiddleware` - emits the `409` + `X-Inertia-Location`
   when client and server disagree on the asset version.
4. Registers `Inertia303Middleware` - upgrades `302` to `303` on non-GET
   Inertia redirects.
5. Registers `InertiaValidationRedirectMiddleware` - turns a `422` on an
   Inertia visit into a `303` back to the form page with the errors
   flashed. See [Validation failures](#validation-failures).
6. Registers `InertiaErrorPageMiddleware`, **only when** `cfg` names an
   `.error_page(...)` - turns the framework's own error responses into
   that page. See [Error pages](#error-pages).

Order matters: the headers middleware is registered first, so it is the
outermost and sees every response - including the `409` the version
middleware returns before the handler ever runs. The validation-redirect
middleware is registered last, so it is innermost - closest to the
handler - and sees a `422` before the other three middlewares get a
chance to touch it.

`install` also **retains the config**. Every `InertiaResponse` built
afterwards starts from it, so `.frontend(...)`, `.version(...)`,
`.default_title(...)`, `.ssr(...)` and `.encrypt_history(...)` set here
reach every page without a handler passing anything. A handler that wants
different settings for one page still overrides with `.with_config(...)`;
an app that never calls `Inertia::install` gets `InertiaConfig::default()`;
and calling `install` again replaces the retained config.

`.with_config(...)` replaces the config wholesale, `version` included.
`InertiaVersionMiddleware` still resolves the version `Inertia::install`
was given, so a config here that doesn't carry the same `.version(...)`
makes the page object advertise a version the middleware will bounce - the
client takes one extra full page load after visiting that page. Set
`.version(...)` on the override to match.

Register `SessionMiddleware` **ahead of** `Inertia::install` if you use
flash data. The version middleware re-flashes the session before bouncing
the client, so a flashed error survives the follow-up full-page GET; it
can only do that inside a session scope.

Register [`LocaleMiddleware`](localization.md) **ahead of it too**, if you
use an [error page](#error-pages). A middleware's post-`next` code runs
after everything inside it has already returned, so the error-page
middleware renders once any scope opened inside it has been popped -
which for the locale middleware means the page would get the app's
default locale instead of the visitor's. The Inertia layer reads nothing
from localization, so putting locale outside it costs nothing. The
scaffolded `bootstrap.rs` already does this. The same reasoning applies
to any middleware of yours whose request scope the error page needs to
read.

Skip the call only if you genuinely don't want one of these middlewares
(rare; all four close real failure modes - cache poisoning across the two
representations of a URL, silent stale-bundle, form-replay-on-redirect,
and a validation `422` dead-ending in the client's error modal instead of
reaching `form.errors`).

## Error pages

An Inertia visit that gets back a non-2xx from the framework does not
show an error page - it shows a crash screen:

```
All Inertia requests must receive a valid Inertia response, however a
plain JSON response was received.
```

The client checks one thing before it will render anything: an
`X-Inertia: true` header on the response. A `403` from an
[authorization](authorization.md) check or an RBAC permission
middleware, a `404` for an unrouted path, a `429` from the
[rate limiter](rate-limiting.md), a `500` from a
[failing handler](errors.md) - all of them carry the framework's JSON
error body and no such header, so the client hands them to its modal. A user with the wrong role clicks a nav
link and the app appears to break.

Name a page component and the framework renders those responses through
it instead, keeping the status code:

```rust
use suprnova::{Inertia, InertiaConfig};

pub fn register_http_stack() {
    Inertia::install(
        &InertiaConfig::new()
            .version(env!("CARGO_PKG_VERSION"))
            .error_page("Error"),
    )
    .expect("Inertia install failed (production needs a built frontend manifest)");
}
```

`"Error"` is resolved exactly like any other page name, so
`frontend/src/pages/Error.svelte` (or `.tsx`, or `.vue`) is all it takes.
**The three starters ship one and set `.error_page("Error")` already** -
a new project is covered without doing anything.

One ordering rule comes with it: **register `LocaleMiddleware` before
`Inertia::install`**, or error pages render in the app's default locale
rather than the visitor's. The error page is built on the way out, after
every middleware registered inside the Inertia layer has returned and
popped whatever scope it opened. The scaffolded `bootstrap.rs` gets this
right; if you wrote your own, check it. The same holds for any
request-scoped middleware of your own that the error page's shared props
read.

### What the page receives

| Prop | Type | Always present | What it is |
|---|---|---|---|
| `status` | `number` | yes | The original HTTP status - `403`, `404`, `500`. |
| `message` | `string` | yes | The error body's `message`, or the status's reason phrase when it carried none. Already sanitized: a `5xx` reads `"Internal Server Error"`, never the underlying error - and that holds under `APP_DEBUG=true` too. The dev-only `debug_message` field the JSON path adds there is deliberately not read, so the raw error stays in the log and the JSON response and never renders into a page. |
| `request_id` | `string` | no | Present only when the error body carried one. The same id the structured log records, so the page can show a reference the operator can search. |

```svelte
<script lang="ts">
  interface ErrorProps {
    status: number
    message: string
    request_id?: string
  }

  let { status, message, request_id }: ErrorProps = $props()
</script>

<h1>{status}</h1>
<p>{message}</p>
{#if request_id}<p>Reference: {request_id}</p>{/if}
```

Declare the props in the component rather than importing them from
`types/inertia-props.ts`: [`suprnova generate-types`](frontend-typescript-types.md) rewrites
that file from your own `#[derive(InertiaProps)]` structs, and these
props come from the framework.

### What survives the swap

The status code is kept, and so is every header the original response
set, **except** two groups.

**What described the body being replaced.** Every `Content-*` field
(`Content-Length` on a page four times the size of the JSON it replaced
is a framing bug) and `Transfer-Encoding`.
`Content-Security-Policy` is carved out of that rule by name - it shares
the prefix by historical accident and is response policy, not
representation metadata.

**What governed how that body could be stored.** `Cache-Control`,
`Expires`, `Age`, `ETag`, `Last-Modified`. The page carries your shared
props - `auth.user`, flash, the locale share - where the error body it
replaced was the same for everyone, so it must never inherit permission
to be stored by a shared cache and handed to a different visitor, nor
validators that belong to an entity it is not. The page sets
`Cache-Control: no-cache, private` for itself instead, the same default
Laravel gives a session-bearing response.

Everything else carries: `Retry-After` on a `429` still tells the client
when to come back, `WWW-Authenticate` on a `401` still carries the
challenge, and `Vary`, `Set-Cookie`, and your request-id header all
arrive intact. The rule is stated as what gets dropped rather than what
gets kept, so a header the framework has never heard of survives instead
of silently disappearing.

Both audiences are covered. An Inertia XHR visit gets the JSON page
object with `X-Inertia: true`; a hard navigation - someone pasting
`/admin/articles` into the address bar - gets the full HTML shell, the
same one a first load of any page gets. So the error page works whether
the user arrived through the SPA or not.

### What it never touches

The middleware only stands in where nobody else has an answer. It leaves
alone:

- **Validation `422`s.** `InertiaValidationRedirectMiddleware` owns
  those - see [Validation failures](#validation-failures). A `422` that
  survives that middleware (no `errors` object, or a Precognition
  dry-run) keeps its body too.
- **Anything carrying `X-Inertia-Location`.** The `409` version bounce,
  and the `redirect_to` form of the RBAC middlewares. The client acts on
  the header, not the body.
- **Redirects.** Only `400`-`599` is in scope.
- **API clients.** A request whose `Accept` prefers `application/json`
  over `text/html` keeps the JSON contract it has always had. `curl`'s
  `*/*` counts as no preference, so it keeps JSON too. Only an Inertia
  visit or a browser navigation gets a page.
- **Responses that already are Inertia pages.** A handler that rendered
  its own page and gave it a `410` keeps its own component.
- **Bodies that are not the framework's error shape.** Your own HTML
  error page, plain text that is not the router's own `404 Not Found`, or
  a JSON envelope keyed differently - none of those is overruled.
- **Everything, when `error_page` is unset.** The middleware is not
  registered at all, so an app that has not opted in runs exactly the
  code it ran before.

### Which bodies get rewritten

The gate is the **shape of the body**, not who wrote it. At a `400`-`599`
status, exactly three shapes are replaced:

- an empty body;
- a JSON object whose `message` is a string - the framework's own error
  envelope, and anything else shaped like it;
- the router's fixed `404 Not Found` plain-text body.

Everything else passes through. That means a `401` a middleware of yours
answers with `HttpResponse::json(json!({ "message": "Unauthenticated." }))`
**does** become the error page - which is the point, since that is exactly
the response the client would otherwise modal - and it means only
`message` and `request_id` survive into the props. An envelope carrying
`errors`, `code`, or anything else loses those fields when it becomes a
page.

If a middleware of yours must keep its own JSON body on an error status,
give it a shape the gate does not match - key the human-readable text as
something other than `message` - or set `X-Inertia: true` on the response
yourself, which marks it as already being an Inertia response and takes
it out of scope. Both are one line at the point that builds the response.

One gap worth knowing: a handler that **panics** is out of reach. The
panic net wraps the whole middleware chain, so the synthesized `500` is
built after every middleware frame has already unwound. Panicking
handlers still surface the client's modal. Return `Err(...)` rather than
panicking (see [Errors](errors.md)) and the error page covers it.

If the page itself fails to render - the component cannot be resolved,
SSR is down, a shared prop errors - the framework logs a `warn` with the
request id and returns the original error response. A broken error page
never masks the error it was rendering.

### Why Suprnova diverges

Laravel puts this in the exception handler: you edit
`bootstrap/app.php`, match on the status yourself, and call
`Inertia::render('Error', ['status' => $response->getStatusCode()])`
with `$response->setStatusCode(...)` to put the code back. That is
flexible, and it is also a piece of framework plumbing every project
rewrites by hand, usually after seeing the modal in production first.

Here it is one config line, because the decision is the same for every
app: an Inertia visit or a browser navigation gets a page, an API client
gets JSON, and everything another contract owns is left alone. The
trade is that the rule is a fixed one rather than a `match` you write, so
opting a particular response out means giving it a body the gate does not
recognize, or marking it as already-Inertia - see
[Which bodies get rewritten](#which-bodies-get-rewritten).

## Server-driven `<head>` elements

Inertia 3.5 added a client option for letting the server decide what goes in
`<head>` - useful when meta tags depend on the record you just loaded, and you
don't want the title and OG tags to live in two places.

This needs no framework support. The client reads the elements from an
**ordinary prop**, so any handler can supply them:

```rust
#[handler]
async fn show(RouteParam(post): RouteParam<Post>, req: Request) -> Response {
    inertia_response!(&req, "Posts/Show", {
        "post": post,
        "head": [
            format!("<title>{}</title>", post.title),
            format!(r#"<meta property="og:title" content="{}">"#, post.title),
        ],
    })
}
```

Opt in on the client:

```js
createInertiaApp({
  serverHead: true,        // reads the `head` prop
  // serverHead: 'meta',   // or read a differently-named prop
  // serverHead: (page) => [...],  // or compute from the whole page
})
```

Each string is an HTML element. The client stamps a `data-inertia` attribute on
anything that lacks one so it can diff head elements across navigations; supply
your own `data-inertia="og-title"` when you want stable identity rather than
positional matching.

Escape anything interpolated from user data - these strings are injected as
HTML, so the usual rules apply.

## SSR

Suprnova talks to an out-of-process SSR worker - typically the
`@inertiajs/{svelte,react,vue}/server` `createServer()` bundle run
under Node / Bun / Deno - over HTTP loopback. Enable it on the config you
hand to [`Inertia::install`](#bootstrap-inertia-install) - that config is
what every response starts from, so there is nothing to plumb through
your handlers:

```rust
Inertia::install(
    &InertiaConfig::new()
        .ssr("http://127.0.0.1:13714")  // worker URL
        .ssr_timeout(std::time::Duration::from_millis(500))
        .ssr_exclude("/admin/**")
        .ssr_max_response_bytes(8 * 1024 * 1024),
)?;
```

SSR is off by default, and it is a property of the config: on for every
response built from the installed config, off for any response that
overrides with a `.with_config(...)` which doesn't set it. When enabled,
the framework posts the page
object to `<url>/render` and inlines `{ head, body }` in the HTML
shell. On worker error or timeout the response falls back to CSR
(an empty `<div id="app">` the client hydrates) and the
`on_ssr_error(...)` hook fires; flip `ssr_throw_on_error(true)` in CI
to make those failures hard 500s instead.

Before it dispatches at all, the gateway can check that the built SSR
bundle exists on disk - opt in with `.ssr_bundle_path(...)`, pointed at
the conventional `frontend/bootstrap/ssr/ssr.js` (the check itself is on
by default, `.ssr_ensure_bundle_exists(true)`, but has no effect until a
path is set - this is deliberately not auto-detected, so enabling SSR
against a test double never has to also stub a bundle on disk). A
missing bundle falls back to CSR immediately, without paying
`ssr_timeout` on a connection that was never going to succeed. This
mirrors Laravel's `ensure_bundle_exists` config.

```rust
Inertia::install(
    &InertiaConfig::new()
        .ssr("http://127.0.0.1:13714")
        .ssr_bundle_path("frontend/bootstrap/ssr/ssr.js")
        .ssr_timeout(std::time::Duration::from_millis(500))
        .ssr_exclude("/admin/**")
        .ssr_max_response_bytes(8 * 1024 * 1024),
)?;
```

`suprnova new` scaffolds `frontend/src/ssr.{ts,tsx}` and a `build:ssr`
npm script for every starter. Build it, then boot the worker:

```bash
cd frontend && npm run build:ssr
suprnova ssr:start
```

`suprnova ssr:check` verifies the worker is actually answering - it
hits the worker's own `GET /health` route, which every `createServer()`
bundle exposes without any extra code.

## Configuration

Inertia behaviour is configured programmatically via `InertiaConfig`, and
the config you hand to [`Inertia::install`](#bootstrap-inertia-install) is
the one every response starts from. The one env var the framework reads
directly is `SUPRNOVA_FRONTEND` (`svelte` / `react` / `vue`), and it only
supplies the default entry-point filename and page-component extensions
when the config doesn't say - an explicit `.frontend(Frontend::React)` on
the installed config wins, and is what `suprnova new --frontend react`
scaffolds. Everything else is builder-shaped:

```rust
use suprnova::{InertiaConfig, Frontend};

let cfg = InertiaConfig::new()
    .frontend(Frontend::Svelte)               // overrides SUPRNOVA_FRONTEND
    .vite_dev_server("http://localhost:5765")
    .entry_point("src/main.ts")
    .version(env!("CARGO_PKG_VERSION"))
    .default_title("My App")
    .manifest_path("public/assets/.vite/manifest.json")
    .assets_base_url("/assets")
    .max_concurrent_resolvers(16)             // cap lazy-prop fan-out
    .with_all_errors(false)                   // one message per field, or all
    .url_resolver(|req| req.path_and_query()) // how `page.url` is derived
    .production();                            // false → loads from Vite dev server
```

Frontend-specific defaults:

| Frontend | Default entry point | Page extensions |
|---|---|---|
| Svelte (default) | `src/main.ts` | `.svelte` |
| React | `src/main.tsx` | `.tsx`, `.jsx` |
| Vue | `src/main.ts` | `.vue` |

### The `url` field

`page.url` is the path **and** query string of the request
(`/users?page=2&sort=name`). The client writes it into `history.state`, so
it is what back/forward navigation and `router.reload()` replay - drop the
query and every paginated or filtered page silently resets to page one.
`InertiaVersionMiddleware` derives its `X-Inertia-Location` from the
request's path and query too, so by default a 409 asset-version bounce
lands the browser on exactly the URL the page object named.

Override the derivation with `url_resolver` when the URL the client should
record differs from the one that arrived - a locale prefix the SPA doesn't
route on, or a path a reverse proxy rewrote:

```rust
use suprnova::InertiaConfig;

let cfg = InertiaConfig::new()
    .url_resolver(|req| req.path_and_query().replacen("/en", "", 1));
```

The resolver reads the request through `InertiaRequestExt`, and applies to
every response built from the config you pass to
[`Inertia::install`](#bootstrap-inertia-install) - the usual place for a
resolver that should apply app-wide. Override it for a single response
with `InertiaResponse::with_config(cfg)`. A resolver changes `page.url`
only. The 409 bounce keeps naming the URL that actually arrived - that is
the URL the browser has to fetch - so with a resolver in place the two
deliberately differ.

The Vite manifest at `manifest_path` is loaded lazily on first request
and cached for the process lifetime - every response built from the
installed config shares that one cache, so the file is read and parsed
once. When it's missing, production asset tags fall back to a hardcoded
legacy path and a `tracing::warn!` fires so the gap surfaces in logs.

### Why Suprnova diverges

Laravel's Inertia adapter has a single global "shared data"
registry plus a per-request `Inertia::share($k, $v)` call. PHP's
request-per-process model makes this safe: a fresh process per request
means no leakage between concurrent visitors.

Rust's process model is the opposite - one process serves many
concurrent requests across many threads. So the registry lives on
the [container](container.md) (task-local → thread-local → global),
not in process-global statics. `App::inertia_share*` writes to the
active container's `InertiaRegistry`, which gives tests using
`TestContainer::fake()` clean isolation without having to unregister
anything. Same surface as Laravel; different machinery underneath
because the runtime is different.

Nine other Rust-shaped choices worth flagging:

- **Lazy-prop resolvers run concurrently**, capped by
  `max_concurrent_resolvers` (default 16). A page with twelve lazy
  props issues twelve parallel queries inside one Tokio task - that's
  what we built the framework on top of Tokio for. Tune the cap if a
  page has many lazy props each hitting an external service.
- **The compile-time component check** isn't a Laravel feature at all,
  because PHP can't see your frontend files at compile time. Suprnova
  does, so a typo in `inertia_response!("Dashbaord", …)` fails the
  build with a "did you mean Dashboard?" suggestion instead of
  surfacing as a runtime "component not found" later.
- **An empty `200` on an Inertia visit becomes a `303`, not a `302`.**
  Laravel's `onEmptyResponse` returns `redirect()->back()` (a 302) and
  relies on its later `302 → 303` conversion for PUT/PATCH/DELETE only. A
  substituted redirect is never a continuation of the original method - the
  client has to issue a GET - so Suprnova says `303` directly instead of
  leaving GET visits on a 302 the client would follow with the original
  verb.
- **`Inertia::location($url)` is two methods here, not one.** `location(url)`
  keeps Laravel's always-`409` contract - it predates the request-aware
  form and pinned-tag consumers depend on that shape not changing.
  `location_for(&req, url)` is the newer, request-aware form: `409` for an
  Inertia XHR, plain `302` for a hard navigation. Reach for `location_for`
  in new code.
- **`Inertia::clearHistory()` is two methods here, not one, either.**
  `.clear_history()` on the builder marks a single response; `App::clear_history()`
  flashes the flag into the session so it survives a redirect. Laravel gets
  away with one method because it's already session-backed - Suprnova
  keeps the response-local form as the default (no session dependency) and
  makes the cross-redirect case an explicit opt-in instead.
- **`.lazy()` isn't Laravel's `Inertia::lazy()`.** Laravel's method is
  deprecated and behaves like `optional()` - `LazyProp` is a straight
  alias for `OptionalProp`, skipped entirely on the initial visit
  (`ResponseFactory.php:174-181`). Suprnova's `.lazy()` is the
  plain-closure convention Laravel itself uses for a callable prop with
  no wrapper at all - included whenever partial-reload filtering lets the
  key through, standard visits included. Reach for `.optional()` for the
  initial-visit-skipped behavior the name "lazy" suggests if you're
  coming from Laravel.
- **Nested `only`/`except` narrow after resolving, not before.** Laravel's
  `Response::resolvePartialProperties` walks the dotted path through the
  raw, not-yet-resolved prop array, so a path into a `LazyProp` or
  `DeferProp` degrades to `null` - the walk hits an unresolved closure and
  stops (`inertia-laravel-2.0.25/src/Response.php:273-297`). Suprnova
  resolves every prop's value first - resolvers are async, so there's no
  synchronous point where they're all plain arrays the way Laravel
  sometimes has - then narrows the resulting JSON value. An unknown or
  type-mismatched nested path is dropped instead of sent back as `null`,
  matching what the client's own reconciliation expects: it deep-merges a
  narrowed object onto what it already holds
  (`inertia-3.6.1/packages/core/src/response.ts:414-425`), and a stray
  `null` would clobber a field the client already has instead of leaving
  it alone.
- **`.scroll_wrapped` is opt-in, not automatic.** Laravel's
  `Inertia::scroll($value, $wrapper = 'data', …)` nests every scroll
  prop's merge instruction under `"data"` by default, because a Laravel
  paginator resource typically returns `{ data: [...], links: {...},
  meta: {...} }` and only the array should merge. Suprnova's built-in
  paginators hand back a bare row array (`Vec<T>`, no envelope), so
  `.scroll` / `.paginate` merge at the prop's root, and `.scroll_wrapped`
  is there for the cases that need the nested path instead.
- **A wrapped scroll prop prefixes its `match_on` fields for you.** On a
  `.scroll_wrapped("posts", "data")` prop, `match_on("id")` emits
  `"posts.data.id"`. Laravel emits the unprefixed `"posts.id"`, which its
  own client then fails to line up against the merge target, so the match
  silently never fires. The nesting point is unambiguous here - a scroll
  prop has at most one wrapper - so Suprnova derives the prefix rather
  than making you type it. Write the bare field name, not the path.

## Next

- [Page Components](frontend-pages.md) - how the frontend resolves a
  component name to a Svelte / React / Vue module
- [TypeScript Types](frontend-typescript-types.md) - `suprnova generate-types`
  emits TS definitions from your `#[derive(InertiaProps)]` structs
- [Data Objects](data.md) - `#[derive(Data)]` for DTOs with per-field
  include/allowlist gating that composes with partial reloads
- [Error Model](error-model.md) - how `Response`, the panic boundary,
  and `FrameworkError` thread through Inertia responses
- [Container](container.md) - the lookup model behind
  `App::inertia_share*` and `InertiaSharedData`
